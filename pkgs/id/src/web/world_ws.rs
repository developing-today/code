//! WebSocket presentation bridge for the optional authoritative world actor.
//!
//! This is only a transport adapter: the protocol lives in
//! [`crate::world_session`] and is shared with the Iroh transport.

use axum::{
    Json,
    body::Bytes,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::Deserialize;

use crate::directory::Refusal;
use crate::directory_auth::{Caller, Signed};
use crate::directory_view::DirectoryAction;
use crate::world_hub::{ResolveError, WorldHub};
use crate::world_session::{Inbound, InviteError, MAX_FRAME_BYTES, SessionIo, run_session};

#[derive(Debug, Deserialize)]
pub(super) struct InviteRequest {
    display_name: String,
    /// World to invite into (default world if absent).
    world: Option<String>,
    expires_in_secs: Option<u64>,
    uses: Option<u32>,
}

/// Narrow, cloneable state for the world endpoints, separate from the rest of
/// the web state so the bridge can be served and tested standalone.
#[derive(Clone, Debug, Default)]
pub struct WorldWebState {
    /// The hosted worlds, when enabled.
    pub hub: Option<WorldHub>,
}

/// Routes for the world session bridge; merge into any `Router<S>` with
/// [`WorldWebState`] as the provided state.
pub fn world_routes() -> axum::Router<WorldWebState> {
    axum::Router::new()
        .route("/ws/world", axum::routing::get(handler))
        .route("/api/world/invite", axum::routing::post(invite_handler))
        .merge(super::explore::explore_routes())
        .route(
            "/api/world/directory",
            axum::routing::get(directory_read_handler).post(directory_write_handler),
        )
}

#[derive(Debug, Deserialize)]
pub(super) struct DirectoryQuery {
    /// World to read (default world if absent).
    world: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DirectoryRequest {
    /// World to act on (default world if absent).
    world: Option<String>,
    #[serde(flatten)]
    action: DirectoryAction,
}

async fn directory_read_handler(
    State(state): State<WorldWebState>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    Query(query): Query<DirectoryQuery>,
) -> Response {
    directory_respond(
        state,
        Asked {
            headers: &headers,
            method: &method,
            uri: &uri,
            body: &[],
        },
        query.world,
        DirectoryAction::View,
    )
    .await
}

async fn directory_write_handler(
    State(state): State<WorldWebState>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let request: DirectoryRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return bad_request(&error.to_string()),
    };
    directory_respond(
        state,
        Asked {
            headers: &headers,
            method: &method,
            uri: &uri,
            body: &body,
        },
        request.world,
        request.action,
    )
    .await
}

/// One HTTP request as the directory sees it: what it presented, and what
/// a signature over it must cover.
struct Asked<'a> {
    headers: &'a HeaderMap,
    method: &'a Method,
    uri: &'a Uri,
    body: &'a [u8],
}

/// The admin token, in `x-world-admin-token`, is read from a header, and the
/// account credential or session, in `authorization: Bearer ...`, too, so
/// neither lands in a URL. A signing key comes from the `x-id-*` headers.
async fn directory_respond(
    state: WorldWebState,
    asked: Asked<'_>,
    world: Option<String>,
    action: DirectoryAction,
) -> Response {
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let token = asked
        .headers
        .get("x-world-admin-token")
        .and_then(|value| value.to_str().ok());
    let lease = match hub.lease(world.as_deref(), token).await {
        Ok(lease) => lease,
        Err(ResolveError::Busy | ResolveError::TooManyWorlds) => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let caller = match caller_of(&hub, &asked, token) {
        Ok(caller) => caller,
        Err(error) => return refusal_response(&error),
    };
    match lease.service().directory(caller, action).await {
        Ok(outcome) => ([(header::CACHE_CONTROL, "no-store")], Json(outcome)).into_response(),
        Err(error) => refusal_response(&error),
    }
}

fn caller_of(hub: &WorldHub, asked: &Asked<'_>, token: Option<&str>) -> anyhow::Result<Caller> {
    let mut caller = Caller {
        admin: token.is_some_and(|token| hub.admin_ok(token)),
        ..Caller::default()
    };
    if let Some(secret) = asked
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
    {
        let presented = Caller::from_secret(secret);
        caller.credential = presented.credential;
        caller.session = presented.session;
    }
    if let Some(signed) = signed_headers(asked.headers) {
        let target = asked
            .uri
            .path_and_query()
            .map_or_else(|| asked.uri.path(), axum::http::uri::PathAndQuery::as_str);
        caller.key = Some(hub.verify_signed(&signed, asked.method.as_str(), target, asked.body)?);
    }
    Ok(caller)
}

fn signed_headers(headers: &HeaderMap) -> Option<Signed> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    Some(Signed {
        key: text("x-id-key")?,
        at: text("x-id-at")?.parse().ok()?,
        nonce: text("x-id-nonce")?,
        signature: text("x-id-signature")?,
    })
}

pub(super) fn refusal_status(error: &anyhow::Error) -> StatusCode {
    match error.downcast_ref::<Refusal>() {
        Some(Refusal::Unauthenticated(_)) => StatusCode::UNAUTHORIZED,
        Some(Refusal::Forbidden(_)) => StatusCode::FORBIDDEN,
        Some(Refusal::NotFound(_)) => StatusCode::NOT_FOUND,
        Some(Refusal::Conflict(_)) => StatusCode::CONFLICT,
        Some(Refusal::RateLimited(_)) => StatusCode::TOO_MANY_REQUESTS,
        None => StatusCode::BAD_REQUEST,
    }
}

fn refusal_response(error: &anyhow::Error) -> Response {
    (
        refusal_status(error),
        Json(serde_json::json!({ "error": format!("{error:#}") })),
    )
        .into_response()
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

async fn invite_handler(
    State(state): State<WorldWebState>,
    headers: HeaderMap,
    Json(request): Json<InviteRequest>,
) -> Response {
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let supplied = headers
        .get("x-world-admin-token")
        .and_then(|value| value.to_str().ok());
    // A valid admin token may create the world it names.
    let lease = match hub.lease(request.world.as_deref(), supplied).await {
        Ok(lease) => lease,
        Err(ResolveError::Busy | ResolveError::TooManyWorlds) => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    match lease
        .service()
        .invite(
            supplied,
            request.display_name,
            request.expires_in_secs,
            request.uses,
        )
        .await
    {
        Ok(invite) => Json(invite).into_response(),
        Err(InviteError::InvalidBounds(message)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response(),
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
    let Some(hub) = state.hub else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ws.max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            let (sender, receiver) = socket.split();
            let mut io = WsIo { sender, receiver };
            run_session(&hub, &mut io).await;
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
    use crate::world_session::WorldService;
    use axum::http::StatusCode;
    use axum::{body::Body, http::Request};
    use ed25519_dalek::SigningKey;

    use crate::directory_auth::sign_request;

    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    use tower::ServiceExt as _;

    fn lobby() -> WorldHandle {
        WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap())
    }

    fn app(world: Option<WorldHandle>, admin_token: Option<&str>) -> axum::Router {
        world_routes().with_state(WorldWebState {
            hub: world.map(|world| {
                WorldHub::single(WorldService::new(world, admin_token.map(str::to_owned)))
            }),
        })
    }

    fn invite_request(auth: Option<&str>) -> Request<Body> {
        invite_body(auth, r#"{"display_name":"guest"}"#)
    }

    fn invite_body(auth: Option<&str>, body: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/world/invite")
            .header("content-type", "application/json");
        if let Some(auth) = auth {
            builder = builder.header("x-world-admin-token", auth);
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    #[tokio::test]
    async fn invite_bounds_are_checked_after_the_admin_secret() {
        let router = app(Some(lobby()), Some("s3cret"));
        for body in [
            r#"{"display_name":"g","expires_in_secs":0}"#,
            r#"{"display_name":"g","expires_in_secs":2592001}"#,
            r#"{"display_name":"g","uses":0}"#,
        ] {
            let response = router
                .clone()
                .oneshot(invite_body(Some("s3cret"), body))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        }
        let response = router
            .clone()
            .oneshot(invite_body(
                Some("wrong"),
                r#"{"display_name":"g","uses":0}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = router
            .oneshot(invite_body(
                Some("s3cret"),
                r#"{"display_name":"g","expires_in_secs":3600,"uses":2}"#,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    fn directory_request(
        method: &str,
        admin: Option<&str>,
        credential: Option<&str>,
        body: &str,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/world/directory")
            .header("content-type", "application/json");
        if let Some(admin) = admin {
            builder = builder.header("x-world-admin-token", admin);
        }
        if let Some(credential) = credential {
            builder = builder.header("authorization", format!("Bearer {credential}"));
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn the_directory_is_signed_up_to_and_read_over_http() {
        let router = app(Some(lobby()), Some("s3cret"));
        let signed = router
            .clone()
            .oneshot(directory_request(
                "POST",
                None,
                None,
                r#"{"action":"sign_up","name":"Ada"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(signed.status(), StatusCode::OK);
        let signed = body_json(signed).await;
        let credential = signed["credential"].as_str().unwrap().to_owned();
        let ada = signed["view"]["viewer"].as_str().unwrap().to_owned();

        let anonymous = body_json(
            router
                .clone()
                .oneshot(directory_request("GET", None, None, ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(anonymous["view"]["viewer"], "anonymous");
        assert_eq!(anonymous["view"]["accounts"][0]["name"], "Ada");
        assert!(anonymous["view"]["accounts"][0]["verified"].is_null());

        let admin = body_json(
            router
                .clone()
                .oneshot(directory_request("GET", Some("s3cret"), None, ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(admin["view"]["viewer"], "admin");
        assert_eq!(admin["view"]["accounts"][0]["verified"], false);

        let own = body_json(
            router
                .clone()
                .oneshot(directory_request("GET", None, Some(&credential), ""))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(own["view"]["viewer"], ada);

        let refused = router
            .oneshot(directory_request("GET", None, Some("acct.nobody.00"), ""))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    }

    fn signed_directory_request(key: &SigningKey, nonce: &str, body: &str) -> Request<Body> {
        let at = crate::world::unix_ms();
        let signed = sign_request(
            key,
            "POST",
            "/api/world/directory",
            body.as_bytes(),
            at,
            nonce,
        );
        Request::builder()
            .method("POST")
            .uri("/api/world/directory")
            .header("content-type", "application/json")
            .header("x-id-key", signed.key)
            .header("x-id-at", signed.at.to_string())
            .header("x-id-nonce", signed.nonce)
            .header("x-id-signature", signed.signature)
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    #[tokio::test]
    async fn a_signed_key_signs_up_and_opens_a_session_that_reads_as_its_account() {
        let router = app(Some(lobby()), None);
        let key = SigningKey::from_bytes(&[8; 32]);
        let id = crate::world::hex_encode(&key.verifying_key().to_bytes());

        let signed_up = router
            .clone()
            .oneshot(signed_directory_request(
                &key,
                "n-up",
                r#"{"action":"sign_up_key","name":"Kay"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(signed_up.status(), StatusCode::OK);

        let replay = router
            .clone()
            .oneshot(signed_directory_request(
                &key,
                "n-up",
                r#"{"action":"view"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

        let opened = body_json(
            router
                .clone()
                .oneshot(signed_directory_request(
                    &key,
                    "n-open",
                    r#"{"action":"open_session"}"#,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(opened["view"]["viewer"], id);
        let session = opened["session"].as_str().unwrap().to_owned();
        assert!(session.starts_with("sess."));

        let read = body_json(
            router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/world/directory")
                        .header("authorization", format!("Bearer {session}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(read["view"]["viewer"], id);
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
    async fn http_invites_and_websocket_joins_route_by_world_name() {
        let opener = crate::world_hub::testing::MemoryOpener::new(false);
        let dyn_opener: std::sync::Arc<dyn crate::world_hub::WorldOpener> = opener.clone();
        let hub = WorldHub::new(
            dyn_opener,
            Some("admin".to_owned()),
            "lobby",
            crate::world_hub::HubLimits::default(),
        );
        let router = world_routes().with_state(WorldWebState { hub: Some(hub) });

        let invite = |world: Option<&str>, auth: &str| {
            let body = match world {
                Some(world) => format!(r#"{{"display_name":"ann","world":"{world}"}}"#),
                None => r#"{"display_name":"ann"}"#.to_owned(),
            };
            Request::builder()
                .method("POST")
                .uri("/api/world/invite")
                .header("content-type", "application/json")
                .header("x-world-admin-token", auth)
                .body(Body::from(body))
                .unwrap()
        };
        // Outsiders cannot conjure worlds by naming them.
        let response = router
            .clone()
            .oneshot(invite(Some("arena"), "wrong"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        // An admin can: the world is created on first invite.
        let response = router
            .clone()
            .oneshot(invite(Some("arena"), "admin"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let capability = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let addr = serve(router).await;

        let mut client = connect(
            addr,
            serde_json::json!({"type":"join","capability": capability,"world":"arena"}),
        )
        .await;
        let snapshot = next_frame(&mut client).await.unwrap();
        assert_eq!(snapshot["snapshot"]["world_id"], "arena");
        // The same capability, without naming the world, finds no such world.
        let mut stray = connect(
            addr,
            serde_json::json!({"type":"join","capability": capability}),
        )
        .await;
        assert_eq!(
            next_frame(&mut stray).await.unwrap()["message"],
            "unknown world"
        );
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

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn browser_websocket_can_upload_and_run_a_roc_module() {
        use iroh_blobs::store::mem::MemStore;

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let service = WorldService::new(world.clone(), Some("admin".to_owned()))
            .with_blob_store(blobs.clone());
        let addr = serve(world_routes().with_state(WorldWebState {
            hub: Some(WorldHub::single(service)),
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
