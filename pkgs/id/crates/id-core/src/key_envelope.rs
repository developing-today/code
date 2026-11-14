//! Versioned local passphrase-wrapped identity key format.
//!
//! The complete fixed-size header is authenticated as AEAD associated data.
//! Parsing validates all KDF parameters before running Argon2id so an imported
//! file cannot request unbounded work.

use anyhow::{Context, Result, bail};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::Aead, aead::Payload};
use rand::RngExt as _;
use zeroize::Zeroizing;

const MAGIC: &[u8; 5] = b"IDKEY";
const VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_XCHACHA20_POLY1305: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 5 + 3 + 12 + SALT_LEN + NONCE_LEN;
const ENVELOPE_LEN: usize = HEADER_LEN + KEY_LEN + TAG_LEN;

/// Version 1 writer parameters (Argon2id v=19): 19 MiB, t=2, p=1.
const WRITE_MEMORY_KIB: u32 = 19_456;
const WRITE_ITERATIONS: u32 = 2;
const WRITE_LANES: u32 = 1;

/// Resource bounds for parameters parsed from untrusted envelope bytes.
const MIN_MEMORY_KIB: u32 = 19_456;
const MAX_MEMORY_KIB: u32 = 262_144;
const MIN_ITERATIONS: u32 = 2;
const MAX_ITERATIONS: u32 = 10;
const MIN_LANES: u32 = 1;
const MAX_LANES: u32 = 4;

/// Encrypt a raw 32-byte identity key into the version 1 envelope.
pub fn encrypt(passphrase: &[u8], key: &[u8; KEY_LEN]) -> Result<Vec<u8>> {
    let mut salt = [0; SALT_LEN];
    let mut nonce = [0; NONCE_LEN];
    rand::rng().fill(&mut salt);
    rand::rng().fill(&mut nonce);

    let params = Params::new(
        WRITE_MEMORY_KIB,
        WRITE_ITERATIONS,
        WRITE_LANES,
        Some(KEY_LEN),
    )
    .context("invalid built-in key-envelope KDF parameters")?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut wrapping_key = Zeroizing::new([0; KEY_LEN]);
    argon2
        .hash_password_into(passphrase, &salt, &mut wrapping_key[..])
        .context("could not derive key-envelope wrapping key")?;

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&[VERSION, KDF_ARGON2ID, CIPHER_XCHACHA20_POLY1305]);
    header.extend_from_slice(&WRITE_MEMORY_KIB.to_le_bytes());
    header.extend_from_slice(&WRITE_ITERATIONS.to_le_bytes());
    header.extend_from_slice(&WRITE_LANES.to_le_bytes());
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);

    let cipher = XChaCha20Poly1305::new_from_slice(&wrapping_key[..])
        .map_err(|_| anyhow::anyhow!("invalid key-envelope wrapping key"))?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: key,
                aad: &header,
            },
        )
        .map_err(|_| anyhow::anyhow!("could not encrypt identity key"))?;

    let mut envelope = header;
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

/// Authenticate and decrypt a version 1 identity key envelope.
pub fn decrypt(passphrase: &[u8], envelope: &[u8]) -> Result<[u8; KEY_LEN]> {
    if envelope.len() != ENVELOPE_LEN {
        bail!("invalid key envelope length");
    }
    if &envelope[..MAGIC.len()] != MAGIC {
        bail!("invalid key envelope magic");
    }
    if envelope[5] != VERSION {
        bail!("unsupported key envelope version");
    }
    if envelope[6] != KDF_ARGON2ID {
        bail!("unsupported key envelope KDF");
    }
    if envelope[7] != CIPHER_XCHACHA20_POLY1305 {
        bail!("unsupported key envelope cipher");
    }

    let memory_kib = u32::from_le_bytes([envelope[8], envelope[9], envelope[10], envelope[11]]);
    let iterations = u32::from_le_bytes([envelope[12], envelope[13], envelope[14], envelope[15]]);
    let lanes = u32::from_le_bytes([envelope[16], envelope[17], envelope[18], envelope[19]]);
    if !(MIN_MEMORY_KIB..=MAX_MEMORY_KIB).contains(&memory_kib)
        || !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations)
        || !(MIN_LANES..=MAX_LANES).contains(&lanes)
        || memory_kib < lanes.saturating_mul(8)
    {
        bail!("key envelope KDF parameters are outside the supported range");
    }

    let salt = &envelope[20..20 + SALT_LEN];
    let nonce = &envelope[20 + SALT_LEN..HEADER_LEN];
    let header = &envelope[..HEADER_LEN];
    let params = Params::new(memory_kib, iterations, lanes, Some(KEY_LEN))
        .context("invalid key envelope KDF parameters")?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut wrapping_key = Zeroizing::new([0; KEY_LEN]);
    argon2
        .hash_password_into(passphrase, salt, &mut wrapping_key[..])
        .context("could not derive key-envelope wrapping key")?;

    let cipher = XChaCha20Poly1305::new_from_slice(&wrapping_key[..])
        .map_err(|_| anyhow::anyhow!("invalid key-envelope wrapping key"))?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: &envelope[HEADER_LEN..],
                aad: header,
            },
        )
        .map_err(|_| anyhow::anyhow!("wrong passphrase or damaged key envelope"))?;
    let plaintext = Zeroizing::new(plaintext);
    if plaintext.len() != KEY_LEN {
        bail!("invalid decrypted identity key length");
    }
    let mut key = [0; KEY_LEN];
    key.copy_from_slice(plaintext.as_slice());
    Ok(key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const TEST_KEY: [u8; KEY_LEN] = [0x5a; KEY_LEN];

    #[test]
    fn round_trips_identity_key() {
        let envelope = encrypt(b"correct horse battery staple", &TEST_KEY).unwrap();
        assert_eq!(envelope.len(), ENVELOPE_LEN);
        assert_eq!(
            decrypt(b"correct horse battery staple", &envelope).unwrap(),
            TEST_KEY
        );
    }

    #[test]
    fn rejects_wrong_passphrase() {
        let envelope = encrypt(b"correct horse battery staple", &TEST_KEY).unwrap();
        assert!(decrypt(b"wrong passphrase", &envelope).is_err());
    }

    #[test]
    fn rejects_truncated_and_tampered_envelopes() {
        let envelope = encrypt(b"correct horse battery staple", &TEST_KEY).unwrap();
        assert!(
            decrypt(
                b"correct horse battery staple",
                &envelope[..envelope.len() - 1]
            )
            .is_err()
        );

        let mut tampered = envelope;
        tampered[20] ^= 1;
        assert!(decrypt(b"correct horse battery staple", &tampered).is_err());
    }

    #[test]
    fn rejects_kdf_parameters_below_the_floor() {
        let mut envelope = encrypt(b"correct horse battery staple", &TEST_KEY).unwrap();
        envelope[8..12].copy_from_slice(&(MIN_MEMORY_KIB - 1).to_le_bytes());
        assert!(decrypt(b"correct horse battery staple", &envelope).is_err());
    }
}
