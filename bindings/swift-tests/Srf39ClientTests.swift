import Foundation
import XCTest
@testable import SolanaClearsign

final class Srf39ClientTests: XCTestCase {
    func testRealTransferCheckedRendersAcrossSwiftAndUniFFI() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-online" })
        let provider = FixtureAccountProvider(accounts: fixture.accountData)
        let client = try SolanaClearSigningClient(idlJSON: try fixtureRoot())

        let display = try await client.display(
            instruction: input(fixture: fixture, scenario: scenario),
            accountProvider: provider
        )

        XCTAssertEqual(display?.intent, "Transfer Tokens")
        XCTAssertEqual(display?.fields.map(\.label), ["Amount", "Token Mint", "To"])
        XCTAssertEqual(display?.fields.first?.value, "0.001469")
        let callCount = await provider.callCount
        XCTAssertEqual(callCount, 1)
    }

    func testMissingProviderDegradesToRawAmount() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-offline" })
        let client = try SolanaClearSigningClient(idlJSON: try fixtureRoot())

        let display = try await client.display(instruction: input(fixture: fixture, scenario: scenario))

        XCTAssertNil(display?.interpolatedIntent)
        XCTAssertEqual(display?.fields.first?.value, "1469 (raw)")
    }

    func testUnknownProgramReturnsNil() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first)
        let source = input(fixture: fixture, scenario: scenario)
        let client = try SolanaClearSigningClient(idlJSON: try fixtureRoot())
        let unknown = SolanaInstructionInput(
            programId: "11111111111111111111111111111111",
            instructionData: source.instructionData,
            accounts: source.accounts
        )

        let display = try await client.display(instruction: unknown)
        XCTAssertNil(display)
    }

    func testMalformedLinkedAccountMapsToAccountDecode() async throws {
        let fixture = try loadFixture()
        let scenario = try XCTUnwrap(fixture.scenarios.first { $0.name == "transferChecked-online" })
        let mint = scenario.accounts[1].address
        let provider = FixtureAccountProvider(accounts: [
            mint: FixtureAccountData(
                programAddress: fixture.programAddress,
                dataBase64: Data(repeating: 0, count: 81).base64EncodedString()
            ),
        ])
        let client = try SolanaClearSigningClient(idlJSON: try fixtureRoot())

        do {
            _ = try await client.display(
                instruction: input(fixture: fixture, scenario: scenario),
                accountProvider: provider
            )
            XCTFail("Expected accountDecode")
        } catch let error as SolanaClearSigningError {
            guard case let .accountDecode(account, _) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertEqual(account, "mint")
        }
    }

    func testInvalidIdlErrorDoesNotLeakFfiType() {
        XCTAssertThrowsError(try SolanaClearSigningClient(idlJSON: "{")) { error in
            guard let error = error as? SolanaClearSigningError,
                  case .invalidJson = error
            else {
                return XCTFail("Unexpected error: \(error)")
            }
        }
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
