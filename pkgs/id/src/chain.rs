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

use crate::chat::{ChatMessage, is_chat_message_kind};
use crate::documents::{DocumentRef, is_document_kind, is_removal_kind};
use crate::issuer_rotation::{IssuerRotation, check_rotation, is_rotation_kind};
use crate::note_links::{NoteLinkOp, check_note_link_op, is_note_link_kind};
use crate::notes::{NoteOp, check_note_op, is_note_kind};
use crate::session_checkpoint::{SessionCheckpoint, check_session_checkpoint, is_checkpoint_kind};
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
    /// The note link operation of a `note.link`, and `None` for every other kind.
    /// It is the link body (#3816): signed with the header, so an event carries
    /// its whole link operation, and sync needs no separate body.
    pub link: Option<NoteLinkOp>,
    /// The session checkpoint of a `session.checkpoint`, and `None` for every
    /// other kind. It is the checkpoint body (#3794): the session header and
    /// the segment this checkpoint closes, signed with the header, so an
    /// event carries its whole checkpoint, and sync needs no separate body.
    pub checkpoint: Option<SessionCheckpoint>,
    /// The issuer rotation of an `issuer.rotation` event, and `None` for every
    /// other kind. It is the rotation record (#5526, design #2804 section 3.2): signed
    /// with the header and persisted with it, because the event store keeps the
    /// header but not an event body, so an event carries its whole rotation.
    pub rotation: Option<IssuerRotation>,
    /// The chat message of a `chat.message.v1`, and `None` for every other kind. It is the
    /// message's body (#3799): it is signed with the header, so an event carries its
    /// message, and sync needs no separate body.
    pub chat: Option<ChatMessage>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<NoteLinkOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checkpoint: Option<SessionCheckpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rotation: Option<IssuerRotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat: Option<ChatMessage>,
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
            link: ext.link,
            checkpoint: ext.checkpoint,
            rotation: ext.rotation,
            chat: ext.chat,
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
            link: wire.link,
            checkpoint: wire.checkpoint,
            rotation: wire.rotation,
            chat: wire.chat,
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
    /// A session checkpoint kind must carry a session checkpoint that passes
    /// [`check_session_checkpoint`], and any other kind must carry none.
    /// A rotation kind must carry an issuer rotation that passes
    /// [`check_rotation`], and any other kind must carry none.
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
        match (is_note_link_kind(&self.kind), &self.link) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries a note link operation",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its note link operation",
                    self.kind
                )));
            }
            (true, Some(op)) => {
                check_note_link_op(&self.kind, op)
                    .map_err(|err| ChainError::Extension(err.to_string()))?;
            }
        }
        match (is_checkpoint_kind(&self.kind), &self.checkpoint) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries a session checkpoint",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its session checkpoint",
                    self.kind
                )));
            }
            (true, Some(checkpoint)) => {
                check_session_checkpoint(&self.kind, checkpoint)
                    .map_err(|err| ChainError::Extension(err.to_string()))?;
            }
        }
        match (is_rotation_kind(&self.kind), &self.rotation) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries an issuer rotation",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its issuer rotation",
                    self.kind
                )));
            }
            (true, Some(rotation)) => {
                check_rotation(&self.kind, rotation)
                    .map_err(|err| ChainError::Extension(err.to_string()))?;
            }
        }
        match (is_chat_message_kind(&self.kind), &self.chat) {
            (false, None) => {}
            (false, Some(_)) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} carries a chat message",
                    self.kind
                )));
            }
            (true, None) => {
                return Err(ChainError::Extension(format!(
                    "kind {:?} needs its chat message",
                    self.kind
                )));
            }
            (true, Some(message)) => {
                message
                    .check()
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
    use crate::currency::issuer_id;
    use crate::issuer_rotation::{ISSUER_ROTATION_KIND, RotationMode};

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
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
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

    /// The successor key for the rotation tests: the 32 bytes of 9s.
    fn successor_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    /// A rotation extension carrying `rotation` on the rotation kind.
    fn rotation_ext(rotation: IssuerRotation) -> ChainExt {
        ChainExt {
            rotation: Some(rotation),
            ..ext(ISSUER_ROTATION_KIND, None, vec![])
        }
    }

    /// A well-formed routine rotation out of generation 0, proven by
    /// its successor. `key()` authors it; the successor is
    /// [`successor_key`].
    fn proven_rotation() -> IssuerRotation {
        let successor = successor_key();
        let mut rotation = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&key()),
            hash(42),
            0,
            None,
            issuer_id(&successor),
            None,
        );
        rotation
            .prove_successor(&successor)
            .expect("prove successor");
        rotation
    }

    #[test]
    fn test_rotation_well_formed_signs_and_decodes() {
        let rotation = proven_rotation();
        let event = sign(&key(), 0, None, None, rotation_ext(rotation.clone()))
            .expect("a well-formed rotation is signed");
        assert_eq!(decode(&event.bytes).unwrap(), event);
        assert_eq!(event.header.extensions.rotation, Some(rotation));
    }

    #[test]
    fn test_rotation_malformed_bodies_are_refused_at_sign_and_decode() {
        // Non-consecutive generations: 0 -> 2, proven as built, so only
        // the generation rule can refuse it.
        let successor = successor_key();
        let mut skipped = proven_rotation();
        skipped.to_generation = 2;
        skipped
            .prove_successor(&successor)
            .expect("prove successor");

        // A generation-0 predecessor: the initial transition names a
        // previous rotation.
        let mut predecessor = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&key()),
            hash(42),
            0,
            Some(hash(1)),
            issuer_id(&successor),
            None,
        );
        predecessor
            .prove_successor(&successor)
            .expect("prove successor");

        // A later transition naming no predecessor.
        let mut missing_predecessor = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&key()),
            hash(42),
            1,
            None,
            issuer_id(&successor),
            None,
        );
        missing_predecessor
            .prove_successor(&successor)
            .expect("prove successor");

        // A routine rotation asserting a compromise start, which only
        // recovery rotations may carry.
        let mut asserted = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&key()),
            hash(42),
            0,
            None,
            issuer_id(&successor),
            Some(hash(2)),
        );
        asserted
            .prove_successor(&successor)
            .expect("prove successor");

        // A zeroed successor proof, never countersigned.
        let zeroed = IssuerRotation::unsigned(
            RotationMode::Routine,
            issuer_id(&key()),
            hash(42),
            0,
            None,
            issuer_id(&successor),
            None,
        );

        // A successor proof made by a key that is not the successor.
        let mut forged_proof = proven_rotation();
        forged_proof
            .prove_successor(&key())
            .expect("prove with the wrong key");

        for (name, rotation) in [
            ("non-consecutive generation", skipped),
            ("generation-0 predecessor", predecessor),
            ("missing predecessor", missing_predecessor),
            ("routine asserted compromise", asserted),
            ("zeroed successor proof", zeroed),
            ("forged successor proof", forged_proof),
        ] {
            let ext = rotation_ext(rotation);
            assert!(
                matches!(
                    sign(&key(), 0, None, None, ext.clone()),
                    Err(ChainError::Extension(_))
                ),
                "{name}: sign refuses the malformed rotation"
            );
            // A header signed outside `sign` with the same body is
            // refused at decode too.
            let forged = sign_raw(ext);
            assert!(decode(&forged).is_err(), "{name}: decode refuses it");
        }
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

    /// A `session.checkpoint` event from fixed values, for the golden vector
    /// below: the test key's session, two participants, segment 0 with an
    /// audit blob.
    fn checkpoint_event() -> Event {
        use crate::session_checkpoint::{MacScope, SegmentCheckpoint, SessionHeader};

        let author = key();
        let other = SigningKey::from_bytes(&[7; 32]);
        sign(
            &author,
            0,
            None,
            None,
            ChainExt {
                kind: "session.checkpoint".to_owned(),
                group: None,
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
                note: None,
                link: None,
                checkpoint: Some(SessionCheckpoint {
                    header: SessionHeader {
                        session_id: hash(1),
                        purpose: "collab editing".to_owned(),
                        participants: vec![author.verifying_key(), other.verifying_key()],
                        started_ms: 1_000,
                        nonce: hash(2),
                        mac_key_commitment: hash(3),
                        mac_scope: MacScope::PerParticipant,
                    },
                    segment: SegmentCheckpoint {
                        segment_seq: 0,
                        prev_checkpoint: None,
                        root: hash(4),
                        event_count: 256,
                        start_ms: 1_000,
                        end_ms: 2_800,
                        audit_blob: Some(hash(5)),
                    },
                }),
                rotation: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn test_session_checkpoint_golden_vector_is_pinned() {
        const BYTES: &str = "8601582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8584044bc9d648ffd9e02e2aab846e028b4581a49c4832be635ddacc2a5d3bd31f59a3395c4e85090336563040c8e24cc60bf15ce1a6e987d6ac474f99ac1cb93c50d0000a5646b696e647273657373696f6e2e636865636b706f696e74656166746572806567726f7570f666646f6d61696e6c69642e6576656e74732e76316a636865636b706f696e74a266686561646572a7656e6f6e63655820ab13bedf42e84bae0f7c62c7dd6a8ada571e8829bed6ea558217f0361b5e25d067707572706f73656e636f6c6c61622065646974696e67696d61635f73636f70656e5065725061727469636970616e746a73657373696f6e5f6964582048fc721fbbc172e0925fa27af1671de225ba927134802998b10a1568a188652b6a737461727465645f6d731903e86c7061727469636970616e747382582003a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b85820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c726d61635f6b65795f636f6d6d69746d656e745820e1e0e81d6ea39b0cf8b86ffd440921011f57400cbc3f76a8a171906a9b8d7505677365676d656e74a764726f6f7458200c389a743e34fda435fbd575bb889dbc0d3e66b9f9d81e00be33b7188509e7eb66656e645f6d73190af06873746172745f6d731903e86a61756469745f626c6f62582084cb40e74f0e856bb4bb91233e3cb74113533dca78a74f36f59edaa41895c9466b6576656e745f636f756e741901006b7365676d656e745f736571006f707265765f636865636b706f696e74f6";
        const ID: &str = "4f6c1d23b5b47657e5e5a4d39da727c2613b8cfc00af23453934f44749845fa7";

        let event = checkpoint_event();
        assert_eq!(hex(&event.bytes), BYTES);
        assert_eq!(event.id.to_hex(), ID);
        assert_eq!(decode(&unhex(BYTES)).unwrap(), event);
    }

    #[test]
    fn test_a_checkpoint_kind_without_its_checkpoint_is_refused() {
        let err = sign(
            &key(),
            0,
            None,
            None,
            ext("session.checkpoint", None, Vec::new()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("session checkpoint"), "{err}");
    }

    #[test]
    fn test_a_checkpoint_on_another_kind_is_refused() {
        let mut ext = checkpoint_event().header.extensions.clone();
        ext.kind = "message".to_owned();
        let err = sign(&key(), 0, None, None, ext).unwrap_err();
        assert!(err.to_string().contains("session checkpoint"), "{err}");
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

    use super::{ChainExt, DOMAIN_TAG, Event, decode, sign};

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
                link: None,
                checkpoint: None,
                rotation: None,
                chat: None,
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
                link: None,
                checkpoint: None,
                rotation: None,
                chat: None,
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
