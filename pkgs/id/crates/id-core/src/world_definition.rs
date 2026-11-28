//! Signer-local signed coordinate-world definitions (slice A9124-world-definition,
//! issue #10278; design A-9124.1–A-9124.3).
//!
//! A definition is an immutable, versioned commitment to an array-coordinate
//! world and its content-addressed root.  Signatures establish what one
//! signer says about one definition; this module deliberately does not turn a
//! valid signature into global trust or server authority.
//!
//! The crate-root export belongs to the separate crate-wiring slice.  The
//! module therefore uses the same `crate::` leaf paths as the other extracted
//! ID slices and can be included by path by its integration test.

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile, IdShapeError};

/// Domain separation for the world-definition signature preimage.
pub const WORLD_DEFINITION_DOMAIN: &str = "id-world-definition-v1";

/// The only record version currently admitted.
pub const RECORD_VERSION: u64 = 1;

const SIGNATURE_PREFIX: &[u8] = b"id/world-definition/v1\0";
const MAX_LABEL_BYTES: usize = 256;
const RECORD_FIELDS: &[&str] = &[
    "v",
    "domain",
    "world_id",
    "signer_id",
    "signer_key_id",
    "signer_key",
    "core",
    "coordinate_profile",
    "root_ref",
    "presentation_profile",
    "version",
    "parent_definition",
    "issued_at",
    "valid_from",
    "valid_until",
    "key_history_ref",
    "signature",
];
const CONTENT_ID_FIELDS: &[&str] = &["digest", "profile"];

/// The two address cores known to the ID substrate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CoreKind {
    /// Immutable content-addressed objects.
    Content,
    /// Mutable ordered array-coordinate addresses.
    Array,
}

impl CoreKind {
    /// The canonical record token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Array => "array",
        }
    }

    fn from_token(token: &str) -> Result<Self, WorldDefinitionError> {
        match token {
            "content" => Ok(Self::Content),
            "array" => Ok(Self::Array),
            other => Err(WorldDefinitionError::UnsupportedCore {
                core: other.to_owned(),
            }),
        }
    }
}

/// Why a world definition was rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorldDefinitionError {
    /// Canonical CBOR failed.
    Cbor(CborError),
    /// A referenced ID or profile failed validation.
    Id(IdShapeError),
    /// A field is absent or malformed.
    Field {
        /// Field name.
        field: &'static str,
        /// Stable reason.
        problem: &'static str,
    },
    /// A future record version was supplied.
    UnsupportedVersion(u64),
    /// Only the array core is valid for coordinate worlds.
    UnsupportedCore {
        /// Core token found in the record.
        core: String,
    },
    /// A version after V1 omitted its predecessor.
    MissingParent {
        /// Version that required a parent.
        version: u64,
    },
    /// The signed bytes do not verify under the carried key.
    BadSignature,
    /// The carried public key is not a valid Ed25519 key.
    BadKey,
    /// A local policy denied trust; this is separate from signature validity.
    TrustDenied,
}

impl WorldDefinitionError {
    /// Stable error class for protocol adapters and tests.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cbor(_) | Self::Id(_) | Self::Field { .. } => "malformed",
            Self::UnsupportedVersion(_) => "unsupported_version",
            Self::UnsupportedCore { .. } => "unsupported_core",
            Self::MissingParent { .. } => "missing_parent",
            Self::BadSignature => "bad_signature",
            Self::BadKey => "bad_key",
            Self::TrustDenied => "trust_denied",
        }
    }
}

impl fmt::Display for WorldDefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "world definition encoding: {error}"),
            Self::Id(error) => write!(f, "world definition ID: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "world definition version {version} is unsupported")
            }
            Self::UnsupportedCore { core } => {
                write!(f, "world definition core {core:?} is not array")
            }
            Self::MissingParent { version } => {
                write!(f, "world definition version {version} requires a parent")
            }
            Self::BadSignature => f.write_str("world definition signature is invalid"),
            Self::BadKey => f.write_str("world definition signer key is invalid"),
            Self::TrustDenied => f.write_str("world definition is not trusted by this policy"),
        }
    }
}

impl std::error::Error for WorldDefinitionError {}

impl From<CborError> for WorldDefinitionError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for WorldDefinitionError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

/// Unsigned, versioned world metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldDefinition {
    /// Stable identity of this world.
    pub world_id: ContentId,
    /// Signer-local identity label.
    pub signer_id: String,
    /// Signer key-history identity label.
    pub signer_key_id: String,
    /// Address core; coordinate worlds must use [`CoreKind::Array`].
    pub core: CoreKind,
    /// Coordinate encoding/profile name.
    pub coordinate_profile: String,
    /// Immutable root or manifest reference.
    pub root_ref: ContentId,
    /// Client-side presentation profile name.
    pub presentation_profile: String,
    /// Monotonic definition version, starting at one.
    pub version: u64,
    /// Signed predecessor for a later definition version.
    pub parent_definition: Option<ContentId>,
    /// Issuance timestamp in the declared ID time unit.
    pub issued_at: u64,
    /// Optional validity lower bound.
    pub valid_from: Option<u64>,
    /// Optional validity upper bound.
    pub valid_until: Option<u64>,
    /// Optional signed key-history reference.
    pub key_history_ref: Option<ContentId>,
}

impl WorldDefinition {
    fn validate(&self) -> Result<(), WorldDefinitionError> {
        if self.core != CoreKind::Array {
            return Err(WorldDefinitionError::UnsupportedCore {
                core: self.core.token().to_owned(),
            });
        }
        if self.version == 0 {
            return Err(WorldDefinitionError::Field {
                field: "version",
                problem: "must be positive",
            });
        }
        if self.version > 1 && self.parent_definition.is_none() {
            return Err(WorldDefinitionError::MissingParent {
                version: self.version,
            });
        }
        if self.issued_at == 0 {
            return Err(WorldDefinitionError::Field {
                field: "issued_at",
                problem: "must be positive",
            });
        }
        for (field, value) in [
            ("signer_id", self.signer_id.as_str()),
            ("signer_key_id", self.signer_key_id.as_str()),
            ("coordinate_profile", self.coordinate_profile.as_str()),
            ("presentation_profile", self.presentation_profile.as_str()),
        ] {
            if value.is_empty() {
                return Err(WorldDefinitionError::Field {
                    field,
                    problem: "must not be empty",
                });
            }
            if value.len() > MAX_LABEL_BYTES {
                return Err(WorldDefinitionError::Field {
                    field,
                    problem: "is too long",
                });
            }
        }
        if let (Some(from), Some(until)) = (self.valid_from, self.valid_until)
            && until < from
        {
            return Err(WorldDefinitionError::Field {
                field: "valid_until",
                problem: "must not precede valid_from",
            });
        }
        Ok(())
    }
}

/// A complete signed world definition, including the verification key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedWorldDefinition {
    /// The signed metadata.
    pub definition: WorldDefinition,
    /// Public key used for this definition's signature.
    pub signer_key: [u8; 32],
    /// Domain-separated signature preimage result.
    pub signature: [u8; 64],
    domain: String,
}

impl SignedWorldDefinition {
    /// Sign a new definition with the signer's Ed25519 key.
    pub fn sign(
        definition: WorldDefinition,
        key: &SigningKey,
    ) -> Result<Self, WorldDefinitionError> {
        definition.validate()?;
        let signer_key = key.verifying_key().to_bytes();
        let domain = WORLD_DEFINITION_DOMAIN.to_owned();
        let preimage = signature_preimage(&definition, &signer_key, &domain)?;
        let signature = key.sign(&preimage).to_bytes();
        Ok(Self {
            definition,
            signer_key,
            signature,
            domain,
        })
    }

    /// The domain string carried by this signed record.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Verify structure, signer key binding and the exact signature bytes.
    pub fn verify(&self) -> Result<(), WorldDefinitionError> {
        self.definition.validate()?;
        if self.domain != WORLD_DEFINITION_DOMAIN {
            return Err(WorldDefinitionError::BadSignature);
        }
        let key = VerifyingKey::from_bytes(&self.signer_key)
            .map_err(|_error| WorldDefinitionError::BadKey)?;
        let signature = Signature::from_bytes(&self.signature);
        let preimage = signature_preimage(&self.definition, &self.signer_key, &self.domain)?;
        key.verify(&preimage, &signature)
            .map_err(|_error| WorldDefinitionError::BadSignature)
    }

    /// A valid signer-local signature does not establish global trust.
    #[must_use]
    pub const fn trusts_globally(&self) -> bool {
        false
    }

    /// Apply an already-decided local trust policy to the verified record.
    pub fn trust_policy_result(&self, trusted: bool) -> Result<(), WorldDefinitionError> {
        self.verify()?;
        if trusted {
            Ok(())
        } else {
            Err(WorldDefinitionError::TrustDenied)
        }
    }

    /// Encode the complete signed record as canonical CBOR.
    pub fn encode(&self) -> Result<Vec<u8>, WorldDefinitionError> {
        self.definition.validate()?;
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decode a canonical signed record. Call [`Self::verify`] before trust.
    pub fn decode(bytes: &[u8]) -> Result<Self, WorldDefinitionError> {
        let value = decode_canonical(bytes)?;
        Self::from_value(&value)
    }

    fn to_value(&self) -> Result<Value, WorldDefinitionError> {
        let definition = &self.definition;
        let mut entries = vec![
            ("v", Value::UInt(RECORD_VERSION)),
            ("domain", text(&self.domain)),
            ("world_id", content_id_value(&definition.world_id)),
            ("signer_id", text(&definition.signer_id)),
            ("signer_key_id", text(&definition.signer_key_id)),
            ("signer_key", Value::Bytes(self.signer_key.to_vec())),
            ("core", text(definition.core.token())),
            ("coordinate_profile", text(&definition.coordinate_profile)),
            ("root_ref", content_id_value(&definition.root_ref)),
            (
                "presentation_profile",
                text(&definition.presentation_profile),
            ),
            ("version", Value::UInt(definition.version)),
            ("issued_at", Value::UInt(definition.issued_at)),
            ("signature", Value::Bytes(self.signature.to_vec())),
        ];
        if let Some(parent) = &definition.parent_definition {
            entries.push(("parent_definition", content_id_value(parent)));
        }
        if let Some(valid_from) = definition.valid_from {
            entries.push(("valid_from", Value::UInt(valid_from)));
        }
        if let Some(valid_until) = definition.valid_until {
            entries.push(("valid_until", Value::UInt(valid_until)));
        }
        if let Some(key_history_ref) = &definition.key_history_ref {
            entries.push(("key_history_ref", content_id_value(key_history_ref)));
        }
        Ok(map(entries))
    }

    fn from_value(value: &Value) -> Result<Self, WorldDefinitionError> {
        let fields = Fields::new(value, RECORD_FIELDS)?;
        let version = fields.uint("v")?;
        if version != RECORD_VERSION {
            return Err(WorldDefinitionError::UnsupportedVersion(version));
        }
        let definition = WorldDefinition {
            world_id: content_id_from(fields.required("world_id")?, "world_id")?,
            signer_id: fields.text("signer_id")?,
            signer_key_id: fields.text("signer_key_id")?,
            core: CoreKind::from_token(&fields.text("core")?)?,
            coordinate_profile: fields.text("coordinate_profile")?,
            root_ref: content_id_from(fields.required("root_ref")?, "root_ref")?,
            presentation_profile: fields.text("presentation_profile")?,
            version: fields.uint("version")?,
            parent_definition: fields
                .optional("parent_definition")
                .map(|value| content_id_from(value, "parent_definition"))
                .transpose()?,
            issued_at: fields.uint("issued_at")?,
            valid_from: fields
                .optional("valid_from")
                .map(|v| fields.uint_value(v, "valid_from"))
                .transpose()?,
            valid_until: fields
                .optional("valid_until")
                .map(|v| fields.uint_value(v, "valid_until"))
                .transpose()?,
            key_history_ref: fields
                .optional("key_history_ref")
                .map(|value| content_id_from(value, "key_history_ref"))
                .transpose()?,
        };
        definition.validate()?;
        let domain = fields.text("domain")?;
        let signer_key = fields.fixed_bytes::<32>("signer_key")?;
        let signature = fields.fixed_bytes::<64>("signature")?;
        Ok(Self {
            definition,
            signer_key,
            signature,
            domain,
        })
    }

    #[cfg(test)]
    pub fn set_domain_for_test(&mut self, domain: &str) {
        self.domain = domain.to_owned();
    }
}

fn signature_preimage(
    definition: &WorldDefinition,
    signer_key: &[u8; 32],
    domain: &str,
) -> Result<Vec<u8>, WorldDefinitionError> {
    let body = map_definition(definition)?;
    let encoded = encode_canonical(&body)?;
    let domain_bytes = domain.as_bytes();
    let domain_len =
        u32::try_from(domain_bytes.len()).map_err(|_error| WorldDefinitionError::Field {
            field: "domain",
            problem: "is too long",
        })?;
    let mut out = Vec::with_capacity(
        SIGNATURE_PREFIX.len() + 4 + domain_bytes.len() + signer_key.len() + encoded.len(),
    );
    out.extend_from_slice(SIGNATURE_PREFIX);
    out.extend_from_slice(&domain_len.to_be_bytes());
    out.extend_from_slice(domain_bytes);
    out.extend_from_slice(signer_key);
    out.extend_from_slice(&encoded);
    Ok(out)
}

fn map_definition(definition: &WorldDefinition) -> Result<Value, WorldDefinitionError> {
    let mut entries = vec![
        ("world_id", content_id_value(&definition.world_id)),
        ("signer_id", text(&definition.signer_id)),
        ("signer_key_id", text(&definition.signer_key_id)),
        ("core", text(definition.core.token())),
        ("coordinate_profile", text(&definition.coordinate_profile)),
        ("root_ref", content_id_value(&definition.root_ref)),
        (
            "presentation_profile",
            text(&definition.presentation_profile),
        ),
        ("version", Value::UInt(definition.version)),
        ("issued_at", Value::UInt(definition.issued_at)),
    ];
    if let Some(parent) = &definition.parent_definition {
        entries.push(("parent_definition", content_id_value(parent)));
    }
    if let Some(valid_from) = definition.valid_from {
        entries.push(("valid_from", Value::UInt(valid_from)));
    }
    if let Some(valid_until) = definition.valid_until {
        entries.push(("valid_until", Value::UInt(valid_until)));
    }
    if let Some(key_history_ref) = &definition.key_history_ref {
        entries.push(("key_history_ref", content_id_value(key_history_ref)));
    }
    Ok(map(entries))
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn map(mut entries: Vec<(&'static str, Value)>) -> Value {
    entries.sort_by(|(left, _), (right, _)| {
        left.len().cmp(&right.len()).then_with(|| left.cmp(right))
    });
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (text(key), value))
            .collect(),
    )
}

fn content_id_value(id: &ContentId) -> Value {
    map(vec![
        ("digest", Value::Bytes(id.digest().to_vec())),
        ("profile", text(id.profile().token())),
    ])
}

fn content_id_from(value: &Value, field: &'static str) -> Result<ContentId, WorldDefinitionError> {
    let fields = Fields::new(value, CONTENT_ID_FIELDS)?;
    let profile =
        HashProfile::from_token(&fields.text("profile")?).ok_or(WorldDefinitionError::Field {
            field,
            problem: "has an unsupported profile",
        })?;
    let digest = fields.fixed_bytes::<DIGEST_BYTES>("digest")?;
    Ok(ContentId::new(profile, digest))
}

struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(value: &'a Value, known: &[&str]) -> Result<Self, WorldDefinitionError> {
        let Value::Map(entries) = value else {
            return Err(WorldDefinitionError::Field {
                field: "record",
                problem: "must be a map",
            });
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(WorldDefinitionError::Field {
                    field: "record",
                    problem: "keys must be text",
                });
            };
            if !known.contains(&name.as_str()) {
                return Err(WorldDefinitionError::Cbor(
                    CborError::UnknownCriticalField {
                        field: name.clone(),
                    },
                ));
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

    fn required(&self, field: &'static str) -> Result<&'a Value, WorldDefinitionError> {
        self.optional(field).ok_or(WorldDefinitionError::Field {
            field,
            problem: "is missing",
        })
    }

    fn text(&self, field: &'static str) -> Result<String, WorldDefinitionError> {
        match self.required(field)? {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(WorldDefinitionError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn uint(&self, field: &'static str) -> Result<u64, WorldDefinitionError> {
        self.uint_value(self.required(field)?, field)
    }

    fn uint_value(&self, value: &Value, field: &'static str) -> Result<u64, WorldDefinitionError> {
        match value {
            Value::UInt(value) => Ok(*value),
            _ => Err(WorldDefinitionError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn fixed_bytes<const N: usize>(
        &self,
        field: &'static str,
    ) -> Result<[u8; N], WorldDefinitionError> {
        match self.required(field)? {
            Value::Bytes(bytes) => {
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_error| WorldDefinitionError::Field {
                        field,
                        problem: "has the wrong length",
                    })
            }
            _ => Err(WorldDefinitionError::Field {
                field,
                problem: "must be bytes",
            }),
        }
    }
}
