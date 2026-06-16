# Emend — Agent Context

A quiet, native **macOS (Apple Silicon)** Markdown editor — a cogito.md clone (no Typefully).
**Rust core + Swift/SwiftUI UI.** Notes are plain files on disk (no DB/sync). AI is BYOM (OpenAI-compatible).

**Governance**: [`.sdd/memory/constitution.md`](.sdd/memory/constitution.md) v1.0.0 — read it; Principles I–VII are binding.
Specs of record: `specs/001-markdown-editor/` — [spec](specs/001-markdown-editor/spec.md) · [plan](specs/001-markdown-editor/plan.md) · [research](specs/001-markdown-editor/research.md) · [data-model](specs/001-markdown-editor/data-model.md) · [FFI contract](specs/001-markdown-editor/contracts/ffi-interface.md).

## Active Technologies

- **Rust** (edition 2021, ≥1.85) core: tokio, tree-sitter + tree-sitter-md (incremental editor highlight), comrak (preview HTML), syntect (code highlight), nucleo (Quick Open), notify + notify-debouncer-full (watching), tempfile (atomic writes), reqwest (SSE AI client), thiserror.
- **UniFFI 0.31** boundary → XCFramework (async + foreign-trait callbacks + panic containment).
- **Swift 6 / SwiftUI + AppKit** (Xcode 16.2, macOS 14+): `NSTextView`/TextKit 2 editor, `NSOutlineView` sidebar, `WKWebView` preview (bundled Mermaid + KaTeX), Security framework (Keychain).

## Structure

- `crates/emend-core` — ALL logic, **no FFI dep**, `cargo test` in isolation.
- `crates/emend-ffi` — thin `#[uniffi::export]` shim (the only `uniffi` consumer).
- `swift/EmendCore` — SwiftPM package wrapping the XCFramework + generated bindings.
- `app/Emend` — Xcode macOS app (created during implementation).

## Commands

```bash
just build | test | clippy | fmt-check | check   # check = full pre-push gate
cargo test                                        # core, no Xcode needed
just xcframework                                  # Rust core → EmendCore.xcframework + bindings
swift build && swift test                         # in swift/EmendCore
```

## Project guardrails (do not violate)

- **No panics across FFI** (NFR-003): `emend-core` denies `unwrap_used`/`expect_used`/`panic`; fallible boundary calls return `Result<_, EmendError>`. UniFFI `catch_unwind` contains the rest.
- **FFI ranges are UTF-16 code units** (research §A2) to map onto `NSRange` — never pass UTF-8 offsets across the boundary.
- **Swift owns the text buffer**; per-keystroke edits go to Rust as tiny deltas. Hot path is **synchronous**; async only for AI + search (cancellable via Rust handles, not Swift `Task`).
- **Privacy** (SC-008/FR-035): zero outbound network unless AI is configured AND invoked. Preview WebView blocks remote loads (CSP + nonPersistent + navigation delegate).
- **Autosave is atomic + durable** (FR-009a): tempfile → fsync → rename → fsync dir; debounced (don't fsync per keystroke). Feed post-write `(mtime,len)` to the watcher's self-write suppression so saves don't echo (FR-006a).
- **AI key**: Keychain only, transient to Rust, redacted in the HTTP client — never logged/persisted (NFR-006).
- **Two Markdown engines on purpose**: tree-sitter (editor, incremental, advisory highlight) vs comrak (preview, authoritative HTML). Don't unify them.
- Conventional Commits required (DS-007); `lefthook` runs fmt/clippy/swift-lint pre-commit.

## Recent Changes

- 2026-06-16: `/sdd:constitution` — constitution v1.0.0 ratified (Principles I–VII; testing = strict-core/pragmatic-UI; perf budgets = tracked, non-blocking).
- 2026-06-16: `/sdd:plan` — Phase 0–2 complete. Rust workspace skeleton + Swift `EmendCore` package build/test green; lefthook hooks + CI configured. Next: `/sdd:tasks`.

<!-- MANUAL ADDITIONS START -->
<!-- Add project-specific notes here; this block is preserved across regenerations. -->
<!-- MANUAL ADDITIONS END -->
