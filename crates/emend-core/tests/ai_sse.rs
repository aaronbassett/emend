//! T109 — the **pure** SSE event parser for the BYOM AI client (US6 · FR-032/036;
//! research §B5).
//!
//! This exercises [`emend_core::ai::SseParser`], the tokio-free / reqwest-free
//! line-buffering SSE parser that the `emend-ffi` streaming orchestration feeds
//! raw response byte chunks into. The parser owns ALL the lenient framing the
//! research note calls out, so it can be tested with plain `cargo test`:
//!
//! * a `data:` payload **split across two byte chunks** must reassemble into one
//!   ordered delta (FFI contract "Global rules": tokens are complete strings);
//! * `data: [DONE]` terminates the stream;
//! * comment/heartbeat (`:`-prefixed) and blank lines are ignored;
//! * CRLF and LF line endings are both tolerated;
//! * a server that just closes (no `[DONE]`) is a clean end (any buffered
//!   complete line is flushed).
//!
//! The parser maps each OpenAI Chat-Completions SSE `data:` JSON object to the
//! incremental `choices[0].delta.content` string (an empty/absent content line —
//! e.g. the role-priming first chunk or a finish-reason-only chunk — yields no
//! token).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::ai::{SseEvent, SseParser};

/// One OpenAI-style `data:` line carrying a content delta.
fn delta_line(content: &str) -> String {
    // A minimal Chat-Completions streaming chunk: only the fields the parser
    // reads (`choices[0].delta.content`).
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n",
        serde_json_string(content)
    )
}

/// Tiny JSON string encoder for the fixtures (avoids pulling serde into the test
/// just to build a literal; the parser does the real JSON decode).
fn serde_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Drain every event the parser yields for a chunk into a flat Vec.
fn feed(parser: &mut SseParser, chunk: &[u8]) -> Vec<SseEvent> {
    parser.push_bytes(chunk).collect()
}

/// Collect just the token strings from a list of events (dropping Done).
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
fn parses_single_complete_data_line() {
    let mut p = SseParser::new();
    let events = feed(&mut p, delta_line("Hello").as_bytes());
    assert_eq!(tokens(&events), vec!["Hello"], "one delta → one token");
}

#[test]
fn data_line_split_across_two_chunks_reassembles_in_order() {
    // The single `data:` line is sliced mid-payload into two byte chunks. The
    // parser must buffer the partial line and only emit once the newline arrives,
    // yielding the deltas in order across multiple complete lines.
    let stream = format!("{}{}", delta_line("Hello, "), delta_line("world"));
    let bytes = stream.as_bytes();
    // Split somewhere inside the FIRST data line's JSON payload.
    let split = 15.min(bytes.len() - 1);
    let (a, b) = bytes.split_at(split);

    let mut p = SseParser::new();
    let mut got = feed(&mut p, a);
    got.extend(feed(&mut p, b));

    assert_eq!(
        tokens(&got),
        vec!["Hello, ", "world"],
        "a split data line reassembles into ordered deltas: {got:?}"
    );
}

#[test]
fn split_inside_a_multibyte_utf8_sequence_never_emits_a_partial_codepoint() {
    // A 4-byte emoji split mid-sequence across chunks must not surface a token
    // until the whole line (and thus the whole code point) has arrived.
    let line = delta_line("ab😀cd"); // 😀 = 4 UTF-8 bytes
    let bytes = line.as_bytes();
    // Find the emoji's first byte and split one byte into it.
    let emoji_start = line.find('😀').expect("emoji present");
    let split = emoji_start + 1;
    let (a, b) = bytes.split_at(split);

    let mut p = SseParser::new();
    let first = feed(&mut p, a);
    assert!(
        tokens(&first).is_empty(),
        "a line split mid-codepoint must not emit yet: {first:?}"
    );
    let second = feed(&mut p, b);
    assert_eq!(
        tokens(&second),
        vec!["ab😀cd"],
        "the completed line decodes the whole emoji: {second:?}"
    );
}

#[test]
fn done_sentinel_terminates_the_stream() {
    let stream = format!("{}data: [DONE]\n", delta_line("partial"));
    let mut p = SseParser::new();
    let events = feed(&mut p, stream.as_bytes());
    assert_eq!(tokens(&events), vec!["partial"]);
    assert!(
        events.iter().any(|e| matches!(e, SseEvent::Done)),
        "[DONE] yields a Done terminal: {events:?}"
    );
    // The Done is the LAST event.
    assert!(
        matches!(events.last(), Some(SseEvent::Done)),
        "Done must be terminal: {events:?}"
    );
}

#[test]
fn comment_and_blank_lines_are_ignored() {
    // `:`-prefixed comment/heartbeat lines and blank separator lines carry no
    // delta and must be skipped without error.
    let stream = format!(
        ": this is a heartbeat comment\n\n{}\n: another\n{}",
        delta_line("one").trim_end(),
        delta_line("two")
    );
    let mut p = SseParser::new();
    let events = feed(&mut p, stream.as_bytes());
    assert_eq!(
        tokens(&events),
        vec!["one", "two"],
        "comments/blank lines are skipped: {events:?}"
    );
}

#[test]
fn crlf_line_endings_are_tolerated() {
    // Some servers terminate SSE lines with CRLF; the trailing `\r` must be
    // stripped so the JSON parses and `[DONE]` still matches.
    let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"crlf\"}}]}\r\ndata: [DONE]\r\n";
    let mut p = SseParser::new();
    let events = feed(&mut p, stream.as_bytes());
    assert_eq!(tokens(&events), vec!["crlf"], "CRLF tolerated: {events:?}");
    assert!(
        events.iter().any(|e| matches!(e, SseEvent::Done)),
        "CRLF [DONE] still terminates: {events:?}"
    );
}

#[test]
fn content_less_chunks_yield_no_token() {
    // The role-priming first chunk (delta has `role` but no `content`) and a
    // finish-reason-only chunk produce no visible token.
    let role = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n";
    let finish = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n";
    let stream = format!("{role}{}{finish}", delta_line("body"));
    let mut p = SseParser::new();
    let events = feed(&mut p, stream.as_bytes());
    assert_eq!(
        tokens(&events),
        vec!["body"],
        "only the content-bearing chunk yields a token: {events:?}"
    );
}

#[test]
fn server_that_just_closes_flushes_a_trailing_complete_line() {
    // No `[DONE]`: the server sends a complete data line then closes. The
    // explicit `finish()` (called by the FFI driver when the byte stream ends)
    // surfaces any buffered complete line and reports a clean end.
    let mut p = SseParser::new();
    let mut events = feed(&mut p, delta_line("final").as_bytes());
    // A clean close with nothing buffered yields no further token (the line was
    // already terminated by its newline); `finish` must not error.
    events.extend(p.finish());
    assert_eq!(
        tokens(&events),
        vec!["final"],
        "clean close = done: {events:?}"
    );
}

#[test]
fn finish_flushes_a_buffered_line_with_no_trailing_newline() {
    // A server that closes the connection MID-line — the last `data:` line has no
    // terminating newline. `finish()` must flush that buffered line as a final
    // token (a closed connection is a clean end, research §B5).
    let mut p = SseParser::new();
    // Note: no trailing newline on this last line.
    let no_newline = delta_line("tail");
    let no_newline = no_newline.trim_end_matches('\n');
    let during = feed(&mut p, no_newline.as_bytes());
    assert!(
        tokens(&during).is_empty(),
        "an unterminated line is buffered, not emitted mid-stream: {during:?}"
    );
    let flushed: Vec<SseEvent> = p.finish().collect();
    assert_eq!(
        tokens(&flushed),
        vec!["tail"],
        "finish() flushes the buffered unterminated line: {flushed:?}"
    );
}

#[test]
fn malformed_json_data_line_is_skipped_not_fatal() {
    // A non-JSON `data:` payload from a divergent BYOM server must not abort the
    // whole stream — skip it and keep parsing later valid lines (lenient parsing,
    // research §B5).
    let stream = format!("data: not json at all\n{}", delta_line("recovered"));
    let mut p = SseParser::new();
    let events = feed(&mut p, stream.as_bytes());
    assert_eq!(
        tokens(&events),
        vec!["recovered"],
        "a malformed data line is skipped, later lines still parse: {events:?}"
    );
}
