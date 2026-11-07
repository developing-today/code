//! Pinned-source journal following with atomic, authenticated state generations.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use rand::RngExt as _;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::frames::{read_frame, write_frame};

use super::{
    DurableReceiver, REPLICA_STATUS_FILE, ReplicaRefreshPayload, ReplicaRefreshRecord,
    ReplicaStatus, ReceiverProgress,
    replica_control::{
        SignedReplicaRefreshChunk, SignedReplicaRefreshHeader,
    },
    replica_state::{
        create_working_generation, install_refresh, publish_generation,
    },
};

/// One request understood by the writer replication ALPN. Refresh requests
/// carry the exact cursor and account subjects; normal requests omit them.
#[derive(Serialize)]
struct FollowRequest<'a> {
    world: &'a str,
    session_nonce: [u8; 16],
    refresh_cursor: Option<super::ReplicationCursor>,
    subjects: Vec<String>,
}

/// Follow a pinned writer into a durable replica directory.
///
/// Journal batches and their module, directory-fact and admission-lease
/// refresh are assembled in an unpublished generation. Only after every
/// signature, hash, cursor, journal restore and lease check succeeds does an
/// atomic pointer switch publish that generation. The previous generation
/// remains available on disconnect, refresh failure or process restart.
///
/// # Errors
///
/// Returns an error for invalid pins, invalid signed journal frames, or local
/// storage failures. Transport interruptions reconnect without discarding the
/// last published generation.
pub async fn follow_replica(
    endpoint: &Endpoint,
    source: EndpointId,
    addrs: &[SocketAddr],
    world_id: &str,
    replica_root: &Path,
    once: bool,
) -> Result<()> {
    ensure!(!world_id.is_empty(), "world ID must not be empty");
    let source_bytes = *source.as_bytes();
    let addr = EndpointAddr::from_parts(source.clone(), addrs.iter().copied().map(TransportAddr::Ip));
    let world = world_id.to_owned();
    let root = replica_root.to_path_buf();
    let mut state_nonce = random_nonce();
    let mut working = create_working_generation(&root, &state_nonce)?;
    let mut receiver = open_receiver(&working, &world, source_bytes).await?;
    let mut refresh_tick = tokio::time::interval(Duration::from_secs(30));

    loop {
        let session_nonce = random_nonce();
        receiver.begin_session(session_nonce);
        let connection = match endpoint
            .connect(addr.clone(), crate::world_replication_net::REPLICATION_ALPN)
            .await
        {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!("replication source unavailable; retrying: {error:#}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        ensure!(
            connection.remote_id() == source,
            "replication endpoint identity does not match pinned source"
        );
        let (mut send, recv) = match connection.open_bi().await {
            Ok(streams) => streams,
            Err(error) => {
                tracing::warn!("could not open replication stream; retrying: {error:#}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let request = FollowRequest {
            world: &world,
            session_nonce,
            refresh_cursor: None,
            subjects: Vec::new(),
        };
        write_frame(&mut send, &serde_json::to_vec(&request)?)
            .await
            .context("write replication request")?;
        send.finish().context("finish replication request")?;

        let (frame_tx, mut frame_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let mut recv = recv;
            loop {
                let result = read_frame(&mut recv).await;
                let finished = matches!(result, Ok(None) | Err(_));
                if frame_tx.send(result).await.is_err() || finished {
                    break;
                }
            }
        });

        let mut interrupted = false;
        loop {
            tokio::select! {
                message = frame_rx.recv() => {
                    match message {
                        Some(Ok(Some(frame))) => {
                            if let ReceiverProgress::Committed { cursor } = receiver.accept_frame(&frame)? {
                                let status = ReplicaStatus::now(source_bytes, world.clone(), cursor);
                                let dir = working.clone();
                                tokio::task::spawn_blocking(move || status.write(&dir))
                                    .await
                                    .context("write replica status task")??;
                                if publish_verified_refresh(
                                    endpoint,
                                    &addr,
                                    source,
                                    source_bytes,
                                    &world,
                                    &root,
                                    &mut working,
                                    &mut receiver,
                                    &mut state_nonce,
                                    cursor,
                                ).await? {
                                    let source_text = crate::world::hex_encode(&source_bytes);
                                    eprintln!(
                                        "replica verified source={source_text} world={world} cursor={}:{} staleness=0s status={}",
                                        cursor.ordinal,
                                        crate::world::hex_encode(&cursor.digest),
                                        active_status_path(&root)?.display(),
                                    );
                                    if once {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        Some(Ok(None)) | None => {
                            interrupted = true;
                            break;
                        }
                        Some(Err(error)) => {
                            tracing::warn!("replication stream interrupted; reconnecting: {error:#}");
                            interrupted = true;
                            break;
                        }
                    }
                }
                _ = refresh_tick.tick() => {
                    if let Some(cursor) = receiver.committed_cursor() {
                        if publish_verified_refresh(
                            endpoint,
                            &addr,
                            source,
                            source_bytes,
                            &world,
                            &root,
                            &mut working,
                            &mut receiver,
                            &mut state_nonce,
                            cursor,
                        ).await? {
                            let source_text = crate::world::hex_encode(&source_bytes);
                            eprintln!(
                                "replica refreshed source={source_text} world={world} cursor={}:{} staleness=0s status={}",
                                cursor.ordinal,
                                crate::world::hex_encode(&cursor.digest),
                                active_status_path(&root)?.display(),
                            );
                            if once {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        if interrupted {
            receiver.abort_session();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

async fn open_receiver(
    dir: &Path,
    world: &str,
    source: [u8; 32],
) -> Result<DurableReceiver> {
    let dir = dir.to_path_buf();
    let world = world.to_owned();
    tokio::task::spawn_blocking(move || DurableReceiver::open(&dir, &world, source))
        .await
        .context("open durable replica receiver task")?
        .map_err(Into::into)
}

async fn publish_verified_refresh(
    endpoint: &Endpoint,
    addr: &EndpointAddr,
    source: EndpointId,
    source_bytes: [u8; 32],
    world: &str,
    root: &Path,
    working: &mut PathBuf,
    receiver: &mut DurableReceiver,
    state_nonce: &mut [u8; 16],
    cursor: super::ReplicationCursor,
) -> Result<bool> {
    let journal = receiver.read_journal()?;
    let entries = super::parse_replica_journal(&journal, world)?;
    let subjects = crate::world_store::replica_subjects(&entries)
        .into_iter()
        .collect::<Vec<_>>();
    let refresh = match request_refresh(
        endpoint,
        addr,
        source,
        source_bytes,
        world,
        cursor,
        subjects,
    )
    .await
    {
        Ok(refresh) => refresh,
        Err(error) => {
            tracing::debug!("replica refresh not ready at cursor {}: {error:#}", cursor.ordinal);
            return Ok(false);
        }
    };
    let record = ReplicaRefreshRecord {
        header: refresh.0,
        payload_bytes: refresh.1,
    };
    let dir = working.clone();
    let world_owned = world.to_owned();
    let record_for_install = record.clone();
    let payload = tokio::task::spawn_blocking(move || {
        super::install_refresh(&dir, source_bytes, &world_owned, cursor, &record_for_install)
    })
    .await
    .context("install signed replica refresh task")??;

    let state_dir = working.clone();
    let (verified_service, report, verified_cursor) = crate::world_store::open_replica_for_serving(
        &state_dir,
        world,
        source_bytes,
        crate::world::WorldLimits::default(),
        crate::world_limits::RuntimeLimits::default(),
        |handle| crate::world_session::WorldService::new(handle, None),
    )
    .await?;
    ensure!(verified_cursor.sequence == report.sequence, "replica restore cursor mismatch");
    ensure!(report.module_hash == payload.module_hash, "refreshed module does not match journal");
    ensure!(report.program_error.is_none(), "refreshed program is unavailable: {}", report.program_error.as_deref().unwrap_or("unknown"));
    verified_service.world().shutdown().await?;

    crate::world::publish_generation(root, working, cursor.ordinal, state_nonce)?;
    *state_nonce = random_nonce();
    let next_working = crate::world::create_working_generation(root, state_nonce)?;
    receiver.relocate(&next_working)?;
    *working = next_working;
    Ok(true)
}

async fn request_refresh(
    endpoint: &Endpoint,
    addr: &EndpointAddr,
    source: EndpointId,
    source_bytes: [u8; 32],
    world: &str,
    cursor: super::ReplicationCursor,
    subjects: Vec<String>,
) -> Result<(SignedReplicaRefreshHeader, Vec<u8>)> {
    let nonce = random_nonce();
    let connection = endpoint
        .connect(addr.clone(), crate::world_replication_net::REPLICATION_ALPN)
        .await
        .context("connect for replica refresh")?;
    ensure!(connection.remote_id() == source, "refresh endpoint does not match pinned source");
    let (mut send, mut recv) = connection.open_bi().await.context("open refresh stream")?;
    let request = FollowRequest {
        world,
        session_nonce: nonce,
        refresh_cursor: Some(cursor),
        subjects,
    };
    write_frame(&mut send, &serde_json::to_vec(&request)?)
        .await
        .context("write replica refresh request")?;
    send.finish().context("finish replica refresh request")?;
    let header_bytes = read_frame(&mut recv)
        .await?
        .context("writer closed before refresh header")?;
    let header: SignedReplicaRefreshHeader =
        postcard::from_bytes(&header_bytes).context("decode replica refresh header")?;
    header.verify(source_bytes, world, nonce, cursor)?;
    let mut payload_bytes = Vec::with_capacity(
        usize::try_from(header.body.payload_len).context("refresh payload length")?,
    );
    for index in 0..header.body.chunk_count {
        let bytes = read_frame(&mut recv)
            .await?
            .context("writer closed during replica refresh")?;
        let chunk: SignedReplicaRefreshChunk =
            postcard::from_bytes(&bytes).context("decode replica refresh chunk")?;
        chunk.verify(
            source_bytes,
            world,
            nonce,
            cursor,
            index,
            header.body.chunk_count,
        )?;
        payload_bytes.extend_from_slice(&chunk.body.payload);
    }
    ensure!(
        payload_bytes.len() == usize::try_from(header.body.payload_len)?,
        "replica refresh payload length mismatch"
    );
    ensure!(
        blake3::hash(&payload_bytes).as_bytes() == &header.body.payload_digest,
        "replica refresh payload digest mismatch"
    );
    let payload: ReplicaRefreshPayload =
        postcard::from_bytes(&payload_bytes).context("decode replica refresh payload")?;
    payload
        .admission_lease
        .verify_now(source_bytes, world, cursor)?;
    Ok((header, payload_bytes))
}

fn random_nonce() -> [u8; 16] {
    let mut nonce = [0; 16];
    rand::rng().fill(&mut nonce);
    if nonce.iter().all(|byte| *byte == 0) {
        nonce[0] = 1;
    }
    nonce
}

fn active_status_path(root: &Path) -> Result<PathBuf> {
    Ok(crate::world::active_replica_dir(root)?.join(REPLICA_STATUS_FILE))
}
