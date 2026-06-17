//! Atomic + durable writes and tolerant reads — the byte gateway between Emend
//! and the user's files on disk (research §B4, FR-003a, FR-009a).
//!
//! This module is the single place where note content crosses the filesystem
//! boundary, and it is **Constitution Principle III territory (never lose user
//! data)**. Two guarantees matter:
//!
//! 1. **Atomic + durable writes ([`write_atomic`]).** An external observer — the
//!    file watcher, `git`, an AI agent rewriting files — must never see a
//!    half-written note (FR-009a). We write to a sibling temp file in the
//!    *same directory* as the target, fsync it, then `rename(2)` it over the
//!    target, then fsync the directory. `rename` is atomic on a single
//!    filesystem, so a reader sees the complete old file or the complete new
//!    file, never a prefix or a zero-length truncation.
//!
//! 2. **Tolerant reads ([`read_tolerant`]).** Files written by other tools are
//!    read successfully rather than rejected (FR-003a): a leading UTF-8 BOM is
//!    stripped, line endings are preserved verbatim, and invalid UTF-8 decodes
//!    lossily instead of erroring.
//!
//! ## Durability on macOS — why there is no manual `F_FULLFSYNC` here
//!
//! On Apple platforms a plain POSIX `fsync(2)` schedules the write but does
//! **not** wait for the drive to flush its own cache, so it is not truly
//! durable across power loss; the documented remedy is `fcntl(fd,
//! F_FULLFSYNC)`. We do **not** call `fcntl` ourselves, because Rust's standard
//! library already does it for us: on `target_vendor = "apple"`,
//! [`std::fs::File::sync_all`] is implemented as `fcntl(fd, F_FULLFSYNC)` (and
//! `sync_data` likewise). This was fixed in rust-lang/rust#60121 (issue #55920)
//! and is present in every toolchain at or above our pinned MSRV of 1.85 —
//! verified directly against current `library/std/src/sys/fs/unix.rs`:
//!
//! ```text
//! pub fn fsync(&self) -> io::Result<()> {
//!     cvt_r(|| unsafe { os_fsync(self.as_raw_fd()) })?; ...
//!     #[cfg(target_vendor = "apple")]
//!     unsafe fn os_fsync(fd: c_int) -> c_int { libc::fcntl(fd, libc::F_FULLFSYNC) }
//! }
//! ```
//!
//! Therefore calling `file.sync_all()` from safe Rust gives us the strong
//! Apple durability guarantee with **no extra dependency and no `unsafe` at our
//! call sites** — exactly the decision recorded in research §B4 ("On Apple
//! targets Rust std's `sync_all` already issues `F_FULLFSYNC` → true durability,
//! no manual `fcntl`"). Adding a `rustix`/`libc` `fcntl(F_FULLFSYNC)` call would
//! be redundant work plus an `unsafe` block for zero benefit on our only
//! shipping target. On non-Apple platforms (CI runners, contributors on Linux)
//! `sync_all` is a normal `fsync`, which is the correct best-effort there.
//!
//! `F_FULLFSYNC` is comparatively slow, so callers MUST debounce autosave rather
//! than flush per keystroke (research §B4 risk note; the document/autosave layer
//! owns that policy — this module just makes each individual write durable).

use crate::EmendError;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

/// One leading UTF-8 byte-order mark (`U+FEFF`), as it appears on disk.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// The subdirectory (relative to a note's own folder) where dropped media is
/// stored (FR-013/FR-013a). Obsidian-style `attachments/` next to the note.
pub const ATTACHMENTS_DIR: &str = "attachments";

/// The basename stem used when a dropped attachment has no usable name — an empty
/// or extension-only `suggested_name` (FR-013a "untitled/unsaved" fallback).
const UNTITLED_STEM: &str = "untitled";

/// Atomically and durably write `contents` to `path`, replacing any existing
/// file (FR-009a).
///
/// Sequence (all on the target's own filesystem so the rename is atomic):
/// 1. Create a sibling temp file in `path`'s parent directory.
/// 2. Write `contents`, then `flush` + `sync_all` the temp file. On Apple
///    targets `sync_all` is `fcntl(F_FULLFSYNC)`, so the data is physically
///    durable before we expose it.
/// 3. `persist` (atomic `rename(2)`) the temp file over `path`.
/// 4. `sync_all` the containing directory so the rename itself is durable
///    (a crash after rename but before the directory entry is flushed could
///    otherwise lose the update).
///
/// Because the new bytes only become visible at the target via the final atomic
/// rename, a concurrent reader (or a crash at any point before the rename) sees
/// the complete old file or the complete new file — never a partial one.
///
/// # Errors
///
/// Returns [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
/// [`EmendError::IoFailure`] if the parent directory is missing, the process
/// lacks permission, or any IO step (temp create, write, fsync, rename) fails.
/// Never panics.
pub fn write_atomic(path: impl AsRef<Path>, contents: &str) -> Result<(), EmendError> {
    write_atomic_bytes(path, contents.as_bytes())
}

/// Atomically and durably write raw `bytes` to `path`, replacing any existing
/// file. The byte-oriented sibling of [`write_atomic`] (which is just this with a
/// `&str`'s bytes); used for binary attachments (FR-013a), which are not text.
///
/// Identical sequence to [`write_atomic`] — sibling temp file → `sync_all`
/// (`F_FULLFSYNC` on Apple) → atomic `rename(2)` → directory fsync — so an
/// external observer never sees a half-written file.
///
/// # Errors
///
/// Returns [`EmendError::NotFound`] / [`EmendError::PermissionDenied`] /
/// [`EmendError::IoFailure`] if the parent directory is missing, the process
/// lacks permission, or any IO step (temp create, write, fsync, rename) fails.
/// Never panics.
pub fn write_atomic_bytes(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), EmendError> {
    let path = path.as_ref();

    // The temp file MUST live in the same directory as the target so that
    // `persist` is a same-filesystem `rename(2)` (atomic) rather than a
    // cross-device copy (non-atomic, and a hard error on some mounts). Notes can
    // live on external/network volumes, so we never use the system temp dir.
    let dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut temp = tempfile::Builder::new()
        .prefix(".emend-tmp-")
        .tempfile_in(dir)
        .map_err(|e| map_io(path, &e))?;

    // Write + flush the bytes into the temp file.
    temp.write_all(bytes).map_err(|e| map_io(path, &e))?;
    temp.flush().map_err(|e| map_io(path, &e))?;

    // Durability: on Apple this is `fcntl(F_FULLFSYNC)` (see module docs) — the
    // bytes hit the platter before we make them reachable at the target path.
    temp.as_file().sync_all().map_err(|e| map_io(path, &e))?;

    // Atomic rename over the target. `persist` consumes the temp file; on
    // failure it hands back a `PersistError` whose `.error` is the io::Error.
    temp.persist(path).map_err(|e| map_io(path, &e.error))?;

    // Durably record the rename in the directory entry itself.
    sync_dir(dir)?;

    Ok(())
}

/// Store a dropped media attachment beside a note, returning the **note-relative**
/// reference to insert into the Markdown (FR-013/FR-013a).
///
/// `note_path` is the note the media was dropped into. `bytes` is the media
/// payload; `suggested_name` is the dropped file's name (used for the basename +
/// extension).
///
/// ## `note_path == None` is unsupported in v1
///
/// An attachment is stored in an `attachments/` directory *beside its note*, so
/// it needs a saved note to anchor on. When `note_path` is `None` (the note is
/// still untitled/unsaved) there is no such anchor, and this returns
/// [`EmendError::InvalidConfig`] rather than degrading. (The previous behaviour
/// fell back to the process's current working directory — which is `/` for a
/// sandboxed app, yielding an opaque `PermissionDenied`, and is process-global
/// mutable state besides.) The Swift caller already guards against this by
/// requiring the note to be saved first, so the `None` case is never hit on the
/// happy path; v1 simply makes the unsupported case an explicit, descriptive
/// error instead of a misleading IO failure.
///
/// ## Where it lands
///
/// The attachment is written into an [`ATTACHMENTS_DIR`] (`attachments/`)
/// subdirectory of the note's own folder, created if absent.
///
/// ## Collision-safe naming (FR-013a)
///
/// The filename is made collision-safe the same way the workspace's file ops are:
/// if `suggested_name` is taken in the attachments dir, a ` 2`, ` 3`, … suffix is
/// inserted before the extension (`image.png` → `image 2.png`). An empty or
/// extension-only `suggested_name` falls back to an [`UNTITLED_STEM`] basename
/// (`untitled`, `untitled.png`).
///
/// ## Return value
///
/// The returned string is the path to insert into the note as a relative-path
/// Markdown image — `attachments/<chosen-name>` — using forward slashes
/// (Markdown/portable), regardless of the host separator.
///
/// # Errors
///
/// [`EmendError::InvalidConfig`] if `note_path` is `None` (an attachment requires
/// a saved note in v1; see above). [`EmendError::PermissionDenied`] /
/// [`EmendError::IoFailure`] if the attachments directory cannot be created or the
/// atomic write fails. Never panics.
pub fn store_attachment(
    note_path: Option<&str>,
    bytes: &[u8],
    suggested_name: &str,
) -> Result<String, EmendError> {
    // An attachment lands beside its note, so it needs a saved note to anchor on.
    // An untitled/unsaved note (`note_path == None`) is unsupported in v1: rather
    // than writing into the process CWD (which is `/` under the app sandbox, and
    // is process-global mutable state regardless), return a clear error. The Swift
    // caller guards against this, so it is never reached on the happy path.
    let Some(note_path) = note_path else {
        return Err(EmendError::InvalidConfig {
            detail: "cannot store an attachment for an unsaved note: save the note first"
                .to_owned(),
        });
    };

    // The note's own folder (its parent dir).
    let note_dir: PathBuf = Path::new(note_path)
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let attach_dir = note_dir.join(ATTACHMENTS_DIR);
    std::fs::create_dir_all(&attach_dir).map_err(|e| map_io(&attach_dir, &e))?;

    let desired = sanitize_attachment_name(suggested_name);
    let chosen = free_name(&attach_dir, &desired);
    let target = attach_dir.join(&chosen);

    write_atomic_bytes(&target, bytes)?;

    // Return the note-relative reference with forward slashes (portable Markdown).
    Ok(format!("{ATTACHMENTS_DIR}/{chosen}"))
}

/// Reduce a dropped file's name to a safe attachment basename (FR-013a):
/// take only the final path component (drop any directory parts a drag-source
/// might include), and substitute the [`UNTITLED_STEM`] when the result is empty
/// or extension-only (`.png` → `untitled.png`, `""` → `untitled`).
fn sanitize_attachment_name(suggested: &str) -> String {
    // Final path component only — never let a drop name escape the attachments dir.
    let base = Path::new(suggested)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = base.trim();

    if base.is_empty() {
        return UNTITLED_STEM.to_owned();
    }
    // Extension-only (a leading dot with no stem, e.g. ".png"): prepend the
    // untitled stem so the file has a name, keeping the extension.
    if base.starts_with('.') && !base[1..].contains('.') {
        return format!("{UNTITLED_STEM}{base}");
    }
    base.to_owned()
}

/// Pick a non-colliding basename for `desired` inside `dir` (FR-013a). If
/// `dir/desired` is free, returns `desired`; else appends a space and the lowest
/// integer ≥ 2 that frees the name, inserting it **before the final extension**
/// (`image.png` → `image 2.png`).
///
/// Mirrors `workspace::free_name` deliberately (the attachments dir wants the same
/// Finder-style "name 2" convention) but is kept private to `fs` so this module
/// has no dependency on `workspace`.
fn free_name(dir: &Path, desired: &str) -> String {
    if !dir.join(desired).exists() {
        return desired.to_owned();
    }

    // Split on the LAST dot so multi-dot names keep only their final extension;
    // a leading-dot name (`.env`) has no stem before the dot → whole-name stem.
    let (stem, ext) = match desired.rfind('.') {
        Some(0) | None => (desired, ""),
        Some(idx) => (&desired[..idx], &desired[idx + 1..]),
    };

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
        n = match n.checked_add(1) {
            Some(next) => next,
            None => return candidate,
        };
    }
}

/// Read `path` into a [`String`], tolerating files written by other tools
/// (FR-003a). This is the inverse gateway to [`write_atomic`].
///
/// Decoding contract (kept deliberately simple and lossless-where-possible):
/// * **BOM:** a single leading UTF-8 BOM (`U+FEFF`) is stripped. A BOM anywhere
///   else in the file is left untouched.
/// * **Line endings:** preserved verbatim — CRLF stays CRLF, LF stays LF. Any
///   normalization is a higher layer's choice, not the byte gateway's, so a
///   round-trip through [`write_atomic`] does not silently rewrite newlines.
/// * **Invalid UTF-8:** decoded lossily via [`String::from_utf8_lossy`], so the
///   read always succeeds (returns usable text with `U+FFFD` in place of
///   invalid byte sequences) instead of erroring. Valid UTF-8 round-trips
///   exactly.
///
/// # Errors
///
/// Returns [`EmendError::NotFound`] if the file does not exist,
/// [`EmendError::PermissionDenied`] if it cannot be opened, or
/// [`EmendError::IoFailure`] for any other read failure. Never panics, and
/// never errors merely because the bytes are not valid UTF-8.
pub fn read_tolerant(path: impl AsRef<Path>) -> Result<String, EmendError> {
    let path = path.as_ref();

    let mut file = File::open(path).map_err(|e| map_io(path, &e))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| map_io(path, &e))?;

    // Strip a single leading UTF-8 BOM if present.
    let body = if bytes.starts_with(&UTF8_BOM) {
        &bytes[UTF8_BOM.len()..]
    } else {
        &bytes[..]
    };

    // Lossy decode: never fail on non-UTF-8 input (FR-003a). `Cow` is borrowed
    // when the input is already valid UTF-8 (no allocation in the common case).
    Ok(String::from_utf8_lossy(body).into_owned())
}

/// `fsync` a directory so a rename within it is durable.
///
/// Opening a directory read-only and calling `sync_all` is the portable POSIX
/// way to flush its entries; on Apple targets this too is `F_FULLFSYNC` (see
/// module docs). A best-effort no-op semantics applies on platforms that reject
/// directory fsync — but on macOS/Linux it is meaningful and required.
fn sync_dir(dir: &Path) -> Result<(), EmendError> {
    let handle = File::open(dir).map_err(|e| map_io(dir, &e))?;
    handle.sync_all().map_err(|e| map_io(dir, &e))?;
    Ok(())
}

/// Map a [`std::io::Error`] onto the appropriate [`EmendError`], attaching the
/// offending path. The two cases the UI distinguishes (missing / forbidden) get
/// dedicated variants; everything else is a generic [`EmendError::IoFailure`]
/// carrying the OS message.
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
    // Unit tests assert on their own fixtures; the workspace denies unwrap/expect
    // in library code, so scope the allowance to this test module.
    #![allow(clippy::unwrap_used, reason = "unit test asserts on its own fixtures")]

    use super::{
        read_tolerant, sanitize_attachment_name, store_attachment, write_atomic, ATTACHMENTS_DIR,
    };
    use crate::EmendError;

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        write_atomic(&path, "hello\n").unwrap();
        assert_eq!(read_tolerant(&path).unwrap(), "hello\n");
    }

    #[test]
    fn sanitize_attachment_name_falls_back_to_untitled() {
        assert_eq!(sanitize_attachment_name(""), "untitled");
        assert_eq!(sanitize_attachment_name("   "), "untitled");
        // Extension-only → untitled keeps the extension.
        assert_eq!(sanitize_attachment_name(".png"), "untitled.png");
        // A normal name is kept; directory parts are stripped.
        assert_eq!(sanitize_attachment_name("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_attachment_name("/a/b/photo.jpg"), "photo.jpg");
    }

    #[test]
    fn store_attachment_writes_beside_note_and_returns_rel_ref() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("note.md");
        std::fs::write(&note, "# hi").unwrap();

        let rel = store_attachment(
            Some(note.to_str().unwrap()),
            b"\x89PNG fake bytes",
            "image.png",
        )
        .unwrap();
        assert_eq!(rel, "attachments/image.png");

        // The bytes landed in the note's own attachments/ dir.
        let stored = dir.path().join(ATTACHMENTS_DIR).join("image.png");
        assert_eq!(std::fs::read(stored).unwrap(), b"\x89PNG fake bytes");
    }

    #[test]
    fn store_attachment_is_collision_safe() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("note.md");
        std::fs::write(&note, "# hi").unwrap();
        let note_str = note.to_str().unwrap();

        let first = store_attachment(Some(note_str), b"one", "img.png").unwrap();
        let second = store_attachment(Some(note_str), b"two", "img.png").unwrap();
        assert_eq!(first, "attachments/img.png");
        assert_eq!(second, "attachments/img 2.png");
        // Both files exist with their own bytes (no overwrite, FR-013a).
        let a = dir.path().join(ATTACHMENTS_DIR).join("img.png");
        let b = dir.path().join(ATTACHMENTS_DIR).join("img 2.png");
        assert_eq!(std::fs::read(a).unwrap(), b"one");
        assert_eq!(std::fs::read(b).unwrap(), b"two");
    }

    #[test]
    fn store_attachment_untitled_note_is_unsupported() {
        // No note path → unsupported in v1 (M2): an attachment needs a saved note
        // to anchor on. The previous behaviour wrote into the process CWD (which is
        // `/` under the app sandbox); now it returns a clear error and never
        // touches the filesystem or process-global CWD.
        let err = store_attachment(None, b"data", "drop.bin").unwrap_err();
        assert!(
            matches!(err, EmendError::InvalidConfig { .. }),
            "expected InvalidConfig for an unsaved note, got {err:?}"
        );
    }

    #[test]
    fn read_missing_maps_to_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.md");
        assert!(matches!(
            read_tolerant(&path),
            Err(EmendError::NotFound { .. })
        ));
    }

    #[test]
    fn bom_is_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.md");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"x");
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(read_tolerant(&path).unwrap(), "x");
    }
}
