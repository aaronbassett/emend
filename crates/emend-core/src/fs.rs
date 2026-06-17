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
use std::path::Path;

/// One leading UTF-8 byte-order mark (`U+FEFF`), as it appears on disk.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

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

    // Write + flush the user bytes into the temp file.
    temp.write_all(contents.as_bytes())
        .map_err(|e| map_io(path, &e))?;
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

    use super::{read_tolerant, write_atomic};
    use crate::EmendError;

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        write_atomic(&path, "hello\n").unwrap();
        assert_eq!(read_tolerant(&path).unwrap(), "hello\n");
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
