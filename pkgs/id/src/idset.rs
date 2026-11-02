//! Writer-set objects: the schema types and canonical body encoding for the
//! `idset.writer` and `idset.rescind` event kinds (writer-set spec §2 and §3.3,
//! implementation routing §9 item 1; issue #3854, origin #3789).
//!
//! A writer set is its own signed event kind, with the entry list in the event
//! **body** — never in `ChainExt` — so the signed-header format, and therefore
//! every existing verifier, is unchanged (spec §D1). Bodies produced here are
//! passed to [`crate::chain::sign`] as the event body and checked on receipt by
//! [`crate::chain::decode`] plus [`decode_writer_body`] / [`decode_rescind_body`];
//! no new crypto is involved.
//!
//! The body is a canonical CBOR array. A writer-set body is
//!
//! ```text
//! ["idset.writer", version, set_id, set_version, owner, scope, entries, excludes]
//! ```
//!
//! and a rescind body is
//!
//! ```text
//! ["idset.rescind", version, set_id, from_version, target, through]
//! ```
//!
//! where `version` is [`VERSION`], `set_id` and `through` are 32-byte strings,
//! `set_version` and `from_version` are the publishing event's sequence number
//! in the owner's chain (spec §D2 — a version is a chain position, not a
//! counter the owner invents), `owner` is the owner's 32-byte key, `scope` is
//! text or null, and `through` is a 32-byte string or null.
//!
//! An entry is an array whose first item names its shape:
//!
//! | shape    | items                                       |
//! |----------|---------------------------------------------|
//! | `key`    | `["key", author, include_history]`         |
//! | `group`  | `["group", id, role_min, include_history]` |
//! | `public` | `["public", include_history]`              |
//!
//! `role_min` is the [`Role`] as an unsigned integer: `0` pull, `1` read,
//! `2` write, `3` manage, `4` admin.
//!
//! `include_history` is spelled on every shape (spec §D3): it is per entry, so
//! importing one writer's backlog never imports another's. A body written
//! before the flag existed omits it — the entry array simply ends early — and
//! decodes with `include_history: false`, the prospective default, rather than
//! failing. Encoders always write the flag.
//!
//! Canonicality is judged at the CBOR value level, the way
//! [`crate::chain::decode`] judges a header: the bytes must be the canonical
//! encoding of the value they hold. That is what lets a legacy entry (a
//! shorter array) be canonical bytes and still decode, while a re-encoded
//! entry always carries the flag.
//!
//! This module is types and encoding only. Expansion, projection, and
//! counting are later slices of the spec's routing (§9 items 2–4) and are
//! deliberately absent here.

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_core::Hash;
use unicode_normalization::is_nfc;

use crate::groups::Role;
use crate::waiting::AuthorKey;

/// The kind of a writer-set event, as the header's `kind` names it.
pub const WRITER_KIND: &str = "idset.writer";

/// The kind of a rescind event, as the header's `kind` names it.
pub const RESCIND_KIND: &str = "idset.rescind";

/// The version of the body format this code writes and reads.
pub const VERSION: u64 = 1;

/// The most bytes an idset body may have.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The most entries a writer set may list across `entries` and `excludes`.
pub const MAX_ENTRIES: usize = 4096;

/// The longest `scope`, in UTF-8 bytes.
pub const MAX_SCOPE_BYTES: usize = 512;

/// Nesting limit for a body's CBOR. Bodies are shallow by construction.
const RECURSION_LIMIT: u16 = 8;

/// Declared element limit for one array or string. A body may legitimately
/// hold thousands of entries, unlike a header.
const LENGTH_LIMIT: u64 = 8192;

/// Byte budget for speculative pre-allocation. Matches the header decoder.
const OOM_MITIGATION: usize = 64;

/// A writer set: one version of the set object (spec §D1).
///
/// `version` is the sequence number of the publishing event in the owner's
/// own chain (spec §D2). The type carries it because the body names it, but
/// nothing here mints it: the caller publishing the set passes the sequence
/// the event will occupy, and a reader can check it against the decoded
/// header's `seq_num`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterSet {
    /// The set's stable derived ID (see [`WriterSet::derive_set_id`]).
    pub set_id: Hash,
    /// The publishing event's sequence number in the owner's chain.
    pub version: u64,
    /// The owner: the key whose chain carries this set's versions.
    pub owner: AuthorKey,
    /// What the set is for, in the owner's words. Optional.
    pub scope: Option<String>,
    /// The entries that admit writers.
    pub entries: Vec<Entry>,
    /// The entries that un-admit writers. Exclusions win (spec §D6).
    pub excludes: Excludes,
}

impl WriterSet {
    /// The stable ID of the set owned by `owner` for `purpose`: BLAKE3 over
    /// the owner key and the length-prefixed purpose text (spec §D2).
    #[must_use]
    pub fn derive_set_id(owner: &AuthorKey, purpose: &str) -> Hash {
        let mut input = Vec::with_capacity(32 + 8 + purpose.len());
        input.extend_from_slice(owner);
        input.extend_from_slice(&(purpose.len() as u64).to_le_bytes());
        input.extend_from_slice(purpose.as_bytes());
        Hash::digest(&input)
    }

    /// Checks the rules every writer set meets. The encoder and the decoder
    /// both apply them.
    ///
    /// # Errors
    ///
    /// Returns [`IdsetError::Shape`] for a scope that is too long or not NFC,
    /// or for more than [`MAX_ENTRIES`] entries in total.
    pub fn check(&self) -> Result<(), IdsetError> {
        if let Some(scope) = &self.scope {
            check_scope(scope)?;
        }
        let total = self.entries.len() + self.excludes.0.len();
        if total > MAX_ENTRIES {
            return Err(IdsetError::Shape(format!(
                "{total} entries, the most is {MAX_ENTRIES}"
            )));
        }
        Ok(())
    }
}

/// One entry of a writer set (spec §D3): a union of three shapes, each
/// carrying its own `include_history`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// One author, named by their verifying key.
    Key {
        /// The author's 32-byte key.
        author: AuthorKey,
        /// Whether this entry also admits the author's earlier claims.
        include_history: bool,
    },
    /// A group, expanded one level at the set's frontier (spec §D4),
    /// filtered to members at or above `role_min`.
    Group {
        /// The group's ID.
        id: Hash,
        /// The least role a member must hold to be admitted by this entry.
        role_min: Role,
        /// Whether this entry also admits the admitted members' earlier claims.
        include_history: bool,
    },
    /// The everyone-sentinel: every author. A deliberate, signed choice,
    /// never a default (spec §D5).
    Public {
        /// Whether this entry also admits every author's earlier claims.
        include_history: bool,
    },
}

impl Entry {
    /// Whether this entry admits history as well as future claims.
    #[must_use]
    pub fn include_history(&self) -> bool {
        match self {
            Entry::Key {
                include_history, ..
            }
            | Entry::Group {
                include_history, ..
            }
            | Entry::Public { include_history } => *include_history,
        }
    }
}

/// The exclusions of a writer set (spec §D6): entries evaluated after the
/// admissions, which win over them. `Excludes(vec![Entry::Public { .. }])`
/// admits nobody, prospectively — the emergency stop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Excludes(pub Vec<Entry>);

/// A rescind: an explicit, visible set-version event that erases a target's
/// already-counted claims (spec §3.3). It is a separate kind from the set
/// itself, carries no entries, and is never an admission state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rescind {
    /// The set this rescind applies to.
    pub set_id: Hash,
    /// The set version from whose position the erasure applies.
    pub from_version: u64,
    /// Whose claims are erased. `Entry::Public` erases everyone's.
    pub target: Entry,
    /// The erasure is bounded to claims in this event's causal past, when named.
    pub through: Option<Hash>,
}

/// Why an idset body was refused, or why one could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdsetError {
    /// The body is longer than [`MAX_BODY_BYTES`].
    #[error("an idset body is {0} bytes, the most allowed is {max}", max = MAX_BODY_BYTES)]
    Oversized(usize),
    /// The bytes are CBOR that strict decoding refuses.
    #[error("the idset body is not CBOR: {0}")]
    Cbor(String),
    /// The bytes decode, but they are not the canonical encoding of their value.
    #[error("the idset body is not canonical CBOR")]
    NotCanonical,
    /// The body's tag names another kind.
    #[error("the idset body is for kind {0:?}, not {1:?}")]
    Kind(String, &'static str),
    /// The format version is not [`VERSION`].
    #[error("the idset body has version {0}, this code reads version {v}", v = VERSION)]
    Version(u64),
    /// The body has the wrong shape for its kind.
    #[error("the idset body has the wrong shape: {0}")]
    Shape(String),
}

/// Whether `kind` is the writer-set kind, so its event carries a writer-set body.
#[must_use]
pub fn is_writer_kind(kind: &str) -> bool {
    kind == WRITER_KIND
}

/// Whether `kind` is the rescind kind, so its event carries a rescind body.
#[must_use]
pub fn is_rescind_kind(kind: &str) -> bool {
    kind == RESCIND_KIND
}

/// Encodes a writer set as an event body.
///
/// # Errors
///
/// Returns [`IdsetError::Shape`] for a set that fails [`WriterSet::check`],
/// and [`IdsetError::Oversized`] when the encoding is longer than
/// [`MAX_BODY_BYTES`].
pub fn encode_writer_body(set: &WriterSet) -> Result<Vec<u8>, IdsetError> {
    set.check()?;
    let body = Value::Array(vec![
        Value::TextString(WRITER_KIND.into()),
        Value::Unsigned(VERSION),
        hash_value(&set.set_id),
        Value::Unsigned(set.version),
        Value::ByteString(set.owner.to_vec().into()),
        scope_value(set.scope.as_deref()),
        entries_value(&set.entries),
        entries_value(&set.excludes.0),
    ])
    .encode();
    check_size(body)
}

/// Decodes an event body as a writer set.
///
/// # Errors
///
/// Returns the reason the body was refused. See [`IdsetError`].
pub fn decode_writer_body(body: &[u8]) -> Result<WriterSet, IdsetError> {
    let items = top_level(body, WRITER_KIND, 8)?;
    let set = WriterSet {
        set_id: hash_field(&items[2])?,
        version: unsigned(&items[3])?,
        owner: key_field(&items[4])?,
        scope: scope_field(&items[5])?,
        entries: entries_field(&items[6])?,
        excludes: Excludes(entries_field(&items[7])?),
    };
    set.check()?;
    Ok(set)
}

/// Encodes a rescind as an event body.
///
/// # Errors
///
/// Returns [`IdsetError::Oversized`] when the encoding is longer than
/// [`MAX_BODY_BYTES`].
pub fn encode_rescind_body(rescind: &Rescind) -> Result<Vec<u8>, IdsetError> {
    let body = Value::Array(vec![
        Value::TextString(RESCIND_KIND.into()),
        Value::Unsigned(VERSION),
        hash_value(&rescind.set_id),
        Value::Unsigned(rescind.from_version),
        entry_value(&rescind.target),
        match &rescind.through {
            Some(hash) => hash_value(hash),
            None => Value::null(),
        },
    ])
    .encode();
    check_size(body)
}

/// Decodes an event body as a rescind.
///
/// # Errors
///
/// Returns the reason the body was refused. See [`IdsetError`].
pub fn decode_rescind_body(body: &[u8]) -> Result<Rescind, IdsetError> {
    let items = top_level(body, RESCIND_KIND, 6)?;
    Ok(Rescind {
        set_id: hash_field(&items[2])?,
        from_version: unsigned(&items[3])?,
        target: entry_field(&items[4])?,
        through: match &items[5] {
            Value::SimpleValue(simple) if simple.to_u8() == 22 => None,
            value => Some(hash_field(value)?),
        },
    })
}

/// Checks the encoded size and returns the body.
fn check_size(body: Vec<u8>) -> Result<Vec<u8>, IdsetError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(IdsetError::Oversized(body.len()));
    }
    Ok(body)
}

/// Decodes the body strictly and canonically, then splits the top array and
/// checks its tag and version. Returns the array's items.
fn top_level(
    body: &[u8],
    kind: &'static str,
    count: usize,
) -> Result<Vec<Value<'static>>, IdsetError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(IdsetError::Oversized(body.len()));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(body)
        .map_err(|err| IdsetError::Cbor(err.to_string()))?;
    if value.encode().as_slice() != body {
        return Err(IdsetError::NotCanonical);
    }
    let Value::Array(items) = value else {
        return Err(IdsetError::Shape("the body is not an array".to_owned()));
    };
    let Some(Value::TextString(tag)) = items.first() else {
        return Err(IdsetError::Shape("the body's tag is not text".to_owned()));
    };
    if tag.as_ref() != kind {
        return Err(IdsetError::Kind(tag.to_string(), kind));
    }
    if items.len() != count {
        return Err(IdsetError::Shape(format!(
            "a {kind} body has {count} items, this one has {}",
            items.len()
        )));
    }
    let version = unsigned(&items[1])?;
    if version != VERSION {
        return Err(IdsetError::Version(version));
    }
    Ok(items)
}

/// The CBOR of a hash: its 32 bytes as a byte string.
fn hash_value(hash: &Hash) -> Value<'static> {
    Value::ByteString(hash.as_bytes().to_vec().into())
}

fn entries_value(entries: &[Entry]) -> Value<'static> {
    Value::Array(entries.iter().map(entry_value).collect())
}

fn entry_value(entry: &Entry) -> Value<'static> {
    match entry {
        Entry::Key {
            author,
            include_history,
        } => Value::Array(vec![
            Value::TextString("key".into()),
            Value::ByteString(author.to_vec().into()),
            Value::from(*include_history),
        ]),
        Entry::Group {
            id,
            role_min,
            include_history,
        } => Value::Array(vec![
            Value::TextString("group".into()),
            hash_value(id),
            Value::Unsigned(role_level(*role_min)),
            Value::from(*include_history),
        ]),
        Entry::Public { include_history } => Value::Array(vec![
            Value::TextString("public".into()),
            Value::from(*include_history),
        ]),
    }
}

fn scope_value(scope: Option<&str>) -> Value<'static> {
    match scope {
        Some(scope) => Value::TextString(scope.to_owned().into()),
        None => Value::null(),
    }
}

/// The level of a role, as its unsigned integer.
fn role_level(role: Role) -> u64 {
    match role {
        Role::Pull => 0,
        Role::Read => 1,
        Role::Write => 2,
        Role::Manage => 3,
        Role::Admin => 4,
    }
}

fn unsigned(value: &Value<'_>) -> Result<u64, IdsetError> {
    match value {
        Value::Unsigned(n) => Ok(*n),
        _ => Err(IdsetError::Shape("expected an unsigned integer".to_owned())),
    }
}

fn boolean(value: &Value<'_>) -> Result<bool, IdsetError> {
    match value {
        Value::SimpleValue(simple) => match simple.to_u8() {
            20 => Ok(false),
            21 => Ok(true),
            other => Err(IdsetError::Shape(format!(
                "simple value {other} is not a boolean"
            ))),
        },
        _ => Err(IdsetError::Shape("expected a boolean".to_owned())),
    }
}

/// The 32 bytes of a byte string field.
fn bytes32(value: &Value<'_>, what: &str) -> Result<[u8; 32], IdsetError> {
    let Value::ByteString(bytes) = value else {
        return Err(IdsetError::Shape(format!("{what} is not a byte string")));
    };
    <[u8; 32]>::try_from(bytes.as_ref())
        .map_err(|_| IdsetError::Shape(format!("{what} is {} bytes, it must be 32", bytes.len())))
}

fn hash_field(value: &Value<'_>) -> Result<Hash, IdsetError> {
    Ok(Hash::from_bytes(bytes32(value, "a hash")?))
}

fn key_field(value: &Value<'_>) -> Result<AuthorKey, IdsetError> {
    bytes32(value, "a key")
}

fn scope_field(value: &Value<'_>) -> Result<Option<String>, IdsetError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        Value::TextString(text) => {
            let scope = text.to_string();
            check_scope(&scope)?;
            Ok(Some(scope))
        }
        _ => Err(IdsetError::Shape(
            "the scope is not text or null".to_owned(),
        )),
    }
}

fn check_scope(scope: &str) -> Result<(), IdsetError> {
    if scope.len() > MAX_SCOPE_BYTES {
        return Err(IdsetError::Shape(format!(
            "the scope is {} bytes, the most is {MAX_SCOPE_BYTES}",
            scope.len()
        )));
    }
    if !is_nfc(scope) {
        return Err(IdsetError::Shape("the scope is not NFC".to_owned()));
    }
    Ok(())
}

fn entries_field(value: &Value<'_>) -> Result<Vec<Entry>, IdsetError> {
    let Value::Array(items) = value else {
        return Err(IdsetError::Shape("the entries are not an array".to_owned()));
    };
    if items.len() > MAX_ENTRIES {
        return Err(IdsetError::Shape(format!(
            "{} entries, the most is {MAX_ENTRIES}",
            items.len()
        )));
    }
    items.iter().map(entry_field).collect()
}

/// Decodes one entry. An array that ends before `include_history` is the
/// legacy form: the flag decodes as `false` (spec §D3).
fn entry_field(value: &Value<'_>) -> Result<Entry, IdsetError> {
    let Value::Array(items) = value else {
        return Err(IdsetError::Shape("an entry is not an array".to_owned()));
    };
    let Some(Value::TextString(shape)) = items.first() else {
        return Err(IdsetError::Shape("an entry has no shape".to_owned()));
    };
    /// The flag at `idx`, or `false` when the array ends before it.
    fn flag(items: &[Value<'_>], idx: usize) -> Result<bool, IdsetError> {
        match items.get(idx) {
            None => Ok(false),
            Some(value) => boolean(value),
        }
    }
    fn arity(items: &[Value<'_>], shape: &str, full: usize) -> Result<(), IdsetError> {
        if items.len() > full || items.len() < full - 1 {
            return Err(IdsetError::Shape(format!(
                "a {shape} entry has {} items, it takes {} or {}",
                items.len(),
                full - 1,
                full
            )));
        }
        Ok(())
    }
    match shape.as_ref() {
        "key" => {
            arity(items, "key", 3)?;
            Ok(Entry::Key {
                author: key_field(&items[1])?,
                include_history: flag(items, 2)?,
            })
        }
        "group" => {
            arity(items, "group", 4)?;
            Ok(Entry::Group {
                id: hash_field(&items[1])?,
                role_min: role_field(&items[2])?,
                include_history: flag(items, 3)?,
            })
        }
        "public" => {
            arity(items, "public", 2)?;
            Ok(Entry::Public {
                include_history: flag(items, 1)?,
            })
        }
        other => Err(IdsetError::Shape(format!(
            "the entry shape {other:?} is unknown"
        ))),
    }
}

fn role_field(value: &Value<'_>) -> Result<Role, IdsetError> {
    match unsigned(value)? {
        0 => Ok(Role::Pull),
        1 => Ok(Role::Read),
        2 => Ok(Role::Write),
        3 => Ok(Role::Manage),
        4 => Ok(Role::Admin),
        other => Err(IdsetError::Shape(format!("role level {other} is unknown"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use p2panda_core::SigningKey;

    use super::*;
    use crate::chain::{ChainExt, decode, sign};

    fn owner() -> AuthorKey {
        std::array::from_fn(|i| u8::try_from(i).unwrap())
    }

    fn sample_set() -> WriterSet {
        WriterSet {
            set_id: WriterSet::derive_set_id(&owner(), "dog-care"),
            version: 7,
            owner: owner(),
            scope: Some("dog-care".to_owned()),
            entries: vec![
                Entry::Key {
                    author: [9u8; 32],
                    include_history: true,
                },
                Entry::Group {
                    id: Hash::digest(b"household"),
                    role_min: Role::Read,
                    include_history: false,
                },
                Entry::Public {
                    include_history: false,
                },
            ],
            excludes: Excludes(vec![Entry::Key {
                author: [4u8; 32],
                include_history: false,
            }]),
        }
    }

    #[test]
    fn writer_set_body_round_trips() {
        let set = sample_set();
        let body = encode_writer_body(&set).unwrap();
        assert_eq!(decode_writer_body(&body).unwrap(), set);
    }

    #[test]
    fn rescind_body_round_trips() {
        let rescind = Rescind {
            set_id: WriterSet::derive_set_id(&owner(), "dog-care"),
            from_version: 3,
            target: Entry::Public {
                include_history: false,
            },
            through: Some(Hash::digest(b"up-to-here")),
        };
        let body = encode_rescind_body(&rescind).unwrap();
        assert_eq!(decode_rescind_body(&body).unwrap(), rescind);

        let unbounded = Rescind {
            through: None,
            ..rescind
        };
        let body = encode_rescind_body(&unbounded).unwrap();
        assert_eq!(decode_rescind_body(&body).unwrap(), unbounded);
    }

    #[test]
    fn set_id_is_stable_and_purpose_bound() {
        let a = WriterSet::derive_set_id(&owner(), "dog-care");
        assert_eq!(a, WriterSet::derive_set_id(&owner(), "dog-care"));
        assert_ne!(a, WriterSet::derive_set_id(&owner(), "cat-care"));
        assert_ne!(a, WriterSet::derive_set_id(&[1u8; 32], "dog-care"));
    }

    /// The golden bytes of the sample set's body: a change here is a format
    /// change, not a refactor.
    #[test]
    fn writer_set_body_golden_bytes() {
        let body = encode_writer_body(&sample_set()).unwrap();
        let expected: Vec<u8> = [
            // ["idset.writer", 1, set_id, 7, owner, "dog-care", entries, excludes]
            0x88, 0x6c, b'i', b'd', b's', b'e', b't', b'.', b'w', b'r', b'i', b't', b'e', b'r',
            0x01, 0x58, 0x20,
        ]
        .into_iter()
        .chain(
            WriterSet::derive_set_id(&owner(), "dog-care")
                .as_bytes()
                .iter()
                .copied(),
        )
        .chain([0x07, 0x58, 0x20])
        .chain(owner())
        .chain([0x68, b'd', b'o', b'g', b'-', b'c', b'a', b'r', b'e', 0x83])
        .chain([0x83, 0x63, b'k', b'e', b'y', 0x58, 0x20])
        .chain([9u8; 32])
        .chain([0xf5, 0x84, 0x65, b'g', b'r', b'o', b'u', b'p', 0x58, 0x20])
        .chain(Hash::digest(b"household").as_bytes().iter().copied())
        .chain([
            0x01, 0xf4, 0x82, 0x66, b'p', b'u', b'b', b'l', b'i', b'c', 0xf4,
        ])
        .chain([0x81, 0x83, 0x63, b'k', b'e', b'y', 0x58, 0x20])
        .chain([4u8; 32])
        .chain([0xf4])
        .collect();
        assert_eq!(body, expected);
    }

    /// A body written before `include_history` existed omits the flag on every
    /// entry shape; each decodes as `false`, and re-encoding writes the flag.
    #[test]
    fn legacy_entries_decode_include_history_as_false() {
        let legacy_entry = |shape_items: Vec<Value<'static>>| Value::Array(shape_items);
        let body = Value::Array(vec![
            Value::TextString(WRITER_KIND.into()),
            Value::Unsigned(VERSION),
            hash_value(&Hash::digest(b"set")),
            Value::Unsigned(0),
            Value::ByteString(owner().to_vec().into()),
            Value::null(),
            Value::Array(vec![
                legacy_entry(vec![
                    Value::TextString("key".into()),
                    Value::ByteString([9u8; 32].to_vec().into()),
                ]),
                legacy_entry(vec![
                    Value::TextString("group".into()),
                    hash_value(&Hash::digest(b"g")),
                    Value::Unsigned(2),
                ]),
                legacy_entry(vec![Value::TextString("public".into())]),
            ]),
            Value::Array(vec![]),
        ])
        .encode();
        let set = decode_writer_body(&body).unwrap();
        assert_eq!(
            set.entries,
            vec![
                Entry::Key {
                    author: [9u8; 32],
                    include_history: false,
                },
                Entry::Group {
                    id: Hash::digest(b"g"),
                    role_min: Role::Write,
                    include_history: false,
                },
                Entry::Public {
                    include_history: false,
                },
            ]
        );
        // Re-encoding writes the flag on every entry, so the bytes change but
        // the value does not.
        let rewritten = encode_writer_body(&set).unwrap();
        assert_ne!(rewritten, body);
        assert_eq!(decode_writer_body(&rewritten).unwrap(), set);
    }

    /// The round trip through the signed path: a writer-set body signs with
    /// `chain::sign`, and the received bytes verify and decode with
    /// `chain::decode` — the existing strict path checks the set object with
    /// no new crypto.
    #[test]
    fn writer_set_signs_and_verifies_through_chain() {
        let key = SigningKey::from_bytes(&owner());
        let set = sample_set();
        let body = encode_writer_body(&set).unwrap();
        // The sample set's version is 7, so it signs at its own chain position,
        // with a backlink to the owner's previous event.
        let event = sign(
            &key,
            u32::try_from(set.version).unwrap(),
            Some(Hash::digest(b"previous")),
            Some(&body),
            ChainExt {
                kind: WRITER_KIND.to_owned(),
                group: None,
                after: vec![],
                tag: None,
                document: None,
                removes: None,
                note: None,
                link: None,
            },
        )
        .unwrap();
        let received = decode(&event.bytes).unwrap();
        assert_eq!(received.header.extensions.kind, WRITER_KIND);
        assert_eq!(received.header.payload_hash, Some(Hash::digest(&body)));
        assert_eq!(decode_writer_body(&body).unwrap(), set);
    }

    #[test]
    fn wrong_kind_and_version_are_refused() {
        let body = encode_rescind_body(&Rescind {
            set_id: Hash::digest(b"set"),
            from_version: 0,
            target: Entry::Public {
                include_history: false,
            },
            through: None,
        })
        .unwrap();
        assert!(matches!(
            decode_writer_body(&body),
            Err(IdsetError::Kind(_, WRITER_KIND))
        ));

        let mut items = Value::Array(vec![
            Value::TextString(WRITER_KIND.into()),
            Value::Unsigned(99),
            hash_value(&Hash::digest(b"set")),
            Value::Unsigned(0),
            Value::ByteString(owner().to_vec().into()),
            Value::null(),
            Value::Array(vec![]),
            Value::Array(vec![]),
        ])
        .encode();
        assert!(matches!(
            decode_writer_body(&items),
            Err(IdsetError::Version(99))
        ));
        items.clear();
    }
}
