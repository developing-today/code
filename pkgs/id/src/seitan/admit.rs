//! The admin side: an invite and the admission check. An admin's node runs this. It is the only path from a proof to
//! membership.

use std::fmt;
use std::time::Duration;

use anyhow::Result;

use super::accept::{self, Proof, ProofError};
use super::derive::{self, InviteId};
use super::token::Token;

/// An invite as the admin's chain records it, plus the joiner it admitted.
///
/// This local invite object admits at most one joiner. Keybase's normal sequential old-style V2 flow also records
/// completion in team chain state rather than an invite use count (see the source references and concurrency caveat
/// in `mod.rs`). Expiry, revocation, and the freshness window enforced here are local admission rules.
#[derive(Clone, Debug)]
pub struct Invite {
    id: InviteId,
    public_key: [u8; 32],
    created_ms: u64,
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
    /// The accept was signed more than the window before the invite was created. The field is the window.
    Stale(Duration),
    /// The accept was signed more than the window after the admin's clock. The field is the window.
    FromFuture(Duration),
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
            Self::Stale(window) => {
                return write!(
                    f,
                    "the accept is older than the {} s freshness window before the invite was created",
                    window.as_secs()
                );
            }
            Self::FromFuture(window) => {
                return write!(
                    f,
                    "the accept is more than the {} s freshness window ahead of this server's clock",
                    window.as_secs()
                );
            }
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
    pub fn create(token: &Token, created_ms: u64, expires_ms: u64) -> Result<Self> {
        let sikey = derive::stretch(token)?;
        Ok(Self {
            id: derive::invite_id(&sikey)?,
            public_key: derive::signing_key(&sikey)?.verifying_key().to_bytes(),
            created_ms,
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
    /// been used, the signing time is inside the freshness window, and the signature verifies. Only then is the use
    /// recorded, so a refused accept uses nothing up.
    ///
    /// The window is bound to the invite's own life (N23), not to the wall clock. A home server may hold an accept
    /// for a while before it forwards it, so a window measured from `now` would refuse honest holds. An accept is
    /// admitted when `created − window ≤ ctime ≤ min(now + window, expires)`. The window is inclusive at both edges.
    ///
    /// # Errors
    ///
    /// Returns the first check that fails, as a [`Reject`].
    pub fn admit(
        &mut self,
        proof: &Proof,
        now_ms: u64,
        window: Duration,
    ) -> Result<Admitted, Reject> {
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
        let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
        if fields.ctime_ms.saturating_add(window_ms) < self.created_ms {
            return Err(Reject::Stale(window));
        }
        if fields.ctime_ms > now_ms.saturating_add(window_ms) {
            return Err(Reject::FromFuture(window));
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
