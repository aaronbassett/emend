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
- **Test files (`tests/*.rs`)**: Module-level doc comment (prefixed `//!`) explaining the test scope

**Example** (from `tests/document.rs`):
```rust
//! T018 — failing-first tests for the open-document model (`emend_core::document`).
//!
//! These tests pin down the **UTF-16 boundary** that the per-keystroke hot path
//! depends on (research §A2/§A3, FFI contract §3).
```

## Swift Code Style

### Formatting Tools

| Tool | Configuration | Command |
|------|---------------|---------|
| SwiftFormat | `.swiftlint.yml` + CLI flags | `swiftformat app swift --lint` |
| SwiftLint | `.swiftlint.yml` --strict | `swiftlint lint --strict` |

### SwiftFormat Settings (via CLI in lefthook.yml)

- **Trailing commas**: disabled (SwiftLint rules conflict; rely on SwiftFormat's default)
- **Import ordering**: `testable-bottom` (ensures `@testable` imports appear last)

### SwiftLint Configuration (.swiftlint.yml)

**Line length**: Warning at 100 chars, error at 140 chars (ignores comments/URLs).

**Disabled rules** (conflicts with SwiftFormat):
- `trailing_comma` — SwiftFormat controls this

**Opt-in rules** (enforce stricter checking):
- `force_unwrapping` — Mirrors Rust `unwrap_used` denial (no force-unwraps)
- `implicitly_unwrapped_optional` — Avoid IUOs
- `empty_count` — Use `.isEmpty` over `.count == 0`
- `first_where` — Prefer `first(where:)` over `filter().first`
- `contains_over_first_not_nil` — Prefer `.contains` over `.first(where:) != nil`

**Excluded paths**:
- `swift/EmendCore/Sources/EmendCoreFFI/` — Generated UniFFI bindings (not our code)
- `DerivedData/` and `.build/` — Build artifacts
- `**/*.generated.swift` — All generated files

### Naming Conventions

#### File & Directory Naming

| Type | Convention | Example |
|------|------------|---------|
| SwiftUI views | PascalCase | `MainWindow.swift`, `EmendApp.swift` |
| SwiftUI view components | PascalCase | `EditorPane.swift` |
| Utility extensions | PascalCase + descriptive | `SecurityScopedBookmarks.swift` |
| Test files | `Test.swift` or `Tests.swift` suffix | `BookmarkResolutionTests.swift` |

#### Code Element Naming (Swift)

| Type | Convention | Example |
|------|------------|---------|
| Variables | camelCase | `selectedLocation`, `isVisible`, `bookmarkData` |
| Constants (static) | camelCase | `defaultFolderSize` (or SCREAMING_SNAKE_CASE for compile-time constants) |
| Type names (struct/class/enum) | PascalCase | `MainWindow`, `AiStreamAdapter`, `SearchStreamAdapter` |
| Functions/methods | camelCase, verb-prefix for state change | `addLocation()`, `openDocument()`, `onToken(_:)` |
| Properties | camelCase | `locations`, `selection`, `abiVersion` |
| Boolean properties | `is`/`has` prefix when non-obvious | `isVisible`, `hasError` |

#### SwiftUI Conventions

- **State properties**: Prefix with `@State private var` for internal state
- **View components**: Extract into computed `var` properties for clarity (e.g., `var sidebar: some View`)
- **Closures**: Use trailing closure syntax and verb-based naming for callbacks (e.g., `onTerminate`, `onToken`)

**Example** (from `Streaming.swift`):
```swift
public static func make(
    onTerminate: @escaping @Sendable () -> Void = {}
) -> (sink: AiStreamAdapter, stream: AsyncThrowingStream<String, Error>) {
    // ...
}
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

### Swift: View Composition

Extract complex view logic into computed properties for readability:

```swift
var sidebar: some View {
    List(locations, selection: $selection) { location in
        Label(location.name, systemImage: "folder")
    }
}

var editorPane: some View {
    // Placeholder in Phase 2
    EmptyView()
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

**Scope** (optional): Lowercase, hyphenated, e.g., `(editor)`, `(ffi-boundary)`, `(swift)`.

**Breaking change** (optional): Suffix `!` before `:` (e.g., `feat(ffi)!: new ABI version`).

**Examples**:
```
feat(document): add line-column tracking
fix(fs): tolerate CRLF in file reads
docs: update UTF-16 boundary documentation
test(document): add astral-char UTF-16 tests
ci: enforce MSRV 1.85
```

The commit-msg hook validates the subject line against this pattern and rejects non-conforming commits:

```bash
head -1 "{1}" | grep -Eq '^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9_-]+\))?!?: .+'
```

## Pre-Commit Hooks (Lefthook)

Run once to install: `lefthook install` (or `just hooks`).

Hooks run automatically on `git commit` and validate:

| Hook | Languages | Commands | Parallel |
|------|-----------|----------|----------|
| rust-fmt | `*.rs` | `cargo fmt --check` | Yes |
| rust-clippy | `*.rs` | `cargo clippy --all-targets --offline -- -D warnings` | Yes |
| swift-format | `*.swift` | `swiftformat {staged_files} --lint` (graceful skip if not installed) | Yes |
| swift-lint | `*.swift` | `swiftlint lint --quiet --strict` (graceful skip if not installed) | Yes |
| commit-msg | (all) | Conventional Commits validation | No |

**Pre-commit runs in parallel** for speed. If any check fails, the commit is rejected; staged changes remain staged for fixing.

To run all checks locally (mirrors CI): `just check` or `cargo fmt && cargo clippy && cargo test && (swift checks if tools present)`.

## Code Organization

### Rust Crate Structure

- **`crates/emend-core/src/lib.rs`**: Module declaration + public type re-exports (e.g., `EmendError`, `U16Range`)
- **`crates/emend-core/src/error.rs`**: `EmendError` enum and Display/Error impls
- **`crates/emend-core/src/document.rs`**: Open-document model (shadow rope + UTF-16 indexing)
- **`crates/emend-core/src/fs.rs`**: Atomic+durable file I/O
- **`crates/emend-core/tests/`**: Integration tests (see [Testing](#testing))
- **`crates/emend-ffi/src/lib.rs`**: UniFFI `#[uniffi::export]` shim + panic containment
- **`crates/emend-bench/benches/`**: Criterion micro-benchmarks

### Swift Module Structure

- **`swift/EmendCore/Sources/EmendCore/EmendCore.swift`**: Clean public API, re-exports `EmendCoreFFI`
- **`swift/EmendCore/Sources/EmendCore/Streaming.swift`**: AsyncStream adapters over foreign-trait callbacks
- **`swift/EmendCore/Sources/EmendCoreFFI/`** (generated): UniFFI bindings (excluded from lint)
- **`app/Emend/Emend/`**: SwiftUI app (views, state, utilities)
- **`app/Emend/EmendTests/`**: App-level XCTest tests

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
