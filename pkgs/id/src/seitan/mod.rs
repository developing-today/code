//! Seitan V2 invites: the secret token that a joiner types, and the admin's check of the joiner's signed proof.
//!
//! - [`token`]: the 18-character token. It has about 83 bits of entropy when generated.
//! - [`derive`]: scrypt, the public invite ID, and the Ed25519 key, all derived from the token.
//! - [`accept`]: the joiner's signed accept message.
//! - [`admit`]: the admin's check: revocation, expiry, single use, and the signature.
//!
//! The derivations are checked against Keybase's V2 vector (`go/teams/seitan_v2_test.go`). The accept message
//! follows Keybase's field set and encoding, read from `go/teams/seitan_v2.go` and `go/msgpack`. No Keybase
//! vector covers an accept, so the message bytes are not checked against a Keybase-produced one.
//!
//! The token is the authority. The invite ID and the public key are public. Knowing the public key does not let
//! anyone sign an accept.

pub mod accept;
pub mod admit;
pub mod derive;
pub mod token;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

pub use accept::{Fields, Proof, ProofError};
pub use admit::{Admitted, Invite, Reject};
pub use derive::InviteId;
pub use token::{Token, TokenError};
