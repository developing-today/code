//! Canonical CBOR profile for signed ID/Atlas records
//! (slice A9114-cbor-profile, #10181; design A-9114.3).
//!
//! New signed records are encoded in this deterministic profile
//! so a body signature or envelope ID is byte-stable across Rust
//! and the hosted TypeScript worker. The profile is deliberately
//! narrower than CBOR in general:
//!
//! - Integers and length arguments use the shortest form.
//! - Strings, arrays and maps use definite lengths only.
//! - Map keys are unique and sorted by their encoded bytes.
//! - The value model is closed: null, booleans, integers
//!   (major 0/1), byte strings, UTF-8 text strings, arrays and
//!   maps. Floats, tags, and every other major/additional form
//!   are rejected — there is no ambiguous signed encoding.
//! - Decoding is strict: anything the encoder would not produce
//!   (non-shortest forms, indefinite lengths, duplicate or
//!   out-of-order keys, trailing bytes) is a typed error.
//!
//! Record-level rule (design A-9114.1/A-9114.3): unknown
//! *critical* fields reject. [`validate_record`] checks a decoded
//! map against a field spec: a key listed as critical that the
//! reader does not know is [`CborError::UnknownCriticalField`];
//! unknown non-critical keys are tolerated for forward
//! compatibility.

use std::fmt;

/// A value in the canonical profile's closed model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// CBOR null.
    Null,
    /// CBOR true/false.
    Bool(bool),
    /// Non-negative integer (major type 0).
    UInt(u64),
    /// Negative integer (major type 1), value in `i64` range.
    Int(i64),
    /// Byte string (major type 2).
    Bytes(Vec<u8>),
    /// Text string (major type 3), always valid UTF-8.
    Text(String),
    /// Array (major type 4).
    Array(Vec<Value>),
    /// Map (major type 5); keys are text or integer values,
    /// stored in canonical (encoded-byte sorted) order.
    Map(Vec<(Value, Value)>),
}

/// Typed profile errors. Every rejection of a signed encoding is
/// one of these — never a panic, never a silent coercion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CborError {
    /// Input ended before a complete item.
    UnexpectedEof,
    /// An integer or length used a longer form than necessary.
    NonShortestForm,
    /// An indefinite-length item (forbidden by the profile).
    IndefiniteLength,
    /// A major/additional form outside the closed value model
    /// (floats, tags, simple values other than bool/null, ...).
    UnsupportedType {
        /// The offending initial byte.
        byte: u8,
    },
    /// A negative integer below the `i64` range of the model.
    Overflow,
    /// A text string that is not valid UTF-8.
    InvalidUtf8,
    /// A map key that is not a text or integer value.
    InvalidMapKey,
    /// The same map key appeared twice.
    DuplicateKey,
    /// Map keys were not in canonical sorted order.
    KeysOutOfOrder,
    /// Bytes remained after the first complete item.
    TrailingBytes,
    /// A record carried a critical field the reader does not know.
    UnknownCriticalField {
        /// The unknown critical field name.
        field: String,
    },
    /// Nesting exceeded `MAX_NESTING_DEPTH`.
    TooDeeplyNested,
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of input"),
            Self::NonShortestForm => write!(f, "integer or length not in shortest form"),
            Self::IndefiniteLength => write!(f, "indefinite-length items are not canonical"),
            Self::UnsupportedType { byte } => {
                write!(f, "unsupported CBOR form (initial byte {byte:#04x})")
            }
            Self::Overflow => write!(f, "integer outside the profile's range"),
            Self::InvalidUtf8 => write!(f, "text string is not valid UTF-8"),
            Self::InvalidMapKey => write!(f, "map keys must be text or integer values"),
            Self::DuplicateKey => write!(f, "duplicate map key"),
            Self::KeysOutOfOrder => write!(f, "map keys are not in canonical order"),
            Self::TrailingBytes => write!(f, "trailing bytes after the first item"),
            Self::UnknownCriticalField { field } => {
                write!(f, "unknown critical field: {field}")
            }
            Self::TooDeeplyNested => write!(f, "nesting exceeds the profile's depth limit"),
        }
    }
}

impl std::error::Error for CborError {}

fn encode_argument(major: u8, value: u64, out: &mut Vec<u8>) {
    let major = major << 5;
    if value < 24 {
        out.push(major | value as u8);
    } else if value <= u8::MAX as u64 {
        out.push(major | 24);
        out.push(value as u8);
    } else if value <= u16::MAX as u64 {
        out.push(major | 25);
        out.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= u32::MAX as u64 {
        out.push(major | 26);
        out.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        out.push(major | 27);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn encode_into(value: &Value, out: &mut Vec<u8>) -> Result<(), CborError> {
    match value {
        Value::Null => out.push(0xf6),
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::UInt(n) => encode_argument(0, *n, out),
        Value::Int(n) => {
            // Major type 1 encodes -1 - n.
            encode_argument(1, (-1i128 - *n as i128) as u64, out);
        }
        Value::Bytes(bytes) => {
            encode_argument(2, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        Value::Text(text) => {
            encode_argument(3, text.len() as u64, out);
            out.extend_from_slice(text.as_bytes());
        }
        Value::Array(items) => {
            encode_argument(4, items.len() as u64, out);
            for item in items {
                encode_into(item, out)?;
            }
        }
        Value::Map(entries) => {
            // Encode keys first so ordering is by encoded bytes,
            // exactly as the decoder verifies.
            let mut encoded: Vec<(Vec<u8>, &Value)> = Vec::with_capacity(entries.len());
            for (key, val) in entries {
                if !matches!(key, Value::Text(_) | Value::UInt(_) | Value::Int(_)) {
                    return Err(CborError::InvalidMapKey);
                }
                let mut key_bytes = Vec::new();
                encode_into(key, &mut key_bytes)?;
                if encoded.iter().any(|(existing, _)| *existing == key_bytes) {
                    return Err(CborError::DuplicateKey);
                }
                encoded.push((key_bytes, val));
            }
            encoded.sort_by(|a, b| a.0.cmp(&b.0));
            encode_argument(5, encoded.len() as u64, out);
            for (key_bytes, val) in encoded {
                out.extend_from_slice(&key_bytes);
                encode_into(val, out)?;
            }
        }
    }
    Ok(())
}

/// Encode a value in the canonical profile.
pub fn encode_canonical(value: &Value) -> Result<Vec<u8>, CborError> {
    let mut out = Vec::new();
    encode_into(value, &mut out)?;
    Ok(out)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

/// Maximum nesting depth for decoded values. Each open array or
/// map consumes at least one input byte, so without this bound a
/// few KB of `0x81` bytes would overflow the thread stack via
/// `value_inner` recursion; with it, hostile nesting is a typed
/// error instead of a crash.
pub const MAX_NESTING_DEPTH: u32 = 64;

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, CborError> {
        let byte = *self.bytes.get(self.pos).ok_or(CborError::UnexpectedEof)?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CborError> {
        let end = self.pos.checked_add(len).ok_or(CborError::UnexpectedEof)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(CborError::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    fn argument(&mut self, additional: u8) -> Result<u64, CborError> {
        match additional {
            0..=23 => Ok(additional as u64),
            24 => {
                let value = self.byte()? as u64;
                if value < 24 {
                    return Err(CborError::NonShortestForm);
                }
                Ok(value)
            }
            25 => {
                let value = u16::from_be_bytes(
                    self.take(2)?
                        .try_into()
                        .map_err(|_| CborError::UnexpectedEof)?,
                ) as u64;
                if value <= u8::MAX as u64 {
                    return Err(CborError::NonShortestForm);
                }
                Ok(value)
            }
            26 => {
                let value = u32::from_be_bytes(
                    self.take(4)?
                        .try_into()
                        .map_err(|_| CborError::UnexpectedEof)?,
                ) as u64;
                if value <= u16::MAX as u64 {
                    return Err(CborError::NonShortestForm);
                }
                Ok(value)
            }
            27 => {
                let value = u64::from_be_bytes(
                    self.take(8)?
                        .try_into()
                        .map_err(|_| CborError::UnexpectedEof)?,
                );
                if value <= u32::MAX as u64 {
                    return Err(CborError::NonShortestForm);
                }
                Ok(value)
            }
            31 => Err(CborError::IndefiniteLength),
            _ => Err(CborError::UnsupportedType { byte: additional }),
        }
    }

    fn value(&mut self) -> Result<Value, CborError> {
        self.value_inner(0)
    }

    fn value_inner(&mut self, depth: u32) -> Result<Value, CborError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(CborError::TooDeeplyNested);
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;
        match major {
            0 => Ok(Value::UInt(self.argument(additional)?)),
            1 => {
                let n = self.argument(additional)?;
                if n > i64::MAX as u64 {
                    return Err(CborError::Overflow);
                }
                Ok(Value::Int(-1 - n as i64))
            }
            2 => {
                let len = usize::try_from(self.argument(additional)?)
                    .map_err(|_| CborError::UnexpectedEof)?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = usize::try_from(self.argument(additional)?)
                    .map_err(|_| CborError::UnexpectedEof)?;
                let bytes = self.take(len)?;
                let text = std::str::from_utf8(bytes).map_err(|_| CborError::InvalidUtf8)?;
                Ok(Value::Text(text.to_string()))
            }
            4 => {
                let len = self.argument(additional)?;
                // Every array item consumes at least one input
                // byte, so a declared count larger than the
                // remaining input is truncated, not huge: reject
                // before `with_capacity` can over-allocate.
                if len > (self.bytes.len() - self.pos) as u64 {
                    return Err(CborError::UnexpectedEof);
                }
                let mut items = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    items.push(self.value_inner(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let len = self.argument(additional)?;
                // As above: each entry needs at least a key byte
                // and a value byte, so at most one entry per two
                // remaining bytes — bound the preallocation by that.
                if len > ((self.bytes.len() - self.pos) / 2) as u64 {
                    return Err(CborError::UnexpectedEof);
                }
                let mut entries: Vec<(Value, Value)> = Vec::with_capacity(len as usize);
                let mut previous_key_bytes: Option<Vec<u8>> = None;
                for _ in 0..len {
                    let key_start = self.pos;
                    let key = self.value_inner(depth + 1)?;
                    if !matches!(key, Value::Text(_) | Value::UInt(_) | Value::Int(_)) {
                        return Err(CborError::InvalidMapKey);
                    }
                    let key_bytes = self.bytes[key_start..self.pos].to_vec();
                    if let Some(previous) = &previous_key_bytes {
                        if key_bytes == *previous {
                            return Err(CborError::DuplicateKey);
                        }
                        if key_bytes < *previous {
                            return Err(CborError::KeysOutOfOrder);
                        }
                    }
                    previous_key_bytes = Some(key_bytes);
                    let val = self.value_inner(depth + 1)?;
                    entries.push((key, val));
                }
                Ok(Value::Map(entries))
            }
            7 => match additional {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                _ => Err(CborError::UnsupportedType { byte: initial }),
            },
            // Major type 6 (tags) and anything else is outside the
            // closed model.
            _ => Err(CborError::UnsupportedType { byte: initial }),
        }
    }
}

/// Decode exactly one canonical-profile value; trailing bytes,
/// non-canonical forms and out-of-model types all reject.
pub fn decode_canonical(bytes: &[u8]) -> Result<Value, CborError> {
    let mut reader = Reader { bytes, pos: 0 };
    let value = reader.value()?;
    if reader.pos != bytes.len() {
        return Err(CborError::TrailingBytes);
    }
    Ok(value)
}

/// The field spec a record reader validates against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordSpec {
    /// Fields this reader understands.
    pub known_fields: &'static [&'static str],
    /// Fields that are critical: a record carrying one the
    /// reader does not know must reject rather than proceed
    /// on a partial understanding.
    pub critical_fields: &'static [&'static str],
}

/// Validate a decoded record map against `spec`: every critical
/// field present in the record must be known to the reader.
/// Unknown non-critical fields are tolerated.
pub fn validate_record(record: &Value, spec: &RecordSpec) -> Result<(), CborError> {
    let Value::Map(entries) = record else {
        return Err(CborError::InvalidMapKey);
    };
    for (key, _) in entries {
        let Value::Text(field) = key else {
            continue;
        };
        if spec.critical_fields.contains(&field.as_str())
            && !spec.known_fields.contains(&field.as_str())
        {
            return Err(CborError::UnknownCriticalField {
                field: field.clone(),
            });
        }
    }
    Ok(())
}
