import CryptoKit
import XCTest
import SolanaClearsign
@testable import ClearsignDemo

/// The Subscriptions & Allowances captures from devnet, rendered offline from
/// the bundled snapshots through the same client the inspector uses.
final class DevnetInspectorTests: XCTestCase {
    private let programId = "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44"
    private let usdcDevnet = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
    private let amountInstructions = ["createFixedDelegation", "createRecurringDelegation", "subscribe"]
    private let authorityInstructions = ["initSubscriptionAuthority", "revokeSubscriptionAuthority"]

    func testBundleContainsDevnetResources() async throws {
        let resources = try InspectorResources(bundle: .main)
        let repository = try XCTUnwrap(resources.repository(for: "devnet"))

        XCTAssertEqual(repository.catalog.programId, programId)
        XCTAssertEqual(repository.catalog.programName, "Subscriptions")
        XCTAssertEqual(repository.catalog.rpc, RPCSpec(kind: "public", endpoint: "https://api.devnet.solana.com"))
        let requiredInstructions = amountInstructions + authorityInstructions + ["revokeDelegation", "transferRecurring"]
        XCTAssertEqual(Set(repository.catalog.examples.map(\.instruction)), Set(requiredInstructions))
        XCTAssertEqual(repository.catalog.examples.count, requiredInstructions.count)
        XCTAssertEqual(resources.registry(for: "devnet")?.cluster, "devnet")
        let resolved = try await resources.idlSource.idl(for: programId)
        XCTAssertNotNil(resolved, "the manifest must bundle the Subscriptions IDL")
        XCTAssertEqual(resolved?.provenance.expectedSHA256, try provenanceRootSHA256())
    }

    func testEveryDevnetExampleRendersAndMatchesTheOracle() async throws {
        let (repository, client) = try makeClient()
        let expected = try TestPaths.loadExpectedDisplays(fixture: "subscriptions")
        let rootSHA256 = try provenanceRootSHA256()

        for example in repository.catalog.examples {
            let rendered = try await render(example, repository: repository, client: client)
            let expectation = try XCTUnwrap(expected["\(example.instruction)-devnet"], example.instruction)
            XCTAssertEqual(rendered.canonical, expectation?.display, example.instruction)
            XCTAssertTrue(rendered.idl.digestPinned, example.instruction)
            XCTAssertEqual(rendered.idl.sha256, rootSHA256, example.instruction)
            XCTAssertEqual(rendered.idl.programName, "subscriptions", example.instruction)
            if example.instruction == "transferRecurring" {
                XCTAssertEqual(rendered.hints.linkedAccountReads.map(\.address), [usdcDevnet])
            } else {
                XCTAssertEqual(rendered.hints.linkedAccountReads, [], example.instruction)
            }
        }
    }

    func testDelegationAmountsStillRequireAnExplicitMintBinding() async throws {
        let (repository, client) = try makeClient()
        var checked = 0
        for example in repository.catalog.examples where amountInstructions.contains(example.instruction) && example.instruction != "subscribe" {
            let rendered = try await render(example, repository: repository, client: client)
            XCTAssertEqual(rendered.hints.amounts.count, 1, example.instruction)
            XCTAssertEqual(rendered.hints.amounts.first?.degraded, true, example.instruction)
            XCTAssertEqual(rendered.hints.amounts.first?.decimals, .unsatisfied, example.instruction)
            XCTAssertTrue(rendered.hints.interpolatedIntentSuppressed, example.instruction)
            XCTAssertEqual(
                rendered.diagnostics.map(\.code),
                ["amount_scale_unresolved", "interpolated_intent_unavailable"],
                example.instruction
            )
            XCTAssertTrue(InstructionPreview(rendered: rendered).fields.contains { $0.isRaw }, example.instruction)
            XCTAssertEqual(InstructionPreview(rendered: rendered).diagnostics, rendered.diagnostics)
            checked += 1
        }
        XCTAssertEqual(checked, amountInstructions.count - 1)
    }

    func testCapturedPaymentShowsUsdcAndExplainsAnUnavailableMint() async throws {
        let (repository, client) = try makeClient()
        let example = try XCTUnwrap(repository.catalog.examples.first { $0.instruction == "transferRecurring" })
        let rendered = try await render(example, repository: repository, client: client)
        XCTAssertEqual(rendered.canonical.fields[0].value, "0.1")
        XCTAssertEqual(rendered.hints.amounts.first?.rawValue, "100000")
        XCTAssertFalse(InstructionPreview(rendered: rendered).fields.contains { $0.isRaw })
        XCTAssertFalse(rendered.diagnostics.contains { $0.code == "amount_scale_unresolved" })
        guard case let .tokenAmount(symbol, _, _, appliesToValue)? = rendered.presentation.annotations(for: 0).first?.kind else {
            return XCTFail("Expected a token amount annotation")
        }
        XCTAssertEqual(symbol, "USDC")
        XCTAssertTrue(appliesToValue)

        let resources = try InspectorResources(bundle: .main)
        let noMintClient = SolanaClearSigningClient(
            idlSource: resources.idlSource,
            presentationProvider: resources.registry(for: "devnet")
        )
        let raw = try await render(example, repository: repository, client: noMintClient)
        XCTAssertEqual(raw.canonical.fields[0].value, "100000 (raw)")
        XCTAssertTrue(raw.diagnostics.contains { $0.code == "amount_scale_unresolved" })
        XCTAssertEqual(InstructionPreview(rendered: raw).diagnostics, raw.diagnostics)
    }

    func testCapturedSubscribeUsesSdkFormattedAmountAndRetainsRawDetails() async throws {
        let (repository, client) = try makeClient()
        let example = try XCTUnwrap(repository.catalog.examples.first { $0.instruction == "subscribe" })
        let rendered = try await render(example, repository: repository, client: client)
        let mintField = try XCTUnwrap(rendered.canonical.fields.firstIndex { $0.label == "Token mint" })
        XCTAssertEqual(rendered.canonical.fields[mintField].value, usdcDevnet)
        let amountIndex = try XCTUnwrap(rendered.hints.amounts.first?.fieldIndex)
        let amount = try XCTUnwrap(rendered.presentation.tokenAmount(for: amountIndex))
        XCTAssertEqual(amount.value, "10")
        XCTAssertEqual(amount.decimals, 6)
        XCTAssertEqual(amount.mint, usdcDevnet)
        XCTAssertEqual(rendered.canonical.fields[amountIndex].value, "10000000 (raw)")
        let preview = InstructionPreview(rendered: rendered)
        XCTAssertEqual(preview.fields.first { $0.id == amountIndex }?.value, "10")
        XCTAssertEqual(preview.fields.first { $0.id == amountIndex }?.isRaw, false)
        XCTAssertFalse(InstructionPreview(rendered: rendered).fields.contains { $0.isRaw })
        XCTAssertFalse(rendered.diagnostics.contains { $0.code == "amount_scale_unresolved" })
        let noMetadata = SolanaClearSigningClient(idlSource: try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources")).idlSource)
        let raw = try await render(example, repository: repository, client: noMetadata)
        XCTAssertEqual(InstructionPreview(rendered: raw).fields.first { $0.id == amountIndex }?.value, "10000000 (raw)")
        XCTAssertEqual(InstructionPreview(rendered: raw).fields.first { $0.id == amountIndex }?.isRaw, true)
        XCTAssertTrue(InstructionPreview(rendered: raw).fields.contains { $0.isRaw })
        XCTAssertTrue(raw.diagnostics.contains { $0.code == "amount_scale_unresolved" })
        XCTAssertEqual(InstructionPreview(rendered: raw).diagnostics, raw.diagnostics)
        XCTAssertEqual(rendered.hints.publicKeyArguments.map(\.address), [usdcDevnet])
        XCTAssertEqual(
            rendered.presentation.annotations(for: mintField).map(\.kind),
            [.tokenMint(symbol: "USDC", name: "USD Coin")]
        )
    }

    func testAuthorityInstructionsShowTheTokenMintAndNoAmounts() async throws {
        let (repository, client) = try makeClient()
        var checked = 0
        for example in repository.catalog.examples
            where authorityInstructions.contains(example.instruction)
        {
            let rendered = try await render(example, repository: repository, client: client)
            let mintField = try XCTUnwrap(rendered.canonical.fields.firstIndex { $0.label == "Token mint" })
            XCTAssertEqual(rendered.canonical.fields[mintField].value, usdcDevnet, example.instruction)
            XCTAssertEqual(
                rendered.presentation.annotations(for: mintField).map(\.kind),
                [.tokenMint(symbol: "USDC", name: "USD Coin")],
                example.instruction
            )
            XCTAssertEqual(rendered.hints.amounts, [], example.instruction)
            XCTAssertEqual(rendered.diagnostics, [], example.instruction)
            XCTAssertNotNil(rendered.canonical.interpolatedIntent, example.instruction)
            checked += 1
        }
        XCTAssertEqual(checked, authorityInstructions.count)
    }

    func testBundledIdlCopiesMatchManifestAndConformance() throws {
        let resources = try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources"))
        let bundled = try Data(contentsOf: resources.srf39Directory.appendingPathComponent("subscriptions-root.json"))
        let conformance = try Data(contentsOf: TestPaths.fixtureDirectory("subscriptions").appendingPathComponent("root.json"))
        XCTAssertEqual(bundled, conformance)
        XCTAssertEqual(sha256Hex(bundled), try provenanceRootSHA256())

        let manifest = try JSONDecoder().decode(
            Manifest.self,
            from: Data(contentsOf: resources.srf39Directory.appendingPathComponent("srf39-manifest.json"))
        )
        let entry = try XCTUnwrap(manifest.idls.first { $0.programId == programId })
        XCTAssertEqual(entry.sha256, try provenanceRootSHA256())
        XCTAssertEqual(entry.file, "subscriptions-root.json")
    }

    @MainActor
    func testPrimaryFlowDefaultsToDevnetAndPreservesEveryInstruction() async throws {
        let resources = try InspectorResources(bundle: .main)
        let model = InspectorViewModel(resources: resources, rpc: { _ in nil })
        let expectedIds = ["init-subscription-authority", "create-recurring-delegation", "revoke-delegation"]
        XCTAssertEqual(model.selectedCluster, "devnet")
        XCTAssertEqual(model.primaryExamples.map(\.id), expectedIds)
        XCTAssertEqual(model.additionalExamples.count, 4)
        XCTAssertTrue(Set(model.primaryExamples.map(\.id)).isDisjoint(with: model.additionalExamples.map(\.id)))
        XCTAssertEqual(Set((model.primaryExamples + model.additionalExamples).map(\.id)), Set(model.examples.map(\.id)))

        model.loadInitialExample()
        XCTAssertEqual(model.selectedExampleId, expectedIds[0])
        for example in model.primaryExamples {
            model.select(example)
            for _ in 0 ..< 100 where model.isLoading {
                try await Task.sleep(nanoseconds: 10_000_000)
            }
            XCTAssertFalse(model.isLoading)
            XCTAssertNil(model.errorMessage)
            let inspection = try XCTUnwrap(model.inspection)
            XCTAssertEqual(inspection.primaryInstructions.map(\.id), [example.instructionIndex])
            XCTAssertEqual(inspection.primaryInstructions.count + inspection.otherInstructions.count, inspection.instructions.count)
            XCTAssertEqual(
                Set((inspection.primaryInstructions + inspection.otherInstructions).map(\.id)),
                Set(inspection.instructions.map(\.id))
            )
            guard case .rendered = inspection.primaryInstructions.first?.result else {
                return XCTFail("The main preview must render for \(example.id)")
            }
        }
    }

    @MainActor
    func testViewModelRendersDevnetExampleFromSnapshotsWithoutNetwork() async throws {
        let resources = try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources"))
        let model = InspectorViewModel(resources: resources, rpc: { _ in nil })
        let example = try XCTUnwrap(model.primaryExamples.first)

        model.select(example)
        for _ in 0 ..< 100 where model.isLoading {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(model.inspection?.cluster, "devnet")
        XCTAssertEqual(model.inspection?.transactionSource, .snapshot)
        XCTAssertEqual(model.selectedCluster, "devnet")
        let target = try XCTUnwrap(model.inspection?.instructions.first { $0.id == example.instructionIndex })
        guard case .rendered = target.result else {
            return XCTFail("expected the target instruction to render, got \(target.result)")
        }
    }

    func testPreviewPreservesAllCanonicalTimeValues() async throws {
        let (repository, client) = try makeClient()
        var checkedValues: Set<String> = []
        for example in repository.catalog.examples {
            let rendered = try await render(example, repository: repository, client: client)
            let preview = InstructionPreview(rendered: rendered)
            for hint in rendered.hints.times {
                let index = try XCTUnwrap(hint.fieldIndex)
                let value = rendered.canonical.fields[index].value
                XCTAssertEqual(preview.fields[index].value, value, example.instruction)
                checkedValues.insert(value)
            }
        }
        XCTAssertTrue(checkedValues.contains("00:00:30"), "Duration must retain the engine's format")
        XCTAssertTrue(checkedValues.contains("1970-01-01T00:00:00.000Z"), "Zero timestamps stay canonical")
        XCTAssertTrue(checkedValues.contains { $0.hasPrefix("2026-") && $0.hasSuffix("Z") },
                      "Non-zero timestamps must also stay in UTC")
    }

    // MARK: - Preview

    func testDemoAddressAliasesStayOutsideCanonicalDisplay() async throws {
        let (repository, labelledClient) = try makeClient()
        let resources = try InspectorResources(bundle: .main)
        let plainClient = SolanaClearSigningClient(
            idlSource: resources.idlSource,
            accountDataProvider: InspectorAccountProvider(rpc: nil, snapshots: repository.accountSnapshots)
        )
        let expectedAliases: [String: [String: String]] = [
            "initSubscriptionAuthority": ["Your token account": "Demo setup account"],
            "createFixedDelegation": [:], // This captured spender has no local demo alias.
            "createRecurringDelegation": ["Authorized spender": "Demo billing service"],
            "revokeDelegation": ["Delegation account": "Demo allowance to revoke"],
            "subscribe": ["Merchant": "Demo plan provider"],
            "revokeSubscriptionAuthority": ["Your token account": "Demo account to disconnect"],
            "transferRecurring": [
                "Recipient token account": "Demo merchant account",
                "Source token account": "Demo customer account",
                "Authorized spender": "Demo payment collector",
            ],
        ]
        for example in repository.catalog.examples {
            let labelled = try await render(example, repository: repository, client: labelledClient)
            let plain = try await render(example, repository: repository, client: plainClient)
            XCTAssertEqual(labelled.canonical, plain.canonical, example.instruction)
            XCTAssertEqual(labelled.hints, plain.hints, example.instruction)
            XCTAssertEqual(labelled.idl, plain.idl, example.instruction)

            let fields = InstructionPreview(rendered: labelled).fields
            let aliases = Dictionary(uniqueKeysWithValues: fields.compactMap { field -> (String, String)? in
                guard let alias = field.addressLabel, alias.source == "local demo" else { return nil }
                XCTAssertEqual(field.address, labelled.canonical.fields[field.id].value)
                XCTAssertNotEqual(field.value, alias.label, "The address remains separately available")
                return (field.label, alias.label)
            })
            XCTAssertEqual(aliases, try XCTUnwrap(expectedAliases[example.instruction]), example.instruction)
            XCTAssertTrue(InstructionPreview(rendered: plain).fields.allSatisfy { $0.addressLabel == nil })
            XCTAssertTrue(fields.filter(\.isAmount).allSatisfy { $0.addressLabel == nil })
            if example.instruction == "createFixedDelegation" {
                XCTAssertNotEqual(fields.first { $0.label == "Authorized spender" }?.address,
                                  "11111111111111111111111111111111")
                XCTAssertNil(fields.first { $0.label == "Authorized spender" }?.addressLabel)
            }
        }
        let registry = try XCTUnwrap(resources.registry(for: "devnet"))
        let unknown = await registry.addressLabel(for: "SysvarC1ock11111111111111111111111111111111")
        XCTAssertNil(unknown, "Unmapped addresses must not receive a demo identity")
    }

    func testPreviewsUseIdlLabelsOrderAndSentencesForEveryExample() async throws {
        let (repository, client) = try makeClient()
        for example in repository.catalog.examples {
            let rendered = try await render(example, repository: repository, client: client)
            let preview = InstructionPreview(rendered: rendered)
            XCTAssertEqual(preview.fields.map(\.label), rendered.canonical.fields.map(\.label), example.instruction)
            XCTAssertEqual(preview.fields.map(\.id), Array(rendered.canonical.fields.indices), example.instruction)
            XCTAssertEqual(preview.explanation, rendered.canonical.interpolatedIntent, example.instruction)
            XCTAssertEqual(preview.diagnostics, rendered.diagnostics, example.instruction)
            for field in preview.fields {
                let canonical = rendered.canonical.fields[field.id]
                if let address = field.address {
                    XCTAssertEqual(address, canonical.value, "Copy and accessibility must retain the full address")
                    XCTAssertEqual(field.value, "\(address.prefix(6))…\(address.suffix(6))")
                }
                if field.address == nil {
                    XCTAssertEqual(field.value, rendered.presentation.tokenAmount(for: field.id)?.value ?? canonical.value,
                                   "Swift must not reformat SDK values")
                }
                if field.isAmount {
                    let sdkAmount = rendered.presentation.tokenAmount(for: field.id)
                    XCTAssertEqual(field.value, sdkAmount?.value ?? canonical.value, "The preview must use the SDK value")
                    XCTAssertEqual(field.isRaw, sdkAmount.map { $0.decimals == nil } ?? canonical.value.hasSuffix(" (raw)"))
                }
            }
            if example.instruction == "subscribe" {
                let period = try XCTUnwrap(preview.fields.first { $0.label == "Billing period (hours)" })
                XCTAssertEqual(period.value, "720", "Do not infer units from an instruction or field name")
            }
            for hint in rendered.hints.times where hint.rawValue == "0" {
                let index = try XCTUnwrap(hint.fieldIndex)
                XCTAssertEqual(preview.fields[index].value, rendered.canonical.fields[index].value,
                               "Zero-date meanings must not be inferred in Swift")
            }
        }
    }

    func testPreviewFollowsChangedDisplayMetadataWithoutProgramSpecificRules() async throws {
        let (repository, _) = try makeClient()
        let example = try XCTUnwrap(repository.catalog.examples.first { $0.instruction == "createFixedDelegation" })
        let transaction = try TransactionParser.parse(repository.transaction(for: example))
        let original = try XCTUnwrap(transaction.instructions.first { $0.id == example.instructionIndex }).input
        let root = try String(contentsOf: TestPaths.fixtureDirectory("subscriptions").appendingPathComponent("root.json"), encoding: .utf8)
        let foreignProgram = "11111111111111111111111111111111"
        let changed = root.replacingOccurrences(of: "Total allowance", with: "Display-defined cap")
            .replacingOccurrences(
                of: "Allow ${accounts.delegatee} to withdraw up to ${data.amount} in total — revocable anytime",
                with: "Description from the display IDL"
            )
        for (json, program, pinned) in [
            (changed, programId, false),
            (changed, programId, true),
            (changed.replacingOccurrences(of: programId, with: foreignProgram), foreignProgram, true),
        ] {
            let source = PreviewIdlSource(document: .init(
                programId: program, json: json,
                provenance: .init(sourceId: "preview-test", origin: .other("test"),
                                  expectedSHA256: pinned ? sha256Hex(Data(json.utf8)) : nil)
            ))
            let client = SolanaClearSigningClient(idlSource: source)
            let outcome = try await client.render(instruction: .init(
                programId: program, instructionData: original.instructionData, accounts: original.accounts
            ))
            guard case let .rendered(rendered) = outcome else { return XCTFail("Expected a generic display") }
            let preview = InstructionPreview(rendered: rendered)
            XCTAssertEqual(preview.explanation, rendered.canonical.interpolatedIntent)
            XCTAssertEqual(preview.explanation, "Description from the display IDL")
            XCTAssertTrue(preview.fields.contains { $0.label == "Display-defined cap" })
            XCTAssertEqual(preview.fields.map(\.id), Array(rendered.canonical.fields.indices))
        }
    }

    // MARK: - Helpers

    private func makeClient() throws -> (ExampleRepository, SolanaClearSigningClient) {
        let resources = try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources"))
        let repository = try XCTUnwrap(resources.repository(for: "devnet"))
        let provider = InspectorAccountProvider(rpc: nil, snapshots: repository.accountSnapshots)
        let client = SolanaClearSigningClient(
            idlSource: resources.idlSource,
            accountDataProvider: provider,
            presentationProvider: resources.registry(for: "devnet")
        )
        return (repository, client)
    }

    private func render(
        _ example: CatalogExample,
        repository: ExampleRepository,
        client: SolanaClearSigningClient,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws -> SolanaRenderedInstruction {
        let parsed = try TransactionParser.parse(repository.transaction(for: example))
        let target = try XCTUnwrap(
            parsed.instructions.first { $0.id == example.instructionIndex },
            "\(example.instruction): target instruction is missing", file: file, line: line
        )
        let outcome = try await client.render(instruction: target.input)
        let rendered: SolanaRenderedInstruction?
        switch outcome {
        case let .rendered(value): rendered = value
        case .unsupported: rendered = nil
        }
        return try XCTUnwrap(
            rendered, "\(example.instruction): expected rendered, got \(outcome)", file: file, line: line
        )
    }

    private func provenanceRootSHA256() throws -> String {
        let data = try Data(contentsOf: TestPaths.fixtureDirectory("subscriptions").appendingPathComponent("provenance.json"))
        return try JSONDecoder().decode(Provenance.self, from: data).rootSha256
    }

    private func sha256Hex(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}

private struct Provenance: Decodable {
    let rootSha256: String
}

private final class PreviewIdlSource: Srf39IdlSource, Sendable {
    let document: ResolvedSrf39Idl

    init(document: ResolvedSrf39Idl) { self.document = document }

    func idl(for programId: String) async throws -> ResolvedSrf39Idl? {
        programId == document.programId ? document : nil
    }
}

private struct Manifest: Decodable {
    let idls: [Entry]

    struct Entry: Decodable {
        let programId: String
        let file: String
        let sha256: String?
    }
}
