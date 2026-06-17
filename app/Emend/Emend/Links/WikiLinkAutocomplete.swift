import AppKit
import EmendCore

/// Pure, headless-testable helpers for `[[wiki link]]` detection in the editor
/// buffer (US5 · FR-018/FR-019). The `MarkdownTextView` uses these to drive native
/// NSTextView completion and ⌘-click navigation; the actual index resolution and
/// tab-opening live on the `EditorCoordinator` (it holds the `WorkspaceHandle`).
enum WikiLink {
    private static let openBracket = UInt16(UInt8(ascii: "["))
    private static let closeBracket = UInt16(UInt8(ascii: "]"))
    private static let newline = UInt16(0x000A)

    /// True if `[char char]` occupies indices `(i-1, i)` — e.g. a `[[` whose second
    /// bracket is at `i`.
    private static func doubledEndingAt(_ text: NSString, _ index: Int, _ char: UInt16) -> Bool {
        index >= 1 && index < text.length
            && text.character(at: index) == char && text.character(at: index - 1) == char
    }

    /// True if `[char char]` occupies indices `(i, i+1)` — e.g. a `]]` starting at `i`.
    private static func doubledStartingAt(_ text: NSString, _ index: Int, _ char: UInt16) -> Bool {
        index >= 0 && index + 1 < text.length
            && text.character(at: index) == char && text.character(at: index + 1) == char
    }

    /// If `caret` sits inside an unclosed `[[…` on its line, the UTF-16 range of the
    /// partial target text typed after `[[` (up to the caret) — the range native
    /// completion replaces. Returns `nil` when the caret isn't in an open wiki link.
    static func partialRange(in text: NSString, caret: Int) -> NSRange? {
        guard caret >= 2, caret <= text.length else { return nil }
        var index = caret - 1
        while index >= 1 {
            let char = text.character(at: index)
            if char == newline || char == closeBracket { return nil }
            if doubledEndingAt(text, index, openBracket) {
                let start = index + 1
                return NSRange(location: start, length: caret - start)
            }
            index -= 1
        }
        return nil
    }

    /// The `[[target]]` enclosing `offset`, if any: the inner raw target (trimmed)
    /// and the full span including brackets — for click-to-navigate.
    static func enclosingLink(in text: NSString, at offset: Int) -> (raw: String, span: NSRange)? {
        guard offset >= 0, offset <= text.length, text.length >= 4 else { return nil }
        let line = text.lineRange(for: NSRange(location: min(offset, text.length - 1), length: 0))
        var open = -1
        var scan = min(offset, NSMaxRange(line) - 1)
        while scan >= line.location + 1 {
            if doubledEndingAt(text, scan, openBracket) {
                open = scan - 1
                break
            }
            scan -= 1
        }
        guard open >= 0 else { return nil }
        var close = -1
        var fwd = open + 2
        while fwd < NSMaxRange(line) - 1 {
            if text.character(at: fwd) == newline { break }
            if doubledStartingAt(text, fwd, closeBracket) {
                close = fwd
                break
            }
            fwd += 1
        }
        guard close >= open + 2, offset <= close + 2 else { return nil }
        let innerRange = NSRange(location: open + 2, length: close - (open + 2))
        let raw = text.substring(with: innerRange).trimmingCharacters(in: .whitespaces)
        guard !raw.isEmpty else { return nil }
        return (raw, NSRange(location: open, length: close + 2 - open))
    }

    /// All `[[target]]`/`![[target]]` spans in `text` (for unresolved-link styling):
    /// each entry is the inner raw target and the bracketed span.
    static func allLinks(in text: NSString) -> [(raw: String, span: NSRange)] {
        var results: [(String, NSRange)] = []
        var index = 0
        while index < text.length - 3 {
            guard doubledStartingAt(text, index, openBracket),
                  let link = enclosingLink(in: text, at: index + 2)
            else {
                index += 1
                continue
            }
            results.append((link.raw, link.span))
            index = NSMaxRange(link.span)
        }
        return results
    }
}
