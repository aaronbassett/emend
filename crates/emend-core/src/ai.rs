//! T112 (core half) — the **pure** AI client: SSE parsing, secret hygiene, the
//! max-input guard, and request *building as data* (US6 · FR-032/035/036/036a,
//! NFR-006; research §B5).
//!
//! ## The core/FFI split (binding — Constitution V)
//!
//! `emend-core` MUST stay **tokio-free AND reqwest-free** (CI proves
//! `cargo tree -p emend-core -i {tokio,reqwest,uniffi}` each find nothing). So
//! the AI client is split:
//!
//! * **This module (pure, no tokio, no reqwest):**
//!   - [`SseParser`] — the Server-Sent-Events line parser. It buffers raw
//!     response bytes across chunks, emits one [`SseEvent::Token`] per complete
//!     `data:` content delta, special-cases `data: [DONE]` → [`SseEvent::Done`],
//!     skips comment/heartbeat/blank lines, tolerates CRLF and LF, and treats a
//!     closed connection (no `[DONE]`) as a clean end via [`SseParser::finish`].
//!     A line split mid-byte (even mid-UTF-8-codepoint) across chunks never emits
//!     a partial token (FFI contract "Global rules": complete UTF-8 tokens).
//!   - [`check_input_size`] — the FR-036a max-input guard, rejecting oversized
//!     input with [`EmendError::AiOversizedInput`] **before any send** (the FFI
//!     layer calls it before constructing a request; network gating is structural
//!     since the core has no network).
//!   - [`ApiKey`] — a redacting newtype whose `Debug` AND `Display` render `***`,
//!     never the secret (NFR-006). The real bytes are reachable only through the
//!     explicit [`ApiKey::expose`].
//!   - [`AiConfig`] + [`build_request_body`] / [`build_auth_header`] — pure
//!     request *builders* (headers/body as data), with **no network**.
//!
//! * **`emend-ffi` (has tokio + reqwest):** the streaming orchestration —
//!   `reqwest::bytes_stream()`, `CancellationToken` + `tokio::select!`, a
//!   per-chunk inactivity `tokio::time::timeout` — feeds bytes through
//!   [`SseParser`] and pushes deltas to the foreign `AiSink`.

use crate::EmendError;
use serde::Deserialize;

/// The conventional summary instruction sent as the system message when none is
/// supplied. Kept here (core) so the prompt is testable and consistent.
pub const DEFAULT_SUMMARY_SYSTEM_PROMPT: &str =
    "You are a concise assistant. Summarize the following Markdown document in a \
     short paragraph. Output only the summary, no preamble.";

// =====================================================================
// Secret hygiene — the redacting API-key newtype (NFR-006)
// =====================================================================

/// A redacting wrapper around the AI API key (NFR-006).
///
/// Its [`Debug`] **and** [`Display`] render `***` — never the secret — so the key
/// cannot accidentally land in a log line, a panic message, a tracing field, or
/// any `format!`'d string. The real bytes are reachable **only** through the
/// explicit [`expose`](Self::expose) accessor, which the transport layer calls to
/// set the `Authorization` header and nowhere else.
///
/// The newtype is intentionally **not** `Clone`-derived-with-Debug-leaking and
/// carries no other trait that could surface the secret; it holds the key for the
/// minimal lifetime of one request (the FFI boundary passes the key in as a
/// transient `String`, wraps it here, uses it once, and drops it).
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wrap a raw key string, **trimming** surrounding whitespace once at this
    /// choke point. A pasted key often carries a trailing newline or stray spaces
    /// (e.g. `"sk-abc\n"`); trimming here guarantees the bytes that later reach
    /// [`build_auth_header`] form a clean `Bearer <key>` value rather than a
    /// header corrupted by an embedded `\n`. The value is never copied into any
    /// logging surface; it is only readable via [`expose`](Self::expose).
    #[must_use]
    pub fn new(key: String) -> Self {
        Self(key.trim().to_owned())
    }

    /// The real secret bytes (already trimmed), for setting the `Authorization`
    /// header. This is the **only** way to read the key — named explicitly so an
    /// accidental `{}`/`{:?}` can never surface it (NFR-006).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the key is blank — effectively "no key", so the FFI layer can map
    /// it to [`EmendError::AiNotConfigured`] rather than sending an empty bearer
    /// token. Since [`new`](Self::new) trims, an all-whitespace input is already
    /// stored as `""`, so a plain emptiness check suffices.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for ApiKey {
    /// Always redacted — never prints the secret (NFR-006).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

impl std::fmt::Display for ApiKey {
    /// Always redacted — never prints the secret (NFR-006).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

// =====================================================================
// Max-input guard (FR-036a) — rejected locally, before any send
// =====================================================================

/// Reject an over-limit AI input **before any network call** (FR-036a).
///
/// `input` is the document text about to be sent; `max_bytes` is the configured
/// cap. Returns [`EmendError::AiOversizedInput`] (carrying the actual byte size
/// and the limit, for UI messaging) if `input`'s UTF-8 byte length **exceeds**
/// the cap. The boundary is inclusive: an input *exactly* at the limit is
/// accepted.
///
/// The check measures **bytes** (what crosses the wire), not chars: a short
/// multibyte string can still be oversized.
///
/// # Errors
///
/// [`EmendError::AiOversizedInput`] if `input.len() > max_bytes`.
pub fn check_input_size(input: &str, max_bytes: u64) -> Result<(), EmendError> {
    let bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
    if bytes > max_bytes {
        return Err(EmendError::AiOversizedInput {
            bytes,
            limit: max_bytes,
        });
    }
    Ok(())
}

// =====================================================================
// SSE parsing — the pure, lenient event parser (research §B5)
// =====================================================================

/// One event decoded from the AI SSE stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// A content delta — a **complete** UTF-8 string (the parser buffers partial
    /// bytes across chunks so a split code point is never emitted).
    Token(String),
    /// The terminal `data: [DONE]` sentinel: the stream is finished.
    Done,
}

/// A streaming Server-Sent-Events parser for OpenAI-compatible Chat-Completions
/// responses (research §B5).
///
/// Feed raw response byte chunks via [`push_bytes`](Self::push_bytes) (returns an
/// iterator of the [`SseEvent`]s completed by that chunk), and call
/// [`finish`](Self::finish) once the byte stream ends (a closed connection is a
/// clean end). The parser is **pure** — no tokio, no reqwest — so all the lenient
/// framing is unit-testable.
///
/// ## Framing handled
///
/// * a `data:` payload **split across chunks** reassembles into one delta (the
///   partial line is buffered until its newline arrives);
/// * `data: [DONE]` → [`SseEvent::Done`];
/// * comment/heartbeat lines (`:`-prefixed) and blank lines are ignored;
/// * CRLF and LF line endings (a trailing `\r` is stripped);
/// * a server that just closes (no `[DONE]`) — [`finish`](Self::finish) flushes a
///   buffered complete-but-unterminated line as a final token;
/// * a malformed (non-JSON) `data:` payload is skipped, not fatal — later valid
///   lines still parse (BYOM endpoints diverge).
#[derive(Debug, Default)]
pub struct SseParser {
    /// Bytes received but not yet terminated by a newline. A `data:` line split
    /// across chunks accumulates here until its `\n` arrives.
    buf: Vec<u8>,
}

impl SseParser {
    /// A fresh parser with an empty line buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a raw response byte `chunk`, returning the [`SseEvent`]s it completed.
    ///
    /// Only **newline-terminated** lines are decoded; a trailing partial line is
    /// retained in the internal buffer for the next chunk (or [`finish`]). So a
    /// `data:` payload split mid-byte — even mid-UTF-8-codepoint — never yields a
    /// partial token: the line is only decoded once whole.
    pub fn push_bytes(&mut self, chunk: &[u8]) -> std::vec::IntoIter<SseEvent> {
        self.buf.extend_from_slice(chunk);

        let mut events = Vec::new();
        // Drain every complete line (terminated by `\n`) from the buffer.
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            // `drain(..=nl)` removes the line + its `\n` from the front.
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            // Strip the trailing `\n` and any preceding `\r` (CRLF tolerance).
            let end = line.len().saturating_sub(1); // drop the `\n`
            let line = &line[..end];
            let line = strip_trailing_cr(line);
            if let Some(event) = decode_line(line) {
                events.push(event);
            }
        }
        events.into_iter()
    }

    /// Signal end-of-stream (the connection closed). Flushes any buffered
    /// complete-but-unterminated line as a final token — a server that closes the
    /// connection mid-line, or without a `[DONE]`, is a clean end (research §B5).
    ///
    /// Returns the events the flush produced (zero or one token). Safe to call
    /// even when nothing is buffered.
    pub fn finish(&mut self) -> std::vec::IntoIter<SseEvent> {
        let mut events = Vec::new();
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            let line = strip_trailing_cr(&line);
            if let Some(event) = decode_line(line) {
                events.push(event);
            }
        }
        events.into_iter()
    }
}

/// Strip a single trailing `\r` (CRLF → LF tolerance) from a line slice.
fn strip_trailing_cr(line: &[u8]) -> &[u8] {
    match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    }
}

/// Decode one SSE line (no trailing newline/`\r`) into an [`SseEvent`], or `None`
/// for a line that carries no event (comment, blank, content-less delta, or a
/// malformed payload that is skipped leniently).
fn decode_line(line: &[u8]) -> Option<SseEvent> {
    // A line that is not valid UTF-8 is skipped (the parser never emits partial
    // code points; a whole non-UTF-8 line is a divergent server and is ignored).
    let line = std::str::from_utf8(line).ok()?;
    let trimmed = line.trim();

    // Blank separator line → no event.
    if trimmed.is_empty() {
        return None;
    }
    // Comment / heartbeat line (`:`-prefixed) → no event.
    if trimmed.starts_with(':') {
        return None;
    }
    // Only `data:` lines carry content for Chat-Completions; other SSE fields
    // (`event:`, `id:`, `retry:`) are not used by the OpenAI stream → skipped.
    let payload = trimmed.strip_prefix("data:")?.trim();

    // The terminal sentinel.
    if payload == "[DONE]" {
        return Some(SseEvent::Done);
    }

    // Decode the Chat-Completions chunk and pull out the incremental content. A
    // malformed/non-JSON payload, or a content-less chunk (role-priming or
    // finish-reason-only), yields no token (lenient — research §B5).
    match serde_json::from_str::<StreamChunk>(payload) {
        Ok(chunk) => chunk.first_content().map(SseEvent::Token),
        Err(_) => None,
    }
}

/// The subset of an OpenAI Chat-Completions **streaming** chunk this parser
/// reads. Extra fields on the wire are ignored (serde drops unknown keys), so a
/// divergent BYOM server's extra metadata does not break decoding.
#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    /// The incremental content for this chunk; absent on the role-priming and
    /// finish-reason-only chunks.
    content: Option<String>,
}

impl StreamChunk {
    /// The first choice's content delta, if any and non-empty.
    fn first_content(self) -> Option<String> {
        let content = self.choices.into_iter().next()?.delta.content?;
        if content.is_empty() {
            None
        } else {
            Some(content)
        }
    }
}

// =====================================================================
// Request building (as data) — pure, NO network (research §B5)
// =====================================================================

/// The user-supplied BYOM connection settings for one AI request (FFI contract
/// §7 `AiRequestConfig`). Pure data; the `emend-ffi` transport turns it into a
/// `reqwest` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiConfig {
    /// OpenAI-compatible base URL (e.g. `https://api.openai.com/v1`).
    pub base_url: String,
    /// Model identifier (e.g. `gpt-4o-mini`, or a local model id).
    pub model: String,
    /// Per-request timeout in milliseconds (the FFI transport applies it as a
    /// per-chunk inactivity guard, research §B5).
    pub request_timeout_ms: u64,
    /// Maximum input size in bytes, enforced locally by [`check_input_size`]
    /// before any send (FR-036a).
    pub max_input_bytes: u64,
}

/// Build the Chat-Completions endpoint URL from a base URL (joining
/// `/chat/completions`, tolerating a trailing slash on the base).
#[must_use]
pub fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/chat/completions")
}

/// Build the `Authorization` header **value** for a request (`"Bearer <key>"`).
///
/// Takes the [`ApiKey`] and exposes it **only** here, at the single point where
/// the secret must be on the header — it is never logged or stored (NFR-006). The
/// returned `String` is handed straight to the HTTP client's header map by the
/// FFI transport and dropped.
#[must_use]
pub fn build_auth_header(key: &ApiKey) -> String {
    format!("Bearer {}", key.expose())
}

/// Build the JSON request **body** for a streaming summary request, as a
/// serialized string (pure data; no network).
///
/// Produces an OpenAI Chat-Completions body with `stream: true` and **no**
/// `max_tokens` cap (summaries run to completion), a system message carrying
/// `system_prompt`, and a user message carrying `document`. The caller (FFI
/// transport) sets it as the request body.
///
/// For the non-streaming, token-capped connection probe, use
/// [`build_probe_body`] instead; both delegate to [`build_body`].
///
/// # Errors
///
/// [`EmendError::Internal`] if serialization fails (it does not for these plain
/// types, but the boundary stays no-panic — NFR-003).
pub fn build_request_body(
    model: &str,
    system_prompt: &str,
    document: &str,
) -> Result<String, EmendError> {
    build_body(model, system_prompt, document, true, None)
}

/// Build the JSON request **body** for the **non-streaming** connection probe
/// used by `test_ai_config` (FR-037), as a serialized string (pure data; no
/// network).
///
/// Produces an OpenAI Chat-Completions body with `stream: false` and
/// `max_tokens: 1`: a minimal reachability/auth check that the settings UI awaits
/// and never streams. Capping at one token keeps "Test Connection" from opening a
/// full generation it would only throw away.
///
/// # Errors
///
/// [`EmendError::Internal`] if serialization fails (it does not for these plain
/// types, but the boundary stays no-panic — NFR-003).
pub fn build_probe_body(
    model: &str,
    system_prompt: &str,
    document: &str,
) -> Result<String, EmendError> {
    build_body(model, system_prompt, document, false, Some(1))
}

/// Shared body builder behind [`build_request_body`] and [`build_probe_body`].
///
/// `stream` toggles the OpenAI `stream` flag; `max_tokens` adds an optional
/// upper bound on the completion length (omitted from the JSON when `None`, so an
/// uncapped summary sends no `max_tokens` at all).
fn build_body(
    model: &str,
    system_prompt: &str,
    document: &str,
    stream: bool,
    max_tokens: Option<u32>,
) -> Result<String, EmendError> {
    let body = ChatRequest {
        model,
        stream,
        max_tokens,
        messages: vec![
            ChatMessage {
                role: "system",
                content: system_prompt,
            },
            ChatMessage {
                role: "user",
                content: document,
            },
        ],
    };
    serde_json::to_string(&body).map_err(|e| EmendError::Internal {
        detail: format!("failed to serialize AI request body: {e}"),
    })
}

/// The Chat-Completions request body (serialize-only). Borrows its strings so the
/// builder allocates only the final JSON.
#[derive(Debug, serde::Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    stream: bool,
    /// Optional completion-length cap. `None` omits the field entirely (uncapped
    /// summaries); `Some(1)` is the probe's minimal-cost reachability check.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    messages: Vec<ChatMessage<'a>>,
}

#[derive(Debug, serde::Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[cfg(test)]
mod tests {
    // Unit tests assert on their own fixtures; the workspace denies these in
    // library code, so scope the allowance to this test module.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{
        build_auth_header, build_probe_body, build_request_body, chat_completions_url,
        check_input_size, ApiKey, SseEvent, SseParser,
    };
    use crate::EmendError;

    fn tokens(events: &[SseEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                SseEvent::Token(t) => Some(t.clone()),
                SseEvent::Done => None,
            })
            .collect()
    }

    #[test]
    fn api_key_redacts_in_debug_and_display() {
        let key = ApiKey::new("sk-secret-123".to_owned());
        assert!(!format!("{key:?}").contains("secret"));
        assert!(!format!("{key}").contains("secret"));
        assert_eq!(format!("{key}"), "***");
        assert_eq!(key.expose(), "sk-secret-123");
    }

    #[test]
    fn api_key_trims_surrounding_whitespace_before_exposure() {
        // M2: a pasted key with a trailing newline / surrounding spaces must be
        // trimmed once at the boundary so the Authorization header is clean. We
        // assert via `build_auth_header` (the only sanctioned read path), never by
        // printing the key — Display/Debug stay redacted.
        let key = ApiKey::new("  sk-abc\n".to_owned());
        assert_eq!(build_auth_header(&key), "Bearer sk-abc");
        // Redaction is unaffected by trimming.
        assert_eq!(format!("{key}"), "***");
        assert!(!format!("{key:?}").contains("sk-abc"));

        // An all-whitespace key trims to empty → blank (→ AiNotConfigured upstream).
        assert!(ApiKey::new("  \n\t ".to_owned()).is_blank());
        assert!(!ApiKey::new("sk-x".to_owned()).is_blank());
    }

    #[test]
    fn check_input_size_boundary_is_inclusive() {
        assert!(check_input_size("aaaa", 4).is_ok());
        assert!(matches!(
            check_input_size("aaaaa", 4),
            Err(EmendError::AiOversizedInput { bytes: 5, limit: 4 })
        ));
    }

    #[test]
    fn sse_parses_split_data_line() {
        let line = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n";
        let bytes = line.as_bytes();
        let (a, b) = bytes.split_at(20);
        let mut p = SseParser::new();
        let mut got: Vec<SseEvent> = p.push_bytes(a).collect();
        got.extend(p.push_bytes(b));
        assert_eq!(tokens(&got), vec!["hi"]);
    }

    #[test]
    fn sse_done_is_terminal() {
        let mut p = SseParser::new();
        let events: Vec<SseEvent> = p.push_bytes(b"data: [DONE]\n").collect();
        assert_eq!(events, vec![SseEvent::Done]);
    }

    #[test]
    fn auth_header_and_body_build_as_data() {
        let key = ApiKey::new("k".to_owned());
        assert_eq!(build_auth_header(&key), "Bearer k");
        let body = build_request_body("m", "sys", "doc").expect("body");
        assert!(body.contains("\"model\":\"m\""));
        assert!(body.contains("\"stream\":true"));
        assert!(body.contains("doc"));
    }

    #[test]
    fn summary_body_streams_with_no_token_cap() {
        // H1: the streaming summary path must keep `stream: true` and send NO
        // `max_tokens` (summaries run to completion).
        let body = build_request_body("m", "sys", "doc").expect("body");
        assert!(body.contains("\"stream\":true"));
        assert!(
            !body.contains("max_tokens"),
            "summary body must not cap tokens, got {body}"
        );
    }

    #[test]
    fn probe_body_is_non_streaming_and_token_capped() {
        // H1: the connection probe must be `stream: false` with `max_tokens: 1`
        // (a minimal reachability/auth check, never a full generation).
        let body = build_probe_body("m", "ping", "ping").expect("body");
        assert!(
            body.contains("\"stream\":false"),
            "probe body must be non-streaming, got {body}"
        );
        assert!(
            body.contains("\"max_tokens\":1"),
            "probe body must cap at 1 token, got {body}"
        );
        assert!(body.contains("\"model\":\"m\""));
    }

    #[test]
    fn url_join_tolerates_trailing_slash() {
        assert_eq!(
            chat_completions_url("https://x/v1/"),
            "https://x/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://x/v1"),
            "https://x/v1/chat/completions"
        );
    }
}
