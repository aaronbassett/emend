import Combine
import EmendCore
import Foundation

/// One row in the workspace sidebar tree. A reference type because
/// `NSOutlineView` identifies rows by object identity; children are listed
/// lazily from the Rust core on expansion (research §C6).
final class WorkspaceNode {
    enum Kind: Equatable {
        case location(id: UInt64)
        case folder
        case file
    }

    let url: URL
    let name: String
    let kind: Kind
    /// `nil` until first listed; files never gain children.
    var children: [WorkspaceNode]?

    init(url: URL, name: String, kind: Kind) {
        self.url = url
        self.name = name
        self.kind = kind
    }

    var path: String {
        url.path(percentEncoded: false)
    }

    var isExpandable: Bool {
        kind != .file
    }
}

/// Owns the Rust `WorkspaceHandle` plus the Swift-side concerns the core can't
/// hold: the security-scoped folder bookmarks (the sandbox scope Rust reads and
/// watches inside, research §A4) and their persistence across launches. The
/// handle is the source of truth for the workspace/index; this model adds the
/// scope lifecycle and the sidebar's root nodes.
@MainActor
final class WorkspaceModel: ObservableObject {
    let workspace: WorkspaceHandle

    /// Sidebar roots (the added locations). `revision` bumps only when this set
    /// changes, so the outline view reloads its top level exactly then (not on
    /// every incidental SwiftUI update — which would collapse expanded folders).
    @Published private(set) var roots: [WorkspaceNode] = []
    @Published private(set) var revision = 0

    private let defaults: UserDefaults
    private let bookmarksKey = "com.aaronbassett.Emend.locationBookmarks"
    /// Per-location: the bookmark (for persistence) and the held security scope.
    private var bookmarks: [UInt64: Data] = [:]
    private var heldScopes: [UInt64: URL] = [:]

    init(workspace: WorkspaceHandle = newWorkspace(), defaults: UserDefaults = .standard) {
        self.workspace = workspace
        self.defaults = defaults
        restorePersistedLocations()
    }

    /// Prompt for a folder and add it as a location.
    func addLocation() {
        guard let bookmark = try? SecurityScopedBookmarks.promptForFolder() else { return }
        try? register(bookmark: bookmark, persist: true)
    }

    func removeLocation(_ node: WorkspaceNode) {
        guard case let .location(id) = node.kind else { return }
        try? workspace.removeLocation(id: id)
        bookmarks.removeValue(forKey: id)
        if let url = heldScopes.removeValue(forKey: id) {
            url.stopAccessingSecurityScopedResource()
        }
        roots.removeAll { $0 === node }
        persistBookmarks()
        revision += 1
    }

    /// Lazily-listed children for `node`, cached on the node.
    func children(of node: WorkspaceNode) -> [WorkspaceNode] {
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

    // MARK: - Private

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
    }

    private func persistBookmarks() {
        defaults.set(Array(bookmarks.values), forKey: bookmarksKey)
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
