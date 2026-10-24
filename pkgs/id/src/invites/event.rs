//! The inviter's signed invite event.
//!
//! The bytes are a fixed layout, so one event has exactly one encoding:
//! `inviter (32) || ticket_digest (32) || group (32) || role (1) || expires_ms (8, big-endian)`,
//! then the signature (64). The signature covers the same fields behind a domain tag, so a
//! signature made here cannot be replayed as a signature over some other message.
//!
//! Role codes are part of the format: 0 `Pull`, 1 `Read`, 2 `Write`, 3 `Manage`, 4 `Admin`.

use anyhow::{Context, Result, anyhow, bail, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use iroh_tickets::{Ticket as _, endpoint::EndpointTicket};
use p2panda_core::Hash;

use crate::groups::Role;

const DOMAIN: &[u8] = b"id-invite/v1/event";
const TICKET_DOMAIN: &[u8] = b"id-invite/v1/ticket";
/// The fields before the signature: 32 + 32 + 32 + 1 + 8 bytes.
const FIELDS_LEN: usize = 105;
const EVENT_LEN: usize = FIELDS_LEN + 64;

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
    /// The group the invite adds the joiner to.
    pub group: [u8; 32],
    /// The role the joiner gets.
    pub role: Role,
    /// The time, in milliseconds, from which the invite is refused.
    pub expires_ms: u64,
}

impl InviteEvent {
    /// Signs an invite for `ticket`, to `group` with `role`, expiring at `expires_ms`.
    #[must_use]
    pub fn sign(
        key: &SigningKey,
        group: &[u8; 32],
        role: Role,
        expires_ms: u64,
        ticket: &EndpointTicket,
    ) -> Self {
        let body = InviteBody {
            inviter: key.verifying_key().to_bytes(),
            ticket_digest: ticket_digest(ticket),
            group: *group,
            role,
            expires_ms,
        };
        let signature = key.sign(&body.message()).to_bytes();
        Self { body, signature }
    }

    /// The event's bytes: the fields, then the signature.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.body.fields().as_slice(), self.signature.as_slice()].concat()
    }

    /// Reads an event written by [`InviteEvent::to_bytes`]. The length must be exact.
    ///
    /// # Errors
    ///
    /// Fails if the length is wrong, or the role code is not one this format defines.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() == EVENT_LEN,
            "an invite event is {EVENT_LEN} bytes, not {}",
            bytes.len()
        );
        let (inviter, rest) = bytes
            .split_first_chunk::<32>()
            .context("the invite event has no inviter")?;
        let (ticket_digest, rest) = rest
            .split_first_chunk::<32>()
            .context("the invite event has no ticket digest")?;
        let (group, rest) = rest
            .split_first_chunk::<32>()
            .context("the invite event has no group")?;
        let (&code, rest) = rest.split_first().context("the invite event has no role")?;
        let (expires, rest) = rest
            .split_first_chunk::<8>()
            .context("the invite event has no expiry")?;
        let signature = rest
            .first_chunk::<64>()
            .context("the invite event has no signature")?;
        Ok(Self {
            body: InviteBody {
                inviter: *inviter,
                ticket_digest: *ticket_digest,
                group: *group,
                role: role_from_code(code)?,
                expires_ms: u64::from_be_bytes(*expires),
            },
            signature: *signature,
        })
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
    /// The fixed-layout fields, without the domain tag.
    fn fields(&self) -> Vec<u8> {
        [
            self.inviter.as_slice(),
            self.ticket_digest.as_slice(),
            self.group.as_slice(),
            [role_code(self.role)].as_slice(),
            self.expires_ms.to_be_bytes().as_slice(),
        ]
        .concat()
    }

    fn message(&self) -> Vec<u8> {
        [DOMAIN, &self.fields()].concat()
    }
}

const fn role_code(role: Role) -> u8 {
    match role {
        Role::Pull => 0,
        Role::Read => 1,
        Role::Write => 2,
        Role::Manage => 3,
        Role::Admin => 4,
    }
}

fn role_from_code(code: u8) -> Result<Role> {
    Ok(match code {
        0 => Role::Pull,
        1 => Role::Read,
        2 => Role::Write,
        3 => Role::Manage,
        4 => Role::Admin,
        other => bail!("unknown role code {other} in an invite event"),
    })
}

/// The digest of a ticket's canonical bytes.
pub(super) fn ticket_digest(ticket: &EndpointTicket) -> [u8; 32] {
    *Hash::digest([TICKET_DOMAIN, &ticket.encode_bytes()].concat()).as_bytes()
}
