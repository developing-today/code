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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;

use anyhow::{Context, Result, bail, ensure};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;

/// Re-exported so `crate::world::Isolation` keeps working: the type now lives
/// in [`crate::groups::worlds`] so the crate builds without this feature.
pub use crate::groups::worlds::Isolation;

/// Re-exported so `crate::world::unix_ms` keeps working: the function now
/// lives in [`crate::time`] so the crate builds without this feature.
pub(crate) use crate::time::unix_ms;
mod actor;
mod clock;
mod core;
mod handle;
pub mod hotness;
mod metrics;
mod replica;
mod replication;
mod replication_receiver;
pub mod signed;
mod upgrade;
mod util;

#[cfg(test)]
mod tests;

pub(crate) use actor::{WorldActor, WorldCommand};
pub use clock::{Clock, ManualClock, SystemClock};
pub(crate) use core::AdminHistoryByteLedger;
pub use core::WorldCore;
pub use handle::WorldHandle;
pub use hotness::{FiredSignal, HotnessBand, HotnessReport, HotnessSignal};
pub use metrics::{
    HistogramSnapshot, MetricsSnapshot, RegionWrites, WorldMetrics, WriteLocalitySnapshot,
};
pub use replica::{
    ReplicaCursor, ReplicaRestore, parse_replica_journal, restore_replica_entries,
    restore_replica_entries_with_runtime, restore_replica_journal,
    restore_replica_journal_with_runtime, serve_replica,
};
pub use replication::{
    AcceptedFrame, MAX_REPLICATION_FRAME_BYTES, MAX_REPLICATION_PAYLOAD_BYTES, REPLICATION_DOMAIN,
    REPLICATION_ENVELOPE_VERSION, REPLICATION_FRAME_MAGIC, ReplicationCursor, ReplicationEnvelope,
    ReplicationError, ReplicationKind, ReplicationTracker, SESSION_NONCE_BYTES,
    SignedReplicationEnvelope, advance_cursor, chain_base_cursor, chain_payload, frame_digest,
    journal_entry_line, payload_digest, split_entry_payload,
};
pub use replication_receiver::{
    DurableReceiver, MAX_STAGED_TRANSFER_BYTES, REPLICA_JOURNAL_FILE, ReceiverError,
    ReceiverProgress,
};
pub use upgrade::{
    MAX_PROGRAM_STATE_BYTES, MigrateExport, ProgramSnapshot, RunningProgram,
    SNAPSHOT_FORMAT_CURRENT, SNAPSHOT_FORMAT_V1, SNAPSHOT_MAGIC, UpgradeOutcome, upgrade_program,
};
pub use util::{EventPage, hex_encode, publish_records};
pub(crate) use util::{
    constant_time_eq, hex_decode_32, is_pre_fix_mint_display_name, legacy_sanitize_display_name,
    pre_fix_sanitize_display_name, sanitize_chat, sanitize_display_name, token_digest,
};

/// Identity of one live connection. It stays in memory only and is never part
/// of a world event or journal entry.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct SessionId(u64);

impl SessionId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// A chat participant may send at most this many lines during the rolling window.
pub(crate) const CHAT_RATE_BURST: u8 = 50;
pub(crate) const CHAT_RATE_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
/// Maximum host-authorized event history kept for fixture export.
pub(crate) const MAX_ADMIN_HISTORY_EVENTS: usize = 4096;
/// Maximum encoded history bytes kept in memory or a checkpoint.
pub(crate) const MAX_ADMIN_HISTORY_BYTES: usize = 64 * 1024 * 1024;
/// Maximum distinct participant IDs represented as open in a checkpoint.
///
/// This is a stable journal-format bound, not the current world's participant
/// quota. If a live world exceeds it (for example, while revoked sessions are
/// still closing), compaction is skipped and the complete append-only journal
/// is retained. Restore enforces the same bound regardless of current config.
pub(crate) const MAX_CHECKPOINT_OPEN_PRESENCE: usize = 4096;
/// Largest encoded JSON checkpoint part entry. This remains below the v1
/// replication payload limit so each part is carried whole in one frame.
pub const MAX_CHECKPOINT_PART_BYTES: usize = 192 * 1024;

/// Bounded limits for one world.
#[derive(Clone, Debug)]
pub struct WorldLimits {
    /// Maximum number of active capabilities in this world.
    pub participants: usize,
    /// Maximum UTF-8 chat message length in bytes.
    pub chat_bytes: usize,
    /// Maximum opaque game input size in bytes.
    pub input_bytes: usize,
    /// Require entity-addressed application inputs instead of legacy opaque
    /// inputs. Enable for region-sharded worlds; defaults off for compatibility.
    pub region_sharded: bool,
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
    /// Minimum tick interval a program may subscribe to, in milliseconds.
    /// Subscriptions below this floor are refused at subscription time.
    pub min_tick_ms: u64,
    /// Whether a world's policy grants a capability on its first use.
    pub grant_on_use: bool,
    /// Capabilities the server allows any world to use. Runtime configuration,
    /// not journaled; a world's own ceiling and grants only narrow it.
    pub server_caps: BTreeSet<String>,
}

impl Default for WorldLimits {
    fn default() -> Self {
        Self {
            participants: 256,
            chat_bytes: 2048,
            input_bytes: 16 * 1024,
            region_sharded: false,
            retained_events: 1024,
            presentation_bytes: 8192,
            snapshot_bytes: 512 * 1024,
            checkpoint_every: 1000,
            min_tick_ms: 1000,
            grant_on_use: false,
            server_caps: crate::world_caps::default_ceiling_names(),
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
    /// Mint capabilities with a subset of this set.
    pub const DELEGATE: Self = Self(0b1000);
    /// Read chat lines without being able to send them. `CHAT` also reads.
    pub const HISTORY: Self = Self(0b1_0000);
    /// Install and upgrade the world's program. Carries no capability,
    /// network, file or process authority: only the install path reads it.
    pub const BUILDER: Self = Self(0b10_0000);
    /// Revoke participants. Carries no capability, network, file or process
    /// authority: only the revoke path reads it.
    pub const MODERATOR: Self = Self(0b100_0000);
    /// Default guest permissions.
    pub const GUEST: Self = Self(Self::JOIN.0 | Self::CHAT.0 | Self::INPUT.0);
    /// Every participant scope. The admin authority bits `BUILDER` and
    /// `MODERATOR` are deliberately excluded: they are issued only by the
    /// admin, never implied by a ceiling or handed out by a delegator.
    pub const ALL: Self = Self(Self::GUEST.0 | Self::DELEGATE.0 | Self::HISTORY.0);
    /// Every bit the type recognizes, including the admin authority bits.
    /// [`Self::from_bits`] accepts these so a journaled builder or moderator
    /// capability round-trips, without widening [`Self::ALL`].
    const ALL_BITS: u8 = Self::ALL.0 | Self::BUILDER.0 | Self::MODERATOR.0;

    /// Every recognized bit, including the admin authority bits. Used as
    /// the seed for authority-aware effective-scope computation: unlike
    /// [`Self::ALL`], this lets an admin-issued `BUILDER` / `MODERATOR`
    /// bit survive the ancestor-chain intersection in
    /// `WorldCore::authorize` (design #835 phase 2). It grants nothing on
    /// its own — the chain intersection only ever removes bits.
    pub(crate) const fn all_recognized() -> Self {
        Self(Self::ALL_BITS)
    }

    /// Whether this set includes the builder authority (install modules).
    #[must_use]
    pub const fn is_builder(self) -> bool {
        self.contains(Self::BUILDER)
    }

    /// Whether this set includes the moderator authority (revoke participants).
    #[must_use]
    pub const fn is_moderator(self) -> bool {
        self.contains(Self::MODERATOR)
    }

    /// Role presets used by `id world invite --role`. `spectator` joins and
    /// sees the world but not the chat; `observer` also reads chat without
    /// sending it; `player` submits input; `guest` (the default) also chats;
    /// `delegator` additionally mints narrower capabilities for others;
    /// `builder` additionally installs modules; `moderator` additionally
    /// revokes participants.
    ///
    /// The names are stable; the mapping to scopes is part of the protocol.
    #[must_use]
    pub fn from_role(name: &str) -> Option<Self> {
        match name {
            "spectator" => Some(Self::JOIN),
            "observer" => Some(Self(Self::JOIN.0 | Self::HISTORY.0)),
            "player" => Some(Self(Self::JOIN.0 | Self::INPUT.0)),
            "guest" => Some(Self::GUEST),
            "delegator" => Some(Self(Self::GUEST.0 | Self::DELEGATE.0)),
            "builder" => Some(Self(Self::GUEST.0 | Self::BUILDER.0)),
            "moderator" => Some(Self(Self::GUEST.0 | Self::MODERATOR.0)),
            _ => None,
        }
    }

    /// The names accepted by [`Self::from_role`].
    pub const ROLES: &'static [&'static str] = &[
        "spectator",
        "observer",
        "player",
        "guest",
        "delegator",
        "builder",
        "moderator",
    ];

    /// Raw scope bits, as journaled.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Scopes from raw bits; `None` if any bit names no scope. The admin
    /// authority bits round-trip here even though they are outside [`Self::ALL`].
    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL_BITS == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// Every scope either set includes.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Only the scopes both sets include.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Whether this scope set includes `required`.
    #[must_use]
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    /// This set with the scopes it includes by implication: `CHAT` reads, so
    /// it carries `HISTORY`. Apply it wherever sets are compared as sets.
    #[must_use]
    pub const fn implied(self) -> Self {
        if self.contains(Self::CHAT) {
            Self(self.0 | Self::HISTORY.0)
        } else {
            self
        }
    }

    /// Whether a participant with this set may see chat lines.
    #[must_use]
    pub const fn reads_chat(self) -> bool {
        self.0 & (Self::CHAT.0 | Self::HISTORY.0) != 0
    }

    /// The scopes a delegate of this set may hold: a subset that keeps `JOIN`.
    /// `None` unless this set includes `DELEGATE` and `requested` is a subset.
    /// A set with `CHAT` may delegate `HISTORY`, which `CHAT` includes.
    #[must_use]
    pub const fn attenuate(self, requested: Self) -> Option<Self> {
        if self.contains(Self::DELEGATE)
            && requested.contains(Self::JOIN)
            && self.implied().contains(requested)
        {
            Some(requested)
        } else {
            None
        }
    }

    /// Whether a participant with this set may cause the effect `cap`. Every
    /// connected participant may read; `chat.say` also needs `CHAT`.
    #[must_use]
    pub fn allows_effect(self, cap: &str) -> bool {
        match crate::world_caps::PROVIDER_REGISTRY
            .get(cap)
            .map_or(crate::world_caps::RequiredScope::Join, |provider| {
                provider.required_scope
            }) {
            crate::world_caps::RequiredScope::Chat => self.contains(Self::CHAT),
            // Reading chat needs only the right to read it (`CHAT` or
            // `HISTORY`), the same rule the event stream applies.
            crate::world_caps::RequiredScope::ChatRead => self.reads_chat(),
            crate::world_caps::RequiredScope::Join => self.contains(Self::JOIN),
        }
    }

    /// Scopes named on the wire (`join`, `chat`, `history`, `input`,
    /// `delegate`, `builder`, `moderator`), or `None` if a name is unknown.
    #[must_use]
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        names.into_iter().try_fold(Self(0), |scopes, name| {
            let bit = match name {
                "join" => Self::JOIN,
                "chat" => Self::CHAT,
                "history" => Self::HISTORY,
                "input" => Self::INPUT,
                "delegate" => Self::DELEGATE,
                "builder" => Self::BUILDER,
                "moderator" => Self::MODERATOR,
                _ => return None,
            };
            Some(Self(scopes.0 | bit.0))
        })
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
    /// Entity-addressed input for region-sharded worlds (#3870, design
    /// #2818): the input targets exactly one application-owned entity, so
    /// the host can fence that `entity_ref` during a region-seam handoff
    /// without inspecting the opaque bytes. Region-sharded mode rejects
    /// legacy unscoped [`WorldEventKind::Input`] (phase 2 of #3870 wires
    /// the gate; this packet adds the kind and its journal plumbing).
    EntityInput {
        /// Application-defined stable entity reference (not a world-local
        /// participant ID).
        entity_ref: String,
        /// Opaque input bytes for the program, as with
        /// [`WorldEventKind::Input`].
        input: Vec<u8>,
    },
    /// An event the host originated for the program: capability results,
    /// ticks, presence. Delivered with `participant_id` 0; never sequenced
    /// or broadcast to participant sessions. Admins may export these events
    /// through the history protocol.
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
        /// Journal/admin only: join projections redact it to zero (#240).
        #[serde(default)]
        seed: u64,
        /// Server-observed origin metadata. Absent on older journal entries.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<ModuleProvenance>,
        /// The trust decision that admitted this module (#1069), carried
        /// by the install event itself so the decision and the install
        /// are one durable record: a separate audit append after the
        /// install event could fail and leave a durable install with no
        /// decision (or a split live/restored state). Absent on older
        /// journal entries.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trust: Option<TrustDecision>,
    },
}

/// How the active module artifact entered the world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", content = "build", rename_all = "snake_case")]
pub enum ModuleProvenance {
    /// A raw module artifact was installed; no compiler identity is asserted.
    Artifact,
    /// The server observed a successful source build. These hashes identify
    /// inputs; they are not a signature or a trust attestation.
    SourceBuild(SourceBuildProvenance),
    /// The module was forked from another world: the world's state and module
    /// were copied at the fork point, with the origin recorded.
    Fork(ForkProvenance),
}

/// Provenance recorded when one world is forked from another (F12.2). It
/// names the source world and module so a fork can trace its origin, and
/// whether the fork was marked reusable as a template.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkProvenance {
    /// Provenance record schema, independent of the world journal version.
    pub schema_version: u8,
    /// The world the fork was copied from.
    pub from_world_id: String,
    /// The content hash of the module at the fork point.
    pub from_module_hash: String,
    /// Whether the fork is marked reusable as a template.
    pub template: bool,
}

/// Bounded, deterministic metadata observed for one source compilation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBuildProvenance {
    /// Provenance record schema, independent of the world journal version.
    pub schema_version: u8,
    /// Domain-separated BLAKE3 digest of the compiler executable bytes.
    pub compiler_blake3: String,
    /// Version string the compiler binary reports for itself (`roc
    /// version`), recorded alongside its digest (#1069): the digest
    /// identifies the exact bytes, the version is the human-legible
    /// name for them. Empty when the version probe failed. Defaulted
    /// so provenance records written before this field existed still
    /// deserialize.
    #[serde(default)]
    pub compiler_version: String,
    /// BLAKE3 digest of the platform tree, including relative paths.
    pub platform_blake3: String,
    /// BLAKE3 digest of the submitted source filenames and contents.
    pub source_blake3: String,
    /// Compiler target (`wasm32` or `x64musl`).
    pub target: String,
    /// Stable compiler arguments, excluding the temporary output path.
    pub arguments: Vec<String>,
}

/// Which paths a world's files may name.
///
/// No world file capability exists yet; this is the policy one must check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileScope {
    /// Only the world's own sandbox root.
    #[default]
    Confined,
    /// Any path.
    Unrestricted,
}

/// Whether a file access reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    /// Reading a file.
    Read,
    /// Creating, changing or deleting a file.
    Write,
}

/// A world's sharing and file settings, read in one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldPolicy {
    /// Whether other worlds may read this one.
    pub isolation: Isolation,
    /// Whether other unisolated worlds may write here.
    pub cross_world_write: bool,
    /// Whether an admin forced `cross_world_write` on, so a moderator cannot turn it off.
    pub cross_world_forced: bool,
    /// Which paths the world's files may name.
    pub file_scope: FileScope,
    /// Whether the admin let an isolated world write files.
    pub file_write_override: bool,
}

impl WorldPolicy {
    /// Whether a file access is allowed. Isolated worlds only read unless the
    /// admin override is set; the scope then decides which paths.
    #[must_use]
    pub const fn file_allowed(self, mode: FileMode, inside_root: bool) -> bool {
        let in_scope = match self.file_scope {
            FileScope::Confined => inside_root,
            FileScope::Unrestricted => true,
        };
        let writable = match mode {
            FileMode::Read => true,
            FileMode::Write => {
                matches!(self.isolation, Isolation::Unisolated) || self.file_write_override
            }
        };
        in_scope && writable
    }
}

/// Who asks for a change only an admin or a moderator may make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authority {
    /// The process admin token, already checked by the transport.
    Admin,
    /// A signed request made with this public key, as lower-case hex.
    Signed(String),
}

/// Which side of a region-seam handoff a [`HandoffTransferState`] records
/// (#3870, design #2818).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffDirection {
    /// This world exported the entity (source side): it holds the fence and
    /// the durable recovery payload until coordinator completion.
    Export,
    /// This world imported the entity (destination side): it holds the
    /// dedupe receipt keyed by `handoff_id`.
    Import,
}

/// Maximum complete entity snapshot accepted by a v1 region-seam export.
pub const MAX_ENTITY_TRANSFER_PAYLOAD_BYTES: usize = 16 * 1024;

/// Request passed to a world program to prepare one side of an entity transfer.
/// The hook must be pure: it validates and describes the event but must not
/// mutate program state. The actor persists the resulting event before applying
/// it, so replay observes the same state transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityTransferRequest {
    /// Prepare the source-side removal and complete entity snapshot.
    Export {
        /// Coordinator-generated 128-bit handoff ID, encoded as hex.
        handoff_id: String,
        /// Application-defined stable identifier for the entity.
        entity_ref: String,
        /// Region currently owning the entity.
        source_region: String,
        /// Region that will receive the entity.
        target_region: String,
        /// Current owner epoch at the source.
        prior_owner_epoch: u64,
        /// Hash of the program module preparing the transfer.
        program_module_hash: String,
    },
    /// Validate a destination-side envelope and prepare its import event.
    Import {
        /// Coordinator-generated 128-bit handoff ID, encoded as hex.
        handoff_id: String,
        /// Application-defined stable identifier for the entity.
        entity_ref: String,
        /// Region that exported the entity.
        source_region: String,
        /// Region currently importing the entity.
        target_region: String,
        /// Owner epoch at the source before this transfer.
        prior_owner_epoch: u64,
        /// Hash of the program module expected by the envelope.
        program_module_hash: String,
        /// Versioned identifier for the entity payload format.
        payload_schema: String,
        /// Complete source snapshot to validate and import.
        payload: Vec<u8>,
        /// BLAKE3 digest of `payload`, encoded as lowercase hex.
        payload_digest: String,
    },
}

/// Pure, validated application work returned by
/// [`WorldProgram::prepare_entity_transfer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityTransferPlan {
    /// The source's complete snapshot and deterministic removal event.
    Export {
        /// Versioned identifier for the entity payload format.
        payload_schema: String,
        /// Complete source snapshot; limited to
        /// [`MAX_ENTITY_TRANSFER_PAYLOAD_BYTES`].
        payload: Vec<u8>,
        /// Deterministic program event that removes the entity at the source.
        app_event: String,
    },
    /// The deterministic destination insertion event.
    Import {
        /// Deterministic program event that adds the entity at the destination.
        app_event: String,
    },
}

/// One active region-seam handoff as a world journal records it (#3870
/// phase 1, design #2818 §3). Carried in [`JournalEntry::Checkpoint`] so
/// compaction preserves active export fences/payloads, import receipts and
/// owner epochs exactly as the un-compacted journal would replay them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffTransferState {
    /// Coordinator-generated random 128-bit handoff ID, as hex. Reused for
    /// every retry of this crossing; a new crossing gets a new ID.
    pub handoff_id: String,
    /// Application-defined stable entity reference.
    pub entity_ref: String,
    /// Region (shard world) the entity leaves.
    pub source_region: String,
    /// Region (shard world) the entity enters.
    pub target_region: String,
    /// Which side of the seam this world is.
    pub direction: HandoffDirection,
    /// Owner epoch the entity had at the source before this handoff.
    pub prior_owner_epoch: u64,
    /// Owner epoch at the destination once the handoff completes
    /// (`prior_owner_epoch + 1`); set on import records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_owner_epoch: Option<u64>,
    /// Program module hash both regions must agree on for v1.
    pub program_module_hash: String,
    /// Versioned payload schema name.
    pub payload_schema: String,
    /// Digest of the complete entity payload.
    pub payload_digest: String,
    /// The complete entity payload. Present on export records (the source
    /// is the durable recovery copy until completion); absent on import
    /// records, whose payload rides inside `event`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<u8>>,
    /// The host-originated system event that removes (export) or adds
    /// (import) the entity in the program. Replay applies it from the same
    /// durable fact as the journal entry itself.
    pub event: WorldEvent,
}

/// The lease holder every world starts with: the hosted actor itself
/// (design #2819 bootstrap — existing worlds with no lease records are
/// DO-owned at epoch 0).
pub const DO_WRITE_LEASE_HOLDER: &str = "do";

/// Recommended native lease term (design #2819): 30 seconds, renewed by
/// the DO every 10 seconds, judged only on the DO authority's clock.
pub const WRITE_LEASE_TERM_MS: u64 = 30_000;

/// Recommended interval between DO renewals of a native lease
/// (design #2819).
pub const WRITE_LEASE_RENEWAL_INTERVAL_MS: u64 = 10_000;

/// Why the append gate refused a writer (design #2819, phase 2 of
/// issue #3804). A refusal is retryable in every case: the writer catches
/// up, waits for a grant, or retries under the new holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseRejectionReason {
    /// The writer is not the current lease holder (or there is none).
    NotHolder,
    /// The writer presented an epoch other than the current one.
    StaleEpoch,
    /// The current lease term had expired on the authority's clock.
    Expired,
}

/// Which lease transition a [`WriteLeaseTransition`] records
/// (design #2819).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseTransitionKind {
    /// A lease was granted while no holder stood (bootstrap is not a
    /// grant; the first grant follows a revocation).
    Granted,
    /// The current holder's term was extended; the epoch is unchanged.
    Renewed,
    /// The lease was handed from one holder to another.
    HandedOff,
    /// The lease was revoked; no holder stands until the next grant.
    Revoked,
}

/// One auditable lease transition, in journal order (design #2819:
/// compaction must retain every transition needed to prove lease
/// ownership at represented positions). `epoch` is the epoch in force
/// after the transition (for a renewal, the unchanged epoch).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteLeaseTransition {
    /// The kind of transition.
    pub kind: LeaseTransitionKind,
    /// The holder before the transition (`None` when none stood).
    pub from: Option<String>,
    /// The holder after the transition (`None` after a revocation).
    pub to: Option<String>,
    /// The epoch in force after the transition.
    pub epoch: u64,
    /// The term expiry in force after the transition, in unix
    /// milliseconds (`None` for an untermed holder such as the DO).
    pub expires_at_ms: Option<u64>,
    /// Why a revocation happened; absent for other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One rejected stale append attempt, as the append authority audited
/// it (design #2819). Deduplicated by `(writer, presented_epoch,
/// operation_id)`: repeated deliveries of the same attempt produce one
/// audit record, while a client retry of the same operation under a new
/// holder is a distinct attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseWriteRejection {
    /// The authenticated writer identity that was refused.
    pub writer: String,
    /// The epoch the writer presented.
    pub presented_epoch: u64,
    /// The client operation ID the writer presented.
    pub operation_id: String,
    /// The authority's epoch when the attempt was refused.
    pub observed_epoch: u64,
    /// Why the attempt was refused.
    pub reason: LeaseRejectionReason,
    /// When the authority received the attempt, in unix milliseconds
    /// on the authority's clock.
    pub received_at_ms: u64,
}

/// One entry in a [`WriteLease`]'s audit history, in journal order.
/// Transitions and rejection audits share one ordered list because
/// each rejection audit is only valid against the lease state at its
/// own journal position: a stale write audited at epoch 1 stays valid
/// after a revocation advances the epoch to 2, and a validator that
/// replays every transition before any audit would refuse it
/// (Codex review on PR #5571).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum WriteLeaseHistory {
    /// A lease transition (grant, renewal, handoff, revocation).
    Transition(WriteLeaseTransition),
    /// A rejected append attempt the authority audited.
    Rejection(LeaseWriteRejection),
}

/// The journal-resident write lease for one world (design #2819,
/// phase 2 of issue #3804): the derived state the append gate fences
/// on, rebuilt by journal replay and carried whole in checkpoints so
/// compaction preserves the current state, the monotonic epoch, the
/// transition history, and the rejection audit.
///
/// Safety invariants: epochs never decrease or repeat across grant,
/// handoff, and revocation (a renewal keeps the epoch); there is at
/// most one holder at any epoch; expiry fences the old holder even
/// before a revocation is journaled, because [`WriteLease::check_writer`]
/// judges the term at the append decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteLease {
    /// The current holder's authenticated identity; `None` between a
    /// revocation and the next grant.
    pub holder: Option<String>,
    /// The current epoch. Bootstrap is epoch 0.
    pub epoch: u64,
    /// When the current term expires, in unix milliseconds on the
    /// authority's clock; `None` for an untermed holder.
    pub expires_at_ms: Option<u64>,
    /// Every transition and audited rejection, interleaved in
    /// journal order — the order is load-bearing (see
    /// [`WriteLeaseHistory`]).
    pub history: Vec<WriteLeaseHistory>,
}

impl Default for WriteLease {
    fn default() -> Self {
        Self::bootstrap()
    }
}

impl WriteLease {
    /// The pre-spill state of every existing world: the DO actor is the
    /// incumbent holder at epoch 0, with no term and no history.
    #[must_use]
    pub fn bootstrap() -> Self {
        Self {
            holder: Some(DO_WRITE_LEASE_HOLDER.to_owned()),
            epoch: 0,
            expires_at_ms: None,
            history: Vec::new(),
        }
    }

    /// Every transition applied, in journal order.
    pub fn transitions(&self) -> impl Iterator<Item = &WriteLeaseTransition> {
        self.history.iter().filter_map(|entry| match entry {
            WriteLeaseHistory::Transition(transition) => Some(transition),
            WriteLeaseHistory::Rejection(_) => None,
        })
    }

    /// Every audited rejected append attempt, in journal order.
    pub fn rejections(&self) -> impl Iterator<Item = &LeaseWriteRejection> {
        self.history.iter().filter_map(|entry| match entry {
            WriteLeaseHistory::Transition(_) => None,
            WriteLeaseHistory::Rejection(rejection) => Some(rejection),
        })
    }

    /// Whether this lease is still the untouched bootstrap state (a
    /// checkpoint then omits it, exactly as older checkpoints did).
    #[must_use]
    pub fn is_bootstrap(&self) -> bool {
        *self == Self::bootstrap()
    }

    /// Whether `now_ms` is past the current term. An untermed holder
    /// never expires; a world with no holder has nothing to expire.
    #[must_use]
    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.holder.is_some() && self.expires_at_ms.is_some_and(|expiry| now_ms >= expiry)
    }

    /// The append-gate check (design #2819): a mutation append is
    /// admitted only from the current holder, presenting the current
    /// epoch, before the term expires on the authority's clock. This
    /// is the check the serialized append path performs in the same
    /// operation as the append itself; a caller that passes it and
    /// then appends through any other path has no fencing.
    ///
    /// # Errors
    ///
    /// The [`LeaseRejectionReason`] the gate records for the attempt.
    pub fn check_writer(
        &self,
        writer: &str,
        presented_epoch: u64,
        now_ms: u64,
    ) -> Result<(), LeaseRejectionReason> {
        if self.holder.as_deref() != Some(writer) {
            return Err(LeaseRejectionReason::NotHolder);
        }
        if presented_epoch != self.epoch {
            return Err(LeaseRejectionReason::StaleEpoch);
        }
        if self.is_expired(now_ms) {
            return Err(LeaseRejectionReason::Expired);
        }
        Ok(())
    }

    /// Apply a `LeaseGranted` record: legal only while no holder
    /// stands, and the granted epoch must be exactly the next epoch.
    ///
    /// # Errors
    ///
    /// A grant over a standing holder (that is a handoff, not a grant),
    /// or an epoch that is not `current + 1`.
    pub fn apply_grant(
        &mut self,
        holder: &str,
        epoch: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<()> {
        ensure!(
            self.holder.is_none(),
            "journal grants the write lease to {holder:?} while {:?} holds it",
            self.holder
        );
        self.advance_epoch(epoch, "grant")?;
        self.holder = Some(holder.to_owned());
        self.expires_at_ms = expires_at_ms;
        self.history
            .push(WriteLeaseHistory::Transition(WriteLeaseTransition {
                kind: LeaseTransitionKind::Granted,
                from: None,
                to: Some(holder.to_owned()),
                epoch,
                expires_at_ms,
                reason: None,
            }));
        Ok(())
    }

    /// Apply a `LeaseRenewed` record: only the current holder renews,
    /// the epoch is unchanged, and the term never moves backwards.
    ///
    /// # Errors
    ///
    /// A renewal by a non-holder, under a different epoch, or moving
    /// the expiry backwards.
    pub fn apply_renew(&mut self, holder: &str, epoch: u64, expires_at_ms: u64) -> Result<()> {
        ensure!(
            self.holder.as_deref() == Some(holder),
            "journal renews the write lease for {holder:?}, held by {:?}",
            self.holder
        );
        ensure!(
            epoch == self.epoch,
            "journal renews the write lease at epoch {epoch}, current epoch is {}",
            self.epoch
        );
        ensure!(
            self.expires_at_ms
                .is_none_or(|current| expires_at_ms >= current),
            "journal renewal moves the write lease expiry backwards"
        );
        self.expires_at_ms = Some(expires_at_ms);
        self.history
            .push(WriteLeaseHistory::Transition(WriteLeaseTransition {
                kind: LeaseTransitionKind::Renewed,
                from: Some(holder.to_owned()),
                to: Some(holder.to_owned()),
                epoch,
                expires_at_ms: Some(expires_at_ms),
                reason: None,
            }));
        Ok(())
    }

    /// Apply a `LeaseHandedOff` record: the handoff names the current
    /// holder as its source and advances the epoch by exactly one;
    /// that journal position is the ownership boundary.
    ///
    /// # Errors
    ///
    /// A handoff from a non-holder, or an epoch that is not
    /// `current + 1`.
    pub fn apply_handoff(
        &mut self,
        from: &str,
        to: &str,
        epoch: u64,
        expires_at_ms: Option<u64>,
    ) -> Result<()> {
        ensure!(
            self.holder.as_deref() == Some(from),
            "journal hands the write lease off from {from:?}, held by {:?}",
            self.holder
        );
        self.advance_epoch(epoch, "handoff")?;
        self.holder = Some(to.to_owned());
        self.expires_at_ms = expires_at_ms;
        self.history
            .push(WriteLeaseHistory::Transition(WriteLeaseTransition {
                kind: LeaseTransitionKind::HandedOff,
                from: Some(from.to_owned()),
                to: Some(to.to_owned()),
                epoch,
                expires_at_ms,
                reason: None,
            }));
        Ok(())
    }

    /// Apply a `LeaseRevoked` record: revocation names the current
    /// holder, advances the epoch by exactly one, and leaves no
    /// holder standing.
    ///
    /// # Errors
    ///
    /// A revocation of a non-holder, or an epoch that is not
    /// `current + 1`.
    pub fn apply_revoke(&mut self, former_holder: &str, epoch: u64, reason: &str) -> Result<()> {
        ensure!(
            self.holder.as_deref() == Some(former_holder),
            "journal revokes the write lease from {former_holder:?}, held by {:?}",
            self.holder
        );
        self.advance_epoch(epoch, "revocation")?;
        self.holder = None;
        self.expires_at_ms = None;
        self.history
            .push(WriteLeaseHistory::Transition(WriteLeaseTransition {
                kind: LeaseTransitionKind::Revoked,
                from: Some(former_holder.to_owned()),
                to: None,
                epoch,
                expires_at_ms: None,
                reason: Some(reason.to_owned()),
            }));
        Ok(())
    }

    /// Apply a `LeaseWriteRejected` audit record. The audited attempt
    /// must actually be refusable under the state it names: a writer
    /// that would have passed the gate cannot be audited as rejected.
    /// Duplicate deliveries of one attempt (same writer, presented
    /// epoch, and operation ID) are refused, so a journal carries at
    /// most one audit record per attempt.
    ///
    /// # Errors
    ///
    /// An audit whose observed epoch is not the current epoch, whose
    /// stated reason disagrees with the gate check (expiry is judged
    /// at `received_at_ms`, the authority's own clock reading), or
    /// that duplicates an earlier audit.
    pub fn apply_rejection(&mut self, rejection: LeaseWriteRejection) -> Result<()> {
        ensure!(
            rejection.observed_epoch == self.epoch,
            "journal audits a rejected write at observed epoch {}, current epoch is {}",
            rejection.observed_epoch,
            self.epoch
        );
        ensure!(
            self.check_writer(
                &rejection.writer,
                rejection.presented_epoch,
                rejection.received_at_ms
            ) == Err(rejection.reason),
            "journal audits a rejected write by {:?} whose stated reason {:?} disagrees with the lease gate",
            rejection.writer,
            rejection.reason
        );
        ensure!(
            !self.rejections().any(|prior| {
                prior.writer == rejection.writer
                    && prior.presented_epoch == rejection.presented_epoch
                    && prior.operation_id == rejection.operation_id
            }),
            "journal audits rejected write {:?} twice",
            rejection.operation_id
        );
        self.history.push(WriteLeaseHistory::Rejection(rejection));
        Ok(())
    }

    /// Validate a checkpoint-carried lease as a whole: replaying its
    /// transitions and rejections from bootstrap must reproduce the
    /// exact state the checkpoint claims. Used at restore, where a
    /// checkpoint stands in for the journal it compacted.
    ///
    /// # Errors
    ///
    /// Any transition or rejection that replay refuses, or a final
    /// state that differs from the claimed one.
    pub fn validate_history(&self) -> Result<()> {
        let mut replayed = Self::bootstrap();
        for entry in &self.history {
            match entry {
                WriteLeaseHistory::Transition(transition) => match transition.kind {
                    LeaseTransitionKind::Granted => replayed.apply_grant(
                        transition
                            .to
                            .as_deref()
                            .context("lease grant transition names no holder")?,
                        transition.epoch,
                        transition.expires_at_ms,
                    )?,
                    LeaseTransitionKind::Renewed => replayed.apply_renew(
                        transition
                            .to
                            .as_deref()
                            .context("lease renewal transition names no holder")?,
                        transition.epoch,
                        transition
                            .expires_at_ms
                            .context("lease renewal transition names no expiry")?,
                    )?,
                    LeaseTransitionKind::HandedOff => replayed.apply_handoff(
                        transition
                            .from
                            .as_deref()
                            .context("lease handoff transition names no source")?,
                        transition
                            .to
                            .as_deref()
                            .context("lease handoff transition names no target")?,
                        transition.epoch,
                        transition.expires_at_ms,
                    )?,
                    LeaseTransitionKind::Revoked => replayed.apply_revoke(
                        transition
                            .from
                            .as_deref()
                            .context("lease revocation transition names no holder")?,
                        transition.epoch,
                        transition
                            .reason
                            .as_deref()
                            .context("lease revocation transition names no reason")?,
                    )?,
                },
                WriteLeaseHistory::Rejection(rejection) => {
                    replayed.apply_rejection(rejection.clone())?;
                }
            }
        }
        ensure!(
            replayed == *self,
            "checkpoint write lease history does not reproduce its claimed state"
        );
        Ok(())
    }

    fn advance_epoch(&mut self, epoch: u64, what: &str) -> Result<()> {
        ensure!(
            self.epoch.checked_add(1) == Some(epoch),
            "journal {what} advances the write lease epoch from {} to {epoch}",
            self.epoch
        );
        self.epoch = epoch;
        Ok(())
    }
}

/// One durable fact in the region coordinator's own journal (#3870 phase 1,
/// design #2818 §3). The coordinator records and recovers handoff progress;
/// it never stores entity payloads and never emulates a cross-region
/// transaction. Phase 4 of #3870 builds the coordinator state machine on
/// these records; this packet fixes their format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum CoordinatorJournalEntry {
    /// A handoff was requested. Persisted before any shard side effect.
    HandoffStarted {
        /// Random 128-bit handoff ID, as hex.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region the entity leaves.
        source_region: String,
        /// Region the entity enters.
        target_region: String,
        /// Owner epoch the coordinator expects at the source.
        expected_owner_epoch: u64,
        /// Region-map epoch observed when the transfer was requested.
        region_map_epoch: u64,
        /// Program module hash both regions must run for v1.
        program_module_hash: String,
        /// Versioned payload schema name.
        payload_schema: String,
        /// Unix milliseconds the handoff was requested (operator
        /// visibility only; ordering comes from journal order).
        requested_at: u64,
    },
    /// The coordinator verified the source's durable, applied export
    /// receipt for this handoff.
    HandoffExportObserved {
        /// Handoff ID.
        handoff_id: String,
        /// Digest of the exported payload.
        payload_digest: String,
        /// Unix milliseconds the receipt was observed.
        observed_at: u64,
    },
    /// The coordinator verified the destination's durable, applied import
    /// receipt for this handoff.
    HandoffImportObserved {
        /// Handoff ID.
        handoff_id: String,
        /// Digest of the imported payload.
        payload_digest: String,
        /// Unix milliseconds the receipt was observed.
        observed_at: u64,
    },
    /// The handoff completed: the destination's matching import is durable
    /// and applied. Persisted before the coordinator changes its in-memory
    /// owner map or exposes the destination.
    HandoffCompleted {
        /// Handoff ID.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region the entity left.
        source_region: String,
        /// Region the entity entered.
        target_region: String,
        /// Digest of the transferred payload.
        payload_digest: String,
        /// Owner epoch at the destination (`prior + 1`).
        new_owner_epoch: u64,
        /// Region-map epoch after this completion.
        new_region_map_epoch: u64,
        /// Program module hash the transfer ran under.
        program_module_hash: String,
        /// Versioned payload schema name.
        payload_schema: String,
    },
    /// A handoff was cancelled. Legal only before the source export
    /// committed; the coordinator must first prove no export exists.
    HandoffCancelled {
        /// Handoff ID.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Why the handoff was cancelled.
        reason: String,
    },
}

/// The basis on which the module trust gate admitted a module (#1069).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustBasis {
    /// A publisher signature verified against the configured trust list.
    SignatureVerified,
    /// No trust list is configured, so the gate admits any module.
    NoTrustList,
    /// The server compiled the module from source itself, so the upload
    /// trust gate does not apply.
    ServerCompiled,
}

/// A trust gate decision recorded in the journal as part of the
/// install event it admitted (#1069): the basis, and the publisher key
/// when one verified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustDecision {
    /// The basis on which the module was admitted.
    pub basis: TrustBasis,
    /// Hex public key of the verifying publisher, if any.
    pub publisher: Option<String>,
}

/// A capability authorization denial (design #835): the typed root of
/// every error the capability check returns, so callers of the
/// authorized operation commands (`InstallProgramAuthorized`,
/// `RevokeAuthorized`) can classify a denial from the command's own
/// error — told apart from journal, actor, and program failures —
/// without matching message text. The message stays the non-oracular
/// denial text the actor has always returned: one wording for invalid,
/// expired, revoked, and insufficient capabilities alike.
#[derive(Debug)]
pub struct CapabilityDenied {
    message: &'static str,
}

impl CapabilityDenied {
    pub(crate) const fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl std::fmt::Display for CapabilityDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for CapabilityDenied {}

/// Whether an error returned by a world operation is a capability
/// authorization denial (see [`CapabilityDenied`]).
#[must_use]
pub fn is_capability_denied(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<CapabilityDenied>().is_some())
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
        /// The capability this one was delegated from, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<u64>,
        /// Unix milliseconds at which the capability stops working.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<u64>,
        /// Committed chat and input actions the capability may take. Host-local:
        /// only a capability the host issued has one, never a delegation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uses: Option<u32>,
        /// Actions already taken, written at checkpoint so counts survive compaction.
        /// Host-local, like `uses`.
        #[serde(default)]
        used: u32,
        /// Account the capability acts for; its ceiling and friendships bound it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
    },
    /// A capability was revoked, with every capability delegated from it.
    /// Replay revokes the same tree.
    Revoked {
        /// Participant whose capability was revoked.
        participant_id: u64,
        /// The moderator participant who revoked it, when the revoke came
        /// from a moderator scope rather than the admin. `None` means admin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        by: Option<u64>,
    },
    /// A builder-scoped participant installed a program. Journal-only: it is
    /// the audit record for the builder authority, written alongside the
    /// `ProgramInstalled` event that carries the install itself.
    BuilderInstalled {
        /// The builder participant who installed the program.
        by: u64,
        /// The hash of the installed module.
        module_hash: String,
        /// The seed the program was installed with.
        seed: u64,
    },
    /// The world was forked from another world (F12.2). Journal-only: the
    /// copied entries already reproduce the parent's state, and this entry
    /// records the origin so a fork can trace its provenance.
    Forked {
        /// The world this one was forked from.
        from_world_id: String,
        /// The content hash of the module at the fork point.
        from_module_hash: String,
        /// Whether the fork was marked reusable as a template.
        template: bool,
        /// The sequence the parent had reached when the fork was taken.
        at_sequence: u64,
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
    /// The world's capability ceiling, written whole on every change. Absent
    /// means the whole catalog. Checkpoints carry it in `caps_ceiling`.
    CapsCeiling {
        /// Capability names the world allows.
        ceiling: Vec<String>,
    },
    /// The world's requested execution profile. Limits are resolved against
    /// the current server hard ceilings when a program instance is created.
    ExecutionProfile {
        /// Per-world resource requests; omitted fields inherit server limits.
        profile: crate::world_limits::WorldExecutionProfile,
    },
    /// The world's isolation, written whole on every change. Absent means
    /// [`Isolation::Isolated`].
    Isolation {
        /// The new isolation.
        isolation: Isolation,
    },
    /// Whether other unisolated worlds may write here, set by a moderator or
    /// the admin. `enabled: false` also clears a force. Absent means off.
    CrossWorldWrite {
        /// The new setting.
        enabled: bool,
    },
    /// The admin forced cross-world writes on. Absent means not forced.
    CrossWorldWriteForced,
    /// The world's file scope, written whole on every change. Absent means confined.
    FileScope {
        /// The new scope.
        scope: FileScope,
    },
    /// Whether the admin let an isolated world write files. Absent means off.
    FileWriteOverride {
        /// The new setting.
        enabled: bool,
    },
    /// A `schedule.at` request was accepted: `id` fires once, at `at_ms` (unix
    /// milliseconds). Written before the request's result is delivered.
    ScheduleSet {
        /// The schedule's id, unique among pending schedules.
        id: String,
        /// When it fires, in unix milliseconds.
        at_ms: u64,
    },
    /// A pending schedule was cancelled before it fired.
    ScheduleCancelled {
        /// The cancelled schedule's id.
        id: String,
    },
    /// A pending schedule fired. It follows the fire's `System` entry, which
    /// the program accepted first, so the schedule is consumed only once its
    /// delivery is durable.
    ScheduleFired {
        /// The fired schedule's id.
        id: String,
    },
    /// An application-owned entity was exported to another region shard
    /// (#3870 phase 1, design #2818 §3). Written persist-before-apply: once
    /// this entry is durable the source is fenced for `entity_ref` and this
    /// entry is the durable recovery copy of the payload until the
    /// coordinator journals completion.
    EntityExported {
        /// Coordinator-generated handoff ID (hex); the receipt key.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region exporting the entity.
        source_region: String,
        /// Region importing the entity.
        target_region: String,
        /// Owner epoch at the source before this handoff.
        prior_owner_epoch: u64,
        /// The host-originated system event removing the entity; replay
        /// applies it from this same durable fact.
        source_event: WorldEvent,
        /// Versioned payload schema name.
        payload_schema: String,
        /// Program module hash the destination must match.
        program_module_hash: String,
        /// The complete entity snapshot (bounded by the transfer payload
        /// cap; the cap itself is enforced from phase 2 of #3870).
        payload: Vec<u8>,
        /// Digest of `payload`.
        payload_digest: String,
    },
    /// An application-owned entity was imported from another region shard
    /// (#3870 phase 1, design #2818 §3). Written persist-before-apply; the
    /// durable receipt for `handoff_id`. An exact retry returns the
    /// existing receipt; the same ID with a different digest is a hard
    /// conflict (phase 3 of #3870).
    EntityImported {
        /// Coordinator-generated handoff ID (hex); the dedupe key.
        handoff_id: String,
        /// Application-defined stable entity reference.
        entity_ref: String,
        /// Region that exported the entity.
        source_region: String,
        /// Region importing the entity.
        target_region: String,
        /// Owner epoch at the source before this handoff.
        prior_owner_epoch: u64,
        /// Owner epoch at this destination (`prior_owner_epoch + 1`).
        new_owner_epoch: u64,
        /// Program module hash, matched against the export before commit.
        program_module_hash: String,
        /// Versioned payload schema name.
        payload_schema: String,
        /// Digest of the imported payload.
        payload_digest: String,
        /// The host-originated system event adding the entity (it carries
        /// the payload, hex-encoded, per design #2818 §3).
        destination_event: WorldEvent,
    },
    /// A write lease was granted while no holder stood (design #2819,
    /// phase 2 of issue #3804). Authority-only: the append authority
    /// writes this, never a lease holder on its own behalf. `epoch` is
    /// the resulting epoch (`previous + 1`).
    LeaseGranted {
        /// The new holder's authenticated identity.
        holder: String,
        /// The resulting lease epoch.
        epoch: u64,
        /// When the granted term expires, in unix milliseconds on the
        /// authority's clock; absent for an untermed holder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_ms: Option<u64>,
    },
    /// The current holder's lease term was renewed by the authority
    /// (design #2819). The epoch is unchanged.
    LeaseRenewed {
        /// The holder whose term was renewed.
        holder: String,
        /// The (unchanged) lease epoch.
        epoch: u64,
        /// The new term expiry, in unix milliseconds.
        expires_at_ms: u64,
    },
    /// The write lease was handed from one holder to another
    /// (design #2819). This entry's journal position is the ownership
    /// boundary: before it only the old epoch passes the append gate,
    /// after it only the new one. `epoch` is the resulting epoch.
    LeaseHandedOff {
        /// The holder giving up execution.
        from: String,
        /// The holder taking over execution.
        to: String,
        /// The resulting lease epoch.
        epoch: u64,
        /// When the new holder's term expires, in unix milliseconds;
        /// absent for an untermed holder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_ms: Option<u64>,
    },
    /// The write lease was revoked by the authority (design #2819):
    /// the former holder is fenced at the resulting epoch and no
    /// holder stands until the next grant.
    LeaseRevoked {
        /// The holder whose lease was revoked.
        former_holder: String,
        /// The resulting lease epoch.
        epoch: u64,
        /// Why the lease was revoked.
        reason: String,
    },
    /// An authenticated append attempt was refused by the append gate
    /// and the authority journaled the attempt as an audit record
    /// (design #2819). The attempted entry itself was not applied.
    /// Control metadata: replay rebuilds the audit, never the program.
    LeaseWriteRejected {
        /// The authenticated writer identity that was refused.
        writer: String,
        /// The epoch the writer presented.
        presented_epoch: u64,
        /// The client operation ID the writer presented.
        operation_id: String,
        /// The authority's epoch when the attempt was refused.
        observed_epoch: u64,
        /// Why the attempt was refused.
        reason: LeaseRejectionReason,
        /// When the authority received the attempt, in unix
        /// milliseconds on the authority's clock.
        received_at_ms: u64,
    },
    /// One UTF-8-safe part of a compacted program snapshot. The complete set
    /// immediately precedes its checkpoint and is keyed by that checkpoint's
    /// sequence.
    CheckpointSnapshotPart {
        /// Sequence of the checkpoint these bytes belong to.
        sequence: u64,
        /// Zero-based position within this part set.
        part_index: u32,
        /// Number of parts in this set.
        part_count: u32,
        /// UTF-8 snapshot bytes for this part.
        bytes: String,
    },
    /// One bounded part of the retained participant-event window.
    CheckpointEventsPart {
        /// Sequence of the checkpoint these events belong to.
        sequence: u64,
        /// Zero-based position within this part set.
        part_index: u32,
        /// Number of parts in this set.
        part_count: u32,
        /// Ordered events in this part.
        events: Vec<WorldEvent>,
    },
    /// One bounded part of the retained admin-history window.
    CheckpointAdminPart {
        /// Sequence of the checkpoint these events belong to.
        sequence: u64,
        /// Zero-based position within this part set.
        part_index: u32,
        /// Number of parts in this set.
        part_count: u32,
        /// Ordered events in this part.
        events: Vec<WorldEvent>,
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
        /// Provenance of the active module, if it was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<ModuleProvenance>,
        /// Requested resource profile. Absent in older checkpoints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_profile: Option<crate::world_limits::WorldExecutionProfile>,
        /// The world's capability ceiling. Absent in older checkpoints, and
        /// when it is the whole catalog.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caps_ceiling: Option<Vec<String>>,
        /// Pending schedules as `(id, at_ms)`. Absent when none are pending,
        /// and it must match the `ScheduleSet` entries before it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schedules: Option<Vec<(String, u64)>>,
        /// Active region-seam handoffs (#3870 phase 1, design #2818 §3):
        /// export fences/payloads, import receipts/dedupe keys and owner
        /// epochs, so compaction cannot resurrect an exported entity or
        /// forget an import. Absent when no handoff is active, and in
        /// checkpoints written before journal version 3.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transfers: Option<Vec<HandoffTransferState>>,
        /// The journal-resident write lease as of this checkpoint
        /// (design #2819, phase 2 of issue #3804): current holder,
        /// epoch and term, the full transition history, and the
        /// rejection audit, so compaction cannot reset the epoch or
        /// lose proof of ownership. Absent when the lease is still the
        /// bootstrap state, and in checkpoints written before journal
        /// version 7.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lease: Option<WriteLease>,
        /// The program's own serialization of its state.
        snapshot: String,
        /// Recent events kept for reconnect catch-up.
        events: Vec<WorldEvent>,
        /// Ordered sequenced and host-originated events retained for the
        /// admin-only fixture export. Absent in older checkpoints.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        admin_history: Option<JournalAdminHistory>,
        /// Participants whose join the program has seen without a later leave.
        /// `Some(empty)` records a known-empty set; `None` is reserved for
        /// legacy checkpoints and triggers the documented migration fallback.
        /// See the journal-version contract.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open_presence: Option<Vec<u64>>,
    },
}

/// The bounded ordered event window carried across journal compaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JournalAdminHistory {
    pub(crate) events: Vec<WorldEvent>,
    pub(crate) complete: bool,
}

/// Version written in a new journal's [`JournalEntry::Created`] header.
///
/// Journal upgrade/rollback contract (issue #227): a journal is created at
/// version 1 and stays at version 1 until it contains an entry kind a
/// version-1 binary cannot read. New entry kinds raise the header to the
/// highest required version *before* the entry is appended, by atomically
/// rewriting the journal (sibling temp file, fsync, rename), so a failed bump
/// leaves the old journal untouched. The first such entries were an
/// [`JournalEntry::ExecutionProfile`], a region-handoff entry, an entry
/// carrying a per-world total memory budget, and a checkpoint carrying
/// open program-visible presence. A binary refuses any journal newer than
/// [`MAX_JOURNAL_VERSION`] at the header check in [`WorldCore::restore`],
/// never mid-read. New binaries accept versions 1..=[`MAX_JOURNAL_VERSION`],
/// so existing journals stay readable and a journal that never needs a newer
/// entry kind is never migrated.
pub const JOURNAL_VERSION: u32 = 1;

/// Header version of a journal that contains a
/// [`JournalEntry::ExecutionProfile`] or [`JournalEntry::BuilderInstalled`]:
/// entry kinds a version-1 binary cannot read. See the contract on
/// [`JOURNAL_VERSION`].
pub const JOURNAL_VERSION_EXECUTION_PROFILE: u32 = 2;

/// Header version of a journal that contains a region-seam handoff entry
/// ([`JournalEntry::EntityExported`] / [`JournalEntry::EntityImported`], an
/// entity-addressed [`WorldEventKind::EntityInput`] event, or a checkpoint
/// carrying handoff transfers): the first entry kinds a version-2 binary
/// cannot read (#3870 phase 1, design #2818 §3). See the contract on
/// [`JOURNAL_VERSION`].
pub const JOURNAL_VERSION_REGION_HANDOFF: u32 = 3;

/// Header version of a journal that persists a per-world total memory
/// budget (#557 Phase 1, design `doc/2026-11-01_design_id557.md`): an
/// [`JournalEntry::ExecutionProfile`] or [`JournalEntry::Checkpoint`]
/// whose profile sets `total_memory_bytes`. An older binary would parse
/// the entry yet silently ignore the budget it cannot account for, so the
/// budget requires a version the old binary refuses at the header check.
/// Profiles and checkpoints without a budget keep their existing version.
pub const JOURNAL_VERSION_WORLD_BUDGET: u32 = 4;

/// Header version of a checkpoint that carries open program-visible presence.
/// Version-4 binaries would ignore the field and fail to close those joins on
/// restart, so such checkpoints must be refused by them. This follows the
/// world-budget journal version so both features retain distinct contracts.
/// (Renumbered from 4 to 5 when this branch merged main: main had already
/// assigned version 4 to the world-budget contract above.)
pub const JOURNAL_VERSION_OPEN_PRESENCE: u32 = 5;

/// Header version required by compacted checkpoint part entries. Older
/// binaries refuse these entries at the header check instead of silently
/// restoring an incomplete checkpoint.
pub const JOURNAL_VERSION_CHECKPOINT_PARTS: u32 = 6;

/// Header version of a journal that contains a native-spill write
/// lease entry ([`JournalEntry::LeaseGranted`] and its siblings), or a
/// checkpoint carrying a non-bootstrap [`WriteLease`] (design #2819,
/// phase 2 of issue #3804). Version-6 binaries cannot read the lease
/// records and would restore a world with no fencing state, so they
/// must refuse the journal at the header check.
pub const JOURNAL_VERSION_WRITE_LEASE: u32 = 7;

/// Newest journal version this binary can read. Journals with a higher
/// header version are refused at the header check, before any entry is
/// applied.
pub const MAX_JOURNAL_VERSION: u32 = JOURNAL_VERSION_WRITE_LEASE;

/// The header version a journal must carry once it contains `entry`: version 2
/// for execution-profile entries, version 3 for region-handoff entries,
/// version 4 for entries persisting a per-world total memory budget, version 5
/// for checkpoints with open-presence metadata, and version 6 for checkpoint
/// part entries. Checkpoints combine the requirements of their transfers,
/// retained events, profile and presence field.
#[must_use]
pub fn journal_version_required_by(entry: &JournalEntry) -> u32 {
    // A persisted budget always requires the budget version, on top of any
    // version the entry kind itself already requires.
    let budget_version = |profile: Option<&crate::world_limits::WorldExecutionProfile>| {
        if profile.is_some_and(|p| p.total_memory_bytes.is_some()) {
            JOURNAL_VERSION_WORLD_BUDGET
        } else {
            JOURNAL_VERSION
        }
    };
    match entry {
        JournalEntry::EntityExported { .. } | JournalEntry::EntityImported { .. } => {
            JOURNAL_VERSION_REGION_HANDOFF
        }
        JournalEntry::LeaseGranted { .. }
        | JournalEntry::LeaseRenewed { .. }
        | JournalEntry::LeaseHandedOff { .. }
        | JournalEntry::LeaseRevoked { .. }
        | JournalEntry::LeaseWriteRejected { .. } => JOURNAL_VERSION_WRITE_LEASE,
        JournalEntry::CheckpointSnapshotPart { .. }
        | JournalEntry::CheckpointEventsPart { .. }
        | JournalEntry::CheckpointAdminPart { .. } => JOURNAL_VERSION_CHECKPOINT_PARTS,
        JournalEntry::Committed { event } | JournalEntry::System { event } => match &event.kind {
            WorldEventKind::EntityInput { .. } => JOURNAL_VERSION_REGION_HANDOFF,
            _ => JOURNAL_VERSION,
        },
        // A checkpoint stands in for the entries it compacted, so it must
        // require every version its contents require: its transfer list,
        // and its retained events — a retained `EntityInput` event is a
        // version-3 kind even when no handoff is active (Codex review on
        // PR #3922).
        JournalEntry::Checkpoint {
            transfers,
            lease,
            events,
            open_presence,
            execution_profile,
            ..
        } => {
            let base = if lease.is_some() {
                JOURNAL_VERSION_WRITE_LEASE
            } else if open_presence.is_some() {
                JOURNAL_VERSION_OPEN_PRESENCE
            } else if transfers.is_some()
                || events
                    .iter()
                    .any(|event| matches!(event.kind, WorldEventKind::EntityInput { .. }))
            {
                JOURNAL_VERSION_REGION_HANDOFF
            } else {
                JOURNAL_VERSION
            };
            base.max(budget_version(execution_profile.as_ref()))
        }
        JournalEntry::ExecutionProfile { profile } => {
            JOURNAL_VERSION_EXECUTION_PROFILE.max(budget_version(Some(profile)))
        }
        JournalEntry::BuilderInstalled { .. } => JOURNAL_VERSION_EXECUTION_PROFILE,
        _ => JOURNAL_VERSION,
    }
}

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

    /// Capture the exact durable journal bytes and their header version.
    /// Implementations that do not own a readable journal may leave this
    /// unsupported; the replication feed is available only for durable
    /// journals that can provide an atomic snapshot from the world actor.
    fn replication_snapshot(&mut self) -> Result<JournalSnapshot> {
        bail!("this journal cannot provide a replication snapshot")
    }

    /// Capture a lightweight, readable source for the initial replication
    /// feed. Durable file journals should pin the file and its current length
    /// here, then defer reading the bytes until the network layer requests it.
    /// Journals that cannot provide a deferred source should leave this
    /// unsupported rather than materializing the full image in the actor.
    fn replication_snapshot_source(&mut self) -> Result<JournalSnapshotSource> {
        bail!("this journal cannot provide a deferred replication snapshot source")
    }

    /// Current journal header version, when the implementation can report it.
    fn replication_header_version(&self) -> Option<u32> {
        None
    }

    /// Durably append a group of entries as one all-or-nothing transaction.
    /// Journals that cannot guarantee atomicity must leave the default in
    /// place; an install requiring paired event and audit records will then
    /// fail closed instead of falling back to separate appends.
    ///
    /// # Errors
    ///
    /// Any I/O failure, or if this journal does not support atomic groups.
    fn append_transaction(&mut self, _entries: &[JournalEntry]) -> Result<()> {
        bail!("this journal does not support atomic entry transactions")
    }

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

/// A byte-exact, newline-terminated journal image captured by the actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalSnapshot {
    /// Version in the journal's `Created` header.
    pub header_version: u32,
    /// Exact JSONL bytes, including every entry's newline.
    pub bytes: Vec<u8>,
}

/// A captured snapshot reader. File-backed journals keep only a pinned file
/// handle and the captured byte length in the actor; the network layer reads
/// the bytes after the actor has resumed handling commands.
pub struct JournalSnapshotSource {
    /// Version in the journal's `Created` header.
    pub header_version: u32,
    /// Number of bytes belonging to this captured image.
    pub byte_len: u64,
    reader: Box<dyn Read + Send>,
}

impl std::fmt::Debug for JournalSnapshotSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JournalSnapshotSource")
            .field("header_version", &self.header_version)
            .field("byte_len", &self.byte_len)
            .finish_non_exhaustive()
    }
}

impl JournalSnapshotSource {
    pub(crate) fn from_reader<R>(header_version: u32, byte_len: u64, reader: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self {
            header_version,
            byte_len,
            reader: Box::new(reader),
        }
    }

    /// Read exactly the captured image. This consumes the source, so its file
    /// handle and the resulting byte buffer have one clear owner at a time.
    ///
    /// # Errors
    ///
    /// If the source cannot be read completely, is too large for this target,
    /// or does not end at a journal-entry boundary.
    pub fn read(self) -> Result<JournalSnapshot> {
        let capacity = usize::try_from(self.byte_len).context("journal snapshot is too large")?;
        let mut bytes = Vec::with_capacity(capacity);
        self.reader
            .take(self.byte_len)
            .read_to_end(&mut bytes)
            .context("read captured journal snapshot")?;
        ensure!(
            bytes.len() == capacity,
            "captured journal snapshot changed while reading"
        );
        ensure!(
            bytes.is_empty() || bytes.ends_with(b"\n"),
            "captured journal snapshot has an unterminated final entry"
        );
        Ok(JournalSnapshot {
            header_version: self.header_version,
            bytes,
        })
    }
}

/// A durable change delivered to replication subscribers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicationUpdate {
    /// One entry appended and fsynced after the subscriber's snapshot.
    Append {
        /// Header version the appended entry is encoded under.
        header_version: u32,
        /// Exact serialized JSONL entry bytes, including its newline.
        entry: Vec<u8>,
    },
    /// The journal was compacted or its header version changed.
    Reset(JournalSnapshot),
    /// A reset became necessary but the journal image could not be captured.
    ResetFailed,
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
    /// Server-observed module origin, if this journal version recorded it.
    pub provenance: Option<ModuleProvenance>,
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
    /// Live presence reconstructed from journaled `players` events. Writer
    /// restore clears it after synthesizing the process-restart leaves;
    /// replica restore retains it at the verified cursor.
    pub presence: BTreeMap<u64, u64>,
}

/// The profile requested by the world admin and when it takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionProfileUpdate {
    /// Requested profile; this is not necessarily the active instance's limits.
    pub requested: crate::world_limits::WorldExecutionProfile,
    /// True when a program is currently running with its previously resolved limits.
    pub applies_on_next_program_install_or_process_restart: bool,
}

/// Structured records a program publishes: key to JSON value, key-ordered.
pub type Records = BTreeMap<String, serde_json::Value>;

pub(crate) fn presence_event(participant: u64, name: &str, joined: bool) -> String {
    serde_json::json!({
        "cap": "players",
        "event": if joined { "joined" } else { "left" },
        "participant": {"id": participant, "name": name},
    })
    .to_string()
}

pub(crate) fn presence_change(text: &str) -> Option<(u64, bool)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("cap")?.as_str()? != "players" {
        return None;
    }
    let joined = match value.get("event")?.as_str()? {
        "joined" => true,
        "left" => false,
        _ => return None,
    };
    Some((value.get("participant")?.get("id")?.as_u64()?, joined))
}

/// The host event a program receives when schedule `id` fires. `late` is set
/// only for a schedule the journal left overdue when the world reopened.
pub(crate) fn schedule_fired_event(id: &str, late: bool) -> String {
    let mut event = serde_json::json!({"cap": "schedule", "event": "fired", "id": id});
    if late {
        event["late"] = serde_json::Value::Bool(true);
    }
    event.to_string()
}

/// When the earliest pending schedule is due, as a monotonic deadline. Nothing
/// waits while the program cannot take a fire (`deliverable` is false): a due
/// schedule that cannot be delivered would otherwise wake the loop without end.
///
/// The deadline is anchored to `now_instant` (the injected clock's monotonic
/// face), so a manual clock advancing `now_ms` past a schedule also advances
/// this deadline.
pub(crate) fn next_schedule_wake(
    schedules: &BTreeMap<String, u64>,
    now_ms: u64,
    now_instant: std::time::Instant,
    deliverable: bool,
) -> Option<std::time::Instant> {
    if !deliverable {
        return None;
    }
    let earliest = schedules.values().min().copied()?;
    let wait = std::time::Duration::from_millis(earliest.saturating_sub(now_ms));
    Some(now_instant + wait)
}

/// The newest `count` chat lines in `events`, oldest first, each carrying its
/// sequence, participant id, and text. The retained-events buffer is itself
/// bounded by the world's limits, so a request can never see unbounded history.
pub(crate) fn chat_history(events: &VecDeque<WorldEvent>, count: usize) -> Vec<serde_json::Value> {
    let mut lines: Vec<serde_json::Value> = Vec::new();
    for event in events.iter().rev() {
        if lines.len() >= count {
            break;
        }
        if let WorldEventKind::Chat(text) = &event.kind {
            lines.push(serde_json::json!({
                "sequence": event.sequence,
                "participant": event.participant_id,
                "text": text,
            }));
        }
    }
    lines.reverse();
    lines
}

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
    // A duplicate key is a parse error (#6520): the caps below must see
    // the document as written, never a last-wins collapse of it.
    let value = id_core::json_unique::parse_json_without_duplicate_keys(text)
        .context("world records must be a JSON object")?;
    let serde_json::Value::Object(map) = value else {
        bail!("world records must be a JSON object");
    };
    let records: Records = map.into_iter().collect();
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
        let size = serde_json::to_vec(value).map_or(usize::MAX, |v| v.len());
        ensure!(
            size <= MAX_RECORD_VALUE_BYTES,
            "record {key:?} is {size} bytes; the limit is {MAX_RECORD_VALUE_BYTES}"
        );
    }
    Ok(records)
}

/// Records of `program`, validated; `Ok(None)` when it keeps none.
pub(crate) fn program_records(program: &mut dyn WorldProgram) -> Result<Option<Records>> {
    program
        .records()?
        .map(|text| parse_records(&text))
        .transpose()
}

/// Behavior supplied by the running world program.
///
/// Implementations execute inside the actor's serial command loop. They must
/// not perform network or filesystem I/O. An error from `update` rejects the
/// candidate input before it is committed to the event log. An error from
/// `view` or `layout` does not poison the world: the host renders an error
/// screen naming the failing call, keeps the program's state, and retries on
/// the next event (see F11.17).
pub trait WorldProgram: Send + 'static {
    /// Apply an opaque participant input. Chat is managed by the host and is
    /// not passed to the game program.
    fn update(&mut self, _event: &WorldEvent) -> Result<()> {
        Ok(())
    }

    /// Purely validate and prepare one side of an application-owned entity
    /// transfer. Implementations return the complete entity snapshot and a
    /// deterministic removal event for [`EntityTransferRequest::Export`], or
    /// validate the supplied envelope and return a deterministic insertion
    /// event for [`EntityTransferRequest::Import`]. This hook must not mutate
    /// program state; the actor journals the returned event before calling
    /// [`Self::update`]. Programs without a transfer adapter remain supported
    /// and reject transfer requests by default.
    fn prepare_entity_transfer(
        &self,
        _request: &EntityTransferRequest,
    ) -> Result<EntityTransferPlan> {
        bail!("world program does not support entity transfer")
    }

    /// Return this participant's presentation, or `None` for a chat-only
    /// world. The presentation must be UTF-8 and fit `presentation_bytes`.
    fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
        Ok(None)
    }

    /// Return this participant's presentation laid out for a `width` by
    /// `height` viewport. Defaults to [`Self::view`]; a program that can reflow
    /// overrides it. Rendering must not change state.
    fn layout(
        &mut self,
        viewer: &Participant,
        _width: u16,
        _height: u16,
    ) -> Result<Option<String>> {
        self.view(viewer)
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
pub(crate) struct EmptyWorldProgram;

impl WorldProgram for EmptyWorldProgram {}

/// Snapshot returned on join or reconnect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldSnapshot {
    /// Stable world identifier.
    pub world_id: String,
    /// Content hash of the active program, if this world executes one.
    pub active_module_hash: Option<String>,
    /// Provenance for the active module, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_module_provenance: Option<ModuleProvenance>,
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

/// One actor-consistent snapshot for an admin-authorized fixture export.
/// This type is crate-private so history cannot become a participant API.
#[derive(Clone, Debug)]
pub(crate) struct AdminEventHistory {
    pub(crate) world_id: String,
    pub(crate) module_hash: Option<String>,
    pub(crate) current_sequence: u64,
    pub(crate) complete: bool,
    pub(crate) events: Vec<WorldEvent>,
    pub(crate) records: Records,
}

/// The one projection of an event onto a viewer holding `scopes`.
///
/// Every path that hands an event to a participant — the snapshot, catch-up
/// pages and the live stream — goes through it.
///
/// - A chat line is `None` unless `scopes` may read chat (`CHAT` or
///   `HISTORY`): a join-only participant sees the world, not the talk.
/// - A `ProgramInstalled` event keeps its module hash — joiners need it to
///   fetch the module — but its install seed is zeroed (#240), because a
///   program whose hidden state derives from `init(seed)` must not leak that
///   state to every participant.
/// - A host-originated `System` event is never part of a participant stream.
/// - Anything else is returned as is.
///
/// The journal, the core's retained events and the admin paths keep the real
/// events; replay that needs the seed uses the seed the admin installed with,
/// through the journal/admin path.
#[must_use]
pub fn project_event_for(scopes: WorldScopes, event: &WorldEvent) -> Option<WorldEvent> {
    match &event.kind {
        WorldEventKind::Chat(_) if !scopes.reads_chat() => None,
        WorldEventKind::System { .. } => None,
        WorldEventKind::ProgramInstalled { .. } => {
            // Mutate in place so every other field, including the observed
            // build provenance, is kept as it is.
            let mut shown = event.clone();
            if let WorldEventKind::ProgramInstalled { seed, .. } = &mut shown.kind {
                *seed = 0;
            }
            Some(shown)
        }
        _ => Some(event.clone()),
    }
}

/// Limits on what a capability may do before it stops working.
///
/// Only the host sets `uses`, on a capability it issues. A delegation may set
/// `expires_at` and never `uses` (see [`WorldCore::attenuate`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapabilityBounds {
    /// Unix milliseconds at which the capability stops working.
    pub expires_at: Option<u64>,
    /// Committed chat and input actions the capability may take. Enforced by
    /// the host alone, from its own journal.
    pub uses: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct CapabilityRecord {
    digest: [u8; 32],
    participant: Participant,
    scopes: WorldScopes,
    parent: Option<u64>,
    revoked: bool,
    bounds: CapabilityBounds,
    used: u32,
    subject: Option<String>,
}
