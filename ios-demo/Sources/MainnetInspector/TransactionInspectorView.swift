import SwiftUI
import SolanaClearsign

struct TransactionInspectorView: View {
    @StateObject private var model = InspectorViewModel()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    intro
                    examples
                    result
                    DisclosureGroup("Inspect another Devnet transaction") {
                        signatureSearch.padding(.top, 10)
                    }
                    .font(.subheadline)
                }
                .padding()
                .frame(maxWidth: 760, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
            .background(Color(uiColor: .systemGroupedBackground))
            .navigationTitle("Subscriptions")
            .navigationBarTitleDisplayMode(.inline)
        }
        .task { model.loadInitialExample() }
    }

    private var intro: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Know what you approve")
                    .font(.title2.bold())
                Spacer()
                StatusPill(text: "Devnet", color: .indigo)
            }
            Text("Explore three separate examples of subscription permissions on Solana.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Label("Read-only demo · nothing is signed or submitted", systemImage: "eye")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var signatureSearch: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Paste a Devnet signature", text: $model.signatureInput)
                .font(.caption.monospaced())
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(12)
                .background(.background, in: RoundedRectangle(cornerRadius: 12))
            Button("Inspect transaction") { model.inspectInput() }
                .buttonStyle(.borderedProminent)
                .disabled(model.isLoading)
        }
    }

    private var examples: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(model.primaryExamples.enumerated()), id: \.element.id) { index, example in
                exampleButton(example, number: index + 1)
            }
            if !model.additionalExamples.isEmpty {
                DisclosureGroup("More examples (\(model.additionalExamples.count))") {
                    VStack(spacing: 10) {
                        ForEach(model.additionalExamples) { example in
                            exampleButton(example)
                        }
                    }
                    .padding(.top, 10)
                }
                .font(.subheadline)
                .padding(.top, 4)
            }
        }
    }

    private func exampleButton(_ example: CatalogExample, number: Int? = nil) -> some View {
        let selected = model.selectedExampleId == example.id
        return Button {
            model.select(example)
        } label: {
            HStack(spacing: 12) {
                if let number {
                    Text("\(number)")
                        .font(.subheadline.bold())
                        .frame(width: 28, height: 28)
                        .background(Color.blue.opacity(0.1), in: Circle())
                }
                Text(example.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.primary)
                    .multilineTextAlignment(.leading)
                Spacer(minLength: 8)
                Image(systemName: selected ? "checkmark.circle.fill" : "chevron.right")
                    .foregroundStyle(selected ? Color.blue : .secondary)
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.background, in: RoundedRectangle(cornerRadius: 14))
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(selected ? Color.blue : Color.clear, lineWidth: 2)
            }
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("example-\(example.id)")
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityHint(example.summary)
    }

    @ViewBuilder
    private var result: some View {
        if model.isLoading {
            HStack(spacing: 12) {
                ProgressView()
                Text("Loading transaction preview…")
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else if let error = model.errorMessage {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
                .padding()
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 14))
        } else if let inspection = model.inspection {
            inspectionView(inspection)
                .id(inspection.transaction.signature)
        }
    }

    private func inspectionView(_ inspection: TransactionInspection) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Transaction preview")
                    .font(.headline)
                Spacer()
                StatusPill(
                    text: inspection.transaction.succeeded ? "Succeeded" : "Failed",
                    color: inspection.transaction.succeeded ? .green : .red
                )
            }
            StatusPill(text: inspection.transactionSource.rawValue, color: .blue)

            if inspection.primaryInstructions.isEmpty {
                Text("No top-level Subscriptions instruction was found. Expand the other instructions to inspect this transaction.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            ForEach(inspection.primaryInstructions) { item in
                InstructionInspectionCard(item: item)
            }

            if !inspection.otherInstructions.isEmpty {
                DisclosureGroup {
                    VStack(spacing: 12) {
                        ForEach(inspection.otherInstructions) { item in
                            InstructionInspectionCard(item: item)
                        }
                    }
                    .padding(.top, 10)
                } label: {
                    HStack {
                        Text("Other instructions (\(inspection.otherInstructions.count))")
                        if inspection.otherInstructions.contains(where: {
                            if case .failed = $0.result { return true }
                            return false
                        }) {
                            StatusPill(text: "Render error", color: .red)
                        }
                    }
                }
                .font(.subheadline)
            }

            DisclosureGroup("Transaction details") {
                VStack(alignment: .leading, spacing: 10) {
                    ValueRow(label: "Signature", value: inspection.transaction.signature)
                    ValueRow(label: "Network", value: inspection.cluster)
                    ValueRow(label: "Slot", value: String(inspection.transaction.slot))
                    ValueRow(label: "Version", value: inspection.transaction.version)
                    ValueRow(label: "Account data", value: inspection.accountSource)
                    Text("\(inspection.renderedCount) of \(inspection.instructions.count) top-level instructions rendered. Inner instructions are not included in this preview.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.top, 10)
            }
            .font(.subheadline)
        }
    }
}

private struct InstructionInspectionCard: View {
    let item: InspectedInstruction

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ClearSigningPanel(result: item.result)
            DisclosureGroup("Technical details") {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Instruction #\(item.id)")
                        .font(.caption.weight(.semibold))
                    if case let .rendered(rendered) = item.result {
                        CanonicalDisplayPanel(display: rendered.canonical)
                    }
                    RawInstructionPanel(instruction: item.instruction)
                    if case let .rendered(rendered) = item.result {
                        RenderMetadataPanel(rendered: rendered)
                    }
                }
                .padding(.top, 10)
            }
            .font(.subheadline)
        }
        .padding()
        .background(.background, in: RoundedRectangle(cornerRadius: 16))
    }
}

private struct CanonicalDisplayPanel: View {
    let display: InstructionDisplay

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Original IDL display")
                .font(.subheadline.bold())
            Text(display.intent)
                .font(.subheadline)
            if let sentence = display.interpolatedIntent {
                Text(sentence)
                    .font(.caption)
                    .textSelection(.enabled)
            }
            ForEach(Array(display.fields.enumerated()), id: \.offset) { _, field in
                ValueRow(label: field.label, value: field.value)
            }
        }
    }
}

private struct RawInstructionPanel: View {
    let instruction: ParsedInstruction

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Raw instruction")
                .font(.subheadline.bold())
            ValueRow(label: "Program", value: instruction.input.programId)
            ValueRow(label: "Data (base58)", value: instruction.dataBase58.isEmpty ? "<empty>" : instruction.dataBase58)
            ValueRow(label: "Data (base64)", value: instruction.input.instructionData.base64EncodedString())
            Text("Account metas")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            ForEach(Array(instruction.input.accounts.enumerated()), id: \.offset) { index, account in
                HStack(alignment: .top, spacing: 8) {
                    Text("\(index)")
                        .font(.caption2.monospaced())
                        .foregroundStyle(.secondary)
                    Text(account.pubkey)
                        .font(.caption2.monospaced())
                        .textSelection(.enabled)
                    Spacer(minLength: 4)
                    if account.isSigner { Text("S").foregroundStyle(.orange) }
                    if account.isWritable { Text("W").foregroundStyle(.blue) }
                }
                .font(.caption2.bold())
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
    }
}

private struct ClearSigningPanel: View {
    let result: InstructionRenderResult

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Clear signing")
                .font(.subheadline.bold())
            switch result {
            case let .rendered(rendered):
                renderedBody(rendered)
            case let .unsupported(reason):
                Label(unsupportedMessage(reason), systemImage: "questionmark.circle")
                    .foregroundStyle(.secondary)
            case let .failed(message):
                Label(message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .background(Color.blue.opacity(0.05), in: RoundedRectangle(cornerRadius: 12))
    }

    @ViewBuilder
    private func renderedBody(_ rendered: SolanaRenderedInstruction) -> some View {
        let preview = InstructionPreview(rendered: rendered)
        Text(rendered.canonical.intent)
            .font(.title3.weight(.semibold))
        if let explanation = preview.explanation {
            Text(explanation)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        ForEach(preview.fields) { field in
            let annotations = rendered.presentation.annotations(for: field.id).filter {
                // Address labels are shown with the address in PreviewValueRow.
                if case .addressLabel = $0.kind, field.addressLabel != nil { return false }
                return true
            }
            VStack(alignment: .leading, spacing: 6) {
                PreviewValueRow(field: field)
                ForEach(Array(annotations.enumerated()), id: \.offset) { _, annotation in
                    AnnotationPill(kind: annotation.kind)
                }
            }
            .padding(.vertical, 4)
        }
        DiagnosticsPanel(diagnostics: preview.diagnostics)
    }

    private func unsupportedMessage(_ reason: SolanaUnsupportedReason) -> String {
        switch reason {
        case .idlNotFound:
            return "No bundled sRFC 39 IDL for this program. The raw instruction remains visible."
        case .instructionNotRecognized:
            return "The IDL does not describe this instruction. The raw instruction remains visible."
        case let .instructionDecodeFailed(_, instruction):
            return "Instruction \(instruction) matched but its bytes could not be decoded."
        }
    }

}

private struct PreviewValueRow: View {
    let field: InstructionPreview.Field

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(field.label)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            if let address = field.address {
                if let alias = field.addressLabel, alias.source == "local demo" {
                    DisclosureGroup {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("Resolved in demo app from:")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            addressText(address, fullAddress: address)
                        }
                        .padding(.top, 4)
                    } label: {
                        Text(alias.label)
                            .font(.body.weight(.semibold))
                    }
                    .tint(.secondary)
                    .id(address)
                } else {
                    if let alias = field.addressLabel {
                        Text(alias.label)
                            .font(.body.weight(.semibold))
                        if let source = alias.source {
                            Text(source)
                                .font(.caption2)
                                .foregroundStyle(.purple)
                        }
                    }
                    addressText(field.value, fullAddress: address)
                }
            } else {
                Text(field.value)
                    .font(field.isAmount ? .title2.weight(.semibold) : .body)
                    .foregroundStyle(field.isRaw ? Color.orange : .primary)
                    .textSelection(.enabled)
            }
        }
    }

    private func addressText(_ value: String, fullAddress: String) -> some View {
        Text(value)
            .font(.subheadline.monospaced())
            .accessibilityLabel(fullAddress)
            .accessibilityHint("Touch and hold to copy the full address.")
            .contextMenu {
                Button {
                    UIPasteboard.general.string = fullAddress
                } label: {
                    Label("Copy full address", systemImage: "doc.on.doc")
                }
            }
    }
}

private struct RenderMetadataPanel: View {
    let rendered: SolanaRenderedInstruction

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Divider()
            Text(idlFooter(rendered.idl))
                .font(.caption2.monospaced())
                .foregroundStyle(.secondary)
        }
    }

    private func idlFooter(_ binding: Srf39IdlBinding) -> String {
        let digest = String(binding.sha256.prefix(12))
        let pinned = binding.digestPinned ? "pinned" : "UNPINNED"
        return "IDL \(binding.programName) · sha256 \(digest)… (\(pinned)) · \(binding.provenance.sourceId)"
    }

}

/// The engine owns diagnostic text and severity; the UI only chooses styling.
private struct DiagnosticsPanel: View {
    let diagnostics: [SolanaDiagnostic]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !diagnostics.isEmpty {
                Divider()
                Text("Diagnostics")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                ForEach(Array(diagnostics.enumerated()), id: \.offset) { _, diagnostic in
                    HStack(alignment: .top, spacing: 6) {
                        Image(systemName: diagnostic.severity == .warning ? "exclamationmark.triangle" : "info.circle")
                            .foregroundStyle(diagnostic.severity == .warning ? .orange : .secondary)
                        Text(diagnostic.message)
                            .font(.caption)
                            .foregroundStyle(diagnostic.severity == .warning ? .orange : .secondary)
                            .textSelection(.enabled)
                    }
                }
            }
        }
    }
}

private struct AnnotationPill: View {
    let kind: SolanaFieldAnnotation.Kind

    var body: some View {
        switch kind {
        case let .tokenAmount(symbol, _, _, appliesToValue):
            tokenPill(symbol: symbol, appliesToValue: appliesToValue)
        case let .tokenMint(symbol, name):
            tokenPill(symbol: name.map { "\(symbol) · \($0)" } ?? symbol, appliesToValue: true)
        case let .addressLabel(label, source):
            StatusPill(text: source.map { "\(label) (\($0))" } ?? label, color: .purple)
        }
    }

    private func tokenPill(symbol: String, appliesToValue: Bool) -> some View {
        StatusPill(
            text: appliesToValue ? symbol : "Token: \(symbol)",
            color: appliesToValue ? .blue : .orange
        )
    }
}

private struct ValueRow: View {
    let label: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.caption.monospaced())
                .textSelection(.enabled)
        }
    }
}

private struct StatusPill: View {
    let text: String
    let color: Color

    var body: some View {
        Text(text)
            .font(.caption2.bold())
            .foregroundStyle(color)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(color.opacity(0.12), in: Capsule())
    }
}
