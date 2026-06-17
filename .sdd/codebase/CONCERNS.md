# Known Concerns

> **Purpose**: Document technical debt, known risks, bugs, fragile areas, and improvement opportunities.
> **Generated**: 2026-06-17
> **Last Updated**: 2026-06-17

## Executive Summary

Emend is in **Phase 1 of implementation** (Foundational complete as of 2026-06-17; see `specs/001-markdown-editor/retro/P2.md`). The core architecture is sound and security-conscious, but most features remain inert catalog dependencies. Key risks are around **unimplemented AI key handling** (designed but not coded), **unvalidated security-scoped-bookmark behavior in the signed app**, and **deferred performance regression testing** for incremental Markdown parsing on large documents.

---

## Critical Security Concerns

### SEC-001: AI Key Redaction Not Yet Implemented

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-001** | `crates/emend-core/src/ai.rs` (not yet written) | AI client must redact the API key in `Authorization` header and never log it. Design is specified (research §B5) but code does not exist. | **High** | Implement per research §B5; add test `crates/emend-core/tests/ai_privacy.rs` to verify key never appears in captured logs. (Deferred to Phase 1 tasks T110–T112.) |

### SEC-002: Security-Scoped-Bookmark Validation Only in Signed App

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-002** | `app/Emend/Platform/SecurityScopedBookmarks.swift` + `crates/emend-core/src/fs.rs` | Security-scoped bookmarks are tested with **plain** (non-security-scoped) bookmarks in `BookmarkResolutionTests.swift` because the test process is not sandboxed. Full sandbox behavior—ensuring scope extends to Rust file IO and prevents access outside the granted folder—is only validated in the signed, notarized app. | **High** | (1) Manual testing in the signed app: add a folder, quit, relaunch, confirm reads/writes work without a new prompt. (2) Xcode simulator tests cannot reproduce sandbox constraints; App Store beta or ad-hoc signing required for full validation. Document this in the code review checklist. |

### SEC-003: Catalog Dependencies Inert; Feature Implementations Deferred

| ID | Area | Description | Risk Level | Mitigation |
|----|------|-------------|------------|------------|
| **SEC-003** | `Cargo.toml` (workspace) | `reqwest`, `comrak`, `syntect`, `tree-sitter`, `notify`, `nucleo` are pinned but not yet imported into the code. This is by design (Phase 0 planning resolved technical unknowns; Phase 1 implements features), but introduces a small risk: if a crate is later imported without review, or if a new crate is added, the security implications may be overlooked. | **Medium** | Code review gate (Constitution VII / DS-006): every `/sdd:tasks` change to dependencies or every new `use extern_crate` statement MUST justify its inclusion. Automated `cargo audit` runs pre-release. |

---

## High-Priority Technical Debt

### TD-001: FFI Range Contract (UTF-16 Code Units) Not Fully Enforced

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-001** | `crates/emend-core/src/document.rs` + `app/Emend` (editor integration) | The boundary contract states all text ranges crossing FFI are **UTF-16 code units** (research §A2). Core exports UTF-16 ranges, but Swift-to-Rust edits (per-keystroke deltas) are not yet wired (US1 not implemented). Risk: off-by-one UTF-16 errors in emoji/multi-codepoint text if callers use UTF-8 offsets by mistake. | Type safety; correctness on non-ASCII text | Medium |
| **Prevention** | `U16Range` is a branded newtype; use it consistently. Add property-based tests: random UTF-8 documents → UTF-16 offset ↔ (line,col) round-trips for emoji, CJK, combining marks. | | |

### TD-002: Incremental Parse Performance Budget Not Regression-Tested

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-002** | `crates/emend-bench/benches/smoke.rs` (planned) | Constitution Principle IV mandates ≤50 ms p95 typing latency on 1 MB docs. tree-sitter incremental reparse is chosen specifically for this (research §B1), but no benchmark exists yet to detect regressions. Adding a single character to a 10k-line doc with a complex fenced block (worst case: tail invalidation) is the test case. | Regression silently breaks the core promise; shipping a slow editor | Medium |
| **Prevention** | Implement `crates/emend-bench/benches/smoke.rs` with criterion; measure: (1) single-char insert in middle of 1 MB doc, (2) fence-toggle edit invalidating a tail, (3) large paste operation. Run in CI on every commit. p95 budget is tracked (not hard-blocked per Constitution IV, but reviewed). | | |

### TD-003: Self-Write Suppression Logic Untested

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-003** | `crates/emend-core/src/watcher.rs` (planned: self-write registry) | Autosave must **not** trigger an external-change reload (FR-006a). Design uses post-persist `(mtime,len)` tuple matching + ~300 ms window, but the implementation is deferred and untested. Risk: file-change loop (save triggers reload, user edits, save triggers reload…) if stat identity is wrong. | Data loss, UX regression, potential infinite loop | Medium |
| **Prevention** | Unit test: (1) save a file, stat it, feed `(mtime,len)` to registry; (2) manually trigger FSEvents with matching `(mtime,len)` → verify event suppressed; (3) trigger with different `(mtime,len)` → verify not suppressed. Integration test: rapid autosaves + external edits in the same window → no spurious reloads. | | |

### TD-004: Watcher Coalescing Behavior on Bulk Operations Untested

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-004** | `crates/emend-core/src/watcher.rs` (notify + debouncer-full) | FR-006b requires bounded memory/responsiveness during bulk external operations (e.g., `git checkout` on 10k files). notify coalesces at directory granularity; debouncer queues events. If many files change at once, the event queue could grow unbounded. Untested. | Memory bloat, UI freeze if not debounced correctly | Medium |
| **Prevention** | Integration test: create a test workspace with 5k files; simulate `git checkout` (rapid add/delete/rename on many files); measure: (1) peak queue size, (2) time to process all events, (3) that UI remains responsive. Document max concurrent event thresholds. | | |

### TD-005: Tolerant File Read Does Not Preserve Exact Encoding

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-005** | `crates/emend-core/src/fs.rs::read_tolerant` | To satisfy FR-003a, reads accept UTF-8 BOM, CRLF, and lossy UTF-8 decoding. On round-trip (read → edit → write), the original encoding is lost: BOM is stripped, CRLF is normalized, invalid UTF-8 is replaced with U+FFFD. Files written by tools with specific encodings may degrade slightly. | Subtle data change on first save after opening; user confusion if encoding was intentional | Low |
| **Prevention** | Document the normalization behavior in settings ("Encoding" section). On save, preserve LF/CRLF mode of the original file (stat first load). Consider a future "preserve encoding" flag if users request it. | | |

---

## Medium-Priority Debt

### TD-006: No Dependency Vulnerability Scanning in CI

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-006** | `Cargo.toml`, CI workflow | `cargo audit` is recommended before release but not automated in the GitHub Actions CI. Outdated/vulnerable dependencies could merge without notice. | Supply-chain risk | Low |
| **Prevention** | Add `cargo audit --deny warnings` to the pre-push / CI gate. Pin known-okay versions in `Cargo.lock` (workspace never had lock file; consider adding). | | |

### TD-007: No XCFramework Binary Caching in CI

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-007** | `.github/workflows/ci.yml` (planned), `build-xcframework.sh` | Rebuilding the Rust core and XCFramework on every CI run takes ~2–3 minutes. For a fast feedback loop, cache the built framework. | Slow CI feedback | Low |
| **Prevention** | Add GitHub Actions `actions/cache` for the XCFramework (keyed by Rust toolchain hash + `Cargo.lock`). Cache key should invalidate if any Rust code changes. | | |

### TD-008: App Entitlements Hardcoded; No Feature Flag

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-008** | `app/Emend/Emend/Emend.entitlements` | The sandbox entitlements are hardcoded in the Xcode project. If test builds need to escape the sandbox (e.g., for system-wide directory access in integration tests), there is no configuration. | Testing friction; may require ad-hoc signing for specific test builds | Low |
| **Prevention** | Document the limitation in CLAUDE.md. If needed, create a test entitlements variant (`Emend-Test.entitlements`) without sandbox for integration test builds, though this is deferred unless a real need arises. | | |

---

## Low-Priority / Nice-to-Have Improvements

### TD-009: Error Messages Not Localized

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-009** | `crates/emend-core/src/error.rs`, Swift error rendering | Error messages (e.g., "not found: {path}") are hardcoded in English. No localization exists. v1 ships English-only; future releases could add translations. | UX polish | Low |
| **Prevention** | Design a localization layer (Fluent, genstrings, or simple JSON i18n) post-v1 if user demand emerges. For now, document English-only in the release notes. | | |

### TD-010: No Performance Monitoring / Observability

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-010** | `crates/emend-core`, `app/Emend` | Latency budgets (Constitution IV) are verified by benches + `measure` tests, but the app has no runtime observability (tracing, metrics, signposts) to diagnose user-reported slowdowns. | Hard to debug field performance issues | Low |
| **Prevention** | Post-v1: add `tracing` crate + `os_signpost` integration (already used for perf tests) to instrument hot paths. Emit events; tools like Instruments can consume them. | | |

### TD-011: Wiki-Link Ambiguity Not Resolved Deterministically in All Cases

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-011** | `crates/emend-core/src/index.rs` (planned) | When two notes share a basename (e.g., `notes/a.md` and `archive/a.md`), a `[[a]]` link resolution picks one arbitrarily (per design, FR-019a). The chosen note is deterministic (e.g., shortest path wins), but not explicitly documented in the code. Users might be confused if both match. | Usability: unclear which note was opened | Low |
| **Prevention** | Document the resolution algorithm explicitly in `index.rs`. In the UI, mark ambiguous links visually (e.g., with a disambiguation icon). A future "rename to disambiguate" or "link context menu" could help. | | |

### TD-012: No Pre-v1 Migration Path for Notes from Other Editors

| ID | Area | Description | Impact | Effort |
|----|------|-------------|--------|--------|
| **TD-012** | Outside scope (migration tooling) | Emend is a fresh app with no import/migration from Obsidian, Logseq, etc. Users manually copy files. | Adoption friction | Low (post-v1 feature) |
| **Prevention** | Document manual import steps in the quickstart. Post-v1, consider a migration guide or import wizard for popular formats. | | |

---

## Fragile Areas (Code That Needs Careful Review)

| Area | Why Fragile | Precautions |
|------|-------------|-------------|
| `crates/emend-core/src/fs.rs` | Atomic write choreography is critical to Constitution Principle III; any step (temp creation, sync, rename, dir sync) can silently break durability. | Every change must justify its FFI contract (UTF-16 ranges) and durability guarantees. Pair code review with `fs_atomic.rs` test inspection. |
| `crates/emend-ffi/src/error.rs` | FFI error projection is exhaustive (no wildcard match). Adding a variant to `EmendError` breaks this file at compile time until mirrored. | Always mirror variants exactly. Test that projection round-trips are lossless (`FfiError::from(core_err)` preserves all fields). |
| `crates/emend-ffi/src/panic.rs` | Panic containment is the boundary between crashing FFI panics and recoverable Rust errors. `contain_panic` wraps spawned tasks; missing it → process abort. | Every `tokio::spawn` body must be wrapped. Add a lint or comment: `// SAFETY: panic contained by contain_panic(…)` over the spawn call. |
| `app/Emend/Platform/SecurityScopedBookmarks.swift` | Scope lifecycle (start/stop balance) is error-prone. Unbalanced calls leak scopes; missing calls allow unauthorized file access. | Code review: every `startAccessingSecurityScopedResource` must have a corresponding `stopAccessing…` in the same scope (defer or try-finally). Add a test that simulates an unbalanced call and asserts it fails gracefully. |
| `app/Emend/Emend/UI/PreviewView.swift` (WKWebView CSP/offline) | If CSP or the navigation delegate is modified, remote requests could inadvertently be allowed, leaking document content. Principle II violation. | CSP rule changes require explicit approval. Navigation delegate must be tested in Airplane Mode. |

---

## TODO Items

Active TODO comments in the codebase:

| Location | TODO | Priority | Task ID |
|----------|------|----------|---------|
| `specs/001-markdown-editor/tasks.md:T110` | Implement `crates/emend-core/tests/ai_privacy.rs`: verify no network when AI unconfigured, key never in logs | High | T110 |
| `specs/001-markdown-editor/tasks.md:T112` | Implement `crates/emend-core/src/ai.rs`: reqwest SSE, redacting key, timeout, max-input guard | High | T112 |
| `specs/001-markdown-editor/tasks.md:T083` | Implement `crates/emend-core/tests/preview_offline.rs`: WKWebView zero network access | High | T083 |
| `specs/001-markdown-editor/tasks.md` (Phase 1 features) | Most AI, search, parsing, and export features deferred to Phase 1; see `tasks.md` for complete list | High | T050–T130 |

---

## External Dependency Maintenance Status

| Crate | Status | Notes |
|-------|--------|-------|
| `uniffi` | ✅ Maintained (Mozilla) | 0.31.1 stable; stay on 0.31.x (0.32+ may require toolchain changes) |
| `tokio` | ✅ Active | 1.x stable; LTS cadence; safe to update minor versions |
| `tempfile` | ✅ Maintained | 3.x stable; limited API surface, low churn |
| `ropey` | ✅ Maintained | 1.6.x; watch for Tendril deprecation (rope migration) in 2.0 |
| `thiserror` | ✅ Active | 2.x stable (v1 frozen); modern error-handling standard |
| `tree-sitter` | ✅ Maintained (Zed) | 0.25+ active; incremental parse API stable |
| `comrak` | ✅ Active | 0.52.x; CommonMark spec tracking; watch for breaking spec changes |
| `syntect` | ✅ Maintained (burntsushi) | 5.x stable; theme/syntax set discovery is slow; see §B6 binary dump approach |
| `nucleo` | ✅ Active (helix contributor) | 0.5.x; published crate is stable; prefer vendoring if CI concerns arise |
| `notify` + `notify-debouncer-full` | ✅ Maintained | 8.2.x stable; 9.0 RC available but stay on 8.x until stabilized |
| `reqwest` | ✅ Active (Tokio org) | 0.13.x with `stream` feature; watch for TLS upgrade breaking changes |

---

## Improvement Opportunities

| Area | Current State | Desired State | Benefit |
|------|---------------|---------------|---------|
| **Dependency pinning** | Versions are in `Cargo.toml` workspace; no lock file | Add `Cargo.lock` to the repo; pin transitive deps to prevent surprise breakage | Reproducible builds; easier bisection |
| **Performance observability** | Benches + measure tests only | Add `tracing` + `os_signpost` for runtime instrumentation | Diagnose field slowdowns without rebuilds |
| **Error consistency** | Errors are structured but formatting is ad-hoc | Centralize error message formatting; consider structured error codes (e.g., EMEND-001) | Better logging / user docs |
| **Test coverage for watcher** | Panic containment + atomic writes tested; watcher logic deferred | Add unit/integration tests for file-change coalescing, self-write suppression, rename correlation | Confidence in production file handling |

---

## Monitoring Gaps

| Area | Missing | Impact |
|------|---------|--------|
| **Dependency audits** | No automated `cargo audit` in CI | Could ship with known CVEs |
| **Build reproducibility** | No lock file; transitive deps not pinned | Nondeterministic builds across machines |
| **Performance regression detection** | Benches exist but aren't tracked over time (no baseline comparison) | Slow builds/edits merge without notice |
| **Error telemetry** | App records errors locally but doesn't report them (by design—offline-first) | Hard to detect widespread bugs in field |

---

## Concern Severity Guide

| Level | Definition | Response Time |
|-------|------------|----------------|
| **Critical** | Production impact, security breach, data loss | Immediate (block merge) |
| **High** | Degraded functionality, security risk, test gap for security features | This sprint / before first release |
| **Medium** | Developer experience, correctness edge case, performance concern | Next sprint / before broader testing |
| **Low** | Nice to have, cosmetic, post-v1 enhancement | Backlog |

---

## What Does NOT Belong Here

- Active implementation tasks → Project board/issues
- Security controls (what we do right) → SECURITY.md
- Architecture decisions → ARCHITECTURE.md
- Code conventions → CONVENTIONS.md

---

*This document tracks what needs attention. Update when concerns are resolved or discovered.*
