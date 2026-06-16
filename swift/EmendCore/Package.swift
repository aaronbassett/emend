// swift-tools-version: 6.0
// EmendCore — local SwiftPM package wrapping the Rust core (research §C9).
//
// Layout:
//   Sources/EmendCoreFFI/   generated UniFFI Swift bindings (by scripts/build-xcframework.sh; git-ignored)
//   Sources/EmendCore/      hand-written Swift wrappers re-exporting a clean API
//   ../../xcframework/EmendCore.xcframework  static lib + `emend_ffiFFI` C module (git-ignored)
//
// Run `just xcframework` (or scripts/build-xcframework.sh) to (re)generate the
// xcframework + bindings before building — they are the build output of the Rust
// crate and are not committed.
//
// NOTE (research §A1/§C9): the Swift-6 UniFFI interop miscompile (uniffi #2818)
// only triggers when a module opts INTO `MainActor`-default isolation. Swift
// 6.0.3 (Xcode 16.2) already defaults to `nonisolated` (the `-default-isolation`
// flag that would force it does not exist until Swift 6.2), and this package
// never opts into MainActor-default — so the generated bindings are nonisolated
// as required, and the hot path stays synchronous and callable off the main
// actor. Revisit if the toolchain or a future target enables MainActor-default.

import PackageDescription

let package = Package(
    name: "EmendCore",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "EmendCore", targets: ["EmendCore"])
    ],
    targets: [
        .binaryTarget(
            name: "EmendCoreFFIBinary",
            path: "../../xcframework/EmendCore.xcframework"
        ),
        .target(
            name: "EmendCoreFFI",
            dependencies: ["EmendCoreFFIBinary"]
        ),
        .target(
            name: "EmendCore",
            dependencies: ["EmendCoreFFI"]
        ),
        .testTarget(name: "EmendCoreTests", dependencies: ["EmendCore"])
    ]
)
