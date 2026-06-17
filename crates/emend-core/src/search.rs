//! T073 — the pure, cancellable Quick Open *search driver* over the workspace
//! [`Index`](crate::index::Index) (US3 · FR-017, FR-018/SC-004; research §B2/§B7).
//!
//! Where [`crate::index`] owns the *haystack* and the synchronous ranked
//! [`Index::query`](crate::index::Index::query), this module owns the
//! **streaming, supersedable emission policy** the FFI Quick Open driver (T074)
//! drives a `tokio` task with — but it is itself **synchronous and tokio-free**
//! (Constitution V): it holds **no `uniffi` and no `tokio`** types, so the whole
//! supersede/cancel behaviour is unit-testable with plain `cargo test`
//! (`tests/search_supersede.rs`).
//!
//! ## The split (why this isn't in `emend-ffi`)
//!
//! The contract (FFI §5) streams ranked results through a foreign `SearchSink`
//! and supersedes the prior query on each keystroke (NFR-002). The *async* half
//! of that — the `tokio` task, the `CancellationToken`, the `Arc<dyn SearchSink>`
//! — must live in `emend-ffi` (the only crate allowed `tokio`/`uniffi`). But the
//! *decision logic* ("rank, then emit batches, stopping the instant the query is
//! superseded") is pure and belongs in the core so it can be tested without an
//! async runtime or the FFI toolchain. So this module exposes:
//!
//! * [`Cancel`] — a tiny `AtomicBool`-backed cancel flag (no `tokio`), cloneable
//!   so the driver keeps one handle and the task observes another.
//! * [`quick_open`] — rank `query` over the index via [`Index::query`], then push
//!   results to the caller's `emit` callback **in batches**, re-checking
//!   [`Cancel::is_cancelled`] before every batch so a supersede stops emission
//!   mid-stream (NFR-002). Synchronous; the FFI driver runs it inside its spawned
//!   task and forwards each batch to `SearchSink::on_results`.
//!
//! ## Why batch, and why check between batches
//!
//! Quick Open clears ≤100 ms p95 over tens of thousands of files with large
//! headroom (SC-004 — `benches/quick_open.rs` tracks it), so the *ranking* is not
//! the cost worry. Batching exists for **supersede latency**, not throughput: a
//! user typing fast supersedes the in-flight query within a few milliseconds, and
//! checking the cancel flag between small batches means the stale task abandons
//! its remaining (already-ranked) results promptly instead of delivering a full
//! page the UI is about to discard. Emitting the *whole* result set in one shot
//! would still be correct, just less responsive under fast superseding; batching
//! is the cheap insurance.

use crate::index::{Index, SearchHit};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Default results-per-batch for [`quick_open`] when the caller does not pick a
/// size. Small enough that a supersede is observed within one batch's worth of
/// emission, large enough to avoid a callback per hit.
pub const DEFAULT_BATCH: usize = 32;

/// A tokio-free cancellation flag for a single Quick Open query (NFR-002).
///
/// Backed by an [`Arc<AtomicBool>`] so it is cheap to clone and share: the FFI
/// driver keeps one [`Cancel`] per in-flight query and, on the next keystroke,
/// [`cancel`](Cancel::cancel)s it (supersede) before starting the next; the
/// spawned worker holds a clone and [`quick_open`] polls
/// [`is_cancelled`](Cancel::is_cancelled) between batches to stop emitting.
///
/// This deliberately does **not** use `tokio_util::CancellationToken` — that
/// would pull `tokio` into `emend-core` (Constitution V). The FFI layer bridges
/// its `CancellationToken` to this flag (cancel one → set the other); the core
/// only needs the cheap, runtime-free "should I stop?" check.
///
/// Cloning shares state (`Relaxed` ordering is sufficient: the flag is a simple
/// one-way latch with no other memory it must synchronize-with — a missed-by-one-
/// batch observation is harmless, the next check catches it).
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    /// A fresh, uncancelled flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation (supersede). Idempotent — calling it again, or after
    /// the query already finished, is a harmless no-op. All clones observe it.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested through this flag or any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

/// Run one Quick Open query against `index`, streaming up to `limit` ranked
/// [`SearchHit`]s to `emit` in batches of `batch_size`, stopping early if
/// `cancel` is tripped (NFR-002).
///
/// Returns `true` if the query ran to completion (all ranked results emitted),
/// or `false` if it was **superseded/cancelled** before finishing — so the FFI
/// driver can decide whether to fire its single terminal `on_done` (it does so
/// only for a query that was *not* superseded; a superseded query is terminated
/// silently by the next query's lifecycle, per the contract's §5 note that
/// search has no error terminal).
///
/// ## Emission contract
///
/// * Ranking is a single [`Index::query`] call (best-first, deterministic ties —
///   see that method); this function only governs *how* the ranked slice is
///   handed out.
/// * `emit` is called with **non-empty** batches of at most `batch_size` hits, in
///   ranked order. `batch_size` of `0` is treated as [`DEFAULT_BATCH`] (a caller
///   that means "no batching" should pass `limit`).
/// * The flag is checked **before** the whole query (so an already-superseded
///   query does no work) and **before each batch** (so a supersede mid-stream
///   abandons the remaining, already-ranked results promptly). It is intentionally
///   *not* checked between individual hits within a batch — the batch is the unit
///   of responsiveness.
/// * An empty `query`, a `limit` of `0`, or a no-match query simply emits nothing
///   and returns `true` (completed: there was nothing to stream).
///
/// This is synchronous and tokio-free; the FFI driver (T074) calls it inside a
/// spawned task with `emit` forwarding each batch to the foreign `SearchSink`.
pub fn quick_open<F>(
    index: &Index,
    query: &str,
    limit: usize,
    batch_size: usize,
    cancel: &Cancel,
    mut emit: F,
) -> bool
where
    F: FnMut(Vec<SearchHit>),
{
    // Already superseded before we started: do no ranking, report "not
    // completed" so the driver suppresses its terminal.
    if cancel.is_cancelled() {
        return false;
    }

    let hits = index.query(query, limit);
    let batch = if batch_size == 0 {
        DEFAULT_BATCH
    } else {
        batch_size
    };

    // Drain the ranked results into `emit` one batch at a time, re-checking the
    // cancel flag before handing out each batch so a supersede stops emission
    // mid-stream (NFR-002).
    let mut iter = hits.into_iter();
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let chunk: Vec<SearchHit> = iter.by_ref().take(batch).collect();
        if chunk.is_empty() {
            // Ranked results exhausted (or there were none): completed.
            return true;
        }
        emit(chunk);
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

    use super::{quick_open, Cancel, DEFAULT_BATCH};
    use crate::index::Index;

    /// Seed an index with `n` notes that all match the query `note`, so a query
    /// returns a multi-batch result set.
    fn seeded(n: usize) -> Index {
        let mut index = Index::new();
        for i in 0..n {
            let rel = format!("note-{i:04}.md");
            index.insert(&format!("/root/{rel}"), &rel);
        }
        index
    }

    #[test]
    fn cancel_flag_clones_share_state() {
        let a = Cancel::new();
        let b = a.clone();
        assert!(!a.is_cancelled());
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled(), "a clone must observe the cancel");
    }

    #[test]
    fn already_cancelled_emits_nothing_and_reports_incomplete() {
        let index = seeded(100);
        let cancel = Cancel::new();
        cancel.cancel();
        let mut batches = 0usize;
        let completed = quick_open(&index, "note", 100, 8, &cancel, |_| batches += 1);
        assert!(!completed, "a pre-cancelled query must report incomplete");
        assert_eq!(batches, 0, "a pre-cancelled query must emit nothing");
    }

    #[test]
    fn completes_and_emits_in_batches() {
        let index = seeded(20);
        let cancel = Cancel::new();
        let mut total = 0usize;
        let mut batch_count = 0usize;
        let completed = quick_open(&index, "note", 50, 8, &cancel, |chunk| {
            assert!(!chunk.is_empty(), "batches are never empty");
            assert!(chunk.len() <= 8, "batches respect the size cap");
            total += chunk.len();
            batch_count += 1;
        });
        assert!(completed, "an un-superseded query runs to completion");
        assert_eq!(total, 20, "all matching hits are emitted");
        assert_eq!(batch_count, 3, "20 hits / batch 8 => 3 batches (8+8+4)");
    }

    #[test]
    fn zero_batch_size_falls_back_to_default() {
        let index = seeded(DEFAULT_BATCH * 2);
        let cancel = Cancel::new();
        let mut max_batch = 0usize;
        let completed = quick_open(&index, "note", DEFAULT_BATCH * 4, 0, &cancel, |chunk| {
            max_batch = max_batch.max(chunk.len());
        });
        assert!(completed);
        assert_eq!(
            max_batch, DEFAULT_BATCH,
            "batch_size 0 must use DEFAULT_BATCH, not 'all at once'"
        );
    }

    #[test]
    fn no_match_completes_with_no_emission() {
        let index = seeded(10);
        let cancel = Cancel::new();
        let mut batches = 0usize;
        let completed = quick_open(&index, "zzzzz-no-such", 10, 8, &cancel, |_| batches += 1);
        assert!(
            completed,
            "a no-match query still 'completes' (nothing to do)"
        );
        assert_eq!(batches, 0);
    }

    #[test]
    fn supersede_mid_stream_stops_emission() {
        let index = seeded(100);
        let cancel = Cancel::new();
        // Cancel from inside the emit callback after the first batch — modelling a
        // supersede landing while the stale task is still draining results.
        let mut batches = 0usize;
        let completed = quick_open(&index, "note", 100, 8, &cancel, |_chunk| {
            batches += 1;
            if batches == 1 {
                cancel.cancel();
            }
        });
        assert!(!completed, "a superseded query reports incomplete");
        assert_eq!(
            batches, 1,
            "emission must stop at the next batch boundary after supersede"
        );
    }
}
