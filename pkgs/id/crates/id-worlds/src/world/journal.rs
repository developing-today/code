//! Durable world journal records.
//!
//! [`JournalEntry`] is the commit-ordered fact log a [`WorldJournal`]
//! appends, and the `JOURNAL_VERSION_*` constants with
//! [`journal_version_required_by`] carry the journal-format upgrade and
//! refusal contract (issue #227). The records live here so
//! `world/mod.rs` stays within the production-line limit; they are
//! re-exported from [`crate::world`], so every public path is unchanged.

use std::io::Read;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::signed;
use super::{
    CheckpointBlobEntry, CheckpointKvEntry, CheckpointNotifyEntry, CheckpointSchedule, FileScope,
    HandoffTransferState, Isolation, LeaseRejectionReason, ModuleProvenance, Participant,
    WorldEvent, WorldEventKind, WriteLease, hex_decode, hex_encode, replication,
};

/// One durable fact in the region coordinator's own journal (#3870 phase 1,
/// design #2818 §3). The coordinator records and recovers handoff progress;
/// it never stores entity payloads and never emulates a cross-region
/// transaction. Phase 4 of #3870 builds the coordinator state machine on
/// these records; this packet fixes their format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum CoordinatorJournalEntry {
    /// A handoff was requested. Persisted before any shard side effect.
    HandoffStarted {
        /// Random 128-bit handoff ID, as hex.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region the entity leaves.
        source_region: String,
        /// Region the entity enters.
        target_region: String,
        /// Owner epoch the coordinator expects at the source.
        expected_owner_epoch: u64,
        /// Region-map epoch observed when the transfer was requested.
        region_map_epoch: u64,
        /// Program module hash both regions must run for v1.
        program_module_hash: String,
        /// Versioned payload schema name.
        payload_schema: String,
        /// Unix milliseconds the handoff was requested (operator
        /// visibility only; ordering comes from journal order).
        requested_at: u64,
    },
    /// The coordinator verified the source's durable, applied export
    /// receipt for this handoff.
    HandoffExportObserved {
        /// Handoff ID.
        handoff_id: String,
        /// Digest of the exported payload.
        payload_digest: String,
        /// Unix milliseconds the receipt was observed.
        observed_at: u64,
    },
    /// The coordinator verified the destination's durable, applied import
    /// receipt for this handoff.
    HandoffImportObserved {
        /// Handoff ID.
        handoff_id: String,
        /// Digest of the imported payload.
        payload_digest: String,
        /// Unix milliseconds the receipt was observed.
        observed_at: u64,
    },
    /// The handoff completed: the destination's matching import is durable
    /// and applied. Persisted before the coordinator changes its in-memory
    /// owner map or exposes the destination.
    HandoffCompleted {
        /// Handoff ID.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region the entity left.
        source_region: String,
        /// Region the entity entered.
        target_region: String,
        /// Digest of the transferred payload.
        payload_digest: String,
        /// Owner epoch at the destination (`prior + 1`).
        new_owner_epoch: u64,
        /// Region-map epoch after this completion.
        new_region_map_epoch: u64,
        /// Program module hash the transfer ran under.
        program_module_hash: String,
        /// Versioned payload schema name.
        payload_schema: String,
    },
    /// A handoff was cancelled. Legal only before the source export
    /// committed; the coordinator must first prove no export exists.
    HandoffCancelled {
        /// Handoff ID.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Why the handoff was cancelled.
        reason: String,
    },
}

/// The basis on which the module trust gate admitted a module (#1069).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustBasis {
    /// A publisher signature verified against the configured trust list.
    SignatureVerified,
    /// No trust list is configured, so the gate admits any module.
    NoTrustList,
    /// The server compiled the module from source itself, so the upload
    /// trust gate does not apply.
    ServerCompiled,
}

/// A trust gate decision recorded in the journal as part of the
/// install event it admitted (#1069): the basis, and the publisher key
/// when one verified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustDecision {
    /// The basis on which the module was admitted.
    pub basis: TrustBasis,
    /// Hex public key of the verifying publisher, if any.
    pub publisher: Option<String>,
}

/// One accepted signed move as journaled (#4346): the journal form of
/// [`signed::SignedMoveRecord`], with the key, event id, and signed bytes
/// hex-encoded like the journal's other digests and payloads. Replay
/// converts it back and re-verifies the move against the author's chain
/// (see `WorldCore::restore`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalSignedMove {
    /// Host world sequence of the committed input event.
    pub world_sequence: u64,
    /// Server-assigned participant that submitted the move.
    pub participant_id: u64,
    /// The author's verifying key bytes (hex), as bound to the participant.
    pub author: String,
    /// The move's sequence in the author's own chain.
    pub author_seq: u32,
    /// The signed event's id (hex).
    pub event_id: String,
    /// The signed event's encoded header bytes (hex).
    pub bytes: String,
}

impl JournalSignedMove {
    /// The journal form of an accepted signed-move record.
    #[must_use]
    pub fn from_record(record: &signed::SignedMoveRecord) -> Self {
        Self {
            world_sequence: record.world_sequence,
            participant_id: record.participant_id,
            author: hex_encode(&record.author),
            author_seq: record.author_seq,
            event_id: record.event_id.to_string(),
            bytes: hex_encode(&record.bytes),
        }
    }

    /// Back to the runtime record, refusing malformed encodings.
    ///
    /// # Errors
    ///
    /// Returns an error when a hex field does not decode, the author key
    /// is not 32 bytes, or the event id does not parse.
    pub fn to_record(&self) -> Result<signed::SignedMoveRecord> {
        let author: [u8; 32] = hex_decode(&self.author)
            .and_then(|bytes| bytes.try_into().ok())
            .context("malformed author key in journaled signed move")?;
        let event_id: p2panda_core::Hash = self
            .event_id
            .parse()
            .map_err(|_| anyhow::anyhow!("malformed event id in journaled signed move"))?;
        let bytes = hex_decode(&self.bytes).context("malformed signed move bytes in journal")?;
        Ok(signed::SignedMoveRecord {
            world_sequence: self.world_sequence,
            participant_id: self.participant_id,
            author,
            author_seq: self.author_seq,
            event_id,
            bytes,
        })
    }
}

/// One durable fact about a world, in commit order.
///
/// Capability secrets are never journaled, only their SHA-256 digests, so a
/// leaked journal cannot be used to join. Replaying a journal rebuilds the
/// participants, the event sequence, and (with the module bytes) the program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum JournalEntry {
    /// First entry of every journal.
    Created {
        /// World this journal belongs to.
        world_id: String,
        /// Journal format version.
        version: u32,
    },
    /// A capability was issued.
    Issued {
        /// The new participant.
        participant: Participant,
        /// Hex SHA-256 of the bearer capability.
        digest: String,
        /// Granted scopes (bit set).
        scopes: u8,
        /// The capability this one was delegated from, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<u64>,
        /// Unix milliseconds at which the capability stops working.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<u64>,
        /// Committed chat and input actions the capability may take. Host-local:
        /// only a capability the host issued has one, never a delegation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uses: Option<u32>,
        /// Actions already taken, written at checkpoint so counts survive compaction.
        /// Host-local, like `uses`.
        #[serde(default)]
        used: u32,
        /// Account the capability acts for; its ceiling and friendships bound it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        /// Unix milliseconds at which the world issued the capability, from
        /// the world clock (#11960). `None` for capabilities issued before
        /// times were recorded; never inferred. Requires
        /// [`JOURNAL_VERSION_JOIN_TIMES`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issued_at: Option<u64>,
    },
    /// A capability was revoked, with every capability delegated from it.
    /// Replay revokes the same tree.
    Revoked {
        /// Participant whose capability was revoked.
        participant_id: u64,
        /// The moderator participant who revoked it, when the revoke came
        /// from a moderator scope rather than the admin. `None` means admin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<u64>,
        /// Unix milliseconds at which the revoke took effect, from the world
        /// clock (#11960). `None` for revokes recorded before times were.
        /// Requires [`JOURNAL_VERSION_JOIN_TIMES`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revoked_at: Option<u64>,
    },
    /// A builder-scoped participant installed a program. Journal-only: it is
    /// the audit record for the builder authority, written alongside the
    /// `ProgramInstalled` event that carries the install itself.
    BuilderInstalled {
        /// The builder participant who installed the program.
        by: u64,
        /// The hash of the installed module.
        module_hash: String,
        /// The seed the program was installed with.
        seed: u64,
    },
    /// The world was forked from another world (F12.2). Journal-only: the
    /// copied entries already reproduce the parent's state, and this entry
    /// records the origin so a fork can trace its provenance.
    Forked {
        /// The world this one was forked from.
        from_world_id: String,
        /// The content hash of the module at the fork point.
        from_module_hash: String,
        /// Whether the fork was marked reusable as a template.
        template: bool,
        /// The sequence the parent had reached when the fork was taken.
        at_sequence: u64,
    },
    /// A sequenced event was committed.
    Committed {
        /// The event.
        event: WorldEvent,
    },
    /// A host event delivered to the program (capability result, tick,
    /// presence). Delivered with sequence 0; the program saw it, so replay
    /// must too.
    System {
        /// The event as it was delivered.
        event: WorldEvent,
    },
    /// The world's granted capability set, written whole on every change.
    CapsGranted {
        /// Capability names.
        granted: Vec<String>,
    },
    /// The world's capability ceiling, written whole on every change. Absent
    /// means the whole catalog. Checkpoints carry it in `caps_ceiling`.
    CapsCeiling {
        /// Capability names the world allows.
        ceiling: Vec<String>,
    },
    /// The world's requested execution profile. Limits are resolved against
    /// the current server hard ceilings when a program instance is created.
    ExecutionProfile {
        /// Per-world resource requests; omitted fields inherit server limits.
        profile: crate::world_limits::WorldExecutionProfile,
    },
    /// The world's isolation, written whole on every change. Absent means
    /// [`Isolation::Isolated`].
    Isolation {
        /// The new isolation.
        isolation: Isolation,
    },
    /// Whether other unisolated worlds may write here, set by a moderator or
    /// the admin. `enabled: false` also clears a force. Absent means off.
    CrossWorldWrite {
        /// The new setting.
        enabled: bool,
    },
    /// The admin forced cross-world writes on. Absent means not forced.
    CrossWorldWriteForced,
    /// The world's file scope, written whole on every change. Absent means confined.
    FileScope {
        /// The new scope.
        scope: FileScope,
    },
    /// Whether the admin let an isolated world write files. Absent means off.
    FileWriteOverride {
        /// The new setting.
        enabled: bool,
    },
    /// A `schedule.at` request was accepted: `id` fires at `at_ms` (unix
    /// milliseconds). Written before the request's result is delivered.
    /// A recurring schedule (#1355) also carries `interval_ms` and
    /// `remaining` (total fires still to come, the pending one included);
    /// both are absent for a one-shot schedule, and in entries written
    /// before journal version 8.
    ScheduleSet {
        /// The schedule's id, unique among pending schedules.
        id: String,
        /// When it fires, in unix milliseconds.
        at_ms: u64,
        /// Fixed interval between fires, for a recurring schedule.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interval_ms: Option<u64>,
        /// Fires remaining including the pending one, for a recurring
        /// schedule. Absent means one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remaining: Option<u32>,
        /// The world the fire is delivered to, for a cross-world schedule
        /// (#1355 part 2). Absent means this world.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    /// The admin set this world's schedule-delivery consent for one peer
    /// world (#1355 part 2, the F12.14 reciprocal-grant shape): `send`
    /// lets this world's schedules target `peer`, `receive` lets `peer`'s
    /// schedules be delivered here. Written whole on every change; both
    /// false is a revocation. A cross-world fire needs the source's send
    /// consent and the target's receive consent for the same pair.
    ScheduleGrantSet {
        /// The peer world's name.
        peer: String,
        /// This world may pend schedule fires for `peer`.
        send: bool,
        /// `peer` may deliver schedule fires to this world.
        receive: bool,
    },
    /// The admin set this world's F12.14 portal grant for one peer
    /// world (code#1068 wiring slice): `granted: true` records the
    /// grant, `granted: false` records its revocation. Written only
    /// when the recorded state actually changes (a grant, or the
    /// revocation of a live grant), so replaying it restores the
    /// peer's exact `PortalGrantState` — a revoked peer replays as
    /// revoked, never as never-granted, keeping the two refusal
    /// codes distinct across restarts.
    PortalGrantSet {
        /// The peer world's name.
        peer: String,
        /// Whether the portal to `peer` is granted after this entry.
        granted: bool,
    },
    /// This world durably accepted a cross-world schedule fire from
    /// `source` (#1355 part 2): the receipt, keyed by source world,
    /// schedule id, and the fire's planned time (its occurrence). Written
    /// after the fire's `System` entry, so a source retry after a restart
    /// is answered from the receipt instead of delivering twice.
    ScheduleReceived {
        /// The world the schedule belongs to.
        source: String,
        /// The schedule's id in the source world.
        id: String,
        /// The fire's planned due time in the source world, in unix
        /// milliseconds: the occurrence key.
        at_ms: u64,
    },
    /// A pending schedule was cancelled before it fired.
    ScheduleCancelled {
        /// The cancelled schedule's id.
        id: String,
    },
    /// A pending schedule fired. It follows the fire's `System` entry, which
    /// the program accepted first, so the schedule is consumed only once its
    /// delivery is durable.
    ScheduleFired {
        /// The fired schedule's id.
        id: String,
    },
    /// A `kv.set` request was accepted (#1062): `key` now holds
    /// `value`, the stored value's JSON encoding. Written before the
    /// request's result is delivered. Setting an existing key replaces
    /// its value; replay applies entries in order, so the last write
    /// to a key is the store's state.
    KvSet {
        /// The key being set.
        key: String,
        /// The value's JSON encoding.
        value: String,
    },
    /// A `kv.delete` request removed a live entry (#1062). Written
    /// only when the key was present, before the request's result is
    /// delivered.
    KvDeleted {
        /// The key that was removed.
        key: String,
    },
    /// A `blob.put` request stored new content (#1062): the blob
    /// addressed by `hash` — the content's iroh-blobs hash, as
    /// lowercase hex — now holds `data`, the content as base64.
    /// Written before the request's result is delivered, and only
    /// when the world did not already hold the blob: putting content
    /// the world already holds is a no-op and journals nothing.
    /// Replay decodes `data` and refuses the entry unless it hashes
    /// back to `hash`, so the journal is self-verifying.
    BlobStored {
        /// The blob's content address: its hash as lowercase hex.
        hash: String,
        /// The content, as base64.
        data: String,
    },
    /// A `blob.delete` request removed a live blob (#1062). Written
    /// only when the blob was present, before the request's result
    /// is delivered.
    BlobRemoved {
        /// The hash of the blob that was removed.
        hash: String,
    },
    /// A `notify.send` request was accepted (#1062): the world's
    /// notification log gains this record. Written before the
    /// request's result is delivered; it is the send's audit entry.
    /// Replay appends it and drops the oldest retained notification
    /// when the log is past its cap, exactly as the writer did, and
    /// the rate window is counted from the retained records' times.
    NotifySent {
        /// The notification's title, when the send named one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// The notification's text.
        text: String,
        /// When the send was accepted, in unix milliseconds.
        at_ms: u64,
    },
    /// A participant's author key was bound for signed play (#2814,
    /// #4346). Written when the binding is first made; replaying it
    /// restores the binding and the author's (still empty) chain, so a
    /// bound participant's chain position survives a restart without
    /// re-anchoring. Re-binding the same key replays as a no-op, exactly
    /// as at runtime.
    AuthorKeyBound {
        /// The participant the key is bound to.
        participant_id: u64,
        /// The player's ed25519 verifying key bytes (hex).
        author: String,
    },
    /// A signed move was accepted (#2814, #4346): follows the `Committed`
    /// entry of the input event it cross-references, in the order the
    /// actor persisted them. Replay re-verifies the move against the
    /// author's restored chain before accepting it, so a journal whose
    /// signed history was tampered with is refused, not restored.
    SignedMoveAccepted {
        /// The accepted move, in journal form.
        record: JournalSignedMove,
    },
    /// An application-owned entity was exported to another region shard
    /// (#3870 phase 1, design #2818 §3). Written persist-before-apply: once
    /// this entry is durable the source is fenced for `entity_ref` and this
    /// entry is the durable recovery copy of the payload until the
    /// coordinator journals completion.
    EntityExported {
        /// Coordinator-generated handoff ID (hex); the receipt key.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region exporting the entity.
        source_region: String,
        /// Region importing the entity.
        target_region: String,
        /// Owner epoch at the source before this handoff.
        prior_owner_epoch: u64,
        /// The host-originated system event removing the entity; replay
        /// applies it from this same durable fact.
        source_event: WorldEvent,
        /// Versioned payload schema name.
        payload_schema: String,
        /// Program module hash the destination must match.
        program_module_hash: String,
        /// The complete entity snapshot (bounded by the transfer payload
        /// cap; the cap itself is enforced from phase 2 of #3870).
        payload: Vec<u8>,
        /// Digest of `payload`.
        payload_digest: String,
    },
    /// An application-owned entity was imported from another region shard
    /// (#3870 phase 1, design #2818 §3). Written persist-before-apply; the
    /// durable receipt for `handoff_id`. An exact retry returns the
    /// existing receipt; the same ID with a different digest is a hard
    /// conflict (phase 3 of #3870).
    EntityImported {
        /// Coordinator-generated handoff ID (hex); the dedupe key.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region that exported the entity.
        source_region: String,
        /// Region importing the entity.
        target_region: String,
        /// Owner epoch at the source before this handoff.
        prior_owner_epoch: u64,
        /// Owner epoch at this destination (`prior_owner_epoch + 1`).
        new_owner_epoch: u64,
        /// Program module hash, matched against the export before commit.
        program_module_hash: String,
        /// Versioned payload schema name.
        payload_schema: String,
        /// Digest of the imported payload.
        payload_digest: String,
        /// The host-originated system event adding the entity (it carries
        /// the payload, hex-encoded, per design #2818 §3).
        destination_event: WorldEvent,
    },
    /// A write lease was granted while no holder stood (design #2819,
    /// phase 2 of issue #3804). Authority-only: the append authority
    /// writes this, never a lease holder on its own behalf. `epoch` is
    /// the resulting epoch (`previous + 1`).
    LeaseGranted {
        /// The new holder's authenticated identity.
        holder: String,
        /// The resulting lease epoch.
        epoch: u64,
        /// When the granted term expires, in unix milliseconds on the
        /// authority's clock; absent for an untermed holder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_ms: Option<u64>,
    },
    /// The current holder's lease term was renewed by the authority
    /// (design #2819). The epoch is unchanged.
    LeaseRenewed {
        /// The holder whose term was renewed.
        holder: String,
        /// The (unchanged) lease epoch.
        epoch: u64,
        /// The new term expiry, in unix milliseconds.
        expires_at_ms: u64,
    },
    /// The write lease was handed from one holder to another
    /// (design #2819). This entry's journal position is the ownership
    /// boundary: before it only the old epoch passes the append gate,
    /// after it only the new one. `epoch` is the resulting epoch.
    LeaseHandedOff {
        /// The holder giving up execution.
        from: String,
        /// The holder taking over execution.
        to: String,
        /// The resulting lease epoch.
        epoch: u64,
        /// When the new holder's term expires, in unix milliseconds;
        /// absent for an untermed holder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_ms: Option<u64>,
    },
    /// The write lease was revoked by the authority (design #2819):
    /// the former holder is fenced at the resulting epoch and no
    /// holder stands until the next grant.
    LeaseRevoked {
        /// The holder whose lease was revoked.
        former_holder: String,
        /// The resulting lease epoch.
        epoch: u64,
        /// Why the lease was revoked.
        reason: String,
    },
    /// An authenticated append attempt was refused by the append gate
    /// and the authority journaled the attempt as an audit record
    /// (design #2819). The attempted entry itself was not applied.
    /// Control metadata: replay rebuilds the audit, never the program.
    LeaseWriteRejected {
        /// The authenticated writer identity that was refused.
        writer: String,
        /// The epoch the writer presented.
        presented_epoch: u64,
        /// The client operation ID the writer presented.
        operation_id: String,
        /// The authority's epoch when the attempt was refused.
        observed_epoch: u64,
        /// Why the attempt was refused.
        reason: LeaseRejectionReason,
        /// When the authority received the attempt, in unix
        /// milliseconds on the authority's clock.
        received_at_ms: u64,
    },
    /// One UTF-8-safe part of a compacted program snapshot. The complete set
    /// immediately precedes its checkpoint and is keyed by that checkpoint's
    /// sequence.
    CheckpointSnapshotPart {
        /// Sequence of the checkpoint these bytes belong to.
        sequence: u64,
        /// Zero-based position within this part set.
        part_index: u32,
        /// Number of parts in this set.
        part_count: u32,
        /// UTF-8 snapshot bytes for this part.
        bytes: String,
    },
    /// One bounded part of the retained participant-event window.
    CheckpointEventsPart {
        /// Sequence of the checkpoint these events belong to.
        sequence: u64,
        /// Zero-based position within this part set.
        part_index: u32,
        /// Number of parts in this set.
        part_count: u32,
        /// Ordered events in this part.
        events: Vec<WorldEvent>,
    },
    /// One bounded part of the retained admin-history window.
    CheckpointAdminPart {
        /// Sequence of the checkpoint these events belong to.
        sequence: u64,
        /// Zero-based position within this part set.
        part_index: u32,
        /// Number of parts in this set.
        part_count: u32,
        /// Ordered events in this part.
        events: Vec<WorldEvent>,
    },
    /// The world as of `sequence`, standing in for every event up to it. Only
    /// ever written by compaction, directly after the participant entries.
    Checkpoint {
        /// Sequence of the last event the checkpoint covers.
        sequence: u64,
        /// Active program.
        module_hash: String,
        /// Seed the program was initialized with.
        seed: u64,
        /// Provenance of the active module, if it was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<ModuleProvenance>,
        /// Requested resource profile. Absent in older checkpoints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_profile: Option<crate::world_limits::WorldExecutionProfile>,
        /// The world's capability ceiling. Absent in older checkpoints, and
        /// when it is the whole catalog.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caps_ceiling: Option<Vec<String>>,
        /// Pending schedules. Absent when none are pending, and it must
        /// match the `ScheduleSet` entries before it. One-shot schedules
        /// serialize in the legacy `(id, at_ms)` pair form; recurring
        /// schedules (#1355) serialize as an object carrying their
        /// interval and remaining fires.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schedules: Option<Vec<CheckpointSchedule>>,
        /// Live `kv` store entries (#1062). Absent when the store is
        /// empty, and it must match the `KvSet` entries before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kv: Option<Vec<CheckpointKvEntry>>,
        /// Live blob index (#1062): hash and content size per live
        /// blob. Absent when the world holds no blobs, and it must
        /// match the `BlobStored` entries before it. Only the index
        /// is declared, never the content — the entries carry the
        /// bytes, and each is hash-verified as it replays.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blobs: Option<Vec<CheckpointBlobEntry>>,
        /// Retained `notify` log (#1062): the whole records, in
        /// send order. Absent when the world has sent nothing, and
        /// it must match the `NotifySent` entries before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notifications: Option<Vec<CheckpointNotifyEntry>>,
        /// Active region-seam handoffs (#3870 phase 1, design #2818 §3):
        /// export fences/payloads, import receipts/dedupe keys and owner
        /// epochs, so compaction cannot resurrect an exported entity or
        /// forget an import. Absent when no handoff is active, and in
        /// checkpoints written before journal version 3.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transfers: Option<Vec<HandoffTransferState>>,
        /// The journal-resident write lease as of this checkpoint
        /// (design #2819, phase 2 of issue #3804): current holder,
        /// epoch and term, the full transition history, and the
        /// rejection audit, so compaction cannot reset the epoch or
        /// lose proof of ownership. Absent when the lease is still the
        /// bootstrap state, and in checkpoints written before journal
        /// version 7.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease: Option<WriteLease>,
        /// The program's own serialization of its state.
        snapshot: String,
        /// Recent events kept for reconnect catch-up.
        events: Vec<WorldEvent>,
        /// Ordered sequenced and host-originated events retained for the
        /// admin-only fixture export. Absent in older checkpoints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        admin_history: Option<JournalAdminHistory>,
        /// Participants whose join the program has seen without a later leave.
        /// `Some(empty)` records a known-empty set; `None` is reserved for
        /// legacy checkpoints and triggers the documented migration fallback.
        /// See the journal-version contract.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open_presence: Option<Vec<u64>>,
    },
}

/// The bounded ordered event window carried across journal compaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalAdminHistory {
    pub(crate) events: Vec<WorldEvent>,
    pub(crate) complete: bool,
}

/// Version written in a new journal's [`JournalEntry::Created`] header.
///
/// Journal upgrade/rollback contract (issue #227): a journal is created at
/// version 1 and stays at version 1 until it contains an entry kind a
/// version-1 binary cannot read. New entry kinds raise the header to the
/// highest required version *before* the entry is appended, by atomically
/// rewriting the journal (sibling temp file, fsync, rename), so a failed bump
/// leaves the old journal untouched. The first such entries were an
/// [`JournalEntry::ExecutionProfile`], a region-handoff entry, an entry
/// carrying a per-world total memory budget, and a checkpoint carrying
/// open program-visible presence. A binary refuses any journal newer than
/// [`MAX_JOURNAL_VERSION`] at the header check in [`crate::world::WorldCore::restore`],
/// never mid-read. New binaries accept versions 1..=[`MAX_JOURNAL_VERSION`],
/// so existing journals stay readable and a journal that never needs a newer
/// entry kind is never migrated.
pub const JOURNAL_VERSION: u32 = 1;

/// Header version of a journal that contains a
/// [`JournalEntry::ExecutionProfile`] or [`JournalEntry::BuilderInstalled`]:
/// entry kinds a version-1 binary cannot read. See the contract on
/// [`JOURNAL_VERSION`].
pub const JOURNAL_VERSION_EXECUTION_PROFILE: u32 = 2;

/// Header version of a journal that contains a region-seam handoff entry
/// ([`JournalEntry::EntityExported`] / [`JournalEntry::EntityImported`], an
/// entity-addressed [`WorldEventKind::EntityInput`] event, or a checkpoint
/// carrying handoff transfers): the first entry kinds a version-2 binary
/// cannot read (#3870 phase 1, design #2818 §3). See the contract on
/// [`JOURNAL_VERSION`].
pub const JOURNAL_VERSION_REGION_HANDOFF: u32 = 3;

/// Header version of a journal that persists a per-world total memory
/// budget (#557 Phase 1, design `doc/2026-11-01_design_id557.md`): an
/// [`JournalEntry::ExecutionProfile`] or [`JournalEntry::Checkpoint`]
/// whose profile sets `total_memory_bytes`. An older binary would parse
/// the entry yet silently ignore the budget it cannot account for, so the
/// budget requires a version the old binary refuses at the header check.
/// Profiles and checkpoints without a budget keep their existing version.
pub const JOURNAL_VERSION_WORLD_BUDGET: u32 = 4;

/// Header version of a checkpoint that carries open program-visible presence.
/// Version-4 binaries would ignore the field and fail to close those joins on
/// restart, so such checkpoints must be refused by them. This follows the
/// world-budget journal version so both features retain distinct contracts.
/// (Renumbered from 4 to 5 when this branch merged main: main had already
/// assigned version 4 to the world-budget contract above.)
pub const JOURNAL_VERSION_OPEN_PRESENCE: u32 = 5;

/// Header version required by compacted checkpoint part entries. Older
/// binaries refuse these entries at the header check instead of silently
/// restoring an incomplete checkpoint.
pub const JOURNAL_VERSION_CHECKPOINT_PARTS: u32 = 6;

/// Header version of a journal that contains a native-spill write
/// lease entry ([`JournalEntry::LeaseGranted`] and its siblings), or a
/// checkpoint carrying a non-bootstrap [`WriteLease`] (design #2819,
/// phase 2 of issue #3804). Version-6 binaries cannot read the lease
/// records and would restore a world with no fencing state, so they
/// must refuse the journal at the header check.
pub const JOURNAL_VERSION_WRITE_LEASE: u32 = 7;

/// Header version of a journal that contains a recurring schedule: a
/// [`JournalEntry::ScheduleSet`] carrying an interval / remaining count,
/// or a checkpoint declaring one (#1355). Version-7 binaries would parse
/// the entry yet silently restore the series as a one-shot schedule, so
/// they must refuse the journal at the header check.
pub const JOURNAL_VERSION_SCHEDULE_RECURRENCE: u32 = 8;

/// Header version of a journal that contains a signed-play entry
/// ([`JournalEntry::AuthorKeyBound`] / [`JournalEntry::SignedMoveAccepted`],
/// issue #4346). Version-8 binaries cannot read these entries and would
/// restore a world whose signed chains were silently lost, so they must
/// refuse the journal at the header check. (Signed play originally also
/// claimed version 8 in PR #9037; the rebase onto the #1355 recurrence
/// bump renumbered it to 9.)
pub const JOURNAL_VERSION_SIGNED_PLAY: u32 = 9;

/// Header version of a journal that contains a portal-grant entry
/// ([`JournalEntry::PortalGrantSet`], F12.14 wiring, code#1068).
/// Version-9 binaries cannot read these entries and would restore a
/// world whose portal grants were silently lost, so they must refuse
/// the journal at the header check.
pub const JOURNAL_VERSION_PORTALS: u32 = 10;

/// Header version of a journal that records a program-bundle install.
///
/// That is a `ProgramInstalled` event, or a checkpoint, whose provenance is
/// [`ModuleProvenance::Bundle`] (#8280). Version-10 binaries cannot parse
/// that provenance and would fail mid-replay, so they must refuse the
/// journal at the header check.
pub const JOURNAL_VERSION_PROGRAM_BUNDLE: u32 = 11;

/// Journal version 12 (#11960): `Issued` entries carry `issued_at` and
/// `Revoked` entries carry `revoked_at`. The fields are optional, so a
/// version-11 reader would parse such an entry, drop the time and lose it
/// at its next checkpoint; an entry that carries a time therefore requires
/// this version, and an older binary refuses the journal at the header
/// check instead.
pub const JOURNAL_VERSION_JOIN_TIMES: u32 = 12;

/// Journal version 13 (#1062): the `kv` store entries
/// ([`JournalEntry::KvSet`] / [`JournalEntry::KvDeleted`]) and a
/// checkpoint carrying a `kv` field. A version-12 binary cannot parse
/// the new entry kinds and would fail mid-replay, so a journal
/// containing them must be refused at the header check instead.
pub const JOURNAL_VERSION_KV: u32 = 13;

/// Journal version 14 (#1062): the `blob` store entries
/// ([`JournalEntry::BlobStored`] / [`JournalEntry::BlobRemoved`])
/// and a checkpoint carrying a `blobs` field. A version-13 binary
/// cannot parse the new entry kinds and would fail mid-replay, so a
/// journal containing them must be refused at the header check
/// instead.
pub const JOURNAL_VERSION_BLOB: u32 = 14;

/// Journal version 15 (#1062): the `notify` log entry
/// ([`JournalEntry::NotifySent`]) and a checkpoint carrying a
/// `notifications` field. A version-14 binary cannot parse the new
/// entry kind and would fail mid-replay, so a journal containing
/// them must be refused at the header check instead.
pub const JOURNAL_VERSION_NOTIFY: u32 = 15;

/// Newest journal version this binary can read. Journals with a higher
/// header version are refused at the header check, before any entry is
/// applied.
pub const MAX_JOURNAL_VERSION: u32 = JOURNAL_VERSION_NOTIFY;

/// The header version a journal must carry once it contains `entry`: version 2
/// for execution-profile entries, version 3 for region-handoff entries,
/// version 4 for entries persisting a per-world total memory budget, version 5
/// for checkpoints with open-presence metadata, and version 6 for checkpoint
/// part entries. Checkpoints combine the requirements of their transfers,
/// retained events, profile and presence field.
///
/// Any entry that records a program-bundle install additionally requires
/// [`JOURNAL_VERSION_PROGRAM_BUNDLE`].
#[must_use]
pub fn journal_version_required_by(entry: &JournalEntry) -> u32 {
    let kind = entry_kind_version(entry);
    if records_bundle_install(entry) {
        kind.max(JOURNAL_VERSION_PROGRAM_BUNDLE)
    } else {
        kind
    }
}

/// Whether `event` installs a program from a bundle.
const fn is_bundle_install(event: &WorldEvent) -> bool {
    matches!(
        &event.kind,
        WorldEventKind::ProgramInstalled {
            provenance: Some(provenance),
            ..
        } if provenance.is_bundle()
    )
}

/// Whether `entry` carries bundle provenance anywhere a reader must parse it.
fn records_bundle_install(entry: &JournalEntry) -> bool {
    match entry {
        JournalEntry::Committed { event } | JournalEntry::System { event } => {
            is_bundle_install(event)
        }
        JournalEntry::EntityExported { source_event, .. } => is_bundle_install(source_event),
        JournalEntry::EntityImported {
            destination_event, ..
        } => is_bundle_install(destination_event),
        JournalEntry::CheckpointEventsPart { events, .. }
        | JournalEntry::CheckpointAdminPart { events, .. } => events.iter().any(is_bundle_install),
        JournalEntry::Checkpoint {
            provenance,
            events,
            admin_history,
            ..
        } => {
            provenance.as_ref().is_some_and(ModuleProvenance::is_bundle)
                || events.iter().any(is_bundle_install)
                || admin_history
                    .as_ref()
                    .is_some_and(|history| history.events.iter().any(is_bundle_install))
        }
        _ => false,
    }
}

/// The version `entry`'s own kind and fields require, before provenance.
fn entry_kind_version(entry: &JournalEntry) -> u32 {
    // A persisted budget always requires the budget version, on top of any
    // version the entry kind itself already requires.
    let budget_version = |profile: Option<&crate::world_limits::WorldExecutionProfile>| {
        if profile.is_some_and(|p| p.total_memory_bytes.is_some()) {
            JOURNAL_VERSION_WORLD_BUDGET
        } else {
            JOURNAL_VERSION
        }
    };
    match entry {
        JournalEntry::EntityExported { .. } | JournalEntry::EntityImported { .. } => {
            JOURNAL_VERSION_REGION_HANDOFF
        }
        JournalEntry::LeaseGranted { .. }
        | JournalEntry::LeaseRenewed { .. }
        | JournalEntry::LeaseHandedOff { .. }
        | JournalEntry::LeaseRevoked { .. }
        | JournalEntry::LeaseWriteRejected { .. } => JOURNAL_VERSION_WRITE_LEASE,
        JournalEntry::AuthorKeyBound { .. } | JournalEntry::SignedMoveAccepted { .. } => {
            JOURNAL_VERSION_SIGNED_PLAY
        }
        JournalEntry::CheckpointSnapshotPart { .. }
        | JournalEntry::CheckpointEventsPart { .. }
        | JournalEntry::CheckpointAdminPart { .. } => JOURNAL_VERSION_CHECKPOINT_PARTS,
        JournalEntry::Committed { event } | JournalEntry::System { event } => match &event.kind {
            WorldEventKind::EntityInput { .. } => JOURNAL_VERSION_REGION_HANDOFF,
            _ => JOURNAL_VERSION,
        },
        // A checkpoint stands in for the entries it compacted, so it must
        // require every version its contents require: its transfer list,
        // and its retained events — a retained `EntityInput` event is a
        // version-3 kind even when no handoff is active (Codex review on
        // PR #3922).
        JournalEntry::ScheduleSet {
            interval_ms,
            remaining,
            target,
            ..
        } if interval_ms.is_some() || remaining.is_some() || target.is_some() => {
            JOURNAL_VERSION_SCHEDULE_RECURRENCE
        }
        JournalEntry::ScheduleGrantSet { .. } | JournalEntry::ScheduleReceived { .. } => {
            JOURNAL_VERSION_SCHEDULE_RECURRENCE
        }
        JournalEntry::KvSet { .. } | JournalEntry::KvDeleted { .. } => JOURNAL_VERSION_KV,
        JournalEntry::BlobStored { .. } | JournalEntry::BlobRemoved { .. } => JOURNAL_VERSION_BLOB,
        JournalEntry::NotifySent { .. } => JOURNAL_VERSION_NOTIFY,
        JournalEntry::PortalGrantSet { .. } => JOURNAL_VERSION_PORTALS,
        JournalEntry::Checkpoint {
            transfers,
            lease,
            events,
            open_presence,
            execution_profile,
            schedules,
            kv,
            blobs,
            notifications,
            ..
        } => {
            let base = if notifications.is_some() {
                JOURNAL_VERSION_NOTIFY
            } else if blobs.is_some() {
                JOURNAL_VERSION_BLOB
            } else if kv.is_some() {
                JOURNAL_VERSION_KV
            } else if schedules.as_ref().is_some_and(|declared| {
                declared
                    .iter()
                    .any(|schedule| schedule.interval_ms.is_some() || schedule.remaining != 1)
            }) {
                JOURNAL_VERSION_SCHEDULE_RECURRENCE
            } else if lease.is_some() {
                JOURNAL_VERSION_WRITE_LEASE
            } else if open_presence.is_some() {
                JOURNAL_VERSION_OPEN_PRESENCE
            } else if transfers.is_some()
                || events
                    .iter()
                    .any(|event| matches!(event.kind, WorldEventKind::EntityInput { .. }))
            {
                JOURNAL_VERSION_REGION_HANDOFF
            } else {
                JOURNAL_VERSION
            };
            base.max(budget_version(execution_profile.as_ref()))
        }
        JournalEntry::ExecutionProfile { profile } => {
            JOURNAL_VERSION_EXECUTION_PROFILE.max(budget_version(Some(profile)))
        }
        JournalEntry::BuilderInstalled { .. } => JOURNAL_VERSION_EXECUTION_PROFILE,
        JournalEntry::Issued {
            issued_at: Some(_), ..
        }
        | JournalEntry::Revoked {
            revoked_at: Some(_),
            ..
        } => JOURNAL_VERSION_JOIN_TIMES,
        _ => JOURNAL_VERSION,
    }
}

/// Durable, append-only record of a world's history.
///
/// The actor calls `append` before acknowledging or broadcasting a mutation.
/// If `append` fails, the world turns read-only: acknowledged history must
/// never get ahead of durable history.
pub trait WorldJournal: Send + 'static {
    /// Durably append one entry.
    ///
    /// # Errors
    ///
    /// Any I/O failure; the actor then refuses further mutations.
    fn append(&mut self, entry: &JournalEntry) -> Result<()>;

    /// Durably append a batch of entries in order (group commit, #8239):
    /// where the journal can, the batch lands with a single sync, and
    /// durability-before-ack applies to the batch as a whole - either
    /// every entry is durable or the batch has failed and none may be
    /// acknowledged. The default is the per-entry append loop, which is
    /// always correct, just slower.
    ///
    /// # Errors
    ///
    /// Any I/O failure; the actor then refuses further mutations.
    fn append_batch(&mut self, entries: &[JournalEntry]) -> Result<()> {
        for entry in entries {
            self.append(entry)?;
        }
        Ok(())
    }

    /// Capture the exact durable journal bytes and their header version.
    /// Implementations that do not own a readable journal may leave this
    /// unsupported; the replication feed is available only for durable
    /// journals that can provide an atomic snapshot from the world actor.
    fn replication_snapshot(&mut self) -> Result<JournalSnapshot> {
        bail!("this journal cannot provide a replication snapshot")
    }

    /// Capture a lightweight, readable source for the initial replication
    /// feed. Durable file journals should pin the file and its current length
    /// here, then defer reading the bytes until the network layer requests it.
    /// Journals that cannot provide a deferred source should leave this
    /// unsupported rather than materializing the full image in the actor.
    fn replication_snapshot_source(&mut self) -> Result<JournalSnapshotSource> {
        bail!("this journal cannot provide a deferred replication snapshot source")
    }

    /// Current journal header version, when the implementation can report it.
    fn replication_header_version(&self) -> Option<u32> {
        None
    }

    /// Durably append a group of entries as one all-or-nothing transaction.
    /// Journals that cannot guarantee atomicity must leave the default in
    /// place; an install requiring paired event and audit records will then
    /// fail closed instead of falling back to separate appends.
    ///
    /// # Errors
    ///
    /// Any I/O failure, or if this journal does not support atomic groups.
    fn append_transaction(&mut self, _entries: &[JournalEntry]) -> Result<()> {
        bail!("this journal does not support atomic entry transactions")
    }

    /// Atomically replace the whole journal with `entries` (a header, the
    /// participants and a [`JournalEntry::Checkpoint`]). Either the old or
    /// the new journal survives a crash, never a mixture.
    ///
    /// # Errors
    ///
    /// Journals that cannot compact (the default), or any I/O failure. After
    /// an error the journal must still be a complete record of the world.
    fn compact(&mut self, _entries: &[JournalEntry]) -> Result<()> {
        bail!("this journal cannot be compacted")
    }
}

/// A byte-exact, newline-terminated journal image captured by the actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalSnapshot {
    /// Version in the journal's `Created` header.
    pub header_version: u32,
    /// Exact JSONL bytes, including every entry's newline.
    pub bytes: Vec<u8>,
}

/// A captured snapshot reader. File-backed journals keep only a pinned file
/// handle and the captured byte length in the actor; the network layer reads
/// the bytes after the actor has resumed handling commands.
pub struct JournalSnapshotSource {
    /// Version in the journal's `Created` header.
    pub header_version: u32,
    /// Number of bytes belonging to this captured image.
    pub byte_len: u64,
    reader: Box<dyn Read + Send>,
}

impl std::fmt::Debug for JournalSnapshotSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JournalSnapshotSource")
            .field("header_version", &self.header_version)
            .field("byte_len", &self.byte_len)
            .finish_non_exhaustive()
    }
}

impl JournalSnapshotSource {
    pub(crate) fn from_reader<R>(header_version: u32, byte_len: u64, reader: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self {
            header_version,
            byte_len,
            reader: Box::new(reader),
        }
    }

    /// Read exactly the captured image. This consumes the source, so its file
    /// handle and the resulting byte buffer have one clear owner at a time.
    ///
    /// # Errors
    ///
    /// If the source cannot be read completely, is too large for this target,
    /// or does not end at a journal-entry boundary.
    pub fn read(self) -> Result<JournalSnapshot> {
        let capacity = usize::try_from(self.byte_len).context("journal snapshot is too large")?;
        let mut bytes = Vec::with_capacity(capacity);
        self.reader
            .take(self.byte_len)
            .read_to_end(&mut bytes)
            .context("read captured journal snapshot")?;
        ensure!(
            bytes.len() == capacity,
            "captured journal snapshot changed while reading"
        );
        ensure!(
            bytes.is_empty() || bytes.ends_with(b"\n"),
            "captured journal snapshot has an unterminated final entry"
        );
        Ok(JournalSnapshot {
            header_version: self.header_version,
            bytes,
        })
    }

    /// Scan the captured file for its first entry above the v1 payload bound
    /// without materializing the full journal image. The caller creates a
    /// separate source for this probe and retains its original source for the
    /// network transfer.
    ///
    /// Returns `(ordinal, encoded_size)` with a zero-based ordinal and size
    /// including the trailing newline.
    pub(crate) fn first_oversized_entry(self) -> Result<Option<(usize, usize)>> {
        let mut reader = self.reader.take(self.byte_len);
        let mut remaining = self.byte_len;
        let mut buffer = [0_u8; 8192];
        let mut line = Vec::new();
        let mut entry_size = 0_usize;
        let mut ordinal = 0_usize;
        let mut oversized = None;

        while remaining > 0 {
            let requested = usize::try_from(remaining.min(buffer.len() as u64))
                .context("journal snapshot scan length does not fit this target")?;
            let read = reader
                .read(&mut buffer[..requested])
                .context("scan captured journal snapshot")?;
            ensure!(read > 0, "captured journal snapshot changed while scanning");
            remaining -= read as u64;

            for byte in &buffer[..read] {
                line.push(*byte);
                entry_size = entry_size
                    .checked_add(1)
                    .context("journal entry size overflow while scanning")?;
                if *byte == b'\n' {
                    if entry_size > replication::MAX_REPLICATION_PAYLOAD_BYTES {
                        serde_json::from_slice::<JournalEntry>(&line).with_context(|| {
                            format!("oversized journal entry at ordinal {ordinal} is malformed")
                        })?;
                        if oversized.is_none() {
                            oversized = Some((ordinal, entry_size));
                        }
                    }
                    entry_size = 0;
                    ordinal = ordinal
                        .checked_add(1)
                        .context("journal entry ordinal overflow while scanning")?;
                    line.clear();
                }
            }
        }

        ensure!(
            entry_size == 0,
            "captured journal snapshot has an unterminated final entry"
        );
        Ok(oversized)
    }
}
