//! Five-node bridge-relay PoC — integration test (#2249).
//!
//! Oracle: the #2036 design record (`doc/2026-11-01_design_id2036.md`
//! §4.1 and §5 invariants 1–3) and issue #2249's acceptance list, built
//! on the #2035 investigation's relay model
//! (`doc/2026-10-29T11-05-00Z_investigation_multi_hop_relay_and_p2p_media/`).
//!
//! Topology (the #2035 visibility graph): four mutually visible members
//! {A, B, C, D} plus E reachable only through D. Chained messages fan
//! out over direct Iroh connections on [`CHAINED_RELAY_ALPN`], each
//! relay's gate validating before forwarding (the #3796 phase-3
//! machinery — stock iroh-gossip is NOT the carrier, per the
//! gate-verification finding). A node fans out to its visible
//! neighbours, excluding the neighbour the message arrived from; the
//! arrival neighbour is read off the hop chain (the hop before this
//! node's own, or the origin for a direct delivery).
//!
//! "E firewalled from all but D at the application layer" means the
//! visibility graph is enforced by the sender neighbour sets: only D
//! ever dials E. E's router would still accept a connection from
//! anyone who learned its address — there is no transport-level block
//! in this PoC, and the report says so plainly.
//!
//! What this PoC is: the first end-to-end exercise of the relay path
//! with automatic fan-out (the `ChainedSink` contract says "deliver
//! locally, then fan out" — until now no implementation did the second
//! half), plus the latency measurement. What it is NOT: the permanent
//! feature. No production code changes in this slice; the forwarding
//! sink lives here, in the test, and is disposable by design.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use id_sync::chained_message::{ChainedContent, ChainedEnvelope, RelayedMessage};
use id_sync::envelope_outbox::Outcome;
use id_sync::relay_gate::{AllowlistFrontier, RelayGate};
use id_sync::relay_ingest::{
    CHAINED_RELAY_ALPN, ChainedRelayProtocol, ChainedSink, forward_message,
};
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use tokio::time::sleep;

const GROUP: &str = "group-1";
/// Messages per latency run (acceptance: delivery completeness + medians).
const LATENCY_MESSAGES: u64 = 20;
/// How long one message may take to reach E before the run fails.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

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
            bytes: format!("poc message {sequence}").into_bytes(),
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

/// One delivery as the sink saw it: the forwarded message as
/// delivered (envelope + hop chain) and the arrival instant
/// (latency oracle).
#[derive(Clone, Debug)]
struct Delivery {
    message: RelayedMessage,
    at: Instant,
}

impl Delivery {
    fn message_id(&self) -> String {
        self.message.message_id().unwrap()
    }

    fn hops(&self) -> Vec<String> {
        self.message
            .hops
            .iter()
            .map(|hop| hop.relay.clone())
            .collect()
    }
}

/// The PoC forwarding sink: records the delivery, then fans the
/// forwarded message out to this node's visible neighbours, excluding
/// the neighbour it arrived from (and itself). This is the
/// "in the same manner" fan-out of #2035 §2.1, automated for the
/// first time — the #3796 tests forwarded by hand.
#[derive(Clone)]
struct ForwardingSink {
    self_id: String,
    endpoint: Endpoint,
    neighbours: Arc<Vec<(String, EndpointAddr)>>,
    delivered: Arc<Mutex<Vec<Delivery>>>,
}

impl ChainedSink for ForwardingSink {
    fn deliver(&self, forwarded: &RelayedMessage) {
        let hops: Vec<String> = forwarded.hops.iter().map(|hop| hop.relay.clone()).collect();
        self.delivered.lock().unwrap().push(Delivery {
            message: forwarded.clone(),
            at: Instant::now(),
        });
        // The arrival neighbour is the hop before this node's own;
        // a direct delivery (one hop: this node) arrived from the
        // origin. Exclude it — and ourselves — from the fan-out.
        let arrival = if hops.len() >= 2 {
            hops[hops.len() - 2].clone()
        } else {
            forwarded.envelope.origin.clone()
        };
        for (neighbour_id, addr) in self.neighbours.iter() {
            if *neighbour_id == arrival || *neighbour_id == self.self_id {
                continue;
            }
            let endpoint = self.endpoint.clone();
            let addr = addr.clone();
            let msg = forwarded.clone();
            tokio::spawn(async move {
                let _ = forward_message(&endpoint, addr, &msg).await;
            });
        }
    }
}

/// One PoC node: endpoint, router (held alive), gate protocol, and
/// delivery log. Neighbours are the visibility graph: the relay IDs
/// this node fans out to.
struct PocNode {
    name: &'static str,
    id: String,
    endpoint: Endpoint,
    addr: EndpointAddr,
    delivered: Arc<Mutex<Vec<Delivery>>>,
    protocol: ChainedRelayProtocol<AllowlistFrontier, ForwardingSink>,
    _router: Router,
}

impl PocNode {
    /// Build a node on the given endpoint, whose gate authorizes
    /// `origin_seed` for the group. `neighbours` maps relay IDs to
    /// addresses; it is shared with the sink so fan-out and the node
    /// agree on the graph. The endpoint is the caller's: the
    /// addresses other nodes dial must be this endpoint's, or the
    /// router behind them never sees the traffic.
    async fn new(
        name: &'static str,
        seed: u8,
        origin_seed: u8,
        endpoint: Endpoint,
        addr: EndpointAddr,
        neighbours: Arc<Vec<(String, EndpointAddr)>>,
    ) -> Self {
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let sink = ForwardingSink {
            self_id: key_id(seed),
            endpoint: endpoint.clone(),
            neighbours,
            delivered: Arc::clone(&delivered),
        };
        let mut frontier = AllowlistFrontier::new();
        frontier.allow(GROUP, &key_id(origin_seed));
        let protocol = ChainedRelayProtocol::new(RelayGate::new(&key(seed), frontier), sink);
        let router = Router::builder(endpoint.clone())
            .accept(CHAINED_RELAY_ALPN, protocol.clone())
            .spawn();
        Self {
            name,
            id: key_id(seed),
            endpoint,
            addr,
            delivered,
            protocol,
            _router: router,
        }
    }

    fn delivered(&self) -> Vec<Delivery> {
        self.delivered.lock().unwrap().clone()
    }

    fn delivered_count(&self, message_id: &str) -> usize {
        self.delivered()
            .iter()
            .filter(|d| d.message_id() == message_id)
            .count()
    }
}

/// The five-node visibility graph: A, B, C, D mutually visible; E
/// visible only to D (`extra_bridges` adds further bridges to E, for
/// the duplicate-collapse test). Returns the nodes keyed by name.
async fn five_nodes(extra_bridges: &[(&'static str, u8)]) -> HashMap<&'static str, PocNode> {
    // Endpoints first: addresses are needed before any sink is built.
    let mut endpoints: HashMap<&'static str, (u8, Endpoint, EndpointAddr)> = HashMap::new();
    for (name, seed) in [
        ("a", 1u8),
        ("b", 2),
        ("c", 3),
        ("d", 4),
        ("e", 5),
    ] {
        let ep = endpoint().await;
        let addr = loopback_addr(&ep);
        endpoints.insert(name, (seed, ep, addr));
    }
    for (name, seed) in extra_bridges {
        let ep = endpoint().await;
        let addr = loopback_addr(&ep);
        endpoints.insert(name, (*seed, ep, addr));
    }
    let id_of = |name: &str| key_id(endpoints[name].0);
    let addr_of = |name: &str| endpoints[name].2.clone();

    // Neighbour sets (visibility graph). E is dialled only by its
    // bridges; A/B/C never hold E's address.
    let mut bridges = vec!["d"];
    bridges.extend(extra_bridges.iter().map(|(name, _)| *name));
    let core = ["a", "b", "c"];
    let neighbours_of = |name: &str| -> Arc<Vec<(String, EndpointAddr)>> {
        let mut names: Vec<&str> = Vec::new();
        if core.contains(&name) {
            names.extend(core.iter().filter(|n| **n != name).copied());
            names.extend(bridges.iter().copied());
        } else if bridges.contains(&name) {
            names.extend(core.iter().copied());
            names.push("e");
        } else if name == "e" {
            names.extend(bridges.iter().copied());
        }
        Arc::new(
            names
                .into_iter()
                .map(|n| (id_of(n), addr_of(n)))
                .collect(),
        )
    };

    let mut nodes = HashMap::new();
    for (name, (seed, endpoint, addr)) in &endpoints {
        let node = PocNode::new(
            name,
            *seed,
            1,
            endpoint.clone(),
            addr.clone(),
            neighbours_of(name),
        )
        .await;
        nodes.insert(*name, node);
    }
    nodes
}

/// Wait until `node` has delivered `message_id`, or fail the test.
async fn wait_for_delivery(node: &PocNode, message_id: &str) {
    let deadline = Instant::now() + DELIVERY_TIMEOUT;
    while Instant::now() < deadline {
        if node.delivered_count(message_id) > 0 {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "node {} never delivered message {message_id} within {DELIVERY_TIMEOUT:?}",
        node.name
    );
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

#[tokio::test]
async fn five_node_bridge_delivery_with_latency() {
    // Acceptance 1 + 2: A's chained message reaches E via D with a
    // verifiable hop chain, over automatic fan-out — no hand-driven
    // forwards. Latencies are measured, not asserted (loopback timing
    // is environment-dependent); the report records the medians.
    let nodes = five_nodes(&[]).await;
    let (a, b, c, d, e) = (
        &nodes["a"], &nodes["b"], &nodes["c"], &nodes["d"], &nodes["e"],
    );
    let mut direct = Vec::new();
    let mut bridged = Vec::new();
    for sequence in 0..LATENCY_MESSAGES {
        let msg = RelayedMessage::new(envelope(1, sequence));
        let id = msg.message_id().unwrap();
        let started = Instant::now();
        for target in [b, c, d] {
            let outcome = forward_message(&a.endpoint, target.addr.clone(), &msg).await;
            assert!(
                matches!(outcome, Outcome::Delivered),
                "A must reach {}, got {outcome:?}",
                target.name
            );
        }
        wait_for_delivery(e, &id).await;
        wait_for_delivery(b, &id).await;
        let e_delivery = e
            .delivered()
            .into_iter()
            .find(|delivery| delivery.message_id() == id)
            .unwrap();
        let b_delivery = b
            .delivered()
            .into_iter()
            .find(|delivery| delivery.message_id() == id)
            .unwrap();
        bridged.push(e_delivery.at - started);
        direct.push(b_delivery.at - started);

        // Exactly-once delivery at every member, envelope identical
        // on every path, hop chain [D, E] in order at E.
        for node in [b, c, d, e] {
            assert_eq!(
                node.delivered_count(&id),
                1,
                "{} must deliver message {sequence} exactly once",
                node.name
            );
        }
        // The hop chain records the path actually taken: D is
        // always the last relay before E, but the message may reach
        // D directly from A ([D, E]) or via B/C's fan-out racing
        // A's direct send ([B|C, D, E]) — both are valid; the chain
        // ends [..., D, E] either way, and check_hops below
        // verifies linkage.
        let hops = e_delivery.hops();
        assert!(
            hops.len() == 2 || hops.len() == 3,
            "hop chain at E must be [D, E] or [B|C, D, E], got {hops:?}"
        );
        assert_eq!(hops[hops.len() - 2], d.id);
        assert_eq!(hops[hops.len() - 1], e.id);
        if hops.len() == 3 {
            assert!(
                hops[0] == b.id || hops[0] == c.id,
                "a 3-hop chain must start at B or C, got {hops:?}"
            );
        }
        // The delivered copy verifies: envelope byte-identical to
        // what A sent (same message ID), origin signature valid,
        // hop chain linked and within the cap, per §4.1.
        assert_eq!(e_delivery.message.envelope, msg.envelope);
        e_delivery.message.verify_envelope().unwrap();
        e_delivery.message.check_hops().unwrap();
        assert_eq!(d.protocol.forwarded() as u64, sequence + 1);
    }
    eprintln!(
        "poc relay latencies over {LATENCY_MESSAGES} messages: \
         direct A->B median {:?}, bridged A->D->E median {:?}",
        median(direct),
        median(bridged)
    );
}

#[tokio::test]
async fn two_bridges_collapse_to_one_delivery() {
    // Acceptance 3: with a second bridge D2, E receives duplicate
    // copies and delivers exactly one.
    let nodes = five_nodes(&[("d2", 6)]).await;
    let (a, d, d2, e) = (&nodes["a"], &nodes["d"], &nodes["d2"], &nodes["e"]);
    for sequence in 0..5u64 {
        let msg = RelayedMessage::new(envelope(1, sequence));
        let id = msg.message_id().unwrap();
        for target in [&nodes["b"], &nodes["c"], d, d2] {
            let outcome = forward_message(&a.endpoint, target.addr.clone(), &msg).await;
            assert!(
                matches!(outcome, Outcome::Delivered),
                "A must reach {}, got {outcome:?}",
                target.name
            );
        }
        wait_for_delivery(e, &id).await;
        assert_eq!(
            e.delivered_count(&id),
            1,
            "E must deliver message {sequence} exactly once despite two bridges"
        );
        // Both bridges forwarded it (each exactly once); E's single
        // delivery is the dedupe, not a lost copy.
        assert_eq!(d.protocol.forwarded() as u64, sequence + 1);
        assert_eq!(d2.protocol.forwarded() as u64, sequence + 1);
    }
}

#[tokio::test]
async fn tampered_copy_is_refused_in_five_node_topology() {
    // Acceptance 2, second half: a tampered relayed copy is refused
    // at ingest — nothing delivered anywhere — and the topology still
    // carries a valid message afterwards.
    let nodes = five_nodes(&[]).await;
    let (a, d, e) = (&nodes["a"], &nodes["d"], &nodes["e"]);
    let mut forged = envelope(1, 0);
    forged.content = ChainedContent::Inline {
        bytes: b"forged".to_vec(),
    };
    let outcome = forward_message(
        &a.endpoint,
        d.addr.clone(),
        &RelayedMessage::new(forged),
    )
    .await;
    assert!(
        matches!(outcome, Outcome::Refused(_)),
        "tampered copy must be refused at D, got {outcome:?}"
    );
    assert_eq!(d.protocol.forwarded(), 0);
    assert!(d.delivered().is_empty());
    assert!(e.delivered().is_empty());

    let msg = RelayedMessage::new(envelope(1, 1));
    let id = msg.message_id().unwrap();
    let outcome = forward_message(&a.endpoint, d.addr.clone(), &msg).await;
    assert!(
        matches!(outcome, Outcome::Delivered),
        "valid message must still forward, got {outcome:?}"
    );
    wait_for_delivery(e, &id).await;
    assert_eq!(e.delivered_count(&id), 1);
}
