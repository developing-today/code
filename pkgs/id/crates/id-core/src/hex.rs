//! Hex encoding helpers (leaf utilities with no dependencies).
//!
//! `hex_encode` moved here from `id-worlds` (`world::hex_encode`)
//! as step 6a of the pkgs/id crate split (#3447 umbrella, #4409):
//! the directory modules use it, and `id-directory` must not depend
//! on the worlds crate. `id-worlds` re-exports it at its historical
//! path.

/// Lower-case hex encoding of a byte slice, two characters per byte.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
