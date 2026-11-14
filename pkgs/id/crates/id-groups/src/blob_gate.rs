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
//! - **Local reads are the operator's.** The node's own key and its data directory's client
//!   key — the two keys `build_access_policy` already trusts automatically for writes — may
//!   fetch and observe any blob in the node's own store (see [`decide_with_local`]). The
//!   local CLI and REPL connect with the client key, and locally published content
//!   (documents with no group, plain uploads) is referenced by no world; without this the
//!   gate would lock the operator out of their own store. Local trust extends to no other
//!   peer: a remote requester still needs world membership for every hash.
//!
//! The reference index and the membership query are behind [`WorldAccess`]. The facade's
//! `blob_gate::FoldedWorlds` answers membership from the folded worlds. The index has a
//! producer now: [`build_reference_index`] over the event store's document references,
//! with the worlds crate's `fold_group_events` folding the group log for membership, and
//! `RefreshingWorlds` swapping fresh folds in as they change. `id serve` wires
//! [`blob_events`] with those pieces whenever it has the node's event store open.

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

use id_core::access::AccessPolicy;
use id_events::event_store::{BlockList, BlockLookup};

/// The two questions the gate asks of the world graph. The trait is
/// sync-gating vocabulary owned by the events layer since step 2b of
/// the crate split (#4405); it is re-exported here at its historical
/// path. The implementation over the folded worlds (`FoldedWorlds`)
/// stays in the facade with `groups::worlds` until step 5.
pub use id_events::world_access::WorldAccess;

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
    decide_with_blocks(access, policy, None, requester, hash, request)
}

/// [`decide`], with the operator block list composed in (#2245): a
/// requester blocked by endpoint key is refused fetch and observe alike,
/// member or not. The seam's shape stays the crate's bool verdict (the
/// provider's `AbortReason::Permission` is what a refusal becomes in
/// [`blob_events_with_blocks`]). Pushes are not decided here: a block
/// changes serving only, never what the store ingests.
///
/// `blocks` is `None` for a gate with no list attached — exactly
/// [`decide`]'s behaviour.
pub fn decide_with_blocks(
    access: &impl WorldAccess,
    policy: &BlobPolicy,
    blocks: Option<&dyn BlockList>,
    requester: &EndpointId,
    hash: &Hash,
    request: Request,
) -> bool {
    if let Some(blocks) = blocks {
        // Explicit indeterminate-result policy (#8750): blob serving
        // fails open — a block lookup that cannot answer does not
        // refuse the request — and the failure is logged for the
        // operator instead of being treated as a clean "not blocked".
        // Only a definite block refuses here.
        match blocks.peer_lookup(requester.as_bytes()) {
            BlockLookup::Blocked => return false,
            BlockLookup::Indeterminate(failure) => {
                tracing::warn!(
                    "blob gate: block-list lookup failed ({}); failing open",
                    failure.as_str()
                );
            }
            BlockLookup::Unblocked => {}
        }
    }
    access.worlds_referencing(hash).iter().any(|world| {
        access.is_current_member(world, requester)
            && match request {
                Request::Fetch => true,
                Request::Observe => policy.observe_allowed(world),
            }
    })
}

/// Whether `requester` may make `request` for `hash`, counting local trust.
///
/// A requester in `local_readers` — the node's own key and its data directory's client
/// key, the keys the write policy trusts automatically — may fetch or observe any blob
/// in its own store, referenced or not: that is how the local CLI and REPL keep working
/// when the gate is on, including for locally published content no world references.
/// Everyone else is decided by [`decide`], unchanged.
pub fn decide_with_local(
    access: &impl WorldAccess,
    policy: &BlobPolicy,
    local_readers: &BTreeSet<EndpointId>,
    requester: &EndpointId,
    hash: &Hash,
    request: Request,
) -> bool {
    local_readers.contains(requester) || decide(access, policy, requester, hash, request)
}

/// The child hashes a blob's bytes name when the blob is a `HashSeq`: the sequence is the
/// raw concatenation of 32-byte hashes, so any other length is not a sequence and names no
/// children.
pub fn hashseq_children(bytes: &[u8]) -> Vec<Hash> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(32) {
        return Vec::new();
    }
    bytes
        .chunks_exact(32)
        .map(|chunk| {
            let mut raw = [0u8; 32];
            raw.copy_from_slice(chunk);
            Hash::from_bytes(raw)
        })
        .collect()
}

/// Builds the reference index the gate decides from: blob hash → the worlds that name it.
///
/// `references` pairs each referenced blob with the world that names it (a document event's
/// blob and its group, as the worlds crate's `world_key` spells it). A document with no
/// group names no world: the caller leaves it out, and its blob stays unreferenced.
///
/// `expand` names the children of a blob that is a `HashSeq` collection. Children are
/// attributed to every world that references the collection, transitively: a member of
/// the world may fetch the collection's contents, not just its sequence blob. The caller
/// decides what counts as a collection (serve expands only a blob whose parsed children
/// are all present in its store, so arbitrary 32-byte-multiple content does not spray
/// references onto hashes nobody stored).
pub fn build_reference_index(
    references: impl IntoIterator<Item = (Hash, String)>,
    expand: impl Fn(&Hash) -> Vec<Hash>,
) -> HashMap<Hash, BTreeSet<String>> {
    let mut index: HashMap<Hash, BTreeSet<String>> = HashMap::new();
    for (hash, world) in references {
        index.entry(hash).or_default().insert(world);
    }
    // Attribute HashSeq children to the referencing worlds, transitively. The worklist
    // holds (hash, world) pairs not yet expanded for that world, so the loop ends when
    // every reachable pair has been expanded once.
    let mut worklist: Vec<(Hash, String)> = index
        .iter()
        .flat_map(|(hash, worlds)| worlds.iter().map(move |world| (*hash, world.clone())))
        .collect();
    let mut expanded: BTreeSet<(Hash, String)> = BTreeSet::new();
    while let Some((hash, world)) = worklist.pop() {
        if !expanded.insert((hash, world.clone())) {
            continue;
        }
        for child in expand(&hash) {
            let worlds = index.entry(child).or_default();
            if worlds.insert(world.clone()) {
                worklist.push((child, world.clone()));
            }
        }
    }
    index
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
/// Fetch, `get_many` and observe are decided by [`decide_with_local`] over `access` and
/// `local_readers`. Pushes are decided by `writes`, as in
/// [`id_core::access::blobs_events`]. Every request is intercepted, so no store read
/// happens before the verdict.
///
/// A background task is spawned on the current Tokio runtime and ends when the sender is
/// dropped.
pub fn blob_events<A: WorldAccess>(
    access: Arc<A>,
    policy: BlobPolicy,
    writes: AccessPolicy,
    local_readers: BTreeSet<EndpointId>,
) -> EventSender {
    blob_events_with_blocks(access, policy, writes, None, local_readers)
}

/// [`blob_events`], with the operator block list composed into every
/// serving verdict (#2245): a blocked endpoint is refused `get`,
/// `get_many`, and `observe` through the same `AbortReason::Permission`
/// as a non-member, so a refusal still does not reveal which rule made
/// it. Push verdicts are unchanged — writes are ingest, and a block
/// never rejects what a peer sends, only what it is served.
pub fn blob_events_with_blocks<A: WorldAccess>(
    access: Arc<A>,
    policy: BlobPolicy,
    writes: AccessPolicy,
    blocks: Option<Arc<dyn BlockList>>,
    local_readers: BTreeSet<EndpointId>,
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
                        local_readers.contains(id)
                            || decide_with_blocks(
                                &*access,
                                &policy,
                                blocks.as_deref(),
                                id,
                                &m.request.hash,
                                Request::Fetch,
                            )
                    });
                    let _ = m.tx.send(verdict).await;
                }
                ProviderMessage::GetManyRequestReceived(m) => {
                    let verdict = admit(peers.get(&m.connection_id), |id| {
                        local_readers.contains(id)
                            || m.request.hashes.iter().all(|hash| {
                                decide_with_blocks(
                                    &*access,
                                    &policy,
                                    blocks.as_deref(),
                                    id,
                                    hash,
                                    Request::Fetch,
                                )
                            })
                    });
                    let _ = m.tx.send(verdict).await;
                }
                ProviderMessage::ObserveRequestReceived(m) => {
                    let verdict = admit(peers.get(&m.connection_id), |id| {
                        local_readers.contains(id)
                            || decide_with_blocks(
                                &*access,
                                &policy,
                                blocks.as_deref(),
                                id,
                                &m.request.hash,
                                Request::Observe,
                            )
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

    /// An operator block list naming fixed peers by key (the local
    /// stand-in for the facade's `pull_policy::testing::stub_blocks`,
    /// which folds through the facade's worlds and cannot live here).
    #[derive(Debug, Default)]
    struct StubBlocks {
        peers: std::collections::HashSet<[u8; 32]>,
    }

    impl BlockList for StubBlocks {
        fn is_blocked(&self, kind: id_events::event_store::BlockKind, value: &[u8]) -> bool {
            match kind {
                id_events::event_store::BlockKind::PeerKey => {
                    <[u8; 32]>::try_from(value).is_ok_and(|key| self.peers.contains(&key))
                }
                _ => false,
            }
        }
    }

    fn stub_blocks(peers: &[[u8; 32]]) -> Arc<StubBlocks> {
        Arc::new(StubBlocks {
            peers: peers.iter().copied().collect(),
        })
    }

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
    fn a_local_reader_fetches_an_unreferenced_blob_but_a_stranger_cannot() {
        // No world references blob(9) and nobody is a member of anything: the
        // membership rule alone refuses everyone, which is the lockout the local
        // trust exists to prevent for the operator's own keys.
        let t = Table::default();
        let p = BlobPolicy::default();
        let local = BTreeSet::from([id(7)]);
        assert!(decide_with_local(
            &t,
            &p,
            &local,
            &id(7),
            &blob(9),
            Request::Fetch
        ));
        assert!(!decide_with_local(
            &t,
            &p,
            &local,
            &id(8),
            &blob(9),
            Request::Fetch
        ));
        // Local trust also covers observe of the operator's own store, even
        // when the server policy leaves observe off for worlds.
        assert!(decide_with_local(
            &t,
            &p,
            &local,
            &id(7),
            &blob(9),
            Request::Observe
        ));
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
    fn a_blocked_member_is_refused_fetch_and_observe() {
        // #2245: the block composes over membership — a current member
        // blocked by endpoint key is served nothing.
        let t = one_world(id(1), blob(9));
        let p = BlobPolicy::default().with_observe();
        let blocks = stub_blocks(&[*id(1).as_bytes()]);
        for request in [Request::Fetch, Request::Observe] {
            assert!(!decide_with_blocks(
                &t,
                &p,
                Some(&*blocks),
                &id(1),
                &blob(9),
                request
            ));
        }
        // A list that blocks someone else leaves the member's verdict
        // exactly what membership decides.
        let blocks = stub_blocks(&[*id(2).as_bytes()]);
        assert!(decide_with_blocks(
            &t,
            &p,
            Some(&*blocks),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod producer_tests {
    use std::collections::HashMap;

    use super::*;

    fn blob(n: u8) -> Hash {
        Hash::from_bytes([n; 32])
    }

    #[test]
    fn reference_index_attributes_hashseq_children_transitively() {
        // world "w" references the sequence blob 1; 1 names 2 and 3; 2 names 4.
        let children: HashMap<Hash, Vec<Hash>> =
            HashMap::from([(blob(1), vec![blob(2), blob(3)]), (blob(2), vec![blob(4)])]);
        let index = build_reference_index([(blob(1), "w".to_owned())], |hash| {
            children.get(hash).cloned().unwrap_or_default()
        });
        for hash in [blob(1), blob(2), blob(3), blob(4)] {
            assert_eq!(
                index.get(&hash).map(|w| w.contains("w")),
                Some(true),
                "{hash} must be referenced by w"
            );
        }
        assert!(!index.contains_key(&blob(9)), "unrelated hash stays out");
    }

    #[test]
    fn hashseq_children_parses_concatenated_hashes() {
        let mut bytes = blob(5).as_bytes().to_vec();
        bytes.extend_from_slice(blob(6).as_bytes());
        assert_eq!(hashseq_children(&bytes), vec![blob(5), blob(6)]);
        assert!(hashseq_children(&bytes[..33]).is_empty(), "ragged length");
        assert!(hashseq_children(&[]).is_empty(), "empty names nothing");
    }
}
