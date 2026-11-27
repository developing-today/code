//! Model-oracle tests for signer-local signed coordinate-world definitions
//! (slice A9124-world-definition, issue #10278).
//!
//! Oracle: **model** — a signed definition is accepted only when its
//! canonical bytes, signer key and domain-separated signature agree; it must
//! name the array core, use a positive version, and require a predecessor for
//! later versions. Signature validity is deliberately separate from trust:
//! verifying a signature never grants global authority.
//!
//! The crate-root export belongs to the separate crate-wiring slice, so this
//! test includes the leaf modules by path, as the neighboring ID slices do.

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

#[path = "../src/world_definition.rs"]
mod world_definition;

#[path = "../src/test_forgeries.rs"]
mod test_forgeries;

use ed25519_dalek::SigningKey;
use id_shape::{ContentId, HashProfile};
use test_forgeries::{IDENTITY_KEY, identity_forgery};
use world_definition::{
    CoreKind, SignedWorldDefinition, WORLD_DEFINITION_DOMAIN, WorldDefinition, WorldDefinitionError,
};

const RAW: HashProfile = HashProfile::RawBlake3_256;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn content(seed: u8) -> ContentId {
    ContentId::of_bytes(RAW, &[seed; 32]).expect("fixed content is valid")
}

fn definition(seed: u8, version: u64, parent: Option<ContentId>) -> WorldDefinition {
    WorldDefinition {
        world_id: content(1),
        signer_id: format!("signer-{seed}"),
        signer_key_id: format!("key-{seed}"),
        core: CoreKind::Array,
        coordinate_profile: "array-v1".to_owned(),
        root_ref: content(2),
        presentation_profile: "raw".to_owned(),
        version,
        parent_definition: parent,
        issued_at: 1_700_000_000,
        valid_from: Some(1_700_000_000),
        valid_until: Some(1_800_000_000),
        key_history_ref: Some(content(3)),
    }
}

#[test]
fn canonical_definition_round_trips_and_verifies() {
    let signer = key(7);
    let signed = SignedWorldDefinition::sign(definition(7, 1, None), &signer)
        .expect("the model definition is signable");

    assert_eq!(signed.definition.core, CoreKind::Array);
    assert_eq!(signed.domain(), WORLD_DEFINITION_DOMAIN);
    signed
        .verify()
        .expect("the signer bound signature verifies");

    let bytes = signed.encode().expect("canonical encoding succeeds");
    let decoded = SignedWorldDefinition::decode(&bytes).expect("canonical bytes decode");
    assert_eq!(decoded, signed);
    decoded.verify().expect("the decoded signature verifies");
}

#[test]
fn signature_is_signer_local_not_global_trust() {
    let signer = key(9);
    let signed = SignedWorldDefinition::sign(definition(9, 1, None), &signer).unwrap();

    signed.verify().unwrap();
    assert!(!signed.trusts_globally());
    assert!(signed.trust_policy_result(false).is_err());
    assert!(signed.trust_policy_result(true).is_ok());
}

#[test]
fn tampering_domain_or_signer_key_rejects() {
    let signer = key(11);
    let signed = SignedWorldDefinition::sign(definition(11, 1, None), &signer).unwrap();

    let mut domain_tampered = signed.clone();
    domain_tampered.set_domain_for_test("other-domain");
    assert_eq!(
        domain_tampered.verify().unwrap_err().class(),
        "bad_signature"
    );

    let mut signer_tampered = signed;
    signer_tampered.definition.signer_id = "other-signer".to_owned();
    assert_eq!(
        signer_tampered.verify().unwrap_err().class(),
        "bad_signature"
    );
}

#[test]
fn identity_point_forgery_is_rejected() {
    let signer = key(12);
    let mut forged = SignedWorldDefinition::sign(definition(12, 1, None), &signer).unwrap();
    forged.signer_key = IDENTITY_KEY;
    forged.signature = identity_forgery();

    assert_eq!(forged.verify().unwrap_err().class(), "bad_signature");
}

#[test]
fn model_rejects_wrong_core_and_missing_predecessor() {
    let signer = key(13);
    let mut wrong_core = definition(13, 1, None);
    wrong_core.core = CoreKind::Content;
    assert_eq!(
        SignedWorldDefinition::sign(wrong_core, &signer)
            .unwrap_err()
            .class(),
        "unsupported_core"
    );

    let later = definition(13, 2, None);
    assert_eq!(
        SignedWorldDefinition::sign(later, &signer)
            .unwrap_err()
            .class(),
        "missing_parent"
    );
}

#[test]
fn malformed_canonical_record_is_rejected() {
    let signer = key(15);
    let signed = SignedWorldDefinition::sign(definition(15, 1, None), &signer).unwrap();
    let mut bytes = signed.encode().unwrap();
    bytes.push(0);
    assert!(matches!(
        SignedWorldDefinition::decode(&bytes),
        Err(WorldDefinitionError::Cbor(_))
    ));
}
