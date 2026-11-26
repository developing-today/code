//! Accept-side NodeId allowlist and per-ALPN authorization — the
//! authorization module of `fleet-transport` (re-homed from the
//! standalone-crate slice of PR #15171: main landed its routing core
//! at this crate path first, so the allowlist lives here as a module).
//!
//! Design of record: `doc/2026-11-23T21-10-36Z_design_fleet-transport-pooling`
//! (§2.2, tracking issue #13752), work item 2c (#14720).
//!
//! iroh authenticates the peer in the QUIC/TLS handshake: an accepting
//! endpoint learns the peer's authenticated **NodeId** from the
//! handshake itself. Authentication is therefore intrinsic to the
//! transport — and, in the design's words, **authorization is the
//! application's**. This module is that application layer, written
//! before the 2a pilot endpoint (#14718) lands so the endpoint wires
//! into a tested decision instead of inventing one at the accept loop:
//!
//! - **The allowlist is derived from registry enrollment.** The inputs
//!   are enrollment bindings — a NodeId bound to an agent class and to
//!   the protocols (ALPNs) that class may speak — mirroring the machine
//!   registry's `enrollment` map (agent class → offers, #14714) with
//!   the NodeId binding the design requires (§2.4: a NodeId bound to a
//!   fleet identity in the registry, never one living only in a seat's
//!   local config). The caller parses the registry; this crate is
//!   std-only strings and tables, no I/O, like `fleet-registry`.
//! - **Enforcement is per ALPN.** Being enrolled buys a NodeId exactly
//!   the protocols its binding lists — no more. An authenticated
//!   NodeId speaking an ALPN it is not enrolled for is refused, the
//!   same as an unknown one.
//! - **Revocation is a binding state, not a deletion.** A revoked
//!   binding stays in the table and refuses as `Revoked`, so a refusal
//!   can say *this identity was enrolled and was revoked* instead of
//!   collapsing it into *unknown* — the two need different responses
//!   from an operator reading the log.
//! - **Every refusal is logged with NodeId and ALPN.** [`Allowlist`]
//!   records each refusal in its refusal log; the accepting endpoint
//!   drains it and emits the lines. A refusal that leaves no record
//!   is a bug, on the same doctrine as `fleet-pool`'s decision log.
//!
//! What this crate is not: the endpoint itself (2a, #14718), relay
//! policy (2b), or a NodeId parser — the registry stores `node_id` as
//! an opaque scalar, so a NodeId here is the registry's string,
//! validated only for non-emptiness.

use std::collections::BTreeMap;
use std::fmt;

/// An iroh NodeId as the registry stores it: the opaque string form of
/// the peer's ed25519 public key (`endpoint.node_id` on an iroh offer).
pub type NodeId = String;

/// A protocol identifier as negotiated in the handshake (an ALPN
/// string, e.g. `fleet/laptop-agent/1`).
pub type Alpn = String;

/// Whether an enrollment binding is live or has been revoked.
/// Revoked bindings are kept in the table on purpose (see crate docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    Active,
    Revoked,
}

/// One enrollment binding: a NodeId, the agent class it is enrolled
/// as, the ALPNs that class may speak on the accepting endpoint, and
/// whether the binding is live.
///
/// This is the accept-side projection of the registry's `enrollment`
/// map (#14714) plus the NodeId ↔ identity binding of design §2.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrollment {
    pub node_id: NodeId,
    pub agent_class: String,
    pub alpns: Vec<Alpn>,
    pub state: BindingState,
}

/// Why an allowlist could not be built from the given enrollments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// A NodeId with an empty string form can never be matched
    /// against a handshake peer; accepting it would be dead weight
    /// that reads as coverage.
    EmptyNodeId,
    /// A binding that lists no ALPN enrolls the NodeId for nothing;
    /// the registry validator rejects the same shape ("must list at
    /// least one offer name").
    NoAlpns { node_id: NodeId },
    /// Two bindings for one NodeId: which agent class and ALPN set
    /// wins must be a registry decision, not a merge-order accident.
    DuplicateNodeId { node_id: NodeId },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::EmptyNodeId => write!(f, "enrollment with an empty node_id"),
            BuildError::NoAlpns { node_id } => {
                write!(f, "enrollment for {node_id} lists no ALPNs")
            }
            BuildError::DuplicateNodeId { node_id } => {
                write!(f, "duplicate enrollment for node_id {node_id}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// Why a connection was refused. Recorded in the refusal log with the
/// NodeId and ALPN, and returned to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    /// No enrollment binding exists for this NodeId at all.
    UnknownNodeId,
    /// A binding exists but has been revoked.
    RevokedNodeId,
    /// The NodeId is enrolled and active, but not for this ALPN.
    AlpnNotEnrolled,
}

impl fmt::Display for RefusalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RefusalReason::UnknownNodeId => "unknown-node-id",
            RefusalReason::RevokedNodeId => "revoked-node-id",
            RefusalReason::AlpnNotEnrolled => "alpn-not-enrolled",
        };
        write!(f, "{s}")
    }
}

/// The accept decision for one (NodeId, ALPN) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The peer may connect for this ALPN, enrolled as this agent
    /// class (the endpoint may need the class for its own routing).
    Allowed {
        agent_class: String,
    },
    Refused {
        reason: RefusalReason,
    },
}

/// One logged refusal: who was turned away, for which protocol, and
/// why. The accepting endpoint emits these; tests read them back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub node_id: NodeId,
    pub alpn: Alpn,
    pub reason: RefusalReason,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "refused node_id={} alpn={} reason={}",
            self.node_id, self.alpn, self.reason
        )
    }
}

/// The accept-side allowlist: enrollment bindings by NodeId, plus the
/// log of every refusal this instance has decided.
#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    bindings: BTreeMap<NodeId, Enrollment>,
    refusals: Vec<Refusal>,
}

impl Allowlist {
    /// Derive the allowlist from registry enrollment bindings.
    ///
    /// Fails on the shapes the registry itself would reject —
    /// duplicate bindings for one NodeId and bindings that enroll
    /// for nothing — rather than letting them silently narrow or
    /// widen what the endpoint accepts.
    pub fn from_enrollments(
        enrollments: impl IntoIterator<Item = Enrollment>,
    ) -> Result<Self, BuildError> {
        let mut bindings: BTreeMap<NodeId, Enrollment> = BTreeMap::new();
        for e in enrollments {
            if e.node_id.is_empty() {
                return Err(BuildError::EmptyNodeId);
            }
            if e.alpns.is_empty() {
                return Err(BuildError::NoAlpns {
                    node_id: e.node_id.clone(),
                });
            }
            if bindings.insert(e.node_id.clone(), e.clone()).is_some() {
                return Err(BuildError::DuplicateNodeId {
                    node_id: e.node_id.clone(),
                });
            }
        }
        Ok(Allowlist {
            bindings,
            refusals: Vec::new(),
        })
    }

    /// Decide one accepted-handshake peer. Authentication has already
    /// happened in the handshake; this is authorization only, and it
    /// never treats an authenticated NodeId as an authorized one.
    ///
    /// Every refusal is appended to the refusal log before returning.
    pub fn authorize(&mut self, node_id: &str, alpn: &str) -> Decision {
        let decision = match self.bindings.get(node_id) {
            None => Decision::Refused {
                reason: RefusalReason::UnknownNodeId,
            },
            Some(e) if e.state == BindingState::Revoked => Decision::Refused {
                reason: RefusalReason::RevokedNodeId,
            },
            Some(e) if !e.alpns.iter().any(|a| a == alpn) => Decision::Refused {
                reason: RefusalReason::AlpnNotEnrolled,
            },
            Some(e) => Decision::Allowed {
                agent_class: e.agent_class.clone(),
            },
        };
        if let Decision::Refused { reason } = decision {
            self.refusals.push(Refusal {
                node_id: node_id.to_string(),
                alpn: alpn.to_string(),
                reason,
            });
        }
        decision
    }

    /// Every refusal decided so far, in decision order. The endpoint
    /// drains this (via [`Allowlist::drain_refusals`]) into its log;
    /// reading it here is how tests prove the logging criterion.
    pub fn refusals(&self) -> &[Refusal] {
        &self.refusals
    }

    /// Take the refusal log, leaving it empty for the next batch.
    pub fn drain_refusals(&mut self) -> Vec<Refusal> {
        std::mem::take(&mut self.refusals)
    }

    /// Number of enrolled NodeIds (active and revoked).
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}
