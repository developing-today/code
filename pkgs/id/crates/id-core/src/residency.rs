//! Residency policy vocabulary and a pure evaluator (#3398 design step 1, issue #5529).
//!
//! Design: `doc/2026-11-01_design_id3398.md`. Nothing here touches the network, a store or a
//! protocol, so no route changes behaviour. Later steps call [`evaluate`] before send, accept,
//! persist, queue, execute and retry.
//!
//! The rules, from the design:
//!
//! - A scope is explicit. [`Scope::Unscoped`] content is always allowed here (today's behaviour).
//! - A scoped path is denied unless every required plane has current, trusted, resource-bound
//!   evidence in a permitted jurisdiction. Missing, unknown, expired, revoked and mismatched
//!   evidence all deny.
//! - Evidence counts only when its issuer is one of the operator's configured trust roots and the
//!   operator's [`EvidenceVerifier`] accepts it. A peer's own location statement is never
//!   evidence, and a signature proves authorship, not physical location.
//! - Planes are independent: storage evidence never satisfies processing, transport or metadata.

use std::collections::{BTreeMap, BTreeSet};

/// One of the four separately modelled boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Plane {
    /// Journals, modules, replicas, backups, caches, queues and recovery copies.
    Storage,
    /// A host running a request, world turn, indexer or native/Wasm program.
    Processing,
    /// A network peer, relay, bridge, proxy or store-and-forward hop.
    Transport,
    /// Discovery, logs, traces, metrics and other descriptive exhaust.
    Metadata,
}

impl Plane {
    /// All independent policy planes; used by the fail-closed default policy.
    pub const ALL: [Plane; 4] = [
        Plane::Storage,
        Plane::Processing,
        Plane::Transport,
        Plane::Metadata,
    ];
}

/// A jurisdiction label such as `"eu"`. Compared exactly; an empty label is never valid.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Jurisdiction(pub String);

/// The service or resource a path uses: a store, host, relay or telemetry sink.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub String);

/// Whether content is residency-scoped. Scope is assigned locally; a peer cannot remove it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    /// Existing behavior for content without an explicit residency policy.
    Unscoped,
    /// Content governed by the supplied policy.
    Scoped(Policy),
}

/// What one plane permits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanePolicy {
    /// Jurisdictions permitted for this plane.
    pub jurisdictions: BTreeSet<Jurisdiction>,
    /// Evidence sources the operator accepts for this plane.
    pub evidence_sources: BTreeSet<String>,
    /// Permitted resource identities. Empty permits no resources.
    pub resources: BTreeSet<ResourceId>,
}

impl Default for PlanePolicy {
    fn default() -> Self {
        Self {
            jurisdictions: BTreeSet::new(),
            evidence_sources: BTreeSet::new(),
            resources: BTreeSet::new(),
        }
    }
}

/// Per-plane residency policy. A plane not listed in `planes` permits nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Allow-lists for individual planes. An omitted plane permits nothing.
    pub planes: BTreeMap<Plane, PlanePolicy>,
    /// Planes a path must prove. Defaults to all four.
    pub required: BTreeSet<Plane>,
    /// Issuers this operator trusts for evidence under this policy.
    pub trust_roots: BTreeSet<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            planes: BTreeMap::new(),
            required: Plane::ALL.into_iter().collect(),
            trust_roots: BTreeSet::new(),
        }
    }
}

impl Policy {
    /// Build a default-deny policy requiring evidence for every plane.
    pub fn new(trust_roots: impl IntoIterator<Item = String>) -> Self {
        Self {
            planes: BTreeMap::new(),
            required: Plane::ALL.into_iter().collect(),
            trust_roots: trust_roots.into_iter().collect(),
        }
    }

    /// Add allowed jurisdictions and resource identities for one plane.
    pub fn allow(mut self, plane: Plane, jurisdictions: &[&str], resources: &[&str]) -> Self {
        let p = self.planes.entry(plane).or_default();
        p.jurisdictions
            .extend(jurisdictions.iter().map(|j| Jurisdiction((*j).into())));
        p.resources
            .extend(resources.iter().map(|r| ResourceId((*r).into())));
        self
    }

    /// Add an operator-approved evidence source for a plane.
    pub fn trust_evidence_source(mut self, plane: Plane, source: &str) -> Self {
        self.planes
            .entry(plane)
            .or_default()
            .evidence_sources
            .insert(source.into());
        self
    }
}

/// An issuer's assertion that a resource sits in a jurisdiction for a plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    /// Locally trusted identity of the evidence issuer.
    pub issuer: String,
    /// Resource this evidence is bound to.
    pub resource: ResourceId,
    /// Plane this evidence establishes.
    pub plane: Plane,
    /// Claimed location of the bound resource.
    pub jurisdiction: Jurisdiction,
    /// Evidence source (e.g. provider control plane or contract record).
    pub source: String,
    /// Unix seconds; evidence is valid while `now < expires_at`.
    pub expires_at: u64,
    /// Issuer-assigned identifier, checked by the verifier for revocation.
    pub serial: String,
    /// Authenticated assertion bytes interpreted by the operator's verifier.
    pub signature: Vec<u8>,
}

/// Operator-side check of an assertion's signature and revocation state. Implementations decide
/// whether the issuer's record is trustworthy; the evaluator only enforces the outcome.
pub trait EvidenceVerifier {
    /// Validate that this issuer authenticated this exact assertion.
    fn signature_valid(&self, evidence: &Evidence) -> bool;
    /// Report whether the assertion or its issuer has been revoked.
    fn revoked(&self, evidence: &Evidence) -> bool;
}

/// One resource a path touches on one plane, and the evidence offered for it.
#[derive(Clone, Debug)]
pub struct Hop<'a> {
    /// Boundary plane crossed by this hop.
    pub plane: Plane,
    /// Exact service/resource used by this hop.
    pub resource: ResourceId,
    /// Placement evidence supplied for this resource and plane.
    pub evidence: Option<&'a Evidence>,
}

/// Why a path was denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    /// A scoped policy has no required planes, so its boundary is undefined.
    NoRequiredPlanes,
    /// A required plane has no hop at all, so its placement is unknown.
    PlaneUnknown(Plane),
    /// The hop has no placement evidence.
    NoEvidence(Plane),
    /// The evidence issuer is not in the local policy's trust roots.
    UntrustedIssuer(Plane),
    /// The operator's verifier rejected the signature.
    BadSignature(Plane),
    /// The operator's verifier reports this evidence revoked.
    Revoked(Plane),
    /// The evidence is no longer fresh.
    Expired(Plane),
    /// Evidence is for another resource or plane than the hop uses.
    BindingMismatch(Plane),
    /// The plane or resource is not permitted by the policy.
    OutOfPolicy(Plane),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Every required plane and hop was allowed.
    Allow,
    /// One or more path checks failed.
    Deny(Vec<DenyReason>),
}

impl Decision {
    /// Whether the path is allowed.
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// Evaluate a path. Every hop is checked, and every required plane must be covered. Denial lists
/// every reason found. Nothing is persisted or sent here.
pub fn evaluate(
    scope: &Scope,
    hops: &[Hop<'_>],
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Decision {
    let policy = match scope {
        Scope::Unscoped => return Decision::Allow,
        Scope::Scoped(p) => p,
    };
    let mut reasons = Vec::new();
    if policy.required.is_empty() {
        reasons.push(DenyReason::NoRequiredPlanes);
    }
    for plane in &policy.required {
        if !hops.iter().any(|h| h.plane == *plane) {
            reasons.push(DenyReason::PlaneUnknown(*plane));
        }
    }
    for hop in hops {
        if let Some(r) = check_hop(policy, hop, now, verifier) {
            reasons.push(r);
        }
    }
    if reasons.is_empty() {
        Decision::Allow
    } else {
        Decision::Deny(reasons)
    }
}

fn check_hop(
    policy: &Policy,
    hop: &Hop<'_>,
    now: u64,
    verifier: &dyn EvidenceVerifier,
) -> Option<DenyReason> {
    let plane = hop.plane;
    let Some(ev) = hop.evidence else {
        return Some(DenyReason::NoEvidence(plane));
    };
    if !policy.trust_roots.contains(&ev.issuer) {
        return Some(DenyReason::UntrustedIssuer(plane));
    }
    if !verifier.signature_valid(ev) {
        return Some(DenyReason::BadSignature(plane));
    }
    if verifier.revoked(ev) {
        return Some(DenyReason::Revoked(plane));
    }
    if now >= ev.expires_at {
        return Some(DenyReason::Expired(plane));
    }
    if ev.plane != plane || ev.resource != hop.resource {
        return Some(DenyReason::BindingMismatch(plane));
    }
    let Some(pp) = policy.planes.get(&plane) else {
        return Some(DenyReason::OutOfPolicy(plane));
    };
    if ev.jurisdiction.0.is_empty()
        || !pp.jurisdictions.contains(&ev.jurisdiction)
        || !pp.evidence_sources.contains(&ev.source)
        || !pp.resources.contains(&hop.resource)
    {
        return Some(DenyReason::OutOfPolicy(plane));
    }
    None
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    struct V {
        sig_ok: bool,
        revoked: bool,
    }
    impl EvidenceVerifier for V {
        fn signature_valid(&self, _: &Evidence) -> bool {
            self.sig_ok
        }
        fn revoked(&self, _: &Evidence) -> bool {
            self.revoked
        }
    }
    const OK: V = V {
        sig_ok: true,
        revoked: false,
    };

    fn policy() -> Policy {
        let mut p = Policy::new(["root".to_string()]);
        for pl in Plane::ALL {
            p = p
                .allow(pl, &["eu"], &["r"])
                .trust_evidence_source(pl, "control-plane");
        }
        p
    }

    fn ev(plane: Plane, res: &str) -> Evidence {
        Evidence {
            issuer: "root".into(),
            resource: ResourceId(res.into()),
            plane,
            jurisdiction: Jurisdiction("eu".into()),
            source: "control-plane".into(),
            expires_at: 100,
            serial: "1".into(),
            signature: vec![],
        }
    }

    fn evs() -> Vec<Evidence> {
        Plane::ALL.iter().map(|p| ev(*p, "r")).collect()
    }

    fn hops(evs: &[Evidence]) -> Vec<Hop<'_>> {
        evs.iter()
            .map(|e| Hop {
                plane: e.plane,
                resource: e.resource.clone(),
                evidence: Some(e),
            })
            .collect()
    }

    fn run(evs: &[Evidence], now: u64, v: &V) -> Decision {
        evaluate(&Scope::Scoped(policy()), &hops(evs), now, v)
    }

    #[test]
    fn unscoped_always_allowed() {
        assert!(evaluate(&Scope::Unscoped, &[], 0, &OK).is_allow());
    }

    #[test]
    fn full_evidence_allows() {
        assert!(run(&evs(), 50, &OK).is_allow());
    }

    #[test]
    fn missing_plane_is_unknown_and_denied() {
        let e = evs();
        let h = hops(&e[..3]);
        let d = evaluate(&Scope::Scoped(policy()), &h, 50, &OK);
        assert_eq!(
            d,
            Decision::Deny(vec![DenyReason::PlaneUnknown(Plane::Metadata)])
        );
    }

    #[test]
    fn no_hops_denied() {
        assert!(!evaluate(&Scope::Scoped(policy()), &[], 50, &OK).is_allow());
    }

    #[test]
    fn default_scoped_policy_is_fail_closed() {
        assert!(!evaluate(&Scope::Scoped(Policy::default()), &[], 50, &OK).is_allow());
    }

    #[test]
    fn empty_required_plane_set_is_denied() {
        let mut p = policy();
        p.required.clear();
        assert_eq!(
            evaluate(&Scope::Scoped(p), &[], 50, &OK),
            Decision::Deny(vec![DenyReason::NoRequiredPlanes])
        );
    }

    #[test]
    fn storage_evidence_does_not_cover_other_planes() {
        let e = ev(Plane::Storage, "r");
        let h: Vec<_> = Plane::ALL
            .iter()
            .map(|p| Hop {
                plane: *p,
                resource: ResourceId("r".into()),
                evidence: Some(&e),
            })
            .collect();
        let Decision::Deny(r) = evaluate(&Scope::Scoped(policy()), &h, 50, &OK) else {
            panic!("allowed")
        };
        assert_eq!(r.len(), 3);
        assert!(
            r.iter()
                .all(|x| matches!(x, DenyReason::BindingMismatch(_)))
        );
    }

    #[test]
    fn expired_denied_at_boundary() {
        assert!(!run(&evs(), 100, &OK).is_allow());
        assert!(run(&evs(), 99, &OK).is_allow());
    }

    #[test]
    fn revoked_and_bad_signature_denied() {
        let rev = V {
            sig_ok: true,
            revoked: true,
        };
        let bad = V {
            sig_ok: false,
            revoked: false,
        };
        assert!(
            matches!(run(&evs(), 50, &rev), Decision::Deny(r) if r.contains(&DenyReason::Revoked(Plane::Storage)))
        );
        assert!(
            matches!(run(&evs(), 50, &bad), Decision::Deny(r) if r.contains(&DenyReason::BadSignature(Plane::Storage)))
        );
    }

    #[test]
    fn untrusted_issuer_denied() {
        let mut e = evs();
        e[0].issuer = "peer-self".into();
        assert!(
            matches!(run(&e, 50, &OK), Decision::Deny(r) if r == vec![DenyReason::UntrustedIssuer(Plane::Storage)])
        );
    }

    #[test]
    fn missing_evidence_denied() {
        let e = evs();
        let mut h = hops(&e);
        h[1].evidence = None;
        assert_eq!(
            evaluate(&Scope::Scoped(policy()), &h, 50, &OK),
            Decision::Deny(vec![DenyReason::NoEvidence(Plane::Processing)])
        );
    }

    #[test]
    fn wrong_resource_binding_denied() {
        let e = evs();
        let mut h = hops(&e);
        h[2].resource = ResourceId("other".into());
        assert_eq!(
            evaluate(&Scope::Scoped(policy()), &h, 50, &OK),
            Decision::Deny(vec![DenyReason::BindingMismatch(Plane::Transport)])
        );
    }

    #[test]
    fn out_of_policy_jurisdiction_and_resource_denied() {
        let mut e = evs();
        e[3].jurisdiction = Jurisdiction("us".into());
        assert!(
            matches!(run(&e, 50, &OK), Decision::Deny(r) if r == vec![DenyReason::OutOfPolicy(Plane::Metadata)])
        );
        let p = Policy::new(["root".to_string()]).allow(Plane::Storage, &["eu"], &["other"]);
        let mut p2 = p;
        p2.required = [Plane::Storage].into();
        let e = vec![ev(Plane::Storage, "r")];
        assert!(!evaluate(&Scope::Scoped(p2), &hops(&e), 50, &OK).is_allow());
    }

    #[test]
    fn unsupported_evidence_source_is_denied() {
        let mut e = evs();
        e[0].source = "unverified-screenshot".into();
        assert!(matches!(
            run(&e, 50, &OK),
            Decision::Deny(r) if r == vec![DenyReason::OutOfPolicy(Plane::Storage)]
        ));
    }

    #[test]
    fn plane_absent_from_policy_denied() {
        let mut p = Policy::new(["root".to_string()]);
        p.required = [Plane::Storage].into();
        let e = vec![ev(Plane::Storage, "r")];
        assert!(!evaluate(&Scope::Scoped(p), &hops(&e), 50, &OK).is_allow());
    }

    #[test]
    fn empty_jurisdiction_never_valid() {
        let mut e = evs();
        e[0].jurisdiction = Jurisdiction(String::new());
        let mut p = policy();
        p = p.allow(Plane::Storage, &[""], &[]);
        assert!(!evaluate(&Scope::Scoped(p), &hops(&e), 50, &OK).is_allow());
    }

    #[test]
    fn empty_resource_allowlist_denies() {
        let e = vec![ev(Plane::Storage, "r")];
        let p = Policy::new(["root".to_string()]).allow(Plane::Storage, &["eu"], &[]);
        let mut p = p;
        p.required = [Plane::Storage].into();
        assert!(!evaluate(&Scope::Scoped(p), &hops(&e), 50, &OK).is_allow());
    }

    #[test]
    fn reevaluation_after_expiry_denies_retry() {
        let e = evs();
        assert!(run(&e, 10, &OK).is_allow());
        assert!(!run(&e, 200, &OK).is_allow());
    }
}
