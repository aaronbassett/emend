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
    @StateObject private var preview = PreviewModel()
    @StateObject private var scrollSync = ScrollSync()
    @StateObject private var info = InfoModel()
    @StateObject private var aiConfig = AIConfigStore()
    @StateObject private var summary = SummaryModel()
    @State private var showPreview = false
    @State private var showSummary = false
    @State private var showAISettings = false

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
            ToolbarItem(placement: .primaryAction) {
                Button {
                    showPreview.toggle()
                } label: {
                    Label("Toggle Preview", systemImage: "rectangle.split.2x1")
                }
                .help("Show or hide the live preview")
                .disabled(tabs.active == nil)
            }
            ToolbarItem(placement: .secondaryAction) {
                Button("Export PDF", systemImage: "arrow.down.doc", action: exportPDF)
                    .help("Export the current document to a paginated PDF")
                    .disabled(tabs.active == nil)
            }
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button("Summarize Document", action: startSummary)
                        .disabled(tabs.active == nil || !aiConfig.isConfigured)
                    Button("AI Settings…") { showAISettings = true }
                } label: {
                    Label("AI", systemImage: "sparkles")
                }
                .help("BYOM AI summary")
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
        .sheet(isPresented: $showAISettings) { AISettingsView(store: aiConfig) }
        .sheet(isPresented: $showSummary, onDismiss: summary.cancel) {
            SummaryView(
                model: summary,
                canSummarize: tabs.active != nil && aiConfig.isConfigured,
                onSummarize: startSummary
            )
        }
        // Durability (FR-009/FR-009a): flush all open tabs before the app quits or
        // the window closes, since autosave is otherwise only debounced.
        .onReceive(willTerminatePublisher) { _ in tabs.flushAll() }
        .onReceive(willClosePublisher) { _ in tabs.flushAll() }
        .task {
            conflict.attach(tabs: tabs, workspace: workspace)
            quickOpen.attach(workspace: workspace.workspace) { url in tabs.open(url: url) }
            tabs.onDocEdit = { [weak preview, weak info] in
                preview?.scheduleRefresh()
                info?.refresh()
            }
            preview.workspace = workspace.workspace
            preview.isVisible = showPreview
            preview.setActiveDocument(tabs.active?.handle)
            info.setActiveDocument(tabs.active?.handle)
        }
        .onChange(of: tabs.activeID) { _, _ in
            preview.setActiveDocument(tabs.active?.handle)
            info.setActiveDocument(tabs.active?.handle)
        }
        .onChange(of: showPreview) { _, visible in preview.isVisible = visible }
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
            if showPreview, tabs.active != nil {
                HSplitView {
                    editorStack
                        .frame(minWidth: 280)
                    PreviewWebView(model: preview, scrollSync: scrollSync)
                        .frame(minWidth: 240)
                }
            } else {
                editorStack
            }
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
                        autosave: tab.autosave,
                        scrollSync: scrollSync,
                        isActive: tab.id == tabs.activeID,
                        workspace: workspace.workspace,
                        notePath: tab.url.path(percentEncoded: false),
                        onOpenLink: { url in tabs.open(url: url) }
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
        VStack(spacing: 0) {
            InfoSidebarView(model: info) { line in scrollSync.scrollToLine(line) }
            if let status = tabs.status {
                Divider()
                Text(status)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
            }
        }
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

    /// Present the summary sheet and start a streamed BYOM summary of the active
    /// document (US6). The key is read from the Keychain at invocation only.
    private func startSummary() {
        guard let handle = tabs.active?.handle, aiConfig.isConfigured,
              let key = aiConfig.apiKey() else { return }
        showSummary = true
        summary.summarize(document: handle, config: aiConfig.requestConfig(), apiKey: key)
    }

    /// Export the active document to a paginated PDF (US4 · FR-026). Renders the
    /// preview HTML off-main, then writes via the off-screen print host.
    private func exportPDF() {
        guard let tab = tabs.active else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.pdf]
        panel.nameFieldStringValue = (tab.name as NSString).deletingPathExtension + ".pdf"
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return }

        let handle = tab.handle
        let css = preview.themeCSS
        Task {
            do {
                let html = try await Task.detached { try handle.renderPreviewHtml() }.value
                try await PDFExport.export(html: html, css: css, to: url)
                tabs.status = "Exported PDF to “\(url.lastPathComponent)”."
            } catch {
                tabs.status = (error as? LocalizedError)?.errorDescription ?? error
                    .localizedDescription
            }
        }
    }
}

#Preview {
    MainWindow()
}
