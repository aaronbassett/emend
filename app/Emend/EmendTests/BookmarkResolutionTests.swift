import EmendCore
import XCTest
@testable import Emend

/// Headless tests for the security-scoped-bookmark lifecycle (T029, FR per §A4).
///
/// The test process is not sandboxed, so `.withSecurityScope` bookmarks can't be
/// created here — the tests use plain bookmarks (`options: []`). The resolve /
/// staleness / scope-balancing logic under test is identical regardless of the
/// flag; the security-scope behaviour itself is an OS concern validated when the
/// signed app runs.
final class BookmarkResolutionTests: XCTestCase {
    private func makeTempDir() throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appending(path: "emend-bookmark-test-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    func testBookmarkRoundTripResolvesToSameFolder() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }

        let data = try SecurityScopedBookmarks.makeBookmark(for: dir, options: [])
        let resolved = try SecurityScopedBookmarks.resolve(
            data, resolutionOptions: [], creationOptions: []
        )

        XCTAssertEqual(resolved.url.standardizedFileURL.path, dir.standardizedFileURL.path)
        XCTAssertNil(resolved.refreshedData, "a freshly created bookmark is not stale")
    }

    func testWithScopeRunsBodyAndReturnsValue() {
        let url = FileManager.default.temporaryDirectory
        let component = SecurityScopedBookmarks.withScope(url) { $0.lastPathComponent }
        XCTAssertEqual(component, url.lastPathComponent)
    }

    func testReadThroughScopeReachesRust() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appending(path: "note.md")
        try "hello world".write(to: file, atomically: true, encoding: .utf8)

        // The same chain handshakeRead() runs, with plain bookmark options.
        let data = try SecurityScopedBookmarks.makeBookmark(for: dir, options: [])
        let resolved = try SecurityScopedBookmarks.resolve(
            data, resolutionOptions: [], creationOptions: []
        )
        let text = try SecurityScopedBookmarks.withScope(resolved.url) { folder in
            try readFileAt(path: folder.appending(path: "note.md").path(percentEncoded: false))
        }
        XCTAssertEqual(text, "hello world")
    }
}
