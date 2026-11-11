//! ProseMirror -> Markdown serialization (`prosemirror_to_markdown`
//! and its rendering helpers, including hard-break placement
//! normalization). Split out of `markdown.rs` for #4704.

use super::*;

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
    // #1002: collapse the renderer's stray blank lines at the top of a
    // quote (see `strip_leading_blank_quote_lines`). Stripping runs first
    // so a quote-leading GitHub alert marker becomes the quote's first
    // content line for the restore below.
    let output = strip_leading_blank_quote_lines(&output);
    let output = restore_github_alert_markers(&output);
    Ok(strip_end_list_separators(doc, &output))
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
pub(crate) fn restore_github_alert_markers(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut quote_has_content = [false; 64];
    let mut fence = FenceTracker::default();
    let mut fence_depth: Option<usize> = None;
    let mut html = HtmlBlockTracker::default();
    let mut html_depth: Option<usize> = None;
    let mut at_block_start = true;

    for line in markdown.split_inclusive('\n') {
        let content_start = line.trim_start_matches(' ');
        let leading_spaces = line.len() - content_start.len();
        let (depth, rest) = split_quote_prefix(content_start);

        // Four columns of indentation is code, and the markers below only
        // apply inside a blockquote. A quote nested deeper than the table
        // is left untouched rather than risking an out-of-range walk.
        if leading_spaces > 3 || depth >= quote_has_content.len() {
            quote_has_content.fill(false);
            fence.reset();
            fence_depth = None;
            html.reset();
            html_depth = None;
            output.push_str(line);
            at_block_start = leading_spaces > 3 || rest.trim().is_empty();
            continue;
        }

        // An open fence or HTML block owns every following line of its
        // container, including lines that look like quote markers: that
        // is exactly the literal-code case above.
        if let Some(opened_at) = fence_depth {
            match strip_quote_depth(content_start, opened_at) {
                Some(body) => {
                    fence.feed(fence_feed_line(body));
                    if !fence.in_fence() {
                        fence_depth = None;
                    }
                    output.push_str(line);
                    at_block_start = false;
                    continue;
                }
                None => {
                    fence.reset();
                    fence_depth = None;
                }
            }
        }
        if let Some(opened_at) = html_depth {
            match strip_quote_depth(content_start, opened_at) {
                Some(body) => {
                    html.feed(body, at_block_start);
                    if !html.in_html() {
                        html_depth = None;
                    }
                    output.push_str(line);
                    at_block_start = false;
                    continue;
                }
                None => {
                    html.reset();
                    html_depth = None;
                }
            }
        }

        // A fence or HTML block opening on this line makes its interior
        // literal from here on.
        if fence.feed(fence_feed_line(rest)) {
            fence_depth = Some(depth);
            output.push_str(line);
            at_block_start = false;
            continue;
        }
        if html.feed(rest, at_block_start) {
            html_depth = Some(depth);
            output.push_str(line);
            at_block_start = false;
            continue;
        }

        // Nothing outside a blockquote can open an alert, and a line
        // outside one ends every quote that was open.
        if depth == 0 {
            quote_has_content.fill(false);
            output.push_str(line);
            at_block_start = rest.trim().is_empty() || starts_new_block(rest);
            continue;
        }

        for seen in quote_has_content.iter_mut().skip(depth + 1) {
            *seen = false;
        }

        let trimmed = rest.trim();
        // Comrak emits a hard break with a trailing backslash. Ignore that
        // renderer syntax while recognizing the marker; the suffix is kept
        // intact when replacing the escaped marker below.
        let hardbreak = trimmed.ends_with('\\');
        let marker_token = trimmed.strip_suffix('\\').unwrap_or(trimmed);
        let marker = match marker_token {
            r"\[\!NOTE\]" => Some("[!NOTE]"),
            r"\[\!TIP\]" => Some("[!TIP]"),
            r"\[\!IMPORTANT\]" => Some("[!IMPORTANT]"),
            r"\[\!WARNING\]" => Some("[!WARNING]"),
            r"\[\!CAUTION\]" => Some("[!CAUTION]"),
            _ => None,
        };
        // A marker only opens an alert when nothing above it in the
        // quote -- at any depth -- has content yet. Marking ancestors
        // when a deeper line carries content keeps the marker of a
        // later paragraph in an outer quote escaped (Codex P2).
        let quote_is_empty = depth > 0 && !quote_has_content[..=depth].iter().any(|seen| *seen);
        if quote_is_empty
            && !rest.starts_with(' ')
            && !rest.starts_with('\t')
            && let Some(marker) = marker
        {
            let marker_start = line.find(marker_token).unwrap_or(0);
            let marker_end = marker_start + marker_token.len();
            output.push_str(&line[..marker_start]);
            output.push_str(marker);
            let suffix = &line[marker_end..];
            output.push_str(if hardbreak {
                suffix.strip_prefix('\\').unwrap_or(suffix)
            } else {
                suffix
            });
            for seen in quote_has_content.iter_mut().take(depth + 1).skip(1) {
                *seen = true;
            }
        } else {
            output.push_str(line);
            if !trimmed.is_empty() {
                for seen in quote_has_content.iter_mut().take(depth + 1).skip(1) {
                    *seen = true;
                }
            }
        }
        at_block_start = trimmed.is_empty() || starts_new_block(rest);
    }

    output
}

/// The start number a PM list renders with in Markdown, when it
/// renders as an ordered list: ordered_list always (attrs.order,
/// default 1); a task_list only when it carries attrs.order (#1370
/// ordered task runs render as `1. [ ]`). Bullet-rendering lists and
/// non-lists return None.
pub(crate) fn ordered_start(node: &Value) -> Option<u64> {
    let order = node["attrs"]["order"].as_u64();
    match node["type"].as_str() {
        Some("ordered_list") => Some(order.unwrap_or(1)),
        Some("task_list") => order,
        _ => None,
    }
}

pub(crate) fn list_kind(node: &Value) -> Option<&str> {
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
pub(crate) fn effective_tight(node: &Value) -> bool {
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
/// The fence run a rendered line opens or closes, if any: returns the
/// fence character and the remainder of the line starting at the run.
/// List-marker prefixes are stripped first (blockquote `>` markers and
/// indentation, then bullet / ordered / task-checkbox markers, repeated
/// for nested items), because a code block that is a list item's first
/// block renders its opening fence immediately after the marker
/// (`- ``` `). Plain lines return their own trimmed remainder.
pub(crate) fn list_fence_rest(line: &str) -> Option<(char, &str)> {
    let mut rest = line.trim_start_matches(['>', ' ']).trim_start();
    loop {
        let mut stripped = false;
        for marker in ["- ", "+ ", "* ", "[ ] ", "[x] ", "[X] "] {
            if let Some(r) = rest.strip_prefix(marker) {
                rest = r.trim_start();
                stripped = true;
                break;
            }
        }
        if stripped {
            continue;
        }
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let after = &rest[digits..];
            for marker in [". ", ") "] {
                if let Some(r) = after.strip_prefix(marker) {
                    rest = r.trim_start();
                    stripped = true;
                    break;
                }
            }
        }
        if !stripped {
            break;
        }
    }
    if rest.starts_with("```") {
        Some(('`', rest))
    } else if rest.starts_with("~~~") {
        Some(('~', rest))
    } else {
        None
    }
}

/// Whether a list's first item renders empty (no text, no block content
/// beyond empty paragraphs): an empty list item cannot interrupt a
/// paragraph under CommonMark, whatever its marker (Codex review,
/// PR #3258: `- a\n\n1.\n2. b` saved without the blank reparsed the
/// empty `1.` as lazy continuation text and lost the ordered list).
pub(crate) fn list_starts_with_empty_item(list: &Value) -> bool {
    let Some(first) = list["content"].as_array().and_then(|items| items.first()) else {
        return false;
    };
    pub(crate) fn node_is_empty(node: &Value) -> bool {
        if node["text"].as_str().is_some_and(|t| !t.trim().is_empty()) {
            return false;
        }
        match node["type"].as_str() {
            Some("paragraph" | "list_item" | "task_list_item" | "text") | None => {}
            Some(_) => return false,
        }
        node["content"]
            .as_array()
            .is_none_or(|parts| parts.iter().all(node_is_empty))
    }
    node_is_empty(first)
}

pub(crate) fn strip_end_list_separators(doc: &Value, output: &str) -> String {
    // Recompute with kinds: the drop decision needs them, so collect
    // (prev_kind, kind, tight_pair, merge_safe, needs_blank) tuples
    // alongside.
    let mut kinds: Vec<(String, String, bool, bool, bool)> = Vec::new();
    collect_boundary_kinds(doc, &mut kinds);
    if kinds.is_empty() {
        return output.to_string();
    }
    let is_separator_line =
        |line: &str| line.trim_start_matches(['>', ' ']).trim() == "<!-- end list -->";
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
    // Codex review, PR #3258: raw HTML blocks are literal content too.
    // A backtick-only line inside one (e.g. a <div> wrapping a ``` line)
    // is not a fence opener, and a separator-looking line inside the
    // block is authored HTML, not renderer markup. Track HTML block
    // state (CommonMark: the block runs until a blank line; an HTML
    // comment block runs until its --> closer) and exclude its lines
    // from both fence tracking and separator counting.
    let mut in_html = false;
    let mut html_is_comment = false;
    let mut is_separator: Vec<bool> = Vec::with_capacity(lines.len());
    for line in &lines {
        let html_probe = line.trim_start_matches(['>', ' ']).trim();
        if in_html {
            is_separator.push(false);
            if html_is_comment {
                if html_probe.contains("-->") {
                    in_html = false;
                    html_is_comment = false;
                }
            } else if html_probe.is_empty() {
                in_html = false;
            }
            continue;
        }
        if !in_fence && !is_separator_line(line) {
            let second = html_probe.chars().nth(1);
            let opens_html = html_probe.starts_with('<')
                && second.is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'));
            if opens_html {
                is_separator.push(false);
                if html_probe.starts_with("<!--") {
                    if !html_probe[4..].contains("-->") {
                        in_html = true;
                        html_is_comment = true;
                    }
                } else {
                    in_html = true;
                }
                continue;
            }
        }
        // Codex review, PR #3258: a fence can open immediately after a
        // list marker (a list item whose first block is a code block
        // renders as `- ``` `): strip any list-marker prefixes (bullet,
        // ordered, task checkbox, nested) before looking for the fence
        // run, or the opener is missed, the fence's literal content is
        // counted as a separator, and the closing fence is mistaken
        // for an opener that hides the real separator.
        let fence_rest = list_fence_rest(line);
        let fence_marker = fence_rest.map(|(c, _)| c);
        match fence_marker {
            Some(c) if !in_fence => {
                in_fence = true;
                fence_char = c;
                fence_len = fence_rest
                    .map(|(_, rest)| rest.chars().take_while(|&ch| ch == c).count())
                    .unwrap_or(0);
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
                    && fence_rest.is_some_and(|(_, rest)| {
                        rest.trim_end().chars().all(|ch| ch == c)
                            && rest.trim_end().len() >= fence_len
                    }) =>
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
pub(crate) fn collect_boundary_kinds(
    node: &Value,
    out: &mut Vec<(String, String, bool, bool, bool)>,
) {
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
            // The empty-item guard (below) applies only across
            // different rendered list families: within one family the
            // two runs reparse as a single list, where an empty sibling
            // item needs no blank - keeping one would loosen the merged
            // list and break the save fixpoint (Codex review, PR #3258).
            let needs_blank = (matches!(ordered_start(child), Some(start) if start != 1)
                && ordered_start(prev).is_none())
                || (list_starts_with_empty_item(child)
                    && renders_ordered(prev) != renders_ordered(child));
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
pub(crate) fn is_delimiter_wrapper(value: &NodeValue) -> bool {
    matches!(
        value,
        NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough
    )
}

/// The first text leaf of a mark chain, descending first children
/// through mark wrappers (#1418). `None` when the first inline is
/// anything else (an image or html node is a different mechanism —
/// #1407/#1412 — and keeps the legacy behaviour).
pub(crate) fn first_text_leaf<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
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
pub(crate) fn last_text_leaf<'a>(node: &'a AstNode<'a>) -> Option<&'a AstNode<'a>> {
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
pub(crate) fn last_rendered_char(node: &AstNode<'_>) -> Option<char> {
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
pub(crate) fn first_rendered_char(node: &AstNode<'_>) -> Option<char> {
    match &node.data.borrow().value {
        NodeValue::Text(literal) => literal.to_string().chars().next(),
        NodeValue::Code(code) => code.literal.chars().next(),
        NodeValue::LineBreak | NodeValue::SoftBreak => Some('\n'),
        value if is_mark_wrapper(value) => node.first_child().and_then(first_rendered_char),
        _ => None,
    }
}

pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether the inline character immediately before `node` is a word
/// character (#1418), climbing through mark-wrapper ancestors while
/// `node` sits at their leading edge. No rendered neighbour (block
/// boundary) is not a word character.
pub(crate) fn preceded_by_word_char(node: &AstNode<'_>) -> bool {
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
pub(crate) fn followed_by_word_char(node: &AstNode<'_>) -> bool {
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
pub(crate) fn set_text_leaf_text(leaf: &AstNode<'_>, new_text: &str) {
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
pub(crate) fn hoist_one_edge<'a>(arena: &'a Arena<'a>, wrapper: &'a AstNode<'a>, leading: bool) {
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
pub(crate) fn hoist_delimiter_edge_spaces<'a>(arena: &'a Arena<'a>, root: &'a AstNode<'a>) {
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
pub(crate) fn force_fenced_code_blocks_in_blockquotes<'a>(root: &'a AstNode<'a>) -> Vec<String> {
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
pub(crate) fn strip_fence_placeholder(output: &str, token: &str) -> String {
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

/// Whether a rendered line is a blank blockquote line: up to 3 leading
/// spaces (CommonMark still reads the `>` as a quote marker; 4+ spaces
/// would be indented code), then only `>` markers and whitespace
/// (`>`, `> `, `> > `). A list-item quote opener (`- > `) is NOT one —
/// it carries the list marker, so the list-nested single-blank shape
/// is out of scope here.
fn is_blank_quote_line(line: &str) -> bool {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    rest.contains('>') && rest.chars().all(|c| c == '>' || c == ' ' || c == '\t')
}

/// Whether a rendered line is quoted content (as opposed to a blank
/// quote line or a non-quote line): a `>` marker with non-blank text
/// after it (`> text`, `> ```rust`, `> | a |`, `> -----`).
fn is_quote_content_line(line: &str) -> bool {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return false;
    }
    line[indent..]
        .strip_prefix('>')
        .is_some_and(|after| !after.trim().is_empty())
}

/// #1002: comrak's CommonMark renderer emits blank `> ` lines before a
/// blockquote's first child when that child is a fenced code block, a
/// table, a thematic break, or an HTML block — two at top level, so the
/// first save of any such quote pollutes the file (the PM document is
/// correct throughout and the output is already a fixpoint). Drop those
/// leading blanks from the rendered output, at the start of EVERY quote
/// run (start of file or right after a non-quote line, so a second
/// quote after a blank line is covered too). Blank lines BETWEEN blocks
/// inside a quote are the required separators and are kept: anything
/// after the run's first content line passes through untouched, as does
/// an all-blank run (a lone `>` with no quoted content).
fn strip_leading_blank_quote_lines(output: &str) -> String {
    let lines: Vec<&str> = output.split('\n').collect();
    let mut result: Vec<&str> = Vec::with_capacity(lines.len());
    let mut at_quote_start = true;
    let mut i = 0;
    while i < lines.len() {
        if at_quote_start && is_blank_quote_line(lines[i]) {
            let mut j = i + 1;
            while j < lines.len() && is_blank_quote_line(lines[j]) {
                j += 1;
            }
            if j < lines.len() && is_quote_content_line(lines[j]) {
                i = j;
                continue;
            }
        }
        result.push(lines[i]);
        at_quote_start = !is_blank_quote_line(lines[i]) && !is_quote_content_line(lines[i]);
        i += 1;
    }
    result.join("\n")
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
pub(crate) fn substitute_spaced_destinations<'a>(root: &'a AstNode<'a>) -> Vec<(String, String)> {
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
pub(crate) fn is_block_node(node: &AstNode<'_>) -> bool {
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
pub(crate) fn is_unmarked_text_json(node: &Value) -> bool {
    node["type"].as_str() == Some("text")
        && node["marks"]
            .as_array()
            .is_none_or(|marks| marks.is_empty())
}

/// Whether a JSON node is a `hard_break` — the boundary between two
/// rendered lines inside one block (#1420).
pub(crate) fn is_hard_break_json(node: &Value) -> bool {
    node["type"].as_str() == Some("hard_break")
}

/// Whether an unmarked text node renders as nothing but edge whitespace
/// (empty, or spaces/tabs only). Such a node never shields a neighbour's
/// edge from the line boundary: at a line edge its own whitespace is
/// normalized away too, and mid-line it is a separator either way (#1420).
pub(crate) fn is_whitespace_only_text_json(node: &Value) -> bool {
    is_unmarked_text_json(node)
        && node["text"]
            .as_str()
            .unwrap_or("")
            .chars()
            .all(|c| c == ' ' || c == '\t')
}

/// Whether a hard break has no rendered content after it before the
/// block ends. Whitespace-only text and further breaks do not make a
/// trailing break interior content. This is the placement boundary for
/// #1388: CommonMark cannot spell a break after the last content of a
/// block, so normalize it away before building the comrak AST.
pub(crate) fn hard_break_is_trailing(content: &[Value], idx: usize) -> bool {
    content[idx + 1..]
        .iter()
        .all(|sibling| is_hard_break_json(sibling) || is_whitespace_only_text_json(sibling))
}

/// Apply the save-side hard-break placement policy for a block's inline
/// children (#1388, transferring the #1492 placements): a hard break at
/// block end is dropped explicitly (the renderer silently drops it); a
/// heading is single-line CommonMark content, so an interior break
/// degrades to a space instead of becoming an unspellable backslash-newline.
/// Other interior paragraph breaks retain their existing LineBreak form.
pub(crate) fn normalize_hard_break_placement(node_type: &str, content: &[Value]) -> Vec<Value> {
    if !matches!(node_type, "heading" | "paragraph") {
        return content.to_vec();
    }

    let mut normalized = Vec::with_capacity(content.len());
    for (idx, child) in content.iter().enumerate() {
        if !is_hard_break_json(child) {
            normalized.push(child.clone());
            continue;
        }
        if hard_break_is_trailing(content, idx) {
            continue;
        }
        if node_type == "heading" {
            normalized.push(json!({"type": "text", "text": " "}));
        } else {
            normalized.push(child.clone());
        }
    }
    normalized
}

/// Whether scanning `siblings` (in render order) reaches a line edge —
/// the start/end of the block or a `hard_break` — before any content
/// that renders something (#1420).
pub(crate) fn reaches_line_edge(siblings: impl Iterator<Item = Value>) -> bool {
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
pub(crate) fn normalize_inline_child_json(
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
