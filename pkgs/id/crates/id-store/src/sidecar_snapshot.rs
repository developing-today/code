//! Complete, scoped signed metadata sidecar snapshots (A9114-sidecar-snapshot).
//!
//! A snapshot is an immutable signed envelope. Its body commits the complete
//! namespace and tenant scope, generation, parent, payload identity and
//! signer metadata. The snapshot ID is the raw BLAKE3 digest of the complete
//! canonical envelope, so it is never part of the bytes it names.

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use id_core::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use id_core::id_shape::{ContentId, HashProfile, IdShapeError};
use id_core::signed_envelope::{EnvelopeId, PayloadId};

/// The profile used for digest-derived snapshot IDs.
pub const SNAPSHOT_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// The only signature suite admitted by V1 snapshots.
pub const SUITE_ED25519: &str = "ed25519";

const BODY_VERSION: u64 = 1;
const SIGNATURE_DOMAIN_PREFIX: &[u8] = b"id/signed-body/v1\0";
const MAX_LABEL_BYTES: usize = 256;

/// The snapshot ID is the content ID of the complete signed envelope.
pub type SnapshotId = EnvelopeId;

/// Why a sidecar snapshot was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// The bytes are not canonical CBOR or contain an unknown critical field.
    Cbor(CborError),
    /// An embedded content or payload ID is malformed.
    Id(IdShapeError),
    /// A field is missing, invalid or inconsistent with another field.
    Field {
        /// The field that failed validation.
        field: &'static str,
        /// The reason it failed.
        problem: &'static str,
    },
    /// The body and carried payload lengths differ.
    PayloadLength {
        /// Length committed by the body.
        declared: u64,
        /// Length actually carried.
        actual: u64,
    },
    /// The signing key does not match the body key.
    KeyMismatch,
    /// The body key is not a valid Ed25519 key.
    BadKey,
    /// The detached signature does not verify.
    BadSignature,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "snapshot encoding: {error}"),
            Self::Id(error) => write!(f, "snapshot identity: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::PayloadLength { declared, actual } => {
                write!(f, "payload is {actual} bytes but body commits {declared}")
            }
            Self::KeyMismatch => f.write_str("signing key does not match body key"),
            Self::BadKey => f.write_str("body key is not a valid Ed25519 key"),
            Self::BadSignature => f.write_str("snapshot signature does not verify"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<CborError> for SnapshotError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for SnapshotError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

/// The signed metadata and scope for one immutable snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarBody {
    /// The content whose metadata this snapshot describes.
    pub subject_content_id: ContentId,
    /// The explicit namespace containing the metadata view.
    pub namespace_id: String,
    /// The authenticated tenant or scope name.
    pub tenant_scope: String,
    /// Monotone version within the subject/scope chain.
    pub generation: u64,
    /// The preceding snapshot, required for every generation after zero.
    pub parent_snapshot_id: Option<SnapshotId>,
    /// Advisory creation timestamp; it is not the snapshot identity.
    pub created_at: u64,
    /// Signature domain.
    pub domain: String,
    /// Domain context, such as a metadata schema or operation family.
    pub context: String,
    /// Human-readable signer identifier.
    pub signer_id: String,
    /// Stable identifier for the signing key.
    pub key_id: String,
    /// Ed25519 public key used to verify the detached signature.
    pub key: [u8; 32],
    /// Signature suite.
    pub suite: String,
    /// Profile used for the carried payload.
    pub payload_profile: HashProfile,
    /// Identity of the exact carried payload bytes.
    pub payload_id: PayloadId,
    /// Exact length of the carried payload in bytes.
    pub payload_length: u64,
}

impl SidecarBody {
    fn validate(&self) -> Result<(), SnapshotError> {
        check_label("namespace_id", &self.namespace_id)?;
        check_label("tenant_scope", &self.tenant_scope)?;
        check_label("domain", &self.domain)?;
        check_label("context", &self.context)?;
        check_label("signer_id", &self.signer_id)?;
        check_label("key_id", &self.key_id)?;
        if self.generation == 0 && self.parent_snapshot_id.is_some() {
            return Err(SnapshotError::Field {
                field: "parent_snapshot_id",
                problem: "generation zero cannot have a parent",
            });
        }
        if self.generation > 0 && self.parent_snapshot_id.is_none() {
            return Err(SnapshotError::Field {
                field: "parent_snapshot_id",
                problem: "is required after generation zero",
            });
        }
        if self.payload_profile != self.payload_id.content_id().profile() {
            return Err(SnapshotError::Field {
                field: "payload_profile",
                problem: "does not match payload_id",
            });
        }
        if self.suite != SUITE_ED25519 {
            return Err(SnapshotError::Field {
                field: "suite",
                problem: "unsupported signature suite",
            });
        }
        Ok(())
    }

    /// The canonical deterministic-CBOR body bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        self.validate()?;
        Ok(encode_canonical(&self.to_value())?)
    }

    fn to_value(&self) -> Value {
        let mut fields = vec![
            ("v", Value::UInt(BODY_VERSION)),
            ("key", Value::Bytes(self.key.to_vec())),
            ("suite", text(&self.suite)),
            ("context", text(&self.context)),
            ("domain", text(&self.domain)),
            ("key_id", text(&self.key_id)),
            ("signer_id", text(&self.signer_id)),
            ("tenant_scope", text(&self.tenant_scope)),
            ("namespace_id", text(&self.namespace_id)),
            ("created_at", Value::UInt(self.created_at)),
            ("generation", Value::UInt(self.generation)),
            ("payload_id", payload_id_value(&self.payload_id)),
            ("payload_profile", text(self.payload_profile.token())),
            ("payload_length", Value::UInt(self.payload_length)),
            (
                "subject_content_id",
                content_id_value(&self.subject_content_id),
            ),
        ];
        if let Some(parent) = self.parent_snapshot_id {
            fields.push(("parent_snapshot_id", envelope_id_value(&parent)));
        }
        Value::Map(
            fields
                .into_iter()
                .map(|(name, value)| (text(name), value))
                .collect(),
        )
    }

    fn from_value(value: &Value, payload: &[u8]) -> Result<Self, SnapshotError> {
        let fields = Fields::new(
            value,
            "body",
            &[
                "v",
                "key",
                "suite",
                "context",
                "domain",
                "key_id",
                "signer_id",
                "tenant_scope",
                "namespace_id",
                "created_at",
                "generation",
                "payload_id",
                "payload_profile",
                "payload_length",
                "subject_content_id",
                "parent_snapshot_id",
            ],
        )?;
        let version = fields.uint("v")?;
        if version != BODY_VERSION {
            return Err(SnapshotError::Field {
                field: "v",
                problem: "unsupported body version",
            });
        }
        let payload_profile = profile_from(fields.required("payload_profile")?, "payload_profile")?;
        let encoded_payload_id = content_id_from(fields.required("payload_id")?, "payload_id")?;
        let payload_id = PayloadId::of(payload_profile, payload)?;
        if *payload_id.content_id() != encoded_payload_id {
            return Err(SnapshotError::Id(IdShapeError::DigestMismatch));
        }
        let body = Self {
            subject_content_id: content_id_from(
                fields.required("subject_content_id")?,
                "subject_content_id",
            )?,
            namespace_id: fields.text("namespace_id")?,
            tenant_scope: fields.text("tenant_scope")?,
            generation: fields.uint("generation")?,
            parent_snapshot_id: fields
                .optional("parent_snapshot_id")
                .map(|value| envelope_id_from(value, "parent_snapshot_id"))
                .transpose()?,
            created_at: fields.uint("created_at")?,
            domain: fields.text("domain")?,
            context: fields.text("context")?,
            signer_id: fields.text("signer_id")?,
            key_id: fields.text("key_id")?,
            key: fields.fixed_bytes("key")?,
            suite: fields.text("suite")?,
            payload_profile,
            payload_id,
            payload_length: fields.uint("payload_length")?,
        };
        body.validate()?;
        Ok(body)
    }
}

/// A complete signed sidecar snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarSnapshot {
    /// The signed snapshot body.
    pub body: SidecarBody,
    /// The complete metadata view, encoded by the caller.
    pub payload: Vec<u8>,
    /// Ed25519 signature over the canonical body preimage.
    pub signature: [u8; 64],
}

impl SidecarSnapshot {
    /// Signs a complete metadata snapshot.
    pub fn sign(
        body: SidecarBody,
        payload: Vec<u8>,
        key: &SigningKey,
    ) -> Result<Self, SnapshotError> {
        body.validate()?;
        if key.verifying_key().to_bytes() != body.key {
            return Err(SnapshotError::KeyMismatch);
        }
        check_payload(&body, &payload)?;
        let signature = key.sign(&signature_preimage(&body)?).to_bytes();
        Ok(Self {
            body,
            payload,
            signature,
        })
    }

    /// Canonical bytes for the complete signed envelope.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        self.body.validate()?;
        check_payload(&self.body, &self.payload)?;
        Ok(encode_canonical(&Value::Map(vec![
            (text("body"), self.body.to_value()),
            (text("payload"), Value::Bytes(self.payload.clone())),
            (text("signature"), Value::Bytes(self.signature.to_vec())),
        ]))?)
    }

    /// Decodes a canonical snapshot. Call [`Self::verify`] before trusting it.
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::new(&value, "envelope", &["body", "payload", "signature"])?;
        let signature = fields.bytes("signature")?;
        let payload = fields.bytes("payload")?;
        Ok(Self {
            body: SidecarBody::from_value(fields.required("body")?, &payload)?,
            payload,
            signature: <[u8; 64]>::try_from(signature.as_slice()).map_err(|_error| {
                SnapshotError::Field {
                    field: "signature",
                    problem: "has the wrong length",
                }
            })?,
        })
    }

    /// Computes the digest-derived ID of the complete signed envelope.
    pub fn id(&self) -> Result<SnapshotId, SnapshotError> {
        Ok(EnvelopeId::from_content_id(ContentId::of_bytes(
            SNAPSHOT_PROFILE,
            &self.canonical_bytes()?,
        )?))
    }

    /// Verifies the complete payload, body and detached signature.
    pub fn verify(&self) -> Result<(), SnapshotError> {
        self.body.validate()?;
        check_payload(&self.body, &self.payload)?;
        let key =
            VerifyingKey::from_bytes(&self.body.key).map_err(|_error| SnapshotError::BadKey)?;
        key.verify_strict(
            &signature_preimage(&self.body)?,
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_error| SnapshotError::BadSignature)
    }
}

fn signature_preimage(body: &SidecarBody) -> Result<Vec<u8>, SnapshotError> {
    let encoded = body.canonical_bytes()?;
    let domain = body.domain.as_bytes();
    let domain_len = u32::try_from(domain.len()).map_err(|_error| SnapshotError::Field {
        field: "domain",
        problem: "is too long",
    })?;
    let mut preimage = Vec::with_capacity(SIGNATURE_DOMAIN_PREFIX.len() + 4 + domain.len());
    preimage.extend_from_slice(SIGNATURE_DOMAIN_PREFIX);
    preimage.extend_from_slice(&domain_len.to_be_bytes());
    preimage.extend_from_slice(domain);
    preimage.extend_from_slice(&encoded);
    Ok(preimage)
}

fn check_payload(body: &SidecarBody, payload: &[u8]) -> Result<(), SnapshotError> {
    let actual = payload.len() as u64;
    if actual != body.payload_length {
        return Err(SnapshotError::PayloadLength {
            declared: body.payload_length,
            actual,
        });
    }
    body.payload_id.content_id().verify(payload)?;
    Ok(())
}

fn check_label(field: &'static str, value: &str) -> Result<(), SnapshotError> {
    if value.is_empty() {
        return Err(SnapshotError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if value.len() > MAX_LABEL_BYTES {
        return Err(SnapshotError::Field {
            field,
            problem: "is too long",
        });
    }
    Ok(())
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn content_id_value(id: &ContentId) -> Value {
    Value::Map(vec![
        (text("digest"), Value::Bytes(id.digest().to_vec())),
        (text("profile"), text(id.profile().token())),
    ])
}

fn payload_id_value(id: &PayloadId) -> Value {
    content_id_value(id.content_id())
}

fn envelope_id_value(id: &EnvelopeId) -> Value {
    content_id_value(id.content_id())
}

struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(
        value: &'a Value,
        record: &'static str,
        known: &[&'static str],
    ) -> Result<Self, SnapshotError> {
        let Value::Map(entries) = value else {
            return Err(SnapshotError::Field {
                field: record,
                problem: "must be a map",
            });
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(SnapshotError::Field {
                    field: record,
                    problem: "keys must be text",
                });
            };
            if !known.contains(&name.as_str()) {
                return Err(SnapshotError::Cbor(CborError::UnknownCriticalField {
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

    fn required(&self, field: &'static str) -> Result<&'a Value, SnapshotError> {
        self.optional(field).ok_or(SnapshotError::Field {
            field,
            problem: "is missing",
        })
    }

    fn text(&self, field: &'static str) -> Result<String, SnapshotError> {
        match self.required(field)? {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(SnapshotError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn uint(&self, field: &'static str) -> Result<u64, SnapshotError> {
        match self.required(field)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(SnapshotError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn bytes(&self, field: &'static str) -> Result<Vec<u8>, SnapshotError> {
        match self.required(field)? {
            Value::Bytes(value) => Ok(value.clone()),
            _ => Err(SnapshotError::Field {
                field,
                problem: "must be a byte string",
            }),
        }
    }

    fn fixed_bytes<const N: usize>(&self, field: &'static str) -> Result<[u8; N], SnapshotError> {
        let bytes = self.bytes(field)?;
        <[u8; N]>::try_from(bytes.as_slice()).map_err(|_error| SnapshotError::Field {
            field,
            problem: "has the wrong length",
        })
    }
}

fn profile_from(value: &Value, field: &'static str) -> Result<HashProfile, SnapshotError> {
    let Value::Text(token) = value else {
        return Err(SnapshotError::Field {
            field,
            problem: "must be text",
        });
    };
    HashProfile::from_token(token).ok_or_else(|| {
        SnapshotError::Id(IdShapeError::UnsupportedProfile {
            profile: token.clone(),
        })
    })
}

fn content_id_from(value: &Value, field: &'static str) -> Result<ContentId, SnapshotError> {
    let fields = Fields::new(value, field, &["digest", "profile"])?;
    let profile = profile_from(fields.required("profile")?, "profile")?;
    Ok(ContentId::new(profile, fields.fixed_bytes("digest")?))
}

fn envelope_id_from(value: &Value, field: &'static str) -> Result<EnvelopeId, SnapshotError> {
    Ok(EnvelopeId::from_content_id(content_id_from(value, field)?))
}
