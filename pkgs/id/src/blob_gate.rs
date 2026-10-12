//! Who may fetch or observe a blob (P3.4, decision 5).
//!
//! iroh-blobs serves a `get` for any hash to any peer that connects. Under its default
//! observe mode it also answers `observe` with the blob's size and held ranges, for anyone.
//! This module decides each of those requests from the requester's [`EndpointId`] and the hash:
//!
//! - **Fetch** (`get`, and every hash of a `get_many`) is allowed only to a current member of a
//!   world that references the hash. A world references a blob when one of its artifacts or
//!   documents names the hash.
//! - **Observe** needs the same membership, and the server policy must serve observe for that
//!   world. Observe is off by default. A world can narrow it off for its own blobs, but cannot
//!   switch it on.
//! - **Refusals look the same.** A hash that no world references and a hash the requester may
//!   not reach get the same verdict. The decision reads only the reference index and the
//!   membership table, never the blob store.
//! - **Fails closed.** A request from a connection with no known `EndpointId` is refused, and so
//!   is one whose verdict cannot be delivered.
//!
//! The reference index and the membership query are behind [`WorldAccess`]. No real store
//! implements it yet, so [`blob_events`] is not wired into `id serve`.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use iroh_base::EndpointId;
use iroh_blobs::{
    Hash,
    provider::events::{
        AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode,
    },
};

use crate::access::AccessPolicy;

/// The two questions the gate asks of the world graph.
pub trait WorldAccess: Send + Sync + 'static {
    /// Whether `requester` is a current member of `world`.
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool;

    /// The worlds whose content names `hash`. Empty for a hash that no world names, which
    /// includes every unknown hash.
    fn worlds_referencing(&self, hash: &Hash) -> Vec<String>;
}

/// The server's blob policy.
///
/// `observe` is the server default and is off unless enabled. A world in `narrowed` refuses
/// observe for its blobs even when the server allows it. Fetch has no switch: every fetch is
/// decided by membership.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlobPolicy {
    observe: bool,
    narrowed: BTreeSet<String>,
}

impl BlobPolicy {
    /// Serve observe for every world that does not narrow it.
    #[must_use]
    pub const fn with_observe(mut self) -> Self {
        self.observe = true;
        self
    }

    /// Refuse observe for the blobs `world` references, whatever the server default.
    #[must_use]
    pub fn narrow_observe(mut self, world: impl Into<String>) -> Self {
        self.narrowed.insert(world.into());
        self
    }

    /// Whether observe may be served for the blobs `world` references.
    pub fn observe_allowed(&self, world: &str) -> bool {
        self.observe && !self.narrowed.contains(world)
    }
}

/// The two request kinds the gate decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// A `get` or one hash of a `get_many`.
    Fetch,
    /// An `observe`.
    Observe,
}

/// Whether `requester` may make `request` for `hash`.
///
/// Allowed when some world that references `hash` has the requester as a current member and,
/// for an observe, the policy serves observe for that world.
pub fn decide(
    access: &impl WorldAccess,
    policy: &BlobPolicy,
    requester: &EndpointId,
    hash: &Hash,
    request: Request,
) -> bool {
    access.worlds_referencing(hash).iter().any(|world| {
        access.is_current_member(world, requester)
            && match request {
                Request::Fetch => true,
                Request::Observe => policy.observe_allowed(world),
            }
    })
}

/// The verdict for a request from `peer`: allowed only when the peer is known and `allow`
/// says so. An unknown peer is refused.
fn admit(
    peer: Option<&EndpointId>,
    allow: impl FnOnce(&EndpointId) -> bool,
) -> Result<(), AbortReason> {
    match peer {
        Some(id) if allow(id) => Ok(()),
        _ => Err(AbortReason::Permission),
    }
}

/// Build the [`EventSender`] that a provider passes to `BlobsProtocol::new`.
///
/// Fetch, `get_many` and observe are decided by [`decide`] over `access`. Pushes are decided by
/// `writes`, as in [`crate::access::blobs_events`]. Every request is intercepted, so no store
/// read happens before the verdict.
///
/// A background task is spawned on the current Tokio runtime and ends when the sender is
/// dropped.
pub fn blob_events<A: WorldAccess>(
    access: Arc<A>,
    policy: BlobPolicy,
    writes: AccessPolicy,
) -> EventSender {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderMessage>(64);
    let mask = EventMask {
        connected: ConnectMode::Notify,
        get: RequestMode::Intercept,
        get_many: RequestMode::Intercept,
        push: RequestMode::Intercept,
        observe: ObserveMode::Intercept,
        ..EventMask::DEFAULT
    };
    tokio::spawn(async move {
        let mut peers: HashMap<u64, EndpointId> = HashMap::new();
        while let Some(msg) = rx.recv().await {
            match msg {
                ProviderMessage::ClientConnectedNotify(m) => {
                    if let Some(id) = m.endpoint_id {
                        peers.insert(m.connection_id, id);
                    }
                }
                ProviderMessage::ConnectionClosed(m) => {
                    peers.remove(&m.connection_id);
                }
                ProviderMessage::GetRequestReceived(m) => {
                    let verdict = admit(peers.get(&m.connection_id), |id| {
                        decide(&*access, &policy, id, &m.request.hash, Request::Fetch)
                    });
                    let _ = m.tx.send(verdict).await;
                }
                ProviderMessage::GetManyRequestReceived(m) => {
                    let verdict = admit(peers.get(&m.connection_id), |id| {
                        m.request
                            .hashes
                            .iter()
                            .all(|hash| decide(&*access, &policy, id, hash, Request::Fetch))
                    });
                    let _ = m.tx.send(verdict).await;
                }
                ProviderMessage::ObserveRequestReceived(m) => {
                    let verdict = admit(peers.get(&m.connection_id), |id| {
                        decide(&*access, &policy, id, &m.request.hash, Request::Observe)
                    });
                    let _ = m.tx.send(verdict).await;
                }
                ProviderMessage::PushRequestReceived(m) => {
                    let verdict = admit(peers.get(&m.connection_id), |id| writes.can_write(id));
                    let _ = m.tx.send(verdict).await;
                }
                _ => {}
            }
        }
    });
    EventSender::new(tx, mask)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use iroh_base::SecretKey;
    use proptest::prelude::*;

    use super::*;

    const WORLDS: [&str; 3] = ["alpha", "beta", "gamma"];

    fn id(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    fn blob(n: u8) -> Hash {
        Hash::from_bytes([n; 32])
    }

    /// Membership and references as plain lists, read by both `decide` and [`spec`].
    #[derive(Debug, Default)]
    struct Table {
        members: Vec<(String, EndpointId)>,
        refs: Vec<(Hash, String)>,
    }

    impl WorldAccess for Table {
        fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
            self.members
                .iter()
                .any(|(w, m)| w == world && m == requester)
        }

        fn worlds_referencing(&self, hash: &Hash) -> Vec<String> {
            self.refs
                .iter()
                .filter(|(h, _)| h == hash)
                .map(|(_, w)| w.clone())
                .collect()
        }
    }

    /// The rule as a scan of the lists: some world names the hash, the requester is a member
    /// of it, and an observe is served only where the policy serves observe.
    fn spec(t: &Table, p: &BlobPolicy, r: &EndpointId, h: &Hash, req: Request) -> bool {
        t.refs.iter().any(|(rh, w)| {
            rh == h
                && t.members.iter().any(|(mw, m)| mw == w && m == r)
                && (req == Request::Fetch || (p.observe && !p.narrowed.contains(w)))
        })
    }

    fn one_world(member: EndpointId, hash: Hash) -> Table {
        Table {
            members: vec![("docs".to_owned(), member)],
            refs: vec![(hash, "docs".to_owned())],
        }
    }

    #[test]
    fn a_non_member_is_refused_fetch_and_observe() {
        let t = one_world(id(1), blob(9));
        let p = BlobPolicy::default().with_observe();
        assert!(!decide(&t, &p, &id(2), &blob(9), Request::Fetch));
        assert!(!decide(&t, &p, &id(2), &blob(9), Request::Observe));
    }

    #[test]
    fn a_member_of_the_referencing_world_fetches() {
        let t = one_world(id(1), blob(9));
        assert!(decide(
            &t,
            &BlobPolicy::default(),
            &id(1),
            &blob(9),
            Request::Fetch
        ));
    }

    #[test]
    fn a_member_of_an_unrelated_world_is_refused() {
        let t = Table {
            members: vec![("other".to_owned(), id(1))],
            refs: vec![(blob(9), "docs".to_owned())],
        };
        let p = BlobPolicy::default().with_observe();
        assert!(!decide(&t, &p, &id(1), &blob(9), Request::Fetch));
        assert!(!decide(&t, &p, &id(1), &blob(9), Request::Observe));
    }

    #[test]
    fn observe_is_refused_by_default_even_for_a_member() {
        let t = one_world(id(1), blob(9));
        assert!(!BlobPolicy::default().observe_allowed("docs"));
        assert!(!decide(
            &t,
            &BlobPolicy::default(),
            &id(1),
            &blob(9),
            Request::Observe
        ));
        assert!(decide(
            &t,
            &BlobPolicy::default(),
            &id(1),
            &blob(9),
            Request::Fetch
        ));
    }

    #[test]
    fn observe_is_allowed_for_a_member_when_the_server_enables_it() {
        let t = one_world(id(1), blob(9));
        let p = BlobPolicy::default().with_observe();
        assert!(decide(&t, &p, &id(1), &blob(9), Request::Observe));
    }

    #[test]
    fn a_world_can_narrow_observe_off_while_the_server_allows_it() {
        let t = one_world(id(1), blob(9));
        let p = BlobPolicy::default().with_observe().narrow_observe("docs");
        assert!(!decide(&t, &p, &id(1), &blob(9), Request::Observe));
        assert!(decide(&t, &p, &id(1), &blob(9), Request::Fetch));
    }

    #[test]
    fn a_narrowed_world_does_not_hide_observe_from_another_referencing_world() {
        let t = Table {
            members: vec![("docs".to_owned(), id(1)), ("team".to_owned(), id(1))],
            refs: vec![(blob(9), "docs".to_owned()), (blob(9), "team".to_owned())],
        };
        let p = BlobPolicy::default().with_observe().narrow_observe("docs");
        assert!(decide(&t, &p, &id(1), &blob(9), Request::Observe));
    }

    #[test]
    fn an_unknown_hash_is_refused_like_a_denied_one() {
        let t = one_world(id(1), blob(9));
        let p = BlobPolicy::default().with_observe();
        for request in [Request::Fetch, Request::Observe] {
            // Unknown: no world names blob(5). Denied: a stranger asks for blob(9).
            let unknown = decide(&t, &p, &id(1), &blob(5), request);
            let denied = decide(&t, &p, &id(2), &blob(9), request);
            assert!(!unknown && !denied, "{request:?}");
        }
    }

    proptest! {
        #[test]
        fn decide_matches_the_spec_for_every_pair(
            members in proptest::collection::vec((0usize..3, 1u8..6), 0..12),
            refs in proptest::collection::vec((1u8..6, 0usize..3), 0..12),
            observe in any::<bool>(),
            narrowed in proptest::collection::vec(0usize..3, 0..3),
        ) {
            let t = Table {
                members: members
                    .iter()
                    .map(|&(w, k)| (WORLDS[w].to_owned(), id(k)))
                    .collect(),
                refs: refs
                    .iter()
                    .map(|&(h, w)| (blob(h), WORLDS[w].to_owned()))
                    .collect(),
            };
            let mut p = BlobPolicy::default();
            if observe {
                p = p.with_observe();
            }
            for &w in &narrowed {
                p = p.narrow_observe(WORLDS[w]);
            }
            for requester in 1u8..=6 {
                for hash in 1u8..=5 {
                    for request in [Request::Fetch, Request::Observe] {
                        let got = decide(&t, &p, &id(requester), &blob(hash), request);
                        let want = spec(&t, &p, &id(requester), &blob(hash), request);
                        prop_assert_eq!(
                            got,
                            want,
                            "requester {} hash {} {:?}",
                            requester,
                            hash,
                            request
                        );
                    }
                }
            }
        }
    }
}
