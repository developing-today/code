//! Residency route policy for federation, relays and metadata egress
//! (design #3398 §3 federation and relay rows, §6 metadata gate; issue
//! #5529 step 4).
//!
//! Step 3 ([`crate::residency_sync`]) gated the native-sync and bridge
//! paths. This module gates the remaining route classes the design
//! names, using the same step-1 evaluator
//! ([`id_core::residency::evaluate`]) and the same rule: evidence is
//! pinned from the operator's local deployment inventory, never taken
//! from a peer's own assertions, and no capability, ticket or
//! signature is an input to any decision here — a valid join
//! capability does not override a residency denial (design §3).
//!
//! - **Federation** ([`FederationGate`]): jurisdiction partitioning
//!   is the default. Replication of an explicitly selected projection
//!   to a destination store is admitted only when the destination's
//!   storage evidence evaluates inside the local partition. A
//!   cross-boundary shared world requires an explicit world-level
//!   [`CrossBoundaryConsent`] — participant consent *and* disclosure
//!   recorded — naming the destination jurisdiction and resource;
//!   anything else is denied and creates no replica.
//! - **Relays** ([`RelayGate`]): an Iroh relay is transport, never
//!   residency evidence. A scoped route may use only a relay whose
//!   transport evidence evaluates as approved and attested under the
//!   local policy; unknown or unapproved relays are never selected.
//!   Direct mode removes the relay hop, but it cannot certify the
//!   underlying Internet route: when the policy requires a proven
//!   managed route, direct is denied rather than treated as a
//!   locality guarantee (design §3 relay row, §6).
//! - **Metadata egress** ([`MetadataGate`]): discovery, telemetry and
//!   log destinations are explicit. A scoped deployment declares its
//!   metadata destinations in the policy's Metadata plane; egress to
//!   anything else is denied, so telemetry cannot reintroduce
//!   identifiers that payload policy blocked (design §2 metadata
//!   plane, §6).
//!
//! For [`Scope::Unscoped`] content every gate allows, preserving
//! pre-residency behaviour exactly.

use std::collections::BTreeSet;
use std::sync::Arc;

use id_core::residency::{
    Decision, DenyReason, Evidence, EvidenceVerifier, Hop, Jurisdiction, Plane, Policy, ResourceId,
    Scope, evaluate,
};

/// Why a route was denied. [`RouteDeny::Residency`] carries the
/// step-1 evaluator's own reason codes; the other variants are the
/// route-level failures the evaluator cannot express.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteDeny {
    /// The evaluator denied the underlying path (unknown, expired,
    /// revoked, mismatched or out-of-policy evidence).
    Residency(Vec<DenyReason>),
    /// Federation was asked to replicate an empty projection: only
    /// an explicitly selected projection/object set may replicate
    /// (design §3 federation row), and an empty selection is a
    /// caller error, not an allow.
    EmptyProjection,
    /// The destination lies outside the local partition and no
    /// world-level cross-boundary consent names it.
    CrossBoundaryWithoutConsent,
    /// Cross-boundary consent exists but participant consent or
    /// disclosure is not recorded; both are required (design §3
    /// federation row, §6 disclosure gate).
    CrossBoundaryNotDisclosed,
    /// No candidate relay exists at all.
    NoRelayAvailable,
    /// Candidates exist but none is an approved, attested relay
    /// under the local policy.
    NoApprovedRelay,
    /// The transport policy requires a proven managed route and a
    /// direct route cannot certify the underlying Internet route.
    DirectRouteInsufficient,
    /// The metadata destination is not one of the destinations the
    /// deployment explicitly declared for the Metadata plane.
    MetadataDestinationNotDeclared,
}

/// A route-level decision: the route may proceed, or it is denied
/// before any protected bytes or identifiers move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteDecision {
    /// The route is admitted.
    Allow,
    /// The route is denied; nothing is sent, stored or emitted.
    Deny(RouteDeny),
}

impl RouteDecision {
    /// Whether the route is admitted.
    pub fn is_allow(&self) -> bool {
        matches!(self, RouteDecision::Allow)
    }
}

impl From<Decision> for RouteDecision {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Allow => RouteDecision::Allow,
            Decision::Deny(reasons) => RouteDecision::Deny(RouteDeny::Residency(reasons)),
        }
    }
}

/// Narrow a scope's policy to the planes one route class evaluates,
/// so a policy built the documented way (`Policy::new` leaves
/// `required` at all four planes) is not denied with `PlaneUnknown`
/// for planes the route never touches. Mirrors
/// [`crate::residency_sync`]'s path narrowing.
fn scope_for_planes(scope: &Scope, planes: &[Plane]) -> Scope {
    let Scope::Scoped(policy) = scope else {
        return Scope::Unscoped;
    };
    let mut policy: Policy = policy.clone();
    policy.required.retain(|p| planes.contains(p));
    Scope::Scoped(policy)
}

/// An explicit world-level policy for one cross-boundary shared
/// world (design §3 federation row): the destination jurisdictions
/// and resources it names, with participant consent and disclosure
/// recorded. It is a local operator/world record — a read ticket or
/// join capability is not one of these and cannot substitute for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossBoundaryConsent {
    /// Destination jurisdictions this world may span.
    pub jurisdictions: BTreeSet<Jurisdiction>,
    /// Destination resources this world may replicate to.
    pub resources: BTreeSet<ResourceId>,
    /// Participants consented to the cross-boundary sharing.
    pub participant_consent: bool,
    /// The cross-boundary nature of the world was disclosed to its
    /// participants.
    pub disclosed: bool,
}

/// Residency gate for federation: replication of a selected
/// projection from this world to one destination store.
pub struct FederationGate {
    /// The local scope. [`Scope::Unscoped`] disables the gate.
    pub scope: Scope,
    /// The destination store the projection would replicate to
    /// (Storage plane).
    pub destination_store: ResourceId,
    /// Evidence for the destination store, pinned from the local
    /// operator's deployment inventory — never peer-asserted.
    pub destination_evidence: Vec<Evidence>,
    /// The world-level cross-boundary policy, if one exists.
    pub cross_boundary: Option<CrossBoundaryConsent>,
    /// The operator's evidence verifier (signature + revocation).
    pub verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
}

impl FederationGate {
    /// May the explicitly selected projection `projected` replicate
    /// to the destination store? Evaluated before any replica is
    /// created; a denial creates no replica. Takes no capability or
    /// ticket input — a valid capability plus a destination outside
    /// the allowed set is denied (design §3 federation negative
    /// test).
    pub fn admit_replication(&self, projected: &BTreeSet<ResourceId>, now: u64) -> RouteDecision {
        let Scope::Scoped(policy) = &self.scope else {
            return RouteDecision::Allow;
        };
        if projected.is_empty() {
            return RouteDecision::Deny(RouteDeny::EmptyProjection);
        }
        let evidence = self
            .destination_evidence
            .iter()
            .find(|e| e.plane == Plane::Storage && e.resource == self.destination_store);

        // Partitioning is the default: the destination must sit in
        // the local policy's Storage jurisdictions. Anything outside
        // needs the explicit cross-boundary path below.
        let mut effective: Policy = policy.clone();
        if let Some(ev) = evidence {
            let partition: BTreeSet<Jurisdiction> = policy
                .planes
                .get(&Plane::Storage)
                .map(|p| p.jurisdictions.clone())
                .unwrap_or_default();
            if !partition.contains(&ev.jurisdiction) {
                let Some(consent) = &self.cross_boundary else {
                    return RouteDecision::Deny(RouteDeny::CrossBoundaryWithoutConsent);
                };
                if !consent.participant_consent || !consent.disclosed {
                    return RouteDecision::Deny(RouteDeny::CrossBoundaryNotDisclosed);
                }
                if !consent.jurisdictions.contains(&ev.jurisdiction)
                    || !consent.resources.contains(&self.destination_store)
                {
                    return RouteDecision::Deny(RouteDeny::CrossBoundaryWithoutConsent);
                }
                // The consent names this destination: admit it into
                // the effective policy for this evaluation only. The
                // stored policy (the partition default) is unchanged.
                let entry = effective.planes.entry(Plane::Storage).or_default();
                entry
                    .jurisdictions
                    .extend(consent.jurisdictions.iter().cloned());
                entry.resources.extend(consent.resources.iter().cloned());
            }
        }

        let hops = vec![Hop {
            plane: Plane::Storage,
            resource: self.destination_store.clone(),
            evidence,
        }];
        evaluate(
            &scope_for_planes(&Scope::Scoped(effective), &[Plane::Storage]),
            &hops,
            now,
            &*self.verifier,
        )
        .into()
    }
}

/// How much route proof the transport policy demands (design §3
/// relay row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportRequirement {
    /// Any admitted route — an approved relay, or direct (no relay
    /// hop receives the bytes).
    RelayOrDirect,
    /// The route must be a separately managed network route that can
    /// prove its underlay. Neither an unproven relay path nor direct
    /// mode satisfies this; scoped content is denied until such a
    /// route exists.
    ManagedRouteProven,
}

/// Residency gate for relay selection on one scoped route.
pub struct RelayGate {
    /// The local scope. [`Scope::Unscoped`] disables the gate.
    pub scope: Scope,
    /// Candidate relays, in preference order.
    pub candidates: Vec<ResourceId>,
    /// Evidence for the candidates, pinned from the local operator's
    /// deployment inventory — a relay's own assertion is not
    /// evidence, and a relay is never residency evidence for another
    /// plane.
    pub evidence: Vec<Evidence>,
    /// How much route proof this route's policy demands.
    pub requirement: TransportRequirement,
    /// The operator's evidence verifier (signature + revocation).
    pub verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
}

impl RelayGate {
    /// Select the first candidate, in preference order, that is an
    /// approved, attested relay under the local policy: its
    /// Transport evidence must evaluate as allowed. Unknown
    /// (unevidenced) and unapproved (out-of-policy, expired or
    /// revoked) relays are skipped, never selected; when no
    /// candidate qualifies the route has no relay and the caller
    /// must not fall back to an unapproved one.
    pub fn select_relay(&self, now: u64) -> Result<ResourceId, RouteDeny> {
        if self.candidates.is_empty() {
            return Err(RouteDeny::NoRelayAvailable);
        }
        if matches!(self.scope, Scope::Unscoped) {
            return Ok(self.candidates[0].clone());
        }
        let scoped = scope_for_planes(&self.scope, &[Plane::Transport]);
        for candidate in &self.candidates {
            let evidence = self
                .evidence
                .iter()
                .find(|e| e.plane == Plane::Transport && &e.resource == candidate);
            let hops = vec![Hop {
                plane: Plane::Transport,
                resource: candidate.clone(),
                evidence,
            }];
            if evaluate(&scoped, &hops, now, &*self.verifier).is_allow() {
                return Ok(candidate.clone());
            }
        }
        Err(RouteDeny::NoApprovedRelay)
    }

    /// May this route run direct (no relay)? Direct removes the
    /// known relay hop, but it cannot certify the underlying
    /// Internet route, so under
    /// [`TransportRequirement::ManagedRouteProven`] direct alone
    /// does not satisfy the transport requirement and is denied
    /// (design §3 relay negative test). Under
    /// [`TransportRequirement::RelayOrDirect`] no relay receives the
    /// bytes, so there is no relay hop to evidence and the route is
    /// admitted; the peer path itself remains the sync gate's
    /// subject ([`crate::residency_sync`]).
    pub fn admit_direct(&self) -> RouteDecision {
        match self.requirement {
            TransportRequirement::ManagedRouteProven => {
                RouteDecision::Deny(RouteDeny::DirectRouteInsufficient)
            }
            TransportRequirement::RelayOrDirect => RouteDecision::Allow,
        }
    }
}

/// What kind of metadata is leaving. The kind does not change the
/// decision: one declared-destination gate covers all metadata, so
/// no kind can become the unguarded path (design §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataKind {
    /// Discovery queries and announcements.
    Discovery,
    /// Metrics and traces.
    Telemetry,
    /// Logs and debug/billing identifiers.
    Logs,
}

/// Residency gate for metadata egress to one destination:
/// discovery, observability and log sinks.
pub struct MetadataGate {
    /// The local scope. [`Scope::Unscoped`] disables the gate.
    pub scope: Scope,
    /// The destination the metadata would be emitted to (Metadata
    /// plane): a discovery service, telemetry sink or log store.
    pub destination: ResourceId,
    /// Evidence for the destination, pinned from the local
    /// operator's deployment inventory.
    pub evidence: Vec<Evidence>,
    /// The operator's evidence verifier (signature + revocation).
    pub verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
}

impl MetadataGate {
    /// May metadata of kind `kind` be emitted to the destination?
    /// A scoped deployment's declared metadata destinations are the
    /// resources named in its policy's Metadata plane; egress to an
    /// undeclared destination is denied before evaluation, and a
    /// declared destination still needs current, in-policy evidence.
    /// A relay that was denied for transport is just an undeclared
    /// destination here unless it was explicitly declared — being a
    /// route hop never makes something a metadata destination.
    pub fn admit_egress(&self, _kind: MetadataKind, now: u64) -> RouteDecision {
        let Scope::Scoped(policy) = &self.scope else {
            return RouteDecision::Allow;
        };
        let declared = policy
            .planes
            .get(&Plane::Metadata)
            .map(|p| p.resources.contains(&self.destination))
            .unwrap_or(false);
        if !declared {
            return RouteDecision::Deny(RouteDeny::MetadataDestinationNotDeclared);
        }
        let evidence = self
            .evidence
            .iter()
            .find(|e| e.plane == Plane::Metadata && e.resource == self.destination);
        let hops = vec![Hop {
            plane: Plane::Metadata,
            resource: self.destination.clone(),
            evidence,
        }];
        evaluate(
            &scope_for_planes(&self.scope, &[Plane::Metadata]),
            &hops,
            now,
            &*self.verifier,
        )
        .into()
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use id_core::residency::Policy;

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

    fn evidence(resource: &str, plane: Plane, jurisdiction: &str, expires_at: u64) -> Evidence {
        Evidence {
            issuer: "operator-ca".to_string(),
            resource: ResourceId(resource.to_string()),
            plane,
            jurisdiction: Jurisdiction(jurisdiction.to_string()),
            source: "provider-control-plane".to_string(),
            expires_at,
            serial: format!("{resource}-1"),
            signature: vec![1],
        }
    }

    /// Local partition: Storage in "eu" at the eu store and a second
    /// in-partition store; Transport via the eu relay; Metadata only
    /// to the declared eu telemetry sink. Built the documented way
    /// (`required` left at all four planes; each gate narrows it).
    fn scoped() -> Scope {
        let policy = Policy::new(["operator-ca".to_string()])
            .allow(
                Plane::Storage,
                &["eu"],
                &["eu-store", "peer-eu-store", "us-store"],
            )
            .allow(Plane::Transport, &["eu"], &["relay-eu"])
            .allow(Plane::Metadata, &["eu"], &["telemetry-eu"])
            .trust_evidence_source(Plane::Storage, "provider-control-plane")
            .trust_evidence_source(Plane::Transport, "provider-control-plane")
            .trust_evidence_source(Plane::Metadata, "provider-control-plane");
        Scope::Scoped(policy)
    }

    fn projection() -> BTreeSet<ResourceId> {
        [ResourceId("world-documents".to_string())]
            .into_iter()
            .collect()
    }

    fn federation(scope: Scope, store: &str, jurisdiction: &str) -> FederationGate {
        FederationGate {
            scope,
            destination_store: ResourceId(store.to_string()),
            destination_evidence: vec![evidence(store, Plane::Storage, jurisdiction, NOW + 3600)],
            cross_boundary: None,
            verifier: Arc::new(Verifier {
                revoked_serial: None,
            }),
        }
    }

    // ---- Federation. Oracle: design §3 federation row. ----

    #[test]
    fn unscoped_federation_keeps_existing_behaviour() {
        let gate = federation(Scope::Unscoped, "anywhere-store", "us");
        assert!(gate.admit_replication(&projection(), NOW).is_allow());
    }

    #[test]
    fn in_partition_destination_with_current_evidence_replicates() {
        let gate = federation(scoped(), "peer-eu-store", "eu");
        assert!(gate.admit_replication(&projection(), NOW).is_allow());
    }

    #[test]
    fn empty_projection_is_denied() {
        // Only an explicitly selected projection may replicate.
        let gate = federation(scoped(), "peer-eu-store", "eu");
        let decision = gate.admit_replication(&BTreeSet::new(), NOW);
        assert_eq!(decision, RouteDecision::Deny(RouteDeny::EmptyProjection));
    }

    #[test]
    fn destination_outside_allowed_set_is_denied_and_creates_no_replica() {
        // §3 negative test: a destination outside the allowed set is
        // denied. `us-store` is a named resource with valid current
        // evidence — but its jurisdiction is outside the partition
        // and there is no cross-boundary consent, so the denial
        // stands on residency, not on any capability check (the gate
        // takes no capability input at all).
        let gate = federation(scoped(), "us-store", "us");
        let decision = gate.admit_replication(&projection(), NOW);
        assert_eq!(
            decision,
            RouteDecision::Deny(RouteDeny::CrossBoundaryWithoutConsent)
        );
    }

    #[test]
    fn silent_cross_boundary_join_is_rejected_without_consent_record() {
        // §3 negative test, second half: an attempted silent
        // cross-boundary join — here to a store the policy does not
        // name at all — is rejected and does not create a replica.
        let gate = federation(scoped(), "unknown-store", "us");
        assert!(!gate.admit_replication(&projection(), NOW).is_allow());
    }

    #[test]
    fn cross_boundary_with_consent_and_disclosure_replicates() {
        let mut gate = federation(scoped(), "us-store", "us");
        gate.cross_boundary = Some(CrossBoundaryConsent {
            jurisdictions: [Jurisdiction("us".to_string())].into_iter().collect(),
            resources: [ResourceId("us-store".to_string())].into_iter().collect(),
            participant_consent: true,
            disclosed: true,
        });
        assert!(gate.admit_replication(&projection(), NOW).is_allow());
    }

    #[test]
    fn cross_boundary_consent_without_disclosure_is_denied() {
        let mut gate = federation(scoped(), "us-store", "us");
        gate.cross_boundary = Some(CrossBoundaryConsent {
            jurisdictions: [Jurisdiction("us".to_string())].into_iter().collect(),
            resources: [ResourceId("us-store".to_string())].into_iter().collect(),
            participant_consent: true,
            disclosed: false,
        });
        let decision = gate.admit_replication(&projection(), NOW);
        assert_eq!(
            decision,
            RouteDecision::Deny(RouteDeny::CrossBoundaryNotDisclosed)
        );
    }

    #[test]
    fn cross_boundary_consent_naming_another_destination_is_denied() {
        let mut gate = federation(scoped(), "us-store", "us");
        gate.cross_boundary = Some(CrossBoundaryConsent {
            jurisdictions: [Jurisdiction("us".to_string())].into_iter().collect(),
            resources: [ResourceId("other-store".to_string())]
                .into_iter()
                .collect(),
            participant_consent: true,
            disclosed: true,
        });
        assert!(!gate.admit_replication(&projection(), NOW).is_allow());
    }

    #[test]
    fn expired_destination_evidence_is_denied() {
        // §3 common cases: stale evidence refuses, in-partition or not.
        let mut gate = federation(scoped(), "peer-eu-store", "eu");
        gate.destination_evidence[0].expires_at = NOW - 1;
        let decision = gate.admit_replication(&projection(), NOW);
        assert!(matches!(
            decision,
            RouteDecision::Deny(RouteDeny::Residency(ref reasons))
            if reasons.contains(&DenyReason::Expired(Plane::Storage))
        ));
    }

    #[test]
    fn missing_destination_evidence_is_denied() {
        // §3 common cases: unknown destination refuses.
        let mut gate = federation(scoped(), "peer-eu-store", "eu");
        gate.destination_evidence.clear();
        assert!(!gate.admit_replication(&projection(), NOW).is_allow());
    }

    // ---- Relays. Oracle: design §3 relay row. ----

    fn relay_gate(scope: Scope, candidates: &[&str]) -> RelayGate {
        RelayGate {
            scope,
            candidates: candidates
                .iter()
                .map(|c| ResourceId((*c).to_string()))
                .collect(),
            evidence: vec![evidence("relay-eu", Plane::Transport, "eu", NOW + 3600)],
            requirement: TransportRequirement::RelayOrDirect,
            verifier: Arc::new(Verifier {
                revoked_serial: None,
            }),
        }
    }

    #[test]
    fn approved_attested_relay_is_selected() {
        let gate = relay_gate(scoped(), &["relay-eu"]);
        assert_eq!(
            gate.select_relay(NOW),
            Ok(ResourceId("relay-eu".to_string()))
        );
    }

    #[test]
    fn unknown_relay_is_never_selected() {
        // §3 negative test: an unknown relay never receives
        // protected bytes — it is skipped, and with no approved
        // alternative the route has no relay at all.
        let gate = relay_gate(scoped(), &["relay-unknown", "relay-eu"]);
        assert_eq!(
            gate.select_relay(NOW),
            Ok(ResourceId("relay-eu".to_string()))
        );
        let gate = relay_gate(scoped(), &["relay-unknown"]);
        assert_eq!(gate.select_relay(NOW), Err(RouteDeny::NoApprovedRelay));
    }

    #[test]
    fn unapproved_relay_is_never_selected() {
        // A relay with evidence for a jurisdiction the transport
        // policy does not permit is unapproved even though it is
        // evidenced and current.
        let mut gate = relay_gate(scoped(), &["relay-us"]);
        gate.evidence = vec![evidence("relay-us", Plane::Transport, "us", NOW + 3600)];
        assert_eq!(gate.select_relay(NOW), Err(RouteDeny::NoApprovedRelay));
    }

    #[test]
    fn revoked_relay_evidence_is_never_selected() {
        let mut gate = relay_gate(scoped(), &["relay-eu"]);
        gate.verifier = Arc::new(Verifier {
            revoked_serial: Some("relay-eu-1".to_string()),
        });
        assert_eq!(gate.select_relay(NOW), Err(RouteDeny::NoApprovedRelay));
    }

    #[test]
    fn no_candidates_is_reported() {
        let gate = relay_gate(scoped(), &[]);
        assert_eq!(gate.select_relay(NOW), Err(RouteDeny::NoRelayAvailable));
    }

    #[test]
    fn direct_mode_does_not_satisfy_a_proven_route_requirement() {
        // §3 negative test: changing to direct mode alone does not
        // satisfy a strict transport requirement.
        let mut gate = relay_gate(scoped(), &["relay-eu"]);
        gate.requirement = TransportRequirement::ManagedRouteProven;
        assert_eq!(
            gate.admit_direct(),
            RouteDecision::Deny(RouteDeny::DirectRouteInsufficient)
        );
    }

    #[test]
    fn direct_mode_is_admitted_when_relay_or_direct_is_allowed() {
        let gate = relay_gate(scoped(), &["relay-eu"]);
        assert!(gate.admit_direct().is_allow());
    }

    // ---- Metadata egress. Oracle: design §3 relay row + §6. ----

    fn metadata_gate(scope: Scope, destination: &str) -> MetadataGate {
        MetadataGate {
            scope,
            destination: ResourceId(destination.to_string()),
            evidence: vec![evidence(destination, Plane::Metadata, "eu", NOW + 3600)],
            verifier: Arc::new(Verifier {
                revoked_serial: None,
            }),
        }
    }

    #[test]
    fn declared_metadata_destination_with_current_evidence_is_admitted() {
        let gate = metadata_gate(scoped(), "telemetry-eu");
        assert!(gate.admit_egress(MetadataKind::Telemetry, NOW).is_allow());
        assert!(gate.admit_egress(MetadataKind::Discovery, NOW).is_allow());
    }

    #[test]
    fn forbidden_metadata_destination_never_receives_identifiers() {
        // §3 negative test: a forbidden metadata destination never
        // receives protected bytes or identifiers. The route's relay
        // is not a declared metadata destination merely by being the
        // route's relay, even with well-formed evidence offered.
        let gate = metadata_gate(scoped(), "relay-eu");
        let decision = gate.admit_egress(MetadataKind::Telemetry, NOW);
        assert_eq!(
            decision,
            RouteDecision::Deny(RouteDeny::MetadataDestinationNotDeclared)
        );
    }

    #[test]
    fn undeclared_metadata_destination_is_denied() {
        let gate = metadata_gate(scoped(), "telemetry-us");
        assert!(!gate.admit_egress(MetadataKind::Logs, NOW).is_allow());
    }

    #[test]
    fn declared_destination_with_expired_evidence_is_denied() {
        // A declared destination is necessary, not sufficient:
        // telemetry cannot ride on stale placement evidence.
        let mut gate = metadata_gate(scoped(), "telemetry-eu");
        gate.evidence[0].expires_at = NOW - 1;
        assert!(!gate.admit_egress(MetadataKind::Telemetry, NOW).is_allow());
    }

    #[test]
    fn unscoped_metadata_keeps_existing_behaviour() {
        let gate = metadata_gate(Scope::Unscoped, "anywhere");
        assert!(gate.admit_egress(MetadataKind::Discovery, NOW).is_allow());
    }
}
