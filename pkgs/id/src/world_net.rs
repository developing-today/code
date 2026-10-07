//! Iroh transport for world sessions (`/id-world/1`).
//!
//! One bidirectional QUIC stream carries one session. A frame is a big-endian
//! `u32` byte length followed by one UTF-8 JSON object of at most
//! [`MAX_FRAME_BYTES`]; the JSON is exactly the protocol in
//! [`crate::world_session`], so this module only moves frames.
//!
//! Iroh proves *which node* connected. That is logged for operators but never
//! grants anything: the join capability inside the session is the only
//! authority over a world.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, ReadExactError, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::{JoinHandle, JoinSet},
};

use crate::world_hub::WorldHub;
use crate::world_session::{
    Inbound, MAX_FRAME_BYTES, MAX_WORLD_MODULE_CHUNK_BYTES, SessionIo, module_hash, run_session,
};

/// ALPN for world sessions. Within version 1 frames may gain fields and
/// variants; removing or changing one needs `/id-world/2`.
pub const WORLD_ALPN: &[u8] = b"/id-world/1";

/// Concurrent sessions allowed on one connection.
const MAX_SESSIONS_PER_CONNECTION: usize = 16;

use crate::world_session::encode_hex;

/// Write one frame.
///
/// # Errors
///
/// Fails if the payload exceeds [`MAX_FRAME_BYTES`] or the stream is closed.
pub async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_FRAME_BYTES {
        bail!("frame of {} bytes exceeds {MAX_FRAME_BYTES}", payload.len());
    }
    let len = u32::try_from(payload.len()).context("frame length")?;
    // One write so a frame is never split by a cancelled caller.
    let mut framed = Vec::with_capacity(4 + payload.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(payload);
    send.write_all(&framed).await.context("write frame")?;
    Ok(())
}

/// Read one frame. `Ok(None)` means the peer finished cleanly between frames.
///
/// This is **not** cancel-safe; run it in its own task, not inside `select!`.
///
/// # Errors
///
/// Fails on a truncated frame, a stream error, or a length above
/// [`MAX_FRAME_BYTES`] (the oversized body is never read).
pub async fn read_frame(recv: &mut RecvStream) -> Result<Option<Vec<u8>>> {
    let mut len = [0_u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e).context("read frame length"),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME_BYTES {
        bail!("frame of {len} bytes exceeds {MAX_FRAME_BYTES}");
    }
    let mut body = vec![0_u8; len];
    recv.read_exact(&mut body)
        .await
        .context("read frame body")?;
    Ok(Some(body))
}

/// Accepts world sessions over Iroh. Register with
/// `Router::builder(..).accept(WORLD_ALPN, WorldProtocol::new(hub))`.
#[derive(Clone, Debug)]
pub struct WorldProtocol {
    hub: WorldHub,
}

impl WorldProtocol {
    /// Serve a hub of worlds (or a single [`WorldService`], which converts
    /// into a one-world hub) to Iroh peers.
    #[must_use]
    pub fn new(hub: impl Into<WorldHub>) -> Self {
        Self { hub: hub.into() }
    }
}

impl ProtocolHandler for WorldProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
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
            sessions.spawn(async move {
                let _permit = permit;
                let mut io = StreamIo::new(send, recv);
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
}

impl StreamIo {
    fn new(send: SendStream, mut recv: RecvStream) -> Self {
        let (tx, frames) = mpsc::channel(8);
        let reader = tokio::spawn(async move {
            loop {
                let inbound = match read_frame(&mut recv).await {
                    Ok(Some(body)) => match String::from_utf8(body) {
                        Ok(text) => Inbound::Text(text),
                        Err(_) => Inbound::Unsupported,
                    },
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
        write_frame(&mut self.send, frame.as_bytes()).await.is_ok()
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
        let conn = endpoint
            .connect(addr, WORLD_ALPN)
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
        // The host reads the routing field from a session's first frame only;
        // adding it to every frame is harmless and keeps callers simple.
        match (&self.world, frame) {
            (Some(world), serde_json::Value::Object(object)) if !object.contains_key("world") => {
                let mut routed = object.clone();
                routed.insert("world".to_owned(), serde_json::Value::from(world.clone()));
                write_frame(
                    &mut self.send,
                    serde_json::Value::Object(routed).to_string().as_bytes(),
                )
                .await
            }
            _ => write_frame(&mut self.send, frame.to_string().as_bytes()).await,
        }
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
    /// afterwards.
    ///
    /// # Errors
    ///
    /// Fails with the host's message if the invite is refused.
    pub async fn invite(&mut self, admin_token: &str, display_name: &str) -> Result<String> {
        self.send_json(&serde_json::json!({
            "type": "invite",
            "admin_token": admin_token,
            "display_name": display_name,
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
                Ok((world_id, active_module_hash, current_sequence))
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
            anyhow::ensure!(
                reply["sequence"].as_u64().is_some(),
                "records reply has no sequence"
            );
            let page: crate::world::Records =
                serde_json::from_value(reply["records"].clone()).context("decode records page")?;
            all.extend(page);
            match reply["next_after"].as_str() {
                Some(next) => after = Some(next.to_owned()),
                None => return Ok(all),
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
        self.send_json(&serde_json::json!({
            "type": "download_module",
            "capability": capability,
            "module_hash": expected_hash,
        }))
        .await?;
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
    use crate::world_session::WorldService;

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
        let capability = invite_client.invite("admin", "ann").await.unwrap();
        invite_client.close();

        let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
        client.send_json(&join(&capability, None)).await.unwrap();
        assert_eq!(next(&mut client).await.unwrap()["type"], "snapshot");
        let initial = next(&mut client).await.unwrap();
        assert_eq!(initial["type"], "view");
        assert_eq!(initial["data"], "count=0");

        client
            .send_json(&serde_json::json!({ "type": "input", "data_hex": "696e63" }))
            .await
            .unwrap();
        let event = next(&mut client).await.unwrap();
        assert_eq!(event["type"], "event");
        let view = next(&mut client).await.unwrap();
        assert_eq!(view["data"], "count=1");

        client_ep.close().await;
        router.shutdown().await.unwrap();
        ep.close().await;
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
        let capability = inviter.invite("admin", "ann").await.unwrap();
        inviter.close();

        let mut query = WorldClient::connect(&client_ep, addr.clone())
            .await
            .unwrap();
        let (_, active, _) = query.info(&capability).await.unwrap();
        query.close();
        assert_eq!(active.as_deref(), Some(hash.as_str()));

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
        let capability = inviter.invite("admin", "ann").await.unwrap();
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
        let mut tasks = tokio::task::JoinSet::new();
        for client_no in 0..6 {
            let addr = addr.clone();
            tasks.spawn(async move {
                let client_ep = endpoint().await;
                let root = WorldClient::connect(&client_ep, addr).await.unwrap();
                for world in ["alpha", "beta", "gamma"] {
                    let mut inviter = root.open_session(Some(world)).await.unwrap();
                    let capability = inviter
                        .invite("admin", &format!("player{client_no}"))
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
        let capability = ok.invite("admin", "ann").await.unwrap();
        assert!(!capability.is_empty());

        let mut bad = WorldClient::connect(&client_ep, host.addr.clone())
            .await
            .unwrap();
        let err = bad.invite("wrong", "eve").await.unwrap_err();
        assert!(err.to_string().contains("invite denied"), "{err:#}");

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
