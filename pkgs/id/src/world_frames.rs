//! Client/server frame types for world sessions. Split from `world_session.rs`
//! (F11.10, #540); moves only.

use serde::{Deserialize, Serialize};

use crate::world::{WorldEvent, WorldSnapshot};


// Install frames are only produced/consumed with the `sandbox` feature.
#[cfg_attr(not(feature = "sandbox"), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ClientFrame {
    Join {
        capability: String,
        after: Option<u64>,
    },
    Info {
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
    Invite {
        admin_token: String,
        display_name: String,
        /// Seconds until the invite stops working (default 7 days, at most 30).
        expires_in_secs: Option<u64>,
        /// Committed actions the invite may take; absent means no limit.
        uses: Option<u32>,
        /// Role preset (`spectator`, `observer`, `player`, `guest`, `delegator`); absent
        /// means `guest`.
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
    },
    ModuleInfo {
        world_id: String,
        active_module_hash: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_module_provenance: Option<crate::world::ModuleProvenance>,
        current_sequence: u64,
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
