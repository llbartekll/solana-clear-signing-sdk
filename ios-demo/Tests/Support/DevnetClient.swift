// Thin devnet RPC wrapper over SolanaSwift's JSONRPCAPIClient.
// All host I/O lives here (and in DemoProvider) — never in the Rust core.

import Foundation
import SolanaSwift

actor DevnetClient {
    let api: JSONRPCAPIClient

    init(url: String = "https://api.devnet.solana.com") {
        api = JSONRPCAPIClient(endpoint: .init(address: url, network: .devnet))
    }

    /// Raw account bytes + owner — the exact shape `resolve_account` needs.
    /// NOTE: `BufferInfo<Data>` does NOT work here — Foundation's own
    /// `Data: Decodable` wins over the BufferLayout protocol-extension
    /// decoder and can't parse the ["<b64>","base64"] tuple. A dedicated
    /// BufferLayout wrapper gets the protocol decoder as intended.
    func rawAccount(_ address: String) async -> (owner: String, data: Data)? {
        do {
            let info: BufferInfo<RawAccountData>? = try await api.getAccountInfo(account: address)
            return info.map { ($0.owner, $0.data.bytes) }
        } catch {
            // Missing account or transient error → a provider miss (the SDK
            // degrades honestly). Logged because silent misses hide RPC bugs.
            print("rawAccount(\(address)) miss:", error)
            return nil
        }
    }

    func solBalance(_ address: String) async -> UInt64 {
        (try? await api.getBalance(account: address, commitment: "confirmed")) ?? 0
    }

    func airdrop(_ address: String, lamports: UInt64) async throws -> String {
        try await api.requestAirdrop(account: address, lamports: lamports)
    }

    /// Build → sign locally → submit → poll to confirmation.
    func sendAndConfirm(instructions: [TransactionInstruction], signer: KeyPair) async throws -> String {
        // "finalized" so the default preflight commitment always knows the hash
        let blockhash = try await api.getLatestBlockhash(commitment: "finalized")
        var tx = Transaction(instructions: instructions, recentBlockhash: blockhash, feePayer: signer.publicKey)
        try tx.sign(signers: [signer])
        let wire = try tx.serialize()
        let signature = try await api.sendTransaction(transaction: wire.base64EncodedString())
        try await confirm(signature: signature)
        return signature
    }

    func confirm(signature: String, attempts: Int = 30) async throws {
        for _ in 0 ..< attempts {
            if let status = try? await api.getSignatureStatuses(signatures: [signature]).first ?? nil {
                if status.err != nil {
                    throw DevnetError.transactionFailed(String(describing: status.err))
                }
                if status.confirmationStatus == "confirmed" || status.confirmationStatus == "finalized" {
                    return
                }
            }
            try await Task.sleep(nanoseconds: 1_000_000_000)
        }
        throw DevnetError.confirmationTimeout(signature)
    }
}

/// Uninterpreted account bytes, decodable through SolanaSwift's BufferLayout
/// base64 path (unlike Foundation's `Data`, which has a competing Decodable).
struct RawAccountData: BufferLayout {
    let bytes: Data

    init(from reader: inout BinaryReader) throws {
        bytes = try Data(reader.readAll())
    }

    func serialize(to writer: inout Data) throws {
        writer.append(bytes)
    }
}

enum DevnetError: Error, LocalizedError {
    case transactionFailed(String)
    case confirmationTimeout(String)

    var errorDescription: String? {
        switch self {
        case let .transactionFailed(err): return "Transaction failed: \(err)"
        case let .confirmationTimeout(sig): return "Confirmation timeout for \(sig)"
        }
    }
}
