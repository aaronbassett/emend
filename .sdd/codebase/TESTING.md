# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Overview

Emend follows a **test-first, strict-core** strategy (Constitution VII):
- **Rust core** (`emend-core`): Comprehensive unit/integration tests, run in isolation via `cargo test` (no FFI toolchain required)
- **Swift core package** (`swift/EmendCore`): ABI version smoke tests and AsyncStream adapter tests
- **Swift app** (`app/Emend`): Smoke/linkage tests plus critical-path integration tests; headless XCTest (no GUI automation)
- **Benchmarks**: Non-blocking perf tracking via Criterion (compile-checked in CI)

## Test Framework

| Layer | Framework | Configuration | Command |
|-------|-----------|---------------|---------|
| Rust core (unit/integration) | cargo test (built-in) | None (standard Rust) | `cargo test` |
| Rust benchmarks | Criterion 0.7.x | `crates/emend-bench/Cargo.toml` | `cargo bench` or `cargo bench --no-run` (CI) |
| Swift core package | XCTest | `swift/EmendCore/Tests/` | `swift test` |
| Swift app | XCTest (app-hosted) | `app/Emend/EmendTests/` (headless) | `xcodebuild test -project app/Emend/Emend.xcodeproj -scheme Emend -destination 'platform=macOS,arch=arm64' CODE_SIGNING_ALLOWED=NO` |

### Running Tests

| Command | Purpose | Environment |
|---------|---------|-------------|
| `cargo test` | All Rust unit + integration tests (core, FFI, benchmarks compile-check) | macOS (arm64) or Linux |
| `cargo test --lib` | Rust unit tests only (no integration tests) | macOS or Linux |
| `cargo test -p emend-core` | Tests in `emend-core` crate only | macOS or Linux |
| `cargo test -p emend-ffi` | Tests in `emend-ffi` crate (panic containment, FFI boundary) | macOS or Linux |
| `swift test` | Swift package tests in `swift/EmendCore/` | macOS with Xcode |
| `just app-test` | Full app test (builds XCFramework + Xcode project + runs app tests headless) | macOS 14+ with Xcode 16.2, no signing |
| `just check` | Pre-push gate: fmt + clippy + test + swift-lint (mirrors CI) | macOS with Swift tools |

## Test Organization

### Rust Tests

#### Location Strategy

Tests are **co-located with source code** in two forms:

1. **Unit tests** (inline in source files):
   - Placed in a `#[cfg(test)] mod tests` submodule at the end of each `.rs` file
   - Test the public API of that module in isolation
   - Example: `crates/emend-core/src/lib.rs` contains `#[test] fn u16range_end_is_start_plus_len()`

2. **Integration tests** (separate directory):
   - Located in `crates/{crate-name}/tests/`
   - Test cross-module behavior, require fixtures, exercise the public API as an external consumer would
   - Allowed to use `unwrap`/`expect`/`panic` (scoped exception from workspace lints) because they own their fixtures
   - Examples:
     - `crates/emend-core/tests/document.rs` — UTF-16 boundary correctness (T018)
     - `crates/emend-core/tests/fs_atomic.rs` — File I/O atomicity (T011)
     - `crates/emend-core/tests/parse_incremental.rs` — Incremental parsing + highlight (T021)
     - `crates/emend-core/tests/workspace_ops.rs` — Collision-safe file operations (T054, US2)
     - `crates/emend-core/tests/watcher.rs` — Filesystem watching + conflict resolution (T057, US2)
     - `crates/emend-core/tests/index.rs` — Incremental search index (T073, US2)
     - `crates/emend-core/tests/search_supersede.rs` — Cancellable search driver (T072, US3)
     - `crates/emend-core/tests/path_identity.rs` — Path canonicalization, symlink handling (NFR-007, US2)
     - `crates/emend-core/tests/concurrency.rs` — Workspace concurrent access (US2)
     - `crates/emend-ffi/tests/panic_containment.rs` — Panic capture across FFI (T015)

#### Test Files

| Path | Purpose | Scope |
|------|---------|-------|
| `crates/emend-core/tests/document.rs` | UTF-16↔char conversions, line indexing, edit splicing | Per-keystroke editor hot path correctness (research §A2/A3) |
| `crates/emend-core/tests/fs_atomic.rs` | Atomic+durable writes, tolerance to BOM/CRLF | File I/O reliability (FR-009a, research §B4) |
| `crates/emend-core/tests/parse_incremental.rs` | Tree-sitter incremental parsing, astral chars, edge cases | Highlight synthesis (T021, research §B1) |
| `crates/emend-core/tests/workspace_ops.rs` | Collision-safe create/rename/move, `note 2.md` suffix scheme | Workspace file ops (T054, FR-004/FR-004a/FR-013a, US2) |
| `crates/emend-core/tests/watcher.rs` | Debounced FSEvents, rename correlation, self-write suppression, conflict truth table | File watcher (T057/T065, FR-006a/FR-006b/FR-006c, US2) |
| `crates/emend-core/tests/index.rs` | Incremental search index updates, fuzzy ranking, wiki-link lookup | Quick Open + link resolution (T073, FR-017/FR-017a/FR-019a, US2) |
| `crates/emend-core/tests/search_supersede.rs` | Cancellation flag stops emission at batch boundary, pre-cancelled query emits nothing, un-superseded completes, ranking preserved | Quick Open supersede semantics (T072, FR-017/FR-018/SC-004, NFR-002, US3) |
| `crates/emend-core/tests/path_identity.rs` | Path canonicalization, symlink handling, bounded traversal | Path safety (NFR-007, US2) |
| `crates/emend-core/tests/concurrency.rs` | Workspace concurrent access, edit conflicts, multi-thread safety | Concurrent workspace ops (US2) |
| `crates/emend-ffi/tests/panic_containment.rs` | Panics routed through `contain_panic` surface as `EmendError::Internal` | FFI boundary safety (NFR-003, research §B7) |

#### Lint Exceptions in Tests

Integration tests allow `clippy::unwrap_used`, `clippy::expect_used`, and `clippy::panic` via scoped allow attributes at the file level:

```rust
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]
```

This is necessary because tests must assert on their results; a test that cannot unwrap a fixture cannot test. The workspace-level denial still applies to library code in `src/`.

### Swift Tests

#### Location Strategy

Swift tests follow Xcode conventions: separate `Tests/` directories within each target.

| Target | Test Location | Purpose |
|--------|---------------|---------|
| `EmendCore` package | `swift/EmendCore/Tests/EmendCoreTests/` | Unit tests for the clean API wrapper and streaming |
| `Emend` app | `app/Emend/EmendTests/` | Smoke + linkage tests, critical-path integration tests |

#### Test Files (Comprehensive, US4)

| Path | Purpose | Test Type |
|------|---------|-----------|
| `swift/EmendCore/Tests/EmendCoreTests/EmendCoreTests.swift` | Core ABI version smoke test | Smoke |
| `swift/EmendCore/Tests/EmendCoreTests/StreamingTests.swift` | AsyncStream adapter correctness (`AiStreamAdapter`, `SearchStreamAdapter`) | Unit |
| `app/Emend/EmendTests/EmendCoreLinkageTests.swift` | App links the local `EmendCore` package; core ABI is reachable | Smoke |
| `app/Emend/EmendTests/BookmarkResolutionTests.swift` | Security-scoped bookmark → file I/O flow | Unit integration |
| `app/Emend/EmendTests/SmartListsTests.swift` | Pure smart-list transforms: list continuation, empty-item termination, indentation preservation (T045, headless) | Unit |
| `app/Emend/EmendTests/FormattingCommandsTests.swift` | Formatting transforms: bold, italic, code (T046, headless) | Unit |
| `app/Emend/EmendTests/SyntaxAttributingTests.swift` | Syntax highlight attribute synthesis from tree-sitter (T047, headless) | Unit |
| `app/Emend/EmendTests/EditorPersistenceTests.swift` | End-to-end persistence: `EditorCoordinator` + `AutosaveController` + disk round-trip (T049, headless integration) | Integration |
| `app/Emend/EmendTests/WorkspaceFlowTests.swift` | End-to-end workspace: add folder → list tree → open tab → move/rename (T067, US2 Phase 4) | Integration |
| `app/Emend/EmendTests/QuickOpenTests.swift` | End-to-end Quick Open: seed index → query streams results → Return opens file (T078, US3) | Integration |
| `app/Emend/EmendTests/PreviewExportTests.swift` | PDF export: render long doc → paginate off-screen → verify multi-page PDF (T091, US4 · FR-026/SC-010) | Integration |

#### Test Pattern (XCTest)

```swift
import XCTest
@testable import Emend

final class MyTests: XCTestCase {
    func testSomething() {
        // Arrange
        let value = setupFixture()
        
        // Act
        let result = value.doSomething()
        
        // Assert
        XCTAssertEqual(result, expected)
    }
}
```

Tests are **headless** (no GUI launch) and run in the test bundle (`@testable` import). Classes annotated `@MainActor` or that touch AppKit state are marked with `@MainActor` for Swift 6 strict concurrency compliance.

#### @MainActor Annotation for Headless Tests

Tests that create or interact with `@MainActor` models (e.g., `WorkspaceModel`, `TabModel`, `QuickOpenModel`) must themselves be annotated `@MainActor`:

```swift
@MainActor
final class QuickOpenTests: XCTestCase {
    func testQueryStreamsRankedResultsAndOpensSelection() async throws {
        let dir = try seededWorkspace()
        defer { try? FileManager.default.removeItem(at: dir) }

        let workspace = newWorkspace()
        _ = try workspace.addLocation(folderPath: dir.path, bookmark: Data())
        let indexed = try workspace.reindexAll(maxDepth: 32)
        XCTAssertEqual(indexed, 3)

        var opened: URL?
        let model = QuickOpenModel()
        model.attach(workspace: workspace) { opened = $0 }

        model.query = "beta"
        model.runQuery()
        try await waitForResults(model)
        
        guard let index = model.results.firstIndex(where: { $0.name == "beta.md" }) else {
            return XCTFail("expected a beta.md result")
        }
        model.selection = index
        model.openSelected()
        XCTAssertEqual(opened?.lastPathComponent, "beta.md")
    }
}
```

**Rationale**: These models own `@Published` properties and must update on the main thread. XCTest automatically runs each test on the main thread if the test class is marked `@MainActor`.

#### Headless Integration Testing

Critical-path integration tests drive real Rust/Swift components without a GUI:

**Example: EditorPersistenceTests (T049)**
```swift
@MainActor
final class EditorPersistenceTests: XCTestCase {
    func testTypedTextFlushesToDiskAndRoundTrips() throws {
        // Create a temp file
        let directory = try makeTempDirectory()
        let url = directory.appendingPathComponent("note.md")
        try "".write(to: url, atomically: true, encoding: .utf8)
        let path = url.path(percentEncoded: false)
        
        // Open document in the real Rust core
        let handle = try openDocument(path: path)
        let editor = makeEditor(handle: handle, initialText: "")
        
        // Type text into NSTextView
        type("Persisted through the Rust core", into: editor.textView, at: 0)
        
        // Flush autosave and verify write
        editor.autosave.flushNow()
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), expected)
    }
}
```

This pattern exercises the full stack (Rust core → file I/O → disk → read-back) without launching the app GUI, making it feasible in CI with `CODE_SIGNING_ALLOWED=NO`.

**Example: QuickOpenTests (T078, US3)**
```swift
@MainActor
final class QuickOpenTests: XCTestCase {
    func testQueryStreamsRankedResultsAndOpensSelection() async throws {
        // Arrange: seed a temp workspace with three notes
        let dir = try seededWorkspace()
        defer { try? FileManager.default.removeItem(at: dir) }

        let workspace = newWorkspace()
        _ = try workspace.addLocation(folderPath: dir.path, bookmark: Data())
        // Synchronously seed the index so the query has a populated haystack
        let indexed = try workspace.reindexAll(maxDepth: 32)
        XCTAssertEqual(indexed, 3, "all three notes seeded")

        // Act: attach the real QuickOpenModel, query, and await results
        var opened: URL?
        let model = QuickOpenModel()
        model.attach(workspace: workspace) { opened = $0 }
        
        model.query = "beta"
        model.runQuery()
        try await waitForResults(model)  // Spin runloop until streamed batch lands
        
        // Assert: results contain the match, Return opens it, palette dismisses
        XCTAssertTrue(
            model.results.contains { $0.name == "beta.md" },
            "the matching note appears in results (FR-017)"
        )
        
        guard let index = model.results.firstIndex(where: { $0.name == "beta.md" }) else {
            return XCTFail("expected a beta.md result")
        }
        model.selection = index
        model.openSelected()
        XCTAssertEqual(opened?.lastPathComponent, "beta.md", "Return opens the selection (AC2)")
        XCTAssertFalse(model.isPresented, "opening dismisses the palette (AC3)")
    }

    /// Spin the main runloop until streamed results land or timeout.
    private func waitForResults(_ model: QuickOpenModel, timeout: TimeInterval = 3.0) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        while model.results.isEmpty, Date() < deadline {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }
}
```

This drives the real `WorkspaceHandle` + `QuickOpenModel` end-to-end, exercising the full streaming path from Rust search worker through the `QuickOpenSink` bridge to SwiftUI state updates.

**Example: PreviewExportTests (T091, US4)**
```swift
@MainActor
final class PreviewExportTests: XCTestCase {
    func testExportProducesMultiPagePDF() async throws {
        // Arrange: a document long enough to span several Letter/A4 pages
        let markdown = Self.longDocument(sections: 60)
        let source = try writeTempNote(markdown)
        defer { try? FileManager.default.removeItem(at: source) }

        let handle = try openDocument(path: source.path)
        defer { try? handle.close() }
        
        // Act: render the preview HTML via the core's comrak + syntect renderer
        let html = try handle.renderPreviewHtml()
        XCTAssertTrue(html.contains("Section 1"), "core rendered the document body")

        // Export off-screen via PDFExport (async NSPrintOperation.runModal)
        let output = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-export-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: output) }

        try await PDFExport.export(html: html, css: previewThemeCss(), to: output)

        // Assert: PDF exists and is multi-page (SC-010)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: output.path),
            "the PDF was written to disk"
        )
        let pdf = try XCTUnwrap(PDFDocument(url: output), "the output is a readable PDF")
        XCTAssertGreaterThan(
            pdf.pageCount, 1,
            "a long document paginates into multiple pages (SC-010), not one tall page"
        )
    }
}
```

This drives the real `PDFExport` off-screen render→print pipeline without launching the app, verifying multi-page pagination. The async `NSPrintOperation.runModal(for:…)` is tested with `withCheckedThrowingContinuation` to bridge the selector-based `didRun` callback to async/await.

## Test Patterns

### Rust: Arrange-Act-Assert

```rust
#[test]
fn u16range_end_is_start_plus_len() {
    // Arrange
    let r = U16Range::new(3, 4);
    
    // Act
    let end = r.end();
    
    // Assert
    assert_eq!(end, 7);
}
```

### Rust: UTF-16 Boundary Testing

The `document.rs` integration test (T018) exercises the two critical UTF-16↔char divergence points:

1. **Astral characters** (U+10000 and above):
   - One `char` but two UTF-16 code units (surrogate pair)
   - "😀" (U+1F600) is a canonical test case
   - Tests verify round-trips at boundaries and edits spanning astral chars

2. **Line breaks** (LF and CRLF):
   - Only LF (`\n`) and CRLF (`\r\n`) count as line breaks (not Unicode line separators)
   - CRLF is a *single* line break
   - Columns are UTF-16 code units within the line

Example (from `tests/document.rs`):
```rust
/// An astral char is two UTF-16 code units even though it is a single scalar:
/// "a😀b" is 3 chars but 4 UTF-16 code units (1 + 2 + 1).
#[test]
fn astral_utf16_len_differs_from_char_len() {
    let doc = Document::from_text("a😀b");
    assert_eq!(doc.len_chars(), 3);
    assert_eq!(doc.len_utf16(), 4);  // 1 + 2 + 1
}
```

### Rust: Cancellation & Supersede Testing (US3)

The `search_supersede.rs` integration test (T072) verifies that a **pure, tokio-free** search driver respects cancellation **deterministically** — no async runtime, no timing:

```rust
//! T072 — failing-first tests for the cancellable Quick Open search driver.
//! Asserts that setting a Cancel flag mid-stream stops emission at the next
//! batch boundary, and that an un-superseded query runs to completion.
//! Pure sync driver, no tokio, so behaviour is deterministic.

#[test]
fn setting_cancel_flag_mid_stream_stops_emission() {
    let index = seeded(100);  // 100 notes, all match "note"
    let cancel = Cancel::new();

    // Cancel from inside the emit callback once the first batch lands.
    let mut batches: Vec<Vec<SearchHit>> = Vec::new();
    let completed = quick_open(&index, "note", 100, 8, &cancel, |b| {
        batches.push(b);
        if batches.len() == 1 {
            cancel.cancel();  // Simulate supersede mid-stream
        }
    });

    assert!(!completed, "a superseded query reports incomplete");
    assert_eq!(batches.len(), 1, "emission stops at next batch boundary");
    let emitted: usize = batches.iter().map(Vec::len).sum();
    assert!(emitted < 100, "did not emit the full set");
}

#[test]
fn un_superseded_query_completes_and_streams_all() {
    let index = seeded(20);
    let cancel = Cancel::new();

    let mut sink = Sink::new();
    let completed = quick_open(&index, "note", 50, 8, &cancel, |b| sink.batches.push(b));

    assert!(completed, "an un-superseded query reports completion");
    assert_eq!(sink.total(), 20, "all 20 hits stream through");
    assert_eq!(sink.count(), 3, "batches: 8 + 8 + 4");
}
```

**Key pattern**: The test drives the pure `quick_open` function **synchronously** with a `Cancel` flag and a plain closure sink. No tokio, no timing-dependent assertions. This proves the core's emission logic is correct in isolation; the FFI layer (which *does* spawn tokio tasks and bridge cancellation tokens) is tested separately for panic containment via `panic_containment.rs`.

**Rationale** (Constitution V — decision logic in core, tested in core):
- The *decision* to stop emitting (when `cancel` is set) is made in the pure core driver
- The core driver is tested without tokio or FFI, so its cancellation semantics are deterministic and decoupled from async runtime behavior
- The FFI layer handles tokio spawning, panic containment, and token-to-flag bridging; it inherits correctness from the core

### Rust: Collision-Safe File Operations (T054, US2)

Workspace file operations (`create_note`, `rename_node`, `move_node`) are tested for **collision safety** — they never overwrite existing files/folders and use a deterministic auto-suffix scheme:

```rust
/// Creating `note.md` when it already exists must NOT overwrite it; the new file
/// is auto-suffixed to `note 2.md`, and the original is byte-for-byte intact.
#[test]
fn create_note_collision_auto_suffixes_and_preserves_original() {
    let dir = tempdir().unwrap();
    let ws = Workspace::new();

    let first = ws.create_note(dir.path().to_str().unwrap(), "note.md").unwrap();
    std::fs::write(&first, "ORIGINAL").unwrap();

    let second = ws.create_note(dir.path().to_str().unwrap(), "note.md").unwrap();
    
    // First file is untouched
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "ORIGINAL");
    // Second has the auto-suffix
    assert_eq!(name_of(&second), "note 2.md");
}
```

**Naming scheme** (pinned as executable contract):
- `note.md` taken → `note 2.md`, then `note 3.md`, …
- `folder` taken → `folder 2`, then `folder 3`, …
- Multi-dot `a.tar.gz` taken → `a 2.tar.gz` (split on last dot)

### Rust: Watcher + Conflict Resolution (T057/T065, US2)

File watcher integration tests verify:
1. **Debouncing**: Bursts of FSEvents are coalesced into single updates
2. **Rename correlation**: One rename event (not delete+create) via `FileIdCache`
3. **Self-write suppression**: Our own atomic saves don't echo back (post-write `(mtime,len)` fed to watcher)
4. **Conflict truth table**: Open file changes on disk → clean (reload) vs dirty (preserve+mark stale)

### Rust: Panic Containment Testing

The `panic_containment.rs` test (T015) verifies that forced panics in async tasks surface as `EmendError::Internal` without aborting:

```rust
#[test]
fn forced_panic_surfaces_as_internal_error_and_process_survives() {
    let caught: Result<(), EmendError> =
        with_silent_panic_hook(|| contain_panic(|| panic!("simulated task panic")));
    
    match caught {
        Err(EmendError::Internal { .. }) => {
            // Expected: panic was caught and mapped
        }
        other => panic!("Unexpected result: {:?}", other),
    }
}
```

The `with_silent_panic_hook` helper swaps the panic hook during the test to avoid stderr noise. Synchronization via `OnceLock<Mutex<()>>` ensures tests don't stomp on the global hook.

### Swift: Pure Transform Testing (Headless, Isolated)

Edit transforms return pure `Edit` structures (range + replacement + selection) without side effects. Tests apply edits to plain strings and assert results:

```swift
final class SmartListsTests: XCTestCase {
    private func applied(_ edit: SmartLists.Edit?, to text: String) -> String? {
        guard let edit else { return nil }
        let mutable = NSMutableString(string: text)
        mutable.replaceCharacters(in: edit.range, with: edit.replacement)
        return mutable as String
    }

    func testReturnContinuesBulletList() throws {
        let text = "- hello"
        let edit = SmartLists.newline(in: text as NSString, selection: caret(text.utf16.count))
        XCTAssertEqual(applied(edit, to: text), "- hello\n- ")
        XCTAssertEqual(try XCTUnwrap(edit).selectionAfter, caret(10))
    }
}
```

These **unit tests require no AppKit, no window, no @MainActor**; they run in isolation and are fast.

### Swift: Integration Testing with Real Components

```swift
@MainActor
final class EditorPersistenceTests: XCTestCase {
    func testEditingExistingDocumentPersists() throws {
        // Create a real temp file with initial content
        let directory = try makeTempDirectory()
        let url = directory.appendingPathComponent("seed.md")
        try "# Title\n".write(to: url, atomically: true, encoding: .utf8)
        let path = url.path(percentEncoded: false)
        
        // Open the document in the real editor model
        let handle = try openDocument(path: path)
        let initial = (try? readFileAt(path: path)) ?? ""
        let editor = makeEditor(handle: handle, initialText: initial)
        
        // Edit via the real NSTextView storage
        let end = editor.textView.textStorage?.length ?? 0
        type("body text", into: editor.textView, at: end)
        
        // Flush autosave to disk
        editor.autosave.flushNow()
        try handle.close()
        
        // Verify round-trip
        XCTAssertEqual(try readFileAt(path: path), "# Title\nbody text")
    }
}
```

## Mocking & Test Fixtures

### Rust: No External Mocks

The Rust core avoids mocking libraries (`mockall`, `proptest`) in favor of **pure functions and fixtures**:
- Pure functions (no I/O side effects) are tested directly
- File I/O is tested with real temp files via `tempfile` crate
- Search driver is tested with in-memory `Index` fixtures (no Rust core FFI, no tokio)
- AI/HTTP logic is tested with request/response fixtures (deferred to Phase 2+)

### Swift: Minimal Mocking

Swift tests use **headless XCTest** without mocking frameworks. Smoke tests verify linkage; behavior tests exercise real components or defer to Phase 2+ when new logic lands.

### Test Data

**Fixtures in Rust tests**:
- Hardcoded strings (e.g., `"hello"`, `"a😀b"` for UTF-16 tests)
- Temp files created by `tempfile` crate (atomic cleanup via `defer`)
- Pre-seeded directory trees (`seededDirectory(files:folders:)`) for workspace tests
- In-memory `Index` with `n` pre-inserted notes (search tests; no disk I/O)

**Fixtures in Swift tests**:
- Simple test doubles (e.g., fake bookmarks, `makeTempDirectory()`) or hardcoded test data
- Real `EmendCore` API calls and real `NSTextView` storage (no mocking)
- Real file I/O via `FileManager` to verify end-to-end persistence
- Real Rust workspace handle + SwiftUI model instances for integration tests
- Temp workspace with pre-seeded notes for Quick Open tests
- Long markdown fixture + temp PDF file for export tests (US4)

## Benchmarking

### Criterion Harness

Criterion benchmarks are located in `crates/emend-bench/` with two key properties:

1. **Non-blocking**: Perf budgets are tracked per Constitution I, but regressions do not fail CI
2. **Compile-checked**: `cargo bench --no-run` in CI ensures the harness compiles but does not measure

### Benchmark Files

| Path | Purpose |
|------|---------|
| `crates/emend-bench/benches/smoke.rs` | Smoke benchmark verifying the Criterion pipeline compiles and runs (trivial `U16Range::end()` measurement) |
| `crates/emend-bench/benches/quick_open.rs` | Quick Open fuzzy-search ranking over 10k-entry index; budget ≤100 ms p95 warm (T071, SC-004, US3) |

### Running Benchmarks

```bash
# Measure (slow):
cargo bench

# Compile-check only (CI):
cargo bench --no-run

# Specific benchmark:
cargo bench -- quick_open_10k
```

### Quick Open Benchmark (T071, US3)

Measures a single **warm query** over a 10k-entry index seeded with realistic folder structure (`notes/`, `projects/`, `archive/`). Benchmarks three query shapes because fuzzy-match cost varies:

- **`"note"`** — Common substring matching *every* entry (worst case: full haystack scored, full results streamed)
- **`"note-7777"`** — Near-exact match (typical user typing "I roughly know what I want")
- **`"zzqq"`** — No match (pure scoring cost, zero results)

**Budget**: ≤100 ms p95 warm per Constitution SC-004 and NFR-018. This is tracked non-blocking; regressions are visible but do not fail CI.

## Coverage Requirements

Coverage is **not enforced** in CI but is monitored for regressions:

### Target Metrics (non-blocking)

| Metric | Target |
|--------|--------|
| Line coverage (core logic) | 80%+ |
| Branch coverage (error paths) | 75%+ |
| Function coverage | 85%+ |

### Coverage Exclusions

The following are not counted:
- `crates/emend-ffi/` — Thin UniFFI shim; coverage is validated via Rust core tests + Swift linkage tests
- `swift/EmendCore/Sources/EmendCoreFFI/` — Generated UniFFI bindings
- `app/Emend/Emend/` — AppKit/SwiftUI glue code (tested pragmatically per Constitution VII)
- `*.config.ts`, `*.yml`, `*.toml` — Configuration files

## Test Categories

### Smoke Tests (Rust)

Minimal, fast tests verifying the crate builds and basic APIs work:

| Test | File | Purpose |
|------|------|---------|
| `u16range_end_is_start_plus_len` | `crates/emend-core/src/lib.rs` | Verify `U16Range` calculation |
| `from_text_then_text_round_trips` | `crates/emend-core/src/document.rs` | Document round-trip text identity |

### Smoke Tests (Swift)

Verify linkage and ABI stability:

| Test | File | Purpose |
|------|------|---------|
| `testAbiVersionIsStable` | `swift/EmendCore/Tests/EmendCoreTests/EmendCoreTests.swift` | Core reports stable ABI version |
| `testCoreAbiVersionIsStable` | `app/Emend/EmendTests/EmendCoreLinkageTests.swift` | App links and reaches core ABI |

### Unit Tests (Editor Transforms, Headless)

Pure, isolated tests of editor behavior without UI:

| Test | File | Purpose | Critical |
|------|------|---------|----------|
| Smart list transforms | `app/Emend/EmendTests/SmartListsTests.swift` | Bullet continuation, number increment, task checkbox toggle, indentation preservation (T045) | Yes (per-keystroke UX) |
| Formatting commands | `app/Emend/EmendTests/FormattingCommandsTests.swift` | Bold `**`, italic `*`, code `` ` `` wrap/unwrap (T046) | Yes (core editing) |
| Syntax highlighting | `app/Emend/EmendTests/SyntaxAttributingTests.swift` | NSAttributedString synthesis from tree-sitter blocks (T047) | Yes (visual feedback) |

### Integration Tests

Full-feature tests exercising public APIs and boundaries:

| Test | File | Purpose | Critical |
|------|------|---------|----------|
| UTF-16 round-trips | `crates/emend-core/tests/document.rs` | UTF-16↔char conversions for per-keystroke editor | Yes (US1 hot path) |
| Astral character handling | `crates/emend-core/tests/document.rs` | Astral chars (😀) in documents splice cleanly | Yes (emoji input) |
| CRLF tolerance | `crates/emend-core/tests/document.rs` | Mixed LF/CRLF in same document | Yes (cross-platform) |
| Incremental parsing | `crates/emend-core/tests/parse_incremental.rs` | Tree-sitter incremental updates, edge cases | Yes (highlight synthesis) |
| File atomicity | `crates/emend-core/tests/fs_atomic.rs` | Writes via temp+fsync+rename are atomic | Yes (FR-009a autosave) |
| Collision safety | `crates/emend-core/tests/workspace_ops.rs` | Create/rename/move never overwrite; `note 2.md` suffix scheme | Yes (FR-004/FR-004a, US2) |
| Watcher + conflict resolution | `crates/emend-core/tests/watcher.rs` | Debounce, rename correlation, self-write suppression, truth table | Yes (FR-006a/FR-006b/FR-006c, US2) |
| Search index | `crates/emend-core/tests/index.rs` | Incremental index, fuzzy ranking, wiki-link O(1) lookup | Yes (FR-017/FR-017a/FR-019a, US2) |
| Search supersede | `crates/emend-core/tests/search_supersede.rs` | Cancel flag stops emission at batch boundary; pre-cancelled emits nothing; completion reported correctly | Yes (FR-017/FR-018, NFR-002, US3) |
| Panic containment | `crates/emend-ffi/tests/panic_containment.rs` | Panics in async tasks surface as `EmendError::Internal` | Yes (NFR-003) |
| Bookmark resolution | `app/Emend/EmendTests/BookmarkResolutionTests.swift` | Security-scoped bookmarks resolve to files | Yes (FR-004) |
| Editor persistence | `app/Emend/EmendTests/EditorPersistenceTests.swift` | Full stack: type → autosave → disk → re-read (T049) | Yes (FR-009) |
| Workspace flow | `app/Emend/EmendTests/WorkspaceFlowTests.swift` | Add folder → tree → open tab → move/rename (T067, US2) | Yes (workspace UX) |
| Quick Open flow | `app/Emend/EmendTests/QuickOpenTests.swift` | Seed index → query streams results → Return opens file (T078, US3) | Yes (Quick Open UX, FR-017/FR-018) |
| PDF export | `app/Emend/EmendTests/PreviewExportTests.swift` | Render long doc → off-screen paginate → verify multi-page PDF (T091, US4) | Yes (FR-026/SC-010) |

## CI Integration

### Test Pipeline (`.github/workflows/ci.yml`)

Runs on every `push` to `main` and `pull_request`:

```yaml
jobs:
  rust:
    name: Rust core
    runs-on: macos-14 (Apple Silicon)
    steps:
      1. Checkout
      2. Install Rust (stable + clippy + rustfmt)
      3. Format check (cargo fmt --check)
      4. Clippy (cargo clippy --all-targets -- -D warnings)
      5. Test (cargo test --all)
      6. Bench compile-check (cargo bench --no-run)
      7. MSRV check (cargo +1.85 check --all)  # Workspace rust-version = "1.85"
      
  swift:
    name: Swift app
    runs-on: macos-14 (Apple Silicon)
    steps:
      1. Checkout
      2. Select Xcode 16.2
      3. Install Rust + aarch64-apple-darwin target
      4. Install Swift tooling (swiftformat, swiftlint, xcodegen)
      5. SwiftFormat lint (swiftformat app swift --lint)
      6. SwiftLint (swiftlint lint --strict)
      7. Build XCFramework + Swift bindings
      8. Generate Xcode project (xcodegen)
      9. Build & test app (xcodebuild test ... CODE_SIGNING_ALLOWED=NO)
      
  commits:
    name: Conventional commits
    runs-on: ubuntu-latest
    steps:
      1. Validate PR commit subjects match Conventional Commits
```

### Required Checks (Blocking)

| Check | Blocks Merge |
|-------|-------------|
| Rust format check | Yes |
| Rust clippy | Yes |
| Rust tests (cargo test) | Yes |
| Rust MSRV 1.85 | Yes |
| Swift SwiftFormat lint | Yes |
| Swift SwiftLint (--strict) | Yes |
| Swift app tests (headless) | Yes |
| Conventional commits | Yes (PRs only) |

Benchmark measurements (`cargo bench --no-run` compile-check) are **non-blocking**.

## Test Philosophy

Per **Constitution VII** ("Testing is strict in core, pragmatic in UI"):

### Strict Core Testing

`emend-core` enforces:
- ✅ All public APIs have integration tests
- ✅ UTF-16↔char conversions have exhaustive coverage (astral chars, CRLF, boundaries)
- ✅ Error paths are tested (timeout, cancellation, oversized input)
- ✅ Panic containment is verified across FFI
- ✅ Incremental parsing edge cases are covered (T021)
- ✅ Collision-safe file operations are guaranteed (T054, US2)
- ✅ Watcher + conflict resolution deterministically tested (T057/T065, US2)
- ✅ Incremental search index verified (T073, US2)
- ✅ Cancellable search driver tested synchronously without tokio (T072, US3)

### Pragmatic UI Testing

`app/Emend` enforces:
- ✅ Smoke tests (linkage, ABI version)
- ✅ Pure transform tests (headless, isolated unit tests for editor behavior)
- ✅ Critical-path integration tests (persistence, bookmark resolution, workspace flow, Quick Open end-to-end, PDF export)
- ⏳ Full-app behavior tests deferred until features land (Phase 2+)

**Rationale**: Headless app-hosted testing (via `@testable` + real components) avoids GUI automation costs (signing, rendering, timers) while still verifying end-to-end correctness. Pure transforms are tested in isolation without AppKit. `NSOutlineView` rendering, drag-drop gestures, live-refresh runtime, and on-screen preview rendering remain manual-verification (Constitution VII).

### Benchmark Philosophy

Perf budgets are **tracked, non-blocking** per Constitution I:
- Criterion harness compiles and runs in CI
- Regressions are visible but do not fail CI
- Real measurements happen locally or in performance-focused runs

---

*This document describes HOW to test. Update when testing strategy changes.*
