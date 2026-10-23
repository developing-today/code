//! The joiner's proof. It mirrors `GenerateSeitanSignatureMessage` and `GenerateSignature` in keybase/client `seitan_v2.go`.
//!
//! The joiner sends the message bytes and the signature. The admin verifies the signature over the bytes
//! exactly as received, and only then reads the fields out of them. Nothing is re-encoded before verifying.

use ed25519_dalek::{Signature, Signer, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::derive::{self, InviteId, VERSION};
use crate::token::Token;

/// What the joiner sends: the signed message bytes and the Ed25519 signature over them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    pub message: Vec<u8>,
    pub signature: [u8; 64],
}

/// The fields of a decoded accept message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    pub uid: String,
    pub eldest_seqno: u64,
    pub ctime_ms: u64,
    pub invite_id: InviteId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofError {
    Malformed,
    BadKey,
    BadSignature,
}

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

/// The message the joiner signs: `{stage: "accept", uid, eldest_seqno, ctime, invite_id, version: 2}`, as msgpack.
/// `ctime` is milliseconds since the Unix epoch. `invite_id` is the 32-character hex form.
pub fn message(uid: &str, eldest_seqno: u64, ctime_ms: u64, invite_id: &InviteId) -> Vec<u8> {
    rmp_serde::to_vec_named(&Wire {
        stage: "accept".to_owned(),
        uid: uid.to_owned(),
        eldest_seqno,
        ctime: ctime_ms,
        invite_id: invite_id.to_hex(),
        version: VERSION,
    })
    .expect("encoding a fixed struct cannot fail")
}

/// Joiner side. The joiner types the token, derives the key, and signs. This changes no membership state.
pub fn sign(token: &Token, uid: &str, eldest_seqno: u64, ctime_ms: u64) -> Proof {
    let sikey = derive::stretch(token);
    let invite_id = derive::invite_id(&sikey);
    let key = derive::signing_key(&sikey);
    let message = message(uid, eldest_seqno, ctime_ms, &invite_id);
    let signature = key.sign(&message).to_bytes();
    Proof { message, signature }
}

/// Decodes an accept message. Any stage other than "accept", any version other than 2, and any malformed field is an error.
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

/// Checks the signature over the message bytes as received, against an invite's public key.
pub fn verify(public_key: &[u8; 32], proof: &Proof) -> Result<(), ProofError> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| ProofError::BadKey)?;
    let signature = Signature::from_bytes(&proof.signature);
    key.verify_strict(&proof.message, &signature)
        .map_err(|_| ProofError::BadSignature)
}
