import AppKit
import EmendCore
import SwiftUI

/// SwiftUI wrapper over an AppKit **TextKit 2** `NSTextView` for live Markdown
/// editing (research §A1/§A3/§C1).
///
/// Swift owns the canonical buffer. Each edit is pushed to the Rust core as a
/// tiny UTF-16 delta (synchronous, non-blocking), the buffer is re-attributed
/// from the core's highlight spans (markers dimmed, bold/italic/headings inline),
/// and a debounced atomic autosave is armed. The core's `OpenDocHandle` already
/// holds the same text (its tolerant read matches `readFileAt`), so the buffers
/// start in sync and stay in sync delta-for-delta.
struct MarkdownEditorView: NSViewRepresentable {
    let handle: OpenDocHandle
    let initialText: String
    let isReadOnly: Bool
    let autosave: AutosaveController
    /// The live preview's scroll-sync hub (US4). `nil` when no preview is wired.
    var scrollSync: ScrollSync?
    /// Whether this editor backs the active tab — only the active editor registers
    /// as the scroll-sync source/target.
    var isActive = true

    func makeCoordinator() -> EditorCoordinator {
        EditorCoordinator(handle: handle, autosave: autosave, isReadOnly: isReadOnly)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder

        let textView = Self.makeTextView(baseFont: context.coordinator.baseFont)
        textView.isEditable = !isReadOnly

        // Load the initial text BEFORE wiring the storage delegate, so the load is
        // not mis-read as a user edit (the core already has this text).
        textView.textStorage?.setAttributedString(
            NSAttributedString(string: initialText, attributes: context.coordinator.baseAttributes)
        )
        textView.textStorage?.delegate = context.coordinator
        context.coordinator.attach(textView)
        context.coordinator.scrollSync = scrollSync
        context.coordinator.observeScrolling(in: scrollView)
        context.coordinator.reattribute()

        scrollView.documentView = textView
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.scrollSync = scrollSync
        guard let textView = scrollView.documentView as? NSTextView else { return }
        // Register only the active tab's editor as the scroll-sync source/target,
        // and explicitly detach when it goes inactive so a tab switch can't leave
        // the hub pointing at the previously-active editor (SwiftUI doesn't order
        // the old/new editors' updateNSView calls).
        if isActive {
            scrollSync?.attachEditor(scrollView: scrollView, textView: textView)
        } else {
            scrollSync?.detachEditor(textView)
        }
    }

    /// Build a TextKit 2 `MarkdownTextView` (research §C1): an explicit
    /// `NSTextContentStorage`/`NSTextLayoutManager` stack keeps incremental layout
    /// while letting us use our `NSTextView` subclass for list/formatting keys.
    private static func makeTextView(baseFont: NSFont) -> MarkdownTextView {
        let contentStorage = NSTextContentStorage()
        let layoutManager = NSTextLayoutManager()
        contentStorage.addTextLayoutManager(layoutManager)

        let maxDimension = CGFloat.greatestFiniteMagnitude
        let container = NSTextContainer(size: NSSize(width: 0, height: maxDimension))
        container.widthTracksTextView = true
        layoutManager.textContainer = container

        let textView = MarkdownTextView(frame: .zero, textContainer: container)
        textView.isRichText = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.font = baseFont
        textView.textContainerInset = NSSize(width: 24, height: 24)
        textView.minSize = NSSize(width: 0, height: 0)
        textView.maxSize = NSSize(width: maxDimension, height: maxDimension)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        return textView
    }
}

/// `NSTextView` subclass that turns list/formatting key input into Markdown edits
/// via the pure `SmartLists`/`FormattingCommands` transforms. Edits go through
/// `shouldChangeText`/`didChangeText`, so they register undo and reach the Rust
/// core through the coordinator's storage delegate exactly like typed text.
final class MarkdownTextView: NSTextView {
    override func insertNewline(_ sender: Any?) {
        if handleNewline() { return }
        super.insertNewline(sender)
    }

    override func insertTab(_ sender: Any?) {
        if applyListEdit({ SmartLists.indent(in: $0, selection: $1) }) { return }
        super.insertTab(sender)
    }

    override func insertBacktab(_ sender: Any?) {
        if applyListEdit({ SmartLists.outdent(in: $0, selection: $1) }) { return }
        super.insertBacktab(sender)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if handleFormattingShortcut(event) { return true }
        return super.performKeyEquivalent(with: event)
    }

    // MARK: - List editing

    /// Continue/terminate the list on Return, then renumber the affected ordered
    /// block (grouped as a single undo step). Returns `false` for non-list lines.
    private func handleNewline() -> Bool {
        guard isEditable, let storage = textStorage else { return false }
        guard let edit = SmartLists.newline(
            in: storage.string as NSString,
            selection: selectedRange()
        )
        else { return false }
        undoManager?.beginUndoGrouping()
        apply(range: edit.range, replacement: edit.replacement, selection: edit.selectionAfter)
        renumberCurrentBlock()
        undoManager?.endUndoGrouping()
        return true
    }

    /// Renumber the ordered block at the caret (no-op for bullet lists). Run after
    /// inserting a new ordered item so the tail stays sequential.
    private func renumberCurrentBlock() {
        guard let storage = textStorage,
              let edit = SmartLists.renumber(
                  in: storage.string as NSString,
                  selection: selectedRange()
              )
        else { return }
        apply(range: edit.range, replacement: edit.replacement, selection: edit.selectionAfter)
    }

    private func applyListEdit(_ transform: (NSString, NSRange) -> SmartLists.Edit?) -> Bool {
        guard isEditable, let storage = textStorage else { return false }
        guard let edit = transform(storage.string as NSString, selectedRange())
        else { return false }
        apply(range: edit.range, replacement: edit.replacement, selection: edit.selectionAfter)
        return true
    }

    // MARK: - Formatting shortcuts (⌘B / ⌘I / ⌘K / ⌘⇧T)

    private func handleFormattingShortcut(_ event: NSEvent) -> Bool {
        guard isEditable, let storage = textStorage else { return false }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard let key = event.charactersIgnoringModifiers?.lowercased() else { return false }
        let command = flags == [.command]
        let commandShift = flags == [.command, .shift]
        let text = storage.string as NSString
        let selection = selectedRange()
        let edit: FormattingCommands.Edit
        switch key {
        case "b" where command: edit = FormattingCommands.bold(in: text, selection: selection)
        case "i" where command: edit = FormattingCommands.italic(in: text, selection: selection)
        case "k" where command: edit = FormattingCommands.link(in: text, selection: selection)
        case "t" where commandShift: edit = FormattingCommands.task(in: text, selection: selection)
        default: return false
        }
        apply(range: edit.range, replacement: edit.replacement, selection: edit.selectionAfter)
        return true
    }

    // MARK: - Apply

    private func apply(range: NSRange, replacement: String, selection: NSRange) {
        guard shouldChangeText(in: range, replacementString: replacement) else { return }
        textStorage?.replaceCharacters(in: range, with: replacement)
        didChangeText()
        setSelectedRange(selection)
    }
}

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
        storage.endEditing()
        isApplyingAttributes = false
    }
}
