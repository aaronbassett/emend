//! Emend FFI shim (UniFFI).
//!
//! Thin projection of `emend-core` to Swift. Keep ALL business logic in the core;
//! this crate holds only `#[uniffi::export]` wrappers, the `#[derive(uniffi::Error)]`
//! projection of `EmendError`, and the handle/callback types for cancellation and
//! streaming (research §A1, §B7; contract in contracts/ffi-interface.md).
//!
//! `/sdd:implement` adds:
//!   - `uniffi::setup_scaffolding!();`
//!   - `#[uniffi::export]` functions matching contracts/ffi-interface.md
//!   - foreign-trait sinks (`SearchSink`, `AiSink`, `DocObserver`)
//!   - per-export `catch_unwind` posture so no panic crosses the boundary.

/// Build/version probe — replaced by the generated UniFFI surface during implementation.
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
