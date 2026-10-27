//! Transport-independent, authoritative in-memory world core.
//!
//! A transport should place one `WorldActor` behind its world ID and pass
//! messages to it serially. This module does not open sockets or persist world
//! state. Capability tokens are returned only at issuance and retained only as
//! SHA-256 digests for revocation and membership checks.
//!
//! ## What a grant carries, and what stays local
//!
//! A participant capability is a bearer secret. Its digest, scopes, expiry,
//! parent and subject are the grant, and the journal records them. Nothing
//! about a participant grant is signed here; the signed records are the
//! directory's artifacts (friendships and their removals). A grant carries no
//! use count. A delegation narrows scopes and may set an expiry, nothing else.
//! The host alone enforces a use limit, and only on a capability it issued (an
//! invite or an account grant), by counting actions in its own journal. Those
//! counts are local to the hosting node and need not converge across partitions.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use crate::world_caps::{CapError, CapsReport, Decision, Request, Subscription};
use anyhow::{Context, Result, bail, ensure};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// Re-exported so `crate::world::Isolation` keeps working: the type now lives
/// in [`crate::groups::worlds`] so the crate builds without this feature.
pub use crate::groups::worlds::Isolation;

/// Re-exported so `crate::world::unix_ms` keeps working: the function now
/// lives in [`crate::time`] so the crate builds without this feature.
pub(crate) use crate::time::unix_ms;

/// A chat participant may send at most this many lines during the rolling window.

pub use crate::world_model::*;

/// one actor task, which defines the authoritative event order.
#[derive(Clone, Debug)]
pub struct WorldHandle {
    commands: mpsc::Sender<WorldCommand>,
    events: broadcast::Sender<WorldEvent>,
    revocations: broadcast::Sender<u64>,
    records: watch::Receiver<(u64, Arc<Records>)>,
    view_changed: watch::Receiver<u64>,
}

enum WorldCommand {
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
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Trim `journal` behind a verified snapshot of `program`. `Ok(false)` means
/// the program cannot snapshot.
fn compact_journal(
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
struct WorldActor {
    core: WorldCore,
    /// Sessions and pending email codes. In memory only: a restart ends them.
    auth: crate::directory_auth::DirectoryAuth,
    program: Box<dyn WorldProgram>,
    journal: Option<Box<dyn WorldJournal>>,
    storage_failed: bool,
    program_healthy: bool,
    compaction_off: bool,
    event_sender: broadcast::Sender<WorldEvent>,
    revocation_sender: broadcast::Sender<u64>,
    records_sender: watch::Sender<(u64, Arc<Records>)>,
    view_changed: watch::Sender<u64>,
    /// Live sessions per participant: presence is refcounted.
    presence: HashMap<u64, usize>,
    /// Subscriptions the program asked for (granted or refused-and-remembered).
    active_subs: BTreeSet<Subscription>,
    /// Request ids already executed, so `wants` states intent, not polling.
    handled_requests: BTreeSet<String>,
    /// Subscription specs already reported as refused.
    denied_subs: BTreeSet<String>,
    /// Armed tick intervals to their next fire time.
    ticks: BTreeMap<u64, std::time::Instant>,
    /// Restored schedules already overdue when the world opened. Their first
    /// fire is marked `late`; a cancel or fire removes the id.
    overdue_at_open: BTreeSet<String>,
    /// The participant whose input is being processed. Requests the program
    /// makes in response are checked against that participant's scopes.
    acting: Option<u64>,
}

impl WorldActor {
    async fn run(mut self, mut receiver: mpsc::Receiver<WorldCommand>) {
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
            self.handle(command).await;
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
    fn next_deadline(&self) -> Option<std::time::Instant> {
        let deliverable = self.program_healthy && !self.storage_failed;
        self.ticks
            .values()
            .min()
            .copied()
            .into_iter()
            .chain(next_schedule_wake(
                &self.core.schedules,
                unix_ms(),
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
        if let Some(journal) = self.journal.as_mut()
            && let Err(error) = journal.append(&entry)
        {
            self.storage_failed = true;
            tracing::error!("world journal append failed: {error:#}");
            return Err(error.context("world storage failed; the world is read-only"));
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
    async fn after_change(&mut self) {
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
    fn arm(&mut self, sub: &Subscription) {
        if let Subscription::Tick(ms) = sub
            && !self.presence.is_empty()
            && !self.ticks.contains_key(ms)
        {
            self.ticks.insert(
                *ms,
                std::time::Instant::now() + std::time::Duration::from_millis(*ms),
            );
        }
    }

    /// Deliver one tick event per due interval and re-arm.
    async fn fire_ticks(&mut self) {
        let now = std::time::Instant::now();
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
                "now": unix_ms(),
            });
            self.deliver(event.to_string()).await;
        }
    }

    /// Fire the schedules that are due, earliest first, at most
    /// [`crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP`] per wakeup. Each
    /// fire is delivered as a tick is, and its schedule is consumed only after
    /// that delivery is journaled. A refused fire, or one that could not be
    /// journaled, leaves its schedule pending, so the next run delivers it.
    async fn fire_schedules(&mut self) {
        let now = unix_ms();
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
            _ => Err(CapError::Unknown),
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
                    return Err(CapError::BadArgs);
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
            return Err(CapError::BadArgs);
        };
        match self.core.system_chat(text) {
            Ok(event) => {
                if self
                    .persist(JournalEntry::Committed {
                        event: event.clone(),
                    })
                    .is_err()
                {
                    return Err(CapError::Failed);
                }
                let _ = self.event_sender.send(event.clone());
                Ok(serde_json::json!({"sequence": event.sequence}))
            }
            Err(_) => Err(CapError::BadArgs),
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
            .ok_or(CapError::BadArgs)?;
        let at_ms = args
            .get("at")
            .and_then(serde_json::Value::as_u64)
            .ok_or(CapError::BadArgs)?;
        if let Err(error) = self.core.check_schedule(id, at_ms, unix_ms()) {
            tracing::debug!("world {}: schedule refused: {error:#}", self.core.world_id);
            return Err(CapError::BadArgs);
        }
        if self
            .persist(JournalEntry::ScheduleSet {
                id: id.to_owned(),
                at_ms,
            })
            .is_err()
        {
            return Err(CapError::Failed);
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
            .ok_or(CapError::BadArgs)?;
        if crate::world_caps::validate_schedule_id(id).is_err() {
            return Err(CapError::BadArgs);
        }
        if !self.core.schedules.contains_key(id) {
            return Ok(serde_json::json!({"cancelled": false}));
        }
        if self
            .persist(JournalEntry::ScheduleCancelled { id: id.to_owned() })
            .is_err()
        {
            return Err(CapError::Failed);
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
                    Subscription::Tick(ms) if self.core.ledger.is_granted(sub.cap()) => Some(*ms),
                    _ => None,
                })
                .collect();
            for ms in intervals {
                self.ticks.entry(ms).or_insert_with(|| {
                    std::time::Instant::now() + std::time::Duration::from_millis(ms)
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
        })
    }

    async fn handle(&mut self, command: WorldCommand) {
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
                reply,
            } => {
                // Revocation takes effect in memory even if storage failed:
                // denying access is always the safe side.
                let revoked = self.core.revoke_tree(participant_id);
                let was_active = !revoked.is_empty();
                if was_active {
                    let _ = self.persist(JournalEntry::Revoked { participant_id });
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
                                    self.program_healthy = false;
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
                            self.program_healthy = false;
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
                                        self.program_healthy = false;
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
                                self.program_healthy = false;
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
                        module_hash,
                        seed,
                        provenance: provenance.clone(),
                    },
                };
                let result = self
                    .persist(JournalEntry::Committed {
                        event: candidate.clone(),
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
        match compact_journal(&mut self.core, self.program.as_mut(), journal.as_mut()) {
            Ok(true) => {}
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
        let now = unix_ms();
        let overdue_at_open: BTreeSet<String> = core
            .schedules
            .iter()
            .filter(|(_, at_ms)| **at_ms <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let actor = WorldActor {
            core,
            program,
            journal,
            storage_failed: false,
            program_healthy,
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
            auth: crate::directory_auth::DirectoryAuth::default(),
        };
        tokio::spawn(actor.run(receiver));
        Self {
            commands,
            events,
            revocations,
            records,
            view_changed,
        }
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
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Revoke {
                participant_id,
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
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Input {
                token,
                input,
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
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::InstallProgram {
                module_hash,
                seed,
                provenance,
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

/// Incremental event page for an authorized participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventPage {
    /// Latest committed sequence at response time.
    pub current_sequence: u64,
    /// Earliest retained sequence.
    pub oldest_retained_sequence: u64,
    /// The event cursor fell behind retention and a full snapshot is required.
    pub needs_snapshot: bool,
    /// Events newer than the requested cursor, if it is still retained.
    pub events: Vec<WorldEvent>,
}

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    bool::from(left.ct_eq(right))
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn publish_records(sender: &watch::Sender<(u64, Arc<Records>)>, sequence: u64, records: Records) {
    sender.send_if_modified(|current| {
        if *current.1 == records {
            false
        } else {
            *current = (sequence, Arc::new(records));
            true
        }
    });
}

fn hex_decode_32(encoded: &str) -> Option<[u8; 32]> {
    let bytes = encoded.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        let nibble = |b: u8| char::from(b).to_digit(16);
        *slot = u8::try_from(nibble(pair[0])? << 4 | nibble(pair[1])?).ok()?;
    }
    Some(out)
}

fn sanitize_display_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !ch.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn sanitize_chat(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .collect::<String>()
        .trim()
        .to_owned()
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "world/tests.rs"]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "world/account_tests.rs"]
mod account_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "world/isolation_tests.rs"]
mod isolation_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "world/schedule_tests.rs"]
mod schedule_tests;
