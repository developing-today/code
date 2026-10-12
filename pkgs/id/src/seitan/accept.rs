//! The joiner's proof. It mirrors `GenerateSeitanSignatureMessage` and `GenerateSignature` in keybase/client
//! `go/teams/seitan_v2.go`.
//!
//! The joiner sends the message bytes and the signature. The admin verifies the signature over the bytes exactly as
//! received, and only then reads the fields out of them. Nothing is re-encoded before verifying. The signature is
//! plain Ed25519 over the message, with no domain prefix, as `kbcrypto`'s `NaclSigningKeyPrivate.Sign` does.

use std::fmt;

use anyhow::{Context as _, Result};
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::derive::{self, InviteId, VERSION};
use super::token::Token;

/// What the joiner sends: the signed message bytes and the Ed25519 signature over them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// The msgpack accept message, exactly as signed.
    pub message: Vec<u8>,
    /// The Ed25519 signature over `message`.
    pub signature: [u8; 64],
}

/// The fields of a decoded accept message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    /// The joiner's user ID.
    pub uid: String,
    /// The joiner's eldest sequence number.
    pub eldest_seqno: u64,
    /// The time the joiner signed, in milliseconds since the Unix epoch.
    pub ctime_ms: u64,
    /// The invite the joiner accepts.
    pub invite_id: InviteId,
}

/// Why a proof was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofError {
    /// The message is not an accept, or it is not version 2, or a field is malformed.
    Malformed,
    /// The invite's public key is not a valid Ed25519 point.
    BadKey,
    /// The signature does not verify over the message under the invite's key.
    BadSignature,
}

impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed accept message",
            Self::BadKey => "the invite's public key is not valid",
            Self::BadSignature => "the accept's signature does not match this invite",
        })
    }
}

impl std::error::Error for ProofError {}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    stage: String,
    uid: String,
    eldest_seqno: u64,
    ctime: u64,
    invite_id: String,
    version: u8,
}

/// The message the joiner signs, as msgpack: `{stage: "accept", uid, eldest_seqno, ctime, invite_id, version: 2}`.
///
/// `ctime` is in milliseconds since the Unix epoch, which Keybase's V2 handler also uses. `invite_id` is the
/// 32-character hex form.
///
/// # Errors
///
/// Fails only if the message cannot be encoded, which a fixed struct never triggers.
pub fn message(
    uid: &str,
    eldest_seqno: u64,
    ctime_ms: u64,
    invite_id: &InviteId,
) -> Result<Vec<u8>> {
    rmp_serde::to_vec_named(&Wire {
        stage: "accept".to_owned(),
        uid: uid.to_owned(),
        eldest_seqno,
        ctime: ctime_ms,
        invite_id: invite_id.to_hex(),
        version: VERSION,
    })
    .context("encoding the accept message")
}

/// The joiner's side. The joiner types the token, derives the key, and signs. This changes no membership state.
///
/// # Errors
///
/// Fails only if the message cannot be encoded or a derivation fails, which the fixed inputs never trigger.
pub fn sign(token: &Token, uid: &str, eldest_seqno: u64, ctime_ms: u64) -> Result<Proof> {
    let sikey = derive::stretch(token)?;
    let invite_id = derive::invite_id(&sikey)?;
    let key = derive::signing_key(&sikey)?;
    let message = message(uid, eldest_seqno, ctime_ms, &invite_id)?;
    let signature = key.sign(&message).to_bytes();
    Ok(Proof { message, signature })
}

/// Decodes an accept message. Any stage other than "accept", any version other than 2, and any malformed field is an
/// error.
///
/// # Errors
///
/// Returns [`ProofError::Malformed`] for any message that is not a version 2 accept with a valid invite ID.
pub fn parse(message: &[u8]) -> Result<Fields, ProofError> {
    let wire: Wire = rmp_serde::from_slice(message).map_err(|_| ProofError::Malformed)?;
    if wire.stage != "accept" || wire.version != VERSION {
        return Err(ProofError::Malformed);
    }
    let invite_id = InviteId::from_hex(&wire.invite_id).ok_or(ProofError::Malformed)?;
    Ok(Fields {
        uid: wire.uid,
        eldest_seqno: wire.eldest_seqno,
        ctime_ms: wire.ctime,
        invite_id,
    })
}

/// Checks the signature over the message bytes as received, against an invite's public key. It uses strict
/// verification, which rejects the malleable encodings that an honest signer never produces.
///
/// # Errors
///
/// Returns [`ProofError::BadKey`] if the key is not a valid point, and [`ProofError::BadSignature`] if the signature
/// does not verify.
pub fn verify(public_key: &[u8; 32], proof: &Proof) -> Result<(), ProofError> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| ProofError::BadKey)?;
    let signature = Signature::from_bytes(&proof.signature);
    key.verify_strict(&proof.message, &signature)
        .map_err(|_| ProofError::BadSignature)
}
