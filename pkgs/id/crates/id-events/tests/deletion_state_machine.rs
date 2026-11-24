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
            .apply(
                "alice",
                DeletionAction::Request {
                    request_id: 7,
                    reason: "doxxing content".to_owned(),
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteRequested { request_id: 7, .. }
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
            .apply(
                "alice",
                DeletionAction::Request {
                    request_id: 8,
                    reason: "second request".to_owned(),
                },
                &signer
            )
            .unwrap_err(),
        deletion::DeletionError::AlreadyDeleted
    );
}

#[test]
fn ordinary_gc_defers_referenced_objects_until_the_horizon_is_clear() {
    let signer = key(4);
    let mut machine = DeletionStateMachine::new(OBJECT);
    machine
        .apply(
            "alice",
            DeletionAction::Request {
                request_id: 3,
                reason: "stale draft".to_owned(),
            },
            &signer,
        )
        .unwrap();

    assert_eq!(
        machine
            .gc_sweep("gc", "retention-v1", true, true, &signer)
            .unwrap(),
        TransitionOutcome::Deferred
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteRequested { request_id: 3, .. }
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
        DeletionAction::Request {
            request_id: 1,
            reason: "batched request".to_owned(),
        },
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
    assert!(two.evidence().is_empty());

    let other = key(3);
    two.apply(
        "bob",
        DeletionAction::Request {
            request_id: 2,
            reason: "replica request".to_owned(),
        },
        &other,
    )
    .unwrap();
    assert_eq!(one.evidence().len(), evidence_count);
    assert_eq!(two.evidence().len(), 1);
    two.evidence()[0].verify().unwrap();
}

#[test]
fn request_deliver_grant_produces_a_policy_delete() {
    let signer = key(5);
    let mut machine = DeletionStateMachine::new(OBJECT);
    assert_eq!(
        machine
            .apply(
                "alice",
                DeletionAction::Request {
                    request_id: 11,
                    reason: "non-consensual imagery".to_owned(),
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    // A grant before delivery is not a valid transition.
    assert!(matches!(
        machine.apply(
            "author",
            DeletionAction::Grant {
                request_id: 11,
                policy_id: "legal-removal-v1".to_owned(),
            },
            &signer
        ),
        Err(deletion::DeletionError::InvalidTransition(_))
    ));
    assert_eq!(
        machine
            .apply(
                "moderator",
                DeletionAction::Deliver {
                    request_id: 11,
                    recipients: vec!["author".to_owned(), "group-admin".to_owned()],
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::DeleteRequestDelivered { request_id: 11, .. }
    ));
    assert_eq!(machine.evidence()[1].action, "delete_request_delivered");
    assert_eq!(
        machine
            .apply(
                "author",
                DeletionAction::Grant {
                    request_id: 11,
                    policy_id: "legal-removal-v1".to_owned(),
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert!(matches!(
        machine.state(),
        DeletionState::TrueDeletedOnThisServer { .. }
    ));
    assert_eq!(machine.evidence().len(), 3);
    assert_eq!(machine.evidence()[0].action, "delete_requested");
    assert_eq!(machine.evidence()[0].rationale, "non-consensual imagery");
    assert_eq!(machine.evidence()[2].action, "delete_granted");
    assert!(
        machine.evidence()[2]
            .rationale
            .contains("policy=legal-removal-v1")
    );
    for evidence in machine.evidence() {
        evidence.verify().unwrap();
    }
}

#[test]
fn request_deliver_decline_records_without_a_delete() {
    let signer = key(6);
    let mut machine = DeletionStateMachine::new(OBJECT);
    machine
        .apply(
            "alice",
            DeletionAction::Request {
                request_id: 12,
                reason: "looks off-topic".to_owned(),
            },
            &signer,
        )
        .unwrap();
    machine
        .apply(
            "moderator",
            DeletionAction::Deliver {
                request_id: 12,
                recipients: vec!["server-owner".to_owned()],
            },
            &signer,
        )
        .unwrap();
    assert_eq!(
        machine
            .apply(
                "server-owner",
                DeletionAction::Decline {
                    request_id: 12,
                    reason: "on-topic for this group".to_owned(),
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
    assert_eq!(machine.state(), &DeletionState::Available);
    assert_eq!(machine.evidence().len(), 3);
    assert_eq!(machine.evidence()[2].action, "delete_declined");
    assert_eq!(machine.evidence()[2].rationale, "on-topic for this group");
    for evidence in machine.evidence() {
        evidence.verify().unwrap();
    }
    // Repeating the decline is idempotent; the object stays available and
    // a fresh request can be filed afterwards.
    assert_eq!(
        machine
            .apply(
                "server-owner",
                DeletionAction::Decline {
                    request_id: 12,
                    reason: "on-topic for this group".to_owned(),
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Idempotent
    );
    assert_eq!(machine.evidence().len(), 3);
    assert_eq!(
        machine
            .apply(
                "alice",
                DeletionAction::Request {
                    request_id: 13,
                    reason: "new evidence".to_owned(),
                },
                &signer
            )
            .unwrap(),
        TransitionOutcome::Applied
    );
}
