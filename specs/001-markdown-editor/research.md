# Phase 0 Research: Emend

**Feature**: 001-markdown-editor · **Date**: 2026-06-16

All major technical unknowns the spec deferred to planning are resolved below. Versions verified against crates.io / GitHub / Apple docs as of mid-2026. Format per decision: **Decision · Rationale · Alternatives · Risks/notes**.

> No project constitution exists and no prior feature retros exist, so there are no historical learnings or governance gates to fold in. The spec's own **Development Standards (DS-001..007)** and **Non-Functional Requirements (NFR-001..007)** act as the gates for this plan.

---

## A. Architecture-defining decisions (cross-cutting)

### A1. Rust ↔ Swift interop mechanism
- **Decision**: **UniFFI** (`uniffi 0.31.x`) in proc-macro mode, Swift bindings generated via `uniffi-bindgen-swift`, shipped as an **XCFramework**. The Rust core owns a single long-lived **`tokio` multi-thread runtime**. Async functions exported with `#[uniffi::export(async_runtime = "tokio")]` surface as Swift `async`. **Swift/TextKit owns the editable buffer**; per-keystroke edits are pushed to Rust as small deltas. Streaming (AI tokens, incremental search) crosses via a **foreign-trait callback** (`#[uniffi::export(rust, foreign)]`) invoked once per item. Cancellation crosses via **Rust-owned handle objects** carrying a `tokio_util::sync::CancellationToken` with `cancel()`.
- **Rationale**: UniFFI is the only candidate that gives **automatic panic containment** (every export is `catch_unwind`-wrapped → a panic becomes a catchable Swift `Error`, never an unwind across FFI — satisfies NFR-003), converts Rust `async fn` into native Swift `async`, and is actively maintained. Foreign-trait callbacks map cleanly onto SSE token streaming and incremental search.
- **Alternatives**: *swift-bridge* — no panic containment, no stream support, pre-1.0 with maintenance gaps. *Hand-written C ABI (cbindgen)* — reimplements UniFFI by hand. *Helper process / IPC* — per-keystroke serialize→socket can't meet the 50 ms budget. All rejected.
- **Risks/notes**: UniFFI does **not** wire Swift `Task` cancellation to the Rust future → the handle + `CancellationToken` pattern is mandatory. Swift 6 strict-concurrency interop is partial: **generated async types are not `Sendable`** (uniffi #2274) and a `MainActor`-default bindings module miscompiles (uniffi #2818) → compile the generated bindings target with `SWIFT_DEFAULT_ACTOR_ISOLATION = nonisolated` and prefer **synchronous** exports for the hot per-keystroke path, reserving async for AI/search. XCFramework packaging is a scripted step.

### A2. FFI coordinate system
- **Decision**: All text ranges crossing the boundary are expressed in **UTF-16 code units** (matching `NSRange`/`NSTextRange`). Rust maintains a UTF-8 rope internally but maps to/from UTF-16 offsets at the boundary.
- **Rationale**: Avoids a UTF-8↔UTF-16 conversion of the whole document on every keystroke, which would threaten the 50 ms budget (SC-003). Pin this in the UniFFI interface so it's not rediscovered late.
- **Risks/notes**: Rust must keep an efficient line/offset index that can answer UTF-16 offset ↔ (line,col) queries incrementally.

### A3. Text-buffer ownership & per-keystroke flow
- **Decision**: **Swift `NSTextStorage` is the canonical buffer.** On each edit, Swift sends a tiny delta `{utf16Range, replacement}` to Rust via a non-blocking sync export. Rust updates its shadow rope + incremental parse + search index off the main thread and returns/streams back structural + highlight spans `(utf16Range, styleClass)`. Swift applies spans only for ranges intersecting the current viewport.
- **Rationale**: If Rust owned the buffer, every keystroke would marshal text + layout queries synchronously across FFI on the main thread — exactly the cost SC-003 cannot absorb.

### A4. App Sandbox ↔ Rust file IO handshake *(highest-risk integration — prototype first)*
- **Decision**: Ship **sandboxed** from day one with `com.apple.security.files.user-selected.read-write` + **app-scoped security-scoped bookmarks**. Swift opens the security scope (`startAccessingSecurityScopedResource`) for each location and hands Rust the path; the sandbox extension is process-wide, so Rust's reads/writes/watches succeed while Swift holds the scope.
- **Rationale**: "Add any folder as a location" with persistence across launches is the canonical security-scoped-bookmark use case; adopting the sandbox now avoids a costly retrofit and keeps notarization/App Store open (distribution itself is deferred, per spec Out of Scope).
- **Risks/notes**: Bookmark lifecycle: `NSOpenPanel` → `bookmarkData(options:.withSecurityScope)` → persist → resolve on launch (`bookmarkDataIsStale` → re-create). Balance every `startAccessing…`/`stopAccessing…` (limited concurrent scopes). **Validate the FD/scope handshake with the Rust watcher in week 1.** Testable: add a folder, quit, relaunch → reads + watches with no new prompt.

---

## B. Rust core decisions

### B1. Markdown parsing + incremental strategy (two-engine split)
- **Decision**: **Editor pane** uses `tree-sitter 0.25.x` + `tree-sitter-md 0.5.2` for true edit-aware incremental reparse (`Tree::edit` + `changed_ranges` → minimal re-highlight spans). **Preview pane** uses `comrak 0.52.x` (CommonMark+GFM, canonical HTML) run **debounced off the keystroke path** (~100–150 ms idle).
- **Rationale**: No pure-Rust CommonMark renderer is incremental; reparsing a 1 MB doc per keystroke is the SC-003 risk. tree-sitter is purpose-built for incremental highlight (Helix/Zed). The editor needs only syntactic spans; the preview needs correct HTML — comrak provides it and ships 3 of the 4 custom extensions natively.
- **Custom extensions**: `[[wikilinks]]` → comrak built-in; `==highlight==` → comrak built-in (`<mark>`); task checkboxes → comrak GFM tasklist; **`![[embed]]` → the only custom code** (a scanner post-pass over inline text runs, with cycle detection + max depth per FR-021a). Editor highlight (tree-sitter, advisory) and preview (comrak, authoritative) are deliberately different engines.
- **Alternatives**: hand-rolled block-level dirty-region reparse over pulldown-cmark/comrak (correctness hazards at block boundaries); `markdown-rs` (non-incremental, alpha); pulldown-cmark for both (no incremental). Rejected as primary; pulldown-cmark is the viable preview alternative.
- **Risks/notes**: tree-sitter-md is a split grammar with imperfect conformance — acceptable because it only drives highlighting, never rendered output. A fence-toggle edit invalidates a large tail → p95 must hold for that worst case (regression test).

### B2. Fuzzy search / index
- **Decision**: **`nucleo 0.5.0`** (helix-editor; worker threadpool + lock-free `Injector`) for Quick Open ranking. Authoritative index = arena `Vec<FileEntry>` keyed by `PathId(u32)` + `HashMap<PathBuf,PathId>` (event dispatch) + `HashMap<NormalizedName, SmallVec<[PathId]>>` for **O(1) wiki-link name→path resolution** (separate from fuzzy ranking, per FR-017a/FR-019a).
- **Rationale**: nucleo matches on a background pool (never blocks UI), `Injector::push` is O(1) (no rescan), clears ≤100 ms p95 over 10k entries with large headroom (SC-004). Wiki-link resolution is exact/deterministic, so a plain normalized HashMap, not fuzzy.
- **Alternatives**: `fuzzy-matcher`/SkimMatcherV2 (archived 2026, single-threaded, ~6× slower) — rejected.
- **Risks/notes**: nucleo's published crate is stable-but-static (mature; pin/vendor). Item store is append-only → deletes are tombstones; compact when tombstones exceed ~25–30%. Name normalization must be shared by indexer and resolver.

### B3. File watching
- **Decision**: **`notify 8.2.0` + `notify-debouncer-full 0.7.0`** (recursive FSEvents). Debounce ~300–500 ms. Let the debouncer's `FileIdCache` correlate rename/move (inode stitching → one rename event). Self-write suppression registry layered in front.
- **Self-write suppression**: `HashMap<PathBuf, ExpectedWrite{mtime,len,expires_at}>`; after each atomic autosave rename, stat and record `(mtime,len)`; drop a debounced event iff current `(mtime,len)` matches an unexpired entry (path + identity match, not a bare time window) → genuine external edits in the same window aren't suppressed (FR-006a).
- **Move detection**: `Modify(Name(RenameMode::Both))` → one logical move; fallback pairs unmatched remove+create by basename/FileId; else degrade to delete+create (spec permits).
- **Alternatives**: `notify-debouncer-mini` (no move correlation); raw notify (no coalescing). Rejected. notify 9.0 is RC-only → stay on 8.x.
- **Risks/notes**: FSEvents is directory-granular/coalescing → treat **stat identity, not event identity, as source of truth**. Testable: `git mv a b` → exactly one rename event; an autosave + its own FSEvents notification → zero external-change callbacks (FR-006a); a 10k-file `git checkout` → bounded event count (FR-006b).

### B4. Atomic + durable writes
- **Decision**: **`tempfile 3.27.x`**: `NamedTempFile::new_in(target_dir)` → write → `file.sync_all()` → `persist(target)` (atomic rename) → open parent dir and `sync_all()` it.
- **Rationale**: `persist()` is an atomic `rename(2)` on one filesystem (readers see complete old or new, never partial — FR-009a). On Apple targets Rust std's `sync_all` already issues `F_FULLFSYNC` → true durability, no manual `fcntl`. Same-dir temp avoids cross-filesystem `PersistError` (notes may live on external/network volumes).
- **Risks/notes**: `F_FULLFSYNC` is slow → **do not fsync per keystroke**; debounce autosave (idle / few seconds) and feed the post-persist `(mtime,len)` into the watcher's self-write registry. Testable: kill between temp-write and persist → target intact (no zero-length/partial file).

### B5. AI / HTTP client (BYOM, OpenAI-compatible)
- **Decision**: **`reqwest 0.13.x`** (`stream` feature). Stream via `bytes_stream()`, line-buffer and hand-parse `data: {json}` SSE, special-casing `data: [DONE]`. Cancel/supersede via `tokio_util::sync::CancellationToken` + `tokio::select!`. Per-chunk `tokio::time::timeout` (inactivity guard) + overall deadline; **validate max input size before sending** (FR-036a). Key held in a redacting newtype (`Debug`/`Display` → `***`), set only on the `Authorization` header, never in a tracing field (NFR-006).
- **Rationale**: Manual SSE keeps control over cancellation/timeouts/lenient parsing — essential for BYOM endpoints (Ollama, llama.cpp, vLLM, LM Studio) that diverge from the exact schema. reqwest's whole-request `.timeout()` is wrong for streaming (fires mid-stream) → per-chunk timeout instead.
- **Alternatives**: `async-openai` (strict serde structs fail on non-conformant compatible servers); `reqwest-eventsource` (dormant, pins old reqwest). Rejected as primary.
- **Risks/notes**: Handle `data:` split across chunks, CRLF/LF, heartbeat/comment lines, and servers that omit `[DONE]` (clean end = done). Testable: recorded SSE with a split `data:` line → correct ordered deltas; cancel → first future resolves promptly with no further chunks; auth-error logs never contain the key.

### B6. Code-block syntax highlighting placement
- **Decision**: **Rust via `syntect 5.3.x`**, emitting classed-HTML+CSS for the WKWebView preview (and capable of `(range,style)` spans if a native path is ever needed). The Markdown parser passes the fence language → `find_syntax_by_token`; Swift never re-detects languages. **No runtime JS highlighter.**
- **Rationale**: Single source of truth (one engine, one language-detection path, one `.tmTheme`). WKWebView renders correct colors on first paint (no FOUC, no JS). syntect is line-based/stateful → editor code blocks re-highlight only edited lines (sub-ms).
- **Alternatives**: highlight.js/Prism in WKWebView (second detection engine, FOUC); Shiki (Node/WASM build toolchain); pure-Swift Splash/Highlightr (second language model). Rejected.
- **Risks/notes**: Ship a **binary `SyntaxSet`/`ThemeSet` dump** (load with lazy regex, ~23 ms at launch on a bg thread) — never parse raw YAML on the hot path (~138 ms). Trim to the supported language set. The supported set is the "20+ languages" of FR-025 (default list pinned in `research`/config; tunable later).

### B7. Error model across the boundary
- **Decision**: A `thiserror` core error hierarchy surfaced through UniFFI as a Swift `Error` (`#[derive(uniffi::Error)]`, flat where payloads aren't FFI-safe). Every fallible export returns `Result<T, EmendError>` → Swift `throws`. Variants carry the fields the UI needs (path, retry-after, byte limit, `AiCancelled`, `OversizedInput`, `PermissionDenied`, …). Panic containment in three layers: (1) UniFFI auto `catch_unwind` on every export; (2) spawned tokio task bodies wrapped in `catch_unwind` → reported as an error event, not an abort; (3) `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` in `emend-core`.
- **Rationale**: Satisfies NFR-003 (no panic across FFI, recoverable errors never abort) and gives Swift a typed, matchable surface instead of opaque strings.
- **Risks/notes**: Foreign-trait callbacks must return `Result` and implement `From<UnexpectedUniFFICallbackError>` or a Swift callback error panics. Testable: forcing a `panic!` in an export → Swift catches a thrown error, process stays alive; a panic in a spawned task → captured/reported, runtime keeps serving.

### B8. Cargo workspace layout
- **Decision**:
```
emend/
├─ Cargo.toml                # [workspace], resolver = "2"
├─ crates/
│  ├─ emend-core/            # ALL logic, NO uniffi dep — where cargo test/bench run
│  ├─ emend-ffi/             # cdylib+staticlib; thin #[uniffi::export] shims + error/handle/callback types
│  └─ emend-bench/           # Criterion benches (parse, search, highlight)
└─ scripts/build-xcframework.sh
```
`emend-core` has no FFI dependency (fast, toolchain-free `cargo test`); `emend-ffi` is the only crate depending on `uniffi`; a script builds `aarch64-apple-darwin` + runs `uniffi-bindgen-swift --xcframework`.
- **Rationale**: Keeps the whole logic layer unit/property/bench-testable with plain `cargo test` (no Swift toolchain) and keeps the panic/error boundary auditable. Apple-Silicon-only → single `aarch64-apple-darwin` target.
- **Risks/notes**: Keep `emend-ffi` genuinely thin. Pin `uniffi` and `uniffi-bindgen-swift` together. Testable: `cargo tree -p emend-core` shows no `uniffi`.

---

## C. Swift / macOS frontend decisions

### C1. Editor view technology
- **Decision**: AppKit **`NSTextView` on the TextKit 2 stack** (`NSTextLayoutManager`/`NSTextContentStorage`) wrapped in `NSViewRepresentable`. Dimmed-syntax rendering uses **display attributes, not text substitution**: style visible fragments via `NSTextContentStorageDelegate` (heading sizing, bold/italic, dimmed-marker color, code mono, `==highlight==` background) and `NSTextLayoutManager` rendering attributes only for ephemeral highlights. Markers stay in the buffer (lossless round-trip) but read as low-contrast. Task checkboxes = inline `NSTextAttachment` over the `[ ]`/`[x]` range with click hit-testing → toggle delta to Rust.
- **Rationale**: TextKit 2 lays out only on-screen fragments — the mechanism that makes ≤50 ms p95 on 10k-line docs achievable. Display-attribute styling is the only way to get "markers dimmed, not removed" in one surface.
- **Alternatives**: SwiftUI `TextEditor` (no per-range attributes/viewport hooks/attachments); TextKit 1 (no viewport-only layout, scales worse); 3rd-party **STTextView** kept as Plan B.
- **Risks/notes**: Confirmed bugs — TextKit 2 **rendering attributes can be unreliable inside `NSViewRepresentable`** (prefer the content-storage-delegate path; force `invalidateLayout`); `draw()` may be called more than expected (measure draw counts). Apply spans only for the viewport range; cache the rest for lazy application on scroll.

### C2. Preview technology
- **Decision**: **`WKWebView`** rendering comrak HTML with **bundled (vendored) Mermaid.js + KaTeX** (JS/CSS/fonts in-bundle, loaded via `loadFileURL`/local `baseURL`). Code highlighting consumed as **classed HTML + CSS** (from syntect).
- **Rationale**: Mermaid + LaTeX math are hard requirements with no native macOS equivalent; both are mature JS libs (KaTeX > MathJax for speed/determinism). One HTML pipeline also gives tables, scroll anchors, and PDF export.
- **Risks/notes**: Enforce offline/privacy (SC-008): page **CSP blocks remote loads**, `WKWebsiteDataStore.nonPersistent()`, `WKNavigationDelegate` cancels any non-`file:`/`about:` navigation. KaTeX fonts co-located with `baseURL`. Testable: Airplane Mode → Mermaid+math+table+code render, zero outbound requests.

### C3. Source ↔ preview scroll sync
- **Decision**: **Source-line anchors**: comrak annotates each top-level block with its starting source line (`data-line`). Editor→preview: top visible char → line → nearest anchor → `scrollToLine(n)` (interpolate between anchors). Preview→editor: throttled JS `scroll` posts topmost `data-line` via `WKScriptMessageHandler` → line → range → `scrollToVisible`.
- **Rationale**: Blocks map cleanly to source lines (the VS Code approach), stable and monotonic both directions; degrades gracefully via interpolation.
- **Risks/notes**: Guard the feedback loop (ignore-incoming flag/debounce on the side that just received a command). Recompute the anchor table after Mermaid/KaTeX async layout (`ResizeObserver`).

### C4. PDF export
- **Decision**: Render the preview HTML through a dedicated off-screen `WKWebView` and export via **`NSPrintOperation` (`webView.printOperation(with:)`, `showsPrintPanel=false`, `NSPrintInfo` paper/margins, save-to-PDF)** with an `@media print`/`@page` stylesheet.
- **Rationale**: Highest fidelity to the on-screen preview (Mermaid SVG, KaTeX, tables, highlighted code) and **true pagination** (SC-010).
- **Alternatives**: `WKWebView.createPDF`/`WKPDFConfiguration` — confirmed to produce a **single tall page** and ignore pagination (Apple forums 700418/705138) → kept only if a single long-scroll PDF is acceptable. `NSPrintOperation` on the NSTextView (exports the dimmed editor, not the preview); PDFKit manual layout (reimplements layout). Rejected.
- **Risks/notes**: Testable: a 30-page doc → multi-page PDF with breaks, Mermaid/math/code intact.

### C5. Keychain storage for the AI key
- **Decision**: Security framework **`SecItem*`** (`kSecClassGenericPassword`, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`) behind a tiny first-party wrapper. **Swift owns custody**; the key is read from Keychain immediately before each AI request and passed across FFI as a transient `String` — never persisted Rust-side, never logged.
- **Rationale**: ~50 lines vs a dependency; keeps the secret out of Rust's persistence/log surface (NFR-006). Rust receives it only as an ephemeral parameter for the HTTP call.
- **Alternatives**: KeychainAccess (3rd-party, lightly maintained) — convenience-only. UserDefaults/file (plaintext) — rejected. Rust `security-framework` reading Keychain — splits secret custody across FFI; rejected.
- **Risks/notes**: Redact `Authorization` in the Rust client; type the key param so it isn't `Debug`-printed. Testable: max-verbosity logs contain no key substring.

### C6. Sidebar / file tree
- **Decision**: AppKit **`NSOutlineView`** (data-source-driven, in `NSViewRepresentable`), native drag-and-drop for reorganize, `NSTableCellView` rows (icon + label).
- **Rationale**: Multiple root locations, large trees, per-row custom icons, cross-folder drag-drop — all native and scalable (view recycling, lazy children). SwiftUI `OutlineGroup`/`List` has buggy macOS drag-drop and weaker large-tree performance.
- **Risks/notes**: Do **targeted `reloadItem(_:reloadChildren:)`** on external change, never `reloadData()`. Keep expansion state in the outline view to avoid reload churn.

### C7. Tabs
- **Decision**: **In-app custom tab bar** (one `NSWindow`, one tab per open document), not native `NSWindow` tabbing, not `NSDocument`.
- **Rationale**: Full control over pinning/favorites/reorder/overflow/dirty indicators integrated with the sidebar/Quick Open UX. Native window tabs are for multi-window document merging; `NSDocument` machinery doesn't fit "plain files in arbitrary folders."
- **Risks/notes**: Reimplement ⌘W/⌘⇧[ /⌘⇧] and per-tab state (scroll/selection/undo) yourself. Testable: 20 open docs → instant switching, preserved per-tab state.

### C8. Custom folder icons (200+)
- **Decision**: **SF Symbols** primary (`Image(systemName:)`/`NSImage(systemSymbolName:)`, per-folder tint), supplemented by a **small asset catalog of custom symbol images** for gaps; picker = searchable `LazyVGrid` + color well.
- **Rationale**: 200+ pickable icons, vector/Retina, free dark-mode/tint adaptation, no asset pipeline; custom symbols render through the same path.
- **Risks/notes**: Gate symbols to the min OS (macOS 14) availability.

### C9. Consuming the Rust XCFramework
- **Decision**: Vendor the Rust core as a **`.xcframework`** consumed via a **local SwiftPM package `EmendCore`** (a source target re-exporting the UniFFI-generated Swift, depending on a `binaryTarget`). Workspace = Rust crate + build script (XCFramework + bindings) / `EmendCore` package / Xcode app target.
- **Rationale**: `binaryTarget` is the first-party reproducible way to vendor a prebuilt binary; separates generated FFI from app code and keeps the Rust build out of the app's incremental compile.
- **Risks/notes**: Compile the bindings target with `SWIFT_DEFAULT_ACTOR_ISOLATION = nonisolated` (per A1); prefer sync exports for the hot path. Testable: app builds clean under Swift 6 with the bindings target `nonisolated`; a sync delta round-trip works off the main actor without a data-race diagnostic.

### C10. Swift tooling & tests
- **Decision**: **SwiftFormat + SwiftLint** (checked-in `.swiftformat`/`.swiftlint.yml`). **XCTest** with a unit target (attribute-computation, FFI-contract mapping, Keychain wrapper, bookmark resolution, scroll-anchor math) + a UI target (XCUITest: tabs, sidebar, ⌘P, typing, export) + `measure`/`os_signpost` perf tests enforcing the 50 ms keystroke budget.
- **Rationale**: De-facto standard tooling; XCTest is first-party and required (DS-006). Perf budgets enforced in CI, not hoped for.
- **Risks/notes**: SwiftLint is community-maintained (watch item). Testing the editor: unit-test the **attribute-computation layer headlessly** (given source + spans → assert produced attributes) where the real logic lives; XCUITest only for coarse behaviors; expose AX identifiers for checkbox/marker assertions.

---

## D. Open product-config defaults (non-blocking; tune later)

| Item | Default chosen | Source |
|------|----------------|--------|
| Max supported note size (FR-027a) | Full rich editing ≤ ~5 MB; beyond → open read-only with notice | perf headroom of TextKit 2 + incremental parse |
| Max embed depth (FR-021a) | 8, with cycle detection | prevents runaway recursion |
| "20+" highlighted languages (FR-025) | syntect default set, trimmed to ~30 common langs shipped in the binary dump | B6 |
| Autosave debounce (FR-009) | flush on 1.5 s idle, hard cap 5 s; `F_FULLFSYNC` on flush/close | B4, SC-005 |
| Reading speed (FR-029) | 200 wpm; code blocks & frontmatter excluded | spec Assumptions |
| Watcher debounce (FR-006) | 400 ms | B3 |

These are recorded so `/sdd:tasks` can encode them as constants; none blocks planning. Run `/sdd:clarify` only if you want to lock different values.
