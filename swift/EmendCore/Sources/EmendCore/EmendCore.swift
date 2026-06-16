// EmendCore — clean Swift API over the Rust core.
//
// During /sdd:implement this re-exports the generated UniFFI surface
// (import EmendCoreFFI) and wraps streaming callbacks/handles in idiomatic
// Swift (e.g. AsyncStream adapters per research §A1). Skeleton placeholder:

public enum EmendCore {
    /// ABI version of the core this package targets.
    public static let abiVersion: UInt32 = 1
}
