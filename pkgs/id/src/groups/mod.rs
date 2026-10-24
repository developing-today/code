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

pub mod actions;
pub mod fold;
pub mod friendship;
pub mod log;
pub mod policy;
pub mod warnings;
pub mod worlds;

mod delegation;

#[cfg(test)]
mod tests;

use clap::ValueEnum;
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

pub use delegation::Grant;
pub use fold::{DeviceGrant, FoldError, GroupView, Outcome, Projection, Refusal, fold};
pub use friendship::{Pair, friendship_id};
pub use policy::{
    Delegation, History, Join, OnMemberRemoved, Policy, Removal, RoleRules, Scheme, Template,
};

/// The kind of a `device.add` event (P8.3).
pub const DEVICE_ADD_KIND: &str = "device.add";

/// The kind of a `device.revoke` event (P8.3).
pub const DEVICE_REVOKE_KIND: &str = "device.revoke";

/// A grant of a scope from the event's author to another account (design §7.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegate {
    /// The account granted the scope.
    #[serde(with = "log::wire::key")]
    pub grantee: Key,
    /// The access the grantee may exercise. Admin is a policy right and is not delegable.
    pub scope: Role,
    /// Whether the grantee may grant the scope on. Only a redelegatable grant extends a chain.
    pub can_redelegate: bool,
}

/// An account's key. Accounts are keys in this layer.
pub type Key = [u8; 32];

/// What a member may do, from least to most. `Admin` is a manager that may also change roles and
/// the policy record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// The event's ID.
    #[serde(with = "log::wire::hash")]
    pub id: Hash,
    /// The account that signed it.
    #[serde(with = "log::wire::key")]
    pub author: Key,
    /// The `after` hashes, as signed.
    #[serde(with = "log::wire::hashes")]
    pub after: Vec<Hash>,
    /// The group it names. A normal group's create names none. A friendship's create names the
    /// friendship's derived group.
    #[serde(with = "log::wire::opt_hash")]
    pub group: Option<Hash>,
    /// What it does.
    pub body: Body,
}

/// What a group event does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    /// Creates a group. Anyone may create one.
    Create(Create),
    /// Accepts a friendship request, as its other side.
    Join,
    /// Replaces the policy record. Admin only.
    Policy(Policy),
    /// Changes one member.
    Member(MemberOp),
    /// Grants a scope to an account. The author must hold the scope, and must reach it through a
    /// chain of grants that can be passed on (design §7.2).
    Delegate(Delegate),
    /// Grants a scope that the grantee may use in another world (P2.7, see [`worlds`]). It is judged
    /// in this event's world the way [`Body::Delegate`] is, but it gives its grantee nothing here.
    Cross {
        /// The grant. Its scope may be at most Write.
        delegate: Delegate,
        /// The world the grant names. It is not the event's own world.
        world: String,
    },
    /// Vetoes a grant the event has seen. The author must hold Manage and a chain (design §7).
    /// From this event's causal position on, the grant and what depends on it fail.
    Veto {
        /// The grant's event ID.
        #[serde(with = "log::wire::hash")]
        grant: Hash,
    },
    /// Authorizes or revokes a device on behalf of the account (P8.3).
    Device(DeviceOp),
}

/// What a device operation does (P8.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceOp {
    /// Authorizes a device key to act on behalf of the account with the given scope.
    Add {
        /// The device key.
        #[serde(with = "log::wire::key")]
        device: Key,
        /// The maximum scope the device may exercise.
        scope: Role,
    },
    /// Revokes a device's authorization to act on behalf of the account.
    Revoke {
        /// The device key.
        #[serde(with = "log::wire::key")]
        device: Key,
    },
}

/// A group's creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Create {
    /// The template the group is created from.
    pub template: Template,
    /// The policy record, written out in full.
    pub policy: Policy,
    /// The members besides the author, with their roles. A friendship lists exactly one.
    #[serde(with = "log::wire::members")]
    pub members: Vec<(Key, Role)>,
    /// Whether both friends start as Admin. Only a friendship may set it.
    pub admins_on_create: bool,
}

/// A change to one member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberOp {
    /// Adds a member with a role.
    Add {
        /// The member added.
        #[serde(with = "log::wire::key")]
        member: Key,
        /// Its role.
        role: Role,
    },
    /// Removes a member. The author may remove itself, which is leaving.
    Remove {
        /// The member removed.
        #[serde(with = "log::wire::key")]
        member: Key,
        /// Under the `transfer` delegation mode, the account that takes over the member's grants
        /// (design §7.3). Ignored by the other modes.
        #[serde(with = "log::wire::opt_key")]
        successor: Option<Key>,
    },
    /// Sets a member's role. Admin only.
    SetRole {
        /// The member whose role changes.
        #[serde(with = "log::wire::key")]
        member: Key,
        /// Its new role.
        role: Role,
    },
}
