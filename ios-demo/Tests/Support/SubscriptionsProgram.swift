// Instruction builders for the Subscriptions program, ported 1:1 from the
// program source (single-byte discriminator + #[repr(C, packed)] LE payloads)
// and cross-checked against clients/typescript. Account ORDER is load-bearing.

import Foundation
import SolanaSwift

enum SubscriptionsProgram {
    static let id: PublicKey = "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44"
    /// Circle devnet USDC (decimals 6) — the demo's single supported token.
    static let devnetUsdcMint: PublicKey = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"

    // MARK: - PDAs

    /// seeds: ["SubscriptionAuthority", user, token_mint]
    static func subscriptionAuthorityPda(user: PublicKey, mint: PublicKey) throws -> PublicKey {
        try PublicKey.findProgramAddress(
            seeds: [Data("SubscriptionAuthority".utf8), Data(user.bytes), Data(mint.bytes)],
            programId: id
        ).0
    }

    /// seeds: ["delegation", subscription_authority, delegator, delegatee, nonce_le]
    static func delegationPda(
        authority: PublicKey, delegator: PublicKey, delegatee: PublicKey, nonce: UInt64
    ) throws -> PublicKey {
        try PublicKey.findProgramAddress(
            seeds: [
                Data("delegation".utf8), Data(authority.bytes),
                Data(delegator.bytes), Data(delegatee.bytes), le(nonce),
            ],
            programId: id
        ).0
    }

    // MARK: - Instructions (discriminator byte + packed LE payload)

    /// disc 0 — creates the SubscriptionAuthority PDA and approves it as SPL
    /// delegate with u64::MAX (the "Danger" instruction).
    static func initSubscriptionAuthority(user: PublicKey, mint: PublicKey, userAta: PublicKey) throws -> TransactionInstruction {
        TransactionInstruction(
            keys: [
                .init(publicKey: user, isSigner: true, isWritable: true),
                .init(publicKey: try subscriptionAuthorityPda(user: user, mint: mint), isSigner: false, isWritable: true),
                .init(publicKey: mint, isSigner: false, isWritable: false),
                .init(publicKey: userAta, isSigner: false, isWritable: true),
                .init(publicKey: SystemProgram.id, isSigner: false, isWritable: false),
                .init(publicKey: TokenProgram.id, isSigner: false, isWritable: false),
            ],
            programId: id,
            data: [Data([0])]
        )
    }

    /// disc 1 — fixed (total) allowance. Payload: nonce u64 | amount u64 | expiry i64 | initId i64.
    static func createFixedDelegation(
        delegator: PublicKey, delegatee: PublicKey, mint: PublicKey,
        nonce: UInt64, amount: UInt64, expiryTs: Int64, authorityInitId: Int64
    ) throws -> TransactionInstruction {
        let authority = try subscriptionAuthorityPda(user: delegator, mint: mint)
        var payload = Data([1])
        payload += le(nonce) + le(amount) + le(expiryTs) + le(authorityInitId)
        precondition(payload.count == 33)
        return TransactionInstruction(
            keys: try delegationKeys(authority: authority, delegator: delegator, delegatee: delegatee, nonce: nonce),
            programId: id,
            data: [payload]
        )
    }

    /// disc 2 — recurring allowance. Payload: nonce | amountPerPeriod | periodLengthS | startTs | expiryTs | initId.
    static func createRecurringDelegation(
        delegator: PublicKey, delegatee: PublicKey, mint: PublicKey,
        nonce: UInt64, amountPerPeriod: UInt64, periodLengthSecs: UInt64,
        startTs: Int64, expiryTs: Int64, authorityInitId: Int64
    ) throws -> TransactionInstruction {
        let authority = try subscriptionAuthorityPda(user: delegator, mint: mint)
        var payload = Data([2])
        payload += le(nonce) + le(amountPerPeriod) + le(periodLengthSecs)
        payload += le(startTs) + le(expiryTs) + le(authorityInitId)
        precondition(payload.count == 49)
        return TransactionInstruction(
            keys: try delegationKeys(authority: authority, delegator: delegator, delegatee: delegatee, nonce: nonce),
            programId: id,
            data: [payload]
        )
    }

    /// disc 3 — closes one delegation PDA (rent back to payer).
    static func revokeDelegation(authority: PublicKey, delegationAccount: PublicKey) -> TransactionInstruction {
        TransactionInstruction(
            keys: [
                .init(publicKey: authority, isSigner: true, isWritable: true),
                .init(publicKey: delegationAccount, isSigner: false, isWritable: true),
            ],
            programId: id,
            data: [Data([3])]
        )
    }

    /// disc 14 — the kill switch: SPL Revoke of the program's delegate on the ATA.
    static func revokeSubscriptionAuthority(user: PublicKey, userAta: PublicKey, mint: PublicKey) -> TransactionInstruction {
        TransactionInstruction(
            keys: [
                .init(publicKey: user, isSigner: true, isWritable: false),
                .init(publicKey: userAta, isSigner: false, isWritable: true),
                .init(publicKey: mint, isSigner: false, isWritable: false),
                .init(publicKey: TokenProgram.id, isSigner: false, isWritable: false),
            ],
            programId: id,
            data: [Data([14])]
        )
    }

    private static func delegationKeys(
        authority: PublicKey, delegator: PublicKey, delegatee: PublicKey, nonce: UInt64
    ) throws -> [SolanaSwift.AccountMeta] {
        let delegation = try delegationPda(authority: authority, delegator: delegator, delegatee: delegatee, nonce: nonce)
        return [
            .init(publicKey: delegator, isSigner: true, isWritable: true),
            .init(publicKey: authority, isSigner: false, isWritable: false),
            .init(publicKey: delegation, isSigner: false, isWritable: true),
            .init(publicKey: delegatee, isSigner: false, isWritable: false),
            .init(publicKey: SystemProgram.id, isSigner: false, isWritable: false),
        ]
    }

    // MARK: - On-chain state decode

    /// SubscriptionAuthority (106 B): disc u8 @0 | user @1 | token_mint @33 | payer @65 | bump @97 | init_id i64 @98.
    static func authorityInitId(accountData: Data) -> Int64? {
        guard accountData.count == 106, accountData.first == 0 else { return nil }
        return leI64(accountData, at: 98)
    }

    /// RecurringDelegation (211 B): header(107: disc,version,bump,delegator@3,delegatee@35,payer@67,initId@99)
    /// | authority @107 | mint @139 | periodStart @171 | periodLenS @179 | expiry @187 | amountPerPeriod @195 | pulled @203.
    struct RecurringDelegationState {
        let delegatee: PublicKey
        let mint: PublicKey
        let currentPeriodStartTs: Int64
        let periodLengthSecs: UInt64
        let expiryTs: Int64
        let amountPerPeriod: UInt64
        let amountPulledInPeriod: UInt64
    }

    static func decodeRecurringDelegation(accountData d: Data) -> RecurringDelegationState? {
        // account-type discriminator 3 = RecurringDelegation
        guard d.count == 211, d.first == 3 else { return nil }
        guard
            let delegatee = try? PublicKey(data: d.subdata(in: 35 ..< 67)),
            let mint = try? PublicKey(data: d.subdata(in: 139 ..< 171)),
            let start = leI64(d, at: 171), let periodLen = leU64(d, at: 179),
            let expiry = leI64(d, at: 187), let perPeriod = leU64(d, at: 195),
            let pulled = leU64(d, at: 203)
        else { return nil }
        return RecurringDelegationState(
            delegatee: delegatee, mint: mint, currentPeriodStartTs: start,
            periodLengthSecs: periodLen, expiryTs: expiry,
            amountPerPeriod: perPeriod, amountPulledInPeriod: pulled
        )
    }

    /// SPL token account: mint @0 | owner @32 | amount u64 @64.
    static func tokenAccountAmount(accountData d: Data) -> UInt64? {
        guard d.count >= 72 else { return nil }
        return leU64(d, at: 64)
    }
}

// MARK: - LE helpers

func le(_ v: UInt64) -> Data { withUnsafeBytes(of: v.littleEndian) { Data($0) } }
func le(_ v: Int64) -> Data { withUnsafeBytes(of: v.littleEndian) { Data($0) } }

func leU64(_ data: Data, at offset: Int) -> UInt64? {
    guard data.count >= offset + 8 else { return nil }
    return data.subdata(in: offset ..< offset + 8).withUnsafeBytes { UInt64(littleEndian: $0.loadUnaligned(as: UInt64.self)) }
}

func leI64(_ data: Data, at offset: Int) -> Int64? {
    guard data.count >= offset + 8 else { return nil }
    return data.subdata(in: offset ..< offset + 8).withUnsafeBytes { Int64(littleEndian: $0.loadUnaligned(as: Int64.self)) }
}
