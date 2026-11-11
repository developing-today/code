//! Source scans and recovery for footnote and link-reference definitions.

use serde_json::Value;

use super::parse_helpers::restore_attr_newlines;

use super::markdown_to_prosemirror_inner;

/// Fence tracking for the source scans below: a fence opens with a
/// run of >= 3 backticks or tildes and closes only with a run of the
/// SAME character at least as long (CommonMark) -- a three-backtick
/// line inside a four-backtick block is content, not a close (Codex
/// P2 on the first #1565 cut, which toggled on any fence-shaped line).
#[derive(Default)]
pub(super) struct FenceTracker {
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
pub(super) fn fence_feed_line(line: &str) -> &str {
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
pub(super) struct HtmlBlockTracker {
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
pub(super) fn is_blank_line(line: &str) -> bool {
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

pub(super) fn starts_new_block(stripped: &str) -> bool {
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
        let single_start = if is_link_reference_definition_start(def_stripped) {
            link_reference_definition_label_close(def_stripped)
                .map(|close| (vec![line], def_stripped[close + 2..].to_string(), i + 1))
        } else {
            None
        };
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
            let Some((mut block, mut after, mut j)) = single_start.or(multiline_start) else {
                continue;
            };
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

pub(super) fn preserve_unreferenced_footnote_definitions(
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
