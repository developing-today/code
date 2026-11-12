//! F12.13 journal replication: wire contract and crypto primitives (#806).
//!
//! Phase 1 of the phased plan in `doc/2026-11-01_design_id806.md` (design
//! record on #806, merged in PR #3822). This module defines only the
//! contract every later phase builds on — the signed replication envelope,
//! the cursor/hash-chain rules, bounded payload behavior, and the
//! receiver-side verification state machine. It contains no network
//! serving, no journal I/O, and no replica storage: the writer feed
//! (phase 2), the durable receiver (phase 3), and spectator serving
//! (phases 4–5) consume these types.
//!
//! ## F12.6 seam disposition
//!
//! Design §4.6/§7 gate phase 1 on #1063 slice 1 (task 4885) settling the
//! shared F12.6 framing seam, with an explicit fallback: "If #1063 does not
//! define reusable wire framing, record that explicitly and have F12.13 own
//! its versioned envelope." As of 2026-11-02, #1063 slice 1 (PR #4017) is an
//! audit-log feature — it records audit facts at call sites and defines no
//! reusable wire-framing contract — so the fallback applies and this module
//! is the explicitly versioned, F12.13-owned envelope the design calls for.
//! The envelope is carried as the payload of the existing length-prefixed
//! transport framing ([`crate::frames`]); if F12.6 later defines a reusable
//! outer framing, only that carriage changes, not the envelope, the cursor
//! rules, or the signature contract below.
//!
//! ## Contract summary
//!
//! - One writer. A frame is a [`SignedReplicationEnvelope`]: an
//!   [`ReplicationEnvelope`] plus an Ed25519 signature over the
//!   domain-separated canonical encoding of every envelope field and the
//!   exact payload bytes. Verification uses `verify_strict` and requires
//!   the signer to be the frame's claimed `source`, which the receiver
//!   must additionally pin out of band ([`ReplicationTracker`] is built
//!   with the pinned source). A key supplied only by the stream is never
//!   trusted on its own.
//! - The cursor is `(entry ordinal, rolling digest)` over the exact
//!   journal-entry bytes (the JSONL lines the journal stores, newline
//!   included). The chain base is seeded with the replication domain, the
//!   world ID, the source node ID, and the journal header version, so a
//!   compaction or header bump changes the base and forces a reset
//!   snapshot. The cursor can be recomputed from the current journal file
//!   after a restart with [`chain_base_cursor`] and [`advance_cursor`].
//! - Every signature binds a fresh per-session nonce, so captured frames
//!   cannot be replayed into another sync session. Within a session,
//!   frames also chain by digest: each frame names the digest of the
//!   previous frame's exact encoded bytes.
//! - Payloads carry exact serialized journal-entry bytes, never a lossy
//!   projection. A frame payload is bounded by
//!   [`MAX_REPLICATION_PAYLOAD_BYTES`]; larger batches are split at entry
//!   boundaries with [`split_entry_payload`]. A signed
//!   [`ReplicationKind::Terminal`] frame commits the completed
//!   snapshot/batch cursor; a batch without its terminal frame is not
//!   complete and must not replace replica state.
//! - The writer bounds every newly compacted checkpoint part to
//!   [`MAX_CHECKPOINT_PART_BYTES`] encoded bytes (192 KiB), below the v1
//!   payload limit. Compacted snapshots, retained events, and admin history
//!   are whole journal entries split at their source boundaries, so each
//!   part remains independently replicable without changing this envelope.
//!   This writer-side guarantee does not repair oversized checkpoints left
//!   by older binaries; those are handled separately from the v1 wire path.
//!
//! ## Admission contract (#4592)
//!
//! **A frame is admitted only if its header version names a chain that
//! could exist: inside the restore range
//! `JOURNAL_VERSION..=MAX_JOURNAL_VERSION`, and — for a snapshot, the one
//! frame kind allowed to change the chain's version — never lower than
//! the version of the chain the tracker already holds.**
//!
//! [`ReplicationTracker::accept`] enforces both clauses for every frame
//! kind before any state changes: the envelope range check refuses an
//! out-of-range `header_version` with
//! [`ReplicationError::UnsupportedJournalVersion`], and the Snapshot arm
//! refuses a lower-than-current version. Construction enforces the range
//! clause too: [`ReplicationTracker::new`] refuses an out-of-range
//! session version, so no live tracker can hold one. The Phase 3 closing
//! audit (#4593) dispositioned every precondition
//! [`super::WorldCore::restore`] demands of a journal against these
//! checks; its one confirmed gap — per-entry version requirements are
//! enforced at restore but not yet at accept — is filed as #7970.
//! - A journal compacted by an older binary can still hold one oversized
//!   `Checkpoint` entry above the v1 bound. The feed scans the captured
//!   journal source at build time and compacts the legacy journal once
//!   through the existing compaction path before splitting it (when the
//!   world can snapshot and compact); otherwise the feed fails with the
//!   entry ordinal, encoded size, and payload bound and asks for compaction
//!   rather than failing at [`split_entry_payload`] with no context.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::{JOURNAL_VERSION, JournalEntry, MAX_CHECKPOINT_PART_BYTES, MAX_JOURNAL_VERSION};

/// Domain separator bound into every signature and hash-chain step, so a
/// replication signature or digest can never be reinterpreted by another
/// protocol in this crate (design §4.3: replication-specific domain tag).
pub const REPLICATION_DOMAIN: &[u8] = b"developing-today/id/world-replication/v1\0";

/// Frame marker prefixing the postcard-encoded signed envelope, separating
/// replication frames from any other payload on a shared transport.
pub const REPLICATION_FRAME_MAGIC: &[u8] = b"id-world-replication/v1\n";

/// Version of the envelope defined by this module. A receiver refuses any
/// other version; a future incompatible contract bumps this (and the
/// domain/magic version tags with it).
pub const REPLICATION_ENVELOPE_VERSION: u8 = 1;

/// Largest payload one replication frame may carry, in bytes. Bounded well
/// below the transport frame bound ([`crate::frames::MAX_FRAME_BYTES`]) so
/// a signed envelope plus its encoding overhead always fits one transport
/// frame; larger batches are split at entry boundaries
/// ([`split_entry_payload`]).
pub const MAX_REPLICATION_PAYLOAD_BYTES: usize = 256 * 1024;

/// Largest accepted encoded replication frame, in bytes. Matches the
/// transport framing bound ([`crate::frames::MAX_FRAME_BYTES`]).
pub const MAX_REPLICATION_FRAME_BYTES: usize = 1024 * 1024;

/// Length of the per-session nonce every signature binds, in bytes.
pub const SESSION_NONCE_BYTES: usize = 16;

/// A position in a world's replicated journal: how many entries the chain
/// has absorbed, and the rolling digest over their exact bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationCursor {
    /// Number of journal entries absorbed into the chain so far.
    pub ordinal: u64,
    /// Rolling BLAKE3 digest over the absorbed entry bytes, seeded by
    /// [`chain_base_cursor`].
    pub digest: [u8; 32],
}

/// The chain base for one world's journal under one source and header
/// version: ordinal 0 and a digest seeded with the replication domain,
/// world ID, source node ID, and header version. Compaction or a header
/// bump changes the inputs, so a replica whose cursor is not in the new
/// chain is detected immediately and must take a reset snapshot.
#[must_use]
pub fn chain_base_cursor(
    world_id: &str,
    source: &[u8; 32],
    header_version: u32,
) -> ReplicationCursor {
    let seed = postcard::to_allocvec(&(REPLICATION_DOMAIN, world_id, source, header_version))
        .unwrap_or_default();
    ReplicationCursor {
        ordinal: 0,
        digest: *blake3::hash(&seed).as_bytes(),
    }
}

/// Advance the chain by one journal entry's exact bytes (the stored JSONL
/// line, newline included). Domain-separated and length-prefixed, so two
/// different entry splits of the same byte stream chain differently.
#[must_use]
pub fn advance_cursor(cursor: ReplicationCursor, entry_bytes: &[u8]) -> ReplicationCursor {
    let mut hasher = blake3::Hasher::new();
    hasher.update(REPLICATION_DOMAIN);
    hasher.update(&cursor.digest);
    hasher.update(
        &u64::try_from(entry_bytes.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    hasher.update(entry_bytes);
    ReplicationCursor {
        ordinal: cursor.ordinal + 1,
        digest: *hasher.finalize().as_bytes(),
    }
}

/// The exact replicated bytes of one journal entry: its JSON encoding plus
/// the newline the journal file stores, matching `world_store`'s on-disk
/// line format.
#[must_use]
pub fn journal_entry_line(entry: &JournalEntry) -> Vec<u8> {
    let mut line = serde_json::to_vec(entry).unwrap_or_default();
    line.push(b'\n');
    line
}

/// Digest of a frame's exact encoded bytes (magic included), as named by
/// the next frame's `prev_frame_digest`.
#[must_use]
pub fn frame_digest(frame_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(REPLICATION_DOMAIN);
    hasher.update(frame_bytes);
    *hasher.finalize().as_bytes()
}

/// Split a concatenated journal-entry payload into chunks of at most
/// [`MAX_REPLICATION_PAYLOAD_BYTES`], breaking only at entry (newline)
/// boundaries so every chunk is itself a valid frame payload.
///
/// # Errors
///
/// [`ReplicationError::MalformedPayload`] if a non-empty payload does not
/// end at an entry boundary, or
/// [`ReplicationError::PayloadTooLarge`] if one entry alone exceeds the
/// per-frame bound (such an entry can never be replicated in v1). The feed
/// scans its captured source before this call so a legacy journal is
/// re-compacted or reported actionably instead of failing here with no
/// context.
pub fn split_entry_payload(payload: &[u8]) -> Result<Vec<Vec<u8>>, ReplicationError> {
    if payload.is_empty() {
        return Ok(Vec::new());
    }
    if !payload.ends_with(b"\n") {
        return Err(ReplicationError::MalformedPayload);
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut chunk_end = 0;
    for (idx, byte) in payload.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let entry_end = idx + 1;
        if entry_end - start > MAX_REPLICATION_PAYLOAD_BYTES {
            return Err(ReplicationError::PayloadTooLarge);
        }
        if entry_end - chunk_end > MAX_REPLICATION_PAYLOAD_BYTES {
            chunks.push(payload[chunk_end..start].to_vec());
            chunk_end = start;
        }
        start = entry_end;
    }
    if chunk_end < payload.len() {
        chunks.push(payload[chunk_end..].to_vec());
    }
    Ok(chunks)
}

/// What a replication frame carries (design §4.1/§4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicationKind {
    /// A complete signed reset snapshot: the full journal from the chain
    /// base, sent on first sync, after compaction, and after a header
    /// bump. Replaces the receiver's chain wholesale.
    Snapshot,
    /// An ordered batch of journal entries continuing the current chain.
    Tail,
    /// Commits the snapshot/batch whose cursor the receiver has already
    /// reached. Carries no payload; a batch without its terminal frame is
    /// incomplete and must not replace replica state.
    Terminal,
}

/// The unsigned replication envelope. Every field is bound by the
/// signature in [`SignedReplicationEnvelope`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationEnvelope {
    /// Envelope contract version; must equal
    /// [`REPLICATION_ENVELOPE_VERSION`].
    pub version: u8,
    /// World the journal belongs to.
    pub world_id: String,
    /// Ed25519 public key bytes of the single writer; the only key whose
    /// signature a receiver may accept, and only when it matches the
    /// receiver's pinned source.
    pub source: [u8; 32],
    /// Journal header version the payload's entries belong to. A receiver
    /// refuses a version outside its supported range before applying entries.
    pub header_version: u32,
    /// What this frame carries.
    pub kind: ReplicationKind,
    /// Fresh random nonce of the sync session; bound by the signature so
    /// captured frames cannot be replayed into another session.
    pub session_nonce: [u8; SESSION_NONCE_BYTES],
    /// Chain position before this frame's payload.
    pub cursor_before: ReplicationCursor,
    /// Chain position after this frame's payload. For
    /// [`ReplicationKind::Terminal`], equal to `cursor_before`: the
    /// terminal frame commits the cursor the batch already reached.
    pub cursor_after: ReplicationCursor,
    /// [`frame_digest`] of the previous frame in this session, or all
    /// zeroes for the first frame.
    pub prev_frame_digest: [u8; 32],
    /// BLAKE3 digest of `payload`, domain-separated.
    pub payload_digest: [u8; 32],
    /// Exact journal-entry bytes (JSONL lines), or empty for
    /// [`ReplicationKind::Terminal`].
    pub payload: Vec<u8>,
}

impl ReplicationEnvelope {
    /// Number of journal entries this frame's payload carries, derived
    /// from the cursors so it cannot disagree with them.
    #[must_use]
    pub fn entry_count(&self) -> u64 {
        self.cursor_after
            .ordinal
            .saturating_sub(self.cursor_before.ordinal)
    }

    /// Build a tail frame continuing `cursor_before`, computing the
    /// after-cursor and payload digest from the exact entry bytes.
    ///
    /// # Errors
    ///
    /// Returns the chain errors of [`chain_payload`]:
    /// [`ReplicationError::PayloadTooLarge`] or
    /// [`ReplicationError::MalformedPayload`].
    pub fn tail(
        world_id: &str,
        source: [u8; 32],
        header_version: u32,
        session_nonce: [u8; SESSION_NONCE_BYTES],
        cursor_before: ReplicationCursor,
        prev_frame_digest: [u8; 32],
        payload: Vec<u8>,
    ) -> Result<Self, ReplicationError> {
        let cursor_after = chain_payload(cursor_before, &payload)?;
        Ok(Self {
            version: REPLICATION_ENVELOPE_VERSION,
            world_id: world_id.to_string(),
            source,
            header_version,
            kind: ReplicationKind::Tail,
            session_nonce,
            cursor_before,
            cursor_after,
            prev_frame_digest,
            payload_digest: payload_digest(&payload),
            payload,
        })
    }

    /// Build a snapshot frame: a complete journal from the chain base for
    /// `header_version`, computing the after-cursor and payload digest.
    ///
    /// # Errors
    ///
    /// Returns the chain errors of [`chain_payload`]:
    /// [`ReplicationError::PayloadTooLarge`] or
    /// [`ReplicationError::MalformedPayload`].
    pub fn snapshot(
        world_id: &str,
        source: [u8; 32],
        header_version: u32,
        session_nonce: [u8; SESSION_NONCE_BYTES],
        prev_frame_digest: [u8; 32],
        payload: Vec<u8>,
    ) -> Result<Self, ReplicationError> {
        let cursor_before = chain_base_cursor(world_id, &source, header_version);
        let cursor_after = chain_payload(cursor_before, &payload)?;
        Ok(Self {
            version: REPLICATION_ENVELOPE_VERSION,
            world_id: world_id.to_string(),
            source,
            header_version,
            kind: ReplicationKind::Snapshot,
            session_nonce,
            cursor_before,
            cursor_after,
            prev_frame_digest,
            payload_digest: payload_digest(&payload),
            payload,
        })
    }

    /// Build the terminal frame committing `cursor` — the position the
    /// preceding snapshot/tail frames of this batch already reached.
    #[must_use]
    pub fn terminal(
        world_id: &str,
        source: [u8; 32],
        header_version: u32,
        session_nonce: [u8; SESSION_NONCE_BYTES],
        cursor: ReplicationCursor,
        prev_frame_digest: [u8; 32],
    ) -> Self {
        Self {
            version: REPLICATION_ENVELOPE_VERSION,
            world_id: world_id.to_string(),
            source,
            header_version,
            kind: ReplicationKind::Terminal,
            session_nonce,
            cursor_before: cursor,
            cursor_after: cursor,
            prev_frame_digest,
            payload_digest: payload_digest(&[]),
            payload: Vec::new(),
        }
    }
}

/// Domain-separated BLAKE3 digest of a frame payload.
#[must_use]
pub fn payload_digest(payload: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(REPLICATION_DOMAIN);
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

/// Chain a whole payload (zero or more complete entry lines) from
/// `cursor`, refusing anything that is not a sequence of complete entries
/// within the per-frame payload bound.
///
/// # Errors
///
/// [`ReplicationError::PayloadTooLarge`] if the payload exceeds
/// [`MAX_REPLICATION_PAYLOAD_BYTES`];
/// [`ReplicationError::MalformedPayload`] if it does not end at an entry
/// boundary.
pub fn chain_payload(
    mut cursor: ReplicationCursor,
    payload: &[u8],
) -> Result<ReplicationCursor, ReplicationError> {
    if payload.len() > MAX_REPLICATION_PAYLOAD_BYTES {
        return Err(ReplicationError::PayloadTooLarge);
    }
    if payload.is_empty() {
        return Ok(cursor);
    }
    if !payload.ends_with(b"\n") {
        return Err(ReplicationError::MalformedPayload);
    }
    for entry in payload.split_inclusive(|byte| *byte == b'\n') {
        // A line that does not decode as a journal entry is not an entry,
        // however the chain arithmetic treats its bytes: refuse it here so
        // a verified cursor never covers data the restore path cannot read.
        serde_json::from_slice::<JournalEntry>(entry)
            .map_err(|_| ReplicationError::MalformedPayload)?;
        cursor = advance_cursor(cursor, entry);
    }
    Ok(cursor)
}

/// Check a snapshot/tail payload's own journal header against the frame
/// that carries it (#4486). Chaining only proves the payload is a sequence
/// of decodable entries; restore additionally demands the journal begin
/// with exactly one `Created` header for this world at a supported
/// version and contain no second header. A signed payload violating that
/// would advance the verified cursor over a journal no replica can ever
/// restore, so [`ReplicationTracker::accept`] refuses it here, before any
/// state changes.
///
/// - Snapshot: the first entry must be a `Created` header whose world and
///   version equal the envelope's `world_id` and `header_version`, and no
///   later entry may be a `Created`.
/// - Tail: a tail starting at the chain base carries journal entry 0
///   itself, so its first entry may be that same matching header; any
///   other `Created` in a tail is a second header mid-journal.
///
/// # Errors
///
/// [`ReplicationError::InvalidJournalHeader`] on a header violation;
/// [`ReplicationError::MalformedPayload`] if a line does not decode (the
/// same refusal [`chain_payload`] would give).
fn validate_payload_journal_header(envelope: &ReplicationEnvelope) -> Result<(), ReplicationError> {
    if envelope.payload.is_empty() {
        return match envelope.kind {
            // An empty snapshot carries no header at all: missing header.
            ReplicationKind::Snapshot => Err(ReplicationError::InvalidJournalHeader),
            _ => Ok(()),
        };
    }
    if !envelope.payload.ends_with(b"\n") {
        return Err(ReplicationError::MalformedPayload);
    }
    let mut entries = Vec::new();
    for line in envelope.payload.split_inclusive(|byte| *byte == b'\n') {
        entries.push(
            serde_json::from_slice::<JournalEntry>(line)
                .map_err(|_| ReplicationError::MalformedPayload)?,
        );
    }
    let header_matches = |entry: &JournalEntry| match entry {
        JournalEntry::Created { world_id, version } => {
            world_id == &envelope.world_id && *version == envelope.header_version
        }
        _ => false,
    };
    let is_created = |entry: &JournalEntry| matches!(entry, JournalEntry::Created { .. });
    match envelope.kind {
        ReplicationKind::Snapshot => {
            if !entries.first().is_some_and(|entry| header_matches(entry)) {
                return Err(ReplicationError::InvalidJournalHeader);
            }
            if entries[1..].iter().any(|entry| is_created(entry)) {
                return Err(ReplicationError::InvalidJournalHeader);
            }
        }
        ReplicationKind::Tail => {
            let starts_at_base = envelope.cursor_before
                == chain_base_cursor(
                    &envelope.world_id,
                    &envelope.source,
                    envelope.header_version,
                );
            for (idx, entry) in entries.iter().enumerate() {
                if !is_created(entry) {
                    continue;
                }
                if idx == 0 && starts_at_base && header_matches(entry) {
                    continue;
                }
                return Err(ReplicationError::InvalidJournalHeader);
            }
        }
        ReplicationKind::Terminal => {}
    }
    Ok(())
}

/// Check every rule [`ReplicationTracker::accept`] enforces that is
/// decidable from the envelope alone (#6469): envelope version, journal
/// header version range, payload bound and digest equality, the payload
/// chaining to `cursor_after` (which also refuses a malformed payload),
/// the snapshot base cursor, the terminal shape (empty payload,
/// `cursor_before == cursor_after`), and payload journal-header
/// agreement (#4486). Receiver-state rules — world/pinned-source
/// matching, session nonce, `prev_frame_digest` chaining, a tail's
/// cursor position or a snapshot's version against the tracker's
/// current chain — are deliberately NOT checked here: they are not the
/// envelope's to decide.
///
/// # Errors
///
/// The [`ReplicationError`] naming the first failed rule, exactly as
/// `accept` would name it for the same envelope.
fn validate_envelope_local(envelope: &ReplicationEnvelope) -> Result<(), ReplicationError> {
    if envelope.version != REPLICATION_ENVELOPE_VERSION {
        return Err(ReplicationError::UnsupportedEnvelopeVersion(
            envelope.version,
        ));
    }
    if !(JOURNAL_VERSION..=MAX_JOURNAL_VERSION).contains(&envelope.header_version) {
        return Err(ReplicationError::UnsupportedJournalVersion(
            envelope.header_version,
        ));
    }
    if envelope.payload.len() > MAX_REPLICATION_PAYLOAD_BYTES {
        return Err(ReplicationError::PayloadTooLarge);
    }
    if envelope.payload_digest != payload_digest(&envelope.payload) {
        return Err(ReplicationError::PayloadDigestMismatch);
    }
    match envelope.kind {
        ReplicationKind::Snapshot => {
            let base = chain_base_cursor(
                &envelope.world_id,
                &envelope.source,
                envelope.header_version,
            );
            if envelope.cursor_before != base {
                return Err(ReplicationError::CursorFork);
            }
            validate_payload_journal_header(envelope)?;
            let chained = chain_payload(envelope.cursor_before, &envelope.payload)?;
            if chained != envelope.cursor_after {
                return Err(ReplicationError::EntryChainMismatch);
            }
        }
        ReplicationKind::Tail => {
            validate_payload_journal_header(envelope)?;
            let chained = chain_payload(envelope.cursor_before, &envelope.payload)?;
            if chained != envelope.cursor_after {
                return Err(ReplicationError::EntryChainMismatch);
            }
        }
        ReplicationKind::Terminal => {
            if !envelope.payload.is_empty() || envelope.cursor_before != envelope.cursor_after {
                return Err(ReplicationError::TerminalCursorMismatch);
            }
        }
    }
    Ok(())
}

/// A replication envelope plus its Ed25519 signature — the frame a
/// replication stream actually carries (as the payload of the transport
/// framing, see [`crate::frames`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedReplicationEnvelope {
    /// The envelope; every field is signed.
    pub envelope: ReplicationEnvelope,
    /// Ed25519 signature over [`Self::signing_bytes`], by the key whose
    /// public half is `envelope.source`.
    pub signature: Vec<u8>,
}

impl SignedReplicationEnvelope {
    /// Sign an envelope, refusing to sign for a key that is not the
    /// envelope's claimed source, and refusing any envelope that fails
    /// an envelope-local rule [`ReplicationTracker::accept`] enforces
    /// (#6469, [`validate_envelope_local`]): an unsupported envelope
    /// version or journal header version, an over-bound payload, a
    /// payload digest that does not match the payload, a payload that
    /// does not chain to `cursor_after` (including a malformed
    /// payload), a snapshot whose before-cursor is not the chain base,
    /// a terminal carrying a payload or unequal cursors, or a payload
    /// whose journal header disagrees with the envelope (#4486). A
    /// frame this signer produces is therefore never one every
    /// receiver would refuse on envelope-local grounds; receiver-state
    /// rules (pinned source, session nonce, frame chaining, chain
    /// position) remain `accept`'s alone.
    pub fn sign(envelope: ReplicationEnvelope, key: &SigningKey) -> Option<Self> {
        if envelope.source != *key.verifying_key().as_bytes() {
            return None;
        }
        validate_envelope_local(&envelope).ok()?;
        let mut signed = Self {
            envelope,
            signature: Vec::new(),
        };
        signed.signature = key.sign(&signed.signing_bytes().ok()?).to_bytes().to_vec();
        Some(signed)
    }

    /// Encode this frame: the replication magic, then the postcard
    /// encoding of the signed envelope.
    pub fn encode_frame(&self) -> Result<Vec<u8>, postcard::Error> {
        let payload = postcard::to_allocvec(self)?;
        let mut frame = Vec::with_capacity(REPLICATION_FRAME_MAGIC.len() + payload.len());
        frame.extend_from_slice(REPLICATION_FRAME_MAGIC);
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    /// Decode a frame's bytes. This checks only shape (magic, size bound,
    /// decodability); authentication and chain validation are
    /// [`ReplicationTracker::accept`]'s job — never apply a payload from a
    /// frame that has not passed it.
    pub fn decode_frame(frame: &[u8]) -> Option<Self> {
        if frame.len() > MAX_REPLICATION_FRAME_BYTES {
            return None;
        }
        let payload = frame.strip_prefix(REPLICATION_FRAME_MAGIC)?;
        // `take_from_bytes`, not `from_bytes`: trailing bytes after the
        // envelope must refuse the frame, not be silently ignored — the
        // frame digest covers the whole received buffer, so an unsigned
        // suffix would otherwise desynchronize the frame chain.
        let (signed, rest) = postcard::take_from_bytes::<Self>(payload).ok()?;
        if !rest.is_empty() {
            return None;
        }
        Some(signed)
    }

    /// Verify the signature against the envelope's claimed source key,
    /// with `verify_strict`. Does not check that the source is the
    /// receiver's pinned source — [`ReplicationTracker::accept`] does.
    ///
    /// # Errors
    ///
    /// [`ReplicationError::BadSignature`] if the signature is malformed or
    /// does not verify.
    pub fn verify_signature(&self) -> Result<(), ReplicationError> {
        let verifying_key = VerifyingKey::from_bytes(&self.envelope.source)
            .map_err(|_| ReplicationError::BadSignature)?;
        let signature =
            Signature::from_slice(&self.signature).map_err(|_| ReplicationError::BadSignature)?;
        verifying_key
            .verify_strict(
                &self
                    .signing_bytes()
                    .map_err(|_| ReplicationError::BadSignature)?,
                &signature,
            )
            .map_err(|_| ReplicationError::BadSignature)
    }

    /// Domain-separated canonical encoding of every envelope field and the
    /// exact payload bytes: what the signature binds.
    fn signing_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(&(
            REPLICATION_DOMAIN,
            self.envelope.version,
            &self.envelope.world_id,
            &self.envelope.source,
            self.envelope.header_version,
            self.envelope.kind,
            &self.envelope.session_nonce,
            self.envelope.cursor_before,
            self.envelope.cursor_after,
            &self.envelope.prev_frame_digest,
            &self.envelope.payload_digest,
            &self.envelope.payload,
        ))
    }
}

/// Why a replication frame was refused. Every refusal leaves the
/// receiver's verified state untouched (design §5.3/§5.4).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReplicationError {
    /// The frame bytes do not decode as a replication frame at all.
    #[error("malformed replication frame")]
    MalformedFrame,
    /// The frame's envelope version is not the one this module defines.
    #[error("unsupported replication envelope version {0}")]
    UnsupportedEnvelopeVersion(u8),
    /// The frame or tracker uses a journal header version outside this
    /// binary's supported range; refused at the header boundary.
    #[error("unsupported journal header version {0}")]
    UnsupportedJournalVersion(u32),
    /// A tail/terminal frame's header version differs from the chain's.
    #[error("journal header version changed without a reset snapshot")]
    HeaderVersionChanged,
    /// The frame names a different world than the receiver replicates.
    #[error("replication frame is for a different world")]
    WorldMismatch,
    /// The frame's source is not the receiver's pinned source.
    #[error("replication frame source is not the pinned source")]
    UntrustedSource,
    /// The signature does not verify against the frame's source key.
    #[error("replication frame signature does not verify")]
    BadSignature,
    /// The frame belongs to a different sync session (nonce mismatch):
    /// a replay of captured frames into a new session.
    #[error("replication frame belongs to a different session")]
    NonceMismatch,
    /// The frame does not name the previous frame's digest: a gap or a
    /// reordered/forked frame stream.
    #[error("replication frame does not continue the frame chain")]
    FrameChainMismatch,
    /// The frame starts after the receiver's cursor: entries are missing.
    #[error("replication frame starts after the verified cursor (gap)")]
    CursorGap,
    /// The frame starts before the receiver's cursor: a replay of
    /// already-applied entries.
    #[error("replication frame starts before the verified cursor (replay)")]
    StaleCursor,
    /// The frame starts at the receiver's ordinal but on a different
    /// digest chain: a fork.
    #[error("replication frame continues a different chain (fork)")]
    CursorFork,
    /// The payload does not chain to the frame's claimed after-cursor.
    #[error("replication payload does not chain to the claimed cursor")]
    EntryChainMismatch,
    /// The payload's digest differs from the signed payload digest.
    #[error("replication payload digest mismatch")]
    PayloadDigestMismatch,
    /// The payload exceeds the per-frame bound.
    #[error("replication payload exceeds the per-frame bound")]
    PayloadTooLarge,
    /// The payload is not a sequence of complete entry lines, or a line
    /// does not decode as a journal entry.
    #[error("replication payload is not complete journal entries")]
    MalformedPayload,
    /// The payload's own journal header does not match the frame: a
    /// snapshot that does not begin with exactly one [`JournalEntry::Created`]
    /// header naming the envelope's world and header version, a duplicate
    /// header later in a snapshot, or a header entry carried by a tail past
    /// the journal start. Such a payload chains fine but
    /// [`super::WorldCore::restore`] refuses the journal, so a verified
    /// cursor must never advance over it (#4486).
    #[error("replication payload journal header does not match the frame")]
    InvalidJournalHeader,
    /// A terminal frame names a cursor the batch has not reached — the
    /// batch is truncated or the terminal belongs to another batch.
    #[error("terminal frame does not match the reached cursor")]
    TerminalCursorMismatch,
}

/// A frame that passed [`ReplicationTracker::accept`]: its payload (the
/// exact entry bytes) may now be applied, in accept order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptedFrame {
    /// What the frame carried.
    pub kind: ReplicationKind,
    /// The verified chain position after this frame.
    pub cursor_after: ReplicationCursor,
    /// Exact journal-entry bytes to append (empty for a terminal frame).
    pub payload: Vec<u8>,
}

/// Receiver-side verification state for one sync session with one pinned
/// source (design §4.5): checks the signature, trusted signer, domain,
/// world, session nonce, frame chain, cursor continuity, previous digest,
/// entry decoding, and header version before any batch is made visible.
/// State advances only on a fully verified frame, so a refused frame
/// leaves the last complete replica state intact.
#[derive(Clone, Debug)]
pub struct ReplicationTracker {
    world_id: String,
    pinned_source: [u8; 32],
    session_nonce: [u8; SESSION_NONCE_BYTES],
    header_version: u32,
    cursor: ReplicationCursor,
    last_frame_digest: [u8; 32],
}

impl ReplicationTracker {
    /// Start a session: the chain sits at the base for `header_version`
    /// and no frame has been accepted yet.
    ///
    /// # Errors
    ///
    /// Returns [`ReplicationError::UnsupportedJournalVersion`] when
    /// `header_version` is outside `JOURNAL_VERSION..=MAX_JOURNAL_VERSION`.
    #[must_use]
    pub fn new(
        world_id: &str,
        pinned_source: [u8; 32],
        header_version: u32,
        session_nonce: [u8; SESSION_NONCE_BYTES],
    ) -> Result<Self, ReplicationError> {
        if !(JOURNAL_VERSION..=MAX_JOURNAL_VERSION).contains(&header_version) {
            return Err(ReplicationError::UnsupportedJournalVersion(header_version));
        }

        Ok(Self {
            world_id: world_id.to_string(),
            pinned_source,
            session_nonce,
            header_version,
            cursor: chain_base_cursor(world_id, &pinned_source, header_version),
            last_frame_digest: [0; 32],
        })
    }

    /// The current verified chain position.
    #[must_use]
    pub fn cursor(&self) -> ReplicationCursor {
        self.cursor
    }

    /// The journal header version of the current chain.
    #[must_use]
    pub fn header_version(&self) -> u32 {
        self.header_version
    }

    /// Verify one encoded frame and, if it passes every check, advance
    /// the session state and return the payload to apply.
    ///
    /// This is where the module's admission contract (see the module
    /// doc, #4592) is enforced: a frame is admitted only if its header
    /// version names a chain that could exist.
    ///
    /// # Errors
    ///
    /// The [`ReplicationError`] naming the first failed check; the
    /// tracker's state is unchanged on any error.
    pub fn accept(&mut self, frame_bytes: &[u8]) -> Result<AcceptedFrame, ReplicationError> {
        let signed = SignedReplicationEnvelope::decode_frame(frame_bytes)
            .ok_or(ReplicationError::MalformedFrame)?;
        let envelope = &signed.envelope;
        if envelope.version != REPLICATION_ENVELOPE_VERSION {
            return Err(ReplicationError::UnsupportedEnvelopeVersion(
                envelope.version,
            ));
        }
        // The supported journal range is 1..=MAX_JOURNAL_VERSION,
        // matching validate_committed_journal on the receiver side:
        // a version-0 snapshot accepted here would commit, then brick
        // the replica at its next open (and destroy any valid journal
        // it replaced).
        if !(JOURNAL_VERSION..=MAX_JOURNAL_VERSION).contains(&envelope.header_version) {
            return Err(ReplicationError::UnsupportedJournalVersion(
                envelope.header_version,
            ));
        }
        if envelope.world_id != self.world_id {
            return Err(ReplicationError::WorldMismatch);
        }
        if envelope.source != self.pinned_source {
            return Err(ReplicationError::UntrustedSource);
        }
        signed.verify_signature()?;
        if envelope.session_nonce != self.session_nonce {
            return Err(ReplicationError::NonceMismatch);
        }
        if envelope.prev_frame_digest != self.last_frame_digest {
            return Err(ReplicationError::FrameChainMismatch);
        }
        if envelope.payload.len() > MAX_REPLICATION_PAYLOAD_BYTES {
            return Err(ReplicationError::PayloadTooLarge);
        }
        if envelope.payload_digest != payload_digest(&envelope.payload) {
            return Err(ReplicationError::PayloadDigestMismatch);
        }
        match envelope.kind {
            ReplicationKind::Snapshot => {
                // A snapshot rebases the chain, but journal header
                // versions only ever move forward: a snapshot carrying a
                // LOWER header version than the chain's current one is a
                // stale or forged stream, never a legitimate writer
                // state. Refuse it like the Tail/Terminal arms refuse a
                // version change, leaving verified state untouched —
                // otherwise one snapshot frame silently rolls the
                // replica back and re-admits old-version tails the Tail
                // arm refused moments earlier (issue #4487).
                if envelope.header_version < self.header_version {
                    return Err(ReplicationError::HeaderVersionChanged);
                }
                // Its before-cursor must be the base for its own header
                // version, whatever the current chain is (first sync,
                // compaction, header bump).
                let base = chain_base_cursor(
                    &envelope.world_id,
                    &envelope.source,
                    envelope.header_version,
                );
                if envelope.cursor_before != base {
                    return Err(ReplicationError::CursorFork);
                }
                validate_payload_journal_header(envelope)?;
                let chained = chain_payload(envelope.cursor_before, &envelope.payload)?;
                if chained != envelope.cursor_after {
                    return Err(ReplicationError::EntryChainMismatch);
                }
                self.cursor = envelope.cursor_after;
                self.header_version = envelope.header_version;
            }
            ReplicationKind::Tail => {
                if envelope.header_version != self.header_version {
                    return Err(ReplicationError::HeaderVersionChanged);
                }
                if envelope.cursor_before.ordinal > self.cursor.ordinal {
                    return Err(ReplicationError::CursorGap);
                }
                if envelope.cursor_before.ordinal < self.cursor.ordinal {
                    return Err(ReplicationError::StaleCursor);
                }
                if envelope.cursor_before != self.cursor {
                    return Err(ReplicationError::CursorFork);
                }
                validate_payload_journal_header(envelope)?;
                let chained = chain_payload(envelope.cursor_before, &envelope.payload)?;
                if chained != envelope.cursor_after {
                    return Err(ReplicationError::EntryChainMismatch);
                }
                self.cursor = envelope.cursor_after;
            }
            ReplicationKind::Terminal => {
                if envelope.header_version != self.header_version {
                    return Err(ReplicationError::HeaderVersionChanged);
                }
                if !envelope.payload.is_empty()
                    || envelope.cursor_before != envelope.cursor_after
                    || envelope.cursor_after != self.cursor
                {
                    return Err(ReplicationError::TerminalCursorMismatch);
                }
            }
        }
        self.last_frame_digest = frame_digest(frame_bytes);
        Ok(AcceptedFrame {
            kind: envelope.kind,
            cursor_after: envelope.cursor_after,
            payload: envelope.payload.clone(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{JournalEntry, WorldCore, WorldEvent, WorldEventKind, WorldLimits};

    const WORLD: &str = "world-replication-test";
    const NONCE: [u8; SESSION_NONCE_BYTES] = [7; SESSION_NONCE_BYTES];

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn source_of(key: &SigningKey) -> [u8; 32] {
        *key.verifying_key().as_bytes()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Three real journal entries as their exact replicated line bytes.
    fn entry_payload() -> Vec<u8> {
        entry_payload_with_version(1)
    }

    /// [`entry_payload`] with a chosen journal header version.
    fn entry_payload_with_version(version: u32) -> Vec<u8> {
        let entries = [
            JournalEntry::Created {
                world_id: WORLD.to_string(),
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

    fn tracker(key: &SigningKey) -> ReplicationTracker {
        ReplicationTracker::new(WORLD, source_of(key), 1, NONCE).unwrap()
    }

    #[test]
    fn tracker_requires_a_supported_starting_header_version() {
        let key = signing_key(1);
        assert_eq!(
            ReplicationTracker::new(WORLD, source_of(&key), 0, NONCE).err(),
            Some(ReplicationError::UnsupportedJournalVersion(0))
        );
        assert_eq!(
            ReplicationTracker::new(WORLD, source_of(&key), MAX_JOURNAL_VERSION + 1, NONCE).err(),
            Some(ReplicationError::UnsupportedJournalVersion(
                MAX_JOURNAL_VERSION + 1
            ))
        );

        for version in JOURNAL_VERSION..=MAX_JOURNAL_VERSION {
            assert_eq!(
                ReplicationTracker::new(WORLD, source_of(&key), version, NONCE)
                    .map(|tracker| tracker.header_version()),
                Ok(version)
            );
        }
    }

    #[test]
    fn signed_v0_frames_are_refused_without_advancing_tracker_state() {
        let key = signing_key(1);
        let source = source_of(&key);
        let payload = entry_payload();
        let initial_cursor = chain_base_cursor(WORLD, &source, JOURNAL_VERSION);
        let invalid_frames = [
            ReplicationEnvelope::snapshot(WORLD, source, 0, NONCE, [0; 32], payload.clone())
                .unwrap(),
            ReplicationEnvelope::tail(
                WORLD,
                source,
                0,
                NONCE,
                initial_cursor,
                [0; 32],
                payload.clone(),
            )
            .unwrap(),
            ReplicationEnvelope::terminal(WORLD, source, 0, NONCE, initial_cursor, [0; 32]),
        ];

        for envelope in invalid_frames {
            // Journal version 0 is envelope-locally invalid, so the
            // production signer refuses these (#6469); the bypass
            // signer builds the frames so `accept`'s refusal is tested.
            let frame_bytes = sign_bypass(envelope, &key);
            let mut receiver =
                ReplicationTracker::new(WORLD, source, JOURNAL_VERSION, NONCE).unwrap();
            let before_cursor = receiver.cursor();
            let before_header_version = receiver.header_version();

            assert_eq!(
                receiver.accept(&frame_bytes),
                Err(ReplicationError::UnsupportedJournalVersion(0))
            );
            assert_eq!(receiver.cursor(), before_cursor);
            assert_eq!(receiver.header_version(), before_header_version);

            // A valid first frame still chains from the original digest,
            // proving the refused frame did not mutate hidden tracker state.
            let control = ReplicationEnvelope::tail(
                WORLD,
                source,
                JOURNAL_VERSION,
                NONCE,
                before_cursor,
                [0; 32],
                payload.clone(),
            )
            .unwrap();
            let control_bytes = SignedReplicationEnvelope::sign(control, &key)
                .unwrap()
                .encode_frame()
                .unwrap();
            assert!(receiver.accept(&control_bytes).is_ok());
        }
    }

    #[test]
    fn signed_tails_with_supported_header_versions_are_accepted() {
        let key = signing_key(1);
        let source = source_of(&key);
        for version in JOURNAL_VERSION..=MAX_JOURNAL_VERSION {
            let payload = entry_payload_with_version(version);
            let mut receiver = ReplicationTracker::new(WORLD, source, version, NONCE).unwrap();
            let tail = ReplicationEnvelope::tail(
                WORLD,
                source,
                version,
                NONCE,
                receiver.cursor(),
                [0; 32],
                payload.clone(),
            )
            .unwrap();
            let frame_bytes = SignedReplicationEnvelope::sign(tail, &key)
                .unwrap()
                .encode_frame()
                .unwrap();

            let accepted = receiver.accept(&frame_bytes).unwrap();
            assert_eq!(accepted.kind, ReplicationKind::Tail);
            assert_eq!(receiver.header_version(), version);
        }
    }

    /// A signed tail frame over `payload`, chained from `cursor` with
    /// frame-chain digest `prev`.
    fn signed_tail(
        key: &SigningKey,
        cursor: ReplicationCursor,
        prev: [u8; 32],
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let envelope =
            ReplicationEnvelope::tail(WORLD, source_of(key), 1, NONCE, cursor, prev, payload)
                .unwrap();
        SignedReplicationEnvelope::sign(envelope, key)
            .unwrap()
            .encode_frame()
            .unwrap()
    }

    /// A tail frame built through the bypass signer, for shapes the
    /// production signer refuses (#6469) whose `accept`-side check
    /// ordering is under test.
    fn signed_tail_bypass(
        key: &SigningKey,
        cursor: ReplicationCursor,
        prev: [u8; 32],
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let envelope =
            ReplicationEnvelope::tail(WORLD, source_of(key), 1, NONCE, cursor, prev, payload)
                .unwrap();
        sign_bypass(envelope, key)
    }

    /// Sign an envelope the production signer refuses to sign (#6469),
    /// replicating the signing-bytes contract so the frame still
    /// passes signature verification and reaches the `accept` check
    /// under test. Receiver tests that need a lying signer use this
    /// bypass, never the production `sign` (mirrors `sign_unbounded`
    /// in `replication_receiver`).
    fn sign_bypass(envelope: ReplicationEnvelope, key: &SigningKey) -> Vec<u8> {
        let mut signed = SignedReplicationEnvelope {
            envelope,
            signature: Vec::new(),
        };
        signed.signature = key
            .sign(&signed.signing_bytes().unwrap())
            .to_bytes()
            .to_vec();
        signed.encode_frame().unwrap()
    }

    /// A signed snapshot frame over `payload` built through the bypass
    /// signer, for journal-header shapes the production signer refuses.
    fn signed_snapshot_bypass(
        key: &SigningKey,
        version: u32,
        prev: [u8; 32],
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source_of(key), version, NONCE, prev, payload)
                .unwrap();
        sign_bypass(envelope, key)
    }

    #[test]
    fn chain_base_is_a_pinned_deterministic_test_vector() {
        let source = [3u8; 32];
        let base = chain_base_cursor(WORLD, &source, 1);
        assert_eq!(base.ordinal, 0);
        // Pinned vector: any change to the domain tag, seed fields, or
        // seed encoding changes this digest and must be a deliberate,
        // version-bumped contract change.
        assert_eq!(
            hex(&base.digest),
            "d32092e6f4f8a8e813644b4e8a14ad2439d50c320c41fcb2aa9d24e6c0d1d291"
        );
        // Domain separation: another world, source, or header version is
        // a different chain.
        assert_ne!(base, chain_base_cursor("other-world", &source, 1));
        assert_ne!(base, chain_base_cursor(WORLD, &[4u8; 32], 1));
        assert_ne!(base, chain_base_cursor(WORLD, &source, 2));
        // The chain is reproducible from the entry bytes alone (restart).
        let payload = entry_payload();
        let mut cursor = base;
        for line in payload.split_inclusive(|byte| *byte == b'\n') {
            cursor = advance_cursor(cursor, line);
        }
        assert_eq!(cursor, chain_payload(base, &payload).unwrap());
        assert_eq!(cursor.ordinal, 3);
    }

    #[test]
    fn tail_round_trip_verifies_and_advances_the_cursor() {
        let key = signing_key(1);
        let mut receiver = tracker(&key);
        let payload = entry_payload();
        let frame = signed_tail(
            &key,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
            payload.clone(),
        );
        let decoded = SignedReplicationEnvelope::decode_frame(&frame).unwrap();
        assert_eq!(decoded.envelope.entry_count(), 3);
        let accepted = receiver.accept(&frame).unwrap();
        assert_eq!(accepted.kind, ReplicationKind::Tail);
        assert_eq!(accepted.payload, payload);
        assert_eq!(accepted.cursor_after.ordinal, 3);
        assert_eq!(receiver.cursor(), accepted.cursor_after);
        // The terminal frame then commits the reached cursor.
        let terminal = ReplicationEnvelope::terminal(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            receiver.cursor(),
            frame_digest(&frame),
        );
        let terminal_bytes = SignedReplicationEnvelope::sign(terminal, &key)
            .unwrap()
            .encode_frame()
            .unwrap();
        let committed = receiver.accept(&terminal_bytes).unwrap();
        assert_eq!(committed.kind, ReplicationKind::Terminal);
        assert!(committed.payload.is_empty());
    }

    #[test]
    fn tampered_frames_are_refused() {
        let key = signing_key(1);
        let payload = entry_payload();
        let frame = signed_tail(
            &key,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
            payload,
        );
        // Mutating a payload byte after signing breaks the signature.
        let mut tampered = SignedReplicationEnvelope::decode_frame(&frame).unwrap();
        tampered.envelope.payload[0] ^= 0xff;
        let tampered_bytes = tampered.encode_frame().unwrap();
        assert_eq!(
            tracker(&key).accept(&tampered_bytes),
            Err(ReplicationError::BadSignature)
        );
        // Mutating the claimed cursor after signing also breaks it.
        let mut tampered = SignedReplicationEnvelope::decode_frame(&frame).unwrap();
        tampered.envelope.cursor_after.ordinal += 1;
        let tampered_bytes = tampered.encode_frame().unwrap();
        assert_eq!(
            tracker(&key).accept(&tampered_bytes),
            Err(ReplicationError::BadSignature)
        );
        // A correctly signed frame whose payload digest lies is refused
        // on the digest check, past the signature check.
        let mut lying = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
            entry_payload(),
        )
        .unwrap();
        lying.payload_digest = payload_digest(b"not the payload");
        let lying_bytes = sign_bypass(lying, &key);
        assert_eq!(
            tracker(&key).accept(&lying_bytes),
            Err(ReplicationError::PayloadDigestMismatch)
        );
    }

    #[test]
    fn truncated_frames_and_unfinished_batches_are_refused() {
        let key = signing_key(1);
        let frame = signed_tail(
            &key,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
            entry_payload(),
        );
        // A truncated frame does not decode, and does not advance state.
        let cut = &frame[..frame.len() - 5];
        assert!(SignedReplicationEnvelope::decode_frame(cut).is_none());
        let mut receiver = tracker(&key);
        assert_eq!(receiver.accept(cut), Err(ReplicationError::MalformedFrame));
        assert_eq!(receiver.cursor().ordinal, 0);
        // A terminal frame for a cursor the batch never reached (the
        // batch was truncated before its last entry) does not commit.
        let wrong_terminal = ReplicationEnvelope::terminal(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            ReplicationCursor {
                ordinal: 99,
                digest: [9; 32],
            },
            [0; 32],
        );
        let wrong_bytes = SignedReplicationEnvelope::sign(wrong_terminal, &key)
            .unwrap()
            .encode_frame()
            .unwrap();
        assert_eq!(
            tracker(&key).accept(&wrong_bytes),
            Err(ReplicationError::TerminalCursorMismatch)
        );
    }

    #[test]
    fn gap_fork_and_replay_are_refused() {
        let key = signing_key(1);
        let base = chain_base_cursor(WORLD, &source_of(&key), 1);
        let payload = entry_payload();
        let first_line_end = payload.iter().position(|byte| *byte == b'\n').unwrap() + 1;

        // Gap: a frame that starts at entry 1 while the receiver is at 0.
        let gap_frame = signed_tail(
            &key,
            advance_cursor(base, &payload[..first_line_end]),
            [0; 32],
            payload[first_line_end..].to_vec(),
        );
        assert_eq!(
            tracker(&key).accept(&gap_frame),
            Err(ReplicationError::CursorGap)
        );

        // Fork: two different first frames; after accepting one, the
        // other no longer continues the verified chain.
        let frame_a = signed_tail(&key, base, [0; 32], payload.clone());
        let mut other_payload = payload.clone();
        other_payload.extend_from_slice(&journal_entry_line(&JournalEntry::CapsGranted {
            granted: vec!["builder".to_string()],
        }));
        let frame_b = signed_tail(&key, base, [0; 32], other_payload);
        let mut receiver = tracker(&key);
        receiver.accept(&frame_a).unwrap();
        assert_eq!(
            receiver.accept(&frame_b),
            Err(ReplicationError::FrameChainMismatch)
        );
        // Replay: the already-accepted frame itself is stale now.
        assert_eq!(
            receiver.accept(&frame_a),
            Err(ReplicationError::FrameChainMismatch)
        );
        // A frame naming a bogus previous-frame digest is refused before
        // its cursor is even considered.
        let bad_prev = signed_tail_bypass(&key, receiver.cursor(), [8; 32], payload.clone());
        assert_eq!(
            receiver.accept(&bad_prev),
            Err(ReplicationError::FrameChainMismatch)
        );
    }

    #[test]
    fn unsupported_versions_are_refused_at_the_boundary() {
        let key = signing_key(1);
        let base = chain_base_cursor(WORLD, &source_of(&key), 1);
        // Envelope version from the future.
        let mut future = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            base,
            [0; 32],
            entry_payload(),
        )
        .unwrap();
        future.version = REPLICATION_ENVELOPE_VERSION + 1;
        let future_bytes = sign_bypass(future, &key);
        assert_eq!(
            tracker(&key).accept(&future_bytes),
            Err(ReplicationError::UnsupportedEnvelopeVersion(2))
        );
        // Journal header version newer than this binary reads: refused
        // before any entry is applied (F11.12 header-boundary rule).
        let future_journal = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            MAX_JOURNAL_VERSION + 1,
            NONCE,
            base,
            [0; 32],
            entry_payload(),
        )
        .unwrap();
        let future_journal_bytes = sign_bypass(future_journal, &key);
        let mut receiver = tracker(&key);
        let initial_cursor = receiver.cursor();
        assert_eq!(
            receiver.accept(&future_journal_bytes),
            Err(ReplicationError::UnsupportedJournalVersion(
                MAX_JOURNAL_VERSION + 1
            ))
        );
        assert_eq!(receiver.cursor(), initial_cursor);
        assert_eq!(receiver.header_version(), JOURNAL_VERSION);
    }

    #[test]
    fn untrusted_sources_and_cross_session_replays_are_refused() {
        let key = signing_key(1);
        let stranger = signing_key(2);
        let base = chain_base_cursor(WORLD, &source_of(&key), 1);
        let frame = signed_tail(&key, base, [0; 32], entry_payload());
        // A receiver pinned to a different source refuses the frame.
        let mut pinned_elsewhere =
            ReplicationTracker::new(WORLD, source_of(&stranger), 1, NONCE).unwrap();
        assert_eq!(
            pinned_elsewhere.accept(&frame),
            Err(ReplicationError::UntrustedSource)
        );
        // Signing for another key's source claim is refused at sign time.
        let claim = ReplicationEnvelope::tail(
            WORLD,
            source_of(&stranger),
            1,
            NONCE,
            chain_base_cursor(WORLD, &source_of(&stranger), 1),
            [0; 32],
            entry_payload(),
        )
        .unwrap();
        assert!(SignedReplicationEnvelope::sign(claim, &key).is_none());
        // The same frame replayed into a session with a fresh nonce is
        // refused: the signature binds the session nonce.
        let mut other_session =
            ReplicationTracker::new(WORLD, source_of(&key), 1, [8; SESSION_NONCE_BYTES]).unwrap();
        assert_eq!(
            other_session.accept(&frame),
            Err(ReplicationError::NonceMismatch)
        );
    }

    #[test]
    fn payload_bounds_are_enforced() {
        let key = signing_key(1);
        // An over-bound payload cannot even be signed into a frame.
        let oversized = vec![b'x'; MAX_REPLICATION_PAYLOAD_BYTES + 1];
        let mut envelope = ReplicationEnvelope::terminal(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
        );
        envelope.payload = oversized;
        assert!(SignedReplicationEnvelope::sign(envelope, &key).is_none());
        // Chaining refuses an over-bound or boundary-less payload.
        let base = chain_base_cursor(WORLD, &source_of(&key), 1);
        assert_eq!(
            chain_payload(base, &vec![b'x'; MAX_REPLICATION_PAYLOAD_BYTES + 1]),
            Err(ReplicationError::PayloadTooLarge)
        );
        assert_eq!(
            chain_payload(base, b"no newline at the end"),
            Err(ReplicationError::MalformedPayload)
        );
        // Splitting respects the bound and entry boundaries, and the
        // chunks reassemble to the original payload.
        let line = journal_entry_line(&JournalEntry::CapsGranted {
            granted: vec!["chat".to_string()],
        });
        let mut big = Vec::new();
        while big.len() <= MAX_REPLICATION_PAYLOAD_BYTES {
            big.extend_from_slice(&line);
        }
        let chunks = split_entry_payload(&big).unwrap();
        assert!(chunks.len() >= 2);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= MAX_REPLICATION_PAYLOAD_BYTES)
        );
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.ends_with(b"\n") && !chunk.is_empty())
        );
        assert_eq!(chunks.concat(), big);
        // One entry larger than the bound can never be replicated.
        let giant_line = vec![b'y'; MAX_REPLICATION_PAYLOAD_BYTES + 1];
        let mut giant = giant_line;
        giant.push(b'\n');
        assert_eq!(
            split_entry_payload(&giant),
            Err(ReplicationError::PayloadTooLarge)
        );
        assert_eq!(
            split_entry_payload(b"unterminated"),
            Err(ReplicationError::MalformedPayload)
        );
        assert_eq!(split_entry_payload(&[]), Ok(Vec::new()));
    }

    #[test]
    fn oversized_compacted_checkpoint_round_trips_through_v1_snapshot_frames() {
        let mut core = WorldCore::new(WORLD, WorldLimits::default()).unwrap();
        core.active_module_hash = Some("checkpoint-module".to_owned());
        core.sequence = 3;
        let events: Vec<WorldEvent> = (1..=3)
            .map(|sequence| WorldEvent {
                sequence,
                participant_id: 0,
                kind: WorldEventKind::Chat(format!("{}-{sequence}", "é\"".repeat(35_000))),
            })
            .collect();
        core.events = events.clone().into();
        core.admin_history = events.clone().into();
        core.admin_history_complete = true;
        let snapshot = "🧭\"".repeat(100_000);
        let entries = core.checkpoint_entries(snapshot.clone()).unwrap();
        let header_version = match entries.first() {
            Some(JournalEntry::Created { version, .. }) => *version,
            _ => panic!("compacted journal must start with its header"),
        };
        assert_eq!(
            header_version,
            super::super::JOURNAL_VERSION_CHECKPOINT_PARTS
        );

        let journal_bytes: Vec<u8> = entries.iter().flat_map(journal_entry_line).collect();
        let chunks = split_entry_payload(&journal_bytes).unwrap();
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= MAX_REPLICATION_PAYLOAD_BYTES)
        );

        let key = signing_key(9);
        let source = source_of(&key);
        let base = chain_base_cursor(WORLD, &source, header_version);
        let mut cursor = base;
        let mut previous_frame_digest = [0; 32];
        let mut receiver = ReplicationTracker::new(WORLD, source, header_version, NONCE).unwrap();
        let mut received = Vec::new();
        for (index, payload) in chunks.into_iter().enumerate() {
            let envelope = if index == 0 {
                ReplicationEnvelope::snapshot(
                    WORLD,
                    source,
                    header_version,
                    NONCE,
                    previous_frame_digest,
                    payload,
                )
                .unwrap()
            } else {
                ReplicationEnvelope::tail(
                    WORLD,
                    source,
                    header_version,
                    NONCE,
                    cursor,
                    previous_frame_digest,
                    payload,
                )
                .unwrap()
            };
            cursor = envelope.cursor_after;
            let frame = SignedReplicationEnvelope::sign(envelope, &key)
                .unwrap()
                .encode_frame()
                .unwrap();
            previous_frame_digest = frame_digest(&frame);
            let accepted = receiver.accept(&frame).unwrap();
            received.extend_from_slice(&accepted.payload);
        }
        let terminal = ReplicationEnvelope::terminal(
            WORLD,
            source,
            header_version,
            NONCE,
            cursor,
            previous_frame_digest,
        );
        let terminal_frame = SignedReplicationEnvelope::sign(terminal, &key)
            .unwrap()
            .encode_frame()
            .unwrap();
        assert_eq!(
            receiver.accept(&terminal_frame).unwrap().kind,
            ReplicationKind::Terminal
        );

        assert_eq!(received, journal_bytes);
        // The expected cursor chains each bounded payload in turn; the
        // whole journal is itself over the per-payload bound, which is
        // the defect this test covers.
        let mut expected_cursor = base;
        for chunk in split_entry_payload(&journal_bytes).unwrap() {
            expected_cursor = chain_payload(expected_cursor, &chunk).unwrap();
        }
        assert_eq!(receiver.cursor(), expected_cursor);
        let replicated_entries: Vec<JournalEntry> = received
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        let restored =
            WorldCore::restore_replica(WORLD, WorldLimits::default(), replicated_entries).unwrap();
        assert_eq!(restored.program.unwrap().snapshot, Some(snapshot));
        assert_eq!(
            restored.core.events.iter().cloned().collect::<Vec<_>>(),
            events
        );
        assert!(restored.core.admin_history_complete);
    }

    #[test]
    fn snapshot_rebases_the_chain_on_a_header_bump() {
        let key = signing_key(1);
        let mut receiver = tracker(&key);
        let first = signed_tail(
            &key,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
            entry_payload(),
        );
        receiver.accept(&first).unwrap();
        // After a header bump the writer sends a snapshot on the new
        // chain base; the receiver rebases instead of demanding
        // continuity with the old chain.
        let snapshot_payload = entry_payload_with_version(2);
        let snapshot = ReplicationEnvelope::snapshot(
            WORLD,
            source_of(&key),
            2,
            NONCE,
            frame_digest(&first),
            snapshot_payload.clone(),
        )
        .unwrap();
        let snapshot_bytes = SignedReplicationEnvelope::sign(snapshot, &key)
            .unwrap()
            .encode_frame()
            .unwrap();
        let accepted = receiver.accept(&snapshot_bytes).unwrap();
        assert_eq!(accepted.kind, ReplicationKind::Snapshot);
        assert_eq!(receiver.header_version(), 2);
        assert_eq!(accepted.cursor_after.ordinal, 3);
        assert_eq!(
            accepted.cursor_after,
            chain_payload(
                chain_base_cursor(WORLD, &source_of(&key), 2),
                &snapshot_payload
            )
            .unwrap()
        );
        // A tail on the old header version is now refused.
        let stale_tail = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            receiver.cursor(),
            frame_digest(&snapshot_bytes),
            entry_payload(),
        )
        .unwrap();
        let stale_bytes = sign_bypass(stale_tail, &key);
        assert_eq!(
            receiver.accept(&stale_bytes),
            Err(ReplicationError::HeaderVersionChanged)
        );
    }

    #[test]
    fn snapshot_refuses_a_header_version_downgrade() {
        let key = signing_key(1);
        let mut receiver = tracker(&key);
        let first = signed_tail(
            &key,
            chain_base_cursor(WORLD, &source_of(&key), 1),
            [0; 32],
            entry_payload(),
        );
        receiver.accept(&first).unwrap();
        // Header-bump rebase to v2, as in
        // snapshot_rebases_the_chain_on_a_header_bump.
        let bump = signed_snapshot(&key, 2, frame_digest(&first), entry_payload_with_version(2));
        let accepted = receiver.accept(&bump).unwrap();
        assert_eq!(accepted.kind, ReplicationKind::Snapshot);
        assert_eq!(receiver.header_version(), 2);
        let cursor_after_bump = receiver.cursor();

        // A snapshot on the OLD header version (correct session nonce,
        // correct frame chain, signed by the pinned source) must be
        // refused: header versions never move backwards.
        let downgrade = signed_snapshot(&key, 1, frame_digest(&bump), entry_payload());
        assert_eq!(
            receiver.accept(&downgrade),
            Err(ReplicationError::HeaderVersionChanged)
        );
        // The refused frame leaves verified state untouched.
        assert_eq!(receiver.header_version(), 2);
        assert_eq!(receiver.cursor(), cursor_after_bump);

        // A v1 tail is still refused after the downgrade attempt (the
        // frame chain still names the accepted v2 snapshot, since the
        // refused frame did not advance it).
        let stale_tail = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            receiver.cursor(),
            frame_digest(&bump),
            entry_payload(),
        )
        .unwrap();
        let stale_bytes = sign_bypass(stale_tail, &key);
        assert_eq!(
            receiver.accept(&stale_bytes),
            Err(ReplicationError::HeaderVersionChanged)
        );

        // And the chain still advances normally on v2 afterwards (a
        // tail away from the chain base carries no Created header).
        let v2_payload: Vec<u8> = [
            JournalEntry::CapsGranted {
                granted: vec!["chat".to_string()],
            },
            JournalEntry::CapsCeiling {
                ceiling: vec!["chat".to_string(), "input".to_string()],
            },
        ]
        .iter()
        .flat_map(journal_entry_line)
        .collect();
        let v2_tail = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            2,
            NONCE,
            receiver.cursor(),
            frame_digest(&bump),
            v2_payload,
        )
        .unwrap();
        let v2_bytes = SignedReplicationEnvelope::sign(v2_tail, &key)
            .unwrap()
            .encode_frame()
            .unwrap();
        let accepted = receiver.accept(&v2_bytes).unwrap();
        assert_eq!(accepted.kind, ReplicationKind::Tail);
        assert_eq!(receiver.header_version(), 2);
        assert_eq!(accepted.cursor_after.ordinal, 5);
    }

    /// A signed snapshot frame over `payload` on the version-`version`
    /// chain, with frame-chain digest `prev`.
    fn signed_snapshot(
        key: &SigningKey,
        version: u32,
        prev: [u8; 32],
        payload: Vec<u8>,
    ) -> Vec<u8> {
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source_of(key), version, NONCE, prev, payload)
                .unwrap();
        SignedReplicationEnvelope::sign(envelope, key)
            .unwrap()
            .encode_frame()
            .unwrap()
    }

    #[test]
    fn snapshot_and_tail_payloads_must_carry_the_journals_own_header() {
        // Regression tests for #4486: accept verified payloads whose
        // journal WorldCore::restore refuses, and the verified cursor
        // advanced over a journal no replica could ever restore.
        let key = signing_key(1);
        let base = chain_base_cursor(WORLD, &source_of(&key), 1);
        let good = entry_payload();
        let first_line_end = good.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        let mut receiver = tracker(&key);

        // Missing header: a snapshot whose payload starts mid-journal.
        let frame = signed_snapshot_bypass(&key, 1, [0; 32], good[first_line_end..].to_vec());
        assert_eq!(
            receiver.accept(&frame),
            Err(ReplicationError::InvalidJournalHeader)
        );
        assert_eq!(receiver.cursor(), base);

        // Foreign-world header: the payload's Created names another world
        // than the envelope (and the receiver).
        let foreign = [
            JournalEntry::Created {
                world_id: "other-world".to_string(),
                version: 1,
            },
            JournalEntry::CapsGranted {
                granted: vec!["chat".to_string()],
            },
        ];
        let foreign_payload: Vec<u8> = foreign.iter().flat_map(journal_entry_line).collect();
        let frame = signed_snapshot_bypass(&key, 1, [0; 32], foreign_payload);
        assert_eq!(
            receiver.accept(&frame),
            Err(ReplicationError::InvalidJournalHeader)
        );
        assert_eq!(receiver.cursor(), base);

        // Payload version mismatch: the header's version is not the
        // envelope's header_version.
        let frame = signed_snapshot_bypass(&key, 1, [0; 32], entry_payload_with_version(2));
        assert_eq!(
            receiver.accept(&frame),
            Err(ReplicationError::InvalidJournalHeader)
        );
        assert_eq!(receiver.cursor(), base);

        // Duplicate header: a second Created later in the snapshot.
        let mut duplicate = good.clone();
        duplicate.extend_from_slice(&journal_entry_line(&JournalEntry::Created {
            world_id: WORLD.to_string(),
            version: 1,
        }));
        let frame = signed_snapshot_bypass(&key, 1, [0; 32], duplicate);
        assert_eq!(
            receiver.accept(&frame),
            Err(ReplicationError::InvalidJournalHeader)
        );
        assert_eq!(receiver.cursor(), base);

        // Created in a tail: after a valid snapshot, a tail frame carrying
        // a header entry is refused and the verified cursor does not move.
        let snapshot = signed_snapshot(&key, 1, [0; 32], good.clone());
        receiver.accept(&snapshot).unwrap();
        let reached = receiver.cursor();
        let lying_tail = ReplicationEnvelope::tail(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            reached,
            frame_digest(&snapshot),
            good.clone(),
        )
        .unwrap();
        let frame = sign_bypass(lying_tail, &key);
        assert_eq!(
            receiver.accept(&frame),
            Err(ReplicationError::InvalidJournalHeader)
        );
        assert_eq!(receiver.cursor(), reached);
    }

    #[test]
    fn trailing_bytes_and_undecodable_entries_are_refused() {
        // Regression tests for the Codex review findings on PR #4447.
        let key = signing_key(1);
        let base = chain_base_cursor(WORLD, &source_of(&key), 1);
        let frame = signed_tail(&key, base, [0; 32], entry_payload());
        // Bytes appended to an otherwise valid frame refuse it outright:
        // the envelope decode must not ignore a trailing suffix.
        let mut extended = frame.clone();
        extended.extend_from_slice(b"unsigned suffix");
        assert!(SignedReplicationEnvelope::decode_frame(&extended).is_none());
        let mut receiver = tracker(&key);
        assert_eq!(
            receiver.accept(&extended),
            Err(ReplicationError::MalformedFrame)
        );
        assert_eq!(receiver.cursor(), base);
        // A payload whose lines do not decode as journal entries is
        // refused before the verified cursor advances, even when the
        // pinned source signed it.
        let mut bogus =
            ReplicationEnvelope::terminal(WORLD, source_of(&key), 1, NONCE, base, [0; 32]);
        bogus.kind = ReplicationKind::Tail;
        bogus.payload = b"not-json\n".to_vec();
        bogus.payload_digest = payload_digest(&bogus.payload);
        bogus.cursor_after = ReplicationCursor {
            ordinal: base.ordinal + 1,
            digest: [5; 32],
        };
        let bogus_bytes = sign_bypass(bogus, &key);
        assert_eq!(
            receiver.accept(&bogus_bytes),
            Err(ReplicationError::MalformedPayload)
        );
        assert_eq!(receiver.cursor(), base);
    }

    #[test]
    fn sign_refuses_envelopes_every_receiver_would_reject() {
        // #6469: each envelope-local rule `accept` enforces is pinned
        // at the signer — `sign` returns None for the shape, while a
        // valid constructor envelope still signs and is accepted.
        let key = signing_key(1);
        let source = source_of(&key);
        let base = chain_base_cursor(WORLD, &source, JOURNAL_VERSION);
        let payload = entry_payload();
        let valid_tail = || {
            ReplicationEnvelope::tail(
                WORLD,
                source,
                JOURNAL_VERSION,
                NONCE,
                base,
                [0; 32],
                payload.clone(),
            )
            .unwrap()
        };
        // Control: the unmutated tail signs Some and a fresh tracker
        // accepts it.
        let control = SignedReplicationEnvelope::sign(valid_tail(), &key).unwrap();
        let mut receiver = tracker(&key);
        assert!(receiver.accept(&control.encode_frame().unwrap()).is_ok());

        // 1. Envelope version from the future.
        let mut env = valid_tail();
        env.version = 99;
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        // 2. Journal header version outside the supported range.
        for bad_version in [0, MAX_JOURNAL_VERSION + 1] {
            let mut env = valid_tail();
            env.header_version = bad_version;
            assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        }
        // 3. Payload digest that does not match the payload.
        let mut env = valid_tail();
        env.payload_digest = payload_digest(b"not the payload");
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        // 4. Payload that does not chain to cursor_after.
        let mut env = valid_tail();
        env.cursor_after = ReplicationCursor {
            ordinal: env.cursor_after.ordinal + 1,
            digest: [7; 32],
        };
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        // 5. Malformed payload (lines that do not decode as entries).
        let mut env = valid_tail();
        env.payload = b"not-json\n".to_vec();
        env.payload_digest = payload_digest(&env.payload);
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        // 6. Terminal carrying a payload; terminal with unequal cursors.
        let mut env =
            ReplicationEnvelope::terminal(WORLD, source, JOURNAL_VERSION, NONCE, base, [0; 32]);
        env.payload = payload.clone();
        env.payload_digest = payload_digest(&env.payload);
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        let mut env =
            ReplicationEnvelope::terminal(WORLD, source, JOURNAL_VERSION, NONCE, base, [0; 32]);
        env.cursor_after = advance_cursor(base, b"x\n");
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        // 7. Snapshot whose before-cursor is not the chain base (the
        //    payload is re-chained from the wrong base, so only the
        //    base rule can refuse it).
        let mut env = ReplicationEnvelope::snapshot(
            WORLD,
            source,
            JOURNAL_VERSION,
            NONCE,
            [0; 32],
            payload.clone(),
        )
        .unwrap();
        let wrong_base = advance_cursor(base, b"seed\n");
        env.cursor_before = wrong_base;
        env.cursor_after = chain_payload(wrong_base, &payload).unwrap();
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
        // 8. Payload journal header disagreeing with the envelope:
        //    a snapshot starting mid-journal (missing Created header),
        //    built by the snapshot constructor itself.
        let first_line_end = payload.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        let env = ReplicationEnvelope::snapshot(
            WORLD,
            source,
            JOURNAL_VERSION,
            NONCE,
            [0; 32],
            payload[first_line_end..].to_vec(),
        )
        .unwrap();
        assert!(SignedReplicationEnvelope::sign(env, &key).is_none());
    }
}
