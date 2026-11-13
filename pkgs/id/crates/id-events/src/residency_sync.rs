//! Residency policy gate for native sync (design #3398 §3 native-sync
//! row; issue #5529 step 3).
//!
//! The range-sync pull gate ([`crate::pull_policy`]) answers *who* may
//! pull. Residency answers a different question — *where the bytes may
//! be* — and neither a valid sync identity, a join capability, nor an
//! event signature establishes an allowed location (design §3). This
//! module evaluates the receiving path before event bytes move, on
//! both sides of a session, using the step-1 evaluator
//! ([`id_core::residency::evaluate`]):
//!
//! - **Preflight (send side):** before sending, evaluate the path the
//!   bytes will land on — the peer's receiving store (Storage plane)
//!   and the session transport (Transport plane) — under the LOCAL
//!   scope. A denial means zero protected events are sent.
//! - **Receive side:** before ingesting, evaluate the path the bytes
//!   will occupy here — the local store and the session transport —
//!   under the local scope. A denial means the session is refused
//!   before the range flight, so nothing is ingested.
//! - A session runs only when both evaluations allow. For
//!   [`Scope::Unscoped`] content both always allow, preserving the
//!   pre-residency behaviour exactly.
//!
//! Evidence for the peer's store comes from the operator's deployment
//! inventory (step 2, [`id_core::residency_inventory`]): it is pinned
//! locally, never taken from the peer's own assertions — a peer cannot
//! remove or satisfy its own scope. Expiry is evaluated against the
//! session's start time, so evidence that lapses mid-session does not
//! retroactively taint a completed session, and a later session
//! re-evaluates fresh evidence.
//!
//! The hosted bridge is a session of the same shape — a store, a
//! store, and the hops between them — and is admitted by this same
//! gate: bridge sync for scoped content stays disabled until every
//! store and hop in the session has current evidence. The gate is
//! never a trust authority: it only enforces the outcome of the
//! operator's verifier, and signed-event ingest semantics are
//! unchanged (the receiving store's own ingest still verifies and
//! decides each event).

use std::sync::Arc;

use id_core::residency::{
    Decision, Evidence, EvidenceVerifier, Hop, Plane, ResourceId, Scope, evaluate,
};

/// Residency configuration for one sync relationship: the local scope
/// governing this store's content, the resources a session touches,
/// and the evidence offered for each side.
pub struct SyncResidency {
    /// The local scope. [`Scope::Unscoped`] disables the gate.
    pub scope: Scope,
    /// This node's store resource (Storage plane, receive side).
    pub local_store: ResourceId,
    /// The peer's store resource (Storage plane, send side).
    pub peer_store: ResourceId,
    /// The session transport resource (Transport plane, both sides):
    /// the relay/bridge/direct path the session runs over.
    pub transport: ResourceId,
    /// Evidence for the local store and the transport, from the
    /// local deployment inventory.
    pub local_evidence: Vec<Evidence>,
    /// Evidence for the peer's store and the transport, pinned from
    /// the operator's deployment inventory — never peer-asserted.
    pub peer_evidence: Vec<Evidence>,
    /// The operator's evidence verifier (signature + revocation).
    pub verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
}

impl SyncResidency {
    fn hops<'a>(
        store: &'a ResourceId,
        transport: &'a ResourceId,
        evidence: &'a [Evidence],
    ) -> Vec<Hop<'a>> {
        let find = |plane: Plane, resource: &ResourceId| {
            evidence
                .iter()
                .find(|e| e.plane == plane && &e.resource == resource)
        };
        vec![
            Hop {
                plane: Plane::Storage,
                resource: store.clone(),
                evidence: find(Plane::Storage, store),
            },
            Hop {
                plane: Plane::Transport,
                resource: transport.clone(),
                evidence: find(Plane::Transport, transport),
            },
        ]
    }

    /// Send-side preflight: may event bytes be sent to the peer's
    /// store over this transport, under the local scope? Evaluated
    /// before any event bytes are sent; a denial sends zero protected
    /// events.
    pub fn admit_send(&self, now: u64) -> Decision {
        evaluate(
            &self.scope,
            &Self::hops(&self.peer_store, &self.transport, &self.peer_evidence),
            now,
            &*self.verifier,
        )
    }

    /// Receive-side check: may event bytes arriving over this
    /// transport be ingested into the local store, under the local
    /// scope? Evaluated before the session's range flight; a denial
    /// refuses the session, so nothing is ingested.
    pub fn admit_receive(&self, now: u64) -> Decision {
        evaluate(
            &self.scope,
            &Self::hops(&self.local_store, &self.transport, &self.local_evidence),
            now,
            &*self.verifier,
        )
    }

    /// Session admission: both sides must allow. This is the only
    /// decision [`crate::event_sync::EventSyncProtocol`] consults, and
    /// it takes no membership or capability input — a valid join
    /// capability does not override a residency denial.
    pub fn admit_session(&self, now: u64) -> Decision {
        let send = self.admit_send(now);
        if !send.is_allow() {
            return send;
        }
        self.admit_receive(now)
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use id_core::residency::{Jurisdiction, Policy};

    const NOW: u64 = 1_000_000;

    struct Verifier {
        revoked_serial: Option<String>,
    }

    impl EvidenceVerifier for Verifier {
        fn signature_valid(&self, _evidence: &Evidence) -> bool {
            true
        }

        fn revoked(&self, evidence: &Evidence) -> bool {
            self.revoked_serial.as_deref() == Some(evidence.serial.as_str())
        }
    }

    fn evidence(resource: &str, plane: Plane, expires_at: u64) -> Evidence {
        Evidence {
            issuer: "operator-ca".to_string(),
            resource: ResourceId(resource.to_string()),
            plane,
            jurisdiction: Jurisdiction("eu".to_string()),
            source: "provider-control-plane".to_string(),
            expires_at,
            serial: format!("{resource}-1"),
            signature: vec![1],
        }
    }

    fn scoped() -> Scope {
        let mut policy = Policy::new(["operator-ca".to_string()])
            .allow(Plane::Storage, &["eu"], &["local-store", "peer-store"])
            .allow(Plane::Transport, &["eu"], &["relay-eu"])
            .trust_evidence_source(Plane::Storage, "provider-control-plane")
            .trust_evidence_source(Plane::Transport, "provider-control-plane");
        // A sync path touches the Storage and Transport planes; the
        // policy for it requires exactly those (Policy::new defaults
        // to all four, which would demand evidence this path never
        // produces).
        policy.required = [Plane::Storage, Plane::Transport].into_iter().collect();
        Scope::Scoped(policy)
    }

    fn residency(scope: Scope) -> SyncResidency {
        SyncResidency {
            scope,
            local_store: ResourceId("local-store".to_string()),
            peer_store: ResourceId("peer-store".to_string()),
            transport: ResourceId("relay-eu".to_string()),
            local_evidence: vec![
                evidence("local-store", Plane::Storage, NOW + 3600),
                evidence("relay-eu", Plane::Transport, NOW + 3600),
            ],
            peer_evidence: vec![
                evidence("peer-store", Plane::Storage, NOW + 3600),
                evidence("relay-eu", Plane::Transport, NOW + 3600),
            ],
            verifier: Arc::new(Verifier {
                revoked_serial: None,
            }),
        }
    }

    #[test]
    fn unscoped_content_keeps_existing_behaviour() {
        let mut gate = residency(Scope::Unscoped);
        gate.local_evidence.clear();
        gate.peer_evidence.clear();
        assert!(gate.admit_session(NOW).is_allow());
    }

    #[test]
    fn scoped_session_with_current_evidence_is_admitted() {
        let gate = residency(scoped());
        assert!(gate.admit_send(NOW).is_allow());
        assert!(gate.admit_receive(NOW).is_allow());
        assert!(gate.admit_session(NOW).is_allow());
    }

    #[test]
    fn missing_receiver_evidence_refuses_the_send() {
        // §3 negative row: unknown receiver evidence refuses before
        // event bytes are sent.
        let mut gate = residency(scoped());
        gate.peer_evidence.retain(|e| e.plane != Plane::Storage);
        let decision = gate.admit_send(NOW);
        assert!(!decision.is_allow());
        assert!(!gate.admit_session(NOW).is_allow());
    }

    #[test]
    fn expired_evidence_refuses() {
        // §3 negative row: expired evidence refuses.
        let mut gate = residency(scoped());
        for e in &mut gate.peer_evidence {
            e.expires_at = NOW - 1;
        }
        assert!(!gate.admit_send(NOW).is_allow());
    }

    #[test]
    fn revoked_evidence_refuses() {
        // §3 negative row: revoked evidence refuses.
        let mut gate = residency(scoped());
        gate.verifier = Arc::new(Verifier {
            revoked_serial: Some("peer-store-1".to_string()),
        });
        assert!(!gate.admit_send(NOW).is_allow());
    }

    #[test]
    fn out_of_policy_transport_refuses_the_receive() {
        // The local store is evidenced, but the session transport is
        // not a resource the policy permits for the Transport plane.
        let mut gate = residency(scoped());
        gate.transport = ResourceId("relay-us".to_string());
        assert!(!gate.admit_receive(NOW).is_allow());
        assert!(!gate.admit_session(NOW).is_allow());
    }
}
