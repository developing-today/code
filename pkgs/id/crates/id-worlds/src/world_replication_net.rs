//! Signed journal replication streams for world writers.
//!
//! The dedicated ALPN keeps the authoritative journal feed separate from the
//! interactive participant protocol and the records-only mirror. A client
//! opens one bidirectional stream, writes a JSON request frame containing the
//! selected world, the hub admin credential, and a fresh session nonce,
//! then receives length-prefixed signed replication frames until it disconnects.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use iroh::{
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::{
    frames::{read_frame, write_frame},
    world::{
        JournalSnapshot, ReplicationCursor, ReplicationEnvelope, ReplicationUpdate,
        SignedReplicationEnvelope, chain_base_cursor, frame_digest, split_entry_payload,
    },
    world_hub::WorldHub,
    world_session::MAX_ADMIN_TOKEN_BYTES,
};

/// ALPN for the signed, append-ordered world journal feed.
pub const REPLICATION_ALPN: &[u8] = b"/id-world-replication/1";

/// Bound long-lived replication streams one peer can hold on a connection.
const MAX_STREAMS_PER_CONNECTION: usize = 8;

/// Bound for a single replication frame write (#4726), matching the 30 s
/// bound the request read already has. A peer that stops reading cannot
/// be allowed to pin the stream — and with it the world lease and the
/// hub session permit the stream holds — forever.
const FRAME_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// One request per QUIC stream. `world` defaults to the hub's default world.
#[derive(Deserialize)]
pub struct ReplicationRequest {
    /// World name to replicate; omitted for the server's default world.
    #[serde(default)]
    pub world: Option<String>,
    /// Hub admin credential authorizing access to the raw journal.
    pub admin_token: String,
    /// Fresh, client-generated nonce bound into every signed frame.
    pub session_nonce: [u8; 16],
}

impl std::fmt::Debug for ReplicationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplicationRequest")
            .field("world", &self.world)
            .field("admin_token", &"[REDACTED]")
            .field("session_nonce", &self.session_nonce)
            .finish()
    }
}

/// Serves durable journal snapshots and tails for worlds in a hub.
#[derive(Clone)]
pub struct WorldReplicationProtocol {
    hub: WorldHub,
    signing_key: Arc<SigningKey>,
}

impl std::fmt::Debug for WorldReplicationProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldReplicationProtocol")
            .field("hub", &self.hub)
            .field("signing_key", &"[REDACTED]")
            .finish()
    }
}

impl WorldReplicationProtocol {
    /// Construct a writer protocol using the serving node's Iroh identity key.
    #[must_use]
    pub fn new(hub: WorldHub, signing_key: SigningKey) -> Self {
        Self {
            hub,
            signing_key: Arc::new(signing_key),
        }
    }
}

impl ProtocolHandler for WorldReplicationProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let mut streams = JoinSet::new();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            while streams.try_join_next().is_some() {}
            if streams.len() >= MAX_STREAMS_PER_CONNECTION {
                let _ = send.reset(1_u32.into());
                let _ = recv.stop(1_u32.into());
                continue;
            }
            let hub = self.hub.clone();
            let signing_key = Arc::clone(&self.signing_key);
            streams.spawn(async move {
                if let Err(error) = serve_stream(&hub, &signing_key, send, recv).await {
                    tracing::debug!("world replication stream ended: {error:#}");
                }
            });
        }
        streams.shutdown().await;
        Ok(())
    }
}

async fn serve_stream(
    hub: &WorldHub,
    signing_key: &SigningKey,
    mut send: SendStream,
    mut recv: RecvStream,
) -> Result<()> {
    let request_bytes =
        tokio::time::timeout(std::time::Duration::from_secs(30), read_frame(&mut recv))
            .await
            .context("timed out waiting for replication request")??
            .context("replication stream closed before request")?;
    let request: ReplicationRequest =
        serde_json::from_slice(&request_bytes).context("decode replication request")?;
    let _ = recv.stop(0_u32.into());
    ensure_nonce(&request.session_nonce)?;
    if request.admin_token.len() > MAX_ADMIN_TOKEN_BYTES {
        bail!("replication admin token exceeds {MAX_ADMIN_TOKEN_BYTES} bytes");
    }
    if !hub.admin_ok(&request.admin_token) {
        bail!("replication read denied");
    }
    let lease = hub
        .lease(request.world.as_deref(), None)
        .await
        .map_err(|error| anyhow::anyhow!("open replication world: {}", error.message()))?;
    let (world_id, snapshot_source, mut updates) =
        lease.service().world().replication_feed().await?;
    let source = *signing_key.verifying_key().as_bytes();
    // Only the pinned file handle and captured length crossed the actor
    // boundary. Read the initial image on the blocking pool, then confine its
    // owned bytes to this transfer so they are dropped before following
    // updates for the lifetime of the stream.
    let (mut cursor, mut header_version, mut prev_frame_digest) = {
        let snapshot = tokio::task::spawn_blocking(move || snapshot_source.read())
            .await
            .context("initial replication snapshot read task")??;
        send_snapshot(
            &mut send,
            signing_key,
            &world_id,
            source,
            request.session_nonce,
            snapshot,
            [0; 32],
        )
        .await?
    };

    loop {
        match updates.recv().await {
            Ok(ReplicationUpdate::Append {
                header_version: update_version,
                entry,
            }) => {
                if update_version != header_version {
                    bail!("journal header changed without a reset snapshot");
                }
                let (after, digest) = send_tail(
                    &mut send,
                    signing_key,
                    &world_id,
                    source,
                    request.session_nonce,
                    header_version,
                    cursor,
                    prev_frame_digest,
                    entry,
                )
                .await?;
                cursor = after;
                prev_frame_digest = digest;
            }
            Ok(ReplicationUpdate::Reset(reset)) => {
                let (after, version, digest) = send_snapshot(
                    &mut send,
                    signing_key,
                    &world_id,
                    source,
                    request.session_nonce,
                    reset,
                    prev_frame_digest,
                )
                .await?;
                cursor = after;
                header_version = version;
                prev_frame_digest = digest;
            }
            Ok(ReplicationUpdate::ResetFailed) => {
                bail!("writer could not capture a required journal reset snapshot");
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                bail!(
                    "replication subscriber fell behind by {skipped} durable updates; reconnect for a fresh snapshot"
                );
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                bail!("world replication feed closed");
            }
        }
    }
}

fn ensure_nonce(nonce: &[u8; 16]) -> Result<()> {
    if nonce.iter().all(|byte| *byte == 0) {
        bail!("replication session nonce must not be all zeroes");
    }
    Ok(())
}

// Take ownership of each snapshot so its journal buffer cannot escape into
// the long-lived update loop after the transfer has been sent.
async fn send_snapshot(
    send: &mut SendStream,
    signing_key: &SigningKey,
    world_id: &str,
    source: [u8; 32],
    nonce: [u8; 16],
    snapshot: JournalSnapshot,
    mut prev_frame_digest: [u8; 32],
) -> Result<(ReplicationCursor, u32, [u8; 32])> {
    let chunks = split_entry_payload(&snapshot.bytes).context("split journal snapshot")?;
    if chunks.is_empty() {
        bail!("cannot replicate a journal without its Created header");
    }
    let mut cursor = chain_base_cursor(world_id, &source, snapshot.header_version);
    for (index, payload) in chunks.into_iter().enumerate() {
        let envelope = if index == 0 {
            ReplicationEnvelope::snapshot(
                world_id,
                source,
                snapshot.header_version,
                nonce,
                prev_frame_digest,
                payload,
            )?
        } else {
            ReplicationEnvelope::tail(
                world_id,
                source,
                snapshot.header_version,
                nonce,
                cursor,
                prev_frame_digest,
                payload,
            )?
        };
        cursor = envelope.cursor_after;
        prev_frame_digest = write_envelope(send, signing_key, envelope).await?;
    }
    prev_frame_digest = send_terminal(
        send,
        signing_key,
        world_id,
        source,
        nonce,
        snapshot.header_version,
        cursor,
        prev_frame_digest,
    )
    .await?;
    Ok((cursor, snapshot.header_version, prev_frame_digest))
}

async fn send_tail(
    send: &mut SendStream,
    signing_key: &SigningKey,
    world_id: &str,
    source: [u8; 32],
    nonce: [u8; 16],
    header_version: u32,
    cursor: ReplicationCursor,
    prev_frame_digest: [u8; 32],
    entry: Vec<u8>,
) -> Result<(ReplicationCursor, [u8; 32])> {
    let envelope = ReplicationEnvelope::tail(
        world_id,
        source,
        header_version,
        nonce,
        cursor,
        prev_frame_digest,
        entry,
    )?;
    let after = envelope.cursor_after;
    let digest = write_envelope(send, signing_key, envelope).await?;
    let digest = send_terminal(
        send,
        signing_key,
        world_id,
        source,
        nonce,
        header_version,
        after,
        digest,
    )
    .await?;
    Ok((after, digest))
}

async fn send_terminal(
    send: &mut SendStream,
    signing_key: &SigningKey,
    world_id: &str,
    source: [u8; 32],
    nonce: [u8; 16],
    header_version: u32,
    cursor: ReplicationCursor,
    prev_frame_digest: [u8; 32],
) -> Result<[u8; 32]> {
    let envelope = ReplicationEnvelope::terminal(
        world_id,
        source,
        header_version,
        nonce,
        cursor,
        prev_frame_digest,
    );
    write_envelope(send, signing_key, envelope).await
}

async fn write_envelope(
    send: &mut SendStream,
    signing_key: &SigningKey,
    envelope: ReplicationEnvelope,
) -> Result<[u8; 32]> {
    let signed = SignedReplicationEnvelope::sign(envelope, signing_key)
        .context("replication signing key does not match source")?;
    let bytes = signed.encode_frame().context("encode replication frame")?;
    write_frame_bounded(write_frame(send, &bytes), FRAME_WRITE_TIMEOUT).await?;
    Ok(frame_digest(&bytes))
}

/// Run one frame write under a deadline (#4726). On expiry the returned
/// error says the write *timed out* — distinguishable from an ordinary
/// write failure, which propagates unchanged — and the caller tears the
/// stream down, releasing the world lease and hub session permit it held.
async fn write_frame_bounded<F, T>(write: F, timeout: std::time::Duration) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout, write).await {
        Ok(result) => result,
        Err(_elapsed) => bail!(
            "replication frame write timed out after {}s (peer not reading)",
            timeout.as_secs()
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use iroh::{
        Endpoint, EndpointAddr, TransportAddr,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::{
        world::WorldLimits, world_hub::WorldHub, world_limits::RuntimeLimits,
        world_session::WorldService, world_store::open_world,
    };

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    fn endpoint_addr(endpoint: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            endpoint.id(),
            endpoint.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    async fn request_frame(
        endpoint: &Endpoint,
        addr: EndpointAddr,
        admin_token: &str,
    ) -> Result<Option<Vec<u8>>> {
        let connection = endpoint.connect(addr, REPLICATION_ALPN).await?;
        let (mut send, mut recv) = connection.open_bi().await?;
        let request = serde_json::json!({
            "admin_token": admin_token,
            "session_nonce": vec![1_u8; 16],
        });
        write_frame(&mut send, &serde_json::to_vec(&request)?).await?;
        send.finish()?;
        let frame = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv)).await??;
        connection.close(0_u32.into(), b"test complete");
        Ok(frame)
    }

    #[test]
    fn request_requires_nonzero_session_nonce() {
        assert!(ensure_nonce(&[0; 16]).is_err());
        assert!(ensure_nonce(&[1; 16]).is_ok());
    }

    #[tokio::test]
    async fn a_stalled_frame_write_is_cut_off_by_the_timeout() {
        // A non-reading peer: the write never completes, so only the
        // deadline can end it (#4726 regression test).
        let stalled = write_frame_bounded(
            std::future::pending::<Result<()>>(),
            std::time::Duration::from_millis(10),
        )
        .await;
        let error = stalled.expect_err("a stalled write must be cut off");
        assert!(
            error.to_string().contains("timed out"),
            "timeout error must be distinguishable, got: {error}"
        );
    }

    #[tokio::test]
    async fn a_prompt_frame_write_passes_through_untouched() {
        let written = write_frame_bounded(async { Ok(7_u32) }, std::time::Duration::from_secs(30))
            .await
            .unwrap();
        assert_eq!(written, 7);
        // An ordinary write failure is NOT re-labelled as a timeout.
        let failed = write_frame_bounded::<_, ()>(
            async { anyhow::bail!("stream reset by peer") },
            std::time::Duration::from_secs(30),
        )
        .await
        .expect_err("write failures propagate");
        assert!(failed.to_string().contains("stream reset by peer"));
        assert!(!failed.to_string().contains("timed out"));
    }

    #[test]
    fn request_defaults_to_the_hub_world() {
        let request: ReplicationRequest = serde_json::from_slice(
            br#"{"admin_token":"token","session_nonce":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]}"#,
        )
        .unwrap();
        assert_eq!(request.world, None);
        assert_eq!(request.admin_token, "token");
        assert_eq!(request.session_nonce, [1; 16]);
        let debug = format!("{request:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("\"token\""));
    }

    #[test]
    fn request_requires_an_admin_token() {
        let request = serde_json::from_slice::<ReplicationRequest>(
            br#"{"session_nonce":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]}"#,
        );
        assert!(request.is_err());
    }

    #[tokio::test]
    async fn default_world_requires_admin_before_replication_frames() {
        let dir = TempDir::new().unwrap();
        let (service, _) = open_world(
            dir.path(),
            "lobby",
            WorldLimits::default(),
            RuntimeLimits::default(),
            |world| WorldService::new(world, Some("test-admin".to_owned())),
        )
        .await
        .unwrap();
        let world = service.world().clone();
        let hub = WorldHub::single(service);
        let server_endpoint = endpoint().await;
        let server_addr = endpoint_addr(&server_endpoint);
        let router = Router::builder(server_endpoint.clone())
            .accept(
                REPLICATION_ALPN,
                WorldReplicationProtocol::new(hub, SigningKey::from_bytes(&[7; 32])),
            )
            .spawn();
        let client_endpoint = endpoint().await;

        // `world` is omitted so the request exercises the default-world path.
        // An invalid admin token must close the stream without a journal frame.
        let denied =
            request_frame(&client_endpoint, server_addr.clone(), "not-the-admin-token").await;
        assert!(
            denied.is_err() || denied.as_ref().is_ok_and(Option::is_none),
            "an unauthorized peer must receive no replication frame"
        );

        // The same default-world request still streams for the hub admin.
        let authorized = request_frame(&client_endpoint, server_addr, "test-admin")
            .await
            .unwrap();
        assert!(authorized.is_some(), "authorized replication sends a frame");

        let _ = router.shutdown().await;
        client_endpoint.close().await;
        server_endpoint.close().await;
        world.shutdown().await.unwrap();
    }
}
