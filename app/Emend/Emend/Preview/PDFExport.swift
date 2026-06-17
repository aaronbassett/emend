import AppKit
import WebKit

/// Off-screen PDF export of the live preview (US4 · FR-026 / SC-010, research §C4).
///
/// Renders the same comrak HTML + bundled, offline Mermaid/KaTeX as the on-screen
/// preview into a dedicated off-screen `WKWebView`, then paginates it to a PDF via
/// `NSPrintOperation` with the `@media print`/`@page` rules in `theme.css`.
///
/// `WKWebView.createPDF` is deliberately avoided: it emits a single tall page and
/// ignores pagination (Apple forums 700418/705138). `NSPrintOperation` gives true
/// multi-page output with the highest fidelity to the on-screen preview.
@MainActor
enum PDFExport {
    enum Failure: Error, LocalizedError {
        case templateMissing
        case renderFailed(String)
        case printFailed

        var errorDescription: String? {
            switch self {
            case .templateMissing: "The preview template is missing from the app bundle."
            case let .renderFailed(detail): "Could not render the document for export: \(detail)"
            case .printFailed: "Could not write the PDF."
            }
        }
    }

    /// Render `html`/`css` off-screen and write a paginated PDF to `url`.
    static func export(html: String, css: String, to url: URL) async throws {
        let host = OffscreenPrintHost()
        try await host.export(html: html, css: css, to: url)
    }
}

/// Drives one off-screen render→print cycle. Holds the WebView and a borderless,
/// far-off-screen window (so WebKit lays out and runs Mermaid's async JS rather
/// than throttling an occluded view) for the duration of the export.
@MainActor
private final class OffscreenPrintHost: NSObject, WKNavigationDelegate {
    private var webView: WKWebView?
    private var window: NSWindow?
    private var loadContinuation: CheckedContinuation<Void, Error>?
    private var printContinuation: CheckedContinuation<Bool, Error>?

    func export(html: String, css: String, to url: URL) async throws {
        guard let dir = Bundle.main.url(forResource: "preview", withExtension: nil) else {
            throw PDFExport.Failure.templateMissing
        }
        let template = dir.appendingPathComponent("template.html")
        guard FileManager.default.fileExists(atPath: template.path) else {
            throw PDFExport.Failure.templateMissing
        }

        let printInfo = Self.makePrintInfo(savingTo: url)
        let webView = makeWebView(size: Self.contentSize(for: printInfo))
        defer { cleanup() }

        try await loadTemplate(template, baseDir: dir, into: webView)
        try await renderContent(html: html, css: css, in: webView)
        try await paginate(webView, with: printInfo)
    }

    /// 1. Load the offline template (grants read access to katex/, mermaid, css).
    /// A watchdog guarantees the export can never hang on a stalled navigation.
    private func loadTemplate(_ template: URL, baseDir: URL, into webView: WKWebView) async throws {
        try await withCheckedThrowingContinuation { continuation in
            loadContinuation = continuation
            DispatchQueue.main.asyncAfter(deadline: .now() + 20) { [weak self] in
                guard let self, let pending = loadContinuation else { return }
                loadContinuation = nil
                pending.resume(throwing: PDFExport.Failure.renderFailed("template load timed out"))
            }
            webView.loadFileURL(template, allowingReadAccessTo: baseDir)
        }
    }

    /// 2. Inject content and await Mermaid's async layout (KaTeX is synchronous).
    private func renderContent(html: String, css: String, in webView: WKWebView) async throws {
        do {
            _ = try await webView.callAsyncJavaScript(
                "await window.__emendRenderForPrint(html, css);",
                arguments: ["html": html, "css": css],
                in: nil,
                contentWorld: .page
            )
        } catch {
            throw PDFExport.Failure.renderFailed(error.localizedDescription)
        }
    }

    /// 3. Paginate to PDF. NSPrintOperation.run() (synchronous) deadlocks here: it
    /// blocks the main thread while WebKit's print path needs the main run loop to
    /// deliver web-content-process IPC. The asynchronous runModal(for:…) variant
    /// runs without blocking; the save disposition writes the file, and the did-run
    /// callback resumes the continuation.
    private func paginate(_ webView: WKWebView, with printInfo: NSPrintInfo) async throws {
        guard let window else { throw PDFExport.Failure.printFailed }
        let operation = webView.printOperation(with: printInfo)
        operation.showsPrintPanel = false
        operation.showsProgressPanel = false
        let ok = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<
            Bool,
            Error
        >) in
            printContinuation = continuation
            DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
                guard let self, let pending = printContinuation else { return }
                printContinuation = nil
                pending.resume(throwing: PDFExport.Failure.printFailed)
            }
            operation.runModal(
                for: window,
                delegate: self,
                didRun: #selector(printOperationDidRun(_:success:contextInfo:)),
                contextInfo: nil
            )
        }
        guard ok else { throw PDFExport.Failure.printFailed }
    }

    /// Resume the print step once `runModal(for:…)` completes the save job.
    @objc private func printOperationDidRun(
        _: NSPrintOperation,
        success: Bool,
        contextInfo _: UnsafeMutableRawPointer?
    ) {
        printContinuation?.resume(returning: success)
        printContinuation = nil
    }

    private func makeWebView(size: NSSize) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let frame = NSRect(origin: .zero, size: size)
        let webView = WKWebView(frame: frame, configuration: config)
        webView.navigationDelegate = self
        self.webView = webView

        let window = NSWindow(
            contentRect: NSRect(origin: NSPoint(x: -10000, y: -10000), size: size),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.contentView = webView
        window.orderFrontRegardless()
        self.window = window
        return webView
    }

    private func cleanup() {
        window?.orderOut(nil)
        window?.contentView = nil
        window = nil
        webView?.navigationDelegate = nil
        webView = nil
    }

    // MARK: - Navigation (template load)

    func webView(_: WKWebView, didFinish _: WKNavigation?) {
        loadContinuation?.resume()
        loadContinuation = nil
    }

    func webView(_: WKWebView, didFail _: WKNavigation?, withError error: Error) {
        loadContinuation?.resume(throwing: error)
        loadContinuation = nil
    }

    func webView(
        _: WKWebView,
        didFailProvisionalNavigation _: WKNavigation?,
        withError error: Error
    ) {
        loadContinuation?.resume(throwing: error)
        loadContinuation = nil
    }

    // MARK: - Print configuration

    private static func makePrintInfo(savingTo url: URL) -> NSPrintInfo {
        let info = NSPrintInfo()
        info.topMargin = 36
        info.bottomMargin = 36
        info.leftMargin = 36
        info.rightMargin = 36
        info.horizontalPagination = .fit
        info.verticalPagination = .automatic
        info.isHorizontallyCentered = false
        info.isVerticallyCentered = false
        info.jobDisposition = .save
        info.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL.rawValue] = url
        return info
    }

    private static func contentSize(for info: NSPrintInfo) -> NSSize {
        let paper = info.paperSize
        return NSSize(
            width: max(1, paper.width - info.leftMargin - info.rightMargin),
            height: max(1, paper.height - info.topMargin - info.bottomMargin)
        )
    }
}
