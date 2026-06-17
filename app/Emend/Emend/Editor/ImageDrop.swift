import AppKit

/// Pure, headless-testable helpers for inline image drag-drop (US5 · FR-013/FR-013a).
/// `MarkdownTextView` accepts dropped image files, the `EditorCoordinator` stores
/// each as a collision-safe attachment beside the note (core `storeAttachment`),
/// and the returned note-relative ref is inserted as a Markdown image through the
/// usual Edit path so Swift stays the buffer owner.
enum ImageDrop {
    /// Extensions treated as droppable images.
    static let imageExtensions: Set<String> = [
        "png", "jpg", "jpeg", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp", "svg"
    ]

    /// Image file URLs on a drag pasteboard (filtered by extension).
    static func imageFileURLs(in pasteboard: NSPasteboard) -> [URL] {
        let options: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
        let objects = pasteboard.readObjects(forClasses: [NSURL.self], options: options)
        let urls = (objects as? [URL]) ?? []
        return urls.filter { imageExtensions.contains($0.pathExtension.lowercased()) }
    }

    /// The Markdown to insert for a stored attachment whose note-relative ref is
    /// `ref` (a standard image so it round-trips with other Markdown tools; embeds
    /// `![[…]]` are reserved for note transclusion).
    static func markdown(forImageRef ref: String) -> String {
        "![](\(ref))"
    }
}
