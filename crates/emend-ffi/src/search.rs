//! T074 — FFI projection of streaming, supersedable Quick Open (US3 · FFI
//! contract §5; FR-017, FR-018/SC-004, NFR-002; research §B2/§B7).
//!
//! Thin async shim that drives the **pure, tokio-free** core search driver
//! ([`emend_core::search::quick_open`]) on the boundary's shared `tokio` runtime
//! and forwards its ranked batches to the foreign [`SearchSink`]. As with the
//! rest of `emend-ffi` it holds **no ranking/emission logic** of its own — the
//! decision logic (rank, batch, stop-on-supersede) lives in
//! [`emend_core::search`] (Constitution V) — it only:
//!
//! 1. Bridges the boundary's cancellation primitives: the contract's
//!    `SearchHandle.cancel()` is a [`tokio_util::sync::CancellationToken`]
//!    (parity with [`crate::handles::CancellationHandle`]), while the core's
//!    stop-check is a tokio-free [`emend_core::search::Cancel`] flag.
//!    [`SearchHandle`] holds both and `cancel()` trips both, so a Swift
//!    `cancel()` and a *supersede* (the next query cancelling the prior) reach
//!    the spawned worker identically.
//! 2. Spawns the search on the shared runtime ([`crate::handles::try_runtime`]),
//!    runs the core driver under [`crate::panic::contain_panic`] (a panic in the
//!    worker becomes a contained no-op, never a process abort — NFR-003),
//!    forwards each ranked batch through [`SearchSink::on_results`], and fires the
//!    single terminal [`SearchSink::on_done`] **iff** the query completed (a
//!    superseded query is terminated silently by the next query's lifecycle — the
//!    contract gives search no error terminal, §5).
//!
//! ## Where it lives, and why (contract ambiguity resolved)
//!
//! The contract sketches `quick_open_query(query, sink) -> SearchHandle` as a
//! **free function** with the index implied. In this codebase the search
//! [`Index`](emend_core::index::Index) is **not global** — it lives inside the
//! [`WorkspaceHandle`](crate::workspace::WorkspaceHandle) so file operations keep
//! it in lock-step (see `workspace.rs`'s design note, FR-017a). So Quick Open is
//! exported as a **method on `WorkspaceHandle`**
//! ([`WorkspaceHandle::quick_open_query`](crate::workspace::WorkspaceHandle::quick_open_query)),
//! preserving the contract's exact parameters (`query: String`, `sink: Box<dyn
//! SearchSink>` → UniFFI 0.31 `Arc<dyn SearchSink>`) and return type
//! (`SearchHandle`, exported as `Arc<Self>`).
//!
//! ## Concurrency: the index is shared, the worker locks it briefly
//!
//! To run [`emend_core::search::quick_open`] (which needs `&Index`) on a spawned
//! task **without** holding the whole [`WorkspaceHandle`]'s `Mutex<Inner>` for the
//! search duration (which would block file ops), the `Index` lives behind its own
//! [`Arc<Mutex<Index>>`](std::sync::Mutex) inside `Inner`. The search task clones
//! that `Arc`, locks only the index for the synchronous rank+stream, and releases
//! it. The search is fast (<100 ms p95, SC-004) and synchronous, so this lock is
//! held briefly; concurrent file ops momentarily serialize against it (and update
//! the same index, FR-017a), which is exactly the lock-step the design wants.
//!
//! ## Supersede design (NFR-002)
//!
//! Each keystroke fires a fresh `quick_open_query`. To make the **new** query
//! supersede the **prior** one, the `WorkspaceHandle` keeps the *current* query's
//! [`SearchHandle`] in its locked inner state; every `quick_open_query` first
//! cancels whatever handle is stored (tripping its token + core flag → the stale
//! worker stops emitting and fires no `on_done`), then installs the new handle
//! and spawns the new worker. The returned handle is also handed to Swift so it
//! can `cancel()` explicitly (e.g. closing the palette). Cancellation is
//! idempotent, so a double-cancel (supersede then explicit close) is harmless.

use crate::handles::{try_runtime, SearchHit, SearchSink};
use crate::panic::contain_panic;
use emend_core::index::Index;
use emend_core::search::{quick_open, Cancel};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Default ranked-results cap for a single Quick Open query.
///
/// Quick Open is a "best few" palette, not a directory listing — the UI shows a
/// short ranked list and refines as the user types. Capping keeps each
/// (superseded-within-milliseconds) query cheap; tens of thousands of files are
/// still *scored* (SC-004), but only the top results are materialized/streamed.
const QUICK_OPEN_LIMIT: usize = 200;

/// The shared, lockable search index, co-owned by the [`WorkspaceHandle`] (for
/// lock-step file-op maintenance) and by each spawned Quick Open worker (for the
/// synchronous rank+stream). See the module's "Concurrency" note.
pub(crate) type SharedIndex = Arc<Mutex<Index>>;

/// Rust-owned handle for one in-flight Quick Open query (FFI contract §5:
/// `SearchHandle.cancel()` supersedes an in-flight query, NFR-002).
///
/// Handed to Swift as `Arc<Self>`. Holds both halves of the boundary's
/// cancellation bridge:
///
/// * a [`CancellationToken`] — parity with [`crate::handles::CancellationHandle`];
///   it backs the contract's `cancel()`;
/// * an [`emend_core::search::Cancel`] flag — the tokio-free stop-check the
///   *core* driver polls between batches.
///
/// [`cancel`](Self::cancel) trips **both**, so an explicit Swift `cancel()` and a
/// supersede (the next query cancelling this handle) are indistinguishable to the
/// worker: it stops emitting and fires no terminal `on_done`.
#[derive(Debug, uniffi::Object)]
pub struct SearchHandle {
    /// Boundary-standard cancellation token (parity with `CancellationHandle`).
    token: CancellationToken,
    /// Tokio-free stop flag the core driver polls between batches.
    cancel: Cancel,
}

#[uniffi::export]
impl SearchHandle {
    /// Cancel (supersede) this query (FFI contract §5).
    ///
    /// Idempotent: superseding then explicitly cancelling (or cancelling twice)
    /// is a no-op. After this, the worker stops emitting `on_results` and fires no
    /// `on_done` for this query.
    pub fn cancel(&self) {
        self.token.cancel();
        self.cancel.cancel();
    }
}

impl SearchHandle {
    /// Create a fresh, uncancelled handle (token + core flag both clear).
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            cancel: Cancel::new(),
        }
    }

    /// Clone of the tokio-free cancel flag for the spawned worker to poll.
    fn cancel_flag(&self) -> Cancel {
        self.cancel.clone()
    }
}

/// Start one Quick Open query: build a fresh [`SearchHandle`], spawn the
/// streaming worker on the shared runtime, and return the handle (`Arc`) for the
/// caller to install as the workspace's current query and hand to Swift.
///
/// The worker locks `index` only for the synchronous rank+stream
/// ([`emend_core::search::quick_open`]) — never the workspace's `Inner` lock — so
/// concurrent file ops are not blocked for the search duration (module
/// "Concurrency" note).
///
/// If the runtime cannot be obtained the query degrades gracefully: the terminal
/// `on_done` is delivered synchronously with no results (an empty, completed
/// stream) rather than erroring — Quick Open is a best-effort UI affordance, and
/// the contract gives it no error terminal (§5).
pub(crate) fn start_query(
    index: SharedIndex,
    query: String,
    sink: Arc<dyn SearchSink>,
) -> Arc<SearchHandle> {
    let handle = Arc::new(SearchHandle::new());
    let cancel = handle.cancel_flag();

    match try_runtime() {
        Ok(rt) => {
            rt.spawn(async move {
                // Contain any panic in the worker body so it becomes a no-op
                // terminal rather than aborting the process (NFR-003). On a
                // contained panic we simply stop — search has no error terminal.
                let _ = contain_panic(move || run_worker(&index, &query, &cancel, &sink));
            });
        }
        Err(_) => {
            // No runtime: deliver an empty, completed stream synchronously so the
            // UI's terminal still fires (a brand-new handle is never cancelled).
            if !cancel.is_cancelled() {
                sink.on_done();
            }
        }
    }
    handle
}

/// The spawned worker body: lock the shared index, run the **core** driver
/// ([`emend_core::search::quick_open`]) — which ranks then streams batches,
/// stopping on supersede — forwarding each ranked batch to the foreign sink, and
/// fire the single terminal `on_done` iff it completed.
///
/// Locking the index here (not the workspace `Inner`) keeps file ops unblocked;
/// the lock is held only for the fast synchronous search. A poisoned lock (a
/// prior panic while held — unreachable under the no-panic posture) degrades to
/// an empty completed stream rather than `unwrap`ping (NFR-003).
fn run_worker(index: &SharedIndex, query: &str, cancel: &Cancel, sink: &Arc<dyn SearchSink>) {
    let Ok(guard) = index.lock() else {
        // Poisoned: best-effort empty terminal (search has no error terminal).
        if !cancel.is_cancelled() {
            sink.on_done();
        }
        return;
    };

    // The core driver owns the rank + batch + stop-on-supersede policy; we only
    // project each core hit to the FFI `SearchHit` and forward it. `quick_open`
    // returns `true` iff the stream completed un-superseded.
    let completed = quick_open(
        &guard,
        query,
        QUICK_OPEN_LIMIT,
        emend_core::search::DEFAULT_BATCH,
        cancel,
        |batch| {
            let projected: Vec<SearchHit> = batch.into_iter().map(project_hit).collect();
            sink.on_results(projected);
        },
    );
    drop(guard); // release the index before the terminal callback (non-reentrant)

    if completed {
        sink.on_done();
    }
}

/// Project a core [`SearchHit`](emend_core::index::SearchHit) to the FFI
/// [`SearchHit`]: the core's `rel_path` is the contract's `breadcrumb` source
/// (same mapping `crate::workspace` uses for the synchronous `query`).
fn project_hit(core: emend_core::index::SearchHit) -> SearchHit {
    let emend_core::index::SearchHit {
        path,
        name,
        rel_path,
        score,
    } = core;
    SearchHit {
        path,
        name,
        breadcrumb: rel_path,
        score,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{project_hit, run_worker, SharedIndex};
    use crate::handles::{SearchHit, SearchSink};
    use emend_core::index::Index;
    use emend_core::search::Cancel;
    use std::sync::{Arc, Mutex};

    /// A recording `SearchSink`: captures every batch and whether `on_done` fired.
    #[derive(Default)]
    struct Recorder {
        batches: Mutex<Vec<Vec<SearchHit>>>,
        done: Mutex<bool>,
    }

    impl Recorder {
        fn total_hits(&self) -> usize {
            self.batches
                .lock()
                .map(|b| b.iter().map(Vec::len).sum())
                .unwrap_or(0)
        }
        fn batch_count(&self) -> usize {
            self.batches.lock().map(|b| b.len()).unwrap_or(0)
        }
        fn is_done(&self) -> bool {
            self.done.lock().map(|d| *d).unwrap_or(false)
        }
    }

    impl SearchSink for Recorder {
        fn on_results(&self, batch: Vec<SearchHit>) {
            if let Ok(mut b) = self.batches.lock() {
                b.push(batch);
            }
        }
        fn on_done(&self) {
            if let Ok(mut d) = self.done.lock() {
                *d = true;
            }
        }
    }

    fn seeded_shared(n: usize) -> SharedIndex {
        let mut index = Index::new();
        for i in 0..n {
            let rel = format!("note-{i:04}.md");
            index.insert(&format!("/root/{rel}"), &rel);
        }
        Arc::new(Mutex::new(index))
    }

    #[test]
    fn project_hit_maps_rel_path_to_breadcrumb() {
        let core = emend_core::index::SearchHit {
            path: "/root/note-7.md".to_owned(),
            name: "note-7.md".to_owned(),
            rel_path: "sub/note-7.md".to_owned(),
            score: 100,
        };
        let projected = project_hit(core);
        assert_eq!(projected.path, "/root/note-7.md");
        assert_eq!(projected.breadcrumb, "sub/note-7.md");
        assert_eq!(projected.score, 100);
    }

    #[test]
    fn run_worker_streams_all_batches_then_one_done() {
        let index = seeded_shared(70);
        let cancel = Cancel::new();
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn SearchSink> = rec.clone();
        run_worker(&index, "note", &cancel, &sink);

        assert_eq!(rec.total_hits(), 70, "all matching hits stream through");
        assert!(rec.batch_count() >= 2, "70 hits batch into multiple chunks");
        assert!(rec.is_done(), "an un-superseded stream fires its terminal");
    }

    #[test]
    fn run_worker_superseded_fires_no_terminal() {
        let index = seeded_shared(200);
        let cancel = Cancel::new();
        cancel.cancel(); // superseded before the worker runs
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn SearchSink> = rec.clone();
        run_worker(&index, "note", &cancel, &sink);

        assert_eq!(rec.total_hits(), 0, "a superseded query emits nothing");
        assert!(
            !rec.is_done(),
            "a superseded query fires no terminal on_done"
        );
    }
}
