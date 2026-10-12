//! DHT opt-in (P7.3, decision 15): the mainline DHT is off by default. A world
//! that opts in turns it on for the endpoint, and a world that opts out again
//! turns it off on the next session.
//!
//! No public DHT is contacted here. The DHT stands in as a [`TestDht`]: an
//! address lookup over a `MemoryLookup` that counts what the endpoint asks of
//! it. Real-network DHT runs are owner-blocked (P7.5).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::anyhow;
use futures_lite::stream::Boxed;
use id::{
    connectivity,
    dht::{DhtPolicy, with_lookup},
};
use iroh::{
    Endpoint, EndpointId,
    address_lookup::{AddressLookup, EndpointData, Error, Item, MemoryLookup},
    endpoint::{Connection, RelayMode, presets},
};

const ALPN: &[u8] = b"/id-dht-test/1";

/// How long a check that something does not happen waits for it to.
const SETTLE: Duration = Duration::from_millis(800);

/// How long a dial may take before the test calls it failed.
const DIAL_LIMIT: Duration = Duration::from_secs(15);

/// A stand-in for the mainline DHT. Peers' addresses live in a `MemoryLookup`,
/// and the counters record what the endpoint asks of it. Clones share the
/// counters, so one clone can be handed to each session.
#[derive(Debug, Clone, Default)]
struct TestDht {
    peers: MemoryLookup,
    publishes: Arc<AtomicUsize>,
    resolved: Arc<Mutex<Vec<EndpointId>>>,
}

impl TestDht {
    fn publishes(&self) -> usize {
        self.publishes.load(Ordering::SeqCst)
    }

    fn resolved(&self) -> Vec<EndpointId> {
        self.resolved.lock().unwrap().clone()
    }

    fn reset(&self) {
        self.publishes.store(0, Ordering::SeqCst);
        self.resolved.lock().unwrap().clear();
    }
}

impl AddressLookup for TestDht {
    fn publish(&self, _data: &EndpointData) {
        self.publishes.fetch_add(1, Ordering::SeqCst);
    }

    fn resolve(&self, endpoint_id: EndpointId) -> Option<Boxed<Result<Item, Error>>> {
        self.resolved.lock().unwrap().push(endpoint_id);
        self.peers.resolve(endpoint_id)
    }
}

/// A loopback peer that accepts on [`ALPN`]. It has no lookup of its own, so a
/// dial by ID finds it only through the test DHT.
async fn peer() -> Endpoint {
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let accepting = endpoint.clone();
    tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            tokio::spawn(async move {
                if let Ok(conn) = incoming.await {
                    conn.closed().await;
                }
            });
        }
    });
    endpoint
}

/// One session of the client: an endpoint built under `policy`. The test DHT
/// is mounted only if the policy turns it on.
async fn session(policy: &DhtPolicy, dht: &TestDht) -> Endpoint {
    with_lookup(
        policy,
        Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
        dht.clone(),
    )
    .bind()
    .await
    .unwrap()
}

/// Dials `peer` by node ID alone, so the address can only come from a lookup.
async fn dial_by_id(client: &Endpoint, peer: EndpointId) -> anyhow::Result<Connection> {
    tokio::time::timeout(DIAL_LIMIT, connectivity::connect(client, peer, ALPN))
        .await
        .map_err(|_| anyhow!("the dial did not finish within {DIAL_LIMIT:?}"))?
}

/// Waits until the endpoint has published to the test DHT, for at most ten seconds.
async fn wait_for_publish(dht: &TestDht) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if dht.publishes() > 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn test_dht_is_off_by_default_and_no_lookup_or_publish_happens() {
    let peer = peer().await;
    let dht = TestDht::default();
    dht.peers.add_endpoint_info(peer.addr());

    let policy = DhtPolicy::default();
    assert!(!policy.is_on(), "the default policy turned the DHT on");
    let client = session(&policy, &dht).await;

    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        dht.publishes(),
        0,
        "the endpoint published with the DHT off"
    );
    assert!(
        dht.resolved().is_empty(),
        "the endpoint looked up {:?} with the DHT off",
        dht.resolved()
    );
    assert!(
        dial_by_id(&client, peer.id()).await.is_err(),
        "a dial by ID connected with the DHT off, so a lookup must have run"
    );

    client.close().await;
    peer.close().await;
}

#[tokio::test]
async fn test_world_opt_in_turns_on_publish_and_lookup_for_its_peer() {
    let peer = peer().await;
    let dht = TestDht::default();
    dht.peers.add_endpoint_info(peer.addr());

    let policy = DhtPolicy::from_worlds(["world-a"]);
    assert!(policy.is_on(), "an opted-in world left the DHT off");
    let client = session(&policy, &dht).await;

    assert!(
        wait_for_publish(&dht).await,
        "the endpoint never published to the DHT with a world opted in"
    );
    let conn = dial_by_id(&client, peer.id())
        .await
        .expect("the DHT lookup did not find the peer");
    assert_eq!(conn.remote_id(), peer.id());
    assert!(
        dht.resolved().contains(&peer.id()),
        "no lookup for the peer; resolved {:?}",
        dht.resolved()
    );

    conn.close(0u32.into(), b"done");
    client.close().await;
    peer.close().await;
}

#[tokio::test]
async fn test_opting_out_stops_publish_and_lookup_on_the_next_session() {
    let peer = peer().await;
    let dht = TestDht::default();
    dht.peers.add_endpoint_info(peer.addr());

    let on = DhtPolicy::from_worlds(["world-a"]);
    let client = session(&on, &dht).await;
    assert!(wait_for_publish(&dht).await, "no publish while opted in");
    dial_by_id(&client, peer.id())
        .await
        .expect("the DHT lookup did not find the peer while opted in");
    client.close().await;

    // The world opts out. Its next session is built with no worlds opted in.
    dht.reset();
    let off = DhtPolicy::from_worlds(Vec::<String>::new());
    assert!(!off.is_on(), "no opted-in world left the DHT on");
    let client = session(&off, &dht).await;

    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        dht.publishes(),
        0,
        "the endpoint published after the world opted out"
    );
    assert!(
        dht.resolved().is_empty(),
        "the endpoint looked up {:?} after the world opted out",
        dht.resolved()
    );
    assert!(
        dial_by_id(&client, peer.id()).await.is_err(),
        "a dial by ID connected after the world opted out"
    );

    client.close().await;
    peer.close().await;
}
