/// Cloneable, serialized access to a world core. Every request passes through
/// one actor task, which defines the authoritative event order.
#[derive(Clone, Debug)]
pub struct WorldHandle {
    commands: mpsc::Sender<WorldCommand>,
    events: broadcast::Sender<WorldEvent>,
    revocations: broadcast::Sender<u64>,
    records: watch::Receiver<(u64, Arc<Records>)>,
    view_changed: watch::Receiver<u64>,
    /// The world's host metrics registry (F11.7), shared with the actor
    /// and the session layer.
    metrics: WorldMetrics,
    /// The world's retained audit log (F12.6), shared with the actor
    /// and the session layer.
    audit: WorldAuditLog,
    /// The world's client-provider broker (F13.7 step 3), shared with
    /// the actor and the session layer.
    provider_broker: ProviderBroker,
    /// Whether the actor runs in F12.13 read-only replica mode.
    read_only: bool,
    /// The F12.14 portal transport sink (code#1068), shared by all
    /// clones of this handle. It lives here, not on the actor, so
    /// send orchestration runs outside the actor's command loop:
    /// a send never waits on a peer actor while holding its own
    /// loop (the bidirectional/self-send deadlock CodeRabbit
    /// flagged on PR #10840).
    portal_delivery: PortalDeliverySlot,
}

use super::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::world_caps::{CapsReport, DeviceManifest};

impl WorldHandle {
    /// Spawn a world actor on the current Tokio runtime.
    pub fn spawn(core: WorldCore) -> Self {
        Self::spawn_with_program(core, Box::<EmptyWorldProgram>::default())
    }

    /// Spawn a world actor with an executable program. Only the actor owns
    /// the program instance, so calls are serialized with world mutations.
    pub fn spawn_with_program(core: WorldCore, program: Box<dyn WorldProgram>) -> Self {
        Self::spawn_durable(core, program, None)
    }

    /// Spawn a world actor for a world restored from durable state (see
    /// [`crate::world_store::open_world`]). Request ids the restored program
    /// already lists in `wants` are refused with `actor_denied` / `restarted`
    /// instead of re-executed: they may have run before the restart, and
    /// `handled_requests` is memory-only, so the actor cannot tell done from
    /// never-run. Requests the program issues after the open execute
    /// normally.
    pub fn spawn_restored(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
        device_manifest: Option<DeviceManifest>,
    ) -> Self {
        Self::spawn_durable_inner(
            core,
            program,
            journal,
            false,
            HashMap::new(),
            device_manifest,
            true,
        )
    }

    /// Spawn a world actor that journals every mutation before acknowledging
    /// it. `program` must already reflect the journal (see
    /// [`WorldCore::restore`]); the actor only appends.
    pub fn spawn_durable(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
    ) -> Self {
        Self::spawn_inner(core, program, journal, false, HashMap::new(), None)
    }

    /// Spawn a world actor whose host supplies a device manifest
    /// (#7942): the manifest is enumerated once, here at the adapter
    /// boundary, cached by the actor, and served by `device.info`
    /// for the life of the world — never re-enumerated per request.
    /// `None` keeps the historical behaviour: the host cannot
    /// describe its device and `device.info` refuses `unknown_cap`.
    pub fn spawn_with_device_manifest(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        device_manifest: Option<DeviceManifest>,
    ) -> Self {
        Self::spawn_durable_with_device_manifest(core, program, None, device_manifest)
    }

    /// [`Self::spawn_durable`] with a host-supplied device manifest
    /// (#7942); see [`Self::spawn_with_device_manifest`].
    pub fn spawn_durable_with_device_manifest(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
        device_manifest: Option<DeviceManifest>,
    ) -> Self {
        Self::spawn_inner(
            core,
            program,
            journal,
            false,
            HashMap::new(),
            device_manifest,
        )
    }

    /// The device manifest of a served host (#7942): a headless
    /// server process. It has no display, keys, sensors, or audio
    /// device of its own; programs reach it over the network.
    /// Hosts with real hardware describe themselves instead of
    /// using this.
    #[must_use]
    pub fn host_device_manifest() -> DeviceManifest {
        DeviceManifest {
            display: serde_json::json!({"kind": "headless"}),
            input: serde_json::json!({"keys": 0, "encoder": false, "pointer": false, "text_entry": "none"}),
            sensors: serde_json::json!({"gnss": null, "microphone": false, "camera": false}),
            audio: serde_json::json!({"out": null, "in": null}),
            network: serde_json::json!({"transport": "https"}),
        }
    }

    /// Spawn a replica actor over a verified, already-restored world.
    /// Local mutations and program side effects are refused; the replica
    /// carries no journal to append to. Only a core returned by
    /// [`WorldCore::restore_replica`] may be served this way.
    pub(crate) fn spawn_read_only(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        restored_presence: BTreeMap<u64, u64>,
    ) -> Result<Self> {
        if !core.replica_mode {
            anyhow::bail!("read-only actors require a replica-restored core");
        }
        let presence = restored_presence
            .into_iter()
            .map(|(id, count)| (id, usize::try_from(count).map_or(usize::MAX, |count| count)))
            .collect();
        Ok(Self::spawn_inner(core, program, None, true, presence, None))
    }

    /// As [`Self::spawn_read_only`], with a policy-B admission gate
    /// (F12.13 phase 5): the actor checks the gate before admitting a
    /// spectator who was not present at the verified cursor. A `None`
    /// gate is exactly [`Self::spawn_read_only`].
    pub(crate) fn spawn_read_only_with_admission(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        restored_presence: BTreeMap<u64, u64>,
        admission_gate: Option<ReplicaAdmissionGate>,
    ) -> Result<Self> {
        if !core.replica_mode {
            anyhow::bail!("read-only actors require a replica-restored core");
        }
        let presence = restored_presence
            .into_iter()
            .map(|(id, count)| (id, usize::try_from(count).map_or(usize::MAX, |count| count)))
            .collect();
        Ok(Self::spawn_durable_inner_gated(
            core,
            program,
            None,
            true,
            presence,
            None,
            false,
            admission_gate,
        ))
    }

    /// Whether this handle fronts a read-only replica actor.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn spawn_inner(
        core: WorldCore,
        mut program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
        read_only: bool,
        presence: HashMap<u64, usize>,
        device_manifest: Option<DeviceManifest>,
    ) -> Self {
        Self::spawn_durable_inner(
            core,
            program,
            journal,
            read_only,
            presence,
            device_manifest,
            false,
        )
    }

    /// Request ids the program lists in `wants` right now. A restored open
    /// captures these so the actor can refuse (rather than re-execute) the
    /// pre-restart intent; see
    /// [`WorldActor::stale_wants_at_open`](super::WorldActor::stale_wants_at_open).
    pub(crate) fn stale_wants_from(program: &mut dyn WorldProgram) -> BTreeSet<String> {
        match program.wants() {
            Ok(Some(text)) => crate::world_caps::parse_wants(&text)
                .map(|wants| {
                    wants
                        .requests
                        .iter()
                        .map(|request| request.id.clone())
                        .collect()
                })
                .unwrap_or_default(),
            _ => BTreeSet::new(),
        }
    }

    fn spawn_durable_inner(
        core: WorldCore,
        program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
        read_only: bool,
        presence: HashMap<u64, usize>,
        device_manifest: Option<DeviceManifest>,
        restored: bool,
    ) -> Self {
        Self::spawn_durable_inner_gated(
            core,
            program,
            journal,
            read_only,
            presence,
            device_manifest,
            restored,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_durable_inner_gated(
        core: WorldCore,
        mut program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
        read_only: bool,
        presence: HashMap<u64, usize>,
        device_manifest: Option<DeviceManifest>,
        restored: bool,
        admission_gate: Option<ReplicaAdmissionGate>,
    ) -> Self {
        let (commands, receiver) = mpsc::channel(256);
        let (events, _) = broadcast::channel(256);
        let (revocations, _) = broadcast::channel(256);
        let (replication_sender, _) = broadcast::channel(256);
        let (view_changed_sender, view_changed) = watch::channel(core.sequence);
        // Records are recomputed from the (restored) program, so they are a
        // pure function of the journal and need no storage of their own here.
        let mut program_healthy = true;
        let initial = match program_records(program.as_mut()) {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => {
                tracing::error!("world program records failed: {error:#}");
                program_healthy = false;
                Records::new()
            }
        };
        let (records_sender, records) = watch::channel((core.sequence, Arc::new(initial)));
        // Schedules the journal left pending that are already due fire late.
        let now = core.clock().now_ms();
        let overdue_at_open: BTreeSet<String> = core
            .schedules
            .iter()
            .filter(|(_, schedule)| schedule.at_ms <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let metrics = WorldMetrics::default();
        let audit = WorldAuditLog::new();
        let provider_broker = ProviderBroker::new(core.world_id.clone(), audit.clone());
        // A restored program's current `wants` predates the restart: its
        // request ids are captured now so the first `after_change` passes
        // refuse rather than re-execute them (see
        // [`WorldActor::stale_wants_at_open`]). Fresh worlds start empty.
        let stale_wants_at_open: BTreeSet<String> = if restored {
            Self::stale_wants_from(program.as_mut())
        } else {
            BTreeSet::new()
        };
        let actor = WorldActor {
            core,
            program,
            journal,
            metrics: metrics.clone(),
            audit: audit.clone(),
            provider_broker: provider_broker.clone(),
            storage_failed: false,
            read_only,
            program_healthy,
            pending_transfer_replays: Vec::new(),
            view_failures: 0,
            compaction_off: false,
            event_sender: events.clone(),
            revocation_sender: revocations.clone(),
            records_sender,
            replication_sender,
            view_changed: view_changed_sender,
            presence,
            active_subs: BTreeSet::new(),
            handled_requests: BTreeSet::new(),
            pending_results: std::collections::VecDeque::new(),
            fuel_retry_not_before: None,
            pending_forms: BTreeMap::new(),
            denied_subs: BTreeSet::new(),
            ticks: BTreeMap::new(),
            overdue_at_open,
            stale_wants_at_open,
            acting: None,
            acting_session: None,
            acting_device_input: None,
            device_manifest,
            location_consent: crate::world_caps::LocationConsent::ConsentRequired,
            location_fix: None,
            location_fix_received_at: None,
            loc_watches: BTreeMap::new(),
            schedule_delivery: None,
            portal_rate: BTreeMap::new(),
            auth: crate::directory_auth::DirectoryAuth::default(),
            admission_gate,
        };
        tokio::spawn(actor.run(receiver, restored));
        Self {
            commands,
            events,
            revocations,
            records,
            view_changed,
            metrics,
            audit,
            provider_broker,
            read_only,
            portal_delivery: PortalDeliverySlot::default(),
        }
    }

    /// The world's shared host metrics registry (F11.7): the session
    /// layer records frame bytes into it, the actor records the rest.
    #[must_use]
    pub fn shared_metrics(&self) -> WorldMetrics {
        self.metrics.clone()
    }

    /// The world's shared retained audit log (F12.6): the actor
    /// appends every record it emits.
    #[must_use]
    pub fn shared_audit_log(&self) -> WorldAuditLog {
        self.audit.clone()
    }

    /// The world's shared client-provider broker (F13.7 step 3): the
    /// actor begins requests through it, the session layer registers
    /// offers and replies and receives routed request frames.
    #[must_use]
    pub fn shared_provider_broker(&self) -> ProviderBroker {
        self.provider_broker.clone()
    }

    /// A snapshot of the world's retained audit records (F12.6),
    /// oldest first.
    #[must_use]
    pub fn audit_records(&self) -> Vec<AuditRecord> {
        self.audit.records()
    }

    /// A snapshot of the world's host metrics (F11.7), as surfaced by
    /// `id world stats`.
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed or drops the response.
    pub async fn metrics(&self) -> Result<MetricsSnapshot> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Metrics { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped metrics response")
    }

    /// Whether the world's program is still healthy: `false` once a
    /// guest call has failed inside the actor — a trap in the input
    /// call itself, or in the post-input `after_change` processing
    /// (such as `plaza_wants`) that the input triggered.
    ///
    /// The actor runs commands strictly in order and awaits
    /// `after_change` after each one before receiving the next, so
    /// querying this after an [`input`](Self::input) call returns only
    /// after that input's post-input processing has completed: the
    /// query is a barrier for post-input guest work.
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed or drops the response.
    pub async fn program_healthy(&self) -> Result<bool> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ProgramHealth { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped program health response")
    }

    /// Capture journal-order event history and its records for an admin export.
    /// The session layer must authorize the admin token before calling this.
    pub(crate) async fn admin_event_history(&self) -> Result<AdminEventHistory> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AdminEventHistory { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped admin history response")?
    }

    /// Capture a durable journal image and subscribe atomically to later
    /// appends and reset snapshots. The returned receiver is live before the
    /// snapshot is taken, preventing a write from being missed between them.
    ///
    /// A journal large enough to hold an over-bound entry is probed for a
    /// legacy oversized entry HERE, outside the actor (#8095): the scan is a
    /// full journal read, and awaiting it inside the actor would suspend all
    /// command processing until it finishes. When the probe finds one, the
    /// compaction itself is still a serialized actor operation
    /// ([`WorldCommand::ReplicationCompact`]); the feed is then rebuilt from
    /// the compacted image (the compaction publishes a reset to the first
    /// subscription, so it is replaced, exactly as the in-actor flow did).
    pub async fn replication_feed(
        &self,
    ) -> Result<(
        String,
        JournalSnapshotSource,
        broadcast::Receiver<ReplicationUpdate>,
    )> {
        let (world_id, snapshot_source, probe_source, updates) =
            self.replication_feed_snapshot().await?;
        let Some(probe) = probe_source else {
            return Ok((world_id, snapshot_source, updates));
        };
        let oversized = tokio::task::spawn_blocking(move || probe.first_oversized_entry())
            .await
            .context("scan replication journal for oversized entries")??;
        let Some((ordinal, encoded_size)) = oversized else {
            return Ok((world_id, snapshot_source, updates));
        };
        self.replication_compact(ordinal, encoded_size).await?;
        let (world_id, snapshot_source, _probe_source, updates) =
            self.replication_feed_snapshot().await?;
        Ok((world_id, snapshot_source, updates))
    }

    /// The authenticated sidecars for a replication snapshot of this
    /// world (F12.13 phase 5): the directory facts for `subjects` as
    /// JSONL entry bytes and the active module hash (whose bytes the
    /// caller loads from the world's module store). One actor round
    /// trip, so the facts are the ones in force at the actor's current
    /// state.
    ///
    /// # Errors
    ///
    /// Fails when the actor is closed or the directory facts cannot
    /// be derived for the given subjects.
    pub async fn replica_sidecars(
        &self,
        subjects: std::collections::BTreeSet<String>,
    ) -> Result<(Vec<u8>, Option<String>)> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ReplicaSidecars { subjects, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped replica sidecars response")?
    }

    /// One actor round trip: subscribe, capture the journal image, and —
    /// for a journal above the per-entry payload bound — capture a second
    /// source for the caller to probe outside the actor (#8095).
    async fn replication_feed_snapshot(
        &self,
    ) -> Result<(
        String,
        JournalSnapshotSource,
        Option<JournalSnapshotSource>,
        broadcast::Receiver<ReplicationUpdate>,
    )> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ReplicationFeed { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped replication feed response")?
    }

    /// Ask the actor to force-compact a journal whose out-of-actor probe
    /// found an oversized legacy entry. Serialized in the actor, like every
    /// other journal mutation.
    async fn replication_compact(&self, ordinal: usize, encoded_size: usize) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ReplicationCompact {
                ordinal,
                encoded_size,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped replication compact response")?
    }

    /// The latest `(sequence, records)` published by the world program.
    /// Callers must authorize the reader first (see [`Self::participant_id`]).
    #[must_use]
    pub fn records(&self) -> (u64, Arc<Records>) {
        let current = self.records.borrow();
        (current.0, Arc::clone(&current.1))
    }

    /// Watch record changes (coalesced: a slow reader sees the latest set).
    #[must_use]
    pub fn watch_records(&self) -> watch::Receiver<(u64, Arc<Records>)> {
        self.records.clone()
    }

    /// Subscribe to newly committed world events, as the host committed them:
    /// unprojected, with chat and the real install seed. Whatever forwards
    /// these to a participant must first pass each through
    /// [`WorldHandle::project_event`].
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<WorldEvent> {
        self.events.subscribe()
    }

    /// Subscribe to capability revocations.
    #[must_use]
    pub fn subscribe_revocations(&self) -> broadcast::Receiver<u64> {
        self.revocations.subscribe()
    }

    /// Watch for state changes a rendered view may want to reflect. The value
    /// is the world sequence at the change (unchanged for host events).
    #[must_use]
    pub fn watch_views(&self) -> watch::Receiver<u64> {
        self.view_changed.clone()
    }

    /// Tell the world a participant opened (`true`) or closed (`false`) a
    /// session. Drives presence and, while anyone is present, subscribed
    /// ticks.
    pub async fn presence(&self, participant: u64, joined: bool) {
        let _ = self
            .commands
            .send(WorldCommand::Presence {
                participant,
                joined,
            })
            .await;
    }

    /// Submit a deferred form answer from a trusted device presenter.
    ///
    /// The result is correlated against the pending `form_id`, validated, and
    /// delivered as a `form.result` system event. Rejections and unavailable
    /// forms are represented by [`FormSubmissionStatus`], not by the outer
    /// `Result`; callers must match the status rather than treating `Ok` as
    /// acceptance. A rejected answer remains pending with a fresh presentation
    /// token for the host to re-present, without a server-side retry loop.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is closed, delivery fails, or the
    /// serialized event exceeds [`crate::world_caps::MAX_EVENT_BYTES`]. An
    /// unknown or cancelled form returns `Ok(FormSubmissionStatus::Unavailable)`
    /// with reason `form_cancelled`.
    pub async fn submit_form_result(
        &self,
        result: id_core::world_presentation::FormResult,
    ) -> Result<FormSubmissionStatus> {
        self.submit_form_result_with_session(None, None, result)
            .await
    }

    /// Submit a form result from a joined session. The actor revalidates the
    /// session capability and the global `form.result` grant/ceiling.
    pub async fn submit_form_result_for_session(
        &self,
        token: JoinCapability,
        session_id: crate::world::SessionId,
        result: id_core::world_presentation::FormResult,
    ) -> Result<FormSubmissionStatus> {
        self.submit_form_result_with_session(Some(token), Some(session_id), result)
            .await
    }

    async fn submit_form_result_with_session(
        &self,
        token: Option<JoinCapability>,
        session_id: Option<crate::world::SessionId>,
        result: id_core::world_presentation::FormResult,
    ) -> Result<FormSubmissionStatus> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SubmitFormResult {
                token,
                session_id,
                result,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped form result response")?
    }

    /// Grant and revoke capabilities for this world, and return the report.
    ///
    /// # Errors
    ///
    /// Fails on unknown capability names or world storage failure.
    pub async fn update_caps(&self, grant: Vec<String>, revoke: Vec<String>) -> Result<CapsReport> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::UpdateCaps {
                grant,
                revoke,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped caps response")?
    }

    /// Replace this world's capability ceiling. It applies immediately to new
    /// uses and active subscriptions; it does not rewrite the grants.
    ///
    /// # Errors
    ///
    /// Fails on `*`, on unknown names, or on world storage failure.
    pub async fn set_caps_ceiling(&self, ceiling: Vec<String>) -> Result<CapsReport> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetCapsCeiling { ceiling, reply })
            .await
            .ok()
            .context("world actor is closed")?;
        response
            .await
            .context("world actor dropped caps ceiling response")?
    }

    /// Persist this world's requested resource profile. The current program
    /// instance is deliberately left untouched.
    pub async fn set_execution_profile(
        &self,
        profile: crate::world_limits::WorldExecutionProfile,
    ) -> Result<ExecutionProfileUpdate> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetExecutionProfile { profile, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped execution profile response")?
    }

    /// Durably replace this world's presentation policy (#14731 slice
    /// 2b) and return the stored policy.
    ///
    /// # Errors
    ///
    /// Fails on a policy past its size bounds, on a read-only world, or on
    /// world storage failure.
    pub async fn set_presentation_policy(
        &self,
        policy: id_core::presentation_policy::PresentationPolicy,
    ) -> Result<id_core::presentation_policy::PresentationPolicy> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetPresentationPolicy { policy, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped presentation policy response")?
    }

    /// Read this world's presentation policy. It is public pre-join
    /// metadata (#5970 §5).
    ///
    /// # Errors
    ///
    /// Fails only when the world actor is closed.
    pub async fn presentation_policy(
        &self,
    ) -> Result<id_core::presentation_policy::PresentationPolicy> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::PresentationPolicyInfo { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped presentation policy")
    }

    /// Read this world's currently requested resource profile.
    pub async fn execution_profile(&self) -> Result<crate::world_limits::WorldExecutionProfile> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ExecutionProfileInfo { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped execution profile")
    }

    /// Read the manifest-reconciliation state: the active install seed and
    /// the pending schedules (id to fire time, unix ms). Admin-layer only:
    /// the seed never enters a participant projection (#240).
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed.
    pub async fn manifest_state(&self) -> Result<(u64, BTreeMap<String, u64>)> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ManifestState { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped manifest state response")
    }

    /// Admin: pend one schedule (replacing any pending schedule with the
    /// same id), journaled before it is visible. Returns the pending map.
    ///
    /// # Errors
    ///
    /// A malformed id, a time past the horizon, a full schedule, or world
    /// storage failure.
    pub async fn admin_schedule_at(&self, id: String, at_ms: u64) -> Result<BTreeMap<String, u64>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AdminScheduleAt { id, at_ms, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped schedule response")?
    }

    /// Admin: set this world's schedule-delivery consent for one peer
    /// world (#1355 part 2): `send` lets this world's schedules target
    /// the peer, `receive` lets the peer's schedules be delivered here.
    /// Journaled before it is visible. Returns the full grant map.
    ///
    /// # Errors
    ///
    /// A malformed peer name or world storage failure.
    pub async fn admin_set_schedule_grant(
        &self,
        peer: String,
        send: bool,
        receive: bool,
    ) -> Result<BTreeMap<String, ScheduleGrant>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AdminSetScheduleGrant {
                peer,
                send,
                receive,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped schedule grant response")?
    }

    /// Deliver one cross-world schedule fire arriving from `source`
    /// (#1355 part 2). True only when this world durably accepted it
    /// (receive grant, fire journaled, receipt journaled) or already had.
    ///
    /// # Errors
    ///
    /// Fails only if the world actor is closed; refusal is `Ok(false)`.
    pub async fn deliver_schedule(
        &self,
        source: String,
        id: String,
        at_ms: u64,
        late: bool,
    ) -> Result<bool> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::DeliverSchedule {
                source,
                id,
                at_ms,
                late,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped schedule delivery response")
    }

    /// Install the cross-world schedule delivery sink used by this
    /// world's actor for its own outbound schedule fires (#1355 part 2).
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed.
    pub async fn set_schedule_delivery(
        &self,
        delivery: std::sync::Arc<dyn ScheduleDelivery>,
    ) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetScheduleDelivery { delivery, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped delivery-install response")
    }

    /// Admin: set this world's F12.14 portal grant for one peer world
    /// (code#1068 wiring slice): `granted: true` grants the portal to
    /// `peer`, `granted: false` revokes a live grant (a no-op for a
    /// peer never granted). Journaled before it is visible and
    /// audit-recorded like every admin command. Returns the full
    /// grant book after the change.
    ///
    /// # Errors
    ///
    /// A malformed peer name or world storage failure.
    pub async fn admin_set_portal_grant(
        &self,
        peer: String,
        granted: bool,
    ) -> Result<id_caps::portal::PortalGrants> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AdminSetPortalGrant {
                peer,
                granted,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped portal grant response")?
    }

    /// This world's current F12.14 portal grant book.
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed.
    pub async fn portal_grants(&self) -> Result<id_caps::portal::PortalGrants> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::PortalGrantsInfo { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped portal grants response")
    }

    /// Send one F12.14 portal message to `target` (code#1068 wiring
    /// slice). The send runs the mutual-grant check against both
    /// worlds' books, the 16 KiB size bound (and the delivered-
    /// envelope bound), and the per-ordered-pair rate window
    /// (32 per 60 s), and records the send-side audit entry; the
    /// target records the deliver-side entry on acceptance.
    ///
    /// The orchestration runs HERE, outside the actor's command
    /// loop, in five phases (the actor phases are local-only; the
    /// transport phases run while no actor loop is held):
    /// 1. actor: writability fence + this world's own grant;
    /// 2. transport: read the peer's grant book through the sink -
    ///    an unreadable book (no sink installed, unknown, busy, or
    ///    closed peer) is `Unavailable`, never a peer-not-granted
    ///    refusal, because no book was ever read;
    /// 3. actor: full mutual check + envelope bound + rate admit;
    /// 4. transport: deliver through the sink (the target's
    ///    delivery handler is local-only, so it answers promptly);
    /// 5. actor: record the send-side audit shape for the outcome.
    /// Because no actor ever awaits a peer while processing a
    /// command, simultaneous sends in both directions - and a send
    /// to one's own world - cannot deadlock the command loops
    /// (CodeRabbit finding on PR #10840).
    ///
    /// # Errors
    ///
    /// `PortalSendError::Refused` with the kernel's refusal for a
    /// policy refusal, or `PortalSendError::Unavailable` for a
    /// transport/fencing failure (including a closed actor).
    pub async fn send_portal_message(
        &self,
        target: String,
        message: String,
    ) -> Result<(), PortalSendError> {
        let bytes = message.len();
        // Phase 1 (actor-local).
        self.portal_send_precheck(&target, bytes).await?;
        // Phase 2 (transport, outside the actor).
        let Some(sink) = self.portal_delivery.get() else {
            return Err(PortalSendError::Unavailable(
                "no portal transport installed on this world".to_owned(),
            ));
        };
        let Some(peer_grants) = sink.peer_portal_grants(&target).await else {
            return Err(PortalSendError::Unavailable(format!(
                "portal peer {target} grant book could not be read"
            )));
        };
        // Phase 3 (actor-local): replies with this world's id.
        let from_world = self
            .portal_send_admit(&target, peer_grants, &message)
            .await?;
        // Phase 4 (transport, outside the actor).
        match sink.deliver_portal(&from_world, &target, &message).await {
            Ok(()) => {
                // Phase 5 (actor-local): the accepted audit shape.
                self.portal_send_record(&target, bytes, None).await
            }
            Err(PortalSendError::Refused(refusal)) => {
                // Phase 5: record the target's refusal on this side
                // too; the record call replies with that same
                // refusal, which is the error returned here.
                let _ = self
                    .portal_send_record(&target, bytes, Some(refusal.clone()))
                    .await;
                Err(PortalSendError::Refused(refusal))
            }
            Err(unavailable) => Err(unavailable),
        }
    }

    /// Phase 1 of [`WorldHandle::send_portal_message`]: the actor
    /// fences writability and checks this world's own grant book.
    async fn portal_send_precheck(
        &self,
        target: &str,
        bytes: usize,
    ) -> Result<(), PortalSendError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::PortalSendPrecheck {
                target: target.to_owned(),
                bytes,
                reply,
            })
            .await
            .map_err(|_| PortalSendError::Unavailable("world actor is closed".to_owned()))?;
        response.await.unwrap_or_else(|_| {
            Err(PortalSendError::Unavailable(
                "world actor dropped portal precheck response".to_owned(),
            ))
        })
    }

    /// Phase 3 of [`WorldHandle::send_portal_message`]: the actor
    /// runs the full mutual check against `peer_grants`, the
    /// envelope bound, and rate admission. Replies with this
    /// world's id on success.
    async fn portal_send_admit(
        &self,
        target: &str,
        peer_grants: PortalGrants,
        message: &str,
    ) -> Result<String, PortalSendError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::PortalSendAdmit {
                target: target.to_owned(),
                peer_grants,
                message: message.to_owned(),
                reply,
            })
            .await
            .map_err(|_| PortalSendError::Unavailable("world actor is closed".to_owned()))?;
        response.await.unwrap_or_else(|_| {
            Err(PortalSendError::Unavailable(
                "world actor dropped portal admit response".to_owned(),
            ))
        })
    }

    /// Phase 5 of [`WorldHandle::send_portal_message`]: the actor
    /// records the send-side audit shape for the observed outcome.
    async fn portal_send_record(
        &self,
        target: &str,
        bytes: usize,
        refusal: Option<PortalRefusal>,
    ) -> Result<(), PortalSendError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::PortalSendRecord {
                target: target.to_owned(),
                bytes,
                refusal,
                reply,
            })
            .await
            .map_err(|_| PortalSendError::Unavailable("world actor is closed".to_owned()))?;
        response.await.unwrap_or_else(|_| {
            Err(PortalSendError::Unavailable(
                "world actor dropped portal record response".to_owned(),
            ))
        })
    }

    /// Deliver one F12.14 portal message arriving from `source`
    /// (code#1068 wiring slice): the receiving half of the transport,
    /// called by the hub sink. `Ok` only when this world accepted it.
    ///
    /// # Errors
    ///
    /// A policy refusal or transport failure, as for
    /// [`WorldHandle::send_portal_message`].
    pub async fn deliver_portal_message(
        &self,
        source: String,
        message: String,
    ) -> Result<(), PortalSendError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::DeliverPortal {
                source,
                message,
                reply,
            })
            .await
            .map_err(|_| PortalSendError::Unavailable("world actor is closed".to_owned()))?;
        response.await.unwrap_or_else(|_| {
            Err(PortalSendError::Unavailable(
                "world actor dropped portal delivery response".to_owned(),
            ))
        })
    }

    /// Install the F12.14 portal transport sink used by this
    /// world's outbound portal sends (code#1068 wiring slice). The
    /// sink lives on the handle (shared by its clones), not on the
    /// actor: send orchestration runs outside the actor's command
    /// loop (see [`WorldHandle::send_portal_message`]).
    ///
    /// # Errors
    ///
    /// Never fails today; the `Result` keeps the install surface
    /// symmetric with the other composition installs.
    pub async fn set_portal_delivery(
        &self,
        delivery: std::sync::Arc<dyn PortalDelivery>,
    ) -> Result<()> {
        self.portal_delivery.set(delivery);
        Ok(())
    }

    /// Install the F12.14 portal transport sink only when none is
    /// installed yet (the hub's lease path, code#1068): a sink a
    /// world already carries - its own composition's - is never
    /// displaced by a lease. Returns whether it installed.
    pub(crate) fn install_portal_delivery(
        &self,
        delivery: std::sync::Arc<dyn PortalDelivery>,
    ) -> bool {
        self.portal_delivery.install(delivery)
    }

    /// Admin: cancel a pending schedule by id (a no-op when absent),
    /// journaled before it is visible. Returns the pending map.
    ///
    /// # Errors
    ///
    /// A malformed id or world storage failure.
    pub async fn admin_cancel_schedule(&self, id: String) -> Result<BTreeMap<String, u64>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AdminCancelSchedule { id, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped schedule response")?
    }

    /// Whether `authority` may change this world's lifecycle: `act` names
    /// the change ("wakes the world", "pauses the world", "archives the
    /// world") for the refusal. Only the world's owner (an admin seat in
    /// one of its groups) or the operator may (#10694); nothing changes.
    ///
    /// # Errors
    ///
    /// Fails with a forbidden refusal for any other authority, or if the
    /// world actor is closed.
    pub async fn authorize_lifecycle(&self, authority: Authority, act: &'static str) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AuthorizeLifecycle {
                authority,
                act,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped lifecycle authorization response")?
    }

    /// Change the world's isolation.
    ///
    /// # Errors
    ///
    /// Fails with a forbidden refusal unless the authority is an admin or a
    /// moderator, or on world storage failure.
    pub async fn set_isolation(
        &self,
        authority: Authority,
        isolation: Isolation,
    ) -> Result<Isolation> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetIsolation {
                authority,
                isolation,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped isolation response")?
    }

    /// The world's isolation.
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed.
    pub async fn isolation(&self) -> Result<Isolation> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::IsolationInfo { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped isolation response")
    }

    /// Let other unisolated worlds write here, or stop them.
    ///
    /// # Errors
    ///
    /// Fails with a forbidden refusal unless the authority is an admin or a
    /// moderator, or if a moderator turns off a forced setting, or on storage failure.
    pub async fn set_cross_world_write(
        &self,
        authority: Authority,
        enabled: bool,
    ) -> Result<WorldPolicy> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetCrossWorldWrite {
                authority,
                enabled,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped cross-world write response")?
    }

    /// Force cross-world writes on for an unisolated world.
    ///
    /// # Errors
    ///
    /// Fails with a forbidden refusal unless the authority is the admin, or if
    /// the world is isolated, or on storage failure.
    pub async fn force_cross_world_write(&self, authority: Authority) -> Result<WorldPolicy> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ForceCrossWorldWrite { authority, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped force response")?
    }

    /// Set the file scope, and with `write_override` the admin override.
    ///
    /// # Errors
    ///
    /// Fails with a forbidden refusal unless the authority is an admin or a
    /// moderator, or if a moderator sets the override, or on storage failure.
    pub async fn set_file_scope(
        &self,
        authority: Authority,
        scope: FileScope,
        write_override: Option<bool>,
    ) -> Result<WorldPolicy> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::SetFileScope {
                authority,
                scope,
                write_override,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped file scope response")?
    }

    /// The world's sharing and file settings.
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed.
    pub async fn policy(&self) -> Result<WorldPolicy> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::PolicyInfo { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped policy response")
    }

    /// The world's capability report: grants, what the program wants, and use.
    ///
    /// # Errors
    ///
    /// Fails if the world actor is closed.
    pub async fn caps_report(&self) -> Result<CapsReport> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::CapsInfo { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped caps response")?
    }

    /// Resolve the participant authorized by a join capability.
    pub async fn participant_id(&self, token: JoinCapability) -> Result<u64> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ParticipantId { token, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped participant lookup response")?
    }

    /// Authorize a live capability for `required` scopes, returning the
    /// derived participant ID (design #835 phase 3). The world actor
    /// performs the check against the capability's current scope,
    /// revocation state, and expiry; the error is the actor's
    /// non-oracular "invalid, revoked, or insufficient world
    /// capability" for every denial cause. Session paths use this as
    /// the early check before receiving upload bytes — the authorized
    /// operation commands still re-check at the effect boundary.
    pub async fn authorize(&self, token: JoinCapability, required: WorldScopes) -> Result<u64> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::AuthorizeScope {
                token,
                required,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped authorization response")?
    }

    /// Issue a scoped participant capability.
    pub async fn issue(
        &self,
        name: impl Into<String>,
        scopes: WorldScopes,
    ) -> Result<(Participant, JoinCapability)> {
        self.issue_bounded(name, scopes, CapabilityBounds::default())
            .await
    }

    /// Issue a scoped participant capability that stops working at its bounds.
    pub async fn issue_bounded(
        &self,
        name: impl Into<String>,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
    ) -> Result<(Participant, JoinCapability)> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Issue {
                name: name.into(),
                scopes,
                bounds,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped capability response")?
    }

    /// Delegate a narrower capability from `token`, which must hold `JOIN`.
    pub async fn attenuate(
        &self,
        token: JoinCapability,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        name: impl Into<String>,
    ) -> Result<(Participant, JoinCapability)> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Attenuate {
                token,
                name: name.into(),
                scopes,
                bounds,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped delegation response")?
    }

    /// Delegate a narrower capability to a friend of the account `token`
    /// acts for. The friendship must still hold when the capability is used.
    pub async fn delegate_to_friend(
        &self,
        token: JoinCapability,
        friend: impl Into<String>,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        name: impl Into<String>,
    ) -> Result<(Participant, JoinCapability)> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::DelegateToFriend {
                token,
                friend: friend.into(),
                name: name.into(),
                scopes,
                bounds,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped friend delegation response")?
    }

    /// Read or change the directory as the caller. The caller's proof must
    /// already be checked by the transport. `outbox` says whether this server
    /// can queue what the change sends to other servers.
    pub async fn directory(
        &self,
        caller: crate::directory_auth::Caller,
        action: crate::directory_view::DirectoryAction,
        outbox: bool,
    ) -> Result<crate::directory_view::DirectoryOutcome> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Directory {
                caller,
                action,
                outbox,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped directory response")?
    }

    /// Apply a change prepared by a remote friend action, once its envelope is queued.
    pub async fn apply_directory_entry(
        &self,
        entry: crate::directory::DirectoryEntry,
    ) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ApplyDirectory { entry, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped directory apply response")?
    }

    /// Whether this world's directory holds `account` — `None` when it
    /// does not, otherwise whether the account is verified (#8538). This
    /// is the directory capability authorization consults. Fails on a
    /// read-only replica, which never applies a local directory entry.
    pub async fn directory_account(&self, account: impl Into<String>) -> Result<Option<bool>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::DirectoryAccount {
                account: account.into(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped directory account response")?
    }

    /// Revoke an active capability and everything delegated from it.
    pub async fn revoke(&self, participant_id: u64) -> Result<bool> {
        self.revoke_by(participant_id, None).await
    }

    /// Revoke an active capability and everything delegated from it, recording
    /// the moderator participant that ordered it for the audit journal.
    pub async fn revoke_by(&self, participant_id: u64, by: Option<u64>) -> Result<bool> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Revoke {
                participant_id,
                by,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped revocation response")
    }

    /// Revoke a participant on the authority of a joined moderator's live
    /// capability (design #835 phase 2). The world actor authorizes the
    /// capability itself and derives the audit actor from it. Returns
    /// `Err` when the capability is invalid, expired, revoked, or lacks
    /// `MODERATOR`, and when the revoke's journal append fails (the
    /// in-memory revocation still stands in that case).
    pub async fn revoke_authorized(
        &self,
        token: JoinCapability,
        participant_id: u64,
    ) -> Result<bool> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::RevokeAuthorized {
                token,
                participant_id,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped revocation response")?
    }

    /// Submit a bounded chat message.
    pub async fn chat(&self, token: JoinCapability, text: impl Into<String>) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Chat {
                token,
                text: text.into(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped chat response")?
    }

    /// Submit bounded opaque game input.
    pub async fn input(&self, token: JoinCapability, input: Vec<u8>) -> Result<WorldEvent> {
        self.input_with_session(token, input, None, None).await
    }

    /// Submit bounded opaque game input from a live session connection.
    pub(crate) async fn input_from_session(
        &self,
        token: JoinCapability,
        input: Vec<u8>,
        session_id: SessionId,
        device_input: Option<serde_json::Value>,
    ) -> Result<WorldEvent> {
        self.input_with_session(token, input, Some(session_id), device_input)
            .await
    }

    async fn input_with_session(
        &self,
        token: JoinCapability,
        input: Vec<u8>,
        session_id: Option<SessionId>,
        device_input: Option<serde_json::Value>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Input {
                token,
                input,
                session_id,
                device_input,
                signed_move: None,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped input response")?
    }

    /// Bind the calling participant's author key for signed play (#2814).
    /// Once bound, the participant's inputs must be player-signed.
    pub async fn bind_author_key(
        &self,
        token: JoinCapability,
        author_key: [u8; 32],
    ) -> Result<u64> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::BindAuthorKey {
                token,
                author_key,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped bind response")?
    }

    /// Submit a player-signed game input (#2814): `signed_move` is the
    /// chain event the player signed over exactly `input` (see
    /// [`super::signed::sign_move`]).
    pub async fn input_signed(
        &self,
        token: JoinCapability,
        input: Vec<u8>,
        signed_move: Vec<u8>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Input {
                token,
                input,
                session_id: None,
                device_input: None,
                signed_move: Some(signed_move),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped input response")?
    }

    /// Submit bounded input addressed to one application-owned entity.
    pub async fn entity_input(
        &self,
        token: JoinCapability,
        entity_ref: impl Into<String>,
        input: Vec<u8>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::EntityInput {
                token,
                entity_ref: entity_ref.into(),
                input,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped entity input response")?
    }

    /// Export an entity from this source shard. The actor journals the
    /// complete bounded payload and source event before applying the event and
    /// returns the durable source receipt only after application succeeds.
    pub async fn export_entity(
        &self,
        handoff_id: impl Into<String>,
        entity_ref: impl Into<String>,
        source_region: impl Into<String>,
        target_region: impl Into<String>,
        prior_owner_epoch: u64,
    ) -> Result<HandoffTransferState> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ExportEntity {
                handoff_id: handoff_id.into(),
                entity_ref: entity_ref.into(),
                source_region: source_region.into(),
                target_region: target_region.into(),
                prior_owner_epoch,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped entity export response")?
    }

    /// Import an entity into this destination shard. The envelope is
    /// validated (module hash, payload schema, payload digest) before any
    /// mutation; the actor journals the import event in one
    /// `EntityImported` entry before applying it and returns the durable
    /// receipt only after application succeeds. An exact retry returns
    /// the existing receipt; a conflicting reuse of the handoff ID fails
    /// closed.
    pub async fn import_entity(
        &self,
        handoff_id: impl Into<String>,
        entity_ref: impl Into<String>,
        source_region: impl Into<String>,
        target_region: impl Into<String>,
        prior_owner_epoch: u64,
        program_module_hash: impl Into<String>,
        payload_schema: impl Into<String>,
        payload: Vec<u8>,
        payload_digest: impl Into<String>,
    ) -> Result<HandoffTransferState> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ImportEntity {
                handoff_id: handoff_id.into(),
                entity_ref: entity_ref.into(),
                source_region: source_region.into(),
                target_region: target_region.into(),
                prior_owner_epoch,
                program_module_hash: program_module_hash.into(),
                payload_schema: payload_schema.into(),
                payload,
                payload_digest: payload_digest.into(),
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped entity import response")?
    }

    /// Hand the write lease to `to` at `epoch` (design #2819, #3805).
    /// The actor journals `LeaseHandedOff` before changing its live lease;
    /// from that boundary this actor refuses world mutations unless it is the
    /// holder. Commands queued behind the handoff are refused with a
    /// retryable `not_holder` error and are never executed. The host adapter
    /// must authenticate the caller and have drained/replayed the
    /// destination before calling this.
    pub async fn handoff_write_lease(
        &self,
        to: impl Into<String>,
        epoch: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<u64> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::HandoffWriteLease {
                to: to.into(),
                epoch,
                expires_at_ms,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped lease handoff response")?
    }

    /// Run a spill v2 automatic failover (design #5787 §1, #7572).
    /// The actor plans from its journaled lease and the supplied
    /// grantor-side pool and journals the revoke/grant pair in plan
    /// order; the returned execution names the successor, its epoch,
    /// and the grant position it must replay through before writing.
    /// The host adapter must authenticate the caller and supply pool
    /// observations gathered from the roster's readers.
    pub async fn failover_write_lease(
        &self,
        pool: spill_pool::SpillPool,
        journal_tip: u64,
        max_catch_up_lag: u64,
        term_ms: u64,
    ) -> Result<spill_pool::FailoverExecution> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::FailoverWriteLease {
                pool,
                journal_tip,
                max_catch_up_lag,
                term_ms,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped lease failover response")?
    }

    /// Release a journal copy for a spilled holder only after the holder's
    /// verified replay cursor reaches the current grant boundary. The
    /// serialized authority actor rechecks holder, epoch, term, and boundary
    /// immediately before returning the bytes; callers must authenticate the
    /// holder and cursor and pass the residency gate before invoking this
    /// method, then send only returned bytes. A refused copy is never
    /// released for dispatch. The sanctioned production caller is
    /// [`super::spill_controller::SpillController`], which fixes that
    /// authenticate → residency → dispatch order with no bypass.
    pub async fn dispatch_spill_copy(
        &self,
        holder: impl Into<String>,
        epoch: u64,
        replayed_position: u64,
        journal_copy: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::DispatchSpillCopy {
                holder: holder.into(),
                epoch,
                replayed_position,
                journal_copy,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped spill dispatch response")?
    }

    /// Read a world snapshot.
    pub async fn snapshot(&self, token: JoinCapability) -> Result<WorldSnapshot> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Snapshot { token, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped snapshot response")?
    }

    /// Return the admin-only snapshot and presentation for a read-only host
    /// view. The caller must authenticate the world admin before invoking it.
    pub async fn host_view(&self) -> Result<(WorldSnapshot, Option<String>)> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::HostView { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped host-view response")?
    }

    /// What the holder of `token` may see of a live `event` (from
    /// [`WorldHandle::subscribe`]): `Ok(None)` if the event is not for this
    /// viewer, `Err` if the capability no longer joins. Judged by the
    /// viewer's scopes at this moment.
    pub async fn project_event(
        &self,
        token: JoinCapability,
        event: WorldEvent,
    ) -> Result<Option<WorldEvent>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ProjectEvent {
                token,
                event,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped projection response")?
    }

    /// Catch up from an event cursor, or learn that a new snapshot is needed.
    pub async fn events_after(&self, token: JoinCapability, sequence: u64) -> Result<EventPage> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::EventsAfter {
                token,
                sequence,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped event response")?
    }

    /// Render the current world for an authorized participant. A chat-only
    /// world returns `None`.
    pub async fn view(&self, token: JoinCapability) -> Result<Option<String>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::View { token, reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped view response")?
    }

    /// Render for one viewer at a viewport. Programs that cannot reflow fall
    /// back to their fixed view, and rendering never advances the sequence.
    pub async fn view_at(
        &self,
        token: JoinCapability,
        width: u16,
        height: u16,
    ) -> Result<Option<String>> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::ViewAt {
                token,
                width,
                height,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped layout response")?
    }

    /// Atomically replace the active program and sequence a host-authored
    /// module-installed event. The new program is used after this command.
    pub async fn install_program(
        &self,
        module_hash: String,
        seed: u64,
        program: Box<dyn WorldProgram>,
    ) -> Result<WorldEvent> {
        self.install_program_with_provenance(module_hash, seed, None, program)
            .await
    }

    /// As [`Self::install_program`], preserving server-observed origin metadata.
    pub async fn install_program_with_provenance(
        &self,
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        program: Box<dyn WorldProgram>,
    ) -> Result<WorldEvent> {
        self.install_program_by(module_hash, seed, provenance, None, program)
            .await
    }

    /// As [`Self::install_program_with_provenance`], recording the builder
    /// participant that ordered the install for the audit journal.
    pub async fn install_program_by(
        &self,
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        installed_by: Option<u64>,
        program: Box<dyn WorldProgram>,
    ) -> Result<WorldEvent> {
        self.install_program_with_trust(module_hash, seed, provenance, installed_by, None, program)
            .await
    }

    /// Install a program on the authority of a joined builder's live
    /// capability (design #835 phase 2). The world actor authorizes the
    /// capability itself at the mutation boundary and derives the audit
    /// actor (`BuilderInstalled { by }`) from it. The caller's module
    /// trust decision is carried through to the durable install event,
    /// exactly as on the admin path (#1069): a verified publisher must
    /// stay attributable in the journal.
    pub async fn install_program_authorized(
        &self,
        token: JoinCapability,
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        trust: Option<TrustDecision>,
        program: Box<dyn WorldProgram>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::InstallProgramAuthorized {
                token,
                module_hash,
                seed,
                provenance,
                trust,
                program,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped install response")?
    }

    /// As [`Self::install_program_by`], also recording the module trust
    /// decision that admitted the module for the audit journal (#1069).
    pub async fn install_program_with_trust(
        &self,
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        installed_by: Option<u64>,
        trust: Option<crate::world::TrustDecision>,
        program: Box<dyn WorldProgram>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::InstallProgram {
                module_hash,
                seed,
                provenance,
                installed_by,
                trust,
                program,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped install response")?
    }

    /// Stop the actor after processing all commands queued before shutdown.
    pub async fn shutdown(&self) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Shutdown { reply })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped shutdown response")
    }
}
