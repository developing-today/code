//! HTTP route handlers for the web interface.
//!
//! Defines all the Axum routes and their handlers for serving the web UI.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Form, Json, Router,
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
use super::security::{self, WebSecurity};
use super::templates::{
    AssetUrls, hex_encode, render_binary_viewer, render_blocks, render_editor, render_editor_page,
    render_file_list, render_file_list_content, render_main_page_wrapper, render_media_viewer,
    render_page, render_peers, render_peers_content, render_settings,
};
use id_events::event_store::{Block, BlockKind};
use id_store::tags::TagStore;
use id_sync::fileops::{FileOpError, FileOps, claim_archive_name, snapshot_provenance_tags};
use iroh_base::EndpointId;

mod file_handlers;
use file_handlers::*;

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

/// Verify that uploaded bytes actually look like the image type the
/// multipart field declared (#5342). Before this check, `upload_handler`
/// trusted the declared Content-Type alone, so any bytes (HTML, script,
/// plain text) could be stored — and later served — as an image of the
/// declared type. Raster formats are recognised by their magic
/// signatures; SVG is text, so it is checked structurally by
/// [`svg_document_starts`]. This is a signature check, not a full format
/// decode: it stops type masquerading, it does not prove the rest of
/// the file is well-formed.
fn image_bytes_match_declared_type(content_type: &str, data: &[u8]) -> bool {
    match content_type {
        "image/png" => data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/jpeg" => data.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        "image/webp" => data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP",
        "image/bmp" => data.starts_with(b"BM"),
        // ICO (type 1) and CUR (type 2) share the container header.
        "image/x-icon" => {
            data.starts_with(&[0x00, 0x00, 0x01, 0x00])
                || data.starts_with(&[0x00, 0x00, 0x02, 0x00])
        }
        "image/svg+xml" => match decode_utf16_text(data) {
            // An SVG encoded as UTF-16 (Codex P2 on PR #5357, round 3)
            // is a valid image/svg+xml document whose markup is not
            // ASCII bytes; decode it and run the same check on the text.
            Some(decoded) => svg_document_starts(decoded.as_bytes()),
            None => svg_document_starts(data),
        },
        _ => false,
    }
}

/// Decode UTF-16 text when `data` carries one of its signatures: a
/// UTF-16LE (`FF FE`) or UTF-16BE (`FE FF`) byte-order mark, or the
/// UTF-16 spelling of an opening `<` (`3C 00` / `00 3C`) when there is
/// no BOM. Returns `None` for anything else (the caller then treats
/// the bytes as UTF-8/ASCII, as before). Lossy on unpaired surrogates —
/// only the prolog/root prefix is inspected afterwards.
fn decode_utf16_text(data: &[u8]) -> Option<String> {
    let (body, little_endian) = if data.starts_with(&[0xFF, 0xFE]) {
        (&data[2..], true)
    } else if data.starts_with(&[0xFE, 0xFF]) {
        (&data[2..], false)
    } else if data.starts_with(&[0x3C, 0x00]) {
        (data, true)
    } else if data.starts_with(&[0x00, 0x3C]) {
        (data, false)
    } else {
        return None;
    };
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Whether `data` is an SVG document: after an optional UTF-8 BOM and
/// the legal XML prolog constructs — whitespace, `<?xml ... ?>` or
/// another processing instruction, `<!-- ... -->` comments, and a
/// `<!DOCTYPE ...>` declaration (real-world SVGs carry all of these:
/// see `doc/database_structure_20231217_074440.svg`, a Graphviz export
/// with a declaration, an SVG 1.1 DOCTYPE, and comments before the
/// root) — the document must open with an `svg` root element (plain or namespace-prefixed).
fn svg_document_starts(data: &[u8]) -> bool {
    let mut rest = data;
    if rest.starts_with(&[0xEF, 0xBB, 0xBF]) {
        rest = &rest[3..];
    }
    loop {
        while rest.first().is_some_and(u8::is_ascii_whitespace) {
            rest = &rest[1..];
        }
        if rest.starts_with(b"<?") {
            // XML declaration or processing instruction.
            match rest.windows(2).position(|w| w == b"?>") {
                Some(end) => rest = &rest[end + 2..],
                None => return false,
            }
        } else if rest.starts_with(b"<!--") {
            match rest.windows(3).position(|w| w == b"-->") {
                Some(end) => rest = &rest[end + 3..],
                None => return false,
            }
        } else if rest.starts_with(b"<!DOCTYPE") || rest.starts_with(b"<!doctype") {
            match doctype_end(rest) {
                Some(end) => rest = &rest[end + 1..],
                None => return false,
            }
        } else {
            break;
        }
    }
    svg_root_element_opens(rest)
}

/// Byte index of the `>` that ends a `<!DOCTYPE ...>` declaration at
/// the start of `rest`. The declaration ends at the first `>` that is
/// outside the `[ ... ]` internal subset, outside any quoted literal
/// (`"..."` / `'...'`), and outside any `<!-- ... -->` comment —
/// brackets and angle brackets inside quotes or comments are data,
/// not delimiters (Codex P2 on PR #5357, round 4: an entity value of
/// `"["` used to corrupt the subset depth and reject a legal SVG).
fn doctype_end(rest: &[u8]) -> Option<usize> {
    let mut subset_depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut in_comment = false;
    let mut i = 0usize;
    while i < rest.len() {
        if in_comment {
            if rest[i..].starts_with(b"-->") {
                in_comment = false;
                i += 3;
            } else {
                i += 1;
            }
            continue;
        }
        if let Some(q) = quote {
            if rest[i] == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if rest[i..].starts_with(b"<!--") {
            in_comment = true;
            i += 4;
            continue;
        }
        match rest[i] {
            b'"' | b'\'' => quote = Some(rest[i]),
            b'[' => subset_depth += 1,
            b']' => subset_depth = subset_depth.saturating_sub(1),
            b'>' if subset_depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether `rest` opens an element whose local name is `svg`: the
/// plain `<svg ...>` spelling, or a namespace-prefixed root such as
/// `<s:svg xmlns:s="http://www.w3.org/2000/svg"/>` (Codex P2 on
/// PR #5357 — prefixed SVGs previously uploaded fine and the client
/// allowlist still accepts them). The tag name is read whole, so
/// `<svgx>` and `<x:html>` do not pass.
fn svg_root_element_opens(rest: &[u8]) -> bool {
    if !rest.starts_with(b"<")
        || rest.starts_with(b"</")
        || rest.starts_with(b"<!")
        || rest.starts_with(b"<?")
    {
        return false;
    }
    let name = &rest[1..];
    let end = name
        .iter()
        .position(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
        .unwrap_or(name.len());
    if end == 0 {
        return false;
    }
    let local_name = name[..end]
        .rsplit(|b| *b == b':')
        .next()
        .unwrap_or(&name[..end]);
    local_name == b"svg"
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
    /// Sort order (#4973): `uploaded` (newest upload first) or `origin`
    /// (origin node A-Z, unknown last). Absent keeps the default order.
    pub sort: Option<String>,
    /// Filter to one origin node id, or `unknown` for files with no
    /// recorded origin (#4973).
    pub origin: Option<String>,
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
    /// The sort order applied, echoed for rendering (#4973).
    pub sort: Option<String>,
    /// The origin filter applied, echoed for rendering (#4973).
    pub origin_filter: Option<String>,
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
    /// Node that first wrote the file (#4973), from the `origin` metadata
    /// tag. `None` for files written before origin stamping existed;
    /// listings render it as unknown and never guess.
    pub origin_node: Option<String>,
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
        // The producer's Display form is unsigned decimal digits only:
        // parse::<u64>() would also accept a leading '+', a spelling no
        // producer mints (#4329).
        if !ts_str.is_empty() && ts_str.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(ts) = ts_str.parse::<u64>() {
                return (FileKind::Archive, Some(ts));
            }
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
        // The fraction is a component too: it must be digits, not junk
        // silently truncated away (#1879).
        let fraction = &s[dot_pos + 1..];
        if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        &s[..dot_pos]
    } else {
        s
    };

    // Parse: YYYY-MM-DDThh:mm:ss
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }

    // Every component must parse: a non-numeric segment makes the whole
    // timestamp malformed (#1879). Dropping failed segments let junk like
    // "2026-03-12-draft" collapse onto the valid "2026-03-12" and parse
    // as the same instant.
    let date_strs: Vec<&str> = parts[0].split('-').collect();
    let time_strs: Vec<&str> = parts[1].split(':').collect();

    if date_strs.len() != 3 || time_strs.len() != 3 {
        return None;
    }

    // Components must carry the canonical spellings of the documented
    // format: a 4-digit year and 2-digit everything else, digits
    // only (#4329). parse::<u64>() alone accepts any width and a leading
    // '+', so non-canonical spellings like "2026-3-2T6:4:2Z" or a
    // "+2026" year used to map onto the canonical instant.
    let canonical =
        |s: &str, width: usize| s.len() == width && s.bytes().all(|b| b.is_ascii_digit());
    if !canonical(date_strs[0], 4)
        || !date_strs[1..].iter().all(|p| canonical(p, 2))
        || !time_strs.iter().all(|p| canonical(p, 2))
    {
        return None;
    }

    let date_parts: Vec<u64> = date_strs
        .iter()
        .map(|p| p.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    let time_parts: Vec<u64> = time_strs
        .iter()
        .map(|p| p.parse().ok())
        .collect::<Option<Vec<u64>>>()?;

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

/// Create the guarded main router with all web routes.
///
/// `security` is applied to every request before any route handler runs. Choose
/// an explicit policy appropriate to the listener; `WebSecurity::default()`
/// does not require a token or restrict hosts.
pub fn create_router(state: AppState, security: WebSecurity) -> Router {
    create_router_for_app(
        state,
        &super::app_interface::WebInterface::world_app(),
        security,
    )
}

/// Create the main router for one packaged app, binding the world
/// page at the app's [interfaces.web] route and the live channel at
/// its socket (#2325, adapter mode). With the world app's own
/// interface the result has the same routes, handlers, and state as
/// [`create_router`]. The supplied security policy guards every route.
///
/// # Migration
///
/// Callers must now pass an explicit [`WebSecurity`] policy. The default
/// policy does not require a token or restrict hosts.
pub fn create_router_for_app(
    mut state: AppState,
    interface: &super::app_interface::WebInterface,
    security: WebSecurity,
) -> Router {
    state.web_route = interface.route.clone();
    state.web_socket = interface.socket.clone();
    security::protect(create_router_inner(state), security)
}

/// Create the program-only router for one hosted world (#2146
/// phase 2): the world program frontend at `/`, the static assets
/// the trusted presenter loads, and the world session bridge (the
/// shared world HTTP surface plus the live channel at the app's
/// socket) -- and nothing else. Explorer pages and APIs (files,
/// settings, peers, identity, collab, tags, save/upload) are absent
/// from this router, so a program listener cannot serve them; the
/// session authority is unchanged (a valid world guest capability,
/// never the world admin token). The supplied [`WebSecurity`] policy
/// guards every request to this router as well.
///
/// # Migration
///
/// Callers must now pass an explicit [`WebSecurity`] policy. The default
/// policy does not require a token or restrict hosts.
pub fn create_program_router(
    mut state: AppState,
    world_name: &str,
    security: WebSecurity,
) -> Router {
    state.program_world = Some(world_name.to_owned());
    let world_socket = state
        .web_socket
        .clone()
        .unwrap_or_else(|| "/ws/world".to_owned());
    // The bridge is pinned to the bound world (#5728): the shared
    // bridge state carries a pinned hub view, so every bridge entry
    // point (invite, host-view, metrics, directory, explore,
    // websocket sessions) resolves only this world and refuses any
    // other by name — the program surface is one listener, one world.
    let world_state = super::world_ws::WorldWebState {
        hub: state.world.as_ref().map(|hub| hub.pinned_to(world_name)),
        cookie_secure: state.cookie_secure,
    };
    let router = Router::new()
        .route("/", get(program_page_handler))
        .route("/assets/{*path}", get(assets_handler))
        .merge(super::world_ws::world_routes_at(&world_socket).with_state(world_state));
    let router = router
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .with_state(state);
    security::protect(router, security)
}

fn create_router_inner(state: AppState) -> Router {
    let world_route = state.web_route.clone();
    let world_socket = state.web_socket.clone();
    let router = Router::new()
        // Page routes (return full HTML pages)
        .route("/", get(index_handler))
        .route("/files", get(files_page_handler))
        .route(&world_route, get(world_page_handler))
        .route("/settings", get(settings_handler))
        .route("/peers", get(peers_handler))
        .route("/blocks", get(blocks_page_handler))
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
        .route(
            "/api/blocks",
            get(blocks_partial_handler).post(block_create_handler),
        )
        .route("/api/blocks/remove", post(block_remove_handler))
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
        // Seitan admission API (#8487)
        .route(
            "/api/seitan/invite",
            post(super::seitan::seitan_invite_handler),
        )
        .route(
            "/api/seitan/admit",
            post(super::seitan::seitan_admit_handler),
        )
        .route("/api/identity/me", get(identity_me_handler))
        .route("/api/identity/name", post(identity_update_name_handler))
        // WebSocket for collaboration
        .route("/ws/collab/{doc_id}", get(super::collab::ws_collab_handler))
        // WebSocket for live tag updates
        .route("/ws/tags", get(super::tags_ws::ws_tags_handler))
        // Static assets
        .route("/assets/{*path}", get(assets_handler));
    // Optional authoritative in-memory world session bridge, bound
    // at the app's socket when it declares one (#2325). The shared
    // world HTTP surface (invite, metrics, directory, explore) is
    // merged either way: an app that declares no live channel still
    // serves those routes (Codex P2 on #3894).
    let world_state = super::world_ws::WorldWebState {
        hub: state.world.clone(),
        cookie_secure: state.cookie_secure,
    };
    let router = match &world_socket {
        Some(socket) => {
            router.merge(super::world_ws::world_routes_at(socket).with_state(world_state))
        }
        None => router.merge(super::world_ws::world_http_routes().with_state(world_state)),
    };
    router
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .with_state(state)
}

/// The program-only surface's root: the program frontend for the
/// world this router's binding names (#2146 phase 2). A hub that
/// does not host that world (not its default and not in its world
/// list) is a 404 -- never a lobby for a different world.
async fn program_page_handler(State(state): State<AppState>) -> Response {
    let Some(world_name) = state.program_world.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(hub) = state.world.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let hosted = hub.default_world() == world_name
        || hub.list().iter().any(|world| world.name == world_name);
    if !hosted {
        return StatusCode::NOT_FOUND.into_response();
    }
    Html(super::templates::render_program_page(
        &state.assets,
        world_name,
        state.web_socket.as_deref(),
    ))
    .into_response()
}

async fn world_page_handler(State(state): State<AppState>) -> Response {
    if state.world.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Html(super::templates::render_world_page(
        &state.assets,
        state.web_socket.as_deref(),
    ))
    .into_response()
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
    let content = super::home::render_home(
        &worlds,
        default_world,
        state.world.is_some(),
        &state.web_route,
    );
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
) -> Response {
    let page = match get_file_list_page(&state, &query).await {
        Ok(page) => page,
        Err(error) => return file_list_error_response(error),
    };
    let content = render_file_list(&page);
    if is_partial_request(&headers) {
        // Partial request - return wrapped content with header/footer
        Html(render_main_page_wrapper(&content)).into_response()
    } else {
        Html(render_page("Files", &content, "", &state.assets)).into_response()
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

    let content = render_peers(&peers_data, &blocked_peer_ids(&state).await);
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
) -> Response {
    let page = match get_file_list_page(&state, &query).await {
        Ok(page) => page,
        Err(error) => return file_list_error_response(error),
    };
    // Return just the inner content (list + pagination) for partial replacement
    Html(render_file_list_content(&page)).into_response()
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

    // Return only the inner content: fetchPartial assigns this as
    // innerHTML of #peers-content, so the wrapper must not be included
    // (issue #6956).
    Html(render_peers_content(
        &peers_data,
        &blocked_peer_ids(&state).await,
    ))
}

/// Node-ID strings (peers-page form) that already name a peer-key row on
/// the operator block list (#2245). A store read failure yields an empty
/// set: the page still renders, just without blocked badges.
async fn blocked_peer_ids(state: &AppState) -> std::collections::HashSet<String> {
    match state.local_events.lock().await.blocks() {
        Ok(rows) => rows
            .iter()
            .filter(|block| block.kind == BlockKind::PeerKey)
            .map(|block| hex_encode(&block.value))
            .collect(),
        Err(err) => {
            tracing::warn!("[routes] Could not read the block list: {err:#}");
            std::collections::HashSet::new()
        }
    }
}

/// Which block rows a list/unblock form addresses (#2245).
fn parse_block_kind(raw: Option<&str>) -> Result<BlockKind, String> {
    match raw.unwrap_or("peer_key") {
        "peer_key" => Ok(BlockKind::PeerKey),
        "ip" => Ok(BlockKind::Ip),
        "topic_tag" => Ok(BlockKind::TopicTag),
        other => Err(format!(
            "unknown block kind {other:?}; want peer_key, ip, or topic_tag"
        )),
    }
}

/// Raw row value for `kind` from the operator's text: a peer key parses
/// as an endpoint ID (its 32 key bytes are the row, matching
/// [`BlockList::peer_blocked`](id_events::event_store::BlockList::peer_blocked)),
/// an IP parses and stores in text form (the form
/// [`BlockList::ip_blocked`](id_events::event_store::BlockList::ip_blocked)
/// checks), a topic tag stores verbatim.
fn parse_block_value(kind: BlockKind, raw: &str) -> Result<Vec<u8>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("block value must not be empty".to_owned());
    }
    match kind {
        BlockKind::PeerKey => raw
            .parse::<EndpointId>()
            .map(|id| id.as_bytes().to_vec())
            .map_err(|_| "peer key must be an endpoint ID (64 hex characters)".to_owned()),
        BlockKind::Ip => raw
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.to_string().into_bytes())
            .map_err(|_| "IP must parse as an IP address".to_owned()),
        BlockKind::TopicTag => Ok(raw.as_bytes().to_vec()),
    }
}

/// Only a local path on this host is a valid post-action destination;
/// anything else falls back to the Blocks page (no open redirects).
fn sanitize_return_to(raw: Option<&str>) -> String {
    match raw {
        Some(path) if path.starts_with('/') && !path.starts_with("//") => path.to_owned(),
        _ => "/blocks".to_owned(),
    }
}

/// One operator block form, from a peer row or the Blocks page (#2245).
#[derive(Debug, Deserialize)]
struct BlockForm {
    /// Peer endpoint ID, IP text, or topic tag (per `kind`).
    value: String,
    /// `peer_key` (default), `ip`, or `topic_tag`.
    kind: Option<String>,
    /// Operator's reason; stored on the row.
    reason: Option<String>,
    /// Optional expiry as unix milliseconds.
    expires_at_ms: Option<String>,
    /// Where to land after the write.
    return_to: Option<String>,
}

/// Refusal page for a bad block form: the reason plus a way back. The
/// message carries operator input, so it renders escaped.
fn bad_block_request(message: &str, return_to: &str, assets: &AssetUrls) -> Response {
    let content = format!(
        "<h1>Blocks</h1><p>Could not write the block: {}</p><p><a href=\"{}\">Back</a></p>",
        super::templates::html_escape(message),
        super::templates::html_escape(return_to),
    );
    (
        StatusCode::BAD_REQUEST,
        Html(render_page("Blocks", &content, "", assets)),
    )
        .into_response()
}

/// Handler to write one operator block row (`POST /api/blocks`, #2245).
async fn block_create_handler(
    State(state): State<AppState>,
    Form(form): Form<BlockForm>,
) -> Response {
    let return_to = sanitize_return_to(form.return_to.as_deref());
    let kind = match parse_block_kind(form.kind.as_deref()) {
        Ok(kind) => kind,
        Err(message) => return bad_block_request(&message, &return_to, &state.assets),
    };
    let value = match parse_block_value(kind, &form.value) {
        Ok(value) => value,
        Err(message) => return bad_block_request(&message, &return_to, &state.assets),
    };
    let expires_at_ms = match form.expires_at_ms.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => match raw.parse::<u64>() {
            Ok(ts) => Some(ts),
            Err(_) => {
                return bad_block_request(
                    "expiry must be unix milliseconds, or left blank for a permanent block",
                    &return_to,
                    &state.assets,
                );
            }
        },
    };
    let block = Block {
        kind,
        value,
        added_at_ms: id_core::time::unix_ms(),
        reason: form.reason.as_deref().unwrap_or("").trim().to_owned(),
        expires_at_ms,
    };
    match state.local_events.lock().await.add_block(&block) {
        Ok(()) => {
            tracing::info!("[routes] Operator block written: {kind:?}");
            Redirect::to(&return_to).into_response()
        }
        Err(err) => {
            tracing::error!("[routes] Could not write the operator block: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!(
                    "<h1>Blocks</h1><p>Could not write the block: {}</p>",
                    super::templates::html_escape(&format!("{err:#}")),
                )),
            )
                .into_response()
        }
    }
}

/// Form that names the row an unblock removes (`POST /api/blocks/remove`).
#[derive(Debug, Deserialize)]
struct UnblockForm {
    /// The blocked value exactly as the Blocks page rendered it.
    value: String,
    /// `peer_key` (default), `ip`, or `topic_tag`.
    kind: Option<String>,
    /// Where to land after the removal.
    return_to: Option<String>,
}

/// Handler to remove one operator block row (`POST /api/blocks/remove`,
/// #2245). Removing a row that is not there still lands back on the page.
async fn block_remove_handler(
    State(state): State<AppState>,
    Form(form): Form<UnblockForm>,
) -> Response {
    let return_to = sanitize_return_to(form.return_to.as_deref());
    let kind = match parse_block_kind(form.kind.as_deref()) {
        Ok(kind) => kind,
        Err(message) => return bad_block_request(&message, &return_to, &state.assets),
    };
    let value = match parse_block_value(kind, &form.value) {
        Ok(value) => value,
        Err(message) => return bad_block_request(&message, &return_to, &state.assets),
    };
    match state.local_events.lock().await.remove_block(kind, &value) {
        Ok(removed) => {
            tracing::info!("[routes] Operator block removed: {kind:?} (row present: {removed})");
            Redirect::to(&return_to).into_response()
        }
        Err(err) => {
            tracing::error!("[routes] Could not remove the operator block: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!(
                    "<h1>Blocks</h1><p>Could not remove the block: {}</p>",
                    super::templates::html_escape(&format!("{err:#}")),
                )),
            )
                .into_response()
        }
    }
}

/// Blocks page handler - every operator block row with unblock actions.
async fn blocks_page_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let blocks = match state.local_events.lock().await.blocks() {
        Ok(blocks) => blocks,
        Err(err) => {
            let content = format!(
                "<h1>Blocks</h1><p>Could not read the block list: {}</p>",
                super::templates::html_escape(&format!("{err:#}")),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(render_page("Blocks", &content, "", &state.assets)),
            )
                .into_response();
        }
    };
    let content = render_blocks(&blocks);
    if is_partial_request(&headers) {
        Html(render_main_page_wrapper(&content)).into_response()
    } else {
        Html(render_page("Blocks", &content, "", &state.assets)).into_response()
    }
}

/// API handler for the blocks partial (the Blocks page fragment).
async fn blocks_partial_handler(State(state): State<AppState>) -> Response {
    match state.local_events.lock().await.blocks() {
        Ok(blocks) => Html(render_blocks(&blocks)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not read the block list: {err:#}"),
        )
            .into_response(),
    }
}

/// Static assets handler.
async fn assets_handler(Path(path): Path<String>) -> impl IntoResponse {
    super::assets::static_handler(&path)
}

/// System tag keys that are excluded from user-visible tags display.
const SYSTEM_TAG_KEYS: &[&str] = &[
    "created", "modified", "deleted", "name", "file", "path", "origin",
];

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileListError {
    MetadataTags,
    TagList,
    TagStream,
    SearchTags,
}

fn checked_file_list_result<T, E>(
    result: Result<T, E>,
    failure: FileListError,
) -> Result<T, FileListError> {
    result.map_err(|_| failure)
}

fn file_list_error_response(error: FileListError) -> Response {
    tracing::error!(?error, "Failed to build file list");
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to list files").into_response()
}

async fn next_tag_info<S, T, E>(tags: &mut S) -> Result<Option<T>, FileListError>
where
    S: futures_lite::Stream<Item = Result<T, E>> + Unpin,
{
    use futures_lite::StreamExt;

    match tags.next().await {
        Some(Ok(tag)) => Ok(Some(tag)),
        Some(Err(_)) => Err(FileListError::TagStream),
        None => Ok(None),
    }
}

async fn get_file_list(state: &AppState) -> Result<Vec<FileInfo>, FileListError> {
    use std::collections::HashMap;

    let mut files = Vec::new();

    // Load all tags from the global namespace for created/modified/user-tag lookups.
    let all_meta_tags = checked_file_list_result(
        state.tag_store.list_all(&state.tag_store.global).await,
        FileListError::MetadataTags,
    )?;
    let mut created_map: HashMap<String, u64> = HashMap::new();
    let mut modified_map: HashMap<String, u64> = HashMap::new();
    let mut origin_map: HashMap<String, String> = HashMap::new();
    let mut deleted_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut user_tags_map: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    let mut display_name_map: HashMap<String, (u8, String)> = HashMap::new();
    for tag in &all_meta_tags {
        if tag.key == "created" {
            if let Some(ref v) = tag.value
                && let Some(s) = v.as_str()
                && let Ok(ts) = s.parse::<u64>()
            {
                created_map.insert(tag.subject.display_lossy(), ts);
            }
        } else if tag.key == "origin" {
            if let Some(ref v) = tag.value
                && let Some(s) = v.as_str()
            {
                origin_map.insert(tag.subject.display_lossy(), s.to_owned());
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
        // Resolve display name from name/file/path tags (prefer name > file > path),
        // regardless of the order in which the tags were returned.
        record_display_name(
            &mut display_name_map,
            tag.subject.display_lossy(),
            tag.key.as_bytes(),
            tag.value.as_ref().and_then(|value| value.as_str()),
        );
        // Collect user-visible tags (not system tags)
        if !is_system_tag_key(&tag.key) {
            user_tags_map
                .entry(tag.subject.display_lossy())
                .or_default()
                .push((
                    tag.key.display_lossy(),
                    tag.value
                        .as_ref()
                        .map(id_store::tags::TagValue::display_lossy),
                ));
        }
    }

    // First pass: collect all tags and build hash→primary-name map
    let mut hash_to_primary: HashMap<String, String> = HashMap::new();
    let mut raw_tags: Vec<(String, String, u64)> = Vec::new();

    let mut tags =
        checked_file_list_result(state.store.tags().list().await, FileListError::TagList)?;
    while let Some(tag_info) = next_tag_info(&mut tags).await? {
        let name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();

        // Skip internal tags (e.g., .meta)
        if id_store::tags::is_internal_tag(&name) {
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
        let origin_node = origin_map.get(&name).cloned();
        let tags = user_tags_map.remove(&name).unwrap_or_default();
        let display_name = display_name_map
            .get(&name)
            .map(|(_, display_name)| display_name.clone());

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
            origin_node,
            tags,
            is_deleted,
        });
    }

    // Deduplicate files with the same hash, display name, user tags, and
    // recorded origin (#5382). Origin is part of the key: the origin filter
    // in get_file_list_page runs after this deduplication, so collapsing
    // same-content files from distinct origins here would make the
    // discarded origin unqueryable.
    {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        files.retain(|f| {
            let dedup_key = format!(
                "{}|{}|{:?}|{:?}",
                f.hash,
                f.display_name.as_deref().unwrap_or(&f.name),
                f.tags,
                f.origin_node
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

    Ok(files)
}

fn record_display_name(
    display_name_map: &mut std::collections::HashMap<String, (u8, String)>,
    subject: String,
    key: &[u8],
    value: Option<&str>,
) {
    let priority = match key {
        b"name" => 0,
        b"file" => 1,
        b"path" => 2,
        _ => return,
    };
    let Some(value) = value else {
        return;
    };

    let should_replace = display_name_map
        .get(&subject)
        .is_none_or(|(current_priority, _)| priority < *current_priority);
    if should_replace {
        display_name_map.insert(subject, (priority, value.to_owned()));
    }
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

async fn get_file_list_page(
    state: &AppState,
    query: &FileListQuery,
) -> Result<FileListPage, FileListError> {
    let mut files = get_file_list(state).await?;

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
        let all_tags = checked_file_list_result(
            state.tag_store.list_all(&state.tag_store.global).await,
            FileListError::SearchTags,
        )?;
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

        files.retain(|f| {
            f.name.to_lowercase().contains(&needle_lower) || tag_matches.contains(&f.name)
        });
    }

    // Origin filter (#4973): an exact node id, or `unknown` for files
    // with no recorded origin.
    let origin_filter = query
        .origin
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    if let Some(ref origin) = origin_filter {
        if origin == "unknown" {
            files.retain(|f| f.origin_node.is_none());
        } else {
            files.retain(|f| f.origin_node.as_deref() == Some(origin.as_str()));
        }
    }

    // Sort (#4973): the default order from get_file_list stands unless a
    // sort is requested. `uploaded` is the file's created stamp (falling
    // back to modified, then the tag-parsed timestamp), newest first;
    // `origin` is A-Z with unknown origins last.
    let sort = query
        .sort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    match sort.as_deref() {
        Some("uploaded") => {
            files.sort_by(|a, b| {
                let ta = a.created_at.or(a.modified_at).or(a.timestamp).unwrap_or(0);
                let tb = b.created_at.or(b.modified_at).or(b.timestamp).unwrap_or(0);
                tb.cmp(&ta)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        Some("origin") => {
            files.sort_by(|a, b| {
                let ka = (
                    a.origin_node.is_none(),
                    a.origin_node.as_deref().unwrap_or(""),
                );
                let kb = (
                    b.origin_node.is_none(),
                    b.origin_node.as_deref().unwrap_or(""),
                );
                ka.cmp(&kb)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        _ => {}
    }

    let total = files.len();
    let (page_files, page, per_page) = paginate(&files, query.page, query.per_page);

    Ok(FileListPage {
        files: page_files,
        total,
        page,
        per_page,
        search,
        show_deleted,
        sort,
        origin_filter,
    })
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
    use id_events::local_events::Entry;
    use id_store::tags::TagStore;

    use crate::{AppState, SaveRateLimiter};

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

    #[tokio::test]
    async fn empty_store_file_list_remains_a_successful_empty_result() {
        let (state, _endpoint, _docs) = state().await;

        assert!(get_file_list(&state).await.unwrap().is_empty());
        let page = get_file_list_page(&state, &FileListQuery::default())
            .await
            .unwrap();
        assert_eq!(page.total, 0);
        assert!(page.files.is_empty());
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
    async fn archive_original_tag_preserves_recorded_provenance() {
        let (state, _endpoint, _docs) = state().await;
        let outcome = state
            .store
            .blobs()
            .add_bytes(b"original bytes".to_vec())
            .await
            .unwrap();
        let hash = outcome.hash;
        state
            .store
            .tags()
            .set(iroh_blobs::api::Tag::from("doc.md".to_owned()), hash)
            .await
            .unwrap();
        for (key, value) in [("origin", "node-alpha"), ("created", "1700000000")] {
            state
                .tag_store
                .set_singleton(
                    &state.tag_store.global,
                    b"doc.md",
                    key.as_bytes(),
                    Some(value.as_bytes()),
                    b"",
                )
                .await
                .unwrap();
        }

        let archive =
            archive_original_tag(&state.store, &state.tag_store, "doc.md", &hash.to_string())
                .await
                .unwrap();
        assert!(archive.starts_with("doc.md.archive."), "{archive}");

        // #5383: the archive alias carries the source's recorded
        // provenance, so listings report its real origin.
        for (key, expected) in [
            (b"origin".as_slice(), "node-alpha"),
            (b"created".as_slice(), "1700000000"),
        ] {
            let tags = state
                .tag_store
                .get_by_key(&state.tag_store.global, archive.as_bytes(), key)
                .await
                .unwrap();
            assert_eq!(tags.len(), 1, "{key:?}: {tags:?}");
            assert_eq!(tags[0].value.as_ref().unwrap().as_str(), Some(expected));
        }
    }

    #[tokio::test]
    async fn file_list_dedup_preserves_same_content_files_with_distinct_origins() {
        let (state, _endpoint, _docs) = state().await;
        let outcome = state
            .store
            .blobs()
            .add_bytes(b"shared content".to_vec())
            .await
            .unwrap();
        let hash = outcome.hash;
        for name in ["alpha.md", "beta.md"] {
            state
                .store
                .tags()
                .set(iroh_blobs::api::Tag::from(name.to_owned()), hash)
                .await
                .unwrap();
        }
        // Same display name and same user tag for both files; only the
        // recorded origin differs (#5382).
        for (subject, origin) in [("alpha.md", "node-alpha"), ("beta.md", "node-beta")] {
            for (key, value) in [
                ("name", "shared.md"),
                ("topic", "shared"),
                ("origin", origin),
            ] {
                state
                    .tag_store
                    .set_singleton(
                        &state.tag_store.global,
                        subject.as_bytes(),
                        key.as_bytes(),
                        Some(value.as_bytes()),
                        b"",
                    )
                    .await
                    .unwrap();
            }
        }

        let files = get_file_list(&state).await.unwrap();
        let mut origins: Vec<String> = files
            .iter()
            .filter(|f| f.hash == hash.to_string())
            .filter_map(|f| f.origin_node.clone())
            .collect();
        origins.sort();
        assert_eq!(
            origins,
            vec!["node-alpha".to_owned(), "node-beta".to_owned()],
            "both distinct-origin rows must survive deduplication: {files:?}"
        );

        // Filtering by either recorded origin returns its own row.
        for (origin, expected_name) in [("node-alpha", "alpha.md"), ("node-beta", "beta.md")] {
            let page = get_file_list_page(
                &state,
                &FileListQuery {
                    origin: Some(origin.to_owned()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert_eq!(page.total, 1, "origin filter {origin}");
            assert_eq!(page.files[0].name, expected_name, "origin filter {origin}");
        }
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
    use id_store::tags::TagStore;

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
        (
            create_router(state.clone(), WebSecurity::default()),
            state,
            dir,
            endpoint,
            docs,
        )
    }

    fn hub_with_world(name: &str, admin: Option<&str>) -> id_worlds::world_hub::WorldHub {
        id_worlds::world_hub::WorldHub::single_named(
            name,
            id_worlds::world_session::WorldService::new(
                id_worlds::world::WorldHandle::spawn(
                    id_worlds::world::WorldCore::new(
                        name,
                        id_worlds::world::WorldLimits::default(),
                    )
                    .unwrap(),
                ),
                admin.map(str::to_owned),
            ),
        )
    }

    #[tokio::test]
    async fn program_router_serves_only_the_program_surface() {
        let (_, mut state, _dir, _endpoint, _docs) = router_with_state().await;
        state.world = Some(hub_with_world("arena", Some("s3cret")));
        let app = create_program_router(state, "arena", WebSecurity::default());

        // Root: the program page, pinned to the bound world.
        let root = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(root.status(), StatusCode::OK);
        let body = to_bytes(root.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("data-world-program=\"arena\""), "{text}");
        assert!(text.contains("data-world-view"), "{text}");
        assert!(!text.contains("data-world-admin"), "{text}");

        // Explorer pages and APIs are absent from the program surface.
        for path in [
            "/files",
            "/settings",
            "/peers",
            "/world",
            "/api/files",
            "/api/save",
            "/api/identity/me",
        ] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        }

        // The world session surface is present with its gates intact:
        // minting an invite without the admin secret is refused
        // (401), not missing (404) and never satisfied by the web
        // layer -- the program surface adds no authority of its own.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/world/invite")
                    .header("content-type", "application/json")
                    .body(Body::from("{\"display_name\":\"guest\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    /// Oracle (#5728 acceptance): with a hub hosting two worlds, a
    /// program router bound to one of them refuses the other on
    /// every bridge entry point that names a world — invite,
    /// host-view, directory read and directory write answer 404
    /// without leasing or creating it — and serves the bound world
    /// on the same entry points.
    #[tokio::test]
    async fn program_router_bridge_is_pinned_to_the_bound_world() {
        let (_, mut state, _dir, _endpoint, _docs) = router_with_state().await;
        let opener = id_worlds::world_hub::testing::MemoryOpener::new(false);
        let dyn_opener: std::sync::Arc<dyn id_worlds::world_hub::WorldOpener> = opener.clone();
        let hub = id_worlds::world_hub::WorldHub::new(
            dyn_opener,
            Some("admin".to_owned()),
            "arena",
            id_worlds::world_hub::HubLimits::default(),
        );
        // Host both worlds before the program router exists.
        hub.create("arena").await.unwrap();
        hub.create("vault").await.unwrap();
        state.world = Some(hub);
        let app = create_program_router(state, "arena", WebSecurity::default());

        let post = |uri: &str, body: String| {
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("x-world-admin-token", "admin")
                .body(Body::from(body))
                .unwrap()
        };
        let get_admin = |uri: &str| {
            Request::builder()
                .uri(uri)
                .header("x-world-admin-token", "admin")
                .body(Body::empty())
                .unwrap()
        };

        // Invite: the other world is refused, the bound world and
        // the defaulted (unnamed) invite are served.
        let res = app
            .clone()
            .oneshot(post(
                "/api/world/invite",
                r#"{"display_name":"guest","world":"vault"}"#.to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "invite vault");
        for body in [
            r#"{"display_name":"guest","world":"arena"}"#,
            r#"{"display_name":"guest"}"#,
        ] {
            let res = app
                .clone()
                .oneshot(post("/api/world/invite", body.to_owned()))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "invite {body}");
        }

        // Host-view: the other world is refused, the bound world served.
        let res = app
            .clone()
            .oneshot(get_admin("/api/world/host-view?world=vault"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "host-view vault");
        let res = app
            .clone()
            .oneshot(get_admin("/api/world/host-view?world=arena"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "host-view arena");

        // Directory read: the other world is refused, the bound
        // world is served.
        let res = app
            .clone()
            .oneshot(get_admin("/api/world/directory?world=vault"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "directory read vault");
        let res = app
            .clone()
            .oneshot(get_admin("/api/world/directory?world=arena"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "directory read arena");

        // Directory write: the other world is refused (no account is
        // created there), the bound world accepts the sign-up.
        let res = app
            .clone()
            .oneshot(post(
                "/api/world/directory",
                r#"{"world":"vault","action":"sign_up","name":"Mallory"}"#.to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "directory write vault");
        let res = app
            .clone()
            .oneshot(post(
                "/api/world/directory",
                r#"{"world":"arena","action":"sign_up","name":"Ada"}"#.to_owned(),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "directory write arena");
    }

    #[tokio::test]
    async fn program_router_404s_for_a_world_the_hub_does_not_host() {
        let (_, mut state, _dir, _endpoint, _docs) = router_with_state().await;
        state.world = Some(hub_with_world("arena", None));
        let app = create_program_router(state, "missing", WebSecurity::default());
        let root = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(root.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn program_router_404s_without_a_hub() {
        let (_, state, _dir, _endpoint, _docs) = router_with_state().await;
        let app = create_program_router(state, "arena", WebSecurity::default());
        let root = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(root.status(), StatusCode::NOT_FOUND);
    }

    // ---- Seitan admission (#8487) ----

    fn seitan_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn seitan_now_ms() -> u64 {
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

    async fn seitan_response_json(response: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    /// The account a seitan proof names in these tests: a keyed
    /// account whose id is its public key's hex, as provisioning
    /// records it. Returns (key hex, uid) — the same string here.
    fn seitan_account_key() -> String {
        seitan_hex(
            ed25519_dalek::SigningKey::from_bytes(&[7; 32])
                .verifying_key()
                .as_bytes(),
        )
    }

    /// A router over a state whose seitan state is `seitan` (with the
    /// test account already keyed into its directory when
    /// `with_account`), hosting world `arena` under admin `s3cret`.
    async fn seitan_app(
        seitan: crate::SeitanState,
        with_account: bool,
    ) -> (Router, AppState, tempfile::TempDir, Endpoint, Docs) {
        let (_, mut state, dir, endpoint, docs) = router_with_state().await;
        if with_account {
            seitan
                .sign_up_keyed(&seitan_account_key(), "joiner")
                .unwrap();
        }
        state.seitan = seitan;
        state.world = Some(hub_with_world("arena", Some("s3cret")));
        let app = create_router(state.clone(), WebSecurity::default());
        (app, state, dir, endpoint, docs)
    }

    async fn seitan_register(app: &Router, token: &str, admin: Option<&str>) -> Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/seitan/invite")
            .header("content-type", "application/json");
        if let Some(admin) = admin {
            builder = builder.header("x-world-admin-token", admin);
        }
        let body = serde_json::json!({
            "token": token,
            "world": "arena",
            "expires_ms": seitan_now_ms() + 3_600_000,
        })
        .to_string();
        app.clone()
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    async fn seitan_admit(app: &Router, proof: &id_directory::seitan::Proof) -> Response {
        let body = serde_json::json!({
            "message": seitan_hex(&proof.message),
            "signature": seitan_hex(&proof.signature),
        })
        .to_string();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/seitan/admit")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn seitan_admit_happy_path_under_a_configured_window() {
        // (a) A non-default window (10 min, not the 5-minute default)
        // is exercised at the boundary: a proof signed 8 minutes
        // before the invite was created is outside the default window
        // and inside the configured one, and it admits.
        let seitan = crate::SeitanState::new(
            id_directory::directory::Directory::new(),
            std::time::Duration::from_mins(10),
        );
        let (app, state, _dir, _endpoint, _docs) = seitan_app(seitan, true).await;
        let token = id_directory::seitan::Token::generate();
        let registered = seitan_register(&app, token.as_str(), Some("s3cret")).await;
        assert_eq!(registered.status(), StatusCode::OK);
        let uid = seitan_account_key();
        let proof = id_directory::seitan::accept::sign(
            &token,
            &uid,
            1,
            seitan_now_ms().saturating_sub(480_000),
        )
        .unwrap();
        let admitted = seitan_admit(&app, &proof).await;
        assert_eq!(admitted.status(), StatusCode::OK);
        let body = seitan_response_json(admitted).await;
        assert_eq!(body["uid"], uid);
        assert_eq!(body["world"], "arena");
        assert!(
            state.seitan.is_verified(&uid),
            "AccountVerified must land in the directory before the 200"
        );
    }

    #[tokio::test]
    async fn seitan_admit_refuses_a_stale_proof_through_the_live_path() {
        // (b) Under a 1-minute configured window, a proof signed 5
        // minutes before the invite was created refuses `stale`.
        let seitan = crate::SeitanState::new(
            id_directory::directory::Directory::new(),
            std::time::Duration::from_mins(1),
        );
        let (app, _state, _dir, _endpoint, _docs) = seitan_app(seitan, true).await;
        let token = id_directory::seitan::Token::generate();
        let registered = seitan_register(&app, token.as_str(), Some("s3cret")).await;
        assert_eq!(registered.status(), StatusCode::OK);
        let proof = id_directory::seitan::accept::sign(
            &token,
            &seitan_account_key(),
            1,
            seitan_now_ms().saturating_sub(300_000),
        )
        .unwrap();
        let refused = seitan_admit(&app, &proof).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(seitan_response_json(refused).await["reason"], "stale");
    }

    #[tokio::test]
    async fn seitan_admit_no_config_default_window_admits_a_fresh_proof() {
        // (c) The default state (no server config) admits a fresh
        // proof under the 5-minute default window.
        let (app, _state, _dir, _endpoint, _docs) =
            seitan_app(crate::SeitanState::default(), true).await;
        let token = id_directory::seitan::Token::generate();
        let registered = seitan_register(&app, token.as_str(), Some("s3cret")).await;
        assert_eq!(registered.status(), StatusCode::OK);
        let proof =
            id_directory::seitan::accept::sign(&token, &seitan_account_key(), 1, seitan_now_ms())
                .unwrap();
        let admitted = seitan_admit(&app, &proof).await;
        assert_eq!(admitted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn seitan_admit_unknown_invite_is_404() {
        // (d) A proof for a token that was never registered names an
        // invite the registry does not hold.
        let (app, _state, _dir, _endpoint, _docs) =
            seitan_app(crate::SeitanState::default(), true).await;
        let token = id_directory::seitan::Token::generate();
        let proof =
            id_directory::seitan::accept::sign(&token, &seitan_account_key(), 1, seitan_now_ms())
                .unwrap();
        let refused = seitan_admit(&app, &proof).await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            seitan_response_json(refused).await["reason"],
            "unknown_invite"
        );
    }

    #[tokio::test]
    async fn seitan_admit_unknown_account_is_403_and_does_not_consume_the_invite() {
        // (e) The account check runs before `admit`: an unknown uid
        // refuses 403, and the same proof succeeds once the account
        // exists — the invite was not consumed by the refusal.
        let seitan = crate::SeitanState::default();
        let (app, state, _dir, _endpoint, _docs) = seitan_app(seitan.clone(), false).await;
        let token = id_directory::seitan::Token::generate();
        let registered = seitan_register(&app, token.as_str(), Some("s3cret")).await;
        assert_eq!(registered.status(), StatusCode::OK);
        let uid = seitan_account_key();
        let proof = id_directory::seitan::accept::sign(&token, &uid, 1, seitan_now_ms()).unwrap();
        let refused = seitan_admit(&app, &proof).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            seitan_response_json(refused).await["reason"],
            "unknown_account"
        );

        state.seitan.sign_up_keyed(&uid, "joiner").unwrap();
        let admitted = seitan_admit(&app, &proof).await;
        assert_eq!(admitted.status(), StatusCode::OK);
        assert!(state.seitan.is_verified(&uid));
    }

    #[tokio::test]
    async fn seitan_invite_registration_requires_the_admin_secret() {
        // Minting an invite without the admin secret (or with the
        // wrong one) is refused, like POST /api/world/invite.
        let (app, _state, _dir, _endpoint, _docs) =
            seitan_app(crate::SeitanState::default(), false).await;
        let token = id_directory::seitan::Token::generate();
        let no_secret = seitan_register(&app, token.as_str(), None).await;
        assert_eq!(no_secret.status(), StatusCode::UNAUTHORIZED);
        let wrong_secret = seitan_register(&app, token.as_str(), Some("wrong")).await;
        assert_eq!(wrong_secret.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn seitan_duplicate_registration_does_not_reset_single_use_state() {
        // Codex P2 on PR #8529: a retried registration after the
        // invite was redeemed must not replace the consumed invite
        // with a fresh one — the same proof refuses `used`, it does
        // not admit a second time.
        let (app, _state, _dir, _endpoint, _docs) =
            seitan_app(crate::SeitanState::default(), true).await;
        let token = id_directory::seitan::Token::generate();
        let registered = seitan_register(&app, token.as_str(), Some("s3cret")).await;
        assert_eq!(registered.status(), StatusCode::OK);
        let proof =
            id_directory::seitan::accept::sign(&token, &seitan_account_key(), 1, seitan_now_ms())
                .unwrap();
        let admitted = seitan_admit(&app, &proof).await;
        assert_eq!(admitted.status(), StatusCode::OK);

        let reregistered = seitan_register(&app, token.as_str(), Some("s3cret")).await;
        assert_eq!(reregistered.status(), StatusCode::OK);

        let refused = seitan_admit(&app, &proof).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(seitan_response_json(refused).await["reason"], "used");
    }

    #[tokio::test]
    async fn public_entry_point_honours_a_configured_binding() {
        // A host that has already read its app manifest selects the
        // binding through the public API alone (Codex P2 on PR #3894):
        // parse [interfaces.web] with the re-exported parser and build
        // the app router with the re-exported constructor.
        let (_, mut state, _dir, _endpoint, _docs) = router_with_state().await;
        // The world page handler 404s when no world is hosted; host
        // one so the assertions below measure routing (the configured
        // route registered, the default route absent), not hosting.
        state.world = Some(id_worlds::world_hub::WorldHub::single(
            id_worlds::world_session::WorldService::new(
                id_worlds::world::WorldHandle::spawn(
                    id_worlds::world::WorldCore::new(
                        "lobby",
                        id_worlds::world::WorldLimits::default(),
                    )
                    .unwrap(),
                ),
                None,
            ),
        ));
        let interface = crate::web_interface_from_app_toml(
            "[interfaces.web]\nmode = \"adapter\"\nroute = \"/arena\"\nsocket = \"/ws/arena\"\n",
        )
        .unwrap()
        .unwrap();
        let app = crate::create_router_for_app(state, &interface, WebSecurity::default());
        let arena = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/arena")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(arena.status(), axum::http::StatusCode::NOT_FOUND);
        let world = app
            .oneshot(
                Request::builder()
                    .uri("/world")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(world.status(), axum::http::StatusCode::NOT_FOUND);
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
        for bytes in [
            b"\x89PNG\r\n\x1a\nfirst-bytes".as_slice(),
            b"\x89PNG\r\n\x1a\nsecond-bytes".as_slice(),
        ] {
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
        for (image, bytes) in images.iter().zip([
            b"\x89PNG\r\n\x1a\nfirst-bytes".as_slice(),
            b"\x89PNG\r\n\x1a\nsecond-bytes".as_slice(),
        ]) {
            let name = image["name"].as_str().unwrap();
            let hash = image["hash"].as_str().unwrap();
            assert_eq!(
                upload_name_owner(&state.store, name)
                    .await
                    .unwrap()
                    .unwrap()
                    .to_string(),
                hash
            );
            assert!(
                listed
                    .iter()
                    .any(|entry| entry["name"] == name && entry["hash"] == hash)
            );
            let url = image["url"].as_str().unwrap();
            assert!(url.contains(&format!("filename={name}")));
            let (status, _, actual) = get_blob(router.clone(), url).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(actual, bytes);
            for (key, expected) in [
                (b"content-type".as_slice(), Some("image/png")),
                (b"created".as_slice(), None),
            ] {
                let tags = state
                    .tag_store
                    .get_by_key(&state.tag_store.global, name.as_bytes(), key)
                    .await
                    .unwrap();
                let value = tags
                    .first()
                    .and_then(|tag| tag.value.as_ref())
                    .and_then(|value| value.as_str())
                    .unwrap();
                if let Some(expected) = expected {
                    assert_eq!(value, expected);
                } else {
                    assert!(
                        value.parse::<u64>().is_ok(),
                        "invalid creation timestamp: {value}"
                    );
                }
            }
        }
    }

    async fn create_file(router: Router, name: &str) -> (StatusCode, String) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/new")
                    .header("content-type", "application/json")
                    .body(Body::from(format!("{{\"name\":\"{name}\"}}")))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    // #8407: creating a name that already holds a nonempty file must
    // conflict and leave the existing name binding untouched.
    #[tokio::test]
    async fn create_existing_name_conflicts_and_preserves_original() {
        let (router, state, _dir, _endpoint, _docs) = router_with_state().await;
        let (status, _first) = upload(
            router.clone(),
            multipart_body_named(
                "existing.png",
                "image/png",
                b"\x89PNG\r\n\x1a\noriginal content",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let owner_before = upload_name_owner(&state.store, "existing.png")
            .await
            .unwrap()
            .unwrap();
        let metadata_before = name_metadata(&state, "existing.png").await;
        assert!(
            metadata_before.iter().any(|(_, v)| v.is_some()),
            "upload must record metadata for the existing file"
        );
        let (status, _body) = create_file(router.clone(), "existing.png").await;
        assert_eq!(status, StatusCode::CONFLICT);
        let owner_after = upload_name_owner(&state.store, "existing.png")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner_before, owner_after);
        // #8407 acceptance: metadata is left unchanged too, not just
        // the name binding.
        assert_eq!(metadata_before, name_metadata(&state, "existing.png").await);
    }

    async fn name_metadata(state: &AppState, name: &str) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        for key in [
            b"created".as_slice(),
            b"modified".as_slice(),
            b"content-type".as_slice(),
        ] {
            let tags = state
                .tag_store
                .get_by_key(&state.tag_store.global, name.as_bytes(), key)
                .await
                .unwrap();
            let value = tags
                .first()
                .and_then(|tag| tag.value.as_ref())
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            out.push((String::from_utf8_lossy(key).into_owned(), value));
        }
        out
    }

    async fn save_file(state: AppState, name: &str, text: &str) -> (StatusCode, String) {
        let response = save_handler(
            State(state),
            Json(SaveRequest {
                doc_id: iroh_blobs::Hash::from_bytes([42; 32]).to_string(),
                name: name.to_owned(),
                doc: serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]}),
            }),
        )
        .await;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    // #8407: a create for a name a save already claimed conflicts and
    // leaves the saved binding and metadata intact (create vs another
    // writer, sequential form).
    #[tokio::test]
    async fn create_after_save_conflicts_and_preserves_saved_file() {
        let (router, state, _dir, _endpoint, _docs) = router_with_state().await;
        let (status, body) = save_file(state.clone(), "saved.md", "saved text").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let saved_hash = upload_name_owner(&state.store, "saved.md")
            .await
            .unwrap()
            .unwrap();
        let metadata_before = name_metadata(&state, "saved.md").await;
        let (status, _body) = create_file(router.clone(), "saved.md").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            saved_hash,
            upload_name_owner(&state.store, "saved.md")
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(metadata_before, name_metadata(&state, "saved.md").await);
    }

    // #8407: a create racing a save of the same new name can never be
    // the final binding: whichever claim lands first, the saver's
    // explicit content is what the name resolves to afterwards, and
    // the create either conflicts or is superseded by that save --
    // the empty create blob never silently replaces saved content.
    #[tokio::test]
    async fn concurrent_create_and_save_leave_saved_content_bound() {
        let (router, state, _dir, _endpoint, _docs) = router_with_state().await;
        let guard = state.upload_name_lock.lock().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let create_task = {
            let router = router.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                create_file(router, "raced.md").await
            })
        };
        let save_task = {
            let state = state.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                save_file(state, "raced.md", "raced saved text").await
            })
        };
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(!create_task.is_finished());
        assert!(!save_task.is_finished());
        drop(guard);
        let (create_status, _body) = create_task.await.unwrap();
        assert!(
            create_status == StatusCode::OK || create_status == StatusCode::CONFLICT,
            "unexpected create status: {create_status}"
        );
        let (save_status, save_body) = save_task.await.unwrap();
        assert_eq!(save_status, StatusCode::OK, "{save_body}");
        let saved_hash: String =
            serde_json::from_str::<serde_json::Value>(&save_body).unwrap()["hash"]
                .as_str()
                .unwrap()
                .to_owned();
        let final_owner = upload_name_owner(&state.store, "raced.md")
            .await
            .unwrap()
            .unwrap()
            .to_string();
        assert_eq!(final_owner, saved_hash);
    }

    // #8407: a create for a genuinely new name still succeeds.
    #[tokio::test]
    async fn create_unique_name_succeeds() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, _body) = create_file(router, "fresh.txt").await;
        assert_eq!(status, StatusCode::OK);
    }

    // #8407: two concurrent creates of the same new name -- exactly one
    // wins; the other conflicts instead of replacing the winner's file.
    #[tokio::test]
    async fn concurrent_creates_same_name_exactly_one_wins() {
        let (router, state, _dir, _endpoint, _docs) = router_with_state().await;
        // Hold the shared name lock while both requests start; neither
        // may pass its claim boundary until it is released.
        let guard = state.upload_name_lock.lock().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let router = router.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                create_file(router, "raced.txt").await
            }));
        }
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(tasks.iter().all(|task| !task.is_finished()));
        drop(guard);
        let mut oks = 0;
        let mut conflicts = 0;
        for task in tasks {
            match task.await.unwrap().0 {
                StatusCode::OK => oks += 1,
                StatusCode::CONFLICT => conflicts += 1,
                other => panic!("unexpected create status: {other}"),
            }
        }
        assert_eq!((oks, conflicts), (1, 1));
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
            multipart_body_named("image.png", "image/png", b"\x89PNG\r\n\x1a\nfirst-bytes"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {first}");
        assert!(first.contains("\"name\":\"image.png\""), "body: {first}");
        let first_hash = uploaded_hash(&first);

        let (status, second) = upload(
            router.clone(),
            multipart_body_named("image.png", "image/png", b"\x89PNG\r\n\x1a\nsecond-bytes"),
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
            multipart_body_named("image.png", "image/png", b"\x89PNG\r\n\x1a\nfirst-bytes"),
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
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\nfirst-bytes");
        let (_, _, bytes) = get_blob(router, &format!("/blob/{second_hash}")).await;
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\nsecond-bytes");
    }

    // #1549: the nameless paste-{secs} fallback passes through the
    // same uniquifier, so two nameless uploads in the same second can
    // never land on the same stored name.
    #[tokio::test]
    async fn upload_nameless_uploads_never_share_a_stored_name() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, first) = upload(
            router.clone(),
            multipart_body_noname("image/png", b"\x89PNG\r\n\x1a\nnameless-one"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {first}");
        let (status, second) = upload(
            router.clone(),
            multipart_body_noname("image/png", b"\x89PNG\r\n\x1a\nnameless-two"),
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
        let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
        file.resize(MAX_IMAGE_SIZE, b'y');
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
        let (status, body) = upload(
            router.clone(),
            multipart_body(b"\x89PNG\r\n\x1a\npng-bytes"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let hash = uploaded_hash(&body);
        let (status, headers, bytes) =
            get_blob(router, &format!("/blob/{hash}?filename=evil.html")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\npng-bytes");
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
        let (status, body) = upload(
            router.clone(),
            multipart_body(b"\x89PNG\r\n\x1a\npng-bytes"),
        )
        .await;
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
            multipart_body_named("evil.html", "image/png", b"\x89PNG\r\n\x1a\npng-bytes"),
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

    #[test]
    fn image_bytes_match_declared_type_accepts_real_headers_and_rejects_masquerades() {
        assert!(image_bytes_match_declared_type(
            "image/png",
            b"\x89PNG\r\n\x1a\nrest"
        ));
        assert!(image_bytes_match_declared_type(
            "image/jpeg",
            b"\xff\xd8\xff\xe0rest"
        ));
        assert!(image_bytes_match_declared_type("image/gif", b"GIF87a-rest"));
        assert!(image_bytes_match_declared_type("image/gif", b"GIF89a-rest"));
        assert!(image_bytes_match_declared_type(
            "image/webp",
            b"RIFF\x04\x00\x00\x00WEBPrest"
        ));
        assert!(image_bytes_match_declared_type("image/bmp", b"BM-rest"));
        assert!(image_bytes_match_declared_type(
            "image/x-icon",
            b"\x00\x00\x01\x00rest"
        ));
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"
        ));
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"\xef\xbb\xbf  <?xml version=\"1.0\"?>\n<svg/>"
        ));
        // Codex P2 on PR #5357: legal prolog constructs — a DOCTYPE and
        // comments, as in doc/database_structure_20231217_074440.svg —
        // precede the root element of valid SVGs and must be accepted.
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"\n \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<!-- Generated by graphviz -->\n<!-- Pages: 1 -->\n<svg width=\"994pt\"></svg>"
        ));
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"<!DOCTYPE svg [ <!ENTITY x \"a>b\"> ]>\n<svg/>"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/svg+xml",
            b"<!-- only a comment, no root -->"
        ));
        // Codex P2 on PR #5357 (second round): a namespace-prefixed
        // root is still an SVG root; lookalike tag names are not.
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"<s:svg xmlns:s=\"http://www.w3.org/2000/svg\"/>"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/svg+xml",
            b"<svgx></svgx>"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/svg+xml",
            b"<x:html></x:html>"
        ));
        // Codex P2 on PR #5357 (round 3): UTF-16-encoded SVGs are valid
        // image/svg+xml documents; LE and BE, with and without a BOM,
        // are decoded before the prolog/root check.
        fn utf16(text: &str, little_endian: bool, bom: bool) -> Vec<u8> {
            let mut out = Vec::new();
            if bom {
                out.extend_from_slice(if little_endian {
                    b"\xff\xfe"
                } else {
                    b"\xfe\xff"
                });
            }
            for unit in text.encode_utf16() {
                out.extend_from_slice(&if little_endian {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            out
        }
        let svg_doc = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            &utf16(svg_doc, true, true)
        ));
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            &utf16(svg_doc, false, true)
        ));
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            &utf16("<svg/>", true, false)
        ));
        assert!(!image_bytes_match_declared_type(
            "image/svg+xml",
            &utf16("<html></html>", true, true)
        ));
        // Codex P2 on PR #5357 (round 4): brackets inside quoted
        // literals or comments within the internal subset are data,
        // not subset delimiters.
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"<!DOCTYPE svg [<!ENTITY label \"[\">]><svg xmlns=\"http://www.w3.org/2000/svg\"/>"
        ));
        assert!(image_bytes_match_declared_type(
            "image/svg+xml",
            b"<!DOCTYPE svg [ <!-- [ ] --> <!ENTITY x 'a]b'> ]>\n<svg/>"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/svg+xml",
            b"<!DOCTYPE svg [<!ENTITY label \"[\">]>"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/png",
            b"<html><script>alert(1)</script></html>"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/jpeg",
            b"plain text"
        ));
        assert!(!image_bytes_match_declared_type(
            "image/svg+xml",
            b"<html></html>"
        ));
        assert!(!image_bytes_match_declared_type("image/png", b""));
        assert!(!image_bytes_match_declared_type(
            "image/webp",
            b"RIFF\x04\x00\x00\x00WAVErest"
        ));
    }

    // #5342 (the probe that filed the issue): HTML/script bytes declared
    // image/png were accepted, stored, and served as a PNG. They must
    // now be refused 400, and nothing may be stored or listed.
    #[tokio::test]
    async fn upload_html_declared_as_png_is_refused_and_stores_nothing() {
        let (router, _dir, _endpoint, _docs) = router().await;
        let (status, body) = upload(
            router.clone(),
            multipart_body_named(
                "evil.png",
                "image/png",
                b"<html><script>alert(1)</script></html>",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert!(
            body.contains("do not match the declared image type"),
            "body: {body}"
        );
        assert!(list_images(router).await.is_empty());
    }

    #[tokio::test]
    async fn upload_mismatched_bytes_are_refused_for_other_declared_types() {
        let (router, _dir, _endpoint, _docs) = router().await;
        for (name, content_type, bytes) in [
            ("photo.jpg", "image/jpeg", b"plain text".as_slice()),
            ("icon.svg", "image/svg+xml", b"not an svg".as_slice()),
            (
                "anim.gif",
                "image/gif",
                b"\x89PNG\r\n\x1a\nnot-a-gif".as_slice(),
            ),
            (
                "pic.webp",
                "image/webp",
                b"RIFF\x04\x00\x00\x00WAVErest".as_slice(),
            ),
        ] {
            let (status, body) = upload(
                router.clone(),
                multipart_body_named(name, content_type, bytes),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{content_type}: {body}");
        }
        assert!(list_images(router).await.is_empty());
    }

    #[tokio::test]
    async fn upload_real_headers_for_each_allowed_type_are_accepted() {
        let (router, _dir, _endpoint, _docs) = router().await;
        for (name, content_type, bytes) in [
            ("a.png", "image/png", b"\x89PNG\r\n\x1a\nx".as_slice()),
            ("a.jpg", "image/jpeg", b"\xff\xd8\xff\xe0x".as_slice()),
            ("a.gif", "image/gif", b"GIF89ax".as_slice()),
            (
                "a.webp",
                "image/webp",
                b"RIFF\x04\x00\x00\x00WEBPx".as_slice(),
            ),
            ("a.bmp", "image/bmp", b"BMx".as_slice()),
            ("a.ico", "image/x-icon", b"\x00\x00\x01\x00x".as_slice()),
            (
                "a.svg",
                "image/svg+xml",
                b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>".as_slice(),
            ),
        ] {
            let (status, body) = upload(
                router.clone(),
                multipart_body_named(name, content_type, bytes),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{content_type}: {body}");
        }
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
            // #1766: at the admission bound the refusal is a clear,
            // observable 503 — not a generic 500 — and no existing
            // identity is affected by it.
            if let Some(cap) = err.downcast_ref::<super::identity::IdentityCapacityError>() {
                tracing::warn!("[identity] Registration refused at capacity: {cap}");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "error": "identity_capacity_reached",
                        "detail": cap.to_string(),
                        "limit": cap.limit,
                        "registered": cap.registered,
                    })),
                )
                    .into_response();
            }
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
    use proptest::prelude::*;

    #[test]
    fn file_list_backend_failures_map_to_server_errors() {
        for failure in [
            FileListError::MetadataTags,
            FileListError::TagList,
            FileListError::SearchTags,
        ] {
            let result = checked_file_list_result::<(), _>(Err("injected"), failure);
            assert_eq!(result, Err(failure));
            assert_eq!(
                file_list_error_response(failure).status(),
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
    }

    #[tokio::test]
    async fn file_list_stream_failure_does_not_return_partial_success() {
        let mut tags = futures_lite::stream::iter([
            Ok::<_, &'static str>("first tag"),
            Err("injected stream failure"),
        ]);

        assert_eq!(next_tag_info(&mut tags).await, Ok(Some("first tag")));
        assert_eq!(
            next_tag_info(&mut tags).await,
            Err(FileListError::TagStream)
        );
    }

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
    fn test_extract_plain_text_front_matter_renders_verbatim() {
        // #5639: a front_matter block's payload is its text attr
        // (#1369 node shape); extraction must keep it, fences and
        // all, at its document position - not drop the block whole.
        let doc = plain_doc(vec![
            serde_json::json!({"type": "front_matter", "attrs": {"text": "---\ntitle: Hello\n---\n"}}),
            para("Body"),
        ]);
        assert_eq!(extract_plain_text(&doc), "---\ntitle: Hello\n---\nBody");
        // Front matter as the only block.
        let doc = plain_doc(vec![
            serde_json::json!({"type": "front_matter", "attrs": {"text": "---\ntitle: Hello\n---\n"}}),
        ]);
        assert_eq!(extract_plain_text(&doc), "---\ntitle: Hello\n---");
    }

    #[test]
    fn test_extract_plain_text_front_matter_missing_or_empty_text_is_nothing() {
        // #5639: no attrs, no text attr, or an empty one contributes
        // nothing and does not panic (the #1491 empty-payload rule).
        for fm in [
            serde_json::json!({"type": "front_matter"}),
            serde_json::json!({"type": "front_matter", "attrs": {}}),
            serde_json::json!({"type": "front_matter", "attrs": {"text": ""}}),
        ] {
            let doc = plain_doc(vec![fm, para("Body")]);
            assert_eq!(extract_plain_text(&doc), "Body");
        }
    }

    #[test]
    fn test_plain_mode_roundtrip_preserves_trailing_newline() {
        // #1307 pinned policy: the real load/save pair is byte-stable
        // for the common shapes, including the trailing newline.
        for text in ["a\nb\n", "a\nb", "a\n", "\n", ""] {
            let doc = super::super::markdown::plain_text_to_prosemirror(text);
            assert_eq!(extract_plain_text(&doc), text, "{text:?}");
        }
    }

    #[test]
    fn test_plain_mode_roundtrip_normalizes_crlf_to_lf() {
        // #1307 pinned policy: CRLF normalizes to LF on save, and the
        // trailing-newline state survives the normalization.
        let doc = super::super::markdown::plain_text_to_prosemirror("alpha\r\nbeta\r\n");
        assert_eq!(extract_plain_text(&doc), "alpha\nbeta\n");
        let doc = super::super::markdown::plain_text_to_prosemirror("alpha\r\nbeta");
        assert_eq!(extract_plain_text(&doc), "alpha\nbeta");
    }

    #[test]
    fn test_plain_mode_roundtrip_preserves_lone_terminal_cr() {
        // Codex finding on PR #4613: a carriage return that is NOT part
        // of a CRLF ending is content. Only segments terminated by '\n'
        // may have their CR stripped; the final segment keeps it.
        let doc = super::super::markdown::plain_text_to_prosemirror("alpha\r");
        assert_eq!(extract_plain_text(&doc), "alpha\r");
        let doc = super::super::markdown::plain_text_to_prosemirror("alpha\r\nbeta\r");
        assert_eq!(extract_plain_text(&doc), "alpha\nbeta\r");
        // A CR-only line ending mid-file is likewise preserved.
        let doc = super::super::markdown::plain_text_to_prosemirror("alpha\rbeta\n");
        assert_eq!(extract_plain_text(&doc), "alpha\rbeta\n");
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

    fn footnote_reference(label: &str) -> serde_json::Value {
        serde_json::json!({"type": "footnote_reference", "attrs": {"label": label}})
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
    fn test_extract_plain_text_footnote_reference_between_text_runs() {
        let doc = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
            {"type": "text", "text": "before "},
            footnote_reference("note:α"),
            {"type": "text", "text": " after"}
        ]})]);
        assert_eq!(extract_plain_text(&doc), "before [^note:α] after");
    }

    #[test]
    fn test_extract_plain_text_footnote_reference_in_recursive_container() {
        let doc = plain_doc(vec![serde_json::json!({"type": "bullet_list", "content": [
            {"type": "list_item", "content": [serde_json::json!({"type": "paragraph", "content": [
                {"type": "text", "text": "listed "},
                footnote_reference("1"),
                {"type": "text", "text": " text"}
            ]})]}
        ]})]);
        assert_eq!(extract_plain_text(&doc), "listed [^1] text");
    }

    #[test]
    fn test_extract_plain_text_footnote_reference_missing_or_empty_label_is_nothing() {
        for reference in [
            serde_json::json!({"type": "footnote_reference"}),
            serde_json::json!({"type": "footnote_reference", "attrs": {}}),
            serde_json::json!({"type": "footnote_reference", "attrs": {"label": ""}}),
        ] {
            let doc = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
                {"type": "text", "text": "x"},
                reference,
                {"type": "text", "text": "y"}
            ]})]);
            assert_eq!(extract_plain_text(&doc), "xy");
        }
    }

    // Oracle for the two proptests below (#5366, Codex P1 on PR #6427):
    // plain-text serialization concatenates inline text runs verbatim, so
    // a footnote_reference with a non-empty label L between runs B and A
    // must serialize to exactly B + "[^" + L + "]" + A, at any nesting
    // depth — the implementation only ever push_str's the label between
    // the fixed "[^" and "]" markers.
    proptest! {
        #[test]
        fn prop_extract_plain_text_footnote_reference_label_verbatim(
            before in "\\PC{0,24}",
            label in "\\PC{1,24}",
            after in "\\PC{0,24}",
        ) {
            let doc = plain_doc(vec![serde_json::json!({"type": "paragraph", "content": [
                {"type": "text", "text": before.clone()},
                footnote_reference(&label),
                {"type": "text", "text": after.clone()}
            ]})]);
            prop_assert_eq!(
                extract_plain_text(&doc),
                format!("{before}[^{label}]{after}")
            );
        }

        #[test]
        fn prop_extract_plain_text_footnote_reference_in_recursive_container(
            before in "\\PC{0,24}",
            label in "\\PC{1,24}",
            after in "\\PC{0,24}",
        ) {
            let doc = plain_doc(vec![serde_json::json!({"type": "bullet_list", "content": [
                {"type": "list_item", "content": [serde_json::json!({"type": "paragraph", "content": [
                    {"type": "text", "text": before.clone()},
                    footnote_reference(&label),
                    {"type": "text", "text": after.clone()}
                ]})]}
            ]})]);
            prop_assert_eq!(
                extract_plain_text(&doc),
                format!("{before}[^{label}]{after}")
            );
        }
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
            // Regression (#1879): a near-miss timestamp with a junk date
            // suffix used to parse as valid and classify as Auto.
            "auto-2026-03-12-draftT06:42:30",
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
        let ts = parse_iso_timestamp("2026-03-12T06:42:30.015Z");
        assert!(ts.is_some());
        let t = ts.unwrap();
        // 2026-03-12 is after 2025-01-01 (approx 1735689600)
        assert!(t > 1_735_689_600);
    }

    #[test]
    fn test_parse_iso_timestamp_invalid() {
        assert!(parse_iso_timestamp("not-a-date").is_none());
        assert!(parse_iso_timestamp("").is_none());
        assert!(parse_iso_timestamp("noprefix").is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_rejects_malformed_components() {
        // Regression (#1879): a non-numeric component used to be dropped,
        // so malformed timestamps parsed as the valid instant they
        // surrounded. The valid control is 2026-03-12T06:42:30Z.
        assert_eq!(
            parse_iso_timestamp("2026-03-12T06:42:30Z"),
            Some(1_773_297_750)
        );
        // Junk suffix on the date side.
        assert!(parse_iso_timestamp("2026-03-12-draftT06:42:30").is_none());
        // Junk components on the time side.
        assert!(parse_iso_timestamp("2026-03-12T06:xx:30").is_none());
        assert!(parse_iso_timestamp("2026-03-12T06:42:xx:30").is_none());
        // Extra components, numeric or not (the count check alone is not
        // enough when a bad segment can be dropped to reach three).
        assert!(parse_iso_timestamp("2026-03-12-99T06:42:30").is_none());
        assert!(parse_iso_timestamp("2026-03-12T06:42:30:30").is_none());
        // Missing components.
        assert!(parse_iso_timestamp("2026-03T06:42:30").is_none());
        assert!(parse_iso_timestamp("2026-03-12T06:42").is_none());
        // Junk fractional seconds.
        assert!(parse_iso_timestamp("2026-03-12T06:42:30.abcZ").is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_rejects_noncanonical_spellings() {
        // Regression (#4329): only the canonical fixed-width spellings
        // classify; non-padded, over-wide, and '+'-signed components
        // used to parse onto the canonical instant (control:
        // 2026-03-12T06:42:30Z is Some(1_773_297_750)).
        for s in [
            "2026-3-12T6:42:30Z",
            "2026-03-12T06:4:2Z",
            "2026-3-2T6:4:2Z",
            "+2026-03-12T06:42:30Z",
            "02026-03-12T06:42:30Z",
            "2026-003-12T06:42:30Z",
            "2026-03-12T+06:42:30Z",
            "2026-03-12T06:42:3Z",
        ] {
            assert!(parse_iso_timestamp(s).is_none(), "{s}");
        }
    }

    #[test]
    fn test_classify_tag_noncanonical_spellings_are_primary() {
        // Regression (#4329): near-miss spellings of the producer-shaped
        // names are ordinary files, not auto-backups or archives.
        for name in [
            "auto-2026-3-12T6:42:30Z",
            "auto-+2026-03-12T06:42:30Z",
            "auto-02026-03-12T06:42:30Z",
            "notes.archive.+123",
        ] {
            let (kind, ts) = classify_tag(name);
            assert_eq!(kind, FileKind::Primary, "{name}");
            assert!(ts.is_none(), "{name}");
        }
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
        let ts = parse_iso_timestamp("1970-01-01T00:00:00.000Z");
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

    fn resolve_display_name(tags: &[(&str, &str)]) -> Option<String> {
        let mut display_name_map = std::collections::HashMap::new();
        for (key, value) in tags {
            record_display_name(
                &mut display_name_map,
                "document.md".to_owned(),
                key.as_bytes(),
                Some(value),
            );
        }
        display_name_map
            .remove("document.md")
            .map(|(_, display_name)| display_name)
    }

    #[test]
    fn test_display_name_priority_is_independent_of_tag_order() {
        // Alpha order sorts the `file` key before `name` and `path`.
        assert_eq!(
            resolve_display_name(&[
                ("file", "basename.md"),
                ("name", "Friendly title"),
                ("path", "/source/basename.md"),
            ]),
            Some("Friendly title".to_owned())
        );
        assert_eq!(
            resolve_display_name(&[("file", "basename.md"), ("path", "/source/basename.md")]),
            Some("basename.md".to_owned())
        );
        assert_eq!(
            resolve_display_name(&[("path", "/source/basename.md")]),
            Some("/source/basename.md".to_owned())
        );

        // A single identity tag continues to supply the display name.
        for (key, value) in [
            ("name", "Friendly title"),
            ("file", "basename.md"),
            ("path", "/source/basename.md"),
        ] {
            assert_eq!(
                resolve_display_name(&[(key, value)]),
                Some(value.to_owned())
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod collab_route_tests {
    //! Route-level proof for #2830: the production router registers
    //! `/ws/collab/{doc_id}` as a single-segment route (see
    //! `create_router`); the web client percent-encodes the docId into
    //! that one segment, and axum matches the raw path, so `%2F` stays
    //! inside the segment and Path extraction percent-decodes it back
    //! to the exact filename — while a real second segment still
    //! fails to match. This mini-router uses the identical route
    //! pattern on the same axum stack; the handler is an echo so the
    //! decoded value itself is asserted. (Behavior verified against
    //! the locked axum 0.8.9 with a standalone probe on 2026-11-02;
    //! in-crate execution currently waits on #4340.)
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::Path,
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::ServiceExt as _;

    async fn echo_doc_id(Path(doc_id): Path<String>) -> String {
        doc_id
    }

    async fn get_doc(uri: &str) -> (StatusCode, String) {
        let app = Router::new().route("/ws/collab/{doc_id}", get(echo_doc_id));
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn encoded_slash_round_trips_to_the_exact_filename() {
        assert_eq!(
            get_doc("/ws/collab/a%2Fb.md").await,
            (StatusCode::OK, "a/b.md".to_owned())
        );
    }

    #[tokio::test]
    async fn encoded_percent_names_round_trip() {
        // a%20b.md the file, not "a b.md": the client sends a%2520b.md.
        assert_eq!(
            get_doc("/ws/collab/a%2520b.md").await,
            (StatusCode::OK, "a%20b.md".to_owned())
        );
        assert_eq!(
            get_doc("/ws/collab/100%25.md").await,
            (StatusCode::OK, "100%.md".to_owned())
        );
        assert_eq!(
            get_doc("/ws/collab/notes.md").await,
            (StatusCode::OK, "notes.md".to_owned())
        );
    }

    #[tokio::test]
    async fn a_real_second_segment_does_not_match() {
        assert_eq!(get_doc("/ws/collab/a/b.md").await.0, StatusCode::NOT_FOUND);
    }
}
