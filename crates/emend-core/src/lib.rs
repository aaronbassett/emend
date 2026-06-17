//! Emend core engine.
//!
//! All non-UI logic lives here with **no FFI dependency**, so the whole layer is
//! unit/property/bench-testable with plain `cargo test` (research §B8). The thin
//! UniFFI shim in `emend-ffi` re-exports these capabilities to Swift.
//!
//! Module map (populated by `/sdd:implement`):
//! - [`error`]  structured error type surfaced across the FFI boundary (§B7)
//! - `fs`       atomic+durable writes, reads tolerant of BOM/CRLF (§B4, FR-003a)
//! - `watcher`  debounced file watching + self-write suppression (§B3)
//! - `index`    path/name index for Quick Open + wiki-link resolution (§B2)
//! - `parse`    incremental tree-sitter highlight + comrak preview (§B1)
//! - `search`   nucleo-backed fuzzy ranking (§B2)
//! - `ai`       OpenAI-compatible streaming client, cancellable (§B5)

pub mod error;

/// Atomic+durable writes and tolerant reads — the byte gateway to notes on disk
/// (research §B4, FR-003a/FR-009a). Used by the document/autosave layer.
pub mod fs;

/// Open-document model: the shadow rope + UTF-16/line index behind the editor
/// hot path (research §A2/§A3, FFI contract §3). Backs `open_document` /
/// `close_document` / `push_edit` and the offset↔(line,col) queries the
/// highlight/outline layers build on.
pub mod document;

/// Markdown parsing. Holds the **two deliberately separate engines** (research
/// §B1, Constitution): the incremental tree-sitter editor-highlight engine
/// ([`parse::highlight`]) on the per-keystroke hot path, and (later) the comrak
/// preview engine — kept apart on purpose, never unified.
pub mod parse;

/// The file-based workspace model (US2): locations, lazy directory listing,
/// collision-safe file operations, the favorites/pins/icons/child-order store,
/// and the canonicalization + bounded-traversal primitives that give path
/// identity (NFR-007). Pure `std` + [`fs`]; **no FFI, no async** (Constitution V),
/// shaped to project onto the FFI contract's §1/§2 later.
pub mod workspace;

/// Derived link & task data over a document's Markdown source (US5 · FR-014,
/// FR-019/019a, FR-020): `[[wiki link]]` / `![[embed]]` extraction with UTF-16
/// source ranges, the **deterministic** wiki-link resolution policy on top of
/// the [`index`]'s name map (same-directory → shallowest → lexicographic
/// tie-break), `[[` autocomplete, and the clickable-checkbox `toggle_task`
/// transform. Pure `std` + [`index`]; **no FFI, no async** (Constitution V).
pub mod derived;

/// The workspace search index (US2): the derived, in-memory haystack behind
/// Quick Open (fuzzy name/path ranking, FR-017) and wiki-link resolution (O(1)
/// name map, FR-019a). Maintained **incrementally** — a single create/rename/
/// move/delete touches only the affected entry, never a full rescan (FR-017a) —
/// over a `nucleo-matcher` fuzzy core. Pure `std` + the matcher; **no FFI, no
/// async** (Constitution V), shaped to project onto the FFI contract's §5
/// `SearchHit` later.
pub mod index;

/// The streaming, **cancellable** Quick Open search driver (US3): ranks a query
/// over the [`index`] and emits ranked [`index::SearchHit`]s in batches via a
/// caller-supplied callback, stopping the instant the query is superseded
/// (FR-017, FR-018/SC-004, NFR-002; research §B2/§B7). Pure and **tokio-free** —
/// cancellation is a tiny [`search::Cancel`] (`AtomicBool`), so the supersede
/// behaviour is testable with plain `cargo test`; the FFI Quick Open driver
/// (T074) runs [`search::quick_open`] inside a `tokio` task and forwards each
/// batch to the foreign `SearchSink`. **No FFI, no async runtime** (Constitution
/// V).
pub mod search;

/// Live external-change detection + the conflict model (US2): a thin
/// `notify` + `notify-debouncer-full` wrapper ([`watcher::FsWatcher`]) over a
/// **pure, deterministically-tested** classification core — move correlation
/// ([`watcher::classify`], one rename event not delete+create, FR-006b),
/// self-write suppression ([`watcher::SuppressionRegistry`], identity-keyed so
/// our own atomic saves never echo, FR-006a), and the conflict truth table
/// ([`watcher::resolve_conflict`], FR-006c). `notify` runs on its own threads and
/// posts to a `std::sync::mpsc` channel; **no FFI, no async runtime**
/// (Constitution V), shaped to project onto a foreign-trait `WatchObserver`
/// callback later (T059).
pub mod watcher;

/// The BYOM AI client — the **pure half** (US6 · FR-032/035/036/036a, NFR-006;
/// research §B5). Owns the lenient SSE event parser ([`ai::SseParser`]), the
/// max-input guard ([`ai::check_input_size`], FR-036a), the redacting
/// [`ApiKey`](ai::ApiKey) newtype (NFR-006), and the pure request *builders*
/// (headers/body as data). It deliberately holds **no network**: `emend-core`
/// stays tokio-free AND reqwest-free (Constitution V), so the `reqwest`/`tokio`
/// streaming orchestration lives in `emend-ffi`, feeding raw bytes through
/// [`ai::SseParser`] and pushing deltas to the foreign `AiSink`.
pub mod ai;

/// The crate's primary error type, re-exported at the root for ergonomic use
/// (`emend_core::EmendError`) by the FFI shim and callers.
pub use error::EmendError;

/// UTF-16 code-unit text range — the canonical range unit crossing the FFI
/// boundary so it maps 1:1 onto `NSRange` (research §A2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct U16Range {
    pub start: u32,
    pub len: u32,
}

impl U16Range {
    #[must_use]
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    /// The exclusive end offset (`start + len`), saturating at [`u32::MAX`]
    /// rather than overflowing.
    ///
    /// Inputs arrive from the FFI as `UInt32`, so a hostile/buggy caller could
    /// supply `start + len > u32::MAX`. A plain `+` would panic in debug and
    /// wrap in release *before* the caller's bounds check runs; saturating
    /// instead yields `u32::MAX`, which then cleanly fails every downstream
    /// "offset within document length" check (a document can never be that
    /// long), turning an overflow into a normal out-of-bounds rejection.
    /// `saturating_add` is `const` on `u32`, so this stays a `const fn`.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.start.saturating_add(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::U16Range;

    #[test]
    fn u16range_end_is_start_plus_len() {
        let r = U16Range::new(3, 4);
        assert_eq!(r.end(), 7);
    }

    #[test]
    fn u16range_end_saturates_on_overflow() {
        // start + len would overflow u32; `end()` must saturate (not panic in
        // debug / wrap in release) so the overflowed end cleanly fails the
        // downstream bounds checks instead.
        let r = U16Range::new(u32::MAX, 5);
        assert_eq!(r.end(), u32::MAX);
    }
}
