//! Sessions checked against the shell contract (F13.1): the host's own
//! frames conform, and the contract's journaled-vs-ephemeral classes hold.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::shell_contract::{Conformance, FrameClass, Phase, ShellContract, sample_client_frame};
use crate::world::{Participant, WorldCore, WorldHandle, WorldLimits, WorldScopes};
use crate::world_hub::WorldHub;
use crate::world_session::{Inbound, SessionConfig, SessionIo, WorldService, run_session_with};

struct ChannelIo {
    inbound: mpsc::Receiver<Inbound>,
    outbound: mpsc::Sender<String>,
}

impl SessionIo for ChannelIo {
    async fn recv(&mut self) -> Inbound {
        self.inbound.recv().await.unwrap_or(Inbound::Closed)
    }
    async fn send(&mut self, frame: String) -> bool {
        self.outbound.send(frame).await.is_ok()
    }
}

/// One session whose every frame, both ways, goes through the contract check.
struct CheckedPeer {
    to_server: mpsc::Sender<Inbound>,
    from_server: mpsc::Receiver<String>,
    check: Conformance<'static>,
    seen: Vec<String>,
}

impl CheckedPeer {
    fn connect(service: WorldService) -> Self {
        let hub = WorldHub::single(service);
        let (to_server, inbound) = mpsc::channel(16);
        let (outbound, from_server) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut io = ChannelIo { inbound, outbound };
            run_session_with(&hub, &mut io, SessionConfig::default()).await;
        });
        Self {
            to_server,
            from_server,
            check: Conformance::new(ShellContract::embedded().unwrap()),
            seen: Vec::new(),
        }
    }

    async fn say(&mut self, frame: Value) {
        self.check.client(&frame);
        self.to_server
            .send(Inbound::Text(frame.to_string()))
            .await
            .unwrap();
    }

    /// Every frame the host sends until it goes quiet.
    async fn drain(&mut self) -> Vec<Value> {
        let mut frames = Vec::new();
        while let Ok(Some(text)) =
            tokio::time::timeout(Duration::from_millis(300), self.from_server.recv()).await
        {
            let frame: Value = serde_json::from_str(&text).unwrap();
            self.check.server(&frame);
            self.seen
                .push(frame["type"].as_str().unwrap_or_default().to_owned());
            frames.push(frame);
        }
        frames
    }

    fn assert_conforms(&self) {
        assert!(
            self.check.violations().is_empty(),
            "{:#?}",
            self.check.violations()
        );
    }
}

/// A program with a view, so sessions receive `view` frames.
struct Viewed;

impl crate::world::WorldProgram for Viewed {
    fn view(&mut self, viewer: &Participant) -> anyhow::Result<Option<String>> {
        Ok(Some(format!("hello {}", viewer.display_name)))
    }
}

fn service() -> WorldService {
    WorldService::new(
        WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap()),
        Some("admin".to_owned()),
    )
}

async fn sequence(world: &WorldHandle, token: &crate::world::JoinCapability) -> u64 {
    world
        .snapshot(token.clone())
        .await
        .unwrap()
        .current_sequence
}

#[tokio::test]
async fn a_full_joined_session_conforms_and_never_shows_the_seed() {
    let svc = service();
    let world = svc.world().clone();
    world
        .install_program("ab".repeat(32), 424_242, Box::new(Viewed))
        .await
        .unwrap();
    let delegator = WorldScopes::from_role("delegator").unwrap();
    let (_, guest) = world.issue("ada", delegator).await.unwrap();
    let cap = guest.expose().to_owned();

    let mut peer = CheckedPeer::connect(svc.clone());
    peer.say(json!({"type": "join", "capability": cap, "after": null}))
        .await;
    peer.drain().await;
    assert_eq!(peer.check.phase(), Phase::Joined);
    for frame in [
        json!({"type": "resize", "width": 80, "height": 24}),
        json!({"type": "input", "data_hex": "6869"}),
        json!({"type": "paste", "text": "pasted"}),
        json!({"type": "chat", "text": "hello"}),
        json!({"type": "info", "capability": cap}),
        json!({"type": "records", "capability": cap, "prefix": null, "after": null}),
        json!({"type": "caps", "capability": cap}),
        json!({"type": "attenuate", "name": "spectator", "scopes": ["join"]}),
    ] {
        peer.say(frame).await;
        peer.drain().await;
    }
    // A live reinstall reaches the session as a snapshot plus program_changed.
    world
        .install_program("cd".repeat(32), 777, Box::new(Viewed))
        .await
        .unwrap();
    let frames = peer.drain().await;
    assert!(
        !serde_json::to_string(&frames).unwrap().contains("777"),
        "{frames:?}"
    );
    peer.assert_conforms();
    for kind in [
        "snapshot",
        "view",
        "event",
        "module_info",
        "records",
        "caps",
        "invite",
        "program_changed",
    ] {
        assert!(
            peer.seen.iter().any(|seen| seen == kind),
            "no {kind}: {:?}",
            peer.seen
        );
    }
    assert!(
        !peer.seen.iter().any(|seen| seen == "error"),
        "{:?}",
        peer.seen
    );
}

#[tokio::test]
async fn query_and_upload_sessions_conform() {
    let svc = service();
    let world = svc.world().clone();
    let (_, guest) = world.issue("ada", WorldScopes::GUEST).await.unwrap();
    let cap = guest.expose().to_owned();

    // A query session that pages records and then joins.
    let mut peer = CheckedPeer::connect(svc.clone());
    peer.say(json!({"type": "info", "capability": cap})).await;
    peer.drain().await;
    assert_eq!(peer.check.phase(), Phase::Query);
    peer.say(json!({"type": "records", "capability": cap}))
        .await;
    peer.drain().await;
    peer.say(json!({"type": "join", "capability": cap})).await;
    peer.drain().await;
    assert_eq!(peer.check.phase(), Phase::Joined);
    peer.assert_conforms();

    // One-reply opening frames end the session.
    for opening in [
        json!({"type": "invite", "admin_token": "admin", "display_name": "bo", "role": "player"}),
        json!({"type": "list_worlds", "admin_token": "admin"}),
        json!({"type": "caps", "capability": cap}),
        json!({"type": "set_caps_ceiling", "admin_token": "admin", "ceiling": ["time.now"]}),
        json!({"type": "execution_profile_info", "capability": cap}),
        json!({"type": "invite", "admin_token": "wrong", "display_name": "eve"}),
    ] {
        let mut peer = CheckedPeer::connect(svc.clone());
        peer.say(opening.clone()).await;
        let frames = peer.drain().await;
        assert_eq!(frames.len(), 1, "{opening}: {frames:?}");
        assert_eq!(peer.check.phase(), Phase::Closed, "{opening}");
        peer.assert_conforms();
    }
}

/// The contract's classes are executable: every frame a joined shell may send
/// leaves the journal alone unless the contract calls it an event, and every
/// event frame commits exactly one.
#[tokio::test]
async fn the_journaled_vs_ephemeral_boundary_is_what_the_contract_says() {
    let contract = ShellContract::embedded().unwrap();
    let svc = service();
    let world = svc.world().clone();
    world
        .install_program("ab".repeat(32), 1, Box::new(Viewed))
        .await
        .unwrap();
    let (_, guest) = world.issue("ada", WorldScopes::GUEST).await.unwrap();
    let cap = guest.expose().to_owned();
    let mut peer = CheckedPeer::connect(svc.clone());
    peer.say(json!({"type": "join", "capability": cap})).await;
    peer.drain().await;

    let mut checked = 0;
    for (kind, spec) in &contract.client_frames {
        if !spec.phases.contains(&Phase::Joined) {
            continue;
        }
        let before = sequence(&world, &guest).await;
        peer.say(sample_client_frame(contract, kind, &cap)).await;
        peer.drain().await;
        let after = sequence(&world, &guest).await;
        match spec.class {
            FrameClass::Event => assert_eq!(after, before + 1, "{kind} must commit one event"),
            FrameClass::Ephemeral | FrameClass::Query | FrameClass::Control => {
                assert_eq!(after, before, "{kind} must not touch the journal");
            }
            // Authority-bearing frames may journal configuration; the sample's
            // admin token is wrong, so here they are refused.
            FrameClass::Admin | FrameClass::Delegation => assert_eq!(after, before, "{kind}"),
            FrameClass::Durable | FrameClass::Reply => unreachable!("{kind}"),
        }
        checked += 1;
    }
    assert!(checked >= 10, "{checked}");
    peer.assert_conforms();
}
