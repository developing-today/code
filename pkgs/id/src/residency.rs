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
    Storage,
    Processing,
    Transport,
    Metadata,
}

impl Plane {
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
    Unscoped,
    Scoped(Policy),
}

/// What one plane permits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanePolicy {
    pub jurisdictions: BTreeSet<Jurisdiction>,
    /// Permitted resources. Empty means no resource restriction beyond jurisdiction.
    pub resources: BTreeSet<ResourceId>,
}

/// Per-plane residency policy. A plane not listed in `planes` permits nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub planes: BTreeMap<Plane, PlanePolicy>,
    /// Planes a path must prove. Defaults to all four through [`Policy::new`].
    pub required: BTreeSet<Plane>,
    /// Issuers this operator trusts for evidence under this policy.
    pub trust_roots: BTreeSet<String>,
}

impl Policy {
    pub fn new(trust_roots: impl IntoIterator<Item = String>) -> Self {
        Self {
            planes: BTreeMap::new(),
            required: Plane::ALL.into_iter().collect(),
            trust_roots: trust_roots.into_iter().collect(),
        }
    }

    pub fn allow(mut self, plane: Plane, jurisdictions: &[&str], resources: &[&str]) -> Self {
        let p = self.planes.entry(plane).or_default();
        p.jurisdictions
            .extend(jurisdictions.iter().map(|j| Jurisdiction((*j).into())));
        p.resources
            .extend(resources.iter().map(|r| ResourceId((*r).into())));
        self
    }
}

/// An issuer's assertion that a resource sits in a jurisdiction for a plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub issuer: String,
    pub resource: ResourceId,
    pub plane: Plane,
    pub jurisdiction: Jurisdiction,
    pub source: String,
    /// Unix seconds; evidence is valid while `now < expires_at`.
    pub expires_at: u64,
    pub serial: String,
    pub signature: Vec<u8>,
}

/// Operator-side check of an assertion's signature and revocation state. Implementations decide
/// whether the issuer's record is trustworthy; the evaluator only enforces the outcome.
pub trait EvidenceVerifier {
    fn signature_valid(&self, evidence: &Evidence) -> bool;
    fn revoked(&self, evidence: &Evidence) -> bool;
}

/// One resource a path touches on one plane, and the evidence offered for it.
#[derive(Clone, Debug)]
pub struct Hop<'a> {
    pub plane: Plane,
    pub resource: ResourceId,
    pub evidence: Option<&'a Evidence>,
}

/// Why a path was denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    /// A required plane has no hop at all, so its placement is unknown.
    PlaneUnknown(Plane),
    NoEvidence(Plane),
    UntrustedIssuer(Plane),
    BadSignature(Plane),
    Revoked(Plane),
    Expired(Plane),
    /// Evidence is for another resource or plane than the hop uses.
    BindingMismatch(Plane),
    /// The plane or resource is not permitted by the policy.
    OutOfPolicy(Plane),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(Vec<DenyReason>),
}

impl Decision {
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
        || (!pp.resources.is_empty() && !pp.resources.contains(&hop.resource))
    {
        return Some(DenyReason::OutOfPolicy(plane));
    }
    None
}

#[cfg(test)]
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
            p = p.allow(pl, &["eu"], &[]);
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
        assert!(r
            .iter()
            .all(|x| matches!(x, DenyReason::BindingMismatch(_))));
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
    fn reevaluation_after_expiry_denies_retry() {
        let e = evs();
        assert!(run(&e, 10, &OK).is_allow());
        assert!(!run(&e, 200, &OK).is_allow());
    }
}
