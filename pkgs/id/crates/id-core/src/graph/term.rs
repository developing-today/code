//! Typed-byte graph terms and triples (slice A-GRAPH-9125-01, #10196;
//! design part A §3.1 and §4.2, statements S5.1–S5.7, S5.11).
//!
//! A term is either explicit typed bytes, a `type_tag` plus opaque
//! bytes, or an explicitly tagged reference to another object. Terms
//! are a general foundation, not RDF: equality is exact
//! `(type_tag, bytes)` equality, the codec never normalizes the bytes,
//! and a byte string that looks like an IRI is still only bytes. An
//! object reference is its own kind of term, never a string coercion.
//!
//! # Wire form
//!
//! Each term is a deterministic-CBOR array of exactly three items whose
//! first item names the kind:
//!
//! | kind | form |
//! |------|------|
//! | typed bytes | `[0, type_tag: uint, bytes: bstr]` |
//! | content reference | `[1, profile: tstr, digest: bstr(32)]` |
//! | signed-envelope reference | `[2, profile: tstr, digest: bstr(32)]` |
//!
//! A triple is the array `[subject, predicate, object]`. Every other
//! shape (another kind number, another length, a wrong item type, an
//! unknown hash profile, a digest of the wrong size) is rejected, so
//! one term has exactly one encoding and one encoding one term. Unknown
//! `type_tag` values are preserved opaquely; only an unknown *kind* is
//! refused.

use std::fmt;

use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile};
use crate::signed_envelope::EnvelopeId;

/// Kind number of a typed-bytes term.
pub const KIND_TYPED: u64 = 0;
/// Kind number of a content reference.
pub const KIND_CONTENT: u64 = 1;
/// Kind number of a signed-envelope reference.
pub const KIND_ENVELOPE: u64 = 2;

/// Upper bound on one term's opaque bytes, checked before they are
/// copied.
pub const MAX_TERM_BYTES: usize = 64 * 1024;

/// A reference to another object, by its identity. Never a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectRef {
    /// Content named by profile and digest.
    Content(ContentId),
    /// A signed envelope, named by the hash of its complete canonical
    /// bytes including the signature.
    Envelope(EnvelopeId),
}

/// One position of a triple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    /// Opaque bytes under an application-assigned type tag.
    Typed {
        /// The tag that says how to read `bytes`.
        type_tag: u64,
        /// The exact bytes, never normalized.
        bytes: Vec<u8>,
    },
    /// A typed reference to another object.
    Object(ObjectRef),
}

/// A subject, predicate and object.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Triple {
    /// What the triple is about.
    pub subject: Term,
    /// The relation.
    pub predicate: Term,
    /// The value or target.
    pub object: Term,
}

/// Why a term or triple was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermError {
    /// The bytes are not canonical CBOR.
    Cbor(CborError),
    /// The value does not have the shape of a term or triple.
    Malformed {
        /// What was being read.
        what: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The kind number is not one this reader knows.
    UnknownKind(u64),
    /// A reference names a hash profile this reader does not know.
    UnknownProfile(String),
    /// A term's bytes exceed [`MAX_TERM_BYTES`].
    TooLarge {
        /// The length found.
        len: usize,
    },
}

impl TermError {
    /// A stable failure class: `malformed`, `unsupported` or
    /// `resource_limit`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cbor(_) | Self::Malformed { .. } => "malformed",
            Self::UnknownKind(_) | Self::UnknownProfile(_) => "unsupported",
            Self::TooLarge { .. } => "resource_limit",
        }
    }
}

impl fmt::Display for TermError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "term encoding: {error}"),
            Self::Malformed { what, problem } => write!(f, "{what} {problem}"),
            Self::UnknownKind(kind) => write!(f, "unknown term kind {kind}"),
            Self::UnknownProfile(token) => write!(f, "unknown hash profile {token:?}"),
            Self::TooLarge { len } => {
                write!(
                    f,
                    "term is {len} bytes, over the {MAX_TERM_BYTES}-byte limit"
                )
            }
        }
    }
}

impl std::error::Error for TermError {}

impl From<CborError> for TermError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

const fn malformed(what: &'static str, problem: &'static str) -> TermError {
    TermError::Malformed { what, problem }
}

fn id_value(kind: u64, id: &ContentId) -> Value {
    Value::Array(vec![
        Value::UInt(kind),
        Value::Text(id.profile().token().to_owned()),
        Value::Bytes(id.digest().to_vec()),
    ])
}

fn id_from(profile: &Value, digest: &Value) -> Result<ContentId, TermError> {
    let Value::Text(token) = profile else {
        return Err(malformed("reference", "profile must be text"));
    };
    let profile =
        HashProfile::from_token(token).ok_or_else(|| TermError::UnknownProfile(token.clone()))?;
    let Value::Bytes(digest) = digest else {
        return Err(malformed("reference", "digest must be bytes"));
    };
    let digest = <[u8; DIGEST_BYTES]>::try_from(digest.as_slice())
        .map_err(|_error| malformed("reference", "digest must be 32 bytes"))?;
    Ok(ContentId::new(profile, digest))
}

impl Term {
    /// A typed-bytes term.
    #[must_use]
    pub const fn typed(type_tag: u64, bytes: Vec<u8>) -> Self {
        Self::Typed { type_tag, bytes }
    }

    /// The term as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`TermError::TooLarge`] for bytes over [`MAX_TERM_BYTES`].
    pub fn to_value(&self) -> Result<Value, TermError> {
        Ok(match self {
            Self::Typed { type_tag, bytes } => {
                if bytes.len() > MAX_TERM_BYTES {
                    return Err(TermError::TooLarge { len: bytes.len() });
                }
                Value::Array(vec![
                    Value::UInt(KIND_TYPED),
                    Value::UInt(*type_tag),
                    Value::Bytes(bytes.clone()),
                ])
            }
            Self::Object(ObjectRef::Content(id)) => id_value(KIND_CONTENT, id),
            Self::Object(ObjectRef::Envelope(id)) => id_value(KIND_ENVELOPE, id.content_id()),
        })
    }

    /// Reads a term from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// [`TermError`] for any shape but the three kinds above.
    pub fn from_value(value: &Value) -> Result<Self, TermError> {
        let Value::Array(items) = value else {
            return Err(malformed("term", "must be an array"));
        };
        let [kind, first, second] = items.as_slice() else {
            return Err(malformed("term", "must have exactly three items"));
        };
        let Value::UInt(kind) = kind else {
            return Err(malformed("term", "kind must be an unsigned integer"));
        };
        match *kind {
            KIND_TYPED => {
                let Value::UInt(type_tag) = first else {
                    return Err(malformed("term", "type tag must be an unsigned integer"));
                };
                let Value::Bytes(bytes) = second else {
                    return Err(malformed("term", "bytes must be a byte string"));
                };
                if bytes.len() > MAX_TERM_BYTES {
                    return Err(TermError::TooLarge { len: bytes.len() });
                }
                Ok(Self::Typed {
                    type_tag: *type_tag,
                    bytes: bytes.clone(),
                })
            }
            KIND_CONTENT => Ok(Self::Object(ObjectRef::Content(id_from(first, second)?))),
            KIND_ENVELOPE => Ok(Self::Object(ObjectRef::Envelope(
                EnvelopeId::from_content_id(id_from(first, second)?),
            ))),
            other => Err(TermError::UnknownKind(other)),
        }
    }

    /// `canonical_cbor(term)`.
    ///
    /// # Errors
    ///
    /// As [`Term::to_value`], or an encoding error.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TermError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes one canonical term, rejecting trailing bytes.
    ///
    /// # Errors
    ///
    /// As [`Term::from_value`], or a canonical-CBOR error.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TermError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// The object this term references, if it is a reference.
    #[must_use]
    pub const fn object_ref(&self) -> Option<&ObjectRef> {
        match self {
            Self::Object(reference) => Some(reference),
            Self::Typed { .. } => None,
        }
    }
}

impl Triple {
    /// A triple from its three terms.
    #[must_use]
    pub const fn new(subject: Term, predicate: Term, object: Term) -> Self {
        Self {
            subject,
            predicate,
            object,
        }
    }

    /// The three terms in subject, predicate, object order.
    #[must_use]
    pub const fn terms(&self) -> [&Term; 3] {
        [&self.subject, &self.predicate, &self.object]
    }

    /// The triple as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// As [`Term::to_value`].
    pub fn to_value(&self) -> Result<Value, TermError> {
        Ok(Value::Array(vec![
            self.subject.to_value()?,
            self.predicate.to_value()?,
            self.object.to_value()?,
        ]))
    }

    /// Reads a triple from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// [`TermError`] when the value is not a three-term array.
    pub fn from_value(value: &Value) -> Result<Self, TermError> {
        let Value::Array(items) = value else {
            return Err(malformed("triple", "must be an array"));
        };
        let [subject, predicate, object] = items.as_slice() else {
            return Err(malformed("triple", "must have exactly three terms"));
        };
        Ok(Self {
            subject: Term::from_value(subject)?,
            predicate: Term::from_value(predicate)?,
            object: Term::from_value(object)?,
        })
    }

    /// `canonical_cbor(triple)`.
    ///
    /// # Errors
    ///
    /// As [`Triple::to_value`], or an encoding error.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TermError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes one canonical triple, rejecting trailing bytes.
    ///
    /// # Errors
    ///
    /// As [`Triple::from_value`], or a canonical-CBOR error.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TermError> {
        Self::from_value(&decode_canonical(bytes)?)
    }
}
