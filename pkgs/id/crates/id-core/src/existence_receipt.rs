//! Existence acknowledgements bound to the shared B19 receipt schema
//! (slice A9115-existence-receipt, issue #10256; parent #9115; settled
//! design: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-a-addressing/versioned-design.md`, section "#9115 — Existence
//! acknowledgements, location harvester, privacy", A-9115.1;
//! statements S3.23–S3.25).
//!
//! An existence acknowledgement ([`ExistsAck`]) is the #9115 consumer
//! binding of Part B's B19 V2 shared receipt schema (#10250). It is
//! exactly a B19 receipt of the `verified_present` kind, restricted to
//! the fields an existence answer binds:
//!
//! - the responder identity and the key it answers with,
//! - the subject digest (the B19 payload digest of the content whose
//!   existence is asserted),
//! - the tenant/namespace scope the assertion holds in,
//! - the policy epoch and policy digest in force, and
//! - the observation time,
//!
//! plus the challenge/response presence proof that makes the presence
//! *verified*: `SHA-256("id-sync/b19-presence/v1" || challenge ||
//! content)`, so anyone holding the content can check the answer
//! ([`check_presence`]).
//!
//! The canonical bytes are B19's, byte for byte — same domain string,
//! same field order, same kind tag — so an ack written here is a B19
//! `verified_present` receipt on the wire, and a B19 receipt of that
//! kind decodes here as an ack. Receipts of the other two kinds
//! (`accepted`, `retained_until`) are refused by the ack reader
//! ([`AckError::WrongAssertion`]): they assert different things and
//! never stand in for presence. This module defines no receipt kind
//! of its own and no divergent encoding: the schema is B19's, and
//! this slice is its existence binding in `id-core`. The B19 receipt
//! type itself lives with the federation slices in `id-sync`, which
//! depends on this crate, never the reverse, so the binding is
//! expressed against the shared schema directly.
//!
//! `stored_unverified` is not an ack. It is a
//! [`StoredUnverifiedObservation`]: an unsigned local note that some
//! bytes were seen in storage. It has no wire form as an ack, cannot
//! be signed or verified as one, and
//! [`ExistenceEvidence::proves_present`] is false for it — an
//! observation of unverified storage is never proof of
//! accepted/validated content. An ack is narrow too: it asserts
//! presence at the observation time, under the stated scope and
//! policy epoch, and promises no future retention, availability,
//! authorization or public location.
//!
//! The assertion must pass the signature gate before it is evidence:
//! [`verify`] checks the validity interval and the Ed25519 signature
//! over the canonical bytes under the responder key the body binds.
//! Whether that responder is *authorized* for the scope is the
//! consuming gate's decision — the scope is the authenticated scope
//! string of the tenancy contract (#9083), consumed here as a string;
//! this module does not define tenancy. [`VerifiedAck`] exposes the
//! bound responder, scope and epoch so the gate can decide.
//!
//! Query outcomes keep `NotPresent` and `Indeterminate` as separate
//! internal results ([`ExistenceResult`]); policy may coalesce them
//! externally ([`ExistenceResult::coalesced`]).
//!
//! The crate-root export belongs to the wiring slice (9110-02 line);
//! until it names this module, the integration test includes it by
//! path, as the neighbouring ID slices do.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use sha2::{Digest, Sha256};

/// A receipt's unique id.
pub type AckId = [u8; 32];
/// A SHA-256 digest: a subject digest or a policy digest.
pub type Digest32 = [u8; 32];
/// An Ed25519 verifying key.
pub type KeyBytes = [u8; 32];
/// Logical time, as in the B19 schema (B32's `LogicalTime`).
pub type LogicalTime = u64;

/// Domain prefix of a receipt's canonical bytes (B19 schema).
pub const RECEIPT_DOMAIN: &[u8] = b"id-sync/b19-receipt/v1";
/// Domain prefix of a presence response (B19 schema).
pub const PRESENCE_DOMAIN: &[u8] = b"id-sync/b19-presence/v1";

/// The B19 kind tag of an `accepted` receipt. Never an existence ack.
pub const TAG_ACCEPTED: u8 = 1;
/// The B19 kind tag of a `retained_until` receipt. Never an ack.
pub const TAG_RETAINED_UNTIL: u8 = 2;
/// The B19 kind tag of a `verified_present` receipt: the ack kind.
pub const TAG_VERIFIED_PRESENT: u8 = 3;

/// The schema name of the one assertion an existence ack makes.
pub const ASSERTION_VERIFIED_PRESENT: &str = "verified_present";

/// The subject digest of `content`: the B19 payload digest, SHA-256
/// of the exact bytes.
#[must_use]
pub fn subject_digest(content: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher.finalize().into()
}

/// The response a holder gives to a presence challenge (B19 schema).
#[must_use]
pub fn presence_response(challenge: &[u8; 32], content: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(PRESENCE_DOMAIN);
    hasher.update(challenge);
    hasher.update(content);
    hasher.finalize().into()
}

/// The signed fields of an existence acknowledgement: a B19
/// `verified_present` receipt body, named for the existence binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistsAck {
    /// The unique receipt id.
    pub receipt_id: AckId,
    /// The challenger's nonce.
    pub challenge: [u8; 32],
    /// The responder's presence response over challenge and content.
    pub response: Digest32,
    /// The digest of the content whose existence is asserted.
    pub subject_digest: Digest32,
    /// The responder's claimed identity.
    pub responder: String,
    /// The key the responder answers with.
    pub responder_key: KeyBytes,
    /// The key of the party the ack is issued to (the challenger).
    pub recipient_key: KeyBytes,
    /// The tenant/namespace scope the assertion holds in.
    pub scope: String,
    /// When the responder observed the content (the B19 issue time).
    pub observed_at: LogicalTime,
    /// The responder's policy epoch at observation.
    pub epoch: u64,
    /// The responder's policy in force.
    pub policy_digest: Digest32,
    /// The ack is valid from this time.
    pub valid_from: LogicalTime,
    /// The ack is valid through this time.
    pub valid_until: LogicalTime,
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], AckError> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(AckError::Malformed)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u64(&mut self) -> Result<u64, AckError> {
        self.take::<8>().map(u64::from_be_bytes)
    }

    fn string(&mut self) -> Result<String, AckError> {
        let len = usize::try_from(self.u64()?)
            .ok()
            .ok_or(AckError::Malformed)?;
        if self.0.len() < len {
            return Err(AckError::Malformed);
        }
        let (bytes, rest) = self.0.split_at(len);
        self.0 = rest;
        String::from_utf8(bytes.to_vec())
            .ok()
            .ok_or(AckError::Malformed)
    }

    const fn rest(self) -> &'a [u8] {
        self.0
    }
}

impl ExistsAck {
    /// The canonical bytes the signature covers: the B19 receipt
    /// layout for the `verified_present` kind.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = RECEIPT_DOMAIN.to_vec();
        out.extend_from_slice(&self.receipt_id);
        out.push(TAG_VERIFIED_PRESENT);
        out.extend_from_slice(&self.challenge);
        out.extend_from_slice(&self.response);
        out.extend_from_slice(&self.subject_digest);
        push_field(&mut out, self.responder.as_bytes());
        out.extend_from_slice(&self.responder_key);
        out.extend_from_slice(&self.recipient_key);
        push_field(&mut out, self.scope.as_bytes());
        out.extend_from_slice(&self.observed_at.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.policy_digest);
        out.extend_from_slice(&self.valid_from.to_be_bytes());
        out.extend_from_slice(&self.valid_until.to_be_bytes());
        out
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, AckError> {
        reader.0 = reader
            .0
            .strip_prefix(RECEIPT_DOMAIN)
            .ok_or(AckError::Malformed)?;
        let receipt_id = reader.take()?;
        match reader.take::<1>()?[0] {
            TAG_VERIFIED_PRESENT => {}
            tag @ (TAG_ACCEPTED | TAG_RETAINED_UNTIL) => {
                return Err(AckError::WrongAssertion(tag));
            }
            tag => return Err(AckError::UnknownAssertion(tag)),
        }
        Ok(Self {
            receipt_id,
            challenge: reader.take()?,
            response: reader.take()?,
            subject_digest: reader.take()?,
            responder: reader.string()?,
            responder_key: reader.take()?,
            recipient_key: reader.take()?,
            scope: reader.string()?,
            observed_at: reader.u64()?,
            epoch: reader.u64()?,
            policy_digest: reader.take()?,
            valid_from: reader.u64()?,
            valid_until: reader.u64()?,
        })
    }
}

/// An existence acknowledgement and its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedAck {
    /// The signed fields.
    pub body: ExistsAck,
    /// Ed25519 over [`ExistsAck::canonical_bytes`].
    pub signature: [u8; 64],
}

impl SignedAck {
    /// Wire form: the canonical body bytes, then the 64-byte
    /// signature. Identical to the B19 wire form of a
    /// `verified_present` receipt.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body.canonical_bytes();
        out.extend_from_slice(&self.signature);
        out
    }

    /// Parse the wire form. A receipt of any other B19 kind is
    /// refused as [`AckError::WrongAssertion`], an unknown kind tag
    /// as [`AckError::UnknownAssertion`], and trailing bytes as
    /// [`AckError::Malformed`].
    ///
    /// # Errors
    ///
    /// [`AckError`] for any byte-level or kind problem.
    pub fn decode(bytes: &[u8]) -> Result<Self, AckError> {
        let mut reader = Reader(bytes);
        let body = ExistsAck::read(&mut reader)?;
        let signature = reader.take()?;
        if !reader.rest().is_empty() {
            return Err(AckError::Malformed);
        }
        Ok(Self { body, signature })
    }
}

/// Why an ack could not be issued, parsed or trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckError {
    /// The bytes are not a canonical receipt.
    Malformed,
    /// A kind tag outside the three B19 assertions.
    UnknownAssertion(u8),
    /// A B19 receipt of a different kind (`accepted` or
    /// `retained_until`): a real receipt, but not an existence ack.
    WrongAssertion(u8),
    /// The validity interval is empty.
    InvalidInterval,
    /// The responder key in the body is not a valid Ed25519 key.
    BadKey,
    /// The key is the body's key, but the signature is invalid.
    BadSignature,
    /// The ack is not yet valid.
    NotYetValid,
    /// The ack's validity has ended.
    Expired,
    /// The presence response does not match the content.
    PresenceMismatch,
}

/// Check an ack body against content the caller holds: the subject
/// digest must be the content's digest and the response must be the
/// B19 presence response over the challenge and the content.
///
/// # Errors
///
/// [`AckError::PresenceMismatch`] when either check fails.
pub fn check_presence(body: &ExistsAck, content: &[u8]) -> Result<(), AckError> {
    if subject_digest(content) == body.subject_digest
        && presence_response(&body.challenge, content) == body.response
    {
        Ok(())
    } else {
        Err(AckError::PresenceMismatch)
    }
}

/// The per-ack fields a responder is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckContext {
    /// The unique receipt id.
    pub receipt_id: AckId,
    /// The party the ack is for (the challenger).
    pub recipient_key: KeyBytes,
    /// When the content was observed.
    pub observed_at: LogicalTime,
    /// The responder's policy epoch.
    pub epoch: u64,
    /// Start of validity.
    pub valid_from: LogicalTime,
    /// End of validity.
    pub valid_until: LogicalTime,
}

/// A node that answers existence challenges as one identity.
#[derive(Debug)]
pub struct Responder {
    identity: String,
    key: SigningKey,
    policy_digest: Digest32,
}

impl Responder {
    /// A responder answering as `identity` under `policy_digest`.
    pub fn new(identity: &str, key: SigningKey, policy_digest: Digest32) -> Self {
        Self {
            identity: identity.to_owned(),
            key,
            policy_digest,
        }
    }

    /// The responder's verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> KeyBytes {
        self.key.verifying_key().to_bytes()
    }

    /// Answer a presence challenge over content this responder holds,
    /// and sign the answer as an existence ack in `scope`.
    ///
    /// # Errors
    ///
    /// [`AckError::InvalidInterval`] when the context's validity
    /// interval is empty.
    pub fn answer_challenge(
        &self,
        content: &[u8],
        challenge: [u8; 32],
        scope: &str,
        ctx: AckContext,
    ) -> Result<SignedAck, AckError> {
        if ctx.valid_from > ctx.valid_until {
            return Err(AckError::InvalidInterval);
        }
        let body = ExistsAck {
            receipt_id: ctx.receipt_id,
            challenge,
            response: presence_response(&challenge, content),
            subject_digest: subject_digest(content),
            responder: self.identity.clone(),
            responder_key: self.verifying_key(),
            recipient_key: ctx.recipient_key,
            scope: scope.to_owned(),
            observed_at: ctx.observed_at,
            epoch: ctx.epoch,
            policy_digest: self.policy_digest,
            valid_from: ctx.valid_from,
            valid_until: ctx.valid_until,
        };
        let signature = self.key.sign(&body.canonical_bytes()).to_bytes();
        Ok(SignedAck { body, signature })
    }
}

/// An ack whose validity interval and signature checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAck {
    /// The ack.
    pub ack: SignedAck,
}

impl VerifiedAck {
    /// The assertion this ack makes: always `verified_present`.
    #[must_use]
    pub const fn assertion(&self) -> &'static str {
        ASSERTION_VERIFIED_PRESENT
    }

    /// The responder identity the ack binds.
    #[must_use]
    pub fn responder(&self) -> &str {
        &self.ack.body.responder
    }

    /// The scope the assertion holds in.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.ack.body.scope
    }

    /// The policy epoch the assertion was made under.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.ack.body.epoch
    }
}

/// Verify an ack at `now`: the interval is well formed, the
/// signature covers the canonical bytes under the responder key the
/// body binds, and `now` is inside the validity interval.
///
/// # Errors
///
/// [`AckError`] classed by which check failed.
pub fn verify(ack: &SignedAck, now: LogicalTime) -> Result<VerifiedAck, AckError> {
    let body = &ack.body;
    if body.valid_from > body.valid_until {
        return Err(AckError::InvalidInterval);
    }
    let key = VerifyingKey::from_bytes(&body.responder_key).map_err(|_error| AckError::BadKey)?;
    key.verify_strict(
        &body.canonical_bytes(),
        &Signature::from_bytes(&ack.signature),
    )
    .map_err(|_error| AckError::BadSignature)?;
    if now < body.valid_from {
        return Err(AckError::NotYetValid);
    }
    if now > body.valid_until {
        return Err(AckError::Expired);
    }
    Ok(VerifiedAck { ack: ack.clone() })
}

/// A local note that some bytes were seen in storage:
/// `stored_unverified`. Not an ack: unsigned, unverified, and never
/// proof that the content was accepted or validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUnverifiedObservation {
    /// The digest of the bytes seen.
    pub subject_digest: Digest32,
    /// When they were seen.
    pub observed_at: LogicalTime,
    /// Where they were seen.
    pub location: String,
}

/// Something a node holds as existence evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistenceEvidence {
    /// A verified signed ack.
    Ack(Box<VerifiedAck>),
    /// A `stored_unverified` inventory observation.
    StoredUnverified(StoredUnverifiedObservation),
}

impl ExistenceEvidence {
    /// Whether this proves the subject present: only a verified ack
    /// does. An unverified storage observation never does.
    #[must_use]
    pub const fn proves_present(&self) -> bool {
        matches!(self, Self::Ack(_))
    }
}

/// The internal outcome of an existence query. `NotPresent` and
/// `Indeterminate` are distinct results: a responder that answered
/// "not here" is not the same as one that could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistenceResult {
    /// A verified ack proves the subject present.
    Present(VerifiedAck),
    /// The responder answered: not present.
    NotPresent,
    /// No answer could be obtained or trusted.
    Indeterminate,
}

/// The external, policy-facing outcome: policy may coalesce
/// `NotPresent` and `Indeterminate`, which must never be told apart
/// outside the internal result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoalescedExistence {
    /// A verified ack proves the subject present.
    VerifiedPresent(VerifiedAck),
    /// Not verified present, for either internal reason.
    NotVerified,
}

impl ExistenceResult {
    /// The coalesced external view of this result.
    #[must_use]
    pub fn coalesced(self) -> CoalescedExistence {
        match self {
            Self::Present(ack) => CoalescedExistence::VerifiedPresent(ack),
            Self::NotPresent | Self::Indeterminate => CoalescedExistence::NotVerified,
        }
    }
}
