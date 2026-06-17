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
