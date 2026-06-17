import Foundation

/// Pure, headless-testable inline-formatting transforms (FR-011): bold, italic,
/// link, task. Each is a function of `(text, selection)` so the logic is
/// unit-tested without a window (Constitution VII); the editor view applies the
/// returned `Edit` through the normal text-change path so it reaches the Rust
/// core like any other edit. Ranges are UTF-16 code units (research §A2).
enum FormattingCommands {
    /// A buffer mutation: replace `range` with `replacement`, then select
    /// `selectionAfter`. Ranges are UTF-16 code units.
    struct Edit: Equatable {
        let range: NSRange
        let replacement: String
        let selectionAfter: NSRange
    }

    /// Toggle `**bold**` around the selection (or insert an empty pair with the
    /// caret between the markers when the selection is empty).
    static func bold(in text: NSString, selection: NSRange) -> Edit {
        toggleWrap(text, selection, marker: "**")
    }

    /// Toggle `*italic*` around the selection.
    static func italic(in text: NSString, selection: NSRange) -> Edit {
        toggleWrap(text, selection, marker: "*")
    }

    /// Wrap the selection as a Markdown link `[text](url)`, selecting the `url`
    /// placeholder (or the `text` placeholder when the selection is empty).
    static func link(in text: NSString, selection: NSRange) -> Edit {
        let selected = text.substring(with: selection)
        if selected.isEmpty {
            return Edit(
                range: selection,
                replacement: "[text](url)",
                selectionAfter: NSRange(location: selection.location + 1, length: 4)
            )
        }
        let replacement = "[\(selected)](url)"
        let urlLocation = selection.location + 1 + (selected as NSString).length + 2
        return Edit(
            range: selection,
            replacement: replacement,
            selectionAfter: NSRange(location: urlLocation, length: 3)
        )
    }

    /// Turn the caret's line into a task item, or toggle an existing task's
    /// checkbox between `[ ]` and `[x]`.
    static func task(in text: NSString, selection: NSRange) -> Edit {
        let line = text.lineRange(for: NSRange(location: selection.location, length: 0))
        var contentsEnd = 0
        text.getLineStart(nil, end: nil, contentsEnd: &contentsEnd, for: line)
        let content = text.substring(with: NSRange(
            location: line.location,
            length: contentsEnd - line.location
        ))
        let chars = Array(content)

        var index = 0
        while index < chars.count, chars[index] == " " || chars[index] == "\t" {
            index += 1
        }
        let hasBullet = index + 1 < chars.count
            && (chars[index] == "-" || chars[index] == "*" || chars[index] == "+")
            && chars[index + 1] == " "

        if hasBullet, let stateIndex = taskStateIndex(chars, bulletAt: index) {
            return toggleBox(
                at: line.location + utf16Length(chars, upTo: stateIndex),
                checked: chars[stateIndex] != " ",
                selection: selection
            )
        }
        if hasBullet {
            return insert(
                "[ ] ",
                at: line.location + utf16Length(chars, upTo: index + 2),
                selection: selection
            )
        }
        return insert(
            "- [ ] ",
            at: line.location + utf16Length(chars, upTo: index),
            selection: selection
        )
    }

    // MARK: - Helpers

    private static func toggleWrap(_ text: NSString, _ selection: NSRange, marker: String) -> Edit {
        let markerLength = (marker as NSString).length
        let selected = text.substring(with: selection) as NSString

        // The selection itself spans the markers with non-empty inner content:
        // "**word**" → "word". Strict `>` (not `>=`) so a bare "**"/"*" selection
        // isn't treated as an empty wrap and silently deleted.
        let spansMarkers = selected.length > 2 * markerLength
            && selected.hasPrefix(marker) && selected.hasSuffix(marker)
        if spansMarkers {
            let innerLength = selected.length - 2 * markerLength
            let inner = selected.substring(with: NSRange(
                location: markerLength,
                length: innerLength
            ))
            return Edit(
                range: selection,
                replacement: inner,
                selectionAfter: NSRange(location: selection.location, length: innerLength)
            )
        }

        // Markers sit immediately outside the selection: **|word|** → "word".
        let outerStart = selection.location - markerLength
        let outerEnd = NSMaxRange(selection) + markerLength
        if outerStart >= 0, outerEnd <= text.length {
            let before = text.substring(with: NSRange(location: outerStart, length: markerLength))
            let after = text.substring(with: NSRange(
                location: NSMaxRange(selection),
                length: markerLength
            ))
            if before == marker, after == marker {
                return Edit(
                    range: NSRange(location: outerStart, length: outerEnd - outerStart),
                    replacement: selected as String,
                    selectionAfter: NSRange(location: outerStart, length: selected.length)
                )
            }
        }

        // Otherwise wrap.
        return Edit(
            range: selection,
            replacement: marker + (selected as String) + marker,
            selectionAfter: NSRange(
                location: selection.location + markerLength,
                length: selected.length
            )
        )
    }

    /// Index in `chars` of the checkbox state character (`" "`/`"x"`) for a task
    /// box immediately after the bullet at `bulletAt`, or `nil` if absent.
    private static func taskStateIndex(_ chars: [Character], bulletAt: Int) -> Int? {
        let boxStart = bulletAt + 2
        guard boxStart + 3 < chars.count, chars[boxStart] == "[", chars[boxStart + 2] == "]",
              chars[boxStart + 3] == " ",
              chars[boxStart + 1] == " " || chars[boxStart + 1] == "x" || chars[boxStart + 1] == "X"
        else { return nil }
        return boxStart + 1
    }

    private static func toggleBox(at location: Int, checked: Bool, selection: NSRange) -> Edit {
        Edit(
            range: NSRange(location: location, length: 1),
            replacement: checked ? " " : "x",
            selectionAfter: selection
        )
    }

    private static func insert(_ string: String, at location: Int, selection: NSRange) -> Edit {
        let shifted = selection.location >= location
            ? NSRange(
                location: selection.location + (string as NSString).length,
                length: selection.length
            )
            : selection
        return Edit(
            range: NSRange(location: location, length: 0),
            replacement: string,
            selectionAfter: shifted
        )
    }

    private static func utf16Length(_ chars: [Character], upTo index: Int) -> Int {
        (String(chars[0 ..< index]) as NSString).length
    }
}
