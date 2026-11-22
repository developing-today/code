//! B06 invariant test: observable, recordable rejection events at the
//! universal signed-ingest gate (#10247, statements S1.19 and S1.25).
//!
//! Oracle: **invariant** — every rejected ingest has typed observable
//! evidence and no rejected payload reaches accepted state. The
//! reference is an unobserved B32 gate run in lockstep over generated
//! envelope streams: the observed gate must return exactly its outcome,
//! append exactly one event per refusal and none per admission, leave the
//! admitted set exactly as the bare gate does, and keep a gap-free hash
//! chain. Event fields are checked against values the test computes
//! itself (payload digests with `sha2` directly, key bindings from the
//! test's own table), not against the implementation.
//!
//! Both slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B06 reaches B32 as
//! `super::b32_ingest_gate`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b06_rejection_events.rs"]
mod b06;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use b06::{
    ChainError, GENESIS, KeyStatus, ObservedGate, REASON_CLASSES, RejectionEvent,
    RejectionExporter, Visibility, policy_digest, reason_class, verify_chain,
};
use b32_ingest_gate::{
    Envelope, FreshnessWindow, IngestGate, IngressPath, RejectReason, SignerAuthorization,
    sign_envelope,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const NOW: u64 = 10_000;
const FRESHNESS: FreshnessWindow = FreshnessWindow {
    max_age: 600,
    max_future_skew: 30,
};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn signer(identity: &str, seed: u8, scopes: &[&str]) -> SignerAuthorization {
    SignerAuthorization {
        identity: identity.to_owned(),
        verifying_key: key(seed).verifying_key().to_bytes(),
        scopes: scopes
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
    }
}

/// alice (seed 7) may write sync and query; bob (seed 9) only sync;
/// carol (seed 11) is unknown to the gate.
fn signers() -> Vec<SignerAuthorization> {
    vec![
        signer("alice", 7, &["sync", "query"]),
        signer("bob", 9, &["sync"]),
    ]
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// The test's own view of which identity each key is bound to.
fn expected_status(verifying_key: &[u8; 32]) -> KeyStatus {
    if *verifying_key == key(7).verifying_key().to_bytes() {
        KeyStatus::BoundTo("alice".into())
    } else if *verifying_key == key(9).verifying_key().to_bytes() {
        KeyStatus::BoundTo("bob".into())
    } else {
        KeyStatus::Unbound
    }
}

/// One generated envelope shape.
#[derive(Debug, Clone, Copy)]
enum Shape {
    Valid,
    BobValid,
    Unsigned,
    TamperedPayload,
    WrongClaimedSigner,
    UnknownKey,
    Unauthorized,
    Stale,
    Future,
    EmptyPayload,
    BadSignature,
}

const SHAPES: [Shape; 11] = [
    Shape::Valid,
    Shape::BobValid,
    Shape::Unsigned,
    Shape::TamperedPayload,
    Shape::WrongClaimedSigner,
    Shape::UnknownKey,
    Shape::Unauthorized,
    Shape::Stale,
    Shape::Future,
    Shape::EmptyPayload,
    Shape::BadSignature,
];

fn envelope(shape: Shape, id: u8) -> Envelope {
    let event_id = [id; 32];
    let payload = [b'p', id];
    match shape {
        Shape::Valid => sign_envelope(&key(7), "alice", "sync", &payload, event_id, NOW),
        Shape::BobValid => sign_envelope(&key(9), "bob", "sync", &payload, event_id, NOW),
        Shape::Unsigned => {
            let mut e = sign_envelope(&key(7), "alice", "sync", &payload, event_id, NOW);
            e.signature = None;
            e
        }
        Shape::TamperedPayload => {
            let mut e = sign_envelope(&key(7), "alice", "sync", &payload, event_id, NOW);
            e.payload = b"swapped".to_vec();
            e
        }
        Shape::WrongClaimedSigner => {
            sign_envelope(&key(9), "alice", "sync", &payload, event_id, NOW)
        }
        Shape::UnknownKey => sign_envelope(&key(11), "carol", "sync", &payload, event_id, NOW),
        Shape::Unauthorized => sign_envelope(&key(9), "bob", "query", &payload, event_id, NOW),
        Shape::Stale => sign_envelope(&key(7), "alice", "sync", &payload, event_id, NOW - 601),
        Shape::Future => sign_envelope(&key(7), "alice", "sync", &payload, event_id, NOW + 31),
        Shape::EmptyPayload => sign_envelope(&key(7), "alice", "sync", b"", event_id, NOW),
        Shape::BadSignature => {
            let mut e = sign_envelope(&key(7), "alice", "sync", &payload, event_id, NOW);
            let mut sig = e.signature.unwrap();
            sig[0] ^= 1;
            e.signature = Some(sig);
            e
        }
    }
}

/// An exporter that keeps what it is handed, shared with the test.
#[derive(Clone, Default)]
struct Capture(Rc<RefCell<Vec<RejectionEvent>>>);

impl RejectionExporter for Capture {
    fn export(&mut self, event: &RejectionEvent) {
        self.0.borrow_mut().push(event.clone());
    }
}

/// Run `ops` through an observed gate and a bare B32 gate in lockstep,
/// checking every invariant after every step.
fn check_stream(ops: &[(usize, usize, u8)], export_until: Option<usize>) {
    let mut observed = ObservedGate::new(signers(), FRESHNESS);
    let mut reference = IngestGate::new(signers(), FRESHNESS);
    let capture = Capture::default();
    if export_until.is_some() {
        observed.enable_export(Box::new(capture.clone()));
    }
    let mut exported_while_on = 0_usize;
    for (step, &(shape, path, id)) in ops.iter().enumerate() {
        if export_until == Some(step) {
            assert!(observed.disable_export().is_some());
        }
        let shape = SHAPES[shape % SHAPES.len()];
        let path = IngressPath::ALL[path % IngressPath::ALL.len()];
        let env = envelope(shape, id);
        let before_len = observed.log().len();
        let was_admitted = reference.is_admitted(&env.event_id);

        let want = reference.admit(path, &env, NOW);
        let got = observed.admit(path, &env, NOW);
        assert_eq!(got, want, "observation changed the outcome at step {step}");

        // The admitted set is exactly the bare gate's.
        assert_eq!(observed.gate().admitted_count(), reference.admitted_count());
        assert_eq!(
            observed.gate().is_admitted(&env.event_id),
            reference.is_admitted(&env.event_id)
        );

        match want {
            Ok(_) => assert_eq!(observed.log().len(), before_len, "admission was logged"),
            Err(reason) => {
                // A refusal never changes whether its event id is admitted.
                assert_eq!(reference.is_admitted(&env.event_id), was_admitted);
                assert_eq!(observed.log().len(), before_len + 1, "refusal not logged");
                let event = observed.log().events().last().unwrap();
                assert_eq!(event.sequence, before_len as u64);
                assert_eq!(event.observed_at, NOW);
                assert_eq!(event.path, path);
                assert_eq!(event.class(), reason_class(&reason));
                assert_eq!(event.reason, reason);
                assert_eq!(event.claimed_signer, env.signer);
                assert_eq!(event.verifying_key, env.verifying_key);
                assert_eq!(event.key_status, expected_status(&env.verifying_key));
                assert_eq!(event.signed, env.signature.is_some());
                assert_eq!(event.scope, env.scope);
                assert_eq!(event.event_id, env.event_id);
                assert_eq!(event.claimed_issued_at, env.issued_at);
                assert_eq!(event.declared_digest, env.payload_digest);
                assert_eq!(event.presented_digest, sha(&env.payload));
                assert_eq!(event.policy_digest, observed.policy_digest());
                assert_eq!(event.visibility, Visibility::LocalOnly);
                if export_until.is_some_and(|until| step < until) {
                    exported_while_on += 1;
                }
            }
        }
        observed.log().verify_chain().unwrap();
    }

    // Counters tally the log, with every class present.
    let counts = observed.log().counts();
    assert_eq!(counts.len(), REASON_CLASSES.len());
    for class in REASON_CLASSES {
        let tally = observed
            .log()
            .events()
            .iter()
            .filter(|e| e.class() == class)
            .count() as u64;
        assert_eq!(counts[class], tally);
    }

    // The exporter saw exactly the events recorded while it was on.
    let exported = capture.0.borrow();
    assert_eq!(exported.len(), exported_while_on);
    assert_eq!(
        exported.as_slice(),
        &observed.log().events()[..exported_while_on]
    );
}

#[test]
fn every_rejection_class_is_observed_with_its_evidence() {
    let ops: Vec<(usize, usize, u8)> = (0..SHAPES.len())
        .map(|i| (i, i, 100 + u8::try_from(i).unwrap()))
        .collect();
    check_stream(&ops, None);
    let mut gate = ObservedGate::new(signers(), FRESHNESS);
    for (shape, path, id) in &ops {
        let _ = gate.admit(
            IngressPath::ALL[*path % 5],
            &envelope(SHAPES[*shape], *id),
            NOW,
        );
    }
    // Replay: the first valid envelope again.
    let replay = envelope(Shape::Valid, 100);
    assert_eq!(
        gate.admit(IngressPath::Remote, &replay, NOW),
        Err(RejectReason::Replay)
    );
    let classes: BTreeSet<&str> = gate
        .log()
        .events()
        .iter()
        .map(RejectionEvent::class)
        .collect();
    assert_eq!(classes, REASON_CLASSES.into_iter().collect());
}

#[test]
fn local_and_remote_refusals_are_recorded_alike() {
    let mut gate = ObservedGate::new(signers(), FRESHNESS);
    for path in IngressPath::ALL {
        assert_eq!(
            gate.admit(path, &envelope(Shape::Unsigned, 1), NOW),
            Err(RejectReason::Unsigned)
        );
    }
    let paths: Vec<IngressPath> = gate.log().events().iter().map(|e| e.path).collect();
    assert_eq!(paths, IngressPath::ALL.to_vec());
    assert!(gate.log().events().iter().all(|e| e.class() == "unsigned"));
}

#[test]
fn misattributed_key_records_who_the_key_is_really_bound_to() {
    let mut gate = ObservedGate::new(signers(), FRESHNESS);
    let env = envelope(Shape::WrongClaimedSigner, 3);
    assert_eq!(
        gate.admit(IngressPath::Remote, &env, NOW),
        Err(RejectReason::SignerMismatch {
            claimed: "alice".into(),
            bound_to: "bob".into()
        })
    );
    let event = &gate.log().events()[0];
    assert_eq!(event.claimed_signer, "alice");
    assert_eq!(event.key_status, KeyStatus::BoundTo("bob".into()));
    let unknown = envelope(Shape::UnknownKey, 4);
    let _ = gate.admit(IngressPath::Remote, &unknown, NOW);
    assert_eq!(gate.log().events()[1].key_status, KeyStatus::Unbound);
}

#[test]
fn tampering_with_the_record_is_detected() {
    let mut gate = ObservedGate::new(signers(), FRESHNESS);
    for (i, shape) in [Shape::Unsigned, Shape::Stale, Shape::UnknownKey]
        .into_iter()
        .enumerate()
    {
        let _ = gate.admit(
            IngressPath::Remote,
            &envelope(shape, u8::try_from(i).unwrap()),
            NOW,
        );
    }
    let events = gate.log().events().to_vec();
    assert_eq!(events[0].prev, GENESIS);
    verify_chain(&events).unwrap();

    let mut edited = events.clone();
    edited[1].scope = "query".into();
    assert_eq!(verify_chain(&edited), Err(ChainError::Link(2)));

    let mut dropped = events.clone();
    dropped.remove(1);
    assert_eq!(verify_chain(&dropped), Err(ChainError::Sequence(1)));

    let mut reordered = events.clone();
    reordered.swap(0, 1);
    assert_eq!(verify_chain(&reordered), Err(ChainError::Sequence(0)));

    let mut renumbered = events;
    renumbered.remove(0);
    for (i, event) in renumbered.iter_mut().enumerate() {
        event.sequence = i as u64;
    }
    assert_eq!(verify_chain(&renumbered), Err(ChainError::Link(0)));
}

#[test]
fn policy_digest_names_the_policy_not_the_table_order() {
    let mut reversed = signers();
    reversed.reverse();
    assert_eq!(
        policy_digest(&signers(), FRESHNESS),
        policy_digest(&reversed, FRESHNESS)
    );
    let wider = FreshnessWindow {
        max_age: 601,
        ..FRESHNESS
    };
    assert_ne!(
        policy_digest(&signers(), FRESHNESS),
        policy_digest(&signers(), wider)
    );
    let mut more_scope = signers();
    more_scope[1].scopes.insert("query".into());
    assert_ne!(
        policy_digest(&signers(), FRESHNESS),
        policy_digest(&more_scope, FRESHNESS)
    );
}

#[test]
fn disabling_export_keeps_rejection_fail_closed_and_recorded() {
    let ops: Vec<(usize, usize, u8)> = (0..40)
        .map(|i| (i % SHAPES.len(), i, u8::try_from(i % 7).unwrap()))
        .collect();
    check_stream(&ops, Some(15));
    let mut gate = ObservedGate::new(signers(), FRESHNESS);
    assert!(gate.log().is_empty());
    assert!(!gate.export_enabled());
    gate.enable_export(Box::new(Capture::default()));
    assert!(gate.export_enabled());
    gate.disable_export();
    assert_eq!(
        gate.admit(IngressPath::Local, &envelope(Shape::Unsigned, 9), NOW),
        Err(RejectReason::Unsigned)
    );
    assert_eq!(gate.log().len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn observed_gate_matches_the_bare_gate_and_records_every_refusal(
        ops in proptest::collection::vec((0..SHAPES.len(), 0..5_usize, 0_u8..6), 0..48),
        export_switch in proptest::option::of(0_usize..48),
    ) {
        check_stream(&ops, export_switch);
    }
}
