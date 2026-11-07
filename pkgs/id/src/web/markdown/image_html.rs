use super::*;

/// Escape a value for a double-quoted HTML attribute (#873): `&` first, so
/// the entities introduced here are not themselves re-escaped, then the
/// quote that would terminate the value, and `<`.
pub(super) fn escape_attr_value(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Decode the entities [`escape_attr_value`] can emit (plus `&gt;`/`&apos;`
/// and numeric single-quote) in one left-to-right pass, so `&amp;quot;`
/// decodes to the literal text `&quot;`, never to `"`.
pub(super) fn decode_attr_entities(value: &str) -> String {
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
pub(super) fn parse_tag_attrs(
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
pub(super) fn parse_integral_decimal(raw: &str) -> Option<u64> {
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

pub(super) fn parse_dimension(raw: &str) -> Option<u64> {
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
pub(super) fn find_ascii_case_insensitive(haystack: &str, needle: &[u8]) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|candidate| candidate.eq_ignore_ascii_case(needle))
}

pub(super) fn parse_img_tag(html: &str) -> Option<Value> {
    let trimmed = html.trim();
    let bytes = trimmed.as_bytes();
    if !bytes
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"<img"))
        || !matches!(bytes.get(4).copied(), Some(b' ' | b'>'))
    {
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

/// Parse an HTML `<a ...>` opening tag, returning its `href` and optional
/// `title`. Returns `None` for anything else, including an anchor with no
/// (or an empty) `href`.
pub(super) fn parse_anchor_tag(html: &str) -> Option<(String, Option<String>)> {
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
    Some((href.clone(), title))
}

/// Attach a wrapping anchor's link to an image node (#880/#893), in the
/// same `linkHref`/`linkTitle` attrs the markdown `Image` arm produces.
pub(super) fn image_with_link(mut img: Value, href: &str, link_title: Option<&str>) -> Value {
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
pub(super) fn parse_linked_img_tag(html: &str) -> Option<Value> {
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
