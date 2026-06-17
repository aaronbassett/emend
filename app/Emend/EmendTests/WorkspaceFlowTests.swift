import EmendCore
import Foundation
import XCTest
@testable import Emend

/// End-to-end workspace flow (T067, US2): add a folder → list its tree → open a
/// file in a tab → move/rename. Headless and app-hosted, driving the real Rust
/// `WorkspaceHandle` + the `WorkspaceModel`/`TabModel` glue — not XCUITest, which
/// cannot bootstrap under the project's `CODE_SIGNING_ALLOWED=NO` CI (same
/// rationale as US1's `EditorPersistenceTests`). The `NSOutlineView` rendering and
/// the security-scoped add-location panel remain pragmatic-UI (Constitution VII).
@MainActor
final class WorkspaceFlowTests: XCTestCase {
    // MARK: - Core workspace flow (FFI)

    func testAddLocationListsFolderTree() throws {
        let dir = try seededDirectory(files: ["alpha.md", "beta.md"], folders: ["sub"])
        defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = newWorkspace()

        let location = try workspace.addLocation(folderPath: dir.path, bookmark: Data())
        // `add_location` stores the *canonical* root (US3 path-identity fix), so a
        // non-canonical temp path (/var → /private/var) comes back resolved.
        XCTAssertEqual(location.path, canonicalPath(dir.path))
        XCTAssertTrue(try workspace.listLocations().contains { $0.id == location.id })

        let names = try Set(workspace.listChildren(folderPath: dir.path).map(\.name))
        XCTAssertEqual(names, ["alpha.md", "beta.md", "sub"])
    }

    func testCreateNoteIsCollisionSafe() throws {
        let dir = try seededDirectory(files: ["note.md"], folders: [])
        defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = newWorkspace()

        let created = try workspace.createNote(parent: dir.path, name: "note.md")
        XCTAssertNotEqual(created, dir.appendingPathComponent("note.md").path)
        XCTAssertTrue(FileManager.default.fileExists(atPath: created))
        // The original is preserved.
        XCTAssertTrue(FileManager.default
            .fileExists(atPath: dir.appendingPathComponent("note.md").path))
    }

    // MARK: - WorkspaceModel glue

    func testModelListsFolderChildrenAndMovesFile() throws {
        let dir = try seededDirectory(files: ["doc.md"], folders: ["archive"])
        defer { try? FileManager.default.removeItem(at: dir) }
        let model = WorkspaceModel(workspace: newWorkspace(), defaults: isolatedDefaults())

        let folder = WorkspaceNode(url: dir, name: dir.lastPathComponent, kind: .folder)
        let childNames = Set(model.children(of: folder).map(\.name))
        XCTAssertEqual(childNames, ["doc.md", "archive"])

        let archive = WorkspaceNode(
            url: dir.appendingPathComponent("archive"),
            name: "archive",
            kind: .folder
        )
        let moved = model.move(sourcePath: dir.appendingPathComponent("doc.md").path, into: archive)
        XCTAssertTrue(moved)
        XCTAssertTrue(FileManager.default.fileExists(
            atPath: dir.appendingPathComponent("archive/doc.md").path
        ))
        XCTAssertFalse(FileManager.default
            .fileExists(atPath: dir.appendingPathComponent("doc.md").path))
    }

    func testFavoriteAndIconPersistAcrossModelReload() throws {
        let dir = try seededDirectory(files: ["keep.md"], folders: [])
        defer { try? FileManager.default.removeItem(at: dir) }
        let path = dir.appendingPathComponent("keep.md").path
        let suite = "emend.test.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite) ?? .standard
        defer { defaults.removePersistentDomain(forName: suite) }

        let model = WorkspaceModel(workspace: newWorkspace(), defaults: defaults)
        model.toggleFavorite(path)
        model.setIcon(FolderIcon(symbol: "star.fill", tint: .blue), for: path)
        XCTAssertTrue(model.isFavorite(path))

        // A fresh model on the same store restores the persisted app state.
        let reloaded = WorkspaceModel(workspace: newWorkspace(), defaults: defaults)
        XCTAssertTrue(reloaded.isFavorite(path))
        XCTAssertEqual(reloaded.icon(for: path)?.symbol, "star.fill")
        XCTAssertEqual(reloaded.icon(for: path)?.tint, .blue)
    }

    // MARK: - Tab flow

    func testOpenFileCreatesActiveTab() throws {
        let dir = try seededDirectory(files: ["open-me.md"], folders: [])
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("open-me.md")
        try "# Open me".write(to: url, atomically: true, encoding: .utf8)

        let tabs = TabModel()
        tabs.open(url: url)
        XCTAssertEqual(tabs.tabs.count, 1)
        XCTAssertEqual(tabs.active?.url, url)
        XCTAssertEqual(tabs.active?.text, "# Open me")

        // Re-opening the same file focuses the existing tab rather than duplicating.
        tabs.open(url: url)
        XCTAssertEqual(tabs.tabs.count, 1)
    }

    // MARK: - Helpers

    private func seededDirectory(files: [String], folders: [String]) throws -> URL {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("emend-wsflow-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        for folder in folders {
            try FileManager.default.createDirectory(
                at: dir.appendingPathComponent(folder),
                withIntermediateDirectories: true
            )
        }
        for file in files {
            try "seed".write(
                to: dir.appendingPathComponent(file),
                atomically: true,
                encoding: .utf8
            )
        }
        return dir
    }

    private func isolatedDefaults() -> UserDefaults {
        UserDefaults(suiteName: "emend.test.\(UUID().uuidString)") ?? .standard
    }

    /// The canonical (symlink-resolved) path, matching Rust's `std::fs::canonicalize`
    /// / `realpath(3)` that `add_location` now applies. Foundation's
    /// `resolvingSymlinksInPath` deliberately avoids the `/private` prefix on macOS
    /// (`/var` stays `/var`), so it can't reproduce the core's identity here.
    private func canonicalPath(_ path: String) -> String {
        guard let resolved = realpath(path, nil) else { return path }
        defer { free(resolved) }
        return String(cString: resolved)
    }
}
