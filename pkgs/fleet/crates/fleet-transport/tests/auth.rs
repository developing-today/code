//! Acceptance tests for #14720 (work item 2c): the accept-side
//! NodeId allowlist derived from registry enrollment, enforced per
//! ALPN, with refusals logged by NodeId and ALPN.
//!
//! Oracle: a plain model — a binding table consulted by hand. A peer
//! is allowed iff a binding exists for its NodeId, the binding is
//! active, and the requested ALPN is in the binding's list; every
//! other case refuses with the reason the model names, and every
//! refusal (and only refusals) appears in the refusal log.

use fleet_transport::{Allowlist, BindingState, BuildError, Decision, Enrollment, RefusalReason};
use proptest::prelude::*;

const LAPTOP: &str = "fleet/laptop-agent/1";
const SHELL: &str = "fleet/shell/1";

fn enrolled(node_id: &str, state: BindingState) -> Enrollment {
    Enrollment {
        node_id: node_id.to_string(),
        agent_class: "opencode-lanes".to_string(),
        alpns: vec![LAPTOP.to_string()],
        state,
    }
}

fn allowlist() -> Allowlist {
    Allowlist::from_enrollments([
        enrolled("node-active", BindingState::Active),
        enrolled("node-revoked", BindingState::Revoked),
    ])
    .expect("fixture enrollments are valid")
}

#[test]
fn allowed_nodeid_on_enrolled_alpn_is_allowed() {
    let mut a = allowlist();
    assert_eq!(
        a.authorize("node-active", LAPTOP),
        Decision::Allowed {
            agent_class: "opencode-lanes".to_string()
        }
    );
    assert!(a.refusals().is_empty(), "an allow must not log a refusal");
}

#[test]
fn unknown_nodeid_is_refused_and_logged_with_nodeid_and_alpn() {
    let mut a = allowlist();
    assert_eq!(
        a.authorize("node-stranger", LAPTOP),
        Decision::Refused {
            reason: RefusalReason::UnknownNodeId
        }
    );
    let log = a.refusals();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].node_id, "node-stranger");
    assert_eq!(log[0].alpn, LAPTOP);
    assert_eq!(log[0].reason, RefusalReason::UnknownNodeId);
    // The emitted line carries both identifiers an operator needs.
    let line = log[0].to_string();
    assert!(line.contains("node-stranger"), "{line}");
    assert!(line.contains(LAPTOP), "{line}");
}

#[test]
fn revoked_nodeid_is_refused_as_revoked_not_unknown() {
    let mut a = allowlist();
    assert_eq!(
        a.authorize("node-revoked", LAPTOP),
        Decision::Refused {
            reason: RefusalReason::RevokedNodeId
        }
    );
    assert_eq!(a.refusals().len(), 1);
    assert_eq!(a.refusals()[0].reason, RefusalReason::RevokedNodeId);
}

#[test]
fn enrolled_nodeid_on_unenrolled_alpn_is_refused() {
    let mut a = allowlist();
    assert_eq!(
        a.authorize("node-active", SHELL),
        Decision::Refused {
            reason: RefusalReason::AlpnNotEnrolled
        }
    );
    assert_eq!(a.refusals().len(), 1);
    assert_eq!(a.refusals()[0].alpn, SHELL);
}

#[test]
fn duplicate_enrollment_for_one_nodeid_is_a_build_error() {
    let err = Allowlist::from_enrollments([
        enrolled("node-active", BindingState::Active),
        enrolled("node-active", BindingState::Revoked),
    ])
    .unwrap_err();
    assert_eq!(
        err,
        BuildError::DuplicateNodeId {
            node_id: "node-active".to_string()
        }
    );
}

#[test]
fn enrollment_listing_no_alpns_is_a_build_error() {
    let mut e = enrolled("node-active", BindingState::Active);
    e.alpns.clear();
    assert_eq!(
        Allowlist::from_enrollments([e]).unwrap_err(),
        BuildError::NoAlpns {
            node_id: "node-active".to_string()
        }
    );
}

#[test]
fn drain_refusals_empties_the_log() {
    let mut a = allowlist();
    let _ = a.authorize("node-stranger", LAPTOP);
    assert_eq!(a.drain_refusals().len(), 1);
    assert!(a.refusals().is_empty());
}

proptest! {
    /// Model oracle: decisions and the refusal log match a hand-rolled
    /// lookup over the same bindings, for arbitrary query sequences.
    #[test]
    fn decisions_match_the_model(
        queries in prop::collection::vec(
            (prop::sample::select(vec!["node-active", "node-revoked", "node-stranger"]),
             prop::sample::select(vec![LAPTOP, SHELL])),
            0..32,
        ),
    ) {
        let mut a = allowlist();
        let mut expected_refusals = 0usize;
        for (node, alpn) in &queries {
            let expected = match (*node, *alpn) {
                ("node-revoked", _) => Decision::Refused { reason: RefusalReason::RevokedNodeId },
                ("node-active", LAPTOP) => Decision::Allowed {
                    agent_class: "opencode-lanes".to_string(),
                },
                ("node-active", _) => Decision::Refused { reason: RefusalReason::AlpnNotEnrolled },
                _ => Decision::Refused { reason: RefusalReason::UnknownNodeId },
            };
            if matches!(expected, Decision::Refused { .. }) {
                expected_refusals += 1;
            }
            prop_assert_eq!(a.authorize(node, alpn), expected);
        }
        prop_assert_eq!(a.refusals().len(), expected_refusals);
    }
}
