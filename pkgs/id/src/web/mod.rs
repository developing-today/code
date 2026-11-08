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

mod app_interface;
mod assets;
mod collab;
mod content_mode;
mod explore;
mod home;
mod identity;
mod markdown;
mod routes;
mod security;
mod tags_ws;
mod templates;
pub mod world_ws;

pub use content_mode::{ContentMode, MediaType, detect_mode, detect_mode_with_content};
pub use markdown::{
    markdown_to_prosemirror, plain_text_to_prosemirror, prosemirror_to_markdown,
    raw_text_to_prosemirror,
};

use axum::Router;
use iroh_blobs::api::Store;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::discovery::PeerDiscovery;
use crate::tags::TagStore;

pub use app_interface::{WebInterface, WebInterfaceError, WebMode, web_interface_from_app_toml};
pub use assets::static_handler;
pub use collab::CollabState;
pub use identity::IdentityStore;
#[doc(hidden)]
pub use identity::crash_gate;
pub use routes::{create_program_router, create_router, create_router_for_app};
pub use security::WebSecurity;
pub use templates::{AssetUrls, render_page};

/// Default save rate limit cooldown period.
pub const DEFAULT_SAVE_COOLDOWN: Duration = Duration::from_secs(5);

/// Per-file save rate limiter.
///
/// Tracks the last save time for each file (by name) and rejects saves
/// that happen within the cooldown period. This prevents rapid-fire saves
/// from creating excessive archive entries. Entries are dropped once
/// their cooldown lapses, so the map holds only files saved recently.
#[derive(Debug, Clone)]
pub struct SaveRateLimiter {
    /// Map of filename → last save time.
    last_save: Arc<Mutex<HashMap<String, Instant>>>,
    /// Filenames with a save currently in flight (#2466). `check` alone
    /// cannot stop overlapping saves — it only sees *completed* saves —
    /// so `acquire` reserves the name here for the whole handler run and
    /// a second same-file save is refused instead of racing the first.
    /// A std mutex: the set is only touched for O(1) insert/remove, never
    /// held across an await, and `SavePermit::drop` must release the
    /// reservation synchronously on every handler exit path.
    in_flight: Arc<std::sync::Mutex<HashSet<String>>>,
    /// Minimum time between saves for the same file.
    pub cooldown: Duration,
}

impl SaveRateLimiter {
    /// Create a new rate limiter with the given cooldown duration.
    pub fn new(cooldown: Duration) -> Self {
        Self {
            last_save: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(std::sync::Mutex::new(HashSet::new())),
            cooldown,
        }
    }

    /// Check if a save is allowed for the given filename.
    ///
    /// Returns `Ok(())` if the save is allowed, or `Err(remaining)` with
    /// the time remaining until the next save is allowed.
    pub async fn check(&self, name: &str) -> Result<(), Duration> {
        let mut map = self.last_save.lock().await;
        if let Some(last) = map.get(name) {
            let elapsed = last.elapsed();
            if elapsed < self.cooldown {
                // checked_sub is Some exactly because elapsed < cooldown;
                // ZERO is the unreachable fallback (no time remaining).
                return Err(self.cooldown.checked_sub(elapsed).unwrap_or(Duration::ZERO));
            }
            // The cooldown has lapsed, so the entry can no longer refuse a
            // save; drop it instead of keeping the name forever (#1713).
            map.remove(name);
        }
        Ok(())
    }

    /// Reserve a filename for a save that is about to run (#2466).
    ///
    /// Combines the cooldown `check` with an in-flight reservation taken
    /// atomically under the in-flight lock: if a save for the same name
    /// is already running, returns `Err(self.cooldown)` so the caller can
    /// refuse with the usual "Try again in Ns" contract instead of
    /// racing the in-flight save to the filename tag. On success the
    /// returned `SavePermit` holds the reservation until it is dropped —
    /// on completion (after `complete` records the save) or on any early
    /// error return, which releases the name without recording a save,
    /// so a failed save never strands the filename.
    pub async fn acquire(&self, name: &str) -> Result<SavePermit, Duration> {
        self.check(name).await?;
        let mut flight = self
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !flight.insert(name.to_owned()) {
            return Err(self.cooldown);
        }
        Ok(SavePermit {
            limiter: self.clone(),
            name: name.to_owned(),
        })
    }

    /// Record a successful save for the given filename.
    pub async fn record(&self, name: &str) {
        let mut map = self.last_save.lock().await;
        // Sweep entries whose cooldown has lapsed: they can no longer
        // refuse a save, and keeping them would grow the map by one entry
        // for every filename ever saved (#1713).
        map.retain(|_, last| last.elapsed() < self.cooldown);
        map.insert(name.to_owned(), Instant::now());
    }
}

/// An in-flight save reservation handed out by `SaveRateLimiter::acquire`.
///
/// Dropping the permit releases the filename reservation. Call
/// `complete` once the save has succeeded so the cooldown is recorded;
/// dropping without completing (any early error path in the handler)
/// records nothing.
#[derive(Debug)]
pub struct SavePermit {
    /// The limiter that issued the reservation (cloned handle).
    limiter: SaveRateLimiter,
    /// The reserved filename.
    name: String,
}

impl SavePermit {
    /// Record the save as successful and release the reservation.
    pub async fn complete(self) {
        self.limiter.record(&self.name).await;
        // The reservation itself is released by Drop as `self` goes out
        // of scope at the end of this call.
    }
}

impl Drop for SavePermit {
    fn drop(&mut self) {
        if let Ok(mut flight) = self.limiter.in_flight.lock() {
            flight.remove(&self.name);
        }
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
    /// Serializes upload name resolution, tag assignment and metadata writes
    /// across clones of this state. Independently constructed states, other
    /// processes and non-upload tag writers do not participate in this lock.
    pub(crate) upload_name_lock: Arc<Mutex<()>>,
    /// Client identity store for persistent client sessions.
    pub identity: IdentityStore,
    /// Node-signed event log used to publish durable document revisions.
    pub(crate) local_events: Arc<Mutex<crate::local_events::LocalEvents>>,
    /// Optional hosted world (and its invite secret) for the WebSocket bridge.
    pub world: Option<crate::world_hub::WorldHub>,
    /// The `[interfaces.web]` page route this state's router binds
    /// the world page at (#2325). Defaults to the world app's own
    /// `/world`; `create_router_for_app` overwrites it from the
    /// app's declared interface.
    pub web_route: String,
    /// The `[interfaces.web]` live-channel route, if the app
    /// declares one (#2325). Defaults to the world app's
    /// `/ws/world`.
    pub web_socket: Option<String>,
    /// The world a program-only router serves at its root (#2146
    /// phase 2), when this state's router is a program surface.
    /// `None` for Explorer routers. Set by `create_program_router`.
    pub program_world: Option<String>,
    /// Mark the directory explorer's session cookies `Secure`.
    pub cookie_secure: bool,
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
            .field("local_events", &"<LocalEvents>")
            .field("world", &self.world)
            .field("cookie_secure", &self.cookie_secure)
            .finish()
    }
}

impl AppState {
    /// Create a new application state with persistent identity storage.
    ///
    /// Opens the encrypted identity store (a redb file) for identity persistence
    /// using a key derived from the node's iroh secret key.
    pub async fn new(
        store: Store,
        peers: Option<PeerDiscovery>,
        node_id: String,
        tag_store: Arc<TagStore>,
        secret_key: [u8; 32],
        identity_db_path: std::path::PathBuf,
    ) -> anyhow::Result<Self> {
        let events_dir = identity_db_path.with_file_name(crate::local_events::DIR_NAME);
        let local_events = Arc::new(Mutex::new(crate::local_events::LocalEvents::open(
            &events_dir,
            p2panda_core::SigningKey::from_bytes(&secret_key),
        )?));
        Self::with_local_events(
            store,
            peers,
            node_id,
            tag_store,
            secret_key,
            identity_db_path,
            local_events,
        )
        .await
    }

    /// Create a new application state over a `local_events` store the caller already
    /// opened and shares — `id serve` shares one instance between this state and the
    /// `/id-range-sync/1` sync handler, so revisions saved here are the revisions sync
    /// carries, with no second opener on the store's directory (#792).
    pub async fn with_local_events(
        store: Store,
        peers: Option<PeerDiscovery>,
        node_id: String,
        tag_store: Arc<TagStore>,
        secret_key: [u8; 32],
        identity_db_path: std::path::PathBuf,
        local_events: Arc<Mutex<crate::local_events::LocalEvents>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            collab: Arc::new(CollabState::new()),
            assets: load_asset_urls(),
            peers,
            node_id,
            tag_store,
            save_limiter: SaveRateLimiter::new(DEFAULT_SAVE_COOLDOWN),
            upload_name_lock: Arc::new(Mutex::new(())),
            identity: IdentityStore::new_with_max_identities(
                secret_key,
                identity_db_path,
                identity::configured_max_identities(),
            )
            .await?,
            local_events,
            world: None,
            web_route: "/world".to_owned(),
            web_socket: Some("/ws/world".to_owned()),
            program_world: None,
            cookie_secure: false,
        })
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
/// * `security` - Host allow-list, origin check and optional token (see
///   [`WebSecurity`]); applied to every request
///
/// # Returns
///
/// An Axum router ready to be merged with the serve endpoint.
pub async fn web_router(
    store: Store,
    peers: Option<PeerDiscovery>,
    node_id: String,
    tag_store: Arc<TagStore>,
    secret_key: [u8; 32],
    identity_db_path: std::path::PathBuf,
    security: WebSecurity,
    world: Option<crate::world_hub::WorldHub>,
    cookie_secure: bool,
) -> anyhow::Result<Router> {
    let state = AppState::new(
        store,
        peers,
        node_id,
        tag_store,
        secret_key,
        identity_db_path,
    )
    .await?;
    Ok(finish_router(state, security, world, cookie_secure, None))
}

/// Like [`web_router`], but over a caller-opened, shared `local_events` store (see
/// [`AppState::with_local_events`]).
#[allow(clippy::too_many_arguments)]
pub async fn web_router_with_local_events(
    store: Store,
    peers: Option<PeerDiscovery>,
    node_id: String,
    tag_store: Arc<TagStore>,
    secret_key: [u8; 32],
    identity_db_path: std::path::PathBuf,
    security: WebSecurity,
    world: Option<crate::world_hub::WorldHub>,
    cookie_secure: bool,
    local_events: Arc<Mutex<crate::local_events::LocalEvents>>,
) -> anyhow::Result<Router> {
    let state = AppState::with_local_events(
        store,
        peers,
        node_id,
        tag_store,
        secret_key,
        identity_db_path,
        local_events,
    )
    .await?;
    Ok(finish_router(state, security, world, cookie_secure, None))
}

/// Build one router per listener security over a SINGLE shared
/// [`AppState`] (#2145 multi-listener serve). `surfaces` is parallel
/// to `securities`: each listener's #2146 content target (`None` =
/// Explorer, `Some(world)` = the program-only surface for that world).
///
/// `web_router` opens the identity store exclusively, so building one
/// router per listener fails on the second open ("identity store …
/// is already open"). Here the state is built once and cloned per
/// listener; each clone is finished with that listener's own
/// [`WebSecurity`], so per-bind host/token policy still applies.
#[allow(clippy::too_many_arguments)]
pub async fn web_routers(
    store: Store,
    peers: Option<PeerDiscovery>,
    node_id: String,
    tag_store: Arc<TagStore>,
    secret_key: [u8; 32],
    identity_db_path: std::path::PathBuf,
    securities: Vec<WebSecurity>,
    world: Option<crate::world_hub::WorldHub>,
    cookie_secure: bool,
    local_events: Option<Arc<Mutex<crate::local_events::LocalEvents>>>,
    surfaces: Vec<Option<String>>,
) -> anyhow::Result<Vec<Router>> {
    let state = match local_events {
        Some(events) => {
            AppState::with_local_events(
                store,
                peers,
                node_id,
                tag_store,
                secret_key,
                identity_db_path,
                events,
            )
            .await?
        }
        None => {
            AppState::new(
                store,
                peers,
                node_id,
                tag_store,
                secret_key,
                identity_db_path,
            )
            .await?
        }
    };
    Ok(securities
        .into_iter()
        .zip(surfaces)
        .map(|(security, surface)| {
            finish_router(
                state.clone(),
                security,
                world.clone(),
                cookie_secure,
                surface,
            )
        })
        .collect())
}

/// Applies the world/cookie settings and delegates security to the public
/// constructor, which applies exactly one guard layer.
///
/// `surface` is the listener's #2146 content target: `None` builds the
/// Explorer router; `Some(world)` builds the program-only router for
/// that world (#2146 phase 2).
fn finish_router(
    mut state: AppState,
    security: WebSecurity,
    world: Option<crate::world_hub::WorldHub>,
    cookie_secure: bool,
    surface: Option<String>,
) -> Router {
    state.world = world;
    state.cookie_secure = cookie_secure;
    let security = WebSecurity {
        cookie_secure,
        ..security
    };
    match &surface {
        Some(world) => create_program_router(state, world, security),
        None => create_router(state, security),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_save_limiter_refuses_within_the_cooldown_only_for_that_file() {
        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        limiter.record("a.txt").await;
        let remaining = limiter.check("a.txt").await.unwrap_err();
        assert!(remaining <= Duration::from_secs(5) && !remaining.is_zero());
        assert!(
            limiter.check("b.txt").await.is_ok(),
            "other files are not limited"
        );
    }

    #[tokio::test]
    async fn test_save_limiter_acquire_refuses_a_second_in_flight_save() {
        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        let _permit = limiter.acquire("a.txt").await.expect("first save proceeds");
        let remaining = limiter.acquire("a.txt").await.unwrap_err();
        assert_eq!(
            remaining,
            Duration::from_secs(5),
            "in-flight refusal quotes the cooldown"
        );
        assert!(
            limiter.acquire("b.txt").await.is_ok(),
            "a different file is not refused"
        );
    }

    #[tokio::test]
    async fn test_save_limiter_concurrent_acquires_admit_exactly_one() {
        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        let (first, second) = tokio::join!(limiter.acquire("a.txt"), limiter.acquire("a.txt"));
        let admitted = [first.is_ok(), second.is_ok()]
            .into_iter()
            .filter(|ok| *ok)
            .count();
        assert_eq!(admitted, 1, "exactly one overlapping save proceeds");
    }

    #[tokio::test]
    async fn test_save_limiter_dropped_permit_releases_without_recording() {
        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        {
            let _permit = limiter.acquire("a.txt").await.expect("first save proceeds");
        } // dropped without complete(): the save failed / the handler bailed
        assert!(
            limiter.acquire("a.txt").await.is_ok(),
            "a failed save strands neither the reservation nor a cooldown"
        );
        assert!(limiter.check("a.txt").await.is_ok(), "nothing was recorded");
    }

    #[tokio::test]
    async fn test_save_limiter_completed_permit_records_the_cooldown() {
        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        let permit = limiter.acquire("a.txt").await.expect("first save proceeds");
        permit.complete().await;
        assert!(
            limiter.acquire("a.txt").await.is_err(),
            "the completed save's cooldown refuses the next save"
        );
        assert!(limiter.check("a.txt").await.is_err());
    }

    #[tokio::test]
    async fn test_save_limiter_record_sweeps_expired_entries() {
        let limiter = SaveRateLimiter::new(Duration::from_millis(60));
        limiter.record("a.txt").await;
        limiter.record("b.txt").await;
        assert_eq!(limiter.last_save.lock().await.len(), 2);
        tokio::time::sleep(Duration::from_millis(200)).await;
        limiter.record("c.txt").await;
        let map = limiter.last_save.lock().await;
        assert_eq!(map.len(), 1, "only the fresh entry remains: {map:?}");
        assert!(map.contains_key("c.txt"));
    }

    #[tokio::test]
    async fn test_save_limiter_check_drops_the_expired_entry_it_finds() {
        let limiter = SaveRateLimiter::new(Duration::from_millis(60));
        limiter.record("a.txt").await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            limiter.check("a.txt").await.is_ok(),
            "the cooldown has lapsed"
        );
        assert!(limiter.last_save.lock().await.is_empty());
    }

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
