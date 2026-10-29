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
//!
//! Delegation (P2.5) is judged in [`super::delegation`]. Whether a grant or a removal takes effect
//! can depend on a concurrent event (strong remove), so the fold runs in rounds. Each round judges
//! the events against the set that took effect in the round before, and the fold stops when a
//! round reproduces its own set. The set depends only on the events, so the result does too.

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
use crate::rejection::Reason;

use super::delegation::{Grant, Removal, removed_member};
use super::friendship::{Pair, friendship_id};
use super::policy::{OnMemberRemoved, Policy, Removal as RemovalMode, Template};
use super::{Body, Create, Delegate, DeviceOp, Event, Key, MemberOp, Role};

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
    /// The author does not hold the scope at the grant's position, so it may not grant it
    /// (design §7.2, attenuation).
    Attenuated,
    /// The author holds no chain of grants to the scope it needs (design §7.2). A manager acts
    /// only through a grant that reaches it from an owner.
    NoChain,
    /// A grant may not name Admin. Admin is a policy right, not a scope.
    BadScope,
    /// A grant may not name its own author.
    BadGrantee,
    /// A veto names no grant of this group that the veto has seen.
    UnknownGrant,
    /// A grant that names another world may carry Write at most, and must name a world.
    NotCrossable,
}

impl Refusal {
    /// The reason this refusal gives. A friendship's requester is waiting for the other side, and a
    /// group that does not exist yet is waiting for its create. A dissolved group is frozen by the
    /// removal that dissolved it. Every other refusal is a rule of the group, so it voids the event.
    pub const fn reason(&self) -> Reason {
        match self {
            Self::NoGroup | Self::NotActive => Reason::Waiting,
            Self::Dissolved => Reason::Frozen,
            Self::WrongGroup
            | Self::NotAdmin
            | Self::NotManager
            | Self::NotAMember(_)
            | Self::AlreadyMember(_)
            | Self::WrongFriendshipId
            | Self::BadFriendship
            | Self::BadTemplate(_)
            | Self::BadPolicy
            | Self::BadMembers
            | Self::OverCap { .. }
            | Self::JoinNotSupported
            | Self::NotInvited
            | Self::AlreadyPresent
            | Self::Membership(_)
            | Self::Attenuated
            | Self::NoChain
            | Self::BadScope
            | Self::BadGrantee
            | Self::UnknownGrant
            | Self::NotCrossable => Reason::Voided,
        }
    }
}

/// What became of one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It took effect.
    Applied,
    /// It had no effect.
    Refused(Refusal),
}

impl Outcome {
    /// The reason the event had no effect, or `None` when it took effect.
    pub const fn reason(&self) -> Option<Reason> {
        match self {
            Self::Applied => None,
            Self::Refused(refusal) => Some(refusal.reason()),
        }
    }
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
    /// The grants, vetoes and removals that take effect do not settle on one set. Each round
    /// judges the events against the set of the round before, and the sets keep changing.
    #[error("the events that take effect do not settle")]
    Unsettled,
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

/// An authorization of a device by an account that took effect (P8.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceGrant {
    /// The event ID.
    pub id: Hash,
    /// The group the authorization was made in.
    pub group: Hash,
    /// The authorizing account.
    pub account: Key,
    /// The authorized device.
    pub device: Key,
    /// The authorized scope.
    pub scope: Role,
}

/// A revocation of a device authorization (P8.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRevokeRecord {
    /// The revocation event ID.
    pub id: Hash,
    /// The group.
    pub group: Hash,
    /// The revoking account.
    pub account: Key,
    /// The revoked device.
    pub device: Key,
}

/// The state of every group, and what became of each event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    groups: BTreeMap<Hash, GroupView>,
    outcomes: BTreeMap<Hash, Outcome>,
    effective: BTreeSet<Hash>,
    in_force: BTreeSet<Hash>,
    grants: Vec<Grant>,
    devices: Vec<DeviceGrant>,
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

    /// Whether the event takes effect in the settled state. A grant, a veto or a membership
    /// operation takes effect when it was applied and p2panda-auth's resolver does not filter it.
    /// An event that was applied but filtered does not take effect.
    pub fn takes_effect(&self, id: &Hash) -> bool {
        self.effective.contains(id)
    }

    /// Whether the grant with this event ID is in force at the head of the events (design §7.3).
    /// A concurrent removal of its grantor voids it under strong remove; weak remove stands it. A
    /// veto cuts it, and the group's mode decides the rest. The chain above the grant is not
    /// checked here. Use the outcomes of the actions it was meant to allow for that.
    pub fn grant_in_force(&self, id: &Hash) -> bool {
        self.in_force.contains(id)
    }

    /// Every grant in force at the head of the events, with its grantor, grantee, scope and
    /// target world. See [`Grant`].
    pub fn grants_in_force(&self) -> impl Iterator<Item = &Grant> {
        self.grants.iter()
    }

    /// The active authorized devices for `account` in `group` at the head of the events (P8.3).
    /// Following cascade rules, a removed account has no active devices.
    pub fn active_devices(&self, group: &Hash, account: &Key) -> BTreeSet<Key> {
        let Some(view) = self.groups.get(group) else {
            return BTreeSet::new();
        };
        if !view.members.contains_key(account) {
            return BTreeSet::new();
        }
        self.devices
            .iter()
            .filter(|d| d.group == *group && d.account == *account)
            .map(|d| d.device)
            .collect()
    }

    /// Count of active authorized devices for `account` in `group` (P8.3).
    pub fn active_device_count(&self, group: &Hash, account: &Key) -> usize {
        self.active_devices(group, account).len()
    }

    /// All active authorized devices across all members in `group` at the head of the events (P8.3).
    pub fn group_active_devices(&self, group: &Hash) -> BTreeSet<Key> {
        let Some(view) = self.groups.get(group) else {
            return BTreeSet::new();
        };
        self.devices
            .iter()
            .filter(|d| d.group == *group && view.members.contains_key(&d.account))
            .map(|d| d.device)
            .collect()
    }

    /// Total count of active authorized devices across all members in `group` (P8.3).
    pub fn group_device_count(&self, group: &Hash) -> usize {
        self.group_active_devices(group).len()
    }

    /// Count of active authorized devices for `account` across all active groups (P8.3).
    pub fn account_device_count(&self, account: &Key) -> usize {
        let mut devices = BTreeSet::new();
        for (group, view) in &self.groups {
            if view.members.contains_key(account) {
                for d in &self.devices {
                    if d.group == *group && d.account == *account {
                        devices.insert(d.device);
                    }
                }
            }
        }
        devices.len()
    }
}

/// Computes the state of every group from the whole set of events. The order the events are given
/// in does not change the result.
pub fn fold(events: &[Event]) -> Result<Projection, FoldError> {
    let by_id = index(events)?;
    let order = canonical_order(&by_id)?;
    // The events whose effect depends on the settled set: grants, vetoes, and membership changes.
    let candidates: BTreeSet<Hash> = by_id
        .values()
        .filter(|event| is_candidate(event))
        .map(|event| event.id)
        .collect();
    let rounds = candidates.len() + 1;
    let mut settled = candidates;
    for _ in 0..=rounds {
        let (projection, next) = run(&order, &by_id, &settled)?;
        if next == settled {
            return Ok(projection);
        }
        settled = next;
    }
    Err(FoldError::Unsettled)
}

/// One round: every event judged against `settled`. Returns the projection and the set of
/// candidate events that took effect in this round.
fn run(
    order: &[&Event],
    by_id: &BTreeMap<Hash, &Event>,
    settled: &BTreeSet<Hash>,
) -> Result<(Projection, BTreeSet<Hash>), FoldError> {
    let mut engine = Engine::new(settled.clone());
    let mut outcomes = BTreeMap::new();
    for &event in order {
        let outcome = engine.apply(event, by_id)?;
        outcomes.insert(event.id, outcome);
    }
    let effective = engine.effective();
    let in_force = engine.in_force(by_id)?;
    let projection = engine.into_projection(outcomes, effective.clone(), in_force, by_id);
    Ok((projection, effective))
}

/// Whether the event's effect depends on the settled set.
const fn is_candidate(event: &Event) -> bool {
    matches!(
        event.body,
        Body::Delegate(_)
            | Body::Cross { .. }
            | Body::Veto { .. }
            | Body::Member(_)
            | Body::Device(_)
    )
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
pub(super) fn causal_past(by_id: &BTreeMap<Hash, &Event>, after: &[Hash]) -> BTreeSet<Hash> {
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
pub(super) struct Record {
    pub(super) policy: Policy,
    pub(super) admins: BTreeSet<Key>,
    pub(super) present: BTreeSet<Key>,
    pub(super) friendship: Option<Pair>,
    pub(super) dissolved_by: Option<Hash>,
}

/// The state change an accepted event makes. It is committed once the membership layer accepts
/// the event.
#[derive(Debug)]
pub(super) enum Effect {
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
        successor: Option<Key>,
        by: Hash,
    },
    Delegation {
        delegate: Delegate,
        target: Option<String>,
    },
    Veto(Hash),
    DeviceAdd {
        account: Key,
        device: Key,
        scope: Role,
    },
    DeviceRevoke {
        account: Key,
        device: Key,
    },
}

/// A rule's decision: the membership operation to hand to the membership layer, and the state
/// change to commit if the layer accepts it.
#[derive(Debug)]
pub(super) struct Decision {
    pub(super) group: Hash,
    pub(super) action: Option<GroupAction<AuthId, ()>>,
    pub(super) effect: Effect,
}

/// A rule's verdict: a decision, or the reason it has no effect.
pub(super) type Judged = Result<Decision, Refusal>;

/// Placement of an event in the membership layer, as a label for the layer.
type Placed = Placement;

/// The state while the events are applied in canonical order.
#[derive(Debug, Default)]
pub(super) struct Engine {
    pub(super) membership: Membership,
    pub(super) groups: BTreeMap<Hash, Record>,
    /// The candidate events that took effect in the previous round.
    pub(super) settled: BTreeSet<Hash>,
    /// The candidate events applied in this round.
    applied: BTreeSet<Hash>,
    /// The applied candidates that are membership operations in the auth graph.
    placed: BTreeSet<Hash>,
    /// The grants applied in this round, by event ID.
    pub(super) grants: BTreeMap<Hash, Grant>,
    /// The vetoes applied in this round, each with the grant it vetoes.
    pub(super) vetoes: BTreeMap<Hash, Hash>,
    /// The removals applied in this round, by event ID.
    pub(super) removals: BTreeMap<Hash, Removal>,
    /// The device authorizations applied in this round, by event ID.
    pub(super) devices: BTreeMap<Hash, DeviceGrant>,
    /// The device revocations applied in this round, by event ID.
    pub(super) device_revocations: BTreeMap<Hash, DeviceRevokeRecord>,
}

impl Engine {
    fn new(settled: BTreeSet<Hash>) -> Self {
        Self {
            settled,
            ..Self::default()
        }
    }

    /// Applies one event, and returns what became of it.
    fn apply(
        &mut self,
        event: &Event,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Outcome, FoldError> {
        let decision = match self.judge(event, by_id)? {
            Ok(decision) => decision,
            Err(refusal) => {
                self.decide(event, Placed::Other)?;
                return Ok(Outcome::Refused(refusal));
            }
        };
        let effective_author = match self.effective_author(by_id, decision.group, event) {
            Some((author, _)) => author,
            None => event.author,
        };
        let role = match &decision.action {
            Some(action) => Placed::Auth {
                author: effective_author,
                group: Some(decision.group),
                action: action.clone(),
            },
            None => Placed::Other,
        };
        match self.decide(event, role)? {
            Decided::Applied => {
                if is_candidate(event) {
                    self.applied.insert(event.id);
                    if decision.action.is_some() {
                        self.placed.insert(event.id);
                    }
                }
                self.commit(event, decision.group, decision.effect);
                Ok(Outcome::Applied)
            }
            Decided::Refused { reason, .. } => Ok(Outcome::Refused(Refusal::Membership(reason))),
            Decided::Held(_) | Decided::Duplicate => Err(FoldError::Membership(
                MembershipError::Internal(format!("{} was not decided", event.id)),
            )),
        }
    }

    /// The verdict on one event, before the membership layer sees it.
    fn judge(
        &mut self,
        event: &Event,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Judged, FoldError> {
        match &event.body {
            Body::Create(create) => Ok(self.create(event, create, by_id)),
            Body::Join => Ok(self.join(event)),
            Body::Policy(policy) => Ok(self.policy(event, policy, by_id)),
            Body::Member(op) => match self.member(event, *op, by_id) {
                Ok(decision) => self.manage_chain(by_id, event, *op, decision),
                Err(refusal) => Ok(Err(refusal)),
            },
            Body::Delegate(delegate) => self.delegate(by_id, event, *delegate, None),
            Body::Cross { delegate, world } => self.delegate(by_id, event, *delegate, Some(world)),
            Body::Veto { grant } => self.veto(by_id, event, *grant),
            Body::Device(op) => self.device(by_id, event, *op),
        }
    }

    /// Hands one event to the membership layer. The events are applied in causal order, so it
    /// decides at once.
    fn decide(&mut self, event: &Event, role: Placed) -> Result<Decided, FoldError> {
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

    fn commit(&mut self, event: &Event, group: Hash, effect: Effect) {
        match effect {
            Effect::Create(record) => {
                self.groups.insert(group, record);
            }
            Effect::Delegation { delegate, target } => {
                let grant = Grant {
                    id: event.id,
                    group,
                    from: event.author,
                    to: delegate.grantee,
                    scope: delegate.scope,
                    redelegate: delegate.can_redelegate,
                    target,
                };
                self.grants.insert(event.id, grant);
            }
            Effect::Veto(grant) => {
                self.vetoes.insert(event.id, grant);
            }
            Effect::Remove {
                member,
                dissolves,
                successor,
                by,
            } => {
                let removal = Removal {
                    id: by,
                    group,
                    member,
                    successor,
                    after: event.after.clone(),
                };
                self.removals.insert(by, removal);
                if let Some(record) = self.groups.get_mut(&group) {
                    apply_to(
                        record,
                        Effect::Remove {
                            member,
                            dissolves,
                            successor,
                            by,
                        },
                    );
                }
            }
            Effect::DeviceAdd {
                account,
                device,
                scope,
            } => {
                let grant = DeviceGrant {
                    id: event.id,
                    group,
                    account,
                    device,
                    scope,
                };
                self.devices.insert(event.id, grant);
            }
            Effect::DeviceRevoke { account, device } => {
                let revoke = DeviceRevokeRecord {
                    id: event.id,
                    group,
                    account,
                    device,
                };
                self.device_revocations.insert(event.id, revoke);
            }
            other => {
                if let Some(record) = self.groups.get_mut(&group) {
                    apply_to(record, other);
                }
            }
        }
    }

    /// The candidate events that took effect: applied, and for a membership operation not filtered
    /// by p2panda-auth's resolver in the final state.
    fn effective(&self) -> BTreeSet<Hash> {
        self.applied
            .iter()
            .copied()
            .filter(|id| !self.placed.contains(id) || self.membership.is_effective(id))
            .collect()
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

    fn policy(
        &self,
        event: &Event,
        policy: &Policy,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Decision, Refusal> {
        let group = event.group.ok_or(Refusal::NoGroup)?;
        let record = self.record(&group)?;
        if record.dissolved_by.is_some() {
            return Err(Refusal::Dissolved);
        }
        let Some((_author, author_role)) = self.effective_author(by_id, group, event) else {
            return Err(Refusal::NotActive);
        };
        if author_role < Role::Admin {
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

    fn member(
        &self,
        event: &Event,
        op: MemberOp,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Decision, Refusal> {
        let group = event.group.ok_or(Refusal::NoGroup)?;
        let record = self.record(&group)?;
        if record.dissolved_by.is_some() {
            return Err(Refusal::Dissolved);
        }
        let Some((author, author_role)) = self.effective_author(by_id, group, event) else {
            return Err(Refusal::NotActive);
        };
        let is_admin = author_role == Role::Admin;
        let is_manager = author_role >= Role::Manage;
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
            MemberOp::Remove { member, successor } => {
                if !self.listed(&group, &member) {
                    return Err(Refusal::NotAMember(member));
                }
                if member != author && !is_manager {
                    return Err(Refusal::NotManager);
                }
                // Manage removes members at or below its own level. Admin is above it (design §5.2).
                if member != author && record.admins.contains(&member) && !is_admin {
                    return Err(Refusal::NotAdmin);
                }
                Ok(Decision {
                    group,
                    action: Some(GroupAction::Remove {
                        member: GroupMember::Individual(AuthId::Key(member)),
                    }),
                    effect: Effect::Remove {
                        member,
                        dissolves: record.policy.on_member_removed == OnMemberRemoved::Dissolve,
                        successor,
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

    pub(super) fn record(&self, group: &Hash) -> Result<&Record, Refusal> {
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
    pub(super) fn access(&self, group: &Hash, key: &Key) -> Option<Role> {
        self.membership
            .members(*group)
            .iter()
            .find(|(id, _)| *id == AuthId::Key(*key))
            .map(|(_, access)| access_role(access))
    }

    /// Whether the account may act in the group. A friendship's member must be present too.
    pub(super) fn is_active(&self, group: &Hash, record: &Record, key: &Key) -> bool {
        self.listed(group, key) && (record.friendship.is_none() || record.present.contains(key))
    }

    /// Whether `key` is an active member of `group` at the causal position `after`: listed there,
    /// and present there for a friendship. Weak remove judges an event's authority at its own
    /// position, so an event concurrent with a removal keeps the standing its author had there
    /// (design §6.3). This is the slow path: it does not use the membership layer's frontier memo.
    fn is_active_at(&self, group: &Hash, record: &Record, key: &Key, after: &[Hash]) -> bool {
        let listed = self
            .membership
            .access_at_position_shared(after, *group, *key)
            .ok()
            .flatten()
            .is_some();
        listed && (record.friendship.is_none() || record.present.contains(key))
    }

    /// Whether `key` is active in `group` for an event at the causal position `after`. Strong remove
    /// reads the current state, which is what it has always done; weak remove reads the position
    /// (design §6.3).
    fn is_active_for(&self, group: &Hash, record: &Record, key: &Key, after: &[Hash]) -> bool {
        if self.strong_removal(*group) {
            self.is_active(group, record, key)
        } else {
            self.is_active_at(group, record, key, after)
        }
    }

    /// The member's role in `group` at the causal position `after`, or `None` when it holds none
    /// there. Strong remove reads the current state; weak remove reads the position, so an event
    /// concurrent with a removal keeps its author's role there (design §6.3).
    fn access_for(&self, group: &Hash, key: &Key, after: &[Hash]) -> Option<Role> {
        if self.strong_removal(*group) {
            return self.access(group, key);
        }
        self.membership
            .access_at_position_shared(after, *group, *key)
            .ok()
            .flatten()
            .map(|access| access_role(&access))
            .or_else(|| self.access(group, key))
    }

    /// Whether `key` is listed in `group` for an event at the causal position `after`. Strong
    /// remove reads the current membership; weak remove reads the position, so a device grant
    /// whose account is removed concurrently still resolves (design §6.3).
    fn listed_for(&self, group: &Hash, key: &Key, after: &[Hash]) -> bool {
        if self.strong_removal(*group) {
            return self.listed(group, key);
        }
        self.membership
            .access_at_position_shared(after, *group, *key)
            .ok()
            .flatten()
            .is_some()
    }

    /// Whether `key` is an Admin of `group` for an event at the causal position `after`. Strong
    /// remove reads the current record; weak remove also honours the Admin status the key held
    /// at the position, replayed from the settled events in the event's causal past in canonical
    /// order — a removal concurrent with the event has already struck the key from the current
    /// record's admins, and the membership layer cannot recover the status because it records
    /// Admin as Manage (design §6.3).
    fn admin_for(
        &self,
        by_id: &BTreeMap<Hash, &Event>,
        group: &Hash,
        key: &Key,
        after: &[Hash],
    ) -> bool {
        let record = self.groups.get(group);
        if self.strong_removal(*group) {
            return record.is_some_and(|record| record.admins.contains(key));
        }
        if record.is_some_and(|record| record.admins.contains(key)) {
            return true;
        }
        let past = causal_past(by_id, after);
        let Ok(order) = canonical_order(by_id) else {
            return false;
        };
        let mut admin: Option<bool> = None;
        for event in order {
            if !past.contains(&event.id) {
                continue;
            }
            // The group's creation is never in the settled set (it is not a candidate
            // event), but it seeds the replay: an Admin assigned at creation has no
            // MemberOp::Add to recover the status from.
            let is_creation = event.id == *group && matches!(event.body, Body::Create(_));
            if !is_creation && !self.settled.contains(&event.id) {
                continue;
            }
            match &event.body {
                Body::Create(create) if event.id == *group => {
                    admin = Some(if create.admins_on_create {
                        event.author == *key
                            || create
                                .members
                                .first()
                                .is_some_and(|(member, _)| member == key)
                    } else {
                        event.author == *key
                            || create
                                .members
                                .iter()
                                .any(|(member, role)| member == key && *role == Role::Admin)
                    });
                }
                Body::Member(MemberOp::Add { member, role })
                | Body::Member(MemberOp::SetRole { member, role })
                    if member == key && event.group == Some(*group) =>
                {
                    admin = Some(*role == Role::Admin);
                }
                Body::Member(MemberOp::Remove { member, .. })
                    if member == key && event.group == Some(*group) =>
                {
                    admin = Some(false);
                }
                _ => {}
            }
        }
        admin.unwrap_or(false)
    }

    /// Resolves whether `actor` is an active authorized device for an account in `group` at `after`.
    /// Returns `Some((account, authorized_scope))` if so.
    pub(super) fn authorized_device_account(
        &self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        actor: Key,
        after: &[Hash],
    ) -> Option<(Key, Role)> {
        let past = causal_past(by_id, after);
        for grant in self.devices.values() {
            if grant.group != group || grant.device != actor {
                continue;
            }
            if !self.settled.contains(&grant.id) || !past.contains(&grant.id) {
                continue;
            }
            // Check if revoked in past
            let revoked = self.device_revocations.values().any(|rev| {
                rev.group == group
                    && rev.account == grant.account
                    && rev.device == actor
                    && self.settled.contains(&rev.id)
                    && past.contains(&rev.id)
                    && causal_past(by_id, &[rev.id]).contains(&grant.id)
            });
            if revoked {
                continue;
            }
            // Cascade check: grantor account must still be an active member of group at `after`
            let account_removed = self.removals.values().any(|rem| {
                rem.group == group
                    && rem.member == grant.account
                    && self.settled.contains(&rem.id)
                    && past.contains(&rem.id)
                    && causal_past(by_id, &[rem.id]).contains(&grant.id)
            });
            if account_removed {
                continue;
            }
            if !self.listed_for(&group, &grant.account, after) {
                continue;
            }
            if self.void_device(by_id, grant) {
                continue;
            }
            return Some((grant.account, grant.scope));
        }
        None
    }

    /// Resolves the effective author and maximum role for `event` in `group`.
    pub(super) fn effective_author(
        &self,
        by_id: &BTreeMap<Hash, &Event>,
        group: Hash,
        event: &Event,
    ) -> Option<(Key, Role)> {
        let record = self.groups.get(&group)?;
        if self.is_active_for(&group, record, &event.author, &event.after) {
            let role = if self.admin_for(by_id, &group, &event.author, &event.after) {
                Role::Admin
            } else {
                self.access_for(&group, &event.author, &event.after)
                    .unwrap_or(Role::Pull)
            };
            Some((event.author, role))
        } else if let Some((account, device_scope)) =
            self.authorized_device_account(by_id, group, event.author, &event.after)
        {
            if !self.is_active_for(&group, record, &account, &event.after) {
                return None;
            }
            let account_role = if self.admin_for(by_id, &group, &account, &event.after) {
                Role::Admin
            } else {
                self.access_for(&group, &account, &event.after)
                    .unwrap_or(Role::Pull)
            };
            Some((account, account_role.min(device_scope)))
        } else {
            None
        }
    }

    /// Whether the group voids the events that are concurrent with a removal of their author: strong
    /// remove does (design §6.2). Weak remove stands them, and only the authority check applies
    /// (design §6.3). A group with no record yet, or with `removal = strong` (the default), voids
    /// them. The record read is the group's current one, as with its delegation mode.
    pub(super) fn strong_removal(&self, group: Hash) -> bool {
        self.groups
            .get(&group)
            .is_none_or(|record| record.policy.removal == RemovalMode::Strong)
    }

    /// Whether a settled removal of the authorizing account is concurrent with the device grant, and
    /// the group voids such a grant. Strong remove voids it (design §6.2). Under weak remove only
    /// the authority check applies, so a concurrent device grant stands (design §6.3).
    fn void_device(&self, by_id: &BTreeMap<Hash, &Event>, grant: &DeviceGrant) -> bool {
        if !self.strong_removal(grant.group) {
            return false;
        }
        let past_grant = causal_past(by_id, &[grant.id]);
        self.settled
            .iter()
            .filter_map(|id| by_id.get(id).copied())
            .filter(|event| {
                event.group == Some(grant.group) && removed_member(event) == Some(grant.account)
            })
            .any(|removal| {
                !past_grant.contains(&removal.id)
                    && !causal_past(by_id, &[removal.id]).contains(&grant.id)
            })
    }

    /// Judges a device operation (P8.3).
    pub(super) fn device(
        &mut self,
        by_id: &BTreeMap<Hash, &Event>,
        event: &Event,
        op: DeviceOp,
    ) -> Result<Judged, FoldError> {
        let group = match self.acting_in(event, by_id) {
            Ok(group) => group,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let record = match self.record(&group) {
            Ok(record) => record,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if !self.is_active_for(&group, record, &event.author, &event.after) {
            return Ok(Err(Refusal::NotActive));
        }
        let is_admin = self.admin_for(by_id, &group, &event.author, &event.after);
        match op {
            DeviceOp::Add { device, scope } => {
                if device == event.author {
                    return Ok(Err(Refusal::BadGrantee));
                }
                let holds = if is_admin {
                    true
                } else if scope == Role::Admin {
                    false
                } else {
                    self.holds(group, event.author, scope, &event.after)?
                };
                if !holds {
                    return Ok(Err(Refusal::Attenuated));
                }
                Ok(Ok(Decision {
                    group,
                    action: None,
                    effect: Effect::DeviceAdd {
                        account: event.author,
                        device,
                        scope,
                    },
                }))
            }
            DeviceOp::Revoke { device } => Ok(Ok(Decision {
                group,
                action: None,
                effect: Effect::DeviceRevoke {
                    account: event.author,
                    device,
                },
            })),
        }
    }

    /// The group an event names, when the group is live and the event's author is an active member
    /// or authorized device of it. Otherwise the refusal.
    pub(super) fn acting_in(
        &self,
        event: &Event,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Result<Hash, Refusal> {
        let group = event.group.ok_or(Refusal::NoGroup)?;
        let record = self.record(&group)?;
        if record.dissolved_by.is_some() {
            return Err(Refusal::Dissolved);
        }
        if self.effective_author(by_id, group, event).is_none() {
            return Err(Refusal::NotActive);
        }
        Ok(group)
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

    /// Builds the projection from the final state of the round.
    fn into_projection(
        self,
        outcomes: BTreeMap<Hash, Outcome>,
        effective: BTreeSet<Hash>,
        in_force: BTreeSet<Hash>,
        by_id: &BTreeMap<Hash, &Event>,
    ) -> Projection {
        let membership = &self.membership;
        let grants: Vec<Grant> = self
            .grants
            .values()
            .filter(|grant| in_force.contains(&grant.id))
            .cloned()
            .collect();
        let devices: Vec<DeviceGrant> = self
            .devices
            .values()
            .filter(|grant| {
                if !effective.contains(&grant.id) {
                    return false;
                }
                if !membership
                    .members(grant.group)
                    .iter()
                    .any(|(id, _)| *id == AuthId::Key(grant.account))
                {
                    return false;
                }
                let revoked = self.device_revocations.values().any(|rev| {
                    rev.group == grant.group
                        && rev.account == grant.account
                        && rev.device == grant.device
                        && effective.contains(&rev.id)
                        && causal_past(by_id, &[rev.id]).contains(&grant.id)
                });
                if revoked {
                    return false;
                }
                if self.void_device(by_id, grant) {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
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
        Projection {
            groups,
            outcomes,
            effective,
            in_force,
            grants,
            devices,
        }
    }
}

/// Applies a state change to a group that exists.
fn apply_to(record: &mut Record, effect: Effect) {
    match effect {
        Effect::Create(_)
        | Effect::Delegation { .. }
        | Effect::Veto(_)
        | Effect::DeviceAdd { .. }
        | Effect::DeviceRevoke { .. } => {}
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
            ..
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
pub(super) fn access_role(access: &Access<()>) -> Role {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_an_empty_set_folds_to_nothing() {
        let p = fold(&[]).unwrap();
        assert_eq!(p.groups().count(), 0);
        assert!(!p.takes_effect(&Hash::digest(b"none")));
        assert!(!p.grant_in_force(&Hash::digest(b"none")));
    }
}
