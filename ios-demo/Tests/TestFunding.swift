// Deterministic, PRE-FUNDED devnet test wallet (seed = [42;32], a publicly
// known devnet-only key — never reuse anywhere real). Airdrop faucets are
// IP-rate-limited, so gate tests self-fund from this wallet and only fall
// back to the faucet (then skip) when it somehow runs dry.

import Foundation
import XCTest
import SolanaSwift
@testable import ClearsignDemo

enum TestFunding {
    /// Live integration tests mutate devnet and are intentionally opt-in.
    /// The default test suite must remain deterministic and network-free.
    static func requireDevnetOptIn() throws {
        guard ProcessInfo.processInfo.environment["RUN_DEVNET_TESTS"] == "1" else {
            throw XCTSkip("set RUN_DEVNET_TESTS=1 to run live devnet integration tests")
        }
    }

    /// ed25519 expanded secret = seed || pubkey (TweetNacl layout).
    /// seed = sha256("solana-clearsign-devnet-funder-v1") →
    /// address 7TzvzCkbNe37cpptjFB2A1EvNy75uQA6v1fvKY3mPgJK.
    /// (NOT the well-known [42;32] seed — that address is a token account
    /// on devnet and cannot pay fees.)
    static func fundedSigner() throws -> KeyPair {
        let seed: [UInt8] = [
            0x33, 0xe6, 0x79, 0xbc, 0x4e, 0x2b, 0x1b, 0x37, 0x8a, 0x79, 0xab, 0xab,
            0xe4, 0x32, 0x93, 0x4c, 0xed, 0xf3, 0x2b, 0x48, 0x53, 0xab, 0x45, 0xa1,
            0xe9, 0x18, 0x92, 0x71, 0x9a, 0x47, 0x28, 0x05,
        ]
        let pubkey: [UInt8] = [
            96, 16, 185, 163, 203, 139, 207, 51, 253, 237, 138, 131, 170, 230, 246, 90,
            161, 241, 118, 233, 97, 46, 31, 241, 91, 199, 49, 56, 77, 114, 181, 152,
        ]
        return try KeyPair(secretKey: Data(seed + pubkey))
    }

    /// Ensure the signer can pay for the run (~0.02 SOL of rent + fees).
    static func ensureFunded(_ rpc: DevnetClient, _ signer: KeyPair) async throws {
        let address = signer.publicKey.base58EncodedString
        if await rpc.solBalance(address) >= 50_000_000 { return } // ≥0.05 SOL
        _ = try? await rpc.airdrop(address, lamports: 1_000_000_000)
        for _ in 0 ..< 15 {
            if await rpc.solBalance(address) >= 50_000_000 { return }
            try await Task.sleep(nanoseconds: 1_000_000_000)
        }
        throw XCTSkip("devnet test wallet dry and faucet rate-limited — top up 7TzvzCkbNe37cpptjFB2A1EvNy75uQA6v1fvKY3mPgJK via faucet.solana.com")
    }

    /// Create-ATA only when missing (the fixed wallet may already have one).
    static func ensureAtaInstruction(
        _ rpc: DevnetClient, user: PublicKey, mint: PublicKey
    ) async throws -> (ata: PublicKey, createIx: TransactionInstruction?) {
        let ata = try PublicKey.associatedTokenAddress(
            walletAddress: user, tokenMintAddress: mint, tokenProgramId: TokenProgram.id
        )
        if await rpc.rawAccount(ata.base58EncodedString) != nil {
            return (ata, nil)
        }
        let ix = try AssociatedTokenProgram.createAssociatedTokenAccountInstruction(
            mint: mint, owner: user, payer: user, tokenProgramId: TokenProgram.id
        )
        return (ata, ix)
    }
}
