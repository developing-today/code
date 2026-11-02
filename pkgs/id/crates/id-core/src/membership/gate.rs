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
    /// so a second one could add members to a group it did not make. Which create stands is
    /// decided by the canonical rule (see [`admit`]), never by arrival order.
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
    /// A concurrent create with the smaller event ID: it displaces the standing create named
    /// here, which loses as if it had been refused with [`Rejection::GroupExists`]. Only the
    /// layer can displace, by re-deciding the events it holds in canonical order.
    Displace(Hash),
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
        let standing = state
            .inner
            .operations
            .values()
            .find(|applied| applied.action.is_create() && applied.group_id == op.group_id);
        let Some(standing) = standing else {
            return Admission::Admit;
        };
        if standing.id == op.id {
            return Admission::Admit;
        }
        // A create that causally follows the standing create is sequential: the group was
        // already made when its author wrote it, so it is refused whatever its ID. The standing
        // create is in this operation's past when it is reachable through its auth
        // dependencies.
        let dependencies: HashSet<Hash> = op.dependencies.iter().copied().collect();
        if reaches(state, &dependencies, standing.id) {
            return Admission::Reject(Rejection::GroupExists);
        }
        // Concurrent creates of one group: the canonical rule decides, as in the groups fold
        // (causal order, ties to the smaller event ID), so every replica crowns the same
        // create whatever order the two arrived in. The smaller ID stands; the larger is
        // refused, or displaces the standing create when it arrives second.
        return if op.id < standing.id {
            Admission::Displace(standing.id)
        } else {
            Admission::Reject(Rejection::GroupExists)
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

/// Whether `target` is reachable from `from` by following operation dependencies in `state`.
fn reaches(state: &GroupState, from: &HashSet<Hash>, target: Hash) -> bool {
    let mut seen: HashSet<Hash> = HashSet::new();
    let mut stack: Vec<Hash> = from.iter().copied().collect();
    while let Some(id) = stack.pop() {
        if id == target {
            return true;
        }
        if seen.insert(id)
            && let Some(applied) = state.inner.operations.get(&id)
        {
            stack.extend(applied.dependencies.iter().copied());
        }
    }
    false
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
    /// A create naming group `group_n`, with event ID `Hash::digest([id])`.
    fn create_named(id: u8, group_n: u8, members: &[u8]) -> AuthOperation {
        let initial_members = members
            .iter()
            .map(|m| (GroupMember::Individual(key(*m)), Access::manage()))
            .collect();
        op(
            id,
            group(group_n),
            &[],
            GroupAction::Create { initial_members },
        )
    }

    /// Two event-ID bytes whose digests order as (smaller, larger).
    fn ordered_pair() -> (u8, u8) {
        if Hash::digest([41]) < Hash::digest([42]) {
            (41, 42)
        } else {
            (42, 41)
        }
    }

    #[test]
    fn test_a_concurrent_create_with_the_larger_id_is_rejected() {
        let (small, large) = ordered_pair();
        let state = Graph::process(Graph::init(), &create_named(small, 30, &[1])).unwrap();
        assert_eq!(
            admit(&state, &create_named(large, 30, &[2])),
            Admission::Reject(Rejection::GroupExists)
        );
    }

    #[test]
    fn test_a_concurrent_create_with_the_smaller_id_displaces() {
        let (small, large) = ordered_pair();
        let state = Graph::process(Graph::init(), &create_named(large, 30, &[2])).unwrap();
        assert_eq!(
            admit(&state, &create_named(small, 30, &[1])),
            Admission::Displace(Hash::digest([large]))
        );
    }

    #[test]
    fn test_a_sequential_create_is_rejected_even_with_the_smaller_id() {
        let (small, large) = ordered_pair();
        let state = Graph::process(Graph::init(), &create_named(large, 30, &[2])).unwrap();
        // The second create names the standing create in its dependencies: it was written
        // after the group was made, so the canonical tie-break never applies.
        let mut sequential = create_named(small, 30, &[1]);
        sequential.dependencies = vec![Hash::digest([large])];
        assert_eq!(
            admit(&state, &sequential),
            Admission::Reject(Rejection::GroupExists)
        );
    }
}

