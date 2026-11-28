//! Serve-side adapters for dynamic publications (#2147 slice 3):
//! the real [`ListenerFactory`] / [`MountSink`] the serve-owned
//! [`PublicationManager`](crate::publications_runtime::PublicationManager)
//! drives.
//!
//! - The factory binds a real TCP listener and serves the published
//!   world's program-only router (built by
//!   [`id_web::WebSurfaceFactory`] over the serve process's one
//!   shared web state) on a spawned task, with the per-bind
//!   [`id_web::WebSecurity`] a configured listener for the same
//!   address would get. Binding is synchronous at the socket level
//!   (a staged listener exists before the manager persists the
//!   publication, per the design's apply ordering); only serving is
//!   async, on the captured runtime handle.
//! - The sink applies path mounts to the primary Explorer listener
//!   by editing the shared [`id_web::PublicationMounts`] table the
//!   Explorer routers consult per request, so a mount takes effect
//!   -- and reverts -- with no router rebuild.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use crate::protocol::{
    PublicationControl, PublicationControlHandle, PublicationOp, PublicationOutcome, WirePlacement,
    WirePublicationStatus,
};
use crate::publications::{Placement, Publication};
use crate::publications_runtime::{ListenerFactory, LiveListener, MountSink, PublicationStatus};

/// The publication manager as `cmd_serve` instantiates it.
pub type ServePublicationManager =
    crate::publications_runtime::PublicationManager<ServeListenerFactory, ServeMountSink>;

/// Opens live listeners for listener placements, serving the
/// published world's program surface on each.
pub struct ServeListenerFactory {
    handle: tokio::runtime::Handle,
    surface: id_web::WebSurfaceFactory,
    web_token: Option<String>,
}

impl ServeListenerFactory {
    /// A factory serving through `surface` (the serve process's
    /// shared web state), guarding each dynamic listener with the
    /// serve's web token policy for its bind address.
    pub fn new(
        handle: tokio::runtime::Handle,
        surface: id_web::WebSurfaceFactory,
        web_token: Option<String>,
    ) -> Self {
        Self {
            handle,
            surface,
            web_token,
        }
    }
}

/// One live publication listener: a spawned `axum::serve` task.
struct ServeLiveListener {
    actual_addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl LiveListener for ServeLiveListener {
    fn actual_addr(&self) -> SocketAddr {
        self.actual_addr
    }

    fn close(self: Box<Self>) {
        self.task.abort();
    }
}

impl ListenerFactory for ServeListenerFactory {
    fn bind(&mut self, world: &str, addr: SocketAddr) -> Result<Box<dyn LiveListener>> {
        let std_listener = std::net::TcpListener::bind(addr).with_context(|| {
            format!("publication: failed to bind listener {addr} for world '{world}'")
        })?;
        std_listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(std_listener)?;
        let actual_addr = listener.local_addr()?;
        // The same per-bind security a configured listener for this
        // address receives in cmd_serve (#2145): token policy keyed
        // to the bind IP, no extra grants.
        let security = id_web::WebSecurity::for_bind(addr.ip(), self.web_token.clone(), &[]);
        let router = self.surface.program_router(world, security);
        let task = self.handle.spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::error!("publication listener error: {error}");
            }
        });
        Ok(Box::new(ServeLiveListener { actual_addr, task }))
    }
}

/// Applies path mounts to the primary Explorer listener through
/// the shared mount table.
pub struct ServeMountSink {
    mounts: id_web::PublicationMounts,
    has_explorer: bool,
}

impl ServeMountSink {
    /// A sink over the serve process's shared `mounts` table;
    /// `has_explorer` records whether any configured listener
    /// serves the Explorer surface (path mounts target the first
    /// effective Explorer binding; with none, the manager refuses
    /// the publish before persisting anything).
    pub fn new(mounts: id_web::PublicationMounts, has_explorer: bool) -> Self {
        Self {
            mounts,
            has_explorer,
        }
    }
}

impl MountSink for ServeMountSink {
    fn has_explorer(&self) -> bool {
        self.has_explorer
    }

    fn mount(&mut self, world: &str, path: &str) -> Result<()> {
        // PublicationMounts::mount validates the path and refuses
        // reserved-route shadows and different-world conflicts
        // (#2146 phase 4); the error propagates so the manager
        // reverts the persisted registry entry it had already
        // saved away, keeping the durable and live sets in agreement.
        self.mounts.mount(world, path)
    }

    fn unmount(&mut self, path: &str) -> Result<()> {
        // PublicationMounts::unmount errors when nothing is mounted
        // there, which is exactly this trait's contract: an error
        // means the route was NOT removed.
        self.mounts.unmount(path)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn sink_mounts_and_unmounts_through_the_shared_table() {
        let mounts = id_web::PublicationMounts::new();
        let mut sink = ServeMountSink::new(mounts.clone(), true);
        assert!(sink.has_explorer());
        sink.mount("arena", "/prog").unwrap();
        assert_eq!(
            mounts.lookup("/prog"),
            Some(("/prog".to_owned(), "arena".to_owned()))
        );
        sink.unmount("/prog").unwrap();
        assert_eq!(mounts.lookup("/prog"), None);
        // Contract: unmounting an absent mount errors.
        assert!(sink.unmount("/prog").is_err());
    }

    #[test]
    fn sink_propagates_table_validation_and_conflicts() {
        let mounts = id_web::PublicationMounts::new();
        let mut sink = ServeMountSink::new(mounts.clone(), true);
        // Non-canonical paths: refused by the table, nothing mounted.
        assert!(sink.mount("arena", "prog").is_err());
        assert!(sink.mount("arena", "/prog/").is_err());
        // A reserved Explorer route: refused, not silently shadowed.
        assert!(sink.mount("arena", "/world").is_err());
        assert_eq!(mounts.lookup("/world"), None);
        // A different world at a taken path: refused, original kept.
        sink.mount("arena", "/prog").unwrap();
        assert!(sink.mount("deep", "/prog").is_err());
        assert_eq!(
            mounts.lookup("/prog"),
            Some(("/prog".to_owned(), "arena".to_owned()))
        );
    }

    #[test]
    fn sink_without_explorer_reports_none() {
        let mut sink = ServeMountSink::new(id_web::PublicationMounts::new(), false);
        assert!(!sink.has_explorer());
        // Mounting itself is a table edit; the manager is what
        // refuses it when has_explorer() is false.
        sink.mount("arena", "/prog").unwrap();
    }
}

/// The meta-protocol publication control over the serve-owned
/// manager (#2147 slice 3b): the bridge the protocol handler's
/// operator gate dispatches to. One control per serve process,
/// registered with the handler once the manager exists.
pub struct ServePublicationControl {
    manager: Arc<Mutex<ServePublicationManager>>,
}

impl ServePublicationControl {
    /// A control over the serve process's shared manager.
    pub fn new(manager: Arc<Mutex<ServePublicationManager>>) -> Self {
        Self { manager }
    }

    /// This control as the handle the protocol handler stores.
    #[must_use]
    pub fn handle(self) -> PublicationControlHandle {
        PublicationControlHandle::new(Arc::new(self))
    }
}

impl PublicationControl for ServePublicationControl {
    fn call(
        &self,
        op: PublicationOp,
    ) -> Pin<Box<dyn Future<Output = Result<PublicationOutcome, String>> + Send + '_>> {
        Box::pin(async move {
            // The manager's operations are synchronous and short
            // (validate, bind a socket, save a small JSON registry,
            // edit the mount table), so they run under the lock
            // directly: the lock is also what serializes operator
            // requests against each other, as the design requires
            // of the serve-owned manager.
            let mut manager = self
                .manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match op {
                PublicationOp::Publish(wire) => {
                    let warning = manager
                        .publish(publication_from_wire(wire))
                        .map_err(|error| format!("{error:#}"))?;
                    Ok(PublicationOutcome::Published { warning })
                }
                PublicationOp::Unpublish(wire) => {
                    manager
                        .unpublish(&placement_from_wire(&wire))
                        .map_err(|error| format!("{error:#}"))?;
                    Ok(PublicationOutcome::Unpublished)
                }
                PublicationOp::List => Ok(PublicationOutcome::Listed {
                    items: manager.list().into_iter().map(status_to_wire).collect(),
                }),
            }
        })
    }
}

fn publication_from_wire(wire: crate::protocol::WirePublication) -> Publication {
    Publication {
        world: wire.world,
        placement: placement_from_wire(&wire.placement),
        acknowledge_shadow: wire.acknowledge_shadow,
    }
}

fn placement_from_wire(wire: &WirePlacement) -> Placement {
    match wire {
        WirePlacement::Listener { addr } => Placement::Listener { addr: *addr },
        WirePlacement::PathMount { path } => Placement::PathMount { path: path.clone() },
    }
}

fn placement_to_wire(placement: &Placement) -> WirePlacement {
    match placement {
        Placement::Listener { addr } => WirePlacement::Listener { addr: *addr },
        Placement::PathMount { path } => WirePlacement::PathMount { path: path.clone() },
    }
}

fn status_to_wire(status: PublicationStatus) -> WirePublicationStatus {
    WirePublicationStatus {
        world: status.publication.world.clone(),
        placement: placement_to_wire(&status.publication.placement),
        acknowledge_shadow: status.publication.acknowledge_shadow,
        actual_addr: status.actual_addr,
    }
}
