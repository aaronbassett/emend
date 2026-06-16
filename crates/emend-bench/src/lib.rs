//! Criterion benchmark harness for `emend-core`.
//!
//! This crate carries no logic of its own; it exists solely to host perf
//! benches under `benches/`. Perf budgets are tracked but non-blocking per the
//! constitution. The empty lib target satisfies Cargo's requirement that a
//! crate expose at least one lib/bin target alongside its `[[bench]]` entries.
//!
//! Real benches (e.g. `benches/highlight.rs`, `benches/quick_open.rs`) land in
//! later phases as the corresponding `emend-core` capabilities are wired in.
