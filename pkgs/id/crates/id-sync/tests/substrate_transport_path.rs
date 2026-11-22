//! Third step of the #9106 walking skeleton (#12232): the path of
//! `substrate_sync_path`, carried between two processes over the
//! existing transport.
//!
//! Node A is a [`Replica`] in this process, exactly as in the second
//! step. Node B is a second operating-system process — this same test
//! binary, re-executed as [`node_b_entry`] — holding its own replica
//! behind the federation exchange protocol (`/id-fed-exchange/1`,
//! iroh, the transport every other id protocol rides). The exchange
//! between them is [`reconcile_remote`]: the B09 exchange of the
//! second step, with the wire where the direct calls were.
//!
//! The five steps, in order: a signed record is written at an `id://`
//! address on A; it is held in A's store; it syncs to B over the
//! transport; a conflicting record is written on B (in B's process,
//! through B's own gate, at this test's direction over a test-only
//! control protocol); the conflict is reported on both nodes.
//! Nothing about B is reachable except through the two protocols:
//! before the first exchange, B's address lookup for the record
//! answers "not held", and every byte B serves afterwards entered
//! through its gate during an exchange or a directed produce.
//!
//! Oracles: differential (what B reads equals what A signed; both
//! nodes report the same conflict key, versions and base; the
//! `Exchange` the initiator assembles names the same sends and
//! outcomes the in-process `reconcile` names for this scenario) and
//! metamorphic (repeating the exchange changes nothing).
//!
//! Named limits of this slice: sessions are initiated by A only (the
//! protocol is symmetric — either replica may initiate — but the
//! harness drives one initiator), and both replicas are in-memory;
//! the on-disk store (step B2(b), PR #12497) composes with this
//! transport once both have landed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write as _};
use std::net::{Ipv4Addr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{Connection, RelayMode, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use id_core::connectivity::connect;
use id_core::frames::{read_frame, write_frame};
use id_core::graph::record::{GraphRecord, OpenedRecord, open};
use id_core::graph::term::{Term, Triple};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_uri_format::resolve_context;
use id_core::id_uri_parse::{IdUriV1, UriTarget};
use id_core::signed_envelope::{Envelope as RecordEnvelope, LogicalGraphId};
use id_sync::federation::exchange_net::{
    EXCHANGE_ALPN, ExchangeProtocol, WireEnvelope, WireOutcomeResult, from_wire_outcome,
    reconcile_remote, to_wire_outcome,
};
use id_sync::federation::slices::b07_conflict_report::{
    ConflictStore, IngestOutcome, ObjectId, ReportId, RetentionPolicy, VersionHeader,
    conflict_key,
};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::Replica;
use id_sync::federation::slices::b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, SignerAuthorization, sign_envelope,
};

const START: u64 = 10_000;
const SCOPE: &str = "notes";
const TRUST_DOMAIN: &str = "home";
const NAMESPACE: &str = "notes";
const CONTEXT: &str = "namespace/notes";
const GRAPH: [u8; 32] = [0x11; 32];
const WRITERS: [(&str, u8); 2] = [("alice", 7), ("bob", 9)];
const ALICE: usize = 0;
const BOB: usize = 1;

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

/// A record a writer put on a node: what was signed, its event id and
/// the address a reader on `resolver` would be given.
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

/// Sign `record` as a part A record and wrap it as a part B version
/// that follows `parents`: the envelope a node admits, whichever
/// process the node lives in.
fn build_version(
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> (Written, Envelope) {
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
    (Written { signed, event }, version)
}

/// Produce a record on a local replica.
fn publish(
    node: &mut Replica,
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> (Written, IngestOutcome) {
    let (written, version) = build_version(writer, record, parents, now);
    let outcome = node.produce(&version, now).unwrap();
    (written, outcome)
}

/// Why a node could not serve a record for an address.
#[derive(Debug, PartialEq, Eq)]
enum Unreadable {
    /// The node holds no version under the address's digest.
    NotHeld,
    /// The version's body is not the bytes the address names.
    BodyIsNotTheNamedBytes,
    /// The body is the named bytes, and they are not a valid signed record.
    NotASignedRecord,
    /// The record is valid, and the version that carries it says
    /// something else about it.
    JoinBroken(&'static str),
}

fn digest_of(address: &str) -> EventId {
    let uri = IdUriV1::parse(address).unwrap();
    let UriTarget::Content(named) = uri.target() else {
        panic!("{address} does not name a record");
    };
    *named.digest()
}

/// Read a record given only its address text and the envelope the
/// node named by the address serves for it: the address names the
/// version, the served bytes are checked against the address before
/// they are opened, and every part of the join is checked on the way
/// out — the same checks as the second step's `read`.
fn open_served(
    name: &str,
    served: Option<&Envelope>,
    address: &str,
) -> Result<OpenedRecord, Unreadable> {
    let dispatch = ProfileDispatch::default();
    let uri = IdUriV1::parse(address).unwrap();
    resolve_context(uri.resolver_hint(), name).unwrap();
    let UriTarget::Content(named) = uri.target() else {
        panic!("{address} does not name a record");
    };
    let version = served.ok_or(Unreadable::NotHeld)?;
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

/// Read a record on a local replica from its address.
fn read(node: &Replica, address: &str) -> Result<OpenedRecord, Unreadable> {
    let digest = digest_of(address);
    open_served(node.name(), node.envelope(&digest), address)
}

fn heads(node: &Replica) -> BTreeSet<EventId> {
    node.store().heads(&GRAPH).into_iter().collect()
}

// ---------------------------------------------------------------------
// Node B: the second process, and the test-only control protocol that
// directs it. The control protocol is not part of the path: it stands
// in for B's own local users, the way the second step's direct calls
// stood in for them. Everything the scenario asserts about B is
// observed through it or through the exchange protocol itself.

/// ALPN of the test-only control protocol.
const CONTROL_ALPN: &[u8] = b"/id-fed-test-control/1";

/// One direction to node B.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Control {
    /// Admit this envelope as a local production on B.
    Produce {
        /// The envelope, as the wire carries it.
        envelope: WireEnvelope,
        /// The logical time of the production.
        now: u64,
    },
    /// Serve the envelope held under this event id, if any.
    Envelope {
        /// The event id to serve.
        event: EventId,
    },
    /// Report B's heads and open report for the scenario's graph.
    State,
    /// Answer, then exit.
    Shutdown,
}

/// A conflict report as the control protocol reports it: the fields
/// the scenario asserts on.
#[derive(Debug, Serialize, Deserialize)]
struct WireReport {
    /// The report's own id on B.
    id: ReportId,
    /// The conflict key both nodes share.
    key: [u8; 32],
    /// The diverged object.
    object: ObjectId,
    /// The base the dispute diverged from.
    base: Vec<EventId>,
    /// The conflicting versions, with their signers.
    versions: Vec<(EventId, String)>,
}

/// Node B's answer to one direction.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
enum ControlReply {
    /// The production's outcome on B.
    Outcome {
        /// The outcome, as the wire carries it.
        outcome: WireOutcomeResult,
    },
    /// The envelope B serves for the asked id, if it holds one.
    Envelope {
        /// The envelope, as the wire carries it.
        envelope: Option<WireEnvelope>,
    },
    /// B's heads and open report.
    State {
        /// The graph's heads on B.
        heads: Vec<EventId>,
        /// The open report on B, if one is open.
        open_report: Option<WireReport>,
    },
    /// B is exiting.
    Bye,
}

/// Serves the control protocol over B's replica.
#[derive(Clone, Debug)]
struct ControlProtocol {
    replica: Arc<Mutex<Replica>>,
}

impl ControlProtocol {
    async fn answer(&self, body: &[u8]) -> ControlReply {
        let control: Control = match serde_json::from_slice(body) {
            Ok(control) => control,
            Err(_) => return ControlReply::Bye,
        };
        match control {
            Control::Produce { envelope, now } => {
                let envelope: Envelope = envelope.into();
                let mut replica = self.replica.lock().await;
                let result = replica.produce(&envelope, now);
                ControlReply::Outcome {
                    outcome: to_wire_outcome(&result),
                }
            }
            Control::Envelope { event } => {
                let replica = self.replica.lock().await;
                ControlReply::Envelope {
                    envelope: replica.envelope(&event).map(WireEnvelope::from),
                }
            }
            Control::State => {
                let replica = self.replica.lock().await;
                let store = replica.store();
                let open_report = store.open_report(&GRAPH).and_then(|id| {
                    store.report(&id).map(|stored| WireReport {
                        id,
                        key: stored.report.conflict_key(),
                        object: stored.report.object_id,
                        base: stored.report.base.clone(),
                        versions: stored
                            .report
                            .versions
                            .iter()
                            .map(|v| (v.event_id, v.signer.clone()))
                            .collect(),
                    })
                });
                ControlReply::State {
                    heads: store.heads(&GRAPH),
                    open_report,
                }
            }
            Control::Shutdown => ControlReply::Bye,
        }
    }
}

impl ProtocolHandler for ControlProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let Ok(Some(body)) = read_frame(&mut recv).await else {
                continue;
            };
            let shutdown = matches!(
                serde_json::from_slice::<Control>(&body),
                Ok(Control::Shutdown)
            );
            let reply = self.answer(&body).await;
            let Ok(json) = serde_json::to_vec(&reply) else {
                continue;
            };
            if write_frame(&mut send, &json).await.is_ok() {
                let _ = send.finish();
            }
            if shutdown {
                tokio::time::sleep(Duration::from_millis(50)).await;
                std::process::exit(0);
            }
        }
        Ok(())
    }
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

/// The loopback socket address an endpoint can be reached at.
fn loopback_sock(endpoint: &Endpoint) -> SocketAddr {
    let bound = endpoint.bound_sockets()[0];
    let ip = if bound.ip().is_unspecified() {
        Ipv4Addr::LOCALHOST.into()
    } else {
        bound.ip()
    };
    SocketAddr::new(ip, bound.port())
}

/// Node B's process entry point. Spawned by the scenario test as a
/// re-execution of this test binary; ignored in every other run.
#[tokio::test]
#[ignore = "process entry point for node B; spawned by the transport scenario test"]
async fn node_b_entry() {
    if std::env::var_os("ID_FED_NODE_B").is_none() {
        return;
    }
    let replica = Arc::new(Mutex::new(node("node-b")));
    let endpoint = endpoint().await;
    let _router = Router::builder(endpoint.clone())
        .accept(EXCHANGE_ALPN, ExchangeProtocol::new(replica.clone()))
        .accept(
            CONTROL_ALPN,
            ControlProtocol {
                replica: replica.clone(),
            },
        )
        .spawn();
    println!(
        "FED_NODE id={} sock={}",
        endpoint.id(),
        loopback_sock(&endpoint)
    );
    std::io::stdout().flush().unwrap();
    std::future::pending::<()>().await;
}

/// Node B, from the scenario test's side: a child process and the
/// endpoint that talks to it.
struct RemoteNode {
    child: Child,
    endpoint: Endpoint,
    addr: EndpointAddr,
}

impl RemoteNode {
    async fn spawn(endpoint: Endpoint) -> Self {
        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(exe)
            .args(["--exact", "node_b_entry", "--ignored", "--nocapture"])
            .env("ID_FED_NODE_B", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(Ok(line)) = lines.next() {
                if line.starts_with("FED_NODE ") {
                    let _ = tx.send(line);
                    return;
                }
            }
        });
        let hello = tokio::task::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_secs(60))
        })
        .await
        .unwrap()
        .expect("node B did not announce itself");
        let mut fields = hello.split_whitespace();
        let id: iroh::EndpointId = fields
            .nth(1)
            .and_then(|field| field.strip_prefix("id="))
            .unwrap()
            .parse()
            .unwrap();
        let sock: SocketAddr = fields
            .next()
            .and_then(|field| field.strip_prefix("sock="))
            .unwrap()
            .parse()
            .unwrap();
        let addr = EndpointAddr::from_parts(id, [TransportAddr::Ip(sock)]);
        Self {
            child,
            endpoint,
            addr,
        }
    }

    async fn control(&self, control: &Control) -> ControlReply {
        let run = async {
            let conn = connect(&self.endpoint, self.addr.clone(), CONTROL_ALPN)
                .await
                .unwrap();
            let (mut send, mut recv) = conn.open_bi().await.unwrap();
            let json = serde_json::to_vec(control).unwrap();
            write_frame(&mut send, &json).await.unwrap();
            send.finish().unwrap();
            let frame = read_frame(&mut recv).await.unwrap().unwrap();
            conn.close(0_u32.into(), b"done");
            serde_json::from_slice::<ControlReply>(&frame).unwrap()
        };
        tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("node B did not answer in time")
    }

    /// Direct B to produce a record in its own process, through its
    /// own gate.
    async fn publish(
        &self,
        writer: usize,
        record: &GraphRecord,
        parents: &[EventId],
        now: u64,
    ) -> (Written, IngestOutcome) {
        let (written, version) = build_version(writer, record, parents, now);
        let reply = self
            .control(&Control::Produce {
                envelope: WireEnvelope::from(&version),
                now,
            })
            .await;
        let ControlReply::Outcome { outcome } = reply else {
            panic!("unexpected control reply");
        };
        (written, from_wire_outcome(outcome).unwrap())
    }

    /// Read a record on B from its address: B serves the envelope it
    /// holds, and the bytes are checked here exactly as on a local
    /// read.
    async fn read(&self, address: &str) -> Result<OpenedRecord, Unreadable> {
        let digest = digest_of(address);
        let reply = self.control(&Control::Envelope { event: digest }).await;
        let ControlReply::Envelope { envelope } = reply else {
            panic!("unexpected control reply");
        };
        let envelope: Option<Envelope> = envelope.map(Into::into);
        open_served("node-b", envelope.as_ref(), address)
    }

    /// B's heads for the graph, and its open report if one is open.
    async fn state(&self) -> (BTreeSet<EventId>, Option<WireReport>) {
        let reply = self.control(&Control::State).await;
        let ControlReply::State { heads, open_report } = reply else {
            panic!("unexpected control reply");
        };
        (heads.into_iter().collect(), open_report)
    }

    /// Run one exchange with B as responder and A as initiator.
    async fn exchange(
        &self,
        local: &mut Replica,
        now: u64,
    ) -> id_sync::federation::slices::b09_anti_entropy::Exchange {
        tokio::time::timeout(
            Duration::from_secs(30),
            reconcile_remote(&self.endpoint, self.addr.clone(), local, &link(), now),
        )
        .await
        .expect("the exchange did not finish in time")
        .unwrap()
    }

    async fn shutdown(mut self) {
        let _ = self.control(&Control::Shutdown).await;
        let _ = self.child.wait();
    }
}

impl Drop for RemoteNode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn probe_spawn_and_state() {
    let endpoint = endpoint().await;
    let b = RemoteNode::spawn(endpoint).await;
    eprintln!("probe: child announced at {:?}", b.addr);
    let (heads, report) = b.state().await;
    eprintln!("probe: state heads={heads:?} report={report:?}");
    b.shutdown().await;
}

#[tokio::test]
async fn the_path_runs_between_two_processes_over_the_transport() {
    let mut a = node("node-a");
    let endpoint = endpoint().await;
    let b = RemoteNode::spawn(endpoint).await;

    // Written on A; held in A's store.
    let (base, outcome) = publish(&mut a, ALICE, &note(b"first draft"), &[], START);
    assert_eq!(outcome, IngestOutcome::Applied);

    // Before the exchange, B does not have it: the address resolves
    // to nothing there, and it is not read from anywhere else.
    assert_eq!(b.read(&base.address("node-b")).await, Err(Unreadable::NotHeld));

    // Synced to B over the transport: the exchange names the send,
    // and afterwards B reads exactly what Alice signed on A, from
    // the address.
    let exchange = b.exchange(&mut a, START + 1).await;
    assert_eq!(exchange.forward.offer.send, vec![base.event]);
    let on_b = b.read(&base.address("node-b")).await.unwrap();
    assert_eq!(on_b, base.signed);
    assert_eq!(on_b.envelope.body.signer_id, "alice");
    assert_eq!(
        on_b.assertions().next().unwrap().triple,
        &note(b"first draft").triples[0]
    );
    assert_eq!(read(&a, &base.address("node-a")).unwrap(), base.signed);
    assert_eq!(heads(&a), BTreeSet::from([base.event]));
    assert_eq!(b.state().await.0, BTreeSet::from([base.event]));

    // Repeating the exchange changes nothing.
    assert!(b.exchange(&mut a, START + 2).await.is_quiet());

    // Both writers follow the same record, each in their own
    // process. Locally each write is a plain successor: neither
    // node can know.
    let (from_a, on_a) = publish(
        &mut a,
        ALICE,
        &note(b"alice's edit"),
        &[base.event],
        START + 3,
    );
    let (from_b, on_b) = b
        .publish(BOB, &note(b"bob's edit"), &[base.event], START + 3)
        .await;
    assert_eq!(on_a, IngestOutcome::Applied);
    assert_eq!(on_b, IngestOutcome::Applied);
    assert_ne!(from_a.event, from_b.event);

    // The next exchange reports the conflict on both nodes.
    b.exchange(&mut a, START + 4).await;
    let expected = BTreeSet::from([from_a.event, from_b.event]);
    let key = conflict_key(&[from_a.event, from_b.event]);

    assert_eq!(heads(&a), expected, "node-a heads");
    let report_a = a
        .store()
        .open_report(&GRAPH)
        .expect("node-a holds no open report");
    let stored = a.store().report(&report_a).unwrap();
    assert_eq!(stored.report.conflict_key(), key, "node-a conflict key");
    assert_eq!(stored.report.object_id, GRAPH);
    assert_eq!(stored.report.base, vec![base.event], "node-a base");
    let named: BTreeSet<EventId> = stored.report.versions.iter().map(|v| v.event_id).collect();
    assert_eq!(named, expected, "node-a report versions");

    let (heads_b, report_b) = b.state().await;
    assert_eq!(heads_b, expected, "node-b heads");
    let report_b = report_b.expect("node-b holds no open report");
    assert_eq!(report_b.key, key, "node-b conflict key");
    assert_eq!(report_b.object, GRAPH);
    assert_eq!(report_b.base, vec![base.event], "node-b base");
    let named_b: BTreeSet<EventId> = report_b.versions.iter().map(|(id, _)| *id).collect();
    assert_eq!(named_b, expected, "node-b report versions");
    let signers_b: BTreeSet<&str> = report_b.versions.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(signers_b, BTreeSet::from(["alice", "bob"]));

    // Nothing was merged or dropped: both edits, and the record they
    // follow, are each still readable from their address on B.
    assert_eq!(b.read(&from_a.address("node-b")).await.unwrap(), from_a.signed);
    assert_eq!(b.read(&from_b.address("node-b")).await.unwrap(), from_b.signed);
    assert_eq!(b.read(&base.address("node-b")).await.unwrap(), base.signed);

    // Another exchange neither clears the report nor invents a merge.
    assert!(b.exchange(&mut a, START + 5).await.is_quiet());
    assert!(a.store().open_report(&GRAPH).is_some());
    assert!(b.state().await.1.is_some());

    // Bob resolves it on B, with a record that follows both edits;
    // the next exchange closes the report on A too, and says so.
    let (merged, outcome) = b
        .publish(
            BOB,
            &note(b"both edits, merged by bob"),
            &[from_a.event, from_b.event],
            START + 6,
        )
        .await;
    assert_eq!(outcome, IngestOutcome::Resolved(report_b.id));
    let exchange = b.exchange(&mut a, START + 7).await;
    assert_eq!(
        exchange.backward.outcomes,
        vec![(merged.event, Ok(IngestOutcome::Resolved(report_a)))]
    );
    assert_eq!(heads(&a), BTreeSet::from([merged.event]));
    assert!(a.store().open_report(&GRAPH).is_none());
    assert_eq!(b.state().await.0, BTreeSet::from([merged.event]));
    assert!(b.state().await.1.is_none());
    assert_eq!(read(&a, &merged.address("node-a")).unwrap(), merged.signed);
    assert_eq!(b.read(&merged.address("node-b")).await.unwrap(), merged.signed);
    assert!(b.exchange(&mut a, START + 8).await.is_quiet());

    b.shutdown().await;
}
