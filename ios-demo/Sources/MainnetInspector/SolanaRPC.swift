import Foundation

protocol SolanaRPCServing: Sendable {
    func transaction(signature: String) async throws -> RPCTransaction
    func account(address: String) async throws -> RPCResolvedAccount?
}

/// Resolves a catalog's RPC declaration to a concrete endpoint. Mainnet goes
/// through Alchemy and needs the app's key; public clusters need none.
enum RPCEndpoint {
    static func url(for spec: RPCSpec, alchemyAPIKey: String?) -> URL? {
        switch spec.kind {
        case "alchemy":
            guard let key = alchemyAPIKey else { return nil }
            return URL(string: "https://solana-mainnet.g.alchemy.com/v2/\(key)")
        case "public":
            return spec.endpoint.flatMap { URL(string: $0) }
        default:
            return nil
        }
    }
}

actor SolanaRPCClient: SolanaRPCServing {
    private let endpoint: URL
    private let session: URLSession

    init(endpoint: URL, session: URLSession = .shared) {
        self.endpoint = endpoint
        self.session = session
    }

    func transaction(signature: String) async throws -> RPCTransaction {
        let response: RPCResponse<RPCTransaction> = try await request(
            method: "getTransaction",
            params: [
                signature,
                [
                    "commitment": "finalized",
                    "encoding": "json",
                    "maxSupportedTransactionVersion": 0,
                ],
            ]
        )
        guard let result = response.result else {
            throw SolanaRPCError.transactionNotFound(signature)
        }
        return result
    }

    func account(address: String) async throws -> RPCResolvedAccount? {
        let response: RPCResponse<RPCAccountResult> = try await request(
            method: "getAccountInfo",
            params: [address, ["commitment": "finalized", "encoding": "base64"]]
        )
        guard let result = response.result else {
            throw SolanaRPCError.missingResult("getAccountInfo")
        }
        return result.value.map {
            RPCResolvedAccount(
                owner: $0.owner,
                data: $0.data.bytes,
                contextSlot: result.context.slot
            )
        }
    }

    private func request<Result: Decodable>(
        method: String,
        params: [Any]
    ) async throws -> RPCResponse<Result> {
        var request = URLRequest(url: endpoint)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = 20
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        ])
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, 200 ..< 300 ~= http.statusCode else {
            throw SolanaRPCError.httpStatus((response as? HTTPURLResponse)?.statusCode ?? -1)
        }
        let decoded = try JSONDecoder().decode(RPCResponse<Result>.self, from: data)
        if let error = decoded.error {
            throw SolanaRPCError.server(code: error.code, message: error.message)
        }
        return decoded
    }
}

enum SolanaRPCError: Error, LocalizedError, Equatable {
    case httpStatus(Int)
    case server(code: Int, message: String)
    case missingResult(String)
    case transactionNotFound(String)

    var errorDescription: String? {
        switch self {
        case let .httpStatus(status):
            return "RPC returned HTTP \(status)."
        case let .server(code, message):
            return "RPC error \(code): \(message)"
        case let .missingResult(method):
            return "RPC returned no result for \(method)."
        case let .transactionNotFound(signature):
            return "Transaction \(signature) was not found."
        }
    }
}

struct RPCResponse<Result: Decodable>: Decodable {
    let result: Result?
    let error: RPCServerError?
}

struct RPCServerError: Decodable {
    let code: Int
    let message: String
}

struct RPCTransaction: Decodable, Sendable {
    let slot: UInt64
    let blockTime: Int64?
    let meta: RPCTransactionMeta
    let transaction: RPCTransactionContainer
    let version: RPCTransactionVersion
}

struct RPCTransactionMeta: Decodable, Sendable {
    let err: JSONValue?
    let loadedAddresses: RPCLoadedAddresses?
    let innerInstructions: [RPCInnerInstructionGroup]?
}

struct RPCLoadedAddresses: Decodable, Sendable {
    let writable: [String]
    let readonly: [String]
}

struct RPCInnerInstructionGroup: Decodable, Sendable {
    let index: Int
}

struct RPCTransactionContainer: Decodable, Sendable {
    let message: RPCMessage
    let signatures: [String]
}

struct RPCMessage: Decodable, Sendable {
    let accountKeys: [String]
    let header: RPCMessageHeader
    let instructions: [RPCCompiledInstruction]
}

struct RPCMessageHeader: Decodable, Sendable {
    let numReadonlySignedAccounts: Int
    let numReadonlyUnsignedAccounts: Int
    let numRequiredSignatures: Int
}

struct RPCCompiledInstruction: Decodable, Sendable {
    let accounts: [Int]
    let data: String
    let programIdIndex: Int
}

enum RPCTransactionVersion: Decodable, Sendable, Equatable {
    case legacy
    case numbered(Int)

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let number = try? container.decode(Int.self) {
            guard number == 0 else {
                throw DecodingError.dataCorruptedError(
                    in: container,
                    debugDescription: "Only legacy and v0 transactions are supported"
                )
            }
            self = .numbered(0)
        } else {
            guard try container.decode(String.self) == "legacy" else {
                throw DecodingError.dataCorruptedError(
                    in: container,
                    debugDescription: "Only legacy and v0 transactions are supported"
                )
            }
            self = .legacy
        }
    }

    var displayName: String {
        switch self {
        case .legacy: return "Legacy"
        case let .numbered(number): return "v\(number)"
        }
    }
}

struct RPCAccountResult: Decodable, Sendable {
    let context: RPCContext
    let value: RPCAccountInfo?
}

struct RPCContext: Decodable, Sendable {
    let slot: UInt64
}

struct RPCAccountInfo: Decodable, Sendable {
    let data: RPCBase64Data
    let owner: String
}

struct RPCResolvedAccount: Sendable, Equatable {
    let owner: String
    let data: Data
    let contextSlot: UInt64
}

struct RPCBase64Data: Decodable, Sendable {
    let bytes: Data

    init(from decoder: Decoder) throws {
        var container = try decoder.unkeyedContainer()
        let encoded = try container.decode(String.self)
        let encoding = try container.decode(String.self)
        guard encoding == "base64", let bytes = Data(base64Encoded: encoded) else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Expected [base64, base64] account data"
            )
        }
        self.bytes = bytes
    }
}

enum JSONValue: Decodable, Sendable {
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let value = try? container.decode(Bool.self) { self = .bool(value) }
        else if let value = try? container.decode(Double.self) { self = .number(value) }
        else if let value = try? container.decode(String.self) { self = .string(value) }
        else if let value = try? container.decode([JSONValue].self) { self = .array(value) }
        else { self = .object(try container.decode([String: JSONValue].self)) }
    }
}
