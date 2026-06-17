import AppKit

/// Pasteboard type carrying a dragged workspace node's path (internal reorganize
/// only — not a public file promise).
let workspaceNodePasteboardType = NSPasteboard
    .PasteboardType("com.aaronbassett.Emend.workspaceNode")

/// Drag-drop reorganize for the workspace outline (T063, FR-004/FR-005): drag a
/// file or folder and drop it onto another folder/location to move it. Dropping
/// is "on" the target folder (not between rows); the move runs through the Rust
/// core's collision-safe `moveNode`.
extension Coordinator {
    func outlineView(_: NSOutlineView, pasteboardWriterForItem item: Any) -> NSPasteboardWriting? {
        guard let node = item as? WorkspaceNode,
              node.kind == .file || node.kind == .folder else { return nil }
        let pbItem = NSPasteboardItem()
        pbItem.setString(node.path, forType: workspaceNodePasteboardType)
        return pbItem
    }

    func outlineView(
        _ outlineView: NSOutlineView,
        validateDrop info: NSDraggingInfo,
        proposedItem item: Any?,
        proposedChildIndex _: Int
    ) -> NSDragOperation {
        guard let target = item as? WorkspaceNode, target.isExpandable, target.kind != .favorites,
              let sourcePath = info.draggingPasteboard.string(forType: workspaceNodePasteboardType)
        else { return [] }
        // Reject no-op (same parent), self, and folder-into-own-descendant drops.
        let sourceParent = URL(fileURLWithPath: sourcePath).deletingLastPathComponent().path
        guard sourceParent != target.path,
              target.path != sourcePath,
              !target.path.hasPrefix(sourcePath + "/") else { return [] }
        // Always retarget to drop ON the folder, never between its rows.
        outlineView.setDropItem(target, dropChildIndex: NSOutlineViewDropOnItemIndex)
        return .move
    }

    func outlineView(
        _: NSOutlineView,
        acceptDrop info: NSDraggingInfo,
        item: Any?,
        childIndex _: Int
    ) -> Bool {
        guard let target = item as? WorkspaceNode,
              let sourcePath = info.draggingPasteboard.string(forType: workspaceNodePasteboardType)
        else { return false }
        return model.move(sourcePath: sourcePath, into: target)
    }
}
