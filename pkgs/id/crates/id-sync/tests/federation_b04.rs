//! B04 signature suites — integration test (slice B04, #10207).
//!
//! Oracle (golden, from the issue): an allowed Ed25519
//! deterministic-CBOR profile verifies; an unknown suite, a
//! malformed encoding and a noncanonical encoding each fail closed.
//!
//! Independence: the envelope preimage here is built from the
//! documented rule (`domain prefix || 0x00 || canonical body`) and
//! signed with ed25519-dalek directly — never with a helper from the
//! module under test, which only parses and verifies. One canonical
//! body is pinned as literal hex, so a silent change to the profile
//! bytes fails this test rather than re-baselining itself. The
//! federation roots deliberately do not declare slice modules (slice
//! 9110-02), so this test includes the slice file by path; #9110-09
//! wires versioned public exports later.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b04_signature_suites.rs"]
mod b04_signature_suites;

use b04_signature_suites::{
    ED25519_SIGNATURE_LEN, SIGNATURE_DOMAIN_PREFIX, SUITE_ED25519_DET_CBOR_V1, SuiteError,
    SuiteRegistry, known_suite, signing_preimage,
};
use ed25519_dalek::{Signer, SigningKey};
use id_core::canonical_cbor::{Value, encode_canonical};

/// Test key: a fixed seed, so the goldens are stable across runs.
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// An independent preimage: the documented rule, not the module's
/// helper. Kept literal on purpose.
fn independent_preimage(body_bytes: &[u8]) -> Vec<u8> {
    let mut out = b"id/atlas/sig/v1".to_vec();
    out.push(0);
    out.extend_from_slice(body_bytes);
    out
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn bytes(value: &[u8]) -> Value {
    Value::Bytes(value.to_vec())
}

/// A complete B04 body (everything except `signature`).
fn body(suite: u64) -> Value {
    Value::Map(vec![
        (text("suite"), Value::UInt(suite)),
        (text("key_id"), text("key-1")),
        (text("signer_id"), text("signer-1")),
        (text("event_id"), bytes(&[1u8; 32])),
        (text("object_id"), bytes(&[2u8; 32])),
        (text("sequence"), Value::UInt(7)),
        (
            text("parent_ids"),
            Value::Array(vec![bytes(&[3u8; 32]), bytes(&[4u8; 32])]),
        ),
        (text("issued_at"), Value::UInt(1_700_000_000)),
        (text("payload_digest"), bytes(&[5u8; 32])),
        (text("payload_len"), Value::UInt(128)),
        (text("scope"), text("federation/peer")),
    ])
}

/// Encode `body`, sign the documented preimage with dalek, and return
/// the wire envelope: the body plus the detached `signature` field.
fn signed_envelope(body: &Value, key: &SigningKey) -> Vec<u8> {
    let body_bytes = encode_canonical(body).expect("body encodes");
    let signature = key.sign(&independent_preimage(&body_bytes)).to_bytes();
    let Value::Map(mut entries) = body.clone() else {
        panic!("body is a map");
    };
    entries.push((text("signature"), Value::Bytes(signature.to_vec())));
    encode_canonical(&Value::Map(entries)).expect("envelope encodes")
}

#[test]
fn allowed_suite_verifies() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let envelope = signed_envelope(&body(SUITE_ED25519_DET_CBOR_V1 as u64), &key);

    let verified = SuiteRegistry::v1()
        .verify(&envelope, &public_key)
        .expect("an allowed, well-formed envelope verifies");

    assert_eq!(verified.suite, SUITE_ED25519_DET_CBOR_V1);
    assert_eq!(verified.key_id, "key-1");
    assert_eq!(verified.signer_id.as_deref(), Some("signer-1"));
    assert_eq!(verified.event_id, vec![1u8; 32]);
    assert_eq!(verified.object_id, vec![2u8; 32]);
    assert_eq!(verified.sequence, Some(7));
    assert_eq!(verified.parent_ids, vec![vec![3u8; 32], vec![4u8; 32]]);
    assert_eq!(verified.issued_at, 1_700_000_000);
    assert_eq!(verified.payload_len, 128);
    assert_eq!(verified.scope, "federation/peer");
}

#[test]
fn canonical_body_bytes_are_pinned() {
    // Golden: the wire bytes of one body, pinned so a profile change
    // is a visible test failure instead of a silent re-baseline.
    let encoded = encode_canonical(&body(1)).expect("body encodes");
    let hex: String = encoded.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, GOLDEN_BODY_HEX);
}

/// Filled in from a first run, then re-checked by
/// `canonical_body_bytes_are_pinned` forever after.
const GOLDEN_BODY_HEX: &str = "ab6573636f70656f66656465726174696f6e2f7065657265737569746501666b65795f6964656b65792d31686576656e745f6964582001010101010101010101010101010101010101010101010101010101010101016873657175656e636507696973737565645f61741a6553f100696f626a6563745f696458200202020202020202020202020202020202020202020202020202020202020202697369676e65725f6964687369676e65722d316a706172656e745f6964738258200303030303030303030303030303030303030303030303030303030303030303582004040404040404040404040404040404040404040404040404040404040404046b7061796c6f61645f6c656e18806e7061796c6f61645f64696765737458200505050505050505050505050505050505050505050505050505050505050505";

#[test]
fn unknown_suite_fails_closed() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let envelope = signed_envelope(&body(99), &key);

    assert_eq!(
        SuiteRegistry::v1().verify(&envelope, &public_key),
        Err(SuiteError::UnknownSuite(99))
    );
}

#[test]
fn retired_suite_fails_closed_even_when_known() {
    // Deprecation is a policy move: the suite stays known (so a
    // capability report can name it) but the registry refuses it.
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let envelope = signed_envelope(&body(SUITE_ED25519_DET_CBOR_V1 as u64), &key);

    let retired = SuiteRegistry::with_allowlist(&[]);
    assert_eq!(
        retired.verify(&envelope, &public_key),
        Err(SuiteError::DisallowedSuite(SUITE_ED25519_DET_CBOR_V1))
    );
    assert!(known_suite(SUITE_ED25519_DET_CBOR_V1).is_some());
    assert_eq!(retired.allowed(), [] as [u16; 0]);
}

#[test]
fn noncanonical_encodings_fail_closed() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let envelope = signed_envelope(&body(1), &key);

    // Non-shortest length form for the enclosing map (0xb8 nn where
    // nn < 24 is exactly what the profile forbids): the body and its
    // signature are otherwise untouched, so only the encoding of the
    // length argument is wrong.
    assert_eq!(envelope[0] & 0xe0, 0xa0, "envelope is a canonical map");
    let entry_count = envelope[0] & 0x1f;
    assert!(entry_count < 24, "small envelope keeps an immediate header");
    let mut non_shortest = vec![0xb8, entry_count];
    non_shortest.extend_from_slice(&envelope[1..]);
    assert!(matches!(
        SuiteRegistry::v1().verify(&non_shortest, &public_key),
        Err(SuiteError::Cbor(_))
    ));

    // Trailing bytes after a complete item are not a canonical record.
    let mut trailing = envelope.clone();
    trailing.push(0x00);
    assert!(matches!(
        SuiteRegistry::v1().verify(&trailing, &public_key),
        Err(SuiteError::Cbor(_))
    ));

    // A truncated envelope is malformed, never a partial success.
    let truncated = &envelope[..envelope.len() - 1];
    assert!(matches!(
        SuiteRegistry::v1().verify(truncated, &public_key),
        Err(SuiteError::Cbor(_))
    ));
}

#[test]
fn wrong_types_never_coerce() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();

    // "1" as text is not suite 1.
    let Value::Map(mut entries) = body(1) else {
        panic!("body is a map");
    };
    for (key_value, value) in entries.iter_mut() {
        if *key_value == text("suite") {
            *value = text("1");
        }
    }
    let mut map = entries;
    let body_bytes = encode_canonical(&Value::Map(map.clone())).expect("body encodes");
    let signature = key.sign(&independent_preimage(&body_bytes)).to_bytes();
    map.push((text("signature"), Value::Bytes(signature.to_vec())));
    let envelope = encode_canonical(&Value::Map(map)).expect("envelope encodes");
    assert_eq!(
        SuiteRegistry::v1().verify(&envelope, &public_key),
        Err(SuiteError::WrongType("suite"))
    );

    // A non-map record is not an envelope.
    let not_a_map = encode_canonical(&Value::Array(vec![Value::UInt(1)])).expect("encodes");
    assert_eq!(
        SuiteRegistry::v1().verify(&not_a_map, &public_key),
        Err(SuiteError::WrongType("envelope"))
    );
}

#[test]
fn missing_required_field_rejects() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();

    let Value::Map(mut entries) = body(1) else {
        panic!("body is a map");
    };
    entries.retain(|(field, _)| *field != text("event_id"));
    let body_bytes = encode_canonical(&Value::Map(entries.clone())).expect("body encodes");
    let signature = key.sign(&independent_preimage(&body_bytes)).to_bytes();
    entries.push((text("signature"), Value::Bytes(signature.to_vec())));
    let envelope = encode_canonical(&Value::Map(entries)).expect("envelope encodes");

    assert_eq!(
        SuiteRegistry::v1().verify(&envelope, &public_key),
        Err(SuiteError::MissingField("event_id"))
    );
}

#[test]
fn absent_signature_rejects() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let envelope = encode_canonical(&body(1)).expect("body encodes");
    // Should reject for the missing signature rather than any crypto
    // reason; the field spec is checked first.
    assert_eq!(
        SuiteRegistry::v1().verify(&envelope, &public_key),
        Err(SuiteError::MissingSignature)
    );
}

#[test]
fn bad_signature_shape_rejects() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();

    let Value::Map(mut entries) = body(1) else {
        panic!("body is a map");
    };
    // One byte short of an Ed25519 signature.
    entries.push((
        text("signature"),
        Value::Bytes(vec![0u8; ED25519_SIGNATURE_LEN - 1]),
    ));
    let envelope = encode_canonical(&Value::Map(entries)).expect("envelope encodes");

    assert_eq!(
        SuiteRegistry::v1().verify(&envelope, &public_key),
        Err(SuiteError::BadSignatureLength(ED25519_SIGNATURE_LEN - 1))
    );
}

#[test]
fn invalid_and_misattributed_signatures_reject() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();

    // Signed over a different body: canonical bytes changed after
    // signing must not verify.
    let body_bytes = encode_canonical(&body(1)).expect("body encodes");
    let mut tampered_body = body(1);
    if let Value::Map(entries) = &mut tampered_body {
        for (field, value) in entries.iter_mut() {
            if *field == text("payload_len") {
                *value = Value::UInt(129);
            }
        }
    }

    let signature = key.sign(&independent_preimage(&body_bytes)).to_bytes();
    let Value::Map(mut entries) = tampered_body else {
        panic!("body is a map");
    };
    entries.push((text("signature"), Value::Bytes(signature.to_vec())));
    let envelope = encode_canonical(&Value::Map(entries)).expect("envelope encodes");
    assert_eq!(
        SuiteRegistry::v1().verify(&envelope, &public_key),
        Err(SuiteError::InvalidSignature)
    );

    // Misattribution: a valid signature from a different key.
    let other = SigningKey::from_bytes(&[9u8; 32]);
    let other_envelope = signed_envelope(&body(1), &other);
    assert_eq!(
        SuiteRegistry::v1().verify(&other_envelope, &public_key),
        Err(SuiteError::InvalidSignature)
    );
    // ... and it verifies under its own key, so the rejection above
    // is attribution and not a broken fixture.
    assert!(
        SuiteRegistry::v1()
            .verify(&other_envelope, &other.verifying_key().to_bytes())
            .is_ok()
    );
}

#[test]
fn domain_prefix_is_stable() {
    // The preimage rule is part of the wire contract: a change here
    // invalidates every signature already made.
    assert_eq!(SIGNATURE_DOMAIN_PREFIX, b"id/atlas/sig/v1");
    let preimage = signing_preimage(&[0x01, 0x02]);
    assert_eq!(
        &preimage[..SIGNATURE_DOMAIN_PREFIX.len()],
        SIGNATURE_DOMAIN_PREFIX
    );
    assert_eq!(preimage[SIGNATURE_DOMAIN_PREFIX.len()], 0u8);
    assert_eq!(
        &preimage[SIGNATURE_DOMAIN_PREFIX.len() + 1..],
        &[0x01, 0x02]
    );
}
