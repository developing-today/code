//! Shared tuple checker for the property tests and the regression replay.
//!
//! A tuple element is either a byte string (`bytes` tag) or a UTF-8 string
//! (`string` tag). [`check_pair`] holds the properties the encoding must keep
//! for any two elements: decoding gives back the original values, encoded order
//! follows value order, and a complete field is a prefix of another encoding
//! only when the values are equal.

#![allow(dead_code)] // each test crate uses a different part of this module

use std::cmp::Ordering;

use id::tuple::{TupleEncoder, TupleValue, decode};

/// One tuple element, as the property tests generate it and the regression
/// file spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A byte string, encoded with the `bytes` tag.
    Bytes(Vec<u8>),
    /// A UTF-8 string, encoded with the `string` tag.
    Text(String),
}

impl Value {
    /// Encode this element as a complete field, with its terminator.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Bytes(bytes) => TupleEncoder::new().bytes(bytes).build(),
            Self::Text(text) => TupleEncoder::new().string(text).build(),
        }
    }

    /// The element as `decode` returns it.
    #[must_use]
    pub fn as_tuple_value(&self) -> TupleValue {
        match self {
            Self::Bytes(bytes) => TupleValue::Bytes(bytes.clone()),
            Self::Text(text) => TupleValue::String(text.clone()),
        }
    }

    /// The type tag's place in the sort: bytes (`0x01`) before strings (`0x02`).
    const fn rank(&self) -> u8 {
        match self {
            Self::Bytes(_) => 0,
            Self::Text(_) => 1,
        }
    }
}

/// The order the encoding must reproduce: by type tag, then by content.
#[must_use]
pub fn value_order(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        _ => left.rank().cmp(&right.rank()),
    }
}

/// The order the encoding must reproduce for tuples: element by element, and a
/// shorter tuple that is a leading run of a longer one sorts first.
#[must_use]
pub fn tuple_order(left: &[Value], right: &[Value]) -> Ordering {
    for (a, b) in left.iter().zip(right) {
        match value_order(a, b) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    left.len().cmp(&right.len())
}

/// Encode a tuple of elements into one key.
#[must_use]
pub fn encode_tuple(values: &[Value]) -> Vec<u8> {
    values.iter().flat_map(Value::encode).collect()
}

/// `decode(encode(values)) == values`, element by element.
///
/// # Errors
///
/// Describes the decode failure or the first element that came back changed.
pub fn check_roundtrip(values: &[Value]) -> Result<(), String> {
    let decoded =
        decode(&encode_tuple(values)).map_err(|e| format!("decode of {values:?} failed: {e:#}"))?;
    let want: Vec<TupleValue> = values.iter().map(Value::as_tuple_value).collect();
    if decoded != want {
        return Err(format!("roundtrip of {values:?} gave {decoded:?}"));
    }
    Ok(())
}

/// Check the tuple properties for a pair of elements, using the real encoder.
///
/// # Errors
///
/// Describes the first property that does not hold.
pub fn check_pair(left: &Value, right: &Value) -> Result<(), String> {
    check_roundtrip(std::slice::from_ref(left))?;
    check_roundtrip(std::slice::from_ref(right))?;
    check_roundtrip(&[left.clone(), right.clone()])?;
    check_order_and_prefix(Value::encode, left, right)
}

/// Check encoded order against value order, and the prefix rule, for a pair of
/// elements under the given encoder. Takes the encoder as a parameter so a test
/// can run the same checks against a known-bad encoding.
///
/// # Errors
///
/// Describes the first property that does not hold.
pub fn check_order_and_prefix(
    encode: impl Fn(&Value) -> Vec<u8>,
    left: &Value,
    right: &Value,
) -> Result<(), String> {
    let enc_left = encode(left);
    let enc_right = encode(right);

    // Encoded order equals value order, for one element and for a pair.
    let want = value_order(left, right);
    let got = enc_left.cmp(&enc_right);
    if got != want {
        return Err(format!(
            "order of {left:?} and {right:?}: values say {want:?}, encodings say {got:?}"
        ));
    }
    let pair = [left.clone(), right.clone()];
    let single = [right.clone()];
    let want = tuple_order(&pair, &single);
    let got = [enc_left.as_slice(), enc_right.as_slice()]
        .concat()
        .cmp(&enc_right);
    if got != want {
        return Err(format!(
            "tuple order of {pair:?} and {single:?}: values say {want:?}, encodings say {got:?}"
        ));
    }

    // A complete field is a prefix of another encoding only when the values
    // are equal. The second haystack has another field after the first, so
    // the check also covers a field inside a longer key.
    let equal = left == right;
    let longer = [enc_right.as_slice(), enc_left.as_slice()].concat();
    for (label, haystack) in [("alone", &enc_right), ("followed by a field", &longer)] {
        let is_prefix = haystack.starts_with(&enc_left);
        if is_prefix != equal {
            return Err(format!(
                "{left:?} is {}a prefix of {right:?} ({label}), but the values are {}",
                if is_prefix { "" } else { "not " },
                if equal { "equal" } else { "different" },
            ));
        }
    }
    Ok(())
}

/// Parse one element written as `s:<escaped>` (string) or `b:<escaped>` (bytes).
///
/// # Errors
///
/// Fails on an unknown prefix, a bad escape, or a string that is not UTF-8.
pub fn parse_value(text: &str) -> Result<Value, String> {
    if let Some(rest) = text.strip_prefix("s:") {
        let bytes = unescape(rest)?;
        let string =
            String::from_utf8(bytes).map_err(|e| format!("string {text:?} is not UTF-8: {e}"))?;
        Ok(Value::Text(string))
    } else if let Some(rest) = text.strip_prefix("b:") {
        Ok(Value::Bytes(unescape(rest)?))
    } else {
        Err(format!("value {text:?} must start with s: or b:"))
    }
}

/// Undo the escapes used in regression files: `\\`, `\0`, `\t`, `\n`, `\r`,
/// and `\xHH` (exactly two hex digits) for any byte. Other characters stand
/// for themselves; a space must be written `\x20`.
///
/// # Errors
///
/// Fails on a dangling or unknown escape, a bad hex escape, or a raw space or
/// control character.
pub fn unescape(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\\') => out.push(b'\\'),
                Some('0') => out.push(0),
                Some('t') => out.push(b'\t'),
                Some('n') => out.push(b'\n'),
                Some('r') => out.push(b'\r'),
                Some('x') => {
                    let hex: String = chars.by_ref().take(2).collect();
                    if hex.len() != 2 || !hex.chars().all(|d| d.is_ascii_hexdigit()) {
                        return Err(format!("bad \\x escape in {text:?}"));
                    }
                    out.push(u8::from_str_radix(&hex, 16).unwrap_or_default());
                }
                Some(other) => return Err(format!("unknown escape \\{other} in {text:?}")),
                None => return Err(format!("dangling backslash in {text:?}")),
            },
            ' ' => return Err(format!("raw space in {text:?}; write it as \\x20")),
            c if c.is_ascii_control() => {
                return Err(format!("raw control character in {text:?}"));
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The encoding before the terminator fix: a field ended at its first
    /// `0x00`, so `"a"` looked like a prefix of `"a\0b"`.
    fn single_nul_terminator(value: &Value) -> Vec<u8> {
        let (tag, data) = match value {
            Value::Bytes(bytes) => (1u8, bytes.clone()),
            Value::Text(text) => (2u8, text.as_bytes().to_vec()),
        };
        let mut out = vec![tag];
        out.extend(data);
        out.push(0);
        out
    }

    #[test]
    fn test_unescape_handles_every_escape() {
        assert_eq!(unescape(r"a\0b").unwrap(), b"a\0b");
        assert_eq!(unescape(r"\\\t\n\r").unwrap(), b"\\\t\n\r");
        assert_eq!(unescape(r"\xff\x20").unwrap(), [0xFF, b' ']);
        assert_eq!(unescape("é").unwrap(), "é".as_bytes());
    }

    #[test]
    fn test_unescape_rejects_bad_input() {
        assert!(unescape("a b").is_err());
        assert!(unescape(r"\q").is_err());
        assert!(unescape(r"\x4").is_err());
        assert!(unescape(r"\x+f").is_err());
        assert!(unescape(r"\xzz").is_err());
        assert!(unescape("a\tb").is_err());
        assert!(unescape("trailing\\").is_err());
    }

    #[test]
    fn test_parse_value_picks_the_type_by_prefix() {
        assert_eq!(parse_value("s:a\\0b").unwrap(), Value::Text("a\0b".into()));
        assert_eq!(parse_value("b:\\xff").unwrap(), Value::Bytes(vec![0xFF]));
        assert_eq!(parse_value("s:").unwrap(), Value::Text(String::new()));
        assert!(parse_value("x:a").is_err());
        assert!(parse_value("s:\\xff").is_err(), "0xFF alone is not UTF-8");
    }

    #[test]
    fn test_check_pair_accepts_the_known_nul_cases() {
        let a = Value::Text("a".into());
        let a_nul_b = Value::Text("a\0b".into());
        assert_eq!(check_pair(&a, &a_nul_b), Ok(()));
        assert_eq!(check_pair(&a_nul_b, &a), Ok(()));
        assert_eq!(check_pair(&a, &a), Ok(()));
    }

    #[test]
    fn test_the_checker_would_have_caught_the_single_nul_terminator() {
        let a = Value::Text("a".into());
        let a_nul_b = Value::Text("a\0b".into());
        assert!(check_order_and_prefix(single_nul_terminator, &a, &a_nul_b).is_err());
        let a = Value::Bytes(b"a".to_vec());
        let a_nul_b = Value::Bytes(b"a\0b".to_vec());
        assert!(check_order_and_prefix(single_nul_terminator, &a, &a_nul_b).is_err());
    }
}
