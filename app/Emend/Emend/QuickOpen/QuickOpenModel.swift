import Combine
import EmendCore
import Foundation

/// Drives the ⌘P Quick Open palette (US3 · FR-017/FR-018, NFR-002).
///
/// Bridges the core's streaming, supersedable `quickOpenQuery` to SwiftUI: each
/// keystroke starts a fresh query that supersedes the prior one (the core cancels
/// the previous in-flight search; a monotonic `generation` additionally guards
/// against a late batch from a superseded query landing after the next one began).
/// Ranked `SearchHit`s stream in via a `SearchSink`; Return opens the selection.
@MainActor
final class QuickOpenModel: ObservableObject {
    @Published var query = ""
    @Published private(set) var results: [SearchHit] = []
    @Published var selection = 0
    @Published private(set) var isPresented = false

    private var workspace: WorkspaceHandle?
    private var onOpen: ((URL) -> Void)?
    /// Cancellation handle for the in-flight query (supersede / palette close).
    private var handle: SearchHandle?
    /// Monotonic query id; a sink ignores batches from a superseded generation.
    private var generation: UInt64 = 0

    /// Wire the model to the workspace + a file-opener once the shell exists
    /// (mirrors `ConflictController.attach`, since these come from sibling
    /// `@StateObject`s the initialiser can't see).
    func attach(workspace: WorkspaceHandle, onOpen: @escaping (URL) -> Void) {
        self.workspace = workspace
        self.onOpen = onOpen
    }

    func present() {
        isPresented = true
    }

    func dismiss() {
        isPresented = false
        cancelInFlight()
        query = ""
        results = []
        selection = 0
    }

    /// Run the current query, superseding any in-flight one (called on each
    /// keystroke). An empty query clears the list without touching the core.
    func runQuery() {
        cancelInFlight()
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let workspace else {
            results = []
            selection = 0
            return
        }
        generation &+= 1
        let gen = generation
        results = []
        selection = 0
        let sink = QuickOpenSink(
            onBatch: { [weak self] batch in
                Task { @MainActor in self?.apply(batch: batch, generation: gen) }
            },
            onDone: {}
        )
        handle = workspace.quickOpenQuery(query: trimmed, sink: sink)
    }

    /// Move the highlighted row, clamped to the result range (arrow keys).
    func moveSelection(by delta: Int) {
        guard !results.isEmpty else { return }
        let next = selection + delta
        selection = min(max(0, next), results.count - 1)
    }

    /// Open the highlighted result in a tab and close the palette (Return).
    func openSelected() {
        guard results.indices.contains(selection) else { return }
        let url = URL(fileURLWithPath: results[selection].path)
        let opener = onOpen
        dismiss()
        opener?(url)
    }

    // MARK: - Private

    /// Apply a streamed batch, ignoring any that belongs to a superseded query.
    private func apply(batch: [SearchHit], generation: UInt64) {
        guard generation == self.generation else { return }
        results.append(contentsOf: batch)
        if selection >= results.count { selection = max(0, results.count - 1) }
    }

    private func cancelInFlight() {
        handle?.cancel()
        handle = nil
    }
}

/// Bridges the core's `SearchSink` callbacks (delivered on a background search
/// worker thread) to `@Sendable` closures. Holds only immutable closures, so it
/// is safely `Sendable` (mirrors `FsObserver`).
final class QuickOpenSink: SearchSink {
    private let batchHandler: @Sendable ([SearchHit]) -> Void
    private let doneHandler: @Sendable () -> Void

    init(
        onBatch: @escaping @Sendable ([SearchHit]) -> Void,
        onDone: @escaping @Sendable () -> Void
    ) {
        batchHandler = onBatch
        doneHandler = onDone
    }

    func onResults(batch: [SearchHit]) {
        batchHandler(batch)
    }

    func onDone() {
        doneHandler()
    }
}
