import Foundation

/// Answers "which IDL describes this program?".
///
/// Return `nil` when the source does not know the program; that is an
/// expected outcome, not an error. Throw `Srf39IdlSourceError.unavailable` to
/// report a source failure with a `retryable` hint; any other thrown error is
/// reported to the client as a non-retryable source failure.
public protocol Srf39IdlSource: AnyObject, Sendable {
    func idl(for programId: String) async throws -> ResolvedSrf39Idl?
}

public enum Srf39IdlSourceError: Error, Sendable, Equatable {
    case unavailable(detail: String, retryable: Bool)
}

/// An IDL document as returned by a source, before the client verifies it.
public struct ResolvedSrf39Idl: Sendable, Equatable {
    /// The program the source claims this IDL describes.
    public let programId: String
    /// The IDL JSON exactly as stored. The digest is computed over the UTF-8
    /// bytes of this string, so sources must not re-serialise.
    public let json: String
    public let provenance: Srf39IdlProvenance

    public init(programId: String, json: String, provenance: Srf39IdlProvenance) {
        self.programId = programId
        self.json = json
        self.provenance = provenance
    }
}

public struct Srf39IdlProvenance: Sendable, Equatable {
    /// Opaque identifier of the source instance, e.g. `bundle:srf39-manifest.json`.
    public let sourceId: String
    public let origin: Srf39IdlOrigin
    /// Expected SHA-256 of the JSON bytes as 64 lowercase hex characters.
    /// `nil` leaves the document unpinned, which every render reports as
    /// the `idl_digest_unpinned` diagnostic.
    public let expectedSHA256: String?
    public let version: String?
    /// URL, commit or path identifying the upstream document.
    public let reference: String?

    public init(
        sourceId: String,
        origin: Srf39IdlOrigin,
        expectedSHA256: String? = nil,
        version: String? = nil,
        reference: String? = nil
    ) {
        self.sourceId = sourceId
        self.origin = origin
        self.expectedSHA256 = expectedSHA256
        self.version = version
        self.reference = reference
    }
}

public enum Srf39IdlOrigin: Sendable, Equatable {
    case bundled
    case localFile
    /// Reserved for the program-metadata program's canonical account.
    case programMetadataCanonical(authority: String)
    case other(String)
}

public enum BundledSrf39IdlSourceError: Error, Sendable, Equatable {
    case unsupportedSchemaVersion(Int)
    case duplicateProgramId(String)
    case malformedDigest(programId: String, digest: String)
}

/// Reference source: a manifest listing IDL files that live next to it.
///
/// Files are read lazily on first use and cached for the lifetime of the
/// source. Each manifest entry pins a program id, a file name and (ideally)
/// the SHA-256 of the file bytes; the client re-verifies both.
public actor BundledSrf39IdlSource: Srf39IdlSource {
    public struct Entry: Sendable, Equatable {
        public let programId: String
        public let fileName: String
        public let sha256: String?
        public let version: String?
        public let reference: String?

        public init(
            programId: String,
            fileName: String,
            sha256: String? = nil,
            version: String? = nil,
            reference: String? = nil
        ) {
            self.programId = programId
            self.fileName = fileName
            self.sha256 = sha256
            self.version = version
            self.reference = reference
        }
    }

    private let sourceId: String
    private let directory: URL
    private let entries: [String: Entry]
    private var cache: [String: String] = [:]

    /// Loads `manifestURL` (format: `{"schemaVersion":1,"sourceId":"…","idls":[…]}`)
    /// and resolves each entry's `file` relative to `directory`, which
    /// defaults to the manifest's own directory.
    public init(manifestURL: URL, directory: URL? = nil) throws {
        let manifest = try JSONDecoder().decode(Manifest.self, from: Data(contentsOf: manifestURL))
        guard manifest.schemaVersion == 1 else {
            throw BundledSrf39IdlSourceError.unsupportedSchemaVersion(manifest.schemaVersion)
        }
        let entries = manifest.idls.map {
            Entry(
                programId: $0.programId,
                fileName: $0.file,
                sha256: $0.sha256,
                version: $0.version,
                reference: $0.reference
            )
        }
        try self.init(
            entries: entries,
            directory: directory ?? manifestURL.deletingLastPathComponent(),
            sourceId: manifest.sourceId ?? "bundle:\(manifestURL.lastPathComponent)"
        )
    }

    public init(entries: [Entry], directory: URL, sourceId: String = "bundle") throws {
        var indexed: [String: Entry] = [:]
        for entry in entries {
            if indexed[entry.programId] != nil {
                throw BundledSrf39IdlSourceError.duplicateProgramId(entry.programId)
            }
            if let digest = entry.sha256, !Self.isHexDigest(digest) {
                throw BundledSrf39IdlSourceError.malformedDigest(programId: entry.programId, digest: digest)
            }
            indexed[entry.programId] = entry
        }
        self.entries = indexed
        self.directory = directory
        self.sourceId = sourceId
    }

    public func idl(for programId: String) async throws -> ResolvedSrf39Idl? {
        guard let entry = entries[programId] else { return nil }
        let json: String
        if let cached = cache[programId] {
            json = cached
        } else {
            let url = directory.appendingPathComponent(entry.fileName)
            do {
                json = try String(contentsOf: url, encoding: .utf8)
            } catch {
                throw Srf39IdlSourceError.unavailable(
                    detail: "could not read \(entry.fileName): \(error.localizedDescription)",
                    retryable: false
                )
            }
            cache[programId] = json
        }
        return ResolvedSrf39Idl(
            programId: programId,
            json: json,
            provenance: Srf39IdlProvenance(
                sourceId: sourceId,
                origin: .bundled,
                expectedSHA256: entry.sha256?.lowercased(),
                version: entry.version,
                reference: entry.reference
            )
        )
    }

    private static func isHexDigest(_ value: String) -> Bool {
        value.count == 64 && value.allSatisfy(\.isHexDigit)
    }

    private struct Manifest: Decodable {
        let schemaVersion: Int
        let sourceId: String?
        let idls: [ManifestEntry]
    }

    private struct ManifestEntry: Decodable {
        let programId: String
        let file: String
        let sha256: String?
        let version: String?
        let reference: String?
    }
}
