//! B31 golden test: core directional peer-class behavior and the
//! unknown-peer default (#10186, statements S1.1–S1.11).
//!
//! Oracle: golden — the policy matrix covers class × direction ×
//! operation and the unknown defaults, and `fully_trusted` never
//! bypasses the gate. The verifier is the independent reference
//! state machine below (`reference_decide`): it is written directly
//! from the settled design statements, as a literal transition
//! table, and shares no code with the implementation under test.
//! The implementation is loaded by direct path because slice
//! 9110-09 — not this slice — wires the versioned public exports
//! after the leaf modules land (9110-02 registry rule).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/federation/slices/b31_peer_classes.rs"]
mod b31;

use std::collections::BTreeSet;

use b31::{Decision, DenyReason, Direction, GateEvidence, Operation, PeerClass, PeerPolicy};

const NOW: u64 = 1_000;

fn policy(class: PeerClass, operations: &[Operation]) -> PeerPolicy {
    PeerPolicy {
        peer_id: "peer-1".to_string(),
        class,
        operations: operations.iter().copied().collect::<BTreeSet<_>>(),
        relay_granted: false,
        scope: "scope-1".to_string(),
        policy_version: 1,
        valid_from: None,
        expires_at: None,
    }
}

/// Independent reference state machine, from the design text:
///
/// - unknown / no policy / out-of-window -> deny everything;
/// - accept: send_only forbids; fully_trusted and accept_signed_from
///   accept only when accept is configured AND every gate check
///   passed (fully_trusted never bypasses);
/// - send: accept_signed_from forbids; fully_trusted and send_only
///   send only when send is configured;
/// - relay: only a separate relay grant allows it, class never does;
/// - completeness authority / cross-tenant: class never grants;
/// - an operation in the wrong direction is always denied.
fn reference_decide(
    policy: Option<&PeerPolicy>,
    operation: Operation,
    direction: Direction,
    gate: &GateEvidence,
    now: u64,
) -> Decision {
    let direction_ok = matches!(
        (operation, direction),
        (Operation::AcceptData, Direction::Inbound)
            | (Operation::SendData, Direction::Outbound)
            | (Operation::QueryRelay, Direction::Outbound)
            | (Operation::CompletenessAuthority, Direction::Inbound)
            | (Operation::CrossTenantAccess, Direction::Inbound)
    );
    if !direction_ok {
        return Decision::Deny(DenyReason::WrongDirection);
    }
    let Some(p) = policy else {
        return Decision::Deny(DenyReason::UnknownPeer);
    };
    if p.valid_from.is_some_and(|from| now < from)
        || p.expires_at.is_some_and(|until| now >= until)
        || p.class == PeerClass::Unknown
    {
        return Decision::Deny(DenyReason::UnknownPeer);
    }
    let gate_ok = gate.signature_valid
        && gate.signer_authorized
        && gate.semantically_valid
        && gate.scope_authorized;
    match (p.class, operation) {
        (_, Operation::CompletenessAuthority | Operation::CrossTenantAccess) => {
            Decision::Deny(DenyReason::NotGrantedByClass)
        }
        (_, Operation::QueryRelay) => {
            if p.relay_granted {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotGrantedByClass)
            }
        }
        (PeerClass::SendOnly, Operation::AcceptData) => Decision::Deny(DenyReason::ClassForbids),
        (PeerClass::AcceptSignedFrom, Operation::SendData) => {
            Decision::Deny(DenyReason::ClassForbids)
        }
        (PeerClass::FullyTrusted | PeerClass::AcceptSignedFrom, Operation::AcceptData) => {
            if !p.operations.contains(&Operation::AcceptData) {
                Decision::Deny(DenyReason::NotConfigured)
            } else if !gate_ok {
                Decision::Deny(DenyReason::GateNotPassed)
            } else {
                Decision::Allow
            }
        }
        (PeerClass::FullyTrusted | PeerClass::SendOnly, Operation::SendData) => {
            if p.operations.contains(&Operation::SendData) {
                Decision::Allow
            } else {
                Decision::Deny(DenyReason::NotConfigured)
            }
        }
        (PeerClass::Unknown, _) => Decision::Deny(DenyReason::UnknownPeer),
    }
}

const CLASSES: [PeerClass; 4] = [
    PeerClass::FullyTrusted,
    PeerClass::SendOnly,
    PeerClass::AcceptSignedFrom,
    PeerClass::Unknown,
];
const OPERATIONS: [Operation; 5] = [
    Operation::AcceptData,
    Operation::SendData,
    Operation::QueryRelay,
    Operation::CompletenessAuthority,
    Operation::CrossTenantAccess,
];
const DIRECTIONS: [Direction; 2] = [Direction::Inbound, Direction::Outbound];

fn gate_variants() -> Vec<GateEvidence> {
    let pass = GateEvidence::all_pass();
    vec![
        pass,
        GateEvidence::default(),
        GateEvidence {
            signature_valid: false,
            ..pass
        },
        GateEvidence {
            signer_authorized: false,
            ..pass
        },
        GateEvidence {
            semantically_valid: false,
            ..pass
        },
        GateEvidence {
            scope_authorized: false,
            ..pass
        },
    ]
}

#[test]
fn matrix_matches_reference_for_every_combination() {
    let configured_sets: [&[Operation]; 3] = [
        &[],
        &[Operation::AcceptData],
        &[Operation::AcceptData, Operation::SendData],
    ];
    for class in CLASSES {
        for configured in configured_sets {
            for relay_granted in [false, true] {
                let mut p = policy(class, configured);
                p.relay_granted = relay_granted;
                for operation in OPERATIONS {
                    for direction in DIRECTIONS {
                        for gate in gate_variants() {
                            let expected =
                                reference_decide(Some(&p), operation, direction, &gate, NOW);
                            let actual = b31::decide(Some(&p), operation, direction, &gate, NOW);
                            assert_eq!(
                                actual, expected,
                                "class={class:?} configured={configured:?} relay={relay_granted} \
                                 op={operation:?} dir={direction:?} gate={gate:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn no_policy_is_unknown_and_denies_everything() {
    for operation in OPERATIONS {
        for direction in DIRECTIONS {
            assert_eq!(
                b31::decide(None, operation, direction, &GateEvidence::all_pass(), NOW),
                if direction == operation.direction() {
                    Decision::Deny(DenyReason::UnknownPeer)
                } else {
                    Decision::Deny(DenyReason::WrongDirection)
                },
                "op={operation:?} dir={direction:?}"
            );
        }
    }
}

#[test]
fn fully_trusted_never_bypasses_the_gate() {
    let p = policy(
        PeerClass::FullyTrusted,
        &[Operation::AcceptData, Operation::SendData],
    );
    // Every single gate failure, and total gate failure, deny
    // acceptance even for a fully trusted, fully configured peer.
    for gate in gate_variants().into_iter().skip(1) {
        assert_eq!(
            b31::decide(
                Some(&p),
                Operation::AcceptData,
                Direction::Inbound,
                &gate,
                NOW
            ),
            Decision::Deny(DenyReason::GateNotPassed),
            "gate={gate:?}"
        );
    }
    assert_eq!(
        b31::decide(
            Some(&p),
            Operation::AcceptData,
            Direction::Inbound,
            &GateEvidence::all_pass(),
            NOW
        ),
        Decision::Allow
    );
}

#[test]
fn send_only_rejects_inbound_and_accept_signed_from_grants_no_send() {
    let sender = policy(
        PeerClass::SendOnly,
        &[Operation::AcceptData, Operation::SendData],
    );
    assert_eq!(
        b31::decide(
            Some(&sender),
            Operation::AcceptData,
            Direction::Inbound,
            &GateEvidence::all_pass(),
            NOW
        ),
        Decision::Deny(DenyReason::ClassForbids)
    );
    let accepter = policy(
        PeerClass::AcceptSignedFrom,
        &[Operation::AcceptData, Operation::SendData],
    );
    assert_eq!(
        b31::decide(
            Some(&accepter),
            Operation::SendData,
            Direction::Outbound,
            &GateEvidence::all_pass(),
            NOW
        ),
        Decision::Deny(DenyReason::ClassForbids)
    );
}

#[test]
fn no_class_grants_relay_completeness_or_cross_tenant_by_itself() {
    for class in CLASSES {
        let p = policy(
            class,
            &[
                Operation::AcceptData,
                Operation::SendData,
                Operation::QueryRelay,
            ],
        );
        // An unknown peer is denied as unknown, before the
        // not-granted-by-class reasons are even reached.
        let expected = if class == PeerClass::Unknown {
            DenyReason::UnknownPeer
        } else {
            DenyReason::NotGrantedByClass
        };
        assert_eq!(
            b31::decide(
                Some(&p),
                Operation::QueryRelay,
                Direction::Outbound,
                &GateEvidence::all_pass(),
                NOW
            ),
            Decision::Deny(expected),
            "class={class:?}"
        );
        for operation in [
            Operation::CompletenessAuthority,
            Operation::CrossTenantAccess,
        ] {
            assert_eq!(
                b31::decide(
                    Some(&p),
                    operation,
                    Direction::Inbound,
                    &GateEvidence::all_pass(),
                    NOW
                ),
                Decision::Deny(expected),
                "class={class:?} op={operation:?}"
            );
        }
    }
    // A separate relay grant is the only route to relay, and it does
    // not leak into the other never-by-class operations.
    let mut p = policy(PeerClass::FullyTrusted, &[Operation::QueryRelay]);
    p.relay_granted = true;
    assert_eq!(
        b31::decide(
            Some(&p),
            Operation::QueryRelay,
            Direction::Outbound,
            &GateEvidence::all_pass(),
            NOW
        ),
        Decision::Allow
    );
}

#[test]
fn policy_outside_its_validity_window_is_treated_as_unknown() {
    let mut expired = policy(
        PeerClass::FullyTrusted,
        &[Operation::AcceptData, Operation::SendData],
    );
    expired.expires_at = Some(NOW);
    assert_eq!(
        b31::decide(
            Some(&expired),
            Operation::SendData,
            Direction::Outbound,
            &GateEvidence::all_pass(),
            NOW
        ),
        Decision::Deny(DenyReason::UnknownPeer)
    );
    let mut future = expired.clone();
    future.expires_at = None;
    future.valid_from = Some(NOW + 1);
    assert_eq!(
        b31::decide(
            Some(&future),
            Operation::AcceptData,
            Direction::Inbound,
            &GateEvidence::all_pass(),
            NOW
        ),
        Decision::Deny(DenyReason::UnknownPeer)
    );
}
