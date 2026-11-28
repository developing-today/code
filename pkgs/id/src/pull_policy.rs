//! Who may pull a world's events from this server (P6.5, decision 11).
//!
//! The policy is a local serving rule. It decides what this server sends, and a peer never sees
//! it. Nothing here goes on the wire, so two servers with different policies still sync.
//!
//! - **Open** (the default): any peer may pull the world's events. That is how sync works today.
//! - **Members**: a peer may pull only while it is a current member of the world. Membership is
//!   read when each session starts, so a removed member is refused on the next pull.
//!
//! A [`PullGate`] binds a policy to one world and to the membership it is checked against. The
//! sync sessions check the gate before any event is sent. A refused peer gets no event, and the
//! refusal is clean. An open gate never reads membership.
//!
//! A gate may also carry the operator block list ([`crate::event_store::BlockList`], #2245):
//! a peer blocked by key is refused under every policy, and a gate whose world's name is a
//! blocked topic tag serves nothing. The blocks compose with the policy — they only ever
//! narrow it. Without a list attached, a gate behaves exactly as it did before the list
//! existed. The list is read on each check, so a block or an unblock lands on the next
//! session, as a membership removal does.
//!
//! This is a config setting, not a protocol rule. The policy belongs with the world's policy. The
//! server config's `[policy]` table parses it ([`crate::server_config::ServerConfig::pull_policy`]),
//! and `id serve` builds its live event-sync gate from that value
//! ([`crate::event_sync::gate`]); the node's own event store is not a world and has no membership
//! source, so `members` there refuses every pull rather than serving open. Blobs are not
//! governed here. The blob gate ([`crate::blob_gate`]) still requires membership to fetch, whatever
//! this policy says. Authorized pull is advisory: it stops an honest server from serving a
//! non-member. It does not stop re-serving, recall copies, or hide metadata.

use std::sync::Arc;

use iroh_base::EndpointId;

use crate::blob_gate::WorldAccess;
use crate::event_store::BlockList;

/// Who may pull a world's events from this server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullPolicy {
    /// Any peer may pull. The default, and today's behaviour.
    #[default]
    Open,
    /// Only a current member of the world may pull.
    Members,
}

impl PullPolicy {
    /// Whether `peer` may pull the events of `world`. Only [`PullPolicy::Members`] reads `access`,
    /// and it reads it on each call, so a removal is seen by the next call.
    pub fn admits(self, access: &dyn WorldAccess, world: &str, peer: &EndpointId) -> bool {
        match self {
            Self::Open => true,
            Self::Members => access.is_current_member(world, peer),
        }
    }
}

/// A policy bound to one world, and the membership it is checked against.
#[derive(Clone)]
pub struct PullGate {
    policy: PullPolicy,
    world: String,
    access: Arc<dyn WorldAccess>,
    /// The operator block list, when the gate's owner attached one
    /// ([`PullGate::with_blocks`]). Read on each check.
    blocks: Option<Arc<dyn BlockList>>,
}

impl PullGate {
    /// A gate for `world` under `policy`. `access` answers membership at the time of each check.
    /// No operator block list is attached; [`PullGate::with_blocks`] adds one.
    #[must_use]
    pub fn new<A: WorldAccess>(
        policy: PullPolicy,
        world: impl Into<String>,
        access: Arc<A>,
    ) -> Self {
        Self {
            policy,
            world: world.into(),
            access,
            blocks: None,
        }
    }

    /// Attaches the operator block list (#2245): a peer blocked by key is
    /// refused under every policy, and a gate whose world's name is a
    /// blocked topic tag serves nothing. The list is consulted live, so a
    /// block added after the gate was built is seen by the next check.
    #[must_use]
    pub fn with_blocks(mut self, blocks: Arc<dyn BlockList>) -> Self {
        self.blocks = Some(blocks);
        self
    }

    /// The world this gate serves.
    #[must_use]
    pub fn world(&self) -> &str {
        &self.world
    }

    /// Whether the operator block list blocks `peer` by key. False when
    /// no list is attached.
    #[must_use]
    pub fn peer_blocked(&self, peer: &EndpointId) -> bool {
        self.blocks
            .as_ref()
            .is_some_and(|blocks| blocks.peer_blocked(peer.as_bytes()))
    }

    /// Whether the operator block list blocks this topic tag (raw bytes:
    /// a world name, or a sync topic's hash bytes). False when no list is
    /// attached. Session seams that carry a topic the gate's world does
    /// not name — the `log_sync` handshake — check the tag through here.
    #[must_use]
    pub fn topic_tag_blocked(&self, tag: &[u8]) -> bool {
        self.blocks
            .as_ref()
            .is_some_and(|blocks| blocks.topic_tag_blocked(tag))
    }

    /// Whether `peer` may pull this world's events now: not blocked by
    /// key, the world not blocked as a topic tag, and the policy admits
    /// the peer.
    #[must_use]
    pub fn admits(&self, peer: &EndpointId) -> bool {
        !self.peer_blocked(peer)
            && !self.topic_tag_blocked(self.world.as_bytes())
            && self.policy.admits(self.access.as_ref(), &self.world, peer)
    }
}

impl std::fmt::Debug for PullGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PullGate")
            .field("policy", &self.policy)
            .field("world", &self.world)
            .finish_non_exhaustive()
    }
}

/// A session the pull policy refused. No event was sent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the pull policy refuses this peer the events of world {0:?}")]
pub struct PullRefused(pub String);

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

    #[test]
    fn test_pull_policy_defaults_to_open() {
        assert_eq!(PullPolicy::default(), PullPolicy::Open);
    }

    /// Open pull admits a stranger: today's behaviour, pinned.
    #[test]
    fn test_pull_policy_open_admits_a_non_member() {
        let gate = PullGate::new(
            PullPolicy::Open,
            WORLD,
            live(docs_world(key(1).public(), key(2).public(), false)),
        );
        assert!(gate.admits(&key(9).public()), "a stranger may pull");
        assert!(gate.admits(&key(2).public()), "a member may pull");
    }

    /// The members option admits a current member, the founder included, and refuses a stranger.
    #[test]
    fn test_pull_policy_members_admits_only_a_current_member() {
        let gate = PullGate::new(
            PullPolicy::Members,
            WORLD,
            live(docs_world(key(1).public(), key(2).public(), false)),
        );
        assert!(gate.admits(&key(2).public()), "a member");
        assert!(gate.admits(&key(1).public()), "the founder is a member too");
        assert!(!gate.admits(&key(9).public()), "a stranger is refused");
    }

    /// The check reads membership when it runs, so a removal lands on the next check.
    #[test]
    fn test_pull_policy_members_refuses_a_removed_member_on_the_next_check() {
        let access = live(docs_world(key(1).public(), key(2).public(), false));
        let gate = PullGate::new(PullPolicy::Members, WORLD, Arc::clone(&access));
        assert!(gate.admits(&key(2).public()), "before the removal");
        access.set(docs_world(key(1).public(), key(2).public(), true));
        assert!(!gate.admits(&key(2).public()), "after the removal");
    }

    /// #2245: a peer blocked by key is refused even under the open
    /// policy, which admits every other peer. The block only narrows.
    #[test]
    fn test_pull_gate_refuses_a_blocked_peer_under_open_policy() {
        let gate = PullGate::new(
            PullPolicy::Open,
            WORLD,
            live(docs_world(key(1).public(), key(2).public(), false)),
        )
        .with_blocks(super::testing::stub_blocks(
            &[*key(9).public().as_bytes()],
            &[],
        ));
        assert!(
            !gate.admits(&key(9).public()),
            "the blocked peer is refused"
        );
        assert!(
            gate.admits(&key(2).public()),
            "an unblocked peer still pulls"
        );
    }

    /// #2245: a blocked topic tag is not pulled — a gate whose world is a
    /// blocked tag serves nothing, to members included.
    #[test]
    fn test_pull_gate_refuses_every_peer_when_its_world_is_a_blocked_topic_tag() {
        let gate = PullGate::new(
            PullPolicy::Members,
            WORLD,
            live(docs_world(key(1).public(), key(2).public(), false)),
        )
        .with_blocks(super::testing::stub_blocks(&[], &[WORLD]));
        assert!(
            !gate.admits(&key(2).public()),
            "a member of a blocked topic"
        );
        assert!(
            !gate.admits(&key(1).public()),
            "the founder of a blocked topic"
        );
    }

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

    /// A gate names one world. A member of `docs` is not a member of another world.
    #[test]
    fn test_pull_policy_members_checks_only_the_gates_world() {
        let gate = PullGate::new(
            PullPolicy::Members,
            "other",
            live(docs_world(key(1).public(), key(2).public(), false)),
        );
        assert_eq!(gate.world(), "other");
        assert!(!gate.admits(&key(2).public()));
    }
}
