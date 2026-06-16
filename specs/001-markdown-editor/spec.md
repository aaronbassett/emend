# Feature Specification: Emend — A Quiet, Native macOS Markdown Editor

**Feature Branch**: `001-markdown-editor`
**Created**: 2026-06-16
**Status**: Draft
**Input**: User description: "Create a clone of cogito.md (without the Typefully integration). macOS only (Apple Silicon). Rust backend, Swift frontend. AI features must support BYOM (bring your own model) for any OpenAI-API-compatible model."

**Codebase Documentation**: See [.sdd/codebase/](../../.sdd/codebase/) for technical details

## Overview

A quiet, beautiful, minimal, fast native macOS app for reading, editing, and browsing Markdown files without friction. Notes live as plain Markdown files on disk — no database, no sync service, no proprietary container — so any other tool, workflow, or AI agent can read and write the same files. The app is built for power users and developers who want a polished, distraction-free writing surface paired with a faithful preview and lightweight document insight (including optional AI summaries using a model the user supplies).

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Write Markdown in a live, distraction-free editor (Priority: P1)

A writer opens a Markdown file and edits it in a live editor where structural syntax (heading hashes, emphasis markers, list bullets) is visually dimmed so prose stays readable, while bold, italics, headings, quotes, and lists are reflected inline as they type. Edits autosave to the plain `.md` file on disk with no explicit save step.

**Why this priority**: This is the heart of the product — the writing experience. On its own it is a usable single-file Markdown editor and delivers the core value ("write Markdown like it was meant to be written"). Everything else enriches this surface.

**Independent Test**: Open a single `.md` file, type prose with headings/bold/italic/lists/tasks, confirm syntax dims while formatting renders inline, confirm smart-list renumbering and indentation, then quit and reopen to verify the on-disk file contains correct, clean Markdown.

**Acceptance Scenarios**:

1. **Given** an open Markdown file, **When** the user types `**bold**`, **Then** the text renders bold inline and the `**` markers are dimmed rather than removed.
2. **Given** the user is on a numbered list item, **When** they press Return and add an item, **Then** the list auto-renumbers and maintains indentation.
3. **Given** unsaved edits, **When** the user pauses typing, **Then** the changes are autosaved to the `.md` file on disk within a short, bounded delay with no data loss.
4. **Given** selected text, **When** the user presses the bold/italic/link/task shortcut, **Then** the corresponding Markdown is applied around the selection.

---

### User Story 2 - Browse and manage a file-based workspace (Priority: P1)

A user adds one or more folders on disk as "locations," sees their folders and Markdown files in a sidebar tree, opens files in tabs, and creates/renames/moves/deletes/reorganizes items directly from the sidebar. Because the notes are just files, changes made by other tools or AI agents appear live.

**Why this priority**: The product is for browsing Markdown files, not just editing one. A navigable, file-backed workspace is essential to the "home for your (and your agents') Markdown" promise and is required for linking, Quick Open, and multi-note workflows.

**Independent Test**: Add a folder containing nested Markdown files, confirm the tree renders, open a file in a tab, rename and move a file via drag-and-drop, then modify a file with an external editor and confirm the app reflects the change without a manual reload.

**Acceptance Scenarios**:

1. **Given** no locations, **When** the user adds a folder, **Then** its Markdown files and subfolders appear as a navigable tree.
2. **Given** a workspace, **When** the user drags a file to another folder in the sidebar, **Then** the file moves on disk and the tree updates.
3. **Given** an open file, **When** an external tool modifies that file on disk, **Then** the app refreshes the content within a short, bounded delay.
4. **Given** a folder, **When** the user marks it a Favorite or assigns a custom icon, **Then** it appears under Favorites / shows the chosen icon.

---

### User Story 3 - Find any file instantly with Quick Open (Priority: P2)

A user presses ⌘P and fuzzy-searches files and folders by name or path across the entire workspace, sees ranked results each with a folder breadcrumb, and opens one with Return.

**Why this priority**: Fast navigation is a defining "power user" feature ("find any note in an instant"). It depends on the workspace (US2) but is independently valuable and testable.

**Independent Test**: In a workspace of many files, press ⌘P, type a partial/fuzzy query, confirm relevant results appear ranked with breadcrumbs, and open the top result with Return.

**Acceptance Scenarios**:

1. **Given** a workspace, **When** the user presses ⌘P and types a fuzzy query, **Then** matching files and folders appear ranked, each with its folder path.
2. **Given** Quick Open results, **When** the user presses Return on a selection, **Then** that file opens (in a tab) and Quick Open closes.
3. **Given** a large workspace, **When** the user types, **Then** results update responsively without perceptible lag.

---

### User Story 4 - Read with a faithful preview and export to PDF (Priority: P2)

A user views a native preview that renders the document faithfully — syntax-highlighted code, tables, Mermaid diagrams, and math — with source/preview scrolling kept in sync, and can export the rendered preview as a polished PDF.

**Why this priority**: Reading is the third pillar ("native editor and preview, built as one"). It enriches but does not block the core editing/browsing experience, hence P2.

**Independent Test**: Open a document containing a fenced code block, a table, a Mermaid diagram, and a math expression; confirm each renders correctly in the preview, confirm scroll sync with the source, then export to PDF and confirm the PDF matches the preview.

**Acceptance Scenarios**:

1. **Given** a document with fenced code, **When** previewed, **Then** the code is syntax-highlighted for its declared language (20+ languages supported).
2. **Given** a document with a table, a Mermaid diagram, and a math expression, **When** previewed, **Then** all three render correctly.
3. **Given** editor and preview both visible, **When** the user scrolls one, **Then** the other stays in sync.
4. **Given** a document, **When** the user exports to PDF, **Then** the resulting PDF visually matches the preview.

---

### User Story 5 - Link and connect notes (Priority: P2)

A user creates `[[wiki links]]` between notes with live autocomplete and resolution, embeds other notes with `![[embeds]]`, toggles clickable task checkboxes, highlights text with `==…==`, and drops images inline.

**Why this priority**: Linking turns a folder of files into a connected knowledge base — a key power-user differentiator — but depends on the workspace and editor first.

**Independent Test**: Type `[[`, confirm autocomplete lists matching notes with paths, complete a link, click it to navigate, add an `![[embed]]` and confirm the embedded content appears in preview, click a task checkbox and confirm the underlying `[ ]`/`[x]` toggles.

**Acceptance Scenarios**:

1. **Given** the user types `[[laun`, **When** suggestions appear, **Then** matching notes (e.g., `launch-plan`, `launch-post`) are listed with their folder path, and selecting one inserts a resolved link.
2. **Given** a resolved `[[wiki link]]`, **When** the user clicks it, **Then** the target note opens.
3. **Given** a `![[note]]` embed, **When** previewed, **Then** the referenced note's content is included inline.
4. **Given** a task line `- [ ] item`, **When** the user clicks the checkbox, **Then** the Markdown toggles to `- [x]` (and back) and is saved.
5. **Given** a wiki link whose target does not exist, **When** rendered, **Then** it is visually marked as unresolved.

---

### User Story 6 - Understand a document at a glance, with an AI summary (Priority: P3)

A user opens an info sidebar that shows an AI-generated summary, word/character counts, estimated reading time, task completion (N of M), and a live clickable outline of headings. The AI summary uses a model the user supplies via any OpenAI-API-compatible endpoint (bring your own model).

**Why this priority**: Document insight is a delightful enhancement layered on top of a working editor. The AI portion is optional and must never be required for the app to function, so it is P3.

**Independent Test**: Open a long document, open the info sidebar, confirm word/char/reading-time and "N of M completed" and a live outline; with no AI configured, confirm everything except the summary works and nothing is sent externally; configure an OpenAI-compatible model, request a summary, and confirm a summary appears.

**Acceptance Scenarios**:

1. **Given** an open document, **When** the info sidebar is shown, **Then** it displays word count, character count, estimated reading time, task completion (N of M), and a live outline of headings.
2. **Given** the outline, **When** the user clicks a heading, **Then** the editor scrolls to that section.
3. **Given** no AI configuration, **When** the info sidebar is shown, **Then** all non-AI insights work and no document content is transmitted anywhere.
4. **Given** a configured OpenAI-compatible model, **When** the user requests a summary, **Then** a summary of the document is generated and displayed.
5. **Given** the document changes, **When** the user is editing, **Then** stats, task completion, and outline update live.

---

### User Story 7 - Customize typography and appearance (Priority: P3)

A user adjusts curated typography (font, size, spacing) and the app honors light/dark appearance to suit their reading and writing comfort.

**Why this priority**: A polish feature that increases comfort and "pleasure to use," but the app is fully usable with sensible defaults, so P3.

**Independent Test**: Change font/size/spacing in settings and confirm the editor and preview update; switch system appearance and confirm the app follows light/dark.

**Acceptance Scenarios**:

1. **Given** typography settings, **When** the user changes font, size, or spacing, **Then** the editor and preview reflect the change immediately.
2. **Given** the system appearance changes (light/dark), **When** the app is open, **Then** it adopts the matching native appearance.

---

### Edge Cases

- **External edits / conflicts**: A file open in the editor is modified or deleted on disk by another tool or agent — the app must refresh to reflect on-disk state and must avoid silently overwriting external changes or losing user edits.
- **Large files**: Opening, scrolling, and editing very large documents (e.g., 1 MB / ~10,000 lines) must remain fast.
- **Unresolved / ambiguous links**: `[[link]]` targets that don't exist, or multiple notes sharing a name, must be handled clearly (mark unresolved; disambiguate by path).
- **Non-Markdown / binary files**: Folders contain images, PDFs, and other non-Markdown files — the workspace must handle these gracefully (e.g., images usable as drag-drop sources; non-text files not opened as Markdown).
- **Malformed content**: Broken Markdown, invalid Mermaid syntax, or invalid math expressions must degrade gracefully without crashing the preview.
- **Unsaved changes on close**: Closing a tab/window or quitting must not lose recent edits (autosave guarantees).
- **Moved/deleted locations**: A workspace location folder is renamed, moved, or deleted outside the app — the app must surface this state without crashing.
- **AI failure modes**: No configuration, unreachable endpoint, invalid API key, rate limiting, timeout, or oversized document — each must produce a clear message and never block editing.
- **Permissions**: The user adds a folder the app lacks permission to read/write — the app must request access or explain the limitation.
- **Filename collisions**: Creating, renaming, or moving a file to a name that already exists must be handled without data loss.
- **Symlink cycles / aliasing**: A location contains symlinked folders that form a cycle or alias the same file via two paths — traversal/watching must terminate and must not double-index.
- **Encoding variants**: Files with a UTF-8 BOM, CRLF line endings, or invalid UTF-8 must be read correctly or surfaced clearly, never crash.
- **Embed cycles / depth**: `![[A]]` embeds `B` which embeds `A`, or very deep embed chains — must be detected and bounded.
- **Self-write echo**: The app's own autosave must not be misread as an external change and trigger a reload/conflict.
- **Renamed link targets**: A note referenced by `[[links]]` is renamed/moved — links are not auto-updated in v1 and should show as unresolved rather than silently breaking navigation.

## Requirements *(mandatory)*

### Functional Requirements

**Workspace & Files**

- **FR-001**: Users MUST be able to add any folder on disk as a workspace "location," and remove locations.
- **FR-002**: System MUST display each location's folders and Markdown files as a navigable sidebar tree.
- **FR-003**: System MUST store all notes as plain Markdown (`.md`) files on disk, with no proprietary database, sync service, or container, such that any external tool or agent can read and write the same files.
- **FR-003a**: System MUST correctly read files written by other tools: UTF-8 with or without a BOM and both LF and CRLF line endings MUST be read correctly; malformed YAML frontmatter and non-UTF-8 files MUST be surfaced gracefully and MUST NOT crash parsing or the app.
- **FR-004**: Users MUST be able to create, rename, move, and delete files and folders from the sidebar.
- **FR-004a**: On create/rename/move into a name that already exists, the system MUST NOT overwrite the existing file; it MUST refuse or disambiguate (e.g., auto-suffix) with clear feedback. Renaming or moving a note that is open in a tab MUST keep the tab pointed at the same note.
- **FR-005**: Users MUST be able to reorganize files and folders via drag-and-drop in the sidebar.
- **FR-006**: System MUST detect external file/folder changes (create/modify/delete/rename) and refresh the view within 2 seconds under normal event rates, without a manual reload.
- **FR-006a**: The app's own disk writes (autosave, file operations) MUST NOT be surfaced as external modifications; self-originated writes MUST be distinguished from third-party writes (no spurious reload or conflict prompt for the app's own saves).
- **FR-006b**: File-change events MUST be debounced/coalesced per path; the system MUST remain responsive and memory-bounded when a bulk external operation produces many changes at once (e.g., 10,000 changes from a `git checkout` or an agent rewriting many files), and SHOULD recognize delete+create pairs as moves where the OS reports them.
- **FR-006c**: When an open file with UNSAVED in-memory edits is changed on disk by a third party, the system MUST NOT silently overwrite either version; it MUST preserve the user's unsaved edits, mark the file as externally changed (stale), and let the user choose to reload (discarding local edits) or keep their version. When an open file with NO unsaved edits changes on disk, it MUST reload silently.
- **FR-007**: Users MUST be able to mark files and folders as Favorites and pin notes and folders for quick access.
- **FR-008**: System MUST let users assign a custom folder icon from a library of 200+ icons.
- **FR-009**: System MUST autosave edits to disk shortly after the user pauses (target ≤ 2 seconds), preserving clean plain-Markdown formatting, with no explicit save action and no data loss.
- **FR-009a**: Autosave MUST be atomic and durable — the file watcher and external tools MUST never observe a partially written note, and a write MUST be durable before it is reported complete.

**Editing**

- **FR-010**: System MUST provide a live editor in which structural syntax markers are visually dimmed while formatting (bold, italic, headings, quotes, lists) is reflected inline.
- **FR-011**: System MUST provide keyboard shortcuts for bold, italic, link, and task formatting.
- **FR-012**: System MUST support smart lists with automatic ordered-list renumbering and automatic indent/outdent.
- **FR-013**: Users MUST be able to insert images and media inline via drag-and-drop; dropped media is stored relative to the note and referenced by a relative path.
- **FR-013a**: Dropped media MUST be stored in a defined per-note or per-location attachments directory with collision-safe naming; behavior MUST be defined for the case where the target note is still untitled/unsaved.
- **FR-014**: System MUST render clickable task checkboxes that toggle the underlying `[ ]`/`[x]` Markdown when clicked.
- **FR-015**: System MUST support highlight syntax (`==text==`) rendered as highlighted text.

**Navigation**

- **FR-016**: Users MUST be able to open multiple files in tabs within a window.
- **FR-017**: Users MUST be able to invoke Quick Open (⌘P) to fuzzy-search files and folders across the workspace by name/path, with each result showing its folder breadcrumb, and open a result.
- **FR-017a**: System MUST maintain an index of note names/paths supporting Quick Open and wiki-link resolution. After any file create/rename/move/delete (internal or external), the index MUST reflect the change within 2 seconds, and index updates MUST be incremental (a single change MUST NOT require a full workspace rescan). Quick Open matches file/folder name and path (not full document content) in v1.
- **FR-018**: System MUST keep Quick Open responsive (see SC-004) as workspace size grows into the tens of thousands of files.

**Linking**

- **FR-019**: System MUST support `[[wiki links]]` that resolve to workspace notes, render as clickable links, and navigate to the target on click.
- **FR-019a**: Wiki-link resolution MUST follow a deterministic, documented algorithm (match by note name, disambiguating by path), behave consistently across multiple locations, and handle two notes sharing a basename without choosing arbitrarily. In v1, renaming a note does NOT auto-update wiki links pointing to it (such links may become unresolved, per FR-022).
- **FR-020**: System MUST offer live autocomplete while typing a wiki link, listing matching notes with their folder path.
- **FR-021**: System MUST support Obsidian-style `![[embeds]]` that include the referenced note's content inline in the preview.
- **FR-021a**: Embed rendering MUST detect and stop embed cycles (A embeds B embeds A) and MUST enforce a maximum embed depth, degrading gracefully rather than looping.
- **FR-022**: System MUST visually indicate unresolved wiki links and embeds.

**Preview & Rendering**

- **FR-023**: System MUST provide a faithful preview, available alongside or toggled with the source editor.
- **FR-024**: System MUST synchronize scrolling between source editor and preview.
- **FR-025**: System MUST apply syntax highlighting to fenced code blocks for at least 20 common programming languages.
- **FR-026**: System MUST render Markdown tables, Mermaid diagrams, and mathematical notation (LaTeX-style math).
- **FR-027**: System MUST render large documents quickly, with no perceptible lag while scrolling or typing.
- **FR-027a**: System MUST define a maximum supported note size; beyond it, behavior MUST be graceful (e.g., open read-only or refuse with a clear message) rather than hang or exhaust memory. Editing MUST use incremental re-parsing so a single edit does not re-parse the entire document.
- **FR-028**: Users MUST be able to export the rendered preview as a PDF (see SC-010 for fidelity expectations).

**Document Insight**

- **FR-029**: System MUST provide an info sidebar showing word count, character count, and estimated reading time for the current document.
- **FR-030**: System MUST show task completion tracking (N of M tasks completed) for the current document.
- **FR-031**: System MUST show a live, clickable document outline (headings) that updates as the document changes and scrolls the editor to the selected section.
- **FR-031a**: Derived document data (outline, word/character counts, reading time, task N-of-M, link/embed list) MUST update within a short bound after an edit (target ≤ 300 ms) without degrading typing latency below the SC-003 target; computation MUST be incremental and MUST NOT block the editing path.
- **FR-032**: System MUST be able to generate an AI summary of the current document and display it in the info sidebar. The document summary is the ONLY user-facing AI feature in v1.
- **FR-032a**: The AI integration MUST be built behind a provider/feature abstraction so additional AI-assisted features (e.g., rewrite, continue, ask-about-document, suggested titles) can be added in future versions without re-architecting. These additional features are out of scope for v1.

**AI — Bring Your Own Model (BYOM)**

- **FR-033**: Users MUST be able to configure AI by providing a base URL, API key, and model identifier for any service compatible with the OpenAI Chat Completions API standard.
- **FR-034**: System MUST store the AI API key securely using the platform secure store (never in plain text).
- **FR-035**: System MUST NOT transmit any document content externally unless the user has explicitly configured a model AND invoked an AI feature; with no configuration the app is fully functional offline and sends nothing externally.
- **FR-036**: System MUST handle AI failure modes gracefully (no config, unreachable endpoint, invalid key, rate limit, timeout, oversized input) with clear messaging and without blocking editing.
- **FR-036a**: AI summary requests MUST be cancellable; cancelling or superseding a request MUST abort the in-flight network call and MUST NOT mutate document state. The system MUST enforce a defined request timeout and a defined maximum input size, rejecting oversized input locally (before any network call) with a clear message.
- **FR-037**: Users MUST be able to validate (test) their AI configuration and change or remove it at any time.

**Typography, Appearance & Platform**

- **FR-038**: System MUST provide curated typography with user customization (font, size, spacing) applied to editor and preview.
- **FR-039**: System MUST follow native light/dark appearance.
- **FR-040**: System MUST run as a native macOS application on Apple Silicon.

### Non-Functional Requirements

These cross-cutting requirements arise from the split between a core engine and a native UI; they are stated as behavior, independent of the eventual frontend↔core mechanism.

- **NFR-001 (Non-blocking)**: Any operation that can exceed one UI frame (~16 ms) — file I/O, indexing, parsing, search, AI calls, PDF export — MUST be asynchronous and MUST NOT block the editing/typing path.
- **NFR-002 (Cancellable)**: Long-running operations (search queries, AI requests, PDF export, index rebuilds) MUST be cancellable, and a superseded request MUST be abandoned without affecting later requests.
- **NFR-003 (Crash safety / error model)**: Recoverable errors MUST NOT abort the application; all fallible operations MUST surface structured errors the UI can render, and internal failures MUST be contained at the frontend↔core boundary (no crash propagation).
- **NFR-004 (Concurrency safety)**: Concurrent file-system events, user-initiated file operations, and search/index queries MUST be safe and MUST NOT corrupt the index or workspace model.
- **NFR-005 (Bounded memory)**: Resident memory MUST remain bounded and proportional to open documents plus the index, not to total workspace size; closing a tab MUST release that document's buffer.
- **NFR-006 (Secret hygiene)**: The AI API key MUST NOT be written to logs, crash reports, temporary files, autosave output, or note content, and MUST be held in memory with minimal lifetime.
- **NFR-007 (Path identity)**: Workspace traversal MUST terminate in the presence of symlink cycles, MUST NOT index the same physical file via two different paths, and MUST behave correctly on both case-insensitive and case-sensitive macOS volumes.

### Key Entities *(include if feature involves data)*

- **Location**: A user-added root folder on disk; the entry point for a tree of folders and notes. Attributes: path, display name, order.
- **Folder**: A directory within a location. Attributes: path, optional custom icon, favorite/pinned state, child ordering.
- **Note (Document)**: A Markdown file on disk. Attributes: path, content (UTF-8 Markdown), optional YAML frontmatter; derived: outline, word/char counts, reading time, task list, links/embeds.
- **Tab**: An open note within a window. Attributes: associated note, active state, scroll position.
- **Link / Embed**: A reference from one note to another (`[[…]]` link or `![[…]]` embed). Attributes: raw target, resolved note (or unresolved), kind.
- **Task**: A checkbox list item within a note. Attributes: completion state, source line.
- **AI Provider Configuration**: User-supplied connection to an OpenAI-compatible model. Attributes: base URL, model id, securely-stored API key.
- **Document Summary**: AI-generated summary text associated with a note, produced on demand.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From launch, a user can add a folder, open a note, and begin editing in under 3 seconds.
- **SC-002**: Opening a large document (~1 MB / ~10,000 lines) shows visible rendered content within 500 ms (95th percentile), measured from the open action to first visible content, and scrolls without visible lag.
- **SC-003**: Typing latency in the editor stays imperceptible (≤ 50 ms per keystroke at the 95th percentile) even in large documents.
- **SC-004**: Quick Open returns ranked results within 100 ms (95th percentile, warm index) in a workspace of 10,000 files, and a user can locate and open any file in under 5 seconds.
- **SC-005**: Edits persist to disk within 2 seconds of the user pausing, with zero data loss across quit/relaunch and never producing a partially written file on disk.
- **SC-006**: An external change to an open file (when it has no unsaved local edits) appears in the app within 2 seconds; when there are unsaved edits, the user is notified of the conflict within 2 seconds and no version is lost.
- **SC-007**: 95% of first-time users can add a folder, open a note, and apply bold/italic/task formatting without consulting documentation.
- **SC-008**: With no AI configured, all non-AI functions work fully offline and the app makes no outbound network connection triggered by document content or AI features (verifiable in an integration test).
- **SC-009**: After configuring an OpenAI-compatible model, a document summary is produced and displayed, with in-app overhead — from invoking the summary to dispatching the request, plus from response receipt to display, excluding model inference and network time — under 500 ms.
- **SC-010**: Exported PDFs are content-faithful to the preview: code is syntax-highlighted, tables / Mermaid diagrams / math are rendered, and no content is clipped (pagination and reflow are acceptable).

## Development Standards

These are engineering/process requirements for the project, distinct from the user-facing behavior above. They reflect choices confirmed during specification.

- **DS-001**: Rust code MUST be formatted with rustfmt and MUST pass clippy with no warnings.
- **DS-002**: Swift code MUST be formatted with SwiftFormat and MUST pass SwiftLint.
- **DS-003**: A Git pre-commit hook MUST run the formatters and linters and block commits that fail.
- **DS-004**: Continuous integration (GitHub Actions) MUST build the app, run linters, and run the test suites on every push and pull request, including a macOS Apple Silicon runner for the Swift build.
- **DS-005**: The Rust core MUST have unit and integration tests covering parsing, indexing/search, the AI client, and file watching, runnable via `cargo test`.
- **DS-006**: The Swift app layer MUST have unit and UI tests via XCTest.
- **DS-007**: Commits MUST follow the Conventional Commits standard, and a changelog MUST be generated from commit history for releases.

## Assumptions

- **Platform/architecture constraints (given by the user)**: native macOS app targeting Apple Silicon; a Rust core ("backend") handling file system access, workspace indexing, Markdown parsing/rendering support, search, and the AI client; a Swift/SwiftUI frontend for the UI. These technology choices are recorded in `.sdd/codebase/STACK.md` and are intentionally out of scope for the behavioral requirements above.
- Notes are UTF-8 Markdown; optional document metadata is YAML frontmatter.
- "OpenAI API standard" refers to the Chat Completions API request/response shape; the user supplies base URL, API key, and model id, enabling local (e.g., self-hosted) or hosted providers.
- Reading time uses a conventional words-per-minute estimate (~200 wpm) unless configured otherwise.
- Autosave uses a short debounce (target ≤ 2 seconds after the last edit).
- The app requests the macOS file-access permissions required to read/write user-selected folders.
- A single primary window with tabs is the default; multiple windows are permitted but not required for v1.

## Out of Scope

- Typefully integration and any publishing to X/LinkedIn (explicitly excluded).
- Platforms other than macOS on Apple Silicon (no Windows, Linux, iOS, or Intel Mac support).
- Any built-in cloud sync, hosted account, or proprietary storage — files remain plain on disk.
- Real-time multi-user collaboration.
- A bundled/managed AI model — the product only connects to a model the user brings (BYOM).
- Additional AI features beyond the document summary (rewrite, continue, ask-about-document, suggested titles) — deferred to a future version; the v1 architecture must not preclude them (FR-032a).
- Built-in version control UI (external tools handle VCS over the on-disk files).
- Code-signed / notarized distribution builds — deferred beyond v1 (not selected as a v1 requirement); local development builds are sufficient for v1.
