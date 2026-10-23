//! The `DataScheme` membership: the roster the adapter keeps for the crate.
//!
//! The crate's test membership (`TestDgm`) is not used. The roster follows the control messages the
//! crate processes: `add` and `remove` change it, and `from_welcome` takes the adder's roster from a
//! welcome. The adapter checks it against the group model after each local membership change (see
//! [`super::scheme`]).

use std::collections::{BTreeSet, HashSet};
use std::convert::Infallible;

use p2panda_core::{Hash, VerifyingKey};
use p2panda_encryption::traits::GroupMembership;
use serde::{Deserialize, Serialize};

/// The `DataScheme` membership. A member is its verifying key, and an operation is its envelope hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster;

/// The members one device believes are in the group. Kept sorted, so every device serializes the
/// same members to the same bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterState {
    members: BTreeSet<VerifyingKey>,
}

impl Roster {
    /// An empty roster, before a create or a welcome.
    pub const fn initial_state() -> RosterState {
        RosterState {
            members: BTreeSet::new(),
        }
    }
}

impl GroupMembership<VerifyingKey, Hash> for Roster {
    type State = RosterState;

    type Error = Infallible;

    fn create(
        _my_id: VerifyingKey,
        initial_members: &[VerifyingKey],
    ) -> Result<Self::State, Self::Error> {
        Ok(RosterState {
            members: initial_members.iter().copied().collect(),
        })
    }

    /// The welcome carries the adder's roster from before the add, so it lacks the joiner. The
    /// joiner is put back by the crate's `process_welcome`, in the vendored fork (P4.2).
    fn from_welcome(_my_id: VerifyingKey, y: Self::State) -> Result<Self::State, Self::Error> {
        Ok(y)
    }

    fn add(
        mut y: Self::State,
        _adder: VerifyingKey,
        added: VerifyingKey,
        _operation_id: Hash,
    ) -> Result<Self::State, Self::Error> {
        y.members.insert(added);
        Ok(y)
    }

    fn remove(
        mut y: Self::State,
        _remover: VerifyingKey,
        removed: &VerifyingKey,
        _operation_id: Hash,
    ) -> Result<Self::State, Self::Error> {
        y.members.remove(removed);
        Ok(y)
    }

    fn members(y: &Self::State) -> Result<HashSet<VerifyingKey>, Self::Error> {
        Ok(y.members.iter().copied().collect())
    }
}
