import AppKit
import Foundation

/// Bidirectional source ↔ preview scroll sync (US4 · FR-024, research §C3).
///
/// Both sides are keyed on **1-based source line numbers**. comrak annotates each
/// top-level block with its starting line (`data-line`, emitted by the core's
/// preview renderer); bridge.js builds an anchor table from those. Editor→preview
/// maps the top visible character to a line and calls `__emendScrollToLine`
/// (interpolating between anchors); preview→editor receives a throttled top line
/// from the page and scrolls the text view's matching line to the top.
///
/// The hub is shared between the active editor and the preview WebView. A short
/// per-side mute guards the feedback loop: when we drive one side we ignore the
/// echo the other side reports back (research §C3 "ignore-incoming flag/debounce").
@MainActor
final class ScrollSync: ObservableObject {
    private weak var editorScroll: NSScrollView?
    private weak var editorTextView: NSTextView?
    private var scrollPreviewToLine: ((Int) -> Void)?

    private var muteEditor = false
    private var mutePreview = false
    private var muteEditorTask: Task<Void, Never>?
    private var mutePreviewTask: Task<Void, Never>?
    private let muteWindow: Duration = .milliseconds(160)

    // MARK: - Links

    /// Register the active document's editor as the sync source/target. Switching
    /// tabs re-registers; inactive editors' scroll events are dropped (their text
    /// view won't match `editorTextView`).
    func attachEditor(scrollView: NSScrollView, textView: NSTextView) {
        editorScroll = scrollView
        editorTextView = textView
    }

    func detachEditor(_ textView: NSTextView) {
        guard editorTextView === textView else { return }
        editorScroll = nil
        editorTextView = nil
    }

    /// Register the preview WebView's "scroll to line" entry point (a thin wrapper
    /// over `window.__emendScrollToLine`).
    func attachPreview(scrollToLine: @escaping (Int) -> Void) {
        scrollPreviewToLine = scrollToLine
    }

    func detachPreview() {
        scrollPreviewToLine = nil
    }

    // MARK: - Editor → preview

    /// The active editor's viewport moved: map its top visible line and drive the
    /// preview. Ignored while muted or when the caller isn't the active editor.
    func editorScrolled(from textView: NSTextView) {
        guard !muteEditor, textView === editorTextView else { return }
        guard let scrollPreviewToLine, let line = currentEditorTopLine() else { return }
        mutePreviewBriefly()
        scrollPreviewToLine(line)
    }

    // MARK: - Preview → editor

    /// The preview reported its top visible source line: scroll the editor to match.
    func previewScrolled(toLine line: Int) {
        guard !mutePreview, let textView = editorTextView else { return }
        muteEditorBriefly()
        scrollEditor(textView, toLine: max(1, line))
    }

    // MARK: - Feedback-loop guard

    private func mutePreviewBriefly() {
        mutePreview = true
        mutePreviewTask?.cancel()
        mutePreviewTask = Task { [weak self, muteWindow] in
            try? await Task.sleep(for: muteWindow)
            self?.mutePreview = false
        }
    }

    private func muteEditorBriefly() {
        muteEditor = true
        muteEditorTask?.cancel()
        muteEditorTask = Task { [weak self, muteWindow] in
            try? await Task.sleep(for: muteWindow)
            self?.muteEditor = false
        }
    }
}

// MARK: - Editor line ↔ geometry (TextKit 2)

extension ScrollSync {
    /// The 1-based source line at the top of the editor's visible rect.
    private func currentEditorTopLine() -> Int? {
        guard let textView = editorTextView, let storage = textView.textStorage else { return nil }
        let topY = textView.visibleRect.minY
        let point = NSPoint(x: textView.textContainerInset.width + 1, y: topY + 1)
        let charIndex = textView.characterIndexForInsertion(at: point)
        let nsString = storage.string as NSString
        let clamped = max(0, min(charIndex, nsString.length))
        return lineNumber(in: nsString, atUTF16Index: clamped)
    }

    /// Scroll the editor so `line`'s first character sits at the top of the viewport.
    private func scrollEditor(_ textView: NSTextView, toLine line: Int) {
        guard let storage = textView.textStorage else { return }
        let nsString = storage.string as NSString
        let charIndex = utf16Index(in: nsString, ofLineStart: line)
        guard let viewY = topY(in: textView, forCharacterIndex: charIndex),
              let clip = textView.enclosingScrollView?.contentView else { return }
        let maxY = max(0, textView.frame.height - clip.bounds.height)
        let target = NSPoint(x: clip.bounds.origin.x, y: min(max(0, viewY), maxY))
        clip.scroll(to: target)
        textView.enclosingScrollView?.reflectScrolledClipView(clip)
    }

    /// Count newlines before `index` (1-based line number). comrak source lines are
    /// `\n`-delimited and 1-based, so this matches the `data-line` anchors.
    private func lineNumber(in nsString: NSString, atUTF16Index index: Int) -> Int {
        guard index > 0 else { return 1 }
        var line = 1
        for scalar in (nsString.substring(to: index)).unicodeScalars where scalar == "\n" {
            line += 1
        }
        return line
    }

    /// The UTF-16 offset of `line`'s first character (clamped to the document end).
    private func utf16Index(in nsString: NSString, ofLineStart line: Int) -> Int {
        guard line > 1 else { return 0 }
        var index = 0
        var current = 1
        while current < line {
            let searchRange = NSRange(location: index, length: nsString.length - index)
            let newline = nsString.range(of: "\n", options: [], range: searchRange)
            if newline.location == NSNotFound { return nsString.length }
            index = newline.location + 1
            current += 1
        }
        return index
    }

    /// The y-offset (text-view coordinates) of the layout fragment containing
    /// `index`, via the TextKit 2 layout manager.
    private func topY(in textView: NSTextView, forCharacterIndex index: Int) -> CGFloat? {
        guard let layoutManager = textView.textLayoutManager,
              let contentManager = textView.textContentStorage else { return nil }
        guard let location = contentManager.location(
            contentManager.documentRange.location, offsetBy: index
        ) else { return nil }
        layoutManager.ensureLayout(for: NSTextRange(location: location))
        guard let fragment = layoutManager.textLayoutFragment(for: location) else { return nil }
        return fragment.layoutFragmentFrame.minY + textView.textContainerOrigin.y
    }
}
