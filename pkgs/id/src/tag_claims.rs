//! Tags as signed events: observed-remove claims (Phase 5, P5.1; design §9.1, spike S8).
//!
//! A tag is a claim. `tag.set` names a subject (any event ID), a name, and an optional value. It
//! is attributed to the key that signed it. `tag.clear` names the claims it removes. Both are
//! ordinary events: the kind is in the header's [`ChainExt`](crate::chain::ChainExt), and the body
//! is the payload the header commits to.
//!
//! A clear removes a claim only when the claim is named by the clear and lies in the clear's
//! causal past, which is the set of events its author had seen. A concurrent add the clear did not
//! see survives it. A clear that names a claim its author never saw does nothing to that claim.
//! The past is the transitive closure of each event's `after` hashes and backlink, so the clear's
//! frontier need not name a claim for the clear to remove it.
//!
//! Bodies are canonical CBOR, version [`VERSION`]:
//!
//! - `tag.set`: `[1, subject, name, value]`. The subject is a 32-byte string, the name is text of
//!   1 to [`MAX_NAME_BYTES`] bytes, and the value is text of at most [`MAX_VALUE_BYTES`] bytes or
//!   null.
//! - `tag.clear`: `[1, [claim, ...]]`, with at most [`MAX_CLEAR_CLAIMS`] claims, each a 32-byte
//!   string.
//!
//! Text is NFC, as the header's kind is. The old iroh-docs tag store in [`crate::tags`] is a
//! separate model and is not read here.

use std::collections::{BTreeMap, HashMap, HashSet};

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_core::Hash;
use unicode_normalization::is_nfc;

use crate::waiting::AuthorKey;

/// The kind of a `tag.set` event, as the header's `kind` names it.
pub const SET_KIND: &str = "tag.set";

/// The kind of a `tag.clear` event, as the header's `kind` names it.
pub const CLEAR_KIND: &str = "tag.clear";

/// The body format version this code writes and reads.
pub const VERSION: u64 = 1;

/// The most bytes a tag name may have. A name is at least one byte.
pub const MAX_NAME_BYTES: usize = 128;

/// The most bytes a tag value may have.
pub const MAX_VALUE_BYTES: usize = 1024;

/// The most claims one `tag.clear` may name.
pub const MAX_CLEAR_CLAIMS: usize = 256;

/// The most bytes a tag body may have. A full clear is about 8.7 KiB, so this leaves room.
pub const MAX_TAG_BODY_BYTES: usize = 16 * 1024;

/// Nesting limit for a tag body's CBOR. Matches the group body codec.
const RECURSION_LIMIT: u16 = 8;

/// Declared element limit for one array. Matches the header decoder.
const LENGTH_LIMIT: u64 = 512;

/// Byte budget for speculative pre-allocation. Matches the header decoder.
const OOM_MITIGATION: usize = 64;

/// What a tag event does. The kind of the event says which form the body takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagOp {
    /// Adds a claim: the subject carries `name` (and `value`), attributed to the signer.
    Set {
        /// The event the claim is about. Any event ID.
        subject: Hash,
        /// The tag name.
        name: String,
        /// The tag value. `None` for a bare tag.
        value: Option<String>,
    },
    /// Removes the named claims, if the clear's author had seen them.
    Clear {
        /// The IDs of the `tag.set` events this clear removes.
        claims: Vec<Hash>,
    },
}

/// An applied tag event, as the fold reads it: its ID, its signer, and its operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// The event's ID. For a `tag.set`, this is the claim's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// What the event does.
    pub op: TagOp,
}

/// A claim that is live: a `tag.set` that no applied clear removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    /// The claim's ID: the `tag.set` event's ID.
    pub id: Hash,
    /// The key that signed the claim.
    pub author: AuthorKey,
    /// The subject the claim is about.
    pub subject: Hash,
    /// The tag name.
    pub name: String,
    /// The tag value, if any.
    pub value: Option<String>,
}

/// Why a tag body or operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    /// The kind is not a tag kind.
    #[error("{0:?} is not a tag kind")]
    NotATag(String),
    /// The body is not the canonical encoding of the value it holds.
    #[error("the tag body is not canonical CBOR")]
    NotCanonical,
    /// The body does not decode as CBOR.
    #[error("the tag body does not decode: {0}")]
    Cbor(String),
    /// The body does not have the shape its kind requires.
    #[error("the tag body has the wrong shape: {0}")]
    Shape(String),
    /// The body names a version this code does not read.
    #[error("tag body version {0} is not supported")]
    Version(u64),
    /// A field breaks a limit or the NFC rule.
    #[error("{0}")]
    Field(String),
}

/// Whether `kind` is a tag kind, so its body is a tag operation.
#[must_use]
pub fn is_tag_kind(kind: &str) -> bool {
    kind == SET_KIND || kind == CLEAR_KIND
}

/// Encodes a tag operation as its body.
///
/// # Errors
///
/// Returns [`TagError::Field`] for a name or value that breaks its limit or the NFC rule, and for
/// a clear with more than [`MAX_CLEAR_CLAIMS`] claims. Also returns [`TagError::Field`] for a body
/// over [`MAX_TAG_BODY_BYTES`].
pub fn encode_op(op: &TagOp) -> Result<Vec<u8>, TagError> {
    let value = match op {
        TagOp::Set {
            subject,
            name,
            value,
        } => {
            check_text("name", name, 1, MAX_NAME_BYTES)?;
            let value = match value {
                Some(text) => {
                    check_text("value", text, 0, MAX_VALUE_BYTES)?;
                    Value::TextString(text.as_str().into())
                }
                None => Value::null(),
            };
            Value::Array(vec![
                Value::Unsigned(VERSION),
                Value::ByteString(subject.as_bytes().to_vec().into()),
                Value::TextString(name.as_str().into()),
                value,
            ])
        }
        TagOp::Clear { claims } => {
            if claims.len() > MAX_CLEAR_CLAIMS {
                return Err(TagError::Field(format!(
                    "{} claims, the most is {MAX_CLEAR_CLAIMS}",
                    claims.len()
                )));
            }
            let named = claims
                .iter()
                .map(|claim| Value::ByteString(claim.as_bytes().to_vec().into()))
                .collect();
            Value::Array(vec![Value::Unsigned(VERSION), Value::Array(named)])
        }
    };
    let body = value.encode();
    if body.len() > MAX_TAG_BODY_BYTES {
        return Err(TagError::Field(format!(
            "a tag body of {} bytes, the most is {MAX_TAG_BODY_BYTES}",
            body.len()
        )));
    }
    Ok(body)
}

/// Decodes a tag body, given its event's kind.
///
/// # Errors
///
/// Returns the reason the body is refused. See [`TagError`].
pub fn decode_op(kind: &str, body: &[u8]) -> Result<TagOp, TagError> {
    if !is_tag_kind(kind) {
        return Err(TagError::NotATag(kind.to_owned()));
    }
    if body.len() > MAX_TAG_BODY_BYTES {
        return Err(TagError::Field(format!(
            "a tag body of {} bytes, the most is {MAX_TAG_BODY_BYTES}",
            body.len()
        )));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(body)
        .map_err(|err| TagError::Cbor(err.to_string()))?;
    if value.encode().as_slice() != body {
        return Err(TagError::NotCanonical);
    }
    let items = value
        .as_array()
        .map_err(|err| TagError::Shape(format!("a tag body is an array: {err}")))?;
    let Some((version, rest)) = items.split_first() else {
        return Err(TagError::Shape("a tag body is empty".to_owned()));
    };
    let Value::Unsigned(version) = version else {
        return Err(TagError::Shape("the version is not an integer".to_owned()));
    };
    if *version != VERSION {
        return Err(TagError::Version(*version));
    }
    if kind == SET_KIND {
        decode_set(rest)
    } else {
        decode_clear(rest)
    }
}

/// The fields of a `tag.set` body, after the version.
fn decode_set(rest: &[Value<'static>]) -> Result<TagOp, TagError> {
    let [subject, name, value] = rest else {
        return Err(TagError::Shape(
            "a tag.set has a version, a subject, a name and a value".to_owned(),
        ));
    };
    let subject = decode_hash(subject)?;
    let name = decode_text("name", name, 1, MAX_NAME_BYTES)?;
    let value = match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => None,
        other => Some(decode_text("value", other, 0, MAX_VALUE_BYTES)?),
    };
    Ok(TagOp::Set {
        subject,
        name,
        value,
    })
}

/// The fields of a `tag.clear` body, after the version.
fn decode_clear(rest: &[Value<'static>]) -> Result<TagOp, TagError> {
    let [claims] = rest else {
        return Err(TagError::Shape(
            "a tag.clear has a version and a list of claims".to_owned(),
        ));
    };
    let items = claims
        .as_array()
        .map_err(|err| TagError::Shape(format!("the claims are an array: {err}")))?;
    if items.len() > MAX_CLEAR_CLAIMS {
        return Err(TagError::Field(format!(
            "{} claims, the most is {MAX_CLEAR_CLAIMS}",
            items.len()
        )));
    }
    let claims = items
        .iter()
        .map(decode_hash)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TagOp::Clear { claims })
}

/// A 32-byte string, as a hash.
fn decode_hash(value: &Value<'static>) -> Result<Hash, TagError> {
    let Value::ByteString(bytes) = value else {
        return Err(TagError::Shape("an ID is a byte string".to_owned()));
    };
    let bytes: &[u8] = bytes;
    let array = <[u8; 32]>::try_from(bytes)
        .map_err(|_| TagError::Shape(format!("an ID is {} bytes, it must be 32", bytes.len())))?;
    Ok(Hash::from_bytes(array))
}

/// A text field that is NFC and within its byte limits.
fn decode_text(
    field: &str,
    value: &Value<'static>,
    min: usize,
    max: usize,
) -> Result<String, TagError> {
    let Value::TextString(text) = value else {
        return Err(TagError::Shape(format!("the {field} is not text")));
    };
    let text: &str = text;
    check_text(field, text, min, max)?;
    Ok(text.to_owned())
}

/// Checks a text field: its byte length is in `min..=max`, and it is NFC.
fn check_text(field: &str, text: &str, min: usize, max: usize) -> Result<(), TagError> {
    let length = text.len();
    if !(min..=max).contains(&length) {
        return Err(TagError::Field(format!(
            "the {field} is {length} bytes, it must be {min} to {max}"
        )));
    }
    if !is_nfc(text) {
        return Err(TagError::Field(format!("the {field} is not NFC")));
    }
    Ok(())
}

/// The events that `start` reaches by following parent links: its own causal past, transitively.
/// `parents` maps each event to the events it names, its `after` hashes and its backlink. The
/// starting events are included.
#[must_use]
pub fn causal_past(start: &[Hash], parents: &HashMap<Hash, Vec<Hash>>) -> HashSet<Hash> {
    let mut past = HashSet::new();
    let mut queue: Vec<Hash> = start.to_vec();
    while let Some(id) = queue.pop() {
        if !past.insert(id) {
            continue;
        }
        if let Some(named) = parents.get(&id) {
            queue.extend(named.iter().copied());
        }
    }
    past
}

/// The live claims, given the applied tag events and the parent links of every event the store
/// holds. A claim is live unless an applied clear names it and has it in its causal past.
///
/// The result is a function of the set of events, so it does not depend on the order they arrived.
/// It is sorted by claim ID.
#[must_use]
pub fn live_claims(applied: &[Applied], parents: &HashMap<Hash, Vec<Hash>>) -> Vec<Claim> {
    let mut sets: BTreeMap<[u8; 32], Claim> = BTreeMap::new();
    let mut removed: HashSet<Hash> = HashSet::new();
    for event in applied {
        if let TagOp::Set {
            subject,
            name,
            value,
        } = &event.op
        {
            sets.insert(
                *event.id.as_bytes(),
                Claim {
                    id: event.id,
                    author: event.author,
                    subject: *subject,
                    name: name.clone(),
                    value: value.clone(),
                },
            );
        }
    }
    for event in applied {
        let TagOp::Clear { claims } = &event.op else {
            continue;
        };
        // What the clear's author had seen: the causal past of the clear, not counting the clear.
        let seen = parents
            .get(&event.id)
            .map(|named| causal_past(named, parents))
            .unwrap_or_default();
        removed.extend(claims.iter().filter(|claim| seen.contains(claim)));
    }
    sets.into_values()
        .filter(|claim| !removed.contains(&claim.id))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn hash(n: u8) -> Hash {
        Hash::from_bytes([n; 32])
    }

    #[test]
    fn test_a_set_round_trips_with_and_without_a_value() {
        for value in [None, Some(String::new()), Some("Seattle".to_owned())] {
            let op = TagOp::Set {
                subject: hash(7),
                name: "city".to_owned(),
                value,
            };
            let body = encode_op(&op).unwrap();
            assert_eq!(decode_op(SET_KIND, &body).unwrap(), op);
        }
    }

    #[test]
    fn test_a_clear_round_trips_with_no_claims_and_with_many() {
        for count in [0_u8, 1, 3] {
            let op = TagOp::Clear {
                claims: (0..count).map(hash).collect(),
            };
            let body = encode_op(&op).unwrap();
            assert_eq!(decode_op(CLEAR_KIND, &body).unwrap(), op);
        }
    }

    #[test]
    fn test_a_name_outside_its_limits_is_refused() {
        let empty = TagOp::Set {
            subject: hash(1),
            name: String::new(),
            value: None,
        };
        assert!(encode_op(&empty).is_err());
        let long = TagOp::Set {
            subject: hash(1),
            name: "n".repeat(MAX_NAME_BYTES + 1),
            value: None,
        };
        assert!(encode_op(&long).is_err());
    }

    #[test]
    fn test_a_clear_naming_too_many_claims_is_refused() {
        let op = TagOp::Clear {
            claims: (0..=MAX_CLEAR_CLAIMS)
                .map(|n| Hash::digest(n.to_be_bytes()))
                .collect(),
        };
        assert!(encode_op(&op).is_err());
    }

    #[test]
    fn test_a_body_under_the_wrong_kind_is_refused() {
        let body = encode_op(&TagOp::Clear { claims: vec![] }).unwrap();
        assert!(decode_op(SET_KIND, &body).is_err());
        let body = encode_op(&TagOp::Set {
            subject: hash(1),
            name: "a".to_owned(),
            value: None,
        })
        .unwrap();
        assert!(decode_op(CLEAR_KIND, &body).is_err());
        assert!(decode_op("message", &body).is_err());
    }

    #[test]
    fn test_a_body_with_trailing_bytes_is_refused() {
        let mut body = encode_op(&TagOp::Set {
            subject: hash(1),
            name: "a".to_owned(),
            value: None,
        })
        .unwrap();
        body.push(0x00);
        assert!(decode_op(SET_KIND, &body).is_err());
    }

    #[test]
    fn test_a_subject_that_is_not_32_bytes_is_refused() {
        let body = Value::Array(vec![
            Value::Unsigned(VERSION),
            Value::ByteString(vec![1_u8; 31].into()),
            Value::TextString("a".into()),
            Value::null(),
        ])
        .encode();
        assert!(matches!(
            decode_op(SET_KIND, &body),
            Err(TagError::Shape(_))
        ));
    }
}
