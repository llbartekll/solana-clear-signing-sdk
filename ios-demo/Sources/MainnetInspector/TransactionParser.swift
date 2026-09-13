import Foundation
import SolanaClearsign
import SolanaSwift

struct ParsedTransaction: Sendable, Equatable {
    let signature: String
    let slot: UInt64
    let blockTime: Int64?
    let version: String
    let succeeded: Bool
    let feePayer: String
    let innerInstructionGroupCount: Int
    let instructions: [ParsedInstruction]
}

struct ParsedInstruction: Identifiable, Sendable, Equatable {
    let id: Int
    let dataBase58: String
    let input: SolanaInstructionInput
}

enum TransactionParserError: Error, LocalizedError, Equatable {
    case invalidSignature
    case invalidHeader
    case missingFeePayer
    case invalidProgramIndex(instruction: Int, index: Int)
    case invalidAccountIndex(instruction: Int, index: Int)
    case invalidInstructionData(instruction: Int)

    var errorDescription: String? {
        switch self {
        case .invalidSignature: return "The signature is not a 64-byte base58 value."
        case .invalidHeader: return "The transaction message header is inconsistent."
        case .missingFeePayer: return "The transaction has no fee payer."
        case let .invalidProgramIndex(instruction, index):
            return "Instruction \(instruction) has invalid program index \(index)."
        case let .invalidAccountIndex(instruction, index):
            return "Instruction \(instruction) has invalid account index \(index)."
        case let .invalidInstructionData(instruction):
            return "Instruction \(instruction) contains invalid base58 data."
        }
    }
}

enum TransactionParser {
    static func validateSignature(_ value: String) -> String? {
        let signature = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard 64 ... 88 ~= signature.count, Base58.decode(signature).count == 64 else {
            return nil
        }
        return signature
    }

    static func parse(_ transaction: RPCTransaction) throws -> ParsedTransaction {
        let message = transaction.transaction.message
        let staticKeys = message.accountKeys
        guard let encodedSignature = transaction.transaction.signatures.first,
              let signature = validateSignature(encodedSignature)
        else {
            throw TransactionParserError.invalidSignature
        }
        guard let feePayer = staticKeys.first else { throw TransactionParserError.missingFeePayer }
        let header = message.header
        guard header.numRequiredSignatures >= 0,
              header.numReadonlySignedAccounts >= 0,
              header.numReadonlyUnsignedAccounts >= 0,
              header.numRequiredSignatures <= staticKeys.count,
              header.numReadonlySignedAccounts <= header.numRequiredSignatures,
              header.numReadonlyUnsignedAccounts <= staticKeys.count - header.numRequiredSignatures
        else {
            throw TransactionParserError.invalidHeader
        }

        let loaded = transaction.meta.loadedAddresses ?? RPCLoadedAddresses(writable: [], readonly: [])
        let allKeys = staticKeys + loaded.writable + loaded.readonly
        let loadedWritableEnd = staticKeys.count + loaded.writable.count
        let instructions = try message.instructions.enumerated().map { instructionIndex, instruction in
            guard allKeys.indices.contains(instruction.programIdIndex) else {
                throw TransactionParserError.invalidProgramIndex(
                    instruction: instructionIndex,
                    index: instruction.programIdIndex
                )
            }
            let decoded = Base58.decode(instruction.data)
            guard instruction.data.isEmpty || !decoded.isEmpty else {
                throw TransactionParserError.invalidInstructionData(instruction: instructionIndex)
            }
            let accounts = try instruction.accounts.map { accountIndex -> SolanaAccountMeta in
                guard allKeys.indices.contains(accountIndex) else {
                    throw TransactionParserError.invalidAccountIndex(
                        instruction: instructionIndex,
                        index: accountIndex
                    )
                }
                let isSigner = accountIndex < header.numRequiredSignatures
                let isWritable: Bool
                if accountIndex < staticKeys.count {
                    if isSigner {
                        isWritable = accountIndex
                            < header.numRequiredSignatures - header.numReadonlySignedAccounts
                    } else {
                        isWritable = accountIndex
                            < staticKeys.count - header.numReadonlyUnsignedAccounts
                    }
                } else {
                    isWritable = accountIndex < loadedWritableEnd
                }
                return SolanaAccountMeta(
                    pubkey: allKeys[accountIndex],
                    isSigner: isSigner,
                    isWritable: isWritable
                )
            }
            return ParsedInstruction(
                id: instructionIndex,
                dataBase58: instruction.data,
                input: SolanaInstructionInput(
                    programId: allKeys[instruction.programIdIndex],
                    instructionData: Data(decoded),
                    accounts: accounts,
                    feePayer: feePayer
                )
            )
        }

        return ParsedTransaction(
            signature: signature,
            slot: transaction.slot,
            blockTime: transaction.blockTime,
            version: transaction.version.displayName,
            succeeded: transaction.meta.err == nil,
            feePayer: feePayer,
            innerInstructionGroupCount: transaction.meta.innerInstructions?.count ?? 0,
            instructions: instructions
        )
    }
}
