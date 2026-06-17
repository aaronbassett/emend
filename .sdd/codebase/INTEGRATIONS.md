# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Data Stores

### File System (Primary)

| Service | Type | Purpose | Configuration |
|---------|------|---------|----------------|
| macOS File System | Local Filesystem | Primary note storage (plain `.md` files on disk) | User selects folder via security-scoped bookmarks (research §A4) |

**Connection Patterns:**

- **Reading**: `emend_core::fs::read_tolerant` (UTF-8 BOM stripping, CRLF preservation, lossy UTF-8 decode)
- **Writing**: `emend_core::fs::write_atomic` (temp file + fsync + atomic rename + fsync dir for durability)
- **Watching**: `notify` + `notify-debouncer-full` (file system events with debounce + self-write suppression, not yet wired)
- **No migration**: Plain Markdown; app-managed state lives in `NSUserDefaults` and Keychain

### App Preferences & Configuration

| Service | Type | Purpose | Configuration |
|---------|------|---------|----------------|
| macOS Keychain | Secure Storage | AI API key (transient to Rust, never logged/persisted) | macOS Security framework (`SecKeychain` C APIs via Swift FFI) |
| `NSUserDefaults` | Local Preferences | Editor state, location tree favorites, folder icons, typography settings, AI config metadata | Standard macOS app preferences (per-user, non-synced) |

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

### File Watcher (macOS Native)

| Service | Purpose | Configuration | Failure Mode |
|---------|---------|----------------|--------------|
| macOS File System Events (via `notify` + `notify-debouncer-full` crates) | Detect external note edits; reload + alert user | Debounced to 100ms; self-write suppression (FR-006a) | Silent miss if event queue overflows; user can manually refresh |

**Implementation Status**: Planned phase 0–1 (FR-006a); not yet wired

---

## Syntax Highlighting

### Editor Highlighting (Internal)

| Component | Purpose | Technology | Status |
|-----------|---------|-----------|--------|
| Editor live-syntax highlight | Real-time visual feedback as user types (bold, italic, headings, code, etc.); advisory only — does not affect preview rendering | `tree-sitter-md` (split block + inline Markdown grammar) with incremental tree-sitter runtime | **WIRED** — Phase 3 US1 (Editor MVP); lives in `crates/emend-core/src/parse/highlight.rs` |

**Performance:**
- Incremental per-keystroke reparses (≤50 ms typing budget, SC-003)
- Does NOT block the main thread; UTF-16 coordinate bridging via `ropey::Rope` mirror
- Advisory styling only; preview rendering uses separate `comrak` engine (not yet wired)

**Code Location:**
- Highlighter struct: `crates/emend-core/src/parse/highlight.rs`
- FFI export: `crates/emend-ffi/` (style span collection via handle)
- Swift integration: `app/Emend/Emend/Editor/SyntaxAttributing.swift` (applies spans to `NSTextView`)

---

## Preview Rendering

### Web View

| Service | Purpose | Configuration |
|---------|---------|----------------|
| `WKWebView` (macOS AppKit) | Render preview HTML + Mermaid diagrams + KaTeX math | nonPersistent, navigation delegate (blocks remote loads), CSP header |

**Security:**

- Bundled Mermaid.js (no remote CDN)
- Bundled KaTeX (no remote CDN)
- Content Security Policy (CSP) blocks inline `<script>` and remote loads
- User data (note HTML) rendered, but no AI-generated content exposed

**Implementation Status**: Planned phase 1 (US3 — preview) with authoritative `comrak` engine

---

## Diagram & Math Rendering

### Bundled Libraries (Not External APIs)

| Library | Format | Bundling | Status |
|---------|--------|----------|--------|
| Mermaid.js | Diagrams (flowchart, sequence, etc.) | Bundled as `.js` file in the app; loaded into `WKWebView` | Planned phase 1–2 (FR-041 — diagram support) |
| KaTeX | LaTeX-style math (inline `$…$` and block `$$…$$`) | Bundled as `.js` + CSS in the app; injected into preview HTML | Planned phase 1–2 (FR-042 — math rendering) |

No external CDN loads.

---

## File Export

### PDF Export

| Service | Purpose | Configuration |
|---------|---------|----------------|
| `WKWebView` printing API | Render preview to PDF | Native macOS print dialog; user selects path |

**Implementation Status**: Planned phase 2 (FR-032 — PDF export)

---

## System Integration

### macOS System Services

| Service | Purpose | API | Status |
|---------|---------|-----|--------|
| Security framework (Keychain) | Secure API key storage | `SecKeychain` C APIs via Swift FFI | **WIRED** — core to AI key management |
| NSTextView / TextKit 2 | Native editor surface with split-paragraph storage | AppKit / SwiftUI integration + `MarkdownEditorView` NSViewRepresentable | **WIRED** — phase 0 skeleton, Phase 3 US1 MVP (editor transforms) |
| NSOutlineView | Folder/file tree sidebar | AppKit | **WIRED** — phase 0 skeleton |
| Pasteboard | Copy/paste support | `NSPasteboard` API | Planned phase 1–2 |

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
| External file modified | File on disk changes while edited in Emend | Debounced watcher detects; prompt user (keep mine / reload / merge — TBD in phase 1) |
| AI not configured | User invokes AI feature with no API key | Return `EmendError::AiNotConfigured`; UI shows setup prompt |
| AI endpoint unreachable | Network error or 5xx response | `EmendError::AiHttp` with redacted status; user can retry or abort |
| AI SSE malformed | Broken event stream | `EmendError::AiStreamMalformed`; cancel the request gracefully |
| AI timeout | Request exceeds timeout | `EmendError::AiTimeout`; user can retry |
| User cancels AI request | Cancel token triggered mid-stream | `EmendError::AiCancelled`; clean shutdown of the stream task |
| Preview WebView crashes | Internal WebView failure | Error logged; fall back to plain-text preview or re-render |
| Syntax highlighting lagging | Tree-sitter reparse exceeds 50ms budget | Fall back to previous highlight spans; no visual stutter (advisory-only styling) |

---

## What Does NOT Belong Here

- Internal code architecture → `ARCHITECTURE.md`
- Testing infrastructure → `TESTING.md`
- Security policies → `SECURITY.md`
- Dependency versions and selection → `STACK.md`
- File system operations / Keychain access patterns → See `crates/emend-core/src/fs.rs`, `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift`
- Editor-UI transforms (smart lists, formatting) → `CONVENTIONS.md`

---

*This document maps external service dependencies and protocols. Update when adding new integrations or modifying AI endpoints, file watching, or preview rendering.*
