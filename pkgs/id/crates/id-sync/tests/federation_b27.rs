//! B27 golden test: the public signed append-only claims/disputes log
//! (#10231, statements S1.92-S1.98).
//!
//! Oracle: **golden** — claims and replies are signed append-only with
//! equal authentication, and a unilateral claim never changes global
//! score. The verifier is the independent reference below: it rebuilds
//! every canonical preimage by hand from the oracle text, verifies with
//! dalek directly, and keeps its own ledger whose score must stay zero
//! no matter how many unilateral claims are appended. It shares no
//! acceptance code with the implementation under test.
//!
//! The implementation is loaded by direct path because slice 9110-09 —
//! not this slice — wires the versioned public exports after the leaf
//! modules land (9110-02 registry rule).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/federation/slices/b05_signer_binding.rs"]
#[allow(dead_code)]
mod b05_signer_binding;

#[path = "../src/federation/slices/b27_claim_log.rs"]
mod b27_claim_log;

use b05_signer_binding::BindingRegistry;
use b27_claim_log::{
    CLAIM_DOMAIN, ClaimError, ClaimLog, EventBody, EventId, EventKind, MAX_STATEMENT_BYTES,
    sign_event,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use proptest::prelude::*;

const CLAIMANT_SEED: u8 = 0x27;
const SUBJECT_SEED: u8 = 0x72;
const RELAY_SEED: u8 = 0xB7;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn id(byte: u8) -> EventId {
    [byte; 32]
}

fn registry() -> BindingRegistry {
    let mut registry = BindingRegistry::new();
    registry
        .bind(
            "server:claimant",
            key(CLAIMANT_SEED).verifying_key().to_bytes(),
        )
        .unwrap();
    registry
        .bind(
            "server:subject",
            key(SUBJECT_SEED).verifying_key().to_bytes(),
        )
        .unwrap();
    registry
        .bind("server:relay", key(RELAY_SEED).verifying_key().to_bytes())
        .unwrap();
    registry
}

fn claim_body(event: u8, statement: &[u8]) -> EventBody {
    EventBody {
        event_id: id(event),
        kind: EventKind::Claim,
        author: "server:claimant".to_owned(),
        author_key: key(CLAIMANT_SEED).verifying_key().to_bytes(),
        subject: "server:subject".to_owned(),
        in_reply_to: None,
        statement: statement.to_vec(),
        issued_at: 1_000,
        policy_digest: [0x90; 32],
        expires_at: 2_000,
    }
}

fn reply_body(event: u8, target: EventId) -> EventBody {
    EventBody {
        event_id: id(event),
        kind: EventKind::Reply,
        author: "server:subject".to_owned(),
        author_key: key(SUBJECT_SEED).verifying_key().to_bytes(),
        subject: "server:subject".to_owned(),
        in_reply_to: Some(target),
        statement: b"that never happened; here are my receipts".to_vec(),
        issued_at: 1_010,
        policy_digest: [0x90; 32],
        expires_at: 2_000,
    }
}

/// The oracle text, rebuilt by hand: domain, fixed fields, tag, then
/// length-prefixed strings. Any drift between this and the
/// implementation's canonical bytes fails the golden vectors below.
fn reference_preimage(body: &EventBody) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    let mut out = CLAIM_DOMAIN.to_vec();
    out.extend_from_slice(&body.event_id);
    match body.kind {
        EventKind::Claim => {
            out.push(0x01);
            assert!(body.in_reply_to.is_none());
        }
        EventKind::Reply => {
            out.push(0x02);
            out.extend_from_slice(&body.in_reply_to.unwrap());
        }
    }
    field(&mut out, body.author.as_bytes());
    out.extend_from_slice(&body.author_key);
    field(&mut out, body.subject.as_bytes());
    field(&mut out, &body.statement);
    out.extend_from_slice(&body.issued_at.to_be_bytes());
    out.extend_from_slice(&body.policy_digest);
    out.extend_from_slice(&body.expires_at.to_be_bytes());
    out
}

fn verify_reference(body: &EventBody, signature: &[u8; 64]) {
    let key = VerifyingKey::from_bytes(&body.author_key).unwrap();
    key.verify_strict(&reference_preimage(body), &signature.into())
        .unwrap();
}

// The pinned vectors. Generated from the fixed seeds above; the test
// re-derives them through the independent reference, so a golden that
// stops matching means the wire format drifted.
const GOLDEN_CLAIM_WIRE_HEX: &str = "69642d73796e632f6232372d636c61696d2f7631c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c101000000000000000f7365727665723a636c61696d616e74ee45ecb9aca01a0abd83ef56dd985c8c874e6e7f4aebcedf20bd8d88c2a0add7000000000000000e7365727665723a7375626a65637400000000000000217365727665723a7375626a6563742064726f70706564206f75722077726974657300000000000003e8909090909090909090909090909090909090909090909090909090909090909000000000000007d0a88827c1ccaad2968a2276d2f4a5c25e1870840ad417cc7a3bb9f474def01e70e57b0d22de236639853deea85c568972eeb5313346a82badd91e9ebcaa513602";
const GOLDEN_REPLY_WIRE_HEX: &str = "69642d73796e632f6232372d636c61696d2f7631e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e102c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1000000000000000e7365727665723a7375626a6563745b6489c9c7fd0dcf50545e7c164886ef40491ec06c7f1b123041797e8117535e000000000000000e7365727665723a7375626a656374000000000000002974686174206e657665722068617070656e65643b206865726520617265206d7920726563656970747300000000000003f2909090909090909090909090909090909090909090909090909090909090909000000000000007d0ce6061fbe2da58504dfa226872b422659cdb93ca1342eae9bacd8fd9405dd0914969990d9f2ee8fd3bdc2ea85c0f4b17ac9727035f978a9c0d287a72d8e18202";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn golden_claim_wire_is_stable() {
    let signed = sign_event(
        &claim_body(0xC1, b"server:subject dropped our writes"),
        &key(CLAIMANT_SEED),
    );
    verify_reference(&signed.body, &signed.signature);
    assert_eq!(
        hex(&signed.encode()),
        GOLDEN_CLAIM_WIRE_HEX,
        "claim wire format drifted; see the reference preimage above"
    );
}

#[test]
fn golden_reply_wire_is_stable() {
    let target = id(0xC1);
    let signed = sign_event(&reply_body(0xE1, target), &key(SUBJECT_SEED));
    verify_reference(&signed.body, &signed.signature);
    assert_eq!(
        hex(&signed.encode()),
        GOLDEN_REPLY_WIRE_HEX,
        "reply wire format drifted; see the reference preimage above"
    );
}

#[test]
fn claim_and_reply_append_verify_and_order() {
    let reg = registry();
    let mut log = ClaimLog::new();
    let claim = sign_event(
        &claim_body(0xC1, b"server:subject dropped our writes"),
        &key(CLAIMANT_SEED),
    );
    let reply = sign_event(&reply_body(0xE1, id(0xC1)), &key(SUBJECT_SEED));
    assert_eq!(log.append(&reg, &claim.encode()).unwrap(), id(0xC1));
    assert_eq!(log.append(&reg, &reply.encode()).unwrap(), id(0xE1));
    assert_eq!(log.len(), 2);
    let kinds: Vec<EventKind> = log.iter().map(|e| e.body.kind).collect();
    assert_eq!(kinds, vec![EventKind::Claim, EventKind::Reply]);
    // The reply answers the claim it names.
    assert_eq!(log.get(&id(0xE1)).unwrap().body.in_reply_to, Some(id(0xC1)));
}

#[test]
fn reply_subject_must_match_the_event_it_answers() {
    // Oracle: the module documents "a reply names the same subject as
    // the claim it answers" — a properly signed reply whose subject
    // differs from its target's must be refused, or third-party
    // commentary attaches to someone else's dispute (#16313).
    let reg = registry();
    let mut log = ClaimLog::new();
    let claim = sign_event(
        &claim_body(0xC1, b"server:subject dropped our writes"),
        &key(CLAIMANT_SEED),
    );
    log.append(&reg, &claim.encode()).unwrap();
    // The subject answers, but names a different subject than the
    // claim it answers: refused, and not admitted.
    let mismatched = EventBody {
        subject: "server:someone-else".to_owned(),
        ..reply_body(0xE1, id(0xC1))
    };
    let signed = sign_event(&mismatched, &key(SUBJECT_SEED));
    let err = log.append(&reg, &signed.encode()).unwrap_err();
    assert!(
        matches!(
            err,
            ClaimError::SubjectMismatch {
                ref target_subject,
                ref reply_subject
            } if target_subject == "server:subject" && reply_subject == "server:someone-else"
        ),
        "mismatched-subject reply must fail typed, got {err:?}"
    );
    assert_eq!(log.len(), 1, "the mismatched reply must not be admitted");
    // A matched-subject reply is still admitted.
    let reply = sign_event(&reply_body(0xE1, id(0xC1)), &key(SUBJECT_SEED));
    log.append(&reg, &reply.encode()).unwrap();
    // Reply-to-reply is a correction (the module's own rule) and stays
    // allowed — under the same subject guard: the claimant corrects
    // its claim by answering the subject's reply, naming the same
    // subject.
    let correction = EventBody {
        author: "server:claimant".to_owned(),
        author_key: key(CLAIMANT_SEED).verifying_key().to_bytes(),
        ..reply_body(0xE2, id(0xE1))
    };
    let signed = sign_event(&correction, &key(CLAIMANT_SEED));
    log.append(&reg, &signed.encode()).unwrap();
    assert_eq!(log.len(), 3);
    // But a reply-to-reply naming another subject is refused too: the
    // guard follows the target event, whatever its kind.
    let stray = EventBody {
        subject: "server:someone-else".to_owned(),
        ..reply_body(0xE3, id(0xE1))
    };
    let signed = sign_event(&stray, &key(SUBJECT_SEED));
    assert!(matches!(
        log.append(&reg, &signed.encode()).unwrap_err(),
        ClaimError::SubjectMismatch { .. }
    ));
    assert_eq!(log.len(), 3);
}

#[test]
fn unilateral_claims_never_move_the_score() {
    let reg = registry();
    let mut log = ClaimLog::new();
    // Ten unilateral claims from the claimant, none answered.
    for n in 0..10u8 {
        let body = EventBody {
            event_id: id(0xA0 + n),
            ..claim_body(0xA0 + n, b"server:subject misbehaved again")
        };
        let signed = sign_event(&body, &key(CLAIMANT_SEED));
        log.append(&reg, &signed.encode()).unwrap();
    }
    // The reference ledger: every evidence record feeds a score, and a
    // unilateral allegation with no verdict must contribute nothing.
    let mut score: i64 = 0;
    for evidence in log.evidence() {
        assert!(
            !evidence.alters_reputation_score(),
            "evidence must never carry score authority"
        );
        score += evidence.reputation_delta();
    }
    assert_eq!(score, 0, "unilateral claims moved the global score");
    assert_eq!(log.len(), 10);
}

#[test]
fn equal_authentication_for_claims_and_replies() {
    // Neither side is bound here: a reply from an unknown key must fail
    // exactly the way a claim from an unknown key fails.
    let empty = BindingRegistry::new();
    let mut log = ClaimLog::new();
    let claim = sign_event(
        &claim_body(0xC1, b"server:subject dropped our writes"),
        &key(CLAIMANT_SEED),
    );
    let claim_err = log.append(&empty, &claim.encode()).unwrap_err();
    assert!(matches!(claim_err, ClaimError::Binding(_)), "{claim_err:?}");
    // Bind only the claimant, then a stranger's reply must still fail
    // the same identity gate.
    let mut partial = BindingRegistry::new();
    partial
        .bind(
            "server:claimant",
            key(CLAIMANT_SEED).verifying_key().to_bytes(),
        )
        .unwrap();
    let mut log = ClaimLog::new();
    log.append(&partial, &claim.encode()).unwrap();
    let stranger_reply = EventBody {
        author: "server:stranger".to_owned(),
        author_key: key(RELAY_SEED).verifying_key().to_bytes(),
        ..reply_body(0xE1, id(0xC1))
    };
    let signed = sign_event(&stranger_reply, &key(RELAY_SEED));
    let reply_err = log.append(&partial, &signed.encode()).unwrap_err();
    assert!(matches!(reply_err, ClaimError::Binding(_)), "{reply_err:?}");
}

#[test]
fn tampered_bytes_are_refused() {
    let reg = registry();
    let claim = sign_event(
        &claim_body(0xC1, b"server:subject dropped our writes"),
        &key(CLAIMANT_SEED),
    );
    let mut wire = claim.encode();
    // Flip one byte inside the author key: the binding still resolves
    // (identity names the claimant) but the signature must not verify.
    let key_offset = wire
        .windows(key(CLAIMANT_SEED).verifying_key().to_bytes().len())
        .position(|w| w == key(CLAIMANT_SEED).verifying_key().to_bytes())
        .unwrap();
    wire[key_offset] ^= 0x01;
    let mut log = ClaimLog::new();
    let err = log.append(&reg, &wire).unwrap_err();
    assert!(
        matches!(err, ClaimError::Binding(_) | ClaimError::Malformed),
        "tampered key must fail closed, got {err:?}"
    );
    // A truncated wire form is malformed, never half-admitted.
    let mut log = ClaimLog::new();
    let short = &claim.encode()[..claim.encode().len() - 10];
    assert!(matches!(
        log.append(&reg, short).unwrap_err(),
        ClaimError::Malformed
    ));
}

#[test]
fn structural_rules_hold() {
    let reg = registry();
    let mut log = ClaimLog::new();
    assert!(log.is_empty());
    assert_eq!(
        EventKind::ALL
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>(),
        vec!["claim", "reply"]
    );
    let claim = sign_event(
        &claim_body(0xC1, b"server:subject dropped our writes"),
        &key(CLAIMANT_SEED),
    );
    log.append(&reg, &claim.encode()).unwrap();
    // The same event twice is a duplicate, not a second entry.
    assert!(matches!(
        log.append(&reg, &claim.encode()).unwrap_err(),
        ClaimError::DuplicateId(_)
    ));
    // A reply must answer a claim that is actually in the log.
    let orphan = sign_event(&reply_body(0xE1, id(0x99)), &key(SUBJECT_SEED));
    assert!(matches!(
        log.append(&reg, &orphan.encode()).unwrap_err(),
        ClaimError::UnknownTarget(_)
    ));
    // The kind/target pairing is wire grammar, not log state: a claim
    // carries no target bytes and a reply always carries 32, so decode
    // cannot produce a mismatched pair. The append-time arms stay as
    // defense against future encoder changes.
    let decoded_claim = b27_claim_log::SignedEvent::decode(
        &sign_event(&claim_body(0xC5, b"grammar"), &key(CLAIMANT_SEED)).encode(),
    )
    .unwrap();
    assert_eq!(decoded_claim.body.kind, EventKind::Claim);
    assert_eq!(decoded_claim.body.in_reply_to, None);
    // Oversized statements are refused for claims and replies alike.
    let big = vec![b'x'; MAX_STATEMENT_BYTES + 1];
    let big_claim = sign_event(&claim_body(0xC3, &big), &key(CLAIMANT_SEED));
    assert!(matches!(
        log.append(&reg, &big_claim.encode()).unwrap_err(),
        ClaimError::StatementTooLarge { .. }
    ));
    // Empty statements carry no allegation and no answer.
    let empty = sign_event(&claim_body(0xC4, b""), &key(CLAIMANT_SEED));
    assert!(matches!(
        log.append(&reg, &empty.encode()).unwrap_err(),
        ClaimError::EmptyStatement
    ));
    assert_eq!(log.len(), 1, "only the first claim may be admitted");
    assert!(!log.is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn valid_events_verify_and_mutations_fail(
        seed_byte in 0u8..=255,
        stmt in proptest::collection::vec(any::<u8>(), 1..=48),
    ) {
        let signing = SigningKey::from_bytes(&[seed_byte; 32]);
        let author_key = signing.verifying_key().to_bytes();
        let mut reg = BindingRegistry::new();
        reg.bind("server:t", author_key).unwrap();
        let body = EventBody {
            event_id: id(seed_byte ^ 0x5a),
            kind: EventKind::Claim,
            author: "server:t".to_owned(),
            author_key,
            subject: "server:u".to_owned(),
            in_reply_to: None,
            statement: stmt,
            issued_at: 7,
            policy_digest: [seed_byte; 32],
            expires_at: 9,
        };
        // The reference verifier agrees before the log ever sees it.
        let signed = sign_event(&body, &signing);
        verify_reference(&signed.body, &signed.signature);
        let mut log = ClaimLog::new();
        prop_assert!(log.append(&reg, &signed.encode()).is_ok());
        // A sample of single-byte mutations is refused: the tampered
        // entry is never admitted, whatever broke first. The
        // deterministic sweep below covers every position.
        let wire = signed.encode();
        let positions = [0usize, 1, 31, 32, 63, 64, wire.len() / 2, wire.len() - 65, wire.len() - 1];
        for i in positions.into_iter().filter(|i| *i < wire.len()) {
            let mut bad = wire.clone();
            bad[i] = bad[i].wrapping_add(1);
            let mut probe = ClaimLog::new();
            prop_assert!(probe.append(&reg, &bad).is_err(), "byte {i} mutated but admitted");
        }
    }
}

#[test]
fn every_wire_byte_is_load_bearing() {
    // Deterministic full sweep over the golden claim: mutating any one
    // byte refuses admission. The proptest above varies the content;
    // this one covers every position.
    let reg = registry();
    let wire = unhex(GOLDEN_CLAIM_WIRE_HEX);
    assert!(!wire.is_empty());
    for (i, byte) in wire.iter().enumerate() {
        let mut bad = wire.clone();
        bad[i] = byte.wrapping_add(1);
        let mut probe = ClaimLog::new();
        assert!(
            probe.append(&reg, &bad).is_err(),
            "byte {i} mutated but admitted"
        );
    }
}
