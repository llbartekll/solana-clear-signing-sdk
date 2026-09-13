// Opt-in devnet check, zero SOL required: build a recurring delegation for a
// REAL on-chain SubscriptionAuthority and render it through the sRFC 39
// client. The canonical output needs no account state (the mint is two hops
// away, so the amount stays raw); the PDA cross-check proves the builder and
// the chain agree on the authority address.

import XCTest
import SolanaSwift
import SolanaClearsign
@testable import ClearsignDemo

final class DevnetRenderE2ETests: XCTestCase {
    /// Real USDC SubscriptionAuthority PDAs observed on devnet (2026-07-03).
    private let candidates = [
        "14JMZjqE8eia4f2gebqaZoPDyucr4gckMWJhNP81mL9h",
        "EhpSx8uTVqpYVrJH5yjHSqco1dXomc4RxGyYc4eehwm",
        "G9KTdnY23HAYsdLMShrpyXv1g2qSiFj3Y6NjZXcAEoX",
    ]

    func testRendersAgainstRealOnChainAuthority() async throws {
        try TestFunding.requireDevnetOptIn()
        let rpc = DevnetClient()
        let mint = SubscriptionsProgram.devnetUsdcMint

        var found: (pda: String, owner: String, data: Data)?
        for candidate in candidates {
            if let account = await rpc.rawAccount(candidate),
               account.data.count == 106, account.data.first == 0 {
                found = (candidate, account.owner, account.data)
                break
            }
        }
        guard let authority = found else {
            throw XCTSkip("no candidate authority account live on devnet — refresh candidates")
        }

        let user = try PublicKey(data: authority.data.subdata(in: 1 ..< 33))
        guard let initId = SubscriptionsProgram.authorityInitId(accountData: authority.data) else {
            return XCTFail("authority init_id unreadable")
        }
        let derived = try SubscriptionsProgram.subscriptionAuthorityPda(user: user, mint: mint)
        XCTAssertEqual(
            derived.base58EncodedString, authority.pda,
            "our PDA seeds don't reproduce the chain-created authority address"
        )

        let delegatee = try KeyPair().publicKey
        let instruction = try SubscriptionsProgram.createRecurringDelegation(
            delegator: user, delegatee: delegatee, mint: mint,
            nonce: 7, amountPerPeriod: 5_000_000, periodLengthSecs: 14 * 86_400,
            startTs: Int64(Date().timeIntervalSince1970), expiryTs: 0, authorityInitId: initId
        )

        let resources = try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources"))
        let client = SolanaClearSigningClient(
            idlSource: resources.idlSource,
            presentationProvider: resources.registry(for: "devnet")
        )
        let outcome = try await client.render(instruction: instruction.clearSigningInput)
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("expected a rendered instruction, got \(outcome)")
        }
        XCTAssertEqual(rendered.canonical.intent, "Authorize Recurring Spending")
        XCTAssertEqual(rendered.canonical.fields.first?.value, "5000000 (raw)")
        XCTAssertEqual(rendered.canonical.fields[1].value, "336:00:00")
        XCTAssertEqual(rendered.canonical.fields.last?.value, delegatee.base58EncodedString)
        XCTAssertEqual(rendered.hints.linkedAccountReads, [])
        XCTAssertEqual(
            rendered.diagnostics.map(\.code),
            ["amount_scale_unresolved", "interpolated_intent_unavailable"]
        )
        XCTAssertEqual(TimePresentation.text(for: rendered.hints.times[0]), "14 days")
    }
}

extension TransactionInstruction {
    /// The SDK input for an instruction built by the test helpers.
    var clearSigningInput: SolanaInstructionInput {
        SolanaInstructionInput(
            programId: programId.base58EncodedString,
            instructionData: Data(data),
            accounts: keys.map {
                SolanaAccountMeta(
                    pubkey: $0.publicKey.base58EncodedString,
                    isSigner: $0.isSigner,
                    isWritable: $0.isWritable
                )
            }
        )
    }
}
