//! B32 golden test: the mandatory universal signed-and-validated
//! acceptance gate (#10233, statements S1.12-S1.20).
//!
//! Oracle: golden - unsigned, invalid, and wrong-signer inputs
//! fail before admission on every ingress path (local, loopback,
//! internal, hosted, remote), and a valid local producer signs
//! before passing the same gate. The verifier is the independent
//! reference gate below (`reference_admit`): written directly
//! from the issue's oracle text as a literal checklist, sharing
//! no code with the implementation under test (it rebuilds the
//! preimage and digest itself and verifies with dalek directly).
//! The implementation is loaded by direct path because slice
//! 9110-09 - not this slice - wires the versioned public exports
//! after the leaf modules land (9110-02 registry rule).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
mod b32;

use std::collections::{BTreeSet, HashSet};

use b32::{
    Envelope, FreshnessWindow, IngestGate, IngressPath, RejectReason, SignerAuthorization,
    sign_envelope,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const NOW: u64 = 10_000;
const MAX_AGE: u64 = 600;
const MAX_SKEW: u64 = 30;

fn alice_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn bob_key() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}

fn carol_key() -> SigningKey {
    SigningKey::from_bytes(&[11u8; 32])
}

fn signer(identity: &str, key: &SigningKey, scopes: &[&str]) -> SignerAuthorization {
    SignerAuthorization {
        identity: identity.to_string(),
        verifying_key: key.verifying_key().to_bytes(),
        scopes: scopes
            .iter()
            .map(|scope| scope.to_string())
            .collect::<BTreeSet<_>>(),
    }
}

fn gate() -> IngestGate {
    IngestGate::new(
        vec![
            signer("alice", &alice_key(), &["sync", "query"]),
            signer("bob", &bob_key(), &["sync"]),
        ],
        FreshnessWindow {
            max_age: MAX_AGE,
            max_future_skew: MAX_SKEW,
        },
    )
}

fn valid_envelope() -> Envelope {
    sign_envelope(
        &alice_key(),
        "alice",
        "sync",
        b"hello gate",
        [42u8; 32],
        NOW,
    )
}

// ---------------------------------------------------------------------------
// Independent reference gate, from the oracle text: an envelope is
// admitted only when it is signed, its key is bound to the claimed
// signer, the signature verifies, the payload matches its declared
// digest, the signer is authorized for the scope, it is inside the
// freshness window, and its event id was not admitted before. The
// ingress path never changes the outcome.
// ---------------------------------------------------------------------------

fn reference_preimage(envelope: &Envelope) -> Vec<u8> {
    fn push(out: &mut Vec<u8>, field: &[u8]) {
        out.extend_from_slice(&(field.len() as u64).to_be_bytes());
        out.extend_from_slice(field);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"id-sync/b32-ingest-gate/v1");
    push(&mut out, envelope.signer.as_bytes());
    push(&mut out, &envelope.verifying_key);
    push(&mut out, envelope.scope.as_bytes());
    push(&mut out, &envelope.payload_digest);
    push(&mut out, &envelope.event_id);
    push(&mut out, &envelope.issued_at.to_be_bytes());
    out
}

fn reference_admit(
    admitted: &mut HashSet<[u8; 32]>,
    envelope: &Envelope,
    now: u64,
) -> Result<(), RejectReason> {
    let Some(signature) = envelope.signature else {
        return Err(RejectReason::Unsigned);
    };
    let table: [(&str, [u8; 32], &[&str]); 2] = [
        (
            "alice",
            alice_key().verifying_key().to_bytes(),
            &["sync", "query"],
        ),
        ("bob", bob_key().verifying_key().to_bytes(), &["sync"]),
    ];
    let Some((bound_to, _, scopes)) = table
        .iter()
        .find(|(_, key, _)| *key == envelope.verifying_key)
    else {
        return Err(RejectReason::UnknownSigner);
    };
    if *bound_to != envelope.signer {
        return Err(RejectReason::SignerMismatch {
            claimed: envelope.signer.clone(),
            bound_to: bound_to.to_string(),
        });
    }
    let key = VerifyingKey::from_bytes(&envelope.verifying_key)
        .map_err(|_| RejectReason::InvalidSignature)?;
    key.verify_strict(
        &reference_preimage(envelope),
        &Signature::from_bytes(&signature),
    )
    .map_err(|_| RejectReason::InvalidSignature)?;
    let mut hasher = Sha256::new();
    hasher.update(&envelope.payload);
    let digest: [u8; 32] = hasher.finalize().into();
    if envelope.payload.is_empty() || envelope.scope.is_empty() || digest != envelope.payload_digest
    {
        return Err(RejectReason::SemanticInvalid);
    }
    if !scopes.contains(&envelope.scope.as_str()) {
        return Err(RejectReason::NotAuthorized {
            scope: envelope.scope.clone(),
        });
    }
    if envelope.issued_at > now + MAX_SKEW {
        return Err(RejectReason::FutureSkew);
    }
    if now > envelope.issued_at + MAX_AGE {
        return Err(RejectReason::Stale);
    }
    if !admitted.insert(envelope.event_id) {
        return Err(RejectReason::Replay);
    }
    Ok(())
}

/// Run one single-defect (or valid) envelope through both gates on
/// every ingress path and require identical verdicts.
fn check_matrix(name: &str, envelope: &Envelope) {
    for path in IngressPath::ALL {
        let mut gate = gate();
        let got = gate.admit(path, envelope, NOW).map(|_| ());
        let mut admitted = HashSet::new();
        let want = reference_admit(&mut admitted, envelope, NOW);
        assert_eq!(got, want, "{name} on {path:?}");
    }
}

#[test]
fn valid_envelope_admitted_on_every_path() {
    check_matrix("valid", &valid_envelope());
}

#[test]
fn unsigned_rejected_on_every_path_including_local() {
    let mut envelope = valid_envelope();
    envelope.signature = None;
    check_matrix("unsigned", &envelope);
    let mut gate = gate();
    assert_eq!(
        gate.admit(IngressPath::Local, &envelope, NOW),
        Err(RejectReason::Unsigned)
    );
}

#[test]
fn invalid_signature_rejected_on_every_path() {
    let mut envelope = valid_envelope();
    let mut signature = envelope.signature.unwrap();
    signature[0] ^= 0xff;
    envelope.signature = Some(signature);
    check_matrix("invalid signature", &envelope);
}

#[test]
fn wrong_signer_rejected_on_every_path() {
    // Bob's key and signature, but the envelope claims alice: the
    // signature is cryptographically valid - binding rejects it.
    let mut envelope = sign_envelope(&bob_key(), "bob", "sync", b"hello gate", [42u8; 32], NOW);
    envelope.signer = "alice".to_string();
    // Re-sign so the signature matches the claimed-alice preimage;
    // binding, not signature validity, must be what rejects it.
    let signature = bob_key().sign(&envelope.preimage());
    envelope.signature = Some(signature.to_bytes());
    check_matrix("wrong signer", &envelope);
    let mut gate = gate();
    assert!(matches!(
        gate.admit(IngressPath::Remote, &envelope, NOW),
        Err(RejectReason::SignerMismatch { .. })
    ));
}

#[test]
fn unbound_key_rejected_on_every_path() {
    let envelope = sign_envelope(
        &carol_key(),
        "carol",
        "sync",
        b"hello gate",
        [42u8; 32],
        NOW,
    );
    check_matrix("unbound key", &envelope);
}

#[test]
fn tampered_payload_rejected_on_every_path() {
    let mut envelope = valid_envelope();
    envelope.payload = b"tampered".to_vec();
    check_matrix("tampered payload", &envelope);
}

#[test]
fn unauthorized_scope_rejected_on_every_path() {
    let envelope = sign_envelope(
        &alice_key(),
        "alice",
        "admin",
        b"hello gate",
        [42u8; 32],
        NOW,
    );
    check_matrix("unauthorized scope", &envelope);
}

#[test]
fn stale_and_future_rejected_on_every_path() {
    let stale = sign_envelope(
        &alice_key(),
        "alice",
        "sync",
        b"hello gate",
        [42u8; 32],
        NOW - MAX_AGE - 1,
    );
    check_matrix("stale", &stale);
    let future = sign_envelope(
        &alice_key(),
        "alice",
        "sync",
        b"hello gate",
        [42u8; 32],
        NOW + MAX_SKEW + 1,
    );
    check_matrix("future", &future);
}

#[test]
fn replay_rejected_on_every_path() {
    let envelope = valid_envelope();
    for path in IngressPath::ALL {
        let mut gate = gate();
        assert!(
            gate.admit(path, &envelope, NOW).is_ok(),
            "first on {path:?}"
        );
        assert_eq!(
            gate.admit(path, &envelope, NOW),
            Err(RejectReason::Replay),
            "replay on {path:?}"
        );
        assert_eq!(gate.admitted_count(), 1);
    }
}

#[test]
fn local_producer_signs_before_the_same_gate() {
    // A local producer's envelope is admitted only because it was
    // signed first; the unsigned form of the same production is
    // refused by the local path too (see the unsigned test).
    let envelope = valid_envelope();
    let mut gate = gate();
    let admission = gate.admit(IngressPath::Local, &envelope, NOW).unwrap();
    assert_eq!(admission.signer, "alice");
    assert_eq!(admission.scope, "sync");
    assert_eq!(admission.path, IngressPath::Local);
    assert!(gate.is_admitted(&envelope.event_id));
}

#[test]
fn any_signed_field_mutation_fails() {
    // PBT-style oracle from the design: arbitrary mutation of any
    // signed field causes verification failure.
    let base = valid_envelope();
    let mut mutations: Vec<Envelope> = Vec::new();
    let mut m = base.clone();
    m.signer = "bob".to_string();
    mutations.push(m);
    let mut m = base.clone();
    m.scope = "query".to_string();
    mutations.push(m);
    let mut m = base.clone();
    m.event_id = [43u8; 32];
    mutations.push(m);
    let mut m = base.clone();
    m.issued_at = NOW - 1;
    mutations.push(m);
    let mut m = base.clone();
    m.payload_digest = [1u8; 32];
    mutations.push(m);
    let mut m = base;
    m.verifying_key = bob_key().verifying_key().to_bytes();
    mutations.push(m);
    for mutation in &mutations {
        for path in IngressPath::ALL {
            let mut gate = gate();
            assert!(
                gate.admit(path, mutation, NOW).is_err(),
                "mutation on {path:?}"
            );
            assert_eq!(gate.admitted_count(), 0);
        }
    }
}

#[test]
fn empty_gate_fails_closed() {
    let mut gate = IngestGate::new(
        Vec::new(),
        FreshnessWindow {
            max_age: MAX_AGE,
            max_future_skew: MAX_SKEW,
        },
    );
    assert_eq!(
        gate.admit(IngressPath::Remote, &valid_envelope(), NOW),
        Err(RejectReason::UnknownSigner)
    );
}
