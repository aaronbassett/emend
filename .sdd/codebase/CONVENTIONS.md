# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Overview

Emend enforces strict code conventions across Rust and Swift, with automated enforcement via `lefthook` pre-commit hooks. The guiding principle is **no panics across FFI boundaries** (NFR-003) and **Conventional Commits** enforced at the commit stage (DS-007).

## Rust Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| rustfmt | `rustfmt.toml` | `cargo fmt` |
| clippy | Workspace lints in `Cargo.toml` | `cargo clippy --all-targets -- -D warnings` |

### Style Rules (rustfmt.toml)

| Rule | Convention |
|------|------------|
| Edition | 2021 |
| Max width | 100 characters |
| Newline style | Unix (LF) |
| Heuristics | Default |

### Lint Policy (Workspace Lints)

The workspace enforces **zero panics** across FFI boundaries via workspace lint rules in `Cargo.toml` ([`workspace.lints`]):

```toml
[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"

[workspace.lints.rust]
unsafe_code = "warn"
missing_debug_implementations = "warn"
```

**Rationale**: NFR-003 requires no panics unwind across FFI. Every fallible operation returns `Result<_, EmendError>`; errors surface as normal Swift `Error`s via the UniFFI boundary. The denial is inherited by all crates via `[lints] workspace = true` in their `Cargo.toml`.

**Exception**: Integration tests co-located in `crates/emend-core/tests/` and `crates/emend-ffi/tests/` are scoped to allow these (see [Test Patterns](#test-patterns) below); their fixtures and assertions depend on `unwrap`/`expect` for clarity.

### Error Handling

All fallible operations return `Result<_, EmendError>` using the `thiserror` crate. The `EmendError` enum (`crates/emend-core/src/error.rs`) is the single source of truth for the FFI contract.

**Error Variants** (selected; see `error.rs` for complete list):
- `NotFound { path: String }` — File not found
- `PermissionDenied { path: String }` — Insufficient permissions
- `IoFailure { path, detail }` — Generalized I/O error
- `NameCollision { path }` — File/folder name conflict
- `NoteTooLarge { path, bytes, limit }` — Note exceeds size cap (FR-027a)
- `AiNotConfigured` — AI endpoint not configured
- `AiTimeout` — Request timeout
- `AiCancelled` — User cancelled request
- `AiHttp { status, detail }` — HTTP error (redacted, no API keys)
- `AiStreamMalformed { detail }` — SSE parse failure
- `Internal { detail }` — Caught panic or unexpected failure

**Contract**: The `emend-ffi` crate mirrors `EmendError` 1:1 via `#[derive(uniffi::Error)]` with an exhaustive `From<EmendError>` impl (no catch-all arm). Adding a variant here triggers a compile error in `emend-ffi` until the projection is updated, making FFI safety a compile-time guarantee.

### Naming Conventions

#### File & Directory Naming

| Type | Convention | Example |
|------|------------|---------|
| Crate root | lowercase snake_case | `emend-core`, `emend-ffi` |
| Module files | lowercase snake_case | `error.rs`, `document.rs` |
| Test integration | `tests/` subdirectory | `crates/emend-core/tests/` |
| Benchmark crate | Separate workspace member | `crates/emend-bench/` |

#### Code Element Naming (Rust)

| Type | Convention | Example |
|------|------------|---------|
| Variables | camelCase | `docPath`, `newText` |
| Constants | SCREAMING_SNAKE_CASE | `MAX_NOTE_SIZE`, `DEFAULT_BUFFER_SIZE` |
| Functions | snake_case, verb-prefix when changing state | `open_document()`, `push_edit()`, `len_utf16()` |
| Structs | PascalCase | `Document`, `EmendError`, `U16Range` |
| Enums | PascalCase, singular variant names | `LineCol`, `FileWatchEvent` |
| Trait names | PascalCase, often verb adjectives | `AiSink`, `SearchSink` |

#### Documentation

Doc comments use the standard Rust triple-slash (`///`) and are applied liberally:

- **Public types/functions**: Full doc comment with examples where non-obvious
- **Module root (`lib.rs`)**: Summary of the module's purpose and public surface
- **Complex invariants**: Annotate in doc comments (e.g., UTF-16↔char conversions in `document.rs`)
- **Test files (`tests/*.rs`)**: Module-level doc comment (prefixed `//!`) explaining the test scope and test obligations

**Example** (from `tests/document.rs`):
```rust
//! T018 — failing-first tests for the open-document model (`emend_core::document`).
//!
//! These tests pin down the **UTF-16 boundary** that the per-keystroke hot path
//! depends on (research §A2/§A3, FFI contract §3).
```

**Example** (from `tests/search_supersede.rs`, US3):
```rust
//! T072 — failing-first integration tests for the **cancellable** Quick Open
//! search driver (`emend_core::search`), the pure layer behind the streaming FFI
//! `quick_open_query` (US3 · FR-017, FR-018/SC-004, NFR-002; research §B2/§B7).
//!
//! The driver's whole reason to exist as a *separate* module from
//! [`emend_core::index`] is the **supersede/cancel** behaviour NFR-002 demands...
```

## Swift Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| SwiftFormat | `.swiftformat` + CLI flags | `swiftformat app swift --lint` |
| SwiftLint | `.swiftlint.yml` --strict | `swiftlint lint --strict` |

### SwiftFormat Configuration (`.swiftformat`)

**Key settings**:
- `--maxwidth 100` — Line wrapping at 100 characters
- `--indent 4` — Four-space indentation
- `--self remove` — Remove redundant `self.` prefixes
- `--importgrouping testable-bottom` — System/local imports first, `@testable` imports last
- `--commas inline` — Trailing commas on multi-line constructs
- `--trailingclosures` — Use trailing closure syntax
- `--wraparguments before-first` — Multi-line calls wrap before first argument
- `--disable wrapSingleLineComments` — Preserve intentional long comment lines as written

**Excluded paths**: Generated UniFFI bindings (`swift/EmendCore/Sources/EmendCoreFFI/`), build output (`.build/`, `DerivedData/`)

### SwiftLint Configuration (`.swiftlint.yml`)

**Line length**: Warning at 100 chars, error at 140 chars (ignores comments/URLs).

**Disabled rules** (conflicts with SwiftFormat):
- `trailing_comma` — SwiftFormat controls this via `--commas inline`

**Opt-in rules** (enforce stricter checking):
- `force_unwrapping` — Mirrors Rust `unwrap_used` denial (no force-unwraps)
- `implicitly_unwrapped_optional` — Avoid IUOs
- `empty_count` — Use `.isEmpty` over `.count == 0`
- `first_where` — Prefer `first(where:)` over `filter().first`
- `contains_over_first_not_nil` — Prefer `.contains` over `.first(where:) != nil`
- `closure_spacing` — Enforce consistent closure syntax
- `force_cast` — Disallow `as!` casts
- `force_try` — Disallow `try!`

**Excluded paths**:
- `swift/EmendCore/Sources/EmendCoreFFI/` — Generated UniFFI bindings (not our code)
- `DerivedData/` and `.build/` — Build artifacts
- `**/*.generated.swift` — All generated files

### SwiftFormat ↔ SwiftLint Interaction

**Known conflict**: When SwiftFormat wraps a long multi-line `if`/`guard` condition to stay within 100 chars, it places the opening brace `{` on its own line. SwiftLint's `opening_brace` rule then rejects this (expects brace on same line as `if`/`else`).

**Resolution**: Precompute a boolean `let` to keep conditions within one line, or use `guard … else { return }` where the brace follows `else` (exempt from the conflict):

```swift
// ✓ Preferred: condition on one line
let isValid = (longConditionPart1 && longConditionPart2 && longConditionPart3)
if isValid {
    // ...
}

// ✓ Also acceptable: guard … else { return }
guard let value = optionalValue,
      value > threshold else {
    return
}
```

### Nesting Rules

SwiftLint's `nesting` rule (severity: warning) allows types nested **at most 1 level deep**. When UIKit models or types exceed this, split them into separate files:

**Example**: `WorkspaceModel.swift` (primary) + `WorkspaceNode.swift` (nested `Kind` enum stays internal; tree nodes live in their own file to keep file_length under 400 lines).

### File and Type Length Limits

SwiftLint enforces:
- `file_length`: Error at 400 lines — split large files (e.g., models + private helpers → separate extensions)
- `type_body_length`: Warning at 250 lines — move private helper methods to same-file extensions

**Pattern**: When a file exceeds length, extract helper methods to a same-file `extension` block:

```swift
// WorkspaceModel.swift (primary responsibilities)
@MainActor
final class WorkspaceModel: ObservableObject {
    // Main model responsibilities
}

// WorkspaceModel+Helpers.swift or WorkspaceModel+Private.swift
extension WorkspaceModel {
    // Private helper methods, keeping the primary file lean
}
```

### Naming Conventions

#### File & Directory Naming

| Type | Convention | Example |
|------|------------|---------|
| SwiftUI views | PascalCase | `MainWindow.swift`, `EmendApp.swift` |
| SwiftUI view components | PascalCase | `EditorPane.swift` |
| Utility extensions | PascalCase + descriptive | `SecurityScopedBookmarks.swift` |
| Model classes | PascalCase + `Model` suffix | `WorkspaceModel.swift`, `TabModel.swift` |
| Coordinators (AppKit integration) | PascalCase + `Coordinator` suffix | `WorkspaceOutlineView+Coordinator.swift` |
| Sink bridges (FFI callbacks) | PascalCase + `Sink` suffix | `QuickOpenSink.swift`, `FsObserver.swift` |
| Test files | `Test.swift` or `Tests.swift` suffix | `BookmarkResolutionTests.swift` |

#### Code Element Naming (Swift)

| Type | Convention | Example |
|------|------------|---------|
| Variables | camelCase | `selectedLocation`, `isVisible`, `bookmarkData` |
| Constants (static) | camelCase (or SCREAMING_SNAKE_CASE for compile-time constants) | `defaultFolderSize` |
| Type names (struct/class/enum) | PascalCase | `MainWindow`, `AiStreamAdapter`, `SearchStreamAdapter` |
| Functions/methods | camelCase, verb-prefix for state change | `addLocation()`, `openDocument()`, `onToken(_:)` |
| Properties | camelCase | `locations`, `selection`, `abiVersion` |
| Boolean properties | `is`/`has` prefix when non-obvious | `isVisible`, `hasError` |

#### SwiftUI Conventions

- **State properties**: Prefix with `@State private var` for internal state
- **Main actor**: Test classes touching `@MainActor` or AppKit are annotated `@MainActor` (Swift 6 strict concurrency)
- **View components**: Extract into computed `var` properties or `@ViewBuilder` methods for clarity (e.g., `var sidebar: some View`)
- **Closures**: Use trailing closure syntax and verb-based naming for callbacks (e.g., `onTerminate`, `onToken`)

**Example** (from `Streaming.swift`):
```swift
public static func make(
    onTerminate: @escaping @Sendable () -> Void = {}
) -> (sink: AiStreamAdapter, stream: AsyncThrowingStream<String, Error>) {
    // ...
}
```

## SwiftUI ↔ AppKit Bridging Patterns (US2, Phase 4)

### @MainActor ObservableObject Models

All observable state models that participate in SwiftUI or touch AppKit are marked `@MainActor` for Swift 6 strict concurrency compliance:

```swift
@MainActor
final class WorkspaceModel: ObservableObject {
    @Published private(set) var roots: [WorkspaceNode] = []
    @Published private(set) var fsRefreshTick = 0
    let workspace: WorkspaceHandle
}

@MainActor
final class TabModel: ObservableObject {
    @Published private(set) var tabs: [Tab] = []
    @Published var activeID: Tab.ID?
}
```

**Rationale**: Models own Rust handles and emit `@Published` changes, which must occur on the main thread for SwiftUI binding updates.

### NSViewRepresentable + @MainActor Coordinator

When wrapping AppKit views (e.g., `NSOutlineView`, `NSTextView`), create a coordinator that conforms to AppKit protocols. The coordinator's protocol conformances (`NSOutlineViewDataSource`, `NSOutlineViewDelegate`, `NSMenuDelegate`) are SDK-declared as `@MainActor`, so the coordinator is safe as-is:

```swift
struct WorkspaceOutlineView: NSViewRepresentable {
    @ObservedObject var model: WorkspaceModel
    
    final class Coordinator: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
        // All SDK protocols are @MainActor, so Coordinator is safe
        func outlineView(_: NSOutlineView, numberOfChildrenOfItem: Any?) -> Int {
            // Safe to call model methods here (both are @MainActor)
        }
    }
    
    func makeCoordinator() -> Coordinator {
        Coordinator(model: model)
    }
}
```

**Important**: Not all AppKit delegate protocols are `@MainActor`-marked. For example, `NSTextStorageDelegate` is not. Do NOT directly conform in a `@MainActor` context; instead, create a non-isolated intermediate:

```swift
// ✗ Do not do this (NSTextStorageDelegate has nonisolated methods)
@MainActor
final class TextStorageObserver: NSTextStorageDelegate { }

// ✓ Correct: create a bridge
final class TextStorageObserver: NSTextStorageDelegate {
    private let onChangeMain: @MainActor @Sendable (NSTextStorage) -> Void
    
    nonisolated func textStorageDidProcessEditing(_ notification: Notification) {
        Task { @MainActor in onChangeMain(...) }
    }
}
```

### Cross-Thread Callbacks: Sendable Closures (US2, Phase 4 & US3)

When Rust callbacks arrive on a background thread (e.g., `notify` FSEvents thread or search worker thread), bridge to the main actor via a `@Sendable` closure in a final-class holder. This pattern **holds only immutable closures** and is itself `Sendable`:

```swift
// FsObserver bridges background-thread Rust watcher callbacks to main actor
final class FsObserver: DocObserver, Sendable {
    private let onChange: @Sendable (ChangeEvent) -> Void
    
    init(onChange: @escaping @Sendable (ChangeEvent) -> Void) {
        self.onChange = onChange
    }
    
    func onFsChange(change: ChangeEvent) {
        onChange(change)  // Closure executes on watcher thread
    }
}

// WorkspaceModel uses it:
private lazy var fsObserver = FsObserver { [weak self] change in
    Task { @MainActor in self?.handleFsChange(change) }
}
```

**US3 example** (`QuickOpenSink` bridges streaming search results):
```swift
/// Bridges the core's SearchSink callbacks (on a background search worker)
/// to @Sendable closures. Holds only immutable closures, so it is safely Sendable
/// (mirrors FsObserver).
final class QuickOpenSink: SearchSink, Sendable {
    private let batchHandler: @Sendable ([SearchHit]) -> Void
    private let doneHandler: @Sendable () -> Void

    init(
        onBatch: @escaping @Sendable ([SearchHit]) -> Void,
        onDone: @escaping @Sendable () -> Void
    ) {
        batchHandler = onBatch
        doneHandler = onDone
    }

    func onResults(batch: [SearchHit]) {
        batchHandler(batch)  // Executes on search worker thread
    }

    func onDone() {
        doneHandler()
    }
}

// QuickOpenModel uses it:
let sink = QuickOpenSink(
    onBatch: { [weak self] batch in
        Task { @MainActor in self?.apply(batch: batch, generation: gen) }
    },
    onDone: {}
)
```

**Key pattern**:
1. Sink class is `final` and `Sendable`
2. Holds only `@Sendable` closures (immutable, can cross threads)
3. Callback is invoked directly (on the calling thread)
4. Caller wraps the closure with `Task { @MainActor in … }` to hop to main thread

This avoids `@MainActor` annotation on the sink itself (which would require the calls *from* Rust to be on the main thread — they're not), while still ensuring SwiftUI mutations happen on main.

## Common Patterns

### Rust: Error Propagation

All public functions in `emend-core` return `Result<T, EmendError>`. Use the `?` operator:

```rust
pub fn open_document(path: &Path) -> Result<Document, EmendError> {
    let text = fs::read_string(path)?;  // ? propagates EmendError
    Document::from_text(&text)
}
```

**No `.unwrap()` or `.expect()`** in library code—both violate NFR-003. Panics in async tasks or FFI boundaries are contained by `catch_unwind`, but returning `Result` is clearer.

### Rust: FFI Boundary Types

Cross-FFI types use UniFFI-compatible primitives:
- `String` (not `&str`)
- `u32`, `u64` (not `usize`)
- `bool`, `f32`, `f64`
- Struct/enum fields restricted to the above (see `error.rs` for constraints)

**UTF-16 Code Units**: All text ranges crossing FFI are expressed as `U16Range { start: u32, len: u32 }` (UTF-16 code units) to map 1:1 onto `NSRange` in Swift.

### Rust: Pure Search Driver (US3)

The search module (`emend_core::search`) is a **pure, tokio-free** driver that ranks and streams results:

```rust
/// Rank `query` over the index in batches of `batch_size`, emitting via `sink`.
/// Returns whether the full set was streamed (true) or was superseded (false).
pub fn quick_open(
    index: &Index,
    query: &str,
    limit: usize,
    batch_size: usize,
    cancel: &Cancel,
    mut sink: impl FnMut(Vec<SearchHit>),
) -> bool {
    // Rank the query (synchronous, fast)
    let ranked = index.query(query, limit);
    // Stream batches, checking cancel flag between batches
    for batch in ranked.chunks(batch_size) {
        if cancel.is_cancelled() {
            return false;  // Superseded; worker stops emitting
        }
        sink(batch.to_vec());
    }
    true  // Completed; FFI fires terminal on_done
}
```

**Rationale** (Constitution V — decision logic in core):
- Ranking happens once, synchronously
- Batching logic is deterministic (no async, no timing-dependent decisions)
- Cancellation is a simple `&Cancel` flag, not tokio-dependent
- FFI layer (`emend_ffi/src/search.rs`) bridges the `Cancel` to `CancellationToken` and handles panic containment, but delegates ranking/emission to this pure driver

### Swift: AsyncStream Adapters

The `Streaming.swift` module bridges UniFFI foreign-trait callbacks to Swift `AsyncStream`s:

```swift
public final class AiStreamAdapter: AiSink {
    private let continuation: AsyncThrowingStream<String, Error>.Continuation
    
    public func onToken(text: String) {
        continuation.yield(text)
    }
    
    public func onDone(full _: String) {
        continuation.finish()
    }
    
    public func onError(err: FfiError) {
        continuation.finish(throwing: err)
    }
}
```

Call sites wire an `onTerminate` hook to cancel the Rust work when the stream is torn down.

### Swift: Pure Transform Functions

Editor behavior transforms (e.g., `SmartLists`, `FormattingCommands`) are pure functions over `(text: String, selection: NSRange) -> Edit?`:

```swift
enum SmartLists {
    struct Edit: Equatable {
        let range: NSRange
        let replacement: String
        let selectionAfter: NSRange
    }

    static func newline(in text: NSString, selection: NSRange) -> Edit? {
        // Pure logic: no side effects, no AppKit
    }
}
```

These **headless transforms are unit-tested without a window** (Constitution VII). The editor view applies the returned `Edit` to its `NSTextView` through the normal `shouldChangeText`/`didChangeText` path.

### Swift: View Composition

Extract complex view logic into computed properties for readability:

```swift
var sidebar: some View {
    List(locations, selection: $selection) { location in
        Label(location.name, systemImage: "folder")
    }
}

@ViewBuilder private var editorPane: some View {
    if let doc = openDoc {
        MarkdownEditorView(document: doc)
    } else {
        EmptyView()
    }
}
```

## Git & Commit Conventions

### Conventional Commits (DS-007)

Enforced at commit time by `lefthook` hook (see `lefthook.yml` commit-msg section).

**Format**: `type(scope): description [!]`

**Type**: One of:
- `feat` — New feature
- `fix` — Bug fix
- `docs` — Documentation
- `style` — Code formatting / style (no logic change)
- `refactor` — Code restructure (no logic change)
- `perf` — Performance improvement
- `test` — Adding/updating tests
- `build` — Build system / dependencies
- `ci` — CI/CD configuration
- `chore` — Maintenance / tooling
- `revert` — Revert a prior commit

**Scope** (optional): Lowercase, hyphenated, e.g., `(editor)`, `(ffi-boundary)`, `(swift)`, `(search)`.

**Breaking change** (optional): Suffix `!` before `:` (e.g., `feat(ffi)!: new ABI version`).

**Examples**:
```
feat(document): add line-column tracking
fix(fs): tolerate CRLF in file reads
docs: update UTF-16 boundary documentation
test(document): add astral-char UTF-16 tests
feat(search): add cancellable quick-open query (US3)
ci: enforce MSRV 1.85
```

The commit-msg hook validates the subject line against this pattern and rejects non-conforming commits.

## Pre-Commit Hooks (Lefthook)

Run once to install: `lefthook install` (or `just hooks`).

Hooks run **in parallel** on `git commit` and validate:

| Hook | Glob | Command | Notes |
|------|------|---------|-------|
| rust-fmt | `*.rs` | `cargo fmt --check` | Rejects if unformatted |
| rust-clippy | `*.rs` | `cargo clippy --all-targets --offline -- -D warnings` | Rejects if warnings exist |
| swift-format | `*.swift` | `swiftformat {staged_files} --lint` | Gracefully skips if not installed |
| swift-lint | `*.swift` | `swiftlint lint --quiet --strict` | Gracefully skips if not installed |
| commit-msg | (all) | Conventional Commits validation | Rejects non-conforming subjects |

**Pre-commit runs in parallel** for speed. If any check fails, the commit is rejected; staged changes remain staged for fixing.

To run all checks locally (mirrors CI): `just check` or `cargo fmt && cargo clippy && cargo test && swift-lint`.

## Code Organization

### Rust Crate Structure

- **`crates/emend-core/src/lib.rs`**: Module declaration + public type re-exports (e.g., `EmendError`, `U16Range`)
- **`crates/emend-core/src/error.rs`**: `EmendError` enum and Display/Error impls
- **`crates/emend-core/src/document.rs`**: Open-document model (shadow rope + UTF-16 indexing)
- **`crates/emend-core/src/fs.rs`**: Atomic+durable file I/O
- **`crates/emend-core/src/workspace.rs`**: Workspace file operations, collision-safe create/rename/move
- **`crates/emend-core/src/watcher.rs`**: Filesystem watching, debounce, rename correlation, self-write suppression
- **`crates/emend-core/src/index.rs`**: Incremental search index (nucleo-based fuzzy ranking, wiki-link O(1) lookup)
- **`crates/emend-core/src/search.rs`**: Pure, cancellable quick-open search driver (ranks and streams in batches)
- **`crates/emend-core/tests/`**: Integration tests (see [Testing](#testing))
- **`crates/emend-ffi/src/lib.rs`**: UniFFI `#[uniffi::export]` shim + panic containment
- **`crates/emend-ffi/src/search.rs`**: FFI projection of streaming search (bridges cancellation, spawns worker, panic containment)
- **`crates/emend-bench/benches/`**: Criterion micro-benchmarks

### Swift Module Structure

- **`swift/EmendCore/Sources/EmendCore/EmendCore.swift`**: Clean public API, re-exports `EmendCoreFFI`
- **`swift/EmendCore/Sources/EmendCore/Streaming.swift`**: AsyncStream adapters over foreign-trait callbacks
- **`swift/EmendCore/Sources/EmendCoreFFI/`** (generated): UniFFI bindings (excluded from lint)
- **`app/Emend/Emend/`**: SwiftUI app (views, state, utilities, pure transforms)
  - **`Sidebar/`**: Workspace tree model, `NSOutlineView` coordination, drag-drop logic
  - **`Tabs/`**: Tab management, open-document state
  - **`Editor/`**: Editor view, syntax highlighting, text storage delegates, pure transforms (`SmartLists`, `FormattingCommands`)
  - **`QuickOpen/`**: Quick Open palette model + sink bridge (US3)
- **`app/Emend/EmendTests/`**: App-level XCTest tests (headless, no GUI automation)

### Editor Transform Organization

Pure, testable transforms are organized by feature:
- **`app/Emend/Emend/Editor/SmartLists.swift`** — List continuation + termination logic (T045)
- **`app/Emend/Emend/Editor/FormattingCommands.swift`** — Bold, italic, code formatting (T046)
- **`app/Emend/Emend/Editor/SyntaxAttributing.swift`** — Highlight synthesis from tree-sitter (T047)

Each transform is pure and unit-tested headlessly in `app/Emend/EmendTests/`.

### Quick Open Organization (US3)

- **`crates/emend-core/src/index.rs`**: `Index` type and ranking logic (fuzzy match, basename boost, wiki-link O(1) lookup)
- **`crates/emend-core/src/search.rs`**: Pure `quick_open` driver (ranks, batches, cancels)
- **`crates/emend-ffi/src/search.rs`**: FFI projection (`SearchHandle`, async worker, panic containment)
- **`app/Emend/Emend/QuickOpen/QuickOpenModel.swift`**: SwiftUI state, sink attachment, generation-guarded batch apply
- **`app/Emend/Emend/QuickOpen/QuickOpenSink.swift`**: Sink bridge (holds `@Sendable` closures, hops to `@MainActor`)
- **`crates/emend-core/tests/search_supersede.rs`**: Supersede/cancel semantics (T072, pure tokio-free determinism)
- **`crates/emend-bench/benches/quick_open.rs`**: Perf budget 100 ms p95 warm over 10k index (T071, SC-004)

## Import Ordering

### Rust

Handled by rustfmt automatically. Standard order:
1. External crates (`use tokio::...`, `use thiserror::...`)
2. Crate root (`use emend_core::...`)
3. Internal modules (`use crate::document::...`)
4. Std library (`use std::...`)

### Swift

Handled by SwiftFormat with `--importgrouping testable-bottom`:
1. System frameworks (`import Foundation`, `import SwiftUI`)
2. Local packages (`import EmendCore`)
3. `@testable` imports (test files only, appears last)

---

*This document defines HOW to write code. Update when conventions change.*
