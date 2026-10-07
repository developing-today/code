//! WebSocket presentation bridge for the optional authoritative world actor.
//!
//! This is only a transport adapter: the protocol lives in
//! [`crate::world_session`] and is shared with the Iroh transport.

use axum::{
    Json,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::Deserialize;

use crate::world_session::{
    Inbound, InviteError, MAX_FRAME_BYTES, SessionIo, WorldService, run_session,
};

#[derive(Debug, Deserialize)]
pub(super) struct InviteRequest {
    display_name: String,
}

/// Narrow, cloneable state for the world endpoints, separate from the rest of
/// the web state so the bridge can be served and tested standalone.
#[derive(Clone, Debug, Default)]
pub struct WorldWebState {
    /// The hosted world, when enabled.
    pub service: Option<WorldService>,
}

/// Routes for the world session bridge; merge into any `Router<S>` with
/// [`WorldWebState`] as the provided state.
pub fn world_routes() -> axum::Router<WorldWebState> {
    axum::Router::new()
        .route("/ws/world", axum::routing::get(handler))
        .route("/api/world/invite", axum::routing::post(invite_handler))
}

async fn invite_handler(
    State(state): State<WorldWebState>,
    headers: HeaderMap,
    Json(request): Json<InviteRequest>,
) -> Response {
    let Some(service) = state.service else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let supplied = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match service.invite(supplied, request.display_name).await {
        Ok(invite) => Json(invite).into_response(),
        Err(InviteError::Disabled) => StatusCode::NOT_FOUND.into_response(),
        Err(InviteError::Unauthorized) => StatusCode::UNAUTHORIZED.into_response(),
        Err(InviteError::Unavailable) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "world cannot issue another invite" })),
        )
            .into_response(),
    }
}

async fn handler(State(state): State<WorldWebState>, ws: WebSocketUpgrade) -> Response {
    let Some(service) = state.service else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ws.max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            let (sender, receiver) = socket.split();
            let mut io = WsIo { sender, receiver };
            run_session(&service, &mut io).await;
        })
}

struct WsIo {
    sender: SplitSink<WebSocket, Message>,
    receiver: SplitStream<WebSocket>,
}

impl SessionIo for WsIo {
    async fn recv(&mut self) -> Inbound {
        loop {
            match self.receiver.next().await {
                Some(Ok(Message::Text(text))) if text.len() <= MAX_FRAME_BYTES => {
                    return Inbound::Text(text.to_string());
                }
                Some(Ok(Message::Text(_))) => return Inbound::Oversized,
                Some(Ok(Message::Binary(_))) => return Inbound::Unsupported,
                Some(Ok(Message::Ping(payload))) => {
                    if self.sender.send(Message::Pong(payload)).await.is_err() {
                        return Inbound::Closed;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => return Inbound::Closed,
            }
        }
    }

    async fn send(&mut self, frame: String) -> bool {
        self.sender.send(Message::Text(frame.into())).await.is_ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::world::{JoinCapability, WorldCore, WorldHandle, WorldLimits};
    use axum::{body::Body, http::Request};

    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    use tower::ServiceExt as _;

    fn lobby() -> WorldHandle {
        WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap())
    }

    fn app(world: Option<WorldHandle>, admin_token: Option<&str>) -> axum::Router {
        world_routes().with_state(WorldWebState {
            service: world.map(|world| WorldService::new(world, admin_token.map(str::to_owned))),
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
