use super::*;

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
pub(super) fn markdown_to_prosemirror_inner(markdown: &str, recover_orphans: bool) -> Value {
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
        preserve_unreferenced_footnote_definitions(&mut doc, &protected, newline_token.as_deref());
    }
    doc
}

/// Convert a comrak AST node to `ProseMirror` JSON.
pub(super) fn convert_node<'a>(node: &'a AstNode<'a>, active_marks: &[Mark]) -> Value {
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
pub(super) fn text_node_with_marks(text: &str, marks: &[Mark]) -> Value {
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
pub(super) fn collect_block_children<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
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
pub(super) fn is_end_list_separator<'a>(node: &'a AstNode<'a>) -> bool {
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
pub(super) fn delimiter_attr(delimiter: ListDelimType) -> &'static str {
    match delimiter {
        ListDelimType::Period => "period",
        ListDelimType::Paren => "paren",
    }
}

/// #1380: read the PM `delimiter` attr back to a comrak delimiter.
/// Absent or unknown (legacy PM docs stored by the old codec, which
/// dropped the delimiter) defaults to Period - exactly how the old
/// codec saved every ordered list.
pub(super) fn delimiter_from_attrs(json: &Value) -> ListDelimType {
    match json["attrs"]["delimiter"].as_str() {
        Some("paren") => ListDelimType::Paren,
        _ => ListDelimType::Period,
    }
}

pub(super) fn convert_list_runs<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
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
pub(super) fn collect_list_items<'a>(node: &'a AstNode<'a>) -> Vec<Value> {
    node.children()
        .map(|child| convert_node(child, &[]))
        .filter(|v| !v.is_null())
        .collect()
}

/// The literal of an `HtmlInline` node, if `node` is one.
pub(super) fn html_inline_literal<'a>(node: &'a AstNode<'a>) -> Option<String> {
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
pub(super) fn linked_image_sequence<'a>(children: &[&'a AstNode<'a>]) -> Option<(Value, usize)> {
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
pub(super) fn collect_inline_children<'a>(
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
pub(super) fn get_text_content<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    collect_text(node, &mut text);
    text
}

/// Recursively collect text from a node.
pub(super) fn collect_text<'a>(node: &'a AstNode<'a>, text: &mut String) {
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
pub(super) fn get_alt_markdown<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    for child in node.children() {
        collect_alt_markdown(child, &mut out);
    }
    out
}

pub(super) fn collect_alt_markdown<'a>(node: &'a AstNode<'a>, out: &mut String) {
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
pub(super) fn alt_markdown_to_nodes<'a>(arena: &'a Arena<'a>, alt: &str) -> Vec<&'a AstNode<'a>> {
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
