//! Transport-independent, authoritative in-memory world core.
//!
//! A transport should place one `WorldActor` behind its world ID and pass
//! messages to it serially. This module does not open sockets or persist world
//! state. Capability tokens are returned only at issuance and retained only as
//! SHA-256 digests for revocation and membership checks.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use crate::world_caps::{CapError, CapsReport, Decision, Request, Subscription};
use anyhow::{Context, Result, bail, ensure};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

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
    /// Largest snapshot frame, in bytes. A snapshot carries only the most
    /// recent events that fit; a client rebases its cursor on the snapshot,
    /// it does not need every retained event.
    pub snapshot_bytes: usize,
    /// Trim the journal behind a program snapshot once this many events were
    /// committed since the last one. `0` disables checkpointing.
    pub checkpoint_every: u64,
    /// Slowest tick a program may subscribe to, in milliseconds.
    pub min_tick_ms: u64,
    /// Whether a world's policy grants a capability on its first use.
    pub grant_on_use: bool,
}

impl Default for WorldLimits {
    fn default() -> Self {
        Self {
            participants: 256,
            chat_bytes: 2048,
            input_bytes: 16 * 1024,
            retained_events: 1024,
            presentation_bytes: 8192,
            snapshot_bytes: 512 * 1024,
            checkpoint_every: 1000,
            min_tick_ms: 1000,
            grant_on_use: false,
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
    /// An event the host originated for the program: capability results,
    /// ticks, presence. Delivered with `participant_id` 0; never sequenced,
    /// never broadcast to sessions.
    System {
        /// The event's JSON.
        event: String,
    },
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
    /// A host event delivered to the program (capability result, tick,
    /// presence). Delivered with sequence 0; the program saw it, so replay
    /// must too.
    System {
        /// The event as it was delivered.
        event: WorldEvent,
    },
    /// The world's granted capability set, written whole on every change.
    CapsGranted {
        /// Capability names.
        granted: Vec<String>,
    },
    /// The world as of `sequence`, standing in for every event up to it. Only
    /// ever written by compaction, directly after the participant entries.
    Checkpoint {
        /// Sequence of the last event the checkpoint covers.
        sequence: u64,
        /// Active program.
        module_hash: String,
        /// Seed the program was initialized with.
        seed: u64,
        /// The program's own serialization of its state.
        snapshot: String,
        /// Recent events kept for reconnect catch-up.
        events: Vec<WorldEvent>,
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

    /// Atomically replace the whole journal with `entries` (a header, the
    /// participants and a [`JournalEntry::Checkpoint`]). Either the old or
    /// the new journal survives a crash, never a mixture.
    ///
    /// # Errors
    ///
    /// Journals that cannot compact (the default), or any I/O failure. After
    /// an error the journal must still be a complete record of the world.
    fn compact(&mut self, _entries: &[JournalEntry]) -> Result<()> {
        bail!("this journal cannot be compacted")
    }
}

/// The program a journal says was running, and where to resume it from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoredProgram {
    /// Content hash of the module.
    pub module_hash: String,
    /// Seed it was initialized with.
    pub seed: u64,
    /// State to restore before replaying, if the journal was compacted behind
    /// a checkpoint; otherwise the program starts from `init(seed)`.
    pub snapshot: Option<String>,
}

/// A world rebuilt from its journal, before its program is re-instantiated.
#[derive(Debug)]
pub struct RestoredWorld {
    /// Participants, sequence and retained events as of the last entry.
    pub core: WorldCore,
    /// The last installed program, if any.
    pub program: Option<RestoredProgram>,
    /// Inputs committed after the install or checkpoint, in order, to replay
    /// through it.
    pub replay: Vec<WorldEvent>,
}

/// Structured records a program publishes: key to JSON value, key-ordered.
pub type Records = BTreeMap<String, serde_json::Value>;

/// Most records one world may publish.
pub const MAX_RECORDS: usize = 4096;
/// Longest record key, in bytes.
pub const MAX_RECORD_KEY_BYTES: usize = 256;
/// Largest single record value (serialized JSON), in bytes.
pub const MAX_RECORD_VALUE_BYTES: usize = 8 * 1024;

/// Parse and bound a program's records output. Keys must be non-empty,
/// at most [`MAX_RECORD_KEY_BYTES`], and free of control characters (the
/// storage layer uses `\0` as a key terminator).
///
/// # Errors
///
/// Anything that is not a bounded JSON object of valid keys.
pub fn parse_records(text: &str) -> Result<Records> {
    let records: Records =
        serde_json::from_str(text).context("world records must be a JSON object")?;
    ensure!(
        records.len() <= MAX_RECORDS,
        "world publishes {} records; the limit is {MAX_RECORDS}",
        records.len()
    );
    for (key, value) in &records {
        ensure!(
            !key.is_empty() && key.len() <= MAX_RECORD_KEY_BYTES,
            "record key must be 1..={MAX_RECORD_KEY_BYTES} bytes"
        );
        ensure!(
            !key.chars().any(char::is_control),
            "record key {key:?} contains a control character"
        );
        let size = serde_json::to_vec(value)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        ensure!(
            size <= MAX_RECORD_VALUE_BYTES,
            "record {key:?} is {size} bytes; the limit is {MAX_RECORD_VALUE_BYTES}"
        );
    }
    Ok(records)
}

/// Records of `program`, validated; `Ok(None)` when it keeps none.
fn program_records(program: &mut dyn WorldProgram) -> Result<Option<Records>> {
    program
        .records()?
        .map(|text| parse_records(&text))
        .transpose()
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

    /// Structured records projected from the program state: a JSON object
    /// mapping record keys to JSON values, or `None` if the program keeps no
    /// records. Must be a pure function of state (it is recomputed on replay).
    fn records(&mut self) -> Result<Option<String>> {
        Ok(None)
    }

    /// The program's state as text that [`crate::world_store`] can restore it
    /// from, or `None` if it cannot snapshot (its journal then keeps growing).
    /// An implementation must only return a snapshot it has verified restores
    /// faithfully: the journal is trimmed behind it.
    fn snapshot(&mut self) -> Result<Option<String>> {
        Ok(None)
    }

    /// What the program wants from the server right now (see
    /// [`crate::world_caps`]), or `None` if it makes no use of capabilities.
    /// Must be a pure function of state: the host re-reads it after every
    /// change and after a restart.
    fn wants(&mut self) -> Result<Option<String>> {
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
    /// Earliest event sequence present in this snapshot, or the current
    /// sequence + 1 when it carries no events.
    pub oldest_retained_sequence: u64,
    /// Recent events, bounded by [`WorldLimits::retained_events`] and
    /// [`WorldLimits::snapshot_bytes`] (the newest that fit).
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
    active_seed: u64,
    /// What this world's program may ask the server for.
    pub(crate) ledger: crate::world_caps::CapLedger,
    /// Events committed since the journal last started from a checkpoint.
    since_checkpoint: u64,
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
        ensure!(
            limits.snapshot_bytes > 0,
            "world snapshot limit must be nonzero"
        );
        Ok(Self {
            world_id,
            limits,
            next_participant_id: 1,
            sequence: 0,
            active_module_hash: None,
            active_seed: 0,
            ledger: crate::world_caps::CapLedger::new(Default::default(), limits.grant_on_use),
            since_checkpoint: 0,
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
                JournalEntry::CapsGranted { granted } => {
                    for name in &granted {
                        ensure!(
                            name == "*" || crate::world_caps::is_known(name),
                            "journal grants unknown capability {name:?}"
                        );
                    }
                    core.ledger.set_granted(granted.iter().cloned().collect());
                }
                JournalEntry::System { event } => {
                    ensure!(
                        event.sequence == 0 && event.participant_id == 0,
                        "journal system event claims a sequence"
                    );
                    replay.push(event.clone());
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
                        WorldEventKind::ProgramInstalled { module_hash, seed } => {
                            core.active_module_hash = Some(module_hash.clone());
                            core.active_seed = *seed;
                            program = Some(RestoredProgram {
                                module_hash: module_hash.clone(),
                                seed: *seed,
                                snapshot: None,
                            });
                            replay.clear();
                        }
                        WorldEventKind::Input(_) => replay.push(event.clone()),
                        // System events belong to the journal's tail; a
                        // checkpoint behind them already contains their effect.
                        WorldEventKind::System { .. } | WorldEventKind::Chat(_) => {}
                    }
                    core.commit_prepared(event)?;
                }
                JournalEntry::Checkpoint {
                    sequence,
                    module_hash,
                    seed,
                    snapshot,
                    events,
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
                    core.sequence = sequence;
                    core.events = events.into();
                    while core.events.len() > core.limits.retained_events {
                        core.events.pop_front();
                    }
                    core.active_module_hash = Some(module_hash.clone());
                    core.active_seed = seed;
                    program = Some(RestoredProgram {
                        module_hash,
                        seed,
                        snapshot: Some(snapshot),
                    });
                }
            }
        }
        Ok(RestoredWorld {
            core,
            program,
            replay,
        })
    }

    /// Whether enough events piled up behind the last checkpoint to trim.
    fn checkpoint_due(&self) -> bool {
        self.limits.checkpoint_every > 0
            && self.active_module_hash.is_some()
            && self.since_checkpoint >= self.limits.checkpoint_every
    }

    /// The journal that replaces the current one behind `snapshot`: a header,
    /// every participant (and revocation), then the checkpoint itself.
    fn checkpoint_entries(&self, snapshot: String) -> Result<Vec<JournalEntry>> {
        let module_hash = self
            .active_module_hash
            .clone()
            .context("no active program to checkpoint")?;
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
                entries.push(JournalEntry::Revoked { participant_id: id });
            }
        }
        let granted: Vec<String> = self.ledger.granted().iter().cloned().collect();
        if !granted.is_empty() {
            entries.push(JournalEntry::CapsGranted { granted });
        }
        entries.push(JournalEntry::Checkpoint {
            sequence: self.sequence,
            module_hash,
            seed: self.active_seed,
            snapshot,
            events: self.events.iter().cloned().collect(),
        });
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
    pub(crate) fn system_chat(&mut self, text: &str) -> Result<WorldEvent> {
        ensure!(
            text.len() <= self.limits.chat_bytes,
            "chat message exceeds {} bytes",
            self.limits.chat_bytes
        );
        let text = sanitize_chat(text);
        ensure!(!text.is_empty(), "chat message must not be empty");
        self.commit(0, WorldEventKind::Chat(text))
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
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
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
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
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
        // Take the most recent events that fit the snapshot budget, newest
        // first. A snapshot is a baseline, not a complete replay: clients
        // adopt its sequence as their cursor.
        let mut budget = self.limits.snapshot_bytes;
        let mut events: VecDeque<WorldEvent> = VecDeque::new();
        for event in self.events.iter().rev() {
            let size = serde_json::to_vec(event).map_or(budget, |encoded| encoded.len());
            if size > budget {
                break;
            }
            budget -= size;
            events.push_front(event.clone());
        }
        let oldest_retained_sequence = events
            .front()
            .map_or_else(|| self.sequence.saturating_add(1), |event| event.sequence);
        WorldSnapshot {
            world_id: self.world_id.clone(),
            active_module_hash: self.active_module_hash.clone(),
            current_sequence: self.sequence,
            oldest_retained_sequence,
            events: events.into_iter().collect(),
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
    records: watch::Receiver<(u64, Arc<Records>)>,
    view_changed: watch::Receiver<u64>,
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
    /// Admin: read the capability report.
    CapsInfo {
        reply: oneshot::Sender<Result<CapsReport>>,
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
}

impl WorldActor {
    async fn run(mut self, mut receiver: mpsc::Receiver<WorldCommand>) {
        self.after_change().await;
        loop {
            // Ticks sleep until their earliest deadline; with none armed this
            // branch never wakes.
            let deadline = self.ticks.values().next().copied();
            let command = tokio::select! {
                command = receiver.recv() => match command {
                    Some(command) => command,
                    None => break,
                },
                _ = Self::sleep_until(deadline) => {
                    self.fire_ticks().await;
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
        }
    }

    async fn sleep_until(deadline: Option<std::time::Instant>) {
        match deadline {
            Some(deadline) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await
            }
            None => std::future::pending().await,
        }
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
    async fn deliver(&mut self, text: String) {
        if !self.program_healthy || self.storage_failed {
            return;
        }
        if text.len() > crate::world_caps::MAX_EVENT_BYTES {
            tracing::error!("world event exceeds the delivery bound; dropped");
            return;
        }
        let event = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System { event: text },
        };
        if let Err(error) = self.program.update(&event) {
            self.program_healthy = false;
            tracing::error!("world program rejected a host event: {error:#}");
            return;
        }
        match program_records(self.program.as_mut()) {
            Ok(records) => {
                let records = records.unwrap_or_default();
                publish_records(&self.records_sender, self.core.sequence, records);
            }
            Err(error) => {
                self.program_healthy = false;
                tracing::error!("world program records failed: {error:#}");
                return;
            }
        }
        if self
            .persist(JournalEntry::System {
                event: event.clone(),
            })
            .is_ok()
        {
            self.core.since_checkpoint = self.core.since_checkpoint.saturating_add(1);
        }
        let _ = self.view_changed.send(self.core.sequence);
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

    /// Execute one capability request. Anything the world has not granted is
    /// an error the program sees.
    fn execute(&mut self, request: &Request) -> Result<serde_json::Value, CapError> {
        match self.core.ledger.decide(&request.cap) {
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
        if now_present && !was_present && !self.ticks.is_empty() == false {
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
            let event = serde_json::json!({
                "cap": "players",
                "event": if now_present { "joined" } else { "left" },
                "participant": {"id": participant, "name": name},
            });
            self.deliver(event.to_string()).await;
        }
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
        let missing: Vec<String> = requested
            .iter()
            .filter(|cap| !granted.contains(cap) && !granted.contains(&"*".to_owned()))
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
            },
            catalog: crate::world_caps::CATALOG.to_vec(),
        })
    }

    async fn handle(&mut self, command: WorldCommand) {
        match command {
            WorldCommand::Issue {
                name,
                scopes,
                reply,
            } => {
                let result = if self.storage_failed {
                    Err(anyhow::anyhow!(
                        "world storage failed; the world is read-only"
                    ))
                } else {
                    self.core
                        .issue_capability(&name, scopes)
                        .and_then(|issued| {
                            let entry = self
                                .core
                                .issued_entry(issued.0.id)
                                .context("issued capability vanished")?;
                            self.persist(entry)?;
                            Ok(issued)
                        })
                };
                let _ = reply.send(result);
            }
            WorldCommand::Revoke {
                participant_id,
                reply,
            } => {
                // Revocation takes effect in memory even if storage failed:
                // denying access is always the safe side.
                let was_active = self.core.revoke(participant_id);
                if was_active {
                    let _ = self.persist(JournalEntry::Revoked { participant_id });
                    let _ = self.revocation_sender.send(participant_id);
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
            WorldCommand::InstallProgram {
                module_hash,
                seed,
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
                    kind: WorldEventKind::ProgramInstalled { module_hash, seed },
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
                    // A grant can unblock refused subscriptions on the spot.
                    self.denied_subs.clear();
                    self.caps_report()
                })();
                let _ = reply.send(result);
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

/// Milliseconds since the Unix epoch; 0 if the clock is before it.
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
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

    /// A program driven by numeric inputs: each input switches its `wants`
    /// script, and every host event is recorded. Request ids embed the input
    /// count, so re-sending an input asks again.
    struct Mock {
        script: u64,
        received: Vec<String>,
        asks: u64,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                script: 0,
                received: Vec::new(),
                asks: 0,
            }
        }

        fn results(&self) -> Vec<serde_json::Value> {
            self.received
                .iter()
                .filter_map(|text| serde_json::from_str(text).ok())
                .collect()
        }
    }

    impl WorldProgram for Mock {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            match &event.kind {
                WorldEventKind::Input(bytes) => {
                    let text = std::str::from_utf8(bytes)?;
                    self.script = text.parse().unwrap_or(self.script);
                    self.asks += 1;
                }
                WorldEventKind::System { event } => self.received.push(event.clone()),
                WorldEventKind::Chat(_) | WorldEventKind::ProgramInstalled { .. } => {}
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let id = |tag: &str| format!("{tag}-{}", self.asks);
            let document = match self.script {
                1 => serde_json::json!({
                    "v": 1,
                    "subscribe": ["players"],
                    "requests": [{"id": id("now"), "cap": "time.now"}],
                }),
                2 => serde_json::json!({
                    "requests": [{"id": id("now"), "cap": "time.now"}],
                }),
                3 => serde_json::json!({
                    "requests": [{
                        "id": id("say"),
                        "cap": "chat.say",
                        "args": {"text": "hello from the program"},
                    }],
                }),
                4 => serde_json::json!({"subscribe": ["time.tick:20"]}),
                _ => serde_json::json!({}),
            };
            Ok(Some(document.to_string()))
        }
    }

    async fn caps_world_sync(grant_on_use: bool) -> (WorldHandle, JoinCapability) {
        let limits = WorldLimits {
            grant_on_use,
            ..WorldLimits::default()
        };
        let core = WorldCore::new("caps", limits).unwrap();
        let handle = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let (_, token) = handle.issue("ann", WorldScopes::GUEST).await.unwrap();
        (handle, token)
    }

    #[tokio::test]
    async fn an_ungranted_request_is_refused_and_counted() {
        let (world, token) = caps_world_sync(false).await;
        world.input(token, b"1".to_vec()).await.unwrap();
        // The program hears the refusal as a result event.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = world.caps_report().await.unwrap();
            if report
                .usage
                .get("time.now")
                .is_some_and(|use_| use_.denied > 0)
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no refusal recorded");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let report = world.caps_report().await.unwrap();
        assert_eq!(report.policy, "deny");
        assert!(
            report.missing.contains(&"time.now".to_owned()),
            "{report:?}"
        );
    }

    #[tokio::test]
    async fn a_granted_request_gets_a_real_answer() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(vec!["time.now".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"2".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if world.caps_report().await.unwrap().usage["time.now"].allowed > 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no answer recorded");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn chat_say_commits_a_system_chat_line_that_sessions_see() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        let mut events = world.subscribe();
        world.input(token, b"3".to_vec()).await.unwrap();
        // The participant's own input event arrives first; keep waiting for
        // the host's chat line.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .expect("no chat event")
                .unwrap();
            if let WorldEventKind::Chat(text) = &event.kind {
                assert_eq!(text, "hello from the program");
                assert_eq!(event.participant_id, 0, "the host said it");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "chat never arrived");
        }
    }

    #[tokio::test]
    async fn grant_on_use_grants_what_the_program_actually_uses() {
        let (world, token) = caps_world_sync(true).await;
        world.input(token, b"1".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if world.caps_report().await.unwrap().usage["time.now"].allowed > 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "never granted");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let report = world.caps_report().await.unwrap();
        assert_eq!(report.policy, "grant-on-use");
        assert!(report.granted.contains(&"time.now".to_owned()));
        assert!(report.missing.is_empty());
    }

    #[test]
    fn system_events_and_grants_survive_a_restore() {
        let secret = "ab".repeat(32);
        let token = JoinCapability::from_wire(format!("1.{secret}"));
        let digest = hex_encode(&token_digest(token.expose()));
        let entries = vec![
            JournalEntry::Created {
                world_id: "caps".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::CapsGranted {
                granted: vec!["time.now".to_owned()],
            },
            JournalEntry::Issued {
                participant: Participant {
                    id: 1,
                    display_name: "ann".to_owned(),
                },
                digest,
                scopes: WorldScopes::GUEST.0,
            },
            JournalEntry::System {
                event: WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: r#"{"cap":"result","id":"now-1","ok":true,"value":{"now":1}}"#
                            .to_owned(),
                    },
                },
            },
        ];
        let restored = WorldCore::restore("caps", WorldLimits::default(), entries).unwrap();
        assert!(restored.core.ledger.is_granted("time.now"));
        assert_eq!(restored.core.sequence, 0, "system events carry no sequence");
        assert_eq!(restored.replay.len(), 1, "the program must see it again");
        assert!(
            restored.core.authorize(&token, WorldScopes::JOIN).is_ok(),
            "capabilities restore after a grant entry"
        );
    }

    #[test]
    fn a_checkpoint_keeps_the_granted_set() {
        let mut core = WorldCore::new("caps", WorldLimits::default()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["time.now".to_owned()]));
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::CapsGranted { granted } if granted == &vec!["time.now".to_owned()])));
        // A compacted journal restores with the grant intact.
        let restored = WorldCore::restore("caps", WorldLimits::default(), entries).unwrap();
        assert!(restored.core.ledger.is_granted("time.now"));
    }

    #[tokio::test]
    async fn the_actor_journals_host_events_and_grants() {
        #[derive(Default, Clone)]
        struct Capturing(Arc<std::sync::Mutex<Vec<JournalEntry>>>);
        impl WorldJournal for Capturing {
            fn append(&mut self, entry: &JournalEntry) -> Result<()> {
                self.0.lock().unwrap().push(entry.clone());
                Ok(())
            }
        }
        let captured = Capturing::default();
        let (world, token) = {
            let core = WorldCore::new("j", WorldLimits::default()).unwrap();
            let handle = WorldHandle::spawn_durable(
                core,
                Box::new(Mock::new()),
                Some(Box::new(captured.clone())),
            );
            let (_, token) = handle.issue("ann", WorldScopes::GUEST).await.unwrap();
            (handle, token)
        };
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"3".to_vec()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        world.shutdown().await.unwrap();
        let entries = captured.0.lock().unwrap().clone();
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::CapsGranted { granted } if granted.contains(&"chat.say".to_owned()))),
            "{entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::System { event } if event.kind == WorldEventKind::System { event: r#"{"cap":"result","ok":true,"value":{"sequence":2}}"#.to_owned() }))
                || entries
                    .iter()
                    .any(|entry| matches!(entry, JournalEntry::System { .. })),
            "the chat.say result is journaled: {entries:?}"
        );
        assert!(
            entries.iter().any(
                |entry| matches!(entry, JournalEntry::Committed { event } if matches!(&event.kind, WorldEventKind::Chat(text) if text == "hello from the program"))
            ),
            "the host chat line is journaled: {entries:?}"
        );
    }

    #[tokio::test]
    async fn subscribed_ticks_arrive_only_while_someone_is_present() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(vec!["time.tick".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"4".to_vec()).await.unwrap();
        world.presence(1, true).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        world.presence(1, false).await;
        // Let any in-flight tick settle, then confirm the world goes quiet.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        let quiet = {
            let mut views = world.watch_views();
            views.borrow_and_update();
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            views.has_changed().is_ok_and(|changed| !changed)
        };
        assert!(quiet, "ticks kept firing with nobody present");
        world.shutdown().await.unwrap();
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

    /// Counts inputs; records are the count, or `bad` after an input "bad".
    struct Counter {
        count: u64,
        bad: bool,
    }

    impl WorldProgram for Counter {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::Input(bytes) = &event.kind {
                self.count += 1;
                self.bad = bytes.as_slice() == b"bad";
            }
            Ok(())
        }
        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(if self.bad {
                "[1,2,3]".to_owned()
            } else {
                format!("{{\"count\":{}}}", self.count)
            }))
        }
    }

    #[tokio::test]
    async fn records_follow_committed_inputs_and_bad_output_fails_closed() {
        let world = WorldHandle::spawn_with_program(
            world(16),
            Box::new(Counter {
                count: 0,
                bad: false,
            }),
        );
        assert_eq!(world.records().1["count"], 0);
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        let event = world.input(ann.clone(), b"go".to_vec()).await.unwrap();
        let (sequence, records) = world.records();
        assert_eq!(
            (sequence, records["count"].as_u64()),
            (event.sequence, Some(1))
        );

        // Records that are not a JSON object reject the input and poison the
        // program; the previous records stay published.
        assert!(world.input(ann.clone(), b"bad".to_vec()).await.is_err());
        assert_eq!(world.records().1["count"], 1);
        assert!(world.input(ann.clone(), b"go".to_vec()).await.is_err());
        let snapshot = world.snapshot(ann.clone()).await.unwrap();
        assert_eq!(
            snapshot.current_sequence, event.sequence,
            "nothing committed"
        );

        // A replacement with valid records restores service.
        world
            .install_program(
                "00".repeat(32),
                0,
                Box::new(Counter {
                    count: 10,
                    bad: false,
                }),
            )
            .await
            .unwrap();
        assert_eq!(world.records().1["count"], 10);
        world.input(ann, b"go".to_vec()).await.unwrap();
        assert_eq!(world.records().1["count"], 11);
    }

    #[tokio::test]
    async fn installs_with_invalid_records_are_refused() {
        let world = WorldHandle::spawn(world(16));
        let err = world
            .install_program(
                "00".repeat(32),
                0,
                Box::new(Counter {
                    count: 0,
                    bad: true,
                }),
            )
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("records"), "{err:#}");
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        assert_eq!(world.snapshot(ann).await.unwrap().active_module_hash, None);
    }

    #[test]
    fn record_keys_and_sizes_are_bounded() {
        assert!(parse_records(r#"{"a":1,"b":{"c":[true]}}"#).is_ok());
        assert!(parse_records("[]").is_err());
        assert!(parse_records(r#"{"":1}"#).is_err());
        assert!(parse_records("{\"a\\u0000b\":1}").is_err());
        let long_key = format!(r#"{{"{}":1}}"#, "k".repeat(MAX_RECORD_KEY_BYTES + 1));
        assert!(parse_records(&long_key).is_err());
        let big_value = format!(r#"{{"k":"{}"}}"#, "v".repeat(MAX_RECORD_VALUE_BYTES));
        assert!(parse_records(&big_value).is_err());
        let many: Records = (0..=MAX_RECORDS)
            .map(|i| (i.to_string(), 0.into()))
            .collect();
        assert!(parse_records(&serde_json::to_string(&many).unwrap()).is_err());
    }

    #[test]
    fn snapshots_carry_the_newest_events_that_fit_their_budget() {
        let mut core = WorldCore::new(
            "budget",
            WorldLimits {
                snapshot_bytes: 1024,
                input_bytes: 64,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let (_, token) = join(&mut core, "ann");
        for _ in 0..20u8 {
            core.input(&token, &[b'x'; 48]).unwrap();
        }
        let snapshot = core.snapshot(&token).unwrap();
        assert_eq!(snapshot.current_sequence, 20);
        assert!(!snapshot.events.is_empty());
        assert!(
            serde_json::to_vec(&snapshot.events).unwrap().len() <= 1024,
            "events fit the budget"
        );
        assert!(
            snapshot.events.len() < 20,
            "an oversized log is truncated to the newest events"
        );
        let first = snapshot.events.first().unwrap().sequence;
        assert_eq!(snapshot.oldest_retained_sequence, first);
        assert_eq!(snapshot.events.last().unwrap().sequence, 20);
        // The snapshot is a valid rebase point: continuing from its cursor
        // yields only new events.
        assert!(
            core.events_after(&token, snapshot.current_sequence)
                .unwrap()
                .events
                .is_empty()
        );
    }
}
