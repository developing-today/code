//! Hex and key-text helpers for the wire layer. The facade's directory
//! keeps its own copies for the code that stayed behind (step 3c, #4406);
//! these serve the envelope and artifact modules here.

use anyhow::{Context, Result};
use ed25519_dalek::VerifyingKey;

/// Lower-case hex encoding of a byte slice, two characters per byte.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub(crate) fn hex_to_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    // Every byte must be an ASCII hex digit (#2517): u8::from_str_radix
    // also accepts a leading '+', so without this gate a byte 0x00-0x0f
    // has two accepted spellings ('0x' and '+x') and key strings stop
    // being canonical.
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0_u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// A public key as lower-case hex, if it is an Ed25519 public key.
///
/// # Errors
///
/// Fails if the text is not 64 hex characters or not a valid key.
pub(crate) fn normalize_key(key: &str) -> Result<String> {
    let key = key.trim().to_ascii_lowercase();
    let bytes = hex_to_array::<32>(&key).context("a public key is 64 hex characters")?;
    VerifyingKey::from_bytes(&bytes).context("not an Ed25519 public key")?;
    Ok(key)
}
