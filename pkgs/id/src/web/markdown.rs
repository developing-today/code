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
    if let Some(token) = newline_token {
        restore_attr_newlines(&mut doc, &token);
    }
    doc
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
fn escape_table_code_span_pipes(markdown: &str) -> String {
    if !markdown.contains('`') || !markdown.contains('|') {
        return markdown.to_string();
    }
    let lines: Vec<&str> = markdown.split_inclusive('\n').collect();
    let mut out = String::with_capacity(markdown.len() + 16);
    let mut in_table = false;
    for (idx, raw) in lines.iter().enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let ending = &raw[line.len()..];
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
    if let Some(w) = attr("width")
        && let Ok(n) = w.parse::<u64>()
    {
        attrs["width"] = json!(n);
    }
    if let Some(h) = attr("height")
        && let Ok(n) = h.parse::<u64>()
    {
        attrs["height"] = json!(n);
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
    Ok(strip_end_list_separators(doc, &output))
}

/// The start number a PM list renders with in Markdown, when it
/// renders as an ordered list: ordered_list always (attrs.order,
/// default 1); a task_list only when it carries attrs.order (#1370
/// ordered task runs render as `1. [ ]`). Bullet-rendering lists and
/// non-lists return None.
fn ordered_start(node: &Value) -> Option<u64> {
    let order = node["attrs"]["order"].as_u64();
    match node["type"].as_str() {
        Some("ordered_list") => Some(order.unwrap_or(1)),
        Some("task_list") => order,
        _ => None,
    }
}

fn list_kind(node: &Value) -> Option<&str> {
    match node["type"].as_str() {
        Some(k @ ("bullet_list" | "ordered_list" | "task_list")) => Some(k),
        _ => None,
    }
}

/// The tightness a PM list effectively renders with: the stored
/// `attrs.tight` when present; for a legacy task_list without the
/// attribute, the same inference json_to_ast applies (loose if any
/// item has more than one block child, else tight); bullet/ordered
/// lists without the attribute render loose (json_to_ast defaults
/// them to false). Boundary decisions in collect_boundary_kinds must
/// use this, not the raw attribute: comparing raw attrs mistook a
/// legacy tight task list for loose (CodeRabbit review, PR #3258).
fn effective_tight(node: &Value) -> bool {
    if let Some(tight) = node["attrs"]["tight"].as_bool() {
        return tight;
    }
    if node["type"].as_str() == Some("task_list") {
        return !node["content"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.len() > 1)
            })
        });
    }
    false
}

/// #1397: comrak's renderer separates adjacent list siblings with an
/// `<!-- end list -->` comment - renderer-internal markup that lands in
/// the user's file. Where the two lists are of DIFFERENT PM kinds the
/// comment is the only thing keeping them apart, and it is not needed:
/// a change of list type ends a list on reparse all by itself (and the
/// #1370 parse-side split rebuilds task/plain runs), so the separator
/// line is dropped and the lists sit directly adjacent in the text -
/// tight lists stay tight, loose lists keep their blank lines. Where
/// the kinds are the SAME (two ordered lists, two task lists) dropping
/// it would merge the lists and lose restart/order information, so the
/// separator stays for those pairs. It also stays for a DIFFERENT-kind
/// pair that renders as the same ordered Markdown list (an ordered_list
/// next to a task_list carrying `attrs.order`, or vice versa) when the
/// second list does not start exactly where the first list's numbering
/// ends: merging those on reparse renumbers the second run and loses
/// its restart (Codex review, PR #3258). If the lines found in the output
/// do not line up one-to-one with the boundaries computed from the doc,
/// the output is returned untouched.
fn strip_end_list_separators(doc: &Value, output: &str) -> String {
    // Recompute with kinds: the drop decision needs them, so collect
    // (prev_kind, kind, tight_pair, merge_safe, needs_blank) tuples
    // alongside.
    let mut kinds: Vec<(String, String, bool, bool, bool)> = Vec::new();
    collect_boundary_kinds(doc, &mut kinds);
    if kinds.is_empty() {
        return output.to_string();
    }
    let is_separator_line = |line: &str| {
        line.trim_start_matches(['>', ' ']).trim() == "<!-- end list -->"
    };
    let is_blank_line = |line: &str| line.trim_start_matches(['>', ' ']).trim().is_empty();
    let lines: Vec<&str> = output.lines().collect();
    // Codex review, PR #3258: a separator-looking line inside a fenced
    // code block is authored content, not renderer markup - it must
    // neither count toward the boundary correspondence nor be dropped.
    // Track fence state (``` / ~~~, optionally blockquote-prefixed) and
    // only treat lines outside fences as separators.
    let mut in_fence = false;
    let mut fence_char = '`';
    let mut fence_len = 0usize;
    let mut is_separator: Vec<bool> = Vec::with_capacity(lines.len());
    for line in &lines {
        let trimmed = line.trim_start_matches(['>', ' ']).trim_start();
        let fence_marker = if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        match fence_marker {
            Some(c) if !in_fence => {
                in_fence = true;
                fence_char = c;
                fence_len = trimmed.chars().take_while(|&ch| ch == c).count();
                is_separator.push(false);
            }
            // CodeRabbit review, PR #3258: a fence closes only on a
            // BARE run of the fence character at least as long as the
            // opener. A content line such as ```rust starts with the
            // marker but is not a closer; ending the fence there let a
            // separator-looking line later in the code block be counted
            // (and dropped) as renderer markup, losing authored lines.
            Some(c)
                if c == fence_char
                    && trimmed.trim_end().chars().all(|ch| ch == c)
                    && trimmed.trim_end().len() >= fence_len =>
            {
                in_fence = false;
                is_separator.push(false);
            }
            _ => is_separator.push(!in_fence && is_separator_line(line)),
        }
    }
    let found = is_separator.iter().filter(|b| **b).count();
    if found != kinds.len() {
        return output.to_string();
    }
    let mut boundary = kinds.iter();
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    let mut idx = 0;
    while idx < lines.len() {
        let line = lines[idx];
        if is_separator[idx] {
            match boundary.next() {
                Some((a, b, tight_pair, merge_safe, needs_blank)) if a != b && *merge_safe => {
                    // Drop the separator. The renderer also puts a blank
                    // line on each side of it; with the comment gone,
                    // blanks between the lists would fuse them into ONE
                    // loose list on reparse. A tight pair keeps no blank
                    // line at all; a loose pair keeps exactly one. A
                    // needs_blank pair also keeps exactly one: an ordered
                    // list starting above 1 cannot interrupt the previous
                    // item's paragraph, so with no blank line at all its
                    // first line reparses as a lazy continuation and the
                    // list is lost (Codex review, PR #3258).
                    let prev_blank = kept.last().is_some_and(|l| is_blank_line(l));
                    let next_blank = lines.get(idx + 1).is_some_and(|l| is_blank_line(l));
                    if prev_blank && next_blank {
                        if *tight_pair && !*needs_blank {
                            kept.pop();
                        }
                        idx += 1; // skip the following blank line too
                    }
                }
                _ => kept.push(line),
            }
        } else {
            kept.push(line);
        }
        idx += 1;
    }
    let mut out = kept.join("\n");
    if output.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// (prev PM kind, PM kind, both tight, merge-safe, needs-blank) for every list
/// adjacency in the doc, in render order, interleaved with authored
/// htmlBlocks (recorded as ("", "", false, false, false) so they consume a
/// separator-line slot but are never dropped - see
/// strip_end_list_separators' caller contract). Merge-safe is false
/// only when dropping the separator would merge two ordered-rendering
/// lists whose numbering does not continue across the boundary.
fn collect_boundary_kinds(node: &Value, out: &mut Vec<(String, String, bool, bool, bool)>) {
    let Some(content) = node["content"].as_array() else {
        return;
    };
    let mut prev_list: Option<&Value> = None;
    // Effective tight of the current consecutive bullet_list run's
    // FIRST list (the tight json_to_ast's coalesce renders with).
    let mut bullet_run_tight: Option<bool> = None;
    for child in content {
        let kind = list_kind(child);
        if let (Some(prev), Some(kind)) = (prev_list, kind)
            && !(list_kind(prev) == Some("bullet_list") && kind == "bullet_list")
        {
            // Codex review, PR #3258: when prev is a bullet_list that
            // json_to_ast coalesced with earlier bullet siblings, the
            // rendered list carries the FIRST sibling's tight, so the
            // boundary must compare that effective tightness - the last
            // original sibling's attr can differ and silently loosens
            // the following run on reparse.
            let prev_tight = if list_kind(prev) == Some("bullet_list") {
                bullet_run_tight.unwrap_or_else(|| effective_tight(prev))
            } else {
                effective_tight(prev)
            };
            let child_tight = effective_tight(child);
            let tight_pair = prev_tight && child_tight;
            // Two lists that both render as ordered Markdown merge into
            // one list on reparse if the separator goes; the merge is
            // lossless only when the second list starts exactly where
            // the first list's numbering ends (start + item count), so
            // convert_list_runs recomputes the same run orders.
            let ordered_merge_safe = match (ordered_start(prev), ordered_start(child)) {
                (Some(prev_start), Some(next_start)) => {
                    let prev_count = prev["content"].as_array().map_or(0, |c| c.len()) as u64;
                    // attrs.order comes from the request's PM JSON; a
                    // crafted start near u64::MAX must not overflow the
                    // sum (debug builds panic, release builds wrap and
                    // could drop a separator that should stay).
                    prev_start.checked_add(prev_count) == Some(next_start)
                }
                _ => true,
            };
            // Codex review, PR #3258: two lists that render as the same
            // Markdown list family (bullet-rendering: bullet_list and
            // order-less task_list; ordered-rendering: ordered_list and
            // ordered task_list) merge into ONE list on reparse when the
            // separator drops, and convert_list_runs then copies one
            // run's tight onto both. When their tight attributes differ
            // that silently loosens/tightens a run and breaks the save
            // fixpoint, so the separator stays. Different families split
            // on reparse by marker type and keep their own tightness.
            let renders_ordered = |n: &Value| ordered_start(n).is_some();
            let tight_differs = prev_tight != child_tight;
            let tight_merge_safe =
                !(renders_ordered(prev) == renders_ordered(child) && tight_differs);
            let merge_safe = ordered_merge_safe && tight_merge_safe;
            // Codex review, PR #3258: an ordered-rendering list that
            // starts above 1 cannot interrupt a paragraph, so when the
            // previous list does not itself render ordered, the boundary
            // must keep a blank line or the second list's first line
            // reparses as a lazy continuation of the previous item.
            let needs_blank = matches!(ordered_start(child), Some(start) if start != 1)
                && ordered_start(prev).is_none();
            out.push((
                list_kind(prev).unwrap_or_default().to_string(),
                kind.to_string(),
                tight_pair,
                merge_safe,
                needs_blank,
            ));
        }
        if child["type"].as_str() == Some("code_block")
            && child["attrs"]["htmlBlock"].as_bool() == Some(true)
            && child["content"].as_array().is_some_and(|parts| {
                parts
                    .iter()
                    .filter_map(|n| n["text"].as_str())
                    .collect::<String>()
                    .trim()
                    == "<!-- end list -->"
            })
        {
            out.push((String::new(), String::new(), false, false, false));
        }
        // Codex review, PR #3258: lists inside a table cell are
        // flattened to text by append_table_cell_child and emit no
        // separator, so boundaries collected there have no line in
        // the output to correspond to - skip the cell's subtree.
        if !matches!(child["type"].as_str(), Some("table_header" | "table_cell")) {
            collect_boundary_kinds(child, out);
        }
        if kind.is_some() {
            if kind == Some("bullet_list") {
                if prev_list.is_none_or(|p| list_kind(p) != Some("bullet_list")) {
                    bullet_run_tight = Some(effective_tight(child));
                }
            } else {
                bullet_run_tight = None;
            }
            prev_list = Some(child);
        } else {
            prev_list = None;
            bullet_run_tight = None;
        }
    }
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
    fn test_adjacent_different_kind_lists_no_end_list_artefact() {
        // #1397: adjacent lists of DIFFERENT kinds (bullet vs ordered vs
        // task) must save separated by blank lines only - never with the
        // renderer's <!-- end list --> comment, which used to reopen as a
        // phantom html code_block between the lists.
        for original in [
            "- a\n\n1. b\n",
            "1. a\n\n- b\n",
            "- a\n- b\n\n1. c\n2. d\n",
            "1. [ ] a\n\n- b\n",
            // Ordered plain/task pair whose numbering CONTINUES across
            // the boundary (task run starts at 2): merging on reparse
            // recomputes the same run order, so the separator drops.
            "1. a\n\n2. [ ] b\n",
        ] {
            let (first, second) = two_saves(original);
            assert!(
                !first.contains("end list"),
                "no end-list artefact for {original:?}: {first:?}"
            );
            assert!(
                !first.contains("```html"),
                "no html-fence artefact for {original:?}: {first:?}"
            );
            assert_eq!(first, second, "fixpoint for {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original),
                "reparse yields the same two lists for {original:?}"
            );
        }
    }

    #[test]
    fn test_adjacent_ordered_task_restart_keeps_separator() {
        // Codex review on PR #3258: an ordered_list next to a task_list
        // carrying attrs.order (or vice versa) renders as the SAME
        // ordered Markdown list type. When the second list restarts at
        // 1 instead of continuing the first list's numbering, dropping
        // the separator merges them on reparse and convert_list_runs
        // renumbers the second run (order 2), silently losing the
        // restart - so the separator stays for these pairs.
        for original in [
            "1. plain\n\n<!-- end list -->\n\n1. [ ] restarted\n",
            "1. [ ] first\n\n<!-- end list -->\n\n1. plain\n",
        ] {
            let (first, second) = two_saves(original);
            assert!(
                first.contains("<!-- end list -->"),
                "separator preserved for restart pair {original:?}: {first:?}"
            );
            assert_eq!(first, second, "restart shape is a fixpoint for {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original),
                "reparse preserves both lists and the restart for {original:?}"
            );
            let doc = markdown_to_prosemirror(&first);
            let content = doc["content"].as_array().unwrap();
            assert_eq!(content.len(), 2, "two lists for {original:?}");
            assert_eq!(content[1]["attrs"]["order"], 1, "second list restarts at 1 for {original:?}");
        }
    }

    #[test]
    fn test_adjacent_task_plain_tight_mismatch_keeps_separator() {
        // Codex review on PR #3258 (P2): a tight bullet list next to a
        // loose task list renders as the SAME Markdown list family.
        // Dropping the separator merges them on reparse and
        // convert_list_runs copies tight:false onto both runs, so the
        // next save inserts a blank line inside the first list - the
        // separator stays when tight differs within one render family.
        let original = "- a\n- aa\n\n<!-- end list -->\n\n- [ ] b\n\n- [ ] c\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("<!-- end list -->"),
            "separator preserved for tight-mismatched pair: {first:?}"
        );
        assert_eq!(first, second, "tight-mismatch shape is a fixpoint");
        assert_eq!(
            markdown_to_prosemirror(&first),
            markdown_to_prosemirror(original),
            "reparse preserves both lists and their tightness"
        );
        let doc = markdown_to_prosemirror(&first);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "two lists");
        assert_eq!(content[0]["attrs"]["tight"], true, "first list stays tight");
        assert_eq!(content[1]["attrs"]["tight"], false, "second list stays loose");
    }

    #[test]
    fn test_coalesced_bullet_run_boundary_uses_first_lists_tight() {
        // Codex review on PR #3258 (P2): a loose bullet list followed by
        // a tight bullet list coalesces in json_to_ast using the FIRST
        // list's tight (loose). The boundary to a following tight task
        // list must be judged against that effective tightness: the two
        // render as one loose bullet list, so dropping the separator
        // merged everything into one loose list on reparse and silently
        // loosened the task run (its next save grew a blank line inside
        // it). The separator stays.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "bullet_list", "attrs": {"tight": false}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]},
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "aa"}]}]}
                ]},
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]},
                {"type": "task_list", "attrs": {"tight": true}, "content": [
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "c"}]}]},
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "d"}]}]}
                ]}
            ]
        });
        let first = prosemirror_to_markdown(&doc).unwrap();
        assert!(
            first.contains("<!-- end list -->"),
            "separator kept for coalesced loose-bullet -> tight-task boundary: {first:?}"
        );
        let reparsed = markdown_to_prosemirror(&first);
        let second = prosemirror_to_markdown(&reparsed).unwrap();
        assert_eq!(first, second, "coalesced-boundary shape is a fixpoint");
        let content = reparsed["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "reparse yields bullet + task lists: {reparsed}");
        assert_eq!(content[1]["attrs"]["tight"], true, "task run stays tight");
    }

    #[test]
    fn test_legacy_task_list_boundary_uses_inferred_tight() {
        // CodeRabbit review on PR #3258: collect_boundary_kinds compared
        // raw attrs.tight, so a legacy task_list without the attribute
        // (json_to_ast infers tight for single-paragraph items) counted
        // as tight-differing next to an explicitly tight bullet list and
        // kept a separator the rendered output does not need. With the
        // inferred tightness the pair is a tight same-family pair whose
        // separator drops, and the save is a fixpoint.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "task_list", "content": [
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]}
            ]
        });
        let first = prosemirror_to_markdown(&doc).unwrap();
        assert!(
            !first.contains("end list"),
            "no separator for inferred-tight legacy task pair: {first:?}"
        );
        let reparsed = markdown_to_prosemirror(&first);
        let second = prosemirror_to_markdown(&reparsed).unwrap();
        assert_eq!(first, second, "legacy-tight pair is a fixpoint");
        let content = reparsed["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "reparse yields bullet + task lists: {reparsed}");
        assert_eq!(content[1]["attrs"]["tight"], true, "task run stays tight");
    }

    #[test]
    fn test_fence_closes_only_on_bare_run() {
        // CodeRabbit review on PR #3258 (Major): the fence scanner closed
        // a fence on ANY line starting with the marker, so a ```rust
        // content line ended the block early and a separator-looking line
        // later in the block was counted as renderer markup. A fence now
        // closes only on a bare run at least as long as its opener: the
        // authored line inside the block below must neither count toward
        // the boundary correspondence nor be dropped, while the real
        // bullet -> ordered separator still drops.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "ordered_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]}
            ]
        });
        let out = strip_end_list_separators(
            &doc,
            "```\ncode\n```rust\n<!-- end list -->\nmore\n```\n\n- a\n\n<!-- end list -->\n\n1. b\n",
        );
        assert_eq!(
            out.matches("<!-- end list -->").count(),
            1,
            "only the code block's authored line remains: {out:?}"
        );
        assert!(
            out.contains("```rust\n<!-- end list -->\nmore"),
            "code block content survives intact: {out:?}"
        );
    }

    #[test]
    fn test_boundary_order_overflow_does_not_panic() {
        // CodeRabbit review on PR #3258: the merge-safety check computed
        // prev_start + prev_count on a request-supplied attrs.order; a
        // start near u64::MAX overflowed (panic in debug builds, wrap in
        // release). Checked addition treats overflow as not-merge-safe,
        // so the separator stays and nothing panics.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "ordered_list", "attrs": {"tight": true, "order": u64::MAX}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "task_list", "attrs": {"tight": true, "order": 1}, "content": [
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]}
            ]
        });
        let out = strip_end_list_separators(
            &doc,
            "18446744073709551615. a\n\n<!-- end list -->\n\n1. [ ] b\n",
        );
        assert!(
            out.contains("<!-- end list -->"),
            "separator kept when the start sum overflows: {out:?}"
        );
    }

    #[test]
    fn test_end_list_literal_inside_code_fence_is_not_a_separator() {
        // Codex review on PR #3258 (P2): a fenced code block whose
        // literal content is the separator text is authored content.
        // It must not satisfy the boundary count, and must survive
        // while the real boundary separator drops. (Unit-level: the
        // markdown-text roundtrip of a bare HTML-comment fence hits
        // the parse-side htmlBlock handling, a different mechanism.)
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "ordered_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]},
                {"type": "code_block", "content": [{"type": "text", "text": "<!-- end list -->"}]}
            ]
        });
        let out = strip_end_list_separators(
            &doc,
            "- a\n\n<!-- end list -->\n\n1. b\n\n```\n<!-- end list -->\n```\n",
        );
        assert_eq!(
            out.matches("<!-- end list -->").count(),
            1,
            "only the code block's literal line remains: {out:?}"
        );
        assert!(
            out.contains("```\n<!-- end list -->\n```"),
            "code fence content survives verbatim: {out:?}"
        );
        assert!(
            out.starts_with("- a\n1. b"),
            "the real boundary separator dropped: {out:?}"
        );
    }

    #[test]
    fn test_table_cell_list_boundaries_do_not_consume_separator() {
        // Codex review on PR #3258 (P2): lists inside a table cell are
        // flattened to text and emit no separator. Their boundaries
        // must not consume the correspondence of a real top-level
        // boundary - here the doc has one real boundary (bullet ->
        // ordered at top level) and one cell-internal pair; the saved
        // text drops exactly the real separator.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "table", "content": [
                    {"type": "table_row", "content": [
                        {"type": "table_cell", "content": [
                            {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                                {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]}
                            ]},
                            {"type": "ordered_list", "attrs": {"tight": true}, "content": [
                                {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "y"}]}]}
                            ]}
                        ]}
                    ]}
                ]},
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "ordered_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]}
            ]
        });
        let mut kinds = Vec::new();
        collect_boundary_kinds(&doc, &mut kinds);
        assert_eq!(kinds.len(), 1, "only the top-level boundary is collected: {kinds:?}");
        let out = strip_end_list_separators(&doc, "- a\n\n<!-- end list -->\n\n1. b\n");
        assert!(!out.contains("end list"), "real separator dropped: {out:?}");
    }

    #[test]
    fn test_adjacent_bullet_to_ordered_starting_above_one_keeps_blank() {
        // Codex review on PR #3258 (P2): a tight bullet list followed by
        // a tight ordered list starting above 1 saved as `- a\n2. b` -
        // an ordered marker above 1 cannot interrupt a paragraph, so on
        // reparse `2. b` became a lazy continuation of the bullet item
        // and the ordered list (and its start) was lost. The boundary
        // keeps exactly one blank line instead of none.
        for original in ["- a\n\n2. b\n", "- [ ] a\n\n3. b\n"] {
            let (first, second) = two_saves(original);
            assert_eq!(first, original, "blank kept, start preserved: {first:?}");
            assert!(
                !first.contains("end list"),
                "no end-list artefact for {original:?}: {first:?}"
            );
            assert_eq!(first, second, "fixpoint for {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original),
                "reparse yields the same two lists for {original:?}"
            );
            let doc = markdown_to_prosemirror(&first);
            let content = doc["content"].as_array().unwrap();
            assert_eq!(content.len(), 2, "two lists for {original:?}");
            assert_eq!(content[1]["type"], "ordered_list", "second list stays ordered for {original:?}");
        }
    }

    #[test]
    fn test_adjacent_different_kind_lists_in_blockquote_fixpoint() {
        // #1397: the blockquote shape was not even a save fixpoint.
        let (first, second) = two_saves("> - a\n>\n> 1. b\n");
        assert!(
            !first.contains("end list"),
            "no end-list artefact: {first:?}"
        );
        assert_eq!(first, second, "blockquote shape is a fixpoint");
        let doc = markdown_to_prosemirror(&first);
        let quote = &doc["content"][0];
        assert_eq!(quote["type"], "blockquote");
        let kinds: Vec<&str> = quote["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["bullet_list", "ordered_list"]);
    }

    #[test]
    fn test_saved_end_list_artefact_reopens_without_phantom_block() {
        // #1397: a file already saved WITH the renderer artefact between
        // two lists reopens as exactly the two lists - no phantom
        // code_block between them.
        let doc = markdown_to_prosemirror("- a\n\n<!-- end list -->\n\n1. b\n");
        let kinds: Vec<&str> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["bullet_list", "ordered_list"]);
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
