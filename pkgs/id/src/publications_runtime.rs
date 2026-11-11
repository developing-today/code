//! Runtime apply for dynamic publications (#2147, slices 2–3: the
//! serve-owned manager core).
//!
//! Slice 1 ([`crate::publications`]) is the persisted registry — the
//! desired set, deliberately socket-free. This module is the runtime
//! half the design record for #2147 specifies
//! (`doc/2026-11-04T21-50-26Z_feature_dynamic_publications_runtime`):
//! ONE manager owns the registry and the live resources it maps to,
//! serializes every mutation through the same apply path startup
//! uses, and never lets the durable set and the live set disagree
//! silently.
//!
//! The transactional ordering is the design's 2026-11-04 decision:
//!
//! - **Publish (listener):** validate against a candidate registry,
//!   bind a *staged* listener, atomically save the candidate
//!   registry, then activate the staged listener. A bind failure
//!   changes nothing; a save failure closes the staged listener
//!   without changing the active set.
//! - **Publish (path mount):** validate, save the candidate, then
//!   mount on the primary Explorer listener. Mounts target the
//!   first effective Explorer binding; with no Explorer listener
//!   the publish fails before anything is saved.
//! - **Unpublish:** atomically save the candidate set *before*
//!   removing the live route or stopping the listener — a crash
//!   after the save restarts from the new set, so the order is
//!   safe. Listener closes are infallible; a mount removal that
//!   fails (its contract: the route was not removed) restores
//!   the pre-op registry in memory and on disk before the error
//!   propagates, so the durable and live sets still agree.
//! - **Restore (boot):** apply the persisted set through the same
//!   resource path; a failure part-way rolls back what was applied
//!   and fails loudly, matching slice 1's fail-loud load contract.
//!
//! The manager is generic over the two resource seams
//! ([`ListenerFactory`] and [`MountSink`]) so the apply logic is
//! testable without sockets: `cmd_serve` supplies the real factory
//! (its listener tasks, preserving each listener's `WebSecurity`)
//! and the real Explorer mount sink. Wiring those adapters, the
//! operator-only meta-protocol control surface, and the CLI
//! publish/unpublish/list commands is the remaining slice-3 work;
//! the CLI must drive THIS manager (never edit the registry file
//! independently, and never fall back to a local-store operation
//! when no serve process is running — the design's control
//! decision).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};

use crate::publications::{Placement, Publication, Registry};

/// A live listener opened for a [`Placement::Listener`] publication.
pub trait LiveListener: Send {
    /// The address the listener is actually bound to. For a
    /// requested port of 0 this is the OS-assigned address, which
    /// is what `list` reports while the process runs (the registry
    /// keeps the requested address, per the design decision).
    fn actual_addr(&self) -> SocketAddr;

    /// Stop the listener and release the address.
    fn close(self: Box<Self>);
}

/// Opens live listeners for listener placements. Implemented by
/// `cmd_serve` over its real listener tasks.
pub trait ListenerFactory: Send {
    /// Bind a listener serving `world` at `addr`, or fail without
    /// side effects.
    fn bind(&mut self, world: &str, addr: SocketAddr) -> Result<Box<dyn LiveListener>>;
}

/// Applies path mounts to the primary Explorer listener.
pub trait MountSink: Send {
    /// Whether an Explorer listener exists to mount on at all.
    fn has_explorer(&self) -> bool;

    /// Mount `world` at `path` on the Explorer listener, so the
    /// publication overrides the Explorer there.
    fn mount(&mut self, world: &str, path: &str) -> Result<()>;

    /// Remove the mount at `path`, restoring the Explorer route.
    ///
    /// Contract: an error means the mount was NOT removed — the
    /// route is still live. The manager relies on this to keep the
    /// persisted registry and the live set in agreement when a
    /// removal fails (it restores the registry entry; see
    /// [`PublicationManager::unpublish`]).
    fn unmount(&mut self, path: &str) -> Result<()>;
}

/// One publication as `list` reports it: the persisted record plus
/// the live address for listener placements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationStatus {
    /// The persisted publication.
    pub publication: Publication,
    /// The live bound address for a listener placement (the
    /// requested address is in the publication; for port 0 the two
    /// differ). `None` for path mounts and for a listener that is
    /// not currently live.
    pub actual_addr: Option<SocketAddr>,
}

/// The serve-owned publication manager: the persisted registry
/// plus every live resource it maps to.
pub struct PublicationManager<F: ListenerFactory, M: MountSink> {
    registry: Registry,
    registry_path: PathBuf,
    explorer_routes: Vec<String>,
    factory: F,
    mounts: M,
    listeners: BTreeMap<SocketAddr, Box<dyn LiveListener>>,
    mounted: Vec<String>,
}

impl<F: ListenerFactory, M: MountSink> PublicationManager<F, M> {
    /// A manager over an already-loaded `registry` (slice 1's boot
    /// load). No live resources exist until [`Self::restore`] (or a
    /// publish) applies them.
    pub fn new(
        registry: Registry,
        registry_path: PathBuf,
        explorer_routes: Vec<String>,
        factory: F,
        mounts: M,
    ) -> Self {
        Self {
            registry,
            registry_path,
            explorer_routes,
            factory,
            mounts,
            listeners: BTreeMap::new(),
            mounted: Vec::new(),
        }
    }

    /// The persisted (desired) set.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    fn routes(&self) -> Vec<&str> {
        self.explorer_routes.iter().map(String::as_str).collect()
    }

    /// Apply the persisted set at startup, through the same
    /// resource path as runtime publishes. Returns the boot shadow
    /// warnings (slice 1's [`Registry::boot_warnings`]).
    ///
    /// # Errors
    ///
    /// Fails when any persisted publication cannot be applied (a
    /// listener that will not bind, a path mount with no Explorer
    /// listener); everything applied so far is rolled back first,
    /// so a failed restore leaves no half-applied set behind.
    pub fn restore(&mut self) -> Result<Vec<String>> {
        let warnings = self.registry.boot_warnings(&self.routes());
        let persisted = self.registry.publications.clone();
        for publication in &persisted {
            if let Err(error) = self.activate(publication) {
                self.rollback_all();
                return Err(error).context(format!(
                    "restoring publication of world '{}' at {}",
                    publication.world,
                    publication.placement.describe()
                ));
            }
        }
        Ok(warnings)
    }

    /// Publish `publication`: validate, stage, persist, activate —
    /// in that order (see the module docs). Returns the loud shadow
    /// warning for an acknowledged shadowing mount.
    ///
    /// # Errors
    ///
    /// Fails on any validation refusal from the registry (taken
    /// placement, unacknowledged shadow), a bind failure, a mount
    /// with no Explorer listener, or a persistence failure. On
    /// every failure the live and persisted sets are unchanged.
    pub fn publish(&mut self, publication: Publication) -> Result<Option<String>> {
        let mut candidate = self.registry.clone();
        let warning = candidate.publish(publication.clone(), &self.routes())?;

        // Stage the live resource before persisting, so a bind
        // failure changes nothing at all.
        let staged = match &publication.placement {
            Placement::Listener { addr } => Some(self.factory.bind(&publication.world, *addr)?),
            Placement::PathMount { .. } => {
                ensure!(
                    self.mounts.has_explorer(),
                    "cannot publish a path mount: no Explorer listener is running \
                     (path mounts target the primary Explorer listener)"
                );
                None
            }
        };

        if let Err(error) = candidate.save(&self.registry_path) {
            // Save failed: close the staged listener without
            // changing the active set.
            if let Some(listener) = staged {
                listener.close();
            }
            return Err(error).context("persisting the publications registry");
        }
        self.registry = candidate;

        match &publication.placement {
            Placement::Listener { addr } => {
                let listener = staged.expect("listener placements stage a listener");
                self.listeners.insert(*addr, listener);
            }
            Placement::PathMount { path } => {
                if let Err(error) = self.mounts.mount(&publication.world, path) {
                    // The registry is already persisted with the
                    // mount in it; revert the persisted set so the
                    // two never silently disagree, and say so.
                    let mut reverted = self.registry.clone();
                    let _ = reverted.unpublish(&publication.placement);
                    let _ = reverted.save(&self.registry_path);
                    self.registry = reverted;
                    return Err(error).context("applying the path mount after persisting it");
                }
                self.mounted.push(path.clone());
            }
        }
        Ok(warning)
    }

    /// Unpublish the publication at `placement`: persist the
    /// candidate set first, then remove the live resource.
    ///
    /// # Errors
    ///
    /// Fails when nothing is published at `placement` or the
    /// candidate set cannot be persisted (the live set is then
    /// untouched).
    pub fn unpublish(&mut self, placement: &Placement) -> Result<Publication> {
        // Snapshot for recovery: if removing the live mount fails
        // after the candidate set has been persisted, the pre-op
        // registry is restored (memory and disk) so the durable
        // set and the live set never silently disagree.
        let previous = self.registry.clone();
        let mut candidate = self.registry.clone();
        let removed = candidate.unpublish(placement)?;
        candidate
            .save(&self.registry_path)
            .context("persisting the publications registry")?;
        self.registry = candidate;

        // Act on the REMOVED record's placement, never the caller's
        // spelling: the registry's match normalises spellings
        // (`/arena/`, `//arena`) that canonicalize_mount_path
        // rejects outright, and the stored placement is the
        // canonical path publish saved. Resolving the caller's
        // spelling here would fail after the removal was already
        // persisted, stranding a live mount no registry names.
        match &removed.placement {
            Placement::Listener { addr } => {
                if let Some(listener) = self.listeners.remove(addr) {
                    listener.close();
                }
            }
            Placement::PathMount { path } => {
                if self.mounted.iter().any(|mounted| mounted == path) {
                    if let Err(error) = self.mounts.unmount(path) {
                        // The sink's contract is that an error
                        // means the route was NOT removed, so the
                        // live set is unchanged: restore the durable
                        // set to match it before propagating.
                        let restored = previous.save(&self.registry_path).is_ok();
                        self.registry = previous;
                        return Err(error).context(format!(
                            "removing the live mount at {path} failed; the publication \
                             was restored to the registry{}",
                            if restored {
                                ""
                            } else {
                                " in memory (restoring it on disk also failed)"
                            }
                        ));
                    }
                    self.mounted.retain(|mounted| mounted != path);
                }
            }
        }
        Ok(removed)
    }

    /// The current set as an operator listing shows it: persisted
    /// records in publish order, listener placements annotated
    /// with their live bound address.
    #[must_use]
    pub fn list(&self) -> Vec<PublicationStatus> {
        self.registry
            .publications
            .iter()
            .map(|publication| PublicationStatus {
                publication: publication.clone(),
                actual_addr: match &publication.placement {
                    Placement::Listener { addr } => {
                        self.listeners.get(addr).map(|live| live.actual_addr())
                    }
                    Placement::PathMount { .. } => None,
                },
            })
            .collect()
    }

    /// Activate one already-persisted publication's live resource
    /// (restore's inner step; publish stages its own).
    fn activate(&mut self, publication: &Publication) -> Result<()> {
        match &publication.placement {
            Placement::Listener { addr } => {
                let listener = self.factory.bind(&publication.world, *addr)?;
                self.listeners.insert(*addr, listener);
                Ok(())
            }
            Placement::PathMount { path } => {
                ensure!(
                    self.mounts.has_explorer(),
                    "cannot restore a path mount: no Explorer listener is running"
                );
                self.mounts.mount(&publication.world, path)?;
                self.mounted.push(path.clone());
                Ok(())
            }
        }
    }

    fn rollback_all(&mut self) {
        for (_, listener) in std::mem::take(&mut self.listeners) {
            listener.close();
        }
        for path in std::mem::take(&mut self.mounted) {
            let _ = self.mounts.unmount(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publications::KNOWN_EXPLORER_ROUTES;
    use std::sync::{Arc, Mutex};

    struct FakeState {
        next_port: u16,
        fail_addrs: Vec<SocketAddr>,
        closed: Vec<SocketAddr>,
        mounted: Vec<String>,
        unmounted: Vec<String>,
        // Failure injection + live-state tracking for the model
        // test: `live_listeners` / `live_mounts` are the CURRENT
        // live sets (the `mounted` / `unmounted` logs above are
        // append-only history).
        explorer: bool,
        fail_mounts: Vec<String>,
        fail_unmounts: Vec<String>,
        live_listeners: Vec<SocketAddr>,
        live_mounts: Vec<String>,
    }

    impl Default for FakeState {
        fn default() -> Self {
            Self {
                next_port: 0,
                fail_addrs: Vec::new(),
                closed: Vec::new(),
                mounted: Vec::new(),
                unmounted: Vec::new(),
                explorer: true,
                fail_mounts: Vec::new(),
                fail_unmounts: Vec::new(),
                live_listeners: Vec::new(),
                live_mounts: Vec::new(),
            }
        }
    }

    struct FakeListener {
        actual: SocketAddr,
        state: Arc<Mutex<FakeState>>,
    }

    impl LiveListener for FakeListener {
        fn actual_addr(&self) -> SocketAddr {
            self.actual
        }

        fn close(self: Box<Self>) {
            let mut state = self.state.lock().unwrap();
            state.closed.push(self.actual);
            state.live_listeners.retain(|addr| *addr != self.actual);
        }
    }

    struct FakeFactory {
        state: Arc<Mutex<FakeState>>,
    }

    impl ListenerFactory for FakeFactory {
        fn bind(&mut self, _world: &str, addr: SocketAddr) -> Result<Box<dyn LiveListener>> {
            let mut state = self.state.lock().unwrap();
            ensure!(
                !state.fail_addrs.contains(&addr),
                "address {addr} is taken (fake)"
            );
            let actual = if addr.port() == 0 {
                state.next_port += 1;
                SocketAddr::new(addr.ip(), 40_000 + state.next_port)
            } else {
                addr
            };
            state.live_listeners.push(actual);
            Ok(Box::new(FakeListener {
                actual,
                state: self.state.clone(),
            }))
        }
    }

    struct FakeMounts {
        state: Arc<Mutex<FakeState>>,
    }

    impl MountSink for FakeMounts {
        fn has_explorer(&self) -> bool {
            self.state.lock().unwrap().explorer
        }

        fn mount(&mut self, _world: &str, path: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            ensure!(
                !state.fail_mounts.iter().any(|failed| failed == path),
                "mount at {path} failed (fake)"
            );
            state.mounted.push(path.to_string());
            state.live_mounts.push(path.to_string());
            Ok(())
        }

        fn unmount(&mut self, path: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            ensure!(
                !state.fail_unmounts.iter().any(|failed| failed == path),
                "unmount at {path} failed (fake): the route was not removed"
            );
            state.unmounted.push(path.to_string());
            state.live_mounts.retain(|mounted| mounted != path);
            Ok(())
        }
    }

    fn listener_pub(addr: &str) -> Publication {
        Publication {
            world: "arena".to_string(),
            placement: Placement::Listener {
                addr: addr.parse().unwrap(),
            },
            acknowledge_shadow: false,
        }
    }

    fn mount_pub(path: &str, acknowledge_shadow: bool) -> Publication {
        Publication {
            world: "arena".to_string(),
            placement: Placement::PathMount {
                path: path.to_string(),
            },
            acknowledge_shadow,
        }
    }

    struct Fixture {
        dir: PathBuf,
        file: PathBuf,
        state: Arc<Mutex<FakeState>>,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "id-publications-runtime-test-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("publications.json");
            Self {
                dir,
                file,
                state: Arc::new(Mutex::new(FakeState::default())),
            }
        }

        fn manager(
            &self,
            registry: Registry,
            explorer: bool,
        ) -> PublicationManager<FakeFactory, FakeMounts> {
            self.state.lock().unwrap().explorer = explorer;
            PublicationManager::new(
                registry,
                self.file.clone(),
                KNOWN_EXPLORER_ROUTES
                    .iter()
                    .map(|route| (*route).to_string())
                    .collect(),
                FakeFactory {
                    state: self.state.clone(),
                },
                FakeMounts {
                    state: self.state.clone(),
                },
            )
        }

        fn cleanup(&self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn restore_applies_the_persisted_set_and_reports_actual_addresses() {
        let fx = Fixture::new("restore");
        let mut registry = Registry::default();
        registry
            .publish(listener_pub("127.0.0.1:0"), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        registry
            .publish(mount_pub("/arena", false), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        let mut manager = fx.manager(registry, true);
        let warnings = manager.restore().unwrap();
        assert!(warnings.is_empty());
        let listed = manager.list();
        assert_eq!(listed.len(), 2);
        let actual = listed[0].actual_addr.expect("listener is live");
        assert_ne!(actual.port(), 0, "list reports the OS-assigned port");
        assert_eq!(fx.state.lock().unwrap().mounted, vec!["/arena".to_string()]);
        fx.cleanup();
    }

    #[test]
    fn restore_rolls_back_when_a_bind_fails() {
        let fx = Fixture::new("restore-rollback");
        fx.state
            .lock()
            .unwrap()
            .fail_addrs
            .push("127.0.0.1:4002".parse().unwrap());
        let mut registry = Registry::default();
        registry
            .publish(listener_pub("127.0.0.1:4001"), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        registry
            .publish(listener_pub("127.0.0.1:4002"), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        let mut manager = fx.manager(registry, true);
        assert!(manager.restore().is_err());
        // The first listener was applied, then rolled back.
        assert_eq!(
            fx.state.lock().unwrap().closed,
            vec!["127.0.0.1:4001".parse::<SocketAddr>().unwrap()]
        );
        assert!(manager.list().iter().all(|s| s.actual_addr.is_none()));
        fx.cleanup();
    }

    #[test]
    fn publish_listener_persists_and_reports_the_actual_address() {
        let fx = Fixture::new("publish-listener");
        let mut manager = fx.manager(Registry::default(), true);
        let warning = manager.publish(listener_pub("127.0.0.1:0")).unwrap();
        assert!(warning.is_none());
        let listed = manager.list();
        assert_eq!(listed.len(), 1);
        assert_ne!(listed[0].actual_addr.unwrap().port(), 0);
        // The persisted registry keeps the REQUESTED address.
        let on_disk = Registry::load(&fx.file).unwrap();
        assert_eq!(on_disk, *manager.registry());
        assert_eq!(
            on_disk.publications[0].placement,
            Placement::Listener {
                addr: "127.0.0.1:0".parse().unwrap()
            }
        );
        fx.cleanup();
    }

    #[test]
    fn publish_bind_failure_changes_nothing() {
        let fx = Fixture::new("publish-bind-fails");
        fx.state
            .lock()
            .unwrap()
            .fail_addrs
            .push("127.0.0.1:4002".parse().unwrap());
        let mut manager = fx.manager(Registry::default(), true);
        assert!(manager.publish(listener_pub("127.0.0.1:4002")).is_err());
        assert!(manager.registry().publications.is_empty());
        assert!(!fx.file.exists(), "nothing was persisted");
        fx.cleanup();
    }

    #[test]
    fn publish_save_failure_closes_the_staged_listener() {
        let fx = Fixture::new("publish-save-fails");
        // A registry path whose parent is a FILE: save's temp write
        // fails, after the staged bind succeeded.
        let blocker = fx.dir.join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let mut manager = PublicationManager::new(
            Registry::default(),
            blocker.join("publications.json"),
            KNOWN_EXPLORER_ROUTES
                .iter()
                .map(|route| (*route).to_string())
                .collect(),
            FakeFactory {
                state: fx.state.clone(),
            },
            FakeMounts {
                state: fx.state.clone(),
            },
        );
        assert!(manager.publish(listener_pub("127.0.0.1:4001")).is_err());
        assert!(manager.registry().publications.is_empty());
        assert_eq!(
            fx.state.lock().unwrap().closed,
            vec!["127.0.0.1:4001".parse::<SocketAddr>().unwrap()],
            "the staged listener was closed, not activated"
        );
        fx.cleanup();
    }

    #[test]
    fn publish_mount_shadow_gate_and_explorer_requirement() {
        let fx = Fixture::new("publish-mount");
        let mut manager = fx.manager(Registry::default(), true);
        // Unacknowledged shadow: refused by the registry gate,
        // nothing mounted, nothing persisted.
        assert!(manager.publish(mount_pub("/world", false)).is_err());
        assert!(fx.state.lock().unwrap().mounted.is_empty());
        // Acknowledged: warns loudly and mounts.
        let warning = manager.publish(mount_pub("/world", true)).unwrap();
        assert!(
            warning
                .expect("acknowledged shadow warns")
                .contains("/world")
        );
        assert_eq!(fx.state.lock().unwrap().mounted, vec!["/world".to_string()]);

        // No Explorer listener: the publish fails before saving.
        let fx2 = Fixture::new("publish-mount-no-explorer");
        let mut manager2 = fx2.manager(Registry::default(), false);
        assert!(manager2.publish(mount_pub("/arena", false)).is_err());
        assert!(manager2.registry().publications.is_empty());
        assert!(!fx2.file.exists());
        fx.cleanup();
        fx2.cleanup();
    }

    #[test]
    fn unpublish_reverts_live_and_persisted_state() {
        let fx = Fixture::new("unpublish");
        let mut manager = fx.manager(Registry::default(), true);
        manager.publish(listener_pub("127.0.0.1:4001")).unwrap();
        manager.publish(mount_pub("/arena", false)).unwrap();

        let removed = manager
            .unpublish(&Placement::Listener {
                addr: "127.0.0.1:4001".parse().unwrap(),
            })
            .unwrap();
        assert_eq!(removed.world, "arena");
        assert_eq!(
            fx.state.lock().unwrap().closed,
            vec!["127.0.0.1:4001".parse::<SocketAddr>().unwrap()]
        );
        manager
            .unpublish(&Placement::PathMount {
                path: "/arena".to_string(),
            })
            .unwrap();
        assert_eq!(
            fx.state.lock().unwrap().unmounted,
            vec!["/arena".to_string()]
        );
        assert!(manager.registry().publications.is_empty());
        assert_eq!(Registry::load(&fx.file).unwrap().publications.len(), 0);
        // A second unpublish of the same placement is an error.
        assert!(
            manager
                .unpublish(&Placement::PathMount {
                    path: "/arena".to_string(),
                })
                .is_err()
        );
        fx.cleanup();
    }

    #[test]
    fn unpublish_accepts_a_noncanonical_spelling_and_unmounts() {
        // Regression pin for the PR #7195 Codex P2 (2026-11-11): the
        // registry match normalises `/arena/`, but resolving that
        // spelling after the removal was persisted used to fail,
        // stranding a live mount no registry named.
        let fx = Fixture::new("unpublish-spelling");
        let mut manager = fx.manager(Registry::default(), true);
        manager.publish(mount_pub("/arena", false)).unwrap();
        let removed = manager
            .unpublish(&Placement::PathMount {
                path: "/arena/".to_string(),
            })
            .unwrap();
        assert_eq!(
            removed.placement,
            Placement::PathMount {
                path: "/arena".to_string()
            }
        );
        assert_eq!(
            fx.state.lock().unwrap().unmounted,
            vec!["/arena".to_string()]
        );
        assert!(fx.state.lock().unwrap().live_mounts.is_empty());
        assert!(manager.registry().publications.is_empty());
        fx.cleanup();
    }

    #[test]
    fn unpublish_unmount_failure_restores_the_publication() {
        // Regression pin for the PR #7195 Codex P2 (2026-11-11): an
        // unmount failure after the removal was persisted used to
        // leave the route live but unnamed by any registry, and a
        // retry then failed with "nothing published".
        let fx = Fixture::new("unpublish-unmount-fails");
        let mut manager = fx.manager(Registry::default(), true);
        manager.publish(mount_pub("/arena", false)).unwrap();
        fx.state
            .lock()
            .unwrap()
            .fail_unmounts
            .push("/arena".to_string());
        assert!(
            manager
                .unpublish(&Placement::PathMount {
                    path: "/arena".to_string(),
                })
                .is_err()
        );
        // Everything is exactly as it was before the failed call.
        assert_eq!(manager.registry().publications.len(), 1);
        assert_eq!(Registry::load(&fx.file).unwrap().publications.len(), 1);
        assert_eq!(
            fx.state.lock().unwrap().live_mounts,
            vec!["/arena".to_string()]
        );
        // And a retry with the failure cleared succeeds.
        fx.state.lock().unwrap().fail_unmounts.clear();
        manager
            .unpublish(&Placement::PathMount {
                path: "/arena".to_string(),
            })
            .unwrap();
        assert!(manager.registry().publications.is_empty());
        assert!(fx.state.lock().unwrap().live_mounts.is_empty());
        fx.cleanup();
    }

    // ---- Model-based state-machine coverage (PR #7195 Codex P1) ----
    //
    // Oracle: the module doc's transactional ordering, stated as a
    // reference model — after EVERY operation the manager's
    // registry, the on-disk registry, and the live listener/mount
    // sets agree with the model, and a failed operation changes
    // none of them. Generated publish/unpublish sequences run
    // against bind, save, mount, and unmount failure injection.

    #[derive(Debug, Clone)]
    enum Op {
        PublishListener(usize),
        PublishMount(usize),
        UnpublishListener(usize),
        UnpublishMount(usize, usize),
        ToggleBindFail(usize),
        ToggleMountFail(usize),
        ToggleUnmountFail(usize),
        ToggleSaveFail,
        ToggleExplorer,
    }

    fn op_strategy() -> impl proptest::strategy::Strategy<Value = Op> {
        use proptest::prelude::*;
        prop_oneof![
            (0..2usize).prop_map(Op::PublishListener),
            (0..2usize).prop_map(Op::PublishMount),
            (0..2usize).prop_map(Op::UnpublishListener),
            (0..2usize, 0..3usize).prop_map(|(path, spelling)| Op::UnpublishMount(path, spelling)),
            (0..2usize).prop_map(Op::ToggleBindFail),
            (0..2usize).prop_map(Op::ToggleMountFail),
            (0..2usize).prop_map(Op::ToggleUnmountFail),
            Just(Op::ToggleSaveFail),
            Just(Op::ToggleExplorer),
        ]
    }

    fn toggle(list: &mut Vec<String>, value: &str) {
        if let Some(position) = list.iter().position(|item| item == value) {
            list.remove(position);
        } else {
            list.push(value.to_string());
        }
    }

    fn run_model_case(ops: &[Op]) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let case = NEXT.fetch_add(1, Ordering::SeqCst);
        let fx = Fixture::new(&format!("model-{case}"));
        let mut manager = fx.manager(Registry::default(), true);

        const LISTENER_ADDRS: [&str; 2] = ["127.0.0.1:4001", "127.0.0.1:4002"];
        const MOUNT_PATHS: [&str; 2] = ["/arena", "/world"];
        let mut desired: Vec<Publication> = Vec::new();
        let mut save_broken = false;

        for op in ops {
            // Apply toggles to the fakes, and compute the model's
            // prediction for manager operations from the documented
            // contract — not from the manager's internals.
            match op {
                Op::ToggleBindFail(index) => {
                    let addr: SocketAddr = LISTENER_ADDRS[*index].parse().unwrap();
                    let mut state = fx.state.lock().unwrap();
                    if let Some(position) = state.fail_addrs.iter().position(|item| *item == addr) {
                        state.fail_addrs.remove(position);
                    } else {
                        state.fail_addrs.push(addr);
                    }
                }
                Op::ToggleMountFail(index) => {
                    toggle(
                        &mut fx.state.lock().unwrap().fail_mounts,
                        MOUNT_PATHS[*index],
                    );
                }
                Op::ToggleUnmountFail(index) => {
                    toggle(
                        &mut fx.state.lock().unwrap().fail_unmounts,
                        MOUNT_PATHS[*index],
                    );
                }
                Op::ToggleSaveFail => {
                    save_broken = !save_broken;
                    // Break saves without touching the registry file
                    // itself: save writes `<file>.tmp` first, and a
                    // directory there makes the write fail.
                    let temp = fx.file.with_extension("json.tmp");
                    if save_broken {
                        std::fs::create_dir_all(&temp).unwrap();
                    } else {
                        let _ = std::fs::remove_dir_all(&temp);
                    }
                }
                Op::ToggleExplorer => {
                    let mut state = fx.state.lock().unwrap();
                    state.explorer = !state.explorer;
                }
                Op::PublishListener(index) => {
                    let publication = listener_pub(LISTENER_ADDRS[*index]);
                    let state = fx.state.lock().unwrap();
                    let predicted = !desired.contains(&publication)
                        && !state
                            .fail_addrs
                            .contains(&LISTENER_ADDRS[*index].parse::<SocketAddr>().unwrap())
                        && !save_broken;
                    drop(state);
                    let result = manager.publish(publication.clone());
                    assert_eq!(result.is_ok(), predicted, "op {op:?}");
                    if predicted {
                        desired.push(publication);
                    }
                }
                Op::PublishMount(index) => {
                    let publication = mount_pub(MOUNT_PATHS[*index], true);
                    let state = fx.state.lock().unwrap();
                    let predicted = !desired.contains(&publication)
                        && state.explorer
                        && !state
                            .fail_mounts
                            .iter()
                            .any(|path| path == MOUNT_PATHS[*index])
                        && !save_broken;
                    drop(state);
                    let result = manager.publish(publication.clone());
                    assert_eq!(result.is_ok(), predicted, "op {op:?}");
                    if predicted {
                        desired.push(publication);
                    }
                }
                Op::UnpublishListener(index) => {
                    let placement = Placement::Listener {
                        addr: LISTENER_ADDRS[*index].parse().unwrap(),
                    };
                    let position = desired
                        .iter()
                        .position(|existing| existing.placement == placement);
                    let predicted = position.is_some() && !save_broken;
                    let result = manager.unpublish(&placement);
                    assert_eq!(result.is_ok(), predicted, "op {op:?}");
                    if predicted {
                        desired.remove(position.unwrap());
                    }
                }
                Op::UnpublishMount(index, spelling) => {
                    let canonical = MOUNT_PATHS[*index];
                    let spelled = match spelling {
                        0 => canonical.to_string(),
                        1 => format!("{canonical}/"),
                        _ => format!("/{canonical}"),
                    };
                    let placement = Placement::PathMount { path: spelled };
                    let position = desired.iter().position(|existing| {
                        existing.placement
                            == Placement::PathMount {
                                path: canonical.to_string(),
                            }
                    });
                    let unmount_fails = fx
                        .state
                        .lock()
                        .unwrap()
                        .fail_unmounts
                        .iter()
                        .any(|path| path == canonical);
                    let predicted = position.is_some() && !save_broken && !unmount_fails;
                    let result = manager.unpublish(&placement);
                    assert_eq!(result.is_ok(), predicted, "op {op:?}");
                    if predicted {
                        desired.remove(position.unwrap());
                    }
                }
            }

            // The invariant, after every operation: the manager's
            // registry, the on-disk registry, and the live sets all
            // equal the model.
            assert_eq!(manager.registry().publications, desired, "after {op:?}");
            assert_eq!(
                Registry::load(&fx.file).unwrap().publications,
                desired,
                "disk after {op:?}"
            );
            let state = fx.state.lock().unwrap();
            let mut live_addrs = state.live_listeners.clone();
            live_addrs.sort();
            let mut model_addrs: Vec<SocketAddr> = desired
                .iter()
                .filter_map(|publication| match &publication.placement {
                    Placement::Listener { addr } => Some(*addr),
                    Placement::PathMount { .. } => None,
                })
                .collect();
            model_addrs.sort();
            assert_eq!(live_addrs, model_addrs, "live listeners after {op:?}");
            let model_mounts: Vec<String> = desired
                .iter()
                .filter_map(|publication| match &publication.placement {
                    Placement::PathMount { path } => Some(path.clone()),
                    Placement::Listener { .. } => None,
                })
                .collect();
            assert_eq!(state.live_mounts, model_mounts, "live mounts after {op:?}");
            assert_eq!(manager.list().len(), desired.len(), "list after {op:?}");
        }
        fx.cleanup();
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn manager_matches_the_reference_model(
            ops in proptest::collection::vec(op_strategy(), 1..14)
        ) {
            run_model_case(&ops);
        }
    }
}
