//! Static asset handling with rust-embed.
//!
//! Embeds all web assets (JS, CSS, HTML) into the binary at compile time,
//! enabling single-binary deployment without external files.

// Allow same_name_method warning from rust-embed derive macro
#![allow(clippy::same_name_method)]

use axum::{
    body::Body,
    http::{HeaderValue, Response, StatusCode, header},
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

/// Check if a filename is a content-hashed build artifact.
///
/// The contract is what the `web/dist` build actually emits -- a name
/// is hashed iff one of the two emitters can produce it (#6779; oracle:
/// the emitters, not a pattern resemblance):
///
/// - the JS entry from `bun build --entry-naming=[name].[hash].js`
///   (pkgs/id/web/package.json): `main.` + exactly 8 base36 chars
///   (`[a-z0-9]`) + `.js`, plus that entry's `.js.map` sourcemap;
/// - the stylesheet renamed by pkgs/id/web/scripts/build-manifest.ts:
///   `styles.` + exactly 8 lowercase hex chars (`[0-9a-f]`, a SHA-256
///   prefix) + `.css`.
///
/// Everything else is served `no-cache`. Anchoring on the emitted
/// names is what keeps ordinary dotted names (`app.development.js`,
/// `styles.darkmode.css`, `vendor.production.js`,
/// `editor.settings.js`) out of the one-year immutable cache: a word
/// segment is indistinguishable from a hash by charset alone
/// (letters-only Bun hashes exist). The rule fails in the safe
/// direction -- a future second entry point or renamed stylesheet
/// gets revalidation, not staleness, until this contract is extended
/// deliberately alongside the build change.
fn is_hashed_filename(path: &str) -> bool {
    let Some(name) = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
    else {
        return false;
    };
    if let Some(rest) = name.strip_prefix("main.") {
        let segment = if let Some(seg) = rest.strip_suffix(".js") {
            seg
        } else if let Some(seg) = rest.strip_suffix(".js.map") {
            seg
        } else {
            return false;
        };
        return segment.len() == 8
            && segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    }
    if let Some(rest) = name.strip_prefix("styles.") {
        let Some(segment) = rest.strip_suffix(".css") else {
            return false;
        };
        return segment.len() == 8
            && segment
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    }
    false
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
    fn test_is_hashed_filename_dotted_word_segments() {
        // #6779: ordinary dotted names are not build hashes and must
        // get no-cache, not the one-year immutable header.
        assert!(!is_hashed_filename("app.development.js"));
        assert!(!is_hashed_filename("styles.darkmode.css"));
        assert!(!is_hashed_filename("vendor.production.js"));
        assert!(!is_hashed_filename("editor.settings.js"));
        // A longer or shorter segment is not an emitted hash either.
        assert!(!is_hashed_filename("main.development.js"));
        assert!(!is_hashed_filename("styles.99e0273f1.css"));
    }

    #[test]
    fn test_is_hashed_filename_sourcemap_and_wrong_stem() {
        // The JS entry's sourcemap shares the hashed stem.
        assert!(is_hashed_filename("main.bjbjbv7v.js.map"));
        // Hash-shaped segments on stems the build never emits are not
        // hashed: no other entry point exists in the contract.
        assert!(!is_hashed_filename("app.bjbjbv7v.js"));
        assert!(!is_hashed_filename("other.99e0273f.css"));
    }

    #[test]
    fn test_is_hashed_filename_no_extension() {
        // Files without extension
        assert!(!is_hashed_filename("main"));
        assert!(!is_hashed_filename("main.bjbjbv7v")); // no final extension
    }
}
