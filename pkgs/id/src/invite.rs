//! Invite links: an account's key, a server's node ID and a name.
//!
//! The `id:invite?…` form is a string format for people to copy and paste.
//! Nothing registers the `id:` scheme with an operating system or resolves
//! it. The HTTPS form, `/invite?…`, carries the same query.

use anyhow::{Context, Result, bail, ensure};

const SCHEME: &str = "id:invite?";
const HTTPS_PATH: &str = "/invite?";
const MAX_NAME_CHARS: usize = 64;

/// What an invite names: an account, the server it is on, and the name it states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// The account's public key, as 64 lower-case hex characters.
    pub key: String,
    /// The server's node ID, as 64 lower-case hex characters.
    pub node: String,
    /// The name the account states for itself.
    pub name: String,
}

impl Invite {
    /// An invite for `key` on `node`, naming it `name`.
    ///
    /// # Errors
    ///
    /// Fails if either key is not 64 hex characters, or the name is empty or too long.
    /// Also fails if the name carries a control (category Cc) or format
    /// (category Cf) character anywhere, checked on the raw name before
    /// trimming: an edge tab or newline is refused, not trimmed away —
    /// the name lands on the `/invite` trust prompt and is prefilled as
    /// a petname, and petnames refuse the same characters (#1770).
    pub fn new(key: &str, node: &str, name: &str) -> Result<Self> {
        let key = hex_key(key).context("the account key")?;
        let node = hex_key(node).context("the node ID")?;
        ensure!(
            !name.chars().any(char::is_control),
            "an invite name cannot contain control characters"
        );
        ensure!(
            !name.chars().any(crate::petname::is_format_char),
            "an invite name cannot contain format characters"
        );
        let name = name.trim();
        ensure!(!name.is_empty(), "an invite needs a name");
        ensure!(
            name.chars().count() <= MAX_NAME_CHARS,
            "an invite name is at most {MAX_NAME_CHARS} characters"
        );
        Ok(Self {
            key,
            node,
            name: name.to_owned(),
        })
    }

    /// The `id:invite?…` string.
    #[must_use]
    pub fn url(&self) -> String {
        format!("{SCHEME}{}", self.query())
    }

    /// The `/invite?…` path for the HTTPS confirm page.
    #[must_use]
    pub fn https_path(&self) -> String {
        format!("{HTTPS_PATH}{}", self.query())
    }

    fn query(&self) -> String {
        format!(
            "key={}&node={}&name={}",
            self.key,
            self.node,
            encode(&self.name)
        )
    }

    /// Read an `id:invite?…` string.
    ///
    /// # Errors
    ///
    /// Fails if the text does not start with `id:invite?` or its query is invalid.
    pub fn parse(text: &str) -> Result<Self> {
        let query = text
            .trim()
            .strip_prefix(SCHEME)
            .context("an invite starts with id:invite?")?;
        Self::from_query(query)
    }

    /// Read the query of an invite, the part after `?`.
    ///
    /// # Errors
    ///
    /// Fails if `key`, `node` or `name` is missing or invalid.
    pub fn from_query(query: &str) -> Result<Self> {
        let mut key = None;
        let mut node = None;
        let mut name = None;
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (field, value) = pair.split_once('=').context("a query pair needs =")?;
            match field {
                "key" => {
                    ensure!(key.is_none(), "duplicate key field in invite query");
                    key = Some(decode(value)?);
                }
                "node" => {
                    ensure!(node.is_none(), "duplicate node field in invite query");
                    node = Some(decode(value)?);
                }
                "name" => {
                    ensure!(name.is_none(), "duplicate name field in invite query");
                    name = Some(decode(value)?);
                }
                _ => {}
            }
        }
        let key = key.context("an invite has no key")?;
        let node = node.context("an invite has no node")?;
        let name = name.context("an invite has no name")?;
        Self::new(&key, &node, &name)
    }
}

fn hex_key(text: &str) -> Result<String> {
    let text = text.trim().to_ascii_lowercase();
    ensure!(
        text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()),
        "must be 64 hex characters"
    );
    Ok(text)
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    out
}

fn decode(text: &str) -> Result<String> {
    let raw = text.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&byte) = raw.get(i) {
        match byte {
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            b'%' => {
                let digits = raw.get(i + 1..i + 3).context("a % needs two hex digits")?;
                if !digits.iter().all(u8::is_ascii_hexdigit) {
                    bail!("a % needs two hex digits");
                }
                let value = std::str::from_utf8(digits)?;
                bytes.push(u8::from_str_radix(value, 16)?);
                i += 3;
            }
            other => {
                bytes.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8(bytes).context("the name is not UTF-8")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const KEY: &str = "AA11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";
    const NODE: &str = "bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22";

    #[test]
    fn an_invite_round_trips_through_its_string() {
        let name = "Ann Lee & Co. + 🦀 %/?=#";
        let invite = Invite::new(KEY, NODE, name).unwrap();
        let url = invite.url();
        assert!(url.starts_with("id:invite?key=aa11"));
        assert!(url.contains("&node=bb22"));
        assert_eq!(Invite::parse(&url).unwrap(), invite);
        assert_eq!(Invite::parse(&url).unwrap().name, name);
    }

    #[test]
    fn the_https_path_carries_the_same_query() {
        let invite = Invite::new(KEY, NODE, "Ann Lee").unwrap();
        let path = invite.https_path();
        assert_eq!(
            path,
            "/invite?key=aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11&node=bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22&name=Ann%20Lee"
        );
        let query = path.strip_prefix("/invite?").unwrap();
        assert_eq!(Invite::from_query(query).unwrap(), invite);
    }

    #[test]
    fn plus_reads_as_a_space() {
        let query = format!("key={KEY}&node={NODE}&name=Ann+Lee");
        assert_eq!(Invite::from_query(&query).unwrap().name, "Ann Lee");
    }

    #[test]
    fn a_bad_key_or_node_is_refused() {
        assert!(Invite::new("abcd", NODE, "Ann").is_err());
        assert!(Invite::new(&"g".repeat(64), NODE, "Ann").is_err());
        assert!(Invite::new(KEY, "xyz", "Ann").is_err());
    }

    #[test]
    fn a_missing_field_or_a_bad_escape_is_refused() {
        assert!(Invite::parse(&format!("id:invite?key={KEY}&node={NODE}")).is_err());
        assert!(Invite::parse(&format!("id:invite?key={KEY}&node={NODE}&name=%zz")).is_err());
        assert!(Invite::parse(&format!("id:invite?key={KEY}&node={NODE}&name=%4")).is_err());
    }

    #[test]
    fn the_scheme_is_required() {
        assert!(Invite::parse(&format!("https://x/invite?key={KEY}&node={NODE}&name=A")).is_err());
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert!(Invite::new(KEY, NODE, "   ").is_err());
        assert!(Invite::new(KEY, NODE, &"x".repeat(65)).is_err());
    }

    #[test]
    fn test_duplicate_fields_are_refused() {
        let other_key = "11".repeat(32);
        let other_node = "33".repeat(32);

        // Duplicate key (both valid, invalid first, invalid last)
        let err = Invite::from_query(&format!("key={KEY}&key={other_key}&node={NODE}&name=Ann"))
            .unwrap_err();
        assert!(format!("{err:#}").contains("key"), "{err:#}");
        let err =
            Invite::from_query(&format!("key=invalid&key={KEY}&node={NODE}&name=Ann")).unwrap_err();
        assert!(format!("{err:#}").contains("key"), "{err:#}");
        let err =
            Invite::from_query(&format!("key={KEY}&key=invalid&node={NODE}&name=Ann")).unwrap_err();
        assert!(format!("{err:#}").contains("key"), "{err:#}");

        // Duplicate node (both valid, invalid first, invalid last)
        let err = Invite::from_query(&format!("key={KEY}&node={NODE}&node={other_node}&name=Ann"))
            .unwrap_err();
        assert!(format!("{err:#}").contains("node"), "{err:#}");
        let err = Invite::from_query(&format!("key={KEY}&node=invalid&node={NODE}&name=Ann"))
            .unwrap_err();
        assert!(format!("{err:#}").contains("node"), "{err:#}");
        let err = Invite::from_query(&format!("key={KEY}&node={NODE}&node=invalid&name=Ann"))
            .unwrap_err();
        assert!(format!("{err:#}").contains("node"), "{err:#}");

        // Duplicate name
        let err =
            Invite::from_query(&format!("key={KEY}&node={NODE}&name=Ann&name=Eve")).unwrap_err();
        assert!(format!("{err:#}").contains("name"), "{err:#}");
        // Invalid escapes are refused whether they occur first or last
        assert!(Invite::from_query(&format!("key={KEY}&node={NODE}&name=%zz&name=Eve")).is_err());
        assert!(Invite::from_query(&format!("key={KEY}&node={NODE}&name=Ann&name=%zz")).is_err());

        // Unknown fields keep current handling (ignored, even when repeated)
        let invite = Invite::from_query(&format!(
            "key={KEY}&node={NODE}&name=Ann&extra=1&extra=2&other=test"
        ))
        .unwrap();
        assert_eq!(invite.name, "Ann");
    }

    #[test]
    fn control_characters_are_refused_before_trimming() {
        // Embedded Cc, and Cc at the edges where trim() would
        // otherwise have removed it silently (#1770).
        for name in [
            "Ann\tLee",
            "Ann\nLee",
            "Ann\rLee",
            "Ann\u{7}Lee",
            "\tAnn",
            "Ann\n",
            "\rAnn Lee",
            "\u{b}Ann",
        ] {
            assert!(
                Invite::new(KEY, NODE, name).is_err(),
                "control character accepted in {name:?}"
            );
        }
    }

    #[test]
    fn format_characters_are_refused() {
        // Representative Cf: soft hyphen, zero-width space, a bidi
        // override, the BOM/zero-width no-break space, a tag char.
        for name in [
            "Ann\u{ad}Lee",
            "Ann\u{200b}Lee",
            "Ann\u{202e}Lee",
            "\u{feff}Ann",
            "Ann\u{e0041}",
        ] {
            assert!(
                Invite::new(KEY, NODE, name).is_err(),
                "format character accepted in {name:?}"
            );
        }
    }

    #[test]
    fn control_and_format_characters_are_refused_through_queries() {
        // The same refusals through the percent-encoded parse paths:
        // %09 tab, %0A newline, %0D CR, %E2%80%8B U+200B, %C2%AD
        // U+00AD — at the edges as well as embedded.
        for encoded in [
            "Ann%09Lee",
            "Ann%0ALee",
            "Ann%0DLee",
            "%09Ann",
            "Ann%0A",
            "Ann%E2%80%8BLee",
            "%C2%ADAnn",
        ] {
            let query = format!("key={KEY}&node={NODE}&name={encoded}");
            assert!(
                Invite::from_query(&query).is_err(),
                "encoded control/format character accepted in {encoded}"
            );
            assert!(
                Invite::parse(&format!("id:invite?{query}")).is_err(),
                "encoded control/format character accepted via parse in {encoded}"
            );
        }
    }

    #[test]
    fn ordinary_spaces_still_trim() {
        // The Cc/Cf refusal is on the raw name; plain spaces keep the
        // long-standing trim behaviour (#1770).
        let invite = Invite::new(KEY, NODE, "  Ann Lee  ").unwrap();
        assert_eq!(invite.name, "Ann Lee");
        let invite = Invite::from_query(&format!("key={KEY}&node={NODE}&name=+Ann+Lee+")).unwrap();
        assert_eq!(invite.name, "Ann Lee");
    }
}
