//! Markdown to `ProseMirror` JSON conversion.
//!
//! Converts `CommonMark` markdown to `ProseMirror` JSON format and vice versa.
//! Uses `comrak` for parsing and serialization.
//!
//! # `ProseMirror` Schema
//!
//! This module generates JSON compatible with the `prosemirror-markdown` schema:
//!
//! ## Nodes
//! - `doc`: Root document containing blocks
//! - `paragraph`: Block containing inline content
//! - `heading`: Heading with `level` attribute (1-6)
//! - `blockquote`: Block quote containing blocks
//! - `code_block`: Code block with optional `params` attribute
//! - `horizontal_rule`: Thematic break
//! - `bullet_list`: Unordered list with `tight` attribute
//! - `ordered_list`: Ordered list with `order` and `tight` attributes
//! - `list_item`: List item containing blocks
//! - `task_list`: Task list (GFM `- [ ]`/`- [x]` items) with `tight` attribute; ordered task lists also carry an `order` attribute
//! - `task_list_item`: Task list item with `checked` attribute
//! - `image`: Inline image with `src`, `alt`, `title` attributes
//! - `hard_break`: Hard line break
//! - `text`: Text content
//!
//! ## Marks
//! - `em`: Emphasis (italic)
//! - `strong`: Strong emphasis (bold)
//! - `code`: Inline code
//! - `link`: Hyperlink with `href` and `title` attributes
//! - `strikethrough`: Strikethrough text (GFM `~~text~~`)

use comrak::nodes::{AstNode, ListType, NodeValue};
use comrak::{Arena, Options, format_commonmark, parse_document};
use serde_json::{Value, json};

/// Error type for markdown conversion.
#[derive(Debug)]
pub enum ConversionError {
    /// The JSON structure is missing a required `type` field.
    MissingType,
    /// The JSON contains an unknown node type.
    UnknownType(String),
    /// A node appears where its kind cannot legally go (e.g. an inline
    /// node as a direct child of `doc`, a block node inside a
    /// `paragraph`). Converting it would build an ill-formed comrak AST
    /// that panics the formatter in debug builds and emits invalid
    /// markdown in release builds (#1043).
    MisplacedNode {
        /// The parent node type the child was found under.
        parent: String,
        /// The misplaced child node type.
        child: String,
    },
    /// Failed to format markdown output.
    FormatError(std::fmt::Error),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingType => write!(f, "JSON node missing 'type' field"),
            Self::UnknownType(t) => write!(f, "Unknown node type: {t}"),
            Self::MisplacedNode { parent, child } => {
                write!(f, "Misplaced node: {child} cannot appear inside {parent}")
            }
            Self::FormatError(e) => write!(f, "Markdown format error: {e}"),
        }
    }
}

impl std::error::Error for ConversionError {}

impl From<std::fmt::Error> for ConversionError {
    fn from(e: std::fmt::Error) -> Self {
        Self::FormatError(e)
    }
}

/// Get comrak options for `CommonMark` parsing.
fn commonmark_options() -> Options<'static> {
    let mut options = Options::default();
    // Enable GFM extensions that map to ProseMirror marks/nodes
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    options.extension.table = true;
    options.extension.footnotes = true;
    options.parse.smart = false; // Don't convert quotes/dashes
    options.render.r#unsafe = true; // Allow HTML output (needed for sized images)
    options
}

/// Convert markdown text to `ProseMirror` JSON document.
///
/// # Arguments
///
/// * `markdown` - The markdown text to convert.
///
/// # Returns
///
/// A `serde_json::Value` representing the `ProseMirror` document.
///
/// # Example
///
/// ```ignore
/// let markdown = "# Hello\n\nThis is **bold** text.";
/// let doc = markdown_to_prosemirror(markdown);
/// ```
#[must_use]
pub fn markdown_to_prosemirror(markdown: &str) -> Value {
    markdown_to_prosemirror_inner(markdown, true)
}

/// The conversion itself. `recover_orphans` is false for the synthesis
/// probes `synthesize_footnote_definition` parses: a probe must never
/// run recovery again, so even a block comrak does not recognize as a
/// definition terminates with no node instead of recursing (#1565,
/// Codex P1 on the first cut).
fn markdown_to_prosemirror_inner(markdown: &str, recover_orphans: bool) -> Value {
    let arena = Arena::new();
    let options = commonmark_options();
    // #1423: comrak (per CommonMark) cannot parse a link destination
    // that contains a line ending, even in <...> angle form, so a
    // newline destination never becomes a link at all — the text is
    // left literal and the href is lost on reopen. Protect newlines
    // inside angle-form destinations with a collision-safe token
    // before parsing (the parse-side mirror of the save direction's
    // placeholder substitution in `substitute_spaced_destinations`)
    // and restore them in the converted href/src attrs afterwards.
    let (protected, newline_token) = protect_newline_destinations(markdown);
    // #1424: comrak's table extension splits rows on unescaped pipes
    // before inline parsing, so a pipe inside a code span in a table
    // cell ends the cell early. Escape those pipes in table rows
    // before parsing; comrak then forms the code mark with the literal
    // pipe, and its renderer already emits the same `\|` spelling for
    // code spans in table cells on save, so the directions are
    // symmetric with no save-side mirror needed.
    let protected = escape_table_code_span_pipes(&protected);
    let root = parse_document(&arena, &protected, &options);
    let mut doc = convert_node(root, &[]);
    if let Some(token) = &newline_token {
        restore_attr_newlines(&mut doc, &token);
    }
    // #1565: comrak never emits a footnote definition that nothing
    // references, so the conversion above cannot see one -- and a save
    // then deletes it permanently (a definitions-only document saved
    // as an empty file). Recover unreferenced definitions from the
    // source and append them as the same PM nodes the referenced path
    // produces.
    if recover_orphans {
        preserve_unreferenced_footnote_definitions(&mut doc, &protected, newline_token.as_deref());
    }
    doc
}

/// Fence tracking for the source scans below: a fence opens with a
/// run of >= 3 backticks or tildes and closes only with a run of the
/// SAME character at least as long (CommonMark) -- a three-backtick
/// line inside a four-backtick block is content, not a close (Codex
/// P2 on the first #1565 cut, which toggled on any fence-shaped line).
#[derive(Default)]
struct FenceTracker {
    open: Option<(char, usize)>,
}

/// The line to feed the FenceTracker: a fence delimiter may sit
/// up to three columns in from its container's content edge
/// (CommonMark), but strip_container_prefixes leaves those columns
/// in place (`>   ```` arrives as two leading spaces), and
/// FenceTracker::feed anchors the run at the first character -- so
/// an indented opening fence inside a blockquote was missed and
/// the "code" interior was scanned as live text (Codex P2 on
/// #1565's twenty-fourth cut). Removing up to three leading spaces
/// here keeps the four-column distinction: a line still indented
/// after this (or tab-led, which is four columns at column 0)
/// reaches the tracker space-led and opens no fence, exactly like
/// indented code.
fn fence_feed_line(line: &str) -> &str {
    let spaces = line.chars().take_while(|&c| c == ' ').count().min(3);
    &line[spaces..]
}

impl FenceTracker {
    /// Feed one line (already stripped of up to three leading
    /// spaces). Returns true when the line is a fence delimiter.
    fn feed(&mut self, stripped: &str) -> bool {
        let ch = match stripped.chars().next() {
            Some(c @ ('`' | '~')) => c,
            _ => return false,
        };
        let run = stripped.chars().take_while(|&c| c == ch).count();
        if run < 3 {
            return false;
        }
        if self.open.is_none() && ch == '`' && stripped[run..].contains('`') {
            // A backtick fence's info string cannot contain a
            // backtick (CommonMark), so a line like ```a`b opens
            // no fence at all (Codex P2 on #1565's eighth cut).
            return false;
        }
        match self.open {
            None => {
                self.open = Some((ch, run));
                true
            }
            Some((open_ch, open_run)) => {
                if ch == open_ch && run >= open_run && stripped[run..].trim().is_empty() {
                    self.open = None;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn in_fence(&self) -> bool {
        self.open.is_some()
    }

    /// Forget any open fence: a fence opened inside a container
    /// ends implicitly when the container ends, with no closing
    /// delimiter line (Codex P2 on #1565's tenth cut).
    fn reset(&mut self) {
        self.open = None;
    }
}

/// HTML-block tracking for the source scans below (Codex P2 on
/// #1565's second cut): comrak parses a raw HTML block -- a comment,
/// a script/pre/style/textarea element, a block-level tag, or a lone
/// complete tag -- as literal content until its CommonMark end
/// condition (the closing marker for comments/CDATA/PIs/tag-bodies,
/// a blank line for plain block tags). A definition-shaped line in
/// that interior is literal text, exactly as in a fence; without this
/// tracking the pre-scan synthesized a live footnote out of a
/// comment's interior and a save then moved it outside the comment.
#[derive(Default)]
struct HtmlBlockTracker {
    /// What ends the open block: a literal closing marker, or a
    /// blank line for block-tag (type 6/7) blocks.
    end: Option<HtmlBlockEnd>,
}

#[derive(Clone, Copy, PartialEq)]
enum HtmlBlockEnd {
    Marker(&'static str),
    BlankLine,
}

/// The CommonMark type-6 block-level tag names.
const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// Whether `trimmed` (a whole line, both ends trimmed) is exactly
/// one complete HTML open or closing tag -- CommonMark type 7: a
/// tag name (a letter, then letters/digits/`-`), then for an open
/// tag any number of whitespace-separated attributes (a name, with
/// an optional `=` and an unquoted, single-quoted, or
/// double-quoted value), optional whitespace, an optional `/`,
/// and the final `>`. A closing tag is `</name`, optional
/// whitespace, `>`. Anything else angle-delimited is NOT a tag:
/// an autolink like `<https://example.com>` (the `:` after the
/// name is not whitespace, `/`, or `>`), or a tag-shaped line with
/// junk where an attribute should be. Treating every
/// angle-delimited string as an HTML block opened a
/// blank-line-ended block on the autolink line and swallowed the
/// heading and the valid orphan definition after it (Codex P2 on
/// #1565's seventeenth cut).
fn is_complete_html_tag(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'<' || bytes[bytes.len() - 1] != b'>' {
        return false;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let b = inner.as_bytes();
    let (closing, mut pos) = if b.first() == Some(&b'/') {
        (true, 1)
    } else {
        (false, 0)
    };
    if !b.get(pos).is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let name_start = pos;
    pos += 1;
    while b
        .get(pos)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
    {
        pos += 1;
    }
    // Type 1 names never open a type 7 block with an OPENING tag
    // (they are handled as marker-ended blocks above); a line
    // naming one that reaches here did not match the type-1
    // boundary rule either. The exclusion applies only to opening
    // tags: a complete CLOSING tag of any name opens a type-7
    // block, so `</pre>` makes the following definition-shaped
    // line its literal interior -- rejecting it recovered that
    // line as a live definition (Codex P2 on #1565's
    // twenty-first cut).
    if !closing
        && ["script", "pre", "style", "textarea"]
            .contains(&inner[name_start..pos].to_ascii_lowercase().as_str())
    {
        return false;
    }
    if closing {
        while b.get(pos).is_some_and(|c| c.is_ascii_whitespace()) {
            pos += 1;
        }
        return pos == inner.len();
    }
    loop {
        let ws_start = pos;
        while b.get(pos).is_some_and(|c| c.is_ascii_whitespace()) {
            pos += 1;
        }
        let had_ws = pos > ws_start;
        match b.get(pos) {
            None => return true,
            Some(b'/') => return pos + 1 == inner.len(),
            Some(_) if had_ws => {
                // Attribute name, then an optional `= value`.
                if !b
                    .get(pos)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_' || *c == b':')
                {
                    return false;
                }
                pos += 1;
                while b.get(pos).is_some_and(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b':')
                }) {
                    pos += 1;
                }
                let mut p2 = pos;
                while b.get(p2).is_some_and(|c| c.is_ascii_whitespace()) {
                    p2 += 1;
                }
                if b.get(p2) == Some(&b'=') {
                    p2 += 1;
                    while b.get(p2).is_some_and(|c| c.is_ascii_whitespace()) {
                        p2 += 1;
                    }
                    match b.get(p2) {
                        Some(q @ (b'"' | b'\'')) => {
                            let q = *q;
                            p2 += 1;
                            while b.get(p2).is_some_and(|c| *c != q) {
                                p2 += 1;
                            }
                            if b.get(p2).is_none() {
                                return false;
                            }
                            p2 += 1;
                        }
                        Some(_) => {
                            let vstart = p2;
                            while b.get(p2).is_some_and(|c| {
                                !c.is_ascii_whitespace()
                                    && !matches!(c, b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
                            }) {
                                p2 += 1;
                            }
                            if p2 == vstart {
                                return false;
                            }
                        }
                        None => return false,
                    }
                    pos = p2;
                }
            }
            Some(_) => return false,
        }
    }
}

impl HtmlBlockTracker {
    fn in_html(&self) -> bool {
        self.end.is_some()
    }

    /// Forget any open block: a block opened inside a container
    /// ends implicitly when the container ends, with no closing
    /// line (Codex P2 on #1565's eleventh cut, mirroring
    /// FenceTracker::reset).
    fn reset(&mut self) {
        self.end = None;
    }

    /// Feed one line (already stripped of up to three leading
    /// spaces). Returns true when the line belongs to an HTML block
    /// (its opening line, interior, or closing line).
    /// `may_open_type7` is the caller's paragraph context: a type-7
    /// block (a lone complete inline tag) cannot interrupt a
    /// paragraph, so when the previous line is paragraph content
    /// the caller passes false and a type-7 opening is refused
    /// (Codex P2 on #1565's eighteenth cut). Types 1-6 can
    /// interrupt a paragraph and are always eligible.
    fn feed(&mut self, stripped: &str, may_open_type7: bool) -> bool {
        if let Some(end) = self.end {
            match end {
                HtmlBlockEnd::Marker(marker) => {
                    if stripped.to_ascii_lowercase().contains(marker) {
                        self.end = None;
                    }
                }
                HtmlBlockEnd::BlankLine => {
                    if stripped.trim().is_empty() {
                        self.end = None;
                    }
                }
            }
            return true;
        }
        let Some(rest) = stripped.strip_prefix('<') else {
            return false;
        };
        let lower = stripped.to_ascii_lowercase();
        // Marker-ended blocks (CommonMark types 1-5). The tag-named
        // prefixes require a name boundary: `<prefix>` is not a
        // `<pre>` block (nor `<scripture>` a `<script>`), it is a
        // generic tag block ending at a blank line (Codex P2 on
        // #1565's sixth cut).
        for (prefix, marker) in [
            ("<!--", "-->"),
            ("<![cdata[", "]]>"),
            ("<?", "?>"),
            ("<script", "</script>"),
            ("<pre", "</pre>"),
            ("<style", "</style>"),
            ("<textarea", "</textarea>"),
        ] {
            // The tag-named prefixes follow the CommonMark type-1
            // boundary exactly: whitespace, '>', or end-of-line
            // after the name. Anything else (`<pre/>`, `<prex>`)
            // is a generic tag block ending at a blank line, not a
            // marker-ended block (Codex P2 on #1565's tenth cut).
            let boundary_ok = !prefix[1..].chars().all(|c| c.is_ascii_alphabetic())
                || match lower.as_bytes().get(prefix.len()) {
                    None => true,
                    Some(b) => b.is_ascii_whitespace() || *b == b'>',
                };
            // CDATA is case-sensitive (CommonMark): only the
            // exact uppercase `<![CDATA[` opens a marker-ended
            // block; `<![cdata[` is ordinary text (Codex P2 on
            // #1565's sixteenth cut). Tag names stay
            // case-insensitive.
            let (hay, prefix) = if prefix == "<![cdata[" {
                (stripped, "<![CDATA[")
            } else {
                (lower.as_str(), prefix)
            };
            if boundary_ok && hay.starts_with(prefix) {
                if !hay[prefix.len()..].contains(marker) {
                    self.end = Some(HtmlBlockEnd::Marker(marker));
                }
                return true;
            }
        }
        if rest.starts_with('!')
            && rest[1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
        {
            // Declaration (type 5 covers <![CDATA[ above; a <!X
            // declaration ends at '>').
            if !rest.contains('>') {
                self.end = Some(HtmlBlockEnd::Marker(">"));
            }
            return true;
        }
        // Tag-ended blocks (types 6 and 7): a block-level tag, or a
        // lone complete open/closing tag on the line.
        let tag_body = rest.strip_prefix('/').unwrap_or(rest);
        let tag_name: String = tag_body
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if tag_name.is_empty() {
            return false;
        }
        if HTML_BLOCK_TAGS.contains(&tag_name.as_str())
            && tag_name_boundary_ok(tag_body, tag_name.len())
        {
            self.end = Some(HtmlBlockEnd::BlankLine);
            return true;
        }
        let trimmed = stripped.trim_end();
        if is_complete_html_tag(trimmed) {
            if !may_open_type7 {
                return false;
            }
            self.end = Some(HtmlBlockEnd::BlankLine);
            return true;
        }
        let _ = rest;
        false
    }
}

/// Strip block-container prefixes (blockquote `>`, list markers
/// `-`/`*`/`+`/ordered) from a line, repeatedly, for fence detection:
/// a fence opened inside a list item or blockquote still makes its
/// content literal (Codex P2 on #1565's fourth cut -- `- ```` followed
/// by an indented definition-shaped line parsed as code by comrak,
/// but the scanner missed the opening fence behind the `- ` marker
/// and recovered the interior line as a live definition).
fn strip_container_prefixes(line: &str) -> &str {
    let mut rest = line;
    loop {
        let t = rest.trim_start_matches(' ');
        if let Some(r) = t.strip_prefix('>') {
            rest = r.strip_prefix(' ').unwrap_or(r);
            continue;
        }
        let bytes = t.as_bytes();
        let marker_len = if bytes.len() >= 2
            && matches!(bytes[0], b'-' | b'*' | b'+')
            && (bytes[1] == b' ' || bytes[1] == b'\t')
        {
            Some(1)
        } else {
            let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits >= 1
                && digits <= 9
                && matches!(t.as_bytes().get(digits), Some(b'.') | Some(b')'))
                && matches!(t.as_bytes().get(digits + 1), Some(b' ') | Some(b'\t'))
            {
                Some(digits + 1)
            } else {
                None
            }
        };
        let Some(marker_len) = marker_len else { break };
        rest = strip_marker_padding(&t[marker_len..], marker_len);
    }
    rest
}

/// The content after a list marker, CommonMark-style: one to
/// four columns of padding after the marker are the item's
/// content indent and are stripped with the marker; FIVE or
/// more columns mean the content is an indented code block
/// whose indentation starts after a single padding column, so
/// only that one column is stripped and the remaining
/// indentation survives. Removing all the padding turned
/// `-     ``` ` into a fence opener the document does not have,
/// and the phantom fence then suppressed the valid reference
/// definition after it (Codex P2 on #1565's twenty-first cut).
fn strip_marker_padding(after: &str, marker_end_col: usize) -> &str {
    let mut col = marker_end_col;
    let mut end = 0;
    for c in after.chars() {
        match c {
            ' ' => {
                col += 1;
                end += 1;
            }
            '\t' => {
                col += 4 - (col % 4);
                end += 1;
            }
            _ => break,
        }
    }
    if col - marker_end_col > 4 {
        // Keep all but the first padding character (spaces and
        // tabs are one byte each, so this is a byte offset too).
        &after[after.chars().next().map_or(0, |_| 1)..]
    } else {
        &after[end..]
    }
}

/// The container structure of a recorded container prefix, as
/// a string of marker kinds with all spacing removed: `> ` and
/// `>` are the same blockquote level, so a fence opened under
/// one spelling closes under the other (Codex P2 on #1565's
/// nineteenth cut). Byte-exact prefix matching left such a
/// fence open and suppressed the definitions after it.
fn container_signature(prefix: &str) -> String {
    let mut sig = String::new();
    let mut rest = prefix;
    loop {
        let t = rest.trim_start_matches([' ', '\t']);
        if let Some(r) = t.strip_prefix('>') {
            sig.push('>');
            rest = r;
            continue;
        }
        let bytes = t.as_bytes();
        if bytes.len() >= 2
            && matches!(bytes[0], b'-' | b'*' | b'+')
            && (bytes[1] == b' ' || bytes[1] == b'\t')
        {
            sig.push('*');
            rest = &t[1..];
            continue;
        }
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits >= 1
            && digits <= 9
            && matches!(t.as_bytes().get(digits), Some(b'.') | Some(b')'))
            && matches!(t.as_bytes().get(digits + 1), Some(b' ') | Some(b'\t'))
        {
            sig.push('#');
            rest = &t[digits + 1..];
            continue;
        }
        break;
    }
    sig
}

/// Strip at most `levels` container levels (blockquote `>`,
/// list markers) from a line, mirroring
/// strip_container_prefixes' per-level shapes.
fn strip_container_levels(line: &str, levels: usize) -> &str {
    let mut rest = line;
    for _ in 0..levels {
        let t = rest.trim_start_matches(' ');
        if let Some(r) = t.strip_prefix('>') {
            rest = r.strip_prefix(' ').unwrap_or(r);
            continue;
        }
        let bytes = t.as_bytes();
        let marker_len = if bytes.len() >= 2
            && matches!(bytes[0], b'-' | b'*' | b'+')
            && (bytes[1] == b' ' || bytes[1] == b'\t')
        {
            Some(1)
        } else {
            let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
            if digits >= 1
                && digits <= 9
                && matches!(t.as_bytes().get(digits), Some(b'.') | Some(b')'))
                && matches!(t.as_bytes().get(digits + 1), Some(b' ') | Some(b'\t'))
            {
                Some(digits + 1)
            } else {
                None
            }
        };
        let Some(marker_len) = marker_len else { break };
        rest = strip_marker_padding(&t[marker_len..], marker_len);
    }
    rest
}

/// Whether `stripped` (a line stripped of up to three leading
/// spaces) carries the recorded container `prefix`, matched
/// STRUCTURALLY rather than byte-for-byte: the line's own
/// leading container levels (as many as the recorded prefix
/// has) must have the same signature as the recorded prefix.
fn carries_container_prefix(stripped: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return false;
    }
    let levels = container_signature(prefix).chars().count();
    if levels == 0 {
        return false;
    }
    let own = &stripped[..stripped.len() - strip_container_levels(stripped, levels).len()];
    !own.is_empty() && container_signature(own) == container_signature(prefix)
}

/// A footnote definition block found by the source pre-scan (#1565):
/// its label and the block's verbatim source (the `[^label]:` line
/// plus any indented continuation lines, trailing blank lines cut).
struct FootnoteDefinitionBlock {
    label: String,
    source: String,
    /// Zero-based line index of the definition's opening line, for
    /// interleaving with other source segments in document order.
    line: usize,
}

/// The label of a footnote definition starting on `line` (already
/// stripped of up to three leading spaces), or `None` when the line
/// does not start one. A definition opens as `[^label]:`.
fn footnote_definition_label(line: &str) -> Option<String> {
    let rest = line.strip_prefix("[^")?;
    // The label ends at the FIRST ']' and a ':' must follow it
    // immediately. Searching for "]:" instead accepted `a]b` as the
    // label of `[^a]b]: note` -- a definition comrak does not
    // recognize, whose probe then looped recovery forever (Codex P1).
    let close = rest.find(']')?;
    if !rest[close + 1..].starts_with(':') {
        return None;
    }
    let label = &rest[..close];
    if label.is_empty() || label.contains('[') {
        return None;
    }
    Some(label.to_string())
}

/// A thematic break: at least three of the same `-`, `*`, or `_`,
/// spaces and tabs aside (CommonMark).
fn is_thematic_break(stripped: &str) -> bool {
    let compact: String = stripped
        .chars()
        .filter(|c| *c != ' ' && *c != '\t')
        .collect();
    let first = compact.chars().next();
    compact.len() >= 3
        && matches!(first, Some('-' | '*' | '_'))
        && compact.chars().all(|c| Some(c) == first)
}

/// A setext heading underline: only `=`s (level 1) or `-`s (level 2).
fn is_setext_underline(stripped: &str) -> bool {
    let t = stripped.trim();
    (!t.is_empty() && t.chars().all(|c| c == '=')) || (!t.is_empty() && t.chars().all(|c| c == '-'))
}

/// Whether a line starts a new block and so cannot be a lazy
/// paragraph continuation of a footnote definition: another
/// definition, an ATX heading, a fence, a blockquote, a thematic
/// break, an HTML block opener, or a list/blockquote container line.
/// Whether a line is an ATX heading: 1-6 `#`s followed by a
/// space, a tab, or end-of-line (CommonMark). `#hashtag` is
/// paragraph text, not a heading (Codex P2 on #1565's tenth cut).
fn is_atx_heading(stripped: &str) -> bool {
    let hashes = stripped.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes)
        && matches!(
            stripped.as_bytes().get(hashes),
            None | Some(b' ') | Some(b'\t')
        )
}

/// Whether the character after a block-level tag name is a valid
/// name boundary: whitespace, `/`, `>`, or end-of-line (CommonMark
/// type 6). `<div=foo>` has no boundary after `div`, so it is
/// literal paragraph text, not an HTML block (Codex P2 on
/// #1565's sixteenth cut).
fn tag_name_boundary_ok(tag_body: &str, name_len: usize) -> bool {
    matches!(
        tag_body.as_bytes().get(name_len),
        None | Some(b' ') | Some(b'\t') | Some(b'>') | Some(b'/')
    )
}

/// Whether a line opens an HTML block that can INTERRUPT a
/// paragraph (CommonMark types 1-6): a comment, a CDATA section,
/// a processing instruction, a declaration, a script/pre/style/
/// textarea element, or a block-level tag (opening or closing).
/// A lone inline tag (`<em>`, `<a>`) is type 7, which cannot
/// interrupt a paragraph: as a footnote definition's next line it
/// is a lazy continuation of the definition's paragraph, not a
/// new block (Codex P2 on #1565's eleventh cut).
fn is_html_block_start(stripped: &str) -> bool {
    let Some(rest) = stripped.strip_prefix('<') else {
        return false;
    };
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with("<!--") || stripped.starts_with("<![CDATA[") || lower.starts_with("<?") {
        return true;
    }
    if rest.starts_with('!')
        && rest[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
    {
        return true;
    }
    // Type 1 elements, with the tracker's name boundary
    // (whitespace, '>', or end-of-line after the name).
    for name in ["script", "pre", "style", "textarea"] {
        if lower[1..].starts_with(name)
            && matches!(
                lower.as_bytes().get(1 + name.len()),
                None | Some(b' ') | Some(b'\t') | Some(b'>')
            )
        {
            return true;
        }
    }
    let tag_body = rest.strip_prefix('/').unwrap_or(rest);
    let tag_name: String = tag_body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    HTML_BLOCK_TAGS.contains(&tag_name.as_str()) && tag_name_boundary_ok(tag_body, tag_name.len())
}

/// Whether a stripped line is an ordered list marker whose
/// start number cannot interrupt a paragraph: CommonMark lets
/// only a list starting at 1 do that, so `2. text` after a
/// paragraph line is its lazy continuation, not a new block
/// (Codex P2 on #1565's thirteenth cut).
fn is_non_interrupting_ordered_marker(stripped: &str) -> bool {
    let digits = stripped.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return false;
    }
    if !matches!(stripped.as_bytes().get(digits), Some(b'.') | Some(b')')) {
        return false;
    }
    if !matches!(
        stripped.as_bytes().get(digits + 1),
        Some(b' ') | Some(b'\t')
    ) {
        return false;
    }
    stripped[..digits].parse::<u64>().ok() != Some(1)
}

/// Whether a stripped line opens a code fence: a run of >= 3
/// backticks or tildes whose info string is valid -- a backtick
/// fence's info string cannot contain a backtick (CommonMark), so
/// a line like ```a`b is paragraph text, not a fence (Codex P2 on
/// #1565's fifteenth cut). Mirrors FenceTracker's opening rule.
fn is_fence_opener(stripped: &str) -> bool {
    let Some(ch) = stripped.chars().next() else {
        return false;
    };
    if ch != '`' && ch != '~' {
        return false;
    }
    let run = stripped.chars().take_while(|&c| c == ch).count();
    run >= 3 && !(ch == '`' && stripped[run..].contains('`'))
}

/// The fence a line opens, as (fence char, run length), when the
/// line is a fence opener.
fn fence_opener_run(stripped: &str) -> Option<(char, usize)> {
    if !is_fence_opener(stripped) {
        return None;
    }
    let ch = stripped.chars().next()?;
    Some((ch, stripped.chars().take_while(|&c| c == ch).count()))
}

/// Whether a line closes a fence opened with the given char and run
/// length: a run of the same char at least as long, followed by
/// nothing but spaces and tabs (a closing fence carries no info
/// string, CommonMark). The fence chars are ASCII, so the run
/// length is also the byte count.
fn is_fence_closer(stripped: &str, ch: char, run: usize) -> bool {
    let count = stripped.chars().take_while(|&c| c == ch).count();
    count >= run && stripped[count..].trim().is_empty()
}

/// The indentation of a line in columns: a space is one column
/// and a tab advances to the next multiple of four, so the two
/// spellings ` \t` and `    ` are the same four columns
/// (CommonMark) -- a literal prefix check accepts neither the
/// mixed spelling nor other space/tab combinations (Codex P2 on
/// #1565's fifteenth cut).
fn indentation_columns(line: &str) -> usize {
    let mut col = 0;
    for c in line.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col += 4 - (col % 4),
            _ => break,
        }
    }
    col
}

/// Whether a line is blank in the Markdown sense: nothing but
/// spaces and tabs. Unicode-aware trimming also treats a line of
/// nonbreaking spaces as blank, but CommonMark does not -- such a
/// line is paragraph text, and resetting block state on it
/// promoted the following definition-shaped line out of its
/// paragraph (Codex P2 on #1565's twenty-ninth cut).
fn is_blank_line(line: &str) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

/// Remove exactly `columns` leading indentation columns, accounting for
/// tabs advancing to the next multiple of four. Used to inspect content
/// inside a footnote continuation without erasing additional indentation
/// that may make the line an indented code block.
fn strip_indentation_columns(line: &str, columns: usize) -> &str {
    let mut col = 0;
    for (index, c) in line.char_indices() {
        let next = match c {
            ' ' => col + 1,
            '\t' => col + 4 - (col % 4),
            _ => return line,
        };
        if next >= columns {
            return &line[index + c.len_utf8()..];
        }
        col = next;
    }
    line
}

/// Whether a stripped line carries a list marker as its outermost
/// container prefix (a bullet, or an ordered marker): such a line
/// starts a new list item. The marker shapes mirror
/// strip_container_prefixes' first level.
fn starts_with_list_marker(stripped: &str) -> bool {
    let bytes = stripped.as_bytes();
    if bytes.len() >= 2
        && matches!(bytes[0], b'-' | b'*' | b'+')
        && (bytes[1] == b' ' || bytes[1] == b'\t')
    {
        return true;
    }
    let digits = stripped.chars().take_while(|c| c.is_ascii_digit()).count();
    digits >= 1
        && digits <= 9
        && matches!(stripped.as_bytes().get(digits), Some(b'.') | Some(b')'))
        && matches!(
            stripped.as_bytes().get(digits + 1),
            Some(b' ') | Some(b'\t')
        )
}

fn starts_new_block(stripped: &str) -> bool {
    // A link reference definition is deliberately NOT here: it
    // cannot interrupt a paragraph, so a reference-shaped line
    // after a footnote definition's first line is a lazy
    // continuation of that paragraph (Codex P2 on #1565's
    // twelfth cut).
    footnote_definition_label(stripped).is_some()
        || is_atx_heading(stripped)
        || stripped.starts_with('>')
        || is_fence_opener(stripped)
        || is_thematic_break(stripped)
        || (strip_container_prefixes(stripped) != stripped
            && !is_non_interrupting_ordered_marker(stripped))
        || is_html_block_start(stripped)
}

/// Scan `markdown` for top-level footnote definition blocks (#1565).
/// Fence-aware like `protect_newline_destinations`: a definition-shaped
/// line inside a fenced code block is literal text, not a definition.
/// A block runs from its `[^label]:` line through the following blank
/// or indented (>= 4 spaces / tab) lines -- the CommonMark continuation
/// rule for footnote definitions. The harness is shape-generic so the
/// #1004 link-reference-definition fix can share it later.
fn scan_footnote_definition_blocks(markdown: &str) -> Vec<FootnoteDefinitionBlock> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut blocks = Vec::new();
    let mut fence = FenceTracker::default();
    let mut fence_prefix = String::new();
    let mut html_prefix = String::new();
    let mut html = HtmlBlockTracker::default();
    // Block-start eligibility, mirroring
    // collect_link_reference_definitions: a type-7 HTML opening
    // cannot interrupt a paragraph, so the tracker may only open
    // one where a block can start (Codex P2 on #1565's eighteenth
    // cut: `Body` / `<em>` / `# Heading` / `[^real]: note` -- the
    // tracker opened at `<em>` and swallowed the heading and the
    // definition as HTML interior). Blank lines, block starts,
    // fences, HTML blocks and definitions leave the next line
    // eligible; paragraph text does not.
    let mut at_block_start = true;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // Indentation in COLUMNS, like the reference collector:
        // a tab advances to the next multiple of 4, so a
        // tab-indented line is indented code (indent 4), not a
        // paragraph line at indent 0. Counting spaces only left
        // `\tcode` looking like paragraph text, which refused
        // the HTML block after it and recovered the definition-
        // shaped line inside it as a footnote (Codex P2 on
        // #1565's twentieth cut).
        let indent = indentation_columns(line);
        let stripped = line.trim_start_matches(' ');
        // An already-open HTML block sees every line, however
        // indented: an indented closing marker still closes it,
        // and gating this feed on indent left the tracker open
        // forever, suppressing every later definition (Codex P2
        // on #1565's seventh cut). The indent limit applies only
        // to recognizing an OPENING line.
        // Container prefixes are stripped for an HTML block's
        // OPENING line too: a block opened inside a list item
        // (`- <!--`) still makes its interior literal, and feeding
        // only the unstripped line missed the opening behind the
        // marker (Codex P2 on #1565's eighth cut).
        let container_stripped = strip_container_prefixes(stripped);
        // While an HTML block is open, feed it lines relative to
        // the container it opened in: a blank line inside a
        // blockquote is just `>` once spaces are stripped, and
        // feeding that literal line kept the tracker open past the
        // block's end, suppressing what follows it (Codex P2 on
        // #1565's ninth cut). Mirrors fence_prefix below.
        // (The recorded prefix's exact bytes vary -- `> ` vs `>`
        // on a blank quote line -- so relativity is computed by
        // re-stripping this line's own container prefixes.)
        // An HTML block opened inside a blockquote ends implicitly
        // when the container ends, exactly like a fence (tenth
        // cut): a following line carrying no container prefix at
        // all is outside the quote, so a marker-ended block (a
        // comment) is over even without its closing marker (Codex
        // P2 on #1565's eleventh cut: `> <!--` / `> hidden` /
        // blank / `[^real]: note` -- the tracker stayed open and
        // treated the real definition as comment interior).
        // (The blockquote clause above is the tenth/eleventh cut;
        // the list clause is the fifteenth: a non-blank line at
        // column zero carrying no container prefix is outside the
        // item, exactly as for a list-scoped fence.)
        if html.in_html()
            && !html_prefix.is_empty()
            && !stripped.starts_with(&html_prefix)
            && strip_container_prefixes(stripped) == stripped
            && (html_prefix.contains('>') || (indent == 0 && !stripped.is_empty()))
        {
            html.reset();
            html_prefix.clear();
        }
        let html_line = if html.in_html() && !html_prefix.is_empty() {
            container_stripped
        } else {
            stripped
        };
        let was_in_html = html.in_html();
        if !fence.in_fence()
            && (indent <= 3 || html.in_html())
            && (html.feed(html_line, at_block_start)
                || html.feed(container_stripped, at_block_start))
        {
            if !was_in_html && html.in_html() {
                // Opened on this line (via the container-stripped
                // form when a container prefix is present): remember
                // the prefix for the block's interior lines.
                html_prefix = stripped[..stripped.len() - container_stripped.len()].to_string();
            }
            if !html.in_html() {
                html_prefix.clear();
            }
            at_block_start = true;
            i += 1;
            continue;
        }
        // Container prefixes are stripped to find a fence's OPENING
        // line, and the opening line's prefix is remembered: while
        // the fence is open, only lines carrying THAT prefix are
        // de-prefixed (a fence genuinely inside a blockquote closes
        // on a prefixed delimiter), while a container-shaped line
        // inside a top-level fence stays literal (Codex P2 on
        // #1565's fifth and sixth cuts).
        // A fence opened inside a blockquote ends implicitly
        // when the container ends: a following line carrying no
        // container prefix at all is outside the quote, so the
        // fence is over even without a closing delimiter (Codex
        // P2 on #1565's tenth cut: `> ``` ` / `> code` / blank /
        // `[^real]: note` -- the scanner kept the fence open and
        // skipped the real definition).
        // A fence opened inside a list item ends the same way
        // (fourteenth cut): a non-blank line at column zero carrying
        // no container prefix is outside the item, so the fence is
        // over even without a closing delimiter (`- ``` ` / `  code`
        // / blank / `[^real]: note` -- the scanner kept the
        // list-scoped fence open and skipped the real definition).
        // A fence opened inside a list item ends when a SIBLING
        // item starts: the sibling's marker line sits at column
        // zero and carries a marker just like the opening line,
        // so the container-end check below mistakes it for a
        // continuation of the same item and the fence swallows
        // the sibling's content (Codex P2 on #1565's twenty-ninth
        // cut: `- ``` ` / `- [url]: /target` -- the scan stayed
        // inside the fence and the sibling's definition was
        // never seen). A sibling is a new container, not a
        // continuation.
        if fence.in_fence()
            && !fence_prefix.is_empty()
            && indent == 0
            && starts_with_list_marker(&fence_prefix)
            && starts_with_list_marker(stripped)
        {
            fence.reset();
            fence_prefix.clear();
        }
        if fence.in_fence()
            && !fence_prefix.is_empty()
            && !carries_container_prefix(stripped, &fence_prefix)
            && strip_container_prefixes(stripped) == stripped
            && (fence_prefix.contains('>') || (indent == 0 && !stripped.is_empty()))
        {
            fence.reset();
            fence_prefix.clear();
        }
        let fence_line = if fence.in_fence() {
            if carries_container_prefix(stripped, &fence_prefix) {
                strip_container_levels(stripped, container_signature(&fence_prefix).chars().count())
            } else {
                stripped
            }
        } else {
            container_stripped
        };
        let was_in_fence = fence.in_fence();
        if indent <= 3 && fence.feed(fence_feed_line(fence_line)) {
            if was_in_fence {
                fence_prefix.clear();
            } else {
                fence_prefix = stripped[..stripped.len() - container_stripped.len()].to_string();
            }
            // A fence delimiter is a block boundary: after an
            // (empty) fenced block the next line is block-eligible
            // again, so a type-7 HTML block may open there -- the
            // reference collector already reset this; without it
            // `Body` / ``` ``` ``` / `<em>` / `[^fake]: ...` kept
            // the paragraph's ineligibility, refused the tag, and
            // recovered the fake definition (Codex P2 on #1565's
            // nineteenth cut).
            at_block_start = true;
            i += 1;
            continue;
        }
        if fence.in_fence() || html.in_html() {
            at_block_start = true;
            i += 1;
            continue;
        }
        if indent > 3 {
            // An indented line is indented code at a block start,
            // but indented code cannot interrupt a paragraph: when
            // the previous line was paragraph text this line is
            // its continuation and the paragraph state survives --
            // resetting eligibility here let a following type-7
            // tag open an HTML block mid-paragraph and swallow
            // the heading and the genuine definition after it
            // (Codex P2 on #1565's twenty-second cut). At a block
            // start the state is already eligible, so either way
            // it is left untouched.
            i += 1;
            continue;
        }
        if is_blank_line(stripped) {
            at_block_start = true;
            i += 1;
            continue;
        }
        let Some(label) = footnote_definition_label(stripped) else {
            // A plain line is paragraph text (next line not
            // block-eligible) unless it starts a block itself (a
            // heading, a list item, a thematic break, ...).
            // A setext underline also ends the paragraph when one
            // precedes it, exactly as the reference collector
            // handles it: `Body` / `===` is a heading, so a type-7
            // HTML block may open on the next line and swallow a
            // definition-shaped line as its literal interior;
            // without this the underline left the paragraph open,
            // the tag was refused, and the fake definition was
            // recovered (Codex P2 on #1565's twentieth cut). A
            // lone underline at a block start is paragraph text
            // itself, so the gate is !at_block_start.
            let setext_ends_paragraph = is_setext_underline(stripped) && !at_block_start;
            // A container line's eligibility follows its CONTENT,
            // exactly as the reference collector's fourteenth cut:
            // `- Body` is paragraph text inside the item, so the
            // next line is its continuation -- a type-7 HTML tag
            // there (`  <em>`) cannot open a block, and treating
            // the marker line itself as a block start let the HTML
            // tracker open at the tag and swallow the heading and
            // the genuine definition after it (Codex P2 on #1565's
            // twenty-third cut). Only item/quote content that
            // itself ends or starts a block (blank, heading, break,
            // setext, a new block) leaves the next line eligible.
            let container_eligible = if strip_container_prefixes(stripped) != stripped {
                let c = container_stripped;
                Some(
                    c.trim().is_empty()
                        || is_atx_heading(c)
                        || is_thematic_break(c)
                        || (is_setext_underline(c) && !at_block_start)
                        || starts_new_block(c),
                )
            } else {
                None
            };
            at_block_start = container_eligible.unwrap_or_else(|| starts_new_block(stripped))
                || setext_ends_paragraph;
            i += 1;
            continue;
        };
        let mut source_lines: Vec<&str> = vec![line];
        let mut last_content = 0;
        // Lazy continuation eligibility: the previous line carried
        // paragraph content. The opening line counts only when its
        // body is paragraph text: a body that itself starts a
        // block (`[^a]: # Heading`) cannot accept an unindented
        // lazy continuation, so the next line starts fresh at the
        // top level -- treating any nonempty body as content let
        // the inner loop swallow a following HTML block's opening
        // line and recover the definition-shaped line inside it
        // (Codex P2 on #1565's twenty-first cut).
        let opening_body = stripped[label.len() + 4..].trim_start_matches(' ');
        let mut prev_is_content =
            !opening_body.trim().is_empty() && !starts_new_block(opening_body);
        // A fence opened by the definition's own content -- on the
        // opening line (`[^a]: ``` `) or on a later indented line --
        // makes the lines it covers literal fence content, never
        // paragraph content: they cannot enable an unindented lazy
        // continuation. Classifying each continuation line
        // independently treated `code` inside the fence as a
        // paragraph line, swallowed the following `<em>` into the
        // definition, and promoted the definition-shaped line
        // inside the HTML block to a real footnote (Codex P2 on
        // #1565's twenty-seventh cut). Tracked across continuation
        // lines until the closing fence.
        let mut def_fence = fence_opener_run(opening_body);
        // An HTML block opened by the definition's own content --
        // on the opening line (`[^a]: <!--`) or on a later indented
        // line -- makes the lines it covers literal HTML content,
        // never paragraph content, exactly like def_fence above:
        // while it is open an unindented line is NOT a lazy
        // continuation (HTML blocks take no lazy continuations),
        // so the definition ends there and the outer scan's own
        // HTML tracker owns the line. Ignoring the state let the
        // loop lazy-consume a top-level `<em>` into the definition,
        // bypass the outer tracker, and promote the
        // definition-shaped line inside that top-level block to a
        // real footnote (Codex P2 on #1565's twenty-eighth cut).
        let mut def_html = HtmlBlockTracker::default();
        def_html.feed(opening_body, true);
        let mut j = i + 1;
        while j < lines.len() {
            let next = lines[j];
            if is_blank_line(next) {
                source_lines.push(next);
                // A blank line ends a blank-line-terminated HTML
                // block inside the definition (marker-ended blocks
                // ignore it), mirroring the outer scans.
                def_html.feed("", true);
                prev_is_content = false;
                j += 1;
                continue;
            }
            if indentation_columns(next) >= 4 {
                source_lines.push(next);
                last_content = source_lines.len() - 1;
                // Lazy-continuation eligibility follows THIS line's
                // block type, not its mere presence: an indented
                // block line (`    # Heading`) is a new block inside
                // the definition and cannot accept an unindented
                // lazy paragraph continuation, so a following
                // `<em>` starts a top-level HTML block whose
                // definition-shaped interior is literal -- setting
                // content unconditionally consumed the tag into the
                // definition and then collected the literal line as
                // a real footnote (Codex P2 on #1565's twenty-third
                // cut). Mirrors the opening-body check above.
                let content = strip_indentation_columns(next, 4);
                // The first four columns continue the footnote. Any
                // remaining four columns are still an indented code
                // block, not paragraph content eligible for an
                // unindented lazy continuation (Codex P2 on #1565's
                // twenty-fifth cut).
                // A setext underline under paragraph content ends
                // that paragraph (it becomes a heading inside the
                // definition), so it leaves no lazy-continuation
                // eligibility either -- without this, `[^a]: Title`
                // / `    ===` kept eligibility, the following
                // unindented `<em>` was swallowed as a lazy
                // continuation, and the definition-shaped line
                // inside the HTML block was promoted to a real
                // footnote (Codex P2 on #1565's twenty-sixth cut).
                // A setext-shaped line with NO preceding paragraph
                // is itself paragraph text and stays eligible,
                // mirroring the outer scan's !at_block_start gate.
                if let Some((fence_ch, fence_run)) = def_fence {
                    // Inside a fence opened by this definition:
                    // the line is literal fence content until the
                    // closing fence, and is never paragraph content
                    // (twenty-seventh cut).
                    // A closing fence allows at most three
                    // columns of indentation relative to the
                    // footnote container: a delimiter retaining
                    // four is indented code, not a closer, and
                    // accepting it ended the fence early and let
                    // the lines after it be misclassified
                    // (Codex P2 on #1565's twenty-ninth cut).
                    if indentation_columns(content) <= 3
                        && is_fence_closer(content.trim_start_matches(' '), fence_ch, fence_run)
                    {
                        def_fence = None;
                    }
                    prev_is_content = false;
                // A NEW HTML opening is refused at four or more
                // container-relative indentation columns: that
                // line is indented code inside the footnote, not
                // a block opener -- trimming all its spaces first
                // opened a phantom block and misclassified the
                // real paragraph after it (Codex P2 on #1565's
                // twenty-ninth cut). An already-open block is
                // still fed every line.
                } else if (def_html.in_html() || indentation_columns(content) < 4)
                    && def_html.feed(content.trim_start_matches(' '), !prev_is_content)
                {
                    // Inside an HTML block opened by this
                    // definition (or opening one on this line):
                    // the line is HTML content, never paragraph
                    // content (twenty-eighth cut). A type-7 opening
                    // is refused while a paragraph is open, exactly
                    // as in the outer scans -- the tag is then
                    // paragraph text and falls through to the
                    // classification below on a later line shape.
                    prev_is_content = false;
                } else if indentation_columns(content) < 4 {
                    if let Some(opened) = fence_opener_run(content.trim_start_matches(' ')) {
                        // An indented fence opener inside the
                        // definition opens a fence for the lines
                        // that follow (twenty-seventh cut).
                        def_fence = Some(opened);
                        prev_is_content = false;
                    } else {
                        prev_is_content = !content.trim().is_empty()
                            && !starts_new_block(content.trim_start_matches(' '))
                            && !(prev_is_content && is_setext_underline(content));
                    }
                } else if prev_is_content {
                    // Overindented (eight-plus columns in total)
                    // with a paragraph already open: indented code
                    // cannot interrupt a paragraph, so the line
                    // continues that paragraph and lazy eligibility
                    // survives -- resetting it truncated the
                    // definition at the following unindented line
                    // and deleted that text on save (Codex P2 on
                    // #1565's twenty-eighth cut: `[^a]: First.` /
                    // eight-space continuation / `second.` lost
                    // `second.`).
                } else {
                    prev_is_content = false;
                }
                j += 1;
                continue;
            }
            // Lazy paragraph continuation (Codex P2 on #1565's
            // eighth cut): comrak continues a footnote definition's
            // paragraph onto a following unindented line that does
            // not start a new block (`[^a]: First line.` /
            // `second line.`); collecting only indented lines
            // truncated the definition and lost the continuation
            // on save.
            if prev_is_content
                && !def_html.in_html()
                && !starts_new_block(next.trim_start_matches(' '))
            {
                source_lines.push(next);
                last_content = source_lines.len() - 1;
                // An unindented setext underline lazy-continued
                // onto the definition's paragraph ends it the same
                // way (twenty-sixth cut): the paragraph becomes a
                // heading, so the next line starts fresh.
                prev_is_content = !is_setext_underline(next);
                j += 1;
                continue;
            }
            break;
        }
        source_lines.truncate(last_content + 1);
        blocks.push(FootnoteDefinitionBlock {
            label,
            source: source_lines.join("\n"),
            line: i,
        });
        at_block_start = true;
        i = j;
    }
    blocks
}

/// Append a PM `footnote_definition` node for every definition in
/// `markdown` that the comrak conversion did not already carry (#1565).
/// A label comrak emitted a node for is referenced (or at least
/// carried) and is left exactly as today; only the definitions comrak
/// dropped are synthesized, in source order, at the end of the doc.
/// The document's link reference definitions (`[label]: dest` lines,
/// footnote definitions excluded), verbatim, for the synthesis probe:
/// a reference-style link inside an unreferenced footnote resolves
/// against definitions elsewhere in the document, and a probe without
/// them converts the link to literal text (Codex P2).
/// The text after a link reference definition's destination, given
/// everything after its `]:` (accumulated across the definition's
/// lines). None while the destination itself is still incomplete --
/// nothing collected yet, or an unterminated angle destination.
fn link_reference_definition_title_text(after: &str) -> Option<&str> {
    let text = after.trim();
    if text.is_empty() {
        return None;
    }
    let rest = if let Some(inner) = text.strip_prefix('<') {
        // The destination ends at the first UNESCAPED '>': an
        // escaped `\>` is destination text, and ending there left
        // the remainder to be read as (invalid) trailing title
        // text, dropping a valid definition (Codex P2 on #1565's
        // twenty-first cut).
        let pos = find_unescaped(inner, '>')?;
        &inner[pos + 1..]
    } else {
        let end = text.find(char::is_whitespace).unwrap_or(text.len());
        &text[end..]
    };
    Some(rest.trim_start())
}

/// The byte index of the first occurrence of `target` in `s`
/// outside a backslash escape (same escape rule as
/// contains_unescaped).
fn find_unescaped(s: &str, target: char) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == target {
            return Some(i);
        }
    }
    None
}

/// Whether `s` contains `closer` outside a backslash escape: a
/// `\x` pair makes `x` literal text, and a `\\` pair is a
/// literal backslash, so the character after it counts again.
fn contains_unescaped(s: &str, closer: char) -> bool {
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == closer {
            return true;
        }
    }
    false
}

/// Whether a link reference definition needs more lines: its
/// destination has not arrived yet, or a title that was started
/// (double quote, single quote, or open paren) has not closed
/// (Codex P2 on #1565's seventh cut -- definitions legitimately
/// span lines, with the destination or the title on a following
/// line).
fn link_reference_definition_needs_more(after: &str) -> bool {
    let Some(title) = link_reference_definition_title_text(after) else {
        return true;
    };
    if title.is_empty() {
        return false;
    }
    let closer = match title.as_bytes()[0] {
        b'"' => '"',
        0x27 => 0x27 as char,
        b'(' => ')',
        _ => return false,
    };
    // An ESCAPED delimiter is title text, not the closer: in
    // `"first \"quoted` the title is still open, and treating it
    // as complete stopped collection before the closing line, so
    // the probe received an unterminated definition and the
    // recovered footnote lost its link (Codex P2 on #1565's
    // seventeenth cut).
    !contains_unescaped(&title[1..], closer)
}

/// Whether an accumulated candidate is a VALID link reference
/// definition: a destination is present and whatever follows it
/// is empty or a complete title. `link_reference_definition_title_text`
/// alone returns Some for ANY trailing text, including a bare word
/// (`[bad]: /bad extra`), which is not a title at all -- such a
/// line is no definition, and the lines around it stay one
/// paragraph (Codex P2 on #1565's twentieth cut).
fn link_reference_definition_is_valid(after: &str) -> bool {
    // The destination itself must be syntactically valid, not
    // merely followed by an empty or complete title: `/bad(` has
    // an unmatched parenthesis, so the line is no definition at
    // all, the definition-shaped line after it stays paragraph
    // text, and synthesis must not promote it (Codex P2 on
    // #1565's twenty-fourth cut).
    if !link_reference_destination_is_valid(after) {
        return false;
    }
    let Some(title) = link_reference_definition_title_text(after) else {
        return false;
    };
    if title.is_empty() {
        return true;
    }
    // A complete title must END the definition: text after the
    // closing delimiter (`[bad]: /bad "title" extra`) is not a
    // title at all, and accepting it collected the next line as
    // its own definition out of what is really one paragraph
    // (Codex P2 on #1565's twenty-second cut).
    let closer = match title.as_bytes()[0] {
        b'"' => '"',
        0x27 => '\'',
        b'(' => ')',
        _ => return false,
    };
    match find_unescaped(&title[1..], closer) {
        Some(end) => {
            // A parenthesized title cannot contain an unescaped
            // `(`: `(a(b)` is not a title, so the candidate is
            // no definition and the following definition-shaped
            // line stays paragraph text instead of being
            // promoted on its own (Codex P2 on #1565's
            // twenty-ninth cut).
            !(closer == ')' && contains_unescaped(&title[1..1 + end], '('))
                && title[end + 2..].trim().is_empty()
        }
        None => false,
    }
}

/// Whether the destination at the start of the accumulated
/// after-colon text is syntactically a link destination: an
/// angle destination ends at its unescaped `>` with no unescaped
/// `<` or line ending inside; a bare destination is non-empty,
/// contains no control characters, and keeps its parentheses
/// balanced (escapes exempt) -- the CommonMark destination
/// grammar. `/bad(` fails the balance check even though nothing
/// follows it, which the title-only validation above accepted.
fn link_reference_destination_is_valid(after: &str) -> bool {
    let text = after.trim();
    if text.is_empty() {
        return false;
    }
    if let Some(inner) = text.strip_prefix('<') {
        let Some(pos) = find_unescaped(inner, '>') else {
            return false;
        };
        let dest = &inner[..pos];
        // A title after an angle destination must be separated
        // from the closing `>` by whitespace: `</bad>"title"`
        // has no separator, so the candidate is no definition at
        // all and the lines around it stay one paragraph (Codex
        // P2 on #1565's twenty-ninth cut).
        let rest = &inner[pos + 1..];
        let separated = rest.is_empty() || rest.starts_with(char::is_whitespace);
        return separated && !dest.contains('\n') && !contains_unescaped(dest, '<');
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let dest = &text[..end];
    if dest.is_empty() {
        return false;
    }
    let mut depth = 0usize;
    let mut escaped = false;
    for c in dest.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            c if c.is_control() => return false,
            _ => {}
        }
    }
    depth == 0
}

/// Whether anything follows the destination in the accumulated
/// after-colon text (a title, complete or still arriving).
fn link_reference_definition_has_title(after: &str) -> bool {
    link_reference_definition_title_text(after).is_some_and(|t| !t.is_empty())
}

/// Whether a stripped line opens a link reference definition
/// (`[label]:`), footnote definitions excluded by the caller.
/// The index of the first UNESCAPED `]` in `s`: a `\]` inside
/// a reference label is label text, not the label's end (Codex
/// P2 on #1565's twelfth cut). Shared by the shape check and the
/// collector's destination extraction so they can never disagree
/// about where the label ends.
fn find_label_close(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == ']' {
            return Some(i);
        }
    }
    None
}

/// The closing bracket of a valid link reference label. CommonMark
/// labels cannot be empty/whitespace-only, contain an unescaped opening
/// bracket, or exceed 999 characters. Keeping this check with the shape
/// check prevents a malformed `[]:` candidate from splitting a paragraph
/// and promoting the next definition during synthesis (Codex P2 on
/// #1565's twenty-fifth cut).
fn link_reference_definition_label_close(s: &str) -> Option<usize> {
    if !s.starts_with('[') {
        return None;
    }
    let close = find_label_close(s)?;
    let label = &s[1..close];
    if label.trim().is_empty() || label.chars().count() > 999 {
        return None;
    }
    let mut escaped = false;
    for c in label.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' => return None,
            _ => {}
        }
    }
    Some(close)
}

fn is_link_reference_definition_start(stripped: &str) -> bool {
    match link_reference_definition_label_close(stripped) {
        Some(close) => stripped[close + 1..].starts_with(':'),
        None => false,
    }
}

fn collect_link_reference_definitions(markdown: &str) -> Vec<(usize, String)> {
    let mut fence = FenceTracker::default();
    let mut fence_prefix = String::new();
    let mut html_prefix = String::new();
    let mut html = HtmlBlockTracker::default();
    let mut defs = Vec::new();
    // A link reference definition is only a definition where it can
    // START a block: a definition-shaped line continuing a paragraph
    // is paragraph text, and promoting it into the synthesis context
    // would resolve references the document itself does not honor
    // (Codex P2 on #1565's fifth cut). Track block-start eligibility:
    // blank lines, headings, fences, HTML blocks, list/blockquote
    // lines and other definitions leave the next line block-eligible;
    // paragraph text does not.
    let mut at_block_start = true;
    // Inside a footnote definition's block (until a blank line or
    // a new block starter), a reference-shaped line is the
    // footnote's own lazy continuation, not a standalone
    // definition: collecting it would make the synthesis context
    // resolve as a link what the document keeps literal (Codex
    // P2 on #1565's twelfth cut).
    let mut in_footnote_block = false;
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        // Indentation in COLUMNS: a tab advances to the next
        // multiple of 4, so a tab-indented line is indented code
        // (indent 4), not a paragraph line at indent 0 (Codex P2
        // on #1565's sixteenth cut).
        let indent = {
            let mut col = 0usize;
            for c in line.chars() {
                match c {
                    ' ' => col += 1,
                    '\t' => col += 4 - (col % 4),
                    _ => break,
                }
            }
            col
        };
        let stripped = line.trim_start_matches(' ');
        if is_blank_line(stripped) {
            // A blank line ENDS a blank-line-terminated HTML block;
            // the tracker must see it or it suppresses every later
            // definition (Codex P2 on #1565's sixth cut).
            // A fully blank line carries no container prefix, so a
            // container-scoped HTML block is over (eleventh cut).
            if html.in_html() && html_prefix.contains('>') {
                html.reset();
                html_prefix.clear();
            }
            html.feed(stripped, true);
            in_footnote_block = false;
            at_block_start = true;
            i += 1;
            continue;
        }
        // An already-open HTML block sees every line, however
        // indented: an indented closing marker still closes it,
        // and gating this feed on indent left the tracker open
        // forever, suppressing every later definition (Codex P2
        // on #1565's seventh cut). The indent limit applies only
        // to recognizing an OPENING line.
        let container_stripped = strip_container_prefixes(stripped);
        // While an HTML block is open, feed it lines relative to
        // the container it opened in: a blank line inside a
        // blockquote is just `>` once spaces are stripped, and
        // feeding that literal line kept the tracker open past the
        // block's end, suppressing what follows it (Codex P2 on
        // #1565's ninth cut). Mirrors fence_prefix below.
        // (The recorded prefix's exact bytes vary -- `> ` vs `>`
        // on a blank quote line -- so relativity is computed by
        // re-stripping this line's own container prefixes.)
        // An HTML block opened inside a blockquote ends implicitly
        // when the container ends, exactly like a fence (tenth
        // cut): a following line carrying no container prefix at
        // all is outside the quote, so a marker-ended block (a
        // comment) is over even without its closing marker (Codex
        // P2 on #1565's eleventh cut: `> <!--` / `> hidden` /
        // blank / `[^real]: note` -- the tracker stayed open and
        // treated the real definition as comment interior).
        // (The blockquote clause above is the tenth/eleventh cut;
        // the list clause is the fifteenth: a non-blank line at
        // column zero carrying no container prefix is outside the
        // item, exactly as for a list-scoped fence.)
        if html.in_html()
            && !html_prefix.is_empty()
            && !stripped.starts_with(&html_prefix)
            && strip_container_prefixes(stripped) == stripped
            && (html_prefix.contains('>') || (indent == 0 && !stripped.is_empty()))
        {
            html.reset();
            html_prefix.clear();
        }
        let html_line = if html.in_html() && !html_prefix.is_empty() {
            container_stripped
        } else {
            stripped
        };
        let was_in_html = html.in_html();
        if !fence.in_fence()
            && (indent <= 3 || html.in_html())
            && (html.feed(html_line, at_block_start)
                || html.feed(container_stripped, at_block_start))
        {
            if !was_in_html && html.in_html() {
                // Opened on this line (via the container-stripped
                // form when a container prefix is present): remember
                // the prefix for the block's interior lines.
                html_prefix = stripped[..stripped.len() - container_stripped.len()].to_string();
            }
            if !html.in_html() {
                html_prefix.clear();
            }
            at_block_start = true;
            i += 1;
            continue;
        }
        // A fence opened inside a blockquote ends implicitly
        // when the container ends: a following line carrying no
        // container prefix at all is outside the quote, so the
        // fence is over even without a closing delimiter (Codex
        // P2 on #1565's tenth cut: `> ``` ` / `> code` / blank /
        // `[^real]: note` -- the scanner kept the fence open and
        // skipped the real definition).
        // A fence opened inside a list item ends the same way
        // (fourteenth cut): a non-blank line at column zero carrying
        // no container prefix is outside the item, so the fence is
        // over even without a closing delimiter (`- ``` ` / `  code`
        // / blank / `[^real]: note` -- the scanner kept the
        // list-scoped fence open and skipped the real definition).
        // A fence opened inside a list item ends when a SIBLING
        // item starts: the sibling's marker line sits at column
        // zero and carries a marker just like the opening line,
        // so the container-end check below mistakes it for a
        // continuation of the same item and the fence swallows
        // the sibling's content (Codex P2 on #1565's twenty-ninth
        // cut: `- ``` ` / `- [url]: /target` -- the scan stayed
        // inside the fence and the sibling's definition was
        // never seen). A sibling is a new container, not a
        // continuation.
        if fence.in_fence()
            && !fence_prefix.is_empty()
            && indent == 0
            && starts_with_list_marker(&fence_prefix)
            && starts_with_list_marker(stripped)
        {
            fence.reset();
            fence_prefix.clear();
        }
        if fence.in_fence()
            && !fence_prefix.is_empty()
            && !carries_container_prefix(stripped, &fence_prefix)
            && strip_container_prefixes(stripped) == stripped
            && (fence_prefix.contains('>') || (indent == 0 && !stripped.is_empty()))
        {
            fence.reset();
            fence_prefix.clear();
        }
        let fence_line = if fence.in_fence() {
            if carries_container_prefix(stripped, &fence_prefix) {
                strip_container_levels(stripped, container_signature(&fence_prefix).chars().count())
            } else {
                stripped
            }
        } else {
            container_stripped
        };
        let was_in_fence = fence.in_fence();
        if indent <= 3 && fence.feed(fence_feed_line(fence_line)) {
            if was_in_fence {
                fence_prefix.clear();
            } else {
                fence_prefix = stripped[..stripped.len() - container_stripped.len()].to_string();
            }
            at_block_start = true;
            i += 1;
            continue;
        }
        if fence.in_fence() || html.in_html() || indent > 3 {
            i += 1;
            continue;
        }
        if let Some(label) = footnote_definition_label(stripped) {
            // A footnote definition is its own leaf block; a link
            // definition after it starts a fresh block. The block
            // only swallows a following reference-shaped line as
            // its lazy continuation when the opening body is
            // paragraph text: a body that itself starts a block
            // (`[^a]: # Heading`) cannot accept a lazy
            // continuation, so the next line is a fresh top-level
            // block and a reference definition there is real
            // (Codex P2 on #1565's twenty-ninth cut). Mirrors
            // scan_footnote_definition_blocks' opening-body rule.
            let opening_body = stripped[label.len() + 4..].trim_start_matches(' ');
            in_footnote_block = !opening_body.trim().is_empty() && !starts_new_block(opening_body);
            at_block_start = true;
            i += 1;
            continue;
        }
        // Reference definitions inside a container are
        // document-global (Codex P2 on #1565's tenth cut):
        // detect the definition on the container-stripped line;
        // the original line is still collected verbatim below.
        let def_stripped = if is_link_reference_definition_start(stripped) {
            stripped
        } else {
            container_stripped
        };
        // The container this line sits in (empty at the top
        // level): a definition that opens inside a container
        // continues on lines carrying that same container, and
        // those continuation lines are processed RELATIVE to it
        // -- in the multiline-label accumulation below and in the
        // destination/title collection loop (Codex P2 on #1565's
        // twenty-eighth cut).
        let def_container_prefix = &stripped[..stripped.len() - container_stripped.len()];
        let def_container_levels = container_signature(def_container_prefix).chars().count();
        // A reference label may itself span lines (fourteenth
        // cut): `[long\nlabel]: https://example.com` is a valid
        // definition whose label normalizes to `long label`, but
        // the single-line shape check sees only `[long` and
        // rejects it before the continuation logic can run, so the
        // definition never reaches the synthesis context and a
        // recovered footnote's `[target][long label]` saves as
        // literal brackets instead of a hyperlink. When neither
        // stripped form closes a label on this line, accumulate
        // lines until the closing `]:` arrives.
        let multiline_start: Option<(Vec<&str>, String, usize)> =
            if is_link_reference_definition_start(def_stripped) {
                None
            } else {
                let candidate = if stripped.starts_with('[') {
                    stripped
                } else {
                    container_stripped
                };
                if !candidate.starts_with('[')
                    || candidate.starts_with("[^")
                    || find_label_close(candidate).is_some()
                {
                    None
                } else {
                    let mut text = candidate.to_string();
                    let mut j = i + 1;
                    let mut found = None;
                    while j < lines.len() && text.len() < 1000 {
                        let next = lines[j].trim_start_matches(' ');
                        // Read the segment relative to the
                        // candidate's container (twenty-eighth
                        // cut): `> label]: ...` contributes
                        // `label]: ...` to the label text.
                        let next = if def_container_levels > 0
                            && carries_container_prefix(next, def_container_prefix)
                        {
                            strip_container_levels(next, def_container_levels)
                        } else {
                            next
                        };
                        if is_blank_line(next) {
                            break;
                        }
                        // A line that starts a new block interrupts
                        // the paragraph the label is accumulating
                        // in, so the label cannot span it: stop
                        // here instead of accumulating across the
                        // interruption and accepting a phantom
                        // multiline label out of lines that are
                        // really a heading plus paragraph text
                        // (Codex P2 on #1565's twenty-ninth cut:
                        // `[long` / `# Heading` / `label]: /bad`).
                        if starts_new_block(next) || is_setext_underline(next) {
                            break;
                        }
                        text.push('\n');
                        text.push_str(next);
                        if let Some(close) = find_label_close(&text) {
                            if text[close + 1..].starts_with(':') {
                                if link_reference_definition_label_close(&text).is_some() {
                                    found = Some((
                                        lines[i..=j].to_vec(),
                                        text[close + 2..].to_string(),
                                        j + 1,
                                    ));
                                }
                            }
                            break;
                        }
                        j += 1;
                    }
                    found
                }
            };
        let single_start = is_link_reference_definition_start(def_stripped).then(|| {
            let close =
                link_reference_definition_label_close(def_stripped).expect("start shape checked");
            (vec![line], def_stripped[close + 2..].to_string(), i + 1)
        });
        // A line carrying a list marker starts a NEW list item
        // and so a fresh block: `- [url]: ...` after `- Body` is a
        // definition in its own item, not a continuation of the
        // first item's paragraph, and it likewise ends any footnote
        // block in progress (Codex P2 on #1565's fifteenth cut). A
        // non-1 ordered marker is excepted: it cannot interrupt a
        // paragraph (thirteenth cut), so it starts no item here.
        let new_list_item =
            starts_with_list_marker(stripped) && !is_non_interrupting_ordered_marker(stripped);
        if (at_block_start || new_list_item)
            && (!in_footnote_block || new_list_item)
            && (single_start.is_some() || multiline_start.is_some())
        {
            let (mut block, mut after, mut j) =
                single_start.or(multiline_start).expect("start checked");
            // Rollback point for a title attempted on a FOLLOWING
            // line (twenty-first cut): (block length, `after`, j)
            // before the first such line is appended. A title that
            // never completes does not invalidate the destination
            // already collected -- the candidate stays a valid
            // destination-only definition and the title-looking
            // line remains paragraph text (Codex P2 on #1565's
            // twenty-first cut).
            let mut title_rollback: Option<(usize, String, usize)> = None;
            // Collect the COMPLETE definition block: the destination
            // or the title may sit on following lines, and collecting
            // only the opening line degraded the reference to literal
            // text or silently dropped its title (Codex P2 on
            // #1565's seventh cut).
            while j < lines.len() {
                let next = lines[j];
                let next_stripped = next.trim_start_matches(' ');
                // A continuation line inside the definition's own
                // container is processed relative to that
                // container: the boundary check below mistook the
                // continuing `>` marker of `> [url]:` /
                // `> /target` for a new block and stopped before
                // the destination, so the collector returned no
                // definition and a recovered footnote lost its
                // hyperlink on save (Codex P2 on #1565's
                // twenty-eighth cut). The ORIGINAL line is still
                // what `block` collects verbatim for the probe.
                let next_stripped = if def_container_levels > 0
                    && carries_container_prefix(next_stripped, def_container_prefix)
                {
                    strip_container_levels(next_stripped, def_container_levels)
                } else {
                    next_stripped
                };
                if is_blank_line(next_stripped) {
                    break;
                }
                if link_reference_definition_needs_more(&after) {
                    // Still waiting for the destination or an
                    // unclosed title. A definition-shaped line while
                    // the destination is missing starts its own
                    // block instead of serving as a destination. A
                    // `#fragment` line is NOT definition-shaped: it
                    // is a valid destination token (an ATX heading
                    // needs whitespace or end-of-line after the
                    // hashes), so it is collected below (Codex P2
                    // on #1565's seventeenth cut).
                    // The boundary check also applies while an
                    // angle destination is still unterminated: a
                    // block-starting line cannot be swallowed
                    // into the destination, and consuming it took
                    // the heading AND the valid definition after
                    // it down with the discarded candidate (Codex
                    // P2 on #1565's nineteenth cut). A line
                    // containing an unescaped '>' may still close
                    // the destination and is consumed.
                    let pending_angle_destination = {
                        let t = after.trim_start();
                        t.starts_with('<') && !contains_unescaped(&t[1..], '>')
                    };
                    if (after.trim().is_empty()
                        || (pending_angle_destination && !contains_unescaped(next_stripped, '>')))
                        && (footnote_definition_label(next_stripped).is_some()
                            || is_link_reference_definition_start(next_stripped)
                            || is_atx_heading(next_stripped)
                            || next_stripped.starts_with('>')
                            || next_stripped.starts_with('`'))
                    {
                        break;
                    }
                    block.push(next);
                    after.push('\n');
                    after.push_str(next_stripped);
                    j += 1;
                    continue;
                }
                // Complete so far, but a title line still belongs to
                // this definition.
                if !link_reference_definition_has_title(&after)
                    && matches!(
                        next_stripped.as_bytes().first().copied(),
                        Some(b'"') | Some(0x27) | Some(b'(')
                    )
                {
                    if title_rollback.is_none() {
                        title_rollback = Some((block.len(), after.clone(), j));
                    }
                    block.push(next);
                    after.push('\n');
                    after.push_str(next_stripped);
                    j += 1;
                    continue;
                }
                break;
            }
            if link_reference_definition_is_valid(&after) {
                defs.push((i, block.join("\n")));
                at_block_start = true;
                i = j;
            } else if let Some((cp_len, _, cp_j)) = title_rollback
                .filter(|(_, cp_after, _)| link_reference_definition_is_valid(cp_after))
            {
                // The following-line title never completed: keep
                // the valid destination-only definition and let
                // the title-looking line be reprocessed as the
                // paragraph text it is (twenty-first cut).
                block.truncate(cp_len);
                defs.push((i, block.join("\n")));
                at_block_start = true;
                i = cp_j;
            } else {
                // Not a definition at all (twentieth cut): the
                // opening line is paragraph text, so the next line
                // is its continuation -- resetting block-start
                // eligibility here collected the following line as
                // its own definition (`[bad]: /bad extra` /
                // `[url]: /target` are one paragraph; `[url]` in
                // it is no definition), and synthesis then turned
                // it into a real definition in the probe. Advance
                // one line only, as paragraph text.
                at_block_start = false;
                i += 1;
            }
            continue;
        }
        // Any other line: headings and container lines end a
        // paragraph (next line may start a block); plain text is a
        // paragraph line (next line is its continuation).
        // Thematic breaks and setext underlines also end a
        // paragraph: after `---` a reference definition starts a
        // new block and comrak honors it, but treating `---` as
        // paragraph text left the next line ineligible and the
        // definition was omitted from the synthesis context
        // (Codex P2 on #1565's eighth cut).
        // A setext underline ends a paragraph only when a
        // paragraph precedes it: a lone `=` at a block start is
        // itself paragraph text, and a definition-shaped line
        // after it is that paragraph's continuation, not a
        // definition (Codex P2 on #1565's ninth cut).
        if is_atx_heading(stripped) || is_thematic_break(stripped) || starts_new_block(stripped) {
            in_footnote_block = false;
        }
        let setext_ends_paragraph = is_setext_underline(stripped) && !at_block_start;
        // A blockquote line's eligibility follows its CONTENT
        // (eleventh cut): `> Body` is paragraph text inside the
        // quote, so a definition-shaped line continuing it inside
        // the quote is paragraph text too, not a definition --
        // promoting it resolved references the document does not
        // honor. Only quote content that itself ends or starts a
        // block (blank, heading, break, setext, a new block)
        // leaves the next line eligible. Non-quote container
        // lines (list items) keep the older rule.
        let quote_eligible = stripped.starts_with('>').then(|| {
            let c = container_stripped;
            c.trim().is_empty()
                || is_atx_heading(c)
                || is_thematic_break(c)
                || (is_setext_underline(c) && !at_block_start)
                || starts_new_block(c)
        });
        // A list item line's eligibility follows its CONTENT too
        // (fourteenth cut): `- Body` is paragraph text inside the
        // item, so an indented definition-shaped line continuing
        // it (`  [url]: ...`) is paragraph text, not a reference
        // definition -- the older rule left every container line
        // block-eligible and promoted it into the synthesis
        // context. Only item content that itself ends or starts a
        // block leaves the next line eligible, mirroring the
        // blockquote rule above.
        let list_eligible =
            if quote_eligible.is_none() && strip_container_prefixes(stripped) != stripped {
                let c = container_stripped;
                Some(
                    c.trim().is_empty()
                        || is_atx_heading(c)
                        || is_thematic_break(c)
                        || (is_setext_underline(c) && !at_block_start)
                        || starts_new_block(c),
                )
            } else {
                None
            };
        at_block_start = is_atx_heading(stripped)
            || quote_eligible
                .or(list_eligible)
                .unwrap_or_else(|| strip_container_prefixes(stripped) != stripped)
            || is_thematic_break(stripped)
            || setext_ends_paragraph;
        i += 1;
    }
    defs
}

fn preserve_unreferenced_footnote_definitions(
    doc: &mut Value,
    markdown: &str,
    newline_token: Option<&str>,
) {
    let carried: Vec<String> = doc["content"]
        .as_array()
        .map(|content| {
            content
                .iter()
                .filter(|n| n["type"] == "footnote_definition")
                .filter_map(|n| n["attrs"]["label"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let reference_defs = collect_link_reference_definitions(markdown);
    let blocks = scan_footnote_definition_blocks(markdown);
    let orphans: Vec<&FootnoteDefinitionBlock> = blocks
        .iter()
        .filter(|block| !carried.contains(&block.label))
        .collect();
    // ONE augmented parse recovers every orphan (Codex P2 on #1565's
    // fifth cut): the probe references every orphan label and carries
    // EVERY scanned definition block (orphan and carried alike) plus
    // the document's link reference definitions as resolution context
    // -- an unreferenced definition may itself reference a footnote
    // the main parse already carried (`[^outer]: See [^inner].` where
    // [^inner] is carried), and a probe without that definition
    // degrades the reference to escaped literal text (third cut). A
    // per-orphan probe scaled quadratically with the number of
    // orphans.
    let recovered = synthesize_footnote_definitions(&orphans, &blocks, &reference_defs);
    let mut synthesized = Vec::new();
    // Duplicate labels (fourteenth cut): the scanner keeps every
    // source block, but synthesis resolves one node per label, so
    // appending per block cloned the same node for each duplicate
    // and overwrote the later bodies on save. Append the resolved
    // definition once per label -- the first definition wins,
    // matching how the document itself resolves duplicates.
    let mut seen_labels = std::collections::HashSet::new();
    for block in &orphans {
        if !seen_labels.insert(block.label.clone()) {
            continue;
        }
        if let Some(node) = recovered.get(&block.label) {
            let mut node = node.clone();
            // The probe parsed the PROTECTED source, whose link
            // destinations carry the newline placeholder token; the
            // outer restore ran before this node existed, so restore
            // the token here or it would be saved into the URL
            // (Codex P2).
            if let Some(token) = newline_token {
                restore_attr_newlines(&mut node, token);
            }
            synthesized.push(node);
        }
    }
    if !synthesized.is_empty()
        && let Some(content) = doc["content"].as_array_mut()
    {
        content.extend(synthesized);
    }
}

/// Build the PM nodes for every unreferenced definition in ONE
/// augmented parse: a probe document that references every orphan
/// label (so comrak emits each definition) and carries every
/// definition block plus the document's link reference definitions
/// as resolution context, so references inside a definition resolve
/// exactly as they would in the document. Going through the same
/// conversion guarantees the synthesized nodes have exactly the
/// shape the referenced path produces (#1031 block children
/// included). Returns the first node per label, keyed by label.
fn synthesize_footnote_definitions(
    orphans: &[&FootnoteDefinitionBlock],
    all_blocks: &[FootnoteDefinitionBlock],
    reference_defs: &[(usize, String)],
) -> std::collections::HashMap<String, Value> {
    let mut nodes = std::collections::HashMap::new();
    if orphans.is_empty() {
        return nodes;
    }
    let mut probe = String::new();
    for block in orphans {
        probe.push_str(&format!("x[^{}]\n\n", block.label));
    }
    // Footnote blocks and link reference definitions interleave in
    // DOCUMENT order (Codex P2 on #1565's eighth cut): reference
    // definitions are global and the first definition wins, so
    // appending every collected definition after the footnote
    // blocks reversed precedence whenever a footnote block carried
    // a nested (indented) definition of a label the document had
    // already defined at top level.
    let mut segments: Vec<(usize, &str)> = all_blocks
        .iter()
        .map(|block| (block.line, block.source.as_str()))
        .chain(
            reference_defs
                .iter()
                .map(|(line, text)| (*line, text.as_str())),
        )
        .collect();
    segments.sort_by_key(|(line, _)| *line);
    for (_, text) in segments {
        probe.push_str(text);
        probe.push_str("\n\n");
    }
    let doc = markdown_to_prosemirror_inner(&probe, false);
    if let Some(content) = doc["content"].as_array() {
        for n in content {
            if n["type"] == "footnote_definition" {
                if let Some(label) = n["attrs"]["label"].as_str() {
                    nodes.entry(label.to_string()).or_insert_with(|| n.clone());
                }
            }
        }
    }
    nodes
}

/// Replace each newline inside an angle-form link/image destination
/// (`](<...>)` inline, `]: <...>` reference definition) with a token
/// comrak keeps verbatim in the parsed URL, returning the protected
/// source and the token used (`None` when no destination needed
/// protection). Fenced code blocks are skipped — a destination-shaped
/// string inside a fence is literal text, not a link. A candidate
/// destination that hits a blank line or another `<` before its
/// closing `>` is not a destination and is left alone.
fn protect_newline_destinations(markdown: &str) -> (String, Option<String>) {
    if !markdown.contains('\n') {
        return (markdown.to_string(), None);
    }
    let mut token = String::from("idweb-newline-0-token");
    let mut counter = 0;
    while markdown.contains(&token) {
        counter += 1;
        token = format!("idweb-newline-{counter}-token");
    }
    let bytes = markdown.as_bytes();
    let fence = "`".repeat(3);
    let tilde_fence = "~".repeat(3);
    let mut out = String::with_capacity(markdown.len());
    let mut i = 0;
    let mut in_fence = false;
    let mut protected_any = false;
    while i < bytes.len() {
        // Fence tracking is line-based: a line whose first non-space
        // characters (up to 3 spaces) are ``` or ~~~ toggles fence state.
        if i == 0 || bytes[i - 1] == b'\n' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' && j - i < 3 {
                j += 1;
            }
            if markdown[j..].starts_with(&fence) || markdown[j..].starts_with(&tilde_fence) {
                in_fence = !in_fence;
            }
        }
        if !in_fence && (markdown[i..].starts_with("](<") || markdown[i..].starts_with("]: <")) {
            let lt = if markdown[i..].starts_with("](<") {
                i + 2
            } else {
                i + 3
            };
            // Find the closing '>' before any blank line or nested '<'.
            let mut end = None;
            let mut k = lt + 1;
            while k < bytes.len() {
                match bytes[k] {
                    b'>' => {
                        end = Some(k);
                        break;
                    }
                    b'<' => break,
                    b'\n' if k + 1 < bytes.len() && bytes[k + 1] == b'\n' => break,
                    _ => k += 1,
                }
            }
            if let Some(gt) = end {
                let dest = &markdown[lt + 1..gt];
                if dest.contains('\n') {
                    out.push_str(&markdown[i..=lt]);
                    out.push_str(&dest.replace('\n', &token));
                    i = gt;
                    protected_any = true;
                    continue;
                }
            }
        }
        let Some(ch) = markdown[i..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();
        out.push_str(&markdown[i..i + ch_len]);
        i += ch_len;
    }
    if protected_any {
        (out, Some(token))
    } else {
        (markdown.to_string(), None)
    }
}

/// Whether `line` is a GFM table delimiter row (`| --- | :---: |`,
/// `--- | ---`, …): optional outer pipes, every cell a run of dashes
/// with an optional alignment colon on each side. The line must contain
/// a pipe — a bare `---` is a setext underline or thematic break, and
/// every table comrak itself renders carries pipes in its delimiter
/// row.
fn is_table_delimiter_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('-') || !trimmed.contains('|') {
        return false;
    }
    let body = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let body = body.strip_suffix('|').unwrap_or(body);
    if body.trim().is_empty() {
        return false;
    }
    body.split('|').all(|cell| {
        let cell = cell.trim();
        let cell = cell.strip_prefix(':').unwrap_or(cell);
        let cell = cell.strip_suffix(':').unwrap_or(cell);
        !cell.is_empty() && cell.bytes().all(|b| b == b'-')
    })
}

/// Whether the byte at `i` is backslash-escaped: preceded by an odd
/// number of backslashes.
fn byte_is_escaped(bytes: &[u8], i: usize) -> bool {
    let mut backslashes = 0;
    let mut j = i;
    while j > 0 && bytes[j - 1] == b'\\' {
        backslashes += 1;
        j -= 1;
    }
    backslashes % 2 == 1
}

/// Escape every unescaped pipe inside a code span on a single line
/// (#1424). A code span opens at an unescaped backtick run and closes
/// at the next run of exactly the same length on the same line; an
/// unmatched run is literal text and is left alone, as is everything
/// outside code spans (plain-text pipes are the row syntax itself).
fn escape_code_span_pipes_in_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 8);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' && !byte_is_escaped(bytes, i) {
            let mut run = 0;
            while i + run < bytes.len() && bytes[i + run] == b'`' {
                run += 1;
            }
            // Find the closing run of exactly `run` backticks.
            let mut close = None;
            let mut j = i + run;
            while j < bytes.len() {
                if bytes[j] == b'`' {
                    let mut r = 0;
                    while j + r < bytes.len() && bytes[j + r] == b'`' {
                        r += 1;
                    }
                    if r == run {
                        close = Some((j, j + r));
                        break;
                    }
                    j += r;
                } else {
                    j += 1;
                }
            }
            if let Some((close_start, close_end)) = close {
                out.push_str(&line[i..i + run]);
                let mut backslashes = 0usize;
                for ch in line[i + run..close_start].chars() {
                    if ch == '|' && backslashes % 2 == 0 {
                        out.push('\\');
                    }
                    if ch == '\\' {
                        backslashes += 1;
                    } else {
                        backslashes = 0;
                    }
                    out.push(ch);
                }
                out.push_str(&line[close_start..close_end]);
                i = close_end;
                continue;
            }
            out.push_str(&line[i..i + run]);
            i += run;
            continue;
        }
        let ch_len = line[i..].chars().next().unwrap().len_utf8();
        out.push_str(&line[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Escape pipes inside code spans in the rows of every table in
/// `markdown` (#1424): the header row (identified by the delimiter row
/// that follows it), and the body rows that follow the delimiter row
/// up to the next blank line. Lines outside tables are returned
/// untouched — outside a table `\|` inside a code span is literal
/// text, not an escape, so escaping there would corrupt the span.
/// Fenced code blocks are skipped for the same reason (#2362): a
/// table example inside a fence is literal text comrak renders
/// verbatim, not a live table — the fence tracking mirrors
/// `protect_newline_destinations`.
/// A fence delimiter line (CommonMark §4.5): up to three leading
/// spaces, then a run of at least three backticks or tildes. Either
/// character toggles — the same approximation
/// `protect_newline_destinations` makes.
fn is_fence_line(line: &str) -> bool {
    let mut spaces = 0;
    let mut chars = line.chars();
    let first = loop {
        match chars.next() {
            Some(' ') => {
                spaces += 1;
                if spaces > 3 {
                    return false;
                }
            }
            Some(c) => break c,
            None => return false,
        }
    };
    if first != '`' && first != '~' {
        return false;
    }
    1 + chars.take_while(|&c| c == first).count() >= 3
}

fn escape_table_code_span_pipes(markdown: &str) -> String {
    if !markdown.contains('`') || !markdown.contains('|') {
        return markdown.to_string();
    }
    let lines: Vec<&str> = markdown.split_inclusive('\n').collect();
    let mut out = String::with_capacity(markdown.len() + 16);
    let mut in_table = false;
    let mut in_fence = false;
    for (idx, raw) in lines.iter().enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let ending = &raw[line.len()..];
        if is_fence_line(line) {
            in_fence = !in_fence;
            if in_fence {
                // A table cannot span a fence; a delimiter-looking
                // row inside the fence must not arm `in_table` for
                // whatever follows the fence either.
                in_table = false;
            }
            out.push_str(raw);
            continue;
        }
        if in_fence {
            out.push_str(raw);
            continue;
        }
        if is_table_delimiter_row(line) {
            in_table = true;
            out.push_str(raw);
            continue;
        }
        if in_table && line.trim().is_empty() {
            in_table = false;
        }
        let header_row = lines
            .get(idx + 1)
            .is_some_and(|next| is_table_delimiter_row(next));
        if in_table || header_row {
            out.push_str(&escape_code_span_pipes_in_line(line));
            out.push_str(ending);
        } else {
            out.push_str(raw);
        }
    }
    out
}

/// Restore protected newlines in the converted document: the token
/// only ever stands inside a link `href` or image `src` attr (the
/// parse produced it from a destination), so only those string attrs
/// are rewritten — literal text that happens to contain the token
/// string is untouched.
fn restore_attr_newlines(value: &mut Value, token: &str) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if (key == "href" || key == "src")
                    && let Value::String(text) = val
                    && text.contains(token)
                {
                    *text = text.replace(token, "\n");
                } else {
                    restore_attr_newlines(val, token);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                restore_attr_newlines(item, token);
            }
        }
        _ => {}
    }
}

/// Active marks being applied to inline content.
#[derive(Clone)]
struct Mark {
    mark_type: &'static str,
    attrs: Option<Value>,
}

/// Canonical mark nesting order, outermost first (#944).
///
/// Both conversion directions agree on this order, so a marked text node
/// has exactly one markdown spelling: the parse direction sorts the marks
/// it emits into this order (see [`text_node_with_marks`]) and the save
/// direction sorts incoming marks into this order before wrapping (see
/// `create_marked_text`). Without the shared order, `**[text](url)` and
/// `[**text**](url)` converted into each other on every save — the parse
/// direction appends marks outermost-first (wrapper arms push as they
/// nest) while the save direction treated the array as innermost-first.
fn mark_nesting_rank(mark_type: &str) -> u8 {
    match mark_type {
        "link" => 0,
        "strong" => 1,
        "em" => 2,
        "strikethrough" => 3,
        "code" => 4,
        _ => 5,
    }
}

/// Convert a comrak AST node to `ProseMirror` JSON.
fn convert_node<'a>(node: &'a AstNode<'a>, active_marks: &[Mark]) -> Value {
    let data = node.data.borrow();

    match &data.value {
        NodeValue::Document => {
            let content = collect_block_children(node);
            json!({
                "type": "doc",
                "content": if content.is_empty() {
                    vec![json!({"type": "paragraph"})]
                } else {
                    content
                }
            })
        }

        NodeValue::Paragraph => {
            let content = collect_inline_children(node, active_marks);
            if content.is_empty() {
                json!({"type": "paragraph"})
            } else {
                json!({
                    "type": "paragraph",
                    "content": content
                })
            }
        }

        NodeValue::Heading(heading) => {
            let content = collect_inline_children(node, active_marks);
            let mut node_json = json!({
                "type": "heading",
                "attrs": {"level": heading.level}
            });
            if !content.is_empty() {
                node_json["content"] = json!(content);
            }
            node_json
        }

        NodeValue::BlockQuote => {
            let content = collect_block_children(node);
            json!({
                "type": "blockquote",
                "content": if content.is_empty() {
                    vec![json!({"type": "paragraph"})]
                } else {
                    content
                }
            })
        }

        NodeValue::CodeBlock(code_block) => {
            let text = &code_block.literal;
            // Remove trailing newline if present (ProseMirror doesn't expect it)
            let text = text.strip_suffix('\n').unwrap_or(text);

            let mut node_json = json!({"type": "code_block"});

            // Add params attribute if there's an info string
            if !code_block.info.is_empty() {
                node_json["attrs"] = json!({"params": code_block.info});
            }

            if !text.is_empty() {
                node_json["content"] = json!([{"type": "text", "text": text}]);
            }

            node_json
        }

        NodeValue::ThematicBreak => {
            json!({"type": "horizontal_rule"})
        }

        NodeValue::List(_) => {
            // #1370: a mixed task/plain list converts to several sibling
            // lists (see convert_list_runs); block parents splice them in
            // via collect_block_children. A direct call gets the first
            // run - the remaining call sites are the document root and
            // list-item collection, where a list is always reached
            // through its parent's block children instead.
            convert_list_runs(node)
                .into_iter()
                .next()
                .unwrap_or(Value::Null)
        }

        NodeValue::Item(_) => {
            let content = collect_block_children(node);
            json!({
                "type": "list_item",
                "content": if content.is_empty() {
                    vec![json!({"type": "paragraph"})]
                } else {
                    content
                }
            })
        }

        NodeValue::Text(text) => text_node_with_marks(text.as_ref(), active_marks),

        NodeValue::SoftBreak => {
            // Soft breaks become spaces in ProseMirror
            text_node_with_marks(" ", active_marks)
        }

        NodeValue::LineBreak => {
            json!({"type": "hard_break"})
        }

        NodeValue::Code(code) => {
            // Inline code - add code mark to the text
            let mut marks = active_marks.to_vec();
            marks.push(Mark {
                mark_type: "code",
                attrs: None,
            });
            text_node_with_marks(&code.literal, &marks)
        }

        NodeValue::Emph => {
            // Emphasis - collect children with em mark added
            let mut marks = active_marks.to_vec();
            marks.push(Mark {
                mark_type: "em",
                attrs: None,
            });
            // Return children directly (will be flattened by caller)
            json!(collect_inline_children(node, &marks))
        }

        NodeValue::Strong => {
            // Strong - collect children with strong mark added
            let mut marks = active_marks.to_vec();
            marks.push(Mark {
                mark_type: "strong",
                attrs: None,
            });
            json!(collect_inline_children(node, &marks))
        }

        NodeValue::Link(link) => {
            // Link - collect children with link mark added
            let mut marks = active_marks.to_vec();
            let mut attrs = json!({"href": link.url});
            if !link.title.is_empty() {
                attrs["title"] = json!(link.title);
            }
            marks.push(Mark {
                mark_type: "link",
                attrs: Some(attrs),
            });
            let children = collect_inline_children(node, &marks);
            if children.is_empty() {
                // #996: an empty-text link has no inline children to
                // carry the link mark, so without this the mark — and
                // the href — vanish and save emits nothing. Synthesize
                // an empty text node carrying the mark; the save side
                // wraps it back into a Link node (create_marked_text),
                // so `[](url)` round-trips.
                return json!(vec![text_node_with_marks("", &marks)]);
            }
            json!(children)
        }

        NodeValue::Image(image) => {
            let mut attrs = json!({
                "src": image.url
            });

            // #1417: store the alt in markdown form, not flattened plain
            // text - get_text_content dropped emphasis/strong/code-span
            // structure, so `![*alt*](/x)` saved back as `![alt](/x)`.
            // The PM image node carries `alt` as a plain string
            // (editor.ts schema), so that string IS the markdown source
            // for the alt; the save direction re-parses it into inline
            // nodes (see json_to_ast's image special case). Plain-text
            // alts are unchanged by this encoding. Editor limitation:
            // the alt popover shows/edits the markdown form (e.g.
            // `*alt*`), which is the documented trade-off for #1417.
            let alt = get_alt_markdown(node);
            if !alt.is_empty() {
                attrs["alt"] = json!(alt);
            }

            if !image.title.is_empty() {
                attrs["title"] = json!(image.title);
            }

            // #880: an image inside a link arrives here with a link mark in
            // active_marks (the Link arm is a mark wrapper). Carry the link
            // on the image node itself — the editor schema has no way to
            // wrap an image node in a mark — so the href survives to save.
            if let Some(link_mark) = active_marks.iter().find(|m| m.mark_type == "link")
                && let Some(link_attrs) = &link_mark.attrs
            {
                if let Some(href) = link_attrs["href"].as_str() {
                    attrs["linkHref"] = json!(href);
                }
                if let Some(link_title) = link_attrs["title"].as_str() {
                    attrs["linkTitle"] = json!(link_title);
                }
            }

            // #1407: emphasis/strong/strikethrough wrapping an image
            // ride on the image node as a `marks` array (the text-node
            // shape) — like the link above, the editor schema cannot
            // wrap an image node in a mark, so the marks travel on the
            // node and the save direction re-wraps (see the image case
            // in json_to_ast). The link stays in linkHref (#880).
            let wrap_marks: Vec<Value> = active_marks
                .iter()
                .filter(|m| matches!(m.mark_type, "em" | "strong" | "strikethrough"))
                .map(|m| json!({"type": m.mark_type}))
                .collect();
            let mut image_node = json!({
                "type": "image",
                "attrs": attrs
            });
            if !wrap_marks.is_empty() {
                image_node["marks"] = json!(wrap_marks);
            }
            image_node
        }

        // Unsupported nodes - pass through as best we can
        NodeValue::HtmlBlock(html) => {
            // Check if this is an <img> tag with width/height, or the
            // combined <a><img /></a> sized linked image form (#893)
            let trimmed = html.literal.trim();
            if let Some(img_node) = parse_linked_img_tag(trimmed).or_else(|| parse_img_tag(trimmed))
            {
                // Wrap in paragraph since this is block-level
                return json!({
                    "type": "paragraph",
                    "content": [img_node]
                });
            }
            // Convert HTML blocks to code blocks (safe fallback), with
            // a provenance attr (#982, cf. the #880 linkHref precedent):
            // in PM shape a converted block is otherwise identical to an
            // authored ```html fence, so the save direction uses
            // `htmlBlock` to tell them apart and emit a real HtmlBlock
            // for converted blocks while authored fences stay fenced.
            let text = html.literal.strip_suffix('\n').unwrap_or(&html.literal);
            if text.is_empty() {
                json!({"type": "paragraph"})
            } else {
                json!({
                    "type": "code_block",
                    "attrs": {"params": "html", "htmlBlock": true},
                    "content": [{"type": "text", "text": text}]
                })
            }
        }

        NodeValue::HtmlInline(html) => {
            // Check if this is an <img> tag with width/height, or the
            // combined <a><img /></a> form in a single literal (#893; the
            // split form is merged in collect_inline_children)
            if let Some(img_attrs) = parse_linked_img_tag(html).or_else(|| parse_img_tag(html)) {
                return img_attrs;
            }
            // #943: other inline HTML passes through as an html_inline
            // atom node carrying the raw tag, so the save direction emits
            // it verbatim (NodeValue::HtmlInline) instead of the comrak
            // text renderer backslash-escaping it as literal text.
            json!({
                "type": "html_inline",
                "attrs": {"html": html}
            })
        }

        NodeValue::Table(table) => {
            let children = collect_block_children(node);
            let mut table_json = json!({
                "type": "table",
                "content": if children.is_empty() {
                    vec![json!({"type": "table_row", "content": [{"type": "table_cell", "content": [{"type": "paragraph"}]}]})]
                } else {
                    children
                }
            });
            // #881: carry column alignments on the table node (absent = all
            // None, exactly as before) so the delimiter row survives save.
            if table
                .alignments
                .iter()
                .any(|a| !matches!(a, comrak::nodes::TableAlignment::None))
            {
                let alignments: Vec<Value> = table
                    .alignments
                    .iter()
                    .map(|a| {
                        json!(match a {
                            comrak::nodes::TableAlignment::Left => "left",
                            comrak::nodes::TableAlignment::Center => "center",
                            comrak::nodes::TableAlignment::Right => "right",
                            comrak::nodes::TableAlignment::None => "none",
                        })
                    })
                    .collect();
                table_json["attrs"] = json!({ "alignments": alignments });
            }
            table_json
        }

        NodeValue::TableRow(is_header) => {
            let cell_type = if *is_header {
                "table_header"
            } else {
                "table_cell"
            };
            let children: Vec<Value> = node
                .children()
                .map(|child| {
                    // Table cells contain inline content in comrak,
                    // but ProseMirror needs block content (paragraphs)
                    let inline_content = collect_inline_children(child, active_marks);
                    let content = if inline_content.is_empty() {
                        vec![json!({"type": "paragraph"})]
                    } else {
                        vec![json!({"type": "paragraph", "content": inline_content})]
                    };
                    json!({
                        "type": cell_type,
                        "content": content
                    })
                })
                .collect();
            json!({
                "type": "table_row",
                "content": children
            })
        }

        NodeValue::TableCell => {
            // Fallback: normally handled by TableRow above
            let inline_content = collect_inline_children(node, active_marks);
            let content = if inline_content.is_empty() {
                vec![json!({"type": "paragraph"})]
            } else {
                vec![json!({"type": "paragraph", "content": inline_content})]
            };
            json!({
                "type": "table_cell",
                "content": content
            })
        }

        NodeValue::TaskItem(task) => {
            // Task items have a checked state (symbol present = checked)
            let checked = task.symbol.is_some();
            let content = collect_block_children(node);
            json!({
                "type": "task_list_item",
                "attrs": { "checked": checked },
                "content": if content.is_empty() {
                    vec![json!({"type": "paragraph"})]
                } else {
                    content
                }
            })
        }

        NodeValue::Strikethrough => {
            // Strikethrough - collect children with strikethrough mark added
            let mut marks = active_marks.to_vec();
            marks.push(Mark {
                mark_type: "strikethrough",
                attrs: None,
            });
            json!(collect_inline_children(node, &marks))
        }

        NodeValue::FootnoteReference(reference) => {
            // #945: carry the reference as a dedicated inline atom node so
            // the save direction can emit [^label] again instead of the
            // reference being dropped / backslash-escaped as plain text.
            json!({
                "type": "footnote_reference",
                "attrs": {"label": reference.name}
            })
        }

        NodeValue::FootnoteDefinition(definition) => {
            // #945: carry the definition as a dedicated node with the
            // label, mirroring the reference arm. #1031: carry its BLOCK
            // children (paragraphs, lists, code blocks) as blocks —
            // flattening them to inline content fused multi-paragraph
            // definitions into one run of text on save and silently
            // dropped code blocks (their literal has no child nodes).
            json!({
                "type": "footnote_definition",
                "attrs": {"label": definition.name},
                "content": collect_block_children(node)
            })
        }

        // Other unsupported nodes
        _ => {
            // Try to get text content and return as paragraph
            let text = get_text_content(node);
            if text.is_empty() {
                json!(null)
            } else {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": text}]
                })
            }
        }
    }
}

/// Create a text node with the given marks.
fn text_node_with_marks(text: &str, marks: &[Mark]) -> Value {
    // An empty text node with no marks carries nothing and is dropped,
    // but an empty text node WITH marks is information-bearing: it is
    // the only representation of an empty-text link (#996).
    if text.is_empty() && marks.is_empty() {
        return json!(null);
    }

    let mut node = json!({
        "type": "text",
        "text": text
    });

    if !marks.is_empty() {
        // #944: emit marks in the canonical nesting order so every
        // spelling of the same marked text parses to the same PM doc.
        let mut sorted: Vec<Mark> = marks.to_vec();
        sorted.sort_by_key(|m| mark_nesting_rank(m.mark_type));
        let marks_json: Vec<Value> = sorted
            .iter()
            .map(|m| {
                if let Some(attrs) = &m.attrs {
                    json!({"type": m.mark_type, "attrs": attrs})
                } else {
                    json!({"type": m.mark_type})
                }
            })
            .collect();
        node["marks"] = json!(marks_json);
    }

    node
}

/// Collect block-level children of a node.
fn collect_block_children<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    let children: Vec<&AstNode<'a>> = node.children().collect();
    let mut out = Vec::new();
    for (idx, child) in children.iter().enumerate() {
        // #1370: comrak's renderer separates adjacent lists with
        // <!-- end list --> comments, and the split sibling lists this
        // change emits are adjacent by construction - so on reparse the
        // separator comes back as an HtmlBlock between the lists. It is
        // a separator, not content: drop it when a neighbouring sibling
        // is a list. An authored <!-- end list --> comment NOT next to
        // a list still converts exactly as before (an htmlBlock
        // code_block, #982).
        if is_end_list_separator(child)
            && ((idx > 0 && matches!(children[idx - 1].data.borrow().value, NodeValue::List(_)))
                || children
                    .get(idx + 1)
                    .is_some_and(|c| matches!(c.data.borrow().value, NodeValue::List(_))))
        {
            continue;
        }
        if matches!(child.data.borrow().value, NodeValue::List(_)) {
            // #1370: a list converts to one or more sibling lists.
            out.extend(convert_list_runs(child));
        } else {
            let converted = convert_node(child, &[]);
            if !converted.is_null() {
                out.push(converted);
            }
        }
    }
    out
}

/// True for the HtmlBlock comrak's renderer emits between adjacent
/// lists (see collect_block_children).
fn is_end_list_separator<'a>(node: &'a AstNode<'a>) -> bool {
    match &node.data.borrow().value {
        NodeValue::HtmlBlock(html) => html.literal.trim() == "<!-- end list -->",
        _ => false,
    }
}

/// Convert a comrak list node to one or more ProseMirror list nodes.
///
/// A pure list converts exactly as before (#997/#1036 attrs carried from
/// the source list). #1370: a list mixing task items with plain items
/// used to take its type from the first child only and convert the rest
/// by their own kind, producing a `task_list` holding a `list_item` (or
/// a plain list holding a `task_list_item`) - a shape the save-side
/// placement validation and the editor schema both forbid, so the
/// document could not be saved at all. Instead, group the items into
/// runs of one kind and emit one sibling list per run, in source order.
/// Ordered runs carry an `order` start offset by the items before them
/// so numbering renders unchanged; `tight` is carried from the source
/// list to every run. Placement validation and the editor schema are
/// untouched: every emitted list is pure.
fn convert_list_runs<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    let (list_type, start, tight) = match &node.data.borrow().value {
        NodeValue::List(list) => (list.list_type, list.start, list.tight),
        _ => return Vec::new(),
    };
    // (is_task, items before this run, converted items)
    let mut runs: Vec<(bool, usize, Vec<Value>)> = Vec::new();
    let mut seen = 0usize;
    for child in node.children() {
        let is_task = matches!(child.data.borrow().value, NodeValue::TaskItem(_));
        let item = convert_node(child, &[]);
        if item.is_null() {
            continue;
        }
        match runs.last_mut() {
            Some((kind, _, items)) if *kind == is_task => items.push(item),
            _ => runs.push((is_task, seen, vec![item])),
        }
        seen += 1;
    }
    runs.into_iter()
        .map(|(is_task, offset, content)| match (is_task, list_type) {
            (true, ListType::Ordered) => json!({
                "type": "task_list",
                "attrs": {"order": start + offset, "tight": tight},
                "content": content
            }),
            (true, ListType::Bullet) => json!({
                "type": "task_list",
                "attrs": {"tight": tight},
                "content": content
            }),
            (false, ListType::Ordered) => json!({
                "type": "ordered_list",
                "attrs": {"order": start + offset, "tight": tight},
                "content": content
            }),
            (false, ListType::Bullet) => json!({
                "type": "bullet_list",
                "attrs": {"tight": tight},
                "content": content
            }),
        })
        .collect()
}

/// Collect list item children of a list node.
fn collect_list_items<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    node.children()
        .map(|child| convert_node(child, &[]))
        .filter(|v| !v.is_null())
        .collect()
}

/// The literal of an `HtmlInline` node, if `node` is one.
fn html_inline_literal<'a>(node: &'a AstNode<'a>) -> Option<String> {
    if let NodeValue::HtmlInline(html) = &node.data.borrow().value {
        Some(html.clone())
    } else {
        None
    }
}

/// Detect the split sized-linked-image sequence at the head of `children`
/// (#893): an `<a href="...">` `HtmlInline`, an `<img ... />` `HtmlInline`,
/// and the closing `</a>` `HtmlInline`. Comrak splits the HTML form the
/// save direction emits for a sized linked image into those three sibling
/// nodes; merged here they become one image node carrying `linkHref`, and
/// neither anchor piece falls through as literal text. The closing tag is
/// optional in the match (consumed when present) so a missing one cannot
/// strand the opening tag as text either. Returns the merged image node
/// and how many siblings it consumed.
fn linked_image_sequence<'a>(children: &[&'a AstNode<'a>]) -> Option<(Value, usize)> {
    if children.len() < 2 {
        return None;
    }
    let (href, link_title) = parse_anchor_tag(&html_inline_literal(children[0])?)?;
    let img = parse_img_tag(&html_inline_literal(children[1])?)?;
    let consumed = if children.len() >= 3
        && html_inline_literal(children[2]).is_some_and(|h| h.trim() == "</a>")
    {
        3
    } else {
        2
    };
    Some((image_with_link(img, &href, link_title.as_deref()), consumed))
}

/// Collect inline children of a node, flattening nested mark wrappers.
fn collect_inline_children<'a>(node: &'a AstNode<'a>, active_marks: &[Mark]) -> Vec<Value> {
    let mut result = Vec::new();
    let children: Vec<&AstNode<'a>> = node.children().collect();
    let mut index = 0;

    while index < children.len() {
        // #893: merge a split <a><img /></a> sequence before the pieces
        // are converted individually (see linked_image_sequence).
        if let Some((img, consumed)) = linked_image_sequence(&children[index..]) {
            result.push(img);
            index += consumed;
            continue;
        }

        let child = children[index];
        index += 1;
        let child_data = child.data.borrow();

        // Check if this is a mark wrapper (Emph, Strong, Link)
        let is_mark_wrapper = matches!(
            child_data.value,
            NodeValue::Emph | NodeValue::Strong | NodeValue::Link(_) | NodeValue::Strikethrough
        );
        drop(child_data);

        let converted = convert_node(child, active_marks);
        if is_mark_wrapper {
            // Convert returns an array of children with marks applied
            if let Some(arr) = converted.as_array() {
                for item in arr {
                    if !item.is_null() {
                        result.push(item.clone());
                    }
                }
            }
        } else if !converted.is_null() {
            result.push(converted);
        }
    }

    result
}

/// Get all text content from a node and its descendants.
fn get_text_content<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    collect_text(node, &mut text);
    text
}

/// Recursively collect text from a node.
fn collect_text<'a>(node: &'a AstNode<'a>, text: &mut String) {
    let data = node.data.borrow();

    match &data.value {
        NodeValue::Text(t) => text.push_str(t.as_ref()),
        NodeValue::Code(c) => text.push_str(&c.literal),
        NodeValue::SoftBreak => text.push(' '),
        NodeValue::LineBreak => text.push('\n'),
        _ => {
            drop(data);
            for child in node.children() {
                collect_text(child, text);
            }
        }
    }
}

/// #1417: serialize an image alt subtree back to markdown form.
///
/// The inverse of the save direction's alt re-parse: emphasis, strong,
/// strikethrough, code spans and links keep their markup; literal text
/// is escaped so characters that were literal in the source (e.g. an
/// escaped `\\*`) stay literal when the stored form is re-parsed on
/// save. Nested images/links inside an alt are serialized recursively,
/// so a nested-link alt round-trips instead of flattening.
fn get_alt_markdown<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    for child in node.children() {
        collect_alt_markdown(child, &mut out);
    }
    out
}

fn collect_alt_markdown<'a>(node: &'a AstNode<'a>, out: &mut String) {
    let data = node.data.borrow();
    match &data.value {
        NodeValue::Text(t) => {
            for ch in t.chars() {
                if matches!(ch, '\\' | '*' | '_' | '`' | '[' | ']') {
                    out.push('\\');
                }
                out.push(ch);
            }
        }
        NodeValue::Code(c) => {
            // Fence with one more backtick than the longest run inside.
            let longest = c
                .literal
                .split(|ch| ch != '`')
                .map(str::len)
                .max()
                .unwrap_or(0);
            let fence = "`".repeat(longest + 1);
            out.push_str(&fence);
            if c.literal.starts_with('`') || c.literal.ends_with('`') {
                out.push(' ');
                out.push_str(&c.literal);
                out.push(' ');
            } else {
                out.push_str(&c.literal);
            }
            out.push_str(&fence);
        }
        NodeValue::SoftBreak => out.push(' '),
        NodeValue::LineBreak => out.push('\n'),
        NodeValue::HtmlInline(html) => out.push_str(html),
        NodeValue::Emph => {
            drop(data);
            out.push('*');
            for child in node.children() {
                collect_alt_markdown(child, out);
            }
            out.push('*');
        }
        NodeValue::Strong => {
            drop(data);
            out.push_str("**");
            for child in node.children() {
                collect_alt_markdown(child, out);
            }
            out.push_str("**");
        }
        NodeValue::Strikethrough => {
            drop(data);
            out.push_str("~~");
            for child in node.children() {
                collect_alt_markdown(child, out);
            }
            out.push_str("~~");
        }
        NodeValue::Link(link) => {
            let url = link.url.clone();
            let title = link.title.clone();
            drop(data);
            out.push('[');
            for child in node.children() {
                collect_alt_markdown(child, out);
            }
            out.push_str("](");
            out.push_str(&url);
            if !title.is_empty() {
                out.push_str(" \"");
                out.push_str(&title.replace('"', "\\\""));
                out.push_str("\"");
            }
            out.push(')');
        }
        NodeValue::Image(link) => {
            let url = link.url.clone();
            let title = link.title.clone();
            drop(data);
            out.push_str("![");
            for child in node.children() {
                collect_alt_markdown(child, out);
            }
            out.push_str("](");
            out.push_str(&url);
            if !title.is_empty() {
                out.push_str(" \"");
                out.push_str(&title.replace('"', "\\\""));
                out.push_str("\"");
            }
            out.push(')');
        }
        _ => {
            drop(data);
            for child in node.children() {
                collect_alt_markdown(child, out);
            }
        }
    }
}

/// #1417: re-parse a stored markdown-form alt into inline AST nodes in
/// `arena` (the save direction). Returns the paragraph's children,
/// detached and ready to append under an Image node. Falls back to a
/// single plain Text node when the alt does not parse to a paragraph
/// (defensive; plain alts parse to exactly that anyway).
fn alt_markdown_to_nodes<'a>(arena: &'a Arena<'a>, alt: &str) -> Vec<&'a AstNode<'a>> {
    let options = commonmark_options();
    let root = parse_document(arena, alt, &options);
    if let Some(paragraph) = root
        .children()
        .find(|n| matches!(n.data.borrow().value, NodeValue::Paragraph))
    {
        let nodes: Vec<&'a AstNode<'a>> = paragraph.children().collect();
        for n in &nodes {
            n.detach();
        }
        if !nodes.is_empty() {
            return nodes;
        }
    }
    vec![&*arena.alloc(AstNode::from(NodeValue::Text(alt.to_owned().into())))]
}

/// Escape a value for a double-quoted HTML attribute (#873): `&` first, so
/// the entities introduced here are not themselves re-escaped, then the
/// quote that would terminate the value, and `<`.
fn escape_attr_value(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Decode the entities [`escape_attr_value`] can emit (plus `&gt;`/`&apos;`
/// and numeric single-quote) in one left-to-right pass, so `&amp;quot;`
/// decodes to the literal text `&quot;`, never to `"`.
fn decode_attr_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let (decoded, len) = if let Some(t) = tail.strip_prefix("&quot;") {
            ('"', tail.len() - t.len())
        } else if let Some(t) = tail.strip_prefix("&amp;") {
            ('&', tail.len() - t.len())
        } else if let Some(t) = tail.strip_prefix("&lt;") {
            ('<', tail.len() - t.len())
        } else if let Some(t) = tail.strip_prefix("&gt;") {
            ('>', tail.len() - t.len())
        } else if let Some(t) = tail.strip_prefix("&apos;") {
            ('\'', tail.len() - t.len())
        } else if let Some(t) = tail.strip_prefix("&#39;") {
            ('\'', tail.len() - t.len())
        } else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        out.push(decoded);
        rest = &tail[len..];
    }
    out.push_str(rest);
    out
}

/// Parse the attributes of an HTML tag, starting just past its `<name`
/// prefix (`tag_len` bytes: 4 for `<img`, 2 for `<a`).
///
/// Parses attribute-by-attribute instead of searching substrings (#872):
/// a substring search for `src="` also matches `data-src="`, and
/// attribute-looking text inside an earlier quoted value (e.g. `title=`
/// inside `alt="..."`) was extracted as a real attribute. Scanning
/// name=value pairs left to right records whole attribute names only, and
/// a quoted value is consumed as that attribute's value alone. First
/// occurrence of a name wins (HTML duplicate-attribute rule).
fn parse_tag_attrs(html: &str, tag_len: usize) -> std::collections::HashMap<String, String> {
    let bytes = html.as_bytes();
    let mut pos = tag_len;
    let mut map = std::collections::HashMap::new();
    while pos < bytes.len() {
        // Skip separators between attributes (whitespace, '/' of '/>').
        while pos < bytes.len() && (bytes[pos].is_ascii_whitespace() || bytes[pos] == b'/') {
            pos += 1;
        }
        if pos >= bytes.len() || bytes[pos] == b'>' {
            break;
        }
        // Attribute name: up to '=', whitespace, '/' or '>'.
        let name_start = pos;
        while pos < bytes.len()
            && !bytes[pos].is_ascii_whitespace()
            && !matches!(bytes[pos], b'=' | b'/' | b'>')
        {
            pos += 1;
        }
        if pos == name_start {
            break;
        }
        let name = html[name_start..pos].to_ascii_lowercase();
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let mut value = String::new();
        if pos < bytes.len() && bytes[pos] == b'=' {
            pos += 1;
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos < bytes.len() && matches!(bytes[pos], b'"' | b'\'') {
                let quote = bytes[pos];
                pos += 1;
                let value_start = pos;
                while pos < bytes.len() && bytes[pos] != quote {
                    pos += 1;
                }
                value = decode_attr_entities(&html[value_start..pos]);
                if pos < bytes.len() {
                    pos += 1; // closing quote
                }
            } else {
                let value_start = pos;
                while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' {
                    pos += 1;
                }
                value = decode_attr_entities(&html[value_start..pos]);
            }
        }
        // First occurrence wins (HTML duplicate-attribute rule).
        map.entry(name).or_insert(value);
    }
    map
}

/// Parse an `<img>` width/height value into a pixel count (#2180,
/// grammar pinned by triage task 3225). The whole stack is
/// numeric-pixel -- the PM node stores a JSON number, the emitter
/// writes bare integers, the editor renders `${width}px` -- so only
/// lossless normalizations are accepted: trim surrounding
/// whitespace, strip ONE trailing `px` suffix (ASCII
/// case-insensitive), then the rest must be a pixel integer, where a
/// decimal form counts only when its value is exactly integral
/// (`200.0` -> 200, `200.5` refused). Percents, other units, and
/// junk return `None`; the caller warns instead of dropping the
/// value silently, as the pre-#2180 bare-`u64` parse did.
fn parse_integral_decimal(raw: &str) -> Option<u64> {
    let (negative, raw) = if let Some(raw) = raw.strip_prefix('-') {
        (true, raw)
    } else {
        (false, raw.strip_prefix('+').unwrap_or(raw))
    };

    let (mantissa, exponent) = if let Some(index) = raw.find(|ch| matches!(ch, 'e' | 'E')) {
        (&raw[..index], raw[index + 1..].parse::<i32>().ok()?)
    } else {
        (raw, 0)
    };
    let (integer, fractional) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() && fractional.is_empty() {
        return None;
    }
    if !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let mut digits = String::with_capacity(integer.len() + fractional.len());
    digits.push_str(integer);
    digits.push_str(fractional);
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        // Keep the exact zero spellings accepted by the former float parser,
        // including -0.0, without converting through f64.
        return Some(0);
    }

    let fractional_digits = i32::try_from(fractional.len()).ok()?;
    let scale = exponent.checked_sub(fractional_digits)?;
    let integer = if scale >= 0 {
        let trailing_zeroes = usize::try_from(scale).ok()?;
        if significant.len().checked_add(trailing_zeroes)? > 20 {
            return None;
        }
        let mut integer = String::with_capacity(significant.len() + trailing_zeroes);
        integer.push_str(significant);
        integer.push_str(&"0".repeat(trailing_zeroes));
        integer.parse::<u64>().ok()?
    } else {
        let trailing_zeroes = usize::try_from(scale.checked_neg()?).ok()?;
        if trailing_zeroes > significant.len() {
            return None;
        }
        let split = significant.len() - trailing_zeroes;
        if significant[split..].bytes().any(|byte| byte != b'0') {
            return None;
        }
        significant[..split].parse::<u64>().ok()?
    };

    if negative && integer != 0 {
        return None;
    }
    Some(integer)
}

fn parse_dimension(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    // Byte-wise suffix check: slicing the string at len()-2 to test
    // the suffix would panic when that offset falls inside a multibyte
    // UTF-8 character (e.g. fullwidth digits). When the last two bytes
    // are the ASCII bytes p/x, len()-2 is a char boundary, so the
    // slice below is safe.
    let bytes = trimmed.as_bytes();
    let body = if bytes.len() >= 2 && bytes[bytes.len() - 2..].eq_ignore_ascii_case(b"px") {
        &trimmed[..trimmed.len() - 2]
    } else {
        trimmed
    };
    if let Ok(n) = body.parse::<u64>() {
        return Some(n);
    }
    parse_integral_decimal(body)
}

/// Parse an HTML `<img>` tag and extract attributes into a `ProseMirror` image node.
///
/// Returns `Some(json)` if the input looks like `<img src="..." ... />`, otherwise `None`.
fn parse_img_tag(html: &str) -> Option<Value> {
    let trimmed = html.trim();
    if !trimmed.starts_with("<img ") && !trimmed.starts_with("<img>") {
        return None;
    }

    let parsed = parse_tag_attrs(trimmed, 4);
    let attr = |name: &str| parsed.get(name).map(String::as_str);

    let src = attr("src")?; // src is required
    let mut attrs = json!({"src": src});

    if let Some(alt) = attr("alt") {
        attrs["alt"] = json!(alt);
    }
    if let Some(title) = attr("title") {
        attrs["title"] = json!(title);
    }
    if let Some(w) = attr("width") {
        match parse_dimension(w) {
            Some(n) => attrs["width"] = json!(n),
            None => tracing::warn!("dropping unsupported image width {w:?} in <img> import"),
        }
    }
    if let Some(h) = attr("height") {
        match parse_dimension(h) {
            Some(n) => attrs["height"] = json!(n),
            None => tracing::warn!("dropping unsupported image height {h:?} in <img> import"),
        }
    }

    Some(json!({
        "type": "image",
        "attrs": attrs
    }))
}

/// Parse an HTML `<a ...>` opening tag, returning its `href` and optional
/// `title`. Returns `None` for anything else, including an anchor with no
/// (or an empty) `href`.
fn parse_anchor_tag(html: &str) -> Option<(String, Option<String>)> {
    let trimmed = html.trim();
    let after_name = trimmed.as_bytes().get(2).copied()?;
    if !trimmed.starts_with("<a") || !(after_name.is_ascii_whitespace() || after_name == b'>') {
        return None;
    }
    let parsed = parse_tag_attrs(trimmed, 2);
    let href = parsed.get("href")?;
    if href.is_empty() {
        return None;
    }
    let title = parsed.get("title").filter(|t| !t.is_empty()).cloned();
    Some((href.clone(), title))
}

/// Attach a wrapping anchor's link to an image node (#880/#893), in the
/// same `linkHref`/`linkTitle` attrs the markdown `Image` arm produces.
fn image_with_link(mut img: Value, href: &str, link_title: Option<&str>) -> Value {
    img["attrs"]["linkHref"] = json!(href);
    if let Some(title) = link_title {
        img["attrs"]["linkTitle"] = json!(title);
    }
    img
}

/// Parse the combined `<a href="..."><img ... /></a>` form (#893) — the
/// sized linked image emission when comrak keeps it in a single literal
/// (e.g. an `HtmlBlock`) instead of splitting it into sibling `HtmlInline`
/// nodes (that split form is merged in `collect_inline_children`).
fn parse_linked_img_tag(html: &str) -> Option<Value> {
    let trimmed = html.trim();
    let (href, link_title) = parse_anchor_tag(trimmed)?;
    let img_start = trimmed.find("<img")?;
    if !trimmed[..img_start].trim_end().ends_with('>') || !trimmed[img_start..].contains("</a>") {
        return None;
    }
    let img = parse_img_tag(&trimmed[img_start..])?;
    Some(image_with_link(img, &href, link_title.as_deref()))
}

/// Placement categories for the #1043 validation walk. A node whose type
/// is not listed here is left to `json_to_ast`, which reports it as
/// `ConversionError::UnknownType` — validation never masks that error.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// Block-level node (`paragraph`, `table`, lists, …).
    Block,
    /// Inline node (`text`, `image`, `hard_break`, …).
    Inline,
    /// `list_item` — only legal directly inside a bullet/ordered list.
    ListItem,
    /// `task_list_item` — only legal directly inside a `task_list`.
    TaskListItem,
    /// `table_row` — only legal directly inside a `table`.
    TableRow,
    /// `table_header` / `table_cell` — only legal inside a `table_row`.
    TableCell,
}

fn placement_of(node_type: &str) -> Option<Placement> {
    match node_type {
        "paragraph"
        | "heading"
        | "blockquote"
        | "code_block"
        | "horizontal_rule"
        | "bullet_list"
        | "ordered_list"
        | "task_list"
        | "table"
        | "footnote_definition" => Some(Placement::Block),
        "text" | "image" | "hard_break" | "html_inline" | "footnote_reference" => {
            Some(Placement::Inline)
        }
        "list_item" => Some(Placement::ListItem),
        "task_list_item" => Some(Placement::TaskListItem),
        "table_row" => Some(Placement::TableRow),
        "table_header" | "table_cell" => Some(Placement::TableCell),
        _ => None,
    }
}

/// Whether a child of category `child` may appear directly inside a
/// parent of type `parent`. Mirrors the comrak AST containment rules for
/// the node kinds this codec emits, at the `ProseMirror` level.
fn placement_allowed(parent: &str, child: Placement) -> bool {
    match parent {
        // The document and block containers hold blocks only.
        "doc" | "blockquote" | "list_item" | "task_list_item" => child == Placement::Block,
        // Inline containers hold inline content only.
        "paragraph" | "heading" => child == Placement::Inline,
        "bullet_list" | "ordered_list" => child == Placement::ListItem,
        "task_list" => child == Placement::TaskListItem,
        "table" => child == Placement::TableRow,
        "table_row" => child == Placement::TableCell,
        // PM table cells wrap blocks (unwrapped during conversion) and
        // footnote definitions carry inline content on the PM side
        // (re-wrapped during conversion, #945); both shapes are legal.
        "table_header" | "table_cell" | "footnote_definition" => {
            child == Placement::Block || child == Placement::Inline
        }
        "code_block" => false,
        // Inline leaves and anything unlisted impose no child rules here;
        // structural surprises inside them are json_to_ast's to report.
        _ => true,
    }
}

/// Validate block/inline placement over a `ProseMirror` document before
/// conversion (#1043). `json_to_ast` places any known node type anywhere;
/// a misplaced node (a `text` node directly under `doc`, a `paragraph`
/// inside a `paragraph`, …) produces an ill-formed comrak AST that
/// panics the formatter in debug builds and silently emits invalid
/// markdown in release builds. Reject those documents up front with
/// `ConversionError::MisplacedNode` instead.
fn validate_placement(doc: &Value) -> Result<(), ConversionError> {
    fn walk(node: &Value, parent: Option<&str>) -> Result<(), ConversionError> {
        let Some(node_type) = node["type"].as_str() else {
            // Missing type: json_to_ast reports MissingType; keep that
            // error's precedence.
            return Ok(());
        };
        if let Some(parent_type) = parent
            && let Some(category) = placement_of(node_type)
            && !placement_allowed(parent_type, category)
        {
            return Err(ConversionError::MisplacedNode {
                parent: parent_type.to_owned(),
                child: node_type.to_owned(),
            });
        }
        // code_block content is literal text children only.
        if node_type == "code_block" {
            if let Some(content) = node["content"].as_array() {
                for child in content {
                    if child["type"].as_str() != Some("text") {
                        return Err(ConversionError::MisplacedNode {
                            parent: node_type.to_owned(),
                            child: child["type"].as_str().unwrap_or("<missing>").to_owned(),
                        });
                    }
                }
            }
            return Ok(());
        }
        if let Some(content) = node["content"].as_array() {
            for child in content {
                walk(child, Some(node_type))?;
            }
        }
        Ok(())
    }
    walk(doc, None)
}

/// Convert `ProseMirror` JSON document to markdown text.
///
/// # Arguments
///
/// * `doc` - The `ProseMirror` JSON document.
///
/// # Returns
///
/// The markdown text, or an error if conversion fails.
///
/// # Errors
///
/// Returns `ConversionError` if the JSON structure is invalid.
pub fn prosemirror_to_markdown(doc: &Value) -> Result<String, ConversionError> {
    validate_placement(doc)?;
    let arena = Arena::new();
    let root = json_to_ast(&arena, doc)?;
    // #1418: hoist delimiter-mark edge spaces that a word character
    // touches out of their wrappers, before rendering (see
    // `hoist_delimiter_edge_spaces`).
    hoist_delimiter_edge_spaces(&arena, root);
    // #1496: force info-less code blocks inside blockquotes onto the
    // renderer's fenced branch via a placeholder info string, stripped
    // from the rendered output below (see
    // `force_fenced_code_blocks_in_blockquotes`).
    let fence_tokens = force_fenced_code_blocks_in_blockquotes(root);
    // #1019: comrak's renderer percent-encodes a space in a link/image
    // destination (`my notes.md` -> `my%20notes.md`) instead of emitting
    // the <...> angle form, silently breaking relative file links and
    // mutating the PM href on reparse. #1423: the same mangling hits the
    // other whitespace characters — a tab is encoded to %09 and a newline
    // destroys the link outright (`% `). The renderer has no angle-form
    // option, so swap each whitespace-carrying destination for a unique
    // placeholder before rendering and restore it as <raw destination>
    // afterwards.
    let substitutions = substitute_spaced_destinations(root);
    let mut output = String::new();
    format_commonmark(root, &commonmark_options(), &mut output)?;
    for (placeholder, raw) in substitutions {
        output = output.replace(&placeholder, &format!("<{raw}>"));
    }
    for token in fence_tokens {
        output = strip_fence_placeholder(&output, &token);
    }
    Ok(output)
}

/// Whether a node value is a delimiter-mark wrapper (#1418): the
/// marks whose markdown spelling is a delimiter run (`*`/`**`/`~~`)
/// subject to CommonMark flanking rules. Link (bracket syntax) and
/// code (backtick syntax) tolerate edge spaces and are excluded.
fn is_delimiter_wrapper(value: &NodeValue) -> bool {
    matches!(
        value,
        NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough
    )
}

/// The first text leaf of a mark chain, descending first children
/// through mark wrappers (#1418). `None` when the first inline is
/// anything else (an image or html node is a different mechanism —
/// #1407/#1412 — and keeps the legacy behaviour).
fn first_text_leaf<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
    let mut current = node;
    loop {
        let child = current.first_child()?;
        let value = child.data.borrow().value.clone();
        match value {
            NodeValue::Text(_) => return Some(child),
            _ if is_mark_wrapper(&value) => current = child,
            _ => return None,
        }
    }
}

/// The last text leaf of a mark chain; mirror of `first_text_leaf`.
fn last_text_leaf<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
    let mut current = node;
    loop {
        let child = current.last_child()?;
        let value = child.data.borrow().value.clone();
        match value {
            NodeValue::Text(_) => return Some(child),
            _ if is_mark_wrapper(&value) => current = child,
            _ => return None,
        }
    }
}

/// The last character a node renders as inline content (#1418):
/// text/code literals verbatim, breaks as whitespace, mark wrappers
/// by their last inline descendant. Anything else yields `None`
/// (treated as a non-word neighbour).
fn last_rendered_char(node: &AstNode<'_>) -> Option<char> {
    match &node.data.borrow().value {
        NodeValue::Text(literal) => literal.to_string().chars().last(),
        NodeValue::Code(code) => code.literal.chars().last(),
        NodeValue::LineBreak | NodeValue::SoftBreak => Some('\n'),
        value if is_mark_wrapper(value) => node.last_child().and_then(last_rendered_char),
        _ => None,
    }
}

/// The first character a node renders as; mirror of
/// `last_rendered_char`.
fn first_rendered_char(node: &AstNode<'_>) -> Option<char> {
    match &node.data.borrow().value {
        NodeValue::Text(literal) => literal.to_string().chars().next(),
        NodeValue::Code(code) => code.literal.chars().next(),
        NodeValue::LineBreak | NodeValue::SoftBreak => Some('\n'),
        value if is_mark_wrapper(value) => node.first_child().and_then(first_rendered_char),
        _ => None,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether the inline character immediately before `node` is a word
/// character (#1418), climbing through mark-wrapper ancestors while
/// `node` sits at their leading edge. No rendered neighbour (block
/// boundary) is not a word character.
fn preceded_by_word_char(node: &AstNode<'_>) -> bool {
    let mut current = node;
    loop {
        if let Some(prev) = current.previous_sibling() {
            return last_rendered_char(prev).is_some_and(is_word_char);
        }
        match current.parent() {
            Some(parent) if is_mark_wrapper(&parent.data.borrow().value) => current = parent,
            _ => return false,
        }
    }
}

/// Whether the inline character immediately after `node` is a word
/// character; mirror of `preceded_by_word_char`.
fn followed_by_word_char(node: &AstNode<'_>) -> bool {
    let mut current = node;
    loop {
        if let Some(next) = current.next_sibling() {
            return first_rendered_char(next).is_some_and(is_word_char);
        }
        match current.parent() {
            Some(parent) if is_mark_wrapper(&parent.data.borrow().value) => current = parent,
            _ => return false,
        }
    }
}

/// Replace a text leaf's literal, detaching the leaf (and any mark
/// wrappers it leaves childless) when the replacement is empty
/// (#1418: a span whose whole text was edge spaces).
fn set_text_leaf_text(leaf: &AstNode<'_>, new_text: &str) {
    if !new_text.is_empty() {
        leaf.data.borrow_mut().value = NodeValue::Text(new_text.to_owned().into());
        return;
    }
    let mut current = leaf.parent();
    leaf.detach();
    while let Some(node) = current {
        if !is_mark_wrapper(&node.data.borrow().value) || node.first_child().is_some() {
            break;
        }
        current = node.parent();
        node.detach();
    }
}

/// Hoist one edge of one delimiter wrapper (#1418): move its leading
/// (or trailing) spaces to a plain text node just outside the wrapper
/// — but still inside any outer non-delimiter wrapper such as a link,
/// which is exactly the sibling position next to this wrapper — when
/// a word character touches the wrapper on that side.
fn hoist_one_edge<'a>(arena: &'a Arena<'a>, wrapper: &'a AstNode<'a>, leading: bool) {
    let touches_word = if leading {
        preceded_by_word_char(wrapper)
    } else {
        followed_by_word_char(wrapper)
    };
    if !touches_word {
        return;
    }
    let Some(leaf) = (if leading {
        first_text_leaf(wrapper)
    } else {
        last_text_leaf(wrapper)
    }) else {
        return;
    };
    let text = match &leaf.data.borrow().value {
        NodeValue::Text(literal) => literal.to_string(),
        _ => return,
    };
    let spaces_len = if leading {
        text.len() - text.trim_start_matches(' ').len()
    } else {
        text.len() - text.trim_end_matches(' ').len()
    };
    if spaces_len == 0 {
        return;
    }
    let (spaces, rest) = if leading {
        (text[..spaces_len].to_owned(), text[spaces_len..].to_owned())
    } else {
        (
            text[text.len() - spaces_len..].to_owned(),
            text[..text.len() - spaces_len].to_owned(),
        )
    };
    // Insert the hoisted spaces while the wrapper is still attached:
    // shrinking the leaf afterwards may detach the wrapper itself
    // (a span whose whole text was edge spaces), and the space node
    // must stay at the wrapper's position when that happens.
    let space_node = arena.alloc(AstNode::from(NodeValue::Text(spaces.into())));
    if leading {
        wrapper.insert_before(space_node);
    } else {
        wrapper.insert_after(space_node);
    }
    set_text_leaf_text(leaf, &rest);
}

/// #1418: a delimiter-mark span whose text starts or ends with a
/// space saves with that space inside the delimiters, and comrak
/// renders it as an `&#32;` entity (`a*&#32;b*`). When a word
/// character touches the span on that side, the emitted form fails
/// CommonMark flanking rules on reparse and the mark is destroyed.
/// Hoist such edge spaces out of the delimiter wrappers so the saved
/// form reparses with the mark intact; the space becomes unmarked,
/// which renders identically. Edges a word character does not touch
/// (paragraph boundaries, space/punctuation neighbours, spans padded
/// on both edges) keep the `&#32;` form, which round-trips as-is.
/// Working on the converted AST — after #968's chain merging — means
/// only true span edges are considered: an edge space shared with a
/// neighbouring run of the same span is interior here, not an edge.
fn hoist_delimiter_edge_spaces<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>) {
    let wrappers: Vec<&AstNode<'a>> = root
        .descendants()
        .filter(|node| is_delimiter_wrapper(&node.data.borrow().value))
        .collect();
    // Reverse document order: inner wrappers before their outers (a
    // space hoisted out of an inner wrapper may still sit at the
    // outer wrapper's edge and be hoisted again by its turn), and a
    // wrapper's own leading edge before its trailing edge.
    for wrapper in wrappers.into_iter().rev() {
        hoist_one_edge(arena, wrapper, true);
        hoist_one_edge(arena, wrapper, false);
    }
}

/// #1496: comrak's CommonMark renderer emits an info-less code block
/// as an INDENTED block whenever it can (its fenced branch requires a
/// non-empty info string, the global `prefer_fenced` option, or a few
/// literal edge cases). Inside a blockquote the indented form drops
/// the quote prefix for every following sibling, so the saved file
/// reparses as two blockquotes and everything after the code block
/// leaves the quote. Give each info-less code block inside a
/// blockquote a placeholder info string so the renderer takes the
/// fenced branch, and return the placeholders for stripping from the
/// rendered output — the same placeholder-and-restore pattern as
/// `substitute_spaced_destinations`. Code blocks outside blockquotes
/// keep the renderer's own spelling (the lone-block fence/indent
/// choice is #954's territory, deliberately untouched here).
fn force_fenced_code_blocks_in_blockquotes<'a>(root: &'a AstNode<'a>) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut stack: Vec<(&AstNode<'a>, bool)> = vec![(root, false)];
    while let Some((node, ancestor_is_blockquote)) = stack.pop() {
        let in_blockquote =
            ancestor_is_blockquote || matches!(node.data.borrow().value, NodeValue::BlockQuote);
        for child in node.children() {
            stack.push((child, in_blockquote));
        }
        if !in_blockquote {
            continue;
        }
        let mut data = node.data.borrow_mut();
        if let NodeValue::CodeBlock(code_block) = &mut data.value
            && code_block.info.is_empty()
        {
            // The trailing `-end` keeps placeholders prefix-free
            // (`-1-end` is not a prefix of `-10-end`); restoration is
            // positional regardless (see `strip_fence_placeholder`).
            let token = format!("idweb-blockquote-fence-{}-end", tokens.len());
            code_block.info.clone_from(&token);
            tokens.push(token);
        }
    }
    tokens
}

/// Remove a #1496 fence placeholder from rendered output — ONLY at
/// its generated fence-info position: a run of >= 3 backticks
/// immediately before the placeholder and a newline immediately after
/// it (the renderer's fence line shape). A plain `str::replace` would
/// also strip the placeholder text anywhere else it occurs (user prose,
/// code content) and, with prefix-sharing placeholders, corrupt
/// neighbouring blocks' info strings (Codex review on PR #3770).
fn strip_fence_placeholder(output: &str, token: &str) -> String {
    let mut result = String::with_capacity(output.len());
    let mut rest = output;
    while let Some(pos) = rest.find(token) {
        let before = &rest[..pos];
        let after = &rest[pos + token.len()..];
        let fence_run = before.chars().rev().take_while(|c| *c == '`').count();
        if fence_run >= 3 && after.starts_with('\n') {
            result.push_str(before);
            rest = after;
        } else {
            result.push_str(&rest[..pos + token.len()]);
            rest = after;
        }
    }
    result.push_str(rest);
    result
}

/// Replace every link/image destination containing whitespace (space,
/// tab, newline, …) with a unique placeholder token, returning
/// (placeholder, raw destination) pairs for restoration after rendering
/// (#1019, #1423). The token is a valid bare destination (no whitespace
/// or characters comrak encodes), so the renderer emits it verbatim
/// exactly where the destination belongs. Reference-style links flow
/// through here too: comrak resolves a reference definition into the
/// referencing link's `url`, and the PM model stores that href directly,
/// so no separate definition path exists in this codec.
fn substitute_spaced_destinations<'a>(root: &'a AstNode<'a>) -> Vec<(String, String)> {
    let mut substitutions: Vec<(String, String)> = Vec::new();
    let mut stack: Vec<&AstNode<'a>> = vec![root];
    while let Some(node) = stack.pop() {
        stack.extend(node.children());
        let mut data = node.data.borrow_mut();
        let url = match &mut data.value {
            NodeValue::Link(link) | NodeValue::Image(link) => &mut link.url,
            _ => continue,
        };
        if !url.contains(char::is_whitespace) {
            continue;
        }
        if let Some(index) = substitutions
            .iter()
            .position(|(_, raw)| raw.as_str() == url.as_str())
        {
            *url = substitutions[index].0.clone();
        } else {
            let placeholder = format!(
                "idweb-spaced-destination-{}-placeholder",
                substitutions.len()
            );
            let raw = std::mem::take(url);
            *url = placeholder.clone();
            substitutions.push((placeholder, raw));
        }
    }
    substitutions
}

/// Whether a converted AST node is block-level (as opposed to inline).
/// Used by the footnote_definition save path (#1031) to tell carried
/// block children apart from legacy inline children.
fn is_block_node(node: &AstNode<'_>) -> bool {
    matches!(
        node.data.borrow().value,
        NodeValue::Paragraph
            | NodeValue::Heading(_)
            | NodeValue::BlockQuote
            | NodeValue::List(_)
            | NodeValue::Item(_)
            | NodeValue::TaskItem(_)
            | NodeValue::CodeBlock(_)
            | NodeValue::HtmlBlock(_)
            | NodeValue::ThematicBreak
            | NodeValue::Table(_)
            | NodeValue::FootnoteDefinition(_)
    )
}

/// Convert `ProseMirror` JSON to comrak AST.
/// Whether a JSON node is a `text` node carrying no marks (#1420).
/// Marked runs protect their edge spaces with the marked-span mechanism
/// (#1418); unmarked runs have no such protection, so the save direction
/// normalizes their edge whitespace instead (see
/// `normalize_inline_child_json`).
fn is_unmarked_text_json(node: &Value) -> bool {
    node["type"].as_str() == Some("text")
        && node["marks"]
            .as_array()
            .is_none_or(|marks| marks.is_empty())
}

/// Whether a JSON node is a `hard_break` — the boundary between two
/// rendered lines inside one block (#1420).
fn is_hard_break_json(node: &Value) -> bool {
    node["type"].as_str() == Some("hard_break")
}

/// Whether an unmarked text node renders as nothing but edge whitespace
/// (empty, or spaces/tabs only). Such a node never shields a neighbour's
/// edge from the line boundary: at a line edge its own whitespace is
/// normalized away too, and mid-line it is a separator either way (#1420).
fn is_whitespace_only_text_json(node: &Value) -> bool {
    is_unmarked_text_json(node)
        && node["text"]
            .as_str()
            .unwrap_or("")
            .chars()
            .all(|c| c == ' ' || c == '\t')
}

/// Whether scanning `siblings` (in render order) reaches a line edge —
/// the start/end of the block or a `hard_break` — before any content
/// that renders something (#1420).
fn reaches_line_edge(siblings: impl Iterator<Item = Value>) -> bool {
    for sibling in siblings {
        if is_hard_break_json(&sibling) {
            return true;
        }
        if !is_whitespace_only_text_json(&sibling) {
            return false;
        }
    }
    true
}

/// #1420: normalize one inline child of a block before conversion.
///
/// Unmarked text runs are written verbatim on save, but CommonMark
/// reparse strips edge whitespace (up to 3 leading spaces per line,
/// trailing spaces ignored) and deletes a whitespace-only paragraph
/// outright, so the saved file and the reopened doc disagreed. The
/// pinned policy is normalize-at-save: trim an unmarked run's leading
/// spaces when it sits at a line start (block start or just after a
/// `hard_break`) and its trailing spaces when it sits at a line end
/// (block end or just before a `hard_break`), so the first saved file
/// is already the fixpoint. A whitespace-only run at a line edge
/// becomes an empty text node, so a whitespace-only paragraph saves as
/// an EMPTY paragraph instead of vanishing text. Interior runs (word
/// separators between words/marks) and marked runs are untouched.
fn normalize_inline_child_json(content: &[Value], idx: usize) -> std::borrow::Cow<'_, Value> {
    let child = &content[idx];
    if !is_unmarked_text_json(child) {
        return std::borrow::Cow::Borrowed(child);
    }
    let trim_start = reaches_line_edge(content[..idx].iter().rev().cloned());
    let trim_end = reaches_line_edge(content[idx + 1..].iter().cloned());
    if !trim_start && !trim_end {
        return std::borrow::Cow::Borrowed(child);
    }
    let text = child["text"].as_str().unwrap_or("");
    let mut normalized = text;
    if trim_start {
        normalized = normalized.trim_start_matches([' ', '\t']);
    }
    if trim_end {
        normalized = normalized.trim_end_matches([' ', '\t']);
    }
    if normalized == text {
        return std::borrow::Cow::Borrowed(child);
    }
    let mut owned = child.clone();
    owned["text"] = json!(normalized);
    std::borrow::Cow::Owned(owned)
}

fn json_to_ast<'a>(arena: &'a Arena<'a>, json: &Value) -> Result<&'a AstNode<'a>, ConversionError> {
    let node_type = json["type"].as_str().ok_or(ConversionError::MissingType)?;

    let node_value = match node_type {
        "doc" => NodeValue::Document,

        "paragraph" => NodeValue::Paragraph,

        "heading" => {
            #[allow(clippy::cast_possible_truncation)] // level is clamped to 1-6
            let level = json["attrs"]["level"].as_u64().unwrap_or(1) as u8;
            NodeValue::Heading(comrak::nodes::NodeHeading {
                level: level.clamp(1, 6),
                setext: false,
                closed: false,
            })
        }

        "blockquote" => NodeValue::BlockQuote,

        "code_block" => {
            // #882: editors/collab can split a code block into several text
            // nodes; concatenate ALL of them instead of keeping only the
            // first (which silently truncated the block on save).
            let content = json["content"].as_array().map_or(String::new(), |arr| {
                arr.iter()
                    .filter_map(|n| n["text"].as_str())
                    .collect::<String>()
            });
            let content = content.as_str();
            let info = json["attrs"]["params"].as_str().unwrap_or("");

            // #982: a code_block carrying the htmlBlock provenance attr
            // (set by the parse direction for blocks converted from raw
            // HTML) emits as a real HTML block again, not a ```html
            // fence. Authored fences and already-saved docs have no
            // attr and keep the fenced emit below — no retro-guessing.
            if info == "html" && json["attrs"]["htmlBlock"].as_bool() == Some(true) {
                return Ok(arena.alloc(AstNode::from(NodeValue::HtmlBlock(
                    comrak::nodes::NodeHtmlBlock {
                        literal: format!("{content}\n"),
                        ..Default::default()
                    },
                ))));
            }

            NodeValue::CodeBlock(Box::new(comrak::nodes::NodeCodeBlock {
                fenced: true,
                fence_char: b'`',
                fence_length: 3,
                fence_offset: 0,
                info: info.to_owned(),
                literal: format!("{content}\n"),
                closed: false,
            }))
        }

        "horizontal_rule" => NodeValue::ThematicBreak,

        "bullet_list" => {
            let tight = json["attrs"]["tight"].as_bool().unwrap_or(false);
            NodeValue::List(comrak::nodes::NodeList {
                list_type: ListType::Bullet,
                tight,
                bullet_char: b'-',
                ..Default::default()
            })
        }

        "ordered_list" => {
            #[allow(clippy::cast_possible_truncation)] // order is typically small
            let order = json["attrs"]["order"].as_u64().unwrap_or(1) as usize;
            let tight = json["attrs"]["tight"].as_bool().unwrap_or(false);
            NodeValue::List(comrak::nodes::NodeList {
                list_type: ListType::Ordered,
                start: order,
                tight,
                ..Default::default()
            })
        }

        "list_item" => NodeValue::Item(comrak::nodes::NodeList::default()),

        "task_list" => {
            // Task list is a list whose children are TaskItems.
            // #1036: carry `tight` like bullet_list/ordered_list. When the
            // attr is absent (legacy PM docs stored by the old codec, which
            // dropped it), infer it: loose if any task_list_item has more
            // than one block child, else tight — so legacy loose items stop
            // fusing and legacy tight lists render exactly as before.
            // #997: when the parse side stored `attrs.order`, rebuild an
            // ordered list with that start; without it, a bullet list.
            let tight = json["attrs"]["tight"].as_bool().unwrap_or_else(|| {
                !json["content"].as_array().is_some_and(|items| {
                    items.iter().any(|item| {
                        item["content"]
                            .as_array()
                            .is_some_and(|blocks| blocks.len() > 1)
                    })
                })
            });
            #[allow(clippy::cast_possible_truncation)] // order is typically small
            let order = json["attrs"]["order"].as_u64().map(|o| o as usize);
            match order {
                Some(start) => NodeValue::List(comrak::nodes::NodeList {
                    list_type: ListType::Ordered,
                    start,
                    tight,
                    ..Default::default()
                }),
                None => NodeValue::List(comrak::nodes::NodeList {
                    list_type: ListType::Bullet,
                    tight,
                    bullet_char: b'-',
                    ..Default::default()
                }),
            }
        }

        "task_list_item" => {
            let checked = json["attrs"]["checked"].as_bool().unwrap_or(false);
            NodeValue::TaskItem(comrak::nodes::NodeTaskItem {
                symbol: checked.then_some('x'),
                symbol_sourcepos: comrak::nodes::Sourcepos {
                    start: comrak::nodes::LineColumn { line: 0, column: 0 },
                    end: comrak::nodes::LineColumn { line: 0, column: 0 },
                },
            })
        }

        "table" => {
            // Count columns from first row's children, and count rows
            let rows = json["content"].as_array();
            let num_columns = rows
                .and_then(|r| r.first())
                .and_then(|row| row["content"].as_array())
                .map_or(0, Vec::len);
            let num_rows = rows.map_or(0, Vec::len);
            // #881: read back the alignments convert_node stored on the
            // table node; absent/short/unknown entries default to None.
            let mut alignments: Vec<comrak::nodes::TableAlignment> = json["attrs"]["alignments"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|a| match a.as_str() {
                            Some("left") => comrak::nodes::TableAlignment::Left,
                            Some("center") => comrak::nodes::TableAlignment::Center,
                            Some("right") => comrak::nodes::TableAlignment::Right,
                            _ => comrak::nodes::TableAlignment::None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            alignments.resize(num_columns, comrak::nodes::TableAlignment::None);
            alignments.truncate(num_columns);
            NodeValue::Table(Box::new(comrak::nodes::NodeTable {
                alignments,
                num_columns,
                num_rows,
                num_nonempty_cells: 0,
            }))
        }

        "table_row" => {
            // Determine if this is a header row by checking if children are table_header
            let is_header = json["content"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|c| c["type"].as_str())
                .is_some_and(|t| t == "table_header");
            NodeValue::TableRow(is_header)
        }

        "table_header" | "table_cell" => NodeValue::TableCell,

        "image" => {
            let src = json["attrs"]["src"].as_str().unwrap_or("");
            let title = json["attrs"]["title"].as_str().unwrap_or("");
            let alt = json["attrs"]["alt"].as_str().unwrap_or("");
            let width = json["attrs"]["width"].as_u64();
            let height = json["attrs"]["height"].as_u64();

            // If image has explicit dimensions, emit as HTML <img> tag
            if width.is_some() || height.is_some() {
                // #873: escape values — a raw `"` truncated the attribute on
                // reparse and could inject new attributes.
                let mut tag = format!("<img src=\"{}\"", escape_attr_value(src));
                if !alt.is_empty() {
                    tag.push_str(&format!(" alt=\"{}\"", escape_attr_value(alt)));
                }
                if !title.is_empty() {
                    tag.push_str(&format!(" title=\"{}\"", escape_attr_value(title)));
                }
                if let Some(w) = width {
                    tag.push_str(&format!(" width=\"{w}\""));
                }
                if let Some(h) = height {
                    tag.push_str(&format!(" height=\"{h}\""));
                }
                tag.push_str(" />");
                // #880: a sized image cannot use [![..](..)](..) syntax;
                // keep its link as an HTML anchor around the <img> tag.
                // #893: carry linkTitle too, so the parse direction's
                // linkTitle survives the next save as well.
                if let Some(link_href) = json["attrs"]["linkHref"].as_str()
                    && !link_href.is_empty()
                {
                    let mut anchor = format!("<a href=\"{}\"", escape_attr_value(link_href));
                    if let Some(link_title) = json["attrs"]["linkTitle"].as_str()
                        && !link_title.is_empty()
                    {
                        anchor.push_str(&format!(" title=\"{}\"", escape_attr_value(link_title)));
                    }
                    tag = format!("{anchor}>{tag}</a>");
                }
                NodeValue::HtmlInline(tag)
            } else {
                // Standard markdown image syntax
                NodeValue::Image(Box::new(comrak::nodes::NodeLink {
                    url: src.to_owned(),
                    title: title.to_owned(),
                }))
            }
        }

        // #943: raw inline HTML (e.g. <sub>, <sup>, <u>, <kbd>) carried
        // verbatim; comrak renders HtmlInline without escaping.
        "html_inline" => {
            NodeValue::HtmlInline(json["attrs"]["html"].as_str().unwrap_or("").to_owned())
        }

        // #945: footnote reference atom -> comrak FootnoteReference; the
        // commonmark formatter renders it natively as [^label].
        "footnote_reference" => {
            NodeValue::FootnoteReference(Box::new(comrak::nodes::NodeFootnoteReference {
                name: json["attrs"]["label"].as_str().unwrap_or("").to_owned(),
                ..Default::default()
            }))
        }

        // #945: footnote definition block -> comrak FootnoteDefinition.
        // Its content children were stored as inline nodes by the parse
        // direction; the special case below re-wraps them in a Paragraph
        // (comrak requires block children) and the formatter renders them
        // under the [^label]: heading.
        "footnote_definition" => {
            NodeValue::FootnoteDefinition(comrak::nodes::NodeFootnoteDefinition {
                name: json["attrs"]["label"].as_str().unwrap_or("").to_owned(),
                total_references: 0,
            })
        }

        "hard_break" => NodeValue::LineBreak,

        "text" => {
            let text = json["text"].as_str().unwrap_or("");
            // Handle marks by wrapping in appropriate nodes
            if let Some(marks) = json["marks"].as_array() {
                return Ok(create_marked_text(arena, text, marks));
            }
            NodeValue::Text(text.to_owned().into())
        }

        _ => return Err(ConversionError::UnknownType(node_type.to_owned())),
    };

    let ast_node = arena.alloc(AstNode::from(node_value));

    // Special case: table cells need paragraph unwrapping.
    // comrak expects inline content directly in cells, not wrapped in paragraphs.
    // ProseMirror wraps cell content in paragraphs, so we unwrap them here.
    if node_type == "table_header" || node_type == "table_cell" {
        if let Some(content) = json["content"].as_array() {
            for (idx, child_json) in content.iter().enumerate() {
                if child_json["type"].as_str() == Some("paragraph") {
                    // Unwrap: add paragraph's inline children directly to cell
                    if let Some(para_content) = child_json["content"].as_array() {
                        for (inline_idx, _inline_json) in para_content.iter().enumerate() {
                            let normalized = normalize_inline_child_json(para_content, inline_idx);
                            let inline_node = json_to_ast(arena, &normalized)?;
                            append_table_cell_child(arena, ast_node, inline_node);
                        }
                    }
                } else {
                    let normalized = normalize_inline_child_json(content, idx);
                    let child = json_to_ast(arena, &normalized)?;
                    append_table_cell_child(arena, ast_node, child);
                }
            }
        }
        return Ok(ast_node);
    }

    // Special case: footnote definitions. The parse direction stores
    // the definition's BLOCK children on the PM node (#1031), and
    // comrak's AST requires a FootnoteDefinition's children to be
    // blocks — append block children directly. Legacy #945-shape docs
    // store bare inline nodes instead; group consecutive inline
    // children into a single Paragraph, as before, so those documents
    // still save as a well-formed definition.
    if node_type == "footnote_definition" {
        if let Some(content) = json["content"].as_array() {
            let mut paragraph: Option<&AstNode<'a>> = None;
            for (idx, _child_json) in content.iter().enumerate() {
                let normalized = normalize_inline_child_json(content, idx);
                let child = json_to_ast(arena, &normalized)?;
                if is_block_node(child) {
                    paragraph = None;
                    ast_node.append(child);
                } else {
                    let target = match paragraph {
                        Some(p) => p,
                        None => {
                            // `&*` reborrows the arena's `&mut` as a shared
                            // reference so `p` stays Copy-usable after
                            // being stored in `paragraph`.
                            let p: &'a AstNode<'a> =
                                &*arena.alloc(AstNode::from(NodeValue::Paragraph));
                            ast_node.append(p);
                            paragraph = Some(p);
                            p
                        }
                    };
                    append_inline_child(arena, target, child);
                }
            }
        }
        return Ok(ast_node);
    }

    // Process children (except for code_block which stores content in literal, not as children)
    if node_type != "code_block"
        && let Some(content) = json["content"].as_array()
    {
        // #1032: coalesce consecutive bullet_list siblings into the first
        // list. The PM model has no bullet-marker attribute, so adjacent
        // lists that differed only by marker (- / + / *) are one list in
        // every way the model can express — but as separate comrak List
        // nodes the CommonMark renderer separates them with
        // <!-- end list --> comments, which reparse as ```html fences
        // and grow the file on every save. Move each later list's items
        // into the first list (the first list's tight wins).
        // ordered_list is NOT merged (it carries restart numbering) and
        // task_list is a distinct PM node type, so neither is affected.
        let mut prev_bullet_list: Option<&AstNode<'a>> = None;
        for (idx, child_json) in content.iter().enumerate() {
            let is_bullet_list = child_json["type"].as_str() == Some("bullet_list");
            let normalized = normalize_inline_child_json(content, idx);
            let child = json_to_ast(arena, &normalized)?;
            if is_bullet_list && let Some(list_node) = prev_bullet_list {
                let items: Vec<&AstNode<'a>> = child.children().collect();
                for item in items {
                    item.detach();
                    list_node.append(item);
                }
                continue;
            }
            append_inline_child(arena, ast_node, child);
            prev_bullet_list = if is_bullet_list { Some(child) } else { None };
        }
    }

    // #1044: GFM has no headerless-table representation — comrak emits
    // the delimiter row only when a row is marked as the header row, and
    // without one the saved table reparses as a plain paragraph, silently
    // destroying the table. The editor schema permits rows made only of
    // table_cell, so when no row is a header row, promote the first row:
    // a faithful rendering preserving every cell's content, and a one-time
    // normalization (reparse yields table_header cells in row 0, so the
    // next save is byte-identical). The table's alignments (#881) are
    // stored on the table node and apply to the promoted row unchanged.
    if node_type == "table" {
        let mut first_row: Option<&AstNode<'a>> = None;
        let mut has_header_row = false;
        for child in ast_node.children() {
            if let NodeValue::TableRow(is_header) = &child.data.borrow().value {
                if first_row.is_none() {
                    first_row = Some(child);
                }
                if *is_header {
                    has_header_row = true;
                }
            }
        }
        if !has_header_row && let Some(row) = first_row {
            row.data.borrow_mut().value = NodeValue::TableRow(true);
        }
    }

    // Special case: image needs alt text as child (but NOT for HtmlInline, i.e. sized images)
    // #1417: the stored alt is markdown form (see convert_node's Image
    // arm) - re-parse it into inline nodes so emphasis/strong/code-span
    // markup is emitted as markup, not as one flat (formatter-escaped)
    // Text child. A plain-text alt re-parses to a single Text node,
    // exactly the old behaviour.
    if node_type == "image"
        && matches!(ast_node.data.borrow().value, NodeValue::Image(_))
        && let Some(alt) = json["attrs"]["alt"].as_str()
        && !alt.is_empty()
    {
        for alt_node in alt_markdown_to_nodes(arena, alt) {
            ast_node.append(alt_node);
        }
    }

    // #880 + #1407: an image carrying linkHref (a linked image, see
    // convert_node) and/or a `marks` array (emphasis/strong/
    // strikethrough wrapping the image) is wrapped in the
    // corresponding mark chain so save emits [![alt](src)](href),
    // *![alt](src)*, or both combined. Wrappers use the canonical
    // nesting order (#944) — the order create_marked_text applies —
    // and the returned chain goes through append_inline_child's mark
    // merging (#968), so a marked image inside a marked text span
    // rejoins the span instead of fragmenting it.
    if node_type == "image" && matches!(ast_node.data.borrow().value, NodeValue::Image(_)) {
        let mut wrappers: Vec<(u8, NodeValue)> = Vec::new();
        if let Some(link_href) = json["attrs"]["linkHref"].as_str()
            && !link_href.is_empty()
        {
            wrappers.push((
                mark_nesting_rank("link"),
                NodeValue::Link(Box::new(comrak::nodes::NodeLink {
                    url: link_href.to_owned(),
                    title: json["attrs"]["linkTitle"].as_str().unwrap_or("").to_owned(),
                })),
            ));
        }
        if let Some(marks) = json["marks"].as_array() {
            for mark in marks {
                let mark_type = mark["type"].as_str().unwrap_or("");
                let wrapper = match mark_type {
                    "em" => NodeValue::Emph,
                    "strong" => NodeValue::Strong,
                    "strikethrough" => NodeValue::Strikethrough,
                    _ => continue,
                };
                if !wrappers.iter().any(|(_, w)| same_mark_wrapper(w, &wrapper)) {
                    wrappers.push((mark_nesting_rank(mark_type), wrapper));
                }
            }
        }
        if !wrappers.is_empty() {
            wrappers.sort_by_key(|(rank, _)| *rank);
            // build_mark_chain consumes innermost-first; the sorted
            // list above is outermost-first (canonical rank order).
            let chain: Vec<NodeValue> = wrappers
                .into_iter()
                .map(|(_, wrapper)| wrapper)
                .rev()
                .collect();
            return Ok(build_mark_chain(arena, ast_node, &chain, 0));
        }
    }

    Ok(ast_node)
}

/// Whether comrak's AST validation accepts this node type as a direct
/// child of a `TableCell` (#1017). Mirrors `can_contain_type` for
/// `TableCell` in comrak's nodes.rs (shortcodes/heex variants are not
/// enabled in this codec).
fn valid_table_cell_child(value: &NodeValue) -> bool {
    matches!(
        value,
        NodeValue::Text(..)
            | NodeValue::Code(..)
            | NodeValue::Emph
            | NodeValue::Strong
            | NodeValue::Link(..)
            | NodeValue::Image(..)
            | NodeValue::Strikethrough
            | NodeValue::Highlight
            | NodeValue::Insert
            | NodeValue::HtmlInline(..)
            | NodeValue::Math(..)
            | NodeValue::WikiLink(..)
            | NodeValue::FootnoteReference(..)
            | NodeValue::Superscript
            | NodeValue::SpoileredText
            | NodeValue::Underline
            | NodeValue::Subscript
            | NodeValue::TaskItem(_)
            | NodeValue::Escaped
            | NodeValue::EscapedTag(_)
    )
}

/// Append a converted child to a table cell, degrading anything comrak
/// would reject there instead of building an ill-formed AST that makes
/// `format_commonmark` panic (#1017):
///
/// - valid inline children pass through unchanged;
/// - a hard/soft break degrades to a single space (a table cell is
///   single-line content in markdown);
/// - any other node (a list, code block, heading, nested table, …)
///   flattens to its plain text, words space-joined.
fn append_table_cell_child<'a>(
    arena: &'a Arena<'a>,
    cell: &'a AstNode<'a>,
    child: &'a AstNode<'a>,
) {
    let value = &child.data.borrow().value;
    if valid_table_cell_child(value) {
        append_inline_child(arena, cell, child);
        return;
    }
    if matches!(value, NodeValue::LineBreak | NodeValue::SoftBreak) {
        let space = arena.alloc(AstNode::from(NodeValue::Text(" ".to_owned().into())));
        cell.append(space);
        return;
    }
    let mut segments: Vec<String> = Vec::new();
    collect_cell_text(child, &mut segments);
    let text = segments
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !text.is_empty() {
        let text_node = arena.alloc(AstNode::from(NodeValue::Text(text.into())));
        cell.append(text_node);
    }
}

/// Collect the text of a node being flattened into a table cell (#1017):
/// text/code literals verbatim, breaks as spaces, and a space segment at
/// every other node's boundary so sibling blocks cannot fuse into one
/// word. The caller normalizes whitespace runs.
fn collect_cell_text<'a>(node: &'a AstNode<'a>, segments: &mut Vec<String>) {
    match &node.data.borrow().value {
        NodeValue::Text(literal) => segments.push(literal.to_string()),
        NodeValue::Code(code) => segments.push(code.literal.clone()),
        NodeValue::CodeBlock(code_block) => segments.push(code_block.literal.clone()),
        NodeValue::LineBreak | NodeValue::SoftBreak => segments.push(" ".to_owned()),
        _ => {
            for child in node.children() {
                collect_cell_text(child, segments);
            }
            segments.push(" ".to_owned());
        }
    }
}

/// Whether two node values are the same mark wrapper, so adjacent
/// siblings carrying it are really one marked span (#953).
fn same_mark_wrapper(a: &NodeValue, b: &NodeValue) -> bool {
    match (a, b) {
        (NodeValue::Link(x), NodeValue::Link(y)) => x.url == y.url && x.title == y.title,
        (NodeValue::Strong, NodeValue::Strong)
        | (NodeValue::Emph, NodeValue::Emph)
        | (NodeValue::Strikethrough, NodeValue::Strikethrough) => true,
        _ => false,
    }
}

/// Whether a node value is a mark wrapper (the wrapper kinds
/// `create_marked_text` chains and `same_mark_wrapper` compares).
fn is_mark_wrapper(value: &NodeValue) -> bool {
    matches!(
        value,
        NodeValue::Link(..) | NodeValue::Strong | NodeValue::Emph | NodeValue::Strikethrough
    )
}

/// Peel a pure mark chain — wrapper → wrapper → … → a single
/// non-wrapper leaf, as `create_marked_text` builds — into its wrapper
/// values (outermost first) and the leaf node. Returns `None` for
/// anything else (no wrappers, a wrapper with several children, a
/// wrapper bottoming out in another wrapper): those children keep the
/// legacy append behaviour.
fn peel_mark_chain<'a>(child: &'a AstNode<'a>) -> Option<(Vec<NodeValue>, &'a AstNode<'a>)> {
    let mut wrappers: Vec<NodeValue> = Vec::new();
    let mut current = child;
    loop {
        let value = current.data.borrow().value.clone();
        if !is_mark_wrapper(&value) {
            break;
        }
        let mut children = current.children();
        let only = children.next()?;
        if children.next().is_some() {
            return None;
        }
        wrappers.push(value);
        current = only;
    }
    if wrappers.is_empty() || is_mark_wrapper(&current.data.borrow().value) {
        return None;
    }
    Some((wrappers, current))
}

/// Insert a peeled mark chain's leaf under `parent`, reusing the
/// parent's tail wrapper whenever it carries one of the chain's marks
/// (#968). Adjacent ProseMirror runs are converted one at a time, each
/// in canonical rank order, so neighbouring runs that share a mark can
/// disagree about which shared mark is outermost: `*em **strong**
/// end*` arrives as Emph, Strong→Emph, Emph and used to render as
/// three fragmented sibling spans with `&#32;` boundary entities.
/// Reusing the tail wrapper for the shared mark — wherever it sits in
/// the incoming chain — and nesting the chain's remaining wrappers
/// inside it emits one properly nested span instead. When the tail
/// shares no mark with the chain, the chain is appended fresh, which
/// also subsumes the #953 same-wrapper merge (an identical tail
/// wrapper is simply the shared mark at position 0, recursively).
fn insert_marked_chain<'a>(
    arena: &'a Arena<'a>,
    parent: &'a AstNode<'a>,
    wrappers: &[NodeValue],
    leaf: &'a AstNode<'a>,
) {
    if !wrappers.is_empty()
        && let Some(last) = parent.last_child()
    {
        let last_value = last.data.borrow().value.clone();
        if is_mark_wrapper(&last_value)
            && let Some(shared) = wrappers
                .iter()
                .position(|wrapper| same_mark_wrapper(wrapper, &last_value))
        {
            let rest: Vec<NodeValue> = wrappers
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != shared)
                .map(|(_, wrapper)| wrapper.clone())
                .collect();
            insert_marked_chain(arena, last, &rest, leaf);
            return;
        }
    }
    let mut current = parent;
    for wrapper in wrappers {
        let node = arena.alloc(AstNode::from(wrapper.clone()));
        current.append(node);
        current = node;
    }
    current.append(leaf);
}

/// Append an inline child, merging it into the previous sibling when
/// both are the same mark wrapper (#953): `[a `code` b](href)` reaches
/// the save direction as three text nodes that share the link mark, and
/// three sibling Link nodes render as three fragmented links. Moving
/// the new wrapper's children into the previous wrapper (recursively,
/// so nested wrappers merge too) emits one link containing all three
/// segments. Pure mark chains go through the #968 tail insertion
/// (`insert_marked_chain`), which handles the #953 merge as its
/// shared-mark case; non-chain children keep the original merge.

/// #1412: adopt trailing `html_inline` atoms into the tail mark
/// wrapper when a marked chain that shares that wrapper's mark is
/// about to be inserted. The parse direction emits `html_inline`
/// atoms markless (see the `NodeValue::HtmlInline` arm of
/// `convert_node`), so a mark spanning an atom reaches the save
/// direction as marked run, bare atom, marked run. Appending the
/// atom as a sibling severs the tail wrapper: the next run opens a
/// NEW wrapper and the span fragments (`[a <b>x</b> c](/y)` saved as
/// `[a ](/y)<b>[x](/y)</b>[ c](/y)`, with `&#32;` leaks at emphasis
/// seams). When the incoming chain shares a mark with the wrapper
/// before the atoms, the atoms belong to that span: move them inside
/// the wrapper (recursing along the chain's shared marks, mirroring
/// `insert_marked_chain`'s descent, so nested spans adopt at the
/// deepest shared level), after which the chain insertion merges
/// into the same wrapper as usual. Atoms not followed by a chain
/// sharing the preceding wrapper's mark keep their legacy sibling
/// placement, so unmarked HTML and atoms after a finished span are
/// unchanged. Extending the atom predicate (e.g. to images, #1407)
/// generalizes this to the other atom kinds.
fn adopt_trailing_html_atoms<'a>(parent: &'a AstNode<'a>, wrappers: &[NodeValue]) {
    let children: Vec<&'a AstNode<'a>> = parent.children().collect();
    let mut split = children.len();
    while split > 0
        && matches!(
            children[split - 1].data.borrow().value,
            NodeValue::HtmlInline(..)
        )
    {
        split -= 1;
    }
    if split == children.len() || split == 0 {
        return;
    }
    let tail = children[split - 1];
    let tail_value = tail.data.borrow().value.clone();
    if !is_mark_wrapper(&tail_value) {
        return;
    }
    let Some(shared) = wrappers
        .iter()
        .position(|wrapper| same_mark_wrapper(wrapper, &tail_value))
    else {
        return;
    };
    for atom in &children[split..] {
        atom.detach();
        tail.append(atom);
    }
    let rest: Vec<NodeValue> = wrappers
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != shared)
        .map(|(_, wrapper)| wrapper.clone())
        .collect();
    adopt_trailing_html_atoms(tail, &rest);
}

fn append_inline_child<'a>(arena: &'a Arena<'a>, parent: &'a AstNode<'a>, child: &'a AstNode<'a>) {
    if let Some((wrappers, leaf)) = peel_mark_chain(child) {
        leaf.detach();
        // #1412: html_inline atoms sitting between this chain and the
        // tail mark wrapper join the span before the chain merges in.
        adopt_trailing_html_atoms(parent, &wrappers);
        insert_marked_chain(arena, parent, &wrappers, leaf);
        return;
    }
    if let Some(last) = parent.last_child() {
        let same = {
            let last_value = last.data.borrow();
            let child_value = child.data.borrow();
            same_mark_wrapper(&last_value.value, &child_value.value)
        };
        if same {
            let grandchildren: Vec<&AstNode<'a>> = child.children().collect();
            for grandchild in grandchildren {
                grandchild.detach();
                append_inline_child(arena, last, grandchild);
            }
            return;
        }
    }
    parent.append(child);
}

/// Create marked text node(s) - handles nested marks.
fn create_marked_text<'a>(arena: &'a Arena<'a>, text: &str, marks: &[Value]) -> &'a AstNode<'a> {
    if marks.is_empty() {
        return arena.alloc(AstNode::from(NodeValue::Text(text.to_owned().into())));
    }

    // A code mark replaces the text node itself (code is the innermost
    // mark, rank 4) but does NOT discard the other marks (#953): the
    // remaining marks wrap the Code node via the same chain the plain
    // text path uses, so [`code`](href) keeps its link and **`code`**
    // keeps its strong.
    let has_code = marks
        .iter()
        .any(|mark| mark["type"].as_str() == Some("code"));

    // Collect wrapper values in the canonical nesting order, outermost
    // first (#944): sort the incoming marks — whose array order depends on
    // which spelling or editor produced the doc — so the same marked text
    // always saves to the same markdown.
    let mut sorted_marks: Vec<&Value> = marks.iter().collect();
    sorted_marks.sort_by_key(|mark| mark_nesting_rank(mark["type"].as_str().unwrap_or("")));
    let mut wrappers: Vec<NodeValue> = Vec::new();

    for mark in sorted_marks {
        let mark_type = mark["type"].as_str().unwrap_or("");
        let wrapper_value = match mark_type {
            "em" => NodeValue::Emph,
            "strong" => NodeValue::Strong,
            "strikethrough" => NodeValue::Strikethrough,
            "link" => {
                let href = mark["attrs"]["href"].as_str().unwrap_or("");
                let title = mark["attrs"]["title"].as_str().unwrap_or("");
                NodeValue::Link(Box::new(comrak::nodes::NodeLink {
                    url: href.to_owned(),
                    title: title.to_owned(),
                }))
            }
            _ => continue, // Skip code (handled below) and unknown marks
        };
        wrappers.push(wrapper_value);
    }

    // Start with the innermost node: a Code node when a code mark is
    // present, otherwise the plain text node.
    let inner_node = if has_code {
        arena.alloc(AstNode::from(NodeValue::Code(comrak::nodes::NodeCode {
            num_backticks: 1,
            literal: text.to_owned(),
        })))
    } else {
        arena.alloc(AstNode::from(NodeValue::Text(text.to_owned().into())))
    };

    if wrappers.is_empty() {
        return inner_node;
    }

    // Build wrapper chain recursively - innermost mark wraps the inner
    // node, outer marks wrap that. `build_mark_chain` consumes its list
    // innermost first, so reverse the canonical outermost-first list.
    wrappers.reverse();
    build_mark_chain(arena, inner_node, &wrappers, 0)
}

/// Build a chain of mark wrappers around a text node.
fn build_mark_chain<'a>(
    arena: &'a Arena<'a>,
    inner: &'a AstNode<'a>,
    wrappers: &[NodeValue],
    index: usize,
) -> &'a AstNode<'a> {
    if index >= wrappers.len() {
        return inner;
    }

    let wrapper = arena.alloc(AstNode::from(wrappers[index].clone()));
    wrapper.append(inner);

    if index + 1 >= wrappers.len() {
        wrapper
    } else {
        build_mark_chain(arena, wrapper, wrappers, index + 1)
    }
}

/// Convert plain text to `ProseMirror` JSON document.
///
/// Each line becomes a paragraph.
#[must_use]
pub fn plain_text_to_prosemirror(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .lines()
        .map(|line| {
            if line.is_empty() {
                json!({"type": "paragraph"})
            } else {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": line}]
                })
            }
        })
        .collect();

    json!({
        "type": "doc",
        "content": if paragraphs.is_empty() {
            vec![json!({"type": "paragraph"})]
        } else {
            paragraphs
        }
    })
}

/// Convert text to a raw mode `ProseMirror` document.
///
/// Creates a document with a single `code_block` containing all text.
#[must_use]
pub fn raw_text_to_prosemirror(text: &str) -> Value {
    if text.is_empty() {
        json!({
            "type": "doc",
            "content": [{"type": "code_block"}]
        })
    } else {
        json!({
            "type": "doc",
            "content": [{
                "type": "code_block",
                "content": [{"type": "text", "text": text}]
            }]
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_document() {
        let doc = markdown_to_prosemirror("");
        assert_eq!(doc["type"], "doc");
        assert!(!doc["content"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_paragraph() {
        let doc = markdown_to_prosemirror("Hello world");
        assert_eq!(doc["type"], "doc");
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "paragraph");
        assert_eq!(content[0]["content"][0]["text"], "Hello world");
    }

    #[test]
    fn test_heading() {
        let doc = markdown_to_prosemirror("# Heading 1\n\n## Heading 2");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "heading");
        assert_eq!(content[0]["attrs"]["level"], 1);
        assert_eq!(content[0]["content"][0]["text"], "Heading 1");

        assert_eq!(content[1]["type"], "heading");
        assert_eq!(content[1]["attrs"]["level"], 2);
    }

    #[test]
    fn test_bold() {
        let doc = markdown_to_prosemirror("This is **bold** text");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        // Find the bold text
        let bold_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "strong"))
        });

        assert!(bold_text.is_some());
        assert_eq!(bold_text.unwrap()["text"], "bold");
    }

    #[test]
    fn test_italic() {
        let doc = markdown_to_prosemirror("This is *italic* text");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let italic_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "em"))
        });

        assert!(italic_text.is_some());
        assert_eq!(italic_text.unwrap()["text"], "italic");
    }

    #[test]
    fn test_bold_italic() {
        let doc = markdown_to_prosemirror("This is ***bold italic*** text");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let marked_text = para_content
            .iter()
            .find(|n| n["marks"].as_array().is_some_and(|m| m.len() == 2));

        assert!(marked_text.is_some());
        let marks = marked_text.unwrap()["marks"].as_array().unwrap();
        let mark_types: Vec<&str> = marks.iter().map(|m| m["type"].as_str().unwrap()).collect();
        assert!(mark_types.contains(&"strong"));
        assert!(mark_types.contains(&"em"));
    }

    #[test]
    fn test_inline_code() {
        let doc = markdown_to_prosemirror("Use `code` here");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let code_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "code"))
        });

        assert!(code_text.is_some());
        assert_eq!(code_text.unwrap()["text"], "code");
    }

    #[test]
    fn test_link() {
        let doc = markdown_to_prosemirror("Click [here](https://example.com)");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let link_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "link"))
        });

        assert!(link_text.is_some());
        let link = link_text.unwrap();
        assert_eq!(link["text"], "here");
        let link_mark = link["marks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["type"] == "link")
            .unwrap();
        assert_eq!(link_mark["attrs"]["href"], "https://example.com");
    }

    #[test]
    fn test_empty_link_roundtrips() {
        // #996: an empty-text link has no child text node to carry the
        // link mark; the codec synthesizes an empty marked text node so
        // the href is present in the PM doc and survives save.
        let doc = markdown_to_prosemirror("[](https://e.com)");
        let para_content = doc["content"][0]["content"].as_array().unwrap();
        assert_eq!(para_content.len(), 1);
        assert_eq!(para_content[0]["type"], "text");
        assert_eq!(para_content[0]["text"], "");
        let mark = &para_content[0]["marks"][0];
        assert_eq!(mark["type"], "link");
        assert_eq!(mark["attrs"]["href"], "https://e.com");

        // Byte-identical round-trips, alone, mid-paragraph, and with a
        // title; the second save is a stable fixpoint.
        for original in [
            "[](https://e.com)\n",
            "before [](https://e.com) after\n",
            "[](https://e.com \"T\")\n",
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, original, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_misplaced_nodes_return_error_not_panic() {
        // #1043: json_to_ast placed any known node type anywhere, so a
        // misplaced node built an ill-formed comrak AST — panicking the
        // formatter in debug builds, emitting invalid markdown in
        // release builds. Placement validation must reject each of these
        // with MisplacedNode instead.
        let cases: &[(&str, Value)] = &[
            (
                "text at doc level",
                json!({"type": "doc", "content": [{"type": "text", "text": "orphan"}]}),
            ),
            (
                "image at doc level",
                json!({"type": "doc", "content": [{"type": "image", "attrs": {"src": "x.png"}}]}),
            ),
            (
                "hard_break at doc level",
                json!({"type": "doc", "content": [{"type": "hard_break"}]}),
            ),
            (
                "paragraph inside paragraph",
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
                ]}]}),
            ),
            (
                "heading inside paragraph",
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [
                    {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "x"}]}
                ]}]}),
            ),
            (
                "inline directly inside blockquote",
                json!({"type": "doc", "content": [{"type": "blockquote", "content": [
                    {"type": "text", "text": "x"}
                ]}]}),
            ),
            (
                "paragraph directly inside table (no row)",
                json!({"type": "doc", "content": [{"type": "table", "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
                ]}]}),
            ),
            (
                "list_item inside task_list",
                json!({"type": "doc", "content": [{"type": "task_list", "content": [
                    {"type": "list_item", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
                    ]}
                ]}]}),
            ),
        ];
        for (name, doc) in cases {
            let result = prosemirror_to_markdown(doc);
            assert!(
                matches!(result, Err(ConversionError::MisplacedNode { .. })),
                "{name}: expected MisplacedNode error, got {result:?}"
            );
        }

        // Unknown types keep their existing error (validation must not
        // mask it), and well-formed docs convert exactly as before.
        let unknown = json!({"type": "doc", "content": [{"type": "nosuchnode"}]});
        assert!(matches!(
            prosemirror_to_markdown(&unknown),
            Err(ConversionError::UnknownType(_))
        ));
        let ok = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "hello"}]}]});
        assert_eq!(prosemirror_to_markdown(&ok).unwrap(), "hello\n");
    }

    #[test]
    fn test_code_block() {
        let doc = markdown_to_prosemirror("```rust\nfn main() {}\n```");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "code_block");
        assert_eq!(content[0]["attrs"]["params"], "rust");
        assert_eq!(content[0]["content"][0]["text"], "fn main() {}");
    }

    #[test]
    fn test_blockquote() {
        let doc = markdown_to_prosemirror("> This is a quote");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "blockquote");
        assert_eq!(content[0]["content"][0]["type"], "paragraph");
    }

    #[test]
    fn test_info_less_code_block_keeps_following_paragraph_in_blockquote() {
        let original = "> ```\n> code\n> ```\n>\n> trailing paragraph\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();

        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(original),
            "saving and reopening must keep all blocks inside one blockquote: {saved:?}"
        );
    }

    #[test]
    fn test_info_less_fence_in_blockquote_trailing_content_round_trips() {
        // #1496 acceptance cases: a no-info fence inside a blockquote
        // followed by each kind of trailing quoted content must reopen
        // as ONE blockquote holding every block. Controls (info fence,
        // fence last, table / thematic break mid-quote) pin the shapes
        // that already round-tripped before the fix.
        for original in [
            // (a) paragraph
            "> ```\n> code\n> ```\n>\n> trailing paragraph\n",
            // (b) list
            "> ```\n> code\n> ```\n>\n> - one\n> - two\n",
            // (c) heading
            "> ```\n> code\n> ```\n>\n> # Head\n",
            // (d) second code block
            "> ```\n> code\n> ```\n>\n> ```\n> more\n> ```\n",
            // (e) two paragraphs
            "> ```\n> code\n> ```\n>\n> para one\n>\n> para two\n",
            // control: ```rust mid-quote
            "> ```rust\n> code\n> ```\n>\n> trailing paragraph\n",
            // control: fence as the quote's last block
            "> para\n>\n> ```\n> code\n> ```\n",
            // control: table mid-quote after the fence
            "> ```\n> code\n> ```\n>\n> | a | b |\n> |---|---|\n> | 1 | 2 |\n",
            // control: thematic break mid-quote after the fence
            "> ```\n> code\n> ```\n>\n> ---\n>\n> after\n",
        ] {
            let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
            assert_eq!(
                markdown_to_prosemirror(&saved),
                markdown_to_prosemirror(original),
                "round trip changed the structure for {original:?}: saved as {saved:?}"
            );
            let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
            assert_eq!(resaved, saved, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_blockquote_fence_placeholders_do_not_collide_or_eat_text() {
        // PR #3770 Codex finding: with >= 11 placeholder blocks,
        // `...-1` is a prefix of `...-10`, and an unrestricted replace
        // would corrupt that block's info string; placeholder-shaped
        // user text elsewhere must also survive a save untouched.
        let mut original = String::new();
        for i in 0..12 {
            original.push_str(&format!("> ```\n> code {i}\n> ```\n>\n"));
        }
        original.push_str("> trailing\n");
        original.push_str("\nmention idweb-blockquote-fence-0-end in prose\n");
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(&original)).unwrap();
        assert!(
            saved.contains("idweb-blockquote-fence-0-end in prose"),
            "placeholder-shaped user text was stripped: {saved:?}"
        );
        assert!(
            !saved.contains("```idweb"),
            "a generated placeholder leaked into the output: {saved:?}"
        );
        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(&original),
            "12-block quote changed structure: {saved:?}"
        );
    }

    #[test]
    fn test_bullet_list() {
        let doc = markdown_to_prosemirror("- Item 1\n- Item 2");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "bullet_list");
        let items = content[0]["content"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "list_item");
    }

    #[test]
    fn test_adjacent_bullet_lists_mixed_markers_fixpoint() {
        // #1032: "- a\n+ b\n* c" parses to three adjacent bullet_list
        // nodes (the PM model has no marker attr). Saving must coalesce
        // them into one comrak list: separate adjacent lists make the
        // renderer emit <!-- end list --> comments, which reparse as
        // ```html fences and grow the file on every save.
        let first = prosemirror_to_markdown(&markdown_to_prosemirror("- a\n+ b\n* c\n")).unwrap();
        let second = prosemirror_to_markdown(&markdown_to_prosemirror(&first)).unwrap();
        let third = prosemirror_to_markdown(&markdown_to_prosemirror(&second)).unwrap();
        assert_eq!(first, "- a\n- b\n- c\n", "coalesced single list: {first:?}");
        assert!(
            !first.contains("end list"),
            "no end-list artefact: {first:?}"
        );
        assert!(
            !first.contains("```html"),
            "no html-fence artefact: {first:?}"
        );
        assert_eq!(first, second, "second save is a fixpoint");
        assert_eq!(second, third, "third save is a fixpoint");
    }

    #[test]
    fn test_adjacent_bullet_lists_in_blockquote_fixpoint() {
        // #1032: coalescing applies at any parent, e.g. inside a
        // blockquote, not just at doc level.
        let (first, second) = two_saves("> - a\n> + b\n");
        assert!(
            !first.contains("end list"),
            "no end-list artefact: {first:?}"
        );
        assert!(
            !first.contains("```html"),
            "no html-fence artefact: {first:?}"
        );
        for needle in ["a", "b"] {
            assert!(first.contains(needle), "item {needle} preserved: {first:?}");
        }
        assert_eq!(first, second, "blockquote lists are a fixpoint");
    }

    #[test]
    fn test_adjacent_bullet_lists_first_tight_wins() {
        // #1032: when adjacent bullet lists differ in tight, the merged
        // list keeps the FIRST list's tight (here tight=true, so items
        // render with no blank lines between them; the second list's
        // tight=false must not loosen the merged list).
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a"}]}
                    ]}
                ]},
                {"type": "bullet_list", "attrs": {"tight": false}, "content": [
                    {"type": "list_item", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "b"}]}
                    ]}
                ]}
            ]
        });
        let md = prosemirror_to_markdown(&doc).expect("adjacent bullet lists convert");
        assert_eq!(md, "- a\n- b\n", "first list's tight wins: {md:?}");
    }

    #[test]
    fn test_ordered_list() {
        let doc = markdown_to_prosemirror("1. First\n2. Second");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "ordered_list");
        assert_eq!(content[0]["attrs"]["order"], 1);
    }

    #[test]
    fn test_horizontal_rule() {
        let doc = markdown_to_prosemirror("Before\n\n---\n\nAfter");
        let content = doc["content"].as_array().unwrap();

        let hr = content.iter().find(|n| n["type"] == "horizontal_rule");
        assert!(hr.is_some());
    }

    #[test]
    fn test_image() {
        let doc = markdown_to_prosemirror("![Alt text](image.png \"Title\")");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let image = para_content.iter().find(|n| n["type"] == "image");
        assert!(image.is_some());
        let img = image.unwrap();
        assert_eq!(img["attrs"]["src"], "image.png");
        assert_eq!(img["attrs"]["alt"], "Alt text");
        assert_eq!(img["attrs"]["title"], "Title");
    }

    #[test]
    fn test_image_alt_markup_roundtrips() {
        // #1417: inline markup inside image alt text must survive an
        // open->save round-trip. The PM alt attr stores the markdown
        // form; plain alts are byte-identical to before.
        for (md, expected_alt) in [
            ("![*alt* text](/x.png)\n", "*alt* text"),
            ("![**bold alt**](/x.png)\n", "**bold alt**"),
            ("![a `code` b](/x.png)\n", "a `code` b"),
            ("![a [link](/y) b](/x.png)\n", "a [link](/y) b"),
            ("![plain alt](/x.png)\n", "plain alt"),
        ] {
            let doc = markdown_to_prosemirror(md);
            let img = &doc["content"][0]["content"][0];
            assert_eq!(img["type"], "image", "image node for {md}");
            assert_eq!(img["attrs"]["alt"], expected_alt, "stored alt for {md}");
            let saved = prosemirror_to_markdown(&doc).expect("save converts");
            assert_eq!(saved.trim(), md.trim(), "round-trip for {md}");
            // And the saved form re-parses to the same stored alt (fixpoint).
            let doc2 = markdown_to_prosemirror(&saved);
            assert_eq!(
                doc2["content"][0]["content"][0]["attrs"]["alt"],
                expected_alt
            );
        }
    }

    #[test]
    fn test_image_alt_markup_pm_to_md() {
        // #1417, save direction alone: a PM image whose alt attr is in
        // markdown form emits that markup inside the image brackets.
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {"src": "/x.png", "alt": "a `code` b"}
                }]
            }]
        });
        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert!(
            md.contains("![a `code` b](/x.png)"),
            "code-span alt emitted as markup: {md}"
        );
    }

    #[test]
    fn test_plain_text_to_prosemirror() {
        let doc = plain_text_to_prosemirror("Line 1\nLine 2\n\nLine 4");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content.len(), 4);
        assert_eq!(content[0]["content"][0]["text"], "Line 1");
        assert_eq!(content[1]["content"][0]["text"], "Line 2");
        // Line 3 is empty paragraph
        assert!(content[2]["content"].is_null());
        assert_eq!(content[3]["content"][0]["text"], "Line 4");
    }

    #[test]
    fn test_raw_text_to_prosemirror() {
        let doc = raw_text_to_prosemirror("function test() {\n  return 42;\n}");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "code_block");
        assert!(
            content[0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("function test()")
        );
    }

    #[test]
    fn test_roundtrip_simple() {
        let original = "# Hello\n\nThis is a paragraph.\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        // Roundtrip should produce equivalent markdown
        assert!(result.contains("# Hello"));
        assert!(result.contains("This is a paragraph."));
    }

    #[test]
    fn test_roundtrip_with_formatting() {
        let original = "This has **bold** and *italic* text.\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        assert!(result.contains("**bold**") || result.contains("__bold__"));
        assert!(result.contains("*italic*") || result.contains("_italic_"));
    }

    /// Two consecutive open->save round-trips of `original`, returning both
    /// saved spellings (#944 fixpoint assertions).
    fn two_saves(original: &str) -> (String, String) {
        let first = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
        let second = prosemirror_to_markdown(&markdown_to_prosemirror(&first)).unwrap();
        (first, second)
    }

    #[test]
    fn test_strong_link_roundtrip_fixpoint() {
        // Both spellings of a bold link canonicalize to the link-outermost
        // form and are byte-stable from the first save on (#944).
        for original in [
            "**[text](https://example.com)**\n",
            "[**text**](https://example.com)\n",
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, "[**text**](https://example.com)\n");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
        // The two spellings also parse to the identical PM doc.
        assert_eq!(
            markdown_to_prosemirror("**[text](https://example.com)**"),
            markdown_to_prosemirror("[**text**](https://example.com)")
        );
    }

    #[test]
    fn test_softbreak_marked_span_roundtrip_fixpoint() {
        // A marked span crossing a soft line break saves as the normalized
        // single-line form and is stable from the first save on (#981).
        // The marked " " node the SoftBreak arm produces must merge with
        // its identically-marked neighbours instead of rendering as its
        // own delimited span (which produced ****&#32;**** corruption).
        for (original, expected) in [
            ("**bold\nmore**\n", "**bold more**\n"),
            ("*em\nmore*\n", "*em more*\n"),
            ("~~gone\naway~~\n", "~~gone away~~\n"),
            ("**bold\nmore words here**\n", "**bold more words here**\n"),
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, expected, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
            assert!(!first.contains("&#32;"), "entity in {first:?}");
        }
    }

    #[test]
    fn test_softbreak_link_roundtrip_fixpoint() {
        // A link whose text crosses a soft line break stays ONE link on
        // save, with and without a title (#981) — it must not fragment
        // into per-word links plus a space-only link.
        for (original, expected) in [
            ("[a\nb](https://e.com)\n", "[a b](https://e.com)\n"),
            (
                "[a\nb](https://e.com \"T\")\n",
                "[a b](https://e.com \"T\")\n",
            ),
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, expected, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_nested_mark_spans_roundtrip_fixpoint() {
        // #968: a span whose extra mark ranks OUTER to the mark it
        // shares with its neighbours (strong/link inside em, strong
        // inside strike, em inside strike) used to save as fragmented
        // sibling wrappers with &#32; boundary entities. The save
        // direction must re-nest the runs into one span: byte-identical
        // first save and a fixpoint on the second.
        for original in [
            "*em **strong** end*\n",
            "~~strike **strong** end~~\n",
            "*em [link](https://e.com) end*\n",
            "**strong [link](https://e.com) end**\n",
            "~~strike *em* end~~\n",
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, original, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
            assert!(!first.contains("&#32;"), "entity in {first:?}");
        }
    }

    #[test]
    fn test_nested_mark_clean_cases_do_not_regress() {
        // #968 regression guards: the combinations the #944/#953/#999
        // fixes already round-trip must keep doing so, byte-identical
        // (modulo the documented _em_ -> *em* spelling normalization).
        for (original, expected) in [
            ("**bold ~~strike~~ end**\n", "**bold ~~strike~~ end**\n"),
            ("**bold `code` end**\n", "**bold `code` end**\n"),
            ("*em `code` end*\n", "*em `code` end*\n"),
            (
                "[link *em* text](https://e.com)\n",
                "[link *em* text](https://e.com)\n",
            ),
            ("**a *b `c` d* e**\n", "**a *b `c` d* e**\n"),
            ("*em ~~strike~~ end*\n", "*em ~~strike~~ end*\n"),
            ("**bold _em_ end**\n", "**bold *em* end**\n"),
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, expected, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
            assert!(!first.contains("&#32;"), "entity in {first:?}");
        }
    }

    /// Build a one-paragraph PM doc from (text, mark-types) runs
    /// (#1418 test helper).
    fn pm_runs(runs: &[(&str, &[&str])]) -> Value {
        let content: Vec<Value> = runs
            .iter()
            .map(|(text, marks)| {
                let mut node = json!({"type": "text", "text": text});
                if !marks.is_empty() {
                    node["marks"] = json!(
                        marks
                            .iter()
                            .map(|mark| json!({"type": mark}))
                            .collect::<Vec<_>>()
                    );
                }
                node
            })
            .collect();
        json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": content}]
        })
    }

    #[test]
    fn test_delimiter_edge_space_word_neighbour_roundtrips() {
        // #1418: a delimiter-mark span whose text starts with a space,
        // with a word character touching it on that side, used to save
        // as `a*&#32;b*` — a form that fails CommonMark flanking on
        // reparse, destroying the mark. The edge space is hoisted out
        // of the wrapper instead: the mark survives, the saved bytes
        // are a fixpoint, and the reopened doc is the original with
        // the space as its own unmarked run.
        for (marks, expected_md) in [
            (vec!["em"], "a *b*\n"),
            (vec!["strong"], "a **b**\n"),
            (vec!["strikethrough"], "a ~~b~~\n"),
        ] {
            let pm = pm_runs(&[("a", &[]), (" b", &marks)]);
            let saved = prosemirror_to_markdown(&pm).expect("converts");
            assert_eq!(saved, expected_md, "saved form for {marks:?}");
            assert!(!saved.contains("&#32;"), "entity in {saved:?}");
            let reopened = markdown_to_prosemirror(&saved);
            assert_eq!(
                reopened,
                pm_runs(&[("a ", &[]), ("b", &marks)]),
                "reopened doc for {marks:?}"
            );
            let resaved = prosemirror_to_markdown(&reopened).expect("reconverts");
            assert_eq!(resaved, saved, "fixpoint for {marks:?}");
        }

        // Combined em+strong on the same edge-space span.
        let marks = vec!["strong", "em"];
        let pm = pm_runs(&[("a", &[]), (" b", &marks)]);
        let saved = prosemirror_to_markdown(&pm).expect("converts");
        assert!(!saved.contains("&#32;"), "entity in {saved:?}");
        let reopened = markdown_to_prosemirror(&saved);
        assert_eq!(reopened, pm_runs(&[("a ", &[]), ("b", &marks)]));
        let resaved = prosemirror_to_markdown(&reopened).expect("reconverts");
        assert_eq!(resaved, saved, "fixpoint for combined marks");
    }

    #[test]
    fn test_delimiter_trailing_edge_space_word_neighbour_roundtrips() {
        // #1418 trailing-edge mirror: `a ` + em(`b `) + `c`.
        let pm = pm_runs(&[("a ", &[]), ("b ", &["em"]), ("c", &[])]);
        let saved = prosemirror_to_markdown(&pm).expect("converts");
        assert_eq!(saved, "a *b* c\n");
        let reopened = markdown_to_prosemirror(&saved);
        assert_eq!(
            reopened,
            pm_runs(&[("a ", &[]), ("b", &["em"]), (" c", &[])])
        );
        let resaved = prosemirror_to_markdown(&reopened).expect("reconverts");
        assert_eq!(resaved, saved, "fixpoint");
    }

    #[test]
    fn test_delimiter_edge_space_in_heading_roundtrips() {
        // #1418: the same failure in a heading context.
        let pm = json!({
            "type": "doc",
            "content": [{
                "type": "heading",
                "attrs": {"level": 1},
                "content": [
                    {"type": "text", "text": "a"},
                    {"type": "text", "text": " b", "marks": [{"type": "em"}]}
                ]
            }]
        });
        let saved = prosemirror_to_markdown(&pm).expect("converts");
        assert_eq!(saved, "# a *b*\n");
        let reopened = markdown_to_prosemirror(&saved);
        let resaved = prosemirror_to_markdown(&reopened).expect("reconverts");
        assert_eq!(resaved, saved, "fixpoint");
        assert_eq!(
            reopened["content"][0]["content"][1]["marks"][0]["type"], "em",
            "mark survives in heading"
        );
    }

    #[test]
    fn test_delimiter_edge_space_safe_neighbours_keep_entity_form() {
        // #1418 controls: edges a word character does NOT touch keep
        // the `&#32;` form, which round-trips PM-identical — a span
        // padded on both edges with spaced neighbours, a span whose
        // neighbour on the space-carrying side is itself a space, and
        // a span at the paragraph boundary.
        let (first, second) = two_saves("a *&#32;b&#32;* c\n");
        assert_eq!(first, "a *&#32;b&#32;* c\n");
        assert_eq!(first, second, "fixpoint");

        let pm = pm_runs(&[("a ", &[]), (" b", &["em"])]);
        let saved = prosemirror_to_markdown(&pm).expect("converts");
        assert_eq!(saved, "a *&#32;b*\n");
        assert_eq!(markdown_to_prosemirror(&saved), pm, "PM-identical");

        let pm = pm_runs(&[(" b", &["em"]), ("c", &[])]);
        let saved = prosemirror_to_markdown(&pm).expect("converts");
        assert_eq!(
            markdown_to_prosemirror(&saved),
            pm,
            "PM-identical at boundary"
        );
        let resaved =
            prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).expect("reconverts");
        assert_eq!(resaved, saved, "fixpoint at boundary");
    }

    #[test]
    fn test_spaced_link_destination_roundtrips_in_angle_form() {
        // #1019: a destination containing a space must save in the
        // <...> angle form byte-identically — comrak's renderer would
        // otherwise percent-encode it to %20, breaking relative links
        // and mutating the PM href on reparse.
        let original = "[notes](<my notes.md>)\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, original, "angle form retained");
        assert_eq!(first, second, "fixpoint");
        assert!(!first.contains("%20"), "no percent-encoding: {first}");
        let doc = markdown_to_prosemirror(&first);
        let link = &doc["content"][0]["content"][0]["marks"][0];
        assert_eq!(link["type"], "link");
        assert_eq!(link["attrs"]["href"], "my notes.md", "PM href unmutated");
    }

    #[test]
    fn test_spaced_image_destination_roundtrips_in_angle_form() {
        // #1019: same preservation for image src.
        let original = "![a](<my pic.png>)\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, original, "angle form retained");
        assert_eq!(first, second, "fixpoint");
        let doc = markdown_to_prosemirror(&first);
        let img = &doc["content"][0]["content"][0];
        assert_eq!(img["type"], "image");
        assert_eq!(img["attrs"]["src"], "my pic.png", "PM src unmutated");
    }

    #[test]
    fn test_spaced_destination_with_title_and_editor_href() {
        // #1019: a spaced destination with a title keeps both, and an
        // editor-created PM link whose href contains a space saves in
        // angle form and reparses to the identical href.
        let (first, second) = two_saves("[t](<a b.md> \"T\")\n");
        assert_eq!(first, "[t](<a b.md> \"T\")\n");
        assert_eq!(first, second, "fixpoint");

        let pm = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "t",
                    "marks": [{"type": "link", "attrs": {"href": "https://e.com/a b"}}]
                }]
            }]
        });
        let md = prosemirror_to_markdown(&pm).expect("converts");
        assert_eq!(md, "[t](<https://e.com/a b>)\n");
        let back = markdown_to_prosemirror(&md);
        assert_eq!(
            back["content"][0]["content"][0]["marks"][0]["attrs"]["href"], "https://e.com/a b",
            "href identical after round trip"
        );
    }

    #[test]
    fn test_tab_and_newline_destinations_roundtrip_in_angle_form() {
        // #1423: the #1019 protection only triggered on a literal space,
        // so a tab destination was percent-encoded to %09 on save and a
        // newline destination was mangled to `% `, destroying the link.
        // Both must now save in <...> angle form byte-identically, for
        // links and images alike.
        for original in [
            "[notes](<a\tb.md>)\n",
            "![a](<a\tb.png>)\n",
            "[notes](<a\nb.md>)\n",
            "![a](<a\nb.png>)\n",
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, original, "angle form retained for {original:?}");
            assert_eq!(first, second, "fixpoint for {original:?}");
            assert!(!first.contains("%09"), "no tab encoding: {first:?}");
        }
        // The PM href/src survive unmutated, tab and newline alike.
        let doc = markdown_to_prosemirror("[notes](<a\tb.md>)\n");
        assert_eq!(
            doc["content"][0]["content"][0]["marks"][0]["attrs"]["href"], "a\tb.md",
            "PM href unmutated (tab)"
        );
        let doc = markdown_to_prosemirror("![a](<a\nb.png>)\n");
        assert_eq!(
            doc["content"][0]["content"][0]["attrs"]["src"], "a\nb.png",
            "PM src unmutated (newline)"
        );
    }

    #[test]
    fn test_tab_and_newline_editor_href_and_reference_definition() {
        // #1423, save direction alone: an editor-created PM link whose
        // href carries a tab or newline saves in angle form and reparses
        // to the identical href.
        for href in ["a\tb.md", "a\nb.md"] {
            let pm = json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "t",
                        "marks": [{"type": "link", "attrs": {"href": href}}]
                    }]
                }]
            });
            let md = prosemirror_to_markdown(&pm).expect("converts");
            assert_eq!(md, format!("[t](<{href}>)\n"), "angle form for {href:?}");
            let back = markdown_to_prosemirror(&md);
            assert_eq!(
                back["content"][0]["content"][0]["marks"][0]["attrs"]["href"], href,
                "href identical after round trip for {href:?}"
            );
        }

        // #1423, reference definitions: a reference-style link's
        // destination is resolved into the link href at parse time, so it
        // takes the same placeholder path — the tab destination must
        // survive the save unencoded and reach a fixpoint.
        let (first, second) = two_saves("[notes][r]\n\n[r]: <a\tb.md>\n");
        assert!(
            first.contains("<a\tb.md>"),
            "tab destination kept: {first:?}"
        );
        assert!(!first.contains("%09"), "no tab encoding: {first:?}");
        assert_eq!(first, second, "fixpoint");
        let doc = markdown_to_prosemirror(&first);
        assert_eq!(
            doc["content"][0]["content"][0]["marks"][0]["attrs"]["href"], "a\tb.md",
            "PM href unmutated (reference definition)"
        );
    }

    fn test_footnote_roundtrip_fixpoint() {
        // #945: a footnote reference + definition round-trip open->save
        // intact — emitted as [^label] syntax, never backslash-escaped,
        // never dropped — and the second save is byte-identical.
        let original = "Text with a note.[^1]\n\n[^1]: The footnote body.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^1]"),
            "footnote syntax survives save: {first:?}"
        );
        assert!(
            !first.contains("\\[^"),
            "footnote must not be backslash-escaped: {first:?}"
        );
        assert!(
            first.contains("The footnote body."),
            "definition body survives save: {first:?}"
        );
        assert_eq!(first, second, "footnote round-trip is a fixpoint");
    }

    #[test]
    fn test_multiple_footnotes_roundtrip_fixpoint() {
        // #945: several footnotes keep their own labels and bodies.
        let original = "One.[^a] Two.[^b]\n\n[^a]: First note.\n\n[^b]: Second note.\n";
        let (first, second) = two_saves(original);
        for needle in ["[^a]", "[^b]", "First note.", "Second note."] {
            assert!(first.contains(needle), "{needle} survives save: {first:?}");
        }
        assert!(!first.contains("\\[^"), "no escaped footnotes: {first:?}");
        assert_eq!(first, second, "multi-footnote round-trip is a fixpoint");
    }

    #[test]
    #[test]
    fn test_unreferenced_footnote_definition_survives_save() {
        // #1565: a definition nothing references is dropped by comrak
        // before conversion, so it used to vanish on save. The pre-scan
        // preserves it, and the round-trip is a fixpoint.
        let original = "Body text with no reference.\n\n[^orphan]: The orphan note.\n";
        let doc = markdown_to_prosemirror(original);
        let top = doc["content"].as_array().unwrap();
        let definition = top
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("unreferenced definition carried into the PM doc");
        assert_eq!(definition["attrs"]["label"], "orphan");
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^orphan]:"),
            "orphan definition survives save: {first:?}"
        );
        assert!(
            first.contains("The orphan note."),
            "orphan body survives save: {first:?}"
        );
        assert_eq!(first, second, "orphan round-trip is a fixpoint");
    }

    #[test]
    fn test_mixed_referenced_and_unreferenced_footnotes() {
        // #1565: the referenced definition keeps today's exact path;
        // the unreferenced one is appended by the pre-scan, not
        // duplicated or dropped.
        let original = "Text.[^used]\n\n[^used]: Used note.\n\n[^unused]: Unused note.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .filter_map(|n| n["attrs"]["label"].as_str())
            .collect();
        assert_eq!(labels, ["used", "unused"]);
        let (first, second) = two_saves(original);
        for needle in ["[^used]:", "Used note.", "[^unused]:", "Unused note."] {
            assert!(first.contains(needle), "{needle} survives save: {first:?}");
        }
        assert_eq!(first, second, "mixed round-trip is a fixpoint");
    }

    #[test]
    fn test_definitions_only_document_does_not_save_empty() {
        // #1565: a document that is only a footnote definition saved
        // as an empty file -- the data-loss case from the issue.
        let original = "[^only]: The only content.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^only]:") && first.contains("The only content."),
            "definitions-only document keeps its definition: {first:?}"
        );
        assert_eq!(first, second, "definitions-only round-trip is a fixpoint");
    }

    #[test]
    fn test_list_paragraph_html_line_does_not_swallow_footnote() {
        // #1565 twenty-third cut (Codex P2): `- Body` is paragraph
        // text inside the list item, so the indented `<em>` line
        // continues that paragraph -- a type-7 HTML block cannot
        // interrupt it. The footnote scanner treated the marker
        // line as a block start, opened the HTML tracker at the
        // tag, and swallowed the heading and the genuine orphan
        // definition as HTML interior, deleting the note on save.
        let original = "- Body\n  <em>\n# Heading\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .filter_map(|n| n["attrs"]["label"].as_str())
            .collect();
        assert_eq!(labels, ["real"]);
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^real]:") && first.contains("note"),
            "genuine definition after list-paragraph HTML line survives: {first:?}"
        );
        assert_eq!(first, second, "list-paragraph round-trip is a fixpoint");
    }

    #[test]
    fn test_indented_heading_in_footnote_blocks_lazy_continuation() {
        // #1565 twenty-third cut (Codex P2): an indented heading
        // inside a footnote definition is a new block and cannot
        // accept an unindented lazy continuation, so `<em>` after
        // it starts a top-level HTML block and the definition-
        // shaped line inside it is literal text, not a footnote.
        // The scanner set lazy eligibility unconditionally on the
        // indented line, consumed the tag into the definition,
        // and collected the literal line as a real definition.
        let original = "[^a]: First.\n\n    # Heading\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .filter_map(|n| n["attrs"]["label"].as_str())
            .collect();
        assert_eq!(labels, ["a"], "literal line must not become a definition");
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^a]:") && first.contains("First."),
            "real definition survives: {first:?}"
        );
        assert_eq!(first, second, "indented-heading round-trip is a fixpoint");
    }

    #[test]
    fn test_multiline_reference_label_resolves_in_recovered_footnote() {
        // #1565 fourteenth cut (Codex P2): a link reference
        // definition's label may span lines (`[long\nlabel]:`);
        // the collector's single-line eligibility check rejected
        // it, so the recovered footnote's [target][long label]
        // saved as literal brackets instead of a hyperlink.
        let original = "[^orphan]: [target][long label]\n\n[long\nlabel]: https://example.com\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[target](https://example.com)"),
            "multiline-label reference resolves into a link: {first:?}"
        );
        assert_eq!(first, second, "multiline-label round-trip is a fixpoint");
    }

    #[test]
    fn test_duplicate_footnote_definitions_appended_once() {
        // #1565 fourteenth cut (Codex P2): two orphan blocks with
        // the same label resolved to ONE synthesized node, which
        // the append loop cloned for both blocks -- both saved
        // definitions carried the same body and one original body
        // was overwritten. The resolved definition is appended
        // once per label (first definition wins).
        let original = "[^a]: First.\n\n[^a]: Second.\n";
        let doc = markdown_to_prosemirror(original);
        let defs: Vec<&Value> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .collect();
        assert_eq!(defs.len(), 1, "duplicate label yields one definition node");
        let (first, second) = two_saves(original);
        assert_eq!(
            first.matches("[^a]:").count(),
            1,
            "duplicate definition saved once: {first:?}"
        );
        assert!(
            first.contains("First."),
            "first definition body wins: {first:?}"
        );
        assert_eq!(first, second, "duplicate-label round-trip is a fixpoint");
    }

    #[test]
    fn test_list_paragraph_line_is_not_a_reference_definition() {
        // #1565 fourteenth cut (Codex P2): `  [url]: ...`
        // continuing a list item's paragraph is paragraph text,
        // not a link reference definition, so the orphan
        // footnote's literal [target][url] must NOT resolve into
        // a hyperlink.
        let original = "- Body\n  [url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            !first.contains("](https://example.com)") && first.contains("\\[target\\]"),
            "list-continued reference stays literal (no link formed): {first:?}"
        );
        assert_eq!(first, second, "list-paragraph round-trip is a fixpoint");
    }

    #[test]
    fn test_definition_after_list_scoped_fence_survives() {
        // #1565 fourteenth cut (Codex P2): an unterminated fence
        // inside a list item ends when the item ends; the
        // top-level definition after it is a real definition, not
        // fence interior.
        let original = "- ```\n  code\n\n[^real]: The note.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["real"], "definition after list fence is recovered");
        let (first, second) = two_saves(original);
        assert!(first.contains("The note."), "note survives save: {first:?}");
        assert_eq!(first, second, "list-fence round-trip is a fixpoint");
    }

    #[test]
    fn test_definition_after_container_scoped_html_comment_survives() {
        // #1565 eleventh cut (Codex P2): an HTML comment opened
        // inside a blockquote ends when the quote ends, even
        // without its --> marker; the top-level definition after
        // it is a real definition, not comment interior.
        let original = "> <!--\n> hidden\n\n[^real]: note\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^real]:") && first.contains("note"),
            "definition after container-ended comment survives: {first:?}"
        );
        assert_eq!(first, second, "container HTML round-trip is a fixpoint");
    }

    #[test]
    fn test_container_paragraph_line_is_not_a_reference_definition() {
        // #1565 eleventh cut (Codex P2): `> [url]: ...` continuing
        // a blockquote paragraph is paragraph text, not a link
        // reference definition, so the orphan footnote's literal
        // [target][url] must NOT resolve into a hyperlink.
        let original = "> Body\n> [url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            !first.contains("](https://example.com)") && first.contains("\\[target\\]"),
            "unresolved reference stays literal (no link formed): {first:?}"
        );
        assert_eq!(
            first, second,
            "container paragraph round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_inline_html_continuation_of_definition_survives() {
        // #1565 eleventh cut (Codex P2): a line starting with an
        // inline tag (<em>) continues the footnote definition's
        // paragraph (HTML type 7 cannot interrupt a paragraph);
        // only block-level HTML ends collection.
        let original = "[^a]: First line.\n<em>second</em>\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First line.") && first.contains("second"),
            "inline HTML continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "inline HTML round-trip is a fixpoint");
    }

    #[test]
    fn test_type7_html_after_paragraph_does_not_swallow_footnote() {
        // #1565 eighteenth cut (Codex P2): a lone inline tag cannot
        // interrupt a paragraph (CommonMark type 7), so in
        // `Body` / `<em>` / `# Heading` / `[^real]: note` the tag is
        // paragraph text, the heading still starts a block, and the
        // unreferenced definition after it is live -- the source
        // pre-scan must recover it, not swallow it as the interior
        // of an HTML block opened at the tag.
        let original = "Body\n<em>\n# Heading\n[^real]: note\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^real]:") && first.contains("note"),
            "definition after type-7 continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "type-7 paragraph round-trip is a fixpoint");
    }

    #[test]
    fn test_reference_shaped_line_continues_footnote_paragraph() {
        // #1565 twelfth cut (Codex P2): a link reference
        // definition cannot interrupt a paragraph, so a
        // reference-shaped line after a footnote definition's
        // first line is its lazy continuation and must survive
        // as literal text, not stop collection.
        let original = "[^a]: First line.\n[url]: https://example.com\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First line.")
                && first.contains("\\[url\\]:")
                && !first.contains("[url](https://example.com)")
                && first.contains("https://example.com"),
            "reference-shaped continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "reference continuation is a fixpoint");
    }

    #[test]
    fn test_escaped_bracket_in_reference_label_resolves() {
        // #1565 twelfth cut (Codex P2): a \] inside a reference
        // label is label text; the definition must still be
        // collected (at the unescaped close) so the orphan
        // footnote's reference-style link resolves on save.
        let original = "[a\\]b]: https://example.com\n\n[^orphan]: [target][a\\]b]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("](https://example.com)"),
            "escaped-label reference resolves to a link: {first:?}"
        );
        assert_eq!(first, second, "escaped label round-trip is a fixpoint");
    }

    #[test]
    fn test_ordered_list_marker_two_continues_footnote_paragraph() {
        // #1565 thirteenth cut (Codex P2): an ordered list can
        // interrupt a paragraph only when it starts at 1, so
        // `2. ...` after a footnote definition's first line is a
        // lazy continuation of the definition, not a new block.
        let original = "[^a]: First line.\n2. second line.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First line.") && first.contains("second line."),
            "non-1 ordered marker continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "ordered continuation is a fixpoint");
    }

    #[test]
    fn test_empty_bullet_ends_footnote_like_comrak() {
        // #1565 nineteenth cut (Codex P2, DECLINED with evidence):
        // Codex proposed that an empty bullet cannot interrupt a
        // paragraph, so `* ` and `second line.` after
        // `[^a]: First.` belong to the footnote. Ground truth is
        // comrak, whose boundaries this codec mirrors: parsing
        // the REFERENCED document (`x[^a]` + this definition)
        // puts only `First.` in the definition -- comrak ends the
        // definition at the empty bullet and parses the bullet as
        // a top-level empty list item and `second line.` as a
        // body paragraph. The round trip below preserves every
        // line and is a fixpoint, so nothing is lost on save and
        // the scanner keeps mirroring comrak (see the PR thread
        // for the probe output).
        let original = "[^a]: First.\n* \nsecond line.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First.") && first.contains("second line.") && first.contains("- "),
            "all lines survive the round trip: {first:?}"
        );
        assert_eq!(first, second, "empty-bullet round-trip is a fixpoint");
    }

    #[test]
    fn test_unterminated_angle_destination_does_not_swallow_blocks() {
        // #1565 nineteenth cut (Codex P2): `[bad]: <unclosed`
        // is malformed; the heading after it ends the paragraph
        // and `[url]:` is a valid definition. The collector's
        // boundary checks ran only while nothing followed the
        // colon, so the unterminated angle destination consumed
        // both lines and the whole candidate was discarded,
        // leaving the orphan's link literal.
        let original =
            "[bad]: <unclosed\n# Heading\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("](https://example.com)"),
            "valid definition after malformed angle destination resolves: {first:?}"
        );
        assert_eq!(first, second, "angle-destination boundary is a fixpoint");
    }

    #[test]
    fn test_empty_fence_resets_block_start_for_type7_html() {
        // #1565 nineteenth cut (Codex P2): an empty fenced block
        // ends the preceding paragraph, so `<em>` after it opens
        // a type-7 HTML block and the definition-shaped line is
        // its literal interior. The footnote scan's fence branch
        // never reset block-start eligibility (the reference
        // collector's did), so the tag was refused and the fake
        // definition recovered as a live footnote.
        let original = "Body\n```\n```\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        assert!(
            doc["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["type"] != "footnote_definition"),
            "definition-shaped HTML interior is not synthesized: {doc}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(first, second, "type-7-after-fence round-trip is a fixpoint");
    }

    #[test]
    fn test_setext_heading_resets_block_start_for_type7_html() {
        // #1565 twentieth cut (Codex P2): a setext underline ends
        // the paragraph it underlines, so `<em>` after `Body` /
        // `===` opens a type-7 HTML block and the definition-
        // shaped line is its literal interior. The footnote scan
        // did not recognize setext underlines when updating
        // paragraph eligibility (the reference collector already
        // did), so the tag was refused and the fake definition
        // recovered as a live footnote.
        let original = "Body\n===\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        assert!(
            doc["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["type"] != "footnote_definition"),
            "definition-shaped HTML interior after a setext heading is not synthesized: {doc}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(
            first, second,
            "type-7-after-setext round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_tab_indented_code_resets_block_start_for_type7_html() {
        // #1565 twentieth cut (Codex P2): a tab-indented first
        // line is indented code (tab advances to column 4), so
        // `<em>` after it opens an HTML block whose interior is
        // literal. The footnote scan counted spaces only, treating
        // the code line as paragraph content, refusing the tag,
        // and recovering the fake definition. The four-space
        // spelling is the control: both must scan identically.
        for original in [
            "\tcode\n<em>\n[^fake]: literal\n\n",
            "    code\n<em>\n[^fake]: literal\n\n",
        ] {
            let doc = markdown_to_prosemirror(original);
            assert!(
                doc["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|n| n["type"] != "footnote_definition"),
                "definition-shaped HTML interior after indented code is not synthesized: {doc}"
            );
            let (first, second) = two_saves(original);
            assert_eq!(
                first, second,
                "type-7-after-indented-code round-trip is a fixpoint"
            );
        }
    }

    #[test]
    fn test_invalid_destination_reference_candidate_does_not_split_paragraph() {
        // #1565 twenty-fourth cut (Codex P2): `[bad]: /bad(` is
        // no definition -- the destination itself is invalid (an
        // unmatched parenthesis) even though nothing follows it.
        // The validator accepted it on its empty title alone, so
        // `[url]: /target` was collected as a separate definition
        // and synthesis promoted the recovered footnote's
        // `[target][url]` into a hyperlink the document never had.
        let original = "[bad]: /bad(\n[url]: /target\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[target\\]\\[url\\]"),
            "unresolved reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[target]("),
            "invalid-destination reference is not promoted to a link: {first:?}"
        );
        assert_eq!(
            first, second,
            "invalid-destination round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_indented_fence_in_blockquote_hides_reference_definition() {
        // #1565 twenty-fourth cut (Codex P2): the fence inside the
        // blockquote is indented two columns relative to the
        // container; container stripping left those spaces in
        // place and FenceTracker anchors at the first character,
        // so the opening was missed and the reference collector
        // collected `[url]: /fake` out of literal code -- synthesis
        // then resolved the orphan footnote's `[x][url]` into a
        // hyperlink. With the fence recognized, the definition
        // stays code and the reference stays literal, while the
        // real footnote definition is still recovered.
        let original = ">   ```\n>\n> [url]: /fake\n>   ```\n\n[^a]: [x][url]\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(
            found,
            "real footnote definition after the blockquote fence is recovered"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[x\\]\\[url\\]"),
            "code-interior reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[x]("),
            "fenced reference definition is not promoted to a link: {first:?}"
        );
        assert_eq!(
            first, second,
            "indented blockquote fence round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_indented_code_does_not_enable_footnote_lazy_continuation() {
        // #1565 twenty-fifth cut (Codex P2): remove only the
        // footnote's four-column continuation indent before deciding
        // whether an indented line is paragraph content. The
        // remaining four columns here are code, so the following
        // type-7 HTML block must be handled by the outer scan and its
        // definition-shaped interior must not be synthesized.
        let original = "[^a]: First.\n\n        code\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<_> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "footnote_definition")
            .filter_map(|node| node["attrs"]["label"].as_str())
            .collect();
        assert_eq!(
            labels,
            ["a"],
            "indented code must not synthesize fake: {doc}"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First."),
            "footnote body survives: {first:?}"
        );
        assert!(first.contains("code"), "indented code survives: {first:?}");
        assert_eq!(first, second, "indented-code continuation is a fixpoint");
    }

    #[test]
    fn test_setext_heading_in_footnote_ends_lazy_continuation() {
        // #1565 twenty-sixth cut (Codex P2): an indented setext
        // underline turns the footnote's first line into a heading
        // inside the definition, so the following unindented
        // `<em>` starts a top-level HTML block and the
        // definition-shaped line inside it is literal. The inner
        // continuation update ignored setext underlines, kept
        // lazy eligibility, swallowed `<em>` into the footnote,
        // and promoted the fake definition on save.
        let original = "[^a]: Title\n    ===\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<_> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "footnote_definition")
            .filter_map(|node| node["attrs"]["label"].as_str())
            .collect();
        assert_eq!(
            labels,
            ["a"],
            "setext heading in footnote must not synthesize fake: {doc}"
        );
        let (first, second) = two_saves(original);
        assert!(first.contains("Title"), "footnote body survives: {first:?}");
        assert_eq!(first, second, "setext-in-footnote round-trip is a fixpoint");
    }

    #[test]
    fn test_fenced_block_in_footnote_ends_lazy_continuation() {
        // #1565 twenty-seventh cut (Codex P2): a footnote whose
        // first block is a fenced code block -- the indented `code`
        // line is fence content, not a paragraph line, so the
        // following unindented `<em>` is not a lazy continuation:
        // it starts a top-level HTML block and the definition-shaped
        // line inside it is literal. The continuation loop
        // classified lines independently, treated `code` as
        // paragraph content, swallowed `<em>` into the footnote,
        // and promoted the fake definition on save.
        let original = "[^a]: ```\n    code\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<_> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "footnote_definition")
            .filter_map(|node| node["attrs"]["label"].as_str())
            .collect();
        assert_eq!(
            labels,
            ["a"],
            "fenced block in footnote must not synthesize fake: {doc}"
        );
        let (first, second) = two_saves(original);
        assert!(first.contains("code"), "fence content survives: {first:?}");
        assert_eq!(first, second, "fenced-block round-trip is a fixpoint");
        let saved_doc = markdown_to_prosemirror(&first);
        let saved_labels: Vec<_> = saved_doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "footnote_definition")
            .filter_map(|node| node["attrs"]["label"].as_str())
            .collect();
        assert_eq!(
            saved_labels,
            ["a"],
            "saved form must not promote fake either: {saved_doc}"
        );
    }

    #[test]
    fn test_overindented_continuation_keeps_footnote_paragraph() {
        // #1565 twenty-eighth cut (Codex P2): an eight-space line
        // continues the footnote's open paragraph (indented code
        // cannot interrupt a paragraph), so the unindented
        // `second.` after it is a lazy continuation of the same
        // paragraph. The continuation loop reset eligibility on
        // the overindented line, truncated the collected
        // definition there, and saving deleted `second.`.
        let original = "[^a]: First.\n        continuation\nsecond.\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("second."),
            "overindented continuation keeps the lazy line: {orphan}"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("second."),
            "lazy line survives save: {first:?}"
        );
        assert_eq!(
            first, second,
            "overindented-continuation round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_html_block_in_footnote_does_not_promote_fake_definition() {
        // #1565 twenty-eighth cut (Codex P2): a comment block
        // opened by the footnote's own content takes no lazy
        // continuation: the unindented `<em>` ends the definition
        // and opens a top-level HTML block whose
        // definition-shaped interior is literal. The continuation
        // loop tracked fenced blocks inside the definition but
        // not HTML blocks, lazy-consumed `<em>` into the
        // footnote, bypassed the outer HTML tracker, and
        // promoted `[^fake]:` to a real footnote on save.
        let original = "[^a]: <!--\n    code\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<_> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "footnote_definition")
            .filter_map(|node| node["attrs"]["label"].as_str())
            .collect();
        assert_eq!(
            labels,
            ["a"],
            "HTML block in footnote must not synthesize fake: {doc}"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("code"),
            "comment content survives: {first:?}"
        );
        assert_eq!(first, second, "HTML-in-footnote round-trip is a fixpoint");
        let saved_doc = markdown_to_prosemirror(&first);
        let saved_labels: Vec<_> = saved_doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "footnote_definition")
            .filter_map(|node| node["attrs"]["label"].as_str())
            .collect();
        assert_eq!(
            saved_labels,
            ["a"],
            "saved form must not promote fake either: {saved_doc}"
        );
    }

    #[test]
    fn test_blockquote_multiline_reference_definition_resolves() {
        // #1565 twenty-eighth cut (Codex P2): a reference
        // definition inside a blockquote with its destination on
        // the following quote line is document-global, but the
        // collector's boundary check treated the continuing `>`
        // marker as a new block and stopped before the
        // destination, so the definition never reached the
        // synthesis context and the recovered footnote's
        // `[x][url]` saved as literal brackets.
        let original = "> [url]:\n> /target\n\n[^a]: [x][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("/target"),
            "blockquote multiline destination resolves the orphan link: {orphan}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(
            first, second,
            "blockquote-definition round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_empty_reference_labels_do_not_split_paragraph() {
        // #1565 twenty-fifth cut (Codex P2): empty and
        // whitespace-only labels are not link reference definitions.
        // Treating either as a block boundary promoted the following
        // `[url]` out of the same paragraph and resolved a link that
        // remains literal in the source document.
        for invalid_label in ["[]", "[   ]"] {
            let original =
                format!("{invalid_label}: /bad\n[url]: /target\n\n[^orphan]: [target][url]\n");
            let (first, second) = two_saves(&original);
            assert!(
                first.contains("\\[target\\]\\[url\\]"),
                "invalid label must not promote the following definition: {first:?}"
            );
            assert!(
                !first.contains("[target](/target)"),
                "invalid-label candidate must not resolve the orphan link: {first:?}"
            );
            assert_eq!(first, second, "invalid-label round-trip is a fixpoint");
        }
    }

    #[test]
    fn test_malformed_reference_candidate_does_not_split_paragraph() {
        // #1565 twentieth cut (Codex P2): `[bad]: /bad extra` is
        // no definition -- the bare trailing word is not a title --
        // so it and `[url]: /target` form ONE paragraph and
        // `[url]` inside it is not a definition either. The
        // collector accepted the malformed candidate and reset
        // paragraph state, collected `[url]` separately, and
        // synthesis then resolved the recovered footnote's
        // `[target][url]` into a hyperlink the document never had.
        let original = "[bad]: /bad extra\n[url]: /target\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[target\\]\\[url\\]"),
            "unresolved reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[target]("),
            "malformed-candidate reference is not promoted to a link: {first:?}"
        );
        assert_eq!(
            first, second,
            "malformed-candidate round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_overindented_html_line_inside_footnote_is_code() {
        // #1565 twenty-ninth cut (Codex P2): inside a footnote, a
        // line retaining four indentation columns after the
        // container's four are removed is indented code, not an
        // HTML block opener. Trimming all its spaces opened a
        // phantom HTML block, the real paragraph after it was
        // classified as HTML, and its lazy continuation `lazy`
        // was excluded from the definition and lost on save.
        let original = "[^a]: First.\n\n        <em>\n    paragraph\nlazy\n";
        let doc = markdown_to_prosemirror(original);
        let defs = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .count();
        assert_eq!(defs, 1, "the orphan definition is synthesized once: {doc}");
        let (first, second) = two_saves(original);
        assert!(
            first.contains("lazy"),
            "lazy continuation of the footnote paragraph survives: {first:?}"
        );
        assert!(
            first.contains("paragraph"),
            "footnote paragraph survives: {first:?}"
        );
        assert_eq!(
            first, second,
            "overindented-html footnote round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_overindented_fence_closer_inside_footnote_stays_code() {
        // #1565 twenty-ninth cut (Codex P2): a fence delimiter
        // retaining four indentation columns inside a footnote
        // cannot close the definition's fenced block -- it is
        // indented code. Accepting it as a closer ended the fence
        // early, the following lines were misclassified, and the
        // literal definition-shaped line inside the still-open
        // fence was promoted to a real footnote on save.
        let original = "[^a]: ```\n        ```\n    code\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let defs = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .count();
        assert_eq!(
            defs, 1,
            "only the real orphan definition is synthesized: {doc}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(
            first, second,
            "overindented-closer round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_reference_after_heading_body_footnote_resolves() {
        // #1565 twenty-ninth cut (Codex P2): a footnote whose
        // opening body is a heading cannot accept a lazy
        // continuation, so a reference definition on the next line
        // is a real top-level definition. Marking every footnote
        // opening as inside-a-footnote-paragraph rejected it, and
        // the recovered footnote's valid link lost its URL.
        let original = "[^a]: # Heading\n[url]: /target\n\n[^orphan]: [x][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[x](/target)"),
            "reference after a heading-body footnote resolves: {first:?}"
        );
        assert_eq!(
            first, second,
            "heading-body reference round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_multiline_reference_label_stops_at_heading() {
        // #1565 twenty-ninth cut (Codex P2): a heading interrupts
        // the paragraph an incomplete reference label is
        // accumulating in, so `[long` / `# Heading` / `label]:`
        // is not a multiline label. Accumulating across the
        // heading accepted the phantom label, reset paragraph
        // state, and promoted the following `[url]` definition
        // out of what is really one paragraph.
        let original = "[long\n# Heading\nlabel]: /bad\n[url]: /target\n\n[^a]: [x][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[x\\]\\[url\\]"),
            "interrupted-label reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[x](/target)"),
            "phantom multiline label must not promote the following definition: {first:?}"
        );
        assert_eq!(first, second, "interrupted-label round-trip is a fixpoint");
    }

    #[test]
    fn test_unescaped_open_paren_invalidates_reference_title() {
        // #1565 twenty-ninth cut (Codex P2): a parenthesized
        // title cannot contain an unescaped `(`, so
        // `[bad]: /bad (a(b)` is no definition. Accepting it
        // reset paragraph state and promoted the following
        // `[url]` line out of the same paragraph.
        let original = "[bad]: /bad (a(b)\n[url]: /target\n\n[^orphan]: [x][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[x\\]\\[url\\]"),
            "paren-title reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[x](/target)"),
            "invalid paren title must not promote the following definition: {first:?}"
        );
        assert_eq!(first, second, "paren-title round-trip is a fixpoint");
    }

    #[test]
    fn test_angle_destination_title_needs_whitespace() {
        // #1565 twenty-ninth cut (Codex P2): a title must be
        // separated from an angle destination's closing `>` by
        // whitespace, so `[bad]: </bad>"title"` is no definition.
        // Accepting it promoted the following `[url]` line out of
        // the same paragraph.
        let original = "[bad]: </bad>\"title\"\n[url]: /target\n\n[^orphan]: [x][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[x\\]\\[url\\]"),
            "angle-title reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[x](/target)"),
            "unseparated angle title must not promote the following definition: {first:?}"
        );
        assert_eq!(first, second, "angle-title round-trip is a fixpoint");
    }

    #[test]
    fn test_reference_in_sibling_list_item_after_fence_resolves() {
        // #1565 twenty-ninth cut (Codex P2): a sibling list item
        // ends a fenced block opened in the previous item. The
        // collector mistook the sibling's marker for a
        // continuation of the fence's own item, stayed inside the
        // fence, and never collected the sibling's reference
        // definition, so the recovered footnote lost its link.
        let original = "- ```\n- [url]: /target\n\n[^a]: [x][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[x](/target)"),
            "sibling-item reference after a fence resolves: {first:?}"
        );
        assert_eq!(first, second, "sibling-fence round-trip is a fixpoint");
    }

    #[test]
    fn test_nbsp_line_is_not_blank_for_reference_collection() {
        // #1565 twenty-ninth cut (Codex P2): only spaces and tabs
        // make a line blank in Markdown. A nonbreaking-space line
        // is paragraph text, so the definition-shaped line after
        // it is that paragraph's continuation, not a definition.
        // Unicode trimming treated it as blank, reset paragraph
        // state, and promoted the definition into the probe.
        let original = "Body\n\u{00A0}\n[url]: /target\n\n[^a]: [x][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[x\\]\\[url\\]"),
            "nbsp-paragraph reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[x](/target)"),
            "nbsp line must not reset paragraph state for the following definition: {first:?}"
        );
        assert_eq!(first, second, "nbsp round-trip is a fixpoint");
    }

    #[test]
    fn test_closing_special_tag_opens_type7_html_block() {
        // #1565 twenty-first cut (Codex P2): a complete CLOSING
        // tag opens a type-7 HTML block whatever its name -- the
        // script/pre/style/textarea exclusion applies only to
        // opening tags. `</pre>` therefore makes the following
        // definition-shaped line literal HTML interior; the
        // scanner rejected the tag and recovered the line as a
        // live footnote definition.
        let original = "</pre>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        assert!(
            doc["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["type"] != "footnote_definition"),
            "definition-shaped interior of a closing-tag HTML block is not synthesized: {doc}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(first, second, "closing-tag HTML round-trip is a fixpoint");
    }

    #[test]
    fn test_failed_following_line_title_keeps_destination() {
        // #1565 twenty-first cut (Codex P2): `[url]: /target`
        // followed by an unterminated title line is still a
        // valid destination-only definition; the title-looking
        // line is paragraph text. Appending it unconditionally
        // and then rejecting the whole candidate dropped the
        // definition from the synthesis context, so the recovered
        // footnote's `[target][url]` lost its URL on save.
        let original = "[url]: /target\n\"unterminated\n\n[^a]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[target](/target)"),
            "destination survives a failed following-line title: {first:?}"
        );
        assert_eq!(first, second, "failed-title round-trip is a fixpoint");
    }

    #[test]
    fn test_heading_body_footnote_accepts_no_lazy_continuation() {
        // #1565 twenty-first cut (Codex P2): `[^a]: # Heading`
        // opens the definition with a heading block, which cannot
        // accept an unindented lazy paragraph continuation, so
        // `<em>` starts a top-level HTML block and the
        // definition-shaped line inside it is literal. Treating
        // any nonempty opening body as paragraph content let the
        // definition's inner collection swallow `<em>` and
        // recover the fake definition too.
        let original = "[^a]: # Heading\n<em>\n[^fake]: literal\n\n";
        let doc = markdown_to_prosemirror(original);
        let defs = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .count();
        assert_eq!(
            defs, 1,
            "only the real orphan definition is synthesized: {doc}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(
            first, second,
            "heading-body footnote round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_escaped_angle_destination_keeps_reference_definition() {
        // #1565 twenty-first cut (Codex P2): in an angle
        // destination an escaped `\>` is destination text, not
        // the closing delimiter. Ending the destination there
        // left `b>` to be read as invalid trailing title text
        // and dropped the definition, so the recovered
        // footnote's valid hyperlink lost its URL on save.
        let original = "[url]: <https://example.com/a\\>b>\n\n[^a]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[target]("),
            "escaped-angle-destination reference resolves: {first:?}"
        );
        assert!(
            !first.contains("\\[target\\]\\[url\\]"),
            "reference is not left literal: {first:?}"
        );
        assert_eq!(first, second, "escaped-angle round-trip is a fixpoint");
    }

    #[test]
    fn test_overpadded_list_marker_backticks_are_code_not_fence() {
        // #1565 twenty-first cut (Codex P2): five spaces after a
        // bullet are one padding column plus four columns of
        // indented code, so the backticks are literal code, not a
        // fence opener. Stripping all the padding opened a
        // phantom fence that suppressed the valid reference
        // definition inside the item, and the recovered orphan's
        // hyperlink lost its URL on save.
        let original = "-     ```\n\n  [url]: /target\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[target](/target)"),
            "reference after overpadded-marker code resolves: {first:?}"
        );
        assert_eq!(first, second, "overpadded-marker round-trip is a fixpoint");
    }

    #[test]
    fn test_indented_continuation_preserves_paragraph_context() {
        // #1565 twenty-second cut (Codex P2): an indented line
        // continuing a paragraph is not indented code (code
        // cannot interrupt a paragraph), so the paragraph state
        // survives it and `<em>` on the next line cannot open a
        // type-7 HTML block. Resetting eligibility at the
        // indented line let the tracker swallow the heading and
        // the genuine definition, and recovery lost `real`.
        let original = "Body\n    continuation\n<em>\n# Heading\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        assert!(
            doc["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["type"] == "footnote_definition"),
            "the genuine definition after the paragraph is recovered: {doc}"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^real]:"),
            "recovered definition survives the save: {first:?}"
        );
        assert_eq!(
            first, second,
            "indented-continuation round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_trailing_text_after_title_invalidates_reference() {
        // #1565 twenty-second cut (Codex P2): a title must END
        // a reference definition -- `[bad]: /bad "title" extra`
        // is no definition, so `[url]: /target` stays inside the
        // same paragraph and is no definition either. Accepting
        // any title with a closing delimiter collected both
        // lines separately, and synthesis then resolved the
        // recovered footnote's `[target][url]` into a hyperlink
        // the document never had.
        let original = "[bad]: /bad \"title\" extra\n[url]: /target\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[target\\]\\[url\\]"),
            "unresolved reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[target]("),
            "trailing-text reference is not promoted to a link: {first:?}"
        );
        assert_eq!(
            first, second,
            "trailing-text-after-title round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_blockquote_fence_closes_without_space_after_marker() {
        // #1565 nineteenth cut (Codex P2): a fence opened under
        // `> ` may close under `>` -- the space after the quote
        // marker is optional, so byte-exact prefix matching left
        // the fence open and suppressed the reference definition
        // after it, degrading the orphan's link to literal text.
        let original =
            "> ```\n> code\n>```\n> [url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("](https://example.com)"),
            "definition after structurally-closed quote fence resolves: {first:?}"
        );
        assert_eq!(first, second, "quote-fence round-trip is a fixpoint");
    }

    #[test]
    fn test_unreferenced_multi_paragraph_definition_fixpoint() {
        // #1565 + #1031: an unreferenced multi-paragraph definition is
        // synthesized with its block children, not flattened.
        let original = "Body.\n\n[^m]: First paragraph.\n\n    Second paragraph.\n";
        let doc = markdown_to_prosemirror(original);
        let top = doc["content"].as_array().unwrap();
        let definition = top
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("unreferenced multi-paragraph definition present");
        let child_types: Vec<&str> = definition["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["type"].as_str())
            .collect();
        assert_eq!(child_types, ["paragraph", "paragraph"]);
        let (first, second) = two_saves(original);
        assert_eq!(first, second, "multi-paragraph orphan is a fixpoint");
    }

    #[test]
    fn test_reference_definition_in_new_list_item_resolves() {
        // #1565 fifteenth cut (Codex P2): a new list item starts a
        // fresh block, so a reference definition inside the second
        // item is document-global even though the first item's
        // content is paragraph text; the orphan footnote's
        // reference-style link must resolve on save.
        let original = "- Body\n- [url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("](https://example.com)"),
            "reference in a new list item resolves to a link: {first:?}"
        );
        assert_eq!(
            first, second,
            "list item reference round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_list_scoped_comment_ends_before_real_definition() {
        // #1565 fifteenth cut (Codex P2): an HTML comment opened
        // inside a list item ends when the item ends; a real
        // definition at column zero after it must still be
        // recovered, not treated as comment interior.
        let original = "- <!--\n  hidden\n\n[^real]: note\n";
        let (first, _second) = two_saves(original);
        assert!(
            first.contains("[^real]:") && first.contains("note"),
            "definition after list-scoped comment survives: {first:?}"
        );
        // Known residual (noted on the PR for the next cut): the
        // first save re-emits the comment WITHOUT its list marker
        // on the opening line (`- ` becomes a bare item line and
        // `<!--` an indented line of its own), so on a second save
        // the pre-scan reads the comment as a top-level block and
        // the definition is inside it again. Fixing that shape
        // needs list-context tracking across lines in the scans;
        // the authored shape above is what the Codex finding
        // names, and it now survives.
    }

    #[test]
    fn test_invalid_backtick_fence_info_line_continues_definition() {
        // #1565 fifteenth cut (Codex P2): a backtick fence's info
        // string cannot contain a backtick, so ```a`b opens no
        // fence at all -- it is paragraph text and a lazy
        // continuation of the footnote definition, not a new block
        // that truncates collection.
        let original = "[^a]: First.\n```a`b\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First.") && first.contains("a\\`b"),
            "invalid fence line continues the definition: {first:?}"
        );
        assert_eq!(first, second, "invalid fence round-trip is a fixpoint");
    }

    #[test]
    fn test_space_tab_indented_continuation_belongs_to_definition() {
        // #1565 fifteenth cut (Codex P2): indentation is counted
        // in columns, so a space followed by a tab is four columns
        // and the line is the footnote definition's second
        // paragraph, not a truncation point.
        let original = "[^a]: First.\n\n \tSecond paragraph.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First.") && first.contains("Second paragraph."),
            "space+tab continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "space+tab continuation is a fixpoint");
    }

    #[test]
    fn test_footnote_definition_inside_fence_not_synthesized() {
        // #1565: a definition-shaped line inside a fenced code block
        // is literal text; the pre-scan must not invent a definition
        // for it.
        let original = "Text.\n\n```\n[^fake]: Not a definition.\n```\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(!found, "fenced definition-shaped text stays literal");
    }

    #[test]
    fn test_footnote_definition_inside_html_comment_not_synthesized() {
        // Codex P2 on #1565's second cut: a definition-shaped line in
        // an HTML comment's interior is literal text (comrak keeps
        // the whole comment as an HTML block); the pre-scan must not
        // synthesize a live footnote out of it, while a real orphan
        // definition after the comment is still recovered.
        let original = "<!--\n[^fake]: literal\n-->\n\n[^real]: The real orphan.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["real"], "only the real orphan is recovered");
        let (first, second) = two_saves(original);
        assert_eq!(
            first.matches("[^fake]:").count(),
            1,
            "the fake definition appears only inside the preserved comment: {first:?}"
        );
        assert_eq!(first, second, "html-comment round-trip is a fixpoint");
    }

    #[test]
    fn test_unreferenced_footnote_referencing_another_footnote() {
        // Codex P2 on #1565's third cut: the synthesis probe for an
        // unreferenced definition that references another footnote
        // must include the inner definition, or comrak restores the
        // unresolved reference as escaped literal text and the
        // recovered note loses its footnote_reference node.
        let original = "Body.\n\n[^outer]: See [^inner].\n\n[^inner]: Inner note.\n";
        let doc = markdown_to_prosemirror(original);
        let outer = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition" && n["attrs"]["label"] == "outer")
            .expect("outer definition recovered");
        let outer_json = outer.to_string();
        assert!(
            outer_json.contains("footnote_reference"),
            "outer note keeps its footnote_reference node: {outer_json}"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[^inner]:") && first.contains("Inner note."),
            "inner definition survives save: {first:?}"
        );
        assert!(
            !first.contains("\\[^inner]"),
            "inner reference is not escaped literal text: {first:?}"
        );
        assert_eq!(first, second, "nested-footnote round-trip is a fixpoint");
    }

    #[test]
    fn test_footnote_definition_inside_list_contained_fence_not_synthesized() {
        // Codex P2 on #1565's fourth cut: a fence opened inside a
        // list item (`- ```) makes its indented interior literal
        // code; the pre-scan must see the fence behind the list
        // marker and not recover the interior line as a definition.
        let original = "Text.\n\n- ```\n  [^fake]: literal\n  ```\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(!found, "list-contained fenced text stays literal");
    }

    #[test]
    fn test_container_marker_inside_open_fence_stays_literal() {
        // Codex P2 on #1565's fifth cut: container prefixes are
        // stripped only to find a fence's opening line -- a `> ````
        // line inside an open top-level fence is literal content,
        // not a closing delimiter.
        let original = "Text.\n\n```\n> ```\n[^fake]: literal\n```\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(
            !found,
            "fence interior stays literal past a container-shaped line"
        );
    }

    #[test]
    fn test_paragraph_continuation_reference_line_not_promoted() {
        // Codex P2 on #1565's fifth cut: a definition-shaped line
        // continuing a paragraph is paragraph text, not a link
        // reference definition; the synthesis context must not
        // promote it, or the recovered footnote's bracketed text
        // becomes a hyperlink the document never had.
        let original = "Body line\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            !orphan.to_string().contains("example.com"),
            "paragraph text is not a reference definition: {orphan}"
        );
    }

    #[test]
    fn test_blockquote_contained_fence_closes_and_real_definition_recovered() {
        // Codex P2 on #1565's sixth cut: a fence genuinely inside a
        // blockquote opens AND closes behind the container prefix;
        // remembering the opening prefix lets the closing delimiter
        // register, so the real definition after the quote is not
        // swallowed by the still-open fence.
        let original = "> ```\n> code\n> ```\n\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(found, "definition after a blockquote fence is recovered");
    }

    #[test]
    fn test_lowercase_cdata_does_not_suppress_footnote() {
        // #1565 sixteenth cut (Codex P2): a CDATA block requires
        // the exact uppercase `<![CDATA[` opener; lowercase
        // `<![cdata[` is ordinary text and must not open a
        // marker-ended HTML block that suppresses the definition.
        let original = "<![cdata[\n\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(found, "definition after lowercase cdata text is recovered");
    }

    #[test]
    fn test_block_tag_name_boundary_lazy_continuation() {
        // #1565 sixteenth cut (Codex P2): `<div=foo>` is not a
        // type-6 HTML block (a tag name requires whitespace, `/`,
        // `>`, or end-of-line after it), so the line lazily
        // continues the footnote definition's paragraph.
        let original = "[^a]: First line.\n<div=foo>\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First line.") && first.contains("div=foo"),
            "tag-like continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "tag-like continuation is a fixpoint");
    }

    #[test]
    fn test_reference_definition_after_tab_indented_code_collected() {
        // #1565 sixteenth cut (Codex P2): a tab-indented line is
        // an indented code block (tab = columns to the next
        // multiple of 4), so a reference definition on the next
        // line starts a new block and is collected.
        let original = "\tcode\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("example.com"),
            "reference resolves after tab-indented code: {orphan}"
        );
    }

    #[test]
    fn test_reference_definitions_after_html_block_collected() {
        // Codex P2 on #1565's sixth cut: the blank line ending an
        // HTML block must reach the HTML tracker, or the collector
        // treats every later line as HTML and returns no reference
        // definitions, degrading the orphan's reference-style link.
        let original = "<div>\n</div>\n\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("example.com"),
            "reference-style link resolves after an HTML block: {orphan}"
        );
    }

    #[test]
    fn test_multiline_link_reference_definition_destination_on_next_line() {
        // Codex P2 on #1565's seventh cut: a valid definition with
        // its destination on the following line was omitted from the
        // synthesis context (the text after the colon is empty), so
        // the orphan's reference-style link degraded to literal text.
        let original = "[url]:\n    https://example.com\n\n[^orphan]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("example.com"),
            "multiline destination resolves the orphan link: {orphan}"
        );
        let (first, second) = two_saves(original);
        assert_eq!(
            first, second,
            "multiline-definition round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_multiline_link_reference_definition_title_on_next_line() {
        // Codex P2 on #1565's seventh cut: a definition whose title
        // sits on the following line was collected without it, so
        // the recovered footnote's link silently lost its title.
        let original =
            "[url]: https://example.com\n    \"Target title\"\n\n[^orphan]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("Target title"),
            "continuation-line title is kept on the orphan link: {orphan}"
        );
    }

    #[test]
    fn test_autolink_line_does_not_suppress_footnote() {
        // #1565 seventeenth cut (Codex P2): `<https://example.com>`
        // is an autolink, not a type-7 HTML block -- the generic
        // tag check must validate complete tag syntax (a tag name
        // cannot be followed by `:`), or the line opens a
        // blank-line-ended HTML block that swallows the heading
        // and the valid orphan definition after it.
        let original = "<https://example.com>\n# Heading\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(found, "definition after an autolink line is recovered");
    }

    #[test]
    fn test_escaped_quote_in_multiline_reference_title() {
        // #1565 seventeenth cut (Codex P2): an escaped quote inside
        // a multiline reference title is title text, not its closing
        // delimiter; treating the title as complete stopped
        // collection before the closing line, the probe received an
        // unterminated definition, and the recovered orphan lost
        // its hyperlink.
        let original =
            "[url]: https://example.com \"first \\\"quoted\nsecond\"\n\n[^a]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("example.com"),
            "escaped-quote title resolves the orphan link: {orphan}"
        );
        assert!(
            orphan.to_string().contains("second"),
            "title closing line is collected: {orphan}"
        );
    }

    #[test]
    fn test_fragment_destination_on_continuation_line() {
        // #1565 seventeenth cut (Codex P2): `#section` on the line
        // after `[url]:` is a valid fragment destination, not an
        // ATX heading (a heading needs whitespace or end-of-line
        // after the hashes); treating every `#` line as a block
        // boundary discarded the incomplete definition.
        let original = "[url]:\n#section\n\n[^orphan]: [target][url]\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("#section"),
            "fragment destination resolves the orphan link: {orphan}"
        );
    }

    #[test]
    fn test_indented_html_comment_close_releases_reference_definitions() {
        // Codex P2 on #1565's seventh cut: an indented closing
        // marker ends the real HTML comment, but the collector's
        // indent gate never fed it to the tracker, so the tracker
        // stayed open and suppressed the valid definition after it.
        let original = "[^orphan]: [target][url]\n\n<!--\n    -->\n\n[url]: https://example.com\n";
        let doc = markdown_to_prosemirror(original);
        let orphan = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("orphan definition recovered");
        assert!(
            orphan.to_string().contains("example.com"),
            "definition after an indented comment close is collected: {orphan}"
        );
    }

    #[test]
    fn test_indented_html_comment_close_releases_footnote_definitions() {
        // Codex P2 on #1565's seventh cut, pre-scan half: the same
        // indent gate in the footnote scan kept the comment open
        // forever, so a real orphan after it was never recovered.
        let original = "<!--\n    -->\n\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(
            found,
            "definition after an indented comment close is recovered"
        );
    }

    #[test]
    fn test_tag_with_special_prefix_name_is_blank_line_terminated() {
        // Codex P2 on #1565's sixth cut: `<prefix>` is not a `<pre>`
        // block -- tag-named HTML prefixes require a name boundary,
        // so the generic block ends at the blank line and the real
        // definition after it is recovered.
        let original = "<prefix>\nHTML text\n\n[^real]: note\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(found, "definition after a <prefix> block is recovered");
    }

    #[test]
    fn test_malformed_footnote_label_does_not_recurse() {
        // Codex P1 on #1565's first cut: `[^a]b]: note` scanned as a
        // definition with label `a]b`, which comrak does not
        // recognize; synthesizing it re-ran recovery forever.
        let original = "Text.\n\n[^a]b]: note\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(!found, "malformed label is not a definition");
    }

    #[test]
    fn test_footnote_shaped_text_inside_longer_fence_stays_literal() {
        // Codex P2 on #1565's first cut: a ``` line inside a ````
        // block is content, not the closing fence.
        let original = "Text.\n\n````\n```\n[^fake]: literal\n````\n";
        let doc = markdown_to_prosemirror(original);
        let found = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "footnote_definition");
        assert!(!found, "definition inside a longer fence stays literal");
    }

    #[test]
    fn test_unreferenced_footnote_with_reference_link_resolves() {
        // Codex P2 on #1565's first cut: the probe needs the
        // document's reference definitions or the footnote's
        // reference-style link degrades to literal text.
        let original =
            "Visible [target][url].\n\n[^orphan]: [target][url]\n\n[url]: https://example.com\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("https://example.com"),
            "reference link resolves in recovered footnote: {first:?}"
        );
        assert_eq!(first, second, "reference-context round-trip is a fixpoint");
    }

    #[test]
    fn test_footnote_definition_inside_list_html_comment_not_synthesized() {
        // Codex P2 on #1565's eighth cut: an HTML block opened
        // inside a list container (`- <!--`) still makes its
        // interior literal, but the scan fed the tracker only the
        // unstripped line and missed the opening behind the list
        // marker, recovering the comment's interior as a live
        // footnote.
        let original = "- <!--\n  [^fake]: literal\n  -->\n\n[^real]: The real orphan.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["real"], "only the real orphan is recovered");
        let (first, second) = two_saves(original);
        assert_eq!(
            first.matches("[^fake]:").count(),
            1,
            "the fake definition appears only inside the preserved comment: {first:?}"
        );
        assert_eq!(first, second, "list-comment round-trip is a fixpoint");
    }

    #[test]
    fn test_reference_definition_after_thematic_break_resolves() {
        // Codex P2 on #1565's eighth cut: a thematic break ends a
        // paragraph, so the reference definition after `---` starts
        // a new block and comrak honors it; the collector treated
        // `---` as paragraph text and omitted the definition from
        // the synthesis context, degrading the footnote's link to
        // literal brackets.
        let original = "---\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("https://example.com"),
            "reference after thematic break resolves in recovered footnote: {first:?}"
        );
        assert_eq!(first, second, "thematic-break round-trip is a fixpoint");
    }

    #[test]
    fn test_footnote_after_blockquote_html_blank_line_recovered() {
        // Codex P2 on #1565's ninth cut: an HTML block opened
        // inside a blockquote ends at the quote's blank line
        // (`>`), but the scan fed the tracker the unstripped `>`
        // line, which is not blank, so the tracker stayed open
        // and the footnote definition after the quote was
        // suppressed. The tracker now sees lines relative to the
        // container it opened in.
        let original = "> <div>\n> html\n>\n[^real]: The note.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            ["real"],
            "definition after blockquote HTML is recovered"
        );
        let (first, second) = two_saves(original);
        assert!(first.contains("The note."), "note survives save: {first:?}");
        assert_eq!(first, second, "blockquote-html round-trip is a fixpoint");
    }

    #[test]
    fn test_footnote_after_blockquote_fence_container_end_recovered() {
        // Codex P2 on #1565's tenth cut: a fence opened inside a
        // blockquote ends implicitly when the quote ends (blank
        // line, no closing delimiter); the scanner kept the fence
        // open and skipped the real definition after it.
        let original = "> ```\n> code\n\n[^real]: The note.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            ["real"],
            "definition after blockquote fence is recovered"
        );
        let (first, second) = two_saves(original);
        assert!(first.contains("The note."), "note survives save: {first:?}");
        assert_eq!(first, second, "blockquote-fence round-trip is a fixpoint");
    }

    #[test]
    fn test_footnote_reference_link_uses_blockquote_definition() {
        // Codex P2 on #1565's tenth cut: a reference definition
        // inside a blockquote is document-global, but the
        // collector tested the unstripped line (still starting
        // with `>`) and omitted it from the synthesis context.
        let original = "> [url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("https://example.com"),
            "blockquote reference definition resolves in recovered footnote: {first:?}"
        );
        assert_eq!(
            first, second,
            "blockquote-definition round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_footnote_lazy_continuation_hashtag_line_preserved() {
        // Codex P2 on #1565's tenth cut: `#hashtag` is paragraph
        // text (no whitespace after the hashes), not an ATX
        // heading, so it lazily continues the definition instead
        // of ending it.
        let original = "[^a]: First line.\n#hashtag\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First line."),
            "first line survives: {first:?}"
        );
        assert!(
            first.contains("#hashtag"),
            "hashtag continuation survives: {first:?}"
        );
        assert_eq!(
            first, second,
            "hashtag-continuation round-trip is a fixpoint"
        );
    }

    #[test]
    fn test_footnote_after_self_closing_pre_recovered() {
        // Codex P2 on #1565's tenth cut: `<pre/>` does not open a
        // marker-ended `<pre>` block (the CommonMark type-1
        // boundary is whitespace, `>`, or end-of-line); it is a
        // generic tag block ending at the blank line, so the
        // footnote after it is a real definition.
        let original = "<pre/>\n\n[^real]: The note.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, ["real"], "definition after <pre/> is recovered");
        let (first, second) = two_saves(original);
        assert!(first.contains("The note."), "note survives save: {first:?}");
        assert_eq!(first, second, "self-closing-pre round-trip is a fixpoint");
    }

    #[test]
    fn test_reference_after_lone_setext_marker_not_a_definition() {
        // Codex P2 on #1565's ninth cut: a lone `=` with no
        // preceding paragraph is paragraph text, so the
        // definition-shaped line after it continues that
        // paragraph and is NOT a reference definition. The
        // collector promoted it anyway, resolving the recovered
        // footnote's reference link against a definition the
        // document does not honor.
        let original = "=\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\\[target\\]\\[url\\]"),
            "unresolved reference stays literal in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[target](https"),
            "unresolved reference is not promoted to a link: {first:?}"
        );
        assert_eq!(first, second, "lone-setext round-trip is a fixpoint");
    }

    #[test]
    fn test_reference_after_real_setext_heading_resolves() {
        // Control for the ninth cut's setext gate: after an actual
        // paragraph, `=` is a setext underline, the heading ends
        // the paragraph, and the definition after it IS honored.
        let original = "Title\n=\n[url]: https://example.com\n\n[^orphan]: [target][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("https://example.com"),
            "reference after setext heading resolves in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("[target][url]"),
            "resolved reference does not stay literal: {first:?}"
        );
        assert_eq!(first, second, "setext-heading round-trip is a fixpoint");
    }

    #[test]
    fn test_backtick_fence_with_backtick_info_opens_no_fence() {
        // Codex P2 on #1565's eighth cut: a backtick fence's info
        // string cannot contain a backtick, so ```a`b opens no
        // fence at all; the tracker opened one anyway and
        // suppressed the valid orphan definition after it.
        let original = "```a`b\n\n[^real]: The note.\n";
        let doc = markdown_to_prosemirror(original);
        let labels: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "footnote_definition")
            .map(|n| n["attrs"]["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            ["real"],
            "definition after invalid fence is recovered"
        );
        let (first, second) = two_saves(original);
        assert!(first.contains("The note."), "note survives save: {first:?}");
        assert_eq!(first, second, "invalid-fence round-trip is a fixpoint");
    }

    #[test]
    fn test_reference_definition_precedence_preserved_in_recovery() {
        // Codex P2 on #1565's eighth cut: reference definitions
        // are global and the FIRST definition wins. The probe
        // appended the document's top-level definitions after the
        // footnote blocks, so a nested definition inside the
        // footnote shadowed the earlier top-level one and the
        // recovered footnote linked to the wrong destination.
        let original = "[url]: https://original.com\n\n[^orphan]: [target][url]\n\n    [url]: https://shadow.com\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("https://original.com"),
            "first definition wins in recovered footnote: {first:?}"
        );
        assert!(
            !first.contains("https://shadow.com"),
            "shadow definition does not win: {first:?}"
        );
        assert_eq!(first, second, "precedence round-trip is a fixpoint");
    }

    #[test]
    fn test_footnote_lazy_continuation_preserved() {
        // Codex P2 on #1565's eighth cut: comrak treats an
        // unindented second line as a lazy continuation of a
        // footnote definition's paragraph; the scan collected only
        // indented continuations and lost the line on save.
        let original = "[^a]: First line.\nsecond line.\n";
        let doc = markdown_to_prosemirror(original);
        let definition = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("unreferenced definition carried into the PM doc");
        assert_eq!(definition["attrs"]["label"], "a");
        let (first, second) = two_saves(original);
        assert!(
            first.contains("First line."),
            "first line survives: {first:?}"
        );
        assert!(
            first.contains("second line."),
            "lazy continuation survives save: {first:?}"
        );
        assert_eq!(first, second, "lazy-continuation round-trip is a fixpoint");
    }

    #[test]
    fn test_unreferenced_footnote_newline_destination_restored() {
        // Codex P2 on #1565's first cut: a newline destination inside
        // an unreferenced definition must not save the placeholder
        // token into the URL.
        let (first, second) = two_saves("[^n]: [x](<a\nb>)\n");
        assert!(
            !first.contains("idweb-newline"),
            "no placeholder token in saved output: {first:?}"
        );
        assert_eq!(
            first, second,
            "newline-destination round-trip is a fixpoint"
        );
    }

    fn test_footnote_multi_paragraph_definition_fixpoint() {
        // #1031: a two-paragraph footnote definition must keep its
        // paragraph boundary — the flat #945 shape fused the two
        // paragraphs into one run of text on save.
        let original = "Text.[^1]\n\n[^1]: First paragraph.\n\n    Second paragraph.\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, second, "multi-paragraph definition is a fixpoint");
        assert!(
            !first.contains("First paragraph.Second paragraph."),
            "paragraphs must not fuse: {first:?}"
        );
        // The saved form still parses to a definition carrying two
        // paragraph children.
        let doc = markdown_to_prosemirror(&first);
        let top = doc["content"].as_array().unwrap();
        let definition = top
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("footnote_definition node present");
        let child_types: Vec<&str> = definition["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["type"].as_str())
            .collect();
        assert_eq!(child_types, ["paragraph", "paragraph"]);
    }

    #[test]
    fn test_footnote_definition_with_code_block_child_fixpoint() {
        // #1031: a code block inside a definition was silently dropped
        // by the flat shape (its literal has no child nodes). As a
        // block child it round-trips.
        let original = "Text.[^1]\n\n[^1]: Note.\n\n    ```\n    let x = 1;\n    ```\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, second, "code-block definition is a fixpoint");
        assert!(first.contains("let x = 1;"), "code survives: {first:?}");
        let doc = markdown_to_prosemirror(&first);
        let top = doc["content"].as_array().unwrap();
        let definition = top
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("footnote_definition node present");
        let child_types: Vec<&str> = definition["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["type"].as_str())
            .collect();
        assert_eq!(child_types, ["paragraph", "code_block"]);
    }

    #[test]
    fn test_footnote_legacy_flat_shape_still_saves() {
        // #1031: documents already stored in the legacy #945 flat
        // shape (definition content = bare inline nodes) still save
        // correctly via the inline-grouping fallback.
        let doc = json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "Text."},
                        {"type": "footnote_reference", "attrs": {"label": "1"}}
                    ]
                },
                {
                    "type": "footnote_definition",
                    "attrs": {"label": "1"},
                    "content": [{"type": "text", "text": "Body."}]
                }
            ]
        });
        let saved = prosemirror_to_markdown(&doc).unwrap();
        // comrak renders a definition's paragraph on an indented
        // continuation line (`[^1]:\n    Body.`) — that is its style
        // for parsed definitions too (verified on the standard
        // parse->save path), so assert content preservation, not the
        // same-line form.
        assert!(
            saved.contains("[^1]:") && saved.contains("Body."),
            "legacy doc saves: {saved:?}"
        );
        // And the saved form round-trips as a fixpoint from here.
        let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
        assert_eq!(saved, resaved);
    }

    #[test]
    fn test_footnote_parse_shape_is_footnote_nodes() {
        // #945: the parse direction represents a footnote as dedicated
        // footnote_reference / footnote_definition nodes carrying the
        // label, not as plain (escapable) text.
        let doc = markdown_to_prosemirror("Text.[^1]\n\n[^1]: Body.\n");
        let top = doc["content"].as_array().unwrap();
        let definition = top
            .iter()
            .find(|n| n["type"] == "footnote_definition")
            .expect("footnote_definition node present");
        assert_eq!(definition["attrs"]["label"], "1");
        // #1031: the definition carries BLOCK children — a single
        // paragraph wrapping the inline body, not bare inline nodes.
        let definition_content = definition["content"].as_array().unwrap();
        assert_eq!(definition_content.len(), 1);
        assert_eq!(definition_content[0]["type"], "paragraph");
        let definition_text: Vec<&str> = definition_content[0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["text"].as_str())
            .collect();
        assert_eq!(definition_text, ["Body."]);
        let paragraph = top
            .iter()
            .find(|n| n["type"] == "paragraph")
            .expect("paragraph present");
        let reference = paragraph["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "footnote_reference")
            .expect("footnote_reference node present");
        assert_eq!(reference["attrs"]["label"], "1");
    }

    #[test]
    fn test_html_block_roundtrip_fixpoint() {
        // #982: raw HTML blocks (comment, details, div) round-trip
        // instead of being rewritten as ```html fences on save, and
        // the second save is a fixpoint. Single-segment blocks are
        // byte-identical; the multi-segment <details> case takes
        // comrak's own block separation (a blank line before the
        // closing tag) — verified as comrak-direct behaviour
        // (parse + format_commonmark of the input, no codec involved,
        // produces the same string), so byte-identity is not
        // achievable for it at this layer.
        for (original, expected) in [
            ("<!-- comment -->\n", "<!-- comment -->\n"),
            (
                "<details>\n<summary>Title</summary>\n\nBody text.\n</details>\n",
                "<details>\n<summary>Title</summary>\n\nBody text.\n\n</details>\n",
            ),
            (
                "<div>\nSome content\n</div>\n",
                "<div>\nSome content\n</div>\n",
            ),
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, expected, "HTML block preserved: {original:?}");
            assert_eq!(first, second, "fixpoint for {original:?}");
            assert!(!first.contains("```"), "no fence rewrite: {first:?}");
        }
    }

    #[test]
    fn test_html_block_parse_shape_has_provenance_attr() {
        // #982: the parse direction marks converted HTML blocks with
        // the htmlBlock provenance attr so save can tell them apart
        // from authored fences.
        let doc = markdown_to_prosemirror("<!-- comment -->\n");
        let block = &doc["content"][0];
        assert_eq!(block["type"], "code_block");
        assert_eq!(block["attrs"]["params"], "html");
        assert_eq!(block["attrs"]["htmlBlock"], true);
    }

    #[test]
    fn test_authored_html_fence_stays_fenced() {
        // #982: a user-authored ```html fence round-trips as a fence —
        // it has no provenance attr, so save keeps the fenced emit.
        let original = "```html\n<div>x</div>\n```\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, original, "authored fence preserved");
        assert_eq!(first, second, "fixpoint");

        // Already-saved docs (params=html, no htmlBlock attr) also
        // stay fenced — no retro-guessing about their origin.
        let pm = json!({
            "type": "doc",
            "content": [{
                "type": "code_block",
                "attrs": {"params": "html"},
                "content": [{"type": "text", "text": "<!-- old -->"}]
            }]
        });
        let md = prosemirror_to_markdown(&pm).expect("converts");
        assert_eq!(md, "```html\n<!-- old -->\n```\n");
    }

    #[test]
    fn test_em_link_roundtrip_fixpoint() {
        for original in [
            "*[text](https://example.com)*\n",
            "[*text*](https://example.com)\n",
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, "[*text*](https://example.com)\n");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_strong_link_with_title_roundtrip_fixpoint() {
        // Href and title survive every round-trip (#944 acceptance).
        let (first, second) = two_saves("[**text**](https://example.com \"Title\")\n");
        assert_eq!(first, "[**text**](https://example.com \"Title\")\n");
        assert_eq!(first, second);
    }

    #[test]
    fn test_strikethrough_link_roundtrip_fixpoint() {
        for original in [
            "~~[text](https://example.com)~~\n",
            "[~~text~~](https://example.com)\n",
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, "[~~text~~](https://example.com)\n");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_bold_italic_roundtrip_fixpoint() {
        // Control that already round-tripped stably before #944: the
        // canonical order (strong outside em) must keep it stable.
        let (first, second) = two_saves("***both***\n");
        assert_eq!(first, second);
        assert!(first.contains("***both***"));
    }

    #[test]
    fn test_code_link_roundtrip_fixpoint() {
        // #953: a code mark combined with a link must keep the link on save.
        let (first, second) = two_saves("[`code`](https://x.test)\n");
        assert_eq!(first, "[`code`](https://x.test)\n");
        assert_eq!(first, second, "no fixpoint");
    }

    #[test]
    fn test_code_inside_link_text_roundtrip() {
        // #953: the code segment stays inside the surrounding link instead
        // of fragmenting it into [a ](href)`code`[ b](href).
        let (first, second) = two_saves("[a `code` b](https://x.test)\n");
        assert_eq!(first, "[a `code` b](https://x.test)\n");
        assert_eq!(first, second, "no fixpoint");
    }

    #[test]
    fn test_code_strong_em_strikethrough_roundtrip_fixpoint() {
        // #953: code combined with strong / em / strikethrough keeps both.
        for (original, expected) in [
            ("**`code`**\n", "**`code`**\n"),
            ("*`code`*\n", "*`code`*\n"),
            ("~~`code`~~\n", "~~`code`~~\n"),
        ] {
            let (first, second) = two_saves(original);
            assert_eq!(first, expected, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_plain_code_roundtrip_unchanged() {
        // #953 control: plain inline code saves exactly as before.
        let (first, second) = two_saves("`code`\n");
        assert_eq!(first, "`code`\n");
        assert_eq!(first, second);
    }

    #[test]
    fn test_roundtrip_code_block() {
        let original = "```rust\nfn main() {}\n```\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        assert!(result.contains("```rust"));
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn test_roundtrip_list() {
        let original = "- Item 1\n- Item 2\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        assert!(result.contains("Item 1"));
        assert!(result.contains("Item 2"));
        // Should have list markers (- or *)
        assert!(result.contains('-') || result.contains('*'));
    }

    #[test]
    fn test_strikethrough() {
        let doc = markdown_to_prosemirror("This is ~~deleted~~ text");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let struck_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "strikethrough"))
        });

        assert!(struck_text.is_some());
        assert_eq!(struck_text.unwrap()["text"], "deleted");
    }

    #[test]
    fn test_strikethrough_with_other_marks() {
        let doc = markdown_to_prosemirror("This is **~~bold deleted~~** text");
        let content = doc["content"].as_array().unwrap();
        let para_content = content[0]["content"].as_array().unwrap();

        let marked_text = para_content
            .iter()
            .find(|n| n["marks"].as_array().is_some_and(|m| m.len() == 2));

        assert!(marked_text.is_some());
        let marks = marked_text.unwrap()["marks"].as_array().unwrap();
        let mark_types: Vec<&str> = marks.iter().map(|m| m["type"].as_str().unwrap()).collect();
        assert!(mark_types.contains(&"strong"));
        assert!(mark_types.contains(&"strikethrough"));
    }

    #[test]
    fn test_roundtrip_strikethrough() {
        let original = "This is ~~deleted~~ text.\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        assert!(result.contains("~~deleted~~"));
        assert!(result.contains("This is"));
        assert!(result.contains("text."));
    }

    #[test]
    fn test_task_list() {
        let doc = markdown_to_prosemirror("- [ ] unchecked\n- [x] checked\n");
        let content = doc["content"].as_array().unwrap();

        // Should produce a task_list, not a bullet_list
        assert_eq!(content[0]["type"], "task_list");
        let items = content[0]["content"].as_array().unwrap();
        assert_eq!(items.len(), 2);

        // First item: unchecked
        assert_eq!(items[0]["type"], "task_list_item");
        assert_eq!(items[0]["attrs"]["checked"], false);

        // Second item: checked
        assert_eq!(items[1]["type"], "task_list_item");
        assert_eq!(items[1]["attrs"]["checked"], true);
    }

    #[test]
    fn test_mixed_task_plain_list_splits_at_parse() {
        // #1370: a list mixing checkbox and plain items must split into
        // pure sibling lists at parse - one mixed list is a shape the
        // placement validation and the editor schema both forbid, which
        // made such documents unsavable.
        let doc = markdown_to_prosemirror("- [ ] a\n- plain\n- [x] b\n");
        let content = doc["content"].as_array().unwrap();
        let types: Vec<&str> = content
            .iter()
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["task_list", "bullet_list", "task_list"]);
        assert_eq!(content[0]["content"].as_array().unwrap().len(), 1);
        assert_eq!(content[1]["content"][0]["type"], "list_item");
        assert_eq!(content[2]["content"][0]["attrs"]["checked"], true);

        // Plain-first order splits the other way.
        let doc = markdown_to_prosemirror("- plain\n- [ ] a\n");
        let content = doc["content"].as_array().unwrap();
        let types: Vec<&str> = content
            .iter()
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["bullet_list", "task_list"]);

        // The split document saves (no MisplacedNode) and the save is a
        // fixpoint: reparsing it splits identically.
        for original in ["- [ ] a\n- plain\n- [x] b\n", "- plain\n- [ ] a\n"] {
            let (first, second) = two_saves(original);
            assert_eq!(first, second, "no fixpoint for {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original)
            );
        }
    }

    #[test]
    fn test_mixed_ordered_list_splits_with_order_offsets() {
        // #1370: ordered split runs carry order offsets so the rendered
        // numbering is unchanged.
        let doc = markdown_to_prosemirror("1. [ ] a\n2. plain\n3. [x] b\n");
        let content = doc["content"].as_array().unwrap();
        let types: Vec<&str> = content
            .iter()
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["task_list", "ordered_list", "task_list"]);
        assert_eq!(content[0]["attrs"]["order"], 1);
        assert_eq!(content[1]["attrs"]["order"], 2);
        assert_eq!(content[2]["attrs"]["order"], 3);

        let doc = markdown_to_prosemirror("4. plain\n5. [ ] a\n");
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "ordered_list");
        assert_eq!(content[0]["attrs"]["order"], 4);
        assert_eq!(content[1]["type"], "task_list");
        assert_eq!(content[1]["attrs"]["order"], 5);

        for original in ["1. [ ] a\n2. plain\n3. [x] b\n", "4. plain\n5. [ ] a\n"] {
            let (first, second) = two_saves(original);
            assert_eq!(first, second, "no fixpoint for {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original)
            );
        }
    }

    #[test]
    fn test_authored_end_list_comment_away_from_lists_survives() {
        // #1370 follow-up: collect_block_children drops the renderer's
        // <!-- end list --> separator only when it sits next to a list.
        // The same comment authored away from any list is content like
        // any other HTML block (#982) and must still convert.
        let doc = markdown_to_prosemirror(
            "<!-- end list -->

Some text
",
        );
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "code_block");
        assert_eq!(content[0]["attrs"]["htmlBlock"], true);
        assert_eq!(content[1]["type"], "paragraph");
    }

    #[test]
    fn test_ordered_task_list_roundtrips() {
        // #997: an ordered task list must keep its ordering and start on
        // save instead of being rewritten as a bullet task list.
        let doc = markdown_to_prosemirror("1. [ ] todo\n2. [x] done\n");
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "task_list");
        assert_eq!(content[0]["attrs"]["order"], 1);

        let doc5 = markdown_to_prosemirror("5. [ ] five\n6. [x] six\n");
        assert_eq!(doc5["content"][0]["attrs"]["order"], 5);

        // Byte-identical round-trips at start 1 and start >1; the second
        // save is a stable fixpoint.
        for original in ["1. [ ] todo\n2. [x] done\n", "5. [ ] five\n6. [x] six\n"] {
            let (first, second) = two_saves(original);
            assert_eq!(first, original, "first save for {original:?}");
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }

        // A bullet task list carries no `order` (#997), but does carry
        // `tight` (#1036): a single-item tight list stores tight: true
        // and no order key.
        let bullet = markdown_to_prosemirror("- [ ] todo\n");
        assert_eq!(bullet["content"][0]["type"], "task_list");
        assert!(bullet["content"][0]["attrs"]["order"].is_null());
        assert_eq!(bullet["content"][0]["attrs"]["tight"], true);
    }

    #[test]
    fn test_task_list_mixed_content() {
        // Task list items with text content
        let doc = markdown_to_prosemirror("- [ ] buy milk\n- [x] write code\n- [ ] review PR\n");
        let content = doc["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "task_list");
        let items = content[0]["content"].as_array().unwrap();
        assert_eq!(items.len(), 3);

        // Verify text content is preserved
        let first_item_text = items[0]["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert_eq!(first_item_text, "buy milk");

        assert_eq!(items[0]["attrs"]["checked"], false);
        assert_eq!(items[1]["attrs"]["checked"], true);
        assert_eq!(items[2]["attrs"]["checked"], false);
    }

    #[test]
    fn test_roundtrip_task_list() {
        let original = "- [ ] unchecked item\n- [x] checked item\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        // Should preserve checkbox syntax
        assert!(
            result.contains("[ ]"),
            "Should have unchecked checkbox: {result}"
        );
        assert!(
            result.contains("[x]"),
            "Should have checked checkbox: {result}"
        );
        assert!(result.contains("unchecked item"));
        assert!(result.contains("checked item"));
    }

    #[test]
    fn test_task_list_tight_attr_carried_on_parse() {
        // #1036: parse must carry list.tight on the task_list node.
        let tight_doc = markdown_to_prosemirror("- [ ] a\n- [x] b\n");
        assert_eq!(tight_doc["content"][0]["attrs"]["tight"], true);
        let loose_doc = markdown_to_prosemirror("- [ ] first\n\n  second para\n- [ ] other\n");
        assert_eq!(loose_doc["content"][0]["attrs"]["tight"], false);
    }

    #[test]
    fn test_loose_task_list_roundtrip_fixpoint() {
        // #1036: a loose (multi-paragraph) task item must keep its
        // paragraph boundary on every save and reach a fixpoint by the
        // second save, checked and not. (Save 1 renders the continuation
        // paragraph with a 2-space indent; the reparse normalizes the
        // task-item content indent and save 2 renders 4 spaces — the
        // boundary and text are preserved throughout, which is the
        // acceptance criterion: pre-fix, save 2 FUSED the paragraphs
        // into "first second para".)
        for original in [
            "- [ ] first\n\n  second para\n- [ ] other\n",
            "- [x] first\n\n  second para\n- [ ] other\n",
        ] {
            let (first, second) = two_saves(original);
            let third = prosemirror_to_markdown(&markdown_to_prosemirror(&second)).unwrap();
            for save in [&first, &second, &third] {
                assert!(
                    !save.contains("first second para"),
                    "paragraphs fused for {original:?}: {save:?}"
                );
                assert!(save.contains("first"), "lost text: {save:?}");
                assert!(save.contains("second para"), "lost text: {save:?}");
            }
            assert_eq!(second, third, "no fixpoint by save 2 for {original:?}");
        }
    }

    #[test]
    fn test_task_list_code_block_item_survives() {
        // #1036: a code block inside a task item survives a save as a
        // code block, not inlined into paragraph text.
        let original = "- [ ] item\n\n  ```\n  code\n  ```\n- [ ] other\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, second, "no fixpoint: {first:?}");
        // The code block must survive as a code block. Its spelling is
        // comrak's choice: an info-less code block inside a loose list
        // item renders as an indented code block, not a fence — what
        // matters is that it is not inlined into the paragraph (checked
        // below) and reparses as a code_block (checked at the end).
        assert!(first.contains("code"), "code content lost: {first:?}");
        assert!(
            first.contains("- [ ] other"),
            "trailing item lost: {first:?}"
        );
        assert!(
            !first.contains("item code"),
            "code block inlined: {first:?}"
        );
        let doc = markdown_to_prosemirror(&first);
        let item = &doc["content"][0]["content"][0];
        assert!(
            item["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["type"] == "code_block"),
            "no code_block child after reparse: {item}"
        );
    }

    #[test]
    fn test_tight_task_list_renders_as_before() {
        // #1036: tight lists render byte-identically to the old codec.
        let original = "- [ ] unchecked item\n- [x] checked item\n";
        let (first, second) = two_saves(original);
        assert_eq!(first, original);
        assert_eq!(first, second);
    }

    #[test]
    fn test_legacy_task_list_without_tight_attr_infers_loose() {
        // #1036: legacy PM docs (no tight attr, stored by the old codec)
        // with a multi-block item must save loose, not fuse.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "task_list",
                "content": [
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
                        {"type": "paragraph", "content": [{"type": "text", "text": "second para"}]}
                    ]},
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "other"}]}
                    ]}
                ]
            }]
        });
        let md = prosemirror_to_markdown(&doc).unwrap();
        assert!(
            !md.contains("first second para"),
            "legacy loose item fused: {md:?}"
        );
        let second = prosemirror_to_markdown(&markdown_to_prosemirror(&md)).unwrap();
        assert_eq!(md, second, "legacy loose output is not a fixpoint");
    }

    #[test]
    fn test_legacy_task_list_without_tight_attr_single_para_is_tight() {
        // #1036: a legacy no-attr list whose items are all single
        // paragraphs renders tight exactly as before.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "task_list",
                "content": [
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "one"}]}
                    ]},
                    {"type": "task_list_item", "attrs": {"checked": true}, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "two"}]}
                    ]}
                ]
            }]
        });
        let md = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(md, "- [ ] one\n- [x] two\n");
    }

    #[test]
    fn test_table() {
        let md =
            "| Header 1 | Header 2 |\n| --- | --- |\n| Cell A | Cell B |\n| Cell C | Cell D |\n";
        let doc = markdown_to_prosemirror(md);
        let content = doc["content"].as_array().unwrap();

        // Should have a table at the top level
        assert_eq!(content[0]["type"], "table");

        let rows = content[0]["content"].as_array().unwrap();
        assert_eq!(rows.len(), 3); // 1 header row + 2 data rows

        // First row should have table_header cells
        assert_eq!(rows[0]["type"], "table_row");
        let header_cells = rows[0]["content"].as_array().unwrap();
        assert_eq!(header_cells.len(), 2);
        assert_eq!(header_cells[0]["type"], "table_header");
        assert_eq!(header_cells[1]["type"], "table_header");

        // Header cells should wrap content in paragraphs
        assert_eq!(header_cells[0]["content"][0]["type"], "paragraph");
        assert_eq!(
            header_cells[0]["content"][0]["content"][0]["text"],
            "Header 1"
        );

        // Data rows should have table_cell
        assert_eq!(rows[1]["type"], "table_row");
        let data_cells = rows[1]["content"].as_array().unwrap();
        assert_eq!(data_cells[0]["type"], "table_cell");
        assert_eq!(data_cells[0]["content"][0]["content"][0]["text"], "Cell A");
    }

    #[test]
    fn test_table_with_formatting() {
        let md = "| Normal | **Bold** | *Italic* |\n| --- | --- | --- |\n| plain | **strong** | *em* |\n";
        let doc = markdown_to_prosemirror(md);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "table");

        let rows = content[0]["content"].as_array().unwrap();
        // Data row - check that bold formatting is preserved
        let data_cells = rows[1]["content"].as_array().unwrap();
        let bold_text = &data_cells[1]["content"][0]["content"][0];
        assert_eq!(bold_text["text"], "strong");
        assert_eq!(bold_text["marks"][0]["type"], "strong");

        let italic_text = &data_cells[2]["content"][0]["content"][0];
        assert_eq!(italic_text["text"], "em");
        assert_eq!(italic_text["marks"][0]["type"], "em");
    }

    #[test]
    fn test_roundtrip_table() {
        let original = "| H1 | H2 |\n| --- | --- |\n| A | B |\n";
        let doc = markdown_to_prosemirror(original);
        let result = prosemirror_to_markdown(&doc).unwrap();

        // Should contain table-like structure with pipes
        assert!(
            result.contains('|'),
            "Should have pipe characters: {result}"
        );
        assert!(result.contains("H1"), "Should have header H1: {result}");
        assert!(result.contains("H2"), "Should have header H2: {result}");
        assert!(result.contains('A'), "Should have cell A: {result}");
        assert!(result.contains('B'), "Should have cell B: {result}");
    }

    #[test]
    fn test_table_code_span_pipe_body_cell() {
        // #1424: a pipe inside a code span in a body cell must not
        // split the cell: one cell, code mark intact, literal pipe.
        let md = "| Command | Note |\n| --- | --- |\n| run `a|b` now | c |\n";
        let doc = markdown_to_prosemirror(md);
        let rows = doc["content"][0]["content"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let cells = rows[1]["content"].as_array().unwrap();
        assert_eq!(cells.len(), 2, "no phantom column: {doc}");
        let inline = cells[0]["content"][0]["content"].as_array().unwrap();
        let code = inline
            .iter()
            .find(|n| n["marks"][0]["type"] == "code")
            .expect("code mark survives");
        assert_eq!(code["text"], "a|b");
        // Save emits the escaped spelling comrak's renderer produces
        // for code in table cells, and the round trip is a fixpoint.
        let saved = prosemirror_to_markdown(&doc).unwrap();
        assert!(saved.contains("`a\\|b`"), "escaped on save: {saved}");
        let back = markdown_to_prosemirror(&saved);
        assert_eq!(back, doc, "reparse of saved form is stable");
        assert_eq!(prosemirror_to_markdown(&back).unwrap(), saved);
    }

    #[test]
    fn test_table_code_span_pipe_header_cell() {
        // #1424: the same protection applies to the header row.
        let md = "| `x|y` | H2 |\n| --- | --- |\n| A | B |\n";
        let doc = markdown_to_prosemirror(md);
        let rows = doc["content"][0]["content"].as_array().unwrap();
        let header = rows[0]["content"].as_array().unwrap();
        assert_eq!(header.len(), 2, "header not split: {doc}");
        let code = &header[0]["content"][0]["content"][0];
        assert_eq!(code["text"], "x|y");
        assert_eq!(code["marks"][0]["type"], "code");
        let saved = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(markdown_to_prosemirror(&saved), doc);
    }

    #[test]
    fn test_table_code_span_pipe_only_content() {
        // #1424: a code span with a pipe as a cell's only content.
        let md = "| H1 | H2 |\n| --- | --- |\n| `a|b` | c |\n";
        let doc = markdown_to_prosemirror(md);
        let rows = doc["content"][0]["content"].as_array().unwrap();
        let cells = rows[1]["content"].as_array().unwrap();
        assert_eq!(cells.len(), 2, "no phantom column: {doc}");
        let inline = cells[0]["content"][0]["content"].as_array().unwrap();
        assert_eq!(inline.len(), 1);
        assert_eq!(inline[0]["text"], "a|b");
        assert_eq!(inline[0]["marks"][0]["type"], "code");
        let saved = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(markdown_to_prosemirror(&saved), doc);
    }

    #[test]
    fn test_table_code_span_pipe_one_column_no_drop() {
        // #1424: under a 1-column header the fragment after the pipe
        // was dropped entirely; the whole code span now survives in
        // the single cell.
        let md = "| `h|x` |\n| --- |\n| `a|b` |\n";
        let doc = markdown_to_prosemirror(md);
        let rows = doc["content"][0]["content"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let header = rows[0]["content"].as_array().unwrap();
        assert_eq!(header.len(), 1);
        assert_eq!(header[0]["content"][0]["content"][0]["text"], "h|x");
        let cells = rows[1]["content"].as_array().unwrap();
        assert_eq!(cells.len(), 1, "single column: {doc}");
        let code = &cells[0]["content"][0]["content"][0];
        assert_eq!(code["text"], "a|b", "no fragment dropped: {doc}");
        assert_eq!(code["marks"][0]["type"], "code");
        let saved = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(markdown_to_prosemirror(&saved), doc);
    }

    #[test]
    fn test_code_span_pipe_outside_table_untouched() {
        // #1424 guard: outside a table a pipe in a code span needs no
        // escape, and the preprocessing must not add one (a `\|`
        // there would be literal backslash text in the span).
        let md = "Run `a|b` here.\n";
        let doc = markdown_to_prosemirror(md);
        let inline = doc["content"][0]["content"].as_array().unwrap();
        let code = inline
            .iter()
            .find(|n| n["marks"][0]["type"] == "code")
            .expect("code mark present");
        assert_eq!(code["text"], "a|b");
    }

    #[test]
    fn test_table_example_inside_fence_untouched() {
        // #2362: a table example inside a fenced block is literal
        // text, not a table — the preprocessor must not escape its
        // code-span pipes (comrak renders the fence verbatim, so a
        // spurious backslash is visible corruption, persisted on
        // save).
        let md = "```markdown\n| `a|b` | H2 |\n| --- | --- |\n| `c|d` | e |\n```\n";
        assert_eq!(
            escape_table_code_span_pipes(md),
            md,
            "fenced table example passes through byte-identically"
        );
        let doc = markdown_to_prosemirror(md);
        let block = &doc["content"][0];
        assert_eq!(block["type"], "code_block", "still a code block: {doc}");
        let text = block["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`a|b`"), "literal pipe in the sample: {text}");
        assert!(!text.contains("\\|"), "no spurious backslash: {text}");
        // Tilde fences get the same protection.
        let tilde = "~~~\n| `a|b` | H2 |\n| --- | --- |\n~~~\n";
        assert_eq!(escape_table_code_span_pipes(tilde), tilde);
        // A live table after the fence is still protected (#1424),
        // and the fence's delimiter row did not leak table state.
        let mixed = format!("{md}\n| `x|y` | H2 |\n| --- | --- |\n");
        let out = escape_table_code_span_pipes(&mixed);
        assert!(out.contains("`x\\|y`"), "live table still escaped: {out}");
        assert_eq!(out.matches("\\|").count(), 1, "only the live table: {out}");
    }

    #[test]
    fn test_pm_to_md_table() {
        // Test the PM JSON → markdown direction specifically.
        // This exercises the NodeTable metadata (num_columns, alignments)
        // that comrak needs to render a valid markdown table.
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "table",
                "content": [
                    {
                        "type": "table_row",
                        "content": [
                            {"type": "table_header", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Name"}]}]},
                            {"type": "table_header", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Age"}]}]}
                        ]
                    },
                    {
                        "type": "table_row",
                        "content": [
                            {"type": "table_cell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Alice"}]}]},
                            {"type": "table_cell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "30"}]}]}
                        ]
                    }
                ]
            }]
        });

        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert!(md.contains("Name"), "Should have Name: {md}");
        assert!(md.contains("Age"), "Should have Age: {md}");
        assert!(md.contains("Alice"), "Should have Alice: {md}");
        assert!(md.contains("30"), "Should have 30: {md}");
        assert!(md.contains("---"), "Should have separator: {md}");

        // Now roundtrip: MD back to PM should produce table nodes
        let doc = markdown_to_prosemirror(&md);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "table");
    }

    #[test]
    fn test_pm_to_md_table_without_header_row_is_promoted() {
        // #1044: a PM table whose rows are all table_cell (schema-legal)
        // must save as a table: the first row is promoted to a header
        // row so comrak emits the GFM delimiter row, instead of the
        // table degrading to a plain paragraph on save.
        let cell = |text: &str| json!({"type": "table_cell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]});
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "table",
                "content": [
                    {"type": "table_row", "content": [cell("a"), cell("b")]},
                    {"type": "table_row", "content": [cell("c"), cell("d")]}
                ]
            }]
        });

        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert_eq!(md, "| a | b |\n| --- | --- |\n| c | d |\n");

        // The saved output reparses as a table whose first row is a
        // header row, and a second save is byte-identical (fixpoint).
        let doc = markdown_to_prosemirror(&md);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "table");
        let rows = content[0]["content"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["content"][0]["type"], "table_header");
        assert_eq!(rows[1]["content"][0]["type"], "table_cell");
        let md2 = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(md2, md);
    }

    #[test]
    fn test_pm_to_md_single_row_table_without_header_is_promoted() {
        // #1044: single-row headerless table — the only row is promoted.
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "table",
                "content": [
                    {"type": "table_row", "content": [
                        {"type": "table_cell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "only"}]}]}
                    ]}
                ]
            }]
        });

        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert_eq!(md, "| only |\n| --- |\n");
        let doc = markdown_to_prosemirror(&md);
        assert_eq!(doc["content"][0]["type"], "table");
        assert_eq!(prosemirror_to_markdown(&doc).unwrap(), md);
    }

    #[test]
    fn test_image_basic() {
        let md = "![photo](http://example.com/img.png)\n";
        let doc = markdown_to_prosemirror(md);
        let content = &doc["content"][0]; // paragraph
        let img = &content["content"][0];
        assert_eq!(img["type"], "image");
        assert_eq!(img["attrs"]["src"], "http://example.com/img.png");
        assert_eq!(img["attrs"]["alt"], "photo");
    }

    #[test]
    fn test_image_with_dimensions_pm_to_md() {
        // PM JSON with width/height should serialize as HTML <img> tag
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {
                        "src": "/blob/abc123",
                        "alt": "my photo",
                        "width": 400,
                        "height": 300
                    }
                }]
            }]
        });

        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert!(md.contains("<img"), "Should contain <img tag: {md}");
        assert!(md.contains("src=\"/blob/abc123\""), "Should have src: {md}");
        assert!(md.contains("alt=\"my photo\""), "Should have alt: {md}");
        assert!(md.contains("width=\"400\""), "Should have width: {md}");
        assert!(md.contains("height=\"300\""), "Should have height: {md}");
    }

    #[test]
    fn test_image_without_dimensions_uses_standard_syntax() {
        // PM JSON without width/height should use standard markdown ![alt](src)
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {
                        "src": "/blob/abc123",
                        "alt": "photo"
                    }
                }]
            }]
        });

        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert!(
            md.contains("![photo](/blob/abc123)"),
            "Should use standard syntax: {md}"
        );
        assert!(!md.contains("<img"), "Should NOT contain <img tag: {md}");
    }

    #[test]
    fn test_image_with_dimensions_roundtrip() {
        // HTML <img> tag with dimensions should roundtrip through PM JSON
        let md = "<img src=\"/blob/abc\" alt=\"pic\" width=\"200\" height=\"150\" />\n";
        let doc = markdown_to_prosemirror(md);

        // Should parse as image node with dimensions
        let content = &doc["content"];
        // The <img> may be wrapped in a paragraph
        let img = if content[0]["type"] == "paragraph" {
            &content[0]["content"][0]
        } else {
            &content[0]
        };
        assert_eq!(img["type"], "image");
        assert_eq!(img["attrs"]["src"], "/blob/abc");
        assert_eq!(img["attrs"]["alt"], "pic");
        assert_eq!(img["attrs"]["width"], 200);
        assert_eq!(img["attrs"]["height"], 150);
    }

    #[test]
    fn test_img_dimension_grammar_kept_forms() {
        // #2180: lossless pixel forms are kept (grammar pinned by
        // triage task 3225): bare, padded, px/PX-suffixed,
        // integral-decimal, plus-form, and zero.
        for (raw, want) in [
            ("200", 200),
            (" 200 ", 200),
            ("200px", 200),
            ("200PX", 200),
            ("200.0", 200),
            ("200.00", 200),
            ("+200", 200),
            ("0", 0),
        ] {
            let node = parse_img_tag(&format!("<img src=\"x.png\" width=\"{raw}\" />"))
                .expect("img parses");
            assert_eq!(node["attrs"]["width"], want, "width {raw:?} kept as {want}");
        }
        let node = parse_img_tag("<img src=\"x.png\" height=\"150px\" />").expect("img parses");
        assert_eq!(node["attrs"]["height"], 150, "height shares the grammar");
    }

    #[test]
    fn test_img_dimension_grammar_refused_forms() {
        // #2180: values with no lossless numeric-pixel form are
        // refused (attr absent; the import path warns) -- never
        // silently truncated or reinterpreted.
        for raw in [
            "50%",
            "200.5",
            "200.5px",
            "10em",
            "2rem",
            "abc",
            "",
            "-5",
            "200pxpx",
            "２００",
            "２００px",
        ] {
            let node = parse_img_tag(&format!("<img src=\"x.png\" width=\"{raw}\" />"))
                .expect("img parses");
            assert!(
                node["attrs"].get("width").is_none(),
                "width {raw:?} refused: {node}"
            );
        }
    }

    #[test]
    fn test_img_decimal_dimensions_are_parsed_exactly() {
        for raw in ["199.99999999999999", "18446744073709551616.0"] {
            let node = parse_img_tag(&format!("<img src=\"x.png\" width=\"{raw}\" />"))
                .expect("img parses");
            assert!(
                node["attrs"].get("width").is_none(),
                "inexact or out-of-range width {raw:?} refused: {node}"
            );
        }

        for (raw, expected) in [("200.00", 200), ("18446744073709551615.0", u64::MAX)] {
            let node = parse_img_tag(&format!("<img src=\"x.png\" width=\"{raw}\" />"))
                .expect("img parses");
            assert_eq!(node["attrs"]["width"], expected, "width {raw:?}");
        }
    }

    #[test]
    fn test_img_px_dimension_roundtrips_as_bare_integer() {
        // #2180 end-to-end: `width="200px"` imports as the number
        // 200 and the emitter writes it back as a bare integer.
        let md = "<img src=\"/blob/abc\" width=\"200px\" />\n";
        let doc = markdown_to_prosemirror(md);
        let content = &doc["content"];
        let img = if content[0]["type"] == "paragraph" {
            &content[0]["content"][0]
        } else {
            &content[0]
        };
        assert_eq!(img["type"], "image");
        assert_eq!(img["attrs"]["width"], 200);
        let out = prosemirror_to_markdown(&doc).unwrap();
        assert!(
            out.contains("width=\"200\""),
            "px form re-emits bare: {out}"
        );
    }

    #[test]
    fn test_img_data_prefixed_attributes_do_not_shadow_real_ones() {
        // #872: data-src / data-width / data-alt must not shadow the real
        // attributes, in either order.
        let node =
            parse_img_tag("<img data-src=\"lazy.png\" src=\"real.png\" />").expect("img parses");
        assert_eq!(node["attrs"]["src"], "real.png");
        let node =
            parse_img_tag("<img src=\"real.png\" data-src=\"lazy.png\" />").expect("img parses");
        assert_eq!(node["attrs"]["src"], "real.png");
        let node = parse_img_tag(
            "<img data-width=\"5\" width=\"100\" data-alt=\"decoy\" alt=\"real alt\" src=\"x.png\" />",
        )
        .expect("img parses");
        assert_eq!(node["attrs"]["width"], 100);
        assert_eq!(node["attrs"]["alt"], "real alt");
    }

    #[test]
    fn test_img_attribute_looking_text_inside_a_value_is_not_an_attribute() {
        // #872: `title=` inside the quoted alt value is value text, not an
        // attribute.
        let node =
            parse_img_tag("<img alt=\"my title=\"quoted\" src=\"x.png\" />").expect("img parses");
        assert_eq!(node["attrs"]["src"], "x.png");
        assert!(node["attrs"].get("title").is_none());
    }

    #[test]
    fn test_img_single_quoted_and_spaced_attributes_parse() {
        // #872 secondary facet: legal spellings the substring search missed.
        let node = parse_img_tag("<img src = 'x.png' alt = 'A' />").expect("img parses");
        assert_eq!(node["attrs"]["src"], "x.png");
        assert_eq!(node["attrs"]["alt"], "A");
    }

    #[test]
    fn test_image_quoted_alt_roundtrips_through_html_emission() {
        // #873 + #872: emission escapes, parsing decodes — the value is
        // byte-identical after a full round trip and cannot inject a title.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {"src": "x.png", "alt": "a \"quoted\" caption", "width": 100}
                }]
            }]
        });
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(md.contains("&quot;"), "alt quote escaped on emission: {md}");
        let back = markdown_to_prosemirror(&md);
        let img = &back["content"][0]["content"][0];
        assert_eq!(img["attrs"]["alt"], "a \"quoted\" caption");
        assert_eq!(img["attrs"]["width"], 100);
        assert!(img["attrs"].get("title").is_none());

        // An injection-shaped alt cannot manufacture a title attribute.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {"src": "x.png", "alt": "x\" title=\"injected", "width": 100}
                }]
            }]
        });
        let md = prosemirror_to_markdown(&doc).expect("converts");
        let back = markdown_to_prosemirror(&md);
        let img = &back["content"][0]["content"][0];
        assert_eq!(img["attrs"]["alt"], "x\" title=\"injected");
        assert!(img["attrs"].get("title").is_none());
    }

    #[test]
    fn test_image_ampersand_roundtrips_without_double_escaping() {
        // Convention pin: `&` emits as `&amp;` and decodes back to `&`;
        // a literal `&quot;` in the value stays literal text.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {"src": "x.png", "alt": "fish & chips &quot;sea&quot;", "width": 10}
                }]
            }]
        });
        let md = prosemirror_to_markdown(&doc).expect("converts");
        let back = markdown_to_prosemirror(&md);
        let img = &back["content"][0]["content"][0];
        assert_eq!(img["attrs"]["alt"], "fish & chips &quot;sea&quot;");
    }

    #[test]
    fn test_linked_image_roundtrip_keeps_href() {
        // #880: [![alt](src)](href) — the href rides on the image node as
        // linkHref and is re-emitted as a Link wrapping the image.
        let doc = markdown_to_prosemirror("[![alt](img.png)](https://example.com)\n");
        let img = &doc["content"][0]["content"][0];
        assert_eq!(img["type"], "image");
        assert_eq!(img["attrs"]["src"], "img.png");
        assert_eq!(img["attrs"]["linkHref"], "https://example.com");
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(
            md.contains("[![alt](img.png)](https://example.com)"),
            "linked image re-emitted with href: {md}"
        );
        // And the emitted form re-parses to the same linked image.
        let back = markdown_to_prosemirror(&md);
        assert_eq!(
            back["content"][0]["content"][0]["attrs"]["linkHref"],
            "https://example.com"
        );
        // Plain images gain no linkHref; plain text links are unchanged.
        let doc = markdown_to_prosemirror("![alt](img.png) and [text](https://x.test)\n");
        let content = doc["content"][0]["content"].as_array().expect("inline");
        assert!(content[0]["attrs"].get("linkHref").is_none());
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(md.contains("![alt](img.png)"), "plain image: {md}");
        assert!(md.contains("[text](https://x.test)"), "plain link: {md}");
    }

    #[test]
    fn test_marked_image_roundtrip_keeps_marks() {
        // #1407: emphasis/strong/strikethrough wrapping an image ride
        // on the image node as a `marks` array and are re-emitted as
        // the wrapping mark on save.
        for (md_src, mark) in [
            ("*![alt](img.png)*\n", "em"),
            ("**![alt](img.png)**\n", "strong"),
            ("~~![alt](img.png)~~\n", "strikethrough"),
        ] {
            let doc = markdown_to_prosemirror(md_src);
            let img = &doc["content"][0]["content"][0];
            assert_eq!(img["type"], "image", "parse: {md_src}");
            assert_eq!(img["marks"][0]["type"], mark, "parse marks: {md_src}");
            let md = prosemirror_to_markdown(&doc).expect("converts");
            assert!(md.contains(md_src.trim()), "roundtrip: {md_src} -> {md}");
            // Fixpoint: the emitted form re-parses to the same marks.
            let back = markdown_to_prosemirror(&md);
            assert_eq!(
                back["content"][0]["content"][0]["marks"][0]["type"], mark,
                "fixpoint: {md_src}"
            );
        }
    }

    #[test]
    fn test_marked_linked_image_roundtrip() {
        // #1407 + #880 combined: *[![alt](src)](href)* keeps both the
        // link (linkHref) and the wrapping emphasis (marks).
        let doc = markdown_to_prosemirror("*[![alt](img.png)](https://example.com)*\n");
        let img = &doc["content"][0]["content"][0];
        assert_eq!(img["attrs"]["linkHref"], "https://example.com");
        assert_eq!(img["marks"][0]["type"], "em");
        let md = prosemirror_to_markdown(&doc).expect("converts");
        // Canonical nesting (#944) puts the link outermost, so the
        // emitted spelling wraps the emphasis inside the link text;
        // it re-parses to the same node (fixpoint asserted below).
        assert!(
            md.contains("[*![alt](img.png)*](https://example.com)"),
            "combined roundtrip: {md}"
        );
        let back = markdown_to_prosemirror(&md);
        let back_img = &back["content"][0]["content"][0];
        assert_eq!(back_img["attrs"]["linkHref"], "https://example.com");
        assert_eq!(back_img["marks"][0]["type"], "em");
    }

    #[test]
    fn test_mixed_marked_span_with_image_roundtrips_as_one_span() {
        // #1407: a marked span mixing text and an image must save as
        // ONE span — no fragmentation, no &#32; boundary entities.
        let doc = markdown_to_prosemirror("*text ![alt](img.png) tail*\n");
        let content = doc["content"][0]["content"].as_array().expect("inline");
        assert_eq!(content.len(), 3, "text + image + text: {doc}");
        assert_eq!(content[1]["marks"][0]["type"], "em");
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(md.contains("*text ![alt](img.png) tail*"), "one span: {md}");
        assert!(!md.contains("&#32;"), "no entity leak: {md}");
        // Fixpoint: the emitted form re-parses to the same shape.
        let back = markdown_to_prosemirror(&md);
        let back_content = back["content"][0]["content"].as_array().expect("inline");
        assert_eq!(back_content.len(), 3, "fixpoint shape: {back}");
        assert_eq!(back_content[1]["marks"][0]["type"], "em");
    }

    #[test]
    fn test_sized_linked_image_roundtrip_keeps_href() {
        // #893: a sized linked image is emitted as
        // <a href="..."><img ... width="..." /></a>, and that form must
        // parse back to one image node — href, dimensions, and alt intact,
        // with no literal <a>/</a> text nodes.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "image",
                    "attrs": {
                        "src": "img.png",
                        "alt": "alt",
                        "width": 200,
                        "height": 150,
                        "linkHref": "https://example.com",
                        "linkTitle": "Link title"
                    }
                }]
            }]
        });
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(
            md.contains("<a href=\"https://example.com\""),
            "sized linked image emitted with anchor: {md}"
        );
        assert!(md.contains("width=\"200\""), "width emitted: {md}");
        assert!(md.contains("height=\"150\""), "height emitted: {md}");

        // Full round trip, twice, so the re-emitted form is pinned too.
        for back in [
            markdown_to_prosemirror(&md),
            markdown_to_prosemirror(
                &prosemirror_to_markdown(&markdown_to_prosemirror(&md)).expect("converts"),
            ),
        ] {
            let para_content = back["content"][0]["content"]
                .as_array()
                .expect("inline content");
            assert_eq!(
                para_content.len(),
                1,
                "only the image node, no literal anchor text: {back}"
            );
            let img = &para_content[0];
            assert_eq!(img["type"], "image");
            assert_eq!(img["attrs"]["src"], "img.png");
            assert_eq!(img["attrs"]["alt"], "alt");
            assert_eq!(img["attrs"]["width"], 200);
            assert_eq!(img["attrs"]["height"], 150);
            assert_eq!(img["attrs"]["linkHref"], "https://example.com");
            assert_eq!(img["attrs"]["linkTitle"], "Link title");
        }

        // The split-inline form inside running text: comrak yields the
        // <a>, <img>, and </a> pieces as sibling HtmlInline nodes of the
        // paragraph, which must merge the same way.
        let doc = markdown_to_prosemirror(
            "before <a href=\"https://example.com\"><img src=\"img.png\" alt=\"alt\" width=\"200\" /></a> after\n",
        );
        let para_content = doc["content"][0]["content"]
            .as_array()
            .expect("inline content");
        let img = para_content
            .iter()
            .find(|n| n["type"] == "image")
            .expect("image node present");
        assert_eq!(img["attrs"]["src"], "img.png");
        assert_eq!(img["attrs"]["alt"], "alt");
        assert_eq!(img["attrs"]["width"], 200);
        assert_eq!(img["attrs"]["linkHref"], "https://example.com");
        assert!(
            para_content.iter().all(|n| {
                n["text"]
                    .as_str()
                    .is_none_or(|t| !t.contains("<a") && !t.contains("</a>"))
            }),
            "no literal anchor text nodes: {doc}"
        );
    }

    /// Build a one-column table doc whose single data cell holds the
    /// given PM content (#1017 regression tests).
    fn table_doc_with_cell(cell_content: Value) -> Value {
        json!({
            "type": "doc",
            "content": [{
                "type": "table",
                "content": [
                    {"type": "table_row", "content": [
                        {"type": "table_header", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "h"}]}]}
                    ]},
                    {"type": "table_row", "content": [
                        {"type": "table_cell", "content": cell_content}
                    ]}
                ]
            }]
        })
    }

    #[test]
    fn test_unmarked_leading_spaces_trimmed_at_save() {
        // #1420: "  leading spaces" was written verbatim on save but
        // stripped on reopen. Normalize-at-save makes the first save
        // the fixpoint.
        let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "  leading spaces"}]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "leading spaces\n");
        let reopened = markdown_to_prosemirror(&md);
        assert_eq!(
            reopened,
            json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "leading spaces"}]}]})
        );
        assert_eq!(prosemirror_to_markdown(&reopened).expect("reconverts"), md);
    }

    #[test]
    fn test_unmarked_trailing_spaces_trimmed_at_save() {
        // #1420: trailing spaces of a block-final run.
        let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "foo   "}]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "foo\n");
        let reopened = markdown_to_prosemirror(&md);
        assert_eq!(prosemirror_to_markdown(&reopened).expect("reconverts"), md);
    }

    #[test]
    fn test_unmarked_spaces_around_hard_break_trimmed_at_save() {
        // #1420: a hard_break starts a new rendered line, so leading
        // spaces after it (and trailing spaces before it) are line-edge
        // whitespace and normalize away.
        let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "foo  "},
            {"type": "hard_break"},
            {"type": "text", "text": "  bar"}
        ]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "foo\\\nbar\n");
        let reopened = markdown_to_prosemirror(&md);
        assert_eq!(prosemirror_to_markdown(&reopened).expect("reconverts"), md);
    }

    #[test]
    fn test_unmarked_table_cell_padding_trimmed_at_save() {
        // #1420: cell content is single-line; padding around a cell's
        // text is line-edge whitespace.
        let doc = table_doc_with_cell(json!([{
            "type": "paragraph",
            "content": [{"type": "text", "text": " x "}]
        }]));
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(md.contains("| x |"), "cell text trimmed: {md:?}");
        let reopened = markdown_to_prosemirror(&md);
        assert_eq!(prosemirror_to_markdown(&reopened).expect("reconverts"), md);
    }

    #[test]
    fn test_whitespace_only_paragraph_saves_as_empty_paragraph() {
        // #1420: a whitespace-only paragraph used to be written as
        // "   " and deleted outright on reopen. It now saves as an
        // empty paragraph: alone it reopens as exactly one empty
        // paragraph, and the saved file is a fixpoint from the first
        // save even alongside other paragraphs.
        let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "   "}]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "");
        assert_eq!(
            markdown_to_prosemirror(&md),
            json!({"type": "doc", "content": [{"type": "paragraph"}]})
        );

        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "   "}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
        ]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "x\n", "no whitespace text is written: {md:?}");
        let reopened = markdown_to_prosemirror(&md);
        assert_eq!(prosemirror_to_markdown(&reopened).expect("reconverts"), md);
    }

    #[test]
    fn test_unmarked_interior_spaces_next_to_marks_preserved() {
        // #1420 guard: interior unmarked runs are word separators and
        // DO survive reparse — trimming them would fuse words into the
        // neighbouring mark. Marked runs keep their #1418 entity
        // protection untouched.
        let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "a "},
            {"type": "text", "text": " b ", "marks": [{"type": "em"}]},
            {"type": "text", "text": " c"}
        ]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "a *&#32;b&#32;* c\n");
        assert_eq!(markdown_to_prosemirror(&md), doc);
    }

    #[test]
    fn test_table_cell_hard_break_does_not_panic() {
        // #1017: Shift+Enter in a cell produces a hard_break inside the
        // cell's paragraph; LineBreak is not a valid TableCell child in
        // comrak's AST, so this used to panic in format_commonmark. The
        // break degrades to a space.
        let doc = table_doc_with_cell(json!([{
            "type": "paragraph",
            "content": [
                {"type": "text", "text": "line one"},
                {"type": "hard_break"},
                {"type": "text", "text": "line two"}
            ]
        }]));
        let md = prosemirror_to_markdown(&doc).expect("hard_break cell converts");
        assert!(
            md.contains("line one line two"),
            "break became a space: {md}"
        );
        let second =
            prosemirror_to_markdown(&markdown_to_prosemirror(&md)).expect("second save converts");
        assert_eq!(md, second, "hard_break cell output is a fixpoint");
    }

    #[test]
    fn test_table_cell_list_does_not_panic() {
        // #1017: a bullet_list inside a cell is a block node — invalid
        // under TableCell — and panicked the save. It flattens to text.
        let doc = table_doc_with_cell(json!([{
            "type": "bullet_list",
            "attrs": {"tight": true},
            "content": [
                {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "item one"}]}]},
                {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "item two"}]}]}
            ]
        }]));
        let md = prosemirror_to_markdown(&doc).expect("list cell converts");
        assert!(md.contains("item one item two"), "list flattened: {md}");
        let second =
            prosemirror_to_markdown(&markdown_to_prosemirror(&md)).expect("second save converts");
        assert_eq!(md, second, "list cell output is a fixpoint");
    }

    #[test]
    fn test_table_cell_code_block_does_not_panic() {
        // #1017: a code_block inside a cell likewise flattens to text.
        let doc = table_doc_with_cell(json!([{
            "type": "code_block",
            "content": [{"type": "text", "text": "let x = 1;"}]
        }]));
        let md = prosemirror_to_markdown(&doc).expect("code_block cell converts");
        assert!(md.contains("let x = 1;"), "code flattened: {md}");
        let second =
            prosemirror_to_markdown(&markdown_to_prosemirror(&md)).expect("second save converts");
        assert_eq!(md, second, "code_block cell output is a fixpoint");
    }

    #[test]
    fn test_table_alignment_roundtrip() {
        // #881: alignments ride on the table node and re-emit in the
        // delimiter row.
        let md_in = "| a | b | c | d |\n|:---|:---:|---:|---|\n| 1 | 2 | 3 | 4 |\n";
        let doc = markdown_to_prosemirror(md_in);
        let table = &doc["content"][0];
        assert_eq!(table["type"], "table");
        assert_eq!(
            table["attrs"]["alignments"],
            json!(["left", "center", "right", "none"])
        );
        let md = prosemirror_to_markdown(&doc).expect("converts");
        let back = markdown_to_prosemirror(&md);
        assert_eq!(
            back["content"][0]["attrs"]["alignments"],
            json!(["left", "center", "right", "none"]),
            "alignments survive round trip via: {md}"
        );
        // Unaligned tables carry no alignments attr and render as before.
        let doc = markdown_to_prosemirror("| a |\n|---|\n| 1 |\n");
        assert!(doc["content"][0]["attrs"].get("alignments").is_none());
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(md.contains("| a |"), "plain table: {md}");
    }

    #[test]
    fn test_code_block_multiple_text_nodes_all_kept() {
        // #882: a code_block split into several text nodes must not lose
        // every node after the first on save.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "code_block",
                "attrs": {"params": "rust"},
                "content": [
                    {"type": "text", "text": "fn a() {}\n"},
                    {"type": "text", "text": "fn b() {}"}
                ]
            }]
        });
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert!(md.contains("fn a() {}"), "first node kept: {md}");
        assert!(md.contains("fn b() {}"), "second node kept: {md}");
    }

    #[test]
    fn test_inline_html_sub_sup_u_roundtrip_unescaped() {
        // #943: inline HTML must round-trip open->save as HTML, not as
        // backslash-escaped literal text.
        for original in [
            "H<sub>2</sub>O\n",
            "x<sup>2</sup> + y\n",
            "some <u>underlined</u> text\n",
        ] {
            let (first, second) = two_saves(original);
            assert!(
                !first.contains("\\<"),
                "no backslash-escaping in save of {original:?}: {first}"
            );
            for tag in ["<sub>", "</sub>", "<sup>", "</sup>", "<u>", "</u>"] {
                if original.contains(tag) {
                    assert!(
                        first.contains(tag),
                        "{tag} survives save of {original:?}: {first}"
                    );
                }
            }
            assert_eq!(first, second, "fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_inline_html_parse_shape_is_html_inline_node() {
        // #943: the parse direction represents a tag as one html_inline
        // atom node carrying the raw tag, not as a text node.
        let doc = markdown_to_prosemirror("H<sub>2</sub>O\n");
        let content = &doc["content"][0]["content"];
        let html_nodes: Vec<&Value> = content
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["type"] == "html_inline")
            .collect();
        assert_eq!(html_nodes.len(), 2, "open+close tags: {content}");
        assert_eq!(html_nodes[0]["attrs"]["html"], "<sub>");
        assert_eq!(html_nodes[1]["attrs"]["html"], "</sub>");
    }

    #[test]
    fn test_inline_html_generality_nonformatting_tag_roundtrips() {
        // #943 generality: passthrough is not a sub/sup/u whitelist — any
        // inline tag (here <kbd> and a classed <span>) round-trips.
        let (first, second) = two_saves("press <kbd>Ctrl</kbd> then <span class=\"x\">go</span>\n");
        assert!(first.contains("<kbd>"), "kbd tag survives: {first}");
        assert!(first.contains("</kbd>"), "kbd close survives: {first}");
        assert!(
            first.contains("<span class=\"x\">"),
            "span with attrs survives verbatim: {first}"
        );
        assert!(first.contains("</span>"), "span close survives: {first}");
        assert!(!first.contains("\\<"), "no escaping: {first}");
        assert_eq!(first, second, "fixpoint");
    }
    #[test]
    fn test_marks_spanning_inline_html_roundtrip() {
        // #1412: a mark spanning an html_inline atom must save as ONE
        // span containing the tags, not fragmented per text run, and
        // emphasis forms must not leak &#32; entities at the seams.
        for md in [
            "[a <b>x</b> c](/y)\n",
            "[a <sup>2</sup> c](/y)\n",
            "*a <b>x</b> c*\n",
            "**a <b>x</b> c**\n",
            "~~a <b>x</b> c~~\n",
            "*a <b><i>x</i></b> c*\n",
        ] {
            let (first, second) = two_saves(md);
            assert_eq!(first, md, "round-trip for {md:?}");
            assert_eq!(first, second, "fixpoint for {md:?}");
            assert!(
                !first.contains("&#32;"),
                "no entity leak for {md:?}: {first}"
            );
        }
    }

    #[test]
    fn test_marks_spanning_inline_html_pm_to_md() {
        // #1412, save direction alone: PM text runs carrying a link
        // mark, separated by markless html_inline atoms, emit a single
        // link span containing the tags.
        let pm_json = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "a ", "marks": [{"type": "link", "attrs": {"href": "/y"}}]},
                    {"type": "html_inline", "attrs": {"html": "<b>"}},
                    {"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "/y"}}]},
                    {"type": "html_inline", "attrs": {"html": "</b>"}},
                    {"type": "text", "text": " c", "marks": [{"type": "link", "attrs": {"href": "/y"}}]}
                ]
            }]
        });
        let md = prosemirror_to_markdown(&pm_json).unwrap();
        assert_eq!(md.trim(), "[a <b>x</b> c](/y)", "single link span: {md}");
    }

    #[test]
    fn test_inline_html_outside_marks_stays_outside() {
        // #1412 controls: atoms that do NOT sit between same-mark runs
        // keep their legacy placement outside any mark span.
        for md in ["a <b>x</b> c\n", "*a* <b>x</b>\n"] {
            let (first, second) = two_saves(md);
            assert_eq!(first, md, "round-trip for {md:?}");
            assert_eq!(first, second, "fixpoint for {md:?}");
        }
        // Single-run form ([<b>x</b>](/y)): the save form may differ
        // textually but must re-parse to the identical PM structure.
        let (first, second) = two_saves("[<b>x</b>](/y)\n");
        assert_eq!(first, second, "fixpoint for single-run form: {first}");
        assert_eq!(
            markdown_to_prosemirror(&first),
            markdown_to_prosemirror("[<b>x</b>](/y)\n"),
            "same PM structure for single-run form"
        );
    }
}
