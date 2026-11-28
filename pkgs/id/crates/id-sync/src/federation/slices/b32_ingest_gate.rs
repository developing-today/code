//! B32: the mandatory universal signed-and-validated acceptance
//! gate (#10233, statements S1.12-S1.20).
//!
//! Every ingress path - local, loopback, internal, hosted, and
//! remote - passes through this one gate before admission, and no
//! path is exempt: a local producer signs before admission and is
//! checked by exactly the same rules as a remote peer. The gate
//! enforces, in order, before any state is admitted:
//!
//! 1. signature presence (an unsigned envelope is not an assertion);
//! 2. signer binding (the key must be bound to the claimed signer
//!    identity - signature presence alone is not identity, B05);
//! 3. signature validity over the canonical preimage;
//! 4. semantic validation (the declared payload digest must match
//!    the payload bytes; empty payloads and scopes are invalid);
//! 5. authorization (the bound signer must be authorized for the
//!    envelope's scope);
//! 6. replay and freshness (an already-admitted event id is a
//!    replay; envelopes outside the freshness window are refused).
//!
//! Configuration cannot disable this gate: it has no bypass flag,
//! no path-based exemption, and it fails closed - an unknown signer
//! is rejected, never admitted provisionally.
//!
//! This module is deliberately self-contained: the federation
//! roots record the slice path without declaring it (slice
//! 9110-02), so the slice's integration test includes this file
//! directly and no shared root names it yet; #9110-09 wires
//! versioned public exports after module outputs land.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

/// Length of an Ed25519 verifying key, in bytes.
pub const ED25519_PUBLIC_KEY_LEN: usize = 32;
/// Length of an Ed25519 signature, in bytes.
pub const ED25519_SIGNATURE_LEN: usize = 64;

/// Logical seconds, supplied by the caller.
pub type LogicalTime = u64;
/// The stable identity of an envelope: a caller-chosen unique id
/// bound into the signed preimage, so a replay cannot rebind it.
pub type EventId = [u8; 32];
/// Digest of the payload bytes (SHA-256).
pub type PayloadDigest = [u8; 32];

/// The ingress path an envelope arrived by. The gate's rules are
/// identical for every variant - the path is recorded on the
/// admission for observability, never used to relax a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IngressPath {
    /// Produced by a local producer on this node.
    Local,
    /// Arrived over a loopback interface.
    Loopback,
    /// Arrived from an internal (same-fleet) peer.
    Internal,
    /// Arrived via a hosted service peer.
    Hosted,
    /// Arrived from a remote peer.
    Remote,
}

impl IngressPath {
    /// Every ingress path, for matrix-style checks.
    pub const ALL: [Self; 5] = [
        Self::Local,
        Self::Loopback,
        Self::Internal,
        Self::Hosted,
        Self::Remote,
    ];
}

/// The digest binding a payload's bytes.
pub fn payload_digest(payload: &[u8]) -> PayloadDigest {
    let mut hasher = Sha256::new();
    hasher.update(payload);
    hasher.finalize().into()
}

/// One envelope presented to the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The claimed signer identity.
    pub signer: String,
    /// The verifying key the envelope claims for its signer.
    pub verifying_key: [u8; ED25519_PUBLIC_KEY_LEN],
    /// The signature over the envelope's preimage, or `None` when
    /// the envelope is unsigned.
    pub signature: Option<[u8; ED25519_SIGNATURE_LEN]>,
    /// The scope the envelope asks to be admitted into.
    pub scope: String,
    /// The payload bytes.
    pub payload: Vec<u8>,
    /// The declared digest of `payload`.
    pub payload_digest: PayloadDigest,
    /// The unique event id (replay identity).
    pub event_id: EventId,
    /// When the envelope was issued, in logical seconds.
    pub issued_at: LogicalTime,
}

impl Envelope {
    /// The canonical signed preimage: domain-separated, with every
    /// field that admission depends on bound into it, so mutating
    /// any signed field after signing invalidates the signature.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"id-sync/b32-ingest-gate/v1");
        push_field(&mut out, self.signer.as_bytes());
        push_field(&mut out, &self.verifying_key);
        push_field(&mut out, self.scope.as_bytes());
        push_field(&mut out, &self.payload_digest);
        push_field(&mut out, &self.event_id);
        push_field(&mut out, &self.issued_at.to_be_bytes());
        out
    }
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

/// A signer the gate knows: an identity, the key bound to it, and
/// the scopes it is authorized to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerAuthorization {
    /// The signer identity.
    pub identity: String,
    /// The Ed25519 verifying key bound to that identity.
    pub verifying_key: [u8; ED25519_PUBLIC_KEY_LEN],
    /// The scopes this signer may be admitted into.
    pub scopes: BTreeSet<String>,
}

/// The freshness bounds for admission, in logical seconds
/// (mirrors the B03 replay guard's window semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshnessWindow {
    /// An envelope stays fresh while `now <= issued_at + max_age`.
    pub max_age: LogicalTime,
    /// How far ahead of `now` an `issued_at` may be.
    pub max_future_skew: LogicalTime,
}

/// Why the gate refused an envelope. Admission is all-or-nothing:
/// a refused envelope changes no gate state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// No signature was presented.
    Unsigned,
    /// The verifying key is bound to no known signer.
    UnknownSigner,
    /// The key is bound to a different identity than claimed.
    SignerMismatch {
        /// The identity the envelope claimed.
        claimed: String,
        /// The identity the key is actually bound to.
        bound_to: String,
    },
    /// The signature does not verify over the preimage.
    InvalidSignature,
    /// The payload is semantically invalid (empty payload or
    /// scope, or the declared digest does not match the bytes).
    SemanticInvalid,
    /// The bound signer is not authorized for the scope.
    NotAuthorized {
        /// The scope that was refused.
        scope: String,
    },
    /// The envelope is older than the freshness window allows.
    Stale,
    /// The envelope is dated beyond the allowed future skew.
    FutureSkew,
    /// This event id was already admitted: a replay.
    Replay,
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => write!(f, "envelope is unsigned"),
            Self::UnknownSigner => write!(f, "verifying key is bound to no known signer"),
            Self::SignerMismatch { claimed, bound_to } => {
                write!(
                    f,
                    "key is bound to {bound_to}, not claimed signer {claimed}"
                )
            }
            Self::InvalidSignature => write!(f, "signature does not verify"),
            Self::SemanticInvalid => write!(f, "envelope is semantically invalid"),
            Self::NotAuthorized { scope } => {
                write!(f, "signer is not authorized for scope {scope}")
            }
            Self::Stale => write!(f, "envelope is outside the freshness window"),
            Self::FutureSkew => write!(f, "envelope is dated beyond the allowed future skew"),
            Self::Replay => write!(f, "event id was already admitted"),
        }
    }
}

impl std::error::Error for RejectReason {}

/// A successful admission: the evidence the gate records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    /// The verified signer identity.
    pub signer: String,
    /// The scope admitted into.
    pub scope: String,
    /// The admitted event id.
    pub event_id: EventId,
    /// The path the envelope arrived by (observability only).
    pub path: IngressPath,
}

/// Sign an envelope as a local producer. Local production is not
/// an admission shortcut: the result must still pass the gate's
/// admit check - through the same rules as any remote envelope -
/// before it is admitted.
pub fn sign_envelope(
    signing_key: &SigningKey,
    signer: &str,
    scope: &str,
    payload: &[u8],
    event_id: EventId,
    issued_at: LogicalTime,
) -> Envelope {
    let mut envelope = Envelope {
        signer: signer.to_owned(),
        verifying_key: signing_key.verifying_key().to_bytes(),
        signature: None,
        scope: scope.to_owned(),
        payload: payload.to_vec(),
        payload_digest: payload_digest(payload),
        event_id,
        issued_at,
    };
    let signature = signing_key.sign(&envelope.preimage());
    envelope.signature = Some(signature.to_bytes());
    envelope
}

/// The mandatory acceptance gate. One gate instance owns its
/// signer table and its admitted-event ledger; there is no
/// configuration switch that disables or relaxes it.
#[derive(Debug, Clone)]
pub struct IngestGate {
    signers: Vec<SignerAuthorization>,
    freshness: FreshnessWindow,
    admitted: BTreeMap<EventId, PayloadDigest>,
}

impl IngestGate {
    /// A gate with an explicit signer table. An empty table fails
    /// closed: every envelope is `UnknownSigner`.
    pub fn new(signers: Vec<SignerAuthorization>, freshness: FreshnessWindow) -> Self {
        Self {
            signers,
            freshness,
            admitted: BTreeMap::new(),
        }
    }

    /// How many events this gate has admitted.
    pub fn admitted_count(&self) -> usize {
        self.admitted.len()
    }

    /// Whether `event_id` has already been admitted.
    pub fn is_admitted(&self, event_id: &EventId) -> bool {
        self.admitted.contains_key(event_id)
    }

    /// Run the full gate for one envelope. The `path` is recorded
    /// on success and never changes the checks: local, loopback,
    /// internal, hosted, and remote envelopes face identical
    /// signature, binding, semantic, authorization, and replay
    /// validation before admission.
    pub fn admit(
        &mut self,
        path: IngressPath,
        envelope: &Envelope,
        now: LogicalTime,
    ) -> Result<Admission, RejectReason> {
        // 1. Signature presence.
        let Some(signature) = envelope.signature else {
            return Err(RejectReason::Unsigned);
        };
        // 2. Signer binding: the key must be bound, to the claimed
        // identity - resolved before the signature is parsed, so a
        // valid signature from the wrong key is an identity
        // failure, not an admission (B05).
        let bound = self
            .signers
            .iter()
            .find(|signer| signer.verifying_key == envelope.verifying_key)
            .ok_or(RejectReason::UnknownSigner)?;
        if bound.identity != envelope.signer {
            return Err(RejectReason::SignerMismatch {
                claimed: envelope.signer.clone(),
                bound_to: bound.identity.clone(),
            });
        }
        // 3. Signature validity over the canonical preimage.
        let key = VerifyingKey::from_bytes(&envelope.verifying_key)
            .map_err(|_| RejectReason::InvalidSignature)?;
        let sig = Signature::from_bytes(&signature);
        key.verify_strict(&envelope.preimage(), &sig)
            .map_err(|_| RejectReason::InvalidSignature)?;
        // 4. Semantic validation: the payload must be the payload
        // its declared digest names.
        if envelope.payload.is_empty()
            || envelope.scope.is_empty()
            || payload_digest(&envelope.payload) != envelope.payload_digest
        {
            return Err(RejectReason::SemanticInvalid);
        }
        // 5. Authorization: the bound signer must hold the scope.
        if !bound.scopes.contains(&envelope.scope) {
            return Err(RejectReason::NotAuthorized {
                scope: envelope.scope.clone(),
            });
        }
        // 6. Freshness, then replay.
        if envelope.issued_at > now.saturating_add(self.freshness.max_future_skew) {
            return Err(RejectReason::FutureSkew);
        }
        if now > envelope.issued_at.saturating_add(self.freshness.max_age) {
            return Err(RejectReason::Stale);
        }
        if self.admitted.contains_key(&envelope.event_id) {
            return Err(RejectReason::Replay);
        }
        self.admitted
            .insert(envelope.event_id, envelope.payload_digest);
        Ok(Admission {
            signer: envelope.signer.clone(),
            scope: envelope.scope.clone(),
            event_id: envelope.event_id,
            path,
        })
    }
}
