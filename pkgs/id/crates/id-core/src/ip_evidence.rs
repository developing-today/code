//! Public IP as evidence (design `doc/2026-11-08_design_id5683.md` §4,
//! #5683 step 3, #7784).
//!
//! The public IP a connection arrives from is a *signal*, never an
//! identity. This module derives the only things the framework permits
//! from it, in increasing privacy (design §4.1): a country/region
//! verdict, a boundary verdict ([`PrivacyClass::VerdictOnly`]), and —
//! only when a scope explicitly enables it with a stated retention
//! window and purpose — the raw IP.
//!
//! The hard rules, enforced by construction and by the tests below:
//!
//! - **No weak signal removes access a subject already has** (design
//!   §3.4). The contradiction outcome type has no deny variant at all:
//!   IP evidence can warn or ask for a reversible step-up, and it can
//!   fail to *satisfy* a scoped policy for new access, but it can
//!   never produce a final deny or quarantine.
//! - **Relay/bridge paths make IP evidence structurally unavailable**
//!   (design §4.2): the visible IP is the relay's, so the evidence is
//!   [`IpEvidence::Unknown`] with a named reason, never the relay's
//!   address treated as the user's.
//! - **X-Forwarded-For is not evidence unless explicitly trusted**
//!   (design §4.3): a declaration names which proxies may set it
//!   ([`ForwardTrust`], empty by default); an untrusted header is
//!   ignored, not believed — the same "a peer's own statement is never
//!   evidence" rule `residency.rs` already enforces.
//! - **Two IP signals that disagree resolve to unknown** (design
//!   §4.3), and unknown evidence satisfies nothing: under a scoped
//!   policy it does not grant *new* access, while leaving existing
//!   access untouched.

use std::collections::BTreeSet;
use std::net::IpAddr;

use crate::residency::PrivacyClass;

/// Upper bound on any raw-IP retention window (design §4.1: retention
/// is bounded and auditable; §4's default is connection lifetime plus
/// a bounded window). One day is the framework ceiling, not a
/// recommendation.
pub const MAX_RAW_RETENTION_SECS: u64 = 24 * 60 * 60;

/// Which proxies may set forwarding headers for a scope (design §4.3).
/// Empty — the default — trusts nobody: X-Forwarded-For is ignored and
/// the socket peer is the only network observation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardTrust {
    /// Socket-peer addresses of the trusted proxies, as text.
    pub trusted_proxies: BTreeSet<String>,
}

impl ForwardTrust {
    /// Whether this socket peer may speak for the client behind it.
    /// Configured addresses are compared as parsed addresses, so any
    /// valid spelling of the same address matches (`2001:0db8:0:0:...:1`
    /// names the same proxy as the socket's canonical `2001:db8::1`);
    /// an entry that does not parse as an address falls back to exact
    /// text comparison and so never matches a real peer by accident.
    pub fn trusts(&self, socket_peer: &IpAddr) -> bool {
        self.trusted_proxies.iter().any(|configured| {
            configured.parse::<IpAddr>().map_or_else(
                |_| configured == &socket_peer.to_string(),
                |addr| addr == *socket_peer,
            )
        })
    }
}

/// One connection's network observation, before any trust is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpObservation {
    /// The address the connection actually arrived from.
    pub socket_ip: IpAddr,
    /// The X-Forwarded-For value, if a header was present. Data, not
    /// evidence, until [`ForwardTrust`] says the peer may set it.
    pub forwarded_for: Option<IpAddr>,
    /// The connection arrived over a relay/bridge, so the visible IP
    /// is the relay's (design §4.2).
    pub via_relay: bool,
    /// When the observation was taken (unix seconds).
    pub observed_at: u64,
}

/// A verdict derived from one observation path: the country/region a
/// lookup assigns the address, and whether it falls inside the
/// boundary a declaration cares about. Either may be absent when the
/// lookup cannot say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpVerdict {
    /// ISO-style country code the lookup derived, if any.
    pub country: Option<String>,
    /// Region within the country, if the lookup derives one.
    pub region: Option<String>,
    /// Inside the declaration's boundary, if the lookup can judge.
    pub inside_boundary: Option<bool>,
}

/// Why IP evidence is unknown. Named, so a UI or log can state the
/// error mode (design §4.2) instead of silently treating a missing
/// signal as a negative one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpUnknownReason {
    /// Relay/bridge path: the visible IP is not the user's.
    RelayUnavailable,
    /// The available signals disagree with each other.
    SignalsDisagree,
    /// No observation was available at all.
    NoObservation,
    /// The lookup ran but determined no field at all: a verdict with
    /// no country, region, or boundary judgement is not evidence.
    LookupEmpty,
}

/// The resolved IP evidence for a subject: either one agreed verdict,
/// or explicitly unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpEvidence {
    /// All available signals agree on this verdict.
    Known(IpVerdict),
    /// No usable verdict; satisfies nothing (see module docs).
    Unknown(IpUnknownReason),
}

impl IpEvidence {
    /// Whether this evidence is a usable verdict.
    pub fn is_known(&self) -> bool {
        matches!(self, IpEvidence::Known(_))
    }
}

/// The signals one observation yields under a trust configuration:
/// the socket-peer's verdict always (unless relayed), plus the
/// forwarded verdict only when the peer is a trusted proxy. The
/// lookup is caller-supplied — this layer decides *which* addresses
/// may speak, never what a lookup says about them.
pub fn signals_for(
    observation: &IpObservation,
    trust: &ForwardTrust,
    lookup: &dyn Fn(IpAddr) -> IpVerdict,
) -> Vec<IpVerdict> {
    if observation.via_relay {
        return Vec::new();
    }
    let mut signals = vec![lookup(observation.socket_ip)];
    if let Some(forwarded) = observation.forwarded_for {
        if trust.trusts(&observation.socket_ip) {
            signals.push(lookup(forwarded));
        }
    }
    signals
}

/// Resolve an observation to evidence (design §4.3): relayed
/// observations are structurally unknown; disagreeing signals are
/// unknown; agreeing signals are the verdict.
pub fn resolve_evidence(
    observation: Option<&IpObservation>,
    trust: &ForwardTrust,
    lookup: &dyn Fn(IpAddr) -> IpVerdict,
) -> IpEvidence {
    let Some(observation) = observation else {
        return IpEvidence::Unknown(IpUnknownReason::NoObservation);
    };
    if observation.via_relay {
        return IpEvidence::Unknown(IpUnknownReason::RelayUnavailable);
    }
    let signals = signals_for(observation, trust, lookup);
    resolve_signals(&signals)
}

/// Resolve bare verdict signals: none is unknown, agreement is the
/// verdict, any disagreement (on country, region, or boundary) is
/// unknown — never a coin flip and never the loudest signal.
pub fn resolve_signals(signals: &[IpVerdict]) -> IpEvidence {
    let Some(first) = signals.first() else {
        return IpEvidence::Unknown(IpUnknownReason::NoObservation);
    };
    // A lookup that determined nothing has not produced a verdict,
    // however many signals agree on the emptiness: agreeing on
    // nothing is unknown, never a Known that satisfies a policy.
    if first.country.is_none() && first.region.is_none() && first.inside_boundary.is_none() {
        return IpEvidence::Unknown(IpUnknownReason::LookupEmpty);
    }
    if signals.iter().all(|s| s == first) {
        IpEvidence::Known(first.clone())
    } else {
        IpEvidence::Unknown(IpUnknownReason::SignalsDisagree)
    }
}

/// A stated retention window for a derived IP value (design §4.1):
/// the declaration must state the period and the purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retention {
    /// Why this value is kept.
    pub purpose: String,
    /// Unix seconds after which the value must be gone.
    pub retain_until: u64,
}

/// Why a retention request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionError {
    /// The window exceeds [`MAX_RAW_RETENTION_SECS`].
    WindowTooLong,
    /// The window ends at or before the observation it retains.
    WindowInPast,
    /// Raw-IP retention was enabled without the stated purpose the
    /// retention contract (design §4.1) requires.
    PurposeMissing,
}

/// Reduce evidence to what [`PrivacyClass::VerdictOnly`] permits: the
/// boundary verdict alone. Country and region are derived geography
/// and never ride along under a verdict-only label, however the
/// record was produced.
fn verdict_only(evidence: IpEvidence) -> IpEvidence {
    match evidence {
        IpEvidence::Known(v) => IpEvidence::Known(IpVerdict {
            country: None,
            region: None,
            inside_boundary: v.inside_boundary,
        }),
        unknown => unknown,
    }
}

/// What of an IP observation a scope retains, after the privacy rules
/// are applied. Constructed only through [`retain`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedIpEvidence {
    /// The resolved evidence.
    pub evidence: IpEvidence,
    /// What of it may be revealed downstream (design §4.1 ordering).
    pub privacy_class: PrivacyClass,
    /// The raw IP — present only when a scope explicitly enabled raw
    /// retention with a stated window and purpose.
    pub raw_ip: Option<IpAddr>,
    /// The stated retention, when anything is retained at all.
    pub retention: Option<Retention>,
}

/// Apply the retention rules (design §4.1) to a resolved verdict:
///
/// - Nothing is retained unless an enabled declaration requests it
///   (`retention: Some`); without one, the evidence is verdict-only
///   and in-memory only (`retention: None`, no raw IP, ever).
/// - The raw IP is retained only when the scope *explicitly* enables
///   it (`raw_enabled`) on top of a stated window and purpose; the
///   window must be in the future and within the framework ceiling.
/// - Relayed (unknown) evidence retains no raw IP under any setting:
///   the only address available is the relay's.
pub fn retain(
    evidence: IpEvidence,
    observation: Option<&IpObservation>,
    raw_enabled: bool,
    retention: Option<Retention>,
) -> Result<RetainedIpEvidence, RetentionError> {
    let Some(retention) = retention else {
        return Ok(RetainedIpEvidence {
            evidence: verdict_only(evidence),
            privacy_class: PrivacyClass::VerdictOnly,
            raw_ip: None,
            retention: None,
        });
    };
    let Some(observation) = observation else {
        return Ok(RetainedIpEvidence {
            evidence: verdict_only(evidence),
            privacy_class: PrivacyClass::VerdictOnly,
            raw_ip: None,
            retention: Some(retention),
        });
    };
    if retention.retain_until <= observation.observed_at {
        return Err(RetentionError::WindowInPast);
    }
    if retention.retain_until - observation.observed_at > MAX_RAW_RETENTION_SECS {
        return Err(RetentionError::WindowTooLong);
    }
    if raw_enabled && retention.purpose.trim().is_empty() {
        return Err(RetentionError::PurposeMissing);
    }
    let raw_ip = if raw_enabled && !observation.via_relay {
        Some(observation.socket_ip)
    } else {
        None
    };
    let privacy_class = if raw_ip.is_some() {
        PrivacyClass::Exact
    } else if matches!(&evidence, IpEvidence::Known(v) if v.country.is_some() || v.region.is_some())
    {
        PrivacyClass::Derived
    } else {
        PrivacyClass::VerdictOnly
    };
    Ok(RetainedIpEvidence {
        evidence,
        privacy_class,
        raw_ip,
        retention: Some(retention),
    })
}

/// Whether the access being judged is new or already held. The
/// distinction is the whole of design §3.4: weak evidence may fail to
/// *grant*, it may never *revoke*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    /// Access the subject does not yet hold.
    New,
    /// Access the subject already holds.
    Existing,
}

/// What a declaration chooses to do when IP evidence contradicts it
/// (design §4.3): `warn` or a reversible `step-up`. There is
/// deliberately no deny/quarantine choice — IP alone cannot choose
/// one, so the type does not offer one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContradictionChoice {
    /// Tell the subject (and log) that the signals disagree.
    Warn,
    /// Ask for stronger evidence, reversibly.
    StepUp,
}

/// Why a declaration's IP posture is invalid (design §4.3): it admits
/// on declared evidence while IP evidence may contradict, but names
/// no contradiction behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpPostureError {
    /// Declaration admits, IP contradicts, and no choice was made.
    MissingContradictionChoice,
}

/// The outcome of judging IP evidence beside a declaration. Note what
/// is absent: no variant denies or quarantines. `NotSatisfied` is the
/// evidence failing to *grant* new access under a scoped policy
/// (acceptance, #7784) — a property of the policy's evidence
/// requirement, not a punishment the IP signal imposes, and it is
/// unreachable for existing access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpOutcome {
    /// IP evidence agrees with (or does not speak against) the
    /// declaration; the declaration's own decision stands.
    Consistent,
    /// Declaration admits, IP contradicts: warn, per its choice.
    Warn,
    /// Declaration admits, IP contradicts (or IP is the only signal):
    /// reversible step-up, per its choice.
    StepUp,
    /// IP evidence is unknown and the subject holds this access
    /// already: no effect. Unknown is not negative.
    NoEffect,
    /// New access under a scoped policy whose evidence requirement
    /// this evidence does not satisfy (unknown, or no declaration and
    /// IP alone). Grants nothing; revokes nothing.
    NotSatisfied,
}

/// Judge IP evidence beside a declaration (design §4.3, all rows):
///
/// - Declaration admits and IP *contradicts* (both judge the boundary
///   and disagree): the declaration's stated choice — `warn` or
///   reversible `step-up`. No choice is an invalid declaration
///   ([`IpPostureError::MissingContradictionChoice`]). Either way the
///   access kind does not matter: existing access is never denied,
///   and new access follows the declaration, not the IP.
/// - No declaration (IP is the only signal): at most a reversible
///   step-up for new access; for existing access, no effect. IP alone
///   never satisfies an evidence requirement.
/// - IP evidence unknown: new access under a scoped policy is not
///   satisfied by it; existing access is untouched.
pub fn judge(
    declaration_admits: Option<bool>,
    declaration_boundary: Option<bool>,
    evidence: &IpEvidence,
    access: AccessKind,
    choice: Option<ContradictionChoice>,
) -> Result<IpOutcome, IpPostureError> {
    let IpEvidence::Known(verdict) = evidence else {
        return Ok(match access {
            AccessKind::New => IpOutcome::NotSatisfied,
            AccessKind::Existing => IpOutcome::NoEffect,
        });
    };
    match declaration_admits {
        None => Ok(match access {
            AccessKind::New => IpOutcome::StepUp,
            AccessKind::Existing => IpOutcome::NoEffect,
        }),
        Some(false) => Ok(IpOutcome::Consistent),
        Some(true) => {
            let contradicts = matches!(
                (declaration_boundary, verdict.inside_boundary),
                (Some(want), Some(got)) if want != got
            );
            if !contradicts {
                return Ok(IpOutcome::Consistent);
            }
            match choice {
                None => Err(IpPostureError::MissingContradictionChoice),
                Some(ContradictionChoice::Warn) => Ok(IpOutcome::Warn),
                Some(ContradictionChoice::StepUp) => Ok(IpOutcome::StepUp),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn verdict(country: &str, inside: bool) -> IpVerdict {
        IpVerdict {
            country: Some(country.to_string()),
            region: None,
            inside_boundary: Some(inside),
        }
    }

    fn obs(socket: IpAddr, forwarded: Option<IpAddr>, via_relay: bool) -> IpObservation {
        IpObservation {
            socket_ip: socket,
            forwarded_for: forwarded,
            via_relay,
            observed_at: 1_000,
        }
    }

    /// Oracle for the trust tests: addresses in 10.0.0.0/8 are "NL,
    /// inside"; everything else is "DE, outside" — so socket and
    /// forwarded signals agree or disagree by construction.
    fn lookup(addr: IpAddr) -> IpVerdict {
        match addr {
            IpAddr::V4(v4) if v4.octets()[0] == 10 => verdict("NL", true),
            _ => verdict("DE", false),
        }
    }

    #[test]
    fn untrusted_forwarded_for_is_ignored_not_believed() {
        // Socket in DE, header claims an NL address, trust is empty
        // (the default): the resolved evidence is the socket's alone.
        let o = obs(ip(192, 0, 2, 1), Some(ip(10, 1, 2, 3)), false);
        let evidence = resolve_evidence(Some(&o), &ForwardTrust::default(), &lookup);
        assert_eq!(evidence, IpEvidence::Known(verdict("DE", false)));
    }

    #[test]
    fn trusted_forwarded_for_joins_and_disagreement_is_unknown() {
        // Same observation, but the socket peer is a named proxy:
        // now both signals speak, they disagree, and the result is
        // unknown — not the header's claim and not the socket's.
        let mut trust = ForwardTrust::default();
        trust.trusted_proxies.insert("192.0.2.1".to_string());
        let o = obs(ip(192, 0, 2, 1), Some(ip(10, 1, 2, 3)), false);
        let evidence = resolve_evidence(Some(&o), &trust, &lookup);
        assert_eq!(
            evidence,
            IpEvidence::Unknown(IpUnknownReason::SignalsDisagree)
        );
    }

    #[test]
    fn trusted_forwarded_for_that_agrees_is_the_verdict() {
        let mut trust = ForwardTrust::default();
        trust.trusted_proxies.insert("192.0.2.1".to_string());
        let o = obs(ip(192, 0, 2, 1), Some(ip(198, 51, 100, 7)), false);
        let evidence = resolve_evidence(Some(&o), &trust, &lookup);
        assert_eq!(evidence, IpEvidence::Known(verdict("DE", false)));
    }

    #[test]
    fn relay_makes_ip_evidence_structurally_unknown() {
        let o = obs(ip(10, 1, 2, 3), None, true);
        let evidence = resolve_evidence(Some(&o), &ForwardTrust::default(), &lookup);
        assert_eq!(
            evidence,
            IpEvidence::Unknown(IpUnknownReason::RelayUnavailable)
        );
        // And no raw IP is retained for a relayed observation, even
        // when a scope asks for it: the only address is the relay's.
        let retained = retain(
            evidence,
            Some(&o),
            true,
            Some(Retention {
                purpose: "audit".into(),
                retain_until: 2_000,
            }),
        )
        .unwrap();
        assert_eq!(retained.raw_ip, None);
    }

    #[test]
    fn raw_ip_retained_only_when_explicitly_enabled_with_a_bounded_window() {
        let o = obs(ip(192, 0, 2, 1), None, false);
        let evidence = IpEvidence::Known(verdict("DE", false));
        // No retention requested: verdict only, nothing stored.
        let r = retain(evidence.clone(), Some(&o), true, None).unwrap();
        assert_eq!(r.raw_ip, None);
        assert_eq!(r.privacy_class, PrivacyClass::VerdictOnly);
        // Window stated but raw not enabled: derived only.
        let window = Retention {
            purpose: "audit".into(),
            retain_until: 2_000,
        };
        let r = retain(evidence.clone(), Some(&o), false, Some(window.clone())).unwrap();
        assert_eq!(r.raw_ip, None);
        assert_eq!(r.privacy_class, PrivacyClass::Derived);
        // Both: the raw IP, exactly classed.
        let r = retain(evidence, Some(&o), true, Some(window)).unwrap();
        assert_eq!(r.raw_ip, Some(ip(192, 0, 2, 1)));
        assert_eq!(r.privacy_class, PrivacyClass::Exact);
        // Windows out of bounds are refused, not clamped.
        let past = Retention {
            purpose: "audit".into(),
            retain_until: 1_000,
        };
        assert_eq!(
            retain(
                IpEvidence::Known(verdict("DE", false)),
                Some(&o),
                true,
                Some(past)
            ),
            Err(RetentionError::WindowInPast)
        );
        let huge = Retention {
            purpose: "audit".into(),
            retain_until: 1_000 + MAX_RAW_RETENTION_SECS + 1,
        };
        assert_eq!(
            retain(
                IpEvidence::Known(verdict("DE", false)),
                Some(&o),
                true,
                Some(huge)
            ),
            Err(RetentionError::WindowTooLong)
        );
    }

    #[test]
    fn disagreeing_signals_resolve_to_unknown_and_unknown_grants_no_new_access() {
        // Acceptance (#7784): disagreeing IP signals -> unknown, and
        // unknown denies *new* access under a scoped policy — as
        // NotSatisfied, the evidence requirement failing, while
        // existing access is untouched.
        let signals = vec![verdict("NL", true), verdict("DE", false)];
        let evidence = resolve_signals(&signals);
        assert_eq!(
            evidence,
            IpEvidence::Unknown(IpUnknownReason::SignalsDisagree)
        );
        assert_eq!(
            judge(
                Some(true),
                Some(true),
                &evidence,
                AccessKind::New,
                Some(ContradictionChoice::Warn)
            ),
            Ok(IpOutcome::NotSatisfied)
        );
        assert_eq!(
            judge(
                Some(true),
                Some(true),
                &evidence,
                AccessKind::Existing,
                Some(ContradictionChoice::Warn)
            ),
            Ok(IpOutcome::NoEffect)
        );
    }

    #[test]
    fn contradiction_follows_the_declarations_choice_and_needs_one() {
        let contradicting = IpEvidence::Known(verdict("DE", false));
        // Declaration admits an inside-boundary subject; IP says out.
        assert_eq!(
            judge(
                Some(true),
                Some(true),
                &contradicting,
                AccessKind::New,
                None
            ),
            Err(IpPostureError::MissingContradictionChoice)
        );
        assert_eq!(
            judge(
                Some(true),
                Some(true),
                &contradicting,
                AccessKind::Existing,
                Some(ContradictionChoice::Warn)
            ),
            Ok(IpOutcome::Warn)
        );
        assert_eq!(
            judge(
                Some(true),
                Some(true),
                &contradicting,
                AccessKind::New,
                Some(ContradictionChoice::StepUp)
            ),
            Ok(IpOutcome::StepUp)
        );
        // Agreement is not a contradiction and needs no choice.
        let agreeing = IpEvidence::Known(verdict("NL", true));
        assert_eq!(
            judge(Some(true), Some(true), &agreeing, AccessKind::New, None),
            Ok(IpOutcome::Consistent)
        );
    }

    /// Property (oracle: design §3.4 / §4.3): IP-derived evidence
    /// alone never yields an outcome that removes existing access.
    /// Exhaustive over the whole input space of `judge`: for
    /// `AccessKind::Existing` the outcome is never `NotSatisfied`
    /// (the only non-granting outcome, itself new-access-only) and
    /// never an error-driven denial — there is no deny variant to
    /// reach.
    #[test]
    fn property_ip_evidence_alone_never_removes_existing_access() {
        let evidences = [
            IpEvidence::Known(verdict("NL", true)),
            IpEvidence::Known(verdict("DE", false)),
            IpEvidence::Known(IpVerdict {
                country: None,
                region: None,
                inside_boundary: None,
            }),
            IpEvidence::Unknown(IpUnknownReason::RelayUnavailable),
            IpEvidence::Unknown(IpUnknownReason::SignalsDisagree),
            IpEvidence::Unknown(IpUnknownReason::NoObservation),
        ];
        for evidence in &evidences {
            for admits in [None, Some(true), Some(false)] {
                for boundary in [None, Some(true), Some(false)] {
                    for choice in [
                        None,
                        Some(ContradictionChoice::Warn),
                        Some(ContradictionChoice::StepUp),
                    ] {
                        let outcome =
                            judge(admits, boundary, evidence, AccessKind::Existing, choice);
                        if let Ok(outcome) = outcome {
                            assert_ne!(
                                outcome,
                                IpOutcome::NotSatisfied,
                                "existing access affected: evidence={evidence:?} admits={admits:?} \
                                 boundary={boundary:?} choice={choice:?}"
                            );
                            assert!(
                                matches!(
                                    outcome,
                                    IpOutcome::Consistent
                                        | IpOutcome::Warn
                                        | IpOutcome::StepUp
                                        | IpOutcome::NoEffect
                                ),
                                "existing access got a granting/revoking outcome: {outcome:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_lookup_result_is_unknown_not_a_known_verdict() {
        // Codex finding on PR #8551: a lookup that determines no field
        // must not produce Known evidence that satisfies a policy.
        let empty = IpVerdict {
            country: None,
            region: None,
            inside_boundary: None,
        };
        assert_eq!(
            resolve_signals(&[empty.clone(), empty]),
            IpEvidence::Unknown(IpUnknownReason::LookupEmpty)
        );
        let o = obs(ip(192, 0, 2, 1), None, false);
        let evidence = resolve_evidence(Some(&o), &ForwardTrust::default(), &|_| IpVerdict {
            country: None,
            region: None,
            inside_boundary: None,
        });
        assert!(!evidence.is_known());
        assert_eq!(
            judge(
                Some(true),
                Some(true),
                &evidence,
                AccessKind::New,
                Some(ContradictionChoice::Warn)
            ),
            Ok(IpOutcome::NotSatisfied)
        );
    }

    #[test]
    fn verdict_only_retention_strips_geography() {
        // Codex finding on PR #8551: a VerdictOnly record may carry
        // the boundary verdict alone, never country/region.
        let o = obs(ip(192, 0, 2, 1), None, false);
        let mut v = verdict("DE", false);
        v.region = Some("Bavaria".to_string());
        let r = retain(IpEvidence::Known(v.clone()), Some(&o), true, None).unwrap();
        assert_eq!(r.privacy_class, PrivacyClass::VerdictOnly);
        let IpEvidence::Known(stripped) = &r.evidence else {
            panic!("expected known boundary verdict, got {:?}", r.evidence);
        };
        assert_eq!(stripped.country, None);
        assert_eq!(stripped.region, None);
        assert_eq!(stripped.inside_boundary, Some(false));
        // The missing-observation early return classifies the same way.
        let r = retain(
            IpEvidence::Known(v),
            None,
            false,
            Some(Retention {
                purpose: "audit".into(),
                retain_until: 2_000,
            }),
        )
        .unwrap();
        assert_eq!(r.privacy_class, PrivacyClass::VerdictOnly);
        let IpEvidence::Known(stripped) = &r.evidence else {
            panic!("expected known boundary verdict, got {:?}", r.evidence);
        };
        assert_eq!(stripped.country, None);
        assert_eq!(stripped.region, None);
    }

    #[test]
    fn raw_retention_requires_a_stated_purpose() {
        // Codex finding on PR #8551: an empty or whitespace purpose
        // does not satisfy the retention contract for raw IPs.
        let o = obs(ip(192, 0, 2, 1), None, false);
        for purpose in ["", "   "] {
            assert_eq!(
                retain(
                    IpEvidence::Known(verdict("DE", false)),
                    Some(&o),
                    true,
                    Some(Retention {
                        purpose: purpose.into(),
                        retain_until: 2_000,
                    })
                ),
                Err(RetentionError::PurposeMissing)
            );
        }
    }

    #[test]
    fn trusted_proxy_matching_normalizes_address_spelling() {
        // Codex finding on PR #8551: an expanded IPv6 spelling in the
        // trust set names the same proxy as the canonical socket form.
        let mut trust = ForwardTrust::default();
        trust
            .trusted_proxies
            .insert("2001:0db8:0000:0000:0000:0000:0000:0001".to_string());
        let socket: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(trust.trusts(&socket));
        // The forwarded signal now joins and, disagreeing, makes the
        // evidence unknown instead of silently resolving the proxy.
        let o = obs(socket, Some(ip(10, 1, 2, 3)), false);
        let evidence = resolve_evidence(Some(&o), &trust, &lookup);
        assert_eq!(
            evidence,
            IpEvidence::Unknown(IpUnknownReason::SignalsDisagree)
        );
    }
}
