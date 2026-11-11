//! Probe: SignedReplicationEnvelope::sign must refuse dead-on-arrival envelopes.
//!
//! The 2026-11-04 probe (archive PR #7533) showed sign() signing envelopes
//! every receiver rejects: the receiver's validate path refused them with
//! specific errors, but the signer happily produced the frames. #6469 moved
//! the envelope-local rules into the signer via validate_envelope_local, so
//! sign() now returns None for each malformed case below. This test pins
//! that property: every case must be refused at signing time, and the valid
//! control envelope must still sign and be accepted.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ed25519_dalek::SigningKey;

use id::world::{
    JournalEntry, MAX_JOURNAL_VERSION, ReplicationCursor, ReplicationEnvelope, ReplicationKind,
    ReplicationTracker, SESSION_NONCE_BYTES, SignedReplicationEnvelope, chain_base_cursor,
    chain_payload, journal_entry_line, payload_digest,
};

const WORLD: &str = "probe-world";
const NONCE: [u8; SESSION_NONCE_BYTES] = [7; SESSION_NONCE_BYTES];

fn key() -> SigningKey {
    SigningKey::from_bytes(&[1; 32])
}

fn source(key: &SigningKey) -> [u8; 32] {
    *key.verifying_key().as_bytes()
}

fn payload_for(world: &str, version: u32) -> Vec<u8> {
    let entries = [
        JournalEntry::Created {
            world_id: world.to_string(),
            version,
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

fn valid_tail(key: &SigningKey) -> ReplicationEnvelope {
    ReplicationEnvelope::tail(
        WORLD,
        source(key),
        1,
        NONCE,
        chain_base_cursor(WORLD, &source(key), 1),
        [0; 32],
        payload_for(WORLD, 1),
    )
    .unwrap()
}

/// The signer must refuse the envelope outright: before #6469 each case
/// below signed fine and was then refused by every receiver with the
/// ReplicationError named in the case comment.
fn assert_signer_refuses(key: &SigningKey, env: ReplicationEnvelope, case: &str) {
    assert!(
        SignedReplicationEnvelope::sign(env, key).is_none(),
        "sign() must refuse: {case}"
    );
}

#[test]
fn probe_signer_refuses_dead_on_arrival_envelopes() {
    let key = key();

    // Control: a valid tail signs and is accepted.
    {
        let signed = SignedReplicationEnvelope::sign(valid_tail(&key), &key).unwrap();
        let bytes = signed.encode_frame().unwrap();
        let mut tracker = ReplicationTracker::new(WORLD, source(&key), 1, NONCE).unwrap();
        assert!(tracker.accept(&bytes).is_ok(), "control must accept");
    }

    // 1. Envelope version 99. Receiver verdict was UnsupportedEnvelopeVersion(99).
    let mut env = valid_tail(&key);
    env.version = 99;
    assert_signer_refuses(&key, env, "case 1 version");

    // 2a. Journal header version 0. Receiver verdict was UnsupportedJournalVersion(0).
    let mut env = valid_tail(&key);
    env.header_version = 0;
    assert_signer_refuses(&key, env, "case 2a header 0");

    // 2b. Journal header version above max. Receiver verdict was
    // UnsupportedJournalVersion(MAX_JOURNAL_VERSION + 1).
    let mut env = valid_tail(&key);
    env.header_version = MAX_JOURNAL_VERSION + 1;
    assert_signer_refuses(&key, env, "case 2b header too new");

    // 3. Payload digest lies. Receiver verdict was PayloadDigestMismatch.
    let mut env = valid_tail(&key);
    env.payload_digest = payload_digest(b"not the payload");
    assert_signer_refuses(&key, env, "case 3 digest");

    // 4. Cursor after does not chain from payload. Receiver verdict was
    // EntryChainMismatch.
    let mut env = valid_tail(&key);
    env.cursor_after.ordinal += 1;
    assert_signer_refuses(&key, env, "case 4 cursor");

    // 5. Malformed payload (not journal entries), digest corrected.
    // Receiver verdict was MalformedPayload.
    let mut env = valid_tail(&key);
    env.payload = b"not json\n".to_vec();
    env.payload_digest = payload_digest(&env.payload);
    assert_signer_refuses(&key, env, "case 5 malformed");

    // 6. Terminal carrying a payload. Receiver verdict was
    // TerminalCursorMismatch.
    let base = chain_base_cursor(WORLD, &source(&key), 1);
    let mut env = ReplicationEnvelope::terminal(WORLD, source(&key), 1, NONCE, base, [0; 32]);
    env.payload = payload_for(WORLD, 1);
    env.payload_digest = payload_digest(&env.payload);
    assert_eq!(env.kind, ReplicationKind::Terminal);
    assert_signer_refuses(&key, env, "case 6 terminal payload");

    // 7. Snapshot whose before-cursor is not the chain base. Receiver
    // verdict was CursorFork.
    let wrong_before = ReplicationCursor {
        ordinal: 5,
        digest: [9; 32],
    };
    let payload = payload_for(WORLD, 1);
    let env = ReplicationEnvelope {
        version: 1,
        world_id: WORLD.to_string(),
        source: source(&key),
        header_version: 1,
        kind: ReplicationKind::Snapshot,
        session_nonce: NONCE,
        cursor_before: wrong_before,
        cursor_after: chain_payload(wrong_before, &payload).unwrap(),
        prev_frame_digest: [0; 32],
        payload_digest: payload_digest(&payload),
        payload,
    };
    assert_signer_refuses(&key, env, "case 7 snapshot base");

    // 8. Snapshot payload journal header names another world. Receiver
    // verdict was InvalidJournalHeader.
    let env = ReplicationEnvelope::snapshot(
        WORLD,
        source(&key),
        1,
        NONCE,
        [0; 32],
        payload_for("other-world", 1),
    )
    .unwrap();
    assert_signer_refuses(&key, env, "case 8 journal header");
}
