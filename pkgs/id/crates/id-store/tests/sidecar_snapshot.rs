//! Model test for complete signed metadata sidecar snapshots (slice
//! A9114-sidecar-snapshot, issue #10242; design A-9114.1–A-9114.3,
//! S3.11–S3.14, S3.18).
//!
//! Oracle: **model** — expected `Canonical signed-envelope digest and
//! a model of complete scope; snapshot digest is not recursively
//! embedded in signed Body.` The reference model holds, per scope,
//! the complete metadata view as a plain map, the head generation,
//! and the head snapshot digest computed with id-core's
//! `ContentId::of_bytes` over the envelope bytes — digest derivation
//! is id-core's, never the module's. Generated operations run against
//! the model and against the module, and every resulting state and
//! error class is compared: an accepted snapshot's decoded view must
//! equal the model's complete view exactly (completeness), its
//! parent must be the model's prior head digest, and the head must
//! advance by exactly one generation or not at all.
//!
//! Fixed tests cover the design's named rules: genesis shape, CAS
//! staleness, scope continuity, unknown critical fields rejecting,
//! absent parent omitted (never null), digest-derived file names,
//! and non-recursion of the snapshot digest.
//!
//! Why the module is included by path: slice 9110-02 owns the crate
//! roots, so `sidecar_snapshot` is not exported from id-store yet.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/sidecar_snapshot.rs"]
mod sidecar_snapshot;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;

use id_core::canonical_cbor::Value;
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use sidecar_snapshot::{
    HEAD_FILE_NAME, SidecarBodyV1, SidecarEnvelopeV1, SidecarHeadV1, snapshot_file_name,
};

const PROFILE: HashProfile = HashProfile::RawBlake3_256;
const NAMESPACE: &str = "ns/alpha";
const TENANT: &str = "tenant/a";
const DOMAIN: &str = "id.sidecar/v1";
const SIGNER: &str = "sidecar-writer";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn subject() -> ContentId {
    ContentId::of_bytes(PROFILE, b"subject-1").unwrap()
}

fn snapshot(
    view: &BTreeMap<String, Value>,
    generation: u64,
    parent: Option<sidecar_snapshot::SnapshotId>,
    created_at: u64,
) -> SidecarEnvelopeV1 {
    snapshot_scoped(view, generation, parent, created_at, TENANT)
}

fn snapshot_scoped(
    view: &BTreeMap<String, Value>,
    generation: u64,
    parent: Option<sidecar_snapshot::SnapshotId>,
    created_at: u64,
    tenant_scope: &str,
) -> SidecarEnvelopeV1 {
    let payload = sidecar_snapshot::encode_view(view).unwrap();
    let body = SidecarBodyV1::for_payload(
        subject(),
        NAMESPACE.to_owned(),
        tenant_scope.to_owned(),
        generation,
        parent,
        created_at,
        DOMAIN.to_owned(),
        SIGNER.to_owned(),
        key().verifying_key().to_bytes(),
        PROFILE,
        &payload,
    )
    .unwrap();
    SidecarEnvelopeV1::sign(body, payload, &key()).unwrap()
}

/// The model's independent digest of an envelope's canonical bytes.
fn model_digest(envelope: &SidecarEnvelopeV1) -> ContentId {
    ContentId::of_bytes(PROFILE, &envelope.canonical_bytes().unwrap()).unwrap()
}

/// Reference model of one scope: the complete view, the head
/// generation and head digest, and every accepted envelope.
#[derive(Default)]
struct Model {
    view: BTreeMap<String, Value>,
    generation: Option<u64>,
    head_digest: Option<ContentId>,
    accepted: Vec<SidecarEnvelopeV1>,
}

impl Model {
    /// Applies an accepted envelope to the model and returns the
    /// state the implementation must match.
    fn accept(&mut self, envelope: &SidecarEnvelopeV1) -> (u64, ContentId) {
        let digest = model_digest(envelope);
        let generation = envelope.body.generation;
        self.view = envelope.view().unwrap();
        self.generation = Some(generation);
        self.head_digest = Some(digest);
        self.accepted.push(envelope.clone());
        (generation, digest)
    }
}

struct Rng(u64);

impl Rng {
    const fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    const fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

#[test]
fn generated_operations_match_the_model() {
    let mut rng = Rng(0x9114_0242_5EED_0001);
    let mut model = Model::default();
    let mut head: Option<SidecarHeadV1> = None;
    let mut clock = 1_700_000_000_000u64;

    for step in 0..160u64 {
        clock += 1;
        let op = if model.accepted.is_empty() {
            0 // first operation must create the genesis snapshot
        } else {
            rng.below(10)
        };
        match op {
            // Put or remove a view entry, then snapshot the complete
            // resulting view and advance (or create) the head.
            0..=5 => {
                let key_name = format!("k{}", rng.below(6));
                if op <= 3 {
                    let value = match rng.below(3) {
                        0 => Value::Text(format!("v{step}")),
                        1 => Value::UInt(step),
                        _ => Value::Bool(step % 2 == 0),
                    };
                    model.view.insert(key_name, value);
                } else {
                    model.view.remove(&key_name);
                }
                let generation = model.generation.map_or(0, |g| g + 1);
                let parent = head.as_ref().map(|_| {
                    sidecar_snapshot::SnapshotId::from_content_id(model.head_digest.unwrap())
                });
                let envelope = snapshot(&model.view, generation, parent, clock);
                let new_head = match &head {
                    None => SidecarHeadV1::genesis(&envelope, &key()).unwrap(),
                    Some(h) => h
                        .advance(model.accepted.last().unwrap(), &envelope, &key())
                        .unwrap(),
                };
                let (want_generation, want_digest) = model.accept(&envelope);
                assert_eq!(new_head.generation, want_generation, "step {step}");
                assert_eq!(
                    *new_head.snapshot_id.content_id(),
                    want_digest,
                    "step {step}"
                );
                assert_eq!(
                    *envelope.snapshot_id().unwrap().content_id(),
                    want_digest,
                    "step {step}: snapshot id is the model digest of the envelope"
                );
                assert_eq!(
                    envelope.view().unwrap(),
                    model.view,
                    "step {step}: snapshot carries the complete view, not a delta"
                );
                // Round-trip: stored bytes decode to the same records.
                let decoded =
                    SidecarEnvelopeV1::decode(&envelope.canonical_bytes().unwrap()).unwrap();
                assert_eq!(decoded, envelope, "step {step}");
                decoded.verify(&ProfileDispatch::default()).unwrap();
                let decoded_head =
                    SidecarHeadV1::decode(&new_head.canonical_bytes().unwrap()).unwrap();
                assert_eq!(decoded_head, new_head, "step {step}");
                decoded_head.verify().unwrap();
                head = Some(new_head);
            }
            // Replay an old snapshot as the successor: the model says
            // its generation is not the head generation plus one.
            6 => {
                let pick = rng.below(u64::try_from(model.accepted.len()).unwrap());
                let old = &model.accepted[usize::try_from(pick).unwrap()];
                if old.body.generation == model.generation.unwrap() + 1 {
                    continue;
                }
                let error = head
                    .as_ref()
                    .unwrap()
                    .advance(model.accepted.last().unwrap(), old, &key())
                    .unwrap_err();
                assert_eq!(error.class(), "generation_mismatch", "step {step}");
            }
            // A successor in a different tenant scope: rejected, and
            // the head does not move.
            7 => {
                let generation = model.generation.unwrap() + 1;
                let parent =
                    sidecar_snapshot::SnapshotId::from_content_id(model.head_digest.unwrap());
                let foreign =
                    snapshot_scoped(&model.view, generation, Some(parent), clock, "tenant/other");
                let error = head
                    .as_ref()
                    .unwrap()
                    .advance(model.accepted.last().unwrap(), &foreign, &key())
                    .unwrap_err();
                assert_eq!(error.class(), "scope_mismatch", "step {step}");
            }
            // A successor with the right generation but no parent:
            // rejected as a parent mismatch.
            8 => {
                let generation = model.generation.unwrap() + 1;
                let orphan = snapshot(&model.view, generation, None, clock);
                let error = head
                    .as_ref()
                    .unwrap()
                    .advance(model.accepted.last().unwrap(), &orphan, &key())
                    .unwrap_err();
                assert_eq!(error.class(), "parent_mismatch", "step {step}");
            }
            // Tampering: a flipped payload byte fails the payload-ID
            // check; a flipped signature byte fails signature
            // verification and changes the snapshot digest while the
            // payload ID stands.
            _ => {
                let current = model.accepted.last().unwrap();
                let mut payload_tampered = current.clone();
                payload_tampered.raw_payload_bytes[0] ^= 1;
                let error = payload_tampered
                    .verify(&ProfileDispatch::default())
                    .unwrap_err();
                assert_eq!(error.class(), "digest_mismatch", "step {step}");

                let mut signature_tampered = current.clone();
                signature_tampered.signature[0] ^= 1;
                let error = signature_tampered
                    .verify(&ProfileDispatch::default())
                    .unwrap_err();
                assert_eq!(error.class(), "bad_signature", "step {step}");
                assert_ne!(
                    signature_tampered.snapshot_id().unwrap(),
                    current.snapshot_id().unwrap(),
                    "step {step}: the snapshot digest covers the signature"
                );
                assert_eq!(
                    signature_tampered.body.payload_id, current.body.payload_id,
                    "step {step}: the payload ID does not cover the signature"
                );
            }
        }
        // After every operation the head state equals the model.
        let h = head.as_ref().unwrap();
        assert_eq!(Some(h.generation), model.generation, "step {step}");
        assert_eq!(
            Some(*h.snapshot_id.content_id()),
            model.head_digest,
            "step {step}"
        );
    }
    assert!(
        model.accepted.len() > 40,
        "the run accepted a substantial chain: {}",
        model.accepted.len()
    );
}

#[test]
fn genesis_shape_and_stale_head() {
    let mut view = BTreeMap::new();
    view.insert("title".to_owned(), Value::Text("hello".to_owned()));

    // Genesis must be generation 0 with no parent.
    let gen3 = snapshot(&view, 3, None, 1);
    assert_eq!(
        SidecarHeadV1::genesis(&gen3, &key()).unwrap_err().class(),
        "generation_mismatch"
    );
    let parented = snapshot(
        &view,
        0,
        Some(sidecar_snapshot::SnapshotId::from_content_id(
            ContentId::of_bytes(PROFILE, b"elsewhere").unwrap(),
        )),
        1,
    );
    assert_eq!(
        SidecarHeadV1::genesis(&parented, &key())
            .unwrap_err()
            .class(),
        "parent_mismatch"
    );

    let genesis = snapshot(&view, 0, None, 1);
    let head = SidecarHeadV1::genesis(&genesis, &key()).unwrap();
    assert_eq!(head.generation, 0);

    // Advancing against a `current` the head does not point at is a
    // stale compare-and-swap, even if the successor is well formed.
    view.insert("extra".to_owned(), Value::UInt(1));
    let next = snapshot(&view, 1, Some(genesis.snapshot_id().unwrap()), 2);
    let stranger = snapshot(&BTreeMap::new(), 0, None, 1);
    assert_eq!(
        head.advance(&stranger, &next, &key()).unwrap_err().class(),
        "stale_head"
    );
    let head = head.advance(&genesis, &next, &key()).unwrap();
    assert_eq!(head.generation, 1);
    assert_eq!(head.snapshot_id, next.snapshot_id().unwrap());
}

#[test]
fn snapshot_digest_is_not_recursively_embedded() {
    let mut view = BTreeMap::new();
    view.insert("a".to_owned(), Value::UInt(1));
    let envelope = snapshot(&view, 0, None, 42);
    let digest = *envelope.snapshot_id().unwrap().content_id().digest();
    let body_bytes = envelope.body.canonical_bytes().unwrap();
    assert!(
        !body_bytes.windows(32).any(|window| window == digest),
        "the signed body must not contain its own snapshot digest"
    );
    // A body carrying an extra snapshot-id field is an unknown
    // critical field and rejects at decode.
    let mut value = envelope.body.to_value().unwrap();
    let Value::Map(entries) = &mut value else {
        panic!("body is a map");
    };
    entries.push((
        Value::Text("snapshot_id".to_owned()),
        Value::Bytes(digest.to_vec()),
    ));
    let bytes = id_core::canonical_cbor::encode_canonical(&value).unwrap();
    let decoded = id_core::canonical_cbor::decode_canonical(&bytes).unwrap();
    assert_eq!(
        SidecarBodyV1::from_value(&decoded).unwrap_err().class(),
        "malformed"
    );
}

#[test]
fn absent_parent_is_omitted_never_null() {
    let view = BTreeMap::new();
    let envelope = snapshot(&view, 0, None, 7);
    let Value::Map(entries) = envelope.body.to_value().unwrap() else {
        panic!("body is a map");
    };
    assert!(
        !entries
            .iter()
            .any(|(k, _)| *k == Value::Text("parent_snapshot_id".to_owned())),
        "genesis body omits parent_snapshot_id entirely"
    );
}

#[test]
fn storage_names_are_digest_derived() {
    assert_eq!(HEAD_FILE_NAME, "hash.metadata");
    let envelope = snapshot(&BTreeMap::new(), 0, None, 9);
    let id = envelope.snapshot_id().unwrap();
    let name = snapshot_file_name(&id);
    assert!(name.starts_with("hash.metadata."));
    assert!(name.ends_with(&hex::encode(id.content_id().digest())));
}

#[test]
fn tampered_head_does_not_verify() {
    let envelope = snapshot(&BTreeMap::new(), 0, None, 11);
    let mut head = SidecarHeadV1::genesis(&envelope, &key()).unwrap();
    head.verify().unwrap();
    head.generation = 9;
    assert_eq!(head.verify().unwrap_err().class(), "bad_signature");
}
