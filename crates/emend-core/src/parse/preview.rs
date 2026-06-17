//! T084 — the **authoritative** Markdown→HTML preview engine (research §B1,
//! FR-023..028, FFI contract §6).
//!
//! This is the comrak half of the deliberate two-engine split (research §B1,
//! Constitution / project guardrail): whole-document, CommonMark-correct,
//! debounced off the keystroke path. It is **never unified** with the incremental
//! tree-sitter editor highlighter ([`crate::parse::highlight`]) — that one is
//! advisory and fast; this one is correct and complete.
//!
//! ## What it renders
//!
//! [`render_preview_html`] runs comrak with CommonMark + GFM (tables, tasklist,
//! strikethrough, autolinks) plus the native extensions research §B1 calls for:
//! `[[wikilinks]]` and `==highlight==`→`<mark>`. Fenced code blocks are coloured
//! by syntect via a [`SyntaxHighlighterAdapter`] backed by
//! [`crate::parse::code_highlight`] (classed HTML + a bundled theme CSS).
//!
//! ## `data-line` scroll-sync anchors (research §C3)
//!
//! comrak's `render.sourcepos` annotates each rendered block with
//! `data-sourcepos="startLine:startCol-endLine:endCol"`. The Swift scroll-sync
//! (research §C3) wants a simple per-block **start line**, so we post-process the
//! HTML to add a `data-line="<startLine>"` attribute beside every
//! `data-sourcepos` (keeping the original too). This gives every top-level block
//! a 1-based source-line anchor the preview can map to/from the editor.
//!
//! ## Embeds (`![[embed]]`) — US5
//!
//! `![[embed]]` resolution — inlining another note's content with cycle/depth
//! guards (FR-021a) — is the **one custom extension comrak doesn't know** (research
//! §B1). [`render_preview_html_with_embeds`] runs the bespoke
//! [`crate::parse::embed::expand_embeds`] **source pass** over the Markdown
//! *before* handing it to comrak, splicing each resolved note's source inline (so
//! comrak parses the combined document as one). The plain [`render_preview_html`]
//! leaves embeds unexpanded (renders the literal `![[…]]` token) for callers that
//! have no resolver — e.g. a standalone snippet render with no workspace.
//!
//! ## Purity (no network, no async)
//!
//! Rendering is a pure `&str -> Result<String, _>` transform: in-memory input,
//! in-memory output, no IO, no `tokio`, no HTTP client reachable from this path
//! (SC-008; guarded by `tests/preview_offline.rs`). Remote URLs in the source
//! (`![img](https://…)`) are emitted as literal `src=`/`href=` references — never
//! dereferenced. The runtime offline guarantee for the rendered page is enforced
//! Swift-side by the WKWebView CSP (T087); this engine simply never fetches.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use comrak::adapters::SyntaxHighlighterAdapter;
use comrak::options::Plugins;
use comrak::{markdown_to_html_with_plugins, Options};

use crate::error::EmendError;
use crate::parse::code_highlight;
use crate::parse::embed::{expand_embeds, EmbedOptions};

/// Tuning knobs for a preview render. Defaults match the FFI `render_preview_html`
/// (all the §B1 extensions on, scroll-sync anchors on).
///
/// Kept as a struct (rather than bare bools) so US5's embed options
/// ([`EmbedOptions`]'s `max_depth` + cycle detection) ride alongside without
/// changing the public render signature.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PreviewOptions {
    /// Emit `data-line` scroll-sync anchors on each block (research §C3). On by
    /// default; the FFI path always wants them.
    pub source_line_anchors: bool,
    /// Embed expansion knobs (US5 · FR-021a): the max nesting depth + cycle
    /// guard used by [`render_preview_html_with_embeds`]. Ignored by the plain
    /// [`render_preview_html`] (which never expands embeds).
    pub embed: EmbedOptions,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            source_line_anchors: true,
            embed: EmbedOptions::default(),
        }
    }
}

/// Render Markdown `source` to preview HTML **without expanding embeds** (FFI
/// contract §6).
///
/// Uses comrak (CommonMark + GFM + wikilinks + `==highlight==`) with syntect-
/// coloured code blocks, and — when [`PreviewOptions::source_line_anchors`] —
/// adds a `data-line="<startLine>"` anchor to every block for scroll-sync.
///
/// `![[embed]]` tokens are left **literal** here (rendered as their raw text):
/// embed resolution needs a workspace resolver, which this signature does not
/// take. Callers with a resolver use [`render_preview_html_with_embeds`]; this
/// remains for standalone snippet rendering with no workspace.
///
/// # Errors
///
/// [`EmendError::Internal`] if rendering fails unexpectedly. Rendering is pure and
/// does not normally fail; the `Result` matches the FFI contract's
/// `render_preview_html -> Result<String, EmendError>`.
pub fn render_preview_html(source: &str, options: &PreviewOptions) -> Result<String, EmendError> {
    render_html(source, options)
}

/// Render Markdown `source` to preview HTML with `![[embed]]` resolution
/// (FFI contract §6; US5 · FR-021/021a).
///
/// Runs the bespoke embed **source pass** ([`expand_embeds`]) first — inlining
/// each resolved note's Markdown, with cycle detection and the depth bound from
/// [`PreviewOptions::embed`] (FR-021a) — then renders the spliced document with
/// the same comrak pipeline as [`render_preview_html`]. `from_note` is the
/// resolved path of `source`'s own note (the FR-019a anchor for its top-level
/// embeds); `resolve` maps `(target, from_note)` to that note's source **and its
/// resolved path**, or `None` if unresolved (the FFI layer wires it to the
/// workspace index + a tolerant on-disk read). The returned resolved path anchors
/// *nested* embeds on their immediate parent note, not the top document (FR-019a;
/// see [`crate::parse::embed`]).
///
/// An unresolved/cyclic/too-deep embed degrades to a visible placeholder rather
/// than looping (FR-021a/FR-022); see [`crate::parse::embed`].
///
/// # Errors
///
/// [`EmendError::Internal`] if rendering fails unexpectedly (rendering is pure
/// and does not normally fail).
pub fn render_preview_html_with_embeds<R>(
    source: &str,
    from_note: &str,
    options: &PreviewOptions,
    resolve: &mut R,
) -> Result<String, EmendError>
where
    R: FnMut(&str, &str) -> Option<(String, String)>,
{
    // US5: splice embedded note sources in BEFORE comrak parses, so headings,
    // code fences, tables, and nested embeds in the embedded note render in the
    // surrounding document's context. Cycle + depth guards live in `expand_embeds`,
    // which also threads each note's resolved path as the `from_note` for ITS
    // nested embeds (FR-019a per-parent anchoring).
    let expanded = expand_embeds(source, from_note, &options.embed, resolve);
    render_html(&expanded, options)
}

/// Shared comrak render + optional `data-line` post-pass for both
/// [`render_preview_html`] and [`render_preview_html_with_embeds`].
fn render_html(source: &str, options: &PreviewOptions) -> Result<String, EmendError> {
    let comrak_opts = build_options();

    // The syntect adapter colours fenced code blocks via `code_highlight`.
    let adapter = EmendSyntectAdapter;
    let mut plugins = Plugins::default();
    plugins.render.codefence_syntax_highlighter = Some(&adapter);

    let html = markdown_to_html_with_plugins(source, &comrak_opts, &plugins);

    if options.source_line_anchors {
        Ok(add_data_line_anchors(&html))
    } else {
        Ok(html)
    }
}

/// Build comrak options: CommonMark + the GFM/native extensions (research §B1)
/// and `render.sourcepos` (so [`add_data_line_anchors`] has something to map).
fn build_options() -> Options<'static> {
    let mut o = Options::default();

    // GFM extensions §B1 calls native.
    o.extension.table = true;
    o.extension.tasklist = true;
    o.extension.strikethrough = true;
    o.extension.autolink = true;

    // Native custom extensions §B1 calls for.
    o.extension.highlight = true; // ==highlight== -> <mark>
    o.extension.wikilinks_title_after_pipe = true; // [[Target]] / [[Target|Title]]

    // `![[embed]]` is the one custom extension comrak doesn't know (research §B1):
    // it is resolved by a SOURCE pass (`render_preview_html_with_embeds` →
    // `crate::parse::embed::expand_embeds`) that runs BEFORE this comrak render,
    // so by the time comrak parses, embeds are already spliced in as inline
    // Markdown. No comrak option configures it.

    // Scroll-sync anchors (research §C3): emit data-sourcepos on every block; the
    // post-pass derives data-line from it.
    o.render.sourcepos = true;

    // Keep raw HTML *escaped* (the default `unsafe_` = false). The preview is
    // untrusted user content rendered in a WebView; we never want to emit raw
    // <script>. Remote URLs in links/images are still emitted as literal
    // attributes (comrak does not fetch them) — see tests/preview_offline.rs.

    o
}

/// Post-process comrak HTML to add `data-line="<startLine>"` beside every
/// `data-sourcepos="<startLine>:<col>-..."` (research §C3).
///
/// comrak writes ` data-sourcepos="L:C-L:C"` on each block. We scan for that
/// attribute, parse the leading line number, and insert ` data-line="L"` right
/// before it. The original `data-sourcepos` is preserved (some consumers want the
/// full span). Pure string work, single pass, no regex dependency.
fn add_data_line_anchors(html: &str) -> String {
    const NEEDLE: &str = " data-sourcepos=\"";
    let mut out = String::with_capacity(html.len() + html.len() / 16);
    let mut rest = html;

    while let Some(pos) = rest.find(NEEDLE) {
        // Everything up to (but not including) the attribute — emit verbatim.
        out.push_str(&rest[..pos]);

        // The attribute value starts after the needle; the start line is the run
        // of ASCII digits up to the first ':'.
        let after = &rest[pos + NEEDLE.len()..];
        let digit_len = after.bytes().take_while(u8::is_ascii_digit).count();

        if digit_len == 0 {
            // Malformed (no leading digit) — emit the needle unchanged and
            // advance past it so the scan continues on the tail (no infinite loop,
            // no duplicated content).
            out.push_str(NEEDLE);
            rest = after;
            continue;
        }

        // Insert ` data-line="L"` before the original ` data-sourcepos="…"`, then
        // re-emit the needle. The attribute VALUE and everything after it stays in
        // `rest` for the next iteration so it is copied exactly once.
        out.push_str(" data-line=\"");
        out.push_str(&after[..digit_len]);
        out.push('"');
        out.push_str(NEEDLE);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// comrak [`SyntaxHighlighterAdapter`] that delegates to
/// [`crate::parse::code_highlight`] (syntect classed HTML).
///
/// `write_highlighted` produces the inner classed spans; `write_pre_tag` /
/// `write_code_tag` emit plain `<pre>`/`<code>` carrying comrak's attributes
/// (which include `data-sourcepos` for code blocks, so the scroll-sync anchor
/// lands on the `<pre>`).
struct EmendSyntectAdapter;

impl SyntaxHighlighterAdapter for EmendSyntectAdapter {
    fn write_highlighted(
        &self,
        output: &mut dyn fmt::Write,
        lang: Option<&str>,
        code: &str,
    ) -> fmt::Result {
        let token = lang.unwrap_or("");
        output.write_str(&code_highlight::highlight_code(token, code))
    }

    fn write_pre_tag(
        &self,
        output: &mut dyn fmt::Write,
        attributes: HashMap<&'static str, Cow<'_, str>>,
    ) -> fmt::Result {
        write_tag(output, "pre", &attributes)
    }

    fn write_code_tag(
        &self,
        output: &mut dyn fmt::Write,
        attributes: HashMap<&'static str, Cow<'_, str>>,
    ) -> fmt::Result {
        write_tag(output, "code", &attributes)
    }
}

/// Write an opening `<tag attr="val" …>` from comrak's attribute map. comrak
/// supplies already-safe attribute values (e.g. `class="language-rust"`,
/// `data-sourcepos="…"`); we emit them in a stable order (sorted by name) so the
/// output is deterministic across runs (`HashMap` iteration order is not).
fn write_tag(
    output: &mut dyn fmt::Write,
    tag: &str,
    attributes: &HashMap<&'static str, Cow<'_, str>>,
) -> fmt::Result {
    write!(output, "<{tag}")?;
    let mut keys: Vec<&&'static str> = attributes.keys().collect();
    keys.sort_unstable();
    for key in keys {
        if let Some(val) = attributes.get(*key) {
            write!(output, " {key}=\"{val}\"")?;
        }
    }
    output.write_str(">")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{
        add_data_line_anchors, render_preview_html, render_preview_html_with_embeds, PreviewOptions,
    };

    #[test]
    fn renders_heading_with_data_line() {
        let html = render_preview_html("# Hi\n", &PreviewOptions::default()).unwrap();
        assert!(html.contains("data-line=\"1\""), "{html}");
        assert!(html.contains("<h1"), "{html}");
    }

    #[test]
    fn anchors_can_be_disabled() {
        let opts = PreviewOptions {
            source_line_anchors: false,
            ..PreviewOptions::default()
        };
        let html = render_preview_html("# Hi\n", &opts).unwrap();
        assert!(!html.contains("data-line="), "anchors off: {html}");
        // sourcepos is still emitted (it's how we'd add anchors), but no data-line.
        assert!(html.contains("data-sourcepos="), "{html}");
    }

    #[test]
    fn data_line_added_for_each_block() {
        let html =
            render_preview_html("# A\n\npara\n\n## B\n", &PreviewOptions::default()).unwrap();
        assert!(html.contains("data-line=\"1\""), "h1 at line 1: {html}");
        assert!(html.contains("data-line=\"3\""), "para at line 3: {html}");
        assert!(html.contains("data-line=\"5\""), "h2 at line 5: {html}");
    }

    #[test]
    fn add_data_line_anchors_handles_malformed_gracefully() {
        // No digit after the needle → left unchanged, no infinite loop.
        let input = "<p data-sourcepos=\"x\">hi</p>";
        let out = add_data_line_anchors(input);
        assert_eq!(out, input);
    }

    #[test]
    fn code_block_is_syntect_classed() {
        let html =
            render_preview_html("```rust\nfn x() {}\n```\n", &PreviewOptions::default()).unwrap();
        assert!(html.contains("<pre"), "{html}");
        assert!(
            html.contains("<span class=\""),
            "classed spans expected: {html}"
        );
    }

    #[test]
    fn code_block_pre_carries_data_line() {
        // The code fence starts on line 1, so the <pre> should anchor there.
        let html =
            render_preview_html("```rust\nfn x() {}\n```\n", &PreviewOptions::default()).unwrap();
        assert!(
            html.contains("data-line=\"1\""),
            "code <pre> anchor: {html}"
        );
    }

    #[test]
    fn embed_is_inlined_and_rendered_through_comrak() {
        // `![[child]]` resolves to a note whose source is a heading; the spliced
        // document renders that heading as an <h2> (proving source-level splice).
        let html = render_preview_html_with_embeds(
            "intro\n\n![[child]]\n",
            "/parent.md",
            &PreviewOptions::default(),
            &mut |name, _from| {
                (name == "child")
                    .then(|| ("## Embedded Heading\n".to_owned(), "/child.md".to_owned()))
            },
        )
        .unwrap();
        assert!(
            html.contains("<h2"),
            "embedded heading should render: {html}"
        );
        assert!(html.contains("Embedded Heading"), "{html}");
        assert!(!html.contains("![[child]]"), "raw embed token gone: {html}");
    }

    #[test]
    fn plain_render_leaves_embed_literal() {
        // Without a resolver, `render_preview_html` does not expand embeds.
        let html = render_preview_html("![[child]]\n", &PreviewOptions::default()).unwrap();
        // The raw target survives somewhere in the output (rendered literally,
        // not inlined). comrak may treat `[[child]]` as a wikilink, but the text
        // `child` and no embedded content is the point.
        assert!(html.contains("child"), "{html}");
        assert!(!html.contains("Embedded"), "nothing was inlined: {html}");
    }

    #[test]
    fn embed_cycle_terminates_in_preview_path() {
        // A↔B cycle through the full preview render must terminate (FR-021a).
        let html = render_preview_html_with_embeds(
            "![[a]]\n",
            "/a.md",
            &PreviewOptions::default(),
            &mut |name, _from| match name {
                "a" => Some(("A ![[b]]\n".to_owned(), "/a.md".to_owned())),
                "b" => Some(("B ![[a]]\n".to_owned(), "/b.md".to_owned())),
                _ => None,
            },
        )
        .unwrap();
        assert!(
            html.len() < 20_000,
            "cyclic embed output bounded: {}",
            html.len()
        );
        assert!(html.contains('A') && html.contains('B'));
    }
}
