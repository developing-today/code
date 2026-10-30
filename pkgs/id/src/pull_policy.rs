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
//! This is a config setting, not a protocol rule. The policy belongs with the world's policy. The
//! server config's `[policy]` table parses it ([`crate::server_config::ServerConfig::pull_policy`]),
//! but `id serve` does not read it yet, and no session builds a gate from config. Blobs are not
//! governed here. The blob gate ([`crate::blob_gate`]) still requires membership to fetch, whatever
//! this policy says. Authorized pull is advisory: it stops an honest server from serving a
//! non-member. It does not stop re-serving, recall copies, or hide metadata.

use std::sync::Arc;

use iroh_base::EndpointId;

use crate::blob_gate::WorldAccess;

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
}

impl PullGate {
    /// A gate for `world` under `policy`. `access` answers membership at the time of each check.
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
        }
    }

    /// The world this gate serves.
    #[must_use]
    pub fn world(&self) -> &str {
        &self.world
    }

    /// Whether `peer` may pull this world's events now.
    #[must_use]
    pub fn admits(&self, peer: &EndpointId) -> bool {
        self.policy.admits(self.access.as_ref(), &self.world, peer)
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
                    successor: None,
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
