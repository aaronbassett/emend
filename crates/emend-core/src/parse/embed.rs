//! T097 — Obsidian-style `![[embed]]` resolution with cycle + depth guards
//! (US5 · FR-021/021a; research §B1 "the only custom code", §D max depth = 8).
//!
//! comrak renders `[[wikilinks]]`, `==highlight==`, GFM tasklists, and tables
//! natively (research §B1), but **`![[embed]]` is the one extension comrak does
//! not know** — it is bespoke. This module is that bespoke pass: a **pre-render
//! source transform** that replaces each `![[Target]]` token with the referenced
//! note's Markdown source, recursively, so the spliced-together Markdown can then
//! be handed to comrak as one document (the preview engine stays the single
//! authoritative HTML renderer — research §B1 / Constitution two-engine rule).
//!
//! ## Why a source pass (not an AST/HTML pass)
//!
//! Inlining at the **source** level means the embedded note's headings, lists,
//! code fences, tables, math, and even its *own* `![[embeds]]` are parsed by
//! comrak in the surrounding document's context — exactly as if the user had
//! pasted the text in. An HTML-level splice would have to re-parse fragments and
//! reconcile two HTML trees; a source splice is simpler and strictly more
//! faithful. The trade-off (source line numbers shift, so `data-line` anchors
//! past an embed are approximate) is acceptable for v1 — embeds are a reading
//! aid, and scroll-sync precision degrades gracefully rather than breaking.
//!
//! ## Termination (FR-021a) — two independent guards
//!
//! Either guard alone terminates; both together also bound the work:
//!
//! 1. **Cycle detection.** A stack of the embed targets currently being expanded
//!    (an `on_stack` set) is threaded through the recursion. Re-entering a note
//!    already on the stack (A→B→A, or A→A) is refused: the re-entrant token is
//!    replaced with an unresolved placeholder instead of recursed into, so a
//!    cycle expands each note **at most once per path** and then stops.
//! 2. **Depth bound.** Recursion stops at [`MAX_EMBED_DEPTH`] (default 8). A long
//!    acyclic chain `A`→`B`→`C`→… is cut off at the bound; the token at the bound
//!    is left as an unresolved placeholder.
//!
//! ## Nested resolution anchors on the immediate parent (FR-019a)
//!
//! Resolution of `![[Target]]` uses the FR-019a same-directory tie-break, which
//! is anchored on the **note the embed was written in** (`from_note`). For nested
//! embeds this must be the *immediately enclosing* note, not the top document:
//! when `A` embeds `B` and `B`'s source contains `![[C]]`, `C` must resolve
//! relative to `B`'s directory (its sibling), not `A`'s — otherwise duplicate
//! basenames across folders pick the wrong note. To make that possible the
//! resolver returns **both** the embedded note's source text *and* its resolved
//! path; the expander then recurses with that resolved path as the `from_note`
//! for the note's own embeds.
//!
//! ## Purity (no IO, no async)
//!
//! [`expand_embeds`] is a pure `&str -> String` transform parameterized by a
//! caller-supplied **resolver closure** (`(target, from_note) -> Option<(source,
//! resolved_path)>`). It does no IO and pulls in **no `tokio`/`uniffi`**
//! (Constitution V): the FFI/preview layer supplies a resolver that consults the
//! workspace index + reads the file, but the recursion/guard logic is
//! unit-testable with a plain `HashMap` resolver (`tests/embeds.rs`).

/// Maximum embed nesting depth before the chain is cut off (FR-021a; research §D
/// fixes the v1 default at 8). `![[a]]` at the top level is depth 0; the embed it
/// pulls in is depth 1; and so on. A note at depth `>= MAX_EMBED_DEPTH` is not
/// expanded (left as an unresolved placeholder).
pub const MAX_EMBED_DEPTH: usize = 8;

/// The opening marker for an embed token (`![[`). An embed is `![[` … `]]`.
const EMBED_OPEN: &str = "![[";
/// The closing marker shared by embeds and wikilinks (`]]`).
const EMBED_CLOSE: &str = "]]";

/// Tuning knobs for [`expand_embeds`]. Kept as a struct so future embed options
/// (e.g. a section anchor `![[note#heading]]`) can land without changing the
/// public signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EmbedOptions {
    /// Maximum nesting depth (FR-021a). Defaults to [`MAX_EMBED_DEPTH`]. A value
    /// of `0` expands nothing (every top-level embed is already at the bound).
    pub max_depth: usize,
}

impl Default for EmbedOptions {
    fn default() -> Self {
        Self {
            max_depth: MAX_EMBED_DEPTH,
        }
    }
}

impl EmbedOptions {
    /// Construct options with an explicit `max_depth`.
    #[must_use]
    pub const fn new(max_depth: usize) -> Self {
        Self { max_depth }
    }
}

/// Expand every `![[Target]]` in `source` by inlining the resolved note's source,
/// recursively, with cycle + depth guards (FR-021a).
///
/// `from_note` is the resolved path of the note `source` itself came from — the
/// anchor for the FR-019a same-directory tie-break of `source`'s own (top-level)
/// embeds. `resolve` maps an embed target *plus the current note's path* —
/// `(raw_target, from_note)` — to that target note's Markdown source **and its
/// resolved path**, or `None` if it does not resolve. The caller owns resolution
/// policy (the preview wires it to the workspace index + a tolerant on-disk
/// read); this function owns only the recursion and its termination guards.
///
/// The returned `resolved_path` is what anchors *nested* resolution: when the
/// embedded note's own source contains `![[…]]`, the expander recurses with that
/// path as the new `from_note`, so each note's embeds resolve relative to **its
/// own** directory — not the top document's (FR-019a; see the module docs).
///
/// ## Behaviour
///
/// * A resolved embed is replaced by the note's source, which is itself scanned
///   for further embeds (recursively, subject to the guards).
/// * An **unresolved** target (resolver returns `None`), a **cycle** (the target
///   is already being expanded on the current path), or a target **at/over the
///   depth bound** is replaced by [`unresolved_placeholder`] — a literal,
///   visible marker — so the output is always finite and the user sees that the
///   embed did not expand (FR-022 graceful degradation), never an infinite loop.
/// * `[[wikilinks]]` (no `!` prefix) are left untouched — comrak renders those.
#[must_use]
pub fn expand_embeds<R>(
    source: &str,
    from_note: &str,
    options: &EmbedOptions,
    resolve: &mut R,
) -> String
where
    R: FnMut(&str, &str) -> Option<(String, String)>,
{
    let mut on_stack: Vec<String> = Vec::new();
    expand_inner(source, from_note, options, resolve, &mut on_stack, 0)
}

/// Recursive worker for [`expand_embeds`]. `from_note` is the resolved path of
/// the note `source` came from (the FR-019a anchor for the embeds *in* `source`);
/// `on_stack` holds the normalized targets currently being expanded along this
/// path (cycle guard); `depth` is the current nesting level (depth guard).
fn expand_inner<R>(
    source: &str,
    from_note: &str,
    options: &EmbedOptions,
    resolve: &mut R,
    on_stack: &mut Vec<String>,
    depth: usize,
) -> String
where
    R: FnMut(&str, &str) -> Option<(String, String)>,
{
    let mut out = String::with_capacity(source.len());
    let mut rest = source;

    while let Some(open_idx) = rest.find(EMBED_OPEN) {
        // Copy everything before the `![[` verbatim.
        out.push_str(&rest[..open_idx]);

        let after_open = &rest[open_idx + EMBED_OPEN.len()..];
        let Some(close_rel) = after_open.find(EMBED_CLOSE) else {
            // No closing `]]` — not a well-formed embed. Emit the `![[` literally
            // and continue scanning past it (no infinite loop).
            out.push_str(EMBED_OPEN);
            rest = after_open;
            continue;
        };

        let inner = &after_open[..close_rel];
        let after_close = &after_open[close_rel + EMBED_CLOSE.len()..];

        let target = embed_target(inner);
        let key = normalize_target(target);

        if depth >= options.max_depth {
            // Depth bound reached (FR-021a): do not expand further.
            out.push_str(&unresolved_placeholder(target));
        } else if on_stack.iter().any(|t| t == &key) {
            // Cycle (FR-021a): this note is already being expanded on this path.
            out.push_str(&unresolved_placeholder(target));
        } else if let Some((embedded, resolved_path)) = resolve(target, from_note) {
            // Resolved: recurse into the embedded note's source one level deeper,
            // with this target pushed on the cycle stack. The embedded note's own
            // (nested) embeds anchor on ITS resolved path — `resolved_path` becomes
            // the `from_note` for the recursion — so a `![[C]]` inside the embedded
            // note resolves relative to the embedded note's directory, not the top
            // document's (FR-019a; see the module docs).
            on_stack.push(key);
            let expanded = expand_inner(
                &embedded,
                &resolved_path,
                options,
                resolve,
                on_stack,
                depth + 1,
            );
            on_stack.pop();
            out.push_str(&expanded);
        } else {
            // Unresolved target (FR-022): visible placeholder, no expansion.
            out.push_str(&unresolved_placeholder(target));
        }

        rest = after_close;
    }

    // Trailing text after the last embed (or the whole string if none).
    out.push_str(rest);
    out
}

/// The embed target from the inside of `![[ … ]]`, taking the part **before** a
/// `|` alias (`![[Target|Display]]` embeds `Target`) and trimming surrounding
/// whitespace.
fn embed_target(inner: &str) -> &str {
    let before_pipe = inner.split('|').next().unwrap_or(inner);
    before_pipe.trim()
}

/// Normalize an embed target into the cycle-stack key: lowercased and trimmed, so
/// `Daily Note`, `daily note`, and `  Daily Note ` are treated as the same note
/// for cycle detection (matching how the index normalizes names).
fn normalize_target(target: &str) -> String {
    target.trim().to_lowercase()
}

/// The visible placeholder substituted for an embed that did not expand —
/// unresolved, cyclic, or past the depth bound (FR-022 / FR-021a graceful
/// degradation). Rendered as italic text so it reads as a non-content notice in
/// the preview without breaking the surrounding Markdown.
#[must_use]
pub fn unresolved_placeholder(target: &str) -> String {
    format!("*(unresolved embed: {target})*")
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

    use super::{embed_target, expand_embeds, normalize_target, EmbedOptions, MAX_EMBED_DEPTH};
    use std::collections::HashMap;

    fn store(notes: &[(&str, &str)]) -> HashMap<String, String> {
        notes
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// A resolver over a name→source map that mirrors the new `(target,
    /// from_note) -> Option<(source, resolved_path)>` contract. These unit tests
    /// don't exercise per-parent anchoring (see `tests/embeds.rs` for that), so
    /// the resolved path is simply the target name — enough to recurse and bound.
    fn resolver(
        notes: &HashMap<String, String>,
    ) -> impl FnMut(&str, &str) -> Option<(String, String)> + '_ {
        move |target: &str, _from_note: &str| {
            notes
                .get(target)
                .map(|src| (src.clone(), target.to_owned()))
        }
    }

    #[test]
    fn embed_target_strips_pipe_alias_and_whitespace() {
        assert_eq!(embed_target(" Daily Note "), "Daily Note");
        assert_eq!(embed_target("Target|Shown"), "Target");
        assert_eq!(embed_target("plain"), "plain");
    }

    #[test]
    fn normalize_target_lowercases_and_trims() {
        assert_eq!(normalize_target("  Daily Note "), "daily note");
    }

    #[test]
    fn wikilink_without_bang_is_left_alone() {
        let notes = store(&[("x", "expanded")]);
        let out = expand_embeds(
            "a [[x]] b\n",
            "/note.md",
            &EmbedOptions::default(),
            &mut resolver(&notes),
        );
        // No `!` prefix → not an embed; comrak handles `[[x]]`, we don't touch it.
        assert_eq!(out, "a [[x]] b\n");
    }

    #[test]
    fn malformed_unclosed_embed_is_emitted_literally() {
        let notes = store(&[]);
        let out = expand_embeds(
            "![[unterminated\n",
            "/note.md",
            &EmbedOptions::default(),
            &mut resolver(&notes),
        );
        assert!(out.contains("![["), "unclosed embed stays literal: {out}");
    }

    #[test]
    fn depth_zero_expands_nothing() {
        let notes = store(&[("x", "body")]);
        let opts = EmbedOptions::new(0);
        let out = expand_embeds("![[x]]\n", "/note.md", &opts, &mut resolver(&notes));
        assert!(!out.contains("body"), "max_depth 0 expands nothing: {out}");
        assert!(out.contains("unresolved embed"), "placeholder shown: {out}");
    }

    #[test]
    fn default_depth_is_spec_value() {
        assert_eq!(MAX_EMBED_DEPTH, 8);
        assert_eq!(EmbedOptions::default().max_depth, 8);
    }
}
