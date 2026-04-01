//! Tag WebSocket and REST API handlers.
//!
//! Provides real-time tag change notifications via WebSocket and a REST API
//! for tag CRUD operations.
//!
//! ## WebSocket (`/ws/tags`)
//!
//! Streams [`TagEvent`]s as JSON text messages. Clients subscribe to receive
//! live updates when tags are set, deleted, or transferred anywhere in the
//! system. No inbound messages are expected (fire-and-forget stream).
//!
//! ## REST API
//!
//! - `GET  /api/tags?subject=README.md` — list tags for a subject
//! - `GET  /api/tags?key=label` — find all files with a given tag key
//! - `GET  /api/tags?key=label&value=rust` — find files by key+value
//! - `GET  /api/tags` — list all tags in the global namespace
//! - `GET  /api/tags/search?q=<query>` — search tags with structured syntax
//! - `POST /api/tags` — set a tag
//! - `DELETE /api/tags` — delete a tag

use axum::{
    Json,
    extract::{Query, State, WebSocketUpgrade, ws::Message},
    http::StatusCode,
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use super::AppState;

// ============================================================================
// REST types
// ============================================================================

/// Query parameters for GET /api/tags.
#[derive(Debug, Deserialize)]
pub struct TagQuery {
    /// Filter by subject (filename).
    pub subject: Option<String>,
    /// Filter by tag key.
    pub key: Option<String>,
    /// Filter by tag value (requires `key`).
    pub value: Option<String>,
    /// Namespace to query (default: "global").
    #[allow(dead_code)]
    pub ns: Option<String>,
}

/// Query parameters for GET /api/tags/search.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Search query string (supports: `key:`, `:value`, `key:value`, `"literal"`, bare word).
    pub q: String,
    /// Namespace to search (default: "global").
    #[allow(dead_code)]
    pub ns: Option<String>,
}

/// Request body for POST /api/tags (set a tag).
#[derive(Debug, Deserialize)]
pub struct SetTagRequest {
    pub subject: String,
    pub key: String,
    pub value: Option<String>,
    /// Namespace to write to (default: "global").
    #[allow(dead_code)]
    pub ns: Option<String>,
}

/// Request body for DELETE /api/tags.
#[derive(Debug, Deserialize)]
pub struct DelTagRequest {
    pub subject: String,
    pub key: String,
    pub value: Option<String>,
    /// If true, delete all tags for this subject.
    #[serde(default)]
    pub all: bool,
    /// Namespace (default: "global").
    #[allow(dead_code)]
    pub ns: Option<String>,
}

/// A tag in REST responses.
#[derive(Debug, Serialize)]
pub struct TagResponse {
    pub subject: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub timestamp: u64,
}

/// Generic success/error response.
#[derive(Debug, Serialize)]
pub struct ApiResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

// ============================================================================
// WebSocket handler
// ============================================================================

/// WebSocket upgrade handler for `/ws/tags`.
///
/// After upgrade, streams tag events as JSON text frames. No inbound
/// messages are processed (ping/pong only for keepalive).
pub async fn ws_tags_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_tags_socket(socket, state))
}

/// Handle a tag WebSocket connection.
///
/// Subscribes to the `TagStore` broadcast channel and forwards events
/// as JSON text messages. Exits when the client disconnects or the
/// broadcast channel is closed.
async fn handle_tags_socket(socket: axum::extract::ws::WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.tag_store.subscribe();

    // Spawn a task to drain inbound messages (keeps the socket alive).
    let drain_handle = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            // Respond to pings, ignore everything else.
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    // Forward broadcast events to the WebSocket.
    loop {
        match rx.recv().await {
            Ok(event) => {
                let json = match serde_json::to_string(&event) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!("[tags_ws] Failed to serialize event: {e}");
                        continue;
                    }
                };
                if sender.send(Message::Text(json)).await.is_err() {
                    break; // Client disconnected.
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("[tags_ws] Lagged {n} events, continuing");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break; // Channel closed (TagStore dropped).
            }
        }
    }

    drain_handle.abort();
    tracing::debug!("[tags_ws] Client disconnected");
}

// ============================================================================
// REST handlers
// ============================================================================

/// GET /api/tags — query tags.
///
/// Supports filtering by subject, key, key+value, or listing all.
pub async fn get_tags_handler(
    State(state): State<AppState>,
    Query(query): Query<TagQuery>,
) -> impl IntoResponse {
    let tag_store = &state.tag_store;
    let ns = &tag_store.global; // TODO: support query.ns for node/custom

    let result = match (
        query.subject.as_deref(),
        query.key.as_deref(),
        query.value.as_deref(),
    ) {
        // Subject + key + value → exact lookup
        (Some(subject), Some(key), value) => tag_store
            .get_by_key(ns, subject.as_bytes(), key.as_bytes())
            .await
            .map(|tags| {
                tags.into_iter()
                    .filter(|t| {
                        value.is_none()
                            || t.value.as_ref().map(super::super::tags::TagValue::as_bytes)
                                == value.map(str::as_bytes)
                    })
                    .collect::<Vec<_>>()
            }),
        // Subject only → all tags for that file
        (Some(subject), None, _) => tag_store.get_tags(ns, subject.as_bytes()).await,
        // Key + value (no subject) → search by key+value across all files
        (None, Some(key), Some(value)) => {
            tag_store
                .find_by_key_value(ns, key.as_bytes(), value.as_bytes())
                .await
        }
        // Key only → search by key name across all files
        (None, Some(key), None) => tag_store.find_by_key(ns, key.as_bytes()).await,
        // No filters → list all
        (None, None, _) => tag_store.list_all(ns).await,
    };

    match result {
        Ok(tags) => {
            let response: Vec<TagResponse> = tags
                .into_iter()
                .map(|t| TagResponse {
                    subject: t.subject.display_lossy(),
                    key: t.key.display_lossy(),
                    value: t.value.map(|v| v.display_lossy()),
                    timestamp: t.timestamp,
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(response))).into_response()
        }
        Err(e) => {
            tracing::error!("[tags_ws] Query failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    message: Some(format!("{e:#}")),
                    count: None,
                }),
            )
                .into_response()
        }
    }
}

/// POST /api/tags — set a tag.
pub async fn set_tag_handler(
    State(state): State<AppState>,
    Json(req): Json<SetTagRequest>,
) -> impl IntoResponse {
    let tag_store = &state.tag_store;
    let ns = &tag_store.global; // TODO: support req.ns

    match tag_store
        .set_tag(
            ns,
            req.subject.as_bytes(),
            req.key.as_bytes(),
            req.value.as_ref().map(String::as_bytes),
            b"",
        )
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                message: None,
                count: None,
            }),
        ),
        Err(e) => {
            tracing::error!("[tags_ws] Set tag failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    message: Some(format!("{e:#}")),
                    count: None,
                }),
            )
        }
    }
}

/// DELETE /api/tags — delete a tag or all tags for a subject.
pub async fn del_tag_handler(
    State(state): State<AppState>,
    Json(req): Json<DelTagRequest>,
) -> impl IntoResponse {
    let tag_store = &state.tag_store;
    let ns = &tag_store.global; // TODO: support req.ns

    let result = if req.all {
        tag_store
            .del_all_tags(ns, req.subject.as_bytes())
            .await
            .map(Some)
    } else {
        tag_store
            .del_tag(
                ns,
                req.subject.as_bytes(),
                req.key.as_bytes(),
                req.value.as_ref().map(String::as_bytes),
            )
            .await
            .map(|()| None)
    };

    match result {
        Ok(count) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                message: None,
                count,
            }),
        ),
        Err(e) => {
            tracing::error!("[tags_ws] Delete tag failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    message: Some(format!("{e:#}")),
                    count: None,
                }),
            )
        }
    }
}

/// GET /api/tags/search?q=<query> — search tags using structured query syntax.
///
/// Query syntax supports `key:`, `:value`, `key:value`, `"literal"`, and
/// bare word searches. Multiple terms (space-separated) are ANDed together.
pub async fn search_tags_handler(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let tag_store = &state.tag_store;
    let ns = &tag_store.global; // TODO: support query.ns

    match tag_store.search_by_query(ns, &query.q).await {
        Ok(tags) => {
            let response: Vec<TagResponse> = tags
                .into_iter()
                .map(|t| TagResponse {
                    subject: t.subject.display_lossy(),
                    key: t.key.display_lossy(),
                    value: t.value.map(|v| v.display_lossy()),
                    timestamp: t.timestamp,
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(response))).into_response()
        }
        Err(e) => {
            tracing::error!("[tags_ws] Search failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    message: Some(format!("{e:#}")),
                    count: None,
                }),
            )
                .into_response()
        }
    }
}
