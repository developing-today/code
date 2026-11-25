use super::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::Instrument;

use crate::world_caps::{CapsReport, Decision, Subscription};

mod command;
mod execution;

pub(crate) use command::WorldCommand;

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
    /// What the action acted on: the world for administrative commands,
    /// the capability for provider use, the named world (or `hub`) for
    /// session admin frames. Added in schema version 2 (#1063); absent
    /// (empty) in version-1 records.
    #[serde(default)]
    pub(crate) target: String,
    /// Correlation id tying this record to the frame or command that
    /// produced it; unique and monotonic per process. Added in schema
    /// version 2 (#1063); absent (empty) in version-1 records.
    #[serde(default)]
    pub(crate) correlation_id: String,
}

/// Monotonic sequence behind audit correlation ids, shared by the
/// actor and the session layer so every audit line a process emits
/// carries a unique correlation id (#1063).
static AUDIT_CORRELATION_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_audit_correlation_id() -> String {
    let seq = AUDIT_CORRELATION_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("audit:{seq}")
}

/// Emit one audit record on the reserved target — the single emission
/// point for the whole audit stream, so the actor and the session
/// layer can never drift in shape.
pub(crate) fn emit_audit_record(record: &AuditRecord) {
    if let Ok(json) = serde_json::to_string(record) {
        tracing::info!(target: AUDIT_TRACE_TARGET, audit_record = %json);
    } else {
        tracing::error!(target: AUDIT_TRACE_TARGET, action = %record.action, "could not serialize audit record");
    }
}

fn authority_identity(authority: &Authority) -> String {
    match authority {
        Authority::Admin => "admin".to_owned(),
        Authority::Signed(public_key) => format!("signed:{public_key}"),
    }
}

/// The most unsigned inputs one group-commit batch may stage (#8239):
/// bounding the drain bounds how long the actor loop spends staging
/// before it persists, and how much one sync must cover.
const MAX_INPUT_BATCH: usize = 64;

/// Whether `error` is a fuel-rate admission refusal (#1067): the
/// program never ran, so the actor stays healthy and the input may
/// be retried once the account's window rolls. Every other program
/// error is a guest failure and poisons the program.
///
/// Checked before any `.context(...)` is applied: the bare program
/// error carries the refusal at its outermost layer.
fn is_fuel_rate_refusal(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<id_caps::account_quota::FuelRateRefusal>()
        .is_some()
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
    /// Cross-world schedule delivery sink (#1355 part 2), installed by
    /// the composition layer. `None` leaves cross-world schedules
    /// pending: no sink, no delivery.
    pub(crate) schedule_delivery: Option<std::sync::Arc<dyn ScheduleDelivery>>,
    /// (The F12.14 portal transport sink lives on the handle, not
    /// the actor: send orchestration runs outside this command loop
    /// so a send never waits on a peer actor while holding it —
    /// see `WorldHandle::send_portal_message`.)
    /// Per-ordered-pair portal rate windows (code#1068 wiring
    /// slice), keyed by target peer world: this actor is always the
    /// source half of the pair. Actor-lifetime only, like the
    /// schedule receipts' in-memory mirrors — the kernel window is a
    /// send-side admission control, not durable state.
    pub(crate) portal_rate: BTreeMap<String, id_caps::portal::PortalRateWindow>,
    /// Host metrics registry (F11.7), shared with the handle and the
    /// session layer; see [`WorldMetrics`].
    pub(crate) metrics: WorldMetrics,
    /// Retained audit log (F12.6), shared with the handle and the
    /// session layer; see [`WorldAuditLog`].
    pub(crate) audit: WorldAuditLog,
    /// Client-provider broker (F13.7 step 3), shared with the handle
    /// and the session layer; see [`ProviderBroker`].
    pub(crate) provider_broker: ProviderBroker,
    pub(crate) storage_failed: bool,
    /// F12.13 replica mode: the world serves spectators from a verified
    /// journal and refuses every local mutation and side effect. Set only
    /// by the replica open path; a writer never runs this way.
    pub(crate) read_only: bool,
    /// F12.13 phase 5 policy-B gate on NEW spectator admissions, set
    /// only for a read-only replica served with an admission gate
    /// (see [`super::ReplicaAdmissionGate`]). `None` — a writer, or a
    /// replica served without a gate — keeps the phase-4 posture:
    /// token admission is journal authorization alone.
    pub(crate) admission_gate: Option<ReplicaAdmissionGate>,
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
    /// Capability results whose delivery was refused at the account's
    /// fuel rate (#1067): the side effect already ran, so the result
    /// event waits here — never re-executed — until the window rolls
    /// and `retry_pending_results` delivers it. Actor lifetime only,
    /// like `handled_requests`: a restart falls back to the existing
    /// restarted-refusal path for the same id.
    pub(crate) pending_results: std::collections::VecDeque<String>,
    /// Earliest actor-clock instant a fuel-refused delivery may be
    /// retried (#1067): one full window past the refusal, which is
    /// guaranteed to land in a fresh budget window — the refused
    /// window started no later than the refusal. `None` while no
    /// refusal is outstanding.
    pub(crate) fuel_retry_not_before: Option<std::time::Instant>,
    /// Forms registered by `form.present`; actor-lifetime only, so a restart
    /// invalidates outstanding UI submissions instead of replaying stale input.
    pub(crate) pending_forms: BTreeMap<String, PendingForm>,
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
    /// Input affordances of the session whose input is currently being
    /// processed. Unlike `device_manifest`, this describes the presenter,
    /// not the hosting device, and is only used for this actor turn.
    pub(crate) acting_device_input: Option<serde_json::Value>,
    /// The host's current location consent (#2341 slice 2): the seam a
    /// host fills from its permission prompt / OS state. The default
    /// host records no consent, so sensor caps refuse
    /// `consent_required` until a host says otherwise.
    pub(crate) location_consent: crate::world_caps::LocationConsent,
    /// The host location source's latest fix (#2341 slice 2), as the
    /// adapter boundary last reported it. `None` — the default, and
    /// the state of any host with no location source — answers
    /// `failed` / `no_fix`; a fix is never fabricated.
    pub(crate) location_fix: Option<crate::world_caps::LocationReading>,
    /// When `location_fix` was received, on the world clock. The
    /// reading's `fix_age_ms` is its age at receipt; serving adds
    /// the time elapsed since receipt, so a cached fix never claims
    /// to be younger than it is (PR #8643 Codex round-2 P2).
    pub(crate) location_fix_received_at: Option<std::time::Instant>,
    /// Armed `loc.watch` intervals to their next fire time.
    pub(crate) loc_watches: BTreeMap<u64, std::time::Instant>,
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

/// One actor-local form presentation and its host-generated generation token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingForm {
    pub(crate) surface: id_core::world_presentation::FormSurface,
    pub(crate) presentation_token: String,
}

impl WorldActor {
    fn audit_record(
        &self,
        actor: impl Into<String>,
        action: &str,
        parameters: serde_json::Value,
        outcome: &str,
    ) {
        let target = self.core.world_id().to_owned();
        let record = AuditRecord {
            schema_version: 2,
            timestamp_ms: self.core.clock().now_ms(),
            world_id: self.core.world_id().to_owned(),
            actor: actor.into(),
            action: action.to_owned(),
            parameters,
            outcome: outcome.to_owned(),
            target,
            correlation_id: next_audit_correlation_id(),
        };
        self.audit.record(record.clone());
        emit_audit_record(&record);
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
        let record = AuditRecord {
            schema_version: 2,
            timestamp_ms: self.core.clock().now_ms(),
            world_id: self.core.world_id().to_owned(),
            actor,
            action: "provider_use".to_owned(),
            parameters: serde_json::json!({"capability": capability}),
            outcome: outcome.to_owned(),
            target: capability.to_owned(),
            correlation_id: next_audit_correlation_id(),
        };
        self.audit.record(record.clone());
        emit_audit_record(&record);
    }

    pub(crate) async fn run(mut self, mut receiver: mpsc::Receiver<WorldCommand>, restored: bool) {
        if restored {
            self.invalidate_restored_form_presentations().await;
        }
        self.after_change().await;
        // A command the batch drain stopped at (the first non-Input) is
        // held here and handled next iteration, in order (#8239).
        let mut pending: Option<WorldCommand> = None;
        loop {
            // Ticks and schedules sleep until their earliest deadline; with
            // neither armed this branch never wakes.
            let deadline = self.next_deadline();
            let command = if let Some(command) = pending.take() {
                command
            } else {
                tokio::select! {
                command = receiver.recv() => match command {
                    Some(command) => command,
                    None => break,
                },
                _ = self.provider_broker.wait_changed() => {
                    self.drain_provider_completions().await;
                    continue;
                },
                () = Self::sleep_until(deadline) => {
                    // Give already-executed capability results first claim
                    // on the fresh fuel window, before recurring timers.
                    self.after_change().await;
                    self.fire_ticks().await;
                    self.fire_loc_watches().await;
                    self.fire_schedules().await;
                    self.after_change().await;
                    continue;
                }
                }
            };
            if matches!(command, WorldCommand::Shutdown { .. }) {
                if let WorldCommand::Shutdown { reply } = command {
                    let _ = reply.send(());
                }
                break;
            }
            self.metrics.record_queue_depth(receiver.len() as u64);
            // Group commit (#8239): an unsigned Input at the head of the
            // queue drains the unsigned Inputs already behind it into one
            // batch, persisted with a single journal sync. The drain stops
            // at the first other command, held in `pending` for the next
            // iteration - command order is never changed.
            let mut command = Some(command);
            let mut input_batch: Vec<WorldCommand> = Vec::new();
            if matches!(
                command.as_ref(),
                Some(WorldCommand::Input {
                    signed_move: None,
                    device_input: None,
                    ..
                })
            ) {
                if let Some(held) = command.take() {
                    input_batch.push(held);
                }
                while input_batch.len() < MAX_INPUT_BATCH {
                    match receiver.try_recv() {
                        Ok(next)
                            if matches!(
                                next,
                                WorldCommand::Input {
                                    signed_move: None,
                                    device_input: None,
                                    ..
                                }
                            ) =>
                        {
                            input_batch.push(next);
                        }
                        Ok(next) => {
                            pending = Some(next);
                            break;
                        }
                        Err(_) => break,
                    }
                }
            }
            let started = std::time::Instant::now();
            if input_batch.len() > 1 {
                async {
                    self.handle_input_batch(input_batch).await;
                }
                .instrument(tracing::info_span!("command", name = "InputBatch"))
                .await;
            } else {
                // An empty batch means the drain above never took the
                // held command; a non-empty batch pops its element. One
                // of the two always yields the command.
                let Some(command) = input_batch.pop().or_else(|| command.take()) else {
                    continue;
                };
                let name = command.name();
                async {
                    self.handle(command).await;
                }
                .instrument(tracing::info_span!("command", name))
                .await;
            }
            self.metrics.record_command(started.elapsed());
            self.after_change().await;
            self.drain_provider_completions().await;
            self.acting = None;
            self.acting_session = None;
            self.acting_device_input = None;
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
        if self.read_only || !self.holds_live_do_lease() {
            return None;
        }
        let deliverable = self.program_healthy && !self.storage_failed;
        let clock = self.core.clock();
        let now_instant = clock.now_instant();
        // A fuel-rate refusal backs delivery off until one full window
        // past the refusal (#1067): the budget's window started no
        // later than the refusal, so that instant is guaranteed to
        // land in a fresh window.
        let fuel_backoff = self
            .fuel_retry_not_before
            .filter(|deadline| *deadline > now_instant);
        self.ticks
            .values()
            .min()
            .copied()
            .into_iter()
            .chain(next_schedule_wake(
                &self.core.schedules,
                clock.now_ms(),
                now_instant,
                deliverable,
                fuel_backoff,
            ))
            .chain(self.loc_watches.values().min().copied())
            .chain(self.provider_broker.next_deadline())
            // A queued capability result needs a wake at the retry
            // deadline too, or the loop would sleep past the rollover
            // with a result still waiting (#1067).
            .chain(fuel_backoff.filter(|_| !self.pending_results.is_empty()))
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
        self.check_storage()?;
        self.check_do_lease()
    }

    fn check_do_lease(&self) -> Result<()> {
        let lease = self.core.write_lease();
        lease
            .check_writer(
                DO_WRITE_LEASE_HOLDER,
                lease.epoch,
                self.core.clock().now_ms(),
            )
            .map_err(anyhow::Error::new)
    }

    fn holds_live_do_lease(&self) -> bool {
        self.check_do_lease().is_ok()
    }

    /// Persist a DO-mediated ownership boundary before changing the live
    /// lease. The host adapter is responsible for authenticating this control
    /// request and meeting the drain/replay barrier before invoking it.
    fn handoff_write_lease(
        &mut self,
        to: String,
        epoch: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<u64> {
        self.check_not_replica()?;
        self.check_storage()?;
        ensure!(
            self.journal.is_some(),
            "write lease handoff requires a durable journal"
        );
        let from = self
            .core
            .write_lease()
            .holder
            .clone()
            .context("write lease has no current holder")?;
        ensure!(from != to, "write lease handoff must change holders");
        ensure!(
            from == DO_WRITE_LEASE_HOLDER || to == DO_WRITE_LEASE_HOLDER,
            "write lease handoff must include the DO authority"
        );
        // Expiry already fenced a native holder at the append gate, but
        // recovery must use revoke/grant, not handoff. Refuse here so an
        // expired term cannot be renewed by changing the holder.
        let now_ms = self.core.clock().now_ms();
        self.core
            .write_lease()
            .check_writer(&from, self.core.write_lease().epoch, now_ms)
            .map_err(anyhow::Error::new)?;
        if to == DO_WRITE_LEASE_HOLDER {
            ensure!(
                expires_at_ms.is_none(),
                "the DO write lease must not have a term expiry"
            );
        } else {
            ensure!(
                expires_at_ms.is_some_and(|expiry| expiry > now_ms),
                "a native write lease requires a future expiry"
            );
        }

        // The handoff's journal position is the ownership boundary
        // (#13711): stamp the next position on the entry and on the
        // lease history in the same step, before the durable append.
        // If the append fails, `next_lease` is dropped and the
        // counter — derived from the history — never advanced, so
        // the retry reuses the same position.
        let position = self.core.next_lease_position();
        let mut next_lease = self.core.write_lease().clone();
        next_lease.apply_handoff(&from, &to, epoch, expires_at_ms, position)?;
        self.persist(JournalEntry::LeaseHandedOff {
            from,
            to,
            epoch,
            expires_at_ms,
            position,
        })?;
        self.core.write_lease = next_lease;
        self.core.since_checkpoint = self.core.since_checkpoint.saturating_add(1);
        Ok(epoch)
    }

    /// Execute a spill v2 automatic failover (design #5787 §1, #7572):
    /// plan from the journaled lease and the grantor-side pool, then
    /// journal the plan's entries in order. The live lease advances to
    /// match the journaled prefix after each entry, so a crash — or a
    /// failed grant append — between the revoke and the grant leaves
    /// the world holder-less (writers refused `holder-lost`), never
    /// with two holders. The host adapter authenticates this control
    /// request and supplies the pool observations and the journal tip;
    /// the returned execution names the grant position the successor
    /// must replay through before it may write.
    fn failover_write_lease(
        &mut self,
        pool: &spill_pool::SpillPool,
        journal_tip: u64,
        max_catch_up_lag: u64,
        term_ms: u64,
    ) -> Result<spill_pool::FailoverExecution> {
        self.check_not_replica()?;
        self.check_storage()?;
        ensure!(
            self.journal.is_some(),
            "write lease failover requires a durable journal"
        );
        let now_ms = self.core.clock().now_ms();
        let plan = pool
            .plan_failover(
                self.core.write_lease(),
                now_ms,
                journal_tip,
                max_catch_up_lag,
            )
            .map_err(anyhow::Error::new)?;
        let execution = plan.execute(self.core.write_lease(), now_ms, term_ms)?;
        let entries = execution.entries().to_vec();
        let last = entries.len() - 1;
        for (index, entry) in entries.into_iter().enumerate() {
            self.persist(entry)?;
            self.core.write_lease = if index == last {
                execution.resulting_lease().clone()
            } else {
                execution.intermediate_lease().clone()
            };
            self.core.since_checkpoint = self.core.since_checkpoint.saturating_add(1);
        }
        Ok(execution)
    }

    /// Policy-B admission, checked at every token-admission entry
    /// point (participant lookup, scope authorization, snapshot): a
    /// participant present at the verified cursor is an existing
    /// session and is never gated; anyone else is a NEW spectator and
    /// the installed gate must admit them at the current time. No gate
    /// installed is no check.
    fn check_admission(&self, participant_id: u64) -> Result<()> {
        let Some(gate) = &self.admission_gate else {
            return Ok(());
        };
        gate.admit(participant_id, self.core.clock().now_ms())
            .map_err(|error| {
                anyhow::anyhow!("replica admission refused for new spectator: {error}")
            })
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
            WorldCommand::DirectoryAccount { account, reply } => {
                let result = self.check_not_replica().map(|()| {
                    let directory = self.core.directory();
                    directory
                        .account(&account)
                        .map(|_| directory.is_verified(&account))
                });
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
                let revoked_at = Some(self.core.clock().now_ms());
                let revoked = self.core.revoke_tree_at(participant_id, by, revoked_at);
                let was_active = !revoked.is_empty();
                if was_active {
                    // Fail-closed: surface a persist failure rather than
                    // reporting an audited success (design #835 phase 2).
                    let persisted = self.persist(JournalEntry::Revoked {
                        participant_id,
                        by,
                        revoked_at,
                    });
                    for id in revoked {
                        let _ = self.revocation_sender.send(id);
                    }
                    self.audit_admin_result(
                        by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                        "Revoke",
                        serde_json::json!({"participant_id": participant_id, "changed": was_active}),
                        persisted.is_ok(),
                    );
                    let _ = reply.send(persisted.is_ok() && was_active);
                    return;
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
                            let revoked_at = Some(self.core.clock().now_ms());
                            let revoked =
                                self.core
                                    .revoke_tree_at(participant_id, Some(by), revoked_at);
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
                                    revoked_at,
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
                let result =
                    self.core
                        .authorize(&token, WorldScopes::JOIN)
                        .and_then(|participant| {
                            self.check_admission(participant).map(|()| participant)
                        });
                let _ = reply.send(result);
            }
            WorldCommand::AuthorizeScope {
                token,
                required,
                reply,
            } => {
                let result = self
                    .core
                    .authorize(&token, required)
                    .and_then(|participant| {
                        self.check_admission(participant).map(|()| participant)
                    });
                let _ = reply.send(result);
            }
            WorldCommand::BindAuthorKey {
                token,
                author_key,
                reply,
            } => {
                // Author-key bindings change what later inputs the world
                // accepts; a replica takes no such local state. A new
                // binding is journaled (#4346) so the participant's chain
                // position survives a restart.
                let result = self.check_writable().and_then(|()| {
                    let (participant_id, entry) =
                        self.core.bind_author_key_entry(&token, author_key)?;
                    if let Some(entry) = entry {
                        self.persist(entry)?;
                    }
                    Ok(participant_id)
                });
                let _ = reply.send(result);
            }
            WorldCommand::Presence {
                participant,
                joined,
            } => {
                self.handle_presence(participant, joined).await;
            }
            WorldCommand::SubmitFormResult {
                token,
                session_id,
                result,
                reply,
            } => {
                let outcome = self.submit_form_result(token, session_id, result).await;
                let _ = reply.send(outcome);
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
                device_input,
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
                        self.acting_device_input = device_input;
                        match self
                            .program
                            .update(&candidate)
                            .and_then(|()| program_records(self.program.as_mut()))
                        {
                            Ok(records) => {
                                // A signed input and its acceptance are one
                                // journal transaction (Codex review on PR
                                // #9037): both entries are persisted
                                // atomically BEFORE either is applied in
                                // memory. The previous two-append order let
                                // a crash leave a committed input whose
                                // signed chain never advanced — restore
                                // replayed the input, a retry executed it
                                // again, and the author's next sequence was
                                // refused.
                                let committed = JournalEntry::Committed {
                                    event: candidate.clone(),
                                };
                                let persisted = match &signed {
                                    Some(signed) => self
                                        .core
                                        .signed_move_record(&candidate, signed)
                                        .and_then(|record| {
                                            self.persist_transaction(&[
                                                committed,
                                                JournalEntry::SignedMoveAccepted {
                                                    record: JournalSignedMove::from_record(&record),
                                                },
                                            ])
                                        }),
                                    None => self.persist(committed),
                                };
                                persisted
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
                                    })
                            }
                            Err(error) => {
                                if is_fuel_rate_refusal(&error) {
                                    // Recoverable admission refusal
                                    // (#1067): the program never ran
                                    // and stays healthy; the same
                                    // input can succeed once the
                                    // account's window rolls.
                                    Err(error.context(
                                        "world program refused input at its fuel rate",
                                    ))
                                } else {
                                    self.program_healthy = false;
                                    Err(error.context("world program rejected input"))
                                }
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
                                if is_fuel_rate_refusal(&error) {
                                    // Recoverable admission refusal
                                    // (#1067): the program never ran
                                    // and stays healthy; the same
                                    // input can succeed once the
                                    // account's window rolls.
                                    Err(error.context(
                                        "world program refused entity input at its fuel rate",
                                    ))
                                } else {
                                    self.program_healthy = false;
                                    Err(error.context("world program rejected entity input"))
                                }
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
            WorldCommand::HandoffWriteLease {
                to,
                epoch,
                expires_at_ms,
                reply,
            } => {
                let result = self.handoff_write_lease(to, epoch, expires_at_ms);
                let _ = reply.send(result);
            }
            WorldCommand::FailoverWriteLease {
                pool,
                journal_tip,
                max_catch_up_lag,
                term_ms,
                reply,
            } => {
                let result =
                    self.failover_write_lease(&pool, journal_tip, max_catch_up_lag, term_ms);
                let _ = reply.send(result);
            }
            WorldCommand::DispatchSpillCopy {
                holder,
                epoch,
                replayed_position,
                journal_copy,
                reply,
            } => {
                let result =
                    self.core
                        .dispatch_spill_copy(&holder, epoch, replayed_position, journal_copy);
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
                // Admission resolves the same JOIN authorization the
                // snapshot itself performs, so the gate sees exactly
                // the participant the snapshot would serve.
                let result = self
                    .core
                    .authorize(&token, WorldScopes::JOIN)
                    .and_then(|participant| self.check_admission(participant))
                    .and_then(|()| self.core.snapshot(&token));
                let _ = reply.send(result);
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
                let sequence = match self.core.sequence.checked_add(1) {
                    Some(sequence) => sequence,
                    None => {
                        self.audit_admin_result(
                            installed_by.map_or_else(|| "admin".to_owned(), |id| format!("participant:{id}")),
                            "InstallProgram",
                            serde_json::json!({"module_hash": module_hash, "seed": seed, "installed_by": installed_by}),
                            false,
                        );
                        let _ = reply.send(Err(anyhow::anyhow!("world sequence exhausted")));
                        return;
                    }
                };
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
                    sequence,
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
                        self.pending_forms.clear();
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
                let state = (self.core.active_seed, self.core.schedule_times());
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
                    if self.core.schedules.get(&id) == Some(&PendingSchedule::one_shot(at_ms)) {
                        return Ok(self.core.schedule_times());
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
                        interval_ms: None,
                        remaining: None,
                        target: None,
                    })?;
                    self.core
                        .schedules
                        .insert(id, PendingSchedule::one_shot(at_ms));
                    Ok(self.core.schedule_times())
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
                    Ok(self.core.schedule_times())
                })();
                self.audit_admin_result(
                    "admin",
                    "AdminCancelSchedule",
                    serde_json::json!({"id": id.clone()}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::AdminSetScheduleGrant {
                peer,
                send,
                receive,
                reply,
            } => {
                let audit_peer = peer.clone();
                let result = (|| -> Result<BTreeMap<String, ScheduleGrant>> {
                    self.check_writable()?;
                    crate::world_hub::validate_world_name(&peer)?;
                    self.persist(JournalEntry::ScheduleGrantSet {
                        peer: peer.clone(),
                        send,
                        receive,
                    })?;
                    if send || receive {
                        self.core
                            .schedule_grants
                            .insert(peer, ScheduleGrant { send, receive });
                    } else {
                        self.core.schedule_grants.remove(&peer);
                    }
                    Ok(self.core.schedule_grants.clone())
                })();
                self.audit_admin_result(
                    "admin",
                    "AdminSetScheduleGrant",
                    serde_json::json!({"peer": audit_peer, "send": send, "receive": receive}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::DeliverSchedule {
                source,
                id,
                at_ms,
                late,
                reply,
            } => {
                let accepted = self
                    .accept_schedule_delivery(&source, &id, at_ms, late)
                    .await;
                let _ = reply.send(accepted);
            }
            WorldCommand::SetScheduleDelivery { delivery, reply } => {
                self.schedule_delivery = Some(delivery);
                let _ = reply.send(());
            }
            WorldCommand::AdminSetPortalGrant {
                peer,
                granted,
                reply,
            } => {
                let audit_peer = peer.clone();
                let result = (|| -> Result<id_caps::portal::PortalGrants> {
                    self.check_writable()?;
                    crate::world_hub::validate_world_name(&peer)?;
                    if granted {
                        self.persist(JournalEntry::PortalGrantSet {
                            peer: peer.clone(),
                            granted: true,
                        })?;
                        self.core.portal_grants.grant(&peer);
                    } else if self.core.portal_grants.state(&peer)
                        == Some(id_caps::portal::PortalGrantState::Granted)
                    {
                        // Only a live grant's revocation is journaled:
                        // revoking a peer that was never granted (or is
                        // already revoked) changes no recorded state,
                        // so it must not mint a revoked tombstone the
                        // live book does not have.
                        self.persist(JournalEntry::PortalGrantSet {
                            peer: peer.clone(),
                            granted: false,
                        })?;
                        self.core.portal_grants.revoke(&peer);
                    }
                    Ok(self.core.portal_grants.clone())
                })();
                self.audit_admin_result(
                    "admin",
                    "AdminSetPortalGrant",
                    serde_json::json!({"peer": audit_peer, "granted": granted}),
                    result.is_ok(),
                );
                let _ = reply.send(result);
            }
            WorldCommand::PortalGrantsInfo { reply } => {
                let _ = reply.send(self.core.portal_grants.clone());
            }
            WorldCommand::PortalSendPrecheck {
                target,
                bytes,
                reply,
            } => {
                let result = self.portal_send_precheck(&target, bytes);
                let _ = reply.send(result);
            }
            WorldCommand::PortalSendAdmit {
                target,
                peer_grants,
                message,
                reply,
            } => {
                let result = self.portal_send_admit(&target, &peer_grants, &message);
                let _ = reply.send(result);
            }
            WorldCommand::PortalSendRecord {
                target,
                bytes,
                refusal,
                reply,
            } => {
                let result = self.portal_send_record(&target, bytes, refusal);
                let _ = reply.send(result);
            }
            WorldCommand::DeliverPortal {
                source,
                message,
                reply,
            } => {
                let result = self.accept_portal_delivery(&source, message).await;
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
                let participants = self.presence.values().filter(|n| **n > 0).count() as u64;
                let metrics = self
                    .metrics
                    .snapshot_with_participants(sessions, participants);
                // No audit record: metrics are readable by any joined
                // participant holding a capability (the session layer's
                // Metrics frame), so recording the read as admin would
                // falsify the audit actor. Participant-readable queries are
                // not administrative actions and stay out of the audit trail.
                let _ = reply.send(metrics);
            }
            WorldCommand::ProgramHealth { reply } => {
                // No audit record, for the same reason as Metrics above:
                // a participant-readable state query, not an admin action.
                let _ = reply.send(self.program_healthy);
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
            WorldCommand::ReplicaSidecars { subjects, reply } => {
                let result = (|| -> Result<(Vec<u8>, Option<String>)> {
                    let facts = self.core.directory().replica_facts_for(&subjects)?;
                    let mut bytes = Vec::new();
                    for entry in &facts {
                        serde_json::to_writer(&mut bytes, entry)?;
                        bytes.push(b'\n');
                    }
                    Ok((bytes, self.core.active_module_hash.clone()))
                })();
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
                kv: None,
                blobs: None,
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
            kv: None,
            blobs: None,
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
                revoked_at: None,
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
                revoked_at: None,
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
    use crate::world_caps::{CapError, DeviceManifest, Request};

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

    fn loc_actor() -> WorldActor {
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["loc.position".to_owned()]));
        // Sensor caps sit outside the default ceiling (#2341): the
        // world must name them in its ceiling as well as grant them.
        let mut allowed = crate::world_caps::default_ceiling_names();
        allowed.insert("loc.position".to_owned());
        core.ledger.set_allowed(allowed);
        WorldActor::for_test(core, Box::new(DeviceInfoProgram), None)
    }

    fn loc_request(args: serde_json::Value) -> Request {
        Request {
            id: "l1".to_owned(),
            cap: "loc.position".to_owned(),
            args,
        }
    }

    fn sample_reading() -> crate::world_caps::LocationReading {
        crate::world_caps::LocationReading {
            lat: 52.5,
            lon: 13.4,
            accuracy_m: 12.0,
            fix_age_ms: 250,
            source: crate::world_caps::LocationSource::Gnss,
        }
    }

    #[test]
    fn loc_position_refuses_until_the_consent_seam_grants() {
        // The default host records no consent (#2341): the refusal is
        // the consent seam's own code, before any source question.
        let mut actor = loc_actor();
        let error = actor
            .execute(&loc_request(serde_json::json!({})))
            .unwrap_err();
        assert_eq!(error.code(), CapError::consent_required().code());

        actor.location_consent = crate::world_caps::LocationConsent::OsPermissionDenied;
        let error = actor
            .execute(&loc_request(serde_json::json!({})))
            .unwrap_err();
        assert_eq!(error.code(), CapError::os_permission_denied().code());
    }

    #[test]
    fn loc_position_is_no_fix_when_the_host_source_has_none() {
        let mut actor = loc_actor();
        actor.location_consent = crate::world_caps::LocationConsent::Granted;
        let error = actor
            .execute(&loc_request(serde_json::json!({})))
            .unwrap_err();
        assert_eq!(error.code(), CapError::no_fix().code());
    }

    #[test]
    fn loc_position_refuses_a_fix_beyond_the_precision_ceiling() {
        let mut actor = loc_actor();
        actor.location_consent = crate::world_caps::LocationConsent::Granted;
        actor.set_location_fix(sample_reading());
        let error = actor
            .execute(&loc_request(serde_json::json!({"max_accuracy_m": 5})))
            .unwrap_err();
        assert_eq!(error.code(), CapError::precision_ceiling().code());

        let value = actor
            .execute(&loc_request(serde_json::json!({"max_accuracy_m": 20})))
            .unwrap();
        assert_eq!(value, sample_reading().to_json());
    }

    #[test]
    fn loc_position_refuses_a_malformed_accuracy_ceiling() {
        // Oracle: PR #8643 Codex P2 — a supplied `max_accuracy_m`
        // that is not an unsigned integer (5.5, "5") is `bad_args`,
        // never a silently disabled precision check: with consent
        // granted and a 12-metre fix available, the malformed
        // requests must not succeed with the imprecise reading.
        let mut actor = loc_actor();
        actor.location_consent = crate::world_caps::LocationConsent::Granted;
        actor.set_location_fix(sample_reading());
        for bad in [serde_json::json!(5.5), serde_json::json!("5")] {
            let error = actor
                .execute(&loc_request(serde_json::json!({"max_accuracy_m": bad})))
                .unwrap_err();
            assert_eq!(error.code(), CapError::bad_args().code());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod authority_boundary_tests {
    use super::*;
    use crate::world_caps::{
        CATALOG_ENTRIES, CapError, DeviceManifest, ProviderKind, ProviderSide, Request,
    };

    struct BoundaryProgram;

    impl WorldProgram for BoundaryProgram {
        fn snapshot(&mut self) -> Result<Option<String>> {
            Ok(Some("{}".to_owned()))
        }
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

    /// An actor with every registered provider granted and inside the
    /// ceiling, so `execute` reaches its dispatch match for each name
    /// instead of stopping at the ledger.
    fn boundary_actor() -> WorldActor {
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        let all: BTreeSet<String> = CATALOG_ENTRIES
            .iter()
            .map(|entry| entry.name.to_owned())
            .collect();
        core.ledger.set_granted(all.clone());
        core.ledger.set_allowed(all);
        let mut actor = WorldActor::for_test(core, Box::new(BoundaryProgram), None);
        // `device.info` reports `unknown_cap` when the host cannot
        // describe its device; a manifest is supplied so the test
        // measures the dispatch arm, not the host's self-description.
        actor.device_manifest = Some(sample_manifest());
        actor
    }

    fn request(cap: &str) -> Request {
        Request {
            id: "b1".to_owned(),
            cap: cap.to_owned(),
            args: serde_json::json!({}),
        }
    }

    #[test]
    fn every_host_request_provider_has_an_execute_arm() {
        // Oracle: the authority-boundary audit (#12597) — the host
        // request dispatch must serve every host-side Request-kind
        // registry entry. A registered provider with no executor is
        // a fail-closed gap, not a bypass, but the audit pins it
        // here so registry growth without an executor fails loudly
        // instead of silently returning `unknown_cap` to a granted
        // world. The two speech request providers are the
        // documented exception (audit finding F-1): registered but
        // with no executor, so a granted request fails closed.
        let mut actor = boundary_actor();
        let mut served = 0;
        for entry in CATALOG_ENTRIES {
            if entry.kind != ProviderKind::Request || entry.side != ProviderSide::Host {
                continue;
            }
            match actor.execute(&request(entry.name)) {
                Ok(_) => served += 1,
                Err(error) if matches!(entry.name, "speech.transcribe" | "speech.synthesize") => {
                    assert_eq!(
                        error.code(),
                        CapError::unknown().code(),
                        "{} is registered with no executor and must fail closed",
                        entry.name
                    );
                }
                Err(error) => {
                    assert_ne!(
                        error.code(),
                        CapError::unknown().code(),
                        "{} must dispatch to an executor, got unknown_cap",
                        entry.name
                    );
                    served += 1;
                }
            }
        }
        let expected = CATALOG_ENTRIES
            .iter()
            .filter(|entry| entry.kind == ProviderKind::Request && entry.side == ProviderSide::Host)
            .count()
            - 2;
        assert_eq!(
            served, expected,
            "every host request provider except the two documented speech exceptions must dispatch"
        );
    }

    #[test]
    fn unregistered_names_fail_closed_with_unknown_cap() {
        // Oracle: the authority-boundary audit (#12597) — a name
        // outside the registry never reaches an executor: the ledger
        // denies it before the dispatch match is consulted.
        let mut actor = boundary_actor();
        for name in [
            "not.a.provider",
            "kv",
            "http",
            "client.file.write",
            "speech",
            "form.unknown",
        ] {
            let error = actor.execute(&request(name)).unwrap_err();
            assert_eq!(
                error.code(),
                CapError::unknown().code(),
                "{name} must fail closed with unknown_cap"
            );
        }
    }

    #[test]
    fn client_providers_never_execute_on_the_host() {
        // Oracle: the authority-boundary audit (#12597) — a
        // client-side provider is brokered to the originating
        // session, never executed by the host, even when a caller
        // reaches `execute` directly instead of the broker route.
        let mut actor = boundary_actor();
        let error = actor.execute(&request("client.file.read")).unwrap_err();
        assert_eq!(error.code(), CapError::failed().code());
        assert_eq!(
            error.reason().map(String::as_str),
            Some("client_provider_async")
        );
    }

    #[test]
    fn event_only_and_subscription_names_do_not_execute() {
        // Oracle: the authority-boundary audit (#12597) — `execute`
        // serves host requests only. `form.result` is an event, so
        // it is refused as event-only; subscriptions are delivered
        // by arm/fire, so naming one as a request fails closed.
        let mut actor = boundary_actor();
        let error = actor.execute(&request("form.result")).unwrap_err();
        assert_eq!(error.code(), CapError::bad_args().code());
        for name in ["time.tick", "players", "loc.watch", "speech.capture"] {
            let error = actor.execute(&request(name)).unwrap_err();
            assert_eq!(
                error.code(),
                CapError::unknown().code(),
                "{name} is a subscription, not a request"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod form_tests {
    use super::*;
    use crate::world_caps::{CapError, Request};
    use id_core::world_presentation::{
        FormAction, FormActionKind, FormField, FormResult, FormSurface,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FormEventRecorder(Arc<Mutex<Vec<String>>>);

    impl WorldProgram for FormEventRecorder {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                self.0.lock().unwrap().push(event.clone());
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let accepted_result = self.0.lock().unwrap().iter().any(|event| {
                serde_json::from_str::<serde_json::Value>(event)
                    .is_ok_and(|event| event["cap"] == "form.result" && event["ok"] != false)
            });
            Ok(accepted_result.then(|| {
                serde_json::json!({
                    "requests": [{
                        "id": "chat-after-form",
                        "cap": "chat.say",
                        "args": {"text": "must not be sent by a JOIN-only participant"}
                    }]
                })
                .to_string()
            }))
        }
    }

    fn form(fields: Vec<FormField>) -> FormSurface {
        FormSurface {
            version: 1,
            form_id: "form-1".to_owned(),
            presentation_token: None,
            title: "Profile".to_owned(),
            fields,
            actions: vec![
                FormAction {
                    id: "submit".to_owned(),
                    label: "Save".to_owned(),
                    kind: FormActionKind::Submit,
                },
                FormAction {
                    id: "cancel".to_owned(),
                    label: "Cancel".to_owned(),
                    kind: FormActionKind::Cancel,
                },
            ],
        }
    }

    fn form_actor() -> (WorldActor, Arc<Mutex<Vec<String>>>) {
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.ledger.set_granted(BTreeSet::from([
            "form.present".to_owned(),
            "form.result".to_owned(),
            "form.cancel".to_owned(),
            "chat.say".to_owned(),
        ]));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut actor =
            WorldActor::for_test(core, Box::new(FormEventRecorder(Arc::clone(&events))), None);
        actor.device_manifest = Some(crate::world_caps::DeviceManifest {
            display: serde_json::json!({"kind": "pixel"}),
            input: serde_json::json!({"keys": 4, "text_entry": "keyboard"}),
            sensors: serde_json::json!({}),
            audio: serde_json::json!({}),
            network: serde_json::json!({}),
        });
        actor.acting_device_input = None;
        (actor, events)
    }

    #[tokio::test]
    async fn test_restored_form_invalidates_when_only_result_capability_remains() {
        let (mut actor, events) = form_actor();
        actor
            .core
            .ledger
            .set_granted(BTreeSet::from(["form.result".to_owned()]));

        actor.invalidate_restored_form_presentations().await;

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
        assert_eq!(event["event"], "invalidated");
        assert_eq!(event["reason"], "restarted");
    }

    fn present(actor: &mut WorldActor, form: &FormSurface) -> Result<serde_json::Value, CapError> {
        actor.execute(&Request {
            id: "present-request".to_owned(),
            cap: "form.present".to_owned(),
            args: serde_json::json!({
                "form_id": form.form_id,
                "form": form,
            }),
        })
    }

    #[test]
    fn text_entry_none_refuses_text_but_allows_encoder_compatible_fields() {
        let (mut actor, _) = form_actor();
        actor.device_manifest = Some(crate::world_caps::DeviceManifest {
            display: serde_json::json!({"kind": "pixel"}),
            input: serde_json::json!({"encoder": true, "text_entry": "none"}),
            sensors: serde_json::json!({}),
            audio: serde_json::json!({}),
            network: serde_json::json!({}),
        });
        let text_form = form(vec![FormField::Text {
            id: "name".to_owned(),
            label: "Name".to_owned(),
            required: true,
            max_length: 32,
        }]);
        assert_eq!(
            present(&mut actor, &text_form)
                .unwrap_err()
                .reason()
                .map(String::as_str),
            Some("text_entry_unavailable")
        );

        let encoder_form = form(vec![FormField::Choice {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            required: true,
            options: vec!["quiet".to_owned(), "loud".to_owned()],
        }]);
        assert!(present(&mut actor, &encoder_form).is_ok());
    }

    #[test]
    fn hosted_headless_manifest_does_not_mask_remote_presenter_input() {
        let (mut actor, _) = form_actor();
        let headless = crate::world::WorldHandle::host_device_manifest();
        actor.device_manifest = Some(headless.clone());
        actor.acting_device_input = Some(serde_json::json!({
            "keys": 128, "text_entry": "keyboard", "encoder": false, "pointer": true
        }));
        let form = form(vec![FormField::Text {
            id: "name".to_owned(),
            label: "Name".to_owned(),
            required: true,
            max_length: 32,
        }]);
        assert!(present(&mut actor, &form).is_ok());
        assert!(form.validate_device_input(&headless.input).is_err());
        assert!(
            form.validate_device_input(&serde_json::json!({
                "keys": 128, "text_entry": "keyboard", "encoder": false, "pointer": false
            }))
            .is_ok()
        );
    }

    #[test]
    fn presenter_without_text_entry_is_refused_even_when_host_can_type() {
        let (mut actor, _) = form_actor();
        actor.acting_device_input = Some(serde_json::json!({
            "keys": 0, "text_entry": "none", "encoder": false, "pointer": false
        }));
        let text_form = form(vec![FormField::Text {
            id: "name".to_owned(),
            label: "Name".to_owned(),
            required: true,
            max_length: 32,
        }]);
        assert_eq!(
            present(&mut actor, &text_form)
                .unwrap_err()
                .reason()
                .map(String::as_str),
            Some("text_entry_unavailable")
        );
    }

    #[tokio::test]
    async fn form_result_correlates_validates_and_keeps_invalid_form_for_re_presentation() {
        let (mut actor, events) = form_actor();
        let form = form(vec![FormField::Choice {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            required: true,
            options: vec!["quiet".to_owned(), "loud".to_owned()],
        }]);
        let presented = present(&mut actor, &form).unwrap();
        let presentation_token = presented["presentation_token"].as_str().unwrap().to_owned();

        let invalid = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: presentation_token.clone(),
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "unsafe"})
                .as_object()
                .unwrap()
                .clone(),
        };
        let FormSubmissionStatus::Rejected {
            presentation_token: retry_token,
        } = actor.submit_form_result(None, None, invalid).await.unwrap()
        else {
            panic!("invalid result must request a re-presentation")
        };
        assert_ne!(
            retry_token, presentation_token,
            "re-presentation rotates token"
        );
        assert!(actor.pending_forms.contains_key("form-1"));
        let events_after_rejection = events.lock().unwrap().clone();
        assert_eq!(
            events_after_rejection.len(),
            1,
            "one refusal, no retry loop"
        );
        let refusal: serde_json::Value = serde_json::from_str(&events_after_rejection[0]).unwrap();
        assert_eq!(refusal["cap"], "form.result");
        assert_eq!(refusal["form_id"], "form-1");
        assert_eq!(refusal["error"], "bad_args");
        assert_eq!(refusal["reason"], "field_invalid");

        let valid = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: retry_token,
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "quiet"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(
            actor.submit_form_result(None, None, valid).await.unwrap(),
            FormSubmissionStatus::Accepted
        );
        assert!(!actor.pending_forms.contains_key("form-1"));
        let delivered = events.lock().unwrap();
        assert_eq!(delivered.len(), 2);
        let result: serde_json::Value = serde_json::from_str(&delivered[1]).unwrap();
        assert_eq!(result["cap"], "form.result");
        assert_eq!(result["form_id"], "form-1");
        assert_eq!(result["action"], "submit");
        assert_eq!(result["values"]["mode"], "quiet");
        assert!(
            delivered
                .iter()
                .all(|event| event.len() <= crate::world_caps::MAX_EVENT_BYTES)
        );
    }

    #[tokio::test]
    async fn test_form_result_keeps_submitter_effect_scope_through_after_change() {
        let (mut actor, events) = form_actor();
        let (_participant, capability) = actor
            .core
            .issue_capability(
                "Ada",
                crate::world::WorldScopes::JOIN,
                crate::world::CapabilityBounds::default(),
            )
            .unwrap();
        let form = form(vec![FormField::Choice {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            required: true,
            options: vec!["quiet".to_owned()],
        }]);
        let presented = present(&mut actor, &form).unwrap();
        let result = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: presented["presentation_token"].as_str().unwrap().to_owned(),
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "quiet"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(
            actor
                .submit_form_result(Some(capability), Some(crate::world::SessionId(42)), result)
                .await
                .unwrap(),
            FormSubmissionStatus::Accepted
        );
        assert!(matches!(
            actor.acting_session,
            Some(crate::world::SessionId(42))
        ));

        // The program reacts to the human result by trying chat.say. The
        // actor must retain the submitting participant through wants execution.
        actor.after_change().await;
        let events = events.lock().unwrap();
        let denied: serde_json::Value = events
            .iter()
            .filter_map(|event| serde_json::from_str::<serde_json::Value>(event).ok())
            .find(|event| event["cap"] == "result" && event["id"] == "chat-after-form")
            .expect("the program's follow-up request should receive a result");
        assert_eq!(denied["error"], "actor_denied");
    }

    #[tokio::test]
    async fn test_form_result_spends_submitter_budget_and_attributes_the_event() {
        let (mut actor, events) = form_actor();
        let (participant, capability) = actor
            .core
            .issue_capability(
                "Ada",
                crate::world::WorldScopes::GUEST,
                crate::world::CapabilityBounds {
                    uses: Some(1),
                    ..crate::world::CapabilityBounds::default()
                },
            )
            .unwrap();
        let form = form(vec![FormField::Choice {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            required: true,
            options: vec!["quiet".to_owned()],
        }]);
        let presented = present(&mut actor, &form).unwrap();
        let result = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: presented["presentation_token"].as_str().unwrap().to_owned(),
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "quiet"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(
            actor
                .submit_form_result(Some(capability.clone()), None, result)
                .await
                .unwrap(),
            FormSubmissionStatus::Accepted
        );
        assert_eq!(actor.core.uses_left(participant.id), Some(0));
        let submitted: serde_json::Value =
            serde_json::from_str(&events.lock().unwrap()[0]).unwrap();
        assert_eq!(submitted["participant_id"], participant.id);
    }

    #[tokio::test]
    async fn test_exhausted_form_result_capability_cannot_trigger_another_event() {
        let (mut actor, events) = form_actor();
        let (_participant, capability) = actor
            .core
            .issue_capability(
                "Ada",
                crate::world::WorldScopes::GUEST,
                crate::world::CapabilityBounds {
                    uses: Some(1),
                    ..crate::world::CapabilityBounds::default()
                },
            )
            .unwrap();
        let form = form(vec![FormField::Choice {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            required: true,
            options: vec!["quiet".to_owned()],
        }]);
        let presented = present(&mut actor, &form).unwrap();
        let invalid = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: presented["presentation_token"].as_str().unwrap().to_owned(),
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "invalid"})
                .as_object()
                .unwrap()
                .clone(),
        };
        let FormSubmissionStatus::Rejected { presentation_token } = actor
            .submit_form_result(Some(capability.clone()), None, invalid)
            .await
            .unwrap()
        else {
            panic!("invalid result should produce one field_invalid event")
        };
        let retry = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token,
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "quiet"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert!(matches!(
            actor
                .submit_form_result(Some(capability), None, retry)
                .await
                .unwrap(),
            FormSubmissionStatus::Unavailable {
                error: "actor_denied",
                reason: "use_limit_exhausted"
            }
        ));
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn form_present_grant_on_use_opens_the_result_channel() {
        let (mut actor, events) = form_actor();
        actor.core.ledger = crate::world_caps::CapLedger::new(BTreeSet::new(), true);
        let form = form(vec![FormField::Choice {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            required: true,
            options: vec!["quiet".to_owned(), "loud".to_owned()],
        }]);
        let presented = present(&mut actor, &form).unwrap();
        assert!(
            actor.core.ledger.is_granted("form.result"),
            "presenting must open the result channel under grant-on-use"
        );
        let valid = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: presented["presentation_token"].as_str().unwrap().to_owned(),
            action: "submit".to_owned(),
            values: serde_json::json!({"mode": "quiet"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(
            actor.submit_form_result(None, None, valid).await.unwrap(),
            FormSubmissionStatus::Accepted
        );
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[test]
    fn form_present_rejects_a_form_whose_token_bearing_view_exceeds_limits() {
        let (mut actor, _) = form_actor();
        let bare = form(vec![FormField::Text {
            id: "name".to_owned(),
            label: String::new(),
            required: true,
            max_length: 32,
        }]);
        let base_len = serde_json::to_vec(&bare).unwrap().len();
        // The bare form fits with 10 bytes to spare; the token-bearing form
        // the program must display does not.
        let pad = id_core::world_presentation::MAX_FORM_BYTES - base_len - 10;
        let oversized = form(vec![FormField::Text {
            id: "name".to_owned(),
            label: "x".repeat(pad),
            required: true,
            max_length: 32,
        }]);
        assert!(
            serde_json::to_vec(&oversized).unwrap().len()
                <= id_core::world_presentation::MAX_FORM_BYTES
        );
        assert!(present(&mut actor, &oversized).is_err());
        assert!(actor.pending_forms.is_empty());
    }

    #[tokio::test]
    async fn oversized_form_result_event_is_rejected_before_delivery() {
        let (mut actor, events) = form_actor();
        actor.device_manifest = Some(crate::world_caps::DeviceManifest {
            display: serde_json::json!({"kind": "pixel"}),
            input: serde_json::json!({"text_entry": "keyboard"}),
            sensors: serde_json::json!({}),
            audio: serde_json::json!({}),
            network: serde_json::json!({}),
        });
        let form = form(vec![
            FormField::Text {
                id: "first".to_owned(),
                label: "First".to_owned(),
                required: true,
                max_length: u16::MAX,
            },
            FormField::Text {
                id: "second".to_owned(),
                label: "Second".to_owned(),
                required: true,
                max_length: u16::MAX,
            },
        ]);
        let presented = present(&mut actor, &form).unwrap();
        let presentation_token = presented["presentation_token"].as_str().unwrap().to_owned();
        let result = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token,
            action: "submit".to_owned(),
            values: serde_json::json!({
                "first": "a".repeat(33_000),
                "second": "b".repeat(33_000),
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        assert!(actor.submit_form_result(None, None, result).await.is_err());
        assert!(events.lock().unwrap().is_empty());
        assert!(actor.pending_forms.contains_key("form-1"));
    }

    #[tokio::test]
    async fn form_cancel_is_idempotent_and_prevents_late_results() {
        let (mut actor, events) = form_actor();
        let form = form(vec![FormField::Confirm {
            id: "confirm".to_owned(),
            label: "Continue?".to_owned(),
            required: true,
        }]);
        let first_presentation = present(&mut actor, &form).unwrap();
        let first_token = first_presentation["presentation_token"]
            .as_str()
            .unwrap()
            .to_owned();
        let cancel = Request {
            id: "cancel-request".to_owned(),
            cap: "form.cancel".to_owned(),
            args: serde_json::json!({"form_id": "form-1"}),
        };
        assert_eq!(actor.execute(&cancel).unwrap()["cancelled"], true);
        assert_eq!(actor.execute(&cancel).unwrap()["cancelled"], false);
        let second_presentation = present(&mut actor, &form).unwrap();
        let second_token = second_presentation["presentation_token"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(first_token, second_token);
        let late_result = FormResult {
            form_id: "form-1".to_owned(),
            presentation_token: first_token,
            action: "submit".to_owned(),
            values: serde_json::json!({"confirm": true})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert_eq!(
            actor
                .submit_form_result(None, None, late_result)
                .await
                .unwrap(),
            FormSubmissionStatus::Unavailable {
                error: "bad_args",
                reason: "stale_presentation",
            }
        );
        assert!(actor.pending_forms.contains_key("form-1"));
        assert!(events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn form_result_requires_current_grant_and_ceiling_authority() {
        for remove_from in ["grant", "ceiling"] {
            let (mut actor, events) = form_actor();
            let form = form(vec![FormField::Choice {
                id: "mode".to_owned(),
                label: "Mode".to_owned(),
                required: true,
                options: vec!["quiet".to_owned()],
            }]);
            let presented = present(&mut actor, &form).unwrap();
            let token = presented["presentation_token"].as_str().unwrap().to_owned();
            if remove_from == "grant" {
                actor.core.ledger.set_granted(BTreeSet::from([
                    "form.present".to_owned(),
                    "form.cancel".to_owned(),
                ]));
            } else {
                let allowed = actor
                    .core
                    .ledger
                    .allowed()
                    .difference(&BTreeSet::from(["form.result".to_owned()]))
                    .cloned()
                    .collect();
                actor.core.ledger.set_allowed(allowed);
            }
            let result = FormResult {
                form_id: "form-1".to_owned(),
                presentation_token: token,
                action: "submit".to_owned(),
                values: serde_json::json!({"mode": "quiet"})
                    .as_object()
                    .unwrap()
                    .clone(),
            };
            assert_eq!(
                actor.submit_form_result(None, None, result).await.unwrap(),
                FormSubmissionStatus::Unavailable {
                    error: "cap_denied",
                    reason: "capability_revoked",
                }
            );
            assert!(events.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn restored_form_authority_notifies_program_to_re_present() {
        let (mut actor, events) = form_actor();
        actor.invalidate_restored_form_presentations().await;
        let delivered = events.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&delivered[0]).unwrap();
        assert_eq!(event["cap"], "form.result");
        assert_eq!(event["event"], "invalidated");
        assert_eq!(event["reason"], "restarted");
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
            .filter(|(_, schedule)| schedule.at_ms <= core.clock().now_ms())
            .map(|(id, _)| id.clone())
            .collect();
        let provider_broker = ProviderBroker::new(core.world_id.clone(), WorldAuditLog::new());
        Self {
            core,
            program,
            journal,
            provider_broker,
            storage_failed: false,
            read_only: false,
            admission_gate: None,
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
            pending_results: std::collections::VecDeque::new(),
            fuel_retry_not_before: None,
            pending_forms: BTreeMap::new(),
            denied_subs: BTreeSet::new(),
            ticks: BTreeMap::new(),
            overdue_at_open,
            stale_wants_at_open: BTreeSet::new(),
            acting: None,
            acting_session: None,
            acting_device_input: None,
            device_manifest: None,
            location_consent: crate::world_caps::LocationConsent::ConsentRequired,
            location_fix: None,
            location_fix_received_at: None,
            loc_watches: BTreeMap::new(),
            schedule_delivery: None,
            portal_rate: BTreeMap::new(),
            auth: crate::directory_auth::DirectoryAuth::default(),
            metrics: WorldMetrics::default(),
            audit: WorldAuditLog::new(),
        }
    }
}
