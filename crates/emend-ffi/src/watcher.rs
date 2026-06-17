//! T059 — FFI projection of the live file watcher + conflict model (US2 · FFI
//! contract §1/§3 "Conflict handling"; FR-006/006a/006b/006c).
//!
//! Thin UniFFI shim over [`emend_core::watcher`]: the real `notify`/FSEvents
//! wrapper ([`emend_core::watcher::FsWatcher`]), its pure classification core,
//! and the conflict truth table. As with [`crate::document`] and
//! [`crate::workspace`], **all** logic stays in the core; this module only:
//!
//! 1. **Projects value types** the core cannot derive `uniffi` on (Constitution
//!    V keeps `emend-core` `uniffi`-free): [`ChangeEvent`] (the post-suppression
//!    external-change variants), [`ConflictState`], and [`ConflictChoice`]. Each
//!    `From`/`Into` conversion is **exhaustive — no wildcard arm** — so a new
//!    core variant breaks compilation here until mirrored, the same
//!    closed-projection discipline as [`crate::error::FfiError`].
//!
//! 2. **Bridges the watcher to a foreign [`DocObserver`]** via [`ObserverBridge`],
//!    a Rust-side adapter implementing the core's
//!    [`WatchObserver`](emend_core::watcher::WatchObserver). The watcher's drain
//!    thread calls `WatchObserver::on_change(core::ChangeEvent)`; the bridge
//!    projects that to the FFI [`ChangeEvent`] and forwards it to
//!    `DocObserver::on_fs_change`. So the Swift side implements **one** foreign
//!    trait ([`DocObserver`]) for both derived-insight pushes (T039) and
//!    file-system change pushes (this task).
//!
//! 3. **Wraps the live watcher** in [`WatchHandle`], a `#[derive(uniffi::Object)]`
//!    handed to Swift as `Arc<Self>`. Dropping the last `Arc` (or calling
//!    [`WatchHandle::stop`]) runs [`FsWatcher`](emend_core::watcher::FsWatcher)'s
//!    `Drop`, which stops the debouncer + joins the drain thread — so the watch
//!    lifecycle is tied to the handle's lifetime, no separate cancellation token
//!    needed (watching is event-driven via the observer, not a polled async
//!    call — see the task brief / contract "Global rules").
//!
//! ## Why a handle, not a `CancellationHandle` (deviation note)
//!
//! The contract's async members (`SearchHandle`/`AiHandle`, §5/§7) cancel via a
//! [`CancellationToken`](tokio_util::sync::CancellationToken). The watcher is
//! **not** a tokio task — `notify` runs on its own OS threads and the core's
//! `FsWatcher` already owns its teardown via `Drop` (stop debouncer → close
//! channel → join drain thread). Modeling cancellation as "drop the handle" is
//! therefore both simpler and structurally correct: there is no future to
//! `select!` on, and reusing `CancellationToken` here would add a token nobody
//! observes. [`WatchHandle::stop`] exists for callers who want a deterministic,
//! intention-revealing teardown before the `Arc` is released.

use crate::error::FfiError;
use crate::handles::DocObserver;
use emend_core::watcher::{
    apply_conflict_choice as core_apply_conflict_choice, ChangeEvent as CoreChangeEvent,
    ConflictChoice as CoreConflictChoice, ConflictState as CoreConflictState, FileIdentity,
    FsWatcher, WatchObserver,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A classified external change, surfaced to Swift after self-write suppression
/// (FFI contract §1; FR-006/006b). The FFI mirror of
/// [`emend_core::watcher::ChangeEvent`].
///
/// A move/rename is delivered as **one** [`ChangeEvent::Renamed`] carrying both
/// endpoints — never a `Removed`+`Created` pair (FR-006b). A self-write echo is
/// dropped in the core's pipeline and never reaches this type (FR-006a).
///
/// The [`From`] below is exhaustive (no wildcard), so a new core variant fails
/// the match at compile time until mirrored here.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ChangeEvent {
    /// A new file or folder appeared on disk.
    Created {
        /// Absolute path of the new entry.
        path: String,
    },
    /// An existing file changed in place (the FR-006c conflict trigger).
    Modified {
        /// Absolute path of the changed file.
        path: String,
    },
    /// A file or folder was removed from disk.
    Removed {
        /// Absolute path of the removed entry.
        path: String,
    },
    /// A move/rename delivered as one event with both endpoints (FR-006b).
    Renamed {
        /// The path before the move.
        from: String,
        /// The path after the move.
        to: String,
    },
}

impl From<CoreChangeEvent> for ChangeEvent {
    /// Exhaustive projection — no wildcard arm.
    fn from(change: CoreChangeEvent) -> Self {
        match change {
            CoreChangeEvent::Created { path } => Self::Created { path },
            CoreChangeEvent::Modified { path } => Self::Modified { path },
            CoreChangeEvent::Removed { path } => Self::Removed { path },
            CoreChangeEvent::Renamed { from, to } => Self::Renamed { from, to },
        }
    }
}

/// The conflict status of an open document relative to its file on disk (FFI
/// contract §3 `conflict_state`; FR-006c). The FFI mirror of
/// [`emend_core::watcher::ConflictState`].
///
/// The contract sketches `Clean | DirtyClean | DirtyExternalChanged`; this
/// matches it variant-for-variant. The [`From`] impls below are exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConflictState {
    /// In-memory buffer matches disk; no unsaved edits.
    Clean,
    /// Unsaved in-memory edits exist, but disk has not changed under them.
    DirtyClean,
    /// Unsaved edits AND the file changed on disk underneath them — a conflict
    /// the user must resolve (no version is auto-discarded).
    DirtyExternalChanged,
}

impl From<CoreConflictState> for ConflictState {
    /// Exhaustive projection — no wildcard arm.
    fn from(state: CoreConflictState) -> Self {
        match state {
            CoreConflictState::Clean => Self::Clean,
            CoreConflictState::DirtyClean => Self::DirtyClean,
            CoreConflictState::DirtyExternalChanged => Self::DirtyExternalChanged,
        }
    }
}

impl From<ConflictState> for CoreConflictState {
    fn from(state: ConflictState) -> Self {
        match state {
            ConflictState::Clean => Self::Clean,
            ConflictState::DirtyClean => Self::DirtyClean,
            ConflictState::DirtyExternalChanged => Self::DirtyExternalChanged,
        }
    }
}

/// The user's resolution of a [`ConflictState::DirtyExternalChanged`] conflict
/// (FFI contract §3 `resolve_conflict`; FR-006c). The FFI mirror of
/// [`emend_core::watcher::ConflictChoice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConflictChoice {
    /// Discard the in-memory edits and reload the on-disk version.
    ReloadFromDisk,
    /// Keep the in-memory edits; the next save overwrites the disk version.
    KeepMine,
}

impl From<ConflictChoice> for CoreConflictChoice {
    fn from(choice: ConflictChoice) -> Self {
        match choice {
            ConflictChoice::ReloadFromDisk => Self::ReloadFromDisk,
            ConflictChoice::KeepMine => Self::KeepMine,
        }
    }
}

/// Apply a user's [`ConflictChoice`] to a [`ConflictState`], returning the state
/// the document moves to (FFI contract §3; FR-006c state machine).
///
/// Pure projection of [`emend_core::watcher::apply_conflict_choice`]: only
/// [`ConflictState::DirtyExternalChanged`] is resolvable; applying a choice to
/// any other state is a no-op that returns it unchanged (a stale UI action must
/// not corrupt the model).
#[uniffi::export]
#[must_use]
pub fn apply_conflict_choice(state: ConflictState, choice: ConflictChoice) -> ConflictState {
    core_apply_conflict_choice(state.into(), choice.into()).into()
}

/// Rust-side adapter that implements the core's
/// [`WatchObserver`](emend_core::watcher::WatchObserver) by forwarding each
/// classified change to a foreign [`DocObserver`].
///
/// The watcher's drain thread holds this as `Arc<dyn WatchObserver>` and calls
/// [`on_change`](WatchObserver::on_change) once per surviving change; we project
/// the core [`ChangeEvent`](CoreChangeEvent) to the FFI [`ChangeEvent`] and hand
/// it to the Swift-implemented [`DocObserver::on_fs_change`]. This is the only
/// place the two trait worlds meet, keeping the core's `WatchObserver` free of
/// any FFI types (Constitution V).
struct ObserverBridge {
    observer: Arc<dyn DocObserver>,
}

impl WatchObserver for ObserverBridge {
    fn on_change(&self, change: CoreChangeEvent) {
        // Non-reentrant by contract: the foreign side queues work rather than
        // calling back into the core from here. We just project + forward.
        self.observer.on_fs_change(change.into());
    }
}

/// Live filesystem-watch handle exported to Swift (FFI contract §1; FR-006).
///
/// Wraps the core's [`FsWatcher`]. Handed to Swift as `Arc<Self>`; the watch
/// runs on the core's own background threads and pushes [`ChangeEvent`]s to the
/// foreign [`DocObserver`] via [`ObserverBridge`]. Dropping the last `Arc` (or
/// calling [`Self::stop`]) runs `FsWatcher`'s `Drop`, which stops the debouncer
/// and joins the drain thread — so **no further callbacks fire after teardown**.
///
/// The inner watcher is behind a `Mutex<Option<FsWatcher>>`: `Some` while live,
/// `None` after an explicit [`Self::stop`]. [`Self::record_self_write`] still
/// works only while live (a stopped watcher has nothing to suppress).
#[derive(uniffi::Object)]
pub struct WatchHandle {
    /// `None` once stopped. The `Mutex` gives interior mutability for the
    /// exported `&self` methods; dropping the `FsWatcher` inside it tears the
    /// watch down.
    watcher: Mutex<Option<FsWatcher>>,
}

impl std::fmt::Debug for WatchHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchHandle")
            .field(
                "watching",
                &self.watcher.try_lock().map(|g| g.is_some()).ok(),
            )
            .finish()
    }
}

impl WatchHandle {
    /// Lock the inner watcher, mapping mutex poisoning (a prior panic while the
    /// lock was held — unreachable given the no-panic posture, but handled rather
    /// than `unwrap`ped per NFR-003) to [`FfiError::Internal`].
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<FsWatcher>>, FfiError> {
        self.watcher.lock().map_err(|_| FfiError::Internal {
            detail: "watch handle lock poisoned".to_owned(),
        })
    }
}

#[uniffi::export]
impl WatchHandle {
    /// Register that the app just atomically wrote `path` with the given
    /// `(mtime_ns, len)` identity, so the matching debounced echo is suppressed
    /// and does **not** surface as an external change (FR-006a).
    ///
    /// The autosave layer calls this immediately after a durable
    /// [`flush`](crate::document::OpenDocHandle::flush) (or any
    /// [`write_atomic`](emend_core::fs::write_atomic)) completes and the target
    /// has been stat'd, *before* the FSEvents echo arrives. A genuine
    /// third-party write landing in the same window carries a different identity
    /// and is **not** suppressed (the core matches on identity, not a bare time
    /// window).
    ///
    /// A no-op after [`Self::stop`] (a stopped watcher has nothing to suppress).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn record_self_write(&self, path: String, mtime_ns: u64, len: u64) -> Result<(), FfiError> {
        let guard = self.lock()?;
        if let Some(watcher) = guard.as_ref() {
            watcher.record_self_write(&path, FileIdentity::new(mtime_ns, len));
        }
        Ok(())
    }

    /// Stop watching: tear down the underlying [`FsWatcher`] (stop the debouncer,
    /// join the drain thread) so no further [`ChangeEvent`]s fire.
    ///
    /// Idempotent: stopping an already-stopped handle is a no-op. Releasing the
    /// last `Arc<WatchHandle>` also stops the watch via [`Drop`]; this is for
    /// callers who want a deterministic, intention-revealing teardown.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn stop(&self) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        // Dropping the `FsWatcher` (taken out of the `Option`) runs its `Drop`,
        // which stops the debouncer and joins the drain thread.
        drop(guard.take());
        Ok(())
    }
}

/// Start watching `root` recursively, pushing classified external changes to the
/// foreign `observer` (FFI contract §1; FR-006).
///
/// Returns a [`WatchHandle`] whose lifetime owns the watch: hold it to keep
/// receiving [`ChangeEvent`]s; drop it (or call [`WatchHandle::stop`]) to stop.
/// Self-write suppression (FR-006a) is wired through
/// [`WatchHandle::record_self_write`]. Move correlation (FR-006b) and the
/// conflict truth table (FR-006c) are handled in the core.
///
/// # Errors
///
/// [`FfiError::IoFailure`] if the debouncer cannot be created or the watch
/// cannot be registered (e.g. `root` is gone or unreadable);
/// [`FfiError::NotFound`] / [`FfiError::PermissionDenied`] via the core's IO
/// mapping.
#[uniffi::export]
pub fn start_watching(
    root: String,
    observer: Arc<dyn DocObserver>,
) -> Result<Arc<WatchHandle>, FfiError> {
    let bridge: Arc<dyn WatchObserver> = Arc::new(ObserverBridge { observer });
    let watcher = FsWatcher::watch(Path::new(&root), bridge)?;
    Ok(Arc::new(WatchHandle {
        watcher: Mutex::new(Some(watcher)),
    }))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{
        apply_conflict_choice, start_watching, ChangeEvent, ConflictChoice, ConflictState,
    };
    use crate::handles::DocObserver;
    use emend_core::watcher::{
        ChangeEvent as CoreChangeEvent, ConflictChoice as CoreConflictChoice,
        ConflictState as CoreConflictState,
    };
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A `DocObserver` that records every pushed FS change for assertions.
    struct RecordingObserver {
        changes: Mutex<Vec<ChangeEvent>>,
    }

    impl DocObserver for RecordingObserver {
        fn on_derived_changed(&self) {}
        fn on_fs_change(&self, change: ChangeEvent) {
            if let Ok(mut v) = self.changes.lock() {
                v.push(change);
            }
        }
    }

    #[test]
    fn change_event_projection_is_exhaustive_and_faithful() {
        assert_eq!(
            ChangeEvent::from(CoreChangeEvent::Created {
                path: "/r/a.md".to_owned()
            }),
            ChangeEvent::Created {
                path: "/r/a.md".to_owned()
            }
        );
        assert_eq!(
            ChangeEvent::from(CoreChangeEvent::Renamed {
                from: "/r/old.md".to_owned(),
                to: "/r/new.md".to_owned(),
            }),
            ChangeEvent::Renamed {
                from: "/r/old.md".to_owned(),
                to: "/r/new.md".to_owned(),
            }
        );
    }

    #[test]
    fn conflict_state_round_trips_through_core() {
        for state in [
            ConflictState::Clean,
            ConflictState::DirtyClean,
            ConflictState::DirtyExternalChanged,
        ] {
            let core: CoreConflictState = state.into();
            let back: ConflictState = core.into();
            assert_eq!(state, back);
        }
    }

    #[test]
    fn conflict_choice_projects_to_core() {
        assert_eq!(
            CoreConflictChoice::from(ConflictChoice::ReloadFromDisk),
            CoreConflictChoice::ReloadFromDisk
        );
        assert_eq!(
            CoreConflictChoice::from(ConflictChoice::KeepMine),
            CoreConflictChoice::KeepMine
        );
    }

    #[test]
    fn apply_conflict_choice_mirrors_core_truth_table() {
        // Reload from a conflict → Clean; KeepMine from a conflict → DirtyClean.
        assert_eq!(
            apply_conflict_choice(
                ConflictState::DirtyExternalChanged,
                ConflictChoice::ReloadFromDisk
            ),
            ConflictState::Clean
        );
        assert_eq!(
            apply_conflict_choice(
                ConflictState::DirtyExternalChanged,
                ConflictChoice::KeepMine
            ),
            ConflictState::DirtyClean
        );
        // A choice on a non-conflict state is a no-op.
        assert_eq!(
            apply_conflict_choice(ConflictState::Clean, ConflictChoice::ReloadFromDisk),
            ConflictState::Clean
        );
    }

    #[test]
    fn watch_delivers_a_creation_to_the_observer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let observer = Arc::new(RecordingObserver {
            changes: Mutex::new(Vec::new()),
        });
        let handle = start_watching(dir.path().to_string_lossy().into_owned(), observer.clone())
            .expect("start watching");

        // Create a file under the watched root; the watcher should classify it
        // as Created and push it to the observer (after the debounce window).
        std::fs::write(dir.path().join("new.md"), b"hi").expect("write note");

        // Poll up to a generous deadline for the debounced event to arrive.
        let deadline = Instant::now() + Duration::from_secs(10);
        let got_created = loop {
            let any_created = observer
                .changes
                .lock()
                .map(|v| v.iter().any(|c| matches!(c, ChangeEvent::Created { .. })))
                .unwrap_or(false);
            if any_created {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        // Stop the watch so no callbacks fire after we drop the observer.
        handle.stop().expect("stop");
        assert!(
            got_created,
            "a file creation under the watched root must reach the observer as Created"
        );
    }

    #[test]
    fn record_self_write_after_stop_is_a_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let observer = Arc::new(RecordingObserver {
            changes: Mutex::new(Vec::new()),
        });
        let handle =
            start_watching(dir.path().to_string_lossy().into_owned(), observer).expect("start");

        handle.stop().expect("stop");
        // After stop there is no watcher; recording a self-write is a clean no-op.
        handle
            .record_self_write("/r/a.md".to_owned(), 1, 1)
            .expect("record after stop is a no-op");
        // Stopping again is idempotent.
        handle.stop().expect("second stop");
    }
}
