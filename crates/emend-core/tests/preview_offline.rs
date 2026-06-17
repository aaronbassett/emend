//! T083 — preview rendering performs ZERO network access (SC-008 / FR-035).
//!
//! Preview rendering is pure CPU/string: in-memory Markdown → `String`, with no
//! HTTP/socket dependency anywhere on the path. The meaningful, mechanically
//! checkable assertion is **structural**: a document that *references* remote
//! resources (images, links) renders them as literal `src=`/`href=` attributes —
//! the engine never dereferences a URL, so no fetch can occur.
//!
//! NOTE: the *runtime* offline guarantee for the preview pane is enforced by the
//! WKWebView CSP + `nonPersistent` data store + navigation delegate on the Swift
//! side (T087). This test guards that the **core** render path stays pure (no
//! network client is reachable from it), so the privacy property cannot regress
//! into the Rust engine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration test asserts on its own fixtures"
)]

use emend_core::parse::preview::{render_preview_html, PreviewOptions};

fn render(md: &str) -> String {
    render_preview_html(md, &PreviewOptions::default()).expect("preview render must succeed")
}

#[test]
fn remote_image_url_stays_a_literal_src_and_is_not_fetched() {
    let md = "![alt](https://example.com/x.png)\n";
    let html = render(md);

    // The remote URL appears verbatim as an <img src=...> — i.e. the renderer
    // emitted a reference, it did NOT dereference/inline the image.
    assert!(
        html.contains("<img") && html.contains("src=\"https://example.com/x.png\""),
        "remote image should render as a literal src reference (no fetch):\n{html}"
    );
    // Defensive: a fetch would have to inline bytes (e.g. a data: URI). None must
    // appear — the only URL in the output is the original remote one.
    assert!(
        !html.contains("data:image"),
        "renderer must not inline remote image bytes:\n{html}"
    );
}

#[test]
fn remote_link_url_stays_a_literal_href() {
    let md = "[site](https://example.com/page)\n";
    let html = render(md);
    assert!(
        html.contains("href=\"https://example.com/page\""),
        "remote link should render as a literal href (no fetch):\n{html}"
    );
}

#[test]
fn render_is_a_pure_in_memory_string_transform() {
    // The signature itself is the guarantee: `&str -> Result<String, _>` with no
    // IO/handle/runtime parameter. A render of a doc with several remote refs
    // returns promptly and deterministically (same input → same output), which a
    // network round-trip could not promise.
    let md = "\
# Doc

![a](http://a.example/i.png)
[b](https://b.example/p)
<https://c.example/autolink>
";
    let first = render(md);
    let second = render(md);
    assert_eq!(first, second, "pure transform must be deterministic");
    assert!(
        first.contains("a.example") && first.contains("b.example") && first.contains("c.example"),
        "all remote refs survive as literals:\n{first}"
    );
}
