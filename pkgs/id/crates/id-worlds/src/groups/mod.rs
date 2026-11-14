//! Groups, as the worlds layer sees them.
//!
//! The group model types and [`fold`] are group-projection
//! vocabulary owned by the events layer (step 2b, #4405), so the
//! model, `policy`, `event`, `fold`, `friendship` and the private
//! `delegation` module live in `id_events::groups`, and this module
//! re-exports them at the paths the facade has always presented —
//! the same re-exports `id_groups::groups` carries. What lives here
//! is [`worlds`]: seam 6 of the crate-split design sends the
//! groups→worlds bridge to the worlds crate, because the bridge is
//! the consumer side of the relationship (the group layer must not
//! know worlds exist).

pub mod worlds;

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
