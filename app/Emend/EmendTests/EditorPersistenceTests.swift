import AppKit
import EmendCore
import XCTest
@testable import Emend

/// End-to-end editor persistence (T049, FR-009): drive the real
/// `EditorCoordinator` + `AutosaveController` over an open document exactly as
/// `MarkdownEditorView` wires them, then confirm edits survive the atomic flush
/// and round-trip back from disk through the Rust core.
///
/// This is headless (no GUI launch / code signing) so it runs in the standard
/// app-hosted test bundle on CI — matching the project's deliberate "no GUI
/// automation in CI" design (Constitution VII), unlike an XCUITest runner which
/// cannot bootstrap under `CODE_SIGNING_ALLOWED=NO`.
@MainActor
final class EditorPersistenceTests: XCTestCase {
    func testTypedTextFlushesToDiskAndRoundTrips() throws {
        let directory = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("note.md")
        try "".write(to: url, atomically: true, encoding: .utf8)
        let path = url.path(percentEncoded: false)
        let expected = "Persisted through the Rust core"

        let handle = try openDocument(path: path)
        let editor = makeEditor(handle: handle, initialText: (try? readFileAt(path: path)) ?? "")
        type(expected, into: editor.textView, at: 0)
        editor.autosave.flushNow()
        try handle.close()

        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), expected)
        XCTAssertEqual(try readFileAt(path: path), expected)
    }

    func testEditingExistingDocumentPersists() throws {
        let directory = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("seed.md")
        try "# Title\n".write(to: url, atomically: true, encoding: .utf8)
        let path = url.path(percentEncoded: false)

        let handle = try openDocument(path: path)
        let initial = (try? readFileAt(path: path)) ?? ""
        let editor = makeEditor(handle: handle, initialText: initial)
        let end = editor.textView.textStorage?.length ?? 0
        type("body text", into: editor.textView, at: end)
        editor.autosave.flushNow()
        try handle.close()

        XCTAssertEqual(try readFileAt(path: path), "# Title\nbody text")
    }

    // MARK: - Helpers

    private func makeTempDirectory() throws -> URL {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("emend-itest-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private struct Editor {
        let textView: NSTextView
        let coordinator: EditorCoordinator
        let autosave: AutosaveController
    }

    /// Wire a real `EditorCoordinator` to a text view as `MarkdownEditorView`
    /// does: load the document's text, then attach the storage delegate so later
    /// edits flow to the core through the production path.
    private func makeEditor(handle: OpenDocHandle, initialText: String) -> Editor {
        let autosave = AutosaveController(handle: handle)
        let coordinator = EditorCoordinator(handle: handle, autosave: autosave, isReadOnly: false)
        let textView = NSTextView()
        textView.textStorage?.setAttributedString(NSAttributedString(string: initialText))
        textView.textStorage?.delegate = coordinator
        coordinator.attach(textView)
        return Editor(textView: textView, coordinator: coordinator, autosave: autosave)
    }

    private func type(_ text: String, into textView: NSTextView, at location: Int) {
        textView.textStorage?.replaceCharacters(
            in: NSRange(location: location, length: 0),
            with: text
        )
    }
}
