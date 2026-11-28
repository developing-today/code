//! Complete signed metadata sidecar snapshots and the mutable head
//! (slice A9114-sidecar-snapshot, issue #10242; parent #9114; design
//! `part-a-addressing/versioned-design.md` section "#9114 — Complete
//! metadata sidecars and mutable head", A-9114.1–A-9114.3, statements
//! S3.11–S3.14 and S3.18).
//!
//! A sidecar snapshot is a complete metadata view within one explicit
//! scope — one subject, one namespace, one tenant scope — carried as
//! the payload of a signed envelope. It is never a delta, and it is
//! never input bytes for the subject's content ID. Storage form
//! (A-9114.2): immutable snapshots named `hash.metadata.<snapshot
//! digest>` plus one mutable `hash.metadata` head per scope; see
//! [`snapshot_file_name`] and [`HEAD_FILE_NAME`].
//!
//! Identities stay separated exactly as in the envelope-identities
//! slice (#10226, `id_core::signed_envelope`):
//!
//! - the payload ID commits the exact raw payload (the encoded
//!   complete view) under the body's explicit payload profile;
//! - the canonical [`SidecarBodyV1`] commits the payload ID, payload
//!   length, domain/context, signer, key and suite, the scoped
//!   subject, the generation and the parent snapshot — never the raw
//!   payload, the signature, or its own snapshot ID;
//! - [`SnapshotId`] (an [`EnvelopeId`]) hashes the complete canonical
//!   signed envelope, payload and signature included, so it is
//!   computed after signing and can never be embedded in bytes it
//!   hashes: no recursive self-ID.
//!
//! The mutable head advances by compare-and-swap on
//! `(generation, snapshot_id)`: the successor's generation is exactly
//! the head generation plus one and its parent is exactly the prior
//! head, so lost updates and ABA are detectable, never silent.
//!
//! The authenticated tenant/scope context is consumed, not defined,
//! here (#9083): namespace and tenant scope are committed in the
//! signed body, and a chain may not change them mid-chain.
//!
//! The crate-root export belongs to slice 9110-02; until that lands
//! the integration test includes this module by path.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use id_core::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use id_core::hash_profile::{ProfileDispatch, VerifyError};
use id_core::id_shape::{ContentId, HashProfile, IdShapeError};
use id_core::signed_envelope::{EnvelopeId, PayloadId, SIGNATURE_DOMAIN_PREFIX, SUITE_ED25519};

/// The body and head format version this module writes and reads.
pub const RECORD_VERSION: u64 = 1;

/// Signing domain of the mutable head record.
pub const HEAD_DOMAIN: &str = "id.sidecar-head/v1";

/// File name of the mutable head within a scope's sidecar directory.
pub const HEAD_FILE_NAME: &str = "hash.metadata";

/// Upper bound on namespace, tenant-scope, domain and signer text,
/// in bytes (mirrors the envelope-identities label bound).
pub const MAX_LABEL_BYTES: usize = 256;

/// Identity of a complete sidecar snapshot: the envelope ID of its
/// signed envelope (design A-9114.1: `SnapshotId = EnvelopeId`).
pub type SnapshotId = EnvelopeId;

/// The immutable storage file name of a snapshot:
/// `hash.metadata.<snapshot-digest>` (A-9114.2). The digest is the
/// signed-envelope ID, never a timestamp.
#[must_use]
pub fn snapshot_file_name(id: &SnapshotId) -> String {
    format!("{HEAD_FILE_NAME}.{}", hex::encode(id.content_id().digest()))
}

/// Why a sidecar snapshot, envelope or head was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarError {
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
    /// The record version is not [`RECORD_VERSION`].
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
    /// The head no longer points at the snapshot offered as current:
    /// a compare-and-swap on `(generation, snapshot_id)` failed.
    StaleHead,
    /// A generation is not the exact successor (or genesis) value.
    GenerationMismatch {
        /// The generation the operation required.
        expected: u64,
        /// The generation the record carries.
        actual: u64,
    },
    /// A snapshot's parent is not the prior head snapshot.
    ParentMismatch,
    /// A snapshot's subject is not the head's subject.
    SubjectMismatch,
    /// A snapshot's namespace or tenant scope differs from the
    /// chain's committed scope.
    ScopeMismatch,
}

impl SidecarError {
    /// A stable failure class.
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
            Self::StaleHead => "stale_head",
            Self::GenerationMismatch { .. } => "generation_mismatch",
            Self::ParentMismatch => "parent_mismatch",
            Self::SubjectMismatch => "subject_mismatch",
            Self::ScopeMismatch => "scope_mismatch",
        }
    }
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "sidecar encoding: {error}"),
            Self::Id(error) => write!(f, "sidecar identity: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported sidecar record version {version}")
            }
            Self::UnsupportedSuite(suite) => write!(f, "unsupported signature suite {suite:?}"),
            Self::PayloadLength { declared, actual } => {
                write!(
                    f,
                    "payload is {actual} bytes but the body commits {declared}"
                )
            }
            Self::BadKey => write!(f, "record key is not a valid Ed25519 public key"),
            Self::KeyMismatch => write!(f, "signing key does not match the body's key"),
            Self::BadSignature => write!(f, "signature does not verify"),
            Self::StaleHead => write!(f, "head no longer points at the offered snapshot"),
            Self::GenerationMismatch { expected, actual } => {
                write!(f, "generation must be {expected}, record carries {actual}")
            }
            Self::ParentMismatch => write!(f, "parent snapshot is not the prior head"),
            Self::SubjectMismatch => write!(f, "subject differs from the head's subject"),
            Self::ScopeMismatch => write!(f, "namespace or tenant scope differs mid-chain"),
        }
    }
}

impl std::error::Error for SidecarError {}

impl From<CborError> for SidecarError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for SidecarError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

impl From<VerifyError> for SidecarError {
    fn from(error: VerifyError) -> Self {
        match error {
            VerifyError::Rejected(error) => Self::Id(error),
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
    entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    Value::Map(entries.into_iter().map(|(k, v)| (text(k), v)).collect())
}

const fn check_label(field: &'static str, label: &str) -> Result<(), SidecarError> {
    if label.is_empty() {
        return Err(SidecarError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if label.len() > MAX_LABEL_BYTES {
        return Err(SidecarError::Field {
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
    ) -> Result<Self, SidecarError> {
        let Value::Map(entries) = value else {
            return Err(SidecarError::Field {
                field: record,
                problem: "must be a map",
            });
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(SidecarError::Field {
                    field: record,
                    problem: "keys must be text",
                });
            };
            if !known.contains(&name.as_str()) {
                // Every V1 field is critical.
                return Err(SidecarError::Cbor(CborError::UnknownCriticalField {
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

    fn required(&self, field: &'static str) -> Result<&'a Value, SidecarError> {
        self.optional(field).ok_or(SidecarError::Field {
            field,
            problem: "is missing",
        })
    }

    fn text(&self, field: &'static str) -> Result<String, SidecarError> {
        match self.required(field)? {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(SidecarError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn uint(&self, field: &'static str) -> Result<u64, SidecarError> {
        match self.required(field)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(SidecarError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn bytes(&self, field: &'static str) -> Result<Vec<u8>, SidecarError> {
        match self.required(field)? {
            Value::Bytes(value) => Ok(value.clone()),
            _ => Err(SidecarError::Field {
                field,
                problem: "must be a byte string",
            }),
        }
    }
}

fn fixed_bytes<const N: usize>(
    value: &Value,
    field: &'static str,
) -> Result<[u8; N], SidecarError> {
    match value {
        Value::Bytes(bytes) => {
            <[u8; N]>::try_from(bytes.as_slice()).map_err(|_error| SidecarError::Field {
                field,
                problem: "has the wrong length",
            })
        }
        _ => Err(SidecarError::Field {
            field,
            problem: "must be a byte string",
        }),
    }
}

fn content_id_from(value: &Value, field: &'static str) -> Result<ContentId, SidecarError> {
    let fields = Fields::new(value, field, &["digest", "profile"])?;
    let token = fields.text("profile")?;
    let profile = HashProfile::from_token(&token)
        .ok_or(IdShapeError::UnsupportedProfile { profile: token })?;
    let digest = fixed_bytes(fields.required("digest")?, field)?;
    Ok(ContentId::new(profile, digest))
}

/// The exact bytes a record signature covers, in the
/// envelope-identities framing: `SIGNATURE_DOMAIN_PREFIX ||
/// u32_be(len(domain)) || domain || canonical_cbor(record)`.
fn signature_preimage(domain: &str, canonical_record: &[u8]) -> Result<Vec<u8>, SidecarError> {
    let domain_len = u32::try_from(domain.len()).map_err(|_error| SidecarError::Field {
        field: "domain",
        problem: "is too long",
    })?;
    let mut preimage = Vec::with_capacity(
        SIGNATURE_DOMAIN_PREFIX.len() + 4 + domain.len() + canonical_record.len(),
    );
    preimage.extend_from_slice(SIGNATURE_DOMAIN_PREFIX);
    preimage.extend_from_slice(&domain_len.to_be_bytes());
    preimage.extend_from_slice(domain.as_bytes());
    preimage.extend_from_slice(canonical_record);
    Ok(preimage)
}

fn check_suite(suite: &str) -> Result<(), SidecarError> {
    if suite == SUITE_ED25519 {
        Ok(())
    } else {
        Err(SidecarError::UnsupportedSuite(suite.to_owned()))
    }
}

/// Encodes a complete metadata view as a snapshot payload: the
/// canonical CBOR of the whole map. A snapshot payload is always the
/// complete view for its scope, never a delta.
///
/// # Errors
///
/// A canonical-encoding error.
pub fn encode_view(view: &BTreeMap<String, Value>) -> Result<Vec<u8>, SidecarError> {
    let entries = view
        .iter()
        .map(|(key, value)| (text(key), value.clone()))
        .collect();
    Ok(encode_canonical(&Value::Map(entries))?)
}

/// Decodes a snapshot payload back into the complete metadata view.
///
/// # Errors
///
/// [`SidecarError`] for non-canonical bytes, a non-map payload or a
/// non-text view key.
pub fn decode_view(payload: &[u8]) -> Result<BTreeMap<String, Value>, SidecarError> {
    let Value::Map(entries) = decode_canonical(payload)? else {
        return Err(SidecarError::Field {
            field: "payload",
            problem: "must be a map",
        });
    };
    let mut view = BTreeMap::new();
    for (key, value) in entries {
        let Value::Text(name) = key else {
            return Err(SidecarError::Field {
                field: "payload",
                problem: "view keys must be text",
            });
        };
        view.insert(name, value);
    }
    Ok(view)
}

/// The canonical signed body of a sidecar snapshot (design
/// `SidecarBodyV1`). Holds references and lengths, never the payload,
/// the signature or its own snapshot ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarBodyV1 {
    /// The subject this metadata describes.
    pub subject_content_id: ContentId,
    /// The namespace the snapshot is complete within.
    pub namespace_id: String,
    /// The tenant scope the snapshot is complete within, consumed
    /// from the authenticated context contract (#9083).
    pub tenant_scope: String,
    /// Position in the scope's snapshot chain; genesis is 0 and each
    /// successor increments exactly once.
    pub generation: u64,
    /// The prior head snapshot; absent at genesis, never null.
    pub parent_snapshot_id: Option<SnapshotId>,
    /// Advisory creation time (unix milliseconds). Never an identity
    /// input: snapshot names are digest-derived (A-9114.2).
    pub created_at: u64,
    /// Signing domain and context.
    pub domain_context: String,
    /// Who signed, as the signer names itself.
    pub signer_id: String,
    /// The signer's Ed25519 public key (the design's `key_id`,
    /// carried in full so the record verifies standalone, as in the
    /// envelope-identities `Body`).
    pub key: [u8; 32],
    /// The signature suite; `ed25519` in V1.
    pub suite: String,
    /// The explicit profile the payload ID is computed under.
    pub payload_profile: HashProfile,
    /// Identity of the raw payload: the [`PayloadId`]'s content ID.
    pub payload_id: ContentId,
    /// Exact length of the raw payload in bytes.
    pub payload_length: u64,
}

impl SidecarBodyV1 {
    /// Builds a body committing `payload` (the encoded complete
    /// view): payload ID and length are derived from the exact bytes.
    ///
    /// # Errors
    ///
    /// The payload profile's own rejection of the payload bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn for_payload(
        subject_content_id: ContentId,
        namespace_id: String,
        tenant_scope: String,
        generation: u64,
        parent_snapshot_id: Option<SnapshotId>,
        created_at: u64,
        domain_context: String,
        signer_id: String,
        key: [u8; 32],
        payload_profile: HashProfile,
        payload: &[u8],
    ) -> Result<Self, SidecarError> {
        Ok(Self {
            subject_content_id,
            namespace_id,
            tenant_scope,
            generation,
            parent_snapshot_id,
            created_at,
            domain_context,
            signer_id,
            key,
            suite: SUITE_ED25519.to_owned(),
            payload_profile,
            payload_id: *PayloadId::of(payload_profile, payload)?.content_id(),
            payload_length: payload.len() as u64,
        })
    }

    /// The body as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`SidecarError::Field`] when a label is empty or over
    /// [`MAX_LABEL_BYTES`].
    pub fn to_value(&self) -> Result<Value, SidecarError> {
        check_label("namespace_id", &self.namespace_id)?;
        check_label("tenant_scope", &self.tenant_scope)?;
        check_label("domain_context", &self.domain_context)?;
        check_label("signer_id", &self.signer_id)?;
        let mut entries = vec![
            ("v", Value::UInt(RECORD_VERSION)),
            (
                "subject_content_id",
                content_id_value(&self.subject_content_id),
            ),
            ("namespace_id", text(&self.namespace_id)),
            ("tenant_scope", text(&self.tenant_scope)),
            ("generation", Value::UInt(self.generation)),
            ("created_at", Value::UInt(self.created_at)),
            ("domain_context", text(&self.domain_context)),
            ("signer_id", text(&self.signer_id)),
            ("key", Value::Bytes(self.key.to_vec())),
            ("suite", text(&self.suite)),
            ("payload_profile", text(self.payload_profile.token())),
            ("payload_id", content_id_value(&self.payload_id)),
            ("payload_length", Value::UInt(self.payload_length)),
        ];
        if let Some(parent) = &self.parent_snapshot_id {
            entries.push(("parent_snapshot_id", content_id_value(parent.content_id())));
        }
        Ok(map(entries))
    }

    /// `canonical_cbor(SidecarBodyV1)`.
    ///
    /// # Errors
    ///
    /// As [`SidecarBodyV1::to_value`].
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SidecarError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a body from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// [`SidecarError`] for an unknown field, a missing or mistyped
    /// field, an unknown version or an unsupported ID profile token.
    pub fn from_value(value: &Value) -> Result<Self, SidecarError> {
        let fields = Fields::new(
            value,
            "body",
            &[
                "v",
                "subject_content_id",
                "namespace_id",
                "tenant_scope",
                "generation",
                "parent_snapshot_id",
                "created_at",
                "domain_context",
                "signer_id",
                "key",
                "suite",
                "payload_profile",
                "payload_id",
                "payload_length",
            ],
        )?;
        let version = fields.uint("v")?;
        if version != RECORD_VERSION {
            return Err(SidecarError::UnsupportedVersion(version));
        }
        let namespace_id = fields.text("namespace_id")?;
        let tenant_scope = fields.text("tenant_scope")?;
        let domain_context = fields.text("domain_context")?;
        let signer_id = fields.text("signer_id")?;
        check_label("namespace_id", &namespace_id)?;
        check_label("tenant_scope", &tenant_scope)?;
        check_label("domain_context", &domain_context)?;
        check_label("signer_id", &signer_id)?;
        let token = fields.text("payload_profile")?;
        let payload_profile = HashProfile::from_token(&token)
            .ok_or(IdShapeError::UnsupportedProfile { profile: token })?;
        Ok(Self {
            subject_content_id: content_id_from(
                fields.required("subject_content_id")?,
                "subject_content_id",
            )?,
            namespace_id,
            tenant_scope,
            generation: fields.uint("generation")?,
            parent_snapshot_id: fields
                .optional("parent_snapshot_id")
                .map(|v| content_id_from(v, "parent_snapshot_id").map(EnvelopeId::from_content_id))
                .transpose()?,
            created_at: fields.uint("created_at")?,
            domain_context,
            signer_id,
            key: fixed_bytes(fields.required("key")?, "key")?,
            suite: fields.text("suite")?,
            payload_profile,
            payload_id: content_id_from(fields.required("payload_id")?, "payload_id")?,
            payload_length: fields.uint("payload_length")?,
        })
    }
}

/// A complete signed sidecar snapshot (design `SidecarEnvelopeV1`):
/// the body, the raw payload it commits (the encoded complete view),
/// and the detached signature over the body's preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarEnvelopeV1 {
    /// The signed body.
    pub body: SidecarBodyV1,
    /// The raw payload bytes, exactly as committed by the body.
    pub raw_payload_bytes: Vec<u8>,
    /// Ed25519 signature over the body's signature preimage.
    pub signature: [u8; 64],
}

impl SidecarEnvelopeV1 {
    /// Signs `body` with `key` and pairs it with `payload`.
    ///
    /// # Errors
    ///
    /// [`SidecarError::KeyMismatch`] when `key` is not the body's
    /// key, [`SidecarError::UnsupportedSuite`] for a non-Ed25519
    /// suite, [`SidecarError::PayloadLength`] or a digest mismatch
    /// when `payload` is not what the body commits, or an encoding
    /// error.
    pub fn sign(
        body: SidecarBodyV1,
        payload: Vec<u8>,
        key: &SigningKey,
    ) -> Result<Self, SidecarError> {
        if key.verifying_key().to_bytes() != body.key {
            return Err(SidecarError::KeyMismatch);
        }
        check_suite(&body.suite)?;
        check_payload(&body, &payload)?;
        let signature = key
            .sign(&signature_preimage(
                &body.domain_context,
                &body.canonical_bytes()?,
            )?)
            .to_bytes();
        Ok(Self {
            body,
            raw_payload_bytes: payload,
            signature,
        })
    }

    /// The envelope as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// As [`SidecarBodyV1::to_value`].
    pub fn to_value(&self) -> Result<Value, SidecarError> {
        Ok(map(vec![
            ("body", self.body.to_value()?),
            ("payload", Value::Bytes(self.raw_payload_bytes.clone())),
            ("signature", Value::Bytes(self.signature.to_vec())),
        ]))
    }

    /// `canonical_cbor(SidecarEnvelopeV1)`: the bytes the snapshot
    /// ID hashes.
    ///
    /// # Errors
    ///
    /// As [`SidecarBodyV1::to_value`].
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SidecarError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes a canonical envelope. Decoding checks structure only;
    /// call [`SidecarEnvelopeV1::verify`] before trusting it.
    ///
    /// # Errors
    ///
    /// [`SidecarError`] for non-canonical bytes or any field problem.
    pub fn decode(bytes: &[u8]) -> Result<Self, SidecarError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::new(&value, "envelope", &["body", "payload", "signature"])?;
        let signature = fields.bytes("signature")?;
        Ok(Self {
            body: SidecarBodyV1::from_value(fields.required("body")?)?,
            raw_payload_bytes: fields.bytes("payload")?,
            signature: <[u8; 64]>::try_from(signature.as_slice()).map_err(|_error| {
                SidecarError::Field {
                    field: "signature",
                    problem: "has the wrong length",
                }
            })?,
        })
    }

    /// The snapshot's identity: the envelope ID over its complete
    /// canonical bytes (body, payload and signature) under the
    /// body's payload profile. Computed after signing and never
    /// written into any bytes it hashes.
    ///
    /// # Errors
    ///
    /// An encoding error, or the profile's own rejection.
    pub fn snapshot_id(&self) -> Result<SnapshotId, SidecarError> {
        Ok(EnvelopeId::from_content_id(ContentId::of_bytes(
            self.body.payload_profile,
            &self.canonical_bytes()?,
        )?))
    }

    /// The complete metadata view this snapshot carries.
    ///
    /// # Errors
    ///
    /// As [`decode_view`].
    pub fn view(&self) -> Result<BTreeMap<String, Value>, SidecarError> {
        decode_view(&self.raw_payload_bytes)
    }

    /// Verifies the payload against the body's payload ID and
    /// length, and the signature against the body's key.
    ///
    /// # Errors
    ///
    /// [`SidecarError`], classed by [`SidecarError::class`].
    pub fn verify(&self, dispatch: &ProfileDispatch) -> Result<(), SidecarError> {
        check_suite(&self.body.suite)?;
        let actual = self.raw_payload_bytes.len() as u64;
        if actual != self.body.payload_length {
            return Err(SidecarError::PayloadLength {
                declared: self.body.payload_length,
                actual,
            });
        }
        dispatch.verify_bytes(&self.body.payload_id, &self.raw_payload_bytes)?;
        let key =
            VerifyingKey::from_bytes(&self.body.key).map_err(|_error| SidecarError::BadKey)?;
        key.verify_strict(
            &signature_preimage(&self.body.domain_context, &self.body.canonical_bytes()?)?,
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_error| SidecarError::BadSignature)
    }
}

fn check_payload(body: &SidecarBodyV1, payload: &[u8]) -> Result<(), SidecarError> {
    let actual = payload.len() as u64;
    if actual != body.payload_length {
        return Err(SidecarError::PayloadLength {
            declared: body.payload_length,
            actual,
        });
    }
    let committed = PayloadId::of(body.payload_profile, payload)?;
    if committed.content_id() != &body.payload_id {
        return Err(SidecarError::Id(IdShapeError::DigestMismatch));
    }
    Ok(())
}

/// The mutable head of one scope's sidecar chain (design
/// `SidecarHeadV1`), itself a signed record. Storage form is the
/// single mutable `hash.metadata` file; the record advances only by
/// the compare-and-swap in [`SidecarHeadV1::advance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarHeadV1 {
    /// The subject whose chain this head points into.
    pub subject_content_id: ContentId,
    /// The generation of the snapshot the head points at.
    pub generation: u64,
    /// The snapshot the head points at.
    pub snapshot_id: SnapshotId,
    /// Who signed this head, as the signer names itself.
    pub signer_id: String,
    /// The head signer's Ed25519 public key.
    pub key: [u8; 32],
    /// Ed25519 signature over the head's unsigned canonical form
    /// under [`HEAD_DOMAIN`].
    pub signature: [u8; 64],
}

impl SidecarHeadV1 {
    fn unsigned_value(
        subject_content_id: &ContentId,
        generation: u64,
        snapshot_id: &SnapshotId,
        signer_id: &str,
        key: &[u8; 32],
    ) -> Value {
        map(vec![
            ("v", Value::UInt(RECORD_VERSION)),
            ("subject_content_id", content_id_value(subject_content_id)),
            ("generation", Value::UInt(generation)),
            ("snapshot_id", content_id_value(snapshot_id.content_id())),
            ("signer_id", text(signer_id)),
            ("key", Value::Bytes(key.to_vec())),
        ])
    }

    fn sign_fields(
        subject_content_id: ContentId,
        generation: u64,
        snapshot_id: SnapshotId,
        signer_id: String,
        key: &SigningKey,
    ) -> Result<Self, SidecarError> {
        let key_bytes = key.verifying_key().to_bytes();
        let unsigned = Self::unsigned_value(
            &subject_content_id,
            generation,
            &snapshot_id,
            &signer_id,
            &key_bytes,
        );
        let canonical = encode_canonical(&unsigned)?;
        let signature = key
            .sign(&signature_preimage(HEAD_DOMAIN, &canonical)?)
            .to_bytes();
        Ok(Self {
            subject_content_id,
            generation,
            snapshot_id,
            signer_id,
            key: key_bytes,
            signature,
        })
    }

    /// Creates the genesis head pointing at a generation-0 snapshot
    /// with no parent.
    ///
    /// # Errors
    ///
    /// [`SidecarError`] when the snapshot does not verify, is not
    /// generation 0 ([`SidecarError::GenerationMismatch`]) or names
    /// a parent ([`SidecarError::ParentMismatch`]).
    pub fn genesis(snapshot: &SidecarEnvelopeV1, key: &SigningKey) -> Result<Self, SidecarError> {
        snapshot.verify(&ProfileDispatch::default())?;
        if snapshot.body.generation != 0 {
            return Err(SidecarError::GenerationMismatch {
                expected: 0,
                actual: snapshot.body.generation,
            });
        }
        if snapshot.body.parent_snapshot_id.is_some() {
            return Err(SidecarError::ParentMismatch);
        }
        Self::sign_fields(
            snapshot.body.subject_content_id,
            0,
            snapshot.snapshot_id()?,
            snapshot.body.signer_id.clone(),
            key,
        )
    }

    /// Advances the head from `current` to `next` by compare-and-swap
    /// on `(generation, snapshot_id)`: the head must still point at
    /// `current`, `next` must be in the same subject, namespace and
    /// tenant scope, its generation must be exactly one more, and its
    /// parent must be exactly the prior head snapshot.
    ///
    /// # Errors
    ///
    /// [`SidecarError`], classed by [`SidecarError::class`]:
    /// `stale_head`, `subject_mismatch`, `scope_mismatch`,
    /// `generation_mismatch`, `parent_mismatch`, or the verification
    /// failure of either record.
    pub fn advance(
        &self,
        current: &SidecarEnvelopeV1,
        next: &SidecarEnvelopeV1,
        key: &SigningKey,
    ) -> Result<Self, SidecarError> {
        self.verify()?;
        if current.snapshot_id()? != self.snapshot_id || current.body.generation != self.generation
        {
            return Err(SidecarError::StaleHead);
        }
        next.verify(&ProfileDispatch::default())?;
        if next.body.subject_content_id != self.subject_content_id {
            return Err(SidecarError::SubjectMismatch);
        }
        if next.body.namespace_id != current.body.namespace_id
            || next.body.tenant_scope != current.body.tenant_scope
        {
            return Err(SidecarError::ScopeMismatch);
        }
        let expected = self.generation + 1;
        if next.body.generation != expected {
            return Err(SidecarError::GenerationMismatch {
                expected,
                actual: next.body.generation,
            });
        }
        if next.body.parent_snapshot_id != Some(self.snapshot_id) {
            return Err(SidecarError::ParentMismatch);
        }
        Self::sign_fields(
            self.subject_content_id,
            next.body.generation,
            next.snapshot_id()?,
            next.body.signer_id.clone(),
            key,
        )
    }

    /// The head as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`SidecarError::Field`] when the signer label is empty or over
    /// [`MAX_LABEL_BYTES`].
    pub fn to_value(&self) -> Result<Value, SidecarError> {
        check_label("signer_id", &self.signer_id)?;
        let Value::Map(mut entries) = Self::unsigned_value(
            &self.subject_content_id,
            self.generation,
            &self.snapshot_id,
            &self.signer_id,
            &self.key,
        ) else {
            return Err(SidecarError::Field {
                field: "head",
                problem: "must be a map",
            });
        };
        entries.push((text("signature"), Value::Bytes(self.signature.to_vec())));
        Ok(Value::Map(entries))
    }

    /// `canonical_cbor(SidecarHeadV1)`.
    ///
    /// # Errors
    ///
    /// As [`SidecarHeadV1::to_value`].
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SidecarError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes a canonical head. Decoding checks structure only;
    /// call [`SidecarHeadV1::verify`] before trusting it.
    ///
    /// # Errors
    ///
    /// [`SidecarError`] for non-canonical bytes or any field problem.
    pub fn decode(bytes: &[u8]) -> Result<Self, SidecarError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::new(
            &value,
            "head",
            &[
                "v",
                "subject_content_id",
                "generation",
                "snapshot_id",
                "signer_id",
                "key",
                "signature",
            ],
        )?;
        let version = fields.uint("v")?;
        if version != RECORD_VERSION {
            return Err(SidecarError::UnsupportedVersion(version));
        }
        let signer_id = fields.text("signer_id")?;
        check_label("signer_id", &signer_id)?;
        let signature = fields.bytes("signature")?;
        Ok(Self {
            subject_content_id: content_id_from(
                fields.required("subject_content_id")?,
                "subject_content_id",
            )?,
            generation: fields.uint("generation")?,
            snapshot_id: EnvelopeId::from_content_id(content_id_from(
                fields.required("snapshot_id")?,
                "snapshot_id",
            )?),
            signer_id,
            key: fixed_bytes(fields.required("key")?, "key")?,
            signature: <[u8; 64]>::try_from(signature.as_slice()).map_err(|_error| {
                SidecarError::Field {
                    field: "signature",
                    problem: "has the wrong length",
                }
            })?,
        })
    }

    /// Verifies the head's signature against its committed key.
    ///
    /// # Errors
    ///
    /// [`SidecarError::BadKey`] or [`SidecarError::BadSignature`].
    pub fn verify(&self) -> Result<(), SidecarError> {
        let unsigned = Self::unsigned_value(
            &self.subject_content_id,
            self.generation,
            &self.snapshot_id,
            &self.signer_id,
            &self.key,
        );
        let canonical = encode_canonical(&unsigned)?;
        let key = VerifyingKey::from_bytes(&self.key).map_err(|_error| SidecarError::BadKey)?;
        key.verify_strict(
            &signature_preimage(HEAD_DOMAIN, &canonical)?,
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_error| SidecarError::BadSignature)
    }
}
