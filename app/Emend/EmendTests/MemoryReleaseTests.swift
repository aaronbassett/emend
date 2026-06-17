import EmendCore
import Foundation
import XCTest
@testable import Emend

/// NFR-005 (bounded memory): resident memory MUST track open documents plus the
/// index, not total workspace size — and **closing a tab MUST release that
/// document's buffer**. This drives `TabModel` directly and asserts the
/// Rust-backed `OpenDocHandle` (which owns the rope) and its `AutosaveController`
/// deallocate once the tab is closed or reloaded.
///
/// Headless and app-hosted (Constitution VII — no GUI automation under CI's
/// `CODE_SIGNING_ALLOWED=NO`): the model is the layer that owns document lifetime,
/// so the release contract is testable here without launching a window. The weak
/// references resolve to `nil` the instant the last strong reference drops, so a
/// surviving buffer (a leak / retain cycle) fails the test deterministically.
@MainActor
final class MemoryReleaseTests: XCTestCase {
    func testClosingTabReleasesDocumentBuffer() throws {
        let directory = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("note.md")
        try "# Heading\n\nSome body text.\n".write(to: url, atomically: true, encoding: .utf8)

        let model = TabModel()
        weak var weakHandle: OpenDocHandle?
        weak var weakAutosave: AutosaveController?
        let tabID: UUID

        // Open in a tight scope so the only strong references to the handle and its
        // autosave live inside the model's `tabs` array — not in a local that would
        // outlive the close and mask a leak.
        do {
            model.open(url: url)
            let tab = try XCTUnwrap(model.tabs.last, "open must add a tab")
            tabID = tab.id
            weakHandle = tab.handle
            weakAutosave = tab.autosave
        }
        XCTAssertNotNil(weakHandle, "the document buffer must be alive while the tab is open")
        XCTAssertEqual(model.tabs.count, 1)

        model.close(tabID)

        XCTAssertTrue(model.tabs.isEmpty, "closing the tab removes it")
        XCTAssertNil(weakHandle, "closing a tab must release the document buffer (NFR-005)")
        XCTAssertNil(weakAutosave, "closing a tab must release its autosave controller (NFR-005)")
    }

    func testReloadReleasesSupersededBuffer() throws {
        // Reload-from-disk recreates the handle with the on-disk text; the buffer it
        // supersedes must be freed so a reloaded document never accumulates stale
        // buffers (NFR-005). The tab itself stays open, now backed by a fresh handle.
        let directory = try makeTempDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("note.md")
        try "first revision\n".write(to: url, atomically: true, encoding: .utf8)

        let model = TabModel()
        model.open(url: url)
        let tabID = try XCTUnwrap(model.tabs.last, "open must add a tab").id

        weak var supersededHandle: OpenDocHandle?
        do {
            supersededHandle = try XCTUnwrap(model.tabs.last).handle
        }
        XCTAssertNotNil(supersededHandle, "the original buffer is alive before reload")

        model.reload(tabID)

        XCTAssertNil(
            supersededHandle,
            "reload must release the superseded document buffer (NFR-005)"
        )
        XCTAssertEqual(model.tabs.count, 1, "reload keeps the tab, backed by a fresh handle")
    }

    // MARK: - Helpers

    private func makeTempDirectory() throws -> URL {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("emend-mem-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }
}
