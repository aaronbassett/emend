<!--
==============================================================================
SYNC IMPACT REPORT — /sdd:constitution
==============================================================================
Version change: (none) → 1.0.0   (initial ratification)
Bump rationale: First adoption of project governance — MAJOR baseline.

Principles defined (7):
  I.   Plain Files, User Sovereignty
  II.  Local-First & Privacy by Default
  III. Never Lose User Data (NON-NEGOTIABLE)
  IV.  Native Performance Is a Feature
  V.   Clean Core/UI Boundary & Crash Safety
  VI.  Minimalism & Simplicity
  VII. Quality Discipline & Reproducible Builds

Added sections:
  - Core Principles (I–VII)
  - Platform & Technology Constraints
  - Development Workflow & Quality Gates
  - Governance

Removed sections: none (initial version).

Template / artifact alignment:
  ✅ specs/001-markdown-editor/plan.md — Constitution Check updated to reference v1.0.0
  ✅ plan-template.md / spec-template.md / tasks-template.md (plugin) — constitution-agnostic
     placeholders ("[Gates determined based on constitution file]"); no change required
  ✅ CLAUDE.md — guardrails already align with Principles II–VII

Deferred TODOs: none. Ratification date = adoption date (sole maintainer accepted).
==============================================================================
-->

# Emend Constitution

Emend is a quiet, fast, native macOS (Apple Silicon) Markdown editor — a home for a
person's (and their agents') Markdown files. These principles are non-negotiable
constraints that govern every feature, plan, and change. They exist to protect the
two things that matter most: **the user's data and trust**, and **the calm,
native feel of the product**.

## Core Principles

### I. Plain Files, User Sovereignty

Notes MUST be stored as plain Markdown (`.md`) files on disk, in folders the user
chooses. The app MUST NOT introduce a proprietary database, sync service, or opaque
container as the source of truth, and MUST NOT lock content into a format only Emend
can read. Any other tool or AI agent MUST be able to read and write the same files
concurrently; Emend is one editor among many, never the gatekeeper. App-managed
metadata (locations, favorites, icons, typography) MAY live in local app storage, but
losing it MUST never harm the user's notes.

*Rationale*: The product's identity is a frictionless home for Markdown that outlives
the app itself. Lock-in would betray that promise and the "for agents and humans" goal.

### II. Local-First & Privacy by Default

Emend MUST be fully functional offline. It MUST NOT transmit document content, file
names, or telemetry off the device unless the user has explicitly configured a
bring-your-own-model (OpenAI-compatible) AI provider AND explicitly invoked an AI
action. With no AI configured, there MUST be zero outbound network connections caused
by document content. The AI API key MUST be stored only in the macOS Keychain, MUST
NOT be written to logs, crash reports, temp files, or note content, and MUST be passed
to the network layer transiently and redacted in any diagnostics. File access MUST use
least privilege (App Sandbox + security-scoped bookmarks scoped to user-added folders).

*Rationale*: Personal notes and a user-supplied secret demand secure-by-default,
opt-in-only network behavior. Privacy is a feature, not a setting buried in defaults.

### III. Never Lose User Data (NON-NEGOTIABLE)

Operations that touch user files MUST be fail-safe. Autosave MUST be atomic and
durable: a reader (the file watcher, an external tool, an agent) MUST never observe a
partially written note, and a write MUST be durable before it is reported complete.
The app MUST distinguish its own writes from third-party writes and MUST NOT echo its
own saves as external changes. When a file with unsaved edits is changed on disk by
another tool, the app MUST follow a deterministic conflict policy that preserves both
versions and lets the user choose — never a silent overwrite, never silent data loss.
Destructive actions MUST be intentional and reversible where feasible.

*Rationale*: Trust is lost permanently the first time an editor eats someone's writing.
This principle outranks performance, features, and elegance.

### IV. Native Performance Is a Feature

The product promise is "fast, polished, a pleasure to use." The editing path MUST
never be blocked by indexing, watching, search, AI, or export; long-running work MUST
be asynchronous and cancellable. Incremental computation MUST be preferred over full
recomputation (a single keystroke MUST NOT re-parse, re-index, or re-render the whole
document). The performance budgets in the spec — typing latency ≤ 50 ms p95, Quick
Open ≤ 100 ms p95 over 10k files, large-document render < 500 ms p95, derived data
(outline/stats) refresh ≤ 300 ms — are **tracked targets**: they MUST be measured and
reported on every CI run (Rust criterion benches + Swift `measure` tests), and a
regression beyond tolerance MUST be reviewed and either justified or fixed. They do not
hard-block merges, but an unexplained regression is a defect.

*Rationale*: Latency is the feel of a native app. Treating budgets as visible,
reviewed metrics keeps the product fast without imposing brittle CI gates on a solo
project.

### V. Clean Core/UI Boundary & Crash Safety

Logic MUST live in the Rust core (`emend-core`) and MUST be testable in isolation
without the FFI or Swift toolchain. The Swift/SwiftUI UI MUST be a thin consumer over a
typed, documented FFI contract; business logic MUST NOT leak into the FFI shim or the
views. No panic may unwind across the FFI boundary, and recoverable errors MUST be
surfaced as structured, typed errors the UI can render — a recoverable error MUST never
abort the app. Components MUST favor low coupling and high cohesion; the boundary's
range/coordinate contract (UTF-16 code units) MUST be honored end-to-end.

*Rationale*: A clean, crash-safe boundary is what makes a two-language native app
maintainable, debuggable, and testable — and what keeps a Rust panic from taking the
whole app down.

### VI. Minimalism & Simplicity

Emend is "simple on the surface, packed underneath." Every addition MUST justify its
complexity; the default answer to scope is subtraction. Prefer mature, widely-used
dependencies over bespoke machinery, and the simplest design that meets the
requirement. Scope discipline is binding: v1 ships only what the spec defines (e.g.,
the single AI feature is the document summary), and new surface area requires an
explicit "why now." A feature that removes steps beats one that adds them.

*Rationale*: The product's calm comes from restraint. Complexity is the enemy of both
the user experience and a solo maintainer's velocity.

### VII. Quality Discipline & Reproducible Builds

Code MUST pass formatting and linting with zero warnings before merge: rustfmt +
clippy (`-D warnings`) for Rust, SwiftFormat + SwiftLint for Swift. **Testing is
strict for the core and pragmatic for the UI**: the Rust core MUST have a test for
every behavior, written alongside or before the implementation (effectively test-first
for the core); the Swift app MUST unit-test its headless logic (attribute computation,
FFI mapping, Keychain, bookmark resolution, scroll math) and cover key flows, but is
NOT required to test-first AppKit/TextKit views. Pre-commit hooks MUST run the
fmt/lint/test gates locally, and CI MUST be green on an Apple Silicon runner before
merge. Commits MUST follow Conventional Commits, and releases MUST carry a changelog.
Dependency versions MUST be pinned and verified against their registry — never bumped
from memory.

*Rationale*: Consistent, automated quality lets a solo developer move fast without
regressions, and concentrates rigor where bugs are most expensive (the core engine).

## Platform & Technology Constraints

- **Target**: macOS 14+ on Apple Silicon (arm64) only. No Intel, iOS, Windows, or Linux.
- **Stack**: Rust core + Swift/SwiftUI (+ AppKit) UI, bridged via UniFFI (XCFramework).
  Markdown uses two engines by design — tree-sitter (incremental editor highlighting,
  advisory) and comrak (authoritative preview HTML); they MUST NOT be conflated.
- **Secrets**: AI API key in Keychain only (Principle II).
- **Dependencies**: pinned and verified; the centralized catalog in `Cargo.toml` is the
  single source of versions. Prefer first-party Apple frameworks and mature crates.
- **Out of scope** (changing these requires a constitution amendment): Typefully or any
  social-publishing integration; cloud sync or hosted accounts; a bundled/managed AI
  model; cross-platform support.

## Development Workflow & Quality Gates

- **Process**: Features follow the SDD flow — `/sdd:specify` → `/sdd:plan` →
  `/sdd:tasks` → `/sdd:implement`. Each feature gets a numbered branch and spec.
- **The merge gate** (`just check` / CI): rustfmt check, clippy `-D warnings`,
  `cargo test`, SwiftFormat/SwiftLint, XCTest (once the app target exists), and the
  Conventional-Commits check. CI MUST be green before merge.
- **Performance**: benches/`measure` tests run in CI and report against the Principle IV
  budgets; regressions are reviewed (non-blocking).
- **Security-sensitive changes**: any change that adds an outbound network call, touches
  the AI key, or alters file-write/atomicity logic MUST be explicitly reviewed against
  Principles II and III before merge.
- **Review**: as a solo project, "review" means deliberate self-review plus, for risky
  changes, an adversarial pass (e.g., a code-review agent). PRs are the integration unit
  so the CI gates apply.

## Governance

This constitution supersedes ad-hoc practice. When a principle and convenience conflict,
the principle wins; the resolution is to change the plan, not to quietly violate the
principle.

- **Amendments**: proposed as a documented change to this file, with a version bump and
  an updated Sync Impact Report. Removing or redefining a principle is a MAJOR change;
  adding or materially expanding guidance is MINOR; clarifications are PATCH.
- **Compliance**: every `/sdd:plan` MUST include a Constitution Check that evaluates the
  feature against these principles; unjustified violations block the plan. Complexity
  that conflicts with Principle VI MUST be justified in the plan's Complexity Tracking.
- **Versioning**: this document uses semantic versioning independent of the app version.

**Version**: 1.0.0 | **Ratified**: 2026-06-16 | **Last Amended**: 2026-06-16
