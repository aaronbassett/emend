import Foundation
import XCTest
@testable import Emend

/// Headless tests for the smart-list transforms (T045). Each transform is pure
/// over `(text, selection)`; the tests apply the returned `Edit` to a plain
/// string and assert the resulting Markdown.
final class SmartListsTests: XCTestCase {
    private func applied(_ edit: SmartLists.Edit?, to text: String) -> String? {
        guard let edit else { return nil }
        let mutable = NSMutableString(string: text)
        mutable.replaceCharacters(in: edit.range, with: edit.replacement)
        return mutable as String
    }

    private func caret(_ location: Int) -> NSRange {
        NSRange(location: location, length: 0)
    }

    // MARK: - Return / newline

    func testReturnContinuesBulletList() throws {
        let text = "- hello"
        let edit = SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count))
        XCTAssertEqual(applied(edit, to: text), "- hello\n- ")
        XCTAssertEqual(try XCTUnwrap(edit).selectionAfter, caret(10))
    }

    func testReturnContinuesOrderedListIncrementsNumber() {
        let text = "1. hello"
        let edit = SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count))
        XCTAssertEqual(applied(edit, to: text), "1. hello\n2. ")
    }

    func testReturnContinuesTaskItemUnchecked() {
        let text = "- [x] done"
        let edit = SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count))
        XCTAssertEqual(applied(edit, to: text), "- [x] done\n- [ ] ")
    }

    func testReturnPreservesIndentation() {
        let text = "  - item"
        let edit = SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count))
        XCTAssertEqual(applied(edit, to: text), "  - item\n  - ")
    }

    func testReturnOnEmptyItemTerminatesList() {
        let text = "- "
        let edit = SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count))
        XCTAssertEqual(applied(edit, to: text), "")
    }

    func testReturnOnNonListLineReturnsNil() {
        let text = "plain text"
        XCTAssertNil(SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count)))
    }

    // MARK: - Renumber

    func testRenumberMakesOrderedListSequential() {
        let text = "1. a\n5. b\n2. c"
        let edit = SmartLists.renumber(in: text as NSString, selection: caret(0))
        XCTAssertEqual(applied(edit, to: text), "1. a\n2. b\n3. c")
    }

    func testRenumberPreservesStartingNumber() {
        let text = "3. a\n3. b"
        let edit = SmartLists.renumber(in: text as NSString, selection: caret(0))
        XCTAssertEqual(applied(edit, to: text), "3. a\n4. b")
    }

    func testRenumberReturnsNilForBulletList() {
        let text = "- a\n- b"
        XCTAssertNil(SmartLists.renumber(in: text as NSString, selection: caret(0)))
    }

    // MARK: - Indent / outdent

    func testIndentAddsUnitToListLine() {
        let text = "- item"
        let edit = SmartLists.indent(in: text as NSString, selection: caret(2))
        XCTAssertEqual(applied(edit, to: text), "  - item")
    }

    func testIndentReturnsNilForNonListLine() {
        let text = "prose"
        XCTAssertNil(SmartLists.indent(in: text as NSString, selection: caret(2)))
    }

    func testOutdentRemovesIndentUnit() {
        let text = "  - item"
        let edit = SmartLists.outdent(in: text as NSString, selection: caret(4))
        XCTAssertEqual(applied(edit, to: text), "- item")
    }

    func testOutdentReturnsNilWhenNotIndented() {
        let text = "- item"
        XCTAssertNil(SmartLists.outdent(in: text as NSString, selection: caret(2)))
    }
}
