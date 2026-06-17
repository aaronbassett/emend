# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Overview

Emend is a privacy-first, offline-by-default Markdown editor governed by Constitution Principle II (Local-First & Privacy by Default) and NFR-006 (AI key handling). The app makes **zero outbound network calls unless the user explicitly configures a bring-your-own-model (BYOM) OpenAI-compatible AI endpoint AND invokes an AI action**. File access is sandboxed with app-scoped security-scoped bookmarks. Autosave is atomic and durable per Constitution Principle III.

---

## Sandbox & File Access

### App Sandbox Configuration

| Setting | Value | File |
|---------|-------|------|
| **Status** | Enabled (shipped day one) | `app/Emend/Emend/Emend.entitlements` |
| **Entitlements** | `com.apple.security.app-sandbox` | ✅ |
| | `com.apple.security.files.user-selected.read-write` | ✅ |
| | `com.apple.security.files.bookmarks.app-scope` | ✅ |

### Security-Scoped Bookmarks (User-Granted Folders)

The app uses **security-scoped bookmarks** to persist access to user-selected folders across app restarts:

| Aspect | Implementation | Location |
|--------|-----------------|----------|
| **Lifecycle** | `NSOpenPanel` → `bookmarkData(options:.withSecurityScope)` → persist → resolve on launch | `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift` |
| **Scope management** | `startAccessingSecurityScopedResource()` / `stopAccessingSecurityScopedResource()` (balanced per scope) | `SecurityScopedBookmarks.swift` |
| **Rust-Swift handshake** | Swift opens the scope; Rust file IO (`read_file_at`, `write_atomic`, `notify` watcher) operates within scope | `crates/emend-ffi/src/lib.rs` (export `read_file_at`) |
| **Validation** | Security-scoped behavior verified in the signed app; tests use plain bookmarks (test process is unsandboxed) | `app/Emend/EmendTests/BookmarkResolutionTests.swift` |

---

## AI & Network Security

### AI Configuration & Key Storage

| Aspect | Implementation | Principle | Notes |
|--------|-----------------|-----------|-------|
| **Storage** | macOS Keychain only (`SecItem` with `kSecClassGenericPassword`, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`) | NFR-006, Constitution II | Never in app preferences, logs, or Rust persistence |
| **Custody** | Swift reads from Keychain immediately before each request; key passed as transient `String` parameter across FFI | NFR-006 | Key never persisted Rust-side |
| **Redaction** | Key held in redacting newtype (`Debug`/`Display` → `***`); set only on `Authorization` header, never in logging fields | NFR-006 | Implementation: `crates/emend-core/src/ai.rs` (when implemented) |
| **Configuration** | OpenAI-compatible endpoint (baseURL, model, key) stored in Keychain (key) + local app store (baseURL, model metadata) | Constitution II | No API key in app store |

### Network Isolation

| Constraint | Implementation | Test |
|-----------|---|---|
| **Default offline** | No network calls without explicit AI configuration + invocation | `crates/emend-core/tests/ai_privacy.rs` (planned) |
| **Preview is offline** | `WKWebView` with CSP blocking remote loads, `nonPersistent` store, no navigation to non-`file:`/`about:` URLs | `app/Emend/Emend/UI/PreviewView.swift` | 
| **Bundled assets** | Mermaid.js + KaTeX vendored locally; loaded via `loadFileURL`, not CDN | `app/Emend/Emend/UI/PreviewView.swift` |
| **AI request validation** | Max input size checked before sending; requests are cancellable via `tokio_util::sync::CancellationToken`; per-chunk timeout + overall deadline | `crates/emend-core/src/ai.rs` (research §B5) |
| **AI error handling** | Failed or timed-out requests never leave sensitive data in logs | Logging best practice (code review gate) |

---

## File System Integrity (Atomic & Durable Writes)

### Autosave Implementation

| Stage | Mechanism | Durability |
|-------|-----------|-----------|
| **Write** | Temp file in same directory as target (not system temp) | Ensures same-filesystem rename |
| **Flush + sync** | `File::sync_all()` (on Apple: `fcntl(F_FULLFSYNC)`) | Physical durability before rename |
| **Atomic rename** | `tempfile::NamedTempFile::persist()` → `rename(2)` | All-or-nothing visibility to readers |
| **Directory sync** | Sync the containing directory after rename | Rename metadata durability |
| **Debounce** | ~1.5 s idle, hard cap 5 s (no fsync per keystroke) | Balances durability with performance |

**Location**: `crates/emend-core/src/fs.rs`

### External Edit Conflict Policy

| Scenario | Behavior | Code |
|----------|----------|------|
| **File changed on disk, no unsaved edits** | Silent reload | `crates/emend-core/src/watcher.rs` (planned) |
| **File changed on disk, unsaved edits in memory** | Preserve both versions; mark stale; let user choose reload or keep local | `app/Emend/Emend/Shell/MainWindow.swift` (planned) |
| **Self-write suppression** | Post-persist `(mtime,len)` tracked; matching event suppressed within ~300 ms window | `crates/emend-core/src/watcher.rs` (planned) |

---

## Error Handling & Panic Containment

### Panic Safety (NFR-003)

| Layer | Mechanism | Coverage |
|-------|-----------|----------|
| **UniFFI exports** | Every `#[uniffi::export]` wrapped in `catch_unwind` → Swift `Error` | All sync/async exports |
| **Spawned tasks** | `tokio::spawn` bodies wrapped in `contain_panic` → `EmendError::Internal` | AI / search tasks |
| **Lint policy** | `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` in `emend-core` | No panics in logic layer |

**Implementation**: `crates/emend-ffi/src/panic.rs`

### Error Model

All fallible operations return `Result<T, EmendError>`:

| Variant | Meaning | UI Rendering |
|---------|---------|---|
| `NotFound { path }` | File/folder not found | User-friendly message with path |
| `PermissionDenied { path }` | App lacks permission | Suggest re-grant folder access |
| `IoFailure { path, detail }` | Disk I/O error | System error context |
| `NameCollision { path }` | Name already exists | Prompt rename / disambiguate |
| `NoteTooLarge { path, bytes, limit }` | Doc exceeds ~5 MB | Open read-only with notice |
| `AiNotConfigured` | No AI endpoint set | Prompt configuration |
| `AiTimeout` | Request timed out | Suggest retry or increase timeout |
| `AiCancelled` | User cancelled request | Silent (expected) |
| `AiOversizedInput { bytes, limit }` | Input exceeds limit | Refuse request; show size |
| `AiHttp { status, detail }` | HTTP error (e.g., 401, 429) | Auth error or rate-limit guidance |
| `AiStreamMalformed { detail }` | SSE stream unparseable | Transient error; suggest retry |
| `Internal { detail }` | Caught panic or internal fault | Generic "unexpected" message |

**Location**: `crates/emend-ffi/src/error.rs`

---

## Input Validation & Sanitization

### Document Input

| Data Type | Validation | Location |
|-----------|-----------|----------|
| **File paths** | Resolved within scope of security-scoped bookmark; no `..` traversal above location root | `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift` |
| **File content** | Tolerant reads: UTF-8 BOM stripped, CRLF preserved, invalid UTF-8 decoded lossily (not rejected) | `crates/emend-core/src/fs.rs` |
| **Markdown syntax** | tree-sitter (editor) and comrak (preview) both handle malformed input gracefully (no crash) | `crates/emend-core/src/parse.rs` (planned) |
| **Wiki links** | Resolved deterministically by name + path; unresolved links marked visually | `crates/emend-core/src/index.rs` (planned) |
| **Embed depth** | Max depth of 8 enforced; cycles detected and stopped | `crates/emend-core/src/parse.rs` (planned) |

### AI Input

| Constraint | Enforcement | Location |
|-----------|-----------|----------|
| **Max input size** | Checked **before** network call (document truncated or refused) | `crates/emend-core/src/ai.rs` (planned) |
| **Streaming parse** | Line-buffered SSE; `data:` split across chunks handled | `crates/emend-core/src/ai.rs` |
| **Cancellation** | Safe to cancel mid-stream; no partial state persisted | `crates/emend-core/src/ai.rs` |

---

## Secrets Management

### Environment Variables

| Category | Naming | Example | Storage |
|----------|--------|---------|---------|
| **AI endpoint config** | User-provided via Settings UI | (no env var) | Keychain (key) + app store (baseURL, model) |

### Development & CI

| Environment | Method |
|-------------|--------|
| **Local dev** | Keychain (same as production) |
| **CI/testing** | Test fixtures use mock endpoints (no real API key) | 

---

## Code Quality & Linting

### Rust

| Tool | Config | Enforcement |
|------|--------|------------|
| **rustfmt** | Workspace default (2-space indent) | Pre-commit hook (`lefthook`) |
| **clippy** | `-D warnings` (deny all) | CI gate |
| **Custom lint** | `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` | `emend-core` only |

### Swift

| Tool | Config | Enforcement |
|------|--------|------------|
| **SwiftFormat** | `.swiftformat` (checked in) | Pre-commit hook |
| **SwiftLint** | `.swiftlint.yml` (checked in) | Pre-commit hook + CI |

---

## Dependency Security

### Rust Dependencies (Core)

Pinned in `Cargo.toml` workspace `[workspace.dependencies]`:

| Crate | Version | Purpose | MSRV |
|-------|---------|---------|------|
| `thiserror` | workspace | Error derives | ≤ 1.85 |
| `tempfile` | workspace | Atomic writes | ≤ 1.85 |
| `ropey` | workspace | UTF-16 rope (document buffer) | ≤ 1.85 |
| `uniffi` | workspace | FFI bindings (emend-ffi only) | ≤ 1.85 |
| `tokio` | workspace | Async runtime | ≤ 1.85 |
| `tokio-util` | workspace | Cancellation tokens | ≤ 1.85 |

**Inert (not yet used)**:
- `tree-sitter`, `tree-sitter-md` (incremental highlight)
- `comrak` (preview HTML)
- `syntect` (code block highlighting)
- `nucleo` (fuzzy search)
- `notify`, `notify-debouncer-full` (file watching)
- `reqwest` (AI HTTP client)

All versions verified at checkout; MSRV pinning enforced by `cargo +1.85 check --all` in CI.

**Location**: `Cargo.toml` ([workspace], workspace.package.rust-version)

---

## Audit Logging

| Event | Logged Data | Retention | Status |
|-------|-------------|-----------|--------|
| **File operations** | Path, operation (read/write/delete), success/error | In-app debug logs (opt-in) | Implemented (fs.rs, error handling) |
| **AI requests** | Endpoint, model, request/response size, latency, error (never key) | In-app debug logs (opt-in) | Planned (ai.rs) |
| **Security sandbox** | Bookmark grant/revoke events (via OS log) | System logs | Native (Security framework) |

**Notes**: All logs are development/diagnostic; no telemetry is sent off-device. Logs are cleared on app exit unless explicitly persisted to a debug file.

---

## Verification & Testing

### Security-Specific Test Coverage

| Test | Coverage | Location | Status |
|------|----------|----------|--------|
| **Panic containment** | FFI exports and spawned tasks never unwind; panic becomes `EmendError::Internal` | `crates/emend-ffi/tests/panic_containment.rs` | ✅ Implemented |
| **Atomic writes** | Kill between write+rename → original file intact | `crates/emend-core/tests/fs_atomic.rs` | ✅ Implemented |
| **Tolerant reads** | BOM/CRLF/invalid UTF-8 read correctly | `crates/emend-ffi/src/lib.rs` (unit test) | ✅ Implemented |
| **Bookmark resolution** | Add folder, quit, relaunch → reads + watches without new prompt | `app/Emend/EmendTests/BookmarkResolutionTests.swift` | ✅ Implemented (plain bookmarks, unsandboxed test) |
| **AI privacy (offline)** | No network with AI unconfigured | `crates/emend-core/tests/ai_privacy.rs` | Planned |
| **AI key redaction** | Logs never contain key substring | Code review + test capture | Planned |
| **Preview offline** | WKWebView makes zero network calls | Airplane mode test | Planned |

---

## Known Limitations & Deferred Work

1. **Security-scoped-bookmark validation** is tested with plain (non-security-scoped) bookmarks in the test process (which is unsandboxed). Full sandbox behavior is validated only in the signed, notarized app.
2. **AI features** (key redaction, privacy tests, timeout handling) are designed and specified but not yet implemented (Phase 1 tasks).
3. **Dependency vulnerability scanning** is not automated in CI; manual `cargo audit` checks recommended pre-release.

---

## What Does NOT Belong Here

- Tech debt and risks → CONCERNS.md
- Testing strategy → TESTING.md
- Code conventions → CONVENTIONS.md

---

*This document defines security controls. Update when security posture changes.*
