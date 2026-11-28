//! Live two-holder PoC for #2261: drive the swarm [`Scheduler`] with real
//! iroh-blobs range fetches over in-process endpoints.
//!
//! Review-gated experimental PoC only (the #2261 Adapt decision): the
//! default `get` path is untouched. These tests prove the scheduler →
//! transport-outcome mapping against live servers on loopback:
//! - `two_holders_split_ranges_live`: the fetcher starts with an empty
//!   store (zero local copy) and assembles the blob purely from
//!   scheduler-assigned `(holder, ChunkRanges)` requests; per-range
//!   provenance must name both holders and the bytes must match.
//! - `holder_leaving_mid_fetch_live`: one holder dies mid-fetch; its
//!   in-flight ranges are requeued (`Missing`) and the fetch still
//!   completes from the survivor.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use bao_tree::ChunkNum;
use id_core::holders::HeldRanges;
use id_core::swarm::{ByteRange, Scheduler, CHUNK_GROUP_BYTES};
use id_core::swarm_fetch::{FetchOutcome, byte_range_to_chunks, drive_with_accounting};
use iroh::{Endpoint, EndpointAddr, endpoint::{RelayMode, presets}, protocol::Router};
use iroh_base::SecretKey;
use iroh_blobs::{
    ALPN as BLOBS_ALPN, BlobFormat, BlobsProtocol, Hash,
    api::{Store, TempTag, blobs::AddBytesOptions},
    get::GetError,
    protocol::{ChunkRanges, GetRequest},
    store::mem::MemStore,
};

/// Groups per test blob: 8 groups = 128 KiB.
const GROUPS: u64 = 8;

fn blob_bytes() -> Vec<u8> {
    let n = (GROUPS * CHUNK_GROUP_BYTES) as usize;
    (0..n).map(|i| (i % 251) as u8).collect()
}

struct Node {
    endpoint: Endpoint,
    store: Store,
    _tag: Option<TempTag>,
    router: Option<Router>,
    addr: EndpointAddr,
}

async fn node(seed: u8, serve: Option<&[u8]>) -> (Node, Option<Hash>) {
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(SecretKey::from_bytes(&[seed; 32]))
        .bind()
        .await
        .unwrap();
    let addr = endpoint.addr();
    let store: Store = MemStore::new().into();
    let mut tag = None;
    let mut hash = None;
    if let Some(bytes) = serve {
        let guard = store
            .add_bytes_with_opts(AddBytesOptions {
                data: bytes.to_vec().into(),
                format: BlobFormat::Raw,
            })
            .temp_tag()
            .await
            .unwrap();
        hash = Some(guard.hash());
        tag = Some(guard);
    }
    let router = Router::builder(endpoint.clone())
        .accept(BLOBS_ALPN, BlobsProtocol::new(&store, None))
        .spawn();
    (
        Node {
            endpoint,
            store,
            _tag: tag,
            router: Some(router),
            addr,
        },
        hash,
    )
}

fn chunks_for(range: ByteRange) -> ChunkRanges {
    let (first, last) = byte_range_to_chunks(range);
    ChunkRanges::from(ChunkNum(first)..ChunkNum(last))
}

/// One live range fetch: connect, `execute_get` the scheduler-assigned
/// chunk ranges, map the transport outcome. Mirrors the heuristic
/// documented in [`id_core::swarm_fetch`]: decode/verification failure
/// is a bad group from this holder, anything else is unavailability.
async fn live_fetch(
    endpoint: &Endpoint,
    store: &Store,
    addr: &EndpointAddr,
    hash: Hash,
    range: ByteRange,
) -> (FetchOutcome, u64) {
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        let conn = endpoint
            .connect(addr.clone(), BLOBS_ALPN)
            .await
            .map_err(|_| FetchOutcome::Unavailable)?;
        let stats = store
            .remote()
            .execute_get(conn, GetRequest::blob_ranges(hash, chunks_for(range)))
            .await
            .map_err(|e| match e {
                GetError::Decode { .. } => FetchOutcome::BadGroup,
                _ => FetchOutcome::Unavailable,
            })?;
        Ok::<u64, FetchOutcome>(stats.payload_bytes_read)
    })
    .await;
    match outcome {
        Ok(Ok(bytes)) => (FetchOutcome::Verified, bytes),
        Ok(Err(mapped)) => (mapped, 0),
        Err(_) => (FetchOutcome::Unavailable, 0),
    }
}

async fn read_all(store: &Store, hash: Hash) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut reader = store.reader(hash);
    let mut out = Vec::new();
    reader.read_to_end(&mut out).await.unwrap();
    out
}

fn holder_names(provenance: &[String]) -> std::collections::BTreeSet<String> {
    provenance
        .iter()
        .filter_map(|line| line.rsplit(" <- holder ").next().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn two_holders_split_ranges_live() {
    let bytes = blob_bytes();
    let blob_size = bytes.len() as u64;
    let (holder_a, _) = node(11, Some(&bytes)).await;
    let (holder_b, hash_b) = node(22, Some(&bytes)).await;
    let hash = hash_b.unwrap();
    let (fetcher, _) = node(33, None).await;
    let addrs = vec![holder_a.addr.clone(), holder_b.addr.clone()];

    let started = Instant::now();
    let mut sched = Scheduler::new(
        blob_size,
        vec![
            ("a".to_string(), HeldRanges::Complete),
            ("b".to_string(), HeldRanges::Complete),
        ],
    );
    let (stats, provenance) = drive_with_accounting(&mut sched, blob_size, |h, r| {
        let (endpoint, store, addr) = (
            fetcher.endpoint.clone(),
            fetcher.store.clone(),
            addrs[h].clone(),
        );
        async move { live_fetch(&endpoint, &store, &addr, hash, r).await }
    })
    .await;
    let wall = started.elapsed();

    assert!(stats.complete, "fetch must complete, provenance={provenance:?}");
    assert_eq!(read_all(&fetcher.store, hash).await, bytes);
    let holders = holder_names(&provenance);
    assert!(
        holders.len() >= 2,
        "both holders must serve ranges, provenance={provenance:?}"
    );
    eprintln!(
        "live 2-holder matrix: groups={GROUPS} waves={} wall={wall:?} duplicated_bytes={} provenance_holders={}",
        stats.waves,
        stats.duplicated_bytes,
        holders.len(),
    );
}

#[tokio::test]
async fn holder_leaving_mid_fetch_live() {
    let bytes = blob_bytes();
    let blob_size = bytes.len() as u64;
    let (holder_a, _) = node(44, Some(&bytes)).await;
    let (mut holder_b, hash_b) = node(55, Some(&bytes)).await;
    let hash = hash_b.unwrap();
    let (fetcher, _) = node(66, None).await;
    let addrs = vec![holder_a.addr.clone(), holder_b.addr.clone()];

    let mut sched = Scheduler::new(
        blob_size,
        vec![
            ("a".to_string(), HeldRanges::Complete),
            ("b".to_string(), HeldRanges::Complete),
        ],
    );

    // Partial manual wave while both holders are alive: fetch all of
    // holder A's ranges plus the first range of holder B, so B has
    // genuinely served bytes before it dies and remaining work is left
    // for the driver. Every fetch here is a real `execute_get`.
    let first = sched.assignments();
    assert!(!first.is_empty());
    let mut b_served_first = false;
    for a in &first {
        for (i, r) in a.ranges.iter().enumerate() {
            if a.holder == 1 && i > 0 {
                continue; // leave the rest of B's ranges for the driver
            }
            let (outcome, _) = live_fetch(
                &fetcher.endpoint,
                &fetcher.store,
                &addrs[a.holder],
                hash,
                *r,
            )
            .await;
            assert_eq!(
                outcome,
                FetchOutcome::Verified,
                "pre-kill fetch must succeed"
            );
            if a.holder == 1 {
                b_served_first = true;
            }
            sched.apply(id_core::swarm::SchedEvent::Verified {
                holder: a.holder,
                range: *r,
            });
        }
    }
    assert!(b_served_first, "B must serve a range before dying");

    // Kill holder B mid-fetch: stop serving and close its endpoint. The
    // driver's subsequent fetches to B are real `execute_get` attempts
    // that fail (bounded by the live_fetch timeout) and are requeued as
    // `Missing` onto survivor A via the Complete->Ranges downgrade.
    if let Some(router) = holder_b.router.take() {
        router.shutdown().await.unwrap();
    }
    holder_b.endpoint.close().await;

    let started = Instant::now();
    let (stats, provenance) = drive_with_accounting(&mut sched, blob_size, |h, r| {
        let (endpoint, store, addr) = (
            fetcher.endpoint.clone(),
            fetcher.store.clone(),
            addrs[h].clone(),
        );
        async move { live_fetch(&endpoint, &store, &addr, hash, r).await }
    })
    .await;
    let wall = started.elapsed();

    assert!(stats.complete, "fetch must survive the leaver, provenance={provenance:?}");
    assert_eq!(read_all(&fetcher.store, hash).await, bytes);
    let holders = holder_names(&provenance);
    assert!(
        holders.len() >= 2,
        "leaver must have served ranges before dying, provenance={provenance:?}"
    );
    eprintln!(
        "live holder-leaving matrix: groups={GROUPS} waves={} wall={wall:?} duplicated_bytes={} provenance_holders={}",
        stats.waves,
        stats.duplicated_bytes,
        holders.len(),
    );
}
