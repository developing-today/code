//! The Seitan V2 derivations, from keybase/client `go/teams/seitan.go` and `seitan_v2.go`.
//!
//! ```text
//! token --scrypt--> SiKey --HMAC-SHA512--> invite ID          (public)
//!                         --HMAC-SHA512--> Ed25519 seed --> signing key (public half is published)
//! ```

use std::fmt;

use ed25519_dalek::SigningKey;
use hmac::{Hmac, KeyInit, Mac};
use scrypt::{Params, scrypt};
use serde::Serialize;
use sha2::Sha512;

use crate::token::Token;

/// Keybase's scrypt cost: N = 2^10, r = 8, p = 1. It is cheap on purpose, so it is only safe
/// for random tokens. Never derive from a token that a person chose.
const SCRYPT_LOG_N: u8 = 10;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
/// The Seitan version in every payload.
pub const VERSION: u8 = 2;
/// `libkb.InviteIDTag`. The Keybase vector ends in 0x27, which is the only check on this value.
const INVITE_ID_TAG: u8 = 0x27;

/// The stretched invite key. Secret: it never leaves the two parties.
#[derive(Clone, PartialEq, Eq)]
pub struct SiKey([u8; 32]);

impl SiKey {
    /// The raw key bytes. Do not log them.
    pub fn as_bytes(&self) -> &[u8; 32] {
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
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Parses the 32-character hex form. Anything else returns `None`.
    pub fn from_hex(s: &str) -> Option<InviteId> {
        if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(InviteId(out))
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
pub fn stretch(token: &Token) -> SiKey {
    stretch_bytes(token.as_str().as_bytes())
}

fn stretch_bytes(password: &[u8]) -> SiKey {
    let params = Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P)
        .expect("constant scrypt parameters are valid");
    let mut out = [0u8; 32];
    scrypt(password, &[], &params, &mut out).expect("32 is a valid output length");
    SiKey(out)
}

/// `SeitanSIKeyV2.GenerateTeamInviteID`: the first 15 bytes of HMAC-SHA512 over `{stage: "invite_id", version: 2}`, then the tag.
pub fn invite_id(sikey: &SiKey) -> InviteId {
    let mac = hmac_sha512(&sikey.0, &stage_payload("invite_id"));
    let mut id = [0u8; 16];
    id[..15].copy_from_slice(&mac[..15]);
    id[15] = INVITE_ID_TAG;
    InviteId(id)
}

/// `SeitanSIKeyV2.generateKeyPair`: the Ed25519 seed is the first 32 bytes of HMAC-SHA512 over `{stage: "eddsa", version: 2}`.
pub fn signing_key(sikey: &SiKey) -> SigningKey {
    let mac = hmac_sha512(&sikey.0, &stage_payload("eddsa"));
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&mac[..32]);
    SigningKey::from_bytes(&seed)
}

/// The msgpack map that Keybase's stage payload structs encode to. Fields are in declaration order.
fn stage_payload(stage: &'static str) -> Vec<u8> {
    rmp_serde::to_vec_named(&StagePayload {
        stage,
        version: VERSION,
    })
    .expect("encoding a fixed struct cannot fail")
}

fn hmac_sha512(key: &[u8], message: &[u8]) -> [u8; 64] {
    let mut mac = <Hmac<Sha512>>::new_from_slice(key).expect("HMAC takes keys of any length");
    mac.update(message);
    let mut out = [0u8; 64];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::STANDARD as B64};

    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[derive(Serialize)]
    struct V1Stage {
        stage: &'static str,
    }

    #[test]
    fn stage_payloads_are_the_keybase_maps_byte_for_byte() {
        // Encoded by hand from the msgpack spec: fixmap(2), then fixstr keys and values in declaration order.
        assert_eq!(
            hex(&stage_payload("invite_id")),
            "82a57374616765a9696e766974655f6964a776657273696f6e02"
        );
        assert_eq!(
            hex(&stage_payload("eddsa")),
            "82a57374616765a56564647361a776657273696f6e02"
        );
    }

    #[test]
    fn v1_vector_checks_the_steps_v1_and_v2_share() {
        // keybase/client go/teams/seitan_test.go, TestSeitanKnownSamples (V1). A V1 token has '+' at index 5,
        // so this skips Token::parse. It checks scrypt, the HMAC-SHA512 truncation, and the tag.
        let sikey = stretch_bytes(b"raw2ewqp249dyod4");
        assert_eq!(
            B64.encode(sikey.0),
            "Yqbj8NgHkIG03wfZX/dxpBpqFoXPXNXyQr+MnvCMbS4="
        );
        let payload = rmp_serde::to_vec_named(&V1Stage { stage: "invite_id" }).unwrap();
        let mac = hmac_sha512(&sikey.0, &payload);
        let mut id = mac[..15].to_vec();
        id.push(INVITE_ID_TAG);
        assert_eq!(hex(&id), "24189cc0ad5851ac52404ee99c7c9c27");
    }

    #[test]
    fn invite_id_hex_round_trips_and_rejects_other_forms() {
        let id = InviteId::from_hex("6303ec43bd61d21edb95a433faf06227").unwrap();
        assert_eq!(id.to_hex(), "6303ec43bd61d21edb95a433faf06227");
        assert!(InviteId::from_hex("6303ec43bd61d21edb95a433faf0622").is_none());
        assert!(InviteId::from_hex("+303ec43bd61d21edb95a433faf06227").is_none());
        assert!(InviteId::from_hex("6303ec43bd61d21edb95a433faf0622g").is_none());
    }
}
