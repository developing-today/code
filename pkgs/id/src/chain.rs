//! Event headers: the `ChainExt` extension on p2panda-core's header, and the
//! sign and decode paths for it.
//!
//! Every event is a p2panda `Header<ChainExt>`. p2panda's header already gives
//! each author a chain: the author's key, a sequence number, and a backlink to
//! the previous event. `ChainExt` adds what the membership rules need on top:
//! the event's `kind`, its `group`, and `after`, the author's causal frontier.
//! The domain tag [`DOMAIN_TAG`] is one of the extension's keys, so it is
//! signed with the rest of the header.
//!
//! Receivers call [`decode`] on the received bytes and nothing else. It checks
//! that the bytes are canonical CBOR, then lets `AnyHeader::decode` check the
//! layout and the signature, which uses `verify_strict`. It never calls the
//! typed `Header::verify`, which returns `true` outside test builds, and it
//! never uses a cached digest: the operation ID is BLAKE3 over the received
//! bytes.

use std::collections::HashSet;

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_core::{AnyHeader, Hash, Header, HeaderError, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::documents::{DocumentRef, is_document_kind, is_removal_kind};
use crate::notes::{NoteOp, check_note_op, is_note_kind};
use crate::tag_claims::{TagOp, check_op, is_tag_kind};

/// The domain tag every `ChainExt` carries, so this header type cannot be
/// confused with another protocol's.
pub const DOMAIN_TAG: &str = "id.events.v1";

/// The most hashes an `after` frontier may hold.
pub const MAX_AFTER: usize = 32;

/// The longest `kind`, in UTF-8 bytes. The shortest is one byte.
pub const MAX_KIND_BYTES: usize = 64;

/// Nesting limit for a header's CBOR. Matches the limit p2panda's decoder uses.
const RECURSION_LIMIT: u16 = 64;

/// Declared element limit for one array, map, or string. Matches p2panda's decoder.
const LENGTH_LIMIT: u64 = 512;

/// Byte budget for speculative pre-allocation. Matches p2panda's decoder.
const OOM_MITIGATION: usize = 64;

/// Why an event header, or the extension it carries, was refused.
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    /// `after` holds more hashes than [`MAX_AFTER`].
    #[error("after holds {0} hashes, the most allowed is {max}", max = MAX_AFTER)]
    TooManyAfter(usize),
    /// `after` names the same hash twice.
    #[error("after names the same hash twice")]
    DuplicateAfter,
    /// `kind` is empty or longer than [`MAX_KIND_BYTES`].
    #[error("kind is {0} bytes, it must be 1 to {max} bytes", max = MAX_KIND_BYTES)]
    KindLength(usize),
    /// `kind` is not in Unicode normalization form NFC.
    #[error("kind is not in Unicode NFC")]
    KindNotNfc,
    /// The domain tag is missing, or it is not [`DOMAIN_TAG`].
    #[error("domain tag {0:?} is not {tag:?}", tag = DOMAIN_TAG)]
    Domain(String),
    /// Sequence 0 takes no backlink.
    #[error("sequence 0 takes no backlink")]
    UnexpectedBacklink,
    /// Every sequence after 0 names the author's previous event.
    #[error("sequence {0} needs a backlink")]
    MissingBacklink(SeqNum),
    /// The bytes, or the extension inside them, are not the canonical encoding
    /// of the value they hold.
    #[error("the bytes are not the canonical encoding of their value")]
    NotCanonical,
    /// A CBOR value that signed headers do not allow: a float, a tag, a simple
    /// value other than false, true, or null, or text that is not NFC.
    #[error("CBOR value refused: {0}")]
    CborRule(String),
    /// The extension does not serialize back to its own bytes, or it cannot be
    /// serialized at all.
    #[error("extension refused: {0}")]
    Extension(String),
    /// The bytes are not CBOR, or they are CBOR that strict decoding refuses.
    #[error("CBOR decoding failed: {0}")]
    Cbor(#[from] cbor_core::Error),
    /// p2panda's header decoder refused the bytes: the layout, the version, or the
    /// signature. It also carries an extension that fails its serde form or a
    /// [`ChainExt`] rule, with the rule named in the message.
    #[error(transparent)]
    Header(#[from] HeaderError),
}

/// The extension every event header carries. It is serialized inside the
/// signed header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Wire", into = "Wire")]
pub struct ChainExt {
    /// What the event is: a group operation, a message, a tag, and so on.
    /// Non-empty, NFC, and at most [`MAX_KIND_BYTES`] bytes.
    pub kind: String,
    /// The group the event belongs to. A node can route by it without
    /// decrypting the payload.
    pub group: Option<Hash>,
    /// The author's causal frontier: the latest events from any author that the
    /// author had seen when writing this one. At most [`MAX_AFTER`] hashes, none
    /// repeated.
    pub after: Vec<Hash>,
    /// The tag operation of a `tag.set` or `tag.clear`, and `None` for every other
    /// kind. It is the tag's body (#182): it is signed with the header, so an event
    /// carries its tag, and sync needs no separate body.
    pub tag: Option<TagOp>,
    /// The document of a `document.put`, and `None` for every other kind. It names the content by
    /// hash and size, and the bytes are a blob (P5.4). It is signed with the header.
    pub document: Option<DocumentRef>,
    /// The ID of the `document.put` a `document.remove` removes, and `None` for every other kind.
    /// It is signed with the header (P5.6).
    pub removes: Option<Hash>,
    /// The note operation of a `note.create` or `note.revise`, and `None` for every
    /// other kind. It is the note's body (#3814): signed with the header, so an event
    /// carries its whole note operation, and sync needs no separate body.
    pub note: Option<NoteOp>,
}

/// The wire form of [`ChainExt`]: the same fields, plus the domain tag. Keys
/// this form does not name are refused. `tag`, `document` and `removes` are left
/// out when they are `None`, so events of other kinds keep their encoding.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    domain: String,
    kind: String,
    group: Option<Hash>,
    after: Vec<Hash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag: Option<TagOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document: Option<DocumentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removes: Option<Hash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<NoteOp>,
}

impl From<ChainExt> for Wire {
    fn from(ext: ChainExt) -> Self {
        Self {
            domain: DOMAIN_TAG.to_owned(),
            kind: ext.kind,
            group: ext.group,
            after: ext.after,
            tag: ext.tag,
            document: ext.document,
            removes: ext.removes,
            note: ext.note,
        }
    }
}

impl TryFrom<Wire> for ChainExt {
    type Error = ChainError;

    fn try_from(wire: Wire) -> Result<Self, Self::Error> {
        if wire.domain != DOMAIN_TAG {
            return Err(ChainError::Domain(wire.domain));
        }
        let ext = Self {
            kind: wire.kind,
            group: wire.group,
            after: wire.after,
            tag: wire.tag,
            document: wire.document,
            removes: wire.removes,
            note: wire.note,
        };
        ext.validate()?;
        Ok(ext)
    }
}

impl ChainExt {
    /// Checks the rules every event's extension must meet. Both the builder and
    /// the decoder apply them, so a signed event always passes them on receipt.
    ///
    /// # Errors
    ///
    /// Refuses more than [`MAX_AFTER`] `after` hashes, a hash repeated in
    /// `after`, a `kind` that is empty or longer than [`MAX_KIND_BYTES`], and a
    /// `kind` that is not NFC. A tag kind must carry a tag operation that fits
    /// its kind, and any other kind must carry none. A document kind must carry
    /// a document that passes [`DocumentRef::check`], and any other kind must
    /// carry none. A removal kind must name the document it removes, and any
    /// other kind must name none. A removal carries no tag and no document, as
    /// any kind that is not a tag or a document kind must not.
    pub fn validate(&self) -> Result<(), ChainError> {
        if self.after.len() > MAX_AFTER {
            return Err(ChainError::TooManyAfter(self.after.len()));
        }
        let mut seen = HashSet::with_capacity(self.after.len());
        if !self.after.iter().all(|hash| seen.insert(hash)) {
            return Err(ChainError::DuplicateAfter);
        }
        let length = self.kind.len();
        if !(1..=MAX_KIND_BYTES).contains(&length) {
            return Err(ChainError::KindLength(length));
        }
        if !is_nfc(&self.kind) {
            return Err(ChainError::KindNotNfc);
        }
        match (is_tag_kind(&self.kind), &self.tag) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries a tag operation",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its tag operation",
                    self.kind
                )));
            }
            (true, Some(op)) => {
                check_op(&self.kind, op).map_err(|err| ChainError::Extension(err.to_string()))?;
            }
        }
        match (is_document_kind(&self.kind), &self.document) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries a document",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its document",
                    self.kind
                )));
            }
            (true, Some(document)) => {
                document
                    .check()
                    .map_err(|err| ChainError::Extension(err.to_string()))?;
            }
        }
        match (is_removal_kind(&self.kind), &self.removes) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} names a removed document",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs the document it removes",
                    self.kind
                )));
            }
            (true, Some(_)) => {}
        }
        match (is_note_kind(&self.kind), &self.note) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries a note operation",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its note operation",
                    self.kind
                )));
            }
            (true, Some(op)) => {
                check_note_op(&self.kind, op)
                    .map_err(|err| ChainError::Extension(err.to_string()))?;
            }
        }
        Ok(())
    }
}

/// A signed event header: its encoded bytes, its operation ID, and the decoded
/// header with its extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// The operation ID: BLAKE3 over `bytes`, signature included.
    pub id: Hash,
    /// The encoded header, signature included. This is what is stored and sent.
    pub bytes: Vec<u8>,
    /// The header, with its extension decoded.
    pub header: Header<ChainExt>,
}

/// Builds and signs an event header.
///
/// `seq_num` counts the author's events from 0. Sequence 0 takes no backlink.
/// Every later sequence names the author's previous event's ID. `body` is the
/// payload: the header commits to its BLAKE3 hash, and the body itself is not
/// part of the event.
///
/// # Errors
///
/// Refuses an extension that fails [`ChainExt::validate`], and a backlink that
/// does not match `seq_num`.
pub fn sign(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    body: Option<&[u8]>,
    ext: ChainExt,
) -> Result<Event, ChainError> {
    ext.validate()?;
    match (seq_num, backlink.is_some()) {
        (0, true) => return Err(ChainError::UnexpectedBacklink),
        (seq, false) if seq > 0 => return Err(ChainError::MissingBacklink(seq)),
        _ => {}
    }

    let mut builder = Header::<ChainExt>::builder()
        .seq_num(seq_num)
        .backlink(backlink);
    if let Some(body) = body {
        builder = builder.body(body);
    }
    let header = builder.build(key, ext);
    let bytes = header.encode();
    Ok(Event {
        id: Hash::digest(&bytes),
        bytes,
        header,
    })
}

/// Verifies and decodes received event bytes. Nothing but the bytes is trusted.
///
/// # Errors
///
/// Refuses bytes that are not canonical CBOR (`Cbor` or `NotCanonical`), and
/// bytes that p2panda's header decoder rejects: the layout, the version, or the
/// signature (`Header`). An extension that is not a valid [`ChainExt`] is
/// refused as well. A missing or unknown key, a wrong domain tag, or a failed
/// [`ChainExt::validate`] comes back as `Header`, with the rule in the message.
/// A key that serde would fill with a default, such as `group`, is `NotCanonical`.
pub fn decode(bytes: &[u8]) -> Result<Event, ChainError> {
    let root = canonical_value(bytes)?;
    let header = Header::<ChainExt>::try_from(AnyHeader::decode(bytes)?)?;

    // The extension is the last field of the header. It must serialize back to
    // the same bytes. This refuses a key that serde fills in with a default, such
    // as a missing `group`, which would otherwise decode as `None`.
    let [.., received] = root.as_array()? else {
        return Err(ChainError::Extension("the header has no fields".to_owned()));
    };
    let again = Value::serialized(&header.extensions)
        .map_err(|err| ChainError::Extension(err.to_string()))?;
    if again.encode() != received.encode() {
        return Err(ChainError::NotCanonical);
    }

    Ok(Event {
        id: Hash::digest(bytes),
        bytes: bytes.to_vec(),
        header,
    })
}

/// Decodes strictly, and accepts the bytes only if they are the canonical
/// encoding of the value they hold. The value rules then apply to the whole
/// header.
fn canonical_value(bytes: &[u8]) -> Result<Value<'static>, ChainError> {
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(bytes)?;
    if value.encode() != bytes {
        return Err(ChainError::NotCanonical);
    }
    check_value(&value)?;
    Ok(value)
}

/// The value rules. Text is NFC, map keys included. The only numbers are
/// integers. Floats, tags, and simple values other than false, true, and null
/// are refused.
fn check_value(value: &Value<'_>) -> Result<(), ChainError> {
    match value {
        Value::Unsigned(_) | Value::Negative(_) | Value::ByteString(_) => Ok(()),
        Value::TextString(text) if is_nfc(text) => Ok(()),
        Value::TextString(_) => Err(ChainError::CborRule("text is not NFC".to_owned())),
        Value::Array(items) => items.iter().try_for_each(check_value),
        Value::Map(map) => map.iter().try_for_each(|(key, val)| {
            check_value(key)?;
            check_value(val)
        }),
        Value::SimpleValue(simple) => match simple.to_u8() {
            20..=22 => Ok(()),
            other => Err(ChainError::CborRule(format!("simple value {other}"))),
        },
        Value::Float(_) => Err(ChainError::CborRule("a float".to_owned())),
        Value::Tag(number, _) => Err(ChainError::CborRule(format!("tag {number}"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use p2panda_core::Extensions;

    use super::*;

    /// The test key: the 32 bytes 00 through 1f. It is not a real key.
    fn key() -> SigningKey {
        SigningKey::from_bytes(&std::array::from_fn(|i| u8::try_from(i).unwrap()))
    }

    fn hash(n: u8) -> Hash {
        Hash::digest([n])
    }

    fn ext(kind: &str, group: Option<Hash>, after: Vec<Hash>) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group,
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
        }
    }

    /// `count` distinct hashes: the digests of the counters `0..count`.
    fn distinct(count: usize) -> Vec<Hash> {
        (0..count).map(|n| Hash::digest(n.to_le_bytes())).collect()
    }

    // Extensions built outside `ChainExt`, so the decoder can be shown a header
    // that the builder would never sign. Each one is signed with the test key,
    // so only the decoder's own rules can refuse it.

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct Forged {
        domain: String,
        kind: String,
        group: Option<Hash>,
        after: Vec<Hash>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct NoGroup {
        domain: String,
        kind: String,
        after: Vec<Hash>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct WithExtraKey {
        domain: String,
        kind: String,
        group: Option<Hash>,
        after: Vec<Hash>,
        extra: u8,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct FloatKind {
        domain: String,
        kind: f64,
        group: Option<Hash>,
        after: Vec<Hash>,
    }

    fn forged_form(domain: &str, kind: &str, after: Vec<Hash>) -> Forged {
        Forged {
            domain: domain.to_owned(),
            kind: kind.to_owned(),
            group: None,
            after,
        }
    }

    /// Signs any extension as sequence 0 with no body, the way `sign` would.
    fn sign_raw<E: Extensions>(ext: E) -> Vec<u8> {
        Header::<E>::builder().build(&key(), ext).encode()
    }

    #[test]
    fn test_after_counts_at_the_limit_are_accepted() {
        for count in [0, 1, MAX_AFTER] {
            let event = sign(&key(), 0, None, None, ext("m", None, distinct(count)))
                .expect("a frontier within the limit is signed");
            assert_eq!(decode(&event.bytes).unwrap(), event, "after {count}");
        }
    }

    #[test]
    fn test_after_one_past_the_limit_is_refused() {
        let after = distinct(MAX_AFTER + 1);
        assert!(matches!(
            sign(&key(), 0, None, None, ext("m", None, after.clone())),
            Err(ChainError::TooManyAfter(33))
        ));
        assert!(matches!(
            ext("m", None, after.clone()).validate(),
            Err(ChainError::TooManyAfter(33))
        ));
        // A header signed outside `sign` with the same frontier is refused too.
        let forged = sign_raw(Forged {
            domain: DOMAIN_TAG.to_owned(),
            kind: "m".to_owned(),
            group: None,
            after,
        });
        assert!(decode(&forged).is_err());
    }

    #[test]
    fn test_repeated_after_hashes_are_refused() {
        let twice = vec![hash(1), hash(2), hash(1)];
        assert!(matches!(
            ext("m", None, twice).validate(),
            Err(ChainError::DuplicateAfter)
        ));
        let adjacent = vec![hash(7), hash(7)];
        assert!(matches!(
            sign(&key(), 0, None, None, ext("m", None, adjacent)),
            Err(ChainError::DuplicateAfter)
        ));
        assert!(ext("m", None, distinct(3)).validate().is_ok());
    }

    #[test]
    fn test_kind_length_bounds() {
        assert!(matches!(
            ext("", None, vec![]).validate(),
            Err(ChainError::KindLength(0))
        ));
        let longest = "a".repeat(MAX_KIND_BYTES);
        assert!(ext(&longest, None, vec![]).validate().is_ok());
        let one_over = "a".repeat(MAX_KIND_BYTES + 1);
        assert!(matches!(
            ext(&one_over, None, vec![]).validate(),
            Err(ChainError::KindLength(65))
        ));
        // Measured in bytes, not characters: 32 two-byte letters is 64 bytes.
        let multibyte = "é".repeat(32);
        assert_eq!(multibyte.len(), MAX_KIND_BYTES);
        assert!(ext(&multibyte, None, vec![]).validate().is_ok());
        let multibyte_over = "é".repeat(33);
        assert!(matches!(
            ext(&multibyte_over, None, vec![]).validate(),
            Err(ChainError::KindLength(66))
        ));
    }

    #[test]
    fn test_kind_must_be_nfc() {
        // "e" followed by a combining acute accent is not NFC. The precomposed
        // letter is.
        assert!(matches!(
            ext("e\u{301}", None, vec![]).validate(),
            Err(ChainError::KindNotNfc)
        ));
        assert!(ext("\u{e9}", None, vec![]).validate().is_ok());
        assert!(sign(&key(), 0, None, None, ext("e\u{301}", None, vec![])).is_err());
    }

    #[test]
    fn test_group_none_and_some_round_trip() {
        for group in [None, Some(hash(9))] {
            let event = sign(
                &key(),
                0,
                None,
                None,
                ext("group.policy", group, distinct(2)),
            )
            .unwrap();
            let decoded = decode(&event.bytes).unwrap();
            assert_eq!(decoded.header.extensions.group, group);
            assert_eq!(decoded.header.extensions.kind, "group.policy");
            assert_eq!(decoded.header.extensions.after, distinct(2));
        }
    }

    #[test]
    fn test_sequence_and_backlink_must_agree() {
        assert!(matches!(
            sign(&key(), 0, Some(hash(1)), None, ext("m", None, vec![])),
            Err(ChainError::UnexpectedBacklink)
        ));
        assert!(matches!(
            sign(&key(), 1, None, None, ext("m", None, vec![])),
            Err(ChainError::MissingBacklink(1))
        ));
        let second = sign(&key(), 1, Some(hash(1)), None, ext("m", None, vec![])).unwrap();
        assert_eq!(second.header.seq_num, 1);
        assert_eq!(second.header.backlink, Some(hash(1)));
    }

    #[test]
    fn test_a_signed_event_decodes_to_the_same_event() {
        let body = b"payload bytes";
        let event = sign(
            &key(),
            0,
            None,
            Some(body),
            ext("message", Some(hash(3)), distinct(5)),
        )
        .unwrap();
        assert_eq!(event.id, Hash::digest(&event.bytes));
        assert_eq!(event.header.payload_hash, Some(Hash::digest(body)));
        assert_eq!(decode(&event.bytes).unwrap(), event);
    }

    #[test]
    fn test_signing_the_same_event_twice_gives_the_same_bytes() {
        let make = || sign(&key(), 0, None, None, ext("m", None, distinct(4))).unwrap();
        assert_eq!(make().bytes, make().bytes);
    }

    #[test]
    fn test_domain_tag_is_checked() {
        let bytes = sign_raw(forged_form("id.events.v0", "m", vec![]));
        let err = decode(&bytes).unwrap_err();
        assert!(
            err.to_string().contains("domain tag"),
            "unexpected error: {err}"
        );
        // A header with no domain key at all is refused too.
        let bytes = sign_raw(NoGroup {
            domain: String::new(),
            kind: "m".to_owned(),
            after: vec![],
        });
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn test_a_missing_group_key_is_refused_as_not_canonical() {
        // serde would decode a missing `group` as `None`. The re-encoding then
        // differs from the received bytes, so the decoder refuses them.
        let bytes = sign_raw(NoGroup {
            domain: DOMAIN_TAG.to_owned(),
            kind: "m".to_owned(),
            after: vec![],
        });
        assert!(matches!(decode(&bytes), Err(ChainError::NotCanonical)));
    }

    #[test]
    fn test_an_unknown_extension_key_is_refused() {
        let bytes = sign_raw(WithExtraKey {
            domain: DOMAIN_TAG.to_owned(),
            kind: "m".to_owned(),
            group: None,
            after: vec![],
            extra: 1,
        });
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn test_a_float_where_text_belongs_is_refused() {
        let bytes = sign_raw(FloatKind {
            domain: DOMAIN_TAG.to_owned(),
            kind: 1.5,
            group: None,
            after: vec![],
        });
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn test_a_header_without_an_extension_is_refused() {
        let bytes = Header::<()>::builder().build(&key(), ()).encode();
        assert!(matches!(
            decode(&bytes),
            Err(ChainError::Header(HeaderError::MissingExtensions))
        ));
    }

    #[test]
    fn test_a_changed_signature_is_refused() {
        let event = sign(&key(), 0, None, None, ext("m", None, distinct(2))).unwrap();
        let mut bytes = event.bytes;
        // The signature is the byte string after the key: 0x58 0x40, then 64 bytes.
        assert_eq!(bytes[37], 0x40, "layout changed, fix the offset");
        bytes[48] ^= 0x01;
        assert!(matches!(
            decode(&bytes),
            Err(ChainError::Header(HeaderError::InvalidSignature))
        ));
    }

    #[test]
    fn test_a_non_shortest_integer_is_refused() {
        // Sequence 0 written as 0x18 0x00 instead of 0x00. The strict decoder
        // refuses the encoding before any signature check.
        let event = sign(&key(), 0, None, None, ext("m", None, vec![])).unwrap();
        assert_eq!(event.bytes[103], 0x00, "layout changed, fix the offset");
        let mut bytes = event.bytes[..103].to_vec();
        bytes.extend([0x18, 0x00]);
        bytes.extend(&event.bytes[104..]);
        assert!(matches!(decode(&bytes), Err(ChainError::Cbor(_))));
    }

    #[test]
    fn test_empty_and_truncated_input_is_refused() {
        assert!(decode(&[]).is_err());
        let event = sign(&key(), 0, None, None, ext("m", None, distinct(2))).unwrap();
        for cut in [1, event.bytes.len() / 2, event.bytes.len() - 1] {
            assert!(decode(&event.bytes[..cut]).is_err(), "cut at {cut}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod golden {
    //! Golden vectors and the dcbor cross-check. The S5 values are the ones the spike
    //! recorded (`spikes/shared/s5-cbor/RESULTS.md` and `run-output.md`). dcbor's encoder is
    //! independent of cbor-core, so agreement between them is evidence that the canonical
    //! bytes are right, not only that they are stable.

    use std::collections::BTreeMap;

    use cbor_core::Value;
    use dcbor::{ByteString, CBOR};
    use p2panda_core::{AnyHeader, Hash, Header, SigningKey};

    use super::{decode, sign, ChainExt, Event, DOMAIN_TAG};

    /// The test key: the 32 bytes 00 through 1f. It is not a real key.
    fn key() -> SigningKey {
        SigningKey::from_bytes(&std::array::from_fn(|i| u8::try_from(i).unwrap()))
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut text, byte| {
            write!(text, "{byte:02x}").unwrap();
            text
        })
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// `{"kind": "group.policy", "group": "g1", "level": 3}`, the S5 golden body.
    fn plain_body_map() -> Value<'static> {
        Value::from(BTreeMap::from([
            (Value::from("kind"), Value::from("group.policy")),
            (Value::from("group"), Value::from("g1")),
            (Value::from("level"), Value::Unsigned(3)),
        ]))
    }

    /// `{"blob": h'00ff10', "empty": h'', "hash": blake3("x")}`
    fn body_with_byte_strings() -> Value<'static> {
        Value::from(BTreeMap::from([
            (Value::from("blob"), Value::from(vec![0x00u8, 0xff, 0x10])),
            (Value::from("empty"), Value::from(Vec::<u8>::new())),
            (
                Value::from("hash"),
                Value::from(Hash::digest(b"x").as_bytes().to_vec()),
            ),
        ]))
    }

    /// `[blake3("a"), blake3("b"), blake3("c")]`, the shape of an `after` frontier.
    fn hash_array() -> Value<'static> {
        let hashes: Vec<Value<'_>> = [b"a", b"b", b"c"]
            .iter()
            .map(|s| Value::from(Hash::digest(s).as_bytes().to_vec()))
            .collect();
        Value::from(hashes)
    }

    /// `{"a": null, "outer": {"inner": [1, -2, {"deep": true}], "tag": h'ab'}, "z": "last"}`
    fn nested() -> Value<'static> {
        let deep = Value::from(BTreeMap::from([(Value::from("deep"), Value::from(true))]));
        let inner = Value::from(vec![Value::Unsigned(1), Value::Negative(1), deep]);
        let outer = Value::from(BTreeMap::from([
            (Value::from("inner"), inner),
            (Value::from("tag"), Value::from(vec![0xabu8])),
        ]));
        Value::from(BTreeMap::from([
            (Value::from("a"), Value::null()),
            (Value::from("outer"), outer),
            (Value::from("z"), Value::from("last")),
        ]))
    }

    #[test]
    fn test_s5_canonical_vectors_match_the_recorded_bytes() {
        let cases = [
            (
                plain_body_map(),
                "a3646b696e646c67726f75702e706f6c6963796567726f7570626731656c6576656c03",
            ),
            (
                body_with_byte_strings(),
                "a364626c6f624300ff10646861736858203ae7d805f6789a6402acb70ad4096a85a56bf6804eaf25c0493ac697548d30b565656d70747940",
            ),
            (
                hash_array(),
                "83582017762fddd969a453925d65717ac3eea21320b66b54342fde15128d6caf21215f582010e5cf3d3c8a4f9f3468c8cc58eea84892a22fdadbc1acb22410190044c1d5535820ea7aa1fc9efdbe106dbb70369a75e9671fa29d52bd55536711bf197477b8f021",
            ),
            (
                nested(),
                "a36161f6617a646c617374656f75746572a26374616741ab65696e6e6572830121a16464656570f5",
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(hex(&value.encode()), expected);
        }
    }

    #[test]
    fn test_dcbor_agrees_with_cbor_core_on_the_s5_values() {
        let mut plain = dcbor::Map::new();
        plain.insert("kind", "group.policy");
        plain.insert("group", "g1");
        plain.insert("level", 3u64);
        assert_eq!(CBOR::from(plain).to_cbor_data(), plain_body_map().encode());

        let mut bytes = dcbor::Map::new();
        bytes.insert("blob", ByteString::from(vec![0x00u8, 0xff, 0x10]));
        bytes.insert("empty", ByteString::from(Vec::<u8>::new()));
        bytes.insert(
            "hash",
            ByteString::from(Hash::digest(b"x").as_bytes().to_vec()),
        );
        assert_eq!(
            CBOR::from(bytes).to_cbor_data(),
            body_with_byte_strings().encode()
        );

        let hashes: Vec<CBOR> = [b"a", b"b", b"c"]
            .iter()
            .map(|s| ByteString::from(Hash::digest(s).as_bytes().to_vec()).into())
            .collect();
        assert_eq!(CBOR::from(hashes).to_cbor_data(), hash_array().encode());

        let mut deep = dcbor::Map::new();
        deep.insert("deep", true);
        let inner = CBOR::from(vec![CBOR::from(1u64), CBOR::from(-2i64), CBOR::from(deep)]);
        let mut outer = dcbor::Map::new();
        outer.insert("inner", inner);
        outer.insert("tag", ByteString::from(vec![0xabu8]));
        let mut root = dcbor::Map::new();
        root.insert("a", CBOR::null());
        root.insert("outer", outer);
        root.insert("z", "last");
        assert_eq!(CBOR::from(root).to_cbor_data(), nested().encode());
    }

    /// The S5 golden header: `Header<()>` with the plain body map as its body, seq 0,
    /// and the test key. It has no extension, so it pins p2panda's header layout and its
    /// signature. It does not pin `ChainExt`, which has its own vectors below.
    #[test]
    fn test_s5_golden_header_keeps_its_operation_id_and_signature() {
        const HEADER: &str = "8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b85840be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d00";
        const OPERATION_ID: &str =
            "90acd5f7c498ccb93dc33ce41f49d166140bef38c1f85cb95940f311b6e7f212";
        const SIGNATURE: &str = "be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f";

        let body = plain_body_map().encode();
        let header = Header::<()>::builder()
            .body(&body)
            .seq_num(0)
            .build(&key(), ());
        let bytes = header.encode();

        assert_eq!(hex(&bytes), HEADER);
        assert_eq!(Hash::digest(&bytes).to_hex(), OPERATION_ID);
        assert_eq!(header.signature.to_hex(), SIGNATURE);
        // The receive path accepts the same bytes and reads the same operation ID.
        assert_eq!(
            AnyHeader::decode(&bytes).unwrap().hash().to_hex(),
            OPERATION_ID
        );
    }

    /// The two `ChainExt` events the golden tests pin. The second one has a body, a
    /// sequence number above 0, a backlink, and a group.
    fn golden_events() -> (Event, Event) {
        let first = sign(
            &key(),
            0,
            None,
            None,
            ChainExt {
                kind: "group.policy".to_owned(),
                group: None,
                after: vec![Hash::digest(b"a"), Hash::digest(b"b")],
                tag: None,
                document: None,
                removes: None,
                note: None,
            },
        )
        .unwrap();
        let body = plain_body_map().encode();
        let second = sign(
            &key(),
            1,
            Some(first.id),
            Some(body.as_slice()),
            ChainExt {
                kind: "message".to_owned(),
                group: Some(Hash::digest(b"g")),
                after: vec![first.id],
                tag: None,
                document: None,
                removes: None,
                note: None,
            },
        )
        .unwrap();
        (first, second)
    }

    #[test]
    fn test_chain_ext_golden_vectors_are_pinned() {
        const FIRST: &str = "8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8584029353dc14afda7aced2938ade60cdee76fbe9eea7adb77765a8e6dc52a6d532d1ed83d4a54d329389048e32f9106dd658dd721abae9b2a95ef56f104684f06060000a4646b696e646c67726f75702e706f6c69637965616674657282582017762fddd969a453925d65717ac3eea21320b66b54342fde15128d6caf21215f582010e5cf3d3c8a4f9f3468c8cc58eea84892a22fdadbc1acb22410190044c1d5536567726f7570f666646f6d61696e6c69642e6576656e74732e7631";
        const FIRST_ID: &str = "58035ddebae67c7e3424eff97938d84e176bf6df4dc90cfb625b2c0d8d2e4c40";
        const FIRST_SIGNATURE: &str = "29353dc14afda7aced2938ade60cdee76fbe9eea7adb77765a8e6dc52a6d532d1ed83d4a54d329389048e32f9106dd658dd721abae9b2a95ef56f104684f0606";
        const SECOND: &str = "8801582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b858401026e903a382f1589cc4a8ea70ff13b54aba5fb5c2144b0afdba7c79dc09e42c6e29930ef099d0dc5a741bad4b984b110766c5e4e3dacfa627a1ed4b2a7fc405182358209f9175fb55f67e7673c1613e4ba07360afb584a26407f3ffc6dfe5ae52fddf4d01582058035ddebae67c7e3424eff97938d84e176bf6df4dc90cfb625b2c0d8d2e4c40a4646b696e64676d65737361676565616674657281582058035ddebae67c7e3424eff97938d84e176bf6df4dc90cfb625b2c0d8d2e4c406567726f75705820805a31dee1a2a0d3fbe08c612de4fe6be78166a3c4d3a4db7805ad839ed47d4d66646f6d61696e6c69642e6576656e74732e7631";
        const SECOND_ID: &str = "847ec6fba8b5eb1ee230bf9068590f502b6c2d40a14689b30e5fa8d6353c0064";
        const SECOND_SIGNATURE: &str = "1026e903a382f1589cc4a8ea70ff13b54aba5fb5c2144b0afdba7c79dc09e42c6e29930ef099d0dc5a741bad4b984b110766c5e4e3dacfa627a1ed4b2a7fc405";

        let (first, second) = golden_events();
        assert_eq!(hex(&first.bytes), FIRST);
        assert_eq!(first.id.to_hex(), FIRST_ID);
        assert_eq!(first.header.signature.to_hex(), FIRST_SIGNATURE);
        assert_eq!(hex(&second.bytes), SECOND);
        assert_eq!(second.id.to_hex(), SECOND_ID);
        assert_eq!(second.header.signature.to_hex(), SECOND_SIGNATURE);
        assert_eq!(decode(&unhex(FIRST)).unwrap(), first);
        assert_eq!(decode(&unhex(SECOND)).unwrap(), second);
    }

    /// dcbor builds the same two headers from its own encoder. Each header must match
    /// cbor-core's bytes, and each signature must verify over dcbor's signed bytes.
    #[test]
    fn test_dcbor_builds_the_same_chain_ext_headers() {
        let (first, second) = golden_events();
        let public = key().verifying_key();
        let author = ByteString::from(public.as_bytes().to_vec());
        let body = plain_body_map().encode();
        let group = Hash::digest(b"g");

        // The extension as dcbor writes it. dcbor sorts the keys, as RFC 8949 requires.
        let wire = |kind: &str, group: Option<&Hash>, after: &[Hash]| {
            let mut map = dcbor::Map::new();
            map.insert("domain", DOMAIN_TAG);
            map.insert("kind", kind);
            map.insert(
                "group",
                group.map_or_else(CBOR::null, |g| {
                    ByteString::from(g.as_bytes().to_vec()).into()
                }),
            );
            let hashes: Vec<CBOR> = after
                .iter()
                .map(|h| ByteString::from(h.as_bytes().to_vec()).into())
                .collect();
            map.insert("after", CBOR::from(hashes));
            CBOR::from(map)
        };
        let signature = |event: &Event| -> CBOR {
            ByteString::from(event.header.signature.to_bytes().to_vec()).into()
        };

        // First event: no body, so no payload size or hash. Sequence 0, so no backlink.
        let signed = CBOR::from(vec![
            CBOR::from(1u64),
            author.clone().into(),
            CBOR::from(0u64),
            CBOR::from(0u64),
            wire(
                "group.policy",
                None,
                &[Hash::digest(b"a"), Hash::digest(b"b")],
            ),
        ])
        .to_cbor_data();
        let full = CBOR::from(vec![
            CBOR::from(1u64),
            author.clone().into(),
            signature(&first),
            CBOR::from(0u64),
            CBOR::from(0u64),
            wire(
                "group.policy",
                None,
                &[Hash::digest(b"a"), Hash::digest(b"b")],
            ),
        ])
        .to_cbor_data();
        assert_eq!(full, first.bytes);
        assert!(public.verify(&signed, &first.header.signature));

        // Second event: the body's size and hash, sequence 1, and the first event's ID as
        // the backlink.
        let body_len = u64::try_from(body.len()).unwrap();
        let signed = CBOR::from(vec![
            CBOR::from(1u64),
            author.clone().into(),
            CBOR::from(body_len),
            ByteString::from(Hash::digest(&body).as_bytes().to_vec()).into(),
            CBOR::from(1u64),
            ByteString::from(first.id.as_bytes().to_vec()).into(),
            wire("message", Some(&group), &[first.id]),
        ])
        .to_cbor_data();
        let full = CBOR::from(vec![
            CBOR::from(1u64),
            author.into(),
            signature(&second),
            CBOR::from(body_len),
            ByteString::from(Hash::digest(&body).as_bytes().to_vec()).into(),
            CBOR::from(1u64),
            ByteString::from(first.id.as_bytes().to_vec()).into(),
            wire("message", Some(&group), &[first.id]),
        ])
        .to_cbor_data();
        assert_eq!(full, second.bytes);
        assert!(public.verify(&signed, &second.header.signature));
    }
}
