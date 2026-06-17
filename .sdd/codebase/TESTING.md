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
     - `crates/emend-ffi/tests/panic_containment.rs` — Panic capture across FFI (T015)

#### Test Files

| Path | Purpose | Scope |
|------|---------|-------|
| `crates/emend-core/tests/document.rs` | UTF-16↔char conversions, line indexing, edit splicing | Per-keystroke editor hot path correctness (research §A2/A3) |
| `crates/emend-core/tests/fs_atomic.rs` | Atomic+durable writes, tolerance to BOM/CRLF | File I/O reliability (FR-009a, research §B4) |
| `crates/emend-core/tests/parse_incremental.rs` | Tree-sitter incremental parsing, astral chars, edge cases | Highlight synthesis (T021, research §B1) |
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

#### Test Files

| Path | Purpose |
|------|---------|
| `swift/EmendCore/Tests/EmendCoreTests/EmendCoreTests.swift` | Core ABI version smoke test |
| `swift/EmendCore/Tests/EmendCoreTests/StreamingTests.swift` | AsyncStream adapter correctness (`AiStreamAdapter`, `SearchStreamAdapter`) |
| `app/Emend/EmendTests/EmendCoreLinkageTests.swift` | App links the local `EmendCore` package; core ABI is reachable |
| `app/Emend/EmendTests/BookmarkResolutionTests.swift` | Security-scoped bookmark → file I/O flow |
| `app/Emend/EmendTests/SmartListsTests.swift` | Pure smart-list transforms: list continuation, empty-item termination, indentation preservation (T045, headless) |
| `app/Emend/EmendTests/FormattingCommandsTests.swift` | Formatting transforms: bold, italic, code (T046, headless) |
| `app/Emend/EmendTests/SyntaxAttributingTests.swift` | Syntax highlight attribute synthesis from tree-sitter (T047, headless) |
| `app/Emend/EmendTests/EditorPersistenceTests.swift` | End-to-end persistence: `EditorCoordinator` + `AutosaveController` + disk round-trip (T049, headless integration) |

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
- AI/HTTP logic is tested with request/response fixtures (deferred to Phase 2+)

### Swift: Minimal Mocking

Swift tests use **headless XCTest** without mocking frameworks. Smoke tests verify linkage; behavior tests exercise real components or defer to Phase 2+ when new logic lands.

### Test Data

**Fixtures in Rust tests**:
- Hardcoded strings (e.g., `"hello"`, `"a😀b"` for UTF-16 tests)
- Temp files created by `tempfile` crate (atomic cleanup via `defer`)

**Fixtures in Swift tests**:
- Simple test doubles (e.g., fake bookmarks, `makeTempDirectory()`) or hardcoded test data
- Real `EmendCore` API calls and real `NSTextView` storage (no mocking)
- Real file I/O via `FileManager` to verify end-to-end persistence

## Benchmarking

### Criterion Harness

Criterion benchmarks are located in `crates/emend-bench/` with two key properties:

1. **Non-blocking**: Perf budgets are tracked per Constitution I, but regressions do not fail CI
2. **Compile-checked**: `cargo bench --no-run` in CI ensures the harness compiles but does not measure

### Benchmark Files

| Path | Purpose |
|------|---------|
| `crates/emend-bench/benches/smoke.rs` | Smoke benchmark verifying the Criterion pipeline compiles and runs (trivial `U16Range::end()` measurement) |
| Future: `benches/highlight.rs` | Editor highlight incremental parsing (Phase 3+) |
| Future: `benches/quick_open.rs` | Fuzzy search ranking (Phase 3+) |

### Running Benchmarks

```bash
# Measure (slow):
cargo bench

# Compile-check only (CI):
cargo bench --no-run

# Specific benchmark:
cargo bench -- u16range_end
```

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
| Panic containment | `crates/emend-ffi/tests/panic_containment.rs` | Panics in async tasks surface as `EmendError::Internal` | Yes (NFR-003) |
| Bookmark resolution | `app/Emend/EmendTests/BookmarkResolutionTests.swift` | Security-scoped bookmarks resolve to files | Yes (FR-004) |
| Editor persistence | `app/Emend/EmendTests/EditorPersistenceTests.swift` | Full stack: type → autosave → disk → re-read (T049) | Yes (FR-009) |

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

### Pragmatic UI Testing

`app/Emend` enforces:
- ✅ Smoke tests (linkage, ABI version)
- ✅ Pure transform tests (headless, isolated unit tests for editor behavior)
- ✅ Critical-path integration tests (persistence, bookmark resolution)
- ⏳ Full-app behavior tests deferred until features land (Phase 2+)

**Rationale**: Headless app-hosted testing (via `@testable` + real components) avoids GUI automation costs (signing, rendering, timers) while still verifying end-to-end correctness. Pure transforms are tested in isolation without AppKit.

### Benchmark Philosophy

Perf budgets are **tracked, non-blocking** per Constitution I:
- Criterion harness compiles and runs in CI
- Regressions are visible but do not fail CI
- Real measurements happen locally or in performance-focused runs

---

*This document describes HOW to test. Update when testing strategy changes.*
