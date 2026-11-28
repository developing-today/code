//! Client-side coordinator for concurrent world sessions.
//!
//! Split from [`super`] (#16255) to keep `world_net` under the
//! production line limit.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointAddr};

use super::WorldClient;

/// Opaque identity for one independently opened [`WorldSessions`] session.
///
/// Cloning a handle refers to the same session; opening the same world again
/// creates a different handle and stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WorldSessionHandle(u64);

static NEXT_WORLD_SESSION_HANDLE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct CoordinatedSession {
    endpoint_id: String,
    client: WorldClient,
}

/// Client-side coordinator for concurrent world sessions (F13.8, #3845).
///
/// [`WorldClient`] is the transport primitive for one server endpoint: one
/// connection, and sessions as streams on it, each bound to one world for
/// its lifetime. `WorldSessions` is the client-side owner above it: it
/// keeps one root connection per distinct server endpoint and retains an
/// independent opaque handle for every opened session, so a single client
/// can hold multiple sessions to the same world, several worlds on one server,
/// or sessions across servers at once.
///
/// Boundaries (per the F13.8 design record): a connection never spans
/// distinct servers (one root [`WorldClient`] per endpoint); hosting is
/// never started implicitly; every session is opened through the existing
/// capability-bearing `open_session` path, so per-world capability checks
/// are unchanged; the sequential `WorldClient::open_session` and one-shot CLI
/// paths are untouched.
#[derive(Debug)]
pub struct WorldSessions {
    // `pub(super)` so the `world_net` test suite keeps its white-box
    // redial/eviction coverage after the #16255 split-out.
    pub(super) endpoint: Endpoint,
    /// Root connections, keyed by endpoint id.
    pub(super) roots: HashMap<String, WorldClient>,
    /// Live streams, each retained under its own opaque session handle.
    pub(super) sessions: HashMap<WorldSessionHandle, CoordinatedSession>,
}

impl WorldSessions {
    /// A coordinator dialing from `endpoint`.
    #[must_use]
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            roots: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    fn endpoint_id(addr: &EndpointAddr) -> String {
        addr.id.to_string()
    }

    fn allocate_session_handle(&self) -> Result<WorldSessionHandle> {
        let id = NEXT_WORLD_SESSION_HANDLE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| anyhow::anyhow!("world session handle space exhausted"))?;
        Ok(WorldSessionHandle(id))
    }

    /// Open a new session for (`addr`, `world`) and return its independent
    /// handle. Opening the same endpoint/world pair more than once creates a
    /// new stream each time; use the returned handle for all later operations.
    ///
    /// The first session to an endpoint dials its root connection; later
    /// sessions to the same endpoint reuse it via
    /// [`WorldClient::open_session`]. A session to a different endpoint
    /// always dials a separate connection.
    ///
    /// # Errors
    ///
    /// Fails if the endpoint is unreachable, the transport cannot allocate a
    /// stream, or the session-handle space is exhausted. World and hub
    /// refusals are returned on the opened session stream after its first
    /// frame is sent.
    pub async fn open_session(
        &mut self,
        addr: EndpointAddr,
        world: Option<&str>,
    ) -> Result<WorldSessionHandle> {
        let endpoint_id = Self::endpoint_id(&addr);
        // A root whose connection has closed (server disconnect or
        // restart) can never serve another stream, and every session
        // keyed to it rides that dead connection: evict both so this
        // endpoint redials on its own, instead of staying wedged until
        // `close_all` also drops healthy sessions on other servers.
        if self
            .roots
            .get(&endpoint_id)
            .is_some_and(WorldClient::is_closed)
        {
            self.roots.remove(&endpoint_id);
            self.sessions
                .retain(|_, session| session.endpoint_id != endpoint_id);
        }
        if !self.roots.contains_key(&endpoint_id) {
            let root = WorldClient::connect(&self.endpoint, addr).await?;
            self.roots.insert(endpoint_id.clone(), root);
        }
        let session = self
            .roots
            .get(&endpoint_id)
            .context("root inserted above")?
            .open_session(world)
            .await?;
        let handle = self.allocate_session_handle()?;
        self.sessions.insert(
            handle,
            CoordinatedSession {
                endpoint_id,
                client: session,
            },
        );
        Ok(handle)
    }

    /// The live session for `handle`, if it has not been closed or evicted.
    pub fn session(&mut self, handle: WorldSessionHandle) -> Option<&mut WorldClient> {
        self.sessions
            .get_mut(&handle)
            .map(|session| &mut session.client)
    }

    /// Whether `handle` currently names a live session.
    #[must_use]
    pub fn has_session(&self, handle: WorldSessionHandle) -> bool {
        self.sessions.contains_key(&handle)
    }

    /// Number of distinct server endpoints with a root connection.
    #[must_use]
    pub fn endpoint_count(&self) -> usize {
        self.roots.len()
    }

    /// Number of live sessions across all endpoints.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Drop one session by handle. Its stream closes; sibling sessions on the
    /// same connection are unaffected (unlike [`WorldClient::close`], which
    /// closes the whole connection).
    pub fn close_session(&mut self, handle: WorldSessionHandle) -> bool {
        self.sessions.remove(&handle).is_some()
    }

    /// Drop every session and close every root connection.
    pub fn close_all(&mut self) {
        self.sessions.clear();
        for (_, root) in self.roots.drain() {
            root.close();
        }
    }
}
