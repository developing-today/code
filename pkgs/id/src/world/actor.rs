use super::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Result, ensure};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::Instrument;

use crate::world_caps::{CapError, CapsReport, Decision, Request, Subscription};

impl WorldCommand {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Issue { .. } => "Issue",
            Self::Attenuate { .. } => "Attenuate",
            Self::DelegateToFriend { .. } => "DelegateToFriend",
            Self::Directory { .. } => "Directory",
            Self::ApplyDirectory { .. } => "ApplyDirectory",
            Self::Revoke { .. } => "Revoke",
            Self::ParticipantId { .. } => "ParticipantId",
            Self::Chat { .. } => "Chat",
            Self::Input { .. } => "Input",
            Self::Snapshot { .. } => "Snapshot",
            Self::EventsAfter { .. } => "EventsAfter",
            Self::ProjectEvent { .. } => "ProjectEvent",
            Self::View { .. } => "View",
            Self::ViewAt { .. } => "ViewAt",
            Self::InstallProgram { .. } => "InstallProgram",
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
    ParticipantId {
        token: JoinCapability,
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
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    Snapshot {
        token: JoinCapability,
        reply: oneshot::Sender<Result<WorldSnapshot>>,
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
    pub(crate) program_healthy: bool,
    /// Times a `view` or `layout` call trapped. Counted for `id world stats`
    /// (F11.17): a trapping view degrades to an error screen, not a poisoned
    /// world, so this counter is the only place the failure is recorded.
    pub(crate) view_failures: u64,
    pub(crate) compaction_off: bool,
    pub(crate) event_sender: broadcast::Sender<WorldEvent>,
    pub(crate) revocation_sender: broadcast::Sender<u64>,
    pub(crate) records_sender: watch::Sender<(u64, Arc<Records>)>,
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
    /// The participant whose input is being processed. Requests the program
    /// makes in response are checked against that participant's scopes.
    pub(crate) acting: Option<u64>,
}

impl WorldActor {
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
    pub(crate) fn next_deadline(&self) -> Option<std::time::Instant> {
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
            !self.storage_failed,
            "world storage failed; the world is read-only"
        );
        if let Some(journal) = self.journal.as_mut() {
            // The append includes its fsync, so this is the full durable
            // cost of the entry (F11.7 journal append + fsync latency).
            let started = std::time::Instant::now();
            if let Err(error) = journal.append(&entry) {
                self.storage_failed = true;
                tracing::error!("world journal append failed: {error:#}");
                return Err(error.context("world storage failed; the world is read-only"));
            }
            self.metrics.record_journal_append(started.elapsed());
        }
        Ok(())
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
    async fn deliver(&mut self, text: String) -> bool {
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
        let journaled = self.persist(JournalEntry::System { event }).is_ok();
        if journaled {
            self.core.since_checkpoint = self.core.since_checkpoint.saturating_add(1);
        }
        let _ = self.view_changed.send(self.core.sequence);
        journaled
    }

    /// Recompute the program's `wants` and serve the delta: new subscriptions
    /// (armed or refused), new requests (executed, result delivered). Passes
    /// repeat while the program reacts to delivered results, so a chain of
    /// asks settles inside one change.
    pub(crate) async fn after_change(&mut self) {
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
                    self.deliver(crate::world_caps::subscription_refused_event(&spec, error))
                        .await;
                }
                continue;
            }
            match self.core.ledger.decide(sub.cap()) {
                Decision::Allow => self.arm(sub),
                Decision::AllowAndGrant => {
                    let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
                    let _ = self.persist(JournalEntry::CapsGranted { granted });
                    self.arm(sub);
                }
                Decision::Deny(error) => {
                    let spec = sub.spec();
                    if self.denied_subs.insert(spec.clone()) {
                        self.deliver(crate::world_caps::subscription_refused_event(&spec, error))
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
    pub(crate) async fn fire_schedules(&mut self) {
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
        match self.core.ledger.decide_for(&request.cap, actor_permits) {
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
            "chat.say" => self.execute_chat(&request.args),
            "chat.history" => Ok(serde_json::json!({
                "lines": chat_history(
                    &self.core.events,
                    request
                        .args
                        .get("count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(16)
                        .min(crate::world_caps::MAX_CHAT_HISTORY_LINES as u64) as usize,
                ),
            })),
            "records.read" => self.execute_records_read(&request.args),
            "schedule.at" => self.execute_schedule_at(&request.args),
            "schedule.cancel" => self.execute_schedule_cancel(&request.args),
            _ => Err(CapError::unknown()),
        }
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
    fn execute_records_read(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, CapError> {
        let records = self.records_sender.borrow().1.clone();
        let prefix = args.get("prefix").and_then(serde_json::Value::as_str);
        let after = args.get("after").and_then(serde_json::Value::as_str);
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
            let size = key.len() + serde_json::to_vec(value).map_or(0, |v| v.len()) + 8;
            if used + size > crate::world_session::MAX_RECORDS_PAGE_BYTES {
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
        match self.core.system_chat(text) {
            Ok(event) => {
                if self
                    .persist(JournalEntry::Committed {
                        event: event.clone(),
                    })
                    .is_err()
                {
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
            catalog: crate::world_caps::CATALOG
                .iter()
                .map(|name| (*name).to_owned())
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
                let result = self.check_storage().and_then(|()| {
                    let issued = self.core.issue_capability(&name, scopes, bounds)?;
                    self.record_issued(issued)
                });
                let _ = reply.send(result);
            }
            WorldCommand::Attenuate {
                token,
                name,
                scopes,
                bounds,
                reply,
            } => {
                let result = self.check_storage().and_then(|()| {
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
                let result = self.check_storage().and_then(|()| {
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
                let _ = reply.send(self.core.directory_mut().apply(&entry));
            }
            WorldCommand::Revoke {
                participant_id,
                by,
                reply,
            } => {
                // Revocation takes effect in memory even if storage failed:
                // denying access is always the safe side.
                let revoked = self.core.revoke_tree(participant_id);
                let was_active = !revoked.is_empty();
                if was_active {
                    let _ = self.persist(JournalEntry::Revoked { participant_id, by });
                    for id in revoked {
                        let _ = self.revocation_sender.send(id);
                    }
                }
                let _ = reply.send(was_active);
            }
            WorldCommand::ParticipantId { token, reply } => {
                let _ = reply.send(self.core.authorize(&token, WorldScopes::JOIN));
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
                    self.core.chat(&token, &text).and_then(|event| {
                        self.persist(JournalEntry::Committed {
                            event: event.clone(),
                        })?;
                        Ok(event)
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
                reply,
            } => {
                let result = match self.core.prepare_input(&token, &input) {
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
            WorldCommand::Snapshot { token, reply } => {
                let _ = reply.send(self.core.snapshot(&token));
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
                                                    if let Ok(Some(crate::world_presentation::WorldSurface::Screen(surface))) = crate::world_presentation::parse_surface(&fallback_text) {
                                                        let cropped = surface.fit(usize::from(width), usize::from(height));
                                                        Ok(Some(serde_json::to_string(&cropped).unwrap_or(fallback_text)))
                                                    } else {
                                                        Ok(Some(fallback_text))
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
                program: mut replacement,
                reply,
            } => {
                // A program whose initial records are invalid is refused
                // before anything is journaled.
                let new_records = match program_records(replacement.as_mut()) {
                    Ok(records) => records.unwrap_or_default(),
                    Err(error) => {
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
                    },
                };
                let result = self
                    .persist(JournalEntry::Committed {
                        event: candidate.clone(),
                    })
                    .and_then(|()| {
                        // The builder authority leaves a journal-only audit
                        // record, alongside the install event.
                        if let Some(by) = installed_by {
                            return self.persist(JournalEntry::BuilderInstalled {
                                by,
                                module_hash: module_hash.clone(),
                                seed,
                            });
                        }
                        Ok(())
                    })
                    .and_then(|()| self.core.commit_prepared(candidate));
                let result = match result {
                    Ok(event) => {
                        if let WorldEventKind::ProgramInstalled { module_hash, .. } = &event.kind {
                            self.core.active_module_hash = Some(module_hash.clone());
                        }
                        self.core.active_module_provenance = provenance;
                        self.core.active_seed = seed;
                        self.program = replacement;
                        self.program_healthy = true;
                        self.compaction_off = false;
                        // A new program starts with a clean slate.
                        self.handled_requests.clear();
                        self.denied_subs.clear();
                        self.active_subs.clear();
                        self.ticks.clear();
                        self.republished(new_records, event.sequence);
                        let _ = self.event_sender.send(event.clone());
                        Ok(event)
                    }
                    Err(error) => Err(error),
                };
                let _ = reply.send(result);
            }
            WorldCommand::UpdateCaps {
                grant,
                revoke,
                reply,
            } => {
                let result = (|| -> Result<CapsReport> {
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
                let _ = reply.send(result);
            }
            WorldCommand::SetCapsCeiling { ceiling, reply } => {
                let result = self.set_caps_ceiling(ceiling);
                let _ = reply.send(result);
            }
            WorldCommand::SetExecutionProfile { profile, reply } => {
                let result = self
                    .persist(JournalEntry::ExecutionProfile { profile })
                    .map(|()| {
                        self.core.execution_profile = profile;
                        ExecutionProfileUpdate {
                            requested: profile,
                            applies_on_next_program_install_or_process_restart: true,
                        }
                    });
                let _ = reply.send(result);
            }
            WorldCommand::ExecutionProfileInfo { reply } => {
                let _ = reply.send(self.core.execution_profile);
            }
            WorldCommand::ManifestState { reply } => {
                let _ = reply.send((self.core.active_seed, self.core.schedules.clone()));
            }
            WorldCommand::AdminScheduleAt { id, at_ms, reply } => {
                let result = (|| -> Result<BTreeMap<String, u64>> {
                    if self.core.schedules.get(&id) == Some(&at_ms) {
                        return Ok(self.core.schedules.clone());
                    }
                    // Validate the replacement before anything is persisted:
                    // a refusal (say, past the horizon) must leave the
                    // existing schedule standing, in the journal as in
                    // memory (#807).
                    let replacing = self.core.schedules.contains_key(&id);
                    self.core.check_schedule_replacing(
                        &id,
                        at_ms,
                        self.core.clock().now_ms(),
                        replacing,
                    )?;
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
                let _ = reply.send(result);
            }
            WorldCommand::AdminCancelSchedule { id, reply } => {
                let result = (|| -> Result<BTreeMap<String, u64>> {
                    crate::world_caps::validate_schedule_id(&id)?;
                    if self.core.schedules.contains_key(&id) {
                        self.persist(JournalEntry::ScheduleCancelled { id: id.clone() })?;
                        self.core.schedules.remove(&id);
                        self.overdue_at_open.remove(&id);
                    }
                    Ok(self.core.schedules.clone())
                })();
                let _ = reply.send(result);
            }
            WorldCommand::SetIsolation {
                authority,
                isolation,
                reply,
            } => {
                let result = self.set_isolation(&authority, isolation);
                let _ = reply.send(result);
            }
            WorldCommand::IsolationInfo { reply } => {
                let _ = reply.send(self.core.isolation());
            }
            WorldCommand::SetCrossWorldWrite {
                authority,
                enabled,
                reply,
            } => {
                let result = self.set_cross_world_write(&authority, enabled);
                let _ = reply.send(result);
            }
            WorldCommand::ForceCrossWorldWrite { authority, reply } => {
                let result = self.force_cross_world_write(&authority);
                let _ = reply.send(result);
            }
            WorldCommand::SetFileScope {
                authority,
                scope,
                write_override,
                reply,
            } => {
                let result = self.set_file_scope(&authority, scope, write_override);
                let _ = reply.send(result);
            }
            WorldCommand::PolicyInfo { reply } => {
                let _ = reply.send(self.core.policy());
            }
            WorldCommand::Metrics { reply } => {
                let sessions = self.presence.values().sum::<usize>() as u64;
                let _ = reply.send(self.metrics.snapshot(sessions));
            }
            WorldCommand::CapsInfo { reply } => {
                let _ = reply.send(self.caps_report());
            }
            WorldCommand::Shutdown { .. } => unreachable!("handled by run"),
        }
    }

    /// Trim the journal behind a verified snapshot, at most once per run of
    /// commits and never while the program cannot snapshot.
    async fn compact_if_due(&mut self) {
        if self.compaction_off
            || self.storage_failed
            || !self.program_healthy
            || !self.core.checkpoint_due()
        {
            return;
        }
        let Some(journal) = self.journal.as_mut() else {
            return;
        };
        let started = std::time::Instant::now();
        match compact_journal(&mut self.core, self.program.as_mut(), journal.as_mut()) {
            Ok(true) => self.metrics.record_checkpoint(started.elapsed()),
            Ok(false) => self.compaction_off = true,
            Err(error) => {
                // The journal is still complete; just do not retry until
                // something changes.
                tracing::warn!("world checkpoint skipped: {error:#}");
                self.compaction_off = true;
            }
        }
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
            program_healthy,
            view_failures: 0,
            compaction_off: false,
            event_sender: events,
            revocation_sender: revocations,
            records_sender,
            view_changed,
            presence: HashMap::new(),
            active_subs: BTreeSet::new(),
            handled_requests: BTreeSet::new(),
            denied_subs: BTreeSet::new(),
            ticks: BTreeMap::new(),
            overdue_at_open,
            acting: None,
            auth: crate::directory_auth::DirectoryAuth::default(),
            metrics: WorldMetrics::default(),
        }
    }
}
