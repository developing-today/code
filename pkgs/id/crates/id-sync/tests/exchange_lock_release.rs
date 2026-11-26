//! Regression for #12636: the responder side of the federation
//! exchange (`/id-fed-exchange/1`) must not hold the replica lock
//! while it waits on the peer.
//!
//! One initiator completes a valid hello and then stops sending. While
//! that session is stalled, waiting for a transfer frame that never
//! comes, the replica it serves stays usable: its owner can take the
//! lock, and a second, healthy exchange with the same responder
//! finishes, both well inside the 30-second session timeout that
//! would otherwise be the only thing to free the lock.
//!
//! A version the responder admits from elsewhere during that wait is
//! not part of the stalled session's offer: it may be newer than the
//! session's logical time, and an initiator that refused it for timing
//! would remember the refusal. It reaches the initiator in the next
//! exchange instead.
//!
//! Venue: like `substrate_transport_path`, the sessions ride
//! QUIC/UDP on loopback, so this test needs UDP sends to be permitted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use tokio::sync::Mutex;

use id_core::connectivity::connect;
use id_core::frames::{read_frame, write_frame};
use id_core::graph::record::GraphRecord;
use id_core::graph::term::{Term, Triple};
use id_core::signed_envelope::LogicalGraphId;
use id_sync::federation::exchange_net::{
    ClientTransfer, EXCHANGE_ALPN, ExchangeProtocol, Hello, HelloReply, ServerTransfer,
    reconcile_remote,
};
use id_sync::federation::slices::b07_conflict_report::{
    ConflictStore, IngestOutcome, ObjectId, RetentionPolicy, VersionHeader,
};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::Replica;
use id_sync::federation::slices::b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, SignerAuthorization, sign_envelope,
};

const SCOPE: &str = "notes";
const TRUST_DOMAIN: &str = "home";
const NOW: u64 = 10_000;
const CONTEXT: &str = "namespace/notes";
const GRAPH: [u8; 32] = [0x11; 32];
const WRITER: &str = "alice";

/// Well under the responder's 30-second session timeout: anything
/// that waits on the stalled session's lock fails this bound.
const PROMPT: Duration = Duration::from_secs(5);

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

/// A replica that admits Alice's notes, with no future skew allowed:
/// a version issued after an exchange's logical time is refused.
fn node(name: &str) -> Replica {
    let signers = vec![SignerAuthorization {
        identity: WRITER.to_owned(),
        verifying_key: key().verifying_key().to_bytes(),
        scopes: [SCOPE.to_owned()].into_iter().collect(),
    }];
    let freshness = FreshnessWindow {
        max_age: 1_000_000,
        max_future_skew: 0,
    };
    Replica::new(
        name,
        TRUST_DOMAIN,
        ConflictStore::new(signers, freshness, RetentionPolicy { retain_for: 3_600 }),
    )
}

/// A signed note from Alice, as a first version issued at `now`.
fn version(text: &[u8], now: u64) -> Envelope {
    let record = GraphRecord::new(
        LogicalGraphId::new(GRAPH),
        vec![Triple::new(
            Term::typed(1, b"urn:note:1".to_vec()),
            Term::typed(2, b"body".to_vec()),
            Term::typed(3, text.to_vec()),
        )],
    );
    let signed = record.sign(CONTEXT, WRITER, &key()).unwrap();
    let event: EventId = *signed.id.content_id().digest();
    let object: ObjectId = *record.graph.as_bytes();
    let header = VersionHeader {
        object_id: object,
        resolves: None,
        parents: Vec::new(),
        body: signed.envelope.canonical_bytes().unwrap(),
    };
    sign_envelope(&key(), WRITER, SCOPE, &header.encode(), event, now)
}

/// An initiator's hello with nothing held, at `now`.
fn empty_hello(name: &str, now: u64) -> Hello {
    let config = link();
    Hello {
        name: name.to_owned(),
        scope: config.scope.clone(),
        trust_domain: config.trust_domain.clone(),
        capacity: config.capacity,
        retention: config.retention,
        now,
        summary: Vec::new(),
        refused: Vec::new(),
    }
}

fn link() -> SyncScopeConfig {
    SyncScopeConfig::new(SCOPE, TRUST_DOMAIN)
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

/// The loopback address an endpoint can be dialled at.
fn loopback_addr(endpoint: &Endpoint) -> EndpointAddr {
    let bound = endpoint.bound_sockets()[0];
    let ip = if bound.ip().is_unspecified() {
        Ipv4Addr::LOCALHOST.into()
    } else {
        bound.ip()
    };
    EndpointAddr::from_parts(
        endpoint.id(),
        [TransportAddr::Ip(SocketAddr::new(ip, bound.port()))],
    )
}

#[tokio::test]
async fn a_peer_stalled_after_hello_does_not_hold_the_replica() {
    let replica = Arc::new(Mutex::new(node("responder")));
    let server = endpoint().await;
    let _router = Router::builder(server.clone())
        .accept(EXCHANGE_ALPN, ExchangeProtocol::new(replica.clone()))
        .spawn();
    let target = loopback_addr(&server);

    // The stalled initiator: a valid hello, the reply read, and then
    // nothing. Its connection and streams stay open for the rest of
    // the test, so the responder's session is parked waiting for the
    // transfer frame.
    let stalled = endpoint().await;
    let conn = tokio::time::timeout(PROMPT, connect(&stalled, target.clone(), EXCHANGE_ALPN))
        .await
        .expect("connecting the stalled initiator timed out")
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let config = link();
    let hello = empty_hello("stalled", NOW);
    write_frame(&mut send, &serde_json::to_vec(&hello).unwrap())
        .await
        .unwrap();
    let frame = tokio::time::timeout(PROMPT, read_frame(&mut recv))
        .await
        .expect("the responder did not answer the hello")
        .unwrap()
        .expect("the responder closed without a hello reply");
    let reply: HelloReply = serde_json::from_slice(&frame).unwrap();
    assert_eq!(reply.name, "responder");

    // The owner can take the lock while the session is stalled.
    let guard = tokio::time::timeout(PROMPT, replica.lock())
        .await
        .expect("the stalled session held the replica lock");
    drop(guard);

    // A healthy exchange with the same responder finishes promptly.
    let healthy = endpoint().await;
    let mut local = node("healthy");
    let exchange = tokio::time::timeout(
        PROMPT,
        reconcile_remote(&healthy, target, &mut local, &config, NOW),
    )
    .await
    .expect("the healthy exchange waited on the stalled session")
    .unwrap();
    assert!(exchange.is_quiet());

    // The stalled streams are only dropped here, after both checks.
    drop((send, recv, conn));
}

#[tokio::test]
async fn a_version_admitted_during_the_wait_waits_for_the_next_exchange() {
    let replica = Arc::new(Mutex::new(node("responder")));
    let server = endpoint().await;
    let _router = Router::builder(server.clone())
        .accept(EXCHANGE_ALPN, ExchangeProtocol::new(replica.clone()))
        .spawn();
    let target = loopback_addr(&server);
    let mut initiator = node("initiator");

    // A session opens at NOW with nothing held on either side, and
    // parks after the hello.
    let client = endpoint().await;
    let conn = tokio::time::timeout(PROMPT, connect(&client, target.clone(), EXCHANGE_ALPN))
        .await
        .expect("connecting timed out")
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let hello = empty_hello("initiator", NOW);
    write_frame(&mut send, &serde_json::to_vec(&hello).unwrap())
        .await
        .unwrap();
    let frame = tokio::time::timeout(PROMPT, read_frame(&mut recv))
        .await
        .expect("the responder did not answer the hello")
        .unwrap()
        .unwrap();
    let reply: HelloReply = serde_json::from_slice(&frame).unwrap();
    assert_eq!(reply.summary, [] as [(ObjectId, Digest32); 0]);

    // While the session waits, the responder's owner admits a version
    // issued after the session's logical time.
    let late = version(b"written during the wait", NOW + 1);
    let outcome = tokio::time::timeout(PROMPT, replica.lock())
        .await
        .expect("the waiting session held the replica lock")
        .produce(&late, NOW + 1)
        .unwrap();
    assert_eq!(outcome, IngestOutcome::Applied);

    // The session resumes: its offer is the one the hello-time
    // inventory supports, so the late version is not in it.
    let transfer = ClientTransfer {
        ids: Vec::new(),
        versions: Vec::new(),
    };
    write_frame(&mut send, &serde_json::to_vec(&transfer).unwrap())
        .await
        .unwrap();
    send.finish().unwrap();
    let frame = tokio::time::timeout(PROMPT, read_frame(&mut recv))
        .await
        .expect("the responder did not answer the transfer")
        .unwrap()
        .unwrap();
    let server_transfer: ServerTransfer = serde_json::from_slice(&frame).unwrap();
    assert_eq!(server_transfer.offer.send, [] as [EventId; 0]);
    assert_eq!(server_transfer.versions, [] as [WireEnvelope; 0]);
    conn.close(0_u32.into(), b"done");

    // The next exchange, at a time the version is valid for, delivers
    // it and the initiator admits it.
    let exchange = tokio::time::timeout(
        PROMPT,
        reconcile_remote(&client, target, &mut initiator, &link(), NOW + 1),
    )
    .await
    .expect("the next exchange did not finish in time")
    .unwrap();
    assert_eq!(exchange.backward.offer.send, vec![late.event_id]);
    assert!(matches!(
        exchange.backward.outcomes.as_slice(),
        [(event, Ok(IngestOutcome::Applied))] if *event == late.event_id
    ));
    assert!(initiator.inventory().contains(&late.event_id));
}
