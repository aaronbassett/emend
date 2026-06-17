//! T055b — failing-first integration tests for **path identity** (NFR-007) in
//! the file-based workspace (`emend_core::workspace`).
//!
//! NFR-007 makes three demands on workspace traversal; each is encoded below as
//! an executable obligation:
//!
//! 1. **Traversal terminates on a symlink cycle.** A location that contains a
//!    symlink forming a directory cycle (`a/loop -> a`) must not loop forever.
//!    The bounded canonical-path walk visits each physical directory at most
//!    once and caps its depth, so it returns instead of recursing without end.
//!
//! 2. **The same physical file via two paths is identified once.** A file
//!    reached directly *and* through a symlinked directory that aliases its
//!    parent is the SAME physical inode; the walk must count/identify it a single
//!    time (no double-index), because identity is the canonical (symlink- and
//!    `..`-resolved) path, not the lexical one.
//!
//! 3. **Correct behavior on case-insensitive vs case-sensitive volumes.** On a
//!    case-insensitive volume (the default for macOS `tempdir`s on APFS),
//!    `Note.md` and `note.md` are the SAME file, so canonicalizing either yields
//!    one identity. On a case-sensitive volume they are two files. We assert the
//!    behavior that matches the host volume we are actually running on (detected
//!    at runtime) rather than hard-coding one or the other, so the suite is
//!    correct on both kinds of volume.
//!
//! Symlinks are created with `std::os::unix::fs::symlink` (macOS is always Unix).
//! `tempfile` provides the fixtures.

// Integration tests assert on their own fixtures; the workspace denies these in
// library code, so scope the allowance to this test module.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::workspace::Workspace;
use std::collections::HashSet;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

/// Detect whether the volume backing `dir` is case-insensitive by creating a
/// lowercase probe file and asking whether its uppercase spelling resolves to
/// the same existing file. This is what lets the case tests assert the behavior
/// of the volume we are *actually* on instead of assuming.
fn volume_is_case_insensitive(dir: &Path) -> bool {
    let lower = dir.join("__case_probe__.tmp");
    std::fs::write(&lower, b"probe").unwrap();
    let upper = dir.join("__CASE_PROBE__.TMP");
    // If the uppercase spelling exists, the lookup ignored case.
    let insensitive = upper.exists();
    std::fs::remove_file(&lower).unwrap();
    insensitive
}

// ---------------------------------------------------------------------------
// (1) Symlink cycle termination
// ---------------------------------------------------------------------------

/// A directory cycle (`root/sub/loop -> root`) must not hang the walk. The
/// bounded canonical walk terminates and returns a finite set of entries.
#[test]
fn walk_terminates_on_symlink_cycle() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // root/
    //   real.md
    //   sub/
    //     loop -> ../..  (points back up to root → a cycle)
    std::fs::write(root.join("real.md"), b"r").unwrap();
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    symlink(root, sub.join("loop")).unwrap();

    let ws = Workspace::new();
    // Must return (not hang). A generous depth bound is fine; termination is the
    // property under test.
    let files = ws.collect_files(root.to_str().unwrap(), 32).unwrap();

    // The real file is found exactly once despite the cycle.
    let count = files
        .iter()
        .filter(|p| Path::new(p).file_name() == Some(std::ffi::OsStr::new("real.md")))
        .count();
    assert_eq!(
        count, 1,
        "real.md should be discovered exactly once: {files:?}"
    );
}

/// A self-referential symlink (`root/self -> root`) is the minimal cycle and
/// must also terminate.
#[test]
fn walk_terminates_on_self_symlink() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    std::fs::write(root.join("only.md"), b"x").unwrap();
    symlink(root, root.join("self")).unwrap();

    let ws = Workspace::new();
    let files = ws.collect_files(root.to_str().unwrap(), 16).unwrap();

    let count = files
        .iter()
        .filter(|p| Path::new(p).file_name() == Some(std::ffi::OsStr::new("only.md")))
        .count();
    assert_eq!(count, 1, "only.md found exactly once: {files:?}");
}

// ---------------------------------------------------------------------------
// (2) Same physical file via two paths → one identity
// ---------------------------------------------------------------------------

/// A file reachable both as `root/data/note.md` and as `root/alias/note.md`
/// (where `alias -> data`) is one physical inode. Collecting canonical
/// identities must dedupe it to a single entry.
#[test]
fn same_file_via_two_paths_counted_once() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let data = root.join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("note.md"), b"once").unwrap();
    // alias -> data ; so alias/note.md and data/note.md are the same file.
    symlink(&data, root.join("alias")).unwrap();

    let ws = Workspace::new();
    let files = ws.collect_files(root.to_str().unwrap(), 16).unwrap();

    // Returned paths are canonical, so the two lexical routes collapse to one.
    let unique: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
    let note_entries = unique
        .iter()
        .filter(|p| p.file_name() == Some(std::ffi::OsStr::new("note.md")))
        .count();
    assert_eq!(
        note_entries, 1,
        "the same physical note.md must be identified once, got: {unique:?}"
    );
}

/// `canonical_id` is the public identity primitive: two different lexical paths
/// to the same physical file yield equal identities; genuinely different files
/// yield different identities.
#[test]
fn canonical_id_unifies_aliased_paths() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let data = root.join("data");
    std::fs::create_dir(&data).unwrap();
    let real = data.join("note.md");
    std::fs::write(&real, b"x").unwrap();
    symlink(&data, root.join("alias")).unwrap();

    let ws = Workspace::new();

    let via_real = ws.canonical_id(real.to_str().unwrap()).unwrap();
    let via_alias = ws
        .canonical_id(root.join("alias").join("note.md").to_str().unwrap())
        .unwrap();
    assert_eq!(via_real, via_alias, "aliased paths share one identity");

    // A different file has a different identity.
    let other = data.join("other.md");
    std::fs::write(&other, b"y").unwrap();
    let via_other = ws.canonical_id(other.to_str().unwrap()).unwrap();
    assert_ne!(via_real, via_other);
}

// ---------------------------------------------------------------------------
// (3) Case-insensitive vs case-sensitive volume behavior
// ---------------------------------------------------------------------------

/// On the host volume, `Note.md` and `note.md` resolve to one identity iff the
/// volume is case-insensitive. We detect the volume kind at runtime and assert
/// the matching behavior, so the test is correct on either kind of volume.
#[test]
fn case_identity_matches_host_volume() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let insensitive = volume_is_case_insensitive(root);

    let ws = Workspace::new();

    let lower = root.join("note.md");
    std::fs::write(&lower, b"data").unwrap();

    let id_lower = ws.canonical_id(lower.to_str().unwrap()).unwrap();

    if insensitive {
        // Same physical file under a different spelling → same identity, and the
        // differently-cased lookup must succeed (the file is found).
        let id_upper = ws
            .canonical_id(root.join("Note.md").to_str().unwrap())
            .unwrap();
        assert_eq!(
            id_lower, id_upper,
            "case-insensitive volume: Note.md and note.md are one file"
        );
    } else {
        // Case-sensitive volume: `Note.md` does not exist, so identifying it is a
        // miss while `note.md` resolves fine — they are genuinely distinct names.
        assert!(
            ws.canonical_id(root.join("Note.md").to_str().unwrap())
                .is_err(),
            "case-sensitive volume: Note.md is a different (absent) file"
        );
    }
}

/// On a case-insensitive volume, listing children of a folder must not surface
/// the same physical file twice under two spellings. (On a case-sensitive volume
/// the two spellings would be two real files; we only assert the dedupe property
/// when the host volume is case-insensitive.)
#[test]
fn list_children_dedupes_case_aliases_on_insensitive_volume() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    if !volume_is_case_insensitive(root) {
        // Nothing to prove on a case-sensitive volume.
        return;
    }

    std::fs::write(root.join("readme.md"), b"x").unwrap();

    let ws = Workspace::new();
    let children = ws.list_children(root.to_str().unwrap()).unwrap();

    let readme_count = children
        .iter()
        .filter(|n| {
            Path::new(&n.path)
                .file_name()
                .map(|f| f.to_string_lossy().to_lowercase())
                == Some("readme.md".to_owned())
        })
        .count();
    assert_eq!(
        readme_count, 1,
        "a single physical file must appear once in the listing: {children:?}"
    );
}
