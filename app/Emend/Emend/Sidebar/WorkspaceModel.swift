import Combine
import EmendCore
import Foundation

/// Owns the Rust `WorkspaceHandle` plus the Swift-side concerns the core can't
/// hold: the security-scoped folder bookmarks (the sandbox scope Rust reads and
/// watches inside, research §A4) and their persistence across launches. The
/// handle is the source of truth for the workspace/index; this model adds the
/// scope lifecycle, the sidebar's root nodes, and per-location file watchers
/// (FR-006) that live-refresh the tree on external change.
@MainActor
final class WorkspaceModel: ObservableObject {
    let workspace: WorkspaceHandle

    /// Invoked (on the main actor) for each external filesystem change after the
    /// sidebar refreshes — the conflict controller uses it to flag open docs.
    var onExternalChange: ((ChangeEvent) -> Void)?

    /// Sidebar roots (the added locations). `revision` bumps only when this set
    /// changes, so the outline view reloads its top level exactly then (not on
    /// every incidental SwiftUI update — which would collapse expanded folders).
    @Published private(set) var roots: [WorkspaceNode] = []
    @Published private(set) var revision = 0
    /// Bumped when external filesystem changes need the outline to reload the
    /// affected (already-loaded) folders — see `consumePendingReloads()`.
    @Published private(set) var fsRefreshTick = 0

    /// Path-keyed app state the core models in memory but does not persist
    /// (favorites/pins/icons). Persisted Swift-side here and replayed into the
    /// core on launch — the data-model envisions a core-owned store, but the core
    /// has no persistence layer yet, so this mirrors how location bookmarks are
    /// persisted.
    struct AppState: Codable {
        var favorites: [String] = []
        var pinned: [String] = []
        var icons: [String: String] = [:]
    }

    private let defaults: UserDefaults
    private let bookmarksKey = "com.aaronbassett.Emend.locationBookmarks"
    private let appStateKey = "com.aaronbassett.Emend.appState"
    /// Per-location: the bookmark (for persistence) and the held security scope.
    private var bookmarks: [UInt64: Data] = [:]
    private var heldScopes: [UInt64: URL] = [:]
    private var watchers: [UInt64: WatchHandle] = [:]
    private var pendingReloads: [WorkspaceNode] = []
    private var appState = AppState()
    private lazy var fsObserver = FsObserver { [weak self] change in
        Task { @MainActor in self?.handleFsChange(change) }
    }

    /// Stable synthetic root for the Favorites group (stable identity matters for
    /// `NSOutlineView`); its children are refreshed when favorites change.
    private let favoritesGroup = WorkspaceNode(
        url: URL(fileURLWithPath: "/"),
        name: "Favorites",
        kind: .favorites
    )

    init(workspace: WorkspaceHandle = newWorkspace(), defaults: UserDefaults = .standard) {
        self.workspace = workspace
        self.defaults = defaults
        loadAppState()
        restorePersistedLocations()
        refreshFavorites()
    }

    /// Outline roots: the Favorites group (when non-empty) above the locations.
    var displayRoots: [WorkspaceNode] {
        appState.favorites.isEmpty ? roots : [favoritesGroup] + roots
    }

    func isFavorite(_ path: String) -> Bool {
        appState.favorites.contains(path)
    }

    func isPinned(_ path: String) -> Bool {
        appState.pinned.contains(path)
    }

    /// Toggle Favorite (FR-007). Structural — refreshes the Favorites group and
    /// bumps `revision` so the top level reloads.
    func toggleFavorite(_ path: String) {
        if let idx = appState.favorites.firstIndex(of: path) {
            appState.favorites.remove(at: idx)
        } else {
            appState.favorites.append(path)
        }
        try? workspace.setFavorite(path: path, favorite: isFavorite(path))
        saveAppState()
        refreshFavorites()
        revision += 1
    }

    /// Toggle Pinned (FR-007). A per-row indicator only — the caller reloads the
    /// affected row, so this does not bump `revision`.
    func togglePin(_ path: String) {
        if let idx = appState.pinned.firstIndex(of: path) {
            appState.pinned.remove(at: idx)
        } else {
            appState.pinned.append(path)
        }
        try? workspace.setPinned(path: path, pinned: isPinned(path))
        saveAppState()
    }

    /// The custom icon set for `path`, if any (display source of truth — the FFI
    /// exposes no icon getter).
    func icon(for path: String) -> FolderIcon? {
        FolderIcon(serialized: appState.icons[path])
    }

    /// Set or clear a folder's custom icon (FR-008). Persists Swift-side and
    /// forwards to the core for consistency.
    func setIcon(_ icon: FolderIcon?, for path: String) {
        if let icon {
            appState.icons[path] = icon.serialized
        } else {
            appState.icons.removeValue(forKey: path)
        }
        try? workspace.setFolderIcon(folderPath: path, icon: icon?.serialized)
        saveAppState()
    }

    /// Prompt for a folder and add it as a location.
    func addLocation() {
        guard let bookmark = try? SecurityScopedBookmarks.promptForFolder() else { return }
        try? register(bookmark: bookmark, persist: true)
    }

    func removeLocation(_ node: WorkspaceNode) {
        guard case let .location(id) = node.kind else { return }
        if let watcher = watchers.removeValue(forKey: id) { try? watcher.stop() }
        try? workspace.removeLocation(id: id)
        bookmarks.removeValue(forKey: id)
        if let url = heldScopes.removeValue(forKey: id) {
            url.stopAccessingSecurityScopedResource()
        }
        roots.removeAll { $0 === node }
        persistBookmarks()
        revision += 1
    }

    /// Lazily-listed children for `node`, cached on the node. The Favorites group
    /// is filled by `refreshFavorites`, never by directory listing.
    func children(of node: WorkspaceNode) -> [WorkspaceNode] {
        if node.kind == .favorites { return node.children ?? [] }
        if let cached = node.children { return cached }
        let listed = listChildren(of: node)
        node.children = listed
        return listed
    }

    /// Drop the cached children so the next `children(of:)` re-lists from disk
    /// (used by targeted refresh on external change).
    func invalidateChildren(of node: WorkspaceNode) {
        node.children = nil
    }

    /// The loaded node at `path`, searching the (expanded) location tree.
    func node(withPath path: String) -> WorkspaceNode? {
        func search(_ nodes: [WorkspaceNode]) -> WorkspaceNode? {
            for node in nodes {
                if node.path == path { return node }
                if let kids = node.children, let found = search(kids) { return found }
            }
            return nil
        }
        return search(roots)
    }

    /// Move `sourcePath` into the `target` folder/location (drag-drop reorganize,
    /// FR-004/FR-005). Invalidates the affected folders' caches and reloads.
    @discardableResult
    func move(sourcePath: String, into target: WorkspaceNode) -> Bool {
        guard target.isExpandable, target.kind != .favorites else { return false }
        let oldParent = URL(fileURLWithPath: sourcePath).deletingLastPathComponent().path
        guard oldParent != target.path else { return false } // no-op into same parent
        guard target.path != sourcePath, !target.path.hasPrefix(sourcePath + "/") else {
            return false // can't drop a folder into itself or a descendant
        }
        let newPath: String
        do {
            newPath = try workspace.moveNode(path: sourcePath, newParent: target.path)
        } catch {
            return false
        }
        repath(from: sourcePath, to: newPath)
        target.children = nil
        node(withPath: oldParent)?.children = nil
        refreshFavorites()
        revision += 1
        return true
    }

    /// Outline nodes whose (already-loaded) children changed on disk and need a
    /// targeted reload. Cleared on read.
    func consumePendingReloads() -> [WorkspaceNode] {
        defer { pendingReloads.removeAll() }
        return pendingReloads
    }

    /// Prime the watchers' self-write suppression with the file's current
    /// (mtime, len) so the app's own autosave doesn't echo as an external change
    /// (FR-006a). Best-effort: the conflict controller also time-windows saves.
    func recordSelfWrite(path: String) {
        var info = stat()
        guard stat(path, &info) == 0 else { return }
        let mtimeNs = UInt64(max(0, info.st_mtimespec.tv_sec)) * 1_000_000_000
            + UInt64(max(0, info.st_mtimespec.tv_nsec))
        let len = UInt64(max(0, info.st_size))
        for watcher in watchers.values {
            try? watcher.recordSelfWrite(path: path, mtimeNs: mtimeNs, len: len)
        }
    }
}

// MARK: - Private helpers (in an extension so the main type stays under the

// SwiftLint type-body-length limit as US2 grows).

private extension WorkspaceModel {
    /// React to an external filesystem change (FR-006): invalidate the affected
    /// loaded folder(s) and the Favorites group so the outline reloads them.
    func handleFsChange(_ change: ChangeEvent) {
        let paths: [String] = switch change {
        case let .created(path), let .modified(path), let .removed(path):
            [path]
        case let .renamed(from, to):
            [from, to]
        }
        var changed = false
        for path in paths {
            let parent = URL(fileURLWithPath: path).deletingLastPathComponent().path
            guard let node = node(withPath: parent) else { continue }
            node.children = nil
            pendingReloads.append(node)
            changed = true
        }
        if !appState.favorites.isEmpty {
            refreshFavorites()
            pendingReloads.append(favoritesGroup)
            changed = true
        }
        if changed { fsRefreshTick += 1 }
        onExternalChange?(change)
    }

    /// Carry favorite/pin/icon state across a move/rename of `oldPath`. Descendant
    /// paths of a moved folder are not repathed (rare; refreshed on next listing).
    private func repath(from oldPath: String, to newPath: String) {
        guard oldPath != newPath else { return }
        if let idx = appState.favorites
            .firstIndex(of: oldPath) { appState.favorites[idx] = newPath }
        if let idx = appState.pinned.firstIndex(of: oldPath) { appState.pinned[idx] = newPath }
        if let icon = appState.icons.removeValue(forKey: oldPath) { appState.icons[newPath] = icon }
        saveAppState()
    }

    private func register(bookmark: Data, persist: Bool) throws {
        let resolved = try SecurityScopedBookmarks.resolve(bookmark)
        let effective = resolved.refreshedData ?? bookmark
        let started = resolved.url.startAccessingSecurityScopedResource()
        do {
            let location = try workspace.addLocation(
                folderPath: resolved.url.path(percentEncoded: false),
                bookmark: effective
            )
            bookmarks[location.id] = effective
            if started { heldScopes[location.id] = resolved.url }
            roots.append(WorkspaceNode(
                url: resolved.url,
                name: location.displayName,
                kind: .location(id: location.id)
            ))
            if let watcher = try? startWatching(
                root: resolved.url.path(percentEncoded: false),
                observer: fsObserver
            ) {
                watchers[location.id] = watcher
            }
            if persist { persistBookmarks() }
            revision += 1
        } catch {
            if started { resolved.url.stopAccessingSecurityScopedResource() }
            throw error
        }
    }

    private func restorePersistedLocations() {
        let stored = (defaults.array(forKey: bookmarksKey) as? [Data]) ?? []
        for data in stored {
            try? register(bookmark: data, persist: false)
        }
        persistBookmarks() // capture any stale-bookmark refreshes
        replayAppStateToCore()
    }

    private func persistBookmarks() {
        defaults.set(Array(bookmarks.values), forKey: bookmarksKey)
    }

    private func loadAppState() {
        guard let data = defaults.data(forKey: appStateKey),
              let decoded = try? JSONDecoder().decode(AppState.self, from: data) else { return }
        appState = decoded
    }

    private func saveAppState() {
        guard let data = try? JSONEncoder().encode(appState) else { return }
        defaults.set(data, forKey: appStateKey)
    }

    /// Re-apply persisted app state to the freshly-created core handle (the core
    /// starts empty each launch).
    private func replayAppStateToCore() {
        for (path, icon) in appState.icons {
            try? workspace.setFolderIcon(folderPath: path, icon: icon)
        }
        for path in appState.favorites {
            try? workspace.setFavorite(path: path, favorite: true)
        }
        for path in appState.pinned {
            try? workspace.setPinned(path: path, pinned: true)
        }
    }

    private func refreshFavorites() {
        favoritesGroup.children = favoriteNodes()
    }

    /// Build nodes for the favorited paths, skipping any that no longer exist.
    private func favoriteNodes() -> [WorkspaceNode] {
        appState.favorites.compactMap { path in
            var isDir: ObjCBool = false
            guard FileManager.default.fileExists(atPath: path, isDirectory: &isDir)
            else { return nil }
            let url = URL(fileURLWithPath: path)
            return WorkspaceNode(
                url: url,
                name: url.lastPathComponent,
                kind: isDir.boolValue ? .folder : .file
            )
        }
    }

    private func listChildren(of node: WorkspaceNode) -> [WorkspaceNode] {
        guard node.isExpandable else { return [] }
        let listed = (try? workspace.listChildren(folderPath: node.path)) ?? []
        return listed.map { fsNode in
            WorkspaceNode(
                url: URL(fileURLWithPath: fsNode.path),
                name: fsNode.name,
                kind: fsNode.kind == .folder ? .folder : .file
            )
        }
    }
}
