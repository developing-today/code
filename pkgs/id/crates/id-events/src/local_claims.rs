//! The node's own local-only tag claims (#1793, issue #1793 bullet 2).
//!
//! A tag claim's scope says where it lives. A shared claim is a signed `tag.set` event in the
//! event store ([`id_store::tag_claims`]): it is in the sync set, so every peer receives it. A
//! **local** claim is the opposite in exactly one respect — it is never signed into the event
//! store, so it is never in the sync set and never reaches another node. It is kept here, in a
//! JSON file beside the event store, and only this node reads it.
//!
//! The file is append-only in effect: a claim is a line, and a `tag.clear` of a local claim
//! removes its line. Rewriting the file is atomic (write a temporary file, then rename), so a
//! crash mid-write leaves either the old file or the new one, never a torn one.
//!
//! Local claims are deliberately not events: their IDs are content-derived
//! ([`LocalClaim::id_for`]) and carry no signature, because there is no second node to convince.
//! That is the whole point of the scope — a claim that must never leave the node must not be
//! something the node could accidentally publish.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use id_core::waiting::AuthorKey;
use id_store::tag_claims::{
    LocalClaim, MAX_NAME_BYTES, MAX_VALUE_BYTES, TagError, TagScope, check_text,
};

/// The file local claims live in, under the event store's directory.
pub const FILE_NAME: &str = "local-claims.jsonl";

/// One line of the local claim file.
#[derive(Debug, Serialize, Deserialize)]
struct Line {
    /// The claim's ID, hex.
    id: String,
    /// The author's key, hex.
    author: String,
    /// The subject, hex.
    subject: String,
    /// The tag name.
    name: String,
    /// The tag value, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

/// The node's local-only claims, keyed by claim ID.
///
/// # Errors
///
/// Every method that reads or writes the file returns an [`std::io::Error`] when the file cannot
/// be read or written, and the file's own parse errors surface as `InvalidData`.
#[derive(Debug, Default)]
pub struct LocalClaims {
    /// The file the claims are persisted in.
    path: PathBuf,
    /// The claims, by ID, in ID order so the file is deterministic.
    claims: BTreeMap<[u8; 32], LocalClaim>,
}

impl LocalClaims {
    /// Opens the local claim store in `dir`, reading `local-claims.jsonl` if it exists. A missing
    /// file is an empty store.
    ///
    /// # Errors
    ///
    /// Fails if the directory cannot be created, the file cannot be read, or a line is not a
    /// well-formed claim.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        let mut claims = BTreeMap::new();
        if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            for (number, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let parsed: Line = serde_json::from_str(line).map_err(|err| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{} line {}: {err}", path.display(), number + 1),
                    )
                })?;
                let claim = parsed.into_claim().map_err(|err| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{} line {}: {err}", path.display(), number + 1),
                    )
                })?;
                claims.insert(*claim.id.as_bytes(), claim);
            }
        }
        Ok(Self { path, claims })
    }

    /// The path of the file the claims are persisted in.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every local claim, in ID order.
    #[must_use]
    pub fn claims(&self) -> Vec<LocalClaim> {
        self.claims.values().cloned().collect()
    }

    /// The local claims on `subject`, in ID order.
    #[must_use]
    pub fn on_subject(&self, subject: &Hash) -> Vec<LocalClaim> {
        self.claims
            .values()
            .filter(|claim| claim.subject == *subject)
            .cloned()
            .collect()
    }

    /// The local claims named `name`, and with `value` if given, on any subject, in ID order.
    #[must_use]
    pub fn matching(&self, name: &str, value: Option<&str>) -> Vec<LocalClaim> {
        self.claims
            .values()
            .filter(|claim| {
                claim.name == name && value.is_none_or(|want| claim.value.as_deref() == Some(want))
            })
            .cloned()
            .collect()
    }

    /// Records a local claim: `name` (and `value`) on `subject`, authored by `author`. The claim's
    /// ID is derived from its content, so recording the same claim twice is a no-op that returns
    /// the same ID. The file is rewritten atomically.
    ///
    /// # Errors
    ///
    /// Fails if the name or value breaks its limit or the NFC rule, or the file cannot be written.
    pub fn set(
        &mut self,
        author: &AuthorKey,
        subject: Hash,
        name: &str,
        value: Option<&str>,
    ) -> std::io::Result<Hash> {
        check_text("name", name, 1, MAX_NAME_BYTES)
            .and_then(|()| match value {
                Some(text) => check_text("value", text, 0, MAX_VALUE_BYTES),
                None => Ok(()),
            })
            .map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, err.to_string())
            })?;
        let id = LocalClaim::id_for(author, &subject, name, value);
        let claim = LocalClaim {
            id,
            author: *author,
            subject,
            name: name.to_owned(),
            value: value.map(str::to_owned),
        };
        self.claims.insert(*id.as_bytes(), claim);
        self.persist()?;
        Ok(id)
    }

    /// Removes the local claims named `name` on `subject` that `author` recorded. Returns the
    /// number removed; zero means nothing was removed. The file is rewritten atomically.
    ///
    /// Only the recording author's own local claims are removed: a local claim has no other
    /// author on this node, and a clear from anyone else has no meaning here.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be written.
    pub fn clear(
        &mut self,
        author: &AuthorKey,
        subject: Hash,
        name: &str,
    ) -> std::io::Result<usize> {
        let before = self.claims.len();
        self.claims.retain(|_, claim| {
            !(claim.subject == subject && claim.name == name && claim.author == *author)
        });
        let removed = before - self.claims.len();
        if removed > 0 {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Rewrites the file atomically: a temporary file in the same directory, then a rename.
    fn persist(&self) -> std::io::Result<()> {
        let temporary = self.path.with_extension("jsonl.tmp");
        let mut file = std::fs::File::create(&temporary)?;
        for claim in self.claims.values() {
            let line = Line {
                id: claim.id.to_hex(),
                author: hex(&claim.author),
                subject: claim.subject.to_hex(),
                name: claim.name.clone(),
                value: claim.value.clone(),
            };
            let text = serde_json::to_string(&line)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
            writeln!(file, "{text}")?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, &self.path)
    }
}

impl Line {
    /// Parses the line into a claim, checking that its ID is the one its content derives.
    fn into_claim(self) -> Result<LocalClaim, String> {
        let id = hash_from_hex(&self.id).map_err(|err| format!("id: {err}"))?;
        let author = author_from_hex(&self.author).map_err(|err| format!("author: {err}"))?;
        let subject = hash_from_hex(&self.subject).map_err(|err| format!("subject: {err}"))?;
        let derived = LocalClaim::id_for(&author, &subject, &self.name, self.value.as_deref());
        if derived != id {
            return Err(format!(
                "the claim ID {} is not the one its content derives ({})",
                self.id,
                derived.to_hex()
            ));
        }
        Ok(LocalClaim {
            id,
            author,
            subject,
            name: self.name,
            value: self.value,
        })
    }
}

/// Parses a 32-byte hash from hex.
fn hash_from_hex(text: &str) -> Result<Hash, String> {
    let bytes = hex_bytes(text)?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| format!("{text:?} is not 32 bytes"))?;
    Ok(Hash::from_bytes(array))
}

/// Parses a 32-byte author key from hex.
fn author_from_hex(text: &str) -> Result<AuthorKey, String> {
    let bytes = hex_bytes(text)?;
    bytes
        .try_into()
        .map_err(|_| format!("{text:?} is not 32 bytes"))
}

/// Decodes lowercase or uppercase hex, two digits per byte.
fn hex_bytes(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("{text:?} has an odd number of hex digits"));
    }
    // Every byte must be an ASCII hex digit (#7750, the #2517 gate the
    // sibling parsers already carry): u8::from_str_radix also accepts a
    // leading plus sign, so without this gate a byte 0x00-0x0f has two accepted
    // spellings. Decoding works on bytes, never str slices at byte
    // offsets, so a multi-byte character in a corrupt line is an error,
    // not a char-boundary panic.
    if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{text:?} is not hex"));
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).map_err(|_| format!("{text:?} is not hex"))?;
            u8::from_str_radix(digits, 16).map_err(|_| format!("{text:?} is not hex"))
        })
        .collect()
}

/// Lowercase hex of a byte string.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The scope a claim name records, if the name carries one. This is the create/edit surface's
/// shorthand: a name written `local:NAME` is a local claim named `NAME`, and any other name is a
/// shared claim. It is a convenience for the CLI only; the scope is stored as a field, never as
/// part of the name.
#[must_use]
pub fn split_scope_prefix(name: &str) -> (TagScope, &str) {
    match name.strip_prefix("local:") {
        Some(rest) if !rest.is_empty() => (TagScope::Local, rest),
        _ => (TagScope::Shared, name),
    }
}

/// The error a scope prefix with an empty name produces, for the CLI to report.
#[must_use]
pub fn scope_prefix_error(name: &str) -> Option<TagError> {
    if name == "local:" {
        return Some(TagError::Field(
            "a local claim needs a name after \"local:\"".to_owned(),
        ));
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn author(n: u8) -> AuthorKey {
        [n; 32]
    }

    fn subject(n: u8) -> Hash {
        Hash::from_bytes([n; 32])
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "id-local-claims-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_a_local_claim_survives_reopening_and_is_not_a_shared_claim() {
        let dir = temp_dir("roundtrip");
        let id = {
            let mut store = LocalClaims::open(&dir).unwrap();
            store
                .set(&author(1), subject(2), "city", Some("Oslo"))
                .unwrap()
        };
        let store = LocalClaims::open(&dir).unwrap();
        let claims = store.on_subject(&subject(2));
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].id, id);
        assert_eq!(claims[0].as_claim().scope, TagScope::Local);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_recording_the_same_local_claim_twice_is_a_no_op() {
        let dir = temp_dir("idempotent");
        let mut store = LocalClaims::open(&dir).unwrap();
        let first = store.set(&author(1), subject(2), "city", None).unwrap();
        let second = store.set(&author(1), subject(2), "city", None).unwrap();
        assert_eq!(first, second);
        assert_eq!(store.claims().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_a_local_claim_id_separates_name_and_value_boundaries() {
        let a = LocalClaim::id_for(&author(1), &subject(2), "a", Some("bc"));
        let b = LocalClaim::id_for(&author(1), &subject(2), "ab", Some("c"));
        let bare = LocalClaim::id_for(&author(1), &subject(2), "abc", None);
        assert_ne!(a, b);
        assert_ne!(a, bare);
    }

    #[test]
    fn test_a_clear_removes_only_the_authors_claims_of_that_name() {
        let dir = temp_dir("clear");
        let mut store = LocalClaims::open(&dir).unwrap();
        store.set(&author(1), subject(2), "city", None).unwrap();
        store.set(&author(1), subject(2), "other", None).unwrap();
        assert_eq!(store.clear(&author(9), subject(2), "city").unwrap(), 0);
        assert_eq!(store.clear(&author(1), subject(2), "city").unwrap(), 1);
        assert_eq!(store.claims().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_a_tampered_line_is_refused_on_open() {
        let dir = temp_dir("tamper");
        {
            let mut store = LocalClaims::open(&dir).unwrap();
            store.set(&author(1), subject(2), "city", None).unwrap();
        }
        let path = dir.join(FILE_NAME);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("\"city\"", "\"town\"")).unwrap();
        assert!(LocalClaims::open(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_the_scope_prefix_selects_a_local_claim() {
        assert_eq!(split_scope_prefix("local:city"), (TagScope::Local, "city"));
        assert_eq!(split_scope_prefix("city"), (TagScope::Shared, "city"));
        assert_eq!(split_scope_prefix("local:"), (TagScope::Shared, "local:"));
        assert!(scope_prefix_error("local:").is_some());
    }

    #[test]
    fn test_hex_bytes_refuses_plus_pairs_and_multibyte_input() {
        // #7750: the plus spelling the #2517 gate refuses in the sibling
        // parsers, and multi-byte input at a misaligned offset (which used
        // to panic on a char boundary instead of erroring).
        assert!(hex_bytes(&"+a".repeat(32)).is_err());
        assert!(hex_bytes("€a").is_err());
        assert!(hex_bytes(&format!("{}€a", "0a".repeat(30))).is_err());
        assert_eq!(hex_bytes(&"0A".repeat(32)).unwrap(), vec![10; 32]);
        assert!(hex_bytes("abc").is_err());
        assert!(hex_bytes(&"zz".repeat(32)).is_err());
    }
}
