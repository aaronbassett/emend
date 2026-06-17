//! Open-document model — the shadow rope and UTF-16/line index behind the
//! per-keystroke editor hot path (research §A2/§A3, FFI contract §3).
//!
//! ## Why a shadow rope at all
//!
//! **Swift's `NSTextStorage` is the canonical buffer** (research §A3). On every
//! keystroke Swift sends a tiny `{utf16Range, replacement}` delta; Rust keeps a
//! [`ropey::Rope`] *shadow* of the same text so it can answer structural,
//! highlight, outline, and search queries off the main thread without
//! re-marshalling the whole document. [`Document`] owns that rope and the index
//! that maps between the two coordinate systems the two sides speak.
//!
//! ## Two coordinate systems, one boundary
//!
//! - The **FFI boundary speaks UTF-16 code units** ([`U16Range`], `u32`), because
//!   that is what `NSRange`/`NSTextRange` use (research §A2). Pinning this avoids
//!   a whole-document UTF-8↔UTF-16 conversion on every keystroke.
//! - **ropey indexes in `char`s** (Unicode scalar values, `usize`). These differ
//!   for any scalar above U+FFFF: an astral char like "😀" (U+1F600) is **one
//!   `char` but two UTF-16 code units** (a surrogate pair). They also differ from
//!   *bytes*, so neither side may be conflated with UTF-8 offsets.
//!
//! Every public method converts at exactly one place, with **checked**
//! conversions (`try_into`, ropey's `try_*` accessors, explicit bounds checks)
//! mapped to [`EmendError`] — **never** an `as` cast that could truncate and
//! **never** a `panic`/`unwrap` (NFR-003: no panic may cross the FFI boundary).
//!
//! ## Surrogate-pair safety
//!
//! A UTF-16 offset that lands *between* the high and low surrogate of an astral
//! char is not a legal text boundary. ropey's [`Rope::utf16_cu_to_char`] rounds
//! such an offset *down* to the enclosing char; silently accepting that would
//! corrupt the user's buffer. We instead detect the split (convert the char back
//! to UTF-16 and require it to match the input) and reject it as an
//! [`EmendError::Internal`] — a programming error from the FFI shim, surfaced as
//! a catchable Swift error rather than data loss.
//!
//! ## Line semantics
//!
//! ropey is built here with `unicode_lines`/`cr_lines` **disabled** (see the
//! crate `Cargo.toml`), so **only LF (`\n`) and CRLF (`\r\n`) are line breaks**,
//! and a CRLF is a *single* break — matching NSTextView/TextKit line semantics.
//! Columns in [`LineCol`] are themselves UTF-16 code units measured from the
//! start of the line.

use crate::fs::read_tolerant;
use crate::{EmendError, U16Range};
use ropey::Rope;
use std::path::Path;

/// A zero-indexed `(line, column)` position. Both fields are **UTF-16 code
/// units**: `line` is the line number (LF/CRLF-delimited), `col` is the offset
/// in UTF-16 code units from the start of that line. This is the shape the
/// editor/outline/highlight layers consume; it maps directly onto AppKit's
/// line/column model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    /// Zero-indexed line number.
    pub line: u32,
    /// Zero-indexed column, in UTF-16 code units from the line start.
    pub col: u32,
}

impl LineCol {
    /// Construct a [`LineCol`] from a line and a UTF-16-code-unit column.
    #[must_use]
    pub const fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }
}

/// An open document: the shadow [`Rope`] plus the UTF-16/line index it answers
/// from. Owned by the core; the FFI shim (T039) wraps it behind an object handle
/// and exposes `open_document`/`push_edit`/`highlight_spans`/etc. over it.
///
/// `Document` deliberately holds **no `uniffi` types** (Constitution V) — it is a
/// plain `&mut self`/owned model so it stays unit-testable with `cargo test`.
#[derive(Debug, Clone)]
pub struct Document {
    /// Shadow of Swift's canonical `NSTextStorage` buffer (research §A3).
    rope: Rope,
}

impl Document {
    /// Maximum note size for full rich editing, in bytes (FR-027a, research §D).
    /// A file at or below this is opened normally; a file *over* it makes
    /// [`Document::open`] return [`EmendError::NoteTooLarge`] so the caller can
    /// fall back to read-only. Chosen as ~5 MB (5 * 1024 * 1024) to match the
    /// TextKit 2 + incremental-parse performance headroom recorded in research.
    pub const MAX_NOTE_BYTES: u64 = 5 * 1024 * 1024;

    /// Build an in-memory document from a string of document text. Used for
    /// non-file documents and tests; [`Document::open`] is the on-disk entry
    /// point.
    ///
    /// Named `from_text` rather than `from_str` deliberately: this is not a
    /// `FromStr`-style *parse* (it cannot fail), and the name avoids the
    /// confusion clippy flags when an inherent `from_str` shadows the
    /// [`std::str::FromStr`] trait method.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
        }
    }

    /// Open the note at `path` into a [`Document`].
    ///
    /// Enforces the **max-note-size cap before building the rope** (FR-027a):
    /// `fs::read_tolerant` is the low-level byte gateway and does *not* itself
    /// cap size, so we stat the file first and bail with
    /// [`EmendError::NoteTooLarge`] when it exceeds [`Document::MAX_NOTE_BYTES`].
    /// The cap is **inclusive** — a file exactly at the limit opens normally.
    /// Reading then goes through [`read_tolerant`], so a leading BOM is stripped,
    /// line endings are preserved, and non-UTF-8 bytes decode lossily (FR-003a).
    ///
    /// # Errors
    ///
    /// - [`EmendError::NoteTooLarge`] if the file exceeds the size cap.
    /// - [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
    ///   [`EmendError::IoFailure`] for the corresponding IO failures (propagated
    ///   from [`read_tolerant`], or from the size stat).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EmendError> {
        let path = path.as_ref();

        // Stat first so an oversized file never allocates a multi-megabyte rope.
        let metadata = std::fs::metadata(path).map_err(|e| map_io(path, &e))?;
        let bytes = metadata.len();
        if bytes > Self::MAX_NOTE_BYTES {
            return Err(EmendError::NoteTooLarge {
                path: path.display().to_string(),
                bytes,
                limit: Self::MAX_NOTE_BYTES,
            });
        }

        let text = read_tolerant(path)?;
        Ok(Self::from_text(&text))
    }

    /// Release this document. Dropping a [`Document`] frees its rope, so an
    /// explicit close is not strictly required; this method exists so the FFI
    /// shim's `close_document(handle)` has an unambiguous, intention-revealing
    /// call site (and a place to hang future teardown such as flushing a pending
    /// autosave). It simply consumes `self`, running [`Drop`].
    #[allow(
        clippy::unused_self,
        reason = "consumes self to run Drop; see doc comment"
    )]
    pub fn close(self) {}

    /// Total length of the document in **UTF-16 code units** — the unit the FFI
    /// boundary reports (research §A2). Not the same as byte length or char
    /// length when the document contains astral chars.
    ///
    /// A `u32` of code units covers ~4 GiB of UTF-16, far above
    /// [`Document::MAX_NOTE_BYTES`], so the length always fits for an opened
    /// note. The saturating fallback only keeps this infallible on the
    /// (unreachable) overflow path without a truncating `as` cast; the
    /// `debug_assert!` makes that assumption fail loudly in tests if the size
    /// cap is ever raised past `u32::MAX` code units.
    #[must_use]
    pub fn len_utf16(&self) -> u32 {
        let units = self.rope.len_utf16_cu();
        debug_assert!(
            u32::try_from(units).is_ok(),
            "UTF-16 length {units} exceeds u32; MAX_NOTE_BYTES should keep this unreachable"
        );
        u32::try_from(units).unwrap_or(u32::MAX)
    }

    /// The full document text. Allocates a fresh `String`; intended for tests,
    /// preview, and flush — **not** the per-keystroke path.
    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Apply one editor delta: replace the text in `range` (UTF-16 code units)
    /// with `replacement`. **This is the per-keystroke hot path** — synchronous,
    /// non-blocking, no IO. It converts the UTF-16 range to a char range via
    /// ropey and splices in place (ropey edits are O(log n)).
    ///
    /// The conversion is bounds- and surrogate-checked: a range that runs past
    /// the end of the document, or whose start/end lands inside a surrogate pair,
    /// is rejected **without mutating the buffer** — a corrupt splice is far
    /// worse than a rejected edit.
    ///
    /// # Errors
    ///
    /// [`EmendError::Internal`] if `range` is out of bounds or splits a surrogate
    /// pair (a programming error from the FFI shim; surfaced as a catchable Swift
    /// error per NFR-003, never a panic).
    pub fn push_edit(&mut self, range: U16Range, replacement: &str) -> Result<(), EmendError> {
        // Resolve BOTH endpoints to char indices up front, so a bad range fails
        // before we touch the rope (the failed edit leaves the buffer intact).
        let start_char = self.utf16_to_char(range.start)?;
        let end_char = self.utf16_to_char(range.end())?;

        // `end` cannot precede `start` after a valid conversion, but guard
        // defensively rather than let `remove` see an inverted range.
        if end_char < start_char {
            return Err(EmendError::Internal {
                detail: format!(
                    "inverted edit range: start_utf16={} end_utf16={}",
                    range.start,
                    range.end()
                ),
            });
        }

        if end_char > start_char {
            self.rope.remove(start_char..end_char);
        }
        if !replacement.is_empty() {
            self.rope.insert(start_char, replacement);
        }
        Ok(())
    }

    /// Convert a UTF-16 code-unit offset to a zero-indexed [`LineCol`].
    ///
    /// `offset` may be `0..=len_utf16()` (the trailing value is the just-past-EOF
    /// caret position). The column is in UTF-16 code units from the line start.
    ///
    /// # Errors
    ///
    /// [`EmendError::Internal`] if `offset` is past the end of the document or
    /// lands inside an astral surrogate pair (not a legal text boundary).
    pub fn utf16_to_line_col(&self, offset: u32) -> Result<LineCol, EmendError> {
        let char_idx = self.utf16_to_char(offset)?;

        // Line of this char (LF/CRLF only — see module docs). `char_to_line`
        // accepts `0..=len_chars`, and `char_idx` is already validated in range.
        let line = self.rope.char_to_line(char_idx);

        // Column = UTF-16 distance from the line's first char to `char_idx`.
        let line_start_char = self.rope.line_to_char(line);
        let line_start_u16 = self.rope.char_to_utf16_cu(line_start_char);
        let col_u16 = offset
            .checked_sub(u32::try_from(line_start_u16).map_err(too_large)?)
            .ok_or_else(|| EmendError::Internal {
                detail: format!("column underflow at utf16 offset {offset}"),
            })?;

        Ok(LineCol {
            line: u32::try_from(line).map_err(too_large)?,
            col: col_u16,
        })
    }

    /// Convert a zero-indexed [`LineCol`] back to a UTF-16 code-unit offset.
    ///
    /// # Errors
    ///
    /// [`EmendError::Internal`] if the line does not exist, or if the column runs
    /// past the end of that line, or if the resulting position splits a surrogate
    /// pair.
    pub fn line_col_to_utf16(&self, pos: LineCol) -> Result<u32, EmendError> {
        let line = usize::try_from(pos.line).map_err(too_large)?;

        // `len_lines()` counts the (possibly empty) final line, so valid line
        // indices are `0..len_lines()`.
        if line >= self.rope.len_lines() {
            return Err(EmendError::Internal {
                detail: format!(
                    "line {} out of range (document has {} lines)",
                    pos.line,
                    self.rope.len_lines()
                ),
            });
        }

        // UTF-16 offset of the line start, then add the column.
        let line_start_char = self.rope.line_to_char(line);
        let line_start_u16 =
            u32::try_from(self.rope.char_to_utf16_cu(line_start_char)).map_err(too_large)?;
        let target_u16 =
            line_start_u16
                .checked_add(pos.col)
                .ok_or_else(|| EmendError::Internal {
                    detail: format!("offset overflow for {pos:?}"),
                })?;

        // The column must not run past the end of THIS line. The end of the line
        // is the start of the next line (or EOF for the final line). Validating
        // here keeps a too-large column from silently landing on a later line.
        let line_end_u16 = if line + 1 < self.rope.len_lines() {
            u32::try_from(self.rope.char_to_utf16_cu(self.rope.line_to_char(line + 1)))
                .map_err(too_large)?
        } else {
            self.len_utf16()
        };
        if target_u16 > line_end_u16 {
            return Err(EmendError::Internal {
                detail: format!(
                    "column {} past end of line {} (line ends at utf16 {})",
                    pos.col, pos.line, line_end_u16
                ),
            });
        }

        // Final guard: reject a position that splits a surrogate pair.
        self.validate_utf16_boundary(target_u16)?;
        Ok(target_u16)
    }

    // -- internal conversions -------------------------------------------------

    /// Convert a UTF-16 code-unit offset to a ropey **char index**, validating
    /// bounds and surrogate-pair alignment. The single place the UTF-16→char
    /// translation happens; every public query funnels through it.
    fn utf16_to_char(&self, offset: u32) -> Result<usize, EmendError> {
        let len_u16 = self.len_utf16();
        if offset > len_u16 {
            return Err(EmendError::Internal {
                detail: format!("utf16 offset {offset} out of bounds (document length {len_u16})"),
            });
        }

        let offset_usize = usize::try_from(offset).map_err(too_large)?;

        // ropey rounds an in-surrogate-pair offset DOWN to the enclosing char.
        // Detect that by converting the char back and requiring an exact match;
        // a mismatch means `offset` split a surrogate pair (illegal boundary).
        let char_idx = self.rope.utf16_cu_to_char(offset_usize);
        let roundtrip = u32::try_from(self.rope.char_to_utf16_cu(char_idx)).map_err(too_large)?;
        if roundtrip != offset {
            return Err(EmendError::Internal {
                detail: format!(
                    "utf16 offset {offset} splits a surrogate pair (nearest char boundary is utf16 {roundtrip})"
                ),
            });
        }
        Ok(char_idx)
    }

    /// Reject a UTF-16 offset that does not land on a char boundary (i.e. one
    /// that splits a surrogate pair). Returns `Ok(())` for legal boundaries.
    fn validate_utf16_boundary(&self, offset: u32) -> Result<(), EmendError> {
        self.utf16_to_char(offset).map(|_| ())
    }
}

/// Map a UTF-16/char/line index that does not fit its target integer type into a
/// reportable error. Unreachable for documents within [`Document::MAX_NOTE_BYTES`]
/// (a `u32` of UTF-16 code units covers ~4 GiB), but reported rather than
/// truncated so no `as` cast can silently corrupt an offset.
fn too_large<E: std::fmt::Display>(err: E) -> EmendError {
    EmendError::Internal {
        detail: format!("document index does not fit in target integer type: {err}"),
    }
}

/// Map a [`std::io::Error`] (from the size stat in [`Document::open`]) onto the
/// appropriate [`EmendError`], attaching the offending path. Mirrors the mapping
/// in [`crate::fs`] so `open`'s errors match `read_tolerant`'s.
fn map_io(path: &Path, err: &std::io::Error) -> EmendError {
    let path_str = path.display().to_string();
    match err.kind() {
        std::io::ErrorKind::NotFound => EmendError::NotFound { path: path_str },
        std::io::ErrorKind::PermissionDenied => EmendError::PermissionDenied { path: path_str },
        _ => EmendError::IoFailure {
            path: path_str,
            detail: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    // Unit tests assert on their own fixtures; the workspace denies these in
    // library code, so scope the allowance to this test module.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{Document, LineCol};
    use crate::U16Range;

    #[test]
    fn from_text_then_text_round_trips() {
        let doc = Document::from_text("abc\ndef");
        assert_eq!(doc.text(), "abc\ndef");
    }

    #[test]
    fn empty_document_is_one_line_zero_len() {
        let doc = Document::from_text("");
        assert_eq!(doc.len_utf16(), 0);
        assert_eq!(
            doc.utf16_to_line_col(0).unwrap(),
            LineCol { line: 0, col: 0 }
        );
    }

    #[test]
    fn push_edit_basic_insert() {
        let mut doc = Document::from_text("ac");
        doc.push_edit(U16Range::new(1, 0), "b").unwrap();
        assert_eq!(doc.text(), "abc");
    }

    #[test]
    fn close_consumes_document() {
        let doc = Document::from_text("x");
        doc.close(); // Compile-time proof the explicit close API exists.
    }
}
