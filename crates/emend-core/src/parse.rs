//! Markdown parsing — the **two-engine split** (research §B1, Constitution).
//!
//! Emend parses Markdown with **two independent engines, on purpose**, and they
//! must never be unified:
//!
//! - [`highlight`] — the **incremental tree-sitter editor-highlight engine**.
//!   It runs on the per-keystroke hot path (≤50 ms typing budget, SC-003), is
//!   **advisory only** (it colours the editor; it is not the source of truth for
//!   rendered output), and reparses *incrementally* so a keystroke reparses a
//!   small neighbourhood rather than the whole document.
//!
//! - the comrak **preview engine** (lands later) — the **authoritative** HTML
//!   renderer for the preview pane. It is whole-document, CommonMark-correct, and
//!   deliberately distinct from the editor highlighter: their jobs, performance
//!   characteristics, and correctness obligations differ.
//!
//! Keeping them separate means the editor can be fast-and-approximate while the
//! preview stays correct-and-complete, without one compromising the other.

pub mod highlight;
