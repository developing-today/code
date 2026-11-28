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
            // form_result is an event only when a pending form accepts the
            // submission; the generic sample names no pending form, so it is
            // refused with a form_result_status frame and commits nothing.
            // Accepted submissions commit exactly one event — covered by
            // joined_form_result_frame_uses_session_auth_and_reaches_program.
            FrameClass::Event if kind == "form_result" => {
                assert_eq!(
                    after, before,
                    "{kind} sample without a pending form must not commit"
                )
            }
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

/// Agent profile (contract revision 11, #6300 sections 4.2/4.3): a
/// session that joins with the `agent_profile` capability flag receives
/// semantic diffs scoped to its subscription, region subscriptions scope
/// out event diffs, and an input carrying a stale-decision token is
/// rejected without committing — while the same input carrying the
/// current token commits exactly one event.
#[tokio::test]
async fn agent_profile_session_gets_scoped_diffs_and_stale_inputs_are_rejected() {
    let svc = service();
    let world = svc.world().clone();
    world
        .install_program("ab".repeat(32), 424_242, Box::new(Viewed))
        .await
        .unwrap();
    let (_, guest) = world.issue("ada", WorldScopes::GUEST).await.unwrap();
    let cap = guest.expose().to_owned();

    fn newest_view_revision(frames: &[Value], fallback: u64) -> u64 {
        frames
            .iter()
            .filter(|frame| frame["type"] == "view")
            .last()
            .and_then(|frame| frame["view_revision"].as_u64())
            .unwrap_or(fallback)
    }

    let mut peer = CheckedPeer::connect(svc.clone());
    peer.say(json!({"type": "join", "capability": cap, "after": null, "agent_profile": true}))
        .await;
    let frames = peer.drain().await;
    let mut current_revision = newest_view_revision(&frames, u64::MAX);
    assert_ne!(
        current_revision,
        u64::MAX,
        "join must deliver a view: {frames:?}"
    );

    // Subscribe to chat events only: the acknowledgement names the
    // subscription and the session's current screen revision.
    peer.say(json!({"type": "screen_subscribe", "event_classes": ["chat_posted"]}))
        .await;
    let frames = peer.drain().await;
    let subscription = frames
        .iter()
        .find(|frame| frame["type"] == "screen_subscription")
        .unwrap_or_else(|| panic!("subscription ack: {frames:?}"));
    assert_eq!(subscription["event_classes"], json!(["chat_posted"]));
    assert_eq!(
        subscription["view_revision"].as_u64().unwrap(),
        current_revision
    );

    // A chat commits, arrives as an event, and — chat_posted being
    // subscribed — also as a semantic diff describing it in world terms.
    peer.say(json!({"type": "chat", "text": "hello agents"}))
        .await;
    let frames = peer.drain().await;
    let diff = frames
        .iter()
        .find(|frame| frame["type"] == "semantic_diff")
        .unwrap_or_else(|| panic!("chat semantic diff: {frames:?}"));
    assert_eq!(diff["changes"][0]["kind"], "chat_posted");
    assert_eq!(diff["changes"][0]["text"], "hello agents");
    current_revision = newest_view_revision(&frames, current_revision);

    // An input event is not a subscribed class: no semantic diff.
    peer.say(json!({"type": "input", "data_hex": "6c"})).await;
    let frames = peer.drain().await;
    assert!(
        !frames.iter().any(|frame| frame["type"] == "semantic_diff"),
        "unsubscribed class leaked a diff: {frames:?}"
    );
    current_revision = newest_view_revision(&frames, current_revision);

    // A region-only subscription scopes event diffs out entirely.
    peer.say(
        json!({"type": "screen_subscribe", "region": {"x": 0, "y": 0, "width": 5, "height": 1}}),
    )
    .await;
    peer.drain().await;
    peer.say(json!({"type": "chat", "text": "again"})).await;
    let frames = peer.drain().await;
    assert!(
        !frames.iter().any(|frame| frame["type"] == "semantic_diff"),
        "region-only subscription leaked an event diff: {frames:?}"
    );
    current_revision = newest_view_revision(&frames, current_revision);

    // Stale-decision rejection: nothing commits, the answer is a
    // `stale_decision` error, and the current view follows it.
    let before = sequence(&world, &guest).await;
    peer.say(json!({"type": "input", "data_hex": "6c", "screen_revision": current_revision + 100}))
        .await;
    let frames = peer.drain().await;
    let error = frames
        .iter()
        .find(|frame| frame["type"] == "error")
        .unwrap_or_else(|| panic!("stale rejection: {frames:?}"));
    assert_eq!(error["code"], "stale_decision");
    assert!(
        frames.iter().any(|frame| frame["type"] == "view"),
        "the current view follows a stale rejection: {frames:?}"
    );
    assert_eq!(
        sequence(&world, &guest).await,
        before,
        "a stale input must not commit"
    );
    current_revision = newest_view_revision(&frames, current_revision);

    // The same input decided against the current revision is applied.
    peer.say(json!({"type": "input", "data_hex": "6c", "screen_revision": current_revision}))
        .await;
    peer.drain().await;
    assert_eq!(
        sequence(&world, &guest).await,
        before + 1,
        "a current input commits exactly one event"
    );
    peer.assert_conforms();
}

/// Reduced fidelity (design section 4.2): a session that never sets the
/// agent flag gets no agent-profile frames, and a `screen_subscribe`
/// from such a session is refused — the flag changes presentation only,
/// never authority.
#[tokio::test]
async fn a_human_session_gets_no_agent_profile_frames() {
    let svc = service();
    let world = svc.world().clone();
    world
        .install_program("ab".repeat(32), 424_242, Box::new(Viewed))
        .await
        .unwrap();
    let (_, guest) = world.issue("ada", WorldScopes::GUEST).await.unwrap();
    let cap = guest.expose().to_owned();

    let mut peer = CheckedPeer::connect(svc.clone());
    peer.say(json!({"type": "join", "capability": cap, "after": null}))
        .await;
    peer.drain().await;
    peer.say(json!({"type": "screen_subscribe", "event_classes": ["chat_posted"]}))
        .await;
    let frames = peer.drain().await;
    assert!(
        frames.iter().any(|frame| frame["type"] == "error"),
        "subscribe without the agent flag is refused: {frames:?}"
    );
    peer.say(json!({"type": "chat", "text": "hello humans"}))
        .await;
    let frames = peer.drain().await;
    assert!(
        !frames
            .iter()
            .any(|frame| frame["type"] == "semantic_diff" || frame["type"] == "screen_subscription"),
        "no agent-profile frames for a human session: {frames:?}"
    );
    peer.assert_conforms();
}
