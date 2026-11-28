//! Seitan V2 invites for a join, following keybase/client `go/teams/seitan_v2.go`.
//!
//! - [`token`]: the 18-character secret that the joiner types.
//! - [`derive`]: scrypt, the invite ID, and the Ed25519 key, all derived from the token.
//! - [`accept`]: the joiner's signed proof.
//! - [`admit`]: the admin's check, with freshness, expiry, revocation, and single use.
//!
//! The derivations are checked against Keybase's vectors in `tests/vectors.rs`. The accept message and the
//! admission checks are this crate's own. They are not Keybase's.

#![forbid(unsafe_code)]

pub mod accept;
pub mod admit;
pub mod derive;
pub mod token;

pub use accept::{Fields, Proof, ProofError};
pub use admit::{Admitted, Invite, Reject, SKEW_MS};
pub use derive::InviteId;
pub use token::{Token, TokenError};
