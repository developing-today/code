//! Signatures that a strict Ed25519 verifier must refuse. The regression tests
//! at each call site that verifies a signature share these fixtures.

/// The identity point, a public key of small order. Its encoding is `01`
/// followed by 31 zero bytes.
pub const IDENTITY_KEY: [u8; 32] = {
    let mut key = [0_u8; 32];
    key[0] = 1;
    key
};

/// The prime group order L, little-endian.
const GROUP_ORDER: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

/// A signature that plain `verify` accepts for any message under
/// [`IDENTITY_KEY`]: R is the identity and S is zero, so `[S]B = R + [k]A`
/// holds whatever `k` is. `verify_strict` refuses it because the key is small.
#[must_use]
pub fn identity_forgery() -> [u8; 64] {
    let mut signature = [0_u8; 64];
    signature[0] = 1;
    signature
}

/// The signature with S replaced by S + L. It is the same equation modulo L,
/// so only the encoding changes: S is no longer less than L. A canonical S is
/// below L, so the sum always fits in 32 bytes.
#[must_use]
pub fn with_non_canonical_s(signature: [u8; 64]) -> [u8; 64] {
    let mut out = signature;
    let mut carry = 0_u16;
    for (index, byte) in out[32..].iter_mut().enumerate() {
        let sum = u16::from(*byte) + u16::from(GROUP_ORDER[index]) + carry;
        let [low, high] = sum.to_le_bytes();
        *byte = low;
        carry = u16::from(high);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

    #[test]
    fn the_identity_forgery_passes_plain_verify_and_fails_strict() {
        let key = VerifyingKey::from_bytes(&IDENTITY_KEY).unwrap();
        let signature = Signature::from_bytes(&identity_forgery());
        for message in [b"".as_slice(), b"any message".as_slice()] {
            assert!(key.verify(message, &signature).is_ok());
            assert!(key.verify_strict(message, &signature).is_err());
        }
    }

    #[test]
    fn a_non_canonical_s_fails_both_forms_and_the_original_passes() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let message = b"canonical";
        let good = key.sign(message).to_bytes();
        let public = key.verifying_key();
        assert!(
            public
                .verify_strict(message, &Signature::from_bytes(&good))
                .is_ok()
        );
        let malleated = Signature::from_bytes(&with_non_canonical_s(good));
        assert!(public.verify(message, &malleated).is_err());
        assert!(public.verify_strict(message, &malleated).is_err());
    }

    #[test]
    fn every_byte_change_to_a_valid_signature_fails_strict() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let message = b"mutate me";
        let good = key.sign(message).to_bytes();
        let public = key.verifying_key();
        for index in 0..64 {
            for mask in [0x01_u8, 0x80, 0xff] {
                let mut changed = good;
                changed[index] ^= mask;
                let signature = Signature::from_bytes(&changed);
                assert!(
                    public.verify_strict(message, &signature).is_err(),
                    "byte {index} xor {mask:#x} was accepted"
                );
            }
        }
    }
}
