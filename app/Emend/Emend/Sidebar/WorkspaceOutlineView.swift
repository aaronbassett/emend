import AppKit
import EmendCore
import SwiftUI

/// The workspace sidebar: a source-list `NSOutlineView` over the `WorkspaceModel`
/// (research §C6). Locations are roots; folders expand lazily via the Rust core.
/// Double-click opens a file (via `onOpenFile`) or toggles a folder. A
/// right-click menu sets a folder's custom icon (FR-008) or removes a location.
/// Top-level changes reload on `model.revision`; an icon change reloads just its
/// row (`reloadItem`), preserving expansion.
struct WorkspaceOutlineView: NSViewRepresentable {
    @ObservedObject var model: WorkspaceModel
    let onOpenFile: (URL) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(model: model, onOpenFile: onOpenFile)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let outline = NSOutlineView()
        outline.headerView = nil
        outline.indentationPerLevel = 14
        outline.autoresizesOutlineColumn = true
        outline.style = .sourceList
        outline.allowsEmptySelection = true

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("name"))
        column.resizingMask = .autoresizingMask
        outline.addTableColumn(column)
        outline.outlineTableColumn = column

        outline.dataSource = context.coordinator
        outline.delegate = context.coordinator
        outline.target = context.coordinator
        outline.doubleAction = #selector(Coordinator.handleDoubleClick)

        let menu = NSMenu()
        menu.delegate = context.coordinator
        outline.menu = menu

        context.coordinator.attach(outline)

        let scrollView = NSScrollView()
        scrollView.documentView = outline
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .noBorder
        return scrollView
    }

    func updateNSView(_: NSScrollView, context: Context) {
        context.coordinator.syncRootsIfChanged(revision: model.revision)
    }
}

/// Data source + delegate + context menu for the workspace outline. Main-actor
/// isolated — all `NSOutlineView` access is on the main thread.
@MainActor
final class Coordinator: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate, NSMenuDelegate {
    private let model: WorkspaceModel
    private let onOpenFile: (URL) -> Void
    private weak var outline: NSOutlineView?
    private var lastRevision = -1
    private var clickedNode: WorkspaceNode?
    private var iconPopover: NSPopover?
    private let cellID = NSUserInterfaceItemIdentifier("WorkspaceCell")

    init(model: WorkspaceModel, onOpenFile: @escaping (URL) -> Void) {
        self.model = model
        self.onOpenFile = onOpenFile
    }

    func attach(_ outline: NSOutlineView) {
        self.outline = outline
        lastRevision = model.revision
        outline.reloadData()
    }

    /// Reload the top level only when the location set actually changed, so
    /// incidental SwiftUI updates don't collapse expanded folders.
    func syncRootsIfChanged(revision: Int) {
        guard revision != lastRevision else { return }
        lastRevision = revision
        outline?.reloadData()
    }

    // MARK: - NSOutlineViewDataSource

    func outlineView(_: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        if let node = item as? WorkspaceNode { return model.children(of: node).count }
        return model.roots.count
    }

    func outlineView(_: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        if let node = item as? WorkspaceNode { return model.children(of: node)[index] }
        return model.roots[index]
    }

    func outlineView(_: NSOutlineView, isItemExpandable item: Any) -> Bool {
        (item as? WorkspaceNode)?.isExpandable ?? false
    }

    // MARK: - NSOutlineViewDelegate

    func outlineView(
        _ outlineView: NSOutlineView,
        viewFor _: NSTableColumn?,
        item: Any
    ) -> NSView? {
        guard let node = item as? WorkspaceNode else { return nil }
        let cell = (outlineView.makeView(withIdentifier: cellID, owner: self) as? NSTableCellView)
            ?? makeCell()
        cell.textField?.stringValue = node.name
        let (image, tint) = iconImage(for: node)
        cell.imageView?.image = image
        cell.imageView?.contentTintColor = tint
        return cell
    }

    @objc func handleDoubleClick() {
        guard let outline, outline.clickedRow >= 0,
              let node = outline.item(atRow: outline.clickedRow) as? WorkspaceNode else { return }
        if node.kind == .file {
            onOpenFile(node.url)
        } else if outline.isItemExpanded(node) {
            outline.collapseItem(node)
        } else {
            outline.expandItem(node)
        }
    }

    // MARK: - NSMenuDelegate (right-click context menu)

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let outline, outline.clickedRow >= 0,
              let node = outline.item(atRow: outline.clickedRow) as? WorkspaceNode
        else {
            clickedNode = nil
            return
        }
        clickedNode = node

        if node.kind != .file {
            menu.addItem(
                withTitle: "Set Icon…",
                action: #selector(setIconAction),
                keyEquivalent: ""
            )
            if model.icon(for: node.path) != nil {
                menu.addItem(
                    withTitle: "Reset Icon",
                    action: #selector(resetIconAction),
                    keyEquivalent: ""
                )
            }
        }
        if case .location = node.kind {
            menu.addItem(.separator())
            menu.addItem(
                withTitle: "Remove Location",
                action: #selector(removeLocationAction),
                keyEquivalent: ""
            )
        }
        for item in menu.items where item.action != nil {
            item.target = self
        }
    }

    @objc private func setIconAction() {
        presentIconPicker()
    }

    @objc private func resetIconAction() {
        guard let node = clickedNode else { return }
        model.setIcon(nil, for: node.path)
        outline?.reloadItem(node, reloadChildren: false)
    }

    @objc private func removeLocationAction() {
        guard let node = clickedNode else { return }
        model.removeLocation(node) // revision bump → reload via updateNSView
    }

    private func presentIconPicker() {
        guard let outline, let node = clickedNode else { return }
        let row = outline.row(forItem: node)
        guard row >= 0 else { return }
        let rect = outline.frameOfCell(atColumn: 0, row: row)
        let picker = FolderIconPicker(current: model.icon(for: node.path)) { [weak self] chosen in
            guard let self else { return }
            model.setIcon(chosen, for: node.path)
            outline.reloadItem(node, reloadChildren: false)
            iconPopover?.close()
        }
        let popover = NSPopover()
        popover.contentViewController = NSHostingController(rootView: picker)
        popover.behavior = .transient
        iconPopover = popover
        popover.show(relativeTo: rect, of: outline, preferredEdge: .maxX)
    }

    // MARK: - Cells

    private func makeCell() -> NSTableCellView {
        let cell = NSTableCellView()
        cell.identifier = cellID

        let imageView = NSImageView()
        imageView.translatesAutoresizingMaskIntoConstraints = false
        let textField = NSTextField(labelWithString: "")
        textField.translatesAutoresizingMaskIntoConstraints = false
        textField.lineBreakMode = .byTruncatingTail

        cell.addSubview(imageView)
        cell.addSubview(textField)
        cell.imageView = imageView
        cell.textField = textField

        NSLayoutConstraint.activate([
            imageView.leadingAnchor.constraint(equalTo: cell.leadingAnchor),
            imageView.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            imageView.widthAnchor.constraint(equalToConstant: 16),
            textField.leadingAnchor.constraint(equalTo: imageView.trailingAnchor, constant: 6),
            textField.trailingAnchor.constraint(equalTo: cell.trailingAnchor),
            textField.centerYAnchor.constraint(equalTo: cell.centerYAnchor)
        ])
        return cell
    }

    /// The icon image + optional tint for a node: a custom folder icon (FR-008)
    /// when set, otherwise the default per kind.
    private func iconImage(for node: WorkspaceNode) -> (NSImage?, NSColor?) {
        if node.kind != .file, let custom = model.icon(for: node.path) {
            let image = NSImage(systemSymbolName: custom.symbol, accessibilityDescription: nil)
            return (image, custom.tint?.nsColor)
        }
        let symbol = switch node.kind {
        case .location: "folder.fill"
        case .folder: "folder"
        case .file: "doc.text"
        }
        return (NSImage(systemSymbolName: symbol, accessibilityDescription: nil), nil)
    }
}
