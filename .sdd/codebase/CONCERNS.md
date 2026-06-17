# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17 (US5 Phase 7 complete; embed resolution, links, tasks, attachments; runtime gaps documented)

## Executive Summary

Emend is in **Phase 2 complete, Phase 1 in progress** (Foundational complete as of 2026-06-17; US2 workspace + conflict handling complete; US4 preview + PDF export complete; US5 embeds + links + attachments complete). The core architecture is sound and security-conscious. **Phase 2 delivered**: UniFFI boundary (panic containment, error model), atomic fs writes, UTF-16 document substrate, three-pane app shell, security-scoped bookmarks, workspace model with bookmark persistence, path identity enforcement, conflict detection + resolution. **US4 (Phase 6)** added: offline Markdown preview with bundled Mermaid + KaTeX, three-layer network isolation (CSP + nonPersistent store + navigation delegate), PDF export with identical privacy guarantees, comrak HTML escaping as the trust boundary for untrusted markdown. **US5 (Phase 7)** added: wiki-link resolution, `![[embed]]` expansion with cycle + depth guards, task list syntax, and attachment storage with collision-safe naming. **Phase 1 in scope**: AI integration (key redaction, privacy tests), scroll-sync runtime validation, editor highlighting, Quick Open index maintenance, and most feature implementations remain. Key risks are around **unimplemented AI key handling** (designed but not coded), **untested self-write suppression + conflict path**, **scroll-sync runtime verification deferred** (core logic tested, runtime integration untested on real macOS events), **deferred performance regression testing** for incremental Markdown parsing on large documents, and **relative image preview not yet implemented** (attachment refs stored but not displayed in preview).

---

## Critical Security Concerns

### SEC-001: AI Key Redaction Not Yet Implemented

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-001** | `crates/emend-core/src/ai.rs` (not yet written) | AI client must redact the API key in `Authorization` header and never log it. Design is specified (research §B5) but code does not exist. | **High** | Implement per research §B5; add test `crates/emend-core/tests/ai_privacy.rs` to verify key never appears in captured logs. (Phase 1 task T112.) |

### SEC-002: Security-Scoped-Bookmark Validation Only in Signed App

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-002** | `app/Emend/Platform/SecurityScopedBookmarks.swift` + `crates/emend-core/src/fs.rs` | Security-scoped bookmarks are tested with **plain** (non-security-scoped) bookmarks in `BookmarkResolutionTests.swift` because the test process is not sandboxed. Full sandbox behavior—ensuring scope extends to Rust file IO and prevents access outside the granted folder—is only validated in the signed, notarized app. | **High** | (1) Manual testing in the signed app: add a folder, quit, relaunch, confirm reads/writes work without a new prompt. (2) Xcode simulator tests cannot reproduce sandbox constraints; App Store beta or ad-hoc signing required for full validation. (3) Document this limitation in the code review checklist. |

### SEC-003: Catalog Dependencies Inert; Feature Implementations Deferred

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-003** | `Cargo.toml` (workspace) + Phase 1 tasks | `reqwest`, `comrak`, `syntect`, `tree-sitter`, `notify`, `nucleo` are pinned but not yet imported into the code. This is by design (Phase 0 planning resolved technical unknowns; Phase 1 imports as needed), but introduces a small risk: if a crate is later imported without review, or if a new crate is added, the security implications may be overlooked. | **Medium** | Code review gate (Constitution VII / DS-006): every Phase 1 task that imports a new crate or adds a dependency MUST justify its inclusion and threat surface. Automated `cargo audit` runs pre-release. |

### SEC-004: Self-Write Suppression + Rename Correlation Untested (US2 Runtime Debt)

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-004** | `crates/emend-core/src/watcher.rs` (self-write registry, rename correlation) | The conflict handling implementation is complete in the pure logic layer (`classify`, `SuppressionRegistry`, `resolve_conflict`), but integration testing is deferred. Self-write suppression relies on post-persist `(mtime,len)` stat identity matching. Rename correlation (single `RawChange::Renamed` from debouncer's `Both` mode) is untested. Risk: if stat identity is wrong or rename is not correlated correctly, file-change loops or loss of external edits could occur. | **High** | (1) Unit tests: feed synthetic `DebouncedEvent` vectors to `classify()` with known `SuppressionRegistry` state; assert correct `RawChange` output. (2) Integration test: rapid autosaves + external edits in overlapping time windows; assert no spurious reloads. (3) Verify rename correlation: delete+create on same file → single `Renamed` event, not two separate changes. (Phase 1 tasks T065, T066.) |

### SEC-005: Live File-Watcher Integration Not Exercised by Headless Tests (US2 Runtime Debt)

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-005** | `crates/emend-core/src/watcher.rs` (FsWatcher real notify integration) + `app/Emend` (MainWindow conflict handling) | The pure conflict-detection core is unit-tested thoroughly. However, real OS filesystem events (FSEvents on macOS) are nondeterministic and directory-coalescing. The integration path—notify event → debouncer → pure classifier → UI conflict banner—is not exercised by the headless test suite. Risk: real OSEvents may exhibit timing/coalescing behavior not covered by synthetic tests. | **Medium** | (1) Manual UI testing: open a document, edit it, modify the file on disk from another tool (or via `touch -m`/`cp`), verify UI correctly detects conflict + offers reload/keep. (2) Stress test: rapid external edits (e.g., `yes "new line" >> file.md`) while buffer is dirty; verify no data loss. (3) Deferred: formal integration test harness (Phase 1 T067). |

### SEC-006: Preview Scroll-Sync Runtime Path Untested (US4 Runtime Debt)

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-006** | `app/Emend/Emend/Preview/PreviewWebView.swift` + `app/Emend/Emend/Preview/ScrollSync.swift` + `crates/emend-core/src/parse/preview.rs` | The preview pane's scroll-sync mechanism (research §C3) works as follows: comrak renders with `data-sourcepos` attributes, Rust post-processes to add `data-line`, Swift's `PreviewWebView` injects content via `window.__emendRender()`, the preview page scrolls and posts `{ line }` back to Swift via `emendScroll` message handler, and `ScrollSync` coordinates editor↔preview scroll position. The **pure core logic** (data-line anchor generation) is tested; the **runtime integration** (bridging editor scroll ↔ preview scroll via WebKit message passing) is **not exercised by headless tests**. Risk: if scroll-sync message passing fails (e.g., message name mismatch, coordinate conversion bug), the editor and preview scroll out of sync, degrading UX. | **Medium** | (1) Manual UI testing: open a multi-page document, scroll the preview, verify the editor scrolls to match and vice versa. (2) Edge cases: scroll to the bottom, top, middle; edit while scrolled to verify re-sync. (3) Verify that collapsed code blocks and long fenced blocks don't break line-number mapping. (4) Formal integration test suite deferred (Phase 1 T086). |

---

## High-Priority Technical Debt

### TD-001: FFI Range Contract (UTF-16 Code Units) Not Fully Enforced

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-001** | `crates/emend-core/src/document.rs` + `app/Emend` (editor integration) | The boundary contract states all text ranges crossing FFI are **UTF-16 code units** (research §A2). Core exports UTF-16 ranges via `U16Range` branded newtype, but Swift-to-Rust edits (per-keystroke deltas) are not yet wired (US1 Phase 3, T035). Risk: off-by-one UTF-16 errors in emoji/multi-codepoint text if callers use UTF-8 offsets by mistake. | Type safety; correctness on non-ASCII text | Medium | Phase 3 (T035 incoming) |
| **Prevention** | `U16Range` is a branded newtype; use it consistently. Add property-based tests: random UTF-8 documents → UTF-16 offset ↔ (line,col) round-trips for emoji, CJK, combining marks. | | | |

### TD-002: Incremental Parse Performance Budget Not Regression-Tested

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-002** | `crates/emend-bench/benches/smoke.rs` (planned) | Constitution Principle IV mandates ≤50 ms p95 typing latency on 1 MB docs. tree-sitter incremental reparse is chosen specifically for this (research §B1), but no benchmark exists yet to detect regressions. Adding a single character to a 10k-line doc with a complex fenced block (worst case: tail invalidation) is the test case. | Regression silently breaks the core promise; shipping a slow editor | Medium | Phase 3 (T138 proposed) |
| **Prevention** | Implement `crates/emend-bench/benches/smoke.rs` with criterion; measure: (1) single-char insert in middle of 1 MB doc, (2) fence-toggle edit invalidating a tail, (3) large paste operation. Run in CI on every commit. p95 budget is tracked (non-blocking per Constitution IV, but reviewed in pre-push). | | | |

### TD-003: Whole-Document Re-Attribution on Each Edit (US1 Phase 3)

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-003** | `crates/emend-core/src/parse.rs` (incremental highlight) + `app/Emend` (editor rendering) | US1 editor MVP re-parses the entire document on each keystroke (tree-sitter `parse()` from tree root, then filter by delta). This is functional but suboptimal for large documents. Research §B1 specifies **incremental reparse** (tree-sitter `parse(old_tree)` API), but full-sync implementation is deferred. | Latency on 100k+ line docs; noticeable stutter during sustained typing | Medium | Phase 3 Polish (T131) |
| **Prevention** | Measure p95 latency on the 1 MB worst-case doc (TD-002 bench). If > 50 ms, implement incremental reparse: pass the previous tree to `parse()` and measure delta. Trade-off: incremental is ~10% slower per-call but amortizes over sustained edits. Decision is data-driven (measure first). | | | |

### TD-004: Self-Write Suppression Logic Untested (Duplicate of SEC-004)

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-004** | `crates/emend-core/src/watcher.rs` (SuppressionRegistry) | Autosave must **not** trigger an external-change reload (FR-006a). Design uses post-persist `(mtime,len)` tuple matching + ~300 ms window, but integration testing is deferred. Risk: file-change loop (save triggers reload, user edits, save triggers reload…) if stat identity is wrong. | Data loss, UX regression, potential infinite loop | Medium | Phase 1 (T066) |
| **Prevention** | Unit test: (1) save a file, stat it, feed `(mtime,len)` to registry; (2) manually trigger FSEvents with matching `(mtime,len)` → verify event suppressed; (3) trigger with different `(mtime,len)` → verify not suppressed. Integration test: rapid autosaves + external edits in the same window → no spurious reloads. | | | |

### TD-005: Watcher Coalescing Behavior on Bulk Operations Untested

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-005** | `crates/emend-core/src/watcher.rs` (notify + debouncer-full) | FR-006b requires bounded memory/responsiveness during bulk external operations (e.g., `git checkout` on 10k files). notify coalesces at directory granularity; debouncer queues events. If many files change at once, the event queue could grow unbounded. Untested. | Memory bloat, UI freeze if not debounced correctly | Medium | Phase 1 (T065) |
| **Prevention** | Integration test: create a test workspace with 5k files; simulate `git checkout` (rapid add/delete/rename on many files); measure: (1) peak queue size, (2) time to process all events, (3) that UI remains responsive. Document max concurrent event thresholds. | | | |

### TD-006: Tolerant File Read Does Not Preserve Exact Encoding

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-006** | `crates/emend-core/src/fs.rs::read_tolerant` | To satisfy FR-003a, reads accept UTF-8 BOM, CRLF, and lossy UTF-8 decoding. On round-trip (read → edit → write), the original encoding is lost: BOM is stripped, CRLF is **preserved** (not normalized), invalid UTF-8 is replaced with U+FFFD. Files written by tools with specific encodings may degrade slightly on first save. | Subtle data change on first save after opening; user confusion if encoding was intentional | Low | Post-v1 |
| **Prevention** | Document the normalization behavior in settings ("Encoding" section). Consider a future "preserve encoding" flag if users request it. CRLF preservation is intentional (research §B4). | | | |

---

## US5 Phase 7 Known Limitations & Testing Gaps

The following are runtime concerns specific to US5 embed, link, task, and attachment functionality:

### US5-001: External-Change Embed Index Maintenance Not Implemented (FR-017a Gap)

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **US5-001** | `crates/emend-core/src/index.rs` + `crates/emend-core/src/parse/embed.rs` | The embed resolver uses the workspace index to look up note names. The index is **seeded** by full reindex (`reindex_all`) on launch/add-location and **maintained incrementally** by internal file ops (`create_note`, `rename`, `delete`, `move_node`). However, **externally-created or externally-deleted files are NOT reflected** in the index until the next full reindex. This means: (1) a note created outside Emend will not resolve in `![[…]]` until manual reindex; (2) Quick Open won't find it either; (3) a note deleted externally will remain in the index with a stale reference. | Embeds and Quick Open miss externally-created files until manual reindex. User frustration if they create a note in another editor and expect `![[NewNote]]` to work immediately. | (1) Workaround: manual reindex via menu (when available). (2) Mitigated in practice: most users create notes in Emend, not externally. | By design (deferred; Phase 1 task T068 proposed to wire `DocObserver.on_fs_change` to `index_insert`/`_remove`/`_rename` FFI methods). |

### US5-002: Relative Image Refs Don't Preview (Attachment Refs)

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **US5-002** | `crates/emend-core/src/fs.rs::store_attachment()` + `crates/emend-core/src/parse/preview.rs` + `app/Emend/Emend/Preview/PreviewWebView.swift` | When a user drops an image into a note, `store_attachment()` returns a relative path like `attachments/photo.png`. This is inserted into the Markdown as `![…](attachments/photo.png)`. comrak renders it as a literal `<img src="attachments/photo.png">`. However, the WebView's `baseURL` is the app bundle (`file:///.../Emend.app/Contents/Resources/preview/template.html`), not the note's folder. So the relative path tries to resolve relative to the bundle, failing silently. **Result**: dropped images don't appear in the preview; external links in the Markdown work (CSP allows `data:` URIs), but local image refs don't. | User drops an image, it's stored, but not visible in preview until they manually type a `file://` absolute path or embed. Looks broken but the file is safe (stored in `attachments/`). | Workaround: manually edit the image link to an absolute `file://` path (developer-facing only; user-facing feature deferred). | By design (Phase 1 follow-up, T089 proposed: construct security-scoped `file://` URLs for dropped attachments, set the WebView's `baseURL` to the note's folder, or post-process relative refs to absolute `file://` URLs). |

### US5-003: Wiki-Link Resolution Ambiguity Not Auto-Resolved on Rename

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **US5-003** | `crates/emend-core/src/index.rs` (wiki-link resolver, Phase 1 T074) | When two notes share a basename (e.g., `notes/a.md` and `archive/a.md`), a `[[a]]` link is resolved deterministically but without user feedback on which one was chosen. Tie-break rule: shortest path wins (so `archive/a.md` loses to `notes/a.md`). If the user renames `notes/a.md` → `b.md`, any link `[[a]]` that pointed to it now points to the runner-up `archive/a.md` (no auto-rewrite). | Silent link target change on rename. Links still resolve, but may point to the "wrong" note if a collision exists and the user doesn't realize it. Confusing UX. | Workaround: (1) Don't have colliding basenames. (2) Use full paths: `[[notes/a]]` is unambiguous. (3) Manual UI link update (not auto-rewrite on rename). | By design (Phase 0); Phase 1 UI enhancement (T074) could show a visual indicator on ambiguous links and offer a "rename to disambiguate" or "link context menu" action. |

---

## US4 Phase 6 Known Runtime Limitations & Testing Gaps

The following are runtime concerns specific to US4 preview/PDF functionality — the **pure core logic is tested**, but the **runtime integration is partially untested**:

### RUNTIME-006: Preview Scroll-Sync Integration Not Fully Tested (US4)

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-006** | `app/Emend/Emend/Preview/PreviewWebView.swift` + `crates/emend-core/src/parse/preview.rs` + `app/Emend/Emend/Preview/ScrollSync.swift` | The data-line anchor generation (Rust) is unit-tested (`preview_render.rs`, `preview_offline.rs`). The WebView injection (`PreviewWebView.Coordinator.render()`) is functional. The **scroll-sync message passing** (preview page posts `{ line }` via `emendScroll`, Swift processes it to scroll editor) is **not exercised by automated tests**. Edge cases: line numbering in nested blocks, fenced code blocks with long content, collapsed regions. | If scroll-sync message fails or coordinate mapping is wrong, preview and editor scrolling are out of sync, degrading UX. | Workaround: manual UI testing. No way to fully test WebKit message passing in CI (would need a running WebView). | By design (runtime integration testing deferred) |
| **Prevention** | Phase 1 task T086 (proposed): add integration tests that exercise scroll-sync end-to-end in the signed app (app-hosted tests with @testable import to drive both editor and preview). For now, manual testing is required: open a multi-page doc, scroll preview, verify editor follows; scroll editor, verify preview follows. | | | |

### RUNTIME-007: Comrak HTML Escaping Is the Trust Boundary for Untrusted Markdown

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-007** | `crates/emend-core/src/parse/preview.rs` + `app/Emend/Emend/Preview/PreviewWebView.swift` | The trust boundary for untrusted user Markdown is **comrak's HTML escaping**. User-supplied raw HTML tags (e.g., `<script>`, `<iframe>`) are entity-escaped (`<` → `&lt;`). If comrak's escaping is ever disabled (e.g., via `Options::unsafe_ = true` or a future option), or if a different rendering engine is used, raw HTML could be executed in the WebView. The Swift side (CSP + nonPersistent store + navigation delegate) provides defense-in-depth, but the core boundary is the escape. | If comrak's escaping is disabled or bypassed, malicious markdown could execute arbitrary JavaScript (XSS). | Defense-in-depth: CSP (default-src 'none') blocks most injection; nonPersistent store prevents state leakage; navigation delegate blocks navigation to external origins. But the first line of defense is comrak escaping. | By design; reviewed during US4 |
| **Prevention** | Code review gate: any change to comrak options (e.g., `Options::unsafe_` or new settings) MUST be explicitly justified and approved. Add a comment in `preview.rs` explaining why escaping is required. Consider a test that validates a malicious markdown payload renders as escaped text (not executable). | | | |

### RUNTIME-008: PDF Export Off-Screen WebView Lifecycle

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-008** | `app/Emend/Emend/Preview/PDFExport.swift` + `OffscreenPrintHost` | PDF export creates an off-screen `WKWebView`, loads the template, injects content, runs Mermaid.js (async), then calls `NSPrintOperation.runModal()` (blocking the main thread but pumping the run loop for WebKit IPC). Timeouts are 20 s (template load) + 30 s (print). If the watchdog fires, the continuation is resumed with an error, but the off-screen window and WebView are still alive (cleanup is deferred). Risk: if a user repeatedly exports while edits are happening, off-screen WebViews could accumulate until memory pressure. | Edge case: rapid successive PDF exports on large documents could leak WebViews. | Workaround: `defer { cleanup() }` ensures cleanup runs even if a timeout fires. But if the continuation is lost (e.g., Swift runtime error), cleanup might not run. | By design; timeouts are defensive |
| **Prevention** | (1) Test: open a 1000-page document, export to PDF 10 times rapidly, measure memory usage (should be constant, not growing). (2) Add @Sendable finalizer to `OffscreenPrintHost` so cleanup is guaranteed even if the continuation is dropped. (3) Monitor in field: if users report memory bloat during batch exports, investigate. | | | |

---

## US2 Phase 4 Known Runtime Limitations

The following are **by design** — deferred to Phase 1 or later, but documented here as runtime constraints that affect the correctness of the live-file-refresh path:

### RUNTIME-001: AppState Duplication (Favorites/Pins/Icons)

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-001** | `app/Emend/Emend/Sidebar/WorkspaceModel.swift` (lines 33–37) + `crates/emend-core/src/workspace.rs` | The data-model specifies app state (favorites, pins, custom folder icons) as **core-owned persistence**. Implementation: Swift-side `AppState` struct in UserDefaults, replayed into Rust core on launch via setters. The core maintains in-memory maps but has no persistence layer. On each edit (toggle favorite, set icon), Swift updates both local state and Rust via setter calls. Risk: if Rust and Swift get out of sync, the UI displays stale state. | App state is duplicated; future refactor should centralize core-side. | Setter calls are synchronous and immediate; Swift is the authoritative display source. | By design (Phase 0 / Phase 1 refactor) |
| **Prevention** | Phase 1 task (future): implement a core-owned async persistence layer (`emend-core/src/appstate.rs`) backed by a lightweight JSON file (or Keychain for sensitive data). Rust → Swift callback on changes. Then remove Swift-side duplication. | | | |

### RUNTIME-002: No FFI Getter for Folder Icon / isFavorite / isPinned

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-002** | `crates/emend-ffi/src/lib.rs` (FFI contract) + `app/Emend/Emend/Sidebar/WorkspaceModel.swift` | The FFI boundary exports setters (e.g., `set_folder_icon`, `set_favorite`) but **not getters**. Swift must independently track these values. On launch, Swift reads UserDefaults + replays state into Rust via setters. The core has the in-memory state, but Swift is the source of truth for display. | If Swift AppState is lost (e.g., corrupted UserDefaults), the UI falls back to defaults (no icons, no favorites), even though Rust has the data. | Synchronous replay on launch ensures state agreement. Swift is the authoritative display source for now. | By design (Phase 1 refactor) |
| **Prevention** | Phase 1: add FFI getters (`get_favorites()`, `get_folder_icon()`, etc.) and use them on launch to rebuild Swift state. Unifies the source of truth. | | | |

### RUNTIME-003: Conflict Banner Always Shown on Dirty External Change

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-003** | `crates/emend-core/src/watcher.rs::resolve_conflict()` + `app/Emend/Emend/Shell/MainWindow.swift` | When a file is dirty in the buffer and changes on disk, the conflict state becomes `DirtyExternalChanged` and the UI shows a reload/keep-mine banner. The core does **not** auto-reload a clean buffer (which would be silent); instead, it preserves user edits and waits for choice. This is correct behavior and prevents data loss, but it can be UX-heavy if external edits are frequent (e.g., auto-formatter running in the background). | Users see the banner every time an external tool touches the file while they're editing. No silent auto-reload; requires explicit choice. | By design (Constitution Principle III: never lose user data). Phase 1 could add a preference ("auto-reload clean files"). | Correct behavior; not a bug |

### RUNTIME-004: Self-Write Suppression Uses Stat Identity (May Miss Matches)

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-004** | `crates/emend-core/src/watcher.rs::SuppressionRegistry` | Self-write suppression compares `(mtime_ns, len)` stat tuples. On some filesystems or with rapid successive writes, `mtime` granularity may be coarse (e.g., 1 second on some volumes). If Swift writes → Rust stats → records identity, then immediately writes again before `mtime` rolls forward, the second write might not be distinguished from external changes (both have same `mtime`). Double layer of protection (Rust registry + Swift time window) mitigates this, but test coverage is incomplete (see TD-004). | Edge case: very rapid successive autosaves might not suppress the second write's event. UI might briefly show an external-change banner even though it was our own save. | (1) Rust + Swift time windows provide defense-in-depth. (2) Full integration test on Phase 1 (T066). | Design is sound; testing deferred |

### RUNTIME-005: Moving Folder Doesn't Re-Path Descendants' AppState

| ID | Area | Description | Impact | Workaround | Status |
|----|------|-------------|--------|-----------|--------|
| **RUNTIME-005** | `crates/emend-core/src/workspace.rs::move_node()` + `app/Emend/Emend/Sidebar/WorkspaceModel.swift` | When a folder is moved, Rust updates the moved item's canonical path. However, app state entries (favorites, pins, icons) are keyed by absolute path string. Descendants of the moved folder **do not** have their app state paths updated. Example: `notes/old/a.md` is favorited; user moves `notes/old/` to `notes/new/`; the old path `notes/old/a.md` stays in favorites (stale), and `notes/new/a.md` is not recognized as the same file. | Descendants lose their app state (icons, pins) after parent folder is moved. | Users must manually re-favorite/re-pin after moving a folder. Or manually edit UserDefaults (developers only). | By design (Phase 0); Phase 1 refactor will address with centralized persistence |
| **Prevention** | Phase 1: when `move_node` is called on a folder, walk its tree and update app state path keys for all descendants. Or unify persistence so paths are resolved canonically (eliminating string keys). | | | |

---

## Medium-Priority Debt

### TD-007: No Dependency Vulnerability Scanning in CI

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-007** | `Cargo.toml`, CI workflow | `cargo audit` is recommended before release but not automated in the GitHub Actions CI. Outdated/vulnerable dependencies could merge without notice. | Supply-chain risk | Low | Backlog (pre-release gate) |
| **Prevention** | Add `cargo audit --deny warnings` to the pre-push / CI gate. Consider pinning known-okay versions in `Cargo.lock` (workspace never had lock file; see TD-009). | | | |

### TD-008: No XCFramework Binary Caching in CI

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-008** | `.github/workflows/ci.yml` (planned), `scripts/build-xcframework.sh` | Rebuilding the Rust core and XCFramework on every CI run takes ~2–3 minutes. For a fast feedback loop, cache the built framework. | Slow CI feedback | Low | Backlog |
| **Prevention** | Add GitHub Actions `actions/cache` for the XCFramework (keyed by Rust toolchain hash + `Cargo.lock`). Cache key should invalidate if any Rust code changes. | | | |

### TD-009: No Cargo.lock; Transitive Dependencies Unpinned

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-009** | `Cargo.lock` (missing), workspace `[workspace.dependencies]` | Direct dependencies are pinned in `Cargo.toml`, but transitive deps are not locked. A minor version bump of a transitive dependency could silently change behavior or introduce a vulnerability across team members or CI. | Non-reproducible builds, hard to bisect, supply-chain surprise | Low | Phase 1 or pre-v1 |
| **Prevention** | Commit `Cargo.lock` to the repo. Use `cargo update --dry-run` to review transitive version changes before committing. Verify that lock updates don't violate the MSRV. | | | |

### TD-010: App Entitlements Hardcoded; No Feature Flag

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-010** | `app/Emend/Emend/Emend.entitlements` | The sandbox entitlements are hardcoded in the Xcode project. If test builds need to escape the sandbox (e.g., for system-wide directory access in integration tests), there is no configuration. | Testing friction; may require ad-hoc signing for specific test builds | Low | Post-v1 |
| **Prevention** | Document the limitation in CLAUDE.md. If needed, create a test entitlements variant (`Emend-Test.entitlements`) without sandbox for integration test builds, though this is deferred unless a real need arises. | | | |

---

## Low-Priority / Nice-to-Have Improvements

### TD-011: Error Messages Not Localized

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-011** | `crates/emend-core/src/error.rs`, Swift error rendering | Error messages (e.g., "not found: {path}") are hardcoded in English. No localization exists. v1 ships English-only; future releases could add translations. | UX polish | Low | Post-v1 |
| **Prevention** | Design a localization layer (Fluent, genstrings, or simple JSON i18n) post-v1 if user demand emerges. For now, document English-only in the release notes. | | | |

### TD-012: No Performance Monitoring / Observability

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-012** | `crates/emend-core`, `app/Emend` | Latency budgets (Constitution IV) are verified by benches + `measure` tests, but the app has no runtime observability (tracing, metrics, signposts) to diagnose user-reported slowdowns. | Hard to debug field performance issues | Low | Post-v1 |
| **Prevention** | Post-v1: add `tracing` crate + `os_signpost` integration (already used for perf tests) to instrument hot paths. Emit events; tools like Instruments can consume them. | | | |

### TD-013: Wiki-Link Ambiguity Not Resolved Deterministically in All Cases

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-013** | `crates/emend-core/src/index.rs` (Phase 1 T074) | When two notes share a basename (e.g., `notes/a.md` and `archive/a.md`), a `[[a]]` link resolution picks one arbitrarily (per design, FR-019a). The chosen note is deterministic (e.g., shortest path wins), but not explicitly documented in the code. Users might be confused if both match. | Usability: unclear which note was opened | Low | Phase 1 (T074) |
| **Prevention** | Document the resolution algorithm explicitly in `index.rs`. In the UI, mark ambiguous links visually (e.g., with a disambiguation icon). A future "rename to disambiguate" or "link context menu" could help. | | | |

### TD-014: No Pre-v1 Migration Path for Notes from Other Editors

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-014** | Outside scope (migration tooling) | Emend is a fresh app with no import/migration from Obsidian, Logseq, etc. Users manually copy files. | Adoption friction | Low | Post-v1 feature |
| **Prevention** | Document manual import steps in the quickstart. Post-v1, consider a migration guide or import wizard for popular formats. | | | |

---

## Fragile Areas (Code That Needs Careful Review)

| Area | Why Fragile | Precautions |
|------|-------------|------------|
| `crates/emend-core/src/fs.rs` | Atomic write choreography is critical to Constitution Principle III; any step (temp creation, sync, rename, dir sync) can silently break durability. | Every change must justify its atomicity contract and durability guarantees. Pair code review with `fs.rs` test inspection (lines 186–221). Always verify `File::sync_all()` is present after temp write and after directory rename. |
| `crates/emend-core/src/watcher.rs` | Pure logic (classify, suppression registry, conflict resolution) is well-tested, but FsWatcher integration with real notify events is untested. Rename correlation (debouncer `Both` mode → single `Renamed` event) is critical to avoid duplicating moves. | Every change to event classification must have corresponding unit test with synthetic `DebouncedEvent` vectors. Rename correlation tests required before Phase 1 merge. Full integration test on real macOS FSEvents strongly recommended. |
| `crates/emend-core/src/parse/embed.rs` | Embed cycle detection and depth bounding prevent infinite loops, but the resolver closure is external (provided by FFI/preview). If a resolver returns stale or circular data, embeds could expand unexpectedly. | Every change to cycle/depth logic must have unit tests with synthetic resolvers (see `tests/embeds.rs`). The FFI layer's resolver must be robust: `render_preview_html_with_embeds` supplies the resolver, and it must handle file-not-found gracefully. |
| `crates/emend-core/src/document.rs` | UTF-16 range handling with surrogate-pair validation; off-by-one errors risk silent text corruption. | Every edit operation must use `U16Range` branded newtype, never raw `u32`. Property-based tests required before Phase 3 merge (T035). Code review: check all `try_from` calls are present, never `as` casts for conversions. |
| `crates/emend-ffi/src/error.rs` | FFI error projection is exhaustive (no wildcard match). Adding a variant to `EmendError` breaks this file at compile time until mirrored. | Always mirror variants exactly. Test that projection round-trips are lossless (`FfiError::from(core_err)` preserves all fields). |
| `crates/emend-ffi/src/panic.rs` | Panic containment is the boundary between crashing FFI panics and recoverable Rust errors. `contain_panic` wraps spawned tasks; missing it → process abort. | Every `tokio::spawn` body must be wrapped. Add a comment: `// SAFETY: panic contained by contain_panic(…)` over the spawn call. Code review gate: verify no bare `spawn()` calls exist. |
| `crates/emend-core/src/parse/preview.rs` | Comrak HTML escaping is the trust boundary for untrusted user markdown. If escaping is disabled (e.g., `unsafe_ = true`) or comrak is replaced, raw HTML could be injected. | Code review gate: any change to `build_options()` must justify the change and document the security implication. Test that malicious markdown (e.g., `<script>alert(1)</script>`) renders as escaped text, not executable. |
| `app/Emend/Emend/Preview/PreviewWebView.swift` | Three-layer isolation (CSP + nonPersistent + navigation delegate) must be kept in sync. A bug in any layer could regress the privacy model. | Code review: verify CSP header is always present in template.html. Verify `config.websiteDataStore = .nonPersistent()` is never removed. Verify navigation delegate allows only `file:` + `about:`. If any of these change, update the SECURITY.md documentation. |
| `crates/emend-core/src/fs.rs::store_attachment()` | Attachment naming is collision-safe (`free_name`) and path normalization ensures portable Markdown. But attachment directory creation (`create_dir_all`) could race in theory. | Code review: verify `create_dir_all` is idempotent (returns success if dir exists). Verify that `free_name` doesn't allow directory separators in the chosen name (it uses `Path::file_name`, which is safe). Unit tests should cover: empty name fallback, collision detection, extension handling. |
| `app/Emend/Emend/Preview/PDFExport.swift` | Off-screen WebView lifecycle and timeout logic are error-prone. A bug could cause WebViews to leak or fail silently. | Code review: verify `defer { cleanup() }` is present. Verify timeouts (20 s template, 30 s print) are reasonable for the largest expected document. Test that watchdog timeout firing (e.g., with a stalled template) results in an error, not a hang. |
| `app/Emend/Platform/SecurityScopedBookmarks.swift` | Scope lifecycle (start/stop balance) is error-prone. Unbalanced calls leak scopes; missing calls allow unauthorized file access. | Code review: every `startAccessingSecurityScopedResource` must have a corresponding `stopAccessing…` in the same scope (defer or try-finally). Add a test that simulates an unbalanced call and asserts it fails gracefully. |
| `app/Emend/Sidebar/WorkspaceModel.swift` (US2 Phase 4) | Bookmark persistence, stale-bookmark refresh, scope lifecycle, and app-state duplication are intertwined. Unbalanced scope calls or losing UserDefaults state could break file access or lose favorites. | Code review: verify all `startAccessing` / `stopAccessing` calls are balanced. Verify bookmark persistence is replayed correctly on launch (test with simulator state wipe). Verify app-state setters are called synchronously after toggling favorites/pins. |
| `app/Emend/Shell/MainWindow.swift` (US1 Phase 3) | Autosave coordination + conflict detection + external file monitoring create a complex state machine. A bug cascades: stale file opened → edits trigger reload → potential data loss. | Review autosave + file-change integration together. Add integration tests: autosave while external tool modifies file → conflict resolution works, no data loss. Test plan from TD-004 / SEC-004. |
| `crates/emend-core/src/parse.rs` (Phase 1 T072) | tree-sitter incremental reparse and comrak HTML generation are two separate engines. A mismatch in Markdown interpretation could render one way in editor, another in preview. | Keep parity tests: parse the same doc in both engines, render to strings, assert visually equivalent. Don't unify the engines (Constitution principle); maintain two tests. |
| `app/Emend/Emend/Preview/PreviewWebView.swift` + `app/Emend/Emend/Preview/ScrollSync.swift` (US4) | Scroll-sync message passing (preview page posts line number to Swift, Swift scrolls editor) is functional but untested. A bug could cause editor and preview to scroll out of sync. | Manual UI testing required (Phase 1 T086). Edge cases: collapsed code blocks, nested lists, long fenced blocks. Verify line-number mapping is correct across complex document structures. |

---

## TODO Items

Active TODO comments or deferred tasks in the codebase (tracked via `/sdd:tasks`):

| Location | TODO | Priority | Task ID | Status |
|----------|------|----------|---------|--------|
| Phase 1 (T068) | Implement FFI methods `index_insert`, `index_remove`, `index_rename` and wire `DocObserver.on_fs_change` to them for external-change embed index maintenance | High | T068 | Deferred |
| Phase 1 (T110) | Implement `crates/emend-core/tests/ai_privacy.rs`: verify no network when AI unconfigured, key never in logs | High | T110 | Deferred |
| Phase 1 (T112) | Implement `crates/emend-core/src/ai.rs`: reqwest SSE, redacting key, timeout, max-input guard | High | T112 | Deferred |
| Phase 1 (T083) | Implement `crates/emend-core/tests/preview_offline.rs`: core rendering is offline (DONE); runtime WebView CSP + nonPersistent test deferred | High | T083 | Partial (core done, runtime deferred) |
| Phase 1 (T086) | Implement scroll-sync integration tests: editor↔preview scroll coordination on real documents | Medium | T086 | Deferred |
| Phase 1 (T065–T067) | Implement `crates/emend-core/src/watcher.rs` integration tests: file watching, debounce, self-write suppression, rename correlation, conflict handling | High | T065–T067 | Logic complete; integration tests deferred |
| Phase 1 (T072–T073) | Implement `crates/emend-core/src/parse.rs`: tree-sitter + comrak integration | High | T072–T073 | Comrak done (US4); tree-sitter editor highlight deferred |
| Phase 1 (T074–T076) | Implement `crates/emend-core/src/index.rs`: Quick Open, wiki-link resolution | High | T074–T076 | Deferred |
| Phase 1 (T089) | Implement local-image preview display for dropped attachments: construct security-scoped `file://` URLs or post-process relative refs to absolute paths | Medium | T089 | Deferred |
| Phase 3 (T035) | Wire Swift editor (keystroke deltas) → Rust core (push_edit) with UTF-16 ranges | High | T035 | In progress (US1) |
| Phase 3 (T131) | Viewport windowing + incremental reparse for large documents | Medium | T131 | Deferred (polish) |
| Phase 3 (T138) | Implement performance benchmark suite (`emend-bench/benches/smoke.rs`) | Medium | T138 | Deferred |

---

## External Dependency Maintenance Status

| Crate | Status | Last Verified | Notes |
|-------|--------|---------------|-------|
| `uniffi` | ✅ Maintained (Mozilla) | 2026-06-17 | 0.31.1 stable; stay on 0.31.x (0.32+ may require toolchain changes) |
| `tokio` | ✅ Active | 2026-06-17 | 1.x stable; LTS cadence; safe to update minor versions |
| `tempfile` | ✅ Maintained | 2026-06-17 | 3.x stable; limited API surface, low churn |
| `ropey` | ✅ Maintained | 2026-06-17 | 1.6.1 stable (2.0 beta intentionally excluded); watch for improvements in 2.0 |
| `thiserror` | ✅ Active | 2026-06-17 | 2.x stable (v1 frozen); modern error-handling standard |
| `tree-sitter` | ✅ Maintained (Zed) | 2026-06-17 | 0.26.x active; incremental parse API stable; unify with tree-sitter-md |
| `tree-sitter-md` | ✅ Maintained | 2026-06-17 | 0.5.x (requires tree-sitter 0.26); parser feature required |
| `comrak` | ✅ Active | 2026-06-17 | 0.52.x; CommonMark spec tracking; **HTML escaping is critical security boundary (US4/US5)** |
| `syntect` | ✅ Maintained (burntsushi) | 2026-06-17 | 5.3.x stable; theme/syntax set discovery is slow; see research §B6 binary dump approach |
| `nucleo` | ✅ Active (helix contributor) | 2026-06-17 | 0.5.x; published crate is stable; prefer vendoring if CI concerns arise |
| `notify` + `notify-debouncer-full` | ✅ Maintained | 2026-06-17 | 8.2.x + 0.7.x stable; 9.0 RC available but stay on 8.x until stabilized |
| `reqwest` | ✅ Active (Tokio org) | 2026-06-17 | 0.13.x with `stream` feature; watch for TLS upgrade breaking changes |
| `criterion` | ✅ Maintained | 2026-06-17 | 0.7.x stable (0.8+ needs 1.86); MSRV constraint enforced in CI |

---

## Improvement Opportunities

| Area | Current State | Desired State | Benefit | Effort |
|------|---------------|---------------|---------|--------|
| **AppState persistence** | Swift-side UserDefaults duplication | Core-owned async persistence layer (emend-core/src/appstate.rs) | Single source of truth; Phase 1 refactor | Medium |
| **FFI getters** | No getters for favorites/pins/icons | Expose `get_favorites()`, `get_folder_icon()`, etc. | Support full launch replay from core; Phase 1 refactor | Low |
| **Dependency pinning** | Versions in `Cargo.toml` workspace; no lock file | Add `Cargo.lock` to the repo; pin transitive deps (TD-009) | Reproducible builds; easier bisection; easier `cargo update` reviews | Low |
| **Performance observability** | Benches + measure tests only | Add `tracing` + `os_signpost` for runtime instrumentation | Diagnose field slowdowns without rebuilds | Low |
| **Error consistency** | Errors are structured but formatting is ad-hoc | Centralize error message formatting; consider structured error codes (e.g., EMEND-001) | Better logging / user docs | Low |
| **Test coverage for watcher** | Pure logic tested; real OSEvents untested | Add integration tests for file-change coalescing, self-write suppression, rename correlation (TD-004, TD-005, SEC-004) | Confidence in production file handling | Medium |
| **Incremental parse benchmark** | Manual performance measurement only | Criterion bench in CI with baseline tracking | Detect perf regressions before merge (TD-002) | Medium |
| **Move-folder re-pathing** | App state paths not updated when folder moved | Walk descendants and update app-state paths on move | Favorites/pins/icons persist after folder move (RUNTIME-005) | Low |
| **Scroll-sync integration testing** | Manual UI testing only | Formal integration tests in Phase 1 (T086) | Confidence in preview↔editor sync on real documents | Medium |
| **Preview CSP runtime validation** | Assumed working; manual testing only | Add runtime test that verifies WKWebView CSP blocks remote loads (low-level, may be difficult in CI) | Confidence that privacy layer is enforced | Medium |
| **Local-image preview** | Attachment refs stored but not displayed | Construct security-scoped `file://` URLs for dropped images (T089) | User can see dropped images in preview | Medium |
| **Embed index maintenance** | Full reindex only; external changes missed | Wire watcher → `index_insert`/`_remove`/`_rename` FFI methods (T068) | Quick Open and embeds find externally-created files immediately | Medium |

---

## Monitoring Gaps

| Area | Missing | Impact | Fix Priority |
|------|---------|--------|--------------|
| **Dependency audits** | No automated `cargo audit` in CI | Could ship with known CVEs | Pre-release gate |
| **Build reproducibility** | No lock file; transitive deps not pinned (TD-009) | Nondeterministic builds across machines | Phase 1 |
| **Performance regression detection** | Benches exist but aren't tracked over time (no baseline comparison) | Slow builds/edits merge without notice | Phase 3 Polish (T138) |
| **Integration test coverage** | Watcher, conflict, bookmark lifecycle untested with real macOS events; scroll-sync integration untested | Hard to debug file-handling bugs in field | Phase 1 (T065–T067, T086) |
| **Error telemetry** | App records errors locally but doesn't report them (by design—offline-first) | Hard to detect widespread bugs in field | Post-v1 (intentional design) |
| **Preview CSP enforcement** | Assumed enforced; no runtime verification in CI | Could regress silently if template is edited | Pre-release gate (manual testing) |
| **External index changes** | No tracking of externally-created/deleted files | Embeds + Quick Open miss external changes until reindex | Phase 1 (T068 proposed) |

---

## Concern Severity Guide

| Level | Definition | Response Time | Examples |
|-------|------------|----------------|----------|
| **Critical** | Production impact, security breach, data loss | Immediate (block merge) | Panic in FFI, atomic write bug, key leak |
| **High** | Degraded functionality, security risk, test gap for security features | Before Phase 1 merge | SEC-001, SEC-002, SEC-003, SEC-004, SEC-005, TD-001, TD-002, TD-004 |
| **Medium** | Developer experience, correctness edge case, performance concern, runtime limitations | During Phase in progress | TD-003 (perf), TD-005 (testing gap), RUNTIME-* (design constraints), SEC-006 (scroll-sync), US5-001/US5-002/US5-003 (deferred gaps) |
| **Low** | Nice to have, cosmetic, post-v1 enhancement | Backlog | TD-011–TD-014, localization, observability |

---

## What Does NOT Belong Here

- Active implementation tasks → Project board/issues / `specs/001-markdown-editor/tasks.md`
- Security controls (what we do right) → SECURITY.md
- Architecture decisions → ARCHITECTURE.md
- Code conventions → CONVENTIONS.md

---

*This document tracks what needs attention. Update when concerns are resolved or discovered.*
