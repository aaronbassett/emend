//! T133 — the dedicated **FR-027a** integration test: max-note-size cap +
//! incremental re-parse on a large document.
//!
//! FR-027a: *"System MUST define a maximum supported note size; beyond it,
//! behavior MUST be graceful (open read-only or refuse with a clear message)
//! rather than hang or exhaust memory. Editing MUST use incremental re-parsing so
//! a single edit does not re-parse the entire document."*
//!
//! The cap is [`Document::MAX_NOTE_BYTES`] (5 MiB), enforced in [`Document::open`]
//! by **stat-before-allocate**: it stats the file and bails with
//! [`EmendError::NoteTooLarge`] *before* a multi-megabyte rope is ever built, so
//! an oversized file is refused rather than OOM'd.
//!
//! This file is deliberately disjoint from `tests/document.rs`, which already
//! pins the basic at-cap / over-cap byte boundary. Here we cover the FR-027a
//! facets `document.rs` does **not**:
//!
//! 1. The "refuse with a clear message" guarantee — the over-cap error carries
//!    the offending **path** and a human-readable `Display` containing
//!    "too large" (not just the numeric `limit`/`bytes` fields).
//! 2. The exact one-byte-over boundary reports `bytes == cap + 1` (proving the
//!    cap is enforced from the *stat*, not from a truncated read), and a file
//!    *exactly* at the cap opens.
//! 3. A large-but-under-cap (~1 MiB) document edits as a **local splice** — a
//!    single `push_edit` changes the length by exactly the inserted delta and
//!    lands the text at the right place, not a full rebuild.
//! 4. The core of FR-027a: a single inserted character on a ~1 MiB document goes
//!    through [`Highlighter::apply_edit`] as an **incremental re-parse** that
//!    succeeds, leaves the highlighter consistent, and keeps it queryable.

// Tests assert on their own fixtures; the workspace denies these in library code,
// but a test that cannot unwrap its own fixtures cannot test. Scoped here, to
// match `tests/document.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::document::Document;
use emend_core::parse::highlight::Highlighter;
use emend_core::{EmendError, U16Range};

// ---------------------------------------------------------------------------
// shared fixtures
// ---------------------------------------------------------------------------

/// Build a Markdown document of roughly `target_bytes` bytes with a realistic mix
/// of headings, prose, emphasis, lists, and the occasional fenced code block, so
/// both the block and inline grammars do non-trivial work. Mirrors the
/// `build_doc` helper in `crates/emend-bench/benches/highlight.rs` so the test
/// and the bench exercise the same shape of document.
fn build_doc(target_bytes: usize) -> String {
    let mut doc = String::with_capacity(target_bytes + 256);
    let mut i = 0usize;
    while doc.len() < target_bytes {
        match i % 12 {
            0 => doc.push_str(&format!("# Section {i}\n\n")),
            6 => doc.push_str("```rust\nfn demo() { let x = 1 + 2; }\n```\n\n"),
            9 => doc.push_str("- a list item with **bold** and *italic* words\n"),
            _ => doc.push_str(
                "This is paragraph text with some **strong** and *emphasised* words, \
                 a `code span`, and a [link](https://example.com) to round it out.\n\n",
            ),
        }
        i += 1;
    }
    doc
}

/// A char-boundary UTF-16 offset roughly in the middle of `text`, to use as a
/// mid-document edit site. Falls back to 0 for an empty document.
fn mid_caret_utf16(text: &str) -> u32 {
    let mut idx = text.len() / 2;
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    let utf16 = text[..idx].encode_utf16().count();
    u32::try_from(utf16).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 1 + 2. Boundary cap behaviour: refuse-with-a-clear-message vs open-at-cap
// ---------------------------------------------------------------------------

/// FR-027a "refuse with a clear message / graceful, not OOM": a file **one byte
/// over** the cap is rejected with [`EmendError::NoteTooLarge`] whose `limit`
/// equals the cap and whose reported `bytes` is exactly `cap + 1`. The exact
/// `bytes == cap + 1` proves the size came from the **stat** (stat-before-
/// allocate) — a truncated/partial read could not report the true over-cap size.
/// The error also carries the offending `path` and a human-readable message
/// containing "too large" (the "clear message" half of the requirement), which
/// `tests/document.rs` does not assert.
#[test]
fn one_byte_over_cap_is_refused_with_a_clear_message() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("one-over.md");

    let cap = Document::MAX_NOTE_BYTES;
    let over = usize::try_from(cap + 1).expect("cap + 1 fits usize on this target");
    std::fs::write(&path, vec![b'a'; over]).expect("write over-cap fixture");

    let err = Document::open(&path).expect_err("a file one byte over the cap must be refused");
    match &err {
        EmendError::NoteTooLarge {
            path: p,
            bytes,
            limit,
        } => {
            // Graceful refusal contract (FR-027a).
            assert_eq!(*limit, cap, "reported limit must be MAX_NOTE_BYTES");
            assert_eq!(
                *bytes,
                cap + 1,
                "reported size must be exactly cap+1 (proves stat-before-allocate)"
            );
            // The offending path is carried through for the UI's error message.
            assert!(
                p.contains("one-over.md"),
                "error should name the offending file, got {p:?}"
            );
        }
        other => panic!("expected NoteTooLarge, got {other:?}"),
    }

    // "clear message" half of FR-027a: the Display is human-readable and says so.
    let message = err.to_string();
    assert!(
        message.contains("too large"),
        "Display should be a clear 'too large' message, got {message:?}"
    );
    assert!(
        message.contains("one-over.md"),
        "Display should name the offending file, got {message:?}"
    );
}

/// FR-027a: the cap is **inclusive** — a file of *exactly* `MAX_NOTE_BYTES` opens
/// normally (the just-under-or-equal side of the boundary the over-cap test
/// pins). `tests/document.rs` checks the at-cap length; here we additionally
/// confirm the at-cap document is a real, queryable [`Document`] (its text round
/// trips to the expected byte length), to keep this file's coverage distinct.
#[test]
fn exactly_at_cap_opens_and_is_usable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("at-cap.md");

    let cap = Document::MAX_NOTE_BYTES;
    let at = usize::try_from(cap).expect("cap fits usize on this target");
    std::fs::write(&path, vec![b'a'; at]).expect("write at-cap fixture");

    let doc = Document::open(&path).expect("a file exactly at the cap must open");
    // All bytes are ASCII 'a', so byte length == char length == UTF-16 length.
    assert_eq!(u64::from(doc.len_utf16()), cap, "at-cap UTF-16 length");
    assert_eq!(doc.text().len(), at, "at-cap text round-trips to cap bytes");
}

// ---------------------------------------------------------------------------
// 3. Large-but-under-cap document edits as a local splice
// ---------------------------------------------------------------------------

/// A ~1 MiB in-memory document applies a single mid-document `push_edit` as a
/// **local splice**: the UTF-16 length grows by exactly the inserted delta and
/// the inserted marker appears at the edit site, with the text immediately before
/// and after the splice preserved. This is the document-side half of FR-027a's
/// "a single edit does not re-parse the entire document" — the rope edit is
/// O(log n), not an O(n) rebuild.
#[test]
fn large_under_cap_document_edits_as_a_local_splice() {
    let doc_text = build_doc(1024 * 1024);
    let mut doc = Document::from_text(&doc_text);

    // `len_utf16()` is correct for the (ASCII-only) fixture: every byte is one
    // UTF-16 code unit, so the lengths coincide and fit in u32 (well under the
    // ~4 GiB a u32 of code units covers, and under MAX_NOTE_BYTES).
    let expected_len = u32::try_from(doc_text.encode_utf16().count()).expect("len fits u32");
    assert_eq!(doc.len_utf16(), expected_len, "initial UTF-16 length");

    // Insert a distinctive marker at a char-boundary offset mid-document.
    let caret = mid_caret_utf16(&doc_text);
    let marker = "[X]";
    let delta = u32::try_from(marker.encode_utf16().count()).expect("delta fits u32");
    doc.push_edit(U16Range::new(caret, 0), marker)
        .expect("mid-document insert should succeed");

    // (a) Length changed by *exactly* the inserted delta — a splice, not a
    //     rebuild that might normalise/re-emit surrounding text.
    assert_eq!(
        doc.len_utf16(),
        expected_len + delta,
        "length must grow by exactly the inserted UTF-16 delta"
    );

    // (b) The spliced text appears at the right place. The fixture is ASCII, so
    //     UTF-16 offsets equal byte offsets here; slice the post-edit text around
    //     the caret and assert the marker sits between the original neighbours.
    let post = doc.text();
    let caret_usize = usize::try_from(caret).expect("caret fits usize");
    let before = &doc_text[..caret_usize];
    let after = &doc_text[caret_usize..];
    let expected = format!("{before}{marker}{after}");
    assert_eq!(
        post, expected,
        "single edit must be a local splice at the caret"
    );
}

// ---------------------------------------------------------------------------
// 4. Incremental re-parse on a large document (the core of FR-027a)
// ---------------------------------------------------------------------------

/// The heart of FR-027a: on a ~1 MiB document, a single inserted character goes
/// through [`Highlighter::apply_edit`] — the **incremental** tree-sitter reparse
/// path — and:
///
/// - returns `Ok(..)` (the edit is accepted, not rejected as out-of-bounds);
/// - reports a non-empty set of changed ranges, each bounded **within** the
///   document (so the UI has something concrete to re-attribute);
/// - leaves the highlighter consistent and **queryable**: `highlight_spans` over
///   a viewport around the edit still returns `Ok`.
///
/// On the changed-range *size*: this asserts only the weaker-but-true property
/// (each changed range lies within the document). It does **not** assert the
/// stronger "changed range ≪ whole document" property, because the API does not
/// guarantee it: `tree-sitter-md` is a split parser whose wrapper rebuilds *all*
/// inline sub-trees on every parse, and a block-structure change yields a
/// tail-spanning changed range (see `highlight.rs::changed_byte_ranges` and the
/// note in `benches/highlight.rs`). Incrementality here is a property of the
/// *reparse work* (the old tree is reused as a baseline), not a hard bound on the
/// returned ranges — so we assert what the implementation can actually promise.
#[test]
fn incremental_reparse_on_large_document_succeeds_and_stays_queryable() {
    let doc_text = build_doc(1024 * 1024);
    let total_before = u32::try_from(doc_text.encode_utf16().count()).expect("len fits u32");

    // Initial whole-document parse (cheap relative to the per-keystroke path).
    let mut hl = Highlighter::new(&doc_text);

    // Insert a single character mid-document — one keystroke's worth of work.
    let caret = mid_caret_utf16(&doc_text);
    let changed = hl
        .apply_edit(U16Range::new(caret, 0), "z")
        .expect("a single mid-document insert must reparse without error");

    // The edit reports at least the touched span; every reported range is bounded
    // within the now-longer document (start <= end <= new length).
    assert!(
        !changed.is_empty(),
        "an accepted edit should report at least one changed range"
    );
    let total_after = total_before + 1; // inserted one ASCII (one-UTF-16-unit) char.
    for r in &changed {
        assert!(
            r.start <= r.end(),
            "changed range must not be inverted: {r:?}"
        );
        assert!(
            r.end() <= total_after,
            "changed range {r:?} must lie within the document (len {total_after})"
        );
    }

    // The highlighter stays queryable after the incremental reparse: a viewport
    // around the edit returns spans without error, all within the document.
    let view_half = 1_000u32;
    let view_start = caret.saturating_sub(view_half);
    let view_len = (view_half * 2).min(total_after - view_start);
    let viewport = U16Range::new(view_start, view_len);
    let spans = hl
        .highlight_spans(viewport)
        .expect("querying spans after an incremental edit must succeed");
    for s in &spans {
        assert!(
            s.range.end() <= total_after,
            "span past EOF after edit: {s:?}"
        );
    }
}
