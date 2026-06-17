//! T110 — AI **privacy / secret-hygiene** invariants enforced in the pure core
//! (US6 · FR-035/036a, NFR-006, SC-008).
//!
//! Two structural guarantees that live in `emend-core` (so they hold regardless
//! of the FFI orchestration, and are testable with plain `cargo test`):
//!
//! 1. **Max input size is rejected BEFORE any send** (FR-036a). The core exposes
//!    a pure [`emend_core::ai::check_input_size`] guard that the FFI layer calls
//!    *before* constructing/dispatching a request; an oversized document yields
//!    [`EmendError::AiOversizedInput`] with no network involvement. Network
//!    gating is structural — `emend-core` has NO network/reqwest/tokio
//!    dependency (CI proves `cargo tree -p emend-core -i reqwest` finds nothing),
//!    so this asserts the *refusal path* the boundary relies on.
//!
//! 2. **The API key never leaks via `Debug`/`Display`** (NFR-006). The key is
//!    held in a redacting newtype [`emend_core::ai::ApiKey`] whose `Debug` AND
//!    `Display` render `***` — never the secret — so it cannot accidentally land
//!    in a log line, a panic message, or a tracing field.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::ai::{check_input_size, ApiKey};
use emend_core::EmendError;

// -- Max input size rejected before any send (FR-036a) ----------------------

#[test]
fn oversized_input_is_rejected_locally() {
    // One byte over the limit must be refused with AiOversizedInput, carrying the
    // actual size and the limit so the UI can message it (FR-036a).
    let limit: u64 = 1024;
    let oversized = "x".repeat(usize::try_from(limit).unwrap() + 1);
    let err = check_input_size(&oversized, limit)
        .expect_err("input over the limit must be rejected before any send");
    match err {
        EmendError::AiOversizedInput { bytes, limit: l } => {
            assert_eq!(bytes, limit + 1, "reports the actual byte size");
            assert_eq!(l, limit, "reports the configured limit");
        }
        other => panic!("expected AiOversizedInput, got {other:?}"),
    }
}

#[test]
fn input_at_the_limit_is_accepted() {
    let limit: u64 = 1024;
    let exactly = "x".repeat(usize::try_from(limit).unwrap());
    assert!(
        check_input_size(&exactly, limit).is_ok(),
        "input exactly at the limit is allowed (boundary is inclusive)"
    );
}

#[test]
fn input_under_the_limit_is_accepted() {
    assert!(check_input_size("small doc", 1024).is_ok());
}

#[test]
fn size_check_measures_utf8_bytes_not_chars() {
    // A multibyte string can be under the char count but over the byte limit; the
    // guard must measure BYTES (what actually crosses the wire), not chars.
    let s = "😀".repeat(10); // 10 chars, 40 UTF-8 bytes
    assert_eq!(s.chars().count(), 10);
    assert_eq!(s.len(), 40);
    // Under a 40-byte limit (inclusive) it is accepted...
    assert!(check_input_size(&s, 40).is_ok());
    // ...but a 39-byte limit rejects it even though it is only 10 chars.
    let err = check_input_size(&s, 39).expect_err("byte count exceeds 39");
    assert!(matches!(
        err,
        EmendError::AiOversizedInput {
            bytes: 40,
            limit: 39
        }
    ));
}

// -- API key redaction (NFR-006) --------------------------------------------

/// A realistic-looking secret used across the redaction assertions.
const SECRET: &str = "sk-emend-SUPER-secret-key-0123456789";

#[test]
fn api_key_debug_never_contains_the_secret() {
    let key = ApiKey::new(SECRET.to_owned());
    let debug = format!("{key:?}");
    assert!(
        !debug.contains(SECRET),
        "Debug must not contain the secret substring: {debug:?}"
    );
    assert!(
        !debug.contains("0123456789"),
        "not even a fragment of the secret may appear: {debug:?}"
    );
    assert!(
        debug.contains("***"),
        "Debug should render the redaction marker: {debug:?}"
    );
}

#[test]
fn api_key_display_never_contains_the_secret() {
    let key = ApiKey::new(SECRET.to_owned());
    let shown = format!("{key}");
    assert!(
        !shown.contains(SECRET),
        "Display must not contain the secret substring: {shown:?}"
    );
    assert_eq!(shown, "***", "Display renders only the redaction marker");
}

#[test]
fn api_key_redacts_inside_a_larger_formatted_message() {
    // The realistic leak vector: the key embedded in a log/error line. Neither a
    // `{}` nor a `{:?}` interpolation may surface it.
    let key = ApiKey::new(SECRET.to_owned());
    let log_line = format!("dispatching request with key={key} (debug={key:?})");
    assert!(
        !log_line.contains(SECRET),
        "an interpolated key must stay redacted in any message: {log_line}"
    );
}

#[test]
fn api_key_exposes_the_secret_only_through_an_explicit_accessor() {
    // The transport layer needs the real bytes for the Authorization header; that
    // access must be EXPLICIT (a named method), never an accidental Display/Debug.
    let key = ApiKey::new(SECRET.to_owned());
    assert_eq!(
        key.expose(),
        SECRET,
        "the explicit accessor returns the real secret for the auth header"
    );
}

#[test]
fn api_key_from_blank_string_is_treated_as_unset() {
    // A blank/whitespace key is effectively "no key" — useful so the FFI layer
    // can map an empty key to AiNotConfigured rather than sending an empty bearer.
    assert!(ApiKey::new("".to_owned()).is_blank());
    assert!(ApiKey::new("   ".to_owned()).is_blank());
    assert!(!ApiKey::new(SECRET.to_owned()).is_blank());
}
