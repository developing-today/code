use sha2::{Digest, Sha512};

/// Derive the 32-byte secret-rotation key for a topic and minute.
///
/// This crate intentionally has no dependency on the DHT or its rotation
/// trait, keeping the deterministic derivation independently reusable.
pub fn derive(topic_hash: [u8; 32], unix_minute: u64, initial_secret_hash: [u8; 32]) -> [u8; 32] {
    let mut hash = Sha512::new();
    hash.update(topic_hash);
    hash.update(unix_minute.to_be_bytes());
    hash.update(initial_secret_hash);
    hash.update(b"rotate");
    hash.finalize()[..32]
        .try_into()
        .expect("SHA-512 output is 64 bytes")
}

#[cfg(test)]
mod tests {
    use super::derive;

    #[test]
    fn derive_matches_the_fixed_legacy_vector() {
        let topic_hash = [0x11; 32];
        let unix_minute = 0x0102_0304_0506_0708;
        let initial_secret_hash = [0x22; 32];
        let expected = [
            0x70, 0xf6, 0xbe, 0x26, 0xf8, 0x91, 0x27, 0xd2, 0xba, 0x46, 0x9b, 0x80, 0x58, 0xe3,
            0x37, 0x17, 0x02, 0x6b, 0x27, 0x4c, 0x01, 0x49, 0xe3, 0x2e, 0x1f, 0x2b, 0x0a, 0xb3,
            0x42, 0x3c, 0x70, 0xf1,
        ];

        assert_eq!(
            derive(topic_hash, unix_minute, initial_secret_hash),
            expected
        );
    }

    #[test]
    fn derive_returns_exactly_32_bytes() {
        let key = derive([0; 32], 0, [0; 32]);

        assert_eq!(key.len(), 32);
    }
}
