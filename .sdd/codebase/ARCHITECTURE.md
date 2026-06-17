# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (incremental: US4 faithful preview + PDF export merged)

## Architecture Overview

Emend is a **hybrid Rust+Swift native macOS Markdown editor** with a cleanly separated boundary:

- **Rust core** (`crates/emend-core`) houses ALL business logic: file I/O, document parsing, preview rendering (comrak + syntect), file watching, indexing, search, and AI client integration. The core is **toolchain-free** — it has no FFI dependency and is fully testable with `cargo test` in isolation.
- **UniFFI shim** (`crates/emend-ffi`) provides a thin boundary layer that exports the core's capabilities to Swift and manages async infrastructure (tokio runtime, cancellation tokens).
- **Swift/SwiftUI app** (`app/Emend`) wraps the core in a native macOS UI with a three-pane layout: sidebar (workspace/favorites), tabbed editor (with per-document state), live preview pane (US4), and a ⌘P Quick Open palette (US3).

The boundary is **synchronous on the hot path** (per-keystroke edits) and **asynchronous only for AI and search** (with cancellable Rust-owned handles). Preview rendering is debounced off the keystroke path and runs off-main-thread. This design allows the core to stay independent and testable while the UI safely dispatches background work.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Layered (horizontal)** | Presentation (Swift/SwiftUI) → API boundary (UniFFI) → Business logic (Rust core) |
| **Modular monolith** | Single deployable macOS app; no microservices or network splits |
| **Rust corelib + FFI shim** | Heavy separation of concerns: business logic in pure Rust, FFI concerns isolated in a thin wrapper |
| **Synchronous hot path, async background** | Per-keystroke edits cross the boundary synchronously; AI and search use async Rust-owned handles; preview renders debounced off-main-thread |
| **UTF-16 boundary contract** | All text ranges crossing the FFI boundary are UTF-16 code units, mapping 1:1 to `NSRange` |
| **Swift owns text buffer** | Canonical text storage lives in NSTextStorage; Rust maintains a shadow ropey rope for structural queries |
| **Core owns preview HTML + theme CSS** | Core generates both the markdown→HTML via comrak and the syntect code-highlight CSS; Swift embeds these offline into a bundled WKWebView template |
| **Clear model/view separation (Swift UI)** | `@MainActor` state models (`WorkspaceModel`, `TabModel`, `ConflictController`, `QuickOpenModel`, `PreviewModel`) own Rust handles and drive views; views are pure presentations of model state |

## Core Components

### 1. Rust Core (`crates/emend-core`)

**Purpose**: The engine — file I/O, document state, parsing, preview rendering, search, watching, AI streaming.

**Location**: `crates/emend-core/src/`

**Modules**:

- **`error.rs`** — Structured `EmendError` type (single source of truth for FFI contract). Variants carry context fields (paths, limits, byte counts) for UI rendering. Exhaustive enum (not `#[non_exhaustive]`) so the FFI projection can be a closed, compiler-checked mirror.
- **`fs.rs`** — Atomic+durable writes and tolerant reads. Write path: temp file in same directory → fsync → atomic rename → fsync directory (guarantees no torn writes). Read path: strips UTF-8 BOM, preserves CRLF, decodes invalid UTF-8 lossily. On macOS, `File::sync_all()` already calls `fcntl(F_FULLFSYNC)` for true durability.
- **`document.rs`** — The open-document model: a shadow ropey rope + UTF-16/char/line indices. Backs all per-keystroke edits, structural queries (highlight, outline), and search. Converts at exactly one place on every boundary call, never panicking — all conversions are checked and mapped to `EmendError`.
- **`workspace.rs`** — File-based workspace model (US2): locations (user-chosen root folders), lazy directory listing, collision-safe file operations, in-memory maps for favorites/pins/icons/child-order. Uses canonicalization + bounded traversal for path identity (NFR-007) and `free_name` for collision-safe naming (FR-004a). Pure `std` + `tempfile`; no async.
- **`index.rs`** — Incremental in-memory search index (US2): arena-based entries, path/name maps, fuzzy ranking via `nucleo-matcher`. Maintained O(1)-ish on file operations (create/rename/move/delete) via `Index::insert/remove/rename`, never full rescan (FR-017a). Backs Quick Open + wiki-link resolution.
- **`search.rs`** (US3) — Pure, tokio-free streaming search driver (T073). Owns the **emission policy** for Quick Open: batches ranked results from `Index::query()` and re-checks a `Cancel` flag between batches for fast supersede (NFR-002). Holds `pub struct Cancel` (Arc-backed atomic bool) so multiple clones share the cancellation state. The core decision logic (rank, batch, stop-on-supersede) lives here for unit testability without an async runtime (`tests/search_supersede.rs`). No `uniffi` or `tokio` dependencies.
- **`watcher.rs`** — Live file watching (US2): thin `notify` + `notify-debouncer-full` wrapper over a pure, deterministically-tested classification core. Includes move correlation (FR-006b), self-write suppression via identity-keyed registry (FR-006a), and conflict truth table (FR-006c). Runs on OS threads, posts to `std::sync::mpsc`; no async runtime.
- **`parse.rs`** (US4 · T084) — Markdown parsing: deliberately **two separate engines** (Constitution): incremental tree-sitter (editor highlight, advisory) vs. comrak (preview HTML, authoritative). Held apart on purpose, never unified.
  - **`parse/highlight.rs`** — Incremental tree-sitter highlighting for the editor (advisory, fast, on-keystroke path).
  - **`parse/preview.rs`** (US4 · T084) — **Authoritative comrak preview engine**: CommonMark + GFM + native `[[wikilink]]` + `==highlight==` extensions, with `data-line` scroll-sync anchors and syntect-coloured code blocks. Pure transform (no I/O, no async, no network). Defers embeds to US5.
  - **`parse/code_highlight.rs`** (US4 · T084) — syntect-based HTML code colouring for preview fenced blocks; vendored binary syntax/theme dump loaded once per session.
- **`ai.rs`** — Placeholder module (to be added by `/sdd:implement`).

**Dependencies**: `thiserror`, `ropey`, `tempfile`, `tree-sitter`, `tree-sitter-md`, `comrak`, `syntect`, `nucleo`, `nucleo-matcher`, `notify`, `notify-debouncer-full`, `reqwest`, `serde`, `tokio` (only in FFI shim), `tokio-util` (only in FFI shim).

**Dependents**: `crates/emend-ffi`, tests, benchmarks.

**Constraints (Constitution V)**:
- **NO FFI dependency** — core never imports `uniffi`. This keeps the core standalone and testable.
- **NO panics** — clippy lints deny `unwrap_used`, `expect_used`, `panic`.
- **No async primitives** — tokio only enters in the FFI shim (async infrastructure lives outside the core).
- **Two markdown engines, not one** — tree-sitter (editor) and comrak (preview) are never unified; their performance and correctness profiles differ (Constitution).

### 2. FFI Shim (`crates/emend-ffi`)

**Purpose**: Thin projection of the core to Swift. Manages async infrastructure, panic containment, and error mapping.

**Location**: `crates/emend-ffi/src/`

**Modules**:

- **`lib.rs`** — FFI entry points (e.g., `read_file_at`, `core_abi_version`, `preview_theme_css`). Each `#[uniffi::export]` function wraps a core function, handling errors. UniFFI's scaffolding automatically wraps calls in `catch_unwind`, so panics cannot unwind into Swift.
- **`error.rs`** — `#[derive(uniffi::Error)]` projection of `EmendError`. Keeps the same `Display` wording so Swift sees the same error message. Exhaustive `From` impls ensure new core error variants force compilation errors here.
- **`panic.rs`** — Custom panic hook (if needed for debugging; not yet implemented).
- **`document.rs`** — FFI projection of document operations: `open_document`, `close_document`, `push_edit`, `highlight_spans`, `render_preview_html`, `flush`. Wraps the core's `Document` in an `OpenDocHandle` (`Arc<Mutex<Document>>`).
  - **`render_preview_html()`** (US4 · T084) — Calls the core's comrak preview engine, returning HTML suitable for injection into the WKWebView template.
- **`workspace.rs`** — FFI projection of workspace + index: `WorkspaceHandle` wrapping both `Workspace` + `SharedIndex` (Index behind `Arc<Mutex<>>`) co-located under one `Mutex<Inner>` (to maintain incremental index updates in lock-step, FR-017a). Exports `Location`, `FsNode`, `NodeKind` value types and methods like `create_note`, `rename`, `move_node`, `query`, `resolve_name`, `quick_open_query` (T074), and `reindex_all` (T078).
- **`search.rs`** (US3 · T074) — FFI projection of streaming Quick Open (§5 of contract). Drives the pure, tokio-free core search driver (`emend_core::search::quick_open`) on the boundary's shared tokio runtime and forwards ranked batches to foreign `SearchSink`. Holds `pub struct SearchHandle` (UniFFI Object, `Arc<Self>`), which bridges cancellation: a `tokio_util::CancellationToken` (parity with `CancellationHandle`) and an `emend_core::search::Cancel` flag. A single `quick_open_query` cancels any previous `SearchHandle` in `WorkspaceHandle.current_search` (supersede, NFR-002), then spawns the new worker. The core driver is fast (<100 ms p95, SC-004); lock the index briefly, not the whole workspace.
- **`watcher.rs`** — FFI projection of the live watcher: `WatchHandle` wrapping `FsWatcher`, with `ChangeEvent` and `ConflictState`/`ConflictChoice` enums. Bridges via `ObserverBridge` to the Swift `DocObserver` foreign trait.
- **`handles.rs`** — Rust-owned infrastructure: the tokio runtime (lives here, not core), `CancellationToken` for async tasks, `SearchHit` value type, and foreign-trait `DocObserver`/`SearchSink`/`AiSink` callbacks for streaming results.

**Dependencies**: `emend-core`, `uniffi`, `thiserror`, `tokio`, `tokio-util`, `tempfile`.

**Dependents**: Swift package (`swift/EmendCore`).

**Constraint**: Keep FFI free of business logic. It is ONLY a projection and scaffolding layer.

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

**Purpose**: Swift `@MainActor` state models that own the Rust handles and coordinate between workspace/tabs/editor/conflicts/search/preview.

**Location**: `app/Emend/Emend/`

**Key models**:

- **`WorkspaceModel`** (`Sidebar/WorkspaceModel.swift`) — `@MainActor ObservableObject` owning the `WorkspaceHandle` (Rust workspace + index). Manages the security-scoped folder bookmark lifecycle, per-location file watchers, the sidebar's root nodes (locations + Favorites group). Publishes `roots` (the outline tree), `revision` (bumped when locations change), and `fsRefreshTick` (for targeted outline reloads). Persists app state (favorites/pins/icons) to UserDefaults.

- **`TabModel`** (`Tabs/TabModel.swift`) — `@MainActor ObservableObject` owning the list of open documents (tabs). Each `Tab` holds its own `OpenDocHandle`, initial text, `AutosaveController`, and a reload token. Manages tab creation/closing, active selection, status messages. Coordinates with `ConflictController` on external changes. Publishes `activeID` so view transitions can swap the active editor/preview.

- **`ConflictController`** (`Editor/ConflictController.swift`) — `@MainActor ObservableObject` detecting when an open document changes on disk underneath the editor (FR-006c). Listens to workspace's `onExternalChange` callback and tracks self-writes from autosave. Flags conflicted tabs; users choose reload-or-keep-mine. Maintains both core-side suppression (identity-keyed) and Swift-side time-window guards to avoid false positives.

- **`QuickOpenModel`** (`QuickOpen/QuickOpenModel.swift`) — `@MainActor ObservableObject` driving the ⌘P Quick Open palette (US3, FR-017/FR-018, NFR-002). Bridges the core's streaming, supersedable `quick_open_query` to SwiftUI: each keystroke starts a fresh query that supersedes the prior (the core cancels the previous in-flight `SearchHandle`; a monotonic `generation` additionally guards against a late batch from a superseded query landing after the next one began). Ranked `SearchHit`s stream in via a `SearchSink` bridge; arrow keys move the selection, Return opens the file.

- **`PreviewModel`** (US4 · `Preview/PreviewModel.swift`) — `@MainActor ObservableObject` driving the live Markdown preview pane (FR-022/FR-025, research §B1). Debounced ~150 ms off the editor's `onDocEdit` signal; renders via the core's `renderPreviewHtml` (comrak + syntect, authoritative) off the main thread (NFR-001). Holds the syntect theme CSS (core-owned, fetched once). Publishes `html` (rendered body fragment) and `version` (bumped on each successful render even if HTML unchanged). When the preview pane becomes visible, schedules an immediate refresh; while hidden, renders are skipped to avoid wasted work.

- **`ScrollSync`** (US4 · `Preview/ScrollSync.swift`) — `@MainActor ObservableObject` managing bidirectional editor ↔ preview scroll sync (FR-024, research §C3). Both sides keyed on 1-based source line numbers: comrak annotates blocks with `data-line` (via core's scroll-sync anchors); bridge.js builds an anchor table. Editor→preview maps the top visible character's line and calls `__emendScrollToLine`; preview→editor receives the top line from the page. Per-side mute window (160 ms) guards the feedback loop to avoid echoing.

### 5. macOS App — View Layer

**Purpose**: SwiftUI views and editor mechanics.

**Location**: `app/Emend/Emend/`

**Major components**:

- **`Shell/MainWindow.swift`** — Single-window three-pane layout with a preview pane (US4): sidebar (workspace outline) | editor pane (tabbed) | preview pane (toggled via "Toggle Preview" toolbar button). Wires up `WorkspaceModel`, `TabModel`, `ConflictController`, `QuickOpenModel`, `PreviewModel`, and `ScrollSync`. Includes Export PDF toolbar action (US4). Hidden ⌘P button registers the Quick Open shortcut window-wide.

- **`Sidebar/WorkspaceOutlineView.swift`** — `NSViewRepresentable` wrapping `NSOutlineView` over the workspace's file tree. Lazy children loading, targeted reloadItem on external FS changes. Context menu + drag-drop.

- **`Sidebar/OutlineDragDrop.swift`** — Drag-drop handlers for moving/renaming files in the outline.

- **`Tabs/TabBarView.swift`** — Tab bar rendering the open documents, active selection, close buttons.

- **`Editor/MarkdownEditorView.swift`** — `NSViewRepresentable` wrapping a TextKit 2 `MarkdownTextView` + coordinate per-keystroke edits. Registers with `ScrollSync` for editor→preview scroll bridging.

- **`Editor/MarkdownTextView.swift`** — `NSTextView` subclass that hooks list/formatting keys (Return, Tab, ⌘B/I/K/⇧T) to pure transforms.

- **`Editor/EditorCoordinator.swift`** — `NSTextStorageDelegate` that extracts UTF-16 deltas, calls `pushEdit()` synchronously, then signals `PreviewModel.scheduleRefresh()` to debounce the preview render.

- **`Editor/SyntaxAttributing.swift`** — Pure function mapping core `StyleSpan`s to AppKit display attributes (bold/italic/headings/code/quote inline, markers dimmed).

- **`Editor/SmartLists.swift`** — Pure transforms: `newline()` (continue/terminate list), `renumber()` (sequential ordered lists), `indent()`/`outdent()` (shift nesting).

- **`Editor/FormattingCommands.swift`** — Pure transforms: `bold()`, `italic()`, `link()`, `task()` (insert markers around selection).

- **`Editor/AutosaveController.swift`** — Debounced (1.5 s idle + 5 s hard cap) atomic flush on private serial queue. Errors surface via callback. Also registers self-writes with the core's conflict suppression.

- **`Preview/PreviewWebView.swift`** (US4) — `NSViewRepresentable` wrapping an offline `WKWebView` that renders the core's comrak HTML with bundled, offline Mermaid + KaTeX and syntect-classed code. Privacy enforced in three layers: template CSP, nonPersistent data store, navigation delegate blocks remote origins. Receives scroll-to-line commands from `ScrollSync`.

- **`Preview/ScrollSync.swift`** (US4) — Bidirectional scroll sync hub (described above under models).

- **`Preview/PDFExport.swift`** (US4 · FR-026 / SC-010) — Off-screen PDF export via a dedicated `WKWebView` (off-screen, positioned far away so WebKit layouts and runs Mermaid's async JS) and `NSPrintOperation` with `@media print` rules. Uses `createPDF` intentionally avoided (Apple 700418/705138); `NSPrintOperation.runModal` gives multi-page fidelity.

- **`QuickOpen/QuickOpenView.swift`** — The ⌘P overlay palette (US3). Renders the filtered `SearchHit` list with arrows/Return/Escape handlers, wired to `QuickOpenModel`.

**Dependencies**: `EmendCore` SwiftPM package, AppKit (`NSTextView`, `NSOutlineView`, `WKWebView`, `NSPrintOperation`), SwiftUI, WebKit.

## Data Flow

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
EditorCoordinator signals PreviewModel.scheduleRefresh()
    ↓
AutosaveController.noteEdit() rearms debounce
```

**Why synchronous**: Every keystroke must be reflected in the buffer immediately. Pushing work to background would introduce latency, risking dropped keystrokes or race conditions with later edits.

**Why off-main-thread**: The Rust core's incremental rope operations are fast enough for per-keystroke throughput but may call `tree-sitter` highlighting — keeping the main thread responsive requires the call not to block.

### Preview Render (US4 · NFR-001)

```
EditorCoordinator signals PreviewModel.scheduleRefresh()
    ↓
PreviewModel coalesces rapid calls via debounce (150 ms idle)
    ↓
scheduleRefresh() spawns a Task.detached (userInitiated priority)
    ↓
Calls `document.renderPreviewHtml()` — pure comrak + syntect work
    ↓
Core renders via emend_core::parse::preview::render_preview_html
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

**Why core-owned HTML + CSS**: The core's `renderPreviewHtml` is the authoritative engine (comrak CommonMark). The theme CSS is syntect-owned, vendored alongside the compiled syntax/theme dump. Both are stable per session and bundled into the app so the WKWebView never needs external resources (privacy, speed, reliability).

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

### AI Path: Long-Running

```
User selects "Request AI" from menu
    ↓
Swift calls `ai_request(config, prompt, stream_sink)` — **async, returns immediately**
    ↓
Rust spawns tokio task, returns CancellationHandle
    ↓
tokio task opens HTTP stream to OpenAI-compatible endpoint
    ↓
For each SSE event, calls foreign-trait `stream_sink.on_chunk(text)`
    ↓
Swift receives chunks via AsyncStream adapter, updates UI
    ↓
If user cancels, Swift calls `handle.cancel()`
    ↓
Rust `CancellationToken` stops the tokio task
```

**Why async + cancellable**: AI requests are I/O bound and may take seconds. Blocking the Rust thread would block the whole app. Cancellation prevents resource waste.

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
| **Swift/SwiftUI app** | UI rendering, event handling, model state | `EmendCore` (boundary), AppKit, WebKit | Directly access files, Rust data structures |
| **Swift models** (@MainActor: WorkspaceModel, TabModel, ConflictController, QuickOpenModel, PreviewModel, ScrollSync) | State ownership, Rust handle lifecycle, pub/sub via @Published | `EmendCore`, AppKit, app views | Other models (one-way data flow via callbacks) |
| **Swift views** | Rendering, event capture, formatted display | App models (read-only via @State/@Environment), AppKit, WebKit | Rust handles directly, file I/O |
| **Swift `EmendCore` wrapper** | Type adaptation, async stream wrapping | Generated UniFFI bindings | App state, UI models |
| **UniFFI boundary** | Foreign-trait scaffolding, error projection, panic containment | `emend-core`, async runtime | Anything beyond scaffolding |
| **Rust core (`emend-core`)** | All business logic: files, documents, parsing, preview, search, AI, workspace, watching | Only standard library + workspace deps | FFI, async runtime, platform code |

**Dependency rules**:
- Higher layers depend on lower layers. Never vice versa.
- Core has **no knowledge** of FFI or Swift.
- FFI is a **thin projection only** — no business logic.
- Swift models own Rust handles via `@MainActor` locks; views are read-only presentations.

## Dependency Rules

1. **Core → nothing but std + deps**. No FFI, no platform code.
2. **FFI → core + uniffi + tokio**. Business logic lives in (1), scaffolding here.
3. **Swift models → EmendCore wrapper only**. Never call generated FFI bindings directly; always go through the `EmendCore` module re-export.
4. **Swift views → models (one-way read-only) + AppKit/WebKit**. Never Rust handles directly.
5. **App → Swift models + views + AppKit**. Never raw FFI.

## Key Interfaces & Contracts

### FFI Contract: Document (with US4 preview additions)

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §3/§6

| Export | Signature | Purpose |
|--------|-----------|---------|
| `open_document(path: String) -> Result<OpenDocHandle, FfiError>` | Create a document handle | Editor initialization |
| `close_document(handle: OpenDocHandle) -> Result<(), FfiError>` | Release a document | Window close |
| `push_edit(handle, range: U16Range, replacement: String) -> Result<(), FfiError>` | Apply keystroke delta | Per-keystroke sync path |
| `highlight_spans(handle, viewport: U16Range) -> Result<Vec<StyleSpan>, FfiError>` | Incremental highlight (tree-sitter, advisory) | Syntax coloring |
| **`handle.render_preview_html() -> Result<String, FfiError>`** (US4 · T084) | **Authoritative comrak HTML + scroll-sync anchors + syntect code coloring** | **Live preview pane + PDF export** |
| `flush(handle) -> Result<(), FfiError>` | Write to disk | Autosave |

### FFI Contract: Preview Assets

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §6 (US4)

| Export | Signature | Purpose |
|--------|-----------|---------|
| **`preview_theme_css() -> String`** (US4 · T084) | **Core-owned syntect theme CSS for code blocks** | **Injected into preview template** |

The theme CSS is a compiled-in `&'static str` (vendored with the syntax/theme dump), so it's infallible, stateless, and session-constant.

### FFI Contract: Workspace & File Operations

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §1/§2

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
| `resolve_name(handle, name: String) -> Result<Vec<String>, FfiError>` | Wiki-link resolution | Link completion |

### FFI Contract: Streaming Search (US3)

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §5

| Export | Signature | Purpose |
|--------|-----------|---------|
| `quick_open_query(handle, query: String, sink: Arc<dyn SearchSink>) -> Arc<SearchHandle>` | Stream ranked results, supersedable (FFI T074) | Quick Open palette (⌘P) |
| `SearchHandle.cancel()` | Cancel the in-flight query (trip both tokio::CancellationToken + core Cancel flag) | Palette close or supersede |
| `SearchSink.on_results(batch: Vec<SearchHit>)` (foreign trait) | Receive a batch of ranked results | Update UI result list |
| `SearchSink.on_done()` (foreign trait) | Terminal callback when query completes | Enable Return to open selection |

The core driver (`emend_core::search::quick_open`, T073) is pure and tokio-free; the FFI async shim (T074) bridges the tokio boundary, cancellation primitives, and the foreign-trait sink.

### FFI Contract: File Watching & Conflict Handling

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md` §1/§3

| Export | Signature | Purpose |
|--------|-----------|---------|
| `start_watching(handle, observer: DocObserver) -> WatchHandle` | Start watching locations | Sidebar |
| `record_self_write(path: String, mtime: i64, len: u64)` | Suppress self-writes | Autosave |
| `conflict_state(handle, path: String) -> ConflictState` | Check doc vs disk | Open document |
| `apply_conflict_choice(handle, path: String, choice: ConflictChoice) -> Result<(), FfiError>` | Resolve user's choice | Conflict resolution |

### Error Type

**Source**: `crates/emend-core/src/error.rs`

Variants carry context for UI rendering:
- `NotFound { path }` — File not found
- `PermissionDenied { path }` — Access denied
- `IoFailure { path, detail }` — Generic I/O
- `NameCollision { path }` — Already exists
- `NoteTooLarge { path, bytes, limit }` — Exceeds size cap
- `InvalidConfig { detail }` — Config error
- `AiNotConfigured` — AI is not set up
- `AiTimeout` — AI request took too long
- `AiCancelled` — User cancelled
- (more by later phases)

All variants use `String` fields only (UniFFI-compatible primitives).

### Edit Model (FFI + Rust)

**U16Range**: `U16Range { start: UInt32, len: UInt32 }` — a slice in UTF-16 code units (maps 1:1 to `NSRange`).

**StyleSpan**: `{ range: U16Range, class: StyleClass }` — a syntax highlighting span (tree-sitter advisory highlight). `StyleClass` is an enum: `heading(Int)`, `strong`, `emphasis`, `inlineCode`, `codeBlock`, `blockQuote`, `listMarker`, `link`, `syntaxMarker`, `highlight`.

**OpenDocHandle**: Opaque Rust handle representing an open `Document`. Returned by `open_document()`, passed to `push_edit()`, `highlightSpans()`, `renderPreviewHtml()`, `flush()`, and `close_document()`.

**SearchHit**: Value struct returned by `quick_open_query` sinks. Contains `path: String` (filesystem path), `name: String` (basename), `score: UInt32` (ranking score, higher is better).

## State Management

| State Type | Location | Pattern | Notes |
|------------|----------|---------|-------|
| **Document buffer (canonical)** | Swift `NSTextStorage` | Source of truth for display; edits flow user → NSTextStorage → Rust | Hot path; kept in sync via `push_edit` deltas |
| **Document (shadow)** | Rust `Document` (ropey rope) | Mirrors NSTextStorage; used for structural queries (highlight, outline, search) | Synced delta-for-delta from Swift |
| **File on disk** | Rust `fs` module | Atomic writes via tempfile + rename | Debounced autosave (Constitution III, FR-009a) |
| **Highlight cache (editor)** | Rust `parse::highlight` module (tree-sitter) | Incremental, invalidated by `push_edit`; advisory only | Computed on-demand by highlight queries |
| **Preview HTML** | Swift `PreviewModel.html` (@Published) | Rendered via core's comrak; debounced off-main-thread; version bumped on each render for injection | Injected into WKWebView template via `__emendRender` |
| **Preview theme CSS** | Core-owned, vendored with syntect | Static, session-constant; fetched once on app startup | Injected into template alongside HTML |
| **Open-document list** | Swift `TabModel` (@Published tabs) | Registry of handles + text + autosave + UI state | Tracks which Rust `Document` handles are live |
| **Workspace (locations, favorites, pins, icons)** | Swift `WorkspaceModel` (@Published roots, etc.) | Owns `WorkspaceHandle` (Rust Workspace); app state persisted to UserDefaults | Master registry of user-added locations |
| **Search index** | Rust `Workspace.index` (behind `Arc<Mutex<>>`) | Fuzzy ranked entries maintained O(1)-ish on file ops | Shared: file ops lock+update, search queries lock briefly |
| **Quick Open results** | Swift `QuickOpenModel` (@Published results) | Streamed batches from `SearchSink`, guarded by monotonic `generation` | Superseded queries' batches are discarded by generation check |
| **File tree (sidebar)** | Swift `NSOutlineView` + `WorkspaceModel.roots` | Lazy children; `revision` bumps for top-level reloads, `fsRefreshTick` for targeted reloads | Reflects workspace + external FS changes |
| **Scroll sync anchor table** | JS (bridge.js) built on page load | One-time construction from `data-line` attributes; both editor and preview reference it | Keyed on 1-based source line numbers |
| **Cancellation tokens** | Rust `handles` module (tokio-util) | Owned by Rust, cancelled by Swift | AI and search long-running tasks |
| **Conflict flags** | Swift `ConflictController` (@Published conflicts) | Set of conflicted tab IDs | Tracks docs that changed on disk + need user resolution |

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Error handling** | Structured `EmendError` enum, mapped at FFI | `crates/emend-core/src/error.rs`, `crates/emend-ffi/src/error.rs` |
| **Panic containment** | UniFFI `catch_unwind`, lint deny `panic`/`unwrap` | Lints in `Cargo.toml`, FFI scaffolding |
| **UTF-16 boundary safety** | `U16Range` type, checked conversions, surrogate-pair detection | `crates/emend-core/src/document.rs` |
| **Atomic durability** | Temp file + fsync + rename + fsync dir | `crates/emend-core/src/fs.rs` |
| **Async cancellation** | `tokio::sync::CancellationToken` + foreign-trait sinks | `crates/emend-ffi/src/handles.rs` |
| **Search cancellation (core layer)** | Arc-backed atomic bool flag (tokio-free) | `crates/emend-core/src/search.rs` |
| **Privacy** | No network unless AI configured + invoked; Keychain for API key; transient to Rust, redacted in HTTP client; WKWebView CSP + offline template | `crates/emend-core`, Swift app bindings, `PreviewWebView` |
| **Incremental syntax highlight (editor)** | tree-sitter (editor, advisory) vs. comrak (preview, authoritative) | `crates/emend-core/src/parse/highlight.rs` vs. `parse/preview.rs` |
| **Two-engine split (Constitution)** | tree-sitter and comrak deliberately never unified; different perf/correctness profiles | `crates/emend-core/src/parse.rs` module docs |
| **Preview authoritativeness** | comrak renders with CommonMark + GFM + extensions; editor highlight is advisory only | `crates/emend-core/src/parse/preview.rs` design doc |
| **Per-keystroke editing** | Swift owns buffer; Rust maintains shadow; deltas via `push_edit()` | `EditorCoordinator`, `EmendCore` |
| **Debounced autosave** | `DispatchQueue` serial queue, 1.5 s idle + 5 s hard cap | `AutosaveController` |
| **Debounced preview render** | Task-based debounce (~150 ms idle), coalesces rapid edits | `PreviewModel.scheduleRefresh()` |
| **Pure transforms (commands)** | `SmartLists` and `FormattingCommands` are pure functions, unit-testable without window | `app/Emend/Emend/Editor/` |
| **Self-write suppression** | Identity-keyed (mtime+len) in core + time-window in Swift `ConflictController` | `crates/emend-core/src/watcher`, `ConflictController` |
| **File watching** | notify + debouncer on OS threads; pure core classifier; foreign-trait bridge to Swift | `crates/emend-core/src/watcher`, `crates/emend-ffi/src/watcher` |
| **Incremental index** | Arena + path/name maps, O(1)-ish updates (no rescan on file ops) | `crates/emend-core/src/index`, `WorkspaceModel` tree updates |
| **Workspace persistence** | Locations + favorites/pins/icons persisted to UserDefaults | `WorkspaceModel`, `AppState` codable struct |
| **Quick Open superseding (NFR-002)** | Core batches + Cancel flag; FFI supersede via current_search; Swift generation guard | `crates/emend-core/src/search`, `crates/emend-ffi/src/search`, `QuickOpenModel` |
| **Bidirectional scroll sync** | Editor ↔ preview via `data-line` anchors + per-side mute window | `ScrollSync`, `bridge.js`, `crates/emend-core/src/parse/preview.rs` |
| **Offline preview rendering** | Core renders pure HTML (no I/O); Swift injects into offline template; CSP blocks remotes | `crates/emend-core/src/parse/preview.rs`, `PreviewWebView`, `template.html` |
| **PDF export multi-page** | Off-screen WKWebView + NSPrintOperation.runModal respects `@media print` rules | `PDFExport`, `theme.css` |

## Build & Deployment

- **Rust workspace** (`cargo build --release`) produces `libemend_ffi.a` (static lib for iOS-style XCFramework).
- **`just xcframework`** runs `uniffi-bindgen-swift`, links the static lib into an XCFramework, and generates Swift bindings (all git-ignored).
- **`just xcodeproj`** regenerates the Xcode app project from `project.yml` (XcodeGen, also git-ignored).
- **Final `.app`** built by Xcode 16.2, signed with automatic signing, deployed to `~/Applications/Emend.app` (or ad-hoc distribution).

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*
