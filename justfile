# Emend developer commands —  `just <recipe>`  (run `just` to list).
set shell := ["bash", "-cu"]

# List recipes
default:
    @just --list

# --- Rust core -----------------------------------------------------------
# Build the whole workspace
build:
    cargo build

# Run all core tests
test:
    cargo test

# Format check (CI) / format (local)
fmt-check:
    cargo fmt --check
fmt:
    cargo fmt

# Lint with clippy, warnings as errors
clippy:
    cargo clippy --all-targets -- -D warnings

# Criterion benches (added during implementation)
bench:
    cargo bench

# --- Swift app -----------------------------------------------------------
# Build the Rust core into an XCFramework for the Swift app
xcframework:
    ./scripts/build-xcframework.sh

# Lint + format-check Swift (no-op if tools absent)
swift-lint:
    @command -v swiftformat >/dev/null && swiftformat --lint app swift || echo "install swiftformat"
    @command -v swiftlint   >/dev/null && swiftlint lint || echo "install swiftlint"

# Build & test the macOS app (requires the Xcode project; see quickstart.md)
app-test:
    xcodebuild test -scheme Emend -destination 'platform=macOS,arch=arm64' | xcpretty || true

# --- Everything ----------------------------------------------------------
# The full pre-push gate (mirrors CI)
check: fmt-check clippy test swift-lint

# One-time: install git hooks
hooks:
    lefthook install
