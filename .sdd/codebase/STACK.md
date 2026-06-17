# Technology Stack

> **Purpose**: Document what executes in this codebase — languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (Phase 10 Polish: perf benchmarking, accessibility identifiers, security review findings, large-file tests)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | 1.85 (pinned MSRV) | Core engine: file IO, watching, indexing, Markdown parsing, search, AI client, typography settings storage |
| Swift | 6.0 (Xcode 16.2+) | Native macOS frontend UI, editor surface, sidebar, tabs, preview, PDF export, wiki-link autocomplete, typography UI, accessibility identifiers |
| C (via UniFFI) | ABI shim | FFI boundary between Rust and Swift |

## Frameworks

| Framework | Version | Purpose |
|-----------|---------|---------|
| SwiftUI | 6.0 | Declarative UI framework for the macOS application |
| AppKit | macOS 14+ | Native APIs for `NSTextView` (TextKit 2), `NSOutlineView`, `WKWebView`, `NSPrintOperation`, `NSFontManager`, `NSFont`, `NSParagraphStyle` (typography controls), Keychain |
| UniFFI | 0.31 | FFI binding generator (proc-macro mode): Rust → C ABI → Swift bindings |

## Critical Dependencies

### Rust Core (`emend-core`)

These packages are actively wired into the runtime:

| Package | Version | Purpose | Wiring Status |
|---------|---------|---------|---------------|
| `ropey` | 1.6.1 | Shadow rope for UTF-16/line indexing in the per-keystroke editor hot path | **WIRED** — backing the `Document` model and `Highlighter` rope |
| `tree-sitter` | 0.26 | Incremental parser runtime for editor-highlight engine (block + inline grammars) | **WIRED** — Phase 3 US1 (Editor MVP), `parse/highlight.rs` |
| `tree-sitter-md` | 0.5 | Split Markdown grammar (block + inline); wrapped by `MarkdownParser`/`MarkdownTree` | **WIRED** — Phase 3 US1, `parse/highlight.rs` |
| `comrak` | 0.52 | CommonMark + GFM preview engine (authoritative, whole-document); distinct from tree-sitter editor highlighter ("two engines on purpose"); renders wikilinks natively | **WIRED** — Phase 6 US4 (Preview + PDF export), Phase 7 US5 (embeds), `parse/preview.rs`; rendered via `render_preview_html()` / `render_preview_html_with_embeds()` FFI exports with comrak's `render.sourcepos` + `data-line` scroll-sync anchors (research §C3) |
| `syntect` | 5.3 | Code-block syntax highlighting (20+ languages) for preview fenced blocks; loads vendored binary `SyntaxSet`/`ThemeSet` dump (`assets/syntaxes-themes.packdump`) | **WIRED** — Phase 6 US4, `parse/code_highlight.rs`; drives `ClassedHTMLGenerator` via `EmendSyntectAdapter` plugged into comrak; theme CSS exported via `preview_theme_css()` FFI |
| `tempfile` | 3.x | Atomic + durable writes via temp file + fsync + rename; also used in tests for isolated file I/O (Phase 10 large-file/bounded-memory tests) | **WIRED** — used in `fs::write_atomic`, `fs::store_attachment` (Phase 7 US5), and `tests/` for isolated FS operations |
| `thiserror` | 2.x | Error type Display/Error derive for `EmendError` enum | **WIRED** — core error handling |
| `nucleo-matcher` | 0.3.1 | Synchronous fuzzy-matching primitive for workspace search index and wiki-link suggestions (lighter than full `nucleo`); used in both Quick Open and link autocomplete | **WIRED** — Phase 4 US2, Phase 7 US5 `derived::wikilink_suggestions()`, `index.rs` — in-memory haystack for Quick Open + wiki-link resolution |
| `notify` | 8.2 | File system watching (macOS FSEvents recursive watcher) | **WIRED** — Phase 4 US2, `watcher.rs` — detects external note edits and changes |
| `notify-debouncer-full` | 0.7 | Debounced file watcher with self-write suppression (FileIdCache) | **WIRED** — Phase 4 US2, `watcher.rs` — coalesces FS bursts, prevents echo-back on autosaves |
| `serde` | 1.x | JSON serialization/deserialization for OpenAI Chat-Completions API request/response shapes | **WIRED** — Phase 8 US6, `ai.rs` (pure, no-network JSON parsing) |
| `serde_json` | 1.x | JSON parsing for OpenAI Chat-Completions responses | **WIRED** — Phase 8 US6, `ai.rs` — ditto |

### Rust FFI Bridge (`emend-ffi`)

| Package | Version | Purpose | Wiring Status |
|---------|---------|---------|---------------|
| `tokio` | 1.x (rt-multi-thread, macros, time, sync) | Async runtime for cancellable AI/search work | **WIRED** — long-lived runtime in the FFI layer |
| `tokio-util` | 0.7 | `CancellationToken` for Rust-owned cancellation handles | **WIRED** — backing async cancellation |
| `reqwest` | 0.13 (stream, native-tls) | HTTP client with SSE streaming for OpenAI-compatible API; macOS-native TLS (Security framework), no rustls/ring; minimal feature surface | **WIRED** — Phase 8 US6, `ai.rs` (streaming orchestration for `summarize_document`), emend-ffi only so emend-core stays network-free (Constitution V); `idna_adapter = "=1.1.0"` pinned to keep MSRV ≤ 1.85 (icu 2.x needs 1.86) |
| `futures-util` | 0.3.32 (std feature) | `StreamExt::next` for draining reqwest's `bytes_stream()` SSE chunks | **WIRED** — Phase 8 US6, `ai.rs` — feeding bytes through the core `SseParser`; already a transitive dep of reqwest |
| `uniffi` | 0.31 | FFI binding scaffold (no UDL, pure proc-macro mode) | **WIRED** — FFI boundary |
| `thiserror` | 2.x | Re-exported error Display for FFI projection | **WIRED** — error bridging |

### Benchmarking (`emend-bench`)

| Package | Version | Purpose | Wiring Status |
|---------|---------|---------|---------------|
| `criterion` | 0.7 | Micro-benchmark harness (perf budgets tracked, non-blocking per Principle IV) | **WIRED** — Phase 10 Polish: perf-budget tracking for SC-002 (open+parse), SC-003 (keystroke reparse), SC-004 (Quick Open search); benchmarks published in retro but non-blocking in CI |

### Swift (`EmendCore`)

No external dependencies beyond the Rust-compiled `EmendCore.xcframework` (generated via UniFFI).

### Swift (`app/Emend`)

Pure AppKit/SwiftUI; no external package dependencies. All editor transforms (`SmartLists`, `FormattingCommands`, `SyntaxAttributing`, `AutosaveController`, `ConflictController`), workspace sidebar (`WorkspaceModel`, `WorkspaceOutlineView`), preview (`PreviewWebView`, `PreviewModel`, `ScrollSync`), PDF export (`PDFExport`), folder-icon picker, tab model, wiki-link `[[` autocomplete (via `NSTextView` completion), info sidebar (document `stats`/`outline` pull via FFI `OpenDocHandle::stats()` / `outline()` on edit-notification, FR-031a), and typography UI (`TypographyPanel`, font picker using `NSFontManager`, paragraph-style picker) are hand-written pure Swift modules using only Foundation/AppKit/SwiftUI. AI key storage uses macOS Security framework Keychain (`SecKeychain` C APIs). Typography settings persist to `NSUserDefaults` and are synced to the Rust core via `SettingsHandle.set_typography()` on app launch and user preference changes. **Phase 10 additions**: accessibility identifiers (`UIAccessibility` / `AccessibilityIdentifier` on all major UI elements for VoiceOver support and future UI automation).

### Catalogued but Inert (Not Yet Wired)

These are pinned in the workspace `[workspace.dependencies]` but not yet imported by any crate:

| Package | Version | Purpose | Why Inert | Planned For |
|---------|---------|---------|-----------|------------|
| `nucleo` | 0.5 | Fuzzy search / Quick Open ranking (full worker-pool engine; current index uses lighter `nucleo-matcher` only) | Not imported | Phase 2 (US2 — streaming Quick Open driver T073) |

## Package Managers & Build Tools

| Tool | Version | Purpose |
|------|---------|---------|
| `cargo` | 1.85+ | Rust build, test, clippy, fmt |
| `just` | (any) | Task runner; see `justfile` for `build`, `test`, `clippy`, `fmt-check`, `check`, `xcframework` |
| `Xcode` | 16.2+ | Swift build, SwiftUI preview, XCTest |
| `Swift` (compiler) | 6.0+ | Swift 6 strict-concurrency mode (Swift 5 for generated UniFFI bindings) |
| `xcodegen` | (latest) | Generate `.xcodeproj` from `project.yml` (Phase 10 Polish: `.xcodeproj` is source-controlled-agnostic, regenerated on build) |

## Runtime Environment

| Environment | Details |
|-------------|---------|
| **OS Target** | macOS 14.0+ (Sonoma+) |
| **Architecture** | arm64 (Apple Silicon) only |
| **Deployment** | Native .app bundle (single-window macOS application) with hardened runtime and network-client entitlement (Phase 10: added after security review to support BYOM AI; gated by code: zero network without user-configured key) |
| **No Database** | Plain `.md` files on disk; app state in macOS Keychain (for AI API key) and user defaults (including typography settings); attachments stored in note-relative `attachments/` subdirectories |
| **No Network by Default** | Zero outbound network unless AI is configured (via user prefs) AND explicitly invoked by the user (SC-008 / FR-035); entitlements enable capability but code enforces policy |

## Build Profile

```toml
[profile.release]
lto = "thin"
codegen-units = 1
```

Thin LTO for faster builds while retaining optimization; single codegen unit for better inlining.

## Cross-Boundary Semantics

- **Text buffer**: Swift owns the canonical `NSTextStorage`; Rust shadows it with a `ropey::Rope` for off-main-thread queries and incremental tree-sitter highlighting.
- **Coordinates**: FFI boundary uses **UTF-16 code units** (not UTF-8 offsets) to map 1:1 onto `NSRange` and avoid per-keystroke transcoding (research §A2). Link/embed token ranges and task checkbox ranges are all UTF-16.
- **Async wiring**: Rust tokio runtime lives in `emend-ffi`; `emend-core` stays purely synchronous for testability (Constitution V, research §B8).
- **Two highlight/preview engines** (Constitution guardrail): `tree-sitter-md` (editor, incremental, advisory) ≠ `comrak` (preview, authoritative, whole-document). Never unified.
  - Editor highlight: `tree-sitter-md` (split block + inline grammar) runs incrementally on the per-keystroke hot path (≤50 ms budget, SC-003); is advisory-only (does not affect preview rendering).
  - Preview render: `comrak` (CommonMark + GFM + wikilinks + `==highlight==`) runs off-main on demand; outputs authoritative preview HTML with `render.sourcepos` source-line anchors; fenced code blocks colored by `syntect` via `ClassedHTMLGenerator`.
  - Embeds expansion: source-level pre-render pass (`parse/embed.rs`) that replaces `![[Target]]` tokens with the referenced note's Markdown before passing to comrak, so embedded content is parsed in the surrounding document's context.
- **Preview theme CSS**: Syntect's theme dump is vendored (`assets/syntaxes-themes.packdump`); the matching CSS is exported by `preview_theme_css()` FFI and injected by Swift into the WebView template (research §B6).
- **Workspace and file watching**: `emend-core` provides synchronous workspace metadata (`workspace.rs`), incremental search index (`index.rs`), and link/embed/task extraction (`derived.rs`); file watcher (`notify` + `notify-debouncer-full`) runs on a dedicated `std::thread`, not tokio (Constitution V).
- **PDF export**: `WKWebView` renders comrak HTML off-screen; `NSPrintOperation` paginates it to PDF with `@media print` rules from `theme.css` (research §C4).
- **Wiki-link resolution**: Deterministic FR-019a policy in `derived::resolve_wikilink()` — same-directory tiebreak → shallowest path → lexicographically smallest. Returns `None` for unresolved/renamed links (stale links are not auto-rewritten in v1).
- **Attachment storage**: Atomic writes to note-relative `attachments/` subdirectory via `fs::store_attachment()` (reuses `write_atomic_bytes` + `fsync` durability); returns portable Markdown reference for insertion.
- **Task toggling**: Synchronous line-based toggle of `[ ]`↔`[x]` via `derived::toggle_task()`, applied as a full-document edit delta so the shadow Document and Highlighter stay in lock-step.
- **Typography settings (US7)**: A thread-safe in-memory `TypographyStore` (`emend-core::settings`) holds global editor + preview typography (font family, size in points, line height multiplier, paragraph spacing). Values are **clamped** on entry (font size `8..=48` pt, line height `1.0..=3.0`, paragraph spacing `0..=64` pt, blank font family → system default) so broken layouts are impossible. Swift owns persistence (UserDefaults) and replays on launch via `SettingsHandle.set_typography()` (FFI contract §8). The editor (`NSTextView` + `NSParagraphStyle`) applies size and line height directly; the preview (`WKWebView`) applies them via injected CSS (font-size, line-height, margin-bottom). No new dependencies — pure `std::Mutex` in the core, AppKit `NSFontManager`/`NSFont`/`NSParagraphStyle` on the Swift side (built-in).
- **AI streaming (US6)**: `emend-ffi` owns the `reqwest` HTTP client + `tokio` orchestration (per-chunk inactivity timeout, `CancellationToken` + `tokio::select!`). Bytes feed through the core `emend_core::ai::SseParser` (redacting `ApiKey` newtype, max-input guard FR-036a, pure JSON parsing) to the foreign `AiSink` callback. The FFI exports: `summarize_document(OpenDocHandle, AiRequestConfig, AiSink) → Arc<AiHandle>` (cancellable) + `test_ai_config(AiRequestConfig) → bool` (validates endpoint before full request). The core (`ai.rs`) exports: `SseParser`, `ApiKey`, `check_input_size()`, `build_request_body()`, `build_auth_header()` — all pure, zero network.
- **Info sidebar (US6)**: FFI exports `OpenDocHandle::stats()` (word/char/task counts via `derived::stats()`) + `outline()` (headings + line numbers via `derived::outline()`). Live pull via `set_doc_observer()` callback on edit (debounced ≤300ms, FR-031a).

## Phase 10 Polish Artifacts

### Performance Benchmarking (Non-Blocking)

Criterion benchmarks verify perf budgets per Constitution Principle IV (tracked, non-blocking):

- **SC-002** (cold open): `benches/open_doc.rs` — measures core open + initial tree-sitter-md parse on a ~1 MiB / ~10k-line document; p95 ≤ 500 ms nominal, currently overbudget (large-file limitation documented; follow-up: debounce advisory highlight)
- **SC-003** (keystroke reparse): `benches/highlight.rs` — re-highlight one edited line in a large doc; p95 ≤ 50 ms nominal
- **SC-004** (Quick Open search): `benches/quick_open.rs` — rank + stream a query over a 10k-entry index; p95 ≤ 100 ms warm (currently ~2.5 ms — 40× headroom)
- **Results**: Published in Phase 10 retro; budgets are not CI gates but inform future optimization priorities

### Bounded-Memory & Large-File Tests

- `tests/large_file_memory.rs` — open a ~5 MB read-only note and verify bounded-memory usage (emend-core max FR-027a cap); isolates FS I/O via `tempfile`
- `tests/weak_reference_cleanup.rs` — verify `OpenDocHandle` + `AutosaveController` deallocate on close/reload (NFR-005 leak test, no GUI automation)

### Accessibility Identifiers (Phase 10)

All major UI elements tagged with `accessibilityIdentifier` for VoiceOver support:

- `editor.textView` — editor text view
- `sidebar.outline` — workspace outline view
- `quickOpen.searchField` — search input
- `quickOpen.results` — results list
- `quickOpen.result.<name>` — individual result rows
- `infoSidebar.stats` — document stats display
- `infoSidebar.outline` — clickable outline
- `preview.webView` — preview render

(No XCUITest target by design per Constitution VII; identifiers aid VoiceOver and keep the door open for future UI-test lanes outside CI)

### Security Review Findings (Phase 10)

- **Network entitlement**: `com.apple.security.network.client` added after review to support BYOM AI (Phase 8 US6). Privacy enforced in code: `summarize_document`/`test_ai_config` refuse before socket when key is blank (SC-008 / FR-035). Preview WebView still isolated: CSP `connect-src 'none'` + `nonPersistent` + nav-blocking delegate.
- **ATS**: Doesn't apply to `reqwest` raw tokio + Security.framework TLS (not CFNetwork/URLSession); local BYOM `http://localhost` endpoints work without exception. Default-deny preserved.
- **Result**: All findings documented and resolved; no open security gaps.

---

## What Does NOT Belong Here

- Directory structure → `STRUCTURE.md`
- System design patterns → `ARCHITECTURE.md`
- External service integrations → `INTEGRATIONS.md`
- Dev tools (linting, formatting) → `CONVENTIONS.md`
- Test frameworks → `TESTING.md`

---

*This document captures only what executes. Keep it focused on languages, frameworks, and dependencies. See CLAUDE.md for governance and `.sdd/memory/constitution.md` for Principles. Phase 10 Polish final.*
