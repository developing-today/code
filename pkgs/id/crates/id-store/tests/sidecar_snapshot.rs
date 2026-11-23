//! Model-oracle tests for complete signed metadata sidecar snapshots
//! (slice A9114-sidecar-snapshot, issue #10242; parent #9114).
//!
//! The oracle is the canonical signed-envelope model: the body commits the
//! complete scope and version metadata, the signature covers only the body,
//! and the snapshot ID hashes the complete envelope after signing.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

#[path = "../src/sidecar_snapshot.rs"]
mod sidecar_snapshot;

use ed25519_dalek::SigningKey;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::signed_envelope::PayloadId;
use sidecar_snapshot::{
    SNAPSHOT_PROFILE, SUITE_ED25519, SidecarBody, SidecarSnapshot, SnapshotError,
};

const RAW: HashProfile = HashProfile::RawBlake3_256;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn body(
    signer: &SigningKey,
    payload: &[u8],
    generation: u64,
    parent_snapshot_id: Option<id_core::signed_envelope::EnvelopeId>,
    created_at: u64,
) -> SidecarBody {
    SidecarBody {
        subject_content_id: ContentId::of_bytes(RAW, b"subject").unwrap(),
        namespace_id: "atlas.metadata".to_owned(),
        tenant_scope: "tenant-a".to_owned(),
        generation,
        parent_snapshot_id,
        created_at,
        domain: "id/sidecar".to_owned(),
        context: "metadata".to_owned(),
        signer_id: "node-a".to_owned(),
        key_id: "key-a".to_owned(),
        key: signer.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_owned(),
        payload_profile: RAW,
        payload_id: PayloadId::of(RAW, payload).unwrap(),
        payload_length: payload.len() as u64,
    }
}

fn snapshot(
    seed: u8,
    payload: &[u8],
    generation: u64,
    parent_snapshot_id: Option<id_core::signed_envelope::EnvelopeId>,
    created_at: u64,
) -> SidecarSnapshot {
    let signer = key(seed);
    SidecarSnapshot::sign(
        body(&signer, payload, generation, parent_snapshot_id, created_at),
        payload.to_vec(),
        &signer,
    )
    .unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn complete_scope_round_trips_and_verifies() {
    let original = snapshot(1, br#"{"key":"value"}"#, 0, None, 1_700_000_000);
    let bytes = original.canonical_bytes().unwrap();
    let decoded = SidecarSnapshot::decode(&bytes).unwrap();

    assert_eq!(decoded, original);
    assert_eq!(decoded.canonical_bytes().unwrap(), bytes);
    decoded.verify().unwrap();
    assert_eq!(decoded.body.namespace_id, "atlas.metadata");
    assert_eq!(decoded.body.tenant_scope, "tenant-a");
}

#[test]
fn snapshot_id_is_the_complete_signed_envelope_digest() {
    let original = snapshot(2, b"metadata", 0, None, 1_700_000_000);
    let bytes = original.canonical_bytes().unwrap();
    let id = original.id().unwrap();

    assert_eq!(id.content_id().profile(), SNAPSHOT_PROFILE);
    assert_eq!(
        *id.content_id(),
        ContentId::of_bytes(SNAPSHOT_PROFILE, &bytes).unwrap()
    );
    assert!(!contains(&bytes, id.content_id().digest()));
}

#[test]
fn timestamp_is_advisory_metadata_not_a_filename_identity() {
    let first = snapshot(3, b"metadata", 0, None, 1_700_000_000);
    let second = snapshot(3, b"metadata", 0, None, 1_700_000_001);

    assert_ne!(first.id().unwrap(), second.id().unwrap());
    assert_eq!(first.body.payload_id, second.body.payload_id);
    assert_eq!(
        first.body.subject_content_id,
        second.body.subject_content_id
    );
}

#[test]
fn parent_reference_and_generation_are_signed() {
    let first = snapshot(4, b"generation 0", 0, None, 1_700_000_000);
    let first_id = first.id().unwrap();
    let second = snapshot(4, b"generation 1", 1, Some(first_id), 1_700_000_001);

    second.verify().unwrap();
    assert_eq!(second.body.generation, 1);
    assert_eq!(second.body.parent_snapshot_id, Some(first_id));
    assert_ne!(first.id().unwrap(), second.id().unwrap());
}

#[test]
fn generation_without_parent_is_rejected() {
    let signer = key(5);
    let error = SidecarSnapshot::sign(
        body(&signer, b"invalid", 1, None, 1_700_000_000),
        b"invalid".to_vec(),
        &signer,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        SnapshotError::Field {
            field: "parent_snapshot_id",
            ..
        }
    ));
}
