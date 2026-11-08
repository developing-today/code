//! Seitan V2 invites: the secret token that a joiner types, and the admin's check of the joiner's signed proof.
//!
//! - [`token`]: the 18-character token. It has about 83 bits of entropy when generated.
//! - [`derive`]: scrypt, the public invite ID, and the Ed25519 key, all derived from the token.
//! - [`accept`]: the joiner's signed accept message.
//! - [`admit`]: the admin's check: revocation, expiry, single use, freshness, and the signature.
//! - [`freshness`]: the window that bounds an accept's signing time. It defaults to 5 minutes and can be set per
//!   server, world, or inviter (N23). It is a local rule, so it is not part of the accept bytes.
//!
//! The derivations are checked against Keybase's V2 vector (`go/teams/seitan_v2_test.go`). The accept message
//! follows Keybase's field set and encoding, read from `go/teams/seitan_v2.go` and `go/msgpack`. No Keybase
//! vector covers an accept, so the message bytes are not checked against a Keybase-produced one.
//!
//! ## Keybase client findings and accepted unknowns
//!
//! In the normal sequential client flow, Keybase's old-style V2 invites are single use across transactions,
//! although they have no `MaxUses` count.
//! In `keybase/client` commit
//! [`952e87e`](https://github.com/keybase/client/commit/952e87eed559ffc3e5c840b3354b2161e4f725f8),
//! `InviteSeitanV2` constructs an `SCTeamInvite` with only type, name, and ID (no expiry or use-count field)
//! ([`teams.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/teams.go#L1445-L1478)),
//! and `IsNewStyleInvite` identifies new-style invites by `MaxUses != nil`
//! ([`chain.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/chain.go#L218-L223)).
//! For old-style invites, `CanConsumeInvite` checks a transaction-local completion map and
//! `ConsumeInviteByID` adds the invite to the transaction's `CompletedInvites`
//! ([`transactions.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/transactions.go#L760-L830)).
//! Chain replay applies that completion as a persistent `COMPLETED` invite status, while active-invite lookups
//! return only `ACTIVE` invites ([`chain.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/chain.go#L498-L509),
//! [`chain.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/chain.go#L603-L610),
//! [`chain.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/chain.go#L655-L674),
//! [`chain.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/chain.go#L2259-L2282)).
//! The normal handler repolls team state before it checks and consumes an invite
//! ([`handler.go`](https://github.com/keybase/client/blob/952e87eed559ffc3e5c840b3354b2161e4f725f8/go/teams/handler.go#L571-L627)).
//! Therefore, after a successful membership transaction commits and a later handler sees the updated chain, it
//! cannot consume that invite again. This establishes sequential single use from client chain state, not atomic
//! exclusion of concurrent or stale handlers, nor server-side replay behavior. This single-use behavior comes from
//! chain state, not an old-style invite use count. Old-style invites also have no expiry field; expiry and the
//! freshness window enforced here remain local rules.
//!
//! The `team/seitan_v2` service endpoint is closed source. The open client does not establish whether it can
//! withhold or replay accepts, or whether it relies on server-side state when handling them. We accept those as
//! service-level unknowns: local admission verifies the signed proof and applies its own expiry, freshness, and
//! single-use state, so the implementation does not rely on undocumented server state for its acceptance
//! decision. Withholding can still deny or delay service; this is not a claim about the endpoint's behavior.
//!
//! A real-client accept sample has not yet been captured; the available Keybase vectors cover derivation, not an
//! accept signature. See issue [#194](https://github.com/developing-today/code/issues/194) for the capture blocker.
//!
//! The token is the authority. The invite ID and the public key are public. Knowing the public key does not let
//! anyone sign an accept.

pub mod accept;
pub mod admit;
pub mod derive;
pub mod freshness;
pub mod token;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

pub use accept::{Fields, Proof, ProofError};
pub use admit::{Admitted, Invite, Reject};
pub use derive::InviteId;
pub use freshness::{DEFAULT_WINDOW, Freshness};
pub use token::{Token, TokenError};
