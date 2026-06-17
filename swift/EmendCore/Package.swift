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
// NOTE (research §A1/§C9): the UniFFI-generated bindings are not Swift-6
// strict-concurrency clean (foreign-trait callback VTables hold process-lifetime
// static pointers), so the EmendCoreFFI target is compiled in Swift 5 language
// mode; EmendCore and the app stay in Swift 6. Separately, the uniffi #2818
// MainActor-default miscompile does not apply: Swift 6.0.3 already defaults to
// `nonisolated` (the `-default-isolation` flag is 6.2+) and this package never
// opts into MainActor-default, so the hot path stays synchronous off the main
// actor. Revisit both if the toolchain or a target enables MainActor-default.

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
            dependencies: ["EmendCoreFFIBinary"],
            // The generated UniFFI bindings are not Swift-6 strict-concurrency
            // clean — foreign-trait callback VTables use process-lifetime static
            // pointers (non-Sendable). Compile the generated target in Swift 5
            // language mode; EmendCore and the app stay Swift 6. The Sendable
            // protocol contracts the bindings declare (AiSink/SearchSink/…) are
            // still enforced on the Swift-6 side that implements them.
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .target(
            name: "EmendCore",
            dependencies: ["EmendCoreFFI"]
        ),
        .testTarget(name: "EmendCoreTests", dependencies: ["EmendCore"])
    ]
)
