//! Immutable content core adapter and verified streaming ingest (slice
//! A9120-content-store, issue #10244; parent #9120; design
//! `part-a-addressing/versioned-design.md` section A-9120.1, statements
//! S4.19–S4.22).
//!
//! Core 1 is immutable bytes addressed by a [`ContentId`]. This module is
//! its store contract over the existing Iroh blob store, which stays the
//! storage boundary: objects are keyed by `iroh_blobs::Hash` exactly as
//! today, and a content ID reaches the store only through the
//! `raw-blake3-256` mapping from slice A9120-content-id. Nothing here
//! rewrites, re-hashes or moves stored data, and any blob Iroh already
//! holds reads back under its old hash unchanged.
//!
//! # Ingest
//!
//! [`ContentStore::ingest`] takes the declared ID, the declared length
//! and a byte stream. It:
//!
//! 1. rejects, before reading anything, an ID whose profile has no Iroh
//!    form ([`ContentStoreError::Unstorable`]), a profile the configured
//!    policy does not admit, and a declared length over the store's
//!    limit;
//! 2. reads the stream into a private staging buffer while hashing it,
//!    reading at most one byte past the declared length;
//! 3. commits only when the stream ended at exactly the declared length
//!    and its digest equals the declared one.
//!
//! A short stream is [`ContentStoreError::Truncated`], a long one
//! [`ContentStoreError::Overlong`], a wrong digest `digest_mismatch`, and
//! a read failure [`ContentStoreError::Io`]. In every one of those cases
//! the staging buffer is dropped and the backend is never called, so a
//! partial upload is staging state and never an accepted object.
//!
//! The digest is unkeyed BLAKE3-256 over the exact bytes, the definition
//! of `raw-blake3-256` and of `iroh_blobs::Hash::new`. As a second check
//! the backend reports the hash it stored under, and a commit whose hash
//! differs from the declared one is [`ContentStoreError::BackendMismatch`].
//!
//! # Reads
//!
//! [`ContentStore::get`] recomputes the digest of the bytes the backend
//! returns and reports [`ContentStoreError::Corrupt`] instead of
//! returning bytes that do not match the ID.
//!
//! # Wiring
//!
//! The crate-root export of this module, and of `content_id` from
//! id-core, belongs to slice 9110-02. Until that lands this module names
//! the content-ID helpers as `super::content_id`, and the integration
//! test includes both by path.

use std::fmt;
use std::future::Future;
use std::io::{self, Read};

use id_core::id_shape::{ContentId, IdShapeError, ParsePolicy};
use iroh_blobs::Hash;

use super::content_id::{ContentIdError, IROH_PROFILE, to_iroh_hash};

/// Bytes read from the stream per call.
const CHUNK_BYTES: usize = 64 * 1024;

/// Default limit on one object, 1 GiB.
pub const DEFAULT_MAX_OBJECT_BYTES: u64 = 1 << 30;

/// The storage adapter seam: a store of complete, immutable blobs keyed
/// by Iroh hash. The existing Iroh blob store implements it.
///
/// `has` is true only for a complete blob. `commit` stores exact bytes
/// and returns the hash the backend stored them under.
pub trait BlobBackend {
    /// Whether a complete blob with this hash is stored.
    fn has(&self, hash: Hash) -> impl Future<Output = Result<bool, anyhow::Error>> + Send;

    /// The stored bytes of a complete blob, or `None`.
    fn read(
        &self,
        hash: Hash,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, anyhow::Error>> + Send;

    /// Stores exact bytes as one complete blob.
    fn commit(&self, bytes: Vec<u8>) -> impl Future<Output = Result<Hash, anyhow::Error>> + Send;
}

impl BlobBackend for iroh_blobs::api::blobs::Blobs {
    async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
        Ok(Self::has(self, hash).await?)
    }

    async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
        if !Self::has(self, hash).await? {
            return Ok(None);
        }
        Ok(Some(self.get_bytes(hash).await?.to_vec()))
    }

    async fn commit(&self, bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
        Ok(self.add_bytes(bytes).await?.hash)
    }
}

/// Why a store operation failed. No failing ingest commits anything.
#[derive(Debug)]
pub enum ContentStoreError {
    /// The ID is under a profile with no Iroh form; it cannot be stored
    /// or looked up here.
    Unstorable(ContentIdError),
    /// The ID or stream was rejected: unsupported profile under the
    /// policy, digest mismatch, or resource limit.
    Rejected(IdShapeError),
    /// The stream ended before the declared length.
    Truncated {
        /// Declared length.
        declared: u64,
        /// Bytes the stream delivered.
        received: u64,
    },
    /// The stream had more bytes than the declared length.
    Overlong {
        /// Declared length.
        declared: u64,
    },
    /// Reading the stream failed.
    Io(io::Error),
    /// The backend failed.
    Backend(anyhow::Error),
    /// The backend stored verified bytes under a different hash.
    BackendMismatch {
        /// The hash the verified bytes have.
        expected: Hash,
        /// The hash the backend reported.
        stored: Hash,
    },
    /// Bytes read back from the backend do not match their ID.
    Corrupt,
}

impl ContentStoreError {
    /// A stable failure class: `profile_mismatch`, `unsupported_profile`,
    /// `digest_mismatch`, `resource_limit`, `malformed`, `truncated`,
    /// `overlong`, `io`, `backend`, `backend_mismatch` or `corrupt`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Unstorable(error) => error.class(),
            Self::Rejected(IdShapeError::UnsupportedProfile { .. }) => "unsupported_profile",
            Self::Rejected(IdShapeError::DigestMismatch) => "digest_mismatch",
            Self::Rejected(IdShapeError::ResourceLimit { .. }) => "resource_limit",
            Self::Rejected(_) => "malformed",
            Self::Truncated { .. } => "truncated",
            Self::Overlong { .. } => "overlong",
            Self::Io(_) => "io",
            Self::Backend(_) => "backend",
            Self::BackendMismatch { .. } => "backend_mismatch",
            Self::Corrupt => "corrupt",
        }
    }
}

impl fmt::Display for ContentStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unstorable(error) => write!(f, "content cannot be stored here: {error}"),
            Self::Rejected(error) => error.fmt(f),
            Self::Truncated { declared, received } => write!(
                f,
                "content stream ended after {received} of {declared} declared bytes"
            ),
            Self::Overlong { declared } => {
                write!(
                    f,
                    "content stream is longer than its declared {declared} bytes"
                )
            }
            Self::Io(error) => write!(f, "reading content failed: {error}"),
            Self::Backend(error) => write!(f, "content backend failed: {error}"),
            Self::BackendMismatch { expected, stored } => {
                write!(f, "content backend stored {expected} under {stored}")
            }
            Self::Corrupt => f.write_str("stored content does not match its ID"),
        }
    }
}

impl std::error::Error for ContentStoreError {}

/// A committed object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ingested {
    /// Its content ID.
    pub id: ContentId,
    /// Its length in bytes.
    pub bytes: u64,
    /// False when the backend already held it; the stream was still
    /// verified in full.
    pub newly_stored: bool,
}

/// Store limits and the profiles it accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentStoreConfig {
    /// Profiles accepted on ingest and lookup. Only `raw-blake3-256` has
    /// an Iroh form, so a policy without it accepts nothing.
    pub policy: ParsePolicy,
    /// Largest object, in bytes.
    pub max_object_bytes: u64,
}

impl Default for ContentStoreConfig {
    fn default() -> Self {
        Self {
            policy: ParsePolicy::ALL,
            max_object_bytes: DEFAULT_MAX_OBJECT_BYTES,
        }
    }
}

/// The immutable content core over a blob backend.
#[derive(Debug, Clone)]
pub struct ContentStore<B> {
    backend: B,
    config: ContentStoreConfig,
}

impl<B: BlobBackend + Sync> ContentStore<B> {
    /// A content store over `backend`.
    pub const fn new(backend: B, config: ContentStoreConfig) -> Self {
        Self { backend, config }
    }

    /// The backend.
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// The configuration.
    pub const fn config(&self) -> ContentStoreConfig {
        self.config
    }

    /// The Iroh hash for `id`, after the profile and policy checks.
    fn key(&self, id: &ContentId) -> Result<Hash, ContentStoreError> {
        let hash = to_iroh_hash(id).map_err(ContentStoreError::Unstorable)?;
        if !self.config.policy.admits(IROH_PROFILE) {
            return Err(ContentStoreError::Rejected(
                IdShapeError::UnsupportedProfile {
                    profile: IROH_PROFILE.token().to_owned(),
                },
            ));
        }
        Ok(hash)
    }

    /// Whether the object is stored.
    ///
    /// # Errors
    ///
    /// An ID under another profile, a profile the policy does not admit,
    /// or a backend failure.
    pub async fn contains(&self, id: &ContentId) -> Result<bool, ContentStoreError> {
        let hash = self.key(id)?;
        self.backend
            .has(hash)
            .await
            .map_err(ContentStoreError::Backend)
    }

    /// The stored bytes, verified against `id`, or `None`.
    ///
    /// # Errors
    ///
    /// As [`ContentStore::contains`], and [`ContentStoreError::Corrupt`]
    /// when the bytes read back do not match the ID.
    pub async fn get(&self, id: &ContentId) -> Result<Option<Vec<u8>>, ContentStoreError> {
        let hash = self.key(id)?;
        let Some(bytes) = self
            .backend
            .read(hash)
            .await
            .map_err(ContentStoreError::Backend)?
        else {
            return Ok(None);
        };
        if blake3::hash(&bytes).as_bytes() == id.digest() {
            Ok(Some(bytes))
        } else {
            Err(ContentStoreError::Corrupt)
        }
    }

    /// Verifies `content` against the declared ID and length while
    /// reading it, and commits it only on a complete match.
    ///
    /// # Errors
    ///
    /// See the module documentation. On any error nothing is committed.
    pub async fn ingest<R: Read>(
        &self,
        declared: &ContentId,
        declared_len: u64,
        content: R,
    ) -> Result<Ingested, ContentStoreError> {
        let expected = self.key(declared)?;
        if declared_len > self.config.max_object_bytes {
            return Err(ContentStoreError::Rejected(IdShapeError::ResourceLimit {
                limit: "content bytes per object",
                actual: usize::try_from(declared_len).unwrap_or(usize::MAX),
            }));
        }
        let staged = stage(declared, declared_len, content)?;
        let newly_stored = !self
            .backend
            .has(expected)
            .await
            .map_err(ContentStoreError::Backend)?;
        if newly_stored {
            let stored = self
                .backend
                .commit(staged)
                .await
                .map_err(ContentStoreError::Backend)?;
            if stored != expected {
                return Err(ContentStoreError::BackendMismatch { expected, stored });
            }
        }
        Ok(Ingested {
            id: *declared,
            bytes: declared_len,
            newly_stored,
        })
    }
}

/// Reads at most `declared_len + 1` bytes into a staging buffer while
/// hashing them, and returns the buffer only on an exact length and
/// digest match.
fn stage<R: Read>(
    declared: &ContentId,
    declared_len: u64,
    content: R,
) -> Result<Vec<u8>, ContentStoreError> {
    let mut limited = content.take(declared_len.saturating_add(1));
    let mut hasher = blake3::Hasher::new();
    let mut staged = Vec::new();
    let mut chunk = vec![0u8; CHUNK_BYTES];
    loop {
        let read = match limited.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(ContentStoreError::Io(error)),
        };
        hasher.update(&chunk[..read]);
        staged.extend_from_slice(&chunk[..read]);
    }
    let received = staged.len() as u64;
    if received < declared_len {
        return Err(ContentStoreError::Truncated {
            declared: declared_len,
            received,
        });
    }
    if received > declared_len {
        return Err(ContentStoreError::Overlong {
            declared: declared_len,
        });
    }
    if hasher.finalize().as_bytes() == declared.digest() {
        Ok(staged)
    } else {
        Err(ContentStoreError::Rejected(IdShapeError::DigestMismatch))
    }
}
