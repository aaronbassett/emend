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

# Lint + format-check Swift (skips gracefully if tools absent; mirrors CI --strict)
swift-lint:
    @if command -v swiftformat >/dev/null; then swiftformat app swift --lint; else echo "swiftformat not installed — skipping (brew install swiftformat)"; fi
    @if command -v swiftlint   >/dev/null; then swiftlint lint --strict;       else echo "swiftlint not installed — skipping (brew install swiftlint)"; fi

# Generate the Xcode project from app/Emend/project.yml (reproducible; the .xcodeproj is git-ignored)
xcodeproj:
    xcodegen generate --spec app/Emend/project.yml --project app/Emend

# Build & test the macOS app (regenerates the project first)
app-test: xcodeproj
    xcodebuild test -project app/Emend/Emend.xcodeproj -scheme Emend -destination 'platform=macOS,arch=arm64' CODE_SIGNING_ALLOWED=NO

# --- Everything ----------------------------------------------------------
# The full pre-push gate (mirrors CI)
check: fmt-check clippy test swift-lint

# One-time: install git hooks
hooks:
    lefthook install
