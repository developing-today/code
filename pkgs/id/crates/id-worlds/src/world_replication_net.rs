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
    EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::{
    frames::{read_frame, write_frame},
    world::{
        DurableReceiver, JournalSnapshot, MAX_REPLICATION_PAYLOAD_BYTES, ReceiverProgress,
        ReplicationCursor, ReplicationEnvelope, ReplicationUpdate, SignedReplicationEnvelope,
        chain_base_cursor, frame_digest, split_entry_payload,
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
        let sidecars = gather_sidecars(&lease, &snapshot.bytes).await?;
        send_snapshot(
            &mut send,
            signing_key,
            &world_id,
            source,
            request.session_nonce,
            snapshot,
            &sidecars,
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
                let sidecars = gather_sidecars(&lease, &reset.bytes).await?;
                let (after, version, digest) = send_snapshot(
                    &mut send,
                    signing_key,
                    &world_id,
                    source,
                    request.session_nonce,
                    reset,
                    &sidecars,
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

/// The authenticated sidecars served inside one snapshot batch
/// (F12.13 phase 5): the directory facts for the snapshot journal's
/// subjects, and the active module's bytes when the writer's module
/// store holds them. Refreshed on every snapshot — the initial sync
/// and every compaction/header reset — which is the replication
/// refresh cadence for facts the journal itself does not carry.
struct SnapshotSidecars {
    directory: Vec<u8>,
    module: Option<Vec<u8>>,
}

/// Capture the sidecars for one snapshot image: subjects are the
/// accounts appearing in the snapshot's capability chains, the facts
/// come from the actor in one round trip immediately after the
/// snapshot image was captured, and the module bytes are loaded from
/// the service's content-addressed store under the actor's active
/// module hash — a load failure aborts the stream rather than serving
/// a journal whose program the replica could never restore.
async fn gather_sidecars(
    lease: &crate::world_hub::WorldLease,
    snapshot_bytes: &[u8],
) -> Result<SnapshotSidecars> {
    let mut subjects = std::collections::BTreeSet::new();
    for line in snapshot_bytes.split_inclusive(|byte| *byte == b'\n') {
        if let Ok(crate::world::JournalEntry::Issued {
            subject: Some(subject),
            ..
        }) = serde_json::from_slice::<crate::world::JournalEntry>(line)
        {
            subjects.insert(subject);
        }
    }
    let service = lease.service();
    let (directory, active_hash) = service.world().replica_sidecars(subjects).await?;
    let module = match (active_hash, service.module_dir.as_ref()) {
        (Some(hash), Some(dir)) => Some(
            dir.load(&hash)
                .context("load active module for replication")?,
        ),
        _ => None,
    };
    Ok(SnapshotSidecars { directory, module })
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
    sidecars: &SnapshotSidecars,
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
    // The sidecars ride inside the batch, after the journal reaches
    // its cursor and before the terminal commits it: the facts frame
    // is always sent (an empty one replaces the replica's facts with
    // none), the module only when the writer holds the active bytes.
    let directory_chunks = if sidecars.directory.is_empty() {
        vec![Vec::new()]
    } else {
        split_entry_payload(&sidecars.directory).context("split directory sidecar")?
    };
    for chunk in directory_chunks {
        let envelope = ReplicationEnvelope::directory(
            world_id,
            source,
            snapshot.header_version,
            nonce,
            cursor,
            prev_frame_digest,
            chunk,
        );
        prev_frame_digest = write_envelope(send, signing_key, envelope).await?;
    }
    if let Some(module) = &sidecars.module {
        for chunk in module.chunks(MAX_REPLICATION_PAYLOAD_BYTES) {
            let envelope = ReplicationEnvelope::module(
                world_id,
                source,
                snapshot.header_version,
                nonce,
                cursor,
                prev_frame_digest,
                chunk.to_vec(),
            );
            prev_frame_digest = write_envelope(send, signing_key, envelope).await?;
        }
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

/// Longest silence tolerated on a replication session before the
/// client tears it down and (in follow mode) reconnects: the server
/// bounds its own frame writes at the same 30 s (#4726), so a silent
/// stream for longer than this is a dead writer, not a quiet one.
pub const REPLICA_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A fresh, non-zero session nonce for one replication session. The
/// server binds every signed frame of the session to this nonce, and
/// the receiver refuses frames minted for any other session.
fn fresh_session_nonce() -> [u8; 16] {
    use rand::RngExt as _;
    loop {
        let nonce: [u8; 16] = rand::rng().random();
        if nonce.iter().any(|byte| *byte != 0) {
            return nonce;
        }
    }
}

/// One replication session against a writer, pumped into `receiver`.
///
/// The session opens with the writer's snapshot (every session does;
/// the receiver rebases on it), then tails live appends. Each frame is
/// verified and staged by the receiver; a batch becomes durable only
/// at its terminal frame, which is when `on_commit` fires.
///
/// With `stop_after_first_commit` the session ends (successfully) at
/// the snapshot's terminal frame — the sync-once posture. Otherwise it
/// runs until the writer closes the stream or a frame goes missing
/// for [`REPLICA_FRAME_TIMEOUT`]; the number of commits absorbed is
/// returned, and a stream that ends before ANY commit is an error
/// (nothing was synced). On any failure the session is aborted, so
/// staged-but-uncommitted bytes never leak into the next session.
async fn pump_replication_session(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    world: Option<&str>,
    admin_token: &str,
    receiver: &mut DurableReceiver,
    stop_after_first_commit: bool,
    on_commit: &mut dyn FnMut(ReplicationCursor),
) -> Result<usize> {
    let nonce = fresh_session_nonce();
    receiver.begin_session(nonce);
    let outcome = pump_replication_session_inner(
        endpoint,
        addr,
        world,
        admin_token,
        nonce,
        receiver,
        stop_after_first_commit,
        on_commit,
    )
    .await;
    if outcome.is_err() {
        receiver.abort_session();
    }
    outcome
}

async fn pump_replication_session_inner(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    world: Option<&str>,
    admin_token: &str,
    nonce: [u8; 16],
    receiver: &mut DurableReceiver,
    stop_after_first_commit: bool,
    on_commit: &mut dyn FnMut(ReplicationCursor),
) -> Result<usize> {
    let connection = endpoint.connect(addr, REPLICATION_ALPN).await?;
    let (mut send, mut recv) = connection.open_bi().await?;
    let request = serde_json::json!({
        "world": world,
        "admin_token": admin_token,
        "session_nonce": nonce,
    });
    write_frame(&mut send, &serde_json::to_vec(&request)?).await?;
    send.finish()?;
    let mut commits = 0_usize;
    loop {
        let frame = tokio::time::timeout(REPLICA_FRAME_TIMEOUT, read_frame(&mut recv))
            .await
            .context("timed out waiting for a replication frame")??
            .context("replication stream closed by the writer")?;
        match receiver.accept_frame(&frame)? {
            ReceiverProgress::Staged { .. } => {}
            ReceiverProgress::Committed { cursor } => {
                commits += 1;
                on_commit(cursor);
                if stop_after_first_commit {
                    connection.close(0_u32.into(), b"replica synced");
                    return Ok(commits);
                }
            }
        }
    }
}

/// Sync a durable replica once from its pinned writer (F12.13 phase
/// 5, operator client): open one replication session, absorb the
/// writer's snapshot into `receiver`, and return the verified chain
/// cursor the replica now stands at. The receiver's journal on disk
/// is the durable result; open it with
/// [`crate::world_store::open_replica_with_admission`] to serve it.
///
/// # Errors
///
/// Fails when the writer is unreachable, refuses the admin token,
/// streams a frame the receiver cannot verify against the pinned
/// source, or closes before the snapshot commits — in every failure
/// case the receiver is left at its previous committed cursor.
pub async fn sync_replica_once(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    world: Option<&str>,
    admin_token: &str,
    receiver: &mut DurableReceiver,
) -> Result<ReplicationCursor> {
    let mut synced = None;
    pump_replication_session(
        endpoint,
        addr,
        world,
        admin_token,
        receiver,
        true,
        &mut |cursor| synced = Some(cursor),
    )
    .await?;
    synced.context("replication session committed no snapshot")
}

/// Follow a writer across one replication session (F12.13 phase 5):
/// absorb the snapshot, then keep absorbing live tail batches until
/// the stream ends, invoking `on_commit` at every durable cursor.
/// Returns the number of commits absorbed in this session. Callers
/// who want to keep following reconnect on return (every new session
/// re-snapshots, so a reconnect can never lose a tail batch).
///
/// # Errors
///
/// As [`sync_replica_once`], except that a session which committed at
/// least one batch before ending returns its commit count instead of
/// the stream error.
pub async fn follow_replica_session(
    endpoint: &iroh::Endpoint,
    addr: EndpointAddr,
    world: Option<&str>,
    admin_token: &str,
    receiver: &mut DurableReceiver,
    on_commit: &mut dyn FnMut(ReplicationCursor),
) -> Result<usize> {
    let mut commits = 0_usize;
    let outcome = pump_replication_session(
        endpoint,
        addr,
        world,
        admin_token,
        receiver,
        false,
        &mut |cursor| {
            commits += 1;
            on_commit(cursor);
        },
    )
    .await;
    match outcome {
        Ok(count) => Ok(count),
        Err(error) => {
            if commits > 0 {
                Ok(commits)
            } else {
                Err(error)
            }
        }
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

    // Oracle: acceptance item 1 of #806 phase 5 — a pinned-source
    // client session absorbs the writer's snapshot into a durable
    // receiver, and a refused session leaves the committed cursor
    // exactly where it stood.
    #[tokio::test]
    async fn sync_replica_once_absorbs_the_writer_snapshot() {
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
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let source = *signing_key.verifying_key().as_bytes();
        let server_endpoint = endpoint().await;
        let server_addr = endpoint_addr(&server_endpoint);
        let router = Router::builder(server_endpoint.clone())
            .accept(
                REPLICATION_ALPN,
                WorldReplicationProtocol::new(hub, signing_key),
            )
            .spawn();
        let client_endpoint = endpoint().await;
        let replica_dir = TempDir::new().unwrap();
        let mut receiver = DurableReceiver::open(replica_dir.path(), "lobby", source).unwrap();
        assert_eq!(receiver.committed_cursor(), None);

        let cursor = sync_replica_once(
            &client_endpoint,
            server_addr.clone(),
            None,
            "test-admin",
            &mut receiver,
        )
        .await
        .unwrap();
        assert_eq!(receiver.committed_cursor(), Some(cursor));
        assert!(
            cursor.ordinal >= 1,
            "the snapshot absorbs at least the Created header"
        );
        assert!(!receiver.read_journal().unwrap().is_empty());

        // A refused session (wrong admin token) commits nothing and
        // the previously synced cursor stands.
        let denied = sync_replica_once(
            &client_endpoint,
            server_addr,
            None,
            "not-the-admin-token",
            &mut receiver,
        )
        .await;
        assert!(denied.is_err());
        assert_eq!(receiver.committed_cursor(), Some(cursor));

        drop(receiver);
        let _ = router.shutdown().await;
        client_endpoint.close().await;
        server_endpoint.close().await;
        world.shutdown().await.unwrap();
    }

    struct Writer {
        world: crate::world::WorldHandle,
        router: Router,
        endpoint: Endpoint,
        addr: EndpointAddr,
    }

    async fn start_writer(dir: &std::path::Path) -> Writer {
        let (service, _) = open_world(
            dir,
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
        let addr = endpoint_addr(&server_endpoint);
        let router = Router::builder(server_endpoint.clone())
            .accept(
                REPLICATION_ALPN,
                WorldReplicationProtocol::new(hub, SigningKey::from_bytes(&[7; 32])),
            )
            .spawn();
        Writer {
            world,
            router,
            endpoint: server_endpoint,
            addr,
        }
    }

    async fn stop_writer(writer: Writer) {
        let _ = writer.router.shutdown().await;
        writer.endpoint.close().await;
        writer.world.shutdown().await.unwrap();
    }

    // Oracle: #806 phase 5 acceptance item 4 — a writer outage on the
    // durable path loses nothing: a sync against the dead writer does
    // not succeed and leaves the committed cursor and journal bytes
    // untouched, and the restarted writer (same directory, same
    // pinned source) resyncs the replica in place to the identical
    // verified cursor with no journal drift.
    #[tokio::test]
    async fn writer_outage_and_restart_resync_without_journal_drift() {
        let writer_dir = TempDir::new().unwrap();
        let writer = start_writer(writer_dir.path()).await;
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let source = *signing_key.verifying_key().as_bytes();
        let client_endpoint = endpoint().await;
        let replica_dir = TempDir::new().unwrap();
        let mut receiver = DurableReceiver::open(replica_dir.path(), "lobby", source).unwrap();

        let cursor = sync_replica_once(
            &client_endpoint,
            writer.addr.clone(),
            None,
            "test-admin",
            &mut receiver,
        )
        .await
        .unwrap();
        let journal = receiver.read_journal().unwrap();
        assert!(!journal.is_empty());

        // Outage: the writer goes away entirely.
        let dead_addr = writer.addr.clone();
        stop_writer(writer).await;
        let outage = tokio::time::timeout(
            Duration::from_secs(20),
            sync_replica_once(
                &client_endpoint,
                dead_addr,
                None,
                "test-admin",
                &mut receiver,
            ),
        )
        .await;
        assert!(
            !matches!(outage, Ok(Ok(_))),
            "a sync against a dead writer must not succeed"
        );
        assert_eq!(receiver.committed_cursor(), Some(cursor));
        assert_eq!(
            receiver.read_journal().unwrap(),
            journal,
            "an outage must not move the committed journal"
        );

        // Restart: same directory, same signing key — the replica
        // resyncs in place to the identical cursor, byte for byte.
        let writer = start_writer(writer_dir.path()).await;
        let resynced = sync_replica_once(
            &client_endpoint,
            writer.addr.clone(),
            None,
            "test-admin",
            &mut receiver,
        )
        .await
        .unwrap();
        assert_eq!(resynced, cursor);
        assert_eq!(receiver.read_journal().unwrap(), journal);

        drop(receiver);
        stop_writer(writer).await;
        client_endpoint.close().await;
    }

    // Oracle: #806 phase 5 acceptance item 4 — a replica restart
    // reopens the durable state exactly where the sync left it, and
    // resyncing an already-current replica is idempotent (the
    // verified cursor and journal bytes do not drift).
    #[tokio::test]
    async fn replica_restart_reopens_synced_state_and_resync_is_idempotent() {
        let writer_dir = TempDir::new().unwrap();
        let writer = start_writer(writer_dir.path()).await;
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let source = *signing_key.verifying_key().as_bytes();
        let client_endpoint = endpoint().await;
        let replica_dir = TempDir::new().unwrap();
        let mut receiver = DurableReceiver::open(replica_dir.path(), "lobby", source).unwrap();

        let cursor = sync_replica_once(
            &client_endpoint,
            writer.addr.clone(),
            None,
            "test-admin",
            &mut receiver,
        )
        .await
        .unwrap();
        let journal = receiver.read_journal().unwrap();

        // Replica restart: the durable receiver reopens at the
        // committed cursor with the journal intact.
        drop(receiver);
        let mut receiver = DurableReceiver::open(replica_dir.path(), "lobby", source).unwrap();
        assert_eq!(receiver.committed_cursor(), Some(cursor));
        assert_eq!(receiver.read_journal().unwrap(), journal);

        // Resync against the same writer changes nothing.
        let again = sync_replica_once(
            &client_endpoint,
            writer.addr.clone(),
            None,
            "test-admin",
            &mut receiver,
        )
        .await
        .unwrap();
        assert_eq!(again, cursor);
        assert_eq!(receiver.read_journal().unwrap(), journal);

        drop(receiver);
        stop_writer(writer).await;
        client_endpoint.close().await;
    }

    async fn stop_writer_active(writer: Writer) {
        // Endpoint first, so an in-flight session's stream closes
        // instead of lingering to the frame timeout.
        writer.endpoint.close().await;
        writer.world.shutdown().await.unwrap();
        let _ = writer.router.shutdown().await;
    }

    // Oracle: #806 phase 5 acceptance item 4 — the follow path
    // reconnects: a follow session whose writer dies ends with the
    // commits it already absorbed (not an error, not a hang), and a
    // fresh follow session against the restarted writer (the CLI
    // reconnect loop's posture) catches the replica up to the same
    // verified cursor with no journal drift.
    #[tokio::test]
    async fn follow_session_reconnects_after_a_writer_restart() {
        let writer_dir = TempDir::new().unwrap();
        let writer = start_writer(writer_dir.path()).await;
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let source = *signing_key.verifying_key().as_bytes();
        let client_endpoint = endpoint().await;
        let replica_dir = TempDir::new().unwrap();
        let mut receiver = DurableReceiver::open(replica_dir.path(), "lobby", source).unwrap();

        // First follow session: absorb the snapshot commit, then the
        // writer dies mid-session. The follow future and the
        // writer-killer future run on this one task (the on_commit
        // callback is &mut dyn FnMut and not Send, so no spawn).
        let (synced, commits) = {
            let (commits_tx, mut commits_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut on_commit = |cursor| {
                let _ = commits_tx.send(cursor);
            };
            let mut follow = Box::pin(follow_replica_session(
                &client_endpoint,
                writer.addr.clone(),
                None,
                "test-admin",
                &mut receiver,
                &mut on_commit,
            ));
            let mut pending_writer = Some(writer);
            let mut killer = Box::pin(async move {
                let first = commits_rx.recv().await;
                if let Some(w) = pending_writer.take() {
                    stop_writer_active(w).await;
                }
                first
            });
            let mut first = None;
            let outcome = tokio::time::timeout(Duration::from_secs(60), async {
                let mut follow_out = None;
                loop {
                    if follow_out.is_some() && first.is_some() {
                        break follow_out.take().expect("follow outcome recorded");
                    }
                    tokio::select! {
                        out = &mut follow, if follow_out.is_none() => {
                            follow_out = Some(out);
                        }
                        done = &mut killer, if first.is_none() => {
                            first = Some(done);
                        }
                    }
                }
            })
            .await
            .expect("the follow session must end when its writer dies");
            let synced = first
                .flatten()
                .expect("the killer must observe the snapshot commit first");
            let commits = outcome.expect("a follow session with commits ends Ok");
            (synced, commits)
        };
        assert!(commits >= 1, "the snapshot commit was absorbed");
        assert_eq!(receiver.committed_cursor(), Some(synced));
        let journal = receiver.read_journal().unwrap();
        assert!(!journal.is_empty());

        // Reconnect: the restarted writer (same directory, same
        // pinned source) serves a fresh follow session that catches
        // the replica up to the same verified cursor, byte for byte.
        let writer = start_writer(writer_dir.path()).await;
        let commits = {
            let (commits_tx, mut commits_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut on_commit = |cursor| {
                let _ = commits_tx.send(cursor);
            };
            let mut follow = Box::pin(follow_replica_session(
                &client_endpoint,
                writer.addr.clone(),
                None,
                "test-admin",
                &mut receiver,
                &mut on_commit,
            ));
            let mut pending_writer = Some(writer);
            let mut killer = Box::pin(async move {
                let first = commits_rx.recv().await;
                if let Some(w) = pending_writer.take() {
                    stop_writer_active(w).await;
                }
                first
            });
            let mut first = None;
            let outcome = tokio::time::timeout(Duration::from_secs(60), async {
                let mut follow_out = None;
                loop {
                    if follow_out.is_some() && first.is_some() {
                        break follow_out.take().expect("follow outcome recorded");
                    }
                    tokio::select! {
                        out = &mut follow, if follow_out.is_none() => {
                            follow_out = Some(out);
                        }
                        done = &mut killer, if first.is_none() => {
                            first = Some(done);
                        }
                    }
                }
            })
            .await
            .expect("the reconnected follow session must end with its writer");
            let caught_up = first
                .flatten()
                .expect("the killer must observe the catch-up commit first");
            assert_eq!(caught_up, synced);
            outcome.expect("the reconnected follow session ends Ok")
        };
        assert!(commits >= 1, "the catch-up commit was absorbed");
        assert_eq!(receiver.committed_cursor(), Some(synced));
        assert_eq!(
            receiver.read_journal().unwrap(),
            journal,
            "a reconnect must not drift the committed journal"
        );

        drop(receiver);
        client_endpoint.close().await;
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
