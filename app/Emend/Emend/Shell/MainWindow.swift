import AppKit
import Combine
import EmendCore
import SwiftUI
import UniformTypeIdentifiers

/// The single-window three-pane shell: locations sidebar | tabbed editor | info.
///
/// The sidebar browses the file-based workspace (US2); opening a file adds a tab.
/// Each tab keeps its own live `MarkdownEditorView` alive (US1) so switching tabs
/// preserves per-document edit state. The info pane (US6) is still a placeholder.
struct MainWindow: View {
    @StateObject private var workspace = WorkspaceModel()
    @StateObject private var tabs = TabModel()

    var body: some View {
        NavigationSplitView {
            sidebar
        } content: {
            editorPane
        } detail: {
            infoPane
        }
        .navigationTitle(tabs.active?.name ?? "Emend")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Open File", systemImage: "doc.badge.plus", action: openFile)
            }
            ToolbarItem(placement: .secondaryAction) {
                Button("Add Location", systemImage: "folder.badge.plus") { workspace.addLocation() }
            }
        }
        // Durability (FR-009/FR-009a): flush all open tabs before the app quits or
        // the window closes, since autosave is otherwise only debounced.
        .onReceive(willTerminatePublisher) { _ in tabs.flushAll() }
        .onReceive(willClosePublisher) { _ in tabs.flushAll() }
    }

    private var willTerminatePublisher: NotificationCenter.Publisher {
        NotificationCenter.default.publisher(for: NSApplication.willTerminateNotification)
    }

    private var willClosePublisher: NotificationCenter.Publisher {
        NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)
    }

    private var sidebar: some View {
        WorkspaceOutlineView(model: workspace) { url in tabs.open(url: url) }
            .navigationSplitViewColumnWidth(min: 200, ideal: 240)
            .overlay {
                if workspace.displayRoots.isEmpty {
                    ContentUnavailableView(
                        "No Locations",
                        systemImage: "folder.badge.plus",
                        description: Text("Add a folder, or open a file to start editing.")
                    )
                }
            }
    }

    private var editorPane: some View {
        VStack(spacing: 0) {
            TabBarView(model: tabs)
            editorStack
        }
        .frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
    }

    @ViewBuilder private var editorStack: some View {
        if tabs.tabs.isEmpty {
            VStack(spacing: 8) {
                Image(systemName: "doc.text")
                    .font(.system(size: 40))
                    .foregroundStyle(.tertiary)
                Text("Open a Markdown file to start editing.")
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            // All open editors stay alive; only the active one is visible and
            // interactive, so switching tabs preserves each buffer.
            ZStack {
                ForEach(tabs.tabs) { tab in
                    MarkdownEditorView(
                        handle: tab.handle,
                        initialText: tab.text,
                        isReadOnly: tab.isReadOnly,
                        autosave: tab.autosave
                    )
                    .id(tab.id)
                    .opacity(tab.id == tabs.activeID ? 1 : 0)
                    .allowsHitTesting(tab.id == tabs.activeID)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var infoPane: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Info")
                .font(.headline)
            if let status = tabs.status {
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
        tabs.open(url: url)
    }
}

#Preview {
    MainWindow()
}
