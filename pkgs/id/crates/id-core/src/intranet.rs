//! Intranet-mode boundary: network-identity gating, 2FA step-up
//! windows, org-escrowed credential proxying and break-glass
//! (#5683 design step 5, issue #7786).
//!
//! Design: `doc/2026-11-08_design_id5683.md` §6 (with §2.4's shared
//! adapter rule and §3.4's IP-only evidence bar). Steps 2–4 of the
//! design built the declaration record ([`crate::boundary`]), the
//! surface adapter ([`crate::boundary_surfaces`]) and the residency
//! evaluator ([`crate::residency`]); this module is the network
//! capability they left unimplemented. It holds no storage and no
//! I/O: declarations, evidence and state are values, and every
//! decision is a pure function of them plus the caller's clock.
//!
//! The rules the design fixes, and where they live here:
//!
//! - **Mechanisms are declared, never inferred** (§6.1). An
//!   [`IntranetDeclaration`] names which [`NetworkMechanism`]s
//!   satisfy its boundary and which are required at its listed
//!   enforcement points. Nothing is selected or upgraded solely
//!   because it is available.
//! - **The network predicate is typed, and renders through the
//!   shared adapter** (§6.2, §2.4). [`evaluate_network`] yields a
//!   [`PredicateResult`]; [`adapt`] maps it with the configured
//!   failure behaviour to the one [`FrameworkOutcome`] every surface
//!   renders. The predicate is *not* translated into a residency
//!   placement hop; the outcome's reason classes name the transport
//!   plane, which is where a connection lives.
//! - **IP-only evidence can never deny** (§3.4, #5883). When the
//!   only evidence offered is a source IP, an adverse predicate
//!   result is restricted to `Warn` or a reversible `StepUp`,
//!   whatever the configured failure behaviour says.
//! - **Step-up is explicit** (§6.3). The declaration enumerates the
//!   events that require step-up and states the window duration; no
//!   event or duration is implied by enabling the boundary.
//!   [`StepUpState`] tracks windows and first-seen boundary
//!   contexts; completing a step-up is logged.
//! - **Escrow proxies assertions, never values** (§6.4). An
//!   [`Assertion`] carries no raw value field, and neither does its
//!   audit record: claim kind, assurance level, issuer, recipient,
//!   declaration version and an opaque reference only. Missing
//!   consent configuration authorizes no proxy use at all.
//! - **Degraded behaviour is chosen, never inferred** (§6.5). A
//!   declaration with no degraded-mode outcome is invalid
//!   ([`IntranetError::MissingDegradedOutcome`]). Break-glass is a
//!   separate opt-in capability with a named role; its only entry
//!   point is the explicit human action [`BreakGlass::use_by`], and
//!   every use is logged loudly. It is never an automatic fallback:
//!   [`gate_generic_activity`] allows past a fail-closed boundary
//!   only when handed a completed break-glass record.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::boundary::{AssuranceLevel, DeclarationId, EnforcementPoint, FailureBehaviour};
use crate::boundary_surfaces::{DecisionLogEntry, EvidenceClass, FrameworkOutcome};
use crate::residency::{DenyReason, Plane};

/// One way a connection can prove it is inside the network boundary
/// (§6.1). Ordered by the design's strength table: source IP is
/// weak (spoofable at the edge), overlay membership is medium, mTLS
/// and IdP assertions are strong (cryptographic identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMechanism {
    /// Source IP within a declared range. Weak.
    SourceIp,
    /// VPN / overlay network membership. Medium.
    OverlayMembership,
    /// mTLS client identity. Strong.
    MtlsIdentity,
    /// Signed assertion from a trusted org IdP. Strong.
    IdpAssertion,
}

impl NetworkMechanism {
    /// The mechanism's stable name, as recorded in decision logs.
    pub const fn name(self) -> &'static str {
        match self {
            NetworkMechanism::SourceIp => "source_ip",
            NetworkMechanism::OverlayMembership => "overlay_membership",
            NetworkMechanism::MtlsIdentity => "mtls_identity",
            NetworkMechanism::IdpAssertion => "idp_assertion",
        }
    }

    /// Whether this mechanism is the weak, IP-only signal (§3.4).
    pub const fn is_ip(self) -> bool {
        matches!(self, NetworkMechanism::SourceIp)
    }
}

/// An org IdP assertion as network evidence: who issued it and how
/// strongly the identity was established. The assertion's contents
/// are not carried here — the boundary judges issuer and assurance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdpAssertionEvidence {
    /// The issuing IdP, matched against the declaration's issuers.
    pub issuer: String,
    /// How strongly the asserted identity was established.
    pub assurance: AssuranceLevel,
}

/// What a connection presents to the network boundary. Every field
/// is optional: an absent mechanism is evidence not offered, which
/// the predicate treats differently from evidence offered and
/// failing (§2.4's `Indeterminate` vs `Unsatisfied`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkEvidence {
    /// The connection's source address, when known.
    pub source_ip: Option<IpAddr>,
    /// The overlay peer identity, when the connection arrived over
    /// a declared overlay.
    pub overlay_peer: Option<String>,
    /// The mTLS client identity, when one was presented.
    pub mtls_identity: Option<String>,
    /// The org IdP assertion, when one was presented.
    pub idp_assertion: Option<IdpAssertionEvidence>,
    /// Whether the boundary itself could be evaluated (IdP up, VPN
    /// up, list source reachable). False is the degraded case
    /// (§6.5), decided by the declaration's degraded outcome, never
    /// by the predicate.
    pub boundary_reachable: bool,
}

impl NetworkEvidence {
    /// Whether IP is the only evidence offered. §3.4's bar keys on
    /// exactly this: the easiest signal to spoof may not drive the
    /// strongest action alone.
    pub fn ip_only(&self) -> bool {
        self.source_ip.is_some()
            && self.overlay_peer.is_none()
            && self.mtls_identity.is_none()
            && self.idp_assertion.is_none()
    }

    /// Whether any mechanism evidence was offered at all.
    pub fn any_offered(&self) -> bool {
        self.source_ip.is_some()
            || self.overlay_peer.is_some()
            || self.mtls_identity.is_some()
            || self.idp_assertion.is_some()
    }
}

/// The typed result of one boundary predicate (§2.4): satisfied,
/// unsatisfied with a reason, or indeterminate with a reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PredicateResult {
    /// The boundary is satisfied, by this mechanism.
    Satisfied {
        /// The mechanism that satisfied the boundary.
        mechanism: NetworkMechanism,
    },
    /// Evidence was offered and did not satisfy the boundary.
    Unsatisfied {
        /// Why not, as a reason class — never an evidence value.
        reason: String,
    },
    /// The boundary could not be judged on the evidence offered
    /// (none offered, or a required mechanism could not be checked).
    Indeterminate {
        /// Why not, as a reason class — never an evidence value.
        reason: String,
    },
}

/// An event the declaration may require step-up for (§6.3). The
/// declaration enumerates its events; nothing else ever steps up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepUpEvent {
    /// Entering from a boundary context this subject has not used
    /// before.
    NewNetwork,
    /// Escalating privilege.
    PrivilegeEscalation,
    /// An administrative action.
    AdminAction,
}

impl StepUpEvent {
    /// The event's stable name, as recorded in step-up logs.
    pub const fn name(self) -> &'static str {
        match self {
            StepUpEvent::NewNetwork => "new_network",
            StepUpEvent::PrivilegeEscalation => "privilege_escalation",
            StepUpEvent::AdminAction => "admin_action",
        }
    }
}

/// The declaration's step-up configuration (§6.3): the enumerated
/// events, and the window a completed step-up grants. A zero
/// duration with a non-empty event set is invalid — a duration must
/// be stated, never implied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepUpPolicy {
    /// Events that require step-up. Empty means no step-up, ever.
    pub events: BTreeSet<StepUpEvent>,
    /// Seconds a completed step-up stays live.
    pub window_seconds: u64,
}

/// What generic activity does when the boundary cannot be
/// evaluated (§6.5). The declaration must choose one; there is no
/// default and no inference from enabling intranet mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedOutcome {
    /// Generic activity is refused while the boundary is down.
    FailClosed,
    /// Generic activity proceeds while the boundary is down.
    FailOpen,
    /// Generic activity proceeds only after a step-up.
    RequireStepUp,
}

/// The consent model for escrowed-credential proxying (§6.4, §11):
/// explicitly selected before any credential is escrowed or any
/// assertion proxied. Absence is not a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentModel {
    /// The user approves each proxy use.
    PerUse,
    /// The user granted standing consent.
    Standing,
    /// The org's authority alone authorizes proxy use.
    OrgAuthority,
}

/// One explicit privacy rule (§6.4): a named reader may be shown the
/// exact value behind assertions of one claim kind. No rule, no
/// value — for anyone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyRule {
    /// The claim kind the rule covers.
    pub claim_kind: String,
    /// The only reader the value may be shown to.
    pub reader: String,
}

/// The declaration's escrow configuration (§6.4).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowPolicy {
    /// The selected consent model. `None` authorizes no proxy use.
    pub consent: Option<ConsentModel>,
    /// Explicit value-disclosure rules. Empty discloses nothing.
    pub privacy: Vec<PrivacyRule>,
}

impl EscrowPolicy {
    /// Whether the declaration's privacy rules authorize showing a
    /// value of `claim_kind` to `reader` (§6.4).
    pub fn value_disclosure_authorized(&self, claim_kind: &str, reader: &str) -> bool {
        self.privacy
            .iter()
            .any(|r| r.claim_kind == claim_kind && r.reader == reader)
    }
}

/// The break-glass capability's declaration (§6.5): separately
/// opted in, with the named role allowed to use it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakGlassPolicy {
    /// The role permitted to break glass, named by the declaration.
    pub role: String,
}

/// The intranet-mode declaration (§6): the network-boundary half of
/// a `CapabilityKind::Network` boundary declaration, carrying the
/// step-5 configuration the generic declaration record has no
/// fields for. `boundary` and `version` tie every decision and
/// audit record back to the boundary declaration in force.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntranetDeclaration {
    /// The boundary declaration this configures.
    pub boundary: DeclarationId,
    /// The boundary declaration's version.
    pub version: u64,
    /// Mechanisms any one of which satisfies the boundary (§6.1).
    pub satisfying: BTreeSet<NetworkMechanism>,
    /// Enforcement points at which `required` replaces `satisfying`.
    pub required_points: BTreeSet<EnforcementPoint>,
    /// Mechanisms all of which are required at `required_points`.
    pub required: BTreeSet<NetworkMechanism>,
    /// Declared source ranges, CIDR notation (v4 or v6).
    pub ip_ranges: Vec<String>,
    /// Declared overlay peer identities.
    pub overlay_peers: BTreeSet<String>,
    /// Declared mTLS client identities.
    pub mtls_identities: BTreeSet<String>,
    /// Trusted IdP issuers for assertion evidence.
    pub idp_issuers: BTreeSet<String>,
    /// Minimum assurance an IdP assertion must carry.
    pub idp_min_assurance: AssuranceLevel,
    /// What a failed predicate does (the boundary's configured
    /// failure behaviour, §2.4).
    pub failure: FailureBehaviour,
    /// Step-up configuration (§6.3).
    pub step_up: StepUpPolicy,
    /// The degraded-mode outcome (§6.5). `None` is invalid.
    pub degraded: Option<DegradedOutcome>,
    /// Escrow configuration (§6.4).
    pub escrow: EscrowPolicy,
    /// Break-glass capability (§6.5). `None` is no break-glass.
    pub break_glass: Option<BreakGlassPolicy>,
}

/// Why an intranet declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntranetError {
    /// §6.5: no degraded-mode outcome was selected, and none may be
    /// inferred — the declaration is invalid.
    #[error("intranet declaration selects no degraded-mode outcome")]
    MissingDegradedOutcome,
    /// §6.1: no mechanism satisfies the boundary, so enabling it
    /// would promise a boundary nothing can cross legitimately.
    #[error("intranet declaration names no satisfying mechanism")]
    NoSatisfyingMechanism,
    /// §6.1: a point is listed as requiring mechanisms, but the
    /// required set is empty.
    #[error("intranet declaration lists required points with an empty required set")]
    RequiredPointWithoutMechanisms,
    /// §6.3: step-up events are enumerated but no window duration
    /// is stated.
    #[error("intranet declaration enumerates step-up events without a window duration")]
    StepUpEventWithoutDuration,
    /// A declared identity set entry is empty.
    #[error("intranet declaration {0} is empty")]
    EmptyIdentity(&'static str),
}

impl IntranetDeclaration {
    /// Validate the declaration against §6's structural rules. A
    /// declaration that fails is invalid and must not be evaluated.
    pub fn validate(&self) -> Result<(), IntranetError> {
        if self.degraded.is_none() {
            return Err(IntranetError::MissingDegradedOutcome);
        }
        if self.satisfying.is_empty() {
            return Err(IntranetError::NoSatisfyingMechanism);
        }
        if !self.required_points.is_empty() && self.required.is_empty() {
            return Err(IntranetError::RequiredPointWithoutMechanisms);
        }
        if !self.step_up.events.is_empty() && self.step_up.window_seconds == 0 {
            return Err(IntranetError::StepUpEventWithoutDuration);
        }
        if self.ip_ranges.iter().any(|r| r.trim().is_empty()) {
            return Err(IntranetError::EmptyIdentity("ip range"));
        }
        if self.overlay_peers.iter().any(|p| p.is_empty()) {
            return Err(IntranetError::EmptyIdentity("overlay peer"));
        }
        if self.mtls_identities.iter().any(|i| i.is_empty()) {
            return Err(IntranetError::EmptyIdentity("mTLS identity"));
        }
        if self.idp_issuers.iter().any(|i| i.is_empty()) {
            return Err(IntranetError::EmptyIdentity("IdP issuer"));
        }
        Ok(())
    }

    /// Whether `mechanism` is satisfied by `evidence` under this
    /// declaration's declared sets.
    fn mechanism_satisfied(&self, mechanism: NetworkMechanism, evidence: &NetworkEvidence) -> bool {
        match mechanism {
            NetworkMechanism::SourceIp => evidence
                .source_ip
                .is_some_and(|ip| self.ip_ranges.iter().any(|cidr| ip_in_cidr(ip, cidr))),
            NetworkMechanism::OverlayMembership => evidence
                .overlay_peer
                .as_ref()
                .is_some_and(|peer| self.overlay_peers.contains(peer)),
            NetworkMechanism::MtlsIdentity => evidence
                .mtls_identity
                .as_ref()
                .is_some_and(|id| self.mtls_identities.contains(id)),
            NetworkMechanism::IdpAssertion => evidence.idp_assertion.as_ref().is_some_and(|a| {
                self.idp_issuers.contains(&a.issuer) && a.assurance >= self.idp_min_assurance
            }),
        }
    }
}

/// Whether `ip` falls inside the CIDR range `cidr` ("addr/prefix",
/// either family; a bare address is a host route). Unparseable
/// ranges match nothing — a malformed declaration entry must never
/// widen the boundary by accident.
pub fn ip_in_cidr(ip: IpAddr, cidr: &str) -> bool {
    let (base, prefix) = match cidr.split_once('/') {
        Some((base, prefix)) => {
            let Ok(prefix) = prefix.trim().parse::<u32>() else {
                return false;
            };
            (base.trim(), Some(prefix))
        }
        None => (cidr.trim(), None),
    };
    let Ok(base) = base.parse::<IpAddr>() else {
        return false;
    };
    match (ip, base) {
        (IpAddr::V4(ip), IpAddr::V4(base)) => {
            let prefix = prefix.unwrap_or(32);
            if prefix > 32 {
                return false;
            }
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(ip) & mask == u32::from(base) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(base)) => {
            let prefix = prefix.unwrap_or(128);
            if prefix > 128 {
                return false;
            }
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(ip) & mask == u128::from(base) & mask
        }
        _ => false,
    }
}

/// Evaluate the network-identity predicate for one connection at
/// one enforcement point (§6.2): the typed result the shared
/// adapter renders. At a point the declaration lists as required,
/// every required mechanism must be satisfied; elsewhere any one
/// satisfying mechanism suffices. Evidence offered and failing is
/// `Unsatisfied`; no usable evidence is `Indeterminate`.
pub fn evaluate_network(
    declaration: &IntranetDeclaration,
    point: EnforcementPoint,
    evidence: &NetworkEvidence,
) -> PredicateResult {
    let required_here = declaration.required_points.contains(&point);
    let mechanisms: Vec<NetworkMechanism> = if required_here {
        declaration.required.iter().copied().collect()
    } else {
        declaration.satisfying.iter().copied().collect()
    };
    if required_here {
        for mechanism in &mechanisms {
            if declaration.mechanism_satisfied(*mechanism, evidence) {
                continue;
            }
            return if evidence.any_offered() {
                PredicateResult::Unsatisfied {
                    reason: format!("required mechanism {} not satisfied", mechanism.name()),
                }
            } else {
                PredicateResult::Indeterminate {
                    reason: format!("required mechanism {} not offered", mechanism.name()),
                }
            };
        }
        // The required set is validated non-empty at these points;
        // name the strongest satisfied mechanism for the log.
        let strongest = mechanisms
            .iter()
            .copied()
            .max()
            .unwrap_or(NetworkMechanism::SourceIp);
        return PredicateResult::Satisfied {
            mechanism: strongest,
        };
    }
    for mechanism in &mechanisms {
        if declaration.mechanism_satisfied(*mechanism, evidence) {
            return PredicateResult::Satisfied {
                mechanism: *mechanism,
            };
        }
    }
    if evidence.any_offered() {
        PredicateResult::Unsatisfied {
            reason: "no declared mechanism satisfied".to_string(),
        }
    } else {
        PredicateResult::Indeterminate {
            reason: "no network evidence offered".to_string(),
        }
    }
}

/// The shared adapter for network predicates (§2.4): map a
/// predicate result and the configured failure behaviour to the one
/// framework outcome, under §3.4's bar — an IP-only adverse result
/// can never become a final `Deny` (it is restricted to `Warn`) or
/// a `Quarantine` (restricted to a reversible `StepUp`), whatever
/// the configured behaviour says.
pub fn adapt(
    result: &PredicateResult,
    failure: FailureBehaviour,
    ip_only: bool,
) -> FrameworkOutcome {
    let placement = vec![match result {
        PredicateResult::Satisfied { .. } => return FrameworkOutcome::Allow,
        PredicateResult::Unsatisfied { .. } => DenyReason::OutOfPolicy(Plane::Transport),
        PredicateResult::Indeterminate { .. } => DenyReason::NoEvidence(Plane::Transport),
    }];
    let behaviour = match (failure, ip_only) {
        (FailureBehaviour::Deny, true) => FailureBehaviour::Warn,
        (FailureBehaviour::Quarantine, true) => FailureBehaviour::StepUp,
        (behaviour, _) => behaviour,
    };
    match behaviour {
        FailureBehaviour::Warn => FrameworkOutcome::Warn {
            placement,
            assurance: Vec::new(),
        },
        FailureBehaviour::StepUp => FrameworkOutcome::StepUp {
            placement,
            assurance: Vec::new(),
        },
        FailureBehaviour::Quarantine => FrameworkOutcome::Quarantine {
            placement,
            assurance: Vec::new(),
        },
        FailureBehaviour::Deny => FrameworkOutcome::Deny {
            placement,
            assurance: Vec::new(),
        },
    }
}

/// The result of gating one generic activity: the outcome to
/// render, and the decision-log entry to persist (§8: every
/// non-plain-allow decision is logged; break-glass and degraded
/// allows are logged too, loudly, because they are the exceptions
/// an admin must be able to find).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkCheck {
    /// The one framework outcome for this check.
    pub outcome: FrameworkOutcome,
    /// The decision-log entry, when this decision must be logged.
    pub log_entry: Option<DecisionLogEntry>,
    /// The predicate result behind the outcome, for callers that
    /// render explanations (§10's appeal path names it).
    pub predicate: PredicateResult,
}

/// Gate one generic (ordinary user) activity at a point (§6.2,
/// §6.5): evaluate the network predicate and render it through the
/// shared adapter; when the boundary itself is unreachable, apply
/// the declaration's degraded outcome instead — fail-closed refuses
/// unless a completed break-glass record is handed in, and no other
/// input can turn a degraded refusal into an allow. A satisfied
/// predicate still yields `StepUp` when the caller names a
/// configured step-up trigger whose window is not live (§2.4,
/// §6.3).
pub fn gate_generic_activity(
    declaration: &IntranetDeclaration,
    scope: &crate::boundary::ScopeRef,
    actor: &crate::boundary::ActorId,
    point: EnforcementPoint,
    evidence: &NetworkEvidence,
    step_up_trigger: Option<StepUpEvent>,
    step_up_live: bool,
    break_glass: Option<&BreakGlassUse>,
    now: u64,
) -> NetworkCheck {
    let log =
        |outcome: &FrameworkOutcome, predicate: &PredicateResult, classes: Vec<EvidenceClass>| {
            Some(DecisionLogEntry {
                surface: "intranet".to_string(),
                scope: scope.clone(),
                boundary_version: declaration.version,
                evidence_classes: classes,
                minimum_assurance: None,
                outcome: outcome.name().to_string(),
                reasons: match predicate {
                    PredicateResult::Satisfied { mechanism } => {
                        vec![format!("satisfied:{}", mechanism.name())]
                    }
                    PredicateResult::Unsatisfied { reason }
                    | PredicateResult::Indeterminate { reason } => vec![reason.clone()],
                },
                actor: actor.clone(),
                at: now,
            })
        };
    let classes = evidence_classes(evidence);

    if !evidence.boundary_reachable {
        let predicate = PredicateResult::Indeterminate {
            reason: "boundary unreachable".to_string(),
        };
        let outcome = match declaration.degraded {
            Some(DegradedOutcome::FailOpen) => FrameworkOutcome::Allow,
            Some(DegradedOutcome::RequireStepUp) => FrameworkOutcome::StepUp {
                placement: vec![DenyReason::NoEvidence(Plane::Transport)],
                assurance: Vec::new(),
            },
            // Missing outcome is invalid at declaration time; if an
            // unvalidated declaration reaches the gate anyway, the
            // gate fails closed rather than inventing an outcome.
            Some(DegradedOutcome::FailClosed) | None => match break_glass {
                Some(_) => FrameworkOutcome::Allow,
                None => FrameworkOutcome::Deny {
                    placement: vec![DenyReason::NoEvidence(Plane::Transport)],
                    assurance: Vec::new(),
                },
            },
        };
        // Degraded decisions are always logged: fail-open and
        // break-glass allows are exactly the exceptions §6.5 wants
        // surfaced, and refusals log by the §8 rule.
        return NetworkCheck {
            log_entry: log(&outcome, &predicate, classes),
            outcome,
            predicate,
        };
    }

    let predicate = evaluate_network(declaration, point, evidence);
    let mut outcome = adapt(&predicate, declaration.failure, evidence.ip_only());
    if matches!(predicate, PredicateResult::Satisfied { .. })
        && !step_up_live
        && step_up_trigger.is_some_and(|e| declaration.step_up.events.contains(&e))
    {
        outcome = FrameworkOutcome::StepUp {
            placement: Vec::new(),
            assurance: Vec::new(),
        };
    }
    let log_entry = if matches!(outcome, FrameworkOutcome::Allow) {
        None
    } else {
        log(&outcome, &predicate, classes)
    };
    NetworkCheck {
        outcome,
        log_entry,
        predicate,
    }
}

/// The evidence classes a decision consulted, for the decision log:
/// one per offered mechanism, naming the transport plane, the
/// mechanism as the source class and the assurance it carried. No
/// raw values — an address, peer id or identity never appears.
fn evidence_classes(evidence: &NetworkEvidence) -> Vec<EvidenceClass> {
    let mut classes = Vec::new();
    let mut push = |mechanism: NetworkMechanism, assurance: AssuranceLevel| {
        classes.push(EvidenceClass {
            plane: "transport".to_string(),
            source: mechanism.name().to_string(),
            assurance,
        });
    };
    if evidence.source_ip.is_some() {
        push(NetworkMechanism::SourceIp, AssuranceLevel::SelfDeclared);
    }
    if evidence.overlay_peer.is_some() {
        push(
            NetworkMechanism::OverlayMembership,
            AssuranceLevel::SelfDeclared,
        );
    }
    if evidence.mtls_identity.is_some() {
        push(
            NetworkMechanism::MtlsIdentity,
            AssuranceLevel::ProviderVerified,
        );
    }
    if let Some(assertion) = &evidence.idp_assertion {
        push(NetworkMechanism::IdpAssertion, assertion.assurance);
    }
    classes.sort();
    classes.dedup();
    classes
}

/// One completed step-up, as logged (§6.3: step-ups are logged —
/// the event, the window it granted, and the declaration version).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepUpLogEntry {
    /// The subject that stepped up.
    pub subject: String,
    /// The event that required the step-up.
    pub event: StepUpEvent,
    /// Unix seconds when the step-up completed.
    pub at: u64,
    /// Unix seconds when the granted window expires.
    pub expires_at: u64,
    /// The boundary declaration version in force.
    pub declaration_version: u64,
}

/// Live step-up state (§6.3): granted windows per subject and
/// event, and the boundary contexts each subject has been seen
/// from (which is what makes a network *new*). In-memory only; a
/// restart forgets windows, which fails toward stepping up again,
/// never toward allowing.
#[derive(Clone, Debug, Default)]
pub struct StepUpState {
    windows: BTreeMap<(String, StepUpEvent), u64>,
    seen_contexts: BTreeMap<String, BTreeSet<String>>,
}

impl StepUpState {
    /// Whether `event` currently requires a step-up for `subject`
    /// under `policy`: the event must be declared, and no live
    /// window may cover it. For `NewNetwork`, the step-up is
    /// required only when `context` is one the subject has not
    /// been seen from; other events ignore the context.
    pub fn step_up_required(
        &self,
        policy: &StepUpPolicy,
        subject: &str,
        event: StepUpEvent,
        context: Option<&str>,
        now: u64,
    ) -> bool {
        if !policy.events.contains(&event) {
            return false;
        }
        if event == StepUpEvent::NewNetwork {
            match context {
                Some(context) => {
                    if self
                        .seen_contexts
                        .get(subject)
                        .is_some_and(|seen| seen.contains(context))
                    {
                        return false;
                    }
                }
                None => return false,
            }
        }
        !self.window_live(subject, event, now)
    }

    /// Whether a live (unexpired) window covers this subject and
    /// event at `now`.
    pub fn window_live(&self, subject: &str, event: StepUpEvent, now: u64) -> bool {
        self.windows
            .get(&(subject.to_string(), event))
            .is_some_and(|expires| *expires > now)
    }

    /// Record a completed step-up: grant the declared window,
    /// remember the boundary context, and return the log entry.
    /// Returns `None` — and grants nothing — for an event the
    /// declaration does not enumerate: step-up is never implied.
    pub fn complete_step_up(
        &mut self,
        policy: &StepUpPolicy,
        declaration_version: u64,
        subject: &str,
        event: StepUpEvent,
        context: Option<&str>,
        now: u64,
    ) -> Option<StepUpLogEntry> {
        if !policy.events.contains(&event) {
            return None;
        }
        let expires_at = now + policy.window_seconds;
        self.windows
            .insert((subject.to_string(), event), expires_at);
        if let Some(context) = context {
            self.seen_contexts
                .entry(subject.to_string())
                .or_default()
                .insert(context.to_string());
        }
        Some(StepUpLogEntry {
            subject: subject.to_string(),
            event,
            at: now,
            expires_at,
            declaration_version,
        })
    }
}

/// One assertion the org proxies on a user's behalf (§6.4): a claim
/// kind, graded and attributed — never a raw value. There is no
/// value field; that absence is the privacy rule's enforcement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assertion {
    /// The user the assertion is about.
    pub subject: String,
    /// The claim kind asserted ("member of group G", "residency
    /// attested", "identity verified").
    pub claim_kind: String,
    /// How strongly the claim was established.
    pub assurance: AssuranceLevel,
    /// Who issued the assertion.
    pub issuer: String,
    /// Who the assertion is proxied to.
    pub recipient: String,
    /// An opaque reference to the escrowed credential. Opaque: it
    /// identifies the escrow record and reveals nothing.
    pub opaque_ref: String,
}

/// The audit record of one proxy use (§6.4): claim kind, assurance
/// level, issuer, recipient, declaration version, opaque reference,
/// subject and time. No raw values — no field can hold one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowAuditEntry {
    /// The user the assertion was about (their visibility view).
    pub subject: String,
    /// The claim kind shared.
    pub claim_kind: String,
    /// The assurance level shared.
    pub assurance: AssuranceLevel,
    /// Who issued the assertion.
    pub issuer: String,
    /// Who received it.
    pub recipient: String,
    /// The boundary declaration version in force.
    pub declaration_version: u64,
    /// The opaque assertion reference.
    pub opaque_ref: String,
    /// Unix seconds when the proxy use happened.
    pub at: u64,
}

/// Why a proxy use was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EscrowError {
    /// §6.4: no consent model is configured, so no proxy use is
    /// authorized — at all, for anyone.
    #[error("escrow consent is not configured; no proxy use is authorized")]
    NoConsentConfigured,
    /// Per-use consent was selected and this use was not approved.
    #[error("per-use consent was not given for this proxy use")]
    ConsentRequired,
    /// The user (or the org) revoked: the assertion is gone and the
    /// server must re-evaluate without it.
    #[error("escrow was revoked for this subject")]
    Revoked,
}

/// The escrow's live state (§6.4): the audit trail of proxy uses
/// and the set of subjects whose escrow has been revoked.
#[derive(Clone, Debug, Default)]
pub struct Escrow {
    audit: Vec<EscrowAuditEntry>,
    revoked: BTreeSet<String>,
}

impl Escrow {
    /// Proxy one assertion to its recipient under the declaration's
    /// escrow policy, recording the audit entry the design
    /// requires. Refuses — and records nothing — when no consent
    /// model is configured, when per-use consent was not given for
    /// this use, or when the subject's escrow has been revoked.
    pub fn proxy_assertion(
        &mut self,
        policy: &EscrowPolicy,
        declaration_version: u64,
        assertion: &Assertion,
        per_use_approved: bool,
        now: u64,
    ) -> Result<EscrowAuditEntry, EscrowError> {
        let consent = policy.consent.ok_or(EscrowError::NoConsentConfigured)?;
        if consent == ConsentModel::PerUse && !per_use_approved {
            return Err(EscrowError::ConsentRequired);
        }
        if self.revoked.contains(&assertion.subject) {
            return Err(EscrowError::Revoked);
        }
        let entry = EscrowAuditEntry {
            subject: assertion.subject.clone(),
            claim_kind: assertion.claim_kind.clone(),
            assurance: assertion.assurance,
            issuer: assertion.issuer.clone(),
            recipient: assertion.recipient.clone(),
            declaration_version,
            opaque_ref: assertion.opaque_ref.clone(),
            at: now,
        };
        self.audit.push(entry.clone());
        Ok(entry)
    }

    /// Revoke a subject's escrow (§6.4 un-escrow): later proxy uses
    /// for the subject are refused, so the server loses the
    /// assertion and re-evaluates.
    pub fn revoke(&mut self, subject: &str) {
        self.revoked.insert(subject.to_string());
    }

    /// Whether this subject's escrow has been revoked.
    pub fn is_revoked(&self, subject: &str) -> bool {
        self.revoked.contains(subject)
    }

    /// The audit entries about one subject — the user's visibility
    /// view (§6.4): which claim kinds and assurance levels were
    /// shared, with whom, and when.
    pub fn audit_for(&self, subject: &str) -> Vec<&EscrowAuditEntry> {
        self.audit.iter().filter(|e| e.subject == subject).collect()
    }

    /// Every audit entry, in the order the uses happened.
    pub fn audit(&self) -> &[EscrowAuditEntry] {
        &self.audit
    }
}

/// A completed break-glass use (§6.5): the record a gate accepts
/// as proof a human broke glass. Produced only by
/// [`BreakGlass::use_by`]; there is no other constructor path a
/// caller can take, and no automatic fallback produces one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakGlassUse {
    /// Who broke glass.
    pub actor: String,
    /// The named role they acted in.
    pub role: String,
    /// Unix seconds when they did it.
    pub at: u64,
    /// The boundary declaration version in force.
    pub declaration_version: u64,
}

/// One break-glass use, as logged (§6.5: logged loudly and
/// surfaced to admins — `loud` is always true and is part of the
/// record, so a quiet break-glass entry cannot be represented).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakGlassLogEntry {
    /// Who broke glass.
    pub actor: String,
    /// The named role they acted in.
    pub role: String,
    /// Unix seconds when they did it.
    pub at: u64,
    /// The boundary declaration version in force.
    pub declaration_version: u64,
    /// Always true: break-glass use is surfaced to admins.
    pub loud: bool,
}

/// Why a break-glass use was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BreakGlassError {
    /// The declaration does not enable a break-glass capability.
    #[error("break-glass is not enabled by this declaration")]
    NotEnabled,
    /// The actor does not hold the declaration's named role.
    #[error("actor does not hold the break-glass role")]
    WrongRole,
}

/// The break-glass capability in force for one declaration
/// (§6.5). Stateless: the policy comes from the declaration, and a
/// use produces both the gate record and its loud log entry.
pub struct BreakGlass;

impl BreakGlass {
    /// A human breaks glass: check the declaration enables it and
    /// the actor holds its named role, then produce the use record
    /// and its loud log entry together — a use that is not logged
    /// cannot be represented, and a log entry without a use cannot
    /// be either.
    pub fn use_by(
        declaration: &IntranetDeclaration,
        actor: &str,
        role: &str,
        now: u64,
    ) -> Result<(BreakGlassUse, BreakGlassLogEntry), BreakGlassError> {
        let policy = declaration
            .break_glass
            .as_ref()
            .ok_or(BreakGlassError::NotEnabled)?;
        if policy.role != role {
            return Err(BreakGlassError::WrongRole);
        }
        let used = BreakGlassUse {
            actor: actor.to_string(),
            role: role.to_string(),
            at: now,
            declaration_version: declaration.version,
        };
        let log = BreakGlassLogEntry {
            actor: actor.to_string(),
            role: role.to_string(),
            at: now,
            declaration_version: declaration.version,
            loud: true,
        };
        Ok((used, log))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::boundary::{
        ActorId, BoundaryDeclaration, BoundaryDeclarationBody, CapabilityKind, EvidenceRequirement,
        ScopeRef, SubjectClass,
    };
    use proptest::prelude::*;

    fn declaration() -> IntranetDeclaration {
        let boundary = BoundaryDeclaration::new(BoundaryDeclarationBody {
            capability: CapabilityKind::Network,
            scope: ScopeRef::Server("srv".to_string()),
            subject_class: SubjectClass::Server,
            evidence_req: EvidenceRequirement {
                minimum_assurance: AssuranceLevel::SelfDeclared,
                accepted_sources: BTreeSet::new(),
            },
            enforcement: [EnforcementPoint::Login].into_iter().collect(),
            failure: FailureBehaviour::Deny,
            data_ref: None,
            enabled: true,
            version: 3,
            declared_by: ActorId("admin".to_string()),
            declared_at: 1,
        })
        .expect("valid boundary declaration");
        IntranetDeclaration {
            boundary: boundary.id,
            version: 3,
            satisfying: [
                NetworkMechanism::OverlayMembership,
                NetworkMechanism::SourceIp,
            ]
            .into_iter()
            .collect(),
            required_points: BTreeSet::new(),
            required: BTreeSet::new(),
            ip_ranges: vec!["10.0.0.0/8".to_string()],
            overlay_peers: ["peer-alice".to_string()].into_iter().collect(),
            mtls_identities: BTreeSet::new(),
            idp_issuers: ["idp.example".to_string()].into_iter().collect(),
            idp_min_assurance: AssuranceLevel::OrgAttested,
            failure: FailureBehaviour::Deny,
            step_up: StepUpPolicy {
                events: [StepUpEvent::AdminAction].into_iter().collect(),
                window_seconds: 600,
            },
            degraded: Some(DegradedOutcome::FailClosed),
            escrow: EscrowPolicy {
                consent: Some(ConsentModel::Standing),
                privacy: Vec::new(),
            },
            break_glass: Some(BreakGlassPolicy {
                role: "oncall-admin".to_string(),
            }),
        }
    }

    fn reachable(ev: NetworkEvidence) -> NetworkEvidence {
        NetworkEvidence {
            boundary_reachable: true,
            ..ev
        }
    }

    fn ip_evidence(ip: &str) -> NetworkEvidence {
        reachable(NetworkEvidence {
            source_ip: Some(ip.parse().expect("ip")),
            ..Default::default()
        })
    }

    #[test]
    fn declaration_missing_degraded_outcome_is_invalid() {
        // Oracle (§6.5): a declaration with no degraded-mode outcome
        // is rejected as invalid — no outcome is ever inferred.
        let mut decl = declaration();
        decl.degraded = None;
        assert_eq!(decl.validate(), Err(IntranetError::MissingDegradedOutcome));
        decl.degraded = Some(DegradedOutcome::FailOpen);
        assert_eq!(decl.validate(), Ok(()));
    }

    #[test]
    fn declaration_step_up_events_without_duration_is_invalid() {
        let mut decl = declaration();
        decl.step_up.window_seconds = 0;
        assert_eq!(
            decl.validate(),
            Err(IntranetError::StepUpEventWithoutDuration)
        );
    }

    #[test]
    fn predicate_satisfied_by_declared_mechanisms_only() {
        let decl = declaration();
        let inside = evaluate_network(
            &decl,
            EnforcementPoint::Login,
            &reachable(NetworkEvidence {
                overlay_peer: Some("peer-alice".to_string()),
                ..Default::default()
            }),
        );
        assert_eq!(
            inside,
            PredicateResult::Satisfied {
                mechanism: NetworkMechanism::OverlayMembership
            }
        );
        // An undeclared peer offers evidence and fails: Unsatisfied,
        // not Indeterminate.
        let outside = evaluate_network(
            &decl,
            EnforcementPoint::Login,
            &reachable(NetworkEvidence {
                overlay_peer: Some("peer-mallory".to_string()),
                ..Default::default()
            }),
        );
        assert!(matches!(outside, PredicateResult::Unsatisfied { .. }));
        // Nothing offered: Indeterminate.
        let none = evaluate_network(
            &decl,
            EnforcementPoint::Login,
            &reachable(NetworkEvidence::default()),
        );
        assert!(matches!(none, PredicateResult::Indeterminate { .. }));
        // CIDR edges: inside, outside, malformed range matches nothing.
        assert!(ip_in_cidr("10.1.2.3".parse().unwrap(), "10.0.0.0/8"));
        assert!(!ip_in_cidr("11.1.2.3".parse().unwrap(), "10.0.0.0/8"));
        assert!(!ip_in_cidr("10.1.2.3".parse().unwrap(), "10.0.0.0/33"));
        assert!(!ip_in_cidr("10.1.2.3".parse().unwrap(), "not-a-range"));
    }

    #[test]
    fn ip_only_adverse_result_never_denies() {
        // Oracle (§3.4): IP-only Unsatisfied/Indeterminate can never
        // become a final Deny, whatever the configured behaviour.
        let decl = declaration();
        let check = gate_generic_activity(
            &decl,
            &ScopeRef::Server("srv".to_string()),
            &ActorId("alice".to_string()),
            EnforcementPoint::Login,
            &ip_evidence("192.0.2.9"),
            None,
            false,
            None,
            1_000,
        );
        assert_eq!(check.outcome.name(), "warn");
        assert!(check.log_entry.is_some(), "a warn decision is logged");
        // Quarantine is likewise restricted to a reversible step-up.
        let outcome = adapt(
            &PredicateResult::Unsatisfied {
                reason: "x".to_string(),
            },
            FailureBehaviour::Quarantine,
            true,
        );
        assert_eq!(outcome.name(), "step_up");
        // With a strong mechanism offered too, Deny stands.
        let outcome = adapt(
            &PredicateResult::Unsatisfied {
                reason: "x".to_string(),
            },
            FailureBehaviour::Deny,
            false,
        );
        assert_eq!(outcome.name(), "deny");
    }

    proptest! {
        /// Oracle (acceptance, §6.5): with the boundary unreachable
        /// and no break-glass, generic activity is never allowed —
        /// for any evidence and any degraded selection the
        /// declaration could carry, fail-closed refuses outright and
        /// the other selections never render a plain allow either
        /// (fail-open is excluded here by construction: it is the
        /// declaration's explicit opposite choice, tested below).
        #[test]
        fn unreachable_without_break_glass_never_allows(
            ip in proptest::option::of("[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}"),
            peer in proptest::option::of("[a-z]{1,12}"),
            degraded in prop_oneof![Just(DegradedOutcome::FailClosed), Just(DegradedOutcome::RequireStepUp)],
        ) {
            let mut decl = declaration();
            decl.degraded = Some(degraded);
            let evidence = NetworkEvidence {
                source_ip: ip.and_then(|s| s.parse().ok()),
                overlay_peer: peer,
                boundary_reachable: false,
                ..Default::default()
            };
            let check = gate_generic_activity(
                &decl,
                &ScopeRef::Server("srv".to_string()),
                &ActorId("alice".to_string()),
                EnforcementPoint::Login,
                &evidence,
                None,
                false,
                None,
                1_000,
            );
            prop_assert!(!check.outcome.is_allow(), "unreachable + no break-glass allowed: {:?}", check.outcome);
            prop_assert!(check.log_entry.is_some(), "degraded decisions are always logged");
        }
    }

    #[test]
    fn unreachable_fail_open_is_the_declarations_explicit_choice() {
        let mut decl = declaration();
        decl.degraded = Some(DegradedOutcome::FailOpen);
        let check = gate_generic_activity(
            &decl,
            &ScopeRef::Server("srv".to_string()),
            &ActorId("alice".to_string()),
            EnforcementPoint::Login,
            &NetworkEvidence::default(),
            None,
            false,
            None,
            1_000,
        );
        assert!(check.outcome.is_allow());
        assert!(check.log_entry.is_some(), "a degraded allow is logged");
    }

    #[test]
    fn break_glass_is_opt_in_roled_and_loud() {
        let decl = declaration();
        // Not the named role: refused.
        assert_eq!(
            BreakGlass::use_by(&decl, "mallory", "user", 10).unwrap_err(),
            BreakGlassError::WrongRole
        );
        // The named role: a use record and a loud log entry, together.
        let (used, log) =
            BreakGlass::use_by(&decl, "alice", "oncall-admin", 10).expect("roled use");
        assert!(log.loud, "break-glass use is logged loudly");
        assert_eq!(log.actor, "alice");
        // The record opens the fail-closed gate; nothing else does.
        let check = gate_generic_activity(
            &decl,
            &ScopeRef::Server("srv".to_string()),
            &ActorId("alice".to_string()),
            EnforcementPoint::Login,
            &NetworkEvidence::default(),
            None,
            false,
            Some(&used),
            1_000,
        );
        assert!(check.outcome.is_allow());
        // A declaration without the capability refuses outright.
        let mut decl = declaration();
        decl.break_glass = None;
        assert_eq!(
            BreakGlass::use_by(&decl, "alice", "oncall-admin", 10).unwrap_err(),
            BreakGlassError::NotEnabled
        );
    }

    #[test]
    fn step_up_window_expiry_forces_restep_up() {
        // Oracle (§6.3): after the window expires, the next
        // privileged action steps up again; step-ups are logged
        // with the declared duration only.
        let decl = declaration();
        let mut state = StepUpState::default();
        let subject = "alice";
        let event = StepUpEvent::AdminAction;
        assert!(state.step_up_required(&decl.step_up, subject, event, None, 0));
        let entry = state
            .complete_step_up(&decl.step_up, decl.version, subject, event, None, 0)
            .expect("declared event steps up");
        assert_eq!(entry.expires_at, 600, "the declared duration, stated");
        assert!(!state.step_up_required(&decl.step_up, subject, event, None, 599));
        assert!(
            state.step_up_required(&decl.step_up, subject, event, None, 600),
            "expiry forces re-step-up on the next privileged action"
        );
        // An undeclared event never steps up and never grants.
        assert!(!state.step_up_required(
            &decl.step_up,
            subject,
            StepUpEvent::PrivilegeEscalation,
            None,
            0
        ));
        assert!(
            state
                .complete_step_up(
                    &decl.step_up,
                    decl.version,
                    subject,
                    StepUpEvent::PrivilegeEscalation,
                    None,
                    0
                )
                .is_none()
        );
    }

    #[test]
    fn step_up_new_network_only_for_unseen_contexts() {
        let policy = StepUpPolicy {
            events: [StepUpEvent::NewNetwork].into_iter().collect(),
            window_seconds: 60,
        };
        let mut state = StepUpState::default();
        assert!(state.step_up_required(
            &policy,
            "alice",
            StepUpEvent::NewNetwork,
            Some("office"),
            0
        ));
        state
            .complete_step_up(
                &policy,
                1,
                "alice",
                StepUpEvent::NewNetwork,
                Some("office"),
                0,
            )
            .expect("declared");
        // Same context after the window: seen, so no step-up.
        assert!(!state.step_up_required(
            &policy,
            "alice",
            StepUpEvent::NewNetwork,
            Some("office"),
            3_600
        ));
        // A new context steps up again.
        assert!(state.step_up_required(
            &policy,
            "alice",
            StepUpEvent::NewNetwork,
            Some("cafe"),
            3_600
        ));
    }

    #[test]
    fn satisfied_predicate_with_live_trigger_still_steps_up() {
        // §2.4: a satisfied predicate can still yield StepUp at an
        // explicitly configured trigger.
        let decl = declaration();
        let check = gate_generic_activity(
            &decl,
            &ScopeRef::Server("srv".to_string()),
            &ActorId("alice".to_string()),
            EnforcementPoint::Login,
            &reachable(NetworkEvidence {
                overlay_peer: Some("peer-alice".to_string()),
                ..Default::default()
            }),
            Some(StepUpEvent::AdminAction),
            false,
            None,
            1_000,
        );
        assert_eq!(check.outcome.name(), "step_up");
    }

    fn assertion() -> Assertion {
        Assertion {
            subject: "alice".to_string(),
            claim_kind: "member of group G".to_string(),
            assurance: AssuranceLevel::OrgAttested,
            issuer: "org-idp".to_string(),
            recipient: "srv".to_string(),
            opaque_ref: "escrow-ref-1".to_string(),
        }
    }

    #[test]
    fn every_proxy_use_produces_an_audit_record_without_values() {
        // Oracle (§6.4): every proxy use produces an audit record
        // carrying claim kind, assurance, issuer, recipient,
        // declaration version and opaque reference — never values.
        let decl = declaration();
        let mut escrow = Escrow::default();
        let entry = escrow
            .proxy_assertion(&decl.escrow, decl.version, &assertion(), false, 42)
            .expect("standing consent authorizes");
        assert_eq!(entry.claim_kind, "member of group G");
        assert_eq!(entry.assurance, AssuranceLevel::OrgAttested);
        assert_eq!(entry.issuer, "org-idp");
        assert_eq!(entry.recipient, "srv");
        assert_eq!(entry.declaration_version, decl.version);
        assert_eq!(entry.opaque_ref, "escrow-ref-1");
        assert_eq!(
            escrow.audit_for("alice").len(),
            1,
            "the user can see the use"
        );
        let serialized = serde_json::to_string(&entry).expect("serializes");
        assert!(
            !serialized.contains("raw") && !serialized.contains("secret"),
            "audit carries no raw value field: {serialized}"
        );
    }

    #[test]
    fn missing_consent_configuration_authorizes_no_proxy_use() {
        // Oracle (§6.4): missing consent configuration means no
        // proxy use is authorized.
        let policy = EscrowPolicy {
            consent: None,
            privacy: Vec::new(),
        };
        let mut escrow = Escrow::default();
        assert_eq!(
            escrow.proxy_assertion(&policy, 1, &assertion(), true, 0),
            Err(EscrowError::NoConsentConfigured)
        );
        assert!(escrow.audit().is_empty(), "a refused use records nothing");
        // Per-use consent selected but not given for this use.
        let policy = EscrowPolicy {
            consent: Some(ConsentModel::PerUse),
            privacy: Vec::new(),
        };
        assert_eq!(
            escrow.proxy_assertion(&policy, 1, &assertion(), false, 0),
            Err(EscrowError::ConsentRequired)
        );
        assert!(
            escrow
                .proxy_assertion(&policy, 1, &assertion(), true, 0)
                .is_ok()
        );
    }

    #[test]
    fn revocation_ends_proxy_use() {
        // §6.4 un-escrow: after revocation the server loses the
        // assertion and must re-evaluate.
        let decl = declaration();
        let mut escrow = Escrow::default();
        assert!(
            escrow
                .proxy_assertion(&decl.escrow, decl.version, &assertion(), false, 0)
                .is_ok()
        );
        escrow.revoke("alice");
        assert_eq!(
            escrow.proxy_assertion(&decl.escrow, decl.version, &assertion(), false, 1),
            Err(EscrowError::Revoked)
        );
    }

    #[test]
    fn value_disclosure_needs_an_explicit_privacy_rule() {
        let mut policy = EscrowPolicy {
            consent: Some(ConsentModel::Standing),
            privacy: Vec::new(),
        };
        assert!(!policy.value_disclosure_authorized("member of group G", "auditor"));
        policy.privacy.push(PrivacyRule {
            claim_kind: "member of group G".to_string(),
            reader: "auditor".to_string(),
        });
        assert!(policy.value_disclosure_authorized("member of group G", "auditor"));
        assert!(!policy.value_disclosure_authorized("member of group G", "anyone-else"));
    }
}
