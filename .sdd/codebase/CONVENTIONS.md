# Coding Conventions

> **Purpose**: Document code style, naming conventions, error handling, and common patterns.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (US7 Phase 9 + Phase 10 Polish)

## Overview

Emend enforces strict code conventions across Rust and Swift, with automated enforcement via `lefthook` pre-commit hooks. The guiding principle is **no panics across FFI boundaries** (NFR-003) and **Conventional Commits** enforced at the commit stage (DS-007). Phase 10 added quality/performance verification without changing core conventions.

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

### Benchmarking Policy (Phase 10)

Performance budgets are **tracked but non-blocking** (Constitution IV). Criterion benches live in `crates/emend-bench/`:

- **Benchmarks must stay panic-free** — use `black_box` to prevent optimizer deletion and fallible `.ok()` instead of `.unwrap()` for any operations that might fail.
- **Setup/cleanup are excluded from timing** via `iter_batched` — the measured work is precisely what the user experiences.
- **Numbers are recorded in implementation reports**, not enforced in CI — regressions surface as documented deviations, not gate failures.

Example (`crates/emend-bench/benches/highlight.rs`, SC-003):
```rust
use std::hint::black_box;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

fn highlight_bench(c: &mut Criterion) {
    c.bench_function("incremental_reparse_1mb", |b| {
        b.iter_batched(
            || build_doc(1_000_000),  // Setup: per-iteration, excluded from timing
            |doc| {
                let mut h = Highlighter::new(&doc);
                // Measured: one keystroke reparse + viewport highlight query
                h.apply_edit(U16Range { start: 500_000, len: 1 });
                black_box(h.highlight_spans(U16Range { start: 499_900, len: 200 }))
            },
            BatchSize::SmallInput,
        );
    });
}
```

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
- `AiOversizedInput { bytes, limit }` — Document exceeds input size cap (FR-036a, US6)
- `Internal { detail }` — Caught panic or unexpected failure

**Contract**: The `emend-ffi` crate mirrors `EmendError` 1:1 via `#[derive(uniffi::Error)]` with an exhaustive `From<EmendError>` impl (no catch-all arm). Adding a variant here triggers a compile error in `emend-ffi` until the projection is updated, making FFI safety a compile-time guarantee.

### Naming Conventions

#### File & Directory Naming

| Type | Convention | Example |
|------|------------|---------|
| Crate root | lowercase snake_case | `emend-core`, `emend-ffi` |
| Module files | lowercase snake_case | `error.rs`, `document.rs`, `ai.rs`, `settings.rs` |
| Test integration | `tests/` subdirectory | `crates/emend-core/tests/` |
| Benchmark crate | Separate workspace member | `crates/emend-bench/` |

#### Code Element Naming (Rust)

| Type | Convention | Example |
|------|------------|---------|
| Variables | camelCase | `docPath`, `newText` |
| Constants | SCREAMING_SNAKE_CASE | `MAX_NOTE_SIZE`, `DEFAULT_BUFFER_SIZE`, `MIN_FONT_SIZE_PT` |
| Functions | snake_case, verb-prefix when changing state | `open_document()`, `push_edit()`, `len_utf16()`, `check_input_size()` |
| Structs | PascalCase | `Document`, `EmendError`, `U16Range`, `ApiKey`, `TypographySettings` |
| Enums | PascalCase, singular variant names | `LineCol`, `FileWatchEvent`, `LinkKind`, `SseEvent` |
| Trait names | PascalCase, often verb adjectives | `AiSink`, `SearchSink` |

#### Documentation

Doc comments use the standard Rust triple-slash (`///`) and are applied liberally:

- **Public types/functions**: Full doc comment with examples where non-obvious
- **Module root (`lib.rs`)**: Summary of the module's purpose and public surface
- **Complex invariants**: Annotate in doc comments (e.g., UTF-16↔char conversions in `document.rs`, secret hygiene in `ai.rs`)
- **Test files (`tests/*.rs`)**: Module-level doc comment (prefixed `//!`) explaining the test scope and test obligations

**Example** (from `tests/document.rs`):
```rust
//! T018 — failing-first tests for the open-document model (`emend_core::document`).
//!
//! These tests pin down the **UTF-16 boundary** that the per-keystroke hot path
//! depends on (research §A2/§A3, FFI contract §3).
```

**Example** (from `tests/links.rs`, US5):
```rust
//! T095 — wiki-link & task extraction + resolution (US5 · FR-019/019a, FR-014).
//!
//! These integration tests pin two behaviours the spec calls load-bearing:
//! 1. **Deterministic resolution for duplicate basenames (FR-019a).**
//! 2. **A rename leaves old links unresolved (FR-019a, v1).**
```

**Example** (from `tests/settings.rs`, US7):
```rust
//! T123 — failing-first integration tests for the typography settings store
//! (`emend_core::settings`), the global editor/preview typography preferences
//! (US7 · FR-038/FR-039; FFI contract §8).
```

**Example** (from `tests/large_file.rs`, Phase 10):
```rust
//! T133 — the dedicated **FR-027a** integration test: max-note-size cap +
//! incremental re-parse on a large document.
//!
//! FR-027a: *"System MUST define a maximum supported note size; beyond it,
//! behavior MUST be graceful (open read-only or refuse with a clear message)
//! rather than hang or exhaust memory."*
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
- `function_body_length` — Warning at 50 lines; extract per-step helpers (US4 · `PDFExport`)
- `nesting` — Allow types nested at most 1 level deep

**Excluded paths**:
- `swift/EmendCore/Sources/EmendCoreFFI/` — Generated UniFFI bindings (not our code)
- `DerivedData/` and `.build/` — Build artifacts
- `**/*.generated.swift` — All generated files

### SwiftFormat ↔ SwiftLint Interaction

**Known conflict**: When SwiftFormat wraps a long multi-line `if`/`guard` condition to stay within 100 chars, it places the opening brace `{` on its own line. SwiftLint's `opening_brace` rule then rejects this (expects brace on same line as `if`/`else`).

**Resolution** (US5 pattern): Precompute a boolean `let` to keep conditions within one line, or use `guard … else { continue }` where the brace follows `else` (exempt from the conflict). Alternatively, extract bracket-pair checks into single-expression helpers:

```swift
// ✓ Preferred: condition on one line via helper
private func doubledStartingAt(_ pos: Int, in text: NSString) -> Bool {
    pos + 1 < text.length && text.character(at: pos) == 91 && text.character(at: pos + 1) == 91
}

if doubledStartingAt(caret - 2, in: text) { /* ... */ }

// ✓ Also acceptable: guard … else { continue }
guard let value = optionalValue,
      value > threshold else {
    continue
}
```

### Nesting Rules

SwiftLint's `nesting` rule (severity: warning) allows types nested **at most 1 level deep**. When UIKit models or types exceed this, split them into separate files:

**Example** (US5): `EditorCoordinator` grew past the 400-line `file_length` limit once the wiki-link/checkbox/drag-drop overrides landed, so it was split into its own file (`EditorCoordinator.swift`).

### File and Type Length Limits

SwiftLint enforces:
- `file_length`: Error at 400 lines — split large files (e.g., models + private helpers → separate extensions)
- `type_body_length`: Warning at 250 lines — move private helper methods to same-file extensions
- `function_body_length`: Warning at 50 lines — extract per-step helpers (US4: `PDFExport` exports via `loadTemplate` / `renderContent` / `paginate` steps)

**Pattern**: When a file exceeds length, extract helper methods to a same-file `extension` block:

```swift
// EditorCoordinator.swift (primary responsibilities)
@MainActor
final class EditorCoordinator: NSObject {
    // Main coordinator responsibilities
}

// EditorCoordinator+Private.swift or EditorCoordinator+Helpers.swift
extension EditorCoordinator {
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
| Model classes | PascalCase + `Model` suffix | `WorkspaceModel.swift`, `TabModel.swift`, `InfoModel.swift`, `TypographyModel.swift` |
| Coordinators (AppKit integration) | PascalCase + `Coordinator` suffix | `EditorCoordinator.swift`, `WorkspaceOutlineView+Coordinator.swift` |
| Sink bridges (FFI callbacks) | PascalCase + `Sink` suffix | `QuickOpenSink.swift`, `FsObserver.swift`, `AiStreamAdapter.swift` |
| Link/task helpers (US5) | PascalCase + descriptive | `WikiLink.swift`, `TaskCheckbox.swift`, `ImageDrop.swift` |
| Security/storage | PascalCase | `KeychainStore.swift`, `SecurityScopedBookmarks.swift` |
| Settings resolvers (US7) | PascalCase | `Typography.swift` |
| Export/utility enums | PascalCase | `PDFExport.swift` |
| Info pane models | PascalCase + `Model` suffix | `InfoModel.swift` |
| Settings UI | PascalCase + `View` or descriptive suffix | `TypographySettingsView.swift` |
| Test files | `Test.swift` or `Tests.swift` suffix | `BookmarkResolutionTests.swift`, `LinkHelpersTests.swift`, `KeychainStoreTests.swift`, `TypographyTests.swift` |

#### Code Element Naming (Swift)

| Type | Convention | Example |
|------|------------|---------|
| Variables | camelCase | `selectedLocation`, `isVisible`, `bookmarkData` |
| Constants (static) | camelCase (or SCREAMING_SNAKE_CASE for compile-time constants) | `defaultFolderSize` |
| Type names (struct/class/enum) | PascalCase | `MainWindow`, `AiStreamAdapter`, `WikiLink`, `KeychainStore`, `TypographySettings` |
| Functions/methods | camelCase, verb-prefix for state change | `addLocation()`, `openDocument()`, `onToken(_:)`, `save(_:account:)`, `apply(_:)` |
| Properties | camelCase | `locations`, `selection`, `abiVersion`, `apiKey`, `settings` |
| Boolean properties | `is`/`has` prefix when non-obvious | `isVisible`, `hasError`, `hasKey` |

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
final class TypographyModel: ObservableObject {
    @Published private(set) var settings: TypographySettings
    let handle: SettingsHandle
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

**US6 example** (`AiStreamAdapter` bridges AI streaming responses):
```swift
/// Bridges the core's AiSink callbacks (on a tokio worker in emend-ffi)
/// to AsyncThrowingStream. Holds immutable closures; safe for crossing thread
/// boundaries (mirrors QuickOpenSink).
public final class AiStreamAdapter: AiSink, Sendable {
    private let onTerminate: @Sendable () -> Void
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

**Key pattern**:
1. Sink class is `final` and `Sendable`
2. Holds only `@Sendable` closures (immutable, can cross threads)
3. Callback is invoked directly (on the calling thread)
4. Caller wraps the closure with `Task { @MainActor in … }` to hop to main thread

This avoids `@MainActor` annotation on the sink itself (which would require the calls *from* Rust to be on the main thread — they're not), while still ensuring SwiftUI mutations happen on main.

### Selector-Based NotificationCenter Observers (US4)

When observing AppKit notifications in a `@MainActor` context, use **selector-based observers** (via `#selector`) to avoid `@Sendable` closure conflicts. The `@MainActor` coordinator's `@objc` methods receive notifications directly on the main thread:

```swift
@MainActor
final class EditorCoordinator: NSObject, NSTextStorageDelegate {
    // ...
    
    /// Observe the scroll view's clip bounds so editor scrolls drive the preview (US4).
    /// The clip view posts on the main thread during scroll.
    func observeScrolling(in scrollView: NSScrollView) {
        let clip = scrollView.contentView
        clip.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(viewportDidScroll),
            name: NSView.boundsDidChangeNotification,
            object: clip
        )
    }
    
    /// Clip bounds moved (a scroll): forward the editor's top line to the preview.
    /// Posted on the main thread during scrolling.
    @objc private func viewportDidScroll() {
        guard let textView else { return }
        scrollSync?.editorScrolled(from: textView)
    }
}
```

**Cleanup in deinit**:
```swift
deinit {
    NotificationCenter.default.removeObserver(self)
}
```

**Rationale**: Selector-based observers automatically post on the main thread for main-thread-posting notifications (e.g., `NSView.boundsDidChangeNotification`), avoiding the need for `Task { @MainActor in … }` wrappers that complicate `@Sendable` closure constraints.

## Accessibility Identifiers (Phase 10)

Even though the app has no XCUITest by design (Constitution VII), **accessibility identifiers** are set for VoiceOver support and to enable future UI-test lanes if needed:

```swift
// Editor view
editor.textView.accessibilityIdentifier = "editor.textView"

// Sidebar
outlineView.accessibilityIdentifier = "sidebar.outline"

// Quick Open palette
searchField.accessibilityIdentifier = "quickOpen.searchField"
results.accessibilityIdentifier = "quickOpen.results"
results.accessibilityIdentifier = "quickOpen.result.\(hit.name)"  // Per-result
```

These cost nothing, aid VoiceOver navigation, and document the UI structure without imposing XCUITest maintenance burden.

## Typography Settings Patterns (US7)

### Clamping & Validation in Core

The `TypographySettings` struct in the core (`crates/emend-core/src/settings.rs`) enforces **clamping at the boundary** — hostile or buggy values from the UI never produce a broken layout:

```rust
/// Editor/preview typography preferences (US7 · FR-038/FR-039).
/// Clamping rules applied in `clamped()`:
/// - font_size_pt: 8..=48 (readable bounds)
/// - line_height: 1.0..=3.0 (single spacing to very loose)
/// - paragraph_spacing_pt: 0..=64 (non-negative, max 64 pt)
/// - font_family: empty → default ("SF Mono" on macOS)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TypographySettings {
    pub font_family: String,
    pub font_size_pt: f32,
    pub line_height: f32,
    pub paragraph_spacing_pt: f32,
}

impl TypographySettings {
    /// Apply clamping rules; idempotent.
    pub fn clamped(self) -> Self {
        // Clamp each field into sane bounds; NaN → default for that field
    }
}

/// The in-memory store (app-managed global state per US2 guardrail).
pub struct TypographyStore { /* ... */ }

impl TypographyStore {
    pub fn new() -> Self { /* sane defaults */ }
    pub fn get(&self) -> TypographySettings { /* read current */ }
    pub fn set(&self, settings: TypographySettings) {
        // Clamp on set; SwiftUI reads the clamped result
    }
}
```

**Rationale** (Constitution V — validation in core, UI trusts the result):
- The core clamps hostile values (size 0, size 9999, negative spacing, NaN)
- The UI calls `apply()`, which passes through the core's `set()`, then reads back the clamped result
- SwiftUI publishes the clamped result, so the layout is always valid
- Tests verify NaN/infinity/out-of-range values are repaired (T123)

### Two-Applier Pattern: Editor + Preview

One settings source (`TypographyModel`), two appliers (`Typography` resolver + `PreviewCSS`):

```swift
/// Resolves core `TypographySettings` into the editor's AppKit attributes
/// and the preview's CSS (US7). One settings source, two appliers.
enum Typography {
    static func font(for settings: TypographySettings) -> NSFont {
        let size = CGFloat(settings.fontSizePt)
        if settings.fontFamily == systemSentinel || settings.fontFamily.isEmpty {
            return .systemFont(ofSize: size)
        }
        return NSFont(name: settings.fontFamily, size: size) ?? .systemFont(ofSize: size)
    }
    
    static func paragraphStyle(for settings: TypographySettings) -> NSParagraphStyle {
        let style = NSMutableParagraphStyle()
        style.lineHeightMultiple = CGFloat(settings.lineHeight)
        style.paragraphSpacing = CGFloat(settings.paragraphSpacingPt)
        return style
    }
    
    static func previewCSS(for settings: TypographySettings) -> String {
        let family = if settings.fontFamily == systemSentinel || settings.fontFamily.isEmpty {
            "-apple-system, BlinkMacSystemFont, system-ui, sans-serif"
        } else {
            "\"\(settings.fontFamily)\", -apple-system, sans-serif"
        }
        return """
        .markdown-body { \
        font-family: \(family); \
        font-size: \(settings.fontSizePt)px; \
        line-height: \(settings.lineHeight); }
        .markdown-body p { margin-bottom: \(settings.paragraphSpacingPt)px; }
        """
    }
}
```

**Key guarantee**: Both appliers receive the *same clamped* settings, so visual sync is guaranteed (no edge-case divergence between editor font size and preview font size).

### Swift-Side Persistence via UserDefaults

Follows the **app-state pattern** established in US2 (core has no persistence layer; Swift-side owns the storage and replays on launch):

```swift
@MainActor
final class TypographyModel: ObservableObject {
    @Published private(set) var settings: TypographySettings
    
    static let defaultSettings = TypographySettings(
        fontFamily: "-apple-system",
        fontSizePt: 14,
        lineHeight: 1.4,
        paragraphSpacingPt: 8
    )
    
    private let handle: SettingsHandle
    private let defaults: UserDefaults
    
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        handle = newSettings()
        // Replay persisted settings from UserDefaults into the core on launch
        if let stored = Self.load(from: defaults, key: key) {
            try? handle.setTypography(settings: stored)
        }
        settings = handle.getTypography()
    }
    
    /// Apply new settings: the core clamps, we read the clamped result back,
    /// publish it (so the editor/preview update), and persist.
    func apply(_ new: TypographySettings) {
        try? handle.setTypography(settings: new)
        let clamped = handle.getTypography()
        settings = clamped
        Self.save(clamped, to: defaults, key: key)
    }
}
```

**Pattern** (US2 guardrail, applied consistently):
1. Core holds settings in memory with no disk I/O (`TypographyStore`)
2. Swift layer owns persistence (UserDefaults key)
3. On launch: load from UserDefaults, pass to core via `setTypography()`, read back clamped value
4. On update: core clamps, UI reads the result, publishes to SwiftUI, persists to UserDefaults

### uniffi Record Equatable for SwiftUI Change-Guards

The `TypographySettings` struct is a `uniffi::Record` and derives `Equatable` (generated by UniFFI), enabling SwiftUI to detect changes:

```swift
// In SwiftUI view:
@ObservedObject var model: TypographyModel

var body: some View {
    if oldSettings != model.settings {  // Equatable comparison
        updateEditorAndPreviewFont()  // Refresh applies live
    }
}
```

**Rationale**: `uniffi::Record` auto-derives `Equatable` across the FFI boundary, so SwiftUI `@Published` changes and view re-renders are automatic and correct.

## PDF Export Patterns (US4)

### Async PrintOperation Pattern

PDF export uses `NSPrintOperation.runModal(for:…)` (async variant) instead of synchronous `run()` to avoid deadlocking the main run loop. The async variant allows WebKit's print IPC to complete:

```swift
/// DO NOT use synchronous run(): it blocks the main thread while WebKit's
/// print path needs the main run loop for IPC. Use runModal(for:…) instead.
@MainActor
private func paginate(_ webView: WKWebView, with printInfo: NSPrintInfo) async throws {
    guard let window else { throw PDFExport.Failure.printFailed }
    let operation = webView.printOperation(with: printInfo)
    operation.showsPrintPanel = false
    operation.showsProgressPanel = false
    
    let ok = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Bool, Error>) in
        printContinuation = continuation
        // Watchdog: ensure export never hangs
        DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
            guard let self, let pending = printContinuation else { return }
            printContinuation = nil
            pending.resume(throwing: PDFExport.Failure.printFailed)
        }
        operation.runModal(
            for: window,
            delegate: self,
            didRun: #selector(printOperationDidRun(_:success:contextInfo:)),
            contextInfo: nil
        )
    }
}

/// Called by runModal when the save completes.
@objc private func printOperationDidRun(
    _: NSPrintOperation,
    success: Bool,
    contextInfo _: UnsafeMutableRawPointer?
) {
    printContinuation?.resume(returning: success)
    printContinuation = nil
}
```

**Key points**:
- `runModal(for:…)` is async-friendly; it runs without blocking the main thread
- Selector-based `didRun:` callback receives the result and resumes the continuation
- Watchdog timeout (30s) guards against stalled WebKit processes
- Both `loadTemplate` and `paginate` steps use `withCheckedThrowingContinuation` to bridge callback-based APIs to async/await

## AI Client Patterns (US6)

### Redacting API Key Pattern

The `ApiKey` newtype (in `crates/emend-core/src/ai.rs`) implements **secret hygiene** (NFR-006). Its `Debug` and `Display` implementations render `***`, never the secret:

```rust
/// A redacting wrapper around the AI API key (NFR-006).
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a raw key string. Only readable via expose().
    pub fn new(key: String) -> Self { Self(key) }
    
    /// The ONLY way to read the secret — explicit naming prevents accidental leaks.
    pub fn expose(&self) -> &str { &self.0 }
    
    /// Whether the key is blank (empty/whitespace).
    pub fn is_blank(&self) -> bool { self.0.trim().is_empty() }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(***)")  // Never prints the secret
    }
}

impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")  // Never prints the secret
    }
}
```

**Swift usage** (`KeychainStore`, US6):
```swift
/// Secure storage for AI API keys via macOS Keychain (SC-008, NFR-006).
/// Keys are never retained in Swift memory; they cross the FFI boundary
/// transient to the Rust core for one request, then dropped.
final class KeychainStore {
    static func save(_ key: String, account: String) throws { /* ... */ }
    static func read(account: String) -> String? { /* ... */ }
    static func hasKey(account: String) -> Bool { /* ... */ }
    static func delete(account: String) -> Bool { /* ... */ }
}
```

**Key guarantees**:
- The secret is wrapped at the Rust boundary and never logged/displayed
- Swift retrieves from Keychain only when needed, keeps it transient
- No copies in temp variables or default parameters

### Pure SSE Parser Pattern

The `SseParser` in `crates/emend-core/src/ai.rs` is a **pure, tokio-free** line-buffering parser for OpenAI-style Server-Sent-Events (T109, US6):

```rust
/// Parses OpenAI Chat-Completions SSE chunks (incremental, tokio-free).
/// - Buffers raw bytes across chunks
/// - Emits complete SseEvent::Token(delta) for each data: line
/// - Tolerates CRLF and LF line endings
/// - Skips comments/heartbeats (: prefix)
/// - Special-cases data: [DONE] → SseEvent::Done
/// - Treats closed connection as clean end
pub struct SseParser { /* ... */ }

impl SseParser {
    pub fn new() -> Self { /* ... */ }
    pub fn push_bytes(&mut self, chunk: &[u8]) -> impl Iterator<Item = SseEvent> { /* ... */ }
    pub fn finish(mut self) -> impl Iterator<Item = SseEvent> { /* ... */ }
}

pub enum SseEvent {
    Token(String),  // One complete incremental delta
    Done,           // data: [DONE] received
}
```

**Rationale** (Constitution V — decision logic in core):
- The parser is pure (no tokio, no reqwest, testable with `cargo test`)
- FFI layer (`emend_ffi/src/ai.rs`) handles tokio spawning and request orchestration
- The core's token emission logic is correct in isolation; the FFI layer inherits correctness

### Pre-Send Input Size Guard

The `check_input_size` function (in `crates/emend-core/src/ai.rs`) enforces **pre-send rejection** of oversized inputs (FR-036a, US6):

```rust
/// Reject an over-limit AI input **before any network call** (FR-036a).
/// Returns AiOversizedInput if input.len() > max_bytes (boundary inclusive).
/// Measures UTF-8 bytes (what crosses the wire), not chars.
pub fn check_input_size(input: &str, max_bytes: u64) -> Result<(), EmendError> { /* ... */ }
```

**Key property**: The FFI layer calls this **before constructing a request**. Network gating is structural — `emend-core` has no `reqwest` dependency (CI proves `cargo tree -p emend-core -i reqwest` finds nothing).

## Link & Task Patterns (US5)

### Pure Wiki-Link & Task Helpers

Link resolution, task toggles, and image drops are **pure functions** over `(text: NSString, range/selection: NSRange) → range/edit?`:

```swift
// Deterministic wiki-link resolution (T095, FR-019a)
enum WikiLink {
    struct Hit {
        let raw: String  // The target name from [[…]]
        let range: NSRange  // The [[…]] span for click hit-testing
        let span: NSRange  // Full [[…]] including brackets
    }
    
    /// Return the open [[word if editing inside one, e.g., "[[Foo" at caret.
    static func partialRange(in text: NSString, caret: Int) -> NSRange? { … }
    
    /// Return the [[Target]] at a click position, or nil.
    static func enclosingLink(in text: NSString, at position: Int) -> Hit? { … }
    
    /// All [[…]] and ![[…]] links in the text.
    static func allLinks(in text: NSString) -> [Hit] { … }
}

// Task checkbox toggle (T095, FR-014)
enum TaskCheckbox {
    struct Edit: Equatable {
        let range: NSRange
        let replacement: String
    }
    
    /// Detect a checkbox ([ ] or [x]) on the line containing offset.
    static func checkboxRange(in text: NSString, atLineContaining offset: Int) -> NSRange? { … }
    
    /// Return the edit to flip [ ] → [x] or vice versa.
    static func toggleEdit(in text: NSString, atLineContaining offset: Int) -> Edit? { … }
}
```

These **headless transforms are unit-tested without a window** (per Constitution VII). The editor applies returned ranges/edits through normal text-delegate and autocomplete paths. See `LinkHelpersTests` for exhaustive coverage; end-to-end resolution + embedding tested in `LinksFlowTests`.

### Checkbox Toggle is a Swift Edit, Not Core Mutation

Checkbox toggling flips the bracket `[ ]` ↔ `[x]` via the **Swift Edit path** (same as `SmartLists`/`FormattingCommands`), NOT the core `toggle_task` FFI. Reason: Swift owns the `NSTextStorage` buffer; a core-side mutation would desync the display.

```swift
// In editor coordinator, respond to a click inside a checkbox:
let edit = TaskCheckbox.toggleEdit(in: text as NSString, atLineContaining: offset)
if let e = edit {
    editor.apply(edit: e)  // Applies via shouldChangeText/didChangeText path; registers undo
}
```

The core `toggle_task` FFI is left available for non-editor surfaces (e.g., an info pane that previews tasks).

### Wiki-Link Autocomplete Inserts the Stem

NSTextView's native completion (via `rangeForUserCompletion` + `completions(forPartialWordRange:…)`) is wired to fetch wiki-link targets. **Autocomplete must insert the stem** (`name.deletingPathExtension`), not the full file name:

**Problem**: `SearchHit.name` carries the file extension (`Beta.md`), but wiki-link resolution matches on the stem (`Beta`). Inserting `[[Beta.md]]` fails to resolve.

**Solution** (US5, caught by flow test): Strip the extension before insertion:

```swift
// In MarkdownEditorView's completion handler
let hit = suggestion  // a SearchHit from the index
let target = (hit.name as NSString).deletingPathExtension  // "Beta"
let completion = target  // Insert as [[Beta]]
```

See `LinksFlowTests.testCoordinatorLinkServices` for the full end-to-end verification.

### Embed Resolution with Cycle & Depth Guards

`![[embed]]` inlines another note's rendered content. The core expander (`expand_embeds` in `crates/emend-core/src/parse/embed.rs`) **owns the cycle + depth logic**:

```rust
/// Expand ![[…]] embeds in Markdown text. The resolver is called for each
/// target and returns the note's source text (or None if unresolved).
/// Cycles and deep chains terminate gracefully via MAX_EMBED_DEPTH (8).
pub fn expand_embeds(
    text: &str,
    opts: &EmbedOptions,
    resolver: &mut dyn FnMut(&str) -> Option<String>,
) -> String { … }
```

The FFI bridges this to Swift via a resolver closure that snapshots the document under the lock, releases it, then resolves each target via the workspace index (lock NOT held during on-disk reads). See `embeds.rs` integration tests (T096) for cycle/depth correctness.

### Attachment Storage is Collision-Safe

`store_attachment` writes binary data to a collision-safe location (using the same `note 2.md` suffix scheme as file ops). It returns a relative ref for markdown embedding:

```swift
// In editor, when a user drags an image:
let ref = try storeAttachment(
    notePath: currentNote.path,
    bytes: imageData,
    suggestedName: "screenshot.png"
)
// ref is e.g., "attachments/screenshot.png" (relative to note folder)

// Insert into the markdown:
let mdText = ImageDrop.markdown(forImageRef: ref)  // "![](attachments/screenshot.png)"
let edit = SmartLists.Edit(range: ..., replacement: mdText, selectionAfter: ...)
```

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

**Rationale** (Constitution V — decision logic in core, tested in core):
- Ranking happens once, synchronously
- Batching logic is deterministic (no async, no timing-dependent decisions)
- Cancellation is a simple `&Cancel` flag, not tokio-dependent
- FFI layer (`emend_ffi/src/search.rs`) bridges the `Cancel` to `CancellationToken` and handles panic containment, but delegates ranking/emission to this pure driver

### Rust: Deterministic Wiki-Link Resolution (US5)

The core `resolve_wikilink` function (`crates/emend-core/src/derived.rs`) implements **deterministic** resolution for duplicate basenames (FR-019a):

```rust
/// Resolve a wiki-link target (the stem, e.g., "Beta" from [[Beta]]) to an
/// absolute path. Returns None if no match found.
/// When multiple notes have the same stem (T095 edge case), the tie-break is:
/// 1. a candidate in the **same directory as source_note** wins;
/// 2. else the **shallowest** path (fewest separators) wins;
/// 3. else the **lexicographically smallest** path string wins.
/// Order is total and reproducible across runs (no HashMap-iteration leak).
pub fn resolve_wikilink(
    index: &Index,
    source_note: &str,  // absolute path
    raw_target: &str,   // stem, e.g., "Beta"
) -> Option<String> { … }
```

The order is **total and deterministic** — see T095 in `crates/emend-core/tests/links.rs` for exhaustive tests of the tie-break logic.

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

Editor behavior transforms (e.g., `SmartLists`, `FormattingCommands`, `WikiLink`, `TaskCheckbox`) are pure functions over `(text: String, selection: NSRange) -> Edit?`:

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

**Scope** (optional): Lowercase, hyphenated, e.g., `(editor)`, `(ffi-boundary)`, `(swift)`, `(search)`, `(preview)`, `(links)` (US5), `(ai)` (US6), `(typography)` (US7).

**Breaking change** (optional): Suffix `!` before `:` (e.g., `feat(ffi)!: new ABI version`).

**Examples**:
```
feat(document): add line-column tracking
fix(fs): tolerate CRLF in file reads
docs: update UTF-16 boundary documentation
test(document): add astral-char UTF-16 tests
feat(search): add cancellable quick-open query (US3)
feat(preview): add PDF export via NSPrintOperation (US4)
feat(links): add wiki-link resolution + embed inlining (US5)
feat(ai): implement BYOM AI client with SSE streaming (US6)
feat(typography): add editor/preview font & spacing settings (US7)
test(quality): add large-file + preview-offline verify tests (Phase 10)
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
| swift-lint | `*.swift` | `swiftlint lint --quiet --strict` | Rejects if --strict violations exist |
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
- **`crates/emend-core/src/preview.rs`**: Live Markdown preview (comrak HTML + syntect code highlighting)
- **`crates/emend-core/src/derived.rs`**: Link extraction, resolution, task detection, and doc stats (US5/US6)
- **`crates/emend-core/src/parse/embed.rs`**: Embed expander with cycle + depth guards (US5)
- **`crates/emend-core/src/ai.rs`**: Pure AI client: SSE parser, secret hygiene, max-input guard (US6)
- **`crates/emend-core/src/settings.rs`**: Typography settings store + clamping validation (US7)
- **`crates/emend-core/tests/`**: Integration tests (see [Testing](#testing))
- **`crates/emend-ffi/src/lib.rs`**: UniFFI `#[uniffi::export]` shim + panic containment
- **`crates/emend-ffi/src/search.rs`**: FFI projection of streaming search (bridges cancellation, spawns worker, panic containment)
- **`crates/emend-ffi/src/ai.rs`**: FFI projection of AI client (tokio orchestration, reqwest dispatch, token bridging)
- **`crates/emend-bench/benches/`**: Criterion micro-benchmarks

### Swift Module Structure

- **`swift/EmendCore/Sources/EmendCore/EmendCore.swift`**: Clean public API, re-exports `EmendCoreFFI`
- **`swift/EmendCore/Sources/EmendCore/Streaming.swift`**: AsyncStream adapters over foreign-trait callbacks
- **`swift/EmendCore/Sources/EmendCoreFFI/`** (generated): UniFFI bindings (excluded from lint)
- **`app/Emend/Emend/`**: SwiftUI app (views, state, utilities, pure transforms)
  - **`Sidebar/`**: Workspace tree model, `NSOutlineView` coordination, drag-drop logic
  - **`Tabs/`**: Tab management, open-document state
  - **`Editor/`**: Editor view, syntax highlighting, text storage delegates, pure transforms (`SmartLists`, `FormattingCommands`, `WikiLink`, `TaskCheckbox`, `ImageDrop`)
  - **`Preview/`**: Live preview model, off-screen WebView render, PDF export (`PDFExport.swift`, US4)
  - **`QuickOpen/`**: Quick Open palette model + sink bridge (US3)
  - **`Info/`**: Info sidebar model + view (US6) — displays stats, outline, AI summary
  - **`Settings/`**: Typography settings model, view, and resolver (US7)
  - **`Platform/`**: macOS-specific utilities (`KeychainStore.swift`, `SecurityScopedBookmarks.swift`, etc.)
  - **`EditorCoordinator.swift`**: Coordinator for `NSTextView` / editor text handling (US5 split to own file for file_length)
- **`app/Emend/EmendTests/`**: App-level XCTest tests (headless, no GUI automation)

### Editor Transform Organization

Pure, testable transforms are organized by feature:
- **`app/Emend/Emend/Editor/SmartLists.swift`** — List continuation + termination logic (T045)
- **`app/Emend/Emend/Editor/FormattingCommands.swift`** — Bold, italic, code formatting (T046)
- **`app/Emend/Emend/Editor/SyntaxAttributing.swift`** — Highlight synthesis from tree-sitter (T047)
- **`app/Emend/Emend/Editor/WikiLink.swift`** — Wiki-link extraction + resolution (US5 · T095)
- **`app/Emend/Emend/Editor/TaskCheckbox.swift`** — Task checkbox toggle (US5 · T095)
- **`app/Emend/Emend/Editor/ImageDrop.swift`** — Drag-drop image attachment (US5 · T102)

Each transform is pure and unit-tested headlessly in `app/Emend/EmendTests/`.

### Quick Open Organization (US3)

- **`crates/emend-core/src/index.rs`**: `Index` type and ranking logic (fuzzy match, basename boost, wiki-link O(1) lookup)
- **`crates/emend-core/src/search.rs`**: Pure `quick_open` driver (ranks, batches, cancels)
- **`crates/emend-ffi/src/search.rs`**: FFI projection (`SearchHandle`, async worker, panic containment)
- **`app/Emend/Emend/QuickOpen/QuickOpenModel.swift`**: SwiftUI state, sink attachment, generation-guarded batch apply
- **`app/Emend/Emend/QuickOpen/QuickOpenSink.swift`**: Sink bridge (holds `@Sendable` closures, hops to `@MainActor`)
- **`crates/emend-core/tests/search_supersede.rs`**: Supersede/cancel semantics (T072, pure tokio-free determinism)
- **`crates/emend-bench/benches/quick_open.rs`**: Perf budget 100 ms p95 warm over 10k index (T071, SC-004)

### Links, Embeds & Attachments Organization (US5)

- **`crates/emend-core/src/derived.rs`**: Link extraction, deterministic resolution, task detection (T095)
- **`crates/emend-core/src/parse/embed.rs`**: Embed expander with cycle + depth guards (T096, MAX_EMBED_DEPTH=8)
- **`app/Emend/Emend/Editor/WikiLink.swift`**: Pure wiki-link range/hit detection (T095)
- **`app/Emend/Emend/Editor/TaskCheckbox.swift`**: Pure checkbox toggle transforms (T095)
- **`app/Emend/Emend/Editor/ImageDrop.swift`**: Attachment storage + markdown refs (T102)
- **`app/Emend/EmendTests/LinkHelpersTests.swift`**: Headless unit tests for pure transforms (T103)
- **`app/Emend/EmendTests/LinksFlowTests.swift`**: End-to-end resolution + embedding + attachment storage (T104)
- **`crates/emend-core/tests/links.rs`**: Core resolution + extraction determinism (T095)
- **`crates/emend-core/tests/embeds.rs`**: Core embed expansion + cycle/depth guards (T096)

### AI Client Organization (US6)

- **`crates/emend-core/src/ai.rs`**: Pure SSE parser, secret hygiene (ApiKey redacting), max-input guard, request builders
- **`crates/emend-core/src/derived.rs`**: Doc stats (word/char count, reading time, task N-of-M), outline with line numbers
- **`crates/emend-ffi/src/ai.rs`**: FFI projection of AI client (tokio orchestration, reqwest dispatch, token bridging)
- **`app/Emend/Emend/Platform/KeychainStore.swift`**: macOS Keychain wrapper for API key storage (SC-008, NFR-006)
- **`app/Emend/Emend/Info/InfoModel.swift`**: SwiftUI state for sidebar info pane (stats, outline, AI summary)
- **`app/Emend/Emend/Info/InfoSidebarView.swift`**: View for stats, outline, AI summary
- **`swift/EmendCore/Sources/EmendCore/Streaming.swift`**: `AiStreamAdapter` foreign-trait bridge to AsyncThrowingStream (US6)
- **`crates/emend-core/tests/derived_stats.rs`**: Doc stats + outline correctness (T108, US6)
- **`crates/emend-core/tests/ai_sse.rs`**: SSE parser edge cases (T109, US6)
- **`crates/emend-core/tests/ai_privacy.rs`**: Secret hygiene + max-input guard (T110, US6)
- **`app/Emend/EmendTests/KeychainStoreTests.swift`**: Keychain round-trip (T119, US6)
- **`app/Emend/EmendTests/InfoSidebarTests.swift`**: Info sidebar stats/outline population (T119, US6)

### Typography Settings Organization (US7)

- **`crates/emend-core/src/settings.rs`**: `TypographySettings` struct, clamping rules, `TypographyStore` in-memory hold (T123)
- **`crates/emend-ffi/src/lib.rs`**: FFI `#[uniffi::export]` for `SettingsHandle` + getter/setter
- **`app/Emend/Emend/Settings/TypographyModel.swift`**: SwiftUI state model, UserDefaults persistence, replay on launch (US2 pattern)
- **`app/Emend/Emend/Settings/Typography.swift`**: Resolver for editor font + preview CSS (two appliers, one source)
- **`app/Emend/Emend/Settings/TypographySettingsView.swift`**: UI for font family, size, line height, paragraph spacing
- **`app/Emend/EmendTests/TypographyTests.swift`**: Model clamp + persistence, resolver font/CSS synthesis (T127)

### Preview & Export Organization (US4)

- **`crates/emend-core/src/preview.rs`**: Core `renderPreviewHtml` (comrak + syntect)
- **`app/Emend/Emend/Preview/PreviewModel.swift`**: Live preview model (debounced off-main rendering)
- **`app/Emend/Emend/Preview/PreviewWebView.swift`**: On-screen live preview rendering + scroll-sync
- **`app/Emend/Emend/Preview/PDFExport.swift`**: Off-screen PDF export (`OffscreenPrintHost`, async printOp)
- **`app/Emend/EmendTests/PreviewExportTests.swift`**: PDF export tests (multi-page pagination verification)

### Quality & Verification Organization (Phase 10)

- **`crates/emend-core/tests/large_file.rs`**: Max-note-size cap + incremental re-parse on ~1 MiB doc (T133, FR-027a)
- **`crates/emend-core/tests/preview_offline.rs`**: Preview rendering stays offline (no network access) (T083, SC-008)
- **`crates/emend-bench/benches/highlight.rs`**: Per-keystroke incremental reparse on large doc (SC-003, T037)
- **`crates/emend-bench/benches/quick_open.rs`**: Fuzzy search 10k-entry index performance (SC-004, T071)
- **`crates/emend-bench/benches/open_doc.rs`**: Core open + initial parse (SC-002, T030)

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
