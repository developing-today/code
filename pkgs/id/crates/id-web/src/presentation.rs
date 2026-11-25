//! App presentation assets and bundles (#10392 §4.6; slice 5, #14228).
//!
//! An app's web version is the client-side files its manifest declares
//! ([`RunManifest::client_side`], from `interfaces.web.assets` in
//! `app.toml`). The ID binary serves them three ways:
//!
//! - **Assets**: `GET /v1/apps/{app}/assets/{path}` serves one declared
//!   file with the content type of its extension. The app's manifest
//!   (`GET /v1/apps/{app}/manifest`) lists each asset with its URL, which
//!   carries the file's content hash (`?v=<hash>`). A request naming the
//!   current hash is answered as immutable; a request naming another hash
//!   is refused as stale, so a cached page never mixes versions.
//! - **Bundle**: `GET /v1/apps/{app}/bundle` answers the whole web version
//!   as one archive: `.tar.zst` by default, or `.zip` with `?format=zip`.
//!   It holds every asset and `manifest.json`. The archives are written
//!   here without compression (zstd raw blocks, zip stored entries), so
//!   any standard tool reads them and no codec is needed.
//! - **Single file**: `GET /v1/apps/{app}/bundle.html` answers the entry
//!   page (`index.html` when declared, else the first `.html` asset) with
//!   its scripts and stylesheets inlined, other referenced assets as
//!   `data:` URLs, and every other page as a
//!   `<template data-page="...">` that client script switches to locally.
//!
//! Whether an app pages by request or in one file is its manifest's
//! `pages` choice ([`Pages`]). Only declared files are served, and each is
//! resolved inside the app folder at read time, so a symlink cannot reach
//! outside it. Like runs, these routes answer loopback clients only (§4.7).

use std::fmt::Write as _;
use std::path::Path as FsPath;
use std::sync::Arc;

use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::runs::{ConnectInfoPeer, RunManifest, RunsConfig, error_response, peer_allowed};

/// Serve one asset: `GET /v1/apps/{app}/assets/{path}`.
pub const ASSET_PATH: &str = "/v1/apps/{app}/assets/{*path}";
/// The whole web version as one archive: `GET /v1/apps/{app}/bundle`.
pub const BUNDLE_PATH: &str = "/v1/apps/{app}/bundle";
/// The web version as one HTML file: `GET /v1/apps/{app}/bundle.html`.
pub const BUNDLE_HTML_PATH: &str = "/v1/apps/{app}/bundle.html";

/// Hex digits of the content hash carried in asset URLs.
const HASH_LEN: usize = 16;

/// How a multi-page app's pages reach the client (§4.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pages {
    /// One request per page (the default).
    #[default]
    Multi,
    /// One HTML file; client script switches pages locally.
    Single,
}

/// One declared asset as the manifest lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssetEntry {
    /// The path inside the app folder.
    pub path: String,
    /// The URL that serves it, with its content hash.
    pub url: String,
    /// The content type it is served with.
    pub content_type: String,
    /// The first hex digits of its SHA-256.
    pub hash: String,
}

/// One declared asset with its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// The listed entry.
    pub entry: AssetEntry,
    /// The file's bytes.
    pub bytes: Vec<u8>,
}

/// The content type served for `path`, from its extension.
#[must_use]
pub fn content_type(path: &str) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_owned()
}

fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().take(HASH_LEN / 2).fold(
        String::with_capacity(HASH_LEN),
        |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        },
    )
}

/// Read one declared asset of `manifest`, resolved inside its app folder.
///
/// # Errors
///
/// Fails when the app has no folder, the path is not declared, or the
/// file cannot be read or resolves outside the folder.
pub fn read_asset(manifest: &RunManifest, path: &str) -> std::io::Result<Asset> {
    if !manifest.client_side.iter().any(|declared| declared == path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{path:?} is not a declared asset"),
        ));
    }
    let root = manifest
        .root
        .as_deref()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "app has no folder"))?;
    let bytes = read_confined(root, path)?;
    let hash = content_hash(&bytes);
    Ok(Asset {
        entry: AssetEntry {
            path: path.to_owned(),
            url: format!("/v1/apps/{}/assets/{path}?v={hash}", manifest.app),
            content_type: content_type(path),
            hash,
        },
        bytes,
    })
}

fn read_confined(root: &FsPath, path: &str) -> std::io::Result<Vec<u8>> {
    let root = root.canonicalize()?;
    let file = root.join(path).canonicalize()?;
    if !file.starts_with(&root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{path:?} resolves outside the app folder"),
        ));
    }
    std::fs::read(file)
}

/// Read every declared asset of `manifest`, in manifest order.
///
/// # Errors
///
/// Fails when any asset cannot be read.
pub fn read_assets(manifest: &RunManifest) -> std::io::Result<Vec<Asset>> {
    manifest
        .client_side
        .iter()
        .map(|path| read_asset(manifest, path))
        .collect()
}

/// The manifest as served, with its assets listed when the app has a
/// folder to serve them from.
///
/// # Errors
///
/// Fails when an asset cannot be read.
pub fn manifest_document(manifest: &RunManifest) -> std::io::Result<serde_json::Value> {
    let mut document = serde_json::to_value(manifest).map_err(std::io::Error::other)?;
    // A manifest with no app folder (one built in code) serves no files.
    if manifest.root.is_none() {
        return Ok(document);
    }
    let assets: Vec<AssetEntry> = read_assets(manifest)?
        .into_iter()
        .map(|asset| asset.entry)
        .collect();
    if let Some(object) = document.as_object_mut() {
        object.insert(
            "assets".to_owned(),
            serde_json::to_value(assets).map_err(std::io::Error::other)?,
        );
    }
    Ok(document)
}

/// The presentation routes, served with the runs router's state.
pub(crate) fn presentation_routes() -> Router<Arc<RunsConfig>> {
    Router::new()
        .route(ASSET_PATH, get(asset_handler))
        .route(BUNDLE_PATH, get(bundle_handler))
        .route(BUNDLE_HTML_PATH, get(bundle_html_handler))
}

fn refuse_non_local() -> Response {
    error_response(
        StatusCode::FORBIDDEN,
        "app assets are served to loopback clients only (#10392 §4.7)",
    )
}

fn io_response(err: &std::io::Error) -> Response {
    let status = match err.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
        std::io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, err.to_string())
}

/// The app's manifest, or the answer that refuses the request.
// The refusal is the answer itself, so it is returned whole.
#[allow(clippy::result_large_err)]
fn app_manifest<'a>(
    config: &'a RunsConfig,
    app: &str,
    peer: Option<std::net::SocketAddr>,
    headers: &HeaderMap,
) -> Result<&'a RunManifest, Response> {
    if !peer_allowed(peer, headers) {
        return Err(refuse_non_local());
    }
    config
        .apps
        .get(app)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, format!("unknown app {app:?}")))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetParams {
    v: Option<String>,
}

async fn asset_handler(
    State(config): State<Arc<RunsConfig>>,
    Path((app, path)): Path<(String, String)>,
    peer: ConnectInfoPeer,
    headers: HeaderMap,
    params: Result<Query<AssetParams>, QueryRejection>,
) -> Response {
    let manifest = match app_manifest(&config, &app, peer.0, &headers) {
        Ok(manifest) => manifest,
        Err(refusal) => return refusal,
    };
    let Ok(Query(params)) = params else {
        return error_response(StatusCode::BAD_REQUEST, "query takes v (the content hash)");
    };
    let asset = match read_asset(manifest, &path) {
        Ok(asset) => asset,
        Err(err) => return io_response(&err),
    };
    let cache = match params.v.as_deref() {
        Some(hash) if hash == asset.entry.hash => "public, max-age=31536000, immutable",
        Some(_) => {
            return error_response(
                StatusCode::NOT_FOUND,
                format!(
                    "{path:?} has changed; its current URL is {}",
                    asset.entry.url
                ),
            );
        }
        None => "no-cache",
    };
    let mut response = asset.bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&asset.entry.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", asset.entry.hash)) {
        headers.insert(header::ETAG, value);
    }
    response
}

/// Which archive a bundle request asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum BundleFormat {
    /// A tar archive in a zstd frame (the default).
    #[default]
    #[serde(rename = "tar.zst")]
    TarZst,
    /// A zip archive.
    #[serde(rename = "zip")]
    Zip,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleParams {
    #[serde(default)]
    format: BundleFormat,
}

/// The files of an app's bundle: every asset, then `manifest.json`.
///
/// # Errors
///
/// Fails when an asset cannot be read.
pub fn bundle_files(manifest: &RunManifest) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut files: Vec<(String, Vec<u8>)> = read_assets(manifest)?
        .into_iter()
        .map(|asset| (asset.entry.path, asset.bytes))
        .collect();
    let document = manifest_document(manifest)?;
    let json = serde_json::to_vec_pretty(&document).map_err(std::io::Error::other)?;
    files.push(("manifest.json".to_owned(), json));
    Ok(files)
}

/// Build an app's bundle in `format`.
///
/// # Errors
///
/// Fails when an asset cannot be read or the archive cannot hold the
/// files.
pub fn build_bundle(manifest: &RunManifest, format: BundleFormat) -> std::io::Result<Vec<u8>> {
    let files = bundle_files(manifest)?;
    match format {
        BundleFormat::TarZst => Ok(zstd_raw(&tar(&files)?)),
        BundleFormat::Zip => zip(&files),
    }
}

async fn bundle_handler(
    State(config): State<Arc<RunsConfig>>,
    Path(app): Path<String>,
    peer: ConnectInfoPeer,
    headers: HeaderMap,
    params: Result<Query<BundleParams>, QueryRejection>,
) -> Response {
    let manifest = match app_manifest(&config, &app, peer.0, &headers) {
        Ok(manifest) => manifest,
        Err(refusal) => return refusal,
    };
    let Ok(Query(params)) = params else {
        return error_response(StatusCode::BAD_REQUEST, "format is tar.zst or zip");
    };
    let bytes = match build_bundle(manifest, params.format) {
        Ok(bytes) => bytes,
        Err(err) => return io_response(&err),
    };
    let (content_type, extension) = match params.format {
        BundleFormat::TarZst => ("application/zstd", "tar.zst"),
        BundleFormat::Zip => ("application/zip", "zip"),
    };
    let disposition = format!(
        "attachment; filename=\"{}-{}.{extension}\"",
        manifest.app, manifest.version
    );
    let mut response = bytes.into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        response_headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

async fn bundle_html_handler(
    State(config): State<Arc<RunsConfig>>,
    Path(app): Path<String>,
    peer: ConnectInfoPeer,
    headers: HeaderMap,
) -> Response {
    let manifest = match app_manifest(&config, &app, peer.0, &headers) {
        Ok(manifest) => manifest,
        Err(refusal) => return refusal,
    };
    let assets = match read_assets(manifest) {
        Ok(assets) => assets,
        Err(err) => return io_response(&err),
    };
    match single_file(&assets) {
        Some(html) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            format!("app {app:?} declares no UTF-8 .html asset to bundle"),
        ),
    }
}

fn is_html(path: &str) -> bool {
    FsPath::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
}

/// The single-file HTML bundle (§4.6).
///
/// The entry page (`index.html` when declared, else the first `.html`
/// asset) has its scripts and stylesheets inlined, other referenced assets
/// as `data:` URLs, and every other page as a `<template data-page="...">`
/// before `</body>`. `None` when there is no UTF-8 HTML asset.
#[must_use]
pub fn single_file(assets: &[Asset]) -> Option<String> {
    let entry = assets
        .iter()
        .find(|asset| asset.entry.path == "index.html")
        .or_else(|| assets.iter().find(|asset| is_html(&asset.entry.path)))?;
    let mut html = inline_page(std::str::from_utf8(&entry.bytes).ok()?, assets);
    let mut templates = String::new();
    for page in assets
        .iter()
        .filter(|asset| is_html(&asset.entry.path) && asset.entry.path != entry.entry.path)
    {
        let Ok(text) = std::str::from_utf8(&page.bytes) else {
            continue;
        };
        let _ = writeln!(
            templates,
            "<template data-page=\"{}\">{}</template>",
            attribute_escape(&page.entry.path),
            inline_page(text, assets)
        );
    }
    match html.rfind("</body>") {
        Some(at) => html.insert_str(at, &templates),
        None => html.push_str(&templates),
    }
    Some(html)
}

fn attribute_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// The declared asset a reference names, if any (`./` prefix allowed).
fn referenced<'a>(assets: &'a [Asset], reference: &str) -> Option<&'a Asset> {
    let reference = reference.strip_prefix("./").unwrap_or(reference);
    assets.iter().find(|asset| asset.entry.path == reference)
}

/// The value of attribute `name` in one tag's text.
fn attribute<'t>(tag: &'t str, name: &str) -> Option<&'t str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before_ok = at > 0 && lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let quote = rest.chars().next()?;
        if quote == '"' || quote == '\'' {
            return rest[1..].split(quote).next();
        }
        return rest
            .split(|c: char| c.is_ascii_whitespace() || c == '>')
            .next();
    }
    None
}

/// Inline one page's scripts and stylesheets and turn its other asset
/// references into `data:` URLs.
fn inline_page(page: &str, assets: &[Asset]) -> String {
    let mut out = String::with_capacity(page.len());
    let mut rest = page;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(close) = rest.find('>') else {
            break;
        };
        let tag = &rest[..=close];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<script")
            && let Some(asset) = attribute(tag, "src").and_then(|src| referenced(assets, src))
            && let Ok(code) = std::str::from_utf8(&asset.bytes)
        {
            let end = rest.to_ascii_lowercase().find("</script>");
            let module = attribute(tag, "type").is_some_and(|kind| kind == "module");
            out.push_str(if module {
                "<script type=\"module\">"
            } else {
                "<script>"
            });
            out.push_str(&code.replace("</script", "<\\/script"));
            out.push_str("</script>");
            rest = end.map_or(&rest[close + 1..], |end| &rest[end + "</script>".len()..]);
            continue;
        }
        if lower.starts_with("<link")
            && attribute(tag, "rel").is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet"))
            && let Some(asset) = attribute(tag, "href").and_then(|href| referenced(assets, href))
            && let Ok(css) = std::str::from_utf8(&asset.bytes)
        {
            out.push_str("<style>");
            out.push_str(&css.replace("</style", "<\\/style"));
            out.push_str("</style>");
            rest = &rest[close + 1..];
            continue;
        }
        out.push_str(&data_urls(tag, assets));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Replace `src`/`href` values in one tag that name non-page assets with
/// `data:` URLs.
fn data_urls(tag: &str, assets: &[Asset]) -> String {
    let mut tag = tag.to_owned();
    for name in ["src", "href"] {
        let Some(value) = attribute(&tag, name).map(str::to_owned) else {
            continue;
        };
        let Some(asset) = referenced(assets, &value).filter(|asset| !is_html(&asset.entry.path))
        else {
            continue;
        };
        let url = format!(
            "data:{};base64,{}",
            asset.entry.content_type,
            base64::engine::general_purpose::STANDARD.encode(&asset.bytes)
        );
        for quote in ['"', '\''] {
            tag = tag.replacen(
                &format!("{quote}{value}{quote}"),
                &format!("{quote}{url}{quote}"),
                1,
            );
        }
    }
    tag
}

/// CRC-32 (IEEE), as zip records it.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn too_large(what: &str, err: std::num::TryFromIntError) -> std::io::Error {
    std::io::Error::other(format!("bundle too large for {what}: {err}"))
}

/// A zip archive of `files`, stored without compression.
///
/// # Errors
///
/// Fails when a file, the archive or the file count exceeds what zip
/// without its 64-bit extension can record.
pub fn zip(files: &[(String, Vec<u8>)]) -> std::io::Result<Vec<u8>> {
    // DOS date 1980-01-01, time 00:00: archives do not depend on the clock.
    const DOS_DATE: u16 = 0x0021;
    const UTF8_NAMES: u16 = 0x0800;
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, bytes) in files {
        let offset = u32::try_from(archive.len()).map_err(|err| too_large("zip", err))?;
        let size = u32::try_from(bytes.len()).map_err(|err| too_large("zip", err))?;
        let name_len = u16::try_from(name.len()).map_err(|err| too_large("zip names", err))?;
        let crc = crc32(bytes);
        archive.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        for field in [20, UTF8_NAMES, 0, 0, DOS_DATE] {
            archive.extend_from_slice(&u16::to_le_bytes(field));
        }
        for field in [crc, size, size] {
            archive.extend_from_slice(&field.to_le_bytes());
        }
        archive.extend_from_slice(&name_len.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(bytes);

        directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        for field in [20, 20, UTF8_NAMES, 0, 0, DOS_DATE] {
            directory.extend_from_slice(&u16::to_le_bytes(field));
        }
        for field in [crc, size, size] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        for field in [name_len, 0, 0, 0, 0] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&0_u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let count = u16::try_from(files.len()).map_err(|err| too_large("zip entries", err))?;
    let directory_offset = u32::try_from(archive.len()).map_err(|err| too_large("zip", err))?;
    let directory_size = u32::try_from(directory.len()).map_err(|err| too_large("zip", err))?;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    for field in [0, 0, count, count] {
        archive.extend_from_slice(&u16::to_le_bytes(field));
    }
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    Ok(archive)
}

/// A ustar archive of `files`.
///
/// # Errors
///
/// Fails when a name does not fit a ustar header.
pub fn tar(files: &[(String, Vec<u8>)]) -> std::io::Result<Vec<u8>> {
    let mut archive = Vec::new();
    for (name, bytes) in files {
        let (prefix, short) = split_tar_name(name)
            .ok_or_else(|| std::io::Error::other(format!("{name:?} is too long for tar")))?;
        let mut header = [0_u8; 512];
        header[..short.len()].copy_from_slice(short.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{:011o}\0", bytes.len());
        if size.len() != 12 {
            return Err(std::io::Error::other(format!(
                "{name:?} is too large for tar"
            )));
        }
        header[124..136].copy_from_slice(size.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        archive.extend_from_slice(&header);
        archive.extend_from_slice(bytes);
        archive.resize(archive.len().next_multiple_of(512), 0);
    }
    archive.resize(archive.len() + 1024, 0);
    Ok(archive)
}

/// Split a tar name into ustar `prefix` (at most 155 bytes) and `name`
/// (at most 100 bytes) at a `/`.
fn split_tar_name(name: &str) -> Option<(&str, &str)> {
    if name.len() <= 100 {
        return Some(("", name));
    }
    name.match_indices('/')
        .map(|(at, _)| (&name[..at], &name[at + 1..]))
        .find(|(prefix, short)| prefix.len() <= 155 && short.len() <= 100 && !short.is_empty())
}

/// `bytes` in one zstd frame of raw (stored) blocks: a valid `.zst` any
/// zstd decoder reads, with no compression.
#[must_use]
pub fn zstd_raw(bytes: &[u8]) -> Vec<u8> {
    // Blocks may not exceed 128 KiB.
    const MAX_BLOCK: usize = 128 * 1024;
    let mut frame = Vec::with_capacity(bytes.len() + 32);
    frame.extend_from_slice(&0xFD2F_B528_u32.to_le_bytes());
    // Frame header: an 8-byte content size, single segment, no checksum,
    // no dictionary.
    frame.push(0b1110_0000);
    frame.extend_from_slice(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    let mut chunks = bytes.chunks(MAX_BLOCK).peekable();
    if chunks.peek().is_none() {
        frame.extend_from_slice(&[1, 0, 0]);
    }
    while let Some(chunk) = chunks.next() {
        let last = u32::from(chunks.peek().is_none());
        // Raw block (type 0): bit 0 last, bits 1-2 type, bits 3-23 size.
        let size = u32::try_from(chunk.len()).unwrap_or(0);
        let header = last | (size << 3);
        frame.extend_from_slice(&header.to_le_bytes()[..3]);
        frame.extend_from_slice(chunk);
    }
    frame
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::runs::ManifestCapabilities;

    fn asset(path: &str, body: &str) -> Asset {
        Asset {
            entry: AssetEntry {
                path: path.to_owned(),
                url: String::new(),
                content_type: content_type(path),
                hash: content_hash(body.as_bytes()),
            },
            bytes: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn demo_app(dir: &FsPath) -> RunManifest {
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<html><head><link rel=\"stylesheet\" href=\"assets/app.css\"></head>\
             <body><img src=\"./assets/logo.svg\"><script src=\"assets/app.js\"></script></body></html>",
        )
        .unwrap();
        std::fs::write(
            dir.join("about.html"),
            "<p>about <img src='assets/logo.svg'></p>",
        )
        .unwrap();
        std::fs::write(dir.join("assets/app.css"), "body { color: red }").unwrap();
        std::fs::write(dir.join("assets/app.js"), "console.log('</script>')").unwrap();
        std::fs::write(dir.join("assets/logo.svg"), "<svg/>").unwrap();
        RunManifest {
            app: "demo".to_owned(),
            version: "1.2.3".to_owned(),
            client_side: [
                "index.html",
                "about.html",
                "assets/app.css",
                "assets/app.js",
                "assets/logo.svg",
            ]
            .map(str::to_owned)
            .to_vec(),
            capabilities: ManifestCapabilities::default(),
            pages: Pages::Single,
            root: Some(dir.to_path_buf()),
        }
    }

    #[test]
    fn crc32_matches_the_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// Read a stored zip back: `(name, bytes)` per central directory entry,
    /// checking each CRC.
    fn unzip(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
        let u16_at = |at: usize| u16::from_le_bytes([archive[at], archive[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(archive[at..at + 4].try_into().unwrap());
        let end = archive.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50);
        let count = usize::from(u16_at(end + 10));
        let mut at = usize::try_from(u32_at(end + 16)).unwrap();
        let mut files = Vec::new();
        for _ in 0..count {
            assert_eq!(u32_at(at), 0x0201_4b50);
            let crc = u32_at(at + 16);
            let size = usize::try_from(u32_at(at + 20)).unwrap();
            let name_len = usize::from(u16_at(at + 28));
            let local = usize::try_from(u32_at(at + 42)).unwrap();
            let name = String::from_utf8(archive[at + 46..at + 46 + name_len].to_vec()).unwrap();
            assert_eq!(u32_at(local), 0x0403_4b50);
            let data_at = local + 30 + usize::from(u16_at(local + 26));
            let bytes = archive[data_at..data_at + size].to_vec();
            assert_eq!(crc32(&bytes), crc);
            files.push((name, bytes));
            at += 46 + name_len;
        }
        files
    }

    /// Unwrap a raw-block zstd frame.
    fn unzstd(frame: &[u8]) -> Vec<u8> {
        assert_eq!(frame[..4], 0xFD2F_B528_u32.to_le_bytes());
        assert_eq!(frame[4], 0b1110_0000);
        let size = usize::try_from(u64::from_le_bytes(frame[5..13].try_into().unwrap())).unwrap();
        let mut at = 13;
        let mut out = Vec::new();
        loop {
            let header = u32::from_le_bytes([frame[at], frame[at + 1], frame[at + 2], 0]);
            at += 3;
            assert_eq!((header >> 1) & 3, 0, "raw block");
            let len = usize::try_from(header >> 3).unwrap();
            out.extend_from_slice(&frame[at..at + len]);
            at += len;
            if header & 1 == 1 {
                break;
            }
        }
        assert_eq!(at, frame.len());
        assert_eq!(out.len(), size);
        out
    }

    /// Read a ustar archive back, checking each header checksum.
    fn untar(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        let mut at = 0;
        while archive[at..at + 512].iter().any(|byte| *byte != 0) {
            let header = &archive[at..at + 512];
            let field = |range: std::ops::Range<usize>| {
                String::from_utf8(header[range].to_vec())
                    .unwrap()
                    .trim_end_matches('\0')
                    .to_owned()
            };
            let mut blank = header.to_vec();
            blank[148..156].copy_from_slice(b"        ");
            let sum: u32 = blank.iter().map(|byte| u32::from(*byte)).sum();
            assert_eq!(u32::from_str_radix(field(148..154).trim(), 8).unwrap(), sum);
            let prefix = field(345..500);
            let short = field(0..100);
            let name = if prefix.is_empty() {
                short
            } else {
                format!("{prefix}/{short}")
            };
            let size = usize::from_str_radix(field(124..135).trim(), 8).unwrap();
            at += 512;
            files.push((name, archive[at..at + size].to_vec()));
            at += size.next_multiple_of(512);
        }
        files
    }

    fn sample_files() -> Vec<(String, Vec<u8>)> {
        let long = format!("{}/{}", "d".repeat(120), "f".repeat(90));
        vec![
            ("a.txt".to_owned(), b"alpha".to_vec()),
            ("empty".to_owned(), Vec::new()),
            ("big.bin".to_owned(), vec![7_u8; 300 * 1024]),
            (long, b"deep".to_vec()),
        ]
    }

    #[test]
    fn zip_and_tar_zst_round_trip() {
        let files = sample_files();
        assert_eq!(unzip(&zip(&files).unwrap()), files);
        assert_eq!(untar(&unzstd(&zstd_raw(&tar(&files).unwrap()))), files);
        assert_eq!(unzstd(&zstd_raw(b"")), b"");
    }

    /// Where the standard tools exist, they read both archives too.
    #[test]
    fn standard_tools_read_the_archives() {
        let has = |tool: &str| {
            std::process::Command::new(tool)
                .arg("--version")
                .output()
                .is_ok_and(|out| out.status.success())
        };
        let dir = tempfile::tempdir().unwrap();
        let files = sample_files();
        if has("zstd") && has("tar") {
            let path = dir.path().join("b.tar.zst");
            std::fs::write(&path, zstd_raw(&tar(&files).unwrap())).unwrap();
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("zstd -dc '{}' | tar -tf -", path.display()))
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            let listed = String::from_utf8(out.stdout).unwrap();
            for (name, _) in &files {
                assert!(listed.lines().any(|line| line == name), "{listed}");
            }
        }
        let unzip_present = std::process::Command::new("unzip")
            .arg("-v")
            .output()
            .is_ok_and(|out| out.status.success());
        if unzip_present {
            let path = dir.path().join("b.zip");
            std::fs::write(&path, zip(&files).unwrap()).unwrap();
            let out = std::process::Command::new("unzip")
                .arg("-t")
                .arg(&path)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        }
    }

    #[test]
    fn single_file_inlines_assets_and_keeps_pages() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = demo_app(dir.path());
        let assets = read_assets(&manifest).unwrap();
        let html = single_file(&assets).unwrap();
        assert!(
            html.contains("<style>body { color: red }</style>"),
            "{html}"
        );
        assert!(
            html.contains("<script>console.log('<\\/script>')</script>"),
            "{html}"
        );
        assert!(!html.contains("assets/app.js"), "{html}");
        assert!(html.contains("src=\"data:image/svg+xml;base64,"), "{html}");
        assert!(
            html.contains(
                "<template data-page=\"about.html\"><p>about <img src='data:image/svg+xml;base64,"
            ),
            "{html}"
        );
        let body_end = html.rfind("</body>").unwrap();
        assert!(html.find("<template").unwrap() < body_end);
        assert_eq!(single_file(&[asset("a.js", "x")]), None);
    }

    #[test]
    fn assets_are_declared_hashed_and_confined() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = demo_app(dir.path());
        let asset = read_asset(&manifest, "assets/app.css").unwrap();
        assert_eq!(asset.entry.content_type, "text/css");
        assert_eq!(asset.entry.hash.len(), HASH_LEN);
        assert_eq!(
            asset.entry.url,
            format!("/v1/apps/demo/assets/assets/app.css?v={}", asset.entry.hash)
        );
        assert_eq!(
            read_asset(&manifest, "app.toml").unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("secret"), "s").unwrap();
            std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("leak"))
                .unwrap();
            let mut leaky = manifest;
            leaky.client_side.push("leak".to_owned());
            assert_eq!(
                read_asset(&leaky, "leak").unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
    }

    #[test]
    fn bundles_hold_every_asset_and_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = demo_app(dir.path());
        let zipped = unzip(&build_bundle(&manifest, BundleFormat::Zip).unwrap());
        let tarred = untar(&unzstd(
            &build_bundle(&manifest, BundleFormat::TarZst).unwrap(),
        ));
        assert_eq!(zipped, tarred);
        let names: Vec<&str> = zipped.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "index.html",
                "about.html",
                "assets/app.css",
                "assets/app.js",
                "assets/logo.svg",
                "manifest.json"
            ]
        );
        let document: serde_json::Value = serde_json::from_slice(&zipped[5].1).unwrap();
        assert_eq!(document["pages"], "single");
        assert_eq!(document["assets"].as_array().unwrap().len(), 5);
    }

    mod http {
        use std::collections::BTreeMap;
        use std::net::SocketAddr;
        use std::path::PathBuf;

        use axum::body::Body;
        use axum::extract::ConnectInfo;
        use axum::http::Request as HttpRequest;
        use tower::ServiceExt as _;

        use super::*;
        use crate::runs::runs_routes;

        async fn get(
            manifest: &RunManifest,
            uri: &str,
            peer: [u8; 4],
        ) -> (StatusCode, HeaderMap, Vec<u8>) {
            let apps = Arc::new(BTreeMap::from([(manifest.app.clone(), manifest.clone())]));
            let router: Router = runs_routes(RunsConfig::new(PathBuf::from("sh")).with_apps(apps));
            let mut request = HttpRequest::get(uri).body(Body::empty()).unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from((peer, 40_000))));
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let headers = response.headers().clone();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, headers, body.to_vec())
        }

        const LOCAL: [u8; 4] = [127, 0, 0, 1];

        #[tokio::test]
        async fn manifest_lists_hashed_assets_served_by_hash() {
            let dir = tempfile::tempdir().unwrap();
            let manifest = demo_app(dir.path());
            let (status, _, body) = get(&manifest, "/v1/apps/demo/manifest", LOCAL).await;
            assert_eq!(status, StatusCode::OK);
            let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(document["pages"], "single");
            assert!(document.get("root").is_none(), "{document}");
            let css = document["assets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|asset| asset["path"] == "assets/app.css")
                .unwrap()
                .clone();
            let url = css["url"].as_str().unwrap();
            let (status, headers, body) = get(&manifest, url, LOCAL).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, b"body { color: red }");
            assert_eq!(headers[header::CONTENT_TYPE], "text/css");
            assert!(
                headers[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("immutable")
            );
            let (status, headers, _) =
                get(&manifest, "/v1/apps/demo/assets/assets/app.css", LOCAL).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
            let (status, _, _) = get(
                &manifest,
                "/v1/apps/demo/assets/assets/app.css?v=0000",
                LOCAL,
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn undeclared_remote_and_unknown_requests_are_refused() {
            let dir = tempfile::tempdir().unwrap();
            let manifest = demo_app(dir.path());
            std::fs::write(dir.path().join("secret.txt"), "no").unwrap();
            for (uri, peer, want) in [
                (
                    "/v1/apps/demo/assets/secret.txt",
                    LOCAL,
                    StatusCode::NOT_FOUND,
                ),
                (
                    "/v1/apps/demo/assets/index.html",
                    [192, 0, 2, 7],
                    StatusCode::FORBIDDEN,
                ),
                (
                    "/v1/apps/nope/assets/index.html",
                    LOCAL,
                    StatusCode::NOT_FOUND,
                ),
                (
                    "/v1/apps/demo/bundle?format=rar",
                    LOCAL,
                    StatusCode::BAD_REQUEST,
                ),
                (
                    "/v1/apps/demo/bundle.html",
                    [192, 0, 2, 7],
                    StatusCode::FORBIDDEN,
                ),
            ] {
                let (status, _, _) = get(&manifest, uri, peer).await;
                assert_eq!(status, want, "{uri}");
            }
        }

        #[tokio::test]
        async fn bundle_routes_answer_archives_and_one_page() {
            let dir = tempfile::tempdir().unwrap();
            let manifest = demo_app(dir.path());
            let (status, _, body) = get(&manifest, "/v1/apps/demo/bundle?format=zip", LOCAL).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(&body[..4], b"PK\x03\x04");
            let (status, _, body) = get(&manifest, "/v1/apps/demo/bundle", LOCAL).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(&body[..4], &0xFD2F_B528_u32.to_le_bytes());
            let (status, headers, body) = get(&manifest, "/v1/apps/demo/bundle.html", LOCAL).await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                headers[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with("text/html")
            );
            let html = String::from_utf8(body).unwrap();
            assert!(html.contains("color: red"), "{html}");
            assert!(!html.contains("src=\"assets/app.js\""), "{html}");
        }
    }
}
