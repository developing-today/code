//! The shared surface adapter for boundary enforcement (#5683 step 6, issue #7787).
//!
//! Design: `doc/2026-11-08_design_id5683.md` §8. Steps 1–5 built the framework
//! core — [`crate::boundary`] declarations and the one evaluator,
//! [`crate::residency::evaluate_declared`] — but nothing outside `id-core`
//! called it. This module is the one adapter every enforcement surface goes
//! through: a surface declares its enforcement points and the placement hops
//! it supplies, and the adapter maps the enabled declaration plus the
//! evaluator's decision to the one [`FrameworkOutcome`] every surface
//! renders. No surface re-implements evaluation or outcome mapping.
//!
//! The three surfaces from the design's scope table:
//!
//! | Surface | Enforcement points | Hops |
//! | --- | --- | --- |
//! | Signup / login | account creation, login | transport (connection), storage (account store) |
//! | World membership / world blobs | world membership, blob storage | storage (world blob store), processing (world turn host) |
//! | Bridge / federation / relay | bridge/federation, relay transit | transport (peer, relay, bridge) |
//!
//! Two §8 hard rules are enforced here, in the adapter, so no surface can
//! get them wrong:
//!
//! - **Peer-asserted evidence is `Self`-grade until the peer is a trust
//!   root.** [`peer_asserted_claim`] caps a peer's claim about a subject at
//!   [`AssuranceLevel::SelfDeclared`] unless the operator has added that
//!   peer to the policy's trust roots. A missing trust configuration never
//!   activates a boundary, and an untrusted claim cannot satisfy an
//!   evidence bar that requires a trusted issuer.
//! - **The decision log carries classes, never values.** Every non-allow
//!   outcome produces a [`DecisionLogEntry`] with the scope, the boundary
//!   version, the evidence *classes* consulted (plane, source, assurance
//!   level) and the outcome — following the #5883 decision log. The entry's
//!   fields cannot hold a raw evidence value (jurisdiction, resource id,
//!   subject, basis, serial or signature) by construction: those types are
//!   simply not fields of the entry.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::boundary::{
    ActorId, AssuranceLevel as BoundaryAssurance, EffectiveBoundary, EnforcementPoint, ScopeRef,
};
use crate::residency::{
    AssuranceDenyReason, AssuranceLevel as ResidencyAssurance, Declaration, DenyReason,
    EvidenceVerifier, Plane, Scope, SubjectEvidence, evaluate_declared,
};

/// One of the three enforcement surfaces in the design's scope table (§8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Signup / login: check before account creation and on each login.
    SignupLogin,
    /// World membership / world blobs: check before membership changes and
    /// before each protected blob read/write.
    WorldMembershipBlobs,
    /// Bridge / federation / relay: check before outbound enqueue/send and
    /// before inbound accept/ingest.
    BridgeFederationRelay,
}

impl Surface {
    /// The surface's stable name, as recorded in the decision log.
    pub const fn name(self) -> &'static str {
        match self {
            Surface::SignupLogin => "signup_login",
            Surface::WorldMembershipBlobs => "world_membership_blobs",
            Surface::BridgeFederationRelay => "bridge_federation_relay",
        }
    }

    /// The declaration enforcement points this surface renders (§8). An
    /// enabled declaration that names none of them does not cover this
    /// surface, and the adapter allows without evaluating.
    pub const fn enforcement_points(self) -> &'static [EnforcementPoint] {
        match self {
            Surface::SignupLogin => &[EnforcementPoint::AccountCreation, EnforcementPoint::Login],
            Surface::WorldMembershipBlobs => &[
                EnforcementPoint::WorldMembership,
                EnforcementPoint::BlobStorage,
            ],
            Surface::BridgeFederationRelay => &[
                EnforcementPoint::BridgeFederation,
                EnforcementPoint::RelayTransit,
            ],
        }
    }

    /// The placement planes this surface supplies hops for (the issue's
    /// scope table). The adapter requires evidence on every one of them, in
    /// addition to any planes the operator's policy itself requires.
    pub const fn hop_planes(self) -> &'static [Plane] {
        match self {
            Surface::SignupLogin => &[Plane::Transport, Plane::Storage],
            Surface::WorldMembershipBlobs => &[Plane::Storage, Plane::Processing],
            Surface::BridgeFederationRelay => &[Plane::Transport],
        }
    }
}

/// The one outcome every surface renders (§8). A surface never maps the
/// evaluator's decision itself; it renders this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameworkOutcome {
    /// Every required plane and hop was allowed.
    Allow,
    /// Allowed, but the user is told and the decision is logged
    /// (`FailureBehaviour::Warn`: allow, tell the user, log).
    Warn {
        /// Placement denials behind the warning.
        placement: Vec<DenyReason>,
        /// Assurance-bar rejections behind the warning.
        assurance: Vec<AssuranceDenyReason>,
    },
    /// Allowed only after a stronger authentication.
    StepUp {
        /// Placement denials behind the step-up.
        placement: Vec<DenyReason>,
        /// Assurance-bar rejections behind the step-up.
        assurance: Vec<AssuranceDenyReason>,
    },
    /// Allowed into a holding state pending review.
    Quarantine {
        /// Placement denials behind the quarantine.
        placement: Vec<DenyReason>,
        /// Assurance-bar rejections behind the quarantine.
        assurance: Vec<AssuranceDenyReason>,
    },
    /// Refused.
    Deny {
        /// Placement denials behind the refusal.
        placement: Vec<DenyReason>,
        /// Assurance-bar rejections behind the refusal.
        assurance: Vec<AssuranceDenyReason>,
    },
}

impl FrameworkOutcome {
    /// Whether the action proceeds without further conditions. `Warn`
    /// proceeds (the user is told; the decision is logged); `StepUp` and
    /// `Quarantine` are not plain access, and `Deny` refuses.
    pub const fn is_allow(&self) -> bool {
        matches!(
            self,
            FrameworkOutcome::Allow | FrameworkOutcome::Warn { .. }
        )
    }

    /// The outcome's stable name, as recorded in the decision log.
    pub const fn name(&self) -> &'static str {
        match self {
            FrameworkOutcome::Allow => "allow",
            FrameworkOutcome::Warn { .. } => "warn",
            FrameworkOutcome::StepUp { .. } => "step_up",
            FrameworkOutcome::Quarantine { .. } => "quarantine",
            FrameworkOutcome::Deny { .. } => "deny",
        }
    }
}

/// One evidence *class* consulted by a decision: which plane, from which
/// source category, at which assurance level. Deliberately has no field
/// that could carry a raw evidence value (§3.3, §8 audit rule).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EvidenceClass {
    /// The boundary plane the evidence was offered for.
    pub plane: String,
    /// The evidence source category (e.g. a provider control plane) — a
    /// class label, never the evidence itself.
    pub source: String,
    /// How strongly the claim was established.
    pub assurance: BoundaryAssurance,
}

/// A #5883-style decision-log entry (§8): scope, boundary version,
/// evidence classes and assurance levels consulted, the outcome, and the
/// actor. Raw evidence values are not logged — no field of this struct
/// can hold one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionLogEntry {
    /// The surface that rendered the decision.
    pub surface: String,
    /// The scope that owns the boundary.
    pub scope: ScopeRef,
    /// The boundary (declaration) version in force.
    pub boundary_version: u64,
    /// The evidence classes consulted, deduplicated and ordered.
    pub evidence_classes: Vec<EvidenceClass>,
    /// The minimum assurance the boundary required, when one was in force.
    pub minimum_assurance: Option<BoundaryAssurance>,
    /// The rendered outcome (`deny`, `step_up`, `quarantine`, or `warn`).
    pub outcome: String,
    /// The typed reason classes behind the outcome — names, never values.
    pub reasons: Vec<String>,
    /// Who the boundary attributes the decision to.
    pub actor: ActorId,
    /// Unix seconds when the decision was rendered.
    pub at: u64,
}

/// The result of one surface check: the outcome to render, and the
/// decision-log entry to persist when the outcome is not a plain allow
/// (§8: every deny, step-up or quarantine is logged; `warn` logs by its
/// own definition).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceCheck {
    /// The one framework outcome for this check.
    pub outcome: FrameworkOutcome,
    /// The decision-log entry for a non-allow outcome; `None` on allow.
    pub log_entry: Option<DecisionLogEntry>,
}

/// `boundary`'s assurance level for `residency`'s: the two modules grew
/// isomorphic level types in separate steps; the adapter is where they
/// meet, so the conversion lives here and nowhere else.
pub const fn assurance_to_boundary(level: ResidencyAssurance) -> BoundaryAssurance {
    match level {
        ResidencyAssurance::SelfDeclared => BoundaryAssurance::SelfDeclared,
        ResidencyAssurance::OrgAttested => BoundaryAssurance::OrgAttested,
        ResidencyAssurance::ProviderVerified => BoundaryAssurance::ProviderVerified,
    }
}

/// `residency`'s assurance level for `boundary`'s; see
/// [`assurance_to_boundary`].
pub const fn assurance_to_residency(level: BoundaryAssurance) -> ResidencyAssurance {
    match level {
        BoundaryAssurance::SelfDeclared => ResidencyAssurance::SelfDeclared,
        BoundaryAssurance::OrgAttested => ResidencyAssurance::OrgAttested,
        BoundaryAssurance::ProviderVerified => ResidencyAssurance::ProviderVerified,
    }
}

/// Whether `peer` is one of the operator's configured trust roots.
pub fn is_trusted_peer(peer: &str, trust_roots: &BTreeSet<String>) -> bool {
    trust_roots.contains(peer)
}

/// A peer's claim about a subject, graded as the design grades it (§8):
/// a federated peer's assertion is `Self`-grade evidence from that peer
/// until the operator adds the peer to its trust roots. An untrusted
/// peer's claim is returned with its assurance capped at
/// [`ResidencyAssurance::SelfDeclared`], so it cannot satisfy a bar that
/// requires a trusted issuer; a trust root's claim passes unchanged.
/// Placement trust is separate and unchanged: [`crate::residency::evaluate`]
/// still rejects placement evidence whose issuer is not a policy trust
/// root, whatever grade the claim carries.
pub fn peer_asserted_claim(
    peer: &str,
    trust_roots: &BTreeSet<String>,
    mut claim: SubjectEvidence,
) -> SubjectEvidence {
    if !is_trusted_peer(peer, trust_roots) {
        claim.assurance = ResidencyAssurance::SelfDeclared;
    }
    claim
}

fn plane_name(plane: Plane) -> &'static str {
    match plane {
        Plane::Storage => "storage",
        Plane::Processing => "processing",
        Plane::Transport => "transport",
        Plane::Metadata => "metadata",
    }
}

fn deny_reason_class(reason: &DenyReason) -> &'static str {
    match reason {
        DenyReason::NoRequiredPlanes => "no_required_planes",
        DenyReason::PlaneUnknown(_) => "plane_unknown",
        DenyReason::NoEvidence(_) => "no_evidence",
        DenyReason::UntrustedIssuer(_) => "untrusted_issuer",
        DenyReason::BadSignature(_) => "bad_signature",
        DenyReason::Revoked(_) => "revoked",
        DenyReason::Expired(_) => "expired",
        DenyReason::BindingMismatch(_) => "binding_mismatch",
        DenyReason::OutOfPolicy(_) => "out_of_policy",
    }
}

fn assurance_reason_class(reason: &AssuranceDenyReason) -> &'static str {
    match reason {
        AssuranceDenyReason::BelowAssuranceBar { .. } => "below_assurance_bar",
        AssuranceDenyReason::NoSubjectEvidence(_) => "no_subject_evidence",
    }
}

/// Check one surface action through the shared evaluator. This is the
/// only path a surface uses: it supplies its hops (as `claims` about the
/// subject), and the adapter does the rest —
///
/// 1. No enabled boundary (`effective` carries no capability/failure), or
///    an enabled boundary that names none of this surface's enforcement
///    points, does not cover this action: allow, no log entry. A missing
///    trust configuration never activates a boundary (§8).
/// 2. Otherwise the declaration for the evaluator is assembled from the
///    effective boundary's minimum assurance and a scope whose required
///    planes are the policy's own plus this surface's hop planes, and
///    [`evaluate_declared`] decides — the adapter adds no evaluation of
///    its own.
/// 3. The decision is mapped through the effective boundary's failure
///    behaviour to the one [`FrameworkOutcome`], and every non-allow
///    outcome yields its [`DecisionLogEntry`].
#[allow(clippy::too_many_arguments)]
pub fn check_surface(
    surface: Surface,
    effective: &EffectiveBoundary,
    boundary_version: u64,
    scope_ref: &ScopeRef,
    actor: &ActorId,
    subject: &str,
    scope: &Scope,
    claims: &[SubjectEvidence],
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> SurfaceCheck {
    let allow = SurfaceCheck {
        outcome: FrameworkOutcome::Allow,
        log_entry: None,
    };
    let (Some(_capability), Some(failure)) = (effective.capability, effective.failure) else {
        return allow;
    };
    if !surface
        .enforcement_points()
        .iter()
        .any(|point| effective.enforcement.contains(point))
    {
        return allow;
    }

    let scoped = match scope {
        Scope::Unscoped => Scope::Unscoped,
        Scope::Scoped(policy) => {
            let mut policy = policy.clone();
            for plane in surface.hop_planes() {
                policy.required.insert(*plane);
            }
            Scope::Scoped(policy)
        }
    };
    let declaration = Declaration {
        scope: scoped,
        subject: subject.to_string(),
        min_assurance: effective
            .minimum_assurance
            .map(assurance_to_residency)
            .unwrap_or(ResidencyAssurance::SelfDeclared),
    };
    let (placement, assurance) = match evaluate_declared(&declaration, claims, now, verifier) {
        crate::residency::AssuredDecision::Allow => return allow,
        crate::residency::AssuredDecision::Deny(reasons) => (reasons, Vec::new()),
        crate::residency::AssuredDecision::AssuranceDenied(reasons) => (Vec::new(), reasons),
    };

    let outcome = match failure {
        crate::boundary::FailureBehaviour::Warn => FrameworkOutcome::Warn {
            placement,
            assurance,
        },
        crate::boundary::FailureBehaviour::StepUp => FrameworkOutcome::StepUp {
            placement,
            assurance,
        },
        crate::boundary::FailureBehaviour::Quarantine => FrameworkOutcome::Quarantine {
            placement,
            assurance,
        },
        crate::boundary::FailureBehaviour::Deny => FrameworkOutcome::Deny {
            placement,
            assurance,
        },
    };

    let (placement, assurance) = match &outcome {
        FrameworkOutcome::Warn {
            placement,
            assurance,
        }
        | FrameworkOutcome::StepUp {
            placement,
            assurance,
        }
        | FrameworkOutcome::Quarantine {
            placement,
            assurance,
        }
        | FrameworkOutcome::Deny {
            placement,
            assurance,
        } => (placement, assurance),
        FrameworkOutcome::Allow => unreachable!("allow returned above"),
    };
    let mut reasons: Vec<String> = placement
        .iter()
        .map(|r| deny_reason_class(r).to_string())
        .chain(
            assurance
                .iter()
                .map(|r| assurance_reason_class(r).to_string()),
        )
        .collect();
    reasons.sort();
    reasons.dedup();

    let evidence_classes: BTreeSet<EvidenceClass> = claims
        .iter()
        .filter(|claim| claim.subject == subject)
        .map(|claim| EvidenceClass {
            plane: plane_name(claim.placement_evidence.plane).to_string(),
            source: claim.placement_evidence.source.clone(),
            assurance: assurance_to_boundary(claim.assurance),
        })
        .collect();

    SurfaceCheck {
        log_entry: Some(DecisionLogEntry {
            surface: surface.name().to_string(),
            scope: scope_ref.clone(),
            boundary_version,
            evidence_classes: evidence_classes.into_iter().collect(),
            minimum_assurance: effective.minimum_assurance,
            outcome: outcome.name().to_string(),
            reasons,
            actor: actor.clone(),
            at: now,
        }),
        outcome,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::boundary::{
        BoundaryDeclaration, BoundaryDeclarationBody, CapabilityKind, FailureBehaviour,
        SubjectClass,
    };
    use crate::residency::{Evidence, Jurisdiction, Policy, ResourceId};

    struct OkVerifier;
    impl EvidenceVerifier for OkVerifier {
        fn signature_valid(&self, _: &Evidence) -> bool {
            true
        }
        fn revoked(&self, _: &Evidence) -> bool {
            false
        }
    }
    const OKV: OkVerifier = OkVerifier;

    const NOW: u64 = 50;

    fn policy() -> Policy {
        let mut p = Policy::new(["root".to_string()]);
        for pl in Plane::ALL {
            p = p
                .allow(pl, &["eu"], &["r"])
                .trust_evidence_source(pl, "control-plane");
        }
        p
    }

    fn claim(subject: &str, plane: Plane, assurance: ResidencyAssurance) -> SubjectEvidence {
        SubjectEvidence {
            subject: subject.into(),
            assurance,
            basis: "attestation-ref".into(),
            privacy_class: crate::residency::PrivacyClass::VerdictOnly,
            placement_evidence: Evidence {
                issuer: "root".into(),
                resource: ResourceId("r".into()),
                plane,
                jurisdiction: Jurisdiction("eu".into()),
                source: "control-plane".into(),
                expires_at: 100,
                serial: "1".into(),
                signature: vec![],
            },
        }
    }

    fn claims(subject: &str, assurance: ResidencyAssurance) -> Vec<SubjectEvidence> {
        Plane::ALL
            .iter()
            .map(|p| claim(subject, *p, assurance))
            .collect()
    }

    fn declaration(
        enforcement: &[EnforcementPoint],
        failure: FailureBehaviour,
        version: u64,
    ) -> BoundaryDeclaration {
        BoundaryDeclaration::new(BoundaryDeclarationBody {
            capability: CapabilityKind::Residency,
            scope: ScopeRef::Server("srv".into()),
            subject_class: SubjectClass::User,
            evidence_req: crate::boundary::EvidenceRequirement {
                minimum_assurance: BoundaryAssurance::OrgAttested,
                accepted_sources: BTreeSet::new(),
            },
            enforcement: enforcement.iter().copied().collect(),
            failure,
            data_ref: None,
            enabled: true,
            version,
            declared_by: ActorId("admin".into()),
            declared_at: 1,
        })
        .expect("valid declaration")
    }

    fn effective_for(
        enforcement: &[EnforcementPoint],
        failure: FailureBehaviour,
    ) -> EffectiveBoundary {
        let d = declaration(enforcement, failure, 7);
        crate::boundary::effective(Some(&d), None, None).expect("composes")
    }

    fn check(
        surface: Surface,
        effective: &EffectiveBoundary,
        claims: &[SubjectEvidence],
    ) -> SurfaceCheck {
        check_surface(
            surface,
            effective,
            7,
            &ScopeRef::Server("srv".into()),
            &ActorId("admin".into()),
            "alice",
            &Scope::Scoped(policy()),
            claims,
            NOW,
            &OKV,
        )
    }

    #[test]
    fn every_surface_allows_through_the_shared_path() {
        // Oracle (acceptance, issue #7787): all three surfaces evaluate
        // through the one evaluator — at-bar claims covering each
        // surface's hops allow on every surface, with no log entry.
        for surface in [
            Surface::SignupLogin,
            Surface::WorldMembershipBlobs,
            Surface::BridgeFederationRelay,
        ] {
            let eff = effective_for(surface.enforcement_points(), FailureBehaviour::Deny);
            let result = check(
                surface,
                &eff,
                &claims("alice", ResidencyAssurance::OrgAttested),
            );
            assert_eq!(result.outcome, FrameworkOutcome::Allow, "{surface:?}");
            assert!(result.log_entry.is_none(), "{surface:?}");
        }
    }

    #[test]
    fn every_surface_denies_through_the_shared_path_and_logs() {
        // Oracle (acceptance, issue #7787): a surface whose hop has no
        // subject evidence denies through the same shared path, and the
        // denial is logged with scope and boundary version.
        for surface in [
            Surface::SignupLogin,
            Surface::WorldMembershipBlobs,
            Surface::BridgeFederationRelay,
        ] {
            let eff = effective_for(surface.enforcement_points(), FailureBehaviour::Deny);
            // Drop one of the surface's own hop planes.
            let missing = surface.hop_planes()[0];
            let offered: Vec<SubjectEvidence> = claims("alice", ResidencyAssurance::OrgAttested)
                .into_iter()
                .filter(|c| c.placement_evidence.plane != missing)
                .collect();
            let result = check(surface, &eff, &offered);
            assert!(
                matches!(result.outcome, FrameworkOutcome::Deny { .. }),
                "{surface:?} got {:?}",
                result.outcome
            );
            let entry = result.log_entry.expect("denial is logged");
            assert_eq!(entry.surface, surface.name());
            assert_eq!(entry.scope, ScopeRef::Server("srv".into()));
            assert_eq!(entry.outcome, "deny");
            assert!(
                entry.reasons.iter().any(|r| r == "no_subject_evidence"),
                "{surface:?} reasons {:?}",
                entry.reasons
            );
        }
    }

    #[test]
    fn failure_behaviour_maps_to_the_one_framework_outcome() {
        // Oracle (design §8): each surface renders the same
        // FrameworkOutcome; the declaration's failure behaviour selects
        // deny / step-up / quarantine / warn, never a per-surface mapping.
        let eff = effective_for(&[EnforcementPoint::Login], FailureBehaviour::StepUp);
        let result = check(
            Surface::SignupLogin,
            &eff,
            &claims("alice", ResidencyAssurance::SelfDeclared),
        );
        assert!(matches!(result.outcome, FrameworkOutcome::StepUp { .. }));
        assert_eq!(result.log_entry.expect("logged").outcome, "step_up");

        let eff = effective_for(&[EnforcementPoint::Login], FailureBehaviour::Quarantine);
        let result = check(
            Surface::SignupLogin,
            &eff,
            &claims("alice", ResidencyAssurance::SelfDeclared),
        );
        assert!(matches!(
            result.outcome,
            FrameworkOutcome::Quarantine { .. }
        ));

        let eff = effective_for(&[EnforcementPoint::Login], FailureBehaviour::Warn);
        let result = check(
            Surface::SignupLogin,
            &eff,
            &claims("alice", ResidencyAssurance::SelfDeclared),
        );
        assert!(matches!(result.outcome, FrameworkOutcome::Warn { .. }));
        assert!(result.outcome.is_allow(), "warn allows, told and logged");
        assert!(result.log_entry.is_some(), "warn is logged");
    }

    #[test]
    fn a_surface_the_declaration_does_not_name_is_not_covered() {
        // Oracle (design §8 + boundary §5.2): an omitted surface is not
        // covered — a declaration naming only account creation does not
        // activate the bridge surface, even with no evidence at all.
        let eff = effective_for(&[EnforcementPoint::AccountCreation], FailureBehaviour::Deny);
        let result = check(Surface::BridgeFederationRelay, &eff, &[]);
        assert_eq!(result.outcome, FrameworkOutcome::Allow);
        assert!(result.log_entry.is_none());
    }

    #[test]
    fn no_enabled_boundary_never_activates_a_surface() {
        // Oracle (design §8): a missing trust configuration never
        // activates a boundary.
        let eff = crate::boundary::effective(None, None, None).expect("composes");
        let result = check(Surface::SignupLogin, &eff, &[]);
        assert_eq!(result.outcome, FrameworkOutcome::Allow);
    }

    #[test]
    fn peer_claim_is_self_grade_until_the_peer_is_a_trust_root() {
        // Oracle (acceptance, issue #7787): a federated peer's location
        // claim is unverified unless the peer is a trust root. The peer
        // asserts ProviderVerified for every plane; graded Self, it is
        // below the OrgAttested bar and the bridge check denies. Added
        // to the trust roots, the same claims keep their grade.
        let peer = "peer-x";
        let asserted: Vec<SubjectEvidence> = Plane::ALL
            .iter()
            .map(|p| {
                let mut c = claim("alice", *p, ResidencyAssurance::ProviderVerified);
                c.placement_evidence.issuer = peer.into();
                c
            })
            .collect();

        let no_roots = BTreeSet::new();
        let graded: Vec<SubjectEvidence> = asserted
            .iter()
            .cloned()
            .map(|c| peer_asserted_claim(peer, &no_roots, c))
            .collect();
        assert!(
            graded
                .iter()
                .all(|c| c.assurance == ResidencyAssurance::SelfDeclared),
            "untrusted peer claims are capped at Self"
        );

        let roots: BTreeSet<String> = [peer.to_string()].into_iter().collect();
        let kept: Vec<SubjectEvidence> = asserted
            .iter()
            .cloned()
            .map(|c| peer_asserted_claim(peer, &roots, c))
            .collect();
        assert!(
            kept.iter()
                .all(|c| c.assurance == ResidencyAssurance::ProviderVerified),
            "a trust root's claims keep their grade"
        );

        // And the graded claims actually decide: with the peer as the
        // policy's only trust root, the Self-graded set is denied at the
        // bar while the trust-root set is decided by placement alone.
        let mut p = Policy::new([peer.to_string()]);
        for pl in Plane::ALL {
            p = p
                .allow(pl, &["eu"], &["r"])
                .trust_evidence_source(pl, "control-plane");
        }
        let eff = effective_for(
            &[EnforcementPoint::BridgeFederation],
            FailureBehaviour::Deny,
        );
        let denied = check_surface(
            Surface::BridgeFederationRelay,
            &eff,
            7,
            &ScopeRef::Server("srv".into()),
            &ActorId("admin".into()),
            "alice",
            &Scope::Scoped(p.clone()),
            &graded,
            NOW,
            &OKV,
        );
        assert!(
            matches!(denied.outcome, FrameworkOutcome::Deny { .. }),
            "Self-graded peer claims below the bar deny, got {:?}",
            denied.outcome
        );
        let allowed = check_surface(
            Surface::BridgeFederationRelay,
            &eff,
            7,
            &ScopeRef::Server("srv".into()),
            &ActorId("admin".into()),
            "alice",
            &Scope::Scoped(p),
            &kept,
            NOW,
            &OKV,
        );
        assert_eq!(allowed.outcome, FrameworkOutcome::Allow);
    }

    #[test]
    fn decision_log_carries_classes_not_values() {
        // Oracle (acceptance, issue #7787; design §3.3): decision-log
        // entries carry evidence classes, not values. Distinctive raw
        // values in the claims must appear nowhere in the serialized
        // entry; the class labels must.
        let mut offered = claims("alice", ResidencyAssurance::SelfDeclared);
        for c in &mut offered {
            c.placement_evidence.jurisdiction = Jurisdiction("atlantis-raw".into());
            c.placement_evidence.resource = ResourceId("store-raw-xyz".into());
            c.placement_evidence.serial = "serial-raw-xyz".into();
            c.placement_evidence.signature = vec![0xde, 0xad, 0xbe, 0xef];
            c.basis = "basis-raw-xyz".into();
        }
        let eff = effective_for(&[EnforcementPoint::Login], FailureBehaviour::Deny);
        let result = check(Surface::SignupLogin, &eff, &offered);
        let entry = result.log_entry.expect("denial is logged");
        let json = serde_json::to_string(&entry).expect("serialize entry");
        for raw in [
            "atlantis-raw",
            "store-raw-xyz",
            "serial-raw-xyz",
            "basis-raw-xyz",
            "alice",
        ] {
            assert!(
                !json.contains(raw),
                "log entry leaked raw value {raw}: {json}"
            );
        }
        assert!(
            json.contains("control-plane"),
            "source class logged: {json}"
        );
        assert!(
            json.contains("self_declared"),
            "assurance class logged: {json}"
        );
        assert!(json.contains("storage"), "plane class logged: {json}");
    }

    #[test]
    fn log_entry_records_the_boundary_version_in_force() {
        // Oracle (design §8 audit): the entry records the boundary
        // version in force for the decision — the version the surface
        // passes from its declaration (7 in the fixture).
        let d = declaration(&[EnforcementPoint::Login], FailureBehaviour::Deny, 7);
        assert_eq!(d.body.version, 7);
        let eff = crate::boundary::effective(Some(&d), None, None).expect("composes");
        let result = check(
            Surface::SignupLogin,
            &eff,
            &claims("alice", ResidencyAssurance::SelfDeclared),
        );
        let entry = result.log_entry.expect("logged");
        assert_eq!(entry.boundary_version, 7);
        assert_eq!(
            entry.minimum_assurance,
            Some(BoundaryAssurance::OrgAttested)
        );
        assert_eq!(entry.actor, ActorId("admin".into()));
    }
}
