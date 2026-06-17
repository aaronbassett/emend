//! T018 — failing-first tests for the open-document model (`emend_core::document`).
//!
//! These tests pin down the **UTF-16 boundary** that the per-keystroke hot path
//! depends on (research §A2/§A3, FFI contract §3). Correctness here is critical:
//! every range crossing the FFI boundary is expressed in **UTF-16 code units**
//! (to map 1:1 onto `NSRange`), while the shadow [`ropey::Rope`] indexes in
//! *chars* (Unicode scalar values). A single off-by-one in the UTF-16↔char
//! conversion corrupts the user's buffer, so the conversions are exercised hard
//! around the two places they diverge from a naive byte/char model:
//!
//! 1. **Astral characters** — any scalar above U+FFFF (e.g. the emoji "😀",
//!    U+1F600) is **one `char` but two UTF-16 code units** (a surrogate pair).
//!    So `len_chars()` and `len_utf16()` disagree, and a UTF-16 offset that
//!    lands *between* the two surrogate halves is meaningless. We assert the
//!    mapping is correct on both sides of such a scalar and that edits spanning
//!    it splice cleanly.
//!
//! 2. **Line breaks** — `\n` and `\r\n`. The model recognizes **only LF and
//!    CRLF** as line breaks (ropey built with `unicode_lines`/`cr_lines` OFF, to
//!    match NSTextView/TextKit line semantics). A CRLF is a *single* line break.
//!    `(line, col)` columns are themselves UTF-16 code units within the line.
//!
//! The other obligations covered:
//! - `push_edit` (insert / delete / replace, at start / middle / end, and
//!   spanning an astral char) mutates the shadow rope to *exactly* the expected
//!   post-edit string, and the UTF-16 length + line index stay correct after.
//! - `open` reads via the tolerant path and enforces the max-note-size cap
//!   (FR-027a): a file over the limit returns [`EmendError::NoteTooLarge`] so the
//!   caller can fall back to read-only.

// Tests assert on known-good fixtures; the workspace denies these in library
// code, but a test that cannot unwrap its own fixtures cannot test. Scoped here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::document::{Document, LineCol};
use emend_core::{EmendError, U16Range};

// "😀" (U+1F600 GRINNING FACE): 1 char / 1 scalar, **2 UTF-16 code units**,
// 4 UTF-8 bytes. The canonical astral-plane test character.
const EMOJI: &str = "😀";

// ---------------------------------------------------------------------------
// UTF-16 length vs char length (the core invariant)
// ---------------------------------------------------------------------------

/// ASCII: one UTF-16 code unit per char, so the two lengths agree.
#[test]
fn ascii_utf16_len_matches_char_len() {
    let doc = Document::from_text("hello");
    assert_eq!(doc.len_utf16(), 5);
}

/// An astral char is two UTF-16 code units even though it is a single scalar:
/// "a😀b" is 3 chars but 4 UTF-16 code units (1 + 2 + 1).
#[test]
fn astral_char_counts_as_two_utf16_code_units() {
    let doc = Document::from_text(&format!("a{EMOJI}b"));
    assert_eq!(doc.len_utf16(), 4);
}

/// A BMP-but-multibyte char (e.g. "é", U+00E9: 2 UTF-8 bytes, 1 UTF-16 unit)
/// must still count as a single UTF-16 code unit — UTF-16 length is not byte
/// length.
#[test]
fn bmp_multibyte_char_is_one_utf16_code_unit() {
    let doc = Document::from_text("é"); // 2 bytes, 1 scalar, 1 UTF-16 unit
    assert_eq!(doc.len_utf16(), 1);
}

// ---------------------------------------------------------------------------
// UTF-16 offset ↔ (line, col) round-trip, including astral chars and CRLF
// ---------------------------------------------------------------------------

/// Round-trips every UTF-16 offset `0..=len` through (line, col) and back. This
/// is the strongest single check: offset → (line,col) → offset must be the
/// identity at *every* valid boundary, including the position just past the last
/// code unit. Offsets that fall *inside* an astral surrogate pair are excluded
/// (they are not valid boundaries and are tested separately).
fn assert_offset_roundtrip(text: &str) {
    let doc = Document::from_text(text);
    let len = doc.len_utf16();
    for off in 0..=len {
        // Skip offsets that split an astral pair: those are not legal boundaries.
        if doc.utf16_to_line_col(off).is_err() {
            continue;
        }
        let lc = doc
            .utf16_to_line_col(off)
            .unwrap_or_else(|e| panic!("offset {off} → (line,col) failed: {e}"));
        let back = doc
            .line_col_to_utf16(lc)
            .unwrap_or_else(|e| panic!("(line,col) {lc:?} → offset failed: {e}"));
        assert_eq!(
            back, off,
            "round-trip mismatch at utf16 offset {off}: {lc:?}"
        );
    }
}

#[test]
fn roundtrip_plain_single_line() {
    assert_offset_roundtrip("hello world");
}

#[test]
fn roundtrip_multi_line_lf() {
    assert_offset_roundtrip("alpha\nbeta\ngamma");
}

#[test]
fn roundtrip_with_astral_chars() {
    // Astral chars on either side of a line break exercise the surrogate-pair
    // accounting in both the offset→(line,col) and the line-start indexing.
    assert_offset_roundtrip(&format!("a{EMOJI}b\nc{EMOJI}{EMOJI}d"));
}

#[test]
fn roundtrip_with_crlf() {
    // CRLF is a single break; the column accounting must not desync across it.
    assert_offset_roundtrip("one\r\ntwo\r\nthree");
}

#[test]
fn roundtrip_empty_document() {
    assert_offset_roundtrip("");
}

#[test]
fn roundtrip_trailing_newline() {
    // A trailing LF means an empty final line; offset==len lands on (line N, 0).
    assert_offset_roundtrip("x\n");
}

// ---------------------------------------------------------------------------
// Specific (line, col) values around astral chars and line breaks
// ---------------------------------------------------------------------------

#[test]
fn line_col_columns_are_utf16_units_not_chars() {
    // "a😀b": columns 0=a, 1=start-of-emoji, 3=b (emoji spans cols 1..3).
    let doc = Document::from_text(&format!("a{EMOJI}b"));
    assert_eq!(
        doc.utf16_to_line_col(0).unwrap(),
        LineCol { line: 0, col: 0 }
    );
    assert_eq!(
        doc.utf16_to_line_col(1).unwrap(),
        LineCol { line: 0, col: 1 }
    );
    // Offset 3 is *after* the emoji's two code units → column 3, not column 2.
    assert_eq!(
        doc.utf16_to_line_col(3).unwrap(),
        LineCol { line: 0, col: 3 }
    );
}

#[test]
fn offset_inside_surrogate_pair_is_rejected() {
    // "😀": the single legal interior boundary set is {0, 2}; offset 1 lands
    // between the high and low surrogate and must be rejected, not silently
    // rounded — corrupting the buffer is worse than erroring.
    let doc = Document::from_text(EMOJI);
    assert!(doc.utf16_to_line_col(0).is_ok());
    assert!(matches!(
        doc.utf16_to_line_col(1),
        Err(EmendError::Internal { .. })
    ));
    assert!(doc.utf16_to_line_col(2).is_ok());
}

#[test]
fn second_line_starts_after_lf() {
    let doc = Document::from_text("ab\ncd");
    // offset 3 is the 'c' at the start of line 1.
    assert_eq!(
        doc.utf16_to_line_col(3).unwrap(),
        LineCol { line: 1, col: 0 }
    );
    assert_eq!(
        doc.line_col_to_utf16(LineCol { line: 1, col: 0 }).unwrap(),
        3
    );
}

#[test]
fn crlf_counts_as_single_break_with_two_code_units() {
    // "a\r\nb": offsets — 0:a, 1:\r, 2:\n, 3:b. The 'b' is line 1, col 0, and
    // the break itself occupies two UTF-16 code units on line 0.
    let doc = Document::from_text("a\r\nb");
    assert_eq!(doc.len_utf16(), 4);
    assert_eq!(
        doc.utf16_to_line_col(3).unwrap(),
        LineCol { line: 1, col: 0 }
    );
    // 'a' through the '\r\n' are all on line 0.
    assert_eq!(doc.utf16_to_line_col(0).unwrap().line, 0);
    assert_eq!(doc.utf16_to_line_col(1).unwrap().line, 0);
}

#[test]
fn out_of_bounds_offset_errors() {
    let doc = Document::from_text("abc"); // len_utf16 == 3, so 3 is valid (EOF), 4 is not.
    assert!(doc.utf16_to_line_col(3).is_ok());
    assert!(matches!(
        doc.utf16_to_line_col(4),
        Err(EmendError::Internal { .. })
    ));
}

#[test]
fn out_of_range_line_col_errors() {
    let doc = Document::from_text("ab\ncd");
    // Line 9 does not exist.
    assert!(matches!(
        doc.line_col_to_utf16(LineCol { line: 9, col: 0 }),
        Err(EmendError::Internal { .. })
    ));
    // Column past the end of line 0 ("ab" → cols 0..=2) is out of range.
    assert!(matches!(
        doc.line_col_to_utf16(LineCol { line: 0, col: 9 }),
        Err(EmendError::Internal { .. })
    ));
}

// ---------------------------------------------------------------------------
// push_edit — delta application produces exactly the expected string
// ---------------------------------------------------------------------------

/// Apply one `{range, replacement}` delta and assert the whole buffer text, the
/// UTF-16 length, and that the line index still round-trips afterward.
fn apply_and_check(initial: &str, range: U16Range, replacement: &str, expected: &str) {
    let mut doc = Document::from_text(initial);
    doc.push_edit(range, replacement)
        .unwrap_or_else(|e| panic!("push_edit failed: {e}"));
    assert_eq!(doc.text(), expected, "post-edit text mismatch");

    // UTF-16 length must equal the expected string's own UTF-16 length.
    let expected_len: u32 = expected
        .encode_utf16()
        .count()
        .try_into()
        .expect("fixture fits in u32");
    assert_eq!(
        doc.len_utf16(),
        expected_len,
        "post-edit utf16 len mismatch"
    );

    // The line index must remain consistent: every boundary still round-trips.
    let len = doc.len_utf16();
    for off in 0..=len {
        if let Ok(lc) = doc.utf16_to_line_col(off) {
            assert_eq!(
                doc.line_col_to_utf16(lc).unwrap(),
                off,
                "line index desynced after edit at offset {off}"
            );
        }
    }
}

#[test]
fn push_edit_insert_at_start() {
    apply_and_check("world", U16Range::new(0, 0), "hello ", "hello world");
}

#[test]
fn push_edit_insert_in_middle() {
    apply_and_check("ac", U16Range::new(1, 0), "b", "abc");
}

#[test]
fn push_edit_insert_at_end() {
    let doc_len = "abc".encode_utf16().count() as u32;
    apply_and_check("abc", U16Range::new(doc_len, 0), "!", "abc!");
}

#[test]
fn push_edit_delete_range() {
    // Remove "BCD" from "aBCDe".
    apply_and_check("aBCDe", U16Range::new(1, 3), "", "ae");
}

#[test]
fn push_edit_replace_range() {
    // Replace "lo wor" with "y, gorgeous w".
    apply_and_check("hello world", U16Range::new(3, 6), "p ", "help ld");
}

#[test]
fn push_edit_replace_whole_document() {
    let len = "old text".encode_utf16().count() as u32;
    apply_and_check("old text", U16Range::new(0, len), "new", "new");
}

#[test]
fn push_edit_insert_astral_char() {
    // Insert an emoji between 'a' and 'b'; length grows by 2 UTF-16 units.
    apply_and_check("ab", U16Range::new(1, 0), EMOJI, &format!("a{EMOJI}b"));
}

#[test]
fn push_edit_delete_astral_char() {
    // Delete the emoji (2 UTF-16 units at offset 1) from "a😀b".
    apply_and_check(&format!("a{EMOJI}b"), U16Range::new(1, 2), "", "ab");
}

#[test]
fn push_edit_replace_spanning_astral_char() {
    // Replace "x😀" (offsets 0..3 = 3 UTF-16 units) with "Z".
    apply_and_check(&format!("x{EMOJI}y"), U16Range::new(0, 3), "Z", "Zy");
}

#[test]
fn push_edit_across_line_break() {
    // Join two lines: delete the '\n' at offset 5 in "alpha\nbeta".
    apply_and_check("alpha\nbeta", U16Range::new(5, 1), "", "alphabeta");
}

#[test]
fn push_edit_sequence_of_keystrokes() {
    // Simulate per-keystroke typing of "Hi😀!" one delta at a time.
    let mut doc = Document::from_text("");
    doc.push_edit(U16Range::new(0, 0), "H").unwrap();
    doc.push_edit(U16Range::new(1, 0), "i").unwrap();
    doc.push_edit(U16Range::new(2, 0), EMOJI).unwrap();
    // After the emoji, the next insert point is offset 4 (1 + 1 + 2).
    doc.push_edit(U16Range::new(4, 0), "!").unwrap();
    assert_eq!(doc.text(), format!("Hi{EMOJI}!"));
    assert_eq!(doc.len_utf16(), 5);
}

#[test]
fn push_edit_out_of_bounds_errors_without_mutating() {
    let mut doc = Document::from_text("abc");
    // Range end (5) past EOF (3) must error and leave the buffer untouched.
    assert!(matches!(
        doc.push_edit(U16Range::new(2, 3), "x"),
        Err(EmendError::Internal { .. })
    ));
    assert_eq!(doc.text(), "abc", "failed edit must not mutate the buffer");
}

#[test]
fn push_edit_split_surrogate_range_errors() {
    // A range whose start lands inside an astral surrogate pair is illegal.
    let mut doc = Document::from_text(EMOJI);
    assert!(matches!(
        doc.push_edit(U16Range::new(1, 1), "x"),
        Err(EmendError::Internal { .. })
    ));
    assert_eq!(doc.text(), EMOJI, "failed edit must not mutate the buffer");
}

// ---------------------------------------------------------------------------
// open() — tolerant read + size cap (FR-027a)
// ---------------------------------------------------------------------------

#[test]
fn open_reads_file_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    std::fs::write(&path, "# Title\n\nbody\n").unwrap();

    let doc = Document::open(&path).unwrap();
    assert_eq!(doc.text(), "# Title\n\nbody\n");
}

#[test]
fn open_strips_bom_via_tolerant_read() {
    // open() must go through fs::read_tolerant, which strips a leading BOM.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bom.md");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice("hi".as_bytes());
    std::fs::write(&path, bytes).unwrap();

    let doc = Document::open(&path).unwrap();
    assert_eq!(doc.text(), "hi");
}

#[test]
fn open_missing_file_maps_to_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.md");
    assert!(matches!(
        Document::open(&path),
        Err(EmendError::NotFound { .. })
    ));
}

#[test]
fn open_over_size_cap_returns_note_too_large() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.md");

    // Write just over the cap so the cap is enforced before the rope is built.
    let over = (Document::MAX_NOTE_BYTES + 1) as usize;
    let big = vec![b'a'; over];
    std::fs::write(&path, big).unwrap();

    match Document::open(&path) {
        Err(EmendError::NoteTooLarge { bytes, limit, .. }) => {
            assert_eq!(limit, Document::MAX_NOTE_BYTES);
            assert!(bytes > limit, "reported size must exceed the limit");
        }
        other => panic!("expected NoteTooLarge, got {other:?}"),
    }
}

#[test]
fn open_at_exactly_the_cap_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("atcap.md");

    // Exactly the cap (boundary) must be allowed — the cap is inclusive.
    let at = Document::MAX_NOTE_BYTES as usize;
    let body = vec![b'a'; at];
    std::fs::write(&path, body).unwrap();

    let doc = Document::open(&path).unwrap();
    assert_eq!(doc.len_utf16(), Document::MAX_NOTE_BYTES as u32);
}
