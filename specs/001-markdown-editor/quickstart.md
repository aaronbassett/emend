# Quickstart: Emend

Native macOS (Apple Silicon) Markdown editor — Rust core + Swift/SwiftUI UI.

## Prerequisites

- macOS 14+ on **Apple Silicon**
- **Xcode 16.2+** (Swift 6.0) — `xcode-select --install` for the CLI tools
- **Rust** (stable ≥ 1.85; `rustup` recommended) with `clippy` + `rustfmt` components
  - `rustup component add clippy rustfmt`
  - Apple-Silicon target is the host default (`aarch64-apple-darwin`)
- **Tooling**: `brew install lefthook just swiftformat swiftlint`
- For binding generation: the `uniffi-bindgen-swift` binary ships inside the `uniffi` crate behind the `cli` feature — install it pinned to the workspace `uniffi` version: `cargo install uniffi --version 0.31.1 --features cli --bin uniffi-bindgen-swift`

## One-time setup

```bash
git clone <repo> && cd emend
lefthook install        # or: just hooks   — installs pre-commit + commit-msg hooks
```

## Available commands (`just`)

| Command | What it does |
|---------|--------------|
| `just build` | Build the Rust workspace |
| `just test` | Run core tests (`cargo test`) |
| `just fmt` / `just fmt-check` | Format / check Rust formatting |
| `just clippy` | Lint Rust (`-D warnings`) |
| `just bench` | Criterion benches (added during implementation) |
| `just xcframework` | Build the Rust core into `EmendCore.xcframework` + generate Swift bindings |
| `just swift-lint` | SwiftFormat + SwiftLint (`--strict`) |
| `just xcodeproj` | Generate `app/Emend/Emend.xcodeproj` from `project.yml` (the `.xcodeproj` is git-ignored) |
| `just app-test` | Build & test the macOS app (runs `xcframework` + `xcodeproj` first, then `xcodebuild test`) |
| `just check` | Full pre-push gate (fmt + clippy + test + swift-lint) — mirrors CI |

Raw equivalents: `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.

## Project layout

```
crates/emend-core   # all logic (files, watcher, index, parse, search, ai) — NO FFI dep
crates/emend-ffi    # thin UniFFI shim → XCFramework
swift/EmendCore     # SwiftPM package wrapping the XCFramework + generated bindings
app/Emend           # Xcode macOS app (created below)
scripts/build-xcframework.sh
```

See [plan.md](./plan.md) and [research.md](./research.md) for the architecture, and [contracts/ffi-interface.md](./contracts/ffi-interface.md) for the Swift↔Rust API.

## Build & run the app (bootstrap, performed during `/sdd:implement`)

1. **Wire UniFFI** in `crates/emend-ffi`: add `uniffi.workspace = true`, call `uniffi::setup_scaffolding!()`, and add the first `#[uniffi::export]` functions from the FFI contract.
2. **Build the core framework**: `just xcframework` → produces `xcframework/EmendCore.xcframework` and generated Swift in `swift/EmendCore/Sources/EmendCoreFFI/`. Then uncomment the `binaryTarget`/`EmendCoreFFI` target in `swift/EmendCore/Package.swift`.
   - **Important**: the generated-bindings target must build with `SWIFT_DEFAULT_ACTOR_ISOLATION = nonisolated` (research §A1/§C9), or UniFFI interop miscompiles under Swift 6.
3. **Generate the app project** with `just xcodeproj` — the target (macOS app, SwiftUI lifecycle, deployment macOS 14, arch arm64, the local `EmendCore` package dependency, and the app-hosted `EmendTests` bundle) is declared in `app/Emend/project.yml` and produced reproducibly by `xcodegen`; the resulting `.xcodeproj` is git-ignored (Constitution VII). Test sources are folder-globbed from `EmendTests/`, so new test files are picked up automatically on regeneration.
4. **Enable the App Sandbox** with entitlements: `com.apple.security.app-sandbox`, `com.apple.security.files.user-selected.read-write`, `com.apple.security.files.bookmarks.app-scope` (research §A4). Prototype the security-scoped-bookmark ↔ Rust file-watcher handshake first — it is the highest-risk integration.
5. **Bundle preview assets**: vendor Mermaid.js + KaTeX (JS/CSS/fonts) into the app bundle for offline rendering (research §C2); set a CSP that blocks remote loads.
6. **Run**: `just xcodeproj` then `xcodebuild -project app/Emend/Emend.xcodeproj -scheme Emend -destination 'platform=macOS,arch=arm64'`, or open the generated project in Xcode.

## Testing

- **Rust core** (no Xcode needed): `cargo test` — unit/integration for parsing, indexing/search, AI client, file watching; `cargo bench` for the latency budgets (`highlight` → SC-003, `quick_open` → SC-004, `open_doc` → SC-002; tracked, non-blocking per Constitution IV).
- **Swift** (`just app-test`): a single **app-hosted** `EmendTests` bundle that `@testable import`s `Emend` and drives the real logic headlessly — attribute mapping, FFI/bookmark/Keychain wrappers, scroll-anchor math, editor flows via `EditorCoordinator` + the pure transforms, preview/PDF export, and tab lifecycle/memory release. There is **no XCUITest target by design** (Constitution VII): CI runs GUI/signing-free (`CODE_SIGNING_ALLOWED=NO`), so a `bundle.ui-testing` runner can't bootstrap; editor flows are exercised headlessly rather than through a launched GUI.

## CI

`.github/workflows/ci.yml` runs on `macos-14` (Apple Silicon): Rust fmt/clippy/test, Swift format+lint (and app build/test once the Xcode project exists), and a Conventional-Commits check on PRs.
