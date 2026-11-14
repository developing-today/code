//! Source-scanning helpers for the Markdown -> ProseMirror conversion:
//! fence/HTML-block tracking, container prefixes, and footnote/link
//! reference source scans. Split out of `markdown.rs` for #4704.

use super::*;

/// Fence tracking for the source scans below: a fence opens with a
/// run of >= 3 backticks or tildes and closes only with a run of the
/// SAME character at least as long (CommonMark) -- a three-backtick
/// line inside a four-backtick block is content, not a close (Codex
/// P2 on the first #1565 cut, which toggled on any fence-shaped line).
#[derive(Default)]
pub(crate) struct FenceTracker {
    pub(crate) open: Option<(char, usize)>,
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
pub(crate) fn fence_feed_line(line: &str) -> &str {
    let spaces = line.chars().take_while(|&c| c == ' ').count().min(3);
    &line[spaces..]
}

impl FenceTracker {
    /// Feed one line (already stripped of up to three leading
    /// spaces). Returns true when the line is a fence delimiter.
    pub(crate) fn feed(&mut self, stripped: &str) -> bool {
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

    pub(crate) fn in_fence(&self) -> bool {
        self.open.is_some()
    }

    /// Forget any open fence: a fence opened inside a container
    /// ends implicitly when the container ends, with no closing
    /// delimiter line (Codex P2 on #1565's tenth cut).
    pub(crate) fn reset(&mut self) {
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
pub(crate) struct HtmlBlockTracker {
    /// What ends the open block: a literal closing marker, or a
    /// blank line for block-tag (type 6/7) blocks.
    pub(crate) end: Option<HtmlBlockEnd>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum HtmlBlockEnd {
    Marker(&'static str),
    BlankLine,
}

/// The CommonMark type-6 block-level tag names.
pub(crate) const HTML_BLOCK_TAGS: &[&str] = &[
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
pub(crate) fn is_complete_html_tag(trimmed: &str) -> bool {
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
    pub(crate) fn in_html(&self) -> bool {
        self.end.is_some()
    }

    /// Forget any open block: a block opened inside a container
    /// ends implicitly when the container ends, with no closing
    /// line (Codex P2 on #1565's eleventh cut, mirroring
    /// FenceTracker::reset).
    pub(crate) fn reset(&mut self) {
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
    pub(crate) fn feed(&mut self, stripped: &str, may_open_type7: bool) -> bool {
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
pub(crate) fn strip_container_prefixes(line: &str) -> &str {
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
pub(crate) fn strip_marker_padding(after: &str, marker_end_col: usize) -> &str {
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
pub(crate) fn container_signature(prefix: &str) -> String {
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
pub(crate) fn strip_container_levels(line: &str, levels: usize) -> &str {
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
pub(crate) fn carries_container_prefix(stripped: &str, prefix: &str) -> bool {
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
pub(crate) struct FootnoteDefinitionBlock {
    pub(crate) label: String,
    pub(crate) source: String,
    /// Zero-based line index of the definition's opening line, for
    /// interleaving with other source segments in document order.
    pub(crate) line: usize,
}

/// The label of a footnote definition starting on `line` (already
/// stripped of up to three leading spaces), or `None` when the line
/// does not start one. A definition opens as `[^label]:`.
pub(crate) fn footnote_definition_label(line: &str) -> Option<String> {
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
pub(crate) fn is_thematic_break(stripped: &str) -> bool {
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
pub(crate) fn is_setext_underline(stripped: &str) -> bool {
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
pub(crate) fn is_atx_heading(stripped: &str) -> bool {
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
pub(crate) fn tag_name_boundary_ok(tag_body: &str, name_len: usize) -> bool {
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
pub(crate) fn is_html_block_start(stripped: &str) -> bool {
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
pub(crate) fn is_non_interrupting_ordered_marker(stripped: &str) -> bool {
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
pub(crate) fn is_fence_opener(stripped: &str) -> bool {
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
pub(crate) fn fence_opener_run(stripped: &str) -> Option<(char, usize)> {
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
pub(crate) fn is_fence_closer(stripped: &str, ch: char, run: usize) -> bool {
    let count = stripped.chars().take_while(|&c| c == ch).count();
    count >= run && stripped[count..].trim().is_empty()
}

/// The indentation of a line in columns: a space is one column
/// and a tab advances to the next multiple of four, so the two
/// spellings ` \t` and `    ` are the same four columns
/// (CommonMark) -- a literal prefix check accepts neither the
/// mixed spelling nor other space/tab combinations (Codex P2 on
/// #1565's fifteenth cut).
pub(crate) fn indentation_columns(line: &str) -> usize {
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
pub(crate) fn is_blank_line(line: &str) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

/// Remove exactly `columns` leading indentation columns, accounting for
/// tabs advancing to the next multiple of four. Used to inspect content
/// inside a footnote continuation without erasing additional indentation
/// that may make the line an indented code block.
pub(crate) fn strip_indentation_columns(line: &str, columns: usize) -> &str {
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
pub(crate) fn starts_with_list_marker(stripped: &str) -> bool {
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

pub(crate) fn starts_new_block(stripped: &str) -> bool {
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
pub(crate) fn scan_footnote_definition_blocks(markdown: &str) -> Vec<FootnoteDefinitionBlock> {
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
