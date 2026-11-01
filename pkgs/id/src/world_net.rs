//! Iroh transport for world sessions (`/id-world/1`).
//!
//! One bidirectional QUIC stream carries one session. A frame is a big-endian
//! `u32` byte length followed by one UTF-8 JSON object of at most
//! [`crate::frames::MAX_FRAME_BYTES`]; the JSON is exactly the protocol in
//! [`crate::world_session`], so this module only moves frames.
//!
//! Iroh proves *which node* connected. That is logged for operators but never
//! grants anything: the join capability inside the session is the only
//! authority over a world.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
};

use crate::abuse::{AbuseEvaluator, AbusePolicy, Counters, EndpointId as AbuseEndpointId, NodeId};
use crate::frames::{charge_frame, read_frame_body, read_frame_len};
use crate::world_hub::WorldHub;
use crate::world_session::{
    Inbound, MAX_WORLD_MODULE_CHUNK_BYTES, SessionIo, module_hash, run_session,
};

/// ALPN for world sessions. Within version 1 frames may gain fields and
/// variants; removing or changing one needs `/id-world/2`.
pub const WORLD_ALPN: &[u8] = b"/id-world/1";

/// Public module state returned by a world's `info` frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldModuleInfo {
    /// World identifier.
    pub world_id: String,
    /// Hash of the active module, if any.
    pub active_module_hash: Option<String>,
    /// Server-observed build origin, if recorded by this host.
    pub active_module_provenance: Option<crate::world::ModuleProvenance>,
    /// Latest committed world sequence.
    pub current_sequence: u64,
}

/// Concurrent sessions allowed on one connection.
const MAX_SESSIONS_PER_CONNECTION: usize = 16;
/// Keep operator-visible transport counters for the most recent day.
const ABUSE_COUNTER_RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;

use crate::world_session::encode_hex;

/// Re-exported so `crate::world_net::{read_frame, write_frame}` keeps working:
/// the framing helpers now live in [`crate::frames`] so the crate builds
/// without the `world` feature.
pub use crate::frames::{read_frame, write_frame};

/// Accepts world sessions over Iroh. Register with
/// `Router::builder(..).accept(WORLD_ALPN, WorldProtocol::new(hub))`.
#[derive(Clone, Debug)]
pub struct WorldProtocol {
    hub: WorldHub,
    /// The shared #2244 P0-A abuse evaluator: one bucket set per remote
    /// endpoint, shared across every stream and connection that
    /// endpoint opens, plus the counter registry operators read via
    /// [`WorldProtocol::abuse_counters`].
    abuse: Arc<Mutex<AbuseEvaluator>>,
}

/// Monotonic milliseconds for the abuse evaluator, from a process-wide
/// epoch. The evaluator only compares deltas, so the epoch is arbitrary.
fn abuse_now_ms() -> u64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl WorldProtocol {
    /// Serve a hub of worlds (or a single [`WorldService`], which converts
    /// into a one-world hub) to Iroh peers.
    #[must_use]
    pub fn new(hub: impl Into<WorldHub>) -> Self {
        Self::with_local_node(hub, [0; 32])
    }

    /// Like [`WorldProtocol::new`], with the serving node's id recorded
    /// as the local side of every counter-registry row. The zero id from
    /// `new` means "this serving node"; rows are keyed by remote
    /// endpoint either way.
    #[must_use]
    pub fn with_local_node(hub: impl Into<WorldHub>, local_node: NodeId) -> Self {
        Self {
            hub: hub.into(),
            abuse: Arc::new(Mutex::new(AbuseEvaluator::new(
                AbusePolicy::default(),
                local_node,
            ))),
        }
    }

    /// Snapshot of the per-(local node, remote endpoint) frame/byte
    /// counters recorded by the shared evaluator — the operator
    /// visibility surface for #2244 P0-A charging.
    #[must_use]
    pub fn abuse_counters(&self) -> Vec<((NodeId, AbuseEndpointId), Counters)> {
        self.abuse
            .lock()
            .map(|abuse| abuse.counters().snapshot())
            .unwrap_or_default()
    }

    fn prune_abuse_state(&self) {
        if let Ok(mut abuse) = self.abuse.lock() {
            let now_ms = abuse_now_ms();
            abuse.prune_idle_endpoints(now_ms);
            abuse.prune_counters(now_ms, ABUSE_COUNTER_RETENTION_MS);
        }
    }
}

impl ProtocolHandler for WorldProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // Bound stale endpoint and counter state as new connections arrive;
        // active/drained buckets are retained until eviction is behaviorally
        // equivalent to starting with a fresh endpoint.
        self.prune_abuse_state();
        let remote = conn.remote_id();
        tracing::debug!("world: connection from node {remote}");
        let permits = Arc::new(Semaphore::new(MAX_SESSIONS_PER_CONNECTION));
        let mut sessions = JoinSet::new();
        while let Ok((mut send, recv)) = conn.accept_bi().await {
            let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                // Over the per-connection bound: refuse this stream only.
                let _ = send.reset(1_u32.into());
                continue;
            };
            let hub = self.hub.clone();
            let abuse = Arc::clone(&self.abuse);
            let remote_endpoint: AbuseEndpointId = *remote.as_bytes();
            sessions.spawn(async move {
                let _permit = permit;
                let mut io = StreamIo::new(send, recv, abuse, remote_endpoint);
                run_session(&hub, &mut io).await;
                io.finish();
            });
        }
        // The peer closed the connection; wind down whatever is left.
        sessions.shutdown().await;
        Ok(())
    }
}

/// [`SessionIo`] over one QUIC stream pair.
///
/// A reader task owns the receive half and forwards parsed frames over a
/// channel, which makes [`SessionIo::recv`] cancel-safe.
struct StreamIo {
    frames: mpsc::Receiver<Inbound>,
    send: SendStream,
    reader: JoinHandle<()>,
    abuse: Arc<Mutex<AbuseEvaluator>>,
    remote: AbuseEndpointId,
    authorized: Arc<AtomicBool>,
}

impl StreamIo {
    fn new(
        send: SendStream,
        mut recv: RecvStream,
        abuse: Arc<Mutex<AbuseEvaluator>>,
        remote: AbuseEndpointId,
    ) -> Self {
        let (tx, frames) = mpsc::channel(8);
        let reader_abuse = Arc::clone(&abuse);
        let reader_authorized = Arc::new(AtomicBool::new(false));
        let reader_authorized_task = Arc::clone(&reader_authorized);
        let reader = tokio::spawn(async move {
            loop {
                // Charge after the length prefix, before the body is
                // allocated (frames::read_frame_len / read_frame_body
                // split), holding the shared lock only for the
                // synchronous charge — never across the body read. A
                // refusal sheds this stream only; the peer connection
                // and its other sessions stay up.
                let inbound = match read_frame_len(&mut recv).await {
                    Ok(Some(len)) => {
                        // The guard drops at the end of this block, before
                        // the body read's await: a refused (or poisoned)
                        // charge sheds this stream, not the connection.
                        let admitted = match reader_abuse.lock() {
                            Ok(mut abuse) => charge_frame(
                                &mut abuse,
                                remote,
                                reader_authorized_task.load(Ordering::Acquire),
                                len as usize,
                                abuse_now_ms(),
                            )
                            .is_ok(),
                            Err(_) => false,
                        };
                        if admitted {
                            match read_frame_body(&mut recv, len).await {
                                Ok(body) => match String::from_utf8(body) {
                                    Ok(text) => Inbound::Text(text),
                                    Err(_) => Inbound::Unsupported,
                                },
                                Err(_) => Inbound::Closed,
                            }
                        } else {
                            Inbound::Closed
                        }
                    }
                    Ok(None) | Err(_) => Inbound::Closed,
                };
                let stop = matches!(inbound, Inbound::Closed);
                if tx.send(inbound).await.is_err() || stop {
                    break;
                }
            }
        });
        Self {
            frames,
            send,
            reader,
            abuse,
            remote,
            authorized: reader_authorized,
        }
    }

    fn finish(&mut self) {
        let _ = self.send.finish();
    }
}

impl Drop for StreamIo {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl SessionIo for StreamIo {
    async fn recv(&mut self) -> Inbound {
        self.frames.recv().await.unwrap_or(Inbound::Closed)
    }

    async fn send(&mut self, frame: String) -> bool {
        // A throttled write waits for its retry allowance instead of
        // returning the same `false` used for a closed transport. The
        // shared lock is released before every wait and before I/O.
        let admitted = charge_frame_with_backpressure(
            &self.abuse,
            self.remote,
            self.authorized.load(Ordering::Acquire),
            frame.len(),
        )
        .await;
        if !admitted {
            return false;
        }
        write_frame(&mut self.send, frame.as_bytes()).await.is_ok()
    }

    fn mark_authorized(&mut self) {
        self.authorized.store(true, Ordering::Release);
    }
}

async fn charge_frame_with_backpressure(
    abuse: &Arc<Mutex<AbuseEvaluator>>,
    endpoint: AbuseEndpointId,
    authorized: bool,
    payload_len: usize,
) -> bool {
    loop {
        let result = match abuse.lock() {
            Ok(mut abuse) => charge_frame(
                &mut abuse,
                endpoint,
                authorized,
                payload_len,
                abuse_now_ms(),
            ),
            Err(_) => return false,
        };
        match result {
            Ok(()) => return true,
            Err(refusal) => {
                let Some(retry_after_ms) = refusal.0.retry_after_ms else {
                    return false;
                };
                tokio::time::sleep(Duration::from_millis(retry_after_ms)).await;
            }
        }
    }
}

/// The text of a client frame addressed to `world`.
///
/// The host reads the routing field from a session's first frame only;
/// adding it to every frame is harmless and keeps callers simple. Every
/// transport's client uses this, so the same frame is the same bytes on iroh
/// and on SSH.
#[must_use]
pub fn route_frame(frame: &serde_json::Value, world: Option<&str>) -> String {
    match (world, frame) {
        (Some(world), serde_json::Value::Object(object)) if !object.contains_key("world") => {
            let mut routed = object.clone();
            routed.insert("world".to_owned(), serde_json::Value::from(world));
            serde_json::Value::Object(routed).to_string()
        }
        _ => frame.to_string(),
    }
}

/// A client-side world connection (one session).
#[derive(Debug)]
pub struct WorldClient {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
    /// World named in every frame this client sends (host default if `None`).
    world: Option<String>,
}

impl WorldClient {
    /// Dial `addr` and open a session stream.
    ///
    /// # Errors
    ///
    /// Fails if the node is unreachable or does not speak [`WORLD_ALPN`].
    pub async fn connect(endpoint: &Endpoint, addr: impl Into<EndpointAddr>) -> Result<Self> {
        let conn = crate::connectivity::connect(endpoint, addr, WORLD_ALPN)
            .await
            .context("connect to world host")?;
        let (send, recv) = conn.open_bi().await.context("open world session")?;
        Ok(Self {
            conn,
            send,
            recv,
            world: None,
        })
    }

    /// Open another session on the same connection, addressed to `world`.
    /// This is how a client moves between worlds without redialing: sessions
    /// are streams, and each is bound to one world for its lifetime.
    ///
    /// # Errors
    ///
    /// Fails if the host refuses another stream on this connection.
    pub async fn open_session(&self, world: Option<&str>) -> Result<Self> {
        let (send, recv) = self.conn.open_bi().await.context("open world session")?;
        Ok(Self {
            conn: self.conn.clone(),
            send,
            recv,
            world: world.map(str::to_owned),
        })
    }

    /// Open another session on this client's connection, addressed to the
    /// same world as this client. Admin frames are served one operation per
    /// stream (the session ends with the reply), so a caller performing
    /// several operations in sequence - `id world apply` is one - opens a
    /// fresh session per operation instead of reusing a finished stream.
    ///
    /// # Errors
    ///
    /// Fails if the host refuses another stream on this connection.
    pub async fn fresh_session(&self) -> Result<Self> {
        self.open_session(self.world.as_deref()).await
    }

    /// Address this client's frames to `world` instead of the host's default.
    #[must_use]
    pub fn with_world(mut self, world: Option<&str>) -> Self {
        self.world = world.map(str::to_owned);
        self
    }

    /// Send one JSON frame.
    ///
    /// # Errors
    ///
    /// Fails if the frame is too large or the stream is closed.
    pub async fn send_json(&mut self, frame: &serde_json::Value) -> Result<()> {
        let text = route_frame(frame, self.world.as_deref());
        write_frame(&mut self.send, text.as_bytes()).await
    }

    /// Compile Roc sources on the host and install the result.
    ///
    /// # Errors
    ///
    /// Fails with the host's refusal, including compiler diagnostics.
    pub async fn compile(
        &mut self,
        admin_token: &str,
        files: Vec<(String, String)>,
        seed: u64,
        native: bool,
    ) -> Result<(String, u64, String)> {
        self.send_json(&serde_json::json!({
            "type": "compile",
            "admin_token": admin_token,
            "files": files
                .into_iter()
                .map(|(name, content)| serde_json::json!({"name": name, "content": content}))
                .collect::<Vec<_>>(),
            "seed": seed,
            "native": native,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("compiled") => Ok((
                reply["module_hash"].as_str().unwrap_or_default().to_owned(),
                reply["sequence"].as_u64().unwrap_or_default(),
                reply["diagnostics"].as_str().unwrap_or_default().to_owned(),
            )),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to compile")
            ),
        }
    }

    /// The world's capability report, as JSON.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the capability is refused.
    pub async fn caps(&mut self, capability: &str) -> Result<serde_json::Value> {
        self.send_json(&serde_json::json!({
            "type": "caps",
            "capability": capability,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "caps" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the capability report")
            );
        }
        Ok(reply["report"].clone())
    }

    /// Grant and revoke capabilities; returns the resulting report.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or names are refused.
    pub async fn update_caps(
        &mut self,
        admin_token: &str,
        grant: &[String],
        revoke: &[String],
    ) -> Result<serde_json::Value> {
        self.send_json(&serde_json::json!({
            "type": "update_caps",
            "admin_token": admin_token,
            "grant": grant,
            "revoke": revoke,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "caps" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the capability change")
            );
        }
        Ok(reply["report"].clone())
    }

    /// Replace the world's capability ceiling. The reply carries the report and
    /// a notice that the ceiling applies immediately.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or names are refused.
    pub async fn set_caps_ceiling(
        &mut self,
        admin_token: &str,
        ceiling: &[String],
    ) -> Result<serde_json::Value> {
        self.send_json(&serde_json::json!({
            "type": "set_caps_ceiling",
            "admin_token": admin_token,
            "ceiling": ceiling,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "caps_ceiling" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the capability ceiling change")
            );
        }
        Ok(reply)
    }

    /// Persist the world's requested resource profile. The response includes
    /// an explicit notice that it applies only on the next install or restart.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or profile is refused.
    /// Pend one schedule on the host (admin), replacing any pending
    /// schedule with the same id; the host journals it before replying.
    /// Returns the full pending schedule map after the change.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token is refused or the
    /// schedule is invalid.
    pub async fn set_schedule(
        &mut self,
        admin_token: &str,
        id: &str,
        at_ms: u64,
    ) -> Result<std::collections::BTreeMap<String, u64>> {
        self.send_json(&serde_json::json!({
            "type": "set_schedule",
            "admin_token": admin_token,
            "id": id,
            "at_ms": at_ms,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("schedules") => serde_json::from_value(reply["schedules"].clone())
                .context("host sent a malformed schedule map"),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to set a schedule")
            ),
        }
    }

    /// Cancel a pending schedule on the host (admin). Returns the full
    /// pending schedule map after the change.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token is refused or the
    /// schedule id is malformed.
    pub async fn cancel_schedule(
        &mut self,
        admin_token: &str,
        id: &str,
    ) -> Result<std::collections::BTreeMap<String, u64>> {
        self.send_json(&serde_json::json!({
            "type": "cancel_schedule",
            "admin_token": admin_token,
            "id": id,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("schedules") => serde_json::from_value(reply["schedules"].clone())
                .context("host sent a malformed schedule map"),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to cancel a schedule")
            ),
        }
    }

    /// Read the manifest-reconciliation state: the pending schedules, and
    /// the active install seed when `admin_token` is the host's admin token
    /// (a capability-only read gets `None` for the seed, #240).
    ///
    /// # Errors
    ///
    /// Fails with the host's message if neither credential is accepted.
    pub async fn manifest_state(
        &mut self,
        admin_token: Option<&str>,
        capability: Option<&str>,
    ) -> Result<(Option<u64>, std::collections::BTreeMap<String, u64>)> {
        self.send_json(&serde_json::json!({
            "type": "world_state",
            "admin_token": admin_token,
            "capability": capability,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("world_state") => Ok((
                reply["seed"].as_u64(),
                serde_json::from_value(reply["schedules"].clone())
                    .context("host sent a malformed schedule map")?,
            )),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to report the world state")
            ),
        }
    }

    /// Persist the world's requested resource profile. The response includes
    /// an explicit notice that it applies only on the next install or restart.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or profile is refused.
    pub async fn set_execution_profile(
        &mut self,
        admin_token: &str,
        profile: crate::world_limits::WorldExecutionProfile,
    ) -> Result<serde_json::Value> {
        self.send_json(&serde_json::json!({
            "type": "set_execution_profile",
            "admin_token": admin_token,
            "profile": profile,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "execution_profile" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the execution profile change")
            );
        }
        Ok(reply)
    }

    /// Read the world's requested execution profile.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if neither a valid admin token nor a valid
    /// guest capability is supplied.
    pub async fn execution_profile(
        &mut self,
        admin_token: Option<&str>,
        capability: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.send_json(&serde_json::json!({
            "type": "execution_profile_info",
            "admin_token": admin_token,
            "capability": capability,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "execution_profile" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the execution profile query")
            );
        }
        Ok(reply)
    }

    /// Let other unisolated worlds write into this world, with the admin token.
    /// Returns the world's policy after the change.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or the change is refused.
    pub async fn set_cross_world_write(
        &mut self,
        admin_token: &str,
        enabled: bool,
    ) -> Result<serde_json::Value> {
        self.policy_request(serde_json::json!({
            "type": "set_cross_world_write",
            "admin_token": admin_token,
            "enabled": enabled,
        }))
        .await
    }

    /// Force cross-world writes on for an unisolated world, with the admin token.
    /// Returns the world's policy after the change.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or the change is refused.
    pub async fn force_cross_world_write(
        &mut self,
        admin_token: &str,
    ) -> Result<serde_json::Value> {
        self.policy_request(serde_json::json!({
            "type": "force_cross_world_write",
            "admin_token": admin_token,
        }))
        .await
    }

    /// Set the file scope (`confined` or `unrestricted`) with the admin token,
    /// and with `write_override` the admin's file write override.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or the change is refused.
    pub async fn set_file_scope(
        &mut self,
        admin_token: &str,
        scope: &str,
        write_override: Option<bool>,
    ) -> Result<serde_json::Value> {
        self.policy_request(serde_json::json!({
            "type": "set_file_scope",
            "admin_token": admin_token,
            "scope": scope,
            "write_override": write_override,
        }))
        .await
    }

    async fn policy_request(&mut self, request: serde_json::Value) -> Result<serde_json::Value> {
        self.send_json(&request).await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "policy" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the policy change")
            );
        }
        Ok(reply["policy"].clone())
    }

    /// Set the world's isolation with the admin token and return the new
    /// setting (`isolated` or `unisolated`).
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or the change is refused.
    pub async fn set_isolation(&mut self, admin_token: &str, isolation: &str) -> Result<String> {
        self.send_json(&serde_json::json!({
            "type": "set_isolation",
            "admin_token": admin_token,
            "isolation": isolation,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "isolation" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused the isolation change")
            );
        }
        reply["isolation"]
            .as_str()
            .map(str::to_owned)
            .context("host sent no isolation")
    }

    /// List the host's worlds as `(default, [(name, open)])`.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token is refused.
    pub async fn list_worlds(
        &mut self,
        admin_token: &str,
    ) -> Result<(String, Vec<(String, bool)>)> {
        self.send_json(&serde_json::json!({
            "type": "list_worlds",
            "admin_token": admin_token,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "worlds" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to list worlds")
            );
        }
        let default = reply["default"].as_str().unwrap_or_default().to_owned();
        let worlds = reply["worlds"]
            .as_array()
            .context("worlds reply had no list")?
            .iter()
            .filter_map(|w| Some((w["name"].as_str()?.to_owned(), w["open"].as_bool()?)))
            .collect();
        Ok((default, worlds))
    }

    /// Create the world `name`; `true` if it did not exist before.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token or name is refused.
    pub async fn create_world(&mut self, admin_token: &str, name: &str) -> Result<bool> {
        self.send_json(&serde_json::json!({
            "type": "create_world",
            "admin_token": admin_token,
            "world": name,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "world_created" {
            bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to create the world")
            );
        }
        Ok(reply["created"].as_bool().unwrap_or(false))
    }

    /// Receive one JSON frame; `None` when the host ended the session.
    ///
    /// # Errors
    ///
    /// Fails on a malformed frame or stream error.
    pub async fn recv_json(&mut self) -> Result<Option<serde_json::Value>> {
        match read_frame(&mut self.recv).await? {
            Some(body) => Ok(Some(serde_json::from_slice(&body).context("decode frame")?)),
            None => Ok(None),
        }
    }

    /// Ask the host to mint a guest capability and return it. The session ends
    /// afterwards. `None` bounds take the host's defaults.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the invite is refused.
    pub async fn invite(
        &mut self,
        admin_token: &str,
        display_name: &str,
        expires_in_secs: Option<u64>,
        uses: Option<u32>,
    ) -> Result<String> {
        self.invite_with_role(admin_token, display_name, expires_in_secs, uses, None)
            .await
    }

    /// Mint a guest capability with an explicit role preset (`spectator`,
    /// `player`, `guest`, `delegator`; `None` means `guest`).
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the invite or the role is refused.
    pub async fn invite_with_role(
        &mut self,
        admin_token: &str,
        display_name: &str,
        expires_in_secs: Option<u64>,
        uses: Option<u32>,
        role: Option<&str>,
    ) -> Result<String> {
        self.send_json(&serde_json::json!({
            "type": "invite",
            "admin_token": admin_token,
            "display_name": display_name,
            "expires_in_secs": expires_in_secs,
            "uses": uses,
            "role": role,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("invite") => reply["capability"]
                .as_str()
                .map(str::to_owned)
                .context("invite reply had no capability"),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("unexpected reply from host")
            ),
        }
    }

    /// Upload and activate a compiled Wasm world module over this Iroh
    /// session. The host pins the bytes in its blob store and switches the
    /// authoritative guest only after compilation and initialization succeed.
    ///
    /// # Errors
    ///
    /// Fails on host refusal, size/hash mismatch, module validation failure,
    /// or stream loss.
    pub async fn install_wasm(
        &mut self,
        admin_token: &str,
        wasm: &[u8],
        seed: u64,
    ) -> Result<String> {
        anyhow::ensure!(
            !wasm.is_empty() && wasm.len() <= crate::world_session::MAX_WORLD_MODULE_BYTES,
            "module size is outside the supported range"
        );
        let hash = module_hash(wasm);
        self.send_json(&serde_json::json!({
            "type": "install_begin",
            "admin_token": admin_token,
            "total_bytes": wasm.len(),
            "module_hash": hash,
            "seed": seed,
        }))
        .await?;
        let ready = self
            .recv_json()
            .await?
            .context("host closed before module upload was accepted")?;
        if ready["type"] != "upload_ready" {
            bail!(
                "{}",
                ready["message"]
                    .as_str()
                    .unwrap_or("host refused module upload")
            );
        }
        let chunk_bytes = ready["chunk_bytes"]
            .as_u64()
            .and_then(|size| usize::try_from(size).ok())
            .filter(|size| (1..=MAX_WORLD_MODULE_CHUNK_BYTES).contains(size))
            .context("host returned an invalid module chunk size")?;
        for (index, chunk) in wasm.chunks(chunk_bytes).enumerate() {
            self.send_json(&serde_json::json!({
                "type": "install_chunk",
                "offset": index * chunk_bytes,
                "data_hex": encode_hex(chunk),
            }))
            .await?;
        }
        self.send_json(&serde_json::json!({"type": "install_end"}))
            .await?;
        let installed = self
            .recv_json()
            .await?
            .context("host closed before module activation response")?;
        if installed["type"] != "module_installed" {
            bail!(
                "{}",
                installed["message"]
                    .as_str()
                    .unwrap_or("host refused module activation")
            );
        }
        let installed_hash = installed["module_hash"]
            .as_str()
            .context("host omitted installed module hash")?;
        anyhow::ensure!(
            installed_hash == hash,
            "host acknowledged a different module hash"
        );
        Ok(hash)
    }

    /// Query which module backs the world's current state. Returns
    /// `(world_id, active_module_hash, current_sequence)`.
    ///
    /// # Errors
    ///
    /// Fails if the capability is refused or the stream breaks.
    pub async fn info(&mut self, capability: &str) -> Result<(String, Option<String>, u64)> {
        let info = self.info_details(capability).await?;
        Ok((
            info.world_id,
            info.active_module_hash,
            info.current_sequence,
        ))
    }

    /// Query the world's host metrics (F11.7): counters and histograms
    /// for commands, journal appends, checkpoints, frame bytes, queue
    /// depth, and live sessions.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the capability is refused, or if
    /// the host is too old to know the `metrics` frame.
    pub async fn metrics(&mut self, capability: &str) -> Result<crate::world::MetricsSnapshot> {
        self.send_json(&serde_json::json!({
            "type": "metrics",
            "capability": capability,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("metrics") => serde_json::from_value(reply["metrics"].clone())
                .context("host sent invalid metrics"),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused to report metrics")
            ),
        }
    }

    /// Query module state, including observed build provenance where available.
    /// Older hosts that do not send provenance continue to decode as `None`.
    pub async fn info_details(&mut self, capability: &str) -> Result<WorldModuleInfo> {
        self.send_json(&serde_json::json!({
            "type": "info",
            "capability": capability,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        match reply["type"].as_str() {
            Some("module_info") => {
                let world_id = reply["world_id"]
                    .as_str()
                    .context("host omitted world id")?
                    .to_owned();
                let active_module_hash = reply["active_module_hash"].as_str().map(str::to_owned);
                let current_sequence = reply["current_sequence"]
                    .as_u64()
                    .context("host omitted sequence")?;
                let active_module_provenance = reply
                    .get("active_module_provenance")
                    .filter(|value| !value.is_null())
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()
                    .context("host sent invalid module provenance")?;
                Ok(WorldModuleInfo {
                    world_id,
                    active_module_hash,
                    active_module_provenance,
                    current_sequence,
                })
            }
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("unexpected reply from host")
            ),
        }
    }

    /// Page through the world's structured records until complete.
    ///
    /// # Errors
    ///
    /// Host refusal, or a malformed page.
    pub async fn records(
        &mut self,
        capability: &str,
        prefix: Option<&str>,
    ) -> Result<crate::world::Records> {
        let mut all = crate::world::Records::new();
        for (_, page) in self.record_pages(capability, prefix).await? {
            all.extend(page);
        }
        Ok(all)
    }

    /// As [`Self::records`], also returning the sequence the records were
    /// read at. Every page must report that sequence: a world that moved
    /// mid-read would otherwise answer with a mix of two states.
    ///
    /// # Errors
    ///
    /// Host refusal, a malformed page, or a world that changed while paging.
    pub async fn records_sequenced(
        &mut self,
        capability: &str,
        prefix: Option<&str>,
    ) -> Result<(u64, crate::world::Records)> {
        let mut pages = self.record_pages(capability, prefix).await?.into_iter();
        let (sequence, mut all) = pages.next().context("host sent no records page")?;
        for (page_sequence, page) in pages {
            anyhow::ensure!(
                page_sequence == sequence,
                "the world changed while its records were read"
            );
            all.extend(page);
        }
        Ok((sequence, all))
    }

    /// Every page of the records query, each tagged with its sequence.
    async fn record_pages(
        &mut self,
        capability: &str,
        prefix: Option<&str>,
    ) -> Result<Vec<(u64, crate::world::Records)>> {
        let mut pages = Vec::new();
        let mut after: Option<String> = None;
        loop {
            self.send_json(&serde_json::json!({
                "type": "records",
                "capability": capability,
                "prefix": prefix,
                "after": after,
            }))
            .await?;
            let reply = self
                .recv_json()
                .await?
                .context("host closed before records reply")?;
            if reply["type"] != "records" {
                bail!(
                    "{}",
                    reply["message"]
                        .as_str()
                        .unwrap_or("host refused the records query")
                );
            }
            let sequence = reply["sequence"]
                .as_u64()
                .context("records reply has no sequence")?;
            let page: crate::world::Records =
                serde_json::from_value(reply["records"].clone()).context("decode records page")?;
            pages.push((sequence, page));
            match reply["next_after"].as_str() {
                Some(next) => after = Some(next.to_owned()),
                None => return Ok(pages),
            }
        }
    }

    /// Ask for a read-only ticket for the world's records document.
    ///
    /// # Errors
    ///
    /// Host refusal or a world without structured records.
    pub async fn records_ticket(&mut self, capability: &str) -> Result<String> {
        self.send_json(&serde_json::json!({
            "type": "records_ticket",
            "capability": capability,
        }))
        .await?;
        let reply = self
            .recv_json()
            .await?
            .context("host closed before ticket reply")?;
        match reply["type"].as_str() {
            Some("records_ticket") => reply["ticket"]
                .as_str()
                .map(str::to_owned)
                .context("ticket reply had no ticket"),
            _ => bail!(
                "{}",
                reply["message"]
                    .as_str()
                    .unwrap_or("host refused a records ticket")
            ),
        }
    }

    /// Download the active module bytes after verifying the host's hash.
    ///
    /// # Errors
    ///
    /// Fails on host refusal, hash mismatch, oversize or truncated transfer.
    pub async fn download_module(
        &mut self,
        capability: &str,
        expected_hash: &str,
    ) -> Result<Vec<u8>> {
        self.download_module_with(
            serde_json::json!({
                "type": "download_module",
                "capability": capability,
                "module_hash": expected_hash,
            }),
            expected_hash,
        )
        .await
    }

    /// Download a stored module by hash as an admin, whether or not it is
    /// the active module (#807). The host serves only modules it holds in
    /// its blob store; a hash it never installed is refused.
    ///
    /// # Errors
    ///
    /// Fails on a bad admin token, a hash the host does not hold, a hash
    /// mismatch, oversize or a truncated transfer.
    pub async fn download_module_admin(
        &mut self,
        admin_token: &str,
        expected_hash: &str,
    ) -> Result<Vec<u8>> {
        self.download_module_with(
            serde_json::json!({
                "type": "download_module_admin",
                "admin_token": admin_token,
                "module_hash": expected_hash,
            }),
            expected_hash,
        )
        .await
    }

    /// Send one module-download frame and read the transfer it starts.
    async fn download_module_with(
        &mut self,
        frame: serde_json::Value,
        expected_hash: &str,
    ) -> Result<Vec<u8>> {
        self.send_json(&frame).await?;
        let begin = self
            .recv_json()
            .await?
            .context("host closed before module transfer")?;
        if begin["type"] != "module_begin" {
            bail!(
                "{}",
                begin["message"]
                    .as_str()
                    .unwrap_or("host refused module download")
            );
        }
        if begin["module_hash"] != expected_hash {
            bail!("host offered a different module than requested");
        }
        let total_bytes = begin["total_bytes"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n > 0 && *n <= crate::world_session::MAX_WORLD_MODULE_BYTES)
            .context("host declared an invalid module size")?;
        let mut bytes = Vec::with_capacity(total_bytes);
        while bytes.len() < total_bytes {
            let chunk = self
                .recv_json()
                .await?
                .context("host closed mid-transfer")?;
            if chunk["type"] != "module_chunk" {
                bail!(
                    "{}",
                    chunk["message"]
                        .as_str()
                        .unwrap_or("unexpected frame during module download")
                );
            }
            let offset = chunk["offset"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .context("host omitted chunk offset")?;
            anyhow::ensure!(offset == bytes.len(), "module chunks arrived out of order");
            let data_hex = chunk["data_hex"]
                .as_str()
                .context("host omitted chunk bytes")?;
            let Some(data) = crate::world_session::decode_hex(data_hex) else {
                bail!("host sent non-hexadecimal module bytes");
            };
            anyhow::ensure!(
                !data.is_empty() && data.len() <= total_bytes - bytes.len(),
                "host sent an invalid module chunk"
            );
            bytes.extend_from_slice(&data);
        }
        let end = self
            .recv_json()
            .await?
            .context("host closed before module transfer completed")?;
        if end["type"] != "module_end" || end["module_hash"] != expected_hash {
            bail!("module transfer did not complete cleanly");
        }
        anyhow::ensure!(
            module_hash(&bytes).as_str() == expected_hash,
            "downloaded module does not match its hash"
        );
        Ok(bytes)
    }

    /// Split into the connection and both stream halves, e.g. to read frames
    /// in a task while writing from another.
    #[must_use]
    pub fn into_parts(self) -> (Connection, SendStream, RecvStream) {
        (self.conn, self.send, self.recv)
    }

    /// Close the connection.
    pub fn close(self) {
        self.conn.close(0_u32.into(), b"done");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use iroh::{
        endpoint::{RelayMode, presets},
        protocol::Router,
    };

    use super::*;
    use crate::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
    use crate::world_session::{INVITE_MAX_SECS, WorldService};

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    struct Host {
        router: Router,
        addr: EndpointAddr,
        world: WorldHandle,
    }

    async fn host(admin: Option<&str>) -> Host {
        let ep = endpoint().await;
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world.clone(), admin.map(str::to_owned));
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(service))
            .spawn();
        // Loopback sockets only: the test endpoints use no relay or discovery.
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );
        Host {
            router,
            addr,
            world,
        }
    }

    async fn next(client: &mut WorldClient) -> Option<serde_json::Value> {
        tokio::time::timeout(std::time::Duration::from_secs(10), client.recv_json())
            .await
            .expect("timed out waiting for a world frame")
            .unwrap()
    }

    #[tokio::test]
    async fn outbound_throttling_waits_for_refill_instead_of_reporting_closed() {
        use crate::abuse::{AbusePolicy, BucketLimits};

        let limits = BucketLimits {
            frames_per_sec: 1_000,
            frame_burst: 1,
            bytes_per_sec: 1_000,
            byte_burst: 1,
        };
        let evaluator = Arc::new(Mutex::new(AbuseEvaluator::new(
            AbusePolicy {
                authorized: limits,
                pre_authorization: limits,
                ..AbusePolicy::default()
            },
            [1; 32],
        )));
        let remote = [2; 32];
        assert!(
            charge_frame_with_backpressure(&evaluator, remote, true, 1).await,
            "first frame fits the burst"
        );
        assert!(
            tokio::time::timeout(
                Duration::from_secs(3),
                charge_frame_with_backpressure(&evaluator, remote, true, 1),
            )
            .await
            .expect("the retry should complete after the token refills"),
            "a rate refusal is backpressure, not a closed stream"
        );
        assert_eq!(
            evaluator
                .lock()
                .unwrap()
                .counters()
                .get(&[1; 32], &remote)
                .unwrap()
                .frames,
            2
        );
    }

    fn join(capability: &str, after: Option<u64>) -> serde_json::Value {
        serde_json::json!({"type": "join", "capability": capability, "after": after})
    }

    #[tokio::test]
    async fn peers_invite_join_chat_and_catch_up_over_iroh() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;

        // Mint two capabilities through the protocol itself.
        let mut capabilities = Vec::new();
        for name in ["ann", "bob"] {
            let mut c = WorldClient::connect(&client_ep, host.addr.clone())
                .await
                .unwrap();
            c.send_json(&serde_json::json!(
                {"type": "invite", "admin_token": "admin", "display_name": name}
            ))
            .await
            .unwrap();
            let invite = next(&mut c).await.unwrap();
            assert_eq!(invite["type"], "invite");
            capabilities.push(invite["capability"].as_str().unwrap().to_owned());
            assert!(next(&mut c).await.is_none(), "invite ends the session");
            c.close();
        }

        let mut ann = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        ann.send_json(&join(&capabilities[0], None)).await.unwrap();
        assert_eq!(next(&mut ann).await.unwrap()["type"], "snapshot");
        let mut bob = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        bob.send_json(&join(&capabilities[1], None)).await.unwrap();
        assert_eq!(next(&mut bob).await.unwrap()["type"], "snapshot");

        ann.send_json(&serde_json::json!({"type": "chat", "text": "hello over iroh"}))
            .await
            .unwrap();
        for c in [&mut ann, &mut bob] {
            let frame = next(c).await.unwrap();
            assert_eq!(frame["type"], "event");
            assert!(frame.to_string().contains("hello over iroh"));
        }

        // A late joiner replays from a cursor without a snapshot.
        let mut late = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        late.send_json(&join(&capabilities[1], Some(0)))
            .await
            .unwrap();
        let replay = next(&mut late).await.unwrap();
        assert_eq!(replay["type"], "event");
        assert!(replay.to_string().contains("hello over iroh"));

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn caps_ceiling_is_admin_only_over_iroh_and_applies_at_once() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;

        let mut outsider = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        assert!(
            outsider
                .set_caps_ceiling("wrong", &["time.now".to_owned()])
                .await
                .is_err()
        );

        let mut admin = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let reply = admin
            .set_caps_ceiling("admin", &["time.now".to_owned()])
            .await
            .unwrap();
        assert_eq!(reply["type"], "caps_ceiling");
        assert_eq!(reply["report"]["ceiling"], serde_json::json!(["time.now"]));
        assert!(
            reply["message"]
                .as_str()
                .unwrap()
                .contains("applies immediately")
        );
        assert_eq!(
            host.world.caps_report().await.unwrap().ceiling,
            vec!["time.now".to_owned()]
        );

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn execution_profile_can_be_queried_and_updated_over_iroh() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;

        let mut invite_client = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let capability = invite_client
            .invite("admin", "ann", None, None)
            .await
            .unwrap();
        invite_client.close();

        // 1. Unauthenticated query is denied
        let mut outsider = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        assert!(outsider.execution_profile(None, None).await.is_err());
        outsider.close();

        // 2. Guest with capability can query profile
        let mut guest = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let res = guest
            .execution_profile(None, Some(&capability))
            .await
            .unwrap();
        assert_eq!(res["type"], "execution_profile");
        guest.close();

        // 3. Admin can update profile
        let mut admin = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let custom_profile = crate::world_limits::WorldExecutionProfile {
            wasm_fuel: Some(750_000),
            native_cpu_seconds: Some(15),
            ..Default::default()
        };
        let set_res = admin
            .set_execution_profile("admin", custom_profile)
            .await
            .unwrap();
        assert_eq!(set_res["type"], "execution_profile");
        assert_eq!(set_res["update"]["requested"]["wasm_fuel"], 750_000);
        admin.close();

        // 4. Admin query sees the updated requested profile
        let mut admin_query = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let query_res = admin_query
            .execution_profile(Some("admin"), None)
            .await
            .unwrap();
        assert_eq!(query_res["update"]["requested"]["wasm_fuel"], 750_000);
        admin_query.close();

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[test]
    fn a_caps_report_from_before_ceilings_still_deserializes() {
        let old = serde_json::json!({
            "granted": ["time.now"],
            "requested": ["time.now"],
            "missing": [],
            "usage": {},
            "policy": "deny",
            "catalog": ["time.now"],
        });
        let report: crate::world_caps::CapsReport = serde_json::from_value(old).unwrap();
        assert_eq!(report.granted, vec!["time.now".to_owned()]);
        assert!(report.ceiling.is_empty() && report.effective.is_empty());
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn roc_guest_updates_authoritative_state_over_iroh() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let sandbox =
            crate::sandbox::Sandbox::compile(wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let program = sandbox.instantiate(7).unwrap();
        let ep = endpoint().await;
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("counter", WorldLimits::default()).unwrap(),
            Box::new(program),
        );
        let service = WorldService::new(world, Some("admin".to_owned()));
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(service.clone()))
            .spawn();
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );
        let client_ep = endpoint().await;

        let mut invite_client = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let capability = invite_client
            .invite("admin", "ann", None, None)
            .await
            .unwrap();
        invite_client.close();

        let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
        client.send_json(&join(&capability, None)).await.unwrap();
        assert_eq!(next(&mut client).await.unwrap()["type"], "snapshot");
        let initial = next(&mut client).await.unwrap();
        assert_eq!(initial["type"], "view");
        assert!(
            initial["data"]
                .as_str()
                .is_some_and(|data| data.contains("count=0"))
        );

        client
            .send_json(&serde_json::json!({ "type": "input", "data_hex": "696e63" }))
            .await
            .unwrap();
        let event = next(&mut client).await.unwrap();
        assert_eq!(event["type"], "event");
        let view = next(&mut client).await.unwrap();
        assert!(
            view["data"]
                .as_str()
                .is_some_and(|data| data.contains("count=1"))
        );

        client_ep.close().await;
        router.shutdown().await.unwrap();
        ep.close().await;
    }

    /// #807: `id world apply`'s hash path - download a stored module as
    /// admin, then install the bytes - sequenced on ONE connection. Admin
    /// frames are served one operation per stream, so each operation runs
    /// on a fresh session; on a reused stream the install would write to a
    /// session the host already finished.
    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn apply_hash_path_sequences_fresh_sessions_on_one_connection() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let wasm2 = include_bytes!("../examples/roc-chat/roc-chat.wasm");
        let ep = endpoint().await;
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let service =
            WorldService::new(world, Some("admin".to_owned())).with_blob_store(blobs.clone());
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(service))
            .spawn();
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );
        let client_ep = endpoint().await;
        let client = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();

        let hash = client
            .fresh_session()
            .await
            .unwrap()
            .install_wasm("admin", wasm, 7)
            .await
            .unwrap();
        let hash2 = client
            .fresh_session()
            .await
            .unwrap()
            .install_wasm("admin", wasm2, 7)
            .await
            .unwrap();
        assert_ne!(hash, hash2);

        let stored = client
            .fresh_session()
            .await
            .unwrap()
            .download_module_admin("admin", &hash)
            .await
            .unwrap();
        assert_eq!(stored, wasm.as_slice());
        let reinstalled = client
            .fresh_session()
            .await
            .unwrap()
            .install_wasm("admin", &stored, 7)
            .await
            .unwrap();
        assert_eq!(reinstalled, hash);

        client.close();
        client_ep.close().await;
        ep.close().await;
        router.shutdown().await.unwrap();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn peers_install_then_download_the_active_module_by_hash() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let ep = endpoint().await;
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let blobs: iroh_blobs::api::Store = iroh_blobs::store::mem::MemStore::new().into();
        let service =
            WorldService::new(world, Some("admin".to_owned())).with_blob_store(blobs.clone());
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(service))
            .spawn();
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );
        let client_ep = endpoint().await;

        let mut admin = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let hash = admin.install_wasm("admin", wasm, 7).await.unwrap();
        admin.close();
        assert_eq!(hash, module_hash(wasm));
        assert!(
            blobs
                .blobs()
                .has(hash.parse::<iroh_blobs::Hash>().unwrap())
                .await
                .unwrap(),
            "the host pins installed modules in its blob store"
        );

        let mut inviter = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let capability = inviter.invite("admin", "ann", None, None).await.unwrap();
        inviter.close();

        let mut query = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let info = query.info_details(&capability).await.unwrap();
        query.close();
        assert_eq!(info.active_module_hash.as_deref(), Some(hash.as_str()));
        assert_eq!(
            info.active_module_provenance,
            Some(crate::world::ModuleProvenance::Artifact)
        );

        let mut fetch = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let bytes = fetch.download_module(&capability, &hash).await.unwrap();
        fetch.close();
        assert_eq!(bytes, wasm.as_slice());

        // A different hash is refused rather than served.
        let mut wrong = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        assert!(
            wrong
                .download_module(&capability, &"0".repeat(64))
                .await
                .is_err()
        );
        wrong.close();

        // #807: a second install makes the first module non-active. The
        // participant download then refuses it, but an admin can still fetch
        // the stored bytes by hash - the path `id world apply` uses to
        // install the module a manifest names.
        let wasm2 = include_bytes!("../examples/roc-chat/roc-chat.wasm");
        let mut admin2 = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let hash2 = admin2.install_wasm("admin", wasm2, 7).await.unwrap();
        admin2.close();
        assert_ne!(hash2, hash);

        let mut fetch2 = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        assert!(
            fetch2.download_module(&capability, &hash).await.is_err(),
            "the first module is no longer the active one"
        );
        fetch2.close();

        let mut admin_fetch = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let stored = admin_fetch
            .download_module_admin("admin", &hash)
            .await
            .unwrap();
        admin_fetch.close();
        assert_eq!(stored, wasm.as_slice());

        let mut bad_admin = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        assert!(
            bad_admin
                .download_module_admin("wrong", &hash)
                .await
                .is_err()
        );
        bad_admin.close();

        let mut missing = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        assert!(
            missing
                .download_module_admin("admin", &"0".repeat(64))
                .await
                .is_err()
        );
        missing.close();

        // Without a capability, nothing is disclosed.
        let mut anon = WorldClient::connect(&client_ep, addr).await.unwrap();
        assert!(anon.download_module(&"0".repeat(64), &hash).await.is_err());
        anon.close();

        client_ep.close().await;
        router.shutdown().await.unwrap();
        ep.close().await;
    }

    /// Publishes 96 ~8 KiB records, so a full read spans several pages that
    /// must fit the transport frame limit.
    struct Bulky;

    impl crate::world::WorldProgram for Bulky {
        fn records(&mut self) -> Result<Option<String>> {
            let payload = "y".repeat(8 * 1024 - 16);
            let records: crate::world::Records = (0..96)
                .map(|i| {
                    (
                        format!("key-{i:03}"),
                        serde_json::Value::from(payload.clone()),
                    )
                })
                .collect();
            Ok(Some(serde_json::to_string(&records)?))
        }
    }

    #[tokio::test]
    async fn paged_records_survive_the_iroh_frame_limit() {
        let ep = endpoint().await;
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("lobby", WorldLimits::default()).unwrap(),
            Box::new(Bulky),
        );
        let service = WorldService::new(world.clone(), Some("admin".to_owned()));
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(service))
            .spawn();
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );
        let client_ep = endpoint().await;
        let mut inviter = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let capability = inviter.invite("admin", "ann", None, None).await.unwrap();
        inviter.close();

        let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
        let records = client.records(&capability, None).await.unwrap();
        assert_eq!(records.len(), 96);
        assert!(records.contains_key("key-000") && records.contains_key("key-095"));
        client.close();
        client_ep.close().await;
        router.shutdown().await.unwrap();
        ep.close().await;
    }

    #[tokio::test]
    async fn one_connection_reaches_many_worlds_and_many_clients_share_a_host() {
        let ep = endpoint().await;
        let opener = crate::world_hub::testing::MemoryOpener::new(false);
        let dyn_opener: Arc<dyn crate::world_hub::WorldOpener> = opener.clone();
        let hub = WorldHub::new(
            dyn_opener,
            Some("admin".to_owned()),
            "lobby",
            crate::world_hub::HubLimits::default(),
        );
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(hub.clone()))
            .spawn();
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );

        // Several independent clients, each hopping across worlds on one
        // connection, all at once.
        let mut tasks = JoinSet::new();
        for client_no in 0..6 {
            let addr = addr.clone();
            tasks.spawn(async move {
                let client_ep = endpoint().await;
                let root = WorldClient::connect(&client_ep, addr).await.unwrap();
                for world in ["alpha", "beta", "gamma"] {
                    let mut inviter = root.open_session(Some(world)).await.unwrap();
                    let capability = inviter
                        .invite("admin", &format!("player{client_no}"), None, None)
                        .await
                        .unwrap();
                    let mut session = root.open_session(Some(world)).await.unwrap();
                    let (world_id, _, _) = session.info(&capability).await.unwrap();
                    assert_eq!(world_id, world);
                }
                client_ep.close().await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        assert_eq!(hub.open_count(), 3);
        router.shutdown().await.unwrap();
        ep.close().await;
    }

    #[tokio::test]
    async fn invite_helper_returns_the_capability_or_the_hosts_reason() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;

        let mut ok = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let capability = ok.invite("admin", "ann", None, None).await.unwrap();
        assert!(!capability.is_empty());

        let mut bad = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let err = bad.invite("wrong", "eve", None, None).await.unwrap_err();
        assert!(err.to_string().contains("invite denied"), "{err:#}");

        let mut too_long = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let err = too_long
            .invite("admin", "fay", Some(INVITE_MAX_SECS + 1), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("30 days"), "{err:#}");

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn node_identity_alone_grants_nothing() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;

        let mut forged = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        forged
            .send_json(&join(&"0".repeat(64), None))
            .await
            .unwrap();
        assert_eq!(next(&mut forged).await.unwrap()["message"], "join denied");
        assert!(next(&mut forged).await.is_none());

        let mut wrong_admin = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        wrong_admin
            .send_json(&serde_json::json!(
                {"type": "invite", "admin_token": "guess", "display_name": "eve"}
            ))
            .await
            .unwrap();
        assert_eq!(
            next(&mut wrong_admin).await.unwrap()["message"],
            "invite denied"
        );

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn oversized_and_invalid_frames_are_rejected_without_a_panic() {
        let host = host(None).await;
        let client_ep = endpoint().await;

        // A declared length over the limit ends the session without the
        // server ever allocating or reading the body.
        let conn = client_ep
            .connect(host.addr.clone(), WORLD_ALPN)
            .await
            .unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        let outcome =
            tokio::time::timeout(std::time::Duration::from_secs(10), read_frame(&mut recv))
                .await
                .expect("server should close the stream");
        assert!(matches!(outcome, Ok(None) | Err(_)));
        conn.close(0_u32.into(), b"done");

        // Non-UTF-8 payload gets a protocol error, not a crash.
        let mut c = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        write_frame(&mut c.send, &[0xff, 0xfe, 0xfd]).await.unwrap();
        assert_eq!(next(&mut c).await.unwrap()["type"], "error");

        // The host is still serving afterwards.
        let (_, cap) = host
            .world
            .issue("ok".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let mut ok = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        ok.send_json(&join(cap.expose(), None)).await.unwrap();
        assert_eq!(next(&mut ok).await.unwrap()["type"], "snapshot");

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn sessions_per_connection_are_bounded() {
        let host = host(None).await;
        let client_ep = endpoint().await;
        let conn = client_ep
            .connect(host.addr.clone(), WORLD_ALPN)
            .await
            .unwrap();
        // Join on more streams than allowed. Accepted sessions stay open, so
        // the extras hit the bound and are reset; streams are accepted in
        // order, so the first MAX_SESSIONS_PER_CONNECTION win.
        let total = MAX_SESSIONS_PER_CONNECTION + 3;
        let mut streams = Vec::new();
        for i in 0..total {
            let (_, cap) = host
                .world
                .issue(format!("p{i}"), WorldScopes::GUEST)
                .await
                .unwrap();
            let (mut send, recv) = conn.open_bi().await.unwrap();
            write_frame(&mut send, join(cap.expose(), None).to_string().as_bytes())
                .await
                .unwrap();
            streams.push((send, recv));
        }
        let (mut accepted, mut reset) = (0, 0);
        for (_, recv) in &mut streams {
            match tokio::time::timeout(std::time::Duration::from_secs(10), read_frame(recv)).await {
                Ok(Ok(Some(_))) => accepted += 1,
                Ok(Err(_)) => reset += 1,
                other => panic!("unexpected: {other:?}"),
            }
        }
        assert_eq!(accepted, MAX_SESSIONS_PER_CONNECTION);
        assert_eq!(reset, total - MAX_SESSIONS_PER_CONNECTION);

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }
}
