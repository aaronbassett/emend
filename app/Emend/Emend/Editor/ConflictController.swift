import Combine
import EmendCore
import Foundation

/// Detects when an open document changes on disk underneath the editor and lets
/// the user resolve it (FR-006c): reload from disk or keep the local buffer.
///
/// The app's own atomic autosaves are suppressed two ways: the core watcher's
/// `recordSelfWrite` (best-effort identity match) and a robust Swift-side time
/// window keyed by path — so a normal save never raises a false conflict.
@MainActor
final class ConflictController: ObservableObject {
    /// Tab ids whose file changed externally and await a user decision.
    @Published private(set) var conflicts: Set<UUID> = []

    private weak var tabs: TabModel?
    private weak var workspace: WorkspaceModel?
    private var recentSelfWrites: [String: Date] = [:]
    /// A save's own filesystem echo arrives within the watcher debounce (~400 ms);
    /// ignore changes to a path for a short window after we wrote it. The Rust
    /// watcher's identity-matched suppression is the primary guard; this is
    /// belt-and-suspenders, kept tight so a real edit isn't masked for long.
    private let selfWriteWindow: TimeInterval = 1.5

    /// Wire the controller to the models once (idempotent).
    func attach(tabs: TabModel, workspace: WorkspaceModel) {
        guard self.tabs == nil else { return }
        self.tabs = tabs
        self.workspace = workspace
        workspace.onExternalChange = { [weak self] event in self?.handleChange(event) }
        workspace.beforeMove = { [weak self] path in self?.tabs?.closeIfOpen(path: path) }
        tabs.onTabFlushed = { [weak self] url in self?.noteSelfWrite(url) }
    }

    func isConflicted(_ id: UUID?) -> Bool {
        guard let id else { return false }
        return conflicts.contains(id)
    }

    /// Resolve a flagged conflict (FR-006c). `reloadFromDisk` discards the local
    /// buffer; `keepMine` keeps it (the next autosave overwrites disk).
    func resolve(_ id: UUID, choice: ConflictChoice) {
        switch choice {
        case .reloadFromDisk:
            tabs?.reload(id)
        case .keepMine:
            // Write the kept buffer to disk now so it stops diverging from disk
            // (rather than waiting for the next incidental edit).
            tabs?.flush(id)
        @unknown default:
            break
        }
        conflicts.remove(id)
    }

    // MARK: - Private

    private func handleChange(_ event: ChangeEvent) {
        // Only an in-place modification is a reload-able conflict. A removed or
        // renamed-away file is gone, so "reload from disk" would read nothing and
        // silently lose the buffer — keep the user's buffer instead (the next
        // autosave re-creates the file). Created is a new sibling, not a conflict.
        guard case let .modified(path) = event else { return }
        guard let tab = tabs?.tab(forPath: path) else { return }
        let elapsed = recentSelfWrites[path].map { Date().timeIntervalSince($0) } ?? .infinity
        if elapsed < selfWriteWindow { return } // our own autosave, not external
        conflicts.insert(tab.id)
    }

    private func noteSelfWrite(_ url: URL) {
        let path = url.path(percentEncoded: false)
        let now = Date()
        recentSelfWrites[path] = now
        // Prune stale entries so the map can't grow unbounded over a session.
        recentSelfWrites = recentSelfWrites
            .filter { now.timeIntervalSince($0.value) < selfWriteWindow }
        workspace?.recordSelfWrite(path: path)
    }
}
