use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use iroh_blobs::api::{Store, TempTag};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::directory_auth::Signed;
use crate::world::{
    CapabilityBounds, FileScope, Isolation, MetricsSnapshot, WorldEvent, WorldHandle, WorldMetrics,
    WorldPolicy, WorldSnapshot,
};
use crate::world_compile::Compiler;
use crate::world_hub::WorldInfo;

/// Largest Wasm world module that may be installed (16 MiB).
///
/// The definition lives in `id-core`'s `world_limits` since the Phase 1
/// crate split (#3447); re-exported here at its historical path.
pub use crate::world_limits::MAX_WORLD_MODULE_BYTES;

/// Largest raw upload chunk; hex + JSON framing remains below [`MAX_FRAME_BYTES`].
pub const MAX_WORLD_MODULE_CHUNK_BYTES: usize = 6 * 1024;

/// Longest accepted capability string, in bytes.
pub(crate) const MAX_CAPABILITY_BYTES: usize = 256;

/// Longest accepted admin secret, in bytes.
pub(crate) const MAX_ADMIN_TOKEN_BYTES: usize = 256;
/// Sent with a capability ceiling change: unlike a profile, it takes effect now.
pub(crate) const CAPS_CEILING_NOTICE: &str =
    "ceiling saved; it applies immediately to new uses and active subscriptions";

/// Simultaneous bounded module uploads accepted by a world service.
pub(crate) const MAX_CONCURRENT_MODULE_UPLOADS: usize = 2;

/// One inbound unit from a transport.
#[derive(Debug)]
pub enum Inbound {
    /// A complete text frame no larger than [`MAX_FRAME_BYTES`].
    Text(String),
    /// A frame larger than [`MAX_FRAME_BYTES`] (its contents are discarded).
    Oversized,
    /// A frame that is not UTF-8 text.
    Unsupported,
    /// The peer is gone or the stream failed; the session must end.
    Closed,
}

/// Moves whole text frames for one session.
///
/// `recv` must be cancel-safe: the driver races it against world events and a
/// cancelled call must not lose or split a frame.
pub trait SessionIo: Send {
    /// Next inbound frame.
    fn recv(&mut self) -> impl Future<Output = Inbound> + Send;
    /// Send one frame. Returns `false` when the peer is gone.
    fn send(&mut self, frame: String) -> impl Future<Output = bool> + Send;
    /// Mark this session authorized after its application-level
    /// capability or admin credential has been validated.
    fn mark_authorized(&mut self) {}
}

/// Tunables for [`run_session_with`].
#[derive(Clone, Copy, Debug)]
pub struct SessionConfig {
    /// How long a peer may take to send its first frame.
    pub join_timeout: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            join_timeout: Duration::from_secs(10),
        }
    }
}

// Install frames are only produced/consumed with the `sandbox` feature.
#[cfg_attr(not(feature = "sandbox"), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientFrame {
    Join {
        capability: String,
        after: Option<u64>,
        /// Opt in to revisioned JSON screen patches. Omitted by v1 clients.
        #[serde(default)]
        screen_patches: bool,
    },
    Info {
        capability: String,
    },
    /// Read the world's host metrics (F11.7): the same snapshot
    /// `id world stats` prints under `metrics`.
    Metrics {
        capability: String,
    },
    /// Page through the world's structured records.
    Records {
        capability: String,
        /// Only keys starting with this prefix.
        prefix: Option<String>,
        /// Continue after this key (exclusive).
        after: Option<String>,
    },
    /// Ask for a read-only doc ticket to replicate the records peer-to-peer.
    RecordsTicket {
        capability: String,
    },
    DownloadModule {
        capability: String,
        module_hash: String,
    },
    /// Admin: download a stored module by hash, whether or not it is the
    /// active module. The participant download above serves only the active
    /// module; an admin reconciling a world to a manifest (#807) needs a
    /// module the host already holds - installed earlier and still in its
    /// blob store - by its hash, to install it under the manifest's seed.
    DownloadModuleAdmin {
        admin_token: String,
        module_hash: String,
    },
    Invite {
        admin_token: String,
        display_name: String,
        /// Seconds until the invite stops working (default 7 days, at most 30).
        expires_in_secs: Option<u64>,
        /// Committed actions the invite may take; absent means no limit.
        uses: Option<u32>,
        /// Role preset (`spectator`, `observer`, `player`, `guest`, `delegator`,
        /// `builder`, `moderator`); absent means `guest`.
        #[serde(default)]
        role: Option<String>,
    },
    /// Admin: list the worlds this server hosts.
    ListWorlds {
        admin_token: String,
    },
    /// Admin: create the world named by the frame's `world` field.
    CreateWorld {
        admin_token: String,
    },
    /// Admin, or a signed request from a moderator: set the world's isolation.
    /// A signed request covers method `PUT`, path `/world/<name>/isolation`,
    /// and the isolation word as its body.
    SetIsolation {
        isolation: Isolation,
        admin_token: Option<String>,
        signed: Option<Signed>,
    },
    /// Admin, or a signed request from a moderator: let other unisolated
    /// worlds write here. A signed request covers `PUT
    /// /world/<name>/cross-world-write` with the body `on` or `off`.
    SetCrossWorldWrite {
        enabled: bool,
        admin_token: Option<String>,
        signed: Option<Signed>,
    },
    /// Admin: force cross-world writes on, overriding the world's own setting.
    ForceCrossWorldWrite {
        admin_token: String,
    },
    /// Admin, or a signed request from a moderator: set the file scope. Only
    /// the admin sets `write_override`. A signed request covers `PUT
    /// /world/<name>/file-scope` with the scope word as its body.
    SetFileScope {
        scope: FileScope,
        write_override: Option<bool>,
        admin_token: Option<String>,
        signed: Option<Signed>,
    },
    /// Admin: compile Roc sources and install the result.
    Compile {
        admin_token: String,
        files: Vec<CompileFile>,
        seed: Option<u64>,
        #[serde(default)]
        native: bool,
    },
    /// Admin: grant or revoke capabilities for this world.
    UpdateCaps {
        admin_token: String,
        grant: Vec<String>,
        revoke: Vec<String>,
    },
    /// Admin: replace this world's capability ceiling. It applies immediately;
    /// grants are kept, and an empty list allows no capability.
    SetCapsCeiling {
        admin_token: String,
        ceiling: Vec<String>,
    },
    /// Admin: pend one schedule (replacing any pending schedule with the
    /// same id), journaled before the reply. The manifest apply path's
    /// `ScheduleAt` step (#807).
    SetSchedule {
        admin_token: String,
        id: String,
        at_ms: u64,
    },
    /// Admin: cancel a pending schedule by id (a no-op when absent),
    /// journaled before the reply. The manifest apply path's
    /// `CancelSchedule` step (#807).
    CancelSchedule {
        admin_token: String,
        id: String,
    },
    /// Read the manifest-reconciliation state: the pending schedules, and
    /// the active install seed when the admin token is valid. A guest
    /// capability alone reads the schedules only — the seed never leaves
    /// the admin layer (#240).
    WorldState {
        admin_token: Option<String>,
        capability: Option<String>,
    },
    /// Admin: persist requested per-world runtime bounds. They do not change
    /// a running guest; they apply on its next install or process restart.
    SetExecutionProfile {
        admin_token: String,
        profile: crate::world_limits::WorldExecutionProfile,
    },
    /// Read the world's requested runtime bounds (needs either admin token or guest capability).
    ExecutionProfileInfo {
        admin_token: Option<String>,
        capability: Option<String>,
    },
    /// Show this world's capability grants, wants and usage.
    Caps {
        capability: String,
    },
    Chat {
        text: String,
    },
    /// Joined: mint a capability with a subset of this session's scopes. A
    /// delegation carries no use count.
    Attenuate {
        name: String,
        scopes: Vec<String>,
        /// Unix milliseconds at which the delegated capability stops working.
        expires_at: Option<u64>,
    },
    /// Joined as an account: delegate a subset of its scopes to one of its friends.
    DelegateToFriend {
        friend: String,
        name: String,
        scopes: Vec<String>,
        expires_at: Option<u64>,
    },
    Input {
        data_hex: String,
    },
    /// Ephemeral dimensions for this connection's presentation only. Never journaled.
    Resize {
        width: u16,
        height: u16,
    },
    /// Explicit bounded clipboard text, delivered as one durable input event.
    Paste {
        text: String,
    },
    InstallBegin {
        admin_token: String,
        total_bytes: usize,
        /// Expected BLAKE3 hash. Optional for browser uploads, whose `WebCrypto`
        /// API does not provide BLAKE3; the host still computes and returns it.
        module_hash: Option<String>,
        seed: u64,
    },
    InstallChunk {
        offset: usize,
        data_hex: String,
    },
    InstallEnd,
}

// Install frames are only produced/consumed with the `sandbox` feature.
#[cfg_attr(not(feature = "sandbox"), allow(dead_code))]
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ServerFrame<'a> {
    Snapshot {
        snapshot: &'a WorldSnapshot,
    },
    Event {
        event: &'a WorldEvent,
    },
    /// Host-rendered presentation for the participant's adapter.
    View {
        sequence: u64,
        data: &'a str,
        /// Optional typed screen surface. Older clients can continue using `data`.
        #[serde(skip_serializing_if = "Option::is_none")]
        surface: Option<&'a crate::world_presentation::WorldSurface>,
        /// Monotonic within this session; independent of the world event cursor.
        view_revision: u64,
        /// Present only when `patch` applies to this exact prior view revision.
        #[serde(skip_serializing_if = "Option::is_none")]
        base_view_revision: Option<u64>,
        /// A negotiated screen delta. `data` is empty and `surface` omitted.
        #[serde(skip_serializing_if = "Option::is_none")]
        patch: Option<&'a crate::world_presentation::ScreenPatch>,
    },
    ModuleInfo {
        world_id: String,
        active_module_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_module_provenance: Option<crate::world::ModuleProvenance>,
        current_sequence: u64,
    },
    /// The world's host metrics snapshot (F11.7).
    Metrics {
        metrics: MetricsSnapshot,
    },
    /// One page of structured records, with the sequence they were read at.
    Records {
        sequence: u64,
        records: crate::world::Records,
        /// Present when more keys follow; pass as `after` to continue.
        next_after: Option<String>,
    },
    /// A read-only iroh-docs ticket for the records document.
    RecordsTicket {
        ticket: String,
    },
    ModuleBegin {
        module_hash: String,
        total_bytes: usize,
    },
    ModuleChunk {
        offset: usize,
        data_hex: String,
    },
    ModuleEnd {
        module_hash: String,
    },
    Invite(&'a InviteResponse),
    /// The worlds this server hosts.
    Worlds {
        default: &'a str,
        worlds: Vec<WorldInfo>,
    },
    WorldCreated {
        world: &'a str,
        created: bool,
    },
    Isolation {
        world: &'a str,
        isolation: Isolation,
    },
    /// The world's sharing and file settings after a change.
    Policy {
        world: &'a str,
        policy: WorldPolicy,
    },
    /// The world's capability report.
    Caps {
        report: crate::world_caps::CapsReport,
    },
    /// The requested profile and an explicit deferred-application notice.
    ExecutionProfile {
        update: crate::world::ExecutionProfileUpdate,
        message: &'static str,
    },
    /// The capability report after a ceiling change, and a notice that the
    /// change applied immediately.
    CapsCeiling {
        report: crate::world_caps::CapsReport,
        message: &'static str,
    },
    /// The pending schedules after a schedule change (id to fire time in
    /// unix milliseconds, as a JSON object).
    Schedules {
        schedules: BTreeMap<String, u64>,
    },
    /// The manifest-reconciliation state: pending schedules, plus the
    /// active install seed for admin readers only (`None` for capability
    /// readers, #240).
    WorldState {
        seed: Option<u64>,
        schedules: BTreeMap<String, u64>,
    },
    /// A compiled module was installed.
    Compiled {
        module_hash: String,
        sequence: u64,
        diagnostics: String,
    },
    UploadReady {
        chunk_bytes: usize,
    },
    ModuleInstalled {
        module_hash: &'a str,
        sequence: u64,
    },
    ProgramChanged {
        module_hash: &'a str,
        sequence: u64,
    },
    Error {
        message: &'a str,
    },
}

/// A freshly minted guest capability.
#[derive(Debug, Serialize)]
pub struct InviteResponse {
    /// Bearer capability; shown once, never logged.
    pub capability: String,
    /// The new participant's ID.
    pub participant_id: u64,
    /// The sanitized display name.
    pub display_name: String,
}

/// Why an invite was not issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteError {
    /// Invites are not enabled (no admin secret configured).
    Disabled,
    /// The supplied admin secret is missing or wrong.
    Unauthorized,
    /// The world refused another participant (full, or shut down).
    Unavailable,
    /// The requested lifetime or use limit is out of range.
    InvalidBounds(&'static str),
}

/// Lifetime of an invite when the admin names none.
pub const INVITE_DEFAULT_SECS: u64 = 7 * 24 * 60 * 60;
/// Longest lifetime an invite may be given.
pub const INVITE_MAX_SECS: u64 = 30 * 24 * 60 * 60;

/// Check an invite's requested lifetime and use limit. The host applies it to
/// every invite, and `id world invite` applies it before dialing.
///
/// # Errors
///
/// Returns the refusal reason for a lifetime outside 1 second to 30 days, or a
/// use limit below one.
pub fn check_invite_bounds(
    expires_in_secs: Option<u64>,
    uses: Option<u32>,
) -> Result<(), &'static str> {
    if expires_in_secs.is_some_and(|secs| secs == 0 || secs > INVITE_MAX_SECS) {
        return Err("invite lifetime must be from 1 second to 30 days");
    }
    if uses == Some(0) {
        return Err("uses must be at least 1");
    }
    Ok(())
}

pub(crate) fn invite_bounds(
    expires_in_secs: Option<u64>,
    uses: Option<u32>,
    now_ms: u64,
) -> Result<CapabilityBounds, &'static str> {
    check_invite_bounds(expires_in_secs, uses)?;
    let secs = expires_in_secs.unwrap_or(INVITE_DEFAULT_SECS);
    Ok(CapabilityBounds {
        expires_at: Some(now_ms.saturating_add(secs * 1000)),
        uses,
    })
}

/// Why an uploaded Wasm program was not installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Module installation is unavailable in this service/build.
    Disabled,
    /// The supplied admin secret is missing or wrong.
    Unauthorized,
    /// Size or content hash validation failed.
    InvalidUpload,
    /// Raw artifacts are disabled by this world's source-only policy.
    RawModuleDisabled,
    /// Wasmtime rejected or could not instantiate the module.
    InvalidModule,
    /// The blob could not be pinned or the world actor refused activation.
    Unavailable,
}

/// One source file in a compile request.
#[derive(Debug, Deserialize)]
#[cfg_attr(not(feature = "sandbox"), allow(dead_code))]
pub(crate) struct CompileFile {
    pub(crate) name: String,
    pub(crate) content: String,
}

/// A world plus the secret that may mint guest capabilities for it.
#[derive(Clone)]
pub struct WorldService {
    pub(crate) world: WorldHandle,
    pub(crate) admin_token: Option<String>,
    pub(crate) blobs: Option<Store>,
    pub(crate) module_pin: Arc<Mutex<Option<TempTag>>>,
    pub(crate) module_hash: Arc<RwLock<Option<String>>>,
    pub(crate) upload_slots: Arc<Semaphore>,
    pub(crate) module_dir: Option<crate::world_store::ModuleDir>,
    pub(crate) records: Option<Arc<crate::world_records::RecordsStore>>,
    pub(crate) compiler: Option<Arc<Compiler>>,
    pub(crate) native: bool,
    /// Raw artifact installs are refused; compiled source remains permitted.
    pub(crate) source_only: bool,
    pub(crate) runtime: crate::world_limits::RuntimeLimits,
    pub(crate) mail: Option<Arc<dyn crate::directory_mail::MailSink>>,
    pub(crate) outbox: Option<Arc<Mutex<crate::envelope_outbox::Outbox>>>,
    pub(crate) profile_operation: Arc<Mutex<()>>,
    /// The world's host metrics registry (F11.7), shared with the actor:
    /// the session layer records the frame bytes it sends into it.
    pub(crate) metrics: WorldMetrics,
}

impl std::fmt::Debug for WorldService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldService")
            .field(
                "admin_token",
                &self.admin_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish_non_exhaustive()
    }
}
