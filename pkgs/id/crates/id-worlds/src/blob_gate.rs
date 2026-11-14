//! The blob gate's folded-worlds adapter.
//!
//! Step 4 of the pkgs/id crate split (#4407) extracted the gate
//! itself — policy, verdicts and provider wiring — into
//! `id_groups::blob_gate`, over the [`WorldAccess`] trait the events
//! layer owns. What stayed in the facade was [`FoldedWorlds`], the
//! `WorldAccess` implementation over the folded worlds, because it
//! folds through [`crate::groups::worlds::Worlds`]: seam 6's bridge
//! file, which the design sends to the worlds crate. Step 5 moves
//! the adapter together with the bridge, here. (`FoldedWorlds`
//! cannot live in `id-groups`: that crate is below this one and
//! must not name world types.)

use std::collections::{BTreeSet, HashMap};
use std::sync::RwLock;

use iroh_base::EndpointId;
use iroh_blobs::Hash;

use id_events::world_access::WorldAccess;

use crate::groups::{Role, worlds::Worlds};

/// [`WorldAccess`] over a full group-log projection or the older per-world folds.
///
/// For a full group-log projection, membership is queried by the exact group hash the world
/// names, so another group's membership or grants cannot leak across worlds. A per-world fold
/// uses [`Worlds::holds_here`], which likewise excludes crossings from another world.
///
/// The reference index maps a blob hash to the worlds that name it. It is built by
/// `id_groups::blob_gate::build_reference_index` from the event store's document
/// references; artifacts are not a source, because they carry SHA-256 hashes, not BLAKE3
/// blob hashes, and only WASM modules are pinned, without a record of their world. A hash
/// the index does not hold is unreferenced, which refuses it to every requester.
#[derive(Debug)]
pub struct FoldedWorlds {
    membership: FoldedMembership,
    references: HashMap<Hash, BTreeSet<String>>,
}

#[derive(Debug)]
enum FoldedMembership {
    PerWorld(Worlds),
    GroupProjection(crate::groups::Projection),
}

impl FoldedWorlds {
    /// Pairs the folded worlds with a reference index.
    pub const fn new(worlds: Worlds, references: HashMap<Hash, BTreeSet<String>>) -> Self {
        Self {
            membership: FoldedMembership::PerWorld(worlds),
            references,
        }
    }

    /// Pairs a full group-log projection with a reference index.
    ///
    /// Unlike `new`, the projection spans all groups in the group log and therefore preserves
    /// its complete causal graph. Each membership request is limited to the group named by its
    /// world key.
    pub const fn from_group_projection(
        projection: crate::groups::Projection,
        references: HashMap<Hash, BTreeSet<String>>,
    ) -> Self {
        Self {
            membership: FoldedMembership::GroupProjection(projection),
            references,
        }
    }
}

impl WorldAccess for FoldedWorlds {
    fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
        match &self.membership {
            FoldedMembership::PerWorld(worlds) => {
                worlds.holds_here(world, *requester.as_bytes(), Role::Pull)
            }
            FoldedMembership::GroupProjection(projection) => {
                group_projection_member(projection, world, requester)
            }
        }
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

/// Folds group-log events into one projection the gate reads membership from.
///
/// The whole log is folded once, as one causal graph: `GroupLog::record` names the log's
/// global heads as `after`, so an event's dependencies routinely live in another group, and
/// partitioning the log by world hands `fold` dependencies it cannot resolve. Keeping the
/// complete graph also preserves cross-group vetoes and removals, which per-world partitions
/// would silently drop.
///
/// The one projection holds every group. [`group_projection_member`] reads membership for the
/// group the requested world names, so another group's membership cannot leak authority.
///
/// # Errors
///
/// Any error [`crate::groups::fold`] gives for the log.
pub fn fold_group_events(
    events: &[crate::groups::Event],
) -> Result<crate::groups::Projection, crate::groups::FoldError> {
    crate::groups::fold(events)
}

/// Whether `requester` is a current `Pull` member of the group `world` names in the full log.
///
/// A world key is the hex of the group's create event hash (see [`world_key`]), so the global
/// projection can be asked about exactly that group.
#[must_use]
fn group_projection_member(
    projection: &crate::groups::Projection,
    world: &str,
    requester: &EndpointId,
) -> bool {
    let Ok(group_id) = id_groups::groups::log::hash_from_hex(world) else {
        return false;
    };
    let actor = *requester.as_bytes();
    projection.group(&group_id).is_some_and(|group| {
        group.is_active()
            && group
                .members
                .get(&actor)
                .is_some_and(|role| *role >= Role::Pull)
    }) || projection.grants_in_force().any(|grant| {
        grant.group == group_id
            && grant.target.is_none()
            && grant.to == actor
            && grant.scope >= Role::Pull
    })
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use iroh_base::SecretKey;

    use super::*;
    use crate::groups::policy::Policy;
    use crate::groups::{Body, Create, Event, MemberOp, Template};
    use id_groups::blob_gate::{BlobPolicy, Request, decide};

    fn id(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    fn blob(n: u8) -> Hash {
        Hash::from_bytes([n; 32])
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
                    mode: crate::groups::RemoveMode::Revoke,
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

    /// `GroupLog::record` names the log's global heads as `after`, so a group created after
    /// another names the first group's events. Fold the complete graph and still keep each
    /// group's membership isolated.
    #[test]
    fn fold_group_events_tolerates_cross_world_causality() {
        fn create() -> Body {
            Body::Create(Create {
                template: Template::Custom,
                policy: Policy::default(),
                members: Vec::new(),
                admins_on_create: false,
            })
        }
        let founder = *id(1).as_bytes();
        let member_a = *id(2).as_bytes();
        let member_b = *id(3).as_bytes();
        let create_a = p2panda_core::Hash::digest(b"create-a");
        let add_a = p2panda_core::Hash::digest(b"add-a");
        let create_b = p2panda_core::Hash::digest(b"create-b");
        let add_b = p2panda_core::Hash::digest(b"add-b");
        let events = vec![
            Event {
                id: create_a,
                author: founder,
                after: Vec::new(),
                group: None,
                body: create(),
            },
            Event {
                id: add_a,
                author: founder,
                after: vec![create_a],
                group: Some(create_a),
                body: Body::Member(MemberOp::Add {
                    member: member_a,
                    role: Role::Read,
                }),
            },
            Event {
                id: create_b,
                author: founder,
                after: vec![create_a, add_a],
                group: None,
                body: create(),
            },
            Event {
                id: add_b,
                author: founder,
                after: vec![create_b, add_a],
                group: Some(create_b),
                body: Body::Member(MemberOp::Add {
                    member: member_b,
                    role: Role::Read,
                }),
            },
        ];
        let projection = fold_group_events(&events).expect("cross-world causality must fold");
        let (key_a, key_b) = (world_key(&create_a), world_key(&create_b));
        let folded = FoldedWorlds::from_group_projection(projection, HashMap::new());
        assert!(folded.is_current_member(&key_a, &id(2)));
        assert!(!folded.is_current_member(&key_a, &id(3)));
        assert!(folded.is_current_member(&key_b, &id(3)));
        assert!(!folded.is_current_member(&key_b, &id(2)));
    }

    /// A group log for one world: founder creates it, adds `member`, then removes them
    /// when `removed`. Returns the events and the world key the fold gives them.
    fn group_events(member: EndpointId, removed: bool) -> (Vec<Event>, String) {
        let founder = *id(1).as_bytes();
        let member_key = *member.as_bytes();
        let create = p2panda_core::Hash::digest(b"gate-create");
        let add = p2panda_core::Hash::digest(b"gate-add");
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
                    role: Role::Read,
                }),
            },
        ];
        if removed {
            events.push(Event {
                id: p2panda_core::Hash::digest(b"gate-remove"),
                author: founder,
                after: vec![add],
                group: Some(create),
                body: Body::Member(MemberOp::Remove {
                    member: member_key,
                    mode: crate::groups::RemoveMode::Revoke,
                }),
            });
        }
        (events, world_key(&create))
    }

    #[test]
    fn folded_group_log_gates_fetch_and_refresh_revokes_it() {
        use id_groups::blob_gate::build_reference_index;
        let member = id(2);
        let (events, world) = group_events(member, false);
        let projection = fold_group_events(&events).expect("the log folds");
        let index = build_reference_index([(blob(7), world.clone())], |_| Vec::new());
        let gate = RefreshingWorlds::new(FoldedWorlds::from_group_projection(projection, index));
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
        let projection = fold_group_events(&events).expect("the log folds");
        let index = build_reference_index([(blob(7), world)], |_| Vec::new());
        gate.refresh(FoldedWorlds::from_group_projection(projection, index));
        assert!(
            !decide(&gate, &policy, &member, &blob(7), Request::Fetch),
            "a removed member loses fetch on the next request"
        );
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
}
