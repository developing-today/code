//! The fold: a group's state from the whole set of its events (design §4.4 and §5).
//!
//! [`fold`] takes every event and puts them in a canonical order: causal order, with ties going to
//! the smaller event ID. It then applies each event in that order. A rule sees the state built by
//! the events before it, so the Admin check, the member cap, and dissolve are all decided against
//! that prefix. The order depends only on the set of events, so the result does not depend on the
//! order they arrived in.
//!
//! Two concurrent events are ordered by ID, so the lower ID wins a contested place. That is the
//! rule for concurrent adds past the cap (design §5.1). The voiding track's rules for removal may
//! refine how a removal meets a concurrent change.
//!
//! Membership operations go to [`crate::membership`], which decides them on p2panda-auth. Each
//! event is handed to it once, so the causal index stays complete. An event this layer refuses is
//! handed over as a no-op.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::Hash;
use thiserror::Error;

use crate::membership::{
    AuthId, CausalEvent, Membership, MembershipError, Outcome as Decided, Refusal as AuthRefusal,
    Role as Placement,
};

use super::friendship::{Pair, friendship_id};
use super::policy::{OnMemberRemoved, Policy, Template};
use super::{Body, Create, Event, Key, MemberOp, Role};

/// Why an event had no effect. A refused event changes no state. It is still decided, so the
/// events after it can be decided too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The event names no group, or a group that does not exist.
    NoGroup,
    /// The event's group is not the one its kind requires.
    WrongGroup,
    /// The group has been dissolved and takes no further change.
    Dissolved,
    /// The author is not an active member. A friendship's requester is not active until the other
    /// side has accepted.
    NotActive,
    /// The author is not an Admin.
    NotAdmin,
    /// The author may not add or remove members.
    NotManager,
    /// The account named is not a member.
    NotAMember(Key),
    /// The account named is already a member.
    AlreadyMember(Key),
    /// The friendship's ID is not the one its pair and counter give.
    WrongFriendshipId,
    /// A friendship names exactly one other account, and it must not be the author.
    BadFriendship,
    /// The template does not admit the policy record.
    BadTemplate(Template),
    /// The policy record is not usable: its role rule is not Admin, or its cap allows no member.
    BadPolicy,
    /// The members list repeats an account, or names the author.
    BadMembers,
    /// The member cap would be exceeded.
    OverCap {
        /// The cap.
        cap: usize,
    },
    /// Only a friendship's mutual request takes a join in this version. The other join modes are
    /// not built yet.
    JoinNotSupported,
    /// The account has no invitation to this friendship.
    NotInvited,
    /// The account has already accepted.
    AlreadyPresent,
    /// The membership layer refused it at its causal position.
    Membership(AuthRefusal),
}

/// What became of one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It took effect.
    Applied,
    /// It had no effect.
    Refused(Refusal),
}

/// The events cannot be used as a set.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FoldError {
    /// Two events share an ID.
    #[error("event {0} appears twice")]
    DuplicateId(Hash),
    /// An event names an `after` hash that is not in the set.
    #[error("event {op} waits for {dependency}")]
    MissingDependency {
        /// The event that names the missing one.
        op: Hash,
        /// The missing event.
        dependency: Hash,
    },
    /// The `after` links form a cycle.
    #[error("the events form a cycle")]
    Cycle,
    /// The membership layer broke an invariant.
    #[error(transparent)]
    Membership(#[from] MembershipError),
}

/// One group as the events determine it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupView {
    /// The policy record, as last set by an Admin.
    pub policy: Policy,
    /// Each member and its role. An Admin is listed as Admin, whatever its access level.
    pub members: BTreeMap<Key, Role>,
    /// The friendship pair, for a friendship.
    pub friendship: Option<Pair>,
    /// For a friendship, the accounts with an applied event in it.
    pub present: BTreeSet<Key>,
    /// The event that dissolved the group, if it has been dissolved.
    pub dissolved_by: Option<Hash>,
}

impl GroupView {
    /// Whether the group is in effect. A friendship is in effect once both sides are present, and
    /// until it dissolves.
    pub fn is_active(&self) -> bool {
        if self.dissolved_by.is_some() {
            return false;
        }
        match self.friendship {
            None => true,
            Some(pair) => pair.members().iter().all(|key| self.present.contains(key)),
        }
    }
}

/// The state of every group, and what became of each event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    groups: BTreeMap<Hash, GroupView>,
    outcomes: BTreeMap<Hash, Outcome>,
}

impl Projection {
    /// The group with this ID.
    pub fn group(&self, id: &Hash) -> Option<&GroupView> {
        self.groups.get(id)
    }

    /// Every group, by ID.
    pub fn groups(&self) -> impl Iterator<Item = (&Hash, &GroupView)> {
        self.groups.iter()
    }

    /// What became of the event with this ID.
    pub fn outcome(&self, id: &Hash) -> Option<&Outcome> {
        self.outcomes.get(id)
    }

    /// Whether `a` and `b` are friends: an active friendship group names the pair.
    pub fn friends(&self, a: &Key, b: &Key) -> bool {
        Pair::of(*a, *b).is_some_and(|pair| {
            self.groups
                .values()
                .any(|group| group.friendship == Some(pair) && group.is_active())
        })
    }
}

/// Computes the state of every group from the whole set of events. The order the events are given
/// in does not change the result.
pub fn fold(events: &[Event]) -> Result<Projection, FoldError> {
    let by_id = index(events)?;
    let order = canonical_order(&by_id)?;
    let mut engine = Engine::default();
    let mut outcomes = BTreeMap::new();
    for event in order {
        let outcome = engine.apply(event, &by_id)?;
        outcomes.insert(event.id, outcome);
    }
    Ok(engine.into_projection(outcomes))
}

/// The events by ID. Every `after` hash must name one of them.
fn index(events: &[Event]) -> Result<BTreeMap<Hash, &Event>, FoldError> {
    let mut by_id = BTreeMap::new();
    for event in events {
        if by_id.insert(event.id, event).is_some() {
            return Err(FoldError::DuplicateId(event.id));
        }
    }
    for event in events {
        for dependency in &event.after {
            if !by_id.contains_key(dependency) {
                return Err(FoldError::MissingDependency {
                    op: event.id,
                    dependency: *dependency,
                });
            }
        }
    }
    Ok(by_id)
}

/// The events in causal order, with ties going to the smaller ID.
fn canonical_order<'a>(by_id: &BTreeMap<Hash, &'a Event>) -> Result<Vec<&'a Event>, FoldError> {
    let mut waiting: BTreeMap<Hash, usize> = BTreeMap::new();
    let mut children: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
    let mut ready = BinaryHeap::new();
    for event in by_id.values() {
        let dependencies: BTreeSet<Hash> = event.after.iter().copied().collect();
        for dependency in &dependencies {
            children.entry(*dependency).or_default().push(event.id);
        }
        if dependencies.is_empty() {
            ready.push(Reverse(event.id));
        }
        waiting.insert(event.id, dependencies.len());
    }
    let mut order = Vec::with_capacity(by_id.len());
    while let Some(Reverse(id)) = ready.pop() {
        let Some(event) = by_id.get(&id) else {
            continue;
        };
        order.push(*event);
        for child in children.remove(&id).unwrap_or_default() {
            if let Some(count) = waiting.get_mut(&child) {
                *count -= 1;
                if *count == 0 {
                    ready.push(Reverse(child));
                }
            }
        }
    }
    if order.len() == by_id.len() {
        Ok(order)
    } else {
        Err(FoldError::Cycle)
    }
}

/// Every event that the `after` hashes reach through `after` links, the hashes included.
fn causal_past(by_id: &BTreeMap<Hash, &Event>, after: &[Hash]) -> BTreeSet<Hash> {
    let mut past = BTreeSet::new();
    let mut stack: Vec<Hash> = after.to_vec();
    while let Some(id) = stack.pop() {
        if !past.insert(id) {
            continue;
        }
        if let Some(event) = by_id.get(&id) {
            stack.extend(event.after.iter().copied());
        }
    }
    past
}

/// What a group holds beyond its membership layer state.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    policy: Policy,
    admins: BTreeSet<Key>,
    present: BTreeSet<Key>,
    friendship: Option<Pair>,
    dissolved_by: Option<Hash>,
}

/// The state change an accepted event makes. It is committed once the membership layer accepts
/// the event.
#[derive(Debug)]
enum Effect {
    Create(Record),
    Present(Key),
    Policy(Policy),
    Grant {
        member: Key,
        admin: bool,
    },
    Remove {
        member: Key,
        dissolves: bool,
        by: Hash,
    },
}

/// A rule's decision: the membership operation to hand to the membership layer, and the state
/// change to commit if the layer accepts it.
#[derive(Debug)]
struct Decision {
    group: Hash,
    action: Option<GroupAction<AuthId, ()>>,
    effect: Effect,
}

/// The state while the events are applied in canonical order.
#[derive(Debug, Default)]
struct Engine {
    membership: Membership,
    groups: BTreeMap<Hash, Record>,
}

impl Engine {
    /// Applies one event, and returns what became of it.
    fn apply(
        &mut self,
        event: &Event,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Outcome, FoldError> {
        let decided = match &event.body {
            Body::Create(create) => self.create(event, create, by_id),
            Body::Join => self.join(event),
            Body::Policy(policy) => self.policy(event, policy),
            Body::Member(op) => self.member(event, *op),
        };
        let decision = match decided {
            Ok(decision) => decision,
            Err(refusal) => {
                self.decide(event, Placement::Other)?;
                return Ok(Outcome::Refused(refusal));
            }
        };
        let role = match &decision.action {
            Some(action) => Placement::Auth {
                author: event.author,
                group: Some(decision.group),
                action: action.clone(),
            },
            None => Placement::Other,
        };
        match self.decide(event, role)? {
            Decided::Applied => {
                self.commit(decision.group, decision.effect);
                Ok(Outcome::Applied)
            }
            Decided::Refused { reason, .. } => Ok(Outcome::Refused(Refusal::Membership(reason))),
            Decided::Held(_) | Decided::Duplicate => Err(FoldError::Membership(
                MembershipError::Internal(format!("{} was not decided", event.id)),
            )),
        }
    }

    /// Hands one event to the membership layer. The events are applied in causal order, so it
    /// decides at once.
    fn decide(&mut self, event: &Event, role: Placement) -> Result<Decided, FoldError> {
        let decided = self.membership.receive(CausalEvent {
            id: event.id,
            after: event.after.clone(),
            role,
        })?;
        decided
            .into_iter()
            .find(|(id, _)| *id == event.id)
            .map(|(_, outcome)| outcome)
            .ok_or_else(|| {
                FoldError::Membership(MembershipError::Internal(format!(
                    "{} was not decided",
                    event.id
                )))
            })
    }

    fn commit(&mut self, group: Hash, effect: Effect) {
        match effect {
            Effect::Create(record) => {
                self.groups.insert(group, record);
            }
            other => {
                if let Some(record) = self.groups.get_mut(&group) {
                    apply_to(record, other);
                }
            }
        }
    }

    fn create(
        &self,
        event: &Event,
        create: &Create,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Decision, Refusal> {
        if !create.template.admits(&create.policy) {
            return Err(Refusal::BadTemplate(create.template));
        }
        if !create.policy.is_valid() {
            return Err(Refusal::BadPolicy);
        }
        if create.template == Template::Friendship {
            self.friendship_request(event, create, by_id)
        } else {
            new_group(event, create)
        }
    }

    /// A friendship request. Its group is the friendship ID for its pair and the number of
    /// friendships of that pair that the request has seen dissolve. A request for a friendship
    /// that already exists is the other side's acceptance.
    fn friendship_request(
        &self,
        event: &Event,
        create: &Create,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Decision, Refusal> {
        let author = event.author;
        let [(other, _)] = create.members.as_slice() else {
            return Err(Refusal::BadFriendship);
        };
        let other = *other;
        let pair = Pair::of(author, other).ok_or(Refusal::BadFriendship)?;
        let Some(group) = event.group else {
            return Err(Refusal::WrongFriendshipId);
        };
        if let Some(record) = self.groups.get(&group) {
            if record.friendship != Some(pair) {
                return Err(Refusal::WrongGroup);
            }
            return self.accept(group, record, author);
        }
        let counter = self.dissolved_before(&pair, &causal_past(by_id, &event.after));
        if group != friendship_id(&pair, counter) {
            return Err(Refusal::WrongFriendshipId);
        }
        let admins = if create.admins_on_create {
            BTreeSet::from([author, other])
        } else {
            BTreeSet::new()
        };
        let record = Record {
            policy: create.policy.clone(),
            admins,
            present: BTreeSet::from([author]),
            friendship: Some(pair),
            dissolved_by: None,
        };
        let initial = vec![
            (
                GroupMember::Individual(AuthId::Key(author)),
                Access::manage(),
            ),
            (
                GroupMember::Individual(AuthId::Key(other)),
                Access::manage(),
            ),
        ];
        Ok(Decision {
            group,
            action: Some(GroupAction::Create {
                initial_members: initial,
            }),
            effect: Effect::Create(record),
        })
    }

    /// The other side accepts a friendship request. The accepter must be the invitee who has not
    /// yet been present.
    fn accept(&self, group: Hash, record: &Record, author: Key) -> Result<Decision, Refusal> {
        if record.friendship.is_none() {
            return Err(Refusal::JoinNotSupported);
        }
        if record.dissolved_by.is_some() {
            return Err(Refusal::Dissolved);
        }
        if !self.listed(&group, &author) {
            return Err(Refusal::NotInvited);
        }
        if record.present.contains(&author) {
            return Err(Refusal::AlreadyPresent);
        }
        Ok(Decision {
            group,
            action: None,
            effect: Effect::Present(author),
        })
    }

    fn join(&self, event: &Event) -> Result<Decision, Refusal> {
        let group = event.group.ok_or(Refusal::NoGroup)?;
        let record = self.record(&group)?;
        self.accept(group, record, event.author)
    }

    fn policy(&self, event: &Event, policy: &Policy) -> Result<Decision, Refusal> {
        let group = event.group.ok_or(Refusal::NoGroup)?;
        let record = self.record(&group)?;
        let author = event.author;
        if record.dissolved_by.is_some() {
            return Err(Refusal::Dissolved);
        }
        if !self.is_active(&group, record, &author) {
            return Err(Refusal::NotActive);
        }
        if !record.admins.contains(&author) {
            return Err(Refusal::NotAdmin);
        }
        if !policy.is_valid() {
            return Err(Refusal::BadPolicy);
        }
        if let Some(cap) = policy.member_cap
            && self.membership.members(group).len() > cap
        {
            return Err(Refusal::OverCap { cap });
        }
        Ok(Decision {
            group,
            action: None,
            effect: Effect::Policy(policy.clone()),
        })
    }

    fn member(&self, event: &Event, op: MemberOp) -> Result<Decision, Refusal> {
        let group = event.group.ok_or(Refusal::NoGroup)?;
        let record = self.record(&group)?;
        let author = event.author;
        if record.dissolved_by.is_some() {
            return Err(Refusal::Dissolved);
        }
        if !self.is_active(&group, record, &author) {
            return Err(Refusal::NotActive);
        }
        let is_admin = record.admins.contains(&author);
        let is_manager = self
            .access(&group, &author)
            .is_some_and(|role| role >= Role::Manage);
        match op {
            MemberOp::Add { member, role } => {
                if !is_manager {
                    return Err(Refusal::NotManager);
                }
                if role == Role::Admin && !is_admin {
                    return Err(Refusal::NotAdmin);
                }
                if self.listed(&group, &member) {
                    return Err(Refusal::AlreadyMember(member));
                }
                if let Some(cap) = record.policy.member_cap
                    && self.membership.members(group).len() >= cap
                {
                    return Err(Refusal::OverCap { cap });
                }
                Ok(Decision {
                    group,
                    action: Some(GroupAction::Add {
                        member: GroupMember::Individual(AuthId::Key(member)),
                        access: access_for(role),
                    }),
                    effect: Effect::Grant {
                        member,
                        admin: role == Role::Admin,
                    },
                })
            }
            MemberOp::Remove { member } => {
                if !self.listed(&group, &member) {
                    return Err(Refusal::NotAMember(member));
                }
                if member != author && !is_manager {
                    return Err(Refusal::NotManager);
                }
                Ok(Decision {
                    group,
                    action: Some(GroupAction::Remove {
                        member: GroupMember::Individual(AuthId::Key(member)),
                    }),
                    effect: Effect::Remove {
                        member,
                        dissolves: record.policy.on_member_removed == OnMemberRemoved::Dissolve,
                        by: event.id,
                    },
                })
            }
            MemberOp::SetRole { member, role } => {
                if !is_admin {
                    return Err(Refusal::NotAdmin);
                }
                let Some(current) = self.access(&group, &member) else {
                    return Err(Refusal::NotAMember(member));
                };
                let target = access_level(role);
                let action = match target.cmp(&current) {
                    Ordering::Greater => Some(GroupAction::Promote {
                        member: GroupMember::Individual(AuthId::Key(member)),
                        access: access_for(target),
                    }),
                    Ordering::Less => Some(GroupAction::Demote {
                        member: GroupMember::Individual(AuthId::Key(member)),
                        access: access_for(target),
                    }),
                    Ordering::Equal => None,
                };
                Ok(Decision {
                    group,
                    action,
                    effect: Effect::Grant {
                        member,
                        admin: role == Role::Admin,
                    },
                })
            }
        }
    }

    fn record(&self, group: &Hash) -> Result<&Record, Refusal> {
        self.groups.get(group).ok_or(Refusal::NoGroup)
    }

    fn listed(&self, group: &Hash, key: &Key) -> bool {
        self.membership
            .members(*group)
            .iter()
            .any(|(id, _)| *id == AuthId::Key(*key))
    }

    /// The member's role as its access level gives it, or `None` when it is not listed. An Admin
    /// shows as Manage here, because Admin is recorded apart from the access level.
    fn access(&self, group: &Hash, key: &Key) -> Option<Role> {
        self.membership
            .members(*group)
            .iter()
            .find(|(id, _)| *id == AuthId::Key(*key))
            .map(|(_, access)| access_role(access))
    }

    /// Whether the account may act in the group. A friendship's member must be present too.
    fn is_active(&self, group: &Hash, record: &Record, key: &Key) -> bool {
        self.listed(group, key) && (record.friendship.is_none() || record.present.contains(key))
    }

    /// How many friendships of `pair` the events in `past` have dissolved. This is the counter a
    /// new request for the pair must name.
    fn dissolved_before(&self, pair: &Pair, past: &BTreeSet<Hash>) -> u32 {
        let count = self
            .groups
            .values()
            .filter(|record| {
                record.friendship == Some(*pair)
                    && record.dissolved_by.is_some_and(|by| past.contains(&by))
            })
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn into_projection(self, outcomes: BTreeMap<Hash, Outcome>) -> Projection {
        let membership = &self.membership;
        let groups = self
            .groups
            .into_iter()
            .map(|(group, record)| {
                let members: BTreeMap<Key, Role> = membership
                    .members(group)
                    .into_iter()
                    .filter_map(|(id, access)| match id {
                        AuthId::Key(key) if record.admins.contains(&key) => {
                            Some((key, Role::Admin))
                        }
                        AuthId::Key(key) => Some((key, access_role(&access))),
                        AuthId::Group(_) => None,
                    })
                    .collect();
                let view = GroupView {
                    policy: record.policy,
                    members,
                    friendship: record.friendship,
                    present: record.present,
                    dissolved_by: record.dissolved_by,
                };
                (group, view)
            })
            .collect();
        Projection { groups, outcomes }
    }
}

/// Applies a state change to a group that exists.
fn apply_to(record: &mut Record, effect: Effect) {
    match effect {
        Effect::Create(_) => {}
        Effect::Present(member) => {
            record.present.insert(member);
        }
        Effect::Policy(policy) => {
            record.policy = policy;
        }
        Effect::Grant { member, admin } => {
            if admin {
                record.admins.insert(member);
            } else {
                record.admins.remove(&member);
            }
        }
        Effect::Remove {
            member,
            dissolves,
            by,
        } => {
            record.admins.remove(&member);
            record.present.remove(&member);
            if dissolves && record.dissolved_by.is_none() {
                record.dissolved_by = Some(by);
            }
        }
    }
}

/// A normal group. Its ID is its create's event ID, and its creator is Admin.
fn new_group(event: &Event, create: &Create) -> Result<Decision, Refusal> {
    let author = event.author;
    if create.admins_on_create {
        return Err(Refusal::BadTemplate(create.template));
    }
    if event.group.is_some() {
        return Err(Refusal::WrongGroup);
    }
    if create.members.iter().any(|(key, _)| *key == author) || has_repeats(&create.members) {
        return Err(Refusal::BadMembers);
    }
    if let Some(cap) = create.policy.member_cap
        && create.members.len() >= cap
    {
        return Err(Refusal::OverCap { cap });
    }
    let mut initial = vec![(
        GroupMember::Individual(AuthId::Key(author)),
        Access::manage(),
    )];
    let mut admins = BTreeSet::from([author]);
    for (member, role) in &create.members {
        initial.push((
            GroupMember::Individual(AuthId::Key(*member)),
            access_for(*role),
        ));
        if *role == Role::Admin {
            admins.insert(*member);
        }
    }
    let record = Record {
        policy: create.policy.clone(),
        admins,
        present: BTreeSet::new(),
        friendship: None,
        dissolved_by: None,
    };
    Ok(Decision {
        group: event.id,
        action: Some(GroupAction::Create {
            initial_members: initial,
        }),
        effect: Effect::Create(record),
    })
}

fn has_repeats(members: &[(Key, Role)]) -> bool {
    let distinct: BTreeSet<Key> = members.iter().map(|(key, _)| *key).collect();
    distinct.len() != members.len()
}

/// The role a member's access level gives. Admin is never an access level.
fn access_role(access: &Access<()>) -> Role {
    if access.is_manage() {
        Role::Manage
    } else if access.is_write() {
        Role::Write
    } else if access.is_read() {
        Role::Read
    } else {
        Role::Pull
    }
}

/// The access level a role needs. Admin is a manager that is also recorded as Admin.
const fn access_level(role: Role) -> Role {
    match role {
        Role::Admin => Role::Manage,
        other => other,
    }
}

fn access_for(role: Role) -> Access<()> {
    match access_level(role) {
        Role::Pull => Access::pull(),
        Role::Read => Access::read(),
        Role::Write => Access::write(),
        Role::Manage | Role::Admin => Access::manage(),
    }
}
