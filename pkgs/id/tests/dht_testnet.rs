//! DHT opt-in over a real DHT (P7.3, decision 15): the mainline lookup the `dht`
//! feature builds, bootstrapped on a local `n0-mainline` testnet. No public DHT
//! is contacted. Run with `cargo test --features dht --test dht_testnet`.
//!
//! The policy is the one `dht_opt_in.rs` checks: off by default, on for an
//! opted-in world, and off again on the next session.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use id::dht::{DhtPolicy, with_lookup};
use iroh::{
    Endpoint, EndpointId, RelayMap,
    address_lookup::AddressLookupBuilder,
    endpoint::{Connection, RelayMode, presets},
};
use iroh_mainline_address_lookup::DhtAddressLookup;
use n0_mainline::{Dht, Testnet};

const ALPN: &[u8] = b"/id-dht-testnet/1";

/// How long a peer may take to find the other through the DHT. The first
/// publish waits for the endpoint to bootstrap, and the record must reach the
/// testnet before a lookup can find it.
const FIND_LIMIT: Duration = Duration::from_secs(90);

/// The mainline lookup, bootstrapped on the local testnet and not the public DHT.
fn testnet_lookup(testnet: &Testnet) -> impl AddressLookupBuilder {
    let mut dht = Dht::builder();
    dht.bootstrap(&testnet.bootstrap);
    DhtAddressLookup::builder().dht_builder(dht)
}

/// One session of an endpoint under `policy`, on the given relays. The
/// testnet lookup is mounted only if the policy turns the DHT on.
async fn session(policy: &DhtPolicy, testnet: &Testnet, relays: &RelayMap) -> Endpoint {
    let builder = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(relays.clone()))
        .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
        .alpns(vec![ALPN.to_vec()]);
    with_lookup(policy, builder, testnet_lookup(testnet))
        .bind()
        .await
        .unwrap()
}

/// A peer that accepts on [`ALPN`] until its connections close.
fn accept_on(endpoint: &Endpoint) {
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
}

/// Dials `peer` by ID alone, retrying until the DHT has its record or
/// [`FIND_LIMIT`] passes.
async fn dial_until_found(client: &Endpoint, peer: EndpointId) -> anyhow::Result<Connection> {
    let started = Instant::now();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), client.connect(peer, ALPN)).await {
            Ok(Ok(conn)) => return Ok(conn),
            Ok(Err(err)) if started.elapsed() >= FIND_LIMIT => return Err(err.into()),
            Err(_elapsed) if started.elapsed() >= FIND_LIMIT => {
                anyhow::bail!("no connection within {FIND_LIMIT:?}")
            }
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

/// A single dial by ID with a short deadline. A `false` means it did not connect.
async fn dial_once(client: &Endpoint, peer: EndpointId) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(15), client.connect(peer, ALPN)).await,
        Ok(Ok(_))
    )
}

#[tokio::test]
async fn test_opted_in_peers_find_each_other_through_the_dht() {
    let testnet = Testnet::new(3).await.unwrap();
    let (relays, _url, _relay) = iroh::test_utils::run_relay_server().await.unwrap();
    let policy = DhtPolicy::from_worlds(["world-a"]);

    let server = session(&policy, &testnet, &relays).await;
    accept_on(&server);
    let client = session(&policy, &testnet, &relays).await;

    let conn = dial_until_found(&client, server.id())
        .await
        .expect("the peer was not found through the DHT");
    assert_eq!(conn.remote_id(), server.id());

    conn.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn test_opting_out_stops_finding_peers_on_the_next_session() {
    let testnet = Testnet::new(3).await.unwrap();
    let (relays, _url, _relay) = iroh::test_utils::run_relay_server().await.unwrap();
    let on = DhtPolicy::from_worlds(["world-a"]);

    let server = session(&on, &testnet, &relays).await;
    accept_on(&server);
    let client = session(&on, &testnet, &relays).await;
    dial_until_found(&client, server.id())
        .await
        .expect("the peer was not found through the DHT while opted in");
    client.close().await;

    // The world opts out. Its next session has no DHT, so the record is on the
    // testnet but this client does not look for it.
    let off = DhtPolicy::from_worlds(Vec::<String>::new());
    let opted_out = session(&off, &testnet, &relays).await;
    assert!(
        !dial_once(&opted_out, server.id()).await,
        "a session with the DHT off found the peer through the DHT"
    );

    // Control: a session that is still opted in finds the same record, so the
    // failure above came from the policy and not from a missing record.
    let control = session(&on, &testnet, &relays).await;
    dial_until_found(&control, server.id())
        .await
        .expect("the control session could not find the peer");

    control.close().await;
    opted_out.close().await;
    server.close().await;
}
