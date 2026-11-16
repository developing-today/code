//! Tagged typed values and the explicit codec boundary.
//!
//! Slice A9118-typed-values (issue #10203), design statement A-9118.2
//! (`part-a-addressing/versioned-design.md` section "#9118 —
//! Array-address core", S4.34–S4.40): a [`TypedValue`] is
//! `{ type_id, payload_bytes }`. The type ID is a stable registered
//! URI-like identifier or a numeric code, scoped by the serialization
//! profile; the tag participates in equality, so distinct types with
//! identical payload are unequal (integer 1 is not text "1").
//! Arbitrary object values are serialized by an explicit codec profile;
//! an unknown non-critical type is preserved opaque, an unknown
//! critical type rejects. There is no implicit stringification: typed
//! accessors check the tag and never coerce across types.
//!
//! Wire format (version [`FORMAT_VERSION`]), all integers big-endian:
//!
//! ```text
//! value   := version u8 | profile_len u64 | profile utf8
//!          | tag u8 | type_body | payload_len u64 | payload
//! tag     := 0x00 registered: name_len u64 | name utf8 (non-empty)
//!          | 0x01 code: code u64
//! ```
//!
//! The format is canonical: every admitted value has exactly one
//! encoding, so `decode(encode(x)) == x` for all admitted values and
//! `encode(decode(bytes)) == bytes` for canonical bytes. The decoder
//! rejects any other version, truncated input, unknown tag bytes,
//! empty names and trailing bytes.
//!
//! Built-in types under [`PROFILE_V1`], with their payload shapes:
//!
//! | type ID               | payload shape                              |
//! |-----------------------|----------------------------------------------|
//! | `id-value/null/v1`    | empty                                        |
//! | `id-value/bytes/v1`   | raw bytes                                    |
//! | `id-value/text/v1`    | UTF-8 bytes (validated, never lossy-coerced) |
//! | `id-value/integer/v1` | exactly 8 bytes, big-endian `i64`            |
//! | `id-value/tuple/v1`   | `count u64` then `count` nested elements,    |
//! |                       | each `tag u8 | type_body | len u64 | bytes`  |
//!
//! Tuples inherit the enclosing profile and recurse through the same
//! shape validation. Nesting deeper than [`MAX_TUPLE_DEPTH`] rejects
//! with [`ValueError::DepthLimit`] so untrusted input cannot exhaust
//! the stack.
//!
//! Shape validation applies to built-in names only under
//! [`PROFILE_V1`]: a type ID is scoped by its profile, so the same
//! name under another profile is an unknown type and is preserved
//! opaque. [`TypedValue::decode_strict`] additionally enforces the
//! profile boundary (wire profile must equal the codec profile) and
//! the criticality rule mirroring the canonical-CBOR record rule:
//! every unknown type that the codec profile lists as critical
//! rejects with [`ValueError::UnknownCriticalType`]; unknown
//! non-critical types decode to opaque values whose bytes round-trip
//! exactly.
//!
//! This module is intentionally dependency-free (pure `std`): the crate
//! root export and manifest wiring belong to slice 9110-02; until that
//! lands, the integration tests include this file by path.

use std::fmt;

/// Version byte of the typed-value wire format defined here.
pub const FORMAT_VERSION: u8 = 0x02;

/// Serialization profile scoping the built-in type IDs below.
pub const PROFILE_V1: &str = "id-value-codec/v1";

/// Built-in type: null. Payload is always empty.
pub const TYPE_NULL: &str = "id-value/null/v1";
/// Built-in type: opaque bytes. Payload is raw bytes.
pub const TYPE_BYTES: &str = "id-value/bytes/v1";
/// Built-in type: UTF-8 text. Payload is validated UTF-8.
pub const TYPE_TEXT: &str = "id-value/text/v1";
/// Built-in type: integer. Payload is exactly 8 bytes, big-endian `i64`.
pub const TYPE_INTEGER: &str = "id-value/integer/v1";
/// Built-in type: tuple of typed values. Payload is `count u64`
/// followed by that many nested typed elements.
pub const TYPE_TUPLE: &str = "id-value/tuple/v1";

const TAG_REGISTERED: u8 = 0x00;
const TAG_CODE: u8 = 0x01;

/// Maximum tuple nesting depth accepted by the decoder.
pub const MAX_TUPLE_DEPTH: usize = 64;

/// The tag half of a [`TypeId`]: a stable registered URI-like
/// identifier, or a numeric code scoped by the profile.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeName {
    /// Stable registered URI-like identifier (e.g. `id-value/text/v1`).
    Registered(String),
    /// Numeric code scoped by the enclosing profile.
    Code(u64),
}

/// A type tag: a [`TypeName`] scoped by its serialization profile.
/// Both halves participate in equality.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeId {
    /// Serialization profile scoping the name (e.g. [`PROFILE_V1`]).
    pub profile: String,
    /// The registered name or numeric code within the profile.
    pub name: TypeName,
}

impl TypeId {
    /// A registered name under [`PROFILE_V1`].
    pub fn registered(name: &str) -> Self {
        Self {
            profile: PROFILE_V1.to_string(),
            name: TypeName::Registered(name.to_string()),
        }
    }

    /// A numeric code under [`PROFILE_V1`].
    pub fn code(code: u64) -> Self {
        Self {
            profile: PROFILE_V1.to_string(),
            name: TypeName::Code(code),
        }
    }

    /// Whether this is one of the five built-in types under
    /// [`PROFILE_V1`].
    pub fn is_builtin(&self) -> bool {
        self.profile == PROFILE_V1
            && matches!(
                self.name,
                TypeName::Registered(ref name)
                    if name.as_str() == TYPE_NULL
                        || name.as_str() == TYPE_BYTES
                        || name.as_str() == TYPE_TEXT
                        || name.as_str() == TYPE_INTEGER
                        || name.as_str() == TYPE_TUPLE
            )
    }

    /// The display name of the tag: the registered name, or
    /// `code(n)` for a numeric code.
    pub fn name_string(&self) -> String {
        match &self.name {
            TypeName::Registered(name) => name.clone(),
            TypeName::Code(code) => format!("code({code})"),
        }
    }
}

/// A tagged value: a [`TypeId`] plus opaque payload bytes.
///
/// Equality is the full triple of profile, tag and payload: two
/// values with identical payload bytes but different tags are
/// unequal. There is no `Display` that renders a bare payload and no
/// conversion that coerces across types; use the typed accessors.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypedValue {
    type_id: TypeId,
    payload: Vec<u8>,
}

impl TypedValue {
    /// The null value: [`TYPE_NULL`] with an empty payload.
    pub fn null() -> Self {
        Self {
            type_id: TypeId::registered(TYPE_NULL),
            payload: Vec::new(),
        }
    }

    /// A bytes value: [`TYPE_BYTES`] carrying raw bytes.
    pub fn bytes(payload: Vec<u8>) -> Self {
        Self {
            type_id: TypeId::registered(TYPE_BYTES),
            payload,
        }
    }

    /// A text value: [`TYPE_TEXT`] carrying the UTF-8 bytes of `text`.
    pub fn text(text: &str) -> Self {
        Self {
            type_id: TypeId::registered(TYPE_TEXT),
            payload: text.as_bytes().to_vec(),
        }
    }

    /// An integer value: [`TYPE_INTEGER`] carrying the big-endian
    /// encoding of `value`.
    pub fn integer(value: i64) -> Self {
        Self {
            type_id: TypeId::registered(TYPE_INTEGER),
            payload: value.to_be_bytes().to_vec(),
        }
    }

    /// A tuple value: [`TYPE_TUPLE`] carrying `count u64` followed by
    /// the wire encoding of each element (tag, body, length, bytes),
    /// inheriting this value's profile.
    pub fn tuple(elements: Vec<TypedValue>) -> Self {
        let mut payload = (elements.len() as u64).to_be_bytes().to_vec();
        for element in &elements {
            element.encode_element(&mut payload);
        }
        Self {
            type_id: TypeId::registered(TYPE_TUPLE),
            payload,
        }
    }

    /// An explicitly tagged value with an unchecked payload. This is
    /// the boundary for foreign types: unknown non-critical types
    /// pass through here and are preserved byte-for-byte. No shape
    /// validation applies; the tag alone determines equality and the
    /// typed accessors still refuse to interpret the payload as a
    /// built-in type.
    pub fn opaque(type_id: TypeId, payload: Vec<u8>) -> Self {
        Self { type_id, payload }
    }

    /// The type tag of this value.
    pub fn type_id(&self) -> &TypeId {
        &self.type_id
    }

    /// The raw payload bytes of this value.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Whether this is the null value.
    pub fn is_null(&self) -> bool {
        self.type_id == TypeId::registered(TYPE_NULL)
    }

    /// The bytes payload. Rejects any non-bytes tag: no coercion.
    pub fn as_bytes(&self) -> Result<&[u8], ValueError> {
        self.require_builtin(TYPE_BYTES, "bytes")?;
        Ok(&self.payload)
    }

    /// The payload as UTF-8 text. Rejects any non-text tag, and
    /// rejects invalid UTF-8 rather than coercing lossily.
    pub fn as_text(&self) -> Result<&str, ValueError> {
        self.require_builtin(TYPE_TEXT, "text")?;
        std::str::from_utf8(&self.payload).map_err(|_| ValueError::InvalidUtf8)
    }

    /// The big-endian `i64` payload. Rejects any non-integer tag.
    pub fn as_integer(&self) -> Result<i64, ValueError> {
        self.require_builtin(TYPE_INTEGER, "integer")?;
        if self.payload.len() != 8 {
            return Err(ValueError::MalformedPayload {
                type_id: self.type_id.name_string(),
            });
        }
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&self.payload);
        Ok(i64::from_be_bytes(raw))
    }

    /// The nested typed elements. Rejects any non-tuple tag, and
    /// rejects malformed element encodings.
    pub fn as_tuple(&self) -> Result<Vec<TypedValue>, ValueError> {
        self.require_builtin(TYPE_TUPLE, "tuple")?;
        let mut cursor = Cursor {
            bytes: &self.payload,
            pos: 0,
        };
        let elements = parse_elements(&mut cursor, 0, PROFILE_V1)?;
        if cursor.remaining() != 0 {
            return Err(ValueError::MalformedPayload {
                type_id: self.type_id.name_string(),
            });
        }
        Ok(elements)
    }

    fn require_builtin(&self, expected_id: &str, expected: &'static str) -> Result<(), ValueError> {
        if self.type_id == TypeId::registered(expected_id) {
            Ok(())
        } else {
            Err(ValueError::WrongType {
                expected,
                found: self.type_id.name_string(),
            })
        }
    }

    /// Encode to the canonical versioned wire form.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(FORMAT_VERSION);
        push_field(&mut out, self.type_id.profile.as_bytes());
        self.encode_element(&mut out);
        out
    }

    fn encode_element(&self, out: &mut Vec<u8>) {
        match &self.type_id.name {
            TypeName::Registered(name) => {
                out.push(TAG_REGISTERED);
                push_field(out, name.as_bytes());
            }
            TypeName::Code(code) => {
                out.push(TAG_CODE);
                out.extend_from_slice(&code.to_be_bytes());
            }
        }
        push_field(out, &self.payload);
    }

    /// Decode the canonical wire form. Validates the payload shape of
    /// recognized built-in types; any other type is preserved opaque.
    pub fn decode(bytes: &[u8]) -> Result<Self, ValueError> {
        let mut cursor = Cursor { bytes, pos: 0 };
        let version = cursor.byte()?;
        if version != FORMAT_VERSION {
            return Err(ValueError::BadVersion(version));
        }
        let profile = cursor.field()?;
        let profile = std::str::from_utf8(profile).map_err(|_| ValueError::BadProfile)?;
        let type_id = decode_type_id(&mut cursor, profile)?;
        let payload = cursor.field()?.to_vec();
        if cursor.remaining() != 0 {
            return Err(ValueError::TrailingBytes);
        }
        let value = Self { type_id, payload };
        value.validate_shape(0)?;
        Ok(value)
    }

    /// Decode against an explicit codec profile: the wire profile
    /// must equal the codec profile, and every unknown type the
    /// profile lists as critical rejects. Unknown non-critical types
    /// decode to opaque values, including inside tuples.
    pub fn decode_strict(profile: &CodecProfile, bytes: &[u8]) -> Result<Self, ValueError> {
        let value = Self::decode(bytes)?;
        if value.type_id.profile != profile.id {
            return Err(ValueError::ProfileMismatch {
                expected: profile.id.to_string(),
                found: value.type_id.profile.clone(),
            });
        }
        value.check_critical(profile, 0)?;
        Ok(value)
    }

    /// Validate the payload shape for recognized built-in types.
    /// Anything else is opaque and needs no validation.
    fn validate_shape(&self, depth: usize) -> Result<(), ValueError> {
        if depth > MAX_TUPLE_DEPTH {
            return Err(ValueError::DepthLimit);
        }
        let TypeName::Registered(name) = &self.type_id.name else {
            return Ok(());
        };
        if self.type_id.profile != PROFILE_V1 {
            return Ok(());
        }
        match name.as_str() {
            TYPE_NULL => {
                if self.payload.is_empty() {
                    Ok(())
                } else {
                    Err(ValueError::MalformedPayload {
                        type_id: name.clone(),
                    })
                }
            }
            TYPE_BYTES => Ok(()),
            TYPE_TEXT => {
                if std::str::from_utf8(&self.payload).is_ok() {
                    Ok(())
                } else {
                    Err(ValueError::InvalidUtf8)
                }
            }
            TYPE_INTEGER => {
                if self.payload.len() == 8 {
                    Ok(())
                } else {
                    Err(ValueError::MalformedPayload {
                        type_id: name.clone(),
                    })
                }
            }
            TYPE_TUPLE => {
                let mut cursor = Cursor {
                    bytes: &self.payload,
                    pos: 0,
                };
                for element in parse_elements(&mut cursor, depth, PROFILE_V1)? {
                    element.validate_shape(depth + 1)?;
                }
                if cursor.remaining() != 0 {
                    return Err(ValueError::MalformedPayload {
                        type_id: name.clone(),
                    });
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Reject unknown critical types, recursing into tuples. Called
    /// only after [`TypedValue::decode`] has validated shapes.
    fn check_critical(&self, profile: &CodecProfile, depth: usize) -> Result<(), ValueError> {
        if depth > MAX_TUPLE_DEPTH {
            return Err(ValueError::DepthLimit);
        }
        if let TypeName::Registered(name) = &self.type_id.name {
            if !self.type_id.is_builtin() && profile.critical.contains(&name.as_str()) {
                return Err(ValueError::UnknownCriticalType {
                    type_id: name.clone(),
                });
            }
            if name.as_str() == TYPE_TUPLE && self.type_id.profile == profile.id {
                let mut cursor = Cursor {
                    bytes: &self.payload,
                    pos: 0,
                };
                for element in parse_elements(&mut cursor, depth, profile.id)? {
                    element.check_critical(profile, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

fn decode_type_id(cursor: &mut Cursor<'_>, profile: &str) -> Result<TypeId, ValueError> {
    let name = match cursor.byte()? {
        TAG_REGISTERED => {
            let raw = cursor.field()?;
            let name =
                std::str::from_utf8(raw).map_err(|_| ValueError::BadProfile)?;
            if name.is_empty() {
                return Err(ValueError::EmptyTypeName);
            }
            TypeName::Registered(name.to_string())
        }
        TAG_CODE => TypeName::Code(cursor.u64()?),
        tag => return Err(ValueError::UnknownTag(tag)),
    };
    Ok(TypeId {
        profile: profile.to_string(),
        name,
    })
}

/// Parse `count u64` followed by that many nested elements from the
/// middle of a tuple payload. Nested elements inherit the enclosing
/// `profile`, so only their tag, body, length and bytes appear.
fn parse_elements(
    cursor: &mut Cursor<'_>,
    depth: usize,
    profile: &str,
) -> Result<Vec<TypedValue>, ValueError> {
    if depth > MAX_TUPLE_DEPTH {
        return Err(ValueError::DepthLimit);
    }
    let count = cursor.u64()?;
    if count > cursor.remaining() as u64 {
        return Err(ValueError::Truncated);
    }
    let count = usize::try_from(count).map_err(|_| ValueError::TooLarge)?;
    let mut elements = Vec::with_capacity(count);
    for _ in 0..count {
        // The tag byte costs at least one byte per element; this also
        // bounds the allocation below by the input size.
        if cursor.remaining() == 0 {
            return Err(ValueError::Truncated);
        }
        let name = match cursor.byte()? {
            TAG_REGISTERED => {
                let raw = cursor.field()?;
                let name =
                    std::str::from_utf8(raw).map_err(|_| ValueError::BadProfile)?;
                if name.is_empty() {
                    return Err(ValueError::EmptyTypeName);
                }
                TypeName::Registered(name.to_string())
            }
            TAG_CODE => TypeName::Code(cursor.u64()?),
            tag => return Err(ValueError::UnknownTag(tag)),
        };
        let payload = cursor.field()?.to_vec();
        let element = TypedValue {
            type_id: TypeId {
                profile: profile.to_string(),
                name,
            },
            payload,
        };
        elements.push(element);
    }
    Ok(elements)
}

/// An explicit codec profile: the boundary for structured object
/// values. Unknown types not listed in `critical` decode to opaque
/// values; unknown types listed there reject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecProfile {
    /// Profile identifier matched against the wire profile.
    pub id: &'static str,
    /// Registered type names the reader cannot proceed without
    /// understanding. Numeric codes cannot be listed here and are
    /// always preserved opaque.
    pub critical: &'static [&'static str],
}

impl CodecProfile {
    /// The V1 codec with no critical types: every unknown type is
    /// preserved opaque.
    pub fn permissive() -> Self {
        Self {
            id: PROFILE_V1,
            critical: &[],
        }
    }

    /// The V1 codec with the given registered type names critical.
    pub fn new(critical: &'static [&'static str]) -> Self {
        Self {
            id: PROFILE_V1,
            critical,
        }
    }
}

/// Why bytes are not an admitted typed value, or why a typed
/// accessor refuses a value. Failures are typed and fail-closed:
/// never a panic, never a silent coercion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueError {
    /// First byte is not [`FORMAT_VERSION`].
    BadVersion(u8),
    /// Input ends before the value is complete.
    Truncated,
    /// Extra bytes follow a complete value.
    TrailingBytes,
    /// A type-tag byte other than registered (0) or code (1).
    UnknownTag(u8),
    /// A declared count or length does not fit this platform.
    TooLarge,
    /// A profile or type name is not valid UTF-8.
    BadProfile,
    /// A registered type name is empty.
    EmptyTypeName,
    /// A text payload is not valid UTF-8.
    InvalidUtf8,
    /// A recognized built-in type carries a payload outside its
    /// declared shape (e.g. a 7-byte integer, a non-empty null).
    MalformedPayload {
        /// The offending built-in type name.
        type_id: String,
    },
    /// Tuple nesting exceeds [`MAX_TUPLE_DEPTH`].
    DepthLimit,
    /// The wire profile is not the codec profile of the reader.
    ProfileMismatch {
        /// The profile the reader enforces.
        expected: String,
        /// The profile on the wire.
        found: String,
    },
    /// An unknown type the codec profile marks critical.
    UnknownCriticalType {
        /// The unknown registered type name.
        type_id: String,
    },
    /// A typed accessor was called on the wrong type. The payload
    /// is never coerced: read it through the matching accessor or
    /// preserve it opaque.
    WrongType {
        /// The built-in kind the accessor requires.
        expected: &'static str,
        /// The tag the value actually carries.
        found: String,
    },
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadVersion(v) => write!(f, "unsupported typed-value format version {v}"),
            Self::Truncated => f.write_str("truncated typed-value encoding"),
            Self::TrailingBytes => f.write_str("trailing bytes after typed-value encoding"),
            Self::UnknownTag(t) => write!(f, "unknown typed-value type tag {t}"),
            Self::TooLarge => f.write_str("typed-value count or length too large"),
            Self::BadProfile => f.write_str("typed-value profile or type name is not UTF-8"),
            Self::EmptyTypeName => f.write_str("typed-value registered type name is empty"),
            Self::InvalidUtf8 => f.write_str("typed-value text payload is not valid UTF-8"),
            Self::MalformedPayload { type_id } => {
                write!(f, "payload does not match built-in type {type_id}")
            }
            Self::DepthLimit => f.write_str("typed-value tuple nesting exceeds the depth limit"),
            Self::ProfileMismatch { expected, found } => write!(
                f,
                "typed-value profile {found} does not match codec profile {expected}"
            ),
            Self::UnknownCriticalType { type_id } => {
                write!(f, "unknown critical typed-value type: {type_id}")
            }
            Self::WrongType { expected, found } => {
                write!(f, "typed value is {found}, not {expected}")
            }
        }
    }
}

impl std::error::Error for ValueError {}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn byte(&mut self) -> Result<u8, ValueError> {
        let byte = *self.bytes.get(self.pos).ok_or(ValueError::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ValueError> {
        if self.remaining() < len {
            return Err(ValueError::Truncated);
        }
        let slice = &self.bytes[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    fn u64(&mut self) -> Result<u64, ValueError> {
        let raw = self.take(8)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(raw);
        Ok(u64::from_be_bytes(buf))
    }

    fn field(&mut self) -> Result<&'a [u8], ValueError> {
        let len = self.u64()?;
        if len > self.remaining() as u64 {
            return Err(ValueError::Truncated);
        }
        let len = usize::try_from(len).map_err(|_| ValueError::TooLarge)?;
        self.take(len)
    }
}

fn push_field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
