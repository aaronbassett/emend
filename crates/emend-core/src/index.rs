//! The workspace search index (US2 · FR-017, FR-017a, FR-018; research §B2).
//!
//! Quick Open and wiki-link resolution both need to answer "which notes match
//! this?" fast, over a workspace that grows into the tens of thousands of files
//! (FR-018). This module is the **derived, in-memory haystack** that backs both
//! (data-model "WorkspaceIndex") — never authoritative, always rebuildable from
//! the files on disk, and maintained **incrementally** so a single
//! create/rename/move/delete updates it in O(1)-ish time without re-walking the
//! tree (FR-017a).
//!
//! It is a synchronous, pure-data structure: like [`crate::workspace`] it holds
//! **no `uniffi` and no `tokio`** types (Constitution V), so it is unit/
//! integration-testable with plain `cargo test`. The later streaming Quick Open
//! *driver* (T073) layers the full `nucleo` worker-pool engine on top; here we
//! use only the lighter `nucleo-matcher` matching primitive (no threadpool, no
//! async). The per-hit shape ([`SearchHit`]) projects cleanly onto the FFI
//! contract's `SearchHit` (§5) later, without importing any FFI machinery.
//!
//! ## Three stores stitched together (research §B2 / data-model)
//!
//! 1. **`entries`** — an append-only arena `Vec<Option<Entry>>` keyed by
//!    [`PathId`] (a `u32` index). This is the fuzzy *haystack* that
//!    [`Index::query`] ranks. Removal writes a **tombstone** (`None`) in place
//!    rather than shifting the vector, so ids stay stable and remove is O(1).
//! 2. **`path_map`** — `HashMap<PathBuf, PathId>` from the canonical absolute
//!    path to its arena slot, for **O(1) event dispatch**: a watcher (or the
//!    workspace) names a changed file by path, and we jump straight to its entry
//!    to update/tombstone it — no scan.
//! 3. **`name_map`** — `HashMap<NormName, Vec<PathId>>` from a normalized
//!    basename **stem** to every entry sharing it, for **O(1) wiki-link name→
//!    path resolution** (FR-019a). This is exact/deterministic, deliberately
//!    *separate* from fuzzy ranking (research §B2).
//!
//! ## Ranking (FR-017)
//!
//! A query is matched as a fuzzy subsequence against two haystacks per entry —
//! the **basename** and the **location-relative path** — and the entry's score
//! is the better of the two, with a **boost when the basename carries the
//! match** and a small bonus for **shorter relative paths**. So `foo` matches
//! `notes/foo.md`, `nt/foo` matches `notes/foo.md` (relative-path subsequence),
//! a basename hit outranks a path-only hit, and a shorter path breaks ties.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A stable handle to an [`Index`] entry: an index into the [`Index::entries`]
/// arena. A `u32` (not `usize`) so the structure is compact at tens-of-thousands
/// scale and the type projects cleanly across the FFI boundary later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct PathId(u32);

/// One indexed file: its canonical absolute path, basename, and the
/// location-relative path the fuzzy ranker matches against.
#[derive(Debug, Clone)]
struct Entry {
    /// Absolute (canonical) path on disk — the identity and what callers open.
    path: String,
    /// Basename (final path component), e.g. `foo.md`.
    name: String,
    /// Path relative to the location root, e.g. `notes/foo.md` — the second
    /// fuzzy haystack so cross-segment queries like `nt/foo` match (FR-017a).
    rel_path: String,
}

/// One ranked Quick Open result (FR-017; projects onto the FFI `SearchHit`).
///
/// `score` is the fuzzy-match rank — **higher ranks first**. The concrete scale
/// is an implementation detail of the ranker (callers compare, not interpret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Absolute path to the matched note (what the caller opens).
    pub path: String,
    /// Basename / display name, e.g. `foo.md`.
    pub name: String,
    /// Location-relative path, e.g. `notes/foo.md` (the breadcrumb source).
    pub rel_path: String,
    /// Fuzzy-match score; higher ranks first.
    pub score: u32,
}

/// The in-memory workspace search index (data-model "WorkspaceIndex").
///
/// Maintained incrementally via [`Index::insert`] / [`Index::remove`] /
/// [`Index::rename`] (each touches only the affected entry — no rescan,
/// FR-017a) and queried via [`Index::query`] (Quick Open) and
/// [`Index::resolve_name`] (wiki-link resolution).
///
/// Holds no `uniffi`/`tokio` types (Constitution V). Not [`Sync`]-shared
/// internally — the owner serializes access (the FFI driver will wrap it).
///
/// The fuzzy `Matcher` is intentionally **not** a field: it carries mutable
/// scratch buffers, and keeping it owned would force [`Index::query`] to take
/// `&mut self` (or hide it behind a `RefCell`). A query allocates a fresh
/// matcher instead — cheap relative to scoring the haystack — so `query` stays a
/// `&self` read and the struct derives `Debug` cleanly.
#[derive(Debug, Default)]
pub struct Index {
    /// Append-only arena; `None` slots are tombstones from removals.
    entries: Vec<Option<Entry>>,
    /// Canonical path → arena slot, for O(1) dispatch on a named change.
    path_map: HashMap<PathBuf, PathId>,
    /// Normalized basename stem → entries sharing it, for O(1) name resolution.
    name_map: HashMap<String, Vec<PathId>>,
    /// Count of live (non-tombstone) entries — kept in sync with the maps so
    /// [`Index::len`] is O(1).
    live: usize,
    /// How many full rebuilds have happened (seeding via [`Index::rebuild`]).
    /// Incremental ops never bump this — the invariant FR-017a's "no full
    /// rescan" rests on, and what the tests assert against.
    rebuilds: u64,
}

impl Index {
    /// Create an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // -- Introspection --------------------------------------------------------

    /// The number of live entries (tombstones excluded). O(1).
    #[must_use]
    pub fn len(&self) -> usize {
        self.live
    }

    /// Whether the index has no live entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// How many **full rebuilds** have occurred. Only the seeding/`clear` path
    /// bumps this; every incremental op leaves it untouched. Callers (and tests)
    /// use it to verify FR-017a's "a single change MUST NOT require a full
    /// workspace rescan" structurally, independent of wall-clock timing.
    #[must_use]
    pub fn rebuild_count(&self) -> u64 {
        self.rebuilds
    }

    // -- Incremental maintenance (FR-017a) ------------------------------------

    /// Add (or update in place) the entry for `path`, whose location-relative
    /// path is `rel_path`. **Incremental and O(1)-ish:** touches only this
    /// entry's slot and the two maps — never re-walks the workspace.
    ///
    /// If `path` is already indexed, its entry is replaced in place (an
    /// idempotent re-insert, e.g. a metadata-only change) keeping its [`PathId`]
    /// stable. Otherwise a new slot is allocated (reusing nothing — tombstones
    /// are left for `clear`/compaction, keeping ids monotonic and stable).
    pub fn insert(&mut self, path: &str, rel_path: &str) {
        let name = basename(path);
        let entry = Entry {
            path: path.to_owned(),
            name: name.clone(),
            rel_path: rel_path.to_owned(),
        };
        let key = PathBuf::from(path);

        if let Some(&id) = self.path_map.get(&key) {
            // Re-insert over an existing path: update in place, re-keying the
            // name map if the basename changed.
            let old_name = self.entries[id.0 as usize].as_ref().map(|e| e.name.clone());
            if let Some(old) = old_name {
                if old != name {
                    self.unmap_name(&old, id);
                    self.map_name(&name, id);
                }
            } else {
                // Slot was a tombstone under this key (shouldn't happen since we
                // drop the path_map entry on remove, but stay correct): re-live.
                self.map_name(&name, id);
                self.live += 1;
            }
            self.entries[id.0 as usize] = Some(entry);
            return;
        }

        // New path: append a slot.
        let id = PathId(u32::try_from(self.entries.len()).unwrap_or(u32::MAX));
        self.entries.push(Some(entry));
        self.path_map.insert(key, id);
        self.map_name(&name, id);
        self.live += 1;
    }

    /// Remove the entry for `path`, if present. **Incremental and O(1)-ish:**
    /// tombstones the one slot and drops it from both maps — no rescan, no
    /// vector shift (so every other entry's [`PathId`] stays valid).
    ///
    /// Removing an unknown path is a silent no-op: a delete event for something
    /// already gone (a double-delete, or an external delete we never indexed)
    /// must not error — the index is best-effort derived state, not a ledger.
    pub fn remove(&mut self, path: &str) {
        let key = PathBuf::from(path);
        let Some(id) = self.path_map.remove(&key) else {
            return;
        };
        if let Some(entry) = self.entries[id.0 as usize].take() {
            self.unmap_name(&entry.name, id);
            self.live -= 1;
        }
    }

    /// Move/rename the entry at `old_path` to `new_path` with new location-
    /// relative path `new_rel`. **Incremental and O(1)-ish:** re-keys the one
    /// entry across both maps in place; the [`PathId`] (arena slot) is preserved.
    ///
    /// Handles all of US2's move-shaped events uniformly (FR-017a): a same-folder
    /// rename (basename changes), a cross-folder move (rel path changes,
    /// basename may stay), or both. If `old_path` was not indexed this degrades
    /// to a plain [`Index::insert`] of `new_path` (we still end in the right
    /// state without a rescan).
    pub fn rename(&mut self, old_path: &str, new_path: &str, new_rel: &str) {
        let old_key = PathBuf::from(old_path);
        let Some(id) = self.path_map.remove(&old_key) else {
            // Nothing to move from — treat as a create of the destination.
            self.insert(new_path, new_rel);
            return;
        };

        let new_name = basename(new_path);
        let old_name = self.entries[id.0 as usize].as_ref().map(|e| e.name.clone());

        // Re-key the name map if the basename changed.
        if let Some(old) = old_name {
            if old != new_name {
                self.unmap_name(&old, id);
                self.map_name(&new_name, id);
            }
        } else {
            self.map_name(&new_name, id);
            self.live += 1;
        }

        // Re-key the path map and rewrite the entry in place.
        self.path_map.insert(PathBuf::from(new_path), id);
        self.entries[id.0 as usize] = Some(Entry {
            path: new_path.to_owned(),
            name: new_name,
            rel_path: new_rel.to_owned(),
        });
    }

    /// Replace the entire contents from `(abs_path, rel_path)` pairs. This is the
    /// **only** full-rebuild path (seeding from [`crate::workspace::Workspace::collect_files`],
    /// or a recovery rebuild) — it bumps [`Index::rebuild_count`]. Everyday
    /// changes go through the incremental ops above and must NOT call this
    /// (FR-017a).
    pub fn rebuild<I, A, R>(&mut self, items: I)
    where
        I: IntoIterator<Item = (A, R)>,
        A: AsRef<str>,
        R: AsRef<str>,
    {
        self.entries.clear();
        self.path_map.clear();
        self.name_map.clear();
        self.live = 0;
        self.rebuilds += 1;
        for (abs, rel) in items {
            self.insert(abs.as_ref(), rel.as_ref());
        }
    }

    // -- Querying (FR-017) ----------------------------------------------------

    /// Return up to `limit` ranked [`SearchHit`]s for `query`, best first
    /// (FR-017). An empty query yields nothing (Quick Open shows its own recents
    /// list in that case, not the whole workspace).
    ///
    /// Each entry is scored against two haystacks — its **basename** and its
    /// **relative path** — as a fuzzy subsequence; the entry takes the better of
    /// the two, **boosted** when the basename carries the match and nudged up for
    /// a **shorter path**. Non-matching entries are dropped. Ties are broken
    /// deterministically (shorter rel path, then path string) so results are
    /// stable across runs.
    #[must_use]
    pub fn query(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }

        // A fresh matcher per query keeps `query` a `&self` method (callers may
        // hold the index behind a shared lock and query concurrently-ish without
        // a `&mut`). Construction is cheap relative to scoring a 10k haystack.
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());

        // Parse the needle once. `CaseMatching::Smart` = case-insensitive unless
        // the query has uppercase; `Normalization::Smart` folds diacritics
        // unless the query carries them. Both are the expected Quick Open feel.
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);

        let mut scored: Vec<SearchHit> = Vec::new();
        let mut name_buf: Vec<char> = Vec::new();
        let mut rel_buf: Vec<char> = Vec::new();

        for slot in &self.entries {
            let Some(entry) = slot else { continue };

            let name_hay = Utf32Str::new(&entry.name, &mut name_buf);
            let name_score = pattern.score(name_hay, &mut matcher);

            let rel_hay = Utf32Str::new(&entry.rel_path, &mut rel_buf);
            let rel_score = pattern.score(rel_hay, &mut matcher);

            let Some(combined) = combine_scores(name_score, rel_score, &entry.rel_path) else {
                continue;
            };
            scored.push(SearchHit {
                path: entry.path.clone(),
                name: entry.name.clone(),
                rel_path: entry.rel_path.clone(),
                score: combined,
            });
        }

        // Higher score first; ties → shorter rel path → path string, so the
        // order is total and deterministic (no arbitrary HashMap-iteration leak).
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.rel_path.len().cmp(&b.rel_path.len()))
                .then_with(|| a.path.cmp(&b.path))
        });
        scored.truncate(limit);
        scored
    }

    // -- Wiki-link resolution (FR-019a) ---------------------------------------

    /// Resolve a wiki-link target by note name: the absolute paths of every entry
    /// whose basename **stem** normalizes to `name` (case-insensitive, extension
    /// ignored). **O(1)** in the name map, independent of workspace size.
    ///
    /// This returns the full *candidate set* without choosing among duplicates —
    /// the deterministic disambiguation policy (path tie-break) is the resolver's
    /// job (FR-019a). An unknown name yields an empty vec.
    #[must_use]
    pub fn resolve_name(&self, name: &str) -> Vec<String> {
        let key = normalize_name(name);
        let Some(ids) = self.name_map.get(&key) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| self.entries[id.0 as usize].as_ref())
            .map(|e| e.path.clone())
            .collect()
    }

    // -- Name-map helpers -----------------------------------------------------

    /// Add `id` under the normalized stem of `name` in the name map.
    fn map_name(&mut self, name: &str, id: PathId) {
        self.name_map
            .entry(normalize_name(name))
            .or_default()
            .push(id);
    }

    /// Drop `id` from the name map's bucket for `name`'s normalized stem,
    /// removing the bucket entirely when it empties (so `resolve_name` of a gone
    /// name is a clean miss, not an empty vec).
    fn unmap_name(&mut self, name: &str, id: PathId) {
        let key = normalize_name(name);
        if let Some(ids) = self.name_map.get_mut(&key) {
            ids.retain(|&existing| existing != id);
            if ids.is_empty() {
                self.name_map.remove(&key);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions (scoring + normalization)
// ---------------------------------------------------------------------------

/// Combine the basename score and the relative-path score into a single rank
/// (FR-017), or `None` if neither haystack matched.
///
/// `nucleo`'s `Pattern::score` returns `u32` where higher = better. We:
/// * take the **better of basename vs path**,
/// * add a fixed **basename boost** when the basename matched at all (so a
///   basename hit outranks a path-only hit of similar fuzzy quality), and
/// * add a small **short-path bonus** that decreases with rel-path length (so
///   among comparable matches the shallower file wins the tie).
fn combine_scores(name_score: Option<u32>, rel_score: Option<u32>, rel_path: &str) -> Option<u32> {
    // A space-separated `nucleo` pattern is an AND of atoms; a query that is not
    // a subsequence of *either* haystack scores `None` on both → no match.
    if name_score.is_none() && rel_score.is_none() {
        return None;
    }

    /// Flat reward for the match living in the basename (FR-017 "favors basename
    /// matches"). Large enough to dominate fuzzy-quality jitter between a
    /// basename match and a path-only match, but it is a *bonus*, not a separate
    /// tier, so a vastly better path match can still surface.
    const BASENAME_BOOST: u32 = 1_000;
    /// Cap for the short-path bonus; subtracting rel-path length from it yields a
    /// small, monotonically-decreasing nudge toward shallower files.
    const SHORT_PATH_BONUS_MAX: u32 = 64;

    let base = name_score.unwrap_or(0).max(rel_score.unwrap_or(0));
    let basename_boost = if name_score.is_some() {
        BASENAME_BOOST
    } else {
        0
    };
    // Shorter rel paths get a larger bonus; saturate so a very deep path simply
    // gets zero rather than wrapping.
    let len = u32::try_from(rel_path.len()).unwrap_or(u32::MAX);
    let short_path_bonus = SHORT_PATH_BONUS_MAX.saturating_sub(len);

    Some(
        base.saturating_add(basename_boost)
            .saturating_add(short_path_bonus),
    )
}

/// The basename (final path component) of an absolute or relative path string.
/// Falls back to the whole string if there is no separator.
fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Normalize a name to its wiki-link resolution key (FR-019a): the basename
/// **stem** (final extension dropped), lowercased. So `Foo.md`, `foo`, and
/// `FOO.MD` all resolve together, matching how users write `[[foo]]`.
fn normalize_name(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
    stem.to_lowercase()
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

    use super::{basename, normalize_name, Index};

    #[test]
    fn basename_strips_directory() {
        assert_eq!(basename("/root/notes/foo.md"), "foo.md");
        assert_eq!(basename("foo.md"), "foo.md");
        assert_eq!(basename("a/b/c"), "c");
    }

    #[test]
    fn normalize_name_drops_extension_and_lowercases() {
        assert_eq!(normalize_name("Foo.md"), "foo");
        assert_eq!(normalize_name("foo"), "foo");
        assert_eq!(normalize_name("FOO.MD"), "foo");
        // Multi-dot keeps the stem before the LAST dot.
        assert_eq!(normalize_name("a.tar.gz"), "a.tar");
    }

    #[test]
    fn rebuild_is_the_only_path_that_bumps_the_counter() {
        let mut index = Index::new();
        assert_eq!(index.rebuild_count(), 0);

        // Incremental ops never bump the rebuild counter.
        index.insert("/root/a.md", "a.md");
        index.rename("/root/a.md", "/root/b.md", "b.md");
        index.remove("/root/b.md");
        assert_eq!(index.rebuild_count(), 0);

        // Only an explicit rebuild does.
        index.rebuild([("/root/c.md", "c.md")]);
        assert_eq!(index.rebuild_count(), 1);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn reinsert_same_path_updates_in_place() {
        let mut index = Index::new();
        index.insert("/root/a.md", "a.md");
        // Re-insert the SAME path with a different rel path (e.g. location root
        // moved): no new slot, count stays 1.
        index.insert("/root/a.md", "deep/a.md");
        assert_eq!(index.len(), 1);
        assert_eq!(index.rebuild_count(), 0);
        let hit = index.query("a", 5).into_iter().next().unwrap();
        assert_eq!(hit.rel_path, "deep/a.md");
    }
}
