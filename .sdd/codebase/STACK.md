# Technology Stack

> **Purpose**: Document what executes in this codebase — languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | 1.85 (pinned MSRV) | Core engine: file IO, watching, indexing, Markdown parsing, search, AI client |
| Swift | 6.0 (Xcode 16.2+) | Native macOS frontend UI, editor surface, sidebar, tabs, preview |
| C (via UniFFI) | ABI shim | FFI boundary between Rust and Swift |

## Frameworks

| Framework | Version | Purpose |
|-----------|---------|---------|
| SwiftUI | 6.0 | Declarative UI framework for the macOS application |
| AppKit | macOS 14+ | Native APIs for `NSTextView` (TextKit 2), `NSOutlineView`, `WKWebView`, Keychain |
| UniFFI | 0.31 | FFI binding generator (proc-macro mode): Rust → C ABI → Swift bindings |

## Critical Dependencies

### Rust Core (`emend-core`)

These packages are actively wired into the runtime:

| Package | Version | Purpose | Wiring Status |
|---------|---------|---------|---------------|
| `ropey` | 1.6.1 | Shadow rope for UTF-16/line indexing in the per-keystroke editor hot path | **WIRED** — backing the `Document` model |
| `tempfile` | 3.x | Atomic + durable writes via temp file + fsync + rename | **WIRED** — used in `fs::write_atomic` |
| `thiserror` | 2.x | Error type Display/Error derive for `EmendError` enum | **WIRED** — core error handling |

### Rust FFI Bridge (`emend-ffi`)

| Package | Version | Purpose | Wiring Status |
|---------|---------|---------|---------------|
| `tokio` | 1.x (rt-multi-thread, macros, time, sync) | Async runtime for cancellable AI/search work | **WIRED** — long-lived runtime in the FFI layer |
| `tokio-util` | 0.7 | `CancellationToken` for Rust-owned cancellation handles | **WIRED** — backing async cancellation |
| `uniffi` | 0.31 | FFI binding scaffold (no UDL, pure proc-macro mode) | **WIRED** — FFI boundary |
| `thiserror` | 2.x | Re-exported error Display for FFI projection | **WIRED** — error bridging |

### Benchmarking (`emend-bench`)

| Package | Version | Purpose | Wiring Status |
|---------|---------|---------|---------------|
| `criterion` | 0.7 | Micro-benchmark harness (perf budgets tracked, non-blocking) | **DEV-ONLY** — benchmarking (phase 3) |

### Catalogued but Inert (Not Yet Wired)

These are pinned in the workspace `[workspace.dependencies]` but not yet imported by any crate:

| Package | Version | Purpose | Why Inert | Planned For |
|---------|---------|---------|-----------|------------|
| `tree-sitter` | 0.25 | Incremental Markdown syntax highlighting | Not imported | Phase 1 (US7 — syntax highlighting) |
| `tree-sitter-md` | 0.5 | Tree-sitter Markdown grammar | Not imported | Phase 1 (US7) |
| `comrak` | 0.52 | CommonMark + GFM parsing for preview HTML | Not imported | Phase 1 (US3 — preview) |
| `syntect` | 5.3 | Code block syntax highlighting (20+ languages) | Not imported | Phase 1 (US7) |
| `nucleo` | 0.5 | Fuzzy search / Quick Open ranking | Not imported | Phase 2 (US2 — location tree + Quick Open) |
| `notify` | 8.2 | File watching (macOS native) | Not imported | Phase 0–1 (FR-006a — autoreload on external change) |
| `notify-debouncer-full` | 0.7 | Debounced file watcher + self-write suppression | Not imported | Phase 0–1 (FR-006a) |
| `reqwest` | 0.13 (json, stream) | HTTP client with SSE streaming for AI | Not imported | Phase 1 (FR-023 — AI client) |
| `serde` / `serde_json` | 1.x | Serialization for AI request/response JSON | Not imported | Phase 1 (FR-023) |

These will be imported into `emend-core` as the corresponding user stories land in `/sdd:implement` phases.

## Package Managers & Build Tools

| Tool | Version | Purpose |
|------|---------|---------|
| `cargo` | 1.85+ | Rust build, test, clippy, fmt |
| `just` | (any) | Task runner; see `justfile` for `build`, `test`, `clippy`, `fmt-check`, `check`, `xcframework` |
| `Xcode` | 16.2+ | Swift build, SwiftUI preview, XCTest |
| `Swift` (compiler) | 6.0+ | Swift 6 strict-concurrency mode (Swift 5 for generated UniFFI bindings) |

## Runtime Environment

| Environment | Details |
|-------------|---------|
| **OS Target** | macOS 14.0+ (Sonoma+) |
| **Architecture** | arm64 (Apple Silicon) only |
| **Deployment** | Native .app bundle (single-window macOS application) |
| **No Database** | Plain `.md` files on disk; app state in macOS Keychain (for API key) and user defaults |
| **No Network by Default** | Zero outbound network unless AI is configured AND invoked by the user |

## Build Profile

```toml
[profile.release]
lto = "thin"
codegen-units = 1
```

Thin LTO for faster builds while retaining optimization; single codegen unit for better inlining.

## Cross-Boundary Semantics

- **Text buffer**: Swift owns the canonical `NSTextStorage`; Rust shadows it with a `ropey::Rope` for off-main-thread queries.
- **Coordinates**: FFI boundary uses **UTF-16 code units** (not UTF-8 offsets) to map 1:1 onto `NSRange` and avoid per-keystroke transcoding (research §A2).
- **Async wiring**: Rust tokio runtime lives in `emend-ffi`; `emend-core` stays purely synchronous for testability (Constitution V, research §B8).

## What Does NOT Belong Here

- Directory structure → `STRUCTURE.md`
- System design patterns → `ARCHITECTURE.md`
- External service integrations → `INTEGRATIONS.md`
- Dev tools (linting, formatting) → `CONVENTIONS.md`
- Test frameworks → `TESTING.md`

---

*This document captures only what executes. Keep it focused on languages, frameworks, and dependencies. See CLAUDE.md for governance and `.sdd/memory/constitution.md` for Principles.*
