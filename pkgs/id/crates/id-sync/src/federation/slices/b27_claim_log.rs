//! B27: the public signed append-only claims/disputes log (#10231,
//! statements S1.92–S1.98; Part B "Public claims log privacy/fairness").
//!
//! A claim is a signed event in which server X says server Y did
//! something wrong (S1.93). The subject can answer in the same log: a
//! reply names the claim it answers, so the accused gets the right of
//! reply (S1.95) with exactly the same authentication and size rules as
//! the claim it answers. Every entry is attributable and
//! non-repudiable: the signature covers the canonical bytes and the
//! signer's key must already be bound to the claimed identity (B05),
//! so an entry is evidence, never a verdict (S1.94).
//!
//! The log is append-only in effect (S1.97): there is no edit, delete,
//! or redact call. A correction is a new signed reply naming the event
//! it corrects; readers see both. Entries feed reputation as evidence
//! (S1.98), but a unilateral allegation can never move a score by
//! itself: [`Evidence::alters_reputation_score`] is false for every
//! record, and [`Evidence::reputation_delta`] is zero. Turning evidence
//! into score movement is a later reputation-slice decision, and there
//! is no first-party adjudicator on this path.
//!
//! Each event records the retention policy digest and expiry its holder
//! applies (Part B retention/quarantine rule): those fields constrain
//! the evidence, they do not promise future availability. Spam and
//! frivolous-claim protection (S1.99) is out of scope here; rate limits
//! beyond the equal per-entry size rule belong to the reputation slice
//! that consumes this evidence.
//!
//! This module is deliberately self-contained: the federation roots
//! record the slice path in `B_SLICE_REGISTRY` without declaring it
//! (slice 9110-02), so the slice's integration test includes this file
//! directly and no shared root names it yet; #9110-09 wires versioned
//! public exports after module outputs land.

use std::collections::BTreeSet;

use ed25519_dalek::{Signer, SigningKey};

use super::b05_signer_binding::{
    BindingError, BindingRegistry, ED25519_PUBLIC_KEY_LEN, ED25519_SIGNATURE_LEN,
    verify_bound_signature,
};

/// A claim or reply's unique id.
pub type EventId = [u8; 32];
/// An Ed25519 verifying key, raw.
pub type KeyBytes = [u8; ED25519_PUBLIC_KEY_LEN];
/// A SHA-256 policy digest.
pub type Digest32 = [u8; 32];

/// Domain prefix of a claim or reply's canonical bytes.
pub const CLAIM_DOMAIN: &[u8] = b"id-sync/b27-claim/v1";

/// The largest statement a claim or a reply may carry, in bytes. The
/// same limit applies to both: the reply opportunity is equal, not
/// smaller.
pub const MAX_STATEMENT_BYTES: usize = 1024;

const TAG_CLAIM: u8 = 1;
const TAG_REPLY: u8 = 2;

/// Which half of the dispute an event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    /// Server X says server Y did something wrong.
    Claim,
    /// An answer naming the claim it replies to.
    Reply,
}

impl EventKind {
    /// Every kind an event can be.
    pub const ALL: [Self; 2] = [Self::Claim, Self::Reply];

    /// The schema name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Reply => "reply",
        }
    }
}

/// The signed fields of a claim or a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventBody {
    /// The unique event id.
    pub event_id: EventId,
    /// Whether this alleges or answers.
    pub kind: EventKind,
    /// The signer's claimed identity.
    pub author: String,
    /// The key that signs the event.
    pub author_key: KeyBytes,
    /// The server the claim is about. A reply names the same subject
    /// as the claim it answers.
    pub subject: String,
    /// The claim this answers. `None` for a claim, `Some` for a reply:
    /// a claim that names a target is neither kind and is refused.
    pub in_reply_to: Option<EventId>,
    /// The allegation or the answer, as opaque bytes.
    pub statement: Vec<u8>,
    /// When the event was issued (the signer's claim, stored for audit
    /// and never trusted for ordering).
    pub issued_at: u64,
    /// The retention policy in force where this is held.
    pub policy_digest: Digest32,
    /// When the holder's retention of this evidence expires.
    pub expires_at: u64,
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ClaimError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(ClaimError::Malformed)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u64(&mut self) -> Result<u64, ClaimError> {
        self.take::<8>().map(u64::from_be_bytes)
    }

    fn string(&mut self) -> Result<String, ClaimError> {
        let len = usize::try_from(self.u64()?)
            .ok()
            .filter(|len| self.0.len() >= *len)
            .ok_or(ClaimError::Malformed)?;
        let (bytes, rest) = self.0.split_at(len);
        self.0 = rest;
        String::from_utf8(bytes.to_vec()).map_err(|_| ClaimError::Malformed)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, ClaimError> {
        let len = usize::try_from(self.u64()?)
            .ok()
            .filter(|len| self.0.len() >= *len)
            .ok_or(ClaimError::Malformed)?;
        let (bytes, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(bytes.to_vec())
    }

    const fn rest(self) -> &'a [u8] {
        self.0
    }
}

impl EventBody {
    /// The canonical bytes the signature covers.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = CLAIM_DOMAIN.to_vec();
        out.extend_from_slice(&self.event_id);
        match self.kind {
            EventKind::Claim => out.push(TAG_CLAIM),
            EventKind::Reply => {
                out.push(TAG_REPLY);
                if let Some(target) = self.in_reply_to {
                    out.extend_from_slice(&target);
                }
            }
        }
        push_field(&mut out, self.author.as_bytes());
        out.extend_from_slice(&self.author_key);
        push_field(&mut out, self.subject.as_bytes());
        push_field(&mut out, &self.statement);
        out.extend_from_slice(&self.issued_at.to_be_bytes());
        out.extend_from_slice(&self.policy_digest);
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, ClaimError> {
        reader.0 = reader
            .0
            .strip_prefix(CLAIM_DOMAIN)
            .ok_or(ClaimError::Malformed)?;
        let event_id = reader.take()?;
        let (kind, in_reply_to) = match reader.take::<1>()?[0] {
            TAG_CLAIM => (EventKind::Claim, None),
            TAG_REPLY => (EventKind::Reply, Some(reader.take()?)),
            tag => return Err(ClaimError::UnknownKind(tag)),
        };
        Ok(Self {
            event_id,
            kind,
            author: reader.string()?,
            author_key: reader.take()?,
            subject: reader.string()?,
            statement: reader.bytes()?,
            in_reply_to,
            issued_at: reader.u64()?,
            policy_digest: reader.take()?,
            expires_at: reader.u64()?,
        })
    }
}

/// A claim or reply and its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedEvent {
    /// The signed fields.
    pub body: EventBody,
    /// Ed25519 over [`EventBody::canonical_bytes`].
    pub signature: [u8; ED25519_SIGNATURE_LEN],
}

impl SignedEvent {
    /// Wire form: the canonical body bytes, then the 64-byte signature.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body.canonical_bytes();
        out.extend_from_slice(&self.signature);
        out
    }

    /// Parse the wire form. Unknown kind tags are refused, and so are
    /// trailing bytes: a longer message is not this event plus extra.
    pub fn decode(bytes: &[u8]) -> Result<Self, ClaimError> {
        let mut reader = Reader(bytes);
        let body = EventBody::read(&mut reader)?;
        let signature = reader.take()?;
        if reader.rest().is_empty() {
            Ok(Self { body, signature })
        } else {
            Err(ClaimError::Malformed)
        }
    }
}

/// Sign an event body with the author's key.
pub fn sign_event(body: &EventBody, signing: &SigningKey) -> SignedEvent {
    SignedEvent {
        body: body.clone(),
        signature: signing.sign(&body.canonical_bytes()).to_bytes(),
    }
}

/// Why an entry could not be appended, parsed, or trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// The bytes are not a canonical claim or reply.
    Malformed,
    /// A kind tag outside claim and reply.
    UnknownKind(u8),
    /// The statement is longer than [`MAX_STATEMENT_BYTES`].
    StatementTooLarge {
        /// The statement's length in bytes.
        len: usize,
        /// The limit it exceeded.
        max: usize,
    },
    /// The author identity is empty: unattributable entries are not loggable.
    EmptyAuthor,
    /// The subject is empty: a claim must name who it is about.
    EmptySubject,
    /// The statement is empty: it alleges nothing and answers nothing.
    EmptyStatement,
    /// A claim named a reply target; it is neither kind.
    ClaimHasTarget,
    /// A reply named no target; there is nothing to answer.
    ReplyMissingTarget,
    /// A reply answered an event the log does not hold.
    UnknownTarget(EventId),
    /// A reply named a different subject than the event it answers:
    /// the answer would attach to someone else's dispute.
    SubjectMismatch {
        /// The subject of the event being answered.
        target_subject: String,
        /// The subject the reply named instead.
        reply_subject: String,
    },
    /// The event id is already in the log: the entry is a duplicate,
    /// not a second event.
    DuplicateId(EventId),
    /// The signer's key binding failed: unbound key, unknown identity,
    /// or a key bound to someone else.
    Binding(BindingError),
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "malformed claim or reply"),
            Self::UnknownKind(tag) => write!(f, "unknown event kind {tag}"),
            Self::StatementTooLarge { len, max } => {
                write!(f, "statement of {len} bytes exceeds the {max}-byte limit")
            }
            Self::EmptyAuthor => write!(f, "author identity is empty"),
            Self::EmptySubject => write!(f, "subject is empty"),
            Self::EmptyStatement => write!(f, "statement is empty"),
            Self::ClaimHasTarget => write!(f, "a claim cannot name a reply target"),
            Self::ReplyMissingTarget => write!(f, "a reply must name its claim"),
            Self::UnknownTarget(_) => write!(f, "reply target is not in the log"),
            Self::SubjectMismatch {
                target_subject,
                reply_subject,
            } => write!(
                f,
                "reply subject {reply_subject} does not match the subject \
                 {target_subject} of the event it answers"
            ),
            Self::DuplicateId(_) => write!(f, "event id is already logged"),
            Self::Binding(source) => write!(f, "signer binding failed: {source}"),
        }
    }
}

impl std::error::Error for ClaimError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Binding(source) => Some(source),
            _ => None,
        }
    }
}

impl From<BindingError> for ClaimError {
    fn from(source: BindingError) -> Self {
        Self::Binding(source)
    }
}

/// A log record handed to reputation as evidence. It carries no score
/// and no verdict: entries are claims, not rulings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The logged event.
    pub event_id: EventId,
    /// Whether this alleged or answered.
    pub kind: EventKind,
    /// Who signed it.
    pub author: String,
    /// Who it is about.
    pub subject: String,
}

impl Evidence {
    /// Whether feeding this record to reputation may move a score.
    /// Always false: a unilateral allegation cannot automatically
    /// alter global reputation, and there is no first-party
    /// adjudicator on this path.
    pub const fn alters_reputation_score(&self) -> bool {
        false
    }

    /// The score movement this record applies. Always zero, by the
    /// same rule: evidence informs later reputation decisions, it does
    /// not enact them.
    pub const fn reputation_delta(&self) -> i64 {
        0
    }
}

/// The public claims/disputes log: signed events in admission order.
/// There is deliberately no remove, edit, or redact call — the log is
/// append-only in effect, and a correction is a new signed reply.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClaimLog {
    events: Vec<SignedEvent>,
    ids: BTreeSet<EventId>,
}

impl ClaimLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Verify a wire event and append it. The author's key must be
    /// bound to the claimed identity before the signature is checked
    /// (B05 order: identity first, cryptography second), and a reply
    /// is admitted only against an event this log already holds whose
    /// subject the reply also names. The target may itself be a reply:
    /// a correction is a reply naming the event it corrects, and the
    /// same-subject rule is what keeps a correction attached to the
    /// dispute it belongs to.
    pub fn append(
        &mut self,
        registry: &BindingRegistry,
        wire: &[u8],
    ) -> Result<EventId, ClaimError> {
        let event = SignedEvent::decode(wire)?;
        let body = &event.body;
        if body.author.is_empty() {
            return Err(ClaimError::EmptyAuthor);
        }
        if body.subject.is_empty() {
            return Err(ClaimError::EmptySubject);
        }
        if body.statement.is_empty() {
            return Err(ClaimError::EmptyStatement);
        }
        if body.statement.len() > MAX_STATEMENT_BYTES {
            return Err(ClaimError::StatementTooLarge {
                len: body.statement.len(),
                max: MAX_STATEMENT_BYTES,
            });
        }
        match (body.kind, body.in_reply_to) {
            // The wire grammar guarantees this consistency (a claim
            // carries no target bytes; a reply always carries 32), so
            // these arms are defensive: they pin the kind invariant
            // against future encoder changes rather than reachable
            // input.
            (EventKind::Claim, None) => {}
            (EventKind::Claim, Some(_)) => return Err(ClaimError::ClaimHasTarget),
            (EventKind::Reply, Some(target)) => {
                let Some(target_event) = self.get(&target) else {
                    return Err(ClaimError::UnknownTarget(target));
                };
                if target_event.body.subject != body.subject {
                    return Err(ClaimError::SubjectMismatch {
                        target_subject: target_event.body.subject.clone(),
                        reply_subject: body.subject.clone(),
                    });
                }
            }
            (EventKind::Reply, None) => return Err(ClaimError::ReplyMissingTarget),
        }
        verify_bound_signature(
            registry,
            &body.author,
            &body.author_key,
            &body.canonical_bytes(),
            &event.signature,
        )?;
        let event_id = body.event_id;
        if !self.ids.insert(event_id) {
            return Err(ClaimError::DuplicateId(event_id));
        }
        self.events.push(event);
        Ok(event_id)
    }

    /// The event with this id, if the log holds it.
    pub fn get(&self, event_id: &EventId) -> Option<&SignedEvent> {
        self.ids
            .contains(event_id)
            .then(|| {
                self.events
                    .iter()
                    .find(|event| event.body.event_id == *event_id)
            })
            .flatten()
    }

    /// Every logged event, in admission order.
    pub fn iter(&self) -> impl Iterator<Item = &SignedEvent> {
        self.events.iter()
    }

    /// How many events the log holds.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the log holds no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Every record as reputation evidence: attributable claims and
    /// replies with no verdict attached.
    pub fn evidence(&self) -> impl Iterator<Item = Evidence> + '_ {
        self.events.iter().map(|event| Evidence {
            event_id: event.body.event_id,
            kind: event.body.kind,
            author: event.body.author.clone(),
            subject: event.body.subject.clone(),
        })
    }
}
