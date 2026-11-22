//! Panic-free parsing of Iroh blob hash text.
//!
//! `iroh_blobs::Hash`'s `FromStr` reads 64 lowercase hex characters or
//! 52 unpadded base32 characters, but on any other length it panics
//! inside its base32 decoder (a hard `assert_eq!` in
//! `data_encoding::Encoding::decode_mut`) instead of returning an error.
//! Any hash text that comes from a request, a file or a user must be
//! parsed with [`parse_hash`], which rejects other lengths first and
//! then accepts exactly what Iroh accepts.

use std::fmt;
use std::str::FromStr as _;

use iroh_blobs::Hash;

/// Length of Iroh's hex hash text.
pub const HEX_LEN: usize = 64;

/// Length of Iroh's unpadded base32 hash text.
pub const BASE32_LEN: usize = 52;

/// The text is not an Iroh hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseHashError;

impl fmt::Display for ParseHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a hash: expected 64 hex or 52 base32 characters")
    }
}

impl std::error::Error for ParseHashError {}

/// Parses Iroh hash text without panicking.
///
/// # Errors
///
/// [`ParseHashError`] for anything Iroh would not accept, including
/// lengths at which `iroh_blobs::Hash::from_str` itself panics.
pub fn parse_hash(text: &str) -> Result<Hash, ParseHashError> {
    if text.len() != HEX_LEN && text.len() != BASE32_LEN {
        return Err(ParseHashError);
    }
    Hash::from_str(text).map_err(|_error| ParseHashError)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn accepts_iroh_hex_and_base32() {
        let hash = Hash::new(b"abc");
        let hex = hash.to_string();
        assert_eq!(hex.len(), HEX_LEN);
        assert_eq!(parse_hash(&hex).unwrap(), hash);
        // Base32 of BLAKE3("abc"), RFC 4648 alphabet, no padding.
        let base32 = "MQ33HLBYIZITH75WHN2SOOUNWVEMKWCGLV45WA75GWOGZVN5TWCQ";
        assert_eq!(parse_hash(base32).unwrap(), hash);
        assert_eq!(parse_hash(&base32.to_ascii_lowercase()).unwrap(), hash);
    }

    #[test]
    fn rejects_other_lengths_without_panicking() {
        let hex = Hash::new(b"abc").to_string();
        for text in [
            "",
            "abc",
            &hex[..63],
            &format!("{hex}0"),
            "not a hash at all",
        ] {
            assert_eq!(parse_hash(text), Err(ParseHashError), "{text:?}");
        }
    }

    #[test]
    fn rejects_what_iroh_rejects_at_valid_lengths() {
        let hex = Hash::new(b"abc").to_string();
        // Iroh's hex reader is lowercase only.
        assert!(parse_hash(&hex.to_ascii_uppercase()).is_err());
        assert!(Hash::from_str(&hex.to_ascii_uppercase()).is_err());
        assert!(parse_hash(&"z".repeat(HEX_LEN)).is_err());
        assert!(parse_hash(&"1".repeat(BASE32_LEN)).is_err());
    }
}
