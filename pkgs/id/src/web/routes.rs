//! HTTP route handlers for the web interface.
//!
//! Defines all the Axum routes and their handlers for serving the web UI.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use super::AppState;
use super::content_mode::{ContentMode, detect_mode, detect_mode_with_content, get_content_type};
use super::markdown::prosemirror_to_markdown;
use super::templates::{
    render_binary_viewer, render_editor, render_editor_page, render_file_list,
    render_main_page_wrapper, render_media_viewer, render_page, render_settings,
};

/// Create the main router with all web routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Page routes (return full HTML pages)
        .route("/", get(index_handler))
        .route("/settings", get(settings_handler))
        .route("/edit/:hash", get(edit_handler))
        // Blob route (serves raw file content)
        .route("/blob/:hash", get(blob_handler))
        // HTMX partial routes (return HTML fragments)
        .route("/api/files", get(files_list_handler))
        // File management API routes
        .route("/api/save", post(save_handler))
        .route("/api/new", post(new_file_handler))
        .route("/api/download", post(download_handler))
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
    // TODO: Get actual node ID from state
    let node_id = "0000000000000000000000000000000000000000000000000000000000000000";
    let content = render_settings(node_id);
    if is_htmx_request(&headers) {
        // HTMX request - return wrapped content with header/footer
        Html(render_main_page_wrapper(&content))
    } else {
        Html(render_page("Settings", &content, "", &state.assets))
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

// --- File Management API ---

/// Request body for saving a file.
#[derive(Debug, Deserialize)]
struct SaveRequest {
    /// Current document hash (used to find and archive old tag).
    doc_id: String,
    /// File name (tag name).
    name: String,
    /// `ProseMirror` document JSON.
    doc: serde_json::Value,
}

/// Response from saving a file.
#[derive(Debug, Serialize)]
struct SaveResponse {
    /// New blob hash after save.
    hash: String,
    /// File name.
    name: String,
    /// Archive tag name (if original was archived).
    archive_name: Option<String>,
}

/// Request body for creating a new file.
#[derive(Debug, Deserialize)]
struct NewFileRequest {
    /// File name for the new file.
    name: String,
}

/// Response from creating a new file.
#[derive(Debug, Serialize)]
struct NewFileResponse {
    /// Blob hash of the new file.
    hash: String,
    /// File name.
    name: String,
}

/// Request body for downloading editor content.
#[derive(Debug, Deserialize)]
struct DownloadRequest {
    /// `ProseMirror` document JSON (current editor state).
    doc: serde_json::Value,
    /// File name (used for format detection and Content-Disposition).
    name: String,
    /// Download format: "raw" (native format) or "json" (`ProseMirror` JSON).
    format: String,
}

/// Save the current editor document to the blob store.
///
/// Converts the `ProseMirror` JSON to the appropriate format based on filename,
/// creates a new blob, archives the original under a timestamped name, and
/// updates the tag to point to the new blob.
async fn save_handler(State(state): State<AppState>, Json(req): Json<SaveRequest>) -> Response {
    tracing::info!(
        "[routes] save_handler: doc_id={}, name={}",
        req.doc_id,
        req.name
    );

    // Convert ProseMirror doc to the appropriate format based on file name
    let bytes = match convert_doc_to_bytes(&req.name, &req.doc) {
        Ok(b) => b,
        Err(err) => {
            tracing::error!("[routes] Failed to convert document: {}", err);
            return (StatusCode::BAD_REQUEST, err).into_response();
        }
    };

    // Add new blob to store
    let add_result = state.store.blobs().add_bytes(bytes).await;
    let outcome = match add_result {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save file").into_response();
        }
    };
    let new_hash = outcome.hash;
    let new_hash_str = new_hash.to_string();

    // Archive the original blob under a timestamped tag
    let archive_name = archive_original_tag(&state.store, &req.name, &req.doc_id).await;

    // Set the tag to point to the new blob
    let tag = iroh_blobs::api::Tag::from(req.name.clone());
    if let Err(err) = state.store.tags().set(tag, new_hash).await {
        tracing::error!("[routes] Failed to set tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to update tag").into_response();
    }

    tracing::info!(
        "[routes] File saved: name={}, hash={}, archive={:?}",
        req.name,
        new_hash_str,
        archive_name
    );

    Json(SaveResponse {
        hash: new_hash_str,
        name: req.name,
        archive_name,
    })
    .into_response()
}

/// Create a new empty file in the blob store.
///
/// Creates appropriate empty content based on the file extension,
/// adds it as a blob, and creates a tag for it.
async fn new_file_handler(
    State(state): State<AppState>,
    Json(req): Json<NewFileRequest>,
) -> Response {
    tracing::info!("[routes] new_file_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Create appropriate empty content based on file type
    let mode = detect_mode(&req.name);
    let content = match mode {
        ContentMode::Rich => b"{}".to_vec(), // Empty PM JSON
        ContentMode::Markdown
        | ContentMode::Plain
        | ContentMode::Raw
        | ContentMode::Binary
        | ContentMode::Media(_) => b"".to_vec(),
    };

    // Add blob to store
    let add_result = state.store.blobs().add_bytes(content).await;
    let outcome = match add_result {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create file").into_response();
        }
    };
    let hash = outcome.hash;
    let hash_str = hash.to_string();

    // Set tag
    let tag = iroh_blobs::api::Tag::from(req.name.clone());
    if let Err(err) = state.store.tags().set(tag, hash).await {
        tracing::error!("[routes] Failed to set tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to set file name").into_response();
    }

    tracing::info!(
        "[routes] New file created: name={}, hash={}",
        req.name,
        hash_str
    );

    Json(NewFileResponse {
        hash: hash_str,
        name: req.name,
    })
    .into_response()
}

/// Download the current editor content in the requested format.
///
/// Supports two formats:
/// - `raw`: Converts `ProseMirror` JSON to the native file format (markdown, plain text, etc.)
/// - `json`: Returns the `ProseMirror` JSON document as-is
///
/// For downloading the original stored blob, use `GET /blob/:hash` directly.
async fn download_handler(Json(req): Json<DownloadRequest>) -> Response {
    tracing::info!(
        "[routes] download_handler: name={}, format={}",
        req.name,
        req.format
    );

    match req.format.as_str() {
        "raw" => {
            // Convert PM doc to native format
            let bytes = match convert_doc_to_bytes(&req.name, &req.doc) {
                Ok(b) => b,
                Err(err) => {
                    return (StatusCode::BAD_REQUEST, err).into_response();
                }
            };
            let content_type = get_content_type(&req.name);
            let filename_encoded = urlencoding::encode(&req.name);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename_encoded}\""),
                )
                .body(Body::from(bytes))
                .unwrap_or_else(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to build response",
                    )
                        .into_response()
                })
        }
        "json" => {
            // Return PM JSON as-is
            let json_bytes = serde_json::to_vec_pretty(&req.doc).unwrap_or_default();
            let json_name = if req.name.ends_with(".pm.json") {
                req.name.clone()
            } else {
                format!("{}.pm.json", req.name)
            };
            let filename_encoded = urlencoding::encode(&json_name);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename_encoded}\""),
                )
                .body(Body::from(json_bytes))
                .unwrap_or_else(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to build response",
                    )
                        .into_response()
                })
        }
        _ => (
            StatusCode::BAD_REQUEST,
            "Invalid format. Use 'raw' or 'json'",
        )
            .into_response(),
    }
}

/// Convert a `ProseMirror` document JSON to bytes in the appropriate file format.
///
/// The format is determined by the file name extension:
/// - `.pm.json` → `ProseMirror` JSON (serialized as-is)
/// - `.md` → Markdown (via `prosemirror_to_markdown`)
/// - `.txt` → Plain text (extracted from paragraphs)
/// - Other text files → Raw text (extracted from `code_block` nodes)
fn convert_doc_to_bytes(name: &str, doc: &serde_json::Value) -> Result<Vec<u8>, String> {
    let mode = detect_mode(name);

    match mode {
        ContentMode::Rich => {
            // .pm.json files: store the ProseMirror JSON directly
            serde_json::to_vec_pretty(doc).map_err(|e| format!("Failed to serialize JSON: {e}"))
        }
        ContentMode::Markdown => {
            // .md files: convert PM doc to markdown
            let markdown = prosemirror_to_markdown(doc)
                .map_err(|e| format!("Failed to convert to markdown: {e}"))?;
            Ok(markdown.into_bytes())
        }
        ContentMode::Plain => {
            // .txt files: extract plain text from paragraphs
            let text = extract_plain_text(doc);
            Ok(text.into_bytes())
        }
        ContentMode::Raw => {
            // Code/config files: extract text from code_block nodes
            let text = extract_raw_text(doc);
            Ok(text.into_bytes())
        }
        ContentMode::Binary | ContentMode::Media(_) => {
            Err("Cannot save binary/media files from editor".to_owned())
        }
    }
}

/// Extract plain text from a `ProseMirror` doc with paragraph nodes.
///
/// Joins paragraphs with newlines, preserving `hard_break` as newlines.
fn extract_plain_text(doc: &serde_json::Value) -> String {
    let mut lines = Vec::new();

    if let Some(content) = doc.get("content").and_then(|c| c.as_array()) {
        for node in content {
            let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match node_type {
                "paragraph" => {
                    let mut line = String::new();
                    if let Some(inline_content) = node.get("content").and_then(|c| c.as_array()) {
                        for inline in inline_content {
                            let inline_type =
                                inline.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match inline_type {
                                "text" => {
                                    if let Some(text) = inline.get("text").and_then(|t| t.as_str())
                                    {
                                        line.push_str(text);
                                    }
                                }
                                "hard_break" => {
                                    line.push('\n');
                                }
                                _ => {}
                            }
                        }
                    }
                    lines.push(line);
                }
                "heading" => {
                    let mut text = String::new();
                    if let Some(inline_content) = node.get("content").and_then(|c| c.as_array()) {
                        for inline in inline_content {
                            if let Some(t) = inline.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                    }
                    lines.push(text);
                }
                _ => {}
            }
        }
    }

    lines.join("\n")
}

/// Extract raw text from a `ProseMirror` doc with `code_block` nodes.
///
/// Used for code and config files where the content is a single `code_block`.
fn extract_raw_text(doc: &serde_json::Value) -> String {
    let mut parts = Vec::new();

    if let Some(content) = doc.get("content").and_then(|c| c.as_array()) {
        for node in content {
            let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if node_type == "code_block"
                && let Some(inline_content) = node.get("content").and_then(|c| c.as_array())
            {
                for inline in inline_content {
                    if let Some(text) = inline.get("text").and_then(|t| t.as_str()) {
                        parts.push(text.to_owned());
                    }
                }
            }
        }
    }

    parts.join("\n")
}

/// Archive the original tag by creating a timestamped copy.
///
/// If a tag with the given name exists and points to the expected hash,
/// creates a new tag `{name}.archive.{timestamp}` pointing to the same hash.
async fn archive_original_tag(
    store: &iroh_blobs::api::Store,
    name: &str,
    expected_hash: &str,
) -> Option<String> {
    use futures_lite::StreamExt;

    let expected: iroh_blobs::Hash = expected_hash.parse().ok()?;

    // Find the current tag
    let mut tags = store.tags().list().await.ok()?;
    let mut found = false;
    while let Some(Ok(tag_info)) = tags.next().await {
        let tag_name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
        if tag_name == name && tag_info.hash == expected {
            found = true;
            break;
        }
    }

    if !found {
        return None;
    }

    // Create archive tag with timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let archive_name = format!("{name}.archive.{timestamp}");
    let archive_tag = iroh_blobs::api::Tag::from(archive_name.clone());

    match store.tags().set(archive_tag, expected).await {
        Ok(()) => {
            tracing::info!(
                "[routes] Archived original: {} -> {}",
                archive_name,
                expected_hash
            );
            Some(archive_name)
        }
        Err(err) => {
            tracing::error!("[routes] Failed to create archive tag: {}", err);
            None
        }
    }
}
