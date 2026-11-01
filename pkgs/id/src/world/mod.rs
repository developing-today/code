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
mod metrics;
mod util;

#[cfg(test)]
mod tests;

pub(crate) use actor::{WorldActor, WorldCommand};
pub use clock::{Clock, ManualClock, SystemClock};
pub use core::WorldCore;
pub use handle::WorldHandle;
pub use metrics::{HistogramSnapshot, MetricsSnapshot, WorldMetrics};
pub use util::{EventPage, hex_encode, publish_records};
pub(crate) use util::{
    constant_time_eq, hex_decode_32, sanitize_chat, sanitize_display_name, token_digest,
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

/// Bounded limits for one world.
#[derive(Clone, Debug)]
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
            retained_events: 1024,
            presentation_bytes: 8192,
            snapshot_bytes: 512 * 1024,
            checkpoint_every: 1000,
            min_tick_ms: 1000,
            grant_on_use: false,
            server_caps: crate::world_caps::catalog_names(),
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
        match cap {
            "chat.say" => self.contains(Self::CHAT),
            // Reading chat needs only the right to read it (`CHAT` or
            // `HISTORY`), the same rule the event stream applies.
            "chat.history" => self.reads_chat(),
            _ => self.contains(Self::JOIN),
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
        /// Journal/admin only: join projections redact it to zero (#240).
        #[serde(default)]
        seed: u64,
        /// Server-observed origin metadata. Absent on older journal entries.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<ModuleProvenance>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
        /// The program's own serialization of its state.
        snapshot: String,
        /// Recent events kept for reconnect catch-up.
        events: Vec<WorldEvent>,
    },
}

/// Version written in a new journal's [`JournalEntry::Created`] header.
///
/// Journal upgrade/rollback contract (issue #227): a journal is created at
/// version 1 and stays at version 1 until it contains an entry kind a
/// version-1 binary cannot read. The first such entry — today only
/// [`JournalEntry::ExecutionProfile`] — bumps the header to
/// [`JOURNAL_VERSION_EXECUTION_PROFILE`] *before* the entry is appended, by
/// atomically rewriting the journal (sibling temp file, fsync, rename), so
/// a failed bump leaves the old journal untouched. A binary refuses any
/// journal newer than [`MAX_JOURNAL_VERSION`] at the header check in
/// [`WorldCore::restore`], never mid-read: a version-1 binary opening a
/// version-2 journal therefore fails early and clearly instead of dying on
/// the profile entry it cannot deserialize. New binaries accept versions
/// 1..=[`MAX_JOURNAL_VERSION`], so existing journals stay readable
/// (forward compatibility), and a journal that never writes a profile
/// entry is never migrated at all.
pub const JOURNAL_VERSION: u32 = 1;

/// Header version of a journal that contains a
/// [`JournalEntry::ExecutionProfile`]: the first entry kind a version-1
/// binary cannot read. See the contract on [`JOURNAL_VERSION`].
pub const JOURNAL_VERSION_EXECUTION_PROFILE: u32 = 2;

/// Newest journal version this binary can read. Journals with a higher
/// header version are refused at the header check, before any entry is
/// applied.
pub const MAX_JOURNAL_VERSION: u32 = JOURNAL_VERSION_EXECUTION_PROFILE;

/// The header version a journal must carry once it contains `entry`:
/// [`JOURNAL_VERSION_EXECUTION_PROFILE`] for the entry kinds a version-1
/// binary cannot read, [`JOURNAL_VERSION`] for everything else.
#[must_use]
pub const fn journal_version_required_by(entry: &JournalEntry) -> u32 {
    match entry {
        JournalEntry::ExecutionProfile { .. } => JOURNAL_VERSION_EXECUTION_PROFILE,
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
/// - Anything else is returned as is.
///
/// The journal, the core's retained events and the admin paths keep the real
/// events; replay that needs the seed uses the seed the admin installed with,
/// through the journal/admin path.
#[must_use]
pub fn project_event_for(scopes: WorldScopes, event: &WorldEvent) -> Option<WorldEvent> {
    match &event.kind {
        WorldEventKind::Chat(_) if !scopes.reads_chat() => None,
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
