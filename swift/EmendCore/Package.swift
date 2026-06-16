// swift-tools-version: 6.0
// EmendCore — local SwiftPM package wrapping the Rust core (research §C9).
//
// Layout:
//   Sources/EmendCoreFFI/   generated UniFFI Swift + headers/modulemap (by scripts/build-xcframework.sh)
//   Sources/EmendCore/      hand-written Swift wrappers re-exporting a clean API
//
// The binaryTarget is enabled once scripts/build-xcframework.sh has produced
// EmendCore.xcframework. Until then the package exposes only the source target
// so the app project can be wired incrementally.
//
// IMPORTANT (research §A1/§C9): the FFI bindings target must use
//   SWIFT_DEFAULT_ACTOR_ISOLATION = nonisolated
// or UniFFI-generated async/sync interop miscompiles under Swift 6.

import PackageDescription

let package = Package(
    name: "EmendCore",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "EmendCore", targets: ["EmendCore"])
    ],
    targets: [
        // .binaryTarget(
        //     name: "EmendCoreFFIBinary",
        //     path: "../../xcframework/EmendCore.xcframework"
        // ),
        // .target(
        //     name: "EmendCoreFFI",
        //     dependencies: ["EmendCoreFFIBinary"],
        //     swiftSettings: [.unsafeFlags(["-default-isolation", "nonisolated"])]
        // ),
        .target(
            name: "EmendCore"
            // dependencies: ["EmendCoreFFI"]
        ),
        .testTarget(name: "EmendCoreTests", dependencies: ["EmendCore"])
    ]
)
