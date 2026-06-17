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
| **Status** | Enabled (shipped Phase 2) | `app/Emend/Emend/Emend.entitlements` |
| **Entitlements** | `com.apple.security.app-sandbox` | ✅ |
| | `com.apple.security.files.user-selected.read-write` | ✅ |
| | `com.apple.security.files.bookmarks.app-scope` | ✅ |

### Security-Scoped Bookmarks (User-Granted Folders)

The app uses **security-scoped bookmarks** to persist access to user-selected folders across app restarts:

| Aspect | Implementation | Location |
|--------|-----------------|----------|
| **Lifecycle** | `NSOpenPanel` → `bookmarkData(options:.withSecurityScope)` → persist → resolve on launch | `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift` |
| **Scope management** | `startAccessingSecurityScopedResource()` / `stopAccessingSecurityScopedResource()` (balanced per scope) | `SecurityScopedBookmarks.swift` |
| **Rust-Swift handshake** | Swift opens the scope; Rust file IO (`read_file_at`, `write_atomic`, `notify` watcher) operates within scope | `crates/emend-ffi/src/lib.rs` (exports) |
| **Validation** | Security-scoped behavior verified in the signed app; tests use plain bookmarks (test process is unsandboxed) | `app/Emend/EmendTests/BookmarkResolutionTests.swift` |
| **Scope lifecycle tests** | Plain bookmarks (no `.withSecurityScope`) parameterized into `resolve()` / `makeBookmark()`; stale-bookmark refresh tested | `app/Emend/EmendTests/BookmarkResolutionTests.swift` |

---

## AI & Network Security

### AI Configuration & Key Storage (Deferred to Phase 1)

| Aspect | Plan | Principle | Notes |
|--------|------|-----------|-------|
| **Storage** | macOS Keychain only (`SecItem` with `kSecClassGenericPassword`, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`) | NFR-006, Constitution II | Design specified (research §B5); implementation deferred |
| **Custody** | Swift reads from Keychain immediately before each request; key passed as transient `String` parameter across FFI | NFR-006 | Not yet wired; hot path reserved for Phase 1 |
| **Redaction** | Key held in redacting newtype (`Debug`/`Display` → `***`); set only on `Authorization` header, never in logging fields | NFR-006 | Implementation: `crates/emend-core/src/ai.rs` (Phase 1 T112) |
| **Configuration** | OpenAI-compatible endpoint (baseURL, model, key) stored in Keychain (key) + local app store (baseURL, model metadata) | Constitution II | No API key in app store |

### Network Isolation

| Constraint | Implementation | Status |
|-----------|---|---|
| **Default offline** | No network calls without explicit AI configuration + invocation | Phase 1 T110 (test: `ai_privacy.rs`) |
| **Preview is offline** | WKWebView with CSP blocking remote loads, `nonPersistent` store, no navigation to non-`file:`/`about:` URLs | Deferred (US2 Phase 1) |
| **Bundled assets** | Mermaid.js + KaTeX vendored locally; loaded via `loadFileURL`, not CDN | Deferred (US4 Phase 1) |
| **AI request validation** | Max input size checked before sending; requests are cancellable via `tokio_util::sync::CancellationToken`; per-chunk timeout + overall deadline | Phase 1 T112 |
| **AI error handling** | Failed or timed-out requests never leave sensitive data in logs | Phase 1 (code review gate) |

---

## File System Integrity (Atomic & Durable Writes)

### Autosave Implementation

| Stage | Mechanism | Durability |
|-------|-----------|-----------|
| **Write** | Temp file in same directory as target (not system temp) | Ensures same-filesystem rename |
| **Flush + sync** | `File::sync_all()` (on Apple: `fcntl(F_FULLFSYNC)` via Rust std) | Physical durability before rename |
| **Atomic rename** | `tempfile::NamedTempFile::persist()` → `rename(2)` | All-or-nothing visibility to readers |
| **Directory sync** | Sync the containing directory after rename | Rename metadata durability |
| **Debounce** | ~1.5 s idle, hard cap 5 s (no fsync per keystroke) | Balances durability with performance (Phase 1) |

**Location**: `crates/emend-core/src/fs.rs` (implemented, tested)

**Key implementation detail**: On Apple targets, Rust's `std::fs::File::sync_all()` automatically calls `fcntl(fd, F_FULLFSYNC)` (fixed in rust-lang/rust#55920, present in MSRV 1.85+). No manual `libc`/`rustix` call needed; no additional `unsafe` blocks at call sites.

### External Edit Conflict Policy

| Scenario | Behavior | Code | Status |
|----------|----------|------|--------|
| **File changed on disk, no unsaved edits** | Silent reload | `crates/emend-core/src/watcher.rs` | Phase 1 T065 |
| **File changed on disk, unsaved edits in memory** | Preserve both versions; mark stale; let user choose reload or keep local | `app/Emend/Emend/Shell/MainWindow.swift` | Phase 1 T067 |
| **Self-write suppression** | Post-persist `(mtime,len)` tracked; matching event suppressed within ~300 ms window | `crates/emend-core/src/watcher.rs` | Phase 1 T066 |

---

## Error Handling & Panic Containment

### Panic Safety (NFR-003)

| Layer | Mechanism | Coverage | Location |
|-------|-----------|----------|----------|
| **Workspace lint policy** | `[workspace.lints.clippy]`: `unwrap_used = "deny"`, `expect_used = "deny"`, `panic = "deny"` | All `emend-core` crates | `Cargo.toml` (lines 89–92) |
| **Test module escape hatch** | `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "unit test asserts")]` | Isolated to test modules | `emend-core/src/document.rs` (lines 268–272) |
| **UniFFI exports** | Every `#[uniffi::export]` wrapped in `catch_unwind` → Swift `Error` | All sync/async exports | `crates/emend-ffi/src/lib.rs` |
| **Spawned tasks** | `tokio::spawn` bodies wrapped in `contain_panic` → `EmendError::Internal` | AI / search tasks (Phase 1) | `crates/emend-ffi/src/panic.rs` |

**Implementation**: `crates/emend-ffi/src/panic.rs` (contains `catch_unwind` wrappers and `contain_panic` helper)

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

**Location**: `crates/emend-core/src/error.rs` (mirrors via `crates/emend-ffi/src/error.rs`)

---

## Input Validation & Sanitization

### Document Input

| Data Type | Validation | Location | Status |
|-----------|-----------|----------|--------|
| **File paths** | Resolved within scope of security-scoped bookmark; no `..` traversal above location root | `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift` | ✅ Implemented |
| **File content (read)** | Tolerant reads: UTF-8 BOM stripped, CRLF preserved, invalid UTF-8 decoded lossily (not rejected) | `crates/emend-core/src/fs.rs` | ✅ Implemented |
| **File content (write)** | Atomic write via tempfile; no special escaping (Markdown is plain text) | `crates/emend-core/src/fs.rs::write_atomic` | ✅ Implemented |
| **Markdown syntax (edit)** | tree-sitter (editor) handles malformed input gracefully (incremental reparse, no crash) | `crates/emend-core/src/parse.rs` | Phase 1 T072 |
| **Markdown syntax (preview)** | comrak (CommonMark) handles malformed input gracefully (no crash, renders as-is) | `crates/emend-core/src/parse.rs` | Phase 1 T073 |
| **Wiki links** | Resolved deterministically by name + path; unresolved links marked visually | `crates/emend-core/src/index.rs` | Phase 1 T074 |
| **Embed depth** | Max depth of 8 enforced; cycles detected and stopped | `crates/emend-core/src/parse.rs` | Phase 1 T080 |

### AI Input (Deferred to Phase 1)

| Constraint | Plan | Status |
|-----------|------|--------|
| **Max input size** | Checked **before** network call (document truncated or refused) | Phase 1 T112 |
| **Streaming parse** | Line-buffered SSE; `data:` split across chunks handled | Phase 1 T112 |
| **Cancellation** | Safe to cancel mid-stream; no partial state persisted | Phase 1 T113 |

---

## Secrets Management

### Environment Variables

No environment variables required for core functionality. AI configuration is stored in Keychain (key) + local app prefs (endpoint metadata), not env vars.

### Development & CI

| Environment | Method |
|-------------|--------|
| **Local dev** | Keychain (same as production); manually tested with mock server (Phase 1) |
| **CI/testing** | Test fixtures use mock HTTP endpoints (no real API key) | 

---

## Code Quality & Linting

### Rust

| Tool | Config | Enforcement |
|------|--------|------------|
| **rustfmt** | Workspace default (2-space indent) | Pre-commit hook (`lefthook`) |
| **clippy** | `-D warnings` (deny all) + custom workspace lints (`unwrap_used`, `expect_used`, `panic`) | CI gate + workspace `[lints]` |
| **Workspace lints** | `clippy::unwrap_used = "deny"`, `expect_used = "deny"`, `panic = "deny"` | Inherited by all crates via `[lints] workspace = true` |
| **MSRV verification** | `cargo +1.85 check --all` | CI gate (prevents surprise dependency bumps) |

**Location**: `Cargo.toml` ([workspace.lints], workspace.package.rust-version)

### Swift

| Tool | Config | Enforcement |
|------|--------|------------|
| **SwiftFormat** | `.swiftformat` (checked in) | Pre-commit hook |
| **SwiftLint** | `.swiftlint.yml` (checked in) | Pre-commit hook + CI |

---

## Dependency Security

### Rust Dependencies (Core)

Pinned in `Cargo.toml` workspace `[workspace.dependencies]` and `[workspace.package]` (MSRV = 1.85):

| Crate | Version | Purpose | MSRV |
|-------|---------|---------|------|
| `thiserror` | 2.x | Error derives | ≤ 1.85 |
| `tempfile` | 3.x | Atomic writes (stdlib `sync_all` handles durability) | ≤ 1.85 |
| `ropey` | 1.6.1 | UTF-16 rope (document buffer); 2.0 beta intentionally excluded | ≤ 1.85 |
| `tokio` | 1.x | Async runtime (emend-ffi only) | ≤ 1.85 |
| `tokio-util` | 0.7.x | Cancellation tokens (emend-ffi only) | ≤ 1.85 |
| `tree-sitter` | 0.26.x | Incremental parser runtime | ≤ 1.85 |
| `tree-sitter-md` | 0.5.x | Markdown grammar (parser feature required) | ≤ 1.85 |
| `comrak` | 0.52.x | CommonMark preview (inert until Phase 1) | ≤ 1.85 |
| `syntect` | 5.3.x | Code highlighting (inert until Phase 1) | ≤ 1.85 |
| `nucleo` | 0.5.x | Fuzzy search (inert until Phase 1) | ≤ 1.85 |
| `notify`, `notify-debouncer-full` | 8.2.x, 0.7.x | File watching (inert until Phase 1) | ≤ 1.85 |
| `reqwest` | 0.13.x | AI HTTP client with SSE (inert until Phase 1) | ≤ 1.85 |
| `uniffi` | 0.31.x | FFI binding generator (emend-ffi only) | ≤ 1.85 |
| `criterion` | 0.7.x | Benchmarking (0.8+ needs 1.86, intentionally excluded) | ≤ 1.85 |

**All versions verified at checkout**; MSRV pinning enforced by `cargo +1.85 check --all` in CI. No version bumps from memory — verified with `cargo update`.

**Location**: `Cargo.toml` ([workspace.dependencies], [workspace.package])

### Inert Dependencies (Not Yet Imported)

The following are pinned but **not used in code** until Phase 1 features land:
- `tree-sitter`, `tree-sitter-md` (Phase 1 T072)
- `comrak` (Phase 1 T073)
- `syntect` (Phase 1 T079)
- `nucleo` (Phase 1 T074–T076)
- `notify`, `notify-debouncer-full` (Phase 1 T065–T067)
- `reqwest` (Phase 1 T112)

This is intentional (Phase 0 planning resolved technical unknowns; Phase 1 imports as needed).

---

## Audit Logging

| Event | Logged Data | Retention | Status |
|-------|-------------|-----------|--------|
| **File operations** | Path, operation (read/write/delete), success/error | In-app debug logs (opt-in) | ✅ Implemented (fs.rs, error handling) |
| **AI requests** | Endpoint, model, request/response size, latency, error (never key) | In-app debug logs (opt-in) | Phase 1 T112 |
| **Security sandbox** | Bookmark grant/revoke events (via OS log) | System logs | Native (Security framework) |

**Notes**: All logs are development/diagnostic; no telemetry is sent off-device. Logs are cleared on app exit unless explicitly persisted to a debug file.

---

## Verification & Testing

### Security-Specific Test Coverage

| Test | Coverage | Location | Status |
|------|----------|----------|--------|
| **Panic containment** | FFI exports and spawned tasks never unwind; panic becomes `EmendError::Internal` | `crates/emend-ffi/tests/panic_containment.rs` | ✅ Implemented |
| **Atomic writes** | Kill between write+rename → original file intact | `crates/emend-core/tests/fs_atomic.rs` | ✅ Implemented |
| **Tolerant reads** | BOM/CRLF/invalid UTF-8 read correctly | `crates/emend-core/src/fs.rs` (unit tests, lines 186–221) | ✅ Implemented |
| **Bookmark resolution** | Add folder, quit, relaunch → reads + watches without new prompt | `app/Emend/EmendTests/BookmarkResolutionTests.swift` | ✅ Implemented (plain bookmarks, unsandboxed test) |
| **Bookmark scope lifecycle** | Stale bookmark re-creation, balanced start/stop calls | `app/Emend/EmendTests/BookmarkResolutionTests.swift` | ✅ Implemented |
| **AI privacy (offline)** | No network with AI unconfigured | `crates/emend-core/tests/ai_privacy.rs` | Phase 1 T110 |
| **AI key redaction** | Logs never contain key substring | Code review + test capture | Phase 1 T112 |
| **Preview offline** | WKWebView makes zero network calls | Airplane mode test + CSP verification | Phase 1 T083 |

---

## Known Limitations & Deferred Work

1. **AI features** (key redaction, privacy tests, timeout handling) are designed and specified but not yet implemented (Phase 1 tasks T110–T113).
2. **Security-scoped-bookmark validation** is tested with plain (non-security-scoped) bookmarks in the test process (which is unsandboxed). Full sandbox behavior is validated only in the signed, notarized app.
3. **Dependency vulnerability scanning** is not automated in CI; manual `cargo audit` checks recommended pre-release (tracked in TD-006).
4. **Performance regression testing** for incremental parsing is deferred (tracked in TD-002).

---

## What Does NOT Belong Here

- Tech debt and risks → CONCERNS.md
- Testing strategy → TESTING.md
- Code conventions → CONVENTIONS.md

---

*This document defines security controls. Update when security posture changes.*
