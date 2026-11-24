//! Automatic chained-relay fan-out with the production sink (#5534).
//!
//! Oracle: the #2036 design record (`doc/2026-11-01_design_id2036.md`
//! §4.1 "deliver locally, then fan out in the same manner" and §5
//! invariants 1-3) and the five-node `PoC` in `tests/relay_poc.rs`
//! (#2249), which proved fan-out with a sink that lived inside the
//! test. This suite runs the same fan-out through
//! [`id_sync::forwarding_sink::ForwardingSink`] — production code — so
//! a bridge forwards a member's chained message to the next peer with
//! no hand-driven `forward_message` calls, excluding the neighbour it
//! arrived from.
//!
//! Topology: origin sends to bridge; bridge's visible neighbours are
//! {origin, leaf}; the message reaches leaf automatically. The origin
//! also serves the relay ALPN with a recording sink, so an echo back
//! to the arrival neighbour would be observed there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use id_sync::chained_message::{ChainedContent, ChainedEnvelope, RelayedMessage};
use id_sync::envelope_outbox::Outcome;
use id_sync::forwarding_sink::{ForwardingSink, LocalSink};
use id_sync::relay_gate::{AllowlistFrontier, RelayGate};
use id_sync::relay_ingest::{CHAINED_RELAY_ALPN, ChainedRelayProtocol, forward_message};
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use tokio::time::sleep;

const GROUP: &str = "group-1";
/// How long a forward may take before the run fails.
const FANOUT_TIMEOUT: Duration = Duration::from_secs(10);
/// Settle window for the negative (no-echo) check.
const ECHO_SETTLE: Duration = Duration::from_millis(250);

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn key_id(seed: u8) -> String {
    hex_encode(&key(seed).verifying_key().to_bytes())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn envelope(origin_seed: u8, sequence: u64) -> ChainedEnvelope {
    ChainedEnvelope::signed(
        &key(origin_seed),
        GROUP,
        sequence,
        Vec::new(),
        "durable",
        ChainedContent::Inline {
            bytes: b"fan-out test".to_vec(),
        },
        1_000,
    )
    .unwrap()
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

fn loopback_addr(ep: &Endpoint) -> EndpointAddr {
    EndpointAddr::from_parts(
        ep.id(),
        ep.bound_sockets().iter().map(|addr| {
            let ip = if addr.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                addr.ip()
            };
            TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
        }),
    )
}

/// Records local deliveries.
#[derive(Clone, Default)]
struct CollectSink(Arc<Mutex<Vec<RelayedMessage>>>);

impl CollectSink {
    fn received(&self) -> Vec<RelayedMessage> {
        self.0.lock().unwrap().clone()
    }
}

impl LocalSink for CollectSink {
    fn deliver(&self, forwarded: &RelayedMessage) {
        self.0.lock().unwrap().push(forwarded.clone());
    }
}

impl id_sync::relay_ingest::ChainedSink for CollectSink {
    fn deliver(&self, forwarded: &RelayedMessage) {
        LocalSink::deliver(self, forwarded);
    }
}

fn member_protocol<S: id_sync::relay_ingest::ChainedSink + Clone + Send + Sync + 'static>(
    relay_seed: u8,
    origin_seed: u8,
    sink: S,
) -> ChainedRelayProtocol<AllowlistFrontier, S> {
    let mut frontier = AllowlistFrontier::new();
    frontier.allow(GROUP, &key_id(origin_seed));
    ChainedRelayProtocol::new(RelayGate::new(&key(relay_seed), frontier), sink)
}

async fn wait_for_delivery(sink: &CollectSink, message_id: &str) {
    let deadline = Instant::now() + FANOUT_TIMEOUT;
    while Instant::now() < deadline {
        if sink
            .received()
            .iter()
            .any(|msg| msg.message_id().unwrap() == message_id)
        {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("no delivery of {message_id} within {FANOUT_TIMEOUT:?}");
}

#[tokio::test]
async fn bridge_forwards_to_next_peer_but_not_back_to_arrival() {
    // Acceptance: a relay with automatic fan-out carries a member's
    // chained message on to a peer it was not addressed to, hop chain
    // extended by the bridge, and does not echo it back to the
    // neighbour it arrived from.
    let origin = endpoint().await;
    let bridge = endpoint().await;
    let leaf = endpoint().await;

    let origin_sink = CollectSink::default();
    let _origin_router = Router::builder(origin.clone())
        .accept(
            CHAINED_RELAY_ALPN,
            member_protocol(1, 1, origin_sink.clone()),
        )
        .spawn();

    let bridge_sink = CollectSink::default();
    let bridge_sink_forwards = ForwardingSink::new(
        key_id(7),
        bridge.clone(),
        vec![
            (key_id(1), loopback_addr(&origin)),
            (key_id(8), loopback_addr(&leaf)),
        ],
        bridge_sink.clone(),
    );
    let bridge_protocol = member_protocol(7, 1, bridge_sink_forwards);
    let _bridge_router = Router::builder(bridge.clone())
        .accept(CHAINED_RELAY_ALPN, bridge_protocol.clone())
        .spawn();

    let leaf_sink = CollectSink::default();
    let leaf_protocol = member_protocol(8, 1, leaf_sink.clone());
    let _leaf_router = Router::builder(leaf.clone())
        .accept(CHAINED_RELAY_ALPN, leaf_protocol.clone())
        .spawn();

    let msg = RelayedMessage::new(envelope(1, 0));
    let id = msg.message_id().unwrap();

    let outcome = forward_message(&origin, loopback_addr(&bridge), &msg).await;
    assert!(
        matches!(outcome, Outcome::Delivered),
        "bridge must accept the origin's message, got {outcome:?}"
    );

    // Local delivery happened at the bridge.
    assert_eq!(bridge_sink.received().len(), 1);

    // The production sink fanned out to the leaf with no hand-driven
    // forward: the leaf has the origin's envelope byte-identical and
    // both relays' hops in order.
    wait_for_delivery(&leaf_sink, &id).await;
    let delivered = leaf_sink.received();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].envelope, msg.envelope);
    assert_eq!(delivered[0].hops.len(), 2);
    assert_eq!(delivered[0].hops[0].relay, key_id(7));
    assert_eq!(delivered[0].hops[1].relay, key_id(8));
    assert_eq!(
        delivered[0].hops[1].previous_hop.as_deref(),
        Some(key_id(7).as_str())
    );
    delivered[0].verify_envelope().unwrap();
    delivered[0].check_hops().unwrap();
    assert_eq!(bridge_protocol.forwarded(), 1);

    // No echo back to the arrival neighbour (or to itself).
    sleep(ECHO_SETTLE).await;
    assert!(
        origin_sink.received().is_empty(),
        "the bridge must not forward the message back to the arrival neighbour"
    );
    assert_eq!(bridge_sink.received().len(), 1);
}
