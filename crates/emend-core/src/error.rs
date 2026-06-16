//! Structured error type for the core, surfaced across the FFI boundary as a
//! Swift `Error` (research §B7). Every fallible boundary call returns
//! `Result<_, EmendError>`; no panic may unwind across FFI (NFR-003).
//!
//! NOTE: skeleton uses hand-written `Display`/`Error` to stay dependency-free.
//! `/sdd:implement` migrates this to `thiserror` and adds `#[derive(uniffi::Error)]`
//! in the `emend-ffi` projection.

use std::fmt;

/// Recoverable, UI-renderable error. Variants carry the fields the UI needs
/// (paths, limits) — see contracts/ffi-interface.md.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EmendError {
    NotFound {
        path: String,
    },
    PermissionDenied {
        path: String,
    },
    NameCollision {
        path: String,
    },
    NoteTooLarge {
        path: String,
        bytes: u64,
        limit: u64,
    },
    InvalidConfig {
        detail: String,
    },
    AiNotConfigured,
    AiTimeout,
    AiCancelled,
    AiOversizedInput {
        bytes: u64,
        limit: u64,
    },
    Internal {
        detail: String,
    },
}

impl fmt::Display for EmendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "not found: {path}"),
            Self::PermissionDenied { path } => write!(f, "permission denied: {path}"),
            Self::NameCollision { path } => write!(f, "name already exists: {path}"),
            Self::NoteTooLarge { path, bytes, limit } => {
                write!(f, "note too large: {path} ({bytes} bytes > {limit})")
            }
            Self::InvalidConfig { detail } => write!(f, "invalid configuration: {detail}"),
            Self::AiNotConfigured => write!(f, "AI is not configured"),
            Self::AiTimeout => write!(f, "AI request timed out"),
            Self::AiCancelled => write!(f, "AI request was cancelled"),
            Self::AiOversizedInput { bytes, limit } => {
                write!(f, "AI input too large ({bytes} bytes > {limit})")
            }
            Self::Internal { detail } => write!(f, "internal error: {detail}"),
        }
    }
}

impl std::error::Error for EmendError {}

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
}
