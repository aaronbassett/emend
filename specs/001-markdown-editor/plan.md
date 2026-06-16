# Implementation Plan: Emend — A Quiet, Native macOS Markdown Editor

**Branch**: `001-markdown-editor` | **Date**: 2026-06-16 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-markdown-editor/spec.md`

## Summary

Emend is a native macOS (Apple Silicon) Markdown editor — a cogito.md clone minus Typefully — built as a **Rust core + Swift/SwiftUI UI**. The core (`emend-core`) owns file IO, recursive folder watching, the workspace index, Markdown parsing, fuzzy search, and a BYOM OpenAI-compatible AI client; it is exposed to Swift through a thin **UniFFI** shim (`emend-ffi`) packaged as an XCFramework. The Swift app owns the live editor (`NSTextView` on TextKit 2, with dimmed-syntax display attributes), a `WKWebView` preview (bundled Mermaid + KaTeX, syntect-highlighted code), an `NSOutlineView` sidebar across security-scoped folder "locations", in-app tabs, Quick Open, and the info sidebar. Notes stay as plain files on disk (no DB/sync); the AI key lives in Keychain. The architecture is driven by three hard constraints: ≤50 ms p95 typing on 1 MB docs (incremental parse + Swift-owned buffer + UTF-16 boundary), live-but-safe file refresh (debounced watcher + self-write suppression + atomic durable writes + explicit conflict policy), and privacy (no network unless AI is configured and invoked). Full technical decisions: [research.md](./research.md).

## Technical Context

**Language/Version**: Rust (edition 2021, rust-version ≥ 1.85; dev toolchain 1.97-nightly present) · Swift 6.0 (Xcode 16.2), target arm64-apple-macosx14.0
**Primary Dependencies**:
- *Core (Rust)*: tokio + tokio-util (runtime/cancellation), tree-sitter + tree-sitter-md (incremental editor highlight), comrak (preview HTML), syntect (code highlighting), nucleo (fuzzy Quick Open), notify + notify-debouncer-full (file watching), tempfile (atomic writes), reqwest (SSE AI client), thiserror (errors)
- *Boundary*: UniFFI 0.31 (Rust→Swift, XCFramework, async + foreign-trait callbacks + panic containment)
- *UI (Swift)*: AppKit/TextKit 2 (`NSTextView`, `NSOutlineView`), SwiftUI, WKWebView, Security framework (Keychain), bundled Mermaid.js + KaTeX
**Storage**: Plain Markdown files on disk (source of truth); core-owned local app-support store for locations/favorites/icons/typography/AI metadata; **Keychain** for the AI API key. No database, no sync.
**Testing**: `cargo test` (core, isolated — no FFI/Swift toolchain) + Criterion benches; XCTest (Swift unit + XCUITest UI) + `measure` perf tests for the 50 ms budget
**Target Platform**: macOS 14+ on Apple Silicon only (sandboxed)
**Project Type**: Hybrid native desktop — Rust workspace (`crates/`) + Swift app (`app/`) consuming a local SwiftPM package (`swift/EmendCore`)
**Performance Goals**: ≤50 ms p95 keystroke→glyph on ~1 MB/10k-line docs (SC-003); large doc visible <500 ms p95 (SC-002); Quick Open ≤100 ms p95 over 10k files (SC-004); derived data (outline/stats) refresh ≤300 ms without blocking typing (FR-031a)
**Constraints**: Offline-by-default — zero outbound network unless AI configured + invoked (SC-008); atomic+durable autosave, never a partial file (FR-009a); no panic across the FFI boundary, recoverable errors never abort (NFR-003); bounded memory ∝ open docs + index (NFR-005)
**Scale/Scope**: Workspaces to ~10k files; 7 prioritized user stories; 54 FR + 7 NFR + 7 DS; v1 AI = document summary only

**Resolved unknowns**: All Phase-0 NEEDS CLARIFICATION items are resolved in research.md (interop = UniFFI; parsing = tree-sitter editor + comrak preview; search = nucleo; watcher = notify+debouncer-full; writes = tempfile+F_FULLFSYNC; AI = reqwest SSE; highlight = syntect; preview = WKWebView+Mermaid+KaTeX; sandbox = security-scoped bookmarks; key = Keychain). Remaining items are non-blocking product-config defaults (research §D).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Gated against **`.sdd/memory/constitution.md` v1.0.0** (Principles I–VII). The spec's
Development Standards (DS-001..007) and Non-Functional Requirements (NFR-001..007) are
the concrete, testable expression of those principles:

| Constitution principle | Plan compliance |
|------|------|
| I. Plain Files, User Sovereignty | ✅ FR-003 — plain `.md` on disk, no DB/sync; agents read/write same files |
| II. Local-First & Privacy by Default | ✅ FR-035/SC-008 (offline; opt-in AI); NFR-006 key in Keychain, redacted; sandbox + scoped bookmarks |
| III. Never Lose User Data | ✅ FR-009a atomic+durable writes; FR-006a self-write suppression; FR-006c conflict policy |
| IV. Native Performance | ✅ NFR-001 non-blocking; incremental parse/index; budgets tracked via criterion + `measure` |
| V. Clean Core/UI Boundary & Crash Safety | ✅ `emend-core` FFI-free + isolated tests; UniFFI catch_unwind + `deny(unwrap/expect/panic)`; UTF-16 boundary |
| VI. Minimalism & Simplicity | ✅ v1 AI = summary only; mature crates; two-engine markdown justified in Complexity Tracking |
| VII. Quality Discipline & Reproducible Builds | ✅ rustfmt/clippy `-D warnings` + SwiftFormat/SwiftLint; strict core tests / pragmatic UI; lefthook + CI; Conventional Commits; pinned deps |

**Result: PASS** (initial and post-design). No violations to justify — the one inherent
complexity (two Markdown engines) is recorded in Complexity Tracking per Principle VI.

## Project Structure

### Documentation (this feature)

```text
specs/001-markdown-editor/
├── spec.md              # Feature spec (/sdd:specify)
├── plan.md              # This file (/sdd:plan)
├── research.md          # Phase 0 — all technical decisions
├── data-model.md        # Phase 1 — entities & state transitions
├── quickstart.md        # Phase 1 — dev setup & build/run/test
├── contracts/
│   └── ffi-interface.md # Phase 1 — the Swift↔Rust UniFFI contract
├── checklists/
│   └── requirements.md  # spec quality checklist (PASS)
└── tasks.md             # Phase 2 — created by /sdd:tasks (NOT this command)
```

### Source Code (repository root)

```text
emend/
├── Cargo.toml                     # Rust workspace + pinned dependency catalog + lint policy
├── rustfmt.toml
├── crates/
│   ├── emend-core/                # ALL core logic; NO uniffi dep; cargo-test in isolation
│   │   └── src/{lib.rs,error.rs,  #   fs, watcher, index, parse, search, ai (added by /sdd:implement)}
│   ├── emend-ffi/                 # thin UniFFI shim: #[uniffi::export], error/handle/callback types
│   └── emend-bench/               # Criterion benches (added during implementation)
├── scripts/
│   └── build-xcframework.sh       # aarch64 build + uniffi-bindgen-swift → EmendCore.xcframework
├── swift/
│   └── EmendCore/                 # local SwiftPM package: binaryTarget(xcframework) + generated bindings + clean wrappers
├── app/
│   └── Emend/                     # Xcode macOS app (AppKit+SwiftUI): editor, preview, sidebar, tabs, settings
│                                  #   — created per quickstart.md (Xcode project not generated by CLI)
├── .swiftformat  .swiftlint.yml  .editorconfig  .gitignore
├── lefthook.yml  justfile
└── .github/workflows/ci.yml
```

**Structure Decision**: A **hybrid layout** — a Rust Cargo workspace (`crates/`) split into pure-logic `emend-core` (toolchain-free `cargo test`) and the FFI shim `emend-ffi`, plus a Swift side split into a reusable local SwiftPM package (`swift/EmendCore`, wrapping the built XCFramework + generated UniFFI bindings) and the Xcode app target (`app/Emend`). This keeps the testable logic boundary clean (research §B8/§C9), isolates the panic/error boundary in `emend-ffi`, and lets the two toolchains build independently. The Xcode app project is scaffolded by the developer following quickstart.md (creating a signed/structured `.xcodeproj` reliably is outside CLI scope and belongs to `/sdd:implement`).

## Complexity Tracking

No constitution gates are violated, so no justifications are required. One inherent complexity is noted for visibility, not as a violation:

| Complexity | Why needed | Simpler alternative rejected because |
|-----------|------------|--------------------------------------|
| Two Markdown engines (tree-sitter editor + comrak preview) | ≤50 ms p95 typing requires *incremental* highlight; preview requires *correct* CommonMark HTML; no single Rust crate does both | A single non-incremental parser reparsing 1 MB per keystroke blows SC-003; a hand-rolled incremental CommonMark parser has block-boundary correctness hazards (research §B1) |

## Phase Status

- ✅ **Phase 0** — research.md (all unknowns resolved; no blocking clarifications)
- ✅ **Phase 1** — data-model.md, contracts/ffi-interface.md, quickstart.md, agent context (CLAUDE.md)
- ✅ **Phase 2** — dev environment: Rust workspace skeleton (builds/tests/clippy/fmt green offline), Swift `EmendCore` package (builds/tests green), lefthook hooks installed, CI workflow, justfile
- ⏭️ **Next** — `/sdd:tasks` to generate the dependency-ordered `tasks.md`

### Tech Debt / follow-ups (greenfield — expected)
- `emend-ffi` UniFFI wiring (`uniffi::setup_scaffolding!()`, exports, callbacks) — first implementation task.
- `app/Emend` Xcode project creation + XCFramework integration (quickstart.md).
- SwiftFormat/SwiftLint binaries not installed in this environment (`brew install swiftformat swiftlint`); CI installs them.
- Network to crates.io was blocked during planning, so heavy Rust deps are cataloged (pinned) but not yet vendored; first online `cargo build` will fetch them.
