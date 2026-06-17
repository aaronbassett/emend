import AppKit
import EmendCore
import SwiftUI
import WebKit

/// The live Markdown preview pane (US4 · research §C2): a `WKWebView` that renders
/// the core's comrak HTML with bundled, offline Mermaid + KaTeX and syntect-classed
/// code.
///
/// Privacy (SC-008 / FR-035) is enforced in three layers: the template's CSP blocks
/// remote origins, the data store is `.nonPersistent()`, and the navigation
/// delegate cancels every navigation that isn't `file:`/`about:` (clicked external
/// links open in the user's browser instead of loading in-page).
struct PreviewWebView: NSViewRepresentable {
    @ObservedObject var model: PreviewModel
    let scrollSync: ScrollSync

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        // Preview→editor scroll sync: the page posts its top source line here (§C3).
        config.userContentController.add(context.coordinator, name: "emendScroll")
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = context.coordinator
        context.coordinator.webView = webView
        context.coordinator.scrollSync = scrollSync
        // Editor→preview: hand the hub a thin wrapper over the page's scroll entry.
        scrollSync.attachPreview { [weak webView] line in
            webView?.evaluateJavaScript(
                "window.__emendScrollToLine(\(line));",
                completionHandler: nil
            )
        }
        context.coordinator.loadTemplate()
        return webView
    }

    func updateNSView(_: WKWebView, context: Context) {
        context.coordinator.render(html: model.html, css: model.css, version: model.version)
    }

    static func dismantleNSView(_ nsView: WKWebView, coordinator: Coordinator) {
        nsView.configuration.userContentController
            .removeScriptMessageHandler(forName: "emendScroll")
        coordinator.scrollSync?.detachPreview()
    }

    /// Owns the `WKWebView`, loads the bundled offline template, and injects each
    /// render via `window.__emendRender` (bridge.js). Main-actor isolated — all
    /// WebKit access is on the main thread.
    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
        weak var webView: WKWebView?
        var scrollSync: ScrollSync?
        private var isLoaded = false
        private var pending: (html: String, css: String)?
        private var lastVersion = -1

        /// Load the vendored offline shell (`Resources/preview/template.html`),
        /// granting read access to the whole `preview/` dir so its relative
        /// `katex/`, `mermaid.min.js`, `theme.css`, and `bridge.js` resolve.
        func loadTemplate() {
            guard let webView,
                  let dir = Bundle.main.url(forResource: "preview", withExtension: nil)
            else { return }
            let template = dir.appendingPathComponent("template.html")
            guard FileManager.default.fileExists(atPath: template.path) else { return }
            webView.loadFileURL(template, allowingReadAccessTo: dir)
        }

        /// Inject a new render (skipping unchanged versions). Queues until the
        /// template has finished loading, then flushes.
        func render(html: String, css: String, version: Int) {
            guard version != lastVersion else { return }
            lastVersion = version
            pending = (html, css)
            flush()
        }

        private func flush() {
            guard isLoaded, let webView, let payload = pending else { return }
            pending = nil
            // Pass html/css as call arguments (not string interpolation): WebKit
            // does the escaping, so arbitrary document content — including lone
            // surrogates that would defeat a JSON round-trip — renders safely.
            Task { @MainActor in
                _ = try? await webView.callAsyncJavaScript(
                    "window.__emendRender(html, css);",
                    arguments: ["html": payload.html, "css": payload.css],
                    in: nil,
                    contentWorld: .page
                )
            }
        }

        func webView(_: WKWebView, didFinish _: WKNavigation?) {
            isLoaded = true
            flush()
        }

        func webView(
            _: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard let url = navigationAction.request.url else {
                decisionHandler(.cancel)
                return
            }
            let isLocal = url.isFileURL || url.scheme == "about"
            if isLocal {
                decisionHandler(.allow)
            } else {
                // Block remote loads (SC-008); send a clicked link to the browser.
                if navigationAction.navigationType == .linkActivated {
                    NSWorkspace.shared.open(url)
                }
                decisionHandler(.cancel)
            }
        }

        /// Preview→editor scroll sync: the page posts `{ line }` as it scrolls.
        func userContentController(
            _: WKUserContentController,
            didReceive message: WKScriptMessage
        ) {
            guard message.name == "emendScroll",
                  let body = message.body as? [String: Any] else { return }
            let line: Int
            if let value = body["line"] as? Int {
                line = value
            } else if let value = body["line"] as? Double {
                line = Int(value)
            } else {
                return
            }
            scrollSync?.previewScrolled(toLine: line)
        }
    }
}
