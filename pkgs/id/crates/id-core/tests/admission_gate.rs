//! Universal signed-and-validated admission gate
//! (slice A9120-signature-gate, issue #10275).
//!
//! Oracle: **model** — ingress model: every accepted state
//! transition has verified signature and all validation decisions;
//! no route bypass exists. The proptest generates admission
//! operations (valid local producers, tampered payloads, forged
//! signatures, unknown and rebound signers, wrong principals,
//! wrong scopes, oversized payloads, unadmitted domains, garbage
//! wire bytes, non-canonical constructions, duplicate admissions)
//! against an independent reference model over plain values; every
//! resulting accepted set and rejection class must match. Targeted
//! tests pin the receipt fields, the no-unsigned-local-exception
//! rule, the wire/constructed path equivalence and the context
//! validation the generator is unlikely to hit.
//!
//! The crate-root export of `admission_gate` belongs to slice
//! 9110-02, so this test includes it, and the modules it builds
//! on, by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../src/admission_gate.rs"]
#[allow(dead_code)]
mod admission_gate;

use std::collections::BTreeSet;

use admission_gate::{
    AdmissionContext, AdmissionGate, AdmissionReceipt, AdmissionRejection, GatePolicy,
    ScopeBindingAuthorizer, StaticSignerDirectory,
};
use ed25519_dalek::{Signer as _, SigningKey};
use id_shape::HashProfile;
use proptest::prelude::*;
use signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519, signature_preimage};

const RAW: HashProfile = HashProfile::RawBlake3_256;
const DOMAIN: &str = "id/atlas";
const OTHER_DOMAIN: &str = "id/other";
const TENANTS: [&str; 2] = ["tenant-a", "tenant-b"];
const SIGNERS: [(&str, u8); 2] = [("alice", 11), ("bob", 22)];
const MALLORY: (&str, u8) = ("mallory", 33);
const MAX_PAYLOAD: u64 = 64;

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public_key(seed: u8) -> [u8; 32] {
    signing_key(seed).verifying_key().to_bytes()
}

fn context(principal: &str, tenant: &str) -> AdmissionContext {
    AdmissionContext::new(principal, tenant, "admission:write").unwrap()
}

fn directory() -> StaticSignerDirectory {
    let mut directory = StaticSignerDirectory::new();
    for (signer_id, seed) in SIGNERS {
        directory.register(signer_id, public_key(seed));
    }
    directory
}

fn gate() -> AdmissionGate {
    AdmissionGate::new(
        GatePolicy::admitting([DOMAIN.to_string()]).with_max_payload_bytes(MAX_PAYLOAD),
        directory(),
        ScopeBindingAuthorizer,
    )
}

/// A gate whose directory binds alice to bob's key: every alice
/// admission must fail signer binding even though her envelopes
/// are self-consistent.
fn poisoned_gate() -> AdmissionGate {
    let mut directory = StaticSignerDirectory::new();
    directory.register("alice", public_key(22));
    directory.register("bob", public_key(22));
    AdmissionGate::new(
        GatePolicy::admitting([DOMAIN.to_string()]).with_max_payload_bytes(MAX_PAYLOAD),
        directory,
        ScopeBindingAuthorizer,
    )
}

fn make_envelope(
    signer_id: &str,
    seed: u8,
    tenant: &str,
    domain: &str,
    payload: Vec<u8>,
) -> Envelope {
    let key = signing_key(seed);
    let body = Body {
        domain: domain.to_string(),
        context: tenant.to_string(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: signer_id.to_string(),
        key: key.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_string(),
        payload_id: PayloadId::of(RAW, &payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    Envelope::sign(body, payload, &key).unwrap()
}

fn valid_envelope(signer: usize, tenant: usize, seed: u8) -> Envelope {
    let (signer_id, key_seed) = SIGNERS[signer];
    make_envelope(signer_id, key_seed, TENANTS[tenant], DOMAIN, vec![seed; 8])
}

fn receipt_digest(receipt: &AdmissionReceipt) -> Vec<u8> {
    receipt.envelope_id().content_id().digest().to_vec()
}

fn envelope_digest(envelope: &Envelope) -> Vec<u8> {
    envelope.id(RAW).unwrap().content_id().digest().to_vec()
}

/// A gate outcome reduced to what the model predicts: the
/// admitted envelope digest, or the rejection's class.
fn outcome(result: Result<AdmissionReceipt, AdmissionRejection>) -> Result<Vec<u8>, &'static str> {
    result
        .map(|receipt| receipt_digest(&receipt))
        .map_err(|rejection| rejection.class())
}

// ---------------------------------------------------------------------
// Targeted tests
// ---------------------------------------------------------------------

#[test]
fn valid_local_producer_is_admitted_with_a_full_receipt() {
    let envelope = valid_envelope(0, 0, 7);
    let receipt = gate()
        .admit(&envelope, &context("alice", "tenant-a"))
        .unwrap();
    assert_eq!(receipt_digest(&receipt), envelope_digest(&envelope));
    assert_eq!(
        receipt.payload_id(),
        &PayloadId::of(RAW, &vec![7u8; 8]).unwrap()
    );
    assert_eq!(receipt.signer_id(), "alice");
    assert_eq!(receipt.tenant_scope(), "tenant-a");
    assert_eq!(receipt.payload_length(), 8);
}

#[test]
fn unsigned_envelope_is_rejected_no_local_exception() {
    // Invariant 1: local producers sign before entering the gate;
    // an unsigned envelope is refused like any forgery.
    let mut envelope = valid_envelope(0, 0, 7);
    envelope.signature = [0u8; 64];
    let rejection = gate()
        .admit(&envelope, &context("alice", "tenant-a"))
        .unwrap_err();
    assert_eq!(rejection, AdmissionRejection::BadSignature);
    assert_eq!(rejection.class(), "bad_signature");
}

#[test]
fn canonical_wire_path_matches_the_constructed_path() {
    // No route bypass: wire bytes and constructed envelopes face
    // the identical pipeline and mint identical receipts.
    let gate = gate();
    let envelope = valid_envelope(1, 1, 9);
    let ctx = context("bob", "tenant-b");
    let via_constructed = gate.admit(&envelope, &ctx).unwrap();
    let via_wire = gate
        .admit_canonical(&envelope.canonical_bytes().unwrap(), &ctx)
        .unwrap();
    assert_eq!(via_constructed, via_wire);

    let bytes = envelope.canonical_bytes().unwrap();
    let truncated = &bytes[..bytes.len() - 3];
    let rejection = gate.admit_canonical(truncated, &ctx).unwrap_err();
    assert_eq!(rejection.class(), "malformed");
}

#[test]
fn default_policy_admits_any_domain_within_the_default_budget() {
    let gate = AdmissionGate::new(GatePolicy::default(), directory(), ScopeBindingAuthorizer);
    let envelope = make_envelope("alice", 11, "tenant-a", OTHER_DOMAIN, vec![3u8; 8]);
    let receipt = gate
        .admit(&envelope, &context("alice", "tenant-a"))
        .unwrap();
    assert_eq!(receipt.tenant_scope(), "tenant-a");
}

#[test]
fn context_fields_are_validated() {
    assert_eq!(
        AdmissionContext::new("", "tenant-a", "admission:write")
            .unwrap_err()
            .class(),
        "malformed"
    );
    assert_eq!(
        AdmissionContext::new("alice", "tenant a", "admission:write")
            .unwrap_err()
            .class(),
        "malformed"
    );
    assert_eq!(
        AdmissionContext::new("alice", "tenant-a", "x".repeat(256))
            .unwrap_err()
            .class(),
        "malformed"
    );
    assert!(AdmissionContext::new("alice", "tenant-a", "x".repeat(255)).is_ok());
}

// ---------------------------------------------------------------------
// Model proptest
// ---------------------------------------------------------------------

/// One generated admission operation, as raw indices the harness
/// interprets against the fixed actor table.
type Op = (u8, u8, u8, u8);

fn op_strategy() -> impl Strategy<Value = Op> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>())
}

/// The reference model: the set of envelope digests the gate may
/// have accepted, over plain values. The harness computes each
/// operation's expected outcome from the model's own rules, runs
/// the gate, and compares outcome and accepted set after every
/// operation.
struct Model {
    admitted: BTreeSet<Vec<u8>>,
    last_valid: Option<(Envelope, AdmissionContext)>,
}

impl Model {
    fn new() -> Self {
        Self {
            admitted: BTreeSet::new(),
            last_valid: None,
        }
    }
}

fn run_ops(ops: Vec<Op>) -> Result<(), TestCaseError> {
    let gate = gate();
    let poisoned = poisoned_gate();
    let mut model = Model::new();

    for (kind, a, b, c) in ops {
        let signer = usize::from(a % 2);
        let tenant = usize::from(b % 2);
        let (signer_id, _) = SIGNERS[signer];
        match kind % 12 {
            0 => {
                // Valid local producer admission.
                let envelope = valid_envelope(signer, tenant, c);
                let ctx = context(signer_id, TENANTS[tenant]);
                let digest = envelope_digest(&envelope);
                let result = gate.admit(&envelope, &ctx);
                prop_assert_eq!(outcome(result), Ok(digest.clone()));
                model.admitted.insert(digest);
                model.last_valid = Some((envelope, ctx));
            }
            1 => {
                // Tampered payload: digest no longer matches.
                let mut envelope = valid_envelope(signer, tenant, c);
                envelope.payload[0] ^= 0xff;
                let result = gate.admit(&envelope, &context(signer_id, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("payload"));
            }
            2 => {
                // Forged signature: another key signed the preimage.
                let mut envelope = valid_envelope(signer, tenant, c);
                let other_seed = SIGNERS[1 - signer].1;
                envelope.signature = signing_key(other_seed)
                    .sign(&signature_preimage(&envelope.body).unwrap())
                    .to_bytes();
                let result = gate.admit(&envelope, &context(signer_id, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("bad_signature"));
            }
            3 => {
                // Unknown signer: self-consistent, unregistered.
                let envelope =
                    make_envelope(MALLORY.0, MALLORY.1, TENANTS[tenant], DOMAIN, vec![c; 8]);
                let result = gate.admit(&envelope, &context(MALLORY.0, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("signer_binding"));
            }
            4 => {
                // Rebound key: the directory binds alice to bob's key.
                let envelope = valid_envelope(0, tenant, c);
                let result = poisoned.admit(&envelope, &context("alice", TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("signer_binding"));
            }
            5 => {
                // Wrong principal: a different registered signer
                // presents alice's envelope.
                let envelope = valid_envelope(signer, tenant, c);
                let other = SIGNERS[1 - signer].0;
                let result = gate.admit(&envelope, &context(other, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("signer_binding"));
            }
            6 => {
                // Wrong scope: authenticated for the other tenant.
                let envelope = valid_envelope(signer, 0, c);
                let result = gate.admit(&envelope, &context(signer_id, TENANTS[1]));
                prop_assert_eq!(outcome(result), Err("unauthorized"));
            }
            7 => {
                // Oversized payload: valid in every way except the
                // policy budget.
                let (sid, seed) = SIGNERS[signer];
                let envelope = make_envelope(sid, seed, TENANTS[tenant], DOMAIN, vec![c; 65]);
                let result = gate.admit(&envelope, &context(sid, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("policy"));
            }
            8 => {
                // Unadmitted domain.
                let (sid, seed) = SIGNERS[signer];
                let envelope = make_envelope(sid, seed, TENANTS[tenant], OTHER_DOMAIN, vec![c; 8]);
                let result = gate.admit(&envelope, &context(sid, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("policy"));
            }
            9 => {
                // Garbage wire bytes never decode.
                let result = gate.admit_canonical(&[0xff; 24], &context("alice", "tenant-a"));
                prop_assert_eq!(outcome(result), Err("malformed"));
            }
            10 => {
                // Non-canonical construction: empty domain label.
                let payload = vec![c; 8];
                let body = Body {
                    domain: String::new(),
                    context: TENANTS[tenant].to_string(),
                    subject: None,
                    graph: None,
                    predecessor: None,
                    signer_id: signer_id.to_string(),
                    key: public_key(SIGNERS[signer].1),
                    suite: SUITE_ED25519.to_string(),
                    payload_id: PayloadId::of(RAW, &payload).unwrap(),
                    payload_length: payload.len() as u64,
                };
                let envelope = Envelope {
                    body,
                    payload,
                    signature: [9u8; 64],
                };
                let result = gate.admit(&envelope, &context(signer_id, TENANTS[tenant]));
                prop_assert_eq!(outcome(result), Err("malformed"));
            }
            _ => {
                // Duplicate admission of the last valid envelope:
                // idempotent acceptance, same receipt, set unchanged.
                if let Some((envelope, ctx)) = &model.last_valid {
                    let digest = envelope_digest(envelope);
                    let result = gate.admit(envelope, ctx);
                    prop_assert_eq!(outcome(result), Ok(digest.clone()));
                    prop_assert!(model.admitted.contains(&digest));
                } else {
                    let envelope = valid_envelope(signer, tenant, c);
                    let ctx = context(signer_id, TENANTS[tenant]);
                    let digest = envelope_digest(&envelope);
                    let result = gate.admit(&envelope, &ctx);
                    prop_assert_eq!(outcome(result), Ok(digest.clone()));
                    model.admitted.insert(digest);
                    model.last_valid = Some((envelope, ctx));
                }
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn generated_operations_match_the_model(
        ops in prop::collection::vec(op_strategy(), 0..=24)
    ) {
        run_ops(ops)?;
    }
}
