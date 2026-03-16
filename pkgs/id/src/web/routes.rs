//! HTTP route handlers for the web interface.
//!
//! Defines all the Axum routes and their handlers for serving the web UI.

use axum::{
    Router,
    extract::{Path, State},
    response::{Html, IntoResponse},
    routing::get,
};

use super::AppState;
use super::templates::{render_editor, render_file_list, render_page, render_settings};

/// Create the main router with all web routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Page routes (return full HTML pages)
        .route("/", get(index_handler))
        .route("/settings", get(settings_handler))
        .route("/edit/:hash", get(edit_handler))
        // HTMX partial routes (return HTML fragments)
        .route("/api/files", get(files_list_handler))
        // WebSocket for collaboration
        .route("/ws/collab/:doc_id", get(super::collab::ws_collab_handler))
        // Static assets
        .route("/assets/*path", get(assets_handler))
        .with_state(state)
}

/// Index page handler - shows file list.
async fn index_handler(State(state): State<AppState>) -> impl IntoResponse {
    let files = get_file_list(&state.store).await;
    let content = render_file_list(&files);
    Html(render_page("Files", &content, "", &state.assets))
}

/// Settings page handler.
async fn settings_handler(State(state): State<AppState>) -> impl IntoResponse {
    // TODO: Get actual node ID from state
    let node_id = "0000000000000000000000000000000000000000000000000000000000000000";
    let content = render_settings(node_id);
    Html(render_page("Settings", &content, "", &state.assets))
}

/// Editor page handler - shows `ProseMirror` editor for a file.
async fn edit_handler(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> impl IntoResponse {
    // Try to find the file name from tags
    let name = get_file_name(&state.store, &hash)
        .await
        .unwrap_or_else(|| hash.clone());

    // Get file content from store
    let content = get_file_content(&state.store, &hash).await;

    let editor_html = render_editor(&hash, &name, &content);
    Html(render_page(
        &format!("Edit: {name}"),
        &editor_html,
        "",
        &state.assets,
    ))
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

/// Get file content as HTML.
///
/// Reads the file content from the blob store and converts it to HTML
/// for display in the editor.
async fn get_file_content(store: &iroh_blobs::api::Store, hash: &str) -> String {
    // Parse the hash
    let Ok(hash) = hash.parse::<iroh_blobs::Hash>() else {
        return "<p>Invalid hash format</p>".to_owned();
    };

    // Read the blob content
    let Ok(bytes) = store.blobs().get_bytes(hash).await else {
        return "<p>File not found</p>".to_owned();
    };

    // Convert to string (lossy for non-UTF8)
    let text = String::from_utf8_lossy(&bytes);

    // Escape HTML and wrap in <pre> for plain text display
    // The editor will handle this as text content
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!("<pre>{escaped}</pre>")
}
