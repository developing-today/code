//! Who may fetch or observe a blob (P3.4, decision 5) — facade shim
//! over `id-groups`, plus the folded-worlds adapter.
//!
//! Step 4 of the pkgs/id crate split (#4407): the gate itself —
//! [`BlobPolicy`], [`Request`], [`decide`], [`decide_with_blocks`] and
//! the provider wiring in [`blob_events`] — lives in
//! `id_groups::blob_gate`, over the [`WorldAccess`] trait the events
//! layer owns (step 2b). What stays here is [`FoldedWorlds`], the
//! `WorldAccess` implementation over the folded worlds: it folds
//! through [`crate::groups::worlds::Worlds`], which is seam 6's
//! bridge file and leaves for `id-worlds` in step 5, so the adapter
//! stays in the facade with it (the same split step 2a used for the
//! store traits: trait and gate below, world-folding adapter above).
//! Step 5 moves `FoldedWorlds` together with `worlds.rs`.
//!
//! The gate's own documentation, in `id_groups::blob_gate`: a fetch
//! is allowed only to a current member of a world that references
//! the hash; observe needs the same membership and a server policy
//! that serves observe for that world; refusals look the same
//! whether the hash is unreferenced or the requester may not reach
//! it; and an unknown requester is refused.

use std::collections::{BTreeSet, HashMap};

use iroh_base::EndpointId;
use iroh_blobs::Hash;

use crate::groups::{Role, worlds::Worlds};

pub use id_groups::blob_gate::{
    BlobPolicy, Request, blob_events, blob_events_with_blocks, decide, decide_with_blocks,
};

/// The two questions the gate asks of the world graph, re-exported at
/// its historical path (the trait itself is `id_events::world_access`).
pub use id_events::world_access::WorldAccess;

/// [`WorldAccess`] over the folded worlds.
///
/// Membership is the world's own authority, as [`Worlds::holds_here`] gives it: a member of its
/// group, or a grant in its own chains. A crossing from another world does not count.
///
/// The reference index maps a blob hash to the worlds that name it. Nothing in the store writes
/// it yet. Artifacts carry SHA-256 hashes, not BLAKE3 blob hashes, and only WASM modules are
/// pinned, without a record of their world. So the index is passed in, and a hash it does not
/// hold is unreferenced, which refuses it to every requester.
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use iroh_base::SecretKey;

    use super::*;
    use crate::groups::policy::Policy;
    use crate::groups::{Body, Create, Event, MemberOp, Template};

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
