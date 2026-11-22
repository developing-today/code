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
//! Venue: like `substrate_transport_path`, the sessions ride
//! QUIC/UDP on loopback, so this test needs UDP sends to be permitted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use tokio::sync::Mutex;

use id_core::connectivity::connect;
use id_core::frames::{read_frame, write_frame};
use id_sync::federation::exchange_net::{
    EXCHANGE_ALPN, ExchangeProtocol, Hello, HelloReply, reconcile_remote,
};
use id_sync::federation::slices::b07_conflict_report::{ConflictStore, RetentionPolicy};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::Replica;
use id_sync::federation::slices::b32_ingest_gate::FreshnessWindow;

const SCOPE: &str = "notes";
const TRUST_DOMAIN: &str = "home";
const NOW: u64 = 10_000;

/// Well under the responder's 30-second session timeout: anything
/// that waits on the stalled session's lock fails this bound.
const PROMPT: Duration = Duration::from_secs(5);

fn node(name: &str) -> Replica {
    let freshness = FreshnessWindow {
        max_age: 1_000_000,
        max_future_skew: 30,
    };
    Replica::new(
        name,
        TRUST_DOMAIN,
        ConflictStore::new(Vec::new(), freshness, RetentionPolicy { retain_for: 3_600 }),
    )
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
    let hello = Hello {
        name: "stalled".to_owned(),
        scope: config.scope.clone(),
        trust_domain: config.trust_domain.clone(),
        capacity: config.capacity,
        retention: config.retention,
        now: NOW,
        summary: Vec::new(),
        refused: Vec::new(),
    };
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
