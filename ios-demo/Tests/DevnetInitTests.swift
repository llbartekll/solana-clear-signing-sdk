// M4 GATE: InitSubscriptionAuthority lands on devnet, built/signed/submitted
// entirely by the app's own code path (builders + DevnetClient + local key).
// Init is idempotent per (user, mint) — re-running refreshes the approval.

import XCTest
import SolanaSwift
@testable import ClearsignDemo

final class DevnetInitTests: XCTestCase {
    func testInitSubscriptionAuthorityLandsOnDevnet() async throws {
        try TestFunding.requireDevnetOptIn()
        let rpc = DevnetClient()
        let signer = try TestFunding.fundedSigner()
        let user = signer.publicKey
        print("▸ wallet:", user.base58EncodedString)
        try await TestFunding.ensureFunded(rpc, signer)

        // Build: (ATA if missing) + init authority. Zero USDC needed.
        let mint = SubscriptionsProgram.devnetUsdcMint
        let (ata, createAta) = try await TestFunding.ensureAtaInstruction(rpc, user: user, mint: mint)
        var instructions: [TransactionInstruction] = []
        if let createAta { instructions.append(createAta) }
        instructions.append(
            try SubscriptionsProgram.initSubscriptionAuthority(user: user, mint: mint, userAta: ata)
        )

        // Sign locally, submit, confirm.
        let signature = try await rpc.sendAndConfirm(instructions: instructions, signer: signer)
        print("✓ init landed: https://explorer.solana.com/tx/\(signature)?cluster=devnet")

        // Verify on-chain effects: authority PDA exists, program-owned, init_id parses.
        let auth = try SubscriptionsProgram.subscriptionAuthorityPda(user: user, mint: mint)
        let account = await rpc.rawAccount(auth.base58EncodedString)
        XCTAssertNotNil(account, "SubscriptionAuthority PDA missing after init")
        XCTAssertEqual(account?.owner, SubscriptionsProgram.id.base58EncodedString)
        XCTAssertNotNil(SubscriptionsProgram.authorityInitId(accountData: account!.data))
    }
}
