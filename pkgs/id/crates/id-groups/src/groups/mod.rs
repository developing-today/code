//! Groups (Phase 2, P2.4) — the policy/enforcement remainder of the
//! group layer, in `id-groups` since step 4 of the pkgs/id crate
//! split (#4407).
//!
//! The group model types and [`fold`] are group-projection
//! vocabulary owned by the events layer (step 2b, #4405), so the
//! model, `policy`, `event`, `fold`, `friendship` and the private
//! `delegation` module live in `id_events::groups`, and this module
//! re-exports them at the paths the facade has always presented.
//! What lives here is the remainder over that projection:
//! [`actions`], [`log`] and [`warnings`]. The cross-world bridge
//! (`worlds`) lives in `id-worlds` since step 5 (seam 6).
//!
//! A group changes only by events. [`fold`] computes its state from
//! the whole set of events, so the result does not depend on the order
//! the events arrived in. Membership operations go through
//! [`id_core::membership`], which wraps p2panda-auth. The rules
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

pub use id_events::groups::{
    Body, Create, DEVICE_ADD_KIND, DEVICE_REVOKE_KIND, Delegate, DeviceOp, Event, Grant, Key,
    MemberOp, Role,
};
pub use id_events::groups::{
    Delegation, History, Join, OnMemberRemoved, Policy, Removal, RemoveMode, RemoveModeKind,
    RemovePolicy, RemoveSuccessor, RoleRules, Scheme, SuccessorKind, Template, TransferOpts,
};
pub use id_events::groups::{DeviceGrant, FoldError, GroupView, Outcome, Projection, Refusal};
pub use id_events::groups::{Pair, friendship_id};
pub use id_events::groups::{event, fold, friendship, policy};
