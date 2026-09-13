import Foundation

/// What `SolanaClearSigningClient.render(instruction:)` returns.
public enum SolanaRenderOutcome: Sendable, Equatable {
    case rendered(SolanaRenderedInstruction)
    /// Expected "not mine" outcomes; never a failure.
    case unsupported(SolanaUnsupportedReason)
}

public enum SolanaUnsupportedReason: Sendable, Equatable {
    case idlNotFound(programId: String)
    case instructionNotRecognized(programId: String)
    case instructionDecodeFailed(programId: String, instruction: String)
}

public struct SolanaRenderedInstruction: Sendable, Equatable {
    /// Byte-for-byte what the strict sRFC 39 renderer produced.
    public let canonical: InstructionDisplay
    public let idl: Srf39IdlBinding
    /// SDK-derived, non-normative structure read off the IDL graph.
    public let hints: SolanaDisplayHints
    /// Annotations and presented token amounts; canonical text remains available.
    public let presentation: SolanaPresentationOverlay
    /// In emission order. Branch on `code`, never on `message`.
    public let diagnostics: [SolanaDiagnostic]
}

/// What the client verified about the IDL it rendered with.
public struct Srf39IdlBinding: Sendable, Equatable {
    public let programId: String
    public let programName: String
    /// Actual SHA-256 of the IDL bytes, lowercase hex.
    public let sha256: String
    /// `true` when the source pinned a digest and it matched.
    public let digestPinned: Bool
    public let provenance: Srf39IdlProvenance
}

public struct SolanaDisplayHints: Sendable, Equatable {
    /// The IDL instruction node name.
    public let instructionName: String
    /// `true` when the IDL has a sentence template that was suppressed.
    public let interpolatedIntentSuppressed: Bool
    /// One per amount argument, in IDL order.
    public let amounts: [SolanaAmountHint]
    /// One per date-time or duration argument, in IDL order.
    public let times: [SolanaTimeHint]
    /// One per rendered public-key argument value (array elements included).
    public let publicKeyArguments: [SolanaPublicKeyArgumentHint]
    /// Named accounts in IDL order, then rendered remaining-account fields.
    public let accounts: [SolanaAccountHint]
    /// Linked accounts the renderer needed, sorted by address.
    public let linkedAccountReads: [SolanaLinkedAccountRead]
}

public struct SolanaAmountHint: Sendable, Equatable {
    /// Index into `canonical.fields`, or `nil` when the argument was skipped.
    public let fieldIndex: Int?
    public let argument: String
    /// The struct field name inside a flattened struct argument, if any.
    public let member: String?
    /// The decoded integer before scaling, as a decimal string.
    public let rawValue: String
    /// `true` when the canonical value is shown raw (`… (raw)`).
    public let degraded: Bool
    public let decimals: SolanaDecimalsSource
    public let unit: SolanaUnitSource
    /// Explicit amount/mint relationship from the SDK's namespaced IDL extension.
    public let token: SolanaTokenHint?
}

public struct SolanaTokenHint: Sendable, Equatable {
    public let mint: String?
}

/// A date-time or duration argument. The canonical text is the reference's
/// ISO 8601 / `HH:mm:ss` form, or the bare integer when the value is outside
/// what the reference can format; presentation may show `seconds` in a
/// friendlier form next to it, never instead of it.
public struct SolanaTimeHint: Sendable, Equatable {
    public let fieldIndex: Int?
    public let argument: String
    public let member: String?
    /// The decoded integer as a decimal string.
    public let rawValue: String
    public let display: SolanaTimeDisplay
    /// Whole seconds (`rawValue / ticksPerSecond`, floored), present only when
    /// the canonical layer formatted the value.
    public let seconds: Int64?
    /// `true` when the canonical value is the formatted form.
    public let formatted: Bool
}

public enum SolanaTimeDisplay: Sendable, Equatable {
    /// Ticks since the Unix epoch.
    case dateTime(ticksPerSecond: UInt32)
    /// Elapsed ticks.
    case duration(ticksPerSecond: UInt32)
}

public enum SolanaDecimalsSource: Sendable, Equatable {
    case implicitZero
    case literal(UInt64)
    /// The scale comes from `path` of the instruction account `account`
    /// (bound to `address`), decoded through `linkedAccount`.
    case accountField(account: String, address: String?, linkedAccount: String?, path: String, resolved: UInt8?)
    case unsatisfied
}

public enum SolanaUnitSource: Sendable, Equatable {
    case absent
    case literal(String)
    case accountField(account: String, address: String?, linkedAccount: String?, path: String, resolved: String?)
    case unsatisfied
}

/// An argument (or array element) whose IDL type is a public key. The overlay
/// looks it up like an account field: a label, and a `tokenMint` annotation
/// when the registry knows the address as a mint.
public struct SolanaPublicKeyArgumentHint: Sendable, Equatable {
    public let fieldIndex: Int?
    public let argument: String
    public let member: String?
    /// The element index when the argument is an array of public keys.
    public let element: Int?
    public let address: String
}

public struct SolanaAccountHint: Sendable, Equatable {
    public let fieldIndex: Int?
    /// IDL account name; for remaining accounts, the group's argument name.
    public let name: String
    public let address: String?
    /// The label as rendered, including any `#n` suffix.
    public let label: String
    public let linkedAccount: String?
    /// `true` when the account's data was surfaced through provide/inject.
    public let consumed: Bool
}

public struct SolanaLinkedAccountRead: Sendable, Equatable {
    public let address: String
    public let status: Status

    public enum Status: Sendable, Equatable {
        case fetched(owner: String, length: Int)
        case missing
    }
}

public struct SolanaPresentationOverlay: Sendable, Equatable {
    /// Sorted by `(fieldIndex, kind)`, without duplicates.
    public let annotations: [SolanaFieldAnnotation]
    /// SDK-formatted amounts from explicit IDL bindings. Canonical text is unchanged.
    public let tokenAmounts: [SolanaPresentedTokenAmount]

    public func tokenAmount(for fieldIndex: Int) -> SolanaPresentedTokenAmount? {
        tokenAmounts.first { $0.fieldIndex == fieldIndex }
    }

    /// Annotations attached to one canonical field.
    public func annotations(for fieldIndex: Int) -> [SolanaFieldAnnotation] {
        annotations.filter { $0.fieldIndex == fieldIndex }
    }
}

public struct SolanaPresentedTokenAmount: Sendable, Equatable {
    public let fieldIndex: Int
    /// Numeric text, or an explicitly marked raw integer. Symbol is an annotation.
    public let value: String
    public let mint: String?
    /// Callback scale after cross-checks; nil means the presented amount is raw.
    public let decimals: UInt8?
}

public struct SolanaFieldAnnotation: Sendable, Equatable {
    public let fieldIndex: Int
    public let kind: Kind

    public enum Kind: Sendable, Equatable {
        /// Token identity for the presented amount (if present), otherwise
        /// the canonical amount. When `appliesToValue` is false, the amount is
        /// raw and the symbol must not be shown as its unit.
        case tokenAmount(
            symbol: String,
            name: String?,
            sourceAddress: String,
            appliesToValue: Bool
        )
        /// The field is the account that supplied an amount's scale.
        case tokenMint(symbol: String, name: String?)
        /// A host label for the address in this field; shown next to it, never instead.
        case addressLabel(label: String, source: String?)
    }
}

public struct SolanaDiagnostic: Sendable, Equatable {
    /// Stable snake_case code — the machine contract.
    public let code: String
    public let severity: Severity
    /// Free text; may change between versions.
    public let message: String

    public enum Severity: Sendable, Equatable {
        case info
        case warning
    }
}

public enum Srf39IdlRejection: Sendable, Equatable {
    case programMismatch(requested: String, declared: String)
    /// `expected` is the raw pinned string, also when it is malformed.
    case digestMismatch(expected: String, actual: String)
    case invalid(Srf39IdlParseFailure)
}

public enum Srf39IdlParseFailure: Sendable, Equatable {
    case invalidJson(detail: String)
    case invalidRoot(detail: String)
    case invalidSchema(detail: String)
    case unsupportedIdlNode(path: String, kind: String)
}

// MARK: - FFI conversions

extension SolanaRenderOutcome {
    init(_ ffi: RenderOutcomeFfi) {
        switch ffi {
        case let .rendered(instruction):
            self = .rendered(SolanaRenderedInstruction(instruction))
        case let .unsupported(reason):
            self = .unsupported(SolanaUnsupportedReason(reason))
        }
    }
}

extension SolanaUnsupportedReason {
    init(_ ffi: UnsupportedReasonFfi) {
        switch ffi {
        case let .idlNotFound(programId):
            self = .idlNotFound(programId: programId)
        case let .instructionNotRecognized(programId):
            self = .instructionNotRecognized(programId: programId)
        case let .instructionDecodeFailed(programId, instruction):
            self = .instructionDecodeFailed(programId: programId, instruction: instruction)
        }
    }
}

extension SolanaRenderedInstruction {
    init(_ ffi: RenderedInstructionFfi) {
        canonical = InstructionDisplay(ffi.canonical)
        idl = Srf39IdlBinding(ffi.idl)
        hints = SolanaDisplayHints(ffi.hints)
        presentation = SolanaPresentationOverlay(
            annotations: ffi.presentation.annotations.map(SolanaFieldAnnotation.init),
            tokenAmounts: ffi.presentation.tokenAmounts.map {
                SolanaPresentedTokenAmount(fieldIndex: Int($0.fieldIndex), value: $0.value,
                                           mint: $0.mint, decimals: $0.decimals)
            }
        )
        diagnostics = ffi.diagnostics.map(SolanaDiagnostic.init)
    }
}

extension InstructionDisplay {
    init(_ ffi: Srf39InstructionDisplayFfi) {
        self.init(
            intent: ffi.intent,
            interpolatedIntent: ffi.interpolatedIntent,
            fields: ffi.fields.map { DisplayField(label: $0.label, value: $0.value) }
        )
    }
}

extension Srf39IdlBinding {
    init(_ ffi: IdlBindingFfi) {
        programId = ffi.programId
        programName = ffi.programName
        sha256 = ffi.sha256Hex
        digestPinned = ffi.digestPinned
        provenance = Srf39IdlProvenance(ffi.provenance)
    }
}

extension Srf39IdlProvenance {
    init(_ ffi: IdlProvenanceFfi) {
        self.init(
            sourceId: ffi.sourceId,
            origin: Srf39IdlOrigin(ffi.origin),
            expectedSHA256: ffi.expectedSha256Hex,
            version: ffi.version,
            reference: ffi.reference
        )
    }

    var ffi: IdlProvenanceFfi {
        IdlProvenanceFfi(
            sourceId: sourceId,
            origin: origin.ffi,
            expectedSha256Hex: expectedSHA256,
            version: version,
            reference: reference
        )
    }
}

extension Srf39IdlOrigin {
    init(_ ffi: IdlOriginFfi) {
        switch ffi {
        case .bundled: self = .bundled
        case .localFile: self = .localFile
        case let .programMetadataCanonical(authority): self = .programMetadataCanonical(authority: authority)
        case let .other(detail): self = .other(detail)
        }
    }

    var ffi: IdlOriginFfi {
        switch self {
        case .bundled: return .bundled
        case .localFile: return .localFile
        case let .programMetadataCanonical(authority): return .programMetadataCanonical(authority: authority)
        case let .other(detail): return .other(detail: detail)
        }
    }
}

extension ResolvedSrf39Idl {
    var ffi: ResolvedSrf39IdlFfi {
        ResolvedSrf39IdlFfi(programId: programId, json: json, provenance: provenance.ffi)
    }
}

extension SolanaDisplayHints {
    init(_ ffi: DisplayHintsFfi) {
        instructionName = ffi.instructionName
        interpolatedIntentSuppressed = ffi.interpolatedIntentSuppressed
        amounts = ffi.amounts.map(SolanaAmountHint.init)
        times = ffi.times.map(SolanaTimeHint.init)
        publicKeyArguments = ffi.publicKeyArguments.map(SolanaPublicKeyArgumentHint.init)
        accounts = ffi.accounts.map(SolanaAccountHint.init)
        linkedAccountReads = ffi.linkedAccountReads.map(SolanaLinkedAccountRead.init)
    }
}

extension SolanaAmountHint {
    init(_ ffi: AmountHintFfi) {
        fieldIndex = ffi.fieldIndex.map(Int.init)
        argument = ffi.argument
        member = ffi.member
        rawValue = ffi.rawValue
        degraded = ffi.degraded
        decimals = SolanaDecimalsSource(ffi.decimals)
        unit = SolanaUnitSource(ffi.unit)
        token = ffi.token.map { SolanaTokenHint(mint: $0.mint) }
    }
}

extension SolanaTimeHint {
    init(_ ffi: TimeHintFfi) {
        fieldIndex = ffi.fieldIndex.map(Int.init)
        argument = ffi.argument
        member = ffi.member
        rawValue = ffi.rawValue
        switch ffi.display {
        case let .dateTime(ticksPerSecond):
            display = .dateTime(ticksPerSecond: ticksPerSecond)
        case let .duration(ticksPerSecond):
            display = .duration(ticksPerSecond: ticksPerSecond)
        }
        seconds = ffi.seconds
        formatted = ffi.formatted
    }
}

extension SolanaDecimalsSource {
    init(_ ffi: DecimalsSourceFfi) {
        switch ffi {
        case .implicitZero:
            self = .implicitZero
        case let .literal(value):
            self = .literal(value)
        case let .accountField(account, address, linkedAccount, path, resolved):
            self = .accountField(
                account: account,
                address: address,
                linkedAccount: linkedAccount,
                path: path,
                resolved: resolved
            )
        case .unsatisfied:
            self = .unsatisfied
        }
    }
}

extension SolanaUnitSource {
    init(_ ffi: UnitSourceFfi) {
        switch ffi {
        case .absent:
            self = .absent
        case let .literal(value):
            self = .literal(value)
        case let .accountField(account, address, linkedAccount, path, resolved):
            self = .accountField(
                account: account,
                address: address,
                linkedAccount: linkedAccount,
                path: path,
                resolved: resolved
            )
        case .unsatisfied:
            self = .unsatisfied
        }
    }
}

extension SolanaPublicKeyArgumentHint {
    init(_ ffi: PublicKeyArgumentHintFfi) {
        fieldIndex = ffi.fieldIndex.map(Int.init)
        argument = ffi.argument
        member = ffi.member
        element = ffi.element.map(Int.init)
        address = ffi.address
    }
}

extension SolanaAccountHint {
    init(_ ffi: AccountHintFfi) {
        fieldIndex = ffi.fieldIndex.map(Int.init)
        name = ffi.name
        address = ffi.address
        label = ffi.label
        linkedAccount = ffi.linkedAccount
        consumed = ffi.consumed
    }
}

extension SolanaLinkedAccountRead {
    init(_ ffi: LinkedAccountReadFfi) {
        address = ffi.address
        switch ffi.status {
        case let .fetched(owner, length):
            status = .fetched(owner: owner, length: Int(length))
        case .missing:
            status = .missing
        }
    }
}

extension SolanaFieldAnnotation {
    init(_ ffi: FieldAnnotationFfi) {
        fieldIndex = Int(ffi.fieldIndex)
        switch ffi.kind {
        case let .tokenAmount(symbol, name, sourceAddress, appliesToValue):
            kind = .tokenAmount(
                symbol: symbol,
                name: name,
                sourceAddress: sourceAddress,
                appliesToValue: appliesToValue
            )
        case let .tokenMint(symbol, name):
            kind = .tokenMint(symbol: symbol, name: name)
        case let .addressLabel(label, source):
            kind = .addressLabel(label: label, source: source)
        }
    }
}

extension SolanaTokenMetadata {
    var ffi: TokenMetadataFfi {
        TokenMetadataFfi(
            symbol: symbol,
            name: name,
            decimals: decimals,
            tokenProgram: tokenProgram
        )
    }
}

extension SolanaAddressLabel {
    var ffi: AddressLabelFfi {
        AddressLabelFfi(label: label, source: source)
    }
}

extension SolanaDiagnostic {
    init(_ ffi: FormatDiagnosticFfi) {
        code = ffi.code
        message = ffi.message
        switch ffi.severity {
        case .info: severity = .info
        case .warning: severity = .warning
        }
    }
}

extension Srf39IdlRejection {
    init(_ ffi: IdlRejectionFfi) {
        switch ffi {
        case let .programMismatch(requested, declared):
            self = .programMismatch(requested: requested, declared: declared)
        case let .digestMismatch(expected, actual):
            self = .digestMismatch(expected: expected, actual: actual)
        case let .invalid(failure):
            self = .invalid(Srf39IdlParseFailure(failure))
        }
    }
}

extension Srf39IdlParseFailure {
    init(_ ffi: IdlParseFailureFfi) {
        switch ffi {
        case let .invalidJson(detail): self = .invalidJson(detail: detail)
        case let .invalidRoot(detail): self = .invalidRoot(detail: detail)
        case let .invalidSchema(detail): self = .invalidSchema(detail: detail)
        case let .unsupportedIdlNode(path, kind): self = .unsupportedIdlNode(path: path, kind: kind)
        }
    }
}
