//! Transport-independent, authoritative in-memory world core.
//!
//! A transport should place one `WorldActor` behind its world ID and pass
//! messages to it serially. This module does not open sockets or persist world
//! state. Capability tokens are returned only at issuance and retained only as
//! SHA-256 digests for revocation and membership checks.

use std::collections::{HashMap, VecDeque};

use anyhow::{Context, Result, ensure};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::sync::{broadcast, mpsc, oneshot};

/// Bounded limits for one world.
#[derive(Clone, Copy, Debug)]
pub struct WorldLimits {
    /// Maximum number of active capabilities in this world.
    pub participants: usize,
    /// Maximum UTF-8 chat message length in bytes.
    pub chat_bytes: usize,
    /// Maximum opaque game input size in bytes.
    pub input_bytes: usize,
    /// Number of recent events kept for reconnect catch-up.
    pub retained_events: usize,
    /// Maximum presentation text returned by a world program, in bytes.
    pub presentation_bytes: usize,
}

impl Default for WorldLimits {
    fn default() -> Self {
        Self {
            participants: 256,
            chat_bytes: 2048,
            input_bytes: 16 * 1024,
            retained_events: 1024,
            presentation_bytes: 8192,
        }
    }
}

/// Permission scopes granted by a world capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldScopes(u8);

impl WorldScopes {
    /// Join the world and receive snapshots/events.
    pub const JOIN: Self = Self(0b001);
    /// Send chat messages.
    pub const CHAT: Self = Self(0b010);
    /// Submit game input events.
    pub const INPUT: Self = Self(0b100);
    /// Default guest permissions.
    pub const GUEST: Self = Self(Self::JOIN.0 | Self::CHAT.0 | Self::INPUT.0);

    /// Raw scope bits, as journaled.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether this scope set includes `required`.
    #[must_use]
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

/// Bearer secret that grants access to one world. Never serialize it into a snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinCapability(String);

impl std::fmt::Debug for JoinCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JoinCapability([REDACTED])")
    }
}

impl JoinCapability {
    /// Construct from an untrusted wire token; it is authorized only by its
    /// owning [`WorldCore`] or [`WorldHandle`].
    #[must_use]
    pub const fn from_wire(token: String) -> Self {
        Self(token)
    }

    /// Expose the token for a deliberate invitation/transport response.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Public participant summary; it contains no capability material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Opaque server-assigned participant identifier.
    pub id: u64,
    /// Caller-provided display name, sanitized and bounded at issue time.
    pub display_name: String,
}

/// Versioned event committed by the world authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldEvent {
    /// Monotonic event sequence, starting at one.
    pub sequence: u64,
    /// Server-assigned participant that caused the event.
    pub participant_id: u64,
    /// Event payload.
    pub kind: WorldEventKind,
}

/// Participant-originated event data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum WorldEventKind {
    /// Plain text message, UTF-8 and control-character sanitized.
    Chat(String),
    /// Opaque input for a future game/world module.
    Input(Vec<u8>),
    /// A world admin installed a new program. `participant_id` is zero for
    /// this host-authored event.
    ProgramInstalled {
        /// Iroh BLAKE3 hash of the newly active module artifact.
        module_hash: String,
        /// Seed passed to the program's `init`; with the module and the
        /// inputs after this event it reproduces the program state exactly.
        #[serde(default)]
        seed: u64,
    },
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
    },
    /// A capability was revoked.
    Revoked {
        /// Participant whose capability was revoked.
        participant_id: u64,
    },
    /// A sequenced event was committed.
    Committed {
        /// The event.
        event: WorldEvent,
    },
}

/// Current [`JournalEntry::Created`] version.
pub const JOURNAL_VERSION: u32 = 1;

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
}

/// A world rebuilt from its journal, before its program is re-instantiated.
#[derive(Debug)]
pub struct RestoredWorld {
    /// Participants, sequence and retained events as of the last entry.
    pub core: WorldCore,
    /// `(module_hash, seed)` of the last installed program, if any.
    pub program: Option<(String, u64)>,
    /// Inputs committed after that install, in order, to replay through it.
    pub replay: Vec<WorldEvent>,
}

/// Behavior supplied by the running world program.
///
/// Implementations execute inside the actor's serial command loop. They must
/// not perform network or filesystem I/O. An error from `update` rejects the
/// candidate input before it is committed to the event log. The Wasmtime
/// implementation poisons its guest instance on traps.
pub trait WorldProgram: Send + 'static {
    /// Apply an opaque participant input. Chat is managed by the host and is
    /// not passed to the game program.
    fn update(&mut self, _event: &WorldEvent) -> Result<()> {
        Ok(())
    }

    /// Return this participant's presentation, or `None` for a chat-only
    /// world. The presentation must be UTF-8 and fit `presentation_bytes`.
    fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
        Ok(None)
    }
}

#[derive(Debug, Default)]
struct EmptyWorldProgram;

impl WorldProgram for EmptyWorldProgram {}

/// Snapshot returned on join or reconnect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldSnapshot {
    /// Stable world identifier.
    pub world_id: String,
    /// Content hash of the active program, if this world executes one.
    pub active_module_hash: Option<String>,
    /// Latest committed sequence at snapshot time.
    pub current_sequence: u64,
    /// Earliest retained event sequence, or current sequence + 1 when empty.
    pub oldest_retained_sequence: u64,
    /// Recent event log, bounded by [`WorldLimits::retained_events`].
    pub events: Vec<WorldEvent>,
    /// Public participant list.
    pub participants: Vec<Participant>,
}

#[derive(Clone, Debug)]
struct CapabilityRecord {
    digest: [u8; 32],
    participant: Participant,
    scopes: WorldScopes,
    revoked: bool,
}

/// Mutable world state. Keep it behind one actor/mutex so event sequencing is serialized.
#[derive(Debug)]
pub struct WorldCore {
    world_id: String,
    limits: WorldLimits,
    next_participant_id: u64,
    sequence: u64,
    active_module_hash: Option<String>,
    capabilities: HashMap<u64, CapabilityRecord>,
    events: VecDeque<WorldEvent>,
}

impl WorldCore {
    /// Create an empty in-memory world.
    pub fn new(world_id: impl Into<String>, limits: WorldLimits) -> Result<Self> {
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
        Ok(Self {
            world_id,
            limits,
            next_participant_id: 1,
            sequence: 0,
            active_module_hash: None,
            capabilities: HashMap::new(),
            events: VecDeque::new(),
        })
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
        match entries.next() {
            Some(JournalEntry::Created { world_id, version }) => {
                ensure!(
                    world_id == core.world_id,
                    "journal belongs to world {world_id:?}, not {:?}",
                    core.world_id
                );
                ensure!(
                    version == JOURNAL_VERSION,
                    "unsupported world journal version {version}"
                );
            }
            Some(_) => anyhow::bail!("world journal does not start with a header"),
            None => {
                return Ok(RestoredWorld {
                    core,
                    program,
                    replay,
                });
            }
        }
        for entry in entries {
            match entry {
                JournalEntry::Created { .. } => anyhow::bail!("duplicate world journal header"),
                JournalEntry::Issued {
                    participant,
                    digest,
                    scopes,
                } => {
                    let digest = hex_decode_32(&digest).context("malformed capability digest")?;
                    ensure!(
                        !core.capabilities.contains_key(&participant.id),
                        "participant {} issued twice",
                        participant.id
                    );
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
                            revoked: false,
                        },
                    );
                }
                JournalEntry::Revoked { participant_id } => {
                    ensure!(
                        core.revoke(participant_id),
                        "journal revokes unknown or revoked participant {participant_id}"
                    );
                }
                JournalEntry::Committed { event } => {
                    ensure!(
                        event.sequence == core.sequence.saturating_add(1),
                        "journal sequence jumps from {} to {}",
                        core.sequence,
                        event.sequence
                    );
                    match &event.kind {
                        WorldEventKind::ProgramInstalled { module_hash, seed } => {
                            core.active_module_hash = Some(module_hash.clone());
                            program = Some((module_hash.clone(), *seed));
                            replay.clear();
                        }
                        WorldEventKind::Input(_) => replay.push(event.clone()),
                        WorldEventKind::Chat(_) => {}
                    }
                    core.commit_prepared(event)?;
                }
            }
        }
        Ok(RestoredWorld {
            core,
            program,
            replay,
        })
    }

    /// Journal entry for an already-issued capability.
    fn issued_entry(&self, participant_id: u64) -> Option<JournalEntry> {
        self.capabilities
            .get(&participant_id)
            .map(|record| JournalEntry::Issued {
                participant: record.participant.clone(),
                digest: hex_encode(&record.digest),
                scopes: record.scopes.0,
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

    /// Issue a capability for a participant. Only `JOIN`-scoped capabilities
    /// count toward active participant capacity.
    pub fn issue_capability(
        &mut self,
        display_name: &str,
        scopes: WorldScopes,
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
                revoked: false,
            },
        );
        Ok((participant, JoinCapability(token)))
    }

    /// Revoke a capability by its server-assigned participant ID.
    pub fn revoke(&mut self, participant_id: u64) -> bool {
        let Some(record) = self.capabilities.get_mut(&participant_id) else {
            return false;
        };
        let was_active = !record.revoked;
        record.revoked = true;
        was_active
    }

    /// Commit a chat message after validating capability and bounds.
    pub fn chat(&mut self, token: &JoinCapability, text: &str) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::CHAT)?;
        ensure!(
            text.len() <= self.limits.chat_bytes,
            "chat message exceeds {} bytes",
            self.limits.chat_bytes
        );
        let text = sanitize_chat(text);
        ensure!(!text.is_empty(), "chat message must not be empty");
        self.commit(participant_id, WorldEventKind::Chat(text))
    }

    /// Commit an opaque game input after validating capability and bounds.
    pub fn input(&mut self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let event = self.prepare_input(token, input)?;
        self.commit_prepared(event)
    }

    fn prepare_input(&self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
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

    fn commit_prepared(&mut self, event: WorldEvent) -> Result<WorldEvent> {
        ensure!(
            event.sequence == self.sequence.saturating_add(1),
            "prepared world event is stale"
        );
        self.sequence = event.sequence;
        self.events.push_back(event.clone());
        while self.events.len() > self.limits.retained_events {
            self.events.pop_front();
        }
        Ok(event)
    }

    /// Return the current bounded snapshot after validating join scope.
    pub fn snapshot(&self, token: &JoinCapability) -> Result<WorldSnapshot> {
        self.authorize(token, WorldScopes::JOIN)?;
        Ok(self.snapshot_unchecked())
    }

    fn viewer(&self, token: &JoinCapability) -> Result<Participant> {
        let id = self.authorize(token, WorldScopes::JOIN)?;
        self.capabilities
            .get(&id)
            .map(|record| record.participant.clone())
            .context("world participant is unavailable")
    }

    /// Return retained events newer than `after_sequence`, plus the current
    /// cursor. If the cursor fell behind retention, the caller must resnapshot.
    pub fn events_after(&self, token: &JoinCapability, after_sequence: u64) -> Result<EventPage> {
        self.authorize(token, WorldScopes::JOIN)?;
        let oldest = self.oldest_sequence();
        let needs_snapshot = after_sequence.saturating_add(1) < oldest;
        let events = if needs_snapshot {
            Vec::new()
        } else {
            self.events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .cloned()
                .collect()
        };
        Ok(EventPage {
            current_sequence: self.sequence,
            oldest_retained_sequence: oldest,
            needs_snapshot,
            events,
        })
    }

    fn authorize(&self, token: &JoinCapability, required: WorldScopes) -> Result<u64> {
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
            !record.revoked && record.scopes.contains(required),
            "invalid, revoked, or insufficient world capability"
        );
        Ok(participant_id)
    }

    fn commit(&mut self, participant_id: u64, kind: WorldEventKind) -> Result<WorldEvent> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        let event = WorldEvent {
            sequence: self.sequence,
            participant_id,
            kind,
        };
        self.events.push_back(event.clone());
        while self.events.len() > self.limits.retained_events {
            self.events.pop_front();
        }
        Ok(event)
    }

    fn oldest_sequence(&self) -> u64 {
        self.events
            .front()
            .map_or_else(|| self.sequence.saturating_add(1), |event| event.sequence)
    }

    fn snapshot_unchecked(&self) -> WorldSnapshot {
        let participants = self
            .capabilities
            .values()
            .filter(|record| !record.revoked && record.scopes.contains(WorldScopes::JOIN))
            .map(|record| record.participant.clone())
            .collect();
        WorldSnapshot {
            world_id: self.world_id.clone(),
            active_module_hash: self.active_module_hash.clone(),
            current_sequence: self.sequence,
            oldest_retained_sequence: self.oldest_sequence(),
            events: self.events.iter().cloned().collect(),
            participants,
        }
    }
}

/// Cloneable, serialized access to a world core. Every request passes through
/// one actor task, which defines the authoritative event order.
#[derive(Clone, Debug)]
pub struct WorldHandle {
    commands: mpsc::Sender<WorldCommand>,
    events: broadcast::Sender<WorldEvent>,
    revocations: broadcast::Sender<u64>,
}

enum WorldCommand {
    Issue {
        name: String,
        scopes: WorldScopes,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
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
    View {
        token: JoinCapability,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    InstallProgram {
        module_hash: String,
        seed: u64,
        program: Box<dyn WorldProgram>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
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
        program: Box<dyn WorldProgram>,
        journal: Option<Box<dyn WorldJournal>>,
    ) -> Self {
        let (commands, mut receiver) = mpsc::channel(256);
        let (events, _) = broadcast::channel(256);
        let (revocations, _) = broadcast::channel(256);
        let event_sender = events.clone();
        let revocation_sender = revocations.clone();
        tokio::spawn(async move {
            let mut core = core;
            let mut program = program;
            let mut program_healthy = true;
            let mut journal = journal;
            let mut storage_failed = false;
            // Durably record `entry`, or turn the world read-only.
            let mut persist = |entry: JournalEntry, storage_failed: &mut bool| -> Result<()> {
                ensure!(
                    !*storage_failed,
                    "world storage failed; the world is read-only"
                );
                if let Some(journal) = journal.as_mut()
                    && let Err(error) = journal.append(&entry)
                {
                    *storage_failed = true;
                    tracing::error!("world journal append failed: {error:#}");
                    return Err(error.context("world storage failed; the world is read-only"));
                }
                Ok(())
            };
            while let Some(command) = receiver.recv().await {
                match command {
                    WorldCommand::Issue {
                        name,
                        scopes,
                        reply,
                    } => {
                        let result = if storage_failed {
                            Err(anyhow::anyhow!(
                                "world storage failed; the world is read-only"
                            ))
                        } else {
                            core.issue_capability(&name, scopes).and_then(|issued| {
                                let entry = core
                                    .issued_entry(issued.0.id)
                                    .context("issued capability vanished")?;
                                persist(entry, &mut storage_failed)?;
                                Ok(issued)
                            })
                        };
                        let _ = reply.send(result);
                    }
                    WorldCommand::Revoke {
                        participant_id,
                        reply,
                    } => {
                        // Revocation takes effect in memory even if storage
                        // failed: denying access is always the safe side.
                        let was_active = core.revoke(participant_id);
                        if was_active {
                            let _ = persist(
                                JournalEntry::Revoked { participant_id },
                                &mut storage_failed,
                            );
                            let _ = revocation_sender.send(participant_id);
                        }
                        let _ = reply.send(was_active);
                    }
                    WorldCommand::ParticipantId { token, reply } => {
                        let _ = reply.send(core.authorize(&token, WorldScopes::JOIN));
                    }
                    WorldCommand::Chat { token, text, reply } => {
                        let result = if storage_failed {
                            Err(anyhow::anyhow!(
                                "world storage failed; the world is read-only"
                            ))
                        } else {
                            core.chat(&token, &text).and_then(|event| {
                                persist(
                                    JournalEntry::Committed {
                                        event: event.clone(),
                                    },
                                    &mut storage_failed,
                                )?;
                                Ok(event)
                            })
                        };
                        if let Ok(event) = &result {
                            let _ = event_sender.send(event.clone());
                        }
                        let _ = reply.send(result);
                    }
                    WorldCommand::Input {
                        token,
                        input,
                        reply,
                    } => {
                        let result = match core.prepare_input(&token, &input) {
                            Ok(_) if storage_failed => Err(anyhow::anyhow!(
                                "world storage failed; the world is read-only"
                            )),
                            Ok(candidate) if program_healthy => match program.update(&candidate) {
                                Ok(()) => persist(
                                    JournalEntry::Committed {
                                        event: candidate.clone(),
                                    },
                                    &mut storage_failed,
                                )
                                .and_then(|()| core.commit_prepared(candidate)),
                                Err(error) => {
                                    program_healthy = false;
                                    Err(error.context("world program rejected input"))
                                }
                            },
                            Ok(_) => Err(anyhow::anyhow!(
                                "world program is unavailable after a previous failure"
                            )),
                            Err(error) => Err(error),
                        };
                        if let Ok(event) = &result {
                            let _ = event_sender.send(event.clone());
                        }
                        let _ = reply.send(result);
                    }
                    WorldCommand::Snapshot { token, reply } => {
                        let _ = reply.send(core.snapshot(&token));
                    }
                    WorldCommand::EventsAfter {
                        token,
                        sequence,
                        reply,
                    } => {
                        let _ = reply.send(core.events_after(&token, sequence));
                    }
                    WorldCommand::View { token, reply } => {
                        let result = match core.viewer(&token) {
                            Ok(viewer) if program_healthy => match program.view(&viewer) {
                                Ok(view) => {
                                    if let Some(text) = &view {
                                        if text.len() > core.limits.presentation_bytes {
                                            program_healthy = false;
                                            Err(anyhow::anyhow!(
                                                "world presentation exceeds {} bytes",
                                                core.limits.presentation_bytes
                                            ))
                                        } else {
                                            Ok(view)
                                        }
                                    } else {
                                        Ok(None)
                                    }
                                }
                                Err(error) => {
                                    program_healthy = false;
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
                    WorldCommand::InstallProgram {
                        module_hash,
                        seed,
                        program: replacement,
                        reply,
                    } => {
                        // Actor serialization makes activation and its event
                        // atomic with respect to other world commands.
                        let candidate = WorldEvent {
                            sequence: core.sequence.saturating_add(1),
                            participant_id: 0,
                            kind: WorldEventKind::ProgramInstalled { module_hash, seed },
                        };
                        let result = persist(
                            JournalEntry::Committed {
                                event: candidate.clone(),
                            },
                            &mut storage_failed,
                        )
                        .and_then(|()| core.commit_prepared(candidate));
                        let result = match result {
                            Ok(event) => {
                                if let WorldEventKind::ProgramInstalled { module_hash, .. } =
                                    &event.kind
                                {
                                    core.active_module_hash = Some(module_hash.clone());
                                }
                                program = replacement;
                                program_healthy = true;
                                let _ = event_sender.send(event.clone());
                                Ok(event)
                            }
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }
                    WorldCommand::Shutdown { reply } => {
                        let _ = reply.send(());
                        break;
                    }
                }
            }
        });
        Self {
            commands,
            events,
            revocations,
        }
    }

    /// Subscribe to newly committed world events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<WorldEvent> {
        self.events.subscribe()
    }

    /// Subscribe to capability revocations.
    #[must_use]
    pub fn subscribe_revocations(&self) -> broadcast::Receiver<u64> {
        self.revocations.subscribe()
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
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::Issue {
                name: name.into(),
                scopes,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("world actor is closed"))?;
        response
            .await
            .context("world actor dropped capability response")?
    }

    /// Revoke an active capability.
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

    /// Atomically replace the active program and sequence a host-authored
    /// module-installed event. The new program is used after this command.
    pub async fn install_program(
        &self,
        module_hash: String,
        seed: u64,
        program: Box<dyn WorldProgram>,
    ) -> Result<WorldEvent> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(WorldCommand::InstallProgram {
                module_hash,
                seed,
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

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
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
mod tests {
    use super::*;

    fn world(retained_events: usize) -> WorldCore {
        WorldCore::new(
            "test-world",
            WorldLimits {
                retained_events,
                ..WorldLimits::default()
            },
        )
        .unwrap()
    }

    fn join(core: &mut WorldCore, name: &str) -> (Participant, JoinCapability) {
        core.issue_capability(name, WorldScopes::GUEST).unwrap()
    }

    #[test]
    fn capability_scopes_isolate_actions() {
        let mut core = world(8);
        let (_, token) = join(&mut core, "Guest");
        assert!(core.snapshot(&token).is_ok());
        assert!(core.chat(&token, "hello").is_ok());
        assert!(core.input(&token, b"move").is_ok());
        let (_, join_only) = core
            .issue_capability("spectator", WorldScopes::JOIN)
            .unwrap();
        assert!(core.snapshot(&join_only).is_ok());
        assert!(core.chat(&join_only, "no").is_err());
        assert!(core.input(&join_only, b"no").is_err());
    }

    #[test]
    fn events_are_sequenced_and_reconnect_can_catch_up() {
        let mut core = world(8);
        let (_, token) = join(&mut core, "Ada");
        assert_eq!(core.chat(&token, "hello").unwrap().sequence, 1);
        assert_eq!(core.input(&token, b"move").unwrap().sequence, 2);
        let page = core.events_after(&token, 1).unwrap();
        assert_eq!(page.current_sequence, 2);
        assert!(!page.needs_snapshot);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].sequence, 2);
    }

    #[test]
    fn reconnect_requires_snapshot_when_cursor_is_older_than_retention() {
        let mut core = world(2);
        let (_, token) = join(&mut core, "Ada");
        for msg in ["one", "two", "three"] {
            core.chat(&token, msg).unwrap();
        }
        let page = core.events_after(&token, 0).unwrap();
        assert!(page.needs_snapshot);
        assert!(page.events.is_empty());
        let snapshot = core.snapshot(&token).unwrap();
        assert_eq!(snapshot.current_sequence, 3);
        assert_eq!(snapshot.oldest_retained_sequence, 2);
        assert_eq!(snapshot.events.len(), 2);
    }

    #[test]
    fn revoked_capabilities_cannot_read_or_write() {
        let mut core = world(8);
        let (participant, token) = join(&mut core, "Ada");
        assert!(core.revoke(participant.id));
        assert!(!core.revoke(participant.id));
        assert!(core.snapshot(&token).is_err());
        assert!(core.chat(&token, "hello").is_err());
    }

    #[test]
    fn foreign_and_malformed_capabilities_are_rejected() {
        let mut left = world(8);
        let right = world(8);
        let (_, left_token) = join(&mut left, "Ada");
        let malformed = JoinCapability("not-a-valid-token".to_owned());
        assert!(left.snapshot(&malformed).is_err());
        assert!(right.snapshot(&left_token).is_err());
    }

    #[test]
    fn names_chat_and_input_are_sanitized_and_bounded() {
        let mut core = world(8);
        let (participant, token) = join(&mut core, "\u{1b}[31mAda\n");
        assert_eq!(participant.display_name, "[31mAda");
        assert_eq!(
            core.chat(&token, " hi\u{1b}[31m\n").unwrap().kind,
            WorldEventKind::Chat("hi[31m".to_owned())
        );
        assert!(core.chat(&token, &"x".repeat(2049)).is_err());
        assert!(core.input(&token, &vec![0; 16 * 1024 + 1]).is_err());
        assert!(core.chat(&token, " \n \t ").is_err());
    }

    #[test]
    fn participant_capacity_and_constructor_limits_are_enforced() {
        let mut core = WorldCore::new(
            "small",
            WorldLimits {
                participants: 1,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        join(&mut core, "Ada");
        assert!(core.issue_capability("Grace", WorldScopes::GUEST).is_err());
        assert!(WorldCore::new("", WorldLimits::default()).is_err());
        assert!(
            WorldCore::new(
                "world",
                WorldLimits {
                    retained_events: 0,
                    ..WorldLimits::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn snapshots_do_not_include_capability_secret() {
        let mut core = world(8);
        let (participant, token) = join(&mut core, "Ada");
        let serialized = format!("{:?}", core.snapshot(&token).unwrap());
        assert!(!serialized.contains(token.expose()));
        assert!(serialized.contains(&participant.display_name));
        assert!(!format!("{token:?}").contains(token.expose()));
    }

    #[tokio::test]
    async fn actor_serializes_concurrent_event_mutations() {
        let actor = WorldHandle::spawn(world(32));
        let (_, token) = actor.issue("Ada", WorldScopes::GUEST).await.unwrap();
        let first = actor.chat(token.clone(), "one");
        let second = actor.input(token.clone(), b"two".to_vec());
        let third = actor.chat(token.clone(), "three");
        let (one, two, three) = tokio::join!(first, second, third);
        let mut sequences = [
            one.unwrap().sequence,
            two.unwrap().sequence,
            three.unwrap().sequence,
        ];
        sequences.sort_unstable();
        assert_eq!(sequences, [1, 2, 3]);
        assert_eq!(actor.snapshot(token).await.unwrap().current_sequence, 3);
    }
}
