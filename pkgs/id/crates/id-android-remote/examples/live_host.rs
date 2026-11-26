//! Start a local Iroh world host for the Android emulator proof of concept.
//!
//! Oracle: the Android shell connects to the printed Iroh node ID, uses the
//! printed guest capability, and renders the hosted tic-tac-toe view.

use std::net::SocketAddr;

use id_worlds::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
use id_worlds::world_net::{WORLD_ALPN, WorldProtocol};
use id_worlds::world_session::{WorldService, module_hash};
use iroh::Endpoint;
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh_blobs::store::mem::MemStore;

const TIC_TAC_TOE: &[u8] = include_bytes!("../../../apps/tic-tac-toe/program/tic-tac-toe.wasm");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default())?);
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
        .map_err(|error| anyhow::anyhow!("could not install guest wasm: {error:?}"))?;
    let (_, capability) = world.issue("android-poc", WorldScopes::GUEST).await?;

    let host = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let router = Router::builder(host.clone())
        .accept(WORLD_ALPN, WorldProtocol::new(service))
        .spawn();
    let port = host
        .bound_sockets()
        .iter()
        .find(|socket| socket.is_ipv4())
        .map(SocketAddr::port)
        .ok_or_else(|| anyhow::anyhow!("host did not bind an IPv4 socket"))?;
    println!("ANDROID_ENDPOINT_ADDR={}@10.0.2.2:{port}", host.id());
    println!("ANDROID_JOIN_CAPABILITY={}", capability.expose());
    println!("Host ready; press Ctrl-C to stop.");
    tokio::signal::ctrl_c().await?;

    router.shutdown().await?;
    host.close().await;
    Ok(())
}
