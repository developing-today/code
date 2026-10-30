//! Plain-text rendering of values that are not trusted to be row-safe.
//!
//! Peer names arrive over unsigned gossip, are stored verbatim by
//! [`crate::discovery::PeerDiscovery::update`] and are served verbatim by
//! `MetaRequest::ListPeers`, so a printer must never assume a name is free of
//! bytes that mean something to its own format (#1613).
//!
//! # The ruling
//!
//! Values are **not** validated or rewritten at ingest or serving time — a
//! store may already hold a value that ingest validation could not have
//! cleaned — so the rendering contract lives at the printer: *a printer never
//! lets a value break row structure*. This is the same ruling as the plain-text
//! renderer fix for `id claim` / `id doc` / `id group` (#1611): the value is
//! still printed in full, but anything that could be mistaken for structure is
//! rendered inside a double-quoted JSON string literal, with the offending
//! bytes escaped (`\n`, `\r`, `\t`, `\u00XX`, ...).
//!
//! # Which values are quoted
//!
//! [`plain_field`] quotes a value when it contains a character that could
//! break or blur a **tab-separated** row: any control character (this includes
//! `LF`, `CR` and `TAB`, plus NUL, DEL, `NEL` and the rest) and the Unicode
//! line/paragraph separators `U+2028` / `U+2029`. Everything else — including
//! ASCII space and ordinary Unicode — is printed byte-identically to before.
//!
//! The trigger set is deliberately narrower than the `key=value` renderer's
//! (#1611 also quotes on a space): a space does not delimit a TSV field, so a
//! peer named `my server` must keep displaying as `my server`, while a tab
//! would shift every following column and a newline would forge a row.
//!
//! A quoted field is a JSON string literal, so a consumer that needs the
//! original bytes can parse it as JSON. Values with no structure-breaking
//! character are the common case and stay unwrapped, exactly as before.

/// Returns `true` for characters that could break a tab-separated row.
///
/// Control characters cover `LF`, `CR`, `TAB`, NUL and every other C0/C1
/// code point (including `NEL`, `U+0085`); the Unicode line and paragraph
/// separators are included because some consumers treat them as line breaks
/// even though they are not control characters.
fn breaks_row(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// Renders `value` as one row-safe field.
///
/// A value with no structure-breaking character is returned unchanged. A
/// value that has one is returned as a double-quoted JSON string literal with
/// the structure-breaking characters escaped, so the result is always a single
/// physical line and never contains a raw tab.
///
/// ```rust
/// use id::commands::text::plain_field;
///
/// assert_eq!(plain_field("my-server"), "my-server");
/// assert_eq!(plain_field("my server"), "my server");
/// assert_eq!(plain_field("naïve-✦"), "naïve-✦");
/// assert_eq!(plain_field("a\nb"), r#""a\nb""#);
/// assert_eq!(plain_field("a\tb"), r#""a\tb""#);
/// ```
pub fn plain_field(value: &str) -> String {
    if !value.chars().any(breaks_row) {
        return value.to_owned();
    }

    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('"');
    for c in value.chars() {
        match c {
            '"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\u{0008}' => rendered.push_str("\\b"),
            '\u{000c}' => rendered.push_str("\\f"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            c if breaks_row(c) => {
                use std::fmt::Write as _;
                // Writing into a String cannot fail.
                let _ = write!(rendered, "\\u{:04x}", u32::from(c));
            }
            c => rendered.push(c),
        }
    }
    rendered.push('"');
    rendered
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn space_free_values_render_byte_identically() {
        for value in ["my-server", "peer_1", "naïve", "サーバー", "✦"] {
            assert_eq!(plain_field(value), value);
        }
    }

    #[test]
    fn space_is_not_quoted() {
        // A space does not delimit a TSV field: an ordinary two-word name keeps
        // displaying as written (#1613 acceptance).
        assert_eq!(plain_field("my server"), "my server");
    }

    #[test]
    fn newline_is_quoted_and_escaped() {
        let rendered = plain_field("a\nb");
        assert_eq!(rendered, r#""a\nb""#);
        assert!(!rendered.contains('\n'), "must be one physical line");
    }

    #[test]
    fn carriage_return_is_quoted_and_escaped() {
        let rendered = plain_field("a\rb");
        assert_eq!(rendered, r#""a\rb""#);
        assert!(!rendered.contains('\r'));
    }

    #[test]
    fn tab_is_quoted_and_escaped() {
        let rendered = plain_field("a\tb");
        assert_eq!(rendered, r#""a\tb""#);
        assert!(!rendered.contains('\t'), "must not shift a column");
    }

    #[test]
    fn other_control_characters_and_line_separators_are_escaped() {
        let rendered = plain_field("a\u{0}b\u{7f}c\u{85}d\u{2028}e\u{2029}f");
        assert_eq!(rendered, r#""a\u0000b\u007fc\u0085d\u2028e\u2029f""#);
        for c in rendered.chars() {
            assert!(!breaks_row(c), "escaped output still holds {c:?}");
        }
    }

    #[test]
    fn quotes_and_backslashes_are_escaped_inside_a_quoted_value() {
        assert_eq!(plain_field("a\"b\nc"), r#""a\"b\nc""#);
        assert_eq!(plain_field("a\\b\nc"), r#""a\\b\nc""#);
    }

    #[test]
    fn forged_payload_cannot_add_a_row() {
        let rendered = plain_field(
            "x\n0000000000000000000000000000000000000000000000000000000000000000\tforged\t9\t0",
        );
        assert_eq!(rendered.matches('\t').count(), 0);
        assert!(!rendered.contains('\n'));
    }
}
