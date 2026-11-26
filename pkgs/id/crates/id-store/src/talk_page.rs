//! Talk-page posts and the per-object changelog (slice 1 of #5834, #14274).
//!
//! The records follow the design record in
//! `doc/2026-11-23T21-10-00Z_design_id5834_talk_page_changelog/`:
//!
//! - a [`TalkPost`] is an immutable, author-signed record whose post id is
//!   the content hash of its canonical record. The record commits to the
//!   body by its hash, and the body travels beside the record, so a later
//!   redaction can drop the body while the post id and signature still
//!   verify. The server's receipt time travels beside it in a
//!   [`ReceivedPost`], outside the post id, so an author can name a post
//!   before it reaches a server;
//! - a [`ChangelogEntry`] is an append-only, hash-chained record per
//!   object. Exactly one actor signs each entry ([`SignedEntry`]); the
//!   server then stamps its receipt time, and the entry hash covers the
//!   signed fields, the signature and that receipt time. The next entry's
//!   `prev_hash` names that hash, so changing any byte of an earlier entry,
//!   receipt time included, breaks the chain. Like a post body, the summary
//!   is committed to by its hash and carried beside the record.
//!
//! Nothing here mutates a record: every accessor is read-only, and a
//! changed byte changes the record's identity. The one removal is
//! redaction (slice 3, #14276): a `redact` changelog entry, itself signed
//! and chained, names a post or entry and a [`RedactionReason`]. The
//! redacted body or summary may then be dropped from storage while its
//! signed hash, its author and the chain stay intact, and readers render a
//! [`Tombstone`] showing that something was there, by whom, when it was
//! removed, by whom and why. [`verify_chain`] replays one
//! object's changelog from its first entry or from any retained
//! [`Checkpoint`] and reports the first gap, broken link, foreign entry or
//! bad signature it meets; [`verify_chain_bytes`] does the same for stored
//! canonical bytes.

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use id_core::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use id_core::id_shape::{ContentId, DIGEST_BYTES, HashProfile, IdShapeError};

/// Versioned signing domain for talk-page posts.
pub const TALK_POST_DOMAIN: &str = "id.talk-post/v1";
/// Event kind that carries one post, as its canonical bytes, in the event
/// store (slice 2, #14270). The event is content: its signer is the post's
/// author, and a group-scoped object's posts carry the object's group.
pub const TALK_POST_KIND: &str = "talk.post.v1";
/// Versioned signing domain for changelog entries.
pub const CHANGELOG_ENTRY_DOMAIN: &str = "id.changelog-entry/v1";
/// Record version written by this module.
pub const RECORD_VERSION: u64 = 1;
/// Maximum byte length of an object id.
pub const MAX_OBJECT_ID_BYTES: usize = 128;
/// Maximum byte length of a post body.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Maximum byte length of a changelog summary.
pub const MAX_SUMMARY_BYTES: usize = 1024;
/// Maximum number of references one changelog entry may carry.
pub const MAX_REFS: usize = 64;

/// The hash profile post ids and entry hashes are computed under.
const HASH_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// Why a record was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TalkPageError {
    /// A field violates the record contract.
    Invalid(&'static str),
    /// Canonical record encoding or decoding failed.
    Cbor(CborError),
    /// A content identity failed to compute.
    Id(IdShapeError),
    /// The signer key bytes are not a valid Ed25519 public key.
    BadKey,
    /// The signature does not verify.
    BadSignature,
}

impl fmt::Display for TalkPageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problem) => f.write_str(problem),
            Self::Cbor(error) => write!(f, "record encoding: {error}"),
            Self::Id(error) => write!(f, "record identity: {error}"),
            Self::BadKey => f.write_str("signer key is invalid"),
            Self::BadSignature => f.write_str("signature is invalid"),
        }
    }
}

impl std::error::Error for TalkPageError {}

impl From<CborError> for TalkPageError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for TalkPageError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

/// The fields an author chooses for a new post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostDraft {
    /// The object whose talk page this post belongs to.
    pub object_id: Vec<u8>,
    /// The post this one replies to, if any.
    pub parent_post_id: Option<ContentId>,
    /// An earlier post by the same author that this one supersedes.
    pub supersedes: Option<ContentId>,
    /// The author's clock, in milliseconds since the Unix epoch.
    pub authored_at: u64,
    /// The post text.
    pub body: String,
}

/// One immutable, author-signed talk-page post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TalkPost {
    object_id: Vec<u8>,
    parent_post_id: Option<ContentId>,
    supersedes: Option<ContentId>,
    author: [u8; 32],
    authored_at: u64,
    body_hash: ContentId,
    body: Option<String>,
    signature: [u8; 64],
}

impl TalkPost {
    /// Sign a post. The signing key is the post's author.
    pub fn sign(draft: PostDraft, key: &SigningKey) -> Result<Self, TalkPageError> {
        let mut post = Self {
            object_id: draft.object_id,
            parent_post_id: draft.parent_post_id,
            supersedes: draft.supersedes,
            author: key.verifying_key().to_bytes(),
            authored_at: draft.authored_at,
            body_hash: ContentId::of_bytes(HASH_PROFILE, draft.body.as_bytes())?,
            body: Some(draft.body),
            signature: [0; 64],
        };
        post.validate()?;
        post.signature = key
            .sign(&preimage(
                TALK_POST_DOMAIN,
                &encode_canonical(&post.unsigned_value())?,
            ))
            .to_bytes();
        Ok(post)
    }

    /// Validate every field and verify the author's signature.
    pub fn verify(&self) -> Result<(), TalkPageError> {
        self.validate()?;
        let key = VerifyingKey::from_bytes(&self.author)
            .ok()
            .ok_or(TalkPageError::BadKey)?;
        key.verify_strict(
            &preimage(TALK_POST_DOMAIN, &encode_canonical(&self.unsigned_value())?),
            &Signature::from_bytes(&self.signature),
        )
        .ok()
        .ok_or(TalkPageError::BadSignature)
    }

    /// The post id: the content hash of [`Self::record_bytes`].
    pub fn post_id(&self) -> Result<ContentId, TalkPageError> {
        Ok(ContentId::of_bytes(HASH_PROFILE, &self.record_bytes()?)?)
    }

    /// The canonical signed record: every signed field (the body by its
    /// hash) plus the signature. The post id is its content hash.
    pub fn record_bytes(&self) -> Result<Vec<u8>, TalkPageError> {
        Ok(encode_canonical(&Value::Map(self.record_entries()))?)
    }

    /// Canonical bytes for storage or transport: the record plus the body,
    /// or null where the body was redacted.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, TalkPageError> {
        let mut entries = self.record_entries();
        entries.push((text("body"), self.body.as_deref().map_or(Value::Null, text)));
        Ok(encode_canonical(&Value::Map(entries))?)
    }

    /// Decode canonical bytes and verify the result.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, TalkPageError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::of(
            &value,
            &[
                "author",
                "authored_at",
                "body",
                "body_hash",
                "object_id",
                "parent_post_id",
                "signature",
                "supersedes",
                "v",
            ],
        )?;
        fields.version()?;
        let post = Self {
            object_id: fields.bytes("object_id")?.to_vec(),
            parent_post_id: optional_id(fields.get("parent_post_id")?)?,
            supersedes: optional_id(fields.get("supersedes")?)?,
            author: fields.array("author")?,
            authored_at: fields.uint("authored_at")?,
            body_hash: id_from(fields.get("body_hash")?)?,
            body: fields.optional_text("body")?,
            signature: fields.array("signature")?,
        };
        post.verify()?;
        Ok(post)
    }

    /// Check that this post may supersede `earlier`: it names `earlier`,
    /// and both are by the same author on the same object.
    pub fn check_supersedes(&self, earlier: &Self) -> Result<(), TalkPageError> {
        if self.supersedes != Some(earlier.post_id()?) {
            return Err(TalkPageError::Invalid(
                "post does not name the post it supersedes",
            ));
        }
        if self.author != earlier.author || self.object_id != earlier.object_id {
            return Err(TalkPageError::Invalid(
                "a post may only supersede its own author's post on the same object",
            ));
        }
        Ok(())
    }

    /// The object whose talk page this post belongs to.
    pub fn object_id(&self) -> &[u8] {
        &self.object_id
    }

    /// The post this one replies to, if any.
    pub const fn parent_post_id(&self) -> Option<ContentId> {
        self.parent_post_id
    }

    /// The earlier post this one supersedes, if any.
    pub const fn supersedes(&self) -> Option<ContentId> {
        self.supersedes
    }

    /// The author's public key bytes.
    pub const fn author(&self) -> &[u8; 32] {
        &self.author
    }

    /// The author's clock, in milliseconds since the Unix epoch.
    pub const fn authored_at(&self) -> u64 {
        self.authored_at
    }

    /// The post text, or `None` where it was redacted.
    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    /// The signed hash of the post text, kept after a redaction.
    pub const fn body_hash(&self) -> ContentId {
        self.body_hash
    }

    /// Stamp the server's receipt time beside this post.
    pub const fn receive(self, received_at: u64) -> ReceivedPost {
        ReceivedPost {
            post: self,
            received_at,
        }
    }

    fn validate(&self) -> Result<(), TalkPageError> {
        checked_object_id(&self.object_id)?;
        if let Some(body) = &self.body {
            if body.is_empty() || body.len() > MAX_BODY_BYTES {
                return Err(TalkPageError::Invalid(
                    "post body must be non-empty and within the size limit",
                ));
            }
            self.body_hash
                .verify(body.as_bytes())
                .ok()
                .ok_or(TalkPageError::Invalid(
                    "post body does not match its signed hash",
                ))?;
        }
        if self.supersedes.is_some() && self.supersedes == self.parent_post_id {
            return Err(TalkPageError::Invalid(
                "a post cannot reply to the post it supersedes",
            ));
        }
        Ok(())
    }

    fn unsigned_entries(&self) -> Vec<(Value, Value)> {
        vec![
            (text("author"), Value::Bytes(self.author.to_vec())),
            (text("authored_at"), Value::UInt(self.authored_at)),
            (text("body_hash"), id_value(&self.body_hash)),
            (text("object_id"), Value::Bytes(self.object_id.clone())),
            (
                text("parent_post_id"),
                optional_id_value(self.parent_post_id.as_ref()),
            ),
            (
                text("supersedes"),
                optional_id_value(self.supersedes.as_ref()),
            ),
            (text("v"), Value::UInt(RECORD_VERSION)),
        ]
    }

    fn unsigned_value(&self) -> Value {
        Value::Map(self.unsigned_entries())
    }

    fn record_entries(&self) -> Vec<(Value, Value)> {
        let mut entries = self.unsigned_entries();
        entries.push((text("signature"), Value::Bytes(self.signature.to_vec())));
        entries
    }
}

/// A post plus the time a server received it. Together with the author's
/// clock this is the record's `created_at`; the receipt time is not part
/// of the post id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedPost {
    post: TalkPost,
    received_at: u64,
}

impl ReceivedPost {
    /// The signed post.
    pub const fn post(&self) -> &TalkPost {
        &self.post
    }

    /// The server's receipt time, in milliseconds since the Unix epoch.
    pub const fn received_at(&self) -> u64 {
        self.received_at
    }
}

/// What a changelog entry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChangeKind {
    /// The object's content changed.
    Edit,
    /// A fork of the object was made.
    Fork,
    /// A mark (flag, label or status) was set.
    Mark,
    /// A merge was proposed.
    MergeProposal,
    /// A merge was carried out.
    Merge,
    /// A change was approved.
    Approval,
    /// A change was rejected.
    Reject,
    /// An editing session was recorded.
    Session,
    /// Content was redacted.
    Redact,
}

impl ChangeKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 9] = [
        Self::Edit,
        Self::Fork,
        Self::Mark,
        Self::MergeProposal,
        Self::Merge,
        Self::Approval,
        Self::Reject,
        Self::Session,
        Self::Redact,
    ];

    /// The kind's wire token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Fork => "fork",
            Self::Mark => "mark",
            Self::MergeProposal => "merge_proposal",
            Self::Merge => "merge",
            Self::Approval => "approval",
            Self::Reject => "reject",
            Self::Session => "session",
            Self::Redact => "redact",
        }
    }

    /// Parse a wire token.
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.token() == token)
    }
}

/// One reference a changelog entry carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryRef {
    /// A record named by its content hash (a post, a revision, a proposal).
    Content(ContentId),
    /// The user a proxy acts for. The proxy is the entry's actor and signs
    /// it; this names who it acted on behalf of.
    OnBehalfOf([u8; 32]),
    /// What a `redact` entry removes and why. A `redact` entry carries
    /// exactly one; no other kind carries any.
    Redacts(Redaction),
}

impl EntryRef {
    fn value(&self) -> Value {
        match self {
            Self::Content(id) => Value::Map(vec![(text("content"), id_value(id))]),
            Self::OnBehalfOf(user) => {
                Value::Map(vec![(text("on_behalf_of"), Value::Bytes(user.to_vec()))])
            }
            Self::Redacts(redaction) => {
                let (kind, target) = match redaction.target {
                    RedactionTarget::Post(id) => ("post", id),
                    RedactionTarget::Entry(id) => ("entry", id),
                };
                Value::Map(vec![(
                    text("redacts"),
                    Value::Map(vec![
                        (text("kind"), text(kind)),
                        (text("reason"), text(redaction.reason.token())),
                        (text("target"), id_value(&target)),
                    ]),
                )])
            }
        }
    }

    fn from_value(value: &Value) -> Result<Self, TalkPageError> {
        let Value::Map(entries) = value else {
            return Err(TalkPageError::Invalid("a reference must be a map"));
        };
        match entries.as_slice() {
            [(Value::Text(key), id)] if key == "content" => Ok(Self::Content(id_from(id)?)),
            [(Value::Text(key), Value::Bytes(user))] if key == "on_behalf_of" => {
                Ok(Self::OnBehalfOf(key_bytes(user)?))
            }
            [(Value::Text(key), redaction)] if key == "redacts" => {
                let fields = Fields::of(redaction, &["kind", "reason", "target"])?;
                let target = id_from(fields.get("target")?)?;
                let target = match fields.text("kind")? {
                    "post" => RedactionTarget::Post(target),
                    "entry" => RedactionTarget::Entry(target),
                    _ => return Err(TalkPageError::Invalid("unknown redaction target kind")),
                };
                let reason = RedactionReason::from_token(fields.text("reason")?)
                    .ok_or(TalkPageError::Invalid("unknown redaction reason"))?;
                Ok(Self::Redacts(Redaction { target, reason }))
            }
            _ => Err(TalkPageError::Invalid("unknown reference shape")),
        }
    }
}

/// The fields an actor chooses for a new changelog entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryDraft {
    /// Position in the object's chain, from 0.
    pub seq: u64,
    /// The object this entry belongs to.
    pub object_id: Vec<u8>,
    /// Hash of the previous entry; `None` exactly at `seq` 0.
    pub prev_hash: Option<ContentId>,
    /// What the entry records.
    pub kind: ChangeKind,
    /// The actor's clock, in milliseconds since the Unix epoch.
    pub authored_at: u64,
    /// Records the entry refers to.
    pub refs: Vec<EntryRef>,
    /// An optional one-line summary.
    pub summary: Option<String>,
}

impl EntryDraft {
    /// The first entry of an object's chain.
    pub const fn genesis(object_id: Vec<u8>, kind: ChangeKind, authored_at: u64) -> Self {
        Self {
            seq: 0,
            object_id,
            prev_hash: None,
            kind,
            authored_at,
            refs: Vec::new(),
            summary: None,
        }
    }

    /// The entry that follows `previous` in the same object's chain.
    pub fn after(
        previous: &ChangelogEntry,
        kind: ChangeKind,
        authored_at: u64,
    ) -> Result<Self, TalkPageError> {
        let seq = previous
            .signed
            .seq
            .checked_add(1)
            .ok_or(TalkPageError::Invalid("changelog sequence overflow"))?;
        Ok(Self {
            seq,
            object_id: previous.signed.object_id.clone(),
            prev_hash: Some(previous.entry_hash()?),
            kind,
            authored_at,
            refs: Vec::new(),
            summary: None,
        })
    }

    /// Replace the entry's references.
    #[must_use]
    pub fn with_refs(mut self, refs: Vec<EntryRef>) -> Self {
        self.refs = refs;
        self
    }

    /// Add the redaction a `redact` entry carries.
    #[must_use]
    pub fn with_redaction(mut self, redaction: Redaction) -> Self {
        self.refs.push(EntryRef::Redacts(redaction));
        self
    }

    /// Set the entry's summary.
    #[must_use]
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

/// A changelog entry as its one actor signed it, before a server stamps
/// its receipt time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedEntry {
    seq: u64,
    object_id: Vec<u8>,
    prev_hash: Option<ContentId>,
    kind: ChangeKind,
    actor: [u8; 32],
    authored_at: u64,
    refs: Vec<EntryRef>,
    summary_hash: Option<ContentId>,
    summary: Option<String>,
    signature: [u8; 64],
}

impl SignedEntry {
    /// Sign an entry. The signing key is the entry's one actor.
    pub fn sign(draft: EntryDraft, key: &SigningKey) -> Result<Self, TalkPageError> {
        let mut entry = Self {
            seq: draft.seq,
            object_id: draft.object_id,
            prev_hash: draft.prev_hash,
            kind: draft.kind,
            actor: key.verifying_key().to_bytes(),
            authored_at: draft.authored_at,
            refs: draft.refs,
            summary_hash: draft
                .summary
                .as_deref()
                .map(|summary| ContentId::of_bytes(HASH_PROFILE, summary.as_bytes()))
                .transpose()?,
            summary: draft.summary,
            signature: [0; 64],
        };
        entry.validate()?;
        entry.signature = key
            .sign(&preimage(
                CHANGELOG_ENTRY_DOMAIN,
                &encode_canonical(&Value::Map(entry.unsigned_entries()))?,
            ))
            .to_bytes();
        Ok(entry)
    }

    /// Validate every field and verify the actor's signature.
    pub fn verify(&self) -> Result<(), TalkPageError> {
        self.validate()?;
        let key = VerifyingKey::from_bytes(&self.actor)
            .ok()
            .ok_or(TalkPageError::BadKey)?;
        key.verify_strict(
            &preimage(
                CHANGELOG_ENTRY_DOMAIN,
                &encode_canonical(&Value::Map(self.unsigned_entries()))?,
            ),
            &Signature::from_bytes(&self.signature),
        )
        .ok()
        .ok_or(TalkPageError::BadSignature)
    }

    /// Stamp the server's receipt time, completing the entry.
    pub const fn receive(self, received_at: u64) -> ChangelogEntry {
        ChangelogEntry {
            signed: self,
            received_at,
        }
    }

    /// Position in the object's chain, from 0.
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// The object this entry belongs to.
    pub fn object_id(&self) -> &[u8] {
        &self.object_id
    }

    /// Hash of the previous entry; `None` exactly at `seq` 0.
    pub const fn prev_hash(&self) -> Option<ContentId> {
        self.prev_hash
    }

    /// What the entry records.
    pub const fn kind(&self) -> ChangeKind {
        self.kind
    }

    /// The one actor who signed the entry.
    pub const fn actor(&self) -> &[u8; 32] {
        &self.actor
    }

    /// The actor's clock, in milliseconds since the Unix epoch.
    pub const fn authored_at(&self) -> u64 {
        self.authored_at
    }

    /// Records the entry refers to.
    pub fn refs(&self) -> &[EntryRef] {
        &self.refs
    }

    /// The user a proxy actor acted for, if the entry names one.
    pub fn on_behalf_of(&self) -> Option<&[u8; 32]> {
        self.refs.iter().find_map(|reference| match reference {
            EntryRef::OnBehalfOf(user) => Some(user),
            EntryRef::Content(_) | EntryRef::Redacts(_) => None,
        })
    }

    /// What this entry redacts, if it is a `redact` entry.
    pub fn redaction(&self) -> Option<Redaction> {
        self.refs.iter().find_map(|reference| match reference {
            EntryRef::Redacts(redaction) => Some(*redaction),
            EntryRef::Content(_) | EntryRef::OnBehalfOf(_) => None,
        })
    }

    /// The optional one-line summary, or `None` where it was redacted.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// The signed hash of the summary, kept after a redaction.
    pub const fn summary_hash(&self) -> Option<ContentId> {
        self.summary_hash
    }

    fn validate(&self) -> Result<(), TalkPageError> {
        checked_object_id(&self.object_id)?;
        if (self.seq == 0) != self.prev_hash.is_none() {
            return Err(TalkPageError::Invalid(
                "only the first entry of a chain has no previous hash",
            ));
        }
        if self.refs.len() > MAX_REFS {
            return Err(TalkPageError::Invalid("too many references"));
        }
        let proxies = self
            .refs
            .iter()
            .filter(|reference| matches!(reference, EntryRef::OnBehalfOf(_)))
            .count();
        if proxies > 1 {
            return Err(TalkPageError::Invalid(
                "an entry has one actor and names at most one user it acts for",
            ));
        }
        let redactions = self
            .refs
            .iter()
            .filter(|reference| matches!(reference, EntryRef::Redacts(_)))
            .count();
        if redactions != usize::from(self.kind == ChangeKind::Redact) {
            return Err(TalkPageError::Invalid(
                "a redact entry names exactly one redaction and no other entry names any",
            ));
        }
        if self.on_behalf_of() == Some(&self.actor) {
            return Err(TalkPageError::Invalid(
                "an actor cannot act on behalf of itself",
            ));
        }
        if let Some(summary) = &self.summary {
            if summary.is_empty()
                || summary.len() > MAX_SUMMARY_BYTES
                || summary.chars().any(char::is_control)
            {
                return Err(TalkPageError::Invalid(
                    "summary must be one non-empty line within the size limit",
                ));
            }
            self.summary_hash
                .ok_or(TalkPageError::Invalid("summary has no signed hash"))?
                .verify(summary.as_bytes())
                .ok()
                .ok_or(TalkPageError::Invalid(
                    "summary does not match its signed hash",
                ))?;
        }
        Ok(())
    }

    fn unsigned_entries(&self) -> Vec<(Value, Value)> {
        vec![
            (text("actor"), Value::Bytes(self.actor.to_vec())),
            (text("authored_at"), Value::UInt(self.authored_at)),
            (text("kind"), text(self.kind.token())),
            (text("object_id"), Value::Bytes(self.object_id.clone())),
            (
                text("prev_hash"),
                optional_id_value(self.prev_hash.as_ref()),
            ),
            (
                text("refs"),
                Value::Array(self.refs.iter().map(EntryRef::value).collect()),
            ),
            (text("seq"), Value::UInt(self.seq)),
            (
                text("summary_hash"),
                optional_id_value(self.summary_hash.as_ref()),
            ),
            (text("v"), Value::UInt(RECORD_VERSION)),
        ]
    }
}

/// A complete changelog entry: the actor-signed fields plus the server's
/// receipt time. The entry hash covers all of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangelogEntry {
    signed: SignedEntry,
    received_at: u64,
}

impl ChangelogEntry {
    /// The actor-signed part of the entry.
    pub const fn signed(&self) -> &SignedEntry {
        &self.signed
    }

    /// The server's receipt time, in milliseconds since the Unix epoch.
    pub const fn received_at(&self) -> u64 {
        self.received_at
    }

    /// Validate every field and verify the actor's signature.
    pub fn verify(&self) -> Result<(), TalkPageError> {
        self.signed.verify()
    }

    /// The entry hash the next entry's `prev_hash` must name: the content
    /// hash of [`Self::record_bytes`].
    pub fn entry_hash(&self) -> Result<ContentId, TalkPageError> {
        Ok(ContentId::of_bytes(HASH_PROFILE, &self.record_bytes()?)?)
    }

    /// This entry as a replay checkpoint.
    pub fn checkpoint(&self) -> Result<Checkpoint, TalkPageError> {
        Ok(Checkpoint {
            seq: self.signed.seq,
            hash: self.entry_hash()?,
        })
    }

    /// The canonical chained record: every signed field (the summary by
    /// its hash), the signature and the receipt time.
    pub fn record_bytes(&self) -> Result<Vec<u8>, TalkPageError> {
        Ok(encode_canonical(&Value::Map(self.record_entries()))?)
    }

    /// Canonical bytes for storage or transport: the record plus the
    /// summary, or null where there is none or it was redacted.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, TalkPageError> {
        let mut entries = self.record_entries();
        entries.push((
            text("summary"),
            self.signed.summary.as_deref().map_or(Value::Null, text),
        ));
        Ok(encode_canonical(&Value::Map(entries))?)
    }

    fn record_entries(&self) -> Vec<(Value, Value)> {
        let mut entries = self.signed.unsigned_entries();
        entries.push((text("received_at"), Value::UInt(self.received_at)));
        entries.push((
            text("signature"),
            Value::Bytes(self.signed.signature.to_vec()),
        ));
        entries
    }

    /// Decode canonical bytes and verify the result.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, TalkPageError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::of(
            &value,
            &[
                "actor",
                "authored_at",
                "kind",
                "object_id",
                "prev_hash",
                "received_at",
                "refs",
                "seq",
                "signature",
                "summary",
                "summary_hash",
                "v",
            ],
        )?;
        fields.version()?;
        let Value::Array(refs) = fields.get("refs")? else {
            return Err(TalkPageError::Invalid("refs must be an array"));
        };
        let signed = SignedEntry {
            seq: fields.uint("seq")?,
            object_id: fields.bytes("object_id")?.to_vec(),
            prev_hash: optional_id(fields.get("prev_hash")?)?,
            kind: ChangeKind::from_token(fields.text("kind")?)
                .ok_or(TalkPageError::Invalid("unknown changelog kind"))?,
            actor: fields.array("actor")?,
            authored_at: fields.uint("authored_at")?,
            refs: refs
                .iter()
                .map(EntryRef::from_value)
                .collect::<Result<_, _>>()?,
            summary_hash: optional_id(fields.get("summary_hash")?)?,
            summary: fields.optional_text("summary")?,
            signature: fields.array("signature")?,
        };
        signed.verify()?;
        Ok(signed.receive(fields.uint("received_at")?))
    }
}

/// Why something was redacted: a closed set of reason codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RedactionReason {
    /// Personal information that should not be shown.
    Privacy,
    /// Harassment or other abuse.
    Abuse,
    /// Spam.
    Spam,
    /// A legal requirement.
    Legal,
    /// The original author asked for it.
    AuthorRequest,
    /// Any other reason.
    Other,
}

impl RedactionReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Privacy,
        Self::Abuse,
        Self::Spam,
        Self::Legal,
        Self::AuthorRequest,
        Self::Other,
    ];

    /// The reason's wire token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Privacy => "privacy",
            Self::Abuse => "abuse",
            Self::Spam => "spam",
            Self::Legal => "legal",
            Self::AuthorRequest => "author_request",
            Self::Other => "other",
        }
    }

    /// Parse a wire token.
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.token() == token)
    }
}

/// The record a `redact` entry removes the body or summary of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RedactionTarget {
    /// A talk-page post, by its post id.
    Post(ContentId),
    /// A changelog entry, by its entry hash.
    Entry(ContentId),
}

/// What a `redact` entry removes and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Redaction {
    /// The post or entry whose body or summary is removed.
    pub target: RedactionTarget,
    /// The reason code.
    pub reason: RedactionReason,
}

/// What a reader shows in place of a redacted body or summary. Every field
/// survives redaction: the removed text's signed hash, who wrote it and
/// when, and who removed it, when and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tombstone {
    /// The post or entry whose text was removed.
    pub target: RedactionTarget,
    /// The signed hash of the removed text.
    pub removed_hash: ContentId,
    /// Who wrote the removed text.
    pub original_actor: [u8; 32],
    /// When they wrote it, by their clock.
    pub original_at: u64,
    /// Who redacted it: the `redact` entry's actor.
    pub redacted_by: [u8; 32],
    /// When the redaction was received.
    pub redacted_at: u64,
    /// Why it was redacted.
    pub reason: RedactionReason,
}

impl fmt::Display for Tombstone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.target {
            RedactionTarget::Post(_) => "post",
            RedactionTarget::Entry(_) => "summary",
        };
        write!(
            f,
            "[{what} removed: written by {} at {}, removed by {} at {}, reason: {}]",
            short_key(&self.original_actor),
            self.original_at,
            short_key(&self.redacted_by),
            self.redacted_at,
            self.reason.token(),
        )
    }
}

/// A body or summary as a reader shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rendered<'a> {
    /// The text, unredacted.
    Text(&'a str),
    /// The text was redacted.
    Removed(Tombstone),
}

impl TalkPost {
    /// This post without its body, after checking that `redaction` is a
    /// valid `redact` entry for it. The signed record, and so the post id,
    /// is unchanged.
    pub fn redacted(&self, redaction: &ChangelogEntry) -> Result<Self, TalkPageError> {
        self.tombstone(redaction)?;
        let mut post = self.clone();
        post.body = None;
        Ok(post)
    }

    fn tombstone(&self, redaction: &ChangelogEntry) -> Result<Tombstone, TalkPageError> {
        let target = RedactionTarget::Post(self.post_id()?);
        let reason = redaction_for(redaction, &self.object_id, target)?;
        Ok(Tombstone {
            target,
            removed_hash: self.body_hash,
            original_actor: self.author,
            original_at: self.authored_at,
            redacted_by: redaction.signed.actor,
            redacted_at: redaction.received_at,
            reason,
        })
    }
}

impl ChangelogEntry {
    /// This entry without its summary, after checking that `redaction` is
    /// a later valid `redact` entry for it. The chained record, and so the
    /// entry hash and the chain, is unchanged.
    pub fn summary_redacted(&self, redaction: &Self) -> Result<Self, TalkPageError> {
        self.tombstone(redaction)?;
        let mut entry = self.clone();
        entry.signed.summary = None;
        Ok(entry)
    }

    fn tombstone(&self, redaction: &Self) -> Result<Tombstone, TalkPageError> {
        let removed_hash = self
            .signed
            .summary_hash
            .ok_or(TalkPageError::Invalid("entry has no summary to redact"))?;
        if redaction.signed.seq <= self.signed.seq {
            return Err(TalkPageError::Invalid(
                "a redaction must come after the entry it redacts",
            ));
        }
        let target = RedactionTarget::Entry(self.entry_hash()?);
        let reason = redaction_for(redaction, &self.signed.object_id, target)?;
        Ok(Tombstone {
            target,
            removed_hash,
            original_actor: self.signed.actor,
            original_at: self.signed.authored_at,
            redacted_by: redaction.signed.actor,
            redacted_at: redaction.received_at,
            reason,
        })
    }
}

/// Render a post's body against its object's changelog.
///
/// The changelog should already have been checked with [`verify_chain`];
/// each matching `redact` entry is verified again here. The first one that
/// redacts this post wins.
pub fn render_post<'a>(
    post: &'a TalkPost,
    changelog: &[ChangelogEntry],
) -> Result<Rendered<'a>, TalkPageError> {
    post.verify()?;
    let target = RedactionTarget::Post(post.post_id()?);
    if let Some(redaction) = find_redaction(changelog, target) {
        return Ok(Rendered::Removed(post.tombstone(redaction)?));
    }
    post.body()
        .map(Rendered::Text)
        .ok_or(TalkPageError::Invalid(
            "post body is missing and no redaction covers it",
        ))
}

/// Render an entry's summary against its object's changelog, as
/// [`render_post`] does for a post. `None` means the entry never had one.
pub fn render_summary<'a>(
    entry: &'a ChangelogEntry,
    changelog: &[ChangelogEntry],
) -> Result<Option<Rendered<'a>>, TalkPageError> {
    entry.verify()?;
    if entry.signed.summary_hash.is_none() {
        return Ok(None);
    }
    let target = RedactionTarget::Entry(entry.entry_hash()?);
    if let Some(redaction) = find_redaction(changelog, target) {
        return Ok(Some(Rendered::Removed(entry.tombstone(redaction)?)));
    }
    entry
        .signed
        .summary()
        .map(|summary| Some(Rendered::Text(summary)))
        .ok_or(TalkPageError::Invalid(
            "entry summary is missing and no redaction covers it",
        ))
}

fn find_redaction(
    changelog: &[ChangelogEntry],
    target: RedactionTarget,
) -> Option<&ChangelogEntry> {
    changelog.iter().find(|entry| {
        entry
            .signed
            .redaction()
            .is_some_and(|redaction| redaction.target == target)
    })
}

/// Check that `entry` is a valid `redact` entry on `object_id` for
/// `target`, and return its reason.
fn redaction_for(
    entry: &ChangelogEntry,
    object_id: &[u8],
    target: RedactionTarget,
) -> Result<RedactionReason, TalkPageError> {
    entry.verify()?;
    if entry.signed.object_id != object_id {
        return Err(TalkPageError::Invalid(
            "a redaction belongs to the redacted record's object",
        ));
    }
    match entry.signed.redaction() {
        Some(redaction) if redaction.target == target => Ok(redaction.reason),
        _ => Err(TalkPageError::Invalid(
            "entry is not a redaction of this record",
        )),
    }
}

fn short_key(key: &[u8; 32]) -> String {
    hex::encode(&key[..4])
}

/// A retained point in an object's chain to replay from: the sequence
/// number and entry hash of an entry already verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Checkpoint {
    /// The sequence number of the checkpointed entry.
    pub seq: u64,
    /// The entry hash of the checkpointed entry.
    pub hash: ContentId,
}

/// Why a chain failed to verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// An entry is missing: the chain expected `expected` and found `found`.
    Gap {
        /// The sequence number the chain expected next.
        expected: u64,
        /// The sequence number it found.
        found: u64,
    },
    /// The entry's `prev_hash` does not name the previous entry's hash, so
    /// an earlier entry (or the checkpoint) was changed.
    BrokenLink {
        /// The entry whose link is broken.
        seq: u64,
    },
    /// The entry belongs to another object's chain.
    ForeignEntry {
        /// The foreign entry's sequence number.
        seq: u64,
    },
    /// The entry itself is invalid or its signature does not verify.
    Record {
        /// The entry's sequence number.
        seq: u64,
        /// What is wrong with it.
        error: TalkPageError,
    },
    /// Stored bytes at this position do not decode to a valid entry.
    Decode {
        /// Zero-based position in the slice handed to the verifier.
        index: usize,
        /// What is wrong with the bytes.
        error: TalkPageError,
    },
    /// The chain verified, but its head is not the head the reader retained,
    /// so the last entry was changed or entries were dropped from the end.
    HeadMismatch {
        /// The head the reader retained.
        expected: Option<Checkpoint>,
        /// The head the chain verified to.
        found: Option<Checkpoint>,
    },
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gap { expected, found } => {
                write!(f, "changelog gap: expected entry {expected}, found {found}")
            }
            Self::BrokenLink { seq } => {
                write!(
                    f,
                    "changelog entry {seq} does not link to the entry before it"
                )
            }
            Self::ForeignEntry { seq } => {
                write!(f, "changelog entry {seq} belongs to another object")
            }
            Self::Record { seq, error } => write!(f, "changelog entry {seq}: {error}"),
            Self::Decode { index, error } => {
                write!(f, "stored changelog entry at position {index}: {error}")
            }
            Self::HeadMismatch { .. } => {
                f.write_str("changelog head does not match the retained head")
            }
        }
    }
}

impl std::error::Error for ChainError {}

/// Replay one object's changelog and return the verified head.
///
/// With `from` absent the chain must start at entry 0; with a retained
/// checkpoint it must start at the entry right after it and link to the
/// checkpoint's hash. Returns `from` unchanged when `entries` is empty.
pub fn verify_chain(
    object_id: &[u8],
    from: Option<Checkpoint>,
    entries: &[ChangelogEntry],
) -> Result<Option<Checkpoint>, ChainError> {
    let mut head = from;
    for entry in entries {
        let signed = entry.signed();
        let seq = signed.seq();
        let expected = head.map_or(Some(0), |checkpoint| checkpoint.seq.checked_add(1));
        if expected != Some(seq) {
            return Err(ChainError::Gap {
                expected: expected.unwrap_or(u64::MAX),
                found: seq,
            });
        }
        if signed.object_id() != object_id {
            return Err(ChainError::ForeignEntry { seq });
        }
        entry
            .verify()
            .map_err(|error| ChainError::Record { seq, error })?;
        if signed.prev_hash() != head.map(|checkpoint| checkpoint.hash) {
            return Err(ChainError::BrokenLink { seq });
        }
        head = Some(
            entry
                .checkpoint()
                .map_err(|error| ChainError::Record { seq, error })?,
        );
    }
    Ok(head)
}

/// Like [`verify_chain`], and also require the verified head to equal the
/// head the reader retained, which catches a changed or dropped last entry.
pub fn verify_chain_to_head(
    object_id: &[u8],
    from: Option<Checkpoint>,
    entries: &[ChangelogEntry],
    retained_head: Option<Checkpoint>,
) -> Result<Option<Checkpoint>, ChainError> {
    let found = verify_chain(object_id, from, entries)?;
    if found == retained_head {
        Ok(found)
    } else {
        Err(ChainError::HeadMismatch {
            expected: retained_head,
            found,
        })
    }
}

/// Decode stored canonical entries and replay them with [`verify_chain`].
pub fn verify_chain_bytes(
    object_id: &[u8],
    from: Option<Checkpoint>,
    stored: &[Vec<u8>],
) -> Result<Option<Checkpoint>, ChainError> {
    let entries = stored
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            ChangelogEntry::from_canonical_bytes(bytes)
                .map_err(|error| ChainError::Decode { index, error })
        })
        .collect::<Result<Vec<_>, _>>()?;
    verify_chain(object_id, from, &entries)
}

/// Strict reader over a decoded record map.
struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn of(value: &'a Value, allowed: &[&str]) -> Result<Self, TalkPageError> {
        let Value::Map(entries) = value else {
            return Err(TalkPageError::Invalid("record must be a map"));
        };
        for (key, _) in entries {
            if !matches!(key, Value::Text(name) if allowed.contains(&name.as_str())) {
                return Err(TalkPageError::Invalid("record has an unknown field"));
            }
        }
        Ok(Self { entries })
    }

    fn get(&self, name: &str) -> Result<&'a Value, TalkPageError> {
        self.entries
            .iter()
            .find_map(|(key, value)| match key {
                Value::Text(key) if key == name => Some(value),
                _ => None,
            })
            .ok_or(TalkPageError::Invalid("record is missing a field"))
    }

    fn uint(&self, name: &str) -> Result<u64, TalkPageError> {
        match self.get(name)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(TalkPageError::Invalid(
                "record field must be an unsigned integer",
            )),
        }
    }

    fn bytes(&self, name: &str) -> Result<&'a [u8], TalkPageError> {
        match self.get(name)? {
            Value::Bytes(value) => Ok(value),
            _ => Err(TalkPageError::Invalid("record field must be bytes")),
        }
    }

    fn text(&self, name: &str) -> Result<&'a str, TalkPageError> {
        match self.get(name)? {
            Value::Text(value) => Ok(value),
            _ => Err(TalkPageError::Invalid("record field must be text")),
        }
    }

    fn optional_text(&self, name: &str) -> Result<Option<String>, TalkPageError> {
        match self.get(name)? {
            Value::Null => Ok(None),
            Value::Text(value) => Ok(Some(value.clone())),
            _ => Err(TalkPageError::Invalid("record field must be text or null")),
        }
    }

    fn array<const N: usize>(&self, name: &str) -> Result<[u8; N], TalkPageError> {
        <[u8; N]>::try_from(self.bytes(name)?)
            .ok()
            .ok_or(TalkPageError::Invalid("record field has the wrong length"))
    }

    fn version(&self) -> Result<(), TalkPageError> {
        if self.uint("v")? == RECORD_VERSION {
            Ok(())
        } else {
            Err(TalkPageError::Invalid("unsupported record version"))
        }
    }
}

const fn checked_object_id(object_id: &[u8]) -> Result<(), TalkPageError> {
    if object_id.is_empty() || object_id.len() > MAX_OBJECT_ID_BYTES {
        Err(TalkPageError::Invalid(
            "object id must be non-empty and within the size limit",
        ))
    } else {
        Ok(())
    }
}

fn key_bytes(bytes: &[u8]) -> Result<[u8; 32], TalkPageError> {
    <[u8; 32]>::try_from(bytes)
        .ok()
        .ok_or(TalkPageError::Invalid("a user key must be 32 bytes"))
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn id_value(id: &ContentId) -> Value {
    Value::Array(vec![
        text(id.profile().token()),
        Value::Bytes(id.digest().to_vec()),
    ])
}

fn optional_id_value(id: Option<&ContentId>) -> Value {
    id.map_or(Value::Null, id_value)
}

fn id_from(value: &Value) -> Result<ContentId, TalkPageError> {
    let Value::Array(items) = value else {
        return Err(TalkPageError::Invalid(
            "a content hash must be [profile, digest]",
        ));
    };
    let [Value::Text(token), Value::Bytes(digest)] = items.as_slice() else {
        return Err(TalkPageError::Invalid(
            "a content hash must be [profile, digest]",
        ));
    };
    let profile =
        HashProfile::from_token(token).ok_or(TalkPageError::Invalid("unknown hash profile"))?;
    let digest = <[u8; DIGEST_BYTES]>::try_from(digest.as_slice())
        .ok()
        .ok_or(TalkPageError::Invalid(
            "a content hash digest has the wrong length",
        ))?;
    Ok(ContentId::new(profile, digest))
}

fn optional_id(value: &Value) -> Result<Option<ContentId>, TalkPageError> {
    match value {
        Value::Null => Ok(None),
        other => id_from(other).map(Some),
    }
}

fn preimage(domain: &str, unsigned: &[u8]) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(domain.len() + 1 + unsigned.len());
    preimage.extend_from_slice(domain.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(unsigned);
    preimage
}
