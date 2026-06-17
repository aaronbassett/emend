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
    /// Wiki-link services (US5): the workspace index for `[[` autocomplete /
    /// resolution, this note's path (the resolution origin), and a tab-opener.
    var workspace: WorkspaceHandle?
    var notePath = ""
    var onOpenLink: ((URL) -> Void)?
    /// Typography (US7): font/size/spacing applied to the editor.
    var typography: TypographySettings = TypographyModel.defaultSettings

    func makeCoordinator() -> EditorCoordinator {
        EditorCoordinator(handle: handle, autosave: autosave, isReadOnly: isReadOnly)
    }

    func makeNSView(context: Context) -> NSScrollView {
        // Set typography first — `baseFont`/`baseAttributes` derive from it.
        context.coordinator.typography = typography

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
        textView.coordinator = context.coordinator
        context.coordinator.attach(textView)
        context.coordinator.scrollSync = scrollSync
        context.coordinator.workspace = workspace
        context.coordinator.notePath = notePath
        context.coordinator.onOpenLink = onOpenLink
        context.coordinator.observeScrolling(in: scrollView)
        context.coordinator.reattribute()

        scrollView.documentView = textView
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.scrollSync = scrollSync
        context.coordinator.workspace = workspace
        context.coordinator.notePath = notePath
        context.coordinator.onOpenLink = onOpenLink
        if typography != context.coordinator.typography {
            context.coordinator.applyTypography(typography)
        }
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
        // Stable identifier for UI automation (T134); the editor is the primary
        // surface a test drives.
        textView.setAccessibilityIdentifier("editor.textView")
        return textView
    }
}

/// `NSTextView` subclass that turns list/formatting key input into Markdown edits
/// via the pure `SmartLists`/`FormattingCommands` transforms. Edits go through
/// `shouldChangeText`/`didChangeText`, so they register undo and reach the Rust
/// core through the coordinator's storage delegate exactly like typed text.
final class MarkdownTextView: NSTextView {
    /// Set by the representable so list/formatting keys and link/checkbox clicks
    /// can reach the core through the coordinator (US5).
    weak var coordinator: EditorCoordinator?
    /// Re-entrancy guard: accepting a completion inserts text, which must not
    /// re-trigger the completion UI.
    private var isCompleting = false

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

    @discardableResult
    private func apply(range: NSRange, replacement: String, selection: NSRange) -> Bool {
        guard shouldChangeText(in: range, replacementString: replacement) else { return false }
        textStorage?.replaceCharacters(in: range, with: replacement)
        didChangeText()
        setSelectedRange(selection)
        return true
    }

    // MARK: - Wiki-link autocomplete + click navigation / checkboxes (US5)

    /// After typing inside an open `[[…`, surface the native completion list.
    override func insertText(_ string: Any, replacementRange: NSRange) {
        super.insertText(string, replacementRange: replacementRange)
        guard !isCompleting, isEditable, let storage = textStorage else { return }
        let caret = selectedRange().location
        let inOpenLink = WikiLink.partialRange(in: storage.string as NSString, caret: caret) != nil
        if inOpenLink { complete(nil) }
    }

    /// Restrict native completion to the open `[[…` target; disabled elsewhere.
    override var rangeForUserCompletion: NSRange {
        guard let storage = textStorage,
              let range = WikiLink.partialRange(
                  in: storage.string as NSString, caret: selectedRange().location
              )
        else { return NSRange(location: NSNotFound, length: 0) }
        return range
    }

    override func completions(
        forPartialWordRange charRange: NSRange,
        indexOfSelectedItem index: UnsafeMutablePointer<Int>?
    ) -> [String]? {
        guard let storage = textStorage, charRange.location != NSNotFound,
              NSMaxRange(charRange) <= storage.length else { return nil }
        index?.pointee = 0
        let prefix = (storage.string as NSString).substring(with: charRange)
        return coordinator?.wikiSuggestions(prefix: prefix)
    }

    override func insertCompletion(
        _ word: String, forPartialWordRange charRange: NSRange, movement: Int, isFinal: Bool
    ) {
        isCompleting = true
        defer { isCompleting = false }
        super.insertCompletion(
            word,
            forPartialWordRange: charRange,
            movement: movement,
            isFinal: isFinal
        )
        guard isFinal, let storage = textStorage else { return }
        let caret = selectedRange().location
        let text = storage.string as NSString
        let closeBracket = UInt16(UInt8(ascii: "]"))
        let alreadyClosed = caret + 1 < text.length
            && text.character(at: caret) == closeBracket
            && text.character(at: caret + 1) == closeBracket
        if !alreadyClosed {
            insertText("]]", replacementRange: NSRange(location: caret, length: 0))
        }
    }

    /// Toggle a task checkbox on click; ⌘-click a `[[wiki link]]` to open the note.
    override func mouseDown(with event: NSEvent) {
        guard let storage = textStorage else { return super.mouseDown(with: event) }
        let point = convert(event.locationInWindow, from: nil)
        let index = characterIndexForInsertion(at: point)
        let text = storage.string as NSString
        if isEditable, toggleCheckboxIfClicked(at: index, in: text) { return }
        let isCommandClick = event.modifierFlags.contains(.command)
        if isCommandClick, let link = WikiLink.enclosingLink(in: text, at: index) {
            coordinator?.openWikiLink(rawTarget: link.raw)
            return
        }
        super.mouseDown(with: event)
    }

    /// If `index` lands on a task checkbox, flip it through the Edit path (Swift
    /// owns the buffer; the core sees the delta) and return true; else false.
    private func toggleCheckboxIfClicked(at index: Int, in text: NSString) -> Bool {
        guard let box = TaskCheckbox.checkboxRange(in: text, atLineContaining: index),
              index >= box.location, index <= NSMaxRange(box),
              let edit = TaskCheckbox.toggleEdit(in: text, atLineContaining: index)
        else { return false }
        apply(
            range: edit.range,
            replacement: edit.replacement,
            selection: NSRange(location: index, length: 0)
        )
        return true
    }

    // MARK: - Image drag-drop (US5)

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        if isEditable, !ImageDrop.imageFileURLs(in: sender.draggingPasteboard).isEmpty {
            return .copy
        }
        return super.draggingEntered(sender)
    }

    /// Store dropped images as attachments and insert Markdown image refs at the
    /// drop point (through the Edit path). Non-image drops fall back to NSTextView.
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let urls = ImageDrop.imageFileURLs(in: sender.draggingPasteboard)
        guard isEditable, !urls.isEmpty, let coordinator else {
            return super.performDragOperation(sender)
        }
        let refs = coordinator.storeImageAttachments(urls)
        guard !refs.isEmpty else { return false }
        let markdown = refs.map { ImageDrop.markdown(forImageRef: $0) }.joined(separator: "\n")
        let point = convert(sender.draggingLocation, from: nil)
        let dropIndex = characterIndexForInsertion(at: point)
        let caretAfter = dropIndex + (markdown as NSString).length
        let inserted = apply(
            range: NSRange(location: dropIndex, length: 0),
            replacement: markdown,
            selection: NSRange(location: caretAfter, length: 0)
        )
        // If the insert was vetoed, don't leave the just-written files orphaned.
        if !inserted { coordinator.removeAttachments(refs) }
        return inserted
    }
}
