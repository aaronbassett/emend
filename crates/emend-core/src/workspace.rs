//! The file-based workspace model (US2 · FR-001..008, FR-004a, FR-013a,
//! FR-017a, NFR-007).
//!
//! Emend has **no database**. The workspace is two stores stitched together:
//!
//! 1. **On disk (source of truth for content):** plain folders and `.md` files
//!    in user-chosen [`Location`]s. Any external tool or agent reads/writes the
//!    same files (FR-003). The collision-safe file operations in this module
//!    ([`Workspace::create_note`], [`Workspace::rename`], [`Workspace::move_node`],
//!    …) go through [`crate::fs`] for note bytes and `std::fs` for directory
//!    metadata.
//! 2. **App-managed state (local preferences):** the set of locations plus
//!    per-path favorites, pins, custom folder icons, and manual child order
//!    (the [`Workspace`] struct's in-memory maps). This is *not* derivable from
//!    the files, so the core owns it (data-model "Persistence map").
//!
//! This module deliberately depends on **`std` + `tempfile` only** (via
//! [`crate::fs`]); it holds **no `uniffi` and no `tokio`** types (Constitution V),
//! so the whole thing is unit/integration-testable with plain `cargo test`. The
//! public surface is shaped to project cleanly onto the FFI contract's
//! §1 *Workspace & Locations* and §2 *File operations* later, without importing
//! any FFI machinery here.
//!
//! ## Path identity (NFR-007)
//!
//! Three hazards, one mechanism — **canonicalization**:
//!
//! * **Symlink cycles must terminate.** [`Workspace::collect_files`] is a bounded
//!   walk: it caps recursion at a caller-supplied `max_depth` *and* records the
//!   canonical path of every directory it descends into, so a directory it has
//!   already physically visited (reached again through a symlink) is skipped.
//!   Either guard alone terminates a cycle; both together also bound the work.
//! * **The same physical file via two paths is one identity.** Identity is the
//!   [`Workspace::canonical_id`] — the symlink- and `..`-resolved absolute path
//!   from `std::fs::canonicalize`. Two lexical routes to one inode canonicalize
//!   to the same `PathBuf`, so a `HashSet` of canonical paths dedupes them.
//! * **Case-insensitive vs case-sensitive volumes.** We never compare paths
//!   case-folded ourselves; we let the host filesystem decide. On a
//!   case-insensitive volume `canonicalize` resolves `Note.md` and `note.md` to
//!   the one on-disk file (same identity); on a case-sensitive volume they are
//!   two files (or one is absent), which `canonicalize` reports faithfully.
//!
//! ## Collision-safe naming (FR-004a / FR-013a)
//!
//! Create/rename/move never overwrite an existing entry. When the requested
//! basename is taken, [`free_name`] appends a space and the lowest integer ≥ 2
//! that frees the name, inserting it **before the final extension** for files and
//! at the end for folders: `note.md` → `note 2.md` → `note 3.md`; `folder` →
//! `folder 2`. The scheme is deterministic (next free name is reproducible) and
//! matches the Finder-style "name 2" convention users expect.

use crate::fs::write_atomic;
use crate::EmendError;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A stable identifier for a [`Location`]. Survives relaunch (data-model
/// "Location"). A monotonically increasing `u64` assigned by the [`Workspace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocationId(pub u64);

/// A user-added root folder — the entry point to a tree (data-model "Location").
///
/// `path` is stored as the resolved absolute path the caller supplied (Swift
/// resolves the security-scoped bookmark and hands Rust a usable path, FFI
/// contract §1 / research §A4). Fields are FFI-friendly primitives so this
/// projects onto the contract's `Location` later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Stable id assigned by the workspace.
    pub id: LocationId,
    /// User-visible name; defaults to the folder's basename, user-editable.
    pub display_name: String,
    /// Absolute path to the root folder on disk.
    pub path: String,
    /// Sidebar ordering (ascending).
    pub order: u32,
}

/// Whether an [`FsNode`] is a regular file or a directory (FFI contract §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A regular file.
    File,
    /// A directory.
    Folder,
}

/// One entry in a directory listing (FFI contract §1: `{ path, kind, name }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsNode {
    /// Absolute path to the entry.
    pub path: String,
    /// Basename (final path component).
    pub name: String,
    /// File or folder.
    pub kind: NodeKind,
}

/// The app-managed workspace state: the set of [`Location`]s plus per-path
/// favorites, pins, custom folder icons, and manual child order.
///
/// Holds **no `uniffi`/`tokio`** types (Constitution V). The file-operation
/// methods are `&self` (they touch the filesystem, not this struct's maps);
/// the location/preference mutators are `&mut self`.
#[derive(Debug, Default)]
pub struct Workspace {
    /// Added locations, keyed by id.
    locations: HashMap<LocationId, Location>,
    /// Next id to hand out (monotonic).
    next_location_id: u64,
    /// Paths the user marked as Favorites (FR-007).
    favorites: HashSet<String>,
    /// Paths the user pinned for quick access (FR-007).
    pinned: HashSet<String>,
    /// Per-folder custom icon ids (FR-008); absence = default icon.
    folder_icons: HashMap<String, String>,
    /// Per-folder manual drag-drop child order (FR-005); absence = natural sort.
    child_order: HashMap<String, Vec<String>>,
}

impl Workspace {
    /// Create an empty workspace with no locations or preferences.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // -- Locations (FR-001) ---------------------------------------------------

    /// Add `folder_path` as a workspace location and return the created
    /// [`Location`]. The display name defaults to the folder's basename; `order`
    /// is assigned as the next slot after the existing locations.
    ///
    /// The stored [`Location::path`] is the **canonical** (symlink- and
    /// `..`-resolved) absolute path — the same [`Workspace::canonical_id`] identity
    /// [`Workspace::collect_files`] returns (NFR-007). This is load-bearing: index
    /// seeding walks `collect_files` (canonical), so a verbatim location root would
    /// not prefix-match those canonical file paths, breaking relative-path
    /// breadcrumbs/fuzzy matching (FR-017) and letting the incremental file ops
    /// key the index under a *different* path form than seeding used (a stale/ghost
    /// entry on delete/rename). Canonicalizing the root once here makes canonical
    /// the single path identity for the whole location subtree.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] if the path does not exist, or
    /// [`EmendError::InvalidConfig`] if it exists but is not a directory (a
    /// location must be a readable folder, per data-model "Location" validation).
    pub fn add_location(&mut self, folder_path: &str) -> Result<Location, EmendError> {
        let path = Path::new(folder_path);
        let meta = std::fs::metadata(path).map_err(|e| map_io(path, &e))?;
        if !meta.is_dir() {
            return Err(EmendError::InvalidConfig {
                detail: format!("location is not a directory: {folder_path}"),
            });
        }

        // Store the canonical root as the location's identity (NFR-007 / see the
        // doc comment). `metadata` above already followed the path, so the dir
        // exists and is readable; `canonical_id` keeps the NotFound/permission/IO
        // error posture for the race where it vanishes between the two calls.
        let canonical = self.canonical_id(folder_path)?;
        let canonical_path = canonical.to_string_lossy().into_owned();

        let id = LocationId(self.next_location_id);
        self.next_location_id += 1;
        // Display name defaults to the *requested* path's basename so a symlinked
        // root keeps its user-facing spelling (`Notes`), not the canonical target.
        let display_name = basename(path).unwrap_or_else(|| canonical_path.clone());
        let order = u32::try_from(self.locations.len()).unwrap_or(u32::MAX);

        let location = Location {
            id,
            display_name,
            path: canonical_path,
            order,
        };
        self.locations.insert(id, location.clone());
        Ok(location)
    }

    /// Remove a location by id. Removing an unknown id is a [`EmendError::NotFound`]
    /// so the caller learns the id was stale rather than silently no-op'ing.
    ///
    /// Removing a location does **not** touch the files on disk — it only drops
    /// the app-managed entry (FR-001 "remove locations").
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] if `id` is not a current location.
    pub fn remove_location(&mut self, id: LocationId) -> Result<(), EmendError> {
        if self.locations.remove(&id).is_none() {
            return Err(EmendError::NotFound {
                path: format!("location {}", id.0),
            });
        }
        Ok(())
    }

    /// List locations in sidebar order (ascending `order`, ties broken by id).
    #[must_use]
    pub fn list_locations(&self) -> Vec<Location> {
        let mut out: Vec<Location> = self.locations.values().cloned().collect();
        out.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        out
    }

    /// Reorder locations to match `order` (FFI contract §1 `reorder_locations`).
    /// Any id present in `order` gets its `order` field set to its position;
    /// ids omitted from `order` keep their relative order *after* the listed
    /// ones. Unknown ids in `order` are ignored (a stale UI list must not error).
    pub fn reorder_locations(&mut self, order: &[LocationId]) {
        // Listed ids first, in the given order.
        for (slot, id) in order.iter().enumerate() {
            if let Some(loc) = self.locations.get_mut(id) {
                loc.order = u32::try_from(slot).unwrap_or(u32::MAX);
            }
        }
        // Anything not listed is pushed after, preserving its previous order.
        let listed: HashSet<LocationId> = order.iter().copied().collect();
        let base = u32::try_from(order.len()).unwrap_or(u32::MAX);
        let mut trailing: Vec<LocationId> = self
            .locations
            .keys()
            .copied()
            .filter(|id| !listed.contains(id))
            .collect();
        trailing.sort_by_key(|id| {
            self.locations
                .get(id)
                .map_or((u32::MAX, id.0), |l| (l.order, id.0))
        });
        for (offset, id) in trailing.into_iter().enumerate() {
            if let Some(loc) = self.locations.get_mut(&id) {
                loc.order = base.saturating_add(u32::try_from(offset).unwrap_or(u32::MAX));
            }
        }
    }

    // -- Directory listing (FR-002) -------------------------------------------

    /// List the immediate children of `folder_path` (FFI contract §1; lazy,
    /// non-recursive — the sidebar expands one level at a time).
    ///
    /// Entries are sorted folders-first then by case-insensitive name, which is a
    /// stable natural order for the tree. Symlinks are reported by what they
    /// resolve to (a symlink to a directory reads as [`NodeKind::Folder`]); a
    /// dangling symlink is skipped rather than surfaced as a broken entry.
    ///
    /// On a case-insensitive volume the filesystem yields each physical entry
    /// under a single spelling, so the listing inherently contains one row per
    /// file (NFR-007) — we do not add a second alias.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
    /// [`EmendError::IoFailure`] if the directory cannot be read.
    pub fn list_children(&self, folder_path: &str) -> Result<Vec<FsNode>, EmendError> {
        let dir = Path::new(folder_path);
        let mut nodes = Vec::new();

        for entry in std::fs::read_dir(dir).map_err(|e| map_io(dir, &e))? {
            let entry = entry.map_err(|e| map_io(dir, &e))?;
            let path = entry.path();
            // `metadata()` (not `symlink_metadata`) follows symlinks so a linked
            // directory reads as a folder; a dangling link errors and is skipped.
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let kind = if meta.is_dir() {
                NodeKind::Folder
            } else {
                NodeKind::File
            };
            let name = basename(&path).unwrap_or_default();
            nodes.push(FsNode {
                path: path.to_string_lossy().into_owned(),
                name,
                kind,
            });
        }

        nodes.sort_by(|a, b| {
            let folder_first = (a.kind == NodeKind::File).cmp(&(b.kind == NodeKind::File));
            folder_first.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(nodes)
    }

    // -- Path identity (NFR-007) ----------------------------------------------

    /// The canonical identity of `path`: the symlink- and `..`-resolved absolute
    /// path. Two different lexical routes to the same physical file yield equal
    /// identities; the host filesystem (not us) decides case-sensitivity.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] if the path (or a component) does not exist —
    /// `canonicalize` requires the target to exist. [`EmendError::PermissionDenied`]
    /// / [`EmendError::IoFailure`] for the corresponding resolution failures.
    pub fn canonical_id(&self, path: &str) -> Result<PathBuf, EmendError> {
        let p = Path::new(path);
        std::fs::canonicalize(p).map_err(|e| map_io(p, &e))
    }

    /// Walk `root` to `max_depth` levels deep and return the **canonical** paths
    /// of every regular file found, each exactly once (NFR-007).
    ///
    /// Termination is guaranteed two ways: a hard `max_depth` cap and a visited
    /// set of canonical directory paths (a directory reached again through a
    /// symlink cycle is skipped). File identity is the canonical path, so the
    /// same physical file reached via two routes is collected once.
    ///
    /// `max_depth` counts directory levels below `root` (`root` itself is depth
    /// 0). This is intended for path-identity reasoning and index seeding — the
    /// sidebar uses the lazy [`Workspace::list_children`] instead.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
    /// [`EmendError::IoFailure`] if `root` cannot be canonicalized or read.
    pub fn collect_files(&self, root: &str, max_depth: u32) -> Result<Vec<String>, EmendError> {
        let root = self.canonical_id(root)?;
        let mut visited_dirs: HashSet<PathBuf> = HashSet::new();
        let mut files: HashSet<PathBuf> = HashSet::new();
        self.walk(&root, max_depth, &mut visited_dirs, &mut files)?;

        let mut out: Vec<String> = files
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        out.sort();
        Ok(out)
    }

    /// Recursive worker for [`Workspace::collect_files`]. `dir` is already
    /// canonical. Skips a directory it has physically visited before (cycle
    /// guard) and stops at `remaining_depth == 0` (depth bound).
    fn walk(
        &self,
        dir: &Path,
        remaining_depth: u32,
        visited_dirs: &mut HashSet<PathBuf>,
        files: &mut HashSet<PathBuf>,
    ) -> Result<(), EmendError> {
        // Cycle guard: a directory we already entered (reached again through a
        // symlink) is skipped. `insert` returns false if it was already present.
        if !visited_dirs.insert(dir.to_path_buf()) {
            return Ok(());
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            // A directory that vanished or is unreadable mid-walk is tolerated:
            // path identity / termination is the obligation, not exhaustiveness.
            Err(_) => return Ok(()),
        };

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            // Follow symlinks via `metadata`; resolve to a canonical identity.
            let Ok(meta) = std::fs::metadata(&path) else {
                continue; // dangling symlink or race — skip
            };
            let Ok(canonical) = std::fs::canonicalize(&path) else {
                continue;
            };

            if meta.is_dir() {
                if remaining_depth > 0 {
                    self.walk(&canonical, remaining_depth - 1, visited_dirs, files)?;
                }
            } else {
                files.insert(canonical);
            }
        }
        Ok(())
    }

    // -- Favorites / pins / icons / child order (FR-005/FR-007/FR-008) --------

    /// Mark or unmark `path` as a Favorite (FR-007).
    pub fn set_favorite(&mut self, path: &str, favorite: bool) {
        if favorite {
            self.favorites.insert(path.to_owned());
        } else {
            self.favorites.remove(path);
        }
    }

    /// Whether `path` is currently a Favorite.
    #[must_use]
    pub fn is_favorite(&self, path: &str) -> bool {
        self.favorites.contains(path)
    }

    /// The set of favorited paths, in no particular order (FR-007). The caller
    /// resolves each path's current kind and orders them for display — the
    /// workspace stores favorites as a set, not an ordered list. Backs the FFI
    /// contract's `list_favorites` (§1).
    pub fn favorites(&self) -> impl Iterator<Item = &str> {
        self.favorites.iter().map(String::as_str)
    }

    /// Pin or unpin `path` for quick access (FR-007).
    pub fn set_pinned(&mut self, path: &str, pinned: bool) {
        if pinned {
            self.pinned.insert(path.to_owned());
        } else {
            self.pinned.remove(path);
        }
    }

    /// Whether `path` is currently pinned.
    #[must_use]
    pub fn is_pinned(&self, path: &str) -> bool {
        self.pinned.contains(path)
    }

    /// Assign (`Some`) or clear (`None`) a custom folder icon for `folder_path`
    /// (FR-008). The icon is an opaque id (SF Symbol / custom symbol name).
    pub fn set_folder_icon(&mut self, folder_path: &str, icon: Option<&str>) {
        match icon {
            Some(id) => {
                self.folder_icons
                    .insert(folder_path.to_owned(), id.to_owned());
            }
            None => {
                self.folder_icons.remove(folder_path);
            }
        }
    }

    /// The custom icon id for `folder_path`, or `None` for the default icon.
    #[must_use]
    pub fn folder_icon(&self, folder_path: &str) -> Option<&str> {
        self.folder_icons.get(folder_path).map(String::as_str)
    }

    /// Set the manual drag-drop child order for `folder_path` (FR-005). An empty
    /// `order` clears the override (back to natural sort).
    pub fn set_child_order(&mut self, folder_path: &str, order: Vec<String>) {
        if order.is_empty() {
            self.child_order.remove(folder_path);
        } else {
            self.child_order.insert(folder_path.to_owned(), order);
        }
    }

    /// The manual child order for `folder_path`, or `None` for natural sort.
    #[must_use]
    pub fn child_order(&self, folder_path: &str) -> Option<&[String]> {
        self.child_order.get(folder_path).map(Vec::as_slice)
    }

    // -- Collision-safe file operations (FR-004/FR-004a/FR-013a) --------------

    /// Create a new empty note named `name` under `parent`, returning its path
    /// (FFI contract §2). The name gets a `.md` extension if it lacks one, and
    /// is auto-suffixed to avoid clobbering an existing entry (FR-004a). The file
    /// is created via the atomic writer so an observer never sees a partial file.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
    /// [`EmendError::IoFailure`] if `parent` is missing or the write fails.
    pub fn create_note(&self, parent: &str, name: &str) -> Result<String, EmendError> {
        let parent_dir = Path::new(parent);
        ensure_dir(parent_dir)?;

        let with_ext = ensure_md_extension(name);
        let chosen = free_name(parent_dir, &with_ext);
        let target = parent_dir.join(&chosen);

        // Atomic create of an empty note (FR-009a path). `free_name` guaranteed
        // the target is currently free.
        write_atomic(&target, "")?;
        Ok(target.to_string_lossy().into_owned())
    }

    /// Create a new folder named `name` under `parent`, returning its path
    /// (FFI contract §2). Auto-suffixed to avoid clobbering (FR-004a).
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
    /// [`EmendError::IoFailure`] if `parent` is missing or the directory cannot
    /// be created.
    pub fn create_folder(&self, parent: &str, name: &str) -> Result<String, EmendError> {
        let parent_dir = Path::new(parent);
        ensure_dir(parent_dir)?;

        let chosen = free_name(parent_dir, name);
        let target = parent_dir.join(&chosen);
        std::fs::create_dir(&target).map_err(|e| map_io(&target, &e))?;
        Ok(target.to_string_lossy().into_owned())
    }

    /// Rename `path` to `new_name` within the same parent folder, returning the
    /// new path (FFI contract §2). Collision-safe: renaming onto an occupied name
    /// auto-suffixes instead of overwriting (FR-004a). Renaming to the entry's
    /// own current name is a no-op that returns the unchanged path.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] if `path` does not exist;
    /// [`EmendError::PermissionDenied`] / [`EmendError::IoFailure`] for rename
    /// failures.
    pub fn rename(&self, path: &str, new_name: &str) -> Result<String, EmendError> {
        let src = Path::new(path);
        let meta = std::fs::symlink_metadata(src).map_err(|e| map_io(src, &e))?;
        let parent = src.parent().unwrap_or_else(|| Path::new("."));

        // Files keep their extension policy; folders take the name verbatim.
        let requested = if meta.is_dir() {
            new_name.to_owned()
        } else {
            ensure_md_extension(new_name)
        };

        // Renaming to the same basename is a no-op (not a self-collision).
        if basename(src).as_deref() == Some(requested.as_str()) {
            return Ok(path.to_owned());
        }

        let chosen = free_name(parent, &requested);
        let dest = parent.join(&chosen);
        std::fs::rename(src, &dest).map_err(|e| map_io(src, &e))?;
        Ok(dest.to_string_lossy().into_owned())
    }

    /// Move `path` into `new_parent`, keeping its basename when free and
    /// auto-suffixing on collision (FR-004a / FR-005). Returns the new path.
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] if `path` or `new_parent` is missing;
    /// [`EmendError::PermissionDenied`] / [`EmendError::IoFailure`] for move
    /// failures.
    pub fn move_node(&self, path: &str, new_parent: &str) -> Result<String, EmendError> {
        let src = Path::new(path);
        std::fs::symlink_metadata(src).map_err(|e| map_io(src, &e))?;
        let dest_dir = Path::new(new_parent);
        ensure_dir(dest_dir)?;

        let name = basename(src).ok_or_else(|| EmendError::InvalidConfig {
            detail: format!("cannot move a path with no basename: {path}"),
        })?;
        let chosen = free_name(dest_dir, &name);
        let dest = dest_dir.join(&chosen);
        std::fs::rename(src, &dest).map_err(|e| map_io(src, &e))?;
        Ok(dest.to_string_lossy().into_owned())
    }

    /// Delete `path` (FFI contract §2). A file is removed; a folder is removed
    /// recursively with its contents.
    ///
    /// This touches **disk only**. It does **not** drop any app-managed
    /// preferences (favorite/pin/icon/child-order) keyed on the path: those live
    /// Swift-side, which is the source of truth for them and prunes entries for
    /// paths that no longer exist. (`delete` takes `&self`, so it could not mutate
    /// such maps even if they lived here.)
    ///
    /// # Errors
    ///
    /// [`EmendError::NotFound`] if `path` does not exist;
    /// [`EmendError::PermissionDenied`] / [`EmendError::IoFailure`] for removal
    /// failures.
    pub fn delete(&self, path: &str) -> Result<(), EmendError> {
        let p = Path::new(path);
        let meta = std::fs::symlink_metadata(p).map_err(|e| map_io(p, &e))?;
        if meta.is_dir() {
            std::fs::remove_dir_all(p).map_err(|e| map_io(p, &e))?;
        } else {
            std::fs::remove_file(p).map_err(|e| map_io(p, &e))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Free functions (naming + helpers)
// ---------------------------------------------------------------------------

/// The basename (final path component) of `path` as an owned `String`, or `None`
/// if the path has no final component (e.g. `/`).
fn basename(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Ensure `name` ends in `.md` (case-insensitive check), appending it if not.
/// Notes are `.md` files (data-model "Note").
fn ensure_md_extension(name: &str) -> String {
    let has_md = Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if has_md {
        name.to_owned()
    } else {
        format!("{name}.md")
    }
}

/// Pick a non-colliding basename for `desired` inside `dir` (FR-004a / FR-013a).
///
/// If `dir/desired` is free, returns `desired` verbatim. Otherwise appends a
/// space and the lowest integer ≥ 2 that frees the name, inserting it **before
/// the final extension** (`note.md` → `note 2.md`) for names with an extension,
/// or at the end (`folder` → `folder 2`) for names without one.
///
/// Existence is tested with [`Path::exists`] which follows symlinks; a name
/// occupied by a symlink (even a dangling one's target spelling) is still treated
/// as taken, because the goal is never to clobber a path the user can see.
fn free_name(dir: &Path, desired: &str) -> String {
    if !dir.join(desired).exists() {
        return desired.to_owned();
    }

    // Split on the LAST dot so multi-dot names keep only their final extension
    // (`a.tar.gz` → stem `a.tar`, ext `gz`). A leading-dot dotfile (`.env`) has
    // no stem before the dot, so treat the whole thing as the stem (suffix goes
    // at the end: `.env 2`).
    let (stem, ext) = match desired.rfind('.') {
        Some(0) | None => (desired, ""),
        Some(idx) => (&desired[..idx], &desired[idx + 1..]),
    };

    // Climb 2, 3, 4, … until a free name appears. Bounded in practice by the
    // number of existing collisions; `u32` headroom is far beyond any real dir.
    let mut n: u32 = 2;
    loop {
        let candidate = if ext.is_empty() {
            format!("{stem} {n}")
        } else {
            format!("{stem} {n}.{ext}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
        // Saturate rather than overflow; at u32::MAX a dir is pathological and we
        // return the last candidate (the caller's create will then surface any
        // real IO error). This branch is unreachable for any real filesystem.
        n = match n.checked_add(1) {
            Some(next) => next,
            None => return candidate,
        };
    }
}

/// Verify `dir` exists and is a directory, mapping the failure to the right
/// [`EmendError`]. Used by create/move so a missing parent is a clear error
/// rather than a confusing downstream write failure.
fn ensure_dir(dir: &Path) -> Result<(), EmendError> {
    let meta = std::fs::metadata(dir).map_err(|e| map_io(dir, &e))?;
    if meta.is_dir() {
        Ok(())
    } else {
        Err(EmendError::InvalidConfig {
            detail: format!("parent is not a directory: {}", dir.display()),
        })
    }
}

/// Map a [`std::io::Error`] onto the appropriate [`EmendError`], attaching the
/// offending path. Mirrors the mapping in [`crate::fs`] and [`crate::document`]
/// so workspace errors match the rest of the core.
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
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{ensure_md_extension, free_name, LocationId, Workspace};
    use crate::EmendError;
    use tempfile::tempdir;

    // -- naming helpers -------------------------------------------------------

    #[test]
    fn ensure_md_extension_appends_when_missing() {
        assert_eq!(ensure_md_extension("note"), "note.md");
        assert_eq!(ensure_md_extension("note.md"), "note.md");
        // Case-insensitive: don't double up on `.MD`.
        assert_eq!(ensure_md_extension("note.MD"), "note.MD");
        // A non-md extension is preserved AND .md appended (the file IS a note).
        assert_eq!(ensure_md_extension("note.txt"), "note.txt.md");
    }

    #[test]
    fn free_name_returns_desired_when_free() {
        let dir = tempdir().unwrap();
        assert_eq!(free_name(dir.path(), "note.md"), "note.md");
        assert_eq!(free_name(dir.path(), "folder"), "folder");
    }

    #[test]
    fn free_name_suffixes_before_extension() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), b"x").unwrap();
        assert_eq!(free_name(dir.path(), "note.md"), "note 2.md");
    }

    #[test]
    fn free_name_climbs_suffix() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), b"x").unwrap();
        std::fs::write(dir.path().join("note 2.md"), b"x").unwrap();
        assert_eq!(free_name(dir.path(), "note.md"), "note 3.md");
    }

    #[test]
    fn free_name_folder_has_no_extension_split() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Projects")).unwrap();
        assert_eq!(free_name(dir.path(), "Projects"), "Projects 2");
    }

    #[test]
    fn free_name_multi_dot_splits_last_extension_only() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.tar.gz"), b"x").unwrap();
        assert_eq!(free_name(dir.path(), "a.tar.gz"), "a.tar 2.gz");
    }

    #[test]
    fn free_name_dotfile_suffix_goes_at_end() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), b"x").unwrap();
        assert_eq!(free_name(dir.path(), ".env"), ".env 2");
    }

    // -- locations ------------------------------------------------------------

    #[test]
    fn add_location_assigns_ids_and_defaults_name_to_basename() {
        let dir = tempdir().unwrap();
        let mut ws = Workspace::new();

        let loc = ws.add_location(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(loc.id, LocationId(0));
        assert_eq!(loc.order, 0);
        // Default display name is the folder basename.
        let expected = dir.path().file_name().unwrap().to_string_lossy();
        assert_eq!(loc.display_name, expected);
    }

    #[test]
    fn add_location_rejects_non_directory() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.md");
        std::fs::write(&file, b"x").unwrap();
        let mut ws = Workspace::new();
        assert!(matches!(
            ws.add_location(file.to_str().unwrap()),
            Err(EmendError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn add_location_missing_is_not_found() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        let mut ws = Workspace::new();
        assert!(matches!(
            ws.add_location(missing.to_str().unwrap()),
            Err(EmendError::NotFound { .. })
        ));
    }

    #[test]
    fn list_locations_is_ordered() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let mut ws = Workspace::new();
        let la = ws.add_location(a.path().to_str().unwrap()).unwrap();
        let lb = ws.add_location(b.path().to_str().unwrap()).unwrap();

        let listed = ws.list_locations();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, la.id);
        assert_eq!(listed[1].id, lb.id);
    }

    #[test]
    fn remove_location_drops_it_and_unknown_is_not_found() {
        let dir = tempdir().unwrap();
        let mut ws = Workspace::new();
        let loc = ws.add_location(dir.path().to_str().unwrap()).unwrap();

        ws.remove_location(loc.id).unwrap();
        assert!(ws.list_locations().is_empty());

        assert!(matches!(
            ws.remove_location(loc.id),
            Err(EmendError::NotFound { .. })
        ));
    }

    #[test]
    fn reorder_locations_reassigns_order() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let c = tempdir().unwrap();
        let mut ws = Workspace::new();
        let la = ws.add_location(a.path().to_str().unwrap()).unwrap();
        let lb = ws.add_location(b.path().to_str().unwrap()).unwrap();
        let lc = ws.add_location(c.path().to_str().unwrap()).unwrap();

        // Reverse the order explicitly.
        ws.reorder_locations(&[lc.id, lb.id, la.id]);
        let listed = ws.list_locations();
        assert_eq!(listed[0].id, lc.id);
        assert_eq!(listed[1].id, lb.id);
        assert_eq!(listed[2].id, la.id);
    }

    // -- preferences store ----------------------------------------------------

    #[test]
    fn favorites_pins_icons_child_order_round_trip() {
        let mut ws = Workspace::new();

        assert!(!ws.is_favorite("/a"));
        ws.set_favorite("/a", true);
        assert!(ws.is_favorite("/a"));
        ws.set_favorite("/a", false);
        assert!(!ws.is_favorite("/a"));

        assert!(!ws.is_pinned("/b"));
        ws.set_pinned("/b", true);
        assert!(ws.is_pinned("/b"));

        assert_eq!(ws.folder_icon("/f"), None);
        ws.set_folder_icon("/f", Some("folder.badge.star"));
        assert_eq!(ws.folder_icon("/f"), Some("folder.badge.star"));
        ws.set_folder_icon("/f", None);
        assert_eq!(ws.folder_icon("/f"), None);

        assert_eq!(ws.child_order("/d"), None);
        ws.set_child_order("/d", vec!["b.md".to_owned(), "a.md".to_owned()]);
        assert_eq!(
            ws.child_order("/d"),
            Some(["b.md".to_owned(), "a.md".to_owned()].as_slice())
        );
        // Empty order clears the override.
        ws.set_child_order("/d", vec![]);
        assert_eq!(ws.child_order("/d"), None);
    }
}
