use super::*;

use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

fn admin_history_bytes<'a>(events: impl IntoIterator<Item = &'a WorldEvent>) -> usize {
    events
        .into_iter()
        .map(|event| serde_json::to_vec(event).map_or(usize::MAX, |bytes| bytes.len()))
        .fold(0usize, usize::saturating_add)
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
    /// This world's own capability ceiling. The ledger allows the intersection
    /// of this and the server's `limits.server_caps`.
    pub(crate) caps_ceiling: BTreeSet<String>,
    /// Pending `schedule.at` fires: id to due time, in unix milliseconds.
    pub(crate) schedules: BTreeMap<String, u64>,
    pub(crate) isolation: Isolation,
    pub(crate) cross_world_write: bool,
    pub(crate) cross_world_forced: bool,
    pub(crate) file_scope: FileScope,
    pub(crate) file_write_override: bool,
    /// Events committed since the journal last started from a checkpoint.
    pub(crate) since_checkpoint: u64,
    pub(crate) capabilities: HashMap<u64, CapabilityRecord>,
    /// Recent chat timestamps, pruned to the active rolling window on each send.
    pub(crate) chat_rates: HashMap<u64, VecDeque<std::time::Instant>>,
    pub(crate) events: VecDeque<WorldEvent>,
    /// Bounded journal-order history for admin-authorized fixture export.
    pub(crate) admin_history: VecDeque<WorldEvent>,
    /// False once the oldest part of the journal history has been trimmed.
    pub(crate) admin_history_complete: bool,
    /// Participants whose join the program has seen without a later leave.
    /// Unlike the actor's live-session refcount, this survives compaction so
    /// restore can deliver one leave for sessions lost with the process.
    pub(crate) open_presence: BTreeSet<u64>,
    pub(crate) directory: crate::directory::Directory,
    pub(crate) petnames: crate::petname::Petnames,
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
            caps_ceiling: crate::world_caps::catalog_names(),
            schedules: BTreeMap::new(),
            isolation: Isolation::default(),
            cross_world_write: false,
            cross_world_forced: false,
            file_scope: FileScope::default(),
            file_write_override: false,
            since_checkpoint: 0,
            capabilities: HashMap::new(),
            chat_rates: HashMap::new(),
            events: VecDeque::new(),
            admin_history: VecDeque::new(),
            admin_history_complete: true,
            open_presence: BTreeSet::new(),
            directory: crate::directory::Directory::new(),
            petnames: crate::petname::Petnames::new(),
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
        let mut core = Self::new(world_id, limits)?;
        let mut program = None;
        let mut replay = Vec::new();
        let mut entries = entries.into_iter();
        let journal_version = match entries.next() {
            Some(JournalEntry::Created { world_id, version }) => {
                ensure!(
                    world_id == core.world_id,
                    "journal belongs to world {world_id:?}, not {:?}",
                    core.world_id
                );
                ensure!(
                    (JOURNAL_VERSION..=MAX_JOURNAL_VERSION).contains(&version),
                    "unsupported world journal version {version}"
                );
                version
            }
            Some(_) => anyhow::bail!("world journal does not start with a header"),
            None => {
                return Ok(RestoredWorld {
                    core,
                    program,
                    replay,
                });
            }
        };
        for entry in entries {
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
                    ensure!(
                        !core.capabilities.contains_key(&participant.id),
                        "participant {} issued twice",
                        participant.id
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
                    core.next_participant_id = core.next_participant_id.max(
                        participant
                            .id
                            .checked_add(1)
                            .context("participant ID exhausted")?,
                    );
                    core.capabilities.insert(
                        participant.id,
                        CapabilityRecord {
                            digest,
                            participant,
                            scopes: WorldScopes(scopes),
                            parent,
                            revoked: false,
                            bounds: CapabilityBounds { expires_at, uses },
                            used,
                            subject,
                        },
                    );
                }
                JournalEntry::Revoked { participant_id, .. } => {
                    ensure!(
                        !core.revoke_tree(participant_id).is_empty(),
                        "journal revokes unknown or revoked participant {participant_id}"
                    );
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
                        .resolve(crate::world_limits::RuntimeLimits::default())
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
                JournalEntry::ScheduleSet { id, at_ms } => {
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
                    // No clock here: the due time is what the request set, and
                    // a restore never judges it against now.
                    core.schedules.insert(id, at_ms);
                }
                JournalEntry::ScheduleCancelled { id } => {
                    ensure!(
                        core.schedules.remove(&id).is_some(),
                        "journal cancels schedule {id:?}, which is not pending"
                    );
                }
                JournalEntry::ScheduleFired { id } => {
                    ensure!(
                        core.schedules.remove(&id).is_some(),
                        "journal fires schedule {id:?}, which is not pending"
                    );
                }
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
                    if let WorldEventKind::System { event: text } = &event.kind
                        && let Some((participant, _)) = presence_change(text)
                    {
                        ensure!(
                            core.capabilities.contains_key(&participant),
                            "journal presence event names an unknown participant"
                        );
                    }
                    replay.push(event.clone());
                    core.record_admin_history(event);
                    core.since_checkpoint = core.since_checkpoint.saturating_add(1);
                }
                JournalEntry::Committed { event } => {
                    ensure!(
                        event.sequence == core.sequence.saturating_add(1),
                        "journal sequence jumps from {} to {}",
                        core.sequence,
                        event.sequence
                    );
                    match &event.kind {
                        WorldEventKind::ProgramInstalled {
                            module_hash,
                            seed,
                            provenance,
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
                            replay.clear();
                        }
                        WorldEventKind::Input(_) => replay.push(event.clone()),
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
                    snapshot,
                    events,
                    admin_history,
                    open_presence,
                } => {
                    ensure!(
                        core.sequence == 0 && program.is_none(),
                        "journal checkpoint does not come first"
                    );
                    ensure!(
                        events.last().is_none_or(|last| last.sequence == sequence)
                            && events
                                .windows(2)
                                .all(|pair| pair[1].sequence == pair[0].sequence + 1),
                        "journal checkpoint events are out of order"
                    );
                    ensure!(
                        events.iter().all(|event| {
                            event.sequence > 0
                                && !matches!(&event.kind, WorldEventKind::System { .. })
                        }),
                        "journal checkpoint contains a host system event"
                    );
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
                        .resolve(crate::world_limits::RuntimeLimits::default())
                        .context("invalid world execution profile in checkpoint")?;
                    core.execution_profile = execution_profile;
                    // Absent means the whole catalog: older checkpoints and
                    // default ceilings omit the field.
                    core.caps_ceiling = match caps_ceiling {
                        Some(ceiling) => {
                            crate::world_caps::validate_ceiling_names(&ceiling)
                                .context("invalid capability ceiling in checkpoint")?;
                            ceiling.into_iter().collect()
                        }
                        None => crate::world_caps::catalog_names(),
                    };
                    // The writer emits one `ScheduleSet` per pending schedule
                    // before the checkpoint, so the two must agree exactly.
                    let listed = schedules.unwrap_or_default();
                    let declared: BTreeMap<String, u64> = listed.iter().cloned().collect();
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
        // Sessions end with the process, and a crash skips their leave, so
        // anyone the journal still shows present is gone by now.
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
        core.sync_ledger_ceiling();
        Ok(RestoredWorld {
            core,
            program,
            replay,
        })
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
        // default is omitted and reads back as the whole catalog.
        let caps_ceiling: Option<Vec<String>> = (self.caps_ceiling
            != crate::world_caps::catalog_names())
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
        for (id, at_ms) in &self.schedules {
            entries.push(JournalEntry::ScheduleSet {
                id: id.clone(),
                at_ms: *at_ms,
            });
        }
        let schedules: Option<Vec<(String, u64)>> = (!self.schedules.is_empty()).then(|| {
            self.schedules
                .iter()
                .map(|(id, at_ms)| (id.clone(), *at_ms))
                .collect()
        });
        entries.push(JournalEntry::Checkpoint {
            sequence: self.sequence,
            module_hash,
            seed: self.active_seed,
            provenance: self.active_module_provenance.clone(),
            execution_profile: Some(self.execution_profile),
            caps_ceiling,
            schedules,
            snapshot,
            events: self.events.iter().cloned().collect(),
            admin_history: Some(JournalAdminHistory {
                events: self.admin_history.iter().cloned().collect(),
                complete: self.admin_history_complete,
            }),
            // `Some(empty)` means the writer knows no participants are open;
            // `None` is reserved for legacy checkpoints whose state is unknown.
            open_presence: Some(self.open_presence.iter().copied().collect()),
        });
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
        self.check_schedule_replacing(id, at_ms, now_ms, false)
    }

    /// Like [`Self::check_schedule`], but for a set that replaces the pending
    /// schedule with the same id: the entry being replaced counts as already
    /// cancelled, so it is neither a duplicate nor a use of the capacity.
    /// Callers validate with this BEFORE persisting the replacement's
    /// cancellation, so a refused replacement (say, past the horizon) leaves
    /// the existing schedule standing (#807).
    pub(crate) fn check_schedule_replacing(
        &self,
        id: &str,
        at_ms: u64,
        now_ms: u64,
        replacing: bool,
    ) -> Result<()> {
        crate::world_caps::validate_schedule_id(id)?;
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
        Ok(())
    }

    /// Journal entry for an already-issued capability.
    pub(crate) fn issued_entry(&self, participant_id: u64) -> Option<JournalEntry> {
        self.capabilities
            .get(&participant_id)
            .map(|record| JournalEntry::Issued {
                participant: record.participant.clone(),
                digest: hex_encode(&record.digest),
                scopes: record.scopes.0,
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

    /// The per-world resource requests, independent of current server ceilings.
    #[must_use]
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
        self.capabilities.get(&participant_id)?;
        let mut scopes = WorldScopes::ALL;
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
                return Some(WorldScopes(0));
            }
            current = record.parent;
        }
        Some(scopes)
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
            ensure!(
                record.bounds.expires_at.is_none_or(|expires| now < expires),
                "world capability has expired"
            );
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
        let now = self.clock().now_instant();
        self.chat_rates.retain(|_, sent| {
            while sent
                .front()
                .is_some_and(|at| now.duration_since(*at) >= CHAT_RATE_WINDOW)
            {
                sent.pop_front();
            }
            !sent.is_empty()
        });
        let current = self.chat_rates.get(&participant_id);
        ensure!(
            current.is_none_or(|sent| sent.len() < usize::from(CHAT_RATE_BURST)),
            "chat rate exceeded; try again later"
        );
        let event = WorldEvent {
            sequence: self
                .sequence
                .checked_add(1)
                .context("world event sequence exhausted")?,
            participant_id,
            kind: WorldEventKind::Chat(text),
        };
        self.chat_rates
            .entry(participant_id)
            .or_default()
            .push_back(now);
        Ok(event)
    }

    /// Commit an opaque game input after validating capability and bounds.
    pub fn input(&mut self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let event = self.prepare_input(token, input)?;
        self.commit_prepared(event)
    }

    pub(crate) fn prepare_input(&self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
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
        self.admin_history.push_back(event);
        while self.admin_history.len() > self.limits.retained_events.min(MAX_ADMIN_HISTORY_EVENTS)
            || admin_history_bytes(&self.admin_history) > MAX_ADMIN_HISTORY_BYTES
        {
            if self.admin_history.pop_front().is_none() {
                break;
            }
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
            .unwrap_or(WorldScopes(0))
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
            .context("malformed world capability")?;
        let participant_id = participant
            .parse::<u64>()
            .context("malformed world capability participant ID")?;
        let digest = token_digest(token.expose());
        let record = self
            .capabilities
            .get(&participant_id)
            .context("invalid, revoked, or insufficient world capability")?;
        ensure!(
            constant_time_eq(&digest, &record.digest),
            "invalid, revoked, or insufficient world capability"
        );
        ensure!(
            !record.revoked
                && self
                    .effective_scopes(participant_id)
                    .is_some_and(|scopes| scopes.contains(required)),
            "invalid, revoked, or insufficient world capability"
        );
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
