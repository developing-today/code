use super::*;

/// Placement categories for the #1043 validation walk. A node whose type
/// is not listed here is left to `json_to_ast`, which reports it as
/// `ConversionError::UnknownType` — validation never masks that error.
/// (`front_matter` is intentionally unlisted: its doc-first-child rule
/// needs the child index, so `validate_placement` checks it directly.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Placement {
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

pub(super) fn placement_of(node_type: &str) -> Option<Placement> {
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
pub(super) fn placement_allowed(parent: &str, child: Placement) -> bool {
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
pub(super) fn validate_placement(doc: &Value) -> Result<(), ConversionError> {
    fn walk(
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

/// Convert `ProseMirror` JSON document to markdown text.
///
/// Unknown marks are skipped by design (#1045): marks outside the
/// codec's vocabulary (`em`, `strong`, `strikethrough`, `link`, plus
/// the `code` mark handled specially by `create_marked_text`) are
/// dropped while the marked text itself is kept verbatim. This
/// differs from unknown *nodes*, which return
/// `ConversionError::UnknownType`, because an unknown inline style
/// can be shed without corrupting the document text. Known marks
/// render byte-identically whether or not unknown marks are present.
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
    Ok(restore_github_alert_markers(&output))
}

/// Undo the CommonMark renderer's escaping of a GitHub alert marker when
/// the marker is the first content of its blockquote. Markers elsewhere
/// (for example, followed by words on the same line) remain ordinary
/// escaped text. GitHub requires the marker to be the first thing in the
/// blockquote.
///
/// The scan skips fenced code blocks and raw HTML blocks: those are
/// literal content, so a line like `> \[\!NOTE\]` inside one is authored
/// text, not a renderer-escaped marker, and rewriting it would silently
/// change the code (Codex P2 on the first cut).

/// Whether a node value is a delimiter-mark wrapper (#1418): the
/// marks whose markdown spelling is a delimiter run (`*`/`**`/`~~`)
/// subject to CommonMark flanking rules. Link (bracket syntax) and
/// code (backtick syntax) tolerate edge spaces and are excluded.
pub(super) fn is_delimiter_wrapper(value: &NodeValue) -> bool {
    matches!(
        value,
        NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough
    )
}

/// The first text leaf of a mark chain, descending first children
/// through mark wrappers (#1418). `None` when the first inline is
/// anything else (an image or html node is a different mechanism —
/// #1407/#1412 — and keeps the legacy behaviour).
pub(super) fn first_text_leaf<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
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
pub(super) fn last_text_leaf<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
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
pub(super) fn last_rendered_char(node: &AstNode<'_>) -> Option<char> {
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
pub(super) fn first_rendered_char(node: &AstNode<'_>) -> Option<char> {
    match &node.data.borrow().value {
        NodeValue::Text(literal) => literal.to_string().chars().next(),
        NodeValue::Code(code) => code.literal.chars().next(),
        NodeValue::LineBreak | NodeValue::SoftBreak => Some('\n'),
        value if is_mark_wrapper(value) => node.first_child().and_then(first_rendered_char),
        _ => None,
    }
}

pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether the inline character immediately before `node` is a word
/// character (#1418), climbing through mark-wrapper ancestors while
/// `node` sits at their leading edge. No rendered neighbour (block
/// boundary) is not a word character.
pub(super) fn preceded_by_word_char(node: &AstNode<'_>) -> bool {
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
pub(super) fn followed_by_word_char(node: &AstNode<'_>) -> bool {
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
pub(super) fn set_text_leaf_text(leaf: &AstNode<'_>, new_text: &str) {
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
pub(super) fn hoist_one_edge<'a>(arena: &'a Arena<'a>, wrapper: &'a AstNode<'a>, leading: bool) {
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
pub(super) fn hoist_delimiter_edge_spaces<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>) {
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
/// are fenced by the global `prefer_fenced` render option (#954); this
/// placeholder is kept as belt-and-braces for the quoted shape.
pub(super) fn force_fenced_code_blocks_in_blockquotes<'a>(root: &'a AstNode<'a>) -> Vec<String> {
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
pub(super) fn strip_fence_placeholder(output: &str, token: &str) -> String {
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
pub(super) fn substitute_spaced_destinations<'a>(root: &'a AstNode<'a>) -> Vec<(String, String)> {
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
pub(super) fn is_block_node(node: &AstNode<'_>) -> bool {
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
            | NodeValue::FrontMatter(_)
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
pub(super) fn is_unmarked_text_json(node: &Value) -> bool {
    node["type"].as_str() == Some("text")
        && node["marks"]
            .as_array()
            .is_none_or(|marks| marks.is_empty())
}

/// Whether a JSON node is a `hard_break` — the boundary between two
/// rendered lines inside one block (#1420).
pub(super) fn is_hard_break_json(node: &Value) -> bool {
    node["type"].as_str() == Some("hard_break")
}

/// Whether an unmarked text node renders as nothing but edge whitespace
/// (empty, or spaces/tabs only). Such a node never shields a neighbour's
/// edge from the line boundary: at a line edge its own whitespace is
/// normalized away too, and mid-line it is a separator either way (#1420).
pub(super) fn is_whitespace_only_text_json(node: &Value) -> bool {
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
pub(super) fn reaches_line_edge(siblings: impl Iterator<Item = Value>) -> bool {
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
pub(super) fn normalize_inline_child_json(
    content: &[Value],
    idx: usize,
) -> std::borrow::Cow<'_, Value> {
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

pub(super) fn json_to_ast<'a>(
    arena: &'a Arena<'a>,
    json: &Value,
) -> Result<&'a AstNode<'a>, ConversionError> {
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
pub(super) fn valid_table_cell_child(value: &NodeValue) -> bool {
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
pub(super) fn append_table_cell_child<'a>(
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
pub(super) fn collect_cell_text<'a>(node: &'a AstNode<'a>, segments: &mut Vec<String>) {
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
pub(super) fn same_mark_wrapper(a: &NodeValue, b: &NodeValue) -> bool {
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
pub(super) fn is_mark_wrapper(value: &NodeValue) -> bool {
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
pub(super) fn peel_mark_chain<'a>(
    child: &'a AstNode<'a>,
) -> Option<(Vec<NodeValue>, &'a AstNode<'a>)> {
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
pub(super) fn insert_marked_chain<'a>(
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
pub(super) fn adopt_trailing_html_atoms<'a>(parent: &'a AstNode<'a>, wrappers: &[NodeValue]) {
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

pub(super) fn append_inline_child<'a>(
    arena: &'a Arena<'a>,
    parent: &'a AstNode<'a>,
    child: &'a AstNode<'a>,
) {
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
pub(super) fn create_marked_text<'a>(
    arena: &'a Arena<'a>,
    text: &str,
    marks: &[Value],
) -> &'a AstNode<'a> {
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
pub(super) fn build_mark_chain<'a>(
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
