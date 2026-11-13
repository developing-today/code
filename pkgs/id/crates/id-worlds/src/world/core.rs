//! Durable world reconstruction and authority-bearing core state.
//!
//! Reconstruction is admission: validate each journal entry against the
//! verifier's state rebuilt from earlier admitted entries. Refuse the whole
//! restore on invalid input; never partially apply or silently repair it.
//! Guest snapshots contain program-owned state, not host authority: the host
//! capability ledger is reconstructed and checked separately, and guest
//! requests remain subject to that ledger.

use super::*;

use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

mod checkpoint_parts;

use checkpoint_parts::{
    CheckpointEventPartKind, PendingCheckpointParts, checkpoint_part_json_len,
    split_checkpoint_events, split_checkpoint_snapshot,
};

/// Encoded byte size of one admin-history event, or `None` when the event
/// cannot be serialized at all. Serialization failure is deliberate in the
/// accounting below (#4633): an unserializable event counts as
/// `usize::MAX` bytes, exactly as the previous rescan-every-time helper
/// treated it, so it forces the retained history to trim rather than
/// silently occupying unaccounted space.
fn admin_history_event_bytes(event: &WorldEvent) -> Option<usize> {
    serde_json::to_vec(event).ok().map(|bytes| bytes.len())
}

fn admin_history_bytes<'a>(events: impl IntoIterator<Item = &'a WorldEvent>) -> usize {
    events
        .into_iter()
        .map(|event| admin_history_event_bytes(event).unwrap_or(usize::MAX))
        .fold(0usize, usize::saturating_add)
}

/// Incrementally maintained byte accounting for the retained admin history
/// (#4633). `record_admin_history` used to re-serialize every retained
/// event on every accepted event; this ledger caches each event's encoded
/// size once, when the event is recorded (or once during restore, where a
/// one-shot scan is acceptable), and maintains the running total as events
/// are pushed and trimmed, so recording is amortized O(1) in the retained
/// history size.
///
/// Invariant: `sizes` holds one entry per retained admin-history event, in
/// the same order as `WorldCore::admin_history`. A `None` entry is an event
/// that failed to serialize; while any `None` is retained,
/// [`AdminHistoryByteLedger::effective_bytes`] is `usize::MAX`, matching
/// the legacy rescan semantics.
#[derive(Debug, Default)]
pub(crate) struct AdminHistoryByteLedger {
    sizes: VecDeque<Option<usize>>,
    serializable_bytes: usize,
    unserializable: usize,
}

impl AdminHistoryByteLedger {
    /// Start tracking one newly retained event of the given encoded size.
    pub(crate) fn track(&mut self, size: Option<usize>) {
        self.sizes.push_back(size);
        match size {
            Some(bytes) => {
                self.serializable_bytes = self.serializable_bytes.saturating_add(bytes);
            }
            None => {
                self.unserializable = self.unserializable.saturating_add(1);
            }
        }
    }

    /// Stop tracking the oldest retained event (it was trimmed).
    pub(crate) fn untrack_front(&mut self) {
        if let Some(size) = self.sizes.pop_front() {
            match size {
                Some(bytes) => {
                    self.serializable_bytes = self.serializable_bytes.saturating_sub(bytes);
                }
                None => {
                    self.unserializable = self.unserializable.saturating_sub(1);
                }
            }
        }
    }

    /// The byte total the trim loop compares against the cap: the running
    /// total, or `usize::MAX` while an unserializable event is retained.
    pub(crate) fn effective_bytes(&self) -> usize {
        if self.unserializable > 0 {
            usize::MAX
        } else {
            self.serializable_bytes
        }
    }

    /// Number of events currently tracked; equals the retained history len.
    pub(crate) fn tracked(&self) -> usize {
        self.sizes.len()
    }

    /// Rebuild the ledger for a restored history with a one-shot scan.
    fn rebuild<'a>(events: impl IntoIterator<Item = &'a WorldEvent>) -> Self {
        let mut ledger = Self::default();
        for event in events {
            ledger.track(admin_history_event_bytes(event));
        }
        ledger
    }
}

fn presence_after(events: &[WorldEvent]) -> BTreeSet<u64> {
    let mut present = BTreeSet::new();
    for event in events {
        if let WorldEventKind::System { event } = &event.kind
            && let Some((participant, joined)) = presence_change(event)
        {
            if joined {
                present.insert(participant);
            } else {
                present.remove(&participant);
            }
        }
    }
    present
}

pub(super) fn check_journal_version(version: u32, max_supported: u32) -> Result<()> {
    ensure!(
        (JOURNAL_VERSION..=max_supported).contains(&version),
        "unsupported world journal version {version}"
    );
    Ok(())
}

pub(super) fn validate_entity_ref(entity_ref: &str) -> Result<()> {
    ensure!(
        !entity_ref.trim().is_empty(),
        "entity reference must not be empty"
    );
    ensure!(
        entity_ref.len() <= 256,
        "entity reference exceeds 256 bytes"
    );
    ensure!(
        !entity_ref.chars().any(char::is_control),
        "entity reference contains a control character"
    );
    Ok(())
}

/// Mutable world state. Keep it behind one actor/mutex so event sequencing is serialized.
#[derive(Debug)]
pub struct WorldCore {
    pub(crate) world_id: String,
    pub(crate) limits: WorldLimits,
    /// The clock every time decision reads: schedules, ticks, expiries and
    /// the chat rate window. [`SystemClock`] by default; tests inject a
    /// [`ManualClock`] through [`WorldCore::new_with_clock`].
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) execution_profile: crate::world_limits::WorldExecutionProfile,
    pub(crate) next_participant_id: u64,
    pub(crate) sequence: u64,
    pub(crate) active_module_hash: Option<String>,
    pub(crate) active_module_provenance: Option<ModuleProvenance>,
    pub(crate) active_seed: u64,
    /// What this world's program may ask the server for.
    pub(crate) ledger: crate::world_caps::CapLedger,
    /// The per-world total memory budget ledger (#557): created with
    /// the default ceiling, and replaced by [`WorldCore::restore`]
    /// with the ledger measured over the journal (its limit is the
    /// last persisted profile budget, or the default ceiling).
    pub(crate) memory_budget: crate::world_limits::WorldMemoryBudget,
    /// This world's own capability ceiling. The ledger allows the intersection
    /// of this and the server's `limits.server_caps`.
    pub(crate) caps_ceiling: BTreeSet<String>,
    /// Pending `schedule.at` fires by id (#1355): each record carries its
    /// next due time and, for a recurring schedule, its interval and the
    /// fires remaining. A schedule occupies one slot however many fires
    /// remain.
    pub(crate) schedules: BTreeMap<String, PendingSchedule>,
    pub(crate) isolation: Isolation,
    pub(crate) cross_world_write: bool,
    pub(crate) cross_world_forced: bool,
    pub(crate) file_scope: FileScope,
    pub(crate) file_write_override: bool,
    /// Events committed since the journal last started from a checkpoint.
    pub(crate) since_checkpoint: u64,
    pub(crate) capabilities: HashMap<u64, CapabilityRecord>,
    /// Chat rate limiting as a consumer of the shared #2244 P0-A abuse
    /// evaluator: one frame bucket per participant (keyed by a
    /// participant-derived endpoint id), with the historical chat caps
    /// — a burst of `CHAT_RATE_BURST` refilling over `CHAT_RATE_WINDOW`.
    pub(crate) chat_abuse: crate::abuse::AbuseEvaluator,
    /// Monotonic origin used for chat bucket refill timestamps.
    pub(crate) chat_epoch: std::time::Instant,
    pub(crate) events: VecDeque<WorldEvent>,
    /// Bounded journal-order history for admin-authorized fixture export.
    pub(crate) admin_history: VecDeque<WorldEvent>,
    /// False once the oldest part of the journal history has been trimmed.
    pub(crate) admin_history_complete: bool,
    /// Cached encoded sizes + running byte total for `admin_history`
    /// (#4633); kept in lockstep with it by `record_admin_history` and
    /// `restore_admin_history`.
    pub(crate) admin_history_ledger: AdminHistoryByteLedger,
    /// Participants whose join the program has seen without a later leave.
    /// Unlike the actor's live-session refcount, this survives compaction so
    /// restore can deliver one leave for sessions lost with the process.
    pub(crate) open_presence: BTreeSet<u64>,
    pub(crate) directory: crate::directory::Directory,
    pub(crate) petnames: crate::petname::Petnames,
    /// Active region-seam handoffs this world is a side of (#3870 phase 1,
    /// design #2818 §3): export fences/payloads and import receipts,
    /// rebuilt by journal replay and preserved across compaction by the
    /// checkpoint's `transfers` field.
    pub(crate) handoff_transfers: Vec<HandoffTransferState>,
    /// The journal-resident native-spill write lease (design #2819,
    /// phase 2 of issue #3804): rebuilt by journal replay from the
    /// lease control entries and preserved across compaction by the
    /// checkpoint's `lease` field. Bootstrap (DO holder, epoch 0) for
    /// every world that has never spilled.
    pub(crate) write_lease: WriteLease,
    /// Author keys bound to participants for signed move capture (#2814):
    /// participant id to the player's ed25519 verifying key bytes. A bound
    /// participant's inputs must arrive player-signed (see
    /// [`signed`](super::signed)). Runtime state in this first pass;
    /// journal persistence of bindings is a follow-up.
    pub(crate) author_keys: HashMap<u64, [u8; 32]>,
    /// Each bound participant's expected signed-move chain position.
    pub(crate) move_chains: HashMap<u64, signed::MoveChain>,
    /// Accepted signed moves, in world sequence order.
    pub(crate) signed_moves: Vec<signed::SignedMoveRecord>,
    /// This core came from a verified journal and may only be served as a
    /// spectator replica. It is set only by `restore_replica`.
    pub(crate) replica_mode: bool,
}

/// The abuse-evaluator policy for world chat: the historical caps
/// (`CHAT_RATE_BURST` lines per `CHAT_RATE_WINDOW`) expressed as a
/// token bucket — same burst, same sustained rate.
fn chat_abuse_policy() -> crate::abuse::AbusePolicy {
    let limits = crate::abuse::BucketLimits {
        frames_per_sec: u64::from(CHAT_RATE_BURST) / CHAT_RATE_WINDOW.as_secs().max(1),
        frame_burst: u64::from(CHAT_RATE_BURST),
        bytes_per_sec: 0,
        byte_burst: 0,
    };
    crate::abuse::AbusePolicy {
        authorized: limits,
        pre_authorization: limits,
        max_connections_per_peer: usize::MAX,
    }
}

/// The evaluator endpoint key for one chat participant: the participant
/// id in the first eight bytes, zeroes elsewhere. World-local — these
/// keys never leave the world's own evaluator.
fn chat_endpoint(participant_id: u64) -> crate::abuse::EndpointId {
    let mut endpoint = [0_u8; 32];
    endpoint[..8].copy_from_slice(&participant_id.to_be_bytes());
    endpoint
}

/// Validate the invariants shared by standalone handoff entries and
/// checkpoint-carried transfer state. `prior_transfers` are the already
/// admitted records, so callers validate a checkpoint's list in list order.
/// The owner epoch an entity sits at after a transfer: an export leaves
/// at the epoch it records as prior (ownership departs at that epoch), an
/// import lands at its new epoch. A subsequent crossing for the same
/// entity must chain from exactly this epoch; anything else is a stale
/// ownership claim and must never supersede the recorded state (#3902).
fn owner_epoch_after(transfer: &HandoffTransferState) -> u64 {
    match transfer.direction {
        HandoffDirection::Export => transfer.prior_owner_epoch,
        HandoffDirection::Import => transfer
            .new_owner_epoch
            .unwrap_or(transfer.prior_owner_epoch),
    }
}

/// Enforce the epoch chain against the transfers already recorded for
/// the entity, if any. While the entity is local (its latest transfer is
/// an import), the next crossing must depart from exactly the epoch the
/// import landed at. While it is away (its latest transfer is an
/// export), ownership advances abroad - one step per crossing on other
/// shards - so a returning transfer may chain from any epoch at or
/// beyond the one the export left at, but never an older one: a stale
/// epoch would supersede the fence with an outdated ownership claim.
/// First crossings are unconstrained (the epoch originates with the
/// caller's ownership claim).
fn ensure_epoch_chains(
    transfer: &HandoffTransferState,
    prior_transfers: &[HandoffTransferState],
) -> Result<()> {
    if let Some(latest) = prior_transfers
        .iter()
        .rev()
        .find(|prior| prior.entity_ref == transfer.entity_ref)
    {
        let chained = match latest.direction {
            HandoffDirection::Export => transfer.prior_owner_epoch >= latest.prior_owner_epoch,
            HandoffDirection::Import => transfer.prior_owner_epoch == owner_epoch_after(latest),
        };
        ensure!(
            chained,
            "handoff {:?} claims prior owner epoch {} for entity {:?}, but the latest recorded transfer leaves it at epoch {}",
            transfer.handoff_id,
            transfer.prior_owner_epoch,
            transfer.entity_ref,
            owner_epoch_after(latest)
        );
    }
    Ok(())
}

fn validate_handoff_transfer(
    transfer: &HandoffTransferState,
    prior_transfers: &[HandoffTransferState],
) -> Result<()> {
    let name = match transfer.direction {
        HandoffDirection::Export => "export",
        HandoffDirection::Import => "import",
    };
    ensure!(
        transfer.event.sequence == 0 && transfer.event.participant_id == 0,
        "journal {name} event claims a sequence for handoff {:?}",
        transfer.handoff_id
    );
    match transfer.direction {
        HandoffDirection::Export => ensure!(
            transfer.new_owner_epoch.is_none(),
            "journal export for handoff {:?} carries a new owner epoch",
            transfer.handoff_id
        ),
        HandoffDirection::Import => ensure!(
            transfer.new_owner_epoch.is_some_and(|new_owner_epoch| {
                transfer.prior_owner_epoch.checked_add(1) == Some(new_owner_epoch)
            }),
            "journal import for handoff {:?} does not advance the owner epoch",
            transfer.handoff_id
        ),
    }
    ensure!(
        !prior_transfers.iter().any(|prior| {
            prior.handoff_id == transfer.handoff_id && prior.direction == transfer.direction
        }),
        "journal {name}s handoff {:?} twice",
        transfer.handoff_id
    );
    ensure_epoch_chains(transfer, prior_transfers)?;
    Ok(())
}

/// Verify a journaled destination event carries the import envelope for
/// `payload_digest` (#3870 phase 3, design #2818 \§3): the payload rides
/// hex-encoded inside the versioned system event, and its BLAKE3 digest
/// must match the digest the journal entry records \— a tampered or
/// mismatched envelope is refused at restore, never applied.
fn validate_import_envelope(event: &WorldEvent, payload_digest: &str) -> Result<()> {
    let text = match &event.kind {
        WorldEventKind::System { event } => event,
        _ => {
            return Err(anyhow::anyhow!(
                "journal import event is not a host system event"
            ));
        }
    };
    let envelope: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| anyhow::anyhow!("journal import event is not a transfer envelope"))?;
    ensure!(
        envelope["protocol"] == "world.entity-transfer.v1" && envelope["direction"] == "import",
        "journal import event is not an import envelope"
    );
    ensure!(
        envelope["payload_digest"] == payload_digest,
        "journal import envelope digest does not match the journal entry"
    );
    let payload_hex = envelope["payload_hex"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("journal import envelope carries no payload"))?;
    ensure!(
        payload_hex.len() % 2 == 0 && payload_hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "journal import envelope payload is not hex"
    );
    let payload: Vec<u8> = (0..payload_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&payload_hex[i..i + 2], 16))
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("journal import envelope payload is not hex"))?;
    ensure!(
        payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES,
        "journal import payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
    );
    ensure!(
        hex_encode(blake3::hash(&payload).as_bytes()) == payload_digest,
        "journal import payload digest does not match"
    );
    Ok(())
}

impl WorldCore {
    /// Create an empty in-memory world on the real [`SystemClock`].
    pub fn new(world_id: impl Into<String>, limits: WorldLimits) -> Result<Self> {
        Self::new_with_clock(world_id, limits, Arc::new(SystemClock))
    }

    /// Create an empty in-memory world that reads the time from `clock`.
    pub fn new_with_clock(
        world_id: impl Into<String>,
        limits: WorldLimits,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        let world_id = world_id.into();
        ensure!(!world_id.trim().is_empty(), "world ID must not be empty");
        ensure!(
            limits.participants > 0,
            "world participant limit must be nonzero"
        );
        ensure!(limits.chat_bytes > 0, "world chat limit must be nonzero");
        ensure!(limits.input_bytes > 0, "world input limit must be nonzero");
        ensure!(
            limits.retained_events > 0,
            "world event retention must be nonzero"
        );
        ensure!(
            limits.presentation_bytes > 0,
            "world presentation limit must be nonzero"
        );
        ensure!(
            limits.snapshot_bytes > 0,
            "world snapshot limit must be nonzero"
        );
        ensure!(
            limits.min_tick_ms > 0,
            "world minimum tick interval must be nonzero"
        );
        let server_caps: Vec<String> = limits.server_caps.iter().cloned().collect();
        crate::world_caps::validate_ceiling_names(&server_caps)
            .context("server capability ceiling")?;
        let grant_on_use = limits.grant_on_use;
        let chat_epoch = clock.now_instant();
        let mut core = Self {
            world_id,
            limits,
            clock,
            execution_profile: crate::world_limits::WorldExecutionProfile::default(),
            next_participant_id: 1,
            sequence: 0,
            active_module_hash: None,
            active_module_provenance: None,
            active_seed: 0,
            ledger: crate::world_caps::CapLedger::new(BTreeSet::new(), grant_on_use),
            memory_budget: crate::world_limits::WorldMemoryBudget::default(),
            caps_ceiling: crate::world_caps::default_ceiling_names(),
            schedules: BTreeMap::new(),
            isolation: Isolation::default(),
            cross_world_write: false,
            cross_world_forced: false,
            file_scope: FileScope::default(),
            file_write_override: false,
            since_checkpoint: 0,
            capabilities: HashMap::new(),
            chat_abuse: crate::abuse::AbuseEvaluator::new(chat_abuse_policy(), [0; 32]),
            chat_epoch,
            events: VecDeque::new(),
            admin_history: VecDeque::new(),
            admin_history_complete: true,
            admin_history_ledger: AdminHistoryByteLedger::default(),
            open_presence: BTreeSet::new(),
            directory: crate::directory::Directory::new(),
            petnames: crate::petname::Petnames::new(),
            handoff_transfers: Vec::new(),
            write_lease: WriteLease::bootstrap(),
            author_keys: HashMap::new(),
            move_chains: HashMap::new(),
            signed_moves: Vec::new(),
            replica_mode: false,
        };
        core.sync_ledger_ceiling();
        Ok(core)
    }

    /// The clock every time decision reads.
    pub(crate) fn clock(&self) -> &dyn Clock {
        &*self.clock
    }

    /// Recompute what the ledger may allow: the server's limit intersected with
    /// this world's ceiling. Call whenever either one changes.
    pub(crate) fn sync_ledger_ceiling(&mut self) {
        let allowed = self
            .limits
            .server_caps
            .intersection(&self.caps_ceiling)
            .cloned()
            .collect();
        self.ledger.set_allowed(allowed);
    }

    /// Rebuild a world from its journal.
    ///
    /// # Errors
    ///
    /// Fails if the journal belongs to another world, uses an unknown
    /// version, or is internally inconsistent (out-of-order sequences,
    /// unknown participants, malformed digests). A bad journal is refused,
    /// never partially applied.
    pub fn restore(
        world_id: impl Into<String>,
        limits: WorldLimits,
        entries: impl IntoIterator<Item = JournalEntry>,
    ) -> Result<RestoredWorld> {
        Self::restore_with_runtime(
            world_id,
            limits,
            crate::world_limits::RuntimeLimits::default(),
            entries,
        )
    }

    /// Rebuild a replica world from its verified journal (F12.13 phase 4).
    ///
    /// Same admission rules as [`Self::restore`], but presence-preserving:
    /// no synthetic presence-off events are fed into the program for
    /// participants the journal still shows present. That synthesis is
    /// correct for a writer restart (its sessions really ended) but wrong
    /// for a replica: a snapshot taken while participants remain connected
    /// to the writer would otherwise mark them offline in the replica's
    /// program state even though the journal is intact, breaking
    /// writer-record parity at the snapshot cursor. The replica restores
    /// presence as session state (its actor tracks live sessions) without
    /// journaling or replaying synthetic events.
    ///
    /// # Errors
    ///
    /// Same as [`Self::restore`].
    pub fn restore_replica(
        world_id: impl Into<String>,
        limits: WorldLimits,
        entries: impl IntoIterator<Item = JournalEntry>,
    ) -> Result<RestoredWorld> {
        Self::restore_replica_with_runtime(
            world_id,
            limits,
            crate::world_limits::RuntimeLimits::default(),
            entries,
        )
    }

    /// Like [`Self::restore_replica`], using the current server's resource
    /// ceilings while admitting the verified journal.
    pub fn restore_replica_with_runtime(
        world_id: impl Into<String>,
        limits: WorldLimits,
        runtime: crate::world_limits::RuntimeLimits,
        entries: impl IntoIterator<Item = JournalEntry>,
    ) -> Result<RestoredWorld> {
        let mut restored = Self::restore_inner(world_id, limits, runtime, entries, true)?;
        restored.core.replica_mode = true;
        Ok(restored)
    }

    /// Rebuild a world from its journal, resolving the persisted
    /// per-world total memory budget against the server ceiling in `runtime`.
    ///
    /// # Errors
    ///
    /// Same refusal contract as [`Self::restore`].
    pub fn restore_with_runtime(
        world_id: impl Into<String>,
        limits: WorldLimits,
        runtime: crate::world_limits::RuntimeLimits,
        entries: impl IntoIterator<Item = JournalEntry>,
    ) -> Result<RestoredWorld> {
        Self::restore_inner(world_id, limits, runtime, entries, false)
    }

    /// Shared restore; `preserve_presence` selects replica semantics.
    fn restore_inner(
        world_id: impl Into<String>,
        limits: WorldLimits,
        runtime: crate::world_limits::RuntimeLimits,
        entries: impl IntoIterator<Item = JournalEntry>,
        preserve_presence: bool,
    ) -> Result<RestoredWorld> {
        let mut core = Self::new(world_id, limits)?;
        let mut program = None;
        let mut replay = Vec::new();
        let mut present: BTreeMap<u64, u64> = BTreeMap::new();
        let entries: Vec<JournalEntry> = entries.into_iter().collect();
        // #557 Phase 2: enforce the per-world total memory budget at
        // restore. The effective limit resolves the journal's FINAL
        // persisted profile against the CURRENT server ceiling
        // (`runtime.total_memory_bytes`): replay replaces the whole
        // execution profile, so a later profile whose budget is unset
        // resets an earlier one — an unset budget inherits the current
        // ceiling, and a set budget is clamped beneath it. The
        // persisted request cannot be trusted to already be clamped:
        // `set_execution_profile` journals the unclamped request, so
        // resolving here is what makes lowering the server ceiling
        // constrain restore and raising it reopen a budgetless journal
        // written under a smaller ceiling. Charge the fixed core
        // charge plus each materialized entry (serialized length +
        // structural allowance) BEFORE applying any entry: an
        // over-budget journal is refused whole with a typed
        // [`crate::world_limits::WorldMemoryBudgetRefusal`], no state
        // is partially applied, and the durable journal is never
        // modified by a restore, so the same journal reopens unchanged
        // once the configured budget allows it. This charges the
        // restore/materialization seam only; runtime growth, program
        // reservations, and transient peaks are later #557 phases.
        let mut persisted_total: Option<u64> = None;
        for entry in &entries {
            match entry {
                JournalEntry::ExecutionProfile { profile } => {
                    persisted_total = profile.total_memory_bytes;
                }
                JournalEntry::Checkpoint {
                    execution_profile, ..
                } => {
                    persisted_total = execution_profile
                        .as_ref()
                        .and_then(|profile| profile.total_memory_bytes);
                }
                _ => {}
            }
        }
        let limit = persisted_total.map_or(runtime.total_memory_bytes, |total| {
            total.min(runtime.total_memory_bytes)
        });
        let mut memory_budget = crate::world_limits::WorldMemoryBudget::new(limit);
        memory_budget.try_charge(crate::world_limits::WORLD_MEMORY_BASE_BYTES)?;
        for entry in &entries {
            let serialized_len = serde_json::to_vec(entry).map_or(0, |bytes| bytes.len());
            memory_budget.try_charge(crate::world_limits::journal_entry_charge(serialized_len))?;
        }
        // A compacted journal writes a revoked parent before its descendants'
        // Issued/Revoked pairs. Remember descendants made unusable by that
        // earlier revocation so their later explicit Revoked entries remain
        // valid without allowing duplicate revocation entries.
        let mut inherited_revocations = HashSet::new();
        let mut entries = entries.into_iter();
        let journal_version = match entries.next() {
            Some(JournalEntry::Created { world_id, version }) => {
                ensure!(
                    world_id == core.world_id,
                    "journal belongs to world {world_id:?}, not {:?}",
                    core.world_id
                );
                check_journal_version(version, MAX_JOURNAL_VERSION)?;
                version
            }
            Some(_) => anyhow::bail!("world journal does not start with a header"),
            None => {
                core.memory_budget = memory_budget;
                return Ok(RestoredWorld {
                    core,
                    program,
                    replay,
                    presence: BTreeMap::new(),
                });
            }
        };
        let mut checkpoint_parts = PendingCheckpointParts::default();
        for entry in entries {
            if checkpoint_parts.has_parts()
                && !matches!(
                    &entry,
                    JournalEntry::CheckpointSnapshotPart { .. }
                        | JournalEntry::CheckpointEventsPart { .. }
                        | JournalEntry::CheckpointAdminPart { .. }
                        | JournalEntry::Checkpoint { .. }
                )
            {
                anyhow::bail!("checkpoint parts are not immediately followed by their checkpoint");
            }
            let required_version = journal_version_required_by(&entry);
            ensure!(
                journal_version >= required_version,
                "journal entry requires version {required_version}, but header is version {journal_version}"
            );
            match entry {
                JournalEntry::Created { .. } => anyhow::bail!("duplicate world journal header"),
                JournalEntry::Issued {
                    participant,
                    digest,
                    scopes,
                    parent,
                    expires_at,
                    uses,
                    used,
                    subject,
                } => {
                    let digest = hex_decode_32(&digest).context("malformed capability digest")?;
                    // #2972: participant 0 is the reserved host/system
                    // identity. Mint can never issue it (ids start at 1),
                    // so an Issued entry naming it is a forged host
                    // capability with a journal-chosen digest — refuse
                    // the whole restore before anything is applied.
                    ensure!(
                        participant.id != 0,
                        "participant 0 is the reserved host identity and cannot be issued"
                    );
                    // #2960: the journaled display name must be one mint
                    // could have produced. Mint sanitizes every name
                    // (control and format characters stripped, capped at
                    // 48 chars, trimmed) and refuses an empty result, so a
                    // name that is a fixed point of neither sanitization
                    // — or is empty — comes from a journal no live path
                    // can write; refuse the whole restore. The pre-#2112
                    // sanitizer (control characters only) is also accepted:
                    // journals minted before format stripping landed can
                    // legitimately carry ZWJ/ZWNJ names such as family
                    // emoji, and those worlds restored fine before this
                    // check existed. The pre-#4711 sanitizer (no trim
                    // after the 48-char cap) is also accepted: journals
                    // minted before the boundary fix can legitimately
                    // carry 48-char names ending in whitespace, and
                    // refusing those would brick worlds that minted
                    // fine under the rules of their time.
                    ensure!(
                        !participant.display_name.is_empty()
                            && (participant.display_name
                                == sanitize_display_name(&participant.display_name)
                                || participant.display_name
                                    == legacy_sanitize_display_name(&participant.display_name)
                                || is_pre_fix_mint_display_name(&participant.display_name)),
                        "participant {} has a display name mint could not have produced",
                        participant.id
                    );
                    ensure!(
                        !core.capabilities.contains_key(&participant.id),
                        "participant {} issued twice",
                        participant.id
                    );
                    let active = core
                        .capabilities
                        .values()
                        .filter(|record| !record.revoked)
                        .count();
                    ensure!(
                        active < core.limits.participants,
                        "participant {} exceeds world participant limit {}",
                        participant.id,
                        core.limits.participants
                    );
                    ensure!(
                        parent.is_none_or(|parent| core.capabilities.contains_key(&parent)),
                        "participant {} delegated from an unknown capability",
                        participant.id
                    );
                    ensure!(
                        uses.is_none_or(|uses| uses >= 1),
                        "participant {} has an empty use limit",
                        participant.id
                    );
                    if let Some(parent) = parent.and_then(|id| core.capabilities.get(&id)) {
                        ensure!(
                            expires_at.is_none_or(|expires| parent
                                .bounds
                                .expires_at
                                .is_none_or(|limit| expires <= limit)),
                            "participant {} outlives the capability it was delegated from",
                            participant.id
                        );
                    }
                    let mut ancestor = parent;
                    let mut revoked_by_ancestor = false;
                    while let Some(ancestor_id) = ancestor {
                        let ancestor_record = core
                            .capabilities
                            .get(&ancestor_id)
                            .context("delegated capability ancestor disappeared during restore")?;
                        if ancestor_record.revoked {
                            revoked_by_ancestor = true;
                            break;
                        }
                        ancestor = ancestor_record.parent;
                    }
                    // #2890: mint refuses scope bits from_bits rejects
                    // and scopes without JOIN; a journaled Issued entry
                    // must meet the same bar, or restore would resurrect
                    // a capability mint could never have issued.
                    let Some(validated_scopes) = WorldScopes::from_bits(scopes) else {
                        anyhow::bail!(
                            "participant {} journaled scopes {scopes:#04x} name no scope",
                            participant.id
                        );
                    };
                    ensure!(
                        validated_scopes.contains(WorldScopes::JOIN),
                        "participant {} journaled scopes must include join scope",
                        participant.id
                    );
                    core.next_participant_id = core.next_participant_id.max(
                        participant
                            .id
                            .checked_add(1)
                            .context("participant ID exhausted")?,
                    );
                    let issued_participant_id = participant.id;
                    core.capabilities.insert(
                        issued_participant_id,
                        CapabilityRecord {
                            digest,
                            participant,
                            scopes: validated_scopes,
                            parent,
                            revoked: revoked_by_ancestor,
                            bounds: CapabilityBounds { expires_at, uses },
                            used,
                            subject,
                        },
                    );
                    if revoked_by_ancestor {
                        inherited_revocations.insert(issued_participant_id);
                    }
                }
                JournalEntry::Revoked { participant_id, .. } => {
                    if !inherited_revocations.remove(&participant_id) {
                        ensure!(
                            !core.revoke_tree(participant_id).is_empty(),
                            "journal revokes unknown or revoked participant {participant_id}"
                        );
                    }
                }
                JournalEntry::CapsGranted { granted } => {
                    for name in &granted {
                        ensure!(
                            name == "*" || crate::world_caps::is_known(name),
                            "journal grants unknown capability {name:?}"
                        );
                    }
                    core.ledger.set_granted(granted.iter().cloned().collect());
                }
                JournalEntry::CapsCeiling { ceiling } => {
                    crate::world_caps::validate_ceiling_names(&ceiling)
                        .context("invalid capability ceiling in journal")?;
                    core.caps_ceiling = ceiling.into_iter().collect();
                }
                JournalEntry::ExecutionProfile { profile } => {
                    profile
                        .resolve(runtime)
                        .context("invalid world execution profile in journal")?;
                    core.execution_profile = profile;
                }
                JournalEntry::Isolation { isolation } => core.isolation = isolation,
                JournalEntry::CrossWorldWrite { enabled } => {
                    core.cross_world_write = enabled;
                    if !enabled {
                        core.cross_world_forced = false;
                    }
                }
                JournalEntry::CrossWorldWriteForced => {
                    core.cross_world_write = true;
                    core.cross_world_forced = true;
                }
                JournalEntry::FileScope { scope } => core.file_scope = scope,
                JournalEntry::FileWriteOverride { enabled } => core.file_write_override = enabled,
                JournalEntry::ScheduleSet {
                    id,
                    at_ms,
                    interval_ms,
                    remaining,
                } => {
                    crate::world_caps::validate_schedule_id(&id)
                        .context("invalid schedule id in journal")?;
                    ensure!(
                        !core.schedules.contains_key(&id),
                        "journal schedules {id:?} twice"
                    );
                    ensure!(
                        core.schedules.len() < crate::world_caps::MAX_PENDING_SCHEDULES,
                        "journal holds more than {} pending schedules",
                        crate::world_caps::MAX_PENDING_SCHEDULES
                    );
                    // Recurrence invariants the writer holds: at least one
                    // fire remaining, a positive interval when one is named,
                    // and an interval whenever more than one fire remains
                    // (a mid-series record legitimately pairs an interval
                    // with a single remaining fire).
                    let remaining = remaining.unwrap_or(1);
                    ensure!(
                        remaining >= 1,
                        "journal schedule {id:?} has no fires remaining"
                    );
                    if let Some(interval) = interval_ms {
                        ensure!(interval >= 1, "journal schedule {id:?} has a zero interval");
                    }
                    ensure!(
                        remaining == 1 || interval_ms.is_some(),
                        "journal schedule {id:?} has {remaining} fires but no interval"
                    );
                    // No clock here: the due time is what the request set, and
                    // a restore never judges it against now.
                    core.schedules.insert(
                        id,
                        PendingSchedule {
                            at_ms,
                            interval_ms,
                            remaining,
                        },
                    );
                }
                JournalEntry::ScheduleCancelled { id } => {
                    ensure!(
                        core.schedules.remove(&id).is_some(),
                        "journal cancels schedule {id:?}, which is not pending"
                    );
                }
                JournalEntry::ScheduleFired { id } => {
                    ensure!(
                        core.apply_schedule_fired(&id),
                        "journal fires schedule {id:?}, which is not pending"
                    );
                }
                JournalEntry::EntityExported {
                    handoff_id,
                    entity_ref,
                    source_region,
                    target_region,
                    prior_owner_epoch,
                    source_event,
                    payload_schema,
                    program_module_hash,
                    payload,
                    payload_digest,
                } => {
                    ensure!(
                        source_event.sequence == 0 && source_event.participant_id == 0,
                        "journal export event claims a sequence"
                    );
                    ensure!(
                        matches!(&source_event.kind, WorldEventKind::System { .. }),
                        "journal export event is not a host system event"
                    );
                    validate_entity_ref(&entity_ref)?;
                    ensure!(
                        payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES,
                        "journal export payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
                    );
                    ensure!(
                        payload_digest == hex_encode(blake3::hash(&payload).as_bytes()),
                        "journal export payload digest does not match"
                    );
                    ensure!(
                        !core.handoff_transfers.iter().any(|t| {
                            t.handoff_id == handoff_id && t.direction == HandoffDirection::Export
                        }) && !matches!(
                            core.latest_transfer(&entity_ref),
                            Some(latest) if latest.direction == HandoffDirection::Export
                        ),
                        "journal exports handoff {handoff_id:?} twice or entity {entity_ref:?} while an export is active"
                    );
                    let transfer = HandoffTransferState {
                        handoff_id,
                        entity_ref,
                        source_region,
                        target_region,
                        direction: HandoffDirection::Export,
                        prior_owner_epoch,
                        new_owner_epoch: None,
                        program_module_hash,
                        payload_schema,
                        payload_digest,
                        payload: Some(payload),
                        event: source_event.clone(),
                    };
                    validate_handoff_transfer(&transfer, &core.handoff_transfers)?;
                    core.handoff_transfers.push(transfer);
                    // Replay applies the ownership-changing event from the
                    // same durable fact (design #2818 §3).
                    replay.push(source_event);
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::EntityImported {
                    handoff_id,
                    entity_ref,
                    source_region,
                    target_region,
                    prior_owner_epoch,
                    new_owner_epoch,
                    program_module_hash,
                    payload_schema,
                    payload_digest,
                    destination_event,
                } => {
                    let transfer = HandoffTransferState {
                        handoff_id: handoff_id.clone(),
                        entity_ref: entity_ref.clone(),
                        source_region,
                        target_region,
                        direction: HandoffDirection::Import,
                        prior_owner_epoch,
                        new_owner_epoch: Some(new_owner_epoch),
                        program_module_hash,
                        payload_schema,
                        payload_digest: payload_digest.clone(),
                        payload: None,
                        event: destination_event.clone(),
                    };
                    validate_handoff_transfer(&transfer, &core.handoff_transfers)?;
                    validate_entity_ref(&entity_ref)?;
                    ensure!(
                        !matches!(
                            core.latest_transfer(&entity_ref),
                            Some(latest) if latest.direction == HandoffDirection::Import
                        ),
                        "journal imports entity {entity_ref:?} while an import is active"
                    );
                    validate_import_envelope(&destination_event, &payload_digest)?;
                    core.handoff_transfers.push(transfer);
                    replay.push(destination_event);
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseGranted {
                    holder,
                    epoch,
                    expires_at_ms,
                } => {
                    core.write_lease
                        .apply_grant(&holder, epoch, expires_at_ms)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseRenewed {
                    holder,
                    epoch,
                    expires_at_ms,
                } => {
                    core.write_lease
                        .apply_renew(&holder, epoch, expires_at_ms)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseHandedOff {
                    from,
                    to,
                    epoch,
                    expires_at_ms,
                } => {
                    core.write_lease
                        .apply_handoff(&from, &to, epoch, expires_at_ms)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseRevoked {
                    former_holder,
                    epoch,
                    reason,
                } => {
                    core.write_lease
                        .apply_revoke(&former_holder, epoch, &reason)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseWriteRejected {
                    writer,
                    presented_epoch,
                    operation_id,
                    observed_epoch,
                    reason,
                    received_at_ms,
                } => {
                    core.write_lease.apply_rejection(LeaseWriteRejection {
                        writer,
                        presented_epoch,
                        operation_id,
                        observed_epoch,
                        reason,
                        received_at_ms,
                    })?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::CheckpointSnapshotPart {
                    sequence,
                    part_index,
                    part_count,
                    bytes,
                } => checkpoint_parts.push_snapshot(sequence, part_index, part_count, bytes)?,
                JournalEntry::CheckpointEventsPart {
                    sequence,
                    part_index,
                    part_count,
                    events,
                } => checkpoint_parts.push_events(sequence, part_index, part_count, events)?,
                JournalEntry::CheckpointAdminPart {
                    sequence,
                    part_index,
                    part_count,
                    events,
                } => checkpoint_parts.push_admin(sequence, part_index, part_count, events)?,
                // Audit-only: the install is carried by the `Committed`
                // `ProgramInstalled` event; this entry records who ordered it
                // and changes no restored state.
                JournalEntry::BuilderInstalled { .. } => {}
                // Audit-only: the copied entries already reproduce the parent.
                JournalEntry::Forked { .. } => {}
                JournalEntry::System { event } => {
                    ensure!(
                        event.sequence == 0 && event.participant_id == 0,
                        "journal system event claims a sequence"
                    );
                    ensure!(
                        matches!(&event.kind, WorldEventKind::System { .. }),
                        "journal system entry has a non-system payload"
                    );
                    if let WorldEventKind::System { event: text } = &event.kind {
                        // Deferred form answers are system events so their
                        // values stay out of the public participant event
                        // stream. The authenticated submitter id in the
                        // event payload lets replay spend the same bounded
                        // capability use that the live actor spent.
                        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(text)
                            && payload["cap"] == "form.result"
                            && let Some(submitter) = payload
                                .get("participant_id")
                                .and_then(serde_json::Value::as_u64)
                            && submitter != 0
                        {
                            let mut attributed = event.clone();
                            attributed.participant_id = submitter;
                            core.validate_restored_event_author(&attributed, false)?;
                            core.ensure_uses_left(submitter)?;
                            core.spend(submitter);
                        }
                        if let Some((participant, joined)) = presence_change(text) {
                            ensure!(
                                core.capabilities.contains_key(&participant),
                                "journal presence event names an unknown participant"
                            );
                            // Replica restore also needs per-participant
                            // session counts (F12.13): `open_presence` records
                            // only that a participant is present, not how many
                            // sessions hold them there.
                            let count = present.entry(participant).or_insert(0);
                            if joined {
                                *count += 1;
                            } else {
                                *count = count.saturating_sub(1);
                            }
                            if *count == 0 {
                                present.remove(&participant);
                            }
                        }
                    }
                    replay.push(event.clone());
                    core.record_admin_history(event);
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::Committed { event } => {
                    ensure!(
                        core.sequence.checked_add(1) == Some(event.sequence),
                        "journal sequence jumps from {} to {}",
                        core.sequence,
                        event.sequence
                    );
                    core.validate_restored_event_author(&event, false)?;
                    match &event.kind {
                        WorldEventKind::ProgramInstalled {
                            module_hash,
                            seed,
                            provenance,
                            ..
                        } => {
                            core.active_module_hash = Some(module_hash.clone());
                            core.active_module_provenance = provenance.clone();
                            core.active_seed = *seed;
                            program = Some(RestoredProgram {
                                module_hash: module_hash.clone(),
                                seed: *seed,
                                snapshot: None,
                                provenance: provenance.clone(),
                            });
                            // Transfer receipts outlive the install (the
                            // dedupe ledger is retained whole), so their
                            // events survive the slate wipe too: the
                            // replacement program has not applied them,
                            // and dropping them would leave a stored
                            // receipt acknowledgable for a transfer the
                            // current program never saw (#3902). Rebuild
                            // the replay from the ledger instead of
                            // filtering the events already replayed: a
                            // transfer compacted into a checkpoint sits
                            // in the ledger without ever entering the
                            // replay, and filtering would lose it here.
                            replay = core
                                .handoff_transfers
                                .iter()
                                .map(|transfer| transfer.event.clone())
                                .collect();
                        }
                        WorldEventKind::Input(_) => replay.push(event.clone()),
                        WorldEventKind::EntityInput { entity_ref, .. } => {
                            ensure!(
                                !core.entity_is_fenced(entity_ref),
                                "journal accepts input for fenced entity {entity_ref:?}"
                            );
                            replay.push(event.clone());
                        }
                        WorldEventKind::System { .. } => anyhow::bail!(
                            "host system events must use the unsequenced system journal entry"
                        ),
                        WorldEventKind::Chat(_) => {}
                    }
                    core.commit_prepared(event)?;
                }
                JournalEntry::Checkpoint {
                    sequence,
                    module_hash,
                    seed,
                    provenance,
                    execution_profile,
                    caps_ceiling,
                    schedules,
                    transfers,
                    lease,
                    mut snapshot,
                    mut events,
                    mut admin_history,
                    open_presence,
                } => {
                    if checkpoint_parts.has_parts() {
                        let (assembled_snapshot, assembled_events, assembled_admin) =
                            checkpoint_parts.assemble(sequence)?;
                        ensure!(
                            snapshot.is_empty() && events.is_empty(),
                            "split checkpoint duplicates variable-size checkpoint data"
                        );
                        let complete = admin_history
                            .as_ref()
                            .filter(|history| history.events.is_empty())
                            .map(|history| history.complete)
                            .context("split checkpoint is missing admin-history completeness")?;
                        snapshot = assembled_snapshot;
                        events = assembled_events;
                        admin_history = Some(JournalAdminHistory {
                            events: assembled_admin,
                            complete,
                        });
                        checkpoint_parts = PendingCheckpointParts::default();
                    }
                    ensure!(
                        core.sequence == 0 && program.is_none(),
                        "journal checkpoint does not come first"
                    );
                    ensure!(
                        events.last().is_none_or(|last| last.sequence == sequence)
                            && events.windows(2).all(|pair| {
                                pair[0].sequence.checked_add(1) == Some(pair[1].sequence)
                            }),
                        "journal checkpoint events are out of order"
                    );
                    ensure!(
                        events.iter().all(|event| {
                            event.sequence > 0
                                && !matches!(&event.kind, WorldEventKind::System { .. })
                        }),
                        "journal checkpoint contains a host system event"
                    );
                    for event in &events {
                        core.validate_restored_event_author(event, true)?;
                    }
                    core.sequence = sequence;
                    core.events = events.into();
                    while core.events.len() > core.limits.retained_events {
                        core.events.pop_front();
                    }
                    let history_presence = admin_history
                        .as_ref()
                        .filter(|history| history.complete)
                        .map(|history| presence_after(&history.events));
                    if let Some(admin_history) = admin_history {
                        core.restore_admin_history(
                            admin_history.events,
                            admin_history.complete,
                            sequence,
                        )?;
                    } else {
                        // Older checkpoints retained only sequenced events, so
                        // they can be exported as a partial window but cannot
                        // claim to reproduce host-originated inputs.
                        let events = core.events.iter().cloned().collect();
                        core.restore_admin_history(events, false, sequence)?;
                    }
                    core.open_presence = if let Some(participants) = open_presence {
                        ensure!(
                            participants.len() <= MAX_CHECKPOINT_OPEN_PRESENCE,
                            "checkpoint has more open presences than the journal limit ({MAX_CHECKPOINT_OPEN_PRESENCE})"
                        );
                        let unique: BTreeSet<u64> = participants.iter().copied().collect();
                        ensure!(
                            unique.len() == participants.len(),
                            "checkpoint repeats an open-presence participant"
                        );
                        ensure!(
                            unique.iter().all(|id| core.capabilities.contains_key(id)),
                            "checkpoint open presence names an unknown participant"
                        );
                        if let Some(history_presence) = &history_presence {
                            ensure!(
                                &unique == history_presence,
                                "checkpoint open presence disagrees with complete admin history"
                            );
                        }
                        unique
                    } else {
                        // Complete legacy history recovers the exact set. If
                        // it is absent or partial, the snapshot may still have
                        // stale guest-visible presence and the missing joins
                        // cannot be inferred. Conservatively send one `left`
                        // for every known capability below. This can include
                        // guests who were not present, but removes stale seats
                        // without pretending the export history is complete.
                        match history_presence {
                            Some(present) => present,
                            None => {
                                ensure!(
                                    core.capabilities.len() <= MAX_CHECKPOINT_OPEN_PRESENCE,
                                    "legacy checkpoint has too many capabilities for safe presence cleanup ({MAX_CHECKPOINT_OPEN_PRESENCE})"
                                );
                                core.capabilities.keys().copied().collect()
                            }
                        }
                    };
                    ensure!(
                        core.open_presence.len() <= MAX_CHECKPOINT_OPEN_PRESENCE,
                        "checkpoint presence exceeds the journal limit ({MAX_CHECKPOINT_OPEN_PRESENCE})"
                    );
                    ensure!(
                        core.open_presence
                            .iter()
                            .all(|id| core.capabilities.contains_key(id)),
                        "checkpoint admin history presence names an unknown participant"
                    );
                    core.active_module_hash = Some(module_hash.clone());
                    core.active_module_provenance = provenance.clone();
                    core.active_seed = seed;
                    let execution_profile = execution_profile.unwrap_or_default();
                    execution_profile
                        .resolve(runtime)
                        .context("invalid world execution profile in checkpoint")?;
                    core.execution_profile = execution_profile;
                    // Absent means the default ceiling: older checkpoints and
                    // default ceilings omit the field. A provider registered
                    // since the checkpoint joins only if its manifest default
                    // opts it in — never by registration alone (#838).
                    core.caps_ceiling = match caps_ceiling {
                        Some(ceiling) => {
                            crate::world_caps::validate_ceiling_names(&ceiling)
                                .context("invalid capability ceiling in checkpoint")?;
                            ceiling.into_iter().collect()
                        }
                        None => crate::world_caps::default_ceiling_names(),
                    };
                    // A checkpoint comes first in its journal, so its
                    // transfer list is the whole active-handoff state.
                    let mut validated_transfers = Vec::new();
                    for transfer in transfers.unwrap_or_default() {
                        validate_handoff_transfer(&transfer, &validated_transfers)?;
                        validated_transfers.push(transfer);
                    }
                    core.handoff_transfers = validated_transfers;
                    // A checkpoint stands in for the journal it
                    // compacted, so its lease field is the whole lease
                    // state: validate the carried history reproduces
                    // the claimed state before adopting it, exactly as
                    // replaying the un-compacted lease entries would.
                    // Lease entries are never written before their
                    // checkpoint, so the lease here must still be the
                    // bootstrap state the checkpoint replaces.
                    if let Some(lease) = lease {
                        ensure!(
                            core.write_lease.is_bootstrap(),
                            "journal checkpoint carries a write lease after lease entries"
                        );
                        lease
                            .validate_history()
                            .context("invalid write lease in journal checkpoint")?;
                        core.write_lease = lease;
                    }
                    // The writer emits one `ScheduleSet` per pending schedule
                    // before the checkpoint, so the two must agree exactly.
                    let listed = schedules.unwrap_or_default();
                    let declared: BTreeMap<String, PendingSchedule> = listed
                        .iter()
                        .map(|schedule| {
                            (
                                schedule.id.clone(),
                                PendingSchedule {
                                    at_ms: schedule.at_ms,
                                    interval_ms: schedule.interval_ms,
                                    remaining: schedule.remaining,
                                },
                            )
                        })
                        .collect();
                    ensure!(
                        declared.len() == listed.len() && declared == core.schedules,
                        "journal checkpoint's pending schedules disagree with its schedule entries"
                    );
                    program = Some(RestoredProgram {
                        module_hash,
                        seed,
                        snapshot: Some(snapshot),
                        provenance,
                    });
                }
            }
        }
        ensure!(
            !checkpoint_parts.has_parts(),
            "checkpoint parts are not followed by their checkpoint"
        );
        // Sessions end with the process, and a crash skips their leave, so
        // anyone the journal still shows present is gone by now. Replicas
        // (F12.13) skip this synthesis: see `restore_replica`.
        if !preserve_presence {
            for participant in std::mem::take(&mut core.open_presence) {
                let name = core
                    .participant(participant)
                    .map_or_else(|| participant.to_string(), |p| p.display_name);
                replay.push(WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: presence_event(participant, &name, false),
                    },
                });
                if let Some(event) = replay.last() {
                    core.record_admin_history(event.clone());
                }
            }
        }
        let presence = if preserve_presence {
            present
        } else {
            BTreeMap::new()
        };
        core.memory_budget = memory_budget;
        core.sync_ledger_ceiling();
        Ok(RestoredWorld {
            core,
            program,
            replay,
            presence,
        })
    }

    /// Scopes a participant capability chain records in the journal
    /// itself: the intersection of every ancestor issued scope.
    /// Unlike `effective_scopes`, this never consults the world
    /// directory: friendship and ceilings are mutable current state,
    /// and restore runs before the directory is loaded, so historical
    /// events are judged against what the journal recorded when they
    /// were committed (Codex P1 finding on PR #4518).
    fn journaled_scopes(&self, participant_id: u64) -> Option<WorldScopes> {
        self.capabilities.get(&participant_id)?;
        let mut scopes = WorldScopes::ALL;
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            scopes = scopes.intersect(record.scopes.implied());
            current = record.parent;
        }
        Some(scopes)
    }

    /// Check that a journaled event could have been authored by the
    /// participant state rebuilt so far. For compacted checkpoint events,
    /// revocation timing is no longer represented: `checkpoint_entries`
    /// writes the current revoked flags before the retained historical events.
    /// Keep those events valid when their participant and scope are present;
    /// ordinary `Committed` entries retain enough journal order to reject an
    /// author revoked before that event.
    fn validate_restored_event_author(
        &self,
        event: &WorldEvent,
        checkpoint_event: bool,
    ) -> Result<()> {
        let participant_id = event.participant_id;
        if participant_id == 0 {
            return Ok(());
        }

        let record = self.capabilities.get(&participant_id).with_context(|| {
            format!(
                "journal event at sequence {} uses participant {participant_id} before it was issued",
                event.sequence
            )
        })?;
        ensure!(
            checkpoint_event || !record.revoked,
            "journal event at sequence {} is authored by participant {participant_id} after revocation",
            event.sequence
        );

        let required_scope = match &event.kind {
            WorldEventKind::Input(_) | WorldEventKind::EntityInput { .. } => {
                Some((WorldScopes::INPUT, "INPUT"))
            }
            WorldEventKind::Chat(_) => Some((WorldScopes::CHAT, "CHAT")),
            WorldEventKind::System { .. } | WorldEventKind::ProgramInstalled { .. } => None,
        };
        if let Some((scope, name)) = required_scope {
            ensure!(
                self.journaled_scopes(participant_id)
                    .is_some_and(|scopes| scopes.contains(scope)),
                "journal event at sequence {} uses participant {participant_id} without {name} scope",
                event.sequence
            );
        }
        Ok(())
    }

    /// Whether enough events piled up behind the last checkpoint to trim.
    pub(crate) const fn checkpoint_due(&self) -> bool {
        self.limits.checkpoint_every > 0
            && self.active_module_hash.is_some()
            && self.since_checkpoint >= self.limits.checkpoint_every
    }

    /// The journal that replaces the current one behind `snapshot`: a header,
    /// every participant (and revocation), then the checkpoint itself.
    pub(crate) fn checkpoint_entries(&self, snapshot: String) -> Result<Vec<JournalEntry>> {
        let module_hash = self
            .active_module_hash
            .clone()
            .context("no active program to checkpoint")?;
        ensure!(
            self.open_presence.len() <= MAX_CHECKPOINT_OPEN_PRESENCE,
            "open presence exceeds the checkpoint limit ({MAX_CHECKPOINT_OPEN_PRESENCE}); keep the journal untrimmed"
        );
        ensure!(
            self.open_presence
                .iter()
                .all(|id| self.capabilities.contains_key(id)),
            "open presence names an unknown participant; keep the journal untrimmed"
        );
        let mut ids: Vec<u64> = self.capabilities.keys().copied().collect();
        ids.sort_unstable();
        let mut entries = vec![JournalEntry::Created {
            world_id: self.world_id.clone(),
            version: JOURNAL_VERSION,
        }];
        for id in ids {
            entries.extend(self.issued_entry(id));
            if self
                .capabilities
                .get(&id)
                .is_some_and(|record| record.revoked)
            {
                entries.push(JournalEntry::Revoked {
                    participant_id: id,
                    by: None,
                });
            }
        }
        let granted: Vec<String> = self.ledger.granted().iter().cloned().collect();
        if !granted.is_empty() {
            entries.push(JournalEntry::CapsGranted { granted });
        }
        // The ceiling rides on the checkpoint, so compaction keeps it; the
        // default is omitted and reads back as the default ceiling.
        let caps_ceiling: Option<Vec<String>> = (self.caps_ceiling
            != crate::world_caps::default_ceiling_names())
        .then(|| self.caps_ceiling.iter().cloned().collect());
        if let Some(ceiling) = &caps_ceiling {
            // An older binary cannot read a `CapsCeiling` entry and refuses the
            // journal. Without it, the older binary would ignore the checkpoint
            // field and restore the whole catalog, widening authority silently.
            entries.push(JournalEntry::CapsCeiling {
                ceiling: ceiling.clone(),
            });
        }
        if self.isolation != Isolation::default() {
            entries.push(JournalEntry::Isolation {
                isolation: self.isolation,
            });
        }
        if self.cross_world_forced {
            entries.push(JournalEntry::CrossWorldWriteForced);
        } else if self.cross_world_write {
            entries.push(JournalEntry::CrossWorldWrite { enabled: true });
        }
        if self.file_scope != FileScope::default() {
            entries.push(JournalEntry::FileScope {
                scope: self.file_scope,
            });
        }
        if self.file_write_override {
            entries.push(JournalEntry::FileWriteOverride { enabled: true });
        }
        if self.execution_profile != crate::world_limits::WorldExecutionProfile::default() {
            entries.push(JournalEntry::ExecutionProfile {
                profile: self.execution_profile,
            });
        }
        // One entry per pending schedule, before the checkpoint, mirroring the
        // ceiling: an older binary cannot read `ScheduleSet` and refuses the
        // journal, instead of restoring without the schedules.
        for (id, schedule) in &self.schedules {
            entries.push(JournalEntry::ScheduleSet {
                id: id.clone(),
                at_ms: schedule.at_ms,
                interval_ms: schedule.interval_ms,
                remaining: (schedule.remaining != 1).then_some(schedule.remaining),
            });
        }
        let schedules: Option<Vec<CheckpointSchedule>> = (!self.schedules.is_empty()).then(|| {
            self.schedules
                .iter()
                .map(|(id, schedule)| CheckpointSchedule {
                    id: id.clone(),
                    at_ms: schedule.at_ms,
                    interval_ms: schedule.interval_ms,
                    remaining: schedule.remaining,
                })
                .collect()
        });
        // Active handoffs ride on the checkpoint, mirroring the ceiling and
        // schedules: without them compaction could resurrect an exported
        // entity, forget an import receipt, or drop an owner epoch.
        let transfers: Option<Vec<HandoffTransferState>> =
            (!self.handoff_transfers.is_empty()).then(|| self.handoff_transfers.clone());
        // Never compact a lease history that would fail restore
        // validation into the replacement journal: refuse the
        // compaction and keep the un-compacted journal instead
        // (Codex review on PR #5571).
        if !self.write_lease.is_bootstrap() {
            self.write_lease
                .validate_history()
                .context("write lease history does not validate; keep the journal untrimmed")?;
        }
        let checkpoint_events: Vec<WorldEvent> = self.events.iter().cloned().collect();
        let checkpoint_admin_events: Vec<WorldEvent> = self.admin_history.iter().cloned().collect();
        // A checkpoint that already fits the per-entry bound stays a single
        // inline `Checkpoint` entry, exactly as before this change: its
        // journal keeps the version its contents already require, and older
        // binaries keep reading it. Only a checkpoint whose assembled
        // encoding exceeds the bound is split into part entries (which bump
        // the journal to version 6 via `journal_version_required_by`).
        let make_checkpoint =
            |snapshot: String, events: Vec<WorldEvent>, admin_events: Vec<WorldEvent>| {
                JournalEntry::Checkpoint {
                    sequence: self.sequence,
                    module_hash: module_hash.clone(),
                    seed: self.active_seed,
                    provenance: self.active_module_provenance.clone(),
                    execution_profile: Some(self.execution_profile),
                    caps_ceiling: caps_ceiling.clone(),
                    schedules: schedules.clone(),
                    transfers: transfers.clone(),
                    lease: (!self.write_lease.is_bootstrap()).then(|| self.write_lease.clone()),
                    snapshot,
                    events,
                    admin_history: Some(JournalAdminHistory {
                        events: admin_events,
                        complete: self.admin_history_complete,
                    }),
                    // `Some(empty)` means the writer knows no participants are
                    // open; `None` is reserved for legacy checkpoints whose
                    // state is unknown.
                    open_presence: Some(self.open_presence.iter().copied().collect()),
                }
            };
        let inline_checkpoint = make_checkpoint(
            snapshot.clone(),
            checkpoint_events.clone(),
            checkpoint_admin_events.clone(),
        );
        if checkpoint_part_json_len(&inline_checkpoint)? <= MAX_CHECKPOINT_PART_BYTES {
            entries.push(inline_checkpoint);
        } else {
            entries.extend(split_checkpoint_snapshot(self.sequence, &snapshot)?);
            entries.extend(split_checkpoint_events(
                self.sequence,
                &checkpoint_events,
                CheckpointEventPartKind::Retained,
            )?);
            entries.extend(split_checkpoint_events(
                self.sequence,
                &checkpoint_admin_events,
                CheckpointEventPartKind::Admin,
            )?);
            // The variable-size fields ride in the bounded part entries
            // immediately above; empty placeholders keep the scalar
            // checkpoint schema stable while older checkpoints remain
            // readable through the legacy no-parts path in restore.
            let checkpoint = make_checkpoint(String::new(), Vec::new(), Vec::new());
            ensure!(
                checkpoint_part_json_len(&checkpoint)? <= MAX_CHECKPOINT_PART_BYTES,
                "checkpoint scalar fields exceed the encoded size limit ({MAX_CHECKPOINT_PART_BYTES} bytes); keep the journal untrimmed"
            );
            entries.push(checkpoint);
        }
        // The compacted journal keeps the rollback contract (see
        // `JOURNAL_VERSION`): when the checkpoint re-emits an entry a
        // version-1 binary cannot read, the new header must say so, or the
        // old binary would accept the header and fail mid-read.
        let required = entries
            .iter()
            .map(journal_version_required_by)
            .max()
            .unwrap_or(JOURNAL_VERSION);
        if let Some(JournalEntry::Created { version, .. }) = entries.first_mut() {
            *version = required;
        }
        Ok(entries)
    }

    /// The world's active (non-revoked) participants.
    #[must_use]
    pub(crate) fn participants(&self) -> Vec<Participant> {
        self.capabilities
            .values()
            .filter(|record| !record.revoked)
            .map(|record| record.participant.clone())
            .collect()
    }

    /// One participant, whether revoked or not.
    #[must_use]
    pub(crate) fn participant(&self, id: u64) -> Option<Participant> {
        self.capabilities
            .get(&id)
            .map(|record| record.participant.clone())
    }

    /// Commit a host-authored chat line as participant 0.
    ///
    /// # Errors
    ///
    /// Fails when the text is empty, oversized, or entirely sanitized away.
    pub(crate) fn prepare_system_chat(&self, text: &str) -> Result<WorldEvent> {
        ensure!(
            text.len() <= self.limits.chat_bytes,
            "chat message exceeds {} bytes",
            self.limits.chat_bytes
        );
        let text = sanitize_chat(text);
        ensure!(!text.is_empty(), "chat message must not be empty");
        Ok(WorldEvent {
            sequence: self
                .sequence
                .checked_add(1)
                .context("world event sequence exhausted")?,
            participant_id: 0,
            kind: WorldEventKind::Chat(text),
        })
    }

    /// Whether a `schedule.at` for `id` may be pended, given the host clock
    /// `now_ms`. The error says why; the program sees only `bad_args`.
    ///
    /// # Errors
    ///
    /// A malformed id, a duplicate pending id, a full schedule, or a time past
    /// the horizon.
    pub(crate) fn check_schedule(&self, id: &str, at_ms: u64, now_ms: u64) -> Result<()> {
        self.check_schedule_inner(id, at_ms, None, None, now_ms, false)
    }

    /// Whether a recurring `schedule.at` (#1355) may be pended: the one-shot
    /// checks, the recurrence pair's own bounds, and the whole series —
    /// first fire plus `interval * (count - 1)` — inside the horizon.
    pub(crate) fn check_schedule_recurring(
        &self,
        id: &str,
        at_ms: u64,
        interval_ms: Option<u64>,
        count: Option<u32>,
        now_ms: u64,
    ) -> Result<()> {
        self.check_schedule_inner(id, at_ms, interval_ms, count, now_ms, false)
    }

    /// Apply one fire of a pending schedule: re-arm it in place while
    /// fires remain (fixed-rate: the next due time advances by the
    /// interval from the planned time, never from now), or consume it.
    /// Returns false when no schedule by that id is pending. Shared by
    /// journal replay and the actor's fire path, so both derive the same
    /// next state from the same journaled state.
    pub(crate) fn apply_schedule_fired(&mut self, id: &str) -> bool {
        let Some(schedule) = self.schedules.get(id).copied() else {
            return false;
        };
        match schedule.after_fire() {
            Some(rearmed) => {
                self.schedules.insert(id.to_owned(), rearmed);
            }
            None => {
                self.schedules.remove(id);
            }
        }
        true
    }

    /// The pending schedules as id to next due time: the projection the
    /// admin layer has always returned, whatever a schedule's recurrence.
    pub(crate) fn schedule_times(&self) -> BTreeMap<String, u64> {
        self.schedules
            .iter()
            .map(|(id, schedule)| (id.clone(), schedule.at_ms))
            .collect()
    }

    /// Like [`Self::check_schedule`], but for a set that replaces the pending
    /// schedule with the same id. Whether this is a replacement is derived
    /// from this core's schedule map: the entry being replaced counts as
    /// already cancelled, so it is neither a duplicate nor a use of capacity.
    /// Callers validate with this BEFORE persisting the replacement's
    /// cancellation, so a refused replacement (say, past the horizon) leaves
    /// the existing schedule standing (#807).
    pub(crate) fn check_schedule_replacing(&self, id: &str, at_ms: u64, now_ms: u64) -> Result<()> {
        let replacing = self.schedules.contains_key(id);
        self.check_schedule_inner(id, at_ms, None, None, now_ms, replacing)
    }

    fn check_schedule_inner(
        &self,
        id: &str,
        at_ms: u64,
        interval_ms: Option<u64>,
        count: Option<u32>,
        now_ms: u64,
        replacing: bool,
    ) -> Result<()> {
        crate::world_caps::validate_schedule_id(id)?;
        crate::world_caps::validate_schedule_recurrence(interval_ms, count)?;
        ensure!(
            replacing || !self.schedules.contains_key(id),
            "schedule {id:?} is already pending; cancel it before scheduling it again"
        );
        let pending = self.schedules.len() - usize::from(replacing);
        ensure!(
            pending < crate::world_caps::MAX_PENDING_SCHEDULES,
            "world already has {} pending schedules",
            crate::world_caps::MAX_PENDING_SCHEDULES
        );
        let horizon = now_ms.saturating_add(crate::world_caps::MAX_SCHEDULE_HORIZON_MS);
        ensure!(
            at_ms <= horizon,
            "schedule {id:?} is more than {} days past the host clock",
            crate::world_caps::MAX_SCHEDULE_HORIZON_DAYS
        );
        if let (Some(interval), Some(count)) = (interval_ms, count) {
            // The series' last fire is as much a schedule time as its
            // first: it must land inside the same horizon.
            let last_fire = interval
                .checked_mul(u64::from(count - 1))
                .and_then(|span| at_ms.checked_add(span));
            ensure!(
                last_fire.is_some_and(|last| last <= horizon),
                "schedule {id:?} series ends more than {} days past the host clock",
                crate::world_caps::MAX_SCHEDULE_HORIZON_DAYS
            );
        }
        Ok(())
    }

    /// Journal entry for an already-issued capability.
    pub(crate) fn issued_entry(&self, participant_id: u64) -> Option<JournalEntry> {
        self.capabilities
            .get(&participant_id)
            .map(|record| JournalEntry::Issued {
                participant: record.participant.clone(),
                digest: hex_encode(&record.digest),
                scopes: record.scopes.bits(),
                parent: record.parent,
                expires_at: record.bounds.expires_at,
                uses: record.bounds.uses,
                used: record.used,
                subject: record.subject.clone(),
            })
    }

    /// Latest committed event sequence.
    #[must_use]
    pub const fn current_sequence(&self) -> u64 {
        self.sequence
    }

    /// The world's identifier.
    #[must_use]
    pub fn world_id(&self) -> &str {
        &self.world_id
    }

    /// Active region-seam handoffs this world is a side of (#3870 phase 1).
    #[must_use]
    pub(crate) fn handoff_transfers(&self) -> &[HandoffTransferState] {
        &self.handoff_transfers
    }

    /// The world's journal-resident native-spill write lease
    /// (design #2819, phase 2 of issue #3804).
    #[must_use]
    pub(crate) fn write_lease(&self) -> &WriteLease {
        &self.write_lease
    }

    /// The per-world resource requests, independent of current server ceilings.
    /// The world's total-memory budget ledger (#557): the effective
    /// limit, the logical charge measured at restore, its high-water
    /// mark, and the refusal count. Logical accounting, not RSS.
    pub const fn memory_budget(&self) -> &crate::world_limits::WorldMemoryBudget {
        &self.memory_budget
    }

    pub const fn execution_profile(&self) -> crate::world_limits::WorldExecutionProfile {
        self.execution_profile
    }

    /// Issue a capability for a participant. Only `JOIN`-scoped capabilities
    /// count toward active participant capacity.
    pub fn issue_capability(
        &mut self,
        display_name: &str,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
    ) -> Result<(Participant, JoinCapability)> {
        self.mint(display_name, scopes, None, None, bounds)
    }

    /// Issue a capability that acts for the account `credential` belongs to.
    /// Its scopes may not exceed that account's ceiling.
    pub fn issue_for_account(
        &mut self,
        credential: &str,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
    ) -> Result<(Participant, JoinCapability)> {
        let account = self
            .directory
            .account_for(credential)
            .context("invalid account credential")?;
        ensure!(
            self.directory.ceiling(&account).implied().contains(scopes),
            "this account may not hold those scopes"
        );
        self.mint(display_name, scopes, None, Some(account), bounds)
    }

    /// Delegate a capability: a new participant whose scopes are a subset of
    /// the delegating capability's, and which is revoked with it. Its expiry
    /// may not be later than the delegator's. It carries no use count: each
    /// action it takes counts against the host-issued limit above it, if any.
    pub fn attenuate(
        &mut self,
        token: &JoinCapability,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
    ) -> Result<(Participant, JoinCapability)> {
        self.delegate(token, scopes, bounds, display_name, None)
    }

    /// Delegate to a friend of the account this capability acts for. The
    /// friend's ceiling must already hold the scopes, and the friendship must
    /// still hold each time the capability is used.
    pub fn delegate_to_friend(
        &mut self,
        token: &JoinCapability,
        friend: &str,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
    ) -> Result<(Participant, JoinCapability)> {
        let parent = self.authorize(token, WorldScopes::JOIN)?;
        let owner = self
            .capabilities
            .get(&parent)
            .and_then(|record| record.subject.clone())
            .context("only an account's capability can delegate to a friend")?;
        ensure!(
            self.directory.are_friends(&owner, friend),
            "you can only delegate to a friend"
        );
        ensure!(
            self.directory.ceiling(friend).implied().contains(scopes),
            "that friend may not hold those scopes"
        );
        self.delegate(token, scopes, bounds, display_name, Some(friend.to_owned()))
    }

    pub(crate) fn delegate(
        &mut self,
        token: &JoinCapability,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
        subject: Option<String>,
    ) -> Result<(Participant, JoinCapability)> {
        let parent = self.authorize(token, WorldScopes::JOIN)?;
        let parent_scopes = self
            .effective_scopes(parent)
            .context("world participant is unavailable")?;
        let parent_record = self
            .capabilities
            .get(&parent)
            .context("world participant is unavailable")?;
        let scopes = parent_scopes
            .attenuate(scopes)
            .context("this capability may not delegate those scopes")?;
        if let Some(expires_at) = bounds.expires_at {
            ensure!(
                expires_at > self.clock().now_ms(),
                "delegated expiry must be in the future"
            );
            ensure!(
                parent_record
                    .bounds
                    .expires_at
                    .is_none_or(|limit| expires_at <= limit),
                "this capability may not delegate a later expiry"
            );
        }
        ensure!(
            bounds.uses.is_none(),
            "a delegated capability carries no use count; the host's limit still applies"
        );
        self.mint(display_name, scopes, Some(parent), subject, bounds)
    }

    /// What a capability may do right now: its own scopes, held to the ceiling
    /// of each account in its chain. A delegation to a friend stops once the
    /// friendship ends.
    pub(crate) fn effective_scopes(&self, participant_id: u64) -> Option<WorldScopes> {
        self.effective_scopes_seeded(participant_id, WorldScopes::ALL)
    }

    /// [`Self::effective_scopes`] with a different starting set. Seeding
    /// with [`WorldScopes::all_recognized`] keeps admin-issued authority
    /// bits (`BUILDER` / `MODERATOR`) visible to the intersection; the
    /// public [`Self::effective_scopes`] seeds with [`WorldScopes::ALL`],
    /// which deliberately excludes them, so no ceiling, delegation, or
    /// projection path can ever surface them.
    fn effective_scopes_seeded(
        &self,
        participant_id: u64,
        seed: WorldScopes,
    ) -> Option<WorldScopes> {
        self.capabilities.get(&participant_id)?;
        let mut scopes = seed;
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            scopes = scopes.intersect(record.scopes.implied());
            if let Some(subject) = &record.subject {
                scopes = scopes.intersect(self.directory.ceiling(subject).implied());
            }
            let parent = record.parent.and_then(|id| self.capabilities.get(&id));
            if let Some(subject) = &record.subject
                && let Some(parent_subject) = parent.and_then(|p| p.subject.as_ref())
                && subject != parent_subject
                && !self.directory.are_friends(parent_subject, subject)
            {
                return Some(WorldScopes::EMPTY);
            }
            current = record.parent;
        }
        if self.replica_mode {
            Some(scopes.intersect(WorldScopes::JOIN))
        } else {
            Some(scopes)
        }
    }

    /// Fewest actions the capability or any ancestor may still take, or `None`
    /// if no capability in the chain is limited.
    pub(crate) fn uses_left(&self, participant_id: u64) -> Option<u32> {
        let mut left = None;
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            if let Some(limit) = record.bounds.uses {
                let remaining = limit.saturating_sub(record.used);
                left = Some(left.map_or(remaining, |so_far: u32| so_far.min(remaining)));
            }
            current = record.parent;
        }
        left
    }

    pub(crate) fn ensure_not_expired(&self, participant_id: u64) -> Result<()> {
        let now = self.clock().now_ms();
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            if !record.bounds.expires_at.is_none_or(|expires| now < expires) {
                return Err(CapabilityDenied::new("world capability has expired").into());
            }
            current = record.parent;
        }
        Ok(())
    }

    pub(crate) fn ensure_uses_left(&self, participant_id: u64) -> Result<()> {
        ensure!(
            self.uses_left(participant_id).is_none_or(|left| left > 0),
            "world capability has no uses left"
        );
        Ok(())
    }

    /// Count one committed action against the participant and each ancestor.
    pub(crate) fn spend(&mut self, participant_id: u64) {
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get_mut(&id)) {
            record.used = record.used.saturating_add(1);
            current = record.parent;
        }
    }

    pub(crate) fn mint(
        &mut self,
        display_name: &str,
        scopes: WorldScopes,
        parent: Option<u64>,
        subject: Option<String>,
        bounds: CapabilityBounds,
    ) -> Result<(Participant, JoinCapability)> {
        let active = self
            .capabilities
            .values()
            .filter(|record| !record.revoked)
            .count();
        ensure!(
            active < self.limits.participants,
            "world participant limit reached"
        );
        ensure!(
            scopes.contains(WorldScopes::JOIN),
            "world capability must include join scope"
        );
        let display_name = sanitize_display_name(display_name);
        ensure!(
            !display_name.is_empty(),
            "participant display name must not be empty"
        );
        let id = self.next_participant_id;
        self.next_participant_id = self
            .next_participant_id
            .checked_add(1)
            .context("participant ID exhausted")?;
        let mut secret = [0_u8; 32];
        rand::rng().fill(&mut secret);
        let token = format!("{id}.{}", hex_encode(&secret));
        let digest = token_digest(&token);
        let participant = Participant { id, display_name };
        self.capabilities.insert(
            id,
            CapabilityRecord {
                digest,
                participant: participant.clone(),
                scopes,
                parent,
                revoked: false,
                bounds,
                used: 0,
                subject,
            },
        );
        Ok((participant, JoinCapability(token)))
    }

    /// Revoke a capability and every capability delegated from it. Returns
    /// the participants newly revoked; empty if none was active.
    pub fn revoke_tree(&mut self, participant_id: u64) -> Vec<u64> {
        let mut revoked = Vec::new();
        let mut pending = vec![participant_id];
        while let Some(id) = pending.pop() {
            let Some(record) = self.capabilities.get_mut(&id) else {
                continue;
            };
            if record.revoked {
                continue;
            }
            record.revoked = true;
            revoked.push(id);
            pending.extend(
                self.capabilities
                    .values()
                    .filter(|child| child.parent == Some(id))
                    .map(|child| child.participant.id),
            );
        }
        revoked
    }

    /// The world's isolation.
    #[must_use]
    pub const fn isolation(&self) -> Isolation {
        self.isolation
    }

    /// The world's sharing and file settings.
    #[must_use]
    pub const fn policy(&self) -> WorldPolicy {
        WorldPolicy {
            isolation: self.isolation,
            cross_world_write: self.cross_world_write,
            cross_world_forced: self.cross_world_forced,
            file_scope: self.file_scope,
            file_write_override: self.file_write_override,
        }
    }

    /// The directory of accounts, groups and friends this world is held to.
    #[must_use]
    pub const fn directory(&self) -> &crate::directory::Directory {
        &self.directory
    }

    /// The directory of accounts, groups and friends this world is held to.
    pub const fn directory_mut(&mut self) -> &mut crate::directory::Directory {
        &mut self.directory
    }

    /// The directory and the private petnames, borrowed together.
    pub const fn directory_and_petnames_mut(
        &mut self,
    ) -> (
        &mut crate::directory::Directory,
        &mut crate::petname::Petnames,
    ) {
        (&mut self.directory, &mut self.petnames)
    }

    /// Replace the petname store, as the world's data directory is opened.
    pub fn set_petnames(&mut self, petnames: crate::petname::Petnames) {
        self.petnames = petnames;
    }

    /// Commit a chat message after validating capability and bounds.
    pub fn chat(&mut self, token: &JoinCapability, text: &str) -> Result<WorldEvent> {
        let event = self.prepare_chat(token, text)?;
        self.commit_prepared(event)
    }

    pub(crate) fn prepare_chat(
        &mut self,
        token: &JoinCapability,
        text: &str,
    ) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::CHAT)?;
        self.ensure_uses_left(participant_id)?;
        ensure!(
            text.len() <= self.limits.chat_bytes,
            "chat message exceeds {} bytes",
            self.limits.chat_bytes
        );
        let text = sanitize_chat(text);
        ensure!(!text.is_empty(), "chat message must not be empty");
        // Consume one token from the participant's shared-bucket frame
        // allowance; a refusal is the structured #2244 refusal (reason +
        // bounded retry value), not prose.
        let now_ms = u64::try_from(
            self.clock()
                .now_instant()
                .saturating_duration_since(self.chat_epoch)
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        self.chat_abuse.prune_idle_endpoints(now_ms);
        self.chat_abuse.prune_counters(now_ms, 60 * 60 * 1_000);
        self.chat_abuse
            .check_frame(chat_endpoint(participant_id), true, now_ms)
            .map_err(crate::abuse::RefusalError::from)?;
        // Prepare only: the caller journals the event and commit_prepared
        // applies it (chat() above does exactly that). Main's version of this
        // body ended in self.commit(...); merged into prepare_chat that
        // double-applied the event and commit_prepared refused it as stale.
        Ok(WorldEvent {
            sequence: self
                .sequence
                .checked_add(1)
                .context("world event sequence exhausted")?,
            participant_id,
            kind: WorldEventKind::Chat(text),
        })
    }

    /// Commit an opaque game input after validating capability and bounds.
    pub fn input(&mut self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let event = self.prepare_input(token, input)?;
        self.commit_prepared(event)
    }

    /// Bind `author_key` (the player's ed25519 verifying key) to the
    /// participant the capability names, opting that participant into
    /// signed play (#2814): from now on its inputs must arrive as
    /// player-signed moves via [`WorldCore::input_signed`], and the
    /// unsigned path refuses them. Re-binding the same key is a no-op;
    /// binding a different key is refused (key rotation is out of scope).
    ///
    /// # Errors
    ///
    /// Fails if the capability does not authorize input, or a different
    /// key is already bound to the participant.
    pub fn bind_author_key(&mut self, token: &JoinCapability, author_key: [u8; 32]) -> Result<u64> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        if let Some(bound) = self.author_keys.get(&participant_id) {
            ensure!(
                *bound == author_key,
                "participant {participant_id} already has a different author key bound"
            );
        } else {
            self.author_keys.insert(participant_id, author_key);
            self.move_chains.entry(participant_id).or_default();
        }
        Ok(participant_id)
    }

    /// Validate a player-signed input and prepare its event, verifying
    /// the signed move against the participant's bound key and current
    /// chain position without advancing the chain (see
    /// [`WorldCore::record_signed_move`]).
    ///
    /// # Errors
    ///
    /// As [`WorldCore::prepare_input`], plus: the participant has no
    /// bound author key, or the signed move fails
    /// [`signed::MoveChain::verify`].
    pub(crate) fn prepare_input_signed(
        &self,
        token: &JoinCapability,
        input: &[u8],
        signed_move: &[u8],
    ) -> Result<(WorldEvent, crate::chain::Event)> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        ensure!(
            !self.limits.region_sharded,
            "region-sharded worlds require entity-addressed input"
        );
        self.ensure_uses_left(participant_id)?;
        ensure!(
            input.len() <= self.limits.input_bytes,
            "world input exceeds {} bytes",
            self.limits.input_bytes
        );
        ensure!(!input.is_empty(), "world input must not be empty");
        let author = self.author_keys.get(&participant_id).copied().ok_or_else(|| {
            anyhow::anyhow!(
                "participant {participant_id} has no bound author key; bind one before signed play"
            )
        })?;
        let chain = self
            .move_chains
            .get(&participant_id)
            .cloned()
            .unwrap_or_default();
        let signed = chain.verify(signed_move, &self.world_id, input, &author)?;
        let sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        Ok((
            WorldEvent {
                sequence,
                participant_id,
                kind: WorldEventKind::Input(input.to_vec()),
            },
            signed,
        ))
    }

    /// Record a signed move whose prepared event has just committed:
    /// advance the author's chain past it and keep the cross-referenced
    /// record. The actor calls this immediately after
    /// [`WorldCore::commit_prepared`] for the prepared event, while no
    /// other input can interleave.
    ///
    /// # Errors
    ///
    /// Fails if the participant has no bound author key, or the signed
    /// move is no longer the author's expected next move at commit time.
    pub(crate) fn record_signed_move(
        &mut self,
        event: &WorldEvent,
        signed: &crate::chain::Event,
    ) -> Result<()> {
        let participant_id = event.participant_id;
        let author = self
            .author_keys
            .get(&participant_id)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("participant {participant_id} has no bound author key")
            })?;
        let chain = self.move_chains.entry(participant_id).or_default();
        ensure!(
            signed.header.seq_num == chain.next_seq() && signed.header.backlink == chain.frontier(),
            "signed move is no longer the author's expected next move at commit"
        );
        chain.accept(signed);
        self.signed_moves.push(signed::SignedMoveRecord {
            world_sequence: event.sequence,
            participant_id,
            author,
            author_seq: signed.header.seq_num,
            event_id: signed.id,
            bytes: signed.bytes.clone(),
        });
        Ok(())
    }

    /// Commit a player-signed game input (#2814): the move is verified
    /// against the participant's bound key and chain position, committed
    /// as a normal input event, and recorded as signed evidence.
    ///
    /// # Errors
    ///
    /// As [`WorldCore::prepare_input_signed`].
    pub fn input_signed(
        &mut self,
        token: &JoinCapability,
        input: &[u8],
        signed_move: &[u8],
    ) -> Result<WorldEvent> {
        let (event, signed) = self.prepare_input_signed(token, input, signed_move)?;
        let event = self.commit_prepared(event)?;
        self.record_signed_move(&event, &signed)?;
        Ok(event)
    }

    /// Accepted signed moves, in world sequence order (#2814): the
    /// evidence base an attestation witness replays.
    #[must_use]
    pub fn signed_moves(&self) -> &[signed::SignedMoveRecord] {
        &self.signed_moves
    }

    pub(crate) fn prepare_input(&self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        ensure!(
            !self.author_keys.contains_key(&participant_id),
            "participant {participant_id} has a bound author key; inputs must be player-signed (#2814)"
        );
        ensure!(
            !self.limits.region_sharded,
            "region-sharded worlds require entity-addressed input"
        );
        self.ensure_uses_left(participant_id)?;
        ensure!(
            input.len() <= self.limits.input_bytes,
            "world input exceeds {} bytes",
            self.limits.input_bytes
        );
        ensure!(!input.is_empty(), "world input must not be empty");
        let sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        Ok(WorldEvent {
            sequence,
            participant_id,
            kind: WorldEventKind::Input(input.to_vec()),
        })
    }

    pub(crate) fn prepare_entity_input(
        &self,
        token: &JoinCapability,
        entity_ref: &str,
        input: &[u8],
    ) -> Result<WorldEvent> {
        validate_entity_ref(entity_ref)?;
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        ensure!(
            !self.author_keys.contains_key(&participant_id),
            "participant {participant_id} has a bound author key; inputs must be player-signed (#2814)"
        );
        self.ensure_uses_left(participant_id)?;
        ensure!(
            input.len() <= self.limits.input_bytes,
            "world input exceeds {} bytes",
            self.limits.input_bytes
        );
        ensure!(!input.is_empty(), "world input must not be empty");
        ensure!(
            !self.entity_is_fenced(entity_ref),
            "entity {entity_ref:?} is fenced after export"
        );
        let sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        Ok(WorldEvent {
            sequence,
            participant_id,
            kind: WorldEventKind::EntityInput {
                entity_ref: entity_ref.to_owned(),
                input: input.to_vec(),
            },
        })
    }

    /// The most recent ownership transfer recorded for `entity_ref`, if
    /// any. The ledger is append-only and chronological, so the last entry
    /// for an entity is its current ownership state; earlier entries are
    /// history, retained for handoff-ID deduplication.
    pub(crate) fn latest_transfer(&self, entity_ref: &str) -> Option<&HandoffTransferState> {
        self.handoff_transfers
            .iter()
            .rev()
            .find(|transfer| transfer.entity_ref == entity_ref)
    }

    /// An entity is fenced while its latest ownership transfer is an
    /// export: the fence derives from the current ownership state, not
    /// from any historical export. An entity imported back (design #2818
    /// §3 permits later crossings) is writable again; every historical
    /// receipt stays in the ledger for deduplication.
    pub(crate) fn entity_is_fenced(&self, entity_ref: &str) -> bool {
        matches!(
            self.latest_transfer(entity_ref),
            Some(transfer) if transfer.direction == HandoffDirection::Export
        )
    }

    /// Install a durable source export in the in-memory transfer ledger. The
    /// caller must append the matching journal entry first.
    pub(crate) fn validate_entity_export(&self, transfer: &HandoffTransferState) -> Result<()> {
        ensure!(
            self.limits.region_sharded,
            "entity export requires a region-sharded world"
        );
        ensure!(
            transfer.direction == HandoffDirection::Export,
            "source export has the wrong transfer direction"
        );
        validate_entity_ref(&transfer.entity_ref)?;
        ensure!(
            transfer
                .payload
                .as_ref()
                .is_some_and(|payload| payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES),
            "entity transfer payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
        );
        let payload = transfer.payload.as_deref().unwrap_or_default();
        ensure!(
            transfer.payload_digest == hex_encode(blake3::hash(payload).as_bytes()),
            "entity transfer payload digest does not match"
        );
        ensure!(
            !self
                .handoff_transfers
                .iter()
                .any(|existing| existing.handoff_id == transfer.handoff_id)
                && !matches!(
                    self.latest_transfer(&transfer.entity_ref),
                    Some(latest) if latest.direction == HandoffDirection::Export
                ),
            "entity already has an active export or handoff ID is duplicated"
        );
        ensure_epoch_chains(transfer, &self.handoff_transfers)?;
        Ok(())
    }

    /// Install a source fence after its matching journal record is durable.
    pub(crate) fn record_entity_export(&mut self, transfer: HandoffTransferState) {
        debug_assert!(self.validate_entity_export(&transfer).is_ok());
        self.handoff_transfers.push(transfer);
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
    }

    /// Validate a destination import against the dedupe ledger (#3870
    /// phase 3). The actor has already verified the envelope's module
    /// hash, schema and payload digest before mutation; this checks the
    /// durable-state invariants: direction, owner-epoch advance, no
    /// duplicate payload copy, and that neither the handoff ID nor the
    /// entity already carries an import.
    pub(crate) fn validate_entity_import(&self, transfer: &HandoffTransferState) -> Result<()> {
        ensure!(
            self.limits.region_sharded,
            "entity import requires a region-sharded world"
        );
        ensure!(
            transfer.direction == HandoffDirection::Import,
            "destination import has the wrong transfer direction"
        );
        validate_entity_ref(&transfer.entity_ref)?;
        ensure!(
            transfer.new_owner_epoch.is_some_and(|new_owner_epoch| {
                transfer.prior_owner_epoch.checked_add(1) == Some(new_owner_epoch)
            }),
            "destination import does not advance the owner epoch"
        );
        ensure!(
            transfer.payload.is_none(),
            "destination import carries a duplicate payload outside its event"
        );
        ensure!(
            !self
                .handoff_transfers
                .iter()
                .any(|existing| existing.handoff_id == transfer.handoff_id)
                && !matches!(
                    self.latest_transfer(&transfer.entity_ref),
                    Some(latest) if latest.direction == HandoffDirection::Import
                ),
            "entity already has an active import or handoff ID is duplicated"
        );
        ensure_epoch_chains(transfer, &self.handoff_transfers)?;
        Ok(())
    }

    /// Install an import receipt after its matching journal record is durable.
    pub(crate) fn record_entity_import(&mut self, transfer: HandoffTransferState) {
        debug_assert!(self.validate_entity_import(&transfer).is_ok());
        self.handoff_transfers.push(transfer);
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
    }

    pub(crate) fn commit_prepared(&mut self, event: WorldEvent) -> Result<WorldEvent> {
        ensure!(
            event.sequence == self.sequence.saturating_add(1),
            "prepared world event is stale"
        );
        self.sequence = event.sequence;
        self.spend(event.participant_id);
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
        self.events.push_back(event.clone());
        while self.events.len() > self.limits.retained_events {
            self.events.pop_front();
        }
        self.record_admin_history(event.clone());
        Ok(event)
    }

    /// Add an event to the bounded, ordered history reserved for admin export.
    pub(crate) fn record_admin_history(&mut self, event: WorldEvent) {
        if let WorldEventKind::System { event } = &event.kind
            && let Some((participant, joined)) = presence_change(event)
        {
            if joined {
                self.open_presence.insert(participant);
            } else {
                self.open_presence.remove(&participant);
            }
        }
        self.admin_history_ledger
            .track(admin_history_event_bytes(&event));
        self.admin_history.push_back(event);
        while self.admin_history.len() > self.limits.retained_events.min(MAX_ADMIN_HISTORY_EVENTS)
            || self.admin_history_ledger.effective_bytes() > MAX_ADMIN_HISTORY_BYTES
        {
            if self.admin_history.pop_front().is_none() {
                break;
            }
            self.admin_history_ledger.untrack_front();
            self.admin_history_complete = false;
        }
    }

    fn restore_admin_history(
        &mut self,
        events: Vec<WorldEvent>,
        complete: bool,
        sequence: u64,
    ) -> Result<()> {
        ensure!(
            events.len() <= self.limits.retained_events.min(MAX_ADMIN_HISTORY_EVENTS),
            "checkpoint admin history exceeds its event bound"
        );
        ensure!(
            admin_history_bytes(&events) <= MAX_ADMIN_HISTORY_BYTES,
            "checkpoint admin history exceeds its byte bound"
        );
        let mut previous: Option<u64> = None;
        for event in &events {
            match &event.kind {
                WorldEventKind::System { .. } => ensure!(
                    event.sequence == 0 && event.participant_id == 0,
                    "checkpoint admin history system event claims a sequence"
                ),
                _ => {
                    ensure!(
                        event.sequence > 0,
                        "checkpoint admin history event has no sequence"
                    );
                    if let Some(last) = previous {
                        ensure!(
                            event.sequence == last.saturating_add(1),
                            "checkpoint admin history sequence jumps from {last} to {}",
                            event.sequence
                        );
                    }
                    previous = Some(event.sequence);
                }
            }
        }
        ensure!(
            previous.is_none_or(|last| last <= sequence),
            "checkpoint admin history contains a sequence beyond {sequence}"
        );
        ensure!(
            !complete || previous.unwrap_or(0) == sequence,
            "complete checkpoint admin history ends at sequence {}, not {sequence}",
            previous.unwrap_or(0)
        );
        ensure!(
            !complete
                || sequence == 0
                || events
                    .iter()
                    .find(|event| event.sequence > 0)
                    .is_some_and(|event| event.sequence == 1),
            "complete checkpoint admin history does not start at sequence 1"
        );
        self.admin_history_ledger = AdminHistoryByteLedger::rebuild(&events);
        self.admin_history = events.into();
        self.admin_history_complete = complete;
        Ok(())
    }

    /// Return the current bounded snapshot after validating join scope. Its
    /// events are the viewer's projection: no chat without `chat` or
    /// `history`, and no install seeds (#240).
    pub fn snapshot(&self, token: &JoinCapability) -> Result<WorldSnapshot> {
        let participant_id = self.authorize(token, WorldScopes::JOIN)?;
        Ok(self.snapshot_unchecked(self.viewer_scopes(participant_id)))
    }

    /// What the viewer holding `token` may see of `event`, as it is delivered
    /// live. `Err` if the capability no longer joins (revoked, expired, or its
    /// chain was cut), so a session can end; `Ok(None)` if the event is not
    /// for this viewer.
    pub fn project_for(
        &self,
        token: &JoinCapability,
        event: &WorldEvent,
    ) -> Result<Option<WorldEvent>> {
        let participant_id = self.authorize(token, WorldScopes::JOIN)?;
        Ok(project_event_for(self.viewer_scopes(participant_id), event))
    }

    pub(crate) fn viewer_scopes(&self, participant_id: u64) -> WorldScopes {
        self.effective_scopes(participant_id)
            .unwrap_or(WorldScopes::EMPTY)
    }

    pub(crate) fn viewer(&self, token: &JoinCapability) -> Result<Participant> {
        let id = self.authorize(token, WorldScopes::JOIN)?;
        self.capabilities
            .get(&id)
            .map(|record| record.participant.clone())
            .context("world participant is unavailable")
    }

    /// Return retained events newer than `after_sequence`, plus the current
    /// cursor. If the cursor fell behind retention, the caller must resnapshot.
    /// The events are the viewer's projection (see [`project_event_for`]), so
    /// their sequences may skip withheld lines: a caller advances to
    /// `current_sequence`, not to the last sequence it was given plus one.
    /// Retention and `needs_snapshot` do not depend on the viewer.
    pub fn events_after(&self, token: &JoinCapability, after_sequence: u64) -> Result<EventPage> {
        let participant_id = self.authorize(token, WorldScopes::JOIN)?;
        let scopes = self.viewer_scopes(participant_id);
        let oldest = self.oldest_sequence();
        let needs_snapshot = after_sequence.saturating_add(1) < oldest;
        let events = if needs_snapshot {
            Vec::new()
        } else {
            self.events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .filter_map(|event| project_event_for(scopes, event))
                .collect()
        };
        Ok(EventPage {
            current_sequence: self.sequence,
            oldest_retained_sequence: oldest,
            needs_snapshot,
            events,
        })
    }

    pub(crate) fn authorize(&self, token: &JoinCapability, required: WorldScopes) -> Result<u64> {
        let (participant, _) = token
            .expose()
            .split_once('.')
            .ok_or_else(|| CapabilityDenied::new("malformed world capability"))?;
        let participant_id = participant
            .parse::<u64>()
            .map_err(|_| CapabilityDenied::new("malformed world capability participant ID"))?;
        let digest = token_digest(token.expose());
        let record = self.capabilities.get(&participant_id).ok_or_else(|| {
            CapabilityDenied::new("invalid, revoked, or insufficient world capability")
        })?;
        if !constant_time_eq(&digest, &record.digest) {
            return Err(CapabilityDenied::new(
                "invalid, revoked, or insufficient world capability",
            )
            .into());
        }
        // The authority check must not seed from `ALL`: that set excludes
        // the admin bits by design, which would make `BUILDER` /
        // `MODERATOR` unusable through this path. Seed with every
        // recognized bit instead — the ancestor-chain intersection in
        // `effective_scopes_seeded` only removes bits, so a required
        // authority bit survives only when every record in the chain
        // actually carries it.
        if !(!record.revoked
            && self
                .effective_scopes_seeded(participant_id, WorldScopes::all_recognized())
                .is_some_and(|scopes| scopes.contains(required)))
        {
            return Err(CapabilityDenied::new(
                "invalid, revoked, or insufficient world capability",
            )
            .into());
        }
        self.ensure_not_expired(participant_id)?;
        Ok(participant_id)
    }

    pub(crate) fn oldest_sequence(&self) -> u64 {
        self.events
            .front()
            .map_or_else(|| self.sequence.saturating_add(1), |event| event.sequence)
    }

    pub(crate) fn snapshot_unchecked(&self, viewer: WorldScopes) -> WorldSnapshot {
        let participants = self
            .capabilities
            .values()
            .filter(|record| !record.revoked && record.scopes.contains(WorldScopes::JOIN))
            .map(|record| record.participant.clone())
            .collect();
        // Take the most recent events that fit the snapshot budget, newest
        // first. A snapshot is a baseline, not a complete replay: clients
        // adopt its sequence as their cursor. Events the viewer may not see
        // are dropped before the budget is spent, so it counts only what the
        // snapshot carries.
        let mut budget = self.limits.snapshot_bytes;
        let mut events: VecDeque<WorldEvent> = VecDeque::new();
        for event in self
            .events
            .iter()
            .rev()
            .filter_map(|event| project_event_for(viewer, event))
        {
            let size = serde_json::to_vec(&event).map_or(budget, |encoded| encoded.len());
            if size > budget {
                break;
            }
            budget -= size;
            events.push_front(event);
        }
        let oldest_retained_sequence = events
            .front()
            .map_or_else(|| self.sequence.saturating_add(1), |event| event.sequence);
        WorldSnapshot {
            world_id: self.world_id.clone(),
            active_module_hash: self.active_module_hash.clone(),
            active_module_provenance: self.active_module_provenance.clone(),
            current_sequence: self.sequence,
            oldest_retained_sequence,
            events: events.into_iter().collect(),
            participants,
        }
    }
}
