//! End-to-end oracle for #7917 (spike step 5/5 for #6300): the
//! scripted spike run. One agent joins a local world over iroh, reads
//! screens, posts chat, invokes the one named program once, and
//! leaves — every action driven through the host-side policy point
//! ([`id_caps::mcp_policy`]) wrapping the MCP bridge translator, so
//! the run produces the design's audit log as it goes.
//!
//! Oracles, none of them the agent's local view alone:
//! - the policy point's audit log: one event per action in script
//!   order (join, reads, chat, the refused invoke, the admitted
//!   invoke, leave), the hash chain verifying end to end;
//! - the host journal exported through the admin fixture recorder:
//!   both chats and the invocation's input durable, in order;
//! - the bridge's own `screen.read` rendering: the invocation's
//!   result comes back as a semantic diff and the updated screen.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use id::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
use id::world_fixture::{FixtureEvent, record};
use id::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id::world_session::{WorldService, module_hash};
use id_caps::mcp_bridge::{BridgeSession, ToolAction, v1_tool_definitions};
use id_caps::mcp_policy::{
    AuditOutcome, CapabilityCeiling, PolicyOutcome, PolicySession, Quota, RefusalKind, RoleGrant,
};
use id_caps::role_config::QuotaClass;
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use iroh_blobs::store::mem::MemStore;
use serde_json::{Value, json};

const TIC_TAC_TOE: &[u8] = include_bytes!("../apps/tic-tac-toe/program/tic-tac-toe.wasm");
const TIMEOUT: Duration = Duration::from_secs(30);
/// The time reading handed to the policy point. The spike's role
/// caps no tool, so the reading never refuses a call; the reconciled
/// policy point takes it for its per-tool rate-limit windows.
const SPIKE_NOW: u64 = 1_700_000_000;

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

/// The spike's role grant: every surface tool, the tic-tac-toe world,
/// and exactly one program invocation — the §4.6 bound.
fn spike_grant() -> RoleGrant {
    RoleGrant {
        agent_key: [9; 32],
        agent_label: "spike".to_owned(),
        owner: "drew".to_owned(),
        world: "tic-tac-toe".to_owned(),
        capability_ref: "grant-spike-1".to_owned(),
        ceiling: CapabilityCeiling {
            tools: v1_tool_definitions()
                .iter()
                .map(|definition| definition.name.clone())
                .collect(),
            worlds: BTreeSet::from(["tic-tac-toe".to_owned()]),
            object_classes: BTreeSet::from(["cell".to_owned()]),
            program_allowlist: BTreeSet::from(["tic-tac-toe".to_owned()]),
        },
        role: "spike".to_owned(),
        quota_class: QuotaClass {
            budget: Quota {
                frames: 8,
                tool_calls: 32,
                program_invocations: 1,
            },
            replenishment: None,
        },
        rate_limits: Vec::new(),
    }
}

/// The agent side of the spike: one world connection driven purely by
/// policy-checked tool calls. Frames the policy point admits are sent
/// verbatim; refusals send nothing; `Close` drops the connection.
struct SpikeDriver {
    client: Option<WorldClient>,
    session: PolicySession<'static>,
}

impl SpikeDriver {
    async fn call(&mut self, name: &str, arguments: Value) -> PolicyOutcome {
        let outcome = self.session.act(name, &arguments, SPIKE_NOW);
        match &outcome {
            PolicyOutcome::Executed(ToolAction::SendFrame(frame)) => {
                let frame = frame.clone();
                self.client
                    .as_mut()
                    .unwrap()
                    .send_json(&frame)
                    .await
                    .unwrap();
            }
            PolicyOutcome::Executed(ToolAction::Close) => {
                if let Some(client) = self.client.take() {
                    client.close();
                }
            }
            _ => {}
        }
        outcome
    }

    /// A call that must be answered locally; returns the result text.
    async fn read(&mut self) -> String {
        match self.call("screen.read", json!({})).await {
            PolicyOutcome::Executed(ToolAction::Local(text)) => text,
            other => panic!("screen.read must answer locally, got {other:?}"),
        }
    }

    /// Read host frames into the session until `ready` holds.
    async fn pump_until(&mut self, what: &str, ready: impl Fn(&PolicySession<'_>) -> bool) {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if ready(&self.session) {
                    return;
                }
                let frame = self
                    .client
                    .as_mut()
                    .unwrap()
                    .recv_json()
                    .await
                    .unwrap()
                    .expect("host closed the agent session");
                self.session.observe_frame(&frame);
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    /// Pump until `ready` holds and the host stream then stays
    /// quiet for a beat. One commit's frames are generated together
    /// but their order on the wire is not contractual — a
    /// re-rendered view can land behind the semantic diff — so a
    /// caller taking state from the stream (the screen revision an
    /// invoke is decided against) drains to quiescence first;
    /// otherwise the bridge's revision can lag the host's current
    /// one by exactly the trailing view, and the host's
    /// stale-decision gate refuses the invoke.
    async fn pump_drained(&mut self, what: &str, ready: impl Fn(&PolicySession<'_>) -> bool) {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                let frame = match tokio::time::timeout(
                    Duration::from_millis(500),
                    self.client.as_mut().unwrap().recv_json(),
                )
                .await
                {
                    Ok(Ok(Some(frame))) => frame,
                    Ok(other) => panic!("host closed the agent session: {other:?}"),
                    Err(_) => {
                        if ready(&self.session) {
                            return;
                        }
                        continue;
                    }
                };
                self.session.observe_frame(&frame);
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }
}

/// Wait until the raw client receives the durable `event` frame for
/// `sequence`, skipping snapshots, views, and earlier events.
async fn wait_for_event(client: &mut WorldClient, sequence: u64) {
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let frame = client
                .recv_json()
                .await
                .unwrap()
                .expect("host closed the raw session");
            if frame["type"] == "event" && frame["event"]["sequence"] == sequence {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for event at sequence {sequence}"));
}

#[tokio::test]
async fn spike_run_joins_reads_chats_invokes_once_and_leaves() {
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
    let (_, agent_cap) = world.issue("agent", WorldScopes::GUEST).await.unwrap();
    let (_, bo_cap) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
    let (router, addr) = serve(service).await;
    let client_ep = endpoint().await;

    let client = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    let mut agent = SpikeDriver {
        client: Some(client),
        session: PolicySession::new(
            spike_grant(),
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            SigningKey::from_bytes(&[5; 32]),
            SPIKE_NOW,
        ),
    };

    // Join: admitted through the policy point; the host answers with
    // a snapshot and the first view.
    let outcome = agent
        .call("world.join", json!({ "capability": agent_cap.expose() }))
        .await;
    assert!(
        matches!(outcome, PolicyOutcome::Executed(ToolAction::SendFrame(_))),
        "join must be admitted: {outcome:?}"
    );
    agent
        .pump_until("the join view", |session| {
            session.bridge().screen_text().is_some()
        })
        .await;
    let first_read = agent.read().await;
    assert!(
        !first_read.contains("changes since last read"),
        "no diffs before any world change: {first_read}"
    );

    // Subscribe to the classes the script reads back: chat and the
    // invocation's result.
    let outcome = agent
        .call(
            "screen.subscribe",
            json!({ "event_classes": ["chat_posted", "input_applied"] }),
        )
        .await;
    assert!(
        matches!(outcome, PolicyOutcome::Executed(ToolAction::SendFrame(_))),
        "subscribe must be admitted: {outcome:?}"
    );
    agent
        .pump_until("the subscription ack", |session| {
            session.bridge().is_subscribed()
        })
        .await;

    // A human participant joins raw and posts chat (durable
    // sequence 2); the agent reads it via the semantic diff.
    let mut bo = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    bo.send_json(&json!({ "type": "join", "capability": bo_cap.expose(), "after": null }))
        .await
        .unwrap();
    let snapshot = bo.recv_json().await.unwrap().unwrap();
    assert_eq!(snapshot["type"], "snapshot");
    bo.send_json(&json!({ "type": "chat", "text": "hello agent" }))
        .await
        .unwrap();
    agent
        .pump_until("the chat_posted diff", |session| {
            session
                .bridge()
                .pending_changes()
                .iter()
                .any(|change| change["kind"] == "chat_posted" && change["text"] == "hello agent")
        })
        .await;
    let read = agent.read().await;
    assert!(
        read.contains("chat_posted: hello agent"),
        "screen.read must surface the diff: {read}"
    );

    // The agent posts back (durable sequence 3).
    let outcome = agent
        .call("chat.post", json!({ "text": "hello human" }))
        .await;
    assert!(
        matches!(outcome, PolicyOutcome::Executed(ToolAction::SendFrame(_))),
        "chat must be admitted: {outcome:?}"
    );
    wait_for_event(&mut bo, 3).await;

    // Drain the agent's own frames for its chat to quiescence (the
    // host diffs a session's own actions back to it, and a trailing
    // re-rendered view can land behind the diff): the bridge's
    // observed revision must be the host's current one before the
    // invoke's stale-decision token is taken from it.
    agent
        .pump_drained("the agent's own chat diff", |session| {
            session
                .bridge()
                .pending_changes()
                .iter()
                .any(|change| change["kind"] == "chat_posted" && change["text"] == "hello human")
        })
        .await;

    // A program outside the allowlist is refused at the ceiling:
    // audited, charged nothing, and no frame reaches the host.
    let outcome = agent
        .call(
            "program.invoke",
            json!({ "program": "wipe", "object": "board", "arguments": "0" }),
        )
        .await;
    match &outcome {
        PolicyOutcome::Refused(refusal) => assert_eq!(refusal.kind, RefusalKind::Ceiling),
        other => panic!("an unlisted program must be refused, got {other:?}"),
    }
    assert_eq!(agent.session.spend(SPIKE_NOW).program_invocations, 0);

    // The one bounded invocation: the named program on the named
    // object, decided against the current screen revision. Exactly
    // one input frame carries it; the move is durable at sequence 4.
    let revision = agent.session.bridge().view_revision();
    let outcome = agent
        .call(
            "program.invoke",
            json!({
                "program": "tic-tac-toe",
                "object": "board",
                "arguments": "4",
                "screen_revision": revision,
            }),
        )
        .await;
    assert_eq!(
        outcome,
        PolicyOutcome::Executed(ToolAction::SendFrame(json!({
            "type": "input",
            "data_hex": "34",
            "screen_revision": revision,
        })))
    );
    // (No raw-client event wait for the input: the durable proof is
    // the journal oracle below, and the agent-side rendering is the
    // pump that follows.)

    // The result renders back as a semantic diff over the updated
    // screen: cell 4 carries the X.
    agent
        .pump_until("the invocation result", |session| {
            session
                .bridge()
                .screen_text()
                .is_some_and(|text| text.contains("----X----"))
                && session
                    .bridge()
                    .pending_changes()
                    .iter()
                    .any(|change| change["kind"] == "input_applied")
        })
        .await;
    let read = agent.read().await;
    assert!(
        read.contains("----X----"),
        "the read must show the moved board: {read}"
    );
    assert!(
        read.contains("input_applied"),
        "the read must render the invocation's diff: {read}"
    );

    // Leave: never refused, charged nothing, still audited.
    let outcome = agent.call("world.leave", json!({})).await;
    assert_eq!(outcome, PolicyOutcome::Executed(ToolAction::Close));

    // The audit log is the run, complete and in order: one event per
    // action, the refusal among the executions, the chain verifying
    // end to end under the host key.
    let session = &agent.session;
    let tools: Vec<&str> = session
        .audit_events()
        .iter()
        .map(|event| event.tool.as_str())
        .collect();
    assert_eq!(
        tools,
        [
            "world.join",
            "screen.read",
            "screen.subscribe",
            "screen.read",
            "chat.post",
            "program.invoke",
            "program.invoke",
            "screen.read",
            "world.leave",
        ]
    );
    for (index, event) in session.audit_events().iter().enumerate() {
        if index == 5 {
            assert!(
                matches!(event.outcome, AuditOutcome::RefusedCeiling { .. }),
                "the unlisted invoke is the run's refusal: {event:?}"
            );
            assert_eq!(event.quota_charged, Quota::zero());
        } else {
            assert_eq!(event.outcome, AuditOutcome::Executed, "event {index}");
        }
    }
    let invoke_event = &session.audit_events()[6];
    assert_eq!(invoke_event.object.as_deref(), Some("tic-tac-toe"));
    assert_eq!(invoke_event.quota_charged.program_invocations, 1);
    session.verify_audit().unwrap();
    let summary = session.owner_summary(SPIKE_NOW);
    assert_eq!(summary.actions, 9);
    assert_eq!(summary.executed, 8);
    assert_eq!(summary.refusals, 1);
    assert_eq!(
        summary.spend,
        Quota {
            frames: 4,
            tool_calls: 7,
            program_invocations: 1,
        }
    );
    bo.close();

    // The durable oracle: the host journal holds both chats and the
    // invocation's input, in order.
    let mut admin = WorldClient::connect(&client_ep, addr.clone())
        .await
        .unwrap();
    let fixture = record(&mut admin, "admin").await.unwrap();
    admin.close();
    let chats: Vec<(u64, String)> = fixture
        .events
        .iter()
        .filter_map(|event| match event {
            FixtureEvent::Chat { sequence, text, .. } => Some((*sequence, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        chats,
        vec![(2, "hello agent".to_owned()), (3, "hello human".to_owned())]
    );
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
    assert_eq!(inputs, vec![(4, b"4".to_vec())]);
    assert_eq!(fixture.header.current_sequence, 4);

    client_ep.close().await;
    router.shutdown().await.unwrap();
}
