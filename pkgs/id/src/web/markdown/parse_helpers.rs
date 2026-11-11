//! Parse-side helpers for the Markdown -> ProseMirror conversion:
//! link reference definitions, footnote synthesis, and destination
//! protection. Split out of `markdown.rs` for #4704.

use super::*;

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
pub(crate) fn link_reference_definition_title_text(after: &str) -> Option<&str> {
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
pub(crate) fn find_unescaped(s: &str, target: char) -> Option<usize> {
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
pub(crate) fn contains_unescaped(s: &str, closer: char) -> bool {
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
pub(crate) fn link_reference_definition_needs_more(after: &str) -> bool {
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
pub(crate) fn link_reference_definition_is_valid(after: &str) -> bool {
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
pub(crate) fn link_reference_destination_is_valid(after: &str) -> bool {
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
pub(crate) fn link_reference_definition_has_title(after: &str) -> bool {
    link_reference_definition_title_text(after).is_some_and(|t| !t.is_empty())
}

/// Whether a stripped line opens a link reference definition
/// (`[label]:`), footnote definitions excluded by the caller.
/// The index of the first UNESCAPED `]` in `s`: a `\]` inside
/// a reference label is label text, not the label's end (Codex
/// P2 on #1565's twelfth cut). Shared by the shape check and the
/// collector's destination extraction so they can never disagree
/// about where the label ends.
pub(crate) fn find_label_close(s: &str) -> Option<usize> {
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
pub(crate) fn link_reference_definition_label_close(s: &str) -> Option<usize> {
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

pub(crate) fn is_link_reference_definition_start(stripped: &str) -> bool {
    match link_reference_definition_label_close(stripped) {
        Some(close) => stripped[close + 1..].starts_with(':'),
        None => false,
    }
}

pub(crate) fn collect_link_reference_definitions(markdown: &str) -> Vec<(usize, String)> {
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

pub(crate) fn preserve_unreferenced_footnote_definitions(
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
pub(crate) fn synthesize_footnote_definitions(
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

/// Replace each newline inside an angle-form link/image destination
/// (`](<...>)` inline, `]: <...>` reference definition) with a token
/// comrak keeps verbatim in the parsed URL, returning the protected
/// source and the token used (`None` when no destination needed
/// protection). Literal contexts are skipped — a destination-shaped
/// string inside a fenced code block, an inline code span, or an
/// indented code block is literal text, not a link (#1707). Fence
/// tracking respects the opening fence's character and length. A
/// candidate destination that hits a blank line — including a
/// whitespace-only one — or another `<` before its closing `>` is
/// not a destination and is left alone.
pub(crate) fn protect_newline_destinations(markdown: &str) -> (String, Option<String>) {
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
    let mut out = String::with_capacity(markdown.len());
    let mut i = 0;
    // The open fence, when inside one: its character and run length.
    // A closing fence must use the same character and be at least as
    // long as the opener — a shorter fence line inside a longer
    // fence is literal content, not the end of the block (#1707).
    let mut fence: Option<(u8, usize)> = None;
    // Whether the current line is a fence line (opens, closes, or is
    // checked against an open fence): its backticks are fence syntax,
    // never an inline code span opener.
    let mut fence_line = false;
    // The open inline code span's backtick run length, when inside
    // one: span content is literal text, so a destination-shaped
    // string inside it must not be protected (#1707).
    let mut span: Option<usize> = None;
    // Whether the current line is an indented code block line
    // (4+ space indent at the start of a block): literal text too.
    let mut in_indented = false;
    let mut prev_line_blank = true;
    let mut prev_line_indented = false;
    let mut protected_any = false;
    while i < bytes.len() {
        // Line-start classification: fences, indented code blocks.
        if i == 0 || bytes[i - 1] == b'\n' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let indent = j - i;
            let line_blank = j >= bytes.len() || bytes[j] == b'\n';
            // Fences are recognised at up to 3 spaces of indent.
            let fence_run =
                if indent <= 3 && j < bytes.len() && (bytes[j] == b'`' || bytes[j] == b'~') {
                    let ch = bytes[j];
                    let mut n = 0;
                    while j + n < bytes.len() && bytes[j + n] == ch {
                        n += 1;
                    }
                    if n >= 3 { Some((ch, n)) } else { None }
                } else {
                    None
                };
            let was_in_fence = fence.is_some();
            match (fence, fence_run) {
                (None, Some(open)) => fence = Some(open),
                (Some((ch, n)), Some((cch, cn))) if cch == ch && cn >= n => fence = None,
                _ => {}
            }
            fence_line = fence_run.is_some() && (was_in_fence || fence.is_some());
            // An indented code block starts on a 4+ indented line
            // after a blank line (or at the start, or continuing an
            // indented block); a 4+ indented line inside a paragraph
            // is a lazy continuation, not code.
            in_indented = indent >= 4 && fence.is_none() && (prev_line_blank || prev_line_indented);
            prev_line_indented = in_indented || (indent >= 4 && line_blank && prev_line_indented);
            prev_line_blank = line_blank;
        }
        let literal_line = fence.is_some() || in_indented || fence_line;
        if !literal_line
            && span.is_none()
            && (markdown[i..].starts_with("](<") || markdown[i..].starts_with("]: <"))
        {
            let lt = if markdown[i..].starts_with("](<") {
                i + 2
            } else {
                i + 3
            };
            // Find the closing '>' before any blank line (a line of
            // only spaces/tabs between the newlines is blank too —
            // the paragraph break ends any destination, #1707) or a
            // nested '<'.
            let mut end = None;
            let mut k = lt + 1;
            while k < bytes.len() {
                match bytes[k] {
                    b'>' => {
                        end = Some(k);
                        break;
                    }
                    b'<' => break,
                    b'\n' => {
                        let mut m = k + 1;
                        while m < bytes.len()
                            && (bytes[m] == b' ' || bytes[m] == b'\t' || bytes[m] == b'\r')
                        {
                            m += 1;
                        }
                        if m >= bytes.len() || bytes[m] == b'\n' {
                            break;
                        }
                        k += 1;
                    }
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
        // Inline code span tracking (#1707): a run of backticks
        // outside fences/indented code opens a span closed by a run
        // of the same length; everything between is literal. Fence
        // lines were already classified at line start, so a run seen
        // here at a line start is a span only when it did not form a
        // fence (shorter than 3, or inside a fence — but fence lines
        // never reach here as non-literal).
        if !literal_line && bytes[i] == b'`' {
            let mut n = 0;
            while i + n < bytes.len() && bytes[i + n] == b'`' {
                n += 1;
            }
            match span {
                None => span = Some(n),
                Some(open) if n == open => span = None,
                _ => {}
            }
            out.push_str(&markdown[i..i + n]);
            i += n;
            continue;
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
pub(crate) fn is_table_delimiter_row(line: &str) -> bool {
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
pub(crate) fn byte_is_escaped(bytes: &[u8], i: usize) -> bool {
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
pub(crate) fn escape_code_span_pipes_in_line(line: &str) -> String {
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
        let Some(ch) = line.get(i..).and_then(|remaining| remaining.chars().next()) else {
            break;
        };
        let ch_len = ch.len_utf8();
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
/// Fenced code blocks are skipped for the same reason (#2362): a
/// table example inside a fence is literal text comrak renders
/// verbatim, not a live table — the fence tracking mirrors
/// `protect_newline_destinations`.
/// A fence delimiter line (CommonMark §4.5): up to three leading
/// spaces, then a run of at least three backticks or tildes. Either
/// character toggles — the same approximation
/// `protect_newline_destinations` makes.
pub(crate) fn is_fence_line(line: &str) -> bool {
    let mut spaces = 0;
    let mut chars = line.chars();
    let first = loop {
        match chars.next() {
            Some(' ') => {
                spaces += 1;
                if spaces > 3 {
                    return false;
                }
            }
            Some(c) => break c,
            None => return false,
        }
    };
    if first != '`' && first != '~' {
        return false;
    }
    1 + chars.take_while(|&c| c == first).count() >= 3
}

pub(crate) fn escape_table_code_span_pipes(markdown: &str) -> String {
    if !markdown.contains('`') || !markdown.contains('|') {
        return markdown.to_string();
    }
    let lines: Vec<&str> = markdown.split_inclusive('\n').collect();
    let mut out = String::with_capacity(markdown.len() + 16);
    let mut in_table = false;
    let mut in_fence = false;
    for (idx, raw) in lines.iter().enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let ending = &raw[line.len()..];
        if is_fence_line(line) {
            in_fence = !in_fence;
            if in_fence {
                // A table cannot span a fence; a delimiter-looking
                // row inside the fence must not arm `in_table` for
                // whatever follows the fence either.
                in_table = false;
            }
            out.push_str(raw);
            continue;
        }
        if in_fence {
            out.push_str(raw);
            continue;
        }
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
pub(crate) fn restore_attr_newlines(value: &mut Value, token: &str) {
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
