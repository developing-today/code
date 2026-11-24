//! Cross-language fixtures for the hosted signed-object bridge
//! (#3818): the native side of the contract the TypeScript bridge in
//! pkgs/id/hosted-worker implements.
//!
//! - BLAKE3 of the official test-vector inputs (byte i = i mod 251)
//!   equals the values the TypeScript implementation produces —
//!   the operation ID hash agrees across the languages.
//! - Events signed by the hosted-worker's TypeScript signer decode
//!   and ingest natively, under the IDs the TypeScript side computed
//!   — the wire format and the signature agree.
//! - Events signed here are printed (run with --nocapture) and
//!   pinned in the TypeScript suite, which decodes and verifies
//!   them byte for byte.
//!
//! Oracle: p2panda-core's Hash (BLAKE3) and chain::decode /
//! EventStore::ingest_batch — the native implementation itself.

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

/// The hosted bridge's BLAKE3 outputs for the official test-vector
/// inputs, pinned from src/blake3.ts (whose empty-input value is the
/// published af1349b9… vector). If these diverge, the two sides
/// compute different operation IDs for the same event bytes.
#[test]
fn blake3_vectors_agree_with_the_hosted_bridge() {
    let cases: [(usize, &str); 11] = [
        (
            0,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
        (
            1,
            "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
        ),
        (
            3,
            "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f",
        ),
        (
            64,
            "4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98",
        ),
        (
            65,
            "de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee",
        ),
        (
            100,
            "8e2eb1bba3040b8f611a1240a0e111c74b45cfc9caed10b95f6372db1c40b8b5",
        ),
        (
            1023,
            "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
        ),
        (
            1024,
            "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
        ),
        (
            1025,
            "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
        ),
        (
            2048,
            "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
        ),
        (
            102400,
            "bc3e3d41a1146b069abffad3c0d44860cf664390afce4d9661f7902e7943e085",
        ),
    ];
    for (len, expected) in cases {
        let input: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            hex(Hash::digest(&input).as_bytes()),
            expected,
            "length {len}"
        );
    }
}

/// Events signed by the TypeScript bridge signer (seed [9;32], the
/// hosted-worker test key), pinned from test/bridge-event.test.ts's
/// generator. Native decode must verify them, and native ingest
/// must accept them under the IDs the TypeScript side computed.
const TS_EVENT_A: &str = "87015820fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f6185840221ed10f17b524e2dd2a913544ad35aae80406883ca69d694ae2ad90f9093493903fc3f881d964bed54ab8a75aa9d0f45d2f315a4312a5b0fc51f7418dc3930d0d5820af28878b29d0dda236c146c93cd4e0c1375143d70279cfa2bec5e9a00290757d00a4646b696e64676d657373616765656166746572806567726f7570f666646f6d61696e6c69642e6576656e74732e7631";
const TS_EVENT_A_ID: &str = "eb580de2dd2cd00b0c8f08653ddaacd6fed967e5a6cdcace542c1b23d50f835c";
const TS_EVENT_B: &str = "87015820fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f61858408ee8069a39ea3f864bb4f716874323044d90ad34d1cf9fe8528d0b3a8cc739b89f6718c18f17ad934208ad85534934c35ea91bb4169d29d9ebac674f1a658c0200015820eb580de2dd2cd00b0c8f08653ddaacd6fed967e5a6cdcace542c1b23d50f835ca4646b696e64676d657373616765656166746572815820eb580de2dd2cd00b0c8f08653ddaacd6fed967e5a6cdcace542c1b23d50f835c6567726f7570f666646f6d61696e6c69642e6576656e74732e7631";
const TS_EVENT_B_ID: &str = "b2eb1da1e7853f66167b2a4c297a65489b44c50cc56462455722745db5b02a18";

#[test]
fn ts_signed_events_decode_and_ingest_natively() {
    let a = unhex(TS_EVENT_A);
    let b = unhex(TS_EVENT_B);
    let decoded_a = id::chain::decode(&a).expect("TS event A decodes natively");
    assert_eq!(hex(decoded_a.id.as_bytes()), TS_EVENT_A_ID);
    let decoded_b = id::chain::decode(&b).expect("TS event B decodes natively");
    assert_eq!(hex(decoded_b.id.as_bytes()), TS_EVENT_B_ID);

    let mut store = EventStore::open_with(&MemStorage::default(), WaitingLimits::default(), NOW_MS)
        .expect("open store");
    let outcomes = store.ingest_batch(&[a, b], NOW_MS).expect("ingest");
    assert_eq!(outcomes.len(), 2);
    assert!(
        matches!(&outcomes[0], Ingest::Accepted { id, .. } if hex(id.as_bytes()) == TS_EVENT_A_ID),
        "A accepted: {:?}",
        outcomes[0]
    );
    assert!(
        matches!(&outcomes[1], Ingest::Accepted { id, .. } if hex(id.as_bytes()) == TS_EVENT_B_ID),
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
    println!("R1 id={} bytes={}", hex(r1.id.as_bytes()), hex(&r1.bytes));
    println!("R2 id={} bytes={}", hex(r2.id.as_bytes()), hex(&r2.bytes));
    println!("R3 id={} bytes={}", hex(r3.id.as_bytes()), hex(&r3.bytes));

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
