//! Canonical CBOR for signed bytes.
//!
//! Canonical means the bytes decode to a value and encoding that value gives the same bytes back.
//! That single check is the rule. On top of it, a signed value may hold only NFC text, integers,
//! byte strings, arrays, maps, false, true, and null. Floats, undefined, other simple values, and
//! tags are refused.

use cbor_core::{DecodeOptions, Strictness, Value};
use unicode_normalization::is_nfc;

/// Nesting limit for signed values. p2panda-core's header decoder uses the same.
const RECURSION_LIMIT: u16 = 64;

/// Why a byte string is not an accepted signed value.
#[derive(Debug)]
pub enum Error {
    /// cbor-core's strict decoder refused the bytes.
    Decode(cbor_core::Error),
    /// The bytes decode, but the value encodes to different bytes.
    NotCanonical,
    /// A text string or a map key is not in Unicode NFC.
    NotNfc,
    /// A float. Signed values carry none.
    Float,
    /// A simple value other than false, true, or null. 23 is undefined.
    SimpleValue(u8),
    /// A tag. No signed value uses one.
    Tag(u64),
}

impl From<cbor_core::Error> for Error {
    fn from(err: cbor_core::Error) -> Self {
        Self::Decode(err)
    }
}

/// Decodes one signed value. Accepts only bytes that are the canonical encoding of the value
/// they denote.
pub fn decode(bytes: &[u8]) -> Result<Value<'static>, Error> {
    let value = DecodeOptions::new()
        .recursion_limit(RECURSION_LIMIT)
        .strictness(Strictness::STRICT)
        .decode_owned(bytes)?;
    if value.encode() != bytes {
        return Err(Error::NotCanonical);
    }
    check(&value)?;
    Ok(value)
}

/// The canonical bytes of a value. Use this for everything that is signed.
pub fn encode(value: &Value) -> Vec<u8> {
    value.encode()
}

/// Admits a public key: a point on the curve that is not of small order. The identity and other
/// low-order keys are refused here, once, so later checks never see them.
pub fn admit_key(bytes: &[u8; 32]) -> Option<ed25519_dalek::VerifyingKey> {
    ed25519_dalek::VerifyingKey::from_bytes(bytes)
        .ok()
        .filter(|key| !key.is_weak())
}

/// Verifies an Ed25519 signature over signed bytes with `verify_strict`, which also refuses a
/// small-order R. A non-canonical S (S >= L) is refused while parsing, in ed25519-dalek 2.x's
/// default build (the `legacy_compatibility` feature turns that check off).
pub fn verify(
    key: &ed25519_dalek::VerifyingKey,
    signed: &[u8],
    signature: &[u8; 64],
) -> Result<(), ed25519_dalek::SignatureError> {
    key.verify_strict(signed, &ed25519_dalek::Signature::from_bytes(signature))
}

/// The value rules, applied after the canonicality check.
fn check(value: &Value) -> Result<(), Error> {
    match value {
        Value::Unsigned(_) | Value::Negative(_) | Value::ByteString(_) => Ok(()),
        Value::TextString(text) if is_nfc(text) => Ok(()),
        Value::TextString(_) => Err(Error::NotNfc),
        Value::Array(items) => items.iter().try_for_each(check),
        Value::Map(map) => map.iter().try_for_each(|(key, val)| {
            check(key)?;
            check(val)
        }),
        Value::SimpleValue(simple) => match simple.to_u8() {
            20..=22 => Ok(()),
            other => Err(Error::SimpleValue(other)),
        },
        Value::Float(_) => Err(Error::Float),
        Value::Tag(number, _) => Err(Error::Tag(*number)),
    }
}
