//! HTTP route handlers for the web interface.
//!
//! Defines all the Axum routes and their handlers for serving the web UI.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
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
use crate::fileops::{FileOpError, FileOps, claim_archive_name};

/// Default number of files per page.
const DEFAULT_PER_PAGE: usize = 50;

/// Maximum number of files per page.
const MAX_PER_PAGE: usize = 200;

/// Maximum size of a single uploaded image file, in bytes.
///
/// This is the *file* limit the web UI advertises and enforces client-side
/// (`MAX_IMAGE_SIZE` in `web/src/image-upload.ts`) — the two constants must
/// stay equal. It is NOT the request-body limit for the upload route: a
/// multipart body carrying a file of exactly this size is larger than the
/// file itself (boundary, part headers, filename), so the route's body
/// limit must exceed it by [`UPLOAD_MULTIPART_ALLOWANCE`] or a max-size
/// image the UI accepted is rejected by the server (#716).
pub(crate) const MAX_IMAGE_SIZE: usize = 10 * 1024 * 1024;

/// Headroom the `/api/upload` body limit grants above [`MAX_IMAGE_SIZE`]
/// for the multipart envelope (boundary lines, `Content-Disposition` with
/// the filename, part `Content-Type`, closing boundary). Measured overhead
/// for a typical filename is ~170 bytes and grows only with filename
/// length, so 1 MiB is generous without meaningfully weakening the bound.
pub(crate) const UPLOAD_MULTIPART_ALLOWANCE: usize = 1024 * 1024;

/// Request-body limit for `/api/upload` only: a max-size image file plus
/// its multipart envelope. Every other route keeps [`DEFAULT_BODY_LIMIT`].
pub(crate) const UPLOAD_BODY_LIMIT: usize = MAX_IMAGE_SIZE + UPLOAD_MULTIPART_ALLOWANCE;

/// Request-body limit for the router as a whole (all non-upload routes).
pub(crate) const DEFAULT_BODY_LIMIT: usize = 10 * 1024 * 1024;

/// Image MIME types allowed for upload.
const ALLOWED_IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    "image/bmp",
    "image/x-icon",
];

/// Map an image MIME type to a file extension.
fn mime_to_extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/x-icon" => "ico",
        _ => "bin",
    }
}

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
#[allow(dead_code)] // not every renderer shows every field
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
        // Only an auto-backup when the remainder actually parses as an
        // ISO date (format: 2026-03-12T06:42:30.015Z). An ordinary file
        // that merely starts with "auto-" (auto-notes.txt) is Primary.
        if let Some(ts) = parse_iso_timestamp(iso_str) {
            return (FileKind::Auto, Some(ts));
        }
    }

    // Check for archive: "{name}.archive.{unix_timestamp}"
    // Only an archive when the suffix actually parses as a unix
    // timestamp (#2793) — the same rule #809 applied to the "auto-"
    // prefix above. An ordinary file that merely contains ".archive."
    // (notes.archive.txt) is Primary. No producer mints a non-numeric
    // suffix: the archive claim path always formats a u64.
    if let Some(pos) = name.rfind(".archive.") {
        let ts_str = &name[pos + ".archive.".len()..];
        if let Ok(ts) = ts_str.parse::<u64>() {
            return (FileKind::Archive, Some(ts));
        }
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

    // Reject anything the arithmetic below cannot represent faithfully
    // instead of rolling it over into a different date (#756). Generated
    // names carry a 4-digit year; the u64 result cannot represent dates
    // before the epoch, and a leap second (second 60) is not modelled.
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
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > month_days[(month - 1) as usize]
        || hour > 23
        || min > 59
        || sec > 59
    {
        return None;
    }

    // Simple days-since-epoch calculation (not accounting for leap seconds)
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    #[allow(clippy::cast_possible_truncation)] // month is 1..=12, fits in usize
    for m in 0..(month.saturating_sub(1) as usize) {
        days += month_days[m];
    }
    days += day - 1;

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

/// Check if a year is a leap year.
const fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Create the main router with all web routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Page routes (return full HTML pages)
        .route("/", get(index_handler))
        .route("/files", get(files_page_handler))
        .route("/world", get(world_page_handler))
        .route("/settings", get(settings_handler))
        .route("/peers", get(peers_handler))
        .route("/groups", get(super::groups::groups_page_handler))
        .route("/edit/{*name}", get(file_by_name_handler))
        .route("/hash/{hash}", get(hash_redirect_handler))
        .route("/view/{*name}", get(view_handler))
        // Blob route (serves raw file content)
        .route("/blob/{hash}", get(blob_handler))
        // Partial routes (return HTML fragments for SPA navigation)
        .route("/api/files", get(files_list_handler))
        .route("/api/home/feed", get(super::home::feed_handler))
        .route("/api/peers", get(peers_partial_handler))
        // File management API routes
        .route("/api/save", post(save_handler))
        .route("/api/new", post(new_file_handler))
        .route("/api/rename", post(rename_handler))
        .route("/api/copy", post(copy_handler))
        .route("/api/download", post(download_handler))
        .route("/api/delete", post(delete_handler))
        .route("/api/restore", post(restore_handler))
        .route("/api/hard-delete", post(hard_delete_handler))
        // Image upload API — route-scoped body limit so a file of exactly
        // MAX_IMAGE_SIZE plus its multipart envelope fits (#716); the
        // route layer overrides the router-wide limit below for this route.
        .route(
            "/api/upload",
            post(upload_handler).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        // Image listing API
        .route("/api/images", get(images_list_handler))
        // Tag REST API
        .route(
            "/api/tags",
            get(super::tags_ws::get_tags_handler)
                .post(super::tags_ws::set_tag_handler)
                .delete(super::tags_ws::del_tag_handler),
        )
        // Tag search endpoint with structured query syntax
        .route("/api/tags/search", get(super::tags_ws::search_tags_handler))
        // Identity API routes
        .route("/api/identity/register", post(identity_register_handler))
        .route("/api/identity/me", get(identity_me_handler))
        .route("/api/identity/name", post(identity_update_name_handler))
        // WebSocket for collaboration
        .route("/ws/collab/{doc_id}", get(super::collab::ws_collab_handler))
        // WebSocket for live tag updates
        .route("/ws/tags", get(super::tags_ws::ws_tags_handler))
        // Static assets
        .route("/assets/{*path}", get(assets_handler))
        // Optional authoritative in-memory world session bridge
        .merge(
            super::world_ws::world_routes().with_state(super::world_ws::WorldWebState {
                hub: state.world.clone(),
                cookie_secure: state.cookie_secure,
            }),
        )
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .with_state(state)
}

async fn world_page_handler(State(state): State<AppState>) -> Response {
    if state.world.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Html(super::templates::render_world_page(&state.assets)).into_response()
}

/// Check if this is a partial request (SPA navigation, returns fragment not full page).
fn is_partial_request(headers: &HeaderMap) -> bool {
    let is_partial = headers.contains_key("X-Partial-Request");
    tracing::debug!("[routes] is_partial_request: {}", is_partial);
    is_partial
}

/// Index page handler - discovery home.
async fn index_handler(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let worlds = state.world.as_ref().map_or_else(Vec::new, |hub| hub.list());
    let default_world = state
        .world
        .as_ref()
        .map_or("lobby", |hub| hub.default_world());
    let content = super::home::render_home(&worlds, default_world, state.world.is_some());
    if is_partial_request(&headers) {
        Html(render_main_page_wrapper(&content))
    } else {
        Html(render_page("Home", &content, "", &state.assets))
    }
}

/// File browser page retained separately from home/discovery.
async fn files_page_handler(
    State(state): State<AppState>,
    Query(query): Query<FileListQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let page = get_file_list_page(&state, &query).await;
    let content = render_file_list(&page);
    if is_partial_request(&headers) {
        // Partial request - return wrapped content with header/footer
        Html(render_main_page_wrapper(&content))
    } else {
        Html(render_page("Files", &content, "", &state.assets))
    }
}

/// Settings page handler.
async fn settings_handler(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let content = render_settings(&state.node_id);
    if is_partial_request(&headers) {
        // Partial request - return wrapped content with header/footer
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
    if is_partial_request(&headers) {
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

/// Look up a content hash for a given tag name.
///
/// Iterates all tags in the store to find one matching `name` and returns its hash.
pub(crate) async fn get_hash_for_name(
    store: &iroh_blobs::api::Store,
    name: &str,
) -> Option<String> {
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

/// Handler for `/edit/*name` — resolve a file by tag name and render the editor.
///
/// Looks up the content hash for the given name, then delegates to
/// edit/view logic based on content mode detection.
async fn file_by_name_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    tracing::info!("[routes] file_by_name_handler called for name: {}", name);

    let is_partial = is_partial_request(&headers);

    // Look up the hash for this tag name
    let Some(hash) = get_hash_for_name(&state.store, &name).await else {
        let escaped = html_escape_inline(&name);
        let err_html = format!("<p>File not found: {escaped}</p>");
        if is_partial {
            return Html(render_editor("", &name, &err_html, ""));
        }
        return Html(render_editor_page("", &name, &err_html, "", &state.assets));
    };

    // Get file content bytes from store
    let content_result = get_file_bytes(&state.store, &hash).await;

    match content_result {
        Ok(bytes) => {
            let mode = detect_mode_with_content(&name, &bytes);

            match mode {
                ContentMode::Media(media_type) => {
                    let viewer_html = render_media_viewer(&hash, &name, media_type);
                    if is_partial {
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
                    if is_partial {
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
                    if is_partial {
                        Html(render_editor(&name, &name, &content, &hash))
                    } else {
                        Html(render_editor_page(
                            &name,
                            &name,
                            &content,
                            &hash,
                            &state.assets,
                        ))
                    }
                }
            }
        }
        Err(err_msg) => {
            if is_partial {
                Html(render_editor(&name, &name, &err_msg, &hash))
            } else {
                Html(render_editor_page(
                    &name,
                    &name,
                    &err_msg,
                    &hash,
                    &state.assets,
                ))
            }
        }
    }
}

/// Handler for `/hash/:hash` — redirect hash-based URLs to name-based URLs.
///
/// Resolves the hash to a file name and issues a 302 redirect to `/edit/{name}`.
/// Returns 404 if no file is found with that hash.
async fn hash_redirect_handler(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> impl IntoResponse {
    tracing::info!("[routes] hash_redirect_handler called for hash: {}", hash);

    if let Some(name) = get_file_name(&state.store, &hash).await {
        let encoded_name = urlencoding::encode(&name);
        let redirect_url = format!("/edit/{encoded_name}");
        tracing::info!("[routes] Redirecting /hash/{} -> {}", hash, redirect_url);
        Redirect::to(&redirect_url).into_response()
    } else {
        tracing::warn!(
            "[routes] hash_redirect_handler: no file found for hash {}",
            hash
        );
        (StatusCode::NOT_FOUND, "File not found for hash").into_response()
    }
}

/// Handler for `/view/*name` — read-only view stub.
///
/// Currently redirects to the editor at `/edit/{name}`.
/// Will be enhanced to provide a proper read-only view in the future.
async fn view_handler(Path(name): Path<String>) -> impl IntoResponse {
    let encoded_name = urlencoding::encode(&name);
    let redirect_url = format!("/edit/{encoded_name}");
    Redirect::to(&redirect_url)
}

/// Content type a blob may be served with, and whether it may be
/// served inline (#908).
///
/// The type implied by the filename (via [`get_content_type`]) is kept
/// only when it is a passive type a browser cannot turn into script
/// execution in this origin when combined with the sandboxing CSP and
/// `nosniff` headers [`blob_handler`] always sends: images (including
/// SVG, which the CSP sandbox neutralises on top-level navigation),
/// video, audio, PDF, and plain/markdown text. Anything else — notably
/// `text/html`, CSS, JavaScript, JSON and XML — is downgraded to
/// `application/octet-stream` and marked non-inline so the caller gets
/// a download, never active content.
fn safe_blob_content_type(filename: &str) -> (&'static str, bool) {
    let content_type = get_content_type(filename);
    let base_type = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim();
    let inline_safe = matches!(
        base_type,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "image/x-icon"
            | "image/bmp"
            | "video/mp4"
            | "video/webm"
            | "video/ogg"
            | "video/quicktime"
            | "video/x-msvideo"
            | "audio/mpeg"
            | "audio/wav"
            | "audio/ogg"
            | "audio/flac"
            | "audio/aac"
            | "audio/mp4"
            | "application/pdf"
            | "text/plain"
            | "text/markdown"
    );
    if inline_safe {
        (content_type, true)
    } else {
        ("application/octet-stream", false)
    }
}

/// Reconcile an uploaded file's name with its validated content type
/// (#908): a name whose extension disagrees with the type the upload
/// was validated as gets that extension replaced (or appended when it
/// has none), so a stored name cannot masquerade as a different type —
/// e.g. a PNG uploaded as `evil.html` is stored as `evil.png`.
/// `image/jpeg` accepts both `jpg` and `jpeg`.
fn reconcile_upload_filename(name: &str, ext: &str) -> String {
    let acceptable: &[&str] = if ext == "jpg" {
        &["jpg", "jpeg"]
    } else {
        &[ext]
    };
    let path = std::path::Path::new(name);
    let current = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    if current.as_deref().is_some_and(|c| acceptable.contains(&c)) {
        return name.to_owned();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    // Keep any directory-free parent prefix out: upload names are plain
    // file names, and `file_stem` already drops the old extension.
    format!("{stem}.{ext}")
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

    // #908: the filename — including the caller-supplied `?filename=`
    // parameter — must never select an *active* content type. Only a
    // fixed allowlist of passive media/text types is served inline;
    // everything else (html, css, js, json, xml, unknown) is served as
    // an attachment download. Every blob response also carries `nosniff`
    // and a sandboxing CSP so even an allowlisted type that can carry
    // script (SVG) cannot execute in this origin on a top-level visit,
    // while `<img>` embedding of images keeps working.
    let (content_type, inline) = safe_blob_content_type(&filename);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox",
        );
    if !inline {
        builder = builder.header(header::CONTENT_DISPOSITION, "attachment");
    }
    builder
        .body(Body::from(bytes.to_vec()))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to build response",
            )
                .into_response()
        })
}

/// API handler for file list (partial fragment).
async fn files_list_handler(
    State(state): State<AppState>,
    Query(query): Query<FileListQuery>,
) -> impl IntoResponse {
    let page = get_file_list_page(&state, &query).await;
    // Return just the inner content (list + pagination) for partial replacement
    Html(render_file_list_content(&page))
}

/// API handler for peers partial (used by auto-refresh).
///
/// Returns just the peers content fragment for replacing #peers-content.
async fn peers_partial_handler(State(state): State<AppState>) -> impl IntoResponse {
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

    Html(render_peers(&peers_data))
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

/// Size in bytes of a complete blob; `0` if it is missing or only partial.
async fn blob_size(store: &iroh_blobs::api::Store, hash: iroh_blobs::Hash) -> u64 {
    match store.blobs().status(hash).await {
        Ok(iroh_blobs::api::blobs::BlobStatus::Complete { size }) => size,
        _ => 0,
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
            if !display_name_map.contains_key(&subj)
                && let Some(ref v) = tag.value
                && let Some(s) = v.as_str()
            {
                display_name_map.insert(subj, s.to_owned());
            }
        }
        // Collect user-visible tags (not system tags)
        if !is_system_tag_key(&tag.key) {
            user_tags_map
                .entry(tag.subject.display_lossy())
                .or_default()
                .push((
                    tag.key.display_lossy(),
                    tag.value
                        .as_ref()
                        .map(super::super::tags::TagValue::display_lossy),
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
        let size = blob_size(&state.store, tag_info.hash).await;

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
/// Generic helper to paginate a slice of items safely.
/// Returns (paginated_items, clamped_page, clamped_per_page).
fn paginate<T: Clone>(
    items: &[T],
    page: Option<usize>,
    per_page: Option<usize>,
) -> (Vec<T>, usize, usize) {
    let total = items.len();
    let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let page = page.unwrap_or(1).max(1);

    let start = page.saturating_sub(1).saturating_mul(per_page);
    let end = start.saturating_add(per_page).min(total);

    let page_items = if start < total {
        items[start..end].to_vec()
    } else {
        Vec::new()
    };

    (page_items, page, per_page)
}

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
    let (page_files, page, per_page) = paginate(&files, query.page, query.per_page);

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
    /// The signed `document.put` event ID for this saved revision.
    document_id: String,
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

/// Response from the image upload endpoint.
#[derive(Debug, Serialize)]
struct UploadResponse {
    /// Blake3 hash of the uploaded file.
    hash: String,
    /// Name/tag assigned to the uploaded file.
    name: String,
    /// URL to access the uploaded file.
    url: String,
}

/// Entry in the image listing response.
#[derive(Debug, Serialize)]
struct ImageEntry {
    hash: String,
    name: String,
    url: String,
    size: u64,
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
/// creates a new blob and signed document revision, archives the original under
/// a timestamped name, and updates the legacy filename tag to point to the new blob.
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
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
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
    let add_result = state.store.blobs().add_bytes(bytes.clone()).await;
    let outcome = match add_result {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save file").into_response();
        }
    };
    let new_hash = outcome.hash;
    let new_hash_str = new_hash.to_string();

    // A saved revision is durable only once its bytes are addressable and its signed event is
    // applied. The mutex protects LocalEvents' synchronous redb writes and is released before any
    // subsequent network-backed metadata operations.
    let filename_tag = [("filename".to_owned(), Some(req.name.clone()))];
    let publish_result = {
        let mut events = state.local_events.lock().await;
        events.publish(get_content_type(&req.name), &bytes, &filename_tag)
    };
    let published = match publish_result {
        Ok(published) => published,
        Err(err) => {
            tracing::error!("[routes] Failed to publish saved revision: {:#}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to publish document revision",
            )
                .into_response();
        }
    };
    let document_id = published.id.to_hex();

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
        .notify_new_version(&req.name, &new_hash_str)
        .await;

    // Record successful save for rate limiting
    state.save_limiter.record(&req.name).await;

    Json(SaveResponse {
        hash: new_hash_str,
        document_id,
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

/// Whether a multipart error is the request-body length limit firing
/// (as opposed to malformed multipart data).
///
/// This must classify via `MultipartError::status()`, NOT by matching on
/// the error's `Display`: in axum 0.8, `MultipartError`'s `Display` is the
/// constant string "Error parsing `multipart/form-data` request" for every
/// error, so a substring match can never fire (#751). axum derives the
/// status from the underlying multer error and returns
/// `413 PAYLOAD_TOO_LARGE` exactly for the length-limit cases
/// (`FieldSizeExceeded` / `StreamSizeExceeded` / a wrapped
/// `LengthLimitError`).
fn is_body_limit_error(err: &axum::extract::multipart::MultipartError) -> bool {
    err.status() == StatusCode::PAYLOAD_TOO_LARGE
}

/// The clear `413` response for an upload that exceeds the size limits.
fn image_too_large_response() -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        format!(
            "Image too large: the upload exceeds the maximum image size of {MAX_IMAGE_SIZE} bytes (10 MiB)"
        ),
    )
        .into_response()
}

/// Upload an image file via multipart form data.
///
/// Accepts a single file field, validates the MIME type against allowed
/// image types, stores the blob, creates a named tag, and sets metadata.
/// The hash `name` currently points at, if the name is taken (#1549).
async fn upload_name_owner(
    store: &iroh_blobs::api::Store,
    name: &str,
) -> Result<Option<iroh_blobs::Hash>, String> {
    store
        .tags()
        .get(name)
        .await
        .map(|info| info.map(|tag| tag.hash))
        .map_err(|err| err.to_string())
}

/// Choose the stored name for an upload of `hash` intended as
/// `intended` (#1549): the intended name itself when it is free or
/// already names this exact blob (an idempotent re-upload of identical
/// bytes keeps its name), otherwise `stem-N.ext` with the smallest
/// N >= 1 that is free or already names this blob — so a later upload
/// never silently un-names an earlier, different image.
async fn uniquify_upload_name(
    store: &iroh_blobs::api::Store,
    intended: &str,
    hash: iroh_blobs::Hash,
) -> Result<String, String> {
    match upload_name_owner(store, intended).await? {
        None => return Ok(intended.to_owned()),
        Some(owner) if owner == hash => return Ok(intended.to_owned()),
        Some(_) => {}
    }
    let (stem, ext) = match intended.rfind('.') {
        Some(dot) if dot > 0 => (&intended[..dot], &intended[dot..]),
        _ => (intended, ""),
    };
    for n in 1.. {
        let candidate = format!("{stem}-{n}{ext}");
        match upload_name_owner(store, &candidate).await? {
            None => return Ok(candidate),
            Some(owner) if owner == hash => return Ok(candidate),
            Some(_) => {}
        }
    }
    unreachable!("the counter always finds a free name")
}

async fn upload_handler(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    // Read the first field from the multipart body
    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        Ok(None) => {
            return (StatusCode::BAD_REQUEST, "No file field in upload").into_response();
        }
        Err(err) => {
            tracing::error!("[routes] Failed to read multipart field: {}", err);
            if is_body_limit_error(&err) {
                return image_too_large_response();
            }
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid multipart data: {err}"),
            )
                .into_response();
        }
    };

    // Validate content type
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    if !ALLOWED_IMAGE_TYPES.contains(&content_type.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Unsupported image type: {content_type}"),
        )
            .into_response();
    }

    // Get filename from the field, or generate one. A supplied name is
    // reconciled with the validated content type (#908) so it cannot
    // masquerade as a different type than the bytes were checked as.
    let ext = mime_to_extension(&content_type);
    let filename = match field.file_name().map(ToOwned::to_owned) {
        Some(name) if !name.trim().is_empty() => reconcile_upload_filename(&name, ext),
        _ => {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("paste-{ts}.{ext}")
        }
    };

    // Read the file bytes
    let data = match field.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!("[routes] Failed to read upload bytes: {}", err);
            if is_body_limit_error(&err) {
                return image_too_large_response();
            }
            return (StatusCode::BAD_REQUEST, "Failed to read file data").into_response();
        }
    };

    // The route body limit leaves room for the multipart envelope, so the
    // file itself must be checked against the advertised file limit here —
    // with a clear size error, not a generic multipart/body failure (#716).
    if data.len() > MAX_IMAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Image too large: {} bytes exceeds the maximum image size of {} bytes (10 MiB)",
                data.len(),
                MAX_IMAGE_SIZE
            ),
        )
            .into_response();
    }

    if data.is_empty() {
        return (StatusCode::BAD_REQUEST, "Uploaded file is empty").into_response();
    }

    tracing::info!(
        "[routes] upload_handler: name={}, content_type={}, size={}",
        filename,
        content_type,
        data.len()
    );

    // Store blob
    let outcome = match state.store.blobs().add_bytes(data.to_vec()).await {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add upload blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to store file").into_response();
        }
    };
    let hash = outcome.hash;
    let hash_str = hash.to_string();

    // #2363: Keep the check, unconditional tag set and filename-keyed
    // metadata writes in one critical section across concurrent uploads.
    // This only coordinates handlers sharing this AppState, not independent
    // states/processes or other code that writes blob tags directly.
    let _upload_name_guard = state.upload_name_lock.lock().await;

    // #1549: resolve the stored name before tagging anything. The
    // intended name is kept when free or already naming this blob; a
    // name owned by a different blob is uniquified (stem-N.ext), and
    // the nameless paste-{ts} fallback passes through the same
    // uniquifier, so two uploads can never land on the same stored
    // name. The tag, the filename-keyed metadata, and the response
    // below all use the stored name.
    let filename = match uniquify_upload_name(&state.store, &filename, hash).await {
        Ok(name) => name,
        Err(err) => {
            tracing::error!("[routes] Failed to resolve upload name: {}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to resolve file name",
            )
                .into_response();
        }
    };

    // Set named tag
    let tag = iroh_blobs::api::Tag::from(filename.clone());
    if let Err(err) = state.store.tags().set(tag, hash).await {
        tracing::error!("[routes] Failed to set upload tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to set file name").into_response();
    }

    // Set metadata tags (created, modified, content-type)
    let now_str = crate::tags::now_unix().to_string();
    if let Err(err) = state
        .tag_store
        .set_if_absent(
            &state.tag_store.global,
            filename.as_bytes(),
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
            filename.as_bytes(),
            b"modified",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set modified tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            filename.as_bytes(),
            b"content-type",
            Some(content_type.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set content-type tag: {:#}", err);
    }

    let url = format!(
        "/blob/{hash_str}?filename={}",
        urlencoding::encode(&filename)
    );

    Json(UploadResponse {
        hash: hash_str,
        name: filename,
        url,
    })
    .into_response()
}

/// List all uploaded images.
///
/// Scans blob tags for entries with image content-type metadata.
/// Returns a JSON array of `ImageEntry` objects.
async fn images_list_handler(State(state): State<AppState>) -> Response {
    use futures_lite::StreamExt;

    let mut images: Vec<ImageEntry> = Vec::new();

    // List all tags (named blobs) and filter to those with image content-types
    let mut tags_stream = match state.store.tags().list().await {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("[routes] Failed to list tags for images: {}", err);
            return Json(images).into_response();
        }
    };
    while let Some(Ok(tag_info)) = tags_stream.next().await {
        let name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
        let hash_str = tag_info.hash.to_string();

        // Skip internal tags
        if crate::tags::is_internal_tag(&name) {
            continue;
        }

        // Check content-type metadata via tag store
        let ct_tags = state
            .tag_store
            .get_by_key(&state.tag_store.global, name.as_bytes(), b"content-type")
            .await
            .unwrap_or_default();

        let ct = ct_tags
            .first()
            .and_then(|t| t.value.as_ref())
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !ALLOWED_IMAGE_TYPES.contains(&ct) {
            continue;
        }

        let url = format!("/blob/{hash_str}?filename={}", urlencoding::encode(&name));

        images.push(ImageEntry {
            hash: hash_str,
            name,
            url,
            size: blob_size(&state.store, tag_info.hash).await,
        });
    }

    Json(images).into_response()
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
    let new_name = req.new_name.trim().to_owned();
    let ops = FileOps::new(&state.store, &state.tag_store);
    match ops.rename(&req.name, &new_name, req.archive).await {
        Ok(outcome) => {
            tracing::info!(
                "[routes] File renamed: {} -> {}, hash={}, archived_original={:?}, archived_replaced={:?}",
                req.name,
                new_name,
                outcome.hash,
                outcome.archived_original,
                outcome.archived_replaced
            );
            Json(RenameResponse {
                name: new_name,
                hash: outcome.hash.to_string(),
                archived_original: outcome.archived_original,
                archived_replaced: outcome.archived_replaced,
            })
            .into_response()
        }
        Err(FileOpError::EmptyName) => {
            (StatusCode::BAD_REQUEST, "New name cannot be empty").into_response()
        }
        Err(FileOpError::SameName) => (
            StatusCode::BAD_REQUEST,
            "New name must differ from current name",
        )
            .into_response(),
        Err(FileOpError::NotFound(_)) => (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(err) => {
            tracing::error!("[routes] rename failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to rename file").into_response()
        }
    }
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
    let ops = FileOps::new(&state.store, &state.tag_store);
    match ops.copy(&req.name, &new_name).await {
        Ok(outcome) => {
            tracing::info!(
                "[routes] File copied: {} -> {}, hash={}",
                req.name,
                new_name,
                outcome.hash
            );
            Json(CopyResponse {
                name: new_name,
                hash: outcome.hash.to_string(),
            })
            .into_response()
        }
        Err(FileOpError::EmptyName) => {
            (StatusCode::BAD_REQUEST, "New name cannot be empty").into_response()
        }
        Err(FileOpError::SameName) => (
            StatusCode::BAD_REQUEST,
            "New name must differ from current name",
        )
            .into_response(),
        Err(FileOpError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, "Source file not found").into_response()
        }
        Err(err) => {
            tracing::error!("[routes] copy failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to copy file").into_response()
        }
    }
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
            let json_name = if req.name.to_lowercase().ends_with(".pm.json") {
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
    let ops = FileOps::new(&state.store, &state.tag_store);
    match ops.delete(&req.name).await {
        Ok(outcome) => {
            tracing::info!(
                "[routes] File hard-deleted: name={} (metadata={}, archives={})",
                req.name,
                outcome.metadata_removed,
                outcome.archives_removed
            );
            Json(DeleteResponse {
                name: req.name,
                hard: true,
            })
            .into_response()
        }
        Err(FileOpError::EmptyName) => {
            (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response()
        }
        Err(err) => {
            tracing::error!("[routes] hard delete failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete file").into_response()
        }
    }
}

/// Convert a `ProseMirror` document JSON to bytes in the appropriate file format.
///
/// The format is determined by the file name extension:
/// - `.pm.json` → `ProseMirror` JSON (serialized as-is)
/// - `.md` → Markdown (via `prosemirror_to_markdown`)
/// - `.txt` → Plain text (loss-free recursive extraction; see
///   `extract_plain_text` for the per-block rendering policy)
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
            // .txt files: loss-free plain-text extraction (#1306)
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

/// Extract plain text from a `ProseMirror` doc — loss-free (#1306).
///
/// Plain-text rendering policy (also documented on `convert_doc_to_bytes`):
/// every block contributes its text, one block per line, joined with
/// `"\n"`:
///
/// - `paragraph` / `heading`: their inline text (see
///   `extract_inline_text`); byte-identical to the pre-#1306 output for
///   docs containing only paragraphs and headings.
/// - Lists (`bullet_list` / `ordered_list` / `task_list`): each item
///   contributes its blocks, one item per line for the usual
///   single-paragraph item; nested lists recurse the same way.
/// - `blockquote`: its blocks, recursively.
/// - `code_block`: its raw text verbatim (internal newlines preserved).
/// - `table`: each cell contributes one line, row-major (rows in order,
///   cells in order within a row); a multi-block cell joins its blocks
///   with a single space so the one-line-per-cell shape holds.
/// - `horizontal_rule`: no text — contributes one empty line, acting as
///   a separator between the blocks around it.
/// - Any other block type: recursed into rather than dropped, so new
///   node types degrade to their text instead of vanishing.
///
/// Inline content keeps `text` and `hard_break` (as a newline) and
/// recurses into any other inline node's `content` rather than
/// dropping it. The pre-#1306 version handled only top-level
/// paragraphs/headings and silently deleted everything else on save.
///
/// Inline atoms carry no `content`, so recursion alone loses them.
/// Pinned representations (#1491): an `image` contributes its `alt`
/// text verbatim when non-empty, otherwise its `src` verbatim when
/// non-empty, otherwise nothing. Known remaining gap: `footnote_reference`
/// atoms are not yet rendered — they contribute nothing; that is
/// explicitly out of scope for the #1491 fix.
fn extract_plain_text(doc: &serde_json::Value) -> String {
    let mut lines = Vec::new();
    if let Some(content) = doc.get("content").and_then(|c| c.as_array()) {
        collect_plain_text_lines(content, &mut lines);
    }
    lines.join("\n")
}

/// Append the plain-text lines for a list of block nodes to `lines`.
fn collect_plain_text_lines(nodes: &[serde_json::Value], lines: &mut Vec<String>) {
    for node in nodes {
        let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let content = node.get("content").and_then(|c| c.as_array());
        match node_type {
            "paragraph" | "heading" => lines.push(extract_inline_text(node)),
            "code_block" => lines.push(extract_inline_text(node)),
            "horizontal_rule" => lines.push(String::new()),
            "table" => {
                if let Some(rows) = content {
                    for row in rows {
                        if let Some(cells) = row.get("content").and_then(|c| c.as_array()) {
                            for cell in cells {
                                let mut cell_lines = Vec::new();
                                if let Some(blocks) = cell.get("content").and_then(|c| c.as_array())
                                {
                                    collect_plain_text_lines(blocks, &mut cell_lines);
                                }
                                lines.push(cell_lines.join(" "));
                            }
                        }
                    }
                }
            }
            // Lists, list items, blockquote, table rows/cells reached
            // outside a table, and every other container block: recurse
            // into the block content so nothing is dropped.
            _ => {
                if let Some(blocks) = content {
                    if blocks.iter().any(|b| {
                        b.get("text").is_some()
                            || b.get("type").and_then(|t| t.as_str()) == Some("hard_break")
                    }) {
                        lines.push(extract_inline_text(node));
                    } else {
                        collect_plain_text_lines(blocks, lines);
                    }
                } else if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
                    lines.push(text.to_owned());
                }
            }
        }
    }
}

/// Extract the inline text of one node: `text` verbatim, `hard_break`
/// as a newline, and any other inline node recursed into (its own
/// inline content contributes its text instead of being dropped).
/// The `image` atom has no content — its payload is `attrs`, rendered
/// per the pinned policy on `extract_plain_text` (#1491).
fn extract_inline_text(node: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
        for inline in content {
            let inline_type = inline.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match inline_type {
                "text" => {
                    if let Some(text) = inline.get("text").and_then(|t| t.as_str()) {
                        out.push_str(text);
                    }
                }
                "hard_break" => out.push('\n'),
                "image" => {
                    let attrs = inline.get("attrs");
                    let alt = attrs
                        .and_then(|a| a.get("alt"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !alt.is_empty() {
                        out.push_str(alt);
                    } else if let Some(src) =
                        attrs.and_then(|a| a.get("src")).and_then(|v| v.as_str())
                    {
                        out.push_str(src);
                    }
                }
                _ => out.push_str(&extract_inline_text(inline)),
            }
        }
    }
    out
}

/// Extract raw text from a `ProseMirror` doc with `code_block` nodes.
///
/// Used for code and config files where the content is a single `code_block`.
///
/// Text nodes within one `code_block` are concatenated with NO separator:
/// the editor and collab layer split a block's text into multiple nodes
/// (at cursor/selection boundaries, marks, or peer edit points), and those
/// splits are not line breaks — joining them with "\n" inserted spurious
/// lines on Raw-mode save (#932). A single "\n" separates distinct
/// `code_block`s, matching how the blocks were authored.
fn extract_raw_text(doc: &serde_json::Value) -> String {
    let mut blocks = Vec::new();

    if let Some(content) = doc.get("content").and_then(|c| c.as_array()) {
        for node in content {
            let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if node_type == "code_block"
                && let Some(inline_content) = node.get("content").and_then(|c| c.as_array())
            {
                let mut block = String::new();
                for inline in inline_content {
                    if let Some(text) = inline.get("text").and_then(|t| t.as_str()) {
                        block.push_str(text);
                    }
                }
                blocks.push(block);
            }
        }
    }

    blocks.join("\n")
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
    // Bump past an existing archive tag so two archives within the same
    // second never overwrite each other (#1612).
    // Claim (resolve + set) in one critical section: resolving a free
    // name first and setting it afterwards was check-then-act, so
    // concurrent same-second archives could collide (#2462).
    match claim_archive_name(store, name, timestamp, expected).await {
        Ok(archive_name) => {
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod save_publication_tests {
    use std::{sync::Arc, time::Duration};

    use axum::{body::to_bytes, extract::State};
    use iroh::endpoint::{Endpoint, RelayMode, presets};
    use iroh_blobs::{api::Store, store::mem::MemStore};
    use iroh_docs::protocol::Docs;
    use iroh_gossip::net::Gossip;
    use serde_json::json;

    use super::*;
    use crate::{
        local_events::Entry,
        tags::TagStore,
        web::{AppState, SaveRateLimiter},
    };

    async fn state() -> (AppState, Endpoint, Docs) {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = Docs::memory()
            .spawn(endpoint.clone(), store.clone(), gossip)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            TagStore::init_in(&docs, "save-test", dir.path().join("meta"))
                .await
                .unwrap(),
        );
        let state = AppState::new(
            store,
            None,
            "save-test".to_owned(),
            tags,
            [3; 32],
            dir.path().join(".identity.redb"),
        )
        .await
        .unwrap();
        (state, endpoint, docs)
    }

    async fn save(state: AppState, name: &str, text: &str) -> Response {
        save_handler(
            State(state),
            Json(SaveRequest {
                doc_id: iroh_blobs::Hash::from_bytes([42; 32]).to_string(),
                name: name.to_owned(),
                doc: json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]}),
            }),
        )
        .await
    }

    async fn response_json(response: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn successful_save_publishes_an_applied_signed_document_and_keeps_the_blob_fetchable() {
        let (mut state, _endpoint, _docs) = state().await;
        state.save_limiter = SaveRateLimiter::new(Duration::ZERO);
        let response = save(state.clone(), "revision.md", "first revision").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let event_id: p2panda_core::Hash = body["document_id"].as_str().unwrap().parse().unwrap();
        let history = state.local_events.lock().await.history(event_id).unwrap();
        let (author, document) = history
            .iter()
            .find_map(|entry| match entry {
                Entry::Document {
                    id,
                    author,
                    document,
                } if *id == event_id => Some((*author, document.clone())),
                _ => None,
            })
            .unwrap();
        let blob_hash = iroh_blobs::Hash::from_bytes(*document.content.as_bytes());
        assert_eq!(body["hash"].as_str().unwrap(), blob_hash.to_string());
        let bytes = state.store.blobs().get_bytes(blob_hash).await.unwrap();
        document.verify_content(&bytes).unwrap();
        assert_eq!(document.blob_hash().to_string(), blob_hash.to_string());
        assert_eq!(document.size, u64::try_from(bytes.len()).unwrap());
        assert_eq!(document.media_type, "text/markdown; charset=utf-8");
        assert_eq!(
            author,
            p2panda_core::SigningKey::from_bytes(&[3; 32])
                .verifying_key()
                .as_bytes()
                .to_owned()
        );
        assert!(history.iter().any(|entry| matches!(
            entry,
            Entry::Set { subject, name, value: Some(value), .. }
                if *subject == event_id && name == "filename" && value == "revision.md"
        )));

        // Repeated successful checkpoints get distinct durable revision events.
        let second = save(state.clone(), "revision.md", "second revision").await;
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = response_json(second).await;
        assert_ne!(body["document_id"], second_body["document_id"]);
        let second_id: p2panda_core::Hash = second_body["document_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(state.local_events.lock().await.history(event_id).is_ok());
        assert!(state.local_events.lock().await.history(second_id).is_ok());
    }

    #[tokio::test]
    async fn refused_filename_tag_does_not_return_a_success_response() {
        let (state, _endpoint, _docs) = state().await;
        let response = save(state.clone(), &format!("{}.md", "x".repeat(600)), "content").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            state
                .local_events
                .lock()
                .await
                .frontier()
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod upload_router_tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use iroh::endpoint::{Endpoint, RelayMode, presets};
    use iroh_blobs::{api::Store, store::mem::MemStore};
    use iroh_docs::protocol::Docs;
    use iroh_gossip::net::Gossip;
    use tower::ServiceExt as _;

    use super::*;
    use crate::tags::TagStore;

    const BOUNDARY: &str = "----id-upload-test-boundary";

    async fn router() -> (Router, tempfile::TempDir, Endpoint, Docs) {
        let (router, _, dir, endpoint, docs) = router_with_state().await;
        (router, dir, endpoint, docs)
    }

    async fn router_with_state() -> (Router, AppState, tempfile::TempDir, Endpoint, Docs) {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = Docs::memory()
            .spawn(endpoint.clone(), store.clone(), gossip)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            TagStore::init_in(&docs, "upload-test", dir.path().join("meta"))
                .await
                .unwrap(),
        );
        let state = AppState::new(
            store,
            None,
            "upload-test".to_owned(),
            tags,
            [9; 32],
            dir.path().join(".identity.redb"),
        )
        .await
        .unwrap();
        (create_router(state.clone()), state, dir, endpoint, docs)
    }

    fn multipart_body(file_bytes: &[u8]) -> Vec<u8> {
        multipart_body_named("photo.png", "image/png", file_bytes)
    }

    fn multipart_body_named(filename: &str, content_type: &str, file_bytes: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
                 filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(file_bytes);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    async fn get_blob(router: Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, bytes.to_vec())
    }

    fn header_str<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Extract the blob hash from an upload response's JSON body.
    fn uploaded_hash(upload_body: &str) -> String {
        let start = upload_body.find("/blob/").unwrap() + "/blob/".len();
        let rest = &upload_body[start..];
        rest.chars()
            .take_while(|c| *c != '?' && *c != '"')
            .collect()
    }

    async fn upload(router: Router, body: Vec<u8>) -> (StatusCode, String) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/upload")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={BOUNDARY}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn list_images(router: Router) -> Vec<serde_json::Value> {
        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/images")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn multipart_body_noname(content_type: &str, file_bytes: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(file_bytes);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    #[tokio::test]
    async fn concurrent_uploads_keep_names_and_metadata_consistent() {
        let (router, state, _dir, _endpoint, _docs) = router_with_state().await;
        // Hold the same lock used at the resolve/claim boundary while both
        // requests start; neither may complete its claim until it is released.
        let guard = state.upload_name_lock.lock().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut tasks = Vec::new();
        for bytes in [b"first-bytes".as_slice(), b"second-bytes".as_slice()] {
            let router = router.clone();
            let barrier = barrier.clone();
            let body = multipart_body_named("image.png", "image/png", bytes);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                upload(router, body).await
            }));
        }
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(tasks.iter().all(|task| !task.is_finished()));
        drop(guard);

        let images = {
            let mut results = Vec::new();
            for task in tasks {
                let (status, body) = task.await.unwrap();
                assert_eq!(status, StatusCode::OK, "{body}");
                results.push(serde_json::from_str::<serde_json::Value>(&body).unwrap());
            }
            results
        };
        assert_ne!(images[0]["name"], images[1]["name"]);
        assert_ne!(images[0]["hash"], images[1]["hash"]);
        let listed = list_images(router.clone()).await;
        for (image, bytes) in images.iter().zip([b"first-bytes".as_slice(), b"second-bytes".as_slice()]) {
            let name = image["name"].as_str().unwrap();
            let hash = image["hash"].as_str().unwrap();
            assert_eq!(upload_name_owner(&state.store, name).await.unwrap().unwrap().to_string(), hash);
            assert!(listed.iter().any(|entry| entry["name"] == name && entry["hash"] == hash));
            let url = image["url"].as_str().unwrap();
            assert!(url.contains(&format!("filename={name}")));
            let (status, _, actual) = get_blob(router.clone(), url).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(actual, bytes);
            for (key, expected) in [(b"content-type".as_slice(), Some("image/png")), (b"created".as_slice(), None)] {
                let tags = state.tag_store.get_by_key(&state.tag_store.global, name.as_bytes(), key).await.unwrap();
                let value = tags.first().and_then(|tag| tag.value.as_ref()).and_then(|value| value.as_str()).unwrap();
                if let Some(expected) = expected {
                    assert_eq!(value, expected);
                } else {
                    assert!(value.parse::<u64>().is_ok(), "invalid creation timestamp: {value}");
                }
            }
        }
    }

    // #1549: a second upload under the same filename with different
    // bytes must not silently un-name the first: the second is stored
    // uniquified, both stay listed and resolvable, and the first name
    // still resolves to the first hash. Re-uploading identical bytes
    // under the same name is idempotent.
    #[tokio::test]
    async fn upload_same_name_different_bytes_uniquifies_second() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, first) = upload(
            router.clone(),
            multipart_body_named("image.png", "image/png", b"first-bytes"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {first}");
        assert!(first.contains("\"name\":\"image.png\""), "body: {first}");
        let first_hash = uploaded_hash(&first);

        let (status, second) = upload(
            router.clone(),
            multipart_body_named("image.png", "image/png", b"second-bytes"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {second}");
        assert!(
            second.contains("\"name\":\"image-1.png\""),
            "second upload stored uniquified: {second}"
        );
        assert!(second.contains("filename=image-1.png"), "body: {second}");
        let second_hash = uploaded_hash(&second);
        assert_ne!(first_hash, second_hash);

        let (status, again) = upload(
            router.clone(),
            multipart_body_named("image.png", "image/png", b"first-bytes"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {again}");
        assert!(
            again.contains("\"name\":\"image.png\""),
            "identical re-upload keeps the plain name: {again}"
        );
        assert_eq!(uploaded_hash(&again), first_hash);

        let images = list_images(router.clone()).await;
        let by_name = |name: &str| {
            images
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap_or_else(|| panic!("{name} listed in {images:?}"))
        };
        assert_eq!(by_name("image.png")["hash"], first_hash);
        assert_eq!(by_name("image-1.png")["hash"], second_hash);

        let (_, _, bytes) = get_blob(router.clone(), &format!("/blob/{first_hash}")).await;
        assert_eq!(bytes, b"first-bytes");
        let (_, _, bytes) = get_blob(router, &format!("/blob/{second_hash}")).await;
        assert_eq!(bytes, b"second-bytes");
    }

    // #1549: the nameless paste-{secs} fallback passes through the
    // same uniquifier, so two nameless uploads in the same second can
    // never land on the same stored name.
    #[tokio::test]
    async fn upload_nameless_uploads_never_share_a_stored_name() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, first) = upload(
            router.clone(),
            multipart_body_noname("image/png", b"nameless-one"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {first}");
        let (status, second) = upload(
            router.clone(),
            multipart_body_noname("image/png", b"nameless-two"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {second}");
        let name_of = |body: &str| {
            serde_json::from_str::<serde_json::Value>(body).unwrap()["name"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let (first_name, second_name) = (name_of(&first), name_of(&second));
        assert!(first_name.starts_with("paste-"), "first: {first_name}");
        assert_ne!(
            first_name, second_name,
            "nameless uploads must not share a stored name"
        );
        let images = list_images(router).await;
        let hash_of = |name: &str| {
            images
                .iter()
                .find(|entry| entry["name"] == name)
                .map(|entry| entry["hash"].as_str().unwrap().to_owned())
                .unwrap_or_else(|| panic!("{name} listed in {images:?}"))
        };
        assert_eq!(hash_of(&first_name), uploaded_hash(&first));
        assert_eq!(hash_of(&second_name), uploaded_hash(&second));
    }

    // #751: a body over UPLOAD_BODY_LIMIT must surface as the clear 413,
    // not a generic 400. On PR #747's original code this returned 400,
    // because is_body_limit_error substring-matched MultipartError's
    // constant Display and could never fire.
    #[tokio::test]
    async fn upload_over_body_limit_returns_clear_413() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let file = vec![b'x'; UPLOAD_BODY_LIMIT];
        let (status, body) = upload(router, multipart_body(&file)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "body: {body}");
        assert!(body.contains("Image too large"), "body: {body}");
    }

    // #716/#751: a file of exactly MAX_IMAGE_SIZE (plus its multipart
    // envelope) fits the route body limit and uploads successfully.
    #[tokio::test]
    async fn upload_exactly_max_image_size_succeeds() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let file = vec![b'y'; MAX_IMAGE_SIZE];
        let (status, body) = upload(router, multipart_body(&file)).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert!(body.contains("\"name\":\"photo.png\""), "body: {body}");
    }

    // #716: a file over MAX_IMAGE_SIZE but under the route body limit gets
    // the explicit per-file 413 from upload_handler's length check.
    #[tokio::test]
    async fn upload_over_max_image_size_under_body_limit_returns_413() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let file = vec![b'z'; MAX_IMAGE_SIZE + 1024];
        let (status, body) = upload(router, multipart_body(&file)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "body: {body}");
        assert!(body.contains("Image too large"), "body: {body}");
        assert!(
            body.contains("exceeds the maximum image size"),
            "body: {body}"
        );
    }

    // Non-size multipart errors still return 400 as before (#751).
    #[tokio::test]
    async fn upload_malformed_multipart_returns_400() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, _body) = upload(router, b"garbage without any boundary".to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // #908: a caller-supplied `?filename=evil.html` must not turn a PNG
    // blob into same-origin HTML. The blob is downgraded to a download.
    #[tokio::test]
    async fn blob_html_filename_param_is_not_served_as_html() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, body) = upload(router.clone(), multipart_body(b"png-bytes")).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let hash = uploaded_hash(&body);
        let (status, headers, bytes) =
            get_blob(router, &format!("/blob/{hash}?filename=evil.html")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, b"png-bytes");
        assert_eq!(
            header_str(&headers, "content-type"),
            Some("application/octet-stream")
        );
        assert_eq!(
            header_str(&headers, "content-disposition"),
            Some("attachment")
        );
        assert_eq!(
            header_str(&headers, "x-content-type-options"),
            Some("nosniff")
        );
        let csp = header_str(&headers, "content-security-policy").unwrap_or("");
        assert!(csp.contains("sandbox"), "csp: {csp}");
    }

    // #908: a real PNG is still served inline as image/png (the editor
    // embeds the returned URL as an image src), with protective headers.
    #[tokio::test]
    async fn blob_png_is_served_inline_with_protective_headers() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, body) = upload(router.clone(), multipart_body(b"png-bytes")).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let hash = uploaded_hash(&body);
        let (status, headers, _bytes) =
            get_blob(router, &format!("/blob/{hash}?filename=photo.png")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(header_str(&headers, "content-type"), Some("image/png"));
        assert!(headers.get("content-disposition").is_none());
        assert_eq!(
            header_str(&headers, "x-content-type-options"),
            Some("nosniff")
        );
        let csp = header_str(&headers, "content-security-policy").unwrap_or("");
        assert!(csp.contains("sandbox"), "csp: {csp}");
    }

    // #908: an SVG (which can carry script) keeps its image type for
    // embedding but is served with the sandboxing CSP + nosniff so a
    // top-level visit cannot execute it in this origin.
    #[tokio::test]
    async fn blob_svg_is_served_with_protective_headers() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, body) = upload(
            router.clone(),
            multipart_body_named("icon.svg", "image/svg+xml", b"<svg/>"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let hash = uploaded_hash(&body);
        let (status, headers, _bytes) =
            get_blob(router, &format!("/blob/{hash}?filename=icon.svg")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(header_str(&headers, "content-type"), Some("image/svg+xml"));
        assert_eq!(
            header_str(&headers, "x-content-type-options"),
            Some("nosniff")
        );
        let csp = header_str(&headers, "content-security-policy").unwrap_or("");
        assert!(csp.contains("default-src 'none'"), "csp: {csp}");
        assert!(csp.contains("sandbox"), "csp: {csp}");
    }

    // #908: an upload whose name disagrees with its validated content
    // type is stored under the reconciled name, and the returned URL
    // carries that name (percent-encoded by construction here).
    #[tokio::test]
    async fn upload_name_masquerading_as_other_type_is_reconciled() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, body) = upload(
            router,
            multipart_body_named("evil.html", "image/png", b"png-bytes"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert!(body.contains("\"name\":\"evil.png\""), "body: {body}");
        assert!(body.contains("filename=evil.png"), "body: {body}");
    }

    #[test]
    fn safe_blob_content_type_allowlists_passive_types_only() {
        assert_eq!(safe_blob_content_type("a.png"), ("image/png", true));
        assert_eq!(safe_blob_content_type("a.svg"), ("image/svg+xml", true));
        assert_eq!(safe_blob_content_type("a.pdf"), ("application/pdf", true));
        assert_eq!(
            safe_blob_content_type("evil.html"),
            ("application/octet-stream", false)
        );
        assert_eq!(
            safe_blob_content_type("evil.HTML"),
            ("application/octet-stream", false)
        );
        assert_eq!(
            safe_blob_content_type("x.js"),
            ("application/octet-stream", false)
        );
        assert_eq!(
            safe_blob_content_type("x.json"),
            ("application/octet-stream", false)
        );
        assert_eq!(
            safe_blob_content_type("noext"),
            ("application/octet-stream", false)
        );
    }

    #[test]
    fn reconcile_upload_filename_matches_validated_type() {
        assert_eq!(reconcile_upload_filename("evil.html", "png"), "evil.png");
        assert_eq!(reconcile_upload_filename("photo.png", "png"), "photo.png");
        assert_eq!(reconcile_upload_filename("photo.jpeg", "jpg"), "photo.jpeg");
        assert_eq!(reconcile_upload_filename("photo", "webp"), "photo.webp");
        assert_eq!(
            reconcile_upload_filename("archive.tar.gz", "gif"),
            "archive.tar.gif"
        );
    }
}

// =============================================================================
// Identity API Handlers
// =============================================================================

/// Request body for identity registration.
#[derive(Debug, Deserialize)]
struct IdentityRegisterRequest {
    /// Optional display name for the client.
    name: Option<String>,
}

/// Response body for identity registration and lookup.
#[derive(Debug, Serialize)]
struct IdentityResponse {
    /// Opaque token for the client to store in `localStorage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    /// The client's unique identifier.
    client_id: String,
    /// Display name (if set).
    name: Option<String>,
}

/// Request body for name update.
#[derive(Debug, Deserialize)]
struct IdentityUpdateNameRequest {
    /// The token proving client identity.
    token: String,
    /// New display name (or null/empty to clear).
    name: Option<String>,
}

/// `POST /api/identity/register` - Register a new client identity.
///
/// Returns a signed token that the client stores in `localStorage`
/// for future reconnection.
async fn identity_register_handler(
    State(state): State<AppState>,
    Json(req): Json<IdentityRegisterRequest>,
) -> impl IntoResponse {
    match state.identity.register(req.name).await {
        Ok((token, identity)) => {
            tracing::info!("[identity] Registered new client: {}", identity.client_id);
            (
                StatusCode::OK,
                Json(IdentityResponse {
                    token: Some(token),
                    client_id: identity.client_id,
                    name: identity.name,
                }),
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!("[identity] Registration failed: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /api/identity/me?token=...` - Look up current identity from token.
///
/// Used by the client on page load to check if a stored token is still valid.
/// Returns a refreshed token on success, resetting the 30-day expiry clock.
async fn identity_me_handler(
    State(state): State<AppState>,
    Query(params): Query<IdentityMeQuery>,
) -> impl IntoResponse {
    let Some((fresh_token, identity)) = state.identity.verify_and_refresh(&params.token).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    (
        StatusCode::OK,
        Json(IdentityResponse {
            token: Some(fresh_token),
            client_id: identity.client_id,
            name: identity.name,
        }),
    )
        .into_response()
}

/// Query parameters for the identity lookup endpoint.
#[derive(Debug, Deserialize)]
struct IdentityMeQuery {
    /// The client's token.
    token: String,
}

/// `POST /api/identity/name` - Update display name.
async fn identity_update_name_handler(
    State(state): State<AppState>,
    Json(req): Json<IdentityUpdateNameRequest>,
) -> impl IntoResponse {
    // Verify the token first
    let Some(identity) = state.identity.verify_token(&req.token).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // Update the name
    let Some(updated) = state
        .identity
        .update_name(&identity.client_id, req.name)
        .await
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    // Immediately update cursors in all active documents so other clients
    // (and newly joining clients) see the new name without waiting for
    // the renamed client's next cursor move.
    let display_name = updated
        .name
        .clone()
        .unwrap_or_else(|| super::identity::short_id(&updated.client_id));
    state
        .collab
        .update_client_name(&updated.client_id, &display_name)
        .await;

    tracing::info!(
        "[identity] Updated name for {}: {:?}",
        updated.client_id,
        updated.name
    );

    (
        StatusCode::OK,
        Json(IdentityResponse {
            token: None,
            client_id: updated.client_id,
            name: updated.name,
        }),
    )
        .into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn raw_doc(blocks: &[&[&str]]) -> serde_json::Value {
        let content: Vec<serde_json::Value> = blocks
            .iter()
            .map(|nodes| {
                let inline: Vec<serde_json::Value> = nodes
                    .iter()
                    .map(|t| serde_json::json!({"type": "text", "text": t}))
                    .collect();
                serde_json::json!({"type": "code_block", "content": inline})
            })
            .collect();
        serde_json::json!({"type": "doc", "content": content})
    }

    #[test]
    fn test_extract_raw_text_single_node() {
        let doc = raw_doc(&[&["let x = 42;"]]);
        assert_eq!(extract_raw_text(&doc), "let x = 42;");
    }

    #[test]
    fn test_extract_raw_text_split_mid_line_no_separator() {
        // Regression (#932): editor/collab text-node splits inside one
        // code_block are not line breaks.
        let doc = raw_doc(&[&["let x = ", "42;"]]);
        assert_eq!(extract_raw_text(&doc), "let x = 42;");
    }

    #[test]
    fn test_extract_raw_text_split_inside_string_literal() {
        let doc = raw_doc(&[&["let s = \"hel", "lo\";"]]);
        assert_eq!(extract_raw_text(&doc), "let s = \"hello\";");
    }

    #[test]
    fn test_extract_raw_text_split_doc_byte_identical_to_single_node() {
        let split = raw_doc(&[&["fn main() {", "\n    println!(\"hi\");", "\n}"]]);
        let single = raw_doc(&[&["fn main() {\n    println!(\"hi\");\n}"]]);
        assert_eq!(extract_raw_text(&split), extract_raw_text(&single));
    }

    #[test]
    fn test_extract_raw_text_multiple_blocks_single_newline() {
        let doc = raw_doc(&[&["first"], &["second"]]);
        assert_eq!(extract_raw_text(&doc), "first\nsecond");
    }

    #[test]
    fn test_extract_raw_text_multiple_blocks_with_splits() {
        let doc = raw_doc(&[&["fir", "st"], &["sec", "ond"]]);
        assert_eq!(extract_raw_text(&doc), "first\nsecond");
    }

    fn plain_doc(content: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({"type": "doc", "content": content})
    }

    fn para(text: &str) -> serde_json::Value {
        serde_json::json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
    }

    #[test]
    fn test_extract_plain_text_paragraph_heading_byte_identical() {
        // Pre-#1306 behaviour for paragraph/heading-only docs is unchanged.
        let doc = plain_doc(vec![
            serde_json::json!({"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "Title"}]}),
            para("intro"),
            para("outro"),
        ]);
        assert_eq!(extract_plain_text(&doc), "Title\nintro\noutro");
    }

    #[test]
    fn test_extract_plain_text_bullet_list_items_survive() {
        let doc = plain_doc(vec![
            para("intro"),
            serde_json::json!({"type": "bullet_list", "content": [
                {"type": "list_item", "content": [para("apples")]},
                {"type": "list_item", "content": [para("pears")]}
            ]}),
            para("outro"),
        ]);
        assert_eq!(extract_plain_text(&doc), "intro\napples\npears\noutro");
    }

    #[test]
    fn test_extract_plain_text_blockquote_survives() {
        let doc = plain_doc(vec![
            serde_json::json!({"type": "blockquote", "content": [para("quoted words")]}),
        ]);
        assert_eq!(extract_plain_text(&doc), "quoted words");
    }

    #[test]
    fn test_extract_plain_text_code_block_survives() {
        let doc = plain_doc(vec![
            para("before"),
            serde_json::json!({"type": "code_block", "attrs": {"params": "rust"}, "content": [{"type": "text", "text": "fn main() {}\n"}]}),
            para("after"),
        ]);
        let out = extract_plain_text(&doc);
        assert!(out.contains("fn main() {}"), "out: {out}");
        assert!(out.starts_with("before\n"), "out: {out}");
        assert!(out.ends_with("\nafter"), "out: {out}");
    }

    #[test]
    fn test_extract_plain_text_table_cells_survive_row_major() {
        let cell = |t: &str| serde_json::json!({"type": "table_cell", "content": [para(t)]});
        let doc = plain_doc(vec![serde_json::json!({"type": "table", "content": [
            {"type": "table_row", "content": [cell("a1"), cell("b1")]},
            {"type": "table_row", "content": [cell("a2"), cell("b2")]}
        ]})]);
        assert_eq!(extract_plain_text(&doc), "a1\nb1\na2\nb2");
    }

    #[test]
    fn test_extract_plain_text_mixed_doc_loses_nothing() {
        // The #1306 repro shape: paragraph/list/blockquote/code/table/paragraph.
        let doc = plain_doc(vec![
            para("intro"),
            serde_json::json!({"type": "bullet_list", "content": [{"type": "list_item", "content": [para("item")]}]}),
            serde_json::json!({"type": "blockquote", "content": [para("quote")]}),
            serde_json::json!({"type": "code_block", "content": [{"type": "text", "text": "code"}]}),
            serde_json::json!({"type": "table", "content": [{"type": "table_row", "content": [{"type": "table_cell", "content": [para("cell")]}]}]}),
            para("outro"),
        ]);
        assert_eq!(
            extract_plain_text(&doc),
            "intro\nitem\nquote\ncode\ncell\noutro"
        );
    }

    fn img(attrs: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"type": "image", "attrs": attrs})
    }

    #[test]
    fn test_extract_plain_text_image_only_paragraph_saves_as_alt() {
        // #1491: an image atom's only text is its alt.
        let doc = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
            img(serde_json::json!({"src": "blob:abc", "alt": "a red barn", "title": null}))
        ]})]);
        assert_eq!(extract_plain_text(&doc), "a red barn");
    }

    #[test]
    fn test_extract_plain_text_image_between_text_runs() {
        let doc = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
            {"type": "text", "text": "before "},
            img(serde_json::json!({"src": "blob:abc", "alt": "middle", "title": null})),
            {"type": "text", "text": " after"}
        ]})]);
        assert_eq!(extract_plain_text(&doc), "before middle after");
    }

    #[test]
    fn test_extract_plain_text_image_in_list_item() {
        // Recursive paths funnel through extract_inline_text, so the
        // image policy applies inside list items too.
        let doc = plain_doc(vec![serde_json::json!({"type": "bullet_list", "content": [
            {"type": "list_item", "content": [serde_json::json!({"type": "paragraph", "content": [
                img(serde_json::json!({"src": "blob:abc", "alt": "listed", "title": null}))
            ]})]}
        ]})]);
        assert_eq!(extract_plain_text(&doc), "listed");
    }

    #[test]
    fn test_extract_plain_text_image_without_alt_falls_back_to_src() {
        // Missing alt and empty alt both fall back to src verbatim.
        let missing = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
            img(serde_json::json!({"src": "photo.png", "title": null}))
        ]})]);
        assert_eq!(extract_plain_text(&missing), "photo.png");
        let empty = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
            img(serde_json::json!({"src": "photo.png", "alt": "", "title": null}))
        ]})]);
        assert_eq!(extract_plain_text(&empty), "photo.png");
    }

    #[test]
    fn test_extract_plain_text_image_with_neither_alt_nor_src_is_empty() {
        let doc = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
            {"type": "text", "text": "x"},
            img(serde_json::json!({"alt": "", "title": null})),
            {"type": "text", "text": "y"}
        ]})]);
        assert_eq!(extract_plain_text(&doc), "xy");
    }

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
    fn test_classify_tag_auto_prefix_non_date_is_primary() {
        // Regression (#809): an ordinary file that merely starts with
        // "auto-" is not an auto-backup — only "auto-{ISO_DATE}" is.
        for name in [
            "auto-notes.txt",
            "auto-2026-report.md",
            "auto-",
            "auto-draft",
        ] {
            let (kind, ts) = classify_tag(name);
            assert_eq!(kind, FileKind::Primary, "{name}");
            assert!(ts.is_none(), "{name}");
        }
    }

    #[test]
    fn test_classify_tag_archive() {
        let (kind, ts) = classify_tag("notes.md.archive.1711000000");
        assert_eq!(kind, FileKind::Archive);
        assert_eq!(ts.unwrap(), 1_711_000_000);
    }

    #[test]
    fn test_classify_tag_archive_no_timestamp() {
        // Regression (#2793): an ordinary file that merely contains
        // ".archive." is not an archive — only ".archive.{unix_timestamp}"
        // is, mirroring test_classify_tag_auto_prefix_non_date_is_primary.
        for name in [
            "notes.md.archive.notanumber",
            "notes.archive.txt",
            "meeting.archive.notes",
            "backup.archive.draft",
            "x.archive.",
            "photo.archive.2024-01-01",
        ] {
            let (kind, ts) = classify_tag(name);
            assert_eq!(kind, FileKind::Primary, "{name}");
            assert!(ts.is_none(), "{name}");
        }
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
    fn test_parse_iso_timestamp_rejects_out_of_range_components() {
        // Each of these used to roll over into a different, plausible
        // timestamp instead of returning None (#756).
        assert!(parse_iso_timestamp("2026-13-01T00:00:00Z").is_none());
        assert!(parse_iso_timestamp("2026-00-01T00:00:00Z").is_none());
        assert!(parse_iso_timestamp("2026-03-00T00:00:00Z").is_none());
        assert!(parse_iso_timestamp("2026-03-32T00:00:00Z").is_none());
        assert!(parse_iso_timestamp("2026-02-30T00:00:00Z").is_none());
        assert!(parse_iso_timestamp("2026-03-12T25:00:00Z").is_none());
        assert!(parse_iso_timestamp("2026-03-12T00:60:00Z").is_none());
        assert!(parse_iso_timestamp("2026-03-12T00:00:60Z").is_none());
        // Before the epoch the u64 result cannot represent the date;
        // it used to collapse to Some(0), the same value as the epoch.
        assert!(parse_iso_timestamp("1969-01-01T00:00:00Z").is_none());
        // Absurd years must not be accepted (or iterated year by year).
        assert!(parse_iso_timestamp("999999999-01-01T00:00:00Z").is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_leap_day() {
        // 2024 is a leap year: Feb 29 is real and must keep parsing.
        assert_eq!(
            parse_iso_timestamp("2024-02-29T00:00:00Z"),
            Some(1_709_164_800)
        );
        // 2026 is not: Feb 29 does not exist.
        assert!(parse_iso_timestamp("2026-02-29T00:00:00Z").is_none());
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

    // ========================================================================
    // Upload endpoint tests
    // ========================================================================

    #[test]
    fn test_upload_response_serialization() {
        let resp = UploadResponse {
            hash: "abc123def456".to_owned(),
            name: "photo.png".to_owned(),
            url: "/blob/abc123def456?filename=photo.png".to_owned(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"hash\":\"abc123def456\""));
        assert!(json.contains("\"name\":\"photo.png\""));
        assert!(json.contains("\"url\":\"/blob/abc123def456?filename=photo.png\""));
    }

    #[test]
    fn test_upload_body_limit_fits_max_image_plus_multipart_overhead() {
        // #716: the client's MAX_IMAGE_SIZE (web/src/image-upload.ts, pinned
        // at the same value by image-upload.test.ts) is a *file* limit, while
        // the server body limit counts the whole multipart request. The
        // upload route's body limit must fit a max-size file plus the
        // multipart envelope — including a long filename — or the largest
        // image the UI accepts is rejected by the server.
        assert_eq!(MAX_IMAGE_SIZE, 10 * 1024 * 1024);
        // Multipart overhead for a 255-byte filename, mirroring the measured
        // ~170 bytes for a short name (issue #716): two boundary lines, the
        // Content-Disposition header carrying the filename, the part
        // Content-Type header, and CRLF separators.
        let long_filename = "a".repeat(251) + ".png";
        let overhead = 70 + 70 // boundary lines (incl. closing)
            + "Content-Disposition: form-data; name=\"file\"; filename=\"\"".len()
            + long_filename.len()
            + "Content-Type: image/png".len()
            + 8; // CRLFs
        assert!(
            UPLOAD_BODY_LIMIT >= MAX_IMAGE_SIZE + overhead,
            "upload body limit {UPLOAD_BODY_LIMIT} must fit a {MAX_IMAGE_SIZE}-byte file \
             plus {overhead} bytes of multipart overhead"
        );
        // …while the router-wide limit for the other routes stays at 10 MiB.
        assert_eq!(DEFAULT_BODY_LIMIT, 10 * 1024 * 1024);
        const { assert!(UPLOAD_BODY_LIMIT > DEFAULT_BODY_LIMIT) };
    }

    #[test]
    fn test_allowed_image_types_contains_expected() {
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/png"));
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/jpeg"));
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/gif"));
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/webp"));
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/svg+xml"));
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/bmp"));
        assert!(ALLOWED_IMAGE_TYPES.contains(&"image/x-icon"));
    }

    #[test]
    fn test_allowed_image_types_rejects_non_images() {
        assert!(!ALLOWED_IMAGE_TYPES.contains(&"text/plain"));
        assert!(!ALLOWED_IMAGE_TYPES.contains(&"application/pdf"));
        assert!(!ALLOWED_IMAGE_TYPES.contains(&"application/json"));
        assert!(!ALLOWED_IMAGE_TYPES.contains(&"video/mp4"));
    }

    #[test]
    fn test_mime_to_extension() {
        assert_eq!(mime_to_extension("image/png"), "png");
        assert_eq!(mime_to_extension("image/jpeg"), "jpg");
        assert_eq!(mime_to_extension("image/gif"), "gif");
        assert_eq!(mime_to_extension("image/webp"), "webp");
        assert_eq!(mime_to_extension("image/svg+xml"), "svg");
        assert_eq!(mime_to_extension("image/bmp"), "bmp");
        assert_eq!(mime_to_extension("image/x-icon"), "ico");
        assert_eq!(mime_to_extension("application/octet-stream"), "bin");
        assert_eq!(mime_to_extension("text/plain"), "bin");
    }

    #[test]
    fn test_paginate_normal() {
        let items: Vec<usize> = (0..10).collect();
        let (page_items, page, per_page) = paginate(&items, Some(3), Some(2));
        assert_eq!(page_items, vec![4, 5]);
        assert_eq!(page, 3);
        assert_eq!(per_page, 2);
    }

    #[test]
    fn test_paginate_overflow_page_per_page() {
        let items: Vec<usize> = (0..10).collect();
        // Using page=3, per_page=usize::MAX would overflow in standard multiplication
        // The query string max represents per_page, which we clamp to MAX_PER_PAGE.
        // But even if we didn't clamp, saturating arithmetic prevents panic.
        let (page_items, page, per_page) = paginate(&items, Some(3), Some(usize::MAX));
        assert_eq!(page_items, Vec::<usize>::new());
        assert_eq!(page, 3);
        assert_eq!(per_page, MAX_PER_PAGE);
    }

    #[test]
    fn test_paginate_overflow_page() {
        let items: Vec<usize> = (0..10).collect();
        // page=usize::MAX, per_page=50
        let (page_items, page, per_page) = paginate(&items, Some(usize::MAX), Some(50));
        assert_eq!(page_items, Vec::<usize>::new());
        assert_eq!(page, usize::MAX);
        assert_eq!(per_page, 50);
    }

    #[test]
    fn test_paginate_above_max_per_page() {
        let items: Vec<usize> = (0..300).collect();
        let (page_items, page, per_page) = paginate(&items, Some(1), Some(1000));
        // We clamp per_page to MAX_PER_PAGE (200)
        assert_eq!(page_items.len(), 200);
        assert_eq!(page_items[0], 0);
        assert_eq!(page_items[199], 199);
        assert_eq!(page, 1);
        assert_eq!(per_page, MAX_PER_PAGE);
    }
}
