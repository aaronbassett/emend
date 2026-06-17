import EmendCore
import Foundation

/// Debounced, durable autosave for one open document (research §D; FR-009/009a).
///
/// `noteEdit()` (called after each edit) resets a ~1.5 s idle timer; an
/// independent ~5 s hard cap guarantees a flush during continuous typing. The
/// flush runs on a private serial queue — never the main thread — because it does
/// an `F_FULLFSYNC` write. `flushNow()` forces a synchronous flush on close/quit.
///
/// Autosave failures are surfaced via `onError` (Constitution III — never lose
/// data silently); the buffer stays intact and the next edit reschedules a retry.
final class AutosaveController {
    private let handle: OpenDocHandle
    private let idleInterval: TimeInterval
    private let hardCap: TimeInterval
    private let queue = DispatchQueue(label: "com.aaronbassett.Emend.autosave")

    /// Invoked on the autosave queue when a flush throws. Hop to the main actor
    /// before touching UI.
    var onError: (@Sendable (FfiError) -> Void)?

    /// Invoked on the autosave queue after a successful flush (the file was just
    /// written) so the watcher's self-write suppression can be primed (FR-006a).
    var onFlush: (@Sendable () -> Void)?

    /// Invoked (on the autosave queue) on each edit, before the debounce — lets the
    /// live preview schedule its own (separately debounced) re-render off the same
    /// universal edit signal every change already flows through (US4, research §B1).
    var onEdit: (@Sendable () -> Void)?

    private var idleItem: DispatchWorkItem?
    private var hardCapItem: DispatchWorkItem?
    private var discarded = false

    init(handle: OpenDocHandle, idleInterval: TimeInterval = 1.5, hardCap: TimeInterval = 5.0) {
        self.handle = handle
        self.idleInterval = idleInterval
        self.hardCap = hardCap
    }

    /// Register that the document changed; (re)arm the debounce. Cheap and
    /// non-blocking — safe to call from the per-keystroke path on the main thread.
    func noteEdit() {
        queue.async { [weak self] in
            guard let self else { return }
            onEdit?()
            idleItem?.cancel()
            let idle = DispatchWorkItem { [weak self] in self?.fire() }
            idleItem = idle
            queue.asyncAfter(deadline: .now() + idleInterval, execute: idle)
            if hardCapItem == nil {
                let cap = DispatchWorkItem { [weak self] in self?.fire() }
                hardCapItem = cap
                queue.asyncAfter(deadline: .now() + hardCap, execute: cap)
            }
        }
    }

    /// Flush immediately (close/quit). Blocks until the write completes.
    func flushNow() {
        queue.sync { performFlush() }
    }

    /// Permanently discard this controller's pending writes (the local buffer is
    /// being thrown away on reload-from-disk). Synchronous so any *queued* flush
    /// is skipped and no flush lands on a soon-to-be-closed handle; the controller
    /// is single-use after this.
    func discard() {
        queue.sync {
            discarded = true
            idleItem?.cancel()
            idleItem = nil
            hardCapItem?.cancel()
            hardCapItem = nil
        }
    }

    // MARK: - Private (all on `queue`)

    private func fire() {
        performFlush()
    }

    private func performFlush() {
        guard !discarded else { return }
        idleItem?.cancel()
        idleItem = nil
        hardCapItem?.cancel()
        hardCapItem = nil
        do {
            try handle.flush()
            onFlush?()
        } catch let error as FfiError {
            onError?(error)
        } catch {
            // FFI methods only throw FfiError; nothing else is expected.
        }
    }
}
