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
//! The reference index and the membership query are behind [`WorldAccess`]. [`FoldedWorlds`]
//! answers membership from the folded worlds. The index has a producer now:
//! [`build_reference_index`] over the event store's document references (see
//! `EventStore::document_references`), with [`fold_group_events`] folding the group log for
//! membership, and [`RefreshingWorlds`] swapping fresh folds in as they change. `id serve`
//! wires [`blob_events`] with those pieces whenever it has the node's event store open.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, RwLock},
};

use iroh_base::EndpointId;
use iroh_blobs::{
    Hash,
    provider::events::{
        AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode,
    },
};

use crate::access::AccessPolicy;
use crate::groups::{Role, worlds::Worlds};

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

/// [`WorldAccess`] over the folded worlds.
///
/// Membership is the world's own authority, as [`Worlds::holds_here`] gives it: a member of its
/// group, or a grant in its own chains. A crossing from another world does not count.
///
/// The reference index maps a blob hash to the worlds that name it. It is built by
/// [`build_reference_index`] from the event store's document references; artifacts are not a
/// source, because they carry SHA-256 hashes, not BLAKE3 blob hashes, and only WASM modules
/// are pinned, without a record of their world. A hash the index does not hold is
/// unreferenced, which refuses it to every requester.
#[derive(Debug)]
pub struct FoldedWorlds {
    worlds: Worlds,
    references: HashMap<Hash, BTreeSet<String>>,
}

impl FoldedWorlds {
    /// Pairs the folded worlds with a reference index.
    pub const fn new(worlds: Worlds, references: HashMap<Hash, BTreeSet<String>>) -> Self {
        Self { worlds, references }
    }
}

impl WorldAccess for FoldedWorlds {
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
        self.worlds
            .holds_here(world, *requester.as_bytes(), Role::Pull)
    }

    fn worlds_referencing(&self, hash: &Hash) -> Vec<String> {
        self.references
            .get(hash)
            .map(|worlds| worlds.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// The world key a group is indexed under: the hex of its create event's hash, which is the
/// group hash every later event of the group names. A document event's `group` extension is
/// the same hash, so document references and membership folds agree on the key.
pub fn world_key(group: &p2panda_core::Hash) -> String {
    group.to_string()
}

/// Folds group-log events into the [`Worlds`] the gate reads membership from.
///
/// Events are partitioned by world: a create event opens the world keyed by its own hash
/// (a friendship's create names its derived group instead), and every other event joins
/// the world its `group` names. An event that names no group and creates none belongs to
/// no world and is skipped.
///
/// # Errors
///
/// Any error [`Worlds::fold`] gives for one world.
pub fn fold_group_events(
    events: &[crate::groups::Event],
) -> Result<Worlds, crate::groups::FoldError> {
    let mut by_world: BTreeMap<String, Vec<crate::groups::Event>> = BTreeMap::new();
    for event in events {
        let key = match (&event.body, &event.group) {
            (_, Some(group)) => world_key(group),
            (crate::groups::Body::Create(_), None) => world_key(&event.id),
            _ => continue,
        };
        by_world.entry(key).or_default().push(event.clone());
    }
    Worlds::fold(&by_world, BTreeMap::new())
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
/// blob and its group, as [`world_key`] spells it). A document with no group names no
/// world: the caller leaves it out, and its blob stays unreferenced.
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

/// [`WorldAccess`] whose folded worlds a producer can replace as they change.
///
/// `blob_events` holds one `Arc` for the life of the server; the fold behind it must not
/// be the snapshot from startup, or a removed member would keep fetching and a new
/// document would stay unreferenced until a restart. The producer re-folds and calls
/// [`RefreshingWorlds::refresh`]; each decision then reads the latest fold. A poisoned
/// lock fails closed, like every other gate refusal.
#[derive(Debug)]
pub struct RefreshingWorlds {
    inner: RwLock<FoldedWorlds>,
}

impl RefreshingWorlds {
    /// Starts from an initial fold.
    pub const fn new(initial: FoldedWorlds) -> Self {
        Self {
            inner: RwLock::new(initial),
        }
    }

    /// Replaces the folded worlds every later decision reads.
    pub fn refresh(&self, next: FoldedWorlds) {
        if let Ok(mut current) = self.inner.write() {
            *current = next;
        }
    }
}

impl WorldAccess for RefreshingWorlds {
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
        self.inner
            .read()
            .is_ok_and(|folded| folded.is_current_member(world, requester))
    }

    fn worlds_referencing(&self, hash: &Hash) -> Vec<String> {
        self.inner
            .read()
            .map(|folded| folded.worlds_referencing(hash))
            .unwrap_or_default()
    }
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
    use std::collections::BTreeMap;

    use iroh_base::SecretKey;
    use proptest::prelude::*;

    use super::*;
    use crate::groups::policy::Policy;
    use crate::groups::{Body, Create, Event, MemberOp, Template};

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

    /// The world `docs`, founded by `founder`, which adds `member` as a reader and, when
    /// `removed`, removes them again. Folded as the store folds a world.
    fn folded(founder: EndpointId, member: EndpointId, removed: bool) -> Worlds {
        let (founder, member) = (*founder.as_bytes(), *member.as_bytes());
        let create = p2panda_core::Hash::digest(b"create");
        let add = p2panda_core::Hash::digest(b"add");
        let remove = p2panda_core::Hash::digest(b"remove");
        let mut events = vec![
            Event {
                id: create,
                author: founder,
                after: Vec::new(),
                group: None,
                body: Body::Create(Create {
                    template: Template::Custom,
                    policy: Policy::default(),
                    members: Vec::new(),
                    admins_on_create: false,
                }),
            },
            Event {
                id: add,
                author: founder,
                after: vec![create],
                group: Some(create),
                body: Body::Member(MemberOp::Add {
                    member,
                    role: Role::Read,
                }),
            },
        ];
        if removed {
            events.push(Event {
                id: remove,
                author: founder,
                after: vec![add],
                group: Some(create),
                body: Body::Member(MemberOp::Remove {
                    member,
                    successor: None,
                }),
            });
        }
        let worlds = BTreeMap::from([("docs".to_owned(), events)]);
        Worlds::fold(&worlds, BTreeMap::new()).unwrap()
    }

    /// The folded `docs` world, with `hash` indexed as named by `docs`.
    fn gate(worlds: Worlds, hash: Hash) -> FoldedWorlds {
        let refs = HashMap::from([(hash, BTreeSet::from(["docs".to_owned()]))]);
        FoldedWorlds::new(worlds, refs)
    }

    #[test]
    fn folded_membership_admits_a_member_and_refuses_a_stranger() {
        let g = gate(folded(id(1), id(2), false), blob(9));
        let p = BlobPolicy::default();
        assert!(decide(&g, &p, &id(2), &blob(9), Request::Fetch));
        // The founder is Admin, so is a member too.
        assert!(decide(&g, &p, &id(1), &blob(9), Request::Fetch));
        assert!(!decide(&g, &p, &id(3), &blob(9), Request::Fetch));
    }

    #[test]
    fn a_removed_member_loses_fetch_on_the_folded_world() {
        let g = gate(folded(id(1), id(2), true), blob(9));
        assert!(!decide(
            &g,
            &BlobPolicy::default(),
            &id(2),
            &blob(9),
            Request::Fetch
        ));
    }

    #[test]
    fn an_unindexed_blob_is_refused_even_to_a_member() {
        let g = gate(folded(id(1), id(2), false), blob(9));
        let p = BlobPolicy::default().with_observe();
        assert!(!decide(&g, &p, &id(2), &blob(5), Request::Fetch));
        assert!(!decide(&g, &p, &id(2), &blob(5), Request::Observe));
    }

    #[test]
    fn observe_over_the_folded_world_follows_the_policy() {
        let g = gate(folded(id(1), id(2), false), blob(9));
        assert!(!decide(
            &g,
            &BlobPolicy::default(),
            &id(2),
            &blob(9),
            Request::Observe
        ));
        let p = BlobPolicy::default().with_observe();
        assert!(decide(&g, &p, &id(2), &blob(9), Request::Observe));
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
    use std::collections::{BTreeMap, HashMap};

    use iroh_base::SecretKey;
    use p2panda_core::Hash as PandaHash;

    use super::*;
    use crate::groups::policy::Policy;
    use crate::groups::{Body, Create, Event, MemberOp, Template};

    fn id(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

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

    /// A group log for one world: founder creates it, adds `member`, then removes them
    /// when `removed`. Returns the events and the world key the fold gives them.
    fn group_events(member: EndpointId, removed: bool) -> (Vec<Event>, String) {
        let founder = *id(1).as_bytes();
        let member_key = *member.as_bytes();
        let create = PandaHash::digest(b"gate-create");
        let add = PandaHash::digest(b"gate-add");
        let mut events = vec![
            Event {
                id: create,
                author: founder,
                after: Vec::new(),
                group: None,
                body: Body::Create(Create {
                    template: Template::Custom,
                    policy: Policy::default(),
                    members: Vec::new(),
                    admins_on_create: false,
                }),
            },
            Event {
                id: add,
                author: founder,
                after: vec![create],
                group: Some(create),
                body: Body::Member(MemberOp::Add {
                    member: member_key,
                    role: crate::groups::Role::Read,
                }),
            },
        ];
        if removed {
            events.push(Event {
                id: PandaHash::digest(b"gate-remove"),
                author: founder,
                after: vec![add],
                group: Some(create),
                body: Body::Member(MemberOp::Remove {
                    member: member_key,
                    successor: None,
                }),
            });
        }
        (events, world_key(&create))
    }

    #[test]
    fn folded_group_log_gates_fetch_and_refresh_revokes_it() {
        let member = id(2);
        let (events, world) = group_events(member, false);
        let worlds = fold_group_events(&events).expect("the log folds");
        let index = build_reference_index([(blob(7), world.clone())], |_| Vec::new());
        let gate = RefreshingWorlds::new(FoldedWorlds::new(worlds, index));
        let policy = BlobPolicy::default();
        assert!(
            decide(&gate, &policy, &member, &blob(7), Request::Fetch),
            "a current member fetches a referenced blob"
        );
        assert!(
            !decide(&gate, &policy, &id(3), &blob(7), Request::Fetch),
            "a non-member is refused"
        );
        assert!(
            !decide(&gate, &policy, &member, &blob(8), Request::Fetch),
            "an unreferenced blob is refused even to a member"
        );

        // The member is removed; a refreshed fold must revoke the fetch at once.
        let (events, world) = group_events(member, true);
        let worlds = fold_group_events(&events).expect("the log folds");
        let index = build_reference_index([(blob(7), world)], |_| Vec::new());
        gate.refresh(FoldedWorlds::new(worlds, index));
        assert!(
            !decide(&gate, &policy, &member, &blob(7), Request::Fetch),
            "a removed member loses fetch on the next request"
        );
        let _ = BTreeMap::<String, String>::new();
    }
}
