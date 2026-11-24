//! End-to-end oracle for #7914 criterion 1: an agent joins a local
//! world over iroh, reads a screen via semantic diffs, posts chat, and
//! leaves — end to end.
//!
//! Every frame the agent sends is produced by the MCP bridge translator
//! ([`id_caps::mcp_bridge`]) from a tool call, and every frame it
//! receives is fed back into the bridge; the agent never touches a raw
//! frame. A second participant joins raw, exactly as a human shell
//! does, and posts the chat the agent then reads. Oracles: the bridge's
//! own `screen.read` rendering (the diff must surface, once) and the
//! host journal exported through the admin fixture recorder (both
//! chats durable, in order) — not either client's local view alone.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use id::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};
use id::world_fixture::{FixtureEvent, record};
use id::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id::world_session::{WorldService, module_hash};
use id_caps::mcp_bridge::{BridgeSession, ToolAction, v1_tool_definitions};
use iroh::endpoint::{RelayMode, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, TransportAddr};
use iroh_blobs::store::mem::MemStore;
use serde_json::{Value, json};

const TIC_TAC_TOE: &[u8] = include_bytes!("../apps/tic-tac-toe/program/tic-tac-toe.wasm");
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

/// The agent side of the bridge: one world connection driven purely by
/// tool calls. `Local` results are returned to the caller; frames are
/// sent verbatim; `Close` drops the connection.
struct AgentDriver {
    client: Option<WorldClient>,
    bridge: BridgeSession,
}

impl AgentDriver {
    async fn call(&mut self, name: &str, arguments: Value) -> Option<String> {
        match self.bridge.tool_action(name, &arguments).unwrap() {
            ToolAction::SendFrame(frame) => {
                self.client
                    .as_mut()
                    .unwrap()
                    .send_json(&frame)
                    .await
                    .unwrap();
                None
            }
            ToolAction::Local(text) => Some(text),
            ToolAction::Close => {
                if let Some(client) = self.client.take() {
                    client.close();
                }
                None
            }
        }
    }

    /// Read host frames into the bridge until `ready` holds.
    async fn pump_until(&mut self, what: &str, ready: impl Fn(&BridgeSession) -> bool) {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                if ready(&self.bridge) {
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
                self.bridge.observe_frame(&frame);
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
async fn agent_joins_reads_via_semantic_diffs_posts_chat_and_leaves() {
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
    let mut agent = AgentDriver {
        client: Some(client),
        bridge: BridgeSession::start(&v1_tool_definitions()).unwrap(),
    };

    // world.join: the bridge emits the agent-profile join frame; the
    // host answers with a snapshot and the first view, which the
    // bridge renders for screen.read.
    agent
        .call("world.join", json!({ "capability": agent_cap.expose() }))
        .await;
    agent
        .pump_until("the join view", |bridge| bridge.screen_text().is_some())
        .await;
    let first_read = agent.call("screen.read", json!({})).await.unwrap();
    assert!(
        !first_read.contains("changes since last read"),
        "no diffs before any world change: {first_read}"
    );

    // screen.subscribe: the host acknowledges the chat_posted
    // subscription.
    agent
        .call(
            "screen.subscribe",
            json!({ "event_classes": ["chat_posted"] }),
        )
        .await;
    agent
        .pump_until("the subscription ack", |bridge| bridge.is_subscribed())
        .await;

    // A human participant joins raw, exactly as a terminal shell does,
    // and posts chat (durable sequence 2).
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

    // The agent reads the screen via the semantic diff the host pushes
    // for its subscription: the change in world terms, not a repaint.
    agent
        .pump_until("the chat_posted diff", |bridge| {
            bridge
                .pending_changes()
                .iter()
                .any(|change| change["kind"] == "chat_posted" && change["text"] == "hello agent")
        })
        .await;
    let read = agent.call("screen.read", json!({})).await.unwrap();
    assert!(
        read.contains("chat_posted: hello agent"),
        "screen.read must surface the diff: {read}"
    );
    let reread = agent.call("screen.read", json!({})).await.unwrap();
    assert!(
        !reread.contains("hello agent"),
        "a second read must not replay the drained diff: {reread}"
    );

    // chat.post: the bridge emits the chat frame; bo's raw client sees
    // the durable event at sequence 3.
    agent
        .call("chat.post", json!({ "text": "hello human" }))
        .await;
    wait_for_event(&mut bo, 3).await;

    // world.leave: no frame exists to send — the bridge closes the
    // session, and the translator refuses further session tools.
    match agent.bridge.tool_action("world.leave", &json!({})).unwrap() {
        ToolAction::Close => {
            if let Some(client) = agent.client.take() {
                client.close();
            }
        }
        other => panic!("world.leave must close the session, got {other:?}"),
    }
    assert!(
        agent
            .bridge
            .tool_action("chat.post", &json!({ "text": "gone" }))
            .is_err(),
        "no tool may translate after leaving"
    );
    bo.close();

    // The durable oracle: the host journal holds both chats, in order.
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
    assert_eq!(fixture.header.current_sequence, 3);

    client_ep.close().await;
    router.shutdown().await.unwrap();
}
