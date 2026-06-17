//! T039 — FFI projection of the open-document session + editor highlighting
//! (US1 · FFI contract §3; FR-009/009a, FR-010..015, FR-027a, FR-031a).
//!
//! Thin UniFFI shim over [`emend_core::document::Document`] (the shadow rope)
//! and [`emend_core::parse::highlight::Highlighter`] (the incremental
//! tree-sitter editor highlighter). All real logic lives in the core; this
//! module only:
//!
//! 1. **Projects value types** the core cannot derive `uniffi` on (Constitution
//!    V keeps `emend-core` `uniffi`-free): [`U16Range`], [`StyleClass`],
//!    [`StyleSpan`]. Each gets a `From` conversion to/from its core twin. The
//!    [`StyleClass`] and [`U16Range`] `From`s are **exhaustive — no wildcard
//!    arm** — so a new core variant breaks compilation here until mirrored,
//!    the same closed-projection discipline as [`crate::error::FfiError`].
//!
//! 2. **Wraps the session** in [`OpenDocHandle`], a `#[derive(uniffi::Object)]`
//!    handed to Swift as `Arc<Self>`. Interior mutability is a
//!    `Mutex<DocSession>`; the contract's methods take `&self` and the mutex
//!    gives the `&mut` for edits.
//!
//! 3. **Exports** `open_document` / `push_edit` / `highlight_spans` / `flush` /
//!    `close` matching the contract's names and threading rules, plus the US4
//!    preview exports `render_preview_html` (a method on the handle) and the
//!    free `preview_theme_css` (the core-owned syntect theme CSS — see its docs
//!    for how it resolves the contract's ambiguous `preview_assets_dir`).
//!
//! ## Design choices (documented per the task brief)
//!
//! - **Methods, not free functions.** The contract sketches free functions
//!   (`push_edit(h, …)`), but methods on the `uniffi::Object` are the idiomatic
//!   UniFFI shape and read naturally on the Swift side (`handle.pushEdit(…)`).
//!   `open_document` stays a free function because it *constructs* the handle.
//!
//! - **`push_edit` / `highlight_spans` return `Result`.** The contract draws
//!   them infallible, but the underlying core calls
//!   ([`Document::push_edit`](emend_core::document::Document::push_edit),
//!   [`Highlighter::apply_edit`](emend_core::parse::highlight::Highlighter::apply_edit),
//!   [`Highlighter::highlight_spans`](emend_core::parse::highlight::Highlighter::highlight_spans))
//!   are fallible: a malformed delta or out-of-bounds viewport (a programming
//!   error from the Swift shim) is worth surfacing as a thrown `FfiError`
//!   rather than swallowing. A rejected edit never corrupts the buffer (the
//!   core validates the whole range *before* mutating), so this is a safe,
//!   strictly-more-informative deviation from the sketch.
//!
//! - **`close` is an explicit `&self` method.** Dropping the last `Arc` frees
//!   the session, so a close is not strictly required; the contract still names
//!   `close_document`, so we expose an explicit, intention-revealing `close()`
//!   that consumes the inner session (running its [`Document::close`]). After
//!   `close()` the handle is inert — further calls return
//!   [`FfiError::Internal`] rather than panicking.

use crate::error::FfiError;
use crate::workspace::WorkspaceHandle;
use emend_core::derived::{
    extract_links, toggle_task, LinkKind as CoreLinkKind, LinkRef as CoreLinkRef,
};
use emend_core::document::Document;
use emend_core::parse::highlight::{
    Highlighter, StyleClass as CoreStyleClass, StyleSpan as CoreStyleSpan,
};
use emend_core::U16Range as CoreU16Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// UTF-16 code-unit text range crossing the FFI boundary (FFI contract §3,
/// research §A2) — mirrors [`emend_core::U16Range`] so it maps 1:1 onto
/// `NSRange` on the Swift side.
///
/// A distinct FFI type (rather than re-deriving on the core type) keeps
/// `emend-core` free of any `uniffi` dependency (Constitution V). The two
/// `From` impls below are the single, exhaustive translation point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct U16Range {
    /// Start offset, in UTF-16 code units.
    pub start: u32,
    /// Length of the range, in UTF-16 code units.
    pub len: u32,
}

impl From<CoreU16Range> for U16Range {
    fn from(r: CoreU16Range) -> Self {
        // Destructure exhaustively so a new field on the core type forces a
        // compile error here rather than silently dropping data.
        let CoreU16Range { start, len } = r;
        Self { start, len }
    }
}

impl From<U16Range> for CoreU16Range {
    fn from(r: U16Range) -> Self {
        let U16Range { start, len } = r;
        Self::new(start, len)
    }
}

/// The visual role the editor should give a run of text — the FFI mirror of
/// [`emend_core::parse::highlight::StyleClass`] (FR-010..015). The Swift
/// attributing layer (T042) turns each variant into display attributes.
///
/// Mirrors the core enum **variant-for-variant**, including
/// [`StyleClass::Heading`]'s `level`. The [`From`] below is exhaustive (no
/// wildcard), so adding a variant to the core enum breaks this match until it
/// is mirrored here — keeping the FFI vocabulary a closed, checked projection
/// (same discipline as [`crate::error::FfiError`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StyleClass {
    /// Dimmable Markdown punctuation (`#`, `*`/`_`, backticks, `>`, brackets…).
    SyntaxMarker,
    /// An ATX/setext heading's text. `level` is 1..=6 when determinable, else
    /// `None`.
    Heading {
        /// Heading level 1..=6, or `None` when not determinable.
        level: Option<u8>,
    },
    /// Strong (bold) inline text — `**…**` / `__…__`.
    Strong,
    /// Emphasised (italic) inline text — `*…*` / `_…_`.
    Emphasis,
    /// An inline code span — `` `code` ``.
    InlineCode,
    /// A fenced or indented code block's content.
    CodeBlock,
    /// A block quote's quoted body — `> …`.
    BlockQuote,
    /// A list item's marker run (`-`, `*`, `+`, `1.`).
    ListMarker,
    /// Link/image text or destination.
    Link,
    /// `==highlighted==` text.
    Highlight,
}

impl From<CoreStyleClass> for StyleClass {
    /// Exhaustive projection — no wildcard arm. A new core variant fails this
    /// match at compile time until mirrored above.
    fn from(class: CoreStyleClass) -> Self {
        match class {
            CoreStyleClass::SyntaxMarker => Self::SyntaxMarker,
            CoreStyleClass::Heading { level } => Self::Heading { level },
            CoreStyleClass::Strong => Self::Strong,
            CoreStyleClass::Emphasis => Self::Emphasis,
            CoreStyleClass::InlineCode => Self::InlineCode,
            CoreStyleClass::CodeBlock => Self::CodeBlock,
            CoreStyleClass::BlockQuote => Self::BlockQuote,
            CoreStyleClass::ListMarker => Self::ListMarker,
            CoreStyleClass::Link => Self::Link,
            CoreStyleClass::Highlight => Self::Highlight,
        }
    }
}

/// A styled run of text plus its visual role — the FFI mirror of
/// [`emend_core::parse::highlight::StyleSpan`]. `range` is UTF-16 (FFI contract
/// §3) so it maps directly onto `NSRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct StyleSpan {
    /// The styled run, in UTF-16 code units.
    pub range: U16Range,
    /// How to style it.
    pub class: StyleClass,
}

impl From<CoreStyleSpan> for StyleSpan {
    fn from(span: CoreStyleSpan) -> Self {
        // Destructure exhaustively (forces a compile error if the core struct
        // grows a field) and reuse the field projections above.
        let CoreStyleSpan { range, class } = span;
        Self {
            range: range.into(),
            class: class.into(),
        }
    }
}

/// Whether a [`LinkRef`] is a navigable link (`[[…]]`) or an inline embed
/// (`![[…]]`) — the FFI mirror of [`emend_core::derived::LinkKind`] (US5 ·
/// FR-019/021). The [`From`] is exhaustive (no wildcard), so a new core variant
/// breaks compilation here until mirrored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkKind {
    /// A `[[wiki link]]` — clickable, navigates to the target note (FR-019).
    Link,
    /// A `![[embed]]` — inlines the target's content in the preview (FR-021).
    Embed,
}

impl From<CoreLinkKind> for LinkKind {
    /// Exhaustive projection — no wildcard arm.
    fn from(kind: CoreLinkKind) -> Self {
        match kind {
            CoreLinkKind::Link => Self::Link,
            CoreLinkKind::Embed => Self::Embed,
        }
    }
}

/// A wiki link / embed found in a document (FFI contract §4 `links`). The FFI
/// mirror of [`emend_core::derived::LinkRef`].
///
/// `range` is UTF-16 (FFI contract §4/§5) so it maps onto `NSRange` for
/// click/navigation; `raw_target` is the target as typed (before any `|` alias),
/// which Swift resolves via [`crate::workspace::WorkspaceHandle::resolve_wikilink`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkRef {
    /// Link vs embed.
    pub kind: LinkKind,
    /// The target as typed (before any `|` alias), e.g. `Launch Plan`.
    pub raw_target: String,
    /// The full `[[…]]` / `![[…]]` token's source range in UTF-16 code units.
    pub range: U16Range,
}

impl From<CoreLinkRef> for LinkRef {
    fn from(link: CoreLinkRef) -> Self {
        // Destructure exhaustively so a new core field forces a compile error.
        let CoreLinkRef {
            kind,
            raw_target,
            range,
        } = link;
        Self {
            kind: kind.into(),
            raw_target,
            range: range.into(),
        }
    }
}

/// The mutable state behind one open document: the shadow [`Document`], its
/// [`Highlighter`] (driven in lock-step by the same `(range, replacement)`
/// deltas), and the on-disk [`PathBuf`] [`flush`](OpenDocHandle::flush) writes
/// back to.
///
/// `None` after [`OpenDocHandle::close`] consumes the session, so the handle
/// becomes inert rather than panicking on use-after-close.
struct DocSession {
    /// The note's path on disk, captured at [`open_document`] time so a later
    /// [`flush`](OpenDocHandle::flush) knows where to write the buffer back.
    path: PathBuf,
    doc: Document,
    highlighter: Highlighter,
}

/// Open-document handle exported to Swift (FFI contract §3).
///
/// Handed to Swift as `Arc<Self>`; methods take `&self` and reach the inner
/// `&mut` through the [`Mutex`]. On the **hot path** ([`Self::push_edit`],
/// [`Self::highlight_spans`]) the lock is held only for the in-memory splice /
/// tree walk — **no IO happens under the lock**, so it stays non-blocking per
/// the contract. The one exception is [`Self::flush`], the explicit, debounced
/// durable write-back: it deliberately writes under the lock (the write *is*
/// the work, and it never runs per keystroke).
#[derive(uniffi::Object)]
pub struct OpenDocHandle {
    /// `None` once closed. Wrapped in a `Mutex` for interior mutability (the
    /// exported methods take `&self`, but edits need `&mut`).
    session: Mutex<Option<DocSession>>,
}

impl std::fmt::Debug for OpenDocHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `DocSession` holds a `Highlighter` whose `Debug` is a summary; report
        // only whether the session is open to satisfy
        // `missing_debug_implementations` without taking the lock's contents
        // apart (and without risking a deadlock-y lock in a formatter).
        f.debug_struct("OpenDocHandle")
            .field("open", &self.session.try_lock().map(|g| g.is_some()).ok())
            .finish()
    }
}

impl OpenDocHandle {
    /// Lock the session, mapping mutex poisoning (a prior panic while the lock
    /// was held — should be unreachable given the no-panic posture, but handled
    /// rather than `unwrap`ped per NFR-003) to [`FfiError::Internal`].
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<DocSession>>, FfiError> {
        self.session.lock().map_err(|_| FfiError::Internal {
            detail: "document session lock poisoned".to_owned(),
        })
    }
}

#[uniffi::export]
impl OpenDocHandle {
    /// Apply one editor delta — replace `range` (UTF-16 code units) with
    /// `replacement` — to **both** the shadow [`Document`] and the
    /// [`Highlighter`], keeping the two in lock-step.
    ///
    /// **HOT PATH** (FFI contract §3): synchronous, non-blocking, no IO. The
    /// lock is held only for the two in-memory splices.
    ///
    /// Returns `Result` (a documented deviation from the contract's infallible
    /// sketch — see the module docs): a malformed delta surfaces as a thrown
    /// `FfiError` instead of being swallowed.
    ///
    /// # Errors
    ///
    /// - [`FfiError::Internal`] if `range` is out of bounds, inverted, or splits
    ///   a surrogate pair — the buffer/tree are left **unmodified** (the core
    ///   validates the whole range before mutating either side).
    /// - [`FfiError::Internal`] if the handle is closed or the lock is poisoned.
    pub fn push_edit(&self, range: U16Range, replacement: String) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        let session = guard.as_mut().ok_or_else(closed_handle)?;
        let core_range: CoreU16Range = range.into();

        // Apply to the Document first: it performs the same up-front bounds /
        // surrogate validation as the Highlighter, so if the delta is malformed
        // this rejects it *before* the (more expensive) incremental reparse and
        // — crucially — before either side is mutated, so they cannot drift.
        session.doc.push_edit(core_range, &replacement)?;

        // Feed the IDENTICAL delta to the highlighter so its rope mirror + tree
        // track the document. Both consume the same `(U16Range, &str)`, so once
        // the Document accepted the range the Highlighter will too; mapping any
        // (unreachable) error keeps the no-panic posture.
        session.highlighter.apply_edit(core_range, &replacement)?;
        Ok(())
    }

    /// Editor-highlight spans for nodes intersecting `viewport` (UTF-16 code
    /// units); the editor pulls these lazily on scroll (FFI contract §3).
    ///
    /// Returns `Result` for the same reason as [`Self::push_edit`]: an
    /// out-of-bounds / surrogate-splitting viewport is a caller bug worth
    /// surfacing.
    ///
    /// # Errors
    ///
    /// - [`FfiError::Internal`] if `viewport` is out of bounds or splits a
    ///   surrogate pair.
    /// - [`FfiError::Internal`] if the handle is closed or the lock is poisoned.
    pub fn highlight_spans(&self, viewport: U16Range) -> Result<Vec<StyleSpan>, FfiError> {
        let guard = self.lock()?;
        let session = guard.as_ref().ok_or_else(closed_handle)?;
        let spans = session.highlighter.highlight_spans(viewport.into())?;
        Ok(spans.into_iter().map(StyleSpan::from).collect())
    }

    /// Force a **durable** write-back of the current buffer to the note's path
    /// (FFI contract §3 `flush`; FR-009a).
    ///
    /// Autosave is internal + debounced; this is the explicit-flush path the
    /// Swift `AutosaveController` calls on its idle/hard-cap timers and on
    /// close/quit. It snapshots the buffer ([`Document::text`]) under the lock
    /// and writes it via [`emend_core::fs::write_atomic`] (tempfile → fsync →
    /// atomic rename → dir fsync; `sync_all` is `F_FULLFSYNC` on Apple), so an
    /// external observer never sees a half-written note.
    ///
    /// Unlike [`Self::push_edit`] this is **not** the hot path: it is an
    /// explicit, debounced flush, so doing the IO under the lock is acceptable
    /// here (the write *is* the work) — it does not run per keystroke.
    ///
    /// Self-write suppression — feeding the post-write `(mtime, len)` to the
    /// file watcher so the app's own save does not echo back as an external
    /// change (FR-006a) — arrives with US2's watcher. For now `flush` only
    /// performs the atomic durable write; once the watcher exists this method
    /// will also report the resulting `(mtime, len)` so the save is ignored.
    ///
    /// # Errors
    ///
    /// - [`FfiError::NotFound`] / [`FfiError::PermissionDenied`] /
    ///   [`FfiError::IoFailure`] for the corresponding write failures
    ///   (propagated from [`emend_core::fs::write_atomic`]).
    /// - [`FfiError::Internal`] if the handle is closed or the lock is poisoned.
    pub fn flush(&self) -> Result<(), FfiError> {
        let guard = self.lock()?;
        let session = guard.as_ref().ok_or_else(closed_handle)?;
        // Snapshot the buffer and write it durably to the captured path. The
        // write is the explicit-flush work, so holding the lock across it is
        // fine (this is not the per-keystroke path).
        emend_core::fs::write_atomic(&session.path, &session.doc.text())?;
        Ok(())
    }

    /// Render the document's **current buffer** to preview HTML (FFI contract §6,
    /// US4 · FR-023..028).
    ///
    /// Snapshots the buffer ([`Document::text`]) under the lock and renders it
    /// with the **authoritative** comrak engine
    /// ([`emend_core::parse::preview::render_preview_html`]) — CommonMark + GFM +
    /// the native `[[wikilink]]` / `==highlight==` extensions, with `data-line`
    /// scroll-sync anchors (research §C3) and syntect-coloured code blocks
    /// (research §B6). This is a separate engine from the editor highlighter
    /// reached by [`Self::highlight_spans`] — they are never unified (Constitution).
    ///
    /// Not the hot path: the preview is debounced off the keystroke path
    /// (research §B1), so taking the buffer snapshot under the lock is acceptable.
    /// The render itself is pure CPU/string with **no network access** (SC-008) —
    /// remote URLs in the source stay literal; the runtime offline guarantee for
    /// the rendered page is enforced Swift-side by the WKWebView CSP (T087).
    ///
    /// `![[embed]]` tokens render **literally** here — this variant has no
    /// workspace to resolve them against. Callers that want embeds inlined use
    /// [`Self::render_preview_html_resolving`] (US5), which passes a
    /// [`WorkspaceHandle`] resolver to the embed-aware core renderer. This plain
    /// variant stays for resolver-less callers (a standalone snippet render with no
    /// workspace).
    ///
    /// # Errors
    ///
    /// - [`FfiError::Internal`] if rendering fails unexpectedly (rendering is pure
    ///   and does not normally fail), or if the handle is closed / the lock is
    ///   poisoned.
    pub fn render_preview_html(&self) -> Result<String, FfiError> {
        let guard = self.lock()?;
        let session = guard.as_ref().ok_or_else(closed_handle)?;
        let source = session.doc.text();
        let html = emend_core::parse::preview::render_preview_html(
            &source,
            &emend_core::parse::preview::PreviewOptions::default(),
        )?;
        Ok(html)
    }

    /// Render the document's current buffer to preview HTML **with `![[embed]]`
    /// resolution** against `workspace` (FFI contract §6, US5 · FR-021/021a).
    ///
    /// The embed-aware sibling of [`Self::render_preview_html`]. Both remain: the
    /// plain method stays for resolver-less callers (a standalone snippet render
    /// with no workspace), and this one inlines embedded notes' content for the
    /// live preview. `workspace` is the [`WorkspaceHandle`] that owns the search
    /// index `resolve_wikilink` consults — passed as an `Arc<WorkspaceHandle>`
    /// object handle across the boundary (UniFFI 0.31 supports object-handle
    /// arguments; this is the first such method in the shim, verified to generate
    /// cleanly).
    ///
    /// It snapshots the buffer **and the note's own path** under the lock, then
    /// **releases the lock before rendering** (the comrak/expand pass below runs
    /// off-lock — neither this handle's `Mutex` nor the workspace's index `Mutex`
    /// is held across the render). The resolver closure maps each embed target to
    /// the resolved note's source via
    /// [`WorkspaceHandle::resolve_embed_source`](crate::workspace::WorkspaceHandle)
    /// — which resolves under the index lock then reads off-lock (so the recursive
    /// expander never serializes file ops / Quick Open against the index). The
    /// note's own path is the `from_note` for the deterministic same-directory
    /// tie-break (FR-019a). An unresolved / cyclic / too-deep embed degrades to a
    /// visible placeholder (FR-021a/FR-022) — the expander owns those guards
    /// (`MAX_EMBED_DEPTH = 8`), so this call always terminates.
    ///
    /// A resolver lock-poisoning is recorded and surfaced as [`FfiError::Internal`]
    /// after the render (rather than panicking inside the `FnMut` closure, which
    /// cannot itself return a `Result`), so a corrupt-state render fails loudly
    /// instead of silently dropping every embed.
    ///
    /// Not the hot path: like [`Self::render_preview_html`] the preview is
    /// debounced off the keystroke path (research §B1). No network access (SC-008)
    /// — the render is pure; the only IO is the tolerant per-embed source read,
    /// which never dereferences a URL.
    ///
    /// # Errors
    ///
    /// - [`FfiError::Internal`] if the handle is closed, this handle's lock is
    ///   poisoned, the workspace's index lock is poisoned during resolution, or
    ///   rendering fails unexpectedly (rendering is pure and does not normally
    ///   fail).
    pub fn render_preview_html_resolving(
        &self,
        workspace: Arc<WorkspaceHandle>,
    ) -> Result<String, FfiError> {
        // Snapshot the buffer AND the note's own path under the lock, then drop the
        // guard so the (off-hot-path) render below holds neither this handle's lock
        // nor — via the resolver — the workspace index lock across the comrak pass.
        let (source, from_note) = {
            let guard = self.lock()?;
            let session = guard.as_ref().ok_or_else(closed_handle)?;
            (
                session.doc.text(),
                session.path.to_string_lossy().into_owned(),
            )
            // `guard` drops here, releasing this handle's lock before rendering.
        };

        // The `FnMut` resolver cannot return a `Result`, so capture any
        // lock-poisoning that surfaces during resolution and re-raise it after the
        // render rather than swallowing it (a corrupt index lock must fail loudly,
        // not silently drop every embed).
        let mut resolve_error: Option<FfiError> = None;
        let mut resolve = |raw_target: &str| -> Option<String> {
            match workspace.resolve_embed_source(&from_note, raw_target) {
                Ok(source) => source,
                Err(err) => {
                    // Keep the first error; a `None` here just renders this one
                    // embed as a placeholder, and the error is surfaced below.
                    resolve_error.get_or_insert(err);
                    None
                }
            }
        };

        let html = emend_core::parse::preview::render_preview_html_with_embeds(
            &source,
            &emend_core::parse::preview::PreviewOptions::default(),
            &mut resolve,
        )?;

        if let Some(err) = resolve_error {
            return Err(err);
        }
        Ok(html)
    }

    /// The `[[wiki links]]` and `![[embeds]]` in the current buffer, each with
    /// its UTF-16 source range (FFI contract §4 `links`; FR-019..022).
    ///
    /// Snapshots the buffer ([`Document::text`]) under the lock and scans it with
    /// [`emend_core::derived::extract_links`]. The `raw_target` of each is resolved
    /// to a note path Swift-side via
    /// [`crate::workspace::WorkspaceHandle::resolve_wikilink`] (the index lives in
    /// the workspace handle, not the document) — this method only *extracts* the
    /// references and their ranges.
    ///
    /// Not the hot path (the info-sidebar/derived layer pulls these debounced,
    /// FR-031a), so the buffer snapshot under the lock is acceptable.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the handle is closed or the lock is poisoned.
    pub fn links(&self) -> Result<Vec<LinkRef>, FfiError> {
        let guard = self.lock()?;
        let session = guard.as_ref().ok_or_else(closed_handle)?;
        let source = session.doc.text();
        Ok(extract_links(&source)
            .into_iter()
            .map(LinkRef::from)
            .collect())
    }

    /// Toggle the task checkbox on the line containing the UTF-16 offset `at`
    /// (FFI contract §3 `toggle_task`; FR-014 clickable checkbox).
    ///
    /// Computes the new line text with [`emend_core::derived::toggle_task`]
    /// (flipping `[ ]`↔`[x]`) and applies the change as a single `push_edit`
    /// delta over the toggled line, so the shadow [`Document`] **and** the
    /// [`Highlighter`] stay in lock-step (same path as typing) and a later
    /// [`flush`](Self::flush) writes the toggled Markdown to disk.
    ///
    /// The contract takes a `U16Range`; we use its `start` as the click offset
    /// (the click is a caret position, not a selection). A click that is not on a
    /// task line surfaces an error rather than silently no-op'ing.
    ///
    /// # Errors
    ///
    /// - [`FfiError::InvalidConfig`] if `at` is past EOF or its line is not a task
    ///   checkbox (propagated from the core toggle).
    /// - [`FfiError::Internal`] if the handle is closed, the lock is poisoned, or
    ///   the computed edit is rejected (unreachable — the new text differs only in
    ///   one checkbox char).
    pub fn toggle_task(&self, at: U16Range) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        let session = guard.as_mut().ok_or_else(closed_handle)?;

        let old_text = session.doc.text();
        let new_text = toggle_task(&old_text, at.start)?;

        // Apply the change as a whole-document replacement delta so the Document
        // and Highlighter both track it (the toggle changes exactly one char, but
        // computing a minimal delta here would duplicate the core's line-finding;
        // a whole-doc replace is correct and the toggle is not the hot path).
        let full_len = session.doc.len_utf16();
        let whole = CoreU16Range::new(0, full_len);
        session.doc.push_edit(whole, &new_text)?;
        session.highlighter.apply_edit(whole, &new_text)?;
        Ok(())
    }

    /// Explicit close (FFI contract §3 `close_document`). Consumes the inner
    /// session, running [`Document::close`]; the handle is inert afterwards.
    ///
    /// Idempotent: closing an already-closed handle is a no-op. Releasing the
    /// last `Arc<OpenDocHandle>` also frees the session via [`Drop`], so this is
    /// for callers who want a deterministic, intention-revealing teardown (and a
    /// future hook for flushing a pending autosave).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn close(&self) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        if let Some(session) = guard.take() {
            // Run the core's explicit close (consumes the Document). The
            // Highlighter is dropped with the session.
            session.doc.close();
        }
        Ok(())
    }
}

/// `FfiError::Internal` for a use-after-close.
fn closed_handle() -> FfiError {
    FfiError::Internal {
        detail: "document handle is closed".to_owned(),
    }
}

/// The syntect theme CSS for the preview's classed code blocks (US4 · research
/// §B6 / §C2).
///
/// ## Contract-ambiguity resolution (`preview_assets_dir`)
///
/// The FFI contract (§6) sketches `preview_assets_dir() -> String` as "bundled
/// Mermaid/KaTeX/theme CSS base". That shape does not fit the Rust core: **Mermaid
/// and KaTeX are bundled in the SWIFT app** (vendored JS/CSS/fonts loaded via
/// `loadFileURL`, research §C2) — they are *not* owned by, or knowable to, the
/// Rust core, and there is no filesystem directory the core could meaningfully
/// return for them.
///
/// What the core *does* own is the **syntect theme CSS** for the classed
/// `<span class="…">` runs emitted by `render_preview_html` (research §B6). So we
/// resolve the ambiguous `preview_assets_dir` into a precise, core-owned export:
/// `preview_theme_css()` returns that stylesheet as a `String`, which Swift
/// injects into its WebView HTML template alongside the app-bundled Mermaid/KaTeX
/// assets. This keeps the Rust/Swift ownership split honest (core owns the
/// highlight theme; the app owns its bundled JS libraries) and gives Swift
/// exactly the bytes it needs.
///
/// Infallible: the CSS is a compiled-in `&'static str` (vendored alongside the
/// syntax dump), so this cannot fail and is effectively free to call.
#[uniffi::export]
#[must_use]
pub fn preview_theme_css() -> String {
    emend_core::parse::code_highlight::theme_css().to_owned()
}

/// Open the note at `path` into an [`OpenDocHandle`] (FFI contract §3).
///
/// Reads via [`Document::open`], which enforces the note-size cap before
/// allocating (→ [`FfiError::NoteTooLarge`] for an over-cap file, so the caller
/// can fall back to read-only) and reads tolerant of BOM/CRLF/non-UTF-8
/// (FR-003a). The highlighter is built from the freshly read text so the two
/// start in agreement.
///
/// # Errors
///
/// - [`FfiError::NoteTooLarge`] if the file exceeds the size cap (FR-027a).
/// - [`FfiError::NotFound`] / [`FfiError::PermissionDenied`] /
///   [`FfiError::IoFailure`] for the corresponding IO failures.
#[uniffi::export]
pub fn open_document(path: String) -> Result<Arc<OpenDocHandle>, FfiError> {
    // Retain the path so a later `flush` can write the buffer back to the same
    // file (FR-009a). `Document::open` only borrows it for the read.
    let path = PathBuf::from(path);
    let doc = Document::open(&path)?;
    // Build the highlighter from the document's text so both shadows agree from
    // the first delta onward. `text()` allocates once at open time (not on the
    // hot path), which is acceptable for a one-shot load.
    let highlighter = Highlighter::new(&doc.text());
    Ok(Arc::new(OpenDocHandle {
        session: Mutex::new(Some(DocSession {
            path,
            doc,
            highlighter,
        })),
    }))
}

#[cfg(test)]
mod tests {
    // The exported methods return `Result`/`Arc`; tests assert on their own
    // fixtures. The workspace denies these lints in library code, so scope the
    // allowance to this module (mirrors the core crate's test modules).
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{open_document, preview_theme_css, LinkKind, StyleClass, U16Range};
    use crate::error::FfiError;
    use crate::workspace::new_workspace;

    fn u16_len(s: &str) -> u32 {
        u32::try_from(s.encode_utf16().count()).expect("fits in u32")
    }

    fn write_note(dir: &tempfile::TempDir, name: &str, body: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write fixture note");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn open_push_edit_then_highlight_reflects_edit() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Start with a plain paragraph, then turn it INTO a heading by inserting
        // the ATX marker — proving the highlighter tracks the pushed delta.
        let path = write_note(&dir, "note.md", "Title\n");
        let handle = open_document(path).expect("open");

        // Insert "### " at offset 0 → "### Title\n".
        handle
            .push_edit(U16Range { start: 0, len: 0 }, "### ".to_owned())
            .expect("push_edit insert");

        let text = "### Title\n";
        let spans = handle
            .highlight_spans(U16Range {
                start: 0,
                len: u16_len(text),
            })
            .expect("highlight_spans");

        let classes: Vec<StyleClass> = spans.iter().map(|s| s.class).collect();
        assert!(
            classes.contains(&StyleClass::Heading { level: Some(3) }),
            "edited content should classify as a level-3 heading: {spans:?}"
        );
    }

    #[test]
    fn replace_edit_updates_highlight() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "# One\n");
        let handle = open_document(path).expect("open");

        // Replace the single `#` (1 UTF-16 unit at offset 0) with `###`.
        handle
            .push_edit(U16Range { start: 0, len: 1 }, "###".to_owned())
            .expect("push_edit replace");

        let text = "### One\n";
        let spans = handle
            .highlight_spans(U16Range {
                start: 0,
                len: u16_len(text),
            })
            .expect("highlight_spans");

        assert!(
            spans
                .iter()
                .any(|s| s.class == StyleClass::Heading { level: Some(3) }),
            "after replace the heading should be level 3: {spans:?}"
        );
    }

    #[test]
    fn over_cap_file_round_trips_note_too_large() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One byte over the cap is enough to trip the size guard before any rope
        // is allocated.
        let over_cap = usize::try_from(emend_core::document::Document::MAX_NOTE_BYTES + 1)
            .expect("fits usize");
        let path = write_note(&dir, "huge.md", &"a".repeat(over_cap));

        let err = open_document(path).expect_err("over-cap file must be rejected");
        assert!(
            matches!(err, FfiError::NoteTooLarge { .. }),
            "expected NoteTooLarge, got {err:?}"
        );
    }

    #[test]
    fn out_of_bounds_push_edit_surfaces_error_without_corrupting_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "abc\n");
        let handle = open_document(path).expect("open");

        // Offset 100 is well past EOF → rejected, buffer untouched.
        let err = handle
            .push_edit(U16Range { start: 100, len: 0 }, "x".to_owned())
            .expect_err("out-of-bounds edit must be rejected");
        assert!(
            matches!(err, FfiError::Internal { .. }),
            "expected Internal for OOB edit, got {err:?}"
        );

        // State is intact: a valid edit and query still succeed afterwards.
        handle
            .push_edit(U16Range { start: 0, len: 0 }, "# ".to_owned())
            .expect("valid edit after a rejected one must still work");
        let spans = handle
            .highlight_spans(U16Range {
                start: 0,
                len: u16_len("# abc\n"),
            })
            .expect("highlight after recovery");
        assert!(
            spans
                .iter()
                .any(|s| s.class == StyleClass::Heading { level: Some(1) }),
            "buffer must reflect only the valid edit: {spans:?}"
        );
    }

    #[test]
    fn out_of_bounds_viewport_surfaces_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "abc\n");
        let handle = open_document(path).expect("open");

        let err = handle
            .highlight_spans(U16Range { start: 100, len: 1 })
            .expect_err("OOB viewport must be rejected");
        assert!(
            matches!(err, FfiError::Internal { .. }),
            "expected Internal for OOB viewport, got {err:?}"
        );
    }

    #[test]
    fn close_is_idempotent_and_makes_handle_inert() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "abc\n");
        let handle = open_document(path).expect("open");

        handle.close().expect("first close");
        handle.close().expect("second close is a no-op");

        // After close the handle is inert: edits/queries report Internal rather
        // than panicking (use-after-close safety).
        let edit_err = handle
            .push_edit(U16Range { start: 0, len: 0 }, "x".to_owned())
            .expect_err("edit after close must error");
        assert!(matches!(edit_err, FfiError::Internal { .. }));

        let query_err = handle
            .highlight_spans(U16Range { start: 0, len: 0 })
            .expect_err("query after close must error");
        assert!(matches!(query_err, FfiError::Internal { .. }));
    }

    #[test]
    fn flush_writes_edited_buffer_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "Title\n");
        let handle = open_document(path.clone()).expect("open");

        // Turn the plain line into a heading, then force a durable write-back.
        handle
            .push_edit(U16Range { start: 0, len: 0 }, "# ".to_owned())
            .expect("push_edit insert");
        handle.flush().expect("flush");

        // The bytes on disk must now match the edited buffer.
        let on_disk = std::fs::read_to_string(&path).expect("read flushed note");
        assert_eq!(on_disk, "# Title\n");
    }

    #[test]
    fn flush_after_close_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "abc\n");
        let handle = open_document(path).expect("open");

        handle.close().expect("close");

        let err = handle.flush().expect_err("flush after close must error");
        assert!(
            matches!(err, FfiError::Internal { .. }),
            "expected Internal for flush after close, got {err:?}"
        );
    }

    #[test]
    fn missing_file_maps_to_not_found() {
        let err = open_document("/no/such/emend/note.md".to_owned())
            .expect_err("missing file must error");
        assert!(
            matches!(err, FfiError::NotFound { .. }),
            "expected NotFound, got {err:?}"
        );
    }

    #[test]
    fn render_preview_html_reflects_current_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A heading plus a fenced rust block exercises both the data-line anchor
        // path and the syntect classed-code path.
        let path = write_note(&dir, "note.md", "# Title\n\n```rust\nfn main() {}\n```\n");
        let handle = open_document(path).expect("open");

        let html = handle.render_preview_html().expect("render preview");
        assert!(html.contains("<h1"), "heading should render: {html}");
        assert!(
            html.contains("data-line=\"1\""),
            "scroll-sync anchor should be present: {html}"
        );
        assert!(
            html.contains("<span class=\""),
            "syntect classed code should be present: {html}"
        );

        // The render must track edits: insert a tasklist line and re-render.
        let len = u16_len("# Title\n\n```rust\nfn main() {}\n```\n");
        handle
            .push_edit(U16Range { start: len, len: 0 }, "\n- [x] done\n".to_owned())
            .expect("push edit");
        let html2 = handle.render_preview_html().expect("re-render");
        assert!(
            html2.contains("type=\"checkbox\""),
            "edited-in tasklist should appear in re-render: {html2}"
        );
    }

    #[test]
    fn render_preview_html_after_close_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "# Hi\n");
        let handle = open_document(path).expect("open");
        handle.close().expect("close");

        let err = handle
            .render_preview_html()
            .expect_err("render after close must error");
        assert!(
            matches!(err, FfiError::Internal { .. }),
            "expected Internal for render after close, got {err:?}"
        );
    }

    #[test]
    fn render_preview_html_resolving_inlines_embedded_note() {
        // Two notes: A embeds B (`![[B]]`); B has body text. Rendering A *through
        // the workspace resolver* must inline B's body into A's HTML (proving the
        // FFI embed path resolves a target to real content). The plain
        // `render_preview_html` (no resolver) must NOT inline it — embeds stay
        // literal there.
        let dir = tempfile::tempdir().expect("tempdir");
        // Canonicalize the root so `add_location`/`collect_files`'/the resolver's
        // canonical identity all agree (NFR-007 — macOS `/var` -> `/private/var`).
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");

        let note_a = root.join("A.md");
        let note_b = root.join("B.md");
        std::fs::write(&note_a, "Intro paragraph.\n\n![[B]]\n").expect("write A");
        std::fs::write(&note_b, "## Bravo Heading\n\nThe body of note B.\n").expect("write B");

        // A workspace whose index knows both notes (seed from disk).
        let ws = new_workspace();
        ws.add_location(root.to_string_lossy().into_owned(), Vec::new())
            .expect("add location");
        let count = ws.reindex_all(16).expect("reindex");
        assert_eq!(count, 2, "both notes are seeded into the index");

        // Open note A and render WITH embed resolution against the workspace.
        let handle = open_document(note_a.to_string_lossy().into_owned()).expect("open A");
        let resolved = handle
            .render_preview_html_resolving(ws)
            .expect("render A with embeds");

        // B's body and heading are inlined into A's preview HTML.
        assert!(
            resolved.contains("The body of note B."),
            "embed must inline B's body: {resolved}"
        );
        assert!(
            resolved.contains("Bravo Heading"),
            "embed must inline B's heading: {resolved}"
        );
        assert!(
            resolved.contains("<h2"),
            "B's heading should render as an <h2> in A's context: {resolved}"
        );
        // A's own content is still there.
        assert!(resolved.contains("Intro paragraph."), "{resolved}");
        // The raw embed token is gone (it was inlined, not left literal).
        assert!(
            !resolved.contains("![[B]]"),
            "raw embed token must be replaced: {resolved}"
        );

        // Contrast: the plain (resolver-less) render leaves the embed unexpanded —
        // B's body must NOT appear.
        let plain = handle.render_preview_html().expect("plain render");
        assert!(
            !plain.contains("The body of note B."),
            "plain render must not inline embedded content: {plain}"
        );
    }

    #[test]
    fn render_preview_html_resolving_unresolved_embed_degrades() {
        // An embed whose target is not in the workspace renders a visible
        // placeholder, not an error, and the rest of the document still renders.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let note_a = root.join("A.md");
        std::fs::write(&note_a, "Kept text.\n\n![[Missing]]\n").expect("write A");

        let ws = new_workspace();
        ws.add_location(root.to_string_lossy().into_owned(), Vec::new())
            .expect("add location");
        ws.reindex_all(16).expect("reindex");

        let handle = open_document(note_a.to_string_lossy().into_owned()).expect("open A");
        let html = handle
            .render_preview_html_resolving(ws)
            .expect("render with an unresolved embed must still succeed");

        assert!(
            html.contains("Kept text."),
            "surrounding content renders: {html}"
        );
        assert!(
            html.contains("unresolved embed"),
            "an unresolved embed degrades to a visible placeholder: {html}"
        );
    }

    #[test]
    fn render_preview_html_resolving_after_close_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "# Hi\n");
        let handle = open_document(path).expect("open");
        let ws = new_workspace();
        handle.close().expect("close");

        let err = handle
            .render_preview_html_resolving(ws)
            .expect_err("resolving render after close must error");
        assert!(
            matches!(err, FfiError::Internal { .. }),
            "expected Internal for resolving render after close, got {err:?}"
        );
    }

    #[test]
    fn links_extracts_wiki_links_and_embeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "See [[Target]] and ![[Embed]] here.\n");
        let handle = open_document(path).expect("open");

        let links = handle.links().expect("links");
        assert_eq!(links.len(), 2, "one link + one embed: {links:?}");
        assert_eq!(links[0].kind, LinkKind::Link);
        assert_eq!(links[0].raw_target, "Target");
        assert_eq!(links[1].kind, LinkKind::Embed);
        assert_eq!(links[1].raw_target, "Embed");
    }

    #[test]
    fn toggle_task_flips_checkbox_and_persists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "- [ ] task\n");
        let handle = open_document(path.clone()).expect("open");

        // Toggle at offset 0 (anywhere on the line works).
        handle
            .toggle_task(U16Range { start: 0, len: 0 })
            .expect("toggle");
        handle.flush().expect("flush");

        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert_eq!(on_disk, "- [x] task\n");

        // Toggling again flips it back, proving the doc tracked the edit.
        handle
            .toggle_task(U16Range { start: 0, len: 0 })
            .expect("toggle back");
        handle.flush().expect("flush");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "- [ ] task\n"
        );
    }

    #[test]
    fn toggle_task_on_non_task_line_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "just text\n");
        let handle = open_document(path).expect("open");

        let err = handle
            .toggle_task(U16Range { start: 0, len: 0 })
            .expect_err("toggling a non-task line must error");
        assert!(
            matches!(err, FfiError::InvalidConfig { .. }),
            "expected InvalidConfig, got {err:?}"
        );
    }

    #[test]
    fn links_after_close_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "note.md", "[[x]]\n");
        let handle = open_document(path).expect("open");
        handle.close().expect("close");
        assert!(matches!(
            handle.links().expect_err("links after close"),
            FfiError::Internal { .. }
        ));
    }

    #[test]
    fn preview_theme_css_is_present_and_classed() {
        // The core-owned syntect theme CSS (resolves the contract's
        // `preview_assets_dir` ambiguity — see `preview_theme_css` docs).
        let css = preview_theme_css();
        assert!(!css.is_empty(), "theme CSS must be present");
        assert!(
            css.contains('.'),
            "ClassStyle::Spaced theme CSS should contain class selectors: {}",
            &css[..css.len().min(120)]
        );
    }
}
