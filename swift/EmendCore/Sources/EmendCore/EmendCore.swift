// EmendCore — clean Swift API over the Rust core.
//
// Re-exports the UniFFI-generated surface (`EmendCoreFFI`) so consumers
// `import EmendCore` and get the whole boundary. Idiomatic Swift wrappers
// (e.g. AsyncStream adapters over the foreign-trait sinks — research §A1) are
// added here as the corresponding features land.

@_exported import EmendCoreFFI

public enum EmendCore {
    /// ABI version reported by the Rust core across the FFI boundary.
    ///
    /// Backed by the real `core_abi_version` UniFFI export, so reading it
    /// exercises the Swift↔Rust round-trip.
    public static var abiVersion: UInt32 {
        coreAbiVersion()
    }
}
