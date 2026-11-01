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
}

use super::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::world_caps::CapsReport;

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

    /// Spawn a world actor that journals every mutation before acknowledging
    /// it. `program` must already reflect the journal (see
    /// [`WorldCore::restore`]); the actor only appends.
    pub fn spawn_durable(
        core: WorldCore,
        mut program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
    ) -> Self {
        let (commands, receiver) = mpsc::channel(256);
        let (events, _) = broadcast::channel(256);
        let (revocations, _) = broadcast::channel(256);
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
            .filter(|(_, at_ms)| **at_ms <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let metrics = WorldMetrics::default();
        let actor = WorldActor {
            core,
            program,
            journal,
            metrics: metrics.clone(),
            storage_failed: false,
            program_healthy,
            view_failures: 0,
            compaction_off: false,
            event_sender: events.clone(),
            revocation_sender: revocations.clone(),
            records_sender,
            view_changed: view_changed_sender,
            presence: HashMap::new(),
            active_subs: BTreeSet::new(),
            handled_requests: BTreeSet::new(),
            denied_subs: BTreeSet::new(),
            ticks: BTreeMap::new(),
            overdue_at_open,
            acting: None,
            acting_session: None,
            auth: crate::directory_auth::DirectoryAuth::default(),
        };
        tokio::spawn(actor.run(receiver));
        Self {
            commands,
            events,
            revocations,
            records,
            view_changed,
            metrics,
        }
    }

    /// The world's shared host metrics registry (F11.7): the session
    /// layer records frame bytes into it, the actor records the rest.
    #[must_use]
    pub fn shared_metrics(&self) -> WorldMetrics {
        self.metrics.clone()
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
        self.input_with_session(token, input, None).await
    }

    /// Submit bounded opaque game input from a live session connection.
    pub(crate) async fn input_from_session(
        &self,
        token: JoinCapability,
        input: Vec<u8>,
        session_id: SessionId,
    ) -> Result<WorldEvent> {
        self.input_with_session(token, input, Some(session_id))
            .await
    }

    async fn input_with_session(
        &self,
        token: JoinCapability,
        input: Vec<u8>,
        session_id: Option<SessionId>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Input {
                token,
                input,
                session_id,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped input response")?
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
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::InstallProgram {
                module_hash,
                seed,
                provenance,
                installed_by,
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
