import AppKit
import EmendCore
import SwiftUI
import UniformTypeIdentifiers

/// The single-window three-pane shell: locations sidebar | editor | info.
///
/// US1 wires the editor pane to a live `MarkdownEditorView` over an open file.
/// The location tree (US2) and the info sidebar (US6) replace those placeholders
/// later. "Add Location" still exercises the security-scoped-bookmark ↔ Rust
/// handshake (research §A4).
struct MainWindow: View {
    private struct Location: Identifiable {
        let id = UUID()
        let name: String
        let bookmark: Data
    }

    private struct OpenDocument: Identifiable {
        let id = UUID()
        let name: String
        let handle: OpenDocHandle
        let text: String
        let isReadOnly: Bool
        let autosave: AutosaveController
    }

    @State private var locations: [Location] = []
    @State private var selection: Location.ID?
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
                Button("Add Location", systemImage: "folder.badge.plus", action: addLocation)
            }
        }
    }

    private var sidebar: some View {
        List(locations, selection: $selection) { location in
            Label(location.name, systemImage: "folder")
        }
        .navigationSplitViewColumnWidth(min: 200, ideal: 240)
        .overlay {
            if locations.isEmpty {
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
        openDoc = OpenDocument(
            name: url.lastPathComponent,
            handle: handle,
            text: text,
            isReadOnly: false,
            autosave: AutosaveController(handle: handle)
        )
        status = "Editing “\(url.lastPathComponent)”."
    }

    private func addLocation() {
        do {
            guard let bookmark = try SecurityScopedBookmarks.promptForFolder() else { return }
            let resolved = try SecurityScopedBookmarks.resolve(bookmark)
            locations.append(
                Location(
                    name: resolved.url.lastPathComponent,
                    bookmark: resolved.refreshedData ?? bookmark
                )
            )
            status = handshakeStatus(folder: resolved.url)
        } catch let error as FfiError {
            status = error.userMessage
        } catch {
            status = error.localizedDescription
        }
    }

    /// Demonstrate the scope↔Rust handshake by reading the first file in the
    /// granted folder through the Rust core, while Swift holds the scope.
    private func handshakeStatus(folder: URL) -> String {
        SecurityScopedBookmarks.withScope(folder) { dir in
            let manager = FileManager.default
            let entries = (try? manager.contentsOfDirectory(
                at: dir,
                includingPropertiesForKeys: [.isRegularFileKey]
            )) ?? []
            let firstFile = entries.first { url in
                (try? url.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true
            }
            guard let file = firstFile else {
                return "Added “\(dir.lastPathComponent)” — scope opened (no files to read)."
            }
            do {
                let text = try readFileAt(path: file.path(percentEncoded: false))
                return "Handshake OK: read \(text.utf16.count) UTF-16 code units from "
                    + "“\(file.lastPathComponent)” through Rust."
            } catch let error as FfiError {
                return "Scope opened, but the Rust read failed: \(error.userMessage)"
            } catch {
                return "Scope opened, but the Rust read failed: \(error.localizedDescription)"
            }
        }
    }
}

#Preview {
    MainWindow()
}
