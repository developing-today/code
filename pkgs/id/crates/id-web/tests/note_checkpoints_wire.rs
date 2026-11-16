//! Wire-level integration tests for durable note checkpoints
//! (#3819 Phase 3, slice 4): real WebSocket clients against the
//! real router, exercising the path slices 1–3 built — note
//! sessions keyed by note ID, step broadcast and catch-up, the
//! checkpoint published when the last editor leaves, and a
//! restart whose fresh state rebuilds the log from the persisted
//! events and chains the next checkpoint onto the rebuilt heads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh_blobs::{api::Store, store::mem::MemStore};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;
use tokio_tungstenite::tungstenite::Message;

use id_store::note_checkpoints::NoteCheckpoints;
use id_store::notes::AppliedNote;
use id_store::tags::TagStore;
use id_web::{AppState, CollabState, WebSecurity, create_router, note_session_key};
use p2panda_core::Hash;

const NOTE_BYTES: [u8; 32] = [23u8; 32];
const SEED_TEXT: &[u8] = b"# Wire note\n";

struct Fixture {
    _endpoint: Endpoint,
    _docs: Docs,
    _temp: tempfile::TempDir,
    state: AppState,
}

impl Fixture {
    async fn new(name: &str, seed: [u8; 32]) -> Self {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = Docs::memory()
            .spawn(endpoint.clone(), store.clone(), gossip)
            .await
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            TagStore::init_in(&docs, name, temp.path().join("meta"))
                .await
                .unwrap(),
        );
        let dir = temp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::new(
            store.clone(),
            None,
            name.to_owned(),
            tags,
            seed,
            dir.join("identity.redb"),
        )
        .await
        .unwrap();
        Self {
            _endpoint: endpoint,
            _docs: docs,
            _temp: temp,
            state,
        }
    }
}

/// Serves the router on an ephemeral loopback port; returns the
/// port and the server task (aborted by the caller at test end).
async fn serve(state: AppState) -> (u16, tokio::task::JoinHandle<()>) {
    let security = WebSecurity {
        token: None,
        allowed_hosts: Vec::new(),
        cookie_secure: false,
    };
    let router = create_router(state, security);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (port, task)
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connects a client to the note session. The first frame sent is
/// a Steps message: the socket's auth phase buffers any non-Auth
/// first message and processes it after the Init, so a well-formed
/// Steps at the current version doubles as the auth-phase wake-up
/// (and a deliberately stale one is a harmless no-op probe).
async fn connect(port: u16, note: &Hash, first: serde_json::Value) -> Ws {
    let url = format!(
        "ws://127.0.0.1:{port}/ws/collab/{}?filename=wire.md",
        note_session_key(note)
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(Message::Binary(encode(&first).into()))
        .await
        .unwrap();
    ws
}

fn encode(value: &serde_json::Value) -> Vec<u8> {
    rmp_serde::to_vec(value).unwrap()
}

fn steps(version: u64, marker: &str, client_id: u64) -> serde_json::Value {
    serde_json::json!([1, version, [{ "stepType": "replace", "marker": marker }], client_id])
}

/// Reads frames until `pred` matches a decoded message; returns
/// every message seen along the way. Panics on a 15s drought.
async fn read_until(
    ws: &mut Ws,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> Vec<serde_json::Value> {
    let mut seen = Vec::new();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("timed out waiting for a collab frame")
            .expect("socket closed while waiting")
            .unwrap();
        if let Message::Binary(data) = frame {
            let value: serde_json::Value = rmp_serde::from_slice(&data).unwrap();
            let matched = pred(&value);
            seen.push(value);
            if matched {
                return seen;
            }
        }
    }
}

fn tag(value: &serde_json::Value) -> Option<u64> {
    value.as_array()?.first()?.as_u64()
}

async fn wait_for_events(collab: &CollabState, count: usize) -> Vec<AppliedNote> {
    for _ in 0..100 {
        let events = collab.note_events().await;
        if events.len() >= count {
            return events;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("checkpoint log never reached {count} events");
}

#[tokio::test]
async fn note_session_checkpoints_survive_the_wire_and_a_restart() {
    let note = Hash::from_bytes(NOTE_BYTES);

    // ---- Phase 1: two wire clients, one note session. ----
    let fixture = Fixture::new("wire-notes-one", [9; 32]).await;
    let collab = fixture.state.collab.clone();
    // Seed the session's document the way a host opening an
    // existing note would, so the checkpoint carries real content.
    collab
        .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
        .await;
    let (port, server) = serve(fixture.state).await;

    // Client A's first frame is its first edit (version 0 -> 1).
    let mut a = connect(port, &note, steps(0, "a1", 42)).await;
    let a_seen = read_until(&mut a, |m| tag(m) == Some(3)).await;
    assert!(
        a_seen.iter().any(|m| tag(m) == Some(0)),
        "A saw its Init before the Ack: {a_seen:?}"
    );
    assert_eq!(
        a_seen.last().unwrap()[1].as_u64(),
        Some(1),
        "A's edit acked at version 1"
    );

    // Client B joins with a stale probe, then observes A's second
    // edit as a broadcast Update.
    let mut b = connect(port, &note, steps(999, "probe", 77)).await;
    read_until(&mut b, |m| tag(m) == Some(0)).await;
    // Joining late, B is first caught up from version 0: the
    // replayed Update carries A's earlier step.
    let b_catchup = read_until(&mut b, |m| tag(m) == Some(2)).await;
    assert_eq!(
        b_catchup.last().unwrap()[1][0]["marker"],
        "a1",
        "B was caught up with A's first step"
    );
    a.send(Message::Binary(encode(&steps(1, "a2", 42)).into()))
        .await
        .unwrap();
    read_until(&mut a, |m| tag(m) == Some(3) && m[1].as_u64() == Some(2)).await;
    let b_seen = read_until(&mut b, |m| tag(m) == Some(2) && m[1][0]["marker"] == "a2").await;
    let update = b_seen.last().unwrap();
    assert_eq!(update[1][0]["marker"], "a2", "B received A's step verbatim");
    assert_eq!(
        update[2][0].as_u64(),
        Some(42),
        "the step is attributed to A's client id"
    );

    // B leaving is not session end; A leaving is — and because the
    // session moved since A opened it, a checkpoint is published.
    b.close(None).await.unwrap();
    a.close(None).await.unwrap();
    let events = wait_for_events(&collab, 1).await;
    let log = NoteCheckpoints::from_events(events.clone()).unwrap();
    let record = log
        .record(note)
        .unwrap()
        .expect("the note was checkpointed");
    assert_eq!(record.revisions.len(), 1);
    assert_eq!(record.heads.len(), 1);
    assert!(
        record.revisions[0].parents.is_empty(),
        "the first wire checkpoint is the create"
    );
    let created_content = record.revisions[0].content.clone();
    assert!(
        created_content.size > 0,
        "the checkpoint carries the seeded content"
    );
    server.abort();

    // ---- Phase 2: restart. A fresh state seeded with the ----
    // persisted events rebuilds the log; a new wire session chains
    // the next checkpoint onto the rebuilt head.
    let mut fixture2 = Fixture::new("wire-notes-two", [8; 32]).await;
    let seeded = CollabState::new();
    seeded.seed_note_events(events).await.unwrap();
    seeded
        .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
        .await;
    let collab2 = Arc::new(seeded);
    fixture2.state.collab = collab2.clone();
    let (port2, server2) = serve(fixture2.state).await;

    // Client C edits; client D, joining late, is caught up from
    // version 0 with C's step replayed.
    let mut c = connect(port2, &note, steps(0, "c1", 7)).await;
    read_until(&mut c, |m| tag(m) == Some(3) && m[1].as_u64() == Some(1)).await;
    let mut d = connect(port2, &note, steps(999, "probe", 8)).await;
    let d_seen = read_until(&mut d, |m| tag(m) == Some(2)).await;
    assert_eq!(
        d_seen.last().unwrap()[1][0]["marker"],
        "c1",
        "D was caught up with C's step over the wire"
    );
    d.close(None).await.unwrap();
    c.close(None).await.unwrap();

    let events2 = wait_for_events(&collab2, 2).await;
    let log2 = NoteCheckpoints::from_events(events2).unwrap();
    let record2 = log2.record(note).unwrap().unwrap();
    assert_eq!(
        record2.revisions.len(),
        2,
        "the restart chained a second revision"
    );
    assert_eq!(
        record2.heads.len(),
        1,
        "still a single head — no fork, no merge needed"
    );
    assert_eq!(
        record2.revisions[1].parents,
        vec![record2.revisions[0].id],
        "the second revision builds on the rebuilt head"
    );
    assert_eq!(
        record2.revisions[1].content, created_content,
        "same seeded document, same checkpoint content across the restart"
    );
    server2.abort();
}
