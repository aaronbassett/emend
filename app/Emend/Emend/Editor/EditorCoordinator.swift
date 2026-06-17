import AppKit
import EmendCore

/// Owns the per-document editing loop: delta extraction → core `pushEdit` →
/// re-attribution → autosave. `NSTextStorageDelegate` so it sees the precise
/// edited range and length change. Main-actor isolated — all text-storage access
/// happens on the main thread.
@MainActor
final class EditorCoordinator: NSObject, NSTextStorageDelegate {
    let baseFont = NSFont.systemFont(ofSize: 14)
    private(set) lazy var baseAttributes: [NSAttributedString.Key: Any] = [
        .font: baseFont,
        .foregroundColor: NSColor.textColor
    ]

    private let handle: OpenDocHandle
    private let autosave: AutosaveController
    private let isReadOnly: Bool
    private weak var textView: NSTextView?

    /// The live preview's scroll-sync hub (US4); `nil` when no preview is wired.
    var scrollSync: ScrollSync?
    /// Wiki-link services (US5): the workspace index, this note's resolution
    /// origin, and a tab-opener. Set by the representable.
    var workspace: WorkspaceHandle?
    var notePath = ""
    var onOpenLink: ((URL) -> Void)?

    /// Guards re-entrancy: our own attribute writes also fire `didProcessEditing`.
    private var isApplyingAttributes = false
    private var reattributeScheduled = false

    init(handle: OpenDocHandle, autosave: AutosaveController, isReadOnly: Bool) {
        self.handle = handle
        self.autosave = autosave
        self.isReadOnly = isReadOnly
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    func attach(_ textView: NSTextView) {
        self.textView = textView
    }

    /// Observe the scroll view's clip bounds so editor scrolls drive the preview
    /// (US4 · research §C3). The clip view posts on the main thread during scroll.
    func observeScrolling(in scrollView: NSScrollView) {
        let clip = scrollView.contentView
        clip.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(viewportDidScroll),
            name: NSView.boundsDidChangeNotification,
            object: clip
        )
    }

    /// Clip bounds moved (a scroll): forward the editor's top line to the preview.
    /// Posted on the main thread during scrolling.
    @objc private func viewportDidScroll() {
        guard let textView else { return }
        scrollSync?.editorScrolled(from: textView)
    }

    /// NSTextStorageDelegate is not @MainActor in the SDK, but its callbacks always
    /// fire on the main thread during editing — bounce into the main actor, passing
    /// only Sendable values (the non-Sendable storage is re-accessed via textView).
    nonisolated func textStorage(
        _: NSTextStorage,
        didProcessEditing editedMask: NSTextStorageEditActions,
        range editedRange: NSRange,
        changeInLength delta: Int
    ) {
        let isCharacterEdit = editedMask.contains(.editedCharacters)
        MainActor.assumeIsolated {
            processEdit(isCharacterEdit: isCharacterEdit, range: editedRange, changeInLength: delta)
        }
    }

    private func processEdit(
        isCharacterEdit: Bool,
        range editedRange: NSRange,
        changeInLength delta: Int
    ) {
        guard isCharacterEdit, !isApplyingAttributes, !isReadOnly else { return }
        guard let storage = textView?.textStorage else { return }
        let oldLength = editedRange.length - delta
        guard oldLength >= 0, editedRange.location >= 0,
              NSMaxRange(editedRange) <= storage.length else { return }

        // Push the UTF-16 delta to the core synchronously (a non-blocking in-memory
        // splice; it does NOT touch this NSTextStorage). A thrown error means our
        // offset mapping is wrong (a UTF-16-contract bug) — recover, don't crash.
        let replacement = storage.attributedSubstring(from: editedRange).string
        let range = U16Range(start: UInt32(editedRange.location), len: UInt32(oldLength))
        do {
            try handle.pushEdit(range: range, replacement: replacement)
        } catch {
            return
        }
        autosave.noteEdit()
        scheduleReattribute()
    }

    /// Coalesced re-attribution: re-query the core's spans for the whole document
    /// and re-apply display attributes. Runs after the current edit cycle to avoid
    /// mutating the storage mid-`processEditing`.
    ///
    /// NOTE: whole-document attribution is correct and simple for the MVP; viewport
    /// windowing for very large docs is a tracked perf optimization (Principle IV /
    /// Polish T131) — the core highlighter is already incremental.
    private func scheduleReattribute() {
        guard !reattributeScheduled else { return }
        reattributeScheduled = true
        Task { @MainActor [weak self] in
            self?.reattributeScheduled = false
            self?.reattribute()
        }
    }

    func reattribute() {
        guard let storage = textView?.textStorage else { return }
        let length = storage.length
        guard length >= 0 else { return }
        let spans: [StyleSpan]
        do {
            spans = try handle.highlightSpans(viewport: U16Range(start: 0, len: UInt32(length)))
        } catch {
            return // highlighting is advisory; leave the text readable on failure
        }
        isApplyingAttributes = true
        storage.beginEditing()
        storage.setAttributes(baseAttributes, range: NSRange(location: 0, length: length))
        SyntaxAttributing.apply(spans: spans, to: storage, baseFont: baseFont)
        applyUnresolvedLinkStyling(in: storage)
        storage.endEditing()
        isApplyingAttributes = false
    }

    // MARK: - Wiki links (US5)

    /// `[[` autocomplete candidates (note stems) for `prefix`, via the US2 index.
    func wikiSuggestions(prefix: String) -> [String]? {
        guard let workspace else { return nil }
        let hits = (try? workspace.wikilinkSuggestions(prefix: prefix, limit: 20)) ?? []
        return hits.isEmpty ? nil : hits.map(\.name)
    }

    /// Resolve `rawTarget` against this note and open the matched file in a tab.
    /// `try?` flattens the throwing `String?` so the guard unwraps to a real path;
    /// an unresolved link (or error) simply does nothing.
    func openWikiLink(rawTarget: String) {
        guard let workspace, let onOpenLink,
              let path = try? workspace.resolveWikilink(fromNote: notePath, rawTarget: rawTarget)
        else { return }
        onOpenLink(URL(fileURLWithPath: path))
    }

    /// Mark `[[links]]` that don't resolve in the index with a distinct style so
    /// broken links are visible (US5). Skipped when no workspace is wired. Runs
    /// inside the `reattribute` edit batch (attribute-only — never a char edit).
    private func applyUnresolvedLinkStyling(in storage: NSTextStorage) {
        guard let workspace, !notePath.isEmpty else { return }
        let text = storage.string as NSString
        let underline = NSUnderlineStyle.single.rawValue | NSUnderlineStyle.patternDot.rawValue
        for link in WikiLink.allLinks(in: text) where NSMaxRange(link.span) <= storage.length {
            let resolved = (try? workspace.resolveWikilink(
                fromNote: notePath,
                rawTarget: link.raw
            )) ?? nil
            guard resolved == nil else { continue }
            storage.addAttribute(.foregroundColor, value: NSColor.systemRed, range: link.span)
            storage.addAttribute(.underlineStyle, value: underline, range: link.span)
        }
    }
}
