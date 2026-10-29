//! The tag index on the tuple encoding (Phase 5, P5.2; design §9.2).
//!
//! Each live claim is indexed three times, as a key in [`crate::tuple`] order:
//!
//! - α: `(subject, name, value, claim)`, for "the tags of X", "the tags of X named N" and "the
//!   tags of X named N with value V".
//! - Ω: `(value, name, subject, claim)`, for "the subjects with value V", "with value V and name N"
//!   and "with value V and name N on subject X".
//! - Author: `(author, name, value, subject, claim)`, for "the claims by A", "the claims by A
//!   named N" and "the claims by A named N with value V" — the per-author index design §9.4
//!   search-by-author needs (#800). The index is rebuilt from the live claims on each build,
//!   so the author key needs no persisted format or migration.
//!
//! A bare tag has no value, and its value field is tuple null. Every text and byte field is
//! terminated, so a prefix on a whole field matches that field exactly: the name `a` does not
//! match `a\0b`. The index is local and rebuilt from the store's live claims. It is not replicated.

use std::collections::BTreeMap;

use p2panda_core::Hash;

use crate::tag_claims::Claim;
use crate::tuple::TupleEncoder;
use crate::waiting::AuthorKey;

/// One indexed claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The subject the claim is about.
    pub subject: Hash,
    /// The key that signed the claim.
    pub author: AuthorKey,
    /// The tag name.
    pub name: String,
    /// The tag value, or `None` for a bare tag.
    pub value: Option<String>,
    /// The claim's ID.
    pub claim: Hash,
}

/// The tag index: the live claims, keyed by their tuple encodings in both directions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagIndex {
    alpha: BTreeMap<Vec<u8>, Entry>,
    omega: BTreeMap<Vec<u8>, Entry>,
    by_author: BTreeMap<Vec<u8>, Entry>,
}

impl TagIndex {
    /// Builds the index over `claims`.
    #[must_use]
    pub fn new(claims: &[Claim]) -> Self {
        let mut alpha = BTreeMap::new();
        let mut omega = BTreeMap::new();
        let mut by_author = BTreeMap::new();
        for claim in claims {
            let entry = Entry {
                subject: claim.subject,
                author: claim.author,
                name: claim.name.clone(),
                value: claim.value.clone(),
                claim: claim.id,
            };
            alpha.insert(alpha_key(&entry), entry.clone());
            omega.insert(omega_key(&entry), entry.clone());
            by_author.insert(author_key(&entry), entry);
        }
        Self {
            alpha,
            omega,
            by_author,
        }
    }

    /// The number of claims indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.alpha.len()
    }

    /// Whether no claim is indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.alpha.is_empty()
    }

    /// Every claim on `subject`, whatever its name and value.
    #[must_use]
    pub fn subject(&self, subject: &Hash) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        prefix.bytes(subject.as_bytes());
        scan(&self.alpha, &prefix.build())
    }

    /// Every claim on `subject` named `name`, whatever its value.
    #[must_use]
    pub fn subject_name(&self, subject: &Hash, name: &str) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        prefix.bytes(subject.as_bytes()).string(name);
        scan(&self.alpha, &prefix.build())
    }

    /// Every claim on `subject` named `name` with `value`. `None` is the bare tag.
    #[must_use]
    pub fn subject_name_value(
        &self,
        subject: &Hash,
        name: &str,
        value: Option<&str>,
    ) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        prefix.bytes(subject.as_bytes()).string(name);
        value_field(&mut prefix, value);
        scan(&self.alpha, &prefix.build())
    }

    /// Every claim named `name`, on any subject, with any value. The name is not a leading field of
    /// either key, so this is a scan of the whole Ω index.
    #[must_use]
    pub fn named(&self, name: &str) -> Vec<Entry> {
        self.omega
            .values()
            .filter(|entry| entry.name == name)
            .cloned()
            .collect()
    }

    /// Every claim with `value`, whatever its name and subject. `None` is the bare tag.
    #[must_use]
    pub fn value(&self, value: Option<&str>) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        value_field(&mut prefix, value);
        scan(&self.omega, &prefix.build())
    }

    /// Every claim with `value` and `name`, on any subject.
    #[must_use]
    pub fn value_name(&self, value: Option<&str>, name: &str) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        value_field(&mut prefix, value);
        prefix.string(name);
        scan(&self.omega, &prefix.build())
    }

    /// Every claim with `value` and `name` on `subject`.
    #[must_use]
    pub fn value_name_subject(
        &self,
        value: Option<&str>,
        name: &str,
        subject: &Hash,
    ) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        value_field(&mut prefix, value);
        prefix.string(name).bytes(subject.as_bytes());
        scan(&self.omega, &prefix.build())
    }

    /// Every claim by `author`, on any subject, with any name and value.
    #[must_use]
    pub fn author(&self, author: &AuthorKey) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        prefix.bytes(author);
        scan(&self.by_author, &prefix.build())
    }

    /// Every claim by `author` named `name`, on any subject, with any value.
    #[must_use]
    pub fn author_name(&self, author: &AuthorKey, name: &str) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        prefix.bytes(author).string(name);
        scan(&self.by_author, &prefix.build())
    }

    /// Every claim by `author` named `name` with `value`, on any subject. `None` is the
    /// bare tag.
    #[must_use]
    pub fn author_name_value(
        &self,
        author: &AuthorKey,
        name: &str,
        value: Option<&str>,
    ) -> Vec<Entry> {
        let mut prefix = TupleEncoder::new();
        prefix.bytes(author).string(name);
        value_field(&mut prefix, value);
        scan(&self.by_author, &prefix.build())
    }
}

/// The α key of a claim: `(subject, name, value, claim)`.
fn alpha_key(entry: &Entry) -> Vec<u8> {
    let mut key = TupleEncoder::new();
    key.bytes(entry.subject.as_bytes()).string(&entry.name);
    value_field(&mut key, entry.value.as_deref());
    key.bytes(entry.claim.as_bytes());
    key.build()
}

/// The Ω key of a claim: `(value, name, subject, claim)`.
fn omega_key(entry: &Entry) -> Vec<u8> {
    let mut key = TupleEncoder::new();
    value_field(&mut key, entry.value.as_deref());
    key.string(&entry.name).bytes(entry.subject.as_bytes());
    key.bytes(entry.claim.as_bytes());
    key.build()
}

/// The author key of a claim: `(author, name, value, subject, claim)`.
fn author_key(entry: &Entry) -> Vec<u8> {
    let mut key = TupleEncoder::new();
    key.bytes(&entry.author).string(&entry.name);
    value_field(&mut key, entry.value.as_deref());
    key.bytes(entry.subject.as_bytes());
    key.bytes(entry.claim.as_bytes());
    key.build()
}

/// The value field: a string, or null for a bare tag.
fn value_field(encoder: &mut TupleEncoder, value: Option<&str>) {
    match value {
        Some(text) => encoder.string(text),
        None => encoder.null(),
    };
}

/// The entries whose keys start with `prefix`, in key order.
fn scan(index: &BTreeMap<Vec<u8>, Entry>, prefix: &[u8]) -> Vec<Entry> {
    index
        .range(prefix.to_vec()..)
        .take_while(|(key, _)| key.starts_with(prefix))
        .map(|(_, entry)| entry.clone())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn claim(n: u8, subject: u8, name: &str, value: Option<&str>) -> Claim {
        Claim {
            id: Hash::from_bytes([n; 32]),
            author: [n; 32],
            subject: Hash::from_bytes([subject; 32]),
            name: name.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn test_named_returns_the_claims_with_that_name_on_every_subject() {
        let claims = [
            claim(1, 9, "city", Some("Seattle")),
            claim(2, 8, "city", None),
            claim(3, 9, "name", None),
            claim(4, 7, "cityx", None),
            claim(5, 7, "a\0b", None),
        ];
        let index = TagIndex::new(&claims);
        let mut got: Vec<[u8; 32]> = index
            .named("city")
            .iter()
            .map(|entry| *entry.claim.as_bytes())
            .collect();
        got.sort_unstable();
        assert_eq!(got, vec![[1; 32], [2; 32]]);
        assert!(index.named("a").is_empty(), "a is not a\\0b");
        assert!(index.named("missing").is_empty());
    }

    fn claim_by(id: u8, author: u8, subject: u8, name: &str, value: Option<&str>) -> Claim {
        Claim {
            id: Hash::from_bytes([id; 32]),
            author: [author; 32],
            subject: Hash::from_bytes([subject; 32]),
            name: name.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    #[test]
    fn test_author_returns_only_that_authors_claims() {
        let claims = [
            claim_by(1, 7, 9, "city", Some("Seattle")),
            claim_by(2, 7, 8, "city", None),
            claim_by(3, 6, 9, "city", Some("Seattle")),
            claim_by(4, 7, 9, "name", None),
            claim_by(5, 6, 7, "a\0b", None),
        ];
        let index = TagIndex::new(&claims);
        let mut got: Vec<[u8; 32]> = index
            .author(&[7; 32])
            .iter()
            .map(|entry| *entry.claim.as_bytes())
            .collect();
        got.sort_unstable();
        assert_eq!(got, vec![[1; 32], [2; 32], [4; 32]]);
        assert!(index.author(&[9; 32]).is_empty());
        // The entry carries the author, so a scan can report provenance.
        assert_eq!(index.author(&[6; 32])[0].author, [6; 32]);
    }

    #[test]
    fn test_author_name_and_value_narrow_the_author_scan() {
        let claims = [
            claim_by(1, 7, 9, "city", Some("Seattle")),
            claim_by(2, 7, 8, "city", None),
            claim_by(3, 7, 9, "name", None),
            claim_by(4, 6, 9, "city", Some("Seattle")),
        ];
        let index = TagIndex::new(&claims);
        // Scan order is key order, in which a bare (null) value sorts before a
        // valued one, so compare the ID sets, not the scan sequence.
        let ids = |entries: Vec<Entry>| -> Vec<[u8; 32]> {
            let mut ids: Vec<[u8; 32]> = entries
                .iter()
                .map(|entry| *entry.claim.as_bytes())
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(
            ids(index.author_name(&[7; 32], "city")),
            vec![[1; 32], [2; 32]]
        );
        assert_eq!(
            ids(index.author_name_value(&[7; 32], "city", Some("Seattle"))),
            vec![[1; 32]]
        );
        assert_eq!(
            ids(index.author_name_value(&[7; 32], "city", None)),
            vec![[2; 32]]
        );
        assert!(index.author_name(&[7; 32], "missing").is_empty());
    }
}
