//! Many worlds in one process.
//!
//! A [`WorldHub`] maps world names to [`WorldService`]s. Sessions name their
//! world in the first frame (`"world": "<name>"`, default: the hub's default
//! world) and hold a [`WorldLease`] for as long as they run:
//!
//! - Worlds **open lazily** from storage the first time they are named, with a
//!   single-flight open so concurrent first sessions share one restore.
//! - Only an **admin-authenticated** request may create a world. Every other
//!   unknown name is "unknown world", indistinguishable from a bad capability,
//!   so names cannot be probed or squatted by outsiders.
//! - **Bounds**: a ceiling on concurrent sessions and on open worlds.
//! - **Idle eviction**: a durable world without sessions is shut down after an
//!   idle period and reopens from its journal on next use, so the number of
//!   worlds that *exist* is limited by disk, not by memory.
//!
//! The hub knows nothing about files, blobs or docs: a [`WorldOpener`] builds
//! services, which keeps the hub testable and the storage policy in one place
//! (`serve`).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use serde::Serialize;
use tokio::sync::{OnceCell, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

use crate::world::Isolation;
use crate::world_session::WorldService;

/// Longest world name, in bytes.
pub const MAX_WORLD_NAME_BYTES: usize = 64;

/// Name of the world sessions get when they name none.
pub const DEFAULT_WORLD_NAME: &str = "lobby";

/// Reject names that could escape the worlds directory or confuse operators:
/// `a-z`, `0-9`, `-` and `_`, one path segment.
///
/// # Errors
///
/// Fails with a human-readable reason.
pub fn validate_world_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= MAX_WORLD_NAME_BYTES,
        "world name must be 1 to {MAX_WORLD_NAME_BYTES} characters"
    );
    ensure!(name != "." && name != "..", "world name must not be a path");
    ensure!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "world name may contain only a-z, 0-9, '-' and '_'"
    );
    Ok(())
}

/// A boxed future, so [`WorldOpener`] stays object-safe.
pub type OpenFuture<'a> = Pin<Box<dyn Future<Output = Result<Option<WorldService>>> + Send + 'a>>;

/// Builds worlds for a hub.
pub trait WorldOpener: Send + Sync + 'static {
    /// Open `name`. With `create`, a missing world is made; without it a
    /// missing world is `Ok(None)`.
    fn open<'a>(&'a self, name: &'a str, create: bool) -> OpenFuture<'a>;

    /// Whether `name` exists in storage (without opening it).
    fn exists(&self, name: &str) -> bool;

    /// Names of the worlds in storage.
    fn list(&self) -> Vec<String>;

    /// Whether a closed world can be reopened from storage. Only durable
    /// worlds are ever evicted.
    fn durable(&self) -> bool;
}

/// Ceilings a hub enforces.
#[derive(Clone, Copy, Debug)]
pub struct HubLimits {
    /// Most worlds open at once (idle durable worlds are evicted to make room).
    pub max_open_worlds: usize,
    /// Most sessions at once, across all worlds.
    pub max_sessions: usize,
    /// How long after the hub starts signed requests are refused, because a
    /// restart empties the replay memory. `ZERO` refuses none.
    pub signed_boot_window: Duration,
}

impl Default for HubLimits {
    fn default() -> Self {
        Self {
            max_open_worlds: 256,
            max_sessions: 1024,
            signed_boot_window: Duration::ZERO,
        }
    }
}

/// Why a world could not be leased.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// No such world (or the caller may not create it).
    UnknownWorld,
    /// The name is not a valid world name.
    InvalidName,
    /// The server is at its session ceiling.
    Busy,
    /// The server is at its open-world ceiling and none is idle.
    TooManyWorlds,
    /// Storage failed to open the world.
    Unavailable,
}

impl ResolveError {
    /// Message safe to show a client.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownWorld => "unknown world",
            Self::InvalidName => "invalid world name",
            Self::Busy => "server is busy; try again shortly",
            Self::TooManyWorlds => "server has too many worlds open; try again shortly",
            Self::Unavailable => "world is unavailable",
        }
    }
}

/// One world in a listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldInfo {
    /// World name.
    pub name: String,
    /// Whether it is currently open in memory.
    pub open: bool,
}

struct Slot {
    service: OnceCell<WorldService>,
    leases: AtomicUsize,
    last_active: StdMutex<Instant>,
}

impl Slot {
    fn new() -> Self {
        Self {
            service: OnceCell::new(),
            leases: AtomicUsize::new(0),
            last_active: StdMutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        if let Ok(mut last) = self.last_active.lock() {
            *last = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_active
            .lock()
            .map_or(Duration::ZERO, |last| last.elapsed())
    }
}

struct HubInner {
    opener: Option<Arc<dyn WorldOpener>>,
    admin_token: Option<String>,
    default_world: String,
    limits: HubLimits,
    sessions: Arc<Semaphore>,
    slots: StdMutex<HashMap<String, Arc<Slot>>>,
    replays: StdMutex<crate::directory_auth::ReplayGuard>,
}

/// Registry of worlds. Cheap to clone.
#[derive(Clone)]
pub struct WorldHub {
    inner: Arc<HubInner>,
}

impl std::fmt::Debug for WorldHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldHub")
            .field("default_world", &self.inner.default_world)
            .field("open", &self.open_count())
            .finish_non_exhaustive()
    }
}

impl From<WorldService> for WorldHub {
    fn from(service: WorldService) -> Self {
        Self::single(service)
    }
}

/// A session's claim on one open world; releasing it makes the world
/// eligible for idle eviction again.
pub struct WorldLease {
    service: WorldService,
    name: String,
    slot: Arc<Slot>,
    _permit: OwnedSemaphorePermit,
}

impl std::fmt::Debug for WorldLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldLease")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl WorldLease {
    /// The leased world.
    #[must_use]
    pub const fn service(&self) -> &WorldService {
        &self.service
    }

    /// The world's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl WorldLease {
    /// Whether this session may read `owner`'s directory.
    ///
    /// # Errors
    ///
    /// See [`WorldLease::authorize`].
    pub async fn authorize_read(&self, owner: &Self) -> Result<(), ResolveError> {
        self.authorize(owner, CrossAccess::Read).await
    }

    /// Whether this session's world may write into `owner`.
    ///
    /// # Errors
    ///
    /// See [`WorldLease::authorize`].
    pub async fn authorize_write(&self, owner: &Self) -> Result<(), ResolveError> {
        self.authorize(owner, CrossAccess::Write).await
    }

    /// The single cross-world gate. Same-world access always passes. A denial
    /// is reported as an unknown world, like a missing one.
    ///
    /// # Errors
    ///
    /// [`ResolveError::UnknownWorld`] when denied; [`ResolveError::Unavailable`]
    /// when either world cannot answer.
    async fn authorize(&self, owner: &Self, access: CrossAccess) -> Result<(), ResolveError> {
        if self.name == owner.name {
            return Ok(());
        }
        let reader = self
            .service
            .world()
            .policy()
            .await
            .map_err(|_| ResolveError::Unavailable)?;
        let target = owner
            .service
            .world()
            .policy()
            .await
            .map_err(|_| ResolveError::Unavailable)?;
        if cross_world_allowed(
            access,
            reader.isolation,
            target.isolation,
            target.cross_world_write,
        ) {
            Ok(())
        } else {
            Err(ResolveError::UnknownWorld)
        }
    }
}

/// What one world asks of another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossAccess {
    /// Reading the other world's directory.
    Read,
    /// Writing into the other world.
    Write,
}

/// The cross-world rule. A read needs both worlds unisolated. A write also
/// needs the target's cross-world write setting on.
#[must_use]
pub const fn cross_world_allowed(
    access: CrossAccess,
    reader: Isolation,
    owner: Isolation,
    owner_write: bool,
) -> bool {
    let both_unisolated = matches!(
        (reader, owner),
        (Isolation::Unisolated, Isolation::Unisolated)
    );
    match access {
        CrossAccess::Read => both_unisolated,
        CrossAccess::Write => both_unisolated && owner_write,
    }
}

impl Drop for WorldLease {
    fn drop(&mut self) {
        self.slot.touch();
        self.slot.leases.fetch_sub(1, Ordering::AcqRel);
    }
}

impl WorldHub {
    /// A hub that builds worlds with `opener`.
    #[must_use]
    pub fn new(
        opener: Arc<dyn WorldOpener>,
        admin_token: Option<String>,
        default_world: impl Into<String>,
        limits: HubLimits,
    ) -> Self {
        Self::build(Some(opener), admin_token, default_world.into(), limits)
    }

    /// A hub around one existing world, named [`DEFAULT_WORLD_NAME`]: it can
    /// neither create nor evict worlds.
    #[must_use]
    pub fn single(service: WorldService) -> Self {
        Self::single_named(DEFAULT_WORLD_NAME, service)
    }

    /// Like [`Self::single`] with an explicit name.
    #[must_use]
    pub fn single_named(name: &str, service: WorldService) -> Self {
        let hub = Self::build(
            None,
            service.admin_token().map(str::to_owned),
            name.to_owned(),
            HubLimits::default(),
        );
        let slot = Arc::new(Slot::new());
        let _ = slot.service.set(service);
        if let Ok(mut slots) = hub.inner.slots.lock() {
            slots.insert(name.to_owned(), slot);
        }
        hub
    }

    fn build(
        opener: Option<Arc<dyn WorldOpener>>,
        admin_token: Option<String>,
        default_world: String,
        limits: HubLimits,
    ) -> Self {
        Self {
            inner: Arc::new(HubInner {
                opener,
                admin_token,
                default_world,
                limits,
                sessions: Arc::new(Semaphore::new(limits.max_sessions.max(1))),
                slots: StdMutex::new(HashMap::new()),
                replays: StdMutex::new(crate::directory_auth::ReplayGuard::after_boot(
                    crate::world::unix_ms(),
                    limits.signed_boot_window,
                )),
            }),
        }
    }

    /// Name sessions get when they name no world.
    #[must_use]
    pub fn default_world(&self) -> &str {
        &self.inner.default_world
    }

    /// Whether `token` is the process-wide admin token.
    #[must_use]
    pub fn admin_ok(&self, token: &str) -> bool {
        self.inner
            .admin_token
            .as_deref()
            .filter(|expected| !expected.is_empty())
            .is_some_and(|expected| {
                crate::world_session::secret_eq(expected.as_bytes(), token.as_bytes())
            })
    }

    /// The key a signed request proves, once its signature and time check and
    /// its nonce is unused.
    ///
    /// # Errors
    ///
    /// Fails with an unauthenticated refusal for a bad, stale, or reused request,
    /// and with a rate-limited refusal during the boot window.
    pub fn verify_signed(
        &self,
        signed: &crate::directory_auth::Signed,
        method: &str,
        target: &str,
        body: &[u8],
    ) -> Result<String> {
        let mut replays = self
            .inner
            .replays
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::directory_auth::verify_signed(
            &mut replays,
            signed,
            method,
            target,
            body,
            crate::world::unix_ms(),
        )
    }

    /// Number of worlds currently open.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.inner.slots.lock().map_or(0, |slots| slots.len())
    }

    /// Claim `name` (default world when `None`) for a session.
    ///
    /// `admin` is the admin token the request carries, if any: a valid one
    /// may create a world that does not exist yet.
    ///
    /// # Errors
    ///
    /// See [`ResolveError`].
    pub async fn lease(
        &self,
        name: Option<&str>,
        admin: Option<&str>,
    ) -> Result<WorldLease, ResolveError> {
        let name = name.unwrap_or(&self.inner.default_world);
        validate_world_name(name).map_err(|_| ResolveError::InvalidName)?;
        let permit = Arc::clone(&self.inner.sessions)
            .try_acquire_owned()
            .map_err(|_| ResolveError::Busy)?;
        let create = admin.is_some_and(|token| self.admin_ok(token));
        self.lease_with(name, create, permit).await
    }

    async fn lease_with(
        &self,
        name: &str,
        create: bool,
        permit: OwnedSemaphorePermit,
    ) -> Result<WorldLease, ResolveError> {
        let (slot, evicted) = self.acquire_slot(name, create)?;
        for service in evicted {
            let _ = service.world().shutdown().await;
        }
        let opened = slot
            .service
            .get_or_try_init(|| async {
                let Some(opener) = self.inner.opener.as_ref() else {
                    return Err(ResolveError::UnknownWorld);
                };
                match opener.open(name, create).await {
                    Ok(Some(service)) => Ok(service),
                    Ok(None) => Err(ResolveError::UnknownWorld),
                    Err(error) => {
                        tracing::error!("world {name}: open failed: {error:#}");
                        Err(ResolveError::Unavailable)
                    }
                }
            })
            .await;
        match opened {
            Ok(service) => Ok(WorldLease {
                service: service.clone(),
                name: name.to_owned(),
                slot,
                _permit: permit,
            }),
            Err(error) => {
                slot.leases.fetch_sub(1, Ordering::AcqRel);
                if let Ok(mut slots) = self.inner.slots.lock()
                    && slot.leases.load(Ordering::Acquire) == 0
                    && slot.service.get().is_none()
                    && slots.get(name).is_some_and(|held| Arc::ptr_eq(held, &slot))
                {
                    slots.remove(name);
                }
                Err(error)
            }
        }
    }

    /// Find or create the slot for `name` and count a lease on it. Returns
    /// services evicted to make room, to be shut down by the caller.
    fn acquire_slot(
        &self,
        name: &str,
        create: bool,
    ) -> Result<(Arc<Slot>, Vec<WorldService>), ResolveError> {
        let mut slots = self
            .inner
            .slots
            .lock()
            .map_err(|_| ResolveError::Unavailable)?;
        if let Some(slot) = slots.get(name) {
            // Counted under the map lock so eviction cannot slip in between.
            slot.leases.fetch_add(1, Ordering::AcqRel);
            slot.touch();
            return Ok((Arc::clone(slot), Vec::new()));
        }
        let opener = self
            .inner
            .opener
            .as_ref()
            .ok_or(ResolveError::UnknownWorld)?;
        if !create && !opener.exists(name) {
            return Err(ResolveError::UnknownWorld);
        }
        let mut evicted = Vec::new();
        while slots.len() >= self.inner.limits.max_open_worlds.max(1) {
            let victim = opener
                .durable()
                .then(|| {
                    slots
                        .iter()
                        .filter(|(_, slot)| {
                            slot.leases.load(Ordering::Acquire) == 0 && slot.service.get().is_some()
                        })
                        .max_by_key(|(_, slot)| slot.idle_for())
                        .map(|(key, _)| key.clone())
                })
                .flatten();
            let Some(victim) = victim else {
                return Err(ResolveError::TooManyWorlds);
            };
            if let Some(slot) = slots.remove(&victim)
                && let Some(service) = slot.service.get()
            {
                evicted.push(service.clone());
            }
        }
        let slot = Arc::new(Slot::new());
        slot.leases.store(1, Ordering::Release);
        slots.insert(name.to_owned(), Arc::clone(&slot));
        Ok((slot, evicted))
    }

    /// Open the default world (creating it), for startup.
    ///
    /// # Errors
    ///
    /// Fails if the world cannot be opened.
    pub async fn open_default(&self) -> Result<WorldService> {
        let permit = Arc::clone(&self.inner.sessions)
            .acquire_owned()
            .await
            .map_err(|e| anyhow::anyhow!("session limiter closed: {e}"))?;
        let name = self.inner.default_world.clone();
        let lease = self
            .lease_with(&name, true, permit)
            .await
            .map_err(|e| anyhow::anyhow!("open default world: {}", e.message()))?;
        Ok(lease.service().clone())
    }

    /// Create `name` if it does not exist. Returns whether it was created.
    ///
    /// # Errors
    ///
    /// See [`ResolveError`]; hubs without an opener cannot create worlds.
    pub async fn create(&self, name: &str) -> Result<bool, ResolveError> {
        validate_world_name(name).map_err(|_| ResolveError::InvalidName)?;
        let opener = self
            .inner
            .opener
            .as_ref()
            .ok_or(ResolveError::UnknownWorld)?;
        let existed = opener.exists(name)
            || self
                .inner
                .slots
                .lock()
                .is_ok_and(|slots| slots.contains_key(name));
        let permit = Arc::clone(&self.inner.sessions)
            .try_acquire_owned()
            .map_err(|_| ResolveError::Busy)?;
        let _lease = self.lease_with(name, true, permit).await?;
        Ok(!existed)
    }

    /// Worlds in storage plus any open ones, sorted by name.
    #[must_use]
    pub fn list(&self) -> Vec<WorldInfo> {
        let mut names: std::collections::BTreeMap<String, bool> = self
            .inner
            .opener
            .as_ref()
            .map(|opener| opener.list())
            .unwrap_or_default()
            .into_iter()
            .map(|name| (name, false))
            .collect();
        if let Ok(slots) = self.inner.slots.lock() {
            for (name, slot) in slots.iter() {
                if slot.service.get().is_some() {
                    names.insert(name.clone(), true);
                }
            }
        }
        names
            .into_iter()
            .map(|(name, open)| WorldInfo { name, open })
            .collect()
    }

    /// Shut down durable worlds that no session has used for `idle`. They
    /// reopen from their journal when next named. Returns how many closed.
    pub async fn evict_idle(&self, idle: Duration) -> usize {
        if !self.inner.opener.as_ref().is_some_and(|o| o.durable()) {
            return 0;
        }
        let victims: Vec<WorldService> = {
            let Ok(mut slots) = self.inner.slots.lock() else {
                return 0;
            };
            let names: Vec<String> = slots
                .iter()
                .filter(|(_, slot)| {
                    slot.service.get().is_some()
                        && slot.leases.load(Ordering::Acquire) == 0
                        && slot.idle_for() >= idle
                })
                .map(|(name, _)| name.clone())
                .collect();
            names
                .into_iter()
                .filter_map(|name| slots.remove(&name))
                .filter_map(|slot| slot.service.get().cloned())
                .collect()
        };
        let closed = victims.len();
        for service in victims {
            let _ = service.world().shutdown().await;
        }
        closed
    }

    /// Run [`Self::evict_idle`] every `period` until the hub is dropped.
    pub fn spawn_evictor(&self, period: Duration, idle: Duration) -> JoinHandle<()> {
        let weak: Weak<HubInner> = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let closed = Self { inner }.evict_idle(idle).await;
                if closed > 0 {
                    tracing::info!("world hub: closed {closed} idle world(s)");
                }
            }
        })
    }

    /// Shut every open world down (process exit).
    pub async fn shutdown_all(&self) {
        let services: Vec<WorldService> = self
            .inner
            .slots
            .lock()
            .map(|mut slots| {
                slots
                    .drain()
                    .filter_map(|(_, slot)| slot.service.get().cloned())
                    .collect()
            })
            .unwrap_or_default();
        for service in services {
            let _ = service.world().shutdown().await;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod testing {
    use super::*;
    use crate::world::{WorldCore, WorldHandle, WorldLimits};

    /// In-memory worlds; `known` simulates worlds that exist in storage.
    pub(crate) struct MemoryOpener {
        pub(crate) known: StdMutex<Vec<String>>,
        pub(crate) opens: AtomicUsize,
        pub(crate) durable: bool,
    }

    impl MemoryOpener {
        pub(crate) fn new(durable: bool) -> Arc<Self> {
            Arc::new(Self {
                known: StdMutex::new(Vec::new()),
                opens: AtomicUsize::new(0),
                durable,
            })
        }
    }

    impl WorldOpener for MemoryOpener {
        fn open<'a>(&'a self, name: &'a str, create: bool) -> OpenFuture<'a> {
            Box::pin(async move {
                self.opens.fetch_add(1, Ordering::SeqCst);
                // Make concurrent first opens overlap.
                tokio::time::sleep(Duration::from_millis(20)).await;
                let mut known = self.known.lock().unwrap();
                if !known.iter().any(|n| n == name) {
                    if !create {
                        return Ok(None);
                    }
                    known.push(name.to_owned());
                }
                let world = WorldHandle::spawn(WorldCore::new(name, WorldLimits::default())?);
                Ok(Some(WorldService::new(world, Some("admin".to_owned()))))
            })
        }

        fn exists(&self, name: &str) -> bool {
            self.known.lock().unwrap().iter().any(|n| n == name)
        }

        fn list(&self) -> Vec<String> {
            self.known.lock().unwrap().clone()
        }

        fn durable(&self) -> bool {
            self.durable
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::testing::MemoryOpener;
    use super::*;
    use crate::world::{Authority, WorldCore, WorldHandle, WorldLimits, WorldScopes};

    fn new_hub(opener: &Arc<MemoryOpener>, limits: HubLimits) -> WorldHub {
        let opener: Arc<dyn WorldOpener> = opener.clone();
        WorldHub::new(opener, Some("admin".to_owned()), "lobby", limits)
    }

    #[test]
    fn cross_world_truth_table() {
        use CrossAccess::{Read, Write};
        use Isolation::{Isolated, Unisolated};
        for owner_write in [false, true] {
            // Reads ignore the target's write setting.
            assert!(cross_world_allowed(
                Read,
                Unisolated,
                Unisolated,
                owner_write
            ));
            assert!(!cross_world_allowed(
                Read,
                Unisolated,
                Isolated,
                owner_write
            ));
            assert!(!cross_world_allowed(
                Read,
                Isolated,
                Unisolated,
                owner_write
            ));
            assert!(!cross_world_allowed(Read, Isolated, Isolated, owner_write));
        }
        // A write needs the writer unisolated, the target unisolated, and its setting on.
        for (reader, owner, owner_write, expected) in [
            (Unisolated, Unisolated, true, true),
            (Unisolated, Unisolated, false, false),
            (Unisolated, Isolated, true, false),
            (Unisolated, Isolated, false, false),
            (Isolated, Unisolated, true, false),
            (Isolated, Unisolated, false, false),
            (Isolated, Isolated, true, false),
            (Isolated, Isolated, false, false),
        ] {
            assert_eq!(
                cross_world_allowed(Write, reader, owner, owner_write),
                expected,
                "write from {reader:?} into {owner:?} with write {owner_write}"
            );
        }
    }

    #[tokio::test]
    async fn authorize_read_follows_isolation() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        let arena = hub.lease(Some("arena"), Some("admin")).await.unwrap();
        let garden = hub.lease(Some("garden"), Some("admin")).await.unwrap();
        assert_eq!(arena.authorize_read(&arena).await, Ok(()));
        assert_eq!(
            arena.authorize_read(&garden).await,
            Err(ResolveError::UnknownWorld)
        );

        arena
            .service()
            .world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        assert_eq!(
            arena.authorize_read(&garden).await,
            Err(ResolveError::UnknownWorld),
            "the owner must be unisolated too"
        );
        assert_eq!(
            garden.authorize_read(&arena).await,
            Err(ResolveError::UnknownWorld),
            "an isolated reader sees nothing"
        );

        garden
            .service()
            .world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        assert_eq!(arena.authorize_read(&garden).await, Ok(()));
        assert_eq!(garden.authorize_read(&arena).await, Ok(()));

        arena
            .service()
            .world()
            .set_isolation(Authority::Admin, Isolation::Isolated)
            .await
            .unwrap();
        assert_eq!(
            garden.authorize_read(&arena).await,
            Err(ResolveError::UnknownWorld)
        );
    }

    #[test]
    fn names_are_one_safe_path_segment() {
        for ok in ["lobby", "a", "tic-tac_toe9", &"x".repeat(64)] {
            validate_world_name(ok).unwrap();
        }
        for bad in [
            "",
            ".",
            "..",
            "../evil",
            "a/b",
            "A",
            "sp ace",
            &"x".repeat(65),
        ] {
            assert!(validate_world_name(bad).is_err(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn only_an_admin_can_create_a_world() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        assert_eq!(
            hub.lease(Some("arena"), None).await.err(),
            Some(ResolveError::UnknownWorld)
        );
        assert_eq!(
            hub.lease(Some("arena"), Some("wrong")).await.err(),
            Some(ResolveError::UnknownWorld),
            "a bad token is no better than none, and says nothing more"
        );
        assert_eq!(opener.opens.load(Ordering::SeqCst), 0);
        let lease = hub.lease(Some("arena"), Some("admin")).await.unwrap();
        assert_eq!(lease.name(), "arena");
        drop(lease);
        // Once it exists, anyone may name it (a capability is still needed).
        hub.lease(Some("arena"), None).await.unwrap();
        assert_eq!(
            hub.lease(Some("../etc"), Some("admin")).await.err(),
            Some(ResolveError::InvalidName)
        );
    }

    #[tokio::test]
    async fn authorize_write_needs_the_target_setting() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        let arena = hub.lease(Some("arena"), Some("admin")).await.unwrap();
        let garden = hub.lease(Some("garden"), Some("admin")).await.unwrap();
        for world in [&arena, &garden] {
            world
                .service()
                .world()
                .set_isolation(Authority::Admin, Isolation::Unisolated)
                .await
                .unwrap();
        }
        assert_eq!(
            garden.authorize_write(&arena).await,
            Err(ResolveError::UnknownWorld),
            "the target's cross-world write is off by default"
        );
        arena
            .service()
            .world()
            .set_cross_world_write(Authority::Admin, true)
            .await
            .unwrap();
        assert_eq!(garden.authorize_write(&arena).await, Ok(()));
        assert_eq!(
            arena.authorize_write(&garden).await,
            Err(ResolveError::UnknownWorld),
            "garden's write setting is still off"
        );

        garden
            .service()
            .world()
            .set_isolation(Authority::Admin, Isolation::Isolated)
            .await
            .unwrap();
        assert_eq!(
            garden.authorize_write(&arena).await,
            Err(ResolveError::UnknownWorld),
            "an isolated writer writes nowhere"
        );
        garden
            .service()
            .world()
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        arena
            .service()
            .world()
            .set_isolation(Authority::Admin, Isolation::Isolated)
            .await
            .unwrap();
        assert_eq!(
            garden.authorize_write(&arena).await,
            Err(ResolveError::UnknownWorld),
            "an isolated target takes no writes"
        );
    }

    #[tokio::test]
    async fn worlds_are_isolated_from_each_other() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        let a = hub.lease(Some("a"), Some("admin")).await.unwrap();
        let b = hub.lease(Some("b"), Some("admin")).await.unwrap();
        let (_, token_a) = a
            .service()
            .world()
            .issue("ann", WorldScopes::GUEST)
            .await
            .unwrap();
        assert!(
            a.service()
                .world()
                .participant_id(token_a.clone())
                .await
                .is_ok()
        );
        assert!(
            b.service()
                .world()
                .participant_id(token_a.clone())
                .await
                .is_err(),
            "a capability for world a means nothing in world b"
        );
        a.service()
            .world()
            .chat(token_a, "only in a")
            .await
            .unwrap();
        let (_, token_b) = b
            .service()
            .world()
            .issue("bea", WorldScopes::GUEST)
            .await
            .unwrap();
        assert!(
            b.service()
                .world()
                .snapshot(token_b)
                .await
                .unwrap()
                .events
                .is_empty()
        );
    }

    #[tokio::test]
    async fn concurrent_first_sessions_share_one_open() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let hub = hub.clone();
            tasks.spawn(async move { hub.lease(Some("crowd"), Some("admin")).await.map(|_| ()) });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        assert_eq!(opener.opens.load(Ordering::SeqCst), 1, "single-flight open");
        assert_eq!(hub.open_count(), 1);
    }

    #[tokio::test]
    async fn the_session_ceiling_refuses_then_recovers() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(
            &opener,
            HubLimits {
                max_sessions: 2,
                ..HubLimits::default()
            },
        );
        let first = hub.lease(Some("a"), Some("admin")).await.unwrap();
        let _second = hub.lease(Some("a"), Some("admin")).await.unwrap();
        assert_eq!(
            hub.lease(Some("a"), Some("admin")).await.err(),
            Some(ResolveError::Busy)
        );
        drop(first);
        hub.lease(Some("a"), Some("admin")).await.unwrap();
    }

    #[tokio::test]
    async fn open_world_ceiling_refuses_when_nothing_is_evictable() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(
            &opener,
            HubLimits {
                max_open_worlds: 2,
                ..HubLimits::default()
            },
        );
        let _a = hub.lease(Some("a"), Some("admin")).await.unwrap();
        let _b = hub.lease(Some("b"), Some("admin")).await.unwrap();
        assert_eq!(
            hub.lease(Some("c"), Some("admin")).await.err(),
            Some(ResolveError::TooManyWorlds),
            "ephemeral worlds can never be evicted"
        );
        // Existing worlds are still reachable.
        hub.lease(Some("a"), None).await.unwrap();
    }

    #[tokio::test]
    async fn idle_durable_worlds_make_room_for_new_ones() {
        let opener = MemoryOpener::new(true);
        let hub = new_hub(
            &opener,
            HubLimits {
                max_open_worlds: 2,
                ..HubLimits::default()
            },
        );
        drop(hub.lease(Some("a"), Some("admin")).await.unwrap());
        let _b = hub.lease(Some("b"), Some("admin")).await.unwrap();
        // "a" is idle, "b" is leased: "c" evicts "a".
        let _c = hub.lease(Some("c"), Some("admin")).await.unwrap();
        assert_eq!(hub.open_count(), 2);
        let listed: Vec<_> = hub.list().into_iter().map(|w| (w.name, w.open)).collect();
        assert_eq!(
            listed,
            vec![
                ("a".to_owned(), false),
                ("b".to_owned(), true),
                ("c".to_owned(), true)
            ]
        );
        // "a" still exists and reopens on demand (evicting nothing leased).
        drop(_b);
        hub.lease(Some("a"), None).await.unwrap();
    }

    #[tokio::test]
    async fn evict_idle_closes_only_unleased_durable_worlds() {
        let opener = MemoryOpener::new(true);
        let hub = new_hub(&opener, HubLimits::default());
        drop(hub.lease(Some("idle"), Some("admin")).await.unwrap());
        let busy = hub.lease(Some("busy"), Some("admin")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(hub.evict_idle(Duration::from_millis(10)).await, 1);
        assert_eq!(hub.open_count(), 1);
        assert!(
            busy.service().world().shutdown().await.is_ok(),
            "the leased world kept running"
        );

        let ephemeral = MemoryOpener::new(false);
        let hub = new_hub(&ephemeral, HubLimits::default());
        drop(hub.lease(Some("x"), Some("admin")).await.unwrap());
        assert_eq!(
            hub.evict_idle(Duration::ZERO).await,
            0,
            "never evicts ephemeral worlds"
        );
    }

    #[tokio::test]
    async fn a_single_world_hub_serves_only_that_world() {
        let service = WorldService::new(
            WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap()),
            Some("admin".to_owned()),
        );
        let hub = WorldHub::single(service);
        hub.lease(None, None).await.unwrap();
        hub.lease(Some("lobby"), None).await.unwrap();
        assert_eq!(
            hub.lease(Some("other"), Some("admin")).await.err(),
            Some(ResolveError::UnknownWorld),
            "a single-world hub cannot create worlds"
        );
        assert!(hub.admin_ok("admin") && !hub.admin_ok("nope"));
    }

    #[tokio::test]
    async fn create_reports_whether_the_world_is_new() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        assert!(hub.create("fresh").await.unwrap());
        assert!(!hub.create("fresh").await.unwrap());
        assert_eq!(
            hub.create("../x").await.err(),
            Some(ResolveError::InvalidName)
        );
    }
}
