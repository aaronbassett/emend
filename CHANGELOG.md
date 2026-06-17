# Changelog

All notable changes to Emend are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Entries are derived from
the project's [Conventional Commits](https://www.conventionalcommits.org/) history
(DS-007).

## [0.1.0] — 2026-06-17

Initial release of the `001-markdown-editor` feature: a quiet, native macOS
(Apple Silicon) Markdown editor with a Rust core and a Swift/SwiftUI UI. Notes are
plain files on disk; AI is bring-your-own-model (OpenAI-compatible).

### Added

- **Live, distraction-free editor (US1)** — a TextKit 2 `NSTextView` with per-keystroke
  incremental tree-sitter highlighting (dimmed syntax markers), smart list/formatting
  transforms, and debounced, atomic, durable autosave (tempfile → `F_FULLFSYNC` → rename
  → directory fsync).
- **File-based workspace (US2)** — an `NSOutlineView` sidebar over plain folders (lazy
  tree, custom folder icons, favorites/pins, drag-drop reorganize), document tabs, and
  live external-change refresh with a reload / keep-mine conflict UI. No database, no
  sync.
- **Quick Open (US3)** — ⌘P fuzzy file search over an incremental, streaming,
  supersedable index that scales to tens of thousands of files (ranked results,
  breadcrumb, Return-to-open).
- **Faithful preview + PDF export (US4)** — an offline `WKWebView` preview (comrak GFM
  HTML, syntect code highlighting, bundled Mermaid + KaTeX), bidirectional `data-line`
  scroll sync, and paginated PDF export. The preview blocks all remote loads.
- **Links, embeds, tasks & attachments (US5)** — `[[wiki-link]]` autocomplete +
  ⌘-click navigation with unresolved-link styling, clickable task checkboxes,
  `![[embed]]` inlining in the preview (cycle/depth-guarded), and image drag-drop into
  collision-safe attachments.
- **Info sidebar + BYOM AI summary (US6)** — live document stats (words/characters/
  reading time/task N-of-M) and a clickable outline, plus a streamed, cancellable AI
  summary via a user-supplied OpenAI-compatible endpoint. The API key is stored in the
  Keychain only.
- **Typography & appearance (US7)** — customizable font family, size, line height, and
  paragraph spacing applied live to both the editor and the preview (and to PDF export);
  light/dark follows the system automatically.

### Security & Privacy

- **Zero outbound network unless AI is configured *and* invoked** (SC-008): the only
  network path is the BYOM AI client, gated in code before any socket; the preview
  WebView is offline by construction (CSP `connect-src 'none'`, ephemeral data store,
  navigation-blocking delegate).
- **API key hygiene** (NFR-006): Keychain-only storage
  (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`), transient to Rust, redacted in
  logs/`Debug`/`Display`, never persisted Rust-side.
- **App Sandbox, least privilege**: user-selected file access + app-scoped bookmarks,
  and `com.apple.security.network.client` for the AI client only.
- **Crash-safe writes** (FR-009a) and a 5 MB max-note guard with a graceful refusal
  (FR-027a) that stats before allocating.

### Engineering

- **Rust core / UniFFI boundary** — all logic lives in `emend-core` (no FFI dependency,
  unit-testable in isolation); a thin `emend-ffi` UniFFI shim produces the XCFramework
  and Swift bindings, with panic containment and UTF-16 code-unit ranges across the
  boundary. MSRV 1.85.
- **Tracked performance budgets** (Principle IV) — Criterion benches for highlight
  (SC-003), Quick Open (SC-004), and document open (SC-002); see
  `specs/001-markdown-editor/perf-report.md`.
- **Headless, app-hosted test suite** (no XCUITest by design) covering editor
  persistence, workspace flows, links, preview/PDF export, Keychain, typography, and
  document-buffer release (NFR-005); a Rust core/integration suite covering parsing,
  indexing/search, the AI client, file watching, atomic IO, and the large-file cap.

### Known limitations

- External filesystem changes refresh the sidebar but do not incrementally update the
  Quick Open index until the next full reindex (FR-017a, deferred).
- For very large (~1 MB) documents the advisory highlight reparse is O(document size)
  and exceeds the SC-002/SC-003 latency budgets; typical notes are well within budget.
  See `perf-report.md` for the analysis and planned mitigation.
- Relative image references do not yet render in the preview WebView (the base URL is
  the app bundle); local-image preview is a follow-up.

[0.1.0]: https://github.com/aaronbassett/emend/releases/tag/v0.1.0
