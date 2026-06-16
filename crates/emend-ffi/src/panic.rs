//! Panic-containment posture for the FFI boundary (NFR-003 / research §B7).
//!
//! # What UniFFI 0.31 already contains (do NOT re-wrap these)
//!
//! Every `#[uniffi::export]` call — sync or async, infallible (`-> T`) or
//! fallible (`-> Result<T, E>`) — is routed through UniFFI's generated
//! scaffolding into `uniffi_core::ffi::rustcalls::rust_call_with_out_status`,
//! which wraps the user code in `std::panic::catch_unwind`. On an unwinding
//! panic the scaffolding sets the C-ABI `RustCallStatus.code` to
//! `RustCallStatusCode::UnexpectedError` (value `2`) and lowers the panic
//! message (extracted via `downcast_ref::<&'static str>()` / `String`, else
//! `"Unknown panic!"`) into the status error buffer. Swift then throws an
//! `UnexpectedError`/`InternalError`. For async exports the same protection is
//! applied around every `Future::poll` step inside UniFFI's `WrappedFuture`.
//!
//! (Verified against `mozilla/uniffi-rs` v0.31.1:
//! `uniffi_core/src/ffi/rustcalls.rs` and
//! `uniffi_core/src/ffi/rustfuture/future.rs`.)
//!
//! Practical consequence: a plain `#[uniffi::export]` function — even one that
//! panics — will NOT abort the process or unwind into Swift. We therefore do
//! **not** add a redundant `catch_unwind` inside ordinary exports.
//!
//! # The gap UniFFI does NOT cover: detached `tokio::spawn` tasks
//!
//! UniFFI's `catch_unwind` only wraps the future that UniFFI itself polls. A
//! task handed to `tokio::spawn` runs independently on the runtime's worker
//! pool; if it panics, Tokio aborts that one task but the panic never
//! propagates back through UniFFI's scaffolding, so it cannot be surfaced as an
//! `EmendError`. Worse, with `panic = "abort"` (or via a custom panic hook) an
//! escaping panic could take the whole process down — violating NFR-003.
//!
//! Later phases (AI streaming, Quick Open search — research §B5/§B2) will run
//! cancellable work on `tokio::spawn`ed tasks behind Rust-owned handles. To keep
//! those panic-safe, run the spawned body through [`contain_panic`], which maps
//! an unwinding panic to [`EmendError::Internal`] so the boundary can deliver it
//! as a normal terminal error (e.g. `AiSink::on_error`).
//!
//! No Tokio tasks exist yet, so this module is the helper + documentation,
//! staged for those phases.

use emend_core::EmendError;
use std::panic::{self, AssertUnwindSafe};

/// Run `f` under `catch_unwind`, mapping an unwinding panic to
/// [`EmendError::Internal`].
///
/// Use this to wrap the body of work that runs **outside** UniFFI's own
/// `catch_unwind` — chiefly the closure/async body handed to `tokio::spawn`
/// (and `spawn_blocking`). Code that runs directly inside a `#[uniffi::export]`
/// function does not need it: UniFFI already contains those panics.
///
/// The recovered message is best-effort (`&'static str` or `String` payloads;
/// otherwise a generic note) and carries no sensitive data of its own — callers
/// must still avoid panicking with secrets (e.g. an API key) in the message.
///
/// # Examples
///
/// ```
/// # use emend_ffi::panic::contain_panic;
/// # use emend_core::EmendError;
/// let ok = contain_panic(|| 2 + 2);
/// assert_eq!(ok, Ok(4));
///
/// let caught = contain_panic(|| -> i32 { panic!("boom") });
/// assert!(matches!(caught, Err(EmendError::Internal { .. })));
/// ```
pub fn contain_panic<T, F>(f: F) -> Result<T, EmendError>
where
    F: FnOnce() -> T,
{
    // `AssertUnwindSafe`: the closure is consumed exactly once and we never
    // observe `f`'s captured state again after a panic, so there is no broken
    // invariant to leak past the boundary (same reasoning UniFFI uses for its
    // own future polling).
    // `payload.as_ref()` deref-coerces the `Box<dyn Any + Send>` to a
    // `&(dyn Any + Send)` referring to the *boxed* value. Passing `&payload`
    // instead would reflect the `Box` itself, whose type matches neither
    // `&'static str` nor `String`, so every payload would look non-string.
    panic::catch_unwind(AssertUnwindSafe(f)).map_err(|payload| EmendError::Internal {
        detail: panic_message(payload.as_ref()),
    })
}

/// Extract a human-readable message from a panic payload, mirroring how
/// UniFFI's own scaffolding recovers the panic string.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic with non-string payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    // The workspace denies `clippy::panic`/`unwrap_used`; these tests
    // deliberately force panics through the containment path, and assert with
    // `matches!` rather than `unwrap`. The narrow allow keeps the rest of the
    // crate's no-panic posture intact while letting the test express its intent.
    #![allow(
        clippy::panic,
        reason = "tests intentionally force a panic to verify containment"
    )]

    use super::contain_panic;
    use emend_core::EmendError;

    #[test]
    fn passes_value_through_on_success() {
        assert_eq!(contain_panic(|| 41 + 1), Ok(42));
    }

    #[test]
    fn maps_str_panic_to_internal() {
        let caught = contain_panic(|| -> () { panic!("kaboom") });
        assert!(
            matches!(&caught, Err(EmendError::Internal { detail }) if detail.contains("kaboom")),
            "expected Internal carrying the panic message, got {caught:?}"
        );
    }

    #[test]
    fn maps_string_panic_to_internal() {
        let msg = "owned message".to_owned();
        let caught = contain_panic(move || -> () { panic!("{msg}") });
        assert!(
            matches!(&caught, Err(EmendError::Internal { detail }) if detail.contains("owned message")),
            "expected Internal carrying the panic message, got {caught:?}"
        );
    }
}
