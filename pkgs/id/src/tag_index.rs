//! The tag index on the tuple encoding (Phase 5, P5.2; design §9.2).
//!
//! Each live claim is indexed twice, as a key in [`crate::tuple`] order:
//!
//! - α: `(subject, name, value, claim)`, for "the tags of X", "the tags of X named N" and "the
//!   tags of X named N with value V".
//! - Ω: `(value, name, subject, claim)`, for "the subjects with value V", "with value V and name N"
//!   and "with value V and name N on subject X".
//!
//! A bare tag has no value, and its value field is tuple null. Every text and byte field is
//! terminated, so a prefix on a whole field matches that field exactly: the name `a` does not
//! match `a\0b`. The index is local and rebuilt from the store's live claims. It is not replicated.

use std::collections::BTreeMap;

use p2panda_core::Hash;

use crate::tag_claims::Claim;
use crate::tuple::TupleEncoder;

/// One indexed claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The subject the claim is about.
    pub subject: Hash,
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
}

impl TagIndex {
    /// Builds the index over `claims`.
    #[must_use]
    pub fn new(claims: &[Claim]) -> Self {
        let mut alpha = BTreeMap::new();
        let mut omega = BTreeMap::new();
        for claim in claims {
            let entry = Entry {
                subject: claim.subject,
                name: claim.name.clone(),
                value: claim.value.clone(),
                claim: claim.id,
            };
            alpha.insert(alpha_key(&entry), entry.clone());
            omega.insert(omega_key(&entry), entry);
        }
        Self { alpha, omega }
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
}
