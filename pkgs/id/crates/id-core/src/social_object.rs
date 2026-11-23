//! Social object envelope and profiles in id-core (issue #13095;
//! parent #9128; program #9106, epic #9107; design
//! `doc/2026-11-23_design_id9128_social_app_reference_model.md` §2,
//! §7 and §9).
//!
//! One envelope carries every social object: immutable content in the
//! content plane plus signed relations. The data model caps nothing —
//! a comment nests on a comment to any depth — while readers are
//! bounded by policy ([`TraversalPolicy`], §7 defaults).
//!
//! # Profiles
//!
//! The design names four application profiles (§1) over five envelope
//! tags: Feed (`feed-entry`), Profile (`profile`), Board (`topic` and
//! `comment`), Reaction (`reaction`).
//!
//! # Chosen defaults (recorded, per decide-and-proceed)
//!
//! - The envelope's wire form is JSON: content IDs as the profiled
//!   text form (`content/raw-blake3-256.<hex>`), the author as
//!   lowercase hex of the 32-byte Ed25519 verifying key. Unknown JSON
//!   fields reject, matching the crate's "every field critical"
//!   envelope idiom; a canonical-CBOR form is a later slice, not
//!   this one.
//! - `schema_version` starts at 1; version 0 is malformed.
//! - `created_claim` is signer-claimed, untrusted and display-only:
//!   any `i64` validates; no range check.
//! - `succeeds_ref` edge semantics belong to #9129: this slice checks
//!   only well-formedness (a valid content ID, never self-referential).
//! - Profile-specific required fields (§2, §4, §5, §8.2):
//!   - `feed-entry` requires `parent_ref`: the surfaced object (§3).
//!   - `profile` requires `parent_ref` absent: it is a root object (§4).
//!   - `topic` requires `parent_ref`: the board it nests in (§5).
//!   - `comment` requires `parent_ref`: the topic or parent comment (§5).
//!   - `reaction` requires `parent_ref` (the target, §8.2) and an
//!     empty body: `content_ref` must be the content ID of zero bytes.
//!     Where a reaction's kind (👍 vs ❤️) lives is not named by the
//!     design; this slice carries no kind field — a follow-on.
//! - `object_id` and `content_ref` are independent IDs in this slice:
//!   no equality is required or forbidden between them. Binding the
//!   envelope to its content object is #9129's versioning territory.
//!
//! # Traversal
//!
//! [`traverse`] renders a thread page downward from a root over a
//! caller-supplied object set: the child relation is derived from
//! `parent_ref`, so no second index is stored. It is cycle-safe (a
//! visited set; repeats surface as [`TraversalEntry::BackReference`]),
//! deterministic (children in digest order), and bounded by policy:
//! max display depth 32, max 2,000 visited nodes per page, max 8 MiB
//! response (§7 defaults). A bound stops the walk with a
//! [`TraversalCursor`] the caller resumes with; the response size is
//! the exact JSON length of the emitted entries.

use std::collections::{HashMap, HashSet};

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::content_id::{
    ContentId, ContentTextForm, format_content_text, parse_content_text, raw_content_id,
};
use crate::hex::hex_encode;
use crate::id_shape::ParsePolicy;

/// The five envelope tags of the #9128 design (§2). Four application
/// profiles: Feed (`FeedEntry`), Profile (`Profile`), Board (`Topic`,
/// `Comment`), Reaction (`Reaction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocialProfile {
    /// A feed page item surfacing another social object (§3).
    FeedEntry,
    /// A root object owned by one identity (§4).
    Profile,
    /// An accepted submission to a board's `topics` list (§5).
    Topic,
    /// A social object nested under a topic or another comment (§5).
    Comment,
    /// A minimal social object targeting another object (§8.2).
    Reaction,
}

/// A social object envelope (#9128 §2).
///
/// Immutable content in the content plane plus signed relations. The
/// JSON form is the wire form: IDs as profiled text, the author as
/// lowercase hex, absent optional refs omitted, unknown fields
/// rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialObject {
    /// Content hash of the content object.
    #[serde(with = "content_id_text")]
    pub object_id: ContentId,
    /// Signer key: the author's Ed25519 verifying key, 32 raw bytes.
    #[serde(with = "author_hex")]
    pub author: [u8; 32],
    /// Signer-claimed time (Unix seconds). Untrusted, display only.
    pub created_claim: i64,
    /// The object this one nests under, if any.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "opt_content_id_text"
    )]
    pub parent_ref: Option<ContentId>,
    /// Content-plane reference for the body.
    #[serde(with = "content_id_text")]
    pub content_ref: ContentId,
    /// Profile schema version (§3–§6). Starts at 1.
    pub schema_version: u32,
    /// Version lineage, per #9129. Edge semantics are #9129's.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "opt_content_id_text"
    )]
    pub succeeds_ref: Option<ContentId>,
    /// Which application profile this object carries.
    pub profile: SocialProfile,
}

/// Why a social object envelope was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SocialObjectError {
    /// `schema_version` is 0; versions start at 1.
    #[error("schema_version must be at least 1, got {0}")]
    SchemaVersionZero(u32),
    /// The author bytes are not a valid Ed25519 verifying key.
    #[error("author is not a valid Ed25519 verifying key")]
    BadAuthorKey,
    /// The profile requires a `parent_ref` the envelope lacks.
    #[error("profile {profile:?} requires parent_ref (the {what})")]
    MissingParentRef {
        /// The profile that requires a parent.
        profile: SocialProfile,
        /// What the parent is for this profile.
        what: &'static str,
    },
    /// A profile object must not carry a `parent_ref`.
    #[error("profile {profile:?} must not carry parent_ref (it is a root object)")]
    RootMustNotNest {
        /// The profile that must be a root.
        profile: SocialProfile,
    },
    /// A reaction's body must be empty.
    #[error("reaction content_ref must be the empty-content id (empty body), got {0}")]
    ReactionBodyNotEmpty(String),
    /// An object cannot nest under itself.
    #[error("object_id must differ from parent_ref (an object cannot nest under itself)")]
    SelfParent,
    /// An object cannot succeed itself.
    #[error("object_id must differ from succeeds_ref (an object cannot succeed itself)")]
    SelfSuccessor,
    /// The envelope bytes are not well-formed envelope JSON.
    #[error("envelope JSON is malformed: {0}")]
    MalformedJson(String),
}

/// The content ID of zero bytes: the body a reaction carries (§8.2).
#[must_use]
pub fn empty_content_id() -> ContentId {
    raw_content_id(&[])
}

impl SocialObject {
    /// Check envelope well-formedness and profile-specific required
    /// fields. `succeeds_ref` is checked only for well-formedness;
    /// its edge semantics belong to #9129.
    ///
    /// # Errors
    ///
    /// [`SocialObjectError`] for a zero schema version, an invalid
    /// author key, a self-reference, or a profile rule violation.
    pub fn validate(&self) -> Result<(), SocialObjectError> {
        if self.schema_version == 0 {
            return Err(SocialObjectError::SchemaVersionZero(0));
        }
        VerifyingKey::from_bytes(&self.author)
            .map_err(|_error| SocialObjectError::BadAuthorKey)?;
        if Some(self.object_id) == self.parent_ref {
            return Err(SocialObjectError::SelfParent);
        }
        if Some(self.object_id) == self.succeeds_ref {
            return Err(SocialObjectError::SelfSuccessor);
        }
        match self.profile {
            SocialProfile::FeedEntry => {
                require_parent(self, "surfaced object")?;
            }
            SocialProfile::Profile => {
                if self.parent_ref.is_some() {
                    return Err(SocialObjectError::RootMustNotNest {
                        profile: self.profile,
                    });
                }
            }
            SocialProfile::Topic => {
                require_parent(self, "board")?;
            }
            SocialProfile::Comment => {
                require_parent(self, "topic or parent comment")?;
            }
            SocialProfile::Reaction => {
                require_parent(self, "target")?;
                if self.content_ref != empty_content_id() {
                    let text = format_content_text(&self.content_ref, ContentTextForm::Profiled)
                        .unwrap_or_else(|_error| String::from("<unformattable>"));
                    return Err(SocialObjectError::ReactionBodyNotEmpty(text));
                }
            }
        }
        Ok(())
    }

    /// Encode this envelope as JSON.
    ///
    /// # Errors
    ///
    /// [`SocialObjectError::MalformedJson`] if encoding fails (not
    /// expected for a constructed envelope).
    pub fn to_json(&self) -> Result<Vec<u8>, SocialObjectError> {
        serde_json::to_vec(self).map_err(|error| SocialObjectError::MalformedJson(error.to_string()))
    }

    /// Decode and validate an envelope from JSON. Unknown fields,
    /// missing required fields, malformed IDs, unknown profile tags
    /// and profile rule violations all reject.
    ///
    /// # Errors
    ///
    /// [`SocialObjectError::MalformedJson`] for unparseable JSON, or
    /// the validation error for a well-parsed but invalid envelope.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SocialObjectError> {
        let object: Self =
            serde_json::from_slice(bytes).map_err(|error| SocialObjectError::MalformedJson(error.to_string()))?;
        object.validate()?;
        Ok(object)
    }
}

fn require_parent(object: &SocialObject, what: &'static str) -> Result<(), SocialObjectError> {
    if object.parent_ref.is_none() {
        return Err(SocialObjectError::MissingParentRef {
            profile: object.profile,
            what,
        });
    }
    Ok(())
}

/// A content ID as profiled text (`content/raw-blake3-256.<hex>`).
mod content_id_text {
    use super::*;

    pub fn serialize<S>(id: &ContentId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let text = format_content_text(id, ContentTextForm::Profiled)
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ContentId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        parse_content_text(&text, ContentTextForm::Profiled, ParsePolicy::ALL)
            .map_err(serde::de::Error::custom)
    }
}

/// An optional content ID as profiled text, omitted when absent.
mod opt_content_id_text {
    use super::*;

    pub fn serialize<S>(id: &Option<ContentId>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match id {
            Some(id) => content_id_text::serialize(id, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<ContentId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = Option::<String>::deserialize(deserializer)?;
        match text {
            Some(text) => parse_content_text(&text, ContentTextForm::Profiled, ParsePolicy::ALL)
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

/// A list of content IDs as profiled text strings.
mod vec_content_id_text {
    use super::*;

    pub fn serialize<S>(ids: &[ContentId], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let texts: Result<Vec<String>, S::Error> = ids
            .iter()
            .map(|id| {
                format_content_text(id, ContentTextForm::Profiled)
                    .map_err(serde::ser::Error::custom)
            })
            .collect();
        texts?.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<ContentId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let texts = Vec::<String>::deserialize(deserializer)?;
        texts
            .iter()
            .map(|text| {
                parse_content_text(text, ContentTextForm::Profiled, ParsePolicy::ALL)
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

/// An author key as 64 lowercase hex characters.
mod author_hex {
    use super::*;

    pub fn serialize<S>(author: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex_encode(author))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        hex_decode_32(&text).ok_or_else(|| serde::de::Error::custom("author must be 64 lowercase hex characters"))
    }
}

/// Decode 64 lowercase hex characters into 32 bytes.
fn hex_decode_32(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, chunk) in bytes.chunks_exact(2).enumerate() {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        out[index] = (high << 4) | low;
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Default max display depth (#9128 §7): a page renders 33 levels,
/// root at depth 0 through depth 32.
pub const DEFAULT_MAX_DISPLAY_DEPTH: u32 = 32;

/// Default max visited nodes per page (#9128 §7).
pub const DEFAULT_MAX_VISITED_NODES: usize = 2_000;

/// Default max response size per page (#9128 §7): 8 MiB.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Policy bounds for one traversal page. A policy may tighten the §7
/// defaults, never silently loosen them — callers that pass a looser
/// policy do so explicitly, by constructing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraversalPolicy {
    /// Deepest depth visited; the root is depth 0.
    pub max_depth: u32,
    /// Most distinct objects visited on this page.
    pub max_nodes: usize,
    /// Most response bytes on this page (exact JSON length of the
    /// emitted entries).
    pub max_bytes: usize,
}

impl Default for TraversalPolicy {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DISPLAY_DEPTH,
            max_nodes: DEFAULT_MAX_VISITED_NODES,
            max_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

/// One emitted step of a traversal: a visited object, or a
/// back-reference where the walk met an already-visited object
/// instead of recursing into it (§7: cycles display as
/// back-references).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraversalEntry {
    /// A distinct object visited at this depth.
    Node {
        /// The visited object.
        object: SocialObject,
        /// Depth from the root (root is 0).
        depth: u32,
    },
    /// A cycle edge: this object was already visited on this page.
    BackReference {
        /// The already-visited object.
        #[serde(with = "content_id_text")]
        object_id: ContentId,
        /// Depth the repeat was found at.
        depth: u32,
    },
}

/// Which policy bound stopped a traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraversalBound {
    /// A node deeper than `max_depth` was reached.
    Depth,
    /// `max_nodes` distinct objects were already visited.
    Nodes,
    /// The next entry would push the page past `max_bytes`.
    Bytes,
}

/// One not-yet-visited object in a resume cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingItem {
    /// The object to visit next.
    #[serde(with = "content_id_text")]
    pub object_id: ContentId,
    /// The depth it would be visited at.
    pub depth: u32,
}

/// Resume state for a stopped traversal: the unvisited frontier, the
/// visited set, and the running counts, so a resume neither revisits
/// nor loses the bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalCursor {
    /// Frontier not yet visited, in pop order (last is next).
    pub pending: Vec<PendingItem>,
    /// Objects already visited on this page.
    #[serde(with = "vec_content_id_text")]
    pub visited: Vec<ContentId>,
    /// Distinct objects visited so far.
    pub visited_nodes: usize,
    /// Response bytes emitted so far.
    pub bytes: usize,
}

/// How a traversal ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraversalOutcome {
    /// The frontier emptied: every reachable object was visited.
    Complete,
    /// A policy bound stopped the walk; resume with the cursor.
    Stopped {
        /// Which bound stopped the walk.
        bound: TraversalBound,
        /// Resume state.
        cursor: TraversalCursor,
    },
}

/// The result of one traversal page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalResult {
    /// Entries emitted on this page, in visit order.
    pub entries: Vec<TraversalEntry>,
    /// Distinct objects visited on this page.
    pub visited_nodes: usize,
    /// Exact JSON length of the emitted entries.
    pub bytes: usize,
    /// Whether the walk finished or stopped on a bound.
    pub outcome: TraversalOutcome,
}

/// Why a traversal could not run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TraversalError {
    /// The root object is not in the supplied object set.
    #[error("root object {0} is not in the supplied object set")]
    RootNotFound(String),
    /// A resume cursor names an object outside the supplied set.
    #[error("resumed object {0} is not in the supplied object set")]
    UnknownObject(String),
    /// One object ID appears more than once in the supplied set.
    #[error("object {0} appears more than once in the supplied object set")]
    DuplicateObject(String),
    /// A traversal entry could not be encoded to JSON.
    #[error("traversal response could not be encoded: {0}")]
    Encoding(String),
}

fn entry_json_len(entry: &TraversalEntry) -> Result<usize, TraversalError> {
    serde_json::to_vec(entry)
        .map(|bytes| bytes.len())
        .map_err(|error| TraversalError::Encoding(error.to_string()))
}

fn stop_with(
    bound: TraversalBound,
    pending: Vec<PendingItem>,
    visited: &HashSet<ContentId>,
    visited_nodes: usize,
    bytes: usize,
    entries: Vec<TraversalEntry>,
) -> TraversalResult {
    TraversalResult {
        entries,
        visited_nodes,
        bytes,
        outcome: TraversalOutcome::Stopped {
            bound,
            cursor: TraversalCursor {
                pending,
                visited: visited.iter().copied().collect(),
                visited_nodes,
                bytes,
            },
        },
    }
}

/// Walk a thread page downward from `root` over `objects`, following
/// `parent_ref` from parent to children.
///
/// The walk is depth-first pre-order with children in digest order,
/// so the same object set always renders the same page. Repeats
/// (including `parent_ref` cycles) emit
/// [`TraversalEntry::BackReference`] instead of recursing. The walk
/// stops at the first policy bound it meets and reports a cursor;
/// pass that cursor as `resume` with the same object set to
/// continue the page. A fresh walk validates nothing about the
/// envelopes — validate at admission with [`SocialObject::validate`];
/// a resumed walk ignores `root` and continues from the cursor.
///
/// # Errors
///
/// [`TraversalError::RootNotFound`] for a fresh walk whose root is
/// absent, [`TraversalError::DuplicateObject`] for a repeated ID in
/// the set, [`TraversalError::UnknownObject`] for a resumed object
/// outside the set, [`TraversalError::Encoding`] if an entry cannot
/// be encoded.
pub fn traverse(
    objects: &[SocialObject],
    root: ContentId,
    policy: &TraversalPolicy,
    resume: Option<&TraversalCursor>,
) -> Result<TraversalResult, TraversalError> {
    let mut index: HashMap<ContentId, &SocialObject> = HashMap::with_capacity(objects.len());
    for object in objects {
        if index.insert(object.object_id, object).is_some() {
            let text = format_content_text(&object.object_id, ContentTextForm::Profiled)
                .unwrap_or_else(|_error| String::from("<unformattable>"));
            return Err(TraversalError::DuplicateObject(text));
        }
    }
    let mut children: HashMap<ContentId, Vec<ContentId>> = HashMap::new();
    for object in objects {
        if let Some(parent) = object.parent_ref {
            children.entry(parent).or_default().push(object.object_id);
        }
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|id| *id.digest());
    }

    let mut visited: HashSet<ContentId> = HashSet::with_capacity(objects.len());
    let mut visited_nodes: usize;
    let mut bytes: usize;
    let mut stack: Vec<PendingItem>;
    let mut entries: Vec<TraversalEntry> = Vec::new();
    match resume {
        Some(cursor) => {
            for id in &cursor.visited {
                if !index.contains_key(id) {
                    let text = format_content_text(id, ContentTextForm::Profiled)
                        .unwrap_or_else(|_error| String::from("<unformattable>"));
                    return Err(TraversalError::UnknownObject(text));
                }
                visited.insert(*id);
            }
            visited_nodes = cursor.visited_nodes;
            bytes = cursor.bytes;
            stack = cursor.pending.clone();
        }
        None => {
            if !index.contains_key(&root) {
                let text = format_content_text(&root, ContentTextForm::Profiled)
                    .unwrap_or_else(|_error| String::from("<unformattable>"));
                return Err(TraversalError::RootNotFound(text));
            }
            visited_nodes = 0;
            bytes = 0;
            stack = vec![PendingItem {
                object_id: root,
                depth: 0,
            }];
        }
    }

    while let Some(item) = stack.pop() {
        if item.depth > policy.max_depth {
            stack.push(item);
            return Ok(stop_with(
                TraversalBound::Depth,
                stack,
                &visited,
                visited_nodes,
                bytes,
                entries,
            ));
        }
        if visited.contains(&item.object_id) {
            let entry = TraversalEntry::BackReference {
                object_id: item.object_id,
                depth: item.depth,
            };
            let size = entry_json_len(&entry)?;
            if bytes.saturating_add(size) > policy.max_bytes {
                stack.push(item);
                return Ok(stop_with(
                    TraversalBound::Bytes,
                    stack,
                    &visited,
                    visited_nodes,
                    bytes,
                    entries,
                ));
            }
            bytes = bytes.saturating_add(size);
            entries.push(entry);
            continue;
        }
        if visited_nodes >= policy.max_nodes {
            stack.push(item);
            return Ok(stop_with(
                TraversalBound::Nodes,
                stack,
                &visited,
                visited_nodes,
                bytes,
                entries,
            ));
        }
        let Some(object) = index.get(&item.object_id) else {
            let text = format_content_text(&item.object_id, ContentTextForm::Profiled)
                .unwrap_or_else(|_error| String::from("<unformattable>"));
            return Err(TraversalError::UnknownObject(text));
        };
        let entry = TraversalEntry::Node {
            object: (*object).clone(),
            depth: item.depth,
        };
        let size = entry_json_len(&entry)?;
        if bytes.saturating_add(size) > policy.max_bytes {
            stack.push(item);
            return Ok(stop_with(
                TraversalBound::Bytes,
                stack,
                &visited,
                visited_nodes,
                bytes,
                entries,
            ));
        }
        visited.insert(item.object_id);
        visited_nodes = visited_nodes.saturating_add(1);
        bytes = bytes.saturating_add(size);
        entries.push(entry);
        if let Some(kids) = children.get(&item.object_id) {
            for child in kids.iter().rev() {
                stack.push(PendingItem {
                    object_id: *child,
                    depth: item.depth.saturating_add(1),
                });
            }
        }
    }

    Ok(TraversalResult {
        entries,
        visited_nodes,
        bytes,
        outcome: TraversalOutcome::Complete,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;

    fn author_key() -> [u8; 32] {
        *SigningKey::from_bytes(&[7u8; 32])
            .verifying_key()
            .as_bytes()
    }

    fn content_id(seed: u8) -> ContentId {
        raw_content_id(&[seed])
    }

    fn make_object(id_seed: u8, parent: Option<ContentId>, profile: SocialProfile) -> SocialObject {
        SocialObject {
            object_id: content_id(id_seed),
            author: author_key(),
            created_claim: 1_700_000_000,
            parent_ref: parent,
            content_ref: content_id(id_seed.wrapping_add(100)),
            schema_version: 1,
            succeeds_ref: None,
            profile,
        }
    }

    fn make_reaction(id_seed: u8, target: ContentId) -> SocialObject {
        let mut object = make_object(id_seed, Some(target), SocialProfile::Reaction);
        object.content_ref = empty_content_id();
        object
    }

    /// A chain of `len` comment objects: node `i` nests under node
    /// `i - 1`; node 0 is the root (a topic under no parent here, so
    /// the chain itself is the tree).
    fn chain(len: u8) -> (Vec<SocialObject>, ContentId) {
        let mut objects = Vec::new();
        let mut parent = None;
        for seed in 0..len {
            let object = make_object(seed, parent, SocialProfile::Comment);
            parent = Some(object.object_id);
            objects.push(object);
        }
        let root = objects[0].object_id;
        (objects, root)
    }

    #[test]
    fn envelope_round_trips_per_profile() {
        let board = content_id(200);
        let topic = content_id(201);
        let post = content_id(202);
        let cases = [
            make_object(10, Some(post), SocialProfile::FeedEntry),
            make_object(11, None, SocialProfile::Profile),
            make_object(12, Some(board), SocialProfile::Topic),
            make_object(13, Some(topic), SocialProfile::Comment),
            make_reaction(14, post),
        ];
        for object in cases {
            object.validate().expect("fixture must validate");
            let json = object.to_json().expect("encode must succeed");
            let back = SocialObject::from_json(&json).expect("decode must succeed");
            assert_eq!(object, back);
        }
    }

    #[test]
    fn profile_tag_spellings_are_kebab_case() {
        let mut object = make_object(20, Some(content_id(201)), SocialProfile::FeedEntry);
        let json = String::from_utf8(object.to_json().unwrap()).unwrap();
        assert!(json.contains("\"profile\":\"feed-entry\""), "{json}");
        object.profile = SocialProfile::Reaction;
        object.content_ref = empty_content_id();
        let json = String::from_utf8(object.to_json().unwrap()).unwrap();
        assert!(json.contains("\"profile\":\"reaction\""), "{json}");
    }

    #[test]
    fn rejects_unknown_profile_tag() {
        let object = make_object(30, Some(content_id(201)), SocialProfile::Comment);
        let mut json = String::from_utf8(object.to_json().unwrap()).unwrap();
        json = json.replace("\"comment\"", "\"poke\"");
        assert!(matches!(
            SocialObject::from_json(json.as_bytes()),
            Err(SocialObjectError::MalformedJson(_))
        ));
    }

    #[test]
    fn rejects_unknown_field() {
        let object = make_object(31, Some(content_id(201)), SocialProfile::Comment);
        let mut json = String::from_utf8(object.to_json().unwrap()).unwrap();
        json = json.replace("\"profile\":\"comment\"", "\"profile\":\"comment\",\"surprise\":1");
        assert!(matches!(
            SocialObject::from_json(json.as_bytes()),
            Err(SocialObjectError::MalformedJson(_))
        ));
    }

    #[test]
    fn rejects_missing_required_field() {
        let object = make_object(32, Some(content_id(201)), SocialProfile::Comment);
        let json = String::from_utf8(object.to_json().unwrap()).unwrap();
        let stripped = json.replace("\"content_ref\":", "\"content-ref\":");
        // A renamed required field is a missing required field.
        assert!(matches!(
            SocialObject::from_json(stripped.as_bytes()),
            Err(SocialObjectError::MalformedJson(_))
        ));
    }

    #[test]
    fn rejects_malformed_content_id_text() {
        let object = make_object(33, Some(content_id(201)), SocialProfile::Comment);
        let json = String::from_utf8(object.to_json().unwrap()).unwrap();
        let broken = json.replace("content/raw-blake3-256.", "content/nope.");
        assert!(matches!(
            SocialObject::from_json(broken.as_bytes()),
            Err(SocialObjectError::MalformedJson(_))
        ));
    }

    #[test]
    fn rejects_schema_version_zero() {
        let mut object = make_object(40, Some(content_id(201)), SocialProfile::Comment);
        object.schema_version = 0;
        assert_eq!(
            object.validate(),
            Err(SocialObjectError::SchemaVersionZero(0))
        );
    }

    #[test]
    fn rejects_bad_author_key() {
        let mut object = make_object(41, Some(content_id(201)), SocialProfile::Comment);
        object.author = [0u8; 32];
        assert_eq!(object.validate(), Err(SocialObjectError::BadAuthorKey));
        let short = make_object(41, Some(content_id(201)), SocialProfile::Comment);
        let json = String::from_utf8(short.to_json().unwrap()).unwrap();
        let broken = json.replace(&hex_encode(&short.author), "abc");
        assert!(matches!(
            SocialObject::from_json(broken.as_bytes()),
            Err(SocialObjectError::MalformedJson(_))
        ));
    }

    #[test]
    fn rejects_self_parent_and_self_successor() {
        let mut object = make_object(42, Some(content_id(201)), SocialProfile::Comment);
        object.parent_ref = Some(object.object_id);
        assert_eq!(object.validate(), Err(SocialObjectError::SelfParent));
        let mut object = make_object(43, Some(content_id(201)), SocialProfile::Comment);
        object.succeeds_ref = Some(object.object_id);
        assert_eq!(object.validate(), Err(SocialObjectError::SelfSuccessor));
    }

    #[test]
    fn rejects_profile_with_parent_and_requires_parent_for_the_rest() {
        let mut profile = make_object(50, None, SocialProfile::Profile);
        profile.parent_ref = Some(content_id(201));
        assert_eq!(
            profile.validate(),
            Err(SocialObjectError::RootMustNotNest {
                profile: SocialProfile::Profile
            })
        );
        for (seed, shape) in [
            (51, SocialProfile::FeedEntry),
            (52, SocialProfile::Topic),
            (53, SocialProfile::Comment),
            (54, SocialProfile::Reaction),
        ] {
            let mut object = make_object(seed, None, shape);
            if shape == SocialProfile::Reaction {
                object.content_ref = empty_content_id();
            }
            assert_eq!(
                object.validate(),
                Err(SocialObjectError::MissingParentRef {
                    profile: shape,
                    what: match shape {
                        SocialProfile::FeedEntry => "surfaced object",
                        SocialProfile::Topic => "board",
                        SocialProfile::Comment => "topic or parent comment",
                        SocialProfile::Reaction => "target",
                        SocialProfile::Profile => unreachable!(),
                    }
                }),
                "{shape:?} without parent_ref must reject"
            );
        }
    }

    #[test]
    fn rejects_reaction_with_nonempty_body() {
        let target = content_id(201);
        let object = make_object(55, Some(target), SocialProfile::Reaction);
        assert!(matches!(
            object.validate(),
            Err(SocialObjectError::ReactionBodyNotEmpty(_))
        ));
    }

    #[test]
    fn accepts_succeeds_ref_and_untrusted_created_claim() {
        // succeeds_ref edge semantics are #9129's: any well-formed ID
        // validates here. created_claim is display-only: even a
        // negative claim validates.
        let mut object = make_object(60, Some(content_id(201)), SocialProfile::Comment);
        object.succeeds_ref = Some(content_id(61));
        object.created_claim = -1;
        object.validate().expect("succeeds_ref + negative claim validate");
    }

    #[test]
    fn traversal_depth_bound_hits_exactly() {
        // 33 nodes sit at depths 0..=32: exactly at the default cap.
        let (objects, root) = chain(33);
        let policy = TraversalPolicy::default();
        let result = traverse(&objects, root, &policy, None).expect("traverse must run");
        assert_eq!(result.outcome, TraversalOutcome::Complete);
        assert_eq!(result.visited_nodes, 33);
        // One more node sits at depth 33: the walk stops on it.
        let (objects, root) = chain(34);
        let result = traverse(&objects, root, &policy, None).expect("traverse must run");
        assert_eq!(result.visited_nodes, 33);
        match result.outcome {
            TraversalOutcome::Stopped { bound, cursor } => {
                assert_eq!(bound, TraversalBound::Depth);
                assert_eq!(cursor.pending.len(), 1);
                assert_eq!(cursor.pending[0].depth, 33);
                assert_eq!(cursor.visited_nodes, 33);
            }
            TraversalOutcome::Complete => panic!("34-deep chain must hit the depth bound"),
        }
    }

    #[test]
    fn traversal_nodes_bound_hits_exactly_and_resumes() {
        // 2,000 nodes is exactly the default cap.
        let objects = wide_tree(2_000);
        let root = objects[0].object_id;
        let policy = TraversalPolicy::default();
        let result = traverse(&objects, root, &policy, None).expect("traverse must run");
        assert_eq!(result.outcome, TraversalOutcome::Complete);
        assert_eq!(result.visited_nodes, 2_000);
        // 2,001 nodes: the walk stops with the last one pending.
        let objects = wide_tree(2_001);
        let root = objects[0].object_id;
        let result = traverse(&objects, root, &policy, None).expect("traverse must run");
        assert_eq!(result.visited_nodes, 2_000);
        let cursor = match result.outcome {
            TraversalOutcome::Stopped { bound, cursor } => {
                assert_eq!(bound, TraversalBound::Nodes);
                assert_eq!(cursor.pending.len(), 1);
                cursor
            }
            TraversalOutcome::Complete => panic!("2,001 nodes must hit the nodes bound"),
        };
        // Resume with room for one more node: it visits the last one.
        let roomy = TraversalPolicy {
            max_nodes: 2_001,
            ..policy
        };
        let resumed = traverse(&objects, root, &roomy, Some(&cursor)).expect("resume must run");
        assert_eq!(resumed.outcome, TraversalOutcome::Complete);
        assert_eq!(resumed.visited_nodes, 2_001);
        assert_eq!(resumed.entries.len(), 1);
    }

    /// A root with `count - 1` direct children: a flat tree whose
    /// traversal visits every node at depth 1 after the root.
    fn wide_tree(count: usize) -> Vec<SocialObject> {
        let root = make_object(0, None, SocialProfile::Topic);
        let root_id = root.object_id;
        let mut objects = vec![root];
        for seed in 1..count {
            // Seeds wrap at u8; vary the content id by hashing the
            // seed index as two bytes so every object stays distinct.
            let mut object = make_object(1, Some(root_id), SocialProfile::Comment);
            object.object_id = raw_content_id(&[1, (seed & 0xff) as u8, (seed >> 8) as u8]);
            object.content_ref = raw_content_id(&[2, (seed & 0xff) as u8, (seed >> 8) as u8]);
            objects.push(object);
        }
        objects
    }

    #[test]
    fn traversal_bytes_bound_hits_exactly_and_resumes() {
        let (objects, root) = chain(5);
        let policy = TraversalPolicy::default();
        let full = traverse(&objects, root, &policy, None).expect("traverse must run");
        assert_eq!(full.outcome, TraversalOutcome::Complete);
        let total: usize = full
            .entries
            .iter()
            .map(|entry| serde_json::to_vec(entry).unwrap().len())
            .sum();
        assert_eq!(full.bytes, total);
        // Exactly the total: the page still completes.
        let exact = TraversalPolicy {
            max_bytes: total,
            ..policy
        };
        let result = traverse(&objects, root, &exact, None).expect("traverse must run");
        assert_eq!(result.outcome, TraversalOutcome::Complete);
        // One byte less: the walk stops on the last entry.
        let tight = TraversalPolicy {
            max_bytes: total - 1,
            ..policy
        };
        let result = traverse(&objects, root, &tight, None).expect("traverse must run");
        assert_eq!(result.visited_nodes, 4);
        let cursor = match result.outcome {
            TraversalOutcome::Stopped { bound, cursor } => {
                assert_eq!(bound, TraversalBound::Bytes);
                cursor
            }
            TraversalOutcome::Complete => panic!("one byte short must hit the bytes bound"),
        };
        let resumed = traverse(&objects, root, &exact, Some(&cursor)).expect("resume must run");
        assert_eq!(resumed.outcome, TraversalOutcome::Complete);
        assert_eq!(resumed.visited_nodes, 5);
    }

    #[test]
    fn traversal_cycle_terminates_as_back_reference() {
        let mut a = make_object(70, None, SocialProfile::Comment);
        let mut b = make_object(71, None, SocialProfile::Comment);
        a.parent_ref = Some(b.object_id);
        b.parent_ref = Some(a.object_id);
        let objects = vec![a.clone(), b.clone()];
        let policy = TraversalPolicy::default();
        let result = traverse(&objects, a.object_id, &policy, None).expect("traverse must run");
        assert_eq!(result.outcome, TraversalOutcome::Complete);
        assert_eq!(result.visited_nodes, 2);
        assert_eq!(result.entries.len(), 3);
        assert!(matches!(&result.entries[0], TraversalEntry::Node { depth: 0, .. }));
        assert!(matches!(&result.entries[1], TraversalEntry::Node { depth: 1, .. }));
        match &result.entries[2] {
            TraversalEntry::BackReference { object_id, depth } => {
                assert_eq!(*object_id, a.object_id);
                assert_eq!(*depth, 2);
            }
            TraversalEntry::Node { .. } => panic!("third entry must be a back-reference"),
        }
    }

    #[test]
    fn traversal_rejects_missing_root_and_duplicates() {
        let (objects, root) = chain(3);
        let missing = content_id(99);
        assert!(matches!(
            traverse(&objects, missing, &TraversalPolicy::default(), None),
            Err(TraversalError::RootNotFound(_))
        ));
        let mut doubled = objects.clone();
        doubled.push(objects[0].clone());
        assert!(matches!(
            traverse(&doubled, root, &TraversalPolicy::default(), None),
            Err(TraversalError::DuplicateObject(_))
        ));
    }

    #[test]
    fn traversal_order_is_deterministic_regardless_of_input_order() {
        let (objects, root) = chain(5);
        let policy = TraversalPolicy::default();
        let forward = traverse(&objects, root, &policy, None).expect("traverse must run");
        let mut reversed = objects.clone();
        reversed.reverse();
        let backward = traverse(&reversed, root, &policy, None).expect("traverse must run");
        assert_eq!(forward.entries, backward.entries);
    }

    #[test]
    fn cursor_round_trips_as_json() {
        let (objects, root) = chain(34);
        let policy = TraversalPolicy::default();
        let result = traverse(&objects, root, &policy, None).expect("traverse must run");
        let cursor = match result.outcome {
            TraversalOutcome::Stopped { cursor, .. } => cursor,
            TraversalOutcome::Complete => panic!("34-deep chain must stop"),
        };
        let json = serde_json::to_vec(&cursor).expect("cursor must encode");
        let back: TraversalCursor = serde_json::from_slice(&json).expect("cursor must decode");
        assert_eq!(cursor, back);
    }
}
