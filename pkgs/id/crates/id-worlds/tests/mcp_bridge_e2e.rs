//! End-to-end oracle for #7914 criterion 1: an agent joins a local
//! world over iroh, reads a screen via semantic diffs, posts chat, and
//! leaves — through the [`McpBridge`] wiring, not raw frames.
//!
//! Two worlds, because the host's screen behaviour differs between
//! them: a world with a rendering program sends a `view` on join, so
//! the bridge learns a real screen revision and carries it as the
//! stale-decision token on `chat.post`; a chat-only world sends no
//! view, so the screen is read purely from the semantic diffs the
//! subscription delivers. Both paths are the acceptance criterion.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id_worlds::mcp_bridge::{BridgeError, McpBridge};
use id_worlds::world::{Participant, WorldCore, WorldHandle, WorldLimits, WorldProgram};
use id_worlds::world_net::{WORLD_ALPN, WorldClient, WorldProtocol};
use id_worlds::world_session::WorldService;
use iroh::endpoint::{Endpoint, RelayMode, presets};
use iroh::protocol::Router;
use iroh::{EndpointAddr, TransportAddr};

/// A minimal rendering program: the screen is one greeting line, so
/// the test can tell a real `view` from an empty screen.
struct HelloProgram;

impl WorldProgram for HelloProgram {
    fn view(&mut self, viewer: &Participant) -> anyhow::Result<Option<String>> {
        Ok(Some(format!("hello {}", viewer.display_name)))
    }
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

fn loopback_addr(endpoint: &Endpoint) -> EndpointAddr {
    EndpointAddr::from_parts(
        endpoint.id(),
        endpoint.bound_sockets().iter().map(|addr| {
            let ip = if addr.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                addr.ip()
            };
            TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
        }),
    )
}

struct Host {
    router: Router,
    addr: EndpointAddr,
    host_endpoint: Endpoint,
}

impl Host {
    async fn serve(world: WorldHandle) -> Self {
        let host_endpoint = endpoint().await;
        let service = WorldService::new(world, Some("admin".to_owned()));
        let router = Router::builder(host_endpoint.clone())
            .accept(WORLD_ALPN, WorldProtocol::new(service))
            .spawn();
        let addr = loopback_addr(&host_endpoint);
        Self {
            router,
            addr,
            host_endpoint,
        }
    }

    async fn shutdown(self) {
        self.router.shutdown().await.unwrap();
        self.host_endpoint.close().await;
    }
}

async fn spawn_world(program: bool) -> (Host, WorldHandle) {
    let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
    if program {
        world
            .install_program("ab".repeat(32), 424_242, Box::new(HelloProgram))
            .await
            .unwrap();
    }
    let host = Host::serve(world.clone()).await;
    (host, world)
}

/// Mint a guest capability through the admin invite path, on its own
/// throwaway connection — the bridge itself never mints capabilities.
async fn invite_capability(host_addr: &EndpointAddr, client_endpoint: &Endpoint) -> String {
    let mut inviter = WorldClient::connect(client_endpoint, host_addr.clone())
        .await
        .unwrap();
    let capability = inviter.invite("admin", "ada", None, None).await.unwrap();
    inviter.close();
    capability
}

/// The criterion-1 path over real iroh, with a rendering world: join
/// (view + revision), subscribe to `chat_posted`, read the screen,
/// post chat and read the post back as a semantic diff, then leave.
#[tokio::test]
async fn bridge_joins_reads_screen_posts_chat_and_leaves() {
    let (host, _world) = spawn_world(true).await;
    let client_endpoint = endpoint().await;
    let capability = invite_capability(&host.addr, &client_endpoint).await;

    let mut bridge = McpBridge::start().unwrap();
    let joined = bridge
        .join(
            &client_endpoint,
            host.addr.clone(),
            Some("lobby"),
            &capability,
        )
        .await
        .unwrap();
    assert!(bridge.is_joined());
    assert_eq!(joined.view_revision, Some(0), "first view is revision 0");
    assert!(
        joined.screen_text.contains("hello ada"),
        "join delivers the rendered screen: {}",
        joined.screen_text
    );

    let subscription = bridge
        .subscribe(None, Some(vec!["chat_posted".to_owned()]))
        .await
        .unwrap();
    assert_eq!(
        subscription.event_classes,
        Some(vec!["chat_posted".to_owned()])
    );
    assert_eq!(subscription.view_revision, 0);

    let read = bridge.read_screen(None).await.unwrap();
    assert!(
        read.text.contains("hello ada"),
        "screen read: {}",
        read.text
    );
    assert_eq!(read.view_revision, Some(0));
    assert!(read.diffs.is_empty(), "no diffs before any event");

    let posted = bridge.post_chat("hello agents").await.unwrap();
    assert_eq!(posted.text, "hello agents");
    assert_eq!(posted.diff["kind"], "chat_posted");
    assert_eq!(posted.diff["text"], "hello agents");

    // The post is readable back as a semantic diff: the screen read
    // after the post carries the chat, in world terms.
    let read = bridge.read_screen(None).await.unwrap();
    let seen = read
        .diffs
        .iter()
        .find(|change| change["kind"] == "chat_posted")
        .unwrap_or_else(|| panic!("chat diff in screen read: {:?}", read.diffs));
    assert_eq!(seen["text"], "hello agents");

    // Nothing has been observed as an object: inspection honestly
    // reports it as not visible instead of inventing one.
    assert!(matches!(
        bridge.inspect_object("door:1"),
        Err(BridgeError::ObjectNotVisible(_))
    ));

    bridge.leave().await.unwrap();
    assert!(!bridge.is_joined());
    assert!(matches!(
        bridge.read_screen(None).await,
        Err(BridgeError::Left)
    ));
    assert!(matches!(
        bridge
            .join(
                &client_endpoint,
                host.addr.clone(),
                Some("lobby"),
                &capability
            )
            .await,
        Err(BridgeError::Left)
    ));

    client_endpoint.close().await;
    host.shutdown().await;
}

/// The same path against a chat-only world: no `view` ever arrives,
/// so the screen is read purely from semantic diffs — the join
/// succeeds without a revision, the post carries no stale-decision
/// token, and the diff is still the screen read.
#[tokio::test]
async fn bridge_joins_a_chat_only_world_over_iroh() {
    let (host, _world) = spawn_world(false).await;
    let client_endpoint = endpoint().await;
    let capability = invite_capability(&host.addr, &client_endpoint).await;

    let mut bridge = McpBridge::start().unwrap();
    let joined = bridge
        .join(&client_endpoint, host.addr.clone(), None, &capability)
        .await
        .unwrap();
    assert!(bridge.is_joined());
    assert_eq!(
        joined.view_revision, None,
        "a chat-only world sends no view"
    );
    assert!(joined.screen_text.is_empty());

    bridge
        .subscribe(None, Some(vec!["chat_posted".to_owned()]))
        .await
        .unwrap();
    let posted = bridge.post_chat("hello from the void").await.unwrap();
    assert_eq!(posted.diff["kind"], "chat_posted");
    assert_eq!(posted.diff["text"], "hello from the void");
    assert_eq!(posted.view_revision, None);

    let read = bridge.read_screen(None).await.unwrap();
    assert!(read.text.is_empty(), "still no view to read");
    assert_eq!(read.view_revision, None);
    assert!(
        read.diffs.iter().any(
            |change| change["kind"] == "chat_posted" && change["text"] == "hello from the void"
        ),
        "the screen read is the semantic diff: {:?}",
        read.diffs
    );

    bridge.leave().await.unwrap();
    client_endpoint.close().await;
    host.shutdown().await;
}
