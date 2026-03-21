//! Web interface module for the id file sharing service.
//!
//! This module provides an Axum-based web UI for browsing and editing files,
//! with collaborative editing support via `ProseMirror` and `WebSockets`.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Web Interface                          │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐      │
//! │  │   Axum      │    │   HTMX      │    │ ProseMirror │      │
//! │  │   Router    │───►│   Views     │───►│   Editor    │      │
//! │  └─────────────┘    └─────────────┘    └─────────────┘      │
//! │         │                  │                  │             │
//! │         │           ┌──────┴──────┐    ┌─────┴─────┐        │
//! │         │           │             │    │           │        │
//! │         │     ┌─────▼─────┐ ┌─────▼────▼┐                   │
//! │         │     │   HTML    │ │ WebSocket │                   │
//! │         │     │ Templates │ │  Collab   │                   │
//! │         │     └───────────┘ └───────────┘                   │
//! │         │                                                    │
//! │         ▼                                                    │
//! │  Embedded Assets (rust-embed)                                │
//! │  - CSS: terminal.css, themes.css, editor.css                 │
//! │  - JS: main.js (bundled with Bun)                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **File Browser**: HTMX-powered file listing with lazy loading
//! - **Collaborative Editor**: Real-time editing with prosemirror-collab
//! - **Themes**: Matrix (green-on-black) and Evangelion (orange/purple) themes
//! - **Single Binary**: All assets embedded via rust-embed
//!
//! # Usage
//!
//! Enable the `web` feature and start the server:
//!
//! ```bash
//! cargo build --features web
//! id serve --web --port 3000
//! ```

mod assets;
mod collab;
mod content_mode;
mod markdown;
mod routes;
mod templates;

pub use content_mode::{ContentMode, MediaType, detect_mode, detect_mode_with_content};
pub use markdown::{
    markdown_to_prosemirror, plain_text_to_prosemirror, prosemirror_to_markdown,
    raw_text_to_prosemirror,
};

use axum::Router;
use iroh_blobs::api::Store;
use std::sync::Arc;

pub use assets::static_handler;
pub use collab::CollabState;
pub use routes::create_router;
pub use templates::{AssetUrls, render_page};

/// Shared application state for web handlers.
///
/// Contains references to the blob store and collaborative editing state.
#[derive(Clone)]
pub struct AppState {
    /// The blob store for accessing files.
    pub store: Store,
    /// State for collaborative editing sessions.
    pub collab: Arc<CollabState>,
    /// Asset URLs (with cache-busting hashes).
    pub assets: AssetUrls,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("store", &"<Store>")
            .field("collab", &self.collab)
            .field("assets", &self.assets)
            .finish()
    }
}

impl AppState {
    /// Create a new application state.
    pub fn new(store: Store) -> Self {
        Self {
            store,
            collab: Arc::new(CollabState::new()),
            assets: load_asset_urls(),
        }
    }
}

/// Load asset URLs from the embedded manifest.
///
/// Falls back to default non-hashed URLs if manifest is not found
/// (e.g., during development).
fn load_asset_urls() -> AssetUrls {
    use assets::Assets;

    // Try to load manifest.json
    let Some(manifest_data) = Assets::get("manifest.json") else {
        tracing::debug!("[web] No manifest.json found, using default asset URLs");
        return AssetUrls::default();
    };

    let Ok(manifest_str) = std::str::from_utf8(&manifest_data.data) else {
        tracing::warn!("[web] manifest.json is not valid UTF-8");
        return AssetUrls::default();
    };

    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(manifest_str) else {
        tracing::warn!("[web] Failed to parse manifest.json");
        return AssetUrls::default();
    };

    let main_js = manifest
        .get("main.js")
        .and_then(|v| v.as_str())
        .map_or_else(|| "/assets/main.js".to_owned(), |s| format!("/assets/{s}"));

    let styles_css = manifest
        .get("styles.css")
        .and_then(|v| v.as_str())
        .map_or_else(
            || "/assets/styles.css".to_owned(),
            |s| format!("/assets/{s}"),
        );

    tracing::info!(
        "[web] Loaded asset manifest: main={}, styles={}",
        main_js,
        styles_css
    );

    AssetUrls {
        main_js,
        styles_css,
    }
}

/// Create the web router with all routes configured.
///
/// # Arguments
///
/// * `store` - The blob store to use for file operations
///
/// # Returns
///
/// An Axum router ready to be merged with the serve endpoint.
pub fn web_router(store: Store) -> Router {
    let state = AppState::new(store);
    create_router(state)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_load_asset_urls_from_manifest() {
        let urls = load_asset_urls();
        // Should load hashed URLs from manifest (name.hash.ext format)
        // Check that main_js has at least two dots (name.hash.js)
        let js_dots = urls.main_js.matches('.').count();
        assert!(
            js_dots >= 2,
            "main_js should be hashed (name.hash.js): {}",
            urls.main_js
        );
        let css_dots = urls.styles_css.matches('.').count();
        assert!(
            css_dots >= 2,
            "styles_css should be hashed (name.hash.css): {}",
            urls.styles_css
        );
    }

    #[test]
    fn test_asset_urls_have_correct_prefix() {
        let urls = load_asset_urls();
        assert!(
            urls.main_js.starts_with("/assets/"),
            "main_js should start with /assets/: {}",
            urls.main_js
        );
        assert!(
            urls.styles_css.starts_with("/assets/"),
            "styles_css should start with /assets/: {}",
            urls.styles_css
        );
    }
}
