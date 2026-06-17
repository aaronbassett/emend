# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Architecture Overview

Emend is a **hybrid Rust+Swift native macOS Markdown editor** with a cleanly separated boundary:

- **Rust core** (`crates/emend-core`) houses ALL business logic: file I/O, document parsing, file watching, indexing, search, and AI client integration. The core is **toolchain-free** — it has no FFI dependency and is fully testable with `cargo test` in isolation.
- **UniFFI shim** (`crates/emend-ffi`) provides a thin boundary layer that exports the core's capabilities to Swift and manages async infrastructure (tokio runtime, cancellation tokens).
- **Swift/SwiftUI app** (`app/Emend`) wraps the core in a native macOS UI with `NSTextView` editor, outline sidebar, and preview WebView.

The boundary is **synchronous on the hot path** (per-keystroke edits) and **asynchronous only for AI and search** (with cancellable Rust-owned handles). This design allows the core to stay independent and testable while the UI can safely dispatch background work.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Layered (horizontal)** | Presentation (Swift/SwiftUI) → API boundary (UniFFI) → Business logic (Rust core) |
| **Modular monolith** | Single deployable macOS app; no microservices or network splits |
| **Rust corelib + FFI shim** | Heavy separation of concerns: business logic in pure Rust, FFI concerns isolated in a thin wrapper |
| **Synchronous hot path, async background** | Per-keystroke edits cross the boundary synchronously; AI and search use async Rust-owned handles |
| **UTF-16 boundary contract** | All text ranges crossing the FFI boundary are UTF-16 code units, mapping 1:1 to `NSRange` |
| **Swift owns text buffer** | Canonical text storage lives in NSTextStorage; Rust maintains a shadow ropey rope for structural queries |

## Core Components

### 1. Rust Core (`crates/emend-core`)

**Purpose**: The engine — file I/O, document state, parsing, search, AI streaming, watching.

**Location**: `crates/emend-core/src/`

**Modules**:

- **`error.rs`** — Structured `EmendError` type (single source of truth for FFI contract). Variants carry context fields (paths, limits, byte counts) for UI rendering. Exhaustive enum (not `#[non_exhaustive]`) so the FFI projection can be a closed, compiler-checked mirror.
- **`fs.rs`** — Atomic+durable writes and tolerant reads. Write path: temp file in same directory → fsync → atomic rename → fsync directory (guarantees no torn writes). Read path: strips UTF-8 BOM, preserves CRLF, decodes invalid UTF-8 lossily. On macOS, `File::sync_all()` already calls `fcntl(F_FULLFSYNC)` for true durability.
- **`document.rs`** — The open-document model: a shadow ropey rope + UTF-16/char/line indices. Backs all per-keystroke edits, structural queries (highlight, outline), and search. Converts at exactly one place on every boundary call, never panicking — all conversions are checked and mapped to `EmendError`.
- **Placeholder modules** (to be added by `/sdd:implement`): `watcher`, `index`, `parse`, `search`, `ai`.

**Dependencies**: `thiserror`, `ropey`, `tempfile`, `tokio` (only in FFI shim), `tree-sitter`, `comrak`, `syntect`, `nucleo`, `notify`, `reqwest`, `serde`.

**Dependents**: `crates/emend-ffi`, tests, benchmarks.

**Constraints (Constitution V)**:
- **NO FFI dependency** — core never imports `uniffi`. This keeps the core standalone and testable.
- **NO panics** — clippy lints deny `unwrap_used`, `expect_used`, `panic`.
- **No async primitives** — tokio only enters in the FFI shim (async infrastructure lives outside the core).

### 2. FFI Shim (`crates/emend-ffi`)

**Purpose**: Thin projection of the core to Swift. Manages async infrastructure, panic containment, and error mapping.

**Location**: `crates/emend-ffi/src/`

**Modules**:

- **`lib.rs`** — FFI entry points (e.g., `read_file_at`, `core_abi_version`). Each `#[uniffi::export]` function wraps a core function, handling errors. UniFFI's scaffolding automatically wraps calls in `catch_unwind`, so panics cannot unwind into Swift.
- **`error.rs`** — `#[derive(uniffi::Error)]` projection of `EmendError`. Keeps the same `Display` wording so Swift sees the same error message.
- **`panic.rs`** — Custom panic hook (if needed for debugging; not yet implemented).
- **`handles.rs`** — Rust-owned cancellation tokens and foreign-trait callback sinks (for AI and search streaming results). The tokio runtime lives here, not in the core, so later tasks (T025+) can register async work.

**Dependencies**: `emend-core`, `uniffi`, `thiserror`, `tokio`, `tokio-util`.

**Dependents**: Swift package (`swift/EmendCore`).

**Constraint**: Keep FFI free of business logic. It is ONLY a projection and scaffolding layer.

### 3. Swift Package (`swift/EmendCore`)

**Purpose**: Local SwiftPM package wrapping the Rust XCFramework and offering idiomatic Swift re-exports.

**Location**: `swift/EmendCore/`

**Layout**:

- **`Package.swift`** — Package manifest. Declares two targets: `EmendCoreFFI` (generated UniFFI bindings, compiled in Swift 5 for foreign-trait compatibility) and `EmendCore` (hand-written Swift wrappers, Swift 6).
- **`Sources/EmendCoreFFI/`** — Generated by `uniffi-bindgen-swift` (git-ignored). Exposes the C module and the `emend_ffiFFI` namespace.
- **`Sources/EmendCore/`** — Hand-written wrappers (e.g., `EmendCore.swift`) that re-export the generated surface and add idiomatic adapters (e.g., `AsyncStream` over foreign-trait sinks) as features land.
- **`../../xcframework/EmendCore.xcframework`** — The compiled Rust binary + C module headers (git-ignored, built by `just xcframework`).

**Dependents**: Xcode app (`app/Emend`).

### 4. macOS App (`app/Emend`)

**Purpose**: The SwiftUI native UI — text editor, file navigation, preview, AI integration.

**Location**: `app/Emend/`

**Layout** (generated by XcodeGen from `project.yml`):

- **`Emend/EmendApp.swift`** — Entry point. Single-window `App` hosting `MainWindow`.
- **`Emend/Shell/MainWindow.swift`** — Three-pane layout: editor (left), file tree (center), info sidebar (right). US1 wires the editor pane; US2 and US6 fill the sidebar and info pane later.
- **`Emend/Platform/SecurityScopedBookmarks.swift`** — macOS sandbox integration: resolves security-scoped bookmarks so Rust can read/write user-granted folders.
- **`Emend/Editor/`** — Live editor pane (US1):
  - **`MarkdownEditorView.swift`** — `NSViewRepresentable` wrapping a TextKit 2 `MarkdownTextView` + coordinate per-keystroke edits.
  - **`MarkdownTextView.swift`** — `NSTextView` subclass that hooks list/formatting keys (Return, Tab, ⌘B/I/K/⇧T) to pure transforms.
  - **`EditorCoordinator.swift`** — `NSTextStorageDelegate` that extracts UTF-16 deltas, calls `pushEdit()` synchronously, then schedules re-attribution.
  - **`SyntaxAttributing.swift`** — Pure function mapping core `StyleSpan`s to AppKit display attributes (bold/italic/headings/code/quote inline, markers dimmed).
  - **`SmartLists.swift`** — Pure transforms: `newline()` (continue/terminate list), `renumber()` (sequential ordered lists), `indent()`/`outdent()` (shift nesting).
  - **`FormattingCommands.swift`** — Pure transforms: `bold()`, `italic()`, `link()`, `task()` (insert markers around selection).
  - **`AutosaveController.swift`** — Debounced (1.5 s idle + 5 s hard cap) atomic flush on private serial queue (never main thread). Errors surface via callback.
- **`EmendTests/`** — Unit tests (@testable import Emend). No UI automation; runs fast.

**Dependencies**: `EmendCore` SwiftPM package, AppKit (`NSTextView`, `NSOutlineView`), SwiftUI.

**Structure** (to be filled by `/sdd:implement`):
- `Screens/` — Pane views (location tree, search results, etc.)
- `Models/` — SwiftUI state containers
- `Bindings/` — Adapters from Rust types to Swift UI types
- `Services/` — UI-facing wrappers over `EmendCore`

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
AutosaveController.noteEdit() rearms debounce
```

**Why synchronous**: Every keystroke must be reflected in the buffer immediately. Pushing work to background would introduce latency, risking dropped keystrokes or race conditions with later edits.

**Why off-main-thread**: The Rust core's incremental rope operations are fast enough for per-keystroke throughput but may call `tree-sitter` highlighting in the future — keeping the main thread responsive requires the call not to block.

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

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| **Swift/SwiftUI app** | UI rendering, event handling, editor state | `EmendCore` (boundary), AppKit | Directly access files, Rust data structures |
| **Swift `EmendCore` wrapper** | Type adaptation, async stream wrapping | Generated UniFFI bindings | App state, UI models |
| **UniFFI boundary** | Foreign-trait scaffolding, error projection, panic containment | `emend-core`, async runtime | Anything beyond scaffolding |
| **Rust core (`emend-core`)** | All business logic: files, documents, parsing, search, AI | Only standard library + workspace deps | FFI, async runtime, platform code |

**Dependency rules**:
- Higher layers (Swift, FFI) depend on lower layers (core). Never vice versa.
- Core has **no knowledge** of FFI or Swift.
- FFI is a **thin projection only** — no business logic.

## Dependency Rules

1. **Core → nothing but std + deps**. No FFI, no platform code.
2. **FFI → core + uniffi + tokio**. Business logic lives in (1), scaffolding here.
3. **Swift → EmendCore wrapper only**. Never call generated FFI bindings directly; always go through the `EmendCore` module re-export.
4. **App → Swift + EmendCore + AppKit**. Never raw FFI.

## Key Interfaces & Contracts

### FFI Contract: Open Document

**Location**: `specs/001-markdown-editor/contracts/ffi-interface.md`

| Export | Signature | Purpose |
|--------|-----------|---------|
| `read_file_at(path: String) -> Result<String, FfiError>` | Tolerant read at sandbox boundary | Open document flow |
| `core_abi_version() -> u32` | ABI probe | Version/compatibility check |
| `open_document(path: String) -> Result<OpenDocHandle, FfiError>` | Create a document handle | Editor initialization |
| `close_document(handle: OpenDocHandle) -> Result<(), FfiError>` | Release a document | Window close |
| `push_edit(handle, range: U16Range, replacement: String) -> Result<(), FfiError>` | Apply keystroke delta | Per-keystroke sync path |
| `highlight_spans(handle, viewport: U16Range) -> Result<Vec<StyleSpan>, FfiError>` | Incremental highlight | Syntax coloring |
| `flush(handle) -> Result<(), FfiError>` | Write to disk | Autosave |

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

**StyleSpan**: `{ range: U16Range, class: StyleClass }` — a syntax highlighting span. `StyleClass` is an enum: `heading(Int)`, `strong`, `emphasis`, `inlineCode`, `codeBlock`, `blockQuote`, `listMarker`, `link`, `syntaxMarker`, `highlight`.

**OpenDocHandle**: Opaque Rust handle representing an open `Document`. Returned by `open_document()`, passed to `push_edit()`, `highlightSpans()`, `flush()`, and `close_document()`.

## State Management

| State Type | Location | Pattern | Notes |
|------------|----------|---------|-------|
| **Document buffer (canonical)** | Swift `NSTextStorage` | Source of truth for display; edits flow user → NSTextStorage → Rust | Hot path; kept in sync via `push_edit` deltas |
| **Document (shadow)** | Rust `Document` (ropey rope) | Mirrors NSTextStorage; used for structural queries (highlight, outline, search) | Synced delta-for-delta from Swift |
| **File on disk** | Rust `fs` module | Atomic writes via tempfile + rename | Debounced autosave (Constitution III, FR-009a) |
| **Highlight cache** | Rust `parse` module (tree-sitter) | Incremental, invalidated by `push_edit` | Computed on-demand by highlight queries |
| **Open-document list** | Swift app state (`MainWindow.openDoc`) | Registry of handles + text + autosave + UI state | Tracks which Rust `Document` handles are live |
| **Cancellation tokens** | Rust `handles` module (tokio-util) | Owned by Rust, cancelled by Swift | AI and search long-running tasks |

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Error handling** | Structured `EmendError` enum, mapped at FFI | `crates/emend-core/src/error.rs`, `crates/emend-ffi/src/error.rs` |
| **Panic containment** | UniFFI `catch_unwind`, lint deny `panic`/`unwrap` | Lints in `Cargo.toml`, FFI scaffolding |
| **UTF-16 boundary safety** | `U16Range` type, checked conversions, surrogate-pair detection | `crates/emend-core/src/document.rs` |
| **Atomic durability** | Temp file + fsync + rename + fsync dir | `crates/emend-core/src/fs.rs` |
| **Async cancellation** | `tokio::sync::CancellationToken` + foreign-trait sinks | `crates/emend-ffi/src/handles.rs` |
| **Privacy** | No network unless AI configured + invoked; Keychain for API key; transient to Rust, redacted in HTTP client | `crates/emend-core`, Swift app bindings |
| **Incremental syntax highlight** | tree-sitter (editor, advisory) vs. comrak (preview, authoritative) | `crates/emend-core/src/parse` (Phase 1+) |
| **Per-keystroke editing** | Swift owns buffer; Rust maintains shadow; deltas via `push_edit()` | `EditorCoordinator`, `EmendCore` |
| **Debounced autosave** | `DispatchQueue` serial queue, 1.5 s idle + 5 s hard cap | `AutosaveController` |
| **Pure transforms (commands)** | `SmartLists` and `FormattingCommands` are pure functions, unit-testable without window | `app/Emend/Emend/Editor/` |

## Build & Deployment

- **Rust workspace** (`cargo build --release`) produces `libemend_ffi.a` (static lib for iOS-style XCFramework).
- **`just xcframework`** runs `uniffi-bindgen-swift`, links the static lib into an XCFramework, and generates Swift bindings (all git-ignored).
- **`just xcodeproj`** regenerates the Xcode app project from `project.yml` (XcodeGen, also git-ignored).
- **Final `.app`** built by Xcode 16.2, signed with automatic signing, deployed to `~/Applications/Emend.app` (or ad-hoc distribution).

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*
