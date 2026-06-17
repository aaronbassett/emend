# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-18 (incremental: US6 info sidebar + BYOM AI summary)

## Architecture Overview

Emend is a **hybrid Rust+Swift native macOS Markdown editor** with a cleanly separated boundary:

- **Rust core** (`crates/emend-core`) houses ALL business logic: file I/O, document parsing, preview rendering (comrak + syntect), file watching, indexing, search, link/task/embed resolution, attachments, BYOM AI client (pure SSE parser + request building + secret redaction), and per-document stats/outline for the info sidebar. The core is **toolchain-free** — it has no FFI dependency and is fully testable with `cargo test` in isolation.
- **UniFFI shim** (`crates/emend-ffi`) provides a thin boundary layer that exports the core's capabilities to Swift and manages async infrastructure (tokio runtime, cancellation tokens). The FFI is the **only place reqwest lives** — the core stays tokio/reqwest-free.
- **Swift/SwiftUI app** (`app/Emend`) wraps the core in a native macOS UI with a four-pane layout: sidebar (workspace/favorites), tabbed editor (with per-document state, US5 link/task UI), live preview pane (US4, with inline embeds US5), a ⌘P Quick Open palette (US3), and an info sidebar (US6, with stats/outline/AI summary).

The boundary is **synchronous on the hot path** (per-keystroke edits) and **asynchronous only for AI and search** (with cancellable Rust-owned handles). Preview rendering is debounced off the keystroke path and runs off-main-thread. AI summary streams token-by-token to a foreign sink. This design allows the core to stay independent and testable while the UI safely dispatches background work.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Layered (horizontal)** | Presentation (Swift/SwiftUI) → API boundary (UniFFI) → Business logic (Rust core) |
| **Modular monolith** | Single deployable macOS app; no microservices or network splits |
| **Rust corelib + FFI shim** | Heavy separation of concerns: business logic in pure Rust, FFI concerns isolated in a thin wrapper |
| **Synchronous hot path, async background** | Per-keystroke edits cross the boundary synchronously; AI and search use async Rust-owned handles; preview renders debounced off-main-thread |
| **UTF-16 boundary contract** | All text ranges crossing the FFI boundary are UTF-16 code units, mapping 1:1 to `NSRange` |
| **Swift owns text buffer** | Canonical text storage lives in NSTextStorage; Rust maintains a shadow ropey rope for structural queries |
| **Core owns preview HTML + theme CSS + embeds** | Core generates markdown→HTML via comrak, syntect code-highlight CSS, and resolves embeds against the workspace index; Swift embeds these offline into a bundled WKWebView template |
| **Clear model/view separation (Swift UI)** | `@MainActor` state models (`WorkspaceModel`, `TabModel`, `ConflictController`, `QuickOpenModel`, `PreviewModel`, `InfoModel`) own Rust handles and drive views; views are pure presentations of model state |
| **AI client is pure + FFI** (US6) | **Decision logic (SSE parser, request builder, redacting key) lives in core (tokio/reqwest-free); transport (HTTP, streaming) lives in FFI only** |

## Core Components

### 1. Rust Core (`crates/emend-core`)

**Purpose**: The engine — file I/O, document state, parsing, preview rendering, search, watching, link/task/embed/attachment resolution, pure AI client (SSE parser + request builder + secret redaction), and per-document stats/outline.

**Location**: `crates/emend-core/src/`

**Modules**:

- **`error.rs`** — Structured `EmendError` type (single source of truth for FFI contract). Variants carry context fields (paths, limits, byte counts) for UI rendering. Exhaustive enum (not `#[non_exhaustive]`) so the FFI projection can be a closed, compiler-checked mirror.
- **`fs.rs`** — Atomic+durable writes and tolerant reads. Write path: temp file in same directory → fsync → atomic rename → fsync directory (guarantees no torn writes). Read path: strips UTF-8 BOM, preserves CRLF, decodes invalid UTF-8 lossily. On macOS, `File::sync_all()` already calls `fcntl(F_FULLFSYNC)` for true durability.
- **`document.rs`** — The open-document model: a shadow ropey rope + UTF-16/char/line indices. Backs all per-keystroke edits, structural queries (highlight, outline), and search. Converts at exactly one place on every boundary call, never panicking — all conversions are checked and mapped to `EmendError`.
- **`workspace.rs`** — File-based workspace model (US2): locations (user-chosen root folders), lazy directory listing, collision-safe file operations, in-memory maps for favorites/pins/icons/child-order. Uses canonicalization + bounded traversal for path identity (NFR-007) and `free_name` for collision-safe naming (FR-004a). Exposes `resolve_embed_source` (US5) for embed resolution. Pure `std` + `tempfile`; no async.
- **`index.rs`** — Incremental in-memory search index (US2): arena-based entries, path/name maps, fuzzy ranking via `nucleo-matcher`. Maintained O(1)-ish on file operations (create/rename/move/delete) via `Index::insert/remove/rename`, never full rescan (FR-017a). Backs Quick Open + wiki-link resolution (FR-019). `resolve_wikilink` implements deterministic tie-break for duplicate basenames (same dir → shallowest → lexicographic).
- **`search.rs`** (US3) — Pure, tokio-free streaming search driver (T073). Owns the **emission policy** for Quick Open: batches ranked results from `Index::query()` and re-checks a `Cancel` flag between batches for fast supersede (NFR-002). Holds `pub struct Cancel` (Arc-backed atomic bool) so multiple clones share the cancellation state. The core decision logic (rank, batch, stop-on-supersede) lives here for unit testability without an async runtime (`tests/search_supersede.rs`). No `uniffi` or `tokio` dependencies.
- **`watcher.rs`** — Live file watching (US2): thin `notify` + `notify-debouncer-full` wrapper over a pure, deterministically-tested classification core. Includes move correlation (FR-006b), self-write suppression via identity-keyed registry (FR-006a), and conflict truth table (FR-006c). Runs on OS threads, posts to `std::sync::mpsc`; no async runtime.
- **`parse.rs`** (US4 · T084) — Markdown parsing: deliberately **two separate engines** (Constitution): incremental tree-sitter (editor highlight, advisory) vs. comrak (preview HTML, authoritative). Held apart on purpose, never unified.
  - **`parse/highlight.rs`** — Incremental tree-sitter highlighting for the editor (advisory, fast, on-keystroke path).
  - **`parse/preview.rs`** (US4 · T084) — **Authoritative comrak preview engine**: CommonMark + GFM + native `[[wikilink]]` + `==highlight==` extensions, with `data-line` scroll-sync anchors and syntect-coloured code blocks. Pure transform (no I/O, no async, no network). `render_preview_html_with_embeds` resolves embeds (US5) via a caller-supplied resolver.
  - **`parse/code_highlight.rs`** (US4 · T084) — syntect-based HTML code colouring for preview fenced blocks; vendored binary syntax/theme dump loaded once per session.
  - **`parse/embed.rs`** (US5 · T097) — Embed resolution logic: given a resolver callback (workspace index), inlines embedded notes' content into the HTML tree; handles missing/recursive embeds gracefully.
- **`derived.rs`** (US5 · T097 / US6 · T111) — Per-document link/task scanning, resolution, and stats (FFI contract §5/§4, FR-014..022/FR-029..031). Pure `std` + `index`; no `uniffi` or `tokio`.
  - **`extract_links`** — Scan document source for `[[wiki links]]` and `![[embeds]]`, returning UTF-16-ranged `LinkRef`s for editor click/navigation/styling.
  - **`resolve_wikilink`** — Apply FR-019a's deterministic tie-break to pick one target from the index's candidate set (same dir → shallowest → lexicographic).
  - **`wikilink_suggestions`** — Autocomplete candidates for `[[` (fuzzy ranking via index).
  - **`toggle_task`** — Flip the `[ ]`/`[x]` of a task on a given line (returns new document text, unit-testable via `tests/links.rs`).
  - **`stats()`** (US6 · T111) — **Word/char/reading-time/task counts over document source (FR-029/030)**; recomputed cheaply on each edit.
  - **`outline()`** (US6 · T111) — **ATX heading scan with 1-based line numbers for click→scroll in info sidebar (FR-031/031a)**; pure, O(n) scan.
- **`ai.rs`** (US6 · T112) — **Pure, tokio-free SSE parser + request builder + secret redaction** (FR-032/035/036/036a, NFR-006, research §B5). Deliberately split from FFI (core stays tokio/reqwest-free):
  - **`SseParser`** — Buffers raw response bytes, emits complete UTF-8 tokens, tolerates CRLF/LF, handles `data: [DONE]`, skips malformed payloads (lenient framing for BYOM endpoints).
  - **`ApiKey`** — Redacting newtype whose `Debug` and `Display` render `***` (never the secret); only readable via explicit `expose()`.
  - **`check_input_size`** — Rejects oversized documents locally, before any network send (FR-036a).
  - **`build_request_body` / `build_auth_header`** — Pure request builders (headers/body as data, no network).
  - **`DEFAULT_SUMMARY_SYSTEM_PROMPT`** — Conventional system instruction, testable + consistent.

**Dependencies**: `thiserror`, `ropey`, `tempfile`, `tree-sitter`, `tree-sitter-md`, `comrak`, `syntect`, `nucleo`, `nucleo-matcher`, `notify`, `notify-debouncer-full`, `serde` (only for AI SSE deserialization, not for serialization).

**Dependents**: `crates/emend-ffi`, tests, benchmarks.

**Constraints (Constitution V)**:
- **NO FFI dependency** — core never imports `uniffi`. This keeps the core standalone and testable.
- **NO panics** — clippy lints deny `unwrap_used`, `expect_used`, `panic`.
- **No async primitives, no reqwest** — tokio and reqwest only enter in the FFI shim (async infrastructure + transport live outside the core).
- **Two markdown engines, not one** — tree-sitter (editor) and comrak (preview) are never unified; their performance and correctness profiles differ (Constitution).

### 2. FFI Shim (`crates/emend-ffi`)

**Purpose**: Thin projection of the core to Swift. Manages async infrastructure, panic containment, error mapping, and **the only place reqwest lives**. Bridges the tokio runtime, cancellation, and foreign-trait sinks.

**Location**: `crates/emend-ffi/src/`

**Modules**:

- **`lib.rs`** — FFI entry points (e.g., `read_file_at`, `core_abi_version`, `preview_theme_css`). Each `#[uniffi::export]` function wraps a core function, handling errors. UniFFI's scaffolding automatically wraps calls in `catch_unwind`, so panics cannot unwind into Swift.
- **`error.rs`** — `#[derive(uniffi::Error)]` projection of `EmendError`. Keeps the same `Display` wording so Swift sees the same error message. Exhaustive `From` impls ensure new core error variants force compilation errors here.
- **`panic.rs`** — Custom panic hook (if needed for debugging; not yet implemented).
- **`document.rs`** — FFI projection of document operations: `open_document`, `close_document`, `push_edit`, `highlight_spans`, `render_preview_html`, `flush`, `extract_links` (US5 · T097), **`stats`/`outline`** (US6 · T111). Wraps the core's `Document` in an `OpenDocHandle` (`Arc<Mutex<Document>>`).
  - **`stats()`** (US6 · T111) — **Call core's derived stats, return `DocStats` value type for info sidebar.**
  - **`outline()`** (US6 · T111) — **Call core's derived outline scan, return `Vec<OutlineItem>` for info sidebar tree.**
  - **`render_preview_html()`** (US4 · T084) — Calls the core's comrak preview engine, returning HTML suitable for injection into the WKWebView template.
  - **`render_preview_html_resolving(workspace)`** (US5 · T097) — Resolves embeds against the workspace's index before rendering (FFI contract §6). Drops locks before rendering/IO to avoid deadlock.
  - **`extract_links()`** (US5 · T097) — Scan the document for `[[wiki links]]` and `![[embeds]]`, returning UTF-16-ranged `LinkRef`s for editor UI.
  - **`toggle_task(offset)`** (US5 · T097) — Flip a task checkbox on the line containing the offset (returns new document text, integrated into undo via `push_edit`).
  - **`store_attachment(data, ext)`** (US5 · T097) — Store a binary attachment (image, etc.) beside the note with collision-safe naming, returning a note-relative ref for insertion.
- **`workspace.rs`** — FFI projection of workspace + index: `WorkspaceHandle` wrapping both `Workspace` + `SharedIndex` (Index behind `Arc<Mutex<>>`) co-located under one `Mutex<Inner>` (to maintain incremental index updates in lock-step, FR-017a). Exports `Location`, `FsNode`, `NodeKind` value types and methods like `create_note`, `rename`, `move_node`, `query`, `resolve_name`, `quick_open_query` (T074), `reindex_all` (T078), `resolve_wikilink` (US5 · T097), `resolve_embed_source` (US5 · T097), and `wikilink_suggestions` (US5 · T097).
- **`search.rs`** (US3 · T074) — FFI projection of streaming Quick Open (§5 of contract). Drives the pure, tokio-free core search driver (`emend_core::search::quick_open`) on the boundary's shared tokio runtime and forwards ranked batches to foreign `SearchSink`. Holds `pub struct SearchHandle` (UniFFI Object, `Arc<Self>`), which bridges cancellation: a `tokio_util::CancellationToken` (parity with `CancellationHandle`) and an `emend_core::search::Cancel` flag. A single `quick_open_query` cancels any previous `SearchHandle` in `WorkspaceHandle.current_search` (supersede, NFR-002), then spawns the new worker. The core driver is fast (<100 ms p95, SC-004); lock the index briefly, not the whole workspace.
- **`watcher.rs`** — FFI projection of the live watcher: `WatchHandle` wrapping `FsWatcher`, with `ChangeEvent` and `ConflictState`/`ConflictChoice` enums. Bridges via `ObserverBridge` to the Swift `DocObserver` foreign trait.
- **`ai.rs`** (US6 · T112/T113) — **FFI streaming AI client (the ONLY reqwest + tokio user in the Rust boundary)**. Wraps core's pure SSE parser / request builders:
  - **`summarize_document(handle, config, api_key, sink)`** (US6 · T112) — **Start a streaming AI summary; pre-validates (blank key → no network, oversized doc → no network); spawns on tokio; streams tokens to foreign `AiSink`; returns `AiHandle` for cancellation.**
  - **`test_ai_config(cfg, api_key)`** (US6 · FR-037) — **Validates AI config with a minimal probe request; sync at boundary (blocks on tokio), transient key (never persisted).**
  - **`AiHandle.cancel()`** (US6) — **Idempotent cancel via `CancellationToken`; on_error(AiCancelled) terminal, no further tokens.**
  - **Per-chunk inactivity timeout (research §B5)**: `tokio::time::timeout` on each `stream.next()` — NOT a whole-request deadline (which would fire mid-stream on slow models).
- **`handles.rs`** — Rust-owned infrastructure: the tokio runtime (lives here, not core), `CancellationToken` for async tasks, `SearchHit` value type, `LinkRef` and `LinkKind` value projections (US5 · T097), **`DocStats`, `OutlineItem`** (US6), and foreign-trait `DocObserver`/`SearchSink`/`AiSink` callbacks for streaming results.

**Dependencies**: `emend-core`, `uniffi`, `thiserror`, `tokio`, `tokio-util`, `tempfile`, `reqwest` (ONLY in this crate), `futures-util`.

**Dependents**: Swift package (`swift/EmendCore`).

**Constraint**: Keep FFI free of business logic. It is ONLY a projection and scaffolding layer (except for streaming orchestration, which is unavoidable).

### 3. Swift Package (`swift/EmendCore`)

**Purpose**: Local SwiftPM package wrapping the Rust XCFramework and offering idiomatic Swift re-exports.

**Location**: `swift/EmendCore/`

**Layout**:

- **`Package.swift`** — Package manifest. Declares two targets: `EmendCoreFFI` (generated UniFFI bindings, compiled in Swift 5 for foreign-trait compatibility) and `EmendCore` (hand-written Swift wrappers, Swift 6).
- **`Sources/EmendCoreFFI/`** — Generated by `uniffi-bindgen-swift` (git-ignored). Exposes the C module and the `emend_ffiFFI` namespace.
- **`Sources/EmendCore/`** — Hand-written wrappers that re-export the generated surface and add idiomatic adapters (e.g., `AsyncStream` over foreign-trait sinks) as features land.
- **`../../xcframework/EmendCore.xcframework`** — The compiled Rust binary + C module headers (git-ignored, built by `just xcframework`).

**Dependents**: Xcode app (`app/Emend`).

### 4. macOS App — Model & Orchestration Layer

**Purpose**: Swift `@MainActor` state models that own the Rust handles and coordinate between workspace/tabs/editor/conflicts/search/preview/links/info/AI.

**Location**: `app/Emend/Emend/`

**Key models**:

- **`WorkspaceModel`** (`Sidebar/WorkspaceModel.swift`) — `@MainActor ObservableObject` owning the `WorkspaceHandle` (Rust workspace + index). Manages the security-scoped folder bookmark lifecycle, per-location file watchers, the sidebar's root nodes (locations + Favorites group). Publishes `roots` (the outline tree), `revision` (bumped when locations change), and `fsRefreshTick` (for targeted outline reloads). Persists app state (favorites/pins/icons) to UserDefaults.

- **`TabModel`** (`Tabs/TabModel.swift`) — `@MainActor ObservableObject` owning the list of open documents (tabs). Each `Tab` holds its own `OpenDocHandle`, initial text, `AutosaveController`, and a reload token. Manages tab creation/closing, active selection, status messages. Coordinates with `ConflictController` on external changes. Publishes `activeID` so view transitions can swap the active editor/preview.

- **`ConflictController`** (`Editor/ConflictController.swift`) — `@MainActor ObservableObject` detecting when an open document changes on disk underneath the editor (FR-006c). Listens to workspace's `onExternalChange` callback and tracks self-writes from autosave. Flags conflicted tabs; users choose reload-or-keep-mine. Maintains both core-side suppression (identity-keyed) and Swift-side time-window guards to avoid false positives.

- **`QuickOpenModel`** (`QuickOpen/QuickOpenModel.swift`) — `@MainActor ObservableObject` driving the ⌘P Quick Open palette (US3, FR-017/FR-018, NFR-002). Bridges the core's streaming, supersedable `quick_open_query` to SwiftUI: each keystroke starts a fresh query that supersedes the prior (the core cancels the previous in-flight `SearchHandle`; a monotonic `generation` additionally guards against a late batch from a superseded query landing after the next one began). Ranked `SearchHit`s stream in via a `SearchSink` bridge; arrow keys move the selection, Return opens the file.

- **`PreviewModel`** (US4 · `Preview/PreviewModel.swift`) — `@MainActor ObservableObject` driving the live Markdown preview pane (FR-022/FR-025, research §B1). Debounced ~150 ms off the editor's `onDocEdit` signal; renders via the core's `renderPreviewHtml` (comrak + syntect, authoritative, with embeds resolved US5) off the main thread (NFR-001). Holds the syntect theme CSS (core-owned, fetched once). Publishes `html` (rendered body fragment) and `version` (bumped on each successful render even if HTML unchanged). When the preview pane becomes visible, schedules an immediate refresh; while hidden, renders are skipped to avoid wasted work. Calls `renderPreviewHtmlResolving` to inline embeds (US5).

- **`ScrollSync`** (US4 · `Preview/ScrollSync.swift`) — `@MainActor ObservableObject` managing bidirectional editor ↔ preview scroll sync (FR-024, research §C3). Both sides keyed on 1-based source line numbers: comrak annotates blocks with `data-line` (via core's scroll-sync anchors); bridge.js builds an anchor table. Editor→preview maps the top visible character's line and calls `__emendScrollToLine`; preview→editor receives the top line from the page. Per-side mute window (160 ms) guards the feedback loop to avoid echoing.

- **`InfoModel`** (US6 · `Info/InfoModel.swift`) — **`@MainActor ObservableObject` powering the info sidebar (FR-029..031). Holds the active document's `DocStats` (word/char/reading-time/tasks) and `OutlineItem`s (headings). Stats are recomputed on each edit via `document.stats()`; outline is scanned on-demand. Publishes them for display + click→scroll integration.**

- **`AIConfigStore`** (US6 · `AI/AIConfig.swift`) — **Swift-side BYOM configuration holder: base URL, model, timeout, max input. Persisted to UserDefaults. No key is stored (it lives in Keychain only, read on each request).**

- **`SummaryModel`** (US6 · `AI/SummaryView.swift`) — **`@MainActor ObservableObject` for one streaming AI summary. Holds accumulated `fullText`, per-token updates via `AiSink` bridge, cancel handle, terminal state (done/error). Attached to a specific tab/document.**

### 5. macOS App — View Layer

**Purpose**: SwiftUI views and editor mechanics.

**Location**: `app/Emend/Emend/`

**Major components**:

- **`Shell/MainWindow.swift`** — Four-pane layout (US6): sidebar (workspace outline) | editor pane (tabbed) | preview pane (toggled via "Toggle Preview" toolbar button) | info sidebar (toggled via "Toggle Info" button). Wires up `WorkspaceModel`, `TabModel`, `ConflictController`, `QuickOpenModel`, `PreviewModel`, `ScrollSync`, and `InfoModel`. Includes Export PDF toolbar action (US4). Hidden ⌘P button registers the Quick Open shortcut window-wide. Settings menu launches AI config view.

- **`Sidebar/WorkspaceOutlineView.swift`** — `NSViewRepresentable` wrapping `NSOutlineView` over the workspace's file tree. Lazy children loading, targeted reloadItem on external FS changes. Context menu + drag-drop.

- **`Sidebar/OutlineDragDrop.swift`** — Drag-drop handlers for moving/renaming files in the outline.

- **`Tabs/TabBarView.swift`** — Tab bar rendering the open documents, active selection, close buttons.

- **`Editor/MarkdownEditorView.swift`** — `NSViewRepresentable` wrapping a TextKit 2 `MarkdownTextView` + coordinate per-keystroke edits. Registers with `ScrollSync` for editor→preview scroll bridging. Wires up wiki-link UI (US5): native completion on `[[`, ⌘-click navigation via `EditorCoordinator`.

- **`Editor/MarkdownTextView.swift`** — `NSTextView` subclass that hooks list/formatting keys (Return, Tab, ⌘B/I/K/⇧T) to pure transforms. Integrates task-checkbox clicking (US5) and image drag-drop (US5).

- **`Editor/EditorCoordinator.swift`** — `NSTextStorageDelegate` that extracts UTF-16 deltas, calls `pushEdit()` synchronously, then signals `PreviewModel.scheduleRefresh()` to debounce the preview render. US5: owns workspace handle for wiki-link resolution, click navigation, and link-autocomplete. US6: signals `InfoModel.refresh()` to recompute stats/outline.

- **`Editor/SyntaxAttributing.swift`** — Pure function mapping core `StyleSpan`s to AppKit display attributes (bold/italic/headings/code/quote inline, markers dimmed).

- **`Editor/SmartLists.swift`** — Pure transforms: `newline()` (continue/terminate list), `renumber()` (sequential ordered lists), `indent()`/`outdent()` (shift nesting).

- **`Editor/FormattingCommands.swift`** — Pure transforms: `bold()`, `italic()`, `link()`, `task()` (insert markers around selection).

- **`Editor/AutosaveController.swift`** — Debounced (1.5 s idle + 5 s hard cap) atomic flush on private serial queue. Errors surface via callback. Also registers self-writes with the core's conflict suppression.

- **`Editor/TaskCheckbox.swift`** (US5 · T097) — Pure helpers for task-checkbox detection: `checkboxRange(in:atLineContaining:)` and `toggleEdit(in:atLineContaining:)` return the checkbox range or a toggle edit (pure, headless-testable).

- **`Editor/ImageDrop.swift`** (US5 · T097) — Pure helpers for image drag-drop: `imageFileURLs(in:)` filters dropped URLs, `markdown(forImageRef:)` generates the Markdown `![](ref)` line.

- **`Links/WikiLinkAutocomplete.swift`** (US5 · T097) — Pure helpers for wiki-link autocomplete: `partialRange(in:caret:)` detects an open `[[…` and returns the range to replace with completions, `enclosingLink(in:at:)` finds the `[[target]]` enclosing an offset (for click-to-navigate), `allLinks(in:)` collects all link spans in the buffer (for styling unresolved links).

- **`Info/InfoSidebarView.swift`** (US6 · FR-029..031) — **Toggleable right sidebar showing document stats (word/char/reading time/tasks) and an outline tree (clickable headings → scroll to line). Wired to `InfoModel`.**

- **`AI/AISettingsView.swift`** (US6) — **Modal/sheet for configuring BYOM endpoint: base URL, model, timeout. Includes a test-config button (calls `test_ai_config`). No key field; key is from Keychain.**

- **`AI/SummaryView.swift`** (US6) — **Streaming AI summary UI: shows full text as it arrives token-by-token. Cancel button, error display. Wired to `SummaryModel` via `AiSink` bridge (foreign trait).**

- **`Platform/KeychainStore.swift`** (US6 · research §C5, NFR-006) — **Tiny Keychain wrapper for the BYOM API key. Save/read/delete/hasKey. Device-local, no iCloud. Read on each request, never persisted or logged Rust-side.**

- **`Preview/PreviewWebView.swift`** (US4) — `NSViewRepresentable` wrapping an offline `WKWebView` that renders the core's comrak HTML with bundled, offline Mermaid + KaTeX and syntect-classed code. Privacy enforced in three layers: template CSP, nonPersistent data store, navigation delegate blocks remote origins. Receives scroll-to-line commands from `ScrollSync`.

- **`Preview/ScrollSync.swift`** (US4) — Bidirectional scroll sync hub (described above under models).

- **`Preview/PDFExport.swift`** (US4 · FR-026 / SC-010) — Off-screen PDF export via a dedicated `WKWebView` (off-screen, positioned far away so WebKit layouts and runs Mermaid's async JS) and `NSPrintOperation` with `@media print` rules. Uses `createPDF` intentionally avoided (Apple 700418/705138); `NSPrintOperation.runModal` gives multi-page fidelity.

- **`QuickOpen/QuickOpenView.swift`** — The ⌘P overlay palette (US3). Renders the filtered `SearchHit` list with arrows/Return/Escape handlers, wired to `QuickOpenModel`.

**Dependencies**: `EmendCore` SwiftPM package, AppKit (`NSTextView`, `NSOutlineView`, `WKWebView`, `NSPrintOperation`), SwiftUI, WebKit, Security (Keychain).

## Data Flow

### AI Summary (US6 · FR-032/036/036a)

```
User clicks "Request AI Summary" (or equivalent)
    ↓
Swift reads config from UserDefaults (base URL, model, timeout, max input)
    ↓
Swift reads API key from Keychain (or nil if not set)
    ↓
Creates SummaryModel + calls emend_core::summarize_document(handle, config, key, sink)
    ↓
FFI validates pre-network (blank key → AiNotConfigured; oversized doc → AiOversizedInput)
    ↓
Spawns tokio task; returns AiHandle for cancellation (NFR-002)
    ↓
**FFI** opens HTTP stream to OpenAI-compatible endpoint (reqwest, per-chunk timeout, no whole-request timeout)
    ↓
For each SSE chunk, calls emend_core::ai::SseParser to decode complete UTF-8 tokens
    ↓
Each token pushed to foreign AiSink.on_token() → SummaryModel accumulates text
    ↓
On [DONE] or stream close, calls AiSink.on_done(full) with the accumulated text
    ↓
On any error/cancellation, calls AiSink.on_error(err) (exactly one terminal)
    ↓
Swift displays accumulated text streaming in SummaryView
    ↓
User clicks Cancel → handle.cancel() → CancellationToken fires → AiCancelled terminal
```

**Why split (core pure, FFI reqwest)**:
- Core has no tokio/reqwest dependency (Constitution V), fully testable with `cargo test`.
- FFI handles only: HTTP client, tokio runtime, streaming orchestration, cancellation signaling.
- Core owns: SSE parsing (lenient, testable), request building (pure data), secret redaction (never leaked).

**Why per-chunk timeout**:
- A whole-request timeout fires mid-stream on slow models (research §B5).
- Per-chunk inactivity timeout detects genuinely stalled connections, allows streaming.

**Why Keychain, transient key**:
- Swift owns the secret's custody (device-local, no iCloud, no sync).
- Key is read immediately before each request and handed to Rust as a transient `String`.
- Never persisted or logged Rust-side; only set on Authorization header inside spawned task.

### Info Sidebar Stats & Outline (US6 · FR-029..031)

```
Editor signals EditorCoordinator.onDocEdit
    ↓
EditorCoordinator signals InfoModel.refresh()
    ↓
InfoModel calls document.stats() and document.outline() off-main (Task.detached)
    ↓
Core scans document source:
  * Word count (whitespace-delimited, containing letter or digit; no bare punctuation)
  * Char count (Unicode scalar values, i.e. `char`s)
  * Reading time (ceil(words / 200) minutes)
  * Task counts ([x]/[X] done, total)
  * Heading outline (ATX level 1..=6, 1-based line numbers, trimmed text)
    ↓
Swift updates @Published properties: words, chars, readingMinutes, tasksDone, tasksTotal, outlineItems
    ↓
InfoSidebarView renders stats grid + outline tree
    ↓
User clicks outline item → calls ScrollSync.scrollToLine(1basedLine) → editor jumps to that line
```

**Why off-main**: Stats/outline are cheap to recompute (O(n) scans), but avoid main-thread blocking on large docs.

**Why pure core functions**: No dependencies, unit-testable, no platform code.

### Hot Path: Per-Keystroke Edit

```
User types in NSTextView
    ↓
MarkdownTextView's NSTextStorageDelegate fires didProcessEditing
    ↓
EditorCoordinator extracts (range, oldLength, replacement) from NSTextStorage
    ↓
Calls Rust `push_edit(doc_handle, UTF16Range, replacement)` — **synchronous, off-main-thread**
    ↓
Rust updates shadow rope in `Document`
    ↓
Returns new rope length + diagnostic updates
    ↓
Swift updates NSTextStorage (resets insertion point, etc.)
    ↓
EditorCoordinator schedules re-attribution via Task @MainActor
    ↓
reattribute() calls `highlightSpans(viewport)` and applies SyntaxAttributing
    ↓
EditorCoordinator signals PreviewModel.scheduleRefresh() + InfoModel.refresh()
    ↓
AutosaveController.noteEdit() rearms debounce
```

**Why synchronous**: Every keystroke must be reflected in the buffer immediately. Pushing work to background would introduce latency, risking dropped keystrokes or race conditions with later edits.

**Why off-main-thread**: The Rust core's incremental rope operations are fast enough for per-keystroke throughput but may call `tree-sitter` highlighting — keeping the main thread responsive requires the call not to block.

### Preview Render with Embed Resolution (US4/US5)

```
EditorCoordinator signals PreviewModel.scheduleRefresh()
    ↓
PreviewModel coalesces rapid calls via debounce (150 ms idle)
    ↓
scheduleRefresh() spawns a Task.detached (userInitiated priority)
    ↓
Calls `document.renderPreviewHtmlResolving(workspace)` — pure comrak + syntect work
    ↓
Core renders via emend_core::parse::preview::render_preview_html_with_embeds
    ↓
For each embed `![[target]]`, calls workspace.resolve_embed_source(target)
    ↓
Workspace resolves target against index (deterministic tie-break), reads the note, returns source
    ↓
Core inlines the embedded note's HTML (comrak re-renders it) into the parent's tree
    ↓
Returns HTML with data-line scroll-sync anchors + syntect-classed code
    ↓
Swift updates @Published html + version
    ↓
PreviewWebView.updateNSView injects via window.__emendRender
    ↓
Template.html re-renders the #emend-content fragment
    ↓
Mermaid/KaTeX resolve (client-side, bundled, offline)
    ↓
bridge.js builds anchor table, optionally syncs scroll from editor
```

**Why debounced off-main-thread**: Preview rendering can be CPU-heavy on large docs with many code blocks. Debouncing coalesces rapid edits (typing bursts). Running off-main-thread keeps the UI responsive. Off-main-thread does **not** mean async — it's a `Task.detached`, which runs on a background GCD queue, not on the tokio runtime.

**Why core-owned HTML + CSS + embeds**: The core's `renderPreviewHtmlResolving` is the authoritative engine (comrak CommonMark). Embeds are resolved by the workspace index (US5) with locks dropped before rendering to avoid deadlock. The theme CSS is syntect-owned, vendored alongside the compiled syntax/theme dump. Both are stable per session and bundled into the app so the WKWebView never needs external resources (privacy, speed, reliability).

### Scroll Sync (US4 · FR-024, research §C3)

```
User scrolls the editor NSTextView
    ↓
EditorCoordinator observes scroll events, calls ScrollSync.editorScrolled()
    ↓
ScrollSync (unmuted) maps top visible character's 1-based line
    ↓
Calls webView.evaluateJavaScript("window.__emendScrollToLine(line)")
    ↓
bridge.js interpolates data-line anchors, smooth-scrolls the preview
    ↓
Page fires window.__emendOnScroll with its top visible line
    ↓
WKScriptMessageHandler calls ScrollSync.previewScrolled()
    ↓
ScrollSync (unmuted) scrolls NSTextView to that line + mutes briefly
    ↓
Editor's scroll event fires again, echoes back to preview, but mute window
    rejects it (160 ms guard)
```

**Why bidirectional + muted**: Both sides can scroll independently. Muting prevents echoes (user scrolls editor → preview scrolls → editor scrolls in response → preview scrolls, etc.). Short 160 ms mute window balances user interaction responsiveness with echo suppression.

### Quick Open Search (US3, NFR-002: Supersede)

```
User presses ⌘P
    ↓
QuickOpenModel.present() shows the overlay
    ↓
User types; each keystroke fires QuickOpenModel.runQuery()
    ↓
runQuery() increments generation, cancels any in-flight SearchHandle
    ↓
Calls `workspace.quick_open_query(trimmed, sink)` — **async, returns immediately**
    ↓
FFI (T074) cancels the previous SearchHandle in workspace.current_search (NFI-002)
    ↓
Spawns tokio worker running emend_core::search::quick_open over workspace.index
    ↓
Core ranks query via Index::query(), batches results, polls Cancel flag (T073)
    ↓
Each batch emitted to SearchSink.on_results(); Swift sink ignores if generation is stale
    ↓
On completion (or supersede if the next keystroke already fired), sink fires on_done()
    ↓
QuickOpenModel updates @Published results; QuickOpenView renders ranked list
    ↓
User presses Return → openSelected() opens the highlighted file in a tab and closes palette
    ↓
User presses Escape → dismiss() cancels the in-flight handle via SearchHandle.cancel()
```

**Why async + cancellable**: Search is I/O bound (file scanning) and ranks thousands of files. Blocking would freeze the app. Cancellation (supersede + explicit close) prevents resource waste.

**Why batching + generation guards**: Batching means a stale superseded worker stops emitting mid-stream within one batch (low latency, <32 results worth). A monotonic `generation` on the Swift side ignores late batches from superseded queries — belt-and-suspenders redundancy.

### Wiki-Link Navigation & Autocomplete (US5 · FR-019/020)

```
User types `[[` in the editor
    ↓
MarkdownTextView's completion delegate fires, WikiLinkAutocomplete.partialRange() detects
    ↓
Returns the UTF-16 range of the partial target typed after `[[`
    ↓
Completion controller replaces that range with selected completions
    ↓
EditorCoordinator calls workspace.wikilink_suggestions() for candidates
    ↓
Core's Index queries with the partial text, fuzzy-ranks note names
    ↓
Completions streamed via SearchSink (same as Quick Open, US3)
    ↓
User selects a completion; it's inserted (via native NSTextView completion)
    ↓
EditorCoordinator signals PreviewModel.scheduleRefresh()
    ↓
Preview renders with the link resolved (via core's index)

---

User ⌘-clicks a `[[target]]` in the editor
    ↓
MarkdownEditorView's mouse handler detects the click
    ↓
WikiLinkAutocomplete.enclosingLink() finds the `[[…]]` enclosing the click
    ↓
EditorCoordinator.workspace.resolve_wikilink() applies FR-019a's deterministic tie-break
    ↓
Opens the target note in a new tab via onOpenLink callback
```

**Why two systems for links**: Autocomplete (`wikilink_suggestions`) is for typing; ⌘-click navigation (`resolve_wikilink`) is for browsing. Both use the same index, but autocomplete streams candidates while navigation resolves a single target deterministically.

### Task Checkbox Toggle (US5 · FR-014)

```
User clicks a `[ ]`/`[x]` checkbox in the editor
    ↓
MarkdownTextView detects the click, TaskCheckbox.checkboxRange() finds the checkbox
    ↓
TaskCheckbox.toggleEdit() returns the edit (flip `[ ]` → `[x]` or vice versa)
    ↓
EditorCoordinator applies the edit via the normal Edit path (calls push_edit)
    ↓
Edit registers undo, triggers re-attribute, signals preview refresh
```

**Why Swift-side toggle**: Swift owns the text buffer. The toggle is a pure Edit, not a core FFI call. The core's `toggle_task` is for non-editor surfaces (info pane, preview context menu, etc.). Both apply the same transform; the editor version integrates with undo.

### Image Drag-Drop & Attachment Storage (US5 · FR-013/013a)

```
User drags image file(s) onto the editor
    ↓
MarkdownTextView's NSDraggingDestination handler fires
    ↓
ImageDrop.imageFileURLs() filters dropped URLs by extension (.png, .jpg, etc.)
    ↓
For each image, EditorCoordinator calls workspace.storeAttachment(data, ext)
    ↓
Core stores image beside the note with collision-safe naming (free_name)
    ↓
Returns a note-relative ref (e.g., "attachments/image_1.png")
    ↓
EditorCoordinator inserts `![](attachments/image_1.png)` as a standard Markdown image
    ↓
Edit goes through push_edit, registers undo, triggers re-attribute + preview
```

**Why note-relative**: Attachments live in the same directory as the note (or a subdirectory). Paths are relative so the note can be moved without breaking them.

### Embed Rendering in Preview (US5 · FR-021/021a)

```
Core sees `![[target]]` in the document source during preview render
    ↓
PreviewModel.renderPreviewHtmlResolving() calls workspace.resolve_embed_source(target)
    ↓
Workspace looks up target in the index, reads the source of the target note
    ↓
Returns the target note's Markdown source (or None if not found)
    ↓
Core's parse::preview::render_preview_html_with_embeds re-renders that source as HTML
    ↓
Inlines the HTML into the parent's syntax tree at the `![[target]]` position
    ↓
Final HTML has the embedded note's content expanded inline (recursive embeds stopped)
    ↓
Preview displays the inlined content
```

**Why embeds don't auto-link**: Embeds are pure content inclusion (transclusion), not navigation. The embedded note's own links and embeds render inline too (recursion stopped by a depth limit).

### Workspace & File Changes

```
External tool modifies file on disk (or git checkout, AI agent, etc.)
    ↓
Rust watcher (notify + debouncer) detects ChangeEvent
    ↓
Core classifies: move correlation (FR-006b), checks self-write registry (FR-006a)
    ↓
Surviving event → foreign DocObserver callback (Rust→Swift bridge)
    ↓
WorkspaceModel.handleFsChange() fires onExternalChange callback
    ↓
ConflictController checks if the changed path has an open tab + is recent self-write
    ↓
If external change (not our autosave), flag tab as conflicted
    ↓
Swift renders conflict banner; user resolves (reload or keep-mine)
```

**Why the split**: The core's deterministic classification is unit-tested; the Swift time-window guard is a pragmatic UI-level dedup.

### PDF Export (US4 · FR-026, research §C4)

```
User clicks "Export PDF" toolbar button
    ↓
MainWindow calls PDFExport.export(html:css:to:) async
    ↓
PDFExport spins up OffscreenPrintHost with a far-off-screen NSWindow
    ↓
1. Loads template.html + grants read access to preview/ dir
    ↓
2. Injects html + css via __emendRender (same as live preview)
    ↓
3. Waits for page readiness + Mermaid async layout (KaTeX is sync)
    ↓
4. Builds NSPrintInfo(savingTo:url) with @page rules from theme.css
    ↓
5. Calls NSPrintOperation(view:printInfo:).runModal (NOT run())
    ↓
   (runModal blocks until user confirms save; run() would deadlock WKWebView)
    ↓
6. PDF written to url; OffscreenPrintHost cleans up
    ↓
User sees PDF in Finder
```

**Why async + off-screen**: Export must not block the UI. Off-screen window (positioned far away, not hidden) ensures WebKit layouts and runs Mermaid's async JS rather than throttling an occluded view.

**Why NSPrintOperation.runModal, not createPDF**: `createPDF` emits a single tall page and ignores pagination (Apple forums 700418/705138). `runModal` respects `@media print` / `@page` rules and generates true multi-page PDFs with pagination logic.

### Formatting & List Commands

```
User presses ⌘B or Tab
    ↓
MarkdownTextView.performKeyEquivalent() / insertTab()
    ↓
SmartLists.indent() or FormattingCommands.bold() — pure, given (text, selection) → Edit
    ↓
apply(range:replacement:selection:) calls shouldChangeText → replaceCharacters → didChangeText
    ↓
NSTextStorageDelegate fires, EditorCoordinator extracts delta, calls push_edit()
```

Commands are **pure transforms** — they live in isolation without the editor's context. This enables **unit testing without a window** (Constitution VII).

### Sidebar Navigation & File Tree

```
WorkspaceModel loads persisted locations on launch
    ↓
Each location gets a DisplayRoot + per-path child-order / favorites / pins
    ↓
WorkspaceOutlineView renders NSOutlineView hierarchy (lazy children)
    ↓
User expands folder → outline calls workspace.collect_files(loc_id, folder_path)
    ↓
Rust returns FsNode list (canonicalized, deduplicated paths)
    ↓
View caches children, renders as outline items
    ↓
External FS change → watcher delivers ChangeEvent
    ↓
WorkspaceModel.pendingReloads accumulates affected folders
    ↓
Next run loop, consumePendingReloads() calls NSOutlineView.reloadItem() on changed parents
```

**Why NSOutlineView**: Native macOS feel; lazy children avoid upfront FS walk; targeted reloads on external FS changes are efficient.

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| **Swift/SwiftUI app** | UI rendering, event handling, model state | `EmendCore` (boundary), AppKit, WebKit, Security | Directly access files, Rust data structures |
| **Swift models** (@MainActor: WorkspaceModel, TabModel, ConflictController, QuickOpenModel, PreviewModel, ScrollSync, InfoModel) | State ownership, Rust handle lifecycle, pub/sub via @Published | `EmendCore`, AppKit, app views, Keychain | Other models (one-way data flow via callbacks) |
| **Swift views** | Rendering, event capture, formatted display | App models (read-only via @State/@Environment), AppKit, WebKit | Rust handles directly, file I/O |
| **Swift `EmendCore` wrapper** | Type adaptation, async stream wrapping | Generated UniFFI bindings | App state, UI models |
| **UniFFI boundary** | Foreign-trait scaffolding, error projection, panic containment | `emend-core`, async runtime | Anything beyond scaffolding |
| **Rust core (`emend-core`)** | All business logic: files, documents, parsing, preview, search, links/tasks/embeds/attachments, AI (pure), stats/outline, workspace, watching | Only standard library + workspace deps | FFI, async runtime, platform code |
| **FFI shim (`emend-ffi`)** | Transport, streaming, cancellation orchestration, reqwest client | Core logic, tokio runtime | Platform/OS code (that's Swift's job) |

**Dependency rules**:
- Higher layers depend on lower layers. Never vice versa.
- Core has **no knowledge** of FFI or Swift.
- FFI is a **thin projection only** — no business logic (except unavoidable streaming orchestration).
- Swift models own Rust handles via `@MainActor` locks; views are read-only presentations.
- **Keychain is Swift-only** — the key never crosses into Rust persistently; it is read transiently and passed as a `String` on each request.

## Dependency Rules

1. **Core → nothing but std + deps (no tokio, no reqwest)**. No FFI, no platform code.
2. **FFI → core + uniffi + tokio + reqwest**. Business logic lives in (1), scaffolding + transport here.
3. **Swift models → EmendCore wrapper only**. Never call generated FFI bindings directly; always go through the `EmendCore` module re-export.
4. **Swift views → models (one-way read-only) + AppKit/WebKit**. Never Rust handles directly.
5. **App → Swift models + views + AppKit + Security**. Never raw FFI.

## Key Interfaces & Contracts

### FFI Contract: Document (with US4 preview + US5 links/tasks/embeds/attachments + US6 stats/outline)

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §3/§6/§5/§4

| Export | Signature | Purpose |
|--------|-----------|---------|
| `open_document(path: String) -> Result<OpenDocHandle, FfiError>` | Create a document handle | Editor initialization |
| `close_document(handle: OpenDocHandle) -> Result<(), FfiError>` | Release a document | Window close |
| `push_edit(handle, range: U16Range, replacement: String) -> Result<(), FfiError>` | Apply keystroke delta | Per-keystroke sync path |
| `highlight_spans(handle, viewport: U16Range) -> Result<Vec<StyleSpan>, FfiError>` | Incremental highlight (tree-sitter, advisory) | Syntax coloring |
| **`handle.stats() -> Result<DocStats, FfiError>`** (US6 · T111) | **Word/char/reading-time/task counts** | **Info sidebar stats** |
| **`handle.outline() -> Result<Vec<OutlineItem>, FfiError>`** (US6 · T111) | **Headings with 1-based line numbers** | **Info sidebar outline tree** |
| **`handle.render_preview_html() -> Result<String, FfiError>`** (US4 · T084) | **Authoritative comrak HTML + scroll-sync anchors + syntect code coloring** | **Live preview pane + PDF export** |
| **`handle.render_preview_html_resolving(workspace) -> Result<String, FfiError>`** (US5 · T097) | **Same as above, but resolves embeds inline** | **Live preview with embedded notes** |
| **`handle.extract_links() -> Result<Vec<LinkRef>, FfiError>`** (US5 · T097) | **Scan document for `[[wiki links]]` and `![[embeds]]`** | **Editor link styling, autocomplete, click navigation** |
| **`handle.toggle_task(offset: u32) -> Result<String, FfiError>`** (US5 · T097) | **Flip checkbox on the line containing offset** | **Task checkbox clicking** |
| **`handle.store_attachment(data: Vec<u8>, ext: String) -> Result<String, FfiError>`** (US5 · T097) | **Store image/file beside note, return note-relative ref** | **Drag-drop image insertion** |
| `flush(handle) -> Result<(), FfiError>` | Write to disk | Autosave |

### FFI Contract: AI (US6)

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §7 (US6)

| Export | Signature | Purpose |
|--------|-----------|---------|
| **`summarize_document(handle, config, api_key, sink) -> AiHandle`** (US6 · T112) | **Stream summary token-by-token to foreign AiSink; validates pre-network (blank key, oversized doc); returns handle for cancel** | **BYOM AI summary in info sidebar** |
| **`test_ai_config(config, api_key) -> Result<(), FfiError>`** (US6 · FR-037) | **Probe endpoint with minimal non-streaming request; validates auth + reachability** | **Settings UI config test button** |
| **`AiHandle.cancel()`** (US6) | **Idempotent; trips CancellationToken; on_error(AiCancelled) terminal** | **User cancel button** |

**Foreign trait `AiSink`** (US6):
- `on_token(text: String)` — Each complete UTF-8 delta (buffered by core parser, never partial).
- `on_done(full: String)` — Success terminal: all text received.
- `on_error(err: FfiError)` — Failure/cancellation terminal: exactly one, no further tokens.

### FFI Contract: Preview Assets

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §6 (US4)

| Export | Signature | Purpose |
|--------|-----------|---------|
| **`preview_theme_css() -> String`** (US4 · T084) | **Core-owned syntect theme CSS for code blocks** | **Injected into preview template** |

The theme CSS is a compiled-in `&'static str` (vendored with the syntax/theme dump), so it's infallible, stateless, and session-constant.

### FFI Contract: Workspace & Link Resolution (with US5 link/task/embed/attachment additions)

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §1/§2/§5

| Export | Signature | Purpose |
|--------|-----------|---------|
| `new_workspace() -> WorkspaceHandle` | Create a workspace | Startup |
| `add_location(handle, path: String) -> Result<Location, FfiError>` | Add a user-chosen folder | Sidebar |
| `remove_location(handle, id: u64)` | Remove a location | Sidebar |
| `collect_files(handle, loc_id: u64, folder_path: String) -> Result<Vec<FsNode>, FfiError>` | List folder contents | Outline expansion |
| `create_note(handle, loc_id: u64, parent_path: String, name: String) -> Result<FsNode, FfiError>` | Create a new file | New file in sidebar |
| `rename(handle, path: String, new_name: String) -> Result<FsNode, FfiError>` | Rename a file/folder | Sidebar rename |
| `move_node(handle, path: String, new_parent: String) -> Result<FsNode, FfiError>` | Move a file/folder | Sidebar drag-drop |
| `delete_node(handle, path: String) -> Result<(), FfiError>` | Delete a file/folder | Sidebar delete |
| `reindex_all(handle, max_depth: u32) -> Result<u32, FfiError>` | Seed index from disk (US3) | Startup or after large imports |
| `query(handle, q: String) -> Result<Vec<SearchHit>, FfiError>` | Fuzzy search (blocking) | Wiki-link resolution |
| **`resolve_wikilink(handle, target: String, from_note: String) -> Result<Option<String>, FfiError>`** (US5 · T097) | **Deterministic link resolution with FR-019a tie-break** | **⌘-click link navigation** |
| **`wikilink_suggestions(handle, partial: String) -> Result<Vec<SearchHit>, FfiError>`** (US5 · T097) | **Autocomplete candidates for `[[partial`** | **Native NSTextView completion** |
| **`resolve_embed_source(handle, target: String, from_note: String) -> Result<Option<String>, FfiError>`** (US5 · T097) | **Read source of embedded note** | **Embed rendering in preview** |

### FFI Contract: Streaming Search (US3)

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §5

| Export | Signature | Purpose |
|--------|-----------|---------|
| `quick_open_query(handle, query: String, sink: Arc<dyn SearchSink>) -> Arc<SearchHandle>` | Stream ranked results, supersedable (FFI T074) | Quick Open palette (⌘P) |
| `SearchHandle.cancel()` | Cancel the in-flight query (trip both tokio::CancellationToken + core Cancel flag) | Palette close or supersede |
| `SearchSink.on_results(batch: Vec<SearchHit>)` (foreign trait) | Receive a batch of ranked results | Update UI result list |
| `SearchSink.on_done()` (foreign trait) | Terminal callback when query completes | Enable Return to open selection |

The core driver (`emend_core::search::quick_open`, T073) is pure and tokio-free; the FFI async shim (T074) bridges the tokio boundary, cancellation primitives, and the foreign-trait sink.

### Error Type

**Source**: `crates/emend-core/src/error.rs`

Variants carry context for UI rendering:
- `NotFound { path }` — File not found
- `PermissionDenied { path }` — Access denied
- `IoFailure { path, detail }` — Generic I/O
- `NameCollision { path }` — Already exists
- `NoteTooLarge { path, bytes, limit }` — Exceeds size cap
- `InvalidConfig { detail }` — Config error
- `AiNotConfigured` — AI is not set up (blank key)
- `AiOversizedInput { bytes, limit }` — Document exceeds max input (FR-036a)
- `AiTimeout` — AI request took too long
- `AiCancelled` — User cancelled
- `AiHttp { status, detail }` — HTTP error (status + redacted detail, never key, NFR-006)
- (more by later phases)

All variants use `String` fields only (UniFFI-compatible primitives).

### Edit Model (FFI + Rust)

**U16Range**: `U16Range { start: UInt32, len: UInt32 }` — a slice in UTF-16 code units (maps 1:1 to `NSRange`).

**StyleSpan**: `{ range: U16Range, class: StyleClass }` — a syntax highlighting span (tree-sitter advisory highlight). `StyleClass` is an enum: `heading(Int)`, `strong`, `emphasis`, `inlineCode`, `codeBlock`, `blockQuote`, `listMarker`, `link`, `syntaxMarker`, `highlight`.

**OpenDocHandle**: Opaque Rust handle representing an open `Document`. Returned by `open_document()`, passed to `push_edit()`, `highlightSpans()`, `stats()` (US6), `outline()` (US6), `renderPreviewHtml()`, `renderPreviewHtmlResolving()` (US5), `extractLinks()` (US5), `toggleTask()` (US5), `storeAttachment()` (US5), `flush()`, and `close_document()`.

**SearchHit**: Value struct returned by `quick_open_query` sinks. Contains `path: String` (filesystem path), `name: String` (basename), `score: UInt32` (ranking score, higher is better).

**LinkRef** (US5 · T097): Value struct returned by `extractLinks()` and `wikilink_suggestions()`. Contains `kind: LinkKind` (Link or Embed), `raw_target: String` (the target as typed), `range: U16Range` (the full `[[…]]` or `![[…]]` span in UTF-16 for editor styling/click-testing).

**DocStats** (US6 · T111): Value struct returned by `document.stats()`. Contains `words`, `chars`, `reading_minutes`, `tasks_done`, `tasks_total` (all U32). Recomputed on each edit, cheap O(n) scan.

**OutlineItem** (US6 · T111): Value struct returned by `document.outline()`. Contains `level: u8` (ATX 1..=6), `title: String` (trimmed text), `line: u32` (1-based source line for click→scroll).

**AiRequestConfig** (US6 · T112): Record struct for BYOM config. Contains `base_url`, `model`, `request_timeout_ms`, `max_input_bytes`. Passed by value; never stored Rust-side.

**AiHandle** (US6 · T112): Opaque handle for in-flight AI summary. Holds a `CancellationToken`; only exported method is `cancel()`.

## State Management

| State Type | Location | Pattern | Notes |
|------------|----------|---------|-------|
| **Document buffer (canonical)** | Swift `NSTextStorage` | Source of truth for display; edits flow user → NSTextStorage → Rust | Hot path; kept in sync via `push_edit` deltas |
| **Document (shadow)** | Rust `Document` (ropey rope) | Mirrors NSTextStorage; used for structural queries (highlight, outline, search) | Synced delta-for-delta from Swift |
| **File on disk** | Rust `fs` module | Atomic writes via tempfile + rename | Debounced autosave (Constitution III, FR-009a) |
| **Highlight cache (editor)** | Rust `parse::highlight` module (tree-sitter) | Incremental, invalidated by `push_edit`; advisory only | Computed on-demand by highlight queries |
| **Preview HTML** | Swift `PreviewModel.html` (@Published) | Rendered via core's comrak (+ embeds resolved US5); debounced off-main-thread; version bumped on each render for injection | Injected into WKWebView template via `__emendRender` |
| **Preview theme CSS** | Core-owned, vendored with syntect | Static, session-constant; fetched once on app startup | Injected into template alongside HTML |
| **Open-document list** | Swift `TabModel` (@Published tabs) | Registry of handles + text + autosave + UI state | Tracks which Rust `Document` handles are live |
| **Workspace (locations, favorites, pins, icons)** | Swift `WorkspaceModel` (@Published roots, etc.) | Owns `WorkspaceHandle` (Rust Workspace); app state persisted to UserDefaults | Master registry of user-added locations |
| **Search index** | Rust `Workspace.index` (behind `Arc<Mutex<>>`) | Fuzzy ranked entries maintained O(1)-ish on file ops | Shared: file ops lock+update, search queries lock briefly |
| **Quick Open results** | Swift `QuickOpenModel` (@Published results) | Streamed batches from `SearchSink`, guarded by monotonic `generation` | Superseded queries' batches are discarded by generation check |
| **File tree (sidebar)** | Swift `NSOutlineView` + `WorkspaceModel.roots` | Lazy children; `revision` bumps for top-level reloads, `fsRefreshTick` for targeted reloads | Reflects workspace + external FS changes |
| **Info sidebar stats/outline** (US6) | Swift `InfoModel` (@Published words, chars, readingMinutes, tasksDone, tasksTotal, outlineItems) | Recomputed on each edit via `document.stats()`/`outline()` off-main; cheap O(n) scans | Feeds display + click→scroll integration |
| **Scroll sync anchor table** | JS (bridge.js) built on page load | One-time construction from `data-line` attributes; both editor and preview reference it | Keyed on 1-based source line numbers |
| **Cancellation tokens** | Rust `handles` module (tokio-util) | Owned by Rust, cancelled by Swift | AI and search long-running tasks |
| **Conflict flags** | Swift `ConflictController` (@Published conflicts) | Set of conflicted tab IDs | Tracks docs that changed on disk + need user resolution |
| **Link/embed scans** (US5) | Rust `derived::extract_links()` called on-demand | Not cached; re-scanned per render cycle or when needed for UI | Lightweight: O(n) scan of document source text |
| **Attachment storage** (US5) | Rust `fs` module beside the note (collision-safe names) | Persisted alongside the note file | Relative refs allow notes to be moved |
| **AI config** (US6) | Swift `UserDefaults` (via `AIConfigStore`) | Base URL, model, timeout, max input | No key stored (only in Keychain) |
| **AI API key** (US6) | macOS Keychain (via `KeychainStore`) | Device-local, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` | Read transiently on each request, never Rust-persisted |
| **In-flight AI summary** (US6) | Swift `SummaryModel` (@Published fullText, done, error) | Holds handle, accumulated tokens, terminal state | Per-tab/document; stream via `AiSink` bridge |

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Error handling** | Structured `EmendError` enum, mapped at FFI | `crates/emend-core/src/error.rs`, `crates/emend-ffi/src/error.rs` |
| **Panic containment** | UniFFI `catch_unwind`, lint deny `panic`/`unwrap` | Lints in `Cargo.toml`, FFI scaffolding |
| **UTF-16 boundary safety** | `U16Range` type, checked conversions, surrogate-pair detection | `crates/emend-core/src/document.rs` |
| **Atomic durability** | Temp file + fsync + rename + fsync dir | `crates/emend-core/src/fs.rs` |
| **Async cancellation** | `tokio::sync::CancellationToken` + foreign-trait sinks | `crates/emend-ffi/src/handles.rs` |
| **Search cancellation (core layer)** | Arc-backed atomic bool flag (tokio-free) | `crates/emend-core/src/search.rs` |
| **Privacy** | No network unless AI configured + invoked; Keychain for API key; transient key to Rust, redacted in HTTP client; WKWebView CSP + offline template; per-chunk timeout (not whole-request) | `crates/emend-core/src/ai.rs`, `crates/emend-ffi/src/ai.rs`, Swift Keychain, `PreviewWebView` |
| **Secret hygiene** (US6) | `ApiKey` redacting newtype (Debug/Display → `***`); key read on demand, never persisted/logged Rust-side; per-chunk inactivity timeout for streaming | `crates/emend-core/src/ai.rs` (redaction), `crates/emend-ffi/src/ai.rs` (streaming), `KeychainStore.swift` (custody) |
| **Incremental syntax highlight (editor)** | tree-sitter (editor, advisory) vs. comrak (preview, authoritative) | `crates/emend-core/src/parse/highlight.rs` vs. `parse/preview.rs` |
| **Two-engine split (Constitution)** | tree-sitter and comrak deliberately never unified; different perf/correctness profiles | `crates/emend-core/src/parse.rs` module docs |
| **Preview authoritativeness** | comrak renders with CommonMark + GFM + extensions; editor highlight is advisory only | `crates/emend-core/src/parse/preview.rs` design doc |
| **Per-keystroke editing** | Swift owns buffer; Rust maintains shadow; deltas via `push_edit()` | `EditorCoordinator`, `EmendCore` |
| **Debounced autosave** | `DispatchQueue` serial queue, 1.5 s idle + 5 s hard cap | `AutosaveController` |
| **Debounced preview render** | Task-based debounce (~150 ms idle), coalesces rapid edits | `PreviewModel.scheduleRefresh()` |
| **Off-main info refresh** (US6) | `Task.detached` for stats/outline scans (cheap, but off-main for large docs) | `InfoModel.refresh()` |
| **Pure transforms (commands)** | `SmartLists` and `FormattingCommands` are pure functions, unit-testable without window | `app/Emend/Emend/Editor/` |
| **Self-write suppression** | Identity-keyed (mtime+len) in core + time-window in Swift `ConflictController` | `crates/emend-core/src/watcher`, `ConflictController` |
| **File watching** | notify + debouncer on OS threads; pure core classifier; foreign-trait bridge to Swift | `crates/emend-core/src/watcher`, `crates/emend-ffi/src/watcher` |
| **Incremental index** | Arena + path/name maps, O(1)-ish updates (no rescan on file ops) | `crates/emend-core/src/index`, `WorkspaceModel` tree updates |
| **Workspace persistence** | Locations + favorites/pins/icons persisted to UserDefaults | `WorkspaceModel`, `AppState` codable struct |
| **Quick Open superseding (NFR-002)** | Core batches + Cancel flag; FFI supersede via current_search; Swift generation guard | `crates/emend-core/src/search`, `crates/emend-ffi/src/search`, `QuickOpenModel` |
| **Bidirectional scroll sync** | Editor ↔ preview via `data-line` anchors + per-side mute window | `ScrollSync`, `bridge.js`, `crates/emend-core/src/parse/preview.rs` |
| **Offline preview rendering** | Core renders pure HTML (no I/O); Swift injects into offline template; CSP blocks remotes | `crates/emend-core/src/parse/preview.rs`, `PreviewWebView`, `template.html` |
| **PDF export multi-page** | Off-screen WKWebView + NSPrintOperation.runModal respects `@media print` rules | `PDFExport`, `theme.css` |
| **Wiki-link autocomplete** | Pure `WikiLinkAutocomplete` helpers detect `[[…` and return completable range; native NSTextView completion supplies candidates | `WikiLinkAutocomplete.swift`, `MarkdownTextView.swift` |
| **Wiki-link navigation** | ⌘-click detection → `enclosingLink()` → `resolve_wikilink()` with FR-019a tie-break → `onOpenLink` tab open | `EditorCoordinator`, `WikiLinkAutocomplete.swift` |
| **Task checkbox toggle** | Pure `TaskCheckbox` helpers detect `[ ]`/`[x]` and return toggle edit; Swift-side for undo integration | `TaskCheckbox.swift`, `MarkdownTextView.swift` |
| **Image drag-drop** | Pure `ImageDrop` helpers filter URLs + generate Markdown; core `storeAttachment` for collision-safe storage | `ImageDrop.swift`, `crates/emend-core/src/fs.rs` |
| **Embed rendering** | Core's `render_preview_html_with_embeds` resolves targets via workspace callback, re-renders, inlines HTML | `crates/emend-core/src/parse/preview.rs`, `PreviewModel.swift` |
| **Deterministic link resolution** | Core's `derived::resolve_wikilink` applies FR-019a tie-break: same dir → shallowest → lexicographic | `crates/emend-core/src/derived.rs`, `tests/links.rs` |
| **AI streaming** (US6) | Core SSE parser (lenient, UTF-8-safe) + FFI per-chunk timeout + foreign-trait callbacks for tokens + terminal | `crates/emend-core/src/ai.rs`, `crates/emend-ffi/src/ai.rs` |

## Build & Deployment

- **Rust workspace** (`cargo build --release`) produces `libemend_ffi.a` (static lib for iOS-style XCFramework).
- **`just xcframework`** runs `uniffi-bindgen-swift`, links the static lib into an XCFramework, and generates Swift bindings (all git-ignored).
- **`just xcodeproj`** regenerates the Xcode app project from `project.yml` (XcodeGen, also git-ignored).
- **Final `.app`** built by Xcode 16.2, signed with automatic signing, deployed to `~/Applications/Emend.app` (or ad-hoc distribution).

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*
