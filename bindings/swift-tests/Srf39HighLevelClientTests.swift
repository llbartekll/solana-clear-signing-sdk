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

    func testManifestValidationRejectsNonASCIIDigestsAndEscapingFileNames() throws {
        for digest in [String(repeating: "\u{FF41}", count: 64), String(repeating: "g", count: 64)] {
            XCTAssertThrowsError(try BundledSrf39IdlSource(
                entries: [.init(programId: tokenProgram, fileName: "root.json", sha256: digest)],
                directory: fixtureDirectory
            )) { error in
                XCTAssertEqual(
                    error as? BundledSrf39IdlSourceError,
                    .malformedDigest(programId: tokenProgram, digest: digest)
                )
            }
        }
        for name in ["", ".", "..", "../outside.json", "nested/root.json", "/root.json"] {
            XCTAssertThrowsError(try BundledSrf39IdlSource(
                entries: [.init(programId: tokenProgram, fileName: name)],
                directory: fixtureDirectory
            )) { error in
                XCTAssertEqual(
                    error as? BundledSrf39IdlSourceError,
                    .invalidFileName(programId: tokenProgram, fileName: name)
                )
            }
        }
    }

    func testBundledSourceKeepsExactFileBytes() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let bytes: [UInt8] = [0xEF, 0xBB, 0xBF] + Array("{}".utf8)
        try Data(bytes).write(to: directory.appendingPathComponent("bom.json"))
        try Data([0xFF, 0x7B]).write(to: directory.appendingPathComponent("invalid.json"))

        let source = try BundledSrf39IdlSource(
            entries: [
                .init(programId: "bom", fileName: "bom.json"),
                .init(programId: "invalid", fileName: "invalid.json"),
            ],
            directory: directory
        )
        let resolved = try await source.idl(for: "bom")
        XCTAssertEqual(resolved.map { Array($0.json.utf8) }, bytes)
        do {
            _ = try await source.idl(for: "invalid")
            XCTFail("invalid UTF-8 must not be repaired")
        } catch let Srf39IdlSourceError.unavailable(_, retryable) {
            XCTAssertFalse(retryable)
        }
    }

    func testInvalidationKeepsInlineIdl() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-offline" })
        let client = try SolanaClearSigningClient(idlJSON: fixtureRoot())
        client.invalidate(programId: tokenProgram)
        client.invalidateAll()
        let outcome = try await client.render(instruction: input(fixture: fixture, scenario: scenario))
        guard case .rendered = outcome else { return XCTFail("got \(outcome)") }
    }

    func testCancellationDoesNotWaitForAStalledProvider() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-online" })
        let provider = StalledAccountProvider()
        let client = SolanaClearSigningClient(
            idlSource: try bundledSource(sha256: rootSHA256),
            accountDataProvider: provider
        )
        let instruction = input(fixture: fixture, scenario: scenario)
        let task = Task { try await client.render(instruction: instruction) }
        await provider.waitUntilCalled()
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("expected cancellation")
        } catch is CancellationError {}
        await provider.release()
    }

    func testUnsafeDecodedTextReachesTheOverlayEscaped() async throws {
        let program = "11111111111111111111111111111111"
        let json = """
        {"kind":"rootNode","standard":"codama","version":"1.0.0","program":{"kind":"programNode",
        "name":"memo","publicKey":"\(program)","instructions":[{"kind":"instructionNode","name":"note",
        "arguments":[{"kind":"instructionArgumentNode","name":"text","type":{"kind":"fixedSizeTypeNode",
        "size":4,"type":{"kind":"stringTypeNode","encoding":"utf8"}}}]}]}}
        """
        let client = try SolanaClearSigningClient(idlJSON: json)
        let outcome = try await client.render(instruction: SolanaInstructionInput(
            programId: program, instructionData: Data("a\u{202E}".utf8), accounts: []
        ))
        guard case let .rendered(rendered) = outcome else { return XCTFail("got \(outcome)") }
        XCTAssertEqual(rendered.presentation.escapedText(for: 0)?.value, "a\\u{202e}")
        XCTAssertTrue(rendered.diagnostics.contains { $0.code == "unsafe_display_text" })
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

/// Never answers until released.
private actor StalledAccountProvider: SolanaAccountDataProvider {
    private var pending: [CheckedContinuation<Void, Never>] = []
    private var observers: [CheckedContinuation<Void, Never>] = []

    func accountData(for address: String) async -> SolanaAccountData? {
        await withCheckedContinuation { continuation in
            pending.append(continuation)
            observers.forEach { $0.resume() }
            observers = []
        }
        return nil
    }

    func waitUntilCalled() async {
        guard pending.isEmpty else { return }
        await withCheckedContinuation { observers.append($0) }
    }

    func release() {
        pending.forEach { $0.resume() }
        pending = []
    }
}
