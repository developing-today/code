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

use crate::account_charge::AccountWorldQuota;
use crate::world::{
    Isolation, PortalDelivery, PortalDeliveryFuture, PortalGrants, PortalSendError,
    ScheduleDelivery, ScheduleDeliveryFuture,
};
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

    /// The hub shut `name` down after an eviction (idle, or to make room
    /// for another world); it reopens from storage when next named. Called
    /// once the world's actor has drained. A host-mode opener records the
    /// world asleep here (#10693); the default does nothing.
    fn closed(&self, _name: &str) {}

    /// Whether an operator pinned `name` always-live: the hub never closes
    /// a pinned world for idleness or to make room for another world
    /// (#10694). Asked while the hub's slot map is locked, so it must be
    /// cheap and must not call back into the hub. The default pins none.
    fn pinned(&self, _name: &str) -> bool {
        false
    }
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
    /// Most worlds one account may hold at once (#1067, F12.10
    /// per-account quotas). `0` = no per-account bound: accounts
    /// share the hub-wide ceiling under the existing rules alone,
    /// exactly as before. Mirrors
    /// [`id_caps::account_quota::AccountQuotas::max_worlds`]; a
    /// nonzero value enforces that quota at open/create for calls
    /// that carry an account.
    pub max_worlds_per_account: u32,
}

impl Default for HubLimits {
    fn default() -> Self {
        Self {
            max_open_worlds: 256,
            max_sessions: 1024,
            signed_boot_window: Duration::ZERO,
            max_worlds_per_account: 0,
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
    /// Whether it is currently open in memory. This says nothing about whether the
    /// world is open to join: a stored world opens when it is next named. The
    /// directory JSON ([`directory_json`]) names this field `loaded`, for what it
    /// means; `open` remains the session-wire name for this struct.
    pub open: bool,
    /// How many sessions are live on this world right now (the hub's lease
    /// count for the world; 0 for a stored world). This is a count only: the
    /// directory never names who is present (#812 shape ruling, 2026-10-29).
    pub presence: usize,
}

/// The JSON shape of a world directory listing: the shared serializer for the
/// directory feed (#812). The CLI's `id world list --json` prints exactly this,
/// and the web UI's directory surface is expected to consume the same shape.
///
/// The shape is `{ "default": <name>, "worlds": [{ "name": ..., "loaded": ...,
/// "default": ..., "presence": ... }] }`. The per-world field for
/// [`WorldInfo::open`] is named `loaded`, because that is what the value
/// means — the world is currently open in memory — and not open-to-join: a
/// world with `loaded: false` is stored and opens when selected. `presence`
/// is the count of sessions live on the world right now (#812): a count
/// only, never an identity — who is present is not directory data.
#[must_use]
pub fn directory_json(worlds: &[WorldInfo], default: &str) -> serde_json::Value {
    let worlds: Vec<serde_json::Value> = worlds
        .iter()
        .map(|world| {
            serde_json::json!({
                "name": world.name,
                "loaded": world.open,
                "default": world.name == default,
                "presence": world.presence,
            })
        })
        .collect();
    serde_json::json!({
        "default": default,
        "worlds": worlds,
    })
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
    /// Worlds attributed to accounts for the per-account world-count
    /// quota (#1067, F12.10). Taken only under (or after) the slots
    /// lock, never before it, so the two locks cannot deadlock.
    account_worlds: StdMutex<AccountWorldQuota>,
}

/// Registry of worlds. Cheap to clone.
///
/// A hub may carry a *pin* (#5728): a view of the same registry that
/// names exactly one world. Every world-resolving operation on a
/// pinned view — [`WorldHub::lease`], [`WorldHub::create`],
/// [`WorldHub::list`], [`WorldHub::default_world`] — is confined to
/// the pinned world; naming any other world behaves exactly as if
/// that world did not exist. The program-only router
/// (`create_program_router`) serves its session bridge through a
/// pinned view, so a listener bound to one world cannot be used to
/// reach another, whichever bridge entry point a request takes
/// (HTTP handlers, explore pages, websocket sessions). The pin lives
/// here, at the one choke point every bridge path resolves through,
/// rather than as a check each handler must remember to make.
#[derive(Clone)]
pub struct WorldHub {
    inner: Arc<HubInner>,
    pin: Option<Arc<str>>,
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

/// Releases the lease `acquire_slot` counted if `lease_with` leaves without
/// handing it to a [`WorldLease`] — on the error path, and on cancellation
/// of the open await (for example the envelope acceptor's per-stream
/// deadline dropping a callback mid-lease). Without this, a cancelled open
/// leaks the count forever: the slot can never be evicted, and an
/// uninitialized slot is never removed from the map.
struct SlotLeaseGuard {
    hub: WorldHub,
    name: String,
    slot: Arc<Slot>,
    /// The account this acquire acted for, if any.
    account: Option<String>,
    /// Whether this acquire recorded a new world hold for `account`
    /// (rolled back if the guard drops while armed).
    noted: bool,
    armed: bool,
}

impl Drop for SlotLeaseGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.noted
            && let Some(account) = self.account.as_deref()
            && let Ok(mut worlds) = self.hub.inner.account_worlds.lock()
        {
            worlds.release(account, &self.name);
        }
        self.slot.leases.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut slots) = self.hub.inner.slots.lock()
            && self.slot.leases.load(Ordering::Acquire) == 0
            && self.slot.service.get().is_none()
            && slots
                .get(&self.name)
                .is_some_and(|held| Arc::ptr_eq(held, &self.slot))
        {
            slots.remove(&self.name);
        }
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
            pin: None,
            inner: Arc::new(HubInner {
                opener,
                admin_token,
                default_world,
                limits,
                sessions: Arc::new(Semaphore::new(limits.max_sessions.max(1))),
                slots: StdMutex::new(HashMap::new()),
                replays: StdMutex::new(crate::directory_auth::ReplayGuard::after_boot(
                    crate::time::unix_ms(),
                    limits.signed_boot_window,
                )),
                account_worlds: StdMutex::new(AccountWorldQuota::new(
                    limits.max_worlds_per_account,
                )),
            }),
        }
    }

    /// A view of this hub pinned to `name` (#5728): the same
    /// registry, sessions, and admin token, but every resolution is
    /// confined to the one world (see the type's docs). The unpinned
    /// hub is unaffected.
    #[must_use]
    pub fn pinned_to(&self, name: &str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            pin: Some(Arc::from(name)),
        }
    }

    /// The world this view is pinned to, if any.
    #[must_use]
    pub fn pinned_world(&self) -> Option<&str> {
        self.pin.as_deref()
    }

    /// Whether this view may resolve `name`: always true unpinned;
    /// on a pinned view only the pinned world itself resolves.
    fn pin_allows(&self, name: &str) -> bool {
        self.pin.as_deref().is_none_or(|pin| pin == name)
    }

    /// Name sessions get when they name no world: the pinned world
    /// on a pinned view, the hub's default otherwise.
    #[must_use]
    pub fn default_world(&self) -> &str {
        self.pin.as_deref().unwrap_or(&self.inner.default_world)
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
            crate::time::unix_ms(),
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
        self.lease_as(name, admin, None).await
    }

    /// Claim `name` for a session acting for `account` (#1067, F12.10
    /// per-account world quotas).
    ///
    /// Behaves exactly like [`Self::lease`], except a creation this
    /// call would perform — an admin creation of a world that does
    /// not exist yet — is charged against the account's world-count
    /// quota and refused with [`ResolveError::TooManyWorlds`] when
    /// the account is already at its limit. Naming (or reopening) a
    /// world the hub already knows is never a new hold and is never
    /// refused by the quota; a `None` account skips the quota, as
    /// every pre-quota path does.
    ///
    /// # Errors
    ///
    /// See [`ResolveError`].
    pub async fn lease_as(
        &self,
        name: Option<&str>,
        admin: Option<&str>,
        account: Option<&str>,
    ) -> Result<WorldLease, ResolveError> {
        let name = name.unwrap_or_else(|| self.default_world());
        // A pinned view resolves only its own world (#5728): any
        // other name is UnknownWorld, exactly as if the world did
        // not exist — and it is refused here, before any slot is
        // claimed, so a refused name is never leased or created.
        if !self.pin_allows(name) {
            return Err(ResolveError::UnknownWorld);
        }
        validate_world_name(name).map_err(|_| ResolveError::InvalidName)?;
        let permit = Arc::clone(&self.inner.sessions)
            .try_acquire_owned()
            .map_err(|_| ResolveError::Busy)?;
        let create = admin.is_some_and(|token| self.admin_ok(token));
        self.lease_with(name, create, permit, account).await
    }

    async fn lease_with(
        &self,
        name: &str,
        create: bool,
        permit: OwnedSemaphorePermit,
        account: Option<&str>,
    ) -> Result<WorldLease, ResolveError> {
        let (slot, evicted, noted) = self.acquire_slot(name, create, account)?;
        // Armed until the lease is handed to the caller: dropping the guard
        // (error return, or cancellation of the awaits below) releases the
        // counted lease exactly once, and rolls back a world hold this
        // acquire recorded, so a creation that never opens charges no quota.
        let mut guard = SlotLeaseGuard {
            hub: self.clone(),
            name: name.to_owned(),
            slot: Arc::clone(&slot),
            account: account.map(str::to_owned),
            noted,
            armed: true,
        };
        for (victim, service) in evicted {
            let _ = service.world().shutdown().await;
            if let Some(opener) = self.inner.opener.as_ref() {
                opener.closed(&victim);
            }
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
            Ok(service) => {
                // F12.14 (code#1068): every world this hub opens -
                // created, reopened after eviction, or its pinned
                // single world - carries the hub's portal transport
                // sink, installed on lease when it has none yet, so
                // portal sends work in production and not only in
                // tests that install a sink by hand (CodeRabbit
                // finding on PR #10840). Install-if-absent: a sink
                // a world already carries is never displaced.
                service
                    .world()
                    .install_portal_delivery(self.portal_delivery());
                guard.armed = false;
                Ok(WorldLease {
                    service: service.clone(),
                    name: name.to_owned(),
                    slot,
                    _permit: permit,
                })
            }
            // The guard's drop releases the counted lease and removes the
            // still-uninitialized slot, as the manual cleanup here used to.
            Err(error) => Err(error),
        }
    }

    /// Find or create the slot for `name` and count a lease on it. Returns
    /// the worlds evicted to make room (name and service), to be shut down
    /// by the caller — plus whether this call recorded a new world hold
    /// for `account` (for the [`SlotLeaseGuard`] rollback).
    ///
    /// A slot that already exists is never a new hold, so the
    /// per-account quota cannot refuse a reopen; a `None` account
    /// skips the quota entirely. Refusing with
    /// [`ResolveError::TooManyWorlds`] reuses the global ceiling's
    /// refusal, so every bridge path handles it as it already does.
    fn acquire_slot(
        &self,
        name: &str,
        create: bool,
        account: Option<&str>,
    ) -> Result<(Arc<Slot>, Vec<(String, WorldService)>, bool), ResolveError> {
        let mut slots = self
            .inner
            .slots
            .lock()
            .map_err(|_| ResolveError::Unavailable)?;
        if let Some(slot) = slots.get(name) {
            // Counted under the map lock so eviction cannot slip in between.
            slot.leases.fetch_add(1, Ordering::AcqRel);
            slot.touch();
            return Ok((Arc::clone(slot), Vec::new(), false));
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
                        .filter(|(key, slot)| {
                            slot.leases.load(Ordering::Acquire) == 0
                                && slot.service.get().is_some()
                                && !opener.pinned(key)
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
                evicted.push((victim, service.clone()));
            }
        }
        let slot = Arc::new(Slot::new());
        slot.leases.store(1, Ordering::Release);
        // The per-account quota is checked and recorded under the same
        // map lock as the slot insert, so two concurrent creations for
        // one account cannot both slip past the limit. The hold is
        // rolled back by the lease guard when the open never completes.
        let mut noted = false;
        if let Some(account) = account
            && let Ok(mut worlds) = self.inner.account_worlds.lock()
        {
            match worlds.try_acquire(account, name) {
                Ok(new_hold) => noted = new_hold,
                Err(_) => return Err(ResolveError::TooManyWorlds),
            }
        }
        slots.insert(name.to_owned(), Arc::clone(&slot));
        Ok((slot, evicted, noted))
    }

    /// The cross-world schedule delivery sink backed by this hub
    /// (#1355 part 2): install it on a world's handle
    /// (`WorldHandle::set_schedule_delivery`) and that world's outbound
    /// schedule fires are delivered through the hub to their target
    /// worlds. Delivery leases the target (never creates it) and answers
    /// with the target's durable acceptance.
    #[must_use]
    pub fn schedule_delivery(&self) -> Arc<dyn ScheduleDelivery> {
        Arc::new(HubScheduleDelivery { hub: self.clone() })
    }

    /// The F12.14 portal transport sink backed by this hub
    /// (code#1068 wiring slice): install it on a world's handle
    /// (`WorldHandle::set_portal_delivery`) and that world's outbound
    /// portal sends are checked against the peer's grant book and
    /// delivered through the hub to the target world. The sink
    /// leases worlds (never creates them); policy stays in the
    /// actors.
    #[must_use]
    pub fn portal_delivery(&self) -> Arc<dyn PortalDelivery> {
        Arc::new(HubPortalDelivery { hub: self.clone() })
    }

    /// Open the default world (creating it), for startup.
    ///
    /// # Errors
    ///
    /// Fails if the world cannot be opened.
    pub async fn open_default(&self) -> Result<WorldService> {
        tracing::info!("serve startup: open_default acquiring session permit (#190 breadcrumb)");
        let permit = Arc::clone(&self.inner.sessions)
            .acquire_owned()
            .await
            .map_err(|e| anyhow::anyhow!("session limiter closed: {e}"))?;
        tracing::info!(
            "serve startup: open_default session permit acquired, leasing world (#190 breadcrumb)"
        );
        let name = self.inner.default_world.clone();
        let lease = self
            .lease_with(&name, true, permit, None)
            .await
            .map_err(|e| anyhow::anyhow!("open default world: {}", e.message()))?;
        tracing::info!("serve startup: open_default lease acquired (#190 breadcrumb)");
        Ok(lease.service().clone())
    }

    /// Create `name` if it does not exist. Returns whether it was created.
    ///
    /// # Errors
    ///
    /// See [`ResolveError`]; hubs without an opener cannot create worlds.
    pub async fn create(&self, name: &str) -> Result<bool, ResolveError> {
        self.create_as(name, None).await
    }

    /// Create `name` for `account` if it does not exist (#1067, F12.10
    /// per-account world quotas). Returns whether it was created.
    ///
    /// Behaves exactly like [`Self::create`], except a creation is
    /// charged against the account's world-count quota and refused
    /// with [`ResolveError::TooManyWorlds`] when the account is
    /// already at its limit. Creating nothing new — the world
    /// already exists — is never refused by the quota; a `None`
    /// account skips the quota, as every pre-quota path does.
    ///
    /// # Errors
    ///
    /// See [`ResolveError`]; hubs without an opener cannot create worlds.
    pub async fn create_as(&self, name: &str, account: Option<&str>) -> Result<bool, ResolveError> {
        // A pinned view may create only its own world (#5728).
        if !self.pin_allows(name) {
            return Err(ResolveError::UnknownWorld);
        }
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
        let _lease = self.lease_with(name, true, permit, account).await?;
        Ok(!existed)
    }

    /// Worlds in storage plus any open ones, sorted by name. A
    /// pinned view lists only its own world (#5728): other worlds'
    /// names are not disclosed through it.
    #[must_use]
    pub fn list(&self) -> Vec<WorldInfo> {
        let mut names: std::collections::BTreeMap<String, (bool, usize)> = self
            .inner
            .opener
            .as_ref()
            .map(|opener| opener.list())
            .unwrap_or_default()
            .into_iter()
            .map(|name| (name, (false, 0)))
            .collect();
        if let Ok(slots) = self.inner.slots.lock() {
            for (name, slot) in slots.iter() {
                if slot.service.get().is_some() {
                    names.insert(name.clone(), (true, slot.leases.load(Ordering::Acquire)));
                }
            }
        }
        names
            .into_iter()
            .filter(|(name, _)| self.pin_allows(name))
            .map(|(name, (open, presence))| WorldInfo {
                name,
                open,
                presence,
            })
            .collect()
    }

    /// Shut down durable worlds that no session has used for `idle`. They
    /// reopen from their journal when next named. Returns how many closed.
    pub async fn evict_idle(&self, idle: Duration) -> usize {
        let Some(opener) = self.inner.opener.as_ref().filter(|o| o.durable()) else {
            return 0;
        };
        let victims: Vec<(String, WorldService)> = {
            let Ok(mut slots) = self.inner.slots.lock() else {
                return 0;
            };
            let names: Vec<String> = slots
                .iter()
                .filter(|(name, slot)| {
                    slot.service.get().is_some()
                        && slot.leases.load(Ordering::Acquire) == 0
                        && slot.idle_for() >= idle
                        && !opener.pinned(name)
                })
                .map(|(name, _)| name.clone())
                .collect();
            names
                .into_iter()
                .filter_map(|name| {
                    let slot = slots.remove(&name)?;
                    let service = slot.service.get().cloned()?;
                    Some((name, service))
                })
                .collect()
        };
        let closed = victims.len();
        for (name, service) in victims {
            let _ = service.world().shutdown().await;
            opener.closed(&name);
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
                let closed = Self { inner, pin: None }.evict_idle(idle).await;
                if closed > 0 {
                    tracing::info!("world hub: closed {closed} idle world(s)");
                }
            }
        })
    }

    /// Attempt the due outbox envelopes of every open world every `period`
    /// until the hub is dropped.
    pub fn spawn_outbox_flush(&self, period: Duration, endpoint: iroh::Endpoint) -> JoinHandle<()> {
        let weak: Weak<HubInner> = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                Self { inner, pin: None }.flush_outboxes(&endpoint).await;
            }
        })
    }

    async fn flush_outboxes(&self, endpoint: &iroh::Endpoint) {
        let services: Vec<WorldService> = self
            .inner
            .slots
            .lock()
            .map(|slots| {
                slots
                    .values()
                    .filter_map(|slot| slot.service.get().cloned())
                    .collect()
            })
            .unwrap_or_default();
        let now = crate::time::unix_ms();
        for service in services {
            if let Err(error) = service.flush_outbox(endpoint, now).await {
                tracing::warn!("world hub: outbox flush failed: {error:#}");
            }
        }
    }

    /// The service of `name` when it is open in this hub, without
    /// opening it.
    #[must_use]
    pub fn open_service(&self, name: &str) -> Option<WorldService> {
        if !self.pin_allows(name) {
            return None;
        }
        self.inner
            .slots
            .lock()
            .ok()?
            .get(name)
            .and_then(|slot| slot.service.get().cloned())
    }

    /// Shut `name` down now, for an operator's stop (#10697): its slot
    /// leaves the hub at once, so the next session reopens it from
    /// storage, and sessions still holding it see the world drained.
    /// Unlike an idle close the opener is not told: the caller records
    /// the stop. Returns whether `name` was open; a world still opening
    /// is left alone.
    ///
    /// # Errors
    ///
    /// Fails when the world does not shut down cleanly.
    pub async fn close(&self, name: &str) -> Result<bool> {
        let service = {
            let Ok(mut slots) = self.inner.slots.lock() else {
                return Ok(false);
            };
            if slots
                .get(name)
                .is_none_or(|slot| slot.service.get().is_none())
            {
                return Ok(false);
            }
            slots
                .remove(name)
                .and_then(|slot| slot.service.get().cloned())
        };
        let Some(service) = service else {
            return Ok(false);
        };
        service.world().shutdown().await?;
        Ok(true)
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

#[cfg(any(test, feature = "test-utils"))]
#[allow(clippy::unwrap_used)]
pub mod testing {
    use super::*;
    use crate::world::{WorldCore, WorldHandle, WorldLimits};

    /// In-memory worlds; `known` simulates worlds that exist in storage.
    pub struct MemoryOpener {
        pub(crate) known: StdMutex<Vec<String>>,
        pub(crate) opens: AtomicUsize,
        pub(crate) durable: bool,
        /// Names the hub reported closed, in order.
        pub(crate) closed: StdMutex<Vec<String>>,
        /// Names an operator pinned always-live.
        pub(crate) pinned: StdMutex<Vec<String>>,
    }

    impl MemoryOpener {
        pub fn new(durable: bool) -> Arc<Self> {
            Arc::new(Self {
                known: StdMutex::new(Vec::new()),
                opens: AtomicUsize::new(0),
                durable,
                closed: StdMutex::new(Vec::new()),
                pinned: StdMutex::new(Vec::new()),
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

        fn closed(&self, name: &str) {
            self.closed.lock().unwrap().push(name.to_owned());
        }

        fn pinned(&self, name: &str) -> bool {
            self.pinned.lock().unwrap().iter().any(|n| n == name)
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

    /// Oracle (PR #10840 CodeRabbit regression): worlds leased
    /// through a hub carry the hub's portal transport sink with no
    /// manual install - a mutually granted pair leased from the
    /// hub exchanges a portal message in production shape, where
    /// before this fix every production send refused for want of
    /// a sink.
    #[tokio::test]
    async fn hub_leased_worlds_exchange_portal_without_manual_sink() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        hub.create("world-a").await.unwrap();
        hub.create("world-b").await.unwrap();
        let a = hub.lease(Some("world-a"), None).await.unwrap();
        let b = hub.lease(Some("world-b"), None).await.unwrap();
        a.service()
            .world()
            .admin_set_portal_grant("world-b".to_owned(), true)
            .await
            .unwrap();
        b.service()
            .world()
            .admin_set_portal_grant("world-a".to_owned(), true)
            .await
            .unwrap();
        // No set_portal_delivery call anywhere: the hub lease
        // installed its sink on both worlds.
        a.service()
            .world()
            .send_portal_message("world-b".to_owned(), "hello via hub".to_owned())
            .await
            .unwrap();
    }

    /// Oracle (#5728): a pinned view resolves only its own world.
    /// Leasing or creating another world is `UnknownWorld` — with no
    /// opener call, so nothing is leased or created as a side
    /// effect — an unnamed lease lands on the pinned world (not the
    /// hub default), the listing shows only the pinned world, and
    /// the unpinned hub is unchanged.
    #[tokio::test]
    async fn pinned_view_resolves_only_its_world() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(&opener, HubLimits::default());
        hub.create("arena").await.unwrap();
        hub.create("vault").await.unwrap();
        let opens_before = opener.opens.load(Ordering::SeqCst);

        let view = hub.pinned_to("arena");
        assert_eq!(view.pinned_world(), Some("arena"));
        assert_eq!(view.default_world(), "arena");

        // Other worlds do not resolve, and refusing them opens nothing.
        assert_eq!(
            view.lease(Some("vault"), None).await.unwrap_err(),
            ResolveError::UnknownWorld
        );
        assert_eq!(
            view.lease(Some("vault"), Some("admin")).await.unwrap_err(),
            ResolveError::UnknownWorld
        );
        assert_eq!(
            view.create("vault").await.unwrap_err(),
            ResolveError::UnknownWorld
        );
        assert_eq!(opener.opens.load(Ordering::SeqCst), opens_before);

        // The pinned world resolves, named or unnamed.
        let lease = view.lease(Some("arena"), None).await.unwrap();
        drop(lease);
        let lease = view.lease(None, None).await.unwrap();
        drop(lease);

        // The listing discloses only the pinned world.
        let listed = view.list();
        let names: Vec<&str> = listed.iter().map(|info| info.name.as_str()).collect();
        assert_eq!(names, ["arena"]);

        // The unpinned hub still resolves and lists everything.
        assert_eq!(hub.default_world(), "lobby");
        let lease = hub.lease(Some("vault"), None).await.unwrap();
        drop(lease);
        let listed = hub.list();
        let names: Vec<&str> = listed.iter().map(|info| info.name.as_str()).collect();
        assert!(
            names.contains(&"arena") && names.contains(&"vault"),
            "{names:?}"
        );
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
        assert_eq!(
            b.service().world().snapshot(token_b).await.unwrap().events,
            [] as [crate::world::WorldEvent; 0]
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
        // The opener heard that "a" closed (#10693: a host records it asleep).
        assert_eq!(*opener.closed.lock().unwrap(), vec!["a".to_owned()]);
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
        assert_eq!(*opener.closed.lock().unwrap(), vec!["idle".to_owned()]);
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
    async fn an_idle_world_sleeps_after_its_window_and_not_before() {
        // #10694: the last client left; the world stays open until the
        // whole window has passed, then closes.
        let opener = MemoryOpener::new(true);
        let hub = new_hub(&opener, HubLimits::default());
        drop(hub.lease(Some("quiet"), Some("admin")).await.unwrap());
        assert_eq!(hub.evict_idle(Duration::from_secs(3600)).await, 0);
        assert_eq!(hub.open_count(), 1);
        assert!(opener.closed.lock().unwrap().is_empty());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(hub.evict_idle(Duration::from_millis(10)).await, 1);
        assert_eq!(hub.open_count(), 0);
        assert_eq!(*opener.closed.lock().unwrap(), vec!["quiet".to_owned()]);
    }

    #[tokio::test]
    async fn pinned_worlds_are_never_closed_for_idleness_or_to_make_room() {
        let opener = MemoryOpener::new(true);
        opener.pinned.lock().unwrap().push("kept".to_owned());
        let hub = new_hub(
            &opener,
            HubLimits {
                max_open_worlds: 2,
                ..HubLimits::default()
            },
        );
        drop(hub.lease(Some("kept"), Some("admin")).await.unwrap());
        drop(hub.lease(Some("loose"), Some("admin")).await.unwrap());
        tokio::time::sleep(Duration::from_millis(30)).await;

        // "kept" is the longest idle, but "loose" makes room for "new".
        let new = hub.lease(Some("new"), Some("admin")).await.unwrap();
        assert_eq!(*opener.closed.lock().unwrap(), vec!["loose".to_owned()]);
        // With "new" leased and "kept" pinned there is no room left.
        assert_eq!(
            hub.lease(Some("more"), Some("admin")).await.err(),
            Some(ResolveError::TooManyWorlds)
        );
        drop(new);

        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(hub.evict_idle(Duration::from_millis(10)).await, 1);
        assert_eq!(
            *opener.closed.lock().unwrap(),
            vec!["loose".to_owned(), "new".to_owned()]
        );
        let open: Vec<_> = hub
            .list()
            .into_iter()
            .filter(|w| w.open)
            .map(|w| w.name)
            .collect();
        assert_eq!(open, vec!["kept".to_owned()]);
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

    /// Oracle (#1067 F12.10 world-count slice): a hub with a
    /// per-account world limit refuses a new world for the account
    /// already at its limit — with the existing `TooManyWorlds`
    /// refusal, so every bridge path handles it like the global
    /// ceiling — while another account may still create, and
    /// reopening a world the account already holds never refuses.
    #[tokio::test]
    async fn a_per_account_world_limit_refuses_only_the_full_account() {
        let opener = MemoryOpener::new(false);
        let hub = new_hub(
            &opener,
            HubLimits {
                max_worlds_per_account: 1,
                ..HubLimits::default()
            },
        );
        assert!(hub.create_as("w1", Some("acct-a")).await.unwrap());
        assert_eq!(
            hub.create_as("w2", Some("acct-a")).await.err(),
            Some(ResolveError::TooManyWorlds)
        );
        // Another account is unaffected.
        assert!(hub.create_as("w2", Some("acct-b")).await.unwrap());
        // Reopening an already-held world is idempotent, not a new hold.
        assert!(!hub.create_as("w1", Some("acct-a")).await.unwrap());
        // An admin lease that would create a new world for the full
        // account is refused the same way; a lease of an existing
        // world is not.
        assert_eq!(
            hub.lease_as(Some("w3"), Some("admin"), Some("acct-a"))
                .await
                .err(),
            Some(ResolveError::TooManyWorlds)
        );
        hub.lease_as(Some("w1"), None, Some("acct-a"))
            .await
            .unwrap();
    }

    #[test]
    fn directory_json_shape_names_loaded_not_open() {
        let worlds = vec![
            WorldInfo {
                name: "lobby".to_owned(),
                open: true,
                presence: 2,
            },
            WorldInfo {
                name: "attic".to_owned(),
                open: false,
                presence: 0,
            },
        ];
        let value = directory_json(&worlds, "lobby");
        assert_eq!(value["default"], serde_json::json!("lobby"));
        let list = value["worlds"].as_array().expect("worlds is a list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["name"], serde_json::json!("lobby"));
        assert_eq!(list[0]["loaded"], serde_json::json!(true));
        assert_eq!(list[0]["default"], serde_json::json!(true));
        assert_eq!(list[0]["presence"], serde_json::json!(2));
        assert_eq!(list[1]["presence"], serde_json::json!(0));
        assert_eq!(list[1]["name"], serde_json::json!("attic"));
        assert_eq!(list[1]["loaded"], serde_json::json!(false));
        assert_eq!(list[1]["default"], serde_json::json!(false));
        // The honest shape: no entry may claim an "open" field.
        for entry in list {
            assert!(entry.get("open").is_none(), "entry must not carry `open`");
            assert!(entry["name"].is_string());
            assert!(entry["loaded"].is_boolean());
            assert!(entry["default"].is_boolean());
            assert!(entry["presence"].is_u64());
        }
        // An empty directory is a default with no worlds.
        let value = directory_json(&[], "lobby");
        assert_eq!(value["worlds"], serde_json::json!([]));
    }

    /// An opener whose open never completes, so the lease future can only
    /// end by cancellation.
    struct PendingOpener;

    impl WorldOpener for PendingOpener {
        fn open<'a>(&'a self, _name: &'a str, _create: bool) -> OpenFuture<'a> {
            Box::pin(std::future::pending())
        }

        fn exists(&self, _name: &str) -> bool {
            true
        }

        fn list(&self) -> Vec<String> {
            Vec::new()
        }

        fn durable(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn cancelled_lease_releases_its_slot() {
        let opener: Arc<dyn WorldOpener> = Arc::new(PendingOpener);
        let hub = WorldHub::new(
            opener,
            Some("admin".to_owned()),
            "lobby",
            HubLimits::default(),
        );
        let lease = hub.lease(Some("arena"), Some("admin"));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), lease)
                .await
                .is_err(),
            "the pending open should be cut off by the test deadline"
        );
        // The cancelled lease must not leak: its slot is gone, so the world
        // is neither stuck open nor un-evictable.
        assert_eq!(hub.open_count(), 0);
    }
}

/// [`ScheduleDelivery`] through a [`WorldHub`]: lease the target world
/// and ask its actor to accept the fire. Any failure — unknown world,
/// busy hub, closed actor, refusal — is `false`, which leaves the
/// source occurrence pending for a later retry.
struct HubScheduleDelivery {
    hub: WorldHub,
}

/// [`PortalDelivery`] through a [`WorldHub`]: read the peer's grant
/// book and deliver checked messages by leasing the target world.
/// Any failure to lease is `None` / `Unavailable`: for a book it
/// cannot read the sending handle answers `Unavailable` (a
/// transport failure - no book was read, so no policy refusal is
/// recorded), and a delivery that never arrives is likewise a
/// transport error.
struct HubPortalDelivery {
    hub: WorldHub,
}

impl PortalDelivery for HubPortalDelivery {
    fn peer_portal_grants<'a>(
        &'a self,
        world: &'a str,
    ) -> PortalDeliveryFuture<'a, Option<PortalGrants>> {
        Box::pin(async move {
            let lease = self.hub.lease(Some(world), None).await.ok()?;
            lease.service().world().portal_grants().await.ok()
        })
    }

    fn deliver_portal<'a>(
        &'a self,
        source: &'a str,
        target: &'a str,
        message: &'a str,
    ) -> PortalDeliveryFuture<'a, Result<(), PortalSendError>> {
        Box::pin(async move {
            let lease = self
                .hub
                .lease(Some(target), None)
                .await
                .map_err(|error| PortalSendError::Unavailable(format!("{error:?}")))?;
            lease
                .service()
                .world()
                .deliver_portal_message(source.to_owned(), message.to_owned())
                .await
        })
    }
}

impl ScheduleDelivery for HubScheduleDelivery {
    fn deliver_schedule<'a>(
        &'a self,
        source: &'a str,
        target: &'a str,
        id: &'a str,
        at_ms: u64,
        late: bool,
    ) -> ScheduleDeliveryFuture<'a> {
        Box::pin(async move {
            let Ok(lease) = self.hub.lease(Some(target), None).await else {
                return false;
            };
            lease
                .service()
                .world()
                .deliver_schedule(source.to_owned(), id.to_owned(), at_ms, late)
                .await
                .unwrap_or(false)
        })
    }
}
