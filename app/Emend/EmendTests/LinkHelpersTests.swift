import Foundation
import XCTest
@testable import Emend

/// Headless unit tests for the pure `[[wiki link]]` + task-checkbox transforms
/// (US5 · FR-014/FR-018/FR-019). These back the editor's native autocomplete,
/// ⌘-click navigation, unresolved-link styling, and clickable checkboxes — driving
/// the pure logic directly per the project's headless-testing rule (no XCUITest).
@MainActor
final class LinkHelpersTests: XCTestCase {
    // MARK: - WikiLink.partialRange (drives `[[` autocomplete)

    func testPartialRangeInsideOpenLink() {
        let text = "see [[Foo" as NSString
        let range = WikiLink.partialRange(in: text, caret: text.length)
        XCTAssertEqual(range, NSRange(location: 6, length: 3)) // "Foo"
    }

    func testPartialRangeEmptyJustAfterBrackets() {
        let text = "x [[" as NSString
        XCTAssertEqual(WikiLink.partialRange(in: text, caret: 4), NSRange(location: 4, length: 0))
    }

    func testPartialRangeNilWhenClosed() {
        let text = "[[Foo]] bar" as NSString
        XCTAssertNil(WikiLink.partialRange(in: text, caret: text.length))
    }

    func testPartialRangeNilAcrossNewline() {
        let text = "[[\nFoo" as NSString
        XCTAssertNil(WikiLink.partialRange(in: text, caret: text.length))
    }

    // MARK: - WikiLink.enclosingLink (drives ⌘-click navigation)

    func testEnclosingLinkAtClick() {
        let text = "go to [[My Note]] now" as NSString
        let hit = WikiLink.enclosingLink(in: text, at: 10) // inside "My Note"
        XCTAssertEqual(hit?.raw, "My Note")
        XCTAssertEqual(hit?.span, NSRange(location: 6, length: 11)) // "[[My Note]]"
    }

    func testEnclosingLinkNilOutsideAnyLink() {
        let text = "plain text only" as NSString
        XCTAssertNil(WikiLink.enclosingLink(in: text, at: 4))
    }

    // MARK: - WikiLink.allLinks (drives unresolved-link styling)

    func testAllLinksFindsBothPlainAndEmbed() {
        let text = "a [[One]] b ![[Two]] c" as NSString
        let links = WikiLink.allLinks(in: text)
        XCTAssertEqual(links.map(\.raw), ["One", "Two"])
    }

    // MARK: - TaskCheckbox

    func testCheckboxRangeDetectsUncheckedItem() {
        let text = "- [ ] todo" as NSString
        XCTAssertEqual(
            TaskCheckbox.checkboxRange(in: text, atLineContaining: 8),
            NSRange(location: 2, length: 3)
        )
    }

    func testCheckboxRangeNilForNonTaskLine() {
        let text = "just a paragraph" as NSString
        XCTAssertNil(TaskCheckbox.checkboxRange(in: text, atLineContaining: 3))
    }

    func testToggleEditFlipsBothWays() throws {
        let unchecked = "  - [ ] a" as NSString
        let on = try XCTUnwrap(TaskCheckbox.toggleEdit(in: unchecked, atLineContaining: 0))
        XCTAssertEqual(on.replacement, "x")
        XCTAssertEqual(unchecked.substring(with: on.range), " ")

        let checked = "- [x] done" as NSString
        let off = TaskCheckbox.toggleEdit(in: checked, atLineContaining: 5)
        XCTAssertEqual(off?.replacement, " ")
    }

    func testToggleEditFindsCheckboxOnSecondLine() {
        let text = "intro\n- [ ] second" as NSString
        let edit = TaskCheckbox.toggleEdit(in: text, atLineContaining: 12)
        XCTAssertEqual(edit?.replacement, "x")
        XCTAssertEqual(edit?.range, NSRange(location: 9, length: 1)) // the space in "[ ]"
    }
}
