import AppKit
import EmendCore

/// Maps the core's highlight `StyleSpan`s to AppKit display attributes for the
/// editor (research §C1).
///
/// Markers stay in the buffer for a lossless round-trip but render **dimmed**;
/// bold/italic/headings/code/quote render inline. This is pure given
/// `(text length, spans, base font)` → attributes, so the real styling logic is
/// unit-tested headlessly without a window (Constitution VII). Span ranges are
/// UTF-16 code units, which map 1:1 onto `NSRange` (research §A2).
enum SyntaxAttributing {
    /// Display attributes for one style class layered over `baseFont`.
    static func attributes(
        for styleClass: StyleClass,
        baseFont: NSFont
    ) -> [NSAttributedString.Key: Any] {
        switch styleClass {
        case .syntaxMarker:
            // Markers remain in the text but read low-contrast.
            [.foregroundColor: NSColor.tertiaryLabelColor]
        case let .heading(level):
            [.font: withTraits(systemFont(ofSize: baseFont.pointSize * headingScale(level)), .bold)]
        case .strong:
            [.font: withTraits(baseFont, .bold)]
        case .emphasis:
            [.font: withTraits(baseFont, .italic)]
        case .inlineCode, .codeBlock:
            [
                .font: NSFont.monospacedSystemFont(ofSize: baseFont.pointSize, weight: .regular),
                .foregroundColor: NSColor.secondaryLabelColor
            ]
        case .blockQuote:
            [.foregroundColor: NSColor.secondaryLabelColor]
        case .listMarker:
            [.foregroundColor: NSColor.secondaryLabelColor]
        case .link:
            [.foregroundColor: NSColor.linkColor]
        case .highlight:
            [.backgroundColor: NSColor.systemYellow.withAlphaComponent(0.3)]
        }
    }

    /// Layer the spans' attributes onto `attributed` (later spans override earlier
    /// where ranges overlap — e.g. a dimmed marker inside a heading). Ranges that
    /// fall outside the string are skipped defensively.
    static func apply(
        spans: [StyleSpan],
        to attributed: NSMutableAttributedString,
        baseFont: NSFont
    ) {
        let length = attributed.length
        for span in spans {
            let location = Int(span.range.start)
            let spanLength = Int(span.range.len)
            guard location >= 0, spanLength > 0, location + spanLength <= length else { continue }
            attributed.addAttributes(
                attributes(for: span.class, baseFont: baseFont),
                range: NSRange(location: location, length: spanLength)
            )
        }
    }

    // MARK: - Font helpers (NSFontDescriptor — usable off the main actor)

    private static func headingScale(_ level: UInt8?) -> CGFloat {
        switch level {
        case 1: 1.8
        case 2: 1.5
        case 3: 1.3
        case 4: 1.2
        default: 1.1
        }
    }

    private static func systemFont(ofSize size: CGFloat) -> NSFont {
        NSFont.systemFont(ofSize: size)
    }

    private static func withTraits(
        _ font: NSFont,
        _ traits: NSFontDescriptor.SymbolicTraits
    ) -> NSFont {
        let descriptor = font.fontDescriptor
            .withSymbolicTraits(font.fontDescriptor.symbolicTraits.union(traits))
        return NSFont(descriptor: descriptor, size: font.pointSize) ?? font
    }
}
