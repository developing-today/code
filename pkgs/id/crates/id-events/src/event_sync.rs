//! `/id-range-sync/1` sessions for the node's own event store, as served by `id serve` (#792).
//!
//! The ALPN is [`crate::event_store::sync::ALPN`], split from `log_sync`'s `/id-sync/1`
//! in #179: these sessions open with a range flight, not the handshake's `Request`
//! first frame, and the two shapes no longer share an ALPN.
//! `serve` always uses range sessions on this ALPN, without `choose()` or the topic-scoped
//! handshake. Their sync set is the whole store, not a topic subset.
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
//! The pull gate is built from the operator server config's `[policy] pull_policy`
//! ([`crate::server_config::ServerConfig::pull_policy`]) when `id serve` has one, and
//! is [`PullPolicy::Open`] otherwise — so with no `--server-config` behaviour is
//! exactly as before. This store is not a world and has no membership source: under
//! [`PullPolicy::Members`] [`gate`] therefore refuses every pull (fail closed, see
//! [`NoWorlds`]) rather than silently admitting peers. That is the explicit safe
//! behaviour for the no-world case; a membership source can replace it when the
//! store grows one.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use iroh_base::EndpointId;
use tokio::sync::Mutex;

use crate::event_store::sync::{ALPN, Outcome};
use crate::local_events::LocalEvents;
use crate::pull_policy::{PullGate, PullPolicy};
use crate::residency_sync::SyncResidency;
use crate::world_access::WorldAccess;

/// How long one sync session may run before it is abandoned. Both sides use it, so a
/// silent peer ends the session instead of holding the shared store.
pub const SESSION_DEADLINE: Duration = Duration::from_secs(60);

/// How often `serve` offers a sync session to each peer it has discovered.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(30);

/// How long the accepting side waits, after a completed session, for the dialer
/// to close the connection before it lets the router close it instead (see
/// `EventSyncProtocol::accept`).
/// How long a closing session waits for its peer. `pub` since step
/// 2b: the facade's artifact serving names it, as it did when both
/// lived in one crate.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Membership answers for this store, which is not a world and has no membership
/// source. [`PullPolicy::Open`] admits every peer without reading membership, so
/// under the default policy these answers are never used. Under
/// [`PullPolicy::Members`] they are read, and every answer is `false`: the no-world
/// case fails closed and refuses every pull instead of silently serving open.
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

/// The pull gate for the node's own event store under the configured `policy`.
///
/// [`PullPolicy::Open`] (the default when there is no server config) admits every
/// peer. [`PullPolicy::Members`] reads [`NoWorlds`], which answers `false` for every
/// peer because this store has no membership source: the configured members policy
/// refuses every pull rather than silently serving open (see the module docs).
#[must_use]
pub fn gate(policy: PullPolicy) -> PullGate {
    PullGate::new(policy, "local-events", Arc::new(NoWorlds))
}

/// The pull gate for the node's own event store when no policy is configured: open,
/// per the module docs.
#[must_use]
pub fn open_gate() -> PullGate {
    gate(PullPolicy::Open)
}

/// Serves range-sync sessions over the node's shared [`LocalEvents`] store.
/// Register it with `Router::builder(..).accept(ALPN, EventSyncProtocol::new(events))`,
/// where `events` is the same instance the web layer publishes into.
#[derive(Clone)]
pub struct EventSyncProtocol {
    events: Arc<Mutex<LocalEvents>>,
    gate: PullGate,
    /// Residency gate (#5529 step 3): when present, a session is
    /// refused before the range flight unless the gate admits it.
    residency: Option<Arc<SyncResidency>>,
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
            residency: None,
        }
    }

    /// Serves `events` under `gate` — the policy from the operator server config,
    /// applied to accepted sessions as well as dialled ones (#5119).
    #[must_use]
    pub fn with_gate(events: Arc<Mutex<LocalEvents>>, gate: PullGate) -> Self {
        Self {
            events,
            gate,
            residency: None,
        }
    }

    /// Serves `events` under `gate`, with the residency gate of
    /// #5529 step 3 applied before every accepted session: a scoped
    /// session whose store/transport evidence is missing, expired,
    /// revoked, or out of policy is refused before any event bytes
    /// are ingested, whatever the pull gate or the peer's join
    /// capability would have said.
    #[must_use]
    pub fn with_residency(
        events: Arc<Mutex<LocalEvents>>,
        gate: PullGate,
        residency: Arc<SyncResidency>,
    ) -> Self {
        Self {
            events,
            gate,
            residency: Some(residency),
        }
    }
}

impl ProtocolHandler for EventSyncProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = conn.remote_id();
        // Residency (#5529 step 3): refuse before the range flight,
        // so a denied session ingests nothing. The check runs before
        // the store lock is taken; a refusal holds nothing.
        if let Some(residency) = &self.residency {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let decision = residency.admit_session(now);
            if !decision.is_allow() {
                tracing::debug!(
                    "event sync: session with {peer} refused by residency gate: {decision:?}"
                );
                return Ok(());
            }
        }
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
    sync_with_peer_checked(endpoint, peer, events, gate, None).await
}

/// Dials `peer` and runs one range-sync session, applying the
/// residency gate of #5529 step 3 first when one is configured: the
/// send-side preflight runs before the connection is opened, so a
/// denial sends zero protected events (the connection is never made).
///
/// # Errors
///
/// Returns an error if the residency gate denies the session, the
/// peer cannot be reached, breaks the protocol, or the session does
/// not finish within [`SESSION_DEADLINE`].
pub async fn sync_with_peer_checked(
    endpoint: &Endpoint,
    peer: impl Into<EndpointAddr>,
    events: &Arc<Mutex<LocalEvents>>,
    gate: &PullGate,
    residency: Option<&SyncResidency>,
) -> Result<Outcome> {
    if let Some(residency) = residency {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let decision = residency.admit_session(now);
        if !decision.is_allow() {
            anyhow::bail!("residency gate refused sync session: {decision:?}");
        }
    }
    let conn = id_core::connectivity::connect(endpoint, peer, ALPN).await?;
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
        // The handshake ALPN is the facade's `log_sync::SYNC_ALPN`
        // (`/id-sync/1`); it stays a literal here since step 2b —
        // `log_sync` is above this crate, and the value is the wire
        // constant the distinctness rule is about.
        assert_ne!(ALPN, b"/id-sync/1");
        assert_eq!(ALPN, b"/id-range-sync/1");
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

    /// #5119: under a configured `members` pull policy this store has no
    /// membership source, so the gate refuses every pull — fail closed, never
    /// silently open. The accepting side refuses to send, and the dialling side
    /// refuses too, so nothing crosses either way.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_members_gate_refuses_every_pull_when_the_store_has_no_membership_source() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let a = node(dir_a.path(), 5);
        let b = node(dir_b.path(), 6);
        let published = {
            let mut a = a.lock().await;
            a.publish("text/plain", b"never served", &[]).unwrap()
        };

        let server = endpoint().await;
        let router = Router::builder(server.clone())
            .accept(
                ALPN,
                EventSyncProtocol::with_gate(b.clone(), gate(PullPolicy::Members)),
            )
            .spawn();
        let client = endpoint().await;
        // A refused pull ends the session with `PullRefused` (the sync layer's
        // documented contract: a refused peer gets no event, and the session
        // reports the refusal rather than a successful zero transfer - serve
        // logs the error and carries on). Nothing may move in either direction.
        let error = sync_with_peer(&client, loopback(&server), &a, &gate(PullPolicy::Members))
            .await
            .expect_err("a members gate with no membership source completed a session");
        assert!(
            error.to_string().contains("pull policy refuses"),
            "the session failed for a reason other than the gate: {error:#}"
        );
        assert!(
            b.lock().await.history(published.id).is_err(),
            "B received an event under a refusing members gate"
        );

        router.shutdown().await.unwrap();
        client.close().await;
        server.close().await;
    }
}
