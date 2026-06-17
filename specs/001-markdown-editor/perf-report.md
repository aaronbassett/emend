# Performance Report (T131)

Performance budgets are **tracked, non-blocking** (Constitution Principle IV): this
report records measured numbers against the success-criteria budgets to surface
regressions, not to gate CI. Numbers below are from the Criterion micro-benches in
`crates/emend-bench` (`just bench`) measured on the **development Apple Silicon
machine** (macOS 14, `aarch64-apple-darwin`); CI runners and end-user machines
differ, so treat the absolute values as indicative and the *shape* (which budgets
have headroom vs. which are pressured) as the signal.

Criterion reports the **mean** with a 95% confidence interval `[lower mean upper]`.
The large-document benches use `sample_size(10)` (each 1 MiB sample is expensive),
so the CI upper bound is a coarse proxy for the p95 the budgets are stated against.

## Summary vs. budgets

| Budget | What it measures | Bench | Result (mean, CI) | Verdict |
|--------|------------------|-------|-------------------|---------|
| **SC-004** — Quick Open ≤ 100 ms p95, 10k files, warm | rank + stream a query over a 10k-entry index | `quick_open_10k/note` (matches all 10k) | **2.32 ms** [2.17, 2.49] | ✅ ~40× under budget |
| | | `quick_open_10k/note-07777` (few matches) | **382 µs** [379, 384] | ✅ |
| | | `quick_open_10k/zzqq` (no matches) | **205 µs** [203, 207] | ✅ |
| **SC-003** — typing latency ≤ 50 ms/keystroke p95 | one incremental reparse + viewport span query | `highlight/incremental_edit_64kb` (large-but-typical note) | **39.2 ms** [36.8, 43.5] | ✅ within budget for typical notes |
| | | `highlight/incremental_edit_1mb` (1 MiB worst case) | **702 ms** [624, 789] | ⚠️ over budget — see below |
| **SC-002** — open ~1 MB doc, first visible content < 500 ms p95 | in-memory open (rope build) + initial whole-document parse | `open_doc/open_and_parse_64kb` | **47.0 ms** [41.1, 53.6] | ✅ |
| | | `open_doc/open_and_parse_1mb` | **631 ms** [606, 658] | ⚠️ over budget for the parse — see below |

## Analysis

**SC-004 (Quick Open) is met with very large headroom.** The pure rank-and-stream
path clears a 10k-entry index in ~2.3 ms even for the pathological query that matches
every entry — ~40× under the 100 ms budget, and ~250–500× under it for selective or
no-match queries. No action needed.

**SC-002 / SC-003 are met for the documents users actually edit, and pressured only
at the 1 MiB extreme — both for the same root cause.** The `tree-sitter-md` grammar
this project uses is a *split* parser: its wrapper rebuilds **all** inline sub-trees
on every parse, so both the initial parse (`Highlighter::new`, SC-002) and a
single-keystroke reparse (`apply_edit`, SC-003) are **O(document size)**
(~0.4–0.6 ms/KB on this machine), not O(edit). Consequently:

- For a **large-but-typical 64 KB note**, both budgets are comfortably met: open +
  parse ≈ 47 ms (< 500 ms), per-keystroke ≈ 39 ms (< 50 ms).
- For the **1 MiB worst case**, the core parse/reparse dominates: ≈ 631 ms to open +
  parse and ≈ 702 ms per keystroke — over the respective budgets.

This is a **known, documented limitation** (called out in
`crates/emend-bench/benches/highlight.rs` and `open_doc.rs`), not a regression.

Two facts keep this from being a user-visible cliff in v1, and are why it stays a
tracked/non-blocking metric rather than a release blocker:

1. **First visible content is not gated on the parse (SC-002).** The editor seeds the
   raw text into the TextKit 2 buffer immediately on open; the tree-sitter highlight
   is *advisory* and attributed afterward. The 631 ms is the full open+parse cost —
   the "first visible content" milestone the budget targets happens earlier (the rope
   build + `setAttributedString`), so a large doc still shows text promptly and the
   syntax styling fills in.
2. **The hot edit path itself is O(log n).** The core text splice (`Document::push_edit`
   over the ropey buffer) is sub-millisecond; what costs ~700 ms at 1 MiB is the
   *advisory highlight reparse* layered on top, not the keystroke→buffer→disk path.

### Recommended follow-ups (deferred; tracked per Principle IV)

- For very large documents, **decouple the advisory highlight from the synchronous
  keystroke path** — debounce/coalesce reparses and/or run them off the main thread,
  attributing the viewport optimistically in between. This directly addresses the
  1 MiB SC-003 number without changing the buffer hot path.
- Consider a **size threshold above which highlighting is throttled or simplified**;
  combined with the FR-027a 5 MB read-only cap, this bounds worst-case reparse cost.
- Upstream/grammar: a `tree-sitter-md` build that reuses unchanged inline sub-trees
  would make the reparse genuinely O(edit) and retire this item.

## Swift `measure` tests

No Swift `measure` (XCTMetric) perf tests are included. SC-002/003/004 are measured
at the **core** layer (above), which is where the dominant cost lives and where the
measurement is deterministic and CI-portable. A Swift `measure` test would add the
TextKit layout/attributing overhead but runs only in the app-hosted bundle behind
`xcodebuild` and is sensitive to the windowing environment; the headless
`MemoryReleaseTests` (NFR-005) and `EditorPersistenceTests` cover the editor's
correctness on the same path. Adding `XCTMetric`-based keystroke-latency assertions
in the app bundle is a reasonable future enhancement but is not required to record
the budgets, which the Criterion benches already do faithfully.

## Reproduce

```bash
just bench                                   # all Criterion benches
cargo bench -p emend-bench --bench quick_open   # SC-004
cargo bench -p emend-bench --bench highlight    # SC-003
cargo bench -p emend-bench --bench open_doc     # SC-002
```
