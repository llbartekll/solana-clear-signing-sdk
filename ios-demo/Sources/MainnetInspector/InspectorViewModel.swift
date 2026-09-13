import Foundation
import SolanaClearsign

enum TransactionDataSource: String, Sendable {
    case live = "Live RPC"
    case snapshot = "Bundled snapshot"
}

enum InstructionRenderResult: Sendable, Equatable {
    case rendered(SolanaRenderedInstruction)
    case unsupported(SolanaUnsupportedReason)
    case failed(String)
}

struct InspectedInstruction: Identifiable, Sendable, Equatable {
    let instruction: ParsedInstruction
    let result: InstructionRenderResult

    var id: Int { instruction.id }
}

struct TransactionInspection: Sendable, Equatable {
    let transaction: ParsedTransaction
    let cluster: String
    let programId: String
    let transactionSource: TransactionDataSource
    let accountSource: String
    let targetInstructionIndex: Int?
    let instructions: [InspectedInstruction]

    var renderedCount: Int {
        instructions.reduce(into: 0) { count, item in
            if case .rendered = item.result { count += 1 }
        }
    }

    /// Keep the selected example in focus, including unsupported or failed results.
    /// For pasted signatures, show all instructions belonging to this program.
    var primaryInstructions: [InspectedInstruction] {
        instructions.filter { item in
            if let targetInstructionIndex { return item.id == targetInstructionIndex }
            return item.instruction.input.programId == programId
        }
    }

    var otherInstructions: [InspectedInstruction] {
        let primaryIds = Set(primaryInstructions.map(\.id))
        return instructions.filter { !primaryIds.contains($0.id) }
    }
}

@MainActor
final class InspectorViewModel: ObservableObject {
    @Published private(set) var examples: [CatalogExample] = []
    @Published var signatureInput = ""
    /// The cluster a pasted signature is looked up on.
    @Published private(set) var selectedCluster = ""
    @Published private(set) var selectedExampleId: String?
    @Published private(set) var inspection: TransactionInspection?
    @Published private(set) var isLoading = false
    @Published private(set) var errorMessage: String?

    private let resources: InspectorResources?
    private let rpcFactory: @Sendable (ExampleCatalog) -> (any SolanaRPCServing)?
    private var loadTask: Task<Void, Never>?
    private var requestId = UUID()

    init(bundle: Bundle = .main) {
        let apiKey = AppConfig.alchemyAPIKey
        rpcFactory = { catalog in
            RPCEndpoint.url(for: catalog.rpc, alchemyAPIKey: apiKey).map { SolanaRPCClient(endpoint: $0) }
        }
        do {
            let resources = try InspectorResources(bundle: bundle)
            self.resources = resources
            examples = resources.catalogs.flatMap { $0.catalog.examples }
            selectedCluster = resources.catalogs.first?.cluster ?? ""
        } catch {
            resources = nil
            errorMessage = error.localizedDescription
        }
    }

    init(
        resources: InspectorResources,
        rpc: @escaping @Sendable (ExampleCatalog) -> (any SolanaRPCServing)?
    ) {
        self.resources = resources
        rpcFactory = rpc
        examples = resources.catalogs.flatMap { $0.catalog.examples }
        selectedCluster = resources.catalogs.first?.cluster ?? ""
    }

    private static let primaryExampleIds = [
        "init-subscription-authority", "create-recurring-delegation", "revoke-delegation",
    ]

    var primaryExamples: [CatalogExample] {
        Self.primaryExampleIds.compactMap { id in examples.first { $0.id == id } }
    }

    var additionalExamples: [CatalogExample] {
        examples.filter { !Self.primaryExampleIds.contains($0.id) }
    }

    var selectedExample: CatalogExample? {
        examples.first { $0.id == selectedExampleId }
    }

    func select(_ example: CatalogExample) {
        guard let match = resources?.example(signature: example.signature) else { return }
        selectedExampleId = example.id
        signatureInput = example.signature
        selectedCluster = match.repository.cluster
        start(signature: example.signature, repository: match.repository, example: match.example)
    }

    func loadInitialExample() {
        guard signatureInput.isEmpty, let first = primaryExamples.first else { return }
        select(first)
    }

    func inspectInput() {
        guard let signature = TransactionParser.validateSignature(signatureInput) else {
            errorMessage = TransactionParserError.invalidSignature.localizedDescription
            return
        }
        guard let resources else {
            errorMessage = InspectorError.notConfigured.localizedDescription
            return
        }
        if let match = resources.example(signature: signature) {
            selectedExampleId = match.example.id
            selectedCluster = match.repository.cluster
            start(signature: signature, repository: match.repository, example: match.example)
            return
        }
        guard let repository = resources.repository(for: selectedCluster) else {
            errorMessage = InspectorError.notConfigured.localizedDescription
            return
        }
        selectedExampleId = nil
        start(signature: signature, repository: repository, example: nil)
    }

    private func start(signature: String, repository: ExampleRepository, example: CatalogExample?) {
        loadTask?.cancel()
        requestId = UUID()
        let currentRequest = requestId
        inspection = nil
        errorMessage = nil
        isLoading = true
        loadTask = Task { [weak self] in
            await self?.perform(
                signature: signature,
                repository: repository,
                example: example,
                requestId: currentRequest
            )
        }
    }

    private func perform(
        signature: String,
        repository: ExampleRepository,
        example: CatalogExample?,
        requestId: UUID
    ) async {
        do {
            guard let resources else { throw InspectorError.notConfigured }
            let rpc = rpcFactory(repository.catalog)
            let loaded = try await loadTransaction(
                signature: signature,
                example: example,
                repository: repository,
                rpc: rpc
            )
            try Task.checkCancellation()
            let parsed = try TransactionParser.parse(loaded.transaction)
            if let example {
                try validate(parsed, against: example, programId: repository.catalog.programId)
            }
            let provider = InspectorAccountProvider(
                rpc: rpc,
                snapshots: example == nil ? [:] : repository.accountSnapshots
            )
            // One client per inspection: it shares the long-lived IDL source and
            // the cluster's registry but binds this inspection's account provider.
            let client = SolanaClearSigningClient(
                idlSource: resources.idlSource,
                accountDataProvider: provider,
                presentationProvider: resources.registry(for: repository.cluster)
            )
            var rendered: [InspectedInstruction] = []
            rendered.reserveCapacity(parsed.instructions.count)
            for instruction in parsed.instructions {
                try Task.checkCancellation()
                do {
                    let outcome = try await client.render(instruction: instruction.input)
                    let result: InstructionRenderResult
                    switch outcome {
                    case let .rendered(instruction): result = .rendered(instruction)
                    case let .unsupported(reason): result = .unsupported(reason)
                    }
                    rendered.append(InspectedInstruction(instruction: instruction, result: result))
                } catch {
                    rendered.append(InspectedInstruction(
                        instruction: instruction,
                        result: .failed(error.localizedDescription)
                    ))
                }
            }
            let accountSource = await provider.sourceSummary()
            try Task.checkCancellation()
            guard requestId == self.requestId else { return }
            inspection = TransactionInspection(
                transaction: parsed,
                cluster: repository.cluster,
                programId: repository.catalog.programId,
                transactionSource: loaded.source,
                accountSource: accountSource,
                targetInstructionIndex: example?.instructionIndex,
                instructions: rendered
            )
            isLoading = false
        } catch is CancellationError {
            return
        } catch {
            guard requestId == self.requestId else { return }
            errorMessage = error.localizedDescription
            isLoading = false
        }
    }

    private func loadTransaction(
        signature: String,
        example: CatalogExample?,
        repository: ExampleRepository,
        rpc: (any SolanaRPCServing)?
    ) async throws -> (transaction: RPCTransaction, source: TransactionDataSource) {
        if let rpc {
            let transaction: RPCTransaction
            do {
                transaction = try await rpc.transaction(signature: signature)
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                guard let example else { throw error }
                return (try repository.transaction(for: example), .snapshot)
            }
            return (transaction, .live)
        }
        guard let example else { throw InspectorError.rpcRequired(cluster: repository.cluster) }
        return (try repository.transaction(for: example), .snapshot)
    }

    private func validate(
        _ transaction: ParsedTransaction,
        against example: CatalogExample,
        programId: String
    ) throws {
        guard transaction.signature == example.signature,
              transaction.slot == example.slot,
              transaction.version.caseInsensitiveCompare(example.version) == .orderedSame,
              transaction.succeeded,
              transaction.instructions.indices.contains(example.instructionIndex)
        else {
            throw InspectorError.fixtureMismatch(example.id)
        }
        let target = transaction.instructions[example.instructionIndex]
        guard target.input.programId == programId,
              target.input.instructionData.first == example.opcode
        else {
            throw InspectorError.fixtureMismatch(example.id)
        }
    }
}

enum InspectorError: Error, LocalizedError, Equatable {
    case notConfigured
    case rpcRequired(cluster: String)
    case fixtureMismatch(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured:
            return "The inspector could not load its sRFC 39 resources."
        case let .rpcRequired(cluster):
            return "Arbitrary \(cluster) signatures need an RPC endpoint."
        case let .fixtureMismatch(id):
            return "The fetched transaction does not match example \(id)."
        }
    }
}
