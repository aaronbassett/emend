import EmendCore
import Foundation
import XCTest
@testable import Emend

/// End-to-end US5 link/embed/attachment flow (T104), headless and app-hosted —
/// the project bans XCUITest under signing-free CI, so this drives the real Rust
/// `WorkspaceHandle`/`OpenDocHandle` and the `EditorCoordinator`'s link services
/// directly (same rationale as `QuickOpenTests`/`WorkspaceFlowTests`). The native
/// `[[` completion UI + click hit-testing stay pragmatic-UI; their pure transforms
/// are covered by `LinkHelpersTests`.
@MainActor
final class LinksFlowTests: XCTestCase {
    private func canonical(_ path: String) -> String {
        path.withCString { cString in
            guard let resolved = realpath(cString, nil) else { return path }
            defer { free(resolved) }
            return String(cString: resolved)
        }
    }

    private struct Fixture {
        let workspace: WorkspaceHandle
        let root: URL
        let noteA: String
        let noteB: String
    }

    private func makeWorkspace() throws -> Fixture {
        let base = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-links-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        let root = URL(fileURLWithPath: canonical(base.path))
        let noteB = root.appendingPathComponent("Beta.md")
        let noteA = root.appendingPathComponent("Alpha.md")
        try "# Beta heading\n\nbeta body text\n".write(to: noteB, atomically: true, encoding: .utf8)
        try "See [[Beta]] and embed:\n\n![[Beta]]\n".write(
            to: noteA,
            atomically: true,
            encoding: .utf8
        )

        let workspace = newWorkspace()
        _ = try workspace.addLocation(folderPath: root.path, bookmark: Data())
        _ = try workspace.reindexAll(maxDepth: 32)
        return Fixture(workspace: workspace, root: root, noteA: noteA.path, noteB: noteB.path)
    }

    func testResolveAndSuggestWikilinks() throws {
        let fixture = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: fixture.root) }

        XCTAssertEqual(
            try fixture.workspace.resolveWikilink(fromNote: fixture.noteA, rawTarget: "Beta"),
            fixture.noteB
        )
        XCTAssertNil(try fixture.workspace.resolveWikilink(
            fromNote: fixture.noteA,
            rawTarget: "Nope"
        ))

        // Quick Open's SearchHit carries the file name with extension.
        let suggestions = try fixture.workspace.wikilinkSuggestions(prefix: "Bet", limit: 10)
        XCTAssertTrue(
            suggestions.contains { $0.name == "Beta.md" },
            "Beta is suggested for prefix 'Bet'"
        )
    }

    func testEmbedResolvesIntoPreviewHTML() throws {
        let fixture = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let workspace = fixture.workspace
        let noteA = fixture.noteA

        let handle = try openDocument(path: noteA)
        defer { try? handle.close() }

        // With the workspace, ![[Beta]] inlines Beta's content; without it, literal.
        let resolved = try handle.renderPreviewHtmlResolving(workspace: workspace)
        XCTAssertTrue(resolved.contains("beta body text"), "embed inlines Beta's body")
        XCTAssertFalse(resolved.contains("![[Beta]]"), "the raw embed token is consumed")

        let literal = try handle.renderPreviewHtml()
        XCTAssertFalse(literal.contains("beta body text"), "plain render leaves the embed literal")
    }

    func testStoreAttachmentWritesFileAndReturnsRef() throws {
        let fixture = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: fixture.root) }

        let pixel = Data([0x89, 0x50, 0x4E, 0x47]) // "‰PNG" header bytes — enough to store
        let ref = try storeAttachment(
            notePath: fixture.noteA,
            bytes: pixel,
            suggestedName: "shot.png"
        )
        XCTAssertFalse(ref.isEmpty)
        XCTAssertEqual(ImageDrop.markdown(forImageRef: ref), "![](\(ref))")

        let stored = fixture.root.appendingPathComponent(ref)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: stored.path),
            "the attachment is written note-relative at \(ref)"
        )
    }

    func testCoordinatorLinkServices() throws {
        let fixture = try makeWorkspace()
        defer { try? FileManager.default.removeItem(at: fixture.root) }

        let handle = try openDocument(path: fixture.noteA)
        defer { try? handle.close() }
        let coordinator = EditorCoordinator(
            handle: handle, autosave: AutosaveController(handle: handle), isReadOnly: false
        )
        coordinator.workspace = fixture.workspace
        coordinator.notePath = fixture.noteA

        XCTAssertEqual(coordinator.wikiSuggestions(prefix: "Bet"), ["Beta"])

        var opened: URL?
        coordinator.onOpenLink = { opened = $0 }
        coordinator.openWikiLink(rawTarget: "Beta")
        XCTAssertEqual(opened?.path, fixture.noteB)
    }
}
