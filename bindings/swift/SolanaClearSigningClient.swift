import Foundation

public struct SolanaAccountMeta: Sendable, Equatable {
    public let pubkey: String
    public let isSigner: Bool
    public let isWritable: Bool

    public init(pubkey: String, isSigner: Bool, isWritable: Bool) {
        self.pubkey = pubkey
        self.isSigner = isSigner
        self.isWritable = isWritable
    }
}

public struct SolanaInstructionInput: Sendable, Equatable {
    public let programId: String
    public let instructionData: Data
    public let accounts: [SolanaAccountMeta]
    public let feePayer: String?

    public init(
        programId: String,
        instructionData: Data,
        accounts: [SolanaAccountMeta],
        feePayer: String? = nil
    ) {
        self.programId = programId
        self.instructionData = instructionData
        self.accounts = accounts
        self.feePayer = feePayer
    }
}

public struct SolanaAccountData: Sendable, Equatable {
    public let owner: String
    public let data: Data

    public init(owner: String, data: Data) {
        self.owner = owner
        self.data = data
    }
}

public protocol SolanaAccountDataProvider: AnyObject, Sendable {
    func accountData(for address: String) async -> SolanaAccountData?
}

public struct DisplayField: Sendable, Equatable {
    public let label: String
    public let value: String

    public init(label: String, value: String) {
        self.label = label
        self.value = value
    }
}

public struct InstructionDisplay: Sendable, Equatable {
    public let intent: String
    public let interpolatedIntent: String?
    public let fields: [DisplayField]

    public init(intent: String, interpolatedIntent: String?, fields: [DisplayField]) {
        self.intent = intent
        self.interpolatedIntent = interpolatedIntent
        self.fields = fields
    }
}

public enum SolanaClearSigningError: Error, Sendable, Equatable {
    case invalidJson(detail: String)
    case invalidRoot(detail: String)
    case invalidSchema(detail: String)
    case unsupportedIdlNode(path: String, kind: String)
    case accountDecode(account: String, detail: String)
    case internalFailure(detail: String)
    /// The instruction input is malformed (e.g. `programId` is not a base58 32-byte key).
    case invalidInput(detail: String)
    /// The IDL source answered, but the document failed verification. Fail closed.
    case idlRejected(programId: String, reason: Srf39IdlRejection)
    /// The IDL source itself failed. Never cached; `retryable` is the source's hint.
    case idlSourceFailed(detail: String, retryable: Bool)
}

extension SolanaClearSigningError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case let .invalidJson(detail):
            return "Invalid sRFC 39 IDL JSON: \(detail)"
        case let .invalidRoot(detail):
            return "Invalid sRFC 39 IDL root: \(detail)"
        case let .invalidSchema(detail):
            return "Invalid sRFC 39 IDL schema: \(detail)"
        case let .unsupportedIdlNode(path, kind):
            return "Unsupported sRFC 39 IDL node at \(path): \(kind)"
        case let .accountDecode(account, detail):
            return "Could not decode linked account \(account): \(detail)"
        case let .internalFailure(detail):
            return "sRFC 39 display failed: \(detail)"
        case let .invalidInput(detail):
            return "Invalid instruction input: \(detail)"
        case let .idlRejected(programId, reason):
            return "IDL for \(programId) rejected: \(reason)"
        case let .idlSourceFailed(detail, retryable):
            return "IDL source failed\(retryable ? " (retryable)" : ""): \(detail)"
        }
    }
}

/// Native sRFC 39 client.
///
/// Two ways to construct it:
/// - `init(idlJSON:)` — low-level, one inline IDL parsed eagerly, no
///   provenance (renders carry `idl_digest_unpinned`) or presentation overlay.
///   `display(instruction:accountProvider:)` can supply raw account data.
/// - `init(idlSource:accountDataProvider:presentationProvider:)` — high-level:
///   the IDL for each program is looked up lazily, verified against the
///   program id and its pinned digest, parsed once and cached.
///
/// `display(instruction:)` returns only the canonical sRFC 39 display.
/// `render(instruction:)` adds the IDL binding, structural hints, the
/// presentation overlay and diagnostics. The canonical half is identical.
public final class SolanaClearSigningClient: Sendable {
    private let client: Srf39ClientFfi
    private let sourceAdapter: Srf39IdlSourceAdapter?
    private let accountAdapter: Srf39AccountProviderAdapter?
    private let presentationAdapter: PresentationProviderAdapter?

    /// Low-level: parses `idlJSON` now and serves its primary program and
    /// every `additionalPrograms` entry. Throws exactly like before.
    public init(idlJSON: String) throws {
        do {
            client = try Srf39ClientFfi.fromIdlJson(
                idlJson: idlJSON,
                accountProvider: nil,
                presentationProvider: nil
            )
        } catch let error as Srf39IdlFailureFfi {
            throw SolanaClearSigningError(error)
        }
        sourceAdapter = nil
        accountAdapter = nil
        presentationAdapter = nil
    }

    /// High-level: lazy, multi-program, verified and cached.
    public init(
        idlSource: any Srf39IdlSource,
        accountDataProvider: (any SolanaAccountDataProvider)? = nil,
        presentationProvider: (any SolanaPresentationMetadataProvider)? = nil
    ) {
        let sourceAdapter = Srf39IdlSourceAdapter(source: idlSource)
        let accountAdapter = accountDataProvider.map(Srf39AccountProviderAdapter.init)
        let presentationAdapter = presentationProvider.map(PresentationProviderAdapter.init)
        client = Srf39ClientFfi(
            idlSource: sourceAdapter,
            accountProvider: accountAdapter,
            presentationProvider: presentationAdapter
        )
        self.sourceAdapter = sourceAdapter
        self.accountAdapter = accountAdapter
        self.presentationAdapter = presentationAdapter
    }

    /// Canonical display only. `accountProvider` overrides the client's
    /// provider for this call. Unknown program ⇒ `nil`; rejected IDL ⇒
    /// `idlRejected`; source failure ⇒ `idlSourceFailed`; malformed
    /// `programId` ⇒ `invalidInput`.
    public func display(
        instruction: SolanaInstructionInput,
        accountProvider: (any SolanaAccountDataProvider)? = nil
    ) async throws -> InstructionDisplay? {
        let ffiProvider = accountProvider.map(Srf39AccountProviderAdapter.init)
        do {
            return try await client
                .display(instruction: instruction.ffi, accountProvider: ffiProvider)
                .map(InstructionDisplay.init)
        } catch let error as RenderFailureFfi {
            throw SolanaClearSigningError(error)
        }
    }

    /// Canonical display plus binding, hints, presentation overlay and
    /// diagnostics. Hosts should loop per instruction and isolate errors
    /// per instruction.
    public func render(instruction: SolanaInstructionInput) async throws -> SolanaRenderOutcome {
        do {
            return SolanaRenderOutcome(try await client.render(instruction: instruction.ffi))
        } catch let error as RenderFailureFfi {
            throw SolanaClearSigningError(error)
        }
    }

    /// The addresses whose account state the display would read (parity
    /// with the reference `getRequiredAccountsForDisplay`). `nil` when the
    /// program is unknown or the instruction is not recognised.
    public func requiredAccounts(for instruction: SolanaInstructionInput) async throws -> [String]? {
        do {
            return try await client.requiredAccounts(instruction: instruction.ffi)
        } catch let error as RenderFailureFfi {
            throw SolanaClearSigningError(error)
        }
    }

    /// Forgets the cached IDL for one program so the source is consulted again.
    public func invalidate(programId: String) {
        client.invalidate(programId: programId)
    }

    public func invalidateAll() {
        client.invalidateAll()
    }
}

private extension SolanaInstructionInput {
    var ffi: Srf39InstructionInputFfi {
        Srf39InstructionInputFfi(
            programId: programId,
            instructionData: instructionData,
            accounts: accounts.map {
                Srf39AccountMetaFfi(
                    pubkey: $0.pubkey,
                    isSigner: $0.isSigner,
                    isWritable: $0.isWritable
                )
            },
            feePayer: feePayer
        )
    }
}

private final class Srf39AccountProviderAdapter: Srf39AccountProviderFfi, Sendable {
    private let provider: any SolanaAccountDataProvider

    init(provider: any SolanaAccountDataProvider) {
        self.provider = provider
    }

    func resolveAccount(address: String) async -> Srf39AccountDataFfi? {
        await provider.accountData(for: address).map {
            Srf39AccountDataFfi(owner: $0.owner, data: $0.data)
        }
    }
}

/// Every Swift error is converted into the declared FFI error: an unexpected
/// exception crossing the boundary would otherwise abort the process.
private final class Srf39IdlSourceAdapter: Srf39IdlSourceFfi, Sendable {
    private let source: any Srf39IdlSource

    init(source: any Srf39IdlSource) {
        self.source = source
    }

    func idlForProgram(programId: String) async throws -> ResolvedSrf39IdlFfi? {
        do {
            return try await source.idl(for: programId).map(\.ffi)
        } catch let Srf39IdlSourceError.unavailable(detail, retryable) {
            throw IdlSourceFailureFfi.Failed(detail: detail, retryable: retryable)
        } catch {
            throw IdlSourceFailureFfi.Failed(detail: String(describing: error), retryable: false)
        }
    }
}

private final class PresentationProviderAdapter: PresentationMetadataProviderFfi, Sendable {
    private let provider: any SolanaPresentationMetadataProvider

    init(provider: any SolanaPresentationMetadataProvider) {
        self.provider = provider
    }

    func tokenMetadata(mint: String) async -> TokenMetadataFfi? {
        await provider.tokenMetadata(for: mint).map(\.ffi)
    }

    func addressLabel(address: String) async -> AddressLabelFfi? {
        await provider.addressLabel(for: address).map(\.ffi)
    }
}

private extension SolanaClearSigningError {
    init(_ error: Srf39IdlFailureFfi) {
        switch error {
        case let .InvalidJson(detail):
            self = .invalidJson(detail: detail)
        case let .InvalidRoot(detail):
            self = .invalidRoot(detail: detail)
        case let .InvalidSchema(detail):
            self = .invalidSchema(detail: detail)
        case let .UnsupportedIdlNode(path, kind):
            self = .unsupportedIdlNode(path: path, kind: kind)
        }
    }

    init(_ error: RenderFailureFfi) {
        switch error {
        case let .InvalidInput(detail):
            self = .invalidInput(detail: detail)
        case let .IdlRejected(programId, rejection):
            self = .idlRejected(programId: programId, reason: Srf39IdlRejection(rejection))
        case let .IdlSourceFailed(detail, retryable):
            self = .idlSourceFailed(detail: detail, retryable: retryable)
        case let .AccountDecode(account, detail):
            self = .accountDecode(account: account, detail: detail)
        case let .Internal(detail):
            self = .internalFailure(detail: detail)
        }
    }
}
