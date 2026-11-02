//! `/id-range-sync/1` sessions for the node's own event store, as served by `id serve` (#792).
//!
//! The ALPN is [`crate::event_store::sync::ALPN`], split from `log_sync`'s `/id-sync/1`
//! in #179: these sessions open with a range flight, not the handshake's `Request`
//! first frame, and the two shapes no longer share an ALPN.
//!
//! The session itself is the range sync in [`crate::event_store::sync`]: reconciliation,
//! then both sides send the events the other lacks, and the receiving store's own ingest
//! verifies and decides each one — a tampered or unsigned event is refused by
//! [`crate::event_store::EventStore::ingest`], never applied. This module is only the
//! iroh plumbing around it: the accepting protocol handler, the dialling helper, and
//! the pull gate `serve` uses.
//!
//! The store these sessions serve is the *same* [`LocalEvents`] instance the web layer
//! publishes ProseMirror save revisions into. `serve` opens that store once and shares
//! the `Arc<Mutex<LocalEvents>>` between the web state and this handler, so there is no
//! second, divergent store: a revision saved through the web UI is in the sync set of
//! the next session, and a revision that arrives by sync is in the history the web UI
//! reads. Sharing the instance (rather than opening the same directory twice) is also
//! the only option: the store's database admits one opener at a time.
//!
//! The pull gate is [`PullPolicy::Open`], the documented default for sync: `id serve`
//! does not read a per-world pull policy from config yet, and there is no world whose
//! membership this store belongs to. An open gate never consults membership, so the
//! [`WorldAccess`] it carries is a formality that is never read.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use iroh_base::EndpointId;
use tokio::sync::Mutex;

use crate::blob_gate::WorldAccess;
use crate::event_store::sync::{ALPN, Outcome};
use crate::local_events::LocalEvents;
use crate::pull_policy::{PullGate, PullPolicy};

/// How long one sync session may run before it is abandoned. Both sides use it, so a
/// silent peer ends the session instead of holding the shared store.
pub const SESSION_DEADLINE: Duration = Duration::from_secs(60);

/// How often `serve` offers a sync session to each peer it has discovered.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(30);

/// How long the accepting side waits, after a completed session, for the dialer
/// to close the connection before it lets the router close it instead (see
/// `EventSyncProtocol::accept`).
pub(crate) const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Membership answers for a gate that never asks. [`PullPolicy::Open`] admits every
/// peer without reading membership (see the module docs), so these answers are never
/// used; the type exists only because a [`PullGate`] is built over a [`WorldAccess`].
#[derive(Debug)]
struct NoWorlds;

impl WorldAccess for NoWorlds {
    fn is_current_member(&self, _world: &str, _requester: &EndpointId) -> bool {
        false
    }

    fn worlds_referencing(&self, _hash: &iroh_blobs::Hash) -> Vec<String> {
        Vec::new()
    }
}

/// The pull gate for the node's own event store: open, per the module docs.
#[must_use]
pub fn open_gate() -> PullGate {
    PullGate::new(PullPolicy::Open, "local-events", Arc::new(NoWorlds))
}

/// Serves range-sync sessions over the node's shared [`LocalEvents`] store.
/// Register it with `Router::builder(..).accept(ALPN, EventSyncProtocol::new(events))`,
/// where `events` is the same instance the web layer publishes into.
#[derive(Clone)]
pub struct EventSyncProtocol {
    events: Arc<Mutex<LocalEvents>>,
    gate: PullGate,
}

impl std::fmt::Debug for EventSyncProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventSyncProtocol")
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}

impl EventSyncProtocol {
    /// Serves `events` under an open pull gate.
    #[must_use]
    pub fn new(events: Arc<Mutex<LocalEvents>>) -> Self {
        Self {
            events,
            gate: open_gate(),
        }
    }
}

impl ProtocolHandler for EventSyncProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = conn.remote_id();
        // The lock is held for the whole session: the store takes `&mut`, and the web
        // layer's saves queue behind a session the way they queue behind each other.
        // Sessions are bounded by SESSION_DEADLINE, so the wait is bounded too.
        let mut events = self.events.lock().await;
        let completed = match events
            .sync_session(&conn, false, SESSION_DEADLINE, &self.gate)
            .await
        {
            Ok(outcome) => {
                tracing::debug!(
                    "event sync: session with {peer}: sent {} received {} accepted {} duplicates {} waiting {} refused {}",
                    outcome.sent,
                    outcome.received,
                    outcome.accepted,
                    outcome.duplicates,
                    outcome.waiting,
                    outcome.refused
                );
                true
            }
            Err(error) => {
                tracing::debug!("event sync: session with {peer} ended: {error:#}");
                false
            }
        };
        if completed {
            // The session's last frame is the completion frame the dialer ends
            // its session on. Returning now would let the router close the
            // connection before the dialer has read it, discarding the frame
            // and failing its otherwise-complete session — so, like
            // `ArtifactPullProtocol`, wait for the dialer to close first. The
            // wait is bounded: the store lock is held here, and a dialer that
            // never closes must not hold it (or this handler) forever.
            let _ = tokio::time::timeout(CLOSE_WAIT, conn.closed()).await;
        }
        Ok(())
    }
}

/// Dials `peer` and runs one range-sync session over the shared store, as initiator.
///
/// # Errors
///
/// Returns an error if the peer cannot be reached, breaks the protocol, or the session
/// does not finish within [`SESSION_DEADLINE`].
pub async fn sync_with_peer(
    endpoint: &Endpoint,
    peer: impl Into<EndpointAddr>,
    events: &Arc<Mutex<LocalEvents>>,
    gate: &PullGate,
) -> Result<Outcome> {
    let conn = crate::connectivity::connect(endpoint, peer, ALPN).await?;
    let outcome = {
        let mut events = events.lock().await;
        events
            .sync_session(&conn, true, SESSION_DEADLINE, gate)
            .await
    };
    conn.close(0_u32.into(), b"done");
    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};
    use std::path::Path;

    use iroh::endpoint::{RelayMode, presets};
    use iroh::protocol::Router;
    use p2panda_core::SigningKey;

    use super::*;

    fn node(dir: &Path, byte: u8) -> Arc<Mutex<LocalEvents>> {
        let events = LocalEvents::open(dir, SigningKey::from_bytes(&[byte; 32])).unwrap();
        Arc::new(Mutex::new(events))
    }

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    /// `ep`'s address on its loopback sockets, so a peer dials it without a relay or
    /// discovery.
    fn loopback(ep: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                iroh::TransportAddr::Ip(SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    /// Waits until `events` can read `id`'s history. A session's return on the
    /// dialling side does not imply the accepting side has finished applying what
    /// it received (its handler runs as its own task), so tests wait for the
    /// applied state itself — the same convergence serve reaches on its next
    /// interval.
    async fn wait_for_history(events: &Arc<Mutex<LocalEvents>>, id: p2panda_core::Hash) {
        for _ in 0..1200 {
            if events.lock().await.history(id).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the accepting side never applied {id}");
    }

    /// #179: the range sessions served here must not share an ALPN with the
    /// `log_sync` handshake sessions, whose first frame is a `Request` — a range
    /// session opens with a range flight, and the two shapes misread each other.
    #[test]
    fn test_range_alpn_is_distinct_from_handshake_alpn() {
        assert_ne!(ALPN, crate::log_sync::SYNC_ALPN);
        assert_eq!(ALPN, b"/id-range-sync/1");
        assert_eq!(crate::log_sync::SYNC_ALPN, b"/id-sync/1");
    }

    /// The #792 acceptance test: a document revision published (saved) on node A is
    /// discoverable from node B's history after one sync session, where B serves the
    /// protocol exactly as `serve` registers it and A dials with the production helper.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_a_save_on_one_node_is_discoverable_on_the_other_after_sync() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = node(dir_a.path(), 1);
        let b = node(dir_b.path(), 2);

        // A saves a document: the web layer's publish into its LocalEvents.
        let published = {
            let mut a = a.lock().await;
            a.publish("text/plain", b"hello from A", &[]).unwrap()
        };
        // B has never seen it.
        assert!(b.lock().await.history(published.id).is_err());

        let server = endpoint().await;
        let router = Router::builder(server.clone())
            .accept(ALPN, EventSyncProtocol::new(b.clone()))
            .spawn();
        let client = endpoint().await;
        let outcome = sync_with_peer(&client, loopback(&server), &a, &open_gate())
            .await
            .unwrap();
        assert!(
            outcome.sent >= 1,
            "A sent nothing in the session: {outcome:?}"
        );

        // B discovers the signed revision in the document's history.
        wait_for_history(&b, published.id).await;
        let history = b.lock().await.history(published.id).unwrap();
        assert!(
            history.iter().any(|entry| entry.id() == published.id),
            "B's history does not hold A's revision: {history:?}"
        );

        router.shutdown().await.unwrap();
        client.close().await;
        server.close().await;
    }

    /// A second session between two stores that already agree sends nothing either
    /// way: what arrived by sync is not sent back as new.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_a_second_session_between_equal_stores_sends_nothing() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = node(dir_a.path(), 3);
        let b = node(dir_b.path(), 4);
        let published = {
            let mut a = a.lock().await;
            a.publish("text/plain", b"one revision", &[]).unwrap()
        };

        let server = endpoint().await;
        let router = Router::builder(server.clone())
            .accept(ALPN, EventSyncProtocol::new(b.clone()))
            .spawn();
        let client = endpoint().await;
        sync_with_peer(&client, loopback(&server), &a, &open_gate())
            .await
            .unwrap();
        wait_for_history(&b, published.id).await;
        let second = sync_with_peer(&client, loopback(&server), &a, &open_gate())
            .await
            .unwrap();
        assert_eq!(second.sent, 0, "the second session re-sent events");
        assert_eq!(second.received, 0, "the second session re-received events");

        router.shutdown().await.unwrap();
        client.close().await;
        server.close().await;
    }
}
