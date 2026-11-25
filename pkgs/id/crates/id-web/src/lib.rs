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
pub mod gallery;
mod groups;
mod home;
mod identity;
mod markdown;
pub mod presentation;
mod routes;
pub mod runs;
mod security;
mod seitan;
pub mod sessions;
mod tags_ws;
mod templates;
mod transport_ad;
pub mod world_ws;
pub mod wss_https;

pub use content_mode::{ContentMode, MediaType, detect_mode, detect_mode_with_content};
pub use markdown::{
    markdown_to_prosemirror, plain_text_to_prosemirror, prosemirror_to_markdown,
    raw_text_to_prosemirror,
};

use axum::Router;
use iroh_blobs::api::Store;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use id_core::discovery::PeerDiscovery;
use id_store::tags::TagStore;

pub use app_interface::{WebInterface, WebInterfaceError, WebMode, web_interface_from_app_toml};
pub use assets::static_handler;
pub use collab::{
    CollabState, NOTE_CHECKPOINT_SNAPSHOT_FILE_NAME, NoteCheckpointSnapshot, NoteSnapshotError,
    note_session_key,
};
pub use identity::IdentityStore;
#[doc(hidden)]
pub use identity::crash_gate;
pub use routes::{create_program_router, create_router, create_router_for_app};
pub use runs::{RunApps, RunManifest, RunsConfig, runs_routes};
pub use security::WebSecurity;
pub use seitan::SeitanState;
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
    /// Combines the cooldown `check` with an in-flight reservation: if a
    /// save for the same name is already running, returns
    /// `Err(self.cooldown)` so the caller can refuse with the usual
    /// "Try again in Ns" contract instead of racing the in-flight save
    /// to the filename tag. On success the returned `SavePermit` holds
    /// the reservation until it is dropped — on completion (after
    /// `complete` records the save) or on any early error return, which
    /// releases the name without recording a save, so a failed save
    /// never strands the filename.
    ///
    /// The check and the reservation use different locks, so the pair
    /// is made atomic with respect to completion by re-validating the
    /// cooldown after the reservation is taken (#9587): `complete`
    /// records the save *before* `Drop` releases the reservation, so
    /// any save that completes while this call is between its first
    /// check and its reservation has already recorded its cooldown by
    /// the time the reservation succeeds, and the re-validation below
    /// observes that record and refuses. A refused re-validation
    /// releases the reservation it just took, leaving nothing behind.
    pub async fn acquire(&self, name: &str) -> Result<SavePermit, Duration> {
        self.check(name).await?;
        {
            let mut flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !flight.insert(name.to_owned()) {
                return Err(self.cooldown);
            }
        }
        if let Err(remaining) = self.check(name).await {
            let mut flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            flight.remove(name);
            return Err(remaining);
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
        // Recover a poisoned lock exactly like `acquire` does (#9587):
        // `if let Ok` here would silently leave the name reserved, and
        // every later `acquire` (which recovers the guard and sees the
        // name still present) would refuse that filename forever.
        let mut flight = self
            .limiter
            .in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        flight.remove(&self.name);
    }
}

/// Default number of successful registrations one source may complete
/// inside [`DEFAULT_REGISTER_WINDOW`] before the register handler starts
/// refusing with 429 (#1766).
pub const DEFAULT_REGISTER_LIMIT: usize = 10;

/// Default registration rate-limit window (#1766).
pub const DEFAULT_REGISTER_WINDOW: Duration = Duration::from_secs(60);

/// Per-source rate limiter for `POST /api/identity/register` (#1766).
///
/// The identity store's admission cap bounds how many identities can
/// exist, but the register handler itself was unthrottled: a caller on
/// a non-loopback bind could burn request-rate CPU/IO and squat cap
/// slots with junk registrations. This limiter follows the
/// [`SaveRateLimiter`] pattern — a per-key map whose entries are swept
/// once they can no longer refuse, so the map holds only sources
/// active inside the current window (#1713 discipline).
///
/// The key is the request's source (the peer IP, supplied by the
/// listener's connect-info; see `identity_register_handler` in
/// `routes.rs`). Forwarded headers are never consulted: on the
/// directly exposed listener they are client-spoofable (the
/// `security.rs` doctrine). Only *successful* registrations are
/// counted — a refusal at the admission cap or a failed attempt does
/// not consume budget. The check/record split means a concurrent
/// burst can overshoot the limit by the number of requests in flight;
/// that tolerance is immaterial against the sustained-rate
/// cap-squatting this limiter exists to stop.
#[derive(Debug, Clone)]
pub struct RegisterRateLimiter {
    /// Map of source key → timestamps of successful registrations
    /// inside the current window, oldest first.
    attempts: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    /// Maximum successful registrations per source inside `window`.
    pub limit: usize,
    /// Sliding window length.
    pub window: Duration,
}

impl RegisterRateLimiter {
    /// Create a new registration rate limiter.
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            limit,
            window,
        }
    }

    /// Drop one source's aged-out timestamps; returns the number of
    /// in-window registrations remaining for it.
    fn prune(times: &mut VecDeque<Instant>, now: Instant, window: Duration) -> usize {
        while times
            .front()
            .is_some_and(|t| now.duration_since(*t) >= window)
        {
            times.pop_front();
        }
        times.len()
    }

    /// Check whether a registration from `source` is allowed now.
    ///
    /// Returns `Ok(())` if the source is under its limit, or
    /// `Err(remaining)` with the time until the oldest in-window
    /// registration ages out and a new one is allowed.
    pub async fn check(&self, source: &str) -> Result<(), Duration> {
        if self.limit == 0 {
            return Err(self.window);
        }
        let mut map = self.attempts.lock().await;
        let now = Instant::now();
        if let Some(times) = map.get_mut(source) {
            if Self::prune(times, now, self.window) >= self.limit {
                // The front is the oldest in-window registration
                // (prune kept it because it is younger than the
                // window), so the remaining wait is exact.
                let oldest = *times.front().unwrap_or(&now);
                return Err(self
                    .window
                    .checked_sub(now.duration_since(oldest))
                    .unwrap_or(Duration::ZERO));
            }
            if times.is_empty() {
                map.remove(source);
            }
        }
        Ok(())
    }

    /// Record a successful registration from `source`.
    pub async fn record(&self, source: &str) {
        let mut map = self.attempts.lock().await;
        let now = Instant::now();
        // Sweep every source's aged-out entries: they can no longer
        // refuse a registration, and keeping them would grow the map
        // by one entry for every source ever seen (#1713).
        map.retain(|_, times| Self::prune(times, now, self.window) > 0);
        map.entry(source.to_owned()).or_default().push_back(now);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod register_rate_limiter_tests {
    use super::*;

    #[tokio::test]
    async fn under_limit_is_allowed_and_over_limit_is_refused() {
        let limiter = RegisterRateLimiter::new(2, Duration::from_secs(60));
        limiter.check("203.0.113.7").await.unwrap();
        limiter.record("203.0.113.7").await;
        limiter.check("203.0.113.7").await.unwrap();
        limiter.record("203.0.113.7").await;
        let remaining = limiter.check("203.0.113.7").await.unwrap_err();
        assert!(remaining > Duration::ZERO && remaining <= Duration::from_secs(60));
    }

    #[tokio::test]
    async fn sources_are_independent() {
        let limiter = RegisterRateLimiter::new(1, Duration::from_secs(60));
        limiter.record("198.51.100.1").await;
        assert!(limiter.check("198.51.100.1").await.is_err());
        limiter.check("198.51.100.2").await.unwrap();
    }

    #[tokio::test]
    async fn window_expiry_allows_registration_again() {
        let limiter = RegisterRateLimiter::new(1, Duration::from_millis(60));
        limiter.record("192.0.2.9").await;
        assert!(limiter.check("192.0.2.9").await.is_err());
        tokio::time::sleep(Duration::from_millis(90)).await;
        limiter.check("192.0.2.9").await.unwrap();
    }

    #[tokio::test]
    async fn zero_window_never_refuses() {
        // The routes tests disable the save limiter with a ZERO
        // cooldown; a ZERO window is the same off switch here.
        let limiter = RegisterRateLimiter::new(1, Duration::ZERO);
        for _ in 0..5 {
            limiter.record("203.0.113.99").await;
            limiter.check("203.0.113.99").await.unwrap();
        }
    }
}

/// Explorer route prefixes a publication path mount must not
/// shadow (#2146 phase 4, invariant 3: no implicit shadowing).
/// Segment prefixes grounded in the routes `create_router_inner`,
/// `world_ws` and `explore` register at the time of writing: the
/// page routes, the `/api` and `/ws` trees, static assets, the
/// world page and socket defaults, and the transport-advertisement
/// well-known path. A mount conflicts when it equals a reserved
/// route, sits under one, or is a segment-prefix ancestor of one
/// (the either-direction segment-boundary relation the `id-cli`
/// publication registry's shadow check uses) -- so `/world`,
/// `/api/files` and `/` conflict, while `/worlds` does not.
pub const RESERVED_EXPLORER_ROUTES: &[&str] = &[
    "/",
    "/.well-known",
    "/api",
    "/assets",
    "/blob",
    "/blocks",
    "/edit",
    "/explore",
    "/files",
    "/groups",
    "/hash",
    "/invite",
    "/peers",
    "/settings",
    "/sw.js",
    "/view",
    "/world",
    "/ws",
];

/// Canonicalise a publication mount path, or explain why it cannot
/// be one (#2146 phase 4: canonicalize once, at mount time, and
/// reject invalid or ambiguous paths rather than normalising a
/// spelling into a different mount than the caller named).
///
/// Canonical form: starts with `/`, no empty / `.` / `..`
/// segments, no trailing slash except the root itself, and no
/// whitespace, control, or URL-delimiter (`?`, `#`, `\`)
/// characters anywhere -- the dispatcher matches mount paths
/// against the request path verbatim, so a spelling another layer
/// might read as a different path is ambiguous. `/` is a valid
/// canonical path (the reserved-route check in
/// [`PublicationMounts::mount`] is what refuses it, as a shadow
/// of the whole Explorer).
///
/// # Errors
///
/// Fails on a relative path, an empty / `.` / `..` segment, a
/// trailing slash, or a whitespace / control / delimiter
/// character.
pub fn canonicalize_mount_path(path: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        path.starts_with('/'),
        "mount path must start with '/': {path:?}"
    );
    anyhow::ensure!(
        !path
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '\\')),
        "mount path must not contain whitespace, control, or delimiter characters: {path:?}"
    );
    let mut segments = Vec::new();
    for segment in path.split('/').skip(1) {
        match segment {
            "" if segments.is_empty() && path == "/" => {}
            "" => anyhow::bail!("mount path has an empty segment: {path:?}"),
            "." | ".." => {
                anyhow::bail!("mount path must not contain '{segment}' segments: {path:?}")
            }
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return Ok("/".to_string());
    }
    Ok(format!("/{}", segments.join("/")))
}

/// Whether canonical mount path `mount` shadows reserved Explorer
/// route `route`: a segment-boundary prefix relation in either
/// direction (the mount is the route, sits under it, or contains
/// it). The root mount shadows every route.
fn mount_shadows_route(mount: &str, route: &str) -> bool {
    mount == route
        || mount == "/"
        || route.starts_with(&format!("{mount}/"))
        || mount.starts_with(&format!("{route}/"))
}

/// The first reserved Explorer route the canonical mount path
/// `mount_path` shadows, if any.
#[must_use]
pub fn shadowed_reserved_route(mount_path: &str) -> Option<&'static str> {
    RESERVED_EXPLORER_ROUTES
        .iter()
        .find(|route| mount_shadows_route(mount_path, route))
        .copied()
}

/// Dynamic publication path mounts (#2147 slice 3): mount path ->
/// world name, shared by every clone of an [`AppState`] (all web
/// listeners of one serve process) and by the serve-owned publication
/// manager's mount sink.
///
/// The Explorer router consults this table on every request, so a
/// mount applies and reverts at runtime without rebuilding any
/// router: publishing inserts a row, unpublishing removes it, and
/// the Explorer route the mount shadowed answers again. The table
/// is the live half only -- the persisted set is the publication
/// registry in `id-cli`. The table validates every mount itself
/// (canonical path, reserved-route and taken-path conflicts; see
/// [`PublicationMounts::mount`]), so a row here is always one the
/// dispatcher can honour exactly as written.
#[derive(Clone, Default)]
pub struct PublicationMounts {
    inner: Arc<std::sync::RwLock<std::collections::BTreeMap<String, String>>>,
}

impl std::fmt::Debug for PublicationMounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicationMounts")
            .field("mounts", &self.snapshot())
            .finish()
    }
}

impl PublicationMounts {
    /// An empty mount table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, std::collections::BTreeMap<String, String>> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, std::collections::BTreeMap<String, String>> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Mount `world` at `path` (#2146 phase 4).
    ///
    /// The path is canonicalised and checked here, at mount time,
    /// so no caller can insert a row the dispatcher would read
    /// differently than the caller meant:
    ///
    /// - a non-canonical path (see [`canonicalize_mount_path`])
    ///   is an error;
    /// - a path that shadows a reserved Explorer route (see
    ///   [`RESERVED_EXPLORER_ROUTES`]) is an error -- an
    ///   intentional override is possible only through the #2147
    ///   publication operation's explicit shadow acknowledgement,
    ///   which this table-level API deliberately does not grow;
    /// - a path already mounted to a *different* world is an
    ///   error, never a silent replacement. Re-mounting the same
    ///   world at the same path is idempotent. Nested mounts (a
    ///   mount under another mount, for a different world) are
    ///   not a conflict: [`Self::lookup`] prefers the longest.
    ///
    /// # Errors
    ///
    /// Fails as described above; on error the table is unchanged.
    pub fn mount(&self, world: &str, path: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            !world.trim().is_empty(),
            "mount world name must not be empty"
        );
        let canonical = canonicalize_mount_path(path)?;
        if let Some(route) = shadowed_reserved_route(&canonical) {
            anyhow::bail!(
                "mount path {canonical} shadows the reserved Explorer route {route} \
                 -- refusing to mount world '{world}' there"
            );
        }
        let mut table = self.write();
        match table.get(&canonical) {
            Some(existing) if existing != world => anyhow::bail!(
                "mount path {canonical} is already mounted to world '{existing}' \
                 -- refusing to remount it to '{world}' (unmount it first)"
            ),
            Some(_) => Ok(()),
            None => {
                table.insert(canonical, world.to_owned());
                Ok(())
            }
        }
    }

    /// Remove the mount at `path`, restoring the Explorer route.
    ///
    /// # Errors
    ///
    /// Fails when nothing is mounted at `path` -- the publication
    /// manager's `MountSink` contract relies on this: an error
    /// means the route was NOT removed, so the manager restores
    /// the persisted registry entry it had already saved away.
    pub fn unmount(&self, path: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.write().remove(path).is_some(),
            "no publication is mounted at {path}"
        );
        Ok(())
    }

    /// The mount governing `request_path`, if any: the longest
    /// mount path that equals the request path or is a whole-segment
    /// prefix of it. A root mount (`/`), were one present, would
    /// govern every path. Returns `(mount_path, world)`.
    #[must_use]
    pub fn lookup(&self, request_path: &str) -> Option<(String, String)> {
        self.read()
            .iter()
            .filter(|(mount, _)| {
                mount.as_str() == "/"
                    || request_path == mount.as_str()
                    || request_path
                        .strip_prefix(mount.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|(mount, _)| mount.len())
            .map(|(mount, world)| (mount.clone(), world.clone()))
    }

    /// A snapshot of the live mounts (mount path -> world).
    #[must_use]
    pub fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        self.read().clone()
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
    /// This node's own iroh endpoint, when the serving process wired one
    /// in (#5346): the server-to-server transfer behind the web send
    /// route (`POST /api/files/send`) runs with this endpoint's identity,
    /// exactly like the meta protocol's `Send` handler (#5345). `None`
    /// for states built outside `id serve` (tests, embedded surfaces) —
    /// the send route then answers 503 rather than inventing an identity.
    pub transfer_endpoint: Option<iroh::endpoint::Endpoint>,
    /// The transport-advertisement minter whose current signed ad
    /// the `/.well-known/id-transport-advertisement` handler serves
    /// (#8214 slice 4). `id serve` wires its node's minter in
    /// through [`web_routers_with_surface`]; `None` for states
    /// built anywhere else — the handler then answers 404 rather
    /// than inventing an ad.
    pub transport_ad: Option<Arc<id_core::transport_advertisement::AdMinter>>,
    /// The program gallery whose public read surface (list, fetch, search)
    /// this node serves under `/api/gallery` (#10003). `None` unless the
    /// serving process wires one in; every gallery route then answers 404.
    pub gallery: Option<gallery::SharedGallery>,
    /// The app manifests `POST /v1/runs` checks app runs against and
    /// `GET /v1/apps/{app}/manifest` serves (#10392 §4.5, #14226).
    /// Empty unless the serving process wires its apps in.
    pub run_apps: runs::RunApps,
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
    /// Per-source registration rate limiter (#1766).
    pub register_limiter: RegisterRateLimiter,
    /// Serializes filename-tag claims (upload name resolution, create
    /// existence check, save tag assignment) and the filename-keyed
    /// metadata writes across clones of this state (#2363, #8407).
    /// Independently constructed states, other processes and tag writers
    /// outside these handlers do not participate in this lock.
    pub(crate) upload_name_lock: Arc<Mutex<()>>,
    /// Client identity store for persistent client sessions.
    pub identity: IdentityStore,
    /// Node-signed event log used to publish durable document revisions.
    pub(crate) local_events: Arc<Mutex<id_events::local_events::LocalEvents>>,
    /// Optional hosted world (and its invite secret) for the WebSocket bridge.
    pub world: Option<id_worlds::world_hub::WorldHub>,
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
    /// Dynamic publication path mounts (#2147 slice 3), shared with
    /// the serve-owned publication manager: the live mount table
    /// the Explorer router consults per request. Empty (inert) for
    /// states that serve no publications.
    pub publication_mounts: PublicationMounts,
    /// Mark the directory explorer's session cookies `Secure`.
    pub cookie_secure: bool,
    /// The resolved group-log path for the read-only `/groups` page
    /// (`id serve` passes its `--groups-log` resolution); `None` reads
    /// the default `groups.jsonl`, as the group CLI does.
    pub groups_log: Option<std::path::PathBuf>,
    /// Seitan admission state for `/api/seitan/*` (#8487): the invite
    /// registry, the directory admissions append to, and the freshness
    /// window. `id serve` replaces the default (empty in-memory
    /// directory, default window) with the server-config directory
    /// journal and the configured window; see [`SeitanState`].
    pub seitan: SeitanState,
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
            .field("register_limiter", &self.register_limiter)
            .field("identity", &self.identity)
            .field("local_events", &"<LocalEvents>")
            .field("world", &self.world)
            .field("publication_mounts", &self.publication_mounts)
            .field("cookie_secure", &self.cookie_secure)
            .field("groups_log", &self.groups_log)
            .field("seitan", &self.seitan)
            .field("transport_ad", &self.transport_ad.is_some())
            .field("gallery", &self.gallery.is_some())
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
        let events_dir = identity_db_path.with_file_name(id_events::local_events::DIR_NAME);
        let local_events = Arc::new(Mutex::new(id_events::local_events::LocalEvents::open(
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
        local_events: Arc<Mutex<id_events::local_events::LocalEvents>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            store,
            transfer_endpoint: None,
            transport_ad: None,
            gallery: None,
            run_apps: runs::RunApps::default(),
            collab: Arc::new(CollabState::new()),
            assets: load_asset_urls(),
            peers,
            node_id,
            tag_store,
            save_limiter: SaveRateLimiter::new(DEFAULT_SAVE_COOLDOWN),
            register_limiter: RegisterRateLimiter::new(
                DEFAULT_REGISTER_LIMIT,
                DEFAULT_REGISTER_WINDOW,
            ),
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
            publication_mounts: PublicationMounts::new(),
            cookie_secure: false,
            groups_log: None,
            seitan: SeitanState::default(),
        })
    }

    /// Wire the serving node's own endpoint in as the transfer identity
    /// for the web send route (#5346). `id serve` passes its endpoint
    /// through [`web_routers_with_surface`]; see
    /// [`AppState::transfer_endpoint`].
    pub fn set_transfer_endpoint(&mut self, endpoint: Option<iroh::endpoint::Endpoint>) {
        self.transfer_endpoint = endpoint;
    }

    /// Wire the serving node's transport-advertisement minter in
    /// (#8214 slice 4). `id serve` passes its minter through
    /// [`web_routers_with_surface`]; see [`AppState::transport_ad`].
    pub fn set_transport_ad(
        &mut self,
        minter: Option<Arc<id_core::transport_advertisement::AdMinter>>,
    ) {
        self.transport_ad = minter;
    }

    /// Serve this program gallery's read surface under `/api/gallery`
    /// (#10003); see [`AppState::gallery`].
    pub fn set_gallery(&mut self, gallery: Option<gallery::SharedGallery>) {
        self.gallery = gallery;
    }

    /// Check app runs against, and serve, these app manifests; see
    /// [`AppState::run_apps`].
    pub fn set_run_apps(&mut self, apps: runs::RunApps) {
        self.run_apps = apps;
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
#[allow(clippy::too_many_arguments)]
pub async fn web_router(
    store: Store,
    peers: Option<PeerDiscovery>,
    node_id: String,
    tag_store: Arc<TagStore>,
    secret_key: [u8; 32],
    identity_db_path: std::path::PathBuf,
    security: WebSecurity,
    world: Option<id_worlds::world_hub::WorldHub>,
    cookie_secure: bool,
    groups_log: Option<std::path::PathBuf>,
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
    Ok(finish_router(
        state,
        security,
        world,
        cookie_secure,
        None,
        groups_log,
        SeitanState::default(),
    ))
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
    world: Option<id_worlds::world_hub::WorldHub>,
    cookie_secure: bool,
    groups_log: Option<std::path::PathBuf>,
    local_events: Arc<Mutex<id_events::local_events::LocalEvents>>,
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
    Ok(finish_router(
        state,
        security,
        world,
        cookie_secure,
        None,
        groups_log,
        SeitanState::default(),
    ))
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
    world: Option<id_worlds::world_hub::WorldHub>,
    cookie_secure: bool,
    local_events: Option<Arc<Mutex<id_events::local_events::LocalEvents>>>,
    surfaces: Vec<Option<String>>,
    groups_log: Option<std::path::PathBuf>,
    seitan: SeitanState,
) -> anyhow::Result<Vec<Router>> {
    let (routers, _surface) = web_routers_with_surface(
        store,
        peers,
        node_id,
        tag_store,
        secret_key,
        identity_db_path,
        securities,
        world,
        cookie_secure,
        local_events,
        surfaces,
        groups_log,
        seitan,
        None,
        None,
        None,
        None,
    )
    .await?;
    Ok(routers)
}

/// A factory for building further routers over the SAME shared
/// [`AppState`] the configured listeners were built from (#2147
/// slice 3). The identity store opens exclusively, so a dynamic
/// publication listener cannot build a second state -- it builds
/// its program router here, over a clone of the one shared state,
/// with its own per-bind [`WebSecurity`]. The factory also carries
/// the process's [`PublicationMounts`] table (the same instance
/// every router clone consults), which is how the serve-owned
/// publication manager applies path mounts to the Explorer
/// listeners without rebuilding them.
#[derive(Clone)]
pub struct WebSurfaceFactory {
    state: AppState,
    world: Option<id_worlds::world_hub::WorldHub>,
    cookie_secure: bool,
    groups_log: Option<std::path::PathBuf>,
    seitan: SeitanState,
}

impl std::fmt::Debug for WebSurfaceFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSurfaceFactory")
            .field("publication_mounts", &self.state.publication_mounts)
            .field("world", &self.world.is_some())
            .field("cookie_secure", &self.cookie_secure)
            .field("groups_log", &self.groups_log)
            .finish()
    }
}

impl WebSurfaceFactory {
    /// The shared dynamic-publication mount table.
    #[must_use]
    pub fn publication_mounts(&self) -> PublicationMounts {
        self.state.publication_mounts.clone()
    }

    /// The program-only router for `world_name`, finished with
    /// `security` -- exactly the router a configured program
    /// listener for that world would serve (#2146 phase 2), built
    /// on demand for a dynamic publication listener.
    #[must_use]
    pub fn program_router(&self, world_name: &str, security: WebSecurity) -> Router {
        finish_router(
            self.state.clone(),
            security,
            self.world.clone(),
            self.cookie_secure,
            Some(world_name.to_owned()),
            self.groups_log.clone(),
            self.seitan.clone(),
        )
    }
}

/// Like [`web_routers`], and also returns the [`WebSurfaceFactory`]
/// over the shared state, for dynamic publication listeners and
/// path mounts (#2147 slice 3).
#[allow(clippy::too_many_arguments)]
pub async fn web_routers_with_surface(
    store: Store,
    peers: Option<PeerDiscovery>,
    node_id: String,
    tag_store: Arc<TagStore>,
    secret_key: [u8; 32],
    identity_db_path: std::path::PathBuf,
    securities: Vec<WebSecurity>,
    world: Option<id_worlds::world_hub::WorldHub>,
    cookie_secure: bool,
    local_events: Option<Arc<Mutex<id_events::local_events::LocalEvents>>>,
    surfaces: Vec<Option<String>>,
    groups_log: Option<std::path::PathBuf>,
    seitan: SeitanState,
    transfer_endpoint: Option<iroh::endpoint::Endpoint>,
    transport_ad: Option<Arc<id_core::transport_advertisement::AdMinter>>,
    gallery: Option<gallery::SharedGallery>,
    run_apps: Option<runs::RunApps>,
) -> anyhow::Result<(Vec<Router>, WebSurfaceFactory)> {
    let note_snapshot_path = identity_db_path.with_file_name(NOTE_CHECKPOINT_SNAPSHOT_FILE_NAME);
    let mut state = match local_events {
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
    state.set_transfer_endpoint(transfer_endpoint);
    state.set_transport_ad(transport_ad);
    state.set_gallery(gallery);
    if let Some(apps) = run_apps {
        state.set_run_apps(apps);
    }
    // Note checkpoints granted to a group resolve against the node's
    // group log (#3819); nothing has cloned `collab` yet, so swapping it
    // here reaches every router and the surface factory. The checkpoint
    // log itself persists beside the identity store: restore its
    // snapshot (events, then policies) before anything serves, and let
    // every later mutation rewrite it. A snapshot that does not parse
    // or does not fold fails startup — never a host that silently
    // reopens scoped notes to public.
    let collab = CollabState::new()
        .with_note_group_roles(groups::note_group_roles(groups::resolve_groups_log(
            groups_log.clone(),
        )))
        .with_note_snapshot_path(note_snapshot_path);
    collab.restore_persisted_note_snapshot().await?;
    state.collab = Arc::new(collab);
    let surface = WebSurfaceFactory {
        state: state.clone(),
        world: world.clone(),
        cookie_secure,
        groups_log: groups_log.clone(),
        seitan: seitan.clone(),
    };
    let routers = securities
        .into_iter()
        .zip(surfaces)
        .map(|(security, surface)| {
            finish_router(
                state.clone(),
                security,
                world.clone(),
                cookie_secure,
                surface,
                groups_log.clone(),
                seitan.clone(),
            )
        })
        .collect();
    Ok((routers, surface))
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
    world: Option<id_worlds::world_hub::WorldHub>,
    cookie_secure: bool,
    surface: Option<String>,
    groups_log: Option<std::path::PathBuf>,
    seitan: SeitanState,
) -> Router {
    state.world = world;
    state.groups_log = groups_log;
    state.seitan = seitan;
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    type WorldClient = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn serve_test_router(router: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        addr
    }

    async fn connect_world(addr: std::net::SocketAddr, capability: &str) -> WorldClient {
        use futures_util::SinkExt as _;
        use tokio_tungstenite::tungstenite::Message;

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/world"))
            .await
            .unwrap();
        client
            .send(Message::Text(
                serde_json::json!({
                    "type": "join",
                    "capability": capability,
                    "world": "arena",
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        client
    }

    async fn next_world_frame(client: &mut WorldClient) -> serde_json::Value {
        use futures_util::StreamExt as _;
        use tokio_tungstenite::tungstenite::Message;

        let message = tokio::time::timeout(Duration::from_secs(10), client.next())
            .await
            .expect("timed out waiting for a world frame")
            .expect("world socket closed before a frame")
            .expect("world socket failed");
        match message {
            Message::Text(text) => serde_json::from_str(&text).unwrap(),
            other => panic!("expected a text world frame, got {other:?}"),
        }
    }

    async fn broadcast_chat(sender: &mut WorldClient, other: &mut WorldClient, text: &str) {
        use futures_util::SinkExt as _;
        use tokio_tungstenite::tungstenite::Message;

        sender
            .send(Message::Text(
                serde_json::json!({"type": "chat", "text": text})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        for receiver in [sender, other] {
            let event = next_world_frame(receiver).await;
            assert_eq!(event["type"], "event");
            assert!(event.to_string().contains(text), "{event}");
        }
    }

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_save_limiter_acquire_refuses_when_a_save_completes_between_check_and_reserve() {
        use std::future::Future as _;
        use std::task::{Context, Poll};

        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        let first = limiter.acquire("a.txt").await.expect("first save proceeds");

        // Orchestration (no sleeps, no timing loops — locks and FIFO
        // order force the interleaving):
        //
        // - A helper thread holds the in-flight lock, so the second
        //   acquire cannot take its reservation until the helper
        //   releases it. On the `go` signal the helper first takes
        //   and drops the cooldown lock: the second acquire's first
        //   check is queued ahead of it on that mutex (FIFO, see
        //   below), so acquiring it proves the check has passed. The
        //   helper then records the save through the real `record`
        //   path, removes the first save's name from the in-flight
        //   set (exactly the removal `SavePermit::drop` performs
        //   after `complete` records), and only then releases the
        //   in-flight lock.
        // - The test holds the cooldown lock and polls the second
        //   acquire once by hand: that poll registers it as the first
        //   waiter on the cooldown mutex, ahead of the helper.
        // - The test then drops the cooldown lock, signals `go`, and
        //   polls the second acquire again: the poll drives its check
        //   to a pass and then blocks inside the reservation on the
        //   helper's in-flight lock while the helper completes the
        //   record + release sequence above. The acquire therefore
        //   takes its reservation strictly after the record, in
        //   `complete`'s record-then-release order — the #9587 gap.
        let (locked_tx, locked_rx) = std::sync::mpsc::channel::<()>();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let holder = {
            let limiter = limiter.clone();
            std::thread::spawn(move || {
                let mut flight = limiter
                    .in_flight
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                locked_tx.send(()).unwrap();
                go_rx.recv().unwrap();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(async {
                    // Queued behind the second acquire's first check
                    // (FIFO): holding this lock proves the check ran.
                    let _observed = limiter.last_save.lock().await;
                    drop(_observed);
                    limiter.record("a.txt").await;
                });
                flight.remove("a.txt");
                drop(flight);
                done_tx.send(()).unwrap();
            })
        };
        locked_rx.recv().unwrap();

        let cooldown_guard = limiter.last_save.lock().await;
        let second_limiter = limiter.clone();
        let mut second = Box::pin(async move { second_limiter.acquire("a.txt").await });
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(&waker);
        assert!(
            matches!(second.as_mut().poll(&mut cx), Poll::Pending),
            "the second acquire waits on the held cooldown lock"
        );
        drop(cooldown_guard);
        go_tx.send(()).unwrap();

        // The second poll blocks inside the reservation until the
        // helper releases the in-flight lock; later polls only cover
        // the acquire's re-validation contending for the cooldown
        // lock the helper's record is using. The interleaving itself
        // is fixed by the locks, not by how often this loop polls.
        let mut refused = None;
        for _ in 0..100_000 {
            match second.as_mut().poll(&mut cx) {
                Poll::Ready(result) => {
                    refused = Some(result);
                    break;
                }
                Poll::Pending => std::thread::yield_now(),
            }
        }
        let refused = refused.expect("the second acquire completed");
        done_rx.recv().unwrap();
        holder.join().unwrap();

        assert!(
            refused.is_err(),
            "a save completing between check and reserve must refuse the acquire (#9587)"
        );
        assert!(
            !limiter
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains("a.txt"),
            "a refused acquire leaves no reservation behind"
        );
        drop(first);
    }

    #[tokio::test]
    async fn test_save_limiter_permit_drop_recovers_a_poisoned_in_flight_lock() {
        let limiter = SaveRateLimiter::new(Duration::from_secs(5));
        // Poison the in-flight mutex: panic while holding it.
        let in_flight = limiter.in_flight.clone();
        let poisoning = std::thread::spawn(move || {
            let _guard = in_flight.lock().unwrap();
            panic!("poisoning the in-flight lock for the test");
        });
        assert!(poisoning.join().is_err(), "the poisoning thread panicked");
        assert!(limiter.in_flight.is_poisoned());

        let permit = limiter
            .acquire("a.txt")
            .await
            .expect("acquire recovers the poisoned lock");
        drop(permit);
        assert!(
            !limiter
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains("a.txt"),
            "drop releases the reservation even on a poisoned lock (#9587)"
        );
        assert!(
            limiter.acquire("a.txt").await.is_ok(),
            "the filename is not wedged after a poisoned drop"
        );
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
        use assets::Assets;

        let urls = load_asset_urls();
        if Assets::get("manifest.json").is_none() {
            // Clean checkout: no built frontend, so the documented
            // fallback is the unhashed defaults (see load_asset_urls
            // and pkgs/id/build.rs). The test must not require a build.
            assert_eq!(urls.main_js, "/assets/main.js");
            assert_eq!(urls.styles_css, "/assets/styles.css");
            return;
        }
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

    #[tokio::test]
    async fn mixed_listener_surfaces_serve_their_pages_and_share_world_sessions() {
        use iroh::endpoint::{Endpoint, RelayMode, presets};
        use iroh_blobs::store::mem::MemStore;
        use iroh_docs::protocol::Docs;
        use iroh_gossip::net::Gossip;
        use tower::ServiceExt as _;

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
            TagStore::init_in(&docs, "mixed-listener-test", dir.path().join("meta"))
                .await
                .unwrap(),
        );
        let world = id_worlds::world::WorldHandle::spawn(
            id_worlds::world::WorldCore::new("arena", id_worlds::world::WorldLimits::default())
                .unwrap(),
        );
        let hub = id_worlds::world_hub::WorldHub::single_named(
            "arena",
            id_worlds::world_session::WorldService::new(world.clone(), None),
        );
        let routers = web_routers(
            store,
            None,
            "mixed-listener-test".to_owned(),
            tags,
            [9; 32],
            dir.path().join(".identity.redb"),
            (0..2)
                .map(|_| WebSecurity::for_bind("127.0.0.1".parse().unwrap(), None, &[]))
                .collect(),
            Some(hub),
            false,
            None,
            vec![None, Some("arena".to_owned())],
            None,
            SeitanState::default(),
        )
        .await
        .unwrap();
        assert_eq!(routers.len(), 2, "one router is returned per listener");
        let explorer = routers[0].clone();
        let program = routers[1].clone();

        let explorer_page = explorer
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/world")
                    .header(axum::http::header::HOST, "127.0.0.1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(explorer_page.status(), axum::http::StatusCode::OK);
        let program_page = program
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(axum::http::header::HOST, "127.0.0.1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(program_page.status(), axum::http::StatusCode::OK);
        let program_html = axum::body::to_bytes(program_page.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&program_html).contains("data-world-program=\"arena\""),
            "program listener should serve its configured world"
        );

        for (router, path, expected) in [
            (&explorer, "/files", axum::http::StatusCode::OK),
            (&program, "/files", axum::http::StatusCode::NOT_FOUND),
            (&explorer, "/world", axum::http::StatusCode::OK),
            (&program, "/world", axum::http::StatusCode::NOT_FOUND),
        ] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .header(axum::http::header::HOST, "127.0.0.1")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{path}");
        }

        let explorer_addr = serve_test_router(explorer).await;
        let program_addr = serve_test_router(program).await;
        let (_, explorer_capability) = world
            .issue(
                "explorer-guest".to_owned(),
                id_worlds::world::WorldScopes::GUEST,
            )
            .await
            .unwrap();
        let (_, program_capability) = world
            .issue(
                "program-guest".to_owned(),
                id_worlds::world::WorldScopes::GUEST,
            )
            .await
            .unwrap();
        let mut explorer_client = connect_world(explorer_addr, explorer_capability.expose()).await;
        let mut program_client = connect_world(program_addr, program_capability.expose()).await;
        for client in [&mut explorer_client, &mut program_client] {
            let snapshot = next_world_frame(client).await;
            assert_eq!(snapshot["type"], "snapshot");
            assert_eq!(snapshot["snapshot"]["world_id"], "arena");
        }

        broadcast_chat(&mut explorer_client, &mut program_client, "from-explorer").await;
        broadcast_chat(&mut program_client, &mut explorer_client, "from-program").await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod publication_mounts_tests {
    use super::PublicationMounts;

    #[test]
    fn lookup_matches_whole_segments_and_prefers_the_longest_mount() {
        let mounts = PublicationMounts::new();
        assert_eq!(mounts.lookup("/prog"), None);
        mounts.mount("arena", "/prog").unwrap();
        mounts.mount("deep", "/prog/deep").unwrap();
        assert_eq!(
            mounts.lookup("/prog"),
            Some(("/prog".to_owned(), "arena".to_owned()))
        );
        assert_eq!(
            mounts.lookup("/prog/"),
            Some(("/prog".to_owned(), "arena".to_owned()))
        );
        assert_eq!(
            mounts.lookup("/prog/deep/x"),
            Some(("/prog/deep".to_owned(), "deep".to_owned()))
        );
        // A merely string-prefixed path is not under the mount.
        assert_eq!(mounts.lookup("/program"), None);
        assert_eq!(mounts.lookup("/other"), None);
    }

    #[test]
    fn root_mount_is_refused_as_a_reserved_shadow_and_unmount_reports_absence() {
        let mounts = PublicationMounts::new();
        // Phase 4: `/` is canonical, but mounting there shadows the
        // reserved Explorer route `/` (and with it every route), so
        // the table refuses it and nothing is mounted.
        let error = mounts.mount("arena", "/").unwrap_err().to_string();
        assert!(error.contains("reserved Explorer route /"), "{error}");
        assert_eq!(mounts.lookup("/anything/at/all"), None);
        assert_eq!(mounts.lookup("/anything"), None);
        // The MountSink contract: unmounting what is not mounted is
        // an error (the route was not removed), never a silent Ok.
        assert!(mounts.unmount("/").is_err());
    }

    #[test]
    fn canonical_mount_paths_are_accepted() {
        for (given, canonical) in [
            ("/", "/"),
            ("/prog", "/prog"),
            ("/games/arena", "/games/arena"),
            // A `.` inside a segment is not a `.` segment.
            ("/v1.2/prog", "/v1.2/prog"),
        ] {
            assert_eq!(
                super::canonicalize_mount_path(given).unwrap(),
                canonical,
                "{given}"
            );
        }
        // Target resolution by world name: the table stores the
        // world name the mount was made with.
        let mounts = PublicationMounts::new();
        mounts.mount("arena", "/prog").unwrap();
        assert_eq!(
            mounts.snapshot().get("/prog").map(String::as_str),
            Some("arena")
        );
    }

    #[test]
    fn noncanonical_mount_paths_are_rejected_and_mount_nothing() {
        let mounts = PublicationMounts::new();
        for path in [
            "",
            "prog",
            "prog/",
            "/prog/",
            "//prog",
            "/prog//deep",
            "/./prog",
            "/prog/./deep",
            "/../prog",
            "/prog/../deep",
            "/pro g",
            "/prog\t",
            "/prog?x=1",
            "/prog#frag",
            "/prog\\deep",
            "/prog\u{7f}",
        ] {
            assert!(
                super::canonicalize_mount_path(path).is_err(),
                "canonicalize {path:?}"
            );
            assert!(mounts.mount("arena", path).is_err(), "mount {path:?}");
        }
        // An empty world name names no target either.
        assert!(mounts.mount("", "/prog").is_err());
        assert!(mounts.mount("  ", "/prog").is_err());
        assert!(mounts.snapshot().is_empty());
    }

    #[test]
    fn reserved_explorer_routes_are_refused() {
        let mounts = PublicationMounts::new();
        for path in [
            "/",
            "/.well-known",
            "/.well-known/id-transport-advertisement",
            "/api",
            "/api/files",
            "/assets",
            "/assets/app.js",
            "/blob",
            "/blocks",
            "/edit",
            "/edit/some-file",
            "/explore",
            "/files",
            "/groups",
            "/hash",
            "/invite",
            "/peers",
            "/settings",
            "/sw.js",
            "/view",
            "/world",
            "/world/anything",
            "/ws",
            "/ws/world",
        ] {
            let error = mounts.mount("arena", path).unwrap_err().to_string();
            assert!(error.contains("reserved Explorer route"), "{path}: {error}");
        }
        assert!(mounts.snapshot().is_empty());
        // Near misses are not segment-boundary shadows and mount fine.
        for path in ["/worlds", "/apis", "/asset", "/world-page", "/wsx"] {
            mounts.mount("arena", path).unwrap();
        }
    }

    #[test]
    fn remounting_a_path_to_a_different_world_is_refused() {
        let mounts = PublicationMounts::new();
        mounts.mount("arena", "/prog").unwrap();
        // Same world at the same path: idempotent.
        mounts.mount("arena", "/prog").unwrap();
        // A different world at the same path: refused, and the
        // original mount is untouched (never silently replaced).
        let error = mounts.mount("deep", "/prog").unwrap_err().to_string();
        assert!(
            error.contains("already mounted to world 'arena'"),
            "{error}"
        );
        assert_eq!(
            mounts.lookup("/prog"),
            Some(("/prog".to_owned(), "arena".to_owned()))
        );
        // A nested mount for a different world is not a conflict.
        mounts.mount("deep", "/prog/deep").unwrap();
        assert_eq!(
            mounts.lookup("/prog/deep"),
            Some(("/prog/deep".to_owned(), "deep".to_owned()))
        );
    }

    #[test]
    fn lookup_isolates_sibling_mounts_on_segment_boundaries() {
        let mounts = PublicationMounts::new();
        mounts.mount("arena", "/prog").unwrap();
        mounts.mount("solo", "/solo").unwrap();
        // Each mount resolves only its own subtree, to its own world.
        assert_eq!(
            mounts.lookup("/prog/run"),
            Some(("/prog".to_owned(), "arena".to_owned()))
        );
        assert_eq!(
            mounts.lookup("/solo/run"),
            Some(("/solo".to_owned(), "solo".to_owned()))
        );
        // A path merely string-prefixed by a mount is outside it:
        // it reaches neither that mount nor any sibling.
        assert_eq!(mounts.lookup("/prog2"), None);
        assert_eq!(mounts.lookup("/program"), None);
        assert_eq!(mounts.lookup("/sol"), None);
        assert_eq!(mounts.lookup("/other"), None);
    }
}
