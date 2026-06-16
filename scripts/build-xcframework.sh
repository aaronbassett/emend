#!/usr/bin/env bash
#
# Build the Rust core (emend-ffi) into EmendCore.xcframework + generate the
# UniFFI Swift bindings consumed by the macOS app (research §A1, §C9).
#
# Apple-Silicon only → single aarch64-apple-darwin slice.
#
# Outputs (both git-ignored — regenerated here and in CI, single source of truth
# is the Rust crate):
#   xcframework/EmendCore.xcframework                      static lib + `emend_ffiFFI` C module
#   swift/EmendCore/Sources/EmendCoreFFI/emend_ffi.swift   generated UniFFI Swift bindings
#
# Prerequisites:
#   - emend-ffi uses UniFFI proc-macro mode (`uniffi::setup_scaffolding!()`), so
#     there is no UDL file — bindings are extracted from the compiled library.
#   - uniffi-bindgen-swift pinned to the workspace uniffi version:
#       cargo install uniffi --version 0.31.1 --features cli --bin uniffi-bindgen-swift
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="aarch64-apple-darwin"
LIB_PATH="target/$TARGET/release/libemend_ffi.a"
OUT="$ROOT/xcframework"
SWIFT_BINDINGS_DIR="$ROOT/swift/EmendCore/Sources/EmendCoreFFI"
# The generated Swift imports `emend_ffiFFI`; the xcframework's clang module must
# match that name (see the generated `#if canImport(emend_ffiFFI)`).
MODULE_NAME="emend_ffiFFI"

echo "▶ Building emend-ffi (release, $TARGET)…"
cargo build --release -p emend-ffi --target "$TARGET"

if ! command -v uniffi-bindgen-swift >/dev/null 2>&1; then
  echo "✗ uniffi-bindgen-swift not found. The binary ships inside the uniffi crate"
  echo "  behind the 'cli' feature — install it pinned to the workspace uniffi version:"
  echo "    cargo install uniffi --version 0.31.1 --features cli --bin uniffi-bindgen-swift"
  echo "  (Skipping bindings/xcframework — the Rust build above still succeeded.)"
  exit 0
fi

echo "▶ Generating Swift bindings + C module ($MODULE_NAME)…"
GEN_DIR="$(mktemp -d)"
HEADERS_DIR="$(mktemp -d)"
trap 'rm -rf "$GEN_DIR" "$HEADERS_DIR"' EXIT
uniffi-bindgen-swift \
  --swift-sources --headers --modulemap \
  --module-name "$MODULE_NAME" --modulemap-filename module.modulemap \
  "$LIB_PATH" "$GEN_DIR"

# Generated Swift → the SwiftPM source target (compiled by the app/package).
mkdir -p "$SWIFT_BINDINGS_DIR"
rm -f "$SWIFT_BINDINGS_DIR"/*.swift
cp "$GEN_DIR"/*.swift "$SWIFT_BINDINGS_DIR/"

# C header + modulemap → the XCFramework Headers, so the binaryTarget exposes the
# `emend_ffiFFI` clang module that the generated Swift imports.
cp "$GEN_DIR"/*.h "$HEADERS_DIR/"
cp "$GEN_DIR"/module.modulemap "$HEADERS_DIR/"

echo "▶ Assembling XCFramework…"
rm -rf "$OUT/EmendCore.xcframework"
mkdir -p "$OUT"
xcodebuild -create-xcframework \
  -library "$LIB_PATH" -headers "$HEADERS_DIR" \
  -output "$OUT/EmendCore.xcframework"

echo "✓ Done:"
echo "    $OUT/EmendCore.xcframework"
echo "    $SWIFT_BINDINGS_DIR/ (generated Swift bindings)"
