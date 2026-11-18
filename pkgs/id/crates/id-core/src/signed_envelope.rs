//! Payload, body, signature and signed-envelope identity separation
//! (slice A9114-envelope-identities, issue #10226; parent #9114; design
//! `part-a-addressing/versioned-design.md` section "#9114 — Complete
//! metadata sidecars and mutable head", A-9114.1–A-9114.3, statements
//! S3.11–S3.20).
//!
//! Four identities, each over different bytes, none of them recursive:
//!
//! - [`PayloadId`] = `{profile, H_profile(raw_payload)}`. The raw
//!   payload is hashed exactly as carried; the raw BLAKE3 profile adds no
//!   prefix.
//! - The canonical [`Body`] commits the payload ID, payload length,
//!   signing domain and context, signer, key and suite, and signed
//!   references (scoped subject, logical graph, predecessor envelope).
//!   It never holds the raw payload, the signature, or the ID of the
//!   envelope that encloses it.
//! - The signature covers [`signature_preimage`]: a fixed domain
//!   prefix, the length-framed signing domain, then
//!   `canonical_cbor(Body)`.
//! - [`EnvelopeId`] = `{profile, H_profile(canonical_cbor(Envelope))}`
//!   over the complete signed envelope: body, raw payload and
//!   signature. It is computed after signing and never written into
//!   any bytes it hashes, so a changed signature changes the envelope
//!   ID while the body and payload IDs stay put.
//!
//! [`LogicalGraphId`] is a stable space address assigned by its owner,
//! not derived from any content hash, so it stays the same across every
//! assertion envelope written into that graph.
//!
//! Encoding uses the deterministic CBOR profile of `canonical_cbor`
//! (A-9114.3). Every field of a V1 body and envelope is critical: a
//! reader rejects any field it does not know, because a verifier that
//! dropped an unknown signed field would re-encode different bytes from
//! the ones that were signed. Absent optional fields are omitted, never
//! encoded as null, so each value has exactly one encoding.
//!
//! Payload verification goes through the configured
//! [`ProfileDispatch`] (A9113-profile-dispatch), so a payload under a
//! profile this node does not admit is `unsupported_profile`, not a
//! mismatch.
//!
//! The crate-root export belongs to slice 9110-02; until that lands the
//! integration test includes this module and its dependencies by path.

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::hash_profile::{ProfileDispatch, VerifyError};
use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile, IdShapeError};

/// Fixed prefix of every V1 signature preimage, ending in a NUL so no
/// domain text can extend it.
pub const SIGNATURE_DOMAIN_PREFIX: &[u8] = b"id/signed-body/v1\0";

/// The only signature suite in V1.
pub const SUITE_ED25519: &str = "ed25519";

/// The body format version this module writes and reads.
pub const BODY_VERSION: u64 = 1;

/// Upper bound on the signing-domain and context text, in bytes.
pub const MAX_LABEL_BYTES: usize = 256;

/// Identity of the exact raw payload bytes under an explicit profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PayloadId(ContentId);

impl PayloadId {
    /// Hashes `raw_payload` exactly as carried under `profile`.
    ///
    /// # Errors
    ///
    /// The profile's own rejection, such as non-UTF-8 bytes under the
    /// Atlas text profile.
    pub fn of(profile: HashProfile, raw_payload: &[u8]) -> Result<Self, IdShapeError> {
        ContentId::of_bytes(profile, raw_payload).map(Self)
    }

    /// The underlying content ID.
    #[must_use]
    pub const fn content_id(&self) -> &ContentId {
        &self.0
    }
}

/// Identity of a complete canonical signed envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnvelopeId(ContentId);

impl EnvelopeId {
    /// Wraps an envelope ID read from a signed reference.
    #[must_use]
    pub const fn from_content_id(id: ContentId) -> Self {
        Self(id)
    }

    /// The underlying content ID.
    #[must_use]
    pub const fn content_id(&self) -> &ContentId {
        &self.0
    }
}

/// A stable logical graph (space) address. Assigned by its owner and
/// never computed from assertion content, so it does not change when an
/// assertion in the graph does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogicalGraphId([u8; DIGEST_BYTES]);

impl LogicalGraphId {
    /// A graph address from its assigned bytes.
    #[must_use]
    pub const fn new(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// The address bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.0
    }
}

/// The canonical signed body. Holds references and lengths, never the
/// payload, the signature or its own enclosing envelope ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// Signing domain, also bound into the signature prefix.
    pub domain: String,
    /// Context within the domain, such as a namespace or tenant scope.
    pub context: String,
    /// The scoped subject this record is about.
    pub subject: Option<ContentId>,
    /// The logical graph this record is written into.
    pub graph: Option<LogicalGraphId>,
    /// The envelope this record supersedes or follows.
    pub predecessor: Option<EnvelopeId>,
    /// Who signed, as the signer names itself.
    pub signer_id: String,
    /// The signer's Ed25519 public key.
    pub key: [u8; 32],
    /// The signature suite; [`SUITE_ED25519`] in V1.
    pub suite: String,
    /// Identity of the raw payload.
    pub payload_id: PayloadId,
    /// Exact length of the raw payload in bytes.
    pub payload_length: u64,
}

/// Why an envelope or body was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The bytes are not canonical CBOR, or carry an unknown field.
    Cbor(CborError),
    /// An ID inside the record was rejected, or the payload did not
    /// verify against its payload ID.
    Id(IdShapeError),
    /// A field is missing or has the wrong type or size.
    Field {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The body version is not [`BODY_VERSION`].
    UnsupportedVersion(u64),
    /// The suite is not one this reader verifies.
    UnsupportedSuite(String),
    /// The carried payload is not the length the body commits.
    PayloadLength {
        /// Length committed in the body.
        declared: u64,
        /// Length actually carried.
        actual: u64,
    },
    /// The body's key is not a valid Ed25519 public key.
    BadKey,
    /// The signing key does not match the body's committed key.
    KeyMismatch,
    /// The signature does not verify.
    BadSignature,
}

impl EnvelopeError {
    /// A stable failure class: `malformed`, `unsupported_profile`,
    /// `digest_mismatch`, `invalid_profile_parameters`,
    /// `resource_limit`, `unsupported`, `payload_length`, `bad_key` or
    /// `bad_signature`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Id(IdShapeError::UnsupportedProfile { .. }) => "unsupported_profile",
            Self::Id(IdShapeError::DigestMismatch) => "digest_mismatch",
            Self::Id(IdShapeError::InvalidProfileParameters { .. }) => "invalid_profile_parameters",
            Self::Id(IdShapeError::ResourceLimit { .. }) => "resource_limit",
            Self::Cbor(_) | Self::Field { .. } | Self::Id(_) => "malformed",
            Self::UnsupportedVersion(_) | Self::UnsupportedSuite(_) => "unsupported",
            Self::PayloadLength { .. } => "payload_length",
            Self::BadKey | Self::KeyMismatch => "bad_key",
            Self::BadSignature => "bad_signature",
        }
    }
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "envelope encoding: {error}"),
            Self::Id(error) => write!(f, "envelope identity: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported body version {version}"),
            Self::UnsupportedSuite(suite) => write!(f, "unsupported signature suite {suite:?}"),
            Self::PayloadLength { declared, actual } => {
                write!(
                    f,
                    "payload is {actual} bytes but the body commits {declared}"
                )
            }
            Self::BadKey => write!(f, "body key is not a valid Ed25519 public key"),
            Self::KeyMismatch => write!(f, "signing key does not match the body's key"),
            Self::BadSignature => write!(f, "signature does not verify"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl From<CborError> for EnvelopeError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for EnvelopeError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

impl From<VerifyError> for EnvelopeError {
    fn from(error: VerifyError) -> Self {
        match error {
            VerifyError::Rejected(error) => Self::Id(error),
            // A slice reader never fails; keep the class honest anyway.
            VerifyError::Io(_) => Self::Field {
                field: "payload",
                problem: "could not be read",
            },
        }
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn content_id_value(id: &ContentId) -> Value {
    // Keys in canonical order: "digest" < "profile".
    Value::Map(vec![
        (text("digest"), Value::Bytes(id.digest().to_vec())),
        (text("profile"), text(id.profile().token())),
    ])
}

/// Builds a map from `(key, value)` pairs, in canonical key order
/// (the encoder sorts too; this keeps the value itself canonical).
fn map(mut entries: Vec<(&'static str, Value)>) -> Value {
    // For text keys under 24 bytes, encoded-byte order is shorter
    // first, then bytewise.
    entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    Value::Map(entries.into_iter().map(|(k, v)| (text(k), v)).collect())
}

impl Body {
    /// The body as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError::Field`] when the domain or context is empty or
    /// over [`MAX_LABEL_BYTES`].
    pub fn to_value(&self) -> Result<Value, EnvelopeError> {
        check_label("domain", &self.domain)?;
        check_label("context", &self.context)?;
        let mut entries = vec![
            ("v", Value::UInt(BODY_VERSION)),
            ("domain", text(&self.domain)),
            ("context", text(&self.context)),
            ("signer_id", text(&self.signer_id)),
            ("key", Value::Bytes(self.key.to_vec())),
            ("suite", text(&self.suite)),
            ("payload_id", content_id_value(self.payload_id.content_id())),
            ("payload_length", Value::UInt(self.payload_length)),
        ];
        if let Some(subject) = &self.subject {
            entries.push(("subject", content_id_value(subject)));
        }
        if let Some(graph) = &self.graph {
            entries.push(("graph", Value::Bytes(graph.as_bytes().to_vec())));
        }
        if let Some(predecessor) = &self.predecessor {
            entries.push(("predecessor", content_id_value(predecessor.content_id())));
        }
        Ok(map(entries))
    }

    /// `canonical_cbor(Body)`.
    ///
    /// # Errors
    ///
    /// As [`Body::to_value`].
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, EnvelopeError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a body from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError`] for an unknown field, a missing or mistyped
    /// field, an unknown version or an unsupported ID profile token.
    pub fn from_value(value: &Value) -> Result<Self, EnvelopeError> {
        let fields = Fields::new(
            value,
            "body",
            &[
                "v",
                "domain",
                "context",
                "subject",
                "graph",
                "predecessor",
                "signer_id",
                "key",
                "suite",
                "payload_id",
                "payload_length",
            ],
        )?;
        let version = fields.uint("v")?;
        if version != BODY_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(version));
        }
        let domain = fields.text("domain")?;
        let context = fields.text("context")?;
        check_label("domain", &domain)?;
        check_label("context", &context)?;
        Ok(Self {
            domain,
            context,
            subject: fields
                .optional("subject")
                .map(|v| content_id_from(v, "subject"))
                .transpose()?,
            graph: fields
                .optional("graph")
                .map(|v| fixed_bytes(v, "graph").map(LogicalGraphId::new))
                .transpose()?,
            predecessor: fields
                .optional("predecessor")
                .map(|v| content_id_from(v, "predecessor").map(EnvelopeId))
                .transpose()?,
            signer_id: fields.text("signer_id")?,
            key: fixed_bytes(fields.required("key")?, "key")?,
            suite: fields.text("suite")?,
            payload_id: PayloadId(content_id_from(
                fields.required("payload_id")?,
                "payload_id",
            )?),
            payload_length: fields.uint("payload_length")?,
        })
    }
}

const fn check_label(field: &'static str, label: &str) -> Result<(), EnvelopeError> {
    if label.is_empty() {
        return Err(EnvelopeError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if label.len() > MAX_LABEL_BYTES {
        return Err(EnvelopeError::Field {
            field,
            problem: "is too long",
        });
    }
    Ok(())
}

/// A decoded map whose every key is known.
struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(
        value: &'a Value,
        record: &'static str,
        known: &[&'static str],
    ) -> Result<Self, EnvelopeError> {
        let Value::Map(entries) = value else {
            return Err(EnvelopeError::Field {
                field: record,
                problem: "must be a map",
            });
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(EnvelopeError::Field {
                    field: record,
                    problem: "keys must be text",
                });
            };
            if !known.contains(&name.as_str()) {
                // Every V1 field is critical.
                return Err(EnvelopeError::Cbor(CborError::UnknownCriticalField {
                    field: name.clone(),
                }));
            }
        }
        Ok(Self { entries })
    }

    fn optional(&self, field: &str) -> Option<&'a Value> {
        self.entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Text(name) if name == field))
            .map(|(_, value)| value)
    }

    fn required(&self, field: &'static str) -> Result<&'a Value, EnvelopeError> {
        self.optional(field).ok_or(EnvelopeError::Field {
            field,
            problem: "is missing",
        })
    }

    fn text(&self, field: &'static str) -> Result<String, EnvelopeError> {
        match self.required(field)? {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(EnvelopeError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn uint(&self, field: &'static str) -> Result<u64, EnvelopeError> {
        match self.required(field)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(EnvelopeError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn bytes(&self, field: &'static str) -> Result<Vec<u8>, EnvelopeError> {
        match self.required(field)? {
            Value::Bytes(value) => Ok(value.clone()),
            _ => Err(EnvelopeError::Field {
                field,
                problem: "must be a byte string",
            }),
        }
    }
}

fn fixed_bytes<const N: usize>(
    value: &Value,
    field: &'static str,
) -> Result<[u8; N], EnvelopeError> {
    match value {
        Value::Bytes(bytes) => {
            <[u8; N]>::try_from(bytes.as_slice()).map_err(|_error| EnvelopeError::Field {
                field,
                problem: "has the wrong length",
            })
        }
        _ => Err(EnvelopeError::Field {
            field,
            problem: "must be a byte string",
        }),
    }
}

fn content_id_from(value: &Value, field: &'static str) -> Result<ContentId, EnvelopeError> {
    let fields = Fields::new(value, field, &["digest", "profile"])?;
    let token = fields.text("profile")?;
    let profile = HashProfile::from_token(&token)
        .ok_or(IdShapeError::UnsupportedProfile { profile: token })?;
    let digest = fixed_bytes(fields.required("digest")?, field)?;
    Ok(ContentId::new(profile, digest))
}

/// The exact bytes the body signature covers:
/// `SIGNATURE_DOMAIN_PREFIX || u32_be(len(domain)) || domain ||
/// canonical_cbor(body)`.
///
/// # Errors
///
/// As [`Body::to_value`].
pub fn signature_preimage(body: &Body) -> Result<Vec<u8>, EnvelopeError> {
    let encoded = body.canonical_bytes()?;
    let domain = body.domain.as_bytes();
    // check_label bounds the domain well under u32::MAX.
    let domain_len = u32::try_from(domain.len()).map_err(|_error| EnvelopeError::Field {
        field: "domain",
        problem: "is too long",
    })?;
    let mut preimage =
        Vec::with_capacity(SIGNATURE_DOMAIN_PREFIX.len() + 4 + domain.len() + encoded.len());
    preimage.extend_from_slice(SIGNATURE_DOMAIN_PREFIX);
    preimage.extend_from_slice(&domain_len.to_be_bytes());
    preimage.extend_from_slice(domain);
    preimage.extend_from_slice(&encoded);
    Ok(preimage)
}

/// A signed envelope: the body, the raw payload it commits, and the
/// detached signature over the body's preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The signed body.
    pub body: Body,
    /// The raw payload bytes, exactly as committed by the body.
    pub payload: Vec<u8>,
    /// Ed25519 signature over [`signature_preimage`] of the body.
    pub signature: [u8; 64],
}

impl Envelope {
    /// Signs `body` with `key` and pairs it with `payload`.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError::KeyMismatch`] when `key` is not the body's
    /// key, [`EnvelopeError::UnsupportedSuite`] for a non-Ed25519
    /// suite, [`EnvelopeError::PayloadLength`] or a digest mismatch
    /// when `payload` is not what the body commits, or an encoding
    /// error.
    pub fn sign(body: Body, payload: Vec<u8>, key: &SigningKey) -> Result<Self, EnvelopeError> {
        if key.verifying_key().to_bytes() != body.key {
            return Err(EnvelopeError::KeyMismatch);
        }
        check_suite(&body)?;
        check_payload(&body, &payload, &ProfileDispatch::default())?;
        let signature = key.sign(&signature_preimage(&body)?).to_bytes();
        Ok(Self {
            body,
            payload,
            signature,
        })
    }

    /// The envelope as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// As [`Body::to_value`].
    pub fn to_value(&self) -> Result<Value, EnvelopeError> {
        Ok(map(vec![
            ("body", self.body.to_value()?),
            ("payload", Value::Bytes(self.payload.clone())),
            ("signature", Value::Bytes(self.signature.to_vec())),
        ]))
    }

    /// `canonical_cbor(Envelope)`: the bytes the envelope ID hashes.
    ///
    /// # Errors
    ///
    /// As [`Body::to_value`].
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, EnvelopeError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes a canonical envelope. Decoding checks structure only;
    /// call [`Envelope::verify`] before trusting it.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError`] for non-canonical bytes or any field problem.
    pub fn decode(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::new(&value, "envelope", &["body", "payload", "signature"])?;
        let signature = fields.bytes("signature")?;
        Ok(Self {
            body: Body::from_value(fields.required("body")?)?,
            payload: fields.bytes("payload")?,
            signature: <[u8; 64]>::try_from(signature.as_slice()).map_err(|_error| {
                EnvelopeError::Field {
                    field: "signature",
                    problem: "has the wrong length",
                }
            })?,
        })
    }

    /// The envelope's identity under `profile`, over its complete
    /// canonical bytes (body, payload and signature).
    ///
    /// # Errors
    ///
    /// An encoding error, or the profile's own rejection.
    pub fn id(&self, profile: HashProfile) -> Result<EnvelopeId, EnvelopeError> {
        Ok(EnvelopeId(ContentId::of_bytes(
            profile,
            &self.canonical_bytes()?,
        )?))
    }

    /// Verifies the payload against the body's payload ID and length,
    /// and the signature against the body's key.
    ///
    /// # Errors
    ///
    /// [`EnvelopeError`], classed by [`EnvelopeError::class`].
    pub fn verify(&self, dispatch: &ProfileDispatch) -> Result<(), EnvelopeError> {
        check_suite(&self.body)?;
        check_payload(&self.body, &self.payload, dispatch)?;
        let key =
            VerifyingKey::from_bytes(&self.body.key).map_err(|_error| EnvelopeError::BadKey)?;
        key.verify_strict(
            &signature_preimage(&self.body)?,
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_error| EnvelopeError::BadSignature)
    }
}

fn check_suite(body: &Body) -> Result<(), EnvelopeError> {
    if body.suite == SUITE_ED25519 {
        Ok(())
    } else {
        Err(EnvelopeError::UnsupportedSuite(body.suite.clone()))
    }
}

fn check_payload(
    body: &Body,
    payload: &[u8],
    dispatch: &ProfileDispatch,
) -> Result<(), EnvelopeError> {
    let actual = payload.len() as u64;
    if actual != body.payload_length {
        return Err(EnvelopeError::PayloadLength {
            declared: body.payload_length,
            actual,
        });
    }
    dispatch.verify_bytes(body.payload_id.content_id(), payload)?;
    Ok(())
}
