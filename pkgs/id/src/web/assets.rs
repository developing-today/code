//! Static asset handling with rust-embed.
//!
//! Embeds all web assets (JS, CSS, HTML) into the binary at compile time,
//! enabling single-binary deployment without external files.

// Allow same_name_method warning from rust-embed derive macro
#![allow(clippy::same_name_method)]

use axum::{
    body::Body,
    http::{header, HeaderValue, Response, StatusCode},
};
use rust_embed::Embed;

/// Embedded web assets from the `web/dist` directory.
///
/// These assets are bundled at compile time using rust-embed.
/// The directory structure is preserved:
///
/// - `*.js` - Bundled JavaScript (from Bun)
/// - `*.css` - Stylesheets
/// - `*.html` - HTML templates
#[derive(Embed)]
#[folder = "web/dist"]
#[prefix = ""]
pub struct Assets;

/// Handle requests for static assets.
///
/// Serves embedded files with appropriate MIME types and caching headers.
/// Files with content hashes in their names (e.g., `main.abc123.js`) get
/// immutable caching, while non-hashed files get short cache times.
///
/// # Arguments
///
/// * `path` - The file path relative to the `web/dist` directory
///
/// # Returns
///
/// The file contents with appropriate headers, or 404 if not found.
pub fn static_handler(path: &str) -> Response<Body> {
    // Remove leading slash if present
    let path = path.trim_start_matches('/');

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();

            let mut response = Response::builder().status(StatusCode::OK).header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref())
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );

            // Check if filename contains a hash (e.g., main.abc12345.js)
            // Hashed files are immutable and can be cached forever
            let is_hashed = is_hashed_filename(path);

            if is_hashed {
                // Immutable cache for hashed files (1 year)
                response = response.header(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                );
            } else {
                // Short cache for non-hashed files (no-cache forces revalidation)
                response =
                    response.header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }

            match response.body(Body::from(content.data.into_owned())) {
                Ok(resp) => resp,
                Err(_) => internal_server_error(),
            }
        }
        None => not_found(),
    }
}

/// Check if a filename contains a content hash.
///
/// Matches patterns like `main.abc12xyz.js` or `styles.def67890.css`
/// where the middle segment is 8+ alphanumeric characters.
/// (Bun uses base36 hashes, CSS builder uses hex hashes.)
fn is_hashed_filename(path: &str) -> bool {
    let path = std::path::Path::new(path);
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };

    // Look for pattern: name.hash where hash is 8+ alphanumeric chars
    if let Some(dot_pos) = stem.rfind('.') {
        let potential_hash = &stem[dot_pos + 1..];
        potential_hash.len() >= 8
            && potential_hash
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    } else {
        false
    }
}

/// Build a 404 Not Found response.
fn not_found() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))
        .body(Body::from("not found"))
        .unwrap_or_else(|_| {
            let mut resp = Response::new(Body::empty());
            *resp.status_mut() = StatusCode::NOT_FOUND;
            resp
        })
}

/// Build a 500 Internal Server Error response.
fn internal_server_error() -> Response<Body> {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    resp
}

/// Get the contents of an embedded asset as a string.
///
/// Useful for loading HTML templates at runtime.
///
/// # Arguments
///
/// * `path` - The file path relative to the `web/dist` directory
///
/// # Returns
///
/// The file contents as a string, or `None` if not found or not valid UTF-8.
#[allow(dead_code)]
pub fn get_asset_string(path: &str) -> Option<String> {
    Assets::get(path).and_then(|content| String::from_utf8(content.data.into_owned()).ok())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_assets_embed_compiles() {
        // This test just verifies that the Assets struct compiles correctly
        // Actual asset availability depends on build-time bundling
        let _ = Assets::iter();
    }

    #[test]
    fn test_is_hashed_filename_js_base36() {
        // Bun uses base36 hashes (lowercase alphanumeric)
        assert!(is_hashed_filename("main.bjbjbv7v.js"));
        assert!(is_hashed_filename("main.abc12xyz.js"));
        assert!(is_hashed_filename("main.12345678.js"));
    }

    #[test]
    fn test_is_hashed_filename_css_hex() {
        // CSS build uses hex hashes
        assert!(is_hashed_filename("styles.99e0273f.css"));
        assert!(is_hashed_filename("styles.abcdef12.css"));
    }

    #[test]
    fn test_is_hashed_filename_with_path() {
        assert!(is_hashed_filename("assets/main.bjbjbv7v.js"));
        assert!(is_hashed_filename("/assets/styles.99e0273f.css"));
    }

    #[test]
    fn test_is_hashed_filename_non_hashed() {
        // Regular files without hashes
        assert!(!is_hashed_filename("main.js"));
        assert!(!is_hashed_filename("styles.css"));
        assert!(!is_hashed_filename("index.html"));
        assert!(!is_hashed_filename("manifest.json"));
    }

    #[test]
    fn test_is_hashed_filename_short_hash() {
        // Hash must be at least 8 characters
        assert!(!is_hashed_filename("main.abc.js"));
        assert!(!is_hashed_filename("main.1234567.js")); // 7 chars
        assert!(is_hashed_filename("main.12345678.js")); // 8 chars
    }

    #[test]
    fn test_is_hashed_filename_invalid_chars() {
        // Hash must be lowercase alphanumeric only
        assert!(!is_hashed_filename("main.ABCD1234.js")); // uppercase
        assert!(!is_hashed_filename("main.abc-1234.js")); // hyphen
        assert!(!is_hashed_filename("main.abc_1234.js")); // underscore
    }

    #[test]
    fn test_is_hashed_filename_no_extension() {
        // Files without extension
        assert!(!is_hashed_filename("main"));
        assert!(!is_hashed_filename("main.bjbjbv7v")); // no final extension
    }
}
