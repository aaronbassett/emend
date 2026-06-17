//! T054 (file-ops half) — failing-first integration tests for collision-safe
//! file operations in the file-based workspace (`emend_core::workspace`).
//!
//! These encode the **collision-safety** half of US2's file operations:
//! create / rename / move must NEVER overwrite an existing file or folder
//! (FR-004/FR-004a, FR-013a). Where the requested name is taken, the operation
//! disambiguates with a deterministic auto-suffix rather than clobbering.
//!
//! ## Collision-naming scheme (pinned here as an executable contract)
//!
//! When a target name is already taken, the workspace appends a space + the
//! lowest integer ≥ 2 that yields a free name, inserting it **before the file
//! extension** for files and at the end for folders:
//!
//! - `note.md` taken → `note 2.md`, then `note 3.md`, …
//! - `folder` taken → `folder 2`, then `folder 3`, …
//! - multi-dot `a.tar.gz` taken → `a 2.tar.gz` (suffix splits on the LAST dot, so
//!   only the final `.gz` is treated as the extension — matching how the sidebar
//!   shows the user a single extension).
//!
//! This mirrors the Finder-style "name 2" convention and is fully deterministic,
//! so the next free name is reproducible across runs and machines.
//!
//! NOTE: The conflict **truth table** (open file changed on disk: clean → silent
//! reload, dirty → preserve local + mark stale, per FR-006c) is DEFERRED to the
//! watcher slice (T057/T065). It depends on the file watcher and the open-document
//! dirty-state, neither of which exists in this dependency-free slice. Only the
//! name-collision half of T054 lives here.

// Integration tests assert on their own fixtures; the workspace denies these in
// library code, so scope the allowance to this test module.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::workspace::Workspace;
use emend_core::EmendError;
use std::path::Path;
use tempfile::tempdir;

/// The basename of a path, as a `&str` (test helper).
fn name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// create_note (FR-004/FR-004a)
// ---------------------------------------------------------------------------

/// Creating a note in an empty folder uses the requested name verbatim and the
/// returned path actually exists on disk.
#[test]
fn create_note_uses_requested_name_when_free() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();

    let created = ws
        .create_note(dir.path().to_str().unwrap(), "note.md")
        .unwrap();

    assert_eq!(name_of(&created), "note.md");
    assert!(Path::new(&created).is_file());
}

/// A note name without an extension gets `.md` appended (notes are `.md` files).
#[test]
fn create_note_appends_md_extension_when_missing() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();

    let created = ws
        .create_note(dir.path().to_str().unwrap(), "ideas")
        .unwrap();

    assert_eq!(name_of(&created), "ideas.md");
    assert!(Path::new(&created).is_file());
}

/// Creating `note.md` when it already exists must NOT overwrite it; the new file
/// is auto-suffixed to `note 2.md`, and the original is byte-for-byte intact.
#[test]
fn create_note_collision_auto_suffixes_and_preserves_original() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();

    let first = ws
        .create_note(dir.path().to_str().unwrap(), "note.md")
        .unwrap();
    std::fs::write(&first, "ORIGINAL").unwrap();

    let second = ws
        .create_note(dir.path().to_str().unwrap(), "note.md")
        .unwrap();

    assert_eq!(name_of(&second), "note 2.md");
    assert_ne!(first, second);
    // The original was never touched.
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "ORIGINAL");
    // Both files exist independently.
    assert!(Path::new(&first).is_file());
    assert!(Path::new(&second).is_file());
}

/// A run of collisions climbs the suffix deterministically: 2, then 3, then 4.
#[test]
fn create_note_collisions_climb_suffix_sequence() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let a = ws.create_note(parent, "note.md").unwrap();
    let b = ws.create_note(parent, "note.md").unwrap();
    let c = ws.create_note(parent, "note.md").unwrap();
    let d = ws.create_note(parent, "note.md").unwrap();

    assert_eq!(name_of(&a), "note.md");
    assert_eq!(name_of(&b), "note 2.md");
    assert_eq!(name_of(&c), "note 3.md");
    assert_eq!(name_of(&d), "note 4.md");
}

/// The suffix is inserted before the extension, not after it.
#[test]
fn create_note_suffix_goes_before_extension() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    ws.create_note(parent, "report.md").unwrap();
    let second = ws.create_note(parent, "report.md").unwrap();

    assert_eq!(name_of(&second), "report 2.md");
    assert!(name_of(&second).ends_with(".md"));
}

// ---------------------------------------------------------------------------
// create_folder (FR-004/FR-004a)
// ---------------------------------------------------------------------------

/// Creating a folder when free uses the requested name and the directory exists.
#[test]
fn create_folder_uses_requested_name_when_free() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();

    let created = ws
        .create_folder(dir.path().to_str().unwrap(), "Projects")
        .unwrap();

    assert_eq!(name_of(&created), "Projects");
    assert!(Path::new(&created).is_dir());
}

/// A folder collision auto-suffixes with no extension handling (folders have no
/// extension), and never clobbers the existing folder's contents.
#[test]
fn create_folder_collision_auto_suffixes() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let first = ws.create_folder(parent, "Projects").unwrap();
    std::fs::write(Path::new(&first).join("keep.md"), "x").unwrap();

    let second = ws.create_folder(parent, "Projects").unwrap();

    assert_eq!(name_of(&second), "Projects 2");
    assert!(Path::new(&second).is_dir());
    // The first folder's contents survive.
    assert!(Path::new(&first).join("keep.md").is_file());
}

// ---------------------------------------------------------------------------
// rename (FR-004a)
// ---------------------------------------------------------------------------

/// A rename to a free name moves the file to the new basename in the same folder.
#[test]
fn rename_to_free_name_moves_file() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let original = ws.create_note(parent, "draft.md").unwrap();
    std::fs::write(&original, "BODY").unwrap();

    let renamed = ws.rename(&original, "final.md").unwrap();

    assert_eq!(name_of(&renamed), "final.md");
    assert!(!Path::new(&original).exists(), "old path is gone");
    assert!(Path::new(&renamed).is_file());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "BODY");
}

/// Renaming INTO an occupied name must not overwrite the occupant; it
/// auto-suffixes instead, and the occupant is untouched.
#[test]
fn rename_into_occupied_name_auto_suffixes_and_preserves_occupant() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let occupant = ws.create_note(parent, "taken.md").unwrap();
    std::fs::write(&occupant, "OCCUPANT").unwrap();

    let mover = ws.create_note(parent, "mover.md").unwrap();
    std::fs::write(&mover, "MOVER").unwrap();

    let renamed = ws.rename(&mover, "taken.md").unwrap();

    // Did not clobber the occupant.
    assert_eq!(name_of(&renamed), "taken 2.md");
    assert_eq!(std::fs::read_to_string(&occupant).unwrap(), "OCCUPANT");
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "MOVER");
}

/// Renaming a file to its OWN current name is a harmless no-op that returns the
/// same path (not a self-collision that would suffix to " 2").
#[test]
fn rename_to_same_name_is_noop() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let original = ws.create_note(parent, "same.md").unwrap();
    std::fs::write(&original, "DATA").unwrap();

    let renamed = ws.rename(&original, "same.md").unwrap();

    assert_eq!(name_of(&renamed), "same.md");
    assert!(Path::new(&renamed).is_file());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "DATA");
}

/// Renaming a missing path is a clear NotFound, not a panic.
#[test]
fn rename_missing_path_is_not_found() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();
    let ghost = dir.path().join("ghost.md");

    let err = ws.rename(ghost.to_str().unwrap(), "x.md").unwrap_err();
    assert!(matches!(err, EmendError::NotFound { .. }));
}

// ---------------------------------------------------------------------------
// move_node (FR-004a / FR-005 drag-drop)
// ---------------------------------------------------------------------------

/// Moving a file into another folder relocates it and keeps its basename when
/// the destination is free.
#[test]
fn move_into_folder_relocates_file() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let dest = ws.create_folder(parent, "dest").unwrap();
    let note = ws.create_note(parent, "wander.md").unwrap();
    std::fs::write(&note, "TRAVELLER").unwrap();

    let moved = ws.move_node(&note, &dest).unwrap();

    assert_eq!(name_of(&moved), "wander.md");
    assert!(!Path::new(&note).exists());
    assert!(Path::new(&moved).is_file());
    assert_eq!(Path::new(&moved).parent().unwrap(), Path::new(&dest));
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "TRAVELLER");
}

/// Moving a file into a folder that already has a file of that name must
/// auto-suffix, never overwrite the occupant.
#[test]
fn move_into_folder_with_collision_auto_suffixes() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let dest = ws.create_folder(parent, "dest").unwrap();
    // Occupant already living in dest.
    let occupant = ws.create_note(&dest, "dup.md").unwrap();
    std::fs::write(&occupant, "OCCUPANT").unwrap();

    // A same-named file elsewhere that we move into dest.
    let note = ws.create_note(parent, "dup.md").unwrap();
    std::fs::write(&note, "MOVER").unwrap();

    let moved = ws.move_node(&note, &dest).unwrap();

    assert_eq!(name_of(&moved), "dup 2.md");
    assert_eq!(std::fs::read_to_string(&occupant).unwrap(), "OCCUPANT");
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "MOVER");
}

// ---------------------------------------------------------------------------
// delete (FR-004)
// ---------------------------------------------------------------------------

/// Deleting a file removes it from disk.
#[test]
fn delete_file_removes_it() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let note = ws.create_note(parent, "doomed.md").unwrap();
    assert!(Path::new(&note).is_file());

    ws.delete(&note).unwrap();
    assert!(!Path::new(&note).exists());
}

/// Deleting a folder removes it and its contents (recursive).
#[test]
fn delete_folder_removes_tree() {
    let dir = tempdir().unwrap();
    let parent = dir.path().to_str().unwrap();
    let ws = Workspace::new();

    let folder = ws.create_folder(parent, "tree").unwrap();
    ws.create_note(&folder, "leaf.md").unwrap();

    ws.delete(&folder).unwrap();
    assert!(!Path::new(&folder).exists());
}

/// Deleting a missing path is a clear NotFound, not a panic.
#[test]
fn delete_missing_path_is_not_found() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();
    let ghost = dir.path().join("ghost.md");

    let err = ws.delete(ghost.to_str().unwrap()).unwrap_err();
    assert!(matches!(err, EmendError::NotFound { .. }));
}
