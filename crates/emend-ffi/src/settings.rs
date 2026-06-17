//! T124 (FFI half) — typography settings FFI projection (US7 · FFI contract §8;
//! FR-038/FR-039).
//!
//! Thin UniFFI shim over [`emend_core::settings`]. Like the rest of this crate it
//! holds **no logic** of its own; it only:
//!
//! 1. **Projects the value type** the core cannot derive `uniffi` on
//!    (Constitution V keeps `emend-core` `uniffi`-free): [`TypographySettings`],
//!    a `#[derive(uniffi::Record)]` mirror of
//!    [`emend_core::settings::TypographySettings`] with an **exhaustive** `From`
//!    in each direction (no wildcard arm — a new core field is a compile error
//!    here).
//!
//! 2. **Wraps the in-memory store** in a [`SettingsHandle`]
//!    (`#[derive(uniffi::Object)]`, handed to Swift as `Arc<Self>`) exposing
//!    `get_typography()` / `set_typography()`.
//!
//! ## Why a handle, not free functions (design decision)
//!
//! The contract (§8) sketches `get_typography()` / `set_typography(t)` as **free
//! functions**, but the contract preamble states signatures are *illustrative*
//! and the global typography store needs **per-session state that survives across
//! calls**. A pair of free functions would have to reach a global `static`
//! `Mutex` — exactly the mutable-global pattern this codebase avoids. Instead we
//! follow the established [`WorkspaceHandle`](crate::workspace::WorkspaceHandle)
//! convention: a single `#[uniffi::export]` object owns the
//! [`TypographyStore`](emend_core::settings::TypographyStore) behind its own lock,
//! constructed once per app session via [`new_settings`]. Swift holds the `Arc`,
//! replays its persisted (UserDefaults) value on launch via `set_typography`, and
//! reads back via `get_typography` — matching the US2 app-state pattern (the core
//! has no persistence layer; Swift owns persistence and replays into the core).
//!
//! `set_typography` returns `Result<(), FfiError>` to match the contract's
//! fallible shape, but the only failure is a poisoned lock
//! ([`FfiError::Internal`], unreachable under the no-panic posture): an
//! out-of-range value is **clamped** by the core, never rejected (so a bad value
//! from the boundary can't produce a broken layout — and can't surface as an
//! error either).

use crate::error::FfiError;
use emend_core::settings::{TypographySettings as CoreTypographySettings, TypographyStore};
use std::sync::Arc;

/// Global editor + preview typography preferences (FFI contract §8). The FFI
/// mirror of [`emend_core::settings::TypographySettings`].
///
/// Plain `#[derive(uniffi::Record)]`: all fields are directly-supported scalars.
/// No theme field — v1 follows the system light/dark appearance, handled
/// Swift-side (FR-039). Values are clamped into sane bounds by the core on
/// `set_typography` (size `8..=48` pt, line height `1.0..=3.0`, paragraph
/// spacing `0..=64` pt, a blank font family → the system default), so a value
/// read back via `get_typography` is always in range.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TypographySettings {
    /// Font family name (an AppKit family name / CSS `font-family` token;
    /// `"-apple-system"` is the system font on both sides). Blank → default.
    pub font_family: String,
    /// Font size in points (clamped to `8..=48`).
    pub font_size_pt: f32,
    /// Line height multiplier on the font's natural leading (clamped to
    /// `1.0..=3.0`; `1.0` = single-spacing).
    pub line_height: f32,
    /// Spacing between paragraphs in points (clamped to `0..=64`).
    pub paragraph_spacing_pt: f32,
}

impl From<CoreTypographySettings> for TypographySettings {
    fn from(s: CoreTypographySettings) -> Self {
        // Destructure exhaustively so a new core field forces a compile error
        // here rather than silently dropping data.
        let CoreTypographySettings {
            font_family,
            font_size_pt,
            line_height,
            paragraph_spacing_pt,
        } = s;
        Self {
            font_family,
            font_size_pt,
            line_height,
            paragraph_spacing_pt,
        }
    }
}

impl From<TypographySettings> for CoreTypographySettings {
    fn from(s: TypographySettings) -> Self {
        let TypographySettings {
            font_family,
            font_size_pt,
            line_height,
            paragraph_spacing_pt,
        } = s;
        Self {
            font_family,
            font_size_pt,
            line_height,
            paragraph_spacing_pt,
        }
    }
}

/// Typography settings handle exported to Swift (FFI contract §8).
///
/// Handed to Swift as `Arc<Self>`; methods take `&self`. Owns the core
/// [`TypographyStore`] — one per app session (see the module's "Why a handle"
/// note). The store keeps the settings in memory only; Swift persists them
/// (UserDefaults) and replays on launch via [`Self::set_typography`].
#[derive(Debug, Default, uniffi::Object)]
pub struct SettingsHandle {
    store: TypographyStore,
}

#[uniffi::export]
impl SettingsHandle {
    /// The current typography settings (FFI contract §8 `get_typography`).
    ///
    /// Always in range — `set_typography` clamps on the way in. Infallible:
    /// returns the value directly (the contract's `get_typography()` is not
    /// fallible), and a poisoned lock degrades to the defaults inside the core.
    #[must_use]
    pub fn get_typography(&self) -> TypographySettings {
        self.store.get().into()
    }

    /// Replace the typography settings (FFI contract §8 `set_typography`).
    ///
    /// The value is **clamped** into sane bounds by the core before it is stored,
    /// so an out-of-range size/spacing from the boundary can never produce a
    /// broken layout (and is repaired, not rejected). Returns `Ok(())` on success.
    ///
    /// # Errors
    ///
    /// [`FfiError::Internal`] only if the underlying lock is poisoned (a panic
    /// while held — unreachable under the no-panic posture, NFR-003). There is no
    /// "invalid value" error: bad values are clamped.
    pub fn set_typography(&self, settings: TypographySettings) -> Result<(), FfiError> {
        self.store.set(settings.into());
        Ok(())
    }
}

/// Construct a fresh [`SettingsHandle`] seeded with the sane defaults (FFI
/// contract §8 entry point).
///
/// One per app session; Swift reads/writes typography through the returned
/// handle and replays its persisted value on launch via
/// [`SettingsHandle::set_typography`].
#[uniffi::export]
#[must_use]
pub fn new_settings() -> Arc<SettingsHandle> {
    Arc::new(SettingsHandle::default())
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{new_settings, TypographySettings};

    #[test]
    fn get_returns_defaults_then_round_trips_set() {
        let h = new_settings();

        // A fresh handle reports usable defaults.
        let d = h.get_typography();
        assert!(!d.font_family.is_empty());
        assert!((8.0..=48.0).contains(&d.font_size_pt));

        // An in-range value round-trips verbatim.
        let want = TypographySettings {
            font_family: "SF Mono".to_owned(),
            font_size_pt: 15.0,
            line_height: 1.5,
            paragraph_spacing_pt: 6.0,
        };
        h.set_typography(want.clone()).expect("set");
        assert_eq!(h.get_typography(), want);
    }

    #[test]
    fn set_clamps_out_of_range_values() {
        let h = new_settings();
        h.set_typography(TypographySettings {
            font_family: "   ".to_owned(),
            font_size_pt: 9999.0,
            line_height: 0.0,
            paragraph_spacing_pt: -3.0,
        })
        .expect("set");

        let got = h.get_typography();
        // Blank family fell back, size clamped down, line height up, spacing to 0.
        assert!(!got.font_family.trim().is_empty());
        assert_eq!(got.font_size_pt, 48.0);
        assert_eq!(got.line_height, 1.0);
        assert_eq!(got.paragraph_spacing_pt, 0.0);
    }
}
