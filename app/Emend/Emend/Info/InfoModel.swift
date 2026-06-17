import Combine
import EmendCore
import Foundation

/// Drives the info sidebar (US6 · FR-029/030/031a): live word/char/reading-time
/// stats, N-of-M task completion, and a heading outline. Recomputed from the
/// active document's core handle (`stats`/`outline` are pure, fast scans) —
/// immediately on document switch and debounced on edit, so continuous typing
/// doesn't recompute per keystroke.
@MainActor
final class InfoModel: ObservableObject {
    @Published private(set) var stats: DocStats?
    @Published private(set) var outline: [OutlineItem] = []

    private var handle: OpenDocHandle?
    private var refreshTask: Task<Void, Never>?
    private let debounce: Duration = .milliseconds(150)

    /// Point the sidebar at a new active document (or `nil` when none is open).
    func setActiveDocument(_ handle: OpenDocHandle?) {
        self.handle = handle
        refresh(immediate: true)
    }

    /// Recompute insight. Coalesces rapid edits: each cancels the pending refresh
    /// and re-arms the debounce.
    func refresh(immediate: Bool = false) {
        refreshTask?.cancel()
        guard let handle else {
            stats = nil
            outline = []
            return
        }
        refreshTask = Task { [weak self, debounce] in
            if !immediate {
                try? await Task.sleep(for: debounce)
            }
            if Task.isCancelled { return }
            let computed = await Self.compute(handle)
            if Task.isCancelled { return }
            guard let self else { return }
            stats = computed.stats
            outline = computed.outline
        }
    }

    /// Compute off the main actor (NFR-001): whole-document scans that can exceed a
    /// frame on a large note. Failures leave the previous insight in place.
    private static func compute(
        _ handle: OpenDocHandle
    ) async -> (stats: DocStats?, outline: [OutlineItem]) {
        await Task.detached(priority: .utility) {
            let stats = try? handle.stats()
            let outline = (try? handle.outline()) ?? []
            return (stats, outline)
        }.value
    }
}
