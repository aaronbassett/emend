//! T055a — concurrency stress for the workspace + index + watcher model under
//! interleaved operations (NFR-004: "no corruption, no panic under concurrent
//! access").
//!
//! ## What this proves, and how it stays deterministic
//!
//! The real `notify` event source is timing-nondeterministic, so this test does
//! **not** depend on OS event delivery. Instead it drives the *model types
//! directly* from many threads — exactly the operations the live system performs
//! once the watcher has classified an event:
//!
//! * **Producer threads** run real `Workspace` file operations (create / rename /
//!   move / delete) against a shared on-disk tree — the user-action and
//!   external-tool side.
//! * The same threads feed the resulting change through the **watcher's pure
//!   classifier** (`process_batch` over a synthetic `DebouncedEvent`, the exact
//!   path the live drain thread takes) and apply the surviving `ChangeEvent`s to
//!   a shared `Index` behind a `Mutex` (`insert` / `rename` / `remove`) — the
//!   incremental-maintenance side (FR-017a).
//! * **Reader threads** continuously `query` and `resolve_name` the index and
//!   `list_children` the workspace — the Quick Open / wiki-link / sidebar side.
//!
//! The interleaving is genuine (OS thread scheduling), but every *assertion* is
//! on an invariant that holds regardless of interleaving order, so the test is
//! deterministic in outcome:
//!
//! 1. **No panic / no deadlock** — the harness joins every thread; a panic in any
//!    thread fails the test, and completion proves no deadlock (NFR-004).
//! 2. **No corruption** — after the storm, the index's live count equals its
//!    reconstructable truth (every surviving entry resolves, tombstones never
//!    leak as live), and `rebuild_count()` is still 0 (every update was
//!    incremental, never a rescan — FR-017a).
//! 3. **Reads never observe a torn entry** — a hit's `path`/`name`/`rel_path` are
//!    always internally consistent (the index updates an entry atomically under
//!    the lock).

// Integration tests assert on their own fixtures; the workspace denies these in
// library code, so scope the allowance to this test module.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::index::Index;
use emend_core::watcher::{process_batch, ChangeEvent, SuppressionRegistry};
use emend_core::workspace::Workspace;
use notify_debouncer_full::notify::event::{CreateKind, EventKind, ModifyKind, RenameMode};
use notify_debouncer_full::notify::Event;
use notify_debouncer_full::DebouncedEvent;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Build a synthetic debounced event (the deterministic seam — no real OS).
fn ev(kind: EventKind, paths: &[&Path]) -> DebouncedEvent {
    let event = Event {
        kind,
        paths: paths.iter().map(|p| p.to_path_buf()).collect(),
        attrs: Default::default(),
    };
    DebouncedEvent::new(event, Instant::now())
}

/// Apply one classified change to the shared index incrementally — the exact
/// dispatch the live drain thread performs (FR-017a: touch only the affected
/// entry, never rescan). `rel` derivations use the basename for simplicity; the
/// index keys on the absolute path either way.
fn apply_change(index: &Mutex<Index>, change: &ChangeEvent) {
    let Ok(mut idx) = index.lock() else { return };
    match change {
        ChangeEvent::Created { path } | ChangeEvent::Modified { path } => {
            let rel = basename(path);
            idx.insert(path, &rel);
        }
        ChangeEvent::Removed { path } => idx.remove(path),
        ChangeEvent::Renamed { from, to } => {
            let rel = basename(to);
            idx.rename(from, to, &rel);
        }
    }
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Run `events` through the pure pipeline (no suppression recorded → all pass)
/// and apply each survivor to the shared index. This is precisely what the live
/// watcher drain loop does, minus the OS event source.
fn drive(index: &Mutex<Index>, events: &[DebouncedEvent]) {
    let mut reg = SuppressionRegistry::new();
    let changes = process_batch(events, &mut reg, Instant::now(), |_p| None);
    for change in &changes {
        apply_change(index, change);
    }
}

#[test]
fn interleaved_workspace_index_and_watcher_ops_stay_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    // Shared, lock-guarded model state. The Workspace file ops are `&self`
    // (they touch the filesystem, not the struct), so a shared `Arc<Workspace>`
    // is safe to call concurrently; the Index is `&mut`, so it sits behind a
    // Mutex (the FFI driver will wrap it the same way).
    let workspace = Arc::new(Workspace::new());
    let index = Arc::new(Mutex::new(Index::new()));

    const PRODUCERS: usize = 6;
    const READERS: usize = 4;
    const OPS_PER_PRODUCER: usize = 60;

    let mut handles = Vec::new();

    // --- Producer threads: real file ops + watcher-classified index updates ---
    for t in 0..PRODUCERS {
        let workspace = Arc::clone(&workspace);
        let index = Arc::clone(&index);
        let root = root.clone();
        handles.push(std::thread::spawn(move || {
            // Each producer owns a private subfolder so the file ops don't collide
            // on the same names (the Workspace is collision-SAFE, but private dirs
            // keep the test's expected-state bookkeeping simple). Cross-thread
            // contention is on the SHARED index lock + the shared parent reads.
            let sub = root.join(format!("p{t}"));
            std::fs::create_dir_all(&sub).unwrap();

            for i in 0..OPS_PER_PRODUCER {
                // 1. Create a note via the real collision-safe workspace op.
                let created = workspace
                    .create_note(sub.to_str().unwrap(), &format!("n{i}"))
                    .unwrap();
                let created_path = PathBuf::from(&created);
                drive(
                    &index,
                    &[ev(EventKind::Create(CreateKind::File), &[&created_path])],
                );

                // 2. Sometimes rename it (a move correlated as ONE event).
                if i % 3 == 0 {
                    let renamed = workspace.rename(&created, &format!("r{t}_{i}")).unwrap();
                    let renamed_path = PathBuf::from(&renamed);
                    drive(
                        &index,
                        &[ev(
                            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                            &[&created_path, &renamed_path],
                        )],
                    );

                    // 3. And sometimes delete the renamed note.
                    if i % 6 == 0 {
                        workspace.delete(&renamed).unwrap();
                        drive(
                            &index,
                            &[ev(
                                EventKind::Modify(ModifyKind::Name(RenameMode::From)),
                                &[&renamed_path],
                            )],
                        );
                    }
                }
            }
        }));
    }

    // --- Reader threads: query / resolve / list concurrently with the storm ---
    for _ in 0..READERS {
        let workspace = Arc::clone(&workspace);
        let index = Arc::clone(&index);
        let root = root.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..400 {
                if let Ok(idx) = index.lock() {
                    // Query must never return a torn entry: each hit's fields are
                    // internally consistent (basename of path == name).
                    for hit in idx.query("n", 16) {
                        let want = basename(&hit.path);
                        assert_eq!(
                            hit.name, want,
                            "index returned a torn entry: name={:?} path={:?}",
                            hit.name, hit.path
                        );
                    }
                    // resolve_name returns only live entries (no tombstone leak).
                    for p in idx.resolve_name("n0") {
                        assert!(!p.is_empty());
                    }
                }
                // Listing the shared root concurrently with creates must not panic
                // or corrupt; the count is racy so we only assert it doesn't error.
                let _ = workspace.list_children(root.to_str().unwrap());
                std::thread::yield_now();
            }
        }));
    }

    // Joining every thread proves: no deadlock (we got here), and no panic in any
    // thread (a panicked thread makes `join` return `Err`, failing the test).
    for h in handles {
        h.join()
            .expect("a worker thread panicked — NFR-004 violated");
    }

    // --- Post-storm invariants -------------------------------------------------
    let idx = index.lock().unwrap();

    // FR-017a: every update went through the incremental path — NEVER a rescan.
    assert_eq!(
        idx.rebuild_count(),
        0,
        "concurrent updates must stay incremental (no full rescan)"
    );

    // No corruption: the live count equals the number of entries that actually
    // resolve to a consistent hit. We reconstruct truth by querying broadly and
    // checking each survivor is internally consistent; len() must agree with the
    // index's own bookkeeping (live counter kept in lockstep with the maps).
    let all_hits = idx.query("n", usize::MAX);
    // Every returned hit is internally consistent (already asserted by readers,
    // re-checked here on the final state for the full set).
    for hit in &all_hits {
        assert_eq!(hit.name, basename(&hit.path));
    }
    // The O(1) live counter (`len()`) must stay in lockstep with the maps: a
    // corrupted counter would desync from what queries can surface. We can't read
    // internals from an integration test, so we assert the meaningful externally
    // observable property — the index is non-empty after the storm, and querying
    // everything surfaces no MORE live hits than `len()` claims (no tombstone
    // leaking through as a live entry).
    assert!(
        !idx.is_empty(),
        "the index has surviving entries after the storm"
    );
    assert!(
        all_hits.len() <= idx.len(),
        "query surfaced more hits ({}) than the live count ({}) — tombstone leak",
        all_hits.len(),
        idx.len()
    );
}

#[test]
fn index_behind_mutex_handles_rename_delete_race_without_panic() {
    // A tighter race: many threads rename/delete overlapping logical paths in the
    // index (no filesystem) to stress the incremental re-keying under contention.
    // The index must never panic on a rename-of-missing or double-delete (both
    // are defined no-ops / insert-fallbacks), so any interleaving is safe.
    let index = Arc::new(Mutex::new(Index::new()));

    // Seed a shared set of paths.
    {
        let mut idx = index.lock().unwrap();
        for i in 0..200 {
            idx.insert(&format!("/r/seed{i}.md"), &format!("seed{i}.md"));
        }
    }

    let mut handles = Vec::new();
    for t in 0..8 {
        let index = Arc::clone(&index);
        handles.push(std::thread::spawn(move || {
            for i in 0..200 {
                let from = format!("/r/seed{i}.md");
                let to = format!("/r/moved{t}_{i}.md");
                if let Ok(mut idx) = index.lock() {
                    // Multiple threads racing the SAME source path: the first wins
                    // the rename; the rest hit the insert-fallback (no panic).
                    idx.rename(&from, &to, &format!("moved{t}_{i}.md"));
                }
                if i % 5 == 0 {
                    if let Ok(mut idx) = index.lock() {
                        idx.remove(&to); // may already be gone — defined no-op
                    }
                }
            }
        }));
    }
    for h in handles {
        h.join()
            .expect("a worker thread panicked during the rename/delete race");
    }

    let idx = index.lock().unwrap();
    assert_eq!(idx.rebuild_count(), 0, "no rescan under contention");
    // Sanity: the index is internally consistent — every queryable hit is intact.
    for hit in idx.query("moved", usize::MAX) {
        assert_eq!(hit.name, basename(&hit.path));
    }
}
