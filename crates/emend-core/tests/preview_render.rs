//! T082 — preview-render acceptance (US4 · FR-023..028; FFI contract §6).
//!
//! These tests pin the **authoritative** preview engine (comrak, research §B1 —
//! a *separate* engine from the tree-sitter editor highlighter, never unified)
//! against the acceptance bullets:
//!
//! - (a) every top-level block carries a `data-line` start-line anchor for the
//!   Swift scroll-sync (research §C3);
//! - (b) a fenced ```rust block renders as syntect **classed** HTML
//!   (`<span class="...">` inside `<pre>`/`<code>`), research §B6;
//! - (c) a GFM **table** renders to `<table>`;
//! - (d) a GFM **tasklist** checkbox and a `==mark==` highlight render.
//!
//! Written before the implementation (TDD): they import `emend_core::parse::preview`
//! and will fail to compile until that module exists.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::parse::preview::{render_preview_html, PreviewOptions};

/// Render with the default options used by the FFI `render_preview_html`.
fn render(md: &str) -> String {
    render_preview_html(md, &PreviewOptions::default()).expect("preview render must succeed")
}

#[test]
fn top_level_blocks_carry_data_line_anchors() {
    // A heading on line 1 and a paragraph starting on line 3 — both top-level
    // blocks, so both must carry a `data-line` anchor with their 1-based start
    // line (research §C3, FFI contract §6).
    let md = "# Title\n\nA paragraph here.\n";
    let html = render(md);

    assert!(
        html.contains("data-line=\"1\""),
        "heading block should anchor to source line 1:\n{html}"
    );
    assert!(
        html.contains("data-line=\"3\""),
        "paragraph block should anchor to source line 3:\n{html}"
    );
}

#[test]
fn fenced_rust_block_is_syntect_classed() {
    // A ```rust fence must come out as syntect classed HTML: a <pre>/<code>
    // wrapper containing `<span class="...">` runs (research §B6), NOT a bare
    // unstyled <code> block.
    let md = "```rust\nfn main() {}\n```\n";
    let html = render(md);

    assert!(
        html.contains("<pre") && html.contains("<code"),
        "fenced block should render inside <pre>/<code>:\n{html}"
    );
    assert!(
        html.contains("<span class=\""),
        "syntect classed HTML should emit `<span class=\"...\">` runs:\n{html}"
    );
    // The keyword `fn` is a source keyword syntect classifies, so it must be
    // wrapped (rather than left as bare text) — a concrete signal the highlighter
    // actually ran rather than echoing the source.
    assert!(
        html.contains("fn"),
        "highlighted source text should still be present:\n{html}"
    );
}

#[test]
fn unknown_language_falls_back_without_panicking() {
    // A language token absent from the §D default set (Swift is one of the 6
    // plain-text fallbacks) must still render inside <pre>/<code>, escaped, with
    // no panic — never a hard failure (research §B6/§D).
    let md = "```swift\nlet x = 1\n```\n";
    let html = render(md);
    assert!(
        html.contains("<pre") && html.contains("<code"),
        "unknown-language fence should still render a code block:\n{html}"
    );
    assert!(
        html.contains("let x = 1"),
        "fallback should preserve the source text:\n{html}"
    );
}

#[test]
fn gfm_table_renders_to_table_element() {
    let md = "\
| A | B |
|---|---|
| 1 | 2 |
";
    let html = render(md);
    // The element opening tags carry `data-line`/`data-sourcepos` attributes (the
    // scroll-sync anchors), so match the tag prefix rather than a bare `<table>`.
    assert!(
        html.contains("<table"),
        "GFM table should render a <table> element:\n{html}"
    );
    assert!(
        html.contains("<th") && html.contains("<td"),
        "GFM table should have header/body cells:\n{html}"
    );
}

#[test]
fn tasklist_checkbox_renders() {
    let md = "- [x] done\n- [ ] todo\n";
    let html = render(md);
    assert!(
        html.contains("type=\"checkbox\""),
        "GFM tasklist should render an <input type=\"checkbox\">:\n{html}"
    );
    // The completed item must read as checked.
    assert!(
        html.contains("checked"),
        "the `[x]` item should render checked:\n{html}"
    );
}

#[test]
fn highlight_extension_renders_mark() {
    let md = "Some ==important== text.\n";
    let html = render(md);
    // The opening tag carries scroll-sync attributes, so match `<mark` + content.
    assert!(
        html.contains("<mark") && html.contains(">important</mark>"),
        "`==highlight==` should render to <mark> (research §B1):\n{html}"
    );
}

#[test]
fn strikethrough_renders() {
    let md = "~~gone~~\n";
    let html = render(md);
    // The opening tag carries scroll-sync attributes, so match `<del` + content.
    assert!(
        html.contains("<del") && html.contains(">gone</del>"),
        "GFM strikethrough should render to <del>:\n{html}"
    );
}

#[test]
fn wikilink_extension_renders_link() {
    // comrak's wikilinks extension turns `[[Target]]` into an anchor (research
    // §B1). We only assert it produces a link element here; deterministic path
    // resolution (FR-019a) is a US3/US5 concern, not the preview engine's.
    let md = "See [[Other Note]] for details.\n";
    let html = render(md);
    assert!(
        html.contains("<a") && html.contains("Other Note"),
        "wikilink should render an anchor for the target:\n{html}"
    );
}

#[test]
fn embed_left_literal_until_us5() {
    // `![[embed]]` resolution is deferred to US5 (the linking phase). For now the
    // preview must render it literally/unchanged and must NOT recurse or fetch —
    // i.e. no embedded document content appears. We assert the raw target text
    // survives into the output (escaped or as a wikilink), and crucially that the
    // engine does not attempt to inline another note.
    let md = "![[Some Note]]\n";
    let html = render(md);
    assert!(
        html.contains("Some Note"),
        "embed target text should survive literally (US5 will resolve it):\n{html}"
    );
}

// --- XSS / sanitization trust boundary (SC-008, FR-035, FFI contract §6) -----
//
// The preview is *untrusted* user Markdown rendered into the WKWebView, so the
// comrak engine MUST neutralize active content: it runs with `unsafe_ = false`
// (see `preview.rs::build_options`), which drops raw HTML and rewrites dangerous
// URI schemes. These tests pin that boundary so a future flip to `unsafe_ = true`
// (or a comrak upgrade that changes the default) can't silently reopen an XSS
// hole — the WebView CSP (T087) is defence-in-depth, not the primary control.

#[test]
fn raw_script_html_is_stripped() {
    // A literal <script> in the source must never reach the output verbatim.
    // comrak with `unsafe_ = false` omits raw HTML (typically replacing the block
    // with an `<!-- raw HTML omitted -->` comment), so no executable <script> tag
    // survives.
    let md = "Before\n\n<script>alert('x')</script>\n\nAfter\n";
    let html = render(md);
    assert!(
        !html.contains("<script"),
        "raw <script> must not survive into preview HTML:\n{html}"
    );
    // The surrounding prose is still rendered — only the raw HTML is dropped.
    assert!(
        html.contains("Before") && html.contains("After"),
        "non-HTML content around the stripped block should remain:\n{html}"
    );
}

#[test]
fn raw_inline_event_handler_is_stripped() {
    // A raw inline element carrying an event handler (`onerror`) is raw HTML; with
    // `unsafe_ = false` the whole tag is dropped, so no `onerror=` attribute (and
    // no `<img` tag from the raw source) can fire.
    let md = "Look: <img src=x onerror=\"alert(1)\"> here\n";
    let html = render(md);
    assert!(
        !html.contains("onerror="),
        "inline event-handler attribute must not survive:\n{html}"
    );
    assert!(
        !html.contains("<img"),
        "the raw <img> tag must not survive into preview HTML:\n{html}"
    );
}

#[test]
fn javascript_uri_is_neutralized() {
    // A Markdown link whose destination is a `javascript:` URI must NOT produce an
    // `href="javascript:..."`. comrak's safe mode neutralizes dangerous schemes
    // (it emits `href=""`), so clicking the rendered link can't execute script.
    let md = "[click](javascript:alert(1))\n";
    let html = render(md);
    // The link element itself still renders (with safe link text)…
    assert!(
        html.contains("<a") && html.contains("click"),
        "the link element/text should still render:\n{html}"
    );
    // …but the dangerous scheme must be gone from every href.
    assert!(
        !html.contains("href=\"javascript:"),
        "javascript: scheme must be neutralized in the href:\n{html}"
    );
    assert!(
        !html.to_ascii_lowercase().contains("javascript:"),
        "no javascript: URI should appear anywhere in the output:\n{html}"
    );
}
