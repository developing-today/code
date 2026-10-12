//! Groups (Phase 2, P2.4): members and roles, the signed policy record, the templates groups are
//! created from, and friendship as a two-member group (design §5).
//!
//! A group changes only by events. [`fold`] computes its state from the whole set of events, so the
//! result does not depend on the order the events arrived in. Membership operations go through
//! [`crate::membership`], which wraps p2panda-auth. The rules p2panda-auth does not have are ours:
//! the Admin role, the policy record, the member cap, and dissolve.
//!
//! - Anyone may create a group. A normal group makes its creator Admin.
//! - Only an Admin may change a role or the policy, including the member cap.
//! - A friendship has no Admin unless its create asks for one (`admins_on_create`).
//!
//! Authority and the cap are checked against the events that come before each event in the
//! canonical order. See [`fold`].

pub mod fold;
pub mod friendship;
pub mod policy;

#[cfg(test)]
mod tests;

use p2panda_core::Hash;

pub use fold::{FoldError, GroupView, Outcome, Projection, Refusal, fold};
pub use friendship::{Pair, friendship_id};
pub use policy::{
    Delegation, History, Join, OnMemberRemoved, Policy, Removal, RoleRules, Scheme, Template,
};

/// An account's key. Accounts are keys in this layer.
pub type Key = [u8; 32];

/// What a member may do, from least to most. `Admin` is a manager that may also change roles and
/// the policy record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Replicates ciphertext without keys.
    Pull,
    /// Reads content.
    Read,
    /// Edits content.
    Write,
    /// Adds and removes members.
    Manage,
    /// Manages, and changes roles and the policy record.
    Admin,
}

/// One signed group event, as the group layer sees it. The caller has verified it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// The event's ID.
    pub id: Hash,
    /// The account that signed it.
    pub author: Key,
    /// The `after` hashes, as signed.
    pub after: Vec<Hash>,
    /// The group it names. A normal group's create names none. A friendship's create names the
    /// friendship's derived group.
    pub group: Option<Hash>,
    /// What it does.
    pub body: Body,
}

/// What a group event does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// Creates a group. Anyone may create one.
    Create(Create),
    /// Accepts a friendship request, as its other side.
    Join,
    /// Replaces the policy record. Admin only.
    Policy(Policy),
    /// Changes one member.
    Member(MemberOp),
}

/// A group's creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Create {
    /// The template the group is created from.
    pub template: Template,
    /// The policy record, written out in full.
    pub policy: Policy,
    /// The members besides the author, with their roles. A friendship lists exactly one.
    pub members: Vec<(Key, Role)>,
    /// Whether both friends start as Admin. Only a friendship may set it.
    pub admins_on_create: bool,
}

/// A change to one member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberOp {
    /// Adds a member with a role.
    Add {
        /// The member added.
        member: Key,
        /// Its role.
        role: Role,
    },
    /// Removes a member. The author may remove itself, which is leaving.
    Remove {
        /// The member removed.
        member: Key,
    },
    /// Sets a member's role. Admin only.
    SetRole {
        /// The member whose role changes.
        member: Key,
        /// Its new role.
        role: Role,
    },
}
