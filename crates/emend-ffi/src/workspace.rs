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
use crate::handles::SearchHit;
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
/// lock-step (FR-017a). Behind the [`WorkspaceHandle`]'s `Mutex`.
struct Inner {
    workspace: Workspace,
    index: Index,
}

impl Inner {
    fn new() -> Self {
        Self {
            workspace: Workspace::new(),
            index: Index::new(),
        }
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
        let mut guard = self.lock()?;
        let new_path = guard.workspace.create_note(&parent, &name)?;
        let rel = guard.rel_for(&new_path);
        guard.index.insert(&new_path, &rel);
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
        let mut guard = self.lock()?;
        let new_path = guard.workspace.rename(&path, &new_name)?;
        if new_path != path {
            let rel = guard.rel_for(&new_path);
            guard.index.rename(&path, &new_path, &rel);
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
        let mut guard = self.lock()?;
        let new_path = guard.workspace.move_node(&path, &new_parent)?;
        let rel = guard.rel_for(&new_path);
        guard.index.rename(&path, &new_path, &rel);
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
        let mut guard = self.lock()?;
        guard.workspace.delete(&path)?;
        guard.index.remove(&path);
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
        let mut guard = self.lock()?;
        guard
            .index
            .rebuild(items.into_iter().map(|e| (e.abs_path, e.rel_path)));
        Ok(())
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
        let hits = guard.index.query(&query, limit as usize);
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
        Ok(guard.index.resolve_name(&name))
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{new_workspace, NodeKind};
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
        assert!(note.ends_with("Meeting Notes.md"));

        // The new note is in the index (incrementally inserted by create_note).
        let hits = ws.query("meeting".to_owned(), 10).expect("query");
        assert!(
            hits.iter().any(|h| h.path == note),
            "create_note must index the note so query finds it: {hits:?}"
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
        let resolved = ws.resolve_name("final".to_owned()).expect("resolve");
        assert_eq!(resolved, vec![renamed]);
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
}
