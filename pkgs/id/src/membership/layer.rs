//! The membership layer: decides each causal event once, and holds the events whose past is
//! incomplete.
//!
//! [`Membership::receive`] takes one verified event. An event whose `after` hashes are not all
//! decided is held. An event that is ready is decided: a membership operation is mapped to its
//! auth frontier, passed through the gate, and processed by p2panda-auth. Its outcome is recorded,
//! and every held event it releases is decided in turn.
//!
//! Each decided event is indexed, so a later event can name it in `after`. A refused event
//! becomes a no-op placeholder in the auth graph when a manager stands at its dependencies, so its
//! descendants depend on it and do not stall. Otherwise it is walked through, as a non-auth event.
//!
//! The final state depends only on the set of events, not on the order they arrive in. That is
//! the property the tests in `tests/membership_properties.rs` check.

use std::collections::{HashMap, HashSet, VecDeque};

use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::Hash;
use thiserror::Error;

use super::frontier::{CausalIndex, FrontierError};
use super::gate::{self, Admission, Rejection};
use super::{AuthId, AuthOperation, Graph, GroupState};

/// One event as the membership layer sees it. The caller has verified it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CausalEvent {
    /// The event's ID.
    pub id: Hash,
    /// The event's `after` hashes, as signed.
    pub after: Vec<Hash>,
    /// What the event means for membership.
    pub role: Role,
}

/// What an event means for membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Any event other than a membership operation. It has a causal position and nothing else.
    Other,
    /// A membership operation by `author`. `group` is the group named in the event. A create may
    /// leave it `None`, and its group is then the event's own ID. A friendship's create names its
    /// derived group (design §5.4).
    Auth {
        /// The key that signed the event.
        author: [u8; 32],
        /// The account that actually acted, when `author` is a different identity whose
        /// authority the act exercises (a delegated Manage chain is placed under its root).
        /// `None` means the author acted on its own authority. The layer keys strong-removal
        /// filtering on this account, mirroring how p2panda-auth keys it on the author: a
        /// concurrent removal of the originator voids the operation, whoever it was placed
        /// under.
        originator: Option<[u8; 32]>,
        /// The group the event names, if any.
        group: Option<Hash>,
        /// The membership change.
        action: GroupAction<AuthId, ()>,
    },
}

/// Why the layer refused a membership operation. A refused event changes no membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A non-create with no group.
    Malformed,
    /// The gate rejected the operation before p2panda-auth saw it.
    Gate(Rejection),
    /// p2panda-auth refused the operation at its position. For example, its author lacked manage
    /// access there. The message is the crate's.
    Invalid(String),
}

/// What became of one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Decided and applied. An event with no membership effect is applied too.
    Applied,
    /// Held until these `after` hashes are decided.
    Held(Vec<Hash>),
    /// Refused. `placeholder` says whether a no-op stands in the auth graph in its place.
    Refused {
        /// Why it was refused.
        reason: Refusal,
        /// Whether a placeholder stands in the auth graph.
        placeholder: bool,
    },
    /// Already decided or held. Nothing changed.
    Duplicate,
}

/// A failure of the layer itself. None of these is a refusal of an event; each means the layer
/// cannot keep its invariants.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MembershipError {
    /// The causal index refused a lookup or insert.
    #[error(transparent)]
    Frontier(#[from] FrontierError),
    /// Rebuilding the auth graph from the accepted operations failed. Each one was accepted once,
    /// so this means a broken invariant.
    #[error("rebuilding the auth graph failed at {0}")]
    Rebuild(String),
    /// The gate admitted an operation that still names a missing dependency.
    #[error("internal invariant broken: {0}")]
    Internal(String),
}

/// The membership state of one store: the causal index, the held events, and the auth graph.
#[derive(Debug, Default)]
pub struct Membership {
    index: CausalIndex,
    held: HashMap<Hash, CausalEvent>,
    /// For each `after` hash not yet decided, the held events that name it.
    waiting_on: HashMap<Hash, Vec<Hash>>,
    /// The auth frontier each membership operation was decided with.
    dependencies: HashMap<Hash, Vec<Hash>>,
    state: GroupState,
    /// Every operation accepted into the auth graph, in order. A rebuild replays these.
    accepted: Vec<AuthOperation>,
    /// Membership operations the layer refused. A refused operation takes no effect.
    refused: HashSet<Hash>,
    /// The acting account of each operation placed under a different authority identity,
    /// by operation ID.
    originators: HashMap<Hash, AuthId>,
    /// Delegated operations voided by a concurrent strong removal of their originator.
    /// A voided operation stands in the graph as its placeholder, so its descendants do
    /// not stall, but it takes no effect.
    voided: HashSet<Hash>,
    /// The no-op placeholder standing in the graph for each voided operation, by ID.
    placeholders: HashMap<Hash, AuthOperation>,
    /// The `after` hashes of every decided event, for concurrency checks.
    afters: HashMap<Hash, Vec<Hash>>,
}

impl Membership {
    /// An empty layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one verified event, and returns the outcome of each event it decided, in the order
    /// they were decided. The first entry is the event itself, unless it was a duplicate or was
    /// held. Held events it releases follow it.
    ///
    /// An error means an invariant broke, and the layer's state can no longer be trusted. Discard
    /// the layer after one.
    pub fn receive(&mut self, event: CausalEvent) -> Result<Vec<(Hash, Outcome)>, MembershipError> {
        if self.index.contains(&event.id) || self.held.contains_key(&event.id) {
            return Ok(vec![(event.id, Outcome::Duplicate)]);
        }
        let missing: Vec<Hash> = event
            .after
            .iter()
            .filter(|hash| !self.index.contains(hash))
            .copied()
            .collect();
        if !missing.is_empty() {
            for hash in &missing {
                self.waiting_on.entry(*hash).or_default().push(event.id);
            }
            let id = event.id;
            self.held.insert(id, event);
            return Ok(vec![(id, Outcome::Held(missing))]);
        }

        let mut outcomes = Vec::new();
        let mut ready = VecDeque::from([event]);
        while let Some(next) = ready.pop_front() {
            let id = next.id;
            outcomes.push((id, self.decide(next)?));
            for waiter in self.waiting_on.remove(&id).unwrap_or_default() {
                let Some(held) = self.held.get(&waiter) else {
                    continue;
                };
                if held.after.iter().all(|hash| self.index.contains(hash))
                    && let Some(event) = self.held.remove(&waiter)
                {
                    ready.push_back(event);
                }
            }
        }
        Ok(outcomes)
    }

    /// Decides one event whose `after` hashes are all indexed.
    fn decide(&mut self, event: CausalEvent) -> Result<Outcome, MembershipError> {
        let CausalEvent { id, after, role } = event;
        self.afters.insert(id, after.clone());
        let Role::Auth {
            author,
            group,
            action,
            originator,
        } = role
        else {
            self.index.insert(id, after, None)?;
            return Ok(Outcome::Applied);
        };
        let group = if action.is_create() {
            group.or(Some(id))
        } else {
            group
        };
        let Some(group) = group else {
            self.index.insert(id, after, None)?;
            self.refused.insert(id);
            return Ok(Outcome::Refused {
                reason: Refusal::Malformed,
                placeholder: false,
            });
        };

        let dependencies = self.index.frontier(&after, group)?;
        self.dependencies.insert(id, dependencies.clone());
        let op = AuthOperation {
            id,
            author: AuthId::Key(author),
            dependencies,
            group_id: AuthId::Group(group),
            action,
        };
        if let Some(originator) = originator
            && AuthId::Key(originator) != op.author
        {
            self.originators.insert(id, AuthId::Key(originator));
            // The originator was already removed by an operation concurrent with this
            // one: strong remove voids it, exactly as if it had been placed under the
            // originator itself. It stands in the graph as a placeholder instead.
            if self.concurrently_removed(&op) {
                match self.placeholder_for(&op) {
                    Some(noop) => {
                        self.placeholders.insert(id, noop.clone());
                        self.voided.insert(id);
                        return match self.process(&noop)? {
                            Ok(()) => {
                                self.index.insert(id, after, Some(group))?;
                                Ok(Outcome::Applied)
                            }
                            Err(_) => {
                                self.index.insert(id, after, None)?;
                                Ok(Outcome::Applied)
                            }
                        };
                    }
                    None => {}
                }
            }
        }
        match gate::admit(&self.state, &op) {
            Admission::Admit => {}
            Admission::Wait(missing) => {
                return Err(MembershipError::Internal(format!(
                    "{id} admitted with missing dependencies {missing:?}"
                )));
            }
            Admission::Reject(rejection) => {
                return self.refuse(op, group, after, Refusal::Gate(rejection));
            }
        }
        match self.process(&op)? {
            Ok(()) => {
                self.index.insert(id, after, Some(group))?;
                self.void_concurrent_delegated(&op)?;
                Ok(Outcome::Applied)
            }
            Err(message) => self.refuse(op, group, after, Refusal::Invalid(message)),
        }
    }

    /// Records a refusal. A placeholder stands in the graph when a manager is in the group at the
    /// operation's dependencies. Otherwise the event is walked through.
    fn refuse(
        &mut self,
        op: AuthOperation,
        group: Hash,
        after: Vec<Hash>,
        reason: Refusal,
    ) -> Result<Outcome, MembershipError> {
        let id = op.id;
        self.refused.insert(id);
        let placeholder = match self.manager_at(group, &op.dependencies) {
            Some(target) => {
                // Promoting a member who already manages changes nothing, and the crate does not
                // check the promoter for that case. So this operation is valid whoever wrote it.
                let noop = AuthOperation {
                    action: GroupAction::Promote {
                        member: GroupMember::Individual(target),
                        access: Access::read(),
                    },
                    ..op
                };
                self.process(&noop)?.is_ok()
            }
            None => false,
        };
        self.index.insert(id, after, placeholder.then_some(group))?;
        Ok(Outcome::Refused {
            reason,
            placeholder,
        })
    }

    /// The least active manager of `group` at `dependencies`, or `None` when there is none. The
    /// choice is deterministic, so every peer makes the same placeholder.
    fn manager_at(&self, group: Hash, dependencies: &[Hash]) -> Option<AuthId> {
        let heads: HashSet<Hash> = dependencies.iter().copied().collect();
        Graph::members_at(&self.state, heads, AuthId::Group(group))
            .ok()?
            .into_iter()
            .filter(|(id, access)| matches!(id, AuthId::Key(_)) && access.is_manage())
            .map(|(id, _)| id)
            .min()
    }

    /// Processes `op` on the auth graph and records it as accepted. `process` consumes the state,
    /// so when the crate refuses the operation the state is rebuilt from the accepted operations.
    /// The inner result is the crate's refusal message.
    fn process(&mut self, op: &AuthOperation) -> Result<Result<(), String>, MembershipError> {
        let state = std::mem::take(&mut self.state);
        match Graph::process(state, op) {
            Ok(next) => {
                self.state = next;
                self.accepted.push(op.clone());
                Ok(Ok(()))
            }
            Err(error) => {
                self.state = self.rebuild_state()?;
                Ok(Err(error.to_string()))
            }
        }
    }

    /// The members of `group` and their access, in the current state.
    pub fn members(&self, group: Hash) -> Vec<(AuthId, Access<()>)> {
        self.state.members(AuthId::Group(group))
    }

    /// The auth frontier that the layer derived for a decided membership operation. The stored
    /// `after` of the event is never used as a dependency, so this is what the crate saw.
    pub fn dependencies(&self, id: &Hash) -> Option<&[Hash]> {
        self.dependencies.get(id).map(Vec::as_slice)
    }

    /// Whether the group operation `id` takes effect in the resolved state. It must be accepted at
    /// its position and not refused, the resolver must not filter it, and its author must hold
    /// manage access at its dependencies. The check reads the final state, so an operation stops
    /// taking effect when a later operation makes the resolver filter it.
    pub fn is_effective(&self, id: &Hash) -> bool {
        if self.refused.contains(id) || self.voided.contains(id) {
            return false;
        }
        // The resolver filters the concurrent operations of a removed member, which includes the
        // other half of a mutual removal. A mutual removal still takes effect, so it is exempt.
        let filtered =
            self.state.inner.ignore.contains(id) && !self.state.inner.mutual_removes.contains(id);
        if filtered {
            return false;
        }
        let Some(op) = self.state.inner.operations.get(id) else {
            return false;
        };
        if op.action.is_create() {
            return true;
        }
        let deps: HashSet<Hash> = op.dependencies.iter().copied().collect();
        // Leaving is a removal by the member itself. Any member may leave, so it needs no manage
        // access, only to be a member at its position (the crate checks that, and so does the fold).
        let leaves = matches!(
            &op.action,
            GroupAction::Remove { member: GroupMember::Individual(who) } if *who == op.author
        );
        if leaves {
            return self.access_at(&deps, op.group_id, op.author).is_some();
        }
        self.access_at(&deps, op.group_id, op.author)
            .is_some_and(|access| access.is_manage())
    }

    /// Whether `author` is a member of `group`, with any access, at the causal position that
    /// `after` names. Content is judged by this: it keeps its place while its author holds access.
    ///
    /// # Errors
    ///
    /// Returns [`FrontierError`] when an `after` hash is not indexed.
    pub fn is_member_at(
        &mut self,
        author: [u8; 32],
        group: Hash,
        after: &[Hash],
    ) -> Result<bool, MembershipError> {
        let deps: HashSet<Hash> = self.index.frontier(after, group)?.into_iter().collect();
        Ok(self
            .access_at(&deps, AuthId::Group(group), AuthId::Key(author))
            .is_some())
    }

    /// The access `member` holds in `group` at the causal position that `after` names, or `None`
    /// when it holds none there. Delegation judges grants by this (design §7.2).
    ///
    /// # Errors
    ///
    /// Returns [`MembershipError`] when an `after` hash is not indexed.
    pub fn access_at_position(
        &mut self,
        after: &[Hash],
        group: Hash,
        member: [u8; 32],
    ) -> Result<Option<Access<()>>, MembershipError> {
        let deps: HashSet<Hash> = self.index.frontier(after, group)?.into_iter().collect();
        Ok(self.access_at(&deps, AuthId::Group(group), AuthId::Key(member)))
    }

    /// The same as [`Membership::access_at_position`], borrowing the layer immutably. A caller
    /// that holds the state can ask about a position this way; it copies the frontier index rather
    /// than memoising into it, so prefer the mutable method on a hot path.
    ///
    /// # Errors
    ///
    /// Returns [`MembershipError`] when an `after` hash is not indexed.
    pub fn access_at_position_shared(
        &self,
        after: &[Hash],
        group: Hash,
        member: [u8; 32],
    ) -> Result<Option<Access<()>>, MembershipError> {
        let mut index = self.index.clone();
        let deps: HashSet<Hash> = index.frontier(after, group)?.into_iter().collect();
        Ok(self.access_at(&deps, AuthId::Group(group), AuthId::Key(member)))
    }

    /// The accounts that hold manage access in `group` at the causal position that `after` names.
    /// Empty when the group does not exist there. Delegation takes these as its roots (design
    /// §7.2).
    ///
    /// # Errors
    ///
    /// Returns [`MembershipError`] when an `after` hash is not indexed.
    pub fn managers_at_position(
        &mut self,
        after: &[Hash],
        group: Hash,
    ) -> Result<Vec<[u8; 32]>, MembershipError> {
        let heads: HashSet<Hash> = self.index.frontier(after, group)?.into_iter().collect();
        let managers = Graph::members_at(&self.state, heads, AuthId::Group(group))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, access)| match id {
                AuthId::Key(key) if access.is_manage() => Some(key),
                _ => None,
            })
            .collect();
        Ok(managers)
    }

    /// Whether a create has been decided for `group`.
    pub fn has_group(&self, group: Hash) -> bool {
        self.state.has_group(AuthId::Group(group))
    }

    /// The access `member` holds in `group` at `dependencies`, in the resolved state. The state at
    /// a position is merged from the per-operation states, which the resolver's filter shaped, so
    /// a filtered operation gives no access here.
    fn access_at(
        &self,
        dependencies: &HashSet<Hash>,
        group: AuthId,
        member: AuthId,
    ) -> Option<Access<()>> {
        let groups = self.state.inner.state_at(dependencies).ok()?;
        let members = groups.get(&group)?;
        members
            .access_levels()
            .into_iter()
            .find_map(|(who, access)| (who == GroupMember::Individual(member)).then_some(access))
    }

    /// The member an operation removes, or demotes out of manage access, if any:
    /// the removals p2panda-auth's strong-remove resolver keys its filter on.
    fn removed_member(op: &AuthOperation) -> Option<AuthId> {
        match &op.action {
            GroupAction::Remove { member } => Some(member.id()),
            GroupAction::Demote { member, access } if !access.is_manage() => Some(member.id()),
            _ => None,
        }
    }

    /// Whether `ancestor` is in the causal past of `event`, the event itself included,
    /// walking the recorded `after` hashes through every decided event.
    fn in_past(&self, event: &Hash, ancestor: &Hash) -> bool {
        let mut stack = vec![*event];
        let mut seen = HashSet::new();
        while let Some(next) = stack.pop() {
            if next == *ancestor {
                return true;
            }
            if seen.insert(next)
                && let Some(after) = self.afters.get(&next)
            {
                stack.extend(after.iter().copied());
            }
        }
        false
    }

    /// Whether two decided operations are concurrent: neither is in the other's past.
    fn concurrent(&self, a: &Hash, b: &Hash) -> bool {
        !self.in_past(a, b) && !self.in_past(b, a)
    }

    /// Whether a removal of `op`'s originator already stands accepted, concurrent
    /// with `op`.
    fn concurrently_removed(&self, op: &AuthOperation) -> bool {
        let Some(originator) = self.originators.get(&op.id) else {
            return false;
        };
        self.accepted.iter().any(|other| {
            other.group_id == op.group_id
                && Self::removed_member(other) == Some(*originator)
                && self.concurrent(&other.id, &op.id)
                // A delegated operation that removes the remover in turn is a mutual
                // removal and stands, as in `void_concurrent_delegated`.
                && !matches!(
                    &op.action,
                    GroupAction::Remove { member } if member.id() == other.author
                )
        })
    }

    /// The no-op placeholder for a voided operation, in the shape `refuse` uses: a
    /// promotion of the least manager at the operation's dependencies, which changes
    /// nothing. `None` when no manager stands there.
    fn placeholder_for(&self, op: &AuthOperation) -> Option<AuthOperation> {
        Self::placeholder_in(&self.state, op)
    }

    /// After a removal is applied, void every accepted delegated operation whose
    /// originator it removes and which is concurrent with it, rebuilding the state
    /// with those operations standing as placeholders. A delegated operation that
    /// removes the remover in turn is a mutual removal and stands, as it would if
    /// it had been placed under its originator.
    fn void_concurrent_delegated(
        &mut self,
        removal: &AuthOperation,
    ) -> Result<(), MembershipError> {
        let Some(removed) = Self::removed_member(removal) else {
            return Ok(());
        };
        let mut changed = false;
        let candidates: Vec<AuthOperation> = self
            .accepted
            .iter()
            .filter(|other| {
                other.id != removal.id
                    && other.group_id == removal.group_id
                    && self.originators.get(&other.id) == Some(&removed)
                    && !self.voided.contains(&other.id)
                    && self.concurrent(&other.id, &removal.id)
                    && !matches!(
                        &other.action,
                        GroupAction::Remove { member } if member.id() == removal.author
                    )
            })
            .cloned()
            .collect();
        for candidate in candidates {
            if let Some(noop) = self.placeholder_for(&candidate) {
                self.placeholders.insert(candidate.id, noop);
                self.voided.insert(candidate.id);
                changed = true;
            }
        }
        if changed {
            self.state = self.rebuild_state()?;
        }
        Ok(())
    }

    /// Rebuilds the graph state from the accepted operations, with each voided
    /// operation standing as its placeholder.
    fn rebuild_state(&self) -> Result<GroupState, MembershipError> {
        let ops: Vec<AuthOperation> = self
            .accepted
            .iter()
            .map(|op| {
                self.placeholders
                    .get(&op.id)
                    .cloned()
                    .unwrap_or_else(|| op.clone())
            })
            .collect();
        if self.voided.is_empty() {
            return rebuild(&ops);
        }
        // Voiding an operation can strip a later operation of the authority it was
        // accepted with (a member added by a voided add goes on to act). Replaying
        // such a descendant unchanged fails, and treating that as an invariant
        // failure would make the whole event set unprojectable. Instead the
        // descendant stands as a placeholder too, mirroring the resolver's rule
        // that operations depending on filtered state are not applied.
        // `Graph::process` consumes the state and an error loses it, so each
        // operation is tried as a prefix replay from the last good state.
        let mut state = Graph::init();
        let mut replayed: Vec<AuthOperation> = Vec::new();
        for op in &ops {
            let mut trial = replayed.clone();
            trial.push(op.clone());
            match rebuild(&trial) {
                Ok(next) => {
                    state = next;
                    replayed = trial;
                }
                Err(_) => {
                    if let Some(noop) = Self::placeholder_in(&state, op) {
                        let mut trial = replayed.clone();
                        trial.push(noop);
                        if let Ok(next) = rebuild(&trial) {
                            state = next;
                            replayed = trial;
                        }
                    }
                }
            }
        }
        Ok(state)
    }

    /// The no-op placeholder for `op` against an explicit graph state: a promotion
    /// of the least manager at the operation's dependencies, which changes nothing.
    fn placeholder_in(state: &GroupState, op: &AuthOperation) -> Option<AuthOperation> {
        let AuthId::Group(group) = op.group_id else {
            return None;
        };
        let heads: HashSet<Hash> = op.dependencies.iter().copied().collect();
        let target = Graph::members_at(state, heads, AuthId::Group(group))
            .ok()?
            .into_iter()
            .filter(|(id, access)| matches!(id, AuthId::Key(_)) && access.is_manage())
            .map(|(id, _)| id)
            .min()?;
        Some(AuthOperation {
            action: GroupAction::Promote {
                member: GroupMember::Individual(target),
                access: Access::read(),
            },
            ..op.clone()
        })
    }

    /// Whether the event is held, waiting for an `after` hash.
    pub fn is_held(&self, id: &Hash) -> bool {
        self.held.contains_key(id)
    }

    /// How many events are held.
    pub fn held_len(&self) -> usize {
        self.held.len()
    }
}

/// Replays `accepted` on a fresh graph.
fn rebuild(accepted: &[AuthOperation]) -> Result<GroupState, MembershipError> {
    accepted.iter().try_fold(Graph::init(), |state, op| {
        Graph::process(state, op)
            .map_err(|error| MembershipError::Rebuild(format!("{}: {error}", op.id)))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn key(n: u8) -> [u8; 32] {
        [n; 32]
    }

    fn id(n: u8) -> Hash {
        Hash::digest([n])
    }

    /// An event with no membership effect.
    fn other(n: u8, after: &[u8]) -> CausalEvent {
        CausalEvent {
            id: id(n),
            after: after.iter().map(|a| id(*a)).collect(),
            role: Role::Other,
        }
    }

    /// A membership operation by `author` on group `group` (`None` for a create).
    fn auth(
        n: u8,
        author: u8,
        group: Option<u8>,
        after: &[u8],
        action: GroupAction<AuthId, ()>,
    ) -> CausalEvent {
        CausalEvent {
            id: id(n),
            after: after.iter().map(|a| id(*a)).collect(),
            role: Role::Auth {
                author: key(author),
                group: group.map(id),
                action,
                originator: None,
            },
        }
    }

    fn create(n: u8, author: u8, managers: &[u8]) -> CausalEvent {
        let initial_members = managers
            .iter()
            .map(|m| {
                (
                    GroupMember::Individual(AuthId::Key(key(*m))),
                    Access::manage(),
                )
            })
            .collect();
        auth(
            n,
            author,
            None,
            &[],
            GroupAction::Create { initial_members },
        )
    }

    fn add(member: u8, access: Access<()>) -> GroupAction<AuthId, ()> {
        GroupAction::Add {
            member: GroupMember::Individual(AuthId::Key(key(member))),
            access,
        }
    }

    fn remove(member: u8) -> GroupAction<AuthId, ()> {
        GroupAction::Remove {
            member: GroupMember::Individual(AuthId::Key(key(member))),
        }
    }

    fn outcome_of(outcomes: &[(Hash, Outcome)], n: u8) -> Outcome {
        outcomes
            .iter()
            .find(|(hash, _)| *hash == id(n))
            .map(|(_, outcome)| outcome.clone())
            .unwrap()
    }

    fn member_keys(layer: &Membership, group: u8) -> Vec<AuthId> {
        let mut keys: Vec<AuthId> = layer
            .members(id(group))
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn test_a_create_is_applied_and_its_group_is_its_own_id() {
        let mut layer = Membership::new();
        let outcomes = layer.receive(create(10, 1, &[1, 2])).unwrap();
        assert_eq!(outcomes, vec![(id(10), Outcome::Applied)]);
        assert_eq!(
            member_keys(&layer, 10),
            vec![AuthId::Key(key(1)), AuthId::Key(key(2))]
        );
        assert_eq!(layer.dependencies(&id(10)), Some(&[][..]));
    }

    /// An operation placed under a delegated authority identity, with its originator.
    fn auth_as(
        n: u8,
        author: u8,
        originator: u8,
        group: Option<u8>,
        after: &[u8],
        action: GroupAction<AuthId, ()>,
    ) -> CausalEvent {
        CausalEvent {
            id: id(n),
            after: after.iter().map(|a| id(*a)).collect(),
            role: Role::Auth {
                author: key(author),
                group: group.map(id),
                action,
                originator: Some(key(originator)),
            },
        }
    }

    #[test]
    fn test_a_delegated_operation_is_void_when_its_originator_is_removed_concurrently() {
        // Add first, removal second: the removal voids the delegated add on arrival.
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1])).unwrap();
        layer
            .receive(auth(11, 1, Some(10), &[10], add(2, Access::write())))
            .unwrap();
        layer
            .receive(auth_as(12, 1, 2, Some(10), &[11], add(4, Access::read())))
            .unwrap();
        assert!(member_keys(&layer, 10).contains(&AuthId::Key(key(4))));
        layer
            .receive(auth(13, 1, Some(10), &[11], remove(2)))
            .unwrap();
        assert!(
            !member_keys(&layer, 10).contains(&AuthId::Key(key(4))),
            "strong remove voids the removed member's concurrent delegated operations"
        );
        assert!(!layer.is_effective(&id(12)));
    }

    #[test]
    fn test_a_delegated_operation_after_its_originators_removal_takes_no_effect() {
        // Removal first, delegated add second: the add stands as a placeholder only.
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1])).unwrap();
        layer
            .receive(auth(11, 1, Some(10), &[10], add(2, Access::write())))
            .unwrap();
        layer
            .receive(auth(13, 1, Some(10), &[11], remove(2)))
            .unwrap();
        let outcomes = layer
            .receive(auth_as(12, 1, 2, Some(10), &[11], add(4, Access::read())))
            .unwrap();
        assert_eq!(outcome_of(&outcomes, 12), Outcome::Applied);
        assert!(!member_keys(&layer, 10).contains(&AuthId::Key(key(4))));
        assert!(!layer.is_effective(&id(12)));
    }

    #[test]
    fn test_a_delegated_mutual_removal_stands_in_either_order() {
        // Member 2, acting on member 1's delegated Manage, removes member 3;
        // member 3 concurrently removes member 2. A mutual removal stands in
        // both arrival orders.
        for remove_first in [true, false] {
            let mut layer = Membership::new();
            layer.receive(create(10, 1, &[1, 3])).unwrap();
            layer
                .receive(auth(11, 1, Some(10), &[10], add(2, Access::write())))
                .unwrap();
            let delegated = auth_as(12, 1, 2, Some(10), &[11], remove(3));
            let removal = auth(13, 3, Some(10), &[11], remove(2));
            if remove_first {
                layer.receive(removal).unwrap();
                layer.receive(delegated).unwrap();
            } else {
                layer.receive(delegated).unwrap();
                layer.receive(removal).unwrap();
            }
            assert!(
                layer.is_effective(&id(12)),
                "mutual removal stands (remove_first={remove_first})"
            );
            assert!(!member_keys(&layer, 10).contains(&AuthId::Key(key(3))));
        }
    }

    #[test]
    fn test_a_duplicate_changes_nothing() {
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1])).unwrap();
        let again = layer.receive(create(10, 1, &[1])).unwrap();
        assert_eq!(again, vec![(id(10), Outcome::Duplicate)]);
    }

    #[test]
    fn test_an_event_naming_an_undecided_hash_is_held_until_it_arrives() {
        // Regression: an unknown `after` is held, never treated as a root. Treating it as a root
        // would derive an empty frontier and apply the remove without the add it depends on.
        let mut layer = Membership::new();
        let held = layer
            .receive(auth(12, 1, Some(10), &[99], remove(2)))
            .unwrap();
        assert_eq!(held, vec![(id(12), Outcome::Held(vec![id(99)]))]);
        assert!(layer.is_held(&id(12)));
        assert!(layer.dependencies(&id(12)).is_none());

        layer.receive(create(10, 1, &[1, 2])).unwrap();
        let released = layer.receive(other(99, &[10])).unwrap();
        assert_eq!(released.len(), 2);
        assert_eq!(released[0], (id(99), Outcome::Applied));
        assert_eq!(released[1], (id(12), Outcome::Applied));
        assert_eq!(member_keys(&layer, 10), vec![AuthId::Key(key(1))]);
        assert_eq!(layer.held_len(), 0);
    }

    #[test]
    fn test_held_events_are_released_in_causal_order_whatever_the_arrival_order() {
        let mut layer = Membership::new();
        assert!(matches!(
            layer
                .receive(auth(12, 1, Some(10), &[11], add(3, Access::read())))
                .unwrap()[0]
                .1,
            Outcome::Held(_)
        ));
        assert!(matches!(
            layer
                .receive(auth(11, 1, Some(10), &[10], add(4, Access::read())))
                .unwrap()[0]
                .1,
            Outcome::Held(_)
        ));
        let released = layer.receive(create(10, 1, &[1])).unwrap();
        let order: Vec<Hash> = released.iter().map(|(hash, _)| *hash).collect();
        assert_eq!(order, vec![id(10), id(11), id(12)]);
        assert_eq!(layer.held_len(), 0);
    }

    #[test]
    fn test_the_frontier_names_the_auth_operation_not_the_document_between() {
        // Regression for the literal mapping: a remove whose `after` names a non-auth event must
        // not name that event. p2panda-auth would hold the remove on it forever.
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1, 2, 3])).unwrap();
        layer.receive(other(11, &[10])).unwrap();
        layer
            .receive(auth(12, 1, Some(10), &[11], remove(3)))
            .unwrap();
        assert_eq!(layer.dependencies(&id(12)), Some(&[id(10)][..]));
        assert_eq!(
            member_keys(&layer, 10),
            vec![AuthId::Key(key(1)), AuthId::Key(key(2))]
        );
    }

    #[test]
    fn test_a_refused_event_leaves_a_placeholder_so_descendants_apply() {
        // Key 2 has read access only, so its add is refused by the crate. The placeholder keeps
        // key 1's later add, which names the refused event, from stalling.
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1])).unwrap();
        layer
            .receive(auth(11, 1, Some(10), &[10], add(2, Access::read())))
            .unwrap();
        let refused = layer
            .receive(auth(12, 2, Some(10), &[11], add(3, Access::manage())))
            .unwrap();
        assert!(matches!(
            outcome_of(&refused, 12),
            Outcome::Refused {
                reason: Refusal::Invalid(_),
                placeholder: true
            }
        ));

        let after_placeholder = layer
            .receive(auth(13, 1, Some(10), &[12], add(4, Access::read())))
            .unwrap();
        assert_eq!(outcome_of(&after_placeholder, 13), Outcome::Applied);
        assert_eq!(layer.dependencies(&id(13)), Some(&[id(12)][..]));
        assert_eq!(
            member_keys(&layer, 10),
            vec![
                AuthId::Key(key(1)),
                AuthId::Key(key(2)),
                AuthId::Key(key(4))
            ]
        );
    }

    #[test]
    fn test_a_refused_event_with_no_manager_is_walked_through_so_descendants_do_not_stall() {
        // No manager at all: no placeholder. The descendant names the refused event but depends
        // on the create, and is refused by the crate, not held.
        let mut layer = Membership::new();
        let read_only = GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(AuthId::Key(key(2))), Access::read())],
        };
        layer.receive(auth(10, 1, None, &[], read_only)).unwrap();
        let refused = layer
            .receive(auth(11, 1, Some(10), &[10], remove(2)))
            .unwrap();
        assert!(matches!(
            outcome_of(&refused, 11),
            Outcome::Refused {
                reason: Refusal::Invalid(_),
                placeholder: false
            }
        ));
        let descendant = layer
            .receive(auth(12, 2, Some(10), &[11], add(3, Access::read())))
            .unwrap();
        assert!(matches!(
            outcome_of(&descendant, 12),
            Outcome::Refused {
                reason: Refusal::Invalid(_),
                ..
            }
        ));
        assert_eq!(layer.dependencies(&id(12)), Some(&[id(10)][..]));
        assert!(!layer.is_held(&id(12)));
    }

    #[test]
    fn test_an_operation_for_a_group_absent_at_its_dependencies_is_refused_without_a_panic() {
        // Regression for group/crdt/mod.rs (the expect in apply_action). Group 20 was never
        // created, and its operation names group 10's create. The mapping finds no group-20
        // operation in the past, so the gate refuses it before the crate can panic.
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1])).unwrap();
        let outcomes = layer
            .receive(auth(21, 1, Some(20), &[10], add(3, Access::read())))
            .unwrap();
        assert_eq!(
            outcome_of(&outcomes, 21),
            Outcome::Refused {
                reason: Refusal::Gate(Rejection::NoDependencies),
                placeholder: false,
            }
        );
        assert_eq!(member_keys(&layer, 10), vec![AuthId::Key(key(1))]);
    }

    #[test]
    fn test_a_create_may_name_its_group_so_a_friendship_has_a_derived_id() {
        // A friendship's group is derived from its pair and counter (design §5.4), not from the
        // create's own event ID. The create names that group.
        let mut layer = Membership::new();
        let friendship = id(30);
        let initial_members = [1, 2]
            .iter()
            .map(|n| {
                (
                    GroupMember::Individual(AuthId::Key(key(*n))),
                    Access::manage(),
                )
            })
            .collect();
        let create = CausalEvent {
            id: id(10),
            after: vec![],
            role: Role::Auth {
                author: key(1),
                group: Some(friendship),
                action: GroupAction::Create { initial_members },
                originator: None,
            },
        };
        let outcomes = layer.receive(create).unwrap();
        assert_eq!(outcomes, vec![(id(10), Outcome::Applied)]);
        assert_eq!(
            member_keys(&layer, 30),
            vec![AuthId::Key(key(1)), AuthId::Key(key(2))]
        );
        assert!(layer.members(id(10)).is_empty());
    }

    #[test]
    fn test_a_create_naming_a_group_that_exists_adds_no_members() {
        // Before P2.4 a create's group was its own ID, so no create could reach a group another
        // had made. Now a create names its group, and a stranger's create that names it must not
        // give the stranger access to it.
        let mut layer = Membership::new();
        let made = GroupAction::Create {
            initial_members: vec![
                (
                    GroupMember::Individual(AuthId::Key(key(1))),
                    Access::manage(),
                ),
                (
                    GroupMember::Individual(AuthId::Key(key(2))),
                    Access::manage(),
                ),
            ],
        };
        layer.receive(auth(10, 1, Some(30), &[], made)).unwrap();
        let stranger = GroupAction::Create {
            initial_members: vec![(
                GroupMember::Individual(AuthId::Key(key(9))),
                Access::manage(),
            )],
        };
        let outcomes = layer.receive(auth(11, 9, Some(30), &[], stranger)).unwrap();
        assert!(matches!(
            outcome_of(&outcomes, 11),
            Outcome::Refused {
                reason: Refusal::Gate(Rejection::GroupExists),
                ..
            }
        ));
        assert_eq!(
            member_keys(&layer, 30),
            vec![AuthId::Key(key(1)), AuthId::Key(key(2))]
        );
    }

    #[test]
    fn test_a_non_create_with_no_group_is_refused_as_malformed() {
        let mut layer = Membership::new();
        layer.receive(create(10, 1, &[1])).unwrap();
        let outcomes = layer
            .receive(CausalEvent {
                id: id(11),
                after: vec![id(10)],
                role: Role::Auth {
                    author: key(1),
                    group: None,
                    action: remove(2),
                    originator: None,
                },
            })
            .unwrap();
        assert_eq!(
            outcome_of(&outcomes, 11),
            Outcome::Refused {
                reason: Refusal::Malformed,
                placeholder: false
            }
        );
    }
}
