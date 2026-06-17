//! Global typography settings (US7 · FR-038/FR-039; FFI contract §8).
//!
//! [`TypographySettings`] is the app-managed, global editor + preview typography
//! preference (data-model "TypographySettings"): a font family, a font size in
//! points, a line height (a multiplier on the font's natural line height), and a
//! paragraph spacing in points. It applies to both the `NSTextView` editor and
//! the `WKWebView` preview.
//!
//! ## No theme field — v1 follows the system (FR-039)
//!
//! There is **deliberately no** light/dark theme field here. v1 follows the
//! system appearance automatically, which is a Swift/AppKit concern
//! (`NSApp.effectiveAppearance`), so the core never models it.
//!
//! ## No persistence layer — the core holds it in memory (US2 guardrail)
//!
//! The core has **no persistence layer**: like favorites/pins/icons in
//! [`crate::workspace`], typography lives Swift-side as the source of truth
//! (UserDefaults) and is *replayed* into the core on launch via [`TypographyStore::set`].
//! [`TypographyStore`] is therefore a tiny in-memory cell with `get`/`set`, made
//! thread-safe with a [`Mutex`] exactly like the rest of the core's shared state
//! (the FFI shim hands it to Swift behind an `Arc`). "Persist + round-trip" for
//! this module means the value you `set` is the value you `get` back — no disk.
//!
//! ## Clamping is the single validation gate (no broken layouts)
//!
//! A size or spacing arriving from the boundary is just a number; a hostile or
//! buggy caller could send `0`, `9999`, a negative spacing, a sub-1.0 line
//! height, or even `NaN`/`±∞`. Any of those would produce a broken or invisible
//! layout. So **every value entering the store is clamped** ([`TypographySettings::clamped`]):
//! finite, in-range numbers pass through unchanged; out-of-range numbers are
//! pinned to the nearest bound; non-finite numbers and a blank font family fall
//! back to the per-field default. Clamping is idempotent, and the store applies
//! it on `set`, so [`TypographyStore::get`] can never return an unclamped value
//! (the FFI `set_typography` needs no error path for a bad value — it clamps).
//!
//! This module is pure `std` (`Mutex` only); it holds **no `uniffi` and no
//! `tokio`** types (Constitution V), so it is unit/integration-testable with
//! plain `cargo test`. The value type is shaped to project cleanly onto the FFI
//! contract's §8 `TypographySettings` later, without importing any FFI machinery.

use std::sync::Mutex;

/// Minimum font size in points. Below this, body text is effectively unreadable;
/// a `0` or negative size from the boundary clamps up to here.
pub const MIN_FONT_SIZE_PT: f32 = 8.0;

/// Maximum font size in points. Above this, a single glyph can dominate the
/// viewport; an absurd size (e.g. `9999`) clamps down to here.
pub const MAX_FONT_SIZE_PT: f32 = 48.0;

/// Minimum line height multiplier. `1.0` is single-spacing (the font's natural
/// leading); anything below crushes lines together, so it clamps up to here.
pub const MIN_LINE_HEIGHT: f32 = 1.0;

/// Maximum line height multiplier. Triple-spacing is already extreme for an
/// editor; values above clamp down to here.
pub const MAX_LINE_HEIGHT: f32 = 3.0;

/// Maximum paragraph spacing in points. Paragraph spacing has no sensible
/// negative value (it clamps to `0.0`); this caps the upper end.
pub const MAX_PARAGRAPH_SPACING_PT: f32 = 64.0;

/// The default editor font family.
///
/// `"-apple-system"` is the canonical token for the macOS system font (San
/// Francisco) in both AppKit (`NSFont.systemFont`, resolved by the Swift layer)
/// and CSS (the preview `WKWebView`'s `font-family`), so one default string is
/// correct on both sides of the boundary — a comfortable, system-appropriate
/// reading face.
pub const DEFAULT_FONT_FAMILY: &str = "-apple-system";

/// The default font size in points — a comfortable ~14 pt body size.
pub const DEFAULT_FONT_SIZE_PT: f32 = 14.0;

/// The default line height multiplier — a relaxed 1.4× for comfortable reading.
pub const DEFAULT_LINE_HEIGHT: f32 = 1.4;

/// The default paragraph spacing in points.
pub const DEFAULT_PARAGRAPH_SPACING_PT: f32 = 8.0;

/// Global typography preferences for the editor and preview (data-model
/// "TypographySettings"; FFI contract §8).
///
/// A plain value type with primitive fields, so the FFI shim can mirror it as a
/// `uniffi::Record` 1:1. All numeric fields carry sane bounds enforced by
/// [`Self::clamped`]; construct/repair via [`Self::default`] + struct-update or
/// pass any value through [`Self::clamped`] before trusting it.
///
/// No theme field: v1 follows the system light/dark appearance, handled
/// Swift-side (FR-039).
#[derive(Debug, Clone, PartialEq)]
pub struct TypographySettings {
    /// Font family name. An AppKit family name on the Swift side and a CSS
    /// `font-family` token in the preview; `"-apple-system"` resolves to the
    /// system font on both. A blank value falls back to [`DEFAULT_FONT_FAMILY`].
    pub font_family: String,
    /// Font size in points, clamped to `[MIN_FONT_SIZE_PT, MAX_FONT_SIZE_PT]`.
    pub font_size_pt: f32,
    /// Line height as a multiplier on the font's natural leading, clamped to
    /// `[MIN_LINE_HEIGHT, MAX_LINE_HEIGHT]` (`1.0` = single-spacing).
    pub line_height: f32,
    /// Spacing between paragraphs in points, clamped to
    /// `[0.0, MAX_PARAGRAPH_SPACING_PT]`.
    pub paragraph_spacing_pt: f32,
}

impl Default for TypographySettings {
    /// A usable, system-appropriate default configuration (a ~14 pt system font
    /// with comfortable line height and paragraph spacing). Always in range.
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FONT_FAMILY.to_owned(),
            font_size_pt: DEFAULT_FONT_SIZE_PT,
            line_height: DEFAULT_LINE_HEIGHT,
            paragraph_spacing_pt: DEFAULT_PARAGRAPH_SPACING_PT,
        }
    }
}

impl TypographySettings {
    /// Return a copy with every field validated into a sane range so the result
    /// can never produce a broken layout.
    ///
    /// - **Font family:** a blank/whitespace-only name falls back to
    ///   [`DEFAULT_FONT_FAMILY`]; otherwise kept verbatim.
    /// - **Font size:** clamped to `[MIN_FONT_SIZE_PT, MAX_FONT_SIZE_PT]`; a
    ///   non-finite value (`NaN`/`±∞`) becomes [`DEFAULT_FONT_SIZE_PT`].
    /// - **Line height:** clamped to `[MIN_LINE_HEIGHT, MAX_LINE_HEIGHT]`; a
    ///   non-finite value becomes [`DEFAULT_LINE_HEIGHT`].
    /// - **Paragraph spacing:** clamped to `[0.0, MAX_PARAGRAPH_SPACING_PT]`; a
    ///   non-finite value becomes [`DEFAULT_PARAGRAPH_SPACING_PT`].
    ///
    /// Idempotent: clamping an already-clamped value returns an equal value. This
    /// is the store's single validation gate (applied on every [`TypographyStore::set`]).
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            font_family: if self.font_family.trim().is_empty() {
                DEFAULT_FONT_FAMILY.to_owned()
            } else {
                self.font_family.clone()
            },
            font_size_pt: clamp_finite(
                self.font_size_pt,
                MIN_FONT_SIZE_PT,
                MAX_FONT_SIZE_PT,
                DEFAULT_FONT_SIZE_PT,
            ),
            line_height: clamp_finite(
                self.line_height,
                MIN_LINE_HEIGHT,
                MAX_LINE_HEIGHT,
                DEFAULT_LINE_HEIGHT,
            ),
            paragraph_spacing_pt: clamp_finite(
                self.paragraph_spacing_pt,
                0.0,
                MAX_PARAGRAPH_SPACING_PT,
                DEFAULT_PARAGRAPH_SPACING_PT,
            ),
        }
    }
}

/// Clamp a finite `value` to `[min, max]`; replace a non-finite input
/// (`NaN`/`±∞`) with `fallback`.
///
/// `f32::clamp` would propagate `NaN`, so non-finite values are handled first.
/// `fallback` is always one of the in-range per-field defaults, so the result is
/// guaranteed finite and within `[min, max]`.
fn clamp_finite(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// Thread-safe in-memory store for the single global [`TypographySettings`]
/// (US7 · FR-038). One per app session; the FFI shim hands it to Swift behind an
/// `Arc`.
///
/// Holds **no `uniffi`/`tokio`** types (Constitution V). The settings live in a
/// [`Mutex`] for parity with the rest of the core's shared state; `get` clones
/// out and `set` clamps in, so the lock is held only momentarily and a poisoned
/// lock (a panic while held — unreachable under the no-panic posture) degrades
/// to the defaults rather than propagating.
#[derive(Debug)]
pub struct TypographyStore {
    settings: Mutex<TypographySettings>,
}

impl Default for TypographyStore {
    fn default() -> Self {
        Self {
            settings: Mutex::new(TypographySettings::default()),
        }
    }
}

impl TypographyStore {
    /// Create a store seeded with the sane defaults (the editor can lay out text
    /// before Swift ever replays a saved value).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current typography settings (always in range — `set` clamps on the way
    /// in). Clones the stored value out so the caller never holds the lock.
    ///
    /// A poisoned lock degrades to the defaults rather than panicking (NFR-003):
    /// the only way to poison it is a panic while the lock was held, which the
    /// no-panic posture rules out; returning the defaults keeps `get` infallible
    /// and total, matching the contract's non-`Result` `get_typography()`.
    #[must_use]
    pub fn get(&self) -> TypographySettings {
        self.settings
            .lock()
            .map_or_else(|_| TypographySettings::default(), |s| s.clone())
    }

    /// Replace the settings with `next`, **clamped** into sane bounds first
    /// (so a bad value from the boundary can never reach `get`).
    ///
    /// Infallible at the core level: an out-of-range input is repaired by
    /// [`TypographySettings::clamped`], not rejected, so the boundary's
    /// `set_typography` returns `Ok` after this. A poisoned lock is recovered in
    /// place (the prior holder panicked, leaving the data intact) rather than
    /// propagating — there is no recoverable error here.
    pub fn set(&self, next: TypographySettings) {
        let clamped = next.clamped();
        match self.settings.lock() {
            Ok(mut guard) => *guard = clamped,
            // Recover from poisoning: the protected data is a plain value with no
            // broken invariant, so overwrite it with the clamped value.
            Err(poisoned) => *poisoned.into_inner() = clamped,
        }
    }
}

#[cfg(test)]
mod tests {
    // Unit tests assert on their own fixtures; the workspace denies these in
    // library code, so scope the allowance to this test module.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{
        clamp_finite, TypographySettings, TypographyStore, DEFAULT_FONT_FAMILY, MAX_FONT_SIZE_PT,
        MAX_LINE_HEIGHT, MIN_FONT_SIZE_PT, MIN_LINE_HEIGHT,
    };

    #[test]
    fn default_is_in_range() {
        let d = TypographySettings::default();
        // The default is already a fixed point of `clamped` (in range).
        assert_eq!(d, d.clamped());
        assert!(!d.font_family.is_empty());
    }

    #[test]
    fn clamp_finite_handles_range_and_non_finite() {
        assert_eq!(clamp_finite(5.0, 1.0, 10.0, 3.0), 5.0); // in range
        assert_eq!(clamp_finite(-1.0, 1.0, 10.0, 3.0), 1.0); // below → min
        assert_eq!(clamp_finite(99.0, 1.0, 10.0, 3.0), 10.0); // above → max
        assert_eq!(clamp_finite(f32::NAN, 1.0, 10.0, 3.0), 3.0); // NaN → fallback
        assert_eq!(clamp_finite(f32::INFINITY, 1.0, 10.0, 3.0), 3.0); // ∞ → fallback
    }

    #[test]
    fn clamped_pins_each_field_to_its_bound() {
        let raw = TypographySettings {
            font_family: "  ".to_owned(),
            font_size_pt: 1000.0,
            line_height: 0.0,
            paragraph_spacing_pt: -5.0,
        };
        let c = raw.clamped();
        assert_eq!(c.font_family, DEFAULT_FONT_FAMILY);
        assert_eq!(c.font_size_pt, MAX_FONT_SIZE_PT);
        assert_eq!(c.line_height, MIN_LINE_HEIGHT);
        assert_eq!(c.paragraph_spacing_pt, 0.0);
    }

    #[test]
    fn store_get_set_round_trips_in_range() {
        let store = TypographyStore::new();
        let want = TypographySettings {
            font_family: "Menlo".to_owned(),
            font_size_pt: 16.0,
            line_height: 1.6,
            paragraph_spacing_pt: 12.0,
        };
        store.set(want.clone());
        assert_eq!(store.get(), want);
    }

    #[test]
    fn store_set_clamps_out_of_range() {
        let store = TypographyStore::new();
        store.set(TypographySettings {
            font_family: String::new(),
            font_size_pt: 0.0,
            line_height: 99.0,
            paragraph_spacing_pt: -1.0,
        });
        let got = store.get();
        assert_eq!(got.font_family, DEFAULT_FONT_FAMILY);
        assert_eq!(got.font_size_pt, MIN_FONT_SIZE_PT);
        assert_eq!(got.line_height, MAX_LINE_HEIGHT);
        assert_eq!(got.paragraph_spacing_pt, 0.0);
    }

    #[test]
    fn fresh_store_reports_defaults() {
        assert_eq!(TypographyStore::new().get(), TypographySettings::default());
    }
}
