# STACK

> **Status**: Greenfield / intended stack. No code exists yet. This document records
> the technology constraints stated by the user and reasonable initial choices. Items
> marked **(planning decision)** are open and will be settled in `/sdd:plan`.

## Platform / Target

- **OS**: macOS only.
- **Architecture**: Apple Silicon (arm64) only. No Intel, iOS, Windows, or Linux.
- **App type**: Native desktop application (single primary window with tabs).

## Languages

- **Rust** — core/"backend": file system access, folder/workspace watching, indexing,
  Markdown parsing, search ranking, AI (OpenAI-compatible) client, and other
  non-UI engine logic.
- **Swift / SwiftUI (+ AppKit where needed)** — frontend: native macOS UI, editor
  surface, sidebar, tabs, preview, settings, info sidebar.

## Frontend ↔ Core boundary **(planning decision)**

The Rust core and Swift UI must interoperate. Candidate approaches to evaluate in `/sdd:plan`:

- Rust compiled to a static/dynamic library exposing a C ABI, consumed from Swift,
  with a binding generator (e.g., a UniFFI-style or cbindgen-style approach), **or**
- Rust core as a local helper process the Swift app talks to over IPC.

Decision deferred; the spec is agnostic to which is chosen.

## Likely component areas (indicative, not locked)

- **Markdown engine (Rust)**: CommonMark + GFM parsing, with extensions for wiki links
  `[[…]]`, embeds `![[…]]`, tasks, and highlight `==…==`. Specific crate(s) chosen in plan.
- **Syntax highlighting**: code-block highlighting for 20+ languages (engine TBD).
- **Diagrams & math**: Mermaid diagram rendering and LaTeX-style math rendering (approach TBD —
  may require a rendering component; evaluated in plan).
- **PDF export**: render preview to PDF.
- **File watching (Rust)**: live refresh on external changes.
- **Secure storage**: macOS Keychain for the AI API key.

## AI / BYOM

- **Bring Your Own Model**: user supplies base URL + API key + model id for any
  **OpenAI Chat Completions API–compatible** endpoint (hosted or local/self-hosted).
- No bundled or managed model. No document content leaves the device unless the user
  configures a model and explicitly invokes an AI feature.

## Data / Persistence

- **No database, no sync service, no proprietary container.** Notes are plain `.md`
  files on disk. App-managed state (locations, favorites, pins, folder icons,
  typography, AI config metadata) is local app preferences; the API key lives in Keychain.

## Tooling **(to be confirmed — see spec "Development Standards")**

- Rust: `cargo` (fmt, clippy, test). Swift: Xcode toolchain (build, XCTest).
- Linting/formatting, hooks, CI, and test strategy are confirmed during `/sdd:specify`
  common-elements questions and recorded in the spec.

## Out of scope (stack)

- Cross-platform UI frameworks, Electron/web-shell approaches, cloud backends,
  and any Typefully/social-publishing integration.
