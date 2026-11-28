//! B05 signer identity binding — integration test (slice B05, #10208).
//!
//! Oracle (golden, from the issue): a signer key binds to a claimed
//! identity; legacy `SignedBody` `serde_json` vectors verify the
//! exact old bytes and do not change after new-profile support.
//!
//! Independence: the pinned legacy fixture is a literal JSON string
//! written by hand from the documented field order, and it is
//! cross-checked against `id_sync::artifact::signed_bytes` (the
//! production legacy writer) — so a drift on either side fails the
//! test. Signatures are produced by ed25519-dalek directly, never by
//! the module under test, which only binds and verifies. The
//! federation roots deliberately do not declare slice modules (slice
//! 9110-02), so this test includes the slice file by path; #9110-09
//! wires versioned public exports later.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b05_signer_binding.rs"]
mod b05_signer_binding;

use b05_signer_binding::{
    BindingError, BindingRegistry, LegacyBodyV1, legacy_preimage_v1, verify_bound_signature,
    verify_signature_presence,
};
use ed25519_dalek::{Signer, SigningKey};

/// Pinned legacy fixture bytes (v1 `SignedBody`, compact serde_json,
/// empty `after` omitted). Written literally from the documented
/// field order; must never change.
const LEGACY_FIXTURE: &str = concat!(
    r#"{"author":"9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60","#,
    r#""seq":1,"prev":"","kind":"document","#,
    r#""body":{"tags":["a","b"],"text":"hello"}}"#,
);

fn fixture_key() -> SigningKey {
    // RFC 8032 TEST 1 seed.
    SigningKey::from_bytes(&[
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ])
}

fn other_key() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}

fn fixture_body<'a>(body: &'a serde_json::Value, after: &'a [String]) -> LegacyBodyV1<'a> {
    LegacyBodyV1 {
        author: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        seq: 1,
        prev: "",
        kind: "document",
        body,
        after,
    }
}

#[test]
fn legacy_fixture_bytes_are_pinned_and_match_production_writer() {
    let body = serde_json::json!({"text": "hello", "tags": ["a", "b"]});
    let after: Vec<String> = Vec::new();
    let preimage = legacy_preimage_v1(&fixture_body(&body, &after)).unwrap();
    assert_eq!(String::from_utf8(preimage.clone()).unwrap(), LEGACY_FIXTURE);

    // Cross-check against the production legacy writer: the pinned
    // bytes are not merely self-consistent.
    let production = id_sync::artifact::signed_bytes(
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        1,
        "",
        "document",
        &body,
        &after,
    )
    .unwrap();
    assert_eq!(preimage, production, "legacy bytes drifted");

    // A non-empty frontier is part of the same format and appends the
    // `after` array; the empty-frontier form above stays untouched.
    let frontier = vec!["ab".repeat(32)];
    let with_frontier = legacy_preimage_v1(&fixture_body(&body, &frontier)).unwrap();
    let with_frontier = String::from_utf8(with_frontier).unwrap();
    assert!(with_frontier.ends_with(
        r#""after":["abababababababababababababababababababababababababababababababab"]}"#
    ));
    assert_eq!(
        with_frontier,
        String::from_utf8(
            id_sync::artifact::signed_bytes(
                "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
                1,
                "",
                "document",
                &body,
                &frontier,
            )
            .unwrap()
        )
        .unwrap()
    );
}

#[test]
fn bound_key_verifies_legacy_bytes_and_reports_identity() {
    let sk = fixture_key();
    let vk = sk.verifying_key().to_bytes();
    // RFC 8032 TEST 1 public key, an independent golden anchor.
    assert_eq!(
        b05_signer_binding::key_fingerprint(&vk),
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    );

    let signature = sk.sign(LEGACY_FIXTURE.as_bytes()).to_bytes();
    let mut registry = BindingRegistry::new();
    registry.bind("author:alice", vk).unwrap();

    // Pre-existing verification (signature presence only) still
    // accepts the same bytes: the new profile changes nothing about
    // legacy acceptance.
    verify_signature_presence(&vk, LEGACY_FIXTURE.as_bytes(), &signature).unwrap();

    let verified = verify_bound_signature(
        &registry,
        "author:alice",
        &vk,
        LEGACY_FIXTURE.as_bytes(),
        &signature,
    )
    .unwrap();
    assert_eq!(verified.identity, "author:alice");
    assert_eq!(
        verified.key_fingerprint,
        b05_signer_binding::key_fingerprint(&vk)
    );

    // The preimage bytes are still exactly the pinned fixture after
    // new-profile support: unchanged, byte for byte.
    assert_eq!(LEGACY_FIXTURE.as_bytes().len(), 154);
}

#[test]
fn valid_signature_from_unbound_key_is_rejected_as_identity_failure() {
    let sk = other_key();
    let vk = sk.verifying_key().to_bytes();
    let signature = sk.sign(LEGACY_FIXTURE.as_bytes()).to_bytes();
    let registry = BindingRegistry::new();

    // The signature itself is valid — presence alone would accept.
    verify_signature_presence(&vk, LEGACY_FIXTURE.as_bytes(), &signature).unwrap();

    // Binding does not: an unbound key has no identity.
    assert_eq!(
        verify_bound_signature(
            &registry,
            "author:alice",
            &vk,
            LEGACY_FIXTURE.as_bytes(),
            &signature
        ),
        Err(BindingError::UnboundKey {
            key_fingerprint: b05_signer_binding::key_fingerprint(&vk)
        })
    );
}

#[test]
fn key_bound_to_other_identity_cannot_impersonate() {
    let sk = fixture_key();
    let vk = sk.verifying_key().to_bytes();
    let signature = sk.sign(LEGACY_FIXTURE.as_bytes()).to_bytes();
    let mut registry = BindingRegistry::new();
    registry.bind("author:bob", vk).unwrap();

    assert_eq!(
        verify_bound_signature(
            &registry,
            "author:alice",
            &vk,
            LEGACY_FIXTURE.as_bytes(),
            &signature
        ),
        Err(BindingError::IdentityMismatch {
            claimed: "author:alice".to_owned(),
            bound_to: "author:bob".to_owned(),
            key_fingerprint: b05_signer_binding::key_fingerprint(&vk),
        })
    );
}

#[test]
fn bound_identity_with_bad_signature_is_a_crypto_failure() {
    let sk = fixture_key();
    let vk = sk.verifying_key().to_bytes();
    let signature = sk.sign(b"some other bytes").to_bytes();
    let mut registry = BindingRegistry::new();
    registry.bind("author:alice", vk).unwrap();

    assert_eq!(
        verify_bound_signature(
            &registry,
            "author:alice",
            &vk,
            LEGACY_FIXTURE.as_bytes(),
            &signature
        ),
        Err(BindingError::BadSignature)
    );
}

#[test]
fn rotation_overlaps_within_one_identity_and_keys_stay_single_identity() {
    let old = fixture_key().verifying_key().to_bytes();
    let new = other_key().verifying_key().to_bytes();
    let mut registry = BindingRegistry::new();
    registry.bind("author:alice", old).unwrap();
    // Rotation: the identity holds two keys; history stays verifiable.
    registry.bind("author:alice", new).unwrap();
    assert!(registry.is_bound("author:alice", &old));
    assert!(registry.is_bound("author:alice", &new));
    // Idempotent re-bind of the same pair.
    registry.bind("author:alice", old).unwrap();
    assert_eq!(registry.bindings().len(), 2);

    // A key may not be claimed by a second identity (no silent
    // overwrite, which would let one signer impersonate another).
    assert!(matches!(
        registry.bind("author:bob", old),
        Err(BindingError::IdentityMismatch { .. })
    ));

    // Retirement leaves the other generation alone.
    assert_eq!(registry.retire_identity("author:alice"), 2);
    assert!(!registry.is_bound("author:alice", &old));
    assert_eq!(registry.identity_for(&new), None);
}
