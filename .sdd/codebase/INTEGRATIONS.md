# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Data Stores

### File System (Primary)

| Service | Type | Purpose | Configuration |
|---------|------|---------|----------------|
| macOS File System | Local Filesystem | Primary note storage (plain `.md` files organized in user-selected folders) | User selects folder(s) via security-scoped bookmarks (research §A4); multiple locations supported (Phase 4 US2) |

**Connection Patterns:**

- **Reading**: `emend_core::fs::read_tolerant` (UTF-8 BOM stripping, CRLF preservation, lossy UTF-8 decode)
- **Writing**: `emend_core::fs::write_atomic` (temp file + fsync + atomic rename + fsync dir for durability)
- **Watching**: `notify` + `notify-debouncer-full` (FS events debounced 100ms, self-write suppression via FileIdCache; wired Phase 4 US2)
- **Indexing**: `workspace.rs` (metadata: locations, folders, file tree) + `index.rs` (incremental in-memory search haystack)
- **Conflict Detection**: conflict model when files modified externally (wired Phase 4 US2)
- **No migration**: Plain Markdown; app-managed state lives in `NSUserDefaults` and Keychain

### Workspace & Location Management (Phase 4 US2)

| Component | Type | Purpose | Implementation |
|-----------|------|---------|-----------------|
| `workspace.rs` | In-memory metadata | Workspace state: set of [`Location`]s, per-path favorites, pins, custom folder icons, manual child order | Pure Rust, no FFI/tokio, O(1) file operations via collision-safe naming (`free_name` scheme) |
| `index.rs` | Search haystack | Incremental in-memory index backed by `nucleo-matcher`: fuzzy matching for Quick Open + wiki-link resolution | Synchronous, arena-based `Vec<Option<Entry>>` + path/name maps; O(1) event dispatch; handles tens-of-thousands of files (FR-018) |
| `watcher.rs` | FS event stream | Live file-watcher: detects creates, deletes, renames, moves on disk; surfaces via `WatchHandle` | Thread-based (not tokio) debouncer; conflict state machine; optional user prompt on external changes |

**Conflict Handling (Phase 4 US2):**

When a file is externally modified while open in Emend, the watcher detects it (after debounce) and transitions to `ConflictState`. The UI prompts the user: **keep mine** / **reload** / **merge** (detailed merge logic TBD Phase 1). Resolution updates the document and clears the conflict flag.

**Location Identity (NFR-007):**

Path identity via canonicalization: symlinks resolved, `..` eliminated; case-sensitivity delegated to the host filesystem. Symlink cycles bounded by recursion depth + visited-path set.

### App Preferences & Configuration

| Service | Type | Purpose | Configuration |
|---------|------|---------|----------------|
| macOS Keychain | Secure Storage | AI API key (transient to Rust, never logged/persisted) | macOS Security framework (`SecKeychain` C APIs via Swift FFI) |
| `NSUserDefaults` | Local Preferences | Editor state, location tree favorites, folder icons, typography settings, AI config metadata, tab state | Standard macOS app preferences (per-user, non-synced) |

---

## Authentication & Authorization

### AI API Key Management

- **Storage**: macOS Keychain (encrypted by OS)
- **Access Pattern**: Swift app queries Keychain → passes plaintext to Rust (transient) → used in HTTP headers only → never logged, never written to disk
- **Privacy**: Redacted in HTTP client logs (NFR-006)
- **User Control**: Zero network unless AI is configured AND explicitly invoked

**Code Location**: 

- `crates/emend-ffi/` — async scaffolding for cancellable AI requests
- `crates/emend-core/` — `ai` module (phase 1, FR-023)
- `app/Emend/Emend/Platform/` — Keychain bridge (phases 1–2)

---

## External APIs

### AI (OpenAI-Compatible)

| Provider | Purpose | Base URL Config | Auth | Rate Limits |
|----------|---------|-----------------|------|-------------|
| OpenAI or any OpenAI Chat Completions–compatible endpoint | BYOM (Bring Your Own Model): user supplies base URL + model ID | User-provided `AI_BASE_URL` env/prefs | Bearer token (from Keychain) | Configured by the endpoint provider |

**Connection Details:**

- **Protocol**: OpenAI Chat Completions API (HTTP, JSON, SSE streaming)
- **Client**: `reqwest` with SSE support (`stream` feature, not yet wired)
- **Request Shape**: Standard `/v1/chat/completions` POST
- **Response**: Server-Sent Events (delimited `data: …` lines)
- **Streaming Sink**: Foreign-trait callback (research §A1) — Rust collects deltas, Swift renders in real-time
- **Cancellation**: `tokio::sync::CancellationToken` (user can cancel mid-stream)
- **Error Handling**: Captured `EmendError::AiHttp`, `EmendError::AiStreamMalformed`, `EmendError::AiTimeout`, `EmendError::AiCancelled`
- **Privacy**: Request body includes note excerpt (user-configured max length); no ambient document indexing sent without consent

**Implementation Status**: Planned phase 1 (US4 — AI completion) / (US5 — inline editing)

**Code Location** (when wired):

- `crates/emend-core/ai/` — HTTP client + SSE parser
- `crates/emend-ffi/handles/` — async scaffolding + foreign traits
- `app/Emend/Emend/AI/` — UI layer (model selection, key setup, streaming render)

---

### File Watcher (macOS Native, Phase 4 US2)

| Service | Purpose | Configuration | Failure Mode |
|---------|---------|----------------|--------------|
| macOS File System Events (via `notify` + `notify-debouncer-full` crates) | Detect external note edits and FS changes (create/delete/rename/move); reload + alert user | Debounced to 100ms; self-write suppression via FileIdCache (FR-006a); conflict state machine | Silent miss if event queue overflows; user can manually refresh (FR-006c). On conflict: user chooses keep/reload/merge |

**Implementation Status**: Phase 4 US2 (wired); replaces manual refresh

**Code Location:**

- `crates/emend-core/src/watcher.rs` — FS event loop + conflict detection
- `crates/emend-ffi/src/watcher.rs` — FFI handle + callback scaffolding
- `app/Emend/Emend/Editor/ConflictController.swift` — UI prompt (keep mine / reload / merge)

---

### Workspace Search & Quick Open (Phase 4 US2)

| Service | Purpose | Implementation | Status |
|---------|---------|-----------------|--------|
| Workspace search index (`nucleo-matcher`) | Fast fuzzy matching for Quick Open + wiki-link resolution (search 10K+ notes instantly) | In-memory arena-based `Index`; synchronous `query()` ranks by basename ≫ path; O(1) event dispatch on create/rename/delete | **WIRED** Phase 4 US2; incremental updates to `index.rs` |

**Ranking:**

- Fuzzy subsequence match on **basename** (primary, boosted) and **location-relative path** (secondary)
- Score = better of the two; shorter relative path breaks ties
- `nucleo-matcher::Matcher` (no worker pool; synchronous like the index)

**Code Location:**

- `crates/emend-core/src/index.rs` — `Index` struct + `query()` + incremental updates
- `crates/emend-ffi/src/index.rs` — FFI `IndexHandle` + `search_index_query()` export
- `app/Emend/Emend/Tabs/TabModel.swift` + `app/Emend/Emend/Sidebar/WorkspaceModel.swift` — UI consumers (Quick Open, tree rendering)

---

## Markdown Processing & Preview Rendering (Phase 6 US4)

### Preview Engine (Authoritative)

| Component | Purpose | Technology | Status |
|-----------|---------|-----------|--------|
| Preview HTML rendering | Whole-document CommonMark + GFM (tables, tasklist, strikethrough, autolinks) + native extensions (wikilinks, `==highlight==`→`<mark>`) with scroll-sync anchors | `comrak` 0.52 with `render.sourcepos` → `data-line` attributes (research §B1, §C3) | **WIRED** — Phase 6 US4; `crates/emend-core/src/parse/preview.rs`; FFI export `OpenDocHandle::render_preview_html()` |

**Key Properties:**

- **Authoritative & complete**: Renders whole document; **not incremental**; distinctly separate from the editor's incremental tree-sitter highlighter (Constitution guardrail).
- **Deterministic**: Pure `&str -> String` transform; no IO, no network, no async (verified by `tests/preview_offline.rs`).
- **Scroll-sync anchors**: Each block emits `data-line="<line>"` for editor ↔ preview synchronization (research §C3).
- **No remote loads**: Embedded URLs are emitted literally; never dereferenced (verified by CSP on Swift side).
- **Code highlighting**: Fenced code blocks colored by syntect (see below).

**Code Location:**

- Core engine: `crates/emend-core/src/parse/preview.rs` — `render_preview_html()`
- FFI: `crates/emend-ffi/src/document.rs` — method `OpenDocHandle::render_preview_html()` 
- Swift: `app/Emend/Emend/Preview/PreviewWebView.swift` — loads HTML into `WKWebView`

### Code Block Syntax Highlighting (Preview)

| Component | Purpose | Technology | Status |
|-----------|---------|-----------|--------|
| Fenced code block coloring | Highlight code in `` ```lang `` blocks with 20+ languages (Rust, Python, JavaScript, etc.) | `syntect` 5.3 with `ClassedHTMLGenerator` + theme CSS; loads vendored binary `SyntaxSet`/`ThemeSet` dump (`assets/syntaxes-themes.packdump`) | **WIRED** — Phase 6 US4; `crates/emend-core/src/parse/code_highlight.rs`; plugged into comrak via `SyntaxHighlighterAdapter` |

**Key Properties:**

- **Pure Rust**: `fancy-regex` backend (no C `onig` dependency).
- **Vendored dump**: One-time serialization of default `SyntaxSet` + `ThemeSet` committed as `assets/syntaxes-themes.packdump`; loaded via `OnceLock` at first use (not parsed per document).
- **Theme CSS**: Exported as `preview_theme_css()` FFI; injected by Swift into preview template.
- **No remote syntax/theme loads**: Syntax definitions and themes are built-in to the dump.

**Regeneration:**

- Run `cargo run --example gen_syntect_dump` to refresh the vendored dump when updating `syntect` version or desired language support.
- Output written to `crates/emend-core/assets/syntaxes-themes.packdump`; commit and re-run `just xcframework`.

**Code Location:**

- Core engine: `crates/emend-core/src/parse/code_highlight.rs` — `EmendSyntectAdapter`, `theme_css()`
- Example: `crates/emend-core/examples/gen_syntect_dump.rs` — regenerate the vendored dump
- Assets: `crates/emend-core/assets/syntaxes-themes.packdump` (binary dump), `assets/preview-theme.css` (exported CSS)
- FFI: `crates/emend-ffi/src/document.rs` — `preview_theme_css()` free function
- Swift: injected into `app/Emend/Emend/Resources/preview/template.html` via theme CSS

---

## Syntax Highlighting

### Editor Highlighting (Internal)

| Component | Purpose | Technology | Status |
|-----------|---------|-----------|--------|
| Editor live-syntax highlight | Real-time visual feedback as user types (bold, italic, headings, code, etc.); advisory only — does not affect preview rendering | `tree-sitter-md` (split block + inline Markdown grammar) with incremental tree-sitter runtime | **WIRED** — Phase 3 US1 (Editor MVP); lives in `crates/emend-core/src/parse/highlight.rs` |

**Performance:**
- Incremental per-keystroke reparses (≤50 ms typing budget, SC-003)
- Does NOT block the main thread; UTF-16 coordinate bridging via `ropey::Rope` mirror
- Advisory styling only; preview rendering uses separate `comrak` engine (not unified)

**Code Location:**
- Highlighter struct: `crates/emend-core/src/parse/highlight.rs`
- FFI export: `crates/emend-ffi/src/document.rs` — `OpenDocHandle::highlight_spans()`
- Swift integration: `app/Emend/Emend/Editor/SyntaxAttributing.swift` (applies spans to `NSTextView`)

---

## Preview Rendering & Display (Phase 6 US4)

### Web View

| Service | Purpose | Configuration | Security |
|---------|---------|----------------|----------|
| `WKWebView` (macOS AppKit) | Render comrak preview HTML + bundled Mermaid diagrams + KaTeX math | `nonPersistent` session, navigation delegate (blocks remote loads), CSP header, `loadFileURL` for bundled assets | No remote scripts/images/stylesheets; all resources bundled in app |

**Properties:**

- **Rendering**: Displays `render_preview_html()` output with embedded `<script data-mermaid>` and `<math>` tags from comrak.
- **Bundled assets**: Mermaid.js 11.15, KaTeX 0.17 + fonts under `app/Emend/Emend/Resources/preview/`; loaded via `loadFileURL`.
- **Scroll sync**: `bridge.js` handles editor ↔ preview scroll linking via `data-line` anchors (research §C3).
- **CSP**: Blocks inline `<script>`, external `src=`, and all `fetch()`; enforced by `theme.css` `<meta>` headers.
- **User data only**: Renders note HTML; no AI-generated content, no ambient telemetry.

**Implementation Status**: Phase 6 US4 (wired preview); scroll-sync in `PreviewModel.swift`

**Code Location:**

- Swift WKWebView: `app/Emend/Emend/Preview/PreviewWebView.swift`
- Model/sync: `app/Emend/Emend/Preview/PreviewModel.swift` + `app/Emend/Emend/Preview/ScrollSync.swift`
- Template & assets: `app/Emend/Emend/Resources/preview/` (template.html, theme.css, bridge.js, mermaid.min.js, katex/)

---

### Diagram & Math Rendering (Bundled)

| Library | Format | Bundling | Status |
|---------|--------|----------|--------|
| Mermaid.js 11.15 | Diagrams (flowchart, sequence, state, class, etc.) | Bundled as `mermaid.min.js` in app; loaded into `WKWebView` via `loadFileURL`; no CDN | **WIRED** — Phase 6 US4 (embedded in preview; direct CDN not used) |
| KaTeX 0.17 | LaTeX-style math (inline `$…$` and block `$$…$$`) | Bundled as JS + CSS + fonts in `app/Emend/Emend/Resources/preview/katex/`; injected into preview template | **WIRED** — Phase 6 US4 (embedded in preview; direct CDN not used) |

**No external CDN loads** — all assets are shipped with the app and loaded from the local filesystem via `loadFileURL` (offline-capable, verifiable by `tests/preview_offline.rs`).

**Code Location:**

- Assets: `app/Emend/Emend/Resources/preview/mermaid.min.js`, `app/Emend/Emend/Resources/preview/katex/`
- Injected by: `app/Emend/Emend/Resources/preview/template.html`
- Versions: tracked in `app/Emend/Emend/Resources/preview/VERSIONS.md`

---

## File Export

### PDF Export (Phase 6 US4)

| Service | Purpose | Implementation | Status |
|---------|---------|----------------|--------|
| `WKWebView` off-screen render + `NSPrintOperation` | Export the live preview to a paginated PDF file | Dedicated off-screen `WKWebView`; load comrak HTML; run Mermaid JS; paginate with `NSPrintOperation` using `@media print` CSS rules from `theme.css` (research §C4) | **WIRED** — Phase 6 US4; `app/Emend/Emend/Preview/PDFExport.swift` |

**Key Properties:**

- **Multi-page**: Uses `NSPrintOperation` (not `WKWebView.createPDF`, which ignores pagination).
- **High fidelity**: Identical rendering to on-screen preview; Mermaid diagrams and KaTeX render before PDF generation.
- **Print CSS**: `theme.css` defines `@media print` rules for page breaks, margins, and font sizing.
- **Off-screen**: Render happens in a hidden window far off-screen so WebKit doesn't throttle layout (Apple forums 700418/705138).
- **User-selected path**: Save dialog lets user choose output location.

**Failure Modes:**

- Template missing (bundle error): `PDFExport.Failure.templateMissing`
- Render failed (HTML parsing / JS error): `PDFExport.Failure.renderFailed(detail)`
- Print failed (I/O or user cancel): `PDFExport.Failure.printFailed`

**Code Location:**

- Export logic: `app/Emend/Emend/Preview/PDFExport.swift` — `PDFExport.export(html:css:to:)`
- Called by: `app/Emend/Emend/Preview/PreviewModel.swift` — `exportPDF()` action
- Print CSS: `app/Emend/Emend/Resources/preview/theme.css` — `@media print` rules

---

## System Integration

### macOS System Services

| Service | Purpose | API | Status |
|---------|---------|-----|--------|
| Security framework (Keychain) | Secure API key storage | `SecKeychain` C APIs via Swift FFI | **WIRED** — core to AI key management |
| NSTextView / TextKit 2 | Native editor surface with split-paragraph storage | AppKit / SwiftUI integration + `MarkdownEditorView` NSViewRepresentable | **WIRED** — phase 0 skeleton, Phase 3 US1 MVP (editor transforms) |
| NSOutlineView | Folder/file tree sidebar with expansion/collapse | AppKit | **WIRED** — Phase 4 US2 (`WorkspaceOutlineView.swift`) |
| WKWebView | Preview rendering + scroll-sync + PDF export | WebKit framework | **WIRED** — Phase 6 US4 (`PreviewWebView.swift`, `PDFExport.swift`) |
| NSPrintOperation | Paginated PDF export | AppKit | **WIRED** — Phase 6 US4 (`PDFExport.swift`) |
| Pasteboard | Copy/paste support | `NSPasteboard` API | Planned phase 1–2 |
| Security-scoped bookmarks | Persistent folder access permission | AppKit / Swift Security Framework | **WIRED** — Phase 0, research §A4; resolves on launch to a usable filesystem path |

---

## Environment Variables

Critical variables for integration (none required for basic operation; all optional):

| Variable | Required | Purpose | Example | Set By |
|----------|----------|---------|---------|--------|
| `EMEND_FOLDER_PATH` | No | Workspace folder path (normally user-selected via dialog) | `/Users/alice/Documents/Notes` | Security-scoped bookmark (user grants access) |
| `OPENAI_API_KEY` or custom | No | AI API key (if user configures AI) | `sk-…` | Keychain (transient in Rust) |
| `OPENAI_API_BASE` or custom | No | AI endpoint base URL (defaults to OpenAI, user can override) | `https://api.openai.com/v1` | User prefs / `NSUserDefaults` |
| `RUST_LOG` | No | Debug logging level (dev-only) | `debug` | Env or launch args |

No environment file (`.env`) is read by the app — all configuration is stored in Keychain or `NSUserDefaults`.

---

## Failure Modes & Resilience

| Integration | Failure | App Behavior |
|-------------|---------|--------------|
| File system unavailable | Note folder inaccessible | Graceful error prompt; user can select a different folder |
| External file modified | File on disk changes while edited in Emend | Watcher debouncer (100ms) detects; conflict state machine prompts user (keep mine / reload / merge — Phase 4 US2) |
| File watcher overflow | FS event queue saturates (many rapid changes) | Watcher silently misses events; user can manually refresh (FR-006c) |
| Symlink cycle | User adds folder with symlink loops | Bounded by `max_depth` + visited-path canonicalization set (NFR-007) |
| Collision on create/rename/move | Target basename already exists | Collision-safe naming appends space + lowest free integer (`note 2.md` scheme, FR-004a) |
| AI not configured | User invokes AI feature with no API key | Return `EmendError::AiNotConfigured`; UI shows setup prompt |
| AI endpoint unreachable | Network error or 5xx response | `EmendError::AiHttp` with redacted status; user can retry or abort |
| AI SSE malformed | Broken event stream | `EmendError::AiStreamMalformed`; cancel the request gracefully |
| AI timeout | Request exceeds timeout | `EmendError::AiTimeout`; user can retry |
| User cancels AI request | Cancel token triggered mid-stream | `EmendError::AiCancelled`; clean shutdown of the stream task |
| Preview render fails | Unexpected comrak error | Fall back to plain-text display or re-render |
| PDF export: template missing | App bundle error | `PDFExport.Failure.templateMissing`; user sees error dialog |
| PDF export: render failed | WebView fails to load HTML or JS error | `PDFExport.Failure.renderFailed(detail)`; user sees error with details |
| PDF export: print failed | I/O error or user cancel | `PDFExport.Failure.printFailed`; gracefully dismiss |
| Preview WebView crashes | Internal WebKit failure | Error logged; fall back to plain-text preview or re-render |
| Syntax highlighting lagging | Tree-sitter reparse exceeds 50ms budget | Fall back to previous highlight spans; no visual stutter (advisory-only styling) |

---

## What Does NOT Belong Here

- Internal code architecture → `ARCHITECTURE.md`
- Testing infrastructure → `TESTING.md`
- Security policies → `SECURITY.md`
- Dependency versions and selection → `STACK.md`
- File system operations / Keychain access patterns → See `crates/emend-core/src/fs.rs`, `crates/emend-core/src/workspace.rs`, `crates/emend-core/src/watcher.rs`, `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift`
- Editor-UI transforms (smart lists, formatting) → `CONVENTIONS.md`
- Markdown engines (two-engine architecture) → `CONVENTIONS.md` (patterns) & `ARCHITECTURE.md` (design)

---

*This document maps external service dependencies and protocols. Update when adding new integrations or modifying AI endpoints, file watching, workspace management, preview rendering, or PDF export.*
