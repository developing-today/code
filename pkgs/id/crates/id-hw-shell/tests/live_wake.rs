//! Live-host behavioral check for the hardware-shell spike
//! (#10929, design more-shells v1 §3).
//!
//! Review finding 1 of the design record: conformance alone is
//! not a completion gate — behavioral checks for the shell's
//! required workflow are also required. The required workflow
//! here is the whole hardware-shell posture in one pass against a
//! self-hosted world:
//!
//! 1. the device wakes and joins (a second participant, ann, is
//!    already present) — presence folds from the snapshot's
//!    participant list, the only presence a session is given:
//!    the host pushes no live join notifications, which is
//!    exactly why this shell's posture is wake-and-read;
//! 2. the world presents a choice (a form with one choice field);
//! 3. the device confirms it with its one deliberate input, a
//!    `form_result` frame;
//! 4. the host accepts the answer and the program records the
//!    result — the oracle is host-side (the program's recorded
//!    results and the status reply), not the shell's local state;
//!    the journal head is then proven against a durable input
//!    event from ann, which the shell folds live;
//! 5. the device disconnects: a shell that is off is absent. A
//!    third participant, bo, joins while it is off; on the next
//!    wake a fresh join's snapshot re-reads presence — bo is now
//!    in the world — and a final head-only wake resumes with
//!    `after`, replaying just the events since the recorded
//!    head (a resume join is answered with replayed events, no
//!    snapshot, as the host's session tests also show).
//!
//! The world is served in-process over iroh loopback, the same
//! harness shape as id-native-shell's live-session test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use id_hw_shell::HwShell;
use id_worlds::world::{
    Participant, WorldCore, WorldEvent, WorldEventKind, WorldHandle, WorldLimits, WorldProgram,
    WorldScopes,
};
use id_worlds::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id_worlds::world_session::WorldService;
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(30);

/// A world program that presents one choice — "ship the update,
/// now or later?" — through the host's form mechanism and records
/// every answer it receives. Mirrors the form session program of
/// the host's own session tests, with a choice field in place of
/// free text: the hardware subset answers selections, not text.
struct ChoiceProgram {
    token: Arc<Mutex<Option<String>>>,
    results: Arc<Mutex<Vec<Value>>>,
}

fn choice_form(token: Option<&str>) -> Value {
    let mut form = json!({
        "version": 1,
        "form_id": "hw-choice",
        "title": "Ship the update?",
        "fields": [{
            "kind": "choice",
            "id": "when",
            "label": "When",
            "required": true,
            "options": ["now", "later"],
        }],
        "actions": [
            {"id": "submit", "label": "Confirm"},
            {"id": "cancel", "label": "Cancel", "kind": "cancel"},
        ],
    });
    if let Some(token) = token {
        form["presentation_token"] = json!(token);
    }
    form
}

impl WorldProgram for ChoiceProgram {
    fn wants(&mut self) -> anyhow::Result<Option<String>> {
        Ok(Some(
            json!({
                "requests": [{
                    "id": "present-1",
                    "cap": "form.present",
                    "args": {"form_id": "hw-choice", "form": choice_form(None)},
                }],
            })
            .to_string(),
        ))
    }

    fn update(&mut self, event: &WorldEvent) -> anyhow::Result<()> {
        let WorldEventKind::System { event } = &event.kind else {
            return Ok(());
        };
        let value: Value = serde_json::from_str(event)?;
        if value["cap"] == "result"
            && value["id"] == "present-1"
            && value["ok"] == true
            && let Some(token) = value["value"]["presentation_token"].as_str()
        {
            *self.token.lock().unwrap() = Some(token.to_owned());
        } else if value["cap"] == "form.result" {
            self.results.lock().unwrap().push(value);
        }
        Ok(())
    }

    fn view(&mut self, _viewer: &Participant) -> anyhow::Result<Option<String>> {
        let Some(token) = self.token.lock().unwrap().clone() else {
            return Ok(None);
        };
        let mut surface = choice_form(Some(&token));
        surface["kind"] = json!("form");
        Ok(Some(surface.to_string()))
    }
}

/// A loopback endpoint that uses no relay or discovery.
async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

/// Serve `service` over iroh on loopback (mirrors the
/// world_fixture test harness).
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

/// Join a raw (terminal-style) participant and read its snapshot.
async fn join_raw(ep: &Endpoint, addr: EndpointAddr, capability: &str) -> WorldClient {
    let mut client = WorldClient::connect(ep, addr).await.unwrap();
    client
        .send_json(&json!({"type": "join", "capability": capability}))
        .await
        .unwrap();
    let snapshot = client.recv_json().await.unwrap().unwrap();
    assert_eq!(snapshot["type"], "snapshot");
    client
}

/// Fold host frames into the shell until `done` holds.
async fn fold_until(
    client: &mut WorldClient,
    shell: &mut HwShell,
    done: impl Fn(&HwShell) -> bool,
    what: &str,
) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            if done(shell) {
                return;
            }
            let frame = client
                .recv_json()
                .await
                .unwrap()
                .expect("host closed the hardware shell session");
            shell.apply_server_frame(&frame).unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
async fn hw_shell_wakes_sees_presence_and_confirms_a_choice() {
    // A self-hosted world whose program presents one choice.
    // grant_on_use opens the form caps when the program asks, the
    // path the host's own grant-on-use actor test proves.
    let results = Arc::new(Mutex::new(Vec::new()));
    let program = ChoiceProgram {
        token: Arc::new(Mutex::new(None)),
        results: Arc::clone(&results),
    };
    let limits = WorldLimits {
        grant_on_use: true,
        ..WorldLimits::default()
    };
    let core = WorldCore::new("lobby", limits).unwrap();
    let world = WorldHandle::spawn_with_device_manifest(
        core,
        Box::new(program),
        Some(WorldHandle::host_device_manifest()),
    );
    let service = WorldService::new(world.clone(), None);
    let (router, addr) = serve(service).await;
    let client_ep = endpoint().await;

    // ann is already in the world when the device wakes.
    let (_, ann_cap) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let mut ann = join_raw(&client_ep, addr.clone(), ann_cap.expose()).await;

    // The device wakes and joins; it folds until the world
    // presents its choice.
    let (_, hw_cap) = world.issue("hw", WorldScopes::GUEST).await.unwrap();
    let mut shell = HwShell::new();
    let mut hw = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    hw.send_json(&shell.join_frame(hw_cap.expose()).unwrap())
        .await
        .unwrap();
    fold_until(
        &mut hw,
        &mut shell,
        |s| s.choice().is_some(),
        "the presented choice",
    )
    .await;

    // Presence, folded from the snapshot's participant list: ann
    // is present. (Joins and form presentation are not journal
    // events in this world, so the head is still at the snapshot's
    // current_sequence; it first advances on the confirmed answer
    // below.)
    assert!(
        shell.presence().contains("ann"),
        "presence after wake: {:?}",
        shell.presence()
    );
    let head_at_wake = shell.head();
    let choice = shell.choice().expect("choice in force");
    assert_eq!(choice.form_id, "hw-choice");
    assert_eq!(
        choice.actions,
        vec!["submit".to_owned(), "cancel".to_owned()]
    );

    // The device's one deliberate act: confirm the choice, picking
    // the declared option "now".
    let selections = json!({"when": "now"})
        .as_object()
        .expect("selections object")
        .clone();
    let answer = shell.confirm("submit", &selections).unwrap();
    hw.send_json(&answer).await.unwrap();
    fold_until(
        &mut hw,
        &mut shell,
        |s| s.last_status().is_some(),
        "the form_result_status reply",
    )
    .await;
    assert_eq!(shell.last_status(), Some((true, None)));

    // The journal head the device wakes on tracks durable events
    // live: ann's input journals, and the shell's head advances
    // past where it was at wake. (The confirmed answer's own
    // durability is proven host-side by the program's record
    // below; the host delivers form results to the program
    // without broadcasting an event frame to sessions.)
    ann.send_json(&json!({"type": "input", "data_hex": "70696e67"}))
        .await
        .unwrap();
    fold_until(
        &mut hw,
        &mut shell,
        |s| s.head() > head_at_wake,
        "the journal head advancing on ann's durable input",
    )
    .await;

    // The host-side oracle: the program recorded exactly one
    // result, carrying the confirmed selection.
    let recorded = results.lock().unwrap();
    assert_eq!(recorded.len(), 1, "program results: {recorded:?}");
    let recorded_text = recorded[0].to_string();
    assert!(
        recorded_text.contains("\"when\":\"now\""),
        "recorded result carries the selection: {recorded_text}"
    );
    drop(recorded);

    // The device disconnects: a shell that is off is absent. bo
    // joins while it is off — no live frame can reach it, and the
    // host pushes none in any case.
    hw.close();
    let (_, bo_cap) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
    let mut bo = join_raw(&client_ep, addr.clone(), bo_cap.expose()).await;

    // On its next wake the shell takes a fresh join: the snapshot
    // is the only presence read the contract offers, and bo is
    // now in the world.
    let mut hw2 = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    hw2.send_json(&shell.join_frame(hw_cap.expose()).unwrap())
        .await
        .unwrap();
    fold_until(
        &mut hw2,
        &mut shell,
        |s| s.presence().contains("bo"),
        "bo in the wake-up snapshot",
    )
    .await;
    assert!(!shell.diverged());
    hw2.close();

    // A head-only wake resumes with `after`: the host replays
    // just the durable events since the recorded head — no
    // snapshot — and the replay is contiguous, so the shell does
    // not diverge.
    let head_before_resume = shell.head();
    ann.send_json(&json!({"type": "input", "data_hex": "70696e67"}))
        .await
        .unwrap();
    let mut hw3 = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    hw3.send_json(&shell.resume_frame(hw_cap.expose()).unwrap())
        .await
        .unwrap();
    fold_until(
        &mut hw3,
        &mut shell,
        |s| s.head() > head_before_resume,
        "the resumed replay advancing the head",
    )
    .await;
    assert!(!shell.diverged());
    hw3.close();

    ann.close();
    bo.close();

    client_ep.close().await;
    router.shutdown().await.unwrap();
}
