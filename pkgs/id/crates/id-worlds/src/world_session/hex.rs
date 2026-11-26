//! Bounded hex encoding for world session frames.
//!
//! Split from the session module (#12671) to keep it under the
//! production line limit; `world_session` re-exports both functions,
//! so callers keep their `world_session::` paths.

use super::MAX_FRAME_BYTES;

/// Decode a bounded hex string.
#[must_use]
pub fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) || encoded.len() > MAX_FRAME_BYTES * 2 {
        return None;
    }
    encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

/// Encode bytes as lowercase hex.
pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
