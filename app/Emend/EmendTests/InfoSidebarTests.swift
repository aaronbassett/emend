import EmendCore
import Foundation
import XCTest
@testable import Emend

/// Headless coverage for the info sidebar's data (US6 · T119) — drives `InfoModel`
/// against a real `OpenDocHandle` and asserts the live stats + outline. App-hosted,
/// not XCUITest (the `EmendUITests` target doesn't exist by design); the sidebar
/// view itself is pragmatic-UI.
@MainActor
final class InfoSidebarTests: XCTestCase {
    func testStatsAndOutlinePopulateFromDocument() async throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-info-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let note = dir.appendingPathComponent("Doc.md")
        try "# Title\n\nHello world test.\n\n## Section\n\n- [ ] a\n- [x] b\n"
            .write(to: note, atomically: true, encoding: .utf8)

        let handle = try openDocument(path: note.path)
        defer { try? handle.close() }

        let model = InfoModel()
        model.setActiveDocument(handle)

        // Compute is async (off-main); poll briefly for it to land.
        for _ in 0 ..< 100 where model.stats == nil || model.outline.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
        }

        let stats = try XCTUnwrap(model.stats)
        XCTAssertGreaterThan(stats.words, 0)
        XCTAssertEqual(stats.tasksTotal, 2)
        XCTAssertEqual(stats.tasksDone, 1)
        XCTAssertEqual(model.outline.map(\.title), ["Title", "Section"])
        XCTAssertEqual(model.outline.first?.level, 1)
    }
}
