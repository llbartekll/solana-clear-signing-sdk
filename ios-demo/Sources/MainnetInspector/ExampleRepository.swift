import Foundation
import SolanaClearsign

/// One frozen example catalog: a program on a cluster, captured transactions,
/// and the RPC/registry the inspector should use for it.
struct ExampleCatalog: Decodable, Sendable {
    let schemaVersion: Int
    let cluster: String
    let programId: String
    let programName: String
    let rpc: RPCSpec
    let tokenRegistryFile: String
    let extraSnapshotAddresses: [String]?
    let examples: [CatalogExample]
}

struct RPCSpec: Decodable, Sendable, Equatable {
    /// `alchemy` (needs the app's API key) or `public` (uses `endpoint`).
    let kind: String
    let endpoint: String?
}

struct CatalogExample: Decodable, Identifiable, Sendable, Equatable {
    let id: String
    let instruction: String
    let title: String
    let summary: String
    let signature: String
    let slot: UInt64
    let version: String
    let instructionIndex: Int
    let opcode: UInt8
    let snapshotAccountPositions: [Int]?
    let transactionFile: String
}

struct SnapshotAccount: Decodable, Sendable, Equatable {
    let contextSlot: UInt64
    let dataBase64: String
    let dataSha256: String
    let owner: String
    let space: Int

    var accountData: SolanaAccountData? {
        guard let data = Data(base64Encoded: dataBase64), data.count == space else { return nil }
        return SolanaAccountData(owner: owner, data: data)
    }
}

private struct SnapshotAccountsDocument: Decodable {
    let accounts: [String: SnapshotAccount]
}

/// The captures of one catalog (`catalog.json`, `accounts.json`, `Transactions/`).
struct ExampleRepository: Sendable {
    let catalog: ExampleCatalog
    let accountSnapshots: [String: SnapshotAccount]

    private let resourcesDirectory: URL

    /// Bundled captures live in a folder reference, e.g. `DevnetExamples/`.
    init(bundle: Bundle, subdirectory: String) throws {
        guard let catalogURL = bundle.url(
            forResource: "catalog", withExtension: "json", subdirectory: subdirectory
        ) else {
            throw ExampleRepositoryError.missingResource("\(subdirectory)/catalog.json")
        }
        try self.init(resourcesDirectory: catalogURL.deletingLastPathComponent())
    }

    /// Test construction from the repository tree.
    init(resourcesDirectory: URL) throws {
        self.resourcesDirectory = resourcesDirectory
        catalog = try Self.decode(
            resourcesDirectory.appendingPathComponent("catalog.json"),
            as: ExampleCatalog.self
        )
        accountSnapshots = try Self.decode(
            resourcesDirectory.appendingPathComponent("accounts.json"),
            as: SnapshotAccountsDocument.self
        ).accounts
    }

    var cluster: String { catalog.cluster }

    func example(signature: String) -> CatalogExample? {
        catalog.examples.first { $0.signature == signature }
    }

    func transaction(for example: CatalogExample) throws -> RPCTransaction {
        try Self.decode(
            resourcesDirectory.appendingPathComponent("Transactions/\(example.transactionFile)"),
            as: RPCTransaction.self
        )
    }

    private static func decode<Value: Decodable>(_ url: URL, as _: Value.Type) throws -> Value {
        do {
            return try JSONDecoder().decode(Value.self, from: Data(contentsOf: url))
        } catch {
            throw ExampleRepositoryError.invalidResource(url.lastPathComponent, error.localizedDescription)
        }
    }
}

/// The long-lived sRFC 39 resources shared by every inspection: the pinned
/// IDL manifest, one curated token registry per cluster, and the catalogs.
final class InspectorResources: Sendable {
    static let catalogSubdirectories = ["DevnetExamples"]

    let idlSource: BundledSrf39IdlSource
    let catalogs: [ExampleRepository]
    /// Directory holding the manifest, the IDL copies, and the registries.
    let srf39Directory: URL

    private let registries: [String: LocalTokenRegistry]

    init(bundle: Bundle = .main) throws {
        guard let manifestURL = bundle.url(forResource: "srf39-manifest", withExtension: "json") else {
            throw ExampleRepositoryError.missingResource("srf39-manifest.json")
        }
        srf39Directory = manifestURL.deletingLastPathComponent()
        idlSource = try BundledSrf39IdlSource(manifestURL: manifestURL, directory: srf39Directory)
        catalogs = try Self.catalogSubdirectories.map { subdirectory in
            try ExampleRepository(bundle: bundle, subdirectory: subdirectory)
        }
        registries = try Self.loadRegistries(catalogs: catalogs, directory: srf39Directory)
    }

    /// Test construction: `resourcesRoot` is `ios-demo/Resources`.
    init(resourcesRoot: URL) throws {
        srf39Directory = resourcesRoot.appendingPathComponent("Srf39")
        idlSource = try BundledSrf39IdlSource(
            manifestURL: srf39Directory.appendingPathComponent("srf39-manifest.json"),
            directory: srf39Directory
        )
        catalogs = try Self.catalogSubdirectories.map { subdirectory in
            try ExampleRepository(resourcesDirectory: resourcesRoot.appendingPathComponent(subdirectory))
        }
        registries = try Self.loadRegistries(catalogs: catalogs, directory: srf39Directory)
    }

    func registry(for cluster: String) -> LocalTokenRegistry? {
        registries[cluster]
    }

    func repository(for cluster: String) -> ExampleRepository? {
        catalogs.first { $0.cluster == cluster }
    }

    func example(signature: String) -> (repository: ExampleRepository, example: CatalogExample)? {
        for repository in catalogs {
            if let example = repository.example(signature: signature) {
                return (repository, example)
            }
        }
        return nil
    }

    private static func loadRegistries(
        catalogs: [ExampleRepository],
        directory: URL
    ) throws -> [String: LocalTokenRegistry] {
        var registries: [String: LocalTokenRegistry] = [:]
        for repository in catalogs where registries[repository.cluster] == nil {
            let registry = try LocalTokenRegistry(
                url: directory.appendingPathComponent(repository.catalog.tokenRegistryFile)
            )
            guard registry.cluster == repository.cluster else {
                throw ExampleRepositoryError.invalidResource(
                    repository.catalog.tokenRegistryFile,
                    "registry cluster \(registry.cluster) does not match catalog cluster \(repository.cluster)"
                )
            }
            registries[repository.cluster] = registry
        }
        return registries
    }
}

enum ExampleRepositoryError: Error, LocalizedError, Equatable {
    case missingResource(String)
    case invalidResource(String, String)

    var errorDescription: String? {
        switch self {
        case let .missingResource(name): return "Missing bundled resource \(name)."
        case let .invalidResource(name, detail): return "Invalid bundled resource \(name): \(detail)"
        }
    }
}

actor InspectorAccountProvider: SolanaAccountDataProvider {
    private enum CacheEntry {
        case account(SolanaAccountData)
        case missing
    }

    private let rpc: (any SolanaRPCServing)?
    private let snapshots: [String: SnapshotAccount]
    private var cache: [String: CacheEntry] = [:]
    private var origins: [AccountDataOrigin] = []

    init(rpc: (any SolanaRPCServing)?, snapshots: [String: SnapshotAccount]) {
        self.rpc = rpc
        self.snapshots = snapshots
    }

    func accountData(for address: String) async -> SolanaAccountData? {
        if let cached = cache[address] {
            switch cached {
            case let .account(account): return account
            case .missing: return nil
            }
        }
        if let rpc {
            do {
                if let resolved = try await rpc.account(address: address) {
                    let account = SolanaAccountData(owner: resolved.owner, data: resolved.data)
                    cache[address] = .account(account)
                    origins.append(.live(slot: resolved.contextSlot))
                    return account
                }
            } catch {
                // A known example may still use its explicit bundled snapshot.
            }
        }
        if let snapshot = snapshots[address], let account = snapshot.accountData {
            cache[address] = .account(account)
            origins.append(.snapshot(slot: snapshot.contextSlot))
            return account
        }
        cache[address] = .missing
        origins.append(.missing)
        return nil
    }

    func sourceSummary() -> String {
        guard let first = origins.first else { return "Not required" }
        let kinds = Set(origins.map(\.kind))
        if kinds.count > 1 { return "Mixed" }
        switch first {
        case .live:
            let slots = origins.compactMap(\.slot)
            return "Live @ slot \(slots.max() ?? 0)"
        case .snapshot:
            let slots = origins.compactMap(\.slot)
            return "Snapshot @ slot \(slots.max() ?? 0)"
        case .missing:
            return "Missing"
        }
    }
}

private enum AccountDataOrigin: Sendable {
    case live(slot: UInt64)
    case snapshot(slot: UInt64)
    case missing

    enum Kind: Hashable {
        case live
        case snapshot
        case missing
    }

    var kind: Kind {
        switch self {
        case .live: return .live
        case .snapshot: return .snapshot
        case .missing: return .missing
        }
    }

    var slot: UInt64? {
        switch self {
        case let .live(slot), let .snapshot(slot): return slot
        case .missing: return nil
        }
    }
}
