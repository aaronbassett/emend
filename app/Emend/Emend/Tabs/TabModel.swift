import Combine
import EmendCore
import Foundation

/// Owns the open documents shown as tabs (research §C7). Each tab carries its own
/// Rust `OpenDocHandle` + `AutosaveController`; all stay alive while open so
/// switching tabs preserves each editor's buffer and undo history. The Rust core
/// is the source of truth for each document's text.
@MainActor
final class TabModel: ObservableObject {
    struct Tab: Identifiable, Equatable {
        let id: UUID
        let url: URL
        let name: String
        let handle: OpenDocHandle
        /// Text at open (or reload) time — the editor's initial seed.
        let text: String
        let isReadOnly: Bool
        let autosave: AutosaveController
        /// Bumped on reload-from-disk so the editor view recreates with fresh text.
        let reloadToken: Int

        init(
            id: UUID = UUID(),
            url: URL,
            name: String,
            handle: OpenDocHandle,
            text: String,
            isReadOnly: Bool,
            autosave: AutosaveController,
            reloadToken: Int = 0
        ) {
            self.id = id
            self.url = url
            self.name = name
            self.handle = handle
            self.text = text
            self.isReadOnly = isReadOnly
            self.autosave = autosave
            self.reloadToken = reloadToken
        }

        static func == (lhs: Tab, rhs: Tab) -> Bool {
            lhs.id == rhs.id && lhs.reloadToken == rhs.reloadToken
        }
    }

    @Published private(set) var tabs: [Tab] = []
    @Published var activeID: Tab.ID?
    /// Last status/error surfaced to the info pane (Constitution III).
    @Published var status: String?
    /// Invoked after a tab's autosave flushes, so external watchers can suppress
    /// the app's own write (FR-006a).
    var onTabFlushed: ((URL) -> Void)?

    var active: Tab? {
        tabs.first { $0.id == activeID }
    }

    func tab(forPath path: String) -> Tab? {
        tabs.first { $0.url.path(percentEncoded: false) == path }
    }

    /// Open `url` in a tab, focusing an existing tab if the file is already open.
    func open(url: URL) {
        let path = url.path(percentEncoded: false)
        if let existing = tab(forPath: path) {
            activeID = existing.id
            return
        }
        guard let handle = openHandle(path: path) else { return }
        let text = (try? readFileAt(path: path)) ?? ""
        let tab = Tab(
            url: url,
            name: url.lastPathComponent,
            handle: handle,
            text: text,
            isReadOnly: false,
            autosave: makeAutosave(handle, url: url)
        )
        tabs.append(tab)
        activeID = tab.id
        status = "Editing “\(tab.name)”."
    }

    /// Reload a tab from disk, discarding local edits (the "Reload" conflict
    /// choice, FR-006c). Recreates the handle/editor with the on-disk text.
    func reload(_ id: Tab.ID) {
        guard let index = tabs.firstIndex(where: { $0.id == id }) else { return }
        let old = tabs[index]
        old.autosave.discard() // synchronously drop the local buffer + pending flush
        try? old.handle.close()
        let path = old.url.path(percentEncoded: false)
        guard let handle = openHandle(path: path) else { return }
        let text = (try? readFileAt(path: path)) ?? ""
        tabs[index] = Tab(
            id: old.id,
            url: old.url,
            name: old.name,
            handle: handle,
            text: text,
            isReadOnly: old.isReadOnly,
            autosave: makeAutosave(handle, url: old.url),
            reloadToken: old.reloadToken + 1
        )
    }

    /// Close a tab: flush its pending edits, close the core handle, and pick a
    /// neighbouring tab as active.
    func close(_ id: Tab.ID) {
        guard let index = tabs.firstIndex(where: { $0.id == id }) else { return }
        let tab = tabs[index]
        tab.autosave.flushNow()
        try? tab.handle.close()
        tabs.remove(at: index)
        if activeID == id {
            activeID = tabs.indices.contains(index) ? tabs[index].id : tabs.last?.id
        }
    }

    /// Flush every open tab (app quit / window close — FR-009).
    func flushAll() {
        for tab in tabs {
            tab.autosave.flushNow()
        }
    }

    /// Flush one tab now (e.g. resolving a conflict with "Keep Mine" — write the
    /// kept buffer to disk immediately rather than waiting for the next edit).
    func flush(_ id: Tab.ID) {
        tabs.first { $0.id == id }?.autosave.flushNow()
    }

    /// If `path` is open, flush + close its tab. Called before the workspace moves
    /// a file so the open buffer is saved to the old path (which then moves) and
    /// no stale handle keeps autosaving to the vanished path.
    func closeIfOpen(path: String) {
        if let open = tab(forPath: path) { close(open.id) }
    }

    // MARK: - Private

    private func openHandle(path: String) -> OpenDocHandle? {
        do {
            return try openDocument(path: path)
        } catch let error as FfiError {
            // FR-027a: oversized files are rejected by the core; surface the notice.
            status = error.userMessage
            return nil
        } catch {
            status = error.localizedDescription
            return nil
        }
    }

    private func makeAutosave(_ handle: OpenDocHandle, url: URL) -> AutosaveController {
        let autosave = AutosaveController(handle: handle)
        autosave.onError = { [weak self] error in
            let message = error.userMessage
            Task { @MainActor in self?.status = message }
        }
        autosave.onFlush = { [weak self] in
            Task { @MainActor in self?.onTabFlushed?(url) }
        }
        return autosave
    }
}
