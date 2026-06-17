//! T097 — derived link & task data over a document's Markdown source (US5 ·
//! FR-014, FR-019/019a, FR-020; FFI contract §5).
//!
//! This module is the **link/task half of the document's derived data** (the
//! data-model "Note" `derived links` + `Task`). Where [`crate::index`] owns the
//! workspace-wide name→path map, this module owns the per-document scanning and
//! the resolution *policy* on top of that map:
//!
//! * [`extract_links`] — scan a document's source for `[[wiki links]]` and
//!   `![[embeds]]`, returning each as a [`LinkRef`] with its **UTF-16 source
//!   range** (so the editor can map it onto `NSRange` for click/navigation,
//!   FFI contract §5 / research §A2) and its raw target.
//! * [`resolve_wikilink`] — the **deterministic, documented** FR-019a resolution
//!   policy: ask the index for the candidate set, then break ties by a fixed
//!   rule (see below). Renaming a note is *not* auto-rewritten in v1, so a stale
//!   target simply resolves to `None` rather than mis-pointing (FR-019a).
//! * [`wikilink_suggestions`] — autocomplete for `[[` (FR-020): the index's fuzzy
//!   ranking over note names/paths.
//! * [`toggle_task`] — flip the `[ ]`/`[x]` of the task on the line containing a
//!   UTF-16 offset (FR-014), returning the new document text.
//!
//! Pure `std` + [`crate::index`]; **no `uniffi`, no `tokio`** (Constitution V),
//! so the resolution policy and the task toggle are unit-testable with plain
//! `cargo test` (`tests/links.rs`).
//!
//! ## Deterministic duplicate-basename resolution (FR-019a)
//!
//! FR-019a requires resolution to "handle two notes sharing a basename without
//! choosing arbitrarily". The index returns *all* candidates for a name; this
//! module picks one with a **total, documented tie-break** so the same workspace
//! always resolves a given link the same way:
//!
//! 1. **Same directory as the source note wins.** A `[[note]]` written in
//!    `/a/from.md` resolves to a sibling `/a/note.md` over a distant
//!    `/b/note.md` — the "closest note" intuition users expect.
//! 2. **Else the shallowest path wins** (fewest path separators) — a top-level
//!    note outranks a deeply-nested namesake.
//! 3. **Else the lexicographically smallest path string wins** — a final,
//!    arbitrary-but-stable tiebreaker so the order is *total* (never a coin flip).
//!
//! The ranking is computed over the candidate set the index returns (which is
//! itself order-independent), so resolution is reproducible across runs.

use crate::index::{Index, SearchHit};
use crate::{EmendError, U16Range};
use std::path::Path;

/// Conventional reading speed in words per minute (research §D / spec
/// Assumptions): `reading_minutes` is `ceil(words / WORDS_PER_MINUTE)`.
const WORDS_PER_MINUTE: u32 = 200;

/// Aggregated, derived "understand a document at a glance" stats for the info
/// sidebar (US6 · FR-029/030; FFI contract §4 `stats`).
///
/// All counts are over the document's Markdown *source* (the editor buffer), and
/// are cheap to recompute on each edit (FR-031a). Bundled into one value so the
/// FFI `stats` export is a single round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocStats {
    /// Word count (whitespace-delimited tokens that contain a letter or digit, so
    /// bare punctuation runs like `#`/`-`/`**` do not count — FR-029).
    pub words: u32,
    /// Character count (Unicode scalar values, i.e. `char`s — not bytes, not
    /// UTF-16 code units — FR-029).
    pub chars: u32,
    /// Estimated reading time in whole minutes, `ceil(words / 200)` (FR-029);
    /// any non-empty prose is at least 1 minute, an empty document is 0.
    pub reading_minutes: u32,
    /// Number of completed task checkboxes (`[x]`/`[X]`) — FR-030.
    pub tasks_done: u32,
    /// Total number of task checkboxes (complete + incomplete) — FR-030.
    pub tasks_total: u32,
}

/// One heading in the document outline (US6 · FR-031/031a; FFI contract §4
/// `outline`).
///
/// `line` is the **1-based source line** the heading sits on, so the editor can
/// scroll to that line when the user clicks the outline entry (FR-031). `level`
/// is the ATX heading level 1..=6.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    /// ATX heading level, 1..=6.
    pub level: u8,
    /// The heading text, trimmed and with any closing `#` run removed.
    pub title: String,
    /// 1-based source line number of the heading (for click→scroll, FR-031).
    pub line: u32,
}

/// Whether a [`LinkRef`] is a navigable link (`[[…]]`) or an inline embed
/// (`![[…]]`). The FFI mirror of the data-model "LinkRef.kind".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A `[[wiki link]]` — clickable, navigates to the target note (FR-019).
    Link,
    /// A `![[embed]]` — inlines the target note's content in the preview
    /// (FR-021).
    Embed,
}

/// A reference from a document to another note (data-model "LinkRef").
///
/// `range` is in **UTF-16 code units** over the source document so it maps 1:1
/// onto `NSRange` for click/navigation (research §A2). `raw_target` is the target
/// as typed (before a `|` alias), which [`resolve_wikilink`] resolves against the
/// index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    /// Link vs embed.
    pub kind: LinkKind,
    /// The target as typed (the part before any `|` alias), e.g. `Launch Plan`.
    pub raw_target: String,
    /// The full token's source range in UTF-16 code units (covers the `[[…]]` or
    /// `![[…]]`, inclusive of the markers, so the editor can style/hit-test it).
    pub range: U16Range,
}

/// The opening of an embed (`![[`) and a link (`[[`), and the shared close.
const EMBED_OPEN: &str = "![[";
const LINK_OPEN: &str = "[[";
const LINK_CLOSE: &str = "]]";

/// Scan `source` for `[[wiki links]]` and `![[embeds]]`, returning each as a
/// [`LinkRef`] in document order (FFI contract §5; FR-019..022).
///
/// An `![[…]]` is an [`LinkKind::Embed`]; a `[[…]]` (no `!` prefix) is an
/// [`LinkKind::Link`]. The `!` of an embed is included in the token's range so
/// the two never overlap (an embed is *not* also reported as a link). The
/// `raw_target` is the text before any `|` alias, trimmed.
///
/// Ranges are UTF-16 code units measured over `source`. Malformed tokens (an
/// unclosed `[[`) are skipped, not reported.
#[must_use]
pub fn extract_links(source: &str) -> Vec<LinkRef> {
    let mut refs = Vec::new();

    // Walk by char so we can track the UTF-16 offset cheaply (one pass, no
    // per-token re-encode). `byte` indexes into `source` for slicing; `u16_off`
    // is the running UTF-16 code-unit offset for the range fields.
    let bytes = source.as_bytes();
    let mut byte = 0usize;
    let mut u16_off: u32 = 0;

    while byte < bytes.len() {
        // Try to match an embed (`![[`) or a link (`[[`) starting at `byte`.
        let (is_embed, open_len) = if source[byte..].starts_with(EMBED_OPEN) {
            (true, EMBED_OPEN.len())
        } else if source[byte..].starts_with(LINK_OPEN) {
            (false, LINK_OPEN.len())
        } else {
            // Not a token start: advance one char, accounting for its UTF-16 width.
            let ch = next_char(source, byte);
            byte += ch.len_utf8();
            u16_off = u16_off.saturating_add(u32::try_from(ch.len_utf16()).unwrap_or(0));
            continue;
        };

        // Find the closing `]]` after the opener.
        let inner_start = byte + open_len;
        let Some(close_rel) = source[inner_start..].find(LINK_CLOSE) else {
            // Unclosed — skip the opener (advance past it) and keep scanning.
            let opener = &source[byte..inner_start];
            byte = inner_start;
            u16_off = u16_off.saturating_add(utf16_len(opener));
            continue;
        };

        let inner = &source[inner_start..inner_start + close_rel];
        let token_end = inner_start + close_rel + LINK_CLOSE.len();
        let token = &source[byte..token_end];

        let raw_target = inner.split('|').next().unwrap_or(inner).trim().to_owned();
        let token_u16 = utf16_len(token);
        refs.push(LinkRef {
            kind: if is_embed {
                LinkKind::Embed
            } else {
                LinkKind::Link
            },
            raw_target,
            range: U16Range::new(u16_off, token_u16),
        });

        byte = token_end;
        u16_off = u16_off.saturating_add(token_u16);
    }

    refs
}

/// Compute the aggregated [`DocStats`] for a document's Markdown `source` (US6 ·
/// FR-029/030; FFI contract §4 `stats`).
///
/// In one pass over the lines this counts:
/// * **words** — whitespace-delimited tokens containing at least one letter or
///   digit, so pure Markdown punctuation runs (`#`, `-`, `**`, `>`) do not count
///   (FR-029);
/// * **chars** — Unicode scalar values (`char`s) of the whole source (FR-029);
/// * **tasks** — completed (`[x]`/`[X]`) and total checkbox list items, reusing
///   the same checkbox recognition as [`toggle_task`] (FR-030).
///
/// **reading_minutes** is `ceil(words / 200)` (research §D): a non-empty document
/// is at least 1 minute, an empty one is 0.
///
/// Pure and cheap — recomputable on every edit within the FR-031a budget. Fenced
/// code blocks are still counted as words/chars (they are document content); only
/// the *outline* excludes fenced regions ([`outline`]).
#[must_use]
pub fn stats(source: &str) -> DocStats {
    let chars = u32::try_from(source.chars().count()).unwrap_or(u32::MAX);

    let mut words: u32 = 0;
    let mut tasks_total: u32 = 0;
    let mut tasks_done: u32 = 0;

    for line in source.lines() {
        words = words.saturating_add(count_words(line));
        if let Some(done) = task_completion(line) {
            tasks_total = tasks_total.saturating_add(1);
            if done {
                tasks_done = tasks_done.saturating_add(1);
            }
        }
    }

    let reading_minutes = words.div_ceil(WORDS_PER_MINUTE);

    DocStats {
        words,
        chars,
        reading_minutes,
        tasks_done,
        tasks_total,
    }
}

/// Count the "words" on a single line: whitespace-delimited tokens that contain
/// at least one alphanumeric character, so a bare punctuation run (`#`, `-`,
/// `**`, `>`, `|`) is not counted as a word (FR-029).
fn count_words(line: &str) -> u32 {
    let n = line
        .split_whitespace()
        .filter(|tok| tok.chars().any(char::is_alphanumeric))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Whether `line` is a task checkbox list item, and if so its completion state:
/// `Some(true)` for `[x]`/`[X]`, `Some(false)` for `[ ]`, `None` if the line is
/// not a task list item at all. Reuses [`find_checkbox`] so the recognition
/// matches [`toggle_task`] exactly (FR-030/FR-014).
fn task_completion(line: &str) -> Option<bool> {
    let bracket = find_checkbox(line)?;
    // `find_checkbox` guarantees `[<c>]`, so the char between the brackets exists.
    match line.as_bytes().get(bracket + 1) {
        Some(b'x' | b'X') => Some(true),
        Some(b' ') => Some(false),
        _ => None,
    }
}

/// Extract the heading outline from a document's Markdown `source` (US6 ·
/// FR-031/031a; FFI contract §4 `outline`).
///
/// Returns one [`OutlineItem`] per ATX heading (`#`..`######` followed by a
/// space) in document order, each carrying its level, trimmed title (with any
/// closing `#` run removed), and **1-based source line number** so the editor can
/// scroll to it on click (FR-031).
///
/// Fenced code blocks (```` ``` ````/`~~~`) are skipped, so a `#` inside a code
/// block (a shell comment, a C preprocessor directive) is never mistaken for a
/// heading. Setext headings (`===`/`---` underlines) are not part of the v1
/// outline — ATX headings are what the editor produces and the dimmed-syntax
/// surface encourages.
///
/// Pure and incremental-friendly: a single line scan, recomputable within the
/// FR-031a budget.
#[must_use]
pub fn outline(source: &str) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut in_fence = false;
    // Track the active fence marker char (` ``` ` vs `~~~`) so a `~~~` opener is
    // only closed by `~~~`, per CommonMark.
    let mut fence_char = b'`';

    for (idx, line) in source.lines().enumerate() {
        // 1-based source line number for click→scroll (FR-031).
        let line_no = u32::try_from(idx + 1).unwrap_or(u32::MAX);

        if let Some(marker) = fence_marker(line) {
            if in_fence {
                // Inside a fence: only a matching marker closes it.
                if marker == fence_char {
                    in_fence = false;
                }
            } else {
                in_fence = true;
                fence_char = marker;
            }
            continue;
        }
        if in_fence {
            continue; // contents of a code block are never headings
        }

        if let Some((level, title)) = parse_atx_heading(line) {
            items.push(OutlineItem {
                level,
                title,
                line: line_no,
            });
        }
    }

    items
}

/// If `line` is a fenced-code-block delimiter (`` ``` `` or `~~~`, optionally
/// indented up to 3 spaces, optionally with an info string), return its fence
/// char (`` b'`' `` or `b'~'`); else `None`. Used by [`outline`] to skip fenced
/// regions so a `#` inside code is not read as a heading.
fn fence_marker(line: &str) -> Option<u8> {
    let trimmed = line.trim_start_matches(' ');
    // CommonMark allows up to 3 leading spaces; more makes it indented code (not
    // a fence), but treating any indentation as a fence opener is harmless for
    // the outline's purpose (it still only toggles "skip headings").
    for (marker, prefix) in [(b'`', "```"), (b'~', "~~~")] {
        if trimmed.starts_with(prefix) {
            return Some(marker);
        }
    }
    None
}

/// Parse `line` as an ATX heading, returning `(level, title)` or `None`.
///
/// A valid ATX heading is up to 3 leading spaces, a run of 1..=6 `#`, **at least
/// one space**, then the title — with an optional trailing run of `#`
/// (and surrounding spaces) stripped (`## Title ##` → `Title`). `#nospace` is not
/// a heading (CommonMark requires the space), and a 7+ `#` run is not a heading.
fn parse_atx_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start_matches(' ');
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    // CommonMark: the `#` run must be followed by a space (or end of line for an
    // empty heading). `#nospace` is a paragraph, not a heading.
    let after = match rest.strip_prefix(' ') {
        Some(after) => after,
        None if rest.is_empty() => rest, // `###` alone is an empty heading
        None => return None,
    };
    // Strip an optional closing `#` run (and the spaces around it): `## T ##`.
    let title = after
        .trim_end()
        .trim_end_matches('#')
        .trim_end()
        .trim_start()
        .to_owned();
    let level = u8::try_from(hashes).unwrap_or(6);
    Some((level, title))
}

/// Resolve a `[[wiki link]]` target to a single absolute note path, using the
/// **deterministic FR-019a policy** (see the module docs). `from_note` is the
/// absolute path of the note containing the link (used for the same-directory
/// tie-break); `raw_target` is the link target as typed.
///
/// Returns `None` when the target resolves to no note — including the v1 case
/// where a note was renamed and the link still names the old basename: the index
/// no longer carries that name, so this returns `None` (unresolved) rather than
/// mis-pointing at some other note (FR-019a).
#[must_use]
pub fn resolve_wikilink(index: &Index, from_note: &str, raw_target: &str) -> Option<String> {
    let mut candidates = index.resolve_name(raw_target);
    match candidates.len() {
        0 => None,
        1 => candidates.pop(),
        _ => pick_best_candidate(candidates, from_note),
    }
}

/// Apply the FR-019a tie-break to a multi-candidate set, returning the winner.
///
/// Rank key (smaller is better): `(not_same_dir, depth, path)` —
/// 1. `not_same_dir`: `false` (0) for a candidate in `from_note`'s directory,
///    `true` (1) otherwise — so a sibling sorts first.
/// 2. `depth`: number of path separators — the shallowest path next.
/// 3. `path`: the path string itself — a total, stable final tiebreaker.
fn pick_best_candidate(candidates: Vec<String>, from_note: &str) -> Option<String> {
    let from_dir = parent_dir(from_note);
    candidates
        .into_iter()
        .min_by(|a, b| rank_key(a, from_dir).cmp(&rank_key(b, from_dir)))
}

/// The total ordering key for a candidate path (see [`pick_best_candidate`]).
fn rank_key<'a>(path: &'a str, from_dir: Option<&str>) -> (bool, usize, &'a str) {
    let same_dir = parent_dir(path) == from_dir && from_dir.is_some();
    let depth = path.matches('/').count();
    (!same_dir, depth, path)
}

/// The parent directory of `path` as a string slice, or `None` if it has no
/// parent (a bare basename). Uses [`Path`] so it is separator-correct.
fn parent_dir(path: &str) -> Option<&str> {
    Path::new(path)
        .parent()
        .and_then(Path::to_str)
        .filter(|p| !p.is_empty())
}

/// Autocomplete suggestions for a `[[` prefix (FFI contract §5; FR-020): the
/// index's fuzzy ranking over note names/paths, up to `limit` results.
///
/// This is a thin pass-through to [`Index::query`] — the autocomplete dropdown
/// wants the same ranked `name` + `rel_path` (breadcrumb) the Quick Open results
/// carry, so the two share the index's ranking (FR-017/FR-020).
#[must_use]
pub fn wikilink_suggestions(index: &Index, prefix: &str, limit: usize) -> Vec<SearchHit> {
    index.query(prefix, limit)
}

/// Toggle the task checkbox on the line containing the UTF-16 offset `at`,
/// returning the **new document text** (FR-014).
///
/// The clickable task checkbox in the editor reports the click position as a
/// UTF-16 offset; this finds the line that offset falls on, flips its `[ ]`↔`[x]`
/// (treating `[x]`/`[X]` as complete), and returns the whole document with that
/// one line changed. The caller turns the (old, new) text into an editor delta.
///
/// # Errors
///
/// [`EmendError::InvalidConfig`] if `at` is past the end of the document, or the
/// line it lands on is not a task list item (no `- [ ]` / `- [x]` / `* [ ]` /
/// numbered-list `[ ]` checkbox) — a click that is not on a checkbox cannot be
/// toggled.
pub fn toggle_task(source: &str, at: u32) -> Result<String, EmendError> {
    let (line_start, line_end) =
        line_bounds_at(source, at).ok_or_else(|| EmendError::InvalidConfig {
            detail: format!("toggle offset {at} is past the end of the document"),
        })?;

    let line = &source[line_start..line_end];
    let toggled = toggle_task_line(line).ok_or_else(|| EmendError::InvalidConfig {
        detail: "line at the toggle offset is not a task checkbox".to_owned(),
    })?;

    let mut out = String::with_capacity(source.len());
    out.push_str(&source[..line_start]);
    out.push_str(&toggled);
    out.push_str(&source[line_end..]);
    Ok(out)
}

/// The byte range `[start, end)` of the line in `source` that the UTF-16 offset
/// `at` falls on (the line content, **excluding** its trailing `\n`). Returns
/// `None` if `at` is past the end of the document.
///
/// Lines are split on `\n`; a trailing `\r` (CRLF) is kept inside the line slice
/// so the round-trip preserves the original line ending (the toggle only rewrites
/// the checkbox, never the newline style).
fn line_bounds_at(source: &str, at: u32) -> Option<(usize, usize)> {
    // Convert the UTF-16 offset to a byte offset by walking chars once.
    let byte_at = utf16_offset_to_byte(source, at)?;

    // Line start: just after the previous `\n` (or 0).
    let line_start = source[..byte_at].rfind('\n').map_or(0, |nl| nl + 1);
    // Line end: the next `\n` at or after `byte_at` (or EOF). Exclude the `\n`.
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |rel| line_start + rel);
    Some((line_start, line_end))
}

/// Convert a UTF-16 code-unit offset into a byte offset into `source`, or `None`
/// if the offset is past the document's UTF-16 length. An offset that lands on a
/// char boundary maps to that char's byte index; the just-past-EOF offset maps to
/// `source.len()`.
fn utf16_offset_to_byte(source: &str, at: u32) -> Option<usize> {
    if at == 0 {
        return Some(0);
    }
    let mut u16_seen: u32 = 0;
    for (byte_idx, ch) in source.char_indices() {
        if u16_seen >= at {
            return Some(byte_idx);
        }
        u16_seen = u16_seen.saturating_add(u32::try_from(ch.len_utf16()).unwrap_or(0));
    }
    // Past the last char: only the exact end-of-document offset is valid.
    if u16_seen >= at {
        Some(source.len())
    } else {
        None
    }
}

/// Flip a single task-list line's checkbox, returning the rewritten line, or
/// `None` if the line is not a task list item.
///
/// Recognizes the GFM tasklist shape: optional leading whitespace, a list marker
/// (`-`, `*`, `+`, or `N.`/`N)`), a space, then `[ ]` (incomplete) or
/// `[x]`/`[X]` (complete). Only the bracket content is rewritten; the marker,
/// indentation, and the text after the checkbox are preserved verbatim.
fn toggle_task_line(line: &str) -> Option<String> {
    let bracket = find_checkbox(line)?;
    let inner = &line[bracket + 1..bracket + 2]; // the single char between `[` and `]`
    let replacement = match inner {
        " " => 'x',
        "x" | "X" => ' ',
        _ => return None,
    };
    let mut out = String::with_capacity(line.len());
    out.push_str(&line[..bracket + 1]);
    out.push(replacement);
    out.push_str(&line[bracket + 2..]);
    Some(out)
}

/// The byte index of the opening `[` of a task checkbox on `line`, or `None` if
/// the line is not a task list item. A valid checkbox is `[<one char>]` right
/// after a list marker (`- `, `* `, `+ `, or `N. `/`N) `).
fn find_checkbox(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();

    // The list marker: a bullet (`-`/`*`/`+`) followed by a space, or an ordered
    // marker `digits` + `.`|`)` + space.
    let after_marker = strip_bullet(trimmed).or_else(|| strip_ordered(trimmed))?;
    // Where `after_marker` begins within `line`.
    let marker_len = trimmed.len() - after_marker.len();

    // Now expect `[<c>]` immediately (a checkbox), optionally with the text after.
    // `get(2)` is non-panicking (no indexing): a short or multi-byte-char slice
    // simply fails the match and returns `None`.
    let is_checkbox =
        after_marker.starts_with('[') && after_marker.as_bytes().get(2) == Some(&b']');
    if is_checkbox {
        // Offset of the `[` within `line`.
        Some(indent + marker_len)
    } else {
        None
    }
}

/// If `s` starts with a bullet list marker (`- `, `* `, `+ `), return the rest
/// after the marker + its single space; else `None`.
fn strip_bullet(s: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = s.strip_prefix(marker) {
            return Some(rest);
        }
    }
    None
}

/// If `s` starts with an ordered list marker (`N.` or `N)` followed by a space),
/// return the rest after the marker + its single space; else `None`.
fn strip_ordered(s: &str) -> Option<&str> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after_digits = &s[digits..];
    for sep in [". ", ") "] {
        if let Some(rest) = after_digits.strip_prefix(sep) {
            return Some(rest);
        }
    }
    None
}

/// The char at byte index `byte` in `source` (`byte` must be a char boundary).
/// Used by [`extract_links`] to advance one char at a time. Falls back to a
/// space-width char on the (unreachable) non-boundary case rather than panicking.
fn next_char(source: &str, byte: usize) -> char {
    source[byte..].chars().next().unwrap_or(' ')
}

/// The UTF-16 code-unit length of `s` as a `u32` (saturating; a document within
/// the note-size cap never approaches `u32::MAX` code units).
fn utf16_len(s: &str) -> u32 {
    u32::try_from(s.encode_utf16().count()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    // Unit tests assert on their own fixtures; the workspace denies these in
    // library code, so scope the allowance to this test module.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{
        extract_links, find_checkbox, parent_dir, resolve_wikilink, toggle_task, toggle_task_line,
        LinkKind,
    };
    use crate::index::Index;

    fn index_of(pairs: &[(&str, &str)]) -> Index {
        let mut index = Index::new();
        for (abs, rel) in pairs {
            index.insert(abs, rel);
        }
        index
    }

    #[test]
    fn extract_distinguishes_links_from_embeds() {
        let refs = extract_links("[[a]] ![[b]]\n");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].kind, LinkKind::Link);
        assert_eq!(refs[0].raw_target, "a");
        assert_eq!(refs[1].kind, LinkKind::Embed);
        assert_eq!(refs[1].raw_target, "b");
    }

    #[test]
    fn parent_dir_of_root_level_path() {
        assert_eq!(parent_dir("/a/b.md"), Some("/a"));
        assert_eq!(parent_dir("b.md"), None);
    }

    #[test]
    fn resolve_prefers_sibling() {
        let index = index_of(&[("/a/n.md", "a/n.md"), ("/b/n.md", "b/n.md")]);
        assert_eq!(
            resolve_wikilink(&index, "/a/from.md", "n").as_deref(),
            Some("/a/n.md")
        );
    }

    #[test]
    fn find_checkbox_handles_markers() {
        assert!(find_checkbox("- [ ] x").is_some());
        assert!(find_checkbox("* [x] x").is_some());
        assert!(find_checkbox("  + [ ] indented").is_some());
        assert!(find_checkbox("1. [ ] ordered").is_some());
        assert!(find_checkbox("2) [x] ordered paren").is_some());
        assert!(find_checkbox("plain text").is_none());
        assert!(find_checkbox("- not a checkbox").is_none());
    }

    #[test]
    fn toggle_task_line_flips_both_ways() {
        assert_eq!(toggle_task_line("- [ ] a").as_deref(), Some("- [x] a"));
        assert_eq!(toggle_task_line("- [x] a").as_deref(), Some("- [ ] a"));
        assert_eq!(toggle_task_line("- [X] a").as_deref(), Some("- [ ] a"));
        assert_eq!(toggle_task_line("paragraph"), None);
    }

    #[test]
    fn toggle_task_preserves_crlf() {
        // The trailing `\r` is inside the line slice; only the checkbox changes.
        let out = toggle_task("- [ ] a\r\n", 0).unwrap();
        assert_eq!(out, "- [x] a\r\n");
    }
}
