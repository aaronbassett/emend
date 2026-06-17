//! T036 — failing-first integration tests for the **incremental editor-highlight
//! engine** (`emend_core::parse::highlight`).
//!
//! These tests pin down the property that makes the per-keystroke highlight hot
//! path affordable (SC-003, research §B1): tree-sitter reparses *incrementally*,
//! so the set of ranges that actually change between the old tree and the
//! reparsed tree (`Tree::changed_ranges`) is **as small as the edit allows** —
//! not the whole document.
//!
//! Two cases, two opposite obligations:
//!
//! 1. **Edit-local change (the common keystroke).** Typing a word *inside* an
//!    existing paragraph does not alter the document's block structure, so the
//!    reported changed range must stay bounded *near the edit* — it must NOT span
//!    the whole document. This is what lets the UI re-attribute only a small slice
//!    of text per keystroke.
//!
//! 2. **Block-structure change invalidates the tail.** Inserting an opening
//!    code fence (```` ``` ````) reinterprets *everything after it* as code, so
//!    the changed range must extend to (near) the end of the document. This proves
//!    the incremental reparse is *correct*, not merely cheap: when an edit really
//!    does change how later text is parsed, the engine reports it.
//!
//! The engine exposes [`Highlighter::apply_edit`], which applies an editor delta
//! (the same `{utf16Range, replacement}` shape the `Document` hot path takes) and
//! returns the changed ranges of the reparse, in **UTF-16 code units** — the unit
//! the FFI boundary speaks (research §A2). The tests assert on those ranges.

// Tests assert on their own fixtures; the workspace denies these lints in library
// code, but a test that cannot unwrap its own known-good values isn't a test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::parse::highlight::Highlighter;
use emend_core::U16Range;

/// UTF-16 length of `s` — the unit changed ranges are reported in. Pure ASCII in
/// these fixtures, so this equals the byte length, but we compute it honestly so
/// the assertions stay correct if a fixture ever gains an astral char.
fn u16_len(s: &str) -> u32 {
    u32::try_from(s.encode_utf16().count()).expect("fixture fits in u32")
}

/// Build a multi-paragraph document large enough that "whole-document" and
/// "edit-local" changed ranges are unambiguously distinguishable.
fn paragraphs(n: usize) -> String {
    let mut doc = String::new();
    for i in 0..n {
        doc.push_str(&format!(
            "Paragraph number {i} has several words of prose in it.\n\n"
        ));
    }
    doc
}

#[test]
fn edit_inside_paragraph_changes_are_local() {
    let text = paragraphs(40);
    let total = u16_len(&text);

    let mut hl = Highlighter::new(&text);

    // Find a caret well inside the document — just after the word "several" in a
    // middle paragraph — and insert a character there. This does not open or
    // close any block, so block structure is unchanged.
    let needle = "several";
    let byte_pos = text
        .match_indices(needle)
        .nth(20)
        .map(|(idx, _)| idx + needle.len())
        .expect("fixture contains the needle many times");
    // ASCII fixture: byte offset == UTF-16 offset up to this point.
    let caret = u32::try_from(byte_pos).expect("offset fits in u32");

    let changed = hl
        .apply_edit(U16Range::new(caret, 0), "X")
        .expect("incremental reparse of a local insert succeeds");

    // There must be at least one changed range, and the union of changed ranges
    // must be a small neighborhood of the edit — emphatically NOT the whole
    // document. We bound it generously (a few lines' worth) to stay robust to
    // tree-sitter's exact granularity while still failing loudly if the engine
    // ever reparses everything.
    assert!(
        !changed.is_empty(),
        "a real edit should produce at least one changed range"
    );

    let lo = changed.iter().map(|r| r.start).min().unwrap();
    let hi = changed.iter().map(|r| r.end()).max().unwrap();
    let span = hi - lo;

    // The edit sits deep in the document, so a whole-document reparse would start
    // at/near 0. Assert the changed region neither starts at the top nor stretches
    // to the bottom, and that its total span is a small fraction of the document.
    let new_total = total + 1; // we inserted one UTF-16 unit
    assert!(
        span < new_total / 4,
        "changed span {span} should be a small fraction of the {new_total}-unit document \
         (edit-local), but it covered a large region — incremental reparse looks broken"
    );
    assert!(
        lo > 0,
        "changed region starts at offset 0 ({lo}); a mid-document edit should not \
         invalidate the head of the document"
    );
    assert!(
        hi < new_total,
        "changed region reaches EOF ({hi} of {new_total}); a mid-document edit that \
         doesn't change block structure should not invalidate the tail"
    );
    // The changed region should actually contain the edit site.
    assert!(
        lo <= caret + 1 && hi >= caret,
        "changed region [{lo}, {hi}) should bracket the edit at {caret}"
    );
}

#[test]
fn opening_a_code_fence_invalidates_the_tail() {
    // A document whose *first* line we will turn into an opening code fence. Once
    // the fence opens, every following line is reinterpreted as code-fence
    // content until a closing fence (there is none here), so the parse of the
    // entire tail changes.
    let body = paragraphs(40);
    let text = format!("intro line\n\n{body}");

    let mut hl = Highlighter::new(&text);
    let total = u16_len(&text);

    // Insert an opening fence at the very top of the document. Prepending
    // "```\n" makes "intro line" (and everything after) code-fence content.
    let changed = hl
        .apply_edit(U16Range::new(0, 0), "```\n")
        .expect("incremental reparse after opening a fence succeeds");

    assert!(
        !changed.is_empty(),
        "opening a code fence must report changed ranges"
    );

    let hi = changed.iter().map(|r| r.end()).max().unwrap();
    let new_total = total + u16_len("```\n");

    // The tail must be invalidated: the maximum changed offset must reach near
    // the end of the (now larger) document. We allow a small slack from the very
    // last byte to stay robust to how tree-sitter terminates the final range.
    let slack = u16_len("Paragraph number 99 has several words of prose in it.\n\n");
    assert!(
        hi + slack >= new_total,
        "opening a fence should invalidate the tail: max changed offset {hi} should reach \
         near the document end {new_total} (within {slack}), but it stopped short — the \
         block-structure change was not propagated to the tail"
    );
}

#[test]
fn highlight_spans_classifies_basic_markdown() {
    // A small sanity check that the engine produces spans for the obvious
    // constructs the spec calls out (FR-010..015). This is not exhaustive — the
    // exact class set is asserted in the unit tests — but it guards the public
    // surface the FFI export (T039) and Swift attributing (T042) will consume.
    let text = "# Heading\n\nSome **bold** and *italic* text.\n";
    let hl = Highlighter::new(text);
    let spans = hl
        .highlight_spans(U16Range::new(0, u16_len(text)))
        .expect("highlighting a small document succeeds");

    assert!(
        !spans.is_empty(),
        "a document with a heading and emphasis should yield highlight spans"
    );

    // Every span must lie within the document.
    let total = u16_len(text);
    for span in &spans {
        assert!(
            span.range.end() <= total,
            "span {:?} runs past the document end {total}",
            span.range
        );
    }
}
