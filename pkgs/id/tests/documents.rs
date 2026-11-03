//! Documents over the event store and blob fetch (Phase 5, P5.4; design §9.3).
//!
//! A document is a `document.put` event that names its content by BLAKE3 hash and size, and its
//! media type. The bytes are a blob. Two replicas exchange only event bytes, as sync does, and
//! fetch the blob from a provider on request, through the gate in [`id::blob_gate`]:
//!
//! 1. A replica that holds the events holds the same document record and the same tags as the
//!    author, whatever order the events arrive in.
//! 2. The bytes come only to a member of a world that names the blob. A replica outside that world
//!    holds the record and the tags, and is refused the bytes.
//! 3. A clear of a document's tag removes that claim and leaves the document in the record.
//! 4. A document event that does not fit its kind is not signed, and one that carries a payload
//!    body is refused on receipt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{future::IntoFuture, sync::Arc, time::Duration};

use id::{
    access::AccessPolicy,
    blob_gate::{BlobPolicy, WorldAccess, blob_events},
    chain::{ChainExt, Event, sign},
    commands::put::add_raw,
    documents::{DOCUMENT_KIND, DocumentRef, MAX_MEDIA_TYPE_BYTES},
    event_store::{EventStore, Ingest, Refusal},
    tag_claims::{CLEAR_KIND, SET_KIND, TagOp},
    waiting::{AuthorKey, WaitingLimits},
};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use iroh_base::{EndpointId, SecretKey};
use iroh_blobs::{
    BlobsProtocol, Hash as BlobHash, api::Store, get::request::get_blob, store::mem::MemStore,
};
use p2panda_core::{Hash, SeqNum, SigningKey};

/// Every network step must finish within this, so a stuck request fails the test.
const TIMEOUT: Duration = Duration::from_secs(20);

/// iroh-blobs' reset code for a refused request (`AbortReason::Permission`).
const DENIED: u64 = 1;

const BODY: &[u8] = b"# Minutes\n\nThe plan is agreed.";

/// The world graph the blob gate reads: membership and references.
#[derive(Default)]
struct Graph {
    members: Vec<(&'static str, EndpointId)>,
    refs: Vec<(BlobHash, &'static str)>,
}

impl WorldAccess for Graph {
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
        self.members
            .iter()
            .any(|(w, member)| *w == world && member == requester)
    }

    fn worlds_referencing(&self, hash: &BlobHash) -> Vec<String> {
        self.refs
            .iter()
            .filter(|(h, _)| h == hash)
            .map(|(_, w)| (*w).to_owned())
            .collect()
    }
}

/// A signer with its own chain. Each event names the one before it.
struct Writer {
    key: SigningKey,
    seq: SeqNum,
    last: Option<Hash>,
}

impl Writer {
    fn new(n: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[n; 32]),
            seq: 0,
            last: None,
        }
    }

    fn author(&self) -> AuthorKey {
        *self.key.verifying_key().as_bytes()
    }

    /// Signs `ext` after the writer's last event.
    fn send(&mut self, ext: ChainExt) -> Event {
        let event = sign(&self.key, self.seq, self.last, None, ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        event
    }
}

fn document_ext(doc: &DocumentRef, after: Vec<Hash>) -> ChainExt {
    ChainExt {
        kind: DOCUMENT_KIND.to_owned(),
        group: None,
        after,
        tag: None,
        document: Some(doc.clone()),
        removes: None,
        note: None,
    }
}

fn tag_ext(op: TagOp, after: Vec<Hash>) -> ChainExt {
    let kind = match op {
        TagOp::Set { .. } => SET_KIND,
        TagOp::Clear { .. } => CLEAR_KIND,
    };
    ChainExt {
        kind: kind.to_owned(),
        group: None,
        after,
        tag: Some(op),
        document: None,
        removes: None,
        note: None,
    }
}

fn set(subject: Hash, name: &str, value: &str) -> TagOp {
    TagOp::Set {
        subject,
        name: name.to_owned(),
        value: Some(value.to_owned()),
    }
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

/// Delivers one event's bytes, as sync does, and checks that it was not refused.
fn deliver(store: &mut EventStore, event: &Event) {
    let outcome = store.ingest(&event.bytes, 0).unwrap();
    assert!(
        !matches!(outcome, Ingest::Refused(_)),
        "refused: {outcome:?}"
    );
}

/// A node that connects to the provider: an endpoint with a known identity.
struct Peer {
    endpoint: Endpoint,
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
    Peer { endpoint, id }
}

/// A provider that serves `store` through the gate. Pushes are refused for everyone.
async fn provider(store: &Store, graph: Graph) -> (Router, EndpointAddr) {
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(SecretKey::from_bytes(&[200; 32]))
        .bind()
        .await
        .unwrap();
    let addr = endpoint.addr();
    let events = blob_events(
        Arc::new(graph),
        BlobPolicy::default(),
        AccessPolicy::restricted([]),
    );
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
async fn fetch(client: &Peer, server: &EndpointAddr, hash: BlobHash) -> Result<Vec<u8>, String> {
    let conn = within(client.endpoint.connect(server.clone(), iroh_blobs::ALPN))
        .await
        .unwrap();
    let got = within(get_blob(conn.clone(), hash).bytes()).await;
    conn.close(0u32.into(), b"done");
    got.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// The reset code of a fetch the provider refused, or `None` if the fetch was not refused.
async fn fetch_refusal_code(client: &Peer, server: &EndpointAddr, hash: BlobHash) -> Option<u64> {
    let conn = within(client.endpoint.connect(server.clone(), iroh_blobs::ALPN))
        .await
        .unwrap();
    let got = within(get_blob(conn.clone(), hash).bytes()).await;
    conn.close(0u32.into(), b"done");
    got.err()
        .and_then(|e| e.iroh_error_code())
        .map(iroh::endpoint::VarInt::into_inner)
}

/// The author's three events: the document, then two tags on it, one from each writer.
fn authored() -> (DocumentRef, Writer, Vec<Event>) {
    let doc = DocumentRef::for_bytes("text/markdown", BODY).unwrap();
    let mut author = Writer::new(1);
    let mut other = Writer::new(2);
    let published = author.send(document_ext(&doc, vec![]));
    let kind = author.send(tag_ext(
        set(published.id, "kind", "minutes"),
        vec![published.id],
    ));
    let project = other.send(tag_ext(
        set(published.id, "project", "p5"),
        vec![published.id],
    ));
    (doc, author, vec![published, kind, project])
}

#[tokio::test]
async fn test_two_replicas_that_exchange_events_and_blob_bytes_converge_on_the_document_and_its_tags()
 {
    let (doc, author, events) = authored();
    let provider_store: Store = MemStore::new().into();
    let (blob, _keep) = add_raw(&provider_store, BODY.to_vec()).await.unwrap();
    assert_eq!(blob, doc.blob_hash(), "the document names the blob's hash");

    // The author's replica holds every event, in the order written.
    let (_author_dir, mut author_store) = open();
    for event in &events {
        deliver(&mut author_store, event);
    }

    // The other replica gets only the event bytes, in reverse order, so its tags arrive first.
    let (_replica_dir, mut replica) = open();
    for event in events.iter().rev() {
        deliver(&mut replica, event);
    }

    // It is a member of the world that names the blob, so it may fetch the bytes.
    let replica_peer = peer(7).await;
    let graph = Graph {
        members: vec![("docs", replica_peer.id)],
        refs: vec![(blob, "docs")],
    };
    let (router, addr) = provider(&provider_store, graph).await;

    let record = replica.documents().unwrap();
    assert_eq!(record.len(), 1, "one document: {record:?}");
    assert_eq!(record[0].id, events[0].id);
    assert_eq!(record[0].author, author.author());
    assert_eq!(record[0].document, doc);

    let got = fetch(&replica_peer, &addr, record[0].document.blob_hash())
        .await
        .expect("a member may fetch");
    record[0].document.verify_content(&got).unwrap();
    assert_eq!(got, BODY);

    assert_eq!(
        replica.documents().unwrap(),
        author_store.documents().unwrap()
    );
    assert_eq!(
        replica.tag_claims().unwrap(),
        author_store.tag_claims().unwrap()
    );
    assert_eq!(replica.tag_claims().unwrap().len(), 2);

    drop(router);
}

#[tokio::test]
async fn test_a_replica_outside_the_world_has_the_record_but_not_the_bytes() {
    let (doc, _author, events) = authored();
    let provider_store: Store = MemStore::new().into();
    let (blob, _keep) = add_raw(&provider_store, BODY.to_vec()).await.unwrap();

    let (_replica_dir, mut replica) = open();
    for event in &events {
        deliver(&mut replica, event);
    }

    // A member of some other world holds nothing that names this blob.
    let outsider = peer(8).await;
    let graph = Graph {
        members: vec![("docs", peer(7).await.id)],
        refs: vec![(blob, "docs")],
    };
    let (router, addr) = provider(&provider_store, graph).await;

    assert_eq!(replica.documents().unwrap()[0].document, doc);
    assert_eq!(
        fetch_refusal_code(&outsider, &addr, doc.blob_hash()).await,
        Some(DENIED)
    );

    drop(router);
}

#[test]
fn test_a_clear_of_a_document_tag_leaves_the_document_in_the_record() {
    let doc = DocumentRef::for_bytes("text/plain", b"notes").unwrap();
    let mut author = Writer::new(1);
    let mut other = Writer::new(2);
    let published = author.send(document_ext(&doc, vec![]));
    let memo = author.send(tag_ext(
        set(published.id, "kind", "memo"),
        vec![published.id],
    ));
    // The other writer's claim is concurrent with the memo: the author's clear does not see it.
    let draft = other.send(tag_ext(
        set(published.id, "kind", "draft"),
        vec![published.id],
    ));
    let cleared = author.send(tag_ext(
        TagOp::Clear {
            claims: vec![memo.id],
        },
        vec![memo.id],
    ));

    let (_dir, mut store) = open();
    for event in [&published, &memo, &draft, &cleared] {
        deliver(&mut store, event);
    }

    let record = store.documents().unwrap();
    assert_eq!(record.len(), 1, "the clear leaves the document: {record:?}");
    assert_eq!(record[0].document, doc);
    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, draft.id);
    assert_eq!(claims[0].value.as_deref(), Some("draft"));
}

#[test]
fn test_a_document_event_with_a_payload_body_is_refused() {
    let (_doc, _author, events) = authored();
    let (_dir, mut store) = open();
    let outcome = store
        .ingest_with_body(&events[0].bytes, Some(BODY), 0)
        .unwrap();
    assert!(
        matches!(outcome, Ingest::Refused(Refusal::Malformed(_))),
        "{outcome:?}"
    );
    assert!(store.documents().unwrap().is_empty());
}

#[test]
fn test_a_document_event_that_does_not_fit_its_kind_is_not_signed() {
    let key = SigningKey::from_bytes(&[1; 32]);
    let doc = DocumentRef::for_bytes("text/plain", b"notes").unwrap();
    let too_long = DocumentRef {
        media_type: "m".repeat(MAX_MEDIA_TYPE_BYTES + 1),
        ..doc
    };
    assert!(sign(&key, 0, None, None, document_ext(&too_long, vec![])).is_err());
    let missing = ChainExt {
        kind: DOCUMENT_KIND.to_owned(),
        group: None,
        after: vec![],
        tag: None,
        document: None,
        removes: None,
        note: None,
    };
    assert!(sign(&key, 0, None, None, missing).is_err());
    let with_tag = ChainExt {
        tag: Some(set(Hash::digest(b"doc"), "kind", "memo")),
        ..document_ext(&doc, vec![])
    };
    assert!(sign(&key, 0, None, None, with_tag).is_err());
}
