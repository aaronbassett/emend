#!/usr/bin/env bash
#
# Build the Rust core (emend-ffi) into EmendCore.xcframework + generate the
# UniFFI Swift bindings consumed by the macOS app (research §A1, §C9).
#
# Apple-Silicon only → single aarch64-apple-darwin slice.
#
# Prerequisites (wired during /sdd:implement):
#   - emend-ffi depends on `uniffi` and calls `uniffi::setup_scaffolding!()`
#   - `cargo install uniffi-bindgen-swift` (matching the pinned uniffi version)
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="aarch64-apple-darwin"
LIB="libemend_ffi.a"
OUT="$ROOT/xcframework"
BINDINGS="$ROOT/swift/EmendCore/Sources/EmendCoreFFI"

echo "▶ Building emend-ffi (release, $TARGET)…"
cargo build --release -p emend-ffi --target "$TARGET"

if ! command -v uniffi-bindgen-swift >/dev/null 2>&1; then
  echo "✗ uniffi-bindgen-swift not found. Install it (matching the pinned uniffi version):"
  echo "    cargo install uniffi-bindgen-swift"
  echo "  (Skipping bindings/xcframework — Rust build above still succeeded.)"
  exit 0
fi

echo "▶ Generating Swift bindings…"
mkdir -p "$BINDINGS"
uniffi-bindgen-swift \
  --swift-sources --headers --modulemap \
  --out-dir "$BINDINGS" \
  "target/$TARGET/release/$LIB"

echo "▶ Assembling XCFramework…"
rm -rf "$OUT"
xcodebuild -create-xcframework \
  -library "target/$TARGET/release/$LIB" -headers "$BINDINGS" \
  -output "$OUT/EmendCore.xcframework"

echo "✓ Done: $OUT/EmendCore.xcframework  +  bindings in $BINDINGS"
