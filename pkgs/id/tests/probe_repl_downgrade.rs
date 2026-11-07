//! Probe (lane B, 2026-11-02): does ReplicationTracker accept a Snapshot
//! whose header_version is LOWER than the chain's current version,
//! silently rolling the replica's header version back and re-admitting
//! tails on the old version that were refused moments before?
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ed25519_dalek::SigningKey;
use id::world::{
    JournalEntry, ReplicationEnvelope, ReplicationKind, ReplicationTracker, SignedReplicationEnvelope,
    chain_base_cursor, chain_payload, frame_digest, journal_entry_line,
};

const WORLD: &str = "world-replication-test";
const NONCE: [u8; 16] = [7; 16];

fn key() -> SigningKey {
    SigningKey::from_bytes(&[1; 32])
}

fn source_of(key: &SigningKey) -> [u8; 32] {
    *key.verifying_key().as_bytes()
}

fn entry_payload() -> Vec<u8> {
    let entries = [
        JournalEntry::Created {
            world_id: WORLD.to_string(),
            version: 1,
        },
        JournalEntry::CapsGranted {
            granted: vec!["chat".to_string()],
        },
        JournalEntry::CapsCeiling {
            ceiling: vec!["chat".to_string(), "input".to_string()],
        },
    ];
    entries.iter().flat_map(journal_entry_line).collect()
}

fn sign(env: ReplicationEnvelope, key: &SigningKey) -> Vec<u8> {
    SignedReplicationEnvelope::sign(env, key)
        .unwrap()
        .encode_frame()
        .unwrap()
}

#[test]
fn probe_snapshot_header_downgrade() {
    let key = key();
    let src = source_of(&key);
    let payload = entry_payload();
    let mut rx = ReplicationTracker::new(WORLD, src, 1, NONCE);

    // 1. Tail on v1 advances the chain.
    let base1 = chain_base_cursor(WORLD, &src, 1);
    let tail1 = sign(
        ReplicationEnvelope::tail(WORLD, src, 1, NONCE, base1, [0; 32], payload.clone()).unwrap(),
        &key,
    );
    rx.accept(&tail1).unwrap();
    println!("PROBE1 tail v1 accepted, ordinal={}", rx.cursor().ordinal);

    // 2. Snapshot on v2 (header bump) rebases; header moves to 2.
    let snap2 = sign(
        ReplicationEnvelope::snapshot(WORLD, src, 2, NONCE, frame_digest(&tail1), payload.clone())
            .unwrap(),
        &key,
    );
    let acc = rx.accept(&snap2).unwrap();
    assert_eq!(acc.kind, ReplicationKind::Snapshot);
    println!("PROBE2 snapshot v2 accepted, header={}", rx.header_version());

    // 3. Control: a tail on the OLD version is now refused.
    let stale_tail = sign(
        ReplicationEnvelope::tail(WORLD, src, 1, NONCE, rx.cursor(), frame_digest(&snap2), payload.clone())
            .unwrap(),
        &key,
    );
    let stale = rx.accept(&stale_tail);
    println!("PROBE3 tail v1 after bump: {stale:?}");
    assert!(stale.is_err(), "control: stale tail must be refused");

    // 4. The question: a snapshot on v1 again — a header DOWNGRADE.
    //    (Frame-chain digest names snap2, so the frame chain is intact;
    //    the refused stale tail did not advance it.)
    let snap1 = sign(
        ReplicationEnvelope::snapshot(WORLD, src, 1, NONCE, frame_digest(&snap2), payload.clone())
            .unwrap(),
        &key,
    );
    match rx.accept(&snap1) {
        Ok(acc) => {
            println!(
                "PROBE4 BUG: downgrade snapshot v1 ACCEPTED; header now {}, cursor ordinal {} (v1-base chain: {})",
                rx.header_version(),
                acc.cursor_after.ordinal,
                acc.cursor_after == chain_payload(base1, &payload).unwrap(),
            );
            // 5. After the downgrade, old-version tails are admitted again:
            //    the HeaderVersionChanged guard from PROBE3 is defeated.
            let tail1b = sign(
                ReplicationEnvelope::tail(
                    WORLD,
                    src,
                    1,
                    NONCE,
                    rx.cursor(),
                    frame_digest(&snap1),
                    payload.clone(),
                )
                .unwrap(),
                &key,
            );
            match rx.accept(&tail1b) {
                Ok(_) => println!("PROBE5 BUG: tail v1 accepted again after downgrade"),
                Err(e) => println!("PROBE5 tail v1 after downgrade refused: {e:?}"),
            }
        }
        Err(e) => println!("PROBE4 OK: downgrade snapshot refused: {e:?}"),
    }

    // 6. Control: a same-version snapshot (compaction rebase) is legitimate
    //    and must stay accepted in a fresh session.
    let mut rx2 = ReplicationTracker::new(WORLD, src, 1, NONCE);
    let t = sign(
        ReplicationEnvelope::tail(WORLD, src, 1, NONCE, base1, [0; 32], payload.clone()).unwrap(),
        &key,
    );
    rx2.accept(&t).unwrap();
    let snap_same = sign(
        ReplicationEnvelope::snapshot(WORLD, src, 1, NONCE, frame_digest(&t), payload).unwrap(),
        &key,
    );
    println!(
        "PROBE6 same-version snapshot rebase: {:?}",
        rx2.accept(&snap_same).map(|a| a.kind)
    );
}
