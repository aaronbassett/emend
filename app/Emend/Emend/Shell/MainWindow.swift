import EmendCore
import SwiftUI

/// The single-window three-pane shell: locations sidebar | editor | info.
///
/// Phase 2 skeleton — the real `NSOutlineView` location tree (US2), the TextKit 2
/// editor (US1), and the info sidebar (US6) replace these placeholders. The
/// "Add Location" action exercises the security-scoped-bookmark ↔ Rust file-IO
/// handshake end-to-end (research §A4): pick a folder, hold its scope, and read a
/// file inside it through the Rust core.
struct MainWindow: View {
    private struct Location: Identifiable {
        let id = UUID()
        let name: String
        let bookmark: Data
    }

    @State private var locations: [Location] = []
    @State private var selection: Location.ID?
    @State private var status: String?

    var body: some View {
        NavigationSplitView {
            sidebar
        } content: {
            editorPane
        } detail: {
            infoPane
        }
        .navigationTitle("Emend")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
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
                    description: Text("Add a folder to start editing your Markdown.")
                )
            }
        }
    }

    private var editorPane: some View {
        VStack(spacing: 8) {
            Image(systemName: "doc.text")
                .font(.system(size: 40))
                .foregroundStyle(.tertiary)
            Text("Editor")
                .foregroundStyle(.secondary)
        }
        .frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
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
                return "Handshake OK: read \(text.count) characters from "
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
