//! Existence acknowledgements: the #9115 consumer binding of the
//! B19 V2 shared receipt schema (slice A9115-existence-receipt,
//! issue #10256; statements S3.23–S3.25).
//!
//! Oracle: **model** — the reference model in this file implements
//! the documented B19 V2 schema independently: it builds the
//! canonical bytes itself from the documented layout, computes the
//! presence response with `sha2` directly, and signs and verifies
//! with `ed25519-dalek` directly. Every operation the slice lists
//! (answer a challenge, verify, check presence, encode/decode,
//! coalesce a query result) is generated against the model and
//! every resulting state and error is compared. Ed25519 is
//! deterministic, so the fixed vector below is a golden pin: any
//! change to the schema's bytes fails it instead of silently
//! re-baselining.
//!
//! The crate-root export belongs to the wiring slice, so this test
//! includes the leaf module by path, as the neighbouring ID slices
//! do.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

#[path = "../src/existence_receipt.rs"]
mod existence_receipt;

use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use existence_receipt::{
    AckContext, AckError, CoalescedExistence, ExistenceEvidence, ExistenceResult, ExistsAck,
    Responder, SignedAck, StoredUnverifiedObservation, check_presence, presence_response,
    subject_digest, verify,
};

// ---------------------------------------------------------------------------
// Reference model: the B19 V2 schema, written from the documented
// layout, independent of the module under test.
// ---------------------------------------------------------------------------

const MODEL_RECEIPT_DOMAIN: &[u8] = b"id-sync/b19-receipt/v1";
const MODEL_PRESENCE_DOMAIN: &[u8] = b"id-sync/b19-presence/v1";

fn model_push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

/// The model's canonical bytes for any B19 kind.
#[allow(clippy::too_many_arguments)]
fn model_canonical(
    kind: u8,
    receipt_id: &[u8; 32],
    challenge: &[u8; 32],
    response: &[u8; 32],
    until: u64,
    digest: &[u8; 32],
    signer: &str,
    signer_key: &[u8; 32],
    recipient_key: &[u8; 32],
    scope: &str,
    issued_at: u64,
    epoch: u64,
    policy_digest: &[u8; 32],
    valid_from: u64,
    valid_until: u64,
) -> Vec<u8> {
    let mut out = MODEL_RECEIPT_DOMAIN.to_vec();
    out.extend_from_slice(receipt_id);
    out.push(kind);
    match kind {
        1 => {}
        2 => out.extend_from_slice(&until.to_be_bytes()),
        3 => {
            out.extend_from_slice(challenge);
            out.extend_from_slice(response);
        }
        _ => {}
    }
    out.extend_from_slice(digest);
    model_push_field(&mut out, signer.as_bytes());
    out.extend_from_slice(signer_key);
    out.extend_from_slice(recipient_key);
    model_push_field(&mut out, scope.as_bytes());
    out.extend_from_slice(&issued_at.to_be_bytes());
    out.extend_from_slice(&epoch.to_be_bytes());
    out.extend_from_slice(policy_digest);
    out.extend_from_slice(&valid_from.to_be_bytes());
    out.extend_from_slice(&valid_until.to_be_bytes());
    out
}

fn model_presence(challenge: &[u8; 32], content: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(MODEL_PRESENCE_DOMAIN);
    hasher.update(challenge);
    hasher.update(content);
    hasher.finalize().into()
}

fn model_digest(content: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher.finalize().into()
}

/// The model's verdict for a signed ack at `now`, computed with
/// dalek directly: `Ok(())` or the error class as a string.
fn model_verify(body: &ExistsAck, signature: &[u8; 64], now: u64) -> Result<(), &'static str> {
    if body.valid_from > body.valid_until {
        return Err("invalid_interval");
    }
    let key = VerifyingKey::from_bytes(&body.responder_key).map_err(|_| "bad_key")?;
    key.verify(&body.canonical_bytes(), &Signature::from_bytes(signature))
        .map_err(|_| "bad_signature")?;
    if now < body.valid_from {
        return Err("not_yet_valid");
    }
    if now > body.valid_until {
        return Err("expired");
    }
    Ok(())
}

fn err_class(error: &AckError) -> &'static str {
    match error {
        AckError::Malformed => "malformed",
        AckError::UnknownAssertion(_) => "unknown_assertion",
        AckError::WrongAssertion(_) => "wrong_assertion",
        AckError::InvalidInterval => "invalid_interval",
        AckError::BadKey => "bad_key",
        AckError::BadSignature => "bad_signature",
        AckError::NotYetValid => "not_yet_valid",
        AckError::Expired => "expired",
        AckError::PresenceMismatch => "presence_mismatch",
    }
}

// ---------------------------------------------------------------------------
// Fixed fixtures.
// ---------------------------------------------------------------------------

fn responder() -> Responder {
    Responder::new("responder-a", SigningKey::from_bytes(&[3u8; 32]), [9u8; 32])
}

fn ctx() -> AckContext {
    AckContext {
        receipt_id: [1u8; 32],
        recipient_key: [5u8; 32],
        observed_at: 1_700_000_100,
        epoch: 42,
        valid_from: 1_700_000_000,
        valid_until: 1_700_001_000,
    }
}

const CONTENT: &[u8] = b"the exact subject bytes";
const CHALLENGE: [u8; 32] = [7u8; 32];
const SCOPE: &str = "tenant-a/namespace-b";

fn fixed_ack() -> SignedAck {
    responder()
        .answer_challenge(CONTENT, CHALLENGE, SCOPE, ctx())
        .expect("fixed ack answers")
}

// ---------------------------------------------------------------------------
// Golden / schema agreement.
// ---------------------------------------------------------------------------

#[test]
fn canonical_bytes_match_the_b19_model_byte_for_byte() {
    let ack = fixed_ack();
    let model = model_canonical(
        3,
        &ack.body.receipt_id,
        &ack.body.challenge,
        &ack.body.response,
        0,
        &ack.body.subject_digest,
        &ack.body.responder,
        &ack.body.responder_key,
        &ack.body.recipient_key,
        &ack.body.scope,
        ack.body.observed_at,
        ack.body.epoch,
        &ack.body.policy_digest,
        ack.body.valid_from,
        ack.body.valid_until,
    );
    assert_eq!(ack.body.canonical_bytes(), model);
}

#[test]
fn golden_vector_is_pinned() {
    let ack = fixed_ack();
    // SHA-256 of the canonical bytes of the fixed vector above.
    let digest = model_digest(&ack.body.canonical_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, GOLDEN_CANONICAL_SHA256);
    // The signature is the model's own dalek signature over the
    // model bytes: deterministic, so it is a second pin.
    let key = SigningKey::from_bytes(&[3u8; 32]);
    let model_bytes = model_canonical(
        3,
        &ack.body.receipt_id,
        &ack.body.challenge,
        &ack.body.response,
        0,
        &ack.body.subject_digest,
        &ack.body.responder,
        &ack.body.responder_key,
        &ack.body.recipient_key,
        &ack.body.scope,
        ack.body.observed_at,
        ack.body.epoch,
        &ack.body.policy_digest,
        ack.body.valid_from,
        ack.body.valid_until,
    );
    use ed25519_dalek::Signer as _;
    assert_eq!(ack.signature, key.sign(&model_bytes).to_bytes());
}

const GOLDEN_CANONICAL_SHA256: &str =
    "b995ec2a3bfb6cdd8c86ffbde68c9a8ba3cc5380bdca42cc7510628e9f24e4b0";

#[test]
fn presence_response_matches_the_b19_model() {
    assert_eq!(
        presence_response(&CHALLENGE, CONTENT),
        model_presence(&CHALLENGE, CONTENT)
    );
    assert_eq!(subject_digest(CONTENT), model_digest(CONTENT));
    // The ack binds the model's response and digest, not its own
    // private variants.
    let ack = fixed_ack();
    assert_eq!(ack.body.response, model_presence(&CHALLENGE, CONTENT));
    assert_eq!(ack.body.subject_digest, model_digest(CONTENT));
}

// ---------------------------------------------------------------------------
// Operations against the model.
// ---------------------------------------------------------------------------

#[test]
fn answer_verify_and_check_presence() {
    let ack = fixed_ack();
    let verified = verify(&ack, 1_700_000_500).expect("ack verifies inside its interval");
    assert_eq!(verified.assertion(), "verified_present");
    assert_eq!(verified.responder(), "responder-a");
    assert_eq!(verified.scope(), SCOPE);
    assert_eq!(verified.epoch(), 42);
    assert_eq!(
        model_verify(&ack.body, &ack.signature, 1_700_000_500),
        Ok(())
    );
    check_presence(&ack.body, CONTENT).expect("presence checks against the content");
    // Different bytes are a different subject: presence fails.
    assert_eq!(
        check_presence(&ack.body, b"other bytes"),
        Err(AckError::PresenceMismatch)
    );
    // The right digest with a response over other content also fails:
    // the response binds challenge AND content, not the digest alone.
    let mut forged = ack.body.clone();
    forged.response = model_presence(&CHALLENGE, b"other bytes");
    assert_eq!(
        check_presence(&forged, CONTENT),
        Err(AckError::PresenceMismatch)
    );
}

#[test]
fn verify_time_window_matches_the_model() {
    let ack = fixed_ack();
    for (now, expected) in [(1_699_999_999, "not_yet_valid"), (1_700_001_001, "expired")] {
        let got = verify(&ack, now).map(|_| ()).map_err(|e| err_class(&e));
        assert_eq!(got, Err(expected), "now={now}");
        assert_eq!(model_verify(&ack.body, &ack.signature, now), Err(expected));
    }
    for now in [1_700_000_000, 1_700_001_000] {
        assert!(verify(&ack, now).is_ok(), "boundary now={now} verifies");
    }
}

#[test]
fn signed_fields_are_bound_by_the_signature() {
    let ack = fixed_ack();
    // Every bound field, changed after signing, breaks the signature:
    // responder, subject digest, scope, policy epoch, observation
    // time — the exact list the slice names.
    let mut by_responder = ack.clone();
    by_responder.body.responder = "responder-b".to_string();
    let mut by_digest = ack.clone();
    by_digest.body.subject_digest = [2u8; 32];
    let mut by_scope = ack.clone();
    by_scope.body.scope = "tenant-a/other".to_string();
    let mut by_epoch = ack.clone();
    by_epoch.body.epoch = 43;
    let mut by_time = ack.clone();
    by_time.body.observed_at += 1;
    for tampered in [by_responder, by_digest, by_scope, by_epoch, by_time] {
        assert_eq!(
            verify(&tampered, 1_700_000_500)
                .map(|_| ())
                .map_err(|e| err_class(&e)),
            Err("bad_signature")
        );
        assert_eq!(
            model_verify(&tampered.body, &tampered.signature, 1_700_000_500),
            Err("bad_signature")
        );
    }
    // A signature by a different key over the same body also fails.
    let other = SigningKey::from_bytes(&[4u8; 32]);
    use ed25519_dalek::Signer as _;
    let mut resigned = ack.clone();
    resigned.signature = other.sign(&ack.body.canonical_bytes()).to_bytes();
    assert_eq!(
        verify(&resigned, 1_700_000_500)
            .map(|_| ())
            .map_err(|e| err_class(&e)),
        Err("bad_signature")
    );
}

#[test]
fn empty_interval_is_refused_at_issue_and_at_verify() {
    let mut bad_ctx = ctx();
    bad_ctx.valid_from = bad_ctx.valid_until + 1;
    assert_eq!(
        responder().answer_challenge(CONTENT, CHALLENGE, SCOPE, bad_ctx),
        Err(AckError::InvalidInterval)
    );
    // A hand-built ack with an empty interval fails verify too, with
    // the same class the model gives.
    let mut ack = fixed_ack();
    ack.body.valid_from = ack.body.valid_until + 1;
    assert_eq!(
        verify(&ack, ack.body.valid_from)
            .map(|_| ())
            .map_err(|e| err_class(&e)),
        Err("invalid_interval")
    );
}

#[test]
fn encode_decode_round_trip_and_wire_refusals() {
    let ack = fixed_ack();
    let wire = ack.encode();
    assert_eq!(SignedAck::decode(&wire), Ok(ack.clone()));

    // Trailing bytes and truncation are malformed.
    let mut trailed = wire.clone();
    trailed.push(0);
    assert_eq!(SignedAck::decode(&trailed), Err(AckError::Malformed));
    assert_eq!(
        SignedAck::decode(&wire[..wire.len() - 1]),
        Err(AckError::Malformed)
    );
    assert_eq!(SignedAck::decode(&[]), Err(AckError::Malformed));

    // A B19 receipt of another kind is a real receipt but not an
    // existence ack: refused as the wrong assertion, by tag.
    for (kind, until) in [(1u8, 0u64), (2, 1_700_000_900)] {
        let other = model_canonical(
            kind,
            &[1u8; 32],
            &CHALLENGE,
            &ack.body.response,
            until,
            &ack.body.subject_digest,
            "responder-a",
            &ack.body.responder_key,
            &ack.body.recipient_key,
            SCOPE,
            1_700_000_100,
            42,
            &[9u8; 32],
            1_700_000_000,
            1_700_001_000,
        );
        let mut wire = other;
        wire.extend_from_slice(&[0u8; 64]);
        assert_eq!(
            SignedAck::decode(&wire),
            Err(AckError::WrongAssertion(kind)),
            "kind {kind}"
        );
    }

    // A tag outside the three B19 kinds is unknown, not wrong.
    let mut unknown = wire.clone();
    let tag_at = MODEL_RECEIPT_DOMAIN.len() + 32;
    unknown[tag_at] = 9;
    assert_eq!(
        SignedAck::decode(&unknown),
        Err(AckError::UnknownAssertion(9))
    );
}

// ---------------------------------------------------------------------------
// stored_unverified and query results (S3.23–S3.25 separations).
// ---------------------------------------------------------------------------

#[test]
fn stored_unverified_never_proves_present() {
    let observation = StoredUnverifiedObservation {
        subject_digest: subject_digest(CONTENT),
        observed_at: 1_700_000_100,
        location: "laptop/store".to_string(),
    };
    let evidence = ExistenceEvidence::StoredUnverified(observation);
    assert!(!evidence.proves_present());

    let verified = verify(&fixed_ack(), 1_700_000_500).expect("ack verifies");
    let evidence = ExistenceEvidence::Ack(Box::new(verified));
    assert!(evidence.proves_present());
}

#[test]
fn not_present_and_indeterminate_are_distinct_but_coalesce() {
    assert_ne!(ExistenceResult::NotPresent, ExistenceResult::Indeterminate);
    assert_eq!(
        ExistenceResult::NotPresent.coalesced(),
        CoalescedExistence::NotVerified
    );
    assert_eq!(
        ExistenceResult::Indeterminate.coalesced(),
        CoalescedExistence::NotVerified
    );
    let verified = verify(&fixed_ack(), 1_700_000_500).expect("ack verifies");
    assert_eq!(
        ExistenceResult::Present(verified.clone()).coalesced(),
        CoalescedExistence::VerifiedPresent(verified)
    );
}

// ---------------------------------------------------------------------------
// Generated operations against the model.
// ---------------------------------------------------------------------------

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn generated_answer_verify_round_trip(
        content in prop::collection::vec(any::<u8>(), 0..128),
        challenge in any::<[u8; 32]>(),
        seed in any::<[u8; 32]>(),
        scope in "[a-z]{1,8}/[a-z]{1,8}",
        epoch in 0..1_000u64,
        observed_at in 1_000_000..2_000_000u64,
        slack in 0..10_000u64,
        now_offset in 0..20_000i64,
    ) {
        let responder = Responder::new("responder-p", SigningKey::from_bytes(&seed), [9u8; 32]);
        let context = AckContext {
            receipt_id: [1u8; 32],
            recipient_key: [5u8; 32],
            observed_at,
            epoch,
            valid_from: observed_at,
            valid_until: observed_at + slack,
        };
        let ack = responder
            .answer_challenge(&content, challenge, &scope, context)
            .expect("generated context always has a valid interval");
        // The ack binds the model's digest and response.
        prop_assert_eq!(ack.body.subject_digest, model_digest(&content));
        prop_assert_eq!(ack.body.response, model_presence(&challenge, &content));
        prop_assert_eq!(ack.body.epoch, epoch);
        prop_assert_eq!(&ack.body.scope, &scope);
        // Wire round trip is lossless.
        let decoded = SignedAck::decode(&ack.encode()).expect("own wire form decodes");
        prop_assert_eq!(&decoded, &ack);
        // Verify agrees with the model at a generated time.
        let now = (observed_at as i64 + now_offset).max(0) as u64;
        let got = verify(&ack, now).map(|_| ()).map_err(|e| err_class(&e));
        let want = model_verify(&ack.body, &ack.signature, now);
        // The module checks the signature strictly; the model uses
        // the plain verifier — for honestly signed acks they agree.
        prop_assert_eq!(got, want);
        // Presence checks against exactly the answered content.
        prop_assert!(check_presence(&ack.body, &content).is_ok());
    }

    #[test]
    fn generated_single_byte_tamper_never_verifies(
        content in prop::collection::vec(any::<u8>(), 1..64),
        seed in any::<[u8; 32]>(),
        flip_at in 0..400usize,
        flip_with in 1..=255u8,
    ) {
        let responder = Responder::new("responder-t", SigningKey::from_bytes(&seed), [9u8; 32]);
        let context = AckContext {
            receipt_id: [1u8; 32],
            recipient_key: [5u8; 32],
            observed_at: 1_700_000_100,
            epoch: 42,
            valid_from: 1_700_000_000,
            valid_until: 1_700_001_000,
        };
        let ack = responder
            .answer_challenge(&content, CHALLENGE, SCOPE, context)
            .expect("fixed context answers");
        let mut wire = ack.encode();
        let at = flip_at % wire.len();
        wire[at] ^= flip_with;
        // A tampered wire form either fails to decode, or decodes to
        // an ack that fails verification at a time inside the honest
        // interval. It never verifies.
        if let Ok(decoded) = SignedAck::decode(&wire) {
            prop_assert!(verify(&decoded, 1_700_000_500).is_err());
        }
    }
}
