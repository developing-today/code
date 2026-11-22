//! Third step of the #9106 walking skeleton (#12232, slice B2(c)):
//! the five steps of `substrate_sync_path`, with the exchange carried
//! over the existing transport between two processes.
//!
//! Node A runs in this process. Node B runs in a child process — this
//! same test binary, re-invoked as `node_b_entry` — serving the
//! federation sync protocol (`federation::net_exchange`) on one iroh
//! endpoint. The parent drives B's local writes and reads over a
//! test-only control protocol on a second ALPN of the same endpoint;
//! everything the two nodes *exchange* — inventories, offers, signed
//! versions — crosses on the sync protocol, through
//! `net_exchange::exchange` on A's side and the served
//! `FederationSyncProtocol` on B's, and is admitted on both sides by
//! the same B07 store and B32 gate as in process.
//!
//! Oracles, as in `substrate_sync_path`: differential (what B reads
//! from an address equals, byte for byte, what A signed; both nodes
//! report the same conflict key, versions and base), metamorphic
//! (repeating the exchange changes nothing), and a plain-set model of
//! the heads, checked on both processes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use id_core::graph::record::{GraphRecord, OpenedRecord, open};
use id_core::graph::term::{Term, Triple};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::id_uri_format::resolve_context;
use id_core::id_uri_parse::{IdUriV1, UriTarget};
use id_core::signed_envelope::{Envelope as RecordEnvelope, LogicalGraphId};
use id_sync::federation::net_exchange::{
    FEDERATION_SYNC_ALPN, FederationSyncProtocol, NetOutcome, exchange,
};
use id_sync::federation::slices::b07_conflict_report::{
    ConflictStore, IngestOutcome, ObjectId, RetentionPolicy, VersionHeader, conflict_key,
};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::Replica;
use id_sync::federation::slices::b32_ingest_gate::{
    EventId, FreshnessWindow, SignerAuthorization, sign_envelope,
};
use iroh::endpoint::{Connection, RelayMode, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};

const START: u64 = 10_000;
const SCOPE: &str = "notes";
const TRUST_DOMAIN: &str = "home";
const NAMESPACE: &str = "notes";
const CONTEXT: &str = "namespace/notes";
const GRAPH: [u8; 32] = [0x11; 32];
const WRITERS: [(&str, u8); 2] = [("alice", 7), ("bob", 9)];
const ALICE: usize = 0;
const BOB: usize = 1;

const CONTROL_ALPN: &[u8] = b"/id-skeleton-control/1";
const CHILD_ENV: &str = "ID_SKELETON_NODE_B";

// ---------------------------------------------------------------------
// The scenario, shared by both processes (same code as
// substrate_sync_path, so the two files read as siblings).

fn key(writer: usize) -> SigningKey {
    SigningKey::from_bytes(&[WRITERS[writer].1; 32])
}

fn node(name: &str) -> Replica {
    let signers = WRITERS
        .iter()
        .enumerate()
        .map(|(writer, (identity, _))| SignerAuthorization {
            identity: (*identity).to_owned(),
            verifying_key: key(writer).verifying_key().to_bytes(),
            scopes: [SCOPE.to_owned()].into_iter().collect(),
        })
        .collect();
    let freshness = FreshnessWindow {
        max_age: 1_000_000,
        max_future_skew: 30,
    };
    Replica::new(
        name,
        TRUST_DOMAIN,
        ConflictStore::new(signers, freshness, RetentionPolicy { retain_for: 3_600 }),
    )
}

fn link() -> SyncScopeConfig {
    SyncScopeConfig::new(SCOPE, TRUST_DOMAIN)
}

fn note(text: &[u8]) -> GraphRecord {
    GraphRecord::new(
        LogicalGraphId::new(GRAPH),
        vec![Triple::new(
            Term::typed(1, b"urn:note:1".to_vec()),
            Term::typed(2, b"body".to_vec()),
            Term::typed(3, text.to_vec()),
        )],
    )
}

struct Written {
    signed: OpenedRecord,
    event: EventId,
}

impl Written {
    fn address(&self, resolver: &str) -> String {
        IdUriV1::new(
            resolver,
            NAMESPACE,
            UriTarget::Content(*self.signed.id.content_id()),
        )
        .unwrap()
        .format()
    }
}

/// Sign `record`, wrap it as a version following `parents`, and
/// produce it on `node`. The join is decision D14, as in
/// `substrate_sync_path`.
fn publish(
    node: &mut Replica,
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> (Written, Result<IngestOutcome, String>) {
    let identity = WRITERS[writer].0;
    let signed = record.sign(CONTEXT, identity, &key(writer)).unwrap();
    let event: EventId = *signed.id.content_id().digest();
    let object: ObjectId = *record.graph.as_bytes();
    let header = VersionHeader {
        object_id: object,
        resolves: (parents.len() >= 2).then(|| conflict_key(parents)),
        parents: parents.to_vec(),
        body: signed.envelope.canonical_bytes().unwrap(),
    };
    let version = sign_envelope(&key(writer), identity, SCOPE, &header.encode(), event, now);
    let outcome = node
        .produce(&version, now)
        .map_err(|error| format!("{error:?}"));
    (Written { signed, event }, outcome)
}

#[derive(Debug, PartialEq, Eq)]
enum Unreadable {
    NotHeld,
    BodyIsNotTheNamedBytes,
    NotASignedRecord,
    JoinBroken(&'static str),
}

/// Read a record on `node` given only its address text: the same
/// checks, in the same order, as `substrate_sync_path`'s read.
fn read(node: &Replica, address: &str) -> Result<OpenedRecord, Unreadable> {
    let dispatch = ProfileDispatch::default();
    let uri = IdUriV1::parse(address).unwrap();
    resolve_context(uri.resolver_hint(), node.name()).unwrap();
    let UriTarget::Content(named) = uri.target() else {
        panic!("{address} does not name a record");
    };
    let version = node.envelope(named.digest()).ok_or(Unreadable::NotHeld)?;
    let header = VersionHeader::decode(&version.payload).unwrap();
    dispatch
        .verify_bytes(named, &header.body)
        .map_err(|_| Unreadable::BodyIsNotTheNamedBytes)?;
    let envelope =
        RecordEnvelope::decode(&header.body).map_err(|_| Unreadable::NotASignedRecord)?;
    let opened = open(envelope, &dispatch).map_err(|_| Unreadable::NotASignedRecord)?;
    if opened.id.content_id() != named {
        return Err(Unreadable::JoinBroken("event id"));
    }
    if &header.object_id != opened.record.graph.as_bytes() {
        return Err(Unreadable::JoinBroken("object"));
    }
    if version.verifying_key != opened.envelope.body.key {
        return Err(Unreadable::JoinBroken("signer"));
    }
    Ok(opened)
}

fn heads(node: &Replica) -> BTreeSet<EventId> {
    node.store().heads(&GRAPH).into_iter().collect()
}

/// The address of the version `event` as a reader on `resolver` writes it.
fn address_of(event: &EventId, resolver: &str) -> String {
    IdUriV1::new(
        resolver,
        NAMESPACE,
        UriTarget::Content(ContentId::new(HashProfile::RawBlake3_256, *event)),
    )
    .unwrap()
    .format()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap())
        .collect()
}

fn unhex32(text: &str) -> EventId {
    unhex(text).try_into().unwrap()
}

// ---------------------------------------------------------------------
// The control protocol: test-only, drives node B's local side. Nothing
// the nodes exchange travels on it.

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Control {
    Produce {
        writer: usize,
        text: String,
        parents: Vec<String>,
        now: u64,
    },
    Heads,
    Report,
    Read {
        address: String,
    },
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
enum ControlReply {
    Produced {
        event: String,
        outcome: String,
    },
    Heads {
        heads: Vec<String>,
    },
    Report {
        open: bool,
        id: Option<String>,
        key: Option<String>,
        base: Vec<String>,
        versions: Vec<String>,
        signers: Vec<String>,
    },
    Read {
        result: Result<String, String>,
    },
    Bye,
}

#[derive(Clone, Debug)]
struct ControlProtocol {
    replica: Arc<Mutex<Replica>>,
    shutdown: Arc<Notify>,
}

impl ProtocolHandler for ControlProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(streams) => streams,
                Err(_) => return Ok(()),
            };
            let Some(frame) = id_core::frames::read_frame(&mut recv)
                .await
                .map_err(AcceptError::from_err)?
            else {
                return Ok(());
            };
            let control: Control = serde_json::from_slice(&frame).map_err(AcceptError::from_err)?;
            let (reply, stop) = {
                let mut replica = self.replica.lock().await;
                answer_control(&mut replica, control)
            };
            let json = serde_json::to_vec(&reply).map_err(AcceptError::from_err)?;
            id_core::frames::write_frame(&mut send, &json)
                .await
                .map_err(AcceptError::from_err)?;
            send.finish().map_err(AcceptError::from_err)?;
            if stop {
                self.shutdown.notify_one();
                return Ok(());
            }
        }
    }
}

fn answer_control(replica: &mut Replica, control: Control) -> (ControlReply, bool) {
    match control {
        Control::Produce {
            writer,
            text,
            parents,
            now,
        } => {
            let parents: Vec<EventId> = parents.iter().map(|id| unhex32(id)).collect();
            let (written, outcome) = publish(replica, writer, &note(text.as_bytes()), &parents, now);
            (
                ControlReply::Produced {
                    event: hex(&written.event),
                    outcome: format!("{outcome:?}"),
                },
                false,
            )
        }
        Control::Heads => (
            ControlReply::Heads {
                heads: heads(replica).iter().map(|id| hex(id)).collect(),
            },
            false,
        ),
        Control::Report => {
            let Some(id) = replica.store().open_report(&GRAPH) else {
                return (
                    ControlReply::Report {
                        open: false,
                        id: None,
                        key: None,
                        base: vec![],
                        versions: vec![],
                        signers: vec![],
                    },
                    false,
                );
            };
            let stored = replica.store().report(&id).unwrap();
            (
                ControlReply::Report {
                    open: true,
                    id: Some(hex(&id)),
                    key: Some(hex(&stored.report.conflict_key())),
                    base: stored.report.base.iter().map(|id| hex(id)).collect(),
                    versions: stored
                        .report
                        .versions
                        .iter()
                        .map(|version| hex(&version.event_id))
                        .collect(),
                    signers: stored
                        .report
                        .versions
                        .iter()
                        .map(|version| version.signer.clone())
                        .collect(),
                },
                false,
            )
        }
        Control::Read { address } => {
            let result = read(replica, &address)
                .map(|opened| hex(&opened.envelope.canonical_bytes().unwrap()))
                .map_err(|unreadable| format!("{unreadable:?}"));
            (ControlReply::Read { result }, false)
        }
        Control::Shutdown => (ControlReply::Bye, true),
    }
}

/// Node B's process entry. In an ordinary test run (no env var) this
/// returns at once; the parent test spawns this binary with the env
/// var set and `--exact node_b_entry` to run it as the second node.
#[test]
fn node_b_entry() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let sync = FederationSyncProtocol::new(node("node-b"), link());
        let control = ControlProtocol {
            replica: sync.shared_replica(),
            shutdown: Arc::new(Notify::new()),
        };
        let shutdown = control.shutdown.clone();
        let router = Router::builder(endpoint.clone())
            .accept(FEDERATION_SYNC_ALPN, sync)
            .accept(CONTROL_ALPN, control)
            .spawn();
        let sockets = endpoint.bound_sockets();
        let port = sockets
            .iter()
            .find(|socket| socket.is_ipv4())
            .unwrap_or(&sockets[0])
            .port();
        println!("READY {} {port}", endpoint.id());
        use std::io::Write as _;
        std::io::stdout().flush().unwrap();
        shutdown.notified().await;
        router.shutdown().await.unwrap();
        endpoint.close().await;
    });
}

// ---------------------------------------------------------------------
// The parent: node A, in this process.

struct NodeB {
    child: Child,
    target: EndpointAddr,
    conn: Connection,
}

impl NodeB {
    async fn spawn(endpoint: &Endpoint) -> NodeB {
        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(exe)
            .args(["--exact", "node_b_entry", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            if let Some(Ok(line)) = lines.next() {
                let _ = tx.send(line);
            }
        });
        let ready = tokio::task::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_secs(60)).unwrap()
        })
        .await
        .unwrap();
        let mut parts = ready.split_whitespace();
        assert_eq!(parts.next(), Some("READY"), "child banner: {ready}");
        let id = EndpointId::from_str(parts.next().unwrap()).unwrap();
        let port: u16 = parts.next().unwrap().parse().unwrap();
        let target = EndpointAddr::from_parts(
            id,
            [TransportAddr::Ip(std::net::SocketAddr::from((
                std::net::Ipv4Addr::LOCALHOST,
                port,
            )))],
        );
        let conn = id_core::connectivity::connect(endpoint, target.clone(), CONTROL_ALPN)
            .await
            .unwrap();
        NodeB { child, target, conn }
    }

    async fn control(&self, control: Control) -> ControlReply {
        let (mut send, mut recv) = self.conn.open_bi().await.unwrap();
        let json = serde_json::to_vec(&control).unwrap();
        id_core::frames::write_frame(&mut send, &json).await.unwrap();
        send.finish().unwrap();
        let frame = id_core::frames::read_frame(&mut recv)
            .await
            .unwrap()
            .unwrap();
        serde_json::from_slice(&frame).unwrap()
    }

    async fn produce(
        &self,
        writer: usize,
        text: &str,
        parents: &[EventId],
        now: u64,
    ) -> (EventId, String) {
        let ControlReply::Produced { event, outcome } = self
            .control(Control::Produce {
                writer,
                text: text.to_owned(),
                parents: parents.iter().map(|id| hex(id)).collect(),
                now,
            })
            .await
        else {
            panic!("produce answered with a different reply");
        };
        (unhex32(&event), outcome)
    }

    async fn heads(&self) -> BTreeSet<EventId> {
        let ControlReply::Heads { heads } = self.control(Control::Heads).await else {
            panic!("heads answered with a different reply");
        };
        heads.iter().map(|id| unhex32(id)).collect()
    }

    async fn read(&self, address: &str) -> Result<Vec<u8>, String> {
        let ControlReply::Read { result } = self
            .control(Control::Read {
                address: address.to_owned(),
            })
            .await
        else {
            panic!("read answered with a different reply");
        };
        result.map(|bytes| unhex(&bytes))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_five_steps_run_between_two_processes_over_the_transport() {
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let b = NodeB::spawn(&endpoint).await;
    let mut a = node("node-a");

    // Step 1: Alice writes the base record on A.
    let (base, outcome) = publish(&mut a, ALICE, &note(b"first draft"), &[], START);
    assert_eq!(outcome, Ok(IngestOutcome::Applied));

    // Before any exchange, B does not hold it.
    assert_eq!(
        b.read(&base.address("node-b")).await,
        Err("NotHeld".to_owned())
    );

    // Steps 2 and 3: it is stored on A and syncs to B over the wire.
    let carried = exchange(&endpoint, b.target.clone(), &mut a, &link(), START + 1)
        .await
        .unwrap();
    assert_eq!(carried.forward.offer.send, vec![hex(&base.event)]);
    assert_eq!(
        carried.forward.outcomes,
        vec![(base.event, NetOutcome::Applied)]
    );
    assert!(carried.backward.offer.send.is_empty());
    assert_eq!(heads(&a), BTreeSet::from([base.event]));
    assert_eq!(b.heads().await, BTreeSet::from([base.event]));

    // B reads exactly the bytes Alice signed, from the address alone.
    assert_eq!(
        b.read(&base.address("node-b")).await.unwrap(),
        base.signed.envelope.canonical_bytes().unwrap()
    );
    assert_eq!(read(&a, &base.address("node-a")).unwrap(), base.signed);

    // Repeating the exchange changes nothing.
    let carried = exchange(&endpoint, b.target.clone(), &mut a, &link(), START + 2)
        .await
        .unwrap();
    assert!(carried.is_quiet());

    // Step 4: each writer follows the base on their own node.
    let (from_a, outcome) = publish(&mut a, ALICE, &note(b"alice's edit"), &[base.event], START + 3);
    assert_eq!(outcome, Ok(IngestOutcome::Applied));
    let (from_b_event, outcome) = b.produce(BOB, "bob's edit", &[base.event], START + 3).await;
    assert_eq!(outcome, "Ok(Applied)");
    assert_ne!(from_a.event, from_b_event);

    // The exchange reports the conflict on both nodes: each side's
    // admission of the other's edit opens the report there.
    let carried = exchange(&endpoint, b.target.clone(), &mut a, &link(), START + 4)
        .await
        .unwrap();
    assert_eq!(carried.forward.offer.send, vec![hex(&from_a.event)]);
    let ControlReply::Report {
        open: b_open,
        id: b_report,
        key: b_key,
        base: b_base,
        versions: b_versions,
        signers: b_signers,
    } = b.control(Control::Report).await
    else {
        panic!("report answered with a different reply");
    };
    assert!(b_open);
    assert_eq!(
        carried.forward.outcomes,
        vec![(from_a.event, NetOutcome::Conflict(b_report.unwrap()))]
    );
    assert_eq!(carried.backward.offer.send, vec![hex(&from_b_event)]);
    let report_a = a.store().open_report(&GRAPH).unwrap();
    assert_eq!(
        carried.backward.outcomes,
        vec![(from_b_event, NetOutcome::Conflict(hex(&report_a)))]
    );

    // Step 5: both nodes report the same conflict — same key, same
    // versions, same base — and nothing was merged or dropped.
    let expected = BTreeSet::from([from_a.event, from_b_event]);
    let key = conflict_key(&[from_a.event, from_b_event]);
    assert_eq!(hex(&key), b_key.clone().unwrap());
    let stored_a = a.store().report(&report_a).unwrap();
    assert_eq!(stored_a.report.conflict_key(), key);
    assert_eq!(stored_a.report.base, vec![base.event]);
    assert_eq!(b_base, vec![hex(&base.event)]);
    let named_b: BTreeSet<EventId> = b_versions.iter().map(|id| unhex32(id)).collect();
    assert_eq!(named_b, expected);
    let signers_b: BTreeSet<&str> = b_signers.iter().map(String::as_str).collect();
    assert_eq!(signers_b, BTreeSet::from(["alice", "bob"]));
    assert_eq!(heads(&a), expected);
    assert_eq!(b.heads().await, expected);
    for (written, resolver) in [(&base, "node-a"), (&from_a, "node-a")] {
        assert_eq!(read(&a, &written.address(resolver)).unwrap(), written.signed);
    }
    assert_eq!(
        b.read(&from_a.address("node-b")).await.unwrap(),
        from_a.signed.envelope.canonical_bytes().unwrap()
    );
    assert_eq!(
        b.read(&base.address("node-b")).await.unwrap(),
        base.signed.envelope.canonical_bytes().unwrap()
    );
    // Bob's edit, which A holds only from the exchange, reads the same
    // on both nodes.
    let bobs_on_a = read(&a, &address_of(&from_b_event, "node-a")).unwrap();
    assert_eq!(
        b.read(&address_of(&from_b_event, "node-b")).await.unwrap(),
        bobs_on_a.envelope.canonical_bytes().unwrap()
    );

    // Another exchange neither clears the report nor invents a merge.
    let carried = exchange(&endpoint, b.target.clone(), &mut a, &link(), START + 5)
        .await
        .unwrap();
    assert!(carried.is_quiet());
    assert!(a.store().open_report(&GRAPH).is_some());

    // Bob resolves it on B with a record following both edits; the
    // next exchange closes the report on A too.
    let (merged_event, outcome) = b
        .produce(
            BOB,
            "both edits, merged by bob",
            &[from_a.event, from_b_event],
            START + 6,
        )
        .await;
    assert!(outcome.starts_with("Ok(Resolved"), "merge on B: {outcome}");
    let carried = exchange(&endpoint, b.target.clone(), &mut a, &link(), START + 7)
        .await
        .unwrap();
    assert_eq!(
        carried.backward.outcomes,
        vec![(merged_event, NetOutcome::Resolved(hex(&report_a)))]
    );
    assert_eq!(heads(&a), BTreeSet::from([merged_event]));
    assert_eq!(b.heads().await, BTreeSet::from([merged_event]));
    assert!(a.store().open_report(&GRAPH).is_none());
    let ControlReply::Report { open, .. } = b.control(Control::Report).await else {
        panic!("report answered with a different reply");
    };
    assert!(!open);

    // The merged record is readable from its address on both nodes.
    assert!(b.read(&address_of(&merged_event, "node-b")).await.is_ok());
    assert!(read(&a, &address_of(&merged_event, "node-a")).is_ok());

    // Shut B down and see it exit cleanly.
    let ControlReply::Bye = b.control(Control::Shutdown).await else {
        panic!("shutdown answered with a different reply");
    };
    let mut b = b;
    let status = tokio::task::spawn_blocking(move || {
        for _ in 0..100 {
            if let Some(status) = b.child.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = b.child.kill();
        None
    })
    .await
    .unwrap();
    assert!(status.is_some_and(|status| status.success()));
    endpoint.close().await;
}
