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

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = context.coordinator
        context.coordinator.webView = webView
        context.coordinator.loadTemplate()
        return webView
    }

    func updateNSView(_: WKWebView, context: Context) {
        context.coordinator.render(html: model.html, css: model.themeCSS, version: model.version)
    }

    /// Owns the `WKWebView`, loads the bundled offline template, and injects each
    /// render via `window.__emendRender` (bridge.js). Main-actor isolated — all
    /// WebKit access is on the main thread.
    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        weak var webView: WKWebView?
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
            let js = "window.__emendRender(\(jsLiteral(payload.html)), \(jsLiteral(payload.css)));"
            webView.evaluateJavaScript(js, completionHandler: nil)
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

        /// Encode a Swift string as a safe JS string literal (quotes + escapes via
        /// JSON), so arbitrary document content can't break out of the call.
        private func jsLiteral(_ string: String) -> String {
            guard let data = try? JSONSerialization.data(withJSONObject: [string]),
                  let array = String(data: data, encoding: .utf8)
            else { return "\"\"" }
            return String(array.dropFirst().dropLast())
        }
    }
}
