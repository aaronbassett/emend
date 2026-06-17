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
    @StateObject private var conflict = ConflictController()
    @StateObject private var quickOpen = QuickOpenModel()

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
        // ⌘P opens Quick Open (US3, FR-017). A hidden button registers the
        // shortcut window-wide without occupying the toolbar.
        .background {
            Button("Quick Open", action: quickOpen.present)
                .keyboardShortcut("p", modifiers: .command)
                .hidden()
        }
        .overlay { quickOpenOverlay }
        // Durability (FR-009/FR-009a): flush all open tabs before the app quits or
        // the window closes, since autosave is otherwise only debounced.
        .onReceive(willTerminatePublisher) { _ in tabs.flushAll() }
        .onReceive(willClosePublisher) { _ in tabs.flushAll() }
        .task {
            conflict.attach(tabs: tabs, workspace: workspace)
            quickOpen.attach(workspace: workspace.workspace) { url in tabs.open(url: url) }
        }
    }

    @ViewBuilder private var quickOpenOverlay: some View {
        if quickOpen.isPresented {
            ZStack(alignment: .top) {
                Rectangle()
                    .fill(.black.opacity(0.08))
                    .ignoresSafeArea()
                    .onTapGesture { quickOpen.dismiss() }
                QuickOpenView(model: quickOpen)
                    .padding(.top, 90)
            }
        }
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
            if let activeID = tabs.activeID, conflict.isConflicted(activeID) {
                conflictBanner(activeID)
            }
            editorStack
        }
        .frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
    }

    private func conflictBanner(_ id: UUID) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
            Text("This file changed on disk.")
                .font(.callout)
            Spacer()
            Button("Reload") { conflict.resolve(id, choice: .reloadFromDisk) }
            Button("Keep Mine") { conflict.resolve(id, choice: .keepMine) }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(Color.orange.opacity(0.12))
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
                    .id("\(tab.id)-\(tab.reloadToken)")
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
