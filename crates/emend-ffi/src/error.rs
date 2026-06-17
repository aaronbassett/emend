//! FFI projection of [`emend_core::EmendError`].
//!
//! `emend-core` must not depend on `uniffi` (Constitution V), so the core error
//! type cannot itself derive `uniffi::Error`. This module is the thin boundary
//! mirror: a `#[derive(uniffi::Error)]` enum with the *same* variants and an
//! **exhaustive** `From<emend_core::EmendError>` impl (no catch-all arm).
//!
//! Because the core enum is intentionally not `#[non_exhaustive]` (see
//! `emend_core::error`), adding a variant there breaks this match at compile
//! time until it is mirrored here — the boundary stays a closed, checked
//! projection. Variant names and field shapes match `contracts/ffi-interface.md`
//! exactly so the generated Swift `enum` reads as specified.
//!
//! All field types are UniFFI-compatible primitives (`String`, `u16`, `u64`),
//! so this is a rich (non-flat) error: Swift receives the associated values.

use emend_core::EmendError;

/// Swift-facing error. Mirrors [`emend_core::EmendError`] 1:1.
///
/// `thiserror::Error` gives us `Display`, which UniFFI uses for the error's
/// description on the Swift side; the messages are forwarded from the core
/// type's own `Display` so there is a single source of truth for wording.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("not found: {path}")]
    NotFound { path: String },

    #[error("permission denied: {path}")]
    PermissionDenied { path: String },

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

    #[error("AI HTTP error ({status}): {detail}")]
    AiHttp { status: u16, detail: String },

    #[error("AI stream malformed: {detail}")]
    AiStreamMalformed { detail: String },

    #[error("internal error: {detail}")]
    Internal { detail: String },
}

impl From<EmendError> for FfiError {
    /// Exhaustive projection — no wildcard arm. If `emend_core::EmendError`
    /// gains a variant, this match fails to compile until it is added here,
    /// guaranteeing the FFI surface stays in sync with the core (and the
    /// contract).
    fn from(err: EmendError) -> Self {
        match err {
            EmendError::NotFound { path } => Self::NotFound { path },
            EmendError::PermissionDenied { path } => Self::PermissionDenied { path },
            EmendError::IoFailure { path, detail } => Self::IoFailure { path, detail },
            EmendError::NameCollision { path } => Self::NameCollision { path },
            EmendError::NoteTooLarge { path, bytes, limit } => {
                Self::NoteTooLarge { path, bytes, limit }
            }
            EmendError::InvalidConfig { detail } => Self::InvalidConfig { detail },
            EmendError::AiNotConfigured => Self::AiNotConfigured,
            EmendError::AiTimeout => Self::AiTimeout,
            EmendError::AiCancelled => Self::AiCancelled,
            EmendError::AiOversizedInput { bytes, limit } => {
                Self::AiOversizedInput { bytes, limit }
            }
            EmendError::AiHttp { status, detail } => Self::AiHttp { status, detail },
            EmendError::AiStreamMalformed { detail } => Self::AiStreamMalformed { detail },
            EmendError::Internal { detail } => Self::Internal { detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FfiError;
    use emend_core::EmendError;

    #[test]
    fn projects_struct_variant_fields() {
        let core = EmendError::NoteTooLarge {
            path: "big.md".to_owned(),
            bytes: 100,
            limit: 50,
        };
        let ffi: FfiError = core.into();
        assert_eq!(
            ffi,
            FfiError::NoteTooLarge {
                path: "big.md".to_owned(),
                bytes: 100,
                limit: 50,
            }
        );
    }

    #[test]
    fn projects_new_contract_variants() {
        assert_eq!(
            FfiError::from(EmendError::AiHttp {
                status: 401,
                detail: "unauthorized".to_owned(),
            }),
            FfiError::AiHttp {
                status: 401,
                detail: "unauthorized".to_owned(),
            }
        );
        assert_eq!(
            FfiError::from(EmendError::IoFailure {
                path: "p".to_owned(),
                detail: "d".to_owned(),
            }),
            FfiError::IoFailure {
                path: "p".to_owned(),
                detail: "d".to_owned(),
            }
        );
    }

    #[test]
    fn projects_fieldless_variant() {
        assert_eq!(
            FfiError::from(EmendError::AiCancelled),
            FfiError::AiCancelled
        );
    }

    #[test]
    fn display_is_forwarded() {
        // The projected error preserves the core's wording.
        assert_eq!(
            FfiError::from(EmendError::NotFound {
                path: "a/b.md".to_owned(),
            })
            .to_string(),
            "not found: a/b.md"
        );
    }
}
