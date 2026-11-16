//! Plain-text rendering of values that are not trusted to be row-safe.
//!
//! Peer names arrive over unsigned gossip, are stored verbatim by
//! [`id_core::discovery::PeerDiscovery::update`] and are served verbatim by
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
//! `LF`, `CR` and `TAB`, plus NUL, DEL, `NEL` and the rest), Unicode format
//! characters (Cf), and the line/paragraph separators `U+2028` / `U+2029`.
//! Everything else — including ASCII space and ordinary Unicode — is printed
//! byte-identically to before.
//!
//! The trigger set is deliberately narrower than the `key=value` renderer's
//! (#1611 also quotes on a space): a space does not delimit a TSV field, so a
//! peer named `my server` must keep displaying as `my server`, while a tab
//! would shift every following column and a newline would forge a row.
//!
//! A quoted field is a JSON string literal, so a consumer that needs the
//! original bytes can parse it as JSON. Values with no structure-breaking
//! character are the common case and stay unwrapped, exactly as before.
//!
//! # The plain-output contract (design #3455, Phase 3b)
//!
//! A plain-text field is row-safe only if it contains no character that can
//! act on the terminal. There are exactly two such classes:
//!
//! * **Structure characters**: control characters (category Cc, matched by
//!   `char::is_control`) plus the line/paragraph separators `U+2028` /
//!   `U+2029`, which are not controls but which some consumers treat as line
//!   breaks.
//! * **Format characters**: category Cf, per the crate's one shared set
//!   (`crate::is_format_char`). These are invisible or reorder the text
//!   around them (bidi overrides/isolates, zero-width characters), so they
//!   can falsify a row's columns at the terminal even from inside quotes.
//!
//! Every plain renderer in `pkgs/id` accounts for both classes; the
//! disposition differs by surface kind:
//!
//! * **Row renderers escape**: [`plain_field`] (this module) and the
//!   `key=value` renderer (`crate::commands::group`) quote an affected value and
//!   render each offending character as `\uXXXX`, so the value stays complete
//!   and JSON-recoverable but no raw actor reaches the terminal.
//! * **Stripping sanitizers strip**: `id_worlds::world::sanitize_display_name`,
//!   `id_worlds::world::sanitize_chat` and the SSH world's terminal gate drop both
//!   classes (chat keeps `ZWJ`/`ZWNJ` so legitimate emoji survive; the SSH
//!   gate keeps `LF`/`TAB` so terminal layout survives).
//! * **Ingest validators refuse**: directory names/descriptions, invite
//!   names and petnames refuse both classes at the boundary, so no new
//!   stored value carries one.
//!
//! No surface re-derives the character set: the single definition is
//! `crate::is_format_char`, the complete Unicode 16 general-category Cf
//! set, defined once in `id-core` (`crates/id-core/src/unicode.rs`) and
//! shared by validation and rendering.

use std::fmt::Display;

/// Returns `true` for characters that could break a tab-separated row.
///
/// Control characters cover `LF`, `CR`, `TAB`, NUL and every other C0/C1
/// code point (including `NEL`, `U+0085`); format characters can be invisible
/// or reorder text; and the Unicode line and paragraph separators are included
/// because some consumers treat them as line breaks even though they are not
/// control characters.
fn breaks_row(c: char) -> bool {
    c.is_control() || crate::is_format_char(c) || matches!(c, '\u{2028}' | '\u{2029}')
}

fn push_unicode_escape(rendered: &mut String, c: char) {
    use std::fmt::Write as _;

    let code_point = u32::from(c);
    if code_point <= 0xFFFF {
        // Writing into a String cannot fail.
        let _ = write!(rendered, "\\u{code_point:04x}");
    } else {
        // JSON escapes supplementary scalars as a UTF-16 surrogate pair.
        let scalar = code_point - 0x1_0000;
        let high = 0xD800 + (scalar >> 10);
        let low = 0xDC00 + (scalar & 0x3FF);
        let _ = write!(rendered, "\\u{high:04x}\\u{low:04x}");
    }
}

/// Renders `value` as one row-safe field.
///
/// A value with no structure-breaking character is returned unchanged. A
/// value that has one is returned as a double-quoted JSON string literal with
/// the structure-breaking characters escaped, so the result is always a single
/// physical line and never contains a raw tab.
///
/// ```rust
/// use id_cli::commands::text::plain_field;
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
                push_unicode_escape(&mut rendered, c);
            }
            c => rendered.push(c),
        }
    }
    rendered.push('"');
    rendered
}

/// Formats one tab-separated row whose name field may contain untrusted text.
pub fn tsv_row(hash: impl Display, name: &str) -> String {
    format!("{hash}\t{}", plain_field(name))
}

/// Formats one file-listing row (#4973):
/// `<hash>\t<name>\t<origin>\t<uploaded_at>`.
///
/// `origin` is the node that first wrote the file and `uploaded_at` its
/// upload time (unix seconds), both from the file's metadata tags. A file
/// written before those tags existed renders both as `unknown` — the
/// listing never guesses an origin.
pub fn list_tsv_row(
    hash: impl Display,
    name: &str,
    origin: Option<&str>,
    uploaded_at: Option<u64>,
) -> String {
    format!(
        "{}\t{}\t{}",
        tsv_row(hash, name),
        origin.map_or_else(|| "unknown".to_owned(), plain_field),
        uploaded_at.map_or_else(|| "unknown".to_owned(), |ts| ts.to_string())
    )
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
    fn format_characters_are_quoted_and_escaped_alone_or_with_a_tab() {
        let alone = plain_field("\u{202e}");
        assert_eq!(alone, r#""\u202e""#);
        assert!(!alone.contains('\u{202e}'));
        assert_eq!(serde_json::from_str::<String>(&alone).unwrap(), "\u{202e}");

        let with_tab = plain_field("a\u{202e}\tb");
        assert_eq!(with_tab, r#""a\u202e\tb""#);
        assert!(!with_tab.contains('\u{202e}'));
        assert_eq!(
            serde_json::from_str::<String>(&with_tab).unwrap(),
            "a\u{202e}\tb"
        );

        // Join controls are part of the same Cf set: escape them in both the
        // Cf-only and already-quoted paths without losing the stored value.
        let joiner_alone = plain_field("\u{200d}");
        assert_eq!(joiner_alone, r#""\u200d""#);
        assert!(!joiner_alone.contains('\u{200d}'));
        assert_eq!(
            serde_json::from_str::<String>(&joiner_alone).unwrap(),
            "\u{200d}"
        );

        let joiner_with_tab = plain_field("a\u{200d}\tb");
        assert_eq!(joiner_with_tab, r#""a\u200d\tb""#);
        assert!(!joiner_with_tab.contains('\u{200d}'));
        assert_eq!(
            serde_json::from_str::<String>(&joiner_with_tab).unwrap(),
            "a\u{200d}\tb"
        );
    }

    #[test]
    fn every_shared_format_character_is_json_escaped() {
        let mut count = 0;
        for code_point in 0..=0x10_FFFF {
            let Some(c) = char::from_u32(code_point) else {
                continue;
            };
            if !crate::is_format_char(c) {
                continue;
            }

            let value = c.to_string();
            let rendered = plain_field(&value);
            assert!(rendered.starts_with('"') && rendered.ends_with('"'));
            assert!(
                rendered.contains("\\u"),
                "{c:?} was not escaped: {rendered:?}"
            );
            assert!(!rendered.contains(c), "raw format character {c:?} remains");
            assert_eq!(serde_json::from_str::<String>(&rendered).unwrap(), value);
            count += 1;
        }
        assert!(
            count > 100,
            "expected the complete Unicode 16 Cf set, got {count}"
        );
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

    #[test]
    fn list_tsv_row_shows_origin_and_upload_time() {
        assert_eq!(
            list_tsv_row("hash", "a.txt", Some("node-a"), Some(1_700_000_000)),
            "hash\ta.txt\tnode-a\t1700000000"
        );
    }

    #[test]
    fn list_tsv_row_unknown_for_pre_change_files() {
        assert_eq!(
            list_tsv_row("hash", "old.txt", None, None),
            "hash\told.txt\tunknown\tunknown"
        );
    }

    #[test]
    fn tsv_row_keeps_control_characters_inside_the_name_field() {
        let row = tsv_row("hash", "x\nforged\tcolumn");
        assert_eq!(row, format!("hash\t{}", r#""x\nforged\tcolumn""#));
        assert_eq!(
            row.matches('\t').count(),
            1,
            "only the real delimiter remains"
        );
        assert!(!row.contains('\n'), "the name cannot add a physical row");

        assert_eq!(tsv_row("hash", "ordinary name"), "hash\tordinary name");
    }
}
