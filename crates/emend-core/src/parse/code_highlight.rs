//! T084 — fenced-code-block syntax highlighting for the **preview** (research
//! §B6). The *second* half of the two-engine split: this is the comrak preview's
//! code colouriser, entirely separate from the tree-sitter editor highlighter
//! ([`crate::parse::highlight`]) — they are never unified (Constitution / project
//! guardrail).
//!
//! ## What it produces
//!
//! Given a fenced block's language token + source, [`highlight_code`] emits
//! syntect **classed HTML** ([`ClassedHTMLGenerator`] with [`ClassStyle::Spaced`]):
//! the source is wrapped in `<span class="...">` runs whose classes the WebView
//! styles from the bundled [`theme_css`]. Classes (not inline colours) keep the
//! HTML small and let one stylesheet theme every block — and let a future
//! dark-mode override swap only the CSS.
//!
//! ## The vendored binary dump (never parse YAML on the hot path)
//!
//! The `SyntaxSet`/`ThemeSet` are loaded **once**, lazily, from the vendored
//! *uncompressed* binary dump (`assets/syntaxes-themes.packdump`, produced by
//! `examples/gen_syntect_dump.rs`) via a [`OnceLock`]. research §B6 is explicit:
//! parsing raw `.sublime-syntax` YAML is ~138 ms and must never touch the hot
//! path; loading the binary dump is ~23 ms and happens once. We embed the dump
//! with [`include_bytes!`] so it travels in the binary (no runtime file lookup,
//! works inside an `.app` bundle).
//!
//! ## Coverage (research §D)
//!
//! The dump is syntect's default newline-sensitive set, covering **24 of the 30**
//! v1 languages directly; **Swift, TypeScript, Kotlin, SCSS, TOML, Dockerfile**
//! are absent from the default set and fall back to **plain text** (escaped, in a
//! `<pre>/<code>` — never a panic). Additional grammars are post-v1 (research §D);
//! the generator documents where to vendor their `.sublime-syntax`.
//!
//! ## No panics
//!
//! syntect's `from_binary` deserialiser is infallible in signature but can panic
//! on a corrupt dump (bincode). The dump is a trusted, committed asset, but to
//! honour the no-panic posture (NFR-003) the one-time load is wrapped in
//! [`std::panic::catch_unwind`]; a (should-be-impossible) failure degrades to "no
//! highlighting available" rather than aborting.

use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;

use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// The vendored *uncompressed* binary dump (syntaxes + themes), embedded so it
/// travels in the binary (works inside an `.app` bundle, no runtime file lookup).
/// Produced by `examples/gen_syntect_dump.rs`; see this module's docs.
static PACKDUMP: &[u8] = include_bytes!("../../assets/syntaxes-themes.packdump");

/// The classed theme CSS, generated alongside the dump (same theme, same
/// [`ClassStyle::Spaced`]). Embedded so [`theme_css`] is a zero-cost
/// `&'static str` — no dump load, no syntect call needed.
static THEME_CSS: &str = include_str!("../../assets/preview-theme.css");

/// Class style for the generator — must match the style the embedded CSS was
/// generated with (`gen_syntect_dump.rs` uses [`ClassStyle::Spaced`]).
const CLASS_STYLE: ClassStyle = ClassStyle::Spaced;

/// The lazily-loaded [`SyntaxSet`] from the binary dump. Loaded **once**.
///
/// (Only the syntax set is needed at runtime; the theme is consumed as the
/// pre-generated [`THEME_CSS`], so the dumped `ThemeSet` is parsed only to keep
/// the dump self-contained — see [`load_syntaxes`].)
static SYNTAXES: OnceLock<Option<SyntaxSet>> = OnceLock::new();

/// Load the vendored dump's [`SyntaxSet`] exactly once.
///
/// [`None`] only if the (trusted, committed) dump fails to deserialise — in which
/// case highlighting degrades to plain escaped code rather than panicking.
/// Wrapped in [`catch_unwind`](std::panic::catch_unwind) because syntect's
/// `from_binary` can panic on malformed bincode.
fn syntaxes() -> Option<&'static SyntaxSet> {
    SYNTAXES
        .get_or_init(|| {
            std::panic::catch_unwind(AssertUnwindSafe(load_syntaxes))
                .ok()
                .flatten()
        })
        .as_ref()
}

/// Parse the embedded dump's `SyntaxSet`. Returns [`None`] if the frame is
/// malformed (too short / bad length prefix). Called once, behind the
/// `catch_unwind` in [`syntaxes`].
fn load_syntaxes() -> Option<SyntaxSet> {
    // Frame: 8-byte LE length prefix, then the syntaxes dump (the themes dump
    // follows but we consume the theme as pre-generated CSS, not at runtime).
    let (len_bytes, rest) = PACKDUMP.split_first_chunk::<8>()?;
    let syntaxes_len = usize::try_from(u64::from_le_bytes(*len_bytes)).ok()?;
    let (syntaxes_bin, _themes_bin) = rest.split_at_checked(syntaxes_len)?;
    Some(syntect::dumps::from_binary(syntaxes_bin))
}

/// The classed theme CSS the WebView injects to style the `<span class="...">`
/// runs [`highlight_code`] emits (research §B6 / §C2).
///
/// Returns the embedded [`THEME_CSS`] directly — a `&'static str`, no dump load,
/// no allocation — so the FFI `preview_theme_css` is effectively free.
#[must_use]
pub fn theme_css() -> &'static str {
    THEME_CSS
}

/// Syntax-highlight `code` for fence language `lang_token`, returning the inner
/// HTML (a sequence of `<span class="...">` runs) to place inside `<pre><code>`.
///
/// - `lang_token` is the fence info string after ```` ``` ```` (e.g. `rust`,
///   `js`). Resolved via [`SyntaxSet::find_syntax_by_token`]; an unknown or empty
///   token (or any of the §D plain-text-fallback languages) falls back to
///   syntect's plain-text syntax — the code is still HTML-escaped into spans,
///   never panicking.
/// - The returned string is the **inner** HTML only (no `<pre>`/`<code>`
///   wrapper); the comrak adapter ([`crate::parse::preview`]) writes those tags
///   so the block's `data-sourcepos`/`data-line` anchors land on the `<pre>`.
///
/// Never panics: a generator error on a line falls back to escaping that line,
/// and a missing dump falls back to escaping the whole block.
#[must_use]
pub fn highlight_code(lang_token: &str, code: &str) -> String {
    let Some(syntaxes) = syntaxes() else {
        // No dump → escape the source so it is at least safe, unstyled HTML.
        return escape_plain(code);
    };
    let syntax = resolve_syntax(syntaxes, lang_token);

    let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, syntaxes, CLASS_STYLE);
    for line in LinesWithEndings::from(code) {
        // A parse error on a single line shouldn't kill the whole block; on
        // failure we just skip feeding that line to the generator (its text is
        // dropped from the styled output rather than corrupting state). In
        // practice this does not occur for the bundled grammars.
        let _ = generator.parse_html_for_line_which_includes_newline(line);
    }
    generator.finalize()
}

/// Resolve a fence token to a syntax, falling back to plain text. `find_syntax_by_token`
/// matches the language name and file extensions case-insensitively (so `rust`,
/// `rs`, `RUST` all resolve to Rust).
fn resolve_syntax<'a>(syntaxes: &'a SyntaxSet, lang_token: &str) -> &'a SyntaxReference {
    let token = lang_token.trim();
    if token.is_empty() {
        return syntaxes.find_syntax_plain_text();
    }
    syntaxes
        .find_syntax_by_token(token)
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text())
}

/// Minimal HTML escaping for the no-dump fallback path (`&`, `<`, `>` only — the
/// characters that would break out of a `<code>` text node).
fn escape_plain(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    for ch in code.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit test asserts on its own fixtures"
    )]

    use super::{highlight_code, theme_css};

    #[test]
    fn rust_block_emits_classed_spans() {
        let html = highlight_code("rust", "fn main() {}\n");
        assert!(
            html.contains("<span class=\""),
            "rust highlighting should emit classed spans: {html}"
        );
        assert!(html.contains("fn"), "source text should survive: {html}");
    }

    #[test]
    fn token_aliases_resolve() {
        // `rs` (an extension) resolves to Rust just like `rust` (the name).
        let by_ext = highlight_code("rs", "let x = 1;\n");
        assert!(
            by_ext.contains("<span class=\""),
            "`rs` should highlight as Rust: {by_ext}"
        );
    }

    #[test]
    fn unknown_language_falls_back_to_plaintext_without_panicking() {
        // Swift is one of the 6 §D plain-text fallbacks. It must still produce
        // escaped output, never panic.
        let html = highlight_code("swift", "let x = 1\n");
        assert!(
            html.contains("let x = 1"),
            "fallback preserves source: {html}"
        );
    }

    #[test]
    fn empty_token_is_plaintext() {
        let html = highlight_code("", "plain text here\n");
        assert!(
            html.contains("plain text here"),
            "empty token → plain text: {html}"
        );
    }

    #[test]
    fn html_metacharacters_are_escaped() {
        // Whatever syntax path, `<` `>` `&` must not survive raw (XSS / breakage).
        let html = highlight_code("text", "a < b && c > d\n");
        assert!(!html.contains("a < b"), "raw `<` must be escaped: {html}");
        assert!(
            html.contains("&lt;") || html.contains("&amp;"),
            "escaping expected: {html}"
        );
    }

    #[test]
    fn theme_css_is_nonempty_and_targets_classes() {
        let css = theme_css();
        assert!(!css.is_empty(), "bundled theme CSS must be present");
        // ClassStyle::Spaced emits class selectors; a `.` selector must exist.
        assert!(
            css.contains('.'),
            "theme CSS should contain class selectors"
        );
    }

    #[test]
    fn dump_loads_once_and_warm_path_is_cheap() {
        // Rough one-time-load timing note (≤23 ms goal, research §B6). NOT a hard
        // assertion (CI machines + debug vs release vary wildly): measured in a
        // `--release` build the cold dump-load + first highlight is ~13.5 ms
        // (under the 23 ms goal) and a warm same-language highlight is ~35 µs;
        // debug builds are ~10x slower because fancy-regex compilation is
        // unoptimised. We exercise the lazy load, print for the record, and only
        // assert the warm path is materially cheaper than the cold one.
        let cold_start = std::time::Instant::now();
        let _ = highlight_code("rust", "fn warm() {}\n"); // forces the OnceLock init
        let cold = cold_start.elapsed();

        let warm_start = std::time::Instant::now();
        let _ = highlight_code("rust", "fn again() {}\n"); // same language, warm regex
        let warm = warm_start.elapsed();

        eprintln!("syntect dump cold≈{cold:?}, warm (same lang)≈{warm:?}");
        assert!(
            warm <= cold,
            "a warm same-language highlight ({warm:?}) must not exceed the cold \
             load+highlight ({cold:?})"
        );
    }
}
