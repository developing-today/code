//! Transport-neutral world session driver.
//!
//! A *session* is one participant's connection to a [`WorldHandle`]. The
//! wire protocol is one JSON object per frame; this module owns that protocol
//! and nothing else. A transport (WebSocket, Iroh stream, an in-memory test
//! channel) only implements [`SessionIo`] to move whole text frames, so the
//! behavior cannot drift between transports.
//!
//! Client frames (`type` tag): `join {capability, after?}`,
//! `invite {admin_token, display_name}`, `chat {text}`, `input {data_hex}`.
//! The first frame must be `join` or `invite`. Server frames: `snapshot`,
//! `event`, `invite`, `error`.
//!
//! Authorization is the capability alone. A transport may know who the peer
//! is (an Iroh node ID); that never grants world authority.

use std::sync::Arc;
use std::time::Duration;

use iroh_blobs::{
    BlobFormat, Hash,
    api::{Store, TempTag, blobs::AddBytesOptions},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore, broadcast};

use crate::world::{JoinCapability, WorldEvent, WorldHandle, WorldScopes, WorldSnapshot};

/// Largest accepted or emitted frame, in bytes.
///
/// Snapshots and records pages are bounded to fit inside this (see
/// [`crate::world::WorldLimits::snapshot_bytes`] and
/// [`MAX_RECORDS_PAGE_BYTES`]), and both transports enforce it: the Iroh
/// frame reader refuses longer frames and the WebSocket has
/// `max_message_size(MAX_FRAME_BYTES)`.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Largest Wasm world module that may be installed (16 MiB).
pub const MAX_WORLD_MODULE_BYTES: usize = 16 * 1024 * 1024;

/// Largest raw upload chunk; hex + JSON framing remains below [`MAX_FRAME_BYTES`].
pub const MAX_WORLD_MODULE_CHUNK_BYTES: usize = 6 * 1024;

/// Longest accepted capability string, in bytes.
const MAX_CAPABILITY_BYTES: usize = 256;

/// Longest accepted admin secret, in bytes.
const MAX_ADMIN_TOKEN_BYTES: usize = 256;

/// Simultaneous bounded module uploads accepted by a world service.
const MAX_CONCURRENT_MODULE_UPLOADS: usize = 2;

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
enum ClientFrame {
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
    },
    Chat {
        text: String,
    },
    Input {
        data_hex: String,
    },
    InstallBegin {
        admin_token: String,
        total_bytes: usize,
        /// Expected BLAKE3 hash. Optional for browser uploads, whose WebCrypto
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
enum ServerFrame<'a> {
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
    },
    ModuleInfo {
        world_id: String,
        active_module_hash: Option<String>,
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
    /// Wasmtime rejected or could not instantiate the module.
    InvalidModule,
    /// The blob could not be pinned or the world actor refused activation.
    Unavailable,
}

/// A world plus the secret that may mint guest capabilities for it.
#[derive(Clone)]
pub struct WorldService {
    world: WorldHandle,
    admin_token: Option<String>,
    blobs: Option<Store>,
    module_pin: Arc<Mutex<Option<TempTag>>>,
    module_hash: Arc<RwLock<Option<String>>>,
    upload_slots: Arc<Semaphore>,
    module_dir: Option<crate::world_store::ModuleDir>,
    records: Option<Arc<crate::world_records::RecordsStore>>,
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

impl WorldService {
    /// Wrap a world. `admin_token: None` disables invites.
    #[must_use]
    pub fn new(world: WorldHandle, admin_token: Option<String>) -> Self {
        Self {
            world,
            admin_token,
            blobs: None,
            module_pin: Arc::new(Mutex::new(None)),
            module_hash: Arc::new(RwLock::new(None)),
            upload_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_MODULE_UPLOADS)),
            module_dir: None,
            records: None,
        }
    }

    /// Attach the world's records document and start mirroring program
    /// records into it.
    #[must_use]
    pub fn with_records_store(mut self, records: crate::world_records::RecordsStore) -> Self {
        let records = Arc::new(records);
        records.spawn_publisher(&self.world);
        self.records = Some(records);
        self
    }

    /// The world's records document, if it has one.
    #[must_use]
    pub fn records_store(&self) -> Option<&Arc<crate::world_records::RecordsStore>> {
        self.records.as_ref()
    }

    /// Persist installed modules here so a durable world can re-instantiate
    /// its program after a restart.
    #[must_use]
    pub fn with_module_dir(mut self, modules: crate::world_store::ModuleDir) -> Self {
        self.module_dir = Some(modules);
        self
    }

    /// Mark `wasm` as the active module after a restore: pin it in the blob
    /// store (when attached) so peers can download it again.
    ///
    /// # Errors
    ///
    /// Fails if the blob store refuses the bytes.
    pub async fn adopt_module(&self, wasm: Vec<u8>) -> anyhow::Result<()> {
        let hash = module_hash(&wasm);
        if let Some(blobs) = self.blobs.as_ref() {
            let pin = blobs
                .add_bytes_with_opts(AddBytesOptions {
                    data: wasm.into(),
                    format: BlobFormat::Raw,
                })
                .temp_tag()
                .await?;
            *self.module_pin.lock().await = Some(pin);
        }
        *self.module_hash.write().await = Some(hash);
        Ok(())
    }

    /// Attach this node's blob store so uploaded modules survive as pinned
    /// Iroh content for the life of the running world.
    #[must_use]
    pub fn with_blob_store(mut self, blobs: Store) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// Reserve one upload slot before accepting a potentially large body.
    fn try_reserve_module_upload(&self) -> Result<OwnedSemaphorePermit, InstallError> {
        Arc::clone(&self.upload_slots)
            .try_acquire_owned()
            .map_err(|_| InstallError::Unavailable)
    }

    /// The underlying world actor.
    #[must_use]
    pub const fn world(&self) -> &WorldHandle {
        &self.world
    }

    /// Mint a guest capability if `supplied` matches the admin secret.
    ///
    /// # Errors
    ///
    /// See [`InviteError`].
    pub async fn invite(
        &self,
        supplied: Option<&str>,
        display_name: String,
    ) -> Result<InviteResponse, InviteError> {
        self.authorize_admin(supplied)?;
        match self.world.issue(display_name, WorldScopes::GUEST).await {
            Ok((participant, capability)) => Ok(InviteResponse {
                capability: capability.expose().to_owned(),
                participant_id: participant.id,
                display_name: participant.display_name,
            }),
            Err(_) => Err(InviteError::Unavailable),
        }
    }

    /// Validate the separately configured world-administrator token.
    pub fn authorize_admin(&self, supplied: Option<&str>) -> Result<(), InviteError> {
        let Some(expected) = self.admin_token.as_deref().filter(|t| !t.is_empty()) else {
            return Err(InviteError::Disabled);
        };
        if !supplied.is_some_and(|s| {
            s.len() <= MAX_ADMIN_TOKEN_BYTES && secret_eq(expected.as_bytes(), s.as_bytes())
        }) {
            return Err(InviteError::Unauthorized);
        }
        Ok(())
    }

    /// Compile, instantiate and atomically install a Wasm program. The module
    /// is first pinned in Iroh's blob store; only then does the actor activate
    /// it. A failed compile or write leaves the running program unchanged.
    #[cfg(feature = "sandbox")]
    pub async fn install_wasm(
        &self,
        supplied_admin: Option<&str>,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
    ) -> Result<(String, u64), InstallError> {
        let permit = self.try_reserve_module_upload()?;
        self.install_wasm_reserved(supplied_admin, wasm, seed, claimed_hash, permit)
            .await
    }

    #[cfg(feature = "sandbox")]
    async fn install_wasm_reserved(
        &self,
        supplied_admin: Option<&str>,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
        _permit: OwnedSemaphorePermit,
    ) -> Result<(String, u64), InstallError> {
        self.authorize_admin(supplied_admin)
            .map_err(|error| match error {
                InviteError::Disabled => InstallError::Disabled,
                InviteError::Unauthorized => InstallError::Unauthorized,
                InviteError::Unavailable => InstallError::Unavailable,
            })?;
        if wasm.is_empty() || wasm.len() > MAX_WORLD_MODULE_BYTES {
            return Err(InstallError::InvalidUpload);
        }
        let hash = Hash::new(&wasm);
        let module_hash = hash.to_string();
        if claimed_hash != module_hash {
            return Err(InstallError::InvalidUpload);
        }
        let blobs = self.blobs.as_ref().ok_or(InstallError::Disabled)?.clone();
        let limits = crate::sandbox::SandboxLimits {
            module_bytes: MAX_WORLD_MODULE_BYTES,
            ..crate::sandbox::SandboxLimits::default()
        };
        let compile_bytes = wasm.clone();
        let guest = tokio::task::spawn_blocking(move || {
            let sandbox = crate::sandbox::Sandbox::compile(&compile_bytes, limits)?;
            sandbox.instantiate(seed)
        })
        .await
        .map_err(|_| InstallError::InvalidModule)?
        .map_err(|_| InstallError::InvalidModule)?;

        // A durable world must be able to reload this module before the
        // journal may reference it.
        if let Some(modules) = self.module_dir.clone() {
            let bytes = wasm.clone();
            let saved = tokio::task::spawn_blocking(move || modules.save(&bytes))
                .await
                .map_err(|_| InstallError::Unavailable)?
                .map_err(|_| InstallError::Unavailable)?;
            if saved != module_hash {
                return Err(InstallError::Unavailable);
            }
        }

        let pin = blobs
            .add_bytes_with_opts(AddBytesOptions {
                data: wasm.into(),
                format: BlobFormat::Raw,
            })
            .temp_tag()
            .await
            .map_err(|_| InstallError::Unavailable)?;
        if pin.hash() != hash {
            return Err(InstallError::Unavailable);
        }
        let installed = self
            .world
            .install_program(module_hash.clone(), seed, Box::new(guest))
            .await
            .map_err(|_| InstallError::Unavailable)?;
        if !matches!(
            installed.kind,
            crate::world::WorldEventKind::ProgramInstalled { .. }
        ) {
            return Err(InstallError::Unavailable);
        }
        *self.module_pin.lock().await = Some(pin);
        *self.module_hash.write().await = Some(module_hash.clone());
        Ok((module_hash, installed.sequence))
    }

    /// Read the currently pinned module artifact for peer download.
    /// The caller must already have authenticated as a world participant.
    pub async fn module_bytes(&self, expected_hash: &str) -> Result<Vec<u8>, InstallError> {
        let blobs = self.blobs.as_ref().ok_or(InstallError::Disabled)?;
        let pin = self.module_pin.lock().await;
        let Some(pin) = pin.as_ref() else {
            return Err(InstallError::Unavailable);
        };
        if pin.hash().to_string() != expected_hash {
            return Err(InstallError::InvalidUpload);
        }
        let bytes = blobs
            .blobs()
            .get_bytes(pin.hash())
            .await
            .map_err(|_| InstallError::Unavailable)?;
        if bytes.len() > MAX_WORLD_MODULE_BYTES || module_hash(&bytes) != expected_hash {
            return Err(InstallError::Unavailable);
        }
        Ok(bytes.to_vec())
    }

    /// Hash of the currently active module, if the world runs one.
    pub async fn active_module_hash(&self) -> Option<String> {
        self.module_hash.read().await.clone()
    }
}

/// Canonical content identifier expected in an install-begin frame.
#[must_use]
pub fn module_hash(wasm: &[u8]) -> String {
    Hash::new(wasm).to_string()
}

/// Constant-time (for equal lengths) secret comparison.
#[must_use]
pub fn secret_eq(expected: &[u8], supplied: &[u8]) -> bool {
    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
}

/// Run one session to completion with default settings.
pub async fn run_session<I: SessionIo>(service: &WorldService, io: &mut I) {
    run_session_with(service, io, SessionConfig::default()).await;
}

/// Run one session to completion.
pub async fn run_session_with<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    config: SessionConfig,
) {
    let first = match tokio::time::timeout(config.join_timeout, io.recv()).await {
        Ok(Inbound::Text(text)) => text,
        Ok(Inbound::Oversized) => {
            let _ = send_error(io, "world frame is too large").await;
            return;
        }
        Ok(Inbound::Unsupported) => {
            let _ = send_error(io, "frames must be UTF-8 text").await;
            return;
        }
        Ok(Inbound::Closed) => return,
        Err(_) => {
            let _ = send_error(io, "timed out waiting for join").await;
            return;
        }
    };
    let Ok(frame) = serde_json::from_str::<ClientFrame>(&first) else {
        let _ = send_error(io, "first frame must be a valid join or invite request").await;
        return;
    };
    match frame {
        ClientFrame::Join { capability, after } => {
            if capability.len() > MAX_CAPABILITY_BYTES {
                let _ = send_error(io, "invalid join capability").await;
                return;
            }
            joined(service, io, JoinCapability::from_wire(capability), after).await;
        }
        ClientFrame::Invite {
            admin_token,
            display_name,
        } => match service.invite(Some(&admin_token), display_name).await {
            Ok(invite) => {
                let _ = send_json(io, &ServerFrame::Invite(&invite)).await;
            }
            Err(InviteError::Disabled) => {
                let _ = send_error(io, "invites are not enabled").await;
            }
            Err(InviteError::Unauthorized) => {
                let _ = send_error(io, "invite denied").await;
            }
            Err(InviteError::Unavailable) => {
                let _ = send_error(io, "world cannot issue another invite").await;
            }
        },
        // Query frames keep the session open for further queries: a client
        // can page through records, and may still join later on.
        ClientFrame::Info { .. }
        | ClientFrame::Records { .. }
        | ClientFrame::RecordsTicket { .. }
        | ClientFrame::DownloadModule { .. } => {
            query_session(service, io, frame).await;
        }
        ClientFrame::InstallBegin {
            admin_token,
            total_bytes,
            module_hash,
            seed,
        } => {
            if let Err(message) = install_module_session(
                service,
                io,
                &admin_token,
                total_bytes,
                module_hash.as_deref(),
                seed,
            )
            .await
            {
                let _ = send_error(io, message).await;
            }
        }
        ClientFrame::InstallChunk { .. }
        | ClientFrame::InstallEnd
        | ClientFrame::Chat { .. }
        | ClientFrame::Input { .. } => {
            let _ = send_error(io, "first frame must be join, invite or install_begin").await;
        }
    }
}

/// Serve query frames (`info`, `records`, `records_ticket`,
/// `download_module`) until the peer closes, denies, or sends a frame that
/// ends the session. A `join` frame switches to the joined session.
async fn query_session<I: SessionIo>(service: &WorldService, io: &mut I, first: ClientFrame) {
    let mut next = Some(first);
    loop {
        let frame = match next.take() {
            Some(frame) => frame,
            None => match io.recv().await {
                Inbound::Text(text) => match serde_json::from_str::<ClientFrame>(&text) {
                    Ok(frame) => frame,
                    Err(_) => {
                        let _ = send_error(io, "invalid world frame").await;
                        return;
                    }
                },
                Inbound::Oversized => {
                    let _ = send_error(io, "world frame is too large").await;
                    return;
                }
                Inbound::Unsupported => {
                    let _ = send_error(io, "frames must be UTF-8 text").await;
                    return;
                }
                Inbound::Closed => return,
            },
        };
        match frame {
            ClientFrame::Join { capability, after } => {
                if capability.len() > MAX_CAPABILITY_BYTES {
                    let _ = send_error(io, "invalid join capability").await;
                    return;
                }
                joined(service, io, JoinCapability::from_wire(capability), after).await;
                return;
            }
            ClientFrame::Info { capability } => {
                let token = JoinCapability::from_wire(capability);
                match service.world().snapshot(token).await {
                    Ok(snapshot) => {
                        if !send_json(
                            io,
                            &ServerFrame::ModuleInfo {
                                world_id: snapshot.world_id,
                                active_module_hash: snapshot.active_module_hash,
                                current_sequence: snapshot.current_sequence,
                            },
                        )
                        .await
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = send_error(io, "join denied").await;
                        return;
                    }
                }
            }
            ClientFrame::Records {
                capability,
                prefix,
                after,
            } => {
                let token = JoinCapability::from_wire(capability);
                if let Err(message) = records_session(
                    service.world(),
                    io,
                    token,
                    prefix.as_deref(),
                    after.as_deref(),
                )
                .await
                {
                    let _ = send_error(io, message).await;
                    return;
                }
            }
            ClientFrame::RecordsTicket { capability } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token).await.is_err() {
                    let _ = send_error(io, "join denied").await;
                    return;
                }
                match records_ticket_session(service, io).await {
                    Ok(()) => {}
                    Err(message) => {
                        let _ = send_error(io, message).await;
                        return;
                    }
                }
            }
            ClientFrame::DownloadModule {
                capability,
                module_hash,
            } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().snapshot(token).await.is_err() {
                    let _ = send_error(io, "join denied").await;
                    return;
                }
                if let Err(message) = download_module_session(service, io, &module_hash).await {
                    let _ = send_error(io, message).await;
                    return;
                }
            }
            _ => {
                let _ = send_error(io, "expected a query frame").await;
                return;
            }
        }
    }
}

/// Answer a `records_ticket` query.
async fn records_ticket_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
) -> Result<(), &'static str> {
    match service.records_store() {
        Some(store) => match store.read_ticket().await {
            Ok(ticket) => {
                if send_json(io, &ServerFrame::RecordsTicket { ticket }).await {
                    Ok(())
                } else {
                    Err("connection closed")
                }
            }
            Err(_) => Err("records ticket unavailable"),
        },
        None => Err("world keeps no structured records"),
    }
}

/// Largest page of records returned in one frame, comfortably inside
/// [`MAX_FRAME_BYTES`] after JSON framing overhead.
const MAX_RECORDS_PAGE_BYTES: usize = 768 * 1024;

async fn records_session<I: SessionIo>(
    world: &WorldHandle,
    io: &mut I,
    token: JoinCapability,
    prefix: Option<&str>,
    after: Option<&str>,
) -> Result<(), &'static str> {
    if world.participant_id(token).await.is_err() {
        return Err("join denied");
    }
    let (sequence, records) = world.records();
    let mut page = crate::world::Records::new();
    let mut used = 0usize;
    let mut last_included: Option<String> = None;
    let mut more = false;
    for (key, value) in records.iter() {
        if let Some(prefix) = prefix
            && !key.starts_with(prefix)
        {
            continue;
        }
        if let Some(after) = after
            && key.as_str() <= after
        {
            continue;
        }
        let size = key.len() + serde_json::to_vec(value).map_or(0, |v| v.len()) + 8;
        if used + size > MAX_RECORDS_PAGE_BYTES {
            // `next_after` is exclusive on the client, so it must be the
            // last key *included* here; anything else would skip a record.
            if page.is_empty() {
                return Err("a record is too large to serve in one page");
            }
            more = true;
            break;
        }
        used += size;
        last_included = Some(key.clone());
        page.insert(key.clone(), value.clone());
    }
    let next_after = more.then_some(last_included).flatten();
    if !send_json(
        io,
        &ServerFrame::Records {
            sequence,
            records: page,
            next_after,
        },
    )
    .await
    {
        return Err("connection closed");
    }
    Ok(())
}

async fn download_module_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    requested_hash: &str,
) -> Result<(), &'static str> {
    let Some(active_hash) = service.active_module_hash().await else {
        return Err("world has no installed module");
    };
    if requested_hash != active_hash {
        return Err("requested module is not the active world module");
    }
    let bytes = service
        .module_bytes(&active_hash)
        .await
        .map_err(|_| "module bytes are unavailable")?;
    if !send_json(
        io,
        &ServerFrame::ModuleBegin {
            module_hash: active_hash.clone(),
            total_bytes: bytes.len(),
        },
    )
    .await
    {
        return Err("connection closed");
    }
    for (index, chunk) in bytes.chunks(MAX_WORLD_MODULE_CHUNK_BYTES).enumerate() {
        if !send_json(
            io,
            &ServerFrame::ModuleChunk {
                offset: index * MAX_WORLD_MODULE_CHUNK_BYTES,
                data_hex: encode_hex(chunk),
            },
        )
        .await
        {
            return Err("connection closed");
        }
    }
    if !send_json(
        io,
        &ServerFrame::ModuleEnd {
            module_hash: active_hash,
        },
    )
    .await
    {
        return Err("connection closed");
    }
    Ok(())
}

async fn install_module_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    admin_token: &str,
    total_bytes: usize,
    claimed_hash: Option<&str>,
    seed: u64,
) -> Result<(), &'static str> {
    if admin_token.len() > MAX_ADMIN_TOKEN_BYTES
        || service.authorize_admin(Some(admin_token)).is_err()
    {
        return Err("module install denied");
    }
    if total_bytes == 0
        || total_bytes > MAX_WORLD_MODULE_BYTES
        || claimed_hash.is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err("invalid module size or content hash");
    }
    let upload_permit = service
        .try_reserve_module_upload()
        .map_err(|_| "world is busy installing another module")?;
    #[cfg(not(feature = "sandbox"))]
    {
        let _ = (io, seed, upload_permit);
        return Err("module installation requires the sandbox feature");
    }

    #[cfg(feature = "sandbox")]
    {
        if !send_json(
            io,
            &ServerFrame::UploadReady {
                chunk_bytes: MAX_WORLD_MODULE_CHUNK_BYTES,
            },
        )
        .await
        {
            return Err("connection closed");
        }
        let mut bytes = Vec::with_capacity(total_bytes);
        while bytes.len() < total_bytes {
            let incoming = tokio::time::timeout(Duration::from_secs(30), io.recv())
                .await
                .map_err(|_| "timed out receiving module")?;
            let Inbound::Text(text) = incoming else {
                return Err("invalid module chunk frame");
            };
            let Ok(ClientFrame::InstallChunk { offset, data_hex }) =
                serde_json::from_str::<ClientFrame>(&text)
            else {
                return Err("expected module chunk frame");
            };
            if offset != bytes.len() || data_hex.len() > MAX_WORLD_MODULE_CHUNK_BYTES * 2 {
                return Err("invalid module chunk offset or size");
            }
            let Some(chunk) = decode_hex(&data_hex) else {
                return Err("module chunk must be hexadecimal");
            };
            if chunk.is_empty() || chunk.len() > total_bytes.saturating_sub(bytes.len()) {
                return Err("invalid module chunk length");
            }
            bytes.extend_from_slice(&chunk);
        }

        let incoming = tokio::time::timeout(Duration::from_secs(30), io.recv())
            .await
            .map_err(|_| "timed out waiting for install completion")?;
        if !matches!(incoming, Inbound::Text(ref text) if matches!(serde_json::from_str::<ClientFrame>(text), Ok(ClientFrame::InstallEnd)))
        {
            return Err("expected install_end frame");
        }
        let actual_hash = module_hash(&bytes);
        if claimed_hash.is_some_and(|claimed| actual_hash != claimed.to_ascii_lowercase()) {
            return Err("module content hash mismatch");
        }
        let (module_hash, sequence) = service
            .install_wasm_reserved(Some(admin_token), bytes, seed, &actual_hash, upload_permit)
            .await
            .map_err(|error| match error {
                InstallError::Disabled => "module installation is not enabled on this host",
                InstallError::Unauthorized => "module install denied",
                InstallError::InvalidUpload => "invalid module upload",
                InstallError::InvalidModule => "Wasm module failed validation or initialization",
                InstallError::Unavailable => "world could not activate the module",
            })?;
        if !send_json(
            io,
            &ServerFrame::ModuleInstalled {
                module_hash: &module_hash,
                sequence,
            },
        )
        .await
        {
            return Err("connection closed");
        }
        Ok(())
    }
}

async fn joined<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: JoinCapability,
    after: Option<u64>,
) {
    let world = service.world();
    // Subscribe before taking the snapshot so no event can fall in the gap;
    // events at or below the cursor are skipped when delivered.
    let mut events = world.subscribe();
    let mut revocations = world.subscribe_revocations();
    let Ok(participant_id) = world.participant_id(token.clone()).await else {
        let _ = send_error(io, "join denied").await;
        return;
    };
    let Ok(snapshot) = world.snapshot(token.clone()).await else {
        let _ = send_error(io, "join denied").await;
        return;
    };
    let mut cursor = snapshot.current_sequence;
    if let Some(after) = after {
        match world.events_after(token.clone(), after).await {
            Ok(page) if page.needs_snapshot => {
                let Ok(fresh) = world.snapshot(token.clone()).await else {
                    return;
                };
                cursor = fresh.current_sequence;
                if !send_json(io, &ServerFrame::Snapshot { snapshot: &fresh }).await {
                    return;
                }
            }
            Ok(page) => {
                cursor = page.current_sequence;
                for event in &page.events {
                    if !send_json(io, &ServerFrame::Event { event }).await {
                        return;
                    }
                    if !send_view(world, io, &token, event.sequence).await {
                        return;
                    }
                }
            }
            Err(_) => {
                let _ = send_error(io, "join denied").await;
                return;
            }
        }
    } else if !send_json(
        io,
        &ServerFrame::Snapshot {
            snapshot: &snapshot,
        },
    )
    .await
    {
        return;
    }
    if !send_view(world, io, &token, cursor).await {
        return;
    }

    loop {
        tokio::select! {
            inbound = io.recv() => match inbound {
                Inbound::Text(text) => {
                    if !handle_client_frame(service, io, &token, &text).await {
                        break;
                    }
                }
                Inbound::Oversized => {
                    if !send_error(io, "world frame is too large").await { break; }
                }
                Inbound::Unsupported => {
                    if !send_error(io, "frames must be UTF-8 text").await { break; }
                }
                Inbound::Closed => break,
            },
            event = events.recv() => match event {
                Ok(event) => {
                    if let crate::world::WorldEventKind::ProgramInstalled { ref module_hash, .. } = event.kind {
                        if event.sequence > cursor {
                            let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                            if !send_json(io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                                break;
                            }
                            cursor = event.sequence;
                            if !send_json(io, &ServerFrame::ProgramChanged {
                                module_hash,
                                sequence: cursor,
                            }).await || !send_view(world, io, &token, cursor).await {
                                break;
                            }
                        }
                        continue;
                    }
                    if event.sequence <= cursor {
                        continue;
                    }
                    // Re-validating the capability on every delivery is what
                    // disconnects a revoked participant.
                    if world.snapshot(token.clone()).await.is_err() {
                        break;
                    }
                    if !send_json(io, &ServerFrame::Event { event: &event }).await {
                        break;
                    }
                    cursor = event.sequence;
                    if !send_view(world, io, &token, cursor).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                    if !send_json(io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                        break;
                    }
                    cursor = snapshot.current_sequence;
                    if !send_view(world, io, &token, cursor).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            revoked = revocations.recv() => match revoked {
                Ok(revoked_id) if revoked_id == participant_id => {
                    let _ = send_error(io, "world capability revoked").await;
                    break;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

async fn send_view<I: SessionIo>(
    world: &WorldHandle,
    io: &mut I,
    token: &JoinCapability,
    sequence: u64,
) -> bool {
    match world.view(token.clone()).await {
        Ok(Some(data)) => {
            if data.len() > MAX_FRAME_BYTES {
                return send_error(io, "world presentation is too large").await;
            }
            send_json(
                io,
                &ServerFrame::View {
                    sequence,
                    data: &data,
                },
            )
            .await
        }
        Ok(None) => true,
        Err(_) => false,
    }
}

/// Handle one frame from a joined participant. Returns `false` to end the session.
async fn handle_client_frame<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: &JoinCapability,
    text: &str,
) -> bool {
    let world = service.world();
    let Ok(frame) = serde_json::from_str::<ClientFrame>(text) else {
        return send_error(io, "invalid world frame").await;
    };
    let result = match frame {
        ClientFrame::Chat { text } => world.chat(token.clone(), text).await,
        ClientFrame::Input { data_hex } => match decode_hex(&data_hex) {
            Some(data) => world.input(token.clone(), data).await,
            None => return send_error(io, "input data must be hexadecimal").await,
        },
        ClientFrame::Records { prefix, after, .. } => {
            // Authorized by the session's own capability.
            if let Err(message) = records_session(
                world,
                io,
                token.clone(),
                prefix.as_deref(),
                after.as_deref(),
            )
            .await
            {
                return send_error(io, message).await;
            }
            return true;
        }
        ClientFrame::RecordsTicket { .. } => {
            if let Err(message) = records_ticket_session(service, io).await {
                return send_error(io, message).await;
            }
            return true;
        }
        ClientFrame::Join { .. }
        | ClientFrame::Invite { .. }
        | ClientFrame::Info { .. }
        | ClientFrame::DownloadModule { .. }
        | ClientFrame::InstallBegin { .. }
        | ClientFrame::InstallChunk { .. }
        | ClientFrame::InstallEnd => {
            return send_error(io, "already joined").await;
        }
    };
    // Accepted events reach everyone, including the sender, through the
    // broadcast branch, so only rejections are answered directly.
    result.is_ok() || send_error(io, "world event rejected").await
}

async fn send_json<I: SessionIo>(io: &mut I, frame: &impl Serialize) -> bool {
    match serde_json::to_string(frame) {
        Ok(encoded) if encoded.len() <= MAX_FRAME_BYTES => io.send(encoded).await,
        _ => false,
    }
}

async fn send_error<I: SessionIo>(io: &mut I, message: &str) -> bool {
    send_json(io, &ServerFrame::Error { message }).await
}

/// Decode a bounded hex string.
#[must_use]
pub fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) || encoded.len() > MAX_FRAME_BYTES * 2 {
        return None;
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::world::{WorldCore, WorldLimits};

    /// In-memory transport: tests push frames in and read frames out.
    struct ChannelIo {
        inbound: mpsc::Receiver<Inbound>,
        outbound: mpsc::Sender<String>,
    }

    impl SessionIo for ChannelIo {
        async fn recv(&mut self) -> Inbound {
            self.inbound.recv().await.unwrap_or(Inbound::Closed)
        }
        async fn send(&mut self, frame: String) -> bool {
            self.outbound.send(frame).await.is_ok()
        }
    }

    struct Peer {
        to_server: mpsc::Sender<Inbound>,
        from_server: mpsc::Receiver<String>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Peer {
        fn connect(service: WorldService, config: SessionConfig) -> Self {
            let (to_server, inbound) = mpsc::channel(16);
            let (outbound, from_server) = mpsc::channel(64);
            let task = tokio::spawn(async move {
                let mut io = ChannelIo { inbound, outbound };
                run_session_with(&service, &mut io, config).await;
            });
            Self {
                to_server,
                from_server,
                task,
            }
        }

        async fn say(&self, frame: serde_json::Value) {
            self.to_server
                .send(Inbound::Text(frame.to_string()))
                .await
                .unwrap();
        }

        async fn next(&mut self) -> Option<serde_json::Value> {
            let frame = tokio::time::timeout(Duration::from_secs(5), self.from_server.recv())
                .await
                .expect("timed out waiting for a server frame")?;
            Some(serde_json::from_str(&frame).unwrap())
        }
    }

    /// Publishes 1024 records of about 1 KiB, so one query needs several
    /// pages (the page budget is 768 KiB).
    struct Bulky;

    impl crate::world::WorldProgram for Bulky {
        fn records(&mut self) -> anyhow::Result<Option<String>> {
            let payload = "x".repeat(1024);
            let records: crate::world::Records = (0..1024)
                .map(|i| {
                    (
                        format!("key-{i:04}"),
                        serde_json::Value::from(payload.clone()),
                    )
                })
                .collect();
            Ok(Some(serde_json::to_string(&records)?))
        }
    }

    fn service_with_records(admin: Option<&str>) -> WorldService {
        WorldService::new(
            WorldHandle::spawn_with_program(
                WorldCore::new("lobby", WorldLimits::default()).unwrap(),
                Box::new(Bulky),
            ),
            admin.map(str::to_owned),
        )
    }

    fn service(admin: Option<&str>) -> WorldService {
        WorldService::new(
            WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap()),
            admin.map(str::to_owned),
        )
    }

    #[test]
    fn hex_input_parser_is_bounded_and_strict() {
        assert_eq!(decode_hex("00aBff"), Some(vec![0, 0xab, 0xff]));
        assert_eq!(decode_hex(""), Some(vec![]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("zz"), None);
        assert_eq!(decode_hex(&"0".repeat(MAX_FRAME_BYTES * 2 + 2)), None);
    }

    #[test]
    fn wire_event_and_view_shape_are_presentation_neutral() {
        use crate::world::WorldEventKind;
        let event = WorldEvent {
            sequence: 3,
            participant_id: 9,
            kind: WorldEventKind::Chat("hello".to_owned()),
        };
        let encoded = serde_json::to_string(&ServerFrame::Event { event: &event }).unwrap();
        assert!(encoded.contains("\"type\":\"event\""));
        assert!(encoded.contains("\"sequence\":3"));
        assert!(encoded.contains("\"kind\":\"chat\""));
        assert!(encoded.contains("\"data\":\"hello\""));
        let view = serde_json::to_string(&ServerFrame::View {
            sequence: 3,
            data: "count=1",
        })
        .unwrap();
        assert!(view.contains("\"type\":\"view\""));
        assert!(view.contains("\"data\":\"count=1\""));
    }

    #[test]
    fn secret_comparison_requires_equal_bytes() {
        assert!(secret_eq(b"abc", b"abc"));
        assert!(!secret_eq(b"abc", b"abd"));
        assert!(!secret_eq(b"abc", b"ab"));
    }

    #[tokio::test]
    async fn silent_peers_are_dropped_after_the_join_timeout() {
        let mut peer = Peer::connect(
            service(None),
            SessionConfig {
                join_timeout: Duration::from_millis(50),
            },
        );
        let frame = peer.next().await.unwrap();
        assert_eq!(frame["type"], "error");
        assert!(frame["message"].as_str().unwrap().contains("timed out"));
        assert!(peer.next().await.is_none());
        peer.task.await.unwrap();
    }

    #[tokio::test]
    async fn first_frame_must_be_join_or_invite() {
        let mut peer = Peer::connect(service(Some("a")), SessionConfig::default());
        peer.say(serde_json::json!({"type": "chat", "text": "hi"}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");
        assert!(peer.next().await.is_none());

        let mut garbage = Peer::connect(service(None), SessionConfig::default());
        garbage
            .to_server
            .send(Inbound::Text("not json".to_owned()))
            .await
            .unwrap();
        assert_eq!(garbage.next().await.unwrap()["type"], "error");

        let mut big = Peer::connect(service(None), SessionConfig::default());
        big.to_server.send(Inbound::Oversized).await.unwrap();
        assert_eq!(big.next().await.unwrap()["type"], "error");
        let mut bin = Peer::connect(service(None), SessionConfig::default());
        bin.to_server.send(Inbound::Unsupported).await.unwrap();
        assert_eq!(bin.next().await.unwrap()["type"], "error");
    }

    #[tokio::test]
    async fn invite_frame_issues_a_capability_that_can_join() {
        let svc = service(Some("admin"));

        let mut denied = Peer::connect(svc.clone(), SessionConfig::default());
        denied
            .say(serde_json::json!({"type":"invite","admin_token":"nope","display_name":"x"}))
            .await;
        assert_eq!(denied.next().await.unwrap()["message"], "invite denied");

        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({"type":"invite","admin_token":"admin","display_name":"ann"}))
            .await;
        let invite = host.next().await.unwrap();
        assert_eq!(invite["type"], "invite");
        let capability = invite["capability"].as_str().unwrap().to_owned();

        let mut guest = Peer::connect(svc, SessionConfig::default());
        guest
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(guest.next().await.unwrap()["type"], "snapshot");
    }

    #[tokio::test]
    async fn records_are_paged_prefixed_and_queryable_while_joined() {
        let svc = service_with_records(None);
        let (_, token) = svc
            .world()
            .issue("ann".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();

        // A fresh session can page through records; pages must be bounded and
        // terminate with `next_after: null`.
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        let mut pages = 0;
        let mut seen = crate::world::Records::new();
        let mut after: Option<String> = None;
        loop {
            let mut frame = serde_json::json!({
                "type": "records",
                "capability": token.expose(),
            });
            if let Some(after) = &after {
                frame["after"] = serde_json::Value::from(after.clone());
            }
            peer.say(frame).await;
            let reply = peer.next().await.unwrap();
            assert_eq!(reply["type"], "records");
            assert!(reply["sequence"].as_u64().is_some());
            let page: crate::world::Records =
                serde_json::from_value(reply["records"].clone()).unwrap();
            assert!(!page.is_empty());
            assert!(
                serde_json::to_vec(&page).unwrap().len() <= MAX_RECORDS_PAGE_BYTES,
                "page exceeds the byte budget"
            );
            pages += 1;
            seen.extend(page);
            match reply["next_after"].as_str() {
                Some(next) => after = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(seen.len(), 1024, "every record arrives exactly once");
        assert!(pages > 1, "1 MiB of records cannot fit in one page");
        assert!(seen.contains_key("key-0000") && seen.contains_key("key-1023"));

        // Prefix filtering works on the same session.
        peer.say(serde_json::json!({
            "type": "records",
            "capability": token.expose(),
            "prefix": "key-010",
        }))
        .await;
        let reply = peer.next().await.unwrap();
        let page: crate::world::Records = serde_json::from_value(reply["records"].clone()).unwrap();
        assert_eq!(page.len(), 10, "key-0100..key-0109");

        // A joined session can also request the records document ticket (the
        // reply is an error here only because this service has no docs store).
        let mut joined = Peer::connect(svc, SessionConfig::default());
        joined
            .say(serde_json::json!({"type":"join","capability": token.expose()}))
            .await;
        assert_eq!(joined.next().await.unwrap()["type"], "snapshot");
        joined
            .say(serde_json::json!({"type":"records","capability": token.expose()}))
            .await;
        let reply = joined.next().await.unwrap();
        assert_eq!(reply["type"], "records");
        let page: crate::world::Records = serde_json::from_value(reply["records"].clone()).unwrap();
        assert!(!page.is_empty() && page.len() < 1024, "one bounded page");
        assert!(
            reply["next_after"].is_string(),
            "the page advertises that more records follow"
        );
        // Still joined afterwards.
        joined
            .say(serde_json::json!({"type":"chat","text":"hi"}))
            .await;
        let reply = joined.next().await.unwrap();
        assert_eq!(reply["type"], "event");
    }

    #[tokio::test]
    async fn records_queries_need_a_valid_capability() {
        let svc = service_with_records(None);
        let mut peer = Peer::connect(svc, SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "records",
            "capability": "0.00",
        }))
        .await;
        assert_eq!(peer.next().await.unwrap()["message"], "join denied");
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn admin_upload_pins_validates_and_activates_a_roc_program() {
        use iroh_blobs::store::mem::MemStore;

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let blobs: Store = MemStore::new().into();
        let svc = WorldService::new(world.clone(), Some("admin".to_owned()))
            .with_blob_store(blobs.clone());
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let hash = module_hash(wasm);

        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "install_begin",
            "admin_token": "wrong",
            "total_bytes": wasm.len(),
            "module_hash": hash,
            "seed": 12,
        }))
        .await;
        assert_eq!(
            peer.next().await.unwrap()["message"],
            "module install denied"
        );

        let (_, guest) = world
            .issue("ada".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let mut peer = Peer::connect(svc, SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "install_begin",
            "admin_token": "admin",
            "total_bytes": wasm.len(),
            "module_hash": null,
            "seed": 12,
        }))
        .await;
        let ready = peer.next().await.unwrap();
        assert_eq!(ready["type"], "upload_ready");
        let chunk_bytes = ready["chunk_bytes"].as_u64().unwrap() as usize;
        for (index, chunk) in wasm.chunks(chunk_bytes).enumerate() {
            peer.say(serde_json::json!({
                "type": "install_chunk",
                "offset": index * chunk_bytes,
                "data_hex": encode_hex(chunk),
            }))
            .await;
        }
        peer.say(serde_json::json!({"type": "install_end"})).await;
        let ack = peer.next().await.unwrap();
        assert_eq!(ack["type"], "module_installed");
        assert_eq!(ack["module_hash"], hash);

        let hash: Hash = hash.parse().unwrap();
        assert!(
            blobs.blobs().has(hash).await.unwrap(),
            "uploaded module is pinned in the blob store"
        );
        assert_eq!(
            world.view(guest.clone()).await.unwrap().as_deref(),
            Some("count=0")
        );
        world.input(guest.clone(), b"inc".to_vec()).await.unwrap();
        assert_eq!(world.view(guest).await.unwrap().as_deref(), Some("count=1"));
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn bad_module_replacement_preserves_the_active_program() {
        use iroh_blobs::store::mem::MemStore;

        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let sandbox =
            crate::sandbox::Sandbox::compile(wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("counter", WorldLimits::default()).unwrap(),
            Box::new(sandbox.instantiate(1).unwrap()),
        );
        let blobs: Store = MemStore::new().into();
        let service =
            WorldService::new(world.clone(), Some("admin".to_owned())).with_blob_store(blobs);
        let (_, guest) = world.issue("guest", WorldScopes::GUEST).await.unwrap();
        assert_eq!(
            world.view(guest.clone()).await.unwrap().as_deref(),
            Some("count=0")
        );

        let bad = b"not wasm".to_vec();
        let hash = module_hash(&bad);
        assert_eq!(
            service
                .install_wasm(Some("admin"), bad, 2, &hash)
                .await
                .unwrap_err(),
            InstallError::InvalidModule
        );
        assert_eq!(
            world.view(guest.clone()).await.unwrap().as_deref(),
            Some("count=0")
        );
        world.input(guest.clone(), b"inc".to_vec()).await.unwrap();
        assert_eq!(world.view(guest).await.unwrap().as_deref(), Some("count=1"));
    }

    #[tokio::test]
    async fn invites_are_disabled_without_an_admin_secret() {
        let mut peer = Peer::connect(service(None), SessionConfig::default());
        peer.say(serde_json::json!({"type":"invite","admin_token":"","display_name":"x"}))
            .await;
        assert_eq!(
            peer.next().await.unwrap()["message"],
            "invites are not enabled"
        );
    }

    #[tokio::test]
    async fn chat_is_broadcast_and_rejections_are_private() {
        let svc = service(None);
        let (_, alice) = svc
            .world()
            .issue("alice".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, watcher) = svc
            .world()
            .issue("watcher".to_owned(), WorldScopes::JOIN)
            .await
            .unwrap();
        let mut a = Peer::connect(svc.clone(), SessionConfig::default());
        let mut w = Peer::connect(svc, SessionConfig::default());
        a.say(serde_json::json!({"type":"join","capability":alice.expose()}))
            .await;
        w.say(serde_json::json!({"type":"join","capability":watcher.expose()}))
            .await;
        assert_eq!(a.next().await.unwrap()["type"], "snapshot");
        assert_eq!(w.next().await.unwrap()["type"], "snapshot");

        // A join-only participant cannot speak; only they hear about it.
        w.say(serde_json::json!({"type":"chat","text":"psst"}))
            .await;
        assert_eq!(w.next().await.unwrap()["message"], "world event rejected");

        a.say(serde_json::json!({"type":"chat","text":"hello"}))
            .await;
        for peer in [&mut a, &mut w] {
            let frame = peer.next().await.unwrap();
            assert_eq!(frame["type"], "event");
            assert!(frame.to_string().contains("hello"));
        }
        a.say(serde_json::json!({"type":"input","data_hex":"xyz"}))
            .await;
        assert_eq!(
            a.next().await.unwrap()["message"],
            "input data must be hexadecimal"
        );
        a.say(serde_json::json!({"type":"join","capability":"x"}))
            .await;
        assert_eq!(a.next().await.unwrap()["message"], "already joined");
    }
}
