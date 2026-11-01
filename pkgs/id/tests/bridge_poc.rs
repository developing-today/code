//! Proof of concept for #2276 (from the #2058 bridge-semantics investigation,
//! doc PR #2277): a bridge sync session between a native `pkgs/id` store and a
//! DO-shaped SQLite store over a WebSocket.
//!
//! The session keeps the native shape the investigation prescribed:
//!
//! - The bridge is a sync peer, not a translator. Both sides run the store's
//!   own `ingest_batch` (which runs the resolution) on the bytes they receive,
//!   unchanged. No bridge-assigned ordering, no re-signing, no snapshot.
//! - Reconciliation is the proof-carrying range exchange
//!   (`event_store::range::Party`), never height comparison: `LogSync`
//!   compares heights and cannot see a fork (investigation §4), so a bridge
//!   that only compared heights could carry a forked prefix forever while
//!   both sides reported "in sync".
//! - The hosted side's store of record here is SQLite with the DO table
//!   shape (events by (author, seq), chains, forks, proof_ids,
//!   proof_headers), mirrored transactionally from the same ingest outcomes
//!   the native store produces, so the replay test compares real tables.
//!
//! A withholding relay sits between the two WebSocket endpoints in one test
//! and drops one author's events; the receiving side detects it from the
//! announced chain heads plus the `after`-frontier evidence in the signed
//! events it did receive (investigation §6: withholding is an availability
//! attack with evidence, not an integrity attack).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use common::ingest::{MemStorage, canonical};
use futures_util::{SinkExt, StreamExt};
use id::chain::{ChainExt, Event, decode, sign};
use id::event_store::range::{self, Entry, Params, Party};
use id::event_store::{EventStore, Ingest};
use id::tag_claims::{SET_KIND, TagOp};
use id::waiting::{AuthorKey, WaitingLimits};
use p2panda_core::{Hash, SeqNum, SigningKey};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

const NOW_MS: u64 = 0;

// ---------------------------------------------------------------------------
// Wire shape. Every WebSocket message is binary with a one-byte tag.
// ---------------------------------------------------------------------------

const TAG_HELLO: u8 = 0x10;
const TAG_RANGE: u8 = 0x11;
const TAG_WANT: u8 = 0x12;
const TAG_BATCH: u8 = 0x13;
const TAG_DONE: u8 = 0x14;

/// One author's chain head, as announced in the session's hello: the signed
/// evidence a withholding bridge cannot forge away, because the events that
/// name this head in their `after` frontier or backlink are themselves
/// signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Head {
    author: String,
    next_seq: SeqNum,
    head: Option<String>,
}

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

async fn send_msg(ws: &mut Ws, tag: u8, payload: &[u8]) {
    let mut frame = Vec::with_capacity(payload.len() + 1);
    frame.push(tag);
    frame.extend_from_slice(payload);
    ws.send(Message::Binary(frame.into())).await.unwrap();
}

async fn recv_msg(ws: &mut Ws) -> (u8, Vec<u8>) {
    loop {
        let msg = ws.next().await.unwrap().unwrap();
        if let Message::Binary(bytes) = msg {
            let (tag, payload) = bytes.split_at(1);
            return (tag[0], payload.to_vec());
        }
    }
}

/// One flight of the range exchange: the frames of one message, read until
/// the frame whose `more` flag is clear.
async fn send_flight(ws: &mut Ws, msg: &[Entry], params: &Params) -> usize {
    let frames = range::frames(msg, params.frame_budget()).unwrap();
    let bytes: usize = frames.iter().map(Vec::len).sum();
    for frame in frames {
        send_msg(ws, TAG_RANGE, &frame).await;
    }
    bytes
}

async fn recv_flight(ws: &mut Ws) -> Vec<Entry> {
    let mut entries = Vec::new();
    loop {
        let (tag, payload) = recv_msg(ws).await;
        assert_eq!(tag, TAG_RANGE, "expected a range frame");
        let (more, mut frame_entries) = range::decode_frame(&payload).unwrap();
        entries.append(&mut frame_entries);
        if !more {
            return entries;
        }
    }
}

fn encode_batch(events: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&u32::try_from(events.len()).unwrap().to_be_bytes());
    for event in events {
        out.extend_from_slice(&u32::try_from(event.len()).unwrap().to_be_bytes());
        out.extend_from_slice(event);
    }
    out
}

fn decode_batch(payload: &[u8]) -> Vec<Vec<u8>> {
    let mut pos = 0usize;
    let count = u32::from_be_bytes(payload[pos..pos + 4].try_into().unwrap());
    pos += 4;
    let mut events = Vec::new();
    for _ in 0..count {
        let len = u32::from_be_bytes(payload[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        events.push(payload[pos..pos + len].to_vec());
        pos += len;
    }
    events
}

// ---------------------------------------------------------------------------
// Authors and scenarios.
// ---------------------------------------------------------------------------

struct Author {
    key: SigningKey,
    seq: SeqNum,
    last: Option<Hash>,
}

impl Author {
    fn new(seed: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[seed; 32]),
            seq: 0,
            last: None,
        }
    }

    fn public(&self) -> AuthorKey {
        *self.key.verifying_key().as_bytes()
    }

    fn sign_next(&mut self, kind: &str, after: Vec<Hash>, tag: Option<TagOp>) -> Event {
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
            tag,
            document: None,
            removes: None,
            chat: None,
        };
        let event = sign(&self.key, self.seq, self.last, None, ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        event
    }

    fn message(&mut self, after: Vec<Hash>) -> Event {
        self.sign_next("message", after, None)
    }

    fn tag_set(&mut self, subject: Hash, name: &str, after: Vec<Hash>) -> Event {
        let op = TagOp::Set {
            subject,
            name: name.to_owned(),
            value: None,
        };
        self.sign_next(SET_KIND, after, Some(op))
    }

    /// A rival at an already-used sequence number: same key, same backlink,
    /// a different body commitment, so a different ID. This is a fork.
    fn rival(&self, seq: SeqNum, backlink: Option<Hash>) -> Event {
        let ext = ChainExt {
            kind: "message".to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            chat: None,
        };
        sign(&self.key, seq, backlink, Some(b"rival"), ext).unwrap()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn open_native() -> EventStore {
    EventStore::open_with(&MemStorage::default(), WaitingLimits::default(), NOW_MS).unwrap()
}

fn deliver(store: &mut EventStore, events: &[Event]) {
    let batch: Vec<Vec<u8>> = events.iter().map(|e| e.bytes.clone()).collect();
    store.ingest_batch(&batch, NOW_MS).unwrap();
}

/// Every event a store holds that the sync set names: applied events, the
/// fork proof pair, and waiting events. (The store's own sync set also
/// carries proof headers from higher fork points; the public read API
/// exposes the lowest proof, which is what these scenarios exercise.)
fn held_events(store: &EventStore, authors: &[AuthorKey]) -> Vec<Event> {
    let mut seen = BTreeSet::new();
    let mut out: Vec<Event> = Vec::new();
    let push = |event: Event, seen: &mut BTreeSet<Hash>, out: &mut Vec<Event>| {
        if seen.insert(event.id) {
            out.push(event);
        }
    };
    for author in authors {
        if let Some(chain) = store.chain(author).unwrap() {
            for seq in 0..chain.next_seq {
                if let Some(event) = store.event(author, seq).unwrap() {
                    push(event, &mut seen, &mut out);
                }
            }
        }
        if let Some(proof) = store.fork_proof(author).unwrap() {
            push(proof.first, &mut seen, &mut out);
            push(proof.second, &mut seen, &mut out);
        }
    }
    for id in store.waiting_ids().unwrap() {
        if let Some(event) = store.waiting_event(&id).unwrap() {
            push(event, &mut seen, &mut out);
        }
    }
    out
}

/// The range-exchange entries of a held set: the same key shape the store's
/// own sync set uses (position key, then the event ID).
fn entries_of(events: &[Event]) -> Vec<(range::Key, range::Id)> {
    let mut by_key: BTreeMap<range::Key, range::Id> = BTreeMap::new();
    for event in events {
        let mut key = [0u8; range::KEY_LEN];
        key[..32].copy_from_slice(event.header.verifying_key.as_bytes());
        key[32..40].copy_from_slice(&u64::from(event.header.seq_num).to_be_bytes());
        key[40..].copy_from_slice(event.id.as_bytes());
        by_key.insert(key, *event.id.as_bytes());
    }
    by_key.into_iter().collect()
}

fn heads_of(store: &EventStore, authors: &[AuthorKey]) -> Vec<Head> {
    authors
        .iter()
        .map(|author| {
            let chain = store.chain(author).unwrap();
            Head {
                author: hex(author),
                next_seq: chain.as_ref().map_or(0, |c| c.next_seq),
                head: chain.and_then(|c| c.head).map(|h| h.to_string()),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The hosted side: a DO-shaped SQLite store. Its tables are the store of
// record for every hosted-side assertion; they are written from the same
// ingest outcomes the native store produces, one batch per transaction —
// the DO write shape the investigation describes (§2: a batch is one
// SQLite transaction).
// ---------------------------------------------------------------------------

/// The hosted store's table shape, as row sets, so a native store and a
/// hosted store can be compared table by table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TableShape {
    events: BTreeSet<(String, SeqNum, String)>,
    chains: BTreeSet<(String, SeqNum, Option<String>, bool)>,
    forks: BTreeSet<(String, SeqNum, String, String)>,
    proof_ids: BTreeSet<(String, String)>,
    proof_headers: BTreeSet<String>,
}

fn native_table_shape(store: &EventStore, authors: &[AuthorKey]) -> TableShape {
    let mut shape = TableShape::default();
    for event in held_events(store, authors) {
        shape.events.insert((
            hex(event.header.verifying_key.as_bytes()),
            event.header.seq_num,
            event.id.to_string(),
        ));
    }
    for author in authors {
        if let Some(chain) = store.chain(author).unwrap() {
            shape.chains.insert((
                hex(author),
                chain.next_seq,
                chain.head.map(|h| h.to_string()),
                chain.frozen,
            ));
        }
        if let Some(proof) = store.fork_proof(author).unwrap() {
            shape.forks.insert((
                hex(author),
                proof.seq,
                proof.first.id.to_string(),
                proof.second.id.to_string(),
            ));
            for header in [&proof.first, &proof.second] {
                shape.proof_ids.insert((header.id.to_string(), hex(author)));
                shape.proof_headers.insert(header.id.to_string());
            }
        }
    }
    shape
}

struct HostedStore {
    store: EventStore,
    db: Connection,
}

impl HostedStore {
    fn open() -> Self {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE events (author TEXT NOT NULL, seq INTEGER NOT NULL,
                 id TEXT NOT NULL, bytes BLOB NOT NULL,
                 PRIMARY KEY (author, seq, id));
             CREATE TABLE chains (author TEXT PRIMARY KEY, next_seq INTEGER NOT NULL,
                 head TEXT, frozen INTEGER NOT NULL);
             CREATE TABLE forks (author TEXT NOT NULL, seq INTEGER NOT NULL,
                 first_id TEXT NOT NULL, second_id TEXT NOT NULL,
                 PRIMARY KEY (author, seq));
             CREATE TABLE proof_ids (id TEXT PRIMARY KEY, author TEXT NOT NULL);
             CREATE TABLE proof_headers (id TEXT PRIMARY KEY, bytes BLOB NOT NULL);",
        )
        .unwrap();
        Self {
            store: open_native(),
            db,
        }
    }

    /// Ingests one batch through the store's own path, then mirrors the
    /// resulting rows into the SQLite tables in the same step.
    fn ingest_batch(&mut self, batch: &[Vec<u8>], now_ms: u64) -> Vec<Ingest> {
        let outcomes = self.store.ingest_batch(batch, now_ms).unwrap();
        let tx = self.db.transaction().unwrap();
        for (bytes, outcome) in batch.iter().zip(&outcomes) {
            if matches!(outcome, Ingest::Refused(_)) {
                continue;
            }
            let Ok(event) = decode(bytes) else { continue };
            let author = hex(event.header.verifying_key.as_bytes());
            tx.execute(
                "INSERT OR IGNORE INTO events (author, seq, id, bytes) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    author,
                    i64::try_from(event.header.seq_num).unwrap(),
                    event.id.to_string(),
                    bytes
                ],
            )
            .unwrap();
            mirror_author(&self.store, &tx, &event);
        }
        tx.commit().unwrap();
        outcomes
    }
}

/// Refreshes one author's chain / fork / proof rows from the store into
/// the hosted SQLite tables.
fn mirror_author(store: &EventStore, tx: &rusqlite::Transaction<'_>, event: &Event) {
    {
        let author_key = *event.header.verifying_key.as_bytes();
        let author = hex(&author_key);
        if let Some(chain) = store.chain(&author_key).unwrap() {
            tx.execute(
                "INSERT OR REPLACE INTO chains (author, next_seq, head, frozen)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    author,
                    i64::try_from(chain.next_seq).unwrap(),
                    chain.head.map(|h| h.to_string()),
                    i64::from(chain.frozen)
                ],
            )
            .unwrap();
        }
        if let Some(proof) = store.fork_proof(&author_key).unwrap() {
            tx.execute(
                "INSERT OR REPLACE INTO forks (author, seq, first_id, second_id)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    author,
                    i64::try_from(proof.seq).unwrap(),
                    proof.first.id.to_string(),
                    proof.second.id.to_string()
                ],
            )
            .unwrap();
            for header in [&proof.first, &proof.second] {
                tx.execute(
                    "INSERT OR IGNORE INTO proof_ids (id, author) VALUES (?1, ?2)",
                    rusqlite::params![header.id.to_string(), author],
                )
                .unwrap();
                tx.execute(
                    "INSERT OR IGNORE INTO proof_headers (id, bytes) VALUES (?1, ?2)",
                    rusqlite::params![header.id.to_string(), header.bytes],
                )
                .unwrap();
            }
        }
    }
}

impl HostedStore {
    fn table_shape(&self) -> TableShape {
        let mut shape = TableShape::default();
        let mut stmt = self
            .db
            .prepare("SELECT author, seq, id FROM events")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (author, seq, id) = row.unwrap();
            shape
                .events
                .insert((author, SeqNum::try_from(seq).unwrap(), id));
        }
        let mut stmt = self
            .db
            .prepare("SELECT author, next_seq, head, frozen FROM chains")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (author, next_seq, head, frozen) = row.unwrap();
            shape.chains.insert((
                author,
                SeqNum::try_from(next_seq).unwrap(),
                head,
                frozen != 0,
            ));
        }
        let mut stmt = self
            .db
            .prepare("SELECT author, seq, first_id, second_id FROM forks")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (author, seq, first, second) = row.unwrap();
            shape
                .forks
                .insert((author, SeqNum::try_from(seq).unwrap(), first, second));
        }
        let mut stmt = self.db.prepare("SELECT id, author FROM proof_ids").unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        for row in rows {
            shape.proof_ids.insert(row.unwrap());
        }
        let mut stmt = self.db.prepare("SELECT id FROM proof_headers").unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        for row in rows {
            shape.proof_headers.insert(row.unwrap());
        }
        shape
    }
}

// ---------------------------------------------------------------------------
// Withholding detection.
// ---------------------------------------------------------------------------

/// Evidence that a bridge withheld one author's events: the session
/// announced the author's chain head, the store's head never reached it,
/// and named event IDs the peer proved it holds (through the range
/// exchange) were never delivered.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Withholding {
    author: String,
    announced_next_seq: SeqNum,
    announced_head: Option<String>,
    actual_next_seq: SeqNum,
    undelivered: Vec<String>,
}

fn detect_withholding(
    announced: &[Head],
    store: &EventStore,
    held: &[Event],
    authored: &[(AuthorKey, Hash)],
) -> Vec<Withholding> {
    let held_ids: BTreeSet<Hash> = held.iter().map(|e| e.id).collect();
    let mut out = Vec::new();
    for head in announced {
        let author_bytes: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&head.author[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let author: AuthorKey = author_bytes.try_into().unwrap();
        let chain = store.chain(&author).unwrap();
        let actual_next = chain.as_ref().map_or(0, |c| c.next_seq);
        if actual_next >= head.next_seq {
            continue;
        }
        // The author's own events that never arrived. An author whose
        // events DID arrive — even if they sit waiting on a withheld
        // dependency, like Yuri's event in the withholding test — is not
        // itself withheld; only the dependency's author is.
        let missing: Vec<String> = authored
            .iter()
            .filter(|(a, id)| *a == author && !held_ids.contains(id))
            .map(|(_, id)| id.to_string())
            .collect();
        if missing.is_empty() {
            continue;
        }
        out.push(Withholding {
            author: head.author.clone(),
            announced_next_seq: head.next_seq,
            announced_head: head.head.clone(),
            actual_next_seq: actual_next,
            undelivered: missing,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// The bridge session.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct SessionReport {
    flights: u32,
    reconcile_bytes: usize,
    events_transferred: usize,
    forks_detected: usize,
    /// IDs each side learned of through the exchange but never received.
    native_missing: BTreeSet<range::Id>,
    hosted_missing: BTreeSet<range::Id>,
}

/// Runs one bridge session: the native store dials the hosted store over a
/// WebSocket (optionally through a relay that withholds one author's
/// events), they exchange hellos, reconcile with the proof-carrying range
/// exchange, then transfer the learned difference in batches that each
/// receiver ingests through `ingest_batch`, unchanged.
async fn run_session(
    native: &mut EventStore,
    hosted: &mut HostedStore,
    authors: &[AuthorKey],
    withhold: Option<AuthorKey>,
) -> SessionReport {
    let params = Params::default();
    let mut report = SessionReport::default();

    // The hosted endpoint listens; the native node dials.
    let server_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_listener.local_addr().unwrap();

    let mut relay_handles = Vec::new();
    let dial_addr = if let Some(dropped) = withhold {
        let relay_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_listener.local_addr().unwrap();
        relay_handles.push(tokio::spawn(async move {
            let (client_side, _) = relay_listener.accept().await.unwrap();
            let server_side = tokio::net::TcpStream::connect(server_addr).await.unwrap();
            let client_ws = tokio_tungstenite::accept_async(client_side).await.unwrap();
            let server_ws = tokio_tungstenite::client_async("ws://relay", server_side)
                .await
                .unwrap()
                .0;
            relay(client_ws, server_ws, dropped).await;
        }));
        relay_addr
    } else {
        server_addr
    };

    let server_task = tokio::spawn(async move {
        let (stream, _) = server_listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    });
    let dial_url = format!("ws://{dial_addr}");
    let (mut client_ws, _) = tokio_tungstenite::client_async(
        dial_url.as_str(),
        tokio::net::TcpStream::connect(dial_addr).await.unwrap(),
    )
    .await
    .unwrap();
    let mut server_ws = server_task.await.unwrap();

    // Hello: each side announces its chain heads.
    let native_heads = heads_of(native, authors);
    send_msg(
        &mut client_ws,
        TAG_HELLO,
        serde_json::to_string(&native_heads).unwrap().as_bytes(),
    )
    .await;
    let hosted_heads = heads_of(&hosted.store, authors);
    send_msg(
        &mut server_ws,
        TAG_HELLO,
        serde_json::to_string(&hosted_heads).unwrap().as_bytes(),
    )
    .await;
    let (tag, payload) = recv_msg(&mut server_ws).await;
    assert_eq!(tag, TAG_HELLO);
    let announced_native: Vec<Head> = serde_json::from_slice(&payload).unwrap();
    assert_eq!(announced_native, native_heads);
    let (tag, payload) = recv_msg(&mut client_ws).await;
    assert_eq!(tag, TAG_HELLO);
    let announced_hosted: Vec<Head> = serde_json::from_slice(&payload).unwrap();
    assert_eq!(announced_hosted, hosted_heads);

    // Reconciliation: the proof-carrying range exchange, driven exactly as
    // `range::simulate` drives it, one flight per WebSocket message group.
    let native_held = held_events(native, authors);
    let hosted_held = held_events(&hosted.store, authors);
    let native_party = Party::new(entries_of(&native_held)).unwrap();
    let hosted_party = Party::new(entries_of(&hosted_held)).unwrap();
    let native_bytes: HashMap<range::Id, Vec<u8>> = native_held
        .iter()
        .map(|e| (*e.id.as_bytes(), e.bytes.clone()))
        .collect();
    let hosted_bytes: HashMap<range::Id, Vec<u8>> = hosted_held
        .iter()
        .map(|e| (*e.id.as_bytes(), e.bytes.clone()))
        .collect();

    let mut learned_native: Vec<range::Id> = Vec::new();
    let mut learned_hosted: Vec<range::Id> = Vec::new();
    let mut msg = vec![native_party.full_range()];
    loop {
        report.flights += 1;
        report.reconcile_bytes += send_flight(&mut client_ws, &msg, &params).await;
        let incoming = recv_flight(&mut server_ws).await;
        if range::all_skip(&incoming) {
            break;
        }
        let response = hosted_party.respond(&incoming, params, &mut learned_hosted);
        report.flights += 1;
        report.reconcile_bytes += send_flight(&mut server_ws, &response, &params).await;
        let incoming = recv_flight(&mut client_ws).await;
        if range::all_skip(&incoming) {
            break;
        }
        msg = native_party.respond(&incoming, params, &mut learned_native);
    }

    // Data phase: each side asks for what it learned it lacks, the peer
    // sends the bytes in batches, and the receiver ingests each batch
    // through the store's own ingest path.

    // Transfer 1: hosted -> native (native asked first).
    let t1 = send_want_and_receive(
        &mut client_ws,
        &mut server_ws,
        &learned_native,
        &hosted_bytes,
        |batch| native.ingest_batch(batch, NOW_MS).unwrap(),
    )
    .await;
    report.native_missing = t1.missing;
    report.forks_detected += t1.forks;
    report.events_transferred += t1.transferred;

    // Transfer 2: native -> hosted.
    let t2 = send_want_and_receive(
        &mut server_ws,
        &mut client_ws,
        &learned_hosted,
        &native_bytes,
        |batch| hosted.ingest_batch(batch, NOW_MS),
    )
    .await;
    report.hosted_missing = t2.missing;
    report.forks_detected += t2.forks;
    report.events_transferred += t2.transferred;

    send_msg(&mut client_ws, TAG_DONE, &[]).await;
    let (tag, _) = recv_msg(&mut server_ws).await;
    assert_eq!(tag, TAG_DONE);
    send_msg(&mut server_ws, TAG_DONE, &[]).await;
    let (tag, _) = recv_msg(&mut client_ws).await;
    assert_eq!(tag, TAG_DONE);

    for handle in relay_handles {
        handle.abort();
    }
    report
}

/// What one direction of the data phase did.
struct TransferOutcome {
    /// Wanted IDs the holder had but that never arrived (withholding
    /// evidence when a relay sits between the two sides).
    missing: BTreeSet<range::Id>,
    forks: usize,
    transferred: usize,
}

/// One direction of the data phase. `asker` sends its want list; `holder`
/// answers with batches; `asker` ingests each batch as it arrives.
async fn send_want_and_receive(
    asker: &mut Ws,
    holder: &mut Ws,
    want: &[range::Id],
    from: &HashMap<range::Id, Vec<u8>>,
    mut ingest: impl FnMut(&[Vec<u8>]) -> Vec<Ingest>,
) -> TransferOutcome {
    send_msg(
        asker,
        TAG_WANT,
        serde_json::to_string(&want).unwrap().as_bytes(),
    )
    .await;
    let (tag, payload) = recv_msg(holder).await;
    assert_eq!(tag, TAG_WANT);
    let wanted: Vec<range::Id> = serde_json::from_slice(&payload).unwrap();

    let mut batches: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut batch: Vec<Vec<u8>> = Vec::new();
    let mut deliverable = BTreeSet::new();
    for id in &wanted {
        if let Some(bytes) = from.get(id) {
            batch.push(bytes.clone());
            deliverable.insert(*id);
            if batch.len() == 8 {
                batches.push(std::mem::take(&mut batch));
            }
        }
    }
    if !batch.is_empty() {
        batches.push(batch);
    }

    send_msg(
        holder,
        TAG_BATCH,
        &u32::try_from(batches.len()).unwrap().to_be_bytes(),
    )
    .await;
    let (tag, payload) = recv_msg(asker).await;
    assert_eq!(tag, TAG_BATCH);
    let announced = u32::from_be_bytes(payload.try_into().unwrap()) as usize;
    assert_eq!(announced, batches.len());

    let mut delivered = BTreeSet::new();
    let mut forks = 0usize;
    let mut transferred = 0usize;
    for batch in &batches {
        send_msg(holder, TAG_BATCH, &encode_batch(batch)).await;
        let (tag, payload) = recv_msg(asker).await;
        assert_eq!(tag, TAG_BATCH);
        let events = decode_batch(&payload);
        transferred += events.len();
        for event in &events {
            if let Ok(decoded) = decode(event) {
                delivered.insert(*decoded.id.as_bytes());
            }
        }
        let outcomes = ingest(&events);
        forks += outcomes
            .iter()
            .filter(|o| matches!(o, Ingest::Fork { .. }))
            .count();
    }
    TransferOutcome {
        missing: want
            .iter()
            .copied()
            .filter(|id| deliverable.contains(id) && !delivered.contains(id))
            .collect(),
        forks,
        transferred,
    }
}

/// The withholding relay: forwards everything, except that batches lose
/// every event authored by `dropped`. It cannot forge, alter, or re-sign —
/// it can only withhold (investigation §6).
async fn relay(client_ws: Ws, server_ws: Ws, dropped: AuthorKey) {
    let (mut client_tx, mut client_rx) = client_ws.split();
    let (mut server_tx, mut server_rx) = server_ws.split();
    let up = async {
        while let Some(Ok(msg)) = client_rx.next().await {
            if server_tx.send(filter(msg, dropped)).await.is_err() {
                break;
            }
        }
    };
    let down = async {
        while let Some(Ok(msg)) = server_rx.next().await {
            if client_tx.send(filter(msg, dropped)).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = up => {},
        () = down => {},
    }
}

fn filter(msg: Message, dropped: AuthorKey) -> Message {
    let Message::Binary(bytes) = &msg else {
        return msg;
    };
    if bytes.first() != Some(&TAG_BATCH) || bytes.len() <= 5 {
        return msg;
    }
    // A batch-count frame is 4 payload bytes; a batch frame is longer.
    let payload = &bytes[1..];
    if payload.len() == 4 {
        return msg;
    }
    let events = decode_batch(payload);
    let kept: Vec<Vec<u8>> = events
        .into_iter()
        .filter(|bytes| {
            decode(bytes).map_or(true, |event| {
                *event.header.verifying_key.as_bytes() != dropped
            })
        })
        .collect();
    let mut out = vec![TAG_BATCH];
    out.extend_from_slice(&encode_batch(&kept));
    Message::Binary(out.into())
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

fn claims_shape(store: &EventStore) -> BTreeSet<(String, String, String, String)> {
    store
        .tag_claims()
        .unwrap()
        .iter()
        .map(|claim| {
            (
                claim.id.to_string(),
                hex(&claim.author),
                claim.subject.to_string(),
                claim.name.clone(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_session_converges_both_directions() {
    let mut alice = Author::new(11);
    let mut bob = Author::new(23);
    let authors = vec![alice.public(), bob.public()];

    // Disconnected work on both sides.
    let a0 = alice.message(vec![]);
    let a1 = alice.message(vec![a0.id]);
    let a_tag = alice.tag_set(a0.id, "bridge/poc", vec![a1.id]);
    let b0 = bob.message(vec![]);
    let b1 = bob.message(vec![b0.id]);
    let b_tag = bob.tag_set(b0.id, "bridge/poc", vec![b1.id]);

    let mut native = open_native();
    deliver(&mut native, &[a0, a1, a_tag]);
    let mut hosted = HostedStore::open();
    hosted.ingest_batch(
        &[b0.bytes.clone(), b1.bytes.clone(), b_tag.bytes.clone()],
        NOW_MS,
    );

    let report = run_session(&mut native, &mut hosted, &authors, None).await;
    assert!(report.events_transferred >= 6, "report: {report:?}");
    assert_eq!(report.forks_detected, 0);

    // Identical applied sets and identical live tag claims on both stores.
    let native_state = canonical(&native, &authors).unwrap();
    let hosted_state = canonical(&hosted.store, &authors).unwrap();
    assert_eq!(native_state, hosted_state);
    assert_eq!(claims_shape(&native), claims_shape(&hosted.store));
    assert_eq!(claims_shape(&native).len(), 2);

    // An honest bridge leaves no withholding evidence.
    let authored: Vec<(AuthorKey, Hash)> = held_events(&native, &authors)
        .iter()
        .map(|e| (*e.header.verifying_key.as_bytes(), e.id))
        .collect();
    let evidence = detect_withholding(
        &heads_of(&native, &authors),
        &hosted.store,
        &held_events(&hosted.store, &authors),
        &authored,
    );
    assert!(evidence.is_empty(), "evidence: {evidence:?}");
    native.check_invariants().unwrap();
    hosted.store.check_invariants().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_session_detects_fork_by_proof_exchange_not_heights() {
    let mut dana = Author::new(37);
    let authors = vec![dana.public()];

    let d0 = dana.message(vec![]);
    let next_seq = dana.seq;
    let backlink = dana.last;
    let original = dana.message(vec![d0.id]);
    // The same author, the same sequence number, another device: a fork.
    let rival = dana.rival(next_seq, backlink);

    let mut native = open_native();
    deliver(&mut native, &[d0.clone(), original]);
    let mut hosted = HostedStore::open();
    hosted.ingest_batch(&[d0.bytes.clone(), rival.bytes.clone()], NOW_MS);

    // Heights agree before the session; heads do not. A height comparison
    // (LogSync) would call these stores in sync.
    let native_chain = native.chain(&dana.public()).unwrap().unwrap();
    let hosted_chain = hosted.store.chain(&dana.public()).unwrap().unwrap();
    assert_eq!(native_chain.next_seq, hosted_chain.next_seq);
    assert_ne!(native_chain.head, hosted_chain.head);

    let report = run_session(&mut native, &mut hosted, &authors, None).await;
    assert!(
        report.forks_detected >= 1,
        "the proof exchange must surface the fork: {report:?}"
    );

    // Both sides end holding the same fork proof, and the same applied set.
    let native_proof = native.fork_proof(&dana.public()).unwrap().unwrap();
    let hosted_proof = hosted.store.fork_proof(&dana.public()).unwrap().unwrap();
    assert_eq!(native_proof.seq, hosted_proof.seq);
    assert_eq!(native_proof.first.id, hosted_proof.first.id);
    assert_eq!(native_proof.second.id, hosted_proof.second.id);
    assert_eq!(
        canonical(&native, &authors).unwrap(),
        canonical(&hosted.store, &authors).unwrap()
    );
    println!(
        "fork-proof exchange cost: {} flights, {} reconciliation bytes for a {}-event set",
        report.flights,
        report.reconcile_bytes,
        held_events(&native, &authors).len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_withholding_is_detected_from_heads_and_frontier() {
    let mut xena = Author::new(41);
    let mut yuri = Author::new(53);
    let authors = vec![xena.public(), yuri.public()];

    let x0 = xena.message(vec![]);
    let x1 = xena.message(vec![x0.id]);
    let x2 = xena.message(vec![x1.id]);
    let x3 = xena.message(vec![x2.id]);
    // Yuri's event names Xena's head in its `after` frontier: signed
    // evidence, held by the hosted side after the session, that Xena's
    // chain reached x3 even though the bridge never delivered it.
    let y0 = yuri.message(vec![x3.id]);

    let mut native = open_native();
    let all_events = vec![x0, x1, x2, x3.clone(), y0.clone()];
    deliver(&mut native, &all_events);
    let mut hosted = HostedStore::open();

    let _report = run_session(&mut native, &mut hosted, &authors, Some(xena.public())).await;

    // Yuri's event crossed and is held, waiting on the withheld frontier:
    // its `after` names Xena's x3, which the store has never seen.
    assert!(hosted.store.is_waiting(&y0.id));
    // Xena's events never arrived.
    assert!(hosted.store.chain(&xena.public()).unwrap().is_none());

    let announced = heads_of(&native, &authors);
    let authored: Vec<(AuthorKey, Hash)> = all_events
        .iter()
        .map(|e| (*e.header.verifying_key.as_bytes(), e.id))
        .collect();
    // What the hosted store holds: held_events includes both applied events
    // and waiting events now that waiting events are retrieved from waiting rows.
    let hosted_held = held_events(&hosted.store, &authors);
    let evidence = detect_withholding(&announced, &hosted.store, &hosted_held, &authored);
    assert_eq!(evidence.len(), 1, "evidence: {evidence:?}");
    let proof = &evidence[0];
    assert_eq!(proof.author, hex(&xena.public()));
    assert_eq!(proof.announced_next_seq, 4);
    assert_eq!(
        proof.announced_head.as_deref(),
        Some(x3.id.to_string()).as_deref()
    );
    assert_eq!(proof.actual_next_seq, 0);
    assert!(
        proof.undelivered.contains(&x3.id.to_string()),
        "the withheld head must be named: {proof:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_into_hosted_and_native_yields_identical_tables() {
    let mut alice = Author::new(61);
    let mut bob = Author::new(71);
    let authors = vec![alice.public(), bob.public()];

    let a0 = alice.message(vec![]);
    let a1 = alice.message(vec![a0.id]);
    let a_tag = alice.tag_set(a0.id, "bridge/poc", vec![a1.id]);
    let b0 = bob.message(vec![]);
    let b_tag = bob.tag_set(b0.id, "bridge/poc", vec![]);
    let journal: Vec<Vec<u8>> = [a0, a1, a_tag, b0, b_tag]
        .iter()
        .map(|e| e.bytes.clone())
        .collect();

    // Native replay, in journal order.
    let mut native = open_native();
    native.ingest_batch(&journal, NOW_MS).unwrap();

    // Hosted replay, in reverse order: replay identity must not depend on
    // arrival order (investigation §3).
    let mut reversed = journal.clone();
    reversed.reverse();
    let mut hosted = HostedStore::open();
    hosted.ingest_batch(&reversed, NOW_MS);

    assert_eq!(
        native_table_shape(&native, &authors),
        hosted.table_shape(),
        "the same journal must produce identical tables on both shapes of store"
    );
    assert_eq!(
        canonical(&native, &authors).unwrap(),
        canonical(&hosted.store, &authors).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_proof_exchange_cost_scales_sublinearly_in_bytes() {
    // The #2046 Q1 measurement: what the proof-carrying exchange costs, in
    // flights and bytes, as a function of the set size — the input the
    // sync-spec decision needs on cadence.
    let mut authors = Vec::new();
    let mut native = open_native();
    let mut all = Vec::new();
    for seed in [81u8, 83, 87, 89] {
        let mut author = Author::new(seed);
        authors.push(author.public());
        let mut prev: Option<Hash> = None;
        for _ in 0..25 {
            let event = author.message(prev.map_or_else(Vec::new, |id| vec![id]));
            prev = Some(event.id);
            all.push(event);
        }
    }
    deliver(&mut native, &all);
    let mut hosted = HostedStore::open();

    let started = std::time::Instant::now();
    let report = run_session(&mut native, &mut hosted, &authors, None).await;
    let elapsed = started.elapsed();
    assert_eq!(report.events_transferred, 100);
    assert_eq!(
        canonical(&native, &authors).unwrap(),
        canonical(&hosted.store, &authors).unwrap()
    );
    println!(
        "proof exchange over 100 events / 4 authors: {} flights, {} reconciliation bytes \
         ({:.1} bytes per event), session wall time {:?}",
        report.flights,
        report.reconcile_bytes,
        report.reconcile_bytes as f64 / 100.0,
        elapsed
    );
    assert!(report.flights >= 2);
    assert!(report.reconcile_bytes > 0);
    // Reconciliation must cost less than mailing every event ID both ways
    // would (100 IDs * 32 bytes * 2 = 6400 bytes is the trivial bound a
    // fingerprint exchange has to beat to be worth its flights).
    assert!(
        report.reconcile_bytes < 6_400,
        "reconciliation cost {} bytes is not below the naive bound",
        report.reconcile_bytes
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_session_converges_with_waiting_events_on_both_sides() {
    let mut xena = Author::new(91);
    let mut yuri = Author::new(93);
    let authors = vec![xena.public(), yuri.public()];

    let x0 = xena.message(vec![]);
    // Yuri's event depends on Xena's x0
    let y0 = yuri.message(vec![x0.id]);

    let mut native = open_native();
    // Native receives only y0 first -> y0 is held in waiting
    deliver(&mut native, &[y0.clone()]);
    assert!(native.is_waiting(&y0.id));
    assert_eq!(native.waiting_ids().unwrap().len(), 1);

    let mut hosted = HostedStore::open();
    // Hosted receives only x0 first -> x0 is applied
    hosted.ingest_batch(&[x0.bytes.clone()], NOW_MS);
    assert_eq!(
        hosted
            .store
            .chain(&xena.public())
            .unwrap()
            .unwrap()
            .next_seq,
        1
    );

    // Run the bridge session:
    // Native offers y0 from waiting storage and learns of x0.
    // Hosted offers x0 and learns of y0.
    let report = run_session(&mut native, &mut hosted, &authors, None).await;
    assert_eq!(report.events_transferred, 2);

    // After session, native releases y0 and both events are applied on both sides.
    assert!(!native.is_waiting(&y0.id));
    assert_eq!(native.waiting_ids().unwrap().len(), 0);
    assert!(!hosted.store.is_waiting(&y0.id));

    let native_state = canonical(&native, &authors).unwrap();
    let hosted_state = canonical(&hosted.store, &authors).unwrap();
    assert_eq!(native_state, hosted_state);
    assert_eq!(native_state.applied.len(), 2);
    native.check_invariants().unwrap();
    hosted.store.check_invariants().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sparse_fork_reconciliation_scales_sublinearly_across_set_sizes() {
    // Tests reconciliation cost for a single-fork difference against
    // increasing shared set sizes (20, 60, 100 shared events).
    let mut results = Vec::new();
    for total_shared in [20usize, 60, 100] {
        let mut authors = Vec::new();
        let mut shared_events = Vec::new();
        let per_author = total_shared / 4;
        let mut authors_instances = Vec::new();
        for seed in [101u8, 103, 107, 109] {
            let mut author = Author::new(seed);
            authors.push(author.public());
            let mut prev: Option<Hash> = None;
            for _ in 0..per_author {
                let event = author.message(prev.map_or_else(Vec::new, |id| vec![id]));
                prev = Some(event.id);
                shared_events.push(event);
            }
            authors_instances.push(author);
        }

        let mut native = open_native();
        deliver(&mut native, &shared_events);
        let mut hosted = HostedStore::open();
        let shared_bytes: Vec<Vec<u8>> = shared_events.iter().map(|e| e.bytes.clone()).collect();
        hosted.ingest_batch(&shared_bytes, NOW_MS);

        // Author 0 creates a fork at next sequence:
        let author0 = &mut authors_instances[0];
        let next_seq = author0.seq;
        let backlink = author0.last;
        let native_event = author0.message(vec![]);
        let rival_event = author0.rival(next_seq, backlink);

        deliver(&mut native, &[native_event]);
        hosted.ingest_batch(&[rival_event.bytes.clone()], NOW_MS);

        let report = run_session(&mut native, &mut hosted, &authors, None).await;
        assert!(report.forks_detected >= 1);
        println!(
            "sparse fork reconciliation over {} shared events: {} flights, {} reconciliation bytes",
            total_shared, report.flights, report.reconcile_bytes
        );
        results.push((total_shared, report.flights, report.reconcile_bytes));
    }

    // Flights and reconciliation bytes for a sparse fork differ minimally even as
    // shared history quintuples from 20 to 100 events (sublinear in history size).
    let bytes_20 = results[0].2;
    let bytes_100 = results[2].2;
    assert!(
        bytes_100 < bytes_20 * 3,
        "reconciliation bytes should scale sublinearly with shared history: 20={bytes_20}, 100={bytes_100}"
    );
}
