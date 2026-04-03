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
use comrak::{format_commonmark, parse_document, Arena, Options};
use serde_json::{json, Value};

/// Error type for markdown conversion.
#[derive(Debug)]
pub enum ConversionError {
    /// The JSON structure is missing a required `type` field.
    MissingType,
    /// The JSON contains an unknown node type.
    UnknownType(String),
    /// Failed to format markdown output.
    FormatError(std::fmt::Error),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingType => write!(f, "JSON node missing 'type' field"),
            Self::UnknownType(t) => write!(f, "Unknown node type: {t}"),
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
    options.parse.smart = false; // Don't convert quotes/dashes
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
            json!(collect_inline_children(node, &marks))
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

            json!({
                "type": "image",
                "attrs": attrs
            })
        }

        // Unsupported nodes - pass through as best we can
        NodeValue::HtmlBlock(html) => {
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
            // Inline HTML becomes plain text
            text_node_with_marks(html, active_marks)
        }

        // GFM features we don't support - convert to plain text or skip
        NodeValue::Table(_) | NodeValue::TableRow(_) | NodeValue::TableCell => {
            // Tables are not supported - this shouldn't happen as we collect text
            json!({"type": "paragraph"})
        }

        NodeValue::TaskItem(_) => {
            // Task items become regular list items
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

        NodeValue::Strikethrough => {
            // Strikethrough - collect children with strikethrough mark added
            let mut marks = active_marks.to_vec();
            marks.push(Mark {
                mark_type: "strikethrough",
                attrs: None,
            });
            json!(collect_inline_children(node, &marks))
        }

        NodeValue::FootnoteDefinition(_) | NodeValue::FootnoteReference(_) => {
            // Footnotes not supported - skip
            json!(null)
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
    if text.is_empty() {
        return json!(null);
    }

    let mut node = json!({
        "type": "text",
        "text": text
    });

    if !marks.is_empty() {
        let marks_json: Vec<Value> = marks
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

/// Collect inline children of a node, flattening nested mark wrappers.
fn collect_inline_children<'a>(node: &'a AstNode<'a>, active_marks: &[Mark]) -> Vec<Value> {
    let mut result = Vec::new();

    for child in node.children() {
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
    let arena = Arena::new();
    let root = json_to_ast(&arena, doc)?;
    let mut output = String::new();
    format_commonmark(root, &commonmark_options(), &mut output)?;
    Ok(output)
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
            let content = json["content"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|n| n["text"].as_str())
                .unwrap_or("");
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

        "image" => {
            let src = json["attrs"]["src"].as_str().unwrap_or("");
            let title = json["attrs"]["title"].as_str().unwrap_or("");
            // Alt text will be added as child text node
            NodeValue::Image(Box::new(comrak::nodes::NodeLink {
                url: src.to_owned(),
                title: title.to_owned(),
            }))
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

    // Process children (except for code_block which stores content in literal, not as children)
    if node_type != "code_block"
        && let Some(content) = json["content"].as_array()
    {
        for child_json in content {
            let child = json_to_ast(arena, child_json)?;
            ast_node.append(child);
        }
    }

    // Special case: image needs alt text as child
    if node_type == "image"
        && let Some(alt) = json["attrs"]["alt"].as_str()
        && !alt.is_empty()
    {
        let text_node = arena.alloc(AstNode::from(NodeValue::Text(alt.to_owned().into())));
        ast_node.append(text_node);
    }

    Ok(ast_node)
}

/// Create marked text node(s) - handles nested marks.
fn create_marked_text<'a>(arena: &'a Arena<'a>, text: &str, marks: &[Value]) -> &'a AstNode<'a> {
    if marks.is_empty() {
        return arena.alloc(AstNode::from(NodeValue::Text(text.to_owned().into())));
    }

    // Check if code mark is present - it takes precedence and replaces the text node
    for mark in marks {
        if mark["type"].as_str() == Some("code") {
            return arena.alloc(AstNode::from(NodeValue::Code(comrak::nodes::NodeCode {
                num_backticks: 1,
                literal: text.to_owned(),
            })));
        }
    }

    // Collect wrapper values (innermost first for building)
    let mut wrappers: Vec<NodeValue> = Vec::new();

    for mark in marks {
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
            _ => continue, // Skip unknown marks
        };
        wrappers.push(wrapper_value);
    }

    // Start with text node
    let text_node = arena.alloc(AstNode::from(NodeValue::Text(text.to_owned().into())));

    if wrappers.is_empty() {
        return text_node;
    }

    // Build wrapper chain recursively - innermost mark wraps text, outer marks wrap that
    // marks array is [innermost, ..., outermost] so we build from index 0
    build_mark_chain(arena, text_node, &wrappers, 0)
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
        assert!(content[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("function test()"));
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
}
