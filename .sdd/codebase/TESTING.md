# Testing Strategy

> **Purpose**: Document test frameworks, patterns, organization, and coverage requirements.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (US7 Phase 9 + Phase 10 Polish)

## Overview

Emend follows a **test-first, strict-core** strategy (Constitution VII):
- **Rust core** (`emend-core`): Comprehensive unit/integration tests, run in isolation via `cargo test` (no FFI toolchain required)
- **Swift core package** (`swift/EmendCore`): ABI version smoke tests and AsyncStream adapter tests
- **Swift app** (`app/Emend`): Smoke/linkage tests plus critical-path integration tests; headless XCTest (no GUI automation)
- **Benchmarks**: Non-blocking perf tracking via Criterion (compile-checked in CI)
- **Quality & Verification** (Phase 10): Large-file, preview-offline, and perf-budget tests

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
| `cargo bench` | Criterion benchmarks (measured, non-blocking) | macOS with stable Rust toolchain |
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
     - `crates/emend-core/tests/links.rs` — Wiki-link deterministic resolution (T095, US5)
     - `crates/emend-core/tests/embeds.rs` — Embed expansion + cycle/depth guards (T096, US5)
     - `crates/emend-core/tests/derived_stats.rs` — Doc stats, task N-of-M, outline (T108, US6)
     - `crates/emend-core/tests/ai_sse.rs` — SSE parser edge cases (T109, US6)
     - `crates/emend-core/tests/ai_privacy.rs` — Secret hygiene + max-input guard (T110, US6)
     - `crates/emend-core/tests/settings.rs` — Typography settings store, clamping, round-trip (T123, US7)
     - `crates/emend-core/tests/large_file.rs` — Max-note-size cap + incremental re-parse on large doc (T133, Phase 10)
     - `crates/emend-core/tests/preview_offline.rs` — Preview rendering stays offline (T083, Phase 10)
     - `crates/emend-ffi/tests/panic_containment.rs` — Panic capture across FFI (T015)

#### Test Files (Comprehensive, Phase 10)

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
| `crates/emend-core/tests/links.rs` | Deterministic resolution for duplicate basenames, rename leaves old links unresolved, extraction + suggestions | Wiki-link resolution (T095, FR-019/FR-019a, US5) |
| `crates/emend-core/tests/embeds.rs` | Simple embeds inline, unresolved degrade gracefully, cycles terminate, depth bounded at MAX_EMBED_DEPTH | Embed expansion (T096, FR-021/FR-021a, US5) |
| `crates/emend-core/tests/derived_stats.rs` | Word count, character count, reading time, task N-of-M completion, outline with line numbers | Doc stats + outline (T108, FR-029/030/031a, US6) |
| `crates/emend-core/tests/ai_sse.rs` | SSE chunks reassemble across boundaries, [DONE] terminates, comments/blanks ignored, CRLF/LF tolerated, closed connection clean | SSE parser (T109, FR-032/036, US6) |
| `crates/emend-core/tests/ai_privacy.rs` | Oversized input rejected locally before send, API key Debug/Display redaction | Secret hygiene + max-input (T110, FR-035/036a, NFR-006, SC-008, US6) |
| `crates/emend-core/tests/settings.rs` | Sane defaults, round-trip set→get, clamping out-of-range values (size, spacing, line-height), NaN/infinity repair, font family fallback | Typography store (T123, FR-038/FR-039, US7) |
| `crates/emend-core/tests/large_file.rs` | Max-note-size cap (~5 MiB), stat-before-allocate refusal, local splice on ~1 MiB edit, incremental re-parse correctness | Bounded-memory guarantee (T133, FR-027a, Phase 10 Polish) |
| `crates/emend-core/tests/preview_offline.rs` | Remote image/link URLs rendered as literal references, no fetch, no data-uri inlining | Privacy-by-default (T083, SC-008, Phase 10 Polish) |
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

### Rust Benchmarks (Phase 10, Non-Blocking)

Performance budgets are **tracked but non-blocking** (Constitution IV). Criterion benches measure actual performance and surface regressions without gating CI:

#### Benchmark Files

| Path | Purpose | Budget | Scope |
|------|---------|--------|-------|
| `crates/emend-bench/benches/highlight.rs` | Per-keystroke incremental reparse on large doc (~1 MB) | < 5 ms p95 warm | SC-003: Editor hot path / keystroke latency (T037) |
| `crates/emend-bench/benches/quick_open.rs` | Fuzzy search 10k-entry index, warm cache | < 100 ms p95 warm | SC-004: Quick Open response / interactive feel (T071) |
| `crates/emend-bench/benches/open_doc.rs` | Core open + initial parse (dominant pre-paint) | Tracked | SC-002: Document load / first-content (T030, Phase 10) |
| `crates/emend-bench/benches/smoke.rs` | Basic compilation & setup smoke test | Tracked | Verify benches compile and run (T033) |

**Bench Pattern**:
```rust
use std::hint::black_box;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

fn my_bench(c: &mut Criterion) {
    c.bench_function("operation_name", |b| {
        b.iter_batched(
            || setup_data(),  // Setup: excluded from timing
            |data| {
                // Measured: the actual operation
                black_box(data.do_thing())
            },
            BatchSize::SmallInput,
        );
    });
}
```

**Key points**:
- `iter_batched` excludes setup (data generation, fixture creation) from timing
- `black_box` prevents the optimizer from deleting the work
- Benches stay **panic-free** — use `.ok()` instead of `.unwrap()` for fallible ops
- Results are recorded in implementation reports; deviations are documented, not gated

### Swift Tests

#### Location Strategy

Swift tests follow Xcode conventions: separate `Tests/` directories within each target.

| Target | Test Location | Purpose |
|--------|---------------|---------|
| `EmendCore` package | `swift/EmendCore/Tests/EmendCoreTests/` | Unit tests for the clean API wrapper and streaming |
| `Emend` app | `app/Emend/EmendTests/` | Smoke + linkage tests, critical-path integration tests |

#### Test Files (Comprehensive, US7 + Phase 10)

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
| `app/Emend/EmendTests/LinkHelpersTests.swift` | Pure transforms: wiki-link range detection, task checkbox toggle, image drop markdown (T103, US5, headless) | Unit |
| `app/Emend/EmendTests/LinksFlowTests.swift` | End-to-end links: resolve + suggest wiki-links, embed inlines, store attachments (T104, US5) | Integration |
| `app/Emend/EmendTests/KeychainStoreTests.swift` | Keychain round-trip: save/read/upsert/delete AI API key (T119, US6; skips if env denies Keychain access) | Unit integration |
| `app/Emend/EmendTests/InfoSidebarTests.swift` | Info sidebar stats/outline: word count, char count, task N-of-M, heading tree with line numbers (T119, US6) | Integration |
| `app/Emend/EmendTests/TypographyTests.swift` | Typography settings model clamp + persistence, resolver font/CSS synthesis (T127, US7, headless) | Unit integration |
| `app/Emend/EmendTests/MemoryReleaseTests.swift` | Weak-reference release: TabModel/OpenDocHandle/AutosaveController deallocate on close (T135, NFR-005, Phase 10) | Integration |

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

Tests that create or interact with `@MainActor` models (e.g., `WorkspaceModel`, `TabModel`, `QuickOpenModel`, `InfoModel`, `TypographyModel`) must themselves be annotated `@MainActor`:

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

@MainActor
final class TypographyTests: XCTestCase {
    func testApplyClampsAndPersists() throws {
        let defaults = try freshDefaults()
        let model = TypographyModel(defaults: defaults)

        // Out-of-range values are clamped by the core; the font family is kept.
        model.apply(TypographySettings(
            fontFamily: "Menlo", fontSizePt: 999, lineHeight: 99, paragraphSpacingPt: -5
        ))
        XCTAssertLessThanOrEqual(model.settings.fontSizePt, 48)
        XCTAssertLessThanOrEqual(model.settings.lineHeight, 3.0)
        
        // A new model over the same defaults reads the persisted (clamped) values.
        let reloaded = TypographyModel(defaults: defaults)
        XCTAssertEqual(reloaded.settings, model.settings)
    }
}
```

**Rationale**: These models own `@Published` properties and must update on the main thread. XCTest automatically runs each test on the main thread if the test class is marked `@MainActor`.

#### Keychain Availability Gating (US6)

The `KeychainStoreTests` uses `XCTSkip` when the environment denies Keychain access (e.g., unsigned CI binaries):

```swift
@MainActor
final class KeychainStoreTests: XCTestCase {
    func testRoundTripSaveReadUpsertDelete() throws {
        do {
            try KeychainStore.save("sk-secret-123", account: account)
        } catch let KeychainStore.KeychainError.unexpectedStatus(status) {
            throw XCTSkip("Keychain unavailable in this environment (OSStatus \(status))")
        }

        XCTAssertEqual(KeychainStore.read(account: account), "sk-secret-123")
        // ... rest of test
    }
}
```

**Rationale** (per guardrail US2 — "no GUI-automation under CI's unsigned environment"): The test process isn't sandboxed, so it can access Keychain where available, but CI's unsigned binary may be denied. Rather than failing, the test skips, allowing the wrapper logic to be verified wherever possible.

#### Memory Release Testing (T135, Phase 10)

```swift
@MainActor
final class MemoryReleaseTests: XCTestCase {
    func testAutoSaveControllerReleasesOnDocClose() throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("emend-mem-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        
        let note = dir.appendingPathComponent("note.md")
        try "test".write(to: note, atomically: true, encoding: .utf8)
        
        // Create editor + autosave, get weak ref
        var handle: OpenDocHandle? = try openDocument(path: note.path)
        var autosave: AutosaveController? = AutosaveController(handle: handle!)
        weak var weakHandle = autosave?.handle
        weak var weakAutosave = autosave
        
        autosave = nil
        handle = nil
        
        // Both must release (no leak, no retain cycle)
        XCTAssertNil(weakAutosave, "autosave released when dealloc (NFR-005)")
        XCTAssertNil(weakHandle, "handle released when autosave dealloc (NFR-005)")
    }
}
```

This pattern verifies **no retain cycles** — critical for responsive app memory management (NFR-005).

## Test Patterns

### Rust: Large-File Testing (T133, Phase 10)

The `large_file.rs` integration test verifies **bounded-memory guarantee** (FR-027a) and **incremental re-parse correctness** on large documents:

```rust
//! T133 — max-note-size cap + incremental re-parse on a large document.
//!
//! FR-027a: System MUST define a maximum supported note size; beyond it,
//! behavior MUST be graceful (refuse with a clear message) rather than
//! hang or exhaust memory.
//!
//! Key tests:
//! 1. Stat-before-allocate: over-cap file is refused WITHOUT building a huge rope
//! 2. One-byte-over boundary: error reports bytes == cap + 1 (proves stat-based check)
//! 3. File exactly at cap: opens successfully
//! 4. Large-but-under-cap edit: incremental splice, correct text, not a rebuild
//! 5. Single char on ~1 MB doc: incremental re-parse succeeds, highlighter consistent

#[test]
fn large_file_edit_is_incremental_splice_not_rebuild() {
    let mut doc = Document::from_text(&build_doc(1_000_000));
    let initial_len = doc.len_utf16();
    
    // Insert at position 500_000 (middle of the ~1 MB doc)
    let edit = U16Range { start: 500_000, len: 0 };
    doc.push_edit(edit, "X");  // One character
    
    // Check: length increased by exactly 1
    assert_eq!(doc.len_utf16(), initial_len + 1);
    
    // Check: text landed in the right place
    let text = doc.to_string();
    assert!(text.contains("...sometext...X...moretext..."));  // Simplified check
}

#[test]
fn incremental_reparse_on_large_doc_succeeds() {
    let doc_text = build_doc(1_000_000);
    let mut hl = Highlighter::new(&doc_text);
    
    // Apply a single-char edit at position 500_000
    hl.apply_edit(U16Range { start: 500_000, len: 0 });
    
    // Incremental parse succeeded; highlighter is queryable
    let spans = hl.highlight_spans(U16Range { start: 499_900, len: 200 });
    assert!(!spans.is_empty(), "incremental reparse produced spans");
}
```

**Key guarantees** (Constitution IV, tracked non-blocking):
- Documents over `MAX_NOTE_BYTES` (5 MiB) are refused with a clear `NoteTooLarge` error
- Stat-before-allocate ensures no 5+ MB rope is ever built for an oversized file (OOM-safe)
- Edits on large documents use incremental tree-sitter re-parse, not full rebuild (O(delta) not O(document))

### Rust: Preview Offline Testing (T083, Phase 10)

The `preview_offline.rs` integration test verifies **structural privacy** (SC-008 / FR-035):

```rust
//! T083 — preview rendering performs ZERO network access (SC-008 / FR-035).
//!
//! The meaningful, mechanically checkable assertion is **structural**:
//! a document that references remote resources renders them as literal
//! `src=`/`href=` attributes — the engine never dereferences a URL.

#[test]
fn remote_image_url_stays_a_literal_src_and_is_not_fetched() {
    let md = "![alt](https://example.com/x.png)\n";
    let html = render_preview_html(md, &PreviewOptions::default()).unwrap();

    // The remote URL appears verbatim as an <img src=...> — no fetch/inline.
    assert!(
        html.contains("<img") && html.contains("src=\"https://example.com/x.png\""),
        "remote image should render as a literal src reference (no fetch)"
    );
    // Defensive: a fetch would inline bytes (data: URI). None must appear.
    assert!(
        !html.contains("data:image"),
        "renderer must not inline remote image bytes"
    );
}

#[test]
fn remote_link_url_stays_a_literal_href() {
    let md = "[site](https://example.com/page)\n";
    let html = render_preview_html(md, &PreviewOptions::default()).unwrap();
    
    assert!(
        html.contains("href=\"https://example.com/page\""),
        "remote link URL is literal, not fetched"
    );
}
```

**Key property**: The core render path (`renderPreviewHtml`) has **no `reqwest` dependency** — the compiler enforces this structural guarantee. The privacy property cannot regress into a network call.

## Mocking & Test Fixtures

### Rust: No External Mocks

The Rust core avoids mocking libraries (`mockall`, `proptest`) in favor of **pure functions and fixtures**:
- Pure functions (no I/O side effects) are tested directly
- File I/O is tested with real temp files via `tempfile` crate
- Search driver is tested with in-memory `Index` fixtures (no Rust core FFI, no tokio)
- Link/embed resolution is tested with in-memory `Index` fixtures (T095/T096, US5)
- Settings validation is tested with in-memory `TypographyStore` (T123, US7)
- AI/SSE is tested with hardcoded string fixtures (pure parser, no reqwest)
- Doc stats/outline is tested with hardcoded markdown strings (pure computation)
- Large-file testing uses synthetic ~1 MB documents (T133, Phase 10)

### Swift: Minimal Mocking

Swift tests use **headless XCTest** without mocking frameworks. Smoke tests verify linkage; behavior tests exercise real components or defer to Phase 2+ when new logic lands.

### Test Data

**Fixtures in Rust tests**:
- Hardcoded strings (e.g., `"hello"`, `"a😀b"` for UTF-16 tests, `"[[Beta]]"` for link tests, `"# Title\n\n..."` for stats tests)
- Temp files created by `tempfile` crate (atomic cleanup via `defer`)
- Pre-seeded directory trees (`seededDirectory(files:folders:)`) for workspace tests
- In-memory `Index` with `n` pre-inserted notes (search tests; no disk I/O)
- In-memory note store (`HashMap<String, String>`) for embed tests (T096)
- OpenAI-style SSE chunks with `data:` lines for parser tests (T109)
- In-memory `TypographyStore` with various input values for settings validation (T123)
- Large synthetic documents (build_doc(N_BYTES)) for large-file tests (T133, Phase 10)
- Realistic Markdown with headings, lists, emphasis for stats tests (T108, US6)

---

*This document describes HOW to test. Update when testing strategy changes.*
