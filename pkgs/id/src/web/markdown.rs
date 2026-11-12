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
//! - `front_matter`: YAML front matter block with `text` attribute (legal only as first child of `doc`)
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

use comrak::nodes::{AstNode, ListDelimType, ListType, NodeValue};
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
    options.extension.autolink = true; // #998: bare URLs/emails become links
    options.extension.footnotes = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());
    options.parse.smart = false; // Don't convert quotes/dashes
    options.render.r#unsafe = true; // Allow HTML output (needed for sized images)
    // #954: the CommonMark renderer saves an info-less code block as an
    // indented block whenever it can (its fenced branch otherwise needs a
    // non-empty info string or a literal edge case). Force the fenced
    // spelling for every code block so info-less fences round-trip as
    // fences. Render-only: parsing is unaffected.
    options.render.prefer_fenced = true;
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
    preserve_github_alert_marker_breaks(root);
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
        let footnote_link_references = preserve_unreferenced_footnote_definitions(
            &mut doc,
            &protected,
            newline_token.as_deref(),
        );
        preserve_unreferenced_link_reference_definitions(
            &mut doc,
            root,
            &protected,
            &footnote_link_references,
            newline_token.as_deref(),
        );
    }
    doc
}

/// Keep the line break after an exact GitHub alert marker in a blockquote.
/// The editor normally turns CommonMark soft breaks into spaces, which would
/// make an alert marker followed by its body indistinguishable from ordinary
/// text such as `[!NOTE] extra words` when the document is saved again.
///
/// Only a blockquote whose FIRST child is a paragraph can open with an
/// alert marker: when the first child is another block (a nested quote, a
/// list, a fence), GitHub does not treat a later `[!NOTE]` line as an
/// alert, so promoting its break would change authored plain text
/// (Codex P2 on the first cut).
fn preserve_github_alert_marker_breaks<'a>(node: &'a AstNode<'a>) {
    if matches!(&node.data.borrow().value, NodeValue::BlockQuote) {
        if let Some(first_block) = node.children().next()
            && matches!(&first_block.data.borrow().value, NodeValue::Paragraph)
        {
            let mut first_line = String::new();
            for child in first_block.children() {
                let (text, is_break, is_other) = {
                    let data = child.data.borrow();
                    match &data.value {
                        NodeValue::Text(text) => (Some(text.to_string()), false, false),
                        NodeValue::SoftBreak | NodeValue::LineBreak => (None, true, false),
                        _ => (None, false, true),
                    }
                };

                if let Some(text) = text {
                    first_line.push_str(&text);
                } else if is_break {
                    if is_github_alert_marker(&first_line) {
                        child.data.borrow_mut().value = NodeValue::LineBreak;
                    }
                    break;
                } else if is_other {
                    break;
                }
            }
        }
    }

    for child in node.children() {
        preserve_github_alert_marker_breaks(child);
    }
}

fn is_github_alert_marker(text: &str) -> bool {
    matches!(
        text.trim(),
        "[!NOTE]" | "[!TIP]" | "[!IMPORTANT]" | "[!WARNING]" | "[!CAUTION]"
    )
}

/// Split a line's leading blockquote markers from its content: returns the
/// marker depth and the remainder. Each level is one `>` plus at most one
/// following space or tab, matching how comrak renders nested quotes.
fn split_quote_prefix(line: &str) -> (usize, &str) {
    let mut rest = line;
    let mut depth = 0;
    while let Some(after) = rest.strip_prefix('>') {
        depth += 1;
        rest = after
            .strip_prefix(' ')
            .or_else(|| after.strip_prefix('\t'))
            .unwrap_or(after);
    }
    (depth, rest)
}

/// The content of `line` as seen by the container that opened at
/// `depth` quote levels: exactly that many markers are stripped and any
/// deeper ones stay in place. Returns None when the line is outside that
/// container, which ends a fence or HTML block opened inside it.
fn strip_quote_depth(line: &str, depth: usize) -> Option<&str> {
    let mut rest = line;
    for _ in 0..depth {
        let after = rest.strip_prefix('>')?;
        rest = after
            .strip_prefix(' ')
            .or_else(|| after.strip_prefix('\t'))
            .unwrap_or(after);
    }
    Some(rest)
}

mod markdown_scan;
mod markdown_serialization;
mod parse_helpers;
mod prosemirror_json;

pub(crate) use markdown_scan::*;
pub use markdown_serialization::prosemirror_to_markdown;
pub(crate) use markdown_serialization::*;
pub(crate) use parse_helpers::*;
pub(crate) use prosemirror_json::*;
fn json_to_ast<'a>(arena: &'a Arena<'a>, json: &Value) -> Result<&'a AstNode<'a>, ConversionError> {
    let node_type = json["type"].as_str().ok_or(ConversionError::MissingType)?;

    let node_value = match node_type {
        "doc" => NodeValue::Document,

        "paragraph" => NodeValue::Paragraph,

        "heading" => {
            let level = json["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6);
            #[allow(clippy::cast_possible_truncation)] // level is clamped to 1-6 before narrowing
            let level = level as u8;
            NodeValue::Heading(comrak::nodes::NodeHeading {
                level,
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
                // #954: comrak's own empty-fence literal is empty; appending
                // the usual trailing newline here renders a spurious blank
                // line inside the fence ("```\n\n```").
                literal: if content.is_empty() {
                    String::new()
                } else {
                    format!("{content}\n")
                },
                closed: false,
            }))
        }

        "horizontal_rule" => NodeValue::ThematicBreak,

        // #1369: YAML front matter (`---` delimiter only; comrak has no
        // TOML `+++` support) carried verbatim in the `text` attr.
        "front_matter" => {
            NodeValue::FrontMatter(json["attrs"]["text"].as_str().unwrap_or("").to_owned())
        }

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
            // #1380: carry the delimiter style; absent (legacy docs)
            // defaults to Period in delimiter_from_attrs.
            NodeValue::List(comrak::nodes::NodeList {
                list_type: ListType::Ordered,
                start: order,
                tight,
                delimiter: delimiter_from_attrs(json),
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
                    // #1380: same delimiter carry as ordered_list.
                    delimiter: delimiter_from_attrs(json),
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
            let src = sanitize_image_url(json["attrs"]["src"].as_str().unwrap_or(""));
            let title = json["attrs"]["title"].as_str().unwrap_or("");
            let alt = json["attrs"]["alt"].as_str().unwrap_or("");
            let width = json["attrs"]["width"].as_u64();
            let height = json["attrs"]["height"].as_u64();

            // If image has explicit dimensions, emit as HTML <img> tag
            if width.is_some() || height.is_some() {
                // #873: escape values — a raw `"` truncated the attribute on
                // reparse and could inject new attributes.
                let mut tag = format!("<img src=\"{}\"", escape_attr_value(&src));
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
                    // #5745: a PM doc that arrived as raw ProseMirror JSON
                    // (collab Rich mode) never passed the parse-direction
                    // sanitizer, so neutralize unsafe hrefs on the way out.
                    let safe_href = sanitize_link_url(link_href);
                    let mut anchor = format!("<a href=\"{}\"", escape_attr_value(&safe_href));
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
                    url: src,
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
                    // GFM cells cannot contain block breaks. Preserve word
                    // boundaries when adjacent paragraphs are flattened.
                    if idx > 0 && content[idx - 1]["type"].as_str() == Some("paragraph") {
                        ast_node.append(arena.alloc(AstNode::from(NodeValue::Text(" ".into()))));
                    }
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

    // Process children (except for code_block which stores content in literal, not as children;
    // front_matter likewise stores its payload in the `text` attr, #1369)
    if node_type != "code_block"
        && node_type != "front_matter"
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
        let normalized_content = normalize_hard_break_placement(node_type, content);
        for (idx, child_json) in normalized_content.iter().enumerate() {
            let is_bullet_list = child_json["type"].as_str() == Some("bullet_list");
            let normalized = normalize_inline_child_json(&normalized_content, idx);
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
                    // #5745: same save-direction neutralization as above.
                    url: sanitize_link_url(link_href),
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
                    // #5745: same save-direction neutralization as above.
                    url: sanitize_link_url(href),
                    title: title.to_owned(),
                }))
            }
            // #1045: `code` is skipped here because it is handled
            // below as the innermost node; any other unrecognized mark
            // is skipped by design — the text survives, the unknown
            // styling is shed (see `prosemirror_to_markdown`).
            _ => continue,
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
/// Each line becomes a paragraph. The split is on the newline
/// character (not `str::lines`), so a trailing newline yields a final
/// empty paragraph and the save side (`extract_plain_text`, which joins
/// paragraphs with a newline) reproduces it exactly: no-edit open/save
/// is byte-stable for the trailing newline (#1307). A carriage return
/// left by a CRLF ending is stripped, but only from segments the newline
/// actually terminated: a lone trailing CR is content and survives. CRLF
/// files are normalized to LF on save, with their trailing-newline state
/// preserved (Raw mode stays byte-exact).
#[must_use]
pub fn plain_text_to_prosemirror(text: &str) -> Value {
    let segments: Vec<&str> = text.split('\n').collect();
    let last = segments.len() - 1;
    let paragraphs: Vec<Value> = segments
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            // Only a segment actually terminated by '\n' can carry the
            // CR of a CRLF ending; the final segment keeps a lone
            // trailing CR verbatim (Codex finding on PR #4613).
            if i < last {
                line.strip_suffix('\r').unwrap_or(line)
            } else {
                line
            }
        })
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
    fn test_heading_levels_are_clamped_before_u8_cast_on_save() {
        let cases: &[(Option<u64>, u8)] = &[
            (None, 1),
            (Some(0), 1),
            (Some(1), 1),
            (Some(2), 2),
            (Some(3), 3),
            (Some(4), 4),
            (Some(5), 5),
            (Some(6), 6),
            (Some(7), 6),
            (Some(255), 6),
            (Some(256), 6),
            (Some(261), 6),
            (Some(1_000), 6),
            (Some(u64::MAX), 6),
        ];

        for (level, expected_level) in cases {
            let heading = match level {
                Some(level) => json!({
                    "type": "heading",
                    "attrs": {"level": level},
                    "content": [{"type": "text", "text": "Heading"}]
                }),
                None => json!({
                    "type": "heading",
                    "content": [{"type": "text", "text": "Heading"}]
                }),
            };
            let doc = json!({"type": "doc", "content": [heading]});
            let expected = format!("{} Heading\n", "#".repeat(usize::from(*expected_level)));

            assert_eq!(
                prosemirror_to_markdown(&doc).unwrap(),
                expected,
                "heading level {level:?}"
            );
        }
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
    fn test_image_executable_schemes_neutralized() {
        // #6364: the image URL oracle allows only http/https, scheme-less
        // references, and the explicitly allowed raster data-image types.
        for src in [
            "javascript:alert",
            "JaVaScRiPt:alert",
            "vbscript:msgbox",
            "data:text/html,hello",
            "data:image/svg+xml;base64,PHN2Zz4=",
            "file:///etc/passwd",
            "unknown-scheme:payload",
        ] {
            let md = format!("![open]({src})\n");
            let doc = markdown_to_prosemirror(&md);
            let image = doc["content"][0]["content"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["type"] == "image")
                .unwrap_or_else(|| panic!("no image node for {md:?}"));
            assert_eq!(image["attrs"]["src"], "#", "unsafe src survived for {md:?}");

            let (first, second) = two_saves(&md);
            assert_eq!(first, "![open](#)\n", "save not neutralized for {md:?}");
            assert_eq!(first, second, "no sanitized fixpoint for {md:?}");
        }
    }

    #[test]
    fn test_image_url_oracle_and_safe_forms() {
        // #6364 policy oracle: accepted references remain byte-identical;
        // unsafe, obfuscated, unsupported, and malformed data schemes fail
        // closed to the inert fragment.
        for (url, expected) in [
            ("relative.png", true),
            ("/blob/abc", true),
            ("//images.example/x.png", true),
            ("#fragment", true),
            ("images/name:part.png", true),
            ("http://images.example/x.png", true),
            ("HTTPS://images.example/x.png", true),
            ("data:image/png;base64,AA==", true),
            ("DaTa:IMAGE/JPEG,AA==", true),
            ("data:image/webp;charset=utf-8,AA==", true),
            ("javascript:alert(1)", false),
            ("JaVaScRiPt:alert(1)", false),
            ("java\tscript:alert(1)", false),
            ("h\nttps://images.example/x.png", false),
            ("data:text/html,hello", false),
            ("data:image/svg+xml,<svg>", false),
            ("data:image/png", false),
            ("file:///etc/passwd", false),
            ("mailto:image@example.test", false),
        ] {
            assert_eq!(
                image_url_is_safe(url),
                expected,
                "image URL oracle mismatch for {url:?}"
            );
            assert_eq!(
                sanitize_image_url(url),
                if expected { url } else { "#" },
                "image URL sanitizer mismatch for {url:?}"
            );
        }
    }

    #[test]
    fn test_raw_html_image_sources_are_sanitized() {
        // Raw HTML imports bypass the Markdown destination parser, so they
        // must use the same image-specific oracle directly.
        for src in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java\tscript:alert(1)",
            "data:image/svg+xml,<svg>",
        ] {
            let html = format!("<img src=\"{src}\" alt=\"open\" />\n");
            let doc = markdown_to_prosemirror(&html);
            let image = &doc["content"][0]["content"][0];
            assert_eq!(image["type"], "image", "raw image not imported: {html:?}");
            assert_eq!(image["attrs"]["src"], "#", "unsafe src survived: {html:?}");
        }
    }

    #[test]
    fn test_prosemirror_image_sources_are_sanitized_on_save() {
        // Collab PM JSON can bypass import sanitization. Cover both Markdown
        // and sized-HTML emission paths so each save direction is protected.
        for attrs in [
            json!({"src": "javascript:alert(1)", "alt": "open"}),
            json!({"src": "data:text/html,hello", "alt": "open", "width": 240}),
        ] {
            let doc = json!({
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "image", "attrs": attrs}]}]
            });
            let first = prosemirror_to_markdown(&doc).expect("save converts");
            assert!(
                first.contains("src=\"#\"") || first.contains("](#)"),
                "unsafe src in {first:?}"
            );
            let reparsed = markdown_to_prosemirror(&first);
            assert_eq!(reparsed["content"][0]["content"][0]["attrs"]["src"], "#");
            let second = prosemirror_to_markdown(&reparsed).expect("second save converts");
            assert_eq!(first, second, "sanitized image did not reach a fixpoint");
        }
    }

    #[test]
    fn test_link_executable_schemes_neutralized() {
        // #5745 regression: a javascript: link must not produce an
        // executable href. Oracle: the allowed set documented on
        // link_url_is_safe (http/https/mailto or scheme-less).
        for md in [
            "[open](javascript:alert(document.domain))",
            "[open](JaVaScRiPt:alert(1))",
            "[open](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
            "[open](vbscript:msgbox(1))",
            "[open](  javascript:alert(1))",
            "[open](unknown-scheme:payload)",
        ] {
            let doc = markdown_to_prosemirror(md);
            let para_content = doc["content"][0]["content"].as_array().unwrap();
            let link_text = para_content
                .iter()
                .find(|n| {
                    n["marks"]
                        .as_array()
                        .is_some_and(|m| m.iter().any(|mark| mark["type"] == "link"))
                })
                .unwrap_or_else(|| panic!("no link mark for {md}"));
            // The link text survives; only the target is neutralized.
            assert_eq!(link_text["text"], "open", "link text lost for {md}");
            let link_mark = link_text["marks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["type"] == "link")
                .unwrap();
            assert_eq!(
                link_mark["attrs"]["href"], "#",
                "executable href survived for {md}"
            );
            // Save direction: the emitted Markdown carries the inert
            // href, and re-parsing it is a stable fixpoint.
            let saved = prosemirror_to_markdown(&doc).unwrap();
            assert_eq!(saved, "[open](#)\n", "save not neutralized for {md}");
            let (first, second) = two_saves(&saved);
            assert_eq!(first, second, "no fixpoint after sanitize for {md}");
        }
    }

    #[test]
    fn test_link_tab_obfuscated_destination_forms_no_link() {
        // #5745: a literal TAB inside a Markdown link destination is
        // rejected by Comrak's destination grammar, so the source
        // stays literal text and no link node — hence no href at
        // all — is produced; there is nothing executable to render.
        // TAB-obfuscated hrefs that arrive through the raw-HTML or
        // ProseMirror-JSON paths (where no Comrak destination parse
        // happens) are covered by the sanitizer's tab-stripping probe
        // and the save-direction test below.
        let doc = markdown_to_prosemirror("[open](java\tscript:alert(1))");
        let para_content = doc["content"][0]["content"].as_array().unwrap();
        assert!(
            para_content.iter().all(|n| {
                n["marks"]
                    .as_array()
                    .is_none_or(|marks| marks.iter().all(|mark| mark["type"] != "link"))
            }),
            "tab-obfuscated destination produced a link mark: {para_content:?}"
        );
    }

    #[test]
    fn test_link_safe_schemes_preserved() {
        // #5745: every member of the documented allowed set (plus the
        // scheme-less forms) keeps its href byte-for-byte, in both
        // directions.
        for (md, expected_href) in [
            ("[a](http://example.com)", "http://example.com"),
            (
                "[a](https://example.com/x?y=1)",
                "https://example.com/x?y=1",
            ),
            ("[a](HTTPS://example.com)", "HTTPS://example.com"),
            ("[a](mailto:me@example.com)", "mailto:me@example.com"),
            ("[a](notes.md)", "notes.md"),
            ("[a](/abs/path.md)", "/abs/path.md"),
            ("[a](#section)", "#section"),
            ("[a](//example.com/x)", "//example.com/x"),
        ] {
            let doc = markdown_to_prosemirror(md);
            let para_content = doc["content"][0]["content"].as_array().unwrap();
            let link_mark = para_content[0]["marks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["type"] == "link")
                .unwrap_or_else(|| panic!("no link mark for {md}"));
            assert_eq!(
                link_mark["attrs"]["href"], expected_href,
                "safe href mutated for {md}"
            );
            let (first, second) = two_saves(&format!("{md}\n"));
            assert_eq!(first, format!("{md}\n"), "safe link not preserved for {md}");
            assert_eq!(first, second, "no fixpoint for {md}");
        }
    }

    #[test]
    fn test_link_sanitizer_save_direction_and_linked_image() {
        // #5745: a PM doc that never passed the parse direction (the
        // collab Rich path stores raw ProseMirror JSON) is neutralized
        // on save.
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "click",
                    "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]
                }]
            }]
        });
        let saved = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(saved, "[click](#)\n");

        // Same path with a TAB-obfuscated scheme: the sanitizer strips
        // TAB/LF/CR before reading the scheme, so this is neutralized
        // too (a Markdown destination could never carry it — Comrak
        // rejects it — but raw ProseMirror JSON can).
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "click",
                    "marks": [{"type": "link", "attrs": {"href": "java\tscript:alert(1)"}}]
                }]
            }]
        });
        let saved = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(saved, "[click](#)\n");

        // A raw-HTML anchor wrapping an image (the parse_anchor_tag
        // path) gets the same neutralization instead of an executable
        // linkHref.
        let doc = markdown_to_prosemirror(
            "<a href=\"javascript:alert(1)\"><img src=\"x.png\" width=\"240\" /></a>",
        );
        let node = &doc["content"][0]["content"][0];
        assert_eq!(node["type"], "image");
        assert_eq!(node["attrs"]["linkHref"], "#");
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
    fn test_bare_url_becomes_link_mark() {
        // #998: with the autolink extension enabled, a bare URL parses
        // to a text node carrying a link mark, like the angle form.
        let doc = markdown_to_prosemirror("visit https://example.com/path?a=1 now");
        let para_content = doc["content"][0]["content"].as_array().unwrap();
        let link_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "link"))
        });
        assert!(link_text.is_some(), "bare URL has no link mark");
        let link_mark = link_text.unwrap()["marks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["type"] == "link")
            .unwrap();
        assert_eq!(link_mark["attrs"]["href"], "https://example.com/path?a=1");
    }

    #[test]
    fn test_bare_email_becomes_link_mark() {
        // #998: a bare email address autolinks with a mailto: href.
        let doc = markdown_to_prosemirror("mail me@example.com today");
        let para_content = doc["content"][0]["content"].as_array().unwrap();
        let link_text = para_content.iter().find(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "link"))
        });
        assert!(link_text.is_some(), "bare email has no link mark");
        let link = link_text.unwrap();
        assert_eq!(link["text"], "me@example.com");
        let link_mark = link["marks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["type"] == "link")
            .unwrap();
        assert_eq!(link_mark["attrs"]["href"], "mailto:me@example.com");
    }

    #[test]
    fn test_bare_url_fixpoint_and_code_span_negative() {
        // #998: a bare URL round-trips to a stable fixpoint, and a URL
        // inside a code span stays literal text with no link mark.
        let (first, second) = two_saves("visit https://example.com/path?a=1 now\n");
        assert_eq!(first, second, "no fixpoint for bare URL");
        assert!(
            first.contains("https://example.com/path?a=1"),
            "URL lost in save: {first:?}"
        );

        let doc = markdown_to_prosemirror("run `https://example.com` now");
        let para_content = doc["content"][0]["content"].as_array().unwrap();
        let has_link = para_content.iter().any(|n| {
            n["marks"]
                .as_array()
                .is_some_and(|m| m.iter().any(|mark| mark["type"] == "link"))
        });
        assert!(!has_link, "code-span URL must not carry a link mark");
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
    fn test_unknown_marks_are_skipped_by_design() {
        // #1045: unknown marks are shed while their text survives —
        // unlike unknown nodes, which error. Pins the documented
        // skip policy on `prosemirror_to_markdown`.
        let text = |marks: Value| {
            json!({"type": "doc", "content": [{"type": "paragraph", "content": [
                {"type": "text", "text": "hi", "marks": marks}
            ]}]})
        };
        // Bare unknown mark: text survives, mark vanishes.
        let doc = text(json!([{"type": "underline"}]));
        assert_eq!(prosemirror_to_markdown(&doc).unwrap(), "hi\n");
        // Mixed known + unknown: the known mark still renders.
        let doc = text(json!([{"type": "strong"}, {"type": "underline"}]));
        assert_eq!(prosemirror_to_markdown(&doc).unwrap(), "**hi**\n");
        // Unknown mark alongside code + link: code-mark special
        // handling is unchanged.
        let doc = text(json!([
            {"type": "code"},
            {"type": "link", "attrs": {"href": "http://e.com"}},
            {"type": "underline"},
        ]));
        assert_eq!(
            prosemirror_to_markdown(&doc).unwrap(),
            "[`hi`](http://e.com)\n"
        );
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
    fn test_blockquote_leading_block_round_trips_without_extra_blank_lines() {
        // #1002: comrak's formatter inserts empty quote lines before a
        // quote's first block when it is a fence, table, or rule unless
        // the save path strips them. Inputs use the renderer's canonical
        // spellings (padded table delimiters, `-----` rules, `"> "` blank
        // quote lines), so the first save is byte-identical and stable.
        // The last two cases are controls: quotes starting with a
        // paragraph, and a paragraph-then-fence quote whose single
        // interior separator must survive.
        for original in [
            "> ```rust\n> code\n> ```\n",
            "> | a | b |\n> | --- | --- |\n> | 1 | 2 |\n",
            "> -----\n> \n> after\n",
            "> > ```rust\n> > code\n> > ```\n",
            "> ```rust\n> code\n> ```\n\n> second\n",
            "> paragraph\n",
            "> # heading\n",
            "> - item\n",
            "> paragraph\n> \n> ```rust\n> code\n> ```\n",
        ] {
            let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
            assert_eq!(saved, original, "first save polluted {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&saved),
                markdown_to_prosemirror(original),
                "structure changed for {original:?}"
            );
            let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
            assert_eq!(resaved, saved, "no fixpoint for {original:?}");
        }
        // Unpadded table delimiters still take comrak's canonical padded
        // spelling (pre-existing behaviour, also outside quotes), but the
        // save must not add leading blank quote lines and must reopen to
        // the same document.
        let unpadded = "> | a | b |\n> |---|---|\n> | 1 | 2 |\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(unpadded)).unwrap();
        assert_eq!(
            saved, "> | a | b |\n> | --- | --- |\n> | 1 | 2 |\n",
            "table canonicalized without leading blanks: {saved:?}"
        );
        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(unpadded),
            "table structure changed"
        );
    }

    #[test]
    fn test_top_level_fence_with_quote_like_content_round_trips() {
        // #8665: strip_leading_blank_quote_lines had no fence state, so
        // a top-level fenced block whose first content lines are `> `
        // and `> text` lost its first line on save — the fence opener
        // reset the quote-run scan and the literal code content was
        // stripped as if it led a blockquote.
        for original in [
            "```\n> \n> text\n```\n",
            "```rust\n> \n> text\n```\n",
            "```\n> \n> text\n```\n\n> quote\n",
        ] {
            let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
            assert_eq!(saved, original, "first save polluted {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&saved),
                markdown_to_prosemirror(original),
                "structure changed for {original:?}"
            );
            let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
            assert_eq!(resaved, saved, "no fixpoint for {original:?}");
        }
        // Control: quote-like lines inside a blockquote's own fence are
        // content too — structure and fixpoint must hold.
        let quoted = "> ```\n> \n> text\n> ```\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(quoted)).unwrap();
        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(quoted),
            "quoted fence structure changed: {saved:?}"
        );
        let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
        assert_eq!(resaved, saved, "no fixpoint for quoted fence");
        // Tilde fences are canonicalized to backticks by the renderer,
        // but their content must survive intact and reach a fixpoint.
        let tilde = "~~~\n> \n> text\n~~~\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(tilde)).unwrap();
        assert_eq!(
            saved, "```\n> \n> text\n```\n",
            "tilde fence content changed: {saved:?}"
        );
        let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
        assert_eq!(resaved, saved, "no fixpoint for tilde fence");
    }

    #[test]
    fn test_list_fence_does_not_suppress_quote_cleanup() {
        // PR #8678 Codex P2: a list item whose first block is fenced
        // code renders its opener behind the item marker (- ```), so
        // the fence scan missed it and mistook the closing delimiter
        // for a new fence; the phantom fence then suppressed the
        // #1002 leading-blank cleanup for every later quote in the
        // document. The rule-first quote below needs that cleanup.
        let original = "- ```\n  code\n  ```\n\n> -----\n> \n> after\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
        assert_eq!(saved, original, "first save polluted {original:?}");
        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(original),
            "structure changed for {original:?}"
        );
        let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
        assert_eq!(resaved, saved, "no fixpoint for {original:?}");
    }

    #[test]
    fn test_html_comment_fence_does_not_suppress_quote_cleanup() {
        // PR #8678 Codex P2, second round: a fence-shaped line inside
        // a raw HTML comment is literal content, not a fence opener.
        // Treating it as one opened a phantom fence that suppressed
        // the #1002 cleanup for the rest of the document (the
        // rule-first quote below needs it).
        let original = "<!--\n```\n-->\n\n> -----\n> \n> after\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
        assert_eq!(saved, original, "first save polluted {original:?}");
        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(original),
            "structure changed for {original:?}"
        );
        let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
        assert_eq!(resaved, saved, "no fixpoint for {original:?}");
    }

    #[test]
    fn test_indented_html_comment_terminator_releases_cleanup() {
        // PR #8678 Codex P2, third round: the htmlBlock serialization
        // path preserves indentation, so a comment terminator can be
        // indented four spaces. The scan's indent limit applies to
        // opening detection, not to an already-active block: the
        // terminator must still be fed, or html state leaks into the
        // rest of the document and suppresses the #1002 cleanup for
        // the rule-first quote below.
        let original = "<!--\n    -->\n\n> -----\n> \n> after\n";
        let saved = prosemirror_to_markdown(&markdown_to_prosemirror(original)).unwrap();
        assert_eq!(saved, original, "first save polluted {original:?}");
        assert_eq!(
            markdown_to_prosemirror(&saved),
            markdown_to_prosemirror(original),
            "structure changed for {original:?}"
        );
        let resaved = prosemirror_to_markdown(&markdown_to_prosemirror(&saved)).unwrap();
        assert_eq!(resaved, saved, "no fixpoint for {original:?}");
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
            assert_eq!(
                first, second,
                "restart shape is a fixpoint for {original:?}"
            );
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original),
                "reparse preserves both lists and the restart for {original:?}"
            );
            let doc = markdown_to_prosemirror(&first);
            let content = doc["content"].as_array().unwrap();
            assert_eq!(content.len(), 2, "two lists for {original:?}");
            assert_eq!(
                content[1]["attrs"]["order"], 1,
                "second list restarts at 1 for {original:?}"
            );
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
        assert_eq!(
            content[1]["attrs"]["tight"], false,
            "second list stays loose"
        );
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
        assert_eq!(
            content.len(),
            2,
            "reparse yields bullet + task lists: {reparsed}"
        );
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
        assert_eq!(
            content.len(),
            2,
            "reparse yields bullet + task lists: {reparsed}"
        );
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
    fn test_end_list_separator_not_confused_by_raw_html_blocks() {
        // Codex review on PR #3258 (P2): raw HTML blocks containing
        // backtick-only lines must not be read as code fences. The
        // first div's ``` line used to open a phantom fence that hid
        // the real separator, the second div's ``` closed it, and the
        // authored <!-- end list --> inside the second div was then
        // counted (and dropped) as the renderer separator.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "code_block", "attrs": {"params": "html", "htmlBlock": true}, "content": [{"type": "text", "text": "<div>\n```\n</div>"}]},
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "ordered_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]},
                {"type": "code_block", "attrs": {"params": "html", "htmlBlock": true}, "content": [{"type": "text", "text": "<div>\n```\n<!-- end list -->\n</div>"}]}
            ]
        });
        let out = strip_end_list_separators(
            &doc,
            "<div>\n```\n</div>\n\n- a\n\n<!-- end list -->\n\n1. b\n\n<div>\n```\n<!-- end list -->\n</div>\n",
        );
        assert_eq!(
            out.matches("<!-- end list -->").count(),
            1,
            "only the authored HTML comment remains: {out:?}"
        );
        assert!(
            out.contains("- a\n1. b"),
            "the real boundary separator dropped: {out:?}"
        );
        assert!(
            out.contains("<div>\n```\n<!-- end list -->\n</div>"),
            "the raw HTML block survives verbatim: {out:?}"
        );
    }

    #[test]
    fn test_empty_item_blank_guard_not_applied_within_same_list_family() {
        // Codex review on PR #3258 (P2): a tight task run followed by a
        // tight bullet run beginning with an empty item renders as ONE
        // bullet-family list on reparse; the empty item is a sibling,
        // not an interruption, so no blank is kept when their separator
        // drops - keeping one would loosen the merged list and break
        // the save fixpoint.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "task_list", "attrs": {"tight": true}, "content": [
                    {"type": "task_list_item", "attrs": {"checked": false}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
                ]},
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": []}]},
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]}
            ]
        });
        let out = strip_end_list_separators(&doc, "- [ ] a\n\n<!-- end list -->\n\n- \n- b\n");
        assert_eq!(
            out, "- [ ] a\n- \n- b\n",
            "no blank kept between same-family runs: {out:?}"
        );
    }

    #[test]
    fn test_end_list_literal_inside_fence_after_list_marker_survives() {
        // Codex review on PR #3258 (P2): a list item whose first block
        // is a fenced code block renders its opener immediately after
        // the list marker (`- ``` `). The scanner missed that opener,
        // counted the fence's literal separator-looking line, mistook
        // the closing fence for an opener (hiding the real separator),
        // and the counts still matched - so the literal was deleted.
        // The fence content must survive and the real separator drop.
        let doc = json!({
            "type": "doc",
            "content": [
                {"type": "bullet_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "code_block", "content": [{"type": "text", "text": "<!-- end list -->"}]}]}
                ]},
                {"type": "ordered_list", "attrs": {"tight": true}, "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}
                ]}
            ]
        });
        let out = strip_end_list_separators(
            &doc,
            "- ```\n  <!-- end list -->\n  ```\n\n<!-- end list -->\n\n1. b\n",
        );
        assert_eq!(
            out.matches("<!-- end list -->").count(),
            1,
            "only the fence's literal line remains: {out:?}"
        );
        assert!(
            out.contains("- ```\n  <!-- end list -->\n  ```"),
            "fenced code block survives verbatim: {out:?}"
        );
        assert!(
            out.contains("```\n1. b"),
            "the real boundary separator dropped: {out:?}"
        );
    }

    #[test]
    fn test_adjacent_list_before_empty_first_item_keeps_blank() {
        // Codex review on PR #3258 (P2): `- a\n\n1.\n2. b` saved
        // without the blank line - an empty list item cannot interrupt
        // a paragraph even when its marker starts at 1, so the empty
        // `1.` reparsed as lazy continuation text and the ordered list
        // was lost. The boundary keeps exactly one blank line.
        let original = "- a\n\n1.\n2. b\n";
        let (first, second) = two_saves(original);
        // The blank line is kept; the renderer spells the empty item
        // with a trailing space (`1. `), so the saved form is not
        // byte-identical to the hand-written original.
        assert_eq!(
            first, "- a\n\n1. \n2. b\n",
            "blank kept before the empty item: {first:?}"
        );
        assert!(!first.contains("end list"), "no artefact: {first:?}");
        assert_eq!(first, second, "fixpoint");
        let doc = markdown_to_prosemirror(&first);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "two lists survive reparse: {content:?}");
        assert_eq!(content[1]["type"], "ordered_list");
        assert_eq!(
            content[1]["content"].as_array().unwrap().len(),
            2,
            "both ordered items survive"
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
        assert_eq!(
            kinds.len(),
            1,
            "only the top-level boundary is collected: {kinds:?}"
        );
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
            assert_eq!(
                content[1]["type"], "ordered_list",
                "second list stays ordered for {original:?}"
            );
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
    fn test_github_alert_markers_roundtrip_fixpoint() {
        for marker in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] {
            let original = format!("> [!{marker}]\n> First paragraph.\n>\n> Second paragraph.\n");
            let (first, second) = two_saves(&original);

            assert!(
                first.starts_with(&format!("> [!{marker}]\n")),
                "alert marker must remain unescaped: {first:?}"
            );
            assert!(first.contains("First paragraph."), "{first:?}");
            assert!(first.contains("Second paragraph."), "{first:?}");
            assert_eq!(first, second, "alert must be a save fixpoint: {marker}");
        }
    }

    #[test]
    fn test_non_alert_blockquote_markers_remain_plain() {
        for (original, escaped) in [
            (
                "> [!NOTE] extra words\n> Body.\n",
                r"\[\!NOTE\] extra words",
            ),
            ("> [see appendix]\n", r"\[see appendix\]"),
        ] {
            let (first, second) = two_saves(original);
            assert!(
                first.contains(escaped),
                "non-alert text should keep normal escaping: {first:?}"
            );
            assert_eq!(first, second, "non-alert blockquote must be stable");
        }
    }

    #[test]
    fn test_alert_marker_text_inside_code_fence_is_untouched() {
        // A literal `> \[\!NOTE\]` line inside a fenced block is code, not
        // a renderer-escaped marker: rewriting it would change the code
        // (Codex P2).
        for original in [
            "```\n> \\[\\!NOTE\\]\n```\n",
            "> ```\n> > \\[\\!NOTE\\]\n> ```\n",
            "~~~~\n> \\[\\!NOTE\\]\n~~~~\n",
        ] {
            let (first, second) = two_saves(original);
            assert!(
                first.contains(r"> \[\!NOTE\]"),
                "code fence interior must stay verbatim: {first:?}"
            );
            assert_eq!(first, second, "code fence must be a save fixpoint");
        }
    }

    #[test]
    fn test_nested_first_block_does_not_make_alert() {
        // The outer quote starts with a nested quote, so its later
        // `[!NOTE]` line is ordinary escaped text even with a hard break
        // (Codex P2): unescaping it would make the text look like an alert
        // marker on reload and lose the author's break syntax.
        let original = "> > Nested.\n>\n> \\[\\!NOTE\\]\\\n> Body.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains(r"\[\!NOTE\]"),
            "marker in a non-leading paragraph must stay escaped: {first:?}"
        );
        assert_eq!(first, second, "nested-quote text must be a save fixpoint");
    }

    #[test]
    fn test_fence_first_blockquote_marker_stays_escaped() {
        // #7563: a blockquote whose first block is a fenced code block
        // HAS content, so a standalone escaped marker line in a later
        // paragraph is authored literal text, not an alert opener: it
        // must stay escaped, hard break intact. The restore scan's
        // fence branches never marked quote_has_content, so the quote
        // still counted as empty and the marker was unescaped.
        let original = "> ```\n> code\n> ```\n>\n> \\[\\!NOTE\\]\\\n> Body.\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains(r"\[\!NOTE\]"),
            "marker after a fence-first block must stay escaped: {first:?}"
        );
        assert_eq!(first, second, "fence-first quote must be a save fixpoint");
    }

    #[test]
    fn test_html_first_blockquote_marker_stays_escaped() {
        // #7563: the HTML-block sibling of the fence-first case,
        // pinned directly on the restore scan: in the saved spelling
        // an HTML block as the quote's first block is content, so the
        // later escaped marker line (hard break intact) is authored
        // text and passes through byte-identical. (No two_saves form:
        // the renderer splits an HTML-first quote ahead of the marker
        // paragraph into a separate blockquote, where the marker
        // legitimately IS the first content.)
        let saved = "> <div>\n> html\n> </div>\n>\n> \\[\\!NOTE\\]\\\n> Body.\n";
        assert_eq!(restore_github_alert_markers(saved), saved);
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
    fn test_newline_destination_protection_skips_literal_contexts() {
        // #1707: protect_newline_destinations must leave
        // destination-shaped strings in literal contexts untouched
        // and must stop the destination scan at whitespace-only
        // blank lines.
        for literal in [
            // Inline code span.
            "`[x](<a\nb>)`\n",
            // Indented code block.
            "para\n\n    [x](<a\nb>)\n",
            // A shorter fence line inside a longer fence does not
            // end the literal block.
            "````\n```\n[x](<a\nb>)\n````\n",
            // Plain fenced block (pre-existing behaviour).
            "```\n[x](<a\nb>)\n```\n",
        ] {
            let (protected, token) = protect_newline_destinations(literal);
            assert_eq!(protected, literal, "literal context rewritten: {literal:?}");
            assert!(token.is_none(), "no token for literal context: {literal:?}");
            // End to end: no converted node may carry the token.
            let doc = markdown_to_prosemirror(literal);
            assert!(
                !doc.to_string().contains("idweb-newline"),
                "token leaked into converted doc for {literal:?}: {doc}"
            );
        }
        // A whitespace-only blank line ends the destination: the
        // shape is not a link, so nothing is protected and comrak
        // keeps it literal text (no link mark is invented).
        let broken = "[x](<a\n \nb>)\n";
        let (protected, token) = protect_newline_destinations(broken);
        assert_eq!(
            protected, broken,
            "scan crossed a whitespace-only blank line"
        );
        assert!(token.is_none());
        let doc = markdown_to_prosemirror(broken);
        assert!(
            !doc.to_string().contains("\"link\""),
            "paragraph-broken destination became a link: {doc}"
        );
        // Controls: a genuine newline destination is still
        // protected, and a genuine one after a literal block is too.
        let (protected, token) = protect_newline_destinations("[x](<a\nb>)\n");
        assert!(token.is_some(), "genuine destination unprotected");
        assert!(
            protected.contains("idweb-newline"),
            "genuine destination not tokenised: {protected:?}"
        );
        let mixed = "```\n[x](<a\nb>)\n```\n\n[y](<c\nd>)\n";
        let (protected, token) = protect_newline_destinations(mixed);
        assert!(token.is_some(), "destination after fence unprotected");
        assert!(
            protected.starts_with("```\n[x](<a\nb>)\n```\n"),
            "fenced literal rewritten in mixed doc: {protected:?}"
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

    #[test]
    fn test_unused_link_reference_definitions_survive_roundtrip() {
        let original =
            "Body text with no reference.\n\n[a]: https://a.example\n[b]: https://b.example\n";
        let doc = markdown_to_prosemirror(original);
        let recovered: Vec<&Value> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["type"] == "code_block" && node["attrs"]["htmlBlock"] == true)
            .collect();
        assert_eq!(
            recovered.len(),
            2,
            "both unused definitions are retained: {doc}"
        );

        let (first, second) = two_saves(original);
        for definition in ["[a]: https://a.example", "[b]: https://b.example"] {
            assert!(
                first.contains(definition),
                "{definition} survives: {first:?}"
            );
        }
        assert_eq!(first, second, "unused definitions round-trip to a fixpoint");
    }

    #[test]
    fn test_front_matter_definition_is_not_recovered() {
        // Codex P2 / CodeRabbit on PR #5949: a definition-shaped line
        // inside front matter is metadata; recovering it appended a
        // duplicate outside the block on every save.
        let original = "---\n[docs]: https://example.com\n---\n\nBody\n";
        let (first, second) = two_saves(original);
        assert_eq!(
            first.matches("[docs]:").count(),
            1,
            "front-matter definition is not duplicated: {first:?}"
        );
        assert_eq!(first, second, "front-matter round-trip is a fixpoint");
    }

    #[test]
    fn test_dots_line_does_not_close_front_matter_for_definition_collection() {
        // Codex P2 on PR #5949 (fresh round): comrak's front-matter
        // parser requires the configured `---` closing delimiter; a
        // `...` line does not close the block, so the opening lines
        // are ordinary Markdown and `[url]: /target` is a real
        // definition the orphan footnote's link resolves through.
        let original = "---\n[url]: /target\n...\n\n[^n]: [label][url]\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[label](/target)"),
            "footnote link resolves through the definition: {first:?}"
        );
        assert_eq!(first, second, "dots-delimiter round-trip is a fixpoint");
    }

    #[test]
    fn test_quoted_multiline_definition_label_matches_without_container() {
        // Codex P2 on PR #5949 (fresh round): the quoted definition's
        // label spans two lines; stripping only the first line's
        // container left the continuation's `>` in the matching key,
        // so the unused definition was keyed as the used label
        // `long > label` and silently dropped.
        let original =
            "[long > label]\n\n[long > label]: /used\n\n> [long\n> label]: /keep\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("/keep"),
            "unused quoted multiline definition is preserved: {first:?}"
        );
        assert!(
            !first.contains("[long > label]: /used"),
            "consumed definition is not appended: {first:?}"
        );
        assert_eq!(first, second, "quoted-multiline round-trip is a fixpoint");
    }

    #[test]
    fn test_code_span_bracket_does_not_end_link_text() {
        // Codex P2 on PR #5949 (fresh round): in `[x`]...` the first
        // closing bracket belongs to an inline code span; the link's
        // explicit label is `used`, and keying the use as `x\`` made
        // the genuinely unused `[x`]` definition look consumed.
        let original = "[x`]\u{60}][used]\n\n[used]: /used-target\n[x`]: /keep\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("(/used-target)"),
            "explicit reference resolves: {first:?}"
        );
        assert!(
            first.contains("[x`]: /keep"),
            "genuinely unused definition is preserved: {first:?}"
        );
        assert!(
            !first.contains("[used]: /used-target"),
            "consumed definition is not appended: {first:?}"
        );
        assert_eq!(first, second, "code-span-bracket round-trip is a fixpoint");
    }

    #[test]
    fn test_quoted_multiline_reference_use_ignores_containers() {
        // Codex P2 on PR #5949 (round 4): the quoted shortcut link's
        // label spans two lines; joining the raw source lines kept
        // the continuation's `>` in the used key, so the use was
        // keyed `long > label` and the genuinely unused definition
        // with that literal label was discarded as consumed.
        let original = "> [long\n> label]\n\n[long label]: /used\n[long > label]: /keep\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[long > label]: /keep"),
            "genuinely unused definition is preserved: {first:?}"
        );
        assert!(
            !first.contains("[long label]: /used"),
            "consumed definition is not appended: {first:?}"
        );
        assert_eq!(first, second, "quoted-multiline-use round-trip is a fixpoint");
    }

    #[test]
    fn test_inline_html_bracket_does_not_end_link_text() {
        // Codex P2 on PR #5949 (round 4): the `]` inside an inline
        // HTML attribute is not the link-text boundary; the link's
        // explicit label is `used`, and keying the use as the HTML
        // fragment made the genuinely unused definition look
        // consumed.
        let original =
            "[<i x=\"]\">label</i>][used]\n\n[used]: /used\n[<i x=\"]: /keep\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("(/used)"),
            "explicit reference resolves: {first:?}"
        );
        assert!(
            first.contains("/keep"),
            "genuinely unused definition is preserved: {first:?}"
        );
        assert!(
            !first.contains("[used]: /used"),
            "consumed definition is not appended: {first:?}"
        );
        assert_eq!(first, second, "inline-html-bracket round-trip is a fixpoint");
    }

    #[test]
    fn test_link_use_detection_scoped_to_node_columns() {
        // Codex P2 / CodeRabbit on PR #5949: the literal code span
        // sharing the line must not decide the real link's label.
        let original = "`[unused]` [used]\n\n[unused]: /keep\n[used]: /used-target\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[used](/used-target)"),
            "real shortcut reference resolves: {first:?}"
        );
        assert!(
            !first.contains("[used]: /used-target"),
            "consumed definition is not appended: {first:?}"
        );
        assert!(
            first.contains("[unused]: /keep"),
            "genuinely unused definition is preserved: {first:?}"
        );
        assert_eq!(first, second, "column-scoped round-trip is a fixpoint");
    }

    #[test]
    fn test_multiline_title_literal_marker_survives() {
        // Codex P2 on PR #5949: a multiline title's continuation line
        // is title text, not a list container.
        let original = "Body\n\n[r]: /target \"first\n2. second\"\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("\"first\n2. second\""),
            "multiline title survives verbatim: {first:?}"
        );
        assert_eq!(first, second, "multiline-title round-trip is a fixpoint");
    }

    #[test]
    fn test_autolink_brackets_are_not_a_reference_use() {
        // Codex P2 on PR #5949: the bracket inside an autolink URL
        // must not mark the reference definition as used.
        let original = "<https://example.com/[r]>\n\n[r]: /keep\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[r]: /keep"),
            "unused definition beside an autolink is preserved: {first:?}"
        );
        assert_eq!(first, second, "autolink round-trip is a fixpoint");
    }

    #[test]
    fn test_escaped_label_definition_stays_unused() {
        // Codex P2 on PR #5949 (CommonMark example 545): `[a!]`
        // consumes only the `[a!]` definition; the escaped-label
        // definition `[a\!]` is a different label and is unused,
        // so it must be preserved, not dropped as consumed.
        let original = "[a!]\n\n[a\\!]: /keep\n[a!]: /used-target\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[a\\!](/used-target)"),
            "unescaped reference resolves to its own definition: {first:?}"
        );
        assert!(
            first.contains("[a\\!]: /keep"),
            "escaped-label definition is preserved as unused: {first:?}"
        );
        assert_eq!(first, second, "escaped-label round-trip is a fixpoint");
    }

    #[test]
    fn test_backslash_label_stays_distinct() {
        // CodeRabbit on PR #5949: `[a\\b]` and `[ab]` are different
        // labels; using one must not consume the other's definition.
        let original = "[ab]\n\n[a\\b]: /keep\n[ab]: /used-target\n";
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[ab](/used-target)"),
            "used label resolves: {first:?}"
        );
        assert!(
            first.contains("[a\\b]: /keep"),
            "backslash label definition is preserved: {first:?}"
        );
        assert_eq!(first, second, "backslash-label round-trip is a fixpoint");
    }

    #[test]
    fn test_used_link_reference_definitions_keep_inline_link_behavior() {
        for (original, expected_link) in [
            (
                "[one][r]\n\n[r]: https://r.example\n",
                "[one](https://r.example)",
            ),
            (
                "[text][]\n\n[text]: https://r.example\n",
                "[text](https://r.example)",
            ),
            ("[r]\n\n[r]: https://r.example\n", "[r](https://r.example)"),
            (
                "![alt][image]\n\n[image]: https://r.example/image.png\n",
                "![alt](https://r.example/image.png)",
            ),
        ] {
            let (first, second) = two_saves(original);
            assert!(
                first.contains(expected_link),
                "resolved link preserved: {first:?}"
            );
            assert!(
                !first.contains("]: https://r.example"),
                "used definition consumed: {first:?}"
            );
            assert_eq!(first, second, "used-reference round-trip is a fixpoint");
        }
    }

    #[test]
    fn test_link_reference_definitions_only_document_survives_roundtrip() {
        let original = "[a]: https://a.example\n[b]: https://b.example\n";
        let (first, second) = two_saves(original);
        for definition in ["[a]: https://a.example", "[b]: https://b.example"] {
            assert!(
                first.contains(definition),
                "definitions-only document preserves {definition}: {first:?}"
            );
        }
        assert!(
            !first.trim().is_empty(),
            "definitions-only document does not save empty: {first:?}"
        );
        assert_eq!(first, second, "definitions-only round-trip is a fixpoint");
    }

    #[test]
    fn test_link_reference_definition_recovery_respects_literal_contexts() {
        let original = "```\n[fenced]: https://fenced.example\n```\n\n    [indented]: https://indented.example\n\n<!--\n[html]: https://html.example\n-->\n\n> [quoted]: https://quoted.example\n";
        let doc = markdown_to_prosemirror(original);
        let recovered: Vec<&Value> = doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| {
                node["type"] == "code_block"
                    && node["attrs"]["htmlBlock"] == true
                    && node["content"][0]["text"] == "[quoted]: https://quoted.example"
            })
            .collect();
        assert_eq!(
            recovered.len(),
            1,
            "only the real blockquote definition is recovered: {doc}"
        );
        let (first, second) = two_saves(original);
        assert!(
            first.contains("[quoted]: https://quoted.example"),
            "real quoted definition survives after hoisting to document scope: {first:?}"
        );
        assert!(
            first.contains("```\n[fenced]: https://fenced.example"),
            "fenced sample remains code only: {first:?}"
        );
        assert!(
            first.contains("```\n[indented]: https://indented.example"),
            "indented sample remains code only: {first:?}"
        );
        assert!(
            first.contains("<!--\n[html]: https://html.example"),
            "HTML sample remains literal HTML: {first:?}"
        );
        assert_eq!(first, second, "literal-context round-trip is a fixpoint");
    }

    #[test]
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
    fn test_roundtrip_info_less_code_block_stays_fenced() {
        // #954: info-less fenced blocks must save as fenced, not as
        // 4-space indented blocks. Covers blank-line content,
        // fence-containing content, list nesting, and blockquotes
        // (the blockquote shape is also pinned by #1496).
        for (original, opening_fence) in [
            ("```\nline1\n\n\nline2\n```\n", "```"),
            ("````\n```\ncode\n```\n````\n", "````"),
            ("- ```\n  code\n  ```\n", "- ```"),
            // Blockquote saves with leading empty quote lines (comrak's
            // block separation); the shape pinned here is fenced, not
            // indented.
            ("> ```\n> code\n> ```\n", "> ```"),
        ] {
            let (first, second) = two_saves(original);
            assert!(
                first.lines().any(|l| l == opening_fence),
                "info-less block lost its fence for {original:?}: {first:?}"
            );
            assert!(
                !first.lines().any(|l| l.starts_with("    ")),
                "info-less block saved indented for {original:?}: {first:?}"
            );
            assert_eq!(
                markdown_to_prosemirror(&first),
                markdown_to_prosemirror(original),
                "round trip changed the doc for {original:?}"
            );
            assert_eq!(first, second, "no fixpoint for {original:?}");
        }
    }

    #[test]
    fn test_roundtrip_empty_code_block_has_no_spurious_blank_line() {
        // #954: an empty fenced block must not gain a blank line.
        let (first, second) = two_saves("```\n```\n");
        assert_eq!(first, "```\n```\n", "empty block grew: {first:?}");
        assert_eq!(first, second, "no fixpoint for empty block");
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
    fn test_ordered_list_paren_delimiter_roundtrips() {
        // #1380: the ordered-list delimiter style (`1)` vs `1.`) must
        // survive the codec in both directions instead of every list
        // being rewritten with periods.
        let doc = markdown_to_prosemirror("1) one\n2) two\n");
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "ordered_list");
        assert_eq!(content[0]["attrs"]["delimiter"], "paren");

        let (first, second) = two_saves("1) one\n2) two\n");
        assert_eq!(first, "1) one\n2) two\n", "paren round-trip changed bytes");
        assert_eq!(first, second, "no fixpoint for paren list");

        // Period lists are unchanged and now carry the attr explicitly.
        let doc = markdown_to_prosemirror("1. one\n2. two\n");
        assert_eq!(doc["content"][0]["attrs"]["delimiter"], "period");
        let (first, second) = two_saves("1. one\n2. two\n");
        assert_eq!(first, "1. one\n2. two\n");
        assert_eq!(first, second, "no fixpoint for period list");
    }

    #[test]
    fn test_ordered_task_list_paren_delimiter_roundtrips() {
        // #1380: ordered task lists keep their delimiter too.
        let doc = markdown_to_prosemirror("1) [ ] todo\n2) [x] done\n");
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "task_list");
        assert_eq!(content[0]["attrs"]["order"], 1);
        assert_eq!(content[0]["attrs"]["delimiter"], "paren");

        let (first, second) = two_saves("1) [ ] todo\n2) [x] done\n");
        assert_eq!(first, "1) [ ] todo\n2) [x] done\n");
        assert_eq!(first, second, "no fixpoint for paren task list");
    }

    #[test]
    fn test_legacy_ordered_list_without_delimiter_attr_saves_period() {
        // #1380: legacy PM docs stored by the old codec carry no
        // delimiter attr; they must save exactly as today (period).
        let doc = json!({
            "type": "doc",
            "content": [{
                "type": "ordered_list",
                "attrs": {"order": 1, "tight": true},
                "content": [
                    {"type": "list_item", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "one"}]}
                    ]},
                    {"type": "list_item", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "two"}]}
                    ]}
                ]
            }]
        });
        let md = prosemirror_to_markdown(&doc).unwrap();
        assert_eq!(md, "1. one\n2. two\n", "legacy doc changed: {md:?}");
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
    fn test_table_cell_paragraphs_keep_word_boundaries() {
        let para =
            |text: &str| json!({"type": "paragraph", "content": [{"type": "text", "text": text}]});
        let pm = json!({"type": "doc", "content": [{"type": "table", "content": [
            {"type": "table_row", "content": [{"type": "table_header", "content": [para("head"), para("tail")]}]},
            {"type": "table_row", "content": [{"type": "table_cell", "content": [para("first"), para("second"), para("third")]}]},
            {"type": "table_row", "content": [{"type": "table_cell", "content": [para("single")]}]}
        ]}]});
        let md = prosemirror_to_markdown(&pm).unwrap();
        assert_eq!(
            md,
            "| head tail |\n| --- |\n| first second third |\n| single |\n"
        );
        let reparsed = markdown_to_prosemirror(&md);
        assert_eq!(prosemirror_to_markdown(&reparsed).unwrap(), md);
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
    fn test_uppercase_img_tag_imports_as_image() {
        let doc =
            markdown_to_prosemirror("<IMG SRC=\"/blob/AbC.png\" ALT=\"MiXeD\" WIDTH=\"200\" />\n");
        let img = &doc["content"][0]["content"][0];
        assert_eq!(img["type"], "image");
        assert_eq!(img["attrs"]["src"], "/blob/AbC.png");
        assert_eq!(img["attrs"]["alt"], "MiXeD");
        assert_eq!(img["attrs"]["width"], 200);
    }

    #[test]
    fn test_mixed_case_inline_img_tag_imports_as_image() {
        let doc =
            markdown_to_prosemirror("before <Img src=\"inline.png\" alt=\"Mixed\" /> after\n");
        let content = doc["content"][0]["content"]
            .as_array()
            .expect("paragraph content");
        let img = content
            .iter()
            .find(|node| node["type"] == "image")
            .expect("mixed-case inline image node");
        assert_eq!(img["attrs"]["src"], "inline.png");
        assert_eq!(img["attrs"]["alt"], "Mixed");
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
    fn test_img_tag_accepts_ascii_whitespace_after_name() {
        for separator in ["\n", "\t", "\r", "\x0c"] {
            let html = format!("<img{separator}src=\"x.png\" width=\"240\" alt=\"A\" />");
            let node = parse_img_tag(&html).expect("img with ASCII whitespace parses");
            assert_eq!(node["attrs"]["src"], "x.png", "separator {separator:?}");
            assert_eq!(node["attrs"]["width"], 240, "separator {separator:?}");
            assert_eq!(node["attrs"]["alt"], "A", "separator {separator:?}");
        }
    }

    #[test]
    fn test_linked_img_tag_accepts_newline_after_img_name() {
        let node = parse_linked_img_tag(
            "<a href=\"https://example.com\"><img\nsrc=\"x.png\" width=\"240\" /></a>",
        )
        .expect("linked img with newline parses");
        assert_eq!(node["attrs"]["src"], "x.png");
        assert_eq!(node["attrs"]["width"], 240);
        assert_eq!(node["attrs"]["linkHref"], "https://example.com");
    }

    #[test]
    fn test_img_tag_name_boundary_preserves_existing_forms() {
        let spaced = parse_img_tag("<img src=\"x.png\" />").expect("space form parses");
        assert_eq!(spaced["attrs"]["src"], "x.png");
        assert!(
            parse_img_tag("<img>").is_none(),
            "bare tag still has no required src"
        );
        assert!(parse_img_tag("<imgx src=\"x.png\" />").is_none());
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

    #[test]
    fn test_uppercase_linked_img_tags_import_with_link() {
        let node = parse_linked_img_tag(
            "<A HREF=\"https://example.com/Case\" TITLE=\"Link\"><IMG SRC=\"Img.png\" ALT=\"Pic\" /></a>",
        )
        .expect("case-insensitive linked image parses");
        assert_eq!(node["attrs"]["src"], "Img.png");
        assert_eq!(node["attrs"]["alt"], "Pic");
        assert_eq!(node["attrs"]["linkHref"], "https://example.com/Case");
        assert_eq!(node["attrs"]["linkTitle"], "Link");

        let doc = markdown_to_prosemirror(
            "before <A HREF=\"https://example.com/Case\"><IMG SRC=\"Img.png\" ALT=\"Pic\" /></A> after\n",
        );
        let content = doc["content"][0]["content"]
            .as_array()
            .expect("paragraph content");
        let img = content
            .iter()
            .find(|node| node["type"] == "image")
            .expect("linked image node");
        assert_eq!(img["attrs"]["linkHref"], "https://example.com/Case");
        assert!(
            content.iter().all(|node| node["type"] != "html_inline"),
            "closing anchor is consumed with the image: {doc}"
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
    fn test_heading_mid_hard_break_degrades_to_space() {
        // #1388: a hard_break inside a heading cannot be spelled in
        // CommonMark — the save emitted a backslash-newline inside the
        // ATX heading, re-parse split it into heading + paragraph with
        // a literal backslash, and the backslash doubled on the next
        // save. Mid/start breaks degrade to a single space (headings
        // are single-line content, mirroring the #1017 cell policy).
        let doc = json!({"type": "doc", "content": [{"type": "heading", "attrs": {"level": 1}, "content": [
            {"type": "text", "text": "Title"},
            {"type": "hard_break"},
            {"type": "text", "text": "more here"}
        ]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "# Title more here\n", "break became a space: {md:?}");
        let reopened = markdown_to_prosemirror(&md);
        let blocks = reopened["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "still one block: {reopened:?}");
        assert_eq!(
            blocks[0]["type"], "heading",
            "still a heading: {reopened:?}"
        );
        assert_eq!(
            prosemirror_to_markdown(&reopened).expect("reconverts"),
            md,
            "heading output is a fixpoint"
        );
    }

    #[test]
    fn test_setext_heading_hard_break_roundtrips() {
        // #1388: the creator probe — a setext heading whose lines are
        // joined by a backslash break (and the two-trailing-spaces
        // variant) parses to heading [text, hard_break, text] and must
        // save back to one clean heading, not split with a backslash.
        for source in ["Title\\\nmore here\n=====\n", "Title  \nmore here\n=====\n"] {
            let doc = markdown_to_prosemirror(source);
            let blocks = doc["content"].as_array().unwrap();
            assert_eq!(
                blocks.len(),
                1,
                "parses to one block: {source:?} -> {doc:?}"
            );
            assert_eq!(blocks[0]["type"], "heading", "{source:?} -> {doc:?}");
            let md = prosemirror_to_markdown(&doc).expect("converts");
            assert_eq!(
                md, "# Title more here\n",
                "no split, no backslash: {source:?} -> {md:?}"
            );
            let reopened = markdown_to_prosemirror(&md);
            assert_eq!(
                prosemirror_to_markdown(&reopened).expect("reconverts"),
                md,
                "setext heading output is a fixpoint: {source:?}"
            );
        }
    }

    #[test]
    fn test_heading_edge_hard_breaks_normalize() {
        // #1388: a heading-start break degrades to a space (erased by
        // the #1420 line-edge trim, so no leading gap survives) and a
        // heading-trailing break is dropped — both save clean and are
        // fixpoints.
        for (name, content) in [
            (
                "start",
                vec![
                    json!({"type": "hard_break"}),
                    json!({"type": "text", "text": "Title"}),
                ],
            ),
            (
                "trailing",
                vec![
                    json!({"type": "text", "text": "Title"}),
                    json!({"type": "hard_break"}),
                ],
            ),
        ] {
            let doc = json!({"type": "doc", "content": [
                {"type": "heading", "attrs": {"level": 2}, "content": content}
            ]});
            let md = prosemirror_to_markdown(&doc).expect("converts");
            assert_eq!(md, "## Title\n", "{name} break normalized: {md:?}");
            let reopened = markdown_to_prosemirror(&md);
            assert_eq!(
                prosemirror_to_markdown(&reopened).expect("reconverts"),
                md,
                "{name} heading output is a fixpoint"
            );
        }
    }

    #[test]
    fn test_paragraph_trailing_hard_break_dropped_explicitly() {
        // #1388 with the #1492 placements transferred in: a trailing
        // hard_break at block end (paragraph, blockquote paragraph,
        // list-item paragraph) used to vanish silently in the
        // renderer; the save now drops it explicitly so the first
        // saved file is already the fixpoint.
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "foo"},
                {"type": "hard_break"}
            ]},
            {"type": "blockquote", "content": [
                {"type": "paragraph", "content": [
                    {"type": "text", "text": "quoted"},
                    {"type": "hard_break"}
                ]}
            ]},
            {"type": "bullet_list", "content": [
                {"type": "list_item", "content": [
                    {"type": "paragraph", "content": [
                        {"type": "text", "text": "item"},
                        {"type": "hard_break"}
                    ]}
                ]}
            ]}
        ]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(
            md, "foo\n\n> quoted\n\n- item\n",
            "trailing breaks dropped: {md:?}"
        );
        let reopened = markdown_to_prosemirror(&md);
        assert_eq!(
            prosemirror_to_markdown(&reopened).expect("reconverts"),
            md,
            "trailing-break output is a fixpoint"
        );
    }

    #[test]
    fn test_break_only_paragraph_saves_as_empty_paragraph() {
        // #1388 with the #1492 placement transferred in: a paragraph
        // whose only content is a hard_break used to be deleted
        // outright on save. Like a whitespace-only paragraph (#1420),
        // it now saves as an empty paragraph.
        let doc = json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "hard_break"}
        ]}]});
        let md = prosemirror_to_markdown(&doc).expect("converts");
        assert_eq!(md, "", "break-only paragraph saves empty: {md:?}");
        assert_eq!(
            markdown_to_prosemirror(&md),
            json!({"type": "doc", "content": [{"type": "paragraph"}]}),
            "reopens as an empty paragraph"
        );
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

    #[test]
    fn test_front_matter_roundtrip_fixpoint() {
        // #1369: YAML front matter (`---` delimiter only; TOML `+++`
        // is out of scope) survives open->save verbatim with a
        // multi-key block, and the second save is a fixpoint.
        for original in [
            "---\ntitle: Hello\ntags: [a, b]\n---\n\nBody text.\n",
            "---\ntitle: Hello\n---\n\n# Heading\n",
            "---\ntitle: Hello\n---\n\nJust a paragraph.\n",
            // Blank line inside the block is preserved verbatim.
            "---\ntitle: Hello\n\ndraft: true\n---\n\nBody.\n",
        ] {
            let doc = markdown_to_prosemirror(original);
            let content = doc["content"].as_array().unwrap();
            assert_eq!(
                content[0]["type"], "front_matter",
                "first child for {original:?}: {doc}"
            );
            let (first, second) = two_saves(original);
            assert_eq!(first, original, "round-trip for {original:?}");
            assert_eq!(first, second, "fixpoint for {original:?}");
            assert_eq!(
                markdown_to_prosemirror(&first),
                doc,
                "stable PM for {original:?}"
            );
        }
    }

    #[test]
    fn test_front_matter_mid_doc_dashes_stay_horizontal_rule() {
        // #1369 control: `---` after a paragraph is a thematic break,
        // not front matter.
        let original = "Para one.\n\n---\n\nPara two.\n";
        let doc = markdown_to_prosemirror(original);
        let content = doc["content"].as_array().unwrap();
        assert!(
            content.iter().all(|n| n["type"] != "front_matter"),
            "no front_matter mid-doc: {doc}"
        );
        assert!(
            content.iter().any(|n| n["type"] == "horizontal_rule"),
            "thematic break kept: {doc}"
        );
        let (first, second) = two_saves(original);
        // Thematic breaks normalize to comrak's `-----` spelling; what
        // matters is the node type survives and the save is a fixpoint.
        assert!(first.contains("-----"), "break kept: {first:?}");
        assert_eq!(first, second, "fixpoint: {first:?}");
    }

    #[test]
    fn test_front_matter_unclosed_leading_dashes_stay_break() {
        // #1369 control: an unclosed leading `---` is a thematic break.
        let original = "---\n\nBody.\n";
        let doc = markdown_to_prosemirror(original);
        let content = doc["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "horizontal_rule", "no FM: {doc}");
        let (first, second) = two_saves(original);
        assert!(first.contains("-----"), "break kept: {first:?}");
        assert_eq!(first, second, "fixpoint: {first:?}");
    }

    #[test]
    fn test_front_matter_misplaced_is_rejected() {
        // #1369: `front_matter` is legal only as the first child of
        // `doc`; anywhere else is a MisplacedNode.
        let fm = || json!({"type": "front_matter", "attrs": {"text": "---\na: b\n---\n"}});
        let para = || json!({"type": "paragraph", "content": [{"type": "text", "text": "x"}]});
        let cases: &[(&str, Value)] = &[
            (
                "second child of doc",
                json!({"type": "doc", "content": [para(), fm()]}),
            ),
            (
                "inside blockquote",
                json!({"type": "doc", "content": [{"type": "blockquote", "content": [fm()]}]}),
            ),
            (
                "inside paragraph",
                json!({"type": "doc", "content": [{"type": "paragraph", "content": [fm()]}]}),
            ),
            (
                "content children inside front_matter",
                json!({"type": "doc", "content": [{"type": "front_matter", "attrs": {"text": "---\na: b\n---\n"}, "content": [para()]}]}),
            ),
        ];
        for (name, doc) in cases {
            assert!(
                matches!(
                    prosemirror_to_markdown(doc),
                    Err(ConversionError::MisplacedNode { .. })
                ),
                "{name}: expected MisplacedNode, got {:?}",
                prosemirror_to_markdown(doc)
            );
        }
        // First child of doc saves verbatim.
        let ok = json!({"type": "doc", "content": [fm(), para()]});
        assert_eq!(prosemirror_to_markdown(&ok).unwrap(), "---\na: b\n---\nx\n");
    }
}
