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

    func makeCoordinator() -> EditorCoordinator {
        EditorCoordinator(handle: handle, autosave: autosave, isReadOnly: isReadOnly)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        guard let textView = scrollView.documentView as? NSTextView else { return scrollView }

        textView.isEditable = !isReadOnly
        textView.isRichText = false
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.font = context.coordinator.baseFont
        textView.textContainerInset = NSSize(width: 24, height: 24)

        // Load the initial text BEFORE wiring the storage delegate, so the load is
        // not mis-read as a user edit (the core already has this text).
        textView.textStorage?.setAttributedString(
            NSAttributedString(string: initialText, attributes: context.coordinator.baseAttributes)
        )
        textView.textStorage?.delegate = context.coordinator
        context.coordinator.attach(textView)
        context.coordinator.reattribute()
        return scrollView
    }

    func updateNSView(_: NSScrollView, context _: Context) {}
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

    /// Guards re-entrancy: our own attribute writes also fire `didProcessEditing`.
    private var isApplyingAttributes = false
    private var reattributeScheduled = false

    init(handle: OpenDocHandle, autosave: AutosaveController, isReadOnly: Bool) {
        self.handle = handle
        self.autosave = autosave
        self.isReadOnly = isReadOnly
    }

    func attach(_ textView: NSTextView) {
        self.textView = textView
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
