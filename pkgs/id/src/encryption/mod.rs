//! Group encryption on the `DataScheme` of `p2panda-encryption` (Phase 4, P4.1 to P4.3, P4.8 to P4.11).
//!
//! - [`scheme`]: the adapter. One [`Session`] per device and group, with create, add, remove,
//!   update, recover, send and process. Membership comes from the group model: the caller passes
//!   the member set the model holds after the event.
//! - [`roster`]: the membership the crate keeps. The adapter checks it against the model after each
//!   local membership change.
//! - [`ordering`]: causal delivery, and the [`Envelope`] messages travel in.
//! - [`warnings`]: the server's local warnings about a group's size and re-add churn (P4.9).
//! - [`notice`]: the "scheme: none" notice for a group made without a scheme (P4.10).
//!
//! A group member is the key the group model holds (`crate::groups::Key`). The crate's identity is
//! the matching `p2panda_core::VerifyingKey`.
//!
//! The epoch rotates only on create, update, add by join, remove and recovery. No timer rotates it
//! (N7): the module takes no clock.

pub mod notice;
pub mod ordering;
pub mod roster;
pub mod scheme;
pub mod warnings;

pub use ordering::{Envelope, Orderer, OrdererError};
pub use roster::Roster;
pub use scheme::{
    RECOVERY_STEPS, Received, Recovery, SchemeError, Session, key_bundle, key_of, new_keys,
    registry,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
