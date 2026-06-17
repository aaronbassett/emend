//! T096 — embed resolution with cycle + depth guards (US5 · FR-021/021a) and
//! per-parent nested anchoring (FR-019a).
//!
//! `![[embed]]` inlines another note's content into the preview. Two hazards the
//! spec calls out (FR-021a, Edge Cases "Embed cycles / depth"):
//!
//! 1. **Cycles must terminate.** `A` embeds `B` embeds `A` must NOT loop forever;
//!    the recursion stops and degrades gracefully (the re-entered note is shown
//!    as an unresolved/already-expanded placeholder rather than recursed into).
//! 2. **Depth is bounded.** A long, acyclic embed chain (`A`→`B`→`C`→…) stops at
//!    a maximum depth (default 8, research §D), again degrading gracefully.
//!
//! A third correctness property (FR-019a) is that a *nested* embed resolves
//! relative to its **immediate parent** note, not the top document — proven by
//! [`nested_embed_resolves_relative_to_immediate_parent`].
//!
//! These tests drive the core embed expander
//! ([`emend_core::parse::embed::expand_embeds`]) directly so the bounds are proven
//! without standing up the whole comrak preview path. The expander is given a
//! resolver closure (`(target, from_note) → (source text, resolved path)`),
//! mirroring how the preview wires it to the workspace index + on-disk reads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::parse::embed::{expand_embeds, EmbedOptions, MAX_EMBED_DEPTH};
use std::collections::HashMap;

/// A toy in-memory note store: name (normalized as the user types it) → source.
/// The resolver returns the note's raw Markdown, or `None` if it doesn't exist.
fn store(notes: &[(&str, &str)]) -> HashMap<String, String> {
    notes
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// A name-keyed resolver mirroring the `(target, from_note) -> Option<(source,
/// resolved_path)>` contract. The cycle/depth tests below don't depend on
/// per-parent anchoring, so the resolved path is just the target name — enough
/// for the expander to recurse and the guards to bound it. The per-parent
/// anchoring behaviour itself is proven by
/// [`nested_embed_resolves_relative_to_immediate_parent`], whose resolver keys
/// off `from_note`.
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
fn a_simple_embed_is_inlined() {
    let notes = store(&[("child", "child body\n")]);
    let out = expand_embeds(
        "before ![[child]] after\n",
        "/note.md",
        &EmbedOptions::default(),
        &mut resolver(&notes),
    );
    assert!(
        out.contains("child body"),
        "embed should inline content: {out}"
    );
    assert!(out.contains("before"));
    assert!(out.contains("after"));
    // The raw embed token must be gone (replaced by the content).
    assert!(
        !out.contains("![[child]]"),
        "raw embed token should be replaced: {out}"
    );
}

#[test]
fn unresolved_embed_degrades_gracefully() {
    // No matching note → the expander must not loop or panic; it leaves a clearly
    // unresolved marker and finishes.
    let notes = store(&[]);
    let out = expand_embeds(
        "![[does-not-exist]]\n",
        "/note.md",
        &EmbedOptions::default(),
        &mut resolver(&notes),
    );
    // Output is finite and produced (the exact placeholder text is an impl
    // detail; what matters is that it terminated and did not inline anything).
    assert!(!out.is_empty());
    assert!(!out.contains("loop"));
}

#[test]
fn direct_cycle_terminates() {
    // A embeds B; B embeds A. Without a guard this recurses forever.
    let notes = store(&[("a", "A: ![[b]]\n"), ("b", "B: ![[a]]\n")]);
    let out = expand_embeds(
        "![[a]]\n",
        "/a.md",
        &EmbedOptions::default(),
        &mut resolver(&notes),
    );
    // It TERMINATED (the assertion running at all is the core proof) and produced
    // bounded output: both bodies appear at most a bounded number of times.
    assert!(out.contains("A:"), "a's body should appear: {out}");
    assert!(out.contains("B:"), "b's body should appear: {out}");
    // Output length is bounded — a runaway loop would be enormous; cap generously.
    assert!(
        out.len() < 10_000,
        "cyclic embed output must be bounded, got {} bytes",
        out.len()
    );
}

#[test]
fn self_cycle_terminates() {
    // A note that embeds itself.
    let notes = store(&[("self", "S ![[self]]\n")]);
    let out = expand_embeds(
        "![[self]]\n",
        "/self.md",
        &EmbedOptions::default(),
        &mut resolver(&notes),
    );
    assert!(out.contains('S'), "self body should appear once: {out}");
    assert!(
        out.len() < 10_000,
        "self-cycle must be bounded: {}",
        out.len()
    );
}

#[test]
fn deep_acyclic_chain_stops_at_max_depth() {
    // A chain n0 -> n1 -> n2 -> ... longer than MAX_EMBED_DEPTH. Each note embeds
    // the next and carries a unique marker token, so we can count how many were
    // expanded.
    let chain_len = MAX_EMBED_DEPTH + 5;
    let mut owned: Vec<(String, String)> = Vec::new();
    for i in 0..chain_len {
        let body = format!("M{i} ![[n{}]]\n", i + 1);
        owned.push((format!("n{i}"), body));
    }
    // The final note has no further embed (its target n{chain_len} is absent).
    let notes: HashMap<String, String> = owned.into_iter().collect();

    let out = expand_embeds(
        "![[n0]]\n",
        "/n0.md",
        &EmbedOptions::default(),
        &mut resolver(&notes),
    );

    // The first MAX_EMBED_DEPTH markers expand; markers past the depth bound do
    // NOT (the chain is cut off, degrading gracefully).
    assert!(out.contains("M0"), "the entry note expands: {out}");
    assert!(
        out.contains(&format!("M{}", MAX_EMBED_DEPTH - 1)),
        "content up to the depth bound expands"
    );
    assert!(
        !out.contains(&format!("M{}", MAX_EMBED_DEPTH + 2)),
        "content past the depth bound is NOT expanded: {out}"
    );
    assert!(out.len() < 100_000, "bounded output");
}

#[test]
fn custom_lower_max_depth_is_respected() {
    // A 5-deep chain with max_depth = 2 expands only the first couple levels.
    let notes = store(&[
        ("n0", "L0 ![[n1]]\n"),
        ("n1", "L1 ![[n2]]\n"),
        ("n2", "L2 ![[n3]]\n"),
        ("n3", "L3\n"),
    ]);
    let opts = EmbedOptions::new(2);
    let out = expand_embeds("![[n0]]\n", "/n0.md", &opts, &mut resolver(&notes));
    assert!(out.contains("L0"));
    assert!(out.contains("L1"));
    // Depth 2 means: top-level embed (n0) at depth 0, its embed (n1) at depth 1;
    // n2 would be depth 2 which is at/over the bound → not expanded.
    assert!(
        !out.contains("L3"),
        "content beyond max_depth=2 must not expand: {out}"
    );
}

#[test]
fn max_embed_depth_default_matches_spec() {
    // Research §D / spec Assumptions fix the v1 default at 8.
    assert_eq!(MAX_EMBED_DEPTH, 8);
    assert_eq!(EmbedOptions::default().max_depth, MAX_EMBED_DEPTH);
}

#[test]
fn nested_embed_resolves_relative_to_immediate_parent() {
    // FR-019a per-parent anchoring (M1). Fixture:
    //   * A lives in dir1 and embeds B (which lives in dir2).
    //   * B's source contains `![[C]]`.
    //   * BOTH dir1/C and dir2/C exist, with DISTINCT bodies.
    //
    // The `![[C]]` is written *inside B*, so it must resolve to B's sibling
    // (dir2/C), NOT to A's sibling (dir1/C). Before the M1 fix the expander
    // anchored every nested embed on the TOP document A, so it wrongly picked
    // dir1/C — this test would have inlined "C in dir1" instead of "C in dir2".
    //
    // The resolver models the FR-019a same-directory tie-break: given two
    // duplicate-basename candidates for `C`, it picks the one whose directory
    // matches `from_note`'s directory. By threading each note's *resolved* path as
    // the `from_note` for ITS embeds, the expander asks for `C` with `from_note ==
    // /dir2/B.md`, so the sibling in dir2 wins.

    // On-disk layout (paths only — the resolver maps name+dir → source).
    let a_path = "/dir1/A.md";
    let b_path = "/dir2/B.md";

    // The two duplicate-basename C notes keyed by their directory.
    let c_in_dir1 = "C in dir1\n";
    let c_in_dir2 = "C in dir2\n";

    let mut resolve = |target: &str, from_note: &str| -> Option<(String, String)> {
        // The directory of the note the embed was written in (FR-019a anchor).
        let from_dir = std::path::Path::new(from_note)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        match target {
            "B" => Some(("Body of B.\n\n![[C]]\n".to_owned(), b_path.to_owned())),
            // Two candidates for C share a basename across folders; the tie-break
            // is "same directory as the source note wins" (FR-019a). Pick the
            // candidate in `from_dir`; fall back to dir1 if neither matches (the
            // wrong-but-stable arm, which the assertion below rejects).
            "C" => {
                if from_dir == "/dir2" {
                    Some((c_in_dir2.to_owned(), "/dir2/C.md".to_owned()))
                } else if from_dir == "/dir1" {
                    Some((c_in_dir1.to_owned(), "/dir1/C.md".to_owned()))
                } else {
                    None
                }
            }
            _ => None,
        }
    };

    // A's source: an intro plus the embed of B. A is the top document, so its
    // `from_note` is A's own path (anchoring A's OWN embeds on dir1).
    let out = expand_embeds(
        "Intro of A.\n\n![[B]]\n",
        a_path,
        &EmbedOptions::default(),
        &mut resolve,
    );

    // B was inlined (its body and its nested embed both expanded).
    assert!(out.contains("Body of B."), "B should inline: {out}");
    // The nested `![[C]]` resolved to B's SIBLING (dir2/C), proving nested
    // resolution anchored on the immediate parent B, not the top document A.
    assert!(
        out.contains("C in dir2"),
        "nested embed must resolve relative to its immediate parent (B in dir2): {out}"
    );
    assert!(
        !out.contains("C in dir1"),
        "nested embed must NOT anchor on the top document A's directory (dir1): {out}"
    );
}
