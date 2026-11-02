//! The admission gate (P2.2).
//!
//! The gate decides whether an operation may reach `GroupCrdt::process`. p2panda-auth panics on
//! some inputs that it could have refused: a non-create with no dependencies, and an operation
//! whose group does not exist at its dependencies (`group/crdt/mod.rs`, the `expect` in
//! `apply_action`). The gate turns both into a decision before the crate sees them.
//!
//! The gate does not validate authority. An operation by an author without manage access reaches
//! the crate, which refuses it with a state-change error. Only the crate can say whether the
//! author had authority at the operation's position.

use std::collections::HashSet;

use p2panda_core::Hash;

use super::{AuthOperation, GroupState};

/// Why the gate rejected an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// A group operation other than a create, with no dependencies. It has no position to apply
    /// at, so the crate would panic on it.
    NoDependencies,
    /// The operation's group does not exist at its dependencies. The crate would panic on it.
    GroupAbsent,
    /// A create for a group that a create has already made. A create may name its group (P2.4),
    /// so a second one could add members to a group it did not make. The first create stands.
    GroupExists,
}

/// What the gate decides for one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// The operation may be processed now.
    Admit,
    /// Hold the operation until these dependencies are in the state.
    Wait(Vec<Hash>),
    /// Refuse the operation. The reason is for the void record.
    Reject(Rejection),
}

/// Decides whether `op` may be processed on `state`. Does not change the state.
pub fn admit(state: &GroupState, op: &AuthOperation) -> Admission {
    let missing: Vec<Hash> = op
        .dependencies
        .iter()
        .filter(|dep| !state.inner.operations.contains_key(dep))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Admission::Wait(missing);
    }
    if op.action.is_create() {
        let made = state
            .inner
            .operations
            .values()
            .any(|applied| applied.action.is_create() && applied.group_id == op.group_id);
        return if made {
            Admission::Reject(Rejection::GroupExists)
        } else {
            Admission::Admit
        };
    }
    if op.dependencies.is_empty() {
        return Admission::Reject(Rejection::NoDependencies);
    }
    let dependencies: HashSet<Hash> = op.dependencies.iter().copied().collect();
    match state.inner.state_at(&dependencies) {
        Ok(groups) if groups.contains_key(&op.group_id) => Admission::Admit,
        _ => Admission::Reject(Rejection::GroupAbsent),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use p2panda_auth::Access;
    use p2panda_auth::group::{GroupAction, GroupMember};

    use super::*;
    use crate::membership::{AuthId, Graph};

    fn key(n: u8) -> AuthId {
        AuthId::Key([n; 32])
    }

    fn group(n: u8) -> AuthId {
        AuthId::Group(Hash::digest([n]))
    }

    fn op(id: u8, group_id: AuthId, deps: &[u8], action: GroupAction<AuthId, ()>) -> AuthOperation {
        AuthOperation {
            id: Hash::digest([id]),
            author: key(1),
            dependencies: deps.iter().map(|d| Hash::digest([*d])).collect(),
            group_id,
            action,
        }
    }

    fn create(id: u8, members: &[u8]) -> AuthOperation {
        let initial_members = members
            .iter()
            .map(|m| (GroupMember::Individual(key(*m)), Access::manage()))
            .collect();
        op(id, group(id), &[], GroupAction::Create { initial_members })
    }

    /// The state after creating group `id` with keys 1 and 2 as managers.
    fn state_with_group(id: u8) -> GroupState {
        Graph::process(Graph::init(), &create(id, &[1, 2])).unwrap()
    }

    #[test]
    fn test_a_create_is_admitted_with_no_dependencies() {
        assert_eq!(admit(&Graph::init(), &create(10, &[1])), Admission::Admit);
    }

    #[test]
    fn test_a_non_create_with_no_dependencies_is_rejected() {
        let remove = op(
            11,
            group(10),
            &[],
            GroupAction::Remove {
                member: GroupMember::Individual(key(2)),
            },
        );
        assert_eq!(
            admit(&state_with_group(10), &remove),
            Admission::Reject(Rejection::NoDependencies)
        );
    }

    #[test]
    fn test_a_group_absent_at_its_dependencies_is_rejected() {
        // Group 20 was never created. Its operation names group 10's create, which is present.
        let add = op(
            21,
            group(20),
            &[10],
            GroupAction::Add {
                member: GroupMember::Individual(key(3)),
                access: Access::read(),
            },
        );
        assert_eq!(
            admit(&state_with_group(10), &add),
            Admission::Reject(Rejection::GroupAbsent)
        );
    }

    #[test]
    fn test_missing_dependencies_are_held() {
        let add = op(
            21,
            group(10),
            &[99],
            GroupAction::Add {
                member: GroupMember::Individual(key(3)),
                access: Access::read(),
            },
        );
        assert_eq!(
            admit(&state_with_group(10), &add),
            Admission::Wait(vec![Hash::digest([99])])
        );
    }

    #[test]
    fn test_an_operation_on_a_present_group_is_admitted() {
        let state = state_with_group(10);
        let add = op(
            21,
            group(10),
            &[10],
            GroupAction::Add {
                member: GroupMember::Individual(key(3)),
                access: Access::read(),
            },
        );
        assert_eq!(admit(&state, &add), Admission::Admit);
    }
}
