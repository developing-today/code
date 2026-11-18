//! Group membership on p2panda-auth (Phase 2).
//!
//! p2panda-auth orders group operations by their explicit `dependencies`, and it panics or stalls
//! on a dependency it cannot place. Our events order by `after`, which can name any event. This
//! module bridges the two:
//!
//! - [`frontier`] derives each auth operation's dependencies from its causal past (P2.1). The
//!   result is the maximal group operations of the same group, found by walking `after` through
//!   non-auth events. The stored `after` is never passed to p2panda-auth directly.
//! - [`gate`] decides whether an operation may reach `GroupCrdt::process` (P2.2). It holds an
//!   operation whose dependencies are missing and rejects one whose group does not exist at its
//!   dependencies, which is the case where p2panda-auth panics.
//! - [`layer`] puts the two together. It holds events whose `after` is incomplete, decides each
//!   event once, and leaves a no-op placeholder for an event it refuses, so that its descendants
//!   do not stall.
//!
//! The layer takes events that the caller has already verified (signature, encoding, and
//! `after` closure are the event store's job). It does not read the event store itself.

pub mod body;
pub mod frontier;
pub mod gate;
pub mod layer;

use std::fmt;

use p2panda_auth::group::resolver::StrongRemove;
use p2panda_auth::group::{GroupAction, GroupCrdt, GroupCrdtState};
use p2panda_auth::traits::Operation;
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

pub use layer::{CausalEvent, Membership, MembershipError, Outcome, Refusal, Role};

/// The identity type of p2panda-auth. It names both members and groups, so it is one type with
/// two kinds of value.
///
/// A member is a key. A group is the BLAKE3 hash of its `group.create` event (design §5.5).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, std::hash::Hash, Serialize, Deserialize,
)]
pub enum AuthId {
    /// An author's Ed25519 verifying key.
    Key([u8; 32]),
    /// A group's ID, when it is a member of another group or the target of an operation.
    Group(Hash),
}

impl p2panda_core::traits::Author for AuthId {}

impl fmt::Display for AuthId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => {
                f.write_str("key:")?;
                for byte in key {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
            Self::Group(id) => write!(f, "group:{id}"),
        }
    }
}

/// A group operation as p2panda-auth sees it. The dependencies are the auth frontier, derived by
/// [`frontier`], never copied from the event's `after`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthOperation {
    /// The event's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthId,
    /// The auth frontier: the maximal group operations of this group in the causal past.
    pub dependencies: Vec<Hash>,
    /// The group the action applies to.
    pub group_id: AuthId,
    /// The membership change.
    pub action: GroupAction<AuthId, ()>,
}

impl Operation<AuthId, Hash, ()> for AuthOperation {
    fn id(&self) -> Hash {
        self.id
    }

    fn author(&self) -> AuthId {
        self.author
    }

    fn dependencies(&self) -> Vec<Hash> {
        self.dependencies.clone()
    }

    fn group_id(&self) -> AuthId {
        self.group_id
    }

    fn action(&self) -> GroupAction<AuthId, ()> {
        self.action.clone()
    }
}

/// The p2panda-auth state for every group in one store.
pub type GroupState = GroupCrdtState<AuthId, Hash, AuthOperation, ()>;

/// The p2panda-auth processor, with the strong-remove concurrency rules.
pub type Graph =
    GroupCrdt<AuthId, Hash, AuthOperation, (), StrongRemove<AuthId, Hash, AuthOperation, ()>>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_id_orders_keys_and_groups_apart() {
        let key = AuthId::Key([0; 32]);
        let group = AuthId::Group(Hash::digest(b"group"));
        assert_ne!(key, group);
        assert!(key < group, "keys sort before groups");
        assert!(key.to_string().starts_with("key:"));
        assert!(group.to_string().starts_with("group:"));
    }
}
