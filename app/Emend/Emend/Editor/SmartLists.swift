import Foundation

/// Pure, headless-testable smart-list transforms over the editor buffer (FR-012).
///
/// The editor view applies the returned `Edit` to its `NSTextView` through the
/// normal `shouldChangeText`/`didChangeText` path, so each transform flows to the
/// Rust core as an ordinary delta (the same path as typing). Ranges are UTF-16
/// code units, which map 1:1 onto `NSRange` (research §A2). All logic is pure
/// given `(text, selection)`, so it is unit-tested without a window (Constitution
/// VII).
enum SmartLists {
    /// One indentation level. Two spaces keeps nested Markdown lists compact.
    static let indentUnit = "  "

    /// A buffer mutation: replace `range` with `replacement`, then select
    /// `selectionAfter`. Ranges are UTF-16 code units.
    struct Edit: Equatable {
        let range: NSRange
        let replacement: String
        let selectionAfter: NSRange
    }

    // MARK: - Public transforms

    /// Return-key handling: continue a list item (next bullet/number, preserving
    /// indentation) or terminate the list when the current item is empty. Returns
    /// `nil` when the caret line is not a list item (the caller does a plain
    /// newline).
    static func newline(in text: NSString, selection: NSRange) -> Edit? {
        let line = lineRange(text, at: selection.location)
        guard let item = parse(line: lineContent(text, line)) else { return nil }

        if item.contentEmpty {
            // Empty item: Return removes the marker and exits the list.
            return Edit(
                range: NSRange(location: line.location, length: item.prefixLength),
                replacement: "",
                selectionAfter: NSRange(location: line.location, length: 0)
            )
        }

        let insertion = "\n" + item.continuationPrefix()
        let caretAfter = NSMaxRange(selection) + (insertion as NSString).length
        return Edit(
            range: selection,
            replacement: insertion,
            selectionAfter: NSRange(location: caretAfter, length: 0)
        )
    }

    /// Renumber the contiguous ordered-list block at the caret so its numbers run
    /// sequentially from the first item's number. Returns `nil` when the caret is
    /// not in an ordered list or no change is needed.
    static func renumber(in text: NSString, selection: NSRange) -> Edit? {
        let caretLine = lineRange(text, at: selection.location)
        guard let caretItem = parse(line: lineContent(text, caretLine)),
              case .ordered = caretItem.kind else { return nil }

        let lines = orderedBlock(text, around: caretLine, indent: caretItem.indent)
        guard let first = parse(line: lineContent(text, lines[0])),
              case let .ordered(start, _) = first.kind else { return nil }

        var rebuilt = ""
        var caretDelta = 0
        for (offset, line) in lines.enumerated() {
            guard let item = parse(line: lineContent(text, line)) else { return nil }
            let newPrefix = item.renumberedPrefix(to: start + offset)
            let suffix = (text.substring(with: line) as NSString).substring(from: item.prefixLength)
            rebuilt += newPrefix + suffix
            if selection.location >= line.location + item.prefixLength {
                caretDelta += (newPrefix as NSString).length - item.prefixLength
            }
        }

        let blockRange = NSRange(
            location: lines[0].location,
            length: NSMaxRange(lines[lines.count - 1]) - lines[0].location
        )
        guard rebuilt != text.substring(with: blockRange) else { return nil }
        return Edit(
            range: blockRange,
            replacement: rebuilt,
            selectionAfter: NSRange(
                location: selection.location + caretDelta,
                length: selection.length
            )
        )
    }

    /// Tab: indent the caret's list line by one `indentUnit`. Returns `nil` when
    /// the line is not a list item (the caller inserts a literal tab).
    static func indent(in text: NSString, selection: NSRange) -> Edit? {
        let line = lineRange(text, at: selection.location)
        guard parse(line: lineContent(text, line)) != nil else { return nil }
        return Edit(
            range: NSRange(location: line.location, length: 0),
            replacement: indentUnit,
            selectionAfter: NSRange(
                location: selection.location + (indentUnit as NSString).length,
                length: selection.length
            )
        )
    }

    /// Shift-Tab: outdent the caret's line by one `indentUnit` (or one leading
    /// tab/space). Returns `nil` when there is nothing to remove.
    static func outdent(in text: NSString, selection: NSRange) -> Edit? {
        let line = lineRange(text, at: selection.location)
        let content = lineContent(text, line) as NSString
        let removeLength: Int
        if content.hasPrefix(indentUnit) {
            removeLength = (indentUnit as NSString).length
        } else if content.hasPrefix("\t") || content.hasPrefix(" ") {
            removeLength = 1
        } else {
            return nil
        }
        return Edit(
            range: NSRange(location: line.location, length: removeLength),
            replacement: "",
            selectionAfter: NSRange(
                location: max(line.location, selection.location - removeLength),
                length: selection.length
            )
        )
    }

    // MARK: - Parsed list line

    /// The marker kind of a list line.
    private enum ListKind: Equatable {
        case bullet(String) // "-", "*", "+"
        case ordered(number: Int, delim: String) // delim is "." or ")"
    }

    /// The structural prefix of a single list line.
    private struct ListItem {
        let indent: String
        let kind: ListKind
        let task: String? // "[ ] " / "[x] " incl. trailing space, else nil
        let prefixLength: Int // UTF-16 length of indent + marker + (task)
        let contentEmpty: Bool

        /// Marker for the *next* item: same indent, incremented number, fresh
        /// unchecked box when this item was a task.
        func continuationPrefix() -> String {
            renumberedPrefix(to: orderedNumber + 1, freshTask: true)
        }

        /// This item's prefix with the ordered number replaced by `number`
        /// (no-op for bullets). `freshTask` resets a task box to unchecked.
        func renumberedPrefix(to number: Int, freshTask: Bool = false) -> String {
            let box = task == nil ? "" : (freshTask ? "[ ] " : (task ?? ""))
            switch kind {
            case let .bullet(char):
                return indent + char + " " + box
            case let .ordered(_, delim):
                return indent + String(number) + delim + " " + box
            }
        }

        private var orderedNumber: Int {
            if case let .ordered(number, _) = kind { return number }
            return 0
        }
    }

    // MARK: - Parsing

    private static func parse(line: String) -> ListItem? {
        let chars = Array(line)
        var index = 0
        let indent = scanWhitespace(chars, &index)
        guard let kind = scanMarker(chars, &index) else { return nil }
        let task = scanTaskBox(chars, &index)
        let prefix = String(chars[0 ..< index])
        let content = String(chars[index...]).trimmingCharacters(in: .whitespaces)
        return ListItem(
            indent: indent,
            kind: kind,
            task: task,
            prefixLength: (prefix as NSString).length,
            contentEmpty: content.isEmpty
        )
    }

    private static func scanWhitespace(_ chars: [Character], _ index: inout Int) -> String {
        var result = ""
        while index < chars.count, chars[index] == " " || chars[index] == "\t" {
            result.append(chars[index])
            index += 1
        }
        return result
    }

    private static func scanMarker(_ chars: [Character], _ index: inout Int) -> ListKind? {
        guard index < chars.count else { return nil }
        let char = chars[index]
        if char == "-" || char == "*" || char == "+" {
            guard index + 1 < chars.count, chars[index + 1] == " " else { return nil }
            index += 2
            return .bullet(String(char))
        }
        guard char.isNumber else { return nil }
        var cursor = index
        var digits = ""
        while cursor < chars.count, chars[cursor].isNumber {
            digits.append(chars[cursor])
            cursor += 1
        }
        guard cursor + 1 < chars.count, chars[cursor] == "." || chars[cursor] == ")",
              chars[cursor + 1] == " ", let number = Int(digits) else { return nil }
        let delim = String(chars[cursor])
        index = cursor + 2
        return .ordered(number: number, delim: delim)
    }

    private static func scanTaskBox(_ chars: [Character], _ index: inout Int) -> String? {
        guard index + 3 < chars.count, chars[index] == "[", chars[index + 2] == "]",
              chars[index + 3] == " ",
              chars[index + 1] == " " || chars[index + 1] == "x" || chars[index + 1] == "X"
        else { return nil }
        let box = String(chars[index ..< index + 4])
        index += 4
        return box
    }

    // MARK: - Line navigation

    private static func lineRange(_ text: NSString, at location: Int) -> NSRange {
        text.lineRange(for: NSRange(location: location, length: 0))
    }

    /// The line's text with its trailing line terminator stripped.
    private static func lineContent(_ text: NSString, _ line: NSRange) -> String {
        var contentsEnd = 0
        text.getLineStart(nil, end: nil, contentsEnd: &contentsEnd, for: line)
        return text.substring(with: NSRange(
            location: line.location,
            length: contentsEnd - line.location
        ))
    }

    /// Contiguous ordered-list lines at `indent`, in document order, including
    /// `caretLine`.
    private static func orderedBlock(
        _ text: NSString,
        around caretLine: NSRange,
        indent: String
    ) -> [NSRange] {
        var ranges = [caretLine]
        var start = caretLine.location
        while start > 0 {
            let prev = lineRange(text, at: start - 1)
            guard isOrdered(text, prev, indent: indent) else { break }
            ranges.insert(prev, at: 0)
            start = prev.location
        }
        var end = NSMaxRange(caretLine)
        while end < text.length {
            let next = lineRange(text, at: end)
            guard isOrdered(text, next, indent: indent) else { break }
            ranges.append(next)
            end = NSMaxRange(next)
        }
        return ranges
    }

    private static func isOrdered(_ text: NSString, _ line: NSRange, indent: String) -> Bool {
        guard let item = parse(line: lineContent(text, line)), case .ordered = item.kind else {
            return false
        }
        return item.indent == indent
    }
}
