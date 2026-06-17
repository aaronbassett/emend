//! T037 — incremental editor-highlight benchmark (SC-003, tracked / non-blocking).
//!
//! Budget under test: re-highlight **one edited line** in a ~1 MB / ~10k-line
//! Markdown document in **< 5 ms**. This exercises the per-keystroke hot path the
//! real editor takes — an incremental tree-sitter reparse driven by a tiny delta,
//! followed by a viewport span query.
//!
//! Per the constitution, perf budgets are *tracked but non-blocking*: this bench
//! exists to surface regressions, not to fail CI. The measured number is recorded
//! in the implementation report.
//!
//! ## What is and isn't measured
//!
//! Building the document and the initial whole-document parse are **setup**
//! (`iter_batched` builds a fresh [`Highlighter`] per measured iteration), so they
//! are excluded from the timing. The measured routine is exactly one `apply_edit`
//! (insert a character mid-line — the incremental reparse) followed by one
//! `highlight_spans` over a screen-sized viewport around the edit. That pair is
//! what the UI runs on each keystroke.
//!
//! Keep this file free of `unwrap`/`expect`/`panic` (workspace lint policy,
//! NFR-003): we degrade to defaults instead of unwrapping, and feed every output
//! through `black_box` so the optimiser cannot delete the work.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use emend_core::parse::highlight::Highlighter;
use emend_core::U16Range;

/// Build a Markdown document of roughly `target_bytes` bytes (~1 MB by default),
/// with a realistic mix of headings, prose, emphasis, lists, and the occasional
/// fenced code block so the parse does non-trivial work across both the block and
/// inline grammars.
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

/// Find a byte offset roughly in the middle of the document that sits on a char
/// boundary, to use as the edit site. Falls back to 0 if the document is empty.
fn mid_caret_utf16(text: &str) -> u32 {
    // Aim for the middle, then walk forward to the next char boundary.
    let mut idx = text.len() / 2;
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    // Convert the byte offset to a UTF-16 offset (the unit the API takes).
    let utf16 = text[..idx].encode_utf16().count();
    u32::try_from(utf16).unwrap_or(0)
}

/// One measured keystroke against a document of `target_bytes`: an incremental
/// reparse plus a viewport span query — the exact pair the editor runs per key.
///
/// Two sizes are benched so the tracked metric carries signal at both ends:
/// - **1 MB / ~20k lines** — the worst case the budget is stated against (SC-003).
/// - **~64 KB** — a large-but-typical note, where the per-keystroke cost is what
///   most users actually experience.
///
/// NOTE (perf characterisation, see report): the cost here is dominated by
/// `apply_edit`, not the span query. The `tree-sitter-md` split-parser wrapper
/// rebuilds *all* inline sub-trees on every parse, so the reparse is O(blocks),
/// i.e. ~linear in document size (~0.4 ms/KB on this machine) rather than
/// O(edit). The viewport query itself is sub-millisecond.
fn bench_one_keystroke(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    label: &str,
    target_bytes: usize,
) {
    let doc = build_doc(target_bytes);
    let caret = mid_caret_utf16(&doc);

    // A screen-sized viewport (~2k UTF-16 units) centred on the edit. This is the
    // kind of slice the editor actually re-attributes after a keystroke.
    let view_half = 1000u32;
    let view_start = caret.saturating_sub(view_half);
    let viewport = U16Range::new(view_start, view_half * 2);

    group.bench_function(label, |b| {
        b.iter_batched(
            // SETUP (not timed): a fresh highlighter with the document parsed.
            || Highlighter::new(&doc),
            // ROUTINE (timed): one keystroke's worth of work.
            |mut hl| {
                // Incremental reparse for a single inserted character.
                let changed = hl.apply_edit(black_box(U16Range::new(caret, 0)), black_box("z"));
                // Span query over the viewport around the edit.
                let spans = hl.highlight_spans(black_box(viewport));
                // Defeat dead-code elimination; tolerate the (unreachable here)
                // error path without unwrapping.
                black_box(changed.map(|c| c.len()).unwrap_or(0));
                black_box(spans.map(|s| s.len()).unwrap_or(0));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_highlight_one_line_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("highlight");
    // Each 1 MB sample reparses the whole document (see note above), so cap the
    // sample count low to keep the bench prompt while staying statistically
    // useful; the small-doc case is cheap and uses the default count.
    group.sample_size(10);
    bench_one_keystroke(&mut group, "incremental_edit_1mb", 1024 * 1024);
    bench_one_keystroke(&mut group, "incremental_edit_64kb", 64 * 1024);
    group.finish();
}

criterion_group!(benches, bench_highlight_one_line_edit);
criterion_main!(benches);
