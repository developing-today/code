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

use std::net::SocketAddr;

use anyhow::{Context, Result};

use crate::publications_runtime::{ListenerFactory, LiveListener, MountSink};

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
        self.mounts.mount(world, path);
        Ok(())
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
    fn sink_without_explorer_reports_none() {
        let mut sink = ServeMountSink::new(id_web::PublicationMounts::new(), false);
        assert!(!sink.has_explorer());
        // Mounting itself is a table edit; the manager is what
        // refuses it when has_explorer() is false.
        sink.mount("arena", "/prog").unwrap();
    }
}
