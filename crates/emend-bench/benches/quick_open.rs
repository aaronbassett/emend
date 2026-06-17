//! T071 — Quick Open search benchmark (SC-004, tracked / non-blocking).
//!
//! Budget under test: a Quick Open query over a **10k-entry index** clears in
//! **≤ 100 ms p95 warm** (FR-018 / SC-004). This exercises the exact pure path
//! the FFI `quick_open_query` worker runs per keystroke: rank the query over the
//! [`Index`](emend_core::index::Index) and stream the ranked batches through the
//! core driver [`quick_open`](emend_core::search::quick_open).
//!
//! Per the constitution, perf budgets are *tracked but non-blocking*: this bench
//! surfaces regressions, it does not fail CI. The measured number is recorded in
//! the implementation report. Criterion's default estimate is the **mean**; the
//! HTML/JSON reports also carry the slope and confidence interval. With a budget
//! this generous (the warm query is expected to land far under 100 ms) the mean
//! is a faithful proxy for p95 here; if a future regression pushes it close,
//! switch the metric to an explicit quantile.
//!
//! ## What is and isn't measured
//!
//! Building the 10k-entry index is **setup** (built once, outside the timed
//! loop), so the "warm" query is what's measured: a single
//! `emend_core::search::quick_open` call that ranks the haystack and drains the
//! ranked results through a no-op (black-boxed) sink — the cost the user pays on
//! a keystroke once the index is populated.
//!
//! Three query shapes are benched because fuzzy-match cost varies with how
//! selective the needle is:
//! - **`"note"`** — a common substring matching *every* entry (the worst case:
//!   the ranker scores and sorts the whole haystack and the full result set
//!   streams, capped by `limit`).
//! - **`"note-7777"`** — a near-exact needle matching a handful (typical "I know
//!   roughly what I want" typing).
//! - **`"zzqq"`** — a no-match needle (the ranker still scores every entry but
//!   emits nothing — pure scoring cost).
//!
//! Keep this file free of `unwrap`/`expect`/`panic` (workspace lint policy,
//! NFR-003) and feed outputs through `black_box` so the optimiser can't delete
//! the work.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use emend_core::index::Index;
use emend_core::search::{quick_open, Cancel, DEFAULT_BATCH};

/// Number of indexed notes for the benchmark (FR-018 "tens of thousands"; SC-004
/// states the budget against 10k).
const INDEX_SIZE: usize = 10_000;

/// The per-query ranked-results cap the FFI worker uses (mirrors
/// `emend_ffi::search::QUICK_OPEN_LIMIT`). Re-declared here because the bench
/// links only `emend-core` (the cap is an FFI policy constant); keep them in step.
const QUICK_OPEN_LIMIT: usize = 200;

/// Build a 10k-entry index spread across a handful of nested folders so the
/// relative-path haystack is realistic (the ranker scores basename *and* rel
/// path). Returns a fully-populated, query-ready index.
fn build_index(n: usize) -> Index {
    let mut index = Index::new();
    let folders = [
        "",
        "notes/",
        "notes/daily/",
        "projects/emend/",
        "archive/2026/",
    ];
    for i in 0..n {
        let folder = folders[i % folders.len()];
        let rel = format!("{folder}note-{i:05}.md");
        let abs = format!("/root/{rel}");
        index.insert(&abs, &rel);
    }
    index
}

/// One warm query: rank `query` over `index` and drain the ranked batches through
/// a no-op sink (black-boxed so the work isn't optimised away). Returns the
/// streamed-hit count so the caller can `black_box` it too.
fn run_warm_query(index: &Index, query: &str) -> usize {
    let cancel = Cancel::new();
    let mut streamed = 0usize;
    let completed = quick_open(
        index,
        query,
        QUICK_OPEN_LIMIT,
        DEFAULT_BATCH,
        &cancel,
        |batch| {
            // Simulate the FFI sink's per-batch handoff without any FFI: just
            // count and black-box so the optimiser keeps the streaming work.
            streamed += batch.len();
            black_box(&batch);
        },
    );
    black_box(completed);
    streamed
}

fn bench_quick_open(c: &mut Criterion) {
    let index = build_index(INDEX_SIZE);

    let mut group = c.benchmark_group("quick_open_10k");
    // Each iteration scores the whole 10k haystack; keep the sample count modest
    // so the bench stays prompt while remaining statistically useful.
    group.throughput(Throughput::Elements(INDEX_SIZE as u64));
    group.sample_size(40);

    for query in ["note", "note-07777", "zzqq"] {
        group.bench_with_input(BenchmarkId::from_parameter(query), query, |b, q| {
            b.iter(|| black_box(run_warm_query(&index, black_box(q))));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_quick_open);
criterion_main!(benches);
