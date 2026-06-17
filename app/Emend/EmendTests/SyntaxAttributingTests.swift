import AppKit
import EmendCore
import XCTest
@testable import Emend

/// Headless tests for the editor's span→attribute mapping (T041). Spans are
/// constructed directly so the styling logic is tested in isolation from the
/// tree-sitter highlighter.
final class SyntaxAttributingTests: XCTestCase {
    private let base = NSFont.systemFont(ofSize: 13)

    func testHeadingMarkerIsDimmedAndHeadingIsLargerBold() {
        let source = "### Title"
        let attributed = NSMutableAttributedString(string: source)
        attributed.addAttribute(
            .font, value: base, range: NSRange(location: 0, length: attributed.length)
        )

        let spans = [
            StyleSpan(range: U16Range(start: 0, len: 3), class: .syntaxMarker),
            StyleSpan(
                range: U16Range(start: 0, len: UInt32(source.utf16.count)),
                class: .heading(level: 3)
            )
        ]
        SyntaxAttributing.apply(spans: spans, to: attributed, baseFont: base)

        // Heading text ("Title", offset 5) is larger + bold.
        let headingFont = attributed.attribute(.font, at: 5, effectiveRange: nil) as? NSFont
        XCTAssertNotNil(headingFont)
        XCTAssertGreaterThan(headingFont?.pointSize ?? 0, base.pointSize)
        XCTAssertTrue(headingFont?.fontDescriptor.symbolicTraits.contains(.bold) ?? false)

        // The "###" marker is dimmed (different attribute key than the heading font,
        // so both apply over the marker range).
        let markerColor = attributed.attribute(
            .foregroundColor,
            at: 0,
            effectiveRange: nil
        ) as? NSColor
        XCTAssertEqual(markerColor, NSColor.tertiaryLabelColor)
    }

    func testStrongAndEmphasisAddTraits() {
        let strongFont = SyntaxAttributing
            .attributes(for: .strong, baseFont: base)[.font] as? NSFont
        XCTAssertTrue(strongFont?.fontDescriptor.symbolicTraits.contains(.bold) ?? false)

        let emphasisFont = SyntaxAttributing
            .attributes(for: .emphasis, baseFont: base)[.font] as? NSFont
        XCTAssertTrue(emphasisFont?.fontDescriptor.symbolicTraits.contains(.italic) ?? false)
    }

    func testCodeIsMonospacedAndHighlightHasBackground() {
        let codeFont = SyntaxAttributing
            .attributes(for: .inlineCode, baseFont: base)[.font] as? NSFont
        XCTAssertTrue(codeFont?.fontDescriptor.symbolicTraits.contains(.monoSpace) ?? false)

        let background = SyntaxAttributing
            .attributes(for: .highlight, baseFont: base)[.backgroundColor]
        XCTAssertNotNil(background)
    }

    func testOutOfRangeSpansAreSkipped() {
        let attributed = NSMutableAttributedString(string: "hi")
        // Span past the end must be ignored, not crash.
        let spans = [StyleSpan(range: U16Range(start: 5, len: 10), class: .strong)]
        SyntaxAttributing.apply(spans: spans, to: attributed, baseFont: base)
        XCTAssertEqual(attributed.string, "hi")
    }
}
