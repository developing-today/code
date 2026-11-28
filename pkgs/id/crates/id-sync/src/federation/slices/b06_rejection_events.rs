//! B06: observable, recordable rejection events at the universal
//! signed-ingest gate (#10247, statements S1.19 and S1.25).
//!
//! The B32 gate refuses an envelope fail-closed and changes no state.
//! That is safe but silent: an operator cannot see what was refused or
//! why. This slice wraps the gate so that every refusal also produces
//! one typed [`RejectionEvent`], appended to a local, hash-chained
//! [`RejectionLog`]:
//!
//! - **Observable.** Each event names the ingress path, the B32
//!   [`RejectReason`] and its stable class, the claimed signer and key,
//!   the scope, the event id, the declared payload digest and the digest
//!   of the bytes actually presented. Per-class counters summarise the
//!   log.
//! - **Recordable.** Each event has a canonical, domain-separated byte
//!   form and is chained to the previous event's digest, so a later edit,
//!   drop or reorder of the local record is detectable
//!   ([`RejectionLog::verify_chain`]).
//! - **Admission-time context.** Each event records the key status the
//!   gate observed for the presented key (bound to whom, or unbound) and
//!   a digest of the gate policy in force (signer table and freshness
//!   window). A signed wall-clock time is never treated as proof of
//!   historical validity; the event time is the gate's own logical time.
//! - **Not an accusation.** Events are [`Visibility::LocalOnly`]. Export
//!   to anything else is an optional hook ([`RejectionExporter`]) that is
//!   off unless installed, and disabling it is the rollback: rejection
//!   stays fail-closed and locally observable either way.
//! - **Outcome-neutral.** Observation never changes the gate's answer.
//!   The rejected payload bytes are not stored, only digests, and a
//!   rejected envelope never reaches the admitted set.
//!
//! The federation roots record this slice path without declaring it
//! (slice 9110-02); the integration test includes this file and B32's
//! directly, and this module reaches B32 as `super::b32_ingest_gate`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sha2::{Digest, Sha256};

use super::b32_ingest_gate::{
    Admission, ED25519_PUBLIC_KEY_LEN, Envelope, EventId, FreshnessWindow, IngestGate, IngressPath,
    LogicalTime, PayloadDigest, RejectReason, SignerAuthorization, payload_digest,
};

/// Digest of a rejection record or of the gate policy (SHA-256).
pub type RecordDigest = [u8; 32];

/// The chain value before the first event.
pub const GENESIS: RecordDigest = [0; 32];

const RECORD_DOMAIN: &[u8] = b"id-sync/b06-rejection-event/v1";
const POLICY_DOMAIN: &[u8] = b"id-sync/b06-gate-policy/v1";

/// Who may see a rejection event. Rejection is auditable locally; it
/// never becomes a public accusation by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    /// Kept in this node's local record only.
    LocalOnly,
}

/// What the gate knew about the presented key when it refused.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyStatus {
    /// The key is bound to this identity in the gate's signer table.
    BoundTo(String),
    /// The key is bound to no identity the gate knows.
    Unbound,
}

/// The stable class name of a B32 rejection, for counters and export.
pub const fn reason_class(reason: &RejectReason) -> &'static str {
    match reason {
        RejectReason::Unsigned => "unsigned",
        RejectReason::UnknownSigner => "unknown_signer",
        RejectReason::SignerMismatch { .. } => "signer_mismatch",
        RejectReason::InvalidSignature => "invalid_signature",
        RejectReason::SemanticInvalid => "semantic_invalid",
        RejectReason::NotAuthorized { .. } => "not_authorized",
        RejectReason::Stale => "stale",
        RejectReason::FutureSkew => "future_skew",
        RejectReason::Replay => "replay",
    }
}

/// Every class [`reason_class`] can return, in gate-check order.
pub const REASON_CLASSES: [&str; 9] = [
    "unsigned",
    "unknown_signer",
    "signer_mismatch",
    "invalid_signature",
    "semantic_invalid",
    "not_authorized",
    "stale",
    "future_skew",
    "replay",
];

/// One refused envelope, as the gate observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectionEvent {
    /// Position in the local log, from 0.
    pub sequence: u64,
    /// The gate's logical time when it refused.
    pub observed_at: LogicalTime,
    /// The path the envelope arrived by.
    pub path: IngressPath,
    /// Why the gate refused.
    pub reason: RejectReason,
    /// The signer identity the envelope claimed.
    pub claimed_signer: String,
    /// The verifying key the envelope presented.
    pub verifying_key: [u8; ED25519_PUBLIC_KEY_LEN],
    /// What the gate's signer table said about that key.
    pub key_status: KeyStatus,
    /// Whether a signature was presented at all.
    pub signed: bool,
    /// The scope the envelope asked for.
    pub scope: String,
    /// The envelope's event id.
    pub event_id: EventId,
    /// The `issued_at` the envelope claimed (unverified when the
    /// signature did not verify).
    pub claimed_issued_at: LogicalTime,
    /// The payload digest the envelope declared.
    pub declared_digest: PayloadDigest,
    /// The digest of the payload bytes actually presented. The bytes
    /// themselves are not kept.
    pub presented_digest: PayloadDigest,
    /// Digest of the gate policy in force.
    pub policy_digest: RecordDigest,
    /// Digest of the previous event's record, or [`GENESIS`].
    pub prev: RecordDigest,
    /// Who may see this event.
    pub visibility: Visibility,
}

impl RejectionEvent {
    /// The stable class of [`RejectionEvent::reason`].
    pub const fn class(&self) -> &'static str {
        reason_class(&self.reason)
    }

    /// The canonical record bytes: domain-separated, every field
    /// length-prefixed, the chain link included.
    pub fn record_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(RECORD_DOMAIN);
        push(&mut out, &self.sequence.to_be_bytes());
        push(&mut out, &self.observed_at.to_be_bytes());
        push(&mut out, path_token(self.path).as_bytes());
        push(&mut out, self.class().as_bytes());
        match &self.reason {
            RejectReason::SignerMismatch { claimed, bound_to } => {
                push(&mut out, claimed.as_bytes());
                push(&mut out, bound_to.as_bytes());
            }
            RejectReason::NotAuthorized { scope } => push(&mut out, scope.as_bytes()),
            _ => {}
        }
        push(&mut out, self.claimed_signer.as_bytes());
        push(&mut out, &self.verifying_key);
        match &self.key_status {
            KeyStatus::BoundTo(identity) => {
                push(&mut out, b"bound");
                push(&mut out, identity.as_bytes());
            }
            KeyStatus::Unbound => push(&mut out, b"unbound"),
        }
        push(&mut out, &[u8::from(self.signed)]);
        push(&mut out, self.scope.as_bytes());
        push(&mut out, &self.event_id);
        push(&mut out, &self.claimed_issued_at.to_be_bytes());
        push(&mut out, &self.declared_digest);
        push(&mut out, &self.presented_digest);
        push(&mut out, &self.policy_digest);
        push(&mut out, &self.prev);
        push(
            &mut out,
            match self.visibility {
                Visibility::LocalOnly => b"local-only",
            },
        );
        out
    }

    /// SHA-256 of [`RejectionEvent::record_bytes`]; the next event's
    /// `prev`.
    pub fn digest(&self) -> RecordDigest {
        sha256(&self.record_bytes())
    }
}

/// Where in the chain a local record stops verifying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    /// The event at this position has the wrong sequence number.
    Sequence(usize),
    /// The event at this position does not link to its predecessor.
    Link(usize),
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sequence(at) => write!(f, "rejection log sequence breaks at {at}"),
            Self::Link(at) => write!(f, "rejection log chain breaks at {at}"),
        }
    }
}

impl std::error::Error for ChainError {}

/// The local, append-only record of rejection events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RejectionLog {
    events: Vec<RejectionEvent>,
}

impl RejectionLog {
    /// Every recorded event, oldest first.
    pub fn events(&self) -> &[RejectionEvent] {
        &self.events
    }

    /// How many events are recorded.
    pub const fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing has been refused.
    pub const fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The digest the next event links to.
    pub fn head(&self) -> RecordDigest {
        self.events.last().map_or(GENESIS, RejectionEvent::digest)
    }

    /// Count of events per class; every class is present, zero or not.
    pub fn counts(&self) -> BTreeMap<&'static str, u64> {
        let mut counts: BTreeMap<&'static str, u64> =
            REASON_CLASSES.iter().map(|class| (*class, 0)).collect();
        for event in &self.events {
            *counts.entry(event.class()).or_default() += 1;
        }
        counts
    }

    /// Check sequence numbers and chain links from the first event.
    pub fn verify_chain(&self) -> Result<(), ChainError> {
        verify_chain(&self.events)
    }
}

/// Check that `events` is a gap-free chain starting at [`GENESIS`].
pub fn verify_chain(events: &[RejectionEvent]) -> Result<(), ChainError> {
    let mut prev = GENESIS;
    for (index, event) in events.iter().enumerate() {
        if event.sequence != index as u64 {
            return Err(ChainError::Sequence(index));
        }
        if event.prev != prev {
            return Err(ChainError::Link(index));
        }
        prev = event.digest();
    }
    Ok(())
}

/// An optional sink for rejection events beyond the local record. It
/// sees each event after it is recorded and cannot change the gate's
/// outcome or the local record.
pub trait RejectionExporter {
    /// Receive one recorded event.
    fn export(&mut self, event: &RejectionEvent);
}

/// The B32 gate with every refusal observed and recorded.
pub struct ObservedGate {
    gate: IngestGate,
    bindings: BTreeMap<[u8; ED25519_PUBLIC_KEY_LEN], String>,
    policy_digest: RecordDigest,
    log: RejectionLog,
    exporter: Option<Box<dyn RejectionExporter>>,
}

impl fmt::Debug for ObservedGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservedGate")
            .field("gate", &self.gate)
            .field("policy_digest", &self.policy_digest)
            .field("log", &self.log)
            .field("export_enabled", &self.exporter.is_some())
            .finish_non_exhaustive()
    }
}

impl ObservedGate {
    /// Build the B32 gate from `signers` and `freshness`, observed.
    /// Export is off.
    pub fn new(signers: Vec<SignerAuthorization>, freshness: FreshnessWindow) -> Self {
        let policy_digest = policy_digest(&signers, freshness);
        let bindings = signers
            .iter()
            .map(|signer| (signer.verifying_key, signer.identity.clone()))
            .collect();
        Self {
            gate: IngestGate::new(signers, freshness),
            bindings,
            policy_digest,
            log: RejectionLog::default(),
            exporter: None,
        }
    }

    /// Install an exporter, replacing any other.
    pub fn enable_export(&mut self, exporter: Box<dyn RejectionExporter>) {
        self.exporter = Some(exporter);
    }

    /// Remove the exporter. Rejection stays fail-closed and recorded.
    pub fn disable_export(&mut self) -> Option<Box<dyn RejectionExporter>> {
        self.exporter.take()
    }

    /// Whether an exporter is installed.
    pub const fn export_enabled(&self) -> bool {
        self.exporter.is_some()
    }

    /// The digest of the gate policy every event records.
    pub const fn policy_digest(&self) -> RecordDigest {
        self.policy_digest
    }

    /// The local rejection record.
    pub const fn log(&self) -> &RejectionLog {
        &self.log
    }

    /// The wrapped gate, for its admission queries.
    pub const fn gate(&self) -> &IngestGate {
        &self.gate
    }

    /// Run the B32 gate. The outcome is exactly the gate's; a refusal
    /// is also recorded (and exported, when export is on) before it is
    /// returned.
    pub fn admit(
        &mut self,
        path: IngressPath,
        envelope: &Envelope,
        now: LogicalTime,
    ) -> Result<Admission, RejectReason> {
        let reason = match self.gate.admit(path, envelope, now) {
            Ok(admission) => return Ok(admission),
            Err(reason) => reason,
        };
        let key_status = self
            .bindings
            .get(&envelope.verifying_key)
            .map_or(KeyStatus::Unbound, |identity| {
                KeyStatus::BoundTo(identity.clone())
            });
        let event = RejectionEvent {
            sequence: self.log.events.len() as u64,
            observed_at: now,
            path,
            reason: reason.clone(),
            claimed_signer: envelope.signer.clone(),
            verifying_key: envelope.verifying_key,
            key_status,
            signed: envelope.signature.is_some(),
            scope: envelope.scope.clone(),
            event_id: envelope.event_id,
            claimed_issued_at: envelope.issued_at,
            declared_digest: envelope.payload_digest,
            presented_digest: payload_digest(&envelope.payload),
            policy_digest: self.policy_digest,
            prev: self.log.head(),
            visibility: Visibility::LocalOnly,
        };
        self.log.events.push(event);
        if let (Some(exporter), Some(event)) = (self.exporter.as_mut(), self.log.events.last()) {
            exporter.export(event);
        }
        Err(reason)
    }
}

/// Digest of the gate policy: the signer table, in a canonical order,
/// and the freshness window.
pub fn policy_digest(signers: &[SignerAuthorization], freshness: FreshnessWindow) -> RecordDigest {
    let mut ordered: Vec<&SignerAuthorization> = signers.iter().collect();
    ordered.sort_by(|a, b| {
        (&a.identity, &a.verifying_key, &a.scopes).cmp(&(&b.identity, &b.verifying_key, &b.scopes))
    });
    let mut out = Vec::new();
    out.extend_from_slice(POLICY_DOMAIN);
    push(&mut out, &(ordered.len() as u64).to_be_bytes());
    for signer in ordered {
        push(&mut out, signer.identity.as_bytes());
        push(&mut out, &signer.verifying_key);
        let scopes: &BTreeSet<String> = &signer.scopes;
        push(&mut out, &(scopes.len() as u64).to_be_bytes());
        for scope in scopes {
            push(&mut out, scope.as_bytes());
        }
    }
    push(&mut out, &freshness.max_age.to_be_bytes());
    push(&mut out, &freshness.max_future_skew.to_be_bytes());
    sha256(&out)
}

const fn path_token(path: IngressPath) -> &'static str {
    match path {
        IngressPath::Local => "local",
        IngressPath::Loopback => "loopback",
        IngressPath::Internal => "internal",
        IngressPath::Hosted => "hosted",
        IngressPath::Remote => "remote",
    }
}

fn push(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}
