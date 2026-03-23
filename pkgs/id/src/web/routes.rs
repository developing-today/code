//! HTTP route handlers for the web interface.
//!
//! Defines all the Axum routes and their handlers for serving the web UI.

use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

use super::AppState;
use super::content_mode::{ContentMode, detect_mode_with_content, get_content_type};
use super::templates::{
    render_binary_viewer, render_editor, render_editor_page, render_file_list,
    render_main_page_wrapper, render_media_viewer, render_page, render_peers, render_settings,
};

/// Create the main router with all web routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Page routes (return full HTML pages)
        .route("/", get(index_handler))
        .route("/settings", get(settings_handler))
        .route("/peers", get(peers_handler))
        .route("/edit/:hash", get(edit_handler))
        // Blob route (serves raw file content)
        .route("/blob/:hash", get(blob_handler))
        // HTMX partial routes (return HTML fragments)
        .route("/api/files", get(files_list_handler))
        // WebSocket for collaboration
        .route("/ws/collab/:doc_id", get(super::collab::ws_collab_handler))
        // Static assets
        .route("/assets/*path", get(assets_handler))
        .with_state(state)
}

/// Check if this is an HTMX request (partial content).
fn is_htmx_request(headers: &HeaderMap) -> bool {
    let is_htmx = headers.contains_key("HX-Request");
    tracing::debug!("[routes] is_htmx_request: {}", is_htmx);
    is_htmx
}

/// Index page handler - shows file list.
async fn index_handler(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let files = get_file_list(&state.store).await;
    let content = render_file_list(&files);
    if is_htmx_request(&headers) {
        // HTMX request - return wrapped content with header/footer
        Html(render_main_page_wrapper(&content))
    } else {
        Html(render_page("Files", &content, "", &state.assets))
    }
}

/// Settings page handler.
async fn settings_handler(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let content = render_settings(&state.node_id);
    if is_htmx_request(&headers) {
        // HTMX request - return wrapped content with header/footer
        Html(render_main_page_wrapper(&content))
    } else {
        Html(render_page("Settings", &content, "", &state.assets))
    }
}

/// Peers page handler - shows discovered peers from gossip.
async fn peers_handler(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let peers_data: Vec<(String, String, u64, u64)> = if let Some(ref discovery) = state.peers {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        discovery
            .peers()
            .into_iter()
            .map(|info| {
                let ann = &info.announcement;
                let name = ann.name.clone().unwrap_or_else(|| "-".to_owned());
                let age = now.saturating_sub(ann.timestamp_secs);
                (ann.node_id.to_string(), name, ann.blob_count, age)
            })
            .collect()
    } else {
        Vec::new()
    };

    let content = render_peers(&peers_data);
    if is_htmx_request(&headers) {
        Html(render_main_page_wrapper(&content))
    } else {
        Html(render_page("Peers", &content, "", &state.assets))
    }
}

/// Query parameters for blob requests.
#[derive(Debug, Deserialize)]
struct BlobQuery {
    /// Optional filename for Content-Type detection.
    filename: Option<String>,
}

/// Editor page handler - shows `ProseMirror` editor for a file.
///
/// Routes to different views based on content mode:
/// - Editable modes (Rich, Markdown, Plain, Raw) → Editor
/// - Media modes (Image, Video, Audio, Pdf) → Media viewer
/// - Binary → Binary viewer with download option
async fn edit_handler(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    tracing::info!("[routes] edit_handler called for hash: {}", hash);

    // Try to find the file name from tags
    let name = get_file_name(&state.store, &hash)
        .await
        .unwrap_or_else(|| hash.clone());
    tracing::debug!("[routes] Resolved name: {}", name);

    // Get file content bytes from store
    let content_result = get_file_bytes(&state.store, &hash).await;

    let is_htmx = is_htmx_request(&headers);
    tracing::info!("[routes] edit_handler is_htmx={}, name={}", is_htmx, name);

    match content_result {
        Ok(bytes) => {
            // Detect content mode from filename and content
            let mode = detect_mode_with_content(&name, &bytes);

            match mode {
                ContentMode::Media(media_type) => {
                    // Render media viewer
                    let viewer_html = render_media_viewer(&hash, &name, media_type);
                    if is_htmx {
                        Html(viewer_html)
                    } else {
                        Html(render_page(
                            &format!("View: {name}"),
                            &viewer_html,
                            "",
                            &state.assets,
                        ))
                    }
                }
                ContentMode::Binary => {
                    // Render binary viewer with download option
                    let viewer_html = render_binary_viewer(&hash, &name);
                    if is_htmx {
                        Html(viewer_html)
                    } else {
                        Html(render_page(
                            &format!("File: {name}"),
                            &viewer_html,
                            "",
                            &state.assets,
                        ))
                    }
                }
                _ => {
                    // Editable modes - convert bytes to HTML for editor
                    let content = get_file_content_html(&bytes);
                    if is_htmx {
                        Html(render_editor(&hash, &name, &content))
                    } else {
                        Html(render_editor_page(&hash, &name, &content, &state.assets))
                    }
                }
            }
        }
        Err(err_msg) => {
            // Error loading file
            if is_htmx {
                Html(render_editor(&hash, &name, &err_msg))
            } else {
                Html(render_editor_page(&hash, &name, &err_msg, &state.assets))
            }
        }
    }
}

/// Blob handler - serves raw file content with appropriate Content-Type.
///
/// Used for media files to render directly in browser (images, video, audio, PDF).
async fn blob_handler(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    Query(query): Query<BlobQuery>,
) -> Response {
    // Parse the hash
    let Ok(parsed_hash) = hash.parse::<iroh_blobs::Hash>() else {
        return (StatusCode::BAD_REQUEST, "Invalid hash format").into_response();
    };

    // Read the blob content
    let Ok(bytes) = state.store.blobs().get_bytes(parsed_hash).await else {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    };

    // Determine content type from filename if provided, otherwise from hash lookup
    let filename = match query.filename {
        Some(name) => name,
        None => get_file_name(&state.store, &hash)
            .await
            .unwrap_or_else(|| "file.bin".to_owned()),
    };

    let content_type = get_content_type(&filename);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(bytes.to_vec()))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to build response",
            )
                .into_response()
        })
}

/// API handler for file list (HTMX partial).
async fn files_list_handler(State(state): State<AppState>) -> impl IntoResponse {
    let files = get_file_list(&state.store).await;
    Html(render_file_list(&files))
}

/// Static assets handler.
async fn assets_handler(Path(path): Path<String>) -> impl IntoResponse {
    super::assets::static_handler(&path)
}

/// Get list of files from the store.
///
/// Returns a list of (name, hash, size) tuples.
async fn get_file_list(store: &iroh_blobs::api::Store) -> Vec<(String, String, u64)> {
    use futures_lite::StreamExt;

    let mut files = Vec::new();

    // List all tags
    let Ok(mut tags) = store.tags().list().await else {
        return files;
    };
    while let Some(Ok(tag_info)) = tags.next().await {
        let name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
        let hash = tag_info.hash.to_string();

        // Get blob size - for now we skip size since the API is complex
        // TODO: Use blobs().status() when available
        let size = 0;

        files.push((name, hash, size));
    }

    files
}

/// Get the human-readable name for a hash.
async fn get_file_name(store: &iroh_blobs::api::Store, hash: &str) -> Option<String> {
    use futures_lite::StreamExt;

    // Parse hash
    let hash: iroh_blobs::Hash = hash.parse().ok()?;

    // Find tag with this hash
    let mut tags = store.tags().list().await.ok()?;
    while let Some(Ok(tag_info)) = tags.next().await {
        if tag_info.hash == hash {
            return Some(String::from_utf8_lossy(tag_info.name.as_ref()).to_string());
        }
    }

    None
}

/// Get file bytes from the blob store.
///
/// Returns the file content as bytes if found, or an HTML error message.
async fn get_file_bytes(store: &iroh_blobs::api::Store, hash: &str) -> Result<Vec<u8>, String> {
    // Parse the hash
    let hash = hash
        .parse::<iroh_blobs::Hash>()
        .map_err(|_err| "<p>Invalid hash format</p>".to_owned())?;

    // Read the blob content
    store
        .blobs()
        .get_bytes(hash)
        .await
        .map(|b| b.to_vec())
        .map_err(|_err| "<p>File not found</p>".to_owned())
}

/// Convert file bytes to HTML for editor display.
fn get_file_content_html(bytes: &[u8]) -> String {
    // Convert to string (lossy for non-UTF8)
    let text = String::from_utf8_lossy(bytes);

    // Escape HTML and wrap in <pre> for plain text display
    // The editor will handle this as text content
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!("<pre>{escaped}</pre>")
}
