//! End-to-end tests of blob fetch and observe gating over real QUIC on loopback.
//!
//! A provider serves blobs through `BlobsProtocol` with the gate from `id::blob_gate`. Requesters
//! connect directly (no relay, no discovery). The gate reads a fixed world graph: which worlds
//! name which blob, and who is a current member of each world.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

mod common;

use common::within;

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
    provider_with_access(store, Arc::new(graph), policy).await
}

/// A provider for access implementations whose state changes during a test.
async fn provider_with_access<A>(
    store: &Store,
    access: Arc<A>,
    policy: BlobPolicy,
) -> (Router, EndpointAddr)
where
    A: WorldAccess + Send + Sync + 'static,
{
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(SecretKey::from_bytes(&[200; 32]))
        .bind()
        .await
        .unwrap();
    let addr = endpoint.addr();
    let events = blob_events(
        access,
        policy,
        AccessPolicy::restricted([]),
        std::collections::BTreeSet::new(),
    );
    let router = Router::builder(endpoint)
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(store, Some(events)))
        .spawn();
    (router, addr)
}

/// A provider whose gate also trusts `local` as local readers (the operator's
/// own keys), as `id serve` wires it.
async fn provider_with_local<A>(
    store: &Store,
    access: Arc<A>,
    policy: BlobPolicy,
    local: std::collections::BTreeSet<EndpointId>,
) -> (Router, EndpointAddr)
where
    A: WorldAccess + Send + Sync + 'static,
{
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(SecretKey::from_bytes(&[200; 32]))
        .bind()
        .await
        .unwrap();
    let addr = endpoint.addr();
    let events = blob_events(access, policy, AccessPolicy::restricted([]), local);
    let router = Router::builder(endpoint)
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(store, Some(events)))
        .spawn();
    (router, addr)
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
async fn a_local_reader_fetches_an_unreferenced_blob_but_a_stranger_cannot() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let local = peer(7).await;
    let stranger = peer(8).await;
    // No world references the blob and nobody is a member of anything: this is
    // a locally published blob, readable only through local trust (#7181 P1).
    let graph = Graph {
        members: vec![],
        refs: vec![],
    };
    let (router, addr) = provider_with_local(
        &server,
        Arc::new(graph),
        BlobPolicy::default(),
        std::collections::BTreeSet::from([local.id]),
    )
    .await;

    let got = fetch(&local, &addr, docs)
        .await
        .expect("a local reader may fetch its own store's blobs");
    assert_eq!(got, DOCS_BODY);
    assert_eq!(
        fetch_refusal_code(&stranger, &addr, docs).await,
        Some(DENIED)
    );

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
async fn a_removed_member_is_refused_after_membership_refresh() {
    let server = server_store();
    let (docs, _tag) = put(&server, DOCS_BODY).await;
    let member = peer(4).await;
    let graph = Arc::new(std::sync::RwLock::new(Graph {
        members: vec![("docs", member.id)],
        refs: vec![(docs, "docs")],
    }));
    let access = Arc::new(MutableGraph(graph.clone()));
    let policy = BlobPolicy::default().with_observe();
    let (router, addr) = provider_with_access(&server, access, policy).await;

    assert_eq!(fetch(&member, &addr, docs).await.unwrap(), DOCS_BODY);
    graph.write().unwrap().members.clear();
    assert_eq!(fetch_refusal_code(&member, &addr, docs).await, Some(DENIED));
    assert!(observe(&member, &addr, docs).await.is_err());

    router.shutdown().await.unwrap();
}

/// Membership that a refresh can replace while the provider is serving peers.
struct MutableGraph(Arc<std::sync::RwLock<Graph>>);

impl WorldAccess for MutableGraph {
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
        self.0
            .read()
            .unwrap()
            .members
            .iter()
            .any(|(candidate, member)| *candidate == world && member == requester)
    }

    fn worlds_referencing(&self, hash: &Hash) -> Vec<String> {
        self.0
            .read()
            .unwrap()
            .refs
            .iter()
            .filter(|(candidate, _)| candidate == hash)
            .map(|(_, world)| (*world).to_owned())
            .collect()
    }
}

#[tokio::test]
async fn a_member_can_fetch_a_hashseq_child_but_a_non_member_cannot() {
    let server = server_store();
    let (child, _child_tag) = put(&server, DOCS_BODY).await;
    let (hashseq, _hashseq_tag) = put(&server, child.as_bytes()).await;
    let member = peer(5).await;
    let stranger = peer(6).await;
    // The sequence itself and its child are both attributed to the same world; the index
    // builder's transitive attribution is separately exercised by blob_gate unit tests.
    let graph = Graph {
        members: vec![("docs", member.id)],
        refs: vec![(hashseq, "docs"), (child, "docs")],
    };
    let (router, addr) = provider(&server, graph, BlobPolicy::default()).await;

    assert_eq!(fetch(&member, &addr, child).await.unwrap(), DOCS_BODY);
    assert_eq!(
        fetch_refusal_code(&stranger, &addr, child).await,
        Some(DENIED)
    );

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
