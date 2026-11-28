//! Static/configured hash-profile dispatch and streaming content
//! verification (slice A9113-profile-dispatch, issue #10198; parent
//! #9113; design `part-a-addressing/versioned-design.md` section
//! "#9113 — Extensible ID shapes and content portability", A-9113.2,
//! statements S3.2–S3.10).
//!
//! A profile token selects exactly one compiled verifier from the
//! static table [`COMPILED_VERIFIERS`], and a configured
//! [`ProfileDispatch`] decides which of those this node admits. There
//! is no per-object translation map and no registry lookup: the only
//! input to dispatch is the self-describing profile carried by the ID
//! itself (its token or tag), and the only verification is
//! recomputation under that profile. A digest under one profile is
//! never translated into, or accepted as, another profile's digest.
//!
//! [`ProfileDispatch::verify_content`] implements the design's
//! `verify_content(ContentId, byte_stream) -> Verified | DigestMismatch
//! | UnsupportedProfile`. An unsupported or policy-excluded profile is
//! rejected before a single byte of content is read. Content is hashed
//! incrementally from any [`Read`]; the `atlas-chunk-sha256-utf8`
//! profile also validates UTF-8 incrementally, so a code point split
//! across reads is accepted and invalid text rejects as invalid profile
//! parameters, not as a digest mismatch. A stream longer than the
//! configured byte budget rejects as a resource limit.
//!
//! This module builds on the shape types in `id_shape` (slice
//! A9113-shape-types). Its crate-root export belongs to slice 9110-02;
//! until that lands, the integration test includes it by path.

use std::fmt;
use std::io::{self, Read};

use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile, IdShapeError, ParsePolicy};

/// Default content budget for one verification: 64 GiB. Operational,
/// not a property of any profile.
pub const DEFAULT_MAX_CONTENT_BYTES: u64 = 64 << 30;

/// Read buffer size used while streaming content.
const CHUNK_BYTES: usize = 64 * 1024;

/// One compiled verifier: the profile it implements and how to start
/// an incremental digest for it.
pub struct CompiledVerifier {
    profile: HashProfile,
    start: fn() -> DigestState,
}

impl CompiledVerifier {
    /// The profile this verifier implements.
    #[must_use]
    pub const fn profile(&self) -> HashProfile {
        self.profile
    }

    /// The profile token this verifier answers to.
    #[must_use]
    pub fn token(&self) -> &'static str {
        self.profile.token()
    }

    /// A fresh incremental digest for this profile.
    #[must_use]
    pub fn start(&self) -> DigestState {
        (self.start)()
    }
}

impl fmt::Debug for CompiledVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledVerifier")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

/// Every verifier compiled into this build, one per [`HashProfile`],
/// in profile-tag order. Adding a profile adds an entry; nothing else
/// is consulted at dispatch time.
pub static COMPILED_VERIFIERS: [CompiledVerifier; 2] = [
    CompiledVerifier {
        profile: HashProfile::RawBlake3_256,
        start: start_raw_blake3,
    },
    CompiledVerifier {
        profile: HashProfile::AtlasChunkSha256Utf8,
        start: start_atlas_sha256_utf8,
    },
];

fn start_raw_blake3() -> DigestState {
    DigestState(Inner::RawBlake3(Box::new(blake3::Hasher::new())))
}

fn start_atlas_sha256_utf8() -> DigestState {
    use sha2::Digest as _;
    DigestState(Inner::AtlasSha256Utf8 {
        hasher: sha2::Sha256::new(),
        pending: Vec::new(),
    })
}

/// An incremental digest under one profile.
pub struct DigestState(Inner);

enum Inner {
    RawBlake3(Box<blake3::Hasher>),
    AtlasSha256Utf8 {
        hasher: sha2::Sha256,
        /// The tail of an incomplete UTF-8 sequence (at most 3 bytes).
        pending: Vec<u8>,
    },
}

impl fmt::Debug for DigestState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let profile = match self.0 {
            Inner::RawBlake3(_) => HashProfile::RawBlake3_256,
            Inner::AtlasSha256Utf8 { .. } => HashProfile::AtlasChunkSha256Utf8,
        };
        f.debug_tuple("DigestState").field(&profile).finish()
    }
}

const ATLAS_NOT_UTF8: IdShapeError = IdShapeError::InvalidProfileParameters {
    profile: HashProfile::AtlasChunkSha256Utf8,
    detail: "atlas-chunk-sha256-utf8 is defined over UTF-8 text",
};

impl DigestState {
    /// Feeds the next bytes of content.
    ///
    /// # Errors
    ///
    /// [`IdShapeError::InvalidProfileParameters`] when the bytes so far
    /// cannot be text under a UTF-8 profile.
    pub fn update(&mut self, bytes: &[u8]) -> Result<(), IdShapeError> {
        match &mut self.0 {
            Inner::RawBlake3(hasher) => {
                hasher.update(bytes);
            }
            Inner::AtlasSha256Utf8 { hasher, pending } => {
                use sha2::Digest as _;
                hasher.update(bytes);
                pending.extend_from_slice(bytes);
                match std::str::from_utf8(pending) {
                    Ok(_) => pending.clear(),
                    Err(error) if error.error_len().is_none() => {
                        // An incomplete trailing sequence: keep it for
                        // the next read.
                        let valid = error.valid_up_to();
                        pending.drain(..valid);
                    }
                    Err(_) => return Err(ATLAS_NOT_UTF8),
                }
            }
        }
        Ok(())
    }

    /// Finishes the digest.
    ///
    /// # Errors
    ///
    /// [`IdShapeError::InvalidProfileParameters`] when a UTF-8 profile's
    /// content ends inside a code point.
    pub fn finish(self) -> Result<[u8; DIGEST_BYTES], IdShapeError> {
        match self.0 {
            Inner::RawBlake3(hasher) => Ok(*hasher.finalize().as_bytes()),
            Inner::AtlasSha256Utf8 { hasher, pending } => {
                use sha2::Digest as _;
                if !pending.is_empty() {
                    return Err(ATLAS_NOT_UTF8);
                }
                let mut out = [0u8; DIGEST_BYTES];
                out.copy_from_slice(&hasher.finalize());
                Ok(out)
            }
        }
    }
}

/// A successful verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    /// The profile the content was verified under.
    pub profile: HashProfile,
    /// How many content bytes were read.
    pub bytes: u64,
}

/// Why content did not verify.
#[derive(Debug)]
pub enum VerifyError {
    /// The profile is unsupported or excluded, the digest did not
    /// match, the content is invalid for the profile, or a budget was
    /// exceeded.
    Rejected(IdShapeError),
    /// Reading the content failed.
    Io(io::Error),
}

impl VerifyError {
    /// The shape error, when this is a rejection rather than I/O.
    #[must_use]
    pub const fn rejection(&self) -> Option<&IdShapeError> {
        match self {
            Self::Rejected(error) => Some(error),
            Self::Io(_) => None,
        }
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => error.fmt(f),
            Self::Io(error) => write!(f, "reading content: {error}"),
        }
    }
}

impl std::error::Error for VerifyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rejected(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

impl From<IdShapeError> for VerifyError {
    fn from(error: IdShapeError) -> Self {
        Self::Rejected(error)
    }
}

/// The configured side of dispatch: which compiled profiles this node
/// admits, and the per-verification content budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDispatch {
    policy: ParsePolicy,
    max_content_bytes: u64,
}

impl Default for ProfileDispatch {
    fn default() -> Self {
        Self::new(ParsePolicy::ALL, DEFAULT_MAX_CONTENT_BYTES)
    }
}

impl ProfileDispatch {
    /// Dispatch admitting `policy`'s profiles, with a content budget.
    #[must_use]
    pub const fn new(policy: ParsePolicy, max_content_bytes: u64) -> Self {
        Self {
            policy,
            max_content_bytes,
        }
    }

    /// The admitted-profile policy.
    #[must_use]
    pub const fn policy(&self) -> ParsePolicy {
        self.policy
    }

    /// The compiled verifier for `profile`, if this node admits it.
    ///
    /// # Errors
    ///
    /// [`IdShapeError::UnsupportedProfile`] when the policy excludes it.
    pub fn resolve(&self, profile: HashProfile) -> Result<&'static CompiledVerifier, IdShapeError> {
        if !self.policy.admits(profile) {
            return Err(IdShapeError::UnsupportedProfile {
                profile: profile.token().to_owned(),
            });
        }
        COMPILED_VERIFIERS
            .iter()
            .find(|verifier| verifier.profile == profile)
            .ok_or_else(|| IdShapeError::UnsupportedProfile {
                profile: profile.token().to_owned(),
            })
    }

    /// The compiled verifier named by a self-describing profile token.
    /// Matching is exact: no prefix, case-folding or alias.
    ///
    /// # Errors
    ///
    /// [`IdShapeError::UnsupportedProfile`] for an unknown token or one
    /// the policy excludes.
    pub fn resolve_token(&self, token: &str) -> Result<&'static CompiledVerifier, IdShapeError> {
        let profile =
            HashProfile::from_token(token).ok_or_else(|| IdShapeError::UnsupportedProfile {
                profile: token.to_owned(),
            })?;
        self.resolve(profile)
    }

    /// Verifies a byte stream against `id` by recomputing its digest
    /// under the ID's own profile.
    ///
    /// The profile is resolved before `content` is touched, so an
    /// unsupported profile reads nothing.
    ///
    /// # Errors
    ///
    /// [`VerifyError::Rejected`] with `UnsupportedProfile`,
    /// `DigestMismatch`, `InvalidProfileParameters` or `ResourceLimit`;
    /// [`VerifyError::Io`] when reading fails.
    pub fn verify_content<R: Read>(
        &self,
        id: &ContentId,
        mut content: R,
    ) -> Result<Verified, VerifyError> {
        let verifier = self.resolve(id.profile())?;
        let mut state = verifier.start();
        let mut buffer = vec![0u8; CHUNK_BYTES];
        let mut total = 0u64;
        loop {
            let read = match content.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(VerifyError::Io(error)),
            };
            total = total.saturating_add(read as u64);
            if total > self.max_content_bytes {
                return Err(VerifyError::Rejected(IdShapeError::ResourceLimit {
                    limit: "content bytes per verification",
                    actual: usize::try_from(total).unwrap_or(usize::MAX),
                }));
            }
            state.update(&buffer[..read])?;
        }
        if state.finish()? == *id.digest() {
            Ok(Verified {
                profile: verifier.profile,
                bytes: total,
            })
        } else {
            Err(VerifyError::Rejected(IdShapeError::DigestMismatch))
        }
    }

    /// [`ProfileDispatch::verify_content`] over an in-memory slice.
    ///
    /// # Errors
    ///
    /// As [`ProfileDispatch::verify_content`]; never [`VerifyError::Io`].
    pub fn verify_bytes(&self, id: &ContentId, content: &[u8]) -> Result<Verified, VerifyError> {
        self.verify_content(id, content)
    }
}
