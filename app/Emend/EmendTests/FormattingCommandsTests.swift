import Foundation
import XCTest
@testable import Emend

/// Headless tests for the inline-formatting transforms (T046). Each transform is
/// pure over `(text, selection)`; the tests apply the returned `Edit` and assert
/// the resulting Markdown and the surviving selection.
final class FormattingCommandsTests: XCTestCase {
    private func applied(_ edit: FormattingCommands.Edit, to text: String) -> String {
        let mutable = NSMutableString(string: text)
        mutable.replaceCharacters(in: edit.range, with: edit.replacement)
        return mutable as String
    }

    private func substring(_ text: String, _ range: NSRange) -> String {
        (text as NSString).substring(with: range)
    }

    // MARK: - Bold / italic

    func testBoldWrapsSelection() {
        let text = "hello world"
        let edit = FormattingCommands.bold(
            in: text as NSString,
            selection: NSRange(location: 6, length: 5)
        )
        let result = applied(edit, to: text)
        XCTAssertEqual(result, "hello **world**")
        XCTAssertEqual(substring(result, edit.selectionAfter), "world")
    }

    func testBoldEmptySelectionInsertsPairWithCaretBetween() {
        let edit = FormattingCommands.bold(
            in: "" as NSString,
            selection: NSRange(location: 0, length: 0)
        )
        XCTAssertEqual(applied(edit, to: ""), "****")
        XCTAssertEqual(edit.selectionAfter, NSRange(location: 2, length: 0))
    }

    func testBoldUnwrapsSelectionThatIncludesMarkers() {
        let text = "**word**"
        let edit = FormattingCommands.bold(
            in: text as NSString,
            selection: NSRange(location: 0, length: 8)
        )
        XCTAssertEqual(applied(edit, to: text), "word")
    }

    func testItalicWrapsSelection() {
        let text = "hi"
        let edit = FormattingCommands.italic(
            in: text as NSString,
            selection: NSRange(location: 0, length: 2)
        )
        XCTAssertEqual(applied(edit, to: text), "*hi*")
    }

    func testItalicOnBareDoubleAsteriskIsNotDeleted() {
        // "**" selected with the italic marker "*" must NOT unwrap to "" (data loss).
        let text = "**"
        let edit = FormattingCommands.italic(
            in: text as NSString,
            selection: NSRange(location: 0, length: 2)
        )
        XCTAssertNotEqual(applied(edit, to: text), "")
    }

    func testBoldOnBareMarkersIsNotDeleted() {
        // "****" selected with the bold marker "**" must NOT unwrap to "" (data loss).
        let text = "****"
        let edit = FormattingCommands.bold(
            in: text as NSString,
            selection: NSRange(location: 0, length: 4)
        )
        XCTAssertNotEqual(applied(edit, to: text), "")
    }

    // MARK: - Link

    func testLinkWrapsSelectionAndSelectsURL() {
        let text = "Anthropic"
        let edit = FormattingCommands.link(
            in: text as NSString,
            selection: NSRange(location: 0, length: 9)
        )
        let result = applied(edit, to: text)
        XCTAssertEqual(result, "[Anthropic](url)")
        XCTAssertEqual(substring(result, edit.selectionAfter), "url")
    }

    func testLinkEmptySelectionInsertsTemplate() {
        let edit = FormattingCommands.link(
            in: "" as NSString,
            selection: NSRange(location: 0, length: 0)
        )
        let result = applied(edit, to: "")
        XCTAssertEqual(result, "[text](url)")
        XCTAssertEqual(substring(result, edit.selectionAfter), "text")
    }

    // MARK: - Task

    func testTaskConvertsPlainLine() {
        let text = "buy milk"
        let edit = FormattingCommands.task(
            in: text as NSString,
            selection: NSRange(location: 0, length: 0)
        )
        XCTAssertEqual(applied(edit, to: text), "- [ ] buy milk")
    }

    func testTaskTogglesExistingTask() {
        let text = "- [ ] buy milk"
        let edit = FormattingCommands.task(
            in: text as NSString,
            selection: NSRange(location: 6, length: 0)
        )
        XCTAssertEqual(applied(edit, to: text), "- [x] buy milk")
    }

    func testTaskConvertsBulletLine() {
        let text = "- buy milk"
        let edit = FormattingCommands.task(
            in: text as NSString,
            selection: NSRange(location: 2, length: 0)
        )
        XCTAssertEqual(applied(edit, to: text), "- [ ] buy milk")
    }
}
