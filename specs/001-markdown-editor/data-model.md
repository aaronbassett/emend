# Phase 1 Data Model: Emend

**Feature**: 001-markdown-editor · **Date**: 2026-06-16

Emend has **no database**. Two stores exist:

1. **On disk (source of truth for content)** — plain Markdown files and attachments in user-chosen folders. Any external tool/agent can read/write them (FR-003).
2. **App-managed state (local preferences)** — locations, favorites/pins, folder icons, typography, AI config metadata, window/tab restoration. Stored in a local app-support store (e.g. a small JSON/plist or SQLite owned by the core); the **AI API key lives only in Keychain** (C5/NFR-006), never in this store.

Ownership: the **Rust core** owns the workspace model, the index, parsed/derived data, and the app-state store. **Swift** owns the live `NSTextStorage` buffer and UI/view state. Ranges crossing the boundary are **UTF-16 code units** (A2).

---

## Entities

### Location
A user-added root folder; entry point to a tree.
| Field | Type | Notes |
|------|------|------|
| id | LocationId (stable) | survives relaunch |
| displayName | String | defaults to folder name; user-editable |
| securityScopedBookmark | Data | macOS bookmark (A4); resolved on launch, re-created if stale |
| order | Int | sidebar ordering |
- **Validation**: bookmark must resolve to a readable directory; on failure → surfaced as "unavailable" (Edge Case: moved/deleted location), not a crash.
- **Relationships**: 1 Location → many Folders/Notes (the on-disk subtree).

### Folder
A directory within a Location.
| Field | Type | Notes |
|------|------|------|
| path | AbsPath | identity; case handling per host volume (NFR-007) |
| customIcon | IconId? | SF Symbol / custom symbol name (C8); null = default |
| isFavorite | Bool | appears under Favorites |
| isPinned | Bool | pinned for quick access |
| childOrder | [Path]? | manual drag-drop order; null = natural sort |
- **Relationships**: contains Folders and Notes. Not indexed as content; appears in Quick Open by name/path.

### Note (Document)
A Markdown file on disk.
| Field | Type | Notes |
|------|------|------|
| path | AbsPath | identity; `.md` |
| isFavorite / isPinned | Bool | |
| frontmatter | YAML? | optional; malformed → treated gracefully (FR-003a) |
| content | UTF-8 text | on disk; mirrored as Swift `NSTextStorage` when open |
| diskMtime / diskLen | (timestamp, bytes) | for self-write suppression + conflict detection (B3, FR-006c) |
| *derived* outline | [OutlineItem] | headings; live (FR-031) |
| *derived* stats | DocStats | word/char/reading-time (FR-029) |
| *derived* tasks | [Task] | N-of-M completion (FR-030) |
| *derived* links | [LinkRef] | wiki links + embeds (FR-019..022) |
| *derived* summary | DocumentSummary? | AI, on demand (FR-032) |
- **Validation**: read tolerant of BOM/CRLF/non-UTF-8 (FR-003a). Size > max (default ~5 MB, §D) → open read-only.
- **Encoding**: preserved on round-trip; autosave is atomic+durable (FR-009a).

### OpenDocument / Tab
An open Note in the single window (C7). UI/session state.
| Field | Type | Notes |
|------|------|------|
| noteRef | Note path | |
| isDirty | Bool | unsaved in-memory edits exist |
| externallyChanged | Bool | set when disk changed while dirty (FR-006c) |
| scrollPosition / selection | view state | per-tab, preserved on switch |
| order / isActive | Int / Bool | tab strip |
- **State transitions** (per FR-006c, FR-009/FR-009a):
  - `clean` → user edits → `dirty`
  - `dirty` → autosave (idle ≥1.5 s) → atomic write → `clean` (+ update diskMtime/Len; suppress own watcher event)
  - `clean` + external change → silent reload → `clean`
  - `dirty` + external change → `dirty + externallyChanged` (conflict): **no auto-overwrite**; user picks *Reload* (→ discard local, `clean`) or *Keep mine* (→ stays `dirty`, next save overwrites)

### LinkRef (Wiki link / Embed)
A reference inside a Note.
| Field | Type | Notes |
|------|------|------|
| kind | `link` \| `embed` | `[[…]]` vs `![[…]]` |
| rawTarget | String | as typed |
| resolvedNote | Note path? | null = unresolved (FR-022) |
| sourceRange | Utf16Range | for click/navigation |
- **Resolution** (FR-019a): deterministic — match normalized name via the name→path map (B2); disambiguate by path; consistent across locations; rename does **not** auto-update (may become unresolved). Embeds: cycle-detected, max depth (default 8) (FR-021a).

### Task
A checkbox list item.
| Field | Type | Notes |
|------|------|------|
| isComplete | Bool | `[x]`/`[ ]` |
| sourceRange | Utf16Range | click toggles underlying Markdown (FR-014) |
- Aggregated into DocStats `N of M completed` (FR-030).

### Attachment
Media dropped into a Note (FR-013/FR-013a).
| Field | Type | Notes |
|------|------|------|
| storedPath | RelPath | per-note/location attachments dir; collision-safe name |
| referencedFrom | Note path | inserted as relative-path Markdown image |
- **Validation**: untitled/unsaved note → defined fallback target dir before first save.

### AIProviderConfig
BYOM connection (FR-033/FR-034).
| Field | Type | Notes |
|------|------|------|
| baseURL | URL | OpenAI-compatible chat-completions endpoint |
| model | String | model id |
| apiKeyRef | KeychainRef | **value only in Keychain**, never in app store/logs |
| requestTimeout / maxInputBytes | config | enforced locally before send (FR-036a) |
- **Validation**: "test configuration" performs a minimal request and reports reachability/auth (FR-037). Absent config → no network, app fully offline (FR-035/SC-008).

### DocumentSummary
| Field | Type | Notes |
|------|------|------|
| text | String | AI-generated (FR-032) |
| sourceContentHash | Hash | invalidate when the note changes |
| generatedAt | timestamp | |
- On-demand; v1's only AI feature (FR-032/FR-032a).

### TypographySettings (global)
Font, size, line/paragraph spacing, theme follows system light/dark (FR-038/FR-039). Applies to editor + preview.

### WorkspaceIndex (derived, in-memory)
Not persisted as truth; rebuilt/maintained incrementally (B2/FR-017a).
| Part | Structure | Purpose |
|------|-----------|---------|
| entries | arena `Vec<FileEntry>` keyed by `PathId` | Quick Open haystack (nucleo) |
| pathMap | `HashMap<Path,PathId>` | event dispatch |
| nameMap | `HashMap<NormName,[PathId]>` | O(1) wiki-link resolution |
- Freshness: reflects any create/rename/move/delete within ~2 s (SC-006); updates incremental (no full rescan).

---

## Entity relationship summary

```
Location 1──* Folder 1──* (Folder|Note)
Note 1──* Task
Note 1──* LinkRef ──0..1 Note (resolved target)
Note 1──* Attachment
Note 0..1 DocumentSummary
OpenDocument *──1 Note            (tabs reference notes)
AIProviderConfig 1 (global) ──> Keychain (api key)
WorkspaceIndex (derived) <── all Notes/Folders across Locations
TypographySettings 1 (global)
```

## Persistence map

| Data | Where | Rationale |
|------|-------|-----------|
| Note content + frontmatter + attachments | **Disk** (plain files) | FR-003 — no DB/sync |
| Locations, favorites/pins, folder icons, child order | App-support store (core-owned) | not derivable from files |
| Typography, AI baseURL/model/timeouts | App-support store | preferences |
| **AI API key** | **Keychain only** | NFR-006 / C5 |
| Tabs/window/scroll/selection | Session-restore store | UX continuity |
| Index, outline, stats, tasks, links, summary | **Derived / in-memory** (rebuildable) | computed by core, never authoritative |
