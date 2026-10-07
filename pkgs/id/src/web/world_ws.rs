//! WebSocket presentation bridge for the optional authoritative world actor.

use axum::{
    Json,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use tokio::sync::broadcast;

use crate::world::{JoinCapability, WorldEvent, WorldHandle};

const MAX_FRAME_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Join {
        capability: String,
        after: Option<u64>,
    },
    Chat {
        text: String,
    },
    Input {
        data_hex: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame<'a> {
    Snapshot {
        snapshot: &'a crate::world::WorldSnapshot,
    },
    Event {
        event: &'a WorldEvent,
    },
    Error {
        message: &'a str,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct InviteRequest {
    display_name: String,
}

#[derive(Debug, Serialize)]
pub(super) struct InviteResponse {
    capability: String,
    participant_id: u64,
    display_name: String,
}

/// Narrow, cloneable state for the world endpoints, separate from the rest of
/// the web state so the bridge can be served and tested standalone.
#[derive(Clone, Default)]
pub struct WorldWebState {
    /// Authoritative in-memory world, when enabled.
    pub world: Option<WorldHandle>,
    /// Admin secret required to mint guest capabilities.
    pub admin_token: Option<String>,
}

impl std::fmt::Debug for WorldWebState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldWebState")
            .field("world", &self.world.is_some())
            .field(
                "admin_token",
                &self.admin_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Routes for the world session bridge; merge into any `Router<S>` with
/// [`WorldWebState`] as the provided state.
pub fn world_routes() -> axum::Router<WorldWebState> {
    axum::Router::new()
        .route("/ws/world", axum::routing::get(handler))
        .route("/api/world/invite", axum::routing::post(invite_handler))
}

pub(super) async fn invite_handler(
    State(state): State<WorldWebState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<InviteRequest>,
) -> Response {
    invite(&state, &headers, request).await
}

async fn invite(
    state: &WorldWebState,
    headers: &axum::http::HeaderMap,
    request: InviteRequest,
) -> Response {
    let Some(admin_token) = state.admin_token.as_deref() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if !supplied.is_some_and(|supplied| secret_eq(admin_token.as_bytes(), supplied.as_bytes())) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(world) = state.world.as_ref() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    match world
        .issue(request.display_name, crate::world::WorldScopes::GUEST)
        .await
    {
        Ok((participant, capability)) => Json(InviteResponse {
            capability: capability.expose().to_owned(),
            participant_id: participant.id,
            display_name: participant.display_name,
        })
        .into_response(),
        Err(_) => (
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "world cannot issue another invite" })),
        )
            .into_response(),
    }
}

fn secret_eq(expected: &[u8], supplied: &[u8]) -> bool {
    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
}

pub(super) async fn handler(
    State(state): State<WorldWebState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let Some(world) = state.world else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    ws.max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| handle(socket, world))
}

async fn handle(socket: WebSocket, world: WorldHandle) {
    let (mut sender, mut receiver) = socket.split();
    let first = match receiver.next().await {
        Some(Ok(Message::Text(text))) if text.len() <= MAX_FRAME_BYTES => text,
        _ => return,
    };
    let ClientFrame::Join { capability, after } = (match serde_json::from_str::<ClientFrame>(&first)
    {
        Ok(frame) => frame,
        Err(_) => {
            let _ = send_error(&mut sender, "first frame must be a valid join request").await;
            return;
        }
    }) else {
        let _ = send_error(&mut sender, "first frame must be a join request").await;
        return;
    };
    if capability.len() > 256 {
        let _ = send_error(&mut sender, "invalid join capability").await;
        return;
    }
    let token = JoinCapability::from_wire(capability);
    let mut events = world.subscribe();
    let snapshot = match world.snapshot(token.clone()).await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            let _ = send_error(&mut sender, "join denied").await;
            return;
        }
    };
    let mut cursor = snapshot.current_sequence;
    if let Some(after) = after {
        match world.events_after(token.clone(), after).await {
            Ok(page) if page.needs_snapshot => {
                let fresh = match world.snapshot(token.clone()).await {
                    Ok(snapshot) => snapshot,
                    Err(_) => return,
                };
                cursor = fresh.current_sequence;
                if send_json(&mut sender, &ServerFrame::Snapshot { snapshot: &fresh })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Ok(page) => {
                cursor = page.current_sequence;
                for event in &page.events {
                    if send_json(&mut sender, &ServerFrame::Event { event })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Err(_) => {
                let _ = send_error(&mut sender, "join denied").await;
                return;
            }
        }
    } else if send_json(
        &mut sender,
        &ServerFrame::Snapshot {
            snapshot: &snapshot,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            inbound = receiver.next() => match inbound {
                Some(Ok(Message::Text(text))) if text.len() <= MAX_FRAME_BYTES => {
                    let frame = match serde_json::from_str::<ClientFrame>(&text) {
                        Ok(frame) => frame,
                        Err(_) => {
                            if send_error(&mut sender, "invalid world frame").await.is_err() { break; }
                            continue;
                        }
                    };
                    let result = match frame {
                        ClientFrame::Chat { text } => world.chat(token.clone(), text).await,
                        ClientFrame::Input { data_hex } => {
                            match decode_hex(&data_hex) {
                                Some(data) => world.input(token.clone(), data).await,
                                None => {
                                    if send_error(&mut sender, "input data must be hexadecimal").await.is_err() { break; }
                                    continue;
                                }
                            }
                        }
                        ClientFrame::Join { .. } => {
                            if send_error(&mut sender, "already joined").await.is_err() { break; }
                            continue;
                        }
                    };
                    if result.is_err() && send_error(&mut sender, "world event rejected").await.is_err() { break; }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(Message::Ping(payload))) => if sender.send(Message::Pong(payload)).await.is_err() { break; },
                Some(Ok(Message::Pong(_))) => {},
                Some(Ok(Message::Binary(_))) => if send_error(&mut sender, "binary frames are not supported").await.is_err() { break; },
                Some(Ok(Message::Text(_))) => if send_error(&mut sender, "world frame is too large").await.is_err() { break; },
            },
            event = events.recv() => match event {
                Ok(event) => {
                    if event.sequence <= cursor {
                        continue;
                    }
                    if world.snapshot(token.clone()).await.is_err() { break; }
                    if send_json(&mut sender, &ServerFrame::Event { event: &event }).await.is_err() { break; }
                    cursor = event.sequence;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if let Ok(snapshot) = world.snapshot(token.clone()).await {
                        if send_json(&mut sender, &ServerFrame::Snapshot { snapshot: &snapshot }).await.is_err() { break; }
                        cursor = snapshot.current_sequence;
                    } else { break; }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

async fn send_json<S>(sender: &mut S, frame: &impl Serialize) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Ok(encoded) = serde_json::to_string(frame) else {
        return Err(());
    };
    sender
        .send(Message::Text(encoded.into()))
        .await
        .map_err(|_| ())
}

async fn send_error<S>(sender: &mut S, message: &'static str) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    send_json(sender, &ServerFrame::Error { message }).await
}

fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) || encoded.len() > MAX_FRAME_BYTES * 2 {
        return None;
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hex_input_parser_is_bounded_and_strict() {
        assert_eq!(decode_hex("00aBff"), Some(vec![0, 0xab, 0xff]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("zz"), None);
        assert_eq!(decode_hex(&"00".repeat(MAX_FRAME_BYTES + 1)), None);
    }

    #[test]
    fn admin_secret_comparison_requires_equal_bytes() {
        assert!(secret_eq(b"admin", b"admin"));
        assert!(!secret_eq(b"admin", b"admiN"));
        assert!(!secret_eq(b"admin", b"admin-longer"));
    }

    #[test]
    fn wire_event_shape_is_presentation_neutral() {
        let event = WorldEvent {
            sequence: 3,
            participant_id: 9,
            kind: WorldEventKind::Chat("hello".to_owned()),
        };
        let encoded = serde_json::to_string(&ServerFrame::Event { event: &event }).unwrap();
        assert!(encoded.contains("\"sequence\":3"));
        assert!(encoded.contains("\"kind\":\"chat\""));
        assert!(encoded.contains("\"data\":\"hello\""));
    }

    use axum::{body::Body, http::Request};

    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    use tower::ServiceExt as _;

    use crate::world::{WorldCore, WorldEventKind, WorldLimits};

    fn lobby() -> WorldHandle {
        WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap())
    }

    fn app(world: Option<WorldHandle>, admin_token: Option<&str>) -> axum::Router {
        world_routes().with_state(WorldWebState {
            world,
            admin_token: admin_token.map(str::to_owned),
        })
    }

    fn invite_request(auth: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/world/invite")
            .header("content-type", "application/json");
        if let Some(auth) = auth {
            builder = builder.header("authorization", auth);
        }
        builder
            .body(Body::from(r#"{"display_name":"guest"}"#))
            .unwrap()
    }

    #[tokio::test]
    async fn invite_requires_the_admin_bearer_secret() {
        let router = app(Some(lobby()), Some("s3cret"));
        for auth in [None, Some("Bearer wrong"), Some("s3cret"), Some("Bearer ")] {
            let response = router.clone().oneshot(invite_request(auth)).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        }
        let response = router
            .oneshot(invite_request(Some("Bearer s3cret")))
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["capability"].as_str().is_some_and(|c| !c.is_empty()));
        assert_eq!(json["display_name"], "guest");
    }

    #[tokio::test]
    async fn invite_is_disabled_without_admin_secret_or_world() {
        for router in [app(Some(lobby()), None), app(None, Some("s3cret"))] {
            let response = router
                .oneshot(invite_request(Some("Bearer s3cret")))
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
        }
    }

    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn serve(router: axum::Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        addr
    }

    async fn connect(addr: std::net::SocketAddr, join: serde_json::Value) -> Client {
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/world"))
            .await
            .unwrap();
        client
            .send(ClientMessage::Text(join.to_string().into()))
            .await
            .unwrap();
        client
    }

    async fn next_frame(client: &mut Client) -> Option<serde_json::Value> {
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("timed out waiting for a world frame");
        match next? {
            Ok(ClientMessage::Text(text)) => Some(serde_json::from_str(&text).unwrap()),
            Ok(ClientMessage::Close(_)) | Err(_) => None,
            Ok(other) => panic!("unexpected frame: {other:?}"),
        }
    }

    fn join(capability: &JoinCapability, after: Option<u64>) -> serde_json::Value {
        serde_json::json!({
            "type": "join",
            "capability": capability.expose(),
            "after": after,
        })
    }

    #[tokio::test]
    async fn websocket_join_broadcast_and_catch_up() {
        let world = lobby();
        let addr = serve(app(Some(world.clone()), Some("admin"))).await;
        let (_, alice) = world
            .issue("alice".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, bob) = world
            .issue("bob".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();

        let mut a = connect(addr, join(&alice, None)).await;
        assert_eq!(next_frame(&mut a).await.unwrap()["type"], "snapshot");
        let mut b = connect(addr, join(&bob, None)).await;
        assert_eq!(next_frame(&mut b).await.unwrap()["type"], "snapshot");

        a.send(ClientMessage::Text(
            r#"{"type":"chat","text":"hello"}"#.to_owned().into(),
        ))
        .await
        .unwrap();
        for client in [&mut a, &mut b] {
            let frame = next_frame(client).await.unwrap();
            assert_eq!(frame["type"], "event");
            assert!(frame.to_string().contains("hello"));
        }

        // Reconnect from before the chat: only the missed events replay, no snapshot.
        let mut late = connect(addr, join(&bob, Some(0))).await;
        let replay = next_frame(&mut late).await.unwrap();
        assert_eq!(replay["type"], "event");
        assert!(replay.to_string().contains("hello"));
    }

    #[tokio::test]
    async fn websocket_rejects_bad_first_frames_and_unknown_capabilities() {
        let world = lobby();
        let addr = serve(app(Some(world), Some("admin"))).await;

        let mut bad = connect(addr, serde_json::json!({"type": "chat", "text": "x"})).await;
        assert_eq!(next_frame(&mut bad).await.unwrap()["type"], "error");
        assert!(next_frame(&mut bad).await.is_none());

        let forged = JoinCapability::from_wire("0".repeat(64));
        let mut denied = connect(addr, join(&forged, None)).await;
        assert_eq!(
            next_frame(&mut denied).await.unwrap()["message"],
            "join denied"
        );
        assert!(next_frame(&mut denied).await.is_none());

        let oversized = JoinCapability::from_wire("f".repeat(300));
        let mut long = connect(addr, join(&oversized, None)).await;
        assert_eq!(next_frame(&mut long).await.unwrap()["type"], "error");
    }

    #[tokio::test]
    async fn revoked_participant_is_disconnected_on_next_event() {
        let world = lobby();
        let addr = serve(app(Some(world.clone()), Some("admin"))).await;
        let (alice_id, alice) = world
            .issue("alice".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, bob) = world
            .issue("bob".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let mut a = connect(addr, join(&alice, None)).await;
        assert_eq!(next_frame(&mut a).await.unwrap()["type"], "snapshot");

        assert!(world.revoke(alice_id.id).await.unwrap());
        world.chat(bob, "anyone there?").await.unwrap();
        assert!(next_frame(&mut a).await.is_none());
    }
}
