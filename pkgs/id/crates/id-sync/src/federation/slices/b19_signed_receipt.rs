//! B19: the V2 shared signed receipt schema (#10250, statements
//! S1.100-S1.104; Part B "V2 shared receipt schema").
//!
//! A receipt is a signed, non-repudiable assertion that a third party
//! can later hold against its signer. There are exactly three kinds,
//! and they never stand in for one another:
//!
//! - `accepted`: the universal ingest gate (B32) admitted this exact
//!   payload digest under the stated policy. The only way to issue one
//!   is [`Issuer::accept`], which runs the gate itself.
//! - `retained_until`: a bounded retention commitment up to a stated
//!   time. It is never perpetual: the issuer's policy caps how far ahead
//!   it may reach.
//! - `verified_present`: a challenge-verified existence assertion at a
//!   time. The receipt binds the challenge and the holder's response,
//!   `SHA-256("id-sync/b19-presence/v1" || challenge || content)`, so
//!   anyone holding the content can check it ([`check_presence`]).
//!
//! `stored_unverified` is not a receipt. It is an
//! [`InventoryObservation`]: a local note that some bytes are on disk.
//! It has no wire tag, cannot be signed or decoded as a receipt, and
//! [`Holding::admits_content`] is false for it, so it is never accepted
//! content and never a way around the signed gate.
//!
//! Every receipt binds its unique receipt id, the kind (and the kind's
//! fields), the payload digest, the signer identity and key, the
//! recipient's key, the scope, the issue time and epoch, the policy
//! digest and a validity interval. The canonical bytes are
//!
//! ```text
//! "id-sync/b19-receipt/v1"
//! receipt_id      32 bytes
//! kind            0x01 accepted
//!                 0x02 retained_until, then until (u64 BE)
//!                 0x03 verified_present, then challenge (32) and response (32)
//! payload_digest  32 bytes
//! signer          u64 BE length, then UTF-8 bytes
//! signer_key      32 bytes
//! recipient_key   32 bytes
//! scope           u64 BE length, then UTF-8 bytes
//! issued_at       u64 BE
//! epoch           u64 BE
//! policy_digest   32 bytes
//! valid_from      u64 BE
//! valid_until     u64 BE
//! ```
//!
//! and the Ed25519 signature covers exactly those bytes. Verification
//! resolves the signer's key binding (B05) before it checks the
//! signature, so a valid signature under an unbound or misattributed
//! key fails as an identity error, not a crypto one.
//!
//! This module is deliberately self-contained: the federation roots
//! record slice paths without declaring them (slice 9110-02), so the
//! slice's integration test includes this file directly and no shared
//! root names it yet; #9110-09 wires versioned public exports after
//! module outputs land.

use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use super::b05_signer_binding::{
    BindingError, BindingRegistry, ED25519_PUBLIC_KEY_LEN, ED25519_SIGNATURE_LEN, VerifiedIdentity,
    verify_bound_signature,
};
use super::b32_ingest_gate::{
    Admission, Envelope, IngestGate, IngressPath, LogicalTime, PayloadDigest, RejectReason,
    payload_digest,
};

/// A receipt's unique id.
pub type ReceiptId = [u8; 32];
/// A SHA-256 digest.
pub type Digest32 = [u8; 32];
/// An Ed25519 verifying key.
pub type KeyBytes = [u8; ED25519_PUBLIC_KEY_LEN];

/// Domain prefix of a receipt's canonical bytes.
pub const RECEIPT_DOMAIN: &[u8] = b"id-sync/b19-receipt/v1";
/// Domain prefix of a presence response.
pub const PRESENCE_DOMAIN: &[u8] = b"id-sync/b19-presence/v1";

const TAG_ACCEPTED: u8 = 1;
const TAG_RETAINED_UNTIL: u8 = 2;
const TAG_VERIFIED_PRESENT: u8 = 3;

/// The three signed assertions. There is deliberately no variant for
/// `stored_unverified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Assertion {
    /// The gate admitted the exact digest under the stated policy.
    Accepted,
    /// A bounded retention commitment.
    RetainedUntil,
    /// A challenge-verified existence assertion.
    VerifiedPresent,
}

impl Assertion {
    /// Every assertion a receipt can make.
    pub const ALL: [Self; 3] = [Self::Accepted, Self::RetainedUntil, Self::VerifiedPresent];

    /// The schema name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::RetainedUntil => "retained_until",
            Self::VerifiedPresent => "verified_present",
        }
    }
}

/// What a receipt asserts, with the fields that kind binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptClaim {
    /// The gate admitted the digest.
    Accepted,
    /// The signer commits to retain the digest until this time.
    RetainedUntil {
        /// The end of the commitment.
        until: LogicalTime,
    },
    /// The signer proved it held the content when challenged.
    VerifiedPresent {
        /// The challenger's nonce.
        challenge: [u8; 32],
        /// The holder's response over the challenge and the content.
        response: Digest32,
    },
}

impl ReceiptClaim {
    /// Which assertion this is.
    pub const fn assertion(&self) -> Assertion {
        match self {
            Self::Accepted => Assertion::Accepted,
            Self::RetainedUntil { .. } => Assertion::RetainedUntil,
            Self::VerifiedPresent { .. } => Assertion::VerifiedPresent,
        }
    }
}

/// The signed fields of a receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptBody {
    /// The unique receipt id.
    pub receipt_id: ReceiptId,
    /// What is asserted.
    pub claim: ReceiptClaim,
    /// The payload or content digest the assertion is about.
    pub payload_digest: PayloadDigest,
    /// The signer's claimed identity.
    pub signer: String,
    /// The key that signs the receipt.
    pub signer_key: KeyBytes,
    /// The key of the party the receipt is issued to.
    pub recipient_key: KeyBytes,
    /// The scope the assertion holds in.
    pub scope: String,
    /// When the receipt was issued (the signer's claim).
    pub issued_at: LogicalTime,
    /// The signer's epoch at issue.
    pub epoch: u64,
    /// The signer's policy in force.
    pub policy_digest: Digest32,
    /// The receipt is valid from this time.
    pub valid_from: LogicalTime,
    /// The receipt is valid through this time.
    pub valid_until: LogicalTime,
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ReceiptError> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(ReceiptError::Malformed)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u64(&mut self) -> Result<u64, ReceiptError> {
        self.take::<8>().map(u64::from_be_bytes)
    }

    fn string(&mut self) -> Result<String, ReceiptError> {
        let len = usize::try_from(self.u64()?)
            .ok()
            .ok_or(ReceiptError::Malformed)?;
        if self.0.len() < len {
            return Err(ReceiptError::Malformed);
        }
        let (bytes, rest) = self.0.split_at(len);
        self.0 = rest;
        String::from_utf8(bytes.to_vec())
            .ok()
            .ok_or(ReceiptError::Malformed)
    }

    const fn rest(self) -> &'a [u8] {
        self.0
    }
}

impl ReceiptBody {
    /// The canonical bytes the signature covers.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = RECEIPT_DOMAIN.to_vec();
        out.extend_from_slice(&self.receipt_id);
        match self.claim {
            ReceiptClaim::Accepted => out.push(TAG_ACCEPTED),
            ReceiptClaim::RetainedUntil { until } => {
                out.push(TAG_RETAINED_UNTIL);
                out.extend_from_slice(&until.to_be_bytes());
            }
            ReceiptClaim::VerifiedPresent {
                challenge,
                response,
            } => {
                out.push(TAG_VERIFIED_PRESENT);
                out.extend_from_slice(&challenge);
                out.extend_from_slice(&response);
            }
        }
        out.extend_from_slice(&self.payload_digest);
        push_field(&mut out, self.signer.as_bytes());
        out.extend_from_slice(&self.signer_key);
        out.extend_from_slice(&self.recipient_key);
        push_field(&mut out, self.scope.as_bytes());
        out.extend_from_slice(&self.issued_at.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.policy_digest);
        out.extend_from_slice(&self.valid_from.to_be_bytes());
        out.extend_from_slice(&self.valid_until.to_be_bytes());
        out
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, ReceiptError> {
        reader.0 = reader
            .0
            .strip_prefix(RECEIPT_DOMAIN)
            .ok_or(ReceiptError::Malformed)?;
        let receipt_id = reader.take()?;
        let claim = match reader.take::<1>()?[0] {
            TAG_ACCEPTED => ReceiptClaim::Accepted,
            TAG_RETAINED_UNTIL => ReceiptClaim::RetainedUntil {
                until: reader.u64()?,
            },
            TAG_VERIFIED_PRESENT => ReceiptClaim::VerifiedPresent {
                challenge: reader.take()?,
                response: reader.take()?,
            },
            tag => return Err(ReceiptError::UnknownAssertion(tag)),
        };
        Ok(Self {
            receipt_id,
            claim,
            payload_digest: reader.take()?,
            signer: reader.string()?,
            signer_key: reader.take()?,
            recipient_key: reader.take()?,
            scope: reader.string()?,
            issued_at: reader.u64()?,
            epoch: reader.u64()?,
            policy_digest: reader.take()?,
            valid_from: reader.u64()?,
            valid_until: reader.u64()?,
        })
    }
}

/// A receipt and its signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedReceipt {
    /// The signed fields.
    pub body: ReceiptBody,
    /// Ed25519 over [`ReceiptBody::canonical_bytes`].
    pub signature: [u8; ED25519_SIGNATURE_LEN],
}

impl SignedReceipt {
    /// Wire form: the canonical body bytes, then the 64-byte signature.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body.canonical_bytes();
        out.extend_from_slice(&self.signature);
        out
    }

    /// Parse the wire form. Any unknown kind tag, including one meant
    /// for `stored_unverified`, is refused; so are trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReceiptError> {
        let mut reader = Reader(bytes);
        let body = ReceiptBody::read(&mut reader)?;
        let signature = reader.take()?;
        if !reader.rest().is_empty() {
            return Err(ReceiptError::Malformed);
        }
        Ok(Self { body, signature })
    }
}

/// Why a receipt could not be issued, parsed or trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptError {
    /// The bytes are not a canonical receipt.
    Malformed,
    /// A kind tag outside the three assertions.
    UnknownAssertion(u8),
    /// The validity interval is empty or a retention commitment does
    /// not end after issue.
    InvalidInterval,
    /// A retention commitment reaches past the issuer's policy cap.
    UnboundedRetention {
        /// The latest end the policy allows.
        cap: LogicalTime,
    },
    /// The ingest gate refused the envelope, so nothing was accepted.
    Rejected(RejectReason),
    /// The signer's key binding failed: unbound key, unknown identity,
    /// or a key bound to someone else.
    Binding(BindingError),
    /// The key is bound to the signer, but the signature is invalid.
    BadSignature,
    /// The receipt is not yet valid.
    NotYetValid,
    /// The receipt's validity has ended.
    Expired,
    /// The presence response does not match the content.
    PresenceMismatch,
    /// Another receipt already holds this id.
    DuplicateId(ReceiptId),
}

/// The response a holder gives to a presence challenge.
pub fn presence_response(challenge: &[u8; 32], content: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(PRESENCE_DOMAIN);
    hasher.update(challenge);
    hasher.update(content);
    hasher.finalize().into()
}

/// Check a `verified_present` receipt against content the caller holds.
pub fn check_presence(body: &ReceiptBody, content: &[u8]) -> Result<(), ReceiptError> {
    let ReceiptClaim::VerifiedPresent {
        challenge,
        response,
    } = body.claim
    else {
        return Err(ReceiptError::PresenceMismatch);
    };
    if payload_digest(content) == body.payload_digest
        && presence_response(&challenge, content) == response
    {
        Ok(())
    } else {
        Err(ReceiptError::PresenceMismatch)
    }
}

/// The per-receipt fields an issuer is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptContext {
    /// The unique receipt id.
    pub receipt_id: ReceiptId,
    /// The party the receipt is for.
    pub recipient_key: KeyBytes,
    /// When it is issued.
    pub issued_at: LogicalTime,
    /// The issuer's epoch.
    pub epoch: u64,
    /// Start of validity.
    pub valid_from: LogicalTime,
    /// End of validity.
    pub valid_until: LogicalTime,
}

/// A node that signs receipts as one bound identity.
#[derive(Debug)]
pub struct Issuer {
    identity: String,
    key: SigningKey,
    policy_digest: Digest32,
    max_retention: LogicalTime,
}

impl Issuer {
    /// An issuer signing as `identity` under `policy_digest`, whose
    /// retention commitments may reach at most `max_retention` past
    /// their issue time.
    pub fn new(
        identity: &str,
        key: SigningKey,
        policy_digest: Digest32,
        max_retention: LogicalTime,
    ) -> Self {
        Self {
            identity: identity.to_owned(),
            key,
            policy_digest,
            max_retention,
        }
    }

    /// The issuer's verifying key.
    pub fn verifying_key(&self) -> KeyBytes {
        self.key.verifying_key().to_bytes()
    }

    fn sign(
        &self,
        claim: ReceiptClaim,
        payload_digest: PayloadDigest,
        scope: &str,
        ctx: ReceiptContext,
    ) -> Result<SignedReceipt, ReceiptError> {
        if ctx.valid_from > ctx.valid_until {
            return Err(ReceiptError::InvalidInterval);
        }
        let body = ReceiptBody {
            receipt_id: ctx.receipt_id,
            claim,
            payload_digest,
            signer: self.identity.clone(),
            signer_key: self.verifying_key(),
            recipient_key: ctx.recipient_key,
            scope: scope.to_owned(),
            issued_at: ctx.issued_at,
            epoch: ctx.epoch,
            policy_digest: self.policy_digest,
            valid_from: ctx.valid_from,
            valid_until: ctx.valid_until,
        };
        let signature = self.key.sign(&body.canonical_bytes()).to_bytes();
        Ok(SignedReceipt { body, signature })
    }

    /// Run `envelope` through the gate and, only if it is admitted,
    /// sign an `accepted` receipt for its exact payload digest and
    /// scope. This is the one way to issue `accepted`.
    pub fn accept(
        &self,
        gate: &mut IngestGate,
        path: IngressPath,
        envelope: &Envelope,
        ctx: ReceiptContext,
    ) -> Result<(Admission, SignedReceipt), ReceiptError> {
        if ctx.valid_from > ctx.valid_until {
            return Err(ReceiptError::InvalidInterval);
        }
        let admission = gate
            .admit(path, envelope, ctx.issued_at)
            .map_err(ReceiptError::Rejected)?;
        let receipt = self.sign(
            ReceiptClaim::Accepted,
            envelope.payload_digest,
            &admission.scope,
            ctx,
        )?;
        Ok((admission, receipt))
    }

    /// Commit to retain `payload_digest` until `until`. The commitment
    /// must end after issue and no later than the policy cap.
    pub fn retain_until(
        &self,
        payload_digest: PayloadDigest,
        scope: &str,
        until: LogicalTime,
        ctx: ReceiptContext,
    ) -> Result<SignedReceipt, ReceiptError> {
        if until <= ctx.issued_at {
            return Err(ReceiptError::InvalidInterval);
        }
        let cap = ctx.issued_at.saturating_add(self.max_retention);
        if until > cap {
            return Err(ReceiptError::UnboundedRetention { cap });
        }
        self.sign(
            ReceiptClaim::RetainedUntil { until },
            payload_digest,
            scope,
            ctx,
        )
    }

    /// Answer a presence challenge over content the issuer holds, and
    /// sign the answer as a `verified_present` receipt.
    pub fn verify_present(
        &self,
        content: &[u8],
        challenge: [u8; 32],
        scope: &str,
        ctx: ReceiptContext,
    ) -> Result<SignedReceipt, ReceiptError> {
        let claim = ReceiptClaim::VerifiedPresent {
            challenge,
            response: presence_response(&challenge, content),
        };
        self.sign(claim, payload_digest(content), scope, ctx)
    }
}

/// A receipt whose signer binding, signature and validity checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedReceipt {
    /// The receipt.
    pub receipt: SignedReceipt,
    /// The bound identity that signed it.
    pub identity: VerifiedIdentity,
}

impl VerifiedReceipt {
    /// Which assertion it makes.
    pub const fn assertion(&self) -> Assertion {
        self.receipt.body.claim.assertion()
    }
}

/// Verify a receipt at `now`: the interval is well formed, the signing
/// key is bound to the claimed signer (checked before the signature),
/// the signature covers the canonical bytes, and `now` is inside the
/// validity interval.
pub fn verify(
    registry: &BindingRegistry,
    receipt: &SignedReceipt,
    now: LogicalTime,
) -> Result<VerifiedReceipt, ReceiptError> {
    let body = &receipt.body;
    if body.valid_from > body.valid_until {
        return Err(ReceiptError::InvalidInterval);
    }
    if let ReceiptClaim::RetainedUntil { until } = body.claim
        && until <= body.issued_at
    {
        return Err(ReceiptError::InvalidInterval);
    }
    let identity = verify_bound_signature(
        registry,
        &body.signer,
        &body.signer_key,
        &body.canonical_bytes(),
        &receipt.signature,
    )
    .map_err(|error| match error {
        BindingError::BadSignature => ReceiptError::BadSignature,
        other => ReceiptError::Binding(other),
    })?;
    if now < body.valid_from {
        return Err(ReceiptError::NotYetValid);
    }
    if now > body.valid_until {
        return Err(ReceiptError::Expired);
    }
    Ok(VerifiedReceipt {
        receipt: receipt.clone(),
        identity,
    })
}

/// A local note that some bytes are on disk. Not a receipt: unsigned,
/// unverified, and never accepted content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryObservation {
    /// The digest of the bytes seen.
    pub payload_digest: PayloadDigest,
    /// When they were seen.
    pub observed_at: LogicalTime,
    /// Where they were seen.
    pub location: String,
}

/// Something a node holds evidence of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holding {
    /// A verified signed receipt.
    Receipt(Box<VerifiedReceipt>),
    /// The `stored_unverified` inventory observation.
    StoredUnverified(InventoryObservation),
}

impl Holding {
    /// Whether this admits the digest as accepted content: only a
    /// verified `accepted` receipt does.
    pub const fn admits_content(&self) -> bool {
        matches!(self, Self::Receipt(r) if matches!(r.receipt.body.claim, ReceiptClaim::Accepted))
    }
}

/// Retained receipts, by unique id, so they can be produced as
/// evidence later.
#[derive(Debug, Clone, Default)]
pub struct ReceiptLedger {
    receipts: BTreeMap<ReceiptId, VerifiedReceipt>,
}

impl ReceiptLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retain a verified receipt. Recording the same receipt twice is a
    /// no-op; a different receipt under a held id is refused.
    pub fn record(&mut self, receipt: VerifiedReceipt) -> Result<(), ReceiptError> {
        let id = receipt.receipt.body.receipt_id;
        match self.receipts.get(&id) {
            Some(held) if held.receipt == receipt.receipt => Ok(()),
            Some(_) => Err(ReceiptError::DuplicateId(id)),
            None => {
                self.receipts.insert(id, receipt);
                Ok(())
            }
        }
    }

    /// A retained receipt, by id.
    pub fn get(&self, id: &ReceiptId) -> Option<&VerifiedReceipt> {
        self.receipts.get(id)
    }

    /// How many receipts are retained.
    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    /// Whether nothing is retained.
    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }
}
