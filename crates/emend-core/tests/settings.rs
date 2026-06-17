//! T123 — failing-first integration tests for the typography settings store
//! (`emend_core::settings`), the global editor/preview typography preferences
//! (US7 · FR-038/FR-039; FFI contract §8).
//!
//! `TypographySettings` is **app-managed global state** (data-model
//! "TypographySettings"): font family, font size, line height, and paragraph
//! spacing applied to both the editor and the preview. The core has **no
//! persistence layer** (US2 guardrail): the store holds the settings *in
//! memory* with get/set; persistence is the Swift layer's job (UserDefaults),
//! replayed into the core on launch via `set`. So "persist + round-trip" here
//! means the get/set + value round-trip — the value you `set` is the value you
//! `get` back (post-clamp), with no disk involved.
//!
//! Theme (light/dark) is **deliberately not** a field: v1 follows the system
//! appearance automatically, handled Swift-side (FR-039). These tests pin three
//! obligations:
//!
//! 1. **Sane defaults.** A fresh store yields a usable, system-appropriate
//!    configuration (a non-empty default font family, a comfortable ~14 pt size,
//!    a line height ≥ 1.0, and non-negative paragraph spacing) so the editor can
//!    lay out text before the Swift layer ever replays a saved value.
//!
//! 2. **Round-trip (set → get) for in-range values.** A configuration whose
//!    values are all within bounds survives a `set`/`get` cycle byte-for-byte
//!    (this is the "persist" obligation given the core keeps it in memory).
//!
//! 3. **Clamping/validation of out-of-range values.** A hostile or buggy value
//!    arriving from the boundary (size 0, size 9999, negative spacing, a tiny
//!    sub-1.0 line height) MUST be clamped into sane bounds rather than stored
//!    verbatim — a bad value can never produce a broken layout. Clamping is
//!    idempotent (clamping an already-clamped value is a no-op) and an empty
//!    font family falls back to the default.
//!
//! The store holds **no `uniffi` / no `tokio`** types (Constitution V), so this
//! whole suite runs under plain `cargo test` with no FFI toolchain.

// Integration tests assert on their own fixtures; the workspace denies these in
// library code, so scope the allowance to this test module.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::settings::{
    TypographySettings, TypographyStore, MAX_FONT_SIZE_PT, MAX_LINE_HEIGHT,
    MAX_PARAGRAPH_SPACING_PT, MIN_FONT_SIZE_PT, MIN_LINE_HEIGHT,
};

// -- 1. defaults ------------------------------------------------------------

#[test]
fn defaults_are_sane() {
    let d = TypographySettings::default();

    // A usable default font family (system-appropriate, non-empty).
    assert!(
        !d.font_family.trim().is_empty(),
        "default font family must not be blank"
    );

    // ~14 pt, comfortably inside the allowed range.
    assert!(
        (MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&d.font_size_pt),
        "default font size {} must be within [{MIN_FONT_SIZE_PT}, {MAX_FONT_SIZE_PT}]",
        d.font_size_pt
    );
    assert!(
        (12.0..=16.0).contains(&d.font_size_pt),
        "default font size should be a comfortable ~14pt, got {}",
        d.font_size_pt
    );

    // A comfortable line height (≥ 1.0, single-spacing or looser).
    assert!(
        (MIN_LINE_HEIGHT..=MAX_LINE_HEIGHT).contains(&d.line_height),
        "default line height {} out of bounds",
        d.line_height
    );
    assert!(
        d.line_height >= 1.0,
        "default line height should be at least single-spacing, got {}",
        d.line_height
    );

    // Non-negative paragraph spacing.
    assert!(
        (0.0..=MAX_PARAGRAPH_SPACING_PT).contains(&d.paragraph_spacing_pt),
        "default paragraph spacing {} out of bounds",
        d.paragraph_spacing_pt
    );

    // A fresh store reports the defaults.
    let store = TypographyStore::new();
    assert_eq!(store.get(), TypographySettings::default());
}

// -- 2. round-trip ----------------------------------------------------------

#[test]
fn in_range_values_round_trip() {
    let store = TypographyStore::new();

    let want = TypographySettings {
        font_family: "SF Mono".to_owned(),
        font_size_pt: 18.0,
        line_height: 1.5,
        paragraph_spacing_pt: 8.0,
    };

    // All values are in range, so set is a verbatim store and get returns them.
    store.set(want.clone());
    assert_eq!(store.get(), want);

    // A second set replaces the first (last-write-wins, not merge).
    let second = TypographySettings {
        font_family: "Helvetica Neue".to_owned(),
        font_size_pt: 13.0,
        line_height: 1.2,
        paragraph_spacing_pt: 0.0,
    };
    store.set(second.clone());
    assert_eq!(store.get(), second);
}

// -- 3. clamping / validation ----------------------------------------------

#[test]
fn zero_and_huge_font_size_clamp_into_range() {
    let store = TypographyStore::new();

    // Size 0 → clamped up to the floor.
    store.set(TypographySettings {
        font_size_pt: 0.0,
        ..TypographySettings::default()
    });
    assert_eq!(store.get().font_size_pt, MIN_FONT_SIZE_PT);

    // Size 9999 → clamped down to the ceiling.
    store.set(TypographySettings {
        font_size_pt: 9999.0,
        ..TypographySettings::default()
    });
    assert_eq!(store.get().font_size_pt, MAX_FONT_SIZE_PT);
}

#[test]
fn negative_spacing_and_tiny_line_height_clamp() {
    let store = TypographyStore::new();

    store.set(TypographySettings {
        // Negative paragraph spacing is nonsense for a layout: clamp to the floor.
        paragraph_spacing_pt: -10.0,
        // A sub-single line height would crush lines together: clamp up.
        line_height: 0.1,
        ..TypographySettings::default()
    });

    let got = store.get();
    assert!(
        got.paragraph_spacing_pt >= 0.0,
        "negative paragraph spacing must clamp to >= 0, got {}",
        got.paragraph_spacing_pt
    );
    assert_eq!(got.line_height, MIN_LINE_HEIGHT);
}

#[test]
fn empty_font_family_falls_back_to_default() {
    let store = TypographyStore::new();

    store.set(TypographySettings {
        font_family: "   ".to_owned(),
        ..TypographySettings::default()
    });
    assert_eq!(
        store.get().font_family,
        TypographySettings::default().font_family,
        "a blank font family must fall back to the default"
    );
}

#[test]
fn clamping_is_idempotent_and_done_at_construction() {
    // `clamped()` is the single validation gate; clamping an already-clamped
    // value is a no-op, and out-of-range values are repaired the moment they
    // enter (so a `get` can never return an unclamped value).
    let raw = TypographySettings {
        font_family: String::new(),
        font_size_pt: -5.0,
        line_height: 99.0,
        paragraph_spacing_pt: -1.0,
    };
    let once = raw.clamped();
    let twice = once.clone().clamped();
    assert_eq!(once, twice, "clamping must be idempotent");

    // Every field of the clamped value is within bounds.
    assert!(!once.font_family.trim().is_empty());
    assert!((MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&once.font_size_pt));
    assert!((MIN_LINE_HEIGHT..=MAX_LINE_HEIGHT).contains(&once.line_height));
    assert!((0.0..=MAX_PARAGRAPH_SPACING_PT).contains(&once.paragraph_spacing_pt));

    // And the store applies the same gate on set.
    let store = TypographyStore::new();
    store.set(raw);
    assert_eq!(store.get(), once);
}

#[test]
fn nan_values_are_repaired_not_stored() {
    // A NaN crossing the boundary (a buggy caller) must not poison the layout:
    // the clamp replaces non-finite values with the default for that field.
    let store = TypographyStore::new();
    store.set(TypographySettings {
        font_family: "Menlo".to_owned(),
        font_size_pt: f32::NAN,
        line_height: f32::INFINITY,
        paragraph_spacing_pt: f32::NEG_INFINITY,
    });

    let got = store.get();
    assert!(got.font_size_pt.is_finite(), "font size must be finite");
    assert!(got.line_height.is_finite(), "line height must be finite");
    assert!(
        got.paragraph_spacing_pt.is_finite(),
        "paragraph spacing must be finite"
    );
    assert!((MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&got.font_size_pt));
    assert!((MIN_LINE_HEIGHT..=MAX_LINE_HEIGHT).contains(&got.line_height));
    assert!((0.0..=MAX_PARAGRAPH_SPACING_PT).contains(&got.paragraph_spacing_pt));
    // The (valid) font family survives.
    assert_eq!(got.font_family, "Menlo");
}
