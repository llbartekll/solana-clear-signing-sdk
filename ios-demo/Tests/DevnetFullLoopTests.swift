// Opt-in devnet gate, end to end: init the authority, build a recurring
// delegation, render it through the sRFC 39 client, land it, decode the
// on-chain state, then revoke. Mutates devnet and needs SOL on the funder.

import XCTest
import SolanaSwift
import SolanaClearsign
@testable import ClearsignDemo

final class DevnetFullLoopTests: XCTestCase {
    func testFullLoop_init_delegate_clearsign_revoke() async throws {
        try TestFunding.requireDevnetOptIn()
        let rpc = DevnetClient()
        let signer = try TestFunding.fundedSigner()
        let user = signer.publicKey
        let mint = SubscriptionsProgram.devnetUsdcMint
        print("▸ wallet:", user.base58EncodedString)
        try await TestFunding.ensureFunded(rpc, signer)

        // -- 1. init authority ((ATA if missing) + init; zero USDC needed) ---
        let (ata, createAta) = try await TestFunding.ensureAtaInstruction(rpc, user: user, mint: mint)
        var initInstructions: [TransactionInstruction] = []
        if let createAta { initInstructions.append(createAta) }
        initInstructions.append(
            try SubscriptionsProgram.initSubscriptionAuthority(user: user, mint: mint, userAta: ata)
        )
        let sigInit = try await rpc.sendAndConfirm(instructions: initInstructions, signer: signer)
        print("✓ init: https://explorer.solana.com/tx/\(sigInit)?cluster=devnet")

        let auth = try SubscriptionsProgram.subscriptionAuthorityPda(user: user, mint: mint)
        guard let authAccount = await rpc.rawAccount(auth.base58EncodedString),
              let initId = SubscriptionsProgram.authorityInitId(accountData: authAccount.data)
        else { return XCTFail("authority PDA unreadable after init") }

        // -- 2. build recurring delegation + render it through sRFC 39 -------
        // `startTs == 0` requires a non-zero expiry on-chain, so start now.
        let delegatee = try KeyPair().publicKey
        let delegationIx = try SubscriptionsProgram.createRecurringDelegation(
            delegator: user, delegatee: delegatee, mint: mint,
            nonce: UInt64.random(in: 1 ..< UInt64.max), // fixed wallet reruns → fresh PDA each time
            amountPerPeriod: 5_000_000, periodLengthSecs: 14 * 86_400,
            startTs: Int64(Date().timeIntervalSince1970), expiryTs: 0, authorityInitId: initId
        )

        let resources = try InspectorResources(resourcesRoot: TestPaths.iosDemo.appendingPathComponent("Resources"))
        let client = SolanaClearSigningClient(
            idlSource: resources.idlSource,
            presentationProvider: resources.registry(for: "devnet")
        )
        let outcome = try await client.render(instruction: delegationIx.clearSigningInput)
        guard case let .rendered(rendered) = outcome else {
            return XCTFail("expected a rendered instruction, got \(outcome)")
        }
        XCTAssertEqual(rendered.canonical.intent, "Authorize Recurring Spending")
        XCTAssertEqual(rendered.canonical.fields.first?.value, "5000000 (raw)")
        XCTAssertEqual(rendered.canonical.fields[1].value, "336:00:00")
        print("✓ rendered:", rendered.canonical.fields.map { "\($0.label): \($0.value)" })

        // -- 3. submit the delegation, decode its on-chain state -------------
        let sigDelegate = try await rpc.sendAndConfirm(instructions: [delegationIx], signer: signer)
        print("✓ delegation: https://explorer.solana.com/tx/\(sigDelegate)?cluster=devnet")

        let delegationPda = delegationIx.keys[2].publicKey
        guard let delegationAccount = await rpc.rawAccount(delegationPda.base58EncodedString),
              let state = SubscriptionsProgram.decodeRecurringDelegation(accountData: delegationAccount.data)
        else { return XCTFail("delegation PDA unreadable") }
        XCTAssertEqual(state.amountPerPeriod, 5_000_000)
        XCTAssertEqual(state.periodLengthSecs, 14 * 86_400)
        XCTAssertEqual(state.expiryTs, 0)
        XCTAssertEqual(state.mint.base58EncodedString, mint.base58EncodedString)

        // -- 4. revoke (closes the PDA) ---------------------------------------
        let sigRevoke = try await rpc.sendAndConfirm(
            instructions: [SubscriptionsProgram.revokeDelegation(authority: user, delegationAccount: delegationPda)],
            signer: signer
        )
        print("✓ revoke: https://explorer.solana.com/tx/\(sigRevoke)?cluster=devnet")
        let closed = await rpc.rawAccount(delegationPda.base58EncodedString)
        XCTAssertNil(closed, "delegation PDA should be closed after revoke")
    }
}
