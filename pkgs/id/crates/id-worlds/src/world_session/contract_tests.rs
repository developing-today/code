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

/// F13.7 step 2: the provider offer/request/reply frames conform to the
/// shell contract and fail closed without the step-3 broker. An offer is
/// session-local (no journal write, no reply frame); any reply names an
/// unknown nonce and is refused, never routed. No frame carries a
/// session/participant claim (invariant 5): the wire shapes have no such
/// field to trust.
#[tokio::test]
async fn provider_frames_conform_and_fail_closed_without_a_broker() {
    let contract = ShellContract::embedded().unwrap();
    assert_eq!(contract.revision, 9);
    for kind in ["provider_offer", "provider_reply"] {
        assert!(
            contract.client_frames.contains_key(kind),
            "contract must define client frame {kind}"
        );
    }
    assert!(contract.server_frames.contains_key("provider_request"));

    // Offer names belong to the step-1 reserved namespace: the host
    // refuses anything outside `client.`, and the registry contract
    // (`ProviderManifest::side_contract_violation`) owns that rule.
    assert_eq!(crate::world_caps::CLIENT_PROVIDER_NAMESPACE, "client.");

    // A host request built from the new server frame type conforms, so the
    // step-3 broker can send it once offer-gating lands. The reply matches
    // on this connection's session, so the frame carries no session field.
    let request = serde_json::json!({
        "type": "provider_request",
        "nonce": "n-1",
        "provider": "client.file.read",
        "args": {"path": "notes.txt"},
        "max_bytes": 32768,
        "deadline": 1799999999999u64,
    });
    assert_eq!(
        contract.check_server_frame(&request).unwrap(),
        "provider_request"
    );
    assert!(request.get("session").is_none());
    assert!(request.get("participant_id").is_none());

    // Client shapes conform in the joined phase and are refused earlier.
    let offer = serde_json::json!({"type": "provider_offer", "providers": ["client.file.read"]});
    assert_eq!(
        contract.check_client_frame(&offer, Phase::Joined).unwrap(),
        "provider_offer"
    );
    assert!(contract.check_client_frame(&offer, Phase::Query).is_err());
    let reply = serde_json::json!({
        "type": "provider_reply",
        "nonce": "n-1",
        "ok": false,
        "error": "denied",
    });
    assert_eq!(
        contract.check_client_frame(&reply, Phase::Joined).unwrap(),
        "provider_reply"
    );

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
    assert_eq!(peer.check.phase(), Phase::Joined);

    // A namespaced offer is accepted silently: no reply frame, no journal
    // write. The session stays alive for the frames that follow.
    let before = sequence(&world, &guest).await;
    peer.say(offer).await;
    let frames = peer.drain().await;
    assert!(frames.is_empty(), "{frames:?}");
    assert_eq!(sequence(&world, &guest).await, before);

    // An offer outside the reserved namespace is refused, not recorded.
    peer.say(json!({"type": "provider_offer", "providers": ["time.now"]}))
        .await;
    let frames = peer.drain().await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["type"], "error");
    assert_eq!(sequence(&world, &guest).await, before);

    // Every nonce is unknown until the step-3 broker tracks them: a
    // well-formed reply is refused and never routed anywhere.
    peer.say(reply).await;
    let frames = peer.drain().await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["type"], "error");
    assert_eq!(frames[0]["message"], "unknown provider reply");
    assert_eq!(sequence(&world, &guest).await, before);

    // A malformed reply is refused as malformed, not as unknown.
    peer.say(json!({"type": "provider_reply", "nonce": "", "ok": true}))
        .await;
    let frames = peer.drain().await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert_eq!(frames[0]["message"], "invalid provider reply");
    assert_eq!(sequence(&world, &guest).await, before);

    peer.assert_conforms();
}
