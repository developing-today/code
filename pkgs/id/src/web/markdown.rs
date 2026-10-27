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
//! - `task_list`: Task list (GFM `- [ ]`/`- [x]` items); ordered task lists carry an `order` attribute
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
    let root = parse_document(&arena, markdown, &options);
    convert_node(root, &[])
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

        NodeValue::List(list) => {
            // Check if this is a task list by looking at children
            let is_task_list = node
                .children()
                .next()
                .is_some_and(|child| matches!(child.data.borrow().value, NodeValue::TaskItem(_)));

            if is_task_list {
                let content = collect_list_items(node);
                // #997: an ordered task list carries its start in
                // `attrs.order`, mirroring `ordered_list` below, or the
                // ordering is lost on save. Bullet task lists keep
                // emitting no attrs, preserving their current JSON shape.
                match list.list_type {
                    ListType::Ordered => {
                        json!({
                            "type": "task_list",
                            "attrs": {"order": list.start},
                            "content": content
                        })
                    }
                    ListType::Bullet => {
                        json!({
                            "type": "task_list",
                            "content": content
                        })
                    }
                }
            } else {
                let content = collect_list_items(node);
                let tight = list.tight;

                match list.list_type {
                    ListType::Ordered => {
                        json!({
                            "type": "ordered_list",
                            "attrs": {
                                "order": list.start,
                                "tight": tight
                            },
                            "content": content
                        })
                    }
                    ListType::Bullet => {
                        json!({
                            "type": "bullet_list",
                            "attrs": {"tight": tight},
                            "content": content
                        })
                    }
                }
            }
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

            // Get alt text from children (text content)
            let alt = get_text_content(node);
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

            json!({
                "type": "image",
                "attrs": attrs
            })
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
            // Convert HTML blocks to code blocks (safe fallback)
            let text = html.literal.strip_suffix('\n').unwrap_or(&html.literal);
            if text.is_empty() {
                json!({"type": "paragraph"})
            } else {
                json!({
                    "type": "code_block",
                    "attrs": {"params": "html"},
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
            // #945: a definition's AST children are block nodes (usually a
            // single paragraph); carry their inline content directly on a
            // block node with the label, mirroring the reference arm.
            let mut content = Vec::new();
            for child in node.children() {
                content.extend(collect_inline_children(child, &[]));
            }
            json!({
                "type": "footnote_definition",
                "attrs": {"label": definition.name},
                "content": content
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
    node.children()
        .map(|child| convert_node(child, &[]))
        .filter(|v| !v.is_null())
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
        "paragraph" | "heading" | "blockquote" | "code_block" | "horizontal_rule" | "bullet_list"
        | "ordered_list" | "task_list" | "table" | "footnote_definition" => Some(Placement::Block),
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
    // #1019: comrak's renderer percent-encodes a space in a link/image
    // destination (`my notes.md` -> `my%20notes.md`) instead of emitting
    // the <...> angle form, silently breaking relative file links and
    // mutating the PM href on reparse. The renderer has no angle-form
    // option, so swap each spaced destination for a unique placeholder
    // before rendering and restore it as <raw destination> afterwards.
    let substitutions = substitute_spaced_destinations(root);
    let mut output = String::new();
    format_commonmark(root, &commonmark_options(), &mut output)?;
    for (placeholder, raw) in substitutions {
        output = output.replace(&placeholder, &format!("<{raw}>"));
    }
    Ok(output)
}

/// Replace every link/image destination containing a space with a
/// unique placeholder token, returning (placeholder, raw destination)
/// pairs for restoration after rendering (#1019). The token is a valid
/// bare destination (no spaces or characters comrak encodes), so the
/// renderer emits it verbatim exactly where the destination belongs.
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
        if !url.contains(' ') {
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

/// Convert `ProseMirror` JSON to comrak AST.
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
            // Task list is a list whose children are TaskItems. #997: when
            // the parse side stored `attrs.order`, rebuild an ordered list
            // with that start; without it, a bullet list as before.
            #[allow(clippy::cast_possible_truncation)] // order is typically small
            let order = json["attrs"]["order"].as_u64().map(|o| o as usize);
            match order {
                Some(start) => NodeValue::List(comrak::nodes::NodeList {
                    list_type: ListType::Ordered,
                    start,
                    tight: true,
                    ..Default::default()
                }),
                None => NodeValue::List(comrak::nodes::NodeList {
                    list_type: ListType::Bullet,
                    tight: true,
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
            for child_json in content {
                if child_json["type"].as_str() == Some("paragraph") {
                    // Unwrap: add paragraph's inline children directly to cell
                    if let Some(para_content) = child_json["content"].as_array() {
                        for inline_json in para_content {
                            let inline_node = json_to_ast(arena, inline_json)?;
                            append_table_cell_child(arena, ast_node, inline_node);
                        }
                    }
                } else {
                    let child = json_to_ast(arena, child_json)?;
                    append_table_cell_child(arena, ast_node, child);
                }
            }
        }
        return Ok(ast_node);
    }

    // Special case: footnote definitions. The parse direction stores the
    // definition's inline content directly on the PM node (#945), but
    // comrak's AST requires a FootnoteDefinition's children to be blocks —
    // re-wrap the inline children in a Paragraph here, or the commonmark
    // formatter rejects the document as ill-formed.
    if node_type == "footnote_definition" {
        if let Some(content) = json["content"].as_array()
            && !content.is_empty()
        {
            let paragraph = arena.alloc(AstNode::from(NodeValue::Paragraph));
            for child_json in content {
                let child = json_to_ast(arena, child_json)?;
                append_inline_child(paragraph, child);
            }
            ast_node.append(paragraph);
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
        for child_json in content {
            let is_bullet_list = child_json["type"].as_str() == Some("bullet_list");
            let child = json_to_ast(arena, child_json)?;
            if is_bullet_list && let Some(list_node) = prev_bullet_list {
                let items: Vec<&AstNode<'a>> = child.children().collect();
                for item in items {
                    item.detach();
                    list_node.append(item);
                }
                continue;
            }
            append_inline_child(ast_node, child);
            prev_bullet_list = if is_bullet_list { Some(child) } else { None };
        }
    }

    // Special case: image needs alt text as child (but NOT for HtmlInline, i.e. sized images)
    if node_type == "image"
        && matches!(ast_node.data.borrow().value, NodeValue::Image(_))
        && let Some(alt) = json["attrs"]["alt"].as_str()
        && !alt.is_empty()
    {
        let text_node = arena.alloc(AstNode::from(NodeValue::Text(alt.to_owned().into())));
        ast_node.append(text_node);
    }

    // #880: an image carrying linkHref (a linked image, see convert_node)
    // is wrapped in a Link node so save emits [![alt](src)](href).
    if node_type == "image"
        && matches!(ast_node.data.borrow().value, NodeValue::Image(_))
        && let Some(link_href) = json["attrs"]["linkHref"].as_str()
        && !link_href.is_empty()
    {
        let link_node = arena.alloc(AstNode::from(NodeValue::Link(Box::new(
            comrak::nodes::NodeLink {
                url: link_href.to_owned(),
                title: json["attrs"]["linkTitle"].as_str().unwrap_or("").to_owned(),
            },
        ))));
        link_node.append(ast_node);
        return Ok(link_node);
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
        append_inline_child(cell, child);
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

/// Append an inline child, merging it into the previous sibling when
/// both are the same mark wrapper (#953): `[a `code` b](href)` reaches
/// the save direction as three text nodes that share the link mark, and
/// three sibling Link nodes render as three fragmented links. Moving
/// the new wrapper's children into the previous wrapper (recursively,
/// so nested wrappers merge too) emits one link containing all three
/// segments. Non-wrapper children are appended unchanged.
fn append_inline_child<'a>(parent: &'a AstNode<'a>, child: &'a AstNode<'a>) {
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
                append_inline_child(last, grandchild);
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
            match prosemirror_to_markdown(doc) {
                Err(ConversionError::MisplacedNode { .. }) => {}
                other => panic!("{name}: expected MisplacedNode error, got {other:?}"),
            }
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
        let definition_text: Vec<&str> = definition["content"]
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

        // A bullet task list keeps its current PM shape: no attrs at all.
        let bullet = markdown_to_prosemirror("- [ ] todo\n");
        assert_eq!(bullet["content"][0]["type"], "task_list");
        assert!(bullet["content"][0]["attrs"].is_null());
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
}
