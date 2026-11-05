//! F12.13 journal replication: durable receiver and verifier (#806).
//!
//! Phase 3 of the phased plan in `doc/2026-11-01_design_id806.md`. Phase 1
//! ([`super::replication`]) defined the signed envelope and the in-memory
//! verification state machine; phase 2 serves a snapshot-plus-subscribe
//! feed where every batch — the initial snapshot, each appended entry, and
//! each reset snapshot — is closed by a signed terminal frame. This module
//! is the receiver half that makes verified batches durable:
//!
//! - **Pinned-source verification.** Every frame goes through
//!   [`ReplicationTracker`], which is built with the receiver's pinned
//!   source; a frame whose signer is not the pinned source is refused
//!   before its payload is ever staged.
//! - **Staged transfers.** Accepted snapshot/tail payloads are staged in
//!   memory only. Nothing becomes durable until the batch's terminal frame
//!   verifies (design §4.4/§4.5): a transfer that is truncated, refused, or
//!   abandoned on disconnect leaves the last committed replica untouched.
//! - **Atomic snapshot install, append-only tails.** A committed snapshot
//!   is written to a scratch file, fsynced, and renamed over the replica
//!   journal, so a crash mid-install never leaves a torn journal. A
//!   committed tail batch is appended to the journal and fsynced; batches
//!   are never rewritten in place.
//! - **Restart and reconnect.** [`DurableReceiver::open`] re-reads the
//!   committed journal, refuses it if it is not a well-formed chain for
//!   this world under the pinned source, and recomputes the verified
//!   cursor from the entry bytes alone, so a restarted receiver knows
//!   exactly where it stands. A new session always opens with a snapshot
//!   (the phase-2 feed contract), which rebases the tracker; staged bytes
//!   from a dead session are discarded, never committed.
//!
//! Tail appends are transactional: before appending, the receiver
//! durably records the journal's pre-append length in an append-pending
//! record (itself installed by rename, so it is never torn); the record
//! is removed only after the appended bytes are fsynced. At open, a
//! leftover record rolls the journal back to the recorded length, so a
//! crash or a partial write mid-append can neither wedge the replica
//! (an unterminated final line) nor silently promote an uncommitted
//! batch (a partial append that happens to end at an entry boundary).
//!
//! The replica journal is the exact `JournalEntry` JSONL the writer
//! stores, at `<dir>/journal.jsonl`. Serving spectators from it is phase
//! 4; this module only receives, verifies, and persists.
//!
//! One scope note: the v1 frame contract carries journal bytes only —
//! there is no module-bytes frame kind. Module hashes named by journal
//! entries (e.g. `BuilderInstalled`) are covered by the chain like every
//! other entry byte; transferring the module bytes themselves is not
//! part of the phase-1/2 contract this receiver verifies against.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::replication::{
    ReplicationCursor, ReplicationError, ReplicationKind, ReplicationTracker, SESSION_NONCE_BYTES,
    SignedReplicationEnvelope, advance_cursor, chain_base_cursor,
};
use super::{JOURNAL_VERSION, JournalEntry, MAX_JOURNAL_VERSION};

/// File name of the committed replica journal inside the receiver's
/// directory: the exact JSONL bytes the writer's journal stores.
pub const REPLICA_JOURNAL_FILE: &str = "journal.jsonl";

/// Scratch file a snapshot is staged on disk under while it is being
/// installed; renamed over [`REPLICA_JOURNAL_FILE`] once fully written
/// and fsynced. A leftover at open time is crash debris and is removed.
const INSTALLING_FILE: &str = "journal.jsonl.installing";

/// Transaction record for an in-flight tail append: the journal's byte
/// length before the append began. Installed by rename from
/// [`APPEND_PENDING_TMP_FILE`] so its contents are never torn; removed
/// once the append is fsynced. A leftover at open rolls the journal
/// back to the recorded length.
const APPEND_PENDING_FILE: &str = "journal.jsonl.append-pending";

/// Scratch file the append-pending record is written through.
const APPEND_PENDING_TMP_FILE: &str = "journal.jsonl.append-pending.tmp";

/// Upper bound on one uncommitted transfer's staged bytes. Per-frame
/// payloads are already bounded by
/// [`super::replication::MAX_REPLICATION_PAYLOAD_BYTES`]; this bounds
/// the batch as a whole so a source that streams chunks forever without
/// a terminal frame cannot grow the receiver's memory without limit.
pub const MAX_STAGED_TRANSFER_BYTES: usize = 64 * 1024 * 1024;

/// Why a receiver operation failed. A failure at any point leaves the
/// last committed replica state intact; a failure that desynchronises
/// the staging area from the verification state (an I/O error during
/// commit, an over-bound staged transfer) also poisons the session, so
/// the next frame is refused with [`ReceiverError::NoSession`] until
/// [`DurableReceiver::begin_session`] opens a fresh one.
#[derive(Debug, thiserror::Error)]
pub enum ReceiverError {
    /// A frame was refused by the verification state machine.
    #[error(transparent)]
    Replication(#[from] ReplicationError),
    /// Replica storage failed.
    #[error("replica storage: {0}")]
    Io(#[from] std::io::Error),
    /// Frames were offered with no live session (never begun, or
    /// poisoned by an earlier commit failure).
    #[error("no replication session is active")]
    NoSession,
    /// The durable journal on disk is not a well-formed chain for this
    /// world: it does not decode, does not start with the world's
    /// `Created` header at a supported version, or carries a second
    /// header. Refused at open, before anything is served or appended.
    #[error("durable replica journal is corrupt: {0}")]
    CorruptReplica(String),
    /// One uncommitted transfer staged more than
    /// [`MAX_STAGED_TRANSFER_BYTES`]; the session is poisoned.
    #[error("staged transfer exceeds the {MAX_STAGED_TRANSFER_BYTES}-byte bound")]
    StagedTransferTooLarge,
    /// A session's first frame was not a snapshot. The phase-2 feed
    /// opens every session with a snapshot, which is what rebases the
    /// verification tracker to the chain base; a tail or terminal first
    /// would be verified against the base but applied to the existing
    /// journal, so the receiver refuses it before the tracker sees it.
    #[error("replication session does not start with a snapshot frame")]
    SessionMustStartWithSnapshot,
}

/// What one accepted frame did to the receiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverProgress {
    /// The frame verified and its payload is staged; the batch is not
    /// durable until its terminal frame commits it.
    Staged {
        /// What the frame carried.
        kind: ReplicationKind,
        /// The verified chain position after this frame.
        cursor_after: ReplicationCursor,
    },
    /// The batch's terminal frame verified and the staged bytes are now
    /// durable at this cursor.
    Committed {
        /// The verified chain position the replica now stands at.
        cursor: ReplicationCursor,
    },
}

/// One uncommitted batch's staged payload.
#[derive(Debug)]
enum Staged {
    /// A reset snapshot (possibly still arriving in chunks): on commit
    /// it replaces the replica journal wholesale.
    Snapshot(Vec<u8>),
    /// Tail batches continuing the committed journal: on commit they are
    /// appended to it.
    Tails(Vec<u8>),
}

impl Staged {
    fn bytes(&self) -> &[u8] {
        match self {
            Staged::Snapshot(bytes) | Staged::Tails(bytes) => bytes,
        }
    }

    fn extend(&mut self, payload: &[u8]) {
        match self {
            Staged::Snapshot(bytes) | Staged::Tails(bytes) => bytes.extend_from_slice(payload),
        }
    }
}

/// A durable replica of one world's journal from one pinned source.
///
/// Owns a directory holding the committed journal; see the module docs
/// for the staging and commit rules. Not synchronised: one receiver is
/// driven by one sync session at a time.
#[derive(Debug)]
pub struct DurableReceiver {
    dir: PathBuf,
    world_id: String,
    pinned_source: [u8; 32],
    committed_cursor: Option<ReplicationCursor>,
    header_version: Option<u32>,
    session: Option<ReplicationTracker>,
    session_started: bool,
    staged: Option<Staged>,
}

impl DurableReceiver {
    /// Open (creating if needed) the replica directory and validate any
    /// committed journal already there, recomputing the verified cursor
    /// from its exact bytes under the pinned source.
    ///
    /// # Errors
    ///
    /// [`ReceiverError::Io`] if the directory or journal cannot be read;
    /// [`ReceiverError::CorruptReplica`] if a committed journal exists
    /// but is not a well-formed chain for `world_id` at a supported
    /// header version.
    pub fn open(
        dir: &Path,
        world_id: &str,
        pinned_source: [u8; 32],
    ) -> Result<Self, ReceiverError> {
        fs::create_dir_all(dir)?;
        roll_back_interrupted_append(dir)?;
        // A leftover installing file is the debris of a crash mid-commit:
        // the rename never happened, so the committed journal (if any) is
        // the intact pre-commit state and the scratch is safe to drop.
        let installing = dir.join(INSTALLING_FILE);
        if installing.exists() {
            fs::remove_file(&installing)?;
        }
        let journal_path = dir.join(REPLICA_JOURNAL_FILE);
        let (committed_cursor, header_version) = if journal_path.exists() {
            let bytes = fs::read(&journal_path)?;
            let (cursor, version) = validate_committed_journal(&bytes, world_id, &pinned_source)?;
            (Some(cursor), Some(version))
        } else {
            (None, None)
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            world_id: world_id.to_string(),
            pinned_source,
            committed_cursor,
            header_version,
            session: None,
            session_started: false,
            staged: None,
        })
    }

    /// The verified chain position of the committed replica, or `None`
    /// before the first batch commits.
    #[must_use]
    pub fn committed_cursor(&self) -> Option<ReplicationCursor> {
        self.committed_cursor
    }

    /// The journal header version of the committed replica, or `None`
    /// before the first batch commits.
    #[must_use]
    pub fn header_version(&self) -> Option<u32> {
        self.header_version
    }

    /// Path of the committed replica journal.
    #[must_use]
    pub fn journal_path(&self) -> PathBuf {
        self.dir.join(REPLICA_JOURNAL_FILE)
    }

    /// The committed journal's exact bytes (empty when nothing has
    /// committed yet).
    ///
    /// # Errors
    ///
    /// [`ReceiverError::Io`] if the journal cannot be read.
    pub fn read_journal(&self) -> Result<Vec<u8>, ReceiverError> {
        let path = self.journal_path();
        if path.exists() {
            Ok(fs::read(path)?)
        } else {
            Ok(Vec::new())
        }
    }

    /// Whether a sync session is live.
    #[must_use]
    pub fn session_active(&self) -> bool {
        self.session.is_some()
    }

    /// Begin a sync session with the client's fresh nonce. Any staged
    /// bytes from a previous session are discarded: an unterminated
    /// transfer never commits. The phase-2 feed opens every session
    /// with a snapshot, which rebases the verification tracker, so a
    /// reconnect against an already-committed replica is safe — the
    /// snapshot either matches the committed journal (and recommits
    /// identical bytes) or replaces it wholesale.
    pub fn begin_session(&mut self, session_nonce: [u8; SESSION_NONCE_BYTES]) {
        // A stored header version outside the supported range cannot be
        // repaired by a fresh session: leave the session closed so every
        // frame is refused with `NoSession` instead of pinning a tracker
        // whose chain base no frame can continue.
        self.session = ReplicationTracker::new(
            &self.world_id,
            self.pinned_source,
            self.header_version.unwrap_or(JOURNAL_VERSION),
            session_nonce,
        )
        .ok();
        self.session_started = false;
        self.staged = None;
    }

    /// Abandon the live session (disconnect): staged bytes are dropped
    /// and the committed replica is left exactly as it was.
    pub fn abort_session(&mut self) {
        self.session = None;
        self.session_started = false;
        self.staged = None;
    }

    /// Verify one encoded frame and stage or commit its payload.
    ///
    /// # Errors
    ///
    /// [`ReceiverError::NoSession`] with no live session;
    /// [`ReceiverError::Replication`] when the frame fails verification
    /// (the session and the committed replica are both untouched, so the
    /// caller may continue or abandon the session);
    /// [`ReceiverError::StagedTransferTooLarge`] or
    /// [`ReceiverError::Io`] when staging or committing fails — both
    /// poison the session, because the staging area may no longer match
    /// the verification state.
    pub fn accept_frame(&mut self, frame_bytes: &[u8]) -> Result<ReceiverProgress, ReceiverError> {
        if self.session.is_none() {
            return Err(ReceiverError::NoSession);
        }
        // Snapshot-first ordering, checked before the tracker sees the
        // frame so a refusal leaves the session state untouched: a new
        // session's tracker sits at the chain base, so a base-chained
        // tail would verify there but must never be appended to an
        // existing journal, and a first terminal must never move the
        // committed cursor without moving the journal.
        if !self.session_started {
            if let Some(signed) = SignedReplicationEnvelope::decode_frame(frame_bytes) {
                if signed.envelope.kind != ReplicationKind::Snapshot {
                    return Err(ReceiverError::SessionMustStartWithSnapshot);
                }
            }
        }
        let (accepted, session_cursor, session_version) = {
            let tracker = self.session.as_mut().ok_or(ReceiverError::NoSession)?;
            let accepted = tracker.accept(frame_bytes)?;
            (accepted, tracker.cursor(), tracker.header_version())
        };
        self.session_started = true;
        match accepted.kind {
            ReplicationKind::Snapshot => {
                self.staged = Some(Staged::Snapshot(accepted.payload));
                self.check_staged_bound()?;
                Ok(ReceiverProgress::Staged {
                    kind: accepted.kind,
                    cursor_after: accepted.cursor_after,
                })
            }
            ReplicationKind::Tail => {
                match self.staged.as_mut() {
                    Some(staged) => staged.extend(&accepted.payload),
                    None => self.staged = Some(Staged::Tails(accepted.payload)),
                }
                self.check_staged_bound()?;
                Ok(ReceiverProgress::Staged {
                    kind: accepted.kind,
                    cursor_after: accepted.cursor_after,
                })
            }
            ReplicationKind::Terminal => {
                if let Err(error) = self.commit_staged() {
                    self.session = None;
                    self.session_started = false;
                    self.staged = None;
                    return Err(error);
                }
                self.committed_cursor = Some(session_cursor);
                self.header_version = Some(session_version);
                Ok(ReceiverProgress::Committed {
                    cursor: session_cursor,
                })
            }
        }
    }

    fn check_staged_bound(&mut self) -> Result<(), ReceiverError> {
        let len = self
            .staged
            .as_ref()
            .map_or(0, |staged| staged.bytes().len());
        if len > MAX_STAGED_TRANSFER_BYTES {
            self.session = None;
            self.session_started = false;
            self.staged = None;
            return Err(ReceiverError::StagedTransferTooLarge);
        }
        Ok(())
    }

    /// Make the staged batch durable. Snapshot batches install
    /// atomically (scratch file + fsync + rename + directory fsync);
    /// tail batches append and fsync.
    fn commit_staged(&mut self) -> Result<(), ReceiverError> {
        let Some(staged) = self.staged.take() else {
            return Ok(());
        };
        match staged {
            Staged::Snapshot(bytes) => {
                // Recover any interrupted tail append before the
                // replacement snapshot commits: a stale append-pending
                // record names a length in the OLD journal, and if it
                // survived to the next open it would truncate the
                // snapshot that replaces that journal. Recovery rolls
                // the old journal back (immaterial here, since the
                // snapshot replaces it wholesale) and clears the record.
                roll_back_interrupted_append(&self.dir)?;
                let installing = self.dir.join(INSTALLING_FILE);
                {
                    let mut file = File::create(&installing)?;
                    file.write_all(&bytes)?;
                    file.sync_all()?;
                }
                fs::rename(&installing, self.journal_path())?;
                self.sync_dir()?;
            }
            Staged::Tails(bytes) => {
                // Transactional append: record the pre-append length
                // durably first, so an interrupted or partial append
                // is rolled back at the next open instead of wedging
                // or silently promoting the batch.
                let journal = self.journal_path();
                let prior_len = if journal.exists() {
                    fs::metadata(&journal)?.len()
                } else {
                    0
                };
                self.write_append_pending(prior_len)?;
                let append_result = (|| -> Result<(), ReceiverError> {
                    let mut file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&journal)?;
                    file.write_all(&bytes)?;
                    file.sync_all()?;
                    Ok(())
                })();
                if let Err(error) = append_result {
                    // Best-effort immediate rollback. When it succeeds
                    // the transaction is fully recovered, so clear the
                    // pending record too: a stale record names a length
                    // in this journal and would roll back a later
                    // replacement snapshot at the next open. When the
                    // rollback fails, leave the record — the next open
                    // (or a snapshot commit's recovery) rolls back
                    // regardless.
                    if self.roll_back_to(prior_len).is_ok() {
                        let _ = fs::remove_file(self.dir.join(APPEND_PENDING_FILE));
                        let _ = self.sync_dir();
                    }
                    return Err(error);
                }
                fs::remove_file(self.dir.join(APPEND_PENDING_FILE))?;
                self.sync_dir()?;
            }
        }
        Ok(())
    }

    fn sync_dir(&self) -> Result<(), ReceiverError> {
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    /// Durably record the journal's pre-append length: written through
    /// a scratch file and renamed into place, so the record itself is
    /// never torn.
    fn write_append_pending(&self, prior_len: u64) -> Result<(), ReceiverError> {
        let tmp = self.dir.join(APPEND_PENDING_TMP_FILE);
        {
            let mut file = File::create(&tmp)?;
            file.write_all(prior_len.to_string().as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, self.dir.join(APPEND_PENDING_FILE))?;
        self.sync_dir()?;
        Ok(())
    }

    /// Truncate the journal back to `len` bytes and fsync it.
    fn roll_back_to(&self, len: u64) -> Result<(), ReceiverError> {
        let journal = self.journal_path();
        if journal.exists() {
            let file = OpenOptions::new().write(true).open(&journal)?;
            file.set_len(len)?;
            file.sync_all()?;
        }
        Ok(())
    }
}

/// If a previous tail append was interrupted (its append-pending record
/// is still present), roll the journal back to the recorded pre-append
/// length and clear the record. Runs at open, before the journal is
/// validated: the rollback restores exactly the last committed batch
/// boundary, whether the partial append stopped mid-line or at an
/// entry boundary.
fn roll_back_interrupted_append(dir: &Path) -> Result<(), ReceiverError> {
    let pending = dir.join(APPEND_PENDING_FILE);
    let tmp = dir.join(APPEND_PENDING_TMP_FILE);
    if tmp.exists() {
        fs::remove_file(&tmp)?;
    }
    if !pending.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&pending)?;
    let prior_len: u64 = text.trim().parse().map_err(|_| {
        ReceiverError::CorruptReplica("append-pending record is not a byte length".to_string())
    })?;
    let journal = dir.join(REPLICA_JOURNAL_FILE);
    if journal.exists() {
        let file = OpenOptions::new().write(true).open(&journal)?;
        file.set_len(prior_len)?;
        file.sync_all()?;
    }
    fs::remove_file(&pending)?;
    File::open(dir)?.sync_all()?;
    Ok(())
}

/// Recompute the verified cursor of a committed journal from its exact
/// bytes, refusing anything the restore path could not read back: the
/// bytes must be complete entry lines, the first must be this world's
/// `Created` header at a supported version, and no second header may
/// appear.
fn validate_committed_journal(
    bytes: &[u8],
    world_id: &str,
    pinned_source: &[u8; 32],
) -> Result<(ReplicationCursor, u32), ReceiverError> {
    let corrupt = |reason: &str| ReceiverError::CorruptReplica(reason.to_string());
    if bytes.is_empty() {
        return Err(corrupt("journal file is empty"));
    }
    if !bytes.ends_with(b"\n") {
        return Err(corrupt("journal does not end at an entry boundary"));
    }
    let mut header_version = None;
    let mut cursor = None;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let entry: JournalEntry = serde_json::from_slice(line)
            .map_err(|_| corrupt("a journal line does not decode as a journal entry"))?;
        match (header_version, &entry) {
            (
                None,
                JournalEntry::Created {
                    world_id: created_world,
                    version,
                },
            ) => {
                if created_world != world_id {
                    return Err(corrupt("journal header names a different world"));
                }
                if *version == 0 || *version > MAX_JOURNAL_VERSION {
                    return Err(corrupt("journal header version is not supported"));
                }
                header_version = Some(*version);
                cursor = Some(chain_base_cursor(world_id, pinned_source, *version));
            }
            (None, _) => return Err(corrupt("journal does not start with its Created header")),
            (Some(_), JournalEntry::Created { .. }) => {
                return Err(corrupt("journal carries a second Created header"));
            }
            (Some(_), _) => {}
        }
        cursor = Some(advance_cursor(
            cursor.ok_or_else(|| corrupt("journal header missing"))?,
            line,
        ));
    }
    match (cursor, header_version) {
        (Some(cursor), Some(version)) => Ok((cursor, version)),
        _ => Err(corrupt("journal header missing")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::replication::{
        ReplicationEnvelope, SignedReplicationEnvelope, chain_payload, frame_digest,
        journal_entry_line, split_entry_payload,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::TempDir;

    const WORLD: &str = "world-receiver-test";
    const NONCE: [u8; SESSION_NONCE_BYTES] = [7; SESSION_NONCE_BYTES];
    const NONCE2: [u8; SESSION_NONCE_BYTES] = [9; SESSION_NONCE_BYTES];

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn source_of(key: &SigningKey) -> [u8; 32] {
        *key.verifying_key().as_bytes()
    }

    /// Journal bytes: a `Created` header at `version` plus two entries.
    fn journal_payload(version: u32) -> Vec<u8> {
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

    fn extra_entry() -> Vec<u8> {
        journal_entry_line(&JournalEntry::CapsGranted {
            granted: vec!["input".to_string()],
        })
    }

    fn sign(envelope: ReplicationEnvelope, key: &SigningKey) -> Vec<u8> {
        SignedReplicationEnvelope::sign(envelope, key)
            .unwrap()
            .encode_frame()
            .unwrap()
    }

    /// Sign an envelope the production signer refuses to sign (an
    /// over-bound payload), replicating the signing-bytes contract so
    /// the frame still passes signature verification and reaches the
    /// check under test. If the contract changes, this helper's frames
    /// fail verification and the tests say so loudly.
    fn sign_unbounded(envelope: ReplicationEnvelope, key: &SigningKey) -> Vec<u8> {
        use crate::world::replication::REPLICATION_DOMAIN;
        let signing_bytes = postcard::to_allocvec(&(
            REPLICATION_DOMAIN,
            envelope.version,
            &envelope.world_id,
            &envelope.source,
            envelope.header_version,
            envelope.kind,
            &envelope.session_nonce,
            envelope.cursor_before,
            envelope.cursor_after,
            &envelope.prev_frame_digest,
            &envelope.payload_digest,
            &envelope.payload,
        ))
        .unwrap();
        let signature = key.sign(&signing_bytes).to_bytes().to_vec();
        SignedReplicationEnvelope {
            envelope,
            signature,
        }
        .encode_frame()
        .unwrap()
    }

    fn receiver(dir: &TempDir, key: &SigningKey) -> DurableReceiver {
        DurableReceiver::open(dir.path(), WORLD, source_of(key)).unwrap()
    }

    /// Reopen a replica directory from a test that already holds a
    /// binding named `receiver` (which shadows the `receiver` helper).
    fn reopen(dir: &TempDir, key: &SigningKey) -> DurableReceiver {
        receiver(dir, key)
    }

    /// Drive one full snapshot batch the way the phase-2 feed sends it:
    /// first chunk as a snapshot frame, the rest as tail frames, then a
    /// terminal frame. Returns the terminal frame's progress.
    fn feed_snapshot(
        receiver: &mut DurableReceiver,
        key: &SigningKey,
        nonce: [u8; SESSION_NONCE_BYTES],
        version: u32,
        payload: &[u8],
    ) -> ReceiverProgress {
        let source = source_of(key);
        let chunks = split_entry_payload(payload).unwrap();
        let mut cursor = chain_base_cursor(WORLD, &source, version);
        let mut prev = [0; 32];
        let mut last = None;
        for (index, chunk) in chunks.into_iter().enumerate() {
            let envelope = if index == 0 {
                ReplicationEnvelope::snapshot(WORLD, source, version, nonce, prev, chunk).unwrap()
            } else {
                ReplicationEnvelope::tail(WORLD, source, version, nonce, cursor, prev, chunk)
                    .unwrap()
            };
            cursor = envelope.cursor_after;
            let frame = sign(envelope, key);
            prev = frame_digest(&frame);
            last = Some(receiver.accept_frame(&frame).unwrap());
        }
        let terminal = ReplicationEnvelope::terminal(WORLD, source, version, nonce, cursor, prev);
        let progress = receiver.accept_frame(&sign(terminal, key)).unwrap();
        assert!(matches!(last, Some(ReceiverProgress::Staged { .. })));
        progress
    }

    #[test]
    fn snapshot_commits_only_on_the_terminal_frame() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        // Feed the snapshot chunks but withhold the terminal frame.
        let payload = journal_payload(1);
        let source = source_of(&key);
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source, 1, NONCE, [0; 32], payload.clone())
                .unwrap();
        let progress = receiver.accept_frame(&sign(envelope, &key)).unwrap();
        assert!(matches!(progress, ReceiverProgress::Staged { .. }));
        assert_eq!(receiver.committed_cursor(), None);
        assert_eq!(receiver.read_journal().unwrap(), Vec::<u8>::new());
        // A truncated transfer that is abandoned changes nothing.
        receiver.abort_session();
        assert_eq!(receiver.read_journal().unwrap(), Vec::<u8>::new());
        // The full batch, terminal included, commits the exact bytes.
        receiver.begin_session(NONCE);
        let progress = feed_snapshot(&mut receiver, &key, NONCE, 1, &payload);
        let expected = chain_payload(chain_base_cursor(WORLD, &source, 1), &payload).unwrap();
        assert_eq!(progress, ReceiverProgress::Committed { cursor: expected });
        assert_eq!(receiver.committed_cursor(), Some(expected));
        assert_eq!(receiver.header_version(), Some(1));
        assert_eq!(receiver.read_journal().unwrap(), payload);
    }

    #[test]
    fn tail_batch_commits_atomically_after_terminal() {
        // Drive a session manually so the frame digests are known: one
        // snapshot chunk, its terminal, then one tail + terminal in the
        // same session (the phase-2 live-append pattern).
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let payload = journal_payload(1);
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source, 1, NONCE, [0; 32], payload.clone())
                .unwrap();
        let after_snapshot = envelope.cursor_after;
        let frame = sign(envelope, &key);
        let digest = frame_digest(&frame);
        receiver.accept_frame(&frame).unwrap();
        let terminal =
            ReplicationEnvelope::terminal(WORLD, source, 1, NONCE, after_snapshot, digest);
        let frame = sign(terminal, &key);
        let mut prev = frame_digest(&frame);
        assert_eq!(
            receiver.accept_frame(&frame).unwrap(),
            ReceiverProgress::Committed {
                cursor: after_snapshot
            }
        );
        // Withhold a tail batch's terminal: nothing is appended.
        let extra = extra_entry();
        let envelope =
            ReplicationEnvelope::tail(WORLD, source, 1, NONCE, after_snapshot, prev, extra.clone())
                .unwrap();
        let after_tail = envelope.cursor_after;
        let frame = sign(envelope, &key);
        prev = frame_digest(&frame);
        receiver.accept_frame(&frame).unwrap();
        assert_eq!(receiver.read_journal().unwrap(), payload);
        // Its terminal appends the exact bytes.
        let terminal = ReplicationEnvelope::terminal(WORLD, source, 1, NONCE, after_tail, prev);
        receiver.accept_frame(&sign(terminal, &key)).unwrap();
        let mut expected = payload.clone();
        expected.extend_from_slice(&extra);
        assert_eq!(receiver.read_journal().unwrap(), expected);
        assert_eq!(receiver.committed_cursor(), Some(after_tail));
    }

    #[test]
    fn tampered_payload_is_refused_and_the_committed_replica_is_intact() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let payload = journal_payload(1);
        feed_snapshot(&mut receiver, &key, NONCE, 1, &payload);
        // A snapshot whose payload was mutated after the envelope (and
        // its payload digest) was built: the digest check refuses it.
        receiver.begin_session(NONCE2);
        let source = source_of(&key);
        let mut envelope =
            ReplicationEnvelope::snapshot(WORLD, source, 1, NONCE2, [0; 32], payload.clone())
                .unwrap();
        envelope.payload[10] ^= 0x01;
        let error = receiver.accept_frame(&sign(envelope, &key)).unwrap_err();
        assert_eq!(
            error.to_string(),
            ReplicationError::PayloadDigestMismatch.to_string()
        );
        receiver.abort_session();
        assert_eq!(receiver.read_journal().unwrap(), payload);
    }

    #[test]
    fn untrusted_source_is_refused() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let stranger = signing_key(2);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let envelope = ReplicationEnvelope::snapshot(
            WORLD,
            source_of(&stranger),
            1,
            NONCE,
            [0; 32],
            journal_payload(1),
        )
        .unwrap();
        let error = receiver
            .accept_frame(&sign(envelope, &stranger))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            ReplicationError::UntrustedSource.to_string()
        );
        assert_eq!(receiver.committed_cursor(), None);
    }

    #[test]
    fn gap_and_fork_tails_are_refused() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let payload = journal_payload(1);
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source, 1, NONCE, [0; 32], payload.clone())
                .unwrap();
        let after_snapshot = envelope.cursor_after;
        let frame = sign(envelope, &key);
        let digest = frame_digest(&frame);
        receiver.accept_frame(&frame).unwrap();
        let terminal =
            ReplicationEnvelope::terminal(WORLD, source, 1, NONCE, after_snapshot, digest);
        let frame = sign(terminal, &key);
        let prev = frame_digest(&frame);
        receiver.accept_frame(&frame).unwrap();
        // Gap: a tail starting two entries past the committed cursor.
        let skipped = advance_cursor(after_snapshot, &extra_entry());
        let envelope =
            ReplicationEnvelope::tail(WORLD, source, 1, NONCE, skipped, prev, extra_entry())
                .unwrap();
        let error = receiver.accept_frame(&sign(envelope, &key)).unwrap_err();
        assert_eq!(error.to_string(), ReplicationError::CursorGap.to_string());
        // Fork: a tail at the committed ordinal but on another chain
        // (a different entry advanced the cursor in the envelope build
        // — same ordinal, different digest).
        let other = journal_entry_line(&JournalEntry::CapsCeiling {
            ceiling: vec!["chat".to_string()],
        });
        let fork_cursor = {
            // Chain the real payload, then present a tail whose
            // before-cursor has the right ordinal but a digest from a
            // different entry at the last position.
            let mut lines = payload.split_inclusive(|byte| *byte == b'\n');
            let mut cursor = chain_base_cursor(WORLD, &source, 1);
            cursor = advance_cursor(cursor, lines.next().unwrap());
            cursor = advance_cursor(cursor, lines.next().unwrap());
            advance_cursor(cursor, &other)
        };
        let envelope =
            ReplicationEnvelope::tail(WORLD, source, 1, NONCE, fork_cursor, prev, extra_entry())
                .unwrap();
        let error = receiver.accept_frame(&sign(envelope, &key)).unwrap_err();
        assert_eq!(error.to_string(), ReplicationError::CursorFork.to_string());
        assert_eq!(receiver.read_journal().unwrap(), payload);
    }

    #[test]
    fn unsupported_journal_version_is_refused_before_any_entry() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let future = MAX_JOURNAL_VERSION + 1;
        let envelope = ReplicationEnvelope::snapshot(
            WORLD,
            source,
            future,
            NONCE,
            [0; 32],
            journal_payload(future),
        )
        .unwrap();
        let error = receiver.accept_frame(&sign(envelope, &key)).unwrap_err();
        assert_eq!(
            error.to_string(),
            ReplicationError::UnsupportedJournalVersion(future).to_string()
        );
        assert_eq!(receiver.committed_cursor(), None);
        assert_eq!(receiver.read_journal().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn version_zero_snapshot_is_refused_before_any_entry() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        // Version 0 is below the supported journal range: acceptance
        // must refuse it, because a committed v0 journal is one
        // validate_committed_journal rejects at the next open -
        // accepting it would brick the replica (and destroy any valid
        // journal it replaced).
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source, 0, NONCE, [0; 32], journal_payload(0))
                .unwrap();
        let error = receiver.accept_frame(&sign(envelope, &key)).unwrap_err();
        assert_eq!(
            error.to_string(),
            ReplicationError::UnsupportedJournalVersion(0).to_string()
        );
        assert_eq!(receiver.committed_cursor(), None);
        assert_eq!(receiver.read_journal().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn oversized_payload_is_refused() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let oversized = vec![b'x'; crate::world::replication::MAX_REPLICATION_PAYLOAD_BYTES + 1];
        let envelope = ReplicationEnvelope {
            version: crate::world::replication::REPLICATION_ENVELOPE_VERSION,
            world_id: WORLD.to_string(),
            source,
            header_version: 1,
            kind: ReplicationKind::Snapshot,
            session_nonce: NONCE,
            cursor_before: chain_base_cursor(WORLD, &source, 1),
            cursor_after: chain_base_cursor(WORLD, &source, 1),
            prev_frame_digest: [0; 32],
            payload_digest: crate::world::replication::payload_digest(&oversized),
            payload: oversized,
        };
        let error = receiver
            .accept_frame(&sign_unbounded(envelope, &key))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            ReplicationError::PayloadTooLarge.to_string()
        );
    }

    #[test]
    fn replayed_frames_from_an_old_session_are_refused() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let envelope =
            ReplicationEnvelope::snapshot(WORLD, source, 1, NONCE, [0; 32], journal_payload(1))
                .unwrap();
        let captured = sign(envelope, &key);
        receiver.accept_frame(&captured).unwrap();
        receiver.abort_session();
        // A new session has a new nonce; the captured frame is bound to
        // the old one and must not replay into it.
        receiver.begin_session(NONCE2);
        let error = receiver.accept_frame(&captured).unwrap_err();
        assert_eq!(
            error.to_string(),
            ReplicationError::NonceMismatch.to_string()
        );
    }

    #[test]
    fn reset_snapshot_replaces_the_journal_and_bumps_the_version() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        feed_snapshot(&mut receiver, &key, NONCE, 1, &journal_payload(1));
        receiver.begin_session(NONCE2);
        let v2 = journal_payload(2);
        let progress = feed_snapshot(&mut receiver, &key, NONCE2, 2, &v2);
        assert!(matches!(progress, ReceiverProgress::Committed { .. }));
        assert_eq!(receiver.header_version(), Some(2));
        assert_eq!(receiver.read_journal().unwrap(), v2);
    }

    #[test]
    fn restart_reopens_the_committed_state() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let payload = journal_payload(1);
        let committed;
        {
            let mut receiver = receiver(&dir, &key);
            receiver.begin_session(NONCE);
            feed_snapshot(&mut receiver, &key, NONCE, 1, &payload);
            committed = receiver.committed_cursor().unwrap();
        }
        let receiver = receiver(&dir, &key);
        assert_eq!(receiver.committed_cursor(), Some(committed));
        assert_eq!(receiver.header_version(), Some(1));
        assert_eq!(receiver.read_journal().unwrap(), payload);
        // A journal that no longer decodes is refused at open.
        fs::write(dir.path().join(REPLICA_JOURNAL_FILE), b"not a journal\n").unwrap();
        let error = DurableReceiver::open(dir.path(), WORLD, source_of(&key)).unwrap_err();
        assert!(matches!(error, ReceiverError::CorruptReplica(_)));
    }

    #[test]
    fn tail_or_terminal_first_in_a_session_is_refused() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let source = source_of(&key);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let payload = journal_payload(1);
        feed_snapshot(&mut receiver, &key, NONCE, 1, &payload);
        let committed = receiver.committed_cursor().unwrap();
        // A new session whose first frame is a base-chained tail
        // carrying the Created header: the tracker would accept it
        // against the chain base, but appending it would duplicate the
        // header in the stored journal. The receiver refuses it before
        // the tracker sees it.
        receiver.begin_session(NONCE2);
        let tail = ReplicationEnvelope::tail(
            WORLD,
            source,
            1,
            NONCE2,
            chain_base_cursor(WORLD, &source, 1),
            [0; 32],
            payload.clone(),
        )
        .unwrap();
        let error = receiver.accept_frame(&sign(tail, &key)).unwrap_err();
        assert!(matches!(error, ReceiverError::SessionMustStartWithSnapshot));
        // A terminal as a session's first frame is likewise refused:
        // it must never move the committed cursor without the journal.
        receiver.begin_session(NONCE2);
        let terminal = ReplicationEnvelope::terminal(
            WORLD,
            source,
            1,
            NONCE2,
            chain_base_cursor(WORLD, &source, 1),
            [0; 32],
        );
        let error = receiver.accept_frame(&sign(terminal, &key)).unwrap_err();
        assert!(matches!(error, ReceiverError::SessionMustStartWithSnapshot));
        receiver.abort_session();
        assert_eq!(receiver.committed_cursor(), Some(committed));
        assert_eq!(receiver.read_journal().unwrap(), payload);
        // And the replica still opens cleanly afterwards.
        let reopened = reopen(&dir, &key);
        assert_eq!(reopened.committed_cursor(), Some(committed));
    }

    #[test]
    fn interrupted_tail_append_is_rolled_back_on_open() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let payload = journal_payload(1);
        feed_snapshot(&mut receiver, &key, NONCE, 1, &payload);
        let committed = receiver.committed_cursor().unwrap();
        drop(receiver);
        let journal = dir.path().join(REPLICA_JOURNAL_FILE);
        // Simulate a crash mid-append in two shapes: a torn final line
        // and a complete extra entry — both under a durable
        // append-pending record naming the pre-append length.
        for debris in [b"{\"created\":partial".as_slice(), extra_entry().as_slice()] {
            fs::write(
                dir.path().join(APPEND_PENDING_TMP_FILE),
                payload.len().to_string(),
            )
            .unwrap();
            fs::rename(
                dir.path().join(APPEND_PENDING_TMP_FILE),
                dir.path().join(APPEND_PENDING_FILE),
            )
            .unwrap();
            let mut file = OpenOptions::new().append(true).open(&journal).unwrap();
            file.write_all(debris).unwrap();
            file.sync_all().unwrap();
            drop(file);
            let reopened = reopen(&dir, &key);
            assert_eq!(reopened.read_journal().unwrap(), payload);
            assert_eq!(reopened.committed_cursor(), Some(committed));
            assert!(!dir.path().join(APPEND_PENDING_FILE).exists());
        }
    }

    #[test]
    fn stale_append_pending_record_cannot_roll_back_a_replacement_snapshot() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let mut receiver = receiver(&dir, &key);
        receiver.begin_session(NONCE);
        let payload = journal_payload(1);
        feed_snapshot(&mut receiver, &key, NONCE, 1, &payload);
        // Simulate a failed tail append whose immediate rollback
        // succeeded but whose append-pending record was left behind:
        // the record still names the pre-append journal length and the
        // journal is exactly that long.
        fs::write(
            dir.path().join(APPEND_PENDING_TMP_FILE),
            payload.len().to_string(),
        )
        .unwrap();
        fs::rename(
            dir.path().join(APPEND_PENDING_TMP_FILE),
            dir.path().join(APPEND_PENDING_FILE),
        )
        .unwrap();
        // A reconnect snapshot commits on the same receiver. Committing
        // it must recover and clear the stale record first; otherwise
        // the next open would truncate the replacement snapshot back
        // to the old pre-append length.
        let replacement = journal_payload(2);
        receiver.begin_session(NONCE2);
        let progress = feed_snapshot(&mut receiver, &key, NONCE2, 2, &replacement);
        assert!(matches!(progress, ReceiverProgress::Committed { .. }));
        assert!(!dir.path().join(APPEND_PENDING_FILE).exists());
        drop(receiver);
        let reopened = reopen(&dir, &key);
        assert_eq!(reopened.read_journal().unwrap(), replacement);
        assert_eq!(reopened.header_version(), Some(2));
    }

    #[test]
    fn frames_without_a_session_are_refused() {
        let dir = TempDir::new().unwrap();
        let key = signing_key(1);
        let mut receiver = receiver(&dir, &key);
        let envelope = ReplicationEnvelope::snapshot(
            WORLD,
            source_of(&key),
            1,
            NONCE,
            [0; 32],
            journal_payload(1),
        )
        .unwrap();
        let error = receiver.accept_frame(&sign(envelope, &key)).unwrap_err();
        assert!(matches!(error, ReceiverError::NoSession));
    }
}
