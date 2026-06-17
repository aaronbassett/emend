//! Live external-change detection + the conflict model (US2 · FR-006, FR-006a,
//! FR-006b, FR-006c, FR-004a; research §B3/§B4).
//!
//! The editor must reflect what other tools do to the files on disk — `git
//! checkout`, an AI agent rewriting notes, a second editor — *without* mistaking
//! its **own** atomic autosaves for a third-party change (FR-006a), and without
//! ever silently clobbering unsaved edits when disk and buffer diverge
//! (FR-006c). This module is the core that delivers that.
//!
//! ## Two layers, deliberately split (the determinism contract)
//!
//! Real `notify`/FSEvents delivery is timing-nondeterministic and
//! directory-coalescing, so the logic that *decides what an event means* is kept
//! strictly separate from the OS event source and is exercised by feeding it
//! **synthetic** debounced events:
//!
//! 1. **Pure classification core** — no threads, no clock-of-its-own, no OS:
//!    * [`classify`] maps one [`DebouncedEvent`] to a [`RawChange`] (and folds the
//!      debouncer's `RenameMode::Both` correlation into a single
//!      [`RawChange::Renamed`] — one move event, never delete+create, FR-006b).
//!    * [`SuppressionRegistry`] records the post-write `(mtime,len)` of each
//!      atomic autosave with a short TTL and **drops** the matching debounced
//!      event so a self-write never echoes (FR-006a). Time is *injected*
//!      ([`Instant`] arguments) so TTL/expiry are tested without sleeping.
//!    * [`resolve_conflict`] is the FR-006c truth table as a pure function.
//!    All of the above are `&self`/value functions over plain data — unit-tested
//!    exhaustively and deterministically (no real fs, no race).
//!
//! 2. **Thin real-`notify` wrapper** ([`FsWatcher`]) — owns a
//!    `notify-debouncer-full` `Debouncer`, whose own background threads post
//!    `DebounceEventResult`s to a [`std::sync::mpsc`] channel (NOT tokio — the
//!    core stays free of an async runtime, Constitution V). A dedicated
//!    `std::thread` drains the channel, runs each event through the *pure* core
//!    above, and dispatches the surviving [`ChangeEvent`]s to a foreign
//!    [`WatchObserver`]. This layer is intentionally tiny; the smarts live in the
//!    deterministically-tested core.
//!
//! ## Self-write suppression keyed by identity, not a time window (FR-006a)
//!
//! After every atomic autosave (research §B4: tempfile → fsync → rename → fsync
//! dir), the writer stats the target and calls [`SuppressionRegistry::record`]
//! with the resulting [`FileIdentity`] (`mtime` + `len`). When a debounced modify
//! event for that path arrives, [`SuppressionRegistry::take_if_self_write`]
//! suppresses it **iff** the path's *current* identity equals an unexpired
//! recorded one. Matching on identity (not a bare "ignore this path for 400 ms"
//! window) means a genuine third-party edit landing in the same window — which
//! changes `mtime`/`len` away from what we recorded — is **not** suppressed
//! (contract obligation 4). The entry is consumed on match (one record suppresses
//! one echo) and expires after [`SuppressionRegistry::TTL`] regardless.
//!
//! ## Conflict truth table (FR-006c)
//!
//! [`resolve_conflict`] encodes exactly:
//! * buffer **clean** + external change → [`ConflictAction::Reload`] (silent
//!   reload; the on-disk version wins because nothing local is at stake).
//! * buffer **dirty** + external change → [`ConflictAction::PreserveLocal`] (keep
//!   the user's unsaved edits, flag the document externally-changed; the UI then
//!   offers Reload-vs-Keep via [`ConflictChoice`] — never an auto-overwrite).
//! * a recognized **self-write** → [`ConflictAction::Ignore`] (our own save, not
//!   a change at all).

use crate::EmendError;
use notify::RecursiveMode;
use notify_debouncer_full::notify::event::{EventKind, ModifyKind, RenameMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer};
use notify_debouncer_full::{notify::RecommendedWatcher, RecommendedCache};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// ===========================================================================
// Pure data types (no OS, no threads — the deterministically-tested core)
// ===========================================================================

/// The stat-derived identity of a file at a moment in time: its modification
/// time (nanoseconds since the Unix epoch) and byte length.
///
/// This is the "fingerprint" the self-write suppression matches on (FR-006a):
/// after an atomic autosave we record the target's `(mtime,len)`, and only an
/// event whose path still carries that exact identity is treated as our own
/// echo. A third-party write changes at least one of the two, so it is *not*
/// suppressed. FSEvents is directory-granular and coalescing, so research §B3 is
/// explicit that **stat identity, not event identity, is the source of truth**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    /// Modification time in nanoseconds since the Unix epoch. A `u64` (not a
    /// `SystemTime`) so the type is `Copy`, comparable, and FFI-friendly later.
    pub mtime_ns: u64,
    /// File length in bytes.
    pub len: u64,
}

impl FileIdentity {
    /// Build an identity from the raw fields.
    #[must_use]
    pub const fn new(mtime_ns: u64, len: u64) -> Self {
        Self { mtime_ns, len }
    }

    /// Stat `path` and capture its current identity.
    ///
    /// The atomic writer calls this immediately after a `persist`+dir-fsync so the
    /// `(mtime,len)` it feeds [`SuppressionRegistry::record`] is the exact one the
    /// watcher will observe for that write.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
    /// [`EmendError::IoFailure`] if the path cannot be stat'd. A `mtime` the host
    /// reports as before the Unix epoch is clamped to `0` rather than erroring
    /// (it only weakens the match, never produces a false suppression).
    pub fn of_path(path: impl AsRef<Path>) -> Result<Self, EmendError> {
        let path = path.as_ref();
        let meta = std::fs::metadata(path).map_err(|e| map_io(path, &e))?;
        let len = meta.len();
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        Ok(Self { mtime_ns, len })
    }
}

/// A classified external change, *before* self-write suppression is applied.
///
/// [`classify`] produces this directly from a [`DebouncedEvent`]; the public
/// [`ChangeEvent`] is the same shape minus the events suppression drops. Splitting
/// the two keeps "what kind of change is this?" (pure, total) separate from "is
/// this one of ours?" (stateful, registry-driven).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawChange {
    /// A file or folder appeared.
    Created { path: PathBuf },
    /// An existing file's content/metadata changed in place.
    Modified { path: PathBuf },
    /// A file or folder was removed.
    Removed { path: PathBuf },
    /// A rename/move the debouncer correlated into ONE event (FR-006b): exactly
    /// one logical move, never a delete+create pair, with both endpoints known.
    Renamed { from: PathBuf, to: PathBuf },
}

/// An external change surfaced to the [`WatchObserver`] after suppression.
///
/// Structurally identical to [`RawChange`]; it is a distinct type so the public
/// callback surface is the *post-suppression* set (a self-write [`RawChange`] is
/// dropped and never becomes a `ChangeEvent`). Projects cleanly onto an FFI
/// record later (T059) without importing any FFI machinery here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeEvent {
    /// A new file/folder appeared on disk.
    Created { path: String },
    /// An open or tracked file changed in place (the FR-006c trigger).
    Modified { path: String },
    /// A file/folder was removed from disk.
    Removed { path: String },
    /// A move/rename, delivered as one event with both endpoints (FR-006b);
    /// the index updates in place via [`crate::index::Index::rename`].
    Renamed { from: String, to: String },
}

impl From<RawChange> for ChangeEvent {
    fn from(raw: RawChange) -> Self {
        match raw {
            RawChange::Created { path } => Self::Created {
                path: path.to_string_lossy().into_owned(),
            },
            RawChange::Modified { path } => Self::Modified {
                path: path.to_string_lossy().into_owned(),
            },
            RawChange::Removed { path } => Self::Removed {
                path: path.to_string_lossy().into_owned(),
            },
            RawChange::Renamed { from, to } => Self::Renamed {
                from: from.to_string_lossy().into_owned(),
                to: to.to_string_lossy().into_owned(),
            },
        }
    }
}

/// The conflict status of an open document relative to its file on disk
/// (FR-006c; mirrors the FFI contract's `ConflictState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictState {
    /// In-memory buffer matches disk; no unsaved edits.
    Clean,
    /// Unsaved in-memory edits exist, but disk has not changed under them.
    DirtyClean,
    /// Unsaved edits AND the file changed on disk underneath them — a conflict
    /// the user must resolve (no version is auto-discarded).
    DirtyExternalChanged,
}

/// The user's resolution of a [`ConflictState::DirtyExternalChanged`] conflict
/// (FR-006c; mirrors the FFI contract's `ConflictChoice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    /// Discard the in-memory edits and reload the on-disk version.
    ReloadFromDisk,
    /// Keep the in-memory edits; the next save overwrites the disk version.
    KeepMine,
}

/// What the core should do with an observed external change, given the buffer's
/// dirty state (the pure FR-006c decision; see [`resolve_conflict`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    /// Reload from disk silently — the buffer is clean, nothing local is lost.
    Reload,
    /// Preserve the user's unsaved edits and flag the document externally
    /// changed; the UI then offers [`ConflictChoice`]. Never an auto-overwrite.
    PreserveLocal,
    /// The change is the app's own write (or otherwise a no-op): do nothing.
    Ignore,
}

/// The FR-006c truth table as a pure, total function.
///
/// * a recognized **self-write** → [`ConflictAction::Ignore`] (highest priority:
///   our own autosave is never a "change", regardless of dirty state);
/// * buffer **clean** + external change → [`ConflictAction::Reload`];
/// * buffer **dirty** + external change → [`ConflictAction::PreserveLocal`].
///
/// `is_self_write` is what [`SuppressionRegistry::take_if_self_write`] returns;
/// `buffer_dirty` is the open document's unsaved-edits flag. Keeping this a free
/// function (no `self`) makes the decision trivially and exhaustively testable.
#[must_use]
pub fn resolve_conflict(buffer_dirty: bool, is_self_write: bool) -> ConflictAction {
    if is_self_write {
        // Our own atomic save echoing back — not a change at all (FR-006a).
        ConflictAction::Ignore
    } else if buffer_dirty {
        // Unsaved edits + a third-party write underneath → preserve + flag,
        // never silently overwrite either side (FR-006c).
        ConflictAction::PreserveLocal
    } else {
        // Clean buffer + external change → silent reload (FR-006c).
        ConflictAction::Reload
    }
}

/// Given the current [`ConflictState`] and the user's [`ConflictChoice`], the
/// state the document moves to (FR-006c state machine, data-model
/// "OpenDocument / Tab").
///
/// Only [`ConflictState::DirtyExternalChanged`] is resolvable:
/// * `ReloadFromDisk` → discard local edits, reload → [`ConflictState::Clean`];
/// * `KeepMine` → keep edits; the conflict flag clears (the next save wins) →
///   [`ConflictState::DirtyClean`].
///
/// Applying a choice to a non-conflicted state is a no-op that returns the state
/// unchanged (a stale UI action must not corrupt the model).
#[must_use]
pub fn apply_conflict_choice(state: ConflictState, choice: ConflictChoice) -> ConflictState {
    match state {
        ConflictState::DirtyExternalChanged => match choice {
            ConflictChoice::ReloadFromDisk => ConflictState::Clean,
            ConflictChoice::KeepMine => ConflictState::DirtyClean,
        },
        // Not a conflict — nothing to resolve.
        other => other,
    }
}

// ===========================================================================
// Pure classification: DebouncedEvent -> RawChange (move correlation, FR-006b)
// ===========================================================================

/// Classify one debounced event into a single [`RawChange`], or `None` for an
/// event we don't surface (access events, metadata-only noise, an `Other`
/// without a usable path).
///
/// The debouncer's `FileIdCache` has already stitched a rename's `From`+`To`
/// into one `Modify(Name(RenameMode::Both))` carrying `[from, to]`, so a move is
/// **one** [`RawChange::Renamed`] here — never a delete+create pair (FR-006b,
/// research §B3). A bare `From`/`To`/`Any` rename that could not be paired
/// degrades to `Removed`/`Created` respectively (the spec permits the degraded
/// form when the OS doesn't give us both ends).
#[must_use]
pub fn classify(event: &DebouncedEvent) -> Option<RawChange> {
    let paths = &event.paths;
    match event.kind {
        EventKind::Create(_) => paths
            .first()
            .map(|p| RawChange::Created { path: p.clone() }),

        EventKind::Remove(_) => paths
            .first()
            .map(|p| RawChange::Removed { path: p.clone() }),

        EventKind::Modify(ModifyKind::Name(rename_mode)) => classify_rename(rename_mode, paths),

        // Content/metadata/size change of an existing file. `Modify(Any)` is the
        // common FSEvents coalesced "something changed here" — treat as modified.
        EventKind::Modify(_) => paths
            .first()
            .map(|p| RawChange::Modified { path: p.clone() }),

        // Access (open/close/read), `Any`, and `Other` carry no actionable
        // mutation for the editor refresh path — drop them.
        EventKind::Access(_) | EventKind::Any | EventKind::Other => None,
    }
}

/// Classify the rename family. `Both` is the correlated move (one event, both
/// endpoints); the unpaired modes degrade as the spec permits.
fn classify_rename(mode: RenameMode, paths: &[PathBuf]) -> Option<RawChange> {
    match mode {
        // The correlated move: paths are [from, to] in that order (notify
        // documents this ordering; the debouncer preserves it).
        RenameMode::Both => match (paths.first(), paths.get(1)) {
            (Some(from), Some(to)) => Some(RawChange::Renamed {
                from: from.clone(),
                to: to.clone(),
            }),
            // `Both` without two paths is malformed; degrade to a modify of
            // whatever single path we did get rather than dropping silently.
            _ => paths
                .first()
                .map(|p| RawChange::Modified { path: p.clone() }),
        },
        // The renamed-away endpoint with no correlated target → looks like a
        // removal from the watched subtree.
        RenameMode::From => paths
            .first()
            .map(|p| RawChange::Removed { path: p.clone() }),
        // The rename target with no correlated source → looks like a creation.
        RenameMode::To => paths
            .first()
            .map(|p| RawChange::Created { path: p.clone() }),
        // `Any`/`Other` rename without endpoints we can pair: surface as a
        // modify of the named path so the view still refreshes (FR-006).
        RenameMode::Any | RenameMode::Other => paths
            .first()
            .map(|p| RawChange::Modified { path: p.clone() }),
    }
}

// ===========================================================================
// Self-write suppression registry (FR-006a)
// ===========================================================================

/// One recorded self-write: the identity we wrote plus when the record expires.
#[derive(Debug, Clone, Copy)]
struct ExpectedWrite {
    identity: FileIdentity,
    expires_at: Instant,
}

/// Drops the app's own atomic-autosave echoes so they never surface as external
/// changes (FR-006a).
///
/// Keyed by **canonical path** → the last self-written [`FileIdentity`] plus a
/// short TTL. The matching rule is identity-equality, not a bare time window, so
/// a real third-party edit in the same window (different `mtime`/`len`) is *not*
/// suppressed (FR-006a, contract obligation 4). A match **consumes** the record
/// (one save suppresses one echo); records also expire after [`Self::TTL`].
///
/// Time is taken as an [`Instant`] **argument** on every method rather than read
/// from a clock inside, so suppression/expiry are tested deterministically with
/// synthetic instants — no `sleep`, no race (the determinism contract).
#[derive(Debug, Default)]
pub struct SuppressionRegistry {
    pending: HashMap<PathBuf, ExpectedWrite>,
}

impl SuppressionRegistry {
    /// How long a recorded self-write stays eligible to suppress its echo. The
    /// debounced FSEvents notification for an autosave arrives within the
    /// watcher debounce (~400 ms, research §D); a generous multiple of that
    /// covers FSEvents latency without keeping stale records that could mask a
    /// later genuine edit.
    pub const TTL: Duration = Duration::from_secs(2);

    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that *we* just wrote `path` with `identity`, eligible to suppress
    /// the matching echo until `now + TTL`. Called right after an atomic save.
    ///
    /// A fresh record for a path replaces any prior one (the latest write is the
    /// one whose echo we expect next).
    pub fn record(&mut self, path: impl AsRef<Path>, identity: FileIdentity, now: Instant) {
        self.pending.insert(
            path.as_ref().to_path_buf(),
            ExpectedWrite {
                identity,
                expires_at: now + Self::TTL,
            },
        );
    }

    /// Decide whether an event for `path` carrying `observed` identity is our own
    /// write, consuming the record if so (FR-006a).
    ///
    /// Returns `true` (suppress this event) **iff** there is an unexpired record
    /// for `path` whose identity equals `observed`. On a match the record is
    /// removed so it can only suppress a single echo. A record present but
    /// expired (at `now`), or present with a *different* identity (a genuine
    /// third-party write landed instead), returns `false` and is left in place
    /// for its own potential echo / natural expiry.
    #[must_use]
    pub fn take_if_self_write(
        &mut self,
        path: impl AsRef<Path>,
        observed: FileIdentity,
        now: Instant,
    ) -> bool {
        let key = path.as_ref();
        let Some(rec) = self.pending.get(key) else {
            return false;
        };
        if now >= rec.expires_at {
            // Stale: a real edit could have happened since; never suppress on an
            // expired record. Drop it so it can't mask a future write either.
            self.pending.remove(key);
            return false;
        }
        if rec.identity == observed {
            // Our own echo — consume the record and suppress.
            self.pending.remove(key);
            true
        } else {
            // Same path, different identity → a genuine external write replaced
            // the one we expected. Do NOT suppress; leave the record to expire.
            false
        }
    }

    /// Drop every record whose TTL has elapsed at `now`. Optional housekeeping:
    /// [`Self::take_if_self_write`] already expires lazily on access, so this is
    /// only needed to bound memory for paths whose echo never arrives.
    pub fn evict_expired(&mut self, now: Instant) {
        self.pending.retain(|_, rec| now < rec.expires_at);
    }

    /// Number of live (not-yet-consumed) records. For tests / introspection.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

// ===========================================================================
// Full pure pipeline: classify + suppress a batch (the unit-tested seam)
// ===========================================================================

/// Run a batch of debounced events through the *entire pure pipeline* and return
/// the surviving [`ChangeEvent`]s (classification + move correlation + self-write
/// suppression), bounded by construction.
///
/// `identity_of` resolves a path's current [`FileIdentity`] (the real watcher
/// passes a `std::fs::metadata`-backed closure; tests pass a synthetic map), so
/// this function itself never touches the filesystem and is fully deterministic
/// given its inputs. A modify whose path stats to the recorded self-write
/// identity is dropped (FR-006a); creates/removes/renames pass through (a
/// self-write is an in-place content change, classified `Modified`).
///
/// Boundedness (FR-006b): the output has at most one entry per input event, and
/// the debouncer already coalesced the burst, so a 10k-file `git checkout`
/// yields a bounded, per-path-deduplicated set rather than an unbounded stream.
pub fn process_batch<F>(
    events: &[DebouncedEvent],
    suppression: &mut SuppressionRegistry,
    now: Instant,
    mut identity_of: F,
) -> Vec<ChangeEvent>
where
    F: FnMut(&Path) -> Option<FileIdentity>,
{
    let mut out = Vec::with_capacity(events.len());
    for event in events {
        let Some(raw) = classify(event) else {
            continue;
        };
        // Self-write suppression only applies to in-place content changes: an
        // autosave is a rename-into-place that the OS surfaces as a modify of the
        // target path. Creates/removes/renames-of-other-files are never our
        // autosave echo, so they pass straight through.
        if let RawChange::Modified { path } = &raw {
            if let Some(observed) = identity_of(path) {
                if suppression.take_if_self_write(path, observed, now) {
                    continue; // our own write — drop it (FR-006a)
                }
            }
        }
        out.push(ChangeEvent::from(raw));
    }
    out
}

// ===========================================================================
// Thin real-`notify` wrapper (intentionally tiny; logic lives above)
// ===========================================================================

/// Sink the [`FsWatcher`] calls once per classified, non-suppressed external
/// change (FR-006). **The owner implements this; the watcher calls it** from its
/// drain thread.
///
/// `Send + Sync` because the watcher invokes it from a background `std::thread`.
/// Shaped to project onto a UniFFI foreign-trait callback later (T059) without
/// importing any FFI machinery here — like `crate::index::SearchHit` vs the FFI
/// `SearchHit`, the boundary type is defined in `emend-ffi`.
pub trait WatchObserver: Send + Sync {
    /// A classified external change survived suppression and should refresh the
    /// view / index. Non-reentrant: do not call back into the watcher from here.
    fn on_change(&self, change: ChangeEvent);
}

/// A live recursive filesystem watcher over one root, with self-write
/// suppression and move correlation (the thin OS-facing layer over the pure core
/// above).
///
/// Owns a `notify-debouncer-full` `Debouncer` (its FSEvents + debounce threads)
/// and a single drain `std::thread` that turns each debounced batch into
/// [`ChangeEvent`]s via [`process_batch`] and forwards them to the
/// [`WatchObserver`]. **No tokio** — `notify` uses its own threads and we use a
/// `std::sync::mpsc` channel (Constitution V). Dropping the watcher stops both.
///
/// The shared [`SuppressionRegistry`] is exposed via [`Self::record_self_write`]
/// so the autosave layer can register a write's `(mtime,len)` the instant it
/// completes, before the echo arrives.
pub struct FsWatcher {
    // The debouncer owns the FSEvents + debounce threads AND the only remaining
    // clone of the channel `Sender`. Held in an `Option` so `Drop` can `stop()`
    // it (which consumes it and drops that sender), letting the drain thread see
    // the channel close. `RecommendedWatcher`/`RecommendedCache` are the defaults.
    debouncer: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
    suppression: Arc<Mutex<SuppressionRegistry>>,
    // The drain thread; joined on drop so no events fire after teardown.
    drain: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for FsWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FsWatcher")
            .field("suppression", &"<SuppressionRegistry>")
            .field("drain", &self.drain.is_some())
            .finish()
    }
}

impl FsWatcher {
    /// The debounce window: external changes are coalesced over ~400 ms before a
    /// batch is delivered (research §D / FR-006).
    pub const DEBOUNCE: Duration = Duration::from_millis(400);

    /// Start watching `root` recursively, delivering classified external changes
    /// to `observer`.
    ///
    /// Spawns the debouncer (its own threads) plus one drain thread. Returns once
    /// the watch is registered; events flow asynchronously thereafter.
    ///
    /// # Errors
    ///
    /// [`EmendError::IoFailure`] if the debouncer cannot be created or the watch
    /// cannot be registered (e.g. the path is gone or unreadable).
    pub fn watch(
        root: impl AsRef<Path>,
        observer: Arc<dyn WatchObserver>,
    ) -> Result<Self, EmendError> {
        let root = root.as_ref();
        let suppression = Arc::new(Mutex::new(SuppressionRegistry::new()));

        // The debouncer posts `DebounceEventResult`s to this std mpsc channel
        // (it implements `DebounceEventHandler` for `mpsc::Sender`). `new_debouncer`
        // takes ownership of the `Sender`, so it becomes the *only* sender — when
        // the debouncer is stopped/dropped, the channel closes and the drain
        // thread's `recv()` returns `Err`, which is how teardown unblocks it.
        let (tx, rx) = channel::<DebounceEventResult>();

        let mut debouncer =
            new_debouncer(Self::DEBOUNCE, None, tx).map_err(|e| watch_io(root, &e))?;
        debouncer
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| watch_io(root, &e))?;

        let drain_suppression = Arc::clone(&suppression);
        let drain = std::thread::Builder::new()
            .name("emend-watch-drain".to_owned())
            .spawn(move || drain_loop(&rx, &drain_suppression, observer.as_ref()))
            .map_err(|e| EmendError::IoFailure {
                path: root.display().to_string(),
                detail: format!("failed to spawn watch drain thread: {e}"),
            })?;

        Ok(Self {
            debouncer: Some(debouncer),
            suppression,
            drain: Some(drain),
        })
    }

    /// Register that the app just atomically wrote `path` with `identity`, so the
    /// matching debounced echo is suppressed (FR-006a). Call immediately after
    /// [`crate::fs::write_atomic`] returns and the target has been stat'd.
    ///
    /// Uses `Instant::now()` as the record time; the pure [`SuppressionRegistry`]
    /// keeps time injectable for tests, but the live path reads the real clock.
    pub fn record_self_write(&self, path: impl AsRef<Path>, identity: FileIdentity) {
        if let Ok(mut reg) = self.suppression.lock() {
            reg.record(path, identity, Instant::now());
        }
        // A poisoned lock (a drain-thread panic) only means we miss one
        // suppression record → at worst a spurious reload of a clean buffer,
        // never data loss. We deliberately do not propagate it (FR-006a is
        // best-effort hygiene, not a correctness invariant).
    }
}

impl Drop for FsWatcher {
    fn drop(&mut self) {
        // Order matters: stop the debouncer FIRST. `stop()` consumes it (and the
        // FSEvents/debounce threads), which drops the channel's only `Sender`.
        // That closes the channel, so the drain thread's blocking `recv()` returns
        // `Err` and the loop exits — only THEN can we join it without deadlocking.
        if let Some(debouncer) = self.debouncer.take() {
            debouncer.stop();
        }
        if let Some(handle) = self.drain.take() {
            // A panicked drain thread is contained (no data loss, see drain_loop);
            // ignore the join result rather than propagate a panic out of `drop`.
            let _ = handle.join();
        }
    }
}

/// The drain thread body: block on the channel, run each debounced batch through
/// the pure pipeline, and dispatch survivors to the observer.
///
/// Exits when the channel closes (every sender dropped at watcher teardown).
fn drain_loop(
    rx: &std::sync::mpsc::Receiver<DebounceEventResult>,
    suppression: &Arc<Mutex<SuppressionRegistry>>,
    observer: &dyn WatchObserver,
) {
    while let Ok(result) = rx.recv() {
        let events = match result {
            Ok(events) => events,
            // A batch of watcher errors (e.g. a transient FSEvents hiccup) is not
            // actionable for the refresh path — skip it rather than tear down.
            Err(_errors) => continue,
        };

        let now = Instant::now();
        let changes = {
            // Hold the lock only across the pure pipeline, not the callbacks.
            let Ok(mut reg) = suppression.lock() else {
                // Poisoned by a prior panic: process without suppression rather
                // than stall (worst case is a spurious clean-buffer reload).
                continue;
            };
            process_batch(&events, &mut reg, now, |p| FileIdentity::of_path(p).ok())
        };

        for change in changes {
            observer.on_change(change);
        }
    }
}

// ===========================================================================
// Error mapping
// ===========================================================================

/// Map a `notify::Error` to an [`EmendError`] for the watch-setup path.
fn watch_io(path: &Path, err: &notify::Error) -> EmendError {
    EmendError::IoFailure {
        path: path.display().to_string(),
        detail: err.to_string(),
    }
}

/// Map a `std::io::Error` to an [`EmendError`], attaching the path — mirrors the
/// mapping in [`crate::fs`] / [`crate::workspace`].
fn map_io(path: &Path, err: &std::io::Error) -> EmendError {
    let path_str = path.display().to_string();
    match err.kind() {
        std::io::ErrorKind::NotFound => EmendError::NotFound { path: path_str },
        std::io::ErrorKind::PermissionDenied => EmendError::PermissionDenied { path: path_str },
        _ => EmendError::IoFailure {
            path: path_str,
            detail: err.to_string(),
        },
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
        reason = "unit test asserts on its own fixtures and results"
    )]

    use super::*;
    use notify_debouncer_full::notify::event::{CreateKind, RemoveKind};
    use notify_debouncer_full::notify::Event;

    // -- synthetic-event helpers ----------------------------------------------

    /// Build a synthetic `DebouncedEvent` with the given kind + paths, so the
    /// pure classifier is tested WITHOUT any real OS event (the determinism
    /// contract). `time` is irrelevant to classification, so use `Instant::now`.
    fn ev(kind: EventKind, paths: &[&str]) -> DebouncedEvent {
        let event = Event {
            kind,
            paths: paths.iter().map(PathBuf::from).collect(),
            attrs: Default::default(),
        };
        DebouncedEvent::new(event, Instant::now())
    }

    // -- classify: create / modify / remove -----------------------------------

    #[test]
    fn classify_create_is_created() {
        let e = ev(EventKind::Create(CreateKind::File), &["/r/new.md"]);
        assert_eq!(
            classify(&e),
            Some(RawChange::Created {
                path: PathBuf::from("/r/new.md")
            })
        );
    }

    #[test]
    fn classify_remove_is_removed() {
        let e = ev(EventKind::Remove(RemoveKind::File), &["/r/gone.md"]);
        assert_eq!(
            classify(&e),
            Some(RawChange::Removed {
                path: PathBuf::from("/r/gone.md")
            })
        );
    }

    #[test]
    fn classify_modify_data_is_modified() {
        let e = ev(EventKind::Modify(ModifyKind::Any), &["/r/edit.md"]);
        assert_eq!(
            classify(&e),
            Some(RawChange::Modified {
                path: PathBuf::from("/r/edit.md")
            })
        );
    }

    #[test]
    fn classify_access_and_any_are_dropped() {
        let e = ev(EventKind::Any, &["/r/x.md"]);
        assert_eq!(classify(&e), None);
    }

    // -- classify: rename correlation (FR-006b) -------------------------------

    #[test]
    fn classify_rename_both_is_one_rename_event() {
        // The debouncer correlates a move into ONE Both event with [from, to].
        let e = ev(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/r/old.md", "/r/new.md"],
        );
        assert_eq!(
            classify(&e),
            Some(RawChange::Renamed {
                from: PathBuf::from("/r/old.md"),
                to: PathBuf::from("/r/new.md"),
            }),
            "a correlated move must surface as exactly one Renamed, not delete+create"
        );
    }

    #[test]
    fn classify_rename_from_degrades_to_removed() {
        let e = ev(
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            &["/r/old.md"],
        );
        assert_eq!(
            classify(&e),
            Some(RawChange::Removed {
                path: PathBuf::from("/r/old.md")
            })
        );
    }

    #[test]
    fn classify_rename_to_degrades_to_created() {
        let e = ev(
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            &["/r/new.md"],
        );
        assert_eq!(
            classify(&e),
            Some(RawChange::Created {
                path: PathBuf::from("/r/new.md")
            })
        );
    }

    // -- suppression registry (FR-006a) ---------------------------------------

    #[test]
    fn self_write_with_matching_identity_is_suppressed_once() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        let id = FileIdentity::new(1_000, 42);
        reg.record("/r/a.md", id, t0);

        // First echo with the SAME identity is suppressed and consumes the record.
        assert!(reg.take_if_self_write("/r/a.md", id, t0));
        // A second echo finds no record → not suppressed (one record, one echo).
        assert!(!reg.take_if_self_write("/r/a.md", id, t0));
        assert_eq!(reg.pending_len(), 0);
    }

    #[test]
    fn third_party_edit_in_window_is_not_suppressed() {
        // FR-006a / contract obligation 4: a genuine external edit landing in the
        // suppression window has a DIFFERENT identity and must NOT be suppressed.
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        reg.record("/r/a.md", FileIdentity::new(1_000, 42), t0);

        // The observed identity differs (someone else wrote different bytes).
        let external = FileIdentity::new(2_000, 99);
        assert!(
            !reg.take_if_self_write("/r/a.md", external, t0),
            "a different identity in the same window is a real edit, not our echo"
        );
        // The record is left in place to await our own echo / natural expiry.
        assert_eq!(reg.pending_len(), 1);
    }

    #[test]
    fn expired_record_does_not_suppress() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        let id = FileIdentity::new(1_000, 42);
        reg.record("/r/a.md", id, t0);

        // At t0 + TTL the record is expired (>=): even a matching identity is not
        // suppressed, and the stale record is dropped.
        let later = t0 + SuppressionRegistry::TTL;
        assert!(!reg.take_if_self_write("/r/a.md", id, later));
        assert_eq!(reg.pending_len(), 0);
    }

    #[test]
    fn unknown_path_is_never_suppressed() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        assert!(!reg.take_if_self_write("/r/never-recorded.md", FileIdentity::new(1, 1), t0));
    }

    #[test]
    fn evict_expired_bounds_memory() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        reg.record("/r/a.md", FileIdentity::new(1, 1), t0);
        reg.record("/r/b.md", FileIdentity::new(2, 2), t0);
        assert_eq!(reg.pending_len(), 2);

        reg.evict_expired(t0 + SuppressionRegistry::TTL);
        assert_eq!(reg.pending_len(), 0, "all TTL-elapsed records are evicted");
    }

    // -- conflict truth table (FR-006c) ---------------------------------------

    #[test]
    fn resolve_conflict_clean_external_reloads() {
        assert_eq!(
            resolve_conflict(false, false),
            ConflictAction::Reload,
            "clean buffer + external change → silent reload"
        );
    }

    #[test]
    fn resolve_conflict_dirty_external_preserves_local() {
        assert_eq!(
            resolve_conflict(true, false),
            ConflictAction::PreserveLocal,
            "dirty buffer + external change → preserve local + flag conflict"
        );
    }

    #[test]
    fn resolve_conflict_self_write_is_ignored_regardless_of_dirty() {
        // A recognized self-write is never a change — dirty state is irrelevant.
        assert_eq!(resolve_conflict(false, true), ConflictAction::Ignore);
        assert_eq!(resolve_conflict(true, true), ConflictAction::Ignore);
    }

    #[test]
    fn apply_conflict_choice_resolves_only_a_conflict() {
        use ConflictState::{Clean, DirtyClean, DirtyExternalChanged};
        // Reload discards local edits.
        assert_eq!(
            apply_conflict_choice(DirtyExternalChanged, ConflictChoice::ReloadFromDisk),
            Clean
        );
        // Keep-mine clears the conflict but stays dirty.
        assert_eq!(
            apply_conflict_choice(DirtyExternalChanged, ConflictChoice::KeepMine),
            DirtyClean
        );
        // A choice applied to a non-conflict state is a no-op.
        assert_eq!(
            apply_conflict_choice(Clean, ConflictChoice::ReloadFromDisk),
            Clean
        );
        assert_eq!(
            apply_conflict_choice(DirtyClean, ConflictChoice::KeepMine),
            DirtyClean
        );
    }

    // -- full pure pipeline: process_batch ------------------------------------

    #[test]
    fn process_batch_suppresses_self_write_modify() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        let id = FileIdentity::new(7, 7);
        reg.record("/r/a.md", id, t0);

        let batch = vec![ev(EventKind::Modify(ModifyKind::Any), &["/r/a.md"])];
        // The path stats to the recorded identity → suppressed → zero output.
        let out = process_batch(&batch, &mut reg, t0, |_p| Some(id));
        assert!(
            out.is_empty(),
            "the app's own write must produce zero external-change events (FR-006a)"
        );
    }

    #[test]
    fn process_batch_passes_through_external_modify() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        // No recorded self-write for this path → a modify passes through.
        let batch = vec![ev(EventKind::Modify(ModifyKind::Any), &["/r/x.md"])];
        let out = process_batch(&batch, &mut reg, t0, |_p| Some(FileIdentity::new(1, 1)));
        assert_eq!(
            out,
            vec![ChangeEvent::Modified {
                path: "/r/x.md".to_owned()
            }]
        );
    }

    #[test]
    fn process_batch_is_bounded_under_a_large_burst() {
        // FR-006b: a bulk operation (here 10k creates) yields a bounded set —
        // at most one ChangeEvent per (already-coalesced) input event.
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        let paths: Vec<String> = (0..10_000).map(|i| format!("/r/f{i}.md")).collect();
        let batch: Vec<DebouncedEvent> = paths
            .iter()
            .map(|p| ev(EventKind::Create(CreateKind::File), &[p.as_str()]))
            .collect();

        let out = process_batch(&batch, &mut reg, t0, |_p| None);
        assert_eq!(
            out.len(),
            10_000,
            "output is bounded by the input event count (no amplification)"
        );
        assert!(out.iter().all(|c| matches!(c, ChangeEvent::Created { .. })));
    }

    #[test]
    fn process_batch_correlates_rename_as_single_event() {
        let mut reg = SuppressionRegistry::new();
        let t0 = Instant::now();
        let batch = vec![ev(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/r/old.md", "/r/new.md"],
        )];
        let out = process_batch(&batch, &mut reg, t0, |_p| None);
        assert_eq!(
            out,
            vec![ChangeEvent::Renamed {
                from: "/r/old.md".to_owned(),
                to: "/r/new.md".to_owned(),
            }],
            "a move is exactly one Renamed event end-to-end"
        );
    }

    // -- FileIdentity::of_path (small real-fs unit, deterministic) ------------

    #[test]
    fn file_identity_reflects_length() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("n.md");
        std::fs::write(&p, b"hello").unwrap();
        let id = FileIdentity::of_path(&p).unwrap();
        assert_eq!(id.len, 5);
    }

    #[test]
    fn file_identity_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("missing.md");
        assert!(matches!(
            FileIdentity::of_path(&p),
            Err(EmendError::NotFound { .. })
        ));
    }
}
