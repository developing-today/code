//! Model oracle for A-GRAPH-9130-01.
//!
//! The independent model asserts that requests, moderation, projection hide
//! and server-local true deletion remain distinct; batches are idempotent;
//! retained evidence stays verifiable; and two replicas do not mutate one
//! another's local signed state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../../id-core/src/graph/deletion.rs"]
mod deletion;

// The slice is intentionally not crate-root wired until the 9110-02
// integration work. This shim supplies the module path used by the source
// when the model includes it directly.
mod canonical_cbor {
    pub use id_core::canonical_cbor::*;
}

// Same crate-root wiring gap as above: deletion.rs calls
// `crate::hex::hex_encode`, which resolves in id-core but not when the
// model includes the file directly.
mod hex {
    pub use id_core::hex::*;
}

use p2panda_core::SigningKey;

use deletion::{DeletionAction, DeletionState, DeletionStateMachine, TransitionOutcome};

const OBJECT: [u8; 32] = [9; 32];

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn state_machine_keeps_request_queue_hide_and_true_delete_distinct() {
    let signer = key(1);
    let mut machine = DeletionStateMachine::new(OBJECT);
    assert_eq!(machine.object(), OBJECT);

    assert_eq!(
        machine
            .apply("alice", DeletionAction::Request { request_id: 7 }, &signer)
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteRequested { request_id: 7 }
    ));
    machine
        .apply(
            "moderator",
            DeletionAction::Queue {
                request_id: 7,
                ticket_id: 42,
            },
            &signer,
        )
        .unwrap();
    machine
        .apply("moderator", DeletionAction::Hide { ticket_id: 42 }, &signer)
        .unwrap();
    assert!(matches!(
        machine.state(),
        DeletionState::HiddenInProjection { ticket_id: 42, .. }
    ));
    assert_eq!(machine.evidence().len(), 3);
    for evidence in machine.evidence() {
        evidence.verify().unwrap();
    }

    machine
        .apply(
            "moderator",
            DeletionAction::ClearHide { ticket_id: 42 },
            &signer,
        )
        .unwrap();
    assert!(matches!(
        machine.state(),
        DeletionState::ModerationQueued {
            request_id: 7,
            ticket_id: 42
        }
    ));

    machine
        .apply(
            "server-admin",
            DeletionAction::AdminTrueDelete {
                policy_id: "legal-removal-v1".to_owned(),
                references_override: true,
            },
            &signer,
        )
        .unwrap();
    assert!(matches!(
        machine.state(),
        DeletionState::TrueDeletedOnThisServer { .. }
    ));
    assert_eq!(
        machine
            .apply("alice", DeletionAction::Request { request_id: 8 }, &signer)
            .unwrap_err(),
        deletion::DeletionError::AlreadyDeleted
    );
}

#[test]
fn reasoned_request_delivers_and_grant_produces_a_policy_delete() {
    let signer = key(5);
    let mut machine = DeletionStateMachine::new(OBJECT);
    let authorities = vec![
        "tag-author".to_owned(),
        "group-admin".to_owned(),
        "server-owner".to_owned(),
    ];

    assert_eq!(
        machine
            .apply(
                "alice",
                DeletionAction::RequestReasoned {
                    request_id: 11,
                    reason: "spam tag on my photo".to_owned(),
                    delivered_to: authorities.clone(),
                },
                &signer,
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    // Delivery record: reason + authority holders retained.
    assert_eq!(machine.deliveries().len(), 1);
    assert_eq!(machine.deliveries()[0].request_id, 11);
    assert_eq!(machine.deliveries()[0].reason, "spam tag on my photo");
    assert_eq!(machine.deliveries()[0].delivered_to, authorities);
    assert_eq!(machine.evidence()[0].action, "delete_requested");
    assert_eq!(machine.evidence()[0].rationale, "spam tag on my photo");

    machine
        .apply(
            "moderator",
            DeletionAction::Queue {
                request_id: 11,
                ticket_id: 50,
            },
            &signer,
        )
        .unwrap();
    // A grant for a different request is refused.
    assert!(
        machine
            .apply(
                "group-admin",
                DeletionAction::Grant {
                    request_id: 12,
                    policy_id: "spam-v1".to_owned(),
                },
                &signer,
            )
            .is_err()
    );

    assert_eq!(
        machine
            .apply(
                "group-admin",
                DeletionAction::Grant {
                    request_id: 11,
                    policy_id: "spam-v1".to_owned(),
                },
                &signer,
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::TrueDeletedOnThisServer { .. }
    ));
    let grant = machine
        .evidence()
        .iter()
        .find(|e| e.action == "delete_granted")
        .expect("grant evidence recorded");
    assert!(grant.rationale.contains("policy=spam-v1"));
    for evidence in machine.evidence() {
        evidence.verify().unwrap();
    }
}

#[test]
fn decline_is_recorded_with_no_delete_and_a_new_request_may_follow() {
    let signer = key(6);
    let mut machine = DeletionStateMachine::new(OBJECT);
    machine
        .apply(
            "bob",
            DeletionAction::RequestReasoned {
                request_id: 21,
                reason: "please remove".to_owned(),
                delivered_to: vec!["server-owner".to_owned()],
            },
            &signer,
        )
        .unwrap();

    assert_eq!(
        machine
            .apply(
                "server-owner",
                DeletionAction::Decline {
                    request_id: 21,
                    reason: "evidence is the audit trail".to_owned(),
                },
                &signer,
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteDeclined { request_id: 21 }
    ));
    let decline = machine
        .evidence()
        .iter()
        .find(|e| e.action == "delete_declined")
        .expect("decline evidence recorded");
    assert_eq!(decline.rationale, "evidence is the audit trail");
    decline.verify().unwrap();

    // No delete happened: a fresh request is still possible.
    assert_eq!(
        machine
            .apply(
                "bob",
                DeletionAction::RequestReasoned {
                    request_id: 22,
                    reason: "second look, new evidence".to_owned(),
                    delivered_to: vec!["server-owner".to_owned()],
                },
                &signer,
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteRequested { request_id: 22 }
    ));
}

#[test]
fn ordinary_gc_defers_referenced_objects_until_the_horizon_is_clear() {
    let signer = key(4);
    let mut machine = DeletionStateMachine::new(OBJECT);
    machine
        .apply("alice", DeletionAction::Request { request_id: 3 }, &signer)
        .unwrap();

    assert_eq!(
        machine
            .gc_sweep("gc", "retention-v1", true, true, &signer)
            .unwrap(),
        TransitionOutcome::Deferred
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteRequested { request_id: 3 }
    ));
    assert_eq!(machine.evidence()[1].action, "gc_deferred");

    assert_eq!(
        machine
            .gc_sweep("gc", "retention-v1", false, true, &signer)
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::TrueDeletedOnThisServer { .. }
    ));
}

#[test]
fn batches_are_atomic_and_idempotent_and_replicas_are_local() {
    let signer = key(2);
    let mut one = DeletionStateMachine::new(OBJECT);
    let mut two = DeletionStateMachine::new(OBJECT);
    let batch = [
        DeletionAction::Request { request_id: 1 },
        DeletionAction::Queue {
            request_id: 1,
            ticket_id: 10,
        },
    ];

    assert_eq!(
        one.apply_batch(99, "alice", &batch, &signer).unwrap(),
        TransitionOutcome::Applied
    );
    let evidence_count = one.evidence().len();
    assert_eq!(
        one.apply_batch(99, "alice", &batch, &signer).unwrap(),
        TransitionOutcome::BatchAlreadyApplied
    );
    assert_eq!(one.evidence().len(), evidence_count);
    assert_eq!(two.state(), &DeletionState::Available);
    assert_eq!(two.evidence(), [] as [deletion::DeletionEvidence; 0]);

    let other = key(3);
    two.apply("bob", DeletionAction::Request { request_id: 2 }, &other)
        .unwrap();
    assert_eq!(one.evidence().len(), evidence_count);
    assert_eq!(two.evidence().len(), 1);
    two.evidence()[0].verify().unwrap();
}
