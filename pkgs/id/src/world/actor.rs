use super::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::Instrument;

use crate::world_caps::{CapError, CapsReport, Decision, Request, Subscription};

/// Reserved tracing target for structured administrative and provider-use audit records.
pub(crate) const AUDIT_TRACE_TARGET: &str = "id::audit";

/// One structured audit event. `parameters` contains only non-secret action data.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AuditRecord {
    /// Independent audit-record schema version.
    pub(crate) schema_version: u8,
    /// Unix timestamp in milliseconds.
    pub(crate) timestamp_ms: u64,
    /// World that processed the action.
    pub(crate) world_id: String,
    /// Public authority or non-secret actor identity.
    pub(crate) actor: String,
    /// Stable command or provider-use name.
    pub(crate) action: String,
    /// Safe, essential parameters for replay or review.
    pub(crate) parameters: serde_json::Value,
    /// `accepted`, `refused`, or a provider-use decision.
    pub(crate) outcome: String,
}

fn authority_identity(authority: &Authority) -> String {
    match authority {
        Authority::Admin => "admin".to_owned(),
        Authority::Signed(public_key) => format!("signed:{public_key}"),
    }
}

impl WorldCommand {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Issue { .. } => "Issue",
            Self::Attenuate { .. } => "Attenuate",
            Self::DelegateToFriend { .. } => "DelegateToFriend",
            Self::Directory { .. } => "Directory",
            Self::ApplyDirectory { .. } => "ApplyDirectory",
            Self::Revoke { .. } => "Revoke",
            Self::RevokeAuthorized { .. } => "RevokeAuthorized",
            Self::ParticipantId { .. } => "ParticipantId",
            Self::AuthorizeScope { .. } => "AuthorizeScope",
            Self::BindAuthorKey { .. } => "BindAuthorKey",
            Self::Chat { .. } => "Chat",
            Self::Input { .. } => "Input",
            Self::EntityInput { .. } => "EntityInput",
            Self::ExportEntity { .. } => "ExportEntity",
            Self::ImportEntity { .. } => "ImportEntity",
            Self::Snapshot { .. } => "Snapshot",
            Self::HostView { .. } => "HostView",
            Self::EventsAfter { .. } => "EventsAfter",
            Self::ProjectEvent { .. } => "ProjectEvent",
            Self::View { .. } => "View",
            Self::ViewAt { .. } => "ViewAt",
            Self::InstallProgram { .. } => "InstallProgram",
            Self::InstallProgramAuthorized { .. } => "InstallProgramAuthorized",
            Self::Presence { .. } => "Presence",
            Self::UpdateCaps { .. } => "UpdateCaps",
            Self::SetCapsCeiling { .. } => "SetCapsCeiling",
            Self::SetExecutionProfile { .. } => "SetExecutionProfile",
            Self::ExecutionProfileInfo { .. } => "ExecutionProfileInfo",
            Self::ManifestState { .. } => "ManifestState",
            Self::AdminScheduleAt { .. } => "AdminScheduleAt",
            Self::AdminCancelSchedule { .. } => "AdminCancelSchedule",
            Self::CapsInfo { .. } => "CapsInfo",
            Self::SetIsolation { .. } => "SetIsolation",
            Self::IsolationInfo { .. } => "IsolationInfo",
            Self::SetCrossWorldWrite { .. } => "SetCrossWorldWrite",
            Self::ForceCrossWorldWrite { .. } => "ForceCrossWorldWrite",
            Self::SetFileScope { .. } => "SetFileScope",
            Self::PolicyInfo { .. } => "PolicyInfo",
            Self::Metrics { .. } => "Metrics",
            Self::AdminEventHistory { .. } => "AdminEventHistory",
            Self::ReplicationFeed { .. } => "ReplicationFeed",
            Self::ReplicationCompact { .. } => "ReplicationCompact",
            Self::Shutdown { .. } => "Shutdown",
        }
    }
}

pub(crate) enum WorldCommand {
    Issue {
        name: String,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
    },
    Attenuate {
        token: JoinCapability,
        name: String,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
    },
    DelegateToFriend {
        token: JoinCapability,
        friend: String,
        name: String,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
    },
    Directory {
        caller: crate::directory_auth::Caller,
        action: crate::directory_view::DirectoryAction,
        /// Whether this server has an outbox to queue what the action sends.
        outbox: bool,
        reply: oneshot::Sender<Result<crate::directory_view::DirectoryOutcome>>,
    },
    ApplyDirectory {
        entry: crate::directory::DirectoryEntry,
        reply: oneshot::Sender<Result<()>>,
    },
    Revoke {
        participant_id: u64,
        /// The moderator participant who ordered the revoke, for the audit
        /// journal. `None` means the admin revoked.
        by: Option<u64>,
        reply: oneshot::Sender<bool>,
    },
    /// Revoke ordered by a joined moderator (design #835 phase 2): the
    /// actor authorizes the live capability itself and derives the audit
    /// actor from it; no caller-supplied actor ID is trusted. The reply is
    /// a `Result` so a journal failure is surfaced instead of being
    /// reported as an audited success; the in-memory revocation stands
    /// either way (fail-closed).
    RevokeAuthorized {
        token: JoinCapability,
        participant_id: u64,
        reply: oneshot::Sender<Result<bool>>,
    },
    ParticipantId {
        token: JoinCapability,
        reply: oneshot::Sender<Result<u64>>,
    },
    /// Authorize a live capability for a scope set without performing
    /// the operation (design #835 phase 3): session paths check this
    /// before receiving upload bytes, while the operation commands
    /// (`InstallProgramAuthorized`, `RevokeAuthorized`) still re-check
    /// at the effect boundary. Replies with the derived participant ID.
    AuthorizeScope {
        token: JoinCapability,
        required: WorldScopes,
        reply: oneshot::Sender<Result<u64>>,
    },
    Chat {
        token: JoinCapability,
        text: String,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    Input {
        token: JoinCapability,
        input: Vec<u8>,
        session_id: Option<SessionId>,
        /// The player-signed move event bytes for this input (#2814), when
        /// the participant plays signed. `None` is the unsigned path,
        /// which refuses participants with a bound author key.
        signed_move: Option<Vec<u8>>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    /// Bind the calling participant's author key for signed play (#2814).
    BindAuthorKey {
        token: JoinCapability,
        author_key: [u8; 32],
        reply: oneshot::Sender<Result<u64>>,
    },
    EntityInput {
        token: JoinCapability,
        entity_ref: String,
        input: Vec<u8>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    ExportEntity {
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
        reply: oneshot::Sender<Result<HandoffTransferState>>,
    },
    ImportEntity {
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
        program_module_hash: String,
        payload_schema: String,
        payload: Vec<u8>,
        payload_digest: String,
        reply: oneshot::Sender<Result<HandoffTransferState>>,
    },
    Snapshot {
        token: JoinCapability,
        reply: oneshot::Sender<Result<WorldSnapshot>>,
    },
    /// Admin-authorized web rendering; the caller must validate the admin
    /// secret before requesting this view. The synthetic viewer is not a
    /// registered participant and never receives presence or event rights.
    HostView {
        reply: oneshot::Sender<Result<(WorldSnapshot, Option<String>)>>,
    },
    EventsAfter {
        token: JoinCapability,
        sequence: u64,
        reply: oneshot::Sender<Result<EventPage>>,
    },
    ProjectEvent {
        token: JoinCapability,
        event: WorldEvent,
        reply: oneshot::Sender<Result<Option<WorldEvent>>>,
    },
    View {
        token: JoinCapability,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    ViewAt {
        token: JoinCapability,
        width: u16,
        height: u16,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    InstallProgram {
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        /// The builder participant who ordered the install, for the audit
        /// journal. `None` means the admin installed.
        installed_by: Option<u64>,
        /// The trust decision that admitted the module, for the audit
        /// journal (#1069). `None` records no decision.
        trust: Option<crate::world::TrustDecision>,
        program: Box<dyn WorldProgram>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    /// Install ordered by a joined builder (design #835 phase 2): the
    /// actor authorizes the live capability itself at the mutation
    /// boundary and derives `installed_by` from it.
    InstallProgramAuthorized {
        token: JoinCapability,
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        trust: Option<crate::world::TrustDecision>,
        program: Box<dyn WorldProgram>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    /// A session opened or closed for this participant.
    Presence {
        participant: u64,
        joined: bool,
    },
    /// Admin: change the granted capability set.
    UpdateCaps {
        grant: Vec<String>,
        revoke: Vec<String>,
        reply: oneshot::Sender<Result<CapsReport>>,
    },
    /// Admin: replace the world's capability ceiling. It applies immediately.
    SetCapsCeiling {
        ceiling: Vec<String>,
        reply: oneshot::Sender<Result<CapsReport>>,
    },
    /// Admin: durably replace this world's requested runtime profile.
    SetExecutionProfile {
        profile: crate::world_limits::WorldExecutionProfile,
        reply: oneshot::Sender<Result<ExecutionProfileUpdate>>,
    },
    /// Read this world's requested runtime profile.
    ExecutionProfileInfo {
        reply: oneshot::Sender<crate::world_limits::WorldExecutionProfile>,
    },
    /// Admin: read the manifest-reconciliation state — the active install
    /// seed and the pending schedules. The seed stays inside the actor and
    /// the admin layer; it is never part of a participant projection (#240).
    ManifestState {
        reply: oneshot::Sender<(u64, BTreeMap<String, u64>)>,
    },
    /// Admin: pend one schedule, replacing any pending schedule with the
    /// same id. Replies with the full pending map after the change.
    AdminScheduleAt {
        id: String,
        at_ms: u64,
        reply: oneshot::Sender<Result<BTreeMap<String, u64>>>,
    },
    /// Admin: cancel a pending schedule by id (a no-op when absent).
    /// Replies with the full pending map after the change.
    AdminCancelSchedule {
        id: String,
        reply: oneshot::Sender<Result<BTreeMap<String, u64>>>,
    },
    /// Admin: read the capability report.
    CapsInfo {
        reply: oneshot::Sender<Result<CapsReport>>,
    },
    /// Admin or moderator: change the world's isolation.
    SetIsolation {
        authority: Authority,
        isolation: Isolation,
        reply: oneshot::Sender<Result<Isolation>>,
    },
    IsolationInfo {
        reply: oneshot::Sender<Isolation>,
    },
    /// Moderator or admin: let other unisolated worlds write here.
    SetCrossWorldWrite {
        authority: Authority,
        enabled: bool,
        reply: oneshot::Sender<Result<WorldPolicy>>,
    },
    /// Admin: force cross-world writes on.
    ForceCrossWorldWrite {
        authority: Authority,
        reply: oneshot::Sender<Result<WorldPolicy>>,
    },
    /// Moderator or admin: set the file scope; the admin alone sets the override.
    SetFileScope {
        authority: Authority,
        scope: FileScope,
        write_override: Option<bool>,
        reply: oneshot::Sender<Result<WorldPolicy>>,
    },
    PolicyInfo {
        reply: oneshot::Sender<WorldPolicy>,
    },
    /// Read this world's host metrics (F11.7): counters and histograms
    /// recorded by the actor, the journal path, and the session layer.
    Metrics {
        reply: oneshot::Sender<MetricsSnapshot>,
    },
    /// Admin-only consistent snapshot for replayable fixture export.
    AdminEventHistory {
        reply: oneshot::Sender<Result<AdminEventHistory>>,
    },
    /// Atomically capture the journal and subscribe to subsequent durable changes.
    /// When the captured journal is large enough to hold an over-bound entry,
    /// a second source is returned as a probe: the caller scans it OUTSIDE
    /// the actor (#8095), because awaiting the scan here would suspend all
    /// command processing for the duration of a full journal read.
    ReplicationFeed {
        reply: oneshot::Sender<
            Result<(
                String,
                JournalSnapshotSource,
                Option<JournalSnapshotSource>,
                broadcast::Receiver<ReplicationUpdate>,
            )>,
        >,
    },
    /// Serialized forced compaction after an out-of-actor probe found an
    /// oversized legacy entry (#8095). Compaction itself stays an actor
    /// operation; only the scan moved out.
    ReplicationCompact {
        ordinal: usize,
        encoded_size: usize,
        reply: oneshot::Sender<Result<()>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Trim `journal` behind a verified snapshot of `program`. `Ok(false)` means
/// the program cannot snapshot.
pub(crate) fn compact_journal(
    core: &mut WorldCore,
    program: &mut dyn WorldProgram,
    journal: &mut dyn WorldJournal,
) -> Result<bool> {
    let Some(snapshot) = program.snapshot()? else {
        return Ok(false);
    };
    let entries = core.checkpoint_entries(snapshot)?;
    journal.compact(&entries)?;
    core.since_checkpoint = 0;
    tracing::info!(
        "world {}: journal trimmed at sequence {}",
        core.world_id,
        core.sequence
    );
    Ok(true)
}

/// The world's single authority: owns the core, the program instance and the
/// journal, and runs commands plus the program's capability needs serially.
pub(crate) struct WorldActor {
    pub(crate) core: WorldCore,
    /// Sessions and pending email codes. In memory only: a restart ends them.
    pub(crate) auth: crate::directory_auth::DirectoryAuth,
    pub(crate) program: Box<dyn WorldProgram>,
    pub(crate) journal: Option<Box<dyn WorldJournal>>,
    /// Host metrics registry (F11.7), shared with the handle and the
    /// session layer; see [`WorldMetrics`].
    pub(crate) metrics: WorldMetrics,
    pub(crate) storage_failed: bool,
    /// F12.13 replica mode: the world serves spectators from a verified
    /// journal and refuses every local mutation and side effect. Set only
    /// by the replica open path; a writer never runs this way.
    pub(crate) read_only: bool,
    pub(crate) program_healthy: bool,
    /// Durable transfers whose journal append succeeded but whose program
    /// apply failed (#3902): their receipts sit in the core dedupe ledger
    /// yet no live program has applied their events. The requirement is
    /// tracked independently of `program_healthy`: an exact retry must not
    /// acknowledge one of these receipts until a program — the current
    /// one or an installed replacement — has actually applied the event.
    pub(crate) pending_transfer_replays: Vec<HandoffTransferState>,
    /// Times a `view` or `layout` call trapped. Counted for `id world stats`
    /// (F11.17): a trapping view degrades to an error screen, not a poisoned
    /// world, so this counter is the only place the failure is recorded.
    pub(crate) view_failures: u64,
    pub(crate) compaction_off: bool,
    pub(crate) event_sender: broadcast::Sender<WorldEvent>,
    pub(crate) revocation_sender: broadcast::Sender<u64>,
    pub(crate) records_sender: watch::Sender<(u64, Arc<Records>)>,
    pub(crate) replication_sender: broadcast::Sender<ReplicationUpdate>,
    pub(crate) view_changed: watch::Sender<u64>,
    /// Live sessions per participant: presence is refcounted.
    pub(crate) presence: HashMap<u64, usize>,
    /// Subscriptions the program asked for (granted or refused-and-remembered).
    pub(crate) active_subs: BTreeSet<Subscription>,
    /// Request ids already executed, so `wants` states intent, not polling.
    pub(crate) handled_requests: BTreeSet<String>,
    /// Subscription specs already reported as refused.
    pub(crate) denied_subs: BTreeSet<String>,
    /// Armed tick intervals to their next fire time.
    pub(crate) ticks: BTreeMap<u64, std::time::Instant>,
    /// Restored schedules already overdue when the world opened. Their first
    /// fire is marked `late`; a cancel or fire removes the id.
    pub(crate) overdue_at_open: BTreeSet<String>,
    /// The hosting device's capability manifest (#2338), enumerated
    /// once by the adapter boundary and cached here: `device.info`
    /// serves this copy, so repeated program requests never trigger
    /// device re-enumeration. `None` when the host cannot describe
    /// its device — `device.info` then refuses with `unknown_cap`.
    pub(crate) device_manifest: Option<crate::world_caps::DeviceManifest>,
    /// Request ids the restored program already listed in `wants` when the
    /// world opened. Restart semantics: a reopen is not a fresh world. The
    /// requests may have executed before the restart, and
    /// [`handled_requests`](Self::handled_requests) is memory-only, so the
    /// actor cannot tell done from never-run. The first `after_change`
    /// passes therefore refuse these ids with `actor_denied` / `restarted`
    /// instead of executing them, then drop them from this set; ids the
    /// program issues after the open execute normally. Only
    /// [`spawn_restored`](WorldHandle::spawn_restored)
    /// seeds this set; every other constructor leaves it empty.
    pub(crate) stale_wants_at_open: BTreeSet<String>,
    /// The participant whose input is being processed. Requests the program
    /// makes in response are checked against that participant's scopes.
    pub(crate) acting: Option<u64>,
    /// The connection whose input is being processed. Ephemeral actor context;
    /// it is deliberately absent from `WorldEvent` and the journal.
    pub(crate) acting_session: Option<SessionId>,
}

impl WorldActor {
    fn audit_record(
        &self,
        actor: impl Into<String>,
        action: &str,
        parameters: serde_json::Value,
        outcome: &str,
    ) {
        let record = AuditRecord {
            schema_version: 1,
            timestamp_ms: self.core.clock().now_ms(),
            world_id: self.core.world_id().to_owned(),
            actor: actor.into(),
            action: action.to_owned(),
            parameters,
            outcome: outcome.to_owned(),
        };
        if let Ok(json) = serde_json::to_string(&record) {
            tracing::info!(target: AUDIT_TRACE_TARGET, audit_record = %json);
        } else {
            tracing::error!(target: AUDIT_TRACE_TARGET, action, "could not serialize audit record");
        }
    }

    fn audit_admin_result(
        &self,
        actor: impl Into<String>,
        action: &str,
        parameters: serde_json::Value,
        accepted: bool,
    ) {
        self.audit_record(
            actor,
            action,
            parameters,
            if accepted { "accepted" } else { "refused" },
        );
    }

    fn audit_provider_decision(&self, capability: &str, decision: &Decision, actor: String) {
        let outcome = match decision {
            Decision::Allow => "allowed",
            Decision::AllowAndGrant => "first_use_grant",
            Decision::Deny(_) => "denied",
        };
        self.audit_record(
            actor,
            "provider_use",
            serde_json::json!({"capability": capability}),
            outcome,
        );
    }

    pub(crate) async fn run(mut self, mut receiver: mpsc::Receiver<WorldCommand>) {
        self.after_change().await;
        loop {
            // Ticks and schedules sleep until their earliest deadline; with
            // neither armed this branch never wakes.
            let deadline = self.next_deadline();
            let command = tokio::select! {
                command = receiver.recv() => match command {
                    Some(command) => command,
                    None => break,
                },
                () = Self::sleep_until(deadline) => {
                    self.fire_ticks().await;
                    self.fire_schedules().await;
                    self.after_change().await;
                    continue;
                }
            };
            if matches!(command, WorldCommand::Shutdown { .. }) {
                if let WorldCommand::Shutdown { reply } = command {
                    let _ = reply.send(());
                }
                break;
            }
            self.metrics.record_queue_depth(receiver.len() as u64);
            let started = std::time::Instant::now();
            let name = command.name();
            async {
                self.handle(command).await;
            }
            .instrument(tracing::info_span!("command", name))
            .await;
            self.metrics.record_command(started.elapsed());
            self.after_change().await;
            self.acting = None;
            self.acting_session = None;
        }
    }

    async fn sleep_until(deadline: Option<std::time::Instant>) {
        match deadline {
            Some(deadline) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            }
            None => std::future::pending().await,
        }
    }

    /// The earliest moment the loop must wake: the soonest armed tick or
    /// pending schedule. Schedules wait only while the program can take them.
    /// A read-only replica never wakes for ticks or schedules: it serves
    /// spectators from its verified journal and fires nothing locally.
    pub(crate) fn next_deadline(&self) -> Option<std::time::Instant> {
        if self.read_only {
            return None;
        }
        let deliverable = self.program_healthy && !self.storage_failed;
        let clock = self.core.clock();
        self.ticks
            .values()
            .min()
            .copied()
            .into_iter()
            .chain(next_schedule_wake(
                &self.core.schedules,
                clock.now_ms(),
                clock.now_instant(),
                deliverable,
            ))
            .min()
    }

    fn check_storage(&self) -> Result<()> {
        ensure!(
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        Ok(())
    }

    /// Refuse a local mutation on a read-only replica (F12.13), or when
    /// storage failed on a writer. Replicas carry no journal to append to
    /// and must never originate authoritative events.
    fn check_not_replica(&self) -> Result<()> {
        ensure!(
            !self.read_only,
            "world is read-only (replica); local mutations are refused"
        );
        Ok(())
    }

    fn check_writable(&self) -> Result<()> {
        self.check_not_replica()?;
        self.check_storage()
    }

    fn record_issued(
        &mut self,
        issued: (Participant, JoinCapability),
    ) -> Result<(Participant, JoinCapability)> {
        let entry = self
            .core
            .issued_entry(issued.0.id)
            .context("issued capability vanished")?;
        self.persist(entry)?;
        Ok(issued)
    }

    /// Durably record `entry`, or turn the world read-only.
    fn persist(&mut self, entry: JournalEntry) -> Result<()> {
        ensure!(
            !self.read_only,
            "world is read-only (replica); local mutations are refused"
        );
        ensure!(
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        let world_id = self.core.world_id().to_owned();
        let replication_sender = self.replication_sender.clone();
        if let Some(journal) = self.journal.as_mut() {
            // The append includes its fsync, so this is the full durable
            // cost of the entry (F11.7 journal append + fsync latency).
            let started = std::time::Instant::now();
            let version_before = journal.replication_header_version();
            if let Err(error) = journal.append(&entry) {
                self.storage_failed = true;
                tracing::error!("world journal append failed: {error:#}");
                if version_before != journal.replication_header_version() {
                    let _ = replication_sender.send(ReplicationUpdate::ResetFailed);
                }
                return Err(error.context("world storage failed; the world is read-only"));
            }
            self.metrics.record_journal_append(started.elapsed());
            // Write locality (#2275): an entity-addressed input is a
            // region-scoped write, so it lands in the locality table
            // under its entity reference.
            if let JournalEntry::Committed { event } | JournalEntry::System { event } = &entry {
                if let WorldEventKind::EntityInput { entity_ref, .. } = &event.kind {
                    self.metrics.record_write_locality(entity_ref);
                }
            }
            if let Some(header_version) = journal.replication_header_version() {
                if version_before != Some(header_version) {
                    let update = journal
                        .replication_snapshot()
                        .map(ReplicationUpdate::Reset)
                        .unwrap_or_else(|error| {
                            tracing::error!(
                                "world {world_id}: replication reset snapshot failed: {error:#}"
                            );
                            ReplicationUpdate::ResetFailed
                        });
                    let _ = replication_sender.send(update);
                } else {
                    let _ = replication_sender.send(ReplicationUpdate::Append {
                        header_version,
                        entry: journal_entry_line(&entry),
                    });
                }
            }
        }
        Ok(())
    }

    /// Durably record related entries as one transaction, or turn the world
    /// read-only. Builder install events and their audit attribution must never
    /// be split across a crash.
    fn persist_transaction(&mut self, entries: &[JournalEntry]) -> Result<()> {
        ensure!(
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        if let Some(journal) = self.journal.as_mut() {
            let started = std::time::Instant::now();
            if let Err(error) = journal.append_transaction(entries) {
                self.storage_failed = true;
                tracing::error!("world journal transaction failed: {error:#}");
                return Err(error.context("world storage failed; the world is read-only"));
            }
            self.metrics.record_journal_append(started.elapsed());
        }
        Ok(())
    }

    fn export_entity(
        &mut self,
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
    ) -> Result<HandoffTransferState> {
        self.check_storage()?;
        ensure!(self.program_healthy, "world program is unavailable");
        ensure!(
            self.journal.is_some(),
            "entity export requires a durable world journal"
        );
        ensure!(
            self.core.limits.region_sharded,
            "entity export requires a region-sharded world"
        );
        ensure!(
            handoff_id.len() == 32 && handoff_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "handoff ID must be 128-bit hexadecimal"
        );
        core::validate_entity_ref(&entity_ref)?;
        for (name, value) in [
            ("source region", source_region.as_str()),
            ("target region", target_region.as_str()),
        ] {
            ensure!(!value.trim().is_empty(), "{name} must not be empty");
            ensure!(value.len() <= 256, "{name} exceeds 256 bytes");
            ensure!(
                !value.chars().any(char::is_control),
                "{name} contains a control character"
            );
        }
        ensure!(
            source_region != target_region,
            "source and target regions match"
        );
        let program_module_hash = self
            .core
            .active_module_hash
            .clone()
            .context("world has no installed program for entity export")?;
        // Exact-retry recovery: if this export already committed (its reply
        // was lost, or the actor restarted and replayed it), return the
        // stored receipt instead of preparing a duplicate the core would
        // refuse. A stored export whose fields differ from the request is
        // not a retry — it falls through and validate_entity_export
        // rejects the conflicting handoff ID / entity.
        if let Some(existing) = self.core.handoff_transfers.iter().find(|transfer| {
            transfer.direction == HandoffDirection::Export && transfer.handoff_id == handoff_id
        }) && existing.entity_ref == entity_ref
            && existing.source_region == source_region
            && existing.target_region == target_region
            && existing.prior_owner_epoch == prior_owner_epoch
            && existing.program_module_hash == program_module_hash
        {
            ensure!(
                !self.transfer_replay_pending(HandoffDirection::Export, &handoff_id),
                "entity export {handoff_id:?} is durably recorded but has not been applied; replay required"
            );
            return Ok(existing.clone());
        }
        let request = EntityTransferRequest::Export {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            prior_owner_epoch,
            program_module_hash: program_module_hash.clone(),
        };
        let EntityTransferPlan::Export {
            payload_schema,
            payload,
            app_event,
        } = self.program.prepare_entity_transfer(&request)?
        else {
            bail!("world program returned an import plan for an export request");
        };
        ensure!(
            !payload_schema.trim().is_empty() && payload_schema.len() <= 128,
            "entity transfer payload schema must be 1..=128 bytes"
        );
        ensure!(
            !payload_schema.chars().any(char::is_control),
            "entity transfer payload schema contains a control character"
        );
        ensure!(
            payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES,
            "entity transfer payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
        );
        let payload_digest = hex_encode(blake3::hash(&payload).as_bytes());
        let event_text = serde_json::json!({
            "protocol": "world.entity-transfer.v1",
            "direction": "export",
            "handoff_id": handoff_id,
            "entity_ref": entity_ref,
            "source_region": source_region,
            "target_region": target_region,
            "prior_owner_epoch": prior_owner_epoch,
            "module_hash": program_module_hash,
            "payload_schema": payload_schema,
            "payload_digest": payload_digest,
            "app_event": app_event,
        })
        .to_string();
        ensure!(
            event_text.len() <= crate::world_caps::MAX_EVENT_BYTES,
            "entity transfer event exceeds {} bytes",
            crate::world_caps::MAX_EVENT_BYTES
        );
        let source_event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: event_text },
        };
        let transfer = HandoffTransferState {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            direction: HandoffDirection::Export,
            prior_owner_epoch,
            new_owner_epoch: None,
            program_module_hash: program_module_hash.clone(),
            payload_schema: payload_schema.clone(),
            payload_digest: payload_digest.clone(),
            payload: Some(payload.clone()),
            event: source_event.clone(),
        };
        self.core.validate_entity_export(&transfer)?;

        // The durable entry is the source-side fence. Install it before
        // applying the prepared event so no subsequent command can mutate
        // this entity, even when the program rejects the post-append update.
        self.persist(JournalEntry::EntityExported {
            handoff_id,
            entity_ref,
            source_region,
            target_region,
            prior_owner_epoch,
            source_event: source_event.clone(),
            payload_schema,
            program_module_hash,
            payload,
            payload_digest,
        })?;
        self.core.record_entity_export(transfer.clone());
        if let Err(error) = self.program.update(&source_event) {
            self.pending_transfer_replays.push(transfer);
            self.program_healthy = false;
            tracing::error!("world program rejected a durable entity export: {error:#}");
            return Err(
                error.context("world program rejected a durable entity export; replay required")
            );
        }
        let records = match program_records(self.program.as_mut()) {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed after entity export: {error:#}");
                return Err(error.context(
                    "world program failed after a durable entity export; replay required",
                ));
            }
        };
        self.republished(records, self.core.sequence);
        let _ = self.view_changed.send(self.core.sequence);
        Ok(transfer)
    }

    /// Whether a durably recorded transfer is still waiting for a program
    /// to apply it (#3902): while it is, its stored receipt is withheld
    /// from exact retries instead of being acknowledged.
    fn transfer_replay_pending(&self, direction: HandoffDirection, handoff_id: &str) -> bool {
        self.pending_transfer_replays
            .iter()
            .any(|pending| pending.direction == direction && pending.handoff_id == handoff_id)
    }

    fn import_entity(
        &mut self,
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
        program_module_hash: String,
        payload_schema: String,
        payload: Vec<u8>,
        payload_digest: String,
    ) -> Result<HandoffTransferState> {
        self.check_storage()?;
        ensure!(self.program_healthy, "world program is unavailable");
        ensure!(
            self.journal.is_some(),
            "entity import requires a durable world journal"
        );
        ensure!(
            self.core.limits.region_sharded,
            "entity import requires a region-sharded world"
        );
        ensure!(
            handoff_id.len() == 32 && handoff_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "handoff ID must be 128-bit hexadecimal"
        );
        core::validate_entity_ref(&entity_ref)?;
        for (name, value) in [
            ("source region", source_region.as_str()),
            ("target region", target_region.as_str()),
        ] {
            ensure!(!value.trim().is_empty(), "{name} must not be empty");
            ensure!(value.len() <= 256, "{name} exceeds 256 bytes");
            ensure!(
                !value.chars().any(char::is_control),
                "{name} contains a control character"
            );
        }
        ensure!(
            source_region != target_region,
            "source and target regions match"
        );
        // Validate the envelope before any mutation (design #2818 \§3):
        // the module hash and payload schema must be ones this destination
        // runs, and the payload must hash to the digest the source recorded.
        let active_module_hash = self
            .core
            .active_module_hash
            .clone()
            .context("world has no installed program for entity import")?;
        ensure!(
            program_module_hash == active_module_hash,
            "entity import program module hash does not match the destination program"
        );
        ensure!(
            !payload_schema.trim().is_empty() && payload_schema.len() <= 128,
            "entity transfer payload schema must be 1..=128 bytes"
        );
        ensure!(
            !payload_schema.chars().any(char::is_control),
            "entity transfer payload schema contains a control character"
        );
        ensure!(
            payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES,
            "entity transfer payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
        );
        ensure!(
            payload_digest == hex_encode(blake3::hash(&payload).as_bytes()),
            "entity transfer payload digest does not match"
        );
        let new_owner_epoch = prior_owner_epoch
            .checked_add(1)
            .context("entity import owner epoch overflow")?;
        // Exact-retry recovery: if this import already committed (its reply
        // was lost, or the actor restarted and replayed it), return the
        // stored receipt instead of applying the entity twice. A stored
        // import whose fields differ from the request is not a retry \— it
        // falls through and validate_entity_import rejects the conflicting
        // handoff ID / entity, failing closed, never overwriting.
        if let Some(existing) = self.core.handoff_transfers.iter().find(|transfer| {
            transfer.direction == HandoffDirection::Import && transfer.handoff_id == handoff_id
        }) && existing.entity_ref == entity_ref
            && existing.source_region == source_region
            && existing.target_region == target_region
            && existing.prior_owner_epoch == prior_owner_epoch
            && existing.new_owner_epoch == Some(new_owner_epoch)
            && existing.program_module_hash == program_module_hash
            && existing.payload_schema == payload_schema
            && existing.payload_digest == payload_digest
        {
            ensure!(
                !self.transfer_replay_pending(HandoffDirection::Import, &handoff_id),
                "entity import {handoff_id:?} is durably recorded but has not been applied; replay required"
            );
            return Ok(existing.clone());
        }
        let request = EntityTransferRequest::Import {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            prior_owner_epoch,
            program_module_hash: program_module_hash.clone(),
            payload_schema: payload_schema.clone(),
            payload: payload.clone(),
            payload_digest: payload_digest.clone(),
        };
        let EntityTransferPlan::Import { app_event } =
            self.program.prepare_entity_transfer(&request)?
        else {
            bail!("world program returned an export plan for an import request");
        };
        let event_text = serde_json::json!({
            "protocol": "world.entity-transfer.v1",
            "direction": "import",
            "handoff_id": handoff_id,
            "entity_ref": entity_ref,
            "source_region": source_region,
            "target_region": target_region,
            "prior_owner_epoch": prior_owner_epoch,
            "new_owner_epoch": new_owner_epoch,
            "module_hash": program_module_hash,
            "payload_schema": payload_schema,
            "payload_digest": payload_digest,
            "payload_hex": hex_encode(&payload),
            "app_event": app_event,
        })
        .to_string();
        ensure!(
            event_text.len() <= crate::world_caps::MAX_EVENT_BYTES,
            "entity transfer event exceeds {} bytes",
            crate::world_caps::MAX_EVENT_BYTES
        );
        let destination_event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: event_text },
        };
        let transfer = HandoffTransferState {
            handoff_id: handoff_id.clone(),
            entity_ref: entity_ref.clone(),
            source_region: source_region.clone(),
            target_region: target_region.clone(),
            direction: HandoffDirection::Import,
            prior_owner_epoch,
            new_owner_epoch: Some(new_owner_epoch),
            program_module_hash: program_module_hash.clone(),
            payload_schema: payload_schema.clone(),
            payload_digest: payload_digest.clone(),
            payload: None,
            event: destination_event.clone(),
        };
        self.core.validate_entity_import(&transfer)?;

        // Persist-before-apply: the durable entry is the import receipt.
        // Record it in the dedupe ledger before applying the prepared
        // event, so a retry after a crash observes the receipt instead of
        // applying the entity twice. The receipt is returned only after
        // the program applies the event; a post-append failure withholds
        // it and recovery replays the durable entry.
        self.persist(JournalEntry::EntityImported {
            handoff_id,
            entity_ref,
            source_region,
            target_region,
            prior_owner_epoch,
            new_owner_epoch,
            program_module_hash,
            payload_schema,
            payload_digest,
            destination_event: destination_event.clone(),
        })?;
        self.core.record_entity_import(transfer.clone());
        if let Err(error) = self.program.update(&destination_event) {
            self.pending_transfer_replays.push(transfer);
            self.program_healthy = false;
            tracing::error!("world program rejected a durable entity import: {error:#}");
            return Err(
                error.context("world program rejected a durable entity import; replay required")
            );
        }
        let records = match program_records(self.program.as_mut()) {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed after entity import: {error:#}");
                return Err(error.context(
                    "world program failed after a durable entity import; replay required",
                ));
            }
        };
        self.republished(records, self.core.sequence);
        let _ = self.view_changed.send(self.core.sequence);
        Ok(transfer)
    }

    /// Publish freshly recomputed records. Participant events push their own
    /// view frame right after the event; host events signal the views watch.
    fn republished(&mut self, records: Records, sequence: u64) {
        publish_records(&self.records_sender, sequence, records);
    }

    /// Deliver a host-originated event to the program: journal it first is
    /// impossible (the program must accept it for the journal to be true), so
    /// the order is update, then journal, exactly like participant input.
    /// Returns whether the program accepted the event and it was journaled.
    /// A read-only replica never delivers: host events are authoritative
    /// program effects and must not originate from a replica.
    async fn deliver(&mut self, text: String) -> bool {
        if self.read_only {
            return false;
        }
        if !self.program_healthy || self.storage_failed {
            return false;
        }
        if text.len() > crate::world_caps::MAX_EVENT_BYTES {
            tracing::error!("world event exceeds the delivery bound; dropped");
            return false;
        }
        let event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: text },
        };
        if let Err(error) = self.program.update(&event) {
            self.program_healthy = false;
            tracing::error!("world program rejected a host event: {error:#}");
            return false;
        }
        match program_records(self.program.as_mut()) {
            Ok(records) => {
                let records = records.unwrap_or_default();
                publish_records(&self.records_sender, self.core.sequence, records);
            }
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed: {error:#}");
                return false;
            }
        }
        let journaled = self
            .persist(JournalEntry::System {
                event: event.clone(),
            })
            .is_ok();
        if journaled {
            self.core.record_admin_history(event);
            self.core.since_checkpoint = self.core.since_checkpoint.saturating_add(1);
        }
        let _ = self.view_changed.send(self.core.sequence);
        journaled
    }

    /// Recompute the program's `wants` and serve the delta: new subscriptions
    /// (armed or refused), new requests (executed, result delivered). Passes
    /// repeat while the program reacts to delivered results, so a chain of
    /// asks settles inside one change. A read-only replica never runs this:
    /// program requests are side effects (chat.say, schedule.at) that must
    /// not originate from a replica. Requests the program already listed
    /// at a restored open are refused, not executed (see
    /// [`stale_wants_at_open`](Self::stale_wants_at_open)).
    pub(crate) async fn after_change(&mut self) {
        if self.read_only {
            return;
        }
        for _ in 0..64 {
            if !self.program_healthy || self.storage_failed {
                break;
            }
            let Some(text) = (match self.program.wants() {
                Ok(wants) => wants,
                Err(error) => {
                    self.program_healthy = false;
                    tracing::error!("world program wants failed: {error:#}");
                    break;
                }
            }) else {
                break;
            };
            let wants = match crate::world_caps::parse_wants(&text) {
                Ok(wants) => wants,
                Err(error) => {
                    self.program_healthy = false;
                    tracing::error!("world program wants invalid: {error:#}");
                    break;
                }
            };
            self.apply_subscriptions(&wants).await;
            let mut fresh = Vec::new();
            for request in &wants.requests {
                if self.handled_requests.insert(request.id.clone()) {
                    fresh.push(request.clone());
                }
            }
            if fresh.is_empty() {
                break;
            }
            // A program inventing endless request ids cannot grow this set
            // without bound; old ids may then re-execute, which is the
            // documented at-least-once behavior.
            if self.handled_requests.len() > 4096 {
                let current: BTreeSet<String> = wants
                    .requests
                    .iter()
                    .map(|request| request.id.clone())
                    .collect();
                self.handled_requests.retain(|id| current.contains(id));
            }
            for request in fresh {
                // Pre-restart intent is never re-executed with world
                // authority: the side effect may already have happened, and
                // there is no acting participant to attribute a repeat to.
                // Refusing (rather than executing) keeps non-idempotent
                // providers (chat.say today; kv / http.fetch / notify as
                // they land) to at-most-once per id across a reopen; the
                // program re-issues the id afterwards if it still wants the
                // effect, and that new intent executes normally.
                if self.stale_wants_at_open.remove(&request.id) {
                    let refusal = crate::world_caps::result_event(
                        &request.id,
                        Err(CapError::actor_denied_with("restarted")),
                    );
                    self.deliver(refusal).await;
                    continue;
                }
                let outcome = self.execute(&request);
                self.deliver(crate::world_caps::result_event(&request.id, outcome))
                    .await;
            }
        }
        self.compact_if_due().await;
    }

    /// Subscribe new subscriptions, drop removed ones, refuse ungranted ones
    /// (once per spec), and keep ticks armed only while someone is present.
    async fn apply_subscriptions(&mut self, wants: &crate::world_caps::Wants) {
        for sub in &wants.subscribe {
            if !self.active_subs.insert(sub.clone()) {
                continue;
            }
            // The tick floor is a hard limit, not a grant question: a
            // sub-floor tick is refused like a denied subscription (once
            // per spec) and never armed, however it was granted.
            if let Subscription::Tick(ms) = sub
                && *ms < self.core.limits.min_tick_ms
            {
                let spec = sub.spec();
                if self.denied_subs.insert(spec.clone()) {
                    let error = crate::world_caps::CapError::bad_args_with(format!(
                        "time.tick interval {ms}ms is below the world minimum {}ms",
                        self.core.limits.min_tick_ms
                    ));
                    self.deliver(crate::world_caps::subscription_refused_event(&spec, &error))
                        .await;
                }
                continue;
            }
            let decision = self.core.ledger.decide(sub.cap());
            self.audit_provider_decision(sub.cap(), &decision, "program".to_owned());
            match decision {
                Decision::Allow => self.arm(sub),
                Decision::AllowAndGrant => {
                    let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
                    let _ = self.persist(JournalEntry::CapsGranted { granted });
                    self.arm(sub);
                }
                Decision::Deny(error) => {
                    let spec = sub.spec();
                    if self.denied_subs.insert(spec.clone()) {
                        self.deliver(crate::world_caps::subscription_refused_event(&spec, &error))
                            .await;
                    }
                }
            }
        }
        let current = wants.subscribe.clone();
        self.active_subs.retain(|sub| current.contains(sub));
        self.denied_subs
            .retain(|spec| current.iter().any(|sub| &sub.spec() == spec));
        if self.presence.values().all(|count| *count == 0) {
            self.ticks.clear();
        }
    }

    /// Arm a granted subscription's timer, if any and if anyone is present.
    /// A tick below the world's `min_tick_ms` floor is never armed.
    fn arm(&mut self, sub: &Subscription) {
        if let Subscription::Tick(ms) = sub
            && *ms >= self.core.limits.min_tick_ms
            && !self.presence.is_empty()
            && !self.ticks.contains_key(ms)
        {
            self.ticks.insert(
                *ms,
                self.core.clock().now_instant() + std::time::Duration::from_millis(*ms),
            );
        }
    }

    /// Deliver one tick event per due interval and re-arm.
    pub(crate) async fn fire_ticks(&mut self) {
        if self.read_only {
            return;
        }
        let now = self.core.clock().now_instant();
        let due: Vec<u64> = self
            .ticks
            .iter()
            .filter(|(_, next)| **next <= now)
            .map(|(ms, _)| *ms)
            .collect();
        for ms in due {
            self.ticks
                .insert(ms, now + std::time::Duration::from_millis(ms));
            let event = serde_json::json!({
                "cap": "time.tick",
                "interval": ms,
                // The tick's "now" reads the injected clock too, so a manual
                // clock drives the payload as well as the deadline.
                "now": self.core.clock().now_ms(),
            });
            self.deliver(event.to_string()).await;
        }
    }

    /// Fire the schedules that are due, earliest first, at most
    /// [`crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP`] per wakeup. Each
    /// fire is delivered as a tick is, and its schedule is consumed only after
    /// that delivery is journaled. A refused fire, or one that could not be
    /// journaled, leaves its schedule pending, so the next run delivers it.
    /// A read-only replica never fires: schedules are writer authority.
    pub(crate) async fn fire_schedules(&mut self) {
        if self.read_only {
            return;
        }
        let now = self.core.clock().now_ms();
        let mut due: Vec<(u64, String)> = self
            .core
            .schedules
            .iter()
            .filter(|(_, at_ms)| **at_ms <= now)
            .map(|(id, at_ms)| (*at_ms, id.clone()))
            .collect();
        due.sort_unstable();
        due.truncate(crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP);
        for (_, id) in due {
            let late = self.overdue_at_open.contains(&id);
            if !self.deliver(schedule_fired_event(&id, late)).await {
                break;
            }
            if self
                .persist(JournalEntry::ScheduleFired { id: id.clone() })
                .is_err()
            {
                break;
            }
            self.core.schedules.remove(&id);
            self.overdue_at_open.remove(&id);
        }
    }

    /// Execute one capability request. Anything the world has not granted is
    /// an error the program sees.
    fn execute(&mut self, request: &Request) -> Result<serde_json::Value, CapError> {
        let actor_permits = match self.acting {
            None => true,
            Some(participant) => self
                .core
                .effective_scopes(participant)
                .is_some_and(|scopes| scopes.allows_effect(&request.cap)),
        };
        let decision = self.core.ledger.decide_for(&request.cap, actor_permits);
        let actor = self
            .acting
            .map_or_else(|| "program".to_owned(), |id| format!("participant:{id}"));
        self.audit_provider_decision(&request.cap, &decision, actor);
        match decision {
            Decision::Allow => {}
            Decision::AllowAndGrant => {
                let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
                let _ = self.persist(JournalEntry::CapsGranted { granted });
            }
            Decision::Deny(error) => return Err(error),
        }
        match request.cap.as_str() {
            "time.now" => Ok(serde_json::json!({"now": unix_ms()})),
            "random.u64" => Ok(serde_json::json!({"value": rand::rng().random::<u64>()})),
            "players.list" => Ok(serde_json::json!({"players": self.players_list()})),
            "world.info" => Ok(serde_json::json!({
                "world": self.core.world_id,
                "sequence": self.core.sequence,
            })),
            "device.info" => self.execute_device_info(&request.args),
            "chat.say" => self.execute_chat(&request.args),
            "chat.history" => {
                // A supplied `count` must be an unsigned integer; only an
                // omitted count falls back to the default. Treating a
                // malformed count as omitted silently turns a client error
                // into a successful read (#7892).
                let count = match request.args.get("count") {
                    None => 16,
                    Some(value) => value
                        .as_u64()
                        .ok_or_else(CapError::bad_args)?
                        .min(crate::world_caps::MAX_CHAT_HISTORY_LINES as u64) as usize,
                };
                Ok(serde_json::json!({
                    "lines": chat_history(
                        &self.core.events,
                        count.min(crate::world_caps::MAX_CHAT_HISTORY_LINES),
                    ),
                }))
            }
            "records.read" => self.execute_records_read(&request.args),
            "schedule.at" => self.execute_schedule_at(&request.args),
            "schedule.cancel" => self.execute_schedule_cancel(&request.args),
            _ => Err(CapError::unknown()),
        }
    }

    /// `device.info` (#2338): the hosting device's capability
    /// manifest, in the v1 shape the taxonomy specifies. The manifest
    /// comes from the adapter boundary's cached copy — this provider
    /// never enumerates the device itself. A host that cannot
    /// describe its device refuses with `unknown_cap`, and a request
    /// naming a manifest version this host does not speak is refused
    /// rather than silently interpreted.
    fn execute_device_info(&self, args: &serde_json::Value) -> Result<serde_json::Value, CapError> {
        crate::world_caps::DeviceManifest::requested_version(args)?;
        let Some(manifest) = &self.device_manifest else {
            return Err(CapError::unknown());
        };
        Ok(manifest.to_json())
    }

    fn players_list(&self) -> Vec<serde_json::Value> {
        self.core
            .participants()
            .into_iter()
            .map(|participant| {
                serde_json::json!({
                    "id": participant.id,
                    "name": participant.display_name,
                    "present": self.presence.get(&participant.id).is_some_and(|c| *c > 0),
                })
            })
            .collect()
    }

    /// `records.read`: a byte-bounded, prefix-scoped page of the world's
    /// published records, key-ordered with the same exclusive `after` cursor
    /// the session protocol uses. Follow `next_after` while it is non-empty.
    /// The page is bounded by the cap-result budget
    /// (world_caps::MAX_RECORDS_RESULT_PAGE_BYTES), not the session
    /// protocol's larger page budget, so an admitted page always fits
    /// the result event that delivers it (#4888).
    fn execute_records_read(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let records = self.records_sender.borrow().1.clone();
        // #7899: a present-but-non-string prefix/after is bad_args,
        // not "omitted" — treating it as omitted silently widens a
        // scoped read to every record, or restarts pagination.
        let prefix = match args.get("prefix") {
            None => None,
            Some(value) => Some(value.as_str().ok_or(CapError::bad_args())?),
        };
        let after = match args.get("after") {
            None => None,
            Some(value) => Some(value.as_str().ok_or(CapError::bad_args())?),
        };
        let mut page: Vec<serde_json::Value> = Vec::new();
        let mut used = 0usize;
        let mut last_included: Option<String> = None;
        let mut more = false;
        for (key, value) in records.iter() {
            if let Some(prefix) = prefix
                && !key.starts_with(prefix)
            {
                continue;
            }
            if let Some(after) = after
                && key.as_str() <= after
            {
                continue;
            }
            // Count the entry as it will actually be serialized in
            // the result value, plus one byte for its array comma.
            let size = serde_json::to_vec(&serde_json::json!({"key": key, "value": value}))
                .map_or(usize::MAX, |entry| entry.len())
                + 1;
            if used + size > crate::world_caps::MAX_RECORDS_RESULT_PAGE_BYTES {
                if page.is_empty() {
                    return Err(CapError::bad_args());
                }
                more = true;
                break;
            }
            used += size;
            last_included = Some(key.clone());
            page.push(serde_json::json!({"key": key, "value": value}));
        }
        Ok(serde_json::json!({
            "records": page,
            "next_after": more.then_some(last_included).flatten(),
        }))
    }

    /// `chat.say` commits a host chat line; its result is the new sequence.
    fn execute_chat(&mut self, args: &serde_json::Value) -> Result<serde_json::Value, CapError> {
        let Some(text) = args.get("text").and_then(serde_json::Value::as_str) else {
            return Err(CapError::bad_args());
        };
        match self.core.prepare_system_chat(text) {
            Ok(event) => {
                if self
                    .persist(JournalEntry::Committed {
                        event: event.clone(),
                    })
                    .is_err()
                {
                    return Err(CapError::failed());
                }
                if self.core.commit_prepared(event.clone()).is_err() {
                    return Err(CapError::failed());
                }
                let _ = self.event_sender.send(event.clone());
                Ok(serde_json::json!({"sequence": event.sequence}))
            }
            Err(_) => Err(CapError::bad_args()),
        }
    }

    /// `schedule.at`: pend one fire of `id` at `at` (unix ms). The schedule is
    /// journaled before the result is delivered. A refused request changes
    /// nothing but the usage count.
    fn execute_schedule_at(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let id = args
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        let at_ms = args
            .get("at")
            .and_then(serde_json::Value::as_u64)
            .ok_or(CapError::bad_args())?;
        if let Err(error) = self
            .core
            .check_schedule(id, at_ms, self.core.clock().now_ms())
        {
            tracing::debug!("world {}: schedule refused: {error:#}", self.core.world_id);
            return Err(CapError::bad_args_with(format!("{error:#}")));
        }
        if self
            .persist(JournalEntry::ScheduleSet {
                id: id.to_owned(),
                at_ms,
            })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.schedules.insert(id.to_owned(), at_ms);
        Ok(serde_json::json!({"id": id, "fires_at": at_ms}))
    }

    /// `schedule.cancel`: drop a pending schedule before it fires. `cancelled`
    /// is false when nothing by that id is pending.
    fn execute_schedule_cancel(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let id = args
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or(CapError::bad_args())?;
        if crate::world_caps::validate_schedule_id(id).is_err() {
            return Err(CapError::bad_args());
        }
        if !self.core.schedules.contains_key(id) {
            return Ok(serde_json::json!({"cancelled": false}));
        }
        if self
            .persist(JournalEntry::ScheduleCancelled { id: id.to_owned() })
            .is_err()
        {
            return Err(CapError::failed());
        }
        self.core.schedules.remove(id);
        self.overdue_at_open.remove(id);
        Ok(serde_json::json!({"cancelled": true}))
    }

    /// A participant opened or closed a session: presence transitions deliver
    /// `players` events and arm or disarm ticks.
    async fn handle_presence(&mut self, participant: u64, joined: bool) {
        let was_present = self
            .presence
            .get(&participant)
            .is_some_and(|count| *count > 0);
        let count = self.presence.entry(participant).or_default();
        if joined {
            *count += 1;
        } else if *count > 0 {
            *count -= 1;
        }
        let now_present = *count > 0;
        if *count == 0 {
            self.presence.remove(&participant);
        }
        if now_present && !was_present && self.ticks.is_empty() {
            let intervals: Vec<u64> = self
                .active_subs
                .iter()
                .filter_map(|sub| match sub {
                    Subscription::Tick(ms)
                        if *ms >= self.core.limits.min_tick_ms
                            && self.core.ledger.is_granted(sub.cap()) =>
                    {
                        Some(*ms)
                    }
                    _ => None,
                })
                .collect();
            for ms in intervals {
                self.ticks.entry(ms).or_insert_with(|| {
                    self.core.clock().now_instant() + std::time::Duration::from_millis(ms)
                });
            }
        }
        if self.presence.is_empty() {
            self.ticks.clear();
        }
        if now_present != was_present
            && self.active_subs.contains(&Subscription::Players)
            && self.core.ledger.is_granted("players")
        {
            let name = self
                .core
                .participant(participant)
                .map_or_else(|| participant.to_string(), |p| p.display_name);
            self.deliver(presence_event(participant, &name, now_present))
                .await;
        }
    }

    fn require_moderation(&self, authority: &Authority, what: &str) -> Result<()> {
        let directory = self.core.directory();
        let allowed = match authority {
            Authority::Admin => true,
            Authority::Signed(key) => directory
                .account_for_key(key)
                .is_some_and(|account| directory.is_moderator(&account)),
        };
        if allowed {
            Ok(())
        } else {
            Err(crate::directory::Refusal::Forbidden(format!(
                "only an admin or a moderator {what}"
            ))
            .into())
        }
    }

    fn set_isolation(&mut self, authority: &Authority, isolation: Isolation) -> Result<Isolation> {
        self.check_writable()?;
        self.require_moderation(authority, "changes isolation")?;
        self.persist(JournalEntry::Isolation { isolation })?;
        self.core.isolation = isolation;
        Ok(isolation)
    }

    fn set_cross_world_write(
        &mut self,
        authority: &Authority,
        enabled: bool,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        self.require_moderation(authority, "changes cross-world writes")?;
        if !enabled && self.core.cross_world_forced && *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "the admin forced cross-world writes on; only the admin can turn them off"
                    .to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::CrossWorldWrite { enabled })?;
        self.core.cross_world_write = enabled;
        if !enabled {
            self.core.cross_world_forced = false;
        }
        Ok(self.core.policy())
    }

    fn force_cross_world_write(&mut self, authority: &Authority) -> Result<WorldPolicy> {
        self.check_writable()?;
        if *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "only the admin forces cross-world writes".to_owned(),
            )
            .into());
        }
        if self.core.isolation == Isolation::Isolated {
            return Err(crate::directory::Refusal::Forbidden(
                "an isolated world cannot be forced to accept cross-world writes; unisolate it first"
                    .to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::CrossWorldWriteForced)?;
        self.core.cross_world_write = true;
        self.core.cross_world_forced = true;
        Ok(self.core.policy())
    }

    fn set_file_scope(
        &mut self,
        authority: &Authority,
        scope: FileScope,
        write_override: Option<bool>,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        self.require_moderation(authority, "changes the file scope")?;
        if write_override.is_some() && *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "only the admin sets the file write override".to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::FileScope { scope })?;
        self.core.file_scope = scope;
        if let Some(enabled) = write_override {
            self.persist(JournalEntry::FileWriteOverride { enabled })?;
            self.core.file_write_override = enabled;
        }
        Ok(self.core.policy())
    }

    /// Replace the world's capability ceiling. It takes effect now: uses and
    /// subscriptions the ceiling excludes are refused from here on. Grants are
    /// kept, so lifting the ceiling again restores them.
    fn set_caps_ceiling(&mut self, ceiling: Vec<String>) -> Result<CapsReport> {
        self.check_writable()?;
        crate::world_caps::validate_ceiling_names(&ceiling)?;
        let ceiling: BTreeSet<String> = ceiling.into_iter().collect();
        self.persist(JournalEntry::CapsCeiling {
            ceiling: ceiling.iter().cloned().collect(),
        })?;
        self.core.caps_ceiling = ceiling;
        self.core.sync_ledger_ceiling();
        self.reconcile_subscriptions();
        self.caps_report()
    }

    /// Re-check subscriptions after an authority change. An armed one that is
    /// no longer granted is dropped and its tick disarmed. Every refused one is
    /// dropped too, so the next pass decides it again: a refusal that still
    /// holds is announced again, and one the change now grants is armed.
    fn reconcile_subscriptions(&mut self) {
        let ledger = &self.core.ledger;
        let refused = &self.denied_subs;
        self.active_subs
            .retain(|sub| !refused.contains(&sub.spec()) && ledger.is_granted(sub.cap()));
        self.denied_subs.clear();
        let active = &self.active_subs;
        self.ticks
            .retain(|ms, _| active.contains(&Subscription::Tick(*ms)));
    }

    /// The capability report an admin sees.
    fn caps_report(&mut self) -> Result<CapsReport> {
        let requested = match self.program.wants() {
            Ok(Some(text)) => crate::world_caps::parse_wants(&text)
                .map(|wants| wants.caps().into_iter().collect::<Vec<_>>())
                .unwrap_or_default(),
            Ok(None) => Vec::new(),
            Err(_) => Vec::new(),
        };
        let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
        let effective: Vec<String> = self.core.ledger.effective().into_iter().collect();
        let missing: Vec<String> = requested
            .iter()
            .filter(|cap| !effective.contains(cap))
            .cloned()
            .collect();
        Ok(CapsReport {
            granted,
            requested,
            missing,
            usage: self.core.ledger.usage().clone(),
            policy: if self.core.ledger.grant_on_use() {
                "grant-on-use"
            } else {
                "deny"
            }
            .to_owned(),
            catalog: crate::world_caps::PROVIDER_REGISTRY
                .ordered_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ceiling: self.core.caps_ceiling.iter().cloned().collect(),
            effective,
            view_failures: self.view_failures,
        })
    }

    pub(crate) async fn handle(&mut self, command: WorldCommand) {
        match command {
            WorldCommand::Issue {
                name,
                scopes,
                bounds,
                reply,
            } => {
                let result = self.check_writable().and_then(|()| {
                    let issued = self.core.issue_capability(&name, scopes, bounds)?;
                    self.record_issued(issued)
                });
                // Issuance grants participant access, so it belongs in the
                // audit trail alongside Revoke: record the non-secret
                // identity, scopes, bounds, and outcome. The capability
                // token itself is a secret and is never recorded.
                let parameters = match &result {
                    Ok((participant, _)) => serde_json::json!({
                        "participant_id": participant.id,
                        "name": participant.display_name,
                        "scopes": format!("{scopes:?}"),
                        "bounds": format!("{bounds:?}"),
                    }),
                    Err(_) => serde_json::json!({
                        "name": name,
                        "scopes": format!("{scopes:?}"),
                        "bounds": format!("{bounds:?}"),
                    }),
                };
                self.audit_admin_result("admin", "Issue", parameters, result.is_ok());
                let _ = reply.send(result);
            }
            WorldCommand::Attenuate {
                token,
                name,
                scopes,
                bounds,
                reply,
            } => {
                let result = self.check_writable().and_then(|()| {
                    let issued = self.core.attenuate(&token, scopes, bounds, &name)?;
                    self.record_issued(issued)
                });
                let _ = reply.send(result);
            }
            WorldCommand::DelegateToFriend {
                token,
                friend,
                name,
                scopes,
                bounds,
                reply,
            } => {
                let result = self.check_writable().and_then(|()| {
                    let issued = self
                        .core
                        .delegate_to_friend(&token, &friend, scopes, bounds, &name)?;
                    self.record_issued(issued)
                });
                let _ = reply.send(result);
            }
            WorldCommand::Directory {
                caller,
                action,
                outbox,
                reply,
            } => {
                // Directory facts are replica state: refuse local changes,
                // but keep the directory's read-only views and artifact
                // reads available through the same service.
                if self.read_only
                    && !matches!(
                        &action,
                        crate::directory_view::DirectoryAction::View
                            | crate::directory_view::DirectoryAction::ReadArtifacts { .. }
                    )
                {
                    let _ = reply.send(Err(anyhow::anyhow!(
                        "world is read-only (replica); local mutations are refused"
                    )));
                    return;
                }
                let world = self.core.world_id().to_owned();
                let now = unix_ms();
                let (directory, petnames) = self.core.directory_and_petnames_mut();
                // Refused before the directory changes, so a refused change leaves nothing half-applied.
                let result = if !outbox
                    && action.queues_for_other_servers(directory, &mut self.auth, &caller, now)
                {
                    Err(crate::directory::Refusal::Forbidden(
                        "this server cannot send to other servers".to_owned(),
                    )
                    .into())
                } else {
                    crate::directory_view::run(
                        directory,
                        petnames,
                        &mut self.auth,
                        &caller,
                        &world,
                        now,
                        action,
                    )
                };
                let _ = reply.send(result);
            }
            WorldCommand::ApplyDirectory { entry, reply } => {
                // See `Directory` above: replicas never apply local entries.
                let result = self
                    .check_not_replica()
                    .and_then(|()| self.core.directory_mut().apply(&entry));
                let _ = reply.send(result);
            }
            WorldCommand::Revoke {
                participant_id,
                by,
                reply,
            } => {
                // Revocation takes effect in memory even if storage failed on
                // a writer: denying access is always the safe side. A
                // read-only replica still refuses: revocations arrive through
                // the verified journal, never as local commands. Report
                // nothing revoked, like an unknown participant.
                if self.read_only {
                    // The replica refusal is a processed administrative
                    // outcome and belongs in the audit trail, like the
                    // writability refusals: emit exactly one refused
                    // record before returning (Codex finding on PR #4017).
                    self.audit_admin_result(
                        by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                        "Revoke",
                        serde_json::json!({"participant_id": participant_id, "changed": false}),
                        false,
                    );
                    let _ = reply.send(false);
                    return;
                }
                let revoked = self.core.revoke_tree(participant_id);
                let was_active = !revoked.is_empty();
                if was_active {
                    let _ = self.persist(JournalEntry::Revoked { participant_id, by });
                    for id in revoked {
                        let _ = self.revocation_sender.send(id);
                    }
                }
                self.audit_admin_result(
                    by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                    "Revoke",
                    serde_json::json!({"participant_id": participant_id, "changed": was_active}),
                    true,
                );
                let _ = reply.send(was_active);
            }
            WorldCommand::RevokeAuthorized {
                token,
                participant_id,
                reply,
            } => {
                let result = match self.core.authorize(&token, WorldScopes::MODERATOR) {
                    Err(error) => Err(error),
                    Ok(by) => {
                        if self.read_only {
                            self.audit_admin_result(
                                format!("participant:{by}"),
                                "RevokeAuthorized",
                                serde_json::json!({"participant_id": participant_id, "changed": false}),
                                false,
                            );
                            Ok(false)
                        } else {
                            let revoked = self.core.revoke_tree(participant_id);
                            let was_active = !revoked.is_empty();
                            if was_active {
                                // Fail-closed: the in-memory revocation and
                                // the disconnects stand even if the journal
                                // append fails, but the failure is surfaced
                                // rather than reported as an audited success
                                // (design #835 phase 2).
                                let persisted = self.persist(JournalEntry::Revoked {
                                    participant_id,
                                    by: Some(by),
                                });
                                for id in revoked {
                                    let _ = self.revocation_sender.send(id);
                                }
                                self.audit_admin_result(
                                    format!("participant:{by}"),
                                    "RevokeAuthorized",
                                    serde_json::json!({"participant_id": participant_id, "changed": was_active}),
                                    persisted.is_ok(),
                                );
                                match persisted {
                                    Ok(()) => Ok(true),
                                    Err(error) => Err(error),
                                }
                            } else {
                                Ok(false)
                            }
                        }
                    }
                };
                let _ = reply.send(result);
            }
            WorldCommand::ParticipantId { token, reply } => {
                let _ = reply.send(self.core.authorize(&token, WorldScopes::JOIN));
            }
            WorldCommand::AuthorizeScope {
                token,
                required,
                reply,
            } => {
                let _ = reply.send(self.core.authorize(&token, required));
            }
            WorldCommand::BindAuthorKey {
                token,
                author_key,
                reply,
            } => {
                // Author-key bindings change what later inputs the world
                // accepts; a replica takes no such local state.
                let result = self
                    .check_writable()
                    .and_then(|()| self.core.bind_author_key(&token, author_key));
                let _ = reply.send(result);
            }
            WorldCommand::Presence {
                participant,
                joined,
            } => {
                self.handle_presence(participant, joined).await;
            }
            WorldCommand::Chat { token, text, reply } => {
                let result = if self.storage_failed {
                    Err(anyhow::anyhow!(
                        "world storage failed; the world is read-only"
                    ))
                } else {
                    self.check_writable().and_then(|()| {
                        self.core.prepare_chat(&token, &text).and_then(|event| {
                            self.persist(JournalEntry::Committed {
                                event: event.clone(),
                            })?;
                            self.core.commit_prepared(event)
                        })
                    })
                };
                if let Ok(event) = &result {
                    let _ = self.event_sender.send(event.clone());
                }
                let _ = reply.send(result);
            }
            WorldCommand::Input {
                token,
                input,
                session_id,
                signed_move,
                reply,
            } => {
                // Refuse before touching the program: a replica must not
                // run participant inputs through its program instance.
                if let Err(error) = self.check_writable() {
                    let _ = reply.send(Err(error));
                    return;
                }
                let prepared = match &signed_move {
                    Some(bytes) => self
                        .core
                        .prepare_input_signed(&token, &input, bytes)
                        .map(|(event, signed)| (event, Some(signed))),
                    None => self
                        .core
                        .prepare_input(&token, &input)
                        .map(|event| (event, None)),
                };
                let result = match prepared {
                    Ok(_) if self.storage_failed => Err(anyhow::anyhow!(
                        "world storage failed; the world is read-only"
                    )),
                    Ok((candidate, signed)) if self.program_healthy => {
                        self.acting = Some(candidate.participant_id);
                        self.acting_session = session_id;
                        match self
                            .program
                            .update(&candidate)
                            .and_then(|()| program_records(self.program.as_mut()))
                        {
                            Ok(records) => self
                                .persist(JournalEntry::Committed {
                                    event: candidate.clone(),
                                })
                                .and_then(|()| self.core.commit_prepared(candidate))
                                .and_then(|event| {
                                    if let Some(signed) = &signed {
                                        self.core.record_signed_move(&event, signed)?;
                                    }
                                    Ok(event)
                                })
                                .inspect(|event| {
                                    let records = records.unwrap_or_default();
                                    self.republished(records, event.sequence);
                                }),
                            Err(error) => {
                                self.program_healthy = false;
                                Err(error.context("world program rejected input"))
                            }
                        }
                    }
                    Ok(_) => Err(anyhow::anyhow!(
                        "world program is unavailable after a previous failure"
                    )),
                    Err(error) => Err(error),
                };
                if let Ok(event) = &result {
                    let _ = self.event_sender.send(event.clone());
                }
                let _ = reply.send(result);
            }
            WorldCommand::EntityInput {
                token,
                entity_ref,
                input,
                reply,
            } => {
                // Refuse before touching the program: a replica must not
                // run entity inputs through its program instance.
                if let Err(error) = self.check_writable() {
                    let _ = reply.send(Err(error));
                    return;
                }
                let result = match self.core.prepare_entity_input(&token, &entity_ref, &input) {
                    Ok(_) if self.storage_failed => Err(anyhow::anyhow!(
                        "world storage failed; the world is read-only"
                    )),
                    Ok(candidate) if self.program_healthy => {
                        self.acting = Some(candidate.participant_id);
                        match self
                            .program
                            .update(&candidate)
                            .and_then(|()| program_records(self.program.as_mut()))
                        {
                            Ok(records) => self
                                .persist(JournalEntry::Committed {
                                    event: candidate.clone(),
                                })
                                .and_then(|()| self.core.commit_prepared(candidate))
                                .inspect(|event| {
                                    self.republished(records.unwrap_or_default(), event.sequence);
                                }),
                            Err(error) => {
                                self.program_healthy = false;
                                Err(error.context("world program rejected entity input"))
                            }
                        }
                    }
                    Ok(_) => Err(anyhow::anyhow!(
                        "world program is unavailable after a previous failure"
                    )),
                    Err(error) => Err(error),
                };
                if let Ok(event) = &result {
                    let _ = self.event_sender.send(event.clone());
                }
                let _ = reply.send(result);
            }
            WorldCommand::ExportEntity {
                handoff_id,
                entity_ref,
                source_region,
                target_region,
                prior_owner_epoch,
                reply,
            } => {
                let result = self.export_entity(
                    handoff_id,
                    entity_ref,
                    source_region,
                    target_region,
                    prior_owner_epoch,
                );
                let _ = reply.send(result);
            }
            WorldCommand::ImportEntity {
                handoff_id,
                entity_ref,
                source_region,
                target_region,
                prior_owner_epoch,
                program_module_hash,
                payload_schema,
                payload,
                payload_digest,
                reply,
            } => {
                let result = self.import_entity(
                    handoff_id,
                    entity_ref,
                    source_region,
                    target_region,
                    prior_owner_epoch,
                    program_module_hash,
                    payload_schema,
                    payload,
                    payload_digest,
                );
                let _ = reply.send(result);
            }
            WorldCommand::Snapshot { token, reply } => {
                let _ = reply.send(self.core.snapshot(&token));
            }
            WorldCommand::HostView { reply } => {
                let result = if self.program_healthy {
                    let viewer = Participant {
                        id: 0,
                        display_name: "Host".to_owned(),
                    };
                    self.program.view(&viewer).and_then(|view| {
                        if view
                            .as_ref()
                            .is_some_and(|text| text.len() > self.core.limits.presentation_bytes)
                        {
                            self.view_failures = self.view_failures.saturating_add(1);
                            anyhow::bail!(
                                "world presentation exceeds {} bytes",
                                self.core.limits.presentation_bytes
                            );
                        }
                        Ok((self.core.snapshot_unchecked(WorldScopes::ALL), view))
                    })
                } else {
                    Err(anyhow::anyhow!(
                        "world program is unavailable after a previous failure"
                    ))
                };
                let _ = reply.send(result);
            }
            WorldCommand::EventsAfter {
                token,
                sequence,
                reply,
            } => {
                let _ = reply.send(self.core.events_after(&token, sequence));
            }
            WorldCommand::ProjectEvent {
                token,
                event,
                reply,
            } => {
                let _ = reply.send(self.core.project_for(&token, &event));
            }
            WorldCommand::View { token, reply } => {
                let result = match self.core.viewer(&token) {
                    Ok(viewer) if self.program_healthy => match self.program.view(&viewer) {
                        Ok(view) => {
                            if let Some(text) = &view {
                                if text.len() > self.core.limits.presentation_bytes {
                                    self.view_failures = self.view_failures.saturating_add(1);
                                    Err(anyhow::anyhow!(
                                        "world presentation exceeds {} bytes",
                                        self.core.limits.presentation_bytes
                                    ))
                                } else {
                                    Ok(view)
                                }
                            } else {
                                Ok(None)
                            }
                        }
                        Err(error) => {
                            // A trapping view degrades to an error screen, not
                            // a poisoned world: keep the program healthy so the
                            // next event retries (F11.17).
                            self.view_failures = self.view_failures.saturating_add(1);
                            Err(error.context("world program view failed"))
                        }
                    },
                    Ok(_) => Err(anyhow::anyhow!(
                        "world program is unavailable after a previous failure"
                    )),
                    Err(error) => Err(error),
                };
                let _ = reply.send(result);
            }
            WorldCommand::ViewAt {
                token,
                width,
                height,
                reply,
            } => {
                let result = match self.core.viewer(&token) {
                    Ok(viewer) if self.program_healthy => {
                        match self.program.layout(&viewer, width, height) {
                            Ok(view) => {
                                if let Some(text) = &view {
                                    if text.len() > self.core.limits.presentation_bytes {
                                        self.view_failures = self.view_failures.saturating_add(1);
                                        // A trapping layout degrades to an error screen and does not poison the world.
                                        // Fall back to the default view, and crop to the requested dimensions.
                                        match self.program.view(&viewer) {
                                            Ok(fallback_view) => {
                                                if let Some(fallback_text) = fallback_view {
                                                    // #942: the fallback result is bound by the
                                                    // same presentation_bytes limit that sent us
                                                    // here. A cropped screen is checked after
                                                    // serialization; a serialization failure
                                                    // never returns the unchecked original text.
                                                    let bounded = if let Ok(Some(crate::world_presentation::WorldSurface::Screen(surface))) = crate::world_presentation::parse_surface(&fallback_text) {
                                                        let cropped = surface.fit(usize::from(width), usize::from(height));
                                                        serde_json::to_string(&cropped).ok()
                                                    } else {
                                                        Some(fallback_text)
                                                    };
                                                    match bounded {
                                                        Some(text)
                                                            if text.len()
                                                                <= self
                                                                    .core
                                                                    .limits
                                                                    .presentation_bytes =>
                                                        {
                                                            Ok(Some(text))
                                                        }
                                                        _ => {
                                                            self.view_failures = self
                                                                .view_failures
                                                                .saturating_add(1);
                                                            Err(anyhow::anyhow!(
                                                                "world presentation exceeds {} bytes",
                                                                self.core.limits.presentation_bytes
                                                            ))
                                                        }
                                                    }
                                                } else {
                                                    Ok(None)
                                                }
                                            }
                                            Err(error) => {
                                                Err(error
                                                    .context("world program view fallback failed"))
                                            }
                                        }
                                    } else {
                                        Ok(view)
                                    }
                                } else {
                                    Ok(None)
                                }
                            }
                            Err(error) => {
                                // As with `view`, a trapping layout degrades to
                                // an error screen and does not poison the world.
                                self.view_failures = self.view_failures.saturating_add(1);
                                Err(error.context("world program layout failed"))
                            }
                        }
                    }
                    Ok(_) => Err(anyhow::anyhow!(
                        "world program is unavailable after a previous failure"
                    )),
                    Err(error) => Err(error),
                };
                let _ = reply.send(result);
            }
            WorldCommand::InstallProgram {
                module_hash,
                seed,
                provenance,
                installed_by,
                trust,
                program: mut replacement,
                reply,
            } => {
                // Refuse before touching the replacement program: installs
                // are writer authority.
                if let Err(error) = self.check_writable() {
                    // The writability refusal is a processed administrative
                    // outcome and belongs in the audit trail, like the
                    // refusals whose writability checks run inside the
                    // recorded result below: emit exactly one record for it.
                    self.audit_admin_result(
                        installed_by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                        "InstallProgram",
                        serde_json::json!({"module_hash": module_hash, "seed": seed, "installed_by": installed_by}),
                        false,
                    );
                    let _ = reply.send(Err(error));
                    return;
                }
                // A program whose initial records are invalid is refused
                // before anything is journaled.
                let new_records = match program_records(replacement.as_mut()) {
                    Ok(records) => records.unwrap_or_default(),
                    Err(error) => {
                        self.audit_admin_result(
                            installed_by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                            "InstallProgram",
                            serde_json::json!({"module_hash": module_hash, "seed": seed, "installed_by": installed_by}),
                            false,
                        );
                        let _ = reply.send(Err(error.context("new world program records failed")));
                        return;
                    }
                };
                // Actor serialization makes activation and its event atomic
                // with respect to other world commands.
                let candidate = WorldEvent {
                    sequence: self.core.sequence.saturating_add(1),
                    participant_id: 0,
                    kind: WorldEventKind::ProgramInstalled {
                        module_hash: module_hash.clone(),
                        seed,
                        provenance: provenance.clone(),
                        trust: trust.clone(),
                    },
                };
                let install_event = JournalEntry::Committed {
                    event: candidate.clone(),
                };
                let persisted = if let Some(by) = installed_by {
                    // The builder authority leaves a journal-only audit
                    // record. Persist it atomically with the install event.
                    self.persist_transaction(&[
                        install_event,
                        JournalEntry::BuilderInstalled {
                            by,
                            module_hash: module_hash.clone(),
                            seed,
                        },
                    ])
                } else {
                    self.persist(install_event)
                };
                let result = persisted.and_then(|()| self.core.commit_prepared(candidate));
                let result = match result {
                    Ok(event) => {
                        if let WorldEventKind::ProgramInstalled { module_hash, .. } = &event.kind {
                            self.core.active_module_hash = Some(module_hash.clone());
                        }
                        self.core.active_module_provenance = provenance;
                        self.core.active_seed = seed;
                        self.program = replacement;
                        // The replacement starts from the same transfer
                        // history a journal restore would apply to it
                        // (#3902): restore retains every handoff transfer
                        // event across an install, so the live install
                        // replays the full retained history here too -
                        // not only the transfers the previous program
                        // never applied. Anything less would let a
                        // restart change application state. Transfers
                        // whose replay fails stay pending, their receipts
                        // stay withheld (see transfer_replay_pending),
                        // and health is declared only once none remain.
                        let mut still_pending = Vec::new();
                        let mut first_replay_error = None;
                        for transfer in self.core.handoff_transfers.clone() {
                            match self.program.update(&transfer.event) {
                                Ok(()) => {}
                                Err(error) => {
                                    tracing::error!(
                                        "world program rejected a transfer replay after install: {error:#}"
                                    );
                                    if first_replay_error.is_none() {
                                        first_replay_error = Some(error);
                                    }
                                    still_pending.push(transfer);
                                }
                            }
                        }
                        self.pending_transfer_replays = still_pending;
                        self.program_healthy = self.pending_transfer_replays.is_empty();
                        self.compaction_off = false;
                        // A new program starts with a clean slate.
                        self.handled_requests.clear();
                        self.denied_subs.clear();
                        self.active_subs.clear();
                        self.ticks.clear();
                        // The records computed before the replay describe
                        // the replacement without the transfers it just
                        // applied; recompute them from the replayed state
                        // before publishing, so an imported entity is
                        // visible in records() as soon as the install is
                        // acknowledged.
                        match program_records(self.program.as_mut()) {
                            Ok(records) => {
                                self.republished(records.unwrap_or_default(), event.sequence);
                                if let Some(error) = first_replay_error {
                                    // The install stands (it is journaled),
                                    // but recovery is incomplete: report the
                                    // replay failure as the install result
                                    // instead of acknowledging success while
                                    // the world stays unhealthy (#3902).
                                    Err(error.context(
                                        "world program rejected a retained transfer replay after install",
                                    ))
                                } else {
                                    let _ = self.event_sender.send(event.clone());
                                    Ok(event)
                                }
                            }
                            Err(error) => {
                                self.program_healthy = false;
                                tracing::error!(
                                    "world program records failed after transfer replay on install: {error:#}"
                                );
                                Err(error.context(
                                    "world program records failed after transfer replay on install",
                                ))
                            }
                        }
                    }
                    Err(error) => Err(error),
                };
                self.audit_admin_result(
                    installed_by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                    "InstallProgram",
                    serde_json::json!({"module_hash": module_hash, "seed": seed, "installed_by": installed_by}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::InstallProgramAuthorized {
                token,
                module_hash,
                seed,
                provenance,
                trust,
                program,
                reply,
            } => {
                // The actor authorizes the live capability at the mutation
                // boundary and derives the audit actor from it; no
                // caller-supplied participant ID is trusted (design #835
                // phase 2). On success the install runs through the exact
                // admin install path with `installed_by` set, so the
                // Committed + BuilderInstalled transaction is unchanged.
                match self.core.authorize(&token, WorldScopes::BUILDER) {
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                    Ok(by) => {
                        let (inner_reply, inner_result) = oneshot::channel();
                        Box::pin(self.handle(WorldCommand::InstallProgram {
                            module_hash,
                            seed,
                            provenance,
                            installed_by: Some(by),
                            trust,
                            program,
                            reply: inner_reply,
                        }))
                        .await;
                        let result = match inner_result.await {
                            Ok(result) => result,
                            Err(_) => Err(anyhow::anyhow!("world actor dropped install response")),
                        };
                        let _ = reply.send(result);
                    }
                }
            }
            WorldCommand::UpdateCaps {
                grant,
                revoke,
                reply,
            } => {
                let audit_parameters = serde_json::json!({
                    "grant": grant.clone(),
                    "revoke": revoke.clone(),
                });
                let result = (|| -> Result<CapsReport> {
                    self.check_writable()?;
                    crate::world_caps::validate_grant_names(&grant)?;
                    crate::world_caps::validate_grant_names(&revoke)?;
                    let mut granted: BTreeSet<String> =
                        self.core.ledger.granted().iter().cloned().collect();
                    for name in revoke {
                        granted.remove(&name);
                    }
                    granted.extend(grant);
                    self.persist(JournalEntry::CapsGranted {
                        granted: granted.iter().cloned().collect(),
                    })?;
                    self.core.ledger.set_granted(granted);
                    self.reconcile_subscriptions();
                    self.caps_report()
                })();
                self.audit_admin_result("admin", "UpdateCaps", audit_parameters, result.is_ok());
                let _ = reply.send(result);
            }
            WorldCommand::SetCapsCeiling { ceiling, reply } => {
                let audit_parameters = serde_json::json!({"ceiling": ceiling.clone()});
                let result = self.set_caps_ceiling(ceiling);
                self.audit_admin_result(
                    "admin",
                    "SetCapsCeiling",
                    audit_parameters,
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::SetExecutionProfile { profile, reply } => {
                let audit_parameters = serde_json::json!({"profile": format!("{profile:?}")});
                let result = self
                    .check_writable()
                    .and_then(|()| self.persist(JournalEntry::ExecutionProfile { profile }))
                    .map(|()| {
                        self.core.execution_profile = profile;
                        ExecutionProfileUpdate {
                            requested: profile,
                            applies_on_next_program_install_or_process_restart: true,
                        }
                    });
                self.audit_admin_result(
                    "admin",
                    "SetExecutionProfile",
                    audit_parameters,
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::ExecutionProfileInfo { reply } => {
                let profile = self.core.execution_profile;
                // No audit record, for the same reason as Metrics below:
                // the execution profile is readable by any joined
                // participant holding a capability (the session layer's
                // ExecutionProfileInfo frame), so an admin audit record
                // would misattribute a participant read.
                let _ = reply.send(profile);
            }
            WorldCommand::ManifestState { reply } => {
                let state = (self.core.active_seed, self.core.schedules.clone());
                // No audit record, for the same reason as Metrics below:
                // the session layer's WorldState frame accepts a
                // participant capability (only the seed is withheld from
                // non-admin callers there), so an admin audit record
                // would misattribute a participant read.
                let _ = reply.send(state);
            }
            WorldCommand::AdminScheduleAt { id, at_ms, reply } => {
                let audit_id = id.clone();
                let result = (|| -> Result<BTreeMap<String, u64>> {
                    self.check_writable()?;
                    if self.core.schedules.get(&id) == Some(&at_ms) {
                        return Ok(self.core.schedules.clone());
                    }
                    // Validate the replacement before anything is persisted:
                    // a refusal (say, past the horizon) must leave the
                    // existing schedule standing, in the journal as in
                    // memory (#807).
                    let replacing = self.core.schedules.contains_key(&id);
                    self.core
                        .check_schedule_replacing(&id, at_ms, self.core.clock().now_ms())?;
                    if replacing {
                        // Journal the cancel before the replacement set:
                        // replay refuses two pending sets for one id.
                        self.persist(JournalEntry::ScheduleCancelled { id: id.clone() })?;
                        self.core.schedules.remove(&id);
                        self.overdue_at_open.remove(&id);
                    }
                    self.persist(JournalEntry::ScheduleSet {
                        id: id.clone(),
                        at_ms,
                    })?;
                    self.core.schedules.insert(id, at_ms);
                    Ok(self.core.schedules.clone())
                })();
                self.audit_admin_result(
                    "admin",
                    "AdminScheduleAt",
                    serde_json::json!({"id": audit_id, "at_ms": at_ms}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::AdminCancelSchedule { id, reply } => {
                let result = (|| -> Result<BTreeMap<String, u64>> {
                    self.check_writable()?;
                    crate::world_caps::validate_schedule_id(&id)?;
                    if self.core.schedules.contains_key(&id) {
                        self.persist(JournalEntry::ScheduleCancelled { id: id.clone() })?;
                        self.core.schedules.remove(&id);
                        self.overdue_at_open.remove(&id);
                    }
                    Ok(self.core.schedules.clone())
                })();
                self.audit_admin_result(
                    "admin",
                    "AdminCancelSchedule",
                    serde_json::json!({"id": id.clone()}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::SetIsolation {
                authority,
                isolation,
                reply,
            } => {
                let result = self.set_isolation(&authority, isolation);
                self.audit_admin_result(
                    authority_identity(&authority),
                    "SetIsolation",
                    serde_json::json!({"isolation": format!("{isolation:?}")}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::IsolationInfo { reply } => {
                let isolation = self.core.isolation();
                // No audit record, for the same reason as Metrics below:
                // the isolation is readable without any caller identity
                // (the handle's isolation() takes no authority), so an
                // admin audit record would falsify the audit actor.
                let _ = reply.send(isolation);
            }
            WorldCommand::SetCrossWorldWrite {
                authority,
                enabled,
                reply,
            } => {
                let result = self.set_cross_world_write(&authority, enabled);
                self.audit_admin_result(
                    authority_identity(&authority),
                    "SetCrossWorldWrite",
                    serde_json::json!({"enabled": enabled}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::ForceCrossWorldWrite { authority, reply } => {
                let result = self.force_cross_world_write(&authority);
                self.audit_admin_result(
                    authority_identity(&authority),
                    "ForceCrossWorldWrite",
                    serde_json::json!({}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::SetFileScope {
                authority,
                scope,
                write_override,
                reply,
            } => {
                let result = self.set_file_scope(&authority, scope, write_override);
                self.audit_admin_result(
                    authority_identity(&authority),
                    "SetFileScope",
                    serde_json::json!({
                        "scope": format!("{scope:?}"),
                        "write_override": write_override,
                    }),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::PolicyInfo { reply } => {
                let policy = self.core.policy();
                // No audit record, for the same reason as Metrics below:
                // the policy is read by the cross-world gate on every
                // access check and by any handle holder, with no caller
                // identity, so an admin audit record would falsify the
                // audit actor and flood the trail.
                let _ = reply.send(policy);
            }
            WorldCommand::Metrics { reply } => {
                let sessions = self.presence.values().sum::<usize>() as u64;
                let metrics = self.metrics.snapshot(sessions);
                // No audit record: metrics are readable by any joined
                // participant holding a capability (the session layer's
                // Metrics frame), so recording the read as admin would
                // falsify the audit actor. Participant-readable queries are
                // not administrative actions and stay out of the audit trail.
                let _ = reply.send(metrics);
            }
            WorldCommand::AdminEventHistory { reply } => {
                let result = (|| -> Result<AdminEventHistory> {
                    ensure!(
                        !self.storage_failed,
                        "world storage failed; admin history is unavailable"
                    );
                    let records = self.records_sender.borrow().1.as_ref().clone();
                    Ok(AdminEventHistory {
                        world_id: self.core.world_id.clone(),
                        module_hash: self.core.active_module_hash.clone(),
                        current_sequence: self.core.sequence,
                        complete: self.core.admin_history_complete,
                        events: self.core.admin_history.iter().cloned().collect(),
                        records,
                    })
                })();
                let _ = reply.send(result);
            }
            WorldCommand::ReplicationFeed { reply } => {
                let result = if self.storage_failed {
                    Err(anyhow::anyhow!(
                        "world storage failed; replication feed is unavailable"
                    ))
                } else {
                    async {
                        // Subscribe before capturing the image. The actor is
                        // the only journal owner and processes commands
                        // serially, so no durable append can fall between
                        // these operations.
                        let updates = self.replication_sender.subscribe();
                        let world_id = self.core.world_id().to_owned();
                        let snapshot_source = self
                            .journal
                            .as_mut()
                            .ok_or_else(|| anyhow::anyhow!("world has no durable journal"))?
                            .replication_snapshot_source()?;

                        // A small journal cannot contain an over-bound line,
                        // so keep its existing fast path: no probe source and
                        // no attempt to serialize a checkpoint. A large one
                        // gets a second source as a probe for the CALLER to
                        // scan outside the actor (#8095): awaiting the scan
                        // here suspended every other command, tick, and
                        // schedule for the duration of a full journal read.
                        // If the probe finds an oversized entry, the caller
                        // sends ReplicationCompact (still a serialized actor
                        // operation) and builds its feed again from the
                        // compacted image.
                        let probe_source = if snapshot_source.byte_len
                            > super::replication::MAX_REPLICATION_PAYLOAD_BYTES as u64
                        {
                            Some(
                                self.journal
                                    .as_mut()
                                    .ok_or_else(|| anyhow::anyhow!("world has no durable journal"))?
                                    .replication_snapshot_source()?,
                            )
                        } else {
                            None
                        };

                        Ok((world_id, snapshot_source, probe_source, updates))
                    }
                    .await
                };
                let _ = reply.send(result);
            }
            WorldCommand::ReplicationCompact {
                ordinal,
                encoded_size,
                reply,
            } => {
                let result = self.force_compact_for_replication(ordinal, encoded_size);
                let _ = reply.send(result);
            }
            WorldCommand::CapsInfo { reply } => {
                let report = self.caps_report();
                // No audit record, for the same reason as Metrics above:
                // the capability report is readable by joined participants
                // (the session layer's Caps frame), so an admin audit
                // record would misattribute a participant read.
                let _ = reply.send(report);
            }
            WorldCommand::Shutdown { .. } => unreachable!("handled by run"),
        }
    }

    /// Trim the journal behind a verified snapshot, at most once per run of
    /// commits and never while the program cannot snapshot.
    async fn compact_if_due(&mut self) {
        if self.read_only
            || self.compaction_off
            || self.storage_failed
            || !self.program_healthy
            || !self.core.checkpoint_due()
        {
            return;
        }
        let started = std::time::Instant::now();
        let compacted = match self.journal.as_mut() {
            Some(journal) => {
                compact_journal(&mut self.core, self.program.as_mut(), journal.as_mut())
            }
            None => return,
        };
        match compacted {
            Ok(true) => {
                self.metrics.record_checkpoint(started.elapsed());
                self.publish_replication_reset();
            }
            Ok(false) => self.compaction_off = true,
            Err(error) => {
                // The journal is still complete; just do not retry until
                // something changes.
                tracing::warn!("world checkpoint skipped: {error:#}");
                self.compaction_off = true;
            }
        }
    }

    /// Re-compact a legacy journal only when a feed discovers an entry that
    /// cannot fit the v1 per-entry payload bound. Unlike policy-driven
    /// compaction, this intentionally ignores `checkpoint_due()`.
    fn force_compact_for_replication(&mut self, ordinal: usize, encoded_size: usize) -> Result<()> {
        let unavailable = if self.read_only {
            Some("the world is read-only")
        } else if self.compaction_off {
            Some("compaction was disabled after an earlier failure")
        } else if self.storage_failed {
            Some("world storage has failed")
        } else if !self.program_healthy {
            Some("the active program is unhealthy")
        } else if self.core.active_module_hash.is_none() {
            Some("there is no active program to checkpoint")
        } else if self.journal.is_none() {
            Some("the world has no durable journal to compact")
        } else {
            None
        };
        if let Some(reason) = unavailable {
            return Err(oversized_journal_compaction_error(
                ordinal,
                encoded_size,
                reason,
            ));
        }

        let started = std::time::Instant::now();
        let Some(journal) = self.journal.as_mut() else {
            return Err(oversized_journal_compaction_error(
                ordinal,
                encoded_size,
                "the world has no durable journal to compact",
            ));
        };
        let compacted = compact_journal(&mut self.core, self.program.as_mut(), journal.as_mut());
        match compacted {
            Ok(true) => {
                self.metrics.record_checkpoint(started.elapsed());
                self.publish_replication_reset();
                Ok(())
            }
            Ok(false) => {
                self.compaction_off = true;
                Err(oversized_journal_compaction_error(
                    ordinal,
                    encoded_size,
                    "the active program cannot provide a checkpoint snapshot",
                ))
            }
            Err(error) => {
                self.compaction_off = true;
                Err(oversized_journal_compaction_error(
                    ordinal,
                    encoded_size,
                    &format!("forced compaction failed: {error:#}"),
                ))
            }
        }
    }

    fn publish_replication_reset(&mut self) {
        let Some(journal) = self.journal.as_mut() else {
            return;
        };
        let update = journal
            .replication_snapshot()
            .map(ReplicationUpdate::Reset)
            .unwrap_or_else(|error| {
                tracing::error!(
                    "world {}: replication reset snapshot failed: {error:#}",
                    self.core.world_id()
                );
                ReplicationUpdate::ResetFailed
            });
        let _ = self.replication_sender.send(update);
    }
}

fn oversized_journal_compaction_error(
    ordinal: usize,
    encoded_size: usize,
    reason: &str,
) -> anyhow::Error {
    anyhow::anyhow!(
        "oversized legacy journal entry at ordinal {ordinal} is {encoded_size} encoded bytes, exceeding MAX_REPLICATION_PAYLOAD_BYTES={}; replication needs compaction before it can continue, but {reason}",
        super::replication::MAX_REPLICATION_PAYLOAD_BYTES
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod replication_feed_tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tempfile::TempDir;

    struct SnapshotProgram;

    impl WorldProgram for SnapshotProgram {
        fn snapshot(&mut self) -> Result<Option<String>> {
            Ok(Some("{}".to_owned()))
        }
    }

    struct CountingSnapshotProgram {
        snapshot_calls: Arc<AtomicUsize>,
    }

    impl WorldProgram for CountingSnapshotProgram {
        fn snapshot(&mut self) -> Result<Option<String>> {
            self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some("recompacted-state".to_owned()))
        }
    }

    struct FeedJournal {
        version: u32,
        entries: Vec<JournalEntry>,
        compactions: Arc<AtomicUsize>,
        source_captures: Arc<AtomicUsize>,
    }

    impl WorldJournal for FeedJournal {
        fn append(&mut self, entry: &JournalEntry) -> Result<()> {
            self.entries.push(entry.clone());
            Ok(())
        }

        fn replication_snapshot(&mut self) -> Result<JournalSnapshot> {
            Ok(JournalSnapshot {
                header_version: self.version,
                bytes: self.entries.iter().flat_map(journal_entry_line).collect(),
            })
        }

        fn replication_snapshot_source(&mut self) -> Result<JournalSnapshotSource> {
            self.source_captures.fetch_add(1, Ordering::SeqCst);
            let bytes: Vec<u8> = self.entries.iter().flat_map(journal_entry_line).collect();
            Ok(JournalSnapshotSource::from_reader(
                self.version,
                bytes.len() as u64,
                Cursor::new(bytes),
            ))
        }

        fn replication_header_version(&self) -> Option<u32> {
            Some(self.version)
        }

        fn compact(&mut self, entries: &[JournalEntry]) -> Result<()> {
            self.compactions.fetch_add(1, Ordering::SeqCst);
            self.entries = entries.to_vec();
            if let Some(JournalEntry::Created { version, .. }) = self.entries.first() {
                self.version = *version;
            }
            Ok(())
        }
    }

    fn legacy_oversized_checkpoint() -> Vec<JournalEntry> {
        vec![
            JournalEntry::Created {
                world_id: "lobby".to_owned(),
                version: 1,
            },
            JournalEntry::Checkpoint {
                sequence: 0,
                module_hash: "legacy-module".to_owned(),
                seed: 0,
                provenance: None,
                execution_profile: None,
                caps_ceiling: None,
                schedules: None,
                transfers: None,
                lease: None,
                snapshot: "x".repeat(super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES),
                events: Vec::new(),
                admin_history: None,
                open_presence: None,
            },
        ]
    }

    // ---- Property coverage for the oversized scanner (#7890 Codex P1) ----
    // Oracle for both properties: an independent first-over-bound-line scan
    // of the serialized journal lines — a line (entry bytes including the
    // trailing newline) is oversized iff its length exceeds
    // MAX_REPLICATION_PAYLOAD_BYTES, and the scanner must name the FIRST
    // such line's ordinal and length, or None when there is none.

    fn prop_filler(len: usize, special: u8) -> String {
        // Mostly plain bytes, with JSON-escaped and multibyte characters
        // mixed in so serialized sizes differ from char counts.
        let mut out = String::with_capacity(len);
        for i in 0..len {
            out.push(match (i as u8).wrapping_add(special) % 16 {
                0 => '"',
                1 => '\\',
                2 => '\u{e9}',
                3 => '\u{1f980}',
                _ => 'x',
            });
        }
        out
    }

    fn prop_checkpoint(snapshot: String) -> JournalEntry {
        JournalEntry::Checkpoint {
            sequence: 0,
            module_hash: "prop-module".to_owned(),
            seed: 0,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            transfers: None,
            lease: None,
            snapshot,
            events: Vec::new(),
            admin_history: None,
            open_presence: None,
        }
    }

    fn prop_entry() -> impl proptest::strategy::Strategy<Value = JournalEntry> {
        use proptest::prelude::*;
        prop_oneof![
            any::<u64>().prop_map(|participant_id| JournalEntry::Revoked {
                participant_id,
                by: None,
            }),
            proptest::collection::vec((0..24_usize, any::<u8>()), 0..=4).prop_map(|parts| {
                JournalEntry::CapsGranted {
                    granted: parts
                        .into_iter()
                        .map(|(len, special)| prop_filler(len, special))
                        .collect(),
                }
            }),
            // Snapshot sizes straddle the payload bound: below, around, above.
            prop_oneof![
                (0..=200_usize, any::<u8>()),
                (
                    super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES - 600
                        ..=super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES + 600,
                    any::<u8>(),
                ),
                (
                    super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES + 1
                        ..=super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES + 8192,
                    any::<u8>(),
                ),
            ]
            .prop_map(|(len, special)| prop_checkpoint(prop_filler(len, special))),
        ]
    }

    fn prop_bounded_entry() -> impl proptest::strategy::Strategy<Value = JournalEntry> {
        use proptest::prelude::*;
        // Every entry bounded far below the payload limit by construction;
        // the journal TOTAL still ranges past it via the entry count.
        prop_oneof![
            any::<u64>().prop_map(|participant_id| JournalEntry::Revoked {
                participant_id,
                by: None,
            }),
            (0..=20_000_usize, any::<u8>())
                .prop_map(|(len, special)| prop_checkpoint(prop_filler(len, special))),
        ]
    }

    #[test]
    fn scanner_matches_first_over_bound_line_oracle() {
        use proptest::prelude::*;
        proptest!(ProptestConfig::with_cases(48), |(entries in proptest::collection::vec(prop_entry(), 1..=10))| {
            let lines: Vec<Vec<u8>> = entries.iter().map(journal_entry_line).collect();
            let oracle = lines
                .iter()
                .enumerate()
                .find(|(_, line)| line.len() > super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES)
                .map(|(ordinal, line)| (ordinal, line.len()));
            let bytes: Vec<u8> = lines.into_iter().flatten().collect();
            let source = JournalSnapshotSource::from_reader(1, bytes.len() as u64, Cursor::new(bytes));
            let scanned = source.first_oversized_entry().unwrap();
            prop_assert_eq!(scanned, oracle);
        });
    }

    #[test]
    fn bounded_journals_never_compact_at_feed_build() {
        use proptest::prelude::*;
        proptest!(ProptestConfig::with_cases(24), |(entries in proptest::collection::vec(prop_bounded_entry(), 1..=24))| {
            // Oracle first: no serialized line may exceed the bound.
            let lines: Vec<Vec<u8>> = entries.iter().map(journal_entry_line).collect();
            prop_assert!(lines
                .iter()
                .all(|line| line.len() <= super::super::replication::MAX_REPLICATION_PAYLOAD_BYTES));
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let compactions = Arc::new(AtomicUsize::new(0));
                let source_captures = Arc::new(AtomicUsize::new(0));
                let journal = FeedJournal {
                    version: 1,
                    entries,
                    compactions: Arc::clone(&compactions),
                    source_captures: Arc::clone(&source_captures),
                };
                let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
                core.active_module_hash = Some("prop-module".to_owned());
                let mut actor = WorldActor::for_test(
                    core,
                    Box::new(SnapshotProgram),
                    Some(Box::new(journal)),
                );
                let (reply, response) = oneshot::channel();
                actor.handle(WorldCommand::ReplicationFeed { reply }).await;
                let (_world_id, _source, probe_source, _updates) =
                    response.await.unwrap().expect("feed capture");
                if let Some(probe) = probe_source {
                    // The caller-side scan (WorldHandle::replication_feed's
                    // step) must find nothing, so no ReplicationCompact is
                    // ever sent for a bounded journal.
                    let scanned = probe.first_oversized_entry().unwrap();
                    assert_eq!(scanned, None);
                }
                assert_eq!(compactions.load(Ordering::SeqCst), 0);
            });
        });
    }

    struct BumpingJournal {
        version: u32,
        entries: Vec<JournalEntry>,
    }

    impl WorldJournal for BumpingJournal {
        fn append(&mut self, entry: &JournalEntry) -> Result<()> {
            self.version = 2;
            if let Some(JournalEntry::Created { version, .. }) = self.entries.first_mut() {
                *version = self.version;
            }
            self.entries.push(entry.clone());
            Ok(())
        }

        fn replication_snapshot(&mut self) -> Result<JournalSnapshot> {
            Ok(JournalSnapshot {
                header_version: self.version,
                bytes: self.entries.iter().flat_map(journal_entry_line).collect(),
            })
        }

        fn replication_header_version(&self) -> Option<u32> {
            Some(self.version)
        }
    }

    /// Write a complete journal entry, then fail as though `sync_data`
    /// returned an error. The bytes remain visible to readers but are not
    /// confirmed durable, matching the failure window in `FileJournal`.
    struct SyncDataFailingJournal {
        path: std::path::PathBuf,
        version: u32,
        entries: Vec<JournalEntry>,
    }

    impl SyncDataFailingJournal {
        fn new(path: &std::path::Path) -> Self {
            let entries = vec![JournalEntry::Created {
                world_id: "lobby".to_owned(),
                version: 1,
            }];
            std::fs::write(
                path,
                entries
                    .iter()
                    .flat_map(journal_entry_line)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            Self {
                path: path.to_owned(),
                version: 1,
                entries,
            }
        }
    }

    impl WorldJournal for SyncDataFailingJournal {
        fn append(&mut self, entry: &JournalEntry) -> Result<()> {
            let required_version = journal_version_required_by(entry);
            if required_version > self.version {
                self.version = required_version;
                if let Some(JournalEntry::Created { version, .. }) = self.entries.first_mut() {
                    *version = required_version;
                }
            }
            self.entries.push(entry.clone());

            // This succeeds like `write_all`, leaving a complete newline-
            // terminated entry; the error models `sync_data` failing after
            // that write.
            std::fs::write(
                &self.path,
                self.entries
                    .iter()
                    .flat_map(journal_entry_line)
                    .collect::<Vec<_>>(),
            )?;
            Err(anyhow::anyhow!(
                "simulated sync_data failure after complete write"
            ))
        }

        fn replication_snapshot(&mut self) -> Result<JournalSnapshot> {
            Ok(JournalSnapshot {
                header_version: self.version,
                bytes: std::fs::read(&self.path)?,
            })
        }

        fn replication_header_version(&self) -> Option<u32> {
            Some(self.version)
        }
    }

    #[test]
    fn header_bump_publishes_a_reset_instead_of_a_tail() {
        let core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        let journal = BumpingJournal {
            version: 1,
            entries: vec![JournalEntry::Created {
                world_id: "lobby".to_owned(),
                version: 1,
            }],
        };
        let mut actor =
            WorldActor::for_test(core, Box::new(SnapshotProgram), Some(Box::new(journal)));
        let mut updates = actor.replication_sender.subscribe();

        actor
            .persist(JournalEntry::Isolation {
                isolation: Isolation::Unisolated,
            })
            .unwrap();

        let ReplicationUpdate::Reset(snapshot) = updates.try_recv().unwrap() else {
            panic!("header bump must publish a reset snapshot");
        };
        assert_eq!(snapshot.header_version, 2);
        let header = snapshot
            .bytes
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<JournalEntry>(header).unwrap(),
            JournalEntry::Created {
                world_id: "lobby".to_owned(),
                version: 2,
            }
        );
    }

    #[tokio::test]
    async fn new_replication_feed_refuses_complete_unconfirmed_entry_after_sync_failure() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let journal = SyncDataFailingJournal::new(&path);
        let core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        let mut actor =
            WorldActor::for_test(core, Box::new(SnapshotProgram), Some(Box::new(journal)));

        let unconfirmed = JournalEntry::Isolation {
            isolation: Isolation::Unisolated,
        };
        assert!(actor.persist(unconfirmed.clone()).is_err());
        assert!(actor.storage_failed);

        let bytes = std::fs::read(path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "the failed append wrote a complete line"
        );
        let written: Vec<JournalEntry> = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(written.last(), Some(&unconfirmed));

        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::ReplicationFeed { reply }).await;
        let feed = response.await.unwrap();
        assert!(
            feed.is_err(),
            "a new feed must not receive a snapshot containing the failed append"
        );
    }

    #[test]
    fn sync_failure_after_header_bump_publishes_reset_failed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let journal = SyncDataFailingJournal::new(&path);
        let core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        let mut actor =
            WorldActor::for_test(core, Box::new(SnapshotProgram), Some(Box::new(journal)));
        let mut updates = actor.replication_sender.subscribe();
        let profile = actor.core.execution_profile();

        let result = actor.persist(JournalEntry::ExecutionProfile { profile });

        assert!(result.is_err());
        assert_eq!(
            actor.journal.as_ref().unwrap().replication_header_version(),
            Some(2)
        );
        assert!(matches!(
            updates.try_recv().unwrap(),
            ReplicationUpdate::ResetFailed
        ));
    }

    #[tokio::test]
    async fn journal_compaction_publishes_a_reset_snapshot() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut limits = WorldLimits::default();
        limits.checkpoint_every = 1;
        let core = WorldCore::new("lobby", limits).unwrap();
        let (journal, _) = crate::world_store::FileJournal::open(&path, "lobby").unwrap();
        let mut actor =
            WorldActor::for_test(core, Box::new(SnapshotProgram), Some(Box::new(journal)));
        actor.core.active_module_hash = Some("test-module-hash".to_owned());
        actor.core.since_checkpoint = 1;
        let mut updates = actor.replication_sender.subscribe();

        actor.compact_if_due().await;

        let ReplicationUpdate::Reset(snapshot) = updates.recv().await.unwrap() else {
            panic!("compaction must publish a reset snapshot");
        };
        assert_eq!(snapshot.bytes, std::fs::read(path).unwrap());
        assert!(snapshot.bytes.ends_with(b"\n"));
    }

    #[tokio::test]
    async fn legacy_oversized_checkpoint_is_recompacted_at_feed_build() {
        use crate::world::{
            MAX_REPLICATION_PAYLOAD_BYTES, ReplicationEnvelope, ReplicationKind,
            ReplicationTracker, SignedReplicationEnvelope, chain_base_cursor, chain_payload,
            frame_digest, split_entry_payload,
        };

        let snapshot_calls = Arc::new(AtomicUsize::new(0));
        let compactions = Arc::new(AtomicUsize::new(0));
        let source_captures = Arc::new(AtomicUsize::new(0));
        let journal = FeedJournal {
            version: 1,
            entries: legacy_oversized_checkpoint(),
            compactions: Arc::clone(&compactions),
            source_captures: Arc::clone(&source_captures),
        };
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        // Restored legacy worlds are never `checkpoint_due` (nothing has been
        // committed since restore), so this must not matter to the feed.
        assert!(!core.checkpoint_due());
        core.active_module_hash = Some("test-module-hash".to_owned());
        let mut actor = WorldActor::for_test(
            core,
            Box::new(CountingSnapshotProgram {
                snapshot_calls: Arc::clone(&snapshot_calls),
            }),
            Some(Box::new(journal)),
        );
        let mut existing_updates = actor.replication_sender.subscribe();

        // The feed flow since #8095: the actor returns a probe source, the
        // CALLER scans it outside the actor, a serialized ReplicationCompact
        // command compacts, and the feed is rebuilt from the compacted image.
        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::ReplicationFeed { reply }).await;
        let (_world_id, _pre_compact_source, probe_source, _pre_compact_updates) =
            response.await.unwrap().expect("first feed capture");
        assert_eq!(
            compactions.load(Ordering::SeqCst),
            0,
            "the actor must not compact (or scan) during ReplicationFeed"
        );
        let probe = probe_source.expect("a >256 KiB journal must yield a probe source");
        let (ordinal, encoded_size) = probe
            .first_oversized_entry()
            .unwrap()
            .expect("the legacy fixture holds an oversized entry");
        let (reply, response) = oneshot::channel();
        actor
            .handle(WorldCommand::ReplicationCompact {
                ordinal,
                encoded_size,
                reply,
            })
            .await;
        response
            .await
            .unwrap()
            .expect("legacy oversized journal must re-compact at feed build");
        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::ReplicationFeed { reply }).await;
        let (_world_id, snapshot_source, _probe_source, mut updates) = response
            .await
            .unwrap()
            .expect("feed rebuilt after compaction");
        assert_eq!(compactions.load(Ordering::SeqCst), 1);
        assert_eq!(snapshot_calls.load(Ordering::SeqCst), 1);
        let ReplicationUpdate::Reset(reset) = existing_updates.try_recv().unwrap() else {
            panic!("feed-time compaction must publish a reset to existing feeds");
        };

        let snapshot = snapshot_source.read().unwrap();
        assert_eq!(reset.bytes, snapshot.bytes);
        let chunks = split_entry_payload(&snapshot.bytes).unwrap();
        assert!(!chunks.is_empty());
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.len() <= MAX_REPLICATION_PAYLOAD_BYTES)
        );

        // End to end: split chunks frame and verify exactly like the network
        // feed does, with a cursor chaining every accepted payload.
        const NONCE: [u8; 16] = [7; 16];
        let key = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
        let source = *key.verifying_key().as_bytes();
        let mut tracker =
            ReplicationTracker::new("lobby", source, snapshot.header_version, NONCE).unwrap();
        let mut previous_frame_digest = [0; 32];
        let mut cursor = chain_base_cursor("lobby", &source, snapshot.header_version);
        let mut received = Vec::new();
        for (index, payload) in chunks.iter().enumerate() {
            let envelope = if index == 0 {
                ReplicationEnvelope::snapshot(
                    "lobby",
                    source,
                    snapshot.header_version,
                    NONCE,
                    previous_frame_digest,
                    payload.clone(),
                )
                .unwrap()
            } else {
                ReplicationEnvelope::tail(
                    "lobby",
                    source,
                    snapshot.header_version,
                    NONCE,
                    cursor,
                    previous_frame_digest,
                    payload.clone(),
                )
                .unwrap()
            };
            cursor = envelope.cursor_after;
            let frame = SignedReplicationEnvelope::sign(envelope, &key)
                .unwrap()
                .encode_frame()
                .unwrap();
            previous_frame_digest = frame_digest(&frame);
            received.extend_from_slice(&tracker.accept(&frame).unwrap().payload);
        }
        let terminal = ReplicationEnvelope::terminal(
            "lobby",
            source,
            snapshot.header_version,
            NONCE,
            cursor,
            previous_frame_digest,
        );
        assert_eq!(
            tracker
                .accept(
                    &SignedReplicationEnvelope::sign(terminal, &key)
                        .unwrap()
                        .encode_frame()
                        .unwrap()
                )
                .unwrap()
                .kind,
            ReplicationKind::Terminal
        );
        assert_eq!(received, snapshot.bytes);
        let mut expected = chain_base_cursor("lobby", &source, snapshot.header_version);
        for chunk in split_entry_payload(&snapshot.bytes).unwrap() {
            expected = chain_payload(expected, &chunk).unwrap();
        }
        assert_eq!(tracker.cursor(), expected);
        assert!(
            updates.try_recv().is_err(),
            "the feed starts after its own compaction reset"
        );
    }

    #[tokio::test]
    async fn legacy_oversized_checkpoint_without_a_program_is_actionable() {
        use crate::world::MAX_REPLICATION_PAYLOAD_BYTES;

        let snapshot_calls = Arc::new(AtomicUsize::new(0));
        let compactions = Arc::new(AtomicUsize::new(0));
        let source_captures = Arc::new(AtomicUsize::new(0));
        let journal = FeedJournal {
            version: 1,
            entries: legacy_oversized_checkpoint(),
            compactions: Arc::clone(&compactions),
            source_captures: Arc::clone(&source_captures),
        };
        let core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        assert!(core.active_module_hash.is_none());
        let mut actor = WorldActor::for_test(
            core,
            Box::new(CountingSnapshotProgram {
                snapshot_calls: Arc::clone(&snapshot_calls),
            }),
            Some(Box::new(journal)),
        );

        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::ReplicationFeed { reply }).await;
        let (_world_id, _source, probe_source, _updates) =
            response.await.unwrap().expect("first feed capture");
        let probe = probe_source.expect("a >256 KiB journal must yield a probe source");
        let (ordinal, encoded_size) = probe
            .first_oversized_entry()
            .unwrap()
            .expect("the legacy fixture holds an oversized entry");
        let (reply, response) = oneshot::channel();
        actor
            .handle(WorldCommand::ReplicationCompact {
                ordinal,
                encoded_size,
                reply,
            })
            .await;
        let error = response
            .await
            .unwrap()
            .expect_err("compaction without a program must fail the feed");
        let message = format!("{error:#}");
        let legacy_bytes: Vec<u8> = legacy_oversized_checkpoint()
            .iter()
            .flat_map(journal_entry_line)
            .collect();
        let encoded_size = legacy_bytes
            .split_inclusive(|byte| *byte == b'\n')
            .nth(1)
            .expect("fixture must hold an oversized checkpoint")
            .len();
        assert!(
            message.contains("ordinal 1"),
            "error must name the entry ordinal, got: {message}"
        );
        assert!(
            message.contains(&encoded_size.to_string()),
            "error must name the encoded size, got: {message}"
        );
        assert!(
            message.contains(&MAX_REPLICATION_PAYLOAD_BYTES.to_string()),
            "error must name the bound, got: {message}"
        );
        assert!(
            message.contains("compaction"),
            "error must ask for compaction, got: {message}"
        );
        assert!(
            !message.contains("not a sequence of complete entries")
                && !message.contains("not complete journal entries"),
            "oversized-entry refusal must stay distinguishable from a malformed payload, got: {message}"
        );
        assert_eq!(compactions.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn oversized_probe_does_not_block_the_actor() {
        // #8095 acceptance: while a >256 KiB journal scan is in flight (here:
        // captured but deliberately not yet scanned), the actor keeps
        // processing other commands. Before the fix, ReplicationFeed awaited
        // the scan inside handle, so every command queued behind it.
        let snapshot_calls = Arc::new(AtomicUsize::new(0));
        let compactions = Arc::new(AtomicUsize::new(0));
        let source_captures = Arc::new(AtomicUsize::new(0));
        let journal = FeedJournal {
            version: 1,
            entries: legacy_oversized_checkpoint(),
            compactions: Arc::clone(&compactions),
            source_captures: Arc::clone(&source_captures),
        };
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("test-module-hash".to_owned());
        let mut actor = WorldActor::for_test(
            core,
            Box::new(CountingSnapshotProgram {
                snapshot_calls: Arc::clone(&snapshot_calls),
            }),
            Some(Box::new(journal)),
        );

        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::ReplicationFeed { reply }).await;
        let (_world_id, _source, probe_source, _updates) =
            response.await.unwrap().expect("first feed capture");
        let probe = probe_source.expect("a >256 KiB journal must yield a probe source");
        assert_eq!(compactions.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot_calls.load(Ordering::SeqCst), 0);

        // The scan has not run, yet the actor answers another command —
        // the probe waits on the caller's side of the command channel.
        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::CapsInfo { reply }).await;
        let _report = response.await.unwrap();

        // Now the caller scans, compacts through the serialized command,
        // and rebuilds the feed — the legacy outcome, without the stall.
        let (ordinal, encoded_size) = probe
            .first_oversized_entry()
            .unwrap()
            .expect("the legacy fixture holds an oversized entry");
        let (reply, response) = oneshot::channel();
        actor
            .handle(WorldCommand::ReplicationCompact {
                ordinal,
                encoded_size,
                reply,
            })
            .await;
        response.await.unwrap().expect("compaction succeeds");
        assert_eq!(compactions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn feed_without_an_oversized_entry_keeps_the_fast_path() {
        let snapshot_calls = Arc::new(AtomicUsize::new(0));
        let compactions = Arc::new(AtomicUsize::new(0));
        let source_captures = Arc::new(AtomicUsize::new(0));
        let journal = FeedJournal {
            version: 1,
            entries: vec![JournalEntry::Created {
                world_id: "lobby".to_owned(),
                version: 1,
            }],
            compactions: Arc::clone(&compactions),
            source_captures: Arc::clone(&source_captures),
        };
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("test-module-hash".to_owned());
        let mut actor = WorldActor::for_test(
            core,
            Box::new(CountingSnapshotProgram {
                snapshot_calls: Arc::clone(&snapshot_calls),
            }),
            Some(Box::new(journal)),
        );

        let (reply, response) = oneshot::channel();
        actor.handle(WorldCommand::ReplicationFeed { reply }).await;
        let (_world_id, snapshot_source, probe_source, mut updates) = response
            .await
            .unwrap()
            .expect("a small journal must keep the fast path");
        assert!(
            probe_source.is_none(),
            "a small journal cannot hold an over-bound entry: no probe (#8095)"
        );
        // Behavior, not timing: no compaction ran, the program was never
        // serialized, and the actor captured exactly one journal source.
        assert_eq!(compactions.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot_calls.load(Ordering::SeqCst), 0);
        assert_eq!(source_captures.load(Ordering::SeqCst), 1);
        assert!(snapshot_source.read().is_ok());
        assert!(updates.try_recv().is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod device_info_tests {
    use super::*;
    use crate::world_caps::{CapError, DeviceManifest};

    struct DeviceInfoProgram;

    impl WorldProgram for DeviceInfoProgram {
        fn snapshot(&mut self) -> Result<Option<String>> {
            Ok(Some("{}".to_owned()))
        }
    }

    fn granted_actor() -> WorldActor {
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["device.info".to_owned()]));
        WorldActor::for_test(core, Box::new(DeviceInfoProgram), None)
    }

    fn sample_manifest() -> DeviceManifest {
        DeviceManifest {
            display: serde_json::json!({"kind": "pixel", "width": 128, "height": 64, "colors": 1}),
            input: serde_json::json!({"keys": 8, "encoder": true, "pointer": false, "text_entry": "none"}),
            sensors: serde_json::json!({"gnss": {"accuracy_m": 5}, "microphone": false, "camera": false}),
            audio: serde_json::json!({"out": "pcm", "in": null}),
            network: serde_json::json!({"transport": "mqtt", "rtt_class": "slow"}),
        }
    }

    fn request(args: serde_json::Value) -> Request {
        Request {
            id: "d1".to_owned(),
            cap: "device.info".to_owned(),
            args,
        }
    }

    #[test]
    fn device_info_result_round_trips_through_the_result_event() {
        // Oracle: the declared v1 manifest contract (taxonomy §3.1) —
        // the value delivered in the normal result event is the
        // manifest the adapter boundary supplied.
        let mut actor = granted_actor();
        actor.device_manifest = Some(sample_manifest());
        let outcome = actor.execute(&request(serde_json::json!({})));
        let event = crate::world_caps::result_event("d1", outcome);
        let event: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["cap"], "result");
        assert_eq!(event["id"], "d1");
        assert_eq!(event["ok"], true);
        assert_eq!(event["value"], sample_manifest().to_json());
        assert_eq!(event["value"]["manifest"]["v"], 1);
    }

    #[test]
    fn device_info_refuses_an_unsupported_manifest_version() {
        // Oracle: the declared v1 manifest contract — a request for
        // any other version is refused, never silently interpreted.
        let mut actor = granted_actor();
        actor.device_manifest = Some(sample_manifest());
        let error = actor
            .execute(&request(serde_json::json!({"v": 2})))
            .unwrap_err();
        assert_eq!(error.code(), CapError::bad_args().code());
    }

    #[test]
    fn device_info_is_unknown_cap_when_the_host_cannot_describe_itself() {
        let mut actor = granted_actor();
        let error = actor.execute(&request(serde_json::json!({}))).unwrap_err();
        assert_eq!(error.code(), CapError::unknown().code());
    }
}

#[cfg(test)]
impl WorldActor {
    /// Assemble an actor without spawning its run loop, for tests that drive
    /// [`WorldActor::handle`], [`WorldActor::after_change`], [`WorldActor::fire_ticks`]
    /// and [`WorldActor::fire_schedules`] directly against a [`ManualClock`].
    pub(crate) fn for_test(
        core: WorldCore,
        mut program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
    ) -> Self {
        let (events, _) = broadcast::channel(256);
        let (revocations, _) = broadcast::channel(256);
        let (replication_sender, _) = broadcast::channel(256);
        let (view_changed, _) = watch::channel(core.sequence);
        let mut program_healthy = true;
        let initial = match program_records(program.as_mut()) {
            Ok(records) => records.unwrap_or_default(),
            Err(_) => {
                program_healthy = false;
                Records::new()
            }
        };
        let (records_sender, _) = watch::channel((core.sequence, Arc::new(initial)));
        // Schedules already due when the world opened fire late, judged by the
        // injected clock so a manual clock controls this too.
        let overdue_at_open: BTreeSet<String> = core
            .schedules
            .iter()
            .filter(|(_, at_ms)| **at_ms <= core.clock().now_ms())
            .map(|(id, _)| id.clone())
            .collect();
        Self {
            core,
            program,
            journal,
            storage_failed: false,
            read_only: false,
            program_healthy,
            pending_transfer_replays: Vec::new(),
            view_failures: 0,
            compaction_off: false,
            event_sender: events,
            revocation_sender: revocations,
            records_sender,
            replication_sender,
            view_changed,
            presence: HashMap::new(),
            active_subs: BTreeSet::new(),
            handled_requests: BTreeSet::new(),
            denied_subs: BTreeSet::new(),
            ticks: BTreeMap::new(),
            overdue_at_open,
            stale_wants_at_open: BTreeSet::new(),
            acting: None,
            acting_session: None,
            device_manifest: None,
            auth: crate::directory_auth::DirectoryAuth::default(),
            metrics: WorldMetrics::default(),
        }
    }
}
