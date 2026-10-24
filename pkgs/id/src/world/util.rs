use super::*;

use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::watch;

/// Incremental event page for an authorized participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventPage {
    /// Latest committed sequence at response time.
    pub current_sequence: u64,
    /// Earliest retained sequence.
    pub oldest_retained_sequence: u64,
    /// The event cursor fell behind retention and a full snapshot is required.
    pub needs_snapshot: bool,
    /// Events newer than the requested cursor, if it is still retained.
    pub events: Vec<WorldEvent>,
}

pub(crate) fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

pub(crate) fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    bool::from(left.ct_eq(right))
}

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

/// Publish a new records snapshot through the watch channel when it differs.
pub fn publish_records(
    sender: &watch::Sender<(u64, Arc<Records>)>,
    sequence: u64,
    records: Records,
) {
    sender.send_if_modified(|current| {
        if *current.1 == records {
            false
        } else {
            *current = (sequence, Arc::new(records));
            true
        }
    });
}

pub(crate) fn hex_decode_32(encoded: &str) -> Option<[u8; 32]> {
    let bytes = encoded.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        let nibble = |b: u8| char::from(b).to_digit(16);
        *slot = u8::try_from(nibble(pair[0])? << 4 | nibble(pair[1])?).ok()?;
    }
    Some(out)
}

pub(crate) fn sanitize_display_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !ch.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(crate) fn sanitize_chat(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .collect::<String>()
        .trim()
        .to_owned()
}
