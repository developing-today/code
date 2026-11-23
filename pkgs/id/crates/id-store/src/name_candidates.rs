//! Signed, collision-preserving string-path name candidates (A-9121.1).
//!
//! Names retain their original UTF-8 spelling. A namespace may opt into
//! an NFKC/case-fold comparison profile for lookup, but comparison never
//! rewrites the stored name. Every target assertion remains a separately
//! signed candidate, so a collision is data rather than an implicit
//! winner.

use std::collections::BTreeSet;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use id_core::canonical_cbor::{CborError, Value, encode_canonical};
use id_core::id_shape::{ContentId, HashProfile, IdShapeError};
use unicode_normalization::UnicodeNormalization;

/// Versioned signing domain for name candidate assertions.
pub const NAME_CANDIDATE_DOMAIN: &str = "id.name-candidate/v1";
/// Record version written by this module.
pub const RECORD_VERSION: u64 = 1;
/// Maximum byte length of source and scope labels.
pub const MAX_LABEL_BYTES: usize = 256;

/// The namespace comparison profile. Neither profile changes the stored
/// components returned by [`NamePath::components`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComparisonProfile {
    /// Compare components byte-for-byte as UTF-8 strings.
    Exact,
    /// Compare NFKC-normalized, Unicode-lowercased components.
    NfkcCaseFold,
}

impl ComparisonProfile {
    fn token(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::NfkcCaseFold => "nfkc-casefold",
        }
    }
}

/// A path of human name components plus its namespace comparison profile.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NamePath {
    components: Vec<String>,
    profile: ComparisonProfile,
}

impl NamePath {
    /// Construct a non-empty path without changing its original spelling.
    pub fn new(
        components: impl IntoIterator<Item = impl Into<String>>,
        profile: ComparisonProfile,
    ) -> Result<Self, NameCandidateError> {
        let components = components.into_iter().map(Into::into).collect::<Vec<_>>();
        if components.is_empty() {
            return Err(NameCandidateError::Invalid("name path must not be empty"));
        }
        if components
            .iter()
            .any(|component| component.is_empty() || component.chars().any(char::is_control))
        {
            return Err(NameCandidateError::Invalid(
                "name components must be non-empty and contain no controls",
            ));
        }
        Ok(Self {
            components,
            profile,
        })
    }

    /// The exact source spelling, in path order.
    #[must_use]
    pub fn components(&self) -> &[String] {
        &self.components
    }

    /// The configured comparison profile.
    #[must_use]
    pub const fn profile(&self) -> ComparisonProfile {
        self.profile
    }

    /// The lookup key. Original components remain untouched.
    #[must_use]
    pub fn comparison_key(&self) -> Vec<String> {
        self.components
            .iter()
            .map(|component| match self.profile {
                ComparisonProfile::Exact => component.clone(),
                ComparisonProfile::NfkcCaseFold => {
                    component.nfkc().flat_map(char::to_lowercase).collect()
                }
            })
            .collect()
    }
}

/// Why a candidate or lookup was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameCandidateError {
    /// A field violates the candidate contract.
    Invalid(&'static str),
    /// Canonical record encoding failed.
    Cbor(CborError),
    /// An embedded content identity failed to compute.
    Id(IdShapeError),
    /// The signer key bytes are not a valid Ed25519 public key.
    BadKey,
    /// The signed assertion does not verify.
    BadSignature,
}

impl fmt::Display for NameCandidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problem) => f.write_str(problem),
            Self::Cbor(error) => write!(f, "candidate encoding: {error}"),
            Self::Id(error) => write!(f, "candidate identity: {error}"),
            Self::BadKey => f.write_str("candidate signer key is invalid"),
            Self::BadSignature => f.write_str("candidate signature is invalid"),
        }
    }
}

impl std::error::Error for NameCandidateError {}

impl From<CborError> for NameCandidateError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for NameCandidateError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

/// One signed name-to-target assertion. Candidates are not deduplicated:
/// distinct signed claims remain distinct even when their comparison keys
/// collide or their targets are equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameCandidate {
    name: NamePath,
    target: Vec<u8>,
    source: String,
    scope: String,
    freshness: u64,
    signer: [u8; 32],
    signature: [u8; 64],
}

impl NameCandidate {
    /// Sign a candidate assertion with the supplied signer and scope.
    pub fn sign(
        name: NamePath,
        target: Vec<u8>,
        source: impl Into<String>,
        scope: impl Into<String>,
        freshness: u64,
        key: &SigningKey,
    ) -> Result<Self, NameCandidateError> {
        let source = source.into();
        let scope = scope.into();
        checked_label(&source)?;
        checked_label(&scope)?;
        let signer = key.verifying_key().to_bytes();
        let unsigned = unsigned_value(&name, &target, &source, &scope, freshness, &signer);
        let signature = key
            .sign(&signature_preimage(&encode_canonical(&unsigned)?))
            .to_bytes();
        Ok(Self {
            name,
            target,
            source,
            scope,
            freshness,
            signer,
            signature,
        })
    }

    /// Verify the signature and all signed fields.
    pub fn verify(&self) -> Result<(), NameCandidateError> {
        checked_label(&self.source)?;
        checked_label(&self.scope)?;
        let key = VerifyingKey::from_bytes(&self.signer).map_err(|_| NameCandidateError::BadKey)?;
        let unsigned = unsigned_value(
            &self.name,
            &self.target,
            &self.source,
            &self.scope,
            self.freshness,
            &self.signer,
        );
        key.verify_strict(
            &signature_preimage(&encode_canonical(&unsigned)?),
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_| NameCandidateError::BadSignature)
    }

    /// The original name path.
    #[must_use]
    pub const fn name(&self) -> &NamePath {
        &self.name
    }

    /// The signed target assertion bytes.
    #[must_use]
    pub fn target(&self) -> &[u8] {
        &self.target
    }

    /// The source/provenance label.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The authorization scope carried by the assertion.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The signer public key bytes.
    #[must_use]
    pub const fn signer(&self) -> &[u8; 32] {
        &self.signer
    }

    /// A stable identity for this complete signed candidate.
    pub fn candidate_id(&self) -> Result<ContentId, NameCandidateError> {
        ContentId::of_bytes(HashProfile::RawBlake3_256, &self.canonical_bytes()?)
            .map_err(Into::into)
    }

    /// Canonical bytes for storage or transport.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NameCandidateError> {
        let mut entries = unsigned_entries(
            &self.name,
            &self.target,
            &self.source,
            &self.scope,
            self.freshness,
            &self.signer,
        );
        entries.push((text("signature"), Value::Bytes(self.signature.to_vec())));
        Ok(encode_canonical(&Value::Map(entries))?)
    }
}

/// A small in-memory candidate collection representing the storage/query
/// seam. Persistence adapters can store each candidate's canonical bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameCandidateStore {
    candidates: Vec<NameCandidate>,
}

impl NameCandidateStore {
    /// Verify and retain one candidate. Existing candidates are never
    /// replaced, so collisions remain observable.
    pub fn insert(&mut self, candidate: NameCandidate) -> Result<(), NameCandidateError> {
        candidate.verify()?;
        self.candidates.push(candidate);
        Ok(())
    }

    /// Return all authorized candidates matching the configured comparison
    /// key, deterministically ordered by their signed candidate identity.
    pub fn query(
        &self,
        name: &NamePath,
        authorized_signers: &BTreeSet<[u8; 32]>,
    ) -> Result<Vec<NameCandidate>, NameCandidateError> {
        let key = name.comparison_key();
        let mut matches = Vec::new();
        for candidate in &self.candidates {
            if !authorized_signers.contains(candidate.signer()) {
                continue;
            }
            candidate.verify()?;
            if candidate.name().profile() == name.profile()
                && candidate.name().comparison_key() == key
            {
                matches.push((
                    candidate.candidate_id()?.digest().to_vec(),
                    candidate.clone(),
                ));
            }
        }
        matches.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(matches
            .into_iter()
            .map(|(_, candidate)| candidate)
            .collect())
    }
}

fn checked_label(label: &str) -> Result<(), NameCandidateError> {
    if label.is_empty() || label.len() > MAX_LABEL_BYTES || label.chars().any(char::is_control) {
        Err(NameCandidateError::Invalid(
            "source and scope must be non-empty visible labels",
        ))
    } else {
        Ok(())
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn path_value(path: &NamePath) -> Value {
    Value::Map(vec![
        (
            text("components"),
            Value::Array(
                path.components
                    .iter()
                    .map(|component| text(component))
                    .collect(),
            ),
        ),
        (text("profile"), text(path.profile.token())),
    ])
}

fn unsigned_entries(
    name: &NamePath,
    target: &[u8],
    source: &str,
    scope: &str,
    freshness: u64,
    signer: &[u8; 32],
) -> Vec<(Value, Value)> {
    vec![
        (text("freshness"), Value::UInt(freshness)),
        (text("name"), path_value(name)),
        (text("scope"), text(scope)),
        (text("signer"), Value::Bytes(signer.to_vec())),
        (text("source"), text(source)),
        (text("target"), Value::Bytes(target.to_vec())),
        (text("v"), Value::UInt(RECORD_VERSION)),
    ]
}

fn unsigned_value(
    name: &NamePath,
    target: &[u8],
    source: &str,
    scope: &str,
    freshness: u64,
    signer: &[u8; 32],
) -> Value {
    Value::Map(unsigned_entries(
        name, target, source, scope, freshness, signer,
    ))
}

fn signature_preimage(unsigned: &[u8]) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(NAME_CANDIDATE_DOMAIN.len() + 1 + unsigned.len());
    preimage.extend_from_slice(NAME_CANDIDATE_DOMAIN.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(unsigned);
    preimage
}
