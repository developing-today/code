//! Wire-level integration tests for durable note checkpoints
//! (#3819 Phase 3, slice 4): real WebSocket clients against the real
//! router. Publication is client-driven: only an explicit `Checkpoint`
//! frame records a revision; steps and cursors stay ephemeral. A
//! restart rebuilds the log from the persisted snapshot and the next
//! checkpoint chains onto the rebuilt head. Concurrent offline heads
//! stay separate until an explicit merge names them as parents.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{path::PathBuf, sync::Arc, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh_blobs::{api::Store, store::mem::MemStore};
use iroh_docs::protocol::Docs;
use iroh_gossip::net::Gossip;
use tokio_tungstenite::tungstenite::Message;

use id_store::note_checkpoints::NoteCheckpoints;
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
            store,
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

/// Serves the real router on an ephemeral loopback port.
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

/// Connects to the note session. The first frame is a non-auth
/// message, which ends the auth wait and is processed after Init.
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

fn cursor(client_id: u64, head: u32) -> serde_json::Value {
    serde_json::json!([4, client_id, head, head, null])
}

fn checkpoint(text: &str) -> serde_json::Value {
    serde_json::json!([10, text])
}

fn tag(value: &serde_json::Value) -> Option<u64> {
    value.as_array()?.first()?.as_u64()
}

/// Reads frames until `pred` matches a decoded message; returns every
/// message seen along the way. Panics on a 15s drought.
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

/// Sends a checkpoint over the wire and returns the reported head hex.
async fn publish(ws: &mut Ws, text: &str) -> String {
    ws.send(Message::Binary(encode(&checkpoint(text)).into()))
        .await
        .unwrap();
    let seen = read_until(ws, |m| tag(m) == Some(11) || tag(m) == Some(5)).await;
    let last = seen.last().unwrap();
    assert_eq!(tag(last), Some(11), "expected Checkpointed, got {last:?}");
    last[1].as_str().unwrap().to_owned()
}

async fn wait_for_events(collab: &CollabState, count: usize) -> NoteCheckpoints {
    for _ in 0..100 {
        let events = collab.note_events().await;
        if events.len() >= count {
            return NoteCheckpoints::from_events(events).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("checkpoint log never reached {count} events");
}

fn snapshot_path(temp_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "note-checkpoints-wire-{temp_name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(id_web::NOTE_CHECKPOINT_SNAPSHOT_FILE_NAME)
}

#[tokio::test]
async fn note_checkpoint_wire_records_signed_revisions_and_keeps_steps_ephemeral() {
    let note = Hash::from_bytes(NOTE_BYTES);
    let fixture = Fixture::new("wire-ephemeral", [9; 32]).await;
    let collab = fixture.state.collab.clone();
    let author = fixture.state.identity.author_key();
    collab
        .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
        .await;
    let (port, server) = serve(fixture.state).await;

    let mut ws = connect(port, &note, steps(0, "e1", 42)).await;
    read_until(&mut ws, |m| tag(m) == Some(3)).await;

    // Editor steps and cursor moves are broadcast, never recorded.
    for i in 0..5u32 {
        ws.send(Message::Binary(
            encode(&steps(1 + u64::from(i), "e", 42)).into(),
        ))
        .await
        .unwrap();
        ws.send(Message::Binary(encode(&cursor(42, i)).into()))
            .await
            .unwrap();
    }
    read_until(&mut ws, |m| tag(m) == Some(3) && m[1].as_u64() == Some(5)).await;
    assert!(
        collab.note_events().await.is_empty(),
        "steps and cursors must not emit durable events"
    );

    let first = publish(&mut ws, "# Checked once").await;
    assert_eq!(first.len(), 64, "head is a hex hash: {first}");
    let log = wait_for_events(&collab, 1).await;
    let record = log.record(note).unwrap().expect("note recorded");
    assert_eq!(record.revisions.len(), 1);
    assert_eq!(record.heads.len(), 1);
    assert!(record.revisions[0].parents.is_empty(), "first is the create");
    assert_eq!(
        record.revisions[0].author, author,
        "the revision is signed by the serving identity"
    );

    let second = publish(&mut ws, "# Checked twice").await;
    let log = wait_for_events(&collab, 2).await;
    let record = log.record(note).unwrap().unwrap();
    assert_eq!(record.revisions.len(), 2);
    assert_eq!(record.heads.len(), 1, "a single head after a chained save");
    assert_eq!(
        record.revisions[1].parents,
        vec![record.revisions[0].id],
        "the second revision names the first as its parent"
    );
    assert_ne!(first, second);

    ws.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn note_checkpoint_wire_restart_rebuilds_from_snapshot_and_chains_onto_head() {
    let note = Hash::from_bytes(NOTE_BYTES);
    let path = snapshot_path("restart");
    let _ = std::fs::remove_file(&path);

    // Phase 1: a live host with a persisted snapshot publishes one checkpoint.
    let mut fixture = Fixture::new("wire-restart-one", [9; 32]).await;
    fixture.state.collab = Arc::new(CollabState::new().with_note_snapshot_path(path.clone()));
    let collab = fixture.state.collab.clone();
    collab
        .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
        .await;
    let (port, server) = serve(fixture.state).await;
    let mut ws = connect(port, &note, steps(0, "r1", 7)).await;
    read_until(&mut ws, |m| tag(m) == Some(3)).await;
    let head_before = publish(&mut ws, "# Before restart").await;
    ws.close(None).await.unwrap();
    server.abort();
    let snapshot_before = collab.note_snapshot().await.unwrap();

    // Phase 2: a fresh host restores the snapshot file on startup.
    let mut fixture2 = Fixture::new("wire-restart-two", [8; 32]).await;
    let restored = Arc::new(CollabState::new().with_note_snapshot_path(path.clone()));
    assert!(
        restored.restore_persisted_note_snapshot().await.unwrap(),
        "the snapshot file is restored"
    );
    assert_eq!(
        restored.note_snapshot().await.unwrap(),
        snapshot_before,
        "events and policies rebuild identically"
    );
    assert_eq!(
        restored.note_heads(note).await.unwrap(),
        vec![Hash::from_bytes(decode_hex(&head_before))]
    );
    restored
        .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
        .await;
    fixture2.state.collab = restored.clone();
    let (port2, server2) = serve(fixture2.state).await;

    // A post-restart session chains its revision onto the rebuilt head.
    let mut ws2 = connect(port2, &note, steps(0, "r2", 8)).await;
    read_until(&mut ws2, |m| tag(m) == Some(3)).await;
    let head_after = publish(&mut ws2, "# After restart").await;
    ws2.close(None).await.unwrap();
    server2.abort();

    let log = wait_for_events(&restored, 2).await;
    let record = log.record(note).unwrap().unwrap();
    assert_eq!(record.revisions.len(), 2);
    assert_eq!(record.heads.len(), 1, "single head, no fork");
    assert_eq!(
        record.revisions[1].parents,
        vec![Hash::from_bytes(decode_hex(&head_before))],
        "the post-restart revision names the rebuilt head as its parent"
    );
    assert_eq!(record.heads, vec![Hash::from_bytes(decode_hex(&head_after))]);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn note_checkpoint_offline_heads_stay_separate_until_an_explicit_merge_names_parents() {
    use id_store::note_checkpoints::{GroupRoles, NoteRole};
    use id_store::documents::DocumentRef;

    let note = Hash::from_bytes(NOTE_BYTES);
    let base_events = {
        let base = Fixture::new("wire-offline-base", [9; 32]).await;
        let (port, server) = serve(base.state.clone()).await;
        base.state
            .collab
            .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
            .await;
        let mut ws = connect(port, &note, steps(0, "b", 1)).await;
        read_until(&mut ws, |m| tag(m) == Some(3)).await;
        publish(&mut ws, "# Shared base").await;
        ws.close(None).await.unwrap();
        server.abort();
        base.state.collab.note_events().await
    };

    // Two hosts that were offline from each other both start from the base.
    let mut heads = Vec::new();
    for name in ["wire-offline-a", "wire-offline-b"] {
        let fixture = Fixture::new(name, [9; 32]).await;
        fixture.state.collab.seed_note_events(base_events.clone()).await.unwrap();
        fixture
            .state
            .collab
            .get_or_create_for_note(&note, "", Some(SEED_TEXT), Some("wire.md"))
            .await;
        let (port, server) = serve(fixture.state.clone()).await;
        let mut ws = connect(port, &note, steps(0, name, 1)).await;
        read_until(&mut ws, |m| tag(m) == Some(3)).await;
        let head = publish(&mut ws, &format!("# Offline {name}")).await;
        heads.push((fixture.state.collab.clone(), head));
        ws.close(None).await.unwrap();
        server.abort();
        std::mem::forget(fixture);
    }

    // Each host recorded its own head; they never joined silently.
    let (log_a, head_a) = (heads[0].0.clone(), heads[0].1.clone());
    let (log_b, head_b) = (heads[1].0.clone(), heads[1].1.clone());
    assert_ne!(head_a, head_b);
    let mut merged_events = log_a.note_events().await;
    let events_b = log_b.note_events().await;
    let base_len = base_events.len();
    for event in events_b.into_iter().skip(base_len) {
        merged_events.push(event);
    }
    let merged = NoteCheckpoints::from_events(merged_events.clone()).unwrap();
    let heads_now = merged.heads(note).unwrap();
    assert_eq!(heads_now.len(), 2, "concurrent offline heads stay separate");

    // The explicit merge names both heads as its parents; nothing is lost.
    let mut merged = merged;
    let content = DocumentRef::for_bytes("text/markdown", b"# Merged").unwrap();
    let merge_event = Hash::from_bytes([77u8; 32]);
    let author = [5u8; 32];
    let record = merged
        .merge_checked(note, merge_event, author, content, &GroupRoles::new(|_, _| None))
        .unwrap();
    assert_eq!(record.heads, vec![merge_event]);
    let merge_rev = record
        .revisions
        .iter()
        .find(|r| r.id == merge_event)
        .unwrap();
    let mut expected = heads_now.clone();
    expected.sort();
    assert_eq!(merge_rev.parents, expected, "the merge names every head");
    let _ = (NoteRole::Write,);
}

fn decode_hex(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}
