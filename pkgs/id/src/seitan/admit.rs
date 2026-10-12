//! The admin side: an invite and the admission check. An admin's node runs this. It is the only path from a proof to
//! membership.

use std::fmt;

use anyhow::Result;

use super::accept::{self, Proof, ProofError};
use super::derive::{self, InviteId};
use super::token::Token;

/// An invite as the admin's chain records it, plus the joiner it admitted.
///
/// A Seitan invite is single use, as Keybase's is. Expiry and revocation are local rules here. Keybase's V2 invites
/// carry neither (see the module notes in `mod.rs`).
#[derive(Clone, Debug)]
pub struct Invite {
    id: InviteId,
    public_key: [u8; 32],
    expires_ms: u64,
    revoked: bool,
    used: Option<Admitted>,
}

/// The joiner that was admitted. The caller then posts the membership change for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admitted {
    /// The joiner's user ID.
    pub uid: String,
    /// The joiner's eldest sequence number.
    pub eldest_seqno: u64,
}

/// Why an accept was refused. A refused accept uses nothing up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// The accept is not a version 2 accept message.
    Malformed,
    /// The accept names an invite ID that this invite does not have.
    UnknownInvite,
    /// The admin revoked the invite.
    Revoked,
    /// The invite has expired, or the accept was signed after it expired.
    Expired,
    /// The invite has already admitted a joiner.
    Used,
    /// The invite's public key is not a valid Ed25519 point.
    BadKey,
    /// The signature does not verify under the invite's key.
    BadSignature,
}

impl From<ProofError> for Reject {
    fn from(e: ProofError) -> Self {
        match e {
            ProofError::Malformed => Self::Malformed,
            ProofError::BadKey => Self::BadKey,
            ProofError::BadSignature => Self::BadSignature,
        }
    }
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed accept message",
            Self::UnknownInvite => "no invite matches this accept",
            Self::Revoked => "the invite was revoked",
            Self::Expired => "the invite has expired",
            Self::Used => "the invite has already been used",
            Self::BadKey => "the invite's public key is not valid",
            Self::BadSignature => "the accept's signature does not match this invite",
        })
    }
}

impl std::error::Error for Reject {}

impl Invite {
    /// Creates a Seitan V2 invite from its token. The admin derives the public half and keeps only the ID, the
    /// public key, and the expiry. The token itself is not kept.
    ///
    /// # Errors
    ///
    /// Fails only if a derivation fails, which the fixed parameters never trigger.
    pub fn create(token: &Token, expires_ms: u64) -> Result<Self> {
        let sikey = derive::stretch(token)?;
        Ok(Self {
            id: derive::invite_id(&sikey)?,
            public_key: derive::signing_key(&sikey)?.verifying_key().to_bytes(),
            expires_ms,
            revoked: false,
            used: None,
        })
    }

    /// The public invite ID.
    #[must_use]
    pub const fn id(&self) -> InviteId {
        self.id
    }

    /// The Ed25519 public key that accepts must verify under. It is public, and knowing it does not let anyone sign
    /// an accept.
    #[must_use]
    pub const fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// The joiner this invite admitted, if it has admitted one.
    #[must_use]
    pub const fn admitted(&self) -> Option<&Admitted> {
        self.used.as_ref()
    }

    /// Revokes the invite. A revoked invite admits no one.
    pub const fn revoke(&mut self) {
        self.revoked = true;
    }

    /// The admin path. The checks run in this order, and the signature comes last because it is the costly one:
    /// the invite ID matches, the invite is not revoked, it has not expired (now, and the signing time), it has not
    /// been used, and the signature verifies. Only then is the use recorded, so a refused accept uses nothing up.
    ///
    /// # Errors
    ///
    /// Returns the first check that fails, as a [`Reject`].
    pub fn admit(&mut self, proof: &Proof, now_ms: u64) -> Result<Admitted, Reject> {
        let fields = accept::parse(&proof.message)?;
        if fields.invite_id != self.id {
            return Err(Reject::UnknownInvite);
        }
        if self.revoked {
            return Err(Reject::Revoked);
        }
        if now_ms > self.expires_ms || fields.ctime_ms > self.expires_ms {
            return Err(Reject::Expired);
        }
        if self.used.is_some() {
            return Err(Reject::Used);
        }
        accept::verify(&self.public_key, proof)?;
        let admitted = Admitted {
            uid: fields.uid,
            eldest_seqno: fields.eldest_seqno,
        };
        self.used = Some(admitted.clone());
        Ok(admitted)
    }
}
