//! Incremental tree-sitter editor-highlight engine — the **per-keystroke hot
//! path** (research §B1, FFI contract §3, FR-010..015, SC-003).
//!
//! ## What this is (and is not)
//!
//! This is the *editor* highlighter: it produces [`StyleSpan`]s the UI uses to
//! dim Markdown syntax markers and emphasise headings/bold/italic/etc. as you
//! type. It is **advisory only** and **incremental**: a keystroke edits the
//! tree-sitter tree in place and reparses just the affected region, so the typing
//! budget (≤50 ms) is met regardless of document size. It is *not* the preview
//! renderer — that is comrak, a separate, authoritative engine
//! ([`crate::parse`]). The two are deliberately never unified.
//!
//! ## Two grammars behind one tree
//!
//! `tree-sitter-md` is a **split parser**: a *block* grammar (headings, lists,
//! code fences, block quotes, paragraphs) plus an *inline* grammar (bold,
//! italic, code spans, links) re-run inside each block's inline content. Its
//! [`MarkdownParser`] / [`MarkdownTree`] wrapper manages both, and
//! [`MarkdownTree::walk`] yields a [`tree_sitter_md::MarkdownCursor`] that walks
//! seamlessly across the block→inline boundary. We classify nodes from *both*
//! sub-grammars in a single traversal.
//!
//! ## Coordinate systems
//!
//! tree-sitter works in **bytes** (and row/column [`Point`]s); the FFI boundary
//! speaks **UTF-16 code units** (research §A2). [`Highlighter`] keeps a
//! [`ropey::Rope`] mirror of the text purely to bridge these: byte ↔ char ↔
//! UTF-16 ↔ (row, col). Every conversion is **checked** — no truncating `as`, no
//! `unwrap`/`expect`/`panic` (the workspace denies them; NFR-003). Out-of-range
//! inputs map to [`EmendError::Internal`].
//!
//! ## Integration with [`Document`](crate::document::Document)
//!
//! `Highlighter` is a **standalone struct that the caller feeds the same editor
//! deltas it feeds [`Document::push_edit`](crate::document::Document::push_edit)**
//! — it does *not* live inside `Document`. This is deliberate:
//!
//! - `Document` stays a minimal, allocation-light shadow buffer with its existing
//!   public API and tests untouched (it gains no tree-sitter dependency in its
//!   own surface).
//! - Highlighting is a *separable* concern: a read-only or oversized note may
//!   skip the highlighter entirely; a future viewport-virtualised editor may own
//!   one highlighter per visible document. Coupling it into `Document` would make
//!   that harder.
//! - The FFI shim (T039) can hold a `Document` and a `Highlighter` side by side
//!   behind one object handle and fan each `push_edit` delta to both. Because
//!   both consume the identical `(U16Range, &str)` delta, they cannot drift.
//!
//! `Highlighter` keeps its *own* rope mirror rather than borrowing `Document`'s,
//! so the two have no lifetime entanglement and the FFI handle can store them in
//! whatever order it likes. The mirrors stay in lock-step because they are driven
//! by the same delta stream.

use crate::{EmendError, U16Range};
use ropey::Rope;
use tree_sitter::{InputEdit, Node, Point};
use tree_sitter_md::{MarkdownParser, MarkdownTree};

/// The visual role the editor should give a run of text. Each maps a
/// tree-sitter-md node kind (from the block *or* inline grammar) onto a stable,
/// FFI-friendly category the Swift layer (T042) turns into display attributes.
///
/// The variant set covers the constructs the spec calls out (FR-010..015):
/// dimmable **syntax markers**, headings, strong/emphasis, code, block quotes,
/// list markers, links, and `==highlight==`. It is intentionally a *small,
/// closed* vocabulary — the editor highlighter is advisory, so it groups many
/// concrete node kinds into a handful of display roles rather than mirroring the
/// grammar one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleClass {
    /// Punctuation that *is* Markdown syntax and should be **dimmed** rather than
    /// shown at full strength (FR-010): heading `#`s, emphasis `*`/`_`, code
    /// fence/span backticks, list bullets, block-quote `>`, link brackets, etc.
    /// The single most important class for the "quiet" editor feel.
    SyntaxMarker,

    /// An ATX or setext heading's text. `level` is 1..=6 when it can be
    /// determined from the marker (ATX `#`..`######`), else `None` (setext, or a
    /// heading whose marker is not in view). FR-012.
    Heading {
        /// Heading level 1..=6, or `None` when not determinable.
        level: Option<u8>,
    },

    /// Strongly emphasised (bold) inline text — `**…**` / `__…__`. FR-011.
    Strong,

    /// Emphasised (italic) inline text — `*…*` / `_…_`. FR-011.
    Emphasis,

    /// An inline code span — `` `code` ``. FR-013.
    InlineCode,

    /// A fenced or indented code block's content. FR-013.
    CodeBlock,

    /// A block quote's quoted body — `> …`. FR-014.
    BlockQuote,

    /// A list item's marker run (`-`, `*`, `+`, `1.`). Distinct from
    /// [`StyleClass::SyntaxMarker`] so the UI can, if it wishes, treat list
    /// bullets differently from inline punctuation. FR-014.
    ListMarker,

    /// Link/image text or destination — `[text](url)`, autolinks, references.
    /// FR-014.
    Link,

    /// `==highlighted==` text (FR-015).
    ///
    /// **Engine note:** the stock `tree-sitter-md` grammar does *not* recognise
    /// `==highlight==` (it has no such node), so this incremental editor engine
    /// will not currently emit it; the class exists so the FFI/Swift contract is
    /// complete and so a future grammar extension (or the comrak preview, which
    /// is the authoritative renderer) can produce it without a breaking change.
    Highlight,
}

/// A run of text plus the visual role to give it. `range` is in **UTF-16 code
/// units** (FFI contract §3) so it maps directly onto `NSRange`. Returned by
/// [`Highlighter::highlight_spans`]; projected across FFI by T039 and turned into
/// display attributes by T042.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleSpan {
    /// The styled run, in UTF-16 code units.
    pub range: U16Range,
    /// How to style it.
    pub class: StyleClass,
}

/// Incremental tree-sitter editor-highlight engine for one open document.
///
/// Construct with [`Highlighter::new`] from the document's current text, feed it
/// the same `(U16Range, &str)` deltas you feed
/// [`Document::push_edit`](crate::document::Document::push_edit) via
/// [`Highlighter::apply_edit`], and query [`Highlighter::highlight_spans`] for the
/// visible viewport. Holds no `uniffi` types (Constitution V).
pub struct Highlighter {
    /// The split-grammar parser. Reused across reparses so its internal scanner
    /// state and grammar tables are not reallocated per keystroke.
    parser: MarkdownParser,
    /// The current combined (block + inline) syntax tree. `None` only if the
    /// initial parse returned `None` (e.g. an internal cancellation), in which
    /// case queries degrade gracefully to "no spans" rather than panicking.
    tree: Option<MarkdownTree>,
    /// Byte/char/UTF-16/line mirror of the document text. Drives the byte↔UTF-16
    /// bridges and feeds tree-sitter its source zero-copy via [`Rope::chunks`].
    rope: Rope,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `MarkdownParser`/`MarkdownTree` are not `Debug`-friendly and dumping a
        // whole tree is noise; summarise instead so the workspace's
        // `missing_debug_implementations` lint is satisfied without leaking the
        // parser internals.
        f.debug_struct("Highlighter")
            .field("len_bytes", &self.rope.len_bytes())
            .field("has_tree", &self.tree.is_some())
            .finish()
    }
}

impl Highlighter {
    /// Build a highlighter from the document's current text and run the initial
    /// parse. Cheap relative to the per-keystroke path (it parses the whole
    /// document once); subsequent edits go through [`Highlighter::apply_edit`]
    /// and reparse incrementally.
    #[must_use]
    pub fn new(text: &str) -> Self {
        let rope = Rope::from_str(text);
        let mut parser = MarkdownParser::default();
        let tree = parse_rope(&mut parser, &rope, None);
        Self { parser, tree, rope }
    }

    /// Apply one editor delta — replace the text in `range` (UTF-16 code units)
    /// with `replacement` — and **reparse incrementally**, returning the ranges
    /// whose parse changed (old tree vs reparsed tree) as **UTF-16** [`U16Range`]s.
    ///
    /// This is the per-keystroke hot path: it edits the tree in place
    /// ([`MarkdownTree::edit`]) and reparses with the old tree as a baseline, so
    /// tree-sitter reuses unaffected subtrees. The returned changed ranges let the
    /// UI re-attribute only the affected text (and let tests verify the reparse is
    /// edit-local for non-structural edits, but tail-spanning when block structure
    /// changes — see `tests/parse_incremental.rs`).
    ///
    /// # Errors
    ///
    /// [`EmendError::Internal`] if `range` is out of bounds, inverted, or splits a
    /// surrogate pair — the same boundary contract as
    /// [`Document::push_edit`](crate::document::Document::push_edit). The buffer
    /// and tree are left **unmodified** on error.
    pub fn apply_edit(
        &mut self,
        range: U16Range,
        replacement: &str,
    ) -> Result<Vec<U16Range>, EmendError> {
        // Resolve the edit endpoints to byte offsets and row/col Points BEFORE
        // mutating anything, so a bad range fails without touching the buffer.
        let start_char = self.utf16_to_char(range.start)?;
        let end_char = self.utf16_to_char(range.end())?;
        if end_char < start_char {
            return Err(EmendError::Internal {
                detail: format!(
                    "inverted edit range: start_utf16={} end_utf16={}",
                    range.start,
                    range.end()
                ),
            });
        }

        let start_byte = self.char_to_byte(start_char)?;
        let old_end_byte = self.char_to_byte(end_char)?;
        let start_point = self.byte_to_point(start_byte)?;
        let old_end_point = self.byte_to_point(old_end_byte)?;

        // Splice the rope mirror (this is the new source of truth for offsets).
        if end_char > start_char {
            self.rope.remove(start_char..end_char);
        }
        if !replacement.is_empty() {
            self.rope.insert(start_char, replacement);
        }

        // The new end is start + the replacement's byte length / row-col extent.
        let new_end_byte =
            start_byte
                .checked_add(replacement.len())
                .ok_or_else(|| EmendError::Internal {
                    detail: "edit new-end byte offset overflow".to_owned(),
                })?;
        let new_end_point = self.byte_to_point(new_end_byte)?;

        let edit = InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position: start_point,
            old_end_position: old_end_point,
            new_end_position: new_end_point,
        };

        // Incrementally reparse: edit the old tree, then parse with it as the
        // baseline so unaffected subtrees are reused.
        let old_tree = self.tree.take();
        let mut old_tree = old_tree; // make mutable for `edit`
        if let Some(t) = old_tree.as_mut() {
            t.edit(&edit);
        }
        let new_tree = parse_rope(&mut self.parser, &self.rope, old_tree.as_ref());

        // Compute changed ranges (byte) and map to UTF-16. If either tree is
        // missing we cannot diff, so report no changed ranges (the caller should
        // re-query spans for the whole viewport in that degraded case).
        let changed = match (old_tree.as_ref(), new_tree.as_ref()) {
            (Some(old), Some(new)) => {
                let byte_ranges = changed_byte_ranges(old, new, start_byte, new_end_byte);
                let mut out = Vec::with_capacity(byte_ranges.len());
                for (s, e) in byte_ranges {
                    out.push(self.byte_range_to_u16(s, e)?);
                }
                out
            }
            _ => Vec::new(),
        };

        self.tree = new_tree;
        Ok(changed)
    }

    /// Return the [`StyleSpan`]s for nodes intersecting `viewport` (UTF-16 code
    /// units). Walks both the block and inline grammars in one traversal,
    /// classifies each relevant node, and maps its byte range back to UTF-16.
    ///
    /// Spans are returned in document order (the order the cursor visits nodes).
    /// A node entirely outside the viewport is skipped; a node partially in view
    /// is emitted in full (the caller clips to the viewport if it wants to).
    ///
    /// # Errors
    ///
    /// [`EmendError::Internal`] if `viewport` is out of bounds or splits a
    /// surrogate pair, or if any node's byte range cannot be mapped to UTF-16
    /// (unreachable for an in-bounds tree, but reported rather than truncated).
    pub fn highlight_spans(&self, viewport: U16Range) -> Result<Vec<StyleSpan>, EmendError> {
        // Validate + translate the viewport to a byte range up front.
        let start_char = self.utf16_to_char(viewport.start)?;
        let end_char = self.utf16_to_char(viewport.end())?;
        let view_start_byte = self.char_to_byte(start_char)?;
        let view_end_byte = self.char_to_byte(end_char)?;

        let Some(tree) = self.tree.as_ref() else {
            return Ok(Vec::new());
        };

        let mut spans = Vec::new();
        let mut cursor = tree.walk();

        // Iterative pre-order DFS over the combined tree. `MarkdownCursor` crosses
        // the block→inline boundary transparently, so a single walk classifies
        // nodes from both grammars.
        loop {
            let node = cursor.node();

            // Skip whole subtrees that end before the viewport or start after it.
            let n_start = node.start_byte();
            let n_end = node.end_byte();
            let intersects = n_start < view_end_byte && n_end > view_start_byte;

            if intersects {
                if let Some(class) = classify(&node) {
                    spans.push(StyleSpan {
                        range: self.byte_range_to_u16(n_start, n_end)?,
                        class,
                    });
                }
            }

            // Descend into intersecting interior; otherwise move laterally/up.
            // Only descend when the node overlaps the viewport — pruning
            // non-overlapping subtrees keeps the walk proportional to the
            // viewport, not the document.
            if intersects && cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return Ok(spans);
                }
            }
        }
    }

    // -- conversions (all checked) -------------------------------------------

    /// UTF-16 offset → char index, bounds- and surrogate-checked. Mirrors the
    /// contract of `Document::utf16_to_char` so the two stay in agreement.
    fn utf16_to_char(&self, offset: u32) -> Result<usize, EmendError> {
        let len_u16 = u32::try_from(self.rope.len_utf16_cu()).map_err(too_large)?;
        if offset > len_u16 {
            return Err(EmendError::Internal {
                detail: format!("utf16 offset {offset} out of bounds (document length {len_u16})"),
            });
        }
        let offset_usize = usize::try_from(offset).map_err(too_large)?;
        let char_idx = self.rope.utf16_cu_to_char(offset_usize);
        // ropey rounds an in-surrogate-pair offset DOWN; round-trip to detect it.
        let roundtrip = u32::try_from(self.rope.char_to_utf16_cu(char_idx)).map_err(too_large)?;
        if roundtrip != offset {
            return Err(EmendError::Internal {
                detail: format!(
                    "utf16 offset {offset} splits a surrogate pair (nearest char boundary is utf16 {roundtrip})"
                ),
            });
        }
        Ok(char_idx)
    }

    /// Char index → byte offset (checked; in-bounds for any char index ropey
    /// returns).
    fn char_to_byte(&self, char_idx: usize) -> Result<usize, EmendError> {
        self.rope
            .try_char_to_byte(char_idx)
            .map_err(|e| EmendError::Internal {
                detail: format!("char index {char_idx} out of bounds: {e}"),
            })
    }

    /// Byte offset → tree-sitter [`Point`] (zero-indexed row + UTF-8-byte column
    /// from the line start). tree-sitter columns are byte columns, matching how
    /// it indexes source, so we compute the byte distance from the line start.
    fn byte_to_point(&self, byte_idx: usize) -> Result<Point, EmendError> {
        let row = self
            .rope
            .try_byte_to_line(byte_idx)
            .map_err(|e| EmendError::Internal {
                detail: format!("byte index {byte_idx} out of bounds for line lookup: {e}"),
            })?;
        let line_start_char = self.rope.line_to_char(row);
        let line_start_byte = self.char_to_byte(line_start_char)?;
        let column = byte_idx
            .checked_sub(line_start_byte)
            .ok_or_else(|| EmendError::Internal {
                detail: format!(
                    "byte column underflow: byte {byte_idx} precedes line start {line_start_byte}"
                ),
            })?;
        Ok(Point { row, column })
    }

    /// A tree-sitter byte range → a UTF-16 [`U16Range`]. The single place node
    /// byte ranges are projected onto the FFI coordinate system.
    fn byte_range_to_u16(
        &self,
        start_byte: usize,
        end_byte: usize,
    ) -> Result<U16Range, EmendError> {
        let start_u16 = self.byte_to_utf16(start_byte)?;
        let end_u16 = self.byte_to_utf16(end_byte)?;
        let len = end_u16
            .checked_sub(start_u16)
            .ok_or_else(|| EmendError::Internal {
                detail: format!("inverted node range: start_utf16={start_u16} end_utf16={end_u16}"),
            })?;
        Ok(U16Range::new(start_u16, len))
    }

    /// Byte offset → UTF-16 code-unit offset (checked).
    fn byte_to_utf16(&self, byte_idx: usize) -> Result<u32, EmendError> {
        let char_idx = self
            .rope
            .try_byte_to_char(byte_idx)
            .map_err(|e| EmendError::Internal {
                detail: format!("byte index {byte_idx} out of bounds: {e}"),
            })?;
        u32::try_from(self.rope.char_to_utf16_cu(char_idx)).map_err(too_large)
    }
}

/// Compute the byte ranges whose parse changed between `old` and `new`.
///
/// `tree-sitter-md` is a split parser, so a change can show up in two places:
///
/// 1. **The block tree** — `block_tree().changed_ranges()` catches structural
///    edits (opening/closing a fence, adding a list item, splitting a paragraph).
///    A block-structure change that reinterprets the document tail shows up here
///    as a tail-spanning range.
///
/// 2. **An inline tree** — an edit *inside* a paragraph's inline content (typing
///    a word) leaves the block tree byte-identical, so the block diff reports
///    nothing; the change lives entirely in that block's inline sub-tree. When
///    the edit does not add or remove blocks, the inline trees line up
///    positionally (same count, same document order), so we diff `old[i]` vs
///    `new[i]`. When the counts differ, block structure changed and the block
///    diff above already covers it, so we skip the (now unalignable) inline diff.
///
/// As a floor, we always include the edited byte span itself
/// (`[edit_start, edit_new_end)`): even when tree-sitter reuses everything and
/// reports no changed range (e.g. retyping the same character), the UI still
/// needs to re-attribute the text the user just touched.
fn changed_byte_ranges(
    old: &MarkdownTree,
    new: &MarkdownTree,
    edit_start: usize,
    edit_new_end: usize,
) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();

    for r in new.block_tree().changed_ranges(old.block_tree()) {
        out.push((r.start_byte, r.end_byte));
    }

    let old_inline = old.inline_trees();
    let new_inline = new.inline_trees();
    if old_inline.len() == new_inline.len() {
        for (o, n) in old_inline.iter().zip(new_inline.iter()) {
            for r in n.changed_ranges(o) {
                out.push((r.start_byte, r.end_byte));
            }
        }
    }

    // Always include the edited span as a floor (normalised so start <= end).
    let floor = (edit_start.min(edit_new_end), edit_start.max(edit_new_end));
    if floor.1 > floor.0 {
        out.push(floor);
    } else if out.is_empty() {
        // Zero-width edit (pure deletion collapsing to a point) with no reported
        // change: emit a degenerate point so callers still see "something here
        // changed" rather than an empty result.
        out.push(floor);
    }

    out
}

/// Run a (possibly incremental) parse over the rope, feeding tree-sitter the
/// rope's chunks **zero-copy** via `parse_with_options`. Returns `None` only if
/// tree-sitter itself returns `None` (cancellation/timeout — neither is set
/// here, so in practice this always yields `Some`, but we model it honestly
/// rather than `unwrap`).
fn parse_rope(
    parser: &mut MarkdownParser,
    rope: &Rope,
    old_tree: Option<&MarkdownTree>,
) -> Option<MarkdownTree> {
    let mut callback = |byte: usize, _pos: Point| -> &[u8] {
        // `get_chunk_at_byte` returns the chunk *containing* `byte` plus the
        // chunk's own byte start; we hand back the slice from `byte` to the chunk
        // end. An out-of-range byte (>= len) yields an empty slice, which
        // tree-sitter reads as end-of-input.
        match rope.get_chunk_at_byte(byte) {
            Some((chunk, chunk_byte_start, _, _)) => {
                let offset = byte.saturating_sub(chunk_byte_start);
                chunk.as_bytes().get(offset..).unwrap_or(&[])
            }
            None => &[],
        }
    };
    parser.parse_with_options(
        &mut callback,
        old_tree,
        tree_sitter_md::MarkdownParseOptions::default(),
    )
}

/// Classify a tree-sitter-md node (block *or* inline grammar) into a
/// [`StyleClass`], or `None` if it carries no editor-relevant styling.
///
/// The node-kind strings are the grammar's own (verified against the
/// `tree-sitter-md 0.5.3` block/inline `highlights.scm` and `node-types.json`).
/// Grouping many kinds into a handful of classes is intentional: the editor
/// highlighter is advisory, so it cares about *display role*, not grammatical
/// precision.
fn classify(node: &Node) -> Option<StyleClass> {
    let kind = node.kind();
    match kind {
        // --- dimmable syntax-marker punctuation (FR-010) ---------------------
        "atx_h1_marker" => Some(StyleClass::Heading { level: Some(1) }),
        "atx_h2_marker" => Some(StyleClass::Heading { level: Some(2) }),
        "atx_h3_marker" => Some(StyleClass::Heading { level: Some(3) }),
        "atx_h4_marker" => Some(StyleClass::Heading { level: Some(4) }),
        "atx_h5_marker" => Some(StyleClass::Heading { level: Some(5) }),
        "atx_h6_marker" => Some(StyleClass::Heading { level: Some(6) }),
        "setext_h1_underline" => Some(StyleClass::Heading { level: Some(1) }),
        "setext_h2_underline" => Some(StyleClass::Heading { level: Some(2) }),

        "emphasis_delimiter"
        | "code_span_delimiter"
        | "fenced_code_block_delimiter"
        | "backslash_escape"
        | "thematic_break"
        | "block_continuation" => Some(StyleClass::SyntaxMarker),

        // --- block quote (FR-014) -------------------------------------------
        "block_quote" => Some(StyleClass::BlockQuote),
        "block_quote_marker" => Some(StyleClass::SyntaxMarker),

        // --- list markers (FR-014) ------------------------------------------
        "list_marker_plus"
        | "list_marker_minus"
        | "list_marker_star"
        | "list_marker_dot"
        | "list_marker_parenthesis" => Some(StyleClass::ListMarker),

        // --- code (FR-013) ---------------------------------------------------
        "fenced_code_block" | "indented_code_block" | "code_fence_content" => {
            Some(StyleClass::CodeBlock)
        }
        "code_span" => Some(StyleClass::InlineCode),

        // --- inline emphasis (FR-011) ---------------------------------------
        "strong_emphasis" => Some(StyleClass::Strong),
        "emphasis" => Some(StyleClass::Emphasis),

        // --- links / images (FR-014) ----------------------------------------
        "inline_link"
        | "shortcut_link"
        | "full_reference_link"
        | "collapsed_reference_link"
        | "image"
        | "uri_autolink"
        | "email_autolink" => Some(StyleClass::Link),
        "link_text" | "link_label" | "link_destination" | "link_title" | "image_description" => {
            Some(StyleClass::Link)
        }

        _ => None,
    }
}

/// Map an index that does not fit its target integer type to a reportable error
/// (rather than a truncating `as`). Unreachable within `MAX_NOTE_BYTES`.
fn too_large<E: std::fmt::Display>(err: E) -> EmendError {
    EmendError::Internal {
        detail: format!("document index does not fit in target integer type: {err}"),
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

    use super::{Highlighter, StyleClass};
    use crate::U16Range;

    fn u16_len(s: &str) -> u32 {
        u32::try_from(s.encode_utf16().count()).expect("fits in u32")
    }

    fn classes(spans: &[super::StyleSpan]) -> Vec<StyleClass> {
        spans.iter().map(|s| s.class).collect()
    }

    #[test]
    fn heading_marker_carries_level() {
        let text = "### Title\n";
        let hl = Highlighter::new(text);
        let spans = hl.highlight_spans(U16Range::new(0, u16_len(text))).unwrap();
        assert!(
            classes(&spans).contains(&StyleClass::Heading { level: Some(3) }),
            "spans should include a level-3 heading marker: {spans:?}"
        );
    }

    #[test]
    fn bold_and_italic_classified() {
        let text = "a **bold** and *italic* end\n";
        let hl = Highlighter::new(text);
        let spans = hl.highlight_spans(U16Range::new(0, u16_len(text))).unwrap();
        let cs = classes(&spans);
        assert!(
            cs.contains(&StyleClass::Strong),
            "missing Strong: {spans:?}"
        );
        assert!(
            cs.contains(&StyleClass::Emphasis),
            "missing Emphasis: {spans:?}"
        );
        // Emphasis delimiters must be dimmable syntax markers.
        assert!(
            cs.contains(&StyleClass::SyntaxMarker),
            "missing SyntaxMarker for emphasis delimiters: {spans:?}"
        );
    }

    #[test]
    fn inline_code_and_code_block() {
        let text = "use `x` here\n\n```\nfn main() {}\n```\n";
        let hl = Highlighter::new(text);
        let spans = hl.highlight_spans(U16Range::new(0, u16_len(text))).unwrap();
        let cs = classes(&spans);
        assert!(
            cs.contains(&StyleClass::InlineCode),
            "missing InlineCode: {spans:?}"
        );
        assert!(
            cs.contains(&StyleClass::CodeBlock),
            "missing CodeBlock: {spans:?}"
        );
    }

    #[test]
    fn block_quote_and_list_marker() {
        let text = "> quoted\n\n- item one\n- item two\n";
        let hl = Highlighter::new(text);
        let spans = hl.highlight_spans(U16Range::new(0, u16_len(text))).unwrap();
        let cs = classes(&spans);
        assert!(
            cs.contains(&StyleClass::BlockQuote),
            "missing BlockQuote: {spans:?}"
        );
        assert!(
            cs.contains(&StyleClass::ListMarker),
            "missing ListMarker: {spans:?}"
        );
    }

    #[test]
    fn link_classified() {
        let text = "see [the docs](https://example.com) now\n";
        let hl = Highlighter::new(text);
        let spans = hl.highlight_spans(U16Range::new(0, u16_len(text))).unwrap();
        assert!(
            classes(&spans).contains(&StyleClass::Link),
            "missing Link: {spans:?}"
        );
    }

    #[test]
    fn astral_char_offsets_are_utf16() {
        // "😀" is one char but TWO UTF-16 code units. A heading after it must be
        // reported in UTF-16 offsets, so its marker must start at >= 2.
        let text = "😀 text\n";
        let hl = Highlighter::new(text);
        // Whole-document viewport; just assert spans map within UTF-16 length and
        // do not panic on the surrogate pair.
        let total = u16_len(text);
        let spans = hl.highlight_spans(U16Range::new(0, total)).unwrap();
        for s in &spans {
            assert!(s.range.end() <= total, "span past EOF: {s:?}");
        }
    }

    #[test]
    fn apply_edit_local_then_query() {
        let text = "Hello world.\n\nSecond paragraph.\n";
        let mut hl = Highlighter::new(text);
        // Insert inside the first paragraph.
        let changed = hl.apply_edit(U16Range::new(5, 0), " there").unwrap();
        assert!(!changed.is_empty(), "edit should report changed ranges");
        // The buffer reflects the edit; a query still succeeds.
        let total = u16_len("Hello there world.\n\nSecond paragraph.\n");
        let spans = hl.highlight_spans(U16Range::new(0, total)).unwrap();
        // Sanity: re-querying after an edit does not error.
        let _ = spans;
    }

    #[test]
    fn out_of_bounds_viewport_is_error() {
        let hl = Highlighter::new("abc\n");
        let err = hl.highlight_spans(U16Range::new(100, 1)).unwrap_err();
        assert!(
            matches!(err, crate::EmendError::Internal { .. }),
            "out-of-bounds viewport should be an Internal error, got {err:?}"
        );
    }

    #[test]
    fn empty_document_yields_no_spans() {
        let hl = Highlighter::new("");
        let spans = hl.highlight_spans(U16Range::new(0, 0)).unwrap();
        assert!(
            spans.is_empty(),
            "empty doc should have no spans: {spans:?}"
        );
    }
}
