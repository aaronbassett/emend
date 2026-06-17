import AppKit
import Combine
import EmendCore
import SwiftUI
import UniformTypeIdentifiers

private extension Notification.Name {
    /// Posted by `AutosaveController.onError` so the window can surface a failed
    /// flush in the status area (Constitution III — never lose data silently).
    static let emendAutosaveFailed = Notification.Name("com.aaronbassett.Emend.autosaveFailed")
}

/// The single-window three-pane shell: locations sidebar | editor | info.
///
/// US1 wires the editor pane to a live `MarkdownEditorView` over an open file.
/// The location tree (US2) and the info sidebar (US6) replace those placeholders
/// later. "Add Location" still exercises the security-scoped-bookmark ↔ Rust
/// handshake (research §A4).
struct MainWindow: View {
    private struct OpenDocument: Identifiable {
        let id = UUID()
        let name: String
        let handle: OpenDocHandle
        let text: String
        let isReadOnly: Bool
        let autosave: AutosaveController
    }

    @StateObject private var workspace = WorkspaceModel()
    @State private var openDoc: OpenDocument?
    @State private var status: String?

    var body: some View {
        NavigationSplitView {
            sidebar
        } content: {
            editorPane
        } detail: {
            infoPane
        }
        .navigationTitle(openDoc?.name ?? "Emend")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Open File", systemImage: "doc.badge.plus", action: openFile)
            }
            ToolbarItem(placement: .secondaryAction) {
                Button("Add Location", systemImage: "folder.badge.plus") { workspace.addLocation() }
            }
        }
        // Durability (FR-009/FR-009a): flush pending edits before the app quits or
        // the window closes, since autosave is otherwise only debounced.
        .onReceive(willTerminatePublisher) { _ in openDoc?.autosave.flushNow() }
        .onReceive(willClosePublisher) { _ in openDoc?.autosave.flushNow() }
        .onReceive(autosaveFailedPublisher) { note in
            if let message = note.userInfo?["message"] as? String { status = message }
        }
    }

    private var willTerminatePublisher: NotificationCenter.Publisher {
        NotificationCenter.default.publisher(for: NSApplication.willTerminateNotification)
    }

    private var willClosePublisher: NotificationCenter.Publisher {
        NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)
    }

    private var autosaveFailedPublisher: NotificationCenter.Publisher {
        NotificationCenter.default.publisher(for: .emendAutosaveFailed)
    }

    private var sidebar: some View {
        WorkspaceOutlineView(model: workspace) { url in open(url: url) }
            .navigationSplitViewColumnWidth(min: 200, ideal: 240)
            .overlay {
                if workspace.roots.isEmpty {
                    ContentUnavailableView(
                        "No Locations",
                        systemImage: "folder.badge.plus",
                        description: Text("Add a folder, or open a file to start editing.")
                    )
                }
            }
    }

    @ViewBuilder private var editorPane: some View {
        if let openDoc {
            MarkdownEditorView(
                handle: openDoc.handle,
                initialText: openDoc.text,
                isReadOnly: openDoc.isReadOnly,
                autosave: openDoc.autosave
            )
            .id(openDoc.id)
            .frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
        } else {
            VStack(spacing: 8) {
                Image(systemName: "doc.text")
                    .font(.system(size: 40))
                    .foregroundStyle(.tertiary)
                Text("Open a Markdown file to start editing.")
                    .foregroundStyle(.secondary)
            }
            .frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var infoPane: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Info")
                .font(.headline)
            if let status {
                Text(status)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            } else {
                Text("Document insight appears here.")
                    .font(.callout)
                    .foregroundStyle(.tertiary)
            }
            Spacer()
        }
        .padding()
        .frame(minWidth: 220)
    }

    private func openFile() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "md"),
            UTType(filenameExtension: "markdown"),
            .plainText,
            .text
        ].compactMap(\.self)
        guard panel.runModal() == .OK, let url = panel.url else { return }
        open(url: url)
    }

    private func open(url: URL) {
        let path = url.path(percentEncoded: false)
        let handle: OpenDocHandle
        do {
            handle = try openDocument(path: path)
        } catch let error as FfiError {
            // FR-027a: oversized files are rejected by the core; surface the notice.
            // (Read-only viewing of oversized files is a later refinement.)
            status = error.userMessage
            return
        } catch {
            status = error.localizedDescription
            return
        }

        let text = (try? readFileAt(path: path)) ?? ""
        if let previous = openDoc {
            previous.autosave.flushNow()
            try? previous.handle.close()
        }
        let autosave = AutosaveController(handle: handle)
        // Surface a failed flush instead of dropping it (Constitution III). The
        // callback runs on the autosave queue, so hop to the main actor.
        autosave.onError = { error in
            let message = error.userMessage
            Task { @MainActor in
                NotificationCenter.default.post(
                    name: .emendAutosaveFailed,
                    object: nil,
                    userInfo: ["message": message]
                )
            }
        }
        openDoc = OpenDocument(
            name: url.lastPathComponent,
            handle: handle,
            text: text,
            isReadOnly: false,
            autosave: autosave
        )
        status = "Editing “\(url.lastPathComponent)”."
    }
}

#Preview {
    MainWindow()
}
