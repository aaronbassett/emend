import EmendCore
import Foundation
import PDFKit
import XCTest
@testable import Emend

/// PDF export (T091, US4 · FR-026 / SC-010): render a long document through the
/// real core preview renderer, export it via the off-screen `PDFExport` print
/// host, and assert true multi-page pagination.
///
/// App-hosted and headless (drives `PDFExport` + the core directly) rather than
/// XCUITest, which can't bootstrap under the project's `CODE_SIGNING_ALLOWED=NO`
/// CI — same rationale as `EditorPersistenceTests`/`QuickOpenTests`. The macos-14
/// runner has a window server, so the off-screen `WKWebView` lays out and prints.
@MainActor
final class PreviewExportTests: XCTestCase {
    func testExportProducesMultiPagePDF() async throws {
        // A document long enough to span several Letter/A4 pages once paginated.
        let markdown = Self.longDocument(sections: 60)
        let source = try writeTempNote(markdown)
        defer { try? FileManager.default.removeItem(at: source) }

        let handle = try openDocument(path: source.path)
        defer { try? handle.close() }
        let html = try handle.renderPreviewHtml()
        XCTAssertTrue(html.contains("Section 1"), "core rendered the document body")

        let output = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-export-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: output) }

        try await PDFExport.export(html: html, css: previewThemeCss(), to: output)

        XCTAssertTrue(
            FileManager.default.fileExists(atPath: output.path),
            "the PDF was written to disk"
        )
        let pdf = try XCTUnwrap(PDFDocument(url: output), "the output is a readable PDF")
        XCTAssertGreaterThan(
            pdf.pageCount, 1,
            "a long document paginates into multiple pages (SC-010), not one tall page"
        )
    }

    func testExportMissingTemplateSurfacesNothingWhenPresent() async throws {
        // Sanity: the bundled preview template the exporter depends on is present,
        // so export does not fail with `.templateMissing` in the test bundle host.
        let output = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-export-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: output) }

        try await PDFExport.export(
            html: "<h1>Short</h1><p>One page.</p>",
            css: previewThemeCss(),
            to: output
        )
        XCTAssertTrue(FileManager.default.fileExists(atPath: output.path))
    }

    // MARK: - Helpers

    private func writeTempNote(_ contents: String) throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-export-\(UUID().uuidString).md")
        try contents.write(to: url, atomically: true, encoding: .utf8)
        return url
    }

    private static func longDocument(sections: Int) -> String {
        var lines: [String] = ["# Export Fixture", ""]
        let paragraph = String(
            repeating: "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ",
            count: 4
        )
        for index in 1 ... sections {
            lines.append("## Section \(index)")
            lines.append("")
            lines.append(paragraph)
            lines.append("")
        }
        return lines.joined(separator: "\n")
    }
}
