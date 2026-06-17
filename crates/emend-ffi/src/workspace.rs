//! T059 — FFI projection of the workspace, file operations, and search index
//! (US2 · FFI contract §1/§2/§5; FR-001..008, FR-004a, FR-005, FR-007, FR-008,
//! FR-017, FR-017a, FR-019a).
//!
//! Thin UniFFI shim over [`emend_core::workspace::Workspace`] (locations,
//! preferences, collision-safe file ops) and [`emend_core::index::Index`] (the
//! derived fuzzy/name search index). As with [`crate::document`] this module
//! holds **no logic** of its own; it only:
//!
//! 1. **Projects value types** the core cannot derive `uniffi` on (Constitution
//!    V keeps `emend-core` `uniffi`-free): [`Location`], [`NodeKind`], [`FsNode`]
//!    and the search [`SearchHit`] (reused from [`crate::handles`]). Each `From`
//!    is **exhaustive — no wildcard arm**.
//!
//! 2. **Wraps the workspace + its index** in one [`WorkspaceHandle`], a
//!    `#[derive(uniffi::Object)]` handed to Swift as `Arc<Self>`, with the core
//!    [`Workspace`](emend_core::workspace::Workspace) and
//!    [`Index`](emend_core::index::Index) co-located behind a single
//!    `Mutex<Inner>`.
//!
//! ## Why the index lives **inside** the `WorkspaceHandle` (design decision)
//!
//! The contract (§5) sketches `quick_open_query` / `resolve_wikilink` as free
//! functions, but the task brief asks us to decide where the index lives. The
//! core `Index` is **derived state that must stay in lock-step with the
//! workspace's file operations** (FR-017a: a single create/rename/move/delete
//! updates the index incrementally, never a full rescan). Co-locating it in the
//! `WorkspaceHandle` lets every file-op method maintain the index in **one
//! place, under one lock** — `create_note` inserts, `rename`/`move_node` call
//! `Index::rename`, `delete` removes — so the haystack can never drift from
//! disk. A separate index handle would force callers to thread two objects
//! together and re-implement that synchronization at the boundary. The streaming,
//! supersedable Quick Open *driver* (T073, `SearchHandle` + `SearchSink`) layers
//! on top later; this slice exposes the synchronous `query`/`resolve_name`
//! primitives the contract's `wikilink_suggestions`/`resolve_wikilink` rest on.
//!
//! Location identity ([`LocationId`]) is projected as a plain `u64` across the
//! boundary (the core's `LocationId(u64)` newtype isn't a UniFFI type); the
//! handle converts at the one call site.

use crate::error::FfiError;
use crate::handles::{SearchHit, SearchSink};
use crate::search::{start_query, SearchHandle, SharedIndex};
use emend_core::index::Index;
use emend_core::workspace::{
    FsNode as CoreFsNode, Location as CoreLocation, LocationId, NodeKind as CoreNodeKind, Workspace,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A user-added root folder (FFI contract §1). The FFI mirror of
/// [`emend_core::workspace::Location`].
///
/// `id` is the core's `LocationId(u64)` flattened to a bare `u64` for the
/// boundary; pass it back to [`WorkspaceHandle::remove_location`] /
/// [`WorkspaceHandle::reorder_locations`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Location {
    /// Stable id assigned by the workspace (survives relaunch).
    pub id: u64,
    /// User-visible name; defaults to the folder's basename, user-editable.
    pub display_name: String,
    /// Absolute path to the root folder on disk.
    pub path: String,
    /// Sidebar ordering (ascending).
    pub order: u32,
}

impl From<CoreLocation> for Location {
    fn from(loc: CoreLocation) -> Self {
        // Destructure exhaustively so a new field on the core type forces a
        // compile error here rather than silently dropping data.
        let CoreLocation {
            id,
            display_name,
            path,
            order,
        } = loc;
        Self {
            id: id.0,
            display_name,
            path,
            order,
        }
    }
}

/// Whether an [`FsNode`] is a regular file or a directory (FFI contract §1). The
/// FFI mirror of [`emend_core::workspace::NodeKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NodeKind {
    /// A regular file.
    File,
    /// A directory.
    Folder,
}

impl From<CoreNodeKind> for NodeKind {
    /// Exhaustive projection — no wildcard arm.
    fn from(kind: CoreNodeKind) -> Self {
        match kind {
            CoreNodeKind::File => Self::File,
            CoreNodeKind::Folder => Self::Folder,
        }
    }
}

/// One entry in a directory listing (FFI contract §1: `{ path, kind, name }`).
/// The FFI mirror of [`emend_core::workspace::FsNode`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FsNode {
    /// Absolute path to the entry.
    pub path: String,
    /// Basename (final path component).
    pub name: String,
    /// File or folder.
    pub kind: NodeKind,
}

impl From<CoreFsNode> for FsNode {
    fn from(node: CoreFsNode) -> Self {
        let CoreFsNode { path, name, kind } = node;
        Self {
            path,
            name,
            kind: kind.into(),
        }
    }
}

/// Project a core [`SearchHit`](emend_core::index::SearchHit) to the FFI
/// [`SearchHit`]. The core hit carries `rel_path`; the contract's `SearchHit`
/// carries a `breadcrumb`, which the index derives from the relative path — so
/// `rel_path` *is* the breadcrumb source (the core's own doc comment says so).
fn search_hit(core: emend_core::index::SearchHit) -> SearchHit {
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

/// The workspace + its derived index, co-located so file ops keep the index in
/// lock-step (FR-017a). Behind the [`WorkspaceHandle`]'s `Mutex<Inner>`.
///
/// The index is itself behind an [`Arc<Mutex<Index>>`](SharedIndex) so a spawned
/// Quick Open worker can lock *only the index* for its synchronous rank+stream
/// without holding `Inner` (which would block file ops for the search duration —
/// see `crate::search`'s "Concurrency" note). File-op methods take `Inner` then
/// lock the index to maintain it incrementally (FR-017a).
struct Inner {
    workspace: Workspace,
    index: SharedIndex,
    /// The in-flight Quick Open query, if any. A new `quick_open_query` cancels
    /// this (supersede, NFR-002) before installing its own handle.
    current_search: Option<Arc<SearchHandle>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            index: Arc::new(Mutex::new(Index::new())),
            current_search: None,
        }
    }

    /// Lock the shared index, mapping poisoning to [`FfiError::Internal`] (parity
    /// with the handle's `lock`). Used by file-op methods to maintain the index.
    fn lock_index(&self) -> Result<std::sync::MutexGuard<'_, Index>, FfiError> {
        self.index.lock().map_err(|_| FfiError::Internal {
            detail: "search index lock poisoned".to_owned(),
        })
    }

    /// The **canonical** index key for `path` — the same `canonical_id` identity
    /// `reindex_all`'s `collect_files` seeds with (NFR-007), so incremental
    /// inserts/renames/removes hit the seeded keys instead of leaving ghost
    /// entries. The caller must canonicalize a path *while it still exists*
    /// (before a delete, after a create); on a canonicalization race (the path
    /// vanished) this degrades to the given `path` rather than failing the op —
    /// the index is best-effort derived state, not a ledger.
    fn index_key(&self, path: &str) -> String {
        self.workspace
            .canonical_id(path)
            .map_or_else(|_| path.to_owned(), |p| p.to_string_lossy().into_owned())
    }

    /// Best-effort location-relative path for `abs_path`, used as the index's
    /// second fuzzy haystack and breadcrumb source. Picks the longest matching
    /// location-root prefix; falls back to the basename when `abs_path` is not
    /// under any known location (the index still ranks it by name).
    fn rel_for(&self, abs_path: &str) -> String {
        let best_root = self
            .workspace
            .list_locations()
            .into_iter()
            .filter(|loc| abs_path.starts_with(&loc.path))
            .map(|loc| loc.path)
            .max_by_key(String::len);

        match best_root {
            // `starts_with` guarantees `root.len()` is a valid byte split point.
            Some(root) => abs_path[root.len()..].trim_start_matches('/').to_owned(),
            None => Path::new(abs_path)
                .file_name()
                .map_or_else(|| abs_path.to_owned(), |n| n.to_string_lossy().into_owned()),
        }
    }
}

/// Workspace handle exported to Swift (FFI contract §1/§2/§5).
///
/// Handed to Swift as `Arc<Self>`; methods take `&self` and reach the inner
/// `&mut` through the [`Mutex`]. One handle owns the whole app-managed workspace
/// state (locations, favorites, pins, folder icons, child order) **and** the
/// derived search index, so file operations update both atomically under one
/// lock.
#[derive(uniffi::Object)]
pub struct WorkspaceHandle {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for WorkspaceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceHandle").finish_non_exhaustive()
    }
}

impl Default for WorkspaceHandle {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
        }
    }
}

impl WorkspaceHandle {
    /// Lock the inner state, mapping mutex poisoning (a prior panic while the
    /// lock was held — unreachable given the no-panic posture, but handled rather
    /// than `unwrap`ped per NFR-003) to [`FfiError::Internal`].
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, FfiError> {
        self.inner.lock().map_err(|_| FfiError::Internal {
            detail: "workspace handle lock poisoned".to_owned(),
        })
    }
}

#[uniffi::export]
impl WorkspaceHandle {
    // -- Locations (FFI contract §1) ------------------------------------------

    /// Add `folder_path` as a workspace location and return the created
    /// [`Location`] (FFI contract §1 `add_location`).
    ///
    /// The `bookmark` bytes are the Swift-resolved security-scoped bookmark
    /// (research §A4): Swift opens the scope and hands Rust a usable path, so the
    /// core only needs the resolved path. The bookmark is accepted here for
    /// contract fidelity (Swift passes what it persisted) but is **not** stored
    /// by the core — the OS scope is process-wide once opened; persistence of the
    /// bookmark is the Swift side's concern.
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] if the path does not exist;
    /// [`FfiError::InvalidConfig`] if it exists but is not a directory.
    pub fn add_location(
        &self,
        folder_path: String,
        bookmark: Vec<u8>,
    ) -> Result<Location, FfiError> {
        // `bookmark` is intentionally unused by the core (see the doc comment):
        // bind it to silence the unused warning while keeping the contract shape.
        let _ = bookmark;
        let mut guard = self.lock()?;
        let loc = guard.workspace.add_location(&folder_path)?;
        Ok(loc.into())
    }

    /// Remove a location by id (FFI contract §1 `remove_location`). Does not touch
    /// files on disk — only drops the app-managed entry.
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] if `id` is not a current location.
    pub fn remove_location(&self, id: u64) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        guard.workspace.remove_location(LocationId(id))?;
        Ok(())
    }

    /// List locations in sidebar order (FFI contract §1 `list_locations`).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn list_locations(&self) -> Result<Vec<Location>, FfiError> {
        let guard = self.lock()?;
        Ok(guard
            .workspace
            .list_locations()
            .into_iter()
            .map(Location::from)
            .collect())
    }

    /// Reorder locations to match `order` (FFI contract §1 `reorder_locations`).
    /// Ids omitted from `order` keep their relative order after the listed ones;
    /// unknown ids are ignored (a stale UI list must not error).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn reorder_locations(&self, order: Vec<u64>) -> Result<(), FfiError> {
        let ids: Vec<LocationId> = order.into_iter().map(LocationId).collect();
        let mut guard = self.lock()?;
        guard.workspace.reorder_locations(&ids);
        Ok(())
    }

    // -- Directory listing (FFI contract §1) ----------------------------------

    /// List the immediate children of `folder_path` (FFI contract §1
    /// `list_children`; lazy, non-recursive). Folders first, then by
    /// case-insensitive name.
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] / [`FfiError::PermissionDenied`] /
    /// [`FfiError::IoFailure`] if the directory cannot be read.
    pub fn list_children(&self, folder_path: String) -> Result<Vec<FsNode>, FfiError> {
        let guard = self.lock()?;
        let nodes = guard.workspace.list_children(&folder_path)?;
        Ok(nodes.into_iter().map(FsNode::from).collect())
    }

    // -- Favorites / pins / icons / child order (FFI contract §1) --------------

    /// Assign (`Some`) or clear (`None`) a custom folder icon (FFI contract §1
    /// `set_folder_icon`; FR-008). The icon is an opaque SF-Symbol id.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn set_folder_icon(
        &self,
        folder_path: String,
        icon: Option<String>,
    ) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        guard
            .workspace
            .set_folder_icon(&folder_path, icon.as_deref());
        Ok(())
    }

    /// Mark or unmark `path` as a Favorite (FFI contract §1 `set_favorite`;
    /// FR-007).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn set_favorite(&self, path: String, favorite: bool) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        guard.workspace.set_favorite(&path, favorite);
        Ok(())
    }

    /// Pin or unpin `path` for quick access (FFI contract §1 `set_pinned`;
    /// FR-007).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn set_pinned(&self, path: String, pinned: bool) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        guard.workspace.set_pinned(&path, pinned);
        Ok(())
    }

    /// Set the manual drag-drop child order for `folder_path` (FFI contract §1
    /// `set_child_order`; FR-005). An empty `order` clears the override.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn set_child_order(&self, folder_path: String, order: Vec<String>) -> Result<(), FfiError> {
        let mut guard = self.lock()?;
        guard.workspace.set_child_order(&folder_path, order);
        Ok(())
    }

    /// List the favorited entries as [`FsNode`]s (FFI contract §1
    /// `list_favorites`).
    ///
    /// The core stores favorites as a set of paths (it does not keep an ordered
    /// favorites list), so this resolves each favorited path's current kind via
    /// `symlink_metadata` and skips any that no longer exist (a deleted favorite
    /// must not surface as a broken row). Entries are sorted folders-first then
    /// by case-insensitive name, matching `list_children`.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn list_favorites(&self) -> Result<Vec<FsNode>, FfiError> {
        let guard = self.lock()?;
        let mut nodes: Vec<FsNode> = guard
            .workspace
            .favorites()
            .filter_map(node_for_path)
            .collect();
        nodes.sort_by(|a, b| {
            let folder_first = (a.kind == NodeKind::File).cmp(&(b.kind == NodeKind::File));
            folder_first.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(nodes)
    }

    // -- Collision-safe file operations (FFI contract §2) ---------------------

    /// Create a new empty note under `parent`, returning its path (FFI contract
    /// §2 `create_note`). Collision-safe (auto-suffixed, FR-004a) and atomic
    /// (FR-009a). Inserts the new note into the search index incrementally
    /// (FR-017a).
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] / [`FfiError::PermissionDenied`] /
    /// [`FfiError::IoFailure`] if `parent` is missing or the write fails.
    pub fn create_note(&self, parent: String, name: String) -> Result<String, FfiError> {
        let guard = self.lock()?;
        let new_path = guard.workspace.create_note(&parent, &name)?;
        // The note now exists, so canonicalize it for the index key (matching the
        // canonical seeding identity). `new_path` is still returned to the caller
        // verbatim — only the index key is canonicalized.
        let key = guard.index_key(&new_path);
        let rel = guard.rel_for(&key);
        guard.lock_index()?.insert(&key, &rel);
        Ok(new_path)
    }

    /// Create a new folder under `parent`, returning its path (FFI contract §2
    /// `create_folder`). Collision-safe (FR-004a). Folders are not indexed (the
    /// index tracks notes), so the index is untouched.
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] / [`FfiError::PermissionDenied`] /
    /// [`FfiError::IoFailure`] if `parent` is missing or the directory cannot be
    /// created.
    pub fn create_folder(&self, parent: String, name: String) -> Result<String, FfiError> {
        let guard = self.lock()?;
        let new_path = guard.workspace.create_folder(&parent, &name)?;
        Ok(new_path)
    }

    /// Rename `path` to `new_name` within its parent, returning the new path (FFI
    /// contract §2 `rename`). Collision-safe (FR-004a). Re-keys the index entry
    /// in place if `path` was indexed (FR-017a).
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] if `path` does not exist;
    /// [`FfiError::PermissionDenied`] / [`FfiError::IoFailure`] on failure.
    pub fn rename(&self, path: String, new_name: String) -> Result<String, FfiError> {
        let guard = self.lock()?;
        // Canonicalize the OLD key while the source still exists, before the fs
        // rename moves it (canonicalize fails on a vanished path).
        let old_key = guard.index_key(&path);
        let new_path = guard.workspace.rename(&path, &new_name)?;
        if new_path != path {
            // The destination now exists, so canonicalize it for the NEW key.
            let new_key = guard.index_key(&new_path);
            let rel = guard.rel_for(&new_key);
            guard.lock_index()?.rename(&old_key, &new_key, &rel);
        }
        Ok(new_path)
    }

    /// Move `path` into `new_parent`, returning the new path (FFI contract §2
    /// `move_node`). Collision-safe (FR-004a / FR-005). Re-keys the index entry
    /// in place (FR-017a).
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] if `path` or `new_parent` is missing;
    /// [`FfiError::PermissionDenied`] / [`FfiError::IoFailure`] on failure.
    pub fn move_node(&self, path: String, new_parent: String) -> Result<String, FfiError> {
        let guard = self.lock()?;
        // Canonicalize the OLD key while the source still exists, before the fs
        // move relocates it (canonicalize fails on a vanished path).
        let old_key = guard.index_key(&path);
        let new_path = guard.workspace.move_node(&path, &new_parent)?;
        // The moved node now exists at the destination, so canonicalize it for the
        // NEW key. `new_path` is still returned to the caller verbatim.
        let new_key = guard.index_key(&new_path);
        let rel = guard.rel_for(&new_key);
        guard.lock_index()?.rename(&old_key, &new_key, &rel);
        Ok(new_path)
    }

    /// Delete `path` (FFI contract §2 `delete`). A file is removed; a folder is
    /// removed recursively, and the index entry for the path is dropped
    /// (FR-017a).
    ///
    /// It does **not** touch app-managed preferences (favorite/pin/icon/
    /// child-order) keyed on the path — those are owned and pruned Swift-side.
    ///
    /// # Errors
    ///
    /// [`FfiError::NotFound`] if `path` does not exist;
    /// [`FfiError::PermissionDenied`] / [`FfiError::IoFailure`] on failure.
    pub fn delete(&self, path: String) -> Result<(), FfiError> {
        let guard = self.lock()?;
        // Canonicalize the key BEFORE removing the file — canonicalize requires the
        // target to exist, so it must run while the path is still on disk. This is
        // the same canonical identity `reindex_all` seeded with, so the entry is
        // actually found and dropped (no stale/ghost Quick Open hit).
        let key = guard.index_key(&path);
        guard.workspace.delete(&path)?;
        guard.lock_index()?.remove(&key);
        Ok(())
    }

    // -- Search index (FFI contract §5) ---------------------------------------

    /// Seed/rebuild the search index from `(abs_path, rel_path)` pairs (the only
    /// full-rebuild path; everyday changes go through the file-op methods, which
    /// update the index incrementally per FR-017a).
    ///
    /// Used at startup after the workspace's locations are added, typically from
    /// [`emend_core::workspace::Workspace::collect_files`]. Pairs are
    /// `(absolute, location-relative)` paths.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn rebuild_index(&self, items: Vec<IndexEntry>) -> Result<(), FfiError> {
        let guard = self.lock()?;
        guard
            .lock_index()?
            .rebuild(items.into_iter().map(|e| (e.abs_path, e.rel_path)));
        Ok(())
    }

    /// Seed/rebuild the Quick Open search index from every file currently on disk
    /// under the workspace's locations (FR-017a, FR-017), returning the number of
    /// entries indexed.
    ///
    /// This is the startup seeding path the manual [`WorkspaceHandle::rebuild_index`]
    /// only described: it walks each location with the core
    /// [`Workspace::collect_files`](emend_core::workspace::Workspace::collect_files)
    /// (canonical, cycle-safe, depth-bounded), maps every absolute file path to its
    /// location-relative path via the same `rel_for` the file-op methods use, and
    /// `rebuild`s the index in one shot — so a freshly launched workspace has a
    /// populated haystack rather than the empty index that left Quick Open with no
    /// results. Everyday changes still flow through the incremental file-op methods
    /// (`create_note`/`rename`/`move_node`/`delete`), never this full rebuild.
    ///
    /// `max_depth` bounds the directory recursion (`root` itself is depth 0), passed
    /// straight to `collect_files`.
    ///
    /// A location whose walk fails (unreadable/vanished root) is **skipped** rather
    /// than aborting the whole reindex, so one bad root cannot leave Quick Open
    /// empty for every other location (best-effort seeding, mirroring the index's
    /// own "derived state, not a ledger" posture).
    ///
    /// Scope: `collect_files` returns **files only**, so the index seeds files only;
    /// folder-in-results (FR-017) is deferred — Quick Open opens results into editor
    /// tabs (only files are openable) and the testable criteria (SC-004) are
    /// file-centric.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn reindex_all(&self, max_depth: u32) -> Result<u32, FfiError> {
        let guard = self.lock()?;

        // Collect every (abs_path, rel_path) pair into owned data BEFORE locking
        // the index: `collect_files`/`rel_for` borrow `guard` immutably and
        // `lock_index` borrows it too, so finishing all walk + rel work first
        // avoids overlapping borrows.
        let mut pairs: Vec<(String, String)> = Vec::new();
        for loc in guard.workspace.list_locations() {
            // Best-effort: a single unreadable/missing root is skipped so it can't
            // break Quick Open for the rest of the workspace.
            let Ok(files) = guard.workspace.collect_files(&loc.path, max_depth) else {
                continue;
            };
            for abs in files {
                let rel = guard.rel_for(&abs);
                pairs.push((abs, rel));
            }
        }

        let count = u32::try_from(pairs.len()).unwrap_or(u32::MAX);
        guard.lock_index()?.rebuild(pairs);
        Ok(count)
    }

    /// Return up to `limit` ranked [`SearchHit`]s for `query`, best first (FFI
    /// contract §5; FR-017). Synchronous, in-memory; the streaming supersedable
    /// Quick Open driver (T073) layers on top later.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn query(&self, query: String, limit: u32) -> Result<Vec<SearchHit>, FfiError> {
        let guard = self.lock()?;
        let hits = guard.lock_index()?.query(&query, limit as usize);
        Ok(hits.into_iter().map(search_hit).collect())
    }

    /// Resolve a wiki-link target by note name (FFI contract §5; FR-019a). Returns
    /// the absolute paths of every note whose basename stem normalizes to `name`
    /// (case-insensitive, extension ignored) — the full candidate set, O(1) in the
    /// name map. Deterministic disambiguation among duplicates is the caller's
    /// policy.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn resolve_name(&self, name: String) -> Result<Vec<String>, FfiError> {
        let guard = self.lock()?;
        let resolved = guard.lock_index()?.resolve_name(&name);
        Ok(resolved)
    }

    /// Resolve a `[[wiki link]]` to a single absolute note path using the
    /// **deterministic** FR-019a policy (FFI contract §5 `resolve_wikilink`).
    ///
    /// `from_note` is the path of the note containing the link (used for the
    /// same-directory tie-break); `raw_target` is the link target as typed.
    /// Delegates to [`emend_core::derived::resolve_wikilink`], which ranks the
    /// index's candidate set by: same directory as the source → shallowest path →
    /// lexicographically smallest path. Returns `None` when the target resolves to
    /// no note — including the v1 case where the target was renamed (links are not
    /// auto-rewritten, so a stale name is unresolved, never mis-pointed).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn resolve_wikilink(
        &self,
        from_note: String,
        raw_target: String,
    ) -> Result<Option<String>, FfiError> {
        let guard = self.lock()?;
        let index = guard.lock_index()?;
        Ok(emend_core::derived::resolve_wikilink(
            &index,
            &from_note,
            &raw_target,
        ))
    }

    /// Autocomplete suggestions for a `[[` prefix (FFI contract §5
    /// `wikilink_suggestions`; FR-020): up to `limit` ranked [`SearchHit`]s over
    /// the index's note names/paths (same ranking as Quick Open).
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] if the lock is poisoned.
    pub fn wikilink_suggestions(
        &self,
        prefix: String,
        limit: u32,
    ) -> Result<Vec<SearchHit>, FfiError> {
        let guard = self.lock()?;
        let index = guard.lock_index()?;
        let hits = emend_core::derived::wikilink_suggestions(&index, &prefix, limit as usize);
        Ok(hits.into_iter().map(search_hit).collect())
    }

    /// Start a streaming, **supersedable** Quick Open query (FFI contract §5
    /// `quick_open_query`; FR-017, FR-018/SC-004, NFR-002).
    ///
    /// Ranked [`SearchHit`]s are streamed to `sink` in batches via
    /// [`SearchSink::on_results`], terminated by exactly one
    /// [`SearchSink::on_done`] when the query completes. The returned
    /// [`SearchHandle`] lets Swift `cancel()` the query explicitly.
    ///
    /// **Supersede (NFR-002):** each call first cancels the previous in-flight
    /// query (if any) — its worker stops emitting and fires no terminal — then
    /// installs and spawns this one. So fast keystrokes never interleave stale
    /// results from an older query. The spawned worker locks only the shared
    /// index for its synchronous rank+stream, not this `Inner`, so it does not
    /// block concurrent file ops (see `crate::search`'s "Concurrency" note).
    ///
    /// Infallible at the boundary (returns the handle directly, not a `Result`):
    /// the contract gives Quick Open no error terminal (§5), and a runtime/lock
    /// failure degrades to an empty, completed stream rather than a thrown error.
    /// A poisoned `Inner` lock is the one exception handled internally — it
    /// returns a fresh, already-cancellable handle and skips spawning.
    #[must_use]
    pub fn quick_open_query(&self, query: String, sink: Arc<dyn SearchSink>) -> Arc<SearchHandle> {
        // Supersede the prior query and install the new handle under the lock so
        // two rapid keystrokes can't both think they're current. If the lock is
        // poisoned (unreachable under the no-panic posture) we still hand back a
        // valid handle and an empty completed stream rather than panicking.
        let Ok(mut guard) = self.inner.lock() else {
            let handle = start_query(Arc::new(Mutex::new(Index::new())), query, sink);
            return handle;
        };
        if let Some(prev) = guard.current_search.take() {
            prev.cancel();
        }
        let index = Arc::clone(&guard.index);
        let handle = start_query(index, query, sink);
        guard.current_search = Some(Arc::clone(&handle));
        handle
    }
}

/// One `(absolute, location-relative)` path pair for
/// [`WorkspaceHandle::rebuild_index`] (FFI contract §5 seeding).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct IndexEntry {
    /// Absolute path on disk.
    pub abs_path: String,
    /// Path relative to the location root (the fuzzy second haystack / breadcrumb).
    pub rel_path: String,
}

/// Build an [`FsNode`] for an existing `path` by stat'ing its current kind, or
/// `None` if it no longer exists. Used by [`WorkspaceHandle::list_favorites`] so
/// a deleted favorite is skipped rather than surfaced as a broken row.
fn node_for_path(path: &str) -> Option<FsNode> {
    let p = Path::new(path);
    let meta = std::fs::symlink_metadata(p).ok()?;
    let kind = if meta.is_dir() {
        NodeKind::Folder
    } else {
        NodeKind::File
    };
    let name = p
        .file_name()
        .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned());
    Some(FsNode {
        path: path.to_owned(),
        name,
        kind,
    })
}

/// Construct a fresh, empty [`WorkspaceHandle`] (FFI contract §1 entry point).
///
/// One per app session; Swift adds locations, lists children, performs file ops,
/// and queries the index through the returned handle.
#[uniffi::export]
#[must_use]
pub fn new_workspace() -> Arc<WorkspaceHandle> {
    Arc::new(WorkspaceHandle::default())
}

/// Store a dropped media attachment and return the **note-relative** Markdown
/// reference to insert (FFI contract §2 `store_attachment`; FR-013/FR-013a).
///
/// A free function (not a [`WorkspaceHandle`] method): the core
/// [`emend_core::fs::store_attachment`] keys entirely off the note's path on
/// disk, needing none of the workspace's in-memory state. `note_path` is the
/// target note, or `None` when it is still untitled/unsaved (a defined fallback
/// dir is used; FR-013a). The attachment is written atomically + durably (an
/// observer never sees a partial file, FR-009a) into an `attachments/`
/// subdirectory beside the note, with a collision-safe name (`img.png` →
/// `img 2.png`).
///
/// # Errors
///
/// [`FfiError::PermissionDenied`] / [`FfiError::IoFailure`] if the attachments
/// directory cannot be created or the write fails.
#[uniffi::export]
pub fn store_attachment(
    note_path: Option<String>,
    bytes: Vec<u8>,
    suggested_name: String,
) -> Result<String, FfiError> {
    emend_core::fs::store_attachment(note_path.as_deref(), &bytes, &suggested_name)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{new_workspace, store_attachment, NodeKind};
    use crate::error::FfiError;

    #[test]
    fn add_list_remove_locations_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = new_workspace();

        let loc = ws
            .add_location(dir.path().to_string_lossy().into_owned(), Vec::new())
            .expect("add location");
        assert_eq!(loc.order, 0);

        let listed = ws.list_locations().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, loc.id);

        ws.remove_location(loc.id).expect("remove");
        assert!(ws.list_locations().expect("list").is_empty());

        // Removing a stale id surfaces NotFound.
        let err = ws.remove_location(loc.id).expect_err("stale remove");
        assert!(matches!(err, FfiError::NotFound { .. }));
    }

    #[test]
    fn add_location_rejects_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.md");
        std::fs::write(&file, b"x").expect("write");
        let ws = new_workspace();
        let err = ws
            .add_location(file.to_string_lossy().into_owned(), Vec::new())
            .expect_err("a file is not a location");
        assert!(matches!(err, FfiError::InvalidConfig { .. }));
    }

    #[test]
    fn create_note_indexes_it_and_query_finds_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = new_workspace();
        let root = dir.path().to_string_lossy().into_owned();
        ws.add_location(root.clone(), Vec::new()).expect("add");

        let note = ws
            .create_note(root.clone(), "Meeting Notes".to_owned())
            .expect("create note");
        // External behavior unchanged: the returned path keeps its requested
        // basename and is whatever `create_note` returns (not canonicalized).
        assert!(note.ends_with("Meeting Notes.md"));

        // The new note is in the index (incrementally inserted by create_note),
        // keyed under its CANONICAL identity (the same form `reindex_all` seeds
        // with, NFR-007) — so on a non-canonical root (macOS `/var` ->
        // `/private/var`) the hit path is the canonical path, not the verbatim
        // return. Compare against the canonical form rather than the raw return.
        let canonical_note = std::fs::canonicalize(&note)
            .expect("canonicalize note")
            .to_string_lossy()
            .into_owned();
        let hits = ws.query("meeting".to_owned(), 10).expect("query");
        assert!(
            hits.iter().any(|h| h.path == canonical_note),
            "create_note must index the note under its canonical key so query \
             finds it: {hits:?}"
        );
    }

    #[test]
    fn rename_rekeys_the_index_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = new_workspace();
        let root = dir.path().to_string_lossy().into_owned();
        ws.add_location(root.clone(), Vec::new()).expect("add");

        let note = ws
            .create_note(root.clone(), "draft".to_owned())
            .expect("create");
        let renamed = ws.rename(note.clone(), "final".to_owned()).expect("rename");

        // The old name no longer resolves; the new one does.
        assert!(ws
            .resolve_name("draft".to_owned())
            .expect("resolve")
            .is_empty());
        // The index re-keys under the CANONICAL destination identity (NFR-007), so
        // `resolve_name` returns the canonical path, not the verbatim `rename`
        // return. Compare against the canonical form (idempotent where the root was
        // already canonical; on macOS `/var` -> `/private/var`).
        let canonical_renamed = std::fs::canonicalize(&renamed)
            .expect("canonicalize renamed")
            .to_string_lossy()
            .into_owned();
        let resolved = ws.resolve_name("final".to_owned()).expect("resolve");
        assert_eq!(resolved, vec![canonical_renamed]);
    }

    #[test]
    fn delete_removes_the_index_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = new_workspace();
        let root = dir.path().to_string_lossy().into_owned();
        ws.add_location(root.clone(), Vec::new()).expect("add");

        let note = ws.create_note(root, "scratch".to_owned()).expect("create");
        ws.delete(note).expect("delete");
        assert!(ws
            .resolve_name("scratch".to_owned())
            .expect("resolve")
            .is_empty());
    }

    #[test]
    fn list_children_reports_kinds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = new_workspace();
        let root = dir.path().to_string_lossy().into_owned();
        ws.add_location(root.clone(), Vec::new()).expect("add");

        ws.create_folder(root.clone(), "Projects".to_owned())
            .expect("folder");
        ws.create_note(root.clone(), "note".to_owned())
            .expect("note");

        let children = ws.list_children(root).expect("list children");
        // Folder sorts before file.
        assert_eq!(children[0].kind, NodeKind::Folder);
        assert_eq!(children[0].name, "Projects");
        assert!(children.iter().any(|c| c.kind == NodeKind::File));
    }

    #[test]
    fn favorites_round_trip_and_list_skips_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = new_workspace();
        let root = dir.path().to_string_lossy().into_owned();
        ws.add_location(root.clone(), Vec::new()).expect("add");
        let note = ws.create_note(root, "fav".to_owned()).expect("create");

        ws.set_favorite(note.clone(), true).expect("favorite");
        let favs = ws.list_favorites().expect("list favorites");
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].path, note);

        // A favorited-then-deleted path is skipped, not surfaced as a broken row.
        ws.delete(note).expect("delete");
        assert!(ws.list_favorites().expect("list").is_empty());
    }

    #[test]
    fn resolve_wikilink_and_suggestions_over_the_index() {
        let ws = new_workspace();
        ws.rebuild_index(vec![
            super::IndexEntry {
                abs_path: "/root/launch-plan.md".to_owned(),
                rel_path: "launch-plan.md".to_owned(),
            },
            super::IndexEntry {
                abs_path: "/root/launch-post.md".to_owned(),
                rel_path: "launch-post.md".to_owned(),
            },
        ])
        .expect("rebuild");

        // Exact name resolves to the one note.
        let resolved = ws
            .resolve_wikilink("/root/from.md".to_owned(), "launch-plan".to_owned())
            .expect("resolve");
        assert_eq!(resolved.as_deref(), Some("/root/launch-plan.md"));

        // A missing target is unresolved (None), never mis-pointed.
        assert_eq!(
            ws.resolve_wikilink("/root/from.md".to_owned(), "missing".to_owned())
                .expect("resolve"),
            None
        );

        // Autocomplete suggests both launch-* notes for the `laun` prefix.
        let hits = ws
            .wikilink_suggestions("laun".to_owned(), 10)
            .expect("suggestions");
        assert!(hits.len() >= 2, "both launch-* notes: {hits:?}");
        assert!(hits.iter().all(|h| h.name.contains("launch")));
    }

    #[test]
    fn store_attachment_writes_and_returns_rel_ref() {
        let dir = tempfile::tempdir().expect("tempdir");
        let note = dir.path().join("note.md");
        std::fs::write(&note, b"# hi").expect("write note");

        let rel = store_attachment(
            Some(note.to_string_lossy().into_owned()),
            b"\x89PNG bytes".to_vec(),
            "image.png".to_owned(),
        )
        .expect("store");
        assert_eq!(rel, "attachments/image.png");

        let stored = dir.path().join("attachments").join("image.png");
        assert_eq!(std::fs::read(stored).expect("read"), b"\x89PNG bytes");

        // A second drop of the same name is collision-safe.
        let rel2 = store_attachment(
            Some(note.to_string_lossy().into_owned()),
            b"second".to_vec(),
            "image.png".to_owned(),
        )
        .expect("store 2");
        assert_eq!(rel2, "attachments/image 2.png");
    }

    #[test]
    fn rebuild_index_seeds_and_query_ranks() {
        let ws = new_workspace();
        ws.rebuild_index(vec![
            super::IndexEntry {
                abs_path: "/root/alpha.md".to_owned(),
                rel_path: "alpha.md".to_owned(),
            },
            super::IndexEntry {
                abs_path: "/root/notes/beta.md".to_owned(),
                rel_path: "notes/beta.md".to_owned(),
            },
        ])
        .expect("rebuild");

        let hits = ws.query("beta".to_owned(), 10).expect("query");
        assert_eq!(
            hits.first().map(|h| h.path.as_str()),
            Some("/root/notes/beta.md")
        );
        // The breadcrumb is the relative path.
        assert_eq!(hits[0].breadcrumb, "notes/beta.md");
    }

    #[test]
    fn reindex_all_seeds_from_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `add_location` now canonicalizes the root internally (NFR-007), so `rel_for`
        // prefix-matches `collect_files`' canonical paths regardless of the root's
        // form. We pass an already-canonical root here (canonicalization is then
        // idempotent); `reindex_all_handles_non_canonical_root` covers the
        // non-canonical case the production gap was about.
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let nested = root.join("notes").join("sub");
        std::fs::create_dir_all(&nested).expect("create nested dirs");
        std::fs::write(root.join("alpha.md"), b"# alpha").expect("write alpha");
        std::fs::write(nested.join("beta.md"), b"# beta").expect("write beta");

        let ws = new_workspace();
        ws.add_location(root.to_string_lossy().into_owned(), Vec::new())
            .expect("add location");

        // Reindex from disk: both files are picked up.
        let count = ws.reindex_all(16).expect("reindex");
        assert_eq!(count, 2);

        // A query for a file stem returns the expected hit with a sensible
        // breadcrumb (the location-relative path).
        let hits = ws.query("beta".to_owned(), 10).expect("query");
        let beta_abs = nested.join("beta.md").to_string_lossy().into_owned();
        assert_eq!(
            hits.first().map(|h| h.path.as_str()),
            Some(beta_abs.as_str())
        );
        assert_eq!(hits[0].breadcrumb, "notes/sub/beta.md");
    }

    /// End-to-end regression for the path-identity bug (NFR-007): a location whose
    /// root is **not** canonical must still seed, query, breadcrumb, and delete
    /// without leaving a stale/ghost index entry.
    ///
    /// The root here is a **symlink** to the real temp dir, so it is deliberately
    /// non-canonical (NOT pre-canonicalized — that pre-canonicalization is exactly
    /// the production gap, since the Swift app hands Rust the raw resolved path).
    /// Before the fix:
    ///   * `add_location` stored the symlink root verbatim, so `rel_for`'s
    ///     `starts_with(loc.path)` failed against `collect_files`' canonical paths
    ///     and fell back to the bare basename (wrong breadcrumb); and
    ///   * `delete` keyed the index by the symlink-rooted path while `reindex_all`
    ///     had seeded the canonical key, so the entry was never found and a ghost
    ///     hit lingered.
    ///
    /// After the fix (`add_location` canonicalizes the root, the file ops key the
    /// index canonically) both are correct.
    #[test]
    fn reindex_all_handles_non_canonical_root() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        // The real tree lives under the canonical temp dir.
        let real_root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let nested = real_root.join("notes").join("sub");
        std::fs::create_dir_all(&nested).expect("create nested dirs");
        std::fs::write(nested.join("beta.md"), b"# beta").expect("write beta");

        // A symlink to the real root: a deliberately NON-canonical path to the same
        // physical directory. This is what `add_location` receives.
        let link_root = dir.path().join("link_root");
        symlink(&real_root, &link_root).expect("symlink to root");
        let non_canonical_root = link_root.to_string_lossy().into_owned();
        // Sanity: the root we add is genuinely not its own canonical form.
        assert_ne!(
            std::fs::canonicalize(&link_root).expect("canonicalize link"),
            link_root,
            "the test root must be non-canonical for this regression to bite"
        );

        let ws = new_workspace();
        ws.add_location(non_canonical_root.clone(), Vec::new())
            .expect("add location");

        // Seed the index from disk and confirm the note is found.
        let count = ws.reindex_all(16).expect("reindex");
        assert_eq!(count, 1, "the single note is seeded");

        let hits = ws.query("beta".to_owned(), 10).expect("query");
        let hit = hits.first().expect("beta is a hit");

        // The seeded hit's path is the canonical note path...
        let beta_canonical = nested.join("beta.md").to_string_lossy().into_owned();
        assert_eq!(hit.path, beta_canonical);
        // ...and the breadcrumb is the LOCATION-RELATIVE path, not the bare
        // basename (the basename fallback was the symptom of the prefix mismatch).
        assert_eq!(hit.breadcrumb, "notes/sub/beta.md");
        assert_ne!(
            hit.breadcrumb, "beta.md",
            "breadcrumb must be the relative path, not the basename fallback"
        );

        // Delete the note through the workspace using a path derived from the
        // non-canonical root (as the sidebar would) and confirm NO ghost lingers.
        let note_via_link = link_root.join("notes").join("sub").join("beta.md");
        ws.delete(note_via_link.to_string_lossy().into_owned())
            .expect("delete");

        // The index must no longer return it — neither by fuzzy query nor by name.
        assert!(
            ws.query("beta".to_owned(), 10).expect("query").is_empty(),
            "a deleted note must not leave a ghost Quick Open hit"
        );
        assert!(
            ws.resolve_name("beta".to_owned())
                .expect("resolve")
                .is_empty(),
            "a deleted note must not leave a ghost wiki-link target"
        );
    }
}
