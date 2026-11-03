//! Content on removal (Phase 5, P5.6; design decision 4: "Content is kept. Removal stops new
//! content"). The author of a document may remove it. The removal is a signed `document.remove`
//! event that names the document's event ID, and it does not delete anything:
//!
//! 1. Content is kept. The removal is an event like any other, and it stays in the store. The
//!    document, its blob and its tags are unchanged, and a member of the world may still fetch the
//!    bytes.
//! 2. Only the removal by the document's author marks the document. A removal from anyone else is
//!    stored and has no effect.
//! 3. The record and the tags do not depend on the order events arrive in.
//! 4. A removal takes no body, and it carries no tag and no document.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{future::IntoFuture, sync::Arc, time::Duration};

use id::{
    access::AccessPolicy,
    blob_gate::{BlobPolicy, WorldAccess, blob_events},
    chain::{ChainExt, Event, sign},
    commands::put::add_raw,
    documents::{DOCUMENT_KIND, DocumentRef, REMOVAL_KIND},
    event_store::{EventStore, Ingest, Refusal},
    tag_claims::{SET_KIND, TagOp},
    waiting::WaitingLimits,
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

fn tag_ext(subject: Hash, name: &str, value: &str, after: Vec<Hash>) -> ChainExt {
    ChainExt {
        kind: SET_KIND.to_owned(),
        group: None,
        after,
        tag: Some(TagOp::Set {
            subject,
            name: name.to_owned(),
            value: Some(value.to_owned()),
        }),
        document: None,
        removes: None,
        note: None,
    }
}

fn removal_ext(target: Hash, after: Vec<Hash>) -> ChainExt {
    ChainExt {
        kind: REMOVAL_KIND.to_owned(),
        group: None,
        after,
        tag: None,
        document: None,
        removes: Some(target),
        note: None,
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

/// The author's events: the document, two tags on it, and then its removal. A second writer
/// tags the document too, so the tags come from both sides.
struct Story {
    doc: DocumentRef,
    other: Writer,
    published: Event,
    kind: Event,
    project: Event,
    removal: Event,
}

fn story() -> Story {
    let doc = DocumentRef::for_bytes("text/markdown", BODY).unwrap();
    let mut author = Writer::new(1);
    let mut other = Writer::new(2);
    let published = author.send(document_ext(&doc, vec![]));
    let kind = author.send(tag_ext(published.id, "kind", "minutes", vec![published.id]));
    let project = other.send(tag_ext(published.id, "project", "p5", vec![published.id]));
    let removal = author.send(removal_ext(published.id, vec![kind.id]));
    Story {
        doc,
        other,
        published,
        kind,
        project,
        removal,
    }
}

/// Everything the author wrote, before the removal.
fn before_removal(s: &Story) -> Vec<&Event> {
    vec![&s.published, &s.kind, &s.project]
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

#[test]
fn test_a_removal_is_kept_as_an_event_and_marks_its_document() {
    let s = story();
    let (_dir, mut store) = open();
    for event in before_removal(&s) {
        deliver(&mut store, event);
    }
    deliver(&mut store, &s.removal);

    // The removal stays in the store, as an event like any other.
    assert!(store.event_applied(&s.removal.id).unwrap());
    assert!(store.event_by_id(&s.removal.id).unwrap().is_some());

    // The document stays in the record, with its content, and it names the removal.
    let record = store.documents().unwrap();
    assert_eq!(record.len(), 1, "{record:?}");
    assert_eq!(record[0].id, s.published.id);
    assert_eq!(record[0].document, s.doc);
    assert_eq!(record[0].removal, Some(s.removal.id));
}

#[test]
fn test_the_removal_leaves_the_tags_and_their_fold_unchanged() {
    let s = story();
    let (_without_dir, mut without) = open();
    for event in before_removal(&s) {
        deliver(&mut without, event);
    }
    let (_with_dir, mut with) = open();
    for event in before_removal(&s) {
        deliver(&mut with, event);
    }
    deliver(&mut with, &s.removal);

    let claims = without.tag_claims().unwrap();
    assert_eq!(claims.len(), 2, "{claims:?}");
    assert_eq!(with.tag_claims().unwrap(), claims);
}

#[test]
fn test_replicas_agree_on_the_event_set_and_the_record_in_any_order() {
    let s = story();
    let all = [&s.published, &s.kind, &s.project, &s.removal];

    // One replica gets the events in order, the other in reverse, so the removal arrives first.
    let (_forward_dir, mut forward) = open();
    for event in all {
        deliver(&mut forward, event);
    }
    let (_reverse_dir, mut reverse) = open();
    for event in all.iter().rev() {
        deliver(&mut reverse, event);
    }

    for event in all {
        assert!(
            forward.event_applied(&event.id).unwrap(),
            "forward: {event:?}"
        );
        assert!(
            reverse.event_applied(&event.id).unwrap(),
            "reverse: {event:?}"
        );
    }
    assert_eq!(reverse.documents().unwrap(), forward.documents().unwrap());
    assert_eq!(reverse.tag_claims().unwrap(), forward.tag_claims().unwrap());
    assert_eq!(
        reverse.documents().unwrap()[0].removal,
        Some(s.removal.id),
        "the removal marks the document in either order"
    );
}

#[test]
fn test_only_the_documents_author_may_remove_it() {
    let mut s = story();
    // The other writer names the document in a removal of their own. It is stored, and it has
    // no effect, since the other writer did not publish the document.
    let other_removal = s
        .other
        .send(removal_ext(s.published.id, vec![s.project.id]));

    let (_dir, mut store) = open();
    for event in before_removal(&s) {
        deliver(&mut store, event);
    }
    deliver(&mut store, &other_removal);

    assert!(store.event_applied(&other_removal.id).unwrap());
    let record = store.documents().unwrap();
    assert_eq!(record.len(), 1, "{record:?}");
    assert_eq!(record[0].removal, None, "{record:?}");
    assert_eq!(record[0].document, s.doc);
}

#[tokio::test]
async fn test_a_member_may_still_fetch_the_bytes_after_removal() {
    // Decision 4 keeps the content. The blob gate is unchanged by a removal: the member who was
    // entitled before it is entitled after it.
    let s = story();
    let provider_store: Store = MemStore::new().into();
    let (blob, _keep) = add_raw(&provider_store, BODY.to_vec()).await.unwrap();
    assert_eq!(blob, s.doc.blob_hash());

    let (_dir, mut store) = open();
    for event in before_removal(&s) {
        deliver(&mut store, event);
    }
    deliver(&mut store, &s.removal);

    let replica = peer(7).await;
    let graph = Graph {
        members: vec![("docs", replica.id)],
        refs: vec![(blob, "docs")],
    };
    let (router, addr) = provider(&provider_store, graph).await;

    let record = store.documents().unwrap();
    let got = fetch(&replica, &addr, record[0].document.blob_hash())
        .await
        .expect("a member keeps its access after the removal");
    record[0].document.verify_content(&got).unwrap();
    assert_eq!(got, BODY);

    drop(router);
}

#[test]
fn test_a_removal_with_a_body_is_refused() {
    let s = story();
    let (_dir, mut store) = open();
    let outcome = store
        .ingest_with_body(&s.removal.bytes, Some(BODY), 0)
        .unwrap();
    assert!(
        matches!(outcome, Ingest::Refused(Refusal::Malformed(_))),
        "{outcome:?}"
    );
    assert!(store.event_by_id(&s.removal.id).unwrap().is_none());
}

#[test]
fn test_a_removal_names_its_target_and_carries_nothing_else() {
    let key = SigningKey::from_bytes(&[1; 32]);
    let target = Hash::digest(b"document");
    let doc = DocumentRef::for_bytes("text/plain", b"notes").unwrap();

    // No target.
    let missing = ChainExt {
        removes: None,
        note: None,
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, missing).is_err());
    // A document or a tag beside the target.
    let with_document = ChainExt {
        document: Some(doc),
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, with_document).is_err());
    let with_tag = ChainExt {
        tag: Some(TagOp::Set {
            subject: target,
            name: "kind".to_owned(),
            value: Some("memo".to_owned()),
        }),
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, with_tag).is_err());
    // A target on any other kind.
    let stray = ChainExt {
        kind: DOCUMENT_KIND.to_owned(),
        document: Some(DocumentRef::for_bytes("text/plain", b"notes").unwrap()),
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, stray).is_err());
    // The well-formed removal is signed.
    assert!(sign(&key, 0, None, None, removal_ext(target, vec![])).is_ok());
}
