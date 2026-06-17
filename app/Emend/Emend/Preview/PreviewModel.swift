import Combine
import EmendCore
import Foundation

/// Drives the live Markdown preview (US4 · FR-022/FR-025, research §B1).
///
/// Holds the active document's handle and re-renders its preview HTML — via the
/// core's `renderPreviewHtml` (comrak + syntect) — **off the main thread**
/// (NFR-001), debounced ~150 ms off the editor's edit signal so continuous typing
/// doesn't re-render per keystroke. The syntect theme CSS is core-owned and fixed
/// for the session, fetched once.
@MainActor
final class PreviewModel: ObservableObject {
    /// Rendered HTML body fragment (injected into the preview WebView's
    /// `#emend-content`). Empty when no document is active.
    @Published private(set) var html = ""
    /// Bumps on every successful render so the WebView re-injects even when the
    /// HTML string is unchanged (e.g. a re-render that produced identical output).
    @Published private(set) var version = 0

    /// The syntect classed-code stylesheet (core-owned; constant for the session).
    let themeCSS: String = previewThemeCss()

    /// Whether the preview pane is visible — renders are skipped while hidden to
    /// avoid wasted work, and a full refresh runs when it reappears.
    var isVisible = false {
        didSet {
            guard isVisible, !oldValue else { return }
            scheduleRefresh(immediate: true)
        }
    }

    private var handle: OpenDocHandle?
    private var refreshTask: Task<Void, Never>?
    private let debounce: Duration = .milliseconds(150)

    /// Point the preview at a new active document (or `nil` when none is open).
    func setActiveDocument(_ handle: OpenDocHandle?) {
        self.handle = handle
        scheduleRefresh(immediate: true)
    }

    /// (Re)render the active document. Coalesces rapid calls: each cancels the
    /// pending one and re-arms the debounce, so a burst of keystrokes yields a
    /// single render ~150 ms after the last.
    func scheduleRefresh(immediate: Bool = false) {
        refreshTask?.cancel()
        guard isVisible, let handle else {
            if handle == nil { html = "" }
            return
        }
        refreshTask = Task { [weak self, debounce] in
            if !immediate {
                try? await Task.sleep(for: debounce)
            }
            if Task.isCancelled { return }
            let rendered = await Self.render(handle)
            if Task.isCancelled { return }
            guard let self, let rendered else { return }
            html = rendered
            version &+= 1
        }
    }

    /// Render off the main actor — `renderPreviewHtml` is whole-document comrak +
    /// syntect work that can exceed a frame on a large doc (NFR-001).
    private static func render(_ handle: OpenDocHandle) async -> String? {
        await Task.detached(priority: .userInitiated) {
            try? handle.renderPreviewHtml()
        }.value
    }
}
