---
description: "Task list for Emend — native macOS Markdown editor (001-markdown-editor)"
---

# Tasks: Emend — A Quiet, Native macOS Markdown Editor

**Input**: Design documents from `/specs/001-markdown-editor/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/ffi-interface.md, `.sdd/memory/constitution.md`

**Tests**: INCLUDED. The constitution (Principle VII) mandates **strict testing for the Rust core (test-first)** and **pragmatic testing for the Swift UI** (headless logic + key flows). Core test tasks precede their implementation; Swift UI test tasks target the headless attribute/logic layer plus coarse XCUITest flows.

**Organization**: Tasks are grouped by user story (spec.md priorities) so each story is an independently testable increment.

## Format: `[ID] [P?] [Story] Description (use <agent>)`

- **[P]**: Parallelizable (different files, no incomplete dependencies)
- **[Story]**: US1–US7 (user-story phases only). `[GIT]` = git workflow op.
- Rust tasks reference `devs:rust-dev`. Swift/AppKit tasks have no mapped agent (implement directly).

## Path Conventions

- Rust core: `crates/emend-core/src/...`, tests `crates/emend-core/tests/...`, benches `crates/emend-bench/benches/...`
- FFI shim: `crates/emend-ffi/src/...`
- Swift core pkg: `swift/EmendCore/Sources/...`, `swift/EmendCore/Tests/...`
- macOS app: `app/Emend/Emend/...`, tests `app/Emend/EmendTests/...`, UI tests `app/Emend/EmendUITests/...`

## Git Workflow Note

The GitHub remote `origin` exists; branch `001-markdown-editor` is already created and rebased onto `main` (two scaffold commits). So the "create branch" setup steps are **already done**. Commit after each logical group; pre-commit/pre-push hooks (lefthook) MUST pass (never `--no-verify`). Each phase ends by pushing and opening/updating the PR, then waiting for green CI before continuing.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Finish project initialization beyond the `/sdd:plan` scaffold (which already created the Cargo workspace, Swift `EmendCore` package, tooling configs, lefthook hooks, and CI).

- [x] T001 [GIT] Verify branch `001-markdown-editor` is based on `origin/main` and the working tree is clean (`git status`, `git log --oneline`)
- [x] T002 Install Swift tooling locally: `brew install swiftformat swiftlint`; verify `just swift-lint` runs (per DS-002)
- [x] T003 [P] Add `crates/emend-bench` crate (Criterion) to the workspace `Cargo.toml` members for perf benches (use devs:rust-dev agent)
- [x] T004 [P] Create the Xcode macOS app target at `app/Emend/` (SwiftUI lifecycle, deployment macOS 14, arch arm64); add the local `swift/EmendCore` package as a dependency
- [x] T005 Configure App Sandbox entitlements in `app/Emend/Emend/Emend.entitlements`: `com.apple.security.app-sandbox`, `...files.user-selected.read-write`, `...files.bookmarks.app-scope` (research §A4)
- [x] T006 [P] Vendor offline preview assets into `app/Emend/Emend/Resources/preview/` (Mermaid.js, KaTeX JS/CSS/fonts, `theme.css`, `template.html`) with a CSP that blocks remote loads (research §C2)
- [x] T007 [GIT] Commit: add bench crate, Xcode app target, entitlements, and bundled preview assets
- [x] T008 [GIT] Push branch to origin (ensure pre-push hooks pass) and open the PR to `main` titled "Emend: Setup complete"
- [x] T009 [GIT] Verify all CI checks pass; report PR ready status

**Checkpoint**: Both toolchains build; app target exists; assets bundled.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The FFI boundary, the core document/IO substrate, and the app shell — shared by every story.

**⚠️ CRITICAL**: No user-story work begins until this phase is complete.

- [x] T010 Create `specs/001-markdown-editor/retro/P2.md` from the retro template
- [x] T011 [GIT] Commit: initialize Phase 2 retro

### FFI boundary & error model

- [x] T012 Wire UniFFI in `crates/emend-ffi`: add `uniffi.workspace = true` + build dep, call `uniffi::setup_scaffolding!()`, export `core_abi_version` (use devs:rust-dev agent)
- [x] T013 Migrate `crates/emend-core/src/error.rs` `EmendError` to `thiserror`; add its `#[derive(uniffi::Error)]` projection in `crates/emend-ffi/src/error.rs` with all variants from contracts/ffi-interface.md (use devs:rust-dev agent)
- [x] T014 [P] Implement panic-containment posture in `crates/emend-ffi/src/lib.rs`: `catch_unwind` wrappers for spawned tokio tasks; document UniFFI's per-export containment (NFR-003) (use devs:rust-dev agent)
- [x] T015 [P] [test] Unit test in `crates/emend-ffi/tests/panic_containment.rs`: a forced `panic!` in an export surfaces as `EmendError`, process survives (use devs:rust-dev agent)
- [x] T016 Make `scripts/build-xcframework.sh` produce real bindings + `EmendCore.xcframework`; enable the `binaryTarget`/`EmendCoreFFI` target in `swift/EmendCore/Package.swift` with `SWIFT_DEFAULT_ACTOR_ISOLATION = nonisolated` (research §A1/§C9)
- [x] T017 [GIT] Commit: UniFFI wiring, error model, panic containment, XCFramework build

### Core text document substrate (UTF-16 boundary)

- [x] T018 [P] [test] Tests in `crates/emend-core/tests/document.rs`: UTF-16 offset↔(line,col) mapping incl. astral chars; `push_edit` delta application (use devs:rust-dev agent)
- [x] T019 Implement `crates/emend-core/src/document.rs`: shadow rope + UTF-16 line/offset index, `open_document`/`close_document`/`push_edit` (sync, non-blocking) returning a doc handle (research §A2/§A3) (use devs:rust-dev agent)
- [x] T020 [GIT] Commit: core document session with UTF-16 mapping

### Atomic file IO + tolerant reads

- [x] T021 [P] [test] Tests in `crates/emend-core/tests/fs_atomic.rs`: kill-between-temp-and-persist leaves target intact; reader never sees partial; BOM/CRLF/non-UTF-8 read tolerantly (FR-003a/FR-009a) (use devs:rust-dev agent)
- [x] T022 Implement `crates/emend-core/src/fs.rs`: atomic+durable write (tempfile→`sync_all`→persist→dir fsync, `F_FULLFSYNC`) and tolerant read (research §B4) (use devs:rust-dev agent)
- [x] T023 [GIT] Commit: atomic durable writes + tolerant reads

### Runtime, cancellation, streaming scaffolding

- [x] T024 Implement the tokio runtime + cancellation handle pattern (`CancellationToken`) and foreign-trait sink scaffolding (`SearchSink`, `AiSink`, `DocObserver`) in `crates/emend-ffi/src/handles.rs` (research §A1/§B7) (use devs:rust-dev agent)
- [x] T025 [P] Implement Swift wrappers in `swift/EmendCore/Sources/EmendCore/`: error mapping + `AsyncStream` adapters over the foreign-trait sinks
- [x] T026 [GIT] Commit: cancellation handles, streaming sinks, Swift adapters

### App shell + sandbox handshake (highest risk — prototype first)

- [x] T027 Implement `app/Emend/Emend/EmendApp.swift` (single window) + `app/Emend/Emend/Shell/MainWindow.swift` (sidebar | editor | info three-pane skeleton)
- [x] T028 Implement `app/Emend/Emend/Platform/SecurityScopedBookmarks.swift`: NSOpenPanel → bookmark create/persist/resolve (stale handling); prototype the scope↔Rust file-IO handshake end-to-end (research §A4)
- [x] T029 [P] [test] `app/Emend/EmendTests/BookmarkResolutionTests.swift`: bookmark round-trip resolve + stale re-create
- [x] T030 [GIT] Commit: app shell + security-scoped bookmark handshake

### Phase close

- [x] T031 Run `/sdd:map incremental` for Phase 2 changes; commit updated `.sdd/codebase/` docs
- [x] T032 Review `retro/P2.md`; extract only critical, project-wide learnings to `CLAUDE.md` (conservative)
- [x] T033 [GIT] Commit: Phase 2 codebase map + retro
- [x] T034 [GIT] Push; create/update PR with Phase 2 summary; verify CI green; report PR ready status

**Checkpoint**: Foundation ready — user stories can begin.

---

## Phase 3: User Story 1 — Live, distraction-free editor (Priority: P1) 🎯 MVP

**Goal**: Open a `.md` file and edit it with dimmed-syntax live rendering, smart lists, formatting shortcuts, and atomic autosave.

**Independent Test**: Open a single file, type headings/bold/italic/lists, confirm markers dim while formatting renders inline, confirm smart-list renumber, quit/reopen → on-disk Markdown is clean and correct.

- [x] T035 [US1] Create `specs/001-markdown-editor/retro/P3.md`; [GIT] commit
- [x] T036 [P] [US1] [test] `crates/emend-core/tests/parse_incremental.rs`: tree-sitter `changed_ranges` is edit-local; fence-toggle invalidates the tail (use devs:rust-dev agent)
- [x] T037 [P] [US1] [test] `crates/emend-bench/benches/highlight.rs`: re-highlight one edited line in a 1MB doc < 5ms (tracked budget, SC-003) (use devs:rust-dev agent)
- [x] T038 [US1] Implement `crates/emend-core/src/parse/highlight.rs`: tree-sitter + tree-sitter-md incremental editor highlight; `highlight_spans(viewport)` returns `(U16Range, StyleClass)` (research §B1) (use devs:rust-dev agent)
- [x] T039 [US1] Export `open_document`/`push_edit`/`highlight_spans` in `crates/emend-ffi/src/lib.rs` per the FFI contract (use devs:rust-dev agent)
- [x] T040 [GIT] Commit: incremental editor highlighting + FFI exports
- [x] T041 [P] [US1] [test] `app/Emend/EmendTests/SyntaxAttributingTests.swift` (headless): given source + spans → assert dimmed-marker ranges + heading fonts (no window)
- [x] T042 [US1] Implement `app/Emend/Emend/Editor/SyntaxAttributing.swift`: map core spans → display attributes (dim markers, inline bold/italic/heading/quote/list, `==highlight==` background)
- [x] T043 [US1] Implement `app/Emend/Emend/Editor/MarkdownEditorView.swift` (NSViewRepresentable over TextKit 2 `NSTextView`); apply attributes via `NSTextContentStorageDelegate` for the viewport range (research §C1)
- [x] T044 [GIT] Commit: TextKit 2 editor view with dimmed-syntax rendering
- [x] T045 [P] [US1] Implement smart lists (auto-renumber/indent/outdent) in `app/Emend/Emend/Editor/SmartLists.swift`
- [x] T046 [P] [US1] Implement formatting shortcuts (bold/italic/link/task) in `app/Emend/Emend/Editor/FormattingCommands.swift`
- [x] T047 [US1] Wire debounced atomic autosave (core `flush`) + self-write suppression in `app/Emend/Emend/Editor/AutosaveController.swift` (FR-009/FR-006a) (use devs:rust-dev agent for the core `flush` export)
- [x] T048 [GIT] Commit: smart lists, shortcuts, autosave
- [x] T049 [US1] [test] `app/Emend/EmendTests/EditorPersistenceTests.swift`: drive the real editor coordinator + autosave so typed edits round-trip to disk through the core (headless app-hosted test; XCUITest dropped — its runner cannot bootstrap under CI's `CODE_SIGNING_ALLOWED=NO`, and the project's CI is GUI/signing-free by design, Constitution VII)
- [x] T050 [US1] Run `/sdd:map incremental`; review `retro/P3.md` → CLAUDE.md (conservative); [GIT] commit
- [x] T051 [GIT] Push; create/update PR "US1 (MVP): live editor"; verify CI green; report PR ready status — PR #3 squash-merged to main (code review resolved, CI green)

**Checkpoint**: 🎯 MVP — Emend can open, edit (dimmed syntax + smart lists + shortcuts), and autosave a Markdown file.

---

## Phase 4: User Story 2 — File-based workspace (Priority: P1)

**Goal**: Add folder "locations", browse them in a sidebar tree, open files in tabs, do file ops, reorganize, favorite/pin, custom icons, and live-refresh on external change.

**Independent Test**: Add a nested folder, tree renders, open in a tab, rename/move via drag-drop, edit a file externally → app refreshes without manual reload.

- [x] T052 [US2] Create `retro/P4.md`; [GIT] commit
- [x] T053 [P] [US2] [test] `crates/emend-core/tests/watcher.rs`: `git mv` → one rename event; autosave → zero external-change callbacks; 10k-file burst is bounded (FR-006a/b) (use devs:rust-dev agent)
- [x] T054 [P] [US2] [test] `crates/emend-core/tests/workspace_ops.rs`: collision-safe create/rename/move; conflict truth table (clean→reload, dirty→preserve) (FR-004a/FR-006c) (use devs:rust-dev agent)
- [x] T055 [P] [US2] [test] `crates/emend-core/tests/index.rs`: single create/rename/delete updates the index in O(1), no full rescan (FR-017a) (use devs:rust-dev agent)
- [x] T055a [P] [US2] [test] `crates/emend-core/tests/concurrency.rs`: parallel watcher events + user create/rename/delete + search queries leave the index/workspace model consistent — no corruption, no panic (NFR-004) (use devs:rust-dev agent)
- [x] T055b [P] [US2] [test] `crates/emend-core/tests/path_identity.rs`: traversal terminates on a symlink cycle; the same physical file via two paths is indexed once; correct behavior on case-insensitive and case-sensitive volumes (NFR-007) (use devs:rust-dev agent)
- [x] T056 [US2] Implement `crates/emend-core/src/workspace.rs`: locations add/remove/list, `list_children`, file ops, favorites/pins/icons/child-order store; canonicalize paths and bound traversal depth for symlink-cycle/case-fold safety (NFR-007) (use devs:rust-dev agent)
- [x] T057 [US2] Implement `crates/emend-core/src/watcher.rs`: notify + debouncer-full, self-write suppression registry, move detection, conflict state (research §B3) (use devs:rust-dev agent)
- [x] T058 [US2] Implement `crates/emend-core/src/index.rs`: nucleo haystack + pathMap + nameMap; incremental updates (research §B2) (use devs:rust-dev agent)
- [x] T059 [US2] Export workspace/watcher/file-op functions + `DocObserver`/conflict APIs in `crates/emend-ffi/src/lib.rs` (use devs:rust-dev agent)
- [x] T060 [GIT] Commit: core workspace, watcher, index + FFI
- [x] T061 [US2] Implement `app/Emend/Emend/Sidebar/WorkspaceOutlineView.swift` (NSOutlineView, targeted `reloadItem`) with add-location via NSOpenPanel (research §C6)
- [x] T062 [P] [US2] Implement `app/Emend/Emend/Sidebar/FolderIconPicker.swift` (SF Symbols grid + tint) and favorites/pins rows
- [x] T063 [P] [US2] Implement sidebar drag-drop reorganize in `app/Emend/Emend/Sidebar/OutlineDragDrop.swift`
- [x] T064 [US2] Implement tabs: `app/Emend/Emend/Tabs/TabModel.swift` + `TabBarView.swift` (open file in tab, per-tab state) (research §C7)
- [x] T065 [US2] Wire live refresh + conflict UI (reload vs keep-mine) in `app/Emend/Emend/Editor/ConflictController.swift`
- [x] T066 [GIT] Commit: sidebar, icons, drag-drop, tabs, live refresh
- [x] T067 [US2] [test] `app/Emend/EmendUITests/WorkspaceFlowTests.swift`: add folder → tree → open tab → rename
- [x] T068 [US2] Run `/sdd:map incremental`; review `retro/P4.md` → CLAUDE.md; [GIT] commit
- [x] T069 [GIT] Push; PR "US2: workspace"; verify CI green; report PR ready status

**Checkpoint**: US1 + US2 work independently — full editing + browsing.

---

## Phase 5: User Story 3 — Quick Open (Priority: P2) — depends on US2

**Goal**: ⌘P fuzzy-search files/folders across the workspace, ranked with breadcrumbs, open with Return.

**Independent Test**: In a large workspace, ⌘P + fuzzy query → ranked results with paths → open top hit.

- [x] T070 [US3] Create `retro/P5.md`; [GIT] commit
- [x] T071 [P] [US3] [test] `crates/emend-bench/benches/quick_open.rs`: p95 ≤100ms over 10k entries (warm) (SC-004) (use devs:rust-dev agent)
- [x] T072 [P] [US3] [test] `crates/emend-core/tests/search_supersede.rs`: superseding a query cancels prior emission (NFR-002) (use devs:rust-dev agent)
- [x] T073 [US3] Implement `crates/emend-core/src/search.rs` + `quick_open_query` streaming via `SearchSink`, supersede/cancel (use devs:rust-dev agent)
- [x] T074 [US3] Export `quick_open_query`/`SearchHandle` in `crates/emend-ffi/src/lib.rs` (use devs:rust-dev agent)
- [x] T075 [GIT] Commit: core Quick Open search + FFI
- [x] T076 [US3] Implement `app/Emend/Emend/QuickOpen/QuickOpenView.swift` (⌘P overlay, ranked rows + breadcrumb, Return-to-open, supersede on keystroke)
- [x] T077 [GIT] Commit: Quick Open overlay
- [x] T078 [US3] [test] `app/Emend/EmendTests/QuickOpenTests.swift`: ⌘P → query → open result (headless app-hosted; XCUITest can't bootstrap under CODE_SIGNING_ALLOWED=NO)
- [x] T079 [US3] Run `/sdd:map incremental`; review `retro/P5.md` → CLAUDE.md; [GIT] commit
- [x] T080 [GIT] Push; PR "US3: Quick Open"; verify CI green; report PR ready status (merged as #5)

**Checkpoint**: Quick Open works across the workspace.

---

## Phase 6: User Story 4 — Faithful preview + PDF export (Priority: P2)

**Goal**: WKWebView preview (highlighted code, tables, Mermaid, math), synced scroll, export to PDF.

**Independent Test**: Doc with code/table/Mermaid/math renders; scroll sync both ways; export → multi-page PDF matching the preview.

- [x] T081 [US4] Create `retro/P6.md`; [GIT] commit
- [x] T082 [P] [US4] [test] `crates/emend-core/tests/preview_render.rs`: comrak HTML has `data-line` anchors + syntect classed code; tables render (use devs:rust-dev agent)
- [x] T083 [P] [US4] [test] `crates/emend-core/tests/preview_offline.rs`: rendering performs zero network access (SC-008) (use devs:rust-dev agent)
- [x] T083a [US4] Generate & vendor the binary syntect `SyntaxSet`/`ThemeSet` dump for the 30-language v1 set (research §D) into `crates/emend-core/assets/`; assert lazy load ≤23ms at startup, never raw-YAML on the hot path (research §B6) (use devs:rust-dev agent)
- [x] T084 [US4] Implement `crates/emend-core/src/parse/preview.rs` (comrak + line anchors) and `crates/emend-core/src/parse/code_highlight.rs` (syntect classed HTML, lazy binary-dump load from T083a) (research §B1/§B6) (use devs:rust-dev agent)
- [x] T085 [US4] Export `render_preview_html`/`preview_assets_dir` in `crates/emend-ffi/src/lib.rs` (use devs:rust-dev agent)
- [x] T086 [GIT] Commit: core preview rendering + FFI
- [x] T087 [US4] Implement `app/Emend/Emend/Preview/PreviewWebView.swift` (WKWebView, offline CSP, nonPersistent store, navigation-blocking delegate, Mermaid/KaTeX) (research §C2)
- [x] T088 [P] [US4] Implement `app/Emend/Emend/Preview/ScrollSync.swift` (bidirectional `data-line` anchor sync, feedback-loop guard) (research §C3)
- [x] T089 [US4] Implement `app/Emend/Emend/Preview/PDFExport.swift` via `NSPrintOperation` on an offscreen WKWebView (paginated, `@media print`) (research §C4) — uses async `runModal(for:…)` (synchronous `run()` deadlocks the main run loop with WebKit IPC)
- [x] T090 [GIT] Commit: preview WebView, scroll sync, PDF export
- [x] T091 [US4] [test] `app/Emend/EmendTests/PreviewExportTests.swift`: render sample doc, export, assert multi-page PDF (app-hosted/headless, not XCUITest — same rationale as other flows; the `EmendUITests` target doesn't exist by design)
- [ ] T092 [US4] Run `/sdd:map incremental`; review `retro/P6.md` → CLAUDE.md; [GIT] commit
- [ ] T093 [GIT] Push; PR "US4: preview + PDF"; verify CI green; report PR ready status

**Checkpoint**: Preview + PDF export work.

---

## Phase 7: User Story 5 — Link and connect notes (Priority: P2) — depends on US2 (index) + US4 (preview)

**Goal**: `[[wiki links]]` with autocomplete + resolution, `![[embeds]]`, clickable task checkboxes, `==highlight==`, drag-drop images.

**Independent Test**: `[[` autocompletes with paths; click navigates; `![[embed]]` inlines in preview; checkbox click toggles `[ ]`/`[x]`.

- [ ] T094 [US5] Create `retro/P7.md`; [GIT] commit
- [ ] T095 [P] [US5] [test] `crates/emend-core/tests/links.rs`: deterministic resolution for duplicate basenames; rename leaves old links unresolved (FR-019a) (use devs:rust-dev agent)
- [ ] T096 [P] [US5] [test] `crates/emend-core/tests/embeds.rs`: embed cycle terminates within max depth (FR-021a) (use devs:rust-dev agent)
- [ ] T097 [US5] Implement `crates/emend-core/src/derived.rs` link/task extraction + `resolve_wikilink`/`wikilink_suggestions`/`toggle_task`; embed resolution with cycle/depth guard in `parse/embed.rs` (use devs:rust-dev agent)
- [ ] T098 [US5] Implement `store_attachment` (collision-safe, untitled fallback) in `crates/emend-core/src/fs.rs` (FR-013a); export link/task/attachment APIs in `crates/emend-ffi/src/lib.rs` (use devs:rust-dev agent)
- [ ] T099 [GIT] Commit: core links, embeds, tasks, attachments + FFI
- [ ] T100 [US5] Implement `app/Emend/Emend/Links/WikiLinkAutocomplete.swift` (live `[[` dropdown with paths) + clickable link navigation
- [ ] T101 [P] [US5] Implement clickable task checkbox attachment + toggle in `app/Emend/Emend/Editor/TaskCheckbox.swift` (FR-014); unresolved-link styling
- [ ] T102 [P] [US5] Implement embed rendering in preview + inline image drag-drop in `app/Emend/Emend/Editor/ImageDrop.swift`
- [ ] T103 [GIT] Commit: wiki-link autocomplete, checkboxes, embeds, image drop
- [ ] T104 [US5] [test] `app/Emend/EmendUITests/LinksFlowTests.swift`: autocomplete → click navigate → checkbox toggle
- [ ] T105 [US5] Run `/sdd:map incremental`; review `retro/P7.md` → CLAUDE.md; [GIT] commit
- [ ] T106 [GIT] Push; PR "US5: links & embeds"; verify CI green; report PR ready status

**Checkpoint**: Linking, embeds, tasks, and image drop work.

---

## Phase 8: User Story 6 — Info sidebar + AI summary (Priority: P3)

**Goal**: Info sidebar with live stats, task completion, clickable outline, and an on-demand BYOM AI summary. No network unless AI configured + invoked.

**Independent Test**: Info sidebar shows stats/tasks/outline live; with no AI config nothing is sent externally; configure an OpenAI-compatible model → summary appears.

- [ ] T107 [US6] Create `retro/P8.md`; [GIT] commit
- [ ] T108 [P] [US6] [test] `crates/emend-core/tests/derived_stats.rs`: word/char/reading-time + N-of-M tasks + outline; live update ≤300ms (FR-029/030/031a) (use devs:rust-dev agent)
- [ ] T109 [P] [US6] [test] `crates/emend-core/tests/ai_sse.rs`: SSE parse with `data:` split across chunks + `[DONE]`; cancel stops emission; oversized input rejected before send (FR-036a) (use devs:rust-dev agent)
- [ ] T110 [P] [US6] [test] `crates/emend-core/tests/ai_privacy.rs`: no network when unconfigured; key never appears in captured logs (SC-008/NFR-006) (use devs:rust-dev agent)
- [ ] T111 [US6] Implement `outline`/`stats`/`links` + `DocObserver` live push in `crates/emend-core/src/derived.rs` (use devs:rust-dev agent)
- [ ] T112 [US6] Implement `crates/emend-core/src/ai.rs`: reqwest SSE streaming, `CancellationToken`, timeout, max-input guard, redacting key newtype; `summarize_document`/`test_ai_config` (research §B5) (use devs:rust-dev agent)
- [ ] T113 [US6] Export AI + derived APIs (with `AiSink`) in `crates/emend-ffi/src/lib.rs` (use devs:rust-dev agent)
- [ ] T114 [GIT] Commit: core derived data + AI client + FFI
- [ ] T115 [US6] Implement `app/Emend/Emend/Info/InfoSidebarView.swift` (stats, task completion, live clickable outline → scroll)
- [ ] T116 [US6] Implement `app/Emend/Emend/Platform/KeychainStore.swift` (SecItem wrapper) + `app/Emend/Emend/AI/AISettingsView.swift` (baseURL/model/key→Keychain, test config) (research §C5)
- [ ] T117 [US6] Implement `app/Emend/Emend/AI/SummaryView.swift` (streamed summary, cancel/supersede, error states)
- [ ] T118 [GIT] Commit: info sidebar, AI settings (Keychain), summary UI
- [ ] T119 [US6] [test] `app/Emend/EmendTests/KeychainStoreTests.swift` (headless) + `app/Emend/EmendUITests/InfoSidebarTests.swift`
- [ ] T120 [US6] Run `/sdd:map incremental`; review `retro/P8.md` → CLAUDE.md; [GIT] commit
- [ ] T121 [GIT] Push; PR "US6: info sidebar + AI"; verify CI green; report PR ready status

**Checkpoint**: Document insight + BYOM AI summary work; privacy preserved.

---

## Phase 9: User Story 7 — Typography & appearance (Priority: P3)

**Goal**: Curated typography with user customization (font/size/spacing) applied to editor + preview; follow native light/dark.

**Independent Test**: Change font/size/spacing → editor + preview update; switch system appearance → app follows.

- [ ] T122 [US7] Create `retro/P9.md`; [GIT] commit
- [ ] T123 [P] [US7] [test] `crates/emend-core/tests/settings.rs`: typography settings persist + round-trip (use devs:rust-dev agent)
- [ ] T124 [US7] Implement `crates/emend-core/src/settings.rs` get/set + export in `crates/emend-ffi/src/lib.rs` (use devs:rust-dev agent)
- [ ] T125 [US7] Implement `app/Emend/Emend/Settings/TypographySettingsView.swift`; apply to editor + preview; bind light/dark to system
- [ ] T126 [GIT] Commit: typography settings (core + UI)
- [ ] T127 [US7] [test] `app/Emend/EmendUITests/TypographyTests.swift`: change font → editor + preview reflect
- [ ] T128 [US7] Run `/sdd:map incremental`; review `retro/P9.md` → CLAUDE.md; [GIT] commit
- [ ] T129 [GIT] Push; PR "US7: typography"; verify CI green; report PR ready status

**Checkpoint**: All seven user stories independently functional.

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: Quality, performance, and verification across stories.

- [ ] T130 Create `retro/P10.md`; [GIT] commit
- [ ] T131 [P] Run all perf benches + Swift `measure` tests; record results vs budgets (SC-002/003/004) in `specs/001-markdown-editor/perf-report.md`; review regressions (tracked, non-blocking per Principle IV) (use devs:rust-dev agent)
- [ ] T132 [P] Verify bounded memory (NFR-005): closing a tab releases the document buffer — add `app/Emend/EmendTests/MemoryReleaseTests.swift`
- [ ] T133 [P] Large-file handling (FR-027a): max-size read-only fallback test in `crates/emend-core/tests/large_file.rs` (use devs:rust-dev agent)
- [ ] T134 [P] Add accessibility identifiers across editor/sidebar/quick-open for XCUITest stability
- [ ] T135 Security review pass against Principles II/III: audit every outbound call, key handling, and atomic-write path; document in `specs/001-markdown-editor/security-review.md`
- [ ] T136 [P] Run `quickstart.md` validation end-to-end; fix any drift
- [ ] T137 [GIT] Commit: performance report, memory/large-file tests, a11y ids, security review
- [ ] T138 Generate `CHANGELOG.md` from Conventional Commits (DS-007)
- [ ] T139 Run final `/sdd:map incremental`; review `retro/P10.md` → CLAUDE.md (conservative); [GIT] commit
- [ ] T140 [GIT] Push; PR "Polish & cross-cutting"; verify CI green; report PR ready status

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (P1)** → no deps.
- **Foundational (P2)** → after Setup. **BLOCKS all user stories.**
- **US1 (P3)** → after Foundational. MVP. No dependency on other stories.
- **US2 (P4)** → after Foundational. Independent of US1 (integrates via shared shell).
- **US3 (P5)** → after **US2** (needs the workspace index).
- **US4 (P6)** → after Foundational (preview engine). Independent of US1–US3.
- **US5 (P7)** → after **US2** (index/resolution) + **US4** (preview for embeds).
- **US6 (P8)** → after Foundational; richer with US2/US4 but independently testable.
- **US7 (P9)** → after Foundational; applies to editor (US1) + preview (US4).
- **Polish (P10)** → after the desired stories.

### Within each story

- Core tests written first and FAIL before core implementation (Constitution VII).
- Core → FFI export → Swift wrapper → Swift UI → integration/UI test.

### Parallel opportunities

- Setup: T003/T004/T006 in parallel.
- Foundational: the three core substrates (document T018-19, fs T021-22, runtime T024) are largely parallel after the error model (T013) lands.
- After Foundational, **US1, US2, US4, US6 can proceed in parallel**; US3 waits on US2; US5 waits on US2+US4.
- Within a story, `[P]` test files and independent Swift views run in parallel.

---

## Parallel Example: User Story 1

```bash
# Core tests first (parallel, must fail before impl):
Task: "parse_incremental.rs — changed_ranges edit-locality (T036)"
Task: "benches/highlight.rs — re-highlight one line <5ms (T037)"

# Then independent Swift pieces (parallel):
Task: "SmartLists.swift (T045)"
Task: "FormattingCommands.swift (T046)"
```

---

## Implementation Strategy

### MVP first (US1 only)

1. Phase 1 Setup → 2. Phase 2 Foundational (CRITICAL) → 3. Phase 3 US1 → **STOP & VALIDATE**: edit + autosave a file end-to-end. Demoable MVP.

### Incremental delivery

Foundational → US1 (MVP) → US2 → US3 → US4 → US5 → US6 → US7 → Polish. Each story is a green, independently testable PR before the next.

### Parallel team strategy

After Foundational: US1, US2, US4, US6 can be staffed concurrently; US3 picks up once US2's index lands; US5 once US2 + US4 land.

---

## Notes

- `[P]` = different files, no incomplete deps. `[Story]` maps to spec.md user stories. `[GIT]` = git op.
- Tests included per Constitution VII: **core test-first**, **UI pragmatic** (headless logic + coarse XCUITest).
- Rust tasks → `devs:rust-dev`. Swift/AppKit tasks: implement directly (no mapped agent).
- Pre-commit/pre-push hooks MUST pass; never `--no-verify`. Conventional Commits required.
- Each phase ends at a PR + green CI checkpoint — stop and await LGTM before merging.
- Performance budgets are **tracked, non-blocking** (Principle IV): record and review regressions, don't gate merges on them.
