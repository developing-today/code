//! Session headers and segment checkpoints as signed events: phase 1 of the
//! #2036 messaging/relay protocol v1 (#3794), per the design record on #2036
//! (`doc/2026-11-01_design_id2036.md`, §4.3 and the phase table in §6) and the
//! residue fields of the #2034 investigation (§5/§7).
//!
//! Live session traffic itself stays out of the durable graph: frames are
//! MAC-authenticated and ephemeral. What the graph carries is the durable
//! contract every later phase builds on — one ordinary signed event kind,
//! `session.checkpoint`, whose header extension carries a [`SessionHeader`]
//! and a [`SegmentCheckpoint`] together.
//!
//! The header is the session's signed-once binding: a fresh session ID, its
//! purpose, the participant set, its start time and nonce, and a commitment
//! to the MAC key. The MAC secret itself is never published — the header
//! carries only `mac_key_commitment`, a hash of the key material — and
//! `mac_scope` states explicitly whether one key authenticates every
//! participant's frames or each participant has their own key. Key
//! derivation is the header author's business; rotation is a new session
//! with a new header, never an edit to this one.
//!
//! A checkpoint closes one ordered segment of the session: it names its
//! immediate predecessor checkpoint (so a gap in the segment chain is
//! detectable), a root over the ordered segment, the number of frames it
//! covers, and the segment's time bounds. A checkpoint attests only to the
//! signer's ordered segment commitment and endpoint state — never to
//! per-frame timing, and never to what every participant saw. The optional
//! audit-blob reference names retained audit material by hash; the blob is
//! not part of the event and may expire under the retention policy without
//! altering the signed checkpoint (#2036 §4.5/§5 rule 8).

use std::collections::HashSet;

use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

/// The kind of a `session.checkpoint` event, as the header's `kind` names it.
pub const CHECKPOINT_KIND: &str = "session.checkpoint";

/// The most bytes a session's purpose may have. The purpose is text inside
/// the header, and the header decoder declares text at most 512 bytes long;
/// a purpose stays well under that.
pub const MAX_PURPOSE_BYTES: usize = 256;

/// The most participants one session header may name.
pub const MAX_PARTICIPANTS: usize = 64;

/// The default segment size, in frames, at which the session owner signs a
/// checkpoint (#2036 §4.3). A default, not a protocol constant: a session
/// may configure its own limit in the `WorldLimits` style.
pub const DEFAULT_CHECKPOINT_EVENT_LIMIT: u64 = 256;

/// The default segment age, in milliseconds, at which the session owner
/// signs a checkpoint, whichever of the two defaults comes first.
pub const DEFAULT_CHECKPOINT_INTERVAL_MS: u64 = 2_000;

/// When a session signs a checkpoint: at `event_limit` frames or
/// `interval_ms` milliseconds, whichever comes first, and at explicit
/// commit/blur/end. Both limits are configurable per session; the defaults
/// are the #2036 values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointPolicy {
    /// Frames per segment before a checkpoint is due.
    pub event_limit: u64,
    /// Milliseconds per segment before a checkpoint is due.
    pub interval_ms: u64,
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            event_limit: DEFAULT_CHECKPOINT_EVENT_LIMIT,
            interval_ms: DEFAULT_CHECKPOINT_INTERVAL_MS,
        }
    }
}

/// Whose MAC key authenticates a participant's frames. The scope is part of
/// the signed header, so every participant can tell which key a frame's MAC
/// must verify under before checking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MacScope {
    /// One MAC key for the whole session, committed once in the header.
    /// Every participant holds it, so a valid MAC proves a participant sent
    /// the frame, not which one.
    Shared,
    /// One MAC key per participant, each committed in the header author's
    /// derivation from the session secret. A valid MAC under a participant's
    /// key attributes the frame to that participant.
    PerParticipant,
}

/// The signed-once binding of a live session: who may speak, about what,
/// from when, and under which MAC key commitment. Carried inside every
/// `session.checkpoint` event of the session, so the binding travels with
/// the checkpoints that rely on it and needs no separate event kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHeader {
    /// The session's fresh ID. Distinct from every event ID: it names the
    /// live session the checkpoints close segments of.
    pub session_id: Hash,
    /// What the session is for, as its author wrote it: non-empty, NFC, and
    /// at most [`MAX_PURPOSE_BYTES`] bytes.
    pub purpose: String,
    /// The participants allowed to send frames in this session: at least
    /// one, at most [`MAX_PARTICIPANTS`], none repeated.
    pub participants: Vec<VerifyingKey>,
    /// When the session started, in milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// A fresh nonce for this session, committed by hash. Together with
    /// `session_id` it makes the header un-replayable across sessions.
    pub nonce: Hash,
    /// A commitment to the MAC key material: a hash of the key, never the
    /// key itself. A receiver who holds the key checks it against this
    /// commitment before trusting any frame's MAC.
    pub mac_key_commitment: Hash,
    /// Which key authenticates a participant's frames (see [`MacScope`]).
    pub mac_scope: MacScope,
}

/// One closed segment of a session: the residue a compacted segment leaves
/// behind (#2034 §5/§7). The checkpoint event that carries it is signed by
/// the session owner; its signature is the event's own, so no signature
/// field is repeated here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentCheckpoint {
    /// The segment's number within the session, counting from 0. Strictly
    /// increasing along the checkpoint chain: each checkpoint's number is
    /// its predecessor's plus one (the chain is verified by following
    /// `prev_checkpoint`, whose own number the receiver checks).
    pub segment_seq: u64,
    /// The event ID of the previous `session.checkpoint` of this session,
    /// and `None` for segment 0 and only segment 0. A missing predecessor
    /// is a detectable gap, never a silent restart.
    pub prev_checkpoint: Option<Hash>,
    /// A root over the ordered segment: a hash committing to every frame
    /// of the segment in order.
    pub root: Hash,
    /// How many frames the segment holds.
    pub event_count: u64,
    /// When the segment's first frame was sent, in milliseconds since the
    /// Unix epoch: never before the session's `started_ms`.
    pub start_ms: u64,
    /// When the segment's last frame was sent, in milliseconds since the
    /// Unix epoch: never before `start_ms`.
    pub end_ms: u64,
    /// The retained audit material for this segment, named by hash, when
    /// the session's retention policy keeps any. The blob is not part of
    /// the event; its later expiry does not alter the signed checkpoint.
    pub audit_blob: Option<Hash>,
}

/// What a `session.checkpoint` event carries: the session's header and the
/// segment this checkpoint closes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    /// The session's signed header (see [`SessionHeader`]).
    pub header: SessionHeader,
    /// The segment this checkpoint closes (see [`SegmentCheckpoint`]).
    pub segment: SegmentCheckpoint,
}

/// Why a session checkpoint was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// The payload is not the one the event's kind names.
    #[error("a {kind:?} event carries the wrong session checkpoint payload")]
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
    /// A field breaks a limit or a shape rule.
    #[error("{0}")]
    Field(String),
}

/// Whether `kind` is the session checkpoint kind, so its event carries a
/// session checkpoint payload.
#[must_use]
pub fn is_checkpoint_kind(kind: &str) -> bool {
    kind == CHECKPOINT_KIND
}

/// Checks a session checkpoint payload against the kind of the event that
/// carries it.
///
/// # Errors
///
/// Returns [`CheckpointError::KindMismatch`] when the carrying event's kind
/// is not [`CHECKPOINT_KIND`]. Returns [`CheckpointError::Field`] when the
/// purpose is empty, not NFC, or over [`MAX_PURPOSE_BYTES`]; when the
/// participant set is empty, over [`MAX_PARTICIPANTS`], or names one
/// participant twice; when segment 0 names a predecessor or a later
/// segment names none; or when the segment's time bounds run backwards or
/// start before the session did.
pub fn check_session_checkpoint(
    kind: &str,
    checkpoint: &SessionCheckpoint,
) -> Result<(), CheckpointError> {
    if kind != CHECKPOINT_KIND {
        return Err(CheckpointError::KindMismatch {
            kind: kind.to_owned(),
        });
    }
    let header = &checkpoint.header;
    if header.purpose.is_empty() {
        return Err(CheckpointError::Field(
            "a session purpose is not empty".to_owned(),
        ));
    }
    if header.purpose.len() > MAX_PURPOSE_BYTES {
        return Err(CheckpointError::Field(format!(
            "a session purpose is {} bytes, it may be at most {MAX_PURPOSE_BYTES}",
            header.purpose.len()
        )));
    }
    if !is_nfc(&header.purpose) {
        return Err(CheckpointError::Field(
            "a session purpose is not NFC".to_owned(),
        ));
    }
    if header.participants.is_empty() {
        return Err(CheckpointError::Field(
            "a session names at least one participant".to_owned(),
        ));
    }
    if header.participants.len() > MAX_PARTICIPANTS {
        return Err(CheckpointError::Field(format!(
            "a session names {} participants, it may name at most {MAX_PARTICIPANTS}",
            header.participants.len()
        )));
    }
    let mut seen = HashSet::with_capacity(header.participants.len());
    if !header
        .participants
        .iter()
        .all(|participant| seen.insert(participant.as_bytes().to_vec()))
    {
        return Err(CheckpointError::Field(
            "a session names a participant twice".to_owned(),
        ));
    }
    let segment = &checkpoint.segment;
    match (segment.segment_seq, segment.prev_checkpoint) {
        (0, Some(_)) => {
            return Err(CheckpointError::Field(
                "segment 0 names no previous checkpoint".to_owned(),
            ));
        }
        (seq, None) if seq > 0 => {
            return Err(CheckpointError::Field(
                "a segment after segment 0 names its previous checkpoint".to_owned(),
            ));
        }
        _ => {}
    }
    if segment.start_ms < header.started_ms {
        return Err(CheckpointError::Field(
            "a segment starts before its session".to_owned(),
        ));
    }
    if segment.end_ms < segment.start_ms {
        return Err(CheckpointError::Field(
            "a segment ends before it starts".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use p2panda_core::SigningKey;

    use super::*;

    fn participant(n: u8) -> VerifyingKey {
        SigningKey::from_bytes(&[n; 32]).verifying_key()
    }

    fn checkpoint() -> SessionCheckpoint {
        SessionCheckpoint {
            header: SessionHeader {
                session_id: Hash::digest(b"session"),
                purpose: "collab editing".to_owned(),
                participants: vec![participant(1), participant(2)],
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
        }
    }

    #[test]
    fn a_well_formed_checkpoint_passes() {
        check_session_checkpoint(CHECKPOINT_KIND, &checkpoint()).unwrap();
        let mut later = checkpoint();
        later.segment.segment_seq = 1;
        later.segment.prev_checkpoint = Some(Hash::digest(b"previous"));
        check_session_checkpoint(CHECKPOINT_KIND, &later).unwrap();
    }

    #[test]
    fn the_checkpoint_policy_defaults_are_the_design_values() {
        let policy = CheckpointPolicy::default();
        assert_eq!(policy.event_limit, 256);
        assert_eq!(policy.interval_ms, 2_000);
    }

    #[test]
    fn a_checkpoint_under_another_kind_is_refused() {
        assert!(matches!(
            check_session_checkpoint("message", &checkpoint()),
            Err(CheckpointError::KindMismatch { .. })
        ));
    }

    #[test]
    fn segment_zero_with_a_predecessor_is_refused() {
        let mut bad = checkpoint();
        bad.segment.prev_checkpoint = Some(Hash::digest(b"previous"));
        assert!(check_session_checkpoint(CHECKPOINT_KIND, &bad).is_err());
    }

    #[test]
    fn a_later_segment_without_a_predecessor_is_refused() {
        let mut bad = checkpoint();
        bad.segment.segment_seq = 3;
        assert!(check_session_checkpoint(CHECKPOINT_KIND, &bad).is_err());
    }

    #[test]
    fn a_duplicate_participant_is_refused() {
        let mut bad = checkpoint();
        bad.header.participants.push(participant(1));
        assert!(check_session_checkpoint(CHECKPOINT_KIND, &bad).is_err());
    }

    #[test]
    fn an_empty_participant_set_is_refused() {
        let mut bad = checkpoint();
        bad.header.participants.clear();
        assert!(check_session_checkpoint(CHECKPOINT_KIND, &bad).is_err());
    }

    #[test]
    fn a_segment_ending_before_it_starts_is_refused() {
        let mut bad = checkpoint();
        bad.segment.end_ms = bad.segment.start_ms - 1;
        assert!(check_session_checkpoint(CHECKPOINT_KIND, &bad).is_err());
    }

    #[test]
    fn a_segment_starting_before_the_session_is_refused() {
        let mut bad = checkpoint();
        bad.segment.start_ms = bad.header.started_ms - 1;
        bad.segment.end_ms = bad.segment.start_ms;
        assert!(check_session_checkpoint(CHECKPOINT_KIND, &bad).is_err());
    }
}
