//! ID-sync consumer integration for the immutable legacy and versioned
//! signed-artifact profiles (C9135-02, #10211).
//!
//! Oracle: **golden** — ID-sync accepts the checked-in exact legacy
//! artifact bytes and a signed versioned envelope through its profile
//! discriminator, without changing the legacy preimage or signing the
//! new envelope ID into its own preimage. Canonical profile vectors are
//! owned by crosscut slice 9110-11; this test checks the ID-sync consumer
//! boundary rather than defining a second codec.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519, signature_preimage};
use id_sync::artifact::{
    LEGACY_PROFILE, RecordProfile, SignedArtifact, record_profile, signed_bytes,
};
use serde_json::Value;

const VECTORS: &str = include_str!("../../../test-vectors/legacy-artifact-signatures.json");

fn test_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn legacy_fixture() -> (SignedArtifact, String) {
    let vectors: Value = serde_json::from_str(VECTORS).unwrap();
    let vector = &vectors["legacy_profile"]["vectors"][0];
    let artifact: SignedArtifact = serde_json::from_value(vector.clone()).unwrap();
    let pinned_preimage = vector["signed_bytes_utf8"].as_str().unwrap().to_owned();
    (artifact, pinned_preimage)
}

fn versioned_envelope(key: &SigningKey, payload: &[u8]) -> Envelope {
    let body = Body {
        domain: "id/artifact".to_owned(),
        context: "namespace/main".to_owned(),
        subject: Some(ContentId::of_bytes(HashProfile::RawBlake3_256, b"subject").unwrap()),
        graph: None,
        predecessor: None,
        signer_id: "node-a".to_owned(),
        key: key.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_id: PayloadId::of(HashProfile::RawBlake3_256, payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    Envelope::sign(body, payload.to_vec(), key).unwrap()
}

#[test]
fn id_sync_reads_both_profiles_without_reinterpreting_legacy_bytes() {
    let (legacy, pinned_preimage) = legacy_fixture();
    assert_eq!(LEGACY_PROFILE, "id-artifact-serde-json/v1");

    let legacy_preimage = signed_bytes(
        &legacy.author,
        legacy.seq,
        &legacy.prev,
        &legacy.kind,
        &legacy.body,
        &legacy.after,
    )
    .unwrap();
    assert_eq!(legacy_preimage, pinned_preimage.as_bytes());
    legacy.verify().unwrap();

    let legacy_wire = serde_json::to_vec(&legacy).unwrap();
    assert_eq!(
        record_profile(&legacy_wire),
        Ok(RecordProfile::LegacyArtifactJson)
    );
    let decoded_legacy: SignedArtifact = serde_json::from_slice(&legacy_wire).unwrap();
    assert_eq!(decoded_legacy, legacy);
    decoded_legacy.verify().unwrap();

    let key = test_key();
    let envelope = versioned_envelope(&key, b"payload bytes");
    let versioned_wire = envelope.canonical_bytes().unwrap();
    assert_eq!(
        record_profile(&versioned_wire),
        Ok(RecordProfile::SignedEnvelopeCbor)
    );
    let decoded_envelope = Envelope::decode(&versioned_wire).unwrap();
    decoded_envelope
        .verify(&ProfileDispatch::default())
        .unwrap();

    // The envelope ID is computed from the complete signed envelope; it is
    // not inserted into Body or the signature preimage.
    let preimage = signature_preimage(&decoded_envelope.body).unwrap();
    let envelope_id = decoded_envelope.id(HashProfile::RawBlake3_256).unwrap();
    assert!(
        !preimage
            .windows(envelope_id.content_id().digest().len())
            .any(|window| window == envelope_id.content_id().digest())
    );
}
