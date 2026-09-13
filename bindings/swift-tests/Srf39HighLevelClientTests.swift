import Foundation
import XCTest
@testable import SolanaClearsign

final class Srf39HighLevelClientTests: XCTestCase {
    private let tokenProgram = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    private let usdc = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
    private let rootSHA256 = "472f41c79165064ba7bd7cd8623d0b730a227b6bfc12faca77dc8e4702b7bdc1"

    func testTransferCheckedRendersWithKnownTokenAndPinnedBinding() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-online" })
        let accounts = FixtureAccountProvider(accounts: fixture.accountData)
        let client = SolanaClearSigningClient(
            idlSource: try bundledSource(sha256: rootSHA256),
            accountDataProvider: accounts,
            presentationProvider: InMemoryPresentation(tokens: [usdc: usdcMetadata])
        )

        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("Expected a rendered instruction, got \(outcome)")
        }

        let expected = try loadExpected()["transferChecked-online"]
        XCTAssertEqual(rendered.canonical, expected?.display)
        XCTAssertEqual(rendered.idl.programId, tokenProgram)
        XCTAssertEqual(rendered.idl.programName, "token")
        XCTAssertEqual(rendered.idl.sha256, rootSHA256)
        XCTAssertTrue(rendered.idl.digestPinned)
        XCTAssertEqual(rendered.idl.provenance.origin, .bundled)
        XCTAssertEqual(
            rendered.hints.amounts.first?.decimals,
            .accountField(account: "mint", address: usdc, linkedAccount: "mint", path: "decimals", resolved: 6)
        )
        XCTAssertFalse(rendered.hints.interpolatedIntentSuppressed)
        XCTAssertEqual(rendered.diagnostics, [])
        XCTAssertEqual(
            rendered.presentation.annotations(for: 0).map(\.kind),
            [.tokenAmount(
                symbol: "USDC",
                name: "USD Coin",
                sourceAddress: usdc,
                appliesToValue: true
            )]
        )
        XCTAssertEqual(
            rendered.presentation.annotations(for: 1).map(\.kind),
            [.tokenMint(symbol: "USDC", name: "USD Coin")]
        )
        let callCount = await accounts.callCount
        XCTAssertEqual(callCount, 1)
    }

    func testOfflineRenderIsDegradedAndReportsDiagnostics() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-offline" })
        let client = SolanaClearSigningClient(
            idlSource: try bundledSource(sha256: rootSHA256),
            presentationProvider: InMemoryPresentation(tokens: [usdc: usdcMetadata])
        )

        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("Expected a rendered instruction, got \(outcome)")
        }

        XCTAssertEqual(rendered.canonical.fields.first?.value, "1469 (raw)")
        XCTAssertNil(rendered.canonical.interpolatedIntent)
        XCTAssertTrue(rendered.hints.interpolatedIntentSuppressed)
        XCTAssertEqual(
            rendered.diagnostics.map(\.code),
            ["linked_account_unavailable", "amount_scale_unresolved", "interpolated_intent_unavailable"]
        )
        XCTAssertEqual(rendered.diagnostics.map(\.severity), [.warning, .warning, .info])
        guard case let .tokenAmount(_, _, _, appliesToValue)? = rendered.presentation.annotations(for: 0).first?.kind else {
            return XCTFail("Expected a token annotation on the amount field")
        }
        XCTAssertFalse(appliesToValue)
    }

    func testUnknownProgramIsUnsupportedAndDisplayReturnsNil() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first)
        let source = input(fixture: fixture, scenario: scenario)
        let unknown = SolanaInstructionInput(
            programId: "11111111111111111111111111111111",
            instructionData: source.instructionData,
            accounts: source.accounts
        )
        let client = SolanaClearSigningClient(idlSource: try bundledSource(sha256: rootSHA256))

        let outcome = try await client.render(instruction: unknown)
        XCTAssertEqual(outcome, .unsupported(.idlNotFound(programId: "11111111111111111111111111111111")))
        let display = try await client.display(instruction: unknown)
        XCTAssertNil(display)
        let required = try await client.requiredAccounts(for: unknown)
        XCTAssertNil(required)
    }

    func testDigestMismatchIsRejected() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first)
        let wrong = "0" + rootSHA256.dropFirst()
        let client = SolanaClearSigningClient(idlSource: try bundledSource(sha256: wrong))

        do {
            _ = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
            XCTFail("Expected idlRejected")
        } catch let error as SolanaClearSigningError {
            XCTAssertEqual(
                error,
                .idlRejected(programId: tokenProgram, reason: .digestMismatch(expected: wrong, actual: rootSHA256))
            )
        }
    }

    func testProgramMismatchIsRejected() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first)
        let other = "11111111111111111111111111111111"
        let source = try BundledSrf39IdlSource(
            entries: [.init(programId: other, fileName: "root.json", sha256: rootSHA256)],
            directory: fixtureDirectory
        )
        let client = SolanaClearSigningClient(idlSource: source)
        let base = input(fixture: fixture, scenario: scenario)
        let instruction = SolanaInstructionInput(
            programId: other,
            instructionData: base.instructionData,
            accounts: base.accounts
        )

        do {
            _ = try await client.render(instruction: instruction)
            XCTFail("Expected idlRejected")
        } catch let error as SolanaClearSigningError {
            XCTAssertEqual(
                error,
                .idlRejected(programId: other, reason: .programMismatch(requested: other, declared: tokenProgram))
            )
        }
    }

    func testThrowingSourceBecomesSourceFailureWithoutCrashing() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first)
        let client = SolanaClearSigningClient(idlSource: ThrowingSource(error: CocoaError(.fileNoSuchFile)))

        do {
            _ = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
            XCTFail("Expected idlSourceFailed")
        } catch let error as SolanaClearSigningError {
            guard case let .idlSourceFailed(_, retryable) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertFalse(retryable)
        }

        let retryable = SolanaClearSigningClient(
            idlSource: ThrowingSource(error: Srf39IdlSourceError.unavailable(detail: "later", retryable: true))
        )
        do {
            _ = try await retryable.render(instruction: input(fixture: fixture, scenario: scenario))
            XCTFail("Expected idlSourceFailed")
        } catch let error as SolanaClearSigningError {
            XCTAssertEqual(error, .idlSourceFailed(detail: "later", retryable: true))
        }
    }

    func testMalformedProgramIdIsInvalidInput() async throws {
        let client = SolanaClearSigningClient(idlSource: try bundledSource(sha256: rootSHA256))
        let instruction = SolanaInstructionInput(programId: "not-base58-0OIl", instructionData: Data(), accounts: [])
        do {
            _ = try await client.display(instruction: instruction)
            XCTFail("Expected invalidInput")
        } catch let error as SolanaClearSigningError {
            guard case .invalidInput = error else { return XCTFail("Unexpected error: \(error)") }
        }
    }

    func testInlineClientRendersUnpinned() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "revoke-online" })
        let client = try SolanaClearSigningClient(idlJSON: try fixtureRoot())

        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("Expected a rendered instruction, got \(outcome)")
        }
        XCTAssertFalse(rendered.idl.digestPinned)
        XCTAssertEqual(rendered.diagnostics.map(\.code), ["idl_digest_unpinned"])
        XCTAssertEqual(rendered.hints.amounts, [])
    }

    func testManifestValidationRejectsBadEntries() throws {
        XCTAssertThrowsError(try BundledSrf39IdlSource(
            entries: [
                .init(programId: tokenProgram, fileName: "root.json"),
                .init(programId: tokenProgram, fileName: "root.json"),
            ],
            directory: fixtureDirectory
        )) { error in
            XCTAssertEqual(error as? BundledSrf39IdlSourceError, .duplicateProgramId(tokenProgram))
        }
        XCTAssertThrowsError(try BundledSrf39IdlSource(
            entries: [.init(programId: tokenProgram, fileName: "root.json", sha256: "abc")],
            directory: fixtureDirectory
        )) { error in
            XCTAssertEqual(error as? BundledSrf39IdlSourceError, .malformedDigest(programId: tokenProgram, digest: "abc"))
        }
    }

    // MARK: - Helpers

    private var usdcMetadata: SolanaTokenMetadata {
        SolanaTokenMetadata(
            symbol: "USDC",
            name: "USD Coin",
            decimals: 6,
            tokenProgram: tokenProgram
        )
    }

    private func bundledSource(sha256: String) throws -> BundledSrf39IdlSource {
        try BundledSrf39IdlSource(
            entries: [.init(programId: tokenProgram, fileName: "root.json", sha256: sha256, version: "3.4.0")],
            directory: fixtureDirectory,
            sourceId: "test-fixtures"
        )
    }

    private func input(fixture: Fixture, scenario: FixtureScenario) -> SolanaInstructionInput {
        SolanaInstructionInput(
            programId: fixture.programAddress,
            instructionData: Data(base64Encoded: scenario.dataBase64) ?? Data(),
            accounts: scenario.accounts.map {
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

    private func loadExpected() throws -> [String: ExpectedDisplay] {
        let data = try Data(contentsOf: fixtureDirectory.appendingPathComponent("expected.json"))
        return try JSONDecoder().decode([String: ExpectedDisplay].self, from: data)
    }

    private var fixtureDirectory: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("conformance/fixtures/spl-token-instructions")
    }
}

private struct Fixture: Decodable {
    let programAddress: String
    let accountData: [String: FixtureAccountData]
    let scenarios: [FixtureScenario]
}

private struct FixtureScenario: Decodable {
    let name: String
    let dataBase64: String
    let accounts: [FixtureAccount]
}

private struct FixtureAccount: Decodable {
    let address: String
    let role: String
}

private struct FixtureAccountData: Decodable {
    let programAddress: String
    let dataBase64: String
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
    private(set) var callCount = 0

    init(accounts: [String: FixtureAccountData]) {
        self.accounts = accounts
    }

    func accountData(for address: String) -> SolanaAccountData? {
        callCount += 1
        guard let account = accounts[address], let data = Data(base64Encoded: account.dataBase64) else {
            return nil
        }
        return SolanaAccountData(owner: account.programAddress, data: data)
    }
}

private final class InMemoryPresentation: SolanaPresentationMetadataProvider, Sendable {
    private let tokens: [String: SolanaTokenMetadata]

    init(tokens: [String: SolanaTokenMetadata]) {
        self.tokens = tokens
    }

    func tokenMetadata(for mint: String) async -> SolanaTokenMetadata? { tokens[mint] }
    func addressLabel(for address: String) async -> SolanaAddressLabel? { nil }
}

private final class ThrowingSource: Srf39IdlSource, @unchecked Sendable {
    private let error: any Error

    init(error: any Error) {
        self.error = error
    }

    func idl(for programId: String) async throws -> ResolvedSrf39Idl? {
        throw error
    }
}
