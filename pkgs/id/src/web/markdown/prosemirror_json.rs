//! ProseMirror JSON construction from a CommonMark AST (`convert_node`
//! and its inline/list helpers, placement validation). Split out of
//! `markdown.rs` for #4704.

use super::*;

/// Active marks being applied to inline content.
#[derive(Clone)]
pub(crate) struct Mark {
    pub(crate) mark_type: &'static str,
    pub(crate) attrs: Option<Value>,
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
pub(crate) fn mark_nesting_rank(mark_type: &str) -> u8 {
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
pub(crate) fn convert_node<'a>(node: &'a AstNode<'a>, active_marks: &[Mark]) -> Value {
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

        NodeValue::FrontMatter(text) => {
            json!({
                "type": "front_matter",
                "attrs": {
                    "text": text,
                }
            })
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
            // #5745: neutralize executable/unknown URL schemes at this
            // single entry point — every Markdown link (typed, imported,
            // or collaborative, which reaches this converter via
            // collab.rs) carries the sanitized href from here on.
            let mut marks = active_marks.to_vec();
            let mut attrs = json!({"href": sanitize_link_url(&link.url)});
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
                "src": sanitize_image_url(&image.url)
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
pub(crate) fn text_node_with_marks(text: &str, marks: &[Mark]) -> Value {
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
pub(crate) fn collect_block_children<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
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
pub(crate) fn is_end_list_separator<'a>(node: &'a AstNode<'a>) -> bool {
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
/// #1380: the PM `delimiter` attr spelling for a comrak ordered-list
/// delimiter (`.` vs `)`). The editor schema mirrors these spellings.
pub(crate) fn delimiter_attr(delimiter: ListDelimType) -> &'static str {
    match delimiter {
        ListDelimType::Period => "period",
        ListDelimType::Paren => "paren",
    }
}

/// #1380: read the PM `delimiter` attr back to a comrak delimiter.
/// Absent or unknown (legacy PM docs stored by the old codec, which
/// dropped the delimiter) defaults to Period - exactly how the old
/// codec saved every ordered list.
pub(crate) fn delimiter_from_attrs(json: &Value) -> ListDelimType {
    match json["attrs"]["delimiter"].as_str() {
        Some("paren") => ListDelimType::Paren,
        _ => ListDelimType::Period,
    }
}

pub(crate) fn convert_list_runs<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    let (list_type, start, tight, delimiter) = match &node.data.borrow().value {
        NodeValue::List(list) => (list.list_type, list.start, list.tight, list.delimiter),
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
                "attrs": {"order": start + offset, "tight": tight, "delimiter": delimiter_attr(delimiter)},
                "content": content
            }),
            (true, ListType::Bullet) => json!({
                "type": "task_list",
                "attrs": {"tight": tight},
                "content": content
            }),
            (false, ListType::Ordered) => json!({
                "type": "ordered_list",
                "attrs": {"order": start + offset, "tight": tight, "delimiter": delimiter_attr(delimiter)},
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
#[allow(dead_code)]
pub(crate) fn collect_list_items<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    node.children()
        .map(|child| convert_node(child, &[]))
        .filter(|v| !v.is_null())
        .collect()
}

/// The literal of an `HtmlInline` node, if `node` is one.
pub(crate) fn html_inline_literal<'a>(node: &'a AstNode<'a>) -> Option<String> {
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
pub(crate) fn linked_image_sequence<'a>(children: &[&'a AstNode<'a>]) -> Option<(Value, usize)> {
    if children.len() < 2 {
        return None;
    }
    let (href, link_title) = parse_anchor_tag(&html_inline_literal(children[0])?)?;
    let img = parse_img_tag(&html_inline_literal(children[1])?)?;
    let consumed = if children.len() >= 3
        && html_inline_literal(children[2]).is_some_and(|h| h.trim().eq_ignore_ascii_case("</a>"))
    {
        3
    } else {
        2
    };
    Some((image_with_link(img, &href, link_title.as_deref()), consumed))
}

/// Collect inline children of a node, flattening nested mark wrappers.
pub(crate) fn collect_inline_children<'a>(
    node: &'a AstNode<'a>,
    active_marks: &[Mark],
) -> Vec<Value> {
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
pub(crate) fn get_text_content<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    collect_text(node, &mut text);
    text
}

/// Recursively collect text from a node.
pub(crate) fn collect_text<'a>(node: &'a AstNode<'a>, text: &mut String) {
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
pub(crate) fn get_alt_markdown<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    for child in node.children() {
        collect_alt_markdown(child, &mut out);
    }
    out
}

pub(crate) fn collect_alt_markdown<'a>(node: &'a AstNode<'a>, out: &mut String) {
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
pub(crate) fn alt_markdown_to_nodes<'a>(arena: &'a Arena<'a>, alt: &str) -> Vec<&'a AstNode<'a>> {
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
pub(crate) fn escape_attr_value(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Decode the entities [`escape_attr_value`] can emit (plus `&gt;`/`&apos;`
/// and numeric single-quote) in one left-to-right pass, so `&amp;quot;`
/// decodes to the literal text `&quot;`, never to `"`.
pub(crate) fn decode_attr_entities(value: &str) -> String {
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
pub(crate) fn parse_tag_attrs(
    html: &str,
    tag_len: usize,
) -> std::collections::HashMap<String, String> {
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
pub(crate) fn parse_integral_decimal(raw: &str) -> Option<u64> {
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

pub(crate) fn parse_dimension(raw: &str) -> Option<u64> {
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
pub(crate) fn find_ascii_case_insensitive(haystack: &str, needle: &[u8]) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|candidate| candidate.eq_ignore_ascii_case(needle))
}

pub(crate) fn parse_img_tag(html: &str) -> Option<Value> {
    let trimmed = html.trim();
    let bytes = trimmed.as_bytes();
    if !bytes
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"<img"))
        || !matches!(
            bytes.get(4).copied(),
            Some(after_name) if after_name.is_ascii_whitespace() || after_name == b'>'
        )
    {
        return None;
    }

    let parsed = parse_tag_attrs(trimmed, 4);
    let attr = |name: &str| parsed.get(name).map(String::as_str);

    let src = sanitize_image_url(attr("src")?); // src is required
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

/// #5745: the allowed-scheme oracle for link URLs.
///
/// A link URL is safe iff, after removing the characters browsers strip
/// while parsing a URL (TAB/LF/CR anywhere, leading C0 controls and
/// spaces), it either has no scheme at all — a relative or internal
/// reference, a fragment, a query, or a protocol-relative `//host/path`
/// reference, none of which can name an executable scheme — or its
/// scheme is exactly `http`, `https`, or `mailto` (case-insensitive).
/// Everything else (`javascript:`, `data:`, `vbscript:`, any unknown
/// scheme, and any obfuscation of those the stripping above exposes)
/// is unsafe. This explicit allowed set is the oracle every link test
/// for #5745 checks against; do not rely on browser sanitization.
pub(crate) fn link_url_is_safe(url: &str) -> bool {
    let probe: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let probe = probe.trim_start_matches(|c: char| c.is_ascii_control() || c == ' ');
    // A scheme exists only when a `:` precedes any `/`, `?`, or `#`.
    let mut scheme_end = None;
    for (i, c) in probe.char_indices() {
        match c {
            ':' => {
                scheme_end = Some(i);
                break;
            }
            '/' | '?' | '#' => break,
            _ => {}
        }
    }
    let Some(colon) = scheme_end else {
        return true; // no scheme: relative / fragment / query / protocol-relative
    };
    let scheme = &probe[..colon];
    let mut chars = scheme.chars();
    let valid_scheme = !scheme.is_empty()
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        // A `:` in the first segment whose prefix is not a scheme
        // (e.g. a relative path containing a colon) is not a scheme.
        return true;
    }
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto"
    )
}

/// #5745: return `url` unchanged when [`link_url_is_safe`], else the
/// inert fragment `"#"`. The link text and mark survive; only the
/// executable target is removed, and `"#"` re-parses as a safe
/// fragment, so sanitized documents are a stable round-trip fixpoint.
pub(crate) fn sanitize_link_url(url: &str) -> String {
    if link_url_is_safe(url) {
        url.to_owned()
    } else {
        "#".to_owned()
    }
}

/// #6364: the image-specific allowed-scheme oracle.
///
/// Image URLs allow relative/internal references and `http`/`https`, plus
/// data URLs whose media type is one of the supported raster formats. SVG is
/// deliberately excluded because it can contain active content. As with
/// links, inspect the scheme after stripping TAB/LF/CR and leading C0
/// controls/spaces so browser URL parsing cannot expose an obfuscated scheme.
pub(crate) fn image_url_is_safe(url: &str) -> bool {
    let probe: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let probe = probe.trim_start_matches(|c: char| c.is_ascii_control() || c == ' ');

    // A scheme exists only when a `:` precedes any `/`, `?`, or `#`.
    let mut scheme_end = None;
    for (i, c) in probe.char_indices() {
        match c {
            ':' => {
                scheme_end = Some(i);
                break;
            }
            '/' | '?' | '#' => break,
            _ => {}
        }
    }
    let Some(colon) = scheme_end else {
        return true;
    };

    let scheme = &probe[..colon];
    let mut chars = scheme.chars();
    let valid_scheme = !scheme.is_empty()
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid_scheme {
        // Match the link oracle: a colon in a segment that is not a valid
        // scheme (for example a relative path) does not introduce a scheme.
        return true;
    }

    let Some((raw_scheme, _)) = url.split_once(':') else {
        return true;
    };
    if raw_scheme.chars().any(|c| matches!(c, '\t' | '\n' | '\r')) {
        return false;
    }

    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" => true,
        "data" => {
            // Reject control-character spellings in the scheme or data URL
            // header, even if removing them would produce an allowed MIME
            // type. Controls in the encoded payload do not affect the scheme
            // or media type and are left untouched.
            let Some((header, _)) = url.split_once(',') else {
                return false;
            };
            if header.chars().any(|c| matches!(c, '\t' | '\n' | '\r')) {
                return false;
            }
            let Some((metadata, _)) = probe[colon + 1..].split_once(',') else {
                return false;
            };
            let media_type = metadata.split(';').next().unwrap_or_default();
            matches!(
                media_type.to_ascii_lowercase().as_str(),
                "image/png"
                    | "image/gif"
                    | "image/jpeg"
                    | "image/webp"
                    | "image/avif"
                    | "image/bmp"
            )
        }
        _ => false,
    }
}

/// #6364: return `url` unchanged for an allowed image URL, otherwise the
/// inert fragment `#`. Keeping only the target neutralized preserves the
/// image node and makes the sanitized document a stable round-trip fixpoint.
pub(crate) fn sanitize_image_url(url: &str) -> String {
    if image_url_is_safe(url) {
        url.to_owned()
    } else {
        "#".to_owned()
    }
}

/// Parse an HTML `<a ...>` opening tag, returning its `href` and optional
/// `title`. Returns `None` for anything else, including an anchor with no
/// (or an empty) `href`.
pub(crate) fn parse_anchor_tag(html: &str) -> Option<(String, Option<String>)> {
    let trimmed = html.trim();
    let after_name = trimmed.as_bytes().get(2).copied()?;
    if !trimmed
        .as_bytes()
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"<a"))
        || !(after_name.is_ascii_whitespace() || after_name == b'>')
    {
        return None;
    }
    let parsed = parse_tag_attrs(trimmed, 2);
    let href = parsed.get("href")?;
    if href.is_empty() {
        return None;
    }
    let title = parsed.get("title").filter(|t| !t.is_empty()).cloned();
    // #5745: a raw-HTML anchor wrapping an image gets the same
    // allowed-scheme treatment as a Markdown link; an unsafe href is
    // neutralized (not dropped, which would strand the raw anchor tag
    // as a verbatim html_inline atom carrying the executable URL).
    Some((sanitize_link_url(href), title))
}

/// Attach a wrapping anchor's link to an image node (#880/#893), in the
/// same `linkHref`/`linkTitle` attrs the markdown `Image` arm produces.
pub(crate) fn image_with_link(mut img: Value, href: &str, link_title: Option<&str>) -> Value {
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
pub(crate) fn parse_linked_img_tag(html: &str) -> Option<Value> {
    let trimmed = html.trim();
    let (href, link_title) = parse_anchor_tag(trimmed)?;
    let img_start = find_ascii_case_insensitive(trimmed, b"<img")?;
    if !trimmed[..img_start].trim_end().ends_with('>')
        || find_ascii_case_insensitive(&trimmed[img_start..], b"</a>").is_none()
    {
        return None;
    }
    let img = parse_img_tag(&trimmed[img_start..])?;
    Some(image_with_link(img, &href, link_title.as_deref()))
}

/// Placement categories for the #1043 validation walk. A node whose type
/// is not listed here is left to `json_to_ast`, which reports it as
/// `ConversionError::UnknownType` — validation never masks that error.
/// (`front_matter` is intentionally unlisted: its doc-first-child rule
/// needs the child index, so `validate_placement` checks it directly.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
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

pub(crate) fn placement_of(node_type: &str) -> Option<Placement> {
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
pub(crate) fn placement_allowed(parent: &str, child: Placement) -> bool {
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
pub(crate) fn validate_placement(doc: &Value) -> Result<(), ConversionError> {
    pub(crate) fn walk(
        node: &Value,
        parent: Option<&str>,
        index_in_parent: Option<usize>,
    ) -> Result<(), ConversionError> {
        let Some(node_type) = node["type"].as_str() else {
            // Missing type: json_to_ast reports MissingType; keep that
            // error's precedence.
            return Ok(());
        };
        // #1369: `front_matter` is legal only as the first child of `doc`
        // (comrak only parses it there). Anything else is misplaced.
        if node_type == "front_matter" {
            let first_child_of_doc = parent == Some("doc") && index_in_parent == Some(0);
            if !first_child_of_doc {
                return Err(ConversionError::MisplacedNode {
                    parent: parent.unwrap_or("<root>").to_owned(),
                    child: node_type.to_owned(),
                });
            }
        } else if let Some(parent_type) = parent
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
        // front_matter carries no PM child content (its payload is the
        // `text` attr); nothing to recurse into.
        if node_type == "front_matter" {
            if node["content"].as_array().is_some_and(|c| !c.is_empty()) {
                return Err(ConversionError::MisplacedNode {
                    parent: node_type.to_owned(),
                    child: "<content>".to_owned(),
                });
            }
            return Ok(());
        }
        if let Some(content) = node["content"].as_array() {
            for (idx, child) in content.iter().enumerate() {
                walk(child, Some(node_type), Some(idx))?;
            }
        }
        Ok(())
    }
    walk(doc, None, None)
}
