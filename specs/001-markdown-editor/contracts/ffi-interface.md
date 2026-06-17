# Contract: Swift ↔ Rust FFI Interface (UniFFI)

**Feature**: 001-markdown-editor · **Date**: 2026-06-16

Emend exposes no network API; the integration contract is the **UniFFI boundary** between the Swift app and the Rust core (`emend-ffi`). This document is the authoritative shape of that surface. Signatures are illustrative (proc-macro UniFFI, Swift-facing names); the implementation in `/sdd:implement` must match these names, parameters, error behavior, and threading rules.

## Global rules (apply to every member)

- **Ranges are UTF-16 code units** (`U16Range { start: u32, len: u32 }`) — matches `NSRange`. (A2)
- **Every fallible call returns `Result<_, EmendError>`** → Swift `throws`. No panic may cross the boundary (NFR-003 / B7).
- **Hot path is synchronous** (`pushEdit`, span queries); **async** is reserved for AI + search (A1). Async functions are cancellable only via returned **handles**, not Swift `Task` cancellation.
- **Streaming & callback semantics**: AI tokens and incremental search results are delivered through **foreign-trait callbacks** (`AiSink`/`SearchSink`/`DocObserver`) the Swift side implements. Exactly **one terminal callback** occurs per stream (`on_done` on success, `on_error` on failure). After `cancel()`/supersede, the terminal is `on_error(AiCancelled)` and **no further** `on_token`/`on_results` fire. Streamed tokens are **complete UTF-8 strings** — the SSE parser buffers partial bytes across chunks and never emits a split code point. Callbacks are **non-reentrant**: the foreign side MUST NOT call back into the core from within a callback (queue the work instead).
- The **API key is never stored or logged** in the core; it is passed per-request as a transient argument (C5/NFR-006).

---

## Error type

```rust
#[derive(uniffi::Error)]
enum EmendError {
  NotFound { path: String },
  PermissionDenied { path: String },     // sandbox/scope failure (A4)
  IoFailure { path: String, detail: String },
  NameCollision { path: String },        // FR-004a
  NoteTooLarge { path: String, bytes: u64, limit: u64 },  // FR-027a
  InvalidConfig { detail: String },
  AiNotConfigured,                       // FR-035
  AiTimeout,
  AiCancelled,                           // FR-036a (supersede/cancel)
  AiOversizedInput { bytes: u64, limit: u64 },  // FR-036a
  AiHttp { status: u16, detail: String },
  AiStreamMalformed { detail: String },
  Internal { detail: String },           // captured panic / unexpected (B7)
}
```

---

## 1. Workspace & Locations  (US2 · FR-001..008, FR-017a, NFR-007)

```rust
fn add_location(folder_path: String, bookmark: Vec<u8>) -> Result<Location>   // Swift supplies a resolved security-scoped path + bookmark (A4)
fn remove_location(id: LocationId) -> Result<()>
fn list_locations() -> Vec<Location>
fn reorder_locations(order: Vec<LocationId>) -> Result<()>

fn list_children(folder_path: String) -> Result<Vec<FsNode>>   // lazy; FsNode = { path, kind: File|Folder, name }
fn set_folder_icon(folder_path: String, icon: Option<String>) -> Result<()>   // FR-008
fn set_favorite(path: String, favorite: bool) -> Result<()>                    // FR-007
fn set_pinned(path: String, pinned: bool) -> Result<()>
fn set_child_order(folder_path: String, order: Vec<String>) -> Result<()>      // FR-005 drag-drop
fn list_favorites() -> Vec<FsNode>
```

## 2. File operations  (FR-004/FR-004a/FR-005 · atomic per FR-009a)

```rust
fn create_note(parent: String, name: String) -> Result<String>     // returns new path; NameCollision per FR-004a
fn create_folder(parent: String, name: String) -> Result<String>
fn rename(path: String, new_name: String) -> Result<String>        // NameCollision-safe; keeps open tab pointed (FR-004a)
fn move_node(path: String, new_parent: String) -> Result<String>
fn delete(path: String) -> Result<()>
fn store_attachment(note_path: Option<String>, bytes: Vec<u8>, suggested_name: String) -> Result<String>  // FR-013a; returns relative ref. note_path == None is unsupported in v1 (attachment needs a saved note) → InvalidConfig; caller saves the note first.
```

## 3. Document session & editing  (US1 · FR-009/009a, FR-010..015, FR-027a, FR-031a)

```rust
fn open_document(path: String) -> Result<OpenDocHandle>            // reads tolerant of BOM/CRLF/non-UTF-8 (FR-003a); NoteTooLarge → caller opens read-only
fn close_document(h: OpenDocHandle)

// HOT PATH — synchronous, non-blocking, returns immediately (A3)
fn push_edit(h: OpenDocHandle, range: U16Range, replacement: String)

// Highlight/structure spans for a viewport range (editor pulls lazily on scroll)
fn highlight_spans(h: OpenDocHandle, viewport: U16Range) -> Vec<StyleSpan>   // StyleSpan { range, class: StyleClass }
fn toggle_task(h: OpenDocHandle, at: U16Range) -> Result<()>      // FR-014 clickable checkbox

// Autosave is internal+debounced; this forces a durable flush (e.g. on close/quit)
fn flush(h: OpenDocHandle) -> Result<()>                          // atomic+durable (FR-009a)

// Conflict handling (FR-006c)
fn conflict_state(h: OpenDocHandle) -> ConflictState             // Clean | DirtyClean | DirtyExternalChanged
fn resolve_conflict(h: OpenDocHandle, choice: ConflictChoice) -> Result<()>   // ReloadFromDisk | KeepMine
```

## 4. Derived insight  (US6 · FR-029..031 · live per FR-031a)

```rust
fn outline(h: OpenDocHandle) -> Vec<OutlineItem>          // { level, title, range } — click→scroll (FR-031)
fn stats(h: OpenDocHandle) -> DocStats                    // { words, chars, reading_minutes, tasks_done, tasks_total } (FR-029/030)
fn links(h: OpenDocHandle) -> Vec<LinkRef>                // { kind, raw, resolved_path?, range } (FR-019..022)

// Push model so the UI updates live without polling:
trait DocObserver { fn on_derived_changed(&self, h: OpenDocHandle); }   // foreign callback; fired ≤300ms after edits (FR-031a)
fn set_doc_observer(h: OpenDocHandle, obs: Box<dyn DocObserver>)
```

## 5. Quick Open & link resolution  (US3/US5 · FR-017/017a, FR-019a, FR-020)

```rust
// Streaming, supersedable search (each keystroke supersedes the prior query)
fn quick_open_query(query: String, sink: Box<dyn SearchSink>) -> SearchHandle   // ranked results streamed; ≤100ms p95 (SC-004)
trait SearchSink { fn on_results(&self, batch: Vec<SearchHit>); fn on_done(&self); }   // SearchHit { path, name, breadcrumb, score }
// SearchHandle.cancel()  supersedes an in-flight query (NFR-002)

fn resolve_wikilink(from_note: String, raw_target: String) -> Option<String>   // deterministic O(1) (FR-019a)
fn wikilink_suggestions(prefix: String) -> Vec<SearchHit>                       // autocomplete (FR-020)
```

## 6. Preview & export  (US4 · FR-023..028)

```rust
fn render_preview_html(h: OpenDocHandle) -> Result<String>   // comrak HTML with data-line anchors (C3) + syntect classed code (B6); embeds resolved with cycle/depth guard (FR-021a)
fn preview_assets_dir() -> String                            // bundled Mermaid/KaTeX/theme CSS base (C2)
// PDF export is a Swift-side concern (NSPrintOperation on a WKWebView, C4); the core only supplies HTML.
```

## 7. AI — BYOM  (US6 · FR-032..037 · streaming/cancellable per FR-036a)

```rust
// Streaming summary. apiKey is transient; never stored/logged (C5/NFR-006).
fn summarize_document(
    h: OpenDocHandle,
    cfg: AiRequestConfig,          // { base_url, model, request_timeout_ms, max_input_bytes }
    api_key: String,               // transient
    sink: Box<dyn AiSink>,
) -> AiHandle
trait AiSink {
  fn on_token(&self, text: String);          // SSE delta
  fn on_done(&self, full: String);
  fn on_error(&self, err: EmendError);        // AiTimeout/AiCancelled/AiHttp/... (no key in payload)
}
// AiHandle.cancel()  → AiCancelled; superseding a summary cancels the prior (NFR-002, FR-036a)

fn test_ai_config(cfg: AiRequestConfig, api_key: String) -> Result<()>   // FR-037 minimal reachability/auth probe
```

## 8. Settings  (FR-038/039)

```rust
fn get_typography() -> TypographySettings
fn set_typography(t: TypographySettings) -> Result<()>
fn get_ai_config_meta() -> Option<AiConfigMeta>     // base_url/model/timeouts only — NEVER the key
fn set_ai_config_meta(meta: AiConfigMeta) -> Result<()>
```

---

## Contract test obligations (drive Phase-2/tasks)

Each maps to a spec requirement and is mechanically checkable:

1. `push_edit` returns within microseconds and does not block; p95 keystroke→glyph independent of doc size (SC-003).
2. A forced panic inside any export surfaces as a thrown `EmendError`, process survives (NFR-003/B7).
3. `flush` then external kill never leaves a partial file; reader always sees complete old/new (FR-009a).
4. Autosave does **not** produce an external-change callback for its own write (FR-006a); a concurrent third-party edit does.
5. `quick_open_query` p95 ≤100 ms over a 10k-entry index; superseding via `SearchHandle.cancel()` stops result emission (SC-004/NFR-002).
6. `resolve_wikilink` is deterministic for duplicate basenames; rename leaves old links unresolved, not mis-pointed (FR-019a).
7. With no AI config, no AI export opens a network connection; `summarize_document` → `AiNotConfigured`/`AiOversizedInput` enforced before any socket (FR-035/FR-036a/SC-008).
8. Cancelling `AiHandle` resolves promptly as `AiCancelled` with no further `on_token` (FR-036a).
9. `render_preview_html` terminates on an embed cycle within max depth (FR-021a).
10. Captured logs during an AI auth error contain no API-key substring (NFR-006).
11. Streaming terminal semantics: a cancelled `AiSink` receives exactly one `on_error(AiCancelled)` and zero subsequent `on_token`; a recorded SSE stream split mid-code-point still yields only complete UTF-8 tokens; a callback that re-enters the core is rejected/queued, not run reentrantly (I1).
