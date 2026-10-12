//! End-to-end tests of blob fetch and observe gating over real QUIC on loopback.
//!
//! A provider serves blobs through `BlobsProtocol` with the gate from `id::blob_gate`. Requesters
//! connect directly (no relay, no discovery). The gate reads a fixed world graph: which worlds
//! name which blob, and who is a current member of each world.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{future::IntoFuture, sync::Arc, time::Duration};

use futures_lite::StreamExt;
use id::{
    access::AccessPolicy,
    blob_gate::{BlobPolicy, WorldAccess, blob_events},
    commands::put::add_raw,
};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use iroh_base::{EndpointId, SecretKey};
use iroh_blobs::{
    BlobsProtocol, Hash,
    api::{Store, TempTag},
    get::request::get_blob,
    protocol::{GetManyRequest, ObserveRequest},
    store::mem::MemStore,
};

/// Every network step must finish within this, so a stuck request fails the test.
const TIMEOUT: Duration = Duration::from_secs(20);

/// iroh-blobs' reset code for a refused request (`AbortReason::Permission`).
const DENIED: u64 = 1;

const DOCS_BODY: &[u8] = b"quarterly figures, members only";
const OTHER_BODY: &[u8] = b"a blob that only the other world names";

/// The world graph the gate reads: membership and references.
#[derive(Default)]
struct Graph {
    members: Vec<(&'static str, EndpointId)>,
    refs: Vec<(Hash, &'static str)>,
}

impl WorldAccess for Graph {
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
        self.members
            .iter()
            .any(|(w, m)| *w == world && m == requester)
    }

    fn worlds_referencing(&self, hash: &Hash) -> Vec<String> {
        self.refs
            .iter()
            .filter(|(h, _)| h == hash)
            .map(|(_, w)| (*w).to_owned())
            .collect()
    }
}

/// A node that connects to the provider: an endpoint with a known identity and its own store.
struct Peer {
    endpoint: Endpoint,
    store: Store,
    id: EndpointId,
}

async fn peer(seed: u8) -> Peer {
    let key = SecretKey::from_bytes(&[seed; 32]);
    let id = key.public();
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(key)
        .bind()
        .await
        .unwrap();
    let store: Store = MemStore::new().into();
    Peer {
        endpoint,
        store,
        id,
    }
}

/// The provider's blob store. The temp tags keep its blobs alive for the test.
fn server_store() -> Store {
    MemStore::new().into()
}

/// A provider that serves `store` through the gate. Pushes are refused for everyone.
async fn provider(store: &Store, graph: Graph, policy: BlobPolicy) -> (Router, EndpointAddr) {
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(SecretKey::from_bytes(&[200; 32]))
        .bind()
        .await
        .unwrap();
    let addr = endpoint.addr();
    let events = blob_events(Arc::new(graph), policy, AccessPolicy::restricted([]));
    let router = Router::builder(endpoint)
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(store, Some(events)))
        .spawn();
    (router, addr)
}

/// Waits for `fut`, failing the test if it takes longer than [`TIMEOUT`].
async fn within<T>(fut: impl IntoFuture<Output = T>) -> T {
    tokio::time::timeout(TIMEOUT, fut.into_future())
        .await
        .expect("the request must finish within the timeout")
}

/// Fetches `hash` from `server`: the bytes, or the error text.
async fn fetch(client: &Peer, server: &EndpointAddr, hash: Hash) -> Result<Vec<u8>, String> {
    let conn = within(client.endpoint.connect(server.clone(), iroh_blobs::ALPN))
        .await
        .unwrap();
    let got = within(get_blob(conn.clone(), hash).bytes()).await;
    conn.close(0u32.into(), b"done");
    got.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// The reset code of a fetch the provider refused, or `None` if the fetch was not refused.
async fn fetch_refusal_code(client: &Peer, server: &EndpointAddr, hash: Hash) -> Option<u64> {
    let conn = within(client.endpoint.connect(server.clone(), iroh_blobs::ALPN))
        .await
        .unwrap();
    let got = within(get_blob(conn.clone(), hash).bytes()).await;
    conn.close(0u32.into(), b"done");
    got.err()
        .and_then(|e| e.iroh_error_code())
        .map(iroh::endpoint::VarInt::into_inner)
}

/// Observes `hash` on `server`: the size the provider reports, or the error text.
async fn observe(client: &Peer, server: &EndpointAddr, hash: Hash) -> Result<u64, String> {
    let conn = within(client.endpoint.connect(server.clone(), iroh_blobs::ALPN))
        .await
        .unwrap();
    let mut answers = Box::pin(
        client
            .store
            .remote()
            .observe(conn.clone(), ObserveRequest::new(hash)),
    );
    let first = within(answers.next()).await;
    conn.close(0u32.into(), b"done");
    match first {
        Some(Ok(bitfield)) => Ok(bitfield.size()),
        Some(Err(e)) => Err(e.to_string()),
        None => Err("the observe stream ended without an answer".to_owned()),
    }
}

/// Adds `bytes` to `store`. The temp tag keeps the blob in the store while it lives.
async fn put(store: &Store, bytes: &[u8]) -> (Hash, TempTag) {
    add_raw(store, bytes.to_vec()).await.unwrap()
}

#[tokio::test]
async fn a_non_member_is_refused_fetch_and_observe() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(1).await;
    let stranger = peer(2).await;
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    };
    // Observe is on, so only membership can refuse the stranger.
    let policy = BlobPolicy::default().with_observe();
    let (router, addr) = provider(&server, graph, policy).await;

    assert_eq!(
        fetch_refusal_code(&stranger, &addr, docs).await,
        Some(DENIED)
    );
    assert!(observe(&stranger, &addr, docs).await.is_err());
    assert!(!stranger.store.blobs().has(docs).await.unwrap());

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_member_of_the_referencing_world_fetches_the_bytes() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(1).await;
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    };
    let (router, addr) = provider(&server, graph, BlobPolicy::default()).await;

    let got = fetch(&member, &addr, docs)
        .await
        .expect("a member may fetch");
    assert_eq!(got, DOCS_BODY);

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_member_of_the_referencing_world_observes_when_the_server_enables_it() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(1).await;
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    };
    let policy = BlobPolicy::default().with_observe();
    let (router, addr) = provider(&server, graph, policy).await;

    let size = observe(&member, &addr, docs)
        .await
        .expect("observe is enabled");
    assert_eq!(size, DOCS_BODY.len() as u64);

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_member_of_an_unrelated_world_is_refused() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let outsider = peer(3).await;
    // The outsider is a member of "other", which does not name the docs blob.
    let graph = Graph {
        members: vec![("other", outsider.id)],
        refs: vec![(docs, "docs")],
    };
    let policy = BlobPolicy::default().with_observe();
    let (router, addr) = provider(&server, graph, policy).await;

    assert_eq!(
        fetch_refusal_code(&outsider, &addr, docs).await,
        Some(DENIED)
    );
    assert!(observe(&outsider, &addr, docs).await.is_err());

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn observe_is_refused_by_default_even_for_a_member() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(1).await;
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    };
    let (router, addr) = provider(&server, graph, BlobPolicy::default()).await;

    assert!(
        observe(&member, &addr, docs).await.is_err(),
        "observe is off by default, so the size must not be revealed"
    );
    // Fetch is decided by membership alone, so the member still gets the bytes.
    assert_eq!(fetch(&member, &addr, docs).await.unwrap(), DOCS_BODY);

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_world_narrows_observe_off_while_the_server_allows_it() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(1).await;
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    };
    let policy = BlobPolicy::default().with_observe().narrow_observe("docs");
    let (router, addr) = provider(&server, graph, policy).await;

    assert!(observe(&member, &addr, docs).await.is_err());
    assert_eq!(fetch(&member, &addr, docs).await.unwrap(), DOCS_BODY);

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_unknown_hash_and_a_denied_hash_get_the_same_response() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(1).await;
    let stranger = peer(2).await;
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    };
    let policy = BlobPolicy::default().with_observe();
    let (router, addr) = provider(&server, graph, policy).await;

    // Unknown: no world names this hash, and the store does not hold it.
    let unknown = Hash::from_bytes([42; 32]);
    // Denied: the blob exists and a world names it, but the requester is not a member.
    let unknown_code = fetch_refusal_code(&member, &addr, unknown).await;
    let denied_code = fetch_refusal_code(&stranger, &addr, docs).await;
    assert_eq!(unknown_code, Some(DENIED));
    assert_eq!(unknown_code, denied_code);

    let unknown_seen = observe(&member, &addr, unknown).await.expect_err("refused");
    let denied_seen = observe(&stranger, &addr, docs).await.expect_err("refused");
    assert_eq!(unknown_seen, denied_seen);

    router.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_get_many_with_one_denied_hash_is_refused_whole() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let (other, _other_tag) = put(&server, OTHER_BODY).await;
    let member = peer(1).await;
    // The member is in "docs" only, so the "other" blob is denied to it.
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs"), (other, "other")],
    };
    let (router, addr) = provider(&server, graph, BlobPolicy::default()).await;

    let conn = within(member.endpoint.connect(addr.clone(), iroh_blobs::ALPN))
        .await
        .unwrap();
    let mixed = GetManyRequest::from_iter([docs, other]);
    let refused = within(member.store.remote().execute_get_many(conn.clone(), mixed)).await;
    assert!(refused.is_err(), "one denied hash refuses the whole batch");
    assert!(!member.store.blobs().has(docs).await.unwrap());

    let allowed = GetManyRequest::from_iter([docs]);
    within(
        member
            .store
            .remote()
            .execute_get_many(conn.clone(), allowed),
    )
    .await
    .expect("an allowed batch is served");
    assert!(member.store.blobs().has(docs).await.unwrap());

    conn.close(0u32.into(), b"done");
    router.shutdown().await.unwrap();
}
