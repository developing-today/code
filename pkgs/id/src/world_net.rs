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

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
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

use crate::abuse::{
    AbuseEvaluator, AbusePolicy, BudgetPolicy, BudgetUsage, ConnectionId, Counters, Direction,
    EndpointId as AbuseEndpointId, NodeId, RefusalReason,
};
use crate::event_store::BlockList;
use crate::frames::{read_frame_body, read_frame_len};
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
    /// Lifetime count of peers refused by the per-peer connection cap.
    connection_cap_refusals: Arc<AtomicU64>,
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
        Self::with_budgets(hub, local_node, BudgetPolicy::default())
    }

    /// Like [`WorldProtocol::with_local_node`], with the #2247
    /// rolling-window budget policy the shared evaluator enforces
    /// at the frame seam: app-wide inbound/outbound, per-endpoint,
    /// and per-counterparty override scopes (the per-IP scope is
    /// not charged here — Iroh peers are relay-capable and no IP is
    /// directly observed). [`WorldProtocol::new`] and
    /// [`WorldProtocol::with_local_node`] use
    /// [`BudgetPolicy::default`]: the #2247 defaults in code.
    #[must_use]
    pub fn with_budgets(
        hub: impl Into<WorldHub>,
        local_node: NodeId,
        budgets: BudgetPolicy,
    ) -> Self {
        Self::build(hub, local_node, budgets, None)
    }

    /// Like [`WorldProtocol::with_local_node`], with the operator block
    /// list (#2245) attached to the shared evaluator: a connection from
    /// an endpoint blocked by key is refused at admission, before any
    /// session starts, with the evaluator's
    /// [`RefusalReason::BlockedByOperator`] refusal. The list is
    /// consulted live, so a block lands on the next connection.
    #[must_use]
    pub fn with_blocks(
        hub: impl Into<WorldHub>,
        local_node: NodeId,
        blocks: Arc<dyn BlockList>,
    ) -> Self {
        Self::build(hub, local_node, BudgetPolicy::default(), Some(blocks))
    }

    fn build(
        hub: impl Into<WorldHub>,
        local_node: NodeId,
        budgets: BudgetPolicy,
        blocks: Option<Arc<dyn BlockList>>,
    ) -> Self {
        let mut evaluator =
            AbuseEvaluator::with_budgets(AbusePolicy::default(), local_node, budgets);
        if let Some(blocks) = blocks {
            evaluator = evaluator.with_block_list(blocks);
        }
        Self {
            hub: hub.into(),
            abuse: Arc::new(Mutex::new(evaluator)),
            connection_cap_refusals: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of connections refused by the per-peer cap since this
    /// protocol instance was created. Refusal details, including the
    /// peer identity, are also emitted at `WARN` level.
    #[must_use]
    pub fn connection_cap_refusals(&self) -> u64 {
        self.connection_cap_refusals.load(Ordering::Relaxed)
    }

    /// Snapshot of the per-(local node, remote endpoint) frame/byte
    /// counters recorded by the shared evaluator — the operator
    /// visibility surface for #2244 P0-A charging.
    #[must_use]
    /// Estimated #2247 budget usage per scope and window — the
    /// operator-visibility surface for budgets, alongside
    /// [`WorldProtocol::abuse_counters`].
    #[must_use]
    pub fn budget_usage(
        &self,
    ) -> Vec<(
        crate::abuse::BudgetScope,
        crate::abuse::BudgetLimit,
        BudgetUsage,
    )> {
        self.abuse
            .lock()
            .map(|abuse| abuse.budget_usage(abuse_now_ms()))
            .unwrap_or_default()
    }

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
            abuse.prune_budget_state(now_ms);
        }
    }
}

/// Owns one evaluator registration for a live QUIC connection. Keeping
/// this guard in `accept` ties the per-peer slot to the transport's full
/// lifetime, including every early return and unwinding path.
struct PeerConnectionRegistration {
    abuse: Arc<Mutex<AbuseEvaluator>>,
    endpoint: AbuseEndpointId,
    connection: ConnectionId,
}

impl Drop for PeerConnectionRegistration {
    fn drop(&mut self) {
        let mut abuse = self
            .abuse
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        abuse.close_connection(&self.endpoint, self.connection);
    }
}

/// The remote IP of the connection's selected path, when that path
/// is a direct IP path (#2245). A connection whose selected path is
/// a relay or a custom transport has no peer IP an operator block
/// could name: it yields `None`, and IP blocks never apply to it.
/// Best-effort by construction — a direct path that opens after
/// admission is not re-judged, exactly as the issue scopes IP
/// enforcement to the listener.
fn direct_ip_of(conn: &Connection) -> Option<std::net::IpAddr> {
    let paths = conn.paths();
    let selected = paths.iter().find(|path| path.is_selected())?;
    match selected.remote_addr() {
        iroh::TransportAddr::Ip(addr) => Some(addr.ip()),
        _ => None,
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
        let remote_endpoint: AbuseEndpointId = *remote.as_bytes();
        let direct_ip = direct_ip_of(&conn);
        let connection_id = match self.abuse.lock() {
            Ok(mut abuse) => match abuse.open_connection_with_direct_ip(
                remote_endpoint,
                direct_ip,
                abuse_now_ms(),
            ) {
                Ok(connection_id) => connection_id,
                Err(refusal) if refusal.reason == RefusalReason::BlockedByOperator => {
                    // #2245: an operator block is not a cap refusal — it
                    // is not counted as one, and it offers no retry.
                    tracing::warn!(
                        remote = %remote,
                        "world: operator block; refusing connection"
                    );
                    conn.close(0_u32.into(), b"blocked by operator");
                    return Ok(());
                }
                Err(refusal) => {
                    let refusal_count = self
                        .connection_cap_refusals
                        .fetch_add(1, Ordering::Relaxed)
                        .saturating_add(1);
                    tracing::warn!(
                        remote = %remote,
                        refusal_count,
                        retry_after_ms = ?refusal.retry_after_ms,
                        "world: per-peer connection cap reached; refusing connection"
                    );
                    conn.close(0_u32.into(), b"per-peer connection cap reached");
                    return Ok(());
                }
            },
            Err(_) => {
                tracing::error!(
                    remote = %remote,
                    "world: abuse evaluator lock poisoned; refusing connection"
                );
                conn.close(0_u32.into(), b"abuse evaluator unavailable");
                return Ok(());
            }
        };
        let _registration = PeerConnectionRegistration {
            abuse: Arc::clone(&self.abuse),
            endpoint: remote_endpoint,
            connection: connection_id,
        };
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
                // hard refusal sheds this stream only; the peer
                // connection and its other sessions stay up.
                let inbound = match read_frame_len(&mut recv).await {
                    Ok(Some(len)) => {
                        // The charge runs before the body is read, so
                        // an authorized stream can wait out a temporary
                        // refusal with the body still unread (read-side
                        // backpressure); the lock is never held across
                        // the wait or the body read.
                        let admitted = charge_inbound_frame(
                            &reader_abuse,
                            remote,
                            &reader_authorized_task,
                            len as usize,
                        )
                        .await;
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
            Ok(mut abuse) => crate::frames::charge_frame_budgeted(
                &mut abuse,
                endpoint,
                authorized,
                payload_len,
                Direction::Outbound,
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

/// Charge one inbound frame for a stream reader.
///
/// An authorized stream that hits a temporary refusal waits out the
/// refusal's retry interval and charges again, with the frame body still
/// unread on the stream: QUIC flow control then paces the sender instead
/// of the refusal closing the stream mid-transfer (a module upload sends
/// many chunks back-to-back and would otherwise die at the burst limit,
/// well below the supported module size). A pre-authorization refusal, a
/// refusal with no retry offered, and a poisoned evaluator still refuse
/// immediately — the caller sheds this stream only. The shared lock is
/// released before every wait.
async fn charge_inbound_frame(
    abuse: &Arc<Mutex<AbuseEvaluator>>,
    endpoint: AbuseEndpointId,
    authorized: &AtomicBool,
    payload_len: usize,
) -> bool {
    loop {
        let is_authorized = authorized.load(Ordering::Acquire);
        let retry_after_ms = match abuse.lock() {
            Ok(mut abuse) => match crate::frames::charge_frame_budgeted(
                &mut abuse,
                endpoint,
                is_authorized,
                payload_len,
                Direction::Inbound,
                abuse_now_ms(),
            ) {
                Ok(()) => return true,
                Err(refusal) => refusal.0.retry_after_ms,
            },
            Err(_) => return false,
        };
        match retry_after_ms {
            Some(ms) if is_authorized => {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            _ => return false,
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

const MAX_ADMIN_HISTORY_RECORD_BYTES: usize = crate::world::MAX_RECORDS
    * (crate::world::MAX_RECORD_KEY_BYTES + crate::world::MAX_RECORD_VALUE_BYTES + 8);
// Bound the entire transfer, not just the decoded `event` and `records`
// values. The allowance covers top-level framing, per-event/per-page JSON
// wrappers, and worst-case quote/backslash escaping in record keys.
const MAX_ADMIN_HISTORY_RESPONSE_BYTES: usize = crate::world::MAX_ADMIN_HISTORY_BYTES
    + MAX_ADMIN_HISTORY_RECORD_BYTES
    + crate::world::MAX_RECORDS * crate::world::MAX_RECORD_KEY_BYTES
    + (crate::world::MAX_ADMIN_HISTORY_EVENTS + crate::world::MAX_RECORDS + 1) * 256;

#[derive(Default)]
struct AdminHistoryBudget {
    event_bytes: usize,
    record_bytes: usize,
    record_count: usize,
    response_bytes: usize,
}

impl AdminHistoryBudget {
    fn add_frame(&mut self, bytes: usize) -> Result<()> {
        self.response_bytes = self
            .response_bytes
            .checked_add(bytes)
            .context("admin history response byte count overflow")?;
        anyhow::ensure!(
            self.response_bytes <= MAX_ADMIN_HISTORY_RESPONSE_BYTES,
            "admin history response exceeds {MAX_ADMIN_HISTORY_RESPONSE_BYTES} bytes"
        );
        Ok(())
    }

    fn add_event(&mut self, bytes: usize) -> Result<()> {
        self.event_bytes = self
            .event_bytes
            .checked_add(bytes)
            .context("admin event byte count overflow")?;
        anyhow::ensure!(
            self.event_bytes <= crate::world::MAX_ADMIN_HISTORY_BYTES,
            "admin history events exceed {} bytes",
            crate::world::MAX_ADMIN_HISTORY_BYTES
        );
        Ok(())
    }

    fn add_record(&mut self, key: &str, value: &serde_json::Value) -> Result<()> {
        anyhow::ensure!(
            !key.is_empty() && key.len() <= crate::world::MAX_RECORD_KEY_BYTES,
            "admin history contains an invalid record key"
        );
        anyhow::ensure!(
            !key.chars().any(char::is_control),
            "admin history contains a control character in a record key"
        );
        let value_bytes = serde_json::to_vec(value)
            .context("measure an admin history record")?
            .len();
        anyhow::ensure!(
            value_bytes <= crate::world::MAX_RECORD_VALUE_BYTES,
            "admin history record value exceeds {} bytes",
            crate::world::MAX_RECORD_VALUE_BYTES
        );
        self.record_count = self
            .record_count
            .checked_add(1)
            .context("admin record count overflow")?;
        anyhow::ensure!(
            self.record_count <= crate::world::MAX_RECORDS,
            "admin history contains more than {} records",
            crate::world::MAX_RECORDS
        );
        let entry_bytes = key
            .len()
            .checked_add(value_bytes)
            .and_then(|bytes| bytes.checked_add(8))
            .context("admin record byte count overflow")?;
        self.record_bytes = self
            .record_bytes
            .checked_add(entry_bytes)
            .context("admin record byte count overflow")?;
        anyhow::ensure!(
            self.record_bytes <= MAX_ADMIN_HISTORY_RECORD_BYTES,
            "admin history records exceed {MAX_ADMIN_HISTORY_RECORD_BYTES} bytes"
        );
        Ok(())
    }
}

/// Budget for validating records returned by a plain records query.
/// Enforces the same caps as `parse_records` and `read_records`.
#[derive(Default)]
struct RecordsBudget {
    record_count: usize,
    last_key: Option<String>,
}

impl RecordsBudget {
    fn validate_record(&mut self, key: &str, value: &serde_json::Value) -> Result<()> {
        // Check key length and non-empty
        anyhow::ensure!(
            !key.is_empty() && key.len() <= crate::world::MAX_RECORD_KEY_BYTES,
            "record key must be 1..={} bytes",
            crate::world::MAX_RECORD_KEY_BYTES
        );
        
        // Check for control characters
        anyhow::ensure!(
            !key.chars().any(char::is_control),
            "record key {key:?} contains a control character"
        );
        
        // Check value size
        let value_bytes = serde_json::to_vec(value)
            .context("measure record value")?
            .len();
        anyhow::ensure!(
            value_bytes <= crate::world::MAX_RECORD_VALUE_BYTES,
            "record {key:?} is {value_bytes} bytes; the limit is {} bytes",
            crate::world::MAX_RECORD_VALUE_BYTES
        );
        
        // Check total count
        self.record_count = self
            .record_count
            .checked_add(1)
            .context("record count overflow")?;
        anyhow::ensure!(
            self.record_count <= crate::world::MAX_RECORDS,
            "records query contains more than {} records; the limit is {}",
            self.record_count,
            crate::world::MAX_RECORDS
        );
        
        // Check ordering: keys must be strictly ascending
        if let Some(last) = &self.last_key {
            anyhow::ensure!(
                key > last.as_str(),
                "records are out of order: key {key:?} <= last key {last:?}"
            );
        }
        
        self.last_key = Some(key.to_owned());
        Ok(())
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

    /// List the host's worlds as `(default, [(name, open, presence)])`,
    /// where `presence` is the count of sessions live on the world.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the admin token is refused.
    pub async fn list_worlds(
        &mut self,
        admin_token: &str,
    ) -> Result<(String, Vec<(String, bool, usize)>)> {
        self.send_json(&serde_json::json!({
            "type": "list_worlds",
            "admin_token": admin_token,
        }))
        .await?;
        self.read_worlds_reply("host refused to list worlds").await
    }

    /// Read the host's public world directory (#812) as
    /// `(default, [(name, open, presence)])`. No admin token: the
    /// directory is public metadata — names, loaded state, and
    /// presence counts, never identities.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the directory is refused.
    pub async fn directory(&mut self) -> Result<(String, Vec<(String, bool, usize)>)> {
        self.send_json(&serde_json::json!({
            "type": "directory",
        }))
        .await?;
        self.read_worlds_reply("host refused the directory").await
    }

    async fn read_worlds_reply(
        &mut self,
        refuse_message: &str,
    ) -> Result<(String, Vec<(String, bool, usize)>)> {
        let reply = self
            .recv_json()
            .await?
            .context("host closed without replying")?;
        if reply["type"] != "worlds" {
            bail!("{}", reply["message"].as_str().unwrap_or(refuse_message));
        }
        let default = reply["default"].as_str().unwrap_or_default().to_owned();
        let worlds = reply["worlds"]
            .as_array()
            .context("worlds reply had no list")?
            .iter()
            .filter_map(|w| {
                Some((
                    w["name"].as_str()?.to_owned(),
                    w["open"].as_bool()?,
                    w["presence"].as_u64().unwrap_or(0) as usize,
                ))
            })
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
        Ok(self
            .recv_json_with_frame_bytes()
            .await?
            .map(|(frame, _)| frame))
    }

    /// Receive one JSON frame and report its complete serialized wire size,
    /// including the four-byte length prefix.
    async fn recv_json_with_frame_bytes(&mut self) -> Result<Option<(serde_json::Value, usize)>> {
        match read_frame(&mut self.recv).await? {
            Some(body) => {
                let bytes = body
                    .len()
                    .checked_add(4)
                    .context("frame byte count overflow")?;
                let frame = serde_json::from_slice(&body).context("decode frame")?;
                Ok(Some((frame, bytes)))
            }
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
        let mut budget = RecordsBudget::default();
        for (_, page) in self.record_pages(capability, prefix).await? {
            // Validate each page's records against caps, ordering, and total count
            for (key, value) in &page {
                budget.validate_record(key, value)
                    .with_context(|| format!("invalid record in page"))?;
            }
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
        let mut budget = RecordsBudget::default();
        
        // Validate the first page
        for (key, value) in &all {
            budget.validate_record(key, value)
                .with_context(|| format!("invalid record in first page (sequence {sequence})"))?;
        }
        
        for (page_sequence, page) in pages {
            anyhow::ensure!(
                page_sequence == sequence,
                "the world changed while its records were read"
            );
            
            // Validate subsequent pages
            for (key, value) in &page {
                budget.validate_record(key, value)
                    .with_context(|| format!("invalid record in page (sequence {page_sequence})"))?;
            }
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
        let mut budget = RecordsBudget::default();
        loop {
            self.send_json(&serde_json::json!({
                "type": "records",
                "capability": capability,
                "prefix": prefix,
                "after": after.as_deref(),
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
            
            // Validate the page's records against caps, ordering, and total count
            for (key, value) in &page {
                budget.validate_record(key, value)
                    .with_context(|| format!("invalid record in page for sequence {sequence}"))?;
            }
            
            // Check cursor validation: next_after must equal the last key of this page
            match reply["next_after"].as_str() {
                Some(next) => {
                    // Validate that next_after matches the last key in this page
                    if let Some((last_key, _)) = page.iter().next_back() {
                        anyhow::ensure!(
                            next == last_key,
                            "invalid records cursor: next_after={next:?} does not match last key {last_key:?}"
                        );
                    } else {
                        // Empty page should only happen when after is not null
                        anyhow::ensure!(
                            after.is_some(),
                            "empty records page returned on first request"
                        );
                    }
                    
                    // Validate that the cursor advanced from what we sent
                    if let Some(cursor) = &after {
                        anyhow::ensure!(
                            next > cursor.as_str(),
                            "records cursor did not advance: next_after={next:?}, previous cursor={cursor:?}"
                        );
                    }
                    
                    after = Some(next.to_owned());
                }
                None => {
                    // No more pages
                    return Ok(pages);
                }
            }
            
            pages.push((sequence, page));
        }
    }

    /// Export the retained journal-order event window and its records as an
    /// administrator. Join capabilities cannot request this frame.
    ///
    /// # Errors
    ///
    /// Host refusal, malformed/out-of-order frames, or an invalid admin token.
    pub(crate) async fn admin_event_history(
        &mut self,
        admin_token: &str,
    ) -> Result<crate::world::AdminEventHistory> {
        self.send_json(&serde_json::json!({
            "type": "event_history_admin",
            "admin_token": admin_token,
        }))
        .await?;
        let mut budget = AdminHistoryBudget::default();
        let (begin, frame_bytes) = self
            .recv_json_with_frame_bytes()
            .await?
            .context("host closed before admin history export")?;
        budget.add_frame(frame_bytes)?;
        if begin["type"] != "event_history_begin" {
            bail!(
                "{}",
                begin["message"]
                    .as_str()
                    .unwrap_or("host refused admin history export")
            );
        }
        let world_id = begin["world_id"]
            .as_str()
            .context("history reply has no world ID")?
            .to_owned();
        let module_hash = begin["module_hash"].as_str().map(str::to_owned);
        let current_sequence = begin["current_sequence"]
            .as_u64()
            .context("history reply has no sequence")?;
        let complete = begin["complete"]
            .as_bool()
            .context("history reply has no completeness flag")?;
        let event_count = begin["event_count"]
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count <= crate::world::MAX_ADMIN_HISTORY_EVENTS)
            .context("history reply has an invalid event count")?;
        let mut events = Vec::with_capacity(event_count);
        for _ in 0..event_count {
            let (frame, frame_bytes) = self
                .recv_json_with_frame_bytes()
                .await?
                .context("host closed during admin history export")?;
            budget.add_frame(frame_bytes)?;
            if frame["type"] != "event_history_event" {
                bail!(
                    "{}",
                    frame["message"]
                        .as_str()
                        .unwrap_or("unexpected frame during admin history export")
                );
            }
            let event_bytes = serde_json::to_vec(&frame["event"])
                .context("measure an admin history event")?
                .len();
            budget.add_event(event_bytes)?;
            events.push(
                serde_json::from_value(frame["event"].clone())
                    .context("decode an admin history event")?,
            );
        }
        let mut records = crate::world::Records::new();
        let mut after: Option<String> = None;
        loop {
            let (frame, frame_bytes) = self
                .recv_json_with_frame_bytes()
                .await?
                .context("host closed before admin history records")?;
            budget.add_frame(frame_bytes)?;
            if frame["type"] != "records" {
                bail!(
                    "{}",
                    frame["message"]
                        .as_str()
                        .unwrap_or("unexpected frame during admin history records")
                );
            }
            anyhow::ensure!(
                frame["sequence"].as_u64() == Some(current_sequence),
                "admin history records do not match its event sequence"
            );
            let page: crate::world::Records = serde_json::from_value(frame["records"].clone())
                .context("decode admin history records")?;
            let page_cursor = after.clone();
            for (key, value) in page {
                anyhow::ensure!(
                    after.as_deref().is_none_or(|cursor| key.as_str() > cursor),
                    "admin history record pages are out of order"
                );
                budget.add_record(&key, &value)?;
                after = Some(key.clone());
                records.insert(key, value);
            }
            match frame["next_after"].as_str() {
                Some(next) => {
                    anyhow::ensure!(
                        after.as_deref() == Some(next),
                        "invalid admin history cursor"
                    );
                    anyhow::ensure!(
                        page_cursor.as_deref().is_none_or(|cursor| next > cursor),
                        "admin history record cursor did not advance"
                    );
                }
                None => break,
            }
        }
        Ok(crate::world::AdminEventHistory {
            world_id,
            module_hash,
            current_sequence,
            complete,
            events,
            records,
        })
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

    /// Whether this client's connection has closed, locally or by the
    /// peer: a closed connection can never serve another stream.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }
}

/// Client-side coordinator for concurrent world sessions (F13.8, #3845).
///
/// [`WorldClient`] is the transport primitive for one server endpoint: one
/// connection, and sessions as streams on it, each bound to one world for
/// its lifetime. `WorldSessions` is the client-side owner above it: it
/// keeps one root connection per distinct server endpoint and retains
/// independently opened session handles keyed by (endpoint, world), so a
/// single client can hold live sessions to several worlds — on one server
/// or across servers — at once.
///
/// Boundaries (per the F13.8 design record): a connection never spans
/// distinct servers (one root [`WorldClient`] per endpoint); hosting is
/// never started implicitly; every session is opened through the existing
/// capability-bearing `open_session` path, so per-world capability checks
/// are unchanged; the sequential `open_session` and one-shot CLI paths are
/// untouched.
#[derive(Debug)]
pub struct WorldSessions {
    endpoint: Endpoint,
    /// Root connections, keyed by endpoint id.
    roots: HashMap<String, WorldClient>,
    /// Live sessions, keyed by (endpoint id, world name; `""` = host default).
    sessions: HashMap<(String, String), WorldClient>,
}

impl WorldSessions {
    /// A coordinator dialing from `endpoint`.
    #[must_use]
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            roots: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    fn key(addr: &EndpointAddr, world: Option<&str>) -> (String, String) {
        (addr.id.to_string(), world.unwrap_or_default().to_owned())
    }

    /// Open the session for (`addr`, `world`), or return the live one.
    ///
    /// The first session to an endpoint dials its root connection; later
    /// sessions to the same endpoint reuse it via
    /// [`WorldClient::open_session`]. A session to a different endpoint
    /// always dials a separate connection.
    ///
    /// # Errors
    ///
    /// Fails if the endpoint is unreachable or the host refuses a session.
    pub async fn open_session(
        &mut self,
        addr: EndpointAddr,
        world: Option<&str>,
    ) -> Result<&mut WorldClient> {
        let key = Self::key(&addr, world);
        // A root whose connection has closed (server disconnect or
        // restart) can never serve another stream, and every session
        // keyed to it rides that dead connection: evict both so this
        // endpoint redials on its own, instead of staying wedged until
        // `close_all` also drops healthy sessions on other servers.
        if self.roots.get(&key.0).is_some_and(WorldClient::is_closed) {
            self.roots.remove(&key.0);
            self.sessions
                .retain(|(endpoint_id, _), _| endpoint_id != &key.0);
        }
        if !self.sessions.contains_key(&key) {
            if !self.roots.contains_key(&key.0) {
                let root = WorldClient::connect(&self.endpoint, addr).await?;
                self.roots.insert(key.0.clone(), root);
            }
            let session = self
                .roots
                .get(&key.0)
                .context("root inserted above")?
                .open_session(world)
                .await?;
            self.sessions.insert(key.clone(), session);
        }
        Ok(self
            .sessions
            .get_mut(&key)
            .context("session inserted above")?)
    }

    /// The live session for (`addr`, `world`), if one is open.
    pub fn session(
        &mut self,
        addr: &EndpointAddr,
        world: Option<&str>,
    ) -> Option<&mut WorldClient> {
        self.sessions.get_mut(&Self::key(addr, world))
    }

    /// Whether a session for (`addr`, `world`) is currently open.
    #[must_use]
    pub fn has_session(&self, addr: &EndpointAddr, world: Option<&str>) -> bool {
        self.sessions.contains_key(&Self::key(addr, world))
    }

    /// Number of distinct server endpoints with a root connection.
    #[must_use]
    pub fn endpoint_count(&self) -> usize {
        self.roots.len()
    }

    /// Number of live sessions across all endpoints.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Drop one session. Its stream closes; sibling sessions on the same
    /// connection are unaffected (unlike [`WorldClient::close`], which
    /// closes the whole connection).
    pub fn close_session(&mut self, addr: &EndpointAddr, world: Option<&str>) -> bool {
        self.sessions.remove(&Self::key(addr, world)).is_some()
    }

    /// Drop every session and close every root connection.
    pub fn close_all(&mut self) {
        self.sessions.clear();
        for (_, root) in self.roots.drain() {
            root.close();
        }
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
        protocol: WorldProtocol,
    }

    async fn host(admin: Option<&str>) -> Host {
        let ep = endpoint().await;
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world.clone(), admin.map(str::to_owned));
        let protocol = WorldProtocol::new(service);
        let router = Router::builder(ep.clone())
            .accept(WORLD_ALPN, protocol.clone())
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
            protocol,
        }
    }

    async fn endpoint_with_key(key: [u8; 32]) -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .secret_key(iroh_base::SecretKey::from_bytes(&key))
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    async fn wait_for_connection_count(
        protocol: &WorldProtocol,
        endpoint: AbuseEndpointId,
        expected: usize,
    ) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count = protocol
                    .abuse
                    .lock()
                    .unwrap()
                    .active_connection_count(&endpoint);
                if count == expected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed out waiting for registered peer connections");
    }

    async fn next(client: &mut WorldClient) -> Option<serde_json::Value> {
        tokio::time::timeout(std::time::Duration::from_secs(10), client.recv_json())
            .await
            .expect("timed out waiting for a world frame")
            .unwrap()
    }

    #[tokio::test]
    async fn accept_enforces_per_peer_connection_cap_and_releases_slots() {
        let host = host(Some("admin")).await;
        let peer_key = [71; 32];
        let peer_id: AbuseEndpointId = *iroh_base::SecretKey::from_bytes(&peer_key)
            .public()
            .as_bytes();
        let mut clients = Vec::new();

        // Separate endpoint instances with the same identity model distinct
        // concurrent QUIC connections from one peer.
        for _ in 0..4 {
            let client_endpoint = endpoint_with_key(peer_key).await;
            let client = WorldClient::connect(&client_endpoint, host.addr.clone())
                .await
                .unwrap();
            clients.push((client_endpoint, client));
        }
        wait_for_connection_count(&host.protocol, peer_id, 4).await;

        let fifth_endpoint = endpoint_with_key(peer_key).await;
        let fifth = WorldClient::connect(&fifth_endpoint, host.addr.clone()).await;
        if let Ok(client) = fifth {
            client.close();
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.protocol.connection_cap_refusals() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fifth peer connection was not refused");
        assert_eq!(host.protocol.connection_cap_refusals(), 1);
        wait_for_connection_count(&host.protocol, peer_id, 4).await;

        // The same cap is independent per peer, not a server-wide limit.
        let other_endpoint = endpoint_with_key([72; 32]).await;
        let other_id: AbuseEndpointId = *other_endpoint.id().as_bytes();
        let mut other = WorldClient::connect(&other_endpoint, host.addr.clone())
            .await
            .unwrap();
        wait_for_connection_count(&host.protocol, other_id, 1).await;
        assert_eq!(host.protocol.connection_cap_refusals(), 1);
        other
            .invite("admin", "Other peer", None, None)
            .await
            .unwrap();

        // Closing one transport releases its registration and admits a new
        // connection from the peer that had reached the cap.
        let (closed_endpoint, closed_client) = clients.remove(0);
        closed_client.close();
        closed_endpoint.close().await;
        wait_for_connection_count(&host.protocol, peer_id, 3).await;

        let retry_endpoint = endpoint_with_key(peer_key).await;
        let mut retry = WorldClient::connect(&retry_endpoint, host.addr.clone())
            .await
            .unwrap();
        wait_for_connection_count(&host.protocol, peer_id, 4).await;
        retry
            .invite("admin", "Replacement peer", None, None)
            .await
            .unwrap();

        retry.close();
        retry_endpoint.close().await;
        other.close();
        other_endpoint.close().await;
        fifth_endpoint.close().await;
        for (client_endpoint, client) in clients {
            client.close();
            client_endpoint.close().await;
        }
        host.router.shutdown().await.unwrap();
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

    #[tokio::test]
    async fn inbound_refusal_backpressures_authorized_streams_but_sheds_preauth() {
        use crate::abuse::{AbusePolicy, BucketLimits};

        // One token per second against a burst of one: a drained bucket
        // stays empty for ~1 s, so the pre-auth shed below cannot race
        // the refill. At 1_000 tokens/sec the single burst token
        // refilled in 1 ms, and any >= 1 ms scheduling stall between
        // the two pre-auth charges admitted the second one instead —
        // the intermittent "a pre-auth refusal sheds the stream"
        // failure (#5195). The authorized leg keeps its shape: the
        // refusal sleeps the 1 s retry floor and then charges against
        // the refilled bucket, inside the existing 3 s bound.
        let limits = BucketLimits {
            frames_per_sec: 1,
            frame_burst: 1,
            bytes_per_sec: 1,
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

        // Pre-authorization: the first frame fits the burst, the next is
        // refused immediately — no waiting, the stream is shed. Every
        // await in this test is bounded (#5195): a wedged charge fails
        // loudly here instead of stalling the suite.
        let preauth = [3; 32];
        let flag = AtomicBool::new(false);
        assert!(
            tokio::time::timeout(
                Duration::from_secs(5),
                charge_inbound_frame(&evaluator, preauth, &flag, 1),
            )
            .await
            .expect("the first pre-auth charge should return promptly"),
            "the first frame fits the burst"
        );
        assert!(
            !tokio::time::timeout(
                Duration::from_millis(500),
                charge_inbound_frame(&evaluator, preauth, &flag, 1),
            )
            .await
            .expect("a pre-auth refusal returns immediately"),
            "a pre-auth refusal sheds the stream"
        );

        // Authorized: the same burst refusal becomes backpressure — the
        // charge completes once the bucket refills instead of failing.
        let authed = [4; 32];
        let flag = AtomicBool::new(true);
        assert!(
            tokio::time::timeout(
                Duration::from_secs(5),
                charge_inbound_frame(&evaluator, authed, &flag, 1),
            )
            .await
            .expect("the first authorized charge should return promptly"),
            "the first frame fits the burst"
        );
        assert!(
            tokio::time::timeout(
                Duration::from_secs(3),
                charge_inbound_frame(&evaluator, authed, &flag, 1),
            )
            .await
            .expect("the retry should complete after the token refills"),
            "an authorized refusal is backpressure, not a closed stream"
        );
    }

    #[tokio::test]
    async fn successful_join_switches_only_that_stream_to_authorized_limits() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;
        let mut inviter = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        inviter
            .send_json(&serde_json::json!({
                "type": "invite",
                "admin_token": "admin",
                "display_name": "Guest"
            }))
            .await
            .unwrap();
        let invite = next(&mut inviter).await.unwrap();
        let capability = invite["capability"].as_str().unwrap().to_owned();
        inviter.close();

        let mut client = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        client.send_json(&join(&capability, None)).await.unwrap();
        assert_eq!(next(&mut client).await.unwrap()["type"], "snapshot");

        // The pre-authorization burst is 30 frames. The join uses one
        // pre-auth frame; after the capability is validated, this stream
        // can continue past that limit while chat's own 50-message cap
        // remains unchanged.
        for index in 0..31 {
            client
                .send_json(&serde_json::json!({
                    "type": "chat",
                    "text": format!("authorized {index}")
                }))
                .await
                .unwrap();
            assert_eq!(next(&mut client).await.unwrap()["type"], "event");
        }

        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    fn join(capability: &str, after: Option<u64>) -> serde_json::Value {
        serde_json::json!({"type": "join", "capability": capability, "after": after})
    }

    #[test]
    fn admin_history_client_rejects_over_limit_payload() {
        let mut event_budget = AdminHistoryBudget::default();
        assert!(
            event_budget
                .add_event(crate::world::MAX_ADMIN_HISTORY_BYTES)
                .is_ok()
        );
        assert!(event_budget.add_event(1).is_err());

        let mut record_budget = AdminHistoryBudget::default();
        for _ in 0..crate::world::MAX_RECORDS {
            assert!(
                record_budget
                    .add_record("key", &serde_json::Value::Null)
                    .is_ok()
            );
        }
        assert!(
            record_budget
                .add_record("extra", &serde_json::Value::Null)
                .is_err()
        );
        assert!(
            record_budget
                .add_record(
                    "key",
                    &serde_json::Value::String(
                        "x".repeat(crate::world::MAX_RECORD_VALUE_BYTES + 1)
                    )
                )
                .is_err()
        );

        let frame = serde_json::json!({
            "type": "event_history_begin",
            "world_id": "lobby",
            "current_sequence": 0,
            "complete": true,
            "event_count": 0,
            "ignored": "x".repeat(4096),
        });
        let wire_bytes = serde_json::to_vec(&frame).unwrap().len() + 4;
        let mut padded_budget = AdminHistoryBudget {
            response_bytes: MAX_ADMIN_HISTORY_RESPONSE_BYTES - wire_bytes + 1,
            ..AdminHistoryBudget::default()
        };
        assert!(
            padded_budget.add_frame(wire_bytes).is_err(),
            "ignored top-level JSON fields count toward the transfer limit"
        );
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

    /// Host a multi-world hub (MemoryOpener) on one endpoint; returns the
    /// router, the hub, and the loopback address peers should dial.
    async fn hub_host() -> (Router, WorldHub, EndpointAddr, Endpoint) {
        let ep = endpoint().await;
        let opener = crate::world_hub::testing::MemoryOpener::new(false);
        let dyn_opener: Arc<dyn crate::world_hub::WorldOpener> = opener;
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
        (router, hub, addr, ep)
    }

    /// Mint a capability for `world` on `addr` through a throwaway client.
    async fn invite_for(
        client_ep: &Endpoint,
        addr: &EndpointAddr,
        world: Option<&str>,
        name: &str,
    ) -> String {
        let root = WorldClient::connect(client_ep, addr.clone()).await.unwrap();
        let mut inviter = root.open_session(world).await.unwrap();
        let capability = inviter.invite("admin", name, None, None).await.unwrap();
        drop(inviter);
        root.close();
        capability
    }

    #[tokio::test]
    async fn coordinator_holds_two_worlds_on_one_endpoint_concurrently() {
        let (router, hub, addr, host_ep) = hub_host().await;
        let client_ep = endpoint().await;
        let cap_alpha = invite_for(&client_ep, &addr, Some("alpha"), "ann").await;
        let cap_beta = invite_for(&client_ep, &addr, Some("beta"), "bob").await;

        let mut sessions = WorldSessions::new(client_ep.clone());
        // Join both worlds and keep both sessions live at once.
        for (world, cap) in [("alpha", &cap_alpha), ("beta", &cap_beta)] {
            let s = sessions
                .open_session(addr.clone(), Some(world))
                .await
                .unwrap();
            s.send_json(&join(cap, None)).await.unwrap();
            assert_eq!(next(s).await.unwrap()["type"], "snapshot");
        }
        assert_eq!(sessions.endpoint_count(), 1);
        assert_eq!(sessions.session_count(), 2);
        assert_eq!(hub.open_count(), 2);

        // Chat in alpha is delivered to the alpha session only: the beta
        // session stays silent, proving the two live sessions do not
        // cross-talk on the shared connection.
        let alpha = sessions.session(&addr, Some("alpha")).unwrap();
        alpha
            .send_json(&serde_json::json!({"type": "chat", "text": "alpha only"}))
            .await
            .unwrap();
        let frame = next(alpha).await.unwrap();
        assert_eq!(frame["type"], "event");
        assert!(frame.to_string().contains("alpha only"));
        let beta = sessions.session(&addr, Some("beta")).unwrap();
        let silent =
            tokio::time::timeout(std::time::Duration::from_millis(500), beta.recv_json()).await;
        assert!(
            silent.is_err(),
            "beta session received alpha's chat: {silent:?}"
        );

        // Re-opening an already-live session returns it, not a new stream:
        // the same joined session still chats and receives its own event.
        let again = sessions
            .open_session(addr.clone(), Some("alpha"))
            .await
            .unwrap();
        again
            .send_json(&serde_json::json!({"type": "chat", "text": "still alpha"}))
            .await
            .unwrap();
        let frame = next(again).await.unwrap();
        assert!(frame.to_string().contains("still alpha"));
        assert_eq!(sessions.session_count(), 2);

        sessions.close_all();
        client_ep.close().await;
        router.shutdown().await.unwrap();
        host_ep.close().await;
    }

    #[tokio::test]
    async fn coordinator_spans_distinct_server_endpoints() {
        let host_a = host(Some("admin")).await;
        let host_b = host(Some("admin")).await;
        let client_ep = endpoint().await;

        let mut sessions = WorldSessions::new(client_ep.clone());
        for host in [&host_a, &host_b] {
            let cap = invite_for(&client_ep, &host.addr, None, "ann").await;
            let s = sessions
                .open_session(host.addr.clone(), None)
                .await
                .unwrap();
            s.send_json(&join(&cap, None)).await.unwrap();
            assert_eq!(next(s).await.unwrap()["type"], "snapshot");
        }
        // Distinct servers: two root connections, never one shared.
        assert_eq!(sessions.endpoint_count(), 2);
        assert_eq!(sessions.session_count(), 2);

        // A chat on server A reaches only server A's session.
        let a = sessions.session(&host_a.addr, None).unwrap();
        a.send_json(&serde_json::json!({"type": "chat", "text": "on a"}))
            .await
            .unwrap();
        let frame = next(a).await.unwrap();
        assert!(frame.to_string().contains("on a"));

        sessions.close_all();
        client_ep.close().await;
        host_a.router.shutdown().await.unwrap();
        host_b.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn second_join_through_coordinator_does_not_mutate_the_journal() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;
        let cap = invite_for(&client_ep, &host.addr, None, "ann").await;

        // Journal state before any join: the world's committed sequence and
        // its records generation.
        let mut probe = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let (_, _, seq_before) = probe.info(&cap).await.unwrap();
        probe.close();
        let (records_gen_before, _) = host.world.records();

        let mut sessions = WorldSessions::new(client_ep.clone());
        let first = sessions
            .open_session(host.addr.clone(), None)
            .await
            .unwrap();
        first.send_json(&join(&cap, None)).await.unwrap();
        assert_eq!(next(first).await.unwrap()["type"], "snapshot");

        // A second, independent client joins the same world while the first
        // session stays live.
        let mut second = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        second.send_json(&join(&cap, None)).await.unwrap();
        assert_eq!(next(&mut second).await.unwrap()["type"], "snapshot");
        second.close();

        let mut probe = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let (_, _, seq_after) = probe.info(&cap).await.unwrap();
        probe.close();
        let (records_gen_after, _) = host.world.records();
        assert_eq!(
            seq_after, seq_before,
            "a join must not append to the journal"
        );
        assert_eq!(records_gen_after, records_gen_before);

        sessions.close_all();
        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn coordinator_replaces_a_closed_root_connection_on_reopen() {
        let host = host(Some("admin")).await;
        let client_ep = endpoint().await;
        let cap = invite_for(&client_ep, &host.addr, None, "ann").await;

        let mut sessions = WorldSessions::new(client_ep.clone());
        let s = sessions
            .open_session(host.addr.clone(), None)
            .await
            .unwrap();
        s.send_json(&join(&cap, None)).await.unwrap();
        assert_eq!(next(s).await.unwrap()["type"], "snapshot");

        // The root connection dies, as on a server restart: close it and
        // wait until the close has landed.
        let conn = sessions
            .roots
            .get(&host.addr.id.to_string())
            .unwrap()
            .conn
            .clone();
        conn.close(0_u32.into(), b"root connection died");
        conn.closed().await;

        // The caller drops the failed session and reopens: the coordinator
        // must redial a fresh root for this endpoint, not reuse the closed
        // connection (which would wedge this endpoint until close_all).
        assert!(sessions.close_session(&host.addr, None));
        let s = sessions
            .open_session(host.addr.clone(), None)
            .await
            .unwrap();
        s.send_json(&join(&cap, None)).await.unwrap();
        assert_eq!(next(s).await.unwrap()["type"], "snapshot");
        assert_eq!(sessions.endpoint_count(), 1);
        assert_eq!(sessions.session_count(), 1);

        sessions.close_all();
        client_ep.close().await;
        host.router.shutdown().await.unwrap();
    }
}
