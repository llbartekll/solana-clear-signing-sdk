import Foundation
import XCTest
@testable import SolanaClearsign

/// The decorated Subscriptions IDL (`conformance/fixtures/subscriptions`) rendered
/// through both Swift entry points: every oracle scenario through the low-level
/// client, and the high-level client for the hints and overlay the demo relies on.
final class SubscriptionsFixtureTests: XCTestCase {
    private let programId = "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44"
    private let usdcDevnet = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"

    func testEveryScenarioMatchesTheOracleThroughTheLowLevelClient() async throws {
        let fixture = try loadFixture()
        let expected = try loadExpected()
        let client = try SolanaClearSigningClient(idlJSON: try fixtureRoot())
        var checked = 0
        for scenario in fixture.scenarios {
            let provider = scenario.fetchAccounts == true
                ? FixtureAccountProvider(accounts: scenario.accountData ?? fixture.accountData)
                : nil
            let want = try XCTUnwrap(expected[scenario.name], scenario.name)
            XCTAssertEqual(want?.error, scenario.expectedError, scenario.name)
            do {
                let display = try await client.display(
                    instruction: input(fixture: fixture, scenario: scenario),
                    accountProvider: provider
                )
                XCTAssertNil(scenario.expectedError, "Expected a failure for \(scenario.name)")
                XCTAssertEqual(display, want?.display, scenario.name)
            } catch let error as SolanaClearSigningError {
                guard case let .accountDecode(account, _) = error else { throw error }
                XCTAssertEqual(scenario.expectedError, "accountDecode", scenario.name)
                XCTAssertEqual(account, "tokenMint")
            }
            checked += 1
        }
        XCTAssertEqual(checked, fixture.scenarios.count)
        XCTAssertGreaterThan(checked, 0)
    }

    func testSubscribeUsesCallbackDecimalsAndPreservesTheCanonicalResult() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "subscribe-usdc" })
        let client = SolanaClearSigningClient(
            idlSource: try bundledSource(),
            presentationProvider: InMemoryPresentation(tokens: [usdcDevnet: usdcMetadata])
        )

        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("Expected a rendered instruction, got \(outcome)")
        }

        XCTAssertEqual(rendered.idl.programName, "subscriptions")
        XCTAssertTrue(rendered.idl.digestPinned)
        XCTAssertEqual(rendered.idl.sha256, try rootSHA256())
        XCTAssertEqual(rendered.canonical.fields[1].label, "Token mint")
        XCTAssertEqual(rendered.canonical.fields[2].value, "9990000 (raw)")
        XCTAssertNil(rendered.canonical.interpolatedIntent)
        XCTAssertEqual(rendered.hints.amounts.map(\.decimals), [.unsatisfied])
        XCTAssertEqual(rendered.hints.publicKeyArguments.map(\.address), [usdcDevnet])
        XCTAssertEqual(rendered.hints.publicKeyArguments.first?.fieldIndex, 1)
        XCTAssertEqual(rendered.hints.times.first?.seconds, 1_757_000_000)
        XCTAssertEqual(rendered.hints.linkedAccountReads, [])
        XCTAssertEqual(rendered.hints.amounts.first?.token?.mint, usdcDevnet)
        let amount = try XCTUnwrap(rendered.presentation.tokenAmount(for: 2))
        XCTAssertEqual(amount.value, "9.99")
        XCTAssertEqual(amount.decimals, 6)
        XCTAssertEqual(amount.mint, usdcDevnet)
        XCTAssertEqual(rendered.presentation.annotations(for: 2).map(\.kind), [
            .tokenAmount(symbol: "USDC", name: "USD Coin", sourceAddress: usdcDevnet, appliesToValue: true)
        ])
        XCTAssertEqual(
            rendered.presentation.annotations(for: 1).map(\.kind),
            [.tokenMint(symbol: "USDC", name: "USD Coin")]
        )
        XCTAssertEqual(
            rendered.diagnostics.map(\.code),
            ["interpolated_intent_unavailable"]
        )
    }

    func testRecurringDelegationExposesTimeHintsNextToTheDegradedAmount() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "createRecurringDelegation-vendor-bytes" })
        let client = SolanaClearSigningClient(
            idlSource: try bundledSource(),
            accountDataProvider: FixtureAccountProvider(accounts: fixture.accountData)
        )

        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("Expected a rendered instruction, got \(outcome)")
        }

        XCTAssertEqual(rendered.canonical.intent, "Authorize Recurring Spending")
        XCTAssertEqual(rendered.canonical.fields[0].value, "5000000 (raw)")
        XCTAssertEqual(rendered.canonical.fields[1].value, "336:00:00")
        XCTAssertTrue(rendered.hints.interpolatedIntentSuppressed)
        XCTAssertEqual(rendered.hints.times.count, 3)
        XCTAssertEqual(rendered.hints.times[0].display, .duration(ticksPerSecond: 1))
        XCTAssertEqual(rendered.hints.times[0].seconds, 1_209_600)
        XCTAssertTrue(rendered.hints.times[0].formatted)
        XCTAssertEqual(rendered.hints.times[1].display, .dateTime(ticksPerSecond: 1))
        XCTAssertEqual(rendered.hints.times[1].seconds, 0)
        XCTAssertEqual(rendered.hints.linkedAccountReads, [], "nothing is fetched: the mint is out of reach")
    }

    func testCapturedPaymentUsesMintDecimalsAndDoesNotScaleFromTheRegistry() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferRecurring-devnet" })
        for hasMintData in [true, false] {
            let client = SolanaClearSigningClient(
                idlSource: try bundledSource(),
                accountDataProvider: hasMintData
                    ? FixtureAccountProvider(accounts: scenario.accountData ?? fixture.accountData) : nil,
                presentationProvider: InMemoryPresentation(tokens: [usdcDevnet: usdcMetadata])
            )
            let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
            guard case let .rendered(rendered) = outcome else {
                return XCTFail("Expected captured payment, got \(outcome)")
            }
            XCTAssertEqual(rendered.canonical.fields[0].value, hasMintData ? "0.1" : "100000 (raw)")
            XCTAssertEqual(rendered.hints.amounts.first?.rawValue, "100000")
            XCTAssertEqual(rendered.hints.amounts.first?.degraded, !hasMintData)
            guard case let .accountField(account, address, _, path, decimals) = rendered.hints.amounts.first?.decimals else {
                return XCTFail("Expected the IDL to link tokenMint.decimals")
            }
            XCTAssertEqual(account, "tokenMint")
            XCTAssertEqual(address, usdcDevnet)
            XCTAssertEqual(path, "decimals")
            XCTAssertEqual(decimals, hasMintData ? 6 : nil)
            let annotations = rendered.presentation.annotations(for: 0)
            guard case let .tokenAmount(symbol, _, source, appliesToValue)? = annotations.first?.kind else {
                return XCTFail("Expected token metadata alongside the amount")
            }
            XCTAssertEqual(symbol, "USDC")
            XCTAssertEqual(source, usdcDevnet)
            XCTAssertEqual(appliesToValue, hasMintData)
            XCTAssertEqual(rendered.diagnostics.map(\.code), hasMintData ? [] : [
                "linked_account_unavailable", "amount_scale_unresolved",
            ])
        }
    }

    func testRevokeRendersOfflineAndKeepsAddressLabelsSeparate() async throws {
        let fixture = try loadFixture()
        let expected = try loadExpected()
        let delegation = fixture.accounts[2].address
        let client = SolanaClearSigningClient(
            idlSource: try bundledSource(),
            presentationProvider: InMemoryPresentation(
                labels: [delegation: .init(label: "Streaming subscription", source: "contacts")]
            )
        )

        for name in ["revokeDelegation", "revokeDelegation-with-receiver"] {
            let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == name })
            let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
            guard case let .rendered(rendered) = outcome else {
                return XCTFail("Expected a rendered instruction for \(name), got \(outcome)")
            }
            let want = try XCTUnwrap(expected[name])
            XCTAssertEqual(rendered.canonical, want?.display, name)
            XCTAssertEqual(rendered.canonical.fields, [.init(label: "Delegation account", value: delegation)])
            XCTAssertTrue(try XCTUnwrap(rendered.canonical.interpolatedIntent).contains(delegation))
            XCTAssertFalse(rendered.hints.interpolatedIntentSuppressed)
            XCTAssertEqual(rendered.hints.linkedAccountReads, [])
            XCTAssertEqual(rendered.diagnostics, [])
            XCTAssertEqual(
                rendered.presentation.annotations(for: 0).map(\.kind),
                [.addressLabel(label: "Streaming subscription", source: "contacts")]
            )
        }
    }

    func testAuthorityActionsKeepTheirMeaningWithoutTokenMetadata() async throws {
        let fixture = try loadFixture()
        let expected = try loadExpected()
        let offlineClient = SolanaClearSigningClient(idlSource: try bundledSource())
        let labelledClient = SolanaClearSigningClient(
            idlSource: try bundledSource(),
            presentationProvider: InMemoryPresentation(tokens: [usdcDevnet: usdcMetadata])
        )

        for name in ["initSubscriptionAuthority", "revokeSubscriptionAuthority"] {
            let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == name })
            let instruction = try input(fixture: fixture, scenario: scenario)
            let offline = try await offlineClient.render(instruction: instruction)
            let labelled = try await labelledClient.render(instruction: instruction)
            guard case let .rendered(plain) = offline, case let .rendered(enriched) = labelled else {
                return XCTFail("Expected authority actions to render without an account provider: \(name)")
            }
            let want = try XCTUnwrap(expected[name])
            XCTAssertEqual(plain.canonical, want?.display, name)
            XCTAssertEqual(plain.canonical, enriched.canonical)
            XCTAssertEqual(plain.presentation.annotations, [])
            XCTAssertEqual(plain.diagnostics, [])
            XCTAssertEqual(plain.hints.linkedAccountReads, [])
            let mintField = try XCTUnwrap(enriched.hints.accounts.first { $0.name == "tokenMint" }?.fieldIndex)
            XCTAssertEqual(
                enriched.presentation.annotations(for: mintField).map(\.kind),
                [.tokenMint(symbol: "USDC", name: "USD Coin")]
            )
            if name == "initSubscriptionAuthority" {
                XCTAssertTrue(plain.canonical.intent.contains("Unlimited Allowance"))
                XCTAssertTrue(try XCTUnwrap(plain.canonical.interpolatedIntent).contains("maximum token allowance"))
            }
        }
    }

    func testMissingDelegationAccountSuppressesTheSentence() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "revokeDelegation-missing-account" })
        let client = SolanaClearSigningClient(idlSource: try bundledSource())
        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("Expected a partial display, got \(outcome)")
        }
        XCTAssertEqual(rendered.canonical.intent, "Revoke a Delegation")
        XCTAssertEqual(rendered.canonical.fields, [])
        XCTAssertNil(rendered.canonical.interpolatedIntent)
        XCTAssertTrue(rendered.hints.interpolatedIntentSuppressed)
        XCTAssertEqual(rendered.diagnostics.map(\.code), ["interpolated_intent_unavailable"])
    }

    func testSubscribeMetadataCallbackControlsScaleAndMissingDecimalsStayRaw() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "subscribe-usdc" })
        for (decimals, expected) in [(UInt8(9), "0.00999"), (nil, "9990000 (raw)")] {
            let metadata = SolanaTokenMetadata(symbol: "USDC", decimals: decimals)
            let client = SolanaClearSigningClient(
                idlSource: try bundledSource(),
                presentationProvider: InMemoryPresentation(tokens: [usdcDevnet: metadata])
            )
            guard case let .rendered(rendered) = try await client.render(instruction: input(fixture: fixture, scenario: scenario)) else {
                return XCTFail("Expected rendered subscribe")
            }
            let amount = try XCTUnwrap(rendered.presentation.tokenAmount(for: 2))
            XCTAssertEqual(amount.value, expected)
            XCTAssertEqual(amount.decimals, decimals)
            XCTAssertEqual(rendered.canonical.fields[2].value, "9990000 (raw)")
            XCTAssertEqual(rendered.diagnostics.contains { $0.code == "amount_scale_unresolved" }, decimals == nil)
        }
        let noMetadata = SolanaClearSigningClient(idlSource: try bundledSource())
        guard case let .rendered(raw) = try await noMetadata.render(instruction: input(fixture: fixture, scenario: scenario)) else {
            return XCTFail("Expected raw subscribe")
        }
        XCTAssertEqual(raw.presentation.tokenAmount(for: 2)?.value, "9990000 (raw)")
        XCTAssertTrue(raw.diagnostics.contains { $0.code == "token_metadata_not_found" })
    }

    // MARK: - Helpers

    private var usdcMetadata: SolanaTokenMetadata {
        SolanaTokenMetadata(
            symbol: "USDC",
            name: "USD Coin",
            decimals: 6,
            tokenProgram: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        )
    }

    private func bundledSource() throws -> BundledSrf39IdlSource {
        try BundledSrf39IdlSource(
            entries: [.init(programId: programId, fileName: "root.json", sha256: try rootSHA256(), version: "0.1.0")],
            directory: fixtureDirectory,
            sourceId: "test-fixtures"
        )
    }

    private func rootSHA256() throws -> String {
        let data = try Data(contentsOf: fixtureDirectory.appendingPathComponent("provenance.json"))
        let provenance = try JSONDecoder().decode(Provenance.self, from: data)
        return provenance.rootSha256
    }

    private func input(fixture: Fixture, scenario: FixtureScenario) throws -> SolanaInstructionInput {
        SolanaInstructionInput(
            programId: scenario.programAddress ?? fixture.programAddress,
            instructionData: try XCTUnwrap(
                Data(base64Encoded: scenario.dataBase64 ?? fixture.dataBase64),
                "Invalid instruction base64 in \(scenario.name)"
            ),
            accounts: (scenario.accounts ?? fixture.accounts).map {
                SolanaAccountMeta(
                    pubkey: $0.address,
                    isSigner: $0.role.lowercased().contains("signer"),
                    isWritable: $0.role.lowercased().contains("writable")
                )
            }
        )
    }

    private func fixtureRoot() throws -> String {
        try String(contentsOf: fixtureDirectory.appendingPathComponent("root.json"), encoding: .utf8)
    }

    private func loadFixture() throws -> Fixture {
        let data = try Data(contentsOf: fixtureDirectory.appendingPathComponent("cases.json"))
        return try JSONDecoder().decode(Fixture.self, from: data)
    }

    private func loadExpected() throws -> [String: ExpectedOutcome?] {
        let data = try Data(contentsOf: fixtureDirectory.appendingPathComponent("expected.json"))
        return try JSONDecoder().decode([String: ExpectedOutcome?].self, from: data)
    }

    private var fixtureDirectory: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("conformance/fixtures/subscriptions")
    }
}

private struct Provenance: Decodable {
    let rootSha256: String
}

private struct Fixture: Decodable {
    let programAddress: String
    let dataBase64: String
    let accounts: [FixtureAccount]
    let accountData: [String: FixtureAccountData]
    let scenarios: [FixtureScenario]
}

private struct FixtureScenario: Decodable {
    let name: String
    let programAddress: String?
    let dataBase64: String?
    let accounts: [FixtureAccount]?
    let accountData: [String: FixtureAccountData]?
    let fetchAccounts: Bool?
    let expectedError: String?
}

private struct FixtureAccount: Decodable {
    let address: String
    let role: String
}

private struct FixtureAccountData: Decodable {
    let programAddress: String
    let dataBase64: String
}

private enum ExpectedOutcome: Decodable {
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

    var error: String? {
        guard case let .failure(error) = self else { return nil }
        return error
    }
}

private struct ExpectedDisplay: Decodable {
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

private struct ExpectedField: Decodable {
    let label: String
    let value: String
}

private actor FixtureAccountProvider: SolanaAccountDataProvider {
    let accounts: [String: FixtureAccountData]

    init(accounts: [String: FixtureAccountData]) {
        self.accounts = accounts
    }

    func accountData(for address: String) -> SolanaAccountData? {
        guard let account = accounts[address], let data = Data(base64Encoded: account.dataBase64) else {
            return nil
        }
        return SolanaAccountData(owner: account.programAddress, data: data)
    }
}

private final class InMemoryPresentation: SolanaPresentationMetadataProvider, Sendable {
    private let tokens: [String: SolanaTokenMetadata]
    private let labels: [String: SolanaAddressLabel]

    init(tokens: [String: SolanaTokenMetadata] = [:], labels: [String: SolanaAddressLabel] = [:]) {
        self.tokens = tokens
        self.labels = labels
    }

    func tokenMetadata(for mint: String) async -> SolanaTokenMetadata? { tokens[mint] }
    func addressLabel(for address: String) async -> SolanaAddressLabel? { labels[address] }
}
