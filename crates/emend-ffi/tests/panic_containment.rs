//! T015 — proof of the no-panic-across-FFI posture (NFR-003 / research §B7,
//! contract test obligation #2).
//!
//! A forced `panic!` routed through the containment path
//! ([`emend_ffi::panic::contain_panic`]) must surface as a normal
//! [`EmendError`] (and its projected [`FfiError::Internal`]) **without**
//! aborting the process — i.e. this test keeps running and asserts the mapped
//! error rather than dying.
//!
//! This covers the gap UniFFI's own scaffolding does not: code that runs on a
//! detached `tokio::spawn`ed task (later AI/search phases). UniFFI's per-export
//! `catch_unwind` is exercised by the generated scaffolding itself and is not
//! re-tested here.
//!
//! The default panic hook is replaced with a no-op for the duration of the
//! forced panics so the test output stays clean and deterministic (no stderr
//! backtrace noise), then restored.

// This test deliberately triggers panics and asserts on the recovered error;
// the workspace denies `clippy::panic`/`expect_used`. Scoped to this test file.
#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "test intentionally forces panics and asserts on the contained error"
)]

use emend_core::EmendError;
use emend_ffi::error::FfiError;
use emend_ffi::panic::contain_panic;
use std::panic;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Serialize the panic-hook swap across tests in this binary: the panic hook is
/// process-global, so concurrently running tests must not stomp on each other.
fn hook_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let mutex = LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(g) => g,
        // A prior test panicked while holding the guard *outside* the silenced
        // section; the data is `()` so the poisoned state is harmless to reuse.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Run `f` with the panic hook silenced and restored afterwards.
fn with_silent_panic_hook<T>(f: impl FnOnce() -> T) -> T {
    let _guard = hook_guard();
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_info| { /* swallow backtrace noise */ }));
    let out = f();
    panic::set_hook(previous);
    out
}

#[test]
fn forced_panic_surfaces_as_internal_error_and_process_survives() {
    let caught: Result<(), EmendError> =
        with_silent_panic_hook(|| contain_panic(|| panic!("simulated task panic")));

    // 1) The panic became a recoverable error, not an unwind/abort.
    assert!(
        matches!(&caught, Err(EmendError::Internal { detail }) if detail.contains("simulated task panic")),
        "expected Internal carrying the panic message, got {caught:?}"
    );

    // 2) It projects across the FFI boundary as FfiError::Internal.
    let projected: FfiError = caught.expect_err("must be an error").into();
    assert!(
        matches!(&projected, FfiError::Internal { detail } if detail.contains("simulated task panic")),
        "expected FfiError::Internal, got {projected:?}"
    );

    // 3) Reaching this line at all proves the process survived the panic.
    let still_alive = contain_panic(|| 1 + 1);
    assert_eq!(
        still_alive,
        Ok(2),
        "containment must not poison later calls"
    );
}

#[test]
fn success_path_is_transparent() {
    // The happy path returns the value unchanged and never touches the hook.
    assert_eq!(contain_panic(|| "ok"), Ok("ok"));
}

#[test]
fn non_string_panic_payload_is_still_contained() {
    // A panic with a non-string payload must not escape; it maps to Internal
    // with a generic detail. `panic_any` is how you raise a non-string payload
    // (the `panic!` macro requires a format string in edition 2021).
    let caught: Result<(), EmendError> =
        with_silent_panic_hook(|| contain_panic(|| panic::panic_any(42_u32)));
    assert!(
        matches!(caught, Err(EmendError::Internal { .. })),
        "non-string panic payload must still be contained as Internal"
    );
}
