import EmendCoreFFI

// AsyncStream adapters over the UniFFI foreign-trait sinks (research §A1).
//
// The Rust core delivers AI tokens and incremental search results through the
// `AiSink` / `SearchSink` callbacks; these adapters bridge those one-per-item
// callbacks to Swift `AsyncStream`s so call sites can `for await …` over them.
//
// Contract semantics honoured here (contracts/ffi-interface.md):
//   - exactly one terminal callback per stream (`onDone` success / `onError`
//     failure) → each adapter finishes its stream exactly once;
//   - callbacks are non-reentrant → the adapters only yield/finish, never call
//     back into the core.
//
// Cancellation: UniFFI does NOT bridge Swift `Task` cancellation to the Rust
// future, so each factory exposes an `onTerminate` hook. The AI/search call
// sites wire it to the Rust-owned handle's `cancel()` (see `CancellationHandle`)
// so tearing down the `AsyncStream` supersedes the in-flight Rust work.

// MARK: - AI token stream

/// Bridges `AiSink` to an `AsyncThrowingStream<String, Error>` of token deltas.
/// Finishes normally on `onDone`; throws the mapped `FfiError` on `onError`.
public final class AiStreamAdapter: AiSink {
    private let continuation: AsyncThrowingStream<String, Error>.Continuation

    init(continuation: AsyncThrowingStream<String, Error>.Continuation) {
        self.continuation = continuation
    }

    public func onToken(text: String) {
        continuation.yield(text)
    }

    public func onDone(full _: String) {
        continuation.finish()
    }

    public func onError(err: FfiError) {
        continuation.finish(throwing: err)
    }
}

public enum AiStream {
    /// Make an `AiSink` paired with the token stream it drives.
    ///
    /// Pass the returned `sink` to the Rust `summarize_document` export and
    /// `for try await` over `stream`. `onTerminate` runs when the stream is torn
    /// down (consumer cancels / finishes) — wire it to the returned AI handle's
    /// `cancel()` to supersede the Rust work.
    public static func make(
        onTerminate: @escaping @Sendable () -> Void = {}
    ) -> (sink: AiStreamAdapter, stream: AsyncThrowingStream<String, Error>) {
        let (stream, continuation) = AsyncThrowingStream.makeStream(of: String.self)
        continuation.onTermination = { _ in onTerminate() }
        return (AiStreamAdapter(continuation: continuation), stream)
    }
}

// MARK: - Search results stream

/// Bridges `SearchSink` to an `AsyncStream<[SearchHit]>` of ranked batches.
/// Finishes on `onDone`. Search has no error terminal — supersede via the
/// handle's `cancel()`, which simply ends the stream.
public final class SearchStreamAdapter: SearchSink {
    private let continuation: AsyncStream<[SearchHit]>.Continuation

    init(continuation: AsyncStream<[SearchHit]>.Continuation) {
        self.continuation = continuation
    }

    public func onResults(batch: [SearchHit]) {
        continuation.yield(batch)
    }

    public func onDone() {
        continuation.finish()
    }
}

public enum SearchStream {
    /// Make a `SearchSink` paired with the results stream it drives.
    /// `onTerminate` runs on teardown — wire it to the search handle's `cancel()`.
    public static func make(
        onTerminate: @escaping @Sendable () -> Void = {}
    ) -> (sink: SearchStreamAdapter, stream: AsyncStream<[SearchHit]>) {
        let (stream, continuation) = AsyncStream.makeStream(of: [SearchHit].self)
        continuation.onTermination = { _ in onTerminate() }
        return (SearchStreamAdapter(continuation: continuation), stream)
    }
}

// MARK: - Error mapping

public extension FfiError {
    /// Concise, user-facing copy for the boundary error.
    ///
    /// `FfiError` already conforms to `LocalizedError` (UniFFI-generated), but
    /// its `errorDescription` is `String(reflecting:)` — a debug dump. This is
    /// the copy the UI shows. The API key is never part of any `FfiError`
    /// payload (NFR-006), so these strings are safe to surface.
    var userMessage: String {
        switch self {
        case let .NotFound(path):
            "“\(path)” could not be found."
        case let .PermissionDenied(path):
            "Permission denied for “\(path)”."
        case let .IoFailure(path, detail):
            "Could not read or write “\(path)”: \(detail)"
        case let .NameCollision(path):
            "An item named “\(path)” already exists."
        case let .NoteTooLarge(path, bytes, limit):
            "“\(path)” is too large to edit (\(bytes) bytes; limit \(limit))."
        case let .InvalidConfig(detail):
            "Invalid configuration: \(detail)"
        case .AiNotConfigured:
            "No AI provider is configured."
        case .AiTimeout:
            "The AI request timed out."
        case .AiCancelled:
            "The AI request was cancelled."
        case let .AiOversizedInput(bytes, limit):
            "The document is too large for AI (\(bytes) bytes; limit \(limit))."
        case let .AiHttp(status, detail):
            "AI service error (\(status)): \(detail)"
        case let .AiStreamMalformed(detail):
            "The AI response was malformed: \(detail)"
        case let .Internal(detail):
            "An internal error occurred: \(detail)"
        }
    }
}
