//! T108 — derived document **stats**, **task N-of-M**, and **outline** (US6 ·
//! FR-029/030/031a; FFI contract §4 `outline`/`stats`).
//!
//! These are the non-AI "understand a document at a glance" insights:
//!
//! * [`emend_core::derived::stats`] — word count, character count, and estimated
//!   reading time (FR-029), plus task completion N-of-M (FR-030), folded into one
//!   [`DocStats`] so the info sidebar pulls a single value (FFI contract §4
//!   `stats`).
//! * [`emend_core::derived::outline`] — the heading tree, each carrying its
//!   **source line number** so the editor can scroll to the heading on click
//!   (FR-031/031a; FFI contract §4 `outline`).
//!
//! The computation is **pure** over the document source string, so it is tested
//! here with plain `cargo test` — no FFI, no async (Constitution V). The live
//! push (≤300 ms after an edit, FR-031a) is wired FFI-side over the recompute;
//! these tests assert the recompute itself is correct and cheap.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::derived::{outline, stats, DocStats, OutlineItem};

// -- Word / character counts (FR-029) ---------------------------------------

#[test]
fn counts_words_and_chars_for_plain_prose() {
    let src = "The quick brown fox\njumps over the lazy dog.\n";
    let s = stats(src);
    // Nine words across the two lines.
    assert_eq!(s.words, 9, "word count over prose: {s:?}");
    // Character count is the document's char count (not bytes, not UTF-16).
    assert_eq!(s.chars, src.chars().count() as u32, "char count: {s:?}");
}

#[test]
fn word_count_ignores_markdown_punctuation_runs() {
    // Heading hashes, emphasis markers, and list bullets are not words.
    let src = "# Title\n\n- **bold** item\n- plain item\n";
    let s = stats(src);
    // Words: Title, bold, item, plain, item = 5. The `#`, `-`, `**` markers
    // must not inflate the count.
    assert_eq!(
        s.words, 5,
        "markdown punctuation must not count as words: {s:?}"
    );
}

#[test]
fn empty_document_has_zero_stats() {
    let s = stats("");
    assert_eq!(s.words, 0);
    assert_eq!(s.chars, 0);
    assert_eq!(s.reading_minutes, 0);
    assert_eq!(s.tasks_total, 0);
    assert_eq!(s.tasks_done, 0);
}

// -- Reading time (FR-029; ~200 wpm, research §D) ---------------------------

#[test]
fn reading_time_uses_200_wpm_rounding_up() {
    // 200 words → exactly 1 minute.
    let exactly_200 = (0..200)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(stats(&exactly_200).reading_minutes, 1, "200 words = 1 min");

    // 201 words → rounds UP to 2 minutes (a partial minute still shows as a
    // minute so a reader is never told "0 min" for non-empty content).
    let just_over = (0..201)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        stats(&just_over).reading_minutes,
        2,
        "201 words rounds up to 2"
    );
}

#[test]
fn any_nonempty_prose_is_at_least_one_minute() {
    // A handful of words is well under a minute, but reading time should never be
    // 0 for non-empty prose (round any partial minute up to 1).
    let s = stats("just a few words here");
    assert_eq!(
        s.reading_minutes, 1,
        "a short doc still reads as 1 min: {s:?}"
    );
}

// -- Task completion N-of-M (FR-030) ----------------------------------------

#[test]
fn counts_completed_and_total_tasks() {
    let src = "\
# Tasks

- [x] done one
- [ ] not done
- [X] done two (capital X)
- [ ] also pending
- not a task
1. [x] ordered done
";
    let s = stats(src);
    assert_eq!(s.tasks_total, 5, "five checkbox lines: {s:?}");
    assert_eq!(
        s.tasks_done, 3,
        "three are checked (x, X, ordered x): {s:?}"
    );
}

#[test]
fn no_tasks_reports_zero_of_zero() {
    let s = stats("# Heading\n\nJust prose, no checkboxes.\n");
    assert_eq!(s.tasks_total, 0);
    assert_eq!(s.tasks_done, 0);
}

#[test]
fn indented_tasks_are_counted() {
    let src = "- [ ] top\n  - [x] nested done\n  - [ ] nested pending\n";
    let s = stats(src);
    assert_eq!(s.tasks_total, 3, "nested tasks count too: {s:?}");
    assert_eq!(s.tasks_done, 1, "one nested task is done: {s:?}");
}

// -- Outline with source line numbers (FR-031/031a) -------------------------

#[test]
fn outline_lists_atx_headings_with_levels_and_line_numbers() {
    let src = "\
# Top

Some intro prose.

## Section A

text

### Subsection

more

## Section B
";
    let items = outline(src);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Top", "Section A", "Subsection", "Section B"],
        "outline lists headings in document order: {items:?}"
    );

    let levels: Vec<u8> = items.iter().map(|i| i.level).collect();
    assert_eq!(
        levels,
        vec![1, 2, 3, 2],
        "heading levels preserved: {items:?}"
    );

    // Line numbers are 1-based source lines so the editor can scroll to them.
    // "# Top" is line 1; "## Section A" is line 5; "### Subsection" is line 9;
    // "## Section B" is line 13.
    let lines: Vec<u32> = items.iter().map(|i| i.line).collect();
    assert_eq!(
        lines,
        vec![1, 5, 9, 13],
        "1-based source line numbers: {items:?}"
    );
}

#[test]
fn outline_strips_trailing_atx_hashes_and_trims_title() {
    // ATX headings may have an optional closing run of `#`; the title is the text
    // between, trimmed.
    let src = "##   Spaced Title   ##\n";
    let items = outline(src);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Spaced Title", "{items:?}");
    assert_eq!(items[0].level, 2);
}

#[test]
fn outline_ignores_hashes_inside_fenced_code_blocks() {
    // A `#` inside a fenced code block is a comment/shell prompt, NOT a heading —
    // it must not appear in the outline.
    let src = "\
# Real Heading

```sh
# this is a shell comment, not a heading
echo hi
```

## Another Real Heading
";
    let items = outline(src);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Real Heading", "Another Real Heading"],
        "fenced-code `#` lines are not headings: {items:?}"
    );
}

#[test]
fn outline_of_document_with_no_headings_is_empty() {
    let items: Vec<OutlineItem> = outline("Just a paragraph.\n\nAnother one.\n");
    assert!(items.is_empty(), "no headings → empty outline: {items:?}");
}

#[test]
fn outline_requires_space_after_hashes() {
    // `#nospace` is not a heading in CommonMark (no space after the run of `#`).
    let src = "#nospace\n\n# real heading\n";
    let items = outline(src);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, vec!["real heading"], "{items:?}");
}

// -- DocStats is the single info-sidebar value (FFI contract §4) -------------

#[test]
fn doc_stats_bundles_every_field() {
    // One call returns words, chars, reading-time, and task N-of-M together so
    // the FFI `stats` export is a single round-trip (FFI contract §4).
    let src = "# Plan\n\n- [x] ship\n- [ ] celebrate\n\nThree more words here.\n";
    let s: DocStats = stats(src);
    assert!(s.words > 0);
    assert!(s.chars > 0);
    assert_eq!(s.tasks_total, 2);
    assert_eq!(s.tasks_done, 1);
}
