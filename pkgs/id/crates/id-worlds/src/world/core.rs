//! Durable world reconstruction and authority-bearing core state.
//!
//! Reconstruction is admission: validate each journal entry against the
//! verifier's state rebuilt from earlier admitted entries. Refuse the whole
//! restore on invalid input; never partially apply or silently repair it.
//! Guest snapshots contain program-owned state, not host authority: the host
//! capability ledger is reconstructed and checked separately, and guest
//! requests remain subject to that ledger.

use super::*;

use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use id_core::boundary::EnforcementPoint;
use id_core::boundary_surfaces::FrameworkOutcome;
use id_directory::directory::BoundaryGate;

mod checkpoint_parts;
mod operations;

use checkpoint_parts::{
    CheckpointEventPartKind, PendingCheckpointParts, checkpoint_part_json_len,
    split_checkpoint_events, split_checkpoint_snapshot,
};

mod admin_history;

pub(crate) use admin_history::AdminHistoryByteLedger;
use admin_history::{admin_history_bytes, admin_history_event_bytes};

#[derive(Clone)]
pub(crate) struct MembershipBoundaryGateHandle(Arc<dyn BoundaryGate>);

impl MembershipBoundaryGateHandle {
    pub(crate) fn check(&self, subject: &str) -> FrameworkOutcome {
        self.0.check(EnforcementPoint::WorldMembership, subject)
    }
}

impl std::fmt::Debug for MembershipBoundaryGateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MembershipBoundaryGateHandle(..)")
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
    /// The world's `kv` store (#1062): key to the value's JSON
    /// encoding. Bounded by the `MAX_KV_*` caps; every mutation is
    /// journaled before its result is delivered, and replay rebuilds
    /// the store from the journal.
    pub(crate) kv: BTreeMap<String, String>,
    /// The world's blob set (#1062): content address (the blob's
    /// iroh-blobs hash as lowercase hex) to content bytes. Bounded
    /// by the `MAX_BLOB_*` caps; every mutation is journaled before
    /// its result is delivered, and replay rebuilds the set from the
    /// journal, verifying each blob against its address.
    pub(crate) blobs: BTreeMap<String, Vec<u8>>,
    /// `http.fetch` attempt timestamps (#1062), world-clock
    /// milliseconds, oldest first: the rolling rate window
    /// (`MAX_HTTP_FETCH_BURST` per `HTTP_FETCH_RATE_WINDOW_MS`).
    /// Ephemeral by design — the rate window is not journaled,
    /// and a restart resets it.
    pub(crate) http_fetch_attempts: VecDeque<u64>,
    /// `notify` attempt timestamps (#1062), world-clock
    /// milliseconds, oldest first: the rolling rate window
    /// (`MAX_NOTIFY_BURST` per `NOTIFY_RATE_WINDOW_MS`).
    /// Ephemeral by design, like the fetch window — the bound
    /// shapes a live program's interruptions, and a restart
    /// resets it.
    pub(crate) notify_attempts: VecDeque<u64>,
    /// Schedule-delivery consent by peer world (#1355 part 2): this
    /// world's half of the reciprocal F12.14 grant pair.
    pub(crate) schedule_grants: BTreeMap<String, ScheduleGrant>,
    /// This world's F12.14 portal grant book (code#1068 wiring
    /// slice): the admin-written registry of portal peers, restored
    /// from `PortalGrantSet` journal entries. One half of the mutual
    /// check — a send also reads the peer world's book.
    pub(crate) portal_grants: id_caps::portal::PortalGrants,
    /// Cross-world schedule fires this world has durably accepted, keyed
    /// by (source world, schedule id, planned due time): the receipts a
    /// source retry is answered from (#1355 part 2).
    pub(crate) schedule_receipts: BTreeSet<(String, String, u64)>,
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
    /// Runtime boundary gate for account-bound world membership issuance.
    /// Like the directory's signup/login gate, this is reinstalled by the
    /// serving app after journal restore and is never persisted.
    pub(crate) membership_boundary_gate: Option<MembershipBoundaryGateHandle>,
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
    /// [`signed`](super::signed)). Journaled since #4346: bindings,
    /// chain positions, and accepted records are rebuilt by journal
    /// replay and preserved across compaction by `checkpoint_entries`.
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
    let envelope = id_core::json_unique::parse_json_without_duplicate_keys(text)
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
            kv: BTreeMap::new(),
            blobs: BTreeMap::new(),
            http_fetch_attempts: VecDeque::new(),
            notify_attempts: VecDeque::new(),
            schedule_grants: BTreeMap::new(),
            portal_grants: id_caps::portal::PortalGrants::new(),
            schedule_receipts: BTreeSet::new(),
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
            membership_boundary_gate: None,
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
                    issued_at,
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
                    let mut ancestor_revoked_at = None;
                    while let Some(ancestor_id) = ancestor {
                        let ancestor_record = core
                            .capabilities
                            .get(&ancestor_id)
                            .context("delegated capability ancestor disappeared during restore")?;
                        if ancestor_record.revoked {
                            revoked_by_ancestor = true;
                            ancestor_revoked_at = ancestor_record.revoked_at;
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
                            revoked_by: None,
                            bounds: CapabilityBounds { expires_at, uses },
                            used,
                            subject,
                            issued_at,
                            revoked_at: ancestor_revoked_at,
                        },
                    );
                    if revoked_by_ancestor {
                        inherited_revocations.insert(issued_participant_id);
                    }
                }
                JournalEntry::Revoked {
                    participant_id,
                    by,
                    revoked_at,
                } => {
                    if inherited_revocations.remove(&participant_id) {
                        // Revoked with an ancestor before this checkpointed
                        // entry was read: keep the time the entry carries.
                        if let Some(record) = core.capabilities.get_mut(&participant_id) {
                            record.revoked_at = revoked_at;
                        }
                    } else {
                        ensure!(
                            !core
                                .revoke_tree_at(participant_id, by, revoked_at)
                                .is_empty(),
                            "journal revokes unknown or revoked participant {participant_id}"
                        );
                    }
                }
                JournalEntry::AuthorKeyBound {
                    participant_id,
                    author,
                } => {
                    let author =
                        hex_decode_32(&author).context("malformed author key in journal")?;
                    ensure!(
                        core.capabilities.contains_key(&participant_id),
                        "journal binds an author key to unknown participant {participant_id}"
                    );
                    if let Some(bound) = core.author_keys.get(&participant_id) {
                        ensure!(
                            *bound == author,
                            "journal binds a different author key to participant {participant_id}"
                        );
                    } else {
                        core.author_keys.insert(participant_id, author);
                        core.move_chains.entry(participant_id).or_default();
                    }
                }
                JournalEntry::SignedMoveAccepted { record } => {
                    let record = record
                        .to_record()
                        .context("malformed signed move in journal")?;
                    ensure!(
                        core.capabilities.contains_key(&record.participant_id),
                        "journal accepts a signed move for unknown participant {}",
                        record.participant_id
                    );
                    let author = core
                        .author_keys
                        .get(&record.participant_id)
                        .copied()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "journal accepts a signed move for participant {} with no bound author key",
                                record.participant_id
                            )
                        })?;
                    ensure!(
                        author == record.author,
                        "journal signed move author does not match the bound key for participant {}",
                        record.participant_id
                    );
                    // Cross-reference the committed input while it is
                    // retained: with it, replay recomputes the payload
                    // commitment exactly as the live seam did. Once the
                    // input has aged out of the retained window under
                    // compaction, replay falls back to the header checks
                    // (see MoveChain::verify_journaled).
                    let committed_input = match core
                        .events
                        .iter()
                        .find(|event| event.sequence == record.world_sequence)
                    {
                        Some(event) => {
                            // The referenced sequence is still retained,
                            // so it must be this participant's input.
                            // Anything else is a false cross-reference,
                            // not history that aged out (Codex review on
                            // PR #9037).
                            ensure!(
                                event.participant_id == record.participant_id,
                                "journal signed move references world sequence {} \
                                 committed by participant {}, not {}",
                                record.world_sequence,
                                event.participant_id,
                                record.participant_id
                            );
                            match &event.kind {
                                WorldEventKind::Input(input) => Some(input.clone()),
                                _ => bail!(
                                    "journal signed move references world sequence {} \
                                     which is not a committed input",
                                    record.world_sequence
                                ),
                            }
                        }
                        None => {
                            // Header-only verification is legitimate only
                            // for an input that genuinely aged out of the
                            // retained window: its sequence must already
                            // have committed at this point in the replay
                            // AND have fallen off the front of the window.
                            ensure!(
                                record.world_sequence >= 1
                                    && record.world_sequence <= core.sequence,
                                "journal signed move references world sequence {} \
                                 beyond the restored sequence {}",
                                record.world_sequence,
                                core.sequence
                            );
                            if let Some(front) = core.events.front() {
                                ensure!(
                                    record.world_sequence < front.sequence,
                                    "journal signed move references world sequence {} \
                                     inside the retained window, but no input event \
                                     is retained for it",
                                    record.world_sequence
                                );
                            }
                            None
                        }
                    };
                    let world_id = core.world_id.clone();
                    let chain = core.move_chains.entry(record.participant_id).or_default();
                    let event = match &committed_input {
                        Some(input) => chain
                            .verify(&record.bytes, &world_id, input, &author)
                            .context("journal signed move does not re-verify")?,
                        None => chain
                            .verify_journaled(&record.bytes, &world_id, &author)
                            .context("journal signed move does not re-verify")?,
                    };
                    ensure!(
                        event.id == record.event_id && event.header.seq_num == record.author_seq,
                        "journal signed move record does not match its signed bytes"
                    );
                    chain.accept(&event);
                    core.signed_moves.push(record);
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
                    target,
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
                    if let Some(target) = &target {
                        crate::world_hub::validate_world_name(target)
                            .context("invalid schedule target in journal")?;
                    }
                    core.schedules.insert(
                        id,
                        PendingSchedule {
                            at_ms,
                            interval_ms,
                            remaining,
                            target,
                        },
                    );
                }
                JournalEntry::ScheduleGrantSet {
                    peer,
                    send,
                    receive,
                } => {
                    crate::world_hub::validate_world_name(&peer)
                        .context("invalid schedule grant peer in journal")?;
                    if send || receive {
                        core.schedule_grants
                            .insert(peer, ScheduleGrant { send, receive });
                    } else {
                        core.schedule_grants.remove(&peer);
                    }
                }
                JournalEntry::PortalGrantSet { peer, granted } => {
                    crate::world_hub::validate_world_name(&peer)
                        .context("invalid portal grant peer in journal")?;
                    core.portal_grants.restore_state(
                        &peer,
                        Some(if granted {
                            id_caps::portal::PortalGrantState::Granted
                        } else {
                            id_caps::portal::PortalGrantState::Revoked
                        }),
                    );
                }
                JournalEntry::ScheduleReceived { source, id, at_ms } => {
                    crate::world_caps::validate_schedule_id(&id)
                        .context("invalid received schedule id in journal")?;
                    core.schedule_receipts.insert((source, id, at_ms));
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
                JournalEntry::KvSet { key, value } => {
                    // The bounds the writer holds are re-checked here:
                    // a journal that would widen the store past them
                    // refuses the restore instead (#1062).
                    crate::world_caps::validate_kv_key(&key)
                        .context("invalid kv key in journal")?;
                    ensure!(
                        value.len() <= crate::world_caps::MAX_KV_VALUE_BYTES,
                        "journal kv value for {key:?} exceeds {} bytes",
                        crate::world_caps::MAX_KV_VALUE_BYTES
                    );
                    ensure!(
                        core.kv.contains_key(&key)
                            || core.kv.len() < crate::world_caps::MAX_KV_ENTRIES,
                        "journal holds more than {} kv entries",
                        crate::world_caps::MAX_KV_ENTRIES
                    );
                    id_core::json_unique::parse_json_without_duplicate_keys(&value)
                        .context("invalid kv value in journal")?;
                    core.kv.insert(key, value);
                    ensure!(
                        core.kv_bytes() <= crate::world_caps::MAX_KV_TOTAL_BYTES,
                        "journal kv store exceeds {} total bytes",
                        crate::world_caps::MAX_KV_TOTAL_BYTES
                    );
                }
                JournalEntry::KvDeleted { key } => {
                    crate::world_caps::validate_kv_key(&key)
                        .context("invalid kv key in journal")?;
                    ensure!(
                        core.kv.remove(&key).is_some(),
                        "journal deletes kv key {key:?}, which is not stored"
                    );
                }
                JournalEntry::BlobStored { hash, data } => {
                    // The bounds the writer holds are re-checked here,
                    // and content addressing adds one the writer
                    // cannot break but a corrupt or tampered journal
                    // can: the decoded content must hash back to the
                    // address the entry claims (#1062).
                    crate::world_caps::validate_blob_hash(&hash)
                        .context("invalid blob hash in journal")?;
                    let bytes =
                        super::blob_decode(&data).context("journal blob content is not base64")?;
                    ensure!(
                        super::blob_id(&bytes) == hash,
                        "journal blob content does not hash to its address {hash}"
                    );
                    ensure!(
                        bytes.len() <= crate::world_caps::MAX_BLOB_BYTES,
                        "journal blob {hash} exceeds {} bytes",
                        crate::world_caps::MAX_BLOB_BYTES
                    );
                    ensure!(
                        core.blobs.contains_key(&hash)
                            || core.blobs.len() < crate::world_caps::MAX_BLOBS,
                        "journal holds more than {} blobs",
                        crate::world_caps::MAX_BLOBS
                    );
                    core.blobs.insert(hash, bytes);
                    ensure!(
                        core.blob_bytes() <= crate::world_caps::MAX_BLOB_TOTAL_BYTES,
                        "journal blob set exceeds {} total bytes",
                        crate::world_caps::MAX_BLOB_TOTAL_BYTES
                    );
                }
                JournalEntry::BlobRemoved { hash } => {
                    crate::world_caps::validate_blob_hash(&hash)
                        .context("invalid blob hash in journal")?;
                    ensure!(
                        core.blobs.remove(&hash).is_some(),
                        "journal removes blob {hash}, which is not stored"
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
                    position,
                } => {
                    core.write_lease
                        .apply_grant(&holder, epoch, expires_at_ms, position)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseRenewed {
                    holder,
                    epoch,
                    expires_at_ms,
                    position,
                } => {
                    core.write_lease
                        .apply_renew(&holder, epoch, expires_at_ms, position)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseHandedOff {
                    from,
                    to,
                    epoch,
                    expires_at_ms,
                    position,
                } => {
                    core.write_lease
                        .apply_handoff(&from, &to, epoch, expires_at_ms, position)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseRevoked {
                    former_holder,
                    epoch,
                    reason,
                    position,
                } => {
                    core.write_lease
                        .apply_revoke(&former_holder, epoch, &reason, position)?;
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::LeaseWriteRejected {
                    writer,
                    presented_epoch,
                    operation_id,
                    observed_epoch,
                    reason,
                    received_at_ms,
                    position,
                } => {
                    core.write_lease.apply_rejection(LeaseWriteRejection {
                        writer,
                        presented_epoch,
                        operation_id,
                        observed_epoch,
                        reason,
                        received_at_ms,
                        position,
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
                        // A notification is history, not program input:
                        // it is retained by `commit_prepared` below and
                        // never fed to the program's replay (#1062).
                        WorldEventKind::Notification { .. } => {}
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
                    kv,
                    blobs,
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
                                    target: schedule.target.clone(),
                                },
                            )
                        })
                        .collect();
                    ensure!(
                        declared.len() == listed.len() && declared == core.schedules,
                        "journal checkpoint's pending schedules disagree with its schedule entries"
                    );
                    // The same agreement for the `kv` store: the writer
                    // emits one `KvSet` per live entry before the
                    // checkpoint, so the two must agree exactly.
                    let listed_kv = kv.unwrap_or_default();
                    let declared_kv: BTreeMap<String, String> = listed_kv
                        .iter()
                        .map(|entry| (entry.key.clone(), entry.value.clone()))
                        .collect();
                    ensure!(
                        declared_kv.len() == listed_kv.len() && declared_kv == core.kv,
                        "journal checkpoint's kv entries disagree with its kv set entries"
                    );
                    // The same agreement for the blob set, over the
                    // declared index: the writer emits one `BlobStored`
                    // per live blob before the checkpoint, so the
                    // declared (hash, size) pairs must equal the
                    // replayed set exactly.
                    let listed_blobs = blobs.unwrap_or_default();
                    let declared_blobs: BTreeMap<String, usize> = listed_blobs
                        .iter()
                        .map(|entry| (entry.hash.clone(), entry.size))
                        .collect();
                    let replayed_blobs: BTreeMap<String, usize> = core
                        .blobs
                        .iter()
                        .map(|(hash, bytes)| (hash.clone(), bytes.len()))
                        .collect();
                    ensure!(
                        declared_blobs.len() == listed_blobs.len()
                            && declared_blobs == replayed_blobs,
                        "journal checkpoint's blob index disagrees with its blob stored entries"
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
            WorldEventKind::System { .. }
            | WorldEventKind::ProgramInstalled { .. }
            | WorldEventKind::Notification { .. } => None,
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
                    by: self
                        .capabilities
                        .get(&id)
                        .and_then(|record| record.revoked_by),
                    revoked_at: self
                        .capabilities
                        .get(&id)
                        .and_then(|record| record.revoked_at),
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
                target: schedule.target.clone(),
            });
        }
        // Schedule grants and receipts ride along like the schedules:
        // without them compaction could forget a consent (and re-refuse a
        // live series) or an acceptance (and deliver a fire twice).
        for (peer, grant) in &self.schedule_grants {
            entries.push(JournalEntry::ScheduleGrantSet {
                peer: peer.clone(),
                send: grant.send,
                receive: grant.receive,
            });
        }
        // Portal grants ride along like the schedule grants: without
        // them compaction could forget a grant (and re-refuse a live
        // portal) or a revocation (and resurrect one).
        for (peer, state) in self.portal_grants.peers() {
            entries.push(JournalEntry::PortalGrantSet {
                peer: peer.to_owned(),
                granted: state == id_caps::portal::PortalGrantState::Granted,
            });
        }
        for (source, id, at_ms) in &self.schedule_receipts {
            entries.push(JournalEntry::ScheduleReceived {
                source: source.clone(),
                id: id.clone(),
                at_ms: *at_ms,
            });
        }
        // One entry per live `kv` entry, before the checkpoint, like
        // the schedules: without them compaction would forget the
        // store, and an older binary refuses the journal at the
        // version check instead of restoring without it.
        for (key, value) in &self.kv {
            entries.push(JournalEntry::KvSet {
                key: key.clone(),
                value: value.clone(),
            });
        }
        let kv: Option<Vec<CheckpointKvEntry>> = (!self.kv.is_empty()).then(|| {
            self.kv
                .iter()
                .map(|(key, value)| CheckpointKvEntry {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect()
        });
        // One entry per live blob, before the checkpoint, like the
        // `kv` entries: without them compaction would forget the
        // content, and an older binary refuses the journal at the
        // version check instead of restoring without it. The
        // checkpoint itself declares only the index (hash + size) —
        // the entries carry the bytes.
        for (hash, bytes) in &self.blobs {
            entries.push(JournalEntry::BlobStored {
                hash: hash.clone(),
                data: super::blob_encode(bytes),
            });
        }
        let blobs: Option<Vec<CheckpointBlobEntry>> = (!self.blobs.is_empty()).then(|| {
            self.blobs
                .iter()
                .map(|(hash, bytes)| CheckpointBlobEntry {
                    hash: hash.clone(),
                    size: bytes.len(),
                })
                .collect()
        });
        let schedules: Option<Vec<CheckpointSchedule>> = (!self.schedules.is_empty()).then(|| {
            self.schedules
                .iter()
                .map(|(id, schedule)| CheckpointSchedule {
                    id: id.clone(),
                    at_ms: schedule.at_ms,
                    interval_ms: schedule.interval_ms,
                    remaining: schedule.remaining,
                    target: schedule.target.clone(),
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
                    kv: kv.clone(),
                    blobs: blobs.clone(),
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
        // Signed play rides through compaction behind the checkpoint
        // itself (issue #4346): bindings first, then every accepted
        // record in world order, so replay cross-references the
        // checkpoint's retained events and re-verifies each chain in
        // sequence. An older binary refuses these entries at the
        // header (version 8) instead of restoring a world that lost
        // its chains.
        let mut bound_ids: Vec<u64> = self.author_keys.keys().copied().collect();
        bound_ids.sort_unstable();
        for participant_id in bound_ids {
            entries.push(JournalEntry::AuthorKeyBound {
                participant_id,
                author: hex_encode(&self.author_keys[&participant_id]),
            });
        }
        for record in &self.signed_moves {
            entries.push(JournalEntry::SignedMoveAccepted {
                record: JournalSignedMove::from_record(record),
            });
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod duplicate_key_tests {
    use super::*;

    fn import_event(envelope: String) -> WorldEvent {
        WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::System { event: envelope },
        }
    }

    #[test]
    fn an_import_envelope_with_a_duplicated_key_is_refused() {
        let payload = b"entity";
        let digest = hex_encode(blake3::hash(payload).as_bytes());
        let body = format!(
            r#""protocol":"world.entity-transfer.v1","direction":"import","payload_digest":"{digest}","payload_hex":"{}"}}"#,
            hex_encode(payload)
        );
        validate_import_envelope(&import_event(format!("{{{body}")), &digest).unwrap();
        // A lenient parse keeps the last "direction" and passes the check.
        let duplicated = format!(r#"{{"direction":"export",{body}"#);
        assert!(validate_import_envelope(&import_event(duplicated), &digest).is_err());
    }
}
