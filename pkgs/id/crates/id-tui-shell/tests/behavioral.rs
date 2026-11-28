//! Behavioral completion checks (issue #12818, slice-2 item c):
//! join / read / act / reconnect end to end against a LIVE
//! in-process host session, including error-frame refusal and
//! re-snapshot-on-divergence.
//!
//! The unit oracles in `tests/frames_and_session.rs` prove the fold
//! against synthetic frames and the recorded sessions in
//! `tests/suite_session.rs` prove conformance on fixed transcripts.
//! These tests prove the remaining half: the same builder/fold/
//! projection path driven against a real host over loopback iroh,
//! the way the native shell's `live_session.rs` drives it. The
//! oracle is the live world's own durable history — the folded
//! sequences must match the host's, in order — plus the rendered
//! three-panel surface.
//!
//! Out of scope here (recorded in the capability statement's
//! `excludes` or deferred to the terminal loop): the shipped iroh
//! connection loop and the crossterm backend. The form flow stays
//! covered by the recorded sessions: no live program here presents
//! a form.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use id_tui_shell::frames;
use id_tui_shell::present;
use id_tui_shell::render::Prompt;
use id_tui_shell::session::{Session, SessionError, Update};
use id_worlds::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
use id_worlds::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id_worlds::world_session::{WorldService, module_hash};
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use iroh_blobs::store::mem::MemStore;
use serde_json::{Value, json};

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
/// test harness and the native shell's live-session tests).
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

/// Issue a guest capability on a fresh live world and serve it.
/// Returns the endpoint address and both capabilities.
async fn live_world() -> (Router, EndpointAddr, String, String) {
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
    let (_, tui) = world.issue("tui", WorldScopes::GUEST).await.unwrap();
    let (_, bo) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
    let (router, addr) = serve(service).await;
    (
        router,
        addr,
        tui.expose().to_owned(),
        bo.expose().to_owned(),
    )
}

/// Receive the next host frame, failing on timeout or a closed
/// session.
async fn next_frame(client: &mut WorldClient) -> Value {
    tokio::time::timeout(TIMEOUT, client.recv_json())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for a host frame"))
        .unwrap()
        .expect("host closed the session")
}

/// Fold one host frame the statement presents; other wire kinds (the
/// host may send e.g. program-changed notices a presentation client
/// does not fold) are skipped.
fn fold_presented(session: &mut Session, frame: &Value) -> Option<Update> {
    match frame["type"].as_str().unwrap_or_default() {
        "snapshot" | "event" | "view" | "records" | "form_result_status" => {
            Some(session.apply(frame).unwrap())
        }
        _ => None,
    }
}

/// Drain the live stream, folding every presented frame, until the
/// durable event at `sequence` is folded.
async fn fold_until_event(session: &mut Session, client: &mut WorldClient, sequence: u64) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let frame = client
                .recv_json()
                .await
                .unwrap()
                .expect("host closed the session");
            fold_presented(session, &frame);
            if frame["type"] == "event" && frame["event"]["sequence"] == sequence {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for event at sequence {sequence}"));
}

/// Join fresh, folding the snapshot and the first view.
async fn join_fresh(client: &mut WorldClient, capability: &str) -> Session {
    client
        .send_json(&frames::join(capability, None, None).unwrap())
        .await
        .unwrap();
    let mut session = Session::new();
    let snapshot = next_frame(client).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(session.apply(&snapshot), Ok(Update::Snapshot));
    let view = next_frame(client).await;
    assert_eq!(view["type"], "view");
    assert_eq!(session.apply(&view), Ok(Update::View));
    // A resume token arrives only when the client negotiates screen
    // patches (world_session/view.rs); the TUI speaks the text
    // fallback, so a token is optional here. Reconnect continuity
    // below rests on `after`, not the token.
    session
}

fn rendered_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    let size = terminal.backend().buffer().area;
    (0..size.height)
        .flat_map(|y| {
            (0..size.width).map(move |x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, y))
                    .map(|c| c.symbol().to_owned())
                    .unwrap_or_default()
            })
        })
        .collect()
}

#[tokio::test]
async fn live_join_read_act_projects_and_renders() {
    let (router, addr, tui_cap, _bo_cap) = live_world().await;
    let client_ep = endpoint().await;

    let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
    let mut session = join_fresh(&mut client, &tui_cap).await;
    // Install is sequence 1; the fresh join snapshots there.
    assert_eq!(session.last_sequence(), 1);

    // Read: a records query against the live host folds a reply.
    client
        .send_json(&frames::records(&tui_cap, None, None).unwrap())
        .await
        .unwrap();
    let reply = next_frame(&mut client).await;
    assert_eq!(reply["type"], "records");
    assert_eq!(session.apply(&reply), Ok(Update::Records));
    assert!(session.records().is_some());

    // Act: one game move and one chat line are durable events 2 and 3.
    client
        .send_json(&frames::input(b"0", None).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 2).await;
    client
        .send_json(&frames::chat("hello from tui", None).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 3).await;
    let sequences: Vec<u64> = session
        .events()
        .iter()
        .map(|e| e["sequence"].as_u64().unwrap())
        .collect();
    assert_eq!(sequences, vec![2, 3]);

    // The folded live history projects onto the three panels and
    // renders: the chatroom shows our chat line.
    let panels = present::project(&session);
    assert!(
        present::chat_lines(&session)
            .iter()
            .any(|line| line.contains("hello from tui")),
        "chatroom shows the live chat line"
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| id_tui_shell::render::draw(frame, &panels, &Prompt::default()))
        .unwrap();
    assert!(
        rendered_text(&terminal).contains("hello from tui"),
        "the rendered surface shows the live chat line"
    );

    let _ = router;
}

#[tokio::test]
async fn live_error_frame_refuses_without_losing_state() {
    let (router, addr, tui_cap, _bo_cap) = live_world().await;
    let client_ep = endpoint().await;

    let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
    let mut session = join_fresh(&mut client, &tui_cap).await;
    client
        .send_json(&frames::input(b"0", None).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 2).await;
    assert_eq!(session.events().len(), 1);

    // An error frame refuses the session and is never content: the
    // folded event survives and the session keeps working.
    let refused = json!({"type": "error", "message": "admin denied"});
    assert_eq!(
        session.apply(&refused),
        Err(SessionError::Refused("admin denied".to_owned()))
    );
    assert_eq!(session.events().len(), 1, "refusal folds no content");
    assert_eq!(session.last_sequence(), 2);

    client
        .send_json(&frames::chat("still here", None).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 3).await;
    assert_eq!(session.events().len(), 2);

    let _ = router;
}

#[tokio::test]
async fn live_reconnect_resumes_contiguous_and_resnapshots_on_gap() {
    let (router, addr, tui_cap, bo_cap) = live_world().await;
    let client_ep = endpoint().await;

    // Both participants join live; tui folds its move (2) and sees
    // bo's move (3) on its own stream.
    let mut client = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    let mut session = join_fresh(&mut client, &tui_cap).await;
    client
        .send_json(&frames::input(b"0", None).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 2).await;
    let mut bo = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    bo.send_json(&frames::join(&bo_cap, None, None).unwrap())
        .await
        .unwrap();
    // Drain bo's join frames until its stream is live (snapshot then
    // view, possibly with a program-changed notice between).
    for _ in 0..8 {
        let frame = next_frame(&mut bo).await;
        if frame["type"] == "view" {
            break;
        }
    }
    bo.send_json(&frames::input(b"3", None).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 3).await;

    // The tui drops while bo acts again: event 4 happens away.
    session.begin_reconnect();
    let resume = session.resume_token().map(str::to_owned);
    drop(client);
    bo.send_json(&frames::input(b"1", None).unwrap())
        .await
        .unwrap();
    // Drain bo's stream until its own event 4 lands, proving the
    // away-event is durable before the tui returns.
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let frame = bo.recv_json().await.unwrap().expect("host closed bo");
            if frame["type"] == "event" && frame["event"]["sequence"] == 4 {
                break;
            }
        }
    })
    .await
    .unwrap();

    // Reconnect presenting the last applied sequence and the resume
    // token: the host replays contiguously and the fold resumes.
    let mut client = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    client
        .send_json(&frames::join(&tui_cap, Some(3), resume.as_deref()).unwrap())
        .await
        .unwrap();
    fold_until_event(&mut session, &mut client, 4).await;
    assert!(
        !session.diverged(),
        "a contiguous replay resumes, it does not diverge"
    );
    let sequences: Vec<u64> = session
        .events()
        .iter()
        .map(|e| e["sequence"].as_u64().unwrap())
        .collect();
    assert_eq!(sequences, vec![2, 3, 4]);

    // A gapped replay is unprovable continuity (no host can produce
    // a gap on demand, so the gap is induced on live-folded state):
    // the fold marks the session diverged and folds nothing, until
    // a fresh live snapshot re-establishes state.
    session.begin_reconnect();
    let gapped = json!({
        "type": "event",
        "event": {"sequence": 7, "kind": {"kind": "system", "data": {"event": "gap"}}},
    });
    assert_eq!(session.apply(&gapped), Ok(Update::Event));
    assert!(session.diverged(), "the skipped sequences diverge the fold");
    assert!(session.events().is_empty(), "a gapped event is not folded");
    drop(client);
    let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
    client
        .send_json(&frames::join(&tui_cap, None, None).unwrap())
        .await
        .unwrap();
    let snapshot = next_frame(&mut client).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(session.apply(&snapshot), Ok(Update::Snapshot));
    assert!(
        !session.diverged(),
        "a fresh live snapshot re-establishes state"
    );
    assert!(session.events().is_empty());

    let _ = router;
}
