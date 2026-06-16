//! Structured error type for the core, surfaced across the FFI boundary as a
//! Swift `Error` (research §B7). Every fallible boundary call returns
//! `Result<_, EmendError>`; no panic may unwind across FFI (NFR-003).
//!
//! The variant set is the single source of truth for the FFI contract's
//! `Error type` (see `contracts/ffi-interface.md`). The `emend-ffi` crate
//! mirrors it 1:1 in a `#[derive(uniffi::Error)]` projection with an
//! **exhaustive** `From<EmendError>` impl (no catch-all arm), so adding a
//! variant here is a compile error there until the projection is updated.
//!
//! ## Why this enum is intentionally NOT `#[non_exhaustive]`
//!
//! The skeleton marked this `#[non_exhaustive]`. We deliberately drop that:
//! `#[non_exhaustive]` forces every *downstream-crate* `match` to carry a
//! wildcard arm, which would defeat the exhaustiveness guarantee we want on the
//! `emend-ffi` boundary (a wildcard would silently swallow a newly added
//! variant instead of failing to compile). Keeping the enum exhaustive makes
//! the FFI projection a closed, compiler-checked mirror. Both crates live in
//! this workspace and version in lockstep, so we gain nothing from
//! `#[non_exhaustive]` and lose the boundary safety net.

/// Recoverable, UI-renderable error. Variants carry the fields the UI needs
/// (paths, limits) — see `contracts/ffi-interface.md`.
///
/// Field types are restricted to UniFFI-compatible primitives (`String`,
/// `u16`, `u64`) so the `emend-ffi` projection lowers cleanly across the
/// boundary without a flattened error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmendError {
    #[error("not found: {path}")]
    NotFound { path: String },

    #[error("permission denied: {path}")]
    PermissionDenied { path: String },

    /// Underlying filesystem failure that isn't a more specific variant
    /// (e.g. a read/write/rename error). `detail` is a human-readable cause.
    #[error("I/O failure: {path}: {detail}")]
    IoFailure { path: String, detail: String },

    #[error("name already exists: {path}")]
    NameCollision { path: String },

    #[error("note too large: {path} ({bytes} bytes > {limit})")]
    NoteTooLarge {
        path: String,
        bytes: u64,
        limit: u64,
    },

    #[error("invalid configuration: {detail}")]
    InvalidConfig { detail: String },

    #[error("AI is not configured")]
    AiNotConfigured,

    #[error("AI request timed out")]
    AiTimeout,

    #[error("AI request was cancelled")]
    AiCancelled,

    #[error("AI input too large ({bytes} bytes > {limit})")]
    AiOversizedInput { bytes: u64, limit: u64 },

    /// Non-2xx HTTP status from the AI endpoint. `detail` carries a redacted,
    /// key-free description (NFR-006).
    #[error("AI HTTP error ({status}): {detail}")]
    AiHttp { status: u16, detail: String },

    /// The AI SSE stream could not be parsed (malformed delta/event framing).
    #[error("AI stream malformed: {detail}")]
    AiStreamMalformed { detail: String },

    /// Captured panic or otherwise-unexpected internal failure (B7). The FFI
    /// boundary maps caught panics to this variant.
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

#[cfg(test)]
mod tests {
    use super::EmendError;

    #[test]
    fn display_includes_path() {
        let e = EmendError::NotFound {
            path: "a/b.md".to_owned(),
        };
        assert!(e.to_string().contains("a/b.md"));
    }

    #[test]
    fn display_messages_preserved() {
        assert_eq!(
            EmendError::PermissionDenied {
                path: "x".to_owned(),
            }
            .to_string(),
            "permission denied: x"
        );
        assert_eq!(
            EmendError::AiNotConfigured.to_string(),
            "AI is not configured"
        );
        assert_eq!(EmendError::AiTimeout.to_string(), "AI request timed out");
        assert_eq!(
            EmendError::AiCancelled.to_string(),
            "AI request was cancelled"
        );
        assert_eq!(
            EmendError::NoteTooLarge {
                path: "n.md".to_owned(),
                bytes: 10,
                limit: 5,
            }
            .to_string(),
            "note too large: n.md (10 bytes > 5)"
        );
    }

    #[test]
    fn new_variants_render() {
        assert_eq!(
            EmendError::IoFailure {
                path: "p".to_owned(),
                detail: "disk full".to_owned(),
            }
            .to_string(),
            "I/O failure: p: disk full"
        );
        assert_eq!(
            EmendError::AiHttp {
                status: 503,
                detail: "service unavailable".to_owned(),
            }
            .to_string(),
            "AI HTTP error (503): service unavailable"
        );
        assert_eq!(
            EmendError::AiStreamMalformed {
                detail: "bad event".to_owned(),
            }
            .to_string(),
            "AI stream malformed: bad event"
        );
    }

    #[test]
    fn is_std_error() {
        // Compile-time assertion that thiserror still gives us std::error::Error.
        fn assert_error<E: std::error::Error>() {}
        assert_error::<EmendError>();
    }
}
