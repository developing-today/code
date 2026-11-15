//! B01 key lifecycle — integration test (slice B01, #10183).
//!
//! Oracle (invariant, from the issue): a rotation preserves
//! verification/attribution of an event signed under the prior valid
//! key; key lineage is explicit.
//!
//! Independence: signatures here are produced by ed25519-dalek
//! directly and by a hard-coded RFC 8032 golden vector — never by
//! the module under test, which only verifies and manages lineage.
//! The federation roots deliberately do not declare slice modules
//! (slice 9110-02), so this test includes the slice file by path;
//! #9110-09 wires versioned public exports later.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b01_key_lifecycle.rs"]
mod b01_key_lifecycle;

use b01_key_lifecycle::{KeyLineage, LifecycleError};
use ed25519_dalek::{Signer, SigningKey};

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("test vector hex"))
        .collect()
}

fn keypair(seed: [u8; 32]) -> (SigningKey, [u8; 32]) {
    let sk = SigningKey::from_bytes(&seed);
    let vk = sk.verifying_key().to_bytes();
    (sk, vk)
}

fn sign(sk: &SigningKey, msg: &[u8]) -> [u8; 64] {
    sk.sign(msg).to_bytes()
}

/// RFC 8032 section 7.1, TEST 3: an independent golden vector
/// (verify-only: public key, message, signature).
const RFC_PUBLIC: &str = "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025";
const RFC_MESSAGE: &str = "af82";
const RFC_SIGNATURE: &str = "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a";

#[test]
fn rotation_preserves_prior_key_events_with_attribution() {
    let (sk_a, key_a) = keypair([3u8; 32]);
    let (_sk_b, key_b) = keypair([9u8; 32]);
    let mut lineage = KeyLineage::new();
    assert_eq!(lineage.enroll(key_a, 100).unwrap(), 1);

    // Event signed while generation 1 is the only, current key.
    let event = b"federation event signed under generation 1";
    let sig = sign(&sk_a, event);

    // Rotate at t=200 with a 50-second overlap for generation 1.
    assert_eq!(lineage.rotate(key_b, 200, 50).unwrap(), 2);

    // The invariant: the pre-rotation event still verifies, and it
    // attributes to generation 1 — not to the current generation.
    let att = lineage.verify_at(&key_a, event, &sig, 110).unwrap();
    assert_eq!(att.generation, 1);
    assert_eq!(att.fingerprint, lineage.generations()[0].fingerprint());
}

#[test]
fn overlap_window_bounds_old_key_signing_not_history() {
    let (sk_a, key_a) = keypair([3u8; 32]);
    let (sk_b, key_b) = keypair([9u8; 32]);
    let mut lineage = KeyLineage::new();
    lineage.enroll(key_a, 100).unwrap();
    lineage.rotate(key_b, 200, 50).unwrap();

    // Inside the overlap (t=240 < 250): the old key still signs.
    let in_overlap = sign(&sk_a, b"signed during overlap");
    assert_eq!(
        lineage
            .verify_at(&key_a, b"signed during overlap", &in_overlap, 240)
            .unwrap()
            .generation,
        1
    );
    // After the overlap closes (t=260): the old key may not sign new
    // events, even though its signature is cryptographically valid.
    let after = sign(&sk_a, b"signed after overlap");
    assert_eq!(
        lineage.verify_at(&key_a, b"signed after overlap", &after, 260),
        Err(LifecycleError::KeyNotValidAtTime {
            generation: 1,
            at: 260
        })
    );
    // The new key signs from the rotation instant, not before.
    let by_b = sign(&sk_b, b"signed by generation 2");
    assert_eq!(
        lineage
            .verify_at(&key_b, b"signed by generation 2", &by_b, 210)
            .unwrap()
            .generation,
        2
    );
    assert_eq!(
        lineage.verify_at(&key_b, b"signed by generation 2", &by_b, 150),
        Err(LifecycleError::KeyNotValidAtTime {
            generation: 2,
            at: 150
        })
    );
}

#[test]
fn rfc8032_golden_vector_verifies_through_lineage_after_rotation() {
    let public: [u8; 32] = hex_bytes(RFC_PUBLIC).try_into().unwrap();
    let message = hex_bytes(RFC_MESSAGE);
    let signature: [u8; 64] = hex_bytes(RFC_SIGNATURE).try_into().unwrap();

    let (_sk_b, key_b) = keypair([9u8; 32]);
    let mut lineage = KeyLineage::new();
    lineage.enroll(public, 100).unwrap();
    lineage.rotate(key_b, 200, 50).unwrap();
    // The golden-vector event, signed at t=120 under generation 1,
    // verifies after rotation and attributes to generation 1.
    let att = lineage
        .verify_at(&public, &message, &signature, 120)
        .unwrap();
    assert_eq!(att.generation, 1);
}

#[test]
fn unknown_key_and_bad_signature_are_distinct_failures() {
    let (sk_a, key_a) = keypair([3u8; 32]);
    let (sk_c, key_c) = keypair([42u8; 32]);
    let mut lineage = KeyLineage::new();
    lineage.enroll(key_a, 100).unwrap();

    let by_stranger = sign(&sk_c, b"hello");
    assert_eq!(
        lineage.verify_at(&key_c, b"hello", &by_stranger, 110),
        Err(LifecycleError::UnknownKey)
    );
    let genuine = sign(&sk_a, b"hello");
    assert_eq!(
        lineage.verify_at(&key_a, b"tampered", &genuine, 110),
        Err(LifecycleError::BadSignature)
    );
}

#[test]
fn lineage_chain_is_explicit_and_rotations_are_monotone() {
    let (_a, key_a) = keypair([3u8; 32]);
    let (_b, key_b) = keypair([9u8; 32]);
    let (_c, key_c) = keypair([42u8; 32]);
    let mut lineage = KeyLineage::new();
    lineage.enroll(key_a, 100).unwrap();
    lineage.rotate(key_b, 200, 50).unwrap();
    lineage.rotate(key_c, 300, 0).unwrap();

    let gens = lineage.generations();
    assert_eq!(gens.len(), 3);
    assert_eq!(gens[0].supersedes, None);
    assert_eq!(gens[0].superseded_by, Some(2));
    assert_eq!(gens[0].valid_until, Some(250));
    assert_eq!(gens[1].supersedes, Some(1));
    assert_eq!(gens[1].superseded_by, Some(3));
    assert_eq!(gens[1].valid_until, Some(300));
    assert_eq!(gens[2].supersedes, Some(2));
    assert_eq!(gens[2].superseded_by, None);
    assert_eq!(gens[2].valid_until, None);
    assert_eq!(lineage.current().unwrap().generation, 3);

    // Rotations must move forward in time and never reuse a key.
    assert!(matches!(
        lineage.rotate(key_a, 400, 0),
        Err(LifecycleError::DuplicateKey)
    ));
    let (_d, key_d) = keypair([77u8; 32]);
    assert!(matches!(
        lineage.rotate(key_d, 300, 0),
        Err(LifecycleError::NonMonotonicRotation { .. })
    ));
    assert!(matches!(
        lineage.enroll(key_d, 500),
        Err(LifecycleError::AlreadyEnrolled)
    ));
}
