//! Array-address core: nullable ordered byte-slot addresses and their codec.
//!
//! Slice A9118-array-codec (issue #10182), design
//! `part-a-addressing/versioned-design.md` section "#9118 — Array-address
//! core", statements S4.23–S4.28: an [`ArrayAddress`] is a list of
//! `Option<Bytes>` slots with significant order. The empty list is a valid
//! zero-length address. A `None` slot is a meaningful null; `Some([])` is
//! empty bytes and is distinct from `None`; an omitted list position does
//! not exist. There is no fixed semantic maximum.
//!
//! Ordering is componentwise typed ordering — null before bytes, bytes by
//! unsigned lexicographic comparison with a strict prefix sorting before
//! its extension — and then a shorter equal-prefix list before a longer
//! one. That is exactly the standard `Ord` of `Vec<Option<Vec<u8>>>`, and
//! [`ArrayAddress`] delegates to it so the two can never drift.
//!
//! The binary serialization is versioned, lossless and canonical: for
//! every admitted value `decode(encode(x)) == x`, and for canonical bytes
//! `encode(decode(bytes)) == bytes` (the format admits exactly one
//! encoding per value, and the decoder rejects anything else). The URI
//! slot-token form writes `~` for a null slot and `b.` followed by
//! percent-encoded octets for a bytes slot, tokens joined by `/`.
//!
//! This module is intentionally dependency-free (pure `std`): the crate
//! root export and manifest wiring belong to slice 9110-02; until that
//! lands, the integration tests include this file by path.

use std::cmp::Ordering;
use std::fmt;

/// Version byte of the binary array-address format defined here.
pub const FORMAT_VERSION: u8 = 0x01;

const TAG_NULL: u8 = 0x00;
const TAG_BYTES: u8 = 0x01;

/// One address: an ordered list of nullable byte slots.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArrayAddress {
    slots: Vec<Option<Vec<u8>>>,
}

impl ArrayAddress {
    /// Build an address from its slots, in order.
    pub fn new(slots: Vec<Option<Vec<u8>>>) -> Self {
        Self { slots }
    }

    /// The zero-length address `[]`.
    pub fn empty() -> Self {
        Self { slots: Vec::new() }
    }

    /// The slots, in order. `None` is a null slot; `Some(vec![])` is an
    /// empty-bytes slot, which is a different value.
    pub fn slots(&self) -> &[Option<Vec<u8>>] {
        &self.slots
    }

    /// Number of slots (null slots included).
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether this is the zero-length address.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Encode to the canonical versioned binary form:
    /// `version(1) || slot_count(u64 BE) || per slot: tag(0=null, 1=bytes)
    /// [|| payload_len(u64 BE) || payload]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&(self.slots.len() as u64).to_be_bytes());
        for slot in &self.slots {
            match slot {
                None => out.push(TAG_NULL),
                Some(bytes) => {
                    out.push(TAG_BYTES);
                    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
                    out.extend_from_slice(bytes);
                }
            }
        }
        out
    }

    /// Decode the canonical binary form. Rejects any other version,
    /// truncated input, unknown slot tags, overrun lengths and trailing
    /// bytes, so non-canonical bytes never decode.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut cur = Cursor { bytes, pos: 0 };
        let version = cur.byte()?;
        if version != FORMAT_VERSION {
            return Err(DecodeError::BadVersion(version));
        }
        let count = cur.u64()?;
        // Every slot costs at least its tag byte, so a count larger than
        // the remaining input is certainly truncated; this also bounds
        // the allocation below by the input size.
        if count > cur.remaining() as u64 {
            return Err(DecodeError::Truncated);
        }
        let count = usize::try_from(count).map_err(|_| DecodeError::TooLarge)?;
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            match cur.byte()? {
                TAG_NULL => slots.push(None),
                TAG_BYTES => {
                    let len = cur.u64()?;
                    if len > cur.remaining() as u64 {
                        return Err(DecodeError::Truncated);
                    }
                    let len = usize::try_from(len).map_err(|_| DecodeError::TooLarge)?;
                    slots.push(Some(cur.take(len)?.to_vec()));
                }
                tag => return Err(DecodeError::UnknownTag(tag)),
            }
        }
        if cur.remaining() != 0 {
            return Err(DecodeError::TrailingBytes);
        }
        Ok(Self { slots })
    }

    /// The URI slot-token form: `~` for a null slot, `b.` plus
    /// percent-encoded octets for a bytes slot, joined by `/`. The
    /// zero-length address is the empty string.
    pub fn to_uri_tokens(&self) -> String {
        let mut out = String::new();
        for (index, slot) in self.slots.iter().enumerate() {
            if index > 0 {
                out.push('/');
            }
            match slot {
                None => out.push('~'),
                Some(bytes) => {
                    out.push_str("b.");
                    for byte in bytes {
                        push_percent_encoded(&mut out, *byte);
                    }
                }
            }
        }
        out
    }

    /// Parse the URI slot-token form produced by
    /// [`ArrayAddress::to_uri_tokens`]. Percent escapes may use either
    /// hex case on input; encoding always emits uppercase, so canonical
    /// text round-trips exactly.
    pub fn from_uri_tokens(text: &str) -> Result<Self, DecodeError> {
        if text.is_empty() {
            return Ok(Self::empty());
        }
        let mut slots = Vec::new();
        for token in text.split('/') {
            if token == "~" {
                slots.push(None);
            } else if let Some(payload) = token.strip_prefix("b.") {
                slots.push(Some(percent_decode(payload)?));
            } else {
                return Err(DecodeError::BadToken);
            }
        }
        Ok(Self { slots })
    }
}

impl PartialOrd for ArrayAddress {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ArrayAddress {
    /// Componentwise typed ordering (S4.25): null before bytes, bytes
    /// unsigned-lexicographic with a strict prefix first, then a shorter
    /// equal-prefix list first — the standard order of the slot vector.
    fn cmp(&self, other: &Self) -> Ordering {
        self.slots.cmp(&other.slots)
    }
}

impl fmt::Display for ArrayAddress {
    /// The URI slot-token form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_uri_tokens())
    }
}

/// Why a byte string or token string is not a canonical array address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// First byte is not [`FORMAT_VERSION`].
    BadVersion(u8),
    /// Input ends before the value is complete.
    Truncated,
    /// Extra bytes follow a complete value.
    TrailingBytes,
    /// A slot tag other than null (0) or bytes (1).
    UnknownTag(u8),
    /// A declared count or length does not fit this platform.
    TooLarge,
    /// A URI token that is neither `~` nor `b.`-prefixed.
    BadToken,
    /// A malformed percent escape in a URI token.
    BadPercentEscape,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadVersion(v) => write!(f, "unsupported array-address format version {v}"),
            Self::Truncated => f.write_str("truncated array-address encoding"),
            Self::TrailingBytes => f.write_str("trailing bytes after array-address encoding"),
            Self::UnknownTag(t) => write!(f, "unknown array-address slot tag {t}"),
            Self::TooLarge => f.write_str("array-address count or length too large"),
            Self::BadToken => f.write_str("array-address URI token is neither ~ nor b.-prefixed"),
            Self::BadPercentEscape => {
                f.write_str("malformed percent escape in array-address URI token")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let byte = *self.bytes.get(self.pos).ok_or(DecodeError::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < len {
            return Err(DecodeError::Truncated);
        }
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let raw = self.take(8)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(raw);
        Ok(u64::from_be_bytes(buf))
    }
}

/// Bytes allowed raw in a URI token payload: the RFC 3986 unreserved set.
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn push_percent_encoded(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    if is_unreserved(byte) {
        out.push(char::from(byte));
    } else {
        out.push('%');
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn percent_decode(payload: &str) -> Result<Vec<u8>, DecodeError> {
    let raw = payload.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' => {
                let hi = hex_value(*raw.get(index + 1).ok_or(DecodeError::BadPercentEscape)?)?;
                let lo = hex_value(*raw.get(index + 2).ok_or(DecodeError::BadPercentEscape)?)?;
                out.push(hi << 4 | lo);
                index += 3;
            }
            byte if is_unreserved(byte) => {
                out.push(byte);
                index += 1;
            }
            _ => return Err(DecodeError::BadPercentEscape),
        }
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Result<u8, DecodeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DecodeError::BadPercentEscape),
    }
}
