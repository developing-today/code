//! Live-host durable-outcome parity for design #788 step 2.
//!
//! The frame/presentation parity in `tests/parity.rs` proves the
//! native shell and the terminal shell build the same frames and
//! present the same text. This test proves the remaining half of the
//! design's parity criterion — the same scripted frames into a LIVE
//! host produce the same durable outcome — by playing one tic-tac-toe
//! game two ways at once: participant "ann" plays through the native
//! shell's live session loop ([`run_session`]), participant "bo"
//! through a raw `WorldClient` exactly as the terminal shell drives
//! it. The oracle is the host's own journal, exported afterwards
//! through the admin fixture recorder: both shells' inputs must land
//! as the same durable `Input` events, in order, and the recorded
//! records must equal the live world's records.
//!
//! Oracle: the host journal / fixture records (durable outcome), not
//! either client's local view of the game.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use id_native_shell::{SessionView, input_frame, join_frame, run_session};
use id_worlds::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
use id_worlds::world_fixture::{FixtureEvent, record};
use id_worlds::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id_worlds::world_session::{WorldService, module_hash};
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use iroh_blobs::store::mem::MemStore;
use tokio::sync::mpsc;

const TIC_TAC_TOE: &[u8] = include_bytes!("../../../apps/tic-tac-toe/program/tic-tac-toe.wasm");
const TIMEOUT: Duration = Duration::from_secs(30);

/// A loopback endpoint that uses no relay or discovery.
async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

/// Serve `service` over iroh on loopback (mirrors the world_fixture
/// test harness).
async fn serve(service: WorldService) -> (Router, EndpointAddr) {
    let ep = endpoint().await;
    let router = Router::builder(ep.clone())
        .accept(WORLD_ALPN, WorldProtocol::new(service))
        .spawn();
    let addr = EndpointAddr::from_parts(
        ep.id(),
        ep.bound_sockets().iter().map(|socket| {
            let ip = if socket.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                socket.ip()
            };
            TransportAddr::Ip(std::net::SocketAddr::new(ip, socket.port()))
        }),
    );
    (router, addr)
}

/// Wait until the native session publishes a view at or past
/// `sequence` (views follow the durable events they present).
async fn wait_for_view(views: &mut mpsc::UnboundedReceiver<SessionView>, sequence: u64) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let view = views.recv().await.expect("session view channel closed");
            if view.sequence >= sequence {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for a native view at sequence {sequence}"));
}

/// Wait until the terminal client receives the durable `event` frame
/// for `sequence`, skipping snapshots, views, and earlier events.
async fn wait_for_event(client: &mut WorldClient, sequence: u64) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let frame = client
                .recv_json()
                .await
                .unwrap()
                .expect("host closed the terminal session");
            if frame["type"] == "event" && frame["event"]["sequence"] == sequence {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for event at sequence {sequence}"));
}

#[tokio::test]
async fn native_loop_and_terminal_client_reach_the_same_durable_outcome() {
    // A live host with the tic-tac-toe program installed (sequence 1
    // is the install event, as in the world_fixture games).
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
    let (_, ann_cap) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let (_, bo_cap) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
    let (router, addr) = serve(service).await;
    let client_ep = endpoint().await;

    // Terminal side: a raw WorldClient driven exactly as the CLI's
    // join pump drives it — join frame in, frames read off the stream.
    let mut terminal = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    terminal
        .send_json(&join_frame(bo_cap.expose(), None))
        .await
        .unwrap();
    let snapshot = terminal.recv_json().await.unwrap().unwrap();
    assert_eq!(snapshot["type"], "snapshot");

    // Native side: the session loop under test. Outbound frames go in
    // over the channel the binary's UI layer will own; views come out
    // over the channel that feeds `PresentedView`.
    let native_client = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    let (outbound_tx, outbound_rx) = mpsc::channel(32);
    let (views_tx, mut views_rx) = mpsc::unbounded_channel();
    let ann_capability = ann_cap.expose().to_owned();
    let mut session = tokio::spawn(async move {
        run_session(
            native_client,
            None,
            &ann_capability,
            None,
            outbound_rx,
            views_tx,
        )
        .await
    });
    // The join's initial view proves the loop is live before any move.
    wait_for_view(&mut views_rx, 1).await;

    // One game, alternating shells: ann (native) plays 0, 1, 2 and
    // bo (terminal) plays 3, 4 — the fixture game's winning line.
    // Durable sequences: install 1, then inputs 2..=6 in play order.
    for (sequence, cell) in [(2_u64, "0"), (4, "1"), (6, "2")] {
        outbound_tx
            .send(input_frame(cell.as_bytes()))
            .await
            .unwrap();
        wait_for_view(&mut views_rx, sequence).await;
        if cell != "2" {
            let bo_cell = if cell == "0" { "3" } else { "4" };
            let bo_sequence = sequence + 1;
            terminal
                .send_json(&input_frame(bo_cell.as_bytes()))
                .await
                .unwrap();
            wait_for_event(&mut terminal, bo_sequence).await;
        }
    }

    // Ending the outbound channel ends the native session; the state
    // it returns must show every durable input event of the game.
    drop(outbound_tx);
    let state = tokio::time::timeout(TIMEOUT, &mut session)
        .await
        .expect("native session did not end")
        .unwrap()
        .unwrap();
    let input_events = state
        .events()
        .iter()
        .filter(|event| event["kind"]["kind"] == "input")
        .count();
    assert_eq!(
        input_events,
        5,
        "native session state: {:?}",
        state.events()
    );
    assert!(state.view().is_some(), "native session never got a view");
    terminal.close();

    // The durable oracle: the host journal, exported through the
    // admin fixture recorder, holds both shells' inputs as identical
    // Input events in play order, and its records match the live
    // world's records.
    let mut admin = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    let fixture = record(&mut admin, "admin").await.unwrap();
    admin.close();
    assert_eq!(fixture.header.current_sequence, 6);
    let inputs: Vec<(u64, Vec<u8>)> = fixture
        .events
        .iter()
        .filter_map(|event| match event {
            FixtureEvent::Input {
                sequence, bytes, ..
            } => Some((*sequence, bytes.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        inputs,
        vec![
            (2, b"0".to_vec()),
            (3, b"3".to_vec()),
            (4, b"1".to_vec()),
            (5, b"4".to_vec()),
            (6, b"2".to_vec()),
        ]
    );
    let (_, live) = world.records();
    assert_eq!(fixture.header.records.as_ref(), Some(&*live));

    client_ep.close().await;
    router.shutdown().await.unwrap();
}
