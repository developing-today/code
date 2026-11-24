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
mod admission_lease;
pub(crate) mod audit;
mod clock;
mod coordinator;
mod core;
mod handle;
mod handoff_operations;
pub mod hotness;
mod http_fetch;
mod journal;
mod metrics;
mod provider_broker;
mod replica;
mod replication;
mod replication_receiver;
pub mod signed;
pub mod spill_gate;
pub mod spill_pool;
mod upgrade;
mod util;

#[cfg(test)]
mod tests;

pub(crate) use actor::{
    AUDIT_TRACE_TARGET, AuditRecord, WorldActor, WorldCommand, emit_audit_record,
    next_audit_correlation_id,
};
pub use admission_lease::{
    ADMISSION_LEASE_DOMAIN, ADMISSION_LEASE_VERSION, AdmissionLease, AdmissionLeaseError,
    MAX_ADMISSION_LEASE_DURATION_MS, ReplicaAdmissionGate, SignedAdmissionLease,
};
pub use audit::{GrantsState, WorldAuditLog, export_otlp_json, replay_grants};
pub use clock::{Clock, ManualClock, SystemClock};
pub use coordinator::{
    ActiveHandoff, Coordinator, CoordinatorError, EntityInTransit, EntityOwner, ExportReceipt,
    HandoffCompletion, HandoffStage, ImportReceipt, ObserveOutcome, ReconcileOutcome,
    STUCK_THRESHOLD_MS, TransitStage,
};
pub(crate) use core::AdminHistoryByteLedger;
pub use core::WorldCore;
pub use handle::WorldHandle;
pub use handoff_operations::{
    ClientRoute, HandoffOperations, HandoffOpsEntry, HandoffOpsSnapshot, OperationKind, OpsError,
    RouteDecision, StaleRoute,
};
pub use hotness::{FiredSignal, HotnessBand, HotnessReport, HotnessSignal};
/// The F12.14 portal policy types (code#1068), re-exported so the
/// handle/hub portal surface names the same types the kernel checks.
pub use id_caps::portal::{PortalGrantState, PortalGrants, PortalRefusal};
pub use journal::{
    CoordinatorJournalEntry, JOURNAL_VERSION, JOURNAL_VERSION_BLOB,
    JOURNAL_VERSION_CHECKPOINT_PARTS, JOURNAL_VERSION_EXECUTION_PROFILE,
    JOURNAL_VERSION_JOIN_TIMES, JOURNAL_VERSION_KV, JOURNAL_VERSION_LEASE_POSITION,
    JOURNAL_VERSION_NOTIFY, JOURNAL_VERSION_OPEN_PRESENCE, JOURNAL_VERSION_PORTALS,
    JOURNAL_VERSION_PROGRAM_BUNDLE, JOURNAL_VERSION_REGION_HANDOFF,
    JOURNAL_VERSION_SCHEDULE_RECURRENCE, JOURNAL_VERSION_SIGNED_PLAY,
    JOURNAL_VERSION_WORLD_BUDGET, JOURNAL_VERSION_WRITE_LEASE, JournalAckMode,
    JournalAdminHistory, JournalEntry, JournalSignedMove, JournalSnapshot, JournalSnapshotSource,
    MAX_JOURNAL_VERSION, TrustBasis, TrustDecision, WorldJournal, journal_version_required_by,
};
pub use metrics::{
    HistogramSnapshot, MetricsSnapshot, RegionWrites, WorldMetrics, WriteLocalitySnapshot,
};
pub(crate) use provider_broker::{BeginRequest, ProviderBroker};
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
    blob_decode, blob_encode, blob_id, constant_time_eq, hex_decode, hex_decode_32,
    is_pre_fix_mint_display_name, legacy_sanitize_display_name, pre_fix_sanitize_display_name,
    sanitize_chat, sanitize_display_name, token_digest,
};

/// Identity of one live connection. It stays in memory only and is never part
/// of a world event or journal entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// When the world's journal acknowledges an append relative to the
    /// sync that makes it durable (#557). Runtime configuration, not
    /// journaled; the default [`JournalAckMode::Sync`] is the original
    /// durability-before-ack contract. The relaxed modes trade a
    /// bounded (or, for `Async`, sync-point-bounded) loss window for
    /// fewer syncs on the input ack path — see [`JournalAckMode`] for
    /// the guarantees and when to choose each.
    pub journal_ack_mode: JournalAckMode,
    /// Minimum tick interval a program may subscribe to, in milliseconds.
    /// Subscriptions below this floor are refused at subscription time.
    pub min_tick_ms: u64,
    /// Whether a world's policy grants a capability on its first use.
    pub grant_on_use: bool,
    /// Capabilities the server allows any world to use. Runtime configuration,
    /// not journaled; a world's own ceiling and grants only narrow it.
    pub server_caps: BTreeSet<String>,
    /// Hosts the `http.fetch` provider may fetch from (#1062): the
    /// admin allowlist. Runtime configuration, not journaled; empty
    /// by default, which refuses every fetch. Entries are matched
    /// exactly against the URL's normalized host — no wildcards, no
    /// implied subdomains. A bare host admits only the scheme's
    /// default port; `host:port` admits that port only.
    pub http_fetch_hosts: BTreeSet<String>,
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
            journal_ack_mode: JournalAckMode::Sync,
            min_tick_ms: 1000,
            grant_on_use: false,
            server_caps: crate::world_caps::default_ceiling_names(),
            http_fetch_hosts: BTreeSet::new(),
        }
    }
}

/// Permission scopes granted by a world capability. The definition
/// lives in `id-caps` ([`id_caps::world_scopes::WorldScopes`]) so
/// `id-directory` can name the vocabulary without depending on this
/// crate (crate split step 6a, #4409); re-exported here at its
/// historical path.
pub use id_caps::world_scopes::WorldScopes;

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

/// Outcome of submitting a deferred result to a pending form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormSubmissionStatus {
    /// The result matched the pending form and was delivered to the program.
    Accepted,
    /// The result was rejected with `bad_args` / `field_invalid`. The form
    /// stays pending under a fresh host token; the presenter re-presents it
    /// with that token and the server never retries on its own.
    Rejected {
        /// Token the presenter must echo in its next submission.
        presentation_token: String,
    },
    /// The pending form or its authority is no longer valid; the presenter
    /// should exit form mode rather than retrying the same response.
    Unavailable {
        /// Stable wire error code.
        error: &'static str,
        /// Stable reason explaining why the form cannot accept input.
        reason: &'static str,
    },
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
    /// A user-visible outbound notification from the world's program,
    /// sent through the `notify` provider (#1062, F12.5). Host-authored:
    /// `participant_id` is zero, the host commits it on the program's
    /// behalf once the grant/ceiling decision and the rate bound pass.
    /// Broadcast to participant sessions and retained like any committed
    /// event, but it is not a chat line: it never enters chat history,
    /// and the join projection shows it to every participant, not only
    /// chat readers. Journals carrying one require
    /// [`JOURNAL_VERSION_NOTIFY`].
    Notification {
        /// Notification text, UTF-8 and control-character sanitized,
        /// bounded by the world's `chat_bytes` limit.
        text: String,
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
    /// The module was sideloaded from a verified program bundle (#8280,
    /// design D4): the install record names the bundle's identity, ABI,
    /// lineage and signature. Journals carrying it need
    /// [`JOURNAL_VERSION_PROGRAM_BUNDLE`].
    Bundle(BundleProvenance),
}

impl ModuleProvenance {
    /// Whether this provenance is a program-bundle install, which an older
    /// binary cannot read.
    #[must_use]
    pub const fn is_bundle(&self) -> bool {
        matches!(self, Self::Bundle(_))
    }
}

/// How a bundle install follows later versions of its program (D4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPolicy {
    /// Exactly this content hash; nothing changes without a new install.
    #[default]
    Pinned,
    /// Discover later versions on this install's exact (author, name) line.
    Tracking,
}

impl InstallPolicy {
    /// Whether this install opts into update discovery for `candidate`.
    /// This does not authorize applying an update; that remains a separate
    /// owner decision/flow.
    #[must_use]
    pub fn tracks(
        self,
        installed: &crate::program_bundle::ProgramIdentity,
        candidate: &crate::program_bundle::ProgramIdentity,
    ) -> bool {
        self == Self::Tracking && installed.same_channel(candidate)
    }
}

/// The install record of a sideloaded program bundle (#8280).
///
/// It keeps what the verified bundle stated with the install event, so the
/// world's history says which program, by whom, forked from what, is
/// running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleProvenance {
    /// Provenance record schema, independent of the world journal version.
    pub schema_version: u8,
    /// The program's D1 identity; its content hash is the module hash.
    pub program: crate::program_bundle::ProgramIdentity,
    /// The guest ABI the bundle declared.
    pub abi: crate::program_bundle::AbiDescriptor,
    /// Ancestors, oldest first; empty for an original.
    pub lineage: Vec<crate::program_bundle::ProgramIdentity>,
    /// The author's licence statement, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    /// The verified bundle signature, lower-case hex.
    pub bundle_signature: String,
    /// How this install follows later versions.
    #[serde(default)]
    pub policy: InstallPolicy,
}

impl BundleProvenance {
    /// Whether a candidate belongs to this install's opted-in update channel.
    /// Fork lineage is deliberately not part of channel identity.
    #[must_use]
    pub fn tracks(&self, candidate: &crate::program_bundle::ProgramIdentity) -> bool {
        self.policy.tracks(&self.program, candidate)
    }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum LeaseRejectionReason {
    /// No holder stands while the authority is fencing and selecting a
    /// successor. The caller should retry with backoff against the pool.
    #[error("world write rejected: holder_lost (retryable)")]
    HolderLost,
    /// The writer is not the current lease holder in steady state.
    #[error("world write rejected: not_holder (retryable)")]
    NotHolder,
    /// The writer presented an epoch other than the current one.
    #[error("world write rejected: stale_epoch (retryable)")]
    StaleEpoch,
    /// The current lease term had expired on the authority's clock.
    #[error("world write rejected: expired (retryable)")]
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
    /// The transition's durable per-world journal position (#13711):
    /// strictly increasing across the lease history in journal order.
    /// `0` means unstamped: a transition recorded before positions
    /// existed. A checkpoint carries this whole history, so a
    /// position survives compaction exactly when its transition does.
    #[serde(default)]
    pub position: u64,
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
    /// The audit's durable per-world journal position (#13711);
    /// `0` means unstamped (pre-positions entry).
    #[serde(default)]
    pub position: u64,
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

    /// The highest stamped journal position in the lease history
    /// (#13711): the replay watermark — a successor that has
    /// replayed this history has replayed through this position.
    /// `0` when no transition or audit in the history carries a
    /// stamped position (a world that never spilled, or a legacy
    /// journal whose lease entries predate positions). History is
    /// append-only and positions strictly increase in journal order,
    /// so this is also the position the next live lease record
    /// must exceed.
    #[must_use]
    pub fn last_position(&self) -> u64 {
        self.history
            .iter()
            .map(|entry| match entry {
                WriteLeaseHistory::Transition(transition) => transition.position,
                WriteLeaseHistory::Rejection(rejection) => rejection.position,
            })
            .max()
            .unwrap_or(0)
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
        if self.holder.is_none() {
            return Err(LeaseRejectionReason::HolderLost);
        }
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

    /// A stamped journal position must advance past every stamped
    /// position already in the history (#13711); `0` is the
    /// unstamped legacy marker and neither advances the order nor
    /// violates it. Checked after the domain validation, so a bad
    /// epoch or holder reports its own error first.
    fn ensure_position_advances(&self, position: u64) -> Result<()> {
        ensure!(
            position == 0 || position > self.last_position(),
            "journal lease position {position} does not advance past {}",
            self.last_position()
        );
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
        position: u64,
    ) -> Result<()> {
        ensure!(
            self.holder.is_none(),
            "journal grants the write lease to {holder:?} while {:?} holds it",
            self.holder
        );
        self.advance_epoch(epoch, "grant")?;
        self.ensure_position_advances(position)?;
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
                position,
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
    pub fn apply_renew(
        &mut self,
        holder: &str,
        epoch: u64,
        expires_at_ms: u64,
        position: u64,
    ) -> Result<()> {
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
        self.ensure_position_advances(position)?;
        self.expires_at_ms = Some(expires_at_ms);
        self.history
            .push(WriteLeaseHistory::Transition(WriteLeaseTransition {
                kind: LeaseTransitionKind::Renewed,
                from: Some(holder.to_owned()),
                to: Some(holder.to_owned()),
                epoch,
                expires_at_ms: Some(expires_at_ms),
                reason: None,
                position,
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
        position: u64,
    ) -> Result<()> {
        ensure!(
            self.holder.as_deref() == Some(from),
            "journal hands the write lease off from {from:?}, held by {:?}",
            self.holder
        );
        self.advance_epoch(epoch, "handoff")?;
        self.ensure_position_advances(position)?;
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
                position,
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
    pub fn apply_revoke(
        &mut self,
        former_holder: &str,
        epoch: u64,
        reason: &str,
        position: u64,
    ) -> Result<()> {
        ensure!(
            self.holder.as_deref() == Some(former_holder),
            "journal revokes the write lease from {former_holder:?}, held by {:?}",
            self.holder
        );
        self.advance_epoch(epoch, "revocation")?;
        self.ensure_position_advances(position)?;
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
                position,
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
        self.ensure_position_advances(rejection.position)?;
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
                        transition.position,
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
                        transition.position,
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
                        transition.position,
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
                        transition.position,
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
    let value = id_core::json_unique::parse_json_without_duplicate_keys(text).ok()?;
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

/// A pending schedule (#1355): its next due time, and — for a recurring
/// schedule — the fixed interval between fires and how many fires remain
/// (the pending one included). A one-shot schedule is the same state with
/// no interval and one remaining fire. Recurrence is fixed-rate: after
/// firing at planned time `t` the next due time is `t + interval`, never
/// `now + interval`, so late delivery and restarts cannot drift a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSchedule {
    /// When the next fire is due, in unix milliseconds.
    pub(crate) at_ms: u64,
    /// Fixed interval between fires; `None` for a one-shot schedule.
    pub(crate) interval_ms: Option<u64>,
    /// Fires remaining, the pending one included. Always at least one.
    pub(crate) remaining: u32,
    /// The world the fire is delivered to; `None` delivers here.
    pub(crate) target: Option<String>,
}

impl PendingSchedule {
    /// A one-shot schedule due at `at_ms`.
    pub(crate) fn one_shot(at_ms: u64) -> Self {
        Self {
            at_ms,
            interval_ms: None,
            remaining: 1,
            target: None,
        }
    }

    /// The state after one fire: `Some` re-armed state while fires remain,
    /// `None` when the fire consumed the schedule.
    pub(crate) fn after_fire(self) -> Option<Self> {
        if self.remaining <= 1 {
            return None;
        }
        let interval = self.interval_ms?;
        let at_ms = self.at_ms.checked_add(interval)?;
        Some(Self {
            at_ms,
            interval_ms: Some(interval),
            remaining: self.remaining - 1,
            target: self.target,
        })
    }
}

/// Default fires-remaining for a schedule record that names none: one.
fn default_schedule_remaining() -> u32 {
    1
}

/// Whether a schedule record's remaining count is the one-shot default.
fn is_default_schedule_remaining(remaining: &u32) -> bool {
    *remaining == 1
}

/// One pending schedule as a checkpoint declares it (#1355). Serializes
/// in the legacy `(id, at_ms)` pair form while the schedule is a one-shot,
/// so a checkpoint of one-shot schedules stays byte-compatible with
/// journals written before journal version 8; a recurring schedule
/// serializes as an object. Deserialization accepts both forms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointSchedule {
    /// The schedule's id, unique among pending schedules.
    pub id: String,
    /// When the next fire is due, in unix milliseconds.
    pub at_ms: u64,
    /// Fixed interval between fires; `None` for a one-shot schedule.
    pub interval_ms: Option<u64>,
    /// Fires remaining, the pending one included.
    pub remaining: u32,
    /// The world the fire is delivered to; `None` delivers here.
    pub target: Option<String>,
}

impl serde::Serialize for CheckpointSchedule {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.interval_ms.is_none() && self.remaining == 1 && self.target.is_none() {
            return (self.id.clone(), self.at_ms).serialize(serializer);
        }
        #[derive(serde::Serialize)]
        struct Current<'a> {
            id: &'a str,
            at_ms: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            interval_ms: Option<u64>,
            #[serde(skip_serializing_if = "is_default_schedule_remaining")]
            remaining: u32,
            #[serde(skip_serializing_if = "Option::is_none")]
            target: Option<&'a str>,
        }
        Current {
            id: &self.id,
            at_ms: self.at_ms,
            interval_ms: self.interval_ms,
            remaining: self.remaining,
            target: self.target.as_deref(),
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for CheckpointSchedule {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Legacy((String, u64)),
            Current {
                id: String,
                at_ms: u64,
                #[serde(default)]
                interval_ms: Option<u64>,
                #[serde(default = "default_schedule_remaining")]
                remaining: u32,
                #[serde(default)]
                target: Option<String>,
            },
        }
        Ok(match Repr::deserialize(deserializer)? {
            Repr::Legacy((id, at_ms)) => Self {
                id,
                at_ms,
                interval_ms: None,
                remaining: 1,
                target: None,
            },
            Repr::Current {
                id,
                at_ms,
                interval_ms,
                remaining,
                target,
            } => Self {
                id,
                at_ms,
                interval_ms,
                remaining,
                target,
            },
        })
    }
}

/// One live entry of the world's `kv` store as a checkpoint declares
/// it (#1062). The writer also emits one `KvSet` journal entry per
/// live entry before the checkpoint, and replay refuses a checkpoint
/// whose declared entries disagree with them — the schedule treatment
/// ([`CheckpointSchedule`]) applied to the store.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointKvEntry {
    /// The entry's key.
    pub key: String,
    /// The entry's value, as its JSON encoding: the store keeps the
    /// encoding, not a parsed `Value`, so journal and checkpoint
    /// records stay `Eq` and the byte charge is the string's length.
    pub value: String,
}

/// One live blob of the world as a checkpoint declares it (#1062):
/// the blob's index entry, not its content. The writer also emits
/// one `BlobStored` journal entry per live blob before the
/// checkpoint, and replay refuses a checkpoint whose declared index
/// disagrees with them — the schedule treatment
/// ([`CheckpointSchedule`]) applied to the blob set. Only the index
/// is declared because the content would exceed the checkpoint
/// scalar bound on its own; the entries carry the bytes, and the
/// declared size pins what each entry must hold.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointBlobEntry {
    /// The blob's content address: its hash as lowercase hex.
    pub hash: String,
    /// The content's length, in bytes.
    pub size: usize,
}

/// This world's schedule-delivery consent for one peer world (#1355
/// part 2): the pair must be reciprocal across the two worlds for a
/// cross-world fire to happen — the source holds `send` for the target,
/// the target holds `receive` for the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScheduleGrant {
    /// This world may pend schedule fires for the peer.
    pub(crate) send: bool,
    /// The peer may deliver schedule fires to this world.
    pub(crate) receive: bool,
}

/// A boxed future, so [`ScheduleDelivery`] stays object-safe (the
/// [`crate::world_hub::WorldOpener`] pattern).
pub(crate) type ScheduleDeliveryFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>;

/// Delivers one cross-world schedule fire to its target world (#1355
/// part 2). Installed on an actor by the composition layer (the hub);
/// an actor with no sink leaves cross-world schedules pending, exactly
/// like a target that refuses. `true` means the target durably accepted
/// the fire (its receipt is journaled), so the source may consume the
/// occurrence.
/// Why a portal send or delivery did not happen (F12.14 wiring,
/// code#1068). A policy refusal is the kernel's own verdict and is
/// audit-recorded by the refusing side; `Unavailable` is a transport
/// failure (no sink installed, unknown peer world, closed actor,
/// target program did not accept) — no policy decision was made, so
/// no portal audit entry is written for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortalSendError {
    /// The mutual-grant/bounds/rate check refused the message.
    Refused(id_caps::portal::PortalRefusal),
    /// The transport could not carry the message at all.
    Unavailable(String),
}

impl std::fmt::Display for PortalSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "portal refused: {}", refusal.code()),
            Self::Unavailable(reason) => write!(f, "portal transport unavailable: {reason}"),
        }
    }
}

impl std::error::Error for PortalSendError {}

/// A boxed future, so [`PortalDelivery`] stays object-safe (the
/// [`ScheduleDelivery`] pattern).
pub(crate) type PortalDeliveryFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Carries F12.14 portal messages between world actors (code#1068
/// wiring slice). Installed on a world's handle by the composition
/// layer (the hub installs its sink on every lease; see
/// `WorldHub::lease_with`); a handle with no sink cannot evidence
/// a peer's grant, so its sends fail as `Unavailable` — a
/// transport failure, never a policy refusal, because no peer book
/// was ever read. The sink is the transport only: policy lives in
/// the actors, which run `id_caps::portal::check_portal_message`
/// against both worlds' books and record the audit shapes on
/// their own sides.
/// Shared storage for a world's installed [`PortalDelivery`] sink:
/// one slot per world, shared by every clone of its handle, so the
/// composition layer installs once and every handle clone sends
/// through the same sink.
#[derive(Clone, Default)]
pub(crate) struct PortalDeliverySlot {
    inner: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<dyn PortalDelivery>>>>,
}

impl std::fmt::Debug for PortalDeliverySlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PortalDeliverySlot(..)")
    }
}

impl PortalDeliverySlot {
    /// The installed sink, if any.
    pub(crate) fn get(&self) -> Option<std::sync::Arc<dyn PortalDelivery>> {
        self.inner.lock().ok().and_then(|slot| slot.clone())
    }

    /// Install `delivery`, replacing any sink already installed.
    pub(crate) fn set(&self, delivery: std::sync::Arc<dyn PortalDelivery>) {
        if let Ok(mut slot) = self.inner.lock() {
            *slot = Some(delivery);
        }
    }

    /// Install `delivery` only when no sink is installed yet (the
    /// hub's lease path: a world's own explicitly installed sink
    /// is never displaced). Returns whether it installed.
    pub(crate) fn install(&self, delivery: std::sync::Arc<dyn PortalDelivery>) -> bool {
        if let Ok(mut slot) = self.inner.lock() {
            if slot.is_none() {
                *slot = Some(delivery);
                return true;
            }
        }
        false
    }
}

pub(crate) trait PortalDelivery: Send + Sync {
    /// The named world's current portal grant book, or `None` when
    /// the world cannot be reached (unknown, busy, closed).
    fn peer_portal_grants<'a>(
        &'a self,
        world: &'a str,
    ) -> PortalDeliveryFuture<'a, Option<id_caps::portal::PortalGrants>>;

    /// Deliver one already-checked message to `target`'s actor,
    /// which re-checks its own book before accepting.
    fn deliver_portal<'a>(
        &'a self,
        source: &'a str,
        target: &'a str,
        message: &'a str,
    ) -> PortalDeliveryFuture<'a, Result<(), PortalSendError>>;
}

/// The host event a program receives when a portal message from
/// `source` is delivered here: it names its source world as `from`
/// and carries the message text (bounded by the portal check to
/// `MAX_PORTAL_MESSAGE_BYTES` before delivery).
pub(crate) fn portal_message_event(source: &str, message: &str) -> String {
    serde_json::json!({
        "cap": "portal",
        "event": "message",
        "from": source,
        "message": message,
    })
    .to_string()
}

pub(crate) trait ScheduleDelivery: Send + Sync {
    /// Deliver the fire of schedule `id`, planned for `at_ms`, from
    /// `source` to `target`.
    fn deliver_schedule<'a>(
        &'a self,
        source: &'a str,
        target: &'a str,
        id: &'a str,
        at_ms: u64,
        late: bool,
    ) -> ScheduleDeliveryFuture<'a>;
}

/// The host event a program receives when schedule `id` fires. `late` is set
/// only for a schedule the journal left overdue when the world reopened.
pub(crate) fn schedule_fired_event(id: &str, late: bool) -> String {
    schedule_fired_event_from(id, late, None)
}

/// Like [`schedule_fired_event`], for a fire delivered from another
/// world: the event names its source world as `from`.
pub(crate) fn schedule_fired_event_from(id: &str, late: bool, source: Option<&str>) -> String {
    let mut event = serde_json::json!({"cap": "schedule", "event": "fired", "id": id});
    if late {
        event["late"] = serde_json::Value::Bool(true);
    }
    if let Some(source) = source {
        event["from"] = serde_json::json!(source);
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
    schedules: &BTreeMap<String, PendingSchedule>,
    now_ms: u64,
    now_instant: std::time::Instant,
    deliverable: bool,
) -> Option<std::time::Instant> {
    if !deliverable {
        return None;
    }
    let earliest = schedules.values().map(|schedule| schedule.at_ms).min()?;
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
pub struct EmptyWorldProgram;

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
/// - A `Notification` is returned as is: it is addressed to the world's
///   users, not to its chat readers, so the chat-read scope does not gate it.
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
    /// The moderator participant who revoked this capability directly, so a
    /// checkpoint keeps the audit actor (#11778). `None` for an admin revoke,
    /// for one inherited from a revoked ancestor, and while active.
    revoked_by: Option<u64>,
    bounds: CapabilityBounds,
    used: u32,
    subject: Option<String>,
    /// When the world issued this capability (#11960); `None` when the
    /// journal that restored it predates recorded times.
    issued_at: Option<u64>,
    /// When this capability was revoked, directly or with an ancestor
    /// (#11960); `None` while active or when no time was recorded.
    revoked_at: Option<u64>,
}
