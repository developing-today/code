//! Core directional peer-class behavior and the unknown-peer default
//! (slice B31, #10186; design: part-b versioned design, wire/data model,
//! statements S1.1–S1.11).
//!
//! Trust classes are directional policy labels, never cryptographic
//! facts. This module is the pure policy matrix: given a peer's
//! configured [`PeerPolicy`], a direction, an operation, and — for
//! inbound acceptance — the evidence produced by the universal
//! signed-and-validated gate, it decides whether the operation is
//! allowed. It performs no I/O and no cryptography itself; the gate
//! (slice B32) supplies the evidence, this matrix supplies the policy.
//!
//! The rules, exactly as settled in the design:
//!
//! - `fully_trusted` is configured bidirectional policy, never a
//!   bypass: it permits only the operations explicitly configured for
//!   the peer, and inbound acceptance still requires the full gate
//!   (valid signature, authorized signer, semantic validation, scope
//!   authorization) to pass.
//! - `send_only` permits local outbound sends and rejects
//!   peer-originated accepted data, whatever the gate evidence says.
//! - `accept_signed_from` permits configured inbound objects only
//!   after the full gate passes; it grants no outbound send by itself.
//! - An unknown peer — no policy at all, an explicit
//!   [`PeerClass::Unknown`] policy, or a policy outside its validity
//!   window — defaults to no acceptance and no send.
//! - No class grants query relay, completeness authority, or
//!   cross-tenant access by itself: relay requires a separate,
//!   explicit relay grant recorded on the policy, and the other two
//!   are never granted by a peer class.
//!
//! Class changes are versioned per peer ([`PeerPolicy::policy_version`]);
//! this matrix evaluates one policy snapshot at one instant, so a
//! demotion or revocation takes effect as soon as the caller evaluates
//! against the new snapshot.

use std::collections::BTreeSet;

/// A peer's trust class: a directional policy label, never a
/// cryptographic fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PeerClass {
    /// Broad configured operations in both directions, subject to the
    /// configured operation set and — inbound — the full gate. Never
    /// a validation bypass.
    FullyTrusted,
    /// Local outbound sends only; peer-originated data is never
    /// accepted.
    SendOnly,
    /// Inbound acceptance only, and only for objects that pass the
    /// full signed-and-validated gate.
    AcceptSignedFrom,
    /// No configured trust. The restrictive default for any peer the
    /// local node has no policy for.
    Unknown,
}

/// The direction an operation moves data, relative to the local node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// Peer-originated data arriving at the local node (acceptance).
    Inbound,
    /// Locally originated data sent to the peer.
    Outbound,
}

/// An operation the peer-class matrix can decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operation {
    /// Accept a peer-originated signed object into local accepted
    /// data. Inbound only.
    AcceptData,
    /// Send a locally originated object to the peer. Outbound only.
    SendData,
    /// Relay a third party's query through / to this peer. Never
    /// granted by a class alone; needs a separate relay grant.
    QueryRelay,
    /// Treat the peer as authoritative for completeness claims. Never
    /// granted by a peer class.
    CompletenessAuthority,
    /// Access across a tenant boundary via this peer. Never granted
    /// by a peer class.
    CrossTenantAccess,
}

impl Operation {
    /// The only direction in which this operation is meaningful.
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::AcceptData => Direction::Inbound,
            Self::SendData | Self::QueryRelay => Direction::Outbound,
            Self::CompletenessAuthority | Self::CrossTenantAccess => Direction::Inbound,
        }
    }
}

/// Evidence from the universal signed-and-validated gate for one
/// inbound object. Every field must hold before any class may accept
/// the object; `fully_trusted` does not waive any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GateEvidence {
    /// The object's signature verified under the claimed signer's key.
    pub signature_valid: bool,
    /// The signer is the peer (or an identity the peer is bound to)
    /// and is authorized for this kind of object.
    pub signer_authorized: bool,
    /// The object passed semantic validation for its type.
    pub semantically_valid: bool,
    /// The object is inside the scope the peer is configured for.
    pub scope_authorized: bool,
}

impl GateEvidence {
    /// The all-pass evidence: every gate check held.
    #[must_use]
    pub const fn all_pass() -> Self {
        Self {
            signature_valid: true,
            signer_authorized: true,
            semantically_valid: true,
            scope_authorized: true,
        }
    }

    /// Whether every gate check held.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.signature_valid
            && self.signer_authorized
            && self.semantically_valid
            && self.scope_authorized
    }
}

/// One peer's configured policy snapshot (design `PeerPolicy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerPolicy {
    /// The peer this policy governs (an opaque local identifier).
    pub peer_id: String,
    /// The peer's trust class.
    pub class: PeerClass,
    /// The operations explicitly configured for this peer. A class
    /// permits at most these; an operation absent from this set is
    /// denied even for `fully_trusted`.
    pub operations: BTreeSet<Operation>,
    /// Separate, explicit relay grant. No class sets this by itself;
    /// it is recorded only when relay was granted independently of
    /// the peer's class.
    pub relay_granted: bool,
    /// The scope this policy applies to (an opaque local scope id).
    pub scope: String,
    /// Monotonic per-peer policy version; a class change or demotion
    /// is a new version, never an in-place edit of a snapshot.
    pub policy_version: u64,
    /// First instant (unix seconds) the policy is in force, if bounded.
    pub valid_from: Option<u64>,
    /// Instant (unix seconds) at which the policy stops being in
    /// force, if bounded. At or after it the peer is unknown.
    pub expires_at: Option<u64>,
}

impl PeerPolicy {
    /// Whether the policy is in force at `now` (unix seconds).
    #[must_use]
    pub const fn in_force_at(&self, now: u64) -> bool {
        if let Some(from) = self.valid_from
            && now < from
        {
            return false;
        }
        if let Some(until) = self.expires_at
            && now >= until
        {
            return false;
        }
        true
    }
}

/// The matrix's verdict for one (peer, operation, evidence) question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decision {
    /// The operation may proceed.
    Allow,
    /// The operation must not proceed.
    Deny(DenyReason),
}

/// Why the matrix denied an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyReason {
    /// No policy for the peer, an explicit unknown class, or a policy
    /// outside its validity window: the restrictive default.
    UnknownPeer,
    /// The operation is not in the peer's configured operation set.
    NotConfigured,
    /// The peer's class never permits this operation in this
    /// direction (e.g. `send_only` inbound, `accept_signed_from`
    /// outbound).
    ClassForbids,
    /// Inbound acceptance was asked for without the full gate
    /// passing. Applies to every class, `fully_trusted` included.
    GateNotPassed,
    /// The operation is one no peer class grants by itself (relay
    /// without a separate grant, completeness authority,
    /// cross-tenant access).
    NotGrantedByClass,
    /// The operation was evaluated in a direction it does not have.
    WrongDirection,
}

/// Decide one operation for one peer.
///
/// `policy` is the peer's current policy snapshot, or `None` for a
/// peer the local node has no policy for (unknown). `gate` is the
/// gate evidence for the object in question; it is consulted only
/// for inbound acceptance and ignored for outbound sends, which are
/// locally originated and signed before admission by the local gate
/// on the producing side. `now` is unix seconds, used only for the
/// policy validity window.
#[must_use]
pub fn decide(
    policy: Option<&PeerPolicy>,
    operation: Operation,
    direction: Direction,
    gate: &GateEvidence,
    now: u64,
) -> Decision {
    if direction != operation.direction() {
        return Decision::Deny(DenyReason::WrongDirection);
    }
    let Some(policy) = policy.filter(|p| p.in_force_at(now)) else {
        return Decision::Deny(DenyReason::UnknownPeer);
    };
    if policy.class == PeerClass::Unknown {
        return Decision::Deny(DenyReason::UnknownPeer);
    }
    match operation {
        Operation::AcceptData => decide_accept(policy, gate),
        Operation::SendData => decide_send(policy),
        Operation::QueryRelay => {
            if policy.relay_granted {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotGrantedByClass)
            }
        }
        Operation::CompletenessAuthority | Operation::CrossTenantAccess => {
            Decision::Deny(DenyReason::NotGrantedByClass)
        }
    }
}

/// Inbound acceptance: class first, then configuration, then the
/// gate — the gate is never waived, for any class.
fn decide_accept(policy: &PeerPolicy, gate: &GateEvidence) -> Decision {
    match policy.class {
        PeerClass::SendOnly | PeerClass::Unknown => Decision::Deny(DenyReason::ClassForbids),
        PeerClass::FullyTrusted | PeerClass::AcceptSignedFrom => {
            if !policy.operations.contains(&Operation::AcceptData) {
                Decision::Deny(DenyReason::NotConfigured)
            } else if !gate.passed() {
                Decision::Deny(DenyReason::GateNotPassed)
            } else {
                Decision::Allow
            }
        }
    }
}

/// Outbound send: `accept_signed_from` grants no send; the other
/// non-unknown classes send only when send is configured.
fn decide_send(policy: &PeerPolicy) -> Decision {
    match policy.class {
        PeerClass::AcceptSignedFrom | PeerClass::Unknown => {
            Decision::Deny(DenyReason::ClassForbids)
        }
        PeerClass::FullyTrusted | PeerClass::SendOnly => {
            if policy.operations.contains(&Operation::SendData) {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotConfigured)
            }
        }
    }
}
