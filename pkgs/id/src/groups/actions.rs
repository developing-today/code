//! The actions a user takes on groups (P6.7a). Each action is one event, built from its
//! arguments and recorded in the [`GroupLog`]. The fold decides whether it took effect.
//!
//! The functions here check only what the fold does not: that the group an action names exists in
//! the log, and that an option fits the kind of group it is given for. Everything else is the fold's.

use anyhow::{Result, anyhow, bail};
use p2panda_core::Hash;

use super::fold::{GroupView, Outcome};
use super::friendship::{Pair, friendship_id};
use super::log::{GroupLog, Recorded};
use super::policy::{Policy, Template};
use super::{Body, Create, Key, MemberOp, Role};

/// What an action did: the group it acted in, and the event it recorded with what the fold made of
/// it. For a create, the group is the new group's ID, which is the create event's ID for a normal
/// group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// The group the action acted in.
    pub group: Hash,
    /// The event that was recorded. For a delegation it is the grant's ID.
    pub event: Hash,
    /// Whether the event took effect, or why not.
    pub outcome: Outcome,
}

/// A group to create.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewGroup {
    /// A normal group. Its creator is Admin. `max` caps its members, counting the creator.
    Normal {
        /// The member cap, or `None` for no cap.
        max: Option<usize>,
        /// Whether both sides start as Admin. Only a friendship may ask for it.
        admins_on_create: bool,
    },
    /// A friendship with `with`, two members who ask and accept each other.
    Friendship {
        /// The other side.
        with: Key,
        /// Whether both sides start as Admin. Without it a friendship has no Admin.
        admins_on_create: bool,
    },
}

impl GroupLog {
    /// Creates a group. The creator's event is recorded and folded; the new group is active once the
    /// fold applies it (a friendship also needs the other side's accept).
    ///
    /// # Errors
    ///
    /// Fails if an option does not fit the kind of group, or the event cannot be recorded.
    pub fn create(&mut self, author: Key, new: NewGroup) -> Result<Action> {
        match new {
            NewGroup::Normal {
                max,
                admins_on_create,
            } => {
                if admins_on_create {
                    bail!("admins_on_create applies to friendships only");
                }
                let body = Body::Create(Create {
                    template: Template::Custom,
                    policy: Policy {
                        member_cap: max,
                        ..Policy::default()
                    },
                    members: Vec::new(),
                    admins_on_create: false,
                });
                let recorded = self.record(author, None, body)?;
                Ok(Action {
                    group: recorded.event,
                    event: recorded.event,
                    outcome: recorded.outcome,
                })
            }
            NewGroup::Friendship {
                with,
                admins_on_create,
            } => {
                let Some(pair) = Pair::of(author, with) else {
                    bail!("a friendship needs two different accounts");
                };
                let group = friendship_id(&pair, self.dissolved_friendships(&pair)?);
                // The role is not used by a friendship: both sides start with manage access.
                let body = Body::Create(Create {
                    template: Template::Friendship,
                    policy: Policy::friendship(),
                    members: vec![(with, Role::Manage)],
                    admins_on_create,
                });
                let recorded = self.record(author, Some(group), body)?;
                Ok(Action {
                    group,
                    event: recorded.event,
                    outcome: recorded.outcome,
                })
            }
        }
    }

    /// The group as the fold gives it: its policy, members and state.
    ///
    /// # Errors
    ///
    /// Fails if the log has no such group, or the events cannot be folded.
    pub fn view(&self, group: Hash) -> Result<GroupView> {
        self.projection()?
            .group(&group)
            .cloned()
            .ok_or_else(|| anyhow!("no group {} in the log", group.to_hex()))
    }

    /// Adds `member` to `group` with `role`.
    ///
    /// # Errors
    ///
    /// Fails if the group is not in the log, or the event cannot be recorded.
    pub fn add(&mut self, author: Key, group: Hash, member: Key, role: Role) -> Result<Action> {
        self.act(author, group, Body::Member(MemberOp::Add { member, role }))
    }

    /// Removes `member` from `group`. With `successor`, the removal names who takes over the member's
    /// grants under the `transfer` mode.
    ///
    /// # Errors
    ///
    /// Fails if the group is not in the log, or the event cannot be recorded.
    pub fn remove(
        &mut self,
        author: Key,
        group: Hash,
        member: Key,
        successor: Option<Key>,
    ) -> Result<Action> {
        self.act(
            author,
            group,
            Body::Member(MemberOp::Remove { member, successor }),
        )
    }

    /// Sets `member`'s role in `group`.
    ///
    /// # Errors
    ///
    /// Fails if the group is not in the log, or the event cannot be recorded.
    pub fn set_role(
        &mut self,
        author: Key,
        group: Hash,
        member: Key,
        role: Role,
    ) -> Result<Action> {
        self.act(
            author,
            group,
            Body::Member(MemberOp::SetRole { member, role }),
        )
    }

    /// Records one event in a group that the log has.
    fn act(&mut self, author: Key, group: Hash, body: Body) -> Result<Action> {
        self.view(group)?;
        let Recorded { event, outcome } = self.record(author, Some(group), body)?;
        Ok(Action {
            group,
            event,
            outcome,
        })
    }

    /// How many friendships of `pair` have been dissolved. A new request for the pair names the next
    /// counter, which gives it a new group ID.
    fn dissolved_friendships(&self, pair: &Pair) -> Result<u32> {
        let count = self
            .projection()?
            .groups()
            .filter(|(_, view)| view.friendship == Some(*pair) && view.dissolved_by.is_some())
            .count();
        Ok(u32::try_from(count)?)
    }
}
