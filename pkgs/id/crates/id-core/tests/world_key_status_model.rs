//! Model-oracle tests for the signer key-history state machine
//! (slice A9124-world-key-history, issue #10283; design A-9124.3).
//!
//! Oracle: **model** — explicit key history state machine; cryptographic
//! validity, currentness and verifier trust are independent outputs.
//! The proptest at the end generates the listed operations (introduce,
//! rotate, revoke — including wrong authors, out-of-order sequences and
//! tampered signatures) against an independent reference model written
//! over plain values; every resulting state and error class must match.
//! Targeted tests pin the design's named guarantees: unknown key status
//! is never valid, revocation and compromise bound historical validity
//! without erasing it, rotation is a signed successor event, revocation
//! never silently rebinds an old key, and trust is an independent
//! dimension.
//!
//! The crate-root export belongs to the separate crate-wiring slice, so
//! this test includes the leaf modules by path, as the neighboring ID
//! slices do.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../src/world_key_status.rs"]
mod world_key_status;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use world_key_status::{
    HistoricalValidity, KEY_HISTORY_DOMAIN, KeyEventKind, KeyHistory, KeyHistoryEvent, KeyStatus,
    RevocationReason, SignedKeyHistoryEvent, TrustPolicyResult,
};

const SIGNER: &str = "signer-1";
const KEY_IDS: [&str; 4] = ["key-a", "key-b", "key-c", "key-d"];
const KEY_SEEDS: [u8; 4] = [11, 22, 33, 44];

fn signing(index: usize) -> SigningKey {
    SigningKey::from_bytes(&[KEY_SEEDS[index]; 32])
}

fn public(index: usize) -> [u8; 32] {
    signing(index).verifying_key().to_bytes()
}

fn event(
    kind: KeyEventKind,
    key: usize,
    successor: Option<usize>,
    reason: Option<RevocationReason>,
    effective_at: u64,
    sequence: u64,
) -> KeyHistoryEvent {
    KeyHistoryEvent {
        signer_id: SIGNER.to_owned(),
        signer_key_id: KEY_IDS[key].to_owned(),
        kind,
        key: public(key),
        successor_key_id: successor.map(|index| KEY_IDS[index].to_owned()),
        successor_key: successor.map(public),
        reason,
        effective_at,
        sequence,
    }
}

fn signed(
    kind: KeyEventKind,
    key: usize,
    successor: Option<usize>,
    reason: Option<RevocationReason>,
    effective_at: u64,
    sequence: u64,
    author: usize,
) -> SignedKeyHistoryEvent {
    SignedKeyHistoryEvent::sign(
        event(kind, key, successor, reason, effective_at, sequence),
        &signing(author),
    )
    .expect("generated events are well-formed")
}

fn introduce(key: usize, at: u64, sequence: u64) -> SignedKeyHistoryEvent {
    signed(KeyEventKind::Introduced, key, None, None, at, sequence, key)
}

fn rotate(key: usize, successor: usize, at: u64, sequence: u64) -> SignedKeyHistoryEvent {
    signed(
        KeyEventKind::Rotated,
        key,
        Some(successor),
        None,
        at,
        sequence,
        key,
    )
}

fn revoke(
    key: usize,
    reason: RevocationReason,
    at: u64,
    sequence: u64,
    author: usize,
) -> SignedKeyHistoryEvent {
    signed(
        KeyEventKind::Revoked,
        key,
        None,
        Some(reason),
        at,
        sequence,
        author,
    )
}

#[test]
fn unknown_key_is_never_valid() {
    let history = KeyHistory::new();
    let report = history.evaluate("key-a", 5, TrustPolicyResult::Trusted);
    assert_eq!(report.key_status, KeyStatus::Unknown);
    assert!(!report.historical_signature_valid);
    assert!(!report.currently_authorized);
    // The trust dimension is the verifier's own; it manufactures nothing.
    assert_eq!(report.trust_policy_result, TrustPolicyResult::Trusted);
    assert_eq!(
        history.historical_validity("key-a", 5),
        HistoricalValidity::UnknownKey
    );
    assert_eq!(history.current_key_id(), None);
}

#[test]
fn introduction_makes_the_key_current() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    assert_eq!(history.signer_id(), Some(SIGNER));
    assert_eq!(history.status_of("key-a"), KeyStatus::Active);
    assert_eq!(history.current_key_id(), Some("key-a"));
    assert_eq!(
        history.historical_validity("key-a", 9),
        HistoricalValidity::OutsideWindow
    );
    assert_eq!(
        history.historical_validity("key-a", 10),
        HistoricalValidity::Valid
    );
    let report = history.evaluate("key-a", 10, TrustPolicyResult::Untrusted);
    assert!(report.historical_signature_valid);
    assert!(report.currently_authorized);
}

#[test]
fn rotation_moves_currentness_but_keeps_history() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    history
        .apply(&rotate(0, 1, 20, 2))
        .expect("rotation applies");
    assert_eq!(history.status_of("key-a"), KeyStatus::Rotated);
    assert_eq!(history.status_of("key-b"), KeyStatus::Active);
    assert_eq!(history.current_key_id(), Some("key-b"));
    assert_eq!(history.successor_of("key-a"), Some("key-b"));
    // Signatures from before the rotation remain historically valid.
    assert_eq!(
        history.historical_validity("key-a", 19),
        HistoricalValidity::Valid
    );
    assert_eq!(
        history.historical_validity("key-a", 20),
        HistoricalValidity::OutsideWindow
    );
    let report = history.evaluate("key-a", 15, TrustPolicyResult::Trusted);
    assert!(report.historical_signature_valid);
    assert!(!report.currently_authorized);
}

#[test]
fn retirement_revocation_preserves_earlier_history() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    history
        .apply(&revoke(0, RevocationReason::Retired, 30, 2, 0))
        .expect("revocation applies");
    assert_eq!(history.status_of("key-a"), KeyStatus::Revoked);
    assert_eq!(history.current_key_id(), None);
    assert_eq!(
        history.historical_validity("key-a", 29),
        HistoricalValidity::Valid
    );
    assert_eq!(
        history.historical_validity("key-a", 30),
        HistoricalValidity::OutsideWindow
    );
}

#[test]
fn compromise_backdating_bounds_historical_validity() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    history
        .apply(&rotate(0, 1, 20, 2))
        .expect("rotation applies");
    // The successor reports the old key compromised from time 15, before
    // the rotation was recorded: history from 15 onward is invalidated,
    // history before 15 stands.
    history
        .apply(&revoke(0, RevocationReason::Compromised, 15, 3, 1))
        .expect("successor may report the compromise");
    assert_eq!(history.status_of("key-a"), KeyStatus::Compromised);
    assert_eq!(
        history.historical_validity("key-a", 14),
        HistoricalValidity::Valid
    );
    assert_eq!(
        history.historical_validity("key-a", 15),
        HistoricalValidity::OutsideWindow
    );
    assert_eq!(history.current_key_id(), Some("key-b"));
}

#[test]
fn revocation_does_not_silently_rebind_the_old_key() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    history
        .apply(&rotate(0, 1, 20, 2))
        .expect("rotation applies");
    history
        .apply(&revoke(1, RevocationReason::Retired, 30, 3, 1))
        .expect("revocation applies");
    // Key-a still reports its own status and window; the successor link
    // is explicit, and key-b's revocation changes nothing about key-a.
    let report = history.evaluate("key-a", 15, TrustPolicyResult::Trusted);
    assert_eq!(report.key_status, KeyStatus::Rotated);
    assert!(report.historical_signature_valid);
    assert_eq!(history.successor_of("key-a"), Some("key-b"));
    assert_eq!(history.status_of("key-b"), KeyStatus::Revoked);
    assert_eq!(history.current_key_id(), None);
}

#[test]
fn trust_is_an_independent_dimension() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    history
        .apply(&rotate(0, 1, 20, 2))
        .expect("rotation applies");
    let trusted = history.evaluate("key-a", 15, TrustPolicyResult::Trusted);
    let untrusted = history.evaluate("key-a", 15, TrustPolicyResult::Untrusted);
    assert_eq!(trusted.key_status, untrusted.key_status);
    assert_eq!(
        trusted.historical_signature_valid,
        untrusted.historical_signature_valid
    );
    assert_eq!(trusted.currently_authorized, untrusted.currently_authorized);
    assert_ne!(trusted.trust_policy_result, untrusted.trust_policy_result);
}

#[test]
fn tampered_and_wrong_domain_events_are_rejected() {
    let mut history = KeyHistory::new();
    let mut tampered = introduce(0, 10, 1);
    tampered.signature[0] ^= 1;
    let error = history
        .apply(&tampered)
        .expect_err("tampered event is rejected");
    assert_eq!(error.class(), "bad_signature");
    assert_eq!(history.status_of("key-a"), KeyStatus::Unknown);
    assert_eq!(history.next_sequence(), 1);

    let mut wrong_domain = introduce(0, 10, 1);
    wrong_domain.set_domain_for_test("id-world-key-history-v2");
    let error = history
        .apply(&wrong_domain)
        .expect_err("wrong-domain event is rejected");
    assert_eq!(error.class(), "bad_signature");
}

#[test]
fn ordering_duplicates_and_terminal_rules_are_enforced() {
    let mut history = KeyHistory::new();
    let error = history
        .apply(&introduce(0, 10, 2))
        .expect_err("sequence gap is rejected");
    assert_eq!(error.class(), "out_of_order");

    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");
    let error = history
        .apply(&introduce(0, 12, 2))
        .expect_err("second introduction is rejected");
    assert_eq!(error.class(), "duplicate_key");

    let error = history
        .apply(&rotate(2, 3, 12, 2))
        .expect_err("rotation of an unknown key is rejected");
    assert_eq!(error.class(), "unknown_key");

    let wrong_author = signed(KeyEventKind::Introduced, 1, None, None, 12, 2, 2);
    let error = history
        .apply(&wrong_author)
        .expect_err("introduction signed by another key is rejected");
    assert_eq!(error.class(), "signer_mismatch");

    history
        .apply(&rotate(0, 1, 20, 2))
        .expect("rotation applies");
    let error = history
        .apply(&rotate(0, 2, 25, 3))
        .expect_err("rotating a rotated key is rejected");
    assert_eq!(error.class(), "key_not_active");

    history
        .apply(&revoke(1, RevocationReason::Retired, 30, 3, 1))
        .expect("revocation applies");
    let error = history
        .apply(&revoke(1, RevocationReason::Compromised, 31, 4, 1))
        .expect_err("second revocation is rejected");
    assert_eq!(error.class(), "already_revoked");
}

#[test]
fn signer_binding_and_revocation_window_are_enforced() {
    let mut history = KeyHistory::new();
    history
        .apply(&introduce(0, 10, 1))
        .expect("introduction applies");

    let mut foreign = introduce(1, 12, 2);
    foreign.event.signer_id = "signer-2".to_owned();
    let foreign = SignedKeyHistoryEvent::sign(foreign.event.clone(), &signing(1))
        .expect("foreign event is well-formed");
    let error = history
        .apply(&foreign)
        .expect_err("an event for another signer is rejected");
    assert_eq!(error.class(), "signer_mismatch");

    let error = history
        .apply(&revoke(0, RevocationReason::Retired, 5, 2, 0))
        .expect_err("revocation before introduction is rejected");
    assert_eq!(error.class(), "malformed");
}

#[test]
fn signed_events_round_trip_through_canonical_cbor() {
    let events = [
        introduce(0, 10, 1),
        rotate(0, 1, 20, 2),
        revoke(0, RevocationReason::Compromised, 15, 3, 1),
    ];
    let mut direct = KeyHistory::new();
    let mut decoded = KeyHistory::new();
    for event in &events {
        direct.apply(event).expect("direct event applies");
        let bytes = event.encode().expect("event encodes");
        let parsed = SignedKeyHistoryEvent::decode(&bytes).expect("event decodes");
        assert_eq!(parsed.domain(), KEY_HISTORY_DOMAIN);
        assert_eq!(&parsed, event);
        decoded.apply(&parsed).expect("decoded event applies");
    }
    assert_eq!(direct, decoded);
    assert_eq!(decoded.status_of("key-a"), KeyStatus::Compromised);
}

// ---------------------------------------------------------------------------
// Reference model: the same state machine over plain values, written
// independently of the module under test.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    Introduce {
        key: usize,
        at: u64,
        sequence: u64,
        author: usize,
        tamper: bool,
    },
    Rotate {
        key: usize,
        successor: usize,
        at: u64,
        sequence: u64,
        author: usize,
        tamper: bool,
    },
    Revoke {
        key: usize,
        compromised: bool,
        at: u64,
        sequence: u64,
        author: usize,
        tamper: bool,
    },
}

#[derive(Clone, Debug, Default)]
struct ModelKey {
    introduced_at: u64,
    rotated_at: Option<u64>,
    successor: Option<usize>,
    revoked: Option<(u64, bool)>,
}

#[derive(Clone, Debug, Default)]
struct Model {
    keys: BTreeMap<usize, ModelKey>,
    last_sequence: u64,
    chain_at: u64,
}

impl Model {
    fn apply(&mut self, op: &Op) -> Result<(), &'static str> {
        let (sequence, tamper) = match *op {
            Op::Introduce {
                sequence, tamper, ..
            }
            | Op::Rotate {
                sequence, tamper, ..
            }
            | Op::Revoke {
                sequence, tamper, ..
            } => (sequence, tamper),
        };
        if tamper {
            return Err("bad_signature");
        }
        if sequence != self.last_sequence + 1 {
            return Err("out_of_order");
        }
        match *op {
            Op::Introduce {
                key, at, author, ..
            } => {
                if self.keys.contains_key(&key) {
                    return Err("duplicate_key");
                }
                if author != key {
                    return Err("signer_mismatch");
                }
                if at < self.chain_at {
                    return Err("malformed");
                }
                self.keys.insert(
                    key,
                    ModelKey {
                        introduced_at: at,
                        ..ModelKey::default()
                    },
                );
                self.chain_at = at;
            }
            Op::Rotate {
                key,
                successor,
                at,
                author,
                ..
            } => {
                let record = self.keys.get(&key).ok_or("unknown_key")?;
                if record.rotated_at.is_some() || record.revoked.is_some() {
                    return Err("key_not_active");
                }
                if author != key {
                    return Err("signer_mismatch");
                }
                if at < self.chain_at {
                    return Err("malformed");
                }
                if self.keys.contains_key(&successor) {
                    return Err("duplicate_key");
                }
                let record = self.keys.get_mut(&key).expect("record checked above");
                record.rotated_at = Some(at);
                record.successor = Some(successor);
                self.keys.insert(
                    successor,
                    ModelKey {
                        introduced_at: at,
                        ..ModelKey::default()
                    },
                );
                self.chain_at = at;
            }
            Op::Revoke {
                key,
                compromised,
                at,
                author,
                ..
            } => {
                let record = self.keys.get(&key).ok_or("unknown_key")?;
                if record.revoked.is_some() {
                    return Err("already_revoked");
                }
                let successor = record.successor;
                if author != key && successor != Some(author) {
                    return Err("signer_mismatch");
                }
                if at < record.introduced_at {
                    return Err("malformed");
                }
                let record = self.keys.get_mut(&key).expect("record checked above");
                record.revoked = Some((at, compromised));
            }
        }
        self.last_sequence = sequence;
        Ok(())
    }

    fn status(&self, key: usize) -> &'static str {
        let Some(record) = self.keys.get(&key) else {
            return "unknown";
        };
        match record.revoked {
            Some((_, true)) => "compromised",
            Some((_, false)) => "revoked",
            None if record.rotated_at.is_some() => "rotated",
            None => "active",
        }
    }

    fn snapshot(&self) -> Vec<(String, &'static str, Option<String>)> {
        let mut rows: Vec<(String, &'static str, Option<String>)> = KEY_IDS
            .iter()
            .enumerate()
            .map(|(index, id)| {
                (
                    (*id).to_owned(),
                    self.status(index),
                    self.keys
                        .get(&index)
                        .and_then(|record| record.successor)
                        .map(|successor| KEY_IDS[successor].to_owned()),
                )
            })
            .collect();
        let active: Vec<usize> = (0..KEY_IDS.len())
            .filter(|index| self.status(*index) == "active")
            .collect();
        let current = if active.len() == 1 {
            Some(KEY_IDS[active[0]].to_owned())
        } else {
            None
        };
        rows.push(("current".to_owned(), "", current));
        rows
    }
}

fn module_snapshot(history: &KeyHistory) -> Vec<(String, &'static str, Option<String>)> {
    let mut rows: Vec<(String, &'static str, Option<String>)> = KEY_IDS
        .iter()
        .map(|id| {
            (
                (*id).to_owned(),
                history.status_of(id).token(),
                history.successor_of(id).map(str::to_owned),
            )
        })
        .collect();
    rows.push((
        "current".to_owned(),
        "",
        history.current_key_id().map(str::to_owned),
    ));
    rows
}

fn build(op: &Op) -> SignedKeyHistoryEvent {
    let built = match *op {
        Op::Introduce {
            key,
            at,
            sequence,
            author,
            ..
        } => signed(
            KeyEventKind::Introduced,
            key,
            None,
            None,
            at,
            sequence,
            author,
        ),
        Op::Rotate {
            key,
            successor,
            at,
            sequence,
            author,
            ..
        } => signed(
            KeyEventKind::Rotated,
            key,
            Some(successor),
            None,
            at,
            sequence,
            author,
        ),
        Op::Revoke {
            key,
            compromised,
            at,
            sequence,
            author,
            ..
        } => signed(
            KeyEventKind::Revoked,
            key,
            None,
            Some(if compromised {
                RevocationReason::Compromised
            } else {
                RevocationReason::Retired
            }),
            at,
            sequence,
            author,
        ),
    };
    let tamper = match *op {
        Op::Introduce { tamper, .. } | Op::Rotate { tamper, .. } | Op::Revoke { tamper, .. } => {
            tamper
        }
    };
    if tamper {
        let mut built = built;
        built.signature[0] ^= 1;
        built
    } else {
        built
    }
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let base = (
        0usize..4,
        0usize..4,
        0usize..4,
        1u64..=12,
        1u64..=8,
        any::<bool>(),
        any::<bool>(),
    );
    prop_oneof![
        base.clone()
            .prop_map(|(key, _, author, at, sequence, _, tamper)| {
                Op::Introduce {
                    key,
                    at,
                    sequence,
                    author,
                    tamper,
                }
            }),
        base.clone()
            .prop_map(|(key, raw_successor, author, at, sequence, _, tamper)| {
                let successor = if raw_successor == key {
                    (key + 1) % KEY_IDS.len()
                } else {
                    raw_successor
                };
                Op::Rotate {
                    key,
                    successor,
                    at,
                    sequence,
                    author,
                    tamper,
                }
            }),
        base.prop_map(
            |(key, _, author, at, sequence, compromised, tamper)| Op::Revoke {
                key,
                compromised,
                at,
                sequence,
                author,
                tamper,
            }
        ),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn generated_histories_match_the_reference_model(
        ops in prop::collection::vec(op_strategy(), 0..20)
    ) {
        let mut history = KeyHistory::new();
        let mut model = Model::default();
        for op in &ops {
            let expected = model.apply(op);
            let actual = history.apply(&build(op)).map_err(|error| error.class());
            prop_assert_eq!(actual, expected, "op {:?} diverged", op);
            prop_assert_eq!(
                module_snapshot(&history),
                model.snapshot(),
                "state diverged after op {:?}",
                op
            );
        }
    }
}
