//! B19 golden test: the V2 shared signed receipt schema (#10250,
//! statements S1.100-S1.104).
//!
//! Oracle: **golden** — `accepted`, `retained_until` and
//! `verified_present` are distinct signed assertions, and
//! `stored_unverified` never admits content. The verifier is an
//! independent reference in this file: it encodes the documented wire
//! layout itself, checks signatures with `ed25519-dalek` directly
//! against its own key table, and is compared with fixed signed
//! vectors (Ed25519 is deterministic, so fixed keys and fields give
//! fixed bytes). Any change to the schema's bytes fails the pinned
//! digests and signatures instead of silently re-baselining them.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B19 reaches B05
//! and B32 as `super::…`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b05_signer_binding.rs"]
#[allow(dead_code)]
mod b05_signer_binding;

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b19_signed_receipt.rs"]
mod b19;

use b05_signer_binding::{BindingError, BindingRegistry};
use b19::{
    Assertion, Holding, InventoryObservation, Issuer, ReceiptBody, ReceiptClaim, ReceiptContext,
    ReceiptError, ReceiptLedger, SignedReceipt, check_presence, presence_response, verify,
};
use b32_ingest_gate::{
    FreshnessWindow, IngestGate, IngressPath, RejectReason, SignerAuthorization, sign_envelope,
};
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const NOW: u64 = 10_000;
const POLICY: [u8; 32] = [0x50; 32];
const MAX_RETENTION: u64 = 86_400;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn vk(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}

fn sha(bytes: &[u8]) -> [u8; 32] {
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

/// alice (seed 7) issues; bob (seed 9) is bound too; seed 13 is unbound.
fn registry() -> BindingRegistry {
    let mut registry = BindingRegistry::new();
    registry.bind("alice", vk(7)).unwrap();
    registry.bind("bob", vk(9)).unwrap();
    registry
}

fn alice() -> Issuer {
    Issuer::new("alice", key(7), POLICY, MAX_RETENTION)
}

fn ctx(id: u8) -> ReceiptContext {
    ReceiptContext {
        receipt_id: [id; 32],
        recipient_key: vk(9),
        issued_at: NOW,
        epoch: 3,
        valid_from: NOW,
        valid_until: NOW + 3_600,
    }
}

/// The documented wire layout, encoded independently of the slice.
fn reference_bytes(body: &ReceiptBody) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&u64::try_from(bytes.len()).unwrap().to_be_bytes());
        out.extend_from_slice(bytes);
    }
    let mut out = b"id-sync/b19-receipt/v1".to_vec();
    out.extend_from_slice(&body.receipt_id);
    match body.claim {
        ReceiptClaim::Accepted => out.push(0x01),
        ReceiptClaim::RetainedUntil { until } => {
            out.push(0x02);
            out.extend_from_slice(&until.to_be_bytes());
        }
        ReceiptClaim::VerifiedPresent {
            challenge,
            response,
        } => {
            out.push(0x03);
            out.extend_from_slice(&challenge);
            out.extend_from_slice(&response);
        }
    }
    out.extend_from_slice(&body.payload_digest);
    field(&mut out, body.signer.as_bytes());
    out.extend_from_slice(&body.signer_key);
    out.extend_from_slice(&body.recipient_key);
    field(&mut out, body.scope.as_bytes());
    for n in [body.issued_at, body.epoch] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    out.extend_from_slice(&body.policy_digest);
    for n in [body.valid_from, body.valid_until] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    out
}

/// The reference validator's verdict on a receipt.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Valid,
    NotSignersKey,
    BadSignature,
    OutsideValidity,
}

fn reference_verdict(receipt: &SignedReceipt, now: u64) -> Verdict {
    let table = [("alice", vk(7)), ("bob", vk(9))];
    let body = &receipt.body;
    if !table.contains(&(body.signer.as_str(), body.signer_key)) {
        return Verdict::NotSignersKey;
    }
    let key = VerifyingKey::from_bytes(&body.signer_key).unwrap();
    if key
        .verify(
            &reference_bytes(body),
            &Signature::from_bytes(&receipt.signature),
        )
        .is_err()
    {
        return Verdict::BadSignature;
    }
    if now < body.valid_from || now > body.valid_until {
        return Verdict::OutsideValidity;
    }
    Verdict::Valid
}

fn verdict_of(result: &Result<b19::VerifiedReceipt, ReceiptError>) -> Verdict {
    match result {
        Ok(_) => Verdict::Valid,
        Err(ReceiptError::Binding(_)) => Verdict::NotSignersKey,
        Err(ReceiptError::BadSignature) => Verdict::BadSignature,
        Err(ReceiptError::NotYetValid | ReceiptError::Expired) => Verdict::OutsideValidity,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

fn gate() -> IngestGate {
    IngestGate::new(
        vec![SignerAuthorization {
            identity: "bob".into(),
            verifying_key: vk(9),
            scopes: std::iter::once("sync".to_owned()).collect(),
        }],
        FreshnessWindow {
            max_age: 600,
            max_future_skew: 30,
        },
    )
}

/// One fixed receipt of each kind.
fn golden_receipts() -> [SignedReceipt; 3] {
    let issuer = alice();
    let envelope = sign_envelope(&key(9), "bob", "sync", b"hello, receipts", [0xE1; 32], NOW);
    let (_, accepted) = issuer
        .accept(&mut gate(), IngressPath::Remote, &envelope, ctx(1))
        .unwrap();
    let retained = issuer
        .retain_until(sha(b"hello, receipts"), "sync", NOW + 7_200, ctx(2))
        .unwrap();
    let present = issuer
        .verify_present(b"hello, receipts", [0xC4; 32], "sync", ctx(3))
        .unwrap();
    [accepted, retained, present]
}

/// SHA-256 of each golden receipt's canonical bytes, and its signature.
const GOLDEN: [(&str, &str); 3] = [
    (
        "2645ac75bc86f16839898beb17e862f1497e6cb2285df6b68e3c918520c54b43",
        "4e5c796012a4b454b232a4ece2371ee242440930f32b4f489fd8867ca9459e75375d6b682aabebffc47f090f2b3e04ece0bcb9c7448f60610878336c0d4a0602",
    ),
    (
        "337e55f747f8e5a8d4287bde41c60c04937cfd3e52276a6f8c82bbbf44538d2f",
        "078986dd681390b03f3028980eb7a31e16aaf156910a15d17b4254dd722d789afffec67f627a6047c35eb66c84fa65b2c6d1a34f9ce7beabf81148d3793e9405",
    ),
    (
        "54897bcd56caea95fa26bd3d086bcdc26e6afcd42b8794dc9029d24e94674a1b",
        "044208af8b5f23fed1957120373c30147c47ec95bad1dd7db91ac60222faa8da2ca83608863513771bd904473abc41afafd35ab6df5e9b2a706251cfc84e0808",
    ),
];

#[test]
fn golden_vectors_are_fixed_and_match_the_reference_encoding() {
    let receipts = golden_receipts();
    for (receipt, (body_digest, signature)) in receipts.iter().zip(GOLDEN) {
        let bytes = receipt.body.canonical_bytes();
        assert_eq!(bytes, reference_bytes(&receipt.body));
        assert_eq!(hex(&sha(&bytes)), body_digest);
        assert_eq!(hex(&receipt.signature), signature);
        assert_eq!(reference_verdict(receipt, NOW), Verdict::Valid);
        assert!(verify(&registry(), receipt, NOW).is_ok());
        assert_eq!(&SignedReceipt::decode(&receipt.encode()).unwrap(), receipt);
    }
    let kinds: Vec<Assertion> = receipts.iter().map(|r| r.body.claim.assertion()).collect();
    assert_eq!(kinds, Assertion::ALL.to_vec());
    assert_eq!(receipts[0].body.payload_digest, sha(b"hello, receipts"));
    assert_eq!(
        receipts[2].body.claim,
        ReceiptClaim::VerifiedPresent {
            challenge: [0xC4; 32],
            response: presence_response(&[0xC4; 32], b"hello, receipts"),
        }
    );
}

#[test]
fn the_three_assertions_are_distinct_signed_claims() {
    let names: Vec<&str> = Assertion::ALL.iter().map(|a| a.as_str()).collect();
    assert_eq!(names, ["accepted", "retained_until", "verified_present"]);
    let receipts = golden_receipts();
    let claims: Vec<ReceiptClaim> = receipts.iter().map(|r| r.body.claim).collect();
    // Re-labelling a receipt as another kind breaks its signature.
    for receipt in &receipts {
        for claim in &claims {
            if *claim == receipt.body.claim {
                continue;
            }
            let mut relabelled = receipt.clone();
            relabelled.body.claim = *claim;
            assert_eq!(
                verify(&registry(), &relabelled, NOW),
                Err(ReceiptError::BadSignature)
            );
            assert_eq!(reference_verdict(&relabelled, NOW), Verdict::BadSignature);
        }
    }
}

#[test]
fn stored_unverified_never_admits_content() {
    let [accepted, retained, present] = golden_receipts();
    let holding =
        |r: &SignedReceipt| Holding::Receipt(Box::new(verify(&registry(), r, NOW).unwrap()));
    assert!(holding(&accepted).admits_content());
    assert!(!holding(&retained).admits_content());
    assert!(!holding(&present).admits_content());
    assert_eq!(
        verify(&registry(), &present, NOW).unwrap().assertion(),
        Assertion::VerifiedPresent
    );
    let observed = Holding::StoredUnverified(InventoryObservation {
        payload_digest: accepted.body.payload_digest,
        observed_at: NOW,
        location: "blob-store".into(),
    });
    assert!(!observed.admits_content());

    // No wire tag can carry it: every tag but the three is refused.
    let tag_at = b"id-sync/b19-receipt/v1".len() + 32;
    for tag in (0..=u8::MAX).filter(|t| !(1..=3).contains(t)) {
        let mut bytes = accepted.encode();
        bytes[tag_at] = tag;
        assert_eq!(
            SignedReceipt::decode(&bytes),
            Err(ReceiptError::UnknownAssertion(tag))
        );
    }
}

#[test]
fn accepted_is_issued_only_through_the_gate() {
    let issuer = alice();
    let mut gate = gate();
    let mut unsigned = sign_envelope(&key(9), "bob", "sync", b"x", [1; 32], NOW);
    unsigned.signature = None;
    for path in IngressPath::ALL {
        assert_eq!(
            issuer.accept(&mut gate, path, &unsigned, ctx(1)),
            Err(ReceiptError::Rejected(RejectReason::Unsigned))
        );
    }
    let unauthorized = sign_envelope(&key(7), "alice", "sync", b"x", [2; 32], NOW);
    assert_eq!(
        issuer.accept(&mut gate, IngressPath::Local, &unauthorized, ctx(2)),
        Err(ReceiptError::Rejected(RejectReason::UnknownSigner))
    );
    assert_eq!(gate.admitted_count(), 0);

    let envelope = sign_envelope(&key(9), "bob", "sync", b"x", [3; 32], NOW);
    let (admission, receipt) = issuer
        .accept(&mut gate, IngressPath::Remote, &envelope, ctx(3))
        .unwrap();
    assert_eq!(admission.event_id, [3; 32]);
    assert!(gate.is_admitted(&[3; 32]));
    assert_eq!(receipt.body.payload_digest, sha(b"x"));
    assert_eq!(receipt.body.scope, "sync");
    assert_eq!(receipt.body.policy_digest, POLICY);
    assert_eq!(
        issuer.accept(&mut gate, IngressPath::Remote, &envelope, ctx(4)),
        Err(ReceiptError::Rejected(RejectReason::Replay))
    );
}

#[test]
fn signer_binding_is_checked_before_the_signature() {
    let [accepted, ..] = golden_receipts();
    // Bob's key, claiming to be alice: a valid signature, wrong identity.
    let mut forged = accepted.clone();
    forged.body.signer_key = vk(9);
    forged.signature = key(9).sign_bytes(&forged.body.canonical_bytes());
    assert!(matches!(
        verify(&registry(), &forged, NOW),
        Err(ReceiptError::Binding(BindingError::IdentityMismatch { .. }))
    ));
    // An unbound key with a valid signature.
    let mut unbound = accepted.clone();
    unbound.body.signer_key = vk(13);
    unbound.signature = key(13).sign_bytes(&unbound.body.canonical_bytes());
    assert!(matches!(
        verify(&registry(), &unbound, NOW),
        Err(ReceiptError::Binding(BindingError::UnboundKey { .. }))
    ));
    // The right key with a broken signature.
    let mut broken = accepted;
    broken.signature[5] ^= 0x10;
    assert_eq!(
        verify(&registry(), &broken, NOW),
        Err(ReceiptError::BadSignature)
    );
}

#[test]
fn validity_and_retention_are_bounded() {
    let [accepted, ..] = golden_receipts();
    let registry = registry();
    assert_eq!(
        verify(&registry, &accepted, NOW - 1),
        Err(ReceiptError::NotYetValid)
    );
    assert!(verify(&registry, &accepted, NOW + 3_600).is_ok());
    assert_eq!(
        verify(&registry, &accepted, NOW + 3_601),
        Err(ReceiptError::Expired)
    );

    let issuer = alice();
    assert_eq!(
        issuer.retain_until([0; 32], "sync", NOW + MAX_RETENTION + 1, ctx(9)),
        Err(ReceiptError::UnboundedRetention {
            cap: NOW + MAX_RETENTION
        })
    );
    assert_eq!(
        issuer.retain_until([0; 32], "sync", u64::MAX, ctx(9)),
        Err(ReceiptError::UnboundedRetention {
            cap: NOW + MAX_RETENTION
        })
    );
    assert_eq!(
        issuer.retain_until([0; 32], "sync", NOW, ctx(9)),
        Err(ReceiptError::InvalidInterval)
    );
    let backwards = ReceiptContext {
        valid_from: NOW + 1,
        valid_until: NOW,
        ..ctx(9)
    };
    assert_eq!(
        issuer.verify_present(b"x", [0; 32], "sync", backwards),
        Err(ReceiptError::InvalidInterval)
    );
}

#[test]
fn presence_is_checkable_by_anyone_holding_the_content() {
    let [accepted, _, present] = golden_receipts();
    assert_eq!(check_presence(&present.body, b"hello, receipts"), Ok(()));
    assert_eq!(
        check_presence(&present.body, b"hello, receipt"),
        Err(ReceiptError::PresenceMismatch)
    );
    assert_eq!(
        check_presence(&accepted.body, b"hello, receipts"),
        Err(ReceiptError::PresenceMismatch)
    );
    // A response forged without the content does not check out.
    let mut forged = present.body;
    forged.claim = ReceiptClaim::VerifiedPresent {
        challenge: [0xC4; 32],
        response: sha(b"guess"),
    };
    assert_eq!(
        check_presence(&forged, b"hello, receipts"),
        Err(ReceiptError::PresenceMismatch)
    );
}

#[test]
fn receipts_are_retained_by_unique_id() {
    let [accepted, retained, _] = golden_receipts();
    let registry = registry();
    let mut ledger = ReceiptLedger::new();
    assert!(ledger.is_empty());
    ledger
        .record(verify(&registry, &accepted, NOW).unwrap())
        .unwrap();
    ledger
        .record(verify(&registry, &accepted, NOW).unwrap())
        .unwrap();
    assert_eq!(ledger.len(), 1);
    let clash = alice()
        .retain_until(retained.body.payload_digest, "sync", NOW + 60, ctx(1))
        .unwrap();
    assert_eq!(
        ledger.record(verify(&registry, &clash, NOW).unwrap()),
        Err(ReceiptError::DuplicateId([1; 32]))
    );
    assert_eq!(ledger.get(&[1; 32]).unwrap().receipt, accepted);
    assert_eq!(ledger.get(&[1; 32]).unwrap().identity.identity, "alice");
}

trait SignBytes {
    fn sign_bytes(&self, bytes: &[u8]) -> [u8; 64];
}

impl SignBytes for SigningKey {
    fn sign_bytes(&self, bytes: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer;
        self.sign(bytes).to_bytes()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn slice_and_reference_agree_and_no_corruption_verifies(
        kind in 0_u8..3,
        id in any::<[u8; 32]>(),
        digest in any::<[u8; 32]>(),
        scope in "[a-z]{1,12}",
        epoch in any::<u64>(),
        span in 0_u64..10_000,
        until in 1_u64..MAX_RETENTION,
        now in (NOW - 100)..(NOW + 10_100),
        flip in any::<prop::sample::Index>(),
        bit in 0_u8..8,
    ) {
        let issuer = alice();
        let ctx = ReceiptContext {
            receipt_id: id,
            recipient_key: vk(9),
            issued_at: NOW,
            epoch,
            valid_from: NOW,
            valid_until: NOW + span,
        };
        let receipt = match kind {
            0 => issuer.retain_until(digest, &scope, NOW + until, ctx).unwrap(),
            1 => issuer.verify_present(&digest, id, &scope, ctx).unwrap(),
            _ => {
                let envelope = sign_envelope(&key(9), "bob", "sync", &digest, id, NOW);
                issuer.accept(&mut gate(), IngressPath::Remote, &envelope, ctx).unwrap().1
            }
        };
        prop_assert_eq!(receipt.body.canonical_bytes(), reference_bytes(&receipt.body));
        let result = verify(&registry(), &receipt, now);
        prop_assert_eq!(verdict_of(&result), reference_verdict(&receipt, now));

        let mut bytes = receipt.encode();
        let at = flip.index(bytes.len());
        bytes[at] ^= 1 << bit;
        if let Ok(decoded) = SignedReceipt::decode(&bytes) {
            prop_assert!(verify(&registry(), &decoded, NOW).is_err());
        }
    }
}
