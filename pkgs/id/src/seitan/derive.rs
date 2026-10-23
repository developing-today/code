//! The Seitan V2 derivations, from keybase/client `go/teams/seitan.go` and `seitan_v2.go`.
//!
//! ```text
//! token --scrypt--> SiKey --HMAC-SHA512--> invite ID          (public)
//!                         --HMAC-SHA512--> Ed25519 seed --> signing key (the public half is published)
//! ```
//!
//! The scrypt cost is cheap on purpose, and that is safe only for a random token. Never derive from a token that
//! a person chose.

use std::fmt::{self, Write as _};

use anyhow::{Context as _, Result};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, KeyInit, Mac};
use scrypt::{Params, scrypt};
use serde::Serialize;
use sha2::Sha512;

use super::token::Token;

/// Keybase's scrypt cost: N = 2^10, r = 8, p = 1.
const SCRYPT_LOG_N: u8 = 10;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
/// The Seitan version in every payload.
pub(super) const VERSION: u8 = 2;
/// `libkb.InviteIDTag`. The Keybase vector ends in 0x27, which is the only check on this value.
const INVITE_ID_TAG: u8 = 0x27;

/// The stretched invite key. It is secret, and it never leaves the two parties.
#[derive(Clone, PartialEq, Eq)]
pub struct SiKey([u8; 32]);

impl SiKey {
    /// The raw key bytes. Do not log them.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SiKey(<redacted>)")
    }
}

/// The 16-byte invite ID: 15 bytes of HMAC output, then the tag byte. It is public. Its hex form is 32 characters.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InviteId([u8; 16]);

impl InviteId {
    /// The 32-character lowercase hex form.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.0 {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// Parses the 32-character hex form. Anything else returns `None`.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

impl fmt::Display for InviteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for InviteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InviteId({})", self.to_hex())
    }
}

#[derive(Serialize)]
struct StagePayload {
    stage: &'static str,
    version: u8,
}

/// `SeitanIKeyV2.GenerateSIKey`: scrypt over the lowercased token, with an empty salt.
///
/// # Errors
///
/// Fails only if the scrypt parameters are rejected, which the constants never trigger.
pub fn stretch(token: &Token) -> Result<SiKey> {
    stretch_bytes(token.as_str().as_bytes())
}

/// The scrypt stretch over raw bytes. The V1 vector uses this, since its token does not match the V2 shape.
pub(super) fn stretch_bytes(password: &[u8]) -> Result<SiKey> {
    let params = Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P).context("the scrypt parameters")?;
    let mut out = [0u8; 32];
    scrypt(password, &[], &params, &mut out).context("scrypt")?;
    Ok(SiKey(out))
}

/// `SeitanSIKeyV2.GenerateTeamInviteID`: the first 15 bytes of HMAC-SHA512 over `{stage: "invite_id", version: 2}`,
/// then the tag.
///
/// # Errors
///
/// Fails only if the payload cannot be encoded, which a fixed struct never triggers.
pub fn invite_id(sikey: &SiKey) -> Result<InviteId> {
    let mac = hmac_sha512(&sikey.0, &stage_payload("invite_id")?)?;
    let mut id = [0u8; 16];
    id[..15].copy_from_slice(&mac[..15]);
    id[15] = INVITE_ID_TAG;
    Ok(InviteId(id))
}

/// `SeitanSIKeyV2.generateKeyPair`: the Ed25519 seed is the first 32 bytes of HMAC-SHA512 over `{stage: "eddsa", version: 2}`.
///
/// # Errors
///
/// Fails only if the payload cannot be encoded, which a fixed struct never triggers.
pub fn signing_key(sikey: &SiKey) -> Result<SigningKey> {
    let mac = hmac_sha512(&sikey.0, &stage_payload("eddsa")?)?;
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&mac[..32]);
    Ok(SigningKey::from_bytes(&seed))
}

/// The msgpack map that Keybase's stage payload structs encode to. The fields are in declaration order.
pub(super) fn stage_payload(stage: &'static str) -> Result<Vec<u8>> {
    rmp_serde::to_vec_named(&StagePayload {
        stage,
        version: VERSION,
    })
    .context("encoding a stage payload")
}

fn hmac_sha512(key: &[u8], message: &[u8]) -> Result<[u8; 64]> {
    let mut mac = <Hmac<Sha512>>::new_from_slice(key).context("HMAC-SHA512")?;
    mac.update(message);
    let mut out = [0u8; 64];
    out.copy_from_slice(&mac.finalize().into_bytes());
    Ok(out)
}
