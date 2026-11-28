//! Cross-language fixtures for the hosted signed-object bridge
//! (#3818): the native side of the contract the TypeScript bridge in
//! pkgs/id/hosted-worker implements.
//!
//! - BLAKE3 of the official test-vector inputs (byte i = i mod 251)
//!   equals the values in the shared G1 fixture — the operation ID
//!   hash agrees across the languages.
//! - Events signed by the hosted-worker's TypeScript signer decode
//!   and ingest natively, under the IDs the TypeScript side computed
//!   — the wire format and the signature agree.
//! - Events signed here match the shared G1 fixture, which the
//!   TypeScript suite also decodes and verifies byte for byte.
//!
//! Oracle: p2panda-core's Hash (BLAKE3) and chain::decode /
//! EventStore::ingest_batch — the native implementation itself.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::ingest::MemStorage;
use id::chain::{ChainExt, sign};
use id::event_store::{EventStore, Ingest};
use id::session_checkpoint::{MacScope, SegmentCheckpoint, SessionCheckpoint, SessionHeader};
use id::waiting::WaitingLimits;
use p2panda_core::{Hash, SigningKey};

const NOW_MS: u64 = 0;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex fixture"))
        .collect()
}

fn shared_vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/hosted-second-version-g1.json"))
        .expect("shared G1 fixture is valid JSON")
}

/// Shared BLAKE3 outputs for official test-vector inputs. If these
/// diverge, the two sides compute different operation IDs for the
/// same event bytes.
#[test]
fn blake3_vectors_agree_with_the_hosted_bridge() {
    let vectors = shared_vectors();
    let cases = vectors["blake3"].as_array().expect("shared BLAKE3 vectors");
    assert_eq!(
        cases.len(),
        11,
        "shared BLAKE3 fixture retains every vector"
    );
    for case in cases {
        let len = case["input_len"].as_u64().expect("fixture input length") as usize;
        let expected = case["digest_hex"].as_str().expect("fixture digest");
        let input: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            hex(Hash::digest(&input).as_bytes()),
            expected,
            "length {len}"
        );
    }
}

#[test]
fn ts_signed_events_decode_and_ingest_natively() {
    let fixture = shared_vectors();
    let events = fixture["bridge_events"]["typescript_signed"]
        .as_array()
        .expect("shared TypeScript-signed event vectors");
    assert_eq!(
        events.len(),
        2,
        "shared bridge fixture retains both TypeScript-signed events"
    );
    let a_id = events[0]["id_hex"].as_str().expect("event A id");
    let b_id = events[1]["id_hex"].as_str().expect("event B id");
    let a = unhex(events[0]["bytes_hex"].as_str().expect("event A bytes"));
    let b = unhex(events[1]["bytes_hex"].as_str().expect("event B bytes"));
    let decoded_a = id::chain::decode(&a).expect("TS event A decodes natively");
    assert_eq!(hex(decoded_a.id.as_bytes()), a_id);
    let decoded_b = id::chain::decode(&b).expect("TS event B decodes natively");
    assert_eq!(hex(decoded_b.id.as_bytes()), b_id);

    let mut store = EventStore::open_with(&MemStorage::default(), WaitingLimits::default(), NOW_MS)
        .expect("open store");
    let outcomes = store.ingest_batch(&[a, b], NOW_MS).expect("ingest");
    assert_eq!(outcomes.len(), 2);
    assert!(
        matches!(&outcomes[0], Ingest::Accepted { id, .. } if hex(id.as_bytes()) == a_id),
        "A accepted: {:?}",
        outcomes[0]
    );
    assert!(
        matches!(&outcomes[1], Ingest::Accepted { id, .. } if hex(id.as_bytes()) == b_id),
        "B accepted: {:?}",
        outcomes[1]
    );
}

fn ext(kind: &str) -> ChainExt {
    ChainExt {
        kind: kind.to_owned(),
        group: None,
        after: vec![],
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
        thread: None,
    }
}

/// Rust-signed events for the TypeScript suite to pin: a plain
/// chain (R1, R2 with a body and an `after`) and a session
/// checkpoint (R3). Printed for pinning; the assertions here hold
/// the native side self-consistent.
#[test]
fn rust_signed_events_for_the_hosted_bridge() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let r1 = sign(&key, 0, None, None, ext("message")).expect("sign R1");
    let mut ext2 = ext("message");
    ext2.after = vec![r1.id];
    let r2 = sign(&key, 1, Some(r1.id), Some(b"rust body"), ext2).expect("sign R2");
    let checkpoint = SessionCheckpoint {
        header: SessionHeader {
            session_id: Hash::digest(b"session"),
            purpose: "collab editing".to_owned(),
            participants: vec![key.verifying_key()],
            started_ms: 1_000,
            nonce: Hash::digest(b"nonce"),
            mac_key_commitment: Hash::digest(b"mac key"),
            mac_scope: MacScope::PerParticipant,
        },
        segment: SegmentCheckpoint {
            segment_seq: 0,
            prev_checkpoint: None,
            root: Hash::digest(b"root"),
            event_count: 256,
            start_ms: 1_000,
            end_ms: 2_800,
            audit_blob: None,
        },
    };
    let mut ext3 = ext(id::session_checkpoint::CHECKPOINT_KIND);
    ext3.checkpoint = Some(checkpoint);
    let r3 = sign(&key, 2, Some(r2.id), None, ext3).expect("sign R3");
    let fixture = shared_vectors();
    let expected = fixture["bridge_events"]["rust_signed"]
        .as_array()
        .expect("shared Rust-signed event vectors");
    assert_eq!(
        expected.len(),
        3,
        "shared bridge fixture retains all Rust-signed events"
    );
    for (event, vector) in [&r1, &r2, &r3].into_iter().zip(expected) {
        assert_eq!(
            hex(&event.bytes),
            vector["bytes_hex"].as_str().expect("fixture event bytes")
        );
        assert_eq!(
            hex(event.id.as_bytes()),
            vector["id_hex"].as_str().expect("fixture event id")
        );
    }

    let mut store = EventStore::open_with(&MemStorage::default(), WaitingLimits::default(), NOW_MS)
        .expect("open store");
    let outcomes = store
        .ingest_batch(
            &[r1.bytes.clone(), r2.bytes.clone(), r3.bytes.clone()],
            NOW_MS,
        )
        .expect("ingest");
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Ingest::Accepted { .. })),
        "all three accepted: {outcomes:?}"
    );
}
