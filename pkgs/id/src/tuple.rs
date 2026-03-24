//! FoundationDB-style tuple encoding for sort-preserving binary keys.
//!
//! Encodes structured data into binary keys where lexicographic byte order
//! preserves semantic ordering. Supports prefix queries at any depth —
//! field boundaries, mid-field, and mid-array.
//!
//! # Type tags and sort order
//!
//! ```text
//! Tag(s)      Type         Payload
//! 0x00        Null         none (0x00 0xFF inside nested tuples)
//! 0x01        Bytes        escaped data + 0x00 terminator
//! 0x02        String       escaped UTF-8 + 0x00 terminator
//! 0x05        Tuple/Array  recursive elements + 0x00 end
//! 0x06        False        none
//! 0x07        True         none
//! 0x0C–0x13   Neg int      8–1 bytes ones-complement big-endian
//! 0x14        Zero         none
//! 0x15–0x1C   Pos int      1–8 bytes big-endian
//! 0x21        Float64      8 bytes sort-adjusted IEEE 754
//! ```
//!
//! Sort: `null < bytes < string < tuple < false < true < -big…-small < 0 < small…big < float`
//!
//! # Examples
//!
//! ```
//! use id::tuple::{TupleEncoder, decode};
//!
//! // Encode a key for: file="README.md", tag="label", value="rust"
//! let key = TupleEncoder::new()
//!     .string("README.md")
//!     .string("label")
//!     .string("rust")
//!     .build();
//!
//! // Decode it back
//! let values = decode(&key).unwrap();
//! assert_eq!(values[0].as_str(), Some("README.md"));
//! assert_eq!(values[1].as_str(), Some("label"));
//! assert_eq!(values[2].as_str(), Some("rust"));
//!
//! // Prefix query: all tags for README.md
//! let prefix = TupleEncoder::new().string("README.md").build();
//! assert!(key.starts_with(&prefix));
//! ```

use anyhow::{bail, ensure, Result};

// ============================================================================
// Type tag constants
// ============================================================================

const NULL: u8 = 0x00;
const BYTES: u8 = 0x01;
const STRING: u8 = 0x02;
const NESTED: u8 = 0x05;
const FALSE: u8 = 0x06;
const TRUE: u8 = 0x07;
const NEG_INT_8: u8 = 0x0C;
const ZERO: u8 = 0x14;
const POS_INT_1: u8 = 0x15;
const POS_INT_8: u8 = 0x1C;
const FLOAT64: u8 = 0x21;
const ESC: u8 = 0xFF;

// ============================================================================
// Value type
// ============================================================================

/// A decoded tuple element.
#[derive(Debug, Clone, PartialEq)]
pub enum TupleValue {
    /// Null value.
    Null,
    /// Raw byte sequence.
    Bytes(Vec<u8>),
    /// UTF-8 string.
    String(String),
    /// Nested tuple (also used for arrays).
    Tuple(Vec<TupleValue>),
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit IEEE 754 float (NaN excluded).
    Float(f64),
}

impl TupleValue {
    /// Extract as string reference.
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Extract as byte slice.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        if let Self::Bytes(b) = self {
            Some(b)
        } else {
            None
        }
    }

    /// Extract integer value.
    pub fn as_int(&self) -> Option<i64> {
        if let Self::Int(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extract float value.
    pub fn as_float(&self) -> Option<f64> {
        if let Self::Float(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extract boolean value.
    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    /// Extract tuple/array elements.
    pub fn as_tuple(&self) -> Option<&[TupleValue]> {
        if let Self::Tuple(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Check if null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

// ============================================================================
// Encoder
// ============================================================================

/// Builder for sort-preserving binary keys.
///
/// All methods except [`float`](Self::float) return `&mut Self` for chaining.
/// `float()` returns `Result` because NaN is rejected.
///
/// Use `*_prefix()` methods to omit terminators for prefix-match queries.
///
/// # Examples
///
/// ```
/// use id::tuple::TupleEncoder;
///
/// // Encode a struct: {a: 1, b: [0, 1, 2], c: null}
/// let key = TupleEncoder::new()
///     .int(1)
///     .array(|t| { t.int(0).int(1).int(2); })
///     .null()
///     .build();
///
/// // Prefix: match all entries where a=1
/// let prefix = TupleEncoder::new().int(1).build();
/// assert!(key.starts_with(&prefix));
///
/// // Prefix: match entries where a=1 and b starts with [0, 1]
/// let prefix = TupleEncoder::new()
///     .int(1)
///     .array_prefix(|t| { t.int(0).int(1); })
///     .build();
/// assert!(key.starts_with(&prefix));
/// ```
pub struct TupleEncoder {
    buf: Vec<u8>,
    nested: bool,
}

impl TupleEncoder {
    /// Create a new encoder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            nested: false,
        }
    }

    /// Encode a null value.
    pub fn null(&mut self) -> &mut Self {
        self.buf.push(NULL);
        if self.nested {
            self.buf.push(ESC);
        }
        self
    }

    /// Encode a byte slice (full field with terminator).
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.push(BYTES);
        push_esc(&mut self.buf, b);
        self.buf.push(NULL);
        self
    }

    /// Encode byte prefix (no terminator — for "starts with" queries).
    pub fn bytes_prefix(&mut self, b: &[u8]) -> &mut Self {
        self.buf.push(BYTES);
        push_esc(&mut self.buf, b);
        self
    }

    /// Encode a UTF-8 string (full field with terminator).
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.buf.push(STRING);
        push_esc(&mut self.buf, s.as_bytes());
        self.buf.push(NULL);
        self
    }

    /// Encode string prefix (no terminator — for "starts with" queries).
    pub fn string_prefix(&mut self, s: &str) -> &mut Self {
        self.buf.push(STRING);
        push_esc(&mut self.buf, s.as_bytes());
        self
    }

    /// Encode a boolean value.
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.buf.push(if v { TRUE } else { FALSE });
        self
    }

    /// Encode a signed 64-bit integer.
    pub fn int(&mut self, v: i64) -> &mut Self {
        push_int(&mut self.buf, v);
        self
    }

    /// Encode a 64-bit float. Rejects NaN. Canonicalizes `-0.0` to `+0.0`.
    pub fn float(&mut self, v: f64) -> Result<&mut Self> {
        if v.is_nan() {
            bail!("NaN cannot be encoded in tuple keys");
        }
        push_f64(&mut self.buf, v);
        Ok(self)
    }

    /// Encode a nested tuple/array (complete, with end marker).
    pub fn array(&mut self, f: impl FnOnce(&mut Self)) -> &mut Self {
        self.buf.push(NESTED);
        let prev = self.nested;
        self.nested = true;
        f(self);
        self.nested = prev;
        self.buf.push(NULL);
        self
    }

    /// Encode a tuple/array prefix (no end marker — for prefix queries).
    ///
    /// Matches any array whose leading elements match the encoded elements.
    pub fn array_prefix(&mut self, f: impl FnOnce(&mut Self)) -> &mut Self {
        self.buf.push(NESTED);
        let prev = self.nested;
        self.nested = true;
        f(self);
        self.nested = prev;
        self
    }

    /// Return encoded bytes and reset the encoder.
    #[must_use]
    pub fn build(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

impl Default for TupleEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Encoding internals
// ============================================================================

/// Push bytes with `0x00` → `0x00 0xFF` escaping (caller adds terminator).
fn push_esc(buf: &mut Vec<u8>, data: &[u8]) {
    for &b in data {
        buf.push(b);
        if b == NULL {
            buf.push(ESC);
        }
    }
}

/// Minimum bytes to represent a nonzero u64 in big-endian.
fn byte_len(v: u64) -> usize {
    let bits = 64 - v.leading_zeros() as usize;
    (bits + 7) / 8
}

/// Encode a signed 64-bit integer into `buf`.
fn push_int(buf: &mut Vec<u8>, v: i64) {
    if v == 0 {
        buf.push(ZERO);
    } else if v > 0 {
        let u = v as u64;
        let n = byte_len(u);
        buf.push(POS_INT_1 + n as u8 - 1);
        let be = u.to_be_bytes();
        buf.extend_from_slice(&be[8 - n..]);
    } else {
        let abs = v.unsigned_abs();
        let n = byte_len(abs);
        buf.push(ZERO - n as u8); // tag: 0x13 for 1 byte, 0x0C for 8 bytes
        let be = abs.to_be_bytes();
        for &b in &be[8 - n..] {
            buf.push(!b); // ones-complement
        }
    }
}

/// Encode a float64 with sort-preserving IEEE 754 transformation.
fn push_f64(buf: &mut Vec<u8>, v: f64) {
    // Canonicalize -0.0 to +0.0
    let v = if v == 0.0 { 0.0f64 } else { v };
    let mut bytes = v.to_be_bytes();
    if bytes[0] & 0x80 != 0 {
        // Negative: flip all bits so more-negative sorts earlier
        for b in &mut bytes {
            *b = !*b;
        }
    } else {
        // Positive (including +0.0): flip sign bit so positives sort after negatives
        bytes[0] ^= 0x80;
    }
    buf.push(FLOAT64);
    buf.extend_from_slice(&bytes);
}

// ============================================================================
// Decoder
// ============================================================================

/// Decode all elements from tuple-encoded bytes.
///
/// # Errors
///
/// Returns error for malformed data: truncated fields, invalid UTF-8 in
/// string elements, or unknown type tags.
pub fn decode(bytes: &[u8]) -> Result<Vec<TupleValue>> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < bytes.len() {
        match decode_one(bytes, &mut pos, false)? {
            Some(v) => out.push(v),
            None => bail!("unexpected end-of-tuple marker at top level"),
        }
    }
    Ok(out)
}

/// Decode a single element. Returns `None` for end-of-nested-tuple marker.
fn decode_one(data: &[u8], pos: &mut usize, nested: bool) -> Result<Option<TupleValue>> {
    ensure!(*pos < data.len(), "unexpected end of tuple data");
    let tag = data[*pos];
    *pos += 1;

    match tag {
        // Null or end-of-nested-tuple
        NULL => {
            if nested {
                // Inside nested tuple: 0x00 0xFF = null element, bare 0x00 = end
                if *pos < data.len() && data[*pos] == ESC {
                    *pos += 1;
                    Ok(Some(TupleValue::Null))
                } else {
                    Ok(None) // end of nested tuple
                }
            } else {
                Ok(Some(TupleValue::Null))
            }
        }

        // Bytes
        BYTES => {
            let raw = decode_esc(data, pos)?;
            Ok(Some(TupleValue::Bytes(raw)))
        }

        // String (UTF-8)
        STRING => {
            let raw = decode_esc(data, pos)?;
            let s = String::from_utf8(raw)?;
            Ok(Some(TupleValue::String(s)))
        }

        // Nested tuple / array
        NESTED => {
            let mut elements = Vec::new();
            loop {
                ensure!(*pos < data.len(), "unterminated nested tuple");
                match decode_one(data, pos, true)? {
                    Some(v) => elements.push(v),
                    None => break,
                }
            }
            Ok(Some(TupleValue::Tuple(elements)))
        }

        // Booleans
        FALSE => Ok(Some(TupleValue::Bool(false))),
        TRUE => Ok(Some(TupleValue::Bool(true))),

        // Zero
        ZERO => Ok(Some(TupleValue::Int(0))),

        // Positive integer (1–8 bytes, big-endian)
        t if (POS_INT_1..=POS_INT_8).contains(&t) => {
            let n = (t - POS_INT_1 + 1) as usize;
            ensure!(*pos + n <= data.len(), "truncated positive integer");
            let mut be = [0u8; 8];
            be[8 - n..].copy_from_slice(&data[*pos..*pos + n]);
            *pos += n;
            let v = u64::from_be_bytes(be);
            ensure!(v <= i64::MAX as u64, "positive integer exceeds i64::MAX");
            Ok(Some(TupleValue::Int(v as i64)))
        }

        // Negative integer (1–8 bytes, ones-complement big-endian)
        t if (NEG_INT_8..=0x13).contains(&t) => {
            let n = (ZERO - t) as usize;
            ensure!(*pos + n <= data.len(), "truncated negative integer");
            let mut be = [0u8; 8];
            for i in 0..n {
                be[8 - n + i] = !data[*pos + i]; // undo ones-complement
            }
            *pos += n;
            let abs = u64::from_be_bytes(be);
            // Negate: handles i64::MIN correctly via wrapping
            // (abs=2^63 → 0u64.wrapping_sub(2^63) = 2^63 as u64 → as i64 = i64::MIN)
            let value = 0u64.wrapping_sub(abs) as i64;
            Ok(Some(TupleValue::Int(value)))
        }

        // Float64 (sort-adjusted IEEE 754)
        FLOAT64 => {
            ensure!(*pos + 8 <= data.len(), "truncated float64");
            let mut b = [0u8; 8];
            b.copy_from_slice(&data[*pos..*pos + 8]);
            *pos += 8;
            // Reverse the sort transformation
            if b[0] & 0x80 != 0 {
                // Was positive: flip sign bit back
                b[0] ^= 0x80;
            } else {
                // Was negative: flip all bits back
                for x in &mut b {
                    *x = !*x;
                }
            }
            Ok(Some(TupleValue::Float(f64::from_be_bytes(b))))
        }

        _ => bail!("unknown tuple type tag: 0x{tag:02X}"),
    }
}

/// Decode escaped bytes until bare `0x00` terminator.
fn decode_esc(data: &[u8], pos: &mut usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        ensure!(*pos < data.len(), "unterminated escaped sequence");
        let b = data[*pos];
        *pos += 1;
        if b == NULL {
            // 0x00 0xFF = escaped null byte; bare 0x00 = terminator
            if *pos < data.len() && data[*pos] == ESC {
                out.push(NULL);
                *pos += 1;
            } else {
                break;
            }
        } else {
            out.push(b);
        }
    }
    Ok(out)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // --- Roundtrip tests ---

    #[test]
    fn test_roundtrip_null() {
        let key = TupleEncoder::new().null().build();
        assert_eq!(key, vec![NULL]);
        let vals = decode(&key).unwrap();
        assert_eq!(vals, vec![TupleValue::Null]);
    }

    #[test]
    fn test_roundtrip_bool() {
        let key = TupleEncoder::new().bool(false).bool(true).build();
        let vals = decode(&key).unwrap();
        assert_eq!(vals, vec![TupleValue::Bool(false), TupleValue::Bool(true)]);
    }

    #[test]
    fn test_roundtrip_int_zero() {
        let key = TupleEncoder::new().int(0).build();
        assert_eq!(key, vec![ZERO]);
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_int(), Some(0));
    }

    #[test]
    fn test_roundtrip_int_positive() {
        for v in [1i64, 127, 128, 255, 256, 65535, 65536, 1_000_000, i64::MAX] {
            let key = TupleEncoder::new().int(v).build();
            let vals = decode(&key).unwrap();
            assert_eq!(vals[0].as_int(), Some(v), "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_roundtrip_int_negative() {
        for v in [
            -1i64,
            -127,
            -128,
            -255,
            -256,
            -65535,
            -65536,
            -1_000_000,
            i64::MIN,
        ] {
            let key = TupleEncoder::new().int(v).build();
            let vals = decode(&key).unwrap();
            assert_eq!(vals[0].as_int(), Some(v), "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_roundtrip_string() {
        for s in ["", "hello", "README.md", "a longer string with spaces"] {
            let key = TupleEncoder::new().string(s).build();
            let vals = decode(&key).unwrap();
            assert_eq!(vals[0].as_str(), Some(s), "roundtrip failed for {s:?}");
        }
    }

    #[test]
    fn test_roundtrip_string_with_null() {
        let s = "he\0lo";
        let key = TupleEncoder::new().string(s).build();
        // Verify the null is escaped: STRING, h, e, 0x00, 0xFF, l, o, 0x00
        assert_eq!(key, vec![STRING, b'h', b'e', 0x00, ESC, b'l', b'o', 0x00]);
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_str(), Some(s));
    }

    #[test]
    fn test_roundtrip_string_only_null() {
        let s = "\0";
        let key = TupleEncoder::new().string(s).build();
        assert_eq!(key, vec![STRING, 0x00, ESC, 0x00]);
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_str(), Some(s));
    }

    #[test]
    fn test_roundtrip_string_unicode() {
        let s = "héllo 世界 🦀";
        let key = TupleEncoder::new().string(s).build();
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_str(), Some(s));
    }

    #[test]
    fn test_roundtrip_bytes() {
        let b: &[u8] = &[0x00, 0x01, 0xFF, 0x00, 0x42];
        let key = TupleEncoder::new().bytes(b).build();
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_bytes(), Some(b));
    }

    #[test]
    fn test_roundtrip_bytes_empty() {
        let key = TupleEncoder::new().bytes(&[]).build();
        assert_eq!(key, vec![BYTES, 0x00]);
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_bytes(), Some(&[] as &[u8]));
    }

    #[test]
    fn test_roundtrip_float_positive() {
        for v in [0.0f64, 1.0, 3.14, 1e100, f64::INFINITY] {
            let mut enc = TupleEncoder::new();
            enc.float(v).unwrap();
            let key = enc.build();
            let vals = decode(&key).unwrap();
            assert_eq!(vals[0].as_float(), Some(v), "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_roundtrip_float_negative() {
        for v in [-1.0f64, -3.14, -1e100, f64::NEG_INFINITY] {
            let mut enc = TupleEncoder::new();
            enc.float(v).unwrap();
            let key = enc.build();
            let vals = decode(&key).unwrap();
            assert_eq!(vals[0].as_float(), Some(v), "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_float_neg_zero_canonical() {
        let mut enc = TupleEncoder::new();
        enc.float(-0.0).unwrap();
        let key_neg = enc.build();

        let mut enc = TupleEncoder::new();
        enc.float(0.0).unwrap();
        let key_pos = enc.build();

        // -0.0 and +0.0 produce identical encoded bytes
        assert_eq!(key_neg, key_pos);
    }

    #[test]
    fn test_float_nan_rejected() {
        let mut enc = TupleEncoder::new();
        let result = enc.float(f64::NAN);
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip_tuple_empty() {
        let key = TupleEncoder::new().array(|_| {}).build();
        assert_eq!(key, vec![NESTED, NULL]);
        let vals = decode(&key).unwrap();
        assert_eq!(vals[0].as_tuple(), Some(&[] as &[TupleValue]));
    }

    #[test]
    fn test_roundtrip_tuple_with_ints() {
        let key = TupleEncoder::new()
            .array(|t| {
                t.int(0).int(1).int(2).int(3);
            })
            .build();
        let vals = decode(&key).unwrap();
        let elems = vals[0].as_tuple().unwrap();
        assert_eq!(elems.len(), 4);
        assert_eq!(elems[0].as_int(), Some(0));
        assert_eq!(elems[1].as_int(), Some(1));
        assert_eq!(elems[2].as_int(), Some(2));
        assert_eq!(elems[3].as_int(), Some(3));
    }

    #[test]
    fn test_roundtrip_tuple_with_null() {
        // [null, 1] — null inside nested tuple uses 0x00 0xFF escape
        let key = TupleEncoder::new()
            .array(|t| {
                t.null().int(1);
            })
            .build();
        assert_eq!(key, vec![NESTED, NULL, ESC, POS_INT_1, 0x01, NULL]);
        let vals = decode(&key).unwrap();
        let elems = vals[0].as_tuple().unwrap();
        assert_eq!(elems.len(), 2);
        assert!(elems[0].is_null());
        assert_eq!(elems[1].as_int(), Some(1));
    }

    #[test]
    fn test_roundtrip_tuple_with_empty_string() {
        let key = TupleEncoder::new()
            .array(|t| {
                t.string("");
            })
            .build();
        let vals = decode(&key).unwrap();
        let elems = vals[0].as_tuple().unwrap();
        assert_eq!(elems.len(), 1);
        assert_eq!(elems[0].as_str(), Some(""));
    }

    #[test]
    fn test_roundtrip_tuple_nested() {
        // [[1, 2], [3, 4]]
        let key = TupleEncoder::new()
            .array(|t| {
                t.array(|t2| {
                    t2.int(1).int(2);
                })
                .array(|t2| {
                    t2.int(3).int(4);
                });
            })
            .build();
        let vals = decode(&key).unwrap();
        let outer = vals[0].as_tuple().unwrap();
        assert_eq!(outer.len(), 2);
        let inner0 = outer[0].as_tuple().unwrap();
        assert_eq!(inner0[0].as_int(), Some(1));
        assert_eq!(inner0[1].as_int(), Some(2));
        let inner1 = outer[1].as_tuple().unwrap();
        assert_eq!(inner1[0].as_int(), Some(3));
        assert_eq!(inner1[1].as_int(), Some(4));
    }

    #[test]
    fn test_roundtrip_mixed() {
        // ("README.md", "version", 3, true, null)
        let key = TupleEncoder::new()
            .string("README.md")
            .string("version")
            .int(3)
            .bool(true)
            .null()
            .build();
        let vals = decode(&key).unwrap();
        assert_eq!(vals.len(), 5);
        assert_eq!(vals[0].as_str(), Some("README.md"));
        assert_eq!(vals[1].as_str(), Some("version"));
        assert_eq!(vals[2].as_int(), Some(3));
        assert_eq!(vals[3].as_bool(), Some(true));
        assert!(vals[4].is_null());
    }

    #[test]
    fn test_roundtrip_struct() {
        // {a: 1, b: [0, 1, 2, 3], c: null}
        let key = TupleEncoder::new()
            .int(1)
            .array(|t| {
                t.int(0).int(1).int(2).int(3);
            })
            .null()
            .build();
        let vals = decode(&key).unwrap();
        assert_eq!(vals.len(), 3);
        assert_eq!(vals[0].as_int(), Some(1));
        let arr = vals[1].as_tuple().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0].as_int(), Some(0));
        assert_eq!(arr[3].as_int(), Some(3));
        assert!(vals[2].is_null());
    }

    // --- Sort order tests ---

    #[test]
    fn test_sort_positive_ints() {
        let values: Vec<i64> = vec![0, 1, 127, 128, 255, 256, 1000, 65535, 65536, i64::MAX];
        let mut encoded: Vec<Vec<u8>> = values
            .iter()
            .map(|&v| TupleEncoder::new().int(v).build())
            .collect();
        let original = encoded.clone();
        encoded.sort();
        assert_eq!(encoded, original, "positive int sort order mismatch");
    }

    #[test]
    fn test_sort_negative_ints() {
        let values: Vec<i64> = vec![
            i64::MIN,
            -1_000_000,
            -65536,
            -65535,
            -256,
            -255,
            -128,
            -127,
            -1,
        ];
        let mut encoded: Vec<Vec<u8>> = values
            .iter()
            .map(|&v| TupleEncoder::new().int(v).build())
            .collect();
        let original = encoded.clone();
        encoded.sort();
        assert_eq!(encoded, original, "negative int sort order mismatch");
    }

    #[test]
    fn test_sort_ints_across_zero() {
        let values: Vec<i64> = vec![i64::MIN, -1000, -1, 0, 1, 1000, i64::MAX];
        let mut encoded: Vec<Vec<u8>> = values
            .iter()
            .map(|&v| TupleEncoder::new().int(v).build())
            .collect();
        let original = encoded.clone();
        encoded.sort();
        assert_eq!(encoded, original, "int sort across zero mismatch");
    }

    #[test]
    fn test_sort_strings() {
        let values = vec!["", "a", "aa", "ab", "b", "hello", "world"];
        let mut encoded: Vec<Vec<u8>> = values
            .iter()
            .map(|s| TupleEncoder::new().string(s).build())
            .collect();
        let original = encoded.clone();
        encoded.sort();
        assert_eq!(encoded, original, "string sort order mismatch");
    }

    #[test]
    fn test_sort_floats() {
        let values: Vec<f64> = vec![
            f64::NEG_INFINITY,
            -1e100,
            -1.0,
            0.0,
            1.0,
            1e100,
            f64::INFINITY,
        ];
        let mut encoded: Vec<Vec<u8>> = values
            .iter()
            .map(|&v| {
                let mut enc = TupleEncoder::new();
                enc.float(v).unwrap();
                enc.build()
            })
            .collect();
        let original = encoded.clone();
        encoded.sort();
        assert_eq!(encoded, original, "float sort order mismatch");
    }

    #[test]
    fn test_sort_cross_type() {
        // null < bytes < string < tuple < false < true < neg int < zero < pos int < float
        let null = TupleEncoder::new().null().build();
        let bytes = TupleEncoder::new().bytes(b"a").build();
        let string = TupleEncoder::new().string("a").build();
        let tuple = TupleEncoder::new()
            .array(|t| {
                t.int(1);
            })
            .build();
        let f = TupleEncoder::new().bool(false).build();
        let t = TupleEncoder::new().bool(true).build();
        let neg = TupleEncoder::new().int(-1).build();
        let zero = TupleEncoder::new().int(0).build();
        let pos = TupleEncoder::new().int(1).build();
        let float = {
            let mut enc = TupleEncoder::new();
            enc.float(1.0).unwrap();
            enc.build()
        };

        let mut encoded = vec![
            &float, &pos, &zero, &neg, &t, &f, &tuple, &string, &bytes, &null,
        ];
        encoded.sort();
        assert_eq!(
            encoded,
            vec![&null, &bytes, &string, &tuple, &f, &t, &neg, &zero, &pos, &float],
            "cross-type sort order mismatch"
        );
    }

    // --- Prefix query tests ---

    #[test]
    fn test_prefix_string_field_boundary() {
        let full = TupleEncoder::new()
            .string("README.md")
            .string("label")
            .build();
        let prefix = TupleEncoder::new().string("README.md").build();
        assert!(full.starts_with(&prefix));
    }

    #[test]
    fn test_prefix_string_mid_field() {
        // "readme" starts with "read" at the byte level
        let full = TupleEncoder::new().string("readme").build();
        let prefix = TupleEncoder::new().string_prefix("read").build();
        assert!(full.starts_with(&prefix));

        // But full "read" (with terminator) does NOT match "readme" bytes
        let exact = TupleEncoder::new().string("read").build();
        assert!(!full.starts_with(&exact));
    }

    #[test]
    fn test_prefix_string_no_false_match() {
        // "other" should NOT match prefix "read"
        let full = TupleEncoder::new().string("other").build();
        let prefix = TupleEncoder::new().string_prefix("read").build();
        assert!(!full.starts_with(&prefix));
    }

    #[test]
    fn test_prefix_array_elements() {
        // [0, 1, 2, 3] starts with [0, 1]
        let full = TupleEncoder::new()
            .array(|t| {
                t.int(0).int(1).int(2).int(3);
            })
            .build();
        let prefix = TupleEncoder::new()
            .array_prefix(|t| {
                t.int(0).int(1);
            })
            .build();
        assert!(full.starts_with(&prefix));

        // But [0, 9] does NOT start with [0, 1]
        let other = TupleEncoder::new()
            .array(|t| {
                t.int(0).int(9);
            })
            .build();
        assert!(!other.starts_with(&prefix));
    }

    #[test]
    fn test_prefix_multi_field() {
        // Match entries where field0="README.md" AND field1="label"
        let full = TupleEncoder::new()
            .string("README.md")
            .string("label")
            .string("rust")
            .build();
        let prefix = TupleEncoder::new()
            .string("README.md")
            .string("label")
            .build();
        assert!(full.starts_with(&prefix));

        // Different field1 does not match
        let other = TupleEncoder::new()
            .string("README.md")
            .string("version")
            .int(3)
            .build();
        assert!(!other.starts_with(&prefix));
    }

    #[test]
    fn test_prefix_struct_field() {
        // {a: 1, b: [0, 1, 2, 3], c: null}
        let full = TupleEncoder::new()
            .int(1)
            .array(|t| {
                t.int(0).int(1).int(2).int(3);
            })
            .null()
            .build();

        // Prefix: a=1
        let p1 = TupleEncoder::new().int(1).build();
        assert!(full.starts_with(&p1));

        // Prefix: a=1, b starts with [0, 1]
        let p2 = TupleEncoder::new()
            .int(1)
            .array_prefix(|t| {
                t.int(0).int(1);
            })
            .build();
        assert!(full.starts_with(&p2));

        // Prefix: a=2 does NOT match
        let p3 = TupleEncoder::new().int(2).build();
        assert!(!full.starts_with(&p3));
    }

    // --- Integer encoding details ---

    #[test]
    fn test_int_encoding_sizes() {
        // 1 byte: 1–255
        let k = TupleEncoder::new().int(1).build();
        assert_eq!(k, vec![POS_INT_1, 0x01]);

        let k = TupleEncoder::new().int(255).build();
        assert_eq!(k, vec![POS_INT_1, 0xFF]);

        // 2 bytes: 256–65535
        let k = TupleEncoder::new().int(256).build();
        assert_eq!(k, vec![POS_INT_1 + 1, 0x01, 0x00]);

        // Negative 1 byte
        let k = TupleEncoder::new().int(-1).build();
        assert_eq!(k, vec![0x13, 0xFE]); // ones-complement of 0x01

        let k = TupleEncoder::new().int(-255).build();
        assert_eq!(k, vec![0x13, 0x00]); // ones-complement of 0xFF

        // Negative 2 bytes
        let k = TupleEncoder::new().int(-256).build();
        assert_eq!(k, vec![0x12, 0xFE, 0xFF]); // ones-complement of [0x01, 0x00]
    }

    // --- Error handling ---

    #[test]
    fn test_decode_empty() {
        let vals = decode(&[]).unwrap();
        assert!(vals.is_empty());
    }

    #[test]
    fn test_decode_unknown_tag() {
        let result = decode(&[0xFE]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_truncated_int() {
        // Tag says 2-byte positive int, but only 1 byte follows
        let result = decode(&[POS_INT_1 + 1, 0x01]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_truncated_float() {
        // Tag says float64, but only 4 bytes follow
        let result = decode(&[FLOAT64, 0x00, 0x00, 0x00, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_unterminated_string() {
        // String tag followed by data but no null terminator
        let result = decode(&[STRING, b'h', b'i']);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_unterminated_tuple() {
        // Nested tuple start but no end marker
        let result = decode(&[NESTED, POS_INT_1, 0x01]);
        assert!(result.is_err());
    }

    // --- Accessor tests ---

    #[test]
    fn test_accessors_return_none_for_wrong_type() {
        let null = TupleValue::Null;
        assert!(null.is_null());
        assert!(null.as_str().is_none());
        assert!(null.as_int().is_none());
        assert!(null.as_float().is_none());
        assert!(null.as_bool().is_none());
        assert!(null.as_bytes().is_none());
        assert!(null.as_tuple().is_none());

        let s = TupleValue::String("hi".to_owned());
        assert!(!s.is_null());
        assert_eq!(s.as_str(), Some("hi"));
        assert!(s.as_int().is_none());
    }
}
