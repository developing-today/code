//! Group encryption on the `DataScheme` of `p2panda-encryption` (Phase 4, P4.1 to P4.3).
//!
//! - [`scheme`]: the adapter. One [`Session`] per device and group, with create, add, remove,
//!   update, send and process. Membership comes from the group model: the caller passes the member
//!   set the model holds after the event.
//! - [`roster`]: the membership the crate keeps. The adapter checks it against the model after each
//!   local membership change.
//! - [`ordering`]: causal delivery, and the [`Envelope`] messages travel in.
//!
//! A group member is the key the group model holds (`crate::groups::Key`). The crate's identity is
//! the matching `p2panda_core::VerifyingKey`.

pub mod ordering;
pub mod roster;
pub mod scheme;

pub use ordering::{Envelope, Orderer, OrdererError};
pub use roster::Roster;
pub use scheme::{Received, SchemeError, Session, key_bundle, key_of, new_keys, registry};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
