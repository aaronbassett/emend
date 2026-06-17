//! T095 — wiki-link & task extraction + resolution (US5 · FR-019/019a, FR-014).
//!
//! These integration tests pin two behaviours the spec calls load-bearing:
//!
//! 1. **Deterministic resolution for duplicate basenames (FR-019a).** When two
//!    notes share a basename, [`resolve_wikilink`] MUST NOT pick arbitrarily; it
//!    follows the documented tie-break in [`emend_core::derived`]:
//!    a. a candidate in the **same directory as the source note** wins;
//!    b. else the **shallowest** path (fewest separators) wins;
//!    c. else the **lexicographically smallest** path string wins.
//!    The order is total and reproducible across runs (no `HashMap`-iteration
//!    leak), so the same workspace always resolves a link the same way.
//!
//! 2. **A rename leaves old links unresolved (FR-019a, v1).** Renaming a note
//!    does NOT rewrite the `[[links]]` pointing at it; the old target no longer
//!    resolves (returns `None`) rather than mis-pointing at some other note.
//!
//! Plus the supporting extraction/toggle surface (`extract_links`,
//! `wikilink_suggestions`, `toggle_task`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::derived::{
    extract_links, resolve_wikilink, toggle_task, wikilink_suggestions, LinkKind,
};
use emend_core::index::Index;

/// Build an index from `(abs, rel)` pairs for resolution tests.
fn index_of(pairs: &[(&str, &str)]) -> Index {
    let mut index = Index::new();
    for (abs, rel) in pairs {
        index.insert(abs, rel);
    }
    index
}

// -- Link extraction --------------------------------------------------------

#[test]
fn extracts_wiki_links_and_embeds_with_ranges() {
    // A doc with a link and an embed; ranges are UTF-16 code units over the
    // source so the editor can map them onto NSRange.
    let src = "See [[Launch Plan]] and embed ![[Daily Note]] here.\n";
    let links = extract_links(src);
    assert_eq!(links.len(), 2, "one link + one embed: {links:?}");

    let link = &links[0];
    assert_eq!(link.kind, LinkKind::Link);
    assert_eq!(link.raw_target, "Launch Plan");

    let embed = &links[1];
    assert_eq!(embed.kind, LinkKind::Embed);
    assert_eq!(embed.raw_target, "Daily Note");

    // The link's source range must select exactly the `[[Launch Plan]]` text.
    let start = link.range.start as usize;
    let len = link.range.len as usize;
    let units: Vec<u16> = src.encode_utf16().collect();
    let slice = String::from_utf16(&units[start..start + len]).unwrap();
    assert_eq!(slice, "[[Launch Plan]]");
}

#[test]
fn embed_marker_is_not_double_counted_as_a_link() {
    // `![[x]]` is an embed, NOT also a `[[x]]` link — the `!` prefix must claim
    // the whole token so we don't emit two overlapping refs.
    let links = extract_links("![[only an embed]]\n");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].kind, LinkKind::Embed);
}

#[test]
fn pipe_alias_target_is_the_left_side() {
    // `[[Target|Display]]` resolves by Target, not the display alias.
    let links = extract_links("[[Real Target|shown text]]\n");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].raw_target, "Real Target");
}

// -- Deterministic duplicate-basename resolution (FR-019a) ------------------

#[test]
fn duplicate_basename_prefers_same_directory_as_source() {
    // Two `note.md` in different folders. A link FROM `/root/a/from.md` must
    // resolve to the sibling `/root/a/note.md`, not the one in `/root/b`.
    let index = index_of(&[
        ("/root/a/note.md", "a/note.md"),
        ("/root/b/note.md", "b/note.md"),
    ]);
    let resolved = resolve_wikilink(&index, "/root/a/from.md", "note");
    assert_eq!(resolved.as_deref(), Some("/root/a/note.md"));

    // Symmetric: a link from inside `/root/b` resolves to the `/root/b` copy.
    let resolved_b = resolve_wikilink(&index, "/root/b/from.md", "note");
    assert_eq!(resolved_b.as_deref(), Some("/root/b/note.md"));
}

#[test]
fn duplicate_basename_falls_back_to_shallowest_then_lexicographic() {
    // No candidate shares the source's directory, so the tie-break falls through
    // to (b) shallowest path, then (c) lexicographic. `/root/note.md` (depth 1)
    // beats `/root/deep/note.md` (depth 2).
    let index = index_of(&[
        ("/root/deep/note.md", "deep/note.md"),
        ("/root/note.md", "note.md"),
    ]);
    let resolved = resolve_wikilink(&index, "/elsewhere/from.md", "note");
    assert_eq!(
        resolved.as_deref(),
        Some("/root/note.md"),
        "shallowest path wins when no sibling candidate exists"
    );
}

#[test]
fn duplicate_basename_resolution_is_deterministic_across_runs() {
    // Same workspace + same query must resolve identically every time (no
    // arbitrary choice). Run the resolution repeatedly and assert stability.
    let index = index_of(&[
        ("/w/z/note.md", "z/note.md"),
        ("/w/a/note.md", "a/note.md"),
        ("/w/m/note.md", "m/note.md"),
    ]);
    let first = resolve_wikilink(&index, "/w/from.md", "note");
    assert!(first.is_some());
    for _ in 0..50 {
        assert_eq!(resolve_wikilink(&index, "/w/from.md", "note"), first);
    }
    // With no sibling and equal depth, lexicographically smallest path wins.
    assert_eq!(first.as_deref(), Some("/w/a/note.md"));
}

#[test]
fn unique_basename_resolves_directly() {
    let index = index_of(&[("/root/notes/unique.md", "notes/unique.md")]);
    assert_eq!(
        resolve_wikilink(&index, "/root/from.md", "unique").as_deref(),
        Some("/root/notes/unique.md")
    );
    // Case-insensitive + extension-agnostic, matching how users type `[[unique]]`.
    assert_eq!(
        resolve_wikilink(&index, "/root/from.md", "UNIQUE.md").as_deref(),
        Some("/root/notes/unique.md")
    );
}

#[test]
fn unresolved_target_returns_none() {
    let index = index_of(&[("/root/a.md", "a.md")]);
    assert_eq!(resolve_wikilink(&index, "/root/from.md", "missing"), None);
}

// -- Rename leaves old links unresolved (FR-019a, v1: no auto-rewrite) ------

#[test]
fn rename_leaves_old_link_target_unresolved_not_mispointed() {
    // `old.md` is the link target. After renaming it to `new.md`, a link that
    // still says `[[old]]` must resolve to NOTHING (v1 does not auto-rewrite
    // links), and crucially must NOT silently point at the renamed note or any
    // other note.
    let mut index = index_of(&[("/root/old.md", "old.md"), ("/root/other.md", "other.md")]);
    assert_eq!(
        resolve_wikilink(&index, "/root/from.md", "old").as_deref(),
        Some("/root/old.md")
    );

    // Rename old.md -> new.md (the same incremental op the workspace performs).
    index.rename("/root/old.md", "/root/new.md", "new.md");

    // The OLD target no longer resolves — unresolved, not mis-pointed (FR-019a).
    assert_eq!(
        resolve_wikilink(&index, "/root/from.md", "old"),
        None,
        "a renamed note's old name must be UNRESOLVED, never mis-pointed"
    );
    // The new name resolves to the renamed note.
    assert_eq!(
        resolve_wikilink(&index, "/root/from.md", "new").as_deref(),
        Some("/root/new.md")
    );
}

// -- Autocomplete suggestions (FR-020) --------------------------------------

#[test]
fn suggestions_rank_matches_for_a_prefix() {
    let index = index_of(&[
        ("/root/launch-plan.md", "launch-plan.md"),
        ("/root/launch-post.md", "launch-post.md"),
        ("/root/unrelated.md", "unrelated.md"),
    ]);
    let hits = wikilink_suggestions(&index, "laun", 10);
    assert!(
        hits.len() >= 2,
        "both launch-* notes should suggest: {hits:?}"
    );
    assert!(hits.iter().all(|h| h.name.contains("launch")));
    // An unrelated note is not a fuzzy match for `laun`.
    assert!(!hits.iter().any(|h| h.name.contains("unrelated")));
}

// -- Task toggle (FR-014) ---------------------------------------------------

#[test]
fn toggle_unchecked_task_to_checked() {
    let src = "- [ ] write tests\n";
    // Toggling anywhere on the line flips the checkbox.
    let out = toggle_task(src, 0).unwrap();
    assert_eq!(out, "- [x] write tests\n");
}

#[test]
fn toggle_checked_task_back_to_unchecked() {
    let src = "- [x] write tests\n";
    let out = toggle_task(src, 0).unwrap();
    assert_eq!(out, "- [ ] write tests\n");
}

#[test]
fn toggle_uppercase_checked_is_treated_as_complete() {
    // GFM allows `[X]`; toggling it un-checks to `[ ]`.
    let src = "- [X] done\n";
    let out = toggle_task(src, 0).unwrap();
    assert_eq!(out, "- [ ] done\n");
}

#[test]
fn toggle_on_a_specific_line_in_a_multiline_doc() {
    let src = "intro\n- [ ] a\n- [ ] b\n";
    // Offset inside the SECOND task line (`- [ ] b` starts after "intro\n- [ ] a\n").
    let units: Vec<u16> = src.encode_utf16().collect();
    let second_line_start = "intro\n- [ ] a\n".encode_utf16().count();
    let at = u32::try_from(second_line_start).unwrap();
    let out = toggle_task(src, at).unwrap();
    assert_eq!(out, "intro\n- [ ] a\n- [x] b\n");
    // Sanity: only the second task changed.
    assert_eq!(units.len(), src.encode_utf16().count());
}

#[test]
fn toggle_on_a_non_task_line_is_an_error() {
    let src = "just a paragraph\n";
    assert!(
        toggle_task(src, 0).is_err(),
        "a line with no task checkbox cannot be toggled"
    );
}
