import Foundation
import SolanaClearsign

/// Generic presentation of an SDK result. Labels, field order and sentences
/// come from the IDL. Values and diagnostics come from the SDK; the only
/// text transformation here is address shortening, with the full value retained.
struct InstructionPreview {
    let rendered: SolanaRenderedInstruction

    struct Field: Identifiable {
        let id: Int
        let label: String
        let value: String
        let address: String?
        let addressLabel: SolanaAddressLabel?
        let isAmount: Bool
        let isRaw: Bool
    }

    var explanation: String? { rendered.canonical.interpolatedIntent }
    var diagnostics: [SolanaDiagnostic] { rendered.diagnostics }

    var fields: [Field] {
        rendered.canonical.fields.indices.map { index in
            let canonical = rendered.canonical.fields[index]
            let amount = rendered.hints.amounts.first { $0.fieldIndex == index }
            let presentedAmount = rendered.presentation.tokenAmount(for: index)
            let account = rendered.hints.accounts.first { $0.fieldIndex == index }
            let address = account?.address
                ?? rendered.hints.publicKeyArguments.first { $0.fieldIndex == index }?.address
            let value = presentedAmount?.value ?? address.map(Self.shortAddress) ?? canonical.value

            return Field(id: index, label: canonical.label, value: value, address: address,
                         addressLabel: address == nil ? nil : addressLabel(at: index),
                         isAmount: amount != nil,
                         isRaw: presentedAmount.map { $0.decimals == nil } ?? (amount?.degraded == true))
        }
    }

    private func addressLabel(at index: Int) -> SolanaAddressLabel? {
        for annotation in rendered.presentation.annotations(for: index) {
            if case let .addressLabel(label, source) = annotation.kind {
                return SolanaAddressLabel(label: label, source: source)
            }
        }
        return nil
    }

    static func shortAddress(_ address: String) -> String {
        guard address.count > 16 else { return address }
        return "\(address.prefix(6))…\(address.suffix(6))"
    }
}
