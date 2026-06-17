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
        let id = UUID()
        let url: URL
        let name: String
        let handle: OpenDocHandle
        /// Text at open time — the editor's initial seed (the core then owns it).
        let text: String
        let isReadOnly: Bool
        let autosave: AutosaveController

        static func == (lhs: Tab, rhs: Tab) -> Bool {
            lhs.id == rhs.id
        }
    }

    @Published private(set) var tabs: [Tab] = []
    @Published var activeID: Tab.ID?
    /// Last status/error surfaced to the info pane (Constitution III).
    @Published var status: String?

    var active: Tab? {
        tabs.first { $0.id == activeID }
    }

    /// Open `url` in a tab, focusing an existing tab if the file is already open.
    func open(url: URL) {
        let path = url.path(percentEncoded: false)
        if let existing = tabs.first(where: { $0.url.path(percentEncoded: false) == path }) {
            activeID = existing.id
            return
        }
        let handle: OpenDocHandle
        do {
            handle = try openDocument(path: path)
        } catch let error as FfiError {
            // FR-027a: oversized files are rejected by the core; surface the notice.
            status = error.userMessage
            return
        } catch {
            status = error.localizedDescription
            return
        }
        let text = (try? readFileAt(path: path)) ?? ""
        let autosave = AutosaveController(handle: handle)
        autosave.onError = { [weak self] error in
            let message = error.userMessage
            Task { @MainActor in self?.status = message }
        }
        let tab = Tab(
            url: url,
            name: url.lastPathComponent,
            handle: handle,
            text: text,
            isReadOnly: false,
            autosave: autosave
        )
        tabs.append(tab)
        activeID = tab.id
        status = "Editing “\(tab.name)”."
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
}
