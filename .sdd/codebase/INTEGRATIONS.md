# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (US6 additions: AI summary streaming, info sidebar stats/outline, BYOM key storage); 2026-06-17 (US7 additions: typography settings storage + clamping)

## Data Stores

### File System (Primary)

| Service | Type | Purpose | Configuration |
|---------|------|---------|----------------|
| macOS File System | Local Filesystem | Primary note storage (plain `.md` files organized in user-selected folders); attachments stored in note-relative `attachments/` subdirectories | User selects folder(s) via security-scoped bookmarks (research §A4); multiple locations supported (Phase 4 US2); attachments keyed off note's disk path (Phase 7 US5) |

**Connection Patterns:**

- **Reading**: `emend_core::fs::read_tolerant` (UTF-8 BOM stripping, CRLF preservation, lossy UTF-8 decode)
- **Writing notes**: `emend_core::fs::write_atomic` (temp file + fsync + atomic rename + fsync dir for durability)
- **Writing attachments**: `emend_core::fs::store_attachment()` (atomic write to `attachments/` subdir with collision-safe naming, e.g., `image 2.png`; FFI export `/uniffi:store_attachment` — free function; Phase 7 US5)
- **Watching**: `notify` + `notify-debouncer-full` (FS events debounced 100ms, self-write suppression via FileIdCache; wired Phase 4 US2)
- **Indexing**: `workspace.rs` (metadata: locations, folders, file tree) + `index.rs` (incremental in-memory search haystack with `nucleo-matcher`)
- **Link/embed/task extraction**: `derived.rs` (pure Markdown scanning, deterministic wiki-link resolution policy, embed expansion guards, task checkbox toggling; Phase 7 US5)
- **Conflict Detection**: conflict model when files modified externally (wired Phase 4 US2)
- **No migration**: Plain Markdown; app-managed state lives in `NSUserDefaults` and Keychain

### Workspace & Location Management (Phase 4 US2)

| Component | Type | Purpose | Implementation |
|-----------|------|---------|-----------------|
| `workspace.rs` | In-memory metadata | Workspace state: set of [`Location`]s, per-path favorites, pins, custom folder icons, manual child order | Pure Rust, no FFI/tokio, O(1) file operations via collision-safe naming (`free_name` scheme) |
| `index.rs` | Search haystack | Incremental in-memory index backed by `nucleo-matcher`: fuzzy matching for Quick Open + wiki-link suggestions (Phase 7 US5) | Synchronous, arena-based `Vec<Option<Entry>>` + path/name maps; O(1) event dispatch; handles tens-of-thousands of files (FR-018) |
| `watcher.rs` | FS event stream | Live file-watcher: detects creates, deletes, renames, moves on disk; surfaces via `WatchHandle` | Thread-based (not tokio) debouncer; conflict state machine; optional user prompt on external changes |

**Conflict Handling (Phase 4 US2):**

When a file is externally modified while open in Emend, the watcher detects it (after debounce) and transitions to `ConflictState`. The UI prompts the user: **keep mine** / **reload** / **merge** (detailed merge logic TBD Phase 1). Resolution updates the document and clears the conflict flag.

**Location Identity (NFR-007):**

Path identity via canonicalization: symlinks resolved, `..` eliminated; case-sensitivity delegated to the host filesystem. Symlink cycles bounded by recursion depth + visited-path set.

### Derived Data: Links, Embeds, Tasks (Phase 7 US5)

| Component | Type | Purpose | Implementation |
|-----------|------|---------|-----------------|
| `derived.rs` | Per-document analytics | Extract `[[wiki links]]` and `![[embeds]]`, resolve links deterministically, toggle task checkboxes, compute document statistics (word/char/task counts), and generate document outline (headings + line numbers) | Pure `std` — no tokio/uniffi. Five main functions: (1) `extract_links()` scans source for link/embed tokens with UTF-16 ranges; (2) `resolve_wikilink()` applies FR-019a tiebreak policy (same-directory → shallowest → lex-smallest) over index candidates, returns `None` for stale links; (3) `toggle_task()` flips `[ ]`↔`[x]` on a line; (4) `stats()` (US6) counts words/chars/tasks; (5) `outline()` (US6) extracts headings with line numbers |
| `parse/embed.rs` | Embed expansion | Recursively splice embedded note sources into the preview document before comrak rendering (FR-021/021a) | Pure source-level pass: replaces `![[Target]]` with target's source text. Two termination guards: (1) cycle detection (stack-based `on_stack` set); (2) depth bound ([`MAX_EMBED_DEPTH`] = 8, research §D). Embeds are a reading aid — unresolved/cyclic/out-of-depth tokens rendered as placeholders, not errors |

**FFI Exports (Phase 7 US5 + Phase 8 US6):**

- [`OpenDocHandle::links()`](crates/emend-ffi/src/document.rs) — returns all `[[…]]` + `![[…]]` tokens in the current buffer with UTF-16 ranges (FFI contract §4)
- [`OpenDocHandle::toggle_task(at: U16Range)`](crates/emend-ffi/src/document.rs) — toggle checkbox on the line containing the UTF-16 offset; applies edit via full-document delta so shadow Document + Highlighter stay in lock-step
- [`OpenDocHandle::render_preview_html_resolving(workspace:)`](crates/emend-ffi/src/document.rs) — preview engine parameterized by a resolver closure (so embeds can be expanded mid-render via workspace index lookup)
- [`OpenDocHandle::stats()`](crates/emend-ffi/src/document.rs) — (US6) returns `DocStats` (word count, character count, task N-of-M); called by info sidebar on edit notification (FR-031a)
- [`OpenDocHandle::outline()`](crates/emend-ffi/src/document.rs) — (US6) returns `Vec<OutlineItem>` (headings with line numbers + byte offset for click→scroll); called by info sidebar on edit notification (FR-031a)
- [`OpenDocHandle::set_doc_observer(observer: Arc<dyn DocObserver>)`](crates/emend-ffi/src/document.rs) — (US6) register a Rust-owned callback that fires ≤300ms after an edit, triggering Swift info-sidebar re-pull of `outline`/`stats`/`links` (FFI contract §4 `on_derived_changed`, FR-031a)
- [`WorkspaceHandle::resolve_wikilink(from_note, raw_target)`](crates/emend-ffi/src/workspace.rs) — resolve a `[[link]]` using FR-019a policy, returns `Option<String>` absolute path or `None`
- [`WorkspaceHandle::wikilink_suggestions(prefix, limit)`](crates/emend-ffi/src/workspace.rs) — autocomplete suggestions for `[[` (FFI contract §5 `wikilink_suggestions`; FR-020); returns ranked [`SearchHit`]s from the index
- [`store_attachment(note_path, bytes, suggested_name)`](crates/emend-ffi/src/workspace.rs) — free function (not a workspace method); writes bytes atomically to `note_path`'s parent dir's `attachments/` subdir with collision-safe naming; returns portable Markdown reference (FFI contract §2; FR-013/013a)

**App-side Integration (Swift):**

- `NSTextView` completion popup on `[[` for wiki-link autocomplete (calls `WorkspaceHandle.wikilink_suggestions()`)
- Clickable link tokens (`[[Target]]`) resolve via `WorkspaceHandle.resolve_wikilink()` to navigate between notes
- Dragged media files trigger `store_attachment()` and insert the returned reference into the editor
- Checkable task lines (`- [ ]` / `- [x]`) call `OpenDocHandle.toggle_task()` on click, auto-update the editor
- Preview renderer calls `render_preview_html_resolving()` to expand embeds before display
- Info sidebar (US6, FR-031a) pulls `stats()` + `outline()` live by registering a `DocObserver` with `set_doc_observer()` and re-pulling on notification with ≤300ms latency

### App Preferences & Configuration

| Service | Type | Purpose | Configuration |
|---------|------|---------|----------------|
| macOS Keychain | Secure Storage | AI API key (transient to Rust, never logged/persisted) | macOS Security framework (`SecKeychain` C APIs via Swift FFI); user enters key once in app settings; app retrieves plaintext for Rust, wraps it in redacting `ApiKey` newtype, uses only on `Authorization` header, never logs |
| `NSUserDefaults` | Local Preferences | Editor state, location tree favorites, folder icons, **typography settings** (font family, font size, line height, paragraph spacing), AI config metadata (base URL, model ID, request timeout, max input size), tab state | Standard macOS app preferences (per-user, non-synced); typography persisted and replayed to core on launch via `SettingsHandle.set_typography()` |

### Typography Settings (US7)

| Component | Type | Purpose | Implementation |
|-----------|------|---------|-----------------|
| `settings.rs` (emend-core) | In-memory store | Global `TypographySettings` (font family, font size in points, line height multiplier, paragraph spacing in points); thread-safe `TypographyStore` with `get()`/`set()` | Pure `std::Mutex`; values clamped on `set()` so broken layouts impossible; no persistence layer (Swift owns persistence via `NSUserDefaults` and replays on launch); size `8..=48` pt, line height `1.0..=3.0`, paragraph spacing `0..=64` pt, blank font family → system default |
| `settings.rs` (emend-ffi) | FFI projection | `SettingsHandle` object (Arc-wrapped) exported to Swift; `get_typography()` / `set_typography()` (FFI contract §8, FR-038/FR-039) | `#[uniffi::Record]` mirror of core type; exhaustive `From` conversions; clamping idempotent; `set_typography` infallible (out-of-range values clamped, not rejected); new_settings() constructor |

**FFI Exports (US7, FFI contract §8):**

- [`new_settings() → Arc<SettingsHandle>`](crates/emend-ffi/src/settings.rs) — construct a fresh handle seeded with system-appropriate defaults; called once per app session
- [`SettingsHandle::get_typography() → TypographySettings`](crates/emend-ffi/src/settings.rs) — returns current settings (always in range); infallible
- [`SettingsHandle::set_typography(settings: TypographySettings) → Result<(), FfiError>`](crates/emend-ffi/src/settings.rs) — update settings (clamped into sane bounds); out-of-range inputs repaired, not rejected; only fails on poisoned lock (unreachable, NFR-003)

**App-side Integration (Swift, US7):**

- `TypographyPanel.swift` — settings UI (font picker via `NSFontManager`, size slider, line height slider, paragraph spacing slider)
- `EditorCoordinator.swift` — on `SettingsHandle` callback, applies new size + line height to `NSTextView` via `NSParagraphStyle` (first paragraph margin, etc.)
- `PreviewWebView.swift` — on `SettingsHandle` callback, injects CSS (`font-size`, `line-height`, `margin-bottom`) into preview template
- Launch sequence — `AppDelegate`/`AppState` initializes `SettingsHandle`, reads persisted typography from `NSUserDefaults`, calls `set_typography()` to seed the core, then binds to the handle for live updates

**Clamping & Safety (US7):**

- **Font size**: `0` or negative → 8 pt (minimum readable); `9999` → 48 pt (maximum sensible); `NaN`/`±∞` → 14 pt (default)
- **Line height**: `0` or `< 1.0` → 1.0 (single-spacing minimum to avoid crush); `> 3.0` → 3.0 (triple-spacing maximum); `NaN`/`±∞` → 1.4 (default 1.4×)
- **Paragraph spacing**: negative → 0 pt (no upward spacing); `> 64` → 64 pt (caps excessive gaps); `NaN`/`±∞` → 8 pt (default)
- **Font family**: blank/whitespace → `-apple-system` (system font, resolves to SF on both AppKit and CSS sides)
- Clamping applied **on every `set_typography()` call** so `get_typography()` can never return an unclamped value

**Data Flow:**

1. User changes font size in settings UI → Swift updates `NSUserDefaults`
2. Swift calls `SettingsHandle.set_typography()` with new `TypographySettings`
3. Core clamps the value, stores in `TypographyStore`
4. `get_typography()` returns the clamped value
5. Editor listens to changes, updates `NSTextView` + `NSParagraphStyle`
6. Preview listens to changes, injects new CSS into `WKWebView`
7. On app relaunch: Swift reads persisted `TypographySettings` from `NSUserDefaults`, calls `set_typography()` to replay into core (core never persists, Swift owns persistence per US2 guardrail)

---

## Authentication & Authorization

### AI API Key Management (US6, FR-035)

- **Storage**: macOS Keychain (encrypted by OS), user-provided `SecKeychain` secure storage via Security framework
- **Access Pattern**: Swift app queries Keychain → passes plaintext to Rust as transient `String` → wraps in redacting `ApiKey` newtype (NFR-006) → used ONLY in HTTP `Authorization: Bearer` header → never logged, never written to disk
- **Privacy**: `ApiKey` Debug and Display both render `***`; exposed only via explicit `.expose()` method; zero network unless AI is configured AND explicitly invoked (SC-008 / FR-035)
- **User Control**: Zero network unless AI is configured AND invoked by user; info sidebar (FR-031a) does NOT send data to AI — only manual summarize (US6 FR-032) sends document excerpt
- **Error Handling**: Blank/missing key → `EmendError::AiNotConfigured` before any socket (FR-036a / SC-008); redacted in error responses

**Code Location**: 

- `crates/emend-core/src/ai.rs` — `ApiKey` newtype (pure), `check_input_size()` (FR-036a), `SseParser`, request builders
- `crates/emend-ffi/src/ai.rs` — HTTP orchestration (`summarize_document`, `test_ai_config`, `AiHandle`, `AiRequestConfig`), per-chunk timeout, cancellation token
- `app/Emend/Emend/Platform/KeychainStore.swift` — Keychain bridge (Security framework)

---

## External APIs

### AI (OpenAI-Compatible, BYOM, US6)

| Provider | Purpose | Base URL Config | Auth | Rate Limits |
|----------|---------|-----------------|------|-------------|
| OpenAI or any OpenAI Chat Completions–compatible endpoint | BYOM (Bring Your Own Model): user supplies base URL + model ID; used for document summary (FR-032, US6) via streamed `/v1/chat/completions` | User-provided `base_url` in app settings (NSUserDefaults); no environment variable | Bearer token from Keychain (transient to Rust, redacted) | Configured by the endpoint provider |

**Connection Details (US6, FR-032/035/036/036a, NFR-006, SC-008/SC-009):**

- **Protocol**: OpenAI Chat Completions API (HTTP, JSON, SSE streaming)
- **Client**: `reqwest` 0.13 with SSE support (`stream` + `native-tls` features, emend-ffi only)
- **Request Shape**: Standard `/v1/chat/completions` POST with `messages` (system + user), `model`, `stream=true`
- **Response**: Server-Sent Events (delimited `data: {json}` lines); each delta → `AiSink::on_token(delta)` callback
- **Streaming Sink**: Foreign-trait `AiSink` callback (research §A1) — Rust collects deltas, Swift renders in real-time; exactly one terminal: `on_done(full_text)` or `on_error(err)`
- **Cancellation**: `tokio::sync::CancellationToken` + `tokio::select!` (user can cancel mid-stream via `AiHandle::cancel()`, NFR-002, FR-036a)
- **Per-chunk timeout**: `tokio::time::timeout` on each streamed chunk (inactivity guard, not whole-request deadline; research §B5)
- **Error Handling**: Captured in `EmendError` variants: `AiNotConfigured` (no key, before socket), `AiOversizedInput` (exceeds max-input-bytes, before socket, FR-036a), `AiHttp` (network/5xx, redacted status), `AiStreamMalformed` (broken SSE), `AiTimeout` (chunk inactivity), `AiCancelled` (user cancelled)
- **Privacy & Gating (SC-008)**: Max-input guard ([`check_input_size`]) enforced BEFORE socket; redacting `ApiKey` newtype; blank key → error before send; no info-sidebar derived data sent without user action (only explicit summarize → user-configured max excerpt sent)
- **Input Guard (FR-036a)**: Document text truncated to `max_input_bytes` (user configurable, default TBD); oversized → `AiOversizedInput` before request
- **System Prompt**: Default hardcoded: "You are a concise assistant. Summarize the following Markdown document in a short paragraph. Output only the summary, no preamble." (testable, in `emend_core::ai::DEFAULT_SUMMARY_SYSTEM_PROMPT`)

**FFI Exports (FFI contract §7, US6):**

- [`summarize_document(handle: OpenDocHandle, cfg: AiRequestConfig, sink: Arc<dyn AiSink>) → Arc<AiHandle>`](crates/emend-ffi/src/ai.rs) — Start streaming AI summary of a document; returns cancellable handle. Validates: key not blank (AiNotConfigured), input size OK (AiOversizedInput), before any socket. Snapshot document, send excerpt to `/v1/chat/completions`, stream deltas to sink, terminal via `on_done`/`on_error`.
- [`test_ai_config(cfg: AiRequestConfig, api_key: String) → bool`](crates/emend-ffi/src/ai.rs) — Validate AI endpoint by posting empty message; returns true if 2xx, false on error (used by app settings UI to verify key/URL before saving, FR-037)

**Core Exports (pure, no-network, testable):**

- [`SseParser`](crates/emend-core/src/ai.rs) — Line-based Server-Sent-Events parser; feeds chunks, emits complete tokens via `next()` iterator
- [`ApiKey`](crates/emend-core/src/ai.rs) — Redacting newtype (Debug/Display → `***`); `.expose()` only way to read secret
- [`check_input_size(text: &str, max_bytes: u64) → Result<(), EmendError::AiOversizedInput>`](crates/emend-core/src/ai.rs) — FR-036a: reject input before socket if exceeds max
- [`build_request_body(text: &str, system_prompt: &str, model: &str) → String`](crates/emend-core/src/ai.rs) — Pure JSON builder for Chat-Completions request
- [`build_auth_header(key: &ApiKey) -> String`](crates/emend-core/src/ai.rs) — Pure header builder (calls `.expose()` once)
- [`chat_completions_url(base_url: &str) -> String`](crates/emend-core/src/ai.rs) — Constructs `/v1/chat/completions` endpoint

**Implementation Status**: Phase 8 US6 (wired); emend-ffi streaming orchestration + core pure SSE/JSON/key logic

**Code Location**:

- `crates/emend-core/src/ai.rs` — SSE parser, ApiKey, guards, request builders (pure, no tokio/reqwest)
- `crates/emend-ffi/src/ai.rs` — HTTP orchestration, cancellation, timeout, AiHandle, AiRequestConfig, summarize_document, test_ai_config exports
- `app/Emend/Emend/AI/` — UI layer (settings, key entry, summarize button, streaming text render)

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
| Workspace search index (`nucleo-matcher`) | Fast fuzzy matching for Quick Open + wiki-link resolution (search 10K+ notes instantly); reused for wiki-link suggestions (Phase 7 US5) | In-memory arena-based `Index`; synchronous `query()` ranks by basename ≫ path; O(1) event dispatch on create/rename/delete | **WIRED** Phase 4 US2 (Quick Open); Phase 7 US5 (wiki-link suggestions via `wikilink_suggestions()`) |

**Ranking:**

- Fuzzy subsequence match on **basename** (primary, boosted) and **location-relative path** (secondary)
- Score = better of the two; shorter relative path breaks ties
- `nucleo-matcher::Matcher` (no worker pool; synchronous like the index)

**Code Location:**

- `crates/emend-core/src/index.rs` — `Index` struct + `query()` + incremental updates
- `crates/emend-core/src/derived.rs` — `wikilink_suggestions()` (Phase 7 US5)
- `crates/emend-ffi/src/index.rs` — FFI `IndexHandle` + `search_index_query()` export
- `crates/emend-ffi/src/workspace.rs` — `WorkspaceHandle::wikilink_suggestions()` FFI export (Phase 7 US5)
- `app/Emend/Emend/Tabs/TabModel.swift` + `app/Emend/Emend/Sidebar/WorkspaceModel.swift` — UI consumers (Quick Open, tree rendering, wiki-link autocomplete)

---

## Markdown Processing & Preview Rendering (Phase 6 US4 + Phase 7 US5)

### Preview Engine (Authoritative)

| Component | Purpose | Technology | Status |
|-----------|---------|-----------|--------|
| Preview HTML rendering | Whole-document CommonMark + GFM (tables, tasklist, strikethrough, autolinks) + native extensions (wikilinks, `==highlight==`→`<mark>`, embeds via source splice) with scroll-sync anchors | `comrak` 0.52 with `render.sourcepos` → `data-line` attributes (research §C3); embeds expanded by `parse/embed.rs` pre-render pass before comrak (Phase 7 US5, FR-021/021a) | **WIRED** — Phase 6 US4 (Preview + PDF export); Phase 7 US5 (embeds); `crates/emend-core/src/parse/preview.rs`; FFI exports `OpenDocHandle::render_preview_html()` + `render_preview_html_resolving(workspace:)` |

**Key Properties:**

- **Authoritative & complete**: Renders whole document; **not incremental**; distinctly separate from the editor's incremental tree-sitter highlighter (Constitution guardrail).
- **Deterministic**: Pure `&str -> String` transform; no IO, no network, no async (verified by `tests/preview_offline.rs`).
- **Scroll-sync anchors**: Each block emits `data-line="<line>"` for editor ↔ preview synchronization (research §C3).
- **No remote loads**: Embedded URLs are emitted literally; never dereferenced (verified by CSP on Swift side).
- **Code highlighting**: Fenced code blocks colored by syntect (see below).
- **Embeds**: Recursive expansion with cycle + depth guards (Phase 7 US5); resolved via closure so the app supplies the resolver (comrak + fs reading are separate concerns).

**Code Location:**

- Core engine: `crates/emend-core/src/parse/preview.rs` — `render_preview_html()` and `render_preview_html_with_embeds()`
- Embed expansion: `crates/emend-core/src/parse/embed.rs` — `expand_embeds()` pre-render pass with cycle/depth guards
- FFI: `crates/emend-ffi/src/document.rs` — methods `OpenDocHandle::render_preview_html()` and `render_preview_html_resolving()`
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
| `WKWebView` (macOS AppKit) | Render comrak preview HTML + bundled Mermaid diagrams + KaTeX math + embedded notes | `nonPersistent` session, navigation delegate (blocks remote loads), CSP header, `loadFileURL` for bundled assets | No remote scripts/images/stylesheets; all resources bundled in app |

**Properties:**

- **Rendering**: Displays `render_preview_html_resolving()` output with embedded `<script data-mermaid>` and `<math>` tags from comrak.
- **Bundled assets**: Mermaid.js 11.15, KaTeX 0.17 + fonts under `app/Emend/Emend/Resources/preview/`; loaded via `loadFileURL`.
- **Scroll sync**: `bridge.js` handles editor ↔ preview scroll linking via `data-line` anchors (research §C3).
- **CSP**: Blocks inline `<script>`, external `src=`, and all `fetch()`; enforced by `theme.css` `<meta>` headers.
- **User data only**: Renders note HTML; no AI-generated content, no ambient telemetry.
- **Typography applied via CSS injection (US7)**: Preview template receives clamped `font-size`, `line-height`, `margin-bottom` injected from `TypographySettings` on every change.

**Implementation Status**: Phase 6 US4 (wired preview); Phase 7 US5 (embeds); scroll-sync in `PreviewModel.swift`; typography injection in `PreviewWebView.swift` (US7)

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
| Security framework (Keychain) | Secure API key storage (user-provided AI endpoint credentials) | `SecKeychain` C APIs via Swift FFI | **WIRED** — core to AI key management (US6 FR-035) |
| NSTextView / TextKit 2 | Native editor surface with split-paragraph storage; completion popup for `[[` wiki-link autocomplete | AppKit / SwiftUI integration + `MarkdownEditorView` NSViewRepresentable | **WIRED** — phase 0 skeleton, Phase 3 US1 MVP (editor transforms), Phase 7 US5 (completion popup) |
| NSOutlineView | Folder/file tree sidebar with expansion/collapse | AppKit | **WIRED** — Phase 4 US2 (`WorkspaceOutlineView.swift`) |
| WKWebView | Preview rendering + scroll-sync + PDF export + embedded note display | WebKit framework | **WIRED** — Phase 6 US4 (`PreviewWebView.swift`, `PDFExport.swift`), Phase 7 US5 (embed rendering) |
| NSPrintOperation | Paginated PDF export | AppKit | **WIRED** — Phase 6 US4 (`PDFExport.swift`) |
| Pasteboard | Copy/paste support + drag-drop for attachments | `NSPasteboard` API | **WIRED** phase 7 US5 (drop→`store_attachment`); copy/paste planned phase 1–2 |
| Security-scoped bookmarks | Persistent folder access permission | AppKit / Swift Security Framework | **WIRED** — Phase 0, research §A4; resolves on launch to a usable filesystem path |
| NSFontManager / NSFont / NSParagraphStyle | Typography controls for editor (font picker, paragraph styling) | AppKit | **WIRED** — Phase 9 US7 (typography UI, font picker, paragraph spacing) |

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
| Collision on attachment drop | Attachment with same name exists in `attachments/` | Collision-safe naming appends space + lowest free integer before extension (`image 2.png` scheme, FR-013a) |
| AI not configured | User invokes AI feature with no API key | Return `EmendError::AiNotConfigured`; UI shows setup prompt |
| AI endpoint unreachable | Network error or 5xx response | `EmendError::AiHttp` with redacted status; user can retry or abort |
| AI SSE malformed | Broken event stream | `EmendError::AiStreamMalformed`; cancel the request gracefully |
| AI timeout | Request exceeds per-chunk inactivity timeout | `EmendError::AiTimeout`; user can retry |
| AI input oversized | Document exceeds max-input-bytes | `EmendError::AiOversizedInput`; rejected before socket (FR-036a, SC-008) |
| User cancels AI request | Cancel token triggered mid-stream | `EmendError::AiCancelled`; clean shutdown of the stream task |
| AI key blank | User saves empty or whitespace-only key | Treated as not configured; `AiNotConfigured` before send |
| Wiki-link unresolved | Target note not found or renamed | `resolve_wikilink()` returns `None`; editor shows broken-link styling; no auto-rewrite in v1 |
| Embed unresolved | Embed target not found | Placeholder rendered (not an error); embed is a reading aid, not load-bearing |
| Embed cycle | `A→B→A` or `A→A` | Cycle detection stops re-entrance; note expands at most once per path |
| Embed too deep | Nesting exceeds `MAX_EMBED_DEPTH` (8) | Depth-bound stops recursion; token at bound rendered as placeholder |
| Attachment drop fails | I/O or permission error writing to `attachments/` dir | `EmendError::PermissionDenied` or `EmendError::IoFailure`; user sees error dialog |
| Preview render fails | Unexpected comrak error | Fall back to plain-text display or re-render |
| PDF export: template missing | App bundle error | `PDFExport.Failure.templateMissing`; user sees error dialog |
| PDF export: render failed | WebView fails to load HTML or JS error | `PDFExport.Failure.renderFailed(detail)`; user sees error with details |
| PDF export: print failed | I/O error or user cancel | `PDFExport.Failure.printFailed`; gracefully dismiss |
| Preview WebView crashes | Internal WebKit failure | Error logged; fall back to plain-text preview or re-render |
| Syntax highlighting lagging | Tree-sitter reparse exceeds 50ms budget | Fall back to previous highlight spans; no visual stutter (advisory-only styling) |
| Info sidebar stats/outline stale | Observer fires late or document changes fast | Re-pull triggered by observer callback (≤300ms latency); sidebar shows most recent pull |
| Typography settings malformed | Font family blank, size/spacing out of range | Core clamps on `set_typography()` before storing; no error, values repair to safe bounds (e.g., 0 → 8 pt, blank → system font) |

---

## What Does NOT Belong Here

- Internal code architecture → `ARCHITECTURE.md`
- Testing infrastructure → `TESTING.md`
- Security policies → `SECURITY.md`
- Dependency versions and selection → `STACK.md`
- File system operations / Keychain access patterns → See `crates/emend-core/src/fs.rs`, `crates/emend-core/src/workspace.rs`, `crates/emend-core/src/watcher.rs`, `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift`
- Editor-UI transforms (smart lists, formatting, task toggling) → `CONVENTIONS.md`
- Markdown engines (two-engine architecture, embed expansion) → `CONVENTIONS.md` (patterns) & `ARCHITECTURE.md` (design)

---

*This document maps external service dependencies and protocols. Update when adding new integrations or modifying AI endpoints, file watching, workspace management, preview rendering, link/embed/task handling, attachment storage, PDF export, document analytics (stats/outline/observer), or typography settings storage/clamping.*
