//! T083a — reproducible generator for the vendored binary syntect dump.
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run --example gen_syntect_dump
//! ```
//!
//! It serialises syntect's **default newline-sensitive** [`SyntaxSet`] and its
//! **default** [`ThemeSet`] into a single **uncompressed** binary dump at
//! `crates/emend-core/assets/syntaxes-themes.packdump`, plus the bundled theme's
//! classed CSS at `crates/emend-core/assets/preview-theme.css`. Those files are
//! **committed assets** (not build-time artifacts); rerun this only to refresh
//! them (e.g. after a syntect bump or a theme change).
//!
//! ## Why one *uncompressed* combined dump
//!
//! `parse::code_highlight` loads the dump exactly once, lazily, on the preview
//! path (research §B6: "load with lazy regex, ~23 ms"; the hot path must NEVER
//! parse raw YAML). We serialise with [`dump_binary`] so the load path is
//! [`SyntaxSet::from_binary`] / [`ThemeSet::from_binary`] — there is **no flate2
//! decompression** at load time. syntect gz-compresses by default to shrink the
//! on-disk dump, but for Emend the dump is a small in-tree asset and the load
//! happens once at preview-open on a background thread; trading a slightly larger
//! file for skipping decompression keeps that one-time load as cheap as possible.
//!
//! ## Coverage vs the §D 30-language v1 set
//!
//! `SyntaxSet::load_defaults_newlines()` is the practical base (research §B6).
//! Measured against the 30-language set (research §D), it covers **24**:
//!   Rust, JavaScript, Python, Go, C, C++, C#, Java, Ruby, PHP, Objective-C,
//!   Shell/Bash, SQL, HTML, CSS, JSON, YAML, XML, Markdown, Makefile, Lua,
//!   Haskell, Diff, plain-text.
//! The remaining **6 fall back to plain text** because they are absent from the
//! default set: **Swift, TypeScript, Kotlin, SCSS, TOML, Dockerfile**. Adding
//! them is a post-v1 enhancement (research §D explicitly defers extra languages)
//! — vendor their `.sublime-syntax` here and `add_from_folder` before dumping.
//! `code_highlight` degrades these to plain text today (never panics).
//!
//! NOTE: this is an `example`, not library code — it may use `expect`/`?` freely
//! (the workspace's no-panic lints apply to the crate's library/bin surface, not
//! to a developer-run generator).

use std::error::Error;
use std::path::PathBuf;

use syntect::highlighting::{Theme, ThemeSet};
use syntect::html::{css_for_theme_with_class_style, ClassStyle};
use syntect::parsing::SyntaxSet;

/// The theme bundled for the preview. InspiredGitHub reads well on a light
/// background and is the conventional GitHub-flavoured choice; the same classed
/// CSS is what the WebView injects. (Swift may add a dark-mode override later.)
const BUNDLED_THEME: &str = "InspiredGitHub";

/// `ClassStyle::Spaced` matches `code_highlight`'s `ClassedHTMLGenerator`, so the
/// emitted CSS targets the exact `class="..."` runs the generator produces.
const CLASS_STYLE: ClassStyle = ClassStyle::Spaced;

fn main() -> Result<(), Box<dyn Error>> {
    let assets = assets_dir();
    std::fs::create_dir_all(&assets)?;

    // 1. The combined binary dump (uncompressed) the hot path loads once.
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let theme_set = ThemeSet::load_defaults();

    let dump_path = assets.join("syntaxes-themes.packdump");
    let bundle = DumpBundle {
        syntaxes: syntect::dumps::dump_binary(&syntax_set),
        themes: syntect::dumps::dump_binary(&theme_set),
    };
    std::fs::write(&dump_path, bundle.encode())?;

    // 2. The classed theme CSS the WebView injects (research §B6 / §C2).
    let theme: &Theme = theme_set
        .themes
        .get(BUNDLED_THEME)
        .ok_or_else(|| format!("bundled theme {BUNDLED_THEME:?} missing from defaults"))?;
    let css = css_for_theme_with_class_style(theme, CLASS_STYLE)?;
    let css_path = assets.join("preview-theme.css");
    std::fs::write(&css_path, css.as_bytes())?;

    println!(
        "wrote {} ({} syntaxes) and {}",
        dump_path.display(),
        syntax_set.syntaxes().len(),
        css_path.display(),
    );
    Ok(())
}

/// The two dumps concatenated with a tiny length-prefixed framing so the loader
/// can split them back apart without a second file. `dump_binary` is the
/// *uncompressed* serializer (its compressed sibling is `dump_to_file`).
struct DumpBundle {
    syntaxes: Vec<u8>,
    themes: Vec<u8>,
}

impl DumpBundle {
    /// Frame: `[u64 syntaxes_len][syntaxes bytes][themes bytes]`, little-endian.
    /// The loader (`code_highlight::load_dump`) reads the length, splits, and
    /// `from_binary`s each half.
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.syntaxes.len() + self.themes.len());
        let len = self.syntaxes.len() as u64;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&self.syntaxes);
        out.extend_from_slice(&self.themes);
        out
    }
}

/// `crates/emend-core/assets/`, resolved from this example's manifest dir so the
/// generator works regardless of the invoking cwd.
fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}
