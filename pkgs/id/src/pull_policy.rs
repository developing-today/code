//! The pull policy, re-exported from `id-events`.
//!
//! Step 2b of the pkgs/id crate split (#4405): the implementation of
//! this module — the policy value types and the gate — lives in
//! `id_events::pull_policy`, the sync-gating vocabulary the events
//! layer owns. This facade module holds the re-exports, the test
//! fixtures that fold through the facade's own `groups::worlds` (which
//! stays here until step 4), and the one gate test that runs against
//! the real event store, so every historical `id::pull_policy` path
//! is unchanged.

pub use id_events::pull_policy::{PullGate, PullPolicy, PullRefused};

/// Fixtures for the pull tests and the session tests: a folded world, and membership a test can
/// change between two checks.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod testing {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{Arc, Mutex};

    use iroh_base::{EndpointId, SecretKey};
    use p2panda_core::Hash;

    use super::{PullGate, PullPolicy};
    use crate::blob_gate::{FoldedWorlds, WorldAccess};
    use crate::groups::policy::Policy;
    use crate::groups::worlds::Worlds;
    use crate::groups::{Body, Create, Event, MemberOp, Role, Template};

    /// The world every fixture names.
    pub(crate) const WORLD: &str = "docs";

    /// The identity with byte `n` repeated, so a test knows the endpoint ID before it dials.
    pub(crate) fn key(n: u8) -> SecretKey {
        SecretKey::from_bytes(&[n; 32])
    }

    /// The folded `docs` world, founded by `founder`, which adds `member` as a reader and, when
    /// `removed`, removes them again. Folded as the store folds a world.
    pub(crate) fn docs_world(
        founder: EndpointId,
        member: EndpointId,
        removed: bool,
    ) -> FoldedWorlds {
        let (founder, member) = (*founder.as_bytes(), *member.as_bytes());
        let create = Hash::digest(b"create");
        let add = Hash::digest(b"add");
        let remove = Hash::digest(b"remove");
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
        let worlds = BTreeMap::from([(WORLD.to_owned(), events)]);
        let worlds = Worlds::fold(&worlds, BTreeMap::new()).expect("the fixture folds");
        FoldedWorlds::new(worlds, HashMap::new())
    }

    /// Membership that a test can change between two checks, as a removal changes the fold.
    #[derive(Debug)]
    pub(crate) struct Live(Mutex<FoldedWorlds>);

    impl Live {
        pub(crate) fn new(folded: FoldedWorlds) -> Self {
            Self(Mutex::new(folded))
        }

        /// Replaces the membership, as the fold of a newer log does.
        pub(crate) fn set(&self, folded: FoldedWorlds) {
            *self.0.lock().expect("the fixture lock is not poisoned") = folded;
        }
    }

    impl WorldAccess for Live {
        fn is_current_member(&self, world: &str, requester: &EndpointId) -> bool {
            self.0
                .lock()
                .is_ok_and(|folded| folded.is_current_member(world, requester))
        }

        fn worlds_referencing(&self, hash: &iroh_blobs::Hash) -> Vec<String> {
            self.0
                .lock()
                .map(|folded| folded.worlds_referencing(hash))
                .unwrap_or_default()
        }
    }

    /// `folded` as shared membership that a gate can hold.
    pub(crate) fn live(folded: FoldedWorlds) -> Arc<Live> {
        Arc::new(Live::new(folded))
    }

    /// An operator block list with fixed rows, for the gate tests and the
    /// session tests: the peers and topic tags it names are blocked, and
    /// nothing else is.
    #[derive(Debug, Default)]
    pub(crate) struct StubBlocks {
        /// Blocked endpoint keys.
        pub peers: std::collections::HashSet<[u8; 32]>,
        /// Blocked topic tags, as raw bytes.
        pub topics: std::collections::HashSet<Vec<u8>>,
        /// Blocked IPs, in the text form `BlockList::ip_blocked`
        /// formats them in.
        pub ips: std::collections::HashSet<String>,
    }

    impl crate::event_store::BlockList for StubBlocks {
        fn is_blocked(&self, kind: crate::event_store::BlockKind, value: &[u8]) -> bool {
            match kind {
                crate::event_store::BlockKind::PeerKey => {
                    <[u8; 32]>::try_from(value).is_ok_and(|key| self.peers.contains(&key))
                }
                crate::event_store::BlockKind::TopicTag => self.topics.iter().any(|t| t == value),
                crate::event_store::BlockKind::Ip => {
                    std::str::from_utf8(value).is_ok_and(|text| self.ips.contains(text))
                }
            }
        }
    }

    /// A stub block list naming `peers` by key and `topics` by tag.
    pub(crate) fn stub_blocks(peers: &[[u8; 32]], topics: &[&str]) -> Arc<StubBlocks> {
        Arc::new(StubBlocks {
            peers: peers.iter().copied().collect(),
            topics: topics.iter().map(|t| t.as_bytes().to_vec()).collect(),
            ips: std::collections::HashSet::new(),
        })
    }

    /// A stub block list like [`stub_blocks`], additionally naming
    /// `ips` as blocked.
    pub(crate) fn stub_blocks_with_ips(
        peers: &[[u8; 32]],
        topics: &[&str],
        ips: &[std::net::IpAddr],
    ) -> Arc<StubBlocks> {
        Arc::new(StubBlocks {
            peers: peers.iter().copied().collect(),
            topics: topics.iter().map(|t| t.as_bytes().to_vec()).collect(),
            ips: ips.iter().map(|ip| ip.to_string()).collect(),
        })
    }

    /// An open gate over an empty world. It never reads membership.
    pub(crate) fn open_gate() -> PullGate {
        let none: BTreeMap<String, Vec<Event>> = BTreeMap::new();
        let empty = Worlds::fold(&none, BTreeMap::new()).expect("an empty fold");
        PullGate::new(
            PullPolicy::Open,
            WORLD,
            live(FoldedWorlds::new(empty, HashMap::new())),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::testing::{WORLD, docs_world, key, live};
    use super::*;
    use std::sync::Arc;

    /// #2245, against the real store-backed list: a permanent block
    /// refuses the peer, and an expired block does not — expiry is judged
    /// by the list itself, so the gate needs no clock of its own.
    #[test]
    fn test_pull_gate_over_a_store_block_list_enforces_only_live_blocks() {
        use crate::event_store::{Block, BlockKind, EventStore};
        use crate::waiting::WaitingLimits;

        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        let row = |value: &[u8], expires_at_ms: Option<u64>| Block {
            kind: BlockKind::PeerKey,
            value: value.to_vec(),
            added_at_ms: 1_000,
            reason: "gate test".to_owned(),
            expires_at_ms,
        };
        store
            .add_block(&row(key(9).public().as_bytes(), None))
            .unwrap();
        store
            .add_block(&row(key(8).public().as_bytes(), Some(1_500)))
            .unwrap();
        let gate = PullGate::new(
            PullPolicy::Open,
            WORLD,
            live(docs_world(key(1).public(), key(2).public(), false)),
        )
        .with_blocks(Arc::new(store));
        assert!(!gate.admits(&key(9).public()), "a permanent block refuses");
        assert!(
            gate.admits(&key(8).public()),
            "an expired block (expiry 1_500, long past) is not enforced"
        );
        assert!(gate.admits(&key(2).public()), "an unlisted peer pulls");
    }
}
