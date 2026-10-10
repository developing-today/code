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
    pub fn new(key: &str, node: &str, name: &str) -> Result<Self> {
        let key = hex_key(key).context("the account key")?;
        let node = hex_key(node).context("the node ID")?;
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
                "key" => key = Some(value),
                "node" => node = Some(value),
                "name" => name = Some(decode(value)?),
                _ => {}
            }
        }
        let key = key.context("an invite has no key")?;
        let node = node.context("an invite has no node")?;
        let name = name.context("an invite has no name")?;
        Self::new(key, node, &name)
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
}
