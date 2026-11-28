//! Chat anchors: typed conversation-to-object links as signed events (#5872, #5870, #6222).
//!
//! A conversation is a channel group, exactly as in chat v1. There is no talk-page
//! entity: a talk page is a conversation whose log carries a `talk-for` binding. This
//! module defines the additive `chat.anchor.v1` event and the pure fold that derives
//! conversation links and the one canonical talk-page binding from a set of those
//! events.
//!
//! An anchor names its target by [`ObjectRef`] (object ID plus author key) and, for a
//! part anchor, by a [`PartAnchor`] that names a revision and a by-reference selector.
//! It never embeds object content, a preview, or a capability. The selector grammar is
//! the #5870 `pa1` fragment form, implemented by [`PartSelector::encode_v1`] and
//! [`PartSelector::parse_v1`].
//!
//! The fold is a function of the event set alone: delivery order does not matter,
//! duplicate delivery is idempotent, and concurrent maxima are preserved as a visible
//! [`LinkState::Disputed`] / [`TalkBindingState::Disputed`] state instead of being
//! silently resolved by event-ID order. Event-ID order only makes the presentation
//! deterministic.

use std::collections::{BTreeMap, HashMap, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

/// Decodes bytes from either hex text or a raw byte string.
///
/// A real deserializer answers `is_human_readable` honestly, but serde's buffered
/// `Content` deserializer — which an internally tagged enum like [`AnchorOp`] forces on
/// its fields — always claims to be human readable, even when the buffered value is a
/// CBOR byte string. Reading `deserialize_any` instead of trusting that flag accepts
/// both representations, so nested hashes survive the tag.
fn decode_either<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct EitherBytes;

    impl<'de> serde::de::Visitor<'de> for EitherBytes {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a hex string or a byte array")
        }

        fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
            Ok(value.to_vec())
        }

        fn visit_byte_buf<E: serde::de::Error>(self, value: Vec<u8>) -> Result<Self::Value, E> {
            Ok(value)
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
            hex::decode(value).map_err(E::custom)
        }
    }

    deserializer.deserialize_any(EitherBytes)
}

/// A 32-byte key that rides CBOR as raw bytes and JSON as hex, the way p2panda's own
/// hashes do (see its `serialize_hex`).
pub(crate) mod hex_bytes {
    use serde::{Serialize, Serializer};

    /// Serializes 32 bytes as hex for humans, as raw bytes for CBOR.
    pub(crate) fn serialize<S>(value: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            hex::serde::serialize(value, serializer)
        } else {
            serde_bytes::Bytes::new(value).serialize(serializer)
        }
    }

    /// Deserializes from hex or from raw bytes, whichever the codec carries.
    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = super::decode_either(deserializer)?;
        bytes
            .as_slice()
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A [`Hash`] inside an internally tagged enum, where the tag buffering hides whether
/// the codec is human readable; [`hex_bytes`] explains why that matters.
pub(crate) mod hash_bytes {
    use p2panda_core::Hash;
    use serde::{Serialize, Serializer};

    /// Serializes the hash as hex for humans, as raw bytes for CBOR.
    pub(crate) fn serialize<S>(value: &Hash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            hex::serde::serialize(value.as_bytes(), serializer)
        } else {
            serde_bytes::Bytes::new(value.as_bytes()).serialize(serializer)
        }
    }

    /// Deserializes the hash from hex or from raw bytes, whichever the codec carries.
    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Hash, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = super::decode_either(deserializer)?;
        Hash::try_from(bytes.as_slice()).map_err(serde::de::Error::custom)
    }
}

use id_core::waiting::AuthorKey;

/// The kind of a `chat.anchor.v1` event, as the header's `kind` names it.
pub const CHAT_ANCHOR_KIND: &str = "chat.anchor.v1";

/// The most bytes a part, block, or selector-type identifier may have.
pub const MAX_IDENTIFIER_BYTES: usize = 128;

/// The most bytes a region selector's base64url value may have.
pub const MAX_REGION_VALUE_BYTES: usize = 1_024;

/// Whether `kind` is the chat anchor kind, so its event carries an anchor operation.
#[must_use]
pub fn is_chat_anchor_kind(kind: &str) -> bool {
    kind == CHAT_ANCHOR_KIND
}

/// Why a chat anchor was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnchorError {
    /// A field is empty, oversize, not NFC, or otherwise malformed.
    #[error("{0}")]
    Field(String),
    /// A part selector is malformed or does not match its `pa1` form.
    #[error("{0}")]
    Selector(String),
    /// The operation is not the one the event's kind names.
    #[error("a {kind:?} event carries the wrong anchor operation")]
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
}

/// An addressable ID object: its object ID and the key that authored it.
///
/// Both hashes ride the CBOR wire of the signed header. The `Hash` type speaks
/// human-readable hex to JSON, but CBOR is compact: `serde_bytes` keeps both the
/// wire and the JSON codec working instead of assuming one representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRef {
    /// The object's ID: the event ID of the record that carries it.
    #[serde(with = "hash_bytes")]
    pub object_id: Hash,
    /// The key that authored the object.
    #[serde(with = "hex_bytes")]
    pub author: AuthorKey,
}

impl ObjectRef {
    /// A reference to `object_id` by `author`.
    #[must_use]
    pub fn new(object_id: Hash, author: AuthorKey) -> Self {
        Self { object_id, author }
    }

    /// The object ID as a [`Hash`].
    #[must_use]
    pub fn object_hash(&self) -> Hash {
        self.object_id
    }
}

/// The relation vocabulary of a conversation link (#5870).
///
/// `talk-for` is not an ordinary conversation link: it designates the conversation as
/// the object's canonical talk page and is folded separately ([`TalkPageBinding`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnchorRelation {
    /// The conversation discusses the object generally.
    About,
    /// The conversation refers to one selected part of the object.
    AnchoredToPart,
    /// The conversation was forked or spun out of an addressable chat object.
    DerivedFrom,
    /// The conversation is designated as the object's talk page.
    TalkFor,
}

impl AnchorRelation {
    /// Whether a link with this relation requires a part anchor.
    #[must_use]
    pub fn needs_part(self) -> bool {
        matches!(self, Self::AnchoredToPart)
    }
}

/// A by-reference selector for one part of an object revision (#5870).
///
/// Text ranges are zero-based, half-open offsets in Unicode scalar values. Regional
/// selectors carry the object type's canonical value, base64url-encoded, with the
/// object type owning its schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PartSelector {
    /// A half-open range of Unicode scalar values inside one part.
    TextRange {
        /// The part the range is inside.
        part_id: String,
        /// The first selected scalar, zero-based.
        start: u64,
        /// One past the last selected scalar; greater than `start`.
        end: u64,
    },
    /// One stable structured block inside one part.
    Block {
        /// The part the block is inside.
        part_id: String,
        /// The block's identifier, stable within the object's revision lineage.
        block_id: String,
    },
    /// An object-type-defined region selector.
    Region {
        /// The part the region is inside, when the selector names one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part_id: Option<String>,
        /// The registered selector type.
        selector_type: String,
        /// The canonical selector value, unpadded base64url.
        value: String,
    },
}

impl PartSelector {
    /// Checks the selector's well-formedness rule.
    ///
    /// # Errors
    ///
    /// Returns [`AnchorError::Field`] for an empty, oversize, or non-NFC identifier, an
    /// empty or oversize region value that is not base64url, or a text range whose
    /// `end` is not greater than its `start`.
    pub fn check(&self) -> Result<(), AnchorError> {
        match self {
            Self::TextRange {
                part_id,
                start,
                end,
            } => {
                check_identifier(part_id, "part id")?;
                if end <= start {
                    return Err(AnchorError::Field(format!(
                        "a text range ends at {end}, which is not after its start {start}"
                    )));
                }
            }
            Self::Block { part_id, block_id } => {
                check_identifier(part_id, "part id")?;
                check_identifier(block_id, "block id")?;
            }
            Self::Region {
                part_id,
                selector_type,
                value,
            } => {
                if let Some(part_id) = part_id {
                    check_identifier(part_id, "part id")?;
                }
                check_identifier(selector_type, "selector type")?;
                if value.is_empty() {
                    return Err(AnchorError::Field(
                        "a region value is empty, it must be base64url".to_owned(),
                    ));
                }
                if value.len() > MAX_REGION_VALUE_BYTES {
                    return Err(AnchorError::Field(format!(
                        "a region value is {} bytes, it may be at most {MAX_REGION_VALUE_BYTES}",
                        value.len()
                    )));
                }
                if !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
                {
                    return Err(AnchorError::Field(
                        "a region value is not unpadded base64url".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Encodes this selector as the #5870 version-1 URL-fragment form, pinned to
    /// `revision`.
    ///
    /// The revision is the revision's 32-byte ID in lowercase hexadecimal, so every
    /// field stays inside the grammar's allowed ASCII set.
    #[must_use]
    pub fn encode_v1(&self, revision: &Hash) -> String {
        let revision = hex(revision.as_bytes());
        match self {
            Self::TextRange {
                part_id,
                start,
                end,
            } => format!(
                "pa1~r~{revision}~{part}~{start}~{end}",
                part = encode_field(part_id)
            ),
            Self::Block { part_id, block_id } => format!(
                "pa1~b~{revision}~{}~{}",
                encode_field(part_id),
                encode_field(block_id)
            ),
            Self::Region {
                part_id,
                selector_type,
                value,
            } => match part_id {
                Some(part_id) => format!(
                    "pa1~g~{revision}~1~{}~{}~{}",
                    encode_field(part_id),
                    encode_field(selector_type),
                    encode_field(value)
                ),
                None => format!(
                    "pa1~g~{revision}~0~{}~{}",
                    encode_field(selector_type),
                    encode_field(value)
                ),
            },
        }
    }

    /// Parses the #5870 version-1 URL-fragment selector form, returning the pinned
    /// revision and the selector.
    ///
    /// Parsing is strict: a fragment whose form or field count does not match the
    /// grammar is rejected, never parsed leniently. Each field is percent-decoded
    /// exactly once, after the fields have been split, and `%7E` is refused because a
    /// canonical field encoder never emits it.
    ///
    /// # Errors
    ///
    /// Returns [`AnchorError::Selector`] for a wrong prefix, an unknown form, a field
    /// count that does not match the form, an invalid revision, a bad percent escape,
    /// or a selector that fails [`PartSelector::check`].
    pub fn parse_v1(fragment: &str) -> Result<(Hash, Self), AnchorError> {
        let fields = split_fields(fragment)?;
        let [prefix, form, revision, rest @ ..] = fields.as_slice() else {
            return Err(AnchorError::Selector(
                "a selector needs at least a prefix, form, and revision".to_owned(),
            ));
        };
        if prefix != "pa1" {
            return Err(AnchorError::Selector(format!(
                "a selector prefix {prefix:?} is not \"pa1\""
            )));
        }
        let revision = parse_revision(revision)?;
        let selector = match form.as_str() {
            "r" => match rest {
                [part_id, start, end] => Self::TextRange {
                    part_id: part_id.clone(),
                    start: parse_offset(start)?,
                    end: parse_offset(end)?,
                },
                _ => {
                    return Err(AnchorError::Selector(
                        "a range selector needs a part id, start, and end".to_owned(),
                    ));
                }
            },
            "b" => match rest {
                [part_id, block_id] => Self::Block {
                    part_id: part_id.clone(),
                    block_id: block_id.clone(),
                },
                _ => {
                    return Err(AnchorError::Selector(
                        "a block selector needs a part id and a block id".to_owned(),
                    ));
                }
            },
            "g" => match rest {
                [part_marker, selector_type, value] if part_marker == "0" => Self::Region {
                    part_id: None,
                    selector_type: selector_type.clone(),
                    value: value.clone(),
                },
                [part_marker, part_id, selector_type, value] if part_marker == "1" => {
                    Self::Region {
                        part_id: Some(part_id.clone()),
                        selector_type: selector_type.clone(),
                        value: value.clone(),
                    }
                }
                _ => {
                    return Err(AnchorError::Selector(
                        "a region selector needs a part marker, selector type, and value"
                            .to_owned(),
                    ));
                }
            },
            other => {
                return Err(AnchorError::Selector(format!(
                    "a selector form {other:?} is not one of r, b, or g"
                )));
            }
        };
        selector.check()?;
        Ok((revision, selector))
    }
}

/// A part anchor pinned to the object revision in which it was created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartAnchor {
    /// The immutable object revision the selector was created against.
    #[serde(with = "hash_bytes")]
    pub origin_revision_id: Hash,
    /// The by-reference selector inside that revision.
    pub selector: PartSelector,
}

impl PartAnchor {
    /// Checks the part anchor's selector.
    ///
    /// # Errors
    ///
    /// Returns the selector's error when it is malformed.
    pub fn check(&self) -> Result<(), AnchorError> {
        self.selector.check()
    }

    /// Encodes the anchor as its revision-pinned #5870 selector form.
    #[must_use]
    pub fn encode_v1(&self) -> String {
        self.selector.encode_v1(&self.origin_revision_id)
    }

    /// Parses a revision-pinned #5870 selector form.
    ///
    /// # Errors
    ///
    /// Returns the parser's error when the fragment is malformed.
    pub fn parse_v1(fragment: &str) -> Result<Self, AnchorError> {
        let (origin_revision_id, selector) = PartSelector::parse_v1(fragment)?;
        Ok(Self {
            origin_revision_id,
            selector,
        })
    }
}

/// What one `chat.anchor.v1` event does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum AnchorOp {
    /// Binds a conversation to an object, or designates it as the object's talk page.
    Bind {
        /// The addressable object the conversation binds to.
        target: ObjectRef,
        /// The relation the binding asserts.
        relation: AnchorRelation,
        /// The part anchor, required exactly for [`AnchorRelation::AnchoredToPart`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<PartAnchor>,
    },
    /// Removes a previously bound link from the active set, keeping its history.
    Unbind {
        /// The initial bind event's ID: the link's stable `link_id`.
        #[serde(with = "hash_bytes")]
        link_id: Hash,
    },
    /// Records an accepted relocation of a part anchor on a later revision.
    Relocate {
        /// The initial bind event's ID: the link's stable `link_id`.
        #[serde(with = "hash_bytes")]
        link_id: Hash,
        /// The resolved part anchor; the original selector stays in the bind event.
        part: PartAnchor,
    },
}

impl AnchorOp {
    /// Checks the operation's field rules.
    ///
    /// # Errors
    ///
    /// Returns [`AnchorError::Field`] when a bind's relation and part disagree, or the
    /// part anchor is malformed.
    pub fn check(&self) -> Result<(), AnchorError> {
        match self {
            Self::Bind { relation, part, .. } => {
                if relation.needs_part() && part.is_none() {
                    return Err(AnchorError::Field(
                        "an anchored-to-part link needs its part anchor".to_owned(),
                    ));
                }
                if !relation.needs_part() && part.is_some() {
                    return Err(AnchorError::Field(format!(
                        "a {relation:?} link carries a part anchor, only anchored-to-part does"
                    )));
                }
                if let Some(part) = part {
                    part.check()?;
                }
            }
            Self::Unbind { .. } => {}
            Self::Relocate { part, .. } => part.check()?,
        }
        Ok(())
    }

    /// The relation of a bind operation, or `None` for a mutation.
    #[must_use]
    pub fn relation(&self) -> Option<AnchorRelation> {
        match self {
            Self::Bind { relation, .. } => Some(*relation),
            Self::Unbind { .. } | Self::Relocate { .. } => None,
        }
    }

    /// The link a mutation names, or `None` for a bind.
    #[must_use]
    pub fn link_id(&self) -> Option<Hash> {
        match self {
            Self::Bind { .. } => None,
            Self::Unbind { link_id } | Self::Relocate { link_id, .. } => Some(*link_id),
        }
    }
}

/// The body of a `chat.anchor.v1` event: one anchor operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChatAnchor {
    /// What the event does.
    pub op: AnchorOp,
}

impl ChatAnchor {
    /// A binding of `conversation` to `target` under `relation`, with `part` exactly
    /// when the relation needs one.
    ///
    /// # Errors
    ///
    /// Returns the operation's error when its fields disagree or the part is malformed.
    pub fn bind(
        target: ObjectRef,
        relation: AnchorRelation,
        part: Option<PartAnchor>,
    ) -> Result<Self, AnchorError> {
        let anchor = Self {
            op: AnchorOp::Bind {
                target,
                relation,
                part,
            },
        };
        anchor.check()?;
        Ok(anchor)
    }

    /// An unbind of `link_id`.
    #[must_use]
    pub fn unbind(link_id: Hash) -> Self {
        Self {
            op: AnchorOp::Unbind { link_id },
        }
    }

    /// A relocation of `link_id` to `part`.
    ///
    /// # Errors
    ///
    /// Returns the part's error when it is malformed.
    pub fn relocate(link_id: Hash, part: PartAnchor) -> Result<Self, AnchorError> {
        let anchor = Self {
            op: AnchorOp::Relocate { link_id, part },
        };
        anchor.check()?;
        Ok(anchor)
    }

    /// Checks the anchor's operation.
    ///
    /// # Errors
    ///
    /// Returns the operation's error.
    pub fn check(&self) -> Result<(), AnchorError> {
        self.op.check()
    }
}

/// Checks an anchor body against the kind of the event that carries it.
///
/// # Errors
///
/// Returns [`AnchorError::KindMismatch`] when `kind` is not [`CHAT_ANCHOR_KIND`], and
/// the anchor's own error when its operation is malformed.
pub fn check_chat_anchor(kind: &str, anchor: &ChatAnchor) -> Result<(), AnchorError> {
    if !is_chat_anchor_kind(kind) {
        return Err(AnchorError::KindMismatch {
            kind: kind.to_owned(),
        });
    }
    anchor.check()
}

/// An applied `chat.anchor.v1` event, as the fold reads it: its ID, signer,
/// conversation group, projected causal frontier, and operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorRecord {
    /// The event's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// The conversation group the event belongs to.
    pub conversation: Hash,
    /// The nearest anchor events on this event's causal paths: the frontier the fold
    /// uses to tell a causally later mutation from a concurrent one.
    pub after: Vec<Hash>,
    /// What the event does.
    pub anchor: AnchorOp,
}

/// The folded state of one ordinary conversation link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkState {
    /// The bind is in force, with no mutation after it.
    Current,
    /// A later, causally accepted relocation is in force. The original selector is
    /// still in the bind event.
    Relocated,
    /// An explicit, causally unambiguous unbind removed the link from the active set.
    Unbound,
    /// Concurrent maximal mutations disagree; no mutation is chosen.
    Disputed,
    /// The target object was tombstoned. The link and its conversation survive.
    Tombstoned,
}

/// The folded state of one object's talk-page binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TalkBindingState {
    /// One canonical talk page is in force.
    Canonical,
    /// More than one conversation claims the object, or the first binding is
    /// concurrent with another. Visible, never silently forked.
    Disputed,
    /// The canonical binding was explicitly unbound.
    Unbound,
}

/// One folded ordinary conversation link (`about`, `anchored-to-part`, or
/// `derived-from`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedLink {
    /// The initial bind event's ID: the link's stable ID.
    pub link_id: Hash,
    /// The key that signed the initial bind.
    pub author: AuthorKey,
    /// The conversation group the link belongs to.
    pub conversation: Hash,
    /// The addressable target object.
    pub target: ObjectRef,
    /// The asserted relation.
    pub relation: AnchorRelation,
    /// The part anchor in force: the original, or the accepted relocation.
    pub part: Option<PartAnchor>,
    /// The folded state.
    pub state: LinkState,
}

/// One object's folded talk-page binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TalkPageBinding {
    /// The initial `talk-for` bind event's ID.
    pub binding_id: Hash,
    /// The conversation designated as the object's talk page.
    pub conversation: Hash,
    /// The addressable object the conversation is the talk page of.
    pub target: ObjectRef,
    /// The folded state.
    pub state: TalkBindingState,
}

/// The deterministic projection of a set of `chat.anchor.v1` events.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorProjection {
    /// Folded ordinary links, ordered by `link_id`.
    pub links: Vec<ProjectedLink>,
    /// Folded talk-page bindings, ordered by target then `binding_id`.
    pub talk: Vec<TalkPageBinding>,
}

impl AnchorProjection {
    /// Marks every live link targeting `target` tombstoned, keeping the links and their
    /// conversations intact (#5872 invariant 8). A link that is already unbound stays
    /// unbound.
    pub fn tombstone(&mut self, target: &ObjectRef) {
        for link in &mut self.links {
            if link.target == *target && link.state != LinkState::Unbound {
                link.state = LinkState::Tombstoned;
            }
        }
    }
}

/// Folds `records` into the deterministic anchor projection.
///
/// Duplicate deliveries by event ID are dropped (first delivery wins), and the result
/// depends only on the set of records, never on their order. Each `link_id` keeps at
/// most one state: an ordinary link is `Current`, `Relocated`, `Unbound`, or
/// `Disputed` when concurrent maximal mutations remain unresolved. `talk-for` bindings
/// fold separately: the first valid binding for an object is canonical, and any later
/// binding from another conversation makes the object visibly `Disputed`.
#[must_use]
pub fn project_anchors(records: impl IntoIterator<Item = AnchorRecord>) -> AnchorProjection {
    let mut by_id: BTreeMap<[u8; 32], AnchorRecord> = BTreeMap::new();
    for record in records {
        by_id.entry(*record.id.as_bytes()).or_insert(record);
    }
    let records: Vec<AnchorRecord> = by_id.values().cloned().collect();
    let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
    for record in &records {
        parents.insert(record.id, record.after.clone());
    }

    let links = fold_links(&records, &parents);
    let talk = fold_talk(&records, &parents);
    AnchorProjection { links, talk }
}

fn fold_links(records: &[AnchorRecord], parents: &HashMap<Hash, Vec<Hash>>) -> Vec<ProjectedLink> {
    let binds: Vec<&AnchorRecord> = records
        .iter()
        .filter(|record| {
            matches!(
                record.anchor,
                AnchorOp::Bind {
                    relation,
                    ..
                } if relation != AnchorRelation::TalkFor
            )
        })
        .collect();
    let mut links = Vec::with_capacity(binds.len());
    for bind in binds {
        let AnchorOp::Bind {
            target,
            relation,
            part,
        } = &bind.anchor
        else {
            continue;
        };
        let link_id = bind.id;
        let mutations: Vec<&AnchorRecord> = records
            .iter()
            .filter(|record| record.anchor.link_id() == Some(link_id))
            .collect();
        let maximal: Vec<&&AnchorRecord> = mutations
            .iter()
            .filter(|mutation| {
                !mutations
                    .iter()
                    .any(|other| other.id != mutation.id && follows(other.id, mutation.id, parents))
            })
            .collect();

        let (state, part) = if maximal.len() > 1 {
            (LinkState::Disputed, part.clone())
        } else {
            match maximal.first() {
                None => (LinkState::Current, part.clone()),
                Some(record) => match &record.anchor {
                    AnchorOp::Unbind { .. } => (LinkState::Unbound, part.clone()),
                    AnchorOp::Relocate {
                        part: relocated, ..
                    } => {
                        // A relocation that observes an unbind cannot undo it: a
                        // resolved unbind is terminal, and rebinding makes a new link.
                        let dominated = mutations.iter().any(|other| {
                            matches!(other.anchor, AnchorOp::Unbind { .. })
                                && follows(record.id, other.id, parents)
                        });
                        if dominated {
                            (LinkState::Unbound, part.clone())
                        } else {
                            (LinkState::Relocated, Some(relocated.clone()))
                        }
                    }
                    AnchorOp::Bind { .. } => (LinkState::Current, part.clone()),
                },
            }
        };
        links.push(ProjectedLink {
            link_id,
            author: bind.author,
            conversation: bind.conversation,
            target: *target,
            relation: *relation,
            part,
            state,
        });
    }
    links.sort_by_key(|link| *link.link_id.as_bytes());
    links
}

fn fold_talk(records: &[AnchorRecord], parents: &HashMap<Hash, Vec<Hash>>) -> Vec<TalkPageBinding> {
    let mut candidates: BTreeMap<([u8; 32], [u8; 32]), Vec<&AnchorRecord>> = BTreeMap::new();
    for record in records {
        if let AnchorOp::Bind {
            target,
            relation: AnchorRelation::TalkFor,
            ..
        } = &record.anchor
        {
            candidates
                .entry((*target.object_id.as_bytes(), target.author))
                .or_default()
                .push(record);
        }
    }

    let mut talk = Vec::with_capacity(candidates.len());
    for group in candidates.values() {
        let target = match &group[0].anchor {
            AnchorOp::Bind { target, .. } => *target,
            _ => continue,
        };
        // The first valid binding in causal order wins: a candidate is minimal when
        // it observes no other candidate, i.e. nothing else in the group names it
        // as an ancestor. The event-ID minimum only breaks ties deterministically.
        let minima: Vec<&&AnchorRecord> = group
            .iter()
            .filter(|candidate| {
                !group.iter().any(|other| {
                    other.id != candidate.id && follows(candidate.id, other.id, parents)
                })
            })
            .collect();
        let selected = minima
            .iter()
            .min_by_key(|record| *record.id.as_bytes())
            .copied()
            .or_else(|| group.iter().min_by_key(|record| *record.id.as_bytes()));

        let Some(selected) = selected else {
            continue;
        };
        let binding_id = selected.id;
        let conversation = selected.conversation;

        let state = if minima.len() != 1 {
            TalkBindingState::Disputed
        } else {
            let mutations: Vec<&AnchorRecord> = records
                .iter()
                .filter(|record| record.anchor.link_id() == Some(binding_id))
                .collect();
            let maximal_unbind = mutations.iter().any(|mutation| {
                matches!(mutation.anchor, AnchorOp::Unbind { .. })
                    && !mutations.iter().any(|other| {
                        other.id != mutation.id && follows(other.id, mutation.id, parents)
                    })
            });
            if maximal_unbind {
                TalkBindingState::Unbound
            } else if group
                .iter()
                .any(|other| other.id != binding_id && other.conversation != conversation)
            {
                TalkBindingState::Disputed
            } else {
                TalkBindingState::Canonical
            }
        };
        talk.push(TalkPageBinding {
            binding_id,
            conversation,
            target,
            state,
        });
    }
    talk.sort_by_key(|binding| {
        (
            *binding.target.object_id.as_bytes(),
            *binding.binding_id.as_bytes(),
        )
    });
    talk
}

/// Whether `record` causally follows `ancestor` through the anchor-event frontier.
fn follows(record: Hash, ancestor: Hash, parents: &HashMap<Hash, Vec<Hash>>) -> bool {
    let mut stack = parents.get(&record).cloned().unwrap_or_default();
    let mut seen: HashSet<Hash> = HashSet::new();
    while let Some(id) = stack.pop() {
        if id == ancestor {
            return true;
        }
        if seen.insert(id) {
            if let Some(grandparents) = parents.get(&id) {
                stack.extend(grandparents.iter().copied());
            }
        }
    }
    false
}

fn check_identifier(value: &str, label: &str) -> Result<(), AnchorError> {
    if value.is_empty() {
        return Err(AnchorError::Field(format!("a {label} is empty")));
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(AnchorError::Field(format!(
            "a {label} is {} bytes, it may be at most {MAX_IDENTIFIER_BYTES}",
            value.len()
        )));
    }
    if !is_nfc(value) {
        return Err(AnchorError::Field(format!("a {label} is not NFC")));
    }
    Ok(())
}

fn parse_offset(value: &str) -> Result<u64, AnchorError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AnchorError::Selector(format!(
            "a range offset {value:?} is not a decimal scalar value"
        )));
    }
    value.parse::<u64>().map_err(|_| {
        AnchorError::Selector(format!("a range offset {value:?} does not fit in 64 bits"))
    })
}

fn parse_revision(value: &str) -> Result<Hash, AnchorError> {
    if value.len() != 64 {
        return Err(AnchorError::Selector(format!(
            "a revision {:?} is not 32 bytes of hexadecimal",
            value
        )));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair)
            .map_err(|_| AnchorError::Selector("a revision is not UTF-8".to_owned()))?;
        bytes[index] = u8::from_str_radix(pair, 16).map_err(|_| {
            AnchorError::Selector(format!("a revision {value:?} is not hexadecimal"))
        })?;
    }
    Ok(Hash::from_bytes(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(LOWER[usize::from(byte >> 4)]));
        text.push(char::from(LOWER[usize::from(byte & 0x0f)]));
    }
    text
}

/// Encodes one selector field: ASCII alphanumerics, `.`, `_`, and `-` stay literal; a
/// literal `~` becomes `~~`; every other byte is percent-encoded once.
fn encode_field(value: &str) -> String {
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte == b'~' {
            encoded.push_str("~~");
        } else if byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'-' {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(UPPER[usize::from(byte >> 4)]));
            encoded.push(char::from(UPPER[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

/// Splits a fragment into fields on lone `~` separators, decoding `~~` to a literal `~`
/// and percent-decoding each field exactly once. `%7E` and `%7e` are refused.
fn split_fields(fragment: &str) -> Result<Vec<String>, AnchorError> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = fragment.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '~' {
            if chars.peek() == Some(&'~') {
                chars.next();
                current.push('~');
            } else {
                fields.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    fields.push(current);
    fields
        .into_iter()
        .map(|field| percent_decode(&field))
        .collect()
}

fn percent_decode(field: &str) -> Result<String, AnchorError> {
    let bytes = field.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let escape = bytes.get(index + 1..index + 3).ok_or_else(|| {
                AnchorError::Selector("a selector field ends in a bare percent".to_owned())
            })?;
            let escape = std::str::from_utf8(escape)
                .map_err(|_| AnchorError::Selector("a percent escape is not ASCII".to_owned()))?;
            if escape.eq_ignore_ascii_case("7e") {
                return Err(AnchorError::Selector(
                    "a selector field uses %7E, which a canonical encoder never emits".to_owned(),
                ));
            }
            let byte = u8::from_str_radix(escape, 16).map_err(|_| {
                AnchorError::Selector(format!("a percent escape %{escape} is not hexadecimal"))
            })?;
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| AnchorError::Selector("a selector field is not UTF-8".to_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn hash(seed: u8) -> Hash {
        Hash::digest([seed])
    }

    fn author(seed: u8) -> AuthorKey {
        [seed; 32]
    }

    fn object(seed: u8) -> ObjectRef {
        ObjectRef::new(hash(seed), author(seed))
    }

    fn part(seed: u8) -> PartAnchor {
        PartAnchor {
            origin_revision_id: hash(seed),
            selector: PartSelector::Block {
                part_id: format!("part-{seed}"),
                block_id: "block".to_owned(),
            },
        }
    }

    fn record(id: u8, after: &[u8], conversation: u8, anchor: AnchorOp) -> AnchorRecord {
        AnchorRecord {
            id: hash(id),
            author: author(id),
            conversation: hash(conversation),
            after: after.iter().map(|parent| hash(*parent)).collect(),
            anchor,
        }
    }

    fn bind(
        id: u8,
        after: &[u8],
        conversation: u8,
        target: ObjectRef,
        relation: AnchorRelation,
        part: Option<PartAnchor>,
    ) -> AnchorRecord {
        record(
            id,
            after,
            conversation,
            AnchorOp::Bind {
                target,
                relation,
                part,
            },
        )
    }

    fn unbind(id: u8, after: &[u8], conversation: u8, link_id: Hash) -> AnchorRecord {
        record(id, after, conversation, AnchorOp::Unbind { link_id })
    }

    fn relocate(
        id: u8,
        after: &[u8],
        conversation: u8,
        link_id: Hash,
        part: PartAnchor,
    ) -> AnchorRecord {
        record(
            id,
            after,
            conversation,
            AnchorOp::Relocate { link_id, part },
        )
    }

    #[test]
    fn test_selector_round_trips_every_form() {
        let revision = hash(9);
        let selectors = [
            PartSelector::TextRange {
                part_id: "body".to_owned(),
                start: 0,
                end: 4,
            },
            PartSelector::Block {
                part_id: "body".to_owned(),
                block_id: "intro".to_owned(),
            },
            PartSelector::Region {
                part_id: None,
                selector_type: "svg-path".to_owned(),
                value: "M0-0_ab".to_owned(),
            },
            PartSelector::Region {
                part_id: Some("canvas".to_owned()),
                selector_type: "rect".to_owned(),
                value: "x1y2".to_owned(),
            },
            // A field that needs both escapes: a space, a literal tilde, and a percent.
            PartSelector::TextRange {
                part_id: "a b~c%".to_owned(),
                start: 3,
                end: 7,
            },
            // Non-ASCII text is percent-encoded per UTF-8 byte, not per scalar.
            PartSelector::Block {
                part_id: "café".to_owned(),
                block_id: "ünïcode".to_owned(),
            },
        ];
        for selector in selectors {
            let encoded = selector.encode_v1(&revision);
            assert_eq!(
                PartSelector::parse_v1(&encoded).unwrap(),
                (revision, selector.clone()),
                "{encoded}"
            );
            let anchor = PartAnchor {
                origin_revision_id: revision,
                selector,
            };
            assert_eq!(PartAnchor::parse_v1(&anchor.encode_v1()).unwrap(), anchor);
        }

        let encoded = PartSelector::TextRange {
            part_id: "a b~c%".to_owned(),
            start: 3,
            end: 7,
        }
        .encode_v1(&revision);
        assert!(encoded.ends_with("~a%20b~~c%25~3~7"), "{encoded}");
    }

    #[test]
    fn test_selector_parser_refuses_malformed_fragments() {
        let revision = hex(hash(9).as_bytes());
        let malformed = [
            format!("pa2~r~{revision}~body~0~1"),
            format!("pa1~x~{revision}~body"),
            format!("pa1~r~{revision}~body~0~0"),
            format!("pa1~r~{revision}~body~4~3"),
            format!("pa1~r~{revision}~body~0"),
            format!("pa1~r~{revision}~body~0~1~2"),
            "pa1~r~nothex~body~0~1".to_owned(),
            format!("pa1~r~{revision}~body~-1~1"),
            format!("pa1~r~{revision}~body~0~1.5"),
            format!("pa1~b~{revision}~body"),
            format!("pa1~g~{revision}~2~type~value"),
            format!("pa1~g~{revision}~0~type~a+b"),
            format!("pa1~r~{revision}~b%7Edy~0~1"),
            format!("pa1~r~{revision}~b%7edy~0~1"),
            format!("pa1~r~{revision}~body~0~1%"),
            format!("pa1~r~{revision}~body~0~1%FF"),
            "pa1~".to_owned(),
            "pa1".to_owned(),
        ];
        for fragment in malformed {
            // Both grammar failures (`Selector`) and field-rule failures (`Field`)
            // are refusals; nothing malformed parses into a selector.
            assert!(PartSelector::parse_v1(&fragment).is_err(), "{fragment}");
        }

        // Field rules are checked by the parser too, not only by constructors.
        let oversize = "x".repeat(MAX_IDENTIFIER_BYTES + 1);
        assert!(
            PartSelector::TextRange {
                part_id: oversize,
                start: 0,
                end: 1,
            }
            .check()
            .is_err()
        );
        assert!(
            PartSelector::Region {
                part_id: None,
                selector_type: "type".to_owned(),
                value: "x".repeat(MAX_REGION_VALUE_BYTES + 1),
            }
            .check()
            .is_err()
        );
    }

    #[test]
    fn test_bind_relocate_unbind_fold_keeps_the_original_selector() {
        let target = object(1);
        let bind = bind(
            10,
            &[],
            7,
            target,
            AnchorRelation::AnchoredToPart,
            Some(part(20)),
        );
        let bind_id = bind.id;
        let relocated = relocate(11, &[10], 7, bind_id, part(21));
        let projection = project_anchors([bind.clone(), relocated.clone()]);
        assert_eq!(projection.links.len(), 1);
        let link = &projection.links[0];
        assert_eq!(link.link_id, bind_id);
        assert_eq!(link.state, LinkState::Relocated);
        assert_eq!(link.part.as_ref(), Some(&part(21)));
        assert_eq!(link.target, target);
        assert_eq!(link.relation, AnchorRelation::AnchoredToPart);
        // The accepted relocation changes derived state only; the bind event still
        // carries the selector it was created with (invariant 5).
        assert!(matches!(
            &bind.anchor,
            AnchorOp::Bind { part: Some(original), .. } if *original == part(20)
        ));

        // A resolved unbind is terminal: a relocation that observes it cannot revive
        // the link, and coming back means a fresh bind with a new `link_id`.
        let dead = unbind(12, &[10], 7, bind_id);
        let after_death = relocate(13, &[12], 7, bind_id, part(22));
        let projection = project_anchors([bind.clone(), dead, after_death]);
        assert_eq!(projection.links[0].state, LinkState::Unbound);
        assert_eq!(projection.links[0].part.as_ref(), Some(&part(20)));

        // A plain unbind after the bind is `Unbound`; the link and its history stay.
        let unbind = unbind(14, &[10], 7, bind_id);
        let projection = project_anchors([bind, unbind]);
        assert_eq!(projection.links[0].state, LinkState::Unbound);
        assert_eq!(projection.links[0].part.as_ref(), Some(&part(20)));
    }

    #[test]
    fn test_concurrent_mutations_are_disputed_and_never_silently_resolved() {
        let target = object(2);
        let bind = bind(20, &[], 8, target, AnchorRelation::About, None);
        let bind_id = bind.id;
        let removed = unbind(21, &[20], 8, bind_id);
        let moved = relocate(22, &[20], 8, bind_id, part(23));
        // The smaller event ID does not win: both are maximal, so the state is visible.
        let projection = project_anchors([bind.clone(), removed.clone(), moved.clone()]);
        assert_eq!(projection.links[0].state, LinkState::Disputed);
        assert_eq!(projection.links[0].part, None);

        // Delivery order and duplicate delivery change nothing (invariants 3 and 10).
        let records = [bind, removed, moved];
        let expected = project_anchors(records.to_vec());
        for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0], [2, 0, 1], [1, 0, 2]] {
            let permuted: Vec<AnchorRecord> =
                order.iter().map(|index| records[*index].clone()).collect();
            assert_eq!(project_anchors(permuted), expected, "{order:?}");
        }
        let mut duplicated = Vec::new();
        duplicated.extend(records.iter().cloned());
        duplicated.extend(records.iter().cloned());
        duplicated.push(records[1].clone());
        assert_eq!(project_anchors(duplicated), expected);
    }

    #[test]
    fn test_talk_for_is_first_wins_and_conflicts_are_disputed() {
        let target = object(3);
        let first = bind(30, &[], 1, target, AnchorRelation::TalkFor, None);
        // The same conversation restating the claim is not a conflict, and the first
        // binding stays canonical.
        let restated = bind(31, &[30], 1, target, AnchorRelation::TalkFor, None);
        let projection = project_anchors([first.clone(), restated]);
        assert!(
            projection.links.is_empty(),
            "talk-for is not an ordinary link"
        );
        assert_eq!(projection.talk.len(), 1);
        assert_eq!(projection.talk[0].binding_id, first.id);
        assert_eq!(projection.talk[0].conversation, hash(1));
        assert_eq!(projection.talk[0].state, TalkBindingState::Canonical);

        // A later claim from another conversation takes nothing away: the first stays
        // selected, and the disagreement is visible.
        let other_conversation = bind(32, &[30], 2, target, AnchorRelation::TalkFor, None);
        let projection = project_anchors([first.clone(), other_conversation]);
        assert_eq!(projection.talk[0].binding_id, first.id);
        assert_eq!(projection.talk[0].state, TalkBindingState::Disputed);

        // Concurrent first claims are disputed, whatever their event IDs.
        let concurrent = bind(33, &[], 1, target, AnchorRelation::TalkFor, None);
        let projection = project_anchors([first.clone(), concurrent]);
        assert_eq!(projection.talk[0].state, TalkBindingState::Disputed);

        // An unbind of the selected binding removes the canonical talk page.
        let unbind = unbind(34, &[30], 1, first.id);
        let projection = project_anchors([first.clone(), unbind]);
        assert_eq!(projection.talk[0].state, TalkBindingState::Unbound);

        // A talk-for bind carries no part, and an ordinary bind needs one only for
        // anchored-to-part (operator rules of the relation).
        assert!(ChatAnchor::bind(target, AnchorRelation::TalkFor, Some(part(3))).is_err());
        assert!(ChatAnchor::bind(target, AnchorRelation::AnchoredToPart, None).is_err());
        assert!(ChatAnchor::bind(target, AnchorRelation::About, Some(part(3))).is_err());
        assert!(ChatAnchor::bind(target, AnchorRelation::AnchoredToPart, Some(part(3))).is_ok());
    }

    #[test]
    fn test_tombstoning_keeps_links_and_its_unbound_state() {
        let target = object(4);
        let other = object(5);
        let live = bind(40, &[], 9, target, AnchorRelation::About, None);
        let gone = bind(41, &[], 9, target, AnchorRelation::About, None);
        let removed = unbind(42, &[41], 9, gone.id);
        let elsewhere = bind(43, &[], 9, other, AnchorRelation::About, None);
        let mut projection =
            project_anchors([live.clone(), gone.clone(), removed, elsewhere.clone()]);
        projection.tombstone(&target);

        let state = |link_id: Hash| {
            projection
                .links
                .iter()
                .find(|link| link.link_id == link_id)
                .unwrap()
                .state
        };
        assert_eq!(state(live.id), LinkState::Tombstoned);
        // An explicit unbind is not rewritten into a tombstone.
        assert_eq!(state(gone.id), LinkState::Unbound);
        assert_eq!(state(elsewhere.id), LinkState::Current);
        // The links and their conversations survive the tombstone.
        assert_eq!(projection.links.len(), 3);
        assert!(
            projection
                .links
                .iter()
                .all(|link| link.conversation == hash(9))
        );
    }

    #[test]
    fn test_anchor_serialization_carries_references_only() {
        let anchor = ChatAnchor::bind(object(6), AnchorRelation::About, None).unwrap();
        let value = serde_json::to_value(&anchor).unwrap();
        let object_value = value.as_object().unwrap();
        let mut keys: Vec<&str> = object_value.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["op", "relation", "target"]);
        assert_eq!(value["op"], "bind");
        assert_eq!(value["relation"], "about");
        assert_eq!(serde_json::from_value::<ChatAnchor>(value).unwrap(), anchor);

        // A part anchor serializes its revision and by-reference selector, nothing else.
        let part = ChatAnchor::bind(
            object(6),
            AnchorRelation::AnchoredToPart,
            Some(PartAnchor {
                origin_revision_id: hash(7),
                selector: PartSelector::TextRange {
                    part_id: "body".to_owned(),
                    start: 0,
                    end: 4,
                },
            }),
        )
        .unwrap();
        let value = serde_json::to_value(&part).unwrap();
        let part_value = &value["part"];
        let mut keys: Vec<&str> = part_value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["origin_revision_id", "selector"]);
        assert!(value.get("text").is_none());
        assert!(value.get("content").is_none());
        assert!(value.get("body").is_none());
        assert!(value.get("capability").is_none());
    }

    #[test]
    fn test_relation_vocabulary_is_exactly_the_four_relations() {
        // Whatever the wire spells, the fold knows four relations and treats only
        // `talk-for` separately (invariant 3).
        let wire = [
            ("about", AnchorRelation::About),
            ("anchored-to-part", AnchorRelation::AnchoredToPart),
            ("derived-from", AnchorRelation::DerivedFrom),
            ("talk-for", AnchorRelation::TalkFor),
        ];
        for (name, relation) in wire {
            assert_eq!(
                serde_json::to_value(relation).unwrap(),
                serde_json::Value::String(name.to_owned())
            );
        }
        assert!(AnchorRelation::AnchoredToPart.needs_part());
        assert!(!AnchorRelation::TalkFor.needs_part());
    }

    /// A generated operation, with mutations naming an earlier bind by index.
    #[derive(Clone, Debug)]
    enum Op {
        BindAbout,
        BindPart,
        TalkFor { conversation: u8 },
        Unbind { link: usize },
        Relocate { link: usize },
    }

    fn arb_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            Just(Op::BindAbout),
            Just(Op::BindPart),
            (0_u8..2).prop_map(|conversation| Op::TalkFor { conversation }),
            (0_usize..16).prop_map(|link| Op::Unbind { link }),
            (0_usize..16).prop_map(|link| Op::Relocate { link }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// The fold is a function of the event set: reordering delivery and repeating
        /// events change neither the projection nor its ordering (invariants 3, 10).
        #[test]
        fn prop_projection_is_order_and_duplicate_independent(
            ops in proptest::collection::vec(arb_op(), 0..12),
            shuffle_seed in any::<u8>(),
        ) {
            let target = object(50);
            let mut records: Vec<AnchorRecord> = Vec::new();
            let mut bound: Vec<Hash> = Vec::new();
            for (index, op) in ops.iter().enumerate() {
                let id = u8::try_from(index).unwrap();
                // Causal parents are a subset of the earlier records, so the log is a DAG.
                let after: Vec<u8> = (0..id)
                    .filter(|parent| (parent + id).wrapping_add(shuffle_seed) % 3 == 0)
                    .collect();
                let record = match op {
                    Op::BindAbout => {
                        let record = bind(id, &after, 1, target, AnchorRelation::About, None);
                        bound.push(record.id);
                        record
                    }
                    Op::BindPart => {
                        let record =
                            bind(id, &after, 1, target, AnchorRelation::AnchoredToPart, Some(part(id)));
                        bound.push(record.id);
                        record
                    }
                    Op::TalkFor { conversation } => {
                        bind(id, &after, *conversation, target, AnchorRelation::TalkFor, None)
                    }
                    Op::Unbind { link } => {
                        let link_id = bound.get(*link).copied().unwrap_or(hash(255));
                        unbind(id, &after, 1, link_id)
                    }
                    Op::Relocate { link } => {
                        let link_id = bound.get(*link).copied().unwrap_or(hash(255));
                        relocate(id, &after, 1, link_id, part(id))
                    }
                };
                records.push(record);
            }

            let expected = project_anchors(records.clone());

            // An arbitrary deterministic permutation of the delivery order.
            let mut shuffled = records.clone();
            shuffled.sort_by_key(|record| Hash::digest([record.id.as_bytes()[0], shuffle_seed]));
            prop_assert_eq!(project_anchors(shuffled.clone()), expected.clone());

            // Every event delivered twice, and once more in reverse.
            let mut duplicated = shuffled;
            duplicated.extend(records.iter().cloned());
            duplicated.extend(records.iter().rev().cloned());
            prop_assert_eq!(project_anchors(duplicated), expected.clone());

            // Keep the invariants that are cheap to state over any log.
            let talks = records
                .iter()
                .filter(|record| {
                    matches!(
                        record.anchor,
                        AnchorOp::Bind {
                            relation: AnchorRelation::TalkFor,
                            ..
                        }
                    )
                })
                .count();
            prop_assert_eq!(expected.talk.len(), usize::from(talks > 0));
            prop_assert_eq!(
                expected.links.len(),
                records
                    .iter()
                    .filter(|record| {
                        matches!(
                            record.anchor,
                            AnchorOp::Bind { relation, .. } if relation != AnchorRelation::TalkFor
                        )
                    })
                    .count()
            );
            for link in &expected.links {
                prop_assert!(matches!(
                    link.state,
                    LinkState::Current
                        | LinkState::Relocated
                        | LinkState::Unbound
                        | LinkState::Disputed
                ));
                // A link is never projected without a bind naming it.
                prop_assert!(records.iter().any(|record| record.id == link.link_id));
            }
        }
    }
}
