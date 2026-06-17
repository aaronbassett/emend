//! Emend FFI shim (UniFFI).
//!
//! Thin projection of `emend-core` to Swift. Keep ALL business logic in the
//! core; this crate holds only `#[uniffi::export]` wrappers, the
//! `#[derive(uniffi::Error)]` projection of `EmendError` ([`error`]), the
//! panic-containment posture ([`panic`]), and the async-infrastructure
//! scaffolding ([`handles`]) — the long-lived `tokio` runtime, the Rust-owned
//! cancellation handle, and the foreign-trait streaming sinks that later AI /
//! search tasks plug into (research §A1, §B7; contract in
//! `contracts/ffi-interface.md`).
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
//! `/sdd:implement` (later tasks) adds the `#[uniffi::export]` functions that
//! match `contracts/ffi-interface.md` and drive the [`handles`] sinks; the
//! sinks, cancellation handle, and runtime accessor themselves live in
//! [`handles`] as of T024.

pub mod error;
pub mod handles;
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

/// Read a text file at `path`, tolerating BOM / CRLF / non-UTF-8 bytes
/// (`emend_core::fs::read_tolerant`).
///
/// This is the foundational read primitive behind the security-scoped-bookmark
/// handshake (research §A4): Swift opens the scope for a user-granted folder and
/// hands Rust the resolved path; a successful read here proves the sandbox
/// extension is process-wide.
///
/// **Prototype only — uncapped.** It does NOT enforce the `Document` note-size
/// cap (`MAX_NOTE_BYTES`) and would allocate an arbitrarily large file into a
/// `String`; its only caller is the dev handshake. The capped, handle-based
/// loader `open_document` (US1) **supersedes** this — it must not become a
/// load-bearing general-purpose read.
#[uniffi::export]
pub fn read_file_at(path: String) -> Result<String, error::FfiError> {
    emend_core::fs::read_tolerant(path).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::{core_abi_version, error::FfiError, read_file_at};

    #[test]
    fn abi_version_is_stable() {
        assert_eq!(core_abi_version(), 1);
    }

    #[test]
    fn read_file_at_roundtrips_through_the_tolerant_reader(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("note.md");
        std::fs::write(&path, "# hi\r\nbody")?;
        let text = read_file_at(path.to_string_lossy().into_owned())?;
        assert_eq!(text, "# hi\r\nbody"); // CRLF preserved by the tolerant reader
        Ok(())
    }

    #[test]
    fn read_file_at_maps_missing_file_to_not_found() {
        assert!(matches!(
            read_file_at("/no/such/emend/file.md".to_owned()),
            Err(FfiError::NotFound { .. })
        ));
    }
}
