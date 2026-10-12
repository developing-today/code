//! The inviter's signed invite event.
//!
//! The signed bytes are a fixed layout, so one event has exactly one encoding:
//! `DOMAIN || inviter (32) || ticket_digest (32)`. The domain tag keeps a signature made here
//! from being replayed as a signature over some other message.

use anyhow::{Result, anyhow};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use iroh_tickets::{Ticket as _, endpoint::EndpointTicket};
use p2panda_core::Hash;

const DOMAIN: &[u8] = b"id-invite/v1/event";
const TICKET_DOMAIN: &[u8] = b"id-invite/v1/ticket";

/// An invite the inviter has signed for one ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteEvent {
    /// The signed fields.
    pub body: InviteBody,
    /// Ed25519 over the body's message, verified with `verify_strict`.
    pub signature: [u8; 64],
}

/// The fields the inviter signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteBody {
    /// The signing key: the inviter's account key.
    pub inviter: [u8; 32],
    /// The digest of the ticket's canonical bytes. The string form is not used, so two strings
    /// that parse to the same ticket give the same digest.
    pub ticket_digest: [u8; 32],
}

impl InviteEvent {
    /// Signs an invite for `ticket` with `key`. The inviter is `key`'s public key.
    #[must_use]
    pub fn sign(key: &SigningKey, ticket: &EndpointTicket) -> Self {
        let body = InviteBody {
            inviter: key.verifying_key().to_bytes(),
            ticket_digest: ticket_digest(ticket),
        };
        let signature = key.sign(&body.message()).to_bytes();
        Self { body, signature }
    }

    /// Checks the signature against the signing key the event names.
    ///
    /// This proves only that the named key signed the body. It does not prove that the key is
    /// the one a link names. The caller checks that.
    ///
    /// # Errors
    ///
    /// Fails if the named key is not a valid Ed25519 key, or the signature does not verify.
    pub fn verify_signature(&self) -> Result<()> {
        let key = VerifyingKey::from_bytes(&self.body.inviter).map_err(|error| {
            anyhow!("the invite event is signed by a key that is not valid: {error}")
        })?;
        key.verify_strict(
            &self.body.message(),
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|error| anyhow!("the invite event has a bad signature: {error}"))
    }
}

impl InviteBody {
    fn message(&self) -> Vec<u8> {
        [
            DOMAIN,
            self.inviter.as_slice(),
            self.ticket_digest.as_slice(),
        ]
        .concat()
    }
}

/// The digest of a ticket's canonical bytes.
pub(super) fn ticket_digest(ticket: &EndpointTicket) -> [u8; 32] {
    *Hash::digest([TICKET_DOMAIN, &ticket.encode_bytes()].concat()).as_bytes()
}
