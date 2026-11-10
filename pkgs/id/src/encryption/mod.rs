//! Group encryption on the `DataScheme` of `p2panda-encryption` (Phase 4, P4.1 to P4.6 and P4.8 to P4.13).
//!
//! - [`scheme`]: the adapter. One [`Session`] per device and group, with create, add, remove,
//!   update, recover, send and process. Membership comes from the group model: the caller passes
//!   the [`crate::groups::GroupView`] the fold gives after the event, and the adapter takes its
//!   members from that view.
//! - [`roster`]: the membership the crate keeps. The adapter checks it against the model after each
//!   local membership change.
//! - [`ordering`]: causal delivery, and the [`Envelope`] messages travel in.
//! - [`history`]: the history modes a welcome is cut to (`full` or `recent:N`), and lazy history.
//! - [`welcome`]: a welcome above the frame cap, split into checked chunks.
//! - [`warnings`]: the server's local warnings about a group's size and re-add churn (P4.9).
//! - [`notice`]: the "scheme: none" notice for a group made without a scheme (P4.10).
//!
//! A group member is the key the group model holds (`crate::groups::Key`). The crate's identity is
//! the matching `p2panda_core::VerifyingKey`.
//!
//! The epoch rotates only on create, update, add by join, remove and recovery. No timer rotates it
//! (N7): the module takes no clock.

pub mod history;
pub mod notice;
pub mod ordering;
pub mod roster;
pub mod scheme;
pub mod welcome;

/// Re-exported so `crate::encryption::warnings` keeps working: the warnings now live in
/// [`crate::groups::warnings`] so the crate builds without the `world` feature.
pub use crate::groups::warnings;

pub use history::{HistoryMode, HistoryModeError};
pub use ordering::{Envelope, Orderer, OrdererError};
pub use roster::Roster;
pub use scheme::{
    DATASCHEME, MAX_HELD_BYTES, MAX_HELD_MESSAGES, RECOVERY_STEPS, Received, Recovery, SCHEME_V1,
    SchemeError, Session, StalledDelivery, key_bundle, key_of, negotiate, new_keys, registry,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
