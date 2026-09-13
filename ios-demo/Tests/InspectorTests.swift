import XCTest
import SolanaClearsign
@testable import ClearsignDemo

final class InspectorTests: XCTestCase {
    func testBuiltAppBundleContainsAllOfflineResources() throws {
        let resources = try InspectorResources(bundle: .main)

        XCTAssertEqual(resources.catalogs.map(\.cluster), ["devnet"])
        XCTAssertNil(resources.repository(for: "mainnet-beta"))
        XCTAssertNil(Bundle.main.url(forResource: "catalog", withExtension: "json", subdirectory: "MainnetExamples"))
        XCTAssertNil(Bundle.main.url(forResource: "spl-token-instructions-root", withExtension: "json"))
        XCTAssertNil(Bundle.main.url(forResource: "token-registry-mainnet", withExtension: "json"))
        for repository in resources.catalogs {
            for example in repository.catalog.examples {
                _ = try repository.transaction(for: example)
            }
        }
        XCTAssertNil(resources.registry(for: "mainnet-beta"))
        XCTAssertEqual(resources.registry(for: "devnet")?.cluster, "devnet")
    }

    func testRawAmountMessagesUseResolvedContextAcrossTokenInstructions() async throws {
        let repository = try mainnetRepository()
        let client = SolanaClearSigningClient(
            idlSource: try mainnetIdlSource(programId: repository.catalog.programId)
        )
        var checkedAmounts = 0
        for example in repository.catalog.examples {
            let parsed = try TransactionParser.parse(repository.transaction(for: example))
            let original = parsed.instructions[example.instructionIndex].input
            for includeAccounts in [true, false] {
                let input = SolanaInstructionInput(
                    programId: original.programId,
                    instructionData: original.instructionData,
                    accounts: includeAccounts ? original.accounts : []
                )
                guard case let .rendered(rendered) = try await client.render(instruction: input) else {
                    return XCTFail("Expected a partial display for \(example.instruction)")
                }
                let overlay = FieldOverlay(rendered: rendered)
                if rendered.hints.amounts.isEmpty {
                    XCTAssertEqual(overlay.rawAmountExplanations, [], example.instruction)
                } else {
                    checkedAmounts += 1
                    XCTAssertTrue(overlay.hasUnresolvedScale, example.instruction)
                    XCTAssertEqual(overlay.rawAmountExplanations, [includeAccounts
                        ? "Token decimals are unavailable. Amount is shown in base units (raw)."
                        : "Token could not be resolved. Amount is shown in base units (raw)."
                    ], example.instruction)
                }
            }
        }
        XCTAssertEqual(checkedAmounts, 8, "Four checked token instructions, with and without account metas")
    }

    func testHighLevelRenderAnnotatesUsdcAndFlagsUnknownMints() async throws {
        let repository = try mainnetRepository()
        let provider = InspectorAccountProvider(rpc: nil, snapshots: repository.accountSnapshots)
        let client = SolanaClearSigningClient(
            idlSource: try mainnetIdlSource(programId: repository.catalog.programId),
            accountDataProvider: provider,
            presentationProvider: try LocalTokenRegistry(url: mainnetDirectory.appendingPathComponent(repository.catalog.tokenRegistryFile))
        )
        let expected = try loadExpectedDisplays(fixture: "spl-token-instructions")

        for example in repository.catalog.examples {
            let transaction = try repository.transaction(for: example)
            let parsed = try TransactionParser.parse(transaction)
            let target = parsed.instructions[example.instructionIndex]
            let outcome = try await client.render(instruction: target.input)
            guard case let .rendered(rendered) = outcome else {
                return XCTFail("\(example.instruction): expected rendered, got \(outcome)")
            }
            let expectation = try XCTUnwrap(expected["\(example.instruction)-online"])
            XCTAssertEqual(rendered.canonical, expectation?.display, example.instruction)
            XCTAssertTrue(rendered.idl.digestPinned, example.instruction)
            XCTAssertEqual(
                rendered.idl.sha256,
                "472f41c79165064ba7bd7cd8623d0b730a227b6bfc12faca77dc8e4702b7bdc1",
                example.instruction
            )
            let tokenAnnotations = rendered.presentation.annotations.filter {
                if case .addressLabel = $0.kind { return false }
                return true
            }
            let codes = rendered.diagnostics.map(\.code)
            switch example.instruction {
            case "transferChecked":
                XCTAssertEqual(tokenAnnotations.count, 2, example.instruction)
                guard case let .tokenAmount(symbol, _, _, appliesToValue)? = tokenAnnotations.first?.kind else {
                    return XCTFail("\(example.instruction): expected a token amount annotation")
                }
                XCTAssertEqual(symbol, "USDC")
                XCTAssertTrue(appliesToValue)
                XCTAssertEqual(codes, [], example.instruction)
            case "approveChecked", "mintToChecked", "burnChecked":
                XCTAssertEqual(tokenAnnotations, [], example.instruction)
                XCTAssertEqual(codes, ["token_metadata_not_found"], example.instruction)
                let overlay = FieldOverlay(rendered: rendered)
                let amountField = try XCTUnwrap(rendered.hints.amounts.first?.fieldIndex)
                XCTAssertNotNil(overlay.unknownTokenAddress(at: amountField), example.instruction)
            case "closeAccount", "revoke":
                XCTAssertEqual(tokenAnnotations, [], example.instruction)
                XCTAssertEqual(codes, [], example.instruction)
                XCTAssertEqual(rendered.hints.amounts, [], example.instruction)
            default:
                XCTFail("unexpected example \(example.instruction)")
            }
        }
    }

    func testAllSnapshotsReachTheNativeSdk() async throws {
        let repository = try mainnetRepository()
        let idlJSON = try String(
            contentsOf: TestPaths.fixtureDirectory("spl-token-instructions").appendingPathComponent("root.json"),
            encoding: .utf8
        )
        let client = try SolanaClearSigningClient(idlJSON: idlJSON)
        let provider = InspectorAccountProvider(rpc: nil, snapshots: repository.accountSnapshots)
        let expected = try loadExpectedDisplays(fixture: "spl-token-instructions")

        for example in repository.catalog.examples {
            let transaction = try repository.transaction(for: example)
            let parsed = try TransactionParser.parse(transaction)
            let target = parsed.instructions[example.instructionIndex]
            let display = try await client.display(
                instruction: target.input,
                accountProvider: provider
            )
            let expectation = try XCTUnwrap(expected["\(example.instruction)-online"])
            XCTAssertEqual(display, expectation?.display, example.instruction)
        }
    }

    func testRealV0TransferReconstructsAccountRoles() throws {
        let repository = try mainnetRepository()
        let example = try XCTUnwrap(repository.catalog.examples.first { $0.instruction == "transferChecked" })
        let parsed = try TransactionParser.parse(repository.transaction(for: example))
        let accounts = parsed.instructions[example.instructionIndex].input.accounts

        XCTAssertEqual(parsed.version, "v0")
        XCTAssertEqual(accounts.count, 4)
        XCTAssertEqual(accounts.map(\.isSigner), [false, false, false, true])
        XCTAssertEqual(accounts.map(\.isWritable), [true, false, true, false])
    }

    func testLoadedAddressesKeepOrderFlagsAndDuplicates() throws {
        let transaction = RPCTransaction(
            slot: 1,
            blockTime: nil,
            meta: RPCTransactionMeta(
                err: nil,
                loadedAddresses: RPCLoadedAddresses(
                    writable: ["loaded-write"],
                    readonly: ["loaded-read"]
                ),
                innerInstructions: [RPCInnerInstructionGroup(index: 0)]
            ),
            transaction: RPCTransactionContainer(
                message: RPCMessage(
                    accountKeys: ["payer", "static-write", "static-read", "program"],
                    header: RPCMessageHeader(
                        numReadonlySignedAccounts: 0,
                        numReadonlyUnsignedAccounts: 2,
                        numRequiredSignatures: 1
                    ),
                    instructions: [RPCCompiledInstruction(
                        accounts: [0, 1, 4, 5, 4],
                        data: "1",
                        programIdIndex: 3
                    )]
                ),
                signatures: [validTestSignature]
            ),
            version: .numbered(0)
        )

        let parsed = try TransactionParser.parse(transaction)
        let accounts = parsed.instructions[0].input.accounts

        XCTAssertEqual(accounts.map(\.pubkey), [
            "payer", "static-write", "loaded-write", "loaded-read", "loaded-write",
        ])
        XCTAssertEqual(accounts.map(\.isSigner), [true, false, false, false, false])
        XCTAssertEqual(accounts.map(\.isWritable), [true, true, true, false, true])
        XCTAssertEqual(parsed.instructions.count, 1)
        XCTAssertEqual(parsed.innerInstructionGroupCount, 1)
    }

    func testLegacySignerAndWritableBoundaries() throws {
        let transaction = makeTransaction(
            accountKeys: [
                "writable-signer", "readonly-signer", "writable-unsigned",
                "readonly-unsigned", "program",
            ],
            header: RPCMessageHeader(
                numReadonlySignedAccounts: 1,
                numReadonlyUnsignedAccounts: 2,
                numRequiredSignatures: 2
            ),
            instruction: RPCCompiledInstruction(
                accounts: [0, 1, 2, 3],
                data: "1",
                programIdIndex: 4
            )
        )

        let parsed = try TransactionParser.parse(transaction)
        let accounts = parsed.instructions[0].input.accounts

        XCTAssertEqual(parsed.version, "Legacy")
        XCTAssertEqual(accounts.map(\.isSigner), [true, true, false, false])
        XCTAssertEqual(accounts.map(\.isWritable), [true, false, true, false])
    }

    func testParserRejectsInvalidBase58AndIndexes() {
        XCTAssertThrowsError(try TransactionParser.parse(makeTransaction(
            instruction: RPCCompiledInstruction(accounts: [], data: "0", programIdIndex: 1)
        ))) { error in
            XCTAssertEqual(error as? TransactionParserError, .invalidInstructionData(instruction: 0))
        }

        XCTAssertThrowsError(try TransactionParser.parse(makeTransaction(
            instruction: RPCCompiledInstruction(accounts: [], data: "1", programIdIndex: 2)
        ))) { error in
            XCTAssertEqual(
                error as? TransactionParserError,
                .invalidProgramIndex(instruction: 0, index: 2)
            )
        }

        XCTAssertThrowsError(try TransactionParser.parse(makeTransaction(
            instruction: RPCCompiledInstruction(accounts: [2], data: "1", programIdIndex: 1)
        ))) { error in
            XCTAssertEqual(
                error as? TransactionParserError,
                .invalidAccountIndex(instruction: 0, index: 2)
            )
        }

        XCTAssertThrowsError(try TransactionParser.parse(makeTransaction(
            header: RPCMessageHeader(
                numReadonlySignedAccounts: 0,
                numReadonlyUnsignedAccounts: 0,
                numRequiredSignatures: -1
            ),
            instruction: RPCCompiledInstruction(accounts: [], data: "1", programIdIndex: 1)
        ))) { error in
            XCTAssertEqual(error as? TransactionParserError, .invalidHeader)
        }
    }

    func testAccountProviderCachesLiveLookup() async throws {
        let expected = RPCResolvedAccount(
            owner: "owner",
            data: Data([1, 2, 3]),
            contextSlot: 42
        )
        let rpc = CountingRPC(account: expected)
        let provider = InspectorAccountProvider(rpc: rpc, snapshots: [:])

        let first = await provider.accountData(for: "account")
        let second = await provider.accountData(for: "account")
        let callCount = await rpc.accountCallCount
        let source = await provider.sourceSummary()

        XCTAssertEqual(first, SolanaAccountData(owner: "owner", data: Data([1, 2, 3])))
        XCTAssertEqual(second, first)
        XCTAssertEqual(callCount, 1)
        XCTAssertEqual(source, "Live @ slot 42")
    }

    func testRpcAccountTupleDecodesBase64() throws {
        let json = #"{"context":{"slot":7},"value":{"data":["AQID","base64"],"owner":"owner"}}"#
        let result = try JSONDecoder().decode(RPCAccountResult.self, from: Data(json.utf8))

        XCTAssertEqual(result.context.slot, 7)
        XCTAssertEqual(result.value?.owner, "owner")
        XCTAssertEqual(result.value?.data.bytes, Data([1, 2, 3]))
    }

    func testRpcEndpointsFollowTheCatalogDeclaration() {
        XCTAssertNil(RPCEndpoint.url(for: RPCSpec(kind: "alchemy", endpoint: nil), alchemyAPIKey: nil))
        XCTAssertEqual(
            RPCEndpoint.url(for: RPCSpec(kind: "alchemy", endpoint: nil), alchemyAPIKey: "key")?.host,
            "solana-mainnet.g.alchemy.com"
        )
        XCTAssertEqual(
            RPCEndpoint.url(for: RPCSpec(kind: "public", endpoint: "https://api.devnet.solana.com"), alchemyAPIKey: nil)?
                .absoluteString,
            "https://api.devnet.solana.com"
        )
        XCTAssertNil(RPCEndpoint.url(for: RPCSpec(kind: "unknown", endpoint: nil), alchemyAPIKey: "key"))
    }

    @MainActor
    func testViewModelWorksWithoutRpcUsingBundledSnapshot() async throws {
        let resources = try makeResources()
        let model = InspectorViewModel(resources: resources, rpc: { _ in nil })
        let example = try XCTUnwrap(model.primaryExamples.first)

        model.select(example)
        for _ in 0 ..< 100 where model.isLoading {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(model.inspection?.transactionSource, .snapshot)
        XCTAssertEqual(model.inspection?.cluster, "devnet")
        XCTAssertEqual(model.inspection?.targetInstructionIndex, example.instructionIndex)
        XCTAssertGreaterThan(model.inspection?.renderedCount ?? 0, 0)
        XCTAssertEqual(model.selectedCluster, "devnet")
    }

    func testSignatureValidationRejectsMalformedInput() {
        XCTAssertNil(TransactionParser.validateSignature("not a signature"))
        XCTAssertNotNil(TransactionParser.validateSignature(
            "5ofTS1vSSkWo8uKWh5CPwVmgZcu3no9Lvb1HrEZyzvvqGvBXUHEaGf7PnPu92W4vtM5DrS4rEkuWnyxY2fqC4t3z"
        ))
    }

    @MainActor
    func testViewModelPrefersLiveTransactionAndFallsBackOnRpcFailure() async throws {
        let resources = try makeResources()
        let repository = try XCTUnwrap(resources.repository(for: "devnet"))
        let example = try XCTUnwrap(repository.catalog.examples.first)
        let transaction = try repository.transaction(for: example)
        let cases: [(RPCTransaction?, TransactionDataSource)] = [(transaction, .live), (nil, .snapshot)]

        for (response, expectedSource) in cases {
            let rpc = CountingRPC(account: nil, transaction: response)
            let model = InspectorViewModel(resources: resources, rpc: { _ in rpc })
            model.select(example)
            for _ in 0 ..< 100 where model.isLoading {
                try await Task.sleep(nanoseconds: 10_000_000)
            }

            XCTAssertFalse(model.isLoading)
            XCTAssertNil(model.errorMessage)
            let inspection = try XCTUnwrap(model.inspection)
            XCTAssertEqual(inspection.transactionSource, expectedSource)
            XCTAssertEqual(inspection.transaction.signature, example.signature)
            XCTAssertGreaterThan(inspection.renderedCount, 0)
        }
    }

    @MainActor
    func testViewModelRejectsMismatchedLiveTransactionWithoutSnapshotFallback() async throws {
        let resources = try makeResources()
        let repository = try XCTUnwrap(resources.repository(for: "devnet"))
        let example = try XCTUnwrap(repository.catalog.examples.first { $0.instruction == "createRecurringDelegation" })
        let otherExample = try XCTUnwrap(repository.catalog.examples.first { $0.instruction == "revokeDelegation" })
        let rpc = CountingRPC(account: nil, transaction: try repository.transaction(for: otherExample))
        let model = InspectorViewModel(resources: resources, rpc: { _ in rpc })

        model.select(example)
        for _ in 0 ..< 100 where model.isLoading {
            try await Task.sleep(nanoseconds: 10_000_000)
        }

        XCTAssertFalse(model.isLoading)
        XCTAssertNil(model.inspection)
        XCTAssertEqual(model.errorMessage, InspectorError.fixtureMismatch(example.id).localizedDescription)
    }

    private var mainnetDirectory: URL {
        TestPaths.repositoryRoot.appendingPathComponent("conformance/captures/mainnet")
    }

    private func mainnetRepository() throws -> ExampleRepository {
        try ExampleRepository(resourcesDirectory: mainnetDirectory)
    }

    private func mainnetIdlSource(programId: String) throws -> BundledSrf39IdlSource {
        try BundledSrf39IdlSource(
            entries: [.init(
                programId: programId,
                fileName: "root.json",
                sha256: "472f41c79165064ba7bd7cd8623d0b730a227b6bfc12faca77dc8e4702b7bdc1"
            )],
            directory: TestPaths.fixtureDirectory("spl-token-instructions"),
            sourceId: "test:spl-token"
        )
    }

    private func makeResources() throws -> InspectorResources {
        try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources"))
    }

    private func loadExpectedDisplays(fixture: String) throws -> [String: ExpectedOutcome?] {
        try TestPaths.loadExpectedDisplays(fixture: fixture)
    }

    private func makeTransaction(
        accountKeys: [String] = ["payer", "program"],
        header: RPCMessageHeader = RPCMessageHeader(
            numReadonlySignedAccounts: 0,
            numReadonlyUnsignedAccounts: 1,
            numRequiredSignatures: 1
        ),
        instruction: RPCCompiledInstruction
    ) -> RPCTransaction {
        RPCTransaction(
            slot: 1,
            blockTime: nil,
            meta: RPCTransactionMeta(
                err: nil,
                loadedAddresses: nil,
                innerInstructions: nil
            ),
            transaction: RPCTransactionContainer(
                message: RPCMessage(
                    accountKeys: accountKeys,
                    header: header,
                    instructions: [instruction]
                ),
                signatures: [validTestSignature]
            ),
            version: .legacy
        )
    }
}

/// Repository-tree locations shared by the offline demo tests.
enum TestPaths {
    static var iosDemo: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    }

    static var repositoryRoot: URL {
        iosDemo.deletingLastPathComponent()
    }

    static func fixtureDirectory(_ fixture: String) -> URL {
        repositoryRoot.appendingPathComponent("conformance/fixtures/\(fixture)")
    }

    static func loadExpectedDisplays(fixture: String) throws -> [String: ExpectedOutcome?] {
        let url = fixtureDirectory(fixture).appendingPathComponent("expected.json")
        return try JSONDecoder().decode([String: ExpectedOutcome?].self, from: Data(contentsOf: url))
    }
}

enum ExpectedOutcome: Decodable {
    case rendered(ExpectedDisplay)
    case failure(String)

    private enum CodingKeys: String, CodingKey { case error }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let error = try container.decodeIfPresent(String.self, forKey: .error) {
            self = .failure(error)
        } else {
            self = .rendered(try ExpectedDisplay(from: decoder))
        }
    }

    var display: InstructionDisplay? {
        guard case let .rendered(expected) = self else { return nil }
        return expected.display
    }
}

struct ExpectedDisplay: Decodable {
    let intent: String
    let interpolatedIntent: String?
    let fields: [ExpectedField]

    var display: InstructionDisplay {
        InstructionDisplay(
            intent: intent,
            interpolatedIntent: interpolatedIntent,
            fields: fields.map { DisplayField(label: $0.label, value: $0.value) }
        )
    }
}

struct ExpectedField: Decodable {
    let label: String
    let value: String
}

private actor CountingRPC: SolanaRPCServing {
    private let resolvedAccount: RPCResolvedAccount?
    private let resolvedTransaction: RPCTransaction?
    private(set) var accountCallCount = 0

    init(account: RPCResolvedAccount?, transaction: RPCTransaction? = nil) {
        resolvedAccount = account
        resolvedTransaction = transaction
    }

    func transaction(signature _: String) async throws -> RPCTransaction {
        guard let resolvedTransaction else { throw CountingRPCError.notImplemented }
        return resolvedTransaction
    }

    func account(address _: String) async throws -> RPCResolvedAccount? {
        accountCallCount += 1
        return resolvedAccount
    }
}

private enum CountingRPCError: Error {
    case notImplemented
}

private let validTestSignature =
    "5ofTS1vSSkWo8uKWh5CPwVmgZcu3no9Lvb1HrEZyzvvqGvBXUHEaGf7PnPu92W4vtM5DrS4rEkuWnyxY2fqC4t3z"
