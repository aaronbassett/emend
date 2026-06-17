# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Executive Summary

Emend is in **Phase 2 complete, Phase 1 in progress** (Foundational complete as of 2026-06-17; US1 editor MVP begun for Phase 3). The core architecture is sound and security-conscious. **Phase 2 delivered**: UniFFI boundary (panic containment, error model), atomic fs writes, UTF-16 document substrate, three-pane app shell, security-scoped bookmarks. **Phase 1 in scope**: most feature implementations (AI, preview, search, linking, export) remain to be coded. Key risks are around **unimplemented AI key handling** (designed but not coded), **unvalidated security-scoped-bookmark behavior in the signed app**, and **deferred performance regression testing** for incremental Markdown parsing on large documents. **US1 (Phase 3) status**: editor pane + smart lists drafted; whole-document re-attribution on each edit is a tracked perf concern (viewport windowing deferred).

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

### TD-004: Self-Write Suppression Logic Untested

| ID | Area | Description | Impact | Effort | Status |
|----|------|-------------|--------|--------|--------|
| **TD-004** | `crates/emend-core/src/watcher.rs` (planned: self-write registry) | Autosave must **not** trigger an external-change reload (FR-006a). Design uses post-persist `(mtime,len)` tuple matching + ~300 ms window, but the implementation is deferred and untested. Risk: file-change loop (save triggers reload, user edits, save triggers reload…) if stat identity is wrong. | Data loss, UX regression, potential infinite loop | Medium | Phase 1 (T066) |
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
| `crates/emend-core/src/document.rs` | UTF-16 range handling with surrogate-pair validation; off-by-one errors risk silent text corruption. | Every edit operation must use `U16Range` branded newtype, never raw `u32`. Property-based tests required before Phase 3 merge (T035). Code review: check all `try_from` calls are present, never `as` casts for conversions. |
| `crates/emend-ffi/src/error.rs` | FFI error projection is exhaustive (no wildcard match). Adding a variant to `EmendError` breaks this file at compile time until mirrored. | Always mirror variants exactly. Test that projection round-trips are lossless (`FfiError::from(core_err)` preserves all fields). |
| `crates/emend-ffi/src/panic.rs` | Panic containment is the boundary between crashing FFI panics and recoverable Rust errors. `contain_panic` wraps spawned tasks; missing it → process abort. | Every `tokio::spawn` body must be wrapped. Add a comment: `// SAFETY: panic contained by contain_panic(…)` over the spawn call. Code review gate: verify no bare `spawn()` calls exist. |
| `app/Emend/Platform/SecurityScopedBookmarks.swift` | Scope lifecycle (start/stop balance) is error-prone. Unbalanced calls leak scopes; missing calls allow unauthorized file access. | Code review: every `startAccessingSecurityScopedResource` must have a corresponding `stopAccessing…` in the same scope (defer or try-finally). Add a test that simulates an unbalanced call and asserts it fails gracefully. |
| `app/Emend/Shell/MainWindow.swift` (US1 Phase 3) | Bookmark refresh, stale-file detection, and autosave coordination are intertwined. A bug in one cascades (file gets opened stale, edits trigger reload, data loss). | Review autosave + file-change integration together. Add integration tests: autosave while external tool modifies file → conflict resolution works, no data loss. |
| `crates/emend-core/src/parse.rs` (Phase 1 T072) | tree-sitter incremental reparse and comrak HTML generation are two separate engines. A mismatch in Markdown interpretation could render one way in editor, another in preview. | Keep parity tests: parse the same doc in both engines, render to strings, assert visually equivalent. Don't unify the engines (Constitution principle); maintain two tests. |

---

## TODO Items

Active TODO comments or deferred tasks in the codebase (tracked via `/sdd:tasks`):

| Location | TODO | Priority | Task ID | Status |
|----------|------|----------|---------|--------|
| Phase 1 (T110) | Implement `crates/emend-core/tests/ai_privacy.rs`: verify no network when AI unconfigured, key never in logs | High | T110 | Deferred |
| Phase 1 (T112) | Implement `crates/emend-core/src/ai.rs`: reqwest SSE, redacting key, timeout, max-input guard | High | T112 | Deferred |
| Phase 1 (T083) | Implement `crates/emend-core/tests/preview_offline.rs`: WKWebView zero network access | High | T083 | Deferred |
| Phase 1 (T065–T067) | Implement `crates/emend-core/src/watcher.rs`: file watching, debounce, self-write suppression | High | T065–T067 | Deferred |
| Phase 1 (T072–T073) | Implement `crates/emend-core/src/parse.rs`: tree-sitter + comrak integration | High | T072–T073 | Deferred |
| Phase 1 (T074–T076) | Implement `crates/emend-core/src/index.rs`: Quick Open, wiki-link resolution | High | T074–T076 | Deferred |
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
| `comrak` | ✅ Active | 2026-06-17 | 0.52.x; CommonMark spec tracking; watch for breaking spec changes |
| `syntect` | ✅ Maintained (burntsushi) | 2026-06-17 | 5.3.x stable; theme/syntax set discovery is slow; see research §B6 binary dump approach |
| `nucleo` | ✅ Active (helix contributor) | 2026-06-17 | 0.5.x; published crate is stable; prefer vendoring if CI concerns arise |
| `notify` + `notify-debouncer-full` | ✅ Maintained | 2026-06-17 | 8.2.x + 0.7.x stable; 9.0 RC available but stay on 8.x until stabilized |
| `reqwest` | ✅ Active (Tokio org) | 2026-06-17 | 0.13.x with `stream` feature; watch for TLS upgrade breaking changes |
| `criterion` | ✅ Maintained | 2026-06-17 | 0.7.x stable (0.8+ needs 1.86); MSRV constraint enforced in CI |

---

## Improvement Opportunities

| Area | Current State | Desired State | Benefit | Effort |
|------|---------------|---------------|---------|--------|
| **Dependency pinning** | Versions in `Cargo.toml` workspace; no lock file | Add `Cargo.lock` to the repo; pin transitive deps (TD-009) | Reproducible builds; easier bisection; easier `cargo update` reviews | Low |
| **Performance observability** | Benches + measure tests only | Add `tracing` + `os_signpost` for runtime instrumentation | Diagnose field slowdowns without rebuilds | Low |
| **Error consistency** | Errors are structured but formatting is ad-hoc | Centralize error message formatting; consider structured error codes (e.g., EMEND-001) | Better logging / user docs | Low |
| **Test coverage for watcher** | Panic containment + atomic writes tested; watcher logic deferred | Add unit/integration tests for file-change coalescing, self-write suppression, rename correlation (TD-004, TD-005) | Confidence in production file handling | Medium |
| **Incremental parse benchmark** | Manual performance measurement only | Criterion bench in CI with baseline tracking | Detect perf regressions before merge (TD-002) | Medium |

---

## Monitoring Gaps

| Area | Missing | Impact | Fix Priority |
|------|---------|--------|--------------|
| **Dependency audits** | No automated `cargo audit` in CI | Could ship with known CVEs | Pre-release gate |
| **Build reproducibility** | No lock file; transitive deps not pinned (TD-009) | Nondeterministic builds across machines | Phase 1 |
| **Performance regression detection** | Benches exist but aren't tracked over time (no baseline comparison) | Slow builds/edits merge without notice | Phase 3 Polish (T138) |
| **Error telemetry** | App records errors locally but doesn't report them (by design—offline-first) | Hard to detect widespread bugs in field | Post-v1 (intentional design) |

---

## Concern Severity Guide

| Level | Definition | Response Time | Examples |
|-------|------------|----------------|----------|
| **Critical** | Production impact, security breach, data loss | Immediate (block merge) | Panic in FFI, atomic write bug, key leak |
| **High** | Degraded functionality, security risk, test gap for security features | Before Phase 1 merge | SEC-001, SEC-002, SEC-003, TD-001, TD-002 |
| **Medium** | Developer experience, correctness edge case, performance concern | During Phase in progress | TD-003 (perf), TD-004–TD-005 (testing gaps) |
| **Low** | Nice to have, cosmetic, post-v1 enhancement | Backlog | TD-011–TD-014, localization, observability |

---

## What Does NOT Belong Here

- Active implementation tasks → Project board/issues / `specs/001-markdown-editor/tasks.md`
- Security controls (what we do right) → SECURITY.md
- Architecture decisions → ARCHITECTURE.md
- Code conventions → CONVENTIONS.md

---

*This document tracks what needs attention. Update when concerns are resolved or discovered.*
