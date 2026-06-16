# Quickstart: Emend

Native macOS (Apple Silicon) Markdown editor — Rust core + Swift/SwiftUI UI.

## Prerequisites

- macOS 14+ on **Apple Silicon**
- **Xcode 16.2+** (Swift 6.0) — `xcode-select --install` for the CLI tools
- **Rust** (stable ≥ 1.85; `rustup` recommended) with `clippy` + `rustfmt` components
  - `rustup component add clippy rustfmt`
  - Apple-Silicon target is the host default (`aarch64-apple-darwin`)
- **Tooling**: `brew install lefthook just swiftformat swiftlint`
- For binding generation (added during implementation): `cargo install uniffi-bindgen-swift` (match the pinned `uniffi` version)

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
| `just swift-lint` | SwiftFormat + SwiftLint |
| `just app-test` | Build & test the macOS app (needs the Xcode project) |
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
3. **Create the app target** in `app/Emend` (Xcode → macOS App, SwiftUI lifecycle, min deployment macOS 14, arch arm64). Add the local `EmendCore` package as a dependency.
4. **Enable the App Sandbox** with entitlements: `com.apple.security.app-sandbox`, `com.apple.security.files.user-selected.read-write`, `com.apple.security.files.bookmarks.app-scope` (research §A4). Prototype the security-scoped-bookmark ↔ Rust file-watcher handshake first — it is the highest-risk integration.
5. **Bundle preview assets**: vendor Mermaid.js + KaTeX (JS/CSS/fonts) into the app bundle for offline rendering (research §C2); set a CSP that blocks remote loads.
6. **Run**: `xcodebuild -scheme Emend -destination 'platform=macOS,arch=arm64'` or run from Xcode.

## Testing

- **Rust core** (no Xcode needed): `cargo test` — unit/integration for parsing, indexing/search, AI client, file watching; `cargo bench` for the latency budgets.
- **Swift**: XCTest unit target (attribute-computation, FFI mapping, Keychain wrapper, bookmark resolution, scroll-anchor math — testable headlessly) + XCUITest UI target (tabs, sidebar, ⌘P, typing, export) + `measure` perf tests asserting the ≤50 ms keystroke budget.

## CI

`.github/workflows/ci.yml` runs on `macos-14` (Apple Silicon): Rust fmt/clippy/test, Swift format+lint (and app build/test once the Xcode project exists), and a Conventional-Commits check on PRs.
