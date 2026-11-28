//! Content-address core identity: the raw exact-byte Iroh BLAKE3-256
//! content ID with its profile token, and the existing Iroh binary and
//! text forms preserved unchanged (slice A9120-content-id, issue #10227;
//! parent #9120; design `part-a-addressing/versioned-design.md`
//! sections A-9113.2 and A-9120.1, statements S3.4–S3.10 and
//! S4.19–S4.22).
//!
//! Core 1 is immutable bytes addressed by a [`ContentId`]. Its initial
//! profile is `raw-blake3-256`: Iroh's unkeyed BLAKE3-256 over the exact
//! bytes, a 32-byte digest, with no application prefix, normalization,
//! metadata, location or media fields. The identity of a byte string is
//! [`raw_content_id`] of it and nothing else.
//!
//! # Forms
//!
//! - **Profiled** (this design): the `id_shape` text and binary forms,
//!   `content/raw-blake3-256.<64 lowercase hex>`, which carry the
//!   profile token.
//! - **Iroh legacy**: exactly what `iroh_blobs::Hash` reads and writes.
//!   Binary is the bare 32-byte digest. Text is written as 64 lowercase
//!   hex; it is read as 64 lowercase hex or as unpadded base32 in either
//!   case. Parsing and formatting delegate to `iroh_blobs::Hash`, so the
//!   accepted and produced strings are Iroh's own, unchanged. The one
//!   addition is a length check before delegating: Iroh accepts only 64
//!   or 52 characters but panics, rather than erroring, on any other
//!   length, and a parser of untrusted text must not panic.
//!
//! The caller names the form ([`ContentTextForm`]); nothing guesses a
//! form from a string's prefix. A legacy form carries no profile, so it
//! always means `raw-blake3-256`, and only a policy that admits that
//! profile accepts it.
//!
//! A content ID under any other profile has no Iroh form. Asking for one
//! is [`ContentIdError::ProfileMismatch`], never a conversion: there is
//! no translation from one profile's digest to another's.
//!
//! The crate-root export belongs to slice 9110-02; until that lands the
//! integration test includes this module and `id_shape` by path.

use std::fmt;
use std::str::FromStr as _;

use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile, Id, IdShapeError, ParsePolicy};

/// The one profile with Iroh legacy forms.
pub const IROH_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// Length of Iroh's hex text form.
pub const IROH_HEX_LEN: usize = 64;

/// Length of Iroh's unpadded base32 text form.
pub const IROH_BASE32_LEN: usize = 52;

/// Why a content ID could not be read or converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentIdError {
    /// The input was rejected: malformed, unsupported shape or profile.
    Shape(IdShapeError),
    /// The ID is under a profile other than the one the form requires.
    ProfileMismatch {
        /// The profile the requested form requires.
        expected: HashProfile,
        /// The profile the ID is under.
        found: HashProfile,
    },
}

impl ContentIdError {
    /// A stable failure class: `malformed`, `unsupported_shape`,
    /// `unsupported_profile`, `resource_limit` or `profile_mismatch`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Shape(
                IdShapeError::UnsupportedShape { .. } | IdShapeError::UnsupportedVersion { .. },
            ) => "unsupported_shape",
            Self::Shape(IdShapeError::UnsupportedProfile { .. }) => "unsupported_profile",
            Self::Shape(IdShapeError::ResourceLimit { .. }) => "resource_limit",
            Self::Shape(_) => "malformed",
            Self::ProfileMismatch { .. } => "profile_mismatch",
        }
    }
}

impl fmt::Display for ContentIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape(error) => error.fmt(f),
            Self::ProfileMismatch { expected, found } => write!(
                f,
                "content ID is under profile {found}, but this form requires {expected}"
            ),
        }
    }
}

impl std::error::Error for ContentIdError {}

impl From<IdShapeError> for ContentIdError {
    fn from(error: IdShapeError) -> Self {
        Self::Shape(error)
    }
}

/// Which text form a caller is reading or writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTextForm {
    /// `content/<profile-token>.<64 lowercase hex>`.
    Profiled,
    /// Iroh's own hash text: written as lowercase hex, read as hex or
    /// base32. Always `raw-blake3-256`.
    IrohLegacy,
}

/// The `raw-blake3-256` content ID of exactly `bytes`.
#[must_use]
pub fn raw_content_id(bytes: &[u8]) -> ContentId {
    ContentId::new(IROH_PROFILE, *blake3::hash(bytes).as_bytes())
}

/// The content ID an Iroh hash names.
#[must_use]
pub fn from_iroh_hash(hash: &iroh_blobs::Hash) -> ContentId {
    ContentId::new(IROH_PROFILE, *hash.as_bytes())
}

/// The Iroh hash for a `raw-blake3-256` content ID.
///
/// # Errors
///
/// [`ContentIdError::ProfileMismatch`] for any other profile.
pub fn to_iroh_hash(id: &ContentId) -> Result<iroh_blobs::Hash, ContentIdError> {
    require_iroh_profile(id)?;
    Ok(iroh_blobs::Hash::from_bytes(*id.digest()))
}

fn require_iroh_profile(id: &ContentId) -> Result<(), ContentIdError> {
    if id.profile() == IROH_PROFILE {
        Ok(())
    } else {
        Err(ContentIdError::ProfileMismatch {
            expected: IROH_PROFILE,
            found: id.profile(),
        })
    }
}

fn admit_legacy(policy: ParsePolicy) -> Result<(), ContentIdError> {
    if policy.admits(IROH_PROFILE) {
        Ok(())
    } else {
        Err(ContentIdError::Shape(IdShapeError::UnsupportedProfile {
            profile: IROH_PROFILE.token().to_owned(),
        }))
    }
}

fn malformed(detail: &str) -> ContentIdError {
    ContentIdError::Shape(IdShapeError::Malformed {
        detail: detail.to_owned(),
    })
}

/// Reads Iroh's legacy binary form: the bare 32-byte digest.
///
/// # Errors
///
/// Malformed for any other length; unsupported profile when `policy`
/// does not admit `raw-blake3-256`.
pub fn parse_legacy_bytes(bytes: &[u8], policy: ParsePolicy) -> Result<ContentId, ContentIdError> {
    admit_legacy(policy)?;
    let digest = <[u8; DIGEST_BYTES]>::try_from(bytes)
        .map_err(|_error| malformed("an Iroh hash is exactly 32 bytes"))?;
    Ok(ContentId::new(IROH_PROFILE, digest))
}

/// Writes Iroh's legacy binary form.
///
/// # Errors
///
/// [`ContentIdError::ProfileMismatch`] unless the ID is `raw-blake3-256`.
pub fn to_legacy_bytes(id: &ContentId) -> Result<[u8; DIGEST_BYTES], ContentIdError> {
    require_iroh_profile(id)?;
    Ok(*id.digest())
}

/// Reads a content ID in the named text form under `policy`.
///
/// # Errors
///
/// Malformed text, an unsupported shape or profile, or a profiled ID
/// that is not a content ID.
pub fn parse_content_text(
    text: &str,
    form: ContentTextForm,
    policy: ParsePolicy,
) -> Result<ContentId, ContentIdError> {
    match form {
        ContentTextForm::Profiled => match Id::parse_text_with(text, &policy)? {
            Id::Content(id) => Ok(id),
            // A well-formed ID of another shape is not a content ID.
            other => Err(ContentIdError::Shape(IdShapeError::UnsupportedShape {
                tag: other.shape().tag(),
            })),
        },
        ContentTextForm::IrohLegacy => {
            admit_legacy(policy)?;
            // Iroh's parser only ever accepts these two lengths, and
            // `iroh_blobs::Hash::from_str` panics inside its base32
            // decoder on any other length, so the length is checked
            // here first.
            if text.len() != IROH_HEX_LEN && text.len() != IROH_BASE32_LEN {
                return Err(malformed("an Iroh hash is 64 hex or 52 base32 characters"));
            }
            let hash = iroh_blobs::Hash::from_str(text)
                .map_err(|_error| malformed("not an Iroh hash in hex or base32"))?;
            Ok(from_iroh_hash(&hash))
        }
    }
}

/// Writes a content ID in the named text form.
///
/// # Errors
///
/// [`ContentIdError::ProfileMismatch`] when asking for the Iroh form of
/// an ID under another profile.
pub fn format_content_text(
    id: &ContentId,
    form: ContentTextForm,
) -> Result<String, ContentIdError> {
    match form {
        ContentTextForm::Profiled => Ok(Id::Content(*id).to_text()),
        ContentTextForm::IrohLegacy => Ok(to_iroh_hash(id)?.to_string()),
    }
}
