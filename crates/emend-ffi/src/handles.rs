//! Async infrastructure scaffolding for the FFI boundary (research §A1, §B7;
//! contract `contracts/ffi-interface.md` §5/§7 and "Global rules").
//!
//! This module is **plumbing only** — it stands up the pieces that the later
//! AI (T112) and Quick Open search (T073) tasks plug into. It deliberately
//! holds **no** search or AI logic:
//!
//! 1. [`runtime`] — a process-wide, long-lived multi-thread `tokio` runtime the
//!    core owns (research §A1: "The Rust core owns a single long-lived `tokio`
//!    multi-thread runtime"). Sync `#[uniffi::export]` functions that need to
//!    kick off cancellable async work `spawn` onto it; UniFFI itself does not
//!    manage a runtime (`docs/manual/src/internals/async-overview.md`:
//!    "UniFFI can't rely on a Rust async runtime ... Keep a reference to the
//!    runtime"), so a [`std::sync::OnceLock`] holding the `Runtime` is the
//!    idiomatic approach.
//!
//! 2. [`CancellationHandle`] — the Rust-owned handle pattern for cancellation.
//!    UniFFI does **not** wire Swift `Task` cancellation to the Rust future
//!    (research §A1, "Risks/notes"), so cancellation must cross the boundary as
//!    an explicit object carrying a [`CancellationToken`] with a `cancel()`
//!    method. The contract's concrete `SearchHandle`/`AiHandle` (§5/§7) follow
//!    this exact shape; see [`CancellationHandle`] for how those later tasks
//!    reuse it.
//!
//! 3. Foreign-trait streaming **sinks** ([`AiSink`], [`SearchSink`],
//!    [`DocObserver`]) — traits the **Swift side implements and Rust calls**,
//!    one invocation per streamed item. These use
//!    `#[uniffi::export(with_foreign)]`, the current/preferred attribute for
//!    foreign-implemented traits in UniFFI 0.31 (verified against
//!    `mozilla/uniffi-rs@v0.31.1`: `docs/manual/src/foreign_traits.md` and
//!    `docs/manual/src/types/callback_interfaces.md`, which marks the older
//!    `callback_interface` attribute "(soft) deprecated"). A `with_foreign`
//!    trait is handed to Rust as `Arc<dyn Trait>` (vs `Box<dyn Trait>` for the
//!    legacy `callback_interface`), which is what later tasks store in their
//!    spawned task to deliver results.
//!
//! ## Streaming / callback semantics these sinks must honor
//!
//! From the contract's "Global rules" (`contracts/ffi-interface.md`) — the
//! later AI/search tasks that *drive* these sinks are responsible for:
//!
//! - **Exactly one terminal callback per stream**: `on_done` on success or the
//!   error terminal on failure — never both, never neither. After a `cancel()`
//!   or supersede the terminal is the error terminal (`AiCancelled`) and **no
//!   further** `on_token`/`on_results` fire.
//! - **Non-reentrant**: the foreign (Swift) side MUST NOT call back into the
//!   core from inside a callback; it queues the work instead. The Rust driver
//!   therefore treats each callback as a fire-and-(maybe)-forget notification
//!   and never blocks on the foreign side re-entering.
//! - **Complete UTF-8 tokens**: `AiSink::on_token` only ever receives whole
//!   UTF-8 strings; the SSE parser (T112) buffers partial bytes across chunks
//!   so a split code point is never emitted. This is a driver obligation, not
//!   something the trait signature can enforce, but it is documented here so the
//!   driver author sees it next to the trait.
//!
//! ## Foreign-trait method requirements (UniFFI 0.31, verified)
//!
//! - Methods take `&self` and pass every argument **by value** (references in
//!   foreign-trait methods are unsupported — uniffi-rs#2263).
//! - The trait must be explicitly `Send + Sync`: UniFFI *enforces* that all
//!   interfaces are `Send + Sync` but does not add the bound for you
//!   (`docs/manual/src/types/interfaces.md`).
//! - A rich (non-`flat`) `#[derive(uniffi::Error)]` enum is a full bidirectional
//!   FFI type and may be passed **by value as a parameter** — not only in the
//!   `Err` position. Confirmed against the `error-types` fixture
//!   (`fixtures/error-types/src/lib.rs`: `fn get_tuple(t: Option<TupleError>)
//!   -> TupleError`, where `TupleError` derives only `uniffi::Error`) and the
//!   macro (`uniffi_macros/src/enum_.rs`: the error derive routes through
//!   `enum_or_error_ffi_converter_impl`, generating the same full `FfiConverter`
//!   as the `Enum` derive). So [`AiSink::on_error`] can take [`FfiError`]
//!   directly. (`#[uniffi(flat_error)]` enums generate only the lower side and
//!   would NOT work as inputs — [`FfiError`] is intentionally rich.)

use crate::error::FfiError;
use std::sync::OnceLock;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

/// Process-wide, long-lived multi-thread `tokio` runtime owned by the Rust core
/// (research §A1).
///
/// Held in a [`OnceLock`] so the first access builds it and every subsequent
/// access shares the same runtime — async AI/search work all runs on one
/// thread-pool for the life of the process. The runtime is never dropped (no
/// teardown path crosses the FFI boundary), which is exactly the lifetime we
/// want: a single editor process keeps it until exit.
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Accessor for the shared [`tokio` runtime](RUNTIME), building it on first use.
///
/// Later sync exports (AI summary in T112, Quick Open in T073) call this to get
/// a runtime to `spawn` their cancellable work onto, e.g.:
///
/// ```ignore
/// let token = handle.token();            // CancellationHandle, handed back to Swift
/// let sink = sink;                        // Arc<dyn AiSink> from the foreign side
/// runtime().spawn(async move {
///     // body wrapped per research §B7 so a panic becomes a terminal error,
///     // not a process abort — see `crate::panic::contain_panic`.
///     tokio::select! {
///         () = token.cancelled() => sink.on_error(FfiError::AiCancelled),
///         result = do_work() => match result {
///             Ok(full) => sink.on_done(full),
///             Err(e)   => sink.on_error(e),
///         }
///     }
/// });
/// ```
///
/// Most exports should prefer the fallible [`try_runtime`], which reports a
/// construction failure to Swift as a normal error. This infallible accessor
/// exists for call sites where async work is structurally required and the
/// only failure mode (the OS refusing to create *any* tokio runtime) is an
/// unrecoverable environment fault rather than a per-request error.
///
/// On the supported target (`aarch64-apple-darwin`) runtime construction does
/// not fail. The implementation tries the multi-thread builder, then a
/// current-thread builder, before treating a double failure as the
/// unrecoverable fault it is.
#[must_use]
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        // `get_or_init` requires an infallible closure, but every tokio runtime
        // constructor is fallible. We degrade gracefully — multi-thread first,
        // then a current-thread runtime (which still drives spawned tasks) — so
        // a worker-pool failure does not take the app down. Only a host that
        // cannot create *any* runtime reaches the final arm.
        build_runtime()
            .or_else(|_| build_current_thread_runtime())
            .unwrap_or_else(|e| unrecoverable_runtime(&e))
    })
}

/// Last-resort handling when the OS cannot construct any tokio runtime — a
/// genuinely unrecoverable environment fault during one-time process init (not
/// an FFI-crossing call), so an abort here is the honest outcome.
#[allow(
    clippy::panic,
    reason = "one-time process init: the host cannot host any tokio runtime; \
              this is not an FFI-boundary call and there is no panic-free \
              Runtime constructor to fall back to (NFR-003 governs boundary \
              calls, which use try_runtime)"
)]
fn unrecoverable_runtime(err: &std::io::Error) -> ! {
    panic!("emend-core: cannot create a tokio runtime: {err}");
}

/// Fallible runtime accessor — builds the shared runtime on first use and
/// surfaces a construction failure as [`FfiError::Internal`] instead of
/// collapsing to a degraded runtime.
///
/// Preferred by exports that want to report "could not start async work" to
/// Swift as a normal error. Once initialized (by either accessor), subsequent
/// calls return the shared runtime.
///
/// # Errors
///
/// Returns [`FfiError::Internal`] if the multi-thread runtime cannot be built
/// (OS thread-spawn failure). This does not occur on a healthy host.
pub fn try_runtime() -> Result<&'static Runtime, FfiError> {
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let built = build_runtime().map_err(|e| FfiError::Internal {
        detail: format!("failed to build tokio runtime: {e}"),
    })?;
    // Another thread may have initialized concurrently; `get_or_init` keeps the
    // first winner and drops our `built` if we lost — both outcomes are fine.
    Ok(RUNTIME.get_or_init(|| built))
}

/// Build the standard long-lived multi-thread runtime (research §A1).
fn build_runtime() -> std::io::Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("emend-core")
        .build()
}

/// Degraded current-thread runtime, used only if [`build_runtime`] fails inside
/// the infallible [`runtime`] accessor.
///
/// A current-thread runtime still drives spawned tasks (cooperatively), so the
/// app stays functional — just without a worker pool — rather than aborting on
/// a transient worker-pool failure.
fn build_current_thread_runtime() -> std::io::Result<Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_name("emend-core")
        .build()
}

/// Rust-owned cancellation handle exported to Swift (research §A1; contract
/// §5/§7).
///
/// Wraps a [`CancellationToken`]. Async exports return one of these (or a
/// purpose-named wrapper that holds the same token) so the foreign side can
/// `cancel()` in-flight work — UniFFI does not bridge Swift `Task` cancellation
/// to the Rust future, so this explicit handle is the only cancellation path.
///
/// ## Reuse by `SearchHandle` / `AiHandle` (T073 / T112)
///
/// The contract names two concrete handles, `SearchHandle` and `AiHandle`, each
/// with a `cancel()` method. Those later tasks may either:
///
/// - return a `CancellationHandle` directly (simplest — the token is all the
///   handle needs to carry); or
/// - define their own `#[derive(uniffi::Object)]` wrapper that holds a
///   [`CancellationToken`] (via [`CancellationHandle::token`] or a freshly
///   created token) plus any extra per-stream state, exposing the same
///   `cancel()` semantics.
///
/// Either way the pattern is identical: the spawned task holds the *child* token
/// (or a clone) and `select!`s on `token.cancelled()` to honor the contract's
/// "after `cancel()`, terminal is the cancel error and no further items fire".
#[derive(Debug, uniffi::Object)]
pub struct CancellationHandle {
    token: CancellationToken,
}

#[uniffi::export]
impl CancellationHandle {
    /// Cancel the work associated with this handle.
    ///
    /// Idempotent: calling it more than once (or after the work already
    /// finished) is a no-op. The spawned task observes the cancellation via its
    /// clone of the token (`token.cancelled().await` / `is_cancelled()`),
    /// emits the single terminal cancel error, and stops emitting items
    /// (contract "Global rules").
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

impl CancellationHandle {
    /// Create a handle around a fresh, uncancelled token.
    ///
    /// Rust-internal (not exported): later tasks construct the handle, clone its
    /// [`token`](Self::token) into the spawned task, and return the handle to
    /// Swift.
    #[must_use]
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Build a handle around an existing token (e.g. a child token derived from
    /// a parent scope) so a supersede can cancel a group.
    #[must_use]
    pub fn from_token(token: CancellationToken) -> Self {
        Self { token }
    }

    /// Clone of the underlying token, for the spawned task to `select!` on.
    ///
    /// `CancellationToken` clones share cancellation state, so the task's clone
    /// observes a `cancel()` made through this handle.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Default for CancellationHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Foreign-trait streaming sink for AI responses (contract §7).
///
/// **Swift implements this; Rust calls it.** The AI driver (T112) invokes
/// `on_token` once per SSE delta, then exactly one terminal: `on_done` on
/// success or `on_error` on failure/cancellation.
///
/// `Send + Sync` is required (and written explicitly) because UniFFI enforces
/// it for all interfaces but does not add the bound automatically.
///
/// ## Semantics the driver must uphold (contract "Global rules")
///
/// - Exactly **one** terminal (`on_done` xor `on_error`) per stream.
/// - After a [`CancellationHandle::cancel`], the terminal is
///   `on_error(FfiError::AiCancelled)` and **no** further `on_token` fires.
/// - Each `text` passed to [`on_token`](Self::on_token) is a **complete** UTF-8
///   string (the SSE parser buffers partial bytes across chunks).
/// - Callbacks are **non-reentrant**: Swift must not call back into the core
///   from inside one of these methods.
#[uniffi::export(with_foreign)]
pub trait AiSink: Send + Sync {
    /// One streamed token (a complete UTF-8 SSE delta).
    fn on_token(&self, text: String);

    /// Successful terminal: the full assembled response text. Mutually
    /// exclusive with [`on_error`](Self::on_error).
    fn on_done(&self, full: String);

    /// Failure/cancellation terminal carrying the typed error.
    ///
    /// `err` is taken **by value**: a rich `#[derive(uniffi::Error)]` enum is a
    /// full bidirectional FFI type (verified — see module docs), so [`FfiError`]
    /// works directly as a parameter here. The payload never contains the API
    /// key (NFR-006).
    fn on_error(&self, err: FfiError);
}

/// One Quick Open search result (contract §5).
///
/// Plain `#[derive(uniffi::Record)]`: `String`/`u32` fields are directly
/// supported with no extra attributes. `score` is the fuzzy-match rank
/// (higher = better; concrete scale fixed by the ranker in T073).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchHit {
    /// Absolute path to the matched note.
    pub path: String,
    /// Display name (typically the file stem).
    pub name: String,
    /// Human-readable location trail (e.g. `Work / Projects / Notes`).
    pub breadcrumb: String,
    /// Fuzzy-match score; higher ranks first.
    pub score: u32,
}

/// Foreign-trait streaming sink for Quick Open results (contract §5).
///
/// **Swift implements this; Rust calls it.** The search driver (T073) streams
/// ranked results in batches via `on_results`, then exactly one terminal
/// `on_done`. A superseded/cancelled query simply stops emitting and is
/// terminated by the new query's lifecycle (search has no error terminal in the
/// contract — cancellation is silent supersede).
///
/// `Send + Sync` is required and written explicitly (UniFFI enforces it).
/// Callbacks are non-reentrant (see [`AiSink`] / module docs).
#[uniffi::export(with_foreign)]
pub trait SearchSink: Send + Sync {
    /// A batch of ranked results. `batch` is taken by value; `Vec<SearchHit>`
    /// of a `uniffi::Record` is a supported by-value parameter.
    fn on_results(&self, batch: Vec<SearchHit>);

    /// Terminal: the query has finished streaming (or was superseded). Fires
    /// exactly once per query.
    fn on_done(&self);
}

/// Foreign-trait observer fired when a document's *derived* insight (outline,
/// stats, links) changes after edits (contract §4, FR-031a).
///
/// **Swift implements this; Rust calls it.** Fired ≤300 ms after edits so the
/// UI updates live without polling.
///
/// `Send + Sync` is required and written explicitly.
///
/// ## Payload deferred to T039
///
/// The contract's signature is `on_derived_changed(&self, h: OpenDocHandle)`,
/// but `OpenDocHandle` does not exist until the document-session task (T039)
/// introduces it. To avoid churning a placeholder type, this scaffolding ships
/// the **payload-less** form. T039 adds the `OpenDocHandle` parameter once that
/// type lands; until then a single-document caller needs no discriminator.
#[uniffi::export(with_foreign)]
pub trait DocObserver: Send + Sync {
    /// Derived insight for the (currently implicit) document changed; the UI
    /// should re-pull `outline`/`stats`/`links`.
    ///
    /// T039 will add the `h: OpenDocHandle` argument to identify *which*
    /// document changed.
    fn on_derived_changed(&self);
}

#[cfg(test)]
mod tests {
    use super::{runtime, try_runtime, CancellationHandle};

    #[test]
    fn runtime_initializes_and_is_shared() {
        let a = runtime();
        let b = runtime();
        // Same long-lived instance is handed out every time (OnceLock identity).
        assert!(
            std::ptr::eq(a, b),
            "runtime() must return the shared instance"
        );
    }

    #[test]
    fn try_runtime_returns_same_instance_as_runtime() {
        // No `expect`/`unwrap` (workspace-denied): assert success via `matches!`,
        // then compare identities only on the success path.
        let a = try_runtime();
        assert!(a.is_ok(), "runtime must build on a healthy host: {a:?}");
        if let Ok(a) = a {
            assert!(
                std::ptr::eq(a, runtime()),
                "try_runtime and runtime must share one instance"
            );
        }
    }

    #[test]
    fn runtime_can_drive_a_future() {
        // The runtime is real and can actually execute async work.
        let answer = runtime().block_on(async { 21 * 2 });
        assert_eq!(answer, 42);
    }

    #[test]
    fn cancel_marks_token_cancelled() {
        let handle = CancellationHandle::new();
        let token = handle.token();
        assert!(!token.is_cancelled(), "fresh token starts uncancelled");
        handle.cancel();
        assert!(
            token.is_cancelled(),
            "cancel() must cancel the cloned token observed by the spawned task"
        );
    }

    #[test]
    fn cancel_is_idempotent() {
        let handle = CancellationHandle::new();
        handle.cancel();
        handle.cancel(); // must not panic or misbehave
        assert!(handle.token().is_cancelled());
    }

    #[test]
    fn cancelled_future_resolves_on_the_runtime() {
        // Prove the cancellation actually drives an awaiting task to completion
        // on the shared runtime — the exact mechanism a streaming driver uses
        // to deliver the single terminal cancel error.
        let handle = CancellationHandle::new();
        let task_token = handle.token();
        let resolved = runtime().block_on(async move {
            let waiter = tokio::spawn(async move {
                task_token.cancelled().await;
                "observed-cancel"
            });
            // Cancel from "outside" (as Swift would via the handle), then await.
            handle.cancel();
            // `JoinHandle::await` yields `Result`; map a join failure to a
            // sentinel rather than `unwrap` (workspace-denied).
            waiter.await.unwrap_or("join-error")
        });
        assert_eq!(resolved, "observed-cancel");
    }

    #[test]
    fn from_token_shares_cancellation_state() {
        let token = tokio_util::sync::CancellationToken::new();
        let handle = CancellationHandle::from_token(token.clone());
        handle.cancel();
        assert!(
            token.is_cancelled(),
            "from_token handle must cancel the supplied token (group supersede)"
        );
    }
}
