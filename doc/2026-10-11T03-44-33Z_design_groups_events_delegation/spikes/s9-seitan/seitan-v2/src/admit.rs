//! The admin side: invite state and the admission check. An admin's node runs this. It is the only path from a proof to membership.

use std::fmt;

use crate::accept::{self, Proof, ProofError};
use crate::derive::{self, InviteId};
use crate::token::Token;

/// Allowance for clock skew on `ctime`. It matches the 5-minute envelope window in the design.
pub const SKEW_MS: u64 = 5 * 60 * 1000;

/// An invite as the admin's chain records it, plus what the admin has admitted under it.
#[derive(Clone, Debug)]
pub struct Invite {
    id: InviteId,
    public_key: [u8; 32],
    created_ms: u64,
    expires_ms: u64,
    max_uses: usize,
    revoked: bool,
    admitted: Vec<(String, u64)>,
}

/// The joiner that was admitted. The caller then posts `member.set` for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admitted {
    pub uid: String,
    pub eldest_seqno: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Malformed,
    UnknownInvite,
    Revoked,
    Expired,
    Used,
    AlreadyAdmitted,
    Stale,
    FromFuture,
    BadKey,
    BadSignature,
}

impl From<ProofError> for Reject {
    fn from(e: ProofError) -> Reject {
        match e {
            ProofError::Malformed => Reject::Malformed,
            ProofError::BadKey => Reject::BadKey,
            ProofError::BadSignature => Reject::BadSignature,
        }
    }
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Reject::Malformed => "malformed accept",
            Reject::UnknownInvite => "no such invite",
            Reject::Revoked => "invite revoked",
            Reject::Expired => "invite expired",
            Reject::Used => "invite already used",
            Reject::AlreadyAdmitted => "joiner already admitted",
            Reject::Stale => "accept predates this invite",
            Reject::FromFuture => "accept is from the future",
            Reject::BadKey => "invite public key is invalid",
            Reject::BadSignature => "signature does not match the invite",
        })
    }
}

impl std::error::Error for Reject {}

impl Invite {
    /// Creates a Seitan V2 invite. The admin derives the public half from the token and keeps only the
    /// ID and the public key. The normal sequential Keybase V2 flow records completion in team-chain state; this
    /// spike models the same outcome locally with `max_uses = 1` (not Keybase's mechanism; see the follow-up in
    /// `pkgs/id/src/seitan/mod.rs`).
    ///
    /// # Panics
    /// If `max_uses` is 0.
    pub fn create(token: &Token, created_ms: u64, expires_ms: u64, max_uses: usize) -> Invite {
        assert!(max_uses >= 1, "an invite needs at least one use");
        let sikey = derive::stretch(token);
        Invite {
            id: derive::invite_id(&sikey),
            public_key: derive::signing_key(&sikey).verifying_key().to_bytes(),
            created_ms,
            expires_ms,
            max_uses,
            revoked: false,
            admitted: Vec::new(),
        }
    }

    pub fn id(&self) -> InviteId {
        self.id
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    pub fn revoke(&mut self) {
        self.revoked = true;
    }

    /// The admin path. Checks run in this order, and the signature comes last because it is the costly one:
    /// the invite is known, not revoked, not expired (now, and the signing time), has uses left, has not admitted
    /// this joiner, `ctime` is inside the freshness window, and the signature verifies. Only then is the use recorded.
    ///
    /// Freshness is bound to the invite's own life, not to the wall clock. A home server may hold an accept for days,
    /// so a window measured from now would reject honest holds. The window is [created - SKEW, now + SKEW].
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
        if self.admitted.len() >= self.max_uses {
            return Err(Reject::Used);
        }
        if self
            .admitted
            .iter()
            .any(|(uid, seq)| *uid == fields.uid && *seq == fields.eldest_seqno)
        {
            return Err(Reject::AlreadyAdmitted);
        }
        if fields.ctime_ms.saturating_add(SKEW_MS) < self.created_ms {
            return Err(Reject::Stale);
        }
        if fields.ctime_ms > now_ms.saturating_add(SKEW_MS) {
            return Err(Reject::FromFuture);
        }
        accept::verify(&self.public_key, proof)?;
        self.admitted
            .push((fields.uid.clone(), fields.eldest_seqno));
        Ok(Admitted {
            uid: fields.uid,
            eldest_seqno: fields.eldest_seqno,
        })
    }
}
