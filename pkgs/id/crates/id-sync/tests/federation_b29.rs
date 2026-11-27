//! B29 golden test: ack/recipient-list contradiction detection,
//! evidence retention, collusion threshold and public-log integration
//! (#10266, statements S1.105-S1.108).
//!
//! Oracle: **model** — a positive contradiction is reported only when
//! object/digest, interval, scope, policy, authorization and exact
//! claim semantics all match; otherwise the result is
//! `insufficient_evidence`. The verifier is an independent reference
//! evaluator in this file: it compares the same signed fields with
//! plain field logic, shares no acceptance code with the
//! implementation, and is cross-checked against it on fixed vectors
//! and on proptest-mutated dimensions. Fixed signed vectors pin the
//! commitment canonical bytes and the finding statement (Ed25519 is
//! deterministic, so fixed keys and fields give fixed bytes).
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), #9110-09 wires the
//! versioned public exports later, and B29 reaches B05/B19/B27/B32 as
//! `super::…`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b05_signer_binding.rs"]
#[allow(dead_code)]
mod b05_signer_binding;

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
#[allow(dead_code)]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b19_signed_receipt.rs"]
mod b19_signed_receipt;

#[path = "../src/federation/slices/b27_claim_log.rs"]
mod b27_claim_log;

#[path = "../src/federation/slices/b29_receipt_contradiction.rs"]
mod b29;

use b05_signer_binding::BindingRegistry;
use b19_signed_receipt::{
    Assertion, Digest32, KeyBytes, ReceiptBody, ReceiptClaim, SignedReceipt, VerifiedReceipt,
    verify as verify_receipt,
};
use b27_claim_log::{ClaimLog, EventKind, sign_event};
use b29::{
    AuthorizationDigest, COLLUSION_WINDOW, CommitmentBody, ContradictionCase, ContradictionCounter,
    DEFAULT_COLLUSION_THRESHOLD, InsufficientReason, MismatchDimension, RecipientSet, TransferId,
    Verdict, evaluate_admission, evaluate_commitments, evaluate_receipts, finding_claim_body,
    finding_statement, publish_finding, sign_commitment, verify_commitment,
};
use ed25519_dalek::{Signer, SigningKey};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const NOW: LogicalTime = 10_000;
type LogicalTime = u64;

const TRANSFER: TransferId = [0x54; 32];
const DIGEST: Digest32 = [0xD1; 32];
const POLICY: Digest32 = [0x50; 32];
const AUTH: AuthorizationDigest = [0xA0; 32];
const RETENTION_POLICY: Digest32 = [0x72; 32];
const RETENTION_EXPIRES: LogicalTime = 50_000;
const SCOPE: &str = "sync/default";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn vk(seed: u8) -> KeyBytes {
    key(seed).verifying_key().to_bytes()
}

fn sha(bytes: &[u8]) -> Digest32 {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write;
        write!(out, "{b:02x}").unwrap();
        out
    })
}

/// committer (seed 11) signs commitments; alice/bob/carol hold
/// recipient keys; relay (seed 77) files log claims.
fn registry() -> BindingRegistry {
    let mut registry = BindingRegistry::new();
    registry.bind("node:committer", vk(11)).unwrap();
    registry.bind("node:alice", vk(21)).unwrap();
    registry.bind("node:bob", vk(22)).unwrap();
    registry.bind("node:carol", vk(23)).unwrap();
    registry.bind("node:relay", vk(77)).unwrap();
    registry
}

fn commitment_body() -> CommitmentBody {
    CommitmentBody {
        transfer_id: TRANSFER,
        payload_digest: DIGEST,
        recipients: RecipientSet::Exact(vec![vk(21), vk(22)]),
        scope: SCOPE.to_string(),
        policy_digest: POLICY,
        authorization: AUTH,
        issued_at: 1_000,
        valid_from: 0,
        valid_until: 20_000,
        retention_policy: RETENTION_POLICY,
        retention_expires: RETENTION_EXPIRES,
        signer: "node:committer".to_string(),
        signer_key: vk(11),
    }
}

fn verified_commitment() -> b29::VerifiedCommitment {
    let signed = sign_commitment(&commitment_body(), &key(11));
    verify_commitment(&registry(), &signed, NOW).unwrap()
}

fn accepted_receipt(recipient_seed: u8, recipient_key: KeyBytes) -> VerifiedReceipt {
    accepted_receipt_full(
        recipient_seed,
        recipient_key,
        DIGEST,
        SCOPE,
        POLICY,
        0,
        20_000,
    )
}

#[allow(clippy::too_many_arguments)]
fn accepted_receipt_full(
    recipient_seed: u8,
    recipient_key: KeyBytes,
    digest: Digest32,
    scope: &str,
    policy: Digest32,
    valid_from: LogicalTime,
    valid_until: LogicalTime,
) -> VerifiedReceipt {
    let body = ReceiptBody {
        receipt_id: [recipient_seed; 32],
        claim: ReceiptClaim::Accepted,
        payload_digest: digest,
        signer: format!(
            "node:{}",
            match recipient_seed {
                21 => "alice",
                22 => "bob",
                23 => "carol",
                _ => "other",
            }
        ),
        signer_key: vk(recipient_seed),
        recipient_key,
        scope: scope.to_string(),
        issued_at: 2_000,
        epoch: 1,
        policy_digest: policy,
        valid_from,
        valid_until,
    };
    let signature = key(recipient_seed).sign(&body.canonical_bytes()).to_bytes();
    // Verify at a time inside the receipt's own validity: the
    // evaluator compares validity intervals, so an interval-mismatch
    // fixture must still be a well-formed verified receipt. B19
    // checks `now` against the receipt's validity, not against NOW.
    verify_receipt(&registry(), &SignedReceipt { body, signature }, valid_from).unwrap()
}

fn other_assertion_receipt() -> VerifiedReceipt {
    let body = ReceiptBody {
        receipt_id: [0x99; 32],
        claim: ReceiptClaim::RetainedUntil { until: 30_000 },
        payload_digest: DIGEST,
        signer: "node:carol".to_string(),
        signer_key: vk(23),
        recipient_key: vk(23),
        scope: SCOPE.to_string(),
        issued_at: 2_000,
        epoch: 1,
        policy_digest: POLICY,
        valid_from: 0,
        valid_until: 40_000,
    };
    let signature = key(23).sign(&body.canonical_bytes()).to_bytes();
    verify_receipt(&registry(), &SignedReceipt { body, signature }, NOW).unwrap()
}

/// The independent reference: plain field logic, no B29 calls.
/// Returns true only for a positive contradiction.
fn reference_admission(
    set: &[KeyBytes],
    digest: &Digest32,
    scope: &str,
    policy: &Digest32,
    c_from: LogicalTime,
    c_until: LogicalTime,
    receipt: &VerifiedReceipt,
) -> bool {
    let other = &receipt.receipt.body;
    if receipt.assertion() != Assertion::Accepted {
        return false;
    }
    if &other.payload_digest != digest {
        return false;
    }
    if other.scope != scope {
        return false;
    }
    if &other.policy_digest != policy {
        return false;
    }
    if c_from > other.valid_until || other.valid_from > c_until {
        return false;
    }
    !set.contains(&other.recipient_key)
}

fn assert_contradiction(verdict: Verdict) {
    assert!(
        matches!(
            verdict,
            Verdict::Contradiction(ContradictionCase::NonMemberAdmission)
        ),
        "expected a non-member contradiction, got {verdict:?}"
    );
}

#[test]
fn positive_non_member_admission_matches_reference() {
    let commitment = verified_commitment();
    let receipt = accepted_receipt(23, vk(23));
    let verdict = evaluate_admission(&commitment, &receipt);
    assert_contradiction(verdict);
    assert!(reference_admission(
        &[vk(21), vk(22)],
        &DIGEST,
        SCOPE,
        &POLICY,
        0,
        20_000,
        &receipt
    ));
}

#[test]
fn member_receipt_is_consistent_not_positive() {
    let commitment = verified_commitment();
    let receipt = accepted_receipt(21, vk(21));
    assert_eq!(
        evaluate_admission(&commitment, &receipt),
        Verdict::InsufficientEvidence(InsufficientReason::Consistent)
    );
    assert!(!reference_admission(
        &[vk(21), vk(22)],
        &DIGEST,
        SCOPE,
        &POLICY,
        0,
        20_000,
        &receipt
    ));
}

#[test]
fn each_mismatched_dimension_is_insufficient() {
    let commitment = verified_commitment();
    let cases = [
        (
            accepted_receipt_full(23, vk(23), [0xE5; 32], SCOPE, POLICY, 0, 20_000),
            MismatchDimension::ObjectDigest,
        ),
        (
            accepted_receipt_full(23, vk(23), DIGEST, "sync/other", POLICY, 0, 20_000),
            MismatchDimension::Scope,
        ),
        (
            accepted_receipt_full(23, vk(23), DIGEST, SCOPE, [0xE5; 32], 0, 20_000),
            MismatchDimension::Policy,
        ),
        (
            accepted_receipt_full(23, vk(23), DIGEST, SCOPE, POLICY, 30_000, 40_000),
            MismatchDimension::Interval,
        ),
    ];
    for (receipt, dimension) in cases {
        assert_eq!(
            evaluate_admission(&commitment, &receipt),
            Verdict::InsufficientEvidence(InsufficientReason::Dimension(dimension)),
            "wrong reason for {dimension:?}"
        );
        assert!(!reference_admission(
            &[vk(21), vk(22)],
            &DIGEST,
            SCOPE,
            &POLICY,
            0,
            20_000,
            &receipt
        ));
    }
    assert_eq!(
        evaluate_admission(&commitment, &other_assertion_receipt()),
        Verdict::InsufficientEvidence(InsufficientReason::Dimension(
            MismatchDimension::ClaimSemantics
        ))
    );
}

#[test]
fn no_counterpart_is_insufficient_not_positive() {
    let commitment = verified_commitment();
    let (verdict, finding) = evaluate_receipts(&commitment, &[]);
    assert_eq!(
        verdict,
        Verdict::InsufficientEvidence(InsufficientReason::NoCounterpart)
    );
    assert!(finding.is_none());
    let (verdict, _) = evaluate_receipts(&commitment, &[other_assertion_receipt()]);
    assert_eq!(
        verdict,
        Verdict::InsufficientEvidence(InsufficientReason::NoCounterpart)
    );
}

#[test]
fn receipts_batch_prefers_contradiction_over_consistency() {
    let commitment = verified_commitment();
    let receipts = vec![accepted_receipt(21, vk(21)), accepted_receipt(23, vk(23))];
    let (verdict, finding) = evaluate_receipts(&commitment, &receipts);
    assert_contradiction(verdict);
    let finding = finding.unwrap();
    assert_eq!(finding.transfer_id, TRANSFER);
    assert_eq!(finding.payload_digest, DIGEST);
    assert_eq!(finding.peer, "node:committer");
    assert!(finding.is_live(NOW));
    assert!(!finding.is_live(RETENTION_EXPIRES + 1));
}

#[test]
fn divergent_commitments_disagree_on_digest_or_set() {
    let first = verified_commitment();
    let mut other_body = commitment_body();
    other_body.payload_digest = [0xE5; 32];
    let other_signed = sign_commitment(&other_body, &key(11));
    let other = verify_commitment(&registry(), &other_signed, NOW).unwrap();
    let (verdict, finding) = evaluate_commitments(&first, &other);
    assert!(matches!(
        verdict,
        Verdict::Contradiction(ContradictionCase::DivergentCommitments)
    ));
    assert!(finding.is_some());

    let mut set_body = commitment_body();
    set_body.recipients = RecipientSet::Exact(vec![vk(21)]);
    let set_signed = sign_commitment(&set_body, &key(11));
    let set_other = verify_commitment(&registry(), &set_signed, NOW).unwrap();
    let (verdict, _) = evaluate_commitments(&first, &set_other);
    assert!(matches!(
        verdict,
        Verdict::Contradiction(ContradictionCase::DivergentCommitments)
    ));
}

#[test]
fn identical_or_dimension_split_commitments_are_insufficient() {
    let first = verified_commitment();
    let (verdict, finding) = evaluate_commitments(&first, &first);
    assert_eq!(
        verdict,
        Verdict::InsufficientEvidence(InsufficientReason::Consistent)
    );
    assert!(finding.is_none());

    for mutate in [
        (|b: &mut CommitmentBody| b.scope = "sync/other".to_string()),
        (|b: &mut CommitmentBody| b.policy_digest = [0xE5; 32]),
        (|b: &mut CommitmentBody| b.authorization = [0xE5; 32]),
        (|b: &mut CommitmentBody| {
            b.valid_from = 30_000;
            b.valid_until = 40_000;
        }),
        (|b: &mut CommitmentBody| b.transfer_id = [0xE5; 32]),
    ] {
        let mut body = commitment_body();
        mutate(&mut body);
        let signed = sign_commitment(&body, &key(11));
        // Verify inside the mutated validity: the evaluator compares
        // intervals, so an interval-mismatch fixture must still verify.
        let check_at = body.valid_from;
        let other = verify_commitment(&registry(), &signed, check_at).unwrap();
        let (verdict, finding) = evaluate_commitments(&first, &other);
        assert!(
            matches!(
                verdict,
                Verdict::InsufficientEvidence(InsufficientReason::Dimension(_))
            ),
            "expected a dimension mismatch, got {verdict:?}"
        );
        assert!(finding.is_none());
    }
}

#[test]
fn root_commitment_enforces_inclusion_proof() {
    // Two-key set {alice, bob}: leaf order after canonical sort is by
    // key bytes. Build the proof for alice's membership by hand.
    let alice = vk(21);
    let bob = vk(22);
    let mut sorted = vec![alice, bob];
    sorted.sort_unstable();
    let left_is_alice = sorted[0] == alice;
    let mut hasher = Sha256::new();
    hasher.update([0x00]);
    hasher.update(sorted[0]);
    let left: Digest32 = hasher.finalize().into();
    let mut hasher = Sha256::new();
    hasher.update([0x00]);
    hasher.update(sorted[1]);
    let right: Digest32 = hasher.finalize().into();
    let mut hasher = Sha256::new();
    hasher.update([0x01]);
    hasher.update(left);
    hasher.update(right);
    let root: Digest32 = hasher.finalize().into();
    let (sibling, sibling_is_left) = if left_is_alice {
        (right, false)
    } else {
        (left, true)
    };
    let mut body = commitment_body();
    body.recipients = RecipientSet::Root {
        root,
        proof: b29::MembershipProof {
            leaf_key: alice,
            path: vec![(sibling, sibling_is_left)],
        },
    };
    let signed = sign_commitment(&body, &key(11));
    let commitment = verify_commitment(&registry(), &signed, NOW).unwrap();
    assert!(commitment.commitment.body.recipients.contains(&alice));
    assert!(!commitment.commitment.body.recipients.contains(&bob));

    // A bad proof fails closed: the key is treated as a non-member.
    let mut bad = body.clone();
    bad.recipients = RecipientSet::Root {
        root,
        proof: b29::MembershipProof {
            leaf_key: alice,
            path: vec![([0xE5; 32], sibling_is_left)],
        },
    };
    assert!(!bad.recipients.contains(&alice));

    let receipt = accepted_receipt(21, alice);
    assert_contradiction(evaluate_admission(
        &verify_commitment(&registry(), &sign_commitment(&bad, &key(11)), NOW).unwrap(),
        &receipt,
    ));
}

#[test]
fn commitment_shape_failures_reject_before_signature() {
    let mut empty = commitment_body();
    empty.recipients = RecipientSet::Exact(vec![]);
    let signed = sign_commitment(&empty, &key(11));
    assert!(matches!(
        verify_commitment(&registry(), &signed, NOW),
        Err(b29::CommitmentError::EmptyRecipients)
    ));

    let mut inverted = commitment_body();
    inverted.valid_from = 5_000;
    inverted.valid_until = 4_000;
    let signed = sign_commitment(&inverted, &key(11));
    assert!(matches!(
        verify_commitment(&registry(), &signed, NOW),
        Err(b29::CommitmentError::InvalidInterval)
    ));

    // Unbound signer fails as an identity error, not a crypto one.
    let mut rogue = commitment_body();
    rogue.signer = "node:rogue".to_string();
    rogue.signer_key = vk(99);
    let signed = sign_commitment(&rogue, &key(99));
    assert!(matches!(
        verify_commitment(&registry(), &signed, NOW),
        Err(b29::CommitmentError::Binding(_))
    ));
}

#[test]
fn finding_publishes_to_claim_log_without_moving_score() {
    let commitment = verified_commitment();
    let receipt = accepted_receipt(23, vk(23));
    let (_, finding) = evaluate_receipts(&commitment, &[receipt]);
    let finding = finding.unwrap();

    let statement = finding_statement(&finding).unwrap();
    assert!(statement.len() <= b27_claim_log::MAX_STATEMENT_BYTES);

    let registry = registry();
    let mut log = ClaimLog::new();
    let event_id = sha(b"b29-finding-1");
    let body = finding_claim_body(&finding, event_id, "node:relay", &vk(77), NOW).unwrap();
    assert_eq!(body.kind, EventKind::Claim);
    assert_eq!(body.subject, "node:committer");
    let event = sign_event(&body, &key(77));
    let appended = log.append(&registry, &event.encode()).unwrap();
    assert_eq!(appended, event_id);
    assert_eq!(log.len(), 1);

    // The public-log integration rule: publishing a contradiction
    // informs later decisions; it moves no score by itself.
    let mut deltas = 0;
    for evidence in log.evidence() {
        assert!(!evidence.alters_reputation_score());
        deltas += evidence.reputation_delta();
    }
    assert_eq!(deltas, 0);

    // Same path through the convenience call: sign, append and read
    // back the log's evidence view in one step.
    let mut log2 = ClaimLog::new();
    let event_id2 = sha(b"b29-finding-2");
    let appended2 = publish_finding(
        &mut log2,
        &registry,
        &finding,
        event_id2,
        "node:relay",
        &key(77),
        NOW,
    )
    .unwrap();
    assert_eq!(appended2, event_id2);
    assert_eq!(log2.len(), 1);
}

#[test]
fn collusion_counter_dedups_caps_and_expires() {
    let commitment = verified_commitment();
    let receipt = accepted_receipt(23, vk(23));
    let (_, finding) = evaluate_receipts(&commitment, &[receipt]);
    let base = finding.unwrap();

    let mut counter = ContradictionCounter::new();
    // Retries of one event count once, however often re-reported.
    counter.record(&base);
    counter.record(&base);
    counter.record(&base);
    assert_eq!(counter.count_attributable("node:committer", NOW), 1);
    assert!(!counter.meets_threshold("node:committer", DEFAULT_COLLUSION_THRESHOLD, NOW));

    // Two more distinct objects reach the starting-default threshold.
    let mut second = base.clone();
    second.transfer_id = [0x01; 32];
    let mut third = base.clone();
    third.transfer_id = [0x02; 32];
    counter.record(&second);
    counter.record(&third);
    assert_eq!(counter.count_attributable("node:committer", NOW), 3);
    assert!(counter.meets_threshold("node:committer", DEFAULT_COLLUSION_THRESHOLD, NOW));
    assert!(!counter.meets_threshold("node:committer", 4, NOW));

    // Other peers are unaffected, and expired findings never count.
    assert_eq!(counter.count_attributable("node:alice", NOW), 0);
    assert_eq!(
        counter.count_attributable("node:committer", RETENTION_EXPIRES + 1),
        0
    );
    assert!(!counter.meets_threshold("node:committer", 1, RETENTION_EXPIRES + 1));
    assert_eq!(COLLUSION_WINDOW, 600);
}

#[test]
fn pinned_vectors_catch_schema_drift() {
    let body = commitment_body();
    let bytes = body.canonical_bytes();
    assert_eq!(hex(&bytes), PINNED_COMMITMENT_HEX);

    let commitment = verified_commitment();
    let receipt = accepted_receipt(23, vk(23));
    let (_, finding) = evaluate_receipts(&commitment, &[receipt]);
    assert_eq!(
        hex(&finding_statement(&finding.unwrap()).unwrap()),
        PINNED_FINDING_HEX
    );
}

/// First pin of the B29 wire layout: fixed keys and fields give fixed
/// bytes, so any future schema change fails here instead of silently
/// re-baselining. Refresh only with a deliberate schema bump.
const PINNED_COMMITMENT_HEX: &str = "69642d73796e632f6232392d636f6d6d69746d656e742f76315454545454545454545454545454545454545454545454545454545454545454d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1010000000000000002511c34a1a2cb521df16bb246b8de8e7997ce235c7e76b22a3d7503a24819dd8ad54207da194977dcf46adbfec2bc2e75b52d5a8a42184fedfdc00024f0e3e8da000000000000000c73796e632f64656661756c745050505050505050505050505050505050505050505050505050505050505050a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a000000000000003e800000000000000000000000000004e207272727272727272727272727272727272727272727272727272727272727272000000000000c350000000000000000e6e6f64653a636f6d6d697474657266be7e332c7a453332bd9d0a7f7db055f5c5ef1a06ada66d98b39fb6810c473a";
const PINNED_FINDING_HEX: &str = "69642d73796e632f6232392d66696e64696e672f76316e6f6e2d6d656d6265722d61646d697373696f6e5454545454545454545454545454545454545454545454545454545454545454d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1000000000000000e6e6f64653a636f6d6d697474657200000000000003e87272727272727272727272727272727272727272727272727272727272727272000000000000c350";

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn single_dimension_mutation_never_goes_positive(
        dim in 0..6u8,
        other_scope in "[a-z/]{1,12}",
    ) {
        let commitment = verified_commitment();
        let receipt = match dim {
            0 => accepted_receipt_full(23, vk(23), sha(b"mutated"), SCOPE, POLICY, 0, 20_000),
            1 => accepted_receipt_full(
                23,
                vk(23),
                DIGEST,
                &format!("sync/{other_scope}"),
                POLICY,
                0,
                20_000,
            ),
            2 => accepted_receipt_full(23, vk(23), DIGEST, SCOPE, sha(b"policy"), 0, 20_000),
            3 => accepted_receipt_full(23, vk(23), DIGEST, SCOPE, POLICY, 30_000, 40_000),
            4 => other_assertion_receipt(),
            _ => accepted_receipt(21, vk(21)),
        };
        let verdict = evaluate_admission(&commitment, &receipt);
        let reference = reference_admission(
            &[vk(21), vk(22)],
            &DIGEST,
            SCOPE,
            &POLICY,
            0,
            20_000,
            &receipt,
        );
        prop_assert!(!reference, "mutated dimension must clear the reference");
        prop_assert!(
            !matches!(verdict, Verdict::Contradiction(_)),
            "mutated dimension must not go positive: {verdict:?}"
        );
    }
}
