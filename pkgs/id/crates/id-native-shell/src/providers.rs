//! F13.7 step 4 — client grant and local audit adapter for the native
//! shell (design `doc/2026-11-01_design_id789.md` §4 and §7 step 4).
//!
//! The host broker (step 3, `id-worlds` `provider_broker`) is the
//! policy point for what a guest may *request*; this module is the
//! local authority for what this shell will *do*. The two checks are
//! independent by design: a host "allow" cannot bypass the local
//! check here, and a local grant alone never causes the host to route
//! a request. Concretely, this module provides:
//!
//! - **Per-world local opt-in.** A provider is usable only where the
//!   local user has granted it for that world. The default is no
//!   grants anywhere: nothing is offered, and every request is
//!   refused. Grant-on-use is never performed — a request for an
//!   ungranted provider is a refusal, not a prompt or an implicit
//!   grant.
//! - **Local bounds.** Each grant carries bounds (`max_bytes`,
//!   `per_minute`, `in_flight`) that compose by minimum with the
//!   manifest and world bounds on the host side; locally they are
//!   enforced on every request, and an over-bound result is refused
//!   whole (`result_too_large`), never silently truncated.
//! - **Immediate revoke.** Revoking a grant removes it from the
//!   offer set at once and makes the next request for that
//!   world/provider refuse with `provider_revoked`; no new execution
//!   starts under a revoked grant.
//! - **A local audit entry per decision.** Every request handled
//!   here — executed or refused — appends one user-readable entry to
//!   the local audit log (world, provider, nonce, outcome, refusal
//!   code). This is the client-local log the design separates from
//!   the host F12.6 record: it proves the local check happened, and
//!   it never carries argument values or result bytes.
//!
//! The adapter seam is [`ClientProviderExecutor`]: step 5 registers
//! the first real executor ([`FileReadExecutor`], `client.file.read`)
//! through [`ProviderAdapter::grant_file_read`], and #10930 registers
//! the second ([`NotifyExecutor`], `client.notify`) through
//! [`ProviderAdapter::grant_notify`]; a provider with no
//! registered executor refuses `provider_unavailable` and an ungranted
//! one refuses `provider_not_granted` — no provider is enabled by
//! default, and a malicious or mistaken host request still meets the
//! local check.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// Refusal: the local user has not opted this provider in for this
/// world (and grant-on-use never applies).
pub const REFUSAL_NOT_GRANTED: &str = "provider_not_granted";
/// Refusal: the provider was opted in for this world and the local
/// user has since revoked the grant.
pub const REFUSAL_REVOKED: &str = "provider_revoked";
/// Refusal: the provider is granted but this shell has no executor
/// registered for it (no provider is enabled by default).
pub const REFUSAL_UNAVAILABLE: &str = "provider_unavailable";
/// Refusal: the grant's per-minute local bound is exhausted.
pub const REFUSAL_RATE_LIMITED: &str = "provider_rate_limited";
/// Refusal: the grant's in-flight local bound is exhausted.
pub const REFUSAL_BUSY: &str = "provider_busy";
/// Refusal: the request arrived with no time left to answer it.
pub const REFUSAL_EXPIRED: &str = "provider_timeout";
/// Refusal: the executor produced more than the local byte bound;
/// the result is refused whole, never truncated.
pub const REFUSAL_TOO_LARGE: &str = "result_too_large";
/// Refusal: the request frame lacks a usable nonce or provider name.
pub const REFUSAL_MALFORMED: &str = "provider_malformed_request";
/// Refusal: a `client.file.read` execution was attempted without the
/// local user's disclosure acknowledgment — the opt-in that states
/// returned bytes become journaled world data (design invariant 10
/// and the §4 durability disclosure; Drew's §9 Q2 ruling A,
/// 2026-11-16: journaled results with opt-in disclosure).
pub const REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED: &str = "content_disclosure_not_acknowledged";
/// Refusal: the requested path is absolute, traverses above the
/// user-selected root, escapes it through a symlink, or is not a
/// usable relative path.
pub const REFUSAL_INVALID_PATH: &str = "file_read_invalid_path";
/// Refusal: no file exists at the resolved path under the root.
pub const REFUSAL_NOT_FOUND: &str = "file_read_not_found";
/// Refusal: the resolved path exists but is not a regular file.
pub const REFUSAL_NOT_A_FILE: &str = "file_read_not_a_file";
/// Refusal: the local filesystem refused the read itself.
pub const REFUSAL_IO: &str = "file_read_io";

/// The first client-side provider's registry name (F13.7 step 5;
/// the manifest is `id_caps::world_caps::CLIENT_FILE_READ_MANIFEST`).
pub const CLIENT_FILE_READ_PROVIDER: &str = "client.file.read";
/// The mobile shell's notification provider (#10930, shells v1 slice 4;
/// the manifest is `id_caps::world_caps::CLIENT_NOTIFY_MANIFEST`).
pub const CLIENT_NOTIFY_PROVIDER: &str = "client.notify";
/// Refusal: a `client.notify` request carried a non-unsigned `head`.
pub const REFUSAL_INVALID_HEAD: &str = "notify_invalid_head";
/// Refusal: a `client.notify` request carried a content-bearing field.
/// A notification is a hint to re-sync, never a data channel, so any
/// content field is refused whole rather than delivered.
pub const REFUSAL_CONTENT_FORBIDDEN: &str = "notify_content_forbidden";

/// Rate-limit window for `per_minute` bounds, in milliseconds.
const RATE_WINDOW_MS: u64 = 60_000;
/// Maximum local audit entries retained; oldest dropped first. The
/// local log is a session record, not a durable store.
pub const MAX_AUDIT_ENTRIES: usize = 512;

/// Local bounds on one granted provider, set by the local user at
/// grant time. They compose by minimum with the manifest and world
/// bounds the host enforces; the local values here are the ones this
/// shell will never exceed regardless of what a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalBounds {
    /// Maximum result bytes one execution may return.
    pub max_bytes: u32,
    /// Maximum executions started per minute.
    pub per_minute: u32,
    /// Maximum executions in flight at once.
    pub in_flight: u32,
}

impl LocalBounds {
    /// The design's starting bounds for the first slice
    /// (`client.file.read`: 32 KiB, ten reads/minute, two in flight).
    pub const DESIGN_DEFAULT: Self = Self {
        max_bytes: 32_768,
        per_minute: 10,
        in_flight: 2,
    };

    /// The starting bounds for `client.notify` (#10930): the reply is
    /// a fixed-shape acknowledgment carrying no content, so the byte
    /// bound is the manifest's small ack cap; hints may burst (a catch-up
    /// run can surface several), so the rate allows thirty per minute
    /// with four in flight.
    pub const NOTIFY_DEFAULT: Self = Self {
        max_bytes: 1024,
        per_minute: 30,
        in_flight: 4,
    };

    /// The effective bound of two bound sets: the minimum of each
    /// field, per the design's compose-by-minimum rule.
    #[must_use]
    pub fn compose_minimum(self, other: Self) -> Self {
        Self {
            max_bytes: self.max_bytes.min(other.max_bytes),
            per_minute: self.per_minute.min(other.per_minute),
            in_flight: self.in_flight.min(other.in_flight),
        }
    }
}

/// One local provider execution. Implementors hold the actual local
/// resource (step 5: a user-selected file root); the policy in this
/// module decides whether `execute` is reached at all.
///
/// `Err` carries a stable refusal code that is returned to the host
/// verbatim and recorded in the local audit entry — never a message
/// containing argument values, paths, or content.
pub trait ClientProviderExecutor: Send {
    /// Execute one request whose local checks have passed.
    ///
    /// # Errors
    ///
    /// Returns a stable refusal code when the resource refuses.
    fn execute(&mut self, args: &Value, max_bytes: u32) -> Result<Vec<u8>, String>;
}

/// The first real client provider (F13.7 step 5): bounded read-only
/// file access under one user-selected root, per the design's
/// first-provider safety rules — only paths beneath the root,
/// absolute paths and parent traversal refused, symlink escape
/// refused (the canonical target must stay beneath the canonical
/// root), at most the effective byte bound returned, and an
/// over-bound file refused whole ([`REFUSAL_TOO_LARGE`]), never
/// silently truncated.
///
/// Disclosure is part of the executor, not just the grant path:
/// constructed with `disclosure_acknowledged: false`, every
/// execution refuses [`REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED`], so no
/// registration route can serve reads whose journal disclosure the
/// local user has not opted into.
pub struct FileReadExecutor {
    root: PathBuf,
    disclosure_acknowledged: bool,
}

impl FileReadExecutor {
    /// Create an executor for `root`. The root is canonicalized
    /// once here and must be an existing directory; every request
    /// path is resolved against this canonical root.
    ///
    /// # Errors
    ///
    /// Returns [`REFUSAL_IO`] when the root cannot be canonicalized
    /// or is not a directory.
    pub fn new(root: impl Into<PathBuf>, disclosure_acknowledged: bool) -> Result<Self, String> {
        let root = root
            .into()
            .canonicalize()
            .map_err(|_| REFUSAL_IO.to_owned())?;
        if !root.is_dir() {
            return Err(REFUSAL_IO.to_owned());
        }
        Ok(Self {
            root,
            disclosure_acknowledged,
        })
    }

    /// The canonical user-selected root this executor serves.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Check a request path's shape: it must be non-empty, relative,
    /// and free of parent or root components.
    fn relative_path(requested: &str) -> Result<&Path, String> {
        if requested.is_empty() {
            return Err(REFUSAL_INVALID_PATH.to_owned());
        }
        let relative = Path::new(requested);
        if relative.is_absolute() {
            return Err(REFUSAL_INVALID_PATH.to_owned());
        }
        for component in relative.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(REFUSAL_INVALID_PATH.to_owned());
                }
            }
        }
        Ok(relative)
    }

    /// Open `relative` beneath the root and return the open handle.
    ///
    /// On Linux the open is `openat2` with `RESOLVE_BENEATH`, relative
    /// to a handle on the root directory, so the kernel refuses any
    /// symlink or component that leaves the root at the moment of the
    /// open. There is no check-then-open window to race. The returned
    /// handle is the one that is validated and read.
    ///
    /// Fallback, used where `openat2` is unavailable (`ENOSYS`), where
    /// a symlink targets an absolute path that `RESOLVE_BENEATH`
    /// refuses (`EXDEV`, which in-root aliases may use and are still
    /// served), or on non-Linux targets: canonicalize, require the
    /// canonical path to stay beneath the root, then reopen the
    /// canonical suffix with symlinks refused, or open the canonical
    /// path directly where `openat2` does not exist.
    fn open_within_root(&self, relative: &Path) -> Result<File, String> {
        #[cfg(target_os = "linux")]
        {
            let root_dir = File::open(&self.root).map_err(|_| REFUSAL_IO.to_owned())?;
            match open_beneath(
                &root_dir,
                relative,
                libc::RESOLVE_BENEATH | libc::RESOLVE_NO_MAGICLINKS,
            ) {
                Ok(file) => return Ok(file),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(REFUSAL_NOT_FOUND.to_owned());
                }
                Err(error)
                    if matches!(
                        error.raw_os_error(),
                        Some(libc::EXDEV | libc::ELOOP | libc::ENOSYS | libc::EPERM | libc::EAGAIN)
                    ) => {}
                Err(_) => return Err(REFUSAL_IO.to_owned()),
            }
        }
        let canonical = self
            .root
            .join(relative)
            .canonicalize()
            .map_err(|_| REFUSAL_NOT_FOUND.to_owned())?;
        let suffix = canonical
            .strip_prefix(&self.root)
            .map_err(|_| REFUSAL_INVALID_PATH.to_owned())?;
        #[cfg(target_os = "linux")]
        let opened = File::open(&self.root)
            .and_then(|root_dir| {
                open_beneath(
                    &root_dir,
                    suffix,
                    libc::RESOLVE_BENEATH | libc::RESOLVE_NO_MAGICLINKS | libc::RESOLVE_NO_SYMLINKS,
                )
            })
            .or_else(|error| {
                // A kernel without openat2 (before 5.6), or a seccomp
                // filter that refuses it, gets the canonical open the
                // non-Linux targets use, rather than refusing every read.
                if matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EPERM)) {
                    File::open(&canonical)
                } else {
                    Err(error)
                }
            });
        #[cfg(not(target_os = "linux"))]
        let opened = File::open(&canonical);
        opened.map_err(|_| REFUSAL_IO.to_owned())
    }
}

/// Open `relative` beneath the directory handle `root_dir` with
/// `openat2(2)`, using the given `RESOLVE_*` flags.
#[cfg(target_os = "linux")]
fn open_beneath(root_dir: &File, relative: &Path, resolve: u64) -> std::io::Result<File> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};
    use std::os::unix::ffi::OsStrExt as _;

    let path = std::ffi::CString::new(relative.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `open_how` is plain integers; all-zero is a valid value
    // and every field the kernel reads is set below.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_RDONLY | libc::O_CLOEXEC) as u64;
    how.resolve = resolve;
    // SAFETY: `path` is a valid NUL-terminated string, `how` outlives
    // the call, and the size passed is exactly `size_of::<open_how>()`.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root_dir.as_raw_fd(),
            path.as_ptr(),
            std::ptr::from_ref(&how),
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a non-negative return from `openat2` is a new, owned fd.
    Ok(unsafe { File::from_raw_fd(fd as libc::c_int) })
}

impl ClientProviderExecutor for FileReadExecutor {
    fn execute(&mut self, args: &Value, max_bytes: u32) -> Result<Vec<u8>, String> {
        if !self.disclosure_acknowledged {
            return Err(REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED.to_owned());
        }
        let Some(requested) = args["path"].as_str() else {
            return Err(REFUSAL_INVALID_PATH.to_owned());
        };
        let relative = Self::relative_path(requested)?;
        let file = self.open_within_root(relative)?;
        let metadata = file.metadata().map_err(|_| REFUSAL_IO.to_owned())?;
        if !metadata.is_file() {
            return Err(REFUSAL_NOT_A_FILE.to_owned());
        }
        // Read at most one byte past the bound: an over-bound file
        // is refused whole, never silently truncated. The bytes come
        // from the same handle that was validated above.
        let mut bytes = Vec::new();
        file.take(u64::from(max_bytes) + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| REFUSAL_IO.to_owned())?;
        if bytes.len() > max_bytes as usize {
            return Err(REFUSAL_TOO_LARGE.to_owned());
        }
        Ok(bytes)
    }
}

/// The second client provider (#10930, shells v1 slice 4): the mobile
/// shell's local notification surface behind `client.notify`.
///
/// A notification is a hint to re-sync, never a data channel: the only
/// argument the registry schema allows is the optional journal `head`
/// the hint refers to, and the reply is a fixed-shape acknowledgment
/// carrying no content (empty bytes). The shell renders its own local
/// wording from the hint and, on return, runs catch-up from the client
/// projection plus the host view (see [`notify_needs_resync`]). There
/// is deliberately no disclosure gate here, unlike file read: a hint
/// carries no world bytes into the journal, so there is nothing to
/// disclose.
///
/// Content-bearing fields (`text`, `content`, `body`, `data`,
/// `message`) are refused with [`REFUSAL_CONTENT_FORBIDDEN`]: a host
/// that tries to push world content through the hint channel meets the
/// local check, not a delivery.
#[derive(Clone, Debug, Default)]
pub struct NotifyExecutor {
    surfaced: Vec<Option<u64>>,
}

impl NotifyExecutor {
    /// A new notifier with no surfaced hints yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The hint heads surfaced so far, oldest first (`None` is a hint
    /// that carried no `head`). The record proves the local surface
    /// happened; it never carries world content.
    #[must_use]
    pub fn surfaced_heads(&self) -> &[Option<u64>] {
        &self.surfaced
    }

    fn check_args(args: &Value) -> Result<Option<u64>, String> {
        for forbidden in ["text", "content", "body", "data", "message"] {
            if args.get(forbidden).is_some_and(|value| !value.is_null()) {
                return Err(REFUSAL_CONTENT_FORBIDDEN.to_owned());
            }
        }
        match args.get("head") {
            None => Ok(None),
            Some(value) if value.is_null() => Ok(None),
            Some(value) => value
                .as_u64()
                .ok_or_else(|| REFUSAL_INVALID_HEAD.to_owned())
                .map(Some),
        }
    }
}

impl ClientProviderExecutor for NotifyExecutor {
    fn execute(&mut self, args: &Value, _max_bytes: u32) -> Result<Vec<u8>, String> {
        let head = Self::check_args(args)?;
        self.surfaced.push(head);
        Ok(Vec::new())
    }
}

/// Whether a surfaced `client.notify` hint requires the shell to
/// re-sync on return (the behavioral half of #10930's Done).
///
/// - No local snapshot (`local_head` is `None`): re-sync; there is
///   nothing to render without a fresh snapshot.
/// - Hint without a `head` (`hint_head` is `None`): re-sync; a
///   headless hint means something changed.
/// - Otherwise re-sync exactly when the shell is behind the hint
///   (`local < hint`); a shell already past the hint ignores it.
#[must_use]
pub fn notify_needs_resync(local_head: Option<u64>, hint_head: Option<u64>) -> bool {
    match (local_head, hint_head) {
        (None, _) => true,
        (_, None) => true,
        (Some(local), Some(hint)) => local < hint,
    }
}

/// One user-readable local audit entry: the record that the local
/// adapter executed or refused a provider request. Deliberately
/// carries no argument values, local paths, or result bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAuditEntry {
    /// When the decision was made (milliseconds, caller's clock).
    pub at_ms: u64,
    /// The world the request arrived for.
    pub world: String,
    /// The provider named by the request.
    pub provider: String,
    /// The host-minted nonce the request carried.
    pub nonce: String,
    /// `executed` or `refused`.
    pub outcome: &'static str,
    /// The stable refusal code when `outcome` is `refused`.
    pub reason: Option<String>,
}

/// Mutable per-grant enforcement state.
#[derive(Clone, Copy, Debug)]
struct GrantState {
    bounds: LocalBounds,
    window_start_ms: u64,
    window_count: u32,
    in_flight: u32,
}

/// The shell-side provider adapter: local grants, executors, and the
/// local audit log for one shell process. Owned by a single session
/// task; all state transitions are immediate on the owning task.
#[derive(Default)]
pub struct ProviderAdapter {
    grants: HashMap<(String, String), GrantState>,
    revoked: HashSet<(String, String)>,
    executors: HashMap<String, Box<dyn ClientProviderExecutor>>,
    /// Executors bound to one world's grant (#11478): a resource the
    /// local user picked for that world, such as a file-read root. A
    /// world's own executor wins over the provider-wide one, and no
    /// other world can reach or replace it.
    world_executors: HashMap<(String, String), Box<dyn ClientProviderExecutor>>,
    audit: Vec<LocalAuditEntry>,
}

impl std::fmt::Debug for ProviderAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderAdapter")
            .field("grants", &self.grants.len())
            .field("executors", &self.executors.keys().collect::<Vec<_>>())
            .field(
                "world_executors",
                &self.world_executors.keys().collect::<Vec<_>>(),
            )
            .field("audit_entries", &self.audit.len())
            .finish_non_exhaustive()
    }
}

impl ProviderAdapter {
    /// A new adapter with no grants and no executors: the shipped
    /// default, in which nothing is offered and every request is
    /// refused by the local check.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the executor that serves one provider name for every
    /// world that has no executor of its own (see
    /// [`Self::register_world_executor`]). A provider with an executor
    /// but no grant for a world is still never offered or executed for
    /// that world.
    pub fn register_executor(&mut self, provider: &str, executor: Box<dyn ClientProviderExecutor>) {
        self.executors.insert(provider.to_owned(), executor);
    }

    /// Register the executor that serves `provider` for `world` only
    /// (#11478). It wins over any provider-wide executor for that
    /// world, and registering one for another world never changes it,
    /// so a resource the local user granted to one world (a file-read
    /// root) cannot be swapped out or reached by another world's grant.
    pub fn register_world_executor(
        &mut self,
        world: &str,
        provider: &str,
        executor: Box<dyn ClientProviderExecutor>,
    ) {
        self.world_executors
            .insert((world.to_owned(), provider.to_owned()), executor);
    }

    /// Whether `world` has an executor for `provider`: its own, or the
    /// provider-wide one.
    fn has_executor(&self, world: &str, provider: &str) -> bool {
        self.world_executors
            .contains_key(&(world.to_owned(), provider.to_owned()))
            || self.executors.contains_key(provider)
    }

    /// Opt `provider` in for `world` with the given local bounds.
    /// This is the local user's grant; it is per-world, and it is
    /// the only way a provider becomes offered or executable.
    pub fn grant(&mut self, world: &str, provider: &str, bounds: LocalBounds) {
        let key = (world.to_owned(), provider.to_owned());
        self.revoked.remove(&key);
        self.grants.insert(
            key,
            GrantState {
                bounds,
                window_start_ms: 0,
                window_count: 0,
                in_flight: 0,
            },
        );
    }

    /// The step-5 opt-in for `client.file.read`: register the
    /// [`FileReadExecutor`] rooted at `root` for `world` alone and
    /// grant it for `world`, in one step, so the shipped shell's
    /// opt-in surface cannot grant the provider without its executor
    /// or vice versa. Each world reads only the root granted to it: a
    /// later grant for another world never changes this one (#11478).
    ///
    /// `disclosure_acknowledged` is the local user's explicit
    /// acknowledgment of the design's durability disclosure: bytes
    /// this provider returns enter the world program and its durable
    /// journal, where the host/operator and anyone with journal
    /// access can later see them (Drew's §9 Q2 ruling A: journaled
    /// results with opt-in disclosure). Without it the grant is
    /// refused with [`REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED`] and
    /// nothing is registered or granted.
    ///
    /// # Errors
    ///
    /// Returns a stable refusal code when the disclosure is not
    /// acknowledged or the root cannot be canonicalized as a
    /// directory.
    pub fn grant_file_read(
        &mut self,
        world: &str,
        root: impl Into<PathBuf>,
        bounds: LocalBounds,
        disclosure_acknowledged: bool,
    ) -> Result<(), String> {
        if !disclosure_acknowledged {
            return Err(REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED.to_owned());
        }
        let executor = FileReadExecutor::new(root, true)?;
        self.register_world_executor(world, CLIENT_FILE_READ_PROVIDER, Box::new(executor));
        self.grant(world, CLIENT_FILE_READ_PROVIDER, bounds);
        Ok(())
    }

    /// The #10930 opt-in for `client.notify`: ensure a
    /// [`NotifyExecutor`] is registered for the shell and grant the
    /// provider for `world` with the given local bounds, so the mobile
    /// shell's notification power is per-world opt-in like every other
    /// client power. Unlike file read there is no disclosure gate and
    /// no per-world resource: a hint carries no world bytes, so one
    /// shared executor serves every granted world and the audit entry
    /// still records which world each hint arrived for.
    pub fn grant_notify(&mut self, world: &str, bounds: LocalBounds) {
        if !self.executors.contains_key(CLIENT_NOTIFY_PROVIDER) {
            self.register_executor(CLIENT_NOTIFY_PROVIDER, Box::new(NotifyExecutor::new()));
        }
        self.grant(world, CLIENT_NOTIFY_PROVIDER, bounds);
    }

    /// Revoke a grant, effective immediately: the provider leaves
    /// the world's offer set and the next request for it refuses
    /// with [`REFUSAL_REVOKED`]. Any executor bound to that world's
    /// grant is dropped with it, so a later bare [`Self::grant`] cannot
    /// revive the old resource. Returns whether a grant existed.
    pub fn revoke(&mut self, world: &str, provider: &str) -> bool {
        let key = (world.to_owned(), provider.to_owned());
        self.world_executors.remove(&key);
        let existed = self.grants.remove(&key).is_some();
        if existed {
            self.revoked.insert(key);
        }
        existed
    }

    /// Whether `provider` is currently granted for `world`.
    #[must_use]
    pub fn is_granted(&self, world: &str, provider: &str) -> bool {
        self.grants
            .contains_key(&(world.to_owned(), provider.to_owned()))
    }

    /// The providers this shell may advertise for `world`: granted
    /// for that world *and* backed by a registered executor, sorted
    /// for a stable offer frame.
    #[must_use]
    pub fn offered_providers(&self, world: &str) -> Vec<String> {
        let mut offered: Vec<String> = self
            .grants
            .keys()
            .filter(|(grant_world, provider)| {
                grant_world == world && self.has_executor(world, provider)
            })
            .map(|(_, provider)| provider.clone())
            .collect();
        offered.sort();
        offered.dedup();
        offered
    }

    /// The `provider_offer` client frame for `world`, or `None`
    /// when nothing is offered (the default: no frame is sent).
    #[must_use]
    pub fn offer_frame(&self, world: &str) -> Option<Value> {
        let providers = self.offered_providers(world);
        if providers.is_empty() {
            None
        } else {
            Some(json!({"type": "provider_offer", "providers": providers}))
        }
    }

    /// The local audit entries recorded so far, oldest first.
    #[must_use]
    pub fn audit_entries(&self) -> &[LocalAuditEntry] {
        &self.audit
    }

    fn record_audit(&mut self, entry: LocalAuditEntry) {
        self.audit.push(entry);
        if self.audit.len() > MAX_AUDIT_ENTRIES {
            let excess = self.audit.len() - MAX_AUDIT_ENTRIES;
            self.audit.drain(..excess);
        }
    }

    fn refuse(
        &mut self,
        at_ms: u64,
        world: &str,
        provider: &str,
        nonce: &str,
        code: &str,
    ) -> Value {
        self.record_audit(LocalAuditEntry {
            at_ms,
            world: world.to_owned(),
            provider: provider.to_owned(),
            nonce: nonce.to_owned(),
            outcome: "refused",
            reason: Some(code.to_owned()),
        });
        json!({
            "type": "provider_reply",
            "nonce": nonce,
            "ok": false,
            "error": code,
        })
    }

    /// Handle one `provider_request` server frame for `world` and
    /// return the `provider_reply` client frame to send back. Every
    /// path — malformed, ungranted, revoked, over a local bound,
    /// executor refusal, oversize result, success — records exactly
    /// one local audit entry, and no path executes a provider the
    /// local user has not granted for this world.
    pub fn handle_request(&mut self, world: &str, frame: &Value, now_ms: u64) -> Value {
        let nonce = frame["nonce"].as_str().unwrap_or_default().to_owned();
        let provider = frame["provider"].as_str().unwrap_or_default().to_owned();
        if nonce.is_empty() || provider.is_empty() {
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_MALFORMED);
        }
        if frame["deadline_ms"].as_u64().unwrap_or_default() == 0 {
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_EXPIRED);
        }
        let key = (world.to_owned(), provider.clone());
        let Some(mut state) = self.grants.get(&key).copied() else {
            let code = if self.revoked.contains(&key) {
                REFUSAL_REVOKED
            } else {
                REFUSAL_NOT_GRANTED
            };
            return self.refuse(now_ms, world, &provider, &nonce, code);
        };
        if now_ms.saturating_sub(state.window_start_ms) >= RATE_WINDOW_MS {
            state.window_start_ms = now_ms;
            state.window_count = 0;
        }
        if state.window_count >= state.bounds.per_minute {
            self.grants.insert(key, state);
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_RATE_LIMITED);
        }
        if state.in_flight >= state.bounds.in_flight {
            self.grants.insert(key, state);
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_BUSY);
        }
        if !self.has_executor(world, &provider) {
            self.grants.insert(key, state);
            return self.refuse(now_ms, world, &provider, &nonce, REFUSAL_UNAVAILABLE);
        }
        state.window_count += 1;
        state.in_flight += 1;
        self.grants.insert(key.clone(), state);
        let args = frame["args"].clone();
        let executor = match self.world_executors.get_mut(&key) {
            Some(executor) => Some(executor),
            None => self.executors.get_mut(&provider),
        };
        let result = match executor {
            Some(executor) => executor.execute(&args, state.bounds.max_bytes),
            None => Err(REFUSAL_UNAVAILABLE.to_owned()),
        };
        if let Some(current) = self.grants.get_mut(&key) {
            current.in_flight = current.in_flight.saturating_sub(1);
        }
        match result {
            Ok(bytes) if bytes.len() <= state.bounds.max_bytes as usize => {
                self.record_audit(LocalAuditEntry {
                    at_ms: now_ms,
                    world: world.to_owned(),
                    provider: provider.clone(),
                    nonce: nonce.clone(),
                    outcome: "executed",
                    reason: None,
                });
                json!({
                    "type": "provider_reply",
                    "nonce": nonce,
                    "ok": true,
                    "result_hex": hex_encode(&bytes),
                })
            }
            Ok(_) => self.refuse(now_ms, world, &provider, &nonce, REFUSAL_TOO_LARGE),
            Err(code) => self.refuse(now_ms, world, &provider, &nonce, &code),
        }
    }
}

/// Lowercase hex encoding for reply result bytes (the contract's
/// `result_hex` field).
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct FakeExecutor(Vec<u8>);

    impl ClientProviderExecutor for FakeExecutor {
        fn execute(&mut self, _args: &Value, _max_bytes: u32) -> Result<Vec<u8>, String> {
            Ok(self.0.clone())
        }
    }

    struct RefusingExecutor;

    impl ClientProviderExecutor for RefusingExecutor {
        fn execute(&mut self, _args: &Value, _max_bytes: u32) -> Result<Vec<u8>, String> {
            Err("provider_local_refusal".to_owned())
        }
    }

    fn request(nonce: &str, provider: &str) -> Value {
        json!({
            "type": "provider_request",
            "nonce": nonce,
            "provider": provider,
            "args": {"path": "notes.txt"},
            "deadline_ms": 5000,
        })
    }

    fn granted_adapter() -> ProviderAdapter {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(FakeExecutor(vec![0x00, 0xff])));
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        adapter
    }

    #[test]
    fn the_default_adapter_offers_nothing_and_refuses_everything() {
        let mut adapter = ProviderAdapter::new();
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], REFUSAL_NOT_GRANTED);
        assert_eq!(adapter.audit_entries().len(), 1);
        assert_eq!(adapter.audit_entries()[0].outcome, "refused");
    }

    #[test]
    fn a_granted_provider_executes_and_the_reply_is_hex_bounded_bytes() {
        let mut adapter = granted_adapter();
        assert_eq!(
            adapter.offer_frame("world-a"),
            Some(json!({"type": "provider_offer", "providers": ["client.file.read"]}))
        );
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result_hex"], "00ff");
        assert_eq!(adapter.audit_entries()[0].outcome, "executed");
        assert_eq!(adapter.audit_entries()[0].reason, None);
    }

    #[test]
    fn grants_are_per_world() {
        let mut adapter = granted_adapter();
        let reply = adapter.handle_request("world-b", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], REFUSAL_NOT_GRANTED);
        assert_eq!(adapter.offer_frame("world-b"), None);
    }

    #[test]
    fn revoke_is_immediate_for_offers_and_requests() {
        let mut adapter = granted_adapter();
        assert!(adapter.revoke("world-a", "client.file.read"));
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], REFUSAL_REVOKED);
        // Re-granting clears the revoked marker.
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 2_000);
        assert_eq!(reply["ok"], true);
    }

    #[test]
    fn a_grant_without_an_executor_is_unavailable_and_never_offered() {
        let mut adapter = ProviderAdapter::new();
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], REFUSAL_UNAVAILABLE);
    }

    #[test]
    fn the_per_minute_bound_refuses_and_then_the_window_resets() {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(FakeExecutor(vec![1])));
        adapter.grant(
            "world-a",
            "client.file.read",
            LocalBounds {
                per_minute: 1,
                ..LocalBounds::DESIGN_DEFAULT
            },
        );
        let first = adapter.handle_request("world-a", &request("01", "client.file.read"), 1_000);
        assert_eq!(first["ok"], true);
        let second = adapter.handle_request("world-a", &request("02", "client.file.read"), 2_000);
        assert_eq!(second["error"], REFUSAL_RATE_LIMITED);
        let third = adapter.handle_request("world-a", &request("03", "client.file.read"), 62_000);
        assert_eq!(third["ok"], true);
    }

    #[test]
    fn an_oversize_result_is_refused_whole_never_truncated() {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(FakeExecutor(vec![7; 8])));
        adapter.grant(
            "world-a",
            "client.file.read",
            LocalBounds {
                max_bytes: 4,
                ..LocalBounds::DESIGN_DEFAULT
            },
        );
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], REFUSAL_TOO_LARGE);
        assert!(reply.get("result_hex").is_none());
    }

    #[test]
    fn executor_refusals_and_expired_requests_are_audited_with_their_codes() {
        let mut adapter = ProviderAdapter::new();
        adapter.register_executor("client.file.read", Box::new(RefusingExecutor));
        adapter.grant("world-a", "client.file.read", LocalBounds::DESIGN_DEFAULT);
        let reply = adapter.handle_request("world-a", &request("ab12", "client.file.read"), 1_000);
        assert_eq!(reply["error"], "provider_local_refusal");
        let mut expired = request("cd34", "client.file.read");
        expired["deadline_ms"] = json!(0);
        let reply = adapter.handle_request("world-a", &expired, 2_000);
        assert_eq!(reply["error"], REFUSAL_EXPIRED);
        let reasons: Vec<Option<String>> = adapter
            .audit_entries()
            .iter()
            .map(|entry| entry.reason.clone())
            .collect();
        assert_eq!(
            reasons,
            vec![
                Some("provider_local_refusal".to_owned()),
                Some(REFUSAL_EXPIRED.to_owned())
            ]
        );
    }

    #[test]
    fn bounds_compose_by_minimum() {
        let composed = LocalBounds::DESIGN_DEFAULT.compose_minimum(LocalBounds {
            max_bytes: 1024,
            per_minute: 60,
            in_flight: 1,
        });
        assert_eq!(
            composed,
            LocalBounds {
                max_bytes: 1024,
                per_minute: 10,
                in_flight: 1,
            }
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod file_read_tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    /// A unique temporary tree: `root/` is the user-selected root,
    /// and the returned base also holds an `outside.txt` sibling the
    /// root must never serve.
    fn temp_tree() -> (PathBuf, PathBuf) {
        let unique = format!(
            "id-file-read-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        );
        let base = std::env::temp_dir().join(unique);
        let root = base.join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("notes.txt"), b"hello world").unwrap();
        std::fs::write(root.join("sub").join("deep.txt"), b"deep").unwrap();
        std::fs::write(base.join("outside.txt"), b"outside").unwrap();
        (base, root)
    }

    fn executor(root: &Path) -> FileReadExecutor {
        FileReadExecutor::new(root.to_path_buf(), true).unwrap()
    }

    #[test]
    fn replacement_after_shape_check_never_serves_outside_bytes() {
        let (base, root) = temp_tree();
        std::fs::write(root.join("swap.txt"), b"inside").unwrap();
        let ex = executor(&root);
        let relative = FileReadExecutor::relative_path("swap.txt").unwrap();

        // Between the shape check and the open, the in-root name is
        // replaced by a symlink to a file outside the root.
        std::fs::remove_file(root.join("swap.txt")).unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), root.join("swap.txt")).unwrap();

        assert_eq!(
            ex.open_within_root(relative).err().as_deref(),
            Some(REFUSAL_INVALID_PATH)
        );
    }

    #[test]
    fn reads_files_beneath_the_root() {
        let (base, root) = temp_tree();
        let mut exec = executor(&root);
        assert_eq!(
            exec.execute(&json!({"path": "notes.txt"}), 32_768).unwrap(),
            b"hello world"
        );
        assert_eq!(
            exec.execute(&json!({"path": "sub/deep.txt"}), 32_768)
                .unwrap(),
            b"deep"
        );
        assert_eq!(
            exec.execute(&json!({"path": "./notes.txt"}), 32_768)
                .unwrap(),
            b"hello world"
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn refuses_absolute_traversal_and_missing_paths() {
        let (base, root) = temp_tree();
        let mut exec = executor(&root);
        for bad in ["/etc/passwd", "../outside.txt", "sub/../../outside.txt", ""] {
            assert_eq!(
                exec.execute(&json!({"path": bad}), 32_768),
                Err(REFUSAL_INVALID_PATH.to_owned()),
                "path {bad:?}"
            );
        }
        assert_eq!(
            exec.execute(&json!({"path": "missing.txt"}), 32_768),
            Err(REFUSAL_NOT_FOUND.to_owned())
        );
        assert_eq!(
            exec.execute(&json!({"path": "sub"}), 32_768),
            Err(REFUSAL_NOT_A_FILE.to_owned())
        );
        assert_eq!(
            exec.execute(&json!({}), 32_768),
            Err(REFUSAL_INVALID_PATH.to_owned())
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_escape_but_serves_in_root_symlinks() {
        let (base, root) = temp_tree();
        std::os::unix::fs::symlink(base.join("outside.txt"), root.join("escape.txt")).unwrap();
        std::os::unix::fs::symlink(root.join("notes.txt"), root.join("alias.txt")).unwrap();
        let mut exec = executor(&root);
        assert_eq!(
            exec.execute(&json!({"path": "escape.txt"}), 32_768),
            Err(REFUSAL_INVALID_PATH.to_owned())
        );
        assert_eq!(
            exec.execute(&json!({"path": "alias.txt"}), 32_768).unwrap(),
            b"hello world"
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn refuses_over_bound_files_whole_never_truncated() {
        let (base, root) = temp_tree();
        std::fs::write(root.join("big.bin"), vec![7_u8; 4096]).unwrap();
        let mut exec = executor(&root);
        assert_eq!(
            exec.execute(&json!({"path": "big.bin"}), 1024),
            Err(REFUSAL_TOO_LARGE.to_owned())
        );
        assert_eq!(
            exec.execute(&json!({"path": "big.bin"}), 4096)
                .unwrap()
                .len(),
            4096
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn disclosure_is_required_to_grant_and_to_execute() {
        let (base, root) = temp_tree();
        // An executor built without the acknowledgment refuses
        // every read, even of an in-root file.
        let mut unacknowledged = FileReadExecutor::new(root.clone(), false).unwrap();
        assert_eq!(
            unacknowledged.execute(&json!({"path": "notes.txt"}), 32_768),
            Err(REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED.to_owned())
        );
        // The adapter-level opt-in refuses to grant at all without
        // it, and offers nothing.
        let mut adapter = ProviderAdapter::new();
        assert_eq!(
            adapter.grant_file_read("world-a", root.clone(), LocalBounds::DESIGN_DEFAULT, false),
            Err(REFUSAL_DISCLOSURE_NOT_ACKNOWLEDGED.to_owned())
        );
        assert_eq!(adapter.offer_frame("world-a"), None);
        // With it, the grant registers and a request executes
        // end-to-end through the adapter's local check.
        adapter
            .grant_file_read("world-a", root, LocalBounds::DESIGN_DEFAULT, true)
            .unwrap();
        assert!(adapter.is_granted("world-a", CLIENT_FILE_READ_PROVIDER));
        let reply = adapter.handle_request(
            "world-a",
            &json!({
                "type": "provider_request",
                "nonce": "n1",
                "provider": CLIENT_FILE_READ_PROVIDER,
                "args": {"path": "notes.txt"},
                "deadline_ms": 5000,
            }),
            1_000,
        );
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result_hex"], json!(hex_encode(b"hello world")));
        std::fs::remove_dir_all(base).ok();
    }

    /// #11478: two worlds granted `client.file.read` on one adapter,
    /// each with its own root holding a distinct sentinel. Each world
    /// reads only its own root, whichever grant came last.
    #[test]
    fn file_read_grants_for_two_worlds_keep_their_own_roots() {
        let read = |adapter: &mut ProviderAdapter, world: &str| {
            adapter.handle_request(
                world,
                &json!({
                    "type": "provider_request",
                    "nonce": format!("n-{world}"),
                    "provider": CLIENT_FILE_READ_PROVIDER,
                    "args": {"path": "sentinel.txt"},
                    "deadline_ms": 5000,
                }),
                1_000,
            )
        };
        for a_first in [true, false] {
            let (base_a, root_a) = temp_tree();
            let (base_b, root_b) = temp_tree();
            std::fs::write(root_a.join("sentinel.txt"), b"root-a").unwrap();
            std::fs::write(root_b.join("sentinel.txt"), b"root-b").unwrap();
            let mut adapter = ProviderAdapter::new();
            let grants = if a_first {
                [("world-a", &root_a), ("world-b", &root_b)]
            } else {
                [("world-b", &root_b), ("world-a", &root_a)]
            };
            for (world, root) in grants {
                adapter
                    .grant_file_read(world, root.clone(), LocalBounds::DESIGN_DEFAULT, true)
                    .unwrap();
            }
            let reply = read(&mut adapter, "world-a");
            assert_eq!(reply["ok"], true, "a_first={a_first}: {reply}");
            assert_eq!(
                reply["result_hex"],
                json!(hex_encode(b"root-a")),
                "a_first={a_first}"
            );
            let reply = read(&mut adapter, "world-b");
            assert_eq!(reply["ok"], true, "a_first={a_first}: {reply}");
            assert_eq!(
                reply["result_hex"],
                json!(hex_encode(b"root-b")),
                "a_first={a_first}"
            );
            // Both worlds still offer the provider.
            assert!(adapter.offer_frame("world-a").is_some());
            assert!(adapter.offer_frame("world-b").is_some());
            std::fs::remove_dir_all(base_a).ok();
            std::fs::remove_dir_all(base_b).ok();
        }
    }

    /// Revoking a world's file-read grant drops the root bound to it,
    /// so a later bare `grant` for that world cannot revive the old
    /// root: with no executor the provider is not offered and a
    /// request refuses as unavailable.
    #[test]
    fn revoke_drops_the_world_bound_root() {
        let (base, root) = temp_tree();
        let mut adapter = ProviderAdapter::new();
        adapter
            .grant_file_read("world-a", root, LocalBounds::DESIGN_DEFAULT, true)
            .unwrap();
        assert!(adapter.revoke("world-a", CLIENT_FILE_READ_PROVIDER));
        adapter.grant(
            "world-a",
            CLIENT_FILE_READ_PROVIDER,
            LocalBounds::DESIGN_DEFAULT,
        );
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request(
            "world-a",
            &json!({
                "type": "provider_request",
                "nonce": "ab12",
                "provider": CLIENT_FILE_READ_PROVIDER,
                "args": {"path": "notes.txt"},
                "deadline_ms": 5000,
            }),
            1_000,
        );
        assert_eq!(reply["error"], REFUSAL_UNAVAILABLE);
        std::fs::remove_dir_all(base).ok();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod notify_tests {
    use super::*;

    fn notify_request(nonce: &str, args: Value) -> Value {
        json!({
            "type": "provider_request",
            "nonce": nonce,
            "provider": CLIENT_NOTIFY_PROVIDER,
            "args": args,
            "deadline_ms": 5000,
        })
    }

    fn notified_adapter() -> ProviderAdapter {
        let mut adapter = ProviderAdapter::new();
        adapter.grant_notify("world-a", LocalBounds::NOTIFY_DEFAULT);
        adapter
    }

    #[test]
    fn the_notify_executor_acks_a_head_hint_with_no_content() {
        let mut executor = NotifyExecutor::new();
        let ack = executor.execute(&json!({"head": 42}), 1024).unwrap();
        assert!(ack.is_empty(), "the ack carries no world content");
        assert_eq!(executor.surfaced_heads(), &[Some(42)]);
    }

    #[test]
    fn the_notify_executor_accepts_a_headless_hint_as_resync() {
        let mut executor = NotifyExecutor::new();
        assert!(executor.execute(&json!({}), 1024).is_ok());
        assert!(executor.execute(&json!({"head": null}), 1024).is_ok());
        assert_eq!(executor.surfaced_heads(), &[None, None]);
    }

    #[test]
    fn the_notify_executor_refuses_content_bearing_fields() {
        let mut executor = NotifyExecutor::new();
        for args in [
            json!({"head": 7, "text": "hello"}),
            json!({"content": "hello"}),
            json!({"body": "hello"}),
            json!({"data": "aGVsbG8="}),
            json!({"message": "hello"}),
        ] {
            assert_eq!(
                executor.execute(&args, 1024),
                Err(REFUSAL_CONTENT_FORBIDDEN.to_owned()),
                "args {args}"
            );
        }
        assert!(executor.surfaced_heads().is_empty());
    }

    #[test]
    fn the_notify_executor_refuses_a_non_unsigned_head() {
        let mut executor = NotifyExecutor::new();
        for args in [
            json!({"head": "42"}),
            json!({"head": -1}),
            json!({"head": 4.5}),
            json!({"head": true}),
        ] {
            assert_eq!(
                executor.execute(&args, 1024),
                Err(REFUSAL_INVALID_HEAD.to_owned()),
                "args {args}"
            );
        }
        assert!(executor.surfaced_heads().is_empty());
    }

    #[test]
    fn a_granted_notify_provider_is_offered_and_replies_with_an_empty_ack() {
        let mut adapter = notified_adapter();
        assert_eq!(
            adapter.offer_frame("world-a"),
            Some(json!({"type": "provider_offer", "providers": ["client.notify"]}))
        );
        let reply =
            adapter.handle_request("world-a", &notify_request("n1", json!({"head": 9})), 1_000);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result_hex"], json!(""));
        assert_eq!(adapter.audit_entries()[0].outcome, "executed");
        // A second world without the grant is still refused, never offered.
        assert_eq!(adapter.offer_frame("world-b"), None);
        let reply =
            adapter.handle_request("world-b", &notify_request("n2", json!({"head": 9})), 1_000);
        assert_eq!(reply["error"], REFUSAL_NOT_GRANTED);
    }

    #[test]
    fn revoke_is_immediate_for_notify_offers_and_requests() {
        let mut adapter = notified_adapter();
        assert!(adapter.revoke("world-a", CLIENT_NOTIFY_PROVIDER));
        assert_eq!(adapter.offer_frame("world-a"), None);
        let reply = adapter.handle_request("world-a", &notify_request("n1", json!({})), 1_000);
        assert_eq!(reply["error"], REFUSAL_REVOKED);
        adapter.grant_notify("world-a", LocalBounds::NOTIFY_DEFAULT);
        let reply = adapter.handle_request("world-a", &notify_request("n2", json!({})), 2_000);
        assert_eq!(reply["ok"], true);
    }

    #[test]
    fn notify_needs_resync_ignores_a_hint_at_or_behind_the_local_head() {
        assert!(notify_needs_resync(None, Some(9)));
        assert!(notify_needs_resync(None, None));
        assert!(notify_needs_resync(Some(3), None));
        assert!(notify_needs_resync(Some(3), Some(9)));
        assert!(!notify_needs_resync(Some(9), Some(9)));
        assert!(!notify_needs_resync(Some(10), Some(9)));
    }

    #[test]
    fn the_notify_then_resync_flow_surfaces_first_and_decides_second() {
        // Behavioral check for #10930's Done: a hint is delivered, the
        // shell surfaces its local notification, and on return the
        // shell re-syncs exactly when it is behind the hinted head.
        // The rendered view itself is rebuilt from the client
        // projection plus the host view (slice 1's catch-up seam,
        // #10927); this test pins the decision that gates that run.
        let mut adapter = notified_adapter();
        let reply =
            adapter.handle_request("world-a", &notify_request("n1", json!({"head": 12})), 1_000);
        assert_eq!(reply["ok"], true);
        // A shell behind the hint must re-sync on return ...
        assert!(notify_needs_resync(Some(7), Some(12)));
        // ... while a shell already past it ignores the hint.
        assert!(!notify_needs_resync(Some(12), Some(12)));
        assert!(!notify_needs_resync(Some(13), Some(12)));
        // A headless hint always re-syncs: something changed.
        let reply = adapter.handle_request("world-a", &notify_request("n2", json!({})), 2_000);
        assert_eq!(reply["ok"], true);
        assert!(notify_needs_resync(Some(13), None));
        // Both deliveries are audited as executions, with no content.
        assert_eq!(adapter.audit_entries().len(), 2);
        assert!(
            adapter
                .audit_entries()
                .iter()
                .all(|entry| entry.outcome == "executed")
        );
    }
}
