//! Content on removal (Phase 5, P5.6; design decision 4: "Content is kept. Removal stops new
//! content"). The author of a document may remove it, and so may a device key the author has
//! authorized in the document's group — an agent acting for the owner (decision on #195,
//! 2026-11-16). The removal is a signed `document.remove` event that names the document's
//! event ID, and it does not delete anything:
//!
//! 1. Content is kept. The removal is an event like any other, and it stays in the store. The
//!    document, its blob and its tags are unchanged, and a member of the world may still fetch the
//!    bytes.
//! 2. Only an effective removal marks the document: the document's author's, or one signed
//!    by a device key the author authorized in the document's group at Write scope or above.
//!    A removal from anyone else is stored and has no effect.
//! 3. The record and the tags do not depend on the order events arrive in.
//! 4. A removal takes no body, and it carries no tag and no document.
//! 5. New content stops: a `tag.set` on the document whose causal past contains an effective
//!    removal does not fold — its author had seen the removal. A tag concurrent with the
//!    removal, whose author had not seen it, is kept, and a `tag.clear` still folds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

mod common;

use common::within;

use id::{
    access::AccessPolicy,
    blob_gate::{blob_events, BlobPolicy, WorldAccess},
    chain::{sign, ChainExt, Event},
    commands::put::add_raw,
    documents::{DocumentRef, DOCUMENT_KIND, REMOVAL_KIND},
    event_store::{EventStore, Ingest, Refusal},
    groups::{
        event::encode as encode_group_event, Body as GroupBody, DeviceOp, Event as GroupEvent,
        Role as GroupRole,
    },
    membership::{body::encode_body, AuthId},
    tag_claims::{TagOp, SET_KIND},
    waiting::WaitingLimits,
};
use iroh::{
    endpoint::{presets, RelayMode},
    protocol::Router,
    Endpoint, EndpointAddr,
};
use iroh_base::{EndpointId, SecretKey};
use iroh_blobs::{
    api::Store, get::request::get_blob, store::mem::MemStore, BlobsProtocol, Hash as BlobHash,
};
use p2panda_auth::{
    group::{GroupAction, GroupMember},
    Access,
};
use p2panda_core::{Hash, SeqNum, SigningKey};

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

    /// Signs `ext` with a body after the writer's last event (group operations travel
    /// with their bodies, as in `tag_clear_authority.rs`).
    fn send_with_body(&mut self, ext: ChainExt, body: &[u8]) -> Event {
        let event = sign(&self.key, self.seq, self.last, Some(body), ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        event
    }

    /// This writer's author key bytes.
    fn author(&self) -> [u8; 32] {
        *self.key.verifying_key().as_bytes()
    }
}

/// A group operation's header extensions: no tag, document, or removal beside it.
fn group_ext(kind: &str, group: Option<Hash>, after: Vec<Hash>) -> ChainExt {
    ChainExt {
        kind: kind.to_owned(),
        group,
        after,
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
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
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
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
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
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
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
    }
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

/// Delivers one event with its body, and checks that it was accepted.
fn deliver_body(store: &mut EventStore, event: &Event, body: &[u8]) {
    let outcome = store.ingest_with_body(&event.bytes, Some(body), 0).unwrap();
    assert!(
        matches!(outcome, Ingest::Accepted { .. }),
        "not accepted: {outcome:?}"
    );
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
        std::collections::BTreeSet::new(),
    );
    let router = Router::builder(endpoint)
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(store, Some(events)))
        .spawn();
    (router, addr)
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
        anchor: None,
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, missing).is_err());
    // A document or a tag beside the target.
    let with_document = ChainExt {
        document: Some(doc),
        anchor: None,
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, with_document).is_err());
    let with_tag = ChainExt {
        tag: Some(TagOp::Set {
            subject: target,
            name: "kind".to_owned(),
            value: Some("memo".to_owned()),
        }),
        anchor: None,
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, with_tag).is_err());
    // A target on any other kind.
    let stray = ChainExt {
        kind: DOCUMENT_KIND.to_owned(),
        document: Some(DocumentRef::for_bytes("text/plain", b"notes").unwrap()),
        anchor: None,
        ..removal_ext(target, vec![])
    };
    assert!(sign(&key, 0, None, None, stray).is_err());
    // The well-formed removal is signed.
    assert!(sign(&key, 0, None, None, removal_ext(target, vec![])).is_ok());
}

#[test]
fn test_owner_removal_stops_tags_whose_author_had_seen_it() {
    // Oracle: the 2026-11-16 decision on #195 — an effective removal stops new content.
    // A tag set whose causal past contains the removal was written with knowledge of
    // it, so it does not fold; the tag from before the removal does.
    let doc = DocumentRef::for_bytes("text/markdown", BODY).unwrap();
    let mut author = Writer::new(31);
    let mut other = Writer::new(32);
    let published = author.send(document_ext(&doc, vec![]));
    let early = author.send(tag_ext(published.id, "kind", "minutes", vec![published.id]));
    let removal = author.send(removal_ext(published.id, vec![early.id]));
    // The author's own later tag: its backlink is the removal and `after` names it.
    let late_author = author.send(tag_ext(published.id, "late", "a", vec![removal.id]));
    // Another writer's later tag that names the removal in `after`: had seen it too.
    let late_other = other.send(tag_ext(
        published.id,
        "late",
        "b",
        vec![published.id, removal.id],
    ));

    let (_dir, mut store) = open();
    for event in [&published, &early, &removal, &late_author, &late_other] {
        deliver(&mut store, event);
    }
    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    assert_eq!(claims[0].id, early.id);
    // The stopped tags stay stored and applied; only the default view loses them.
    assert!(store.event_applied(&late_author.id).unwrap());
    assert!(store.event_applied(&late_other.id).unwrap());
}

#[test]
fn test_concurrent_tag_that_never_saw_the_removal_is_kept() {
    // Oracle: the causal rule stated on #195 — concurrent tags are kept. This tag is
    // delivered after the removal, but its causal past (`after` plus its author's
    // backlink chain) does not contain the removal, so arrival order does not stop it.
    let doc = DocumentRef::for_bytes("text/markdown", BODY).unwrap();
    let mut author = Writer::new(41);
    let mut other = Writer::new(42);
    let published = author.send(document_ext(&doc, vec![]));
    let removal = author.send(removal_ext(published.id, vec![]));
    let concurrent = other.send(tag_ext(published.id, "project", "p5", vec![published.id]));

    let (_dir, mut store) = open();
    deliver(&mut store, &published);
    deliver(&mut store, &removal);
    deliver(&mut store, &concurrent);

    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1, "{claims:?}");
    assert_eq!(claims[0].id, concurrent.id);
    // The document is still marked removed; only the tag survives.
    assert_eq!(store.documents().unwrap()[0].removal, Some(removal.id));
}

#[test]
fn test_removal_by_an_agent_the_owner_authorized_marks_and_stops() {
    // Oracle: the 2026-11-16 decision on #195 — who may remove is the document's owner
    // and agents acting for them. The agent here is a device key the owner authorized
    // in the document's group (P8.3) at Admin scope.
    let doc = DocumentRef::for_bytes("text/markdown", BODY).unwrap();
    let mut owner = Writer::new(51);
    let mut device = Writer::new(52);

    // The owner creates a group; the settled fold gives the creator Admin.
    let create_body = encode_body(&GroupAction::Create {
        initial_members: vec![(
            GroupMember::Individual(AuthId::Key(owner.author())),
            Access::manage(),
        )],
    })
    .unwrap();
    let create = owner.send_with_body(group_ext("group.op", None, vec![]), &create_body);
    let group = create.id;

    // The owner authorizes the device key in that group at Admin scope.
    let grant_wire = GroupEvent {
        id: Hash::digest(b"agent-removal device grant"),
        author: owner.author(),
        after: vec![create.id],
        group: Some(group),
        body: GroupBody::Device(DeviceOp::Add {
            device: device.author(),
            scope: GroupRole::Admin,
        }),
    };
    let grant_body = encode_group_event(&grant_wire).unwrap();
    let grant = owner.send_with_body(
        group_ext("group.event", Some(group), vec![create.id]),
        &grant_body,
    );

    // The owner publishes the document in that group.
    let mut doc_ext = document_ext(&doc, vec![grant.id]);
    doc_ext.group = Some(group);
    let published = owner.send(doc_ext);

    // The device — the owner's agent — removes it.
    let removal = device.send(removal_ext(published.id, vec![]));
    // The owner's later tag had seen the agent's removal: stopped.
    let late = owner.send(tag_ext(published.id, "late", "x", vec![removal.id]));

    let (_dir, mut store) = open();
    deliver_body(&mut store, &create, &create_body);
    deliver_body(&mut store, &grant, &grant_body);
    deliver(&mut store, &published);
    deliver(&mut store, &removal);
    deliver(&mut store, &late);

    let record = store.documents().unwrap();
    assert_eq!(record.len(), 1, "{record:?}");
    assert_eq!(record[0].removal, Some(removal.id), "{record:?}");
    assert!(
        store.tag_claims().unwrap().is_empty(),
        "the post-removal tag must not fold"
    );
}
