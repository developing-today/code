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
}

/// The wire form of [`ChainExt`]: the same fields, plus the domain tag. Keys
/// this form does not name are refused.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    domain: String,
    kind: String,
    group: Option<Hash>,
    after: Vec<Hash>,
}

impl From<ChainExt> for Wire {
    fn from(ext: ChainExt) -> Self {
        Self {
            domain: DOMAIN_TAG.to_owned(),
            kind: ext.kind,
            group: ext.group,
            after: ext.after,
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
    /// `kind` that is not NFC.
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
