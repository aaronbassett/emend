# Security

> **Purpose**: Document authentication, authorization, security controls, and vulnerability status.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (US4 Phase 6 complete — preview + PDF export security controls)

## Overview

Emend is a privacy-first, offline-by-default Markdown editor governed by Constitution Principle II (Local-First & Privacy by Default) and NFR-006 (AI key handling). The app makes **zero outbound network calls unless the user explicitly configures a bring-your-own-model (BYOM) OpenAI-compatible AI endpoint AND invokes an AI action**. File access is sandboxed with app-scoped security-scoped bookmarks. Autosave is atomic and durable per Constitution Principle III. **US4 Phase 6 (complete)**: faithful Markdown preview with offline bundled Mermaid + KaTeX, PDF export with three-layer privacy enforcement (CSP + nonPersistent store + navigation delegate), comrak HTML escaping as the trust boundary for untrusted markdown input.

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
| **Lifecycle** | `NSOpenPanel` → `bookmarkData(options:.withSecurityScope)` → persist in UserDefaults → resolve on launch | `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift` |
| **Scope management** | `startAccessingSecurityScopedResource()` / `stopAccessingSecurityScopedResource()` (balanced per scope, held for session) | `SecurityScopedBookmarks.swift` (lines 54–60) + `WorkspaceModel.swift` (lines 136–138) |
| **Rust-Swift handshake** | Swift opens the scope at location registration; Rust file IO (`read_file_at`, `write_atomic`, `notify` watcher) operates within scope | `crates/emend-ffi/src/lib.rs` (exports) |
| **Stale bookmark refresh** | Transparently re-creates stale bookmarks on resolution; refreshed data persisted back to UserDefaults | `SecurityScopedBookmarks.resolve()` (lines 32–48) |
| **Validation** | Security-scoped behavior verified in the signed app; tests use plain bookmarks (test process is unsandboxed) | `app/Emend/EmendTests/BookmarkResolutionTests.swift` |
| **Scope lifecycle tests** | Plain bookmarks (no `.withSecurityScope`) parameterized into `resolve()` / `makeBookmark()`; stale-bookmark refresh tested | `app/Emend/EmendTests/BookmarkResolutionTests.swift` |

### Path Identity (NFR-007: Symlink Cycles & Case Folding)

Rust enforces canonical path identity in the workspace layer to prevent symlink-cycle attacks and ensure the same physical file has one identity:

| Mechanism | Implementation | Location |
|-----------|---|---|
| **Canonicalization** | Every path resolved via `std::fs::canonicalize()` (resolves symlinks, `..`, and normalizes case on case-insensitive volumes) | `crates/emend-core/src/workspace.rs::canonical_id()` |
| **Bounded traversal** | `Workspace::collect_files()` caps recursion depth + maintains canonical path set to detect already-visited directories (symlink cycle termination) | `crates/emend-core/src/workspace.rs` (lines 200+) |
| **Same-file deduplication** | Identity is the symlink-resolved absolute path; `HashSet<PathBuf>` dedupes lexical aliases to same inode | `crates/emend-core/src/workspace.rs::canonical_id()` |
| **Case-insensitive volumes** | Canonicalize respects host filesystem semantics; `Note.md` and `note.md` resolve to one inode on macOS (correct behavior) | `crates/emend-core/src/workspace.rs` (research §A3) |

---

## Workspace & Location Management

### Location Addition & Persistence

| Aspect | Implementation | Location |
|--------|---|---|
| **User grants folder** | `NSOpenPanel` prompts user; Swift resolves to URL and creates security-scoped bookmark | `SecurityScopedBookmarks.promptForFolder()` (lines 64–73) |
| **Rust registration** | Swift calls `add_location(folder_path, bookmark)` → Rust stores path (bookmark stays Swift-side) | `crates/emend-core/src/workspace.rs::add_location()` |
| **Bookmark persistence** | Swift persists bookmarks in UserDefaults under `com.aaronbassett.Emend.locationBookmarks` | `WorkspaceModel.swift::persistBookmarks()` (lines ~330) |
| **AppState persistence** | Favorites, pins, custom folder icons stored separately in UserDefaults under `com.aaronbassett.Emend.appState` | `WorkspaceModel.swift::AppState` (lines 33–37) + `saveAppState()` |
| **Launch replay** | On app start, Swift restores bookmarks → resolves each → opens scope → passes path to Rust to rebuild workspace state | `WorkspaceModel.init()` (lines 60–66) |

### Location Removal & Scope Cleanup

| Aspect | Implementation | Location |
|--------|---|---|
| **Scope release** | When location removed, Swift calls `stopAccessingSecurityScopedResource()` to balance scope open | `WorkspaceModel.removeLocation()` (lines 136–138) |
| **Watcher cleanup** | Watcher handle for location stopped before scope release | `WorkspaceModel.removeLocation()` (line 133) |
| **Bookmark deletion** | Bookmark removed from UserDefaults | `WorkspaceModel.removeLocation()` + `persistBookmarks()` |

---

## Live File-System Monitoring & Conflict Handling

### Self-Write Suppression (FR-006a)

Autosave must not trigger an external-change notification (race condition). Implementation uses **identity-based suppression** (not time-window):

| Mechanism | Detail | Location |
|-----------|--------|----------|
| **Post-write stat** | After atomic `rename()`, Rust calls `FileIdentity::of_path()` to capture `(mtime_ns, len)` | `crates/emend-core/src/watcher.rs::FileIdentity` (lines 78–94) |
| **Registry recording** | `SuppressionRegistry::record()` stores identity with ~300 ms TTL (injected `Instant` for testing) | `crates/emend-core/src/watcher.rs::SuppressionRegistry` (lines ~200+) |
| **Event suppression** | When debounced modify event arrives, `take_if_self_write()` checks if path's **current identity** matches recorded one; if yes, suppress event (consume one record) | `crates/emend-core/src/watcher.rs::SuppressionRegistry::take_if_self_write()` |
| **Third-party immunity** | A genuine third-party edit in the same time window changes `mtime`/`len`, so it is **not** suppressed (contract obligation 4) | `crates/emend-core/src/watcher.rs` (research §B3, line 38–49) |
| **Double-layer protection** | Rust registry (stat-based) + Swift-side time window (authoritative) ensures autosave never echoes | `WorkspaceModel.swift` (async watcher task) + `crates/emend-core/src/watcher.rs` |

### Conflict Resolution (FR-006c)

When a file changes on disk while the buffer is dirty, the conflict model preserves unsaved work:

| Scenario | Behavior | Code |
|----------|----------|------|
| **File changed on disk, buffer clean** | Silent reload (external version is authoritative) | `crates/emend-core/src/watcher.rs::resolve_conflict()` → `ConflictAction::Reload` |
| **File changed on disk, buffer dirty** | Preserve local edits; flag document as externally-changed; UI offers Reload or Keep | `crates/emend-core/src/watcher.rs::resolve_conflict()` → `ConflictAction::PreserveLocal` |
| **Self-write detected** | Suppress the event (no conflict, just our own save) | `SuppressionRegistry::take_if_self_write()` → `ConflictAction::Ignore` |

**Location**: `crates/emend-core/src/watcher.rs` (lines 51–60 truth table)

**Note on testing**: Self-write suppression + rename correlation are deferred to Phase 1 (T065–T066). Live-refresh path depends on real OS filesystem events (not exercised by headless tests) — needs manual UI verification in signed app.

---

## Preview & PDF Export Security (US4)

### Preview Web View Isolation (FR-035, SC-008)

The preview pane (`WKWebView`) renders untrusted Markdown-derived HTML with three layers of privacy enforcement to guarantee **zero outbound network access**:

| Layer | Implementation | Purpose | Location |
|-------|---|---|---|
| **CSP** | `Content-Security-Policy` header: `default-src 'none'`, `connect-src 'none'`, `script-src 'self'`, `img-src 'self' data:`, `style-src 'self' 'unsafe-inline'` | Prevents remote resource loads; allows only bundled assets + data URIs | `Resources/preview/template.html` (research §C2) |
| **Non-persistent store** | `config.websiteDataStore = .nonPersistent()` | No cookies, localStorage, or persistent caches across sessions | `app/Emend/Emend/Preview/PreviewWebView.swift::PreviewWebView.makeNSView()` (line 24) |
| **Navigation delegate** | `WKNavigationDelegate.decidePolicyFor()` whitelists only `file:` + `about:` URLs; external links are intercepted and opened in the user's browser | Blocks navigation to remote origins; preserves user intent for external links | `app/Emend/Emend/Preview/PreviewWebView.swift::Coordinator.webView(_:decidePolicyFor:)` (lines 96–115) |

**Verification**: Defensive test `crates/emend-core/tests/preview_offline.rs` (Phase 1 T083) verifies the **core rendering path** emits only literal URL attributes (never fetched).

### Markdown Trust Boundary (Comrak Escaping)

Untrusted user Markdown is the threat model; the trust boundary is **comrak's HTML escaping**:

| Aspect | Implementation | Location |
|--------|---|---|
| **Raw HTML escaping** | comrak default: `unsafe_` = `false`. User-supplied HTML tags are entity-escaped (e.g., `<script>` → `&lt;script&gt;`) | `crates/emend-core/src/parse/preview.rs::build_options()` (line 129) |
| **Remote URLs emitted as-is** | Image + link URLs are emitted as literal `src=` / `href=` attributes; comrak does **not** dereference them | `crates/emend-core/src/parse/preview.rs::build_options()` (comment lines 131–132) |
| **Code block syntax highlighting** | Colored code is wrapped in `<span class=…>` (syntect classed HTML, no inline styles or `<script>` | `crates/emend-core/src/parse/code_highlight.rs` (inert until Phase 1 T079) |
| **No embed processing** | Embed syntax `![[…]]` renders literally; embed resolution (with cycle/depth guards) is deferred to US5 | `crates/emend-core/src/parse/preview.rs::build_options()` (lines 120–123) |

**Verification**: Test suite `crates/emend-core/tests/preview_render.rs` validates that comrak renders GFM, wikilinks, and highlight syntax correctly without injecting extra HTML.

### PDF Export Offline Guarantee

The off-screen PDF export path (`PDFExport`) uses an identical offline template + privacy stack to the on-screen preview:

| Stage | Implementation | Location |
|-------|---|---|
| **Template load** | Off-screen `WKWebView` loads the same bundled `template.html` (grants read access to `preview/` dir for KaTeX, Mermaid, CSS) | `app/Emend/Emend/Preview/PDFExport.swift::OffscreenPrintHost.loadTemplate()` (lines 66–76) |
| **Non-persistent store** | PDF export WebView has `config.websiteDataStore = .nonPersistent()` | `PDFExport.swift::OffscreenPrintHost.makeWebView()` (line 134) |
| **Content injection** | HTML + CSS injected via `callAsyncJavaScript()` (same `window.__emendRender()` bridge as on-screen preview) | `PDFExport.swift::OffscreenPrintHost.renderContent()` (lines 79–90) |
| **Watchdog timeout** | Template load and print operations have 20 s + 30 s timeouts respectively to prevent hangs | `PDFExport.swift` (lines 69, 107) |
| **Print pagination** | Delegates to `NSPrintOperation` with `@media print` CSS rules in `theme.css` for true multi-page output (avoids `createPDF`'s single-tall-page limitation) | `PDFExport.swift::OffscreenPrintHost.paginate()` (lines 97–120) |

**Privacy guarantee**: PDF export is offline and uses the same isolation (CSP + nonPersistent + offline assets) as the on-screen preview. The printed output is deterministic (Mermaid + KaTeX are bundled and run synchronously in the off-screen view).

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
| **Preview is offline** | WKWebView with CSP blocking remote loads, `nonPersistent` store, navigation delegate cancels non-`file:`/`about:` URLs, PDF export identical | ✅ US4 T087 (implemented) |
| **Bundled assets** | Mermaid.js + KaTeX vendored locally; loaded via `loadFileURL`, not CDN | ✅ US4 (implemented) |
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
| `PermissionDenied { path }` | App lacks permission (sandbox scope exhausted or bookmark stale) | Suggest re-grant folder access or refresh bookmark |
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
| **File paths** | Resolved within scope of security-scoped bookmark; no `..` traversal above location root; symlink cycles detected via canonicalization + bounded recursion | `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift` + `crates/emend-core/src/workspace.rs` | ✅ Implemented |
| **File content (read)** | Tolerant reads: UTF-8 BOM stripped, CRLF preserved, invalid UTF-8 decoded lossily (not rejected) | `crates/emend-core/src/fs.rs` | ✅ Implemented |
| **File content (write)** | Atomic write via tempfile; no special escaping (Markdown is plain text) | `crates/emend-core/src/fs.rs::write_atomic` | ✅ Implemented |
| **Markdown syntax (edit)** | tree-sitter (editor) handles malformed input gracefully (incremental reparse, no crash) | `crates/emend-core/src/parse.rs` | Phase 1 T072 |
| **Markdown syntax (preview)** | comrak (CommonMark) handles malformed input gracefully (no crash, renders as-is) with HTML escaping (no raw user HTML) | `crates/emend-core/src/parse/preview.rs` | ✅ US4 T084 (implemented) |
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
| `comrak` | 0.52.x | CommonMark preview; HTML escaping enforces trust boundary | ≤ 1.85 |
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
| **Path identity** | Symlink cycles terminated; same file via two paths has one identity | `crates/emend-core/tests/workspace_ops.rs` (Phase 1 T048) | Headless test ready; manual signed-app test pending |
| **Self-write suppression** | Post-persist `(mtime,len)` suppresses matching event; third-party edits not suppressed | `crates/emend-core/tests/watcher_unit.rs` (planned Phase 1 T066) | Deferred to Phase 1 |
| **Conflict resolution** | Clean buffer + external change → reload; dirty buffer + external change → preserve local + flag | `crates/emend-core/tests/watcher_unit.rs` (planned Phase 1 T067) | Deferred to Phase 1 |
| **Preview offline (core path)** | Markdown render is a pure `&str -> String` function; remote URLs emitted as literal `src=`/`href=` (never fetched) | `crates/emend-core/tests/preview_offline.rs` | ✅ US4 T083 (implemented) |
| **Preview HTML rendering** | GFM, wikilinks, highlight syntax render correctly; comrak escapes raw HTML | `crates/emend-core/tests/preview_render.rs` | ✅ US4 T084 (implemented) |
| **Preview CSP + isolation** | WKWebView enforces CSP, nonPersistent store, navigation delegate blocks remote loads | Manual signed-app test + `app/Emend/EmendTests/PreviewExportTests.swift` | ✅ US4 (partial; runtime path requires manual UI verification) |
| **PDF export offline** | Off-screen export uses same isolation (CSP + nonPersistent + bundled assets) as on-screen preview | `app/Emend/EmendTests/PreviewExportTests.swift` | ✅ US4 (implemented) |
| **AI privacy (offline)** | No network with AI unconfigured | `crates/emend-core/tests/ai_privacy.rs` | Phase 1 T110 |
| **AI key redaction** | Logs never contain key substring | Code review + test capture | Phase 1 T112 |

---

## Known Limitations & Deferred Work

1. **AI features** (key redaction, privacy tests, timeout handling) are designed and specified but not yet implemented (Phase 1 tasks T110–T113).
2. **Security-scoped-bookmark validation** is tested with plain (non-security-scoped) bookmarks in the test process (which is unsandboxed). Full sandbox behavior is validated only in the signed, notarized app.
3. **Live-refresh path** (file watcher events, conflict handling) depends on real OS filesystem events (not exercised by headless tests) — needs manual UI verification in signed app.
4. **Dependency vulnerability scanning** is not automated in CI; manual `cargo audit` checks recommended pre-release (tracked in TD-006).
5. **Performance regression testing** for incremental parsing is deferred (tracked in TD-002).
6. **Folder move re-pathing** for descendants' favorite/pin state is deferred (known limitation captured in CONCERNS.md).
7. **Preview scroll-sync runtime path** (Section §C3): the pure core logic for data-line anchors is tested; the runtime integration (editor↔preview scroll sync) requires manual UI verification in the signed app.
8. **Embed resolution** (`![[…]]` with cycle/depth guards) is deferred to US5 and will require a future security review of embed trust boundaries.

---

## What Does NOT Belong Here

- Tech debt and risks → CONCERNS.md
- Testing strategy → TESTING.md
- Code conventions → CONVENTIONS.md

---

*This document defines security controls. Update when security posture changes.*
