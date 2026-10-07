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
    http::{HeaderMap, StatusCode},
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
        .get("x-world-admin-token")
        .and_then(|value| value.to_str().ok());
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{JoinCapability, WorldCore, WorldHandle, WorldLimits};
    use axum::http::StatusCode;
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
            builder = builder.header("x-world-admin-token", auth);
        }
        builder
            .body(Body::from(r#"{"display_name":"guest"}"#))
            .unwrap()
    }

    #[tokio::test]
    async fn invite_requires_the_admin_secret_header() {
        let router = app(Some(lobby()), Some("s3cret"));
        for auth in [None, Some("wrong"), Some("Bearer s3cret"), Some("")] {
            let response = router.clone().oneshot(invite_request(auth)).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = router
            .oneshot(invite_request(Some("s3cret")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
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
                .oneshot(invite_request(Some("s3cret")))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn admin_secret_is_redacted_from_world_service_debug() {
        let service = WorldService::new(lobby(), Some("never-print-me".to_owned()));
        let printed = format!("{service:?}");
        assert!(printed.contains("REDACTED"));
        assert!(!printed.contains("never-print-me"));
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

    #[cfg(feature = "sandbox")]
    use crate::world_session::encode_hex;

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

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn browser_websocket_can_upload_and_run_a_roc_module() {
        use iroh_blobs::store::mem::MemStore;

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let service = WorldService::new(world.clone(), Some("admin".to_owned()))
            .with_blob_store(blobs.clone());
        let addr = serve(world_routes().with_state(WorldWebState {
            service: Some(service),
        }))
        .await;
        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm");
        let (mut uploader, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/world"))
            .await
            .unwrap();
        uploader
            .send(ClientMessage::Text(
                serde_json::json!({
                    "type": "install_begin",
                    "admin_token": "admin",
                    "total_bytes": wasm.len(),
                    "module_hash": null,
                    "seed": 99,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let ready = next_frame(&mut uploader).await.unwrap();
        assert_eq!(ready["type"], "upload_ready");
        let chunk_bytes = ready["chunk_bytes"].as_u64().unwrap() as usize;
        for (index, chunk) in wasm.chunks(chunk_bytes).enumerate() {
            uploader
                .send(ClientMessage::Text(
                    serde_json::json!({
                        "type": "install_chunk",
                        "offset": index * chunk_bytes,
                        "data_hex": encode_hex(chunk),
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
        }
        uploader
            .send(ClientMessage::Text(
                serde_json::json!({"type": "install_end"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let installed = next_frame(&mut uploader).await.unwrap();
        assert_eq!(installed["type"], "module_installed");
        let hash: iroh_blobs::Hash = installed["module_hash"].as_str().unwrap().parse().unwrap();
        assert!(blobs.blobs().has(hash).await.unwrap());

        let (_, capability) = world
            .issue("browser".to_owned(), crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let mut player = connect(addr, join(&capability, None)).await;
        assert_eq!(next_frame(&mut player).await.unwrap()["type"], "snapshot");
        assert_eq!(next_frame(&mut player).await.unwrap()["data"], "count=0");
        player
            .send(ClientMessage::Text(
                r#"{"type":"input","data_hex":"696e63"}"#.into(),
            ))
            .await
            .unwrap();
        assert_eq!(next_frame(&mut player).await.unwrap()["type"], "event");
        assert_eq!(next_frame(&mut player).await.unwrap()["data"], "count=1");
    }

    #[tokio::test]
    async fn revoked_idle_participant_is_disconnected_immediately() {
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
        let closed = next_frame(&mut a).await.unwrap();
        assert_eq!(closed["type"], "error");
        assert_eq!(closed["message"], "world capability revoked");
        assert!(next_frame(&mut a).await.is_none());
        // An idle peer remains connected and can publish after its neighbor is
        // revoked, which demonstrates revocation didn't poison the world.
        assert_eq!(world.chat(bob, "still here").await.unwrap().sequence, 1);
    }
}
