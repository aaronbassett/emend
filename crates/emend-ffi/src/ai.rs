//! T112 (FFI half) / T113 — the **network** orchestration of the BYOM AI summary
//! (US6 · FFI contract §7; FR-032/035/036/036a, NFR-006, SC-008/SC-009;
//! research §B5).
//!
//! This is the only place `reqwest` lives — `emend-core` stays reqwest-free AND
//! tokio-free (Constitution V; CI proves `cargo tree -p emend-core -i
//! {reqwest,tokio,uniffi}` each find nothing). The **decision logic** — SSE
//! framing, the max-input guard, the redacting key, request building — is the
//! pure [`emend_core::ai`] module; this shim only:
//!
//! 1. Validates locally **before any socket** (FR-036a/SC-008): a missing/blank
//!    key → [`FfiError::AiNotConfigured`]; an oversized document →
//!    [`FfiError::AiOversizedInput`] (via [`emend_core::ai::check_input_size`]).
//!    No request is constructed unless a config + key are supplied (zero network
//!    otherwise — SC-008).
//! 2. Spawns the streaming request on the shared `tokio` runtime
//!    ([`crate::handles::try_runtime`]) under [`crate::panic::contain_panic`]
//!    (a panic in the worker becomes a contained terminal error, never an abort —
//!    NFR-003), with:
//!    - cancellation via a [`tokio_util::sync::CancellationToken`] +
//!      `tokio::select!` (the contract's `AiHandle.cancel()` / supersede,
//!      NFR-002/FR-036a);
//!    - a **per-chunk** inactivity `tokio::time::timeout` (NOT reqwest's
//!      whole-request `.timeout()`, which fires mid-stream — research §B5);
//!    - the bytes fed through the core [`emend_core::ai::SseParser`], each delta
//!      pushed to [`AiSink::on_token`], terminated by exactly one
//!      [`AiSink::on_done`]/[`AiSink::on_error`].
//! 3. The **key crosses the boundary as a transient `String`**, is wrapped in the
//!    core [`emend_core::ai::ApiKey`] newtype (redacted `Debug`/`Display`), set
//!    ONLY on the `Authorization` header, and is **never logged or persisted**
//!    (NFR-006). Error payloads carry no key (the redacted error `detail` only).
//!
//! ## Streaming terminal semantics (contract "Global rules")
//!
//! Exactly **one** terminal per request: `on_done(full)` on success, or
//! `on_error(err)` on failure/cancellation. After `cancel()`/supersede the
//! terminal is `on_error(AiCancelled)` and **no** further `on_token` fires. Each
//! `on_token` is a complete UTF-8 string (the core parser buffers partial bytes).

use crate::error::FfiError;
use crate::handles::{try_runtime, AiSink};
use crate::panic::contain_panic;
use emend_core::ai::{
    build_auth_header, build_request_body, chat_completions_url, check_input_size, AiConfig,
    ApiKey, SseEvent, SseParser, DEFAULT_SUMMARY_SYSTEM_PROMPT,
};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::document::OpenDocHandle;

/// The BYOM connection settings for one AI request (FFI contract §7
/// `AiRequestConfig`). The FFI mirror of [`emend_core::ai::AiConfig`].
///
/// Plain `#[derive(uniffi::Record)]`: all fields are directly-supported scalars.
/// `request_timeout_ms` is applied as a **per-chunk** inactivity timeout (not a
/// whole-request deadline — research §B5); `max_input_bytes` is enforced locally
/// before any send (FR-036a).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiRequestConfig {
    /// OpenAI-compatible base URL (e.g. `https://api.openai.com/v1`).
    pub base_url: String,
    /// Model identifier.
    pub model: String,
    /// Per-chunk inactivity timeout in milliseconds.
    pub request_timeout_ms: u64,
    /// Maximum input size in bytes (rejected locally before send, FR-036a).
    pub max_input_bytes: u64,
}

impl From<AiRequestConfig> for AiConfig {
    fn from(cfg: AiRequestConfig) -> Self {
        // Destructure exhaustively so a new field forces a compile error here.
        let AiRequestConfig {
            base_url,
            model,
            request_timeout_ms,
            max_input_bytes,
        } = cfg;
        Self {
            base_url,
            model,
            request_timeout_ms,
            max_input_bytes,
        }
    }
}

/// Rust-owned handle for one in-flight AI summary (FFI contract §7:
/// `AiHandle.cancel()` → `AiCancelled`; superseding cancels the prior, NFR-002 /
/// FR-036a).
///
/// Handed to Swift as `Arc<Self>`. Holds a [`CancellationToken`] the spawned
/// streaming task `select!`s on; [`cancel`](Self::cancel) trips it so the request
/// is aborted promptly and the single terminal becomes
/// `on_error(AiCancelled)` with no further `on_token`.
#[derive(Debug, uniffi::Object)]
pub struct AiHandle {
    token: CancellationToken,
}

#[uniffi::export]
impl AiHandle {
    /// Cancel (supersede) this summary request (FFI contract §7).
    ///
    /// Idempotent: cancelling twice (or after completion) is a no-op. After this,
    /// the in-flight HTTP request is dropped, no further `on_token` fires, and the
    /// terminal is `on_error(AiCancelled)` (if it had not already completed).
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

impl AiHandle {
    /// A fresh, uncancelled handle.
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Clone of the token for the spawned task to `select!` on.
    fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

/// Start a streaming AI summary of `h`'s current document (FFI contract §7
/// `summarize_document`; FR-032/036a).
///
/// The summary text streams to `sink` token-by-token via [`AiSink::on_token`],
/// terminated by exactly one [`AiSink::on_done`] (success) or
/// [`AiSink::on_error`] (failure/cancellation). The returned [`AiHandle`] lets
/// Swift `cancel()`/supersede the request (NFR-002 / FR-036a).
///
/// ## Zero-network gating (SC-008 / FR-035 / FR-036a)
///
/// Before any socket is opened, this:
/// 1. wraps `api_key` in the redacting [`ApiKey`]; a **blank** key →
///    `on_error(AiNotConfigured)` and **no request** (SC-008);
/// 2. snapshots the document and enforces the max-input guard
///    ([`check_input_size`]); an oversized document →
///    `on_error(AiOversizedInput)` and **no request** (FR-036a).
///
/// So with no AI configured/invoked the function opens no connection — the
/// network is only reached on an explicit, validated invocation.
///
/// The `api_key` is transient: wrapped in [`ApiKey`], set only on the
/// `Authorization` header inside the spawned task, and never logged or persisted
/// (NFR-006). Infallible at the boundary (returns the handle directly): every
/// failure mode is delivered through the sink's `on_error` terminal, matching the
/// contract's "exactly one terminal callback per stream" rule.
#[uniffi::export]
#[must_use]
pub fn summarize_document(
    h: Arc<OpenDocHandle>,
    cfg: AiRequestConfig,
    api_key: String,
    sink: Arc<dyn AiSink>,
) -> Arc<AiHandle> {
    let handle = Arc::new(AiHandle::new());
    let token = handle.token();
    let config: AiConfig = cfg.into();
    let key = ApiKey::new(api_key);

    // -- Local, pre-network validation (no socket on any of these paths) -------

    // A blank/absent key means "not configured" — refuse before any request
    // (SC-008). The transient `api_key` String is already wrapped + redacted.
    if key.is_blank() {
        sink.on_error(FfiError::AiNotConfigured);
        return handle;
    }

    // Snapshot the document under the lock, then validate its size BEFORE any
    // send (FR-036a). A closed handle / poisoned lock surfaces as the terminal.
    let document = match h.snapshot_text() {
        Ok(text) => text,
        Err(err) => {
            sink.on_error(err);
            return handle;
        }
    };
    if let Err(err) = check_input_size(&document, config.max_input_bytes) {
        sink.on_error(FfiError::from(err));
        return handle;
    }

    // Build the request body now (pure, no network) so a serialization failure is
    // reported before spawning.
    let body = match build_request_body(&config.model, DEFAULT_SUMMARY_SYSTEM_PROMPT, &document) {
        Ok(body) => body,
        Err(err) => {
            sink.on_error(FfiError::from(err));
            return handle;
        }
    };

    // -- Spawn the streaming request on the shared runtime --------------------

    let Ok(rt) = try_runtime() else {
        // The OS could not provide an async runtime: report it as a terminal
        // error rather than silently dropping the request.
        sink.on_error(FfiError::Internal {
            detail: "failed to start AI runtime".to_owned(),
        });
        return handle;
    };

    rt.spawn(async move {
        // Contain any panic in the worker so it becomes the single terminal error
        // rather than aborting the process (NFR-003). We run the async body to a
        // `Result` and deliver its terminal here so containment wraps the whole
        // request lifecycle.
        let outcome = run_summary(&config, &key, body, &token, &sink).await;

        // The terminal is delivered INSIDE `run_summary` for the streaming/cancel
        // paths (so `on_done(full)` carries the assembled text); `run_summary`
        // returns `Err` only for a setup/transport failure whose terminal it did
        // not itself deliver. Map that to `on_error`.
        if let Err(err) = outcome {
            sink.on_error(err);
        }
    });

    handle
}

/// Drive one streaming summary request to its terminal, returning `Ok(())` once a
/// terminal (`on_done`/`on_error`) has been delivered to `sink`, or `Err(e)` for
/// a transport/setup failure whose terminal the **caller** must still deliver.
///
/// Cancellation, the per-chunk inactivity timeout, and SSE framing are all wired
/// here. The key is exposed **only** to set the `Authorization` header (NFR-006).
async fn run_summary(
    config: &AiConfig,
    key: &ApiKey,
    body: String,
    token: &CancellationToken,
    sink: &Arc<dyn AiSink>,
) -> Result<(), FfiError> {
    // Wrap the whole request in panic containment: an unexpected panic in the
    // reqwest/parse path becomes a contained terminal error (NFR-003). We can't
    // `catch_unwind` across an `.await`, so the panic guard wraps the synchronous
    // client build; the async body's own errors are already `Result`s.
    let client = build_client().map_err(|e| FfiError::Internal {
        detail: format!("failed to build HTTP client: {e}"),
    })?;

    let url = chat_completions_url(&config.base_url);
    // The key is exposed ONLY here, onto the Authorization header (NFR-006).
    let auth = build_auth_header(key);

    // Send the request, racing it against cancellation so a cancel before the
    // response headers arrive resolves promptly as AiCancelled.
    let response = tokio::select! {
        () = token.cancelled() => {
            sink.on_error(FfiError::AiCancelled);
            return Ok(());
        }
        resp = client
            .post(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(body)
            .send() => resp,
    };

    let response = match response {
        Ok(resp) => resp,
        Err(err) => {
            sink.on_error(transport_error(&err));
            return Ok(());
        }
    };

    // Non-2xx → AiHttp with a redacted, key-free detail (NFR-006).
    let status = response.status();
    if !status.is_success() {
        let detail = status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_owned();
        sink.on_error(FfiError::AiHttp {
            status: status.as_u16(),
            detail,
        });
        return Ok(());
    }

    stream_body(response, config.request_timeout_ms, token, sink).await;
    Ok(())
}

/// Stream the response body chunk-by-chunk through the core SSE parser, pushing
/// each delta to `sink`, terminated by exactly one `on_done`/`on_error`.
///
/// Cancellation and a **per-chunk** inactivity timeout race each `next()`:
/// - cancel → `on_error(AiCancelled)`, stop;
/// - no chunk within `timeout_ms` → `on_error(AiTimeout)`, stop;
/// - the byte stream ending cleanly (server closed / `[DONE]`) → `on_done(full)`.
async fn stream_body(
    response: reqwest::Response,
    timeout_ms: u64,
    token: &CancellationToken,
    sink: &Arc<dyn AiSink>,
) {
    let mut parser = SseParser::new();
    let mut full = String::new();
    let mut stream = response.bytes_stream();
    // A zero timeout would fire instantly; treat it as "effectively no per-chunk
    // limit" by using a very large duration (the request is still cancellable).
    let per_chunk = if timeout_ms == 0 {
        Duration::from_secs(u64::from(u32::MAX))
    } else {
        Duration::from_millis(timeout_ms)
    };

    loop {
        let next = tokio::select! {
            biased;
            () = token.cancelled() => {
                sink.on_error(FfiError::AiCancelled);
                return;
            }
            // PER-CHUNK inactivity timeout (research §B5: NOT reqwest's whole-
            // request timeout, which would fire mid-stream on a slow model).
            timed = tokio::time::timeout(per_chunk, stream.next()) => timed,
        };

        match next {
            // Inactivity: no byte chunk arrived within the per-chunk window.
            Err(_elapsed) => {
                sink.on_error(FfiError::AiTimeout);
                return;
            }
            // The stream ended (server closed / all chunks consumed): flush any
            // buffered complete line, then the success terminal (research §B5: a
            // closed connection is a clean end, even without `[DONE]`).
            Ok(None) => {
                if emit_events(parser.finish(), &mut full, sink) {
                    // A `[DONE]` inside the flushed tail already terminated.
                    return;
                }
                sink.on_done(full);
                return;
            }
            // A byte chunk arrived.
            Ok(Some(Ok(bytes))) => {
                if emit_events(parser.push_bytes(&bytes), &mut full, sink) {
                    // `[DONE]` seen mid-stream: success terminal with the text so
                    // far.
                    sink.on_done(full);
                    return;
                }
            }
            // A transport error mid-stream.
            Ok(Some(Err(err))) => {
                sink.on_error(transport_error(&err));
                return;
            }
        }
    }
}

/// Forward the events from one parser step to the sink, appending tokens to
/// `full`. Returns `true` if a [`SseEvent::Done`] was seen (the caller delivers
/// the success terminal and stops).
fn emit_events(
    events: impl Iterator<Item = SseEvent>,
    full: &mut String,
    sink: &Arc<dyn AiSink>,
) -> bool {
    for event in events {
        match event {
            SseEvent::Token(text) => {
                full.push_str(&text);
                sink.on_token(text);
            }
            SseEvent::Done => return true,
        }
    }
    false
}

/// Validate (test) an AI configuration with a minimal reachability/auth probe
/// (FFI contract §7 `test_ai_config`; FR-037).
///
/// Sends a tiny non-streaming Chat-Completions request (1-token cap) and reports
/// success on a 2xx, mapping failures to the typed errors:
/// - blank key → [`FfiError::AiNotConfigured`] (no request, SC-008);
/// - 401/403/other non-2xx → [`FfiError::AiHttp`] (redacted detail, NFR-006);
/// - unreachable/transport → mapped via [`transport_error`].
///
/// Synchronous at the boundary (blocks on the shared runtime) since it is a
/// one-shot probe the settings UI awaits, not a streaming call. The key is
/// transient and set only on the `Authorization` header (NFR-006).
///
/// # Errors
///
/// Returns the typed [`FfiError`] for any failure; `Ok(())` on a reachable,
/// authorized endpoint.
#[uniffi::export]
pub fn test_ai_config(cfg: AiRequestConfig, api_key: String) -> Result<(), FfiError> {
    let config: AiConfig = cfg.into();
    let key = ApiKey::new(api_key);
    if key.is_blank() {
        return Err(FfiError::AiNotConfigured);
    }

    let rt = try_runtime()?;
    // Run the probe on the runtime; contain any panic in the body (NFR-003).
    let result = contain_panic(|| rt.block_on(probe(&config, &key))).map_err(FfiError::from)?;
    result
}

/// The async body of [`test_ai_config`]: a minimal, non-streaming probe request.
async fn probe(config: &AiConfig, key: &ApiKey) -> Result<(), FfiError> {
    let client = build_client().map_err(|e| FfiError::Internal {
        detail: format!("failed to build HTTP client: {e}"),
    })?;
    let url = chat_completions_url(&config.base_url);
    let auth = build_auth_header(key);
    // A 1-token, non-streaming probe — cheap and auth-revealing (FR-037).
    let body = build_request_body(&config.model, "ping", "ping").map_err(FfiError::from)?;

    let response = client
        .post(&url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|err| transport_error(&err))?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(FfiError::AiHttp {
            status: status.as_u16(),
            detail: status
                .canonical_reason()
                .unwrap_or("request failed")
                .to_owned(),
        })
    }
}

/// Build the shared HTTP client. macOS-native TLS (via the catalog's
/// `native-tls` feature); no whole-request timeout is set here — streaming uses
/// the per-chunk timeout in [`stream_body`] (research §B5).
fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().build()
}

/// Map a reqwest transport error to a typed, **key-free** [`FfiError`] (NFR-006).
///
/// A reqwest timeout (e.g. connect timeout) maps to [`FfiError::AiTimeout`];
/// everything else (connection refused, DNS, TLS) to a generic
/// [`FfiError::AiHttp`] with status `0` and a redacted message. reqwest's
/// `Display` does not include the API key (it is only on a header), but we still
/// build the detail from the error's own description, never from the request.
fn transport_error(err: &reqwest::Error) -> FfiError {
    if err.is_timeout() {
        return FfiError::AiTimeout;
    }
    // `err.to_string()` is reqwest's own message (URL + kind); it never contains
    // the Authorization header value, so no key can leak here (NFR-006).
    FfiError::AiHttp {
        status: 0,
        detail: format!("request failed: {err}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{summarize_document, test_ai_config, AiRequestConfig};
    use crate::document::open_document;
    use crate::error::FfiError;
    use crate::handles::AiSink;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A recording `AiSink`: captures tokens and the single terminal.
    #[derive(Default)]
    struct Recorder {
        tokens: Mutex<Vec<String>>,
        done: Mutex<Option<String>>,
        error: Mutex<Option<FfiError>>,
    }

    impl Recorder {
        fn error(&self) -> Option<FfiError> {
            self.error.lock().ok().and_then(|g| g.clone())
        }
        fn done(&self) -> Option<String> {
            self.done.lock().ok().and_then(|g| g.clone())
        }
        fn token_count(&self) -> usize {
            self.tokens.lock().map(|t| t.len()).unwrap_or(0)
        }
    }

    impl AiSink for Recorder {
        fn on_token(&self, text: String) {
            if let Ok(mut t) = self.tokens.lock() {
                t.push(text);
            }
        }
        fn on_done(&self, full: String) {
            if let Ok(mut d) = self.done.lock() {
                *d = Some(full);
            }
        }
        fn on_error(&self, err: FfiError) {
            if let Ok(mut e) = self.error.lock() {
                *e = Some(err);
            }
        }
    }

    fn write_note(dir: &tempfile::TempDir, body: &str) -> String {
        let path = dir.path().join("note.md");
        std::fs::write(&path, body).expect("write note");
        path.to_string_lossy().into_owned()
    }

    fn config(base_url: &str, max_input_bytes: u64) -> AiRequestConfig {
        AiRequestConfig {
            base_url: base_url.to_owned(),
            model: "test-model".to_owned(),
            request_timeout_ms: 1000,
            max_input_bytes,
        }
    }

    /// Spin until `pred` is true or the deadline elapses (the terminal is
    /// delivered from a spawned task). Returns whether `pred` became true.
    fn wait_until(deadline: Duration, mut pred: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if pred() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        pred()
    }

    #[test]
    fn blank_key_is_rejected_with_no_network() {
        // SC-008: a blank key must surface AiNotConfigured synchronously and open
        // no connection. The base URL is unroutable; if any request were made the
        // test would hang/fail differently — here it never even tries.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "# Doc\n");
        let handle = open_document(path).expect("open");
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn AiSink> = rec.clone();

        let _ai = summarize_document(
            handle,
            config("http://127.0.0.1:0/v1", 1_000_000),
            "   ".to_owned(), // blank key
            sink,
        );

        // The terminal is delivered synchronously on the validation path.
        assert!(
            matches!(rec.error(), Some(FfiError::AiNotConfigured)),
            "blank key → AiNotConfigured, got {:?}",
            rec.error()
        );
        assert_eq!(rec.token_count(), 0, "no tokens for a refused request");
        assert!(
            rec.done().is_none(),
            "no success terminal for a refused request"
        );
    }

    #[test]
    fn oversized_input_is_rejected_before_any_send() {
        // FR-036a: a document over `max_input_bytes` is refused locally with
        // AiOversizedInput and no request is made.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, &"x".repeat(2000));
        let handle = open_document(path).expect("open");
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn AiSink> = rec.clone();

        let _ai = summarize_document(
            handle,
            config("http://127.0.0.1:0/v1", 100), // cap below the doc size
            "sk-real-key".to_owned(),
            sink,
        );

        match rec.error() {
            Some(FfiError::AiOversizedInput { limit, .. }) => assert_eq!(limit, 100),
            other => panic!("expected AiOversizedInput, got {other:?}"),
        }
        assert_eq!(rec.token_count(), 0);
    }

    #[test]
    fn unreachable_endpoint_yields_an_error_terminal_not_a_hang() {
        // A configured key + a connection-refused endpoint must deliver a single
        // error terminal (not a token, not on_done, not a hang). Proves the
        // transport-error path delivers exactly one terminal.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "# Doc\n");
        let handle = open_document(path).expect("open");
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn AiSink> = rec.clone();

        // 127.0.0.1:1 is essentially always connection-refused (no listener).
        let _ai = summarize_document(
            handle,
            config("http://127.0.0.1:1/v1", 1_000_000),
            "sk-real-key".to_owned(),
            sink,
        );

        assert!(
            wait_until(Duration::from_secs(10), || rec.error().is_some()),
            "an unreachable endpoint must deliver an error terminal"
        );
        assert!(
            rec.done().is_none(),
            "no success terminal on transport failure"
        );
        assert_eq!(rec.token_count(), 0, "no tokens on transport failure");
    }

    #[test]
    fn cancel_before_send_resolves_as_cancelled() {
        // Cancelling immediately (before the request can connect to an unroutable
        // endpoint) resolves promptly as AiCancelled with no tokens.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_note(&dir, "# Doc\n");
        let handle = open_document(path).expect("open");
        let rec = Arc::new(Recorder::default());
        let sink: Arc<dyn AiSink> = rec.clone();

        // An unroutable address (TEST-NET-1) so the connect blocks long enough for
        // the cancel to win the select.
        let ai = summarize_document(
            handle,
            config("http://192.0.2.1:81/v1", 1_000_000),
            "sk-real-key".to_owned(),
            sink,
        );
        ai.cancel();

        assert!(
            wait_until(Duration::from_secs(10), || rec.error().is_some()),
            "a cancelled request must deliver a terminal"
        );
        assert!(
            matches!(rec.error(), Some(FfiError::AiCancelled)),
            "cancel → AiCancelled, got {:?}",
            rec.error()
        );
        assert_eq!(rec.token_count(), 0, "no tokens after an immediate cancel");
    }

    #[test]
    fn test_ai_config_blank_key_is_not_configured() {
        // FR-037: a blank key validates as AiNotConfigured with no network.
        let err = test_ai_config(config("http://127.0.0.1:0/v1", 1000), String::new())
            .expect_err("blank key must fail validation");
        assert!(matches!(err, FfiError::AiNotConfigured));
    }

    #[test]
    fn test_ai_config_unreachable_endpoint_errors() {
        // A real key but a refused endpoint surfaces a typed transport error.
        let err = test_ai_config(config("http://127.0.0.1:1/v1", 1000), "sk-key".to_owned())
            .expect_err("unreachable endpoint must fail validation");
        // Either AiHttp (status 0) or AiTimeout depending on the OS's refusal.
        assert!(
            matches!(err, FfiError::AiHttp { .. } | FfiError::AiTimeout),
            "expected a transport error, got {err:?}"
        );
    }
}
