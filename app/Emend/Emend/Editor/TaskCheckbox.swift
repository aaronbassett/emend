import AppKit
import Foundation

/// Pure, headless-testable detection of Markdown task checkboxes (US5 · FR-014).
/// The `MarkdownTextView` uses this to make the `[ ]`/`[x]` glyph clickable; the
/// actual toggle goes through the core's `toggleTask` so it registers undo and
/// re-attributes like typing.
enum TaskCheckbox {
    /// `^\s*[-*+]\s+\[[ xX]\]` — an (optionally indented) bullet task item. The
    /// captured group is the `[ ]`/`[x]` checkbox itself.
    private static let pattern = try? NSRegularExpression(
        pattern: "^[ \\t]*[-*+][ \\t]+(\\[[ xX]\\])"
    )

    /// The UTF-16 range of the checkbox on the line containing `offset`, if that
    /// line is a task item — else `nil`. Coordinates are absolute (whole-buffer).
    static func checkboxRange(in text: NSString, atLineContaining offset: Int) -> NSRange? {
        guard let pattern, text.length > 0, offset >= 0, offset <= text.length else { return nil }
        let line = text.lineRange(for: NSRange(location: min(offset, text.length - 1), length: 0))
        let lineString = text.substring(with: line) as NSString
        guard let match = pattern.firstMatch(
            in: lineString as String,
            range: NSRange(location: 0, length: lineString.length)
        ), match.numberOfRanges >= 2 else { return nil }
        let box = match.range(at: 1)
        guard box.location != NSNotFound else { return nil }
        return NSRange(location: line.location + box.location, length: box.length)
    }

    /// The edit that flips the checkbox on the line containing `offset` (`[ ]`↔`[x]`),
    /// or `nil` if that line isn't a task item. Applied through the editor's normal
    /// `Edit` path so Swift stays the buffer owner and the core sees the delta —
    /// the core's `toggleTask` FFI is for non-editor surfaces (preview / info pane).
    static func toggleEdit(
        in text: NSString,
        atLineContaining offset: Int
    ) -> (range: NSRange, replacement: String)? {
        guard let box = checkboxRange(in: text, atLineContaining: offset),
              box.length == 3 else { return nil }
        let mark = NSRange(location: box.location + 1, length: 1)
        let checked = text.substring(with: mark) != " "
        return (mark, checked ? " " : "x")
    }
}
