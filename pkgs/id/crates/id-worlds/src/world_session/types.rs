use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use iroh_blobs::api::{Store, TempTag};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::directory_auth::Signed;
use crate::world::{
    CapabilityBounds, FileScope, Isolation, MetricsSnapshot, SessionId, WorldEvent, WorldHandle,
    WorldMetrics, WorldPolicy, WorldSnapshot,
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

/// Longest accepted resume token, in bytes. A longer token is not an error:
/// it simply cannot name a retained baseline, so the client gets a full view.
pub(crate) const MAX_RESUME_TOKEN_BYTES: usize = 64;

/// Longest accepted admin secret, in bytes.
pub const MAX_ADMIN_TOKEN_BYTES: usize = 256;
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
    /// A complete binary frame no larger than [`MAX_FRAME_BYTES`], on a
    /// transport that carries bytes. Client frames are text by protocol,
    /// so the driver fails a binary frame safely, exactly like
    /// [`Inbound::Unsupported`]; the variant exists so byte-capable
    /// transports can report the frame instead of discarding it.
    Bytes(Vec<u8>),
    /// A frame larger than [`MAX_FRAME_BYTES`] (its contents are discarded).
    Oversized,
    /// A frame that is not UTF-8 text.
    Unsupported,
    /// The peer is gone or the stream failed; the session must end.
    Closed,
}

/// Moves whole frames for one session.
///
/// `recv` must be cancel-safe: the driver races it against world events and a
/// cancelled call must not lose or split a frame.
pub trait SessionIo: Send {
    /// Next inbound frame.
    fn recv(&mut self) -> impl Future<Output = Inbound> + Send;
    /// Send one text frame. Returns `false` when the peer is gone.
    fn send(&mut self, frame: String) -> impl Future<Output = bool> + Send;
    /// Whether this transport can carry binary frames. Only a session
    /// on a byte-capable transport may negotiate binary screen patches
    /// (F12.11 phase 3); on any other transport the negotiation falls
    /// back to the JSON patch path.
    fn supports_bytes(&self) -> bool {
        false
    }
    /// Send one binary frame. Returns `false` when the peer is gone —
    /// and on transports that cannot carry bytes, which never send.
    fn send_bytes(&mut self, frame: Vec<u8>) -> impl Future<Output = bool> + Send {
        let _ = frame;
        std::future::ready(false)
    }
    /// Mark this session authorized after its application-level
    /// capability or admin credential has been validated.
    fn mark_authorized(&mut self) {}
}

/// Tunables for [`run_session_with`].
#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// How long a peer may take to send its first frame.
    pub join_timeout: Duration,
    /// The server blob store documents are served from, when the
    /// transport holds one the world service does not (the SSH shell
    /// receives the store directly; D1-040 decision 7). Falls back to
    /// the world service's own blob store when `None`.
    pub documents_store: Option<Store>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            join_timeout: Duration::from_secs(10),
            documents_store: None,
        }
    }
}

/// A producer environment for pointer input (#1831 slice S2). Every
/// input rule in the model is stated per environment; there is no
/// environment-free event.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InputEnvironment {
    Native,
    Web,
    Tui,
    Android,
}

/// A journaled pointer activation event (#1831 slice S2). The common
/// core is `press` / `release` / `cancel` / `drag_move` (the wire form
/// of a drag) / `scroll`, plus `long_press` where the producer has a
/// timing source; `secondary_press` is a per-environment extra.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointerEvent {
    Press,
    Release,
    Cancel,
    DragMove,
    Scroll,
    LongPress,
    SecondaryPress,
}

/// An ephemeral pointer hover event (#1831 slice S2).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoverEvent {
    HoverEnter,
    HoverLeave,
    HoverMove,
}

/// Which pointer button a frame concerns (#1831 slice S2).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointerButton {
    Primary,
    Secondary,
}

/// Why the host refused a pointer event (#1831 slice S2, the design's
/// refusal table). A refused event is answered explicitly with one of
/// these reasons — never silently dropped.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InputRefusalReason {
    NoTimingSource,
    HoverUnsupported,
    ContextMenuNotClaimed,
    ReservedByBrowser,
    NoSecondaryButton,
    NotReportedByTerminal,
    ButtonIdentityUnavailable,
    NoTrackingMode,
    GestureNotInSubset,
    MultiTouchDeferred,
    NotDeclaredByApp,
    UnknownOrUnsupportedEvent,
}

/// One screen rectangle in cells, origin at the top-left — the region a
/// `screen_subscribe` frame names (contract revision 11, #6300 section
/// 4.2). Bounds are the transport's: the same 500x200 ceiling `resize`
/// accepts; the host clamps views to the presentation maximum itself.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ScreenRegion {
    /// Leftmost column, zero-based.
    pub(crate) x: u16,
    /// Topmost row, zero-based.
    pub(crate) y: u16,
    /// Width in cells, at least 1.
    pub(crate) width: u16,
    /// Height in rows, at least 1.
    pub(crate) height: u16,
}

/// The semantic change kinds a `screen_subscribe` may name and a
/// `semantic_diff` may carry for world events (contract revision 11).
/// Region diffs use `region_changed` and are scoped by the subscription's
/// region instead, so that kind is not subscribable.
pub(crate) const SEMANTIC_EVENT_CLASSES: &[&str] = &[
    "chat_posted",
    "input_applied",
    "object_changed",
    "system_event",
    "program_changed",
];

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
        /// Opt in to negotiated binary screen patches (F12.11 phase 3,
        /// contract revision 12): when `screen_patches` is also set and
        /// the transport carries bytes, patch frames arrive as versioned
        /// MessagePack binary frames instead of JSON `view` frames.
        /// Anywhere else — no patch opt-in, a text-only transport — the
        /// session keeps the JSON patch path unchanged. Omitted by
        /// clients that predate the revision.
        #[serde(default)]
        screen_msgpack: bool,
        /// Resume token from this client's last applied view. A cursor, not a
        /// credential: the join capability above still authorizes the session,
        /// and an unknown, stale or foreign token simply gets a full view.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resume_token: Option<String>,
        /// Opt in to the agent profile (contract revision 11, #6300
        /// section 4.2): the capability flag advertising that this shell
        /// consumes semantic diffs and may send `screen_subscribe`.
        /// Omitted by human shells; a host without the revision ignores
        /// it and serves full views at reduced fidelity, same authority.
        #[serde(default)]
        agent_profile: bool,
        /// Input affordances of this presenter. This is descriptive session
        /// metadata, not an authority grant; omitted by older clients.
        #[serde(default)]
        device_input: Option<serde_json::Value>,
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
    /// Page the server blob store's public document names (D1-040).
    Documents {
        capability: String,
        /// Only names starting with this prefix.
        prefix: Option<String>,
        /// Continue after this name (exclusive).
        after: Option<String>,
    },
    /// Read one public document from the server blob store (D1-040).
    Document {
        capability: String,
        name: String,
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
    /// Admin: export the bounded ordered history, including host events.
    EventHistoryAdmin {
        admin_token: String,
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
    /// Public: list the worlds this server hosts, as the browsable
    /// directory (#812). No admin token: the reply carries directory
    /// metadata only — world names, loaded state, the default, and a
    /// per-world presence *count*. It never names who is present, and
    /// it neither creates nor opens any world. The admin `ListWorlds`
    /// frame remains for operators; this is the deliberate public
    /// boundary for the directory feed.
    Directory {},
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
        /// Stale-decision token (contract revision 11, #6300 section
        /// 4.3): the screen revision this input was decided against.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen_revision: Option<u64>,
    },
    /// Submit an answer from a trusted form presenter on this joined session.
    /// The actor checks session authority, the pending form id, and its
    /// host-generated presentation token before accepting the values.
    FormResult {
        form_id: String,
        presentation_token: String,
        action: String,
        values: serde_json::Map<String, serde_json::Value>,
        /// Stale-decision token; see [`ClientFrame::Chat`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen_revision: Option<u64>,
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
        /// Stale-decision token; see [`ClientFrame::Chat`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen_revision: Option<u64>,
    },
    /// Ephemeral dimensions for this connection's presentation only. Never journaled.
    Resize {
        width: u16,
        height: u16,
    },
    /// Explicit bounded clipboard text, delivered as one durable input event.
    Paste {
        text: String,
        /// Stale-decision token; see [`ClientFrame::Chat`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen_revision: Option<u64>,
    },
    /// Agent profile (contract revision 11, #6300 section 4.2): subscribe
    /// this session to a screen region and/or semantic event classes.
    /// Ephemeral like `Resize`: connection-local presentation context,
    /// never journaled and never seen by other sessions. At least one of
    /// the two fields must be present; an empty `event_classes` list
    /// means all classes, an absent one means no event diffs.
    ScreenSubscribe {
        /// Screen rectangle in cells, origin at the top-left.
        region: Option<ScreenRegion>,
        /// Semantic change kinds to receive as `semantic_diff` frames.
        event_classes: Option<Vec<String>>,
    },
    /// Pointer activation input (#1831 slice S2, input event model design):
    /// journaled and replayed like `Input`. Coordinates are cells in the
    /// target surface's grid — producers map device pixels to cells at the
    /// edge, so a frame never carries pixels. A drag arrives as a `press`,
    /// a sequence of `drag_move` frames, and a `release`.
    Pointer {
        event: PointerEvent,
        environment: InputEnvironment,
        /// Unique only within the producing connection; the host namespaces
        /// it with the connection's identity when journaling.
        pointer_id: u32,
        x: u16,
        y: u16,
        /// Which button a `secondary_press` / `release` concerns.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<PointerButton>,
        /// Producer-measured hold time; `release` only. Carried so replay
        /// derives the same click semantic without a wall clock.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        held_duration_ms: Option<u64>,
        /// The producer's click classification; `release` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_click: Option<bool>,
        /// Cell delta; `scroll` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dx: Option<i64>,
        /// Cell delta; `scroll` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dy: Option<i64>,
        /// Stale-decision token; see [`ClientFrame::Chat`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen_revision: Option<u64>,
    },
    /// Pointer hover (#1831 slice S2): connection-local presentation
    /// context, ephemeral like `Resize` — never journaled and never seen
    /// by other sessions.
    Hover {
        event: HoverEvent,
        environment: InputEnvironment,
        pointer_id: u32,
        x: u16,
        y: u16,
    },
    InstallBegin {
        /// The admin secret, for the pre-join admin path. Joined builder
        /// sessions (design #835 phase 3) omit it and are authorized by
        /// their live capability instead; a joined frame that carries
        /// one is refused as mixed credentials.
        #[serde(default)]
        admin_token: Option<String>,
        total_bytes: usize,
        /// Expected BLAKE3 hash. Optional for browser uploads, whose `WebCrypto`
        /// API does not provide BLAKE3; the host still computes and returns it.
        module_hash: Option<String>,
        /// Optional Ed25519 signature over the domain-separated module digest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<ModuleSignature>,
        seed: u64,
    },
    InstallChunk {
        offset: usize,
        data_hex: String,
    },
    InstallEnd,
    /// Joined moderator: revoke an active participant (design #835
    /// phase 3). The frame names only the target — the audit actor is
    /// derived from this session's live capability by the world actor,
    /// never from the frame.
    Revoke {
        participant_id: u64,
    },
    /// F13.7 step 2: the shell advertises the client-side providers
    /// it can serve for this session — only ones the local user has
    /// opted into. An offer enables nothing by itself: the host
    /// broker (step 3) may only request an offered provider, and
    /// only when world policy also allows it. A shell that never
    /// offers is a v1 shell for provider purposes and receives no
    /// `provider_request` frames.
    ProviderOffer {
        /// Provider names, each under the `client.` namespace.
        providers: Vec<String>,
    },
    /// F13.7 step 2: the shell's answer to one `provider_request`,
    /// correlated by the host-minted `nonce`. Exactly one of
    /// `result_hex` / `error` is meaningful, per `ok`. The reply is
    /// ephemeral: only the broker's accepted result event is
    /// journaled, never the reply itself.
    ProviderReply {
        /// The nonce from the request being answered, hex-encoded.
        nonce: String,
        /// Whether the local adapter executed the request.
        ok: bool,
        /// The bounded result bytes, hex-encoded, when `ok`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_hex: Option<String>,
        /// A stable refusal code when not `ok`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Signer metadata supplied with an uploaded module. Public keys and
/// signatures are lower- or upper-case hexadecimal Ed25519 encodings. The
/// signed message is `b"developing-today/id-world-module/v1\\0"` followed by
/// the 32-byte BLAKE3 digest of the exact uploaded module bytes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModuleSignature {
    pub public_key: String,
    pub signature: String,
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
    /// F13.7 step 2: the host broker asks the originating session's
    /// shell to execute one client-side provider. Sent only to a
    /// session that advertised the provider in `provider_offer`;
    /// shells that do not know this frame ignore it (the contract's
    /// additive-revision rule), and a reply is accepted only for the
    /// bound session and nonce (broker rules, step 3). Ephemeral:
    /// the pending request is never journaled.
    ProviderRequest {
        /// Host-minted correlation nonce, hex-encoded.
        nonce: &'a str,
        /// The provider name, under the `client.` namespace.
        provider: &'a str,
        /// The request arguments, per the provider's manifest schema.
        args: serde_json::Value,
        /// The broker's deadline for the reply, in milliseconds.
        deadline_ms: u64,
    },
    /// Acknowledges a deferred form answer; on field validation failure the
    /// fresh token lets the trusted presenter show the pending form again.
    FormResultStatus {
        form_id: &'a str,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        presentation_token: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<&'a str>,
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
        /// Token naming `view_revision`, for a client that reconnects and asks
        /// to resume instead of taking a full view.
        #[serde(skip_serializing_if = "Option::is_none")]
        resume_token: Option<&'a str>,
    },
    /// Agent profile (contract revision 11, #6300 section 4.2): what
    /// changed, in world terms, scoped to this session's
    /// `screen_subscribe` subscription — never a repaint instruction.
    /// Ephemeral: a diff is derived presentation for one session, built
    /// from the durable event or the screens the session already holds.
    SemanticDiff {
        /// World sequence this diff follows.
        sequence: u64,
        /// Screen revision this diff was computed at.
        view_revision: u64,
        /// Screen revision the diff was computed against, when the host
        /// diffed two screens (region diffs); absent for event diffs.
        #[serde(skip_serializing_if = "Option::is_none")]
        base_view_revision: Option<u64>,
        /// The changes, each an object with a `kind` naming what happened
        /// (`chat_posted`, `region_changed`, ...).
        changes: Vec<serde_json::Value>,
        /// The subscribed region these changes are scoped to, for region
        /// diffs; absent for event diffs.
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<ScreenRegion>,
    },
    /// Agent profile (contract revision 11): the session's now-active
    /// screen subscription, answering a `screen_subscribe` frame.
    ScreenSubscription {
        /// The subscribed region, echoed; absent for event-only
        /// subscriptions.
        region: Option<ScreenRegion>,
        /// The subscribed semantic event classes, echoed; absent for
        /// region-only subscriptions.
        event_classes: Option<Vec<String>>,
        /// The session's current screen revision at subscribe time —
        /// the revision a following input's stale-decision token is
        /// judged against.
        view_revision: u64,
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
    /// One page of public document names, in sorted order. No
    /// `sequence`: the listing is a point-in-time snapshot of the
    /// blob store's tag set, which has no sequence to report (D1-040).
    Documents {
        names: Vec<String>,
        /// Present when more names follow; pass as `after` to continue.
        next_after: Option<String>,
    },
    /// One public document: its UTF-8 text, byte size, and name.
    Document {
        name: String,
        size: u64,
        text: String,
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
    /// Starts an admin-only ordered event-history transfer.
    EventHistoryBegin {
        world_id: String,
        module_hash: Option<String>,
        current_sequence: u64,
        complete: bool,
        event_count: usize,
    },
    /// One sequenced or host-originated event in journal order.
    EventHistoryEvent {
        event: &'a WorldEvent,
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
    /// A joined moderator's revoke was carried out (`revoked: true`)
    /// or named an inactive/unknown target (`revoked: false`, a no-op
    /// that writes no journal record).
    Revoked {
        participant_id: u64,
        revoked: bool,
    },
    ProgramChanged {
        module_hash: &'a str,
        sequence: u64,
    },
    Error {
        message: &'a str,
        /// Stable denial code for joined role/capability refusals
        /// (`actor_denied`, design #835 §5). Omitted for every existing
        /// error, so their serialized shape is unchanged.
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<&'a str>,
    },
    /// A pointer event the host refused (#1831 slice S2): the event the
    /// environment cannot produce, or that the target app did not
    /// declare in its manifest `interfaces.input`, is answered with the
    /// design's named reason — never silently dropped.
    InputRefused {
        event: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        environment: Option<InputEnvironment>,
        reason: InputRefusalReason,
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
    /// The joined session's live capability does not authorize this
    /// install (design #835 phase 3): invalid, expired, revoked, or
    /// missing the `BUILDER` scope. Non-oracular by design — the wire
    /// code is `actor_denied` for every one of those causes.
    ActorDenied,
    /// The server has a module trust list and the artifact is unsigned,
    /// signed by an untrusted key, or has an invalid signature.
    UntrustedModule,
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
    /// Ed25519 public keys authorized to sign module artifacts. Empty keeps
    /// legacy behavior and accepts unsigned modules.
    pub(crate) trusted_module_keys: Vec<[u8; 32]>,
    pub(crate) runtime: crate::world_limits::RuntimeLimits,
    pub(crate) mail: Option<Arc<dyn crate::directory_mail::MailSink>>,
    pub(crate) outbox: Option<Arc<Mutex<crate::envelope_outbox::Outbox>>>,
    pub(crate) profile_operation: Arc<Mutex<()>>,
    /// The world's host metrics registry (F11.7), shared with the actor:
    /// the session layer records the frame bytes it sends into it.
    pub(crate) metrics: WorldMetrics,
    /// Monotonic, process-local IDs for live connections to this service.
    pub(crate) next_session_id: Arc<AtomicU64>,
    /// Bounded retained screens a reconnecting client may resume from, keyed
    /// by an opaque token bound to a participant, viewport and view revision.
    pub(crate) resume_baselines: Arc<Mutex<super::resume::BaselineCache>>,
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
