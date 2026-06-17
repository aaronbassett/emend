import AppKit
import EmendCore
import SwiftUI

/// The workspace sidebar: a source-list `NSOutlineView` over the `WorkspaceModel`
/// (research §C6). Locations are roots; folders expand lazily via the Rust core.
/// Double-click opens a file (via `onOpenFile`) or toggles a folder. Top-level
/// changes reload on `model.revision`; folder contents use targeted
/// `reloadItem(_:reloadChildren:)`.
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

/// Data source + delegate for the workspace outline. Main-actor isolated — all
/// `NSOutlineView` access is on the main thread.
@MainActor
final class Coordinator: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    private let model: WorkspaceModel
    private let onOpenFile: (URL) -> Void
    private weak var outline: NSOutlineView?
    private var lastRevision = -1
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
        cell.imageView?.image = icon(for: node)
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

    // MARK: - Cell construction

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

    private func icon(for node: WorkspaceNode) -> NSImage? {
        let symbol = switch node.kind {
        case .location: "folder.fill"
        case .folder: "folder"
        case .file: "doc.text"
        }
        return NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
    }
}
