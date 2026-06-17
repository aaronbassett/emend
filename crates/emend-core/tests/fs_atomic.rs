//! T021 — failing-first integration tests for the atomic+durable write and
//! tolerant read path (`emend_core::fs`).
//!
//! These tests encode **Constitution Principle III (never lose user data)** as
//! executable obligations:
//!
//! - **FR-009a (atomic + durable writes):** an external reader (file watcher,
//!   `git`, an agent) must *never* observe a partially written note. We prove
//!   this two ways:
//!     1. *Crash-equivalence:* simulate a process death between writing the temp
//!        file and the rename. Because `write_atomic` writes to a sibling temp
//!        file and only `rename(2)`s it into place at the very end, a crash
//!        before the rename leaves the original target byte-for-byte intact
//!        (and never a zero-length / truncated file). We reproduce the temp +
//!        rename sequence directly and assert the target is *only ever*
//!        fully-old or fully-new.
//!     2. *Concurrent observation:* a reader hammering the target while a writer
//!        repeatedly rewrites it only ever sees one of the complete, expected
//!        contents — never a prefix, never empty, never a mix.
//!
//! - **FR-003a (tolerant reads):** files written by other tools — UTF-8 with a
//!   BOM, CRLF line endings, and outright invalid (non-UTF-8) bytes — must read
//!   successfully (return usable text) instead of erroring or crashing.
//!
//! - **Edge cases:** overwrite-in-place, parent-relative target creation, and a
//!   missing file mapping to [`EmendError::NotFound`].
//!
//! Round-trip / decoding contract these tests pin down (see `fs.rs` docs for the
//! authoritative statement):
//! * A single leading UTF-8 BOM (`U+FEFF`, bytes `EF BB BF`) is **stripped** on
//!   read. A BOM in the interior of the file is preserved.
//! * Line endings are **preserved verbatim** — CRLF stays CRLF, LF stays LF.
//!   Normalization (if any) is a higher layer's concern, not the byte gateway's.
//! * Invalid UTF-8 is decoded **lossily** (`U+FFFD` replacement) so the read
//!   always succeeds; bytes that *are* valid UTF-8 round-trip exactly.

// Tests assert on known-good values; the workspace denies these in library code
// but a test that can't unwrap its own fixtures isn't a test. Scoped here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::fs::{read_tolerant, write_atomic};
use emend_core::EmendError;
use std::fs;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Atomicity / durability (FR-009a)
// ---------------------------------------------------------------------------

/// A crash *before* the rename must leave the original target fully intact —
/// never truncated, never zero-length, never a half-written mix.
///
/// We model the crash by performing only the first half of the atomic write
/// (create + populate a sibling temp file) and then *not* renaming, exactly as
/// if the process died at that instant. The target must still hold the original
/// bytes verbatim.
#[test]
fn crash_before_rename_leaves_original_intact() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("note.md");

    let original = "# Original\n\nfully durable content\n";
    write_atomic(&target, original).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), original);

    // Simulate the write being interrupted between "temp written" and "rename":
    // a sibling temp file is created and filled, but the rename never happens.
    let mut tmp = tempfile::Builder::new()
        .prefix(".note.md.")
        .tempfile_in(dir.path())
        .unwrap();
    tmp.write_all(b"# BRAND NEW\n\nthis must never be observed at the target\n")
        .unwrap();
    tmp.flush().unwrap();
    // Drop without persisting == crash before rename.
    drop(tmp);

    // The target is byte-for-byte the original — no partial state leaked.
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        original,
        "target must be the untouched original after a crash before rename"
    );
}

/// The whole point of the temp+rename dance: at no instant during a write is the
/// target observable as anything other than *complete old* or *complete new*.
///
/// A reader thread spins reading the target while a writer thread rewrites it
/// many times. Every read it sees must be one of the two known-complete
/// contents — never empty, never a prefix, never garbage. A non-atomic
/// implementation (e.g. truncate-then-write in place) would let the reader catch
/// a zero-length or partial file and fail this test.
#[test]
fn reader_never_observes_a_partial_file() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("hammered.md");

    let old = "OLD".repeat(50_000); // large enough that a partial write is catchable
    let new = "NEW".repeat(50_000);

    write_atomic(&target, &old).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reader_target = target.clone();
    let reader_old = old.clone();
    let reader_new = new.clone();
    let reader_stop = Arc::clone(&stop);

    let reader = thread::spawn(move || {
        let mut saw_old = false;
        let mut saw_new = false;
        while !reader_stop.load(Ordering::Relaxed) {
            // A raw std read of the target — exactly what an external tool does.
            // A read error (e.g. a transient ENOENT) is tolerated: `rename` is
            // atomic, so the path should never vanish for an in-place overwrite,
            // but we stay lenient and simply retry on the next loop iteration.
            if let Ok(s) = fs::read_to_string(&reader_target) {
                assert!(
                    s == reader_old || s == reader_new,
                    "reader observed a partial/unexpected file of len {}",
                    s.len()
                );
                saw_old |= s == reader_old;
                saw_new |= s == reader_new;
            }
        }
        (saw_old, saw_new)
    });

    // Writer: flip-flop the contents many times via the atomic writer.
    for i in 0..200 {
        let contents = if i % 2 == 0 { &new } else { &old };
        write_atomic(&target, contents).unwrap();
    }
    stop.store(true, Ordering::Relaxed);

    let (saw_old, saw_new) = reader.join().unwrap();
    // We don't *require* the reader to have caught both states (timing), but if
    // it caught anything it must have been one of the two complete contents,
    // which the in-loop assert already guaranteed.
    assert!(
        saw_old || saw_new,
        "reader should have observed at least one complete state"
    );

    // Final state is deterministic: last write (i = 199, odd) wrote `old`.
    assert_eq!(read_tolerant(&target).unwrap(), old);
}

/// `write_atomic` must leave no stray temp files behind in the target's
/// directory after a successful write — only the target itself.
#[test]
fn successful_write_leaves_no_temp_files() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("clean.md");

    write_atomic(&target, "content\n").unwrap();
    write_atomic(&target, "updated\n").unwrap();

    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("clean.md")],
        "only the target should remain; got {entries:?}"
    );
}

// ---------------------------------------------------------------------------
// Tolerant reads (FR-003a)
// ---------------------------------------------------------------------------

/// A leading UTF-8 BOM is stripped; the remaining text round-trips exactly.
#[test]
fn read_strips_leading_utf8_bom() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bom.md");

    let mut bytes = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
    bytes.extend_from_slice("# Heading\n\nbody\n".as_bytes());
    fs::write(&path, &bytes).unwrap();

    let text = read_tolerant(&path).unwrap();
    assert_eq!(text, "# Heading\n\nbody\n");
    assert!(
        !text.starts_with('\u{feff}'),
        "the BOM code point must be gone from the decoded text"
    );
}

/// A BOM-less UTF-8 file is unchanged (no false-positive stripping of a real
/// leading character).
#[test]
fn read_without_bom_is_unchanged() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("plain.md");
    let content = "no bom here\n";
    fs::write(&path, content).unwrap();
    assert_eq!(read_tolerant(&path).unwrap(), content);
}

/// CRLF line endings are preserved verbatim — the byte gateway does not
/// normalize newlines.
#[test]
fn read_preserves_crlf_line_endings() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("crlf.md");
    let content = "line one\r\nline two\r\nline three\r\n";
    fs::write(&path, content).unwrap();

    let text = read_tolerant(&path).unwrap();
    assert_eq!(text, content, "CRLF must survive the read unchanged");
    assert_eq!(text.matches("\r\n").count(), 3);
}

/// LF line endings are likewise preserved (control for the CRLF case).
#[test]
fn read_preserves_lf_line_endings() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("lf.md");
    let content = "a\nb\nc\n";
    fs::write(&path, content).unwrap();
    assert_eq!(read_tolerant(&path).unwrap(), content);
}

/// A BOM followed immediately by CRLF: BOM stripped, CRLF preserved.
#[test]
fn read_strips_bom_and_keeps_crlf_together() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bom_crlf.md");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"first\r\nsecond\r\n");
    fs::write(&path, &bytes).unwrap();

    let text = read_tolerant(&path).unwrap();
    assert_eq!(text, "first\r\nsecond\r\n");
}

/// Invalid (non-UTF-8) bytes do NOT error — the read succeeds with lossy
/// decoding (U+FFFD replacement). Valid surrounding text is preserved exactly.
#[test]
fn read_invalid_utf8_succeeds_lossily() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("latin1.md");

    // 0xFF is never valid in UTF-8; sandwich it between valid ASCII.
    let bytes = [b'h', b'e', b'l', b'l', b'o', 0xFF, b'!', b'\n'];
    fs::write(&path, bytes).unwrap();

    let text = read_tolerant(&path).unwrap();
    assert!(
        text.contains('\u{fffd}'),
        "invalid byte should become the U+FFFD replacement char, got {text:?}"
    );
    assert!(text.starts_with("hello"));
    assert!(text.ends_with("!\n"));
}

/// A file that is *only* invalid bytes still reads (does not error / panic) and
/// yields replacement characters rather than an empty/failed result.
#[test]
fn read_all_invalid_bytes_does_not_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("binary.md");
    fs::write(&path, [0x80, 0x81, 0xFE, 0xFF]).unwrap();

    let text = read_tolerant(&path).unwrap();
    assert!(!text.is_empty());
    assert!(text.chars().all(|c| c == '\u{fffd}'));
}

/// Round-trip through the public API: write text, read it back unchanged
/// (no BOM was added, LF preserved).
#[test]
fn write_then_read_round_trips() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("round.md");
    let content = "# Title\n\n- one\n- two\n";
    write_atomic(&path, content).unwrap();
    assert_eq!(read_tolerant(&path).unwrap(), content);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// Overwriting an existing file replaces its contents wholesale (atomic
/// in-place replace).
#[test]
fn overwrite_existing_file_replaces_contents() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("over.md");

    write_atomic(&path, "first version").unwrap();
    write_atomic(&path, "second version").unwrap();

    assert_eq!(read_tolerant(&path).unwrap(), "second version");
}

/// Writing to a target in a subdirectory works when that directory already
/// exists (the writer puts its temp file in the *same* directory as the target,
/// so the rename stays on one filesystem).
#[test]
fn write_into_existing_subdirectory() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("nested");
    fs::create_dir(&sub).unwrap();
    let path = sub.join("deep.md");

    write_atomic(&path, "deep content\n").unwrap();
    assert_eq!(read_tolerant(&path).unwrap(), "deep content\n");
}

/// Reading a path that does not exist maps to `EmendError::NotFound` carrying
/// the path — not a generic IO failure, and never a panic.
#[test]
fn read_missing_file_is_not_found() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("does-not-exist.md");

    let err = read_tolerant(&path).unwrap_err();
    match err {
        EmendError::NotFound { path: p } => {
            assert!(
                p.contains("does-not-exist.md"),
                "path should be reported: {p}"
            );
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

/// Empty content writes and reads back as empty (no spurious BOM / newline).
#[test]
fn empty_content_round_trips() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("empty.md");
    write_atomic(&path, "").unwrap();
    assert_eq!(read_tolerant(&path).unwrap(), "");
}
