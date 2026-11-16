//! Throwaway PoC for #3795: protocol v1 phase 2 — intermittent-signature
//! live stream for collaborative editing (#2036), per the merged design
//! record `doc/2026-11-01_design_id2036.md` §5 rule 5 and the phase table
//! in §6 (phase 2 row).
//!
//! Program rule: this is a proof of concept, never built into the
//! permanent feature before review. It lives in an integration test so it
//! cannot touch the production collab path (`crates/id-web/src/collab.rs`
//! is unchanged): collab steps/cursors run through the phase-1 contract
//! from #3794 (`id::session_checkpoint`: a signed-once [`SessionHeader`]
//! plus [`SegmentCheckpoint`]s) as MAC-authenticated ephemeral frames.
//!
//! What the tests below prove, mapped to the phase-2 merge acceptance:
//!
//! - End-to-end with two direct peers and one bridge
//!   (`test_two_direct_peers_and_one_bridge_end_to_end`): the bridge is a
//!   member peer that verifies a frame before forwarding it.
//! - Bad MAC, replayed sequence, and wrong-session frames are rejected
//!   before application (`test_bad_mac_replay_and_wrong_session_are_rejected`).
//! - Checkpoints close at the configured limits (256 frames / 2 s by
//!   default, from the phase-1 [`CheckpointPolicy`]) and at explicit
//!   commit, and every committed checkpoint validates against the
//!   phase-1 rules (`test_checkpoint_at_limits_and_explicit_commit`,
//!   `test_default_policy_checkpoints_at_256_frames`).
//! - Raw frames never enter the durable graph: the graph holds only
//!   `session.checkpoint` payloads (roots, counts, time bounds, and the
//!   audit-blob hash of the segment snapshot), never frame payloads
//!   (`test_raw_frames_never_enter_the_event_store`).
//! - A late joiner reconstructs from the latest checkpoint plus the
//!   ephemeral stream since it, and the same path carries reconnect and
//!   partition-heal; loss of the uncheckpointed tail is explicit, never
//!   silent (`test_join_reconnect_and_partition_heal`,
//!   `test_uncheckpointed_tail_loss_is_explicit`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use id::session_checkpoint::{
    CHECKPOINT_KIND, CheckpointPolicy, MacScope, SegmentCheckpoint, SessionCheckpoint,
    SessionHeader, check_session_checkpoint,
};
use p2panda_core::{Hash, SigningKey, VerifyingKey};

// ---------------------------------------------------------------------------
// Collab operations: the steps and cursors of the live document.
// ---------------------------------------------------------------------------

/// One live collab operation, in the shape of a `web/collab.rs` step or
/// cursor move. Positions are character indices so test documents never
/// split a UTF-8 boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    /// Insert `text` at character position `pos`.
    Insert { pos: usize, text: String },
    /// Delete `len` characters starting at character position `pos`.
    Delete { pos: usize, len: usize },
    /// Move the sender's cursor to character position `pos`.
    Cursor { pos: usize },
}

impl Op {
    /// The canonical bytes the frame MAC and the segment root commit to.
    fn canonical_bytes(&self) -> Vec<u8> {
        match self {
            Op::Insert { pos, text } => format!("insert:{pos}:{text}").into_bytes(),
            Op::Delete { pos, len } => format!("delete:{pos}:{len}").into_bytes(),
            Op::Cursor { pos } => format!("cursor:{pos}").into_bytes(),
        }
    }
}

/// The document state a peer reconstructs: the text plus one cursor per
/// participant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Doc {
    text: Vec<char>,
    cursors: HashMap<usize, usize>,
}

impl Doc {
    fn apply(&mut self, sender: usize, op: &Op) {
        match op {
            Op::Insert { pos, text } => {
                let at = (*pos).min(self.text.len());
                for (offset, ch) in text.chars().enumerate() {
                    self.text.insert(at + offset, ch);
                }
            }
            Op::Delete { pos, len } => {
                let start = (*pos).min(self.text.len());
                let end = (start + len).min(self.text.len());
                self.text.drain(start..end);
            }
            Op::Cursor { pos } => {
                self.cursors.insert(sender, *pos);
            }
        }
    }

    fn text(&self) -> String {
        self.text.iter().collect()
    }
}

// ---------------------------------------------------------------------------
// Frames: ephemeral, MAC-authenticated, session-bound, ordered.
// ---------------------------------------------------------------------------

/// One ephemeral live frame. Frames are never written to the durable
/// graph; only the checkpoints that close over them are.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Frame {
    session_id: Hash,
    /// The frame's position in the session's one ordered stream.
    seq: u64,
    /// Index into the header's participant set.
    sender: usize,
    sent_ms: u64,
    op: Op,
    mac: [u8; 32],
}

/// Why a receiver (or the bridge) refused a frame. Every refusal happens
/// before the frame is applied or forwarded.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Reject {
    /// The frame names a different session than the receiver's header.
    WrongSession,
    /// The sender is not in the header's participant set.
    NotParticipant,
    /// The MAC does not verify under the sender's derived key.
    BadMac,
    /// The sequence was already applied: a duplicate or replay.
    Replay { seq: u64 },
    /// The sequence skips ahead of the receiver: frames are missing and
    /// must be recovered from the ephemeral stream, never skipped over.
    Gap { expected: u64, got: u64 },
}

/// The per-participant MAC key: derived from the session key material, so
/// under [`MacScope::PerParticipant`] a valid MAC attributes the frame to
/// its sender. The header commits to the session key, never the key
/// itself: `mac_key_commitment == Hash::digest(session_key)`.
fn participant_key(session_key: &[u8; 32], sender: usize) -> [u8; 32] {
    let mut material = session_key.to_vec();
    material.extend_from_slice(&(sender as u64).to_be_bytes());
    blake3::derive_key("id #2036 poc participant mac v1", &material)
}

/// The bytes a frame's MAC covers: everything that binds the frame to
/// its session, position, sender, time, and payload.
fn mac_input(header: &SessionHeader, seq: u64, sender: usize, sent_ms: u64, op: &Op) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(header.session_id.as_bytes());
    bytes.extend_from_slice(&seq.to_be_bytes());
    bytes.extend_from_slice(&(sender as u64).to_be_bytes());
    bytes.extend_from_slice(&sent_ms.to_be_bytes());
    bytes.extend_from_slice(&op.canonical_bytes());
    bytes
}

fn frame_mac(
    session_key: &[u8; 32],
    header: &SessionHeader,
    seq: u64,
    sender: usize,
    sent_ms: u64,
    op: &Op,
) -> [u8; 32] {
    let key = participant_key(session_key, sender);
    *blake3::keyed_hash(&key, &mac_input(header, seq, sender, sent_ms, op)).as_bytes()
}

fn build_frame(
    session_key: &[u8; 32],
    header: &SessionHeader,
    seq: u64,
    sender: usize,
    sent_ms: u64,
    op: Op,
) -> Frame {
    let mac = frame_mac(session_key, header, seq, sender, sent_ms, &op);
    Frame {
        session_id: header.session_id,
        seq,
        sender,
        sent_ms,
        op,
        mac,
    }
}

/// Verify a frame exactly as a receiver or a forwarding bridge does:
/// session binding first, then membership, then the MAC, then the
/// ordered-sequence checks. Nothing is applied on any error path.
fn verify(
    session_key: &[u8; 32],
    header: &SessionHeader,
    frame: &Frame,
    next_seq: u64,
) -> Result<(), Reject> {
    if frame.session_id != header.session_id {
        return Err(Reject::WrongSession);
    }
    if frame.sender >= header.participants.len() {
        return Err(Reject::NotParticipant);
    }
    let expected = frame_mac(
        session_key,
        header,
        frame.seq,
        frame.sender,
        frame.sent_ms,
        &frame.op,
    );
    if expected != frame.mac {
        return Err(Reject::BadMac);
    }
    if frame.seq < next_seq {
        return Err(Reject::Replay { seq: frame.seq });
    }
    if frame.seq > next_seq {
        return Err(Reject::Gap {
            expected: next_seq,
            got: frame.seq,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The live session: ordered transport, segmenting, checkpoint commits.
// ---------------------------------------------------------------------------

/// A checkpoint as committed to the durable graph, plus the PoC-local
/// bookkeeping a receiver needs to reconstruct from it.
#[derive(Clone, Debug)]
struct CommittedCheckpoint {
    /// The checkpoint event's ID: a hash of its canonical encoding.
    id: Hash,
    checkpoint: SessionCheckpoint,
    /// The first sequence NOT covered by this checkpoint: reconstruction
    /// resumes from here.
    covered_until_seq: u64,
}

/// The range of frame sequences that were lost with the uncheckpointed
/// tail, reported explicitly by reconstruction and resync — never
/// silently skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TailLoss {
    from_seq: u64,
    until_seq: u64,
}

/// One live session, owned by participant 0 (the session owner signs the
/// checkpoints, as the phase-1 contract has it). The session keeps three
/// separate stores, and the tests lean on their separation:
///
/// - `graph`: the durable event store. Only `session.checkpoint`
///   payloads ever enter it.
/// - `blobs`: retained audit material, named by hash from a checkpoint
///   (here: the segment's endpoint document snapshot). Blobs may expire
///   under retention without altering the signed checkpoint.
/// - `ephemeral`: the live frames since the last checkpoint. This is
///   the only place raw frames live, and it is never the graph.
struct LiveSession {
    header: SessionHeader,
    policy: CheckpointPolicy,
    session_key: [u8; 32],
    next_seq: u64,
    segment: Vec<Frame>,
    graph: Vec<CommittedCheckpoint>,
    blobs: HashMap<Hash, Vec<u8>>,
    ephemeral: Vec<Frame>,
    /// The owner's document: the owner applies every frame in order, so
    /// its state at a segment close is the checkpoint's endpoint state.
    owner_doc: Doc,
}

impl LiveSession {
    fn new(
        session_key: [u8; 32],
        participants: &[VerifyingKey],
        started_ms: u64,
        policy: CheckpointPolicy,
    ) -> Self {
        let header = SessionHeader {
            session_id: Hash::digest(b"#3795 poc session"),
            purpose: "collab editing poc".to_owned(),
            participants: participants.to_vec(),
            started_ms,
            nonce: Hash::digest(b"#3795 poc nonce"),
            mac_key_commitment: Hash::digest(&session_key),
            mac_scope: MacScope::PerParticipant,
        };
        Self {
            header,
            policy,
            session_key,
            next_seq: 0,
            segment: Vec::new(),
            graph: Vec::new(),
            blobs: HashMap::new(),
            ephemeral: Vec::new(),
            owner_doc: Doc::default(),
        }
    }

    /// Publish one frame onto the ordered stream: the time-based
    /// checkpoint is closed first when the segment has aged out, the
    /// frame is MACed and applied to the owner's document, and reaching
    /// the segment's frame limit closes the segment immediately.
    fn publish(&mut self, sender: usize, op: Op, sent_ms: u64) -> Frame {
        if let Some(first) = self.segment.first() {
            if sent_ms.saturating_sub(first.sent_ms) >= self.policy.interval_ms {
                self.close_segment();
            }
        }
        let frame = build_frame(
            &self.session_key,
            &self.header,
            self.next_seq,
            sender,
            sent_ms,
            op,
        );
        self.next_seq += 1;
        self.owner_doc.apply(frame.sender, &frame.op);
        self.segment.push(frame.clone());
        self.ephemeral.push(frame.clone());
        if self.segment.len() as u64 >= self.policy.event_limit {
            self.close_segment();
        }
        frame
    }

    /// Explicit commit (the design's commit/blur/end trigger): close the
    /// open segment, if any, and commit its checkpoint to the graph.
    fn commit(&mut self) {
        if !self.segment.is_empty() {
            self.close_segment();
        }
    }

    /// Close the open segment: fold its frames, in order, into a root,
    /// snapshot the owner's endpoint state as the audit blob, validate
    /// the checkpoint against the phase-1 rules, and only then commit
    /// it to the durable graph. The frames themselves stay ephemeral —
    /// once the segment closes, the stream since the checkpoint starts
    /// empty again.
    fn close_segment(&mut self) {
        let segment = std::mem::take(&mut self.segment);
        debug_assert!(!segment.is_empty());
        let mut root = *blake3::hash(b"#2036 segment root v1").as_bytes();
        for frame in &segment {
            let digest = blake3::hash(&mac_input(
                &self.header,
                frame.seq,
                frame.sender,
                frame.sent_ms,
                &frame.op,
            ));
            let mut folded = root.to_vec();
            folded.extend_from_slice(digest.as_bytes());
            root = *blake3::hash(&folded).as_bytes();
        }
        let snapshot = self.owner_doc.text().into_bytes();
        let audit_blob = Hash::digest(&snapshot);
        self.blobs.insert(audit_blob, snapshot);
        let segment_seq = self.graph.len() as u64;
        let prev_checkpoint = self.graph.last().map(|committed| committed.id);
        let checkpoint = SessionCheckpoint {
            header: self.header.clone(),
            segment: SegmentCheckpoint {
                segment_seq,
                prev_checkpoint,
                root: Hash::digest(&root),
                event_count: segment.len() as u64,
                start_ms: segment.first().unwrap().sent_ms,
                end_ms: segment.last().unwrap().sent_ms,
                audit_blob: Some(audit_blob),
            },
        };
        check_session_checkpoint(CHECKPOINT_KIND, &checkpoint).unwrap();
        let id = Hash::digest(&serde_json::to_vec(&checkpoint).unwrap());
        let covered_until_seq = segment.last().unwrap().seq + 1;
        self.graph.push(CommittedCheckpoint {
            id,
            checkpoint,
            covered_until_seq,
        });
        self.ephemeral.clear();
    }

    /// The latest committed checkpoint, if the session has closed one.
    fn latest_checkpoint(&self) -> Option<&CommittedCheckpoint> {
        self.graph.last()
    }

    /// The endpoint document snapshot the latest checkpoint's audit blob
    /// names, verified against the blob hash the checkpoint carries.
    fn latest_snapshot(&self) -> Option<String> {
        let committed = self.latest_checkpoint()?;
        let blob_hash = committed.checkpoint.segment.audit_blob?;
        let blob = self.blobs.get(&blob_hash)?;
        assert_eq!(Hash::digest(blob), blob_hash);
        Some(String::from_utf8(blob.clone()).unwrap())
    }

    /// The ephemeral frames since the latest checkpoint: what a late
    /// joiner or a reconnecting peer is offered. Anything older belongs
    /// to a closed segment and is NOT a backlog — a peer that missed it
    /// reconstructs from the checkpoint instead (design §5 rule 5:
    /// disconnected peers are not sent a backlog).
    fn tail_since_checkpoint(&self) -> &[Frame] {
        &self.ephemeral
    }
}

// ---------------------------------------------------------------------------
// Peers: receivers, the bridge, reconstruction.
// ---------------------------------------------------------------------------

/// A session participant's receiving side: its document and the next
/// sequence it expects.
#[derive(Debug)]
struct Peer {
    doc: Doc,
    next_seq: u64,
}

impl Peer {
    fn new() -> Self {
        Self {
            doc: Doc::default(),
            next_seq: 0,
        }
    }

    /// Receive one frame over a direct path: verify, then apply.
    fn receive(&mut self, session: &LiveSession, frame: &Frame) -> Result<(), Reject> {
        verify(&session.session_key, &session.header, frame, self.next_seq)?;
        self.doc.apply(frame.sender, &frame.op);
        self.next_seq += 1;
        Ok(())
    }

    /// Catch up with the session, on join, reconnect, or partition heal.
    /// State at or before the latest checkpoint is taken from the
    /// checkpoint's snapshot (never from a frame backlog); the ephemeral
    /// tail since the checkpoint is then applied in order. Any local
    /// frames the peer created while cut off were never checkpointed
    /// and never entered the ordered stream: they are discarded, and
    /// the discard is reported as an explicit [`TailLoss`].
    fn sync(&mut self, session: &LiveSession, local_uncheckpointed: &[Frame]) -> SyncReport {
        let mut report = SyncReport {
            from_checkpoint: false,
            applied_tail: 0,
            tail_loss: None,
        };
        if let Some(first) = local_uncheckpointed.first() {
            let last = local_uncheckpointed.last().unwrap();
            report.tail_loss = Some(TailLoss {
                from_seq: first.seq,
                until_seq: last.seq + 1,
            });
        }
        if let Some(committed) = session.latest_checkpoint() {
            if self.next_seq < committed.covered_until_seq || !local_uncheckpointed.is_empty() {
                let snapshot = session.latest_snapshot().unwrap();
                self.doc = Doc {
                    text: snapshot.chars().collect(),
                    cursors: HashMap::new(),
                };
                self.next_seq = committed.covered_until_seq;
                report.from_checkpoint = true;
            }
        }
        for frame in session.tail_since_checkpoint() {
            if frame.seq >= self.next_seq {
                self.receive(session, frame).unwrap();
                report.applied_tail += 1;
            }
        }
        report
    }
}

/// What a [`Peer::sync`] did, so tests can assert the path taken.
#[derive(Debug, PartialEq, Eq)]
struct SyncReport {
    from_checkpoint: bool,
    applied_tail: usize,
    tail_loss: Option<TailLoss>,
}

/// A member bridge: a peer in its own right that forwards frames to the
/// peer behind it — but only frames it has itself verified and applied.
/// A frame the bridge refuses goes no further.
struct Bridge {
    peer: Peer,
}

impl Bridge {
    fn new() -> Self {
        Self { peer: Peer::new() }
    }

    fn forward(&mut self, session: &LiveSession, frame: &Frame) -> Result<Frame, Reject> {
        self.peer.receive(session, frame)?;
        Ok(frame.clone())
    }
}

/// The outcome of reconstructing a document from durable state plus an
/// offered ephemeral tail.
#[derive(Debug)]
struct Reconstruction {
    doc: Doc,
    /// Whether a checkpoint anchored the reconstruction.
    from_checkpoint: bool,
    /// The tail loss, when the offered tail does not cover everything
    /// the session published: the reconstruction is then exactly the
    /// checkpoint state, and the gap is named, never papered over.
    tail_loss: Option<TailLoss>,
}

/// Reconstruct a late joiner's document: start from the latest
/// checkpoint's snapshot, then apply the offered ephemeral tail in
/// order. If the tail starts after the checkpoint's coverage, the
/// frames in between are lost with the uncheckpointed tail and the
/// reconstruction refuses to guess at them.
fn reconstruct(session: &LiveSession, offered_tail: &[Frame]) -> Reconstruction {
    let mut peer = Peer::new();
    let mut from_checkpoint = false;
    if let Some(committed) = session.latest_checkpoint() {
        let snapshot = session.latest_snapshot().unwrap();
        peer.doc = Doc {
            text: snapshot.chars().collect(),
            cursors: HashMap::new(),
        };
        peer.next_seq = committed.covered_until_seq;
        from_checkpoint = true;
    }
    let covered_until = peer.next_seq;
    if let Some(first) = offered_tail.first() {
        if first.seq > covered_until {
            return Reconstruction {
                doc: peer.doc,
                from_checkpoint,
                tail_loss: Some(TailLoss {
                    from_seq: covered_until,
                    until_seq: session.next_seq,
                }),
            };
        }
    }
    for frame in offered_tail {
        if frame.seq >= peer.next_seq {
            peer.receive(session, frame).unwrap();
        }
    }
    // Whatever the session published that this reconstruction never
    // saw — an uncheckpointed tail that vanished with its sender, or a
    // tail offered only in part — is named as a loss, not skipped.
    let tail_loss = if peer.next_seq < session.next_seq {
        Some(TailLoss {
            from_seq: peer.next_seq,
            until_seq: session.next_seq,
        })
    } else {
        None
    };
    Reconstruction {
        doc: peer.doc,
        from_checkpoint,
        tail_loss,
    }
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn participants(count: u8) -> Vec<VerifyingKey> {
    (1..=count)
        .map(|n| SigningKey::from_bytes(&[n; 32]).verifying_key())
        .collect()
}

const START_MS: u64 = 1_000;
const SESSION_KEY: [u8; 32] = [42; 32];

fn small_policy_session(count: u8) -> LiveSession {
    LiveSession::new(
        SESSION_KEY,
        &participants(count),
        START_MS,
        CheckpointPolicy {
            event_limit: 4,
            interval_ms: 2_000,
        },
    )
}

fn insert(pos: usize, text: &str) -> Op {
    Op::Insert {
        pos,
        text: text.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn test_two_direct_peers_and_one_bridge_end_to_end() {
    // Participants: 0 = session owner (A), 1 = B, 2 = the bridge member,
    // 3 = C, who is reachable only through the bridge.
    let mut session = small_policy_session(4);
    let mut a = Peer::new();
    let mut b = Peer::new();
    let mut bridge = Bridge::new();
    let mut c = Peer::new();

    // A and B exchange steps and cursors directly; every frame also
    // reaches C through the bridge, which verifies before forwarding.
    let script: Vec<(usize, Op, u64)> = vec![
        (0, insert(0, "hello"), START_MS),
        (1, insert(5, " world"), START_MS + 10),
        (0, Op::Cursor { pos: 5 }, START_MS + 20),
        (1, Op::Delete { pos: 0, len: 1 }, START_MS + 30),
        (0, insert(0, "H"), START_MS + 40),
        (1, Op::Cursor { pos: 10 }, START_MS + 50),
    ];
    for (sender, op, at) in script {
        let frame = session.publish(sender, op, at);
        a.receive(&session, &frame).unwrap();
        b.receive(&session, &frame).unwrap();
        let forwarded = bridge.forward(&session, &frame).unwrap();
        c.receive(&session, &forwarded).unwrap();
    }
    session.commit();
    // The second half of the script crossed the 4-frame segment limit,
    // and the explicit commit closed the remainder: two checkpoints.
    assert_eq!(session.graph.len(), 2);

    assert_eq!(a.doc.text(), "Hello world");
    assert_eq!(a.doc, b.doc);
    assert_eq!(a.doc, bridge.peer.doc);
    assert_eq!(a.doc, c.doc);
    assert_eq!(a.doc.cursors.get(&0), Some(&5));
    assert_eq!(a.doc.cursors.get(&1), Some(&10));
    // The bridge applied exactly the stream it forwarded.
    assert_eq!(bridge.peer.next_seq, session.next_seq);
}

#[test]
fn test_bad_mac_replay_and_wrong_session_are_rejected() {
    let mut session = small_policy_session(2);
    let mut b = Peer::new();

    let frame = session.publish(0, insert(0, "abc"), START_MS);
    b.receive(&session, &frame).unwrap();

    // Replay: the same frame a second time is refused, not re-applied.
    assert_eq!(
        b.receive(&session, &frame),
        Err(Reject::Replay { seq: frame.seq })
    );
    assert_eq!(b.doc.text(), "abc");

    // Bad MAC: a tampered payload no longer verifies.
    let mut tampered = session.publish(0, insert(3, "d"), START_MS + 10);
    tampered.op = insert(3, "X");
    assert_eq!(b.receive(&session, &tampered), Err(Reject::BadMac));
    // A frame whose MAC was flipped outright is refused the same way.
    let mut flipped = session.publish(1, Op::Cursor { pos: 1 }, START_MS + 20);
    flipped.mac[0] ^= 0xff;
    assert_eq!(b.receive(&session, &flipped), Err(Reject::BadMac));

    // Wrong session: a well-formed frame from another session (same
    // participants, same key — only the session binding differs) is
    // refused on its session ID before anything else.
    let mut other = LiveSession::new(
        SESSION_KEY,
        &participants(2),
        START_MS,
        CheckpointPolicy {
            event_limit: 4,
            interval_ms: 2_000,
        },
    );
    other.header.session_id = Hash::digest(b"a different session");
    let foreign = other.publish(0, insert(0, "zzz"), START_MS);
    assert_eq!(b.receive(&session, &foreign), Err(Reject::WrongSession));

    // A non-participant sender is refused before its MAC is considered.
    let outsider = build_frame(
        &SESSION_KEY,
        &session.header,
        b.next_seq,
        9,
        START_MS + 30,
        insert(0, "q"),
    );
    assert_eq!(b.receive(&session, &outsider), Err(Reject::NotParticipant));

    // The bridge refuses to forward what it cannot verify: the bad
    // frame never reaches the peer behind it.
    let mut bridge = Bridge::new();
    let mut behind = Peer::new();
    let first = session.tail_since_checkpoint()[0].clone();
    let forwarded = bridge.forward(&session, &first).unwrap();
    behind.receive(&session, &forwarded).unwrap();
    let mut bad = session.tail_since_checkpoint()[1].clone();
    bad.mac = [0; 32];
    assert_eq!(bridge.forward(&session, &bad), Err(Reject::BadMac));
    assert_eq!(behind.next_seq, 1, "only the verified frame got through");
}

#[test]
fn test_checkpoint_at_limits_and_explicit_commit() {
    let mut session = small_policy_session(2);

    // Frame limit: the 4th frame of a segment closes it immediately.
    for i in 0..4 {
        session.publish(0, insert(i, "a"), START_MS + i as u64);
    }
    assert_eq!(session.graph.len(), 1);
    let first_id = session.graph[0].id;
    {
        let first = &session.graph[0];
        assert_eq!(first.checkpoint.segment.segment_seq, 0);
        assert_eq!(first.checkpoint.segment.prev_checkpoint, None);
        assert_eq!(first.checkpoint.segment.event_count, 4);
        assert_eq!(first.covered_until_seq, 4);
    }
    // Once the segment closes, the ephemeral stream starts empty: the
    // frames live on only as the checkpoint's root and count.
    assert!(session.tail_since_checkpoint().is_empty());

    // Time limit: a frame arriving 2 s after the open segment's first
    // frame closes that segment before joining the stream.
    session.publish(1, insert(4, "b"), START_MS + 100);
    assert_eq!(session.graph.len(), 1);
    session.publish(1, insert(5, "c"), START_MS + 100 + 2_000);
    assert_eq!(session.graph.len(), 2);
    let second = &session.graph[1];
    assert_eq!(second.checkpoint.segment.segment_seq, 1);
    assert_eq!(
        second.checkpoint.segment.prev_checkpoint,
        Some(first_id),
        "each checkpoint names its predecessor, so a gap is detectable"
    );
    assert_eq!(second.checkpoint.segment.event_count, 1);

    // Explicit commit closes the partial segment that remains.
    session.commit();
    assert_eq!(session.graph.len(), 3);
    let third = &session.graph[2];
    assert_eq!(third.checkpoint.segment.event_count, 1);
    assert_eq!(third.covered_until_seq, session.next_seq);

    // Every committed checkpoint is a valid phase-1 payload, chained in
    // order, and an explicit commit with nothing open is a no-op.
    for committed in &session.graph {
        check_session_checkpoint(CHECKPOINT_KIND, &committed.checkpoint).unwrap();
    }
    session.commit();
    assert_eq!(session.graph.len(), 3);
}

#[test]
fn test_default_policy_checkpoints_at_256_frames() {
    // The phase-1 defaults are the design values, and a session on the
    // default policy checkpoints exactly at the 256th frame.
    let policy = CheckpointPolicy::default();
    assert_eq!(policy.event_limit, 256);
    assert_eq!(policy.interval_ms, 2_000);

    let mut session = LiveSession::new(SESSION_KEY, &participants(2), START_MS, policy);
    for i in 0..255 {
        session.publish(0, Op::Cursor { pos: i }, START_MS);
    }
    assert_eq!(session.graph.len(), 0);
    session.publish(0, Op::Cursor { pos: 255 }, START_MS);
    assert_eq!(session.graph.len(), 1);
    assert_eq!(session.graph[0].checkpoint.segment.event_count, 256);
}

#[test]
fn test_raw_frames_never_enter_the_event_store() {
    let mut session = small_policy_session(2);
    let marker = "zebra-unicorn-marker";
    for i in 0..6 {
        session.publish(0, insert(i * marker.len(), marker), START_MS + i as u64);
    }
    session.commit();
    assert_eq!(session.graph.len(), 2);

    // The durable graph, serialized whole, contains checkpoint payloads
    // only: roots, counts, time bounds, blob hashes. No frame payload —
    // neither the inserted text nor any operation encoding — appears.
    let graph_json = serde_json::to_string(
        &session
            .graph
            .iter()
            .map(|committed| &committed.checkpoint)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!graph_json.contains(marker));
    assert!(!graph_json.contains("insert"));
    for committed in &session.graph {
        check_session_checkpoint(CHECKPOINT_KIND, &committed.checkpoint).unwrap();
        // The checkpoint commits to the segment by root, not by content:
        // its root differs from any hash of a single frame's payload,
        // and the snapshot it names is verified by hash, held outside
        // the graph as an audit blob.
        let blob_hash = committed.checkpoint.segment.audit_blob.unwrap();
        let blob = session.blobs.get(&blob_hash).unwrap();
        assert_eq!(Hash::digest(blob), blob_hash);
    }
}

#[test]
fn test_join_reconnect_and_partition_heal() {
    let mut session = small_policy_session(3);
    let mut a = Peer::new();

    // Six frames: one full checkpointed segment of 4, then a tail of 2.
    let words = ["one ", "two ", "three ", "four ", "five ", "six "];
    let mut frames = Vec::new();
    let mut at = 0;
    for (i, word) in words.iter().enumerate() {
        let frame = session.publish(0, insert(at, word), START_MS + (i as u64) * 10);
        at += word.len();
        a.receive(&session, &frame).unwrap();
        frames.push(frame);
    }
    assert_eq!(session.graph.len(), 1);
    assert_eq!(a.doc.text(), "one two three four five six ");

    // Join: a late joiner reconstructs from the latest checkpoint plus
    // the ephemeral tail since it — no frame backlog for the closed
    // segment is offered or needed.
    let joined = reconstruct(&session, session.tail_since_checkpoint());
    assert!(joined.from_checkpoint);
    assert_eq!(joined.tail_loss, None);
    assert_eq!(joined.doc.text(), a.doc.text());

    // Reconnect: B received the first segment's frames, then went
    // offline and missed the tail. On reconnect it is offered the
    // checkpoint (which it is already level with) plus the tail.
    let mut b = Peer::new();
    for frame in &frames[..4] {
        b.receive(&session, frame).unwrap();
    }
    let report = b.sync(&session, &[]);
    assert_eq!(
        report,
        SyncReport {
            from_checkpoint: false,
            applied_tail: 2,
            tail_loss: None,
        }
    );
    assert_eq!(b.doc.text(), a.doc.text());

    // Partition heal: C was cut off before the session started
    // publishing, edited locally while partitioned (frames that never
    // entered the ordered stream and were never checkpointed), and now
    // heals. Its uncheckpointed local work is discarded and the loss is
    // explicit; its document converges on the checkpoint plus tail.
    let mut c = Peer::new();
    let local = vec![
        build_frame(
            &SESSION_KEY,
            &session.header,
            0,
            2,
            START_MS,
            insert(0, "LOCAL "),
        ),
        build_frame(
            &SESSION_KEY,
            &session.header,
            1,
            2,
            START_MS + 10,
            insert(6, "DRAFT"),
        ),
    ];
    for frame in &local {
        c.doc.apply(frame.sender, &frame.op);
    }
    let report = c.sync(&session, &local);
    assert!(report.from_checkpoint);
    assert_eq!(
        report.tail_loss,
        Some(TailLoss {
            from_seq: 0,
            until_seq: 2,
        })
    );
    assert_eq!(c.doc.text(), a.doc.text());
    assert!(!c.doc.text().contains("LOCAL"));
}

#[test]
fn test_uncheckpointed_tail_loss_is_explicit() {
    let mut session = small_policy_session(2);

    // One checkpointed segment, then two more frames that exist only in
    // the ephemeral stream.
    for i in 0..6 {
        session.publish(0, insert(i, "x"), START_MS + i as u64);
    }
    assert_eq!(session.graph.len(), 1);
    let checkpoint_state = session.latest_snapshot().unwrap();
    assert_eq!(checkpoint_state, "xxxx");

    // The session owner vanishes before committing: the uncheckpointed
    // tail is gone with it. A joiner offered nothing reconstructs
    // exactly the checkpoint state and is told, in so many words, which
    // sequences were lost.
    let crashed = reconstruct(&session, &[]);
    assert!(crashed.from_checkpoint);
    assert_eq!(crashed.doc.text(), checkpoint_state);
    assert_eq!(
        crashed.tail_loss,
        Some(TailLoss {
            from_seq: 4,
            until_seq: 6,
        })
    );

    // A tail that starts after the checkpoint's coverage has a gap at
    // its head: nothing past the gap is applied, and the whole range
    // the session published beyond the checkpoint is the named loss.
    let tail = session.tail_since_checkpoint().to_vec();
    let gapped = reconstruct(&session, &tail[1..]);
    assert_eq!(gapped.doc.text(), checkpoint_state);
    assert_eq!(
        gapped.tail_loss,
        Some(TailLoss {
            from_seq: 4,
            until_seq: 6,
        })
    );
}
