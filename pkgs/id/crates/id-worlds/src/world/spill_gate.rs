//! Residency gate for hot-world spill (design #3398 §3 hot-world spill
//! row, §6 launch gate, §7 step 5; issue #9394).
//!
//! Spill is the last rung of the recorded escalation ladder — shed
//! non-actor work, then partition where appropriate, then consider
//! temporary native execution for indivisible load. This module is the
//! gate that decides whether a spill may happen at all. Scoped spill is
//! enabled only when **all three** gates pass:
//!
//! 1. **Residency**: the step-1 evaluator ([`id_core::residency::evaluate`])
//!    allows the spill path — the execution host (Processing), the
//!    journal location (Storage), and every transport/metadata hop the
//!    spill touches carry current, trusted, resource-bound evidence
//!    inside the policy. An unattested native host is denied here.
//! 2. **Single-writer lease**: the journaled write lease
//!    ([`super::WriteLease`], design #2819) names the spill host as the
//!    current holder at the presented epoch, unexpired on the
//!    authority's clock ([`WriteLease::check_writer`]). A missing,
//!    expired, or foreign-held lease is denied here.
//! 3. **Successor replay**: the host's verified replay cursor reaches the
//!    current grant/handoff boundary before execution is admitted.
//!
//! Until all pass, work stays local, is shed, or reports overload —
//! the gate has no third outcome, and callers move journal/event copies
//! only through [`dispatch_spill`], which sends nothing on a denial:
//! no journal or event copy crosses the boundary when any gate
//! refuses. Because the lease names exactly one holder at any epoch,
//! the original and the spill host cannot both pass the writer check
//! at once — dual-writer spill is impossible by construction, and the
//! tests below prove both directions.
//!
//! The lease check applies to unscoped content too: single-writer is a
//! correctness invariant of spill itself, not a residency feature. The
//! residency check preserves pre-residency behaviour for
//! [`Scope::Unscoped`] exactly, as the step-1 evaluator allows it
//! unconditionally. Evidence is pinned from the operator's local
//! deployment inventory by the caller, never taken from the spill
//! host's own assertions — and neither a signature nor encryption on
//! the spilled copy is an input here: both prove authorship and
//! confidentiality, never locality (design §3, §6).

use id_core::residency::{Decision, DenyReason, EvidenceVerifier, Hop, Scope, evaluate};

use super::{LeaseRejectionReason, LeaseTransitionKind, WriteLease};

/// Why a spill was denied. [`SpillDeny::Residency`] carries the step-1
/// evaluator's own reason codes; [`SpillDeny::Lease`] carries the
/// append gate's rejection reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpillDeny {
    /// The residency evaluator denied the spill path (unknown,
    /// expired, revoked, mismatched or out-of-policy evidence for the
    /// execution host, the journal location, or another hop).
    Residency(Vec<DenyReason>),
    /// The journaled single-writer lease does not admit the spill
    /// host: it is not the holder, presented a stale epoch, or its
    /// term has expired.
    Lease(LeaseRejectionReason),
    /// The successor has not replayed its current grant/handoff boundary.
    ReplayPending {
        /// Verified successor cursor, supplied by the host's replay path.
        replayed_position: u64,
        /// Ownership boundary derived from the authority's lease history.
        required_position: u64,
    },
}

/// The gate's verdict on one spill attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpillDecision {
    /// Residency evidence, the single-writer lease, and replay all pass: the
    /// spill copy may cross to the spill host.
    Allow,
    /// At least one gate refused. Nothing may cross; the work stays
    /// local, is shed, or reports overload.
    Deny(Vec<SpillDeny>),
}

impl SpillDecision {
    /// Whether the spill may proceed.
    #[must_use]
    pub fn is_allow(&self) -> bool {
        matches!(self, SpillDecision::Allow)
    }
}

/// Evaluate one spill attempt: residency for the whole path (`hops`
/// must cover the execution host on Processing, the journal location
/// on Storage, and any transport/metadata hops the spill uses), and
/// the single-writer lease for `spill_host` at `presented_epoch`.
///
/// `now_s` is the evidence clock (unix seconds, the evaluator's
/// clock); `now_ms` is the authority's lease clock (unix
/// milliseconds). Denial lists every reason found across all three gates.
/// `replayed_position` must be a contiguous cursor from the successor's verified
/// replay path, never its unauthenticated assertion or the authority's watermark.
/// A current grant or handoff must
/// be replayed before admission; later renewals and rejection audits do not
/// move this ownership boundary. Bootstrap/unstamped legacy boundaries are 0.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_spill(
    scope: &Scope,
    hops: &[Hop<'_>],
    lease: &WriteLease,
    spill_host: &str,
    presented_epoch: u64,
    replayed_position: u64,
    now_s: u64,
    now_ms: u64,
    verifier: &dyn EvidenceVerifier,
) -> SpillDecision {
    let mut reasons = Vec::new();
    if let Decision::Deny(residency) = evaluate(scope, hops, now_s, verifier) {
        reasons.push(SpillDeny::Residency(residency));
    }
    if let Err(lease_reason) = lease.check_writer(spill_host, presented_epoch, now_ms) {
        reasons.push(SpillDeny::Lease(lease_reason));
    }
    let required_position = lease
        .transitions()
        .filter(|transition| {
            transition.epoch == lease.epoch
                && matches!(
                    transition.kind,
                    LeaseTransitionKind::Granted | LeaseTransitionKind::HandedOff
                )
        })
        .last()
        .map_or(0, |transition| transition.position);
    if replayed_position < required_position {
        reasons.push(SpillDeny::ReplayPending {
            replayed_position,
            required_position,
        });
    }
    if reasons.is_empty() {
        SpillDecision::Allow
    } else {
        SpillDecision::Deny(reasons)
    }
}

/// Move the spill's journal/event copy across the boundary — the only
/// sanctioned way to do it. On [`SpillDecision::Allow`], `send` is
/// called exactly once with the copy. On a denial, `send` is never
/// called: no journal or event copy crosses the boundary, and the
/// denial reasons are returned for the caller's local/shed/overload
/// handling.
///
/// # Errors
///
/// The [`SpillDeny`] reasons of a denied decision, unchanged.
pub fn dispatch_spill(
    decision: &SpillDecision,
    journal_copy: &[u8],
    send: impl FnOnce(&[u8]),
) -> Result<(), Vec<SpillDeny>> {
    match decision {
        SpillDecision::Allow => {
            send(journal_copy);
            Ok(())
        }
        SpillDecision::Deny(reasons) => Err(reasons.clone()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::cell::RefCell;

    use id_core::residency::{Evidence, Jurisdiction, Plane, Policy, ResourceId};
    use proptest::prelude::*;

    use super::super::LeaseWriteRejection;
    use super::*;

    /// Evidence clock, unix seconds; lease clock is `NOW_S * 1000` ms.
    const NOW_S: u64 = 1_000_000;
    const NOW_MS: u64 = NOW_S * 1000;

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

    fn verifier() -> Verifier {
        Verifier {
            revoked_serial: None,
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

    /// Scoped policy: spill host, journal, relay and telemetry sink all
    /// in "eu", evidence from the provider control plane.
    fn scoped() -> Scope {
        let policy = Policy::new(["operator-ca".to_string()])
            .allow(Plane::Processing, &["eu"], &["spill-eu"])
            .allow(Plane::Storage, &["eu"], &["journal-eu"])
            .allow(Plane::Transport, &["eu"], &["relay-eu"])
            .allow(Plane::Metadata, &["eu"], &["telemetry-eu"])
            .trust_evidence_source(Plane::Processing, "provider-control-plane")
            .trust_evidence_source(Plane::Storage, "provider-control-plane")
            .trust_evidence_source(Plane::Transport, "provider-control-plane")
            .trust_evidence_source(Plane::Metadata, "provider-control-plane");
        Scope::Scoped(policy)
    }

    /// The full spill path with fresh evidence for every hop.
    fn attested_hops() -> Vec<(Evidence, Plane, &'static str)> {
        vec![
            (
                evidence("spill-eu", Plane::Processing, "eu", NOW_S + 600),
                Plane::Processing,
                "spill-eu",
            ),
            (
                evidence("journal-eu", Plane::Storage, "eu", NOW_S + 600),
                Plane::Storage,
                "journal-eu",
            ),
            (
                evidence("relay-eu", Plane::Transport, "eu", NOW_S + 600),
                Plane::Transport,
                "relay-eu",
            ),
            (
                evidence("telemetry-eu", Plane::Metadata, "eu", NOW_S + 600),
                Plane::Metadata,
                "telemetry-eu",
            ),
        ]
    }

    fn hops_of<'a>(set: &'a [(Evidence, Plane, &'static str)]) -> Vec<Hop<'a>> {
        set.iter()
            .map(|(ev, plane, resource)| Hop {
                plane: *plane,
                resource: ResourceId((*resource).to_string()),
                evidence: Some(ev),
            })
            .collect()
    }

    /// A lease naming the spill host at epoch 3, term open past NOW.
    fn spill_lease() -> WriteLease {
        WriteLease {
            holder: Some("spill-eu".to_string()),
            epoch: 3,
            expires_at_ms: Some(NOW_MS + 600_000),
            history: Vec::new(),
        }
    }

    fn decide(
        scope: &Scope,
        hops: &[Hop<'_>],
        lease: &WriteLease,
        spill_host: &str,
        epoch: u64,
    ) -> SpillDecision {
        evaluate_spill(
            scope,
            hops,
            lease,
            spill_host,
            epoch,
            lease.last_position(),
            NOW_S,
            NOW_MS,
            &verifier(),
        )
    }

    #[test]
    fn test_successor_dispatch_waits_for_ownership_replay() {
        // Oracle: #5787 §1 step 4. A matching grant is not enough:
        // no dispatch may occur before the successor replays its boundary.
        let mut lease = WriteLease::bootstrap();
        lease.apply_revoke("do", 1, "handoff", 4).unwrap();
        lease
            .apply_grant("spill-eu", 2, Some(NOW_MS + 600_000), 5)
            .unwrap();
        for replayed_position in [0, 4, 5, 6] {
            let decision = evaluate_spill(
                &Scope::Unscoped,
                &[],
                &lease,
                "spill-eu",
                2,
                replayed_position,
                NOW_S,
                NOW_MS,
                &verifier(),
            );
            let sent = RefCell::new(0);
            let result = dispatch_spill(&decision, b"journal-copy", |_| {
                *sent.borrow_mut() += 1;
            });
            let ready = replayed_position >= 5;
            assert_eq!(decision.is_allow(), ready);
            if !ready {
                assert_eq!(
                    decision,
                    SpillDecision::Deny(vec![SpillDeny::ReplayPending {
                        replayed_position,
                        required_position: 5,
                    }])
                );
            }
            assert_eq!(result.is_ok(), ready);
            assert_eq!(*sent.borrow(), usize::from(ready));
        }
    }

    proptest! {
        /// Oracle: #5787 §1 step 4; renewals extend a term, not ownership.
        #[test]
        fn test_ownership_replay_boundary_survives_renewals(
            boundary in 1_u64..1_000_000,
            cursor in 0_u64..1_000_002,
            handoff in any::<bool>(),
        ) {
            let mut lease = WriteLease::bootstrap();
            let epoch = if handoff {
                lease.apply_handoff("do", "spill-eu", 1, Some(NOW_MS + 100), boundary).unwrap();
                1
            } else {
                lease.apply_revoke("do", 1, "expired", boundary).unwrap();
                lease.apply_grant("spill-eu", 2, Some(NOW_MS + 100), boundary + 1).unwrap();
                2
            };
            let required = if handoff { boundary } else { boundary + 1 };
            lease.apply_renew("spill-eu", epoch, NOW_MS + 200, required + 1).unwrap();
            lease.apply_rejection(LeaseWriteRejection {
                writer: "spill-eu".into(),
                presented_epoch: epoch - 1,
                operation_id: "stale-append".into(),
                observed_epoch: epoch,
                reason: LeaseRejectionReason::StaleEpoch,
                received_at_ms: NOW_MS,
                position: required + 2,
            }).unwrap();
            let decision = evaluate_spill(
                &Scope::Unscoped, &[], &lease, "spill-eu", epoch,
                cursor, NOW_S, NOW_MS, &verifier(),
            );
            prop_assert_eq!(decision.is_allow(), cursor >= required);
        }
    }

    #[test]
    fn test_spill_allows_only_when_evidence_and_lease_both_pass() {
        let set = attested_hops();
        let decision = decide(&scoped(), &hops_of(&set), &spill_lease(), "spill-eu", 3);
        assert_eq!(decision, SpillDecision::Allow);
        // The copy crosses exactly once.
        let sent: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        dispatch_spill(&decision, b"journal-copy", |copy| {
            sent.borrow_mut().push(copy.to_vec());
        })
        .unwrap();
        assert_eq!(sent.borrow().len(), 1);
    }

    #[test]
    fn test_an_unattested_spill_host_is_denied_and_no_copy_crosses() {
        // No Processing hop at all: the execution host is unattested.
        let set: Vec<(Evidence, Plane, &'static str)> = attested_hops()
            .into_iter()
            .filter(|(_, plane, _)| *plane != Plane::Processing)
            .collect();
        let decision = decide(&scoped(), &hops_of(&set), &spill_lease(), "spill-eu", 3);
        let SpillDecision::Deny(reasons) = &decision else {
            panic!("unattested host must be denied, got {decision:?}");
        };
        assert!(reasons.iter().any(|r| matches!(
            r,
            SpillDeny::Residency(v) if v.contains(&DenyReason::PlaneUnknown(Plane::Processing))
        )));
        let sent: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        assert!(
            dispatch_spill(&decision, b"journal-copy", |copy| {
                sent.borrow_mut().push(copy.to_vec());
            })
            .is_err()
        );
        assert!(sent.borrow().is_empty(), "no copy may cross on denial");
    }

    #[test]
    fn test_expired_host_evidence_is_denied_and_no_copy_crosses() {
        let mut set = attested_hops();
        set[0].0.expires_at = NOW_S - 1;
        let decision = decide(&scoped(), &hops_of(&set), &spill_lease(), "spill-eu", 3);
        let SpillDecision::Deny(reasons) = &decision else {
            panic!("expired evidence must be denied, got {decision:?}");
        };
        assert!(reasons.iter().any(|r| matches!(
            r,
            SpillDeny::Residency(v) if v.contains(&DenyReason::Expired(Plane::Processing))
        )));
        let sent: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        assert!(
            dispatch_spill(&decision, b"journal-copy", |copy| {
                sent.borrow_mut().push(copy.to_vec());
            })
            .is_err()
        );
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn test_a_missing_or_foreign_lease_is_denied_and_no_copy_crosses() {
        let set = attested_hops();
        // The lease still stands with the original host (bootstrap):
        // the spill host holds nothing.
        let foreign = WriteLease::bootstrap();
        let decision = decide(&scoped(), &hops_of(&set), &foreign, "spill-eu", 0);
        assert_eq!(
            decision,
            SpillDecision::Deny(vec![SpillDeny::Lease(LeaseRejectionReason::NotHolder)])
        );
        let sent: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        assert!(
            dispatch_spill(&decision, b"journal-copy", |copy| {
                sent.borrow_mut().push(copy.to_vec());
            })
            .is_err()
        );
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn test_an_expired_lease_is_denied() {
        let set = attested_hops();
        let mut lease = spill_lease();
        lease.expires_at_ms = Some(NOW_MS - 1);
        let decision = decide(&scoped(), &hops_of(&set), &lease, "spill-eu", 3);
        assert_eq!(
            decision,
            SpillDecision::Deny(vec![SpillDeny::Lease(LeaseRejectionReason::Expired)])
        );
    }

    #[test]
    fn test_a_stale_epoch_is_denied() {
        let set = attested_hops();
        let decision = decide(&scoped(), &hops_of(&set), &spill_lease(), "spill-eu", 2);
        assert_eq!(
            decision,
            SpillDecision::Deny(vec![SpillDeny::Lease(LeaseRejectionReason::StaleEpoch)])
        );
    }

    #[test]
    fn test_original_and_spill_host_cannot_both_accept_writes() {
        let set = attested_hops();
        let lease = spill_lease();
        // After the handoff the lease names the spill host: the gate
        // admits the spill host and refuses the original holder, and
        // the append gate agrees — at most one of them can write.
        assert!(decide(&scoped(), &hops_of(&set), &lease, "spill-eu", 3).is_allow());
        assert_eq!(
            decide(&scoped(), &hops_of(&set), &lease, "do", 3),
            SpillDecision::Deny(vec![SpillDeny::Lease(LeaseRejectionReason::NotHolder)])
        );
        assert!(lease.check_writer("spill-eu", 3, NOW_MS).is_ok());
        assert_eq!(
            lease.check_writer("do", 3, NOW_MS),
            Err(LeaseRejectionReason::NotHolder)
        );
        // Before any handoff (bootstrap), the direction reverses: the
        // original holds the lease and the spill host is refused.
        let bootstrap = WriteLease::bootstrap();
        assert_eq!(
            bootstrap.check_writer("spill-eu", 0, NOW_MS),
            Err(LeaseRejectionReason::NotHolder)
        );
        assert!(bootstrap.check_writer("do", 0, NOW_MS).is_ok());
    }

    #[test]
    fn test_unscoped_spill_needs_the_lease_but_no_evidence() {
        // Pre-residency behaviour: no hops, no evidence — residency
        // allows, and the lease alone decides.
        let decision = decide(&Scope::Unscoped, &[], &spill_lease(), "spill-eu", 3);
        assert_eq!(decision, SpillDecision::Allow);
        let decision = decide(
            &Scope::Unscoped,
            &[],
            &WriteLease::bootstrap(),
            "spill-eu",
            0,
        );
        assert_eq!(
            decision,
            SpillDecision::Deny(vec![SpillDeny::Lease(LeaseRejectionReason::NotHolder)])
        );
    }
}
