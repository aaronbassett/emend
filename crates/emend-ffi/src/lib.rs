//! Emend FFI shim (UniFFI).
//!
//! Thin projection of `emend-core` to Swift. Keep ALL business logic in the
//! core; this crate holds only `#[uniffi::export]` wrappers, the
//! `#[derive(uniffi::Error)]` projection of `EmendError` ([`error`]), the
//! panic-containment posture ([`panic`]), and — in later phases — the
//! handle/callback types for cancellation and streaming (research §A1, §B7;
//! contract in `contracts/ffi-interface.md`).
//!
//! ## UniFFI 0.31 wiring (pure proc-macro mode)
//!
//! [`uniffi::setup_scaffolding!`] emits the per-crate FFI initialization and
//! the `UNIFFI_META_*` metadata that `uniffi-bindgen-swift` reads from the
//! compiled `libemend_ffi.a` to generate the Swift bindings — no UDL file and
//! no `build.rs` are involved. The macro takes an optional namespace string;
//! omitted, it defaults to this crate's module path (`emend_ffi`), which
//! becomes the Swift module namespace. We keep the default so the namespace
//! tracks the crate name.
//!
//! `/sdd:implement` (later tasks) adds:
//!   - `#[uniffi::export]` functions matching `contracts/ffi-interface.md`
//!   - foreign-trait sinks (`SearchSink`, `AiSink`, `DocObserver`)
//!   - Rust-owned cancellation handles for async AI/search work.

pub mod error;
pub mod panic;

uniffi::setup_scaffolding!();

/// Build/version probe across the FFI boundary.
///
/// Infallible and synchronous: UniFFI's scaffolding still wraps the call in
/// `catch_unwind`, so even a (hypothetical) panic here cannot unwind into Swift
/// (see [`panic`]).
#[uniffi::export]
#[must_use]
pub fn core_abi_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    #[test]
    fn abi_version_is_stable() {
        assert_eq!(super::core_abi_version(), 1);
    }
}
