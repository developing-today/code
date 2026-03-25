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
    render_file_list_content, render_main_page_wrapper, render_media_viewer, render_page,
    render_peers, render_settings,
};

/// Default number of files per page.
const DEFAULT_PER_PAGE: usize = 50;

/// Query parameters for the file list (pagination + search).
#[derive(Debug, Deserialize, Default)]
pub struct FileListQuery {
    /// Current page (1-indexed). Defaults to 1.
    pub page: Option<usize>,
    /// Items per page. Defaults to [`DEFAULT_PER_PAGE`].
    pub per_page: Option<usize>,
    /// Search query — matches filenames and tag keys/values.
    pub search: Option<String>,
    /// Whether to show soft-deleted files. Defaults to false.
    pub show_deleted: Option<bool>,
}

/// Paginated file list result.
pub struct FileListPage {
    /// Files for the current page.
    pub files: Vec<FileInfo>,
    /// Total number of files (after filtering, before pagination).
    pub total: usize,
    /// Current page (1-indexed).
    pub page: usize,
    /// Items per page.
    pub per_page: usize,
    /// The search query (if any).
    pub search: Option<String>,
    /// Whether deleted files are shown.
    pub show_deleted: bool,
}

/// Classification of a file based on its tag name pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    /// Normal user-created file.
    Primary,
    /// Auto-backup file matching `auto-{ISO_DATE}` pattern.
    Auto,
    /// Archive file matching `{name}.archive.{unix_timestamp}` pattern.
    Archive,
}

/// Rich file metadata for the file list UI.
#[derive(Debug, Clone)]
#[allow(dead_code)] // size field reserved for future use
pub struct FileInfo {
    /// Tag name (filename).
    pub name: String,
    /// Display name derived from name/file/path metadata tags.
    /// Falls back to `name` if no metadata tags exist.
    pub display_name: Option<String>,
    /// Content hash.
    pub hash: String,
    /// File size in bytes.
    pub size: u64,
    /// File classification.
    pub kind: FileKind,
    /// For auto/archive files: the primary file they relate to (by shared hash).
    pub parent_name: Option<String>,
    /// Unix timestamp parsed from the tag name (if available).
    pub timestamp: Option<u64>,
    /// Creation timestamp from metadata tags (unix seconds).
    pub created_at: Option<u64>,
    /// Last modification timestamp from metadata tags (unix seconds).
    pub modified_at: Option<u64>,
    /// User-visible tags (excludes system tags like created/modified/deleted/archive.*).
    pub tags: Vec<(String, Option<String>)>,
    /// Whether this file is soft-deleted.
    pub is_deleted: bool,
}

/// Classify a tag name into a `FileKind` and extract an optional timestamp.
fn classify_tag(name: &str) -> (FileKind, Option<u64>) {
    // Check for auto-backup: "auto-{ISO_DATE}"
    if let Some(iso_str) = name.strip_prefix("auto-") {
        // Try to parse the ISO date to extract a unix timestamp
        // Format: 2026-03-12T06:42:30.015Z
        let ts = parse_iso_timestamp(iso_str);
        return (FileKind::Auto, ts);
    }

    // Check for archive: "{name}.archive.{unix_timestamp}"
    if let Some(pos) = name.rfind(".archive.") {
        let ts_str = &name[pos + ".archive.".len()..];
        let ts = ts_str.parse::<u64>().ok();
        return (FileKind::Archive, ts);
    }

    (FileKind::Primary, None)
}

/// Parse a simplified ISO 8601 timestamp to unix seconds.
///
/// Handles format like `2026-03-12T06:42:30.015Z` or `2026-03-12T06:42:30Z`.
/// Returns `None` if parsing fails.
fn parse_iso_timestamp(s: &str) -> Option<u64> {
    // Strip trailing 'Z' and optional fractional seconds
    let s = s.strip_suffix('Z').unwrap_or(s);
    let s = if let Some(dot_pos) = s.rfind('.') {
        &s[..dot_pos]
    } else {
        s
    };

    // Parse: YYYY-MM-DDThh:mm:ss
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }

    let date_parts: Vec<u64> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time_parts: Vec<u64> = parts[1].split(':').filter_map(|p| p.parse().ok()).collect();

    if date_parts.len() != 3 || time_parts.len() != 3 {
        return None;
    }

    let (year, month, day) = (date_parts[0], date_parts[1], date_parts[2]);
    let (hour, min, sec) = (time_parts[0], time_parts[1], time_parts[2]);

    // Simple days-since-epoch calculation (not accounting for leap seconds)
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    let month_days = [
        31,
        28 + u64::from(is_leap_year(year)),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    #[allow(clippy::cast_possible_truncation)] // month is max 12, fits in usize
    for m in 0..(month.saturating_sub(1) as usize) {
        if m < month_days.len() {
            days += month_days[m];
        }
    }
    days += day.saturating_sub(1);

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

/// Check if a year is a leap year.
const fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Create the main router with all web routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Page routes (return full HTML pages)
        .route("/", get(index_handler))
        .route("/settings", get(settings_handler))
        .route("/peers", get(peers_handler))
        .route("/edit/:hash", get(edit_handler))
        .route("/file/*name", get(file_by_name_handler))
        // Blob route (serves raw file content)
        .route("/blob/:hash", get(blob_handler))
        // HTMX partial routes (return HTML fragments)
        .route("/api/files", get(files_list_handler))
        // File management API routes
        .route("/api/save", post(save_handler))
        .route("/api/new", post(new_file_handler))
        .route("/api/rename", post(rename_handler))
        .route("/api/copy", post(copy_handler))
        .route("/api/download", post(download_handler))
        .route("/api/delete", post(delete_handler))
        .route("/api/restore", post(restore_handler))
        .route("/api/hard-delete", post(hard_delete_handler))
        // Tag REST API
        .route(
            "/api/tags",
            get(super::tags_ws::get_tags_handler)
                .post(super::tags_ws::set_tag_handler)
                .delete(super::tags_ws::del_tag_handler),
        )
        // Tag search endpoint with structured query syntax
        .route("/api/tags/search", get(super::tags_ws::search_tags_handler))
        // WebSocket for collaboration
        .route("/ws/collab/:doc_id", get(super::collab::ws_collab_handler))
        // WebSocket for live tag updates
        .route("/ws/tags", get(super::tags_ws::ws_tags_handler))
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
async fn index_handler(
    State(state): State<AppState>,
    Query(query): Query<FileListQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let page = get_file_list_page(&state, &query).await;
    let content = render_file_list(&page);
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
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
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

/// Look up a content hash for a given tag name.
///
/// Iterates all tags in the store to find one matching `name` and returns its hash.
async fn get_hash_for_name(store: &iroh_blobs::api::Store, name: &str) -> Option<String> {
    use futures_lite::StreamExt;

    let mut tags = store.tags().list().await.ok()?;
    while let Some(Ok(tag_info)) = tags.next().await {
        let tag_name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
        if tag_name == name {
            return Some(tag_info.hash.to_string());
        }
    }
    None
}

/// Escape HTML special characters for inline error messages.
fn html_escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Handler for `/file/*name` — resolve a file by tag name and render the editor.
///
/// Looks up the content hash for the given name, then delegates to the same
/// edit/view logic as [`edit_handler`].
async fn file_by_name_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    tracing::info!("[routes] file_by_name_handler called for name: {}", name);

    let is_htmx = is_htmx_request(&headers);

    // Look up the hash for this tag name
    let Some(hash) = get_hash_for_name(&state.store, &name).await else {
        let escaped = html_escape_inline(&name);
        let err_html = format!("<p>File not found: {escaped}</p>");
        if is_htmx {
            return Html(render_editor("", &name, &err_html));
        }
        return Html(render_editor_page("", &name, &err_html, &state.assets));
    };

    // Get file content bytes from store
    let content_result = get_file_bytes(&state.store, &hash).await;

    match content_result {
        Ok(bytes) => {
            let mode = detect_mode_with_content(&name, &bytes);

            match mode {
                ContentMode::Media(media_type) => {
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
async fn files_list_handler(
    State(state): State<AppState>,
    Query(query): Query<FileListQuery>,
) -> impl IntoResponse {
    let page = get_file_list_page(&state, &query).await;
    // Return just the inner content (list + pagination) for HTMX partial replacement
    Html(render_file_list_content(&page))
}

/// Static assets handler.
async fn assets_handler(Path(path): Path<String>) -> impl IntoResponse {
    super::assets::static_handler(&path)
}

/// System tag keys that are excluded from user-visible tags display.
const SYSTEM_TAG_KEYS: &[&str] = &["created", "modified", "deleted", "name", "file", "path"];

/// Check if a tag key is a system/internal key that shouldn't be shown in tag pills.
fn is_system_tag_key(key: &[u8]) -> bool {
    // Check against known system keys (all are valid UTF-8)
    if let Ok(s) = std::str::from_utf8(key) {
        SYSTEM_TAG_KEYS.contains(&s) || s.starts_with("archive.")
    } else {
        false
    }
}

/// Get list of files from the store with classification metadata.
///
/// Returns a list of `FileInfo` structs with file kind, parent name, timestamp, and user tags.
/// Files are sorted: primary files first (alphabetically), then auto, then archive.
/// Internal tags (starting with `.`) are excluded.
/// Soft-deleted files are included but marked with `is_deleted = true`.
async fn get_file_list(state: &AppState) -> Vec<FileInfo> {
    use futures_lite::StreamExt;
    use std::collections::HashMap;

    let mut files = Vec::new();

    // Load all tags from the global namespace for created/modified/user-tag lookups.
    let all_meta_tags = state
        .tag_store
        .list_all(&state.tag_store.global)
        .await
        .unwrap_or_default();
    let mut created_map: HashMap<String, u64> = HashMap::new();
    let mut modified_map: HashMap<String, u64> = HashMap::new();
    let mut deleted_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut user_tags_map: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    let mut display_name_map: HashMap<String, String> = HashMap::new();
    for tag in &all_meta_tags {
        if tag.key == "created" {
            if let Some(ref v) = tag.value
                && let Some(s) = v.as_str()
                && let Ok(ts) = s.parse::<u64>()
            {
                created_map.insert(tag.subject.display_lossy(), ts);
            }
        } else if tag.key == "modified" {
            if let Some(ref v) = tag.value
                && let Some(s) = v.as_str()
                && let Ok(ts) = s.parse::<u64>()
            {
                modified_map.insert(tag.subject.display_lossy(), ts);
            }
        } else if tag.key == "deleted" {
            deleted_set.insert(tag.subject.display_lossy());
        }
        // Resolve display name from name/file/path tags (prefer name > file > path)
        if matches!(tag.key.as_bytes(), b"name" | b"file" | b"path") {
            let subj = tag.subject.display_lossy();
            if !display_name_map.contains_key(&subj) {
                if let Some(ref v) = tag.value {
                    if let Some(s) = v.as_str() {
                        display_name_map.insert(subj, s.to_owned());
                    }
                }
            }
        }
        // Collect user-visible tags (not system tags)
        if !is_system_tag_key(&tag.key) {
            user_tags_map
                .entry(tag.subject.display_lossy())
                .or_default()
                .push((
                    tag.key.display_lossy(),
                    tag.value.as_ref().map(|v| v.display_lossy()),
                ));
        }
    }

    // First pass: collect all tags and build hash→primary-name map
    let mut hash_to_primary: HashMap<String, String> = HashMap::new();
    let mut raw_tags: Vec<(String, String, u64)> = Vec::new();

    let Ok(mut tags) = state.store.tags().list().await else {
        return files;
    };
    while let Some(Ok(tag_info)) = tags.next().await {
        let name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();

        // Skip internal tags (e.g., .meta)
        if crate::tags::is_internal_tag(&name) {
            continue;
        }

        let hash = tag_info.hash.to_string();
        let size = 0;

        let (kind, _) = classify_tag(&name);
        if kind == FileKind::Primary {
            hash_to_primary.insert(hash.clone(), name.clone());
        }

        raw_tags.push((name, hash, size));
    }

    // Second pass: build FileInfo with parent_name resolution and metadata dates
    for (name, hash, size) in raw_tags {
        let is_deleted = deleted_set.contains(&name);
        let (kind, timestamp) = classify_tag(&name);

        let parent_name = if kind == FileKind::Primary {
            None
        } else {
            hash_to_primary.get(&hash).cloned()
        };

        let created_at = created_map.get(&name).copied();
        let modified_at = modified_map.get(&name).copied();
        let tags = user_tags_map.remove(&name).unwrap_or_default();
        let display_name = display_name_map.get(&name).cloned();

        files.push(FileInfo {
            name,
            display_name,
            hash,
            size,
            kind,
            parent_name,
            timestamp,
            created_at,
            modified_at,
            tags,
            is_deleted,
        });
    }

    // Deduplicate files with the same hash, display name, and user tags
    {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        files.retain(|f| {
            let dedup_key = format!(
                "{}|{}|{:?}",
                f.hash,
                f.display_name.as_deref().unwrap_or(&f.name),
                f.tags
            );
            seen.insert(dedup_key)
        });
    }

    // Sort: primary first (alphabetically), then auto (by timestamp desc), then archive (by timestamp desc)
    files.sort_by(|a, b| {
        let kind_order = |k: &FileKind| -> u8 {
            match k {
                FileKind::Primary => 0,
                FileKind::Auto => 1,
                FileKind::Archive => 2,
            }
        };
        kind_order(&a.kind).cmp(&kind_order(&b.kind)).then_with(|| {
            if a.kind == FileKind::Primary {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            } else {
                // Newer timestamps first for auto/archive
                b.timestamp.unwrap_or(0).cmp(&a.timestamp.unwrap_or(0))
            }
        })
    });

    files
}

/// Get a paginated, optionally filtered file list.
///
/// When `search` is provided, matches against:
/// 1. Filename (case-insensitive substring)
/// 2. Tag keys/values for the file in the global namespace
///
/// When `show_deleted` is false (default), soft-deleted files are hidden.
async fn get_file_list_page(state: &AppState, query: &FileListQuery) -> FileListPage {
    let mut files = get_file_list(state).await;

    let show_deleted = query.show_deleted.unwrap_or(false);

    // Filter out deleted files unless show_deleted is set
    if !show_deleted {
        files.retain(|f| !f.is_deleted);
    }

    // Apply search filter
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    if let Some(ref needle) = search {
        let needle_lower = needle.to_lowercase();

        // Collect subjects that match via tags (key or value contains needle)
        let mut tag_matches: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Ok(all_tags) = state.tag_store.list_all(&state.tag_store.global).await {
            for tag in &all_tags {
                // Skip internal/system tags for search
                if tag.key.starts_with(b"archive.") {
                    continue;
                }
                if tag
                    .key
                    .display_lossy()
                    .to_lowercase()
                    .contains(&needle_lower)
                    || tag
                        .value
                        .as_ref()
                        .is_some_and(|v| v.display_lossy().to_lowercase().contains(&needle_lower))
                {
                    tag_matches.insert(tag.subject.display_lossy());
                }
            }
        }

        files.retain(|f| {
            f.name.to_lowercase().contains(&needle_lower) || tag_matches.contains(&f.name)
        });
    }

    let total = files.len();
    let per_page = query.per_page.unwrap_or(DEFAULT_PER_PAGE).max(1);
    let page = query.page.unwrap_or(1).max(1);
    let start = (page - 1) * per_page;

    let page_files = if start < total {
        files[start..(start + per_page).min(total)].to_vec()
    } else {
        Vec::new()
    };

    FileListPage {
        files: page_files,
        total,
        page,
        per_page,
        search,
        show_deleted,
    }
}

/// Get the human-readable name for a hash, preferring primary file names.
///
/// When multiple tags share the same hash, returns the first primary
/// (non-auto, non-archive) name found. Falls back to any matching name.
async fn get_file_name(store: &iroh_blobs::api::Store, hash: &str) -> Option<String> {
    use futures_lite::StreamExt;

    // Parse hash
    let hash: iroh_blobs::Hash = hash.parse().ok()?;

    // Find tag with this hash, preferring primary names
    let mut tags = store.tags().list().await.ok()?;
    let mut first_match: Option<String> = None;
    while let Some(Ok(tag_info)) = tags.next().await {
        if tag_info.hash == hash {
            let name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
            let (kind, _) = classify_tag(&name);
            if kind == FileKind::Primary {
                return Some(name);
            }
            if first_match.is_none() {
                first_match = Some(name);
            }
        }
    }

    first_match
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

/// Request body for renaming a file.
#[derive(Debug, Deserialize)]
struct RenameRequest {
    /// Current file name.
    name: String,
    /// New file name.
    new_name: String,
    /// Whether to archive the old name as `{name}.archive.{timestamp}`.
    archive: bool,
}

/// Response from renaming a file.
#[derive(Debug, Serialize)]
struct RenameResponse {
    /// New file name.
    name: String,
    /// Content hash (unchanged).
    hash: String,
    /// Archive tag for the original name (if archived).
    archived_original: Option<String>,
    /// Archive tag for the replaced file (if target name existed).
    archived_replaced: Option<String>,
}

/// Request body for copying a file.
#[derive(Debug, Deserialize)]
struct CopyRequest {
    /// Source file name to copy from.
    name: String,
    /// New file name for the copy.
    new_name: String,
}

/// Response from copying a file.
#[derive(Debug, Serialize)]
struct CopyResponse {
    /// New file name.
    name: String,
    /// Content hash (same as source).
    hash: String,
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
///
/// Rate-limited: rejects saves within the cooldown period (default 5s) per file.
async fn save_handler(State(state): State<AppState>, Json(req): Json<SaveRequest>) -> Response {
    tracing::info!(
        "[routes] save_handler: doc_id={}, name={}",
        req.doc_id,
        req.name
    );

    // Check save rate limit
    if let Err(remaining) = state.save_limiter.check(&req.name).await {
        let secs = remaining.as_secs_f64().ceil() as u64;
        tracing::info!(
            "[routes] Save rate limited for '{}': {}s remaining",
            req.name,
            secs
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            format!("Save rate limited. Try again in {secs}s."),
        )
            .into_response();
    }

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

    // Update metadata tags: set created (if first save) and modified
    let now_str = crate::tags::now_unix().to_string();
    if let Err(err) = state
        .tag_store
        .set_if_absent(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"created",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set created tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"modified",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set modified tag: {:#}", err);
    }
    if let Some(ref archive) = archive_name
        && let Err(err) = state
            .tag_store
            .set_tag(
                &state.tag_store.global,
                req.name.as_bytes(),
                b"archive.save",
                Some(archive.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set archive tag: {}", err);
    }

    tracing::info!(
        "[routes] File saved: name={}, hash={}, archive={:?}",
        req.name,
        new_hash_str,
        archive_name
    );

    // Notify collab clients editing the old hash about the new version
    state
        .collab
        .notify_new_version(&req.doc_id, &new_hash_str, &req.name)
        .await;

    // Record successful save for rate limiting
    state.save_limiter.record(&req.name).await;

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

    // Set metadata tags: created + modified
    let now_str = crate::tags::now_unix().to_string();
    if let Err(err) = state
        .tag_store
        .set_if_absent(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"created",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set created tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"modified",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set modified tag: {:#}", err);
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

/// Rename a file by changing its tag name.
///
/// If the target name already exists, archives it first under
/// `{new_name}.archive.{timestamp}`. Optionally archives the original name
/// as `{name}.archive.{timestamp}` (when `archive` is true) or simply
/// deletes it (when `archive` is false).
async fn rename_handler(State(state): State<AppState>, Json(req): Json<RenameRequest>) -> Response {
    tracing::info!(
        "[routes] rename_handler: name={}, new_name={}, archive={}",
        req.name,
        req.new_name,
        req.archive
    );

    // Validate inputs
    let new_name = req.new_name.trim().to_owned();
    if new_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "New name cannot be empty").into_response();
    }
    if req.name == new_name {
        return (
            StatusCode::BAD_REQUEST,
            "New name must differ from current name",
        )
            .into_response();
    }

    // Look up the hash for the current name
    let tag_info = match state.store.tags().get(&req.name).await {
        Ok(Some(info)) => info,
        Ok(None) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(err) => {
            tracing::error!("[routes] Failed to look up tag: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to look up file").into_response();
        }
    };
    let hash = tag_info.hash;
    let hash_str = hash.to_string();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // If target name already exists, archive the existing file it points to
    let archived_replaced = match state.store.tags().get(&new_name).await {
        Ok(Some(existing)) => {
            let archive_name = format!("{new_name}.archive.{timestamp}");
            match state.store.tags().set(&archive_name, existing.hash).await {
                Ok(()) => {
                    tracing::info!(
                        "[routes] Archived replaced file: {} -> {}",
                        new_name,
                        archive_name
                    );
                    Some(archive_name)
                }
                Err(err) => {
                    tracing::error!("[routes] Failed to archive replaced file: {}", err);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to archive replaced file",
                    )
                        .into_response();
                }
            }
        }
        _ => None,
    };

    // Set the new name tag to point to our hash
    if let Err(err) = state.store.tags().set(&new_name, hash).await {
        tracing::error!("[routes] Failed to set new name tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to rename file").into_response();
    }

    // Handle the old name: archive or delete
    let archived_original = if req.archive {
        let archive_name = format!("{}.archive.{}", req.name, timestamp);
        match state.store.tags().set(&archive_name, hash).await {
            Ok(()) => {
                tracing::info!(
                    "[routes] Archived original: {} -> {}",
                    req.name,
                    archive_name
                );
                Some(archive_name)
            }
            Err(err) => {
                tracing::error!("[routes] Failed to archive original: {}", err);
                None
            }
        }
    } else {
        None
    };

    // Delete the original tag (it has been renamed)
    if let Err(err) = state.store.tags().delete(&req.name).await {
        tracing::error!("[routes] Failed to delete original tag: {}", err);
    }

    // Update metadata tags: transfer from old name to new, record archives
    if let Err(err) = state
        .tag_store
        .transfer_all_tags(
            &state.tag_store.global,
            req.name.as_bytes(),
            new_name.as_bytes(),
        )
        .await
    {
        tracing::warn!("[routes] Failed to transfer tags: {}", err);
    }
    if let Some(ref archive) = archived_original
        && let Err(err) = state
            .tag_store
            .set_tag(
                &state.tag_store.global,
                new_name.as_bytes(),
                b"archive.rename",
                Some(archive.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set archive.rename tag: {}", err);
    }
    if let Some(ref archive) = archived_replaced
        && let Err(err) = state
            .tag_store
            .set_tag(
                &state.tag_store.global,
                new_name.as_bytes(),
                b"archive.replace",
                Some(archive.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set archive.replace tag: {}", err);
    }

    tracing::info!(
        "[routes] File renamed: {} -> {}, hash={}, archived_original={:?}, archived_replaced={:?}",
        req.name,
        new_name,
        hash_str,
        archived_original,
        archived_replaced
    );

    Json(RenameResponse {
        name: new_name,
        hash: hash_str,
        archived_original,
        archived_replaced,
    })
    .into_response()
}

/// Copy a file by creating a new tag pointing to the same content hash.
///
/// If the destination name already exists, the existing file at that name
/// is archived as `{new_name}.archive.{timestamp}` before the copy.
async fn copy_handler(State(state): State<AppState>, Json(req): Json<CopyRequest>) -> Response {
    tracing::info!(
        "[routes] copy_handler: name={}, new_name={}",
        req.name,
        req.new_name
    );

    let new_name = req.new_name.trim().to_owned();
    if new_name.is_empty() {
        return (StatusCode::BAD_REQUEST, "New name cannot be empty").into_response();
    }
    if req.name == new_name {
        return (
            StatusCode::BAD_REQUEST,
            "New name must differ from current name",
        )
            .into_response();
    }

    // Look up the hash for the source file
    let tag_info = match state.store.tags().get(&req.name).await {
        Ok(Some(info)) => info,
        Ok(None) => return (StatusCode::NOT_FOUND, "Source file not found").into_response(),
        Err(err) => {
            tracing::error!("[routes] Failed to look up source tag: {}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to look up source file",
            )
                .into_response();
        }
    };
    let hash = tag_info.hash;
    let hash_str = hash.to_string();

    // If target name already exists, archive the existing file
    if let Ok(Some(existing)) = state.store.tags().get(&new_name).await {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let archive_name = format!("{new_name}.archive.{timestamp}");
        if let Err(err) = state.store.tags().set(&archive_name, existing.hash).await {
            tracing::error!("[routes] Failed to archive replaced file: {}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to archive replaced file",
            )
                .into_response();
        }
        tracing::info!(
            "[routes] Archived replaced file: {} -> {}",
            new_name,
            archive_name
        );
    }

    // Set the new tag pointing to the same hash
    if let Err(err) = state.store.tags().set(&new_name, hash).await {
        tracing::error!("[routes] Failed to set copy tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to copy file").into_response();
    }

    // Copy metadata tags from source to destination
    if let Err(err) = state
        .tag_store
        .copy_all_tags(
            &state.tag_store.global,
            req.name.as_bytes(),
            new_name.as_bytes(),
        )
        .await
    {
        tracing::warn!("[routes] Failed to copy metadata tags: {}", err);
    }

    tracing::info!(
        "[routes] File copied: {} -> {}, hash={}",
        req.name,
        new_name,
        hash_str
    );

    Json(CopyResponse {
        name: new_name,
        hash: hash_str,
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

/// Request body for soft-deleting or hard-deleting a file.
#[derive(Debug, Deserialize)]
struct DeleteRequest {
    /// File name to delete.
    name: String,
}

/// Response from deleting a file.
#[derive(Debug, Serialize)]
struct DeleteResponse {
    /// File name that was deleted.
    name: String,
    /// Whether this was a soft or hard delete.
    hard: bool,
}

/// Soft-delete a file by adding a "deleted" tag.
///
/// The file remains in the store and can be restored by removing the "deleted" tag.
/// Soft-deleted files are hidden from the file list.
async fn delete_handler(State(state): State<AppState>, Json(req): Json<DeleteRequest>) -> Response {
    tracing::info!("[routes] delete_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Verify the file exists
    match state.store.tags().get(&req.name).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(err) => {
            tracing::error!("[routes] Failed to look up tag: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to look up file").into_response();
        }
    }

    // Add "deleted" tag with timestamp
    let now_str = crate::tags::now_unix().to_string();
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"deleted",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::error!("[routes] Failed to set deleted tag: {}", err);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to mark file as deleted",
        )
            .into_response();
    }

    tracing::info!("[routes] File soft-deleted: name={}", req.name);

    Json(DeleteResponse {
        name: req.name,
        hard: false,
    })
    .into_response()
}

/// Restore a soft-deleted file by removing the "deleted" tag.
async fn restore_handler(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Response {
    tracing::info!("[routes] restore_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Remove the "deleted" tag (all values for this key)
    if let Err(err) = state
        .tag_store
        .del_by_key(&state.tag_store.global, req.name.as_bytes(), b"deleted")
        .await
    {
        tracing::error!("[routes] Failed to remove deleted tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to restore file").into_response();
    }

    tracing::info!("[routes] File restored: name={}", req.name);

    Json(DeleteResponse {
        name: req.name,
        hard: false,
    })
    .into_response()
}

/// Hard-delete a file (admin only).
///
/// Removes the blob tag, all archive tags for this file, and all metadata tags.
/// The blob data itself may be garbage-collected later by iroh-blobs.
async fn hard_delete_handler(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Response {
    tracing::info!("[routes] hard_delete_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Delete the blob tag
    if let Err(err) = state.store.tags().delete(&req.name).await {
        tracing::error!("[routes] Failed to delete tag: {}", err);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to delete file tag",
        )
            .into_response();
    }

    // Delete all metadata tags for this subject
    if let Err(err) = state
        .tag_store
        .del_all_tags(&state.tag_store.global, req.name.as_bytes())
        .await
    {
        tracing::warn!("[routes] Failed to delete metadata tags: {}", err);
    }

    // Delete any archive tags that reference this file
    // (archive tags are named like "name.archive.timestamp")
    if let Ok(mut tags) = state.store.tags().list().await {
        use futures_lite::StreamExt;
        let prefix = format!("{}.archive.", req.name);
        while let Some(Ok(tag_info)) = tags.next().await {
            let tag_name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
            if tag_name.starts_with(&prefix)
                && let Err(err) = state.store.tags().delete(&tag_name).await
            {
                tracing::warn!(
                    "[routes] Failed to delete archive tag {}: {}",
                    tag_name,
                    err
                );
            }
        }
    }

    tracing::info!("[routes] File hard-deleted: name={}", req.name);

    Json(DeleteResponse {
        name: req.name,
        hard: true,
    })
    .into_response()
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_tag_primary() {
        let (kind, ts) = classify_tag("my-document.md");
        assert_eq!(kind, FileKind::Primary);
        assert!(ts.is_none());
    }

    #[test]
    fn test_classify_tag_auto() {
        let (kind, ts) = classify_tag("auto-2026-03-12T06:42:30.015Z");
        assert_eq!(kind, FileKind::Auto);
        assert!(ts.is_some());
    }

    #[test]
    fn test_classify_tag_archive() {
        let (kind, ts) = classify_tag("notes.md.archive.1711000000");
        assert_eq!(kind, FileKind::Archive);
        assert_eq!(ts.unwrap(), 1_711_000_000);
    }

    #[test]
    fn test_classify_tag_archive_no_timestamp() {
        // Even without a valid timestamp, .archive. pattern is recognized
        let (kind, ts) = classify_tag("notes.md.archive.notanumber");
        assert_eq!(kind, FileKind::Archive);
        assert!(ts.is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_valid() {
        // 2026-03-12T06:42:30.015Z → should produce a valid timestamp
        let ts = parse_iso_timestamp("auto-2026-03-12T06:42:30.015Z");
        assert!(ts.is_some());
        let t = ts.unwrap();
        // 2026-03-12 is after 2025-01-01 (approx 1735689600)
        assert!(t > 1_735_689_600);
    }

    #[test]
    fn test_parse_iso_timestamp_invalid() {
        assert!(parse_iso_timestamp("auto-not-a-date").is_none());
        assert!(parse_iso_timestamp("auto-").is_none());
        assert!(parse_iso_timestamp("noprefix").is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_epoch() {
        // 1970-01-01T00:00:00.000Z should be 0
        let ts = parse_iso_timestamp("auto-1970-01-01T00:00:00.000Z");
        assert_eq!(ts, Some(0));
    }

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2000)); // divisible by 400
        assert!(!is_leap_year(1900)); // divisible by 100 but not 400
        assert!(is_leap_year(2024)); // divisible by 4
        assert!(!is_leap_year(2023)); // not divisible by 4
    }

    #[test]
    fn test_html_escape_inline() {
        assert_eq!(html_escape_inline("hello"), "hello");
        assert_eq!(html_escape_inline("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape_inline("a&b"), "a&amp;b");
        assert_eq!(html_escape_inline("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_rename_archive_naming_pattern() {
        // The rename handler creates archives with format: {name}.archive.{timestamp}
        // This must be recognized by classify_tag as FileKind::Archive
        let timestamp = 1_711_000_000u64;
        let original_archive = format!("old-name.md.archive.{timestamp}");
        let (kind, ts) = classify_tag(&original_archive);
        assert_eq!(kind, FileKind::Archive);
        assert_eq!(ts, Some(timestamp));

        // Same pattern for the "replaced" file archive
        let replaced_archive = format!("new-name.md.archive.{timestamp}");
        let (kind2, ts2) = classify_tag(&replaced_archive);
        assert_eq!(kind2, FileKind::Archive);
        assert_eq!(ts2, Some(timestamp));
    }

    #[test]
    fn test_classify_tag_dotted_filenames() {
        // Files with dots in their name should be Primary, not Archive
        let (kind, _) = classify_tag("my.file.name.txt");
        assert_eq!(kind, FileKind::Primary);

        let (kind, _) = classify_tag("v1.2.3.tar.gz");
        assert_eq!(kind, FileKind::Primary);
    }

    #[test]
    fn test_classify_tag_archive_preserves_original_name() {
        // Given an archive tag, we can extract the original file name
        let tag = "README.md.archive.1711000000";
        let (kind, _) = classify_tag(tag);
        assert_eq!(kind, FileKind::Archive);
        // The part before ".archive." is the original file name
        let parts: Vec<&str> = tag.splitn(2, ".archive.").collect();
        assert_eq!(parts[0], "README.md");
    }

    // ========================================================================
    // CopyRequest / CopyResponse serde tests
    // ========================================================================

    #[test]
    fn test_copy_request_deserialization() {
        let json = r#"{"name":"source.md","new_name":"dest.md"}"#;
        let req: CopyRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "source.md");
        assert_eq!(req.new_name, "dest.md");
    }

    #[test]
    fn test_copy_request_missing_field() {
        let json = r#"{"name":"source.md"}"#;
        let result: Result<CopyRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_copy_response_serialization() {
        let resp = CopyResponse {
            name: "dest.md".to_owned(),
            hash: "abc123def".to_owned(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"name\":\"dest.md\""));
        assert!(json.contains("\"hash\":\"abc123def\""));
    }

    #[test]
    fn test_copy_request_with_special_chars() {
        let json = r#"{"name":"file with spaces.txt","new_name":"新しいファイル.txt"}"#;
        let req: CopyRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "file with spaces.txt");
        assert_eq!(req.new_name, "新しいファイル.txt");
    }

    // ========================================================================
    // Existing rename tests
    // ========================================================================

    #[test]
    fn test_rename_request_deserialization() {
        let json = r#"{"name":"old.md","new_name":"new.md","archive":true}"#;
        let req: RenameRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "old.md");
        assert_eq!(req.new_name, "new.md");
        assert!(req.archive);
    }

    #[test]
    fn test_rename_response_serialization() {
        let resp = RenameResponse {
            name: "new.md".to_owned(),
            hash: "abc123".to_owned(),
            archived_original: Some("old.md.archive.1711000000".to_owned()),
            archived_replaced: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"name\":\"new.md\""));
        assert!(json.contains("\"archived_original\":\"old.md.archive.1711000000\""));
        assert!(json.contains("\"archived_replaced\":null"));
    }

    #[test]
    fn test_rename_response_with_both_archives() {
        let resp = RenameResponse {
            name: "target.md".to_owned(),
            hash: "def456".to_owned(),
            archived_original: Some("source.md.archive.100".to_owned()),
            archived_replaced: Some("target.md.archive.100".to_owned()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"archived_original\":\"source.md.archive.100\""));
        assert!(json.contains("\"archived_replaced\":\"target.md.archive.100\""));
    }
}
