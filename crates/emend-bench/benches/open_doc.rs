//! T131 — cold-open benchmark (SC-002, tracked / non-blocking).
//!
//! Budget under test: opening a large document (~1 MB / ~10k lines) shows
//! visible rendered content within **500 ms (p95)**. The core-measurable portion
//! before first paint is the *cold open*: build the document substrate
//! ([`Document::from_text`], an O(n) rope build) **plus** the initial
//! whole-document parse ([`Highlighter::new`]) that produces the first set of
//! highlight spans. That pair is the dominant compute the UI waits on before it
//! can paint, so it is exactly what this bench times.
//!
//! Per the constitution, perf budgets are *tracked but non-blocking*: this bench
//! exists to surface regressions, not to fail CI. The measured number is recorded
//! in the implementation report alongside the `highlight` (SC-003) and
//! `quick_open` (SC-004) benches.
//!
//! ## What is and isn't measured
//!
//! Building the document *string* is **setup** (`iter_batched`), so it is
//! excluded from the timing — only the open+parse work is measured. We measure
//! the **in-memory** open ([`Document::from_text`]) rather than the on-disk
//! [`Document::open`] deliberately: `open` adds a `std::fs` read that is
//! OS-page-cache-dominated and not the budgeted *compute* before first paint
//! (and wiring a tempfile into the timed routine would only measure the kernel's
//! cache, not the rope build). The rope build that `open` performs *after* the
//! read is identical to `from_text`, so this captures the real cold-open cost.
//!
//! Keep this file free of `unwrap`/`expect`/`panic` (workspace lint policy,
//! NFR-003): we degrade to defaults instead of unwrapping, and feed every output
//! through `black_box` so the optimiser cannot delete the work.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use emend_core::document::Document;
use emend_core::parse::highlight::Highlighter;
use emend_core::U16Range;

/// Build a Markdown document of roughly `target_bytes` bytes (~1 MB by default),
/// with a realistic mix of headings, prose, emphasis, lists, and the occasional
/// fenced code block so the initial parse does non-trivial work across both the
/// block and inline grammars. Identical to the `build_doc` in
/// `benches/highlight.rs` so the two benches measure the same shape of document.
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

/// One measured cold open of a document of `target_bytes`: the in-memory open
/// ([`Document::from_text`]) plus the initial whole-document parse
/// ([`Highlighter::new`]) plus one whole-document span query — the work the UI
/// runs before it can paint visible content (SC-002).
///
/// The doc string is built as untimed setup; the timed routine threads the same
/// text through both the rope build and the parse, then black-boxes a span count
/// so neither side is optimised away.
fn bench_one_cold_open(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    label: &str,
    target_bytes: usize,
) {
    group.bench_function(label, |b| {
        b.iter_batched(
            // SETUP (not timed): just the document text.
            || build_doc(target_bytes),
            // ROUTINE (timed): the cold open — rope build + initial parse + a
            // first whole-document span query (the first paint's worth of spans).
            |doc_text| {
                // Document substrate (O(n) rope build).
                let document = Document::from_text(black_box(&doc_text));
                let len = document.len_utf16();
                // Initial whole-document parse (the dominant first-paint cost).
                let hl = Highlighter::new(black_box(&doc_text));
                // First viewport query — here the whole document, the upper bound
                // on what a first paint could ask for.
                let spans = hl.highlight_spans(U16Range::new(0, len));
                // Defeat dead-code elimination without unwrapping the (here
                // unreachable) error path.
                black_box(len);
                black_box(&document);
                black_box(spans.map(|s| s.len()).unwrap_or(0));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_cold_open(c: &mut Criterion) {
    let mut group = c.benchmark_group("open_doc");
    // Each 1 MB sample builds a rope and parses the whole document, so cap the
    // sample count low to keep the bench prompt while staying statistically
    // useful; the small-doc case is cheap and shares the same low count.
    group.sample_size(10);
    // 1 MB / ~10k lines — the worst case the budget is stated against (SC-002).
    bench_one_cold_open(&mut group, "open_and_parse_1mb", 1024 * 1024);
    // ~64 KB — a large-but-typical note, for signal at the common-case end.
    bench_one_cold_open(&mut group, "open_and_parse_64kb", 64 * 1024);
    group.finish();
}

criterion_group!(benches, bench_cold_open);
criterion_main!(benches);
