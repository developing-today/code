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
//! │  │   Axum      │    │    SPA      │    │ ProseMirror │      │
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
//! │  - CSS: styles.css (TailwindCSS v4 + DaisyUI, single file)                 │
//! │  - JS: main.js (bundled with Bun)                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **File Browser**: SPA file listing with lazy loading
//! - **Collaborative Editor**: Real-time editing with prosemirror-collab
//! - **Themes**: CRT terminal themes (sneak/blue, arch/green, mech/orange)
//! - **Single Binary**: All assets embedded via rust-embed
//!
//! # Usage
//!
//! Enable the `web` feature and start the server:
//!
//! ```bash
//! cargo build --features web
//! id serve --web
//! ```

mod assets;
mod collab;
mod content_mode;
mod identity;
mod markdown;
mod routes;
mod tags_ws;
mod templates;

pub use content_mode::{ContentMode, MediaType, detect_mode, detect_mode_with_content};
pub use markdown::{
    markdown_to_prosemirror, plain_text_to_prosemirror, prosemirror_to_markdown,
    raw_text_to_prosemirror,
};

use axum::Router;
use iroh_blobs::api::Store;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::discovery::PeerDiscovery;
use crate::tags::TagStore;

pub use assets::static_handler;
pub use collab::CollabState;
pub use identity::IdentityStore;
pub use routes::create_router;
pub use templates::{AssetUrls, render_page};

/// Default save rate limit cooldown period.
pub const DEFAULT_SAVE_COOLDOWN: Duration = Duration::from_secs(5);

/// Per-file save rate limiter.
///
/// Tracks the last save time for each file (by name) and rejects saves
/// that happen within the cooldown period. This prevents rapid-fire saves
/// from creating excessive archive entries.
#[derive(Debug, Clone)]
pub struct SaveRateLimiter {
    /// Map of filename → last save time.
    last_save: Arc<Mutex<HashMap<String, Instant>>>,
    /// Minimum time between saves for the same file.
    pub cooldown: Duration,
}

impl SaveRateLimiter {
    /// Create a new rate limiter with the given cooldown duration.
    pub fn new(cooldown: Duration) -> Self {
        Self {
            last_save: Arc::new(Mutex::new(HashMap::new())),
            cooldown,
        }
    }

    /// Check if a save is allowed for the given filename.
    ///
    /// Returns `Ok(())` if the save is allowed, or `Err(remaining)` with
    /// the time remaining until the next save is allowed.
    pub async fn check(&self, name: &str) -> Result<(), Duration> {
        let map = self.last_save.lock().await;
        if let Some(last) = map.get(name) {
            let elapsed = last.elapsed();
            if elapsed < self.cooldown {
                return Err(self.cooldown - elapsed);
            }
        }
        Ok(())
    }

    /// Record a successful save for the given filename.
    pub async fn record(&self, name: &str) {
        let mut map = self.last_save.lock().await;
        map.insert(name.to_owned(), Instant::now());
    }
}

/// Shared application state for web handlers.
///
/// Contains references to the blob store, collaborative editing state,
/// tag metadata store, and optional peer discovery table.
#[derive(Clone)]
pub struct AppState {
    /// The blob store for accessing files.
    pub store: Store,
    /// State for collaborative editing sessions.
    pub collab: Arc<CollabState>,
    /// Asset URLs (with cache-busting hashes).
    pub assets: AssetUrls,
    /// Optional peer discovery table (populated when gossip is active).
    pub peers: Option<PeerDiscovery>,
    /// This node's public ID (hex-encoded).
    pub node_id: String,
    /// Tag metadata store (α/Ω namespace pairs backed by iroh-docs).
    pub tag_store: Arc<TagStore>,
    /// Per-file save rate limiter.
    pub save_limiter: SaveRateLimiter,
    /// Client identity store for persistent client sessions.
    pub identity: IdentityStore,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("store", &"<Store>")
            .field("collab", &self.collab)
            .field("assets", &self.assets)
            .field("peers", &self.peers.is_some())
            .field("node_id", &self.node_id)
            .field("tag_store", &"<TagStore>")
            .field("save_limiter", &self.save_limiter)
            .field("identity", &self.identity)
            .finish()
    }
}

impl AppState {
    /// Create a new application state.
    pub fn new(
        store: Store,
        peers: Option<PeerDiscovery>,
        node_id: String,
        tag_store: Arc<TagStore>,
    ) -> Self {
        Self {
            store,
            collab: Arc::new(CollabState::new()),
            assets: load_asset_urls(),
            peers,
            node_id,
            tag_store,
            save_limiter: SaveRateLimiter::new(DEFAULT_SAVE_COOLDOWN),
            identity: IdentityStore::new(),
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
/// * `peers` - Optional peer discovery table for the `/peers` page
/// * `node_id` - This node's public ID (hex-encoded)
/// * `tag_store` - The tag metadata store (α/Ω namespace pairs)
///
/// # Returns
///
/// An Axum router ready to be merged with the serve endpoint.
pub fn web_router(
    store: Store,
    peers: Option<PeerDiscovery>,
    node_id: String,
    tag_store: Arc<TagStore>,
) -> Router {
    let state = AppState::new(store, peers, node_id, tag_store);
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
