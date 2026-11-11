//! Parsing safeguards for destinations, tables, and restored attributes.

use serde_json::Value;

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
pub(super) fn protect_newline_destinations(markdown: &str) -> (String, Option<String>) {
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
/// Fenced code blocks are skipped for the same reason (#2362): a
/// table example inside a fence is literal text comrak renders
/// verbatim, not a live table — the fence tracking mirrors
/// `protect_newline_destinations`.
/// A fence delimiter line (CommonMark §4.5): up to three leading
/// spaces, then a run of at least three backticks or tildes. Either
/// character toggles — the same approximation
/// `protect_newline_destinations` makes.
fn is_fence_line(line: &str) -> bool {
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

pub(super) fn escape_table_code_span_pipes(markdown: &str) -> String {
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
pub(super) fn restore_attr_newlines(value: &mut Value, token: &str) {
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
fn link_url_is_safe(url: &str) -> bool {
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
pub(super) fn sanitize_link_url(url: &str) -> String {
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
pub(super) fn image_url_is_safe(url: &str) -> bool {
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
pub(super) fn sanitize_image_url(url: &str) -> String {
    if image_url_is_safe(url) {
        url.to_owned()
    } else {
        "#".to_owned()
    }
}
