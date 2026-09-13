import Foundation

/// Host-curated presentation metadata. Nothing returned here ever changes the
/// canonical display; it feeds annotations and explicitly bound token amounts.
public protocol SolanaPresentationMetadataProvider: AnyObject, Sendable {
    /// Metadata for a mint address, or `nil` when unknown.
    func tokenMetadata(for mint: String) async -> SolanaTokenMetadata?
    /// A human label for an address, or `nil` when unknown.
    func addressLabel(for address: String) async -> SolanaAddressLabel?
}

public struct SolanaTokenMetadata: Sendable, Equatable {
    public let symbol: String
    public let name: String?
    /// Scales explicitly bound token amounts; otherwise only cross-checks the IDL scale.
    public let decimals: UInt8?
    /// Cross-check only: compared with the fetched account owner.
    public let tokenProgram: String?

    public init(
        symbol: String,
        name: String? = nil,
        decimals: UInt8? = nil,
        tokenProgram: String? = nil
    ) {
        self.symbol = symbol
        self.name = name
        self.decimals = decimals
        self.tokenProgram = tokenProgram
    }
}

public struct SolanaAddressLabel: Sendable, Equatable {
    public let label: String
    public let source: String?

    public init(label: String, source: String? = nil) {
        self.label = label
        self.source = source
    }
}

/// The character policy the SDK applies to symbols before annotating an
/// amount: 1…16 characters, first `[A-Za-z0-9]`, rest `[A-Za-z0-9$._-]`.
/// ASCII-only by design so homoglyph tickers are rejected, not displayed.
public enum SolanaSymbolPolicy {
    public static func isAcceptable(_ symbol: String) -> Bool {
        let scalars = Array(symbol.unicodeScalars)
        guard !scalars.isEmpty, scalars.count <= 16 else { return false }
        guard isAlphanumeric(scalars[0]) else { return false }
        return scalars.dropFirst().allSatisfy { scalar in
            isAlphanumeric(scalar) || scalar == "$" || scalar == "." || scalar == "_" || scalar == "-"
        }
    }

    private static func isAlphanumeric(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x30 ... 0x39, 0x41 ... 0x5A, 0x61 ... 0x7A: return true
        default: return false
        }
    }
}

public enum LocalTokenRegistryError: Error, Sendable, Equatable {
    case unsupportedSchemaVersion(Int)
    case duplicateMint(String)
    case unacceptableSymbol(mint: String, symbol: String)
    case duplicateLabel(String)
}

/// Reference provider backed by a bundled JSON registry keyed by mint
/// address. Immutable after loading; one registry per cluster.
public final class LocalTokenRegistry: SolanaPresentationMetadataProvider, Sendable {
    public let cluster: String
    public let sourceId: String?
    private let tokens: [String: SolanaTokenMetadata]
    private let labels: [String: SolanaAddressLabel]

    public convenience init(url: URL) throws {
        try self.init(data: Data(contentsOf: url))
    }

    public init(data: Data) throws {
        let document = try JSONDecoder().decode(Document.self, from: data)
        guard document.schemaVersion == 1 else {
            throw LocalTokenRegistryError.unsupportedSchemaVersion(document.schemaVersion)
        }
        var tokens: [String: SolanaTokenMetadata] = [:]
        for token in document.tokens {
            if tokens[token.mint] != nil {
                throw LocalTokenRegistryError.duplicateMint(token.mint)
            }
            guard SolanaSymbolPolicy.isAcceptable(token.symbol) else {
                throw LocalTokenRegistryError.unacceptableSymbol(mint: token.mint, symbol: token.symbol)
            }
            tokens[token.mint] = SolanaTokenMetadata(
                symbol: token.symbol,
                name: token.name,
                decimals: token.decimals,
                tokenProgram: token.tokenProgram
            )
        }
        var labels: [String: SolanaAddressLabel] = [:]
        for label in document.labels ?? [] {
            if labels[label.address] != nil {
                throw LocalTokenRegistryError.duplicateLabel(label.address)
            }
            labels[label.address] = SolanaAddressLabel(label: label.label, source: label.source)
        }
        cluster = document.cluster
        sourceId = document.source?.id
        self.tokens = tokens
        self.labels = labels
    }

    public func tokenMetadata(for mint: String) async -> SolanaTokenMetadata? {
        tokens[mint]
    }

    public func addressLabel(for address: String) async -> SolanaAddressLabel? {
        labels[address]
    }

    private struct Document: Decodable {
        let schemaVersion: Int
        let cluster: String
        let source: Source?
        let tokens: [Token]
        let labels: [Label]?
    }

    private struct Source: Decodable {
        let id: String
        let retrievedAt: String?
        let reference: String?
    }

    private struct Token: Decodable {
        let mint: String
        let symbol: String
        let name: String?
        let decimals: UInt8?
        let tokenProgram: String?
    }

    private struct Label: Decodable {
        let address: String
        let label: String
        let source: String?
    }
}
