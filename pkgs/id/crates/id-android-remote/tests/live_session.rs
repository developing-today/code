//! Iroh loopback test for the Android remote-drive client.
//!
//! Oracle: the capability-authorized `/id-world/1` `WorldService` emits a
//! contract-valid view, and the Android bridge returns that view unchanged.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::Duration;

use id_android_remote::{join_world_session, parse_endpoint_addr};
use id_worlds::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
use id_worlds::world_net::{WORLD_ALPN, WorldProtocol};
use id_worlds::world_session::{WorldService, module_hash};
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use iroh_blobs::store::mem::MemStore;

const TIC_TAC_TOE: &[u8] = include_bytes!("../../../apps/tic-tac-toe/program/tic-tac-toe.wasm");
const TIMEOUT: Duration = Duration::from_secs(30);

async fn loopback_endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("bind loopback Iroh endpoint")
}

#[tokio::test]
async fn client_joins_live_world_and_receives_shell_contract_view() {
    let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
    let service = WorldService::new(world.clone(), Some("admin".to_owned()))
        .with_blob_store(MemStore::new().into());
    service
        .install_wasm(
            Some("admin"),
            TIC_TAC_TOE.to_vec(),
            7,
            &module_hash(TIC_TAC_TOE),
        )
        .await
        .unwrap();
    let (_, capability) = world
        .issue("android-poc", WorldScopes::GUEST)
        .await
        .unwrap();

    let host = loopback_endpoint().await;
    let router = Router::builder(host.clone())
        .accept(WORLD_ALPN, WorldProtocol::new(service))
        .spawn();
    let addr = EndpointAddr::from_parts(
        host.id(),
        host.bound_sockets().iter().map(|socket| {
            let ip = if socket.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                socket.ip()
            };
            TransportAddr::Ip(std::net::SocketAddr::new(ip, socket.port()))
        }),
    );
    assert_eq!(
        parse_endpoint_addr(&host.id().to_string()).unwrap(),
        EndpointAddr::from(host.id())
    );
    assert_eq!(
        parse_endpoint_addr(&serde_json::to_string(&addr).unwrap()).unwrap(),
        addr
    );
    let client_endpoint = loopback_endpoint().await;

    let (_session, view) = tokio::time::timeout(
        TIMEOUT,
        join_world_session(&client_endpoint, addr, Some("lobby"), capability.expose()),
    )
    .await
    .expect("Android bridge joins before the deadline")
    .expect("the capability-authorized Iroh session succeeds");

    assert!(
        view.sequence > 0,
        "the host view names its durable sequence"
    );
    assert!(
        !view.data.trim().is_empty(),
        "the host view has a non-empty text projection"
    );
    client_endpoint.close().await;
    host.close().await;
    router.shutdown().await.unwrap();
}
