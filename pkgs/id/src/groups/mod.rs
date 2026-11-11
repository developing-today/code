//! Groups (Phase 2, P2.4) — facade shim over `id-events`.
//!
//! Step 2b of the pkgs/id crate split (#4405; design ruling on #4405):
//! the group model types and [`fold`] are group-projection vocabulary
//! owned by the events layer, so `groups::mod`'s model, `policy`,
//! `event`, `fold`, `friendship` and the private `delegation` module
//! live in `id_events::groups` now, and this module re-exports them at
//! their historical paths. The policy/enforcement remainder of the
//! group layer — [`actions`], [`log`], [`warnings`], [`worlds`] —
//! stays here until step 4 extracts `id-groups` over `id-events`.
//!
//! A group changes only by events. [`fold`] computes its state from
//! the whole set of events, so the result does not depend on the order
//! the events arrived in. Membership operations go through
//! [`crate::membership`], which wraps p2panda-auth. The rules
//! p2panda-auth does not have are ours: the Admin role, the policy
//! record, the member cap, and dissolve.
//!
//! - Anyone may create a group. A normal group makes its creator Admin.
//! - Only an Admin may change a role or the policy, including the member cap.
//! - A friendship has no Admin unless its create asks for one (`admins_on_create`).
//!
//! Authority and the cap are checked against the events that come before each event in the
//! canonical order. See [`fold`].

pub mod actions;
pub mod log;
pub mod warnings;
pub mod worlds;

pub use id_events::groups::{event, fold, friendship, policy};
pub use id_events::groups::{
    Body, Create, DEVICE_ADD_KIND, DEVICE_REVOKE_KIND, Delegate, DeviceOp, Event, Grant, Key,
    MemberOp, Role,
};
pub use id_events::groups::{
    DeviceGrant, FoldError, GroupView, Outcome, Projection, Refusal,
};
pub use id_events::groups::{Pair, friendship_id};
pub use id_events::groups::{
    Delegation, History, Join, OnMemberRemoved, Policy, Removal, RemoveMode, RemoveModeKind,
    RemovePolicy, RemoveSuccessor, RoleRules, Scheme, SuccessorKind, Template, TransferOpts,
};
