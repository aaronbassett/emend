//! T055 — failing-first integration tests for the workspace search index
//! (`emend_core::index`), the in-memory haystack behind Quick Open and
//! wiki-link resolution (US2 · FR-017, FR-017a, FR-018; research §B2).
//!
//! The index is **derived, in-memory, and never authoritative** (data-model
//! "WorkspaceIndex"): it is rebuilt/maintained incrementally from the files on
//! disk. These tests pin down the three obligations that make it correct:
//!
//! 1. **Incremental, O(1)-ish updates (FR-017a).** A single create / rename /
//!    move / delete must update the index *in place* — touching only the
//!    affected entry — and MUST NOT trigger a full workspace rescan. Wall-clock
//!    timing is not deterministic across machines, so we assert this
//!    **structurally**: the index exposes a `rebuild_count()` that only the
//!    (test-only / seeding) full-rebuild path increments. After a single
//!    incremental op the rebuild count is unchanged, and `len()` moves by
//!    exactly one — proving the op did not re-walk the tree.
//!
//! 2. **Fuzzy subsequence matching on basename AND relative path (FR-017).**
//!    A query matches as a fuzzy subsequence against (a) the item basename and
//!    (b) its location-relative path. So `foo` matches `notes/foo.md`, and the
//!    cross-segment query `nt/foo` also matches `notes/foo.md`.
//!
//! 3. **Ranking favors basename matches and shorter paths (FR-017).** Given two
//!    items that both contain the query, the one whose *basename* carries the
//!    match outranks one that only matches deeper in its path; ties broken toward
//!    the shorter relative path.
//!
//! The index holds **no `uniffi` / no `tokio`** types (Constitution V), so this
//! whole suite runs under plain `cargo test` with no FFI toolchain. The public
//! shape (path / name / breadcrumb-able rel / score per hit) is deliberately
//! projectable onto the FFI contract's `SearchHit` later (T073) without importing
//! any FFI machinery here.

// Integration tests assert on their own fixtures; the workspace denies these in
// library code, so scope the allowance to this test module.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures and results"
)]

use emend_core::index::Index;

/// Insert one entry from its location-relative path, deriving the absolute path
/// by joining onto a fixed fake root. The index never touches disk (it is a pure
/// in-memory structure), so these paths need not exist — identity is the string.
fn insert_rel(index: &mut Index, rel: &str) {
    let abs = format!("/root/{rel}");
    index.insert(&abs, rel);
}

/// Collect just the matched relative paths, in ranked order, for a query.
fn ranked_rels(index: &Index, query: &str, limit: usize) -> Vec<String> {
    index
        .query(query, limit)
        .into_iter()
        .map(|hit| hit.rel_path)
        .collect()
}

// ---------------------------------------------------------------------------
// (1) Incremental, no-rescan updates (FR-017a)
// ---------------------------------------------------------------------------

#[test]
fn insert_adds_one_entry_without_rebuild() {
    let mut index = Index::new();
    let rebuilds_before = index.rebuild_count();

    insert_rel(&mut index, "notes/foo.md");

    // One entry added, and NOT via a full rescan: the rebuild counter is flat.
    assert_eq!(index.len(), 1);
    assert_eq!(index.rebuild_count(), rebuilds_before);
    assert_eq!(ranked_rels(&index, "foo", 10), vec!["notes/foo.md"]);
}

#[test]
fn create_then_query_reflects_the_new_file_incrementally() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/alpha.md");
    let rebuilds = index.rebuild_count();

    // Before: a search for "beta" finds nothing.
    assert!(ranked_rels(&index, "beta", 10).is_empty());

    // A single create updates the index in place (no rescan).
    insert_rel(&mut index, "notes/beta.md");
    assert_eq!(index.rebuild_count(), rebuilds, "create must not rebuild");
    assert_eq!(index.len(), 2);

    // After: the new file is now findable.
    assert_eq!(ranked_rels(&index, "beta", 10), vec!["notes/beta.md"]);
}

#[test]
fn delete_removes_one_entry_without_rebuild() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");
    insert_rel(&mut index, "notes/bar.md");
    let rebuilds = index.rebuild_count();

    index.remove("/root/notes/foo.md");

    assert_eq!(index.rebuild_count(), rebuilds, "delete must not rebuild");
    assert_eq!(index.len(), 1);
    // `foo` is gone; `bar` remains.
    assert!(ranked_rels(&index, "foo", 10).is_empty());
    assert_eq!(ranked_rels(&index, "bar", 10), vec!["notes/bar.md"]);
}

#[test]
fn rename_updates_only_the_affected_entry_without_rebuild() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");
    insert_rel(&mut index, "notes/keep.md");
    let rebuilds = index.rebuild_count();

    // Rename foo.md -> renamed.md (same folder). One entry mutates in place.
    index.rename(
        "/root/notes/foo.md",
        "/root/notes/renamed.md",
        "notes/renamed.md",
    );

    assert_eq!(index.rebuild_count(), rebuilds, "rename must not rebuild");
    assert_eq!(index.len(), 2, "rename keeps the entry count constant");

    // Old name no longer resolves; new name does; the untouched sibling is intact.
    assert!(ranked_rels(&index, "foo", 10).is_empty());
    assert_eq!(ranked_rels(&index, "renamed", 10), vec!["notes/renamed.md"]);
    assert_eq!(ranked_rels(&index, "keep", 10), vec!["notes/keep.md"]);
}

#[test]
fn move_keeps_basename_changes_rel_path_without_rebuild() {
    let mut index = Index::new();
    insert_rel(&mut index, "inbox/foo.md");
    let rebuilds = index.rebuild_count();

    // Move foo.md from inbox/ to archive/ — basename unchanged, rel path changes.
    index.rename(
        "/root/inbox/foo.md",
        "/root/archive/foo.md",
        "archive/foo.md",
    );

    assert_eq!(index.rebuild_count(), rebuilds, "move must not rebuild");
    assert_eq!(index.len(), 1);
    assert_eq!(ranked_rels(&index, "foo", 10), vec!["archive/foo.md"]);
    // The cross-segment query now matches the NEW path, not the old one.
    assert_eq!(ranked_rels(&index, "ar/foo", 10), vec!["archive/foo.md"]);
}

#[test]
fn rename_onto_occupied_destination_reclaims_the_victim() {
    let mut index = Index::new();
    // Two distinct files with distinct basenames.
    insert_rel(&mut index, "notes/alpha.md");
    insert_rel(&mut index, "notes/beta.md");
    let rebuilds = index.rebuild_count();
    assert_eq!(index.len(), 2);

    // Rename alpha.md ONTO beta.md's already-indexed path. The displaced
    // `beta` entry (the "victim") must be reclaimed, not orphaned: no stale
    // duplicate left behind in any of the three stores.
    index.rename(
        "/root/notes/alpha.md",
        "/root/notes/beta.md",
        "notes/beta.md",
    );

    // Still an incremental op (no rescan), and the over-counted slot was reclaimed.
    assert_eq!(index.rebuild_count(), rebuilds, "rename must not rebuild");
    assert_eq!(
        index.len(),
        1,
        "the victim slot must be reclaimed, not orphaned"
    );

    // `resolve_name` returns exactly the one renamed entry — no stale duplicate.
    assert_eq!(
        index.resolve_name("beta"),
        vec!["/root/notes/beta.md"],
        "exactly one entry resolves to beta — the moved file, not a duplicate"
    );
    // The source name is fully gone.
    assert!(index.resolve_name("alpha").is_empty());

    // A fuzzy query for beta returns a single hit (the reclaimed victim would
    // otherwise surface as a second, identical result).
    assert_eq!(ranked_rels(&index, "beta", 10), vec!["notes/beta.md"]);
}

// ---------------------------------------------------------------------------
// (2) Fuzzy subsequence matching on basename AND relative path (FR-017)
// ---------------------------------------------------------------------------

#[test]
fn basename_subsequence_matches() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");

    // `foo` matches notes/foo.md (FR-017 example).
    assert_eq!(ranked_rels(&index, "foo", 10), vec!["notes/foo.md"]);
    // A non-contiguous subsequence of the basename still matches (`fo` of `foo`).
    assert_eq!(ranked_rels(&index, "fo", 10), vec!["notes/foo.md"]);
}

#[test]
fn cross_segment_path_subsequence_matches() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");

    // `nt/foo` matches notes/foo.md as a subsequence of the relative path
    // (the literal example in FR-017a). It is NOT a subsequence of the bare
    // basename `foo.md`, so this can only pass via the relative-path haystack.
    assert_eq!(ranked_rels(&index, "nt/foo", 10), vec!["notes/foo.md"]);
}

#[test]
fn non_subsequence_query_does_not_match() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");

    // `xyz` is not a subsequence of the basename or the relative path.
    assert!(ranked_rels(&index, "xyz", 10).is_empty());
}

// ---------------------------------------------------------------------------
// (3) Ranking: basename matches and shorter paths win (FR-017)
// ---------------------------------------------------------------------------

#[test]
fn basename_match_outranks_path_only_match() {
    let mut index = Index::new();
    // `report` is the BASENAME of the first, but only appears as a folder segment
    // in the second (whose basename is `summary`).
    insert_rel(&mut index, "report.md");
    insert_rel(&mut index, "report/summary.md");

    let ranked = ranked_rels(&index, "report", 10);
    assert_eq!(
        ranked.first().map(String::as_str),
        Some("report.md"),
        "a basename match must outrank a path-only match"
    );
    // Both are findable; ordering is what we assert.
    assert!(ranked.contains(&"report/summary.md".to_owned()));
}

#[test]
fn shorter_path_breaks_ties_among_basename_matches() {
    let mut index = Index::new();
    // Both basenames are exactly `foo.md` (equal basename match); the shorter
    // relative path should rank first.
    insert_rel(&mut index, "foo.md");
    insert_rel(&mut index, "a/b/c/foo.md");

    let ranked = ranked_rels(&index, "foo", 10);
    assert_eq!(
        ranked.first().map(String::as_str),
        Some("foo.md"),
        "shorter path wins the tie between equal basename matches"
    );
}

#[test]
fn query_respects_limit() {
    let mut index = Index::new();
    for i in 0..10 {
        insert_rel(&mut index, &format!("notes/foo{i}.md"));
    }
    // All ten contain `foo`, but the caller asked for at most 3.
    assert_eq!(index.query("foo", 3).len(), 3);
}

// ---------------------------------------------------------------------------
// Wiki-link resolution: O(1) exact name map (FR-019a, shaped here, used later)
// ---------------------------------------------------------------------------

#[test]
fn resolve_name_finds_paths_by_basename() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");
    insert_rel(&mut index, "archive/foo.md");
    insert_rel(&mut index, "notes/bar.md");

    // Both `foo` notes resolve by stem; the deterministic disambiguation policy
    // (path tie-break) is the resolver's job (FR-019a) — the index just provides
    // the O(1) candidate set. We assert the candidate set, order-independently.
    let mut foos = index.resolve_name("foo");
    foos.sort();
    assert_eq!(foos, vec!["/root/archive/foo.md", "/root/notes/foo.md"]);

    assert_eq!(index.resolve_name("bar"), vec!["/root/notes/bar.md"]);
    assert!(index.resolve_name("missing").is_empty());
}

#[test]
fn resolve_name_updates_after_rename() {
    let mut index = Index::new();
    insert_rel(&mut index, "notes/foo.md");

    // After a rename the old name no longer resolves and the new one does — the
    // name map is maintained incrementally alongside the haystack.
    index.rename("/root/notes/foo.md", "/root/notes/baz.md", "notes/baz.md");

    assert!(index.resolve_name("foo").is_empty());
    assert_eq!(index.resolve_name("baz"), vec!["/root/notes/baz.md"]);
}
