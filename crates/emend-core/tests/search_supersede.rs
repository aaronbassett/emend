//! T072 — failing-first integration tests for the **cancellable** Quick Open
//! search driver (`emend_core::search`), the pure layer behind the streaming FFI
//! `quick_open_query` (US3 · FR-017, FR-018, NFR-002; research §B2/§B7).
//!
//! The driver's whole reason to exist as a *separate* module from
//! [`emend_core::index`] is the **supersede/cancel** behaviour NFR-002 demands: a
//! new query supersedes the previous one's emission. These tests pin that down
//! **deterministically and tokio-free** — no async runtime, no timing — by
//! driving the synchronous [`quick_open`](emend_core::search::quick_open) with a
//! plain [`Cancel`](emend_core::search::Cancel) flag and asserting on exactly
//! which batches were emitted.
//!
//! The contract obligation under test (FFI §5 test obligation #5): "superseding
//! via `SearchHandle.cancel()` stops result emission". The FFI `SearchHandle`
//! bridges its `CancellationToken` to this same [`Cancel`] flag, so proving the
//! flag stops emission here proves the supersede semantics the boundary relies on
//! — without needing the FFI toolchain or a runtime.
//!
//! What's asserted:
//!
//! 1. **A pre-set flag stops all emission** — a query whose flag is already
//!    cancelled (the prior query was superseded before its worker ran) emits
//!    nothing and reports incomplete.
//! 2. **Setting the flag mid-stream stops emission at the next batch boundary** —
//!    the core obligation: once superseded, no further `on_results`-equivalent
//!    batches fire.
//! 3. **An un-superseded query runs to completion** — the baseline: all ranked
//!    hits stream through and the driver reports completion (so the FFI layer
//!    fires its single terminal `on_done`).
//! 4. **Ranking is delegated, ordering preserved** — the driver streams the
//!    index's ranked order unchanged (it governs *emission*, not *ranking*).

// Integration tests assert on their own fixtures; the workspace denies these in
// library code, so scope the allowance to this test module.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::index::{Index, SearchHit};
use emend_core::search::{quick_open, Cancel, DEFAULT_BATCH};

/// Seed an index with `n` notes that all fuzzy-match the query `note`.
fn seeded(n: usize) -> Index {
    let mut index = Index::new();
    for i in 0..n {
        let rel = format!("note-{i:04}.md");
        index.insert(&format!("/root/{rel}"), &rel);
    }
    index
}

/// Collect every emitted batch into a flat `Vec`, recording batch boundaries.
struct Sink {
    batches: Vec<Vec<SearchHit>>,
}

impl Sink {
    fn new() -> Self {
        Self {
            batches: Vec::new(),
        }
    }
    fn total(&self) -> usize {
        self.batches.iter().map(Vec::len).sum()
    }
    fn count(&self) -> usize {
        self.batches.len()
    }
}

// ---------------------------------------------------------------------------
// (1) A pre-cancelled query emits nothing
// ---------------------------------------------------------------------------

#[test]
fn pre_cancelled_flag_emits_nothing() {
    let index = seeded(100);
    let cancel = Cancel::new();
    cancel.cancel();

    let mut sink = Sink::new();
    let completed = quick_open(&index, "note", 100, 8, &cancel, |b| sink.batches.push(b));

    assert!(!completed, "a pre-cancelled query reports incomplete");
    assert_eq!(sink.total(), 0, "a pre-cancelled query emits zero hits");
    assert_eq!(sink.count(), 0, "a pre-cancelled query emits zero batches");
}

// ---------------------------------------------------------------------------
// (2) Setting the flag mid-stream stops emission (the core NFR-002 obligation)
// ---------------------------------------------------------------------------

#[test]
fn setting_cancel_flag_mid_stream_stops_emission() {
    let index = seeded(100);
    let cancel = Cancel::new();

    // Cancel from inside the emit callback once the first batch lands — modelling
    // a supersede (the next keystroke) arriving while the stale worker is still
    // draining its ranked results.
    let mut batches: Vec<Vec<SearchHit>> = Vec::new();
    let completed = quick_open(&index, "note", 100, 8, &cancel, |b| {
        batches.push(b);
        if batches.len() == 1 {
            cancel.cancel();
        }
    });

    assert!(!completed, "a superseded query reports incomplete");
    assert_eq!(
        batches.len(),
        1,
        "emission must stop at the next batch boundary after the flag is set: \
         got {} batches",
        batches.len()
    );
    let emitted: usize = batches.iter().map(Vec::len).sum();
    assert!(
        emitted < 100,
        "a superseded query must NOT emit the full ranked set (emitted {emitted})"
    );
}

#[test]
fn cancel_observed_through_a_clone_stops_emission() {
    // The FFI driver holds one `Cancel`, the worker a clone; prove a cancel on the
    // *original* (as a supersede does) stops the worker's stream observed via the
    // clone — the exact split the boundary uses (NFR-002).
    let index = seeded(100);
    let original = Cancel::new();
    let worker_clone = original.clone();

    let mut batches = 0usize;
    let completed = quick_open(&index, "note", 100, 8, &worker_clone, |_b| {
        batches += 1;
        if batches == 1 {
            // Supersede via the original handle the driver kept.
            original.cancel();
        }
    });

    assert!(!completed);
    assert_eq!(
        batches, 1,
        "the worker's clone observes the original's cancel"
    );
}

// ---------------------------------------------------------------------------
// (3) An un-superseded query runs to completion
// ---------------------------------------------------------------------------

#[test]
fn un_superseded_query_completes_and_streams_all() {
    let index = seeded(20);
    let cancel = Cancel::new();

    let mut sink = Sink::new();
    let completed = quick_open(&index, "note", 50, 8, &cancel, |b| sink.batches.push(b));

    assert!(completed, "an un-superseded query reports completion");
    assert_eq!(sink.total(), 20, "all matching hits stream through");
    assert_eq!(
        sink.count(),
        3,
        "20 hits at batch 8 => 3 batches (8 + 8 + 4)"
    );
    for (i, b) in sink.batches.iter().enumerate() {
        assert!(!b.is_empty(), "batch {i} is non-empty");
        assert!(b.len() <= 8, "batch {i} respects the size cap");
    }
}

#[test]
fn zero_batch_size_uses_default() {
    let index = seeded(DEFAULT_BATCH * 2);
    let cancel = Cancel::new();
    let mut max_batch = 0usize;
    let completed = quick_open(&index, "note", DEFAULT_BATCH * 4, 0, &cancel, |b| {
        max_batch = max_batch.max(b.len());
    });
    assert!(completed);
    assert_eq!(
        max_batch, DEFAULT_BATCH,
        "batch_size 0 falls back to DEFAULT_BATCH"
    );
}

// ---------------------------------------------------------------------------
// (4) The driver streams the index's ranked order unchanged
// ---------------------------------------------------------------------------

#[test]
fn driver_preserves_the_index_ranked_order() {
    // A basename match must outrank a path-only match (Index ranking, FR-017); the
    // driver must stream that order verbatim, never reorder.
    let mut index = Index::new();
    index.insert("/root/alpha.md", "alpha.md"); // basename carries "alpha"
    index.insert("/root/alpha/other.md", "alpha/other.md"); // only the path matches

    // Reference order from the synchronous index query.
    let reference: Vec<String> = index
        .query("alpha", 10)
        .into_iter()
        .map(|h| h.path)
        .collect();

    let cancel = Cancel::new();
    let mut streamed: Vec<String> = Vec::new();
    let completed = quick_open(&index, "alpha", 10, 1, &cancel, |b| {
        streamed.extend(b.into_iter().map(|h| h.path));
    });

    assert!(completed);
    assert_eq!(
        streamed, reference,
        "the driver streams the index's ranked order unchanged"
    );
    assert_eq!(
        streamed.first().map(String::as_str),
        Some("/root/alpha.md"),
        "the basename match ranks first (FR-017), preserved by the driver"
    );
}
