//! ID shape discriminator and strict version/profile parsing
//! (slice A9113-shape-types, issue #10180; parent #9113; settled
//! design: `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-a-addressing/versioned-design.md`, sections A-9113.1 and
//! A-9113.2; statements S3.1–S3.3).
//!
//! Multiple ID shapes are supported and there is no single canonical
//! format (S3.1): consumers dispatch behaviour by shape (S3.2), and
//! new shapes are added without invalidating existing IDs (S3.3).
//! This module is the shape/profile registry and dispatch layer
//! only — it owns no storage core (#9120 owns the content core,
//! #9118 owns the array core and the semantic `ArrayAddress` type;
//! the [`Id::Array`] variant here is the shape-level reference form
//! and converts to that core in the wiring slice).
//!
//! # The tagged sum
//!
//! Per A-9113.1, IDs parse into a tagged sum, never a string-prefix
//! heuristic: `Id = ContentId | ArrayId | DerivedId`. Each value
//! carries an explicit shape tag and [`ID_SHAPE_VERSION`]; an
//! unknown shape tag or version returns
//! [`IdShapeError::UnsupportedShape`] /
//! [`IdShapeError::UnsupportedVersion`], never a guessed fallback.
//!
//! # Canonical binary encoding
//!
//! ```text
//! byte 0   shape tag (0x01 content, 0x02 array, 0x03 derived)
//! byte 1   shape version (0x01)
//! body     per shape:
//!   content: [profile tag][32-byte digest]
//!            profile tags: 0x01 raw-blake3-256,
//!                          0x02 atlas-chunk-sha256-utf8
//!   array:   [slot count u32 LE] then per slot:
//!            0x00 for null, or 0x01 [len u32 LE][bytes]
//!   derived: [kind tag][payload len u32 LE][payload]
//!            kind tags: 0x01 name, 0x02 numeric, 0x03 stream,
//!                       0x04 signed-world
//! ```
//!
//! The encoding is canonical: for every admitted byte string `x`,
//! `encode(parse(x)) == x`, and every known encoding maps to exactly
//! one variant. Parsing is strict — trailing bytes, truncation,
//! non-canonical text forms and over-limit inputs all reject, each
//! with its own error variant (the design's error classes:
//! malformed, unsupported shape/profile, digest mismatch, invalid
//! profile parameters, resource limit).
//!
//! # Text encoding
//!
//! Content IDs use the design's settled text form (#9117 grammar):
//! `content/<profile-token>.<64 lowercase hex>`. Array IDs use the
//! #9117 slot grammar: `array` followed by `/~` (null) or
//! `/b.<percent-encoded bytes>` per slot, with #9117 canonicality
//! (uppercase `%HH`, unreserved octets literal, redundant escapes
//! rejected, decode exactly once). Derived IDs use
//! `derived/<kind-token>/v1.<lowercase hex payload>`; #9117 owns URI
//! transport syntax and does not cover derived families, so this
//! module fixes that text form here.
//!
//! # Profiles (A-9113.2)
//!
//! The initial content profile is `raw-blake3-256`: unkeyed
//! BLAKE3-256 over the exact bytes, 32-byte digest, no prefix or
//! normalization. The Atlas snapshot profile
//! `atlas-chunk-sha256-utf8` (lowercase SHA-256 over exact UTF-8
//! chunk text) is a distinct typed profile, never conflated with
//! the raw-byte profile. SHA-256 signed-artifact commitments remain
//! a separate domain-specific ID and are deliberately *not* a
//! content profile here. Profile dispatch is static/configured
//! parsing via [`ParsePolicy`], never a per-object translation map
//! or registry lookup (the no-map rule, S3.5).

use std::fmt;

/// The shape-encoding version this module reads and writes. Adding
/// a shape or profile is additive within a version; a semantics
/// change needs a new version, and parsers reject versions they do
/// not know instead of guessing.
pub const ID_SHAPE_VERSION: u8 = 1;

/// Digest length shared by every admitted content profile.
pub const DIGEST_BYTES: usize = 32;

/// Maximum number of slots in an array ID (resource limit).
pub const MAX_ARRAY_SLOTS: usize = 1024;

/// Maximum byte length of one array slot (resource limit).
pub const MAX_SLOT_BYTES: usize = 65_536;

/// Maximum byte length of a derived ID payload (resource limit).
pub const MAX_DERIVED_PAYLOAD_BYTES: usize = 65_536;

/// Maximum total length of an encoded ID, binary or text
/// (resource limit).
pub const MAX_ENCODED_BYTES: usize = 1 << 20;

const SHAPE_TAG_CONTENT: u8 = 0x01;
const SHAPE_TAG_ARRAY: u8 = 0x02;
const SHAPE_TAG_DERIVED: u8 = 0x03;

const PROFILE_TAG_RAW_BLAKE3_256: u8 = 0x01;
const PROFILE_TAG_ATLAS_CHUNK_SHA256_UTF8: u8 = 0x02;

const DERIVED_TAG_NAME: u8 = 0x01;
const DERIVED_TAG_NUMERIC: u8 = 0x02;
const DERIVED_TAG_STREAM: u8 = 0x03;
const DERIVED_TAG_SIGNED_WORLD: u8 = 0x04;

/// The shape of an [`Id`] — the discriminator consumers dispatch
/// on (S3.2). Finite and closed: a new shape is a new variant, a
/// new tag, and no change to the interpretation of stored bytes
/// (S3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IdShape {
    /// Content-addressed immutable object (content core, #9120).
    Content,
    /// Array-addressed mutable record (array core, #9118).
    Array,
    /// A derived namespace/lens over one of the two cores
    /// (names, numeric registries, streams, signed worlds).
    Derived,
}

impl IdShape {
    /// The binary shape tag.
    pub fn tag(self) -> u8 {
        match self {
            IdShape::Content => SHAPE_TAG_CONTENT,
            IdShape::Array => SHAPE_TAG_ARRAY,
            IdShape::Derived => SHAPE_TAG_DERIVED,
        }
    }

    /// The text-form shape token.
    pub fn token(self) -> &'static str {
        match self {
            IdShape::Content => "content",
            IdShape::Array => "array",
            IdShape::Derived => "derived",
        }
    }

    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            SHAPE_TAG_CONTENT => Some(IdShape::Content),
            SHAPE_TAG_ARRAY => Some(IdShape::Array),
            SHAPE_TAG_DERIVED => Some(IdShape::Derived),
            _ => None,
        }
    }
}

impl fmt::Display for IdShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// A content hash profile (A-9113.2). Profiles are explicit and
/// additive; a digest under one profile is never reinterpreted
/// under another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HashProfile {
    /// `raw-blake3-256` — unkeyed BLAKE3-256 over the exact bytes;
    /// the initial content profile.
    RawBlake3_256,
    /// `atlas-chunk-sha256-utf8` — lowercase SHA-256 over exact
    /// UTF-8 chunk text; the observed Atlas snapshot profile. Its
    /// producer contract is unverified, so this profile is admitted
    /// for parsing/dispatch, distinct from the raw-byte profile.
    AtlasChunkSha256Utf8,
}

impl HashProfile {
    /// Every admitted profile, in tag order.
    pub const ALL: [HashProfile; 2] = [
        HashProfile::RawBlake3_256,
        HashProfile::AtlasChunkSha256Utf8,
    ];

    /// The profile token used in text forms.
    pub fn token(self) -> &'static str {
        match self {
            HashProfile::RawBlake3_256 => "raw-blake3-256",
            HashProfile::AtlasChunkSha256Utf8 => "atlas-chunk-sha256-utf8",
        }
    }

    /// Strict token parsing: exact match only, no prefix or
    /// case-folding heuristics.
    pub fn from_token(token: &str) -> Option<Self> {
        HashProfile::ALL
            .into_iter()
            .find(|profile| profile.token() == token)
    }

    /// The binary profile tag.
    pub fn tag(self) -> u8 {
        match self {
            HashProfile::RawBlake3_256 => PROFILE_TAG_RAW_BLAKE3_256,
            HashProfile::AtlasChunkSha256Utf8 => PROFILE_TAG_ATLAS_CHUNK_SHA256_UTF8,
        }
    }

    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            PROFILE_TAG_RAW_BLAKE3_256 => Some(HashProfile::RawBlake3_256),
            PROFILE_TAG_ATLAS_CHUNK_SHA256_UTF8 => Some(HashProfile::AtlasChunkSha256Utf8),
            _ => None,
        }
    }

    /// Compute this profile's digest over `bytes`.
    ///
    /// The Atlas profile is defined over UTF-8 text: non-UTF-8
    /// input is an invalid profile parameter, not a digest.
    pub fn digest(&self, bytes: &[u8]) -> Result<[u8; DIGEST_BYTES], IdShapeError> {
        match self {
            HashProfile::RawBlake3_256 => Ok(*blake3::hash(bytes).as_bytes()),
            HashProfile::AtlasChunkSha256Utf8 => {
                if std::str::from_utf8(bytes).is_err() {
                    return Err(IdShapeError::InvalidProfileParameters {
                        profile: *self,
                        detail: "atlas-chunk-sha256-utf8 is defined over UTF-8 text",
                    });
                }
                use sha2::Digest as _;
                let digest = sha2::Sha256::digest(bytes);
                let mut out = [0u8; DIGEST_BYTES];
                out.copy_from_slice(&digest);
                Ok(out)
            }
        }
    }
}

impl fmt::Display for HashProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// Configured profile selection for parsing (A-9113.2: profile
/// dispatch is static/configured parsing). A content ID whose
/// profile is not admitted rejects as
/// [`IdShapeError::UnsupportedProfile`]; shapes and versions are
/// not policy-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsePolicy {
    /// Admit `raw-blake3-256` content IDs.
    pub raw_blake3_256: bool,
    /// Admit `atlas-chunk-sha256-utf8` content IDs.
    pub atlas_chunk_sha256_utf8: bool,
}

impl ParsePolicy {
    /// Admit every known profile.
    pub const ALL: Self = Self {
        raw_blake3_256: true,
        atlas_chunk_sha256_utf8: true,
    };

    /// Admit exactly one content profile.
    pub fn only(profile: HashProfile) -> Self {
        let mut policy = Self {
            raw_blake3_256: false,
            atlas_chunk_sha256_utf8: false,
        };
        match profile {
            HashProfile::RawBlake3_256 => policy.raw_blake3_256 = true,
            HashProfile::AtlasChunkSha256Utf8 => policy.atlas_chunk_sha256_utf8 = true,
        }
        policy
    }

    /// Whether this policy admits `profile`.
    pub fn admits(&self, profile: HashProfile) -> bool {
        match profile {
            HashProfile::RawBlake3_256 => self.raw_blake3_256,
            HashProfile::AtlasChunkSha256Utf8 => self.atlas_chunk_sha256_utf8,
        }
    }
}

impl Default for ParsePolicy {
    fn default() -> Self {
        Self::ALL
    }
}

/// Errors of shape parsing, construction and verification — one
/// variant per error class the design names (A-9113.1/A-9113.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdShapeError {
    /// The input is not a well-formed encoding of any known shape:
    /// truncation, trailing bytes, bad lengths, non-canonical text.
    Malformed {
        /// What was wrong with the input.
        detail: String,
    },
    /// The shape tag is not a known shape. Shape tags are critical:
    /// an unknown tag is never guessed at or skipped.
    UnsupportedShape {
        /// The unknown shape tag byte.
        tag: u8,
    },
    /// The shape is known but its encoding version is not.
    UnsupportedVersion {
        /// The shape whose version was rejected.
        shape: IdShape,
        /// The version byte found.
        version: u8,
    },
    /// The content profile (tag, token, or policy exclusion) is
    /// not admitted by this parser.
    UnsupportedProfile {
        /// The profile tag or token that was rejected.
        profile: String,
    },
    /// The derived-kind tag is not a registered derived family.
    UnsupportedDerivedKind {
        /// The unknown derived-kind tag byte.
        tag: u8,
    },
    /// A recomputed content digest does not match the ID.
    DigestMismatch,
    /// The input cannot be digested under the named profile
    /// (for example non-UTF-8 bytes under the Atlas text profile).
    InvalidProfileParameters {
        /// The profile whose parameters were invalid.
        profile: HashProfile,
        /// What was invalid.
        detail: &'static str,
    },
    /// A configured resource limit was exceeded.
    ResourceLimit {
        /// The limit that was exceeded.
        limit: &'static str,
        /// The actual size or count.
        actual: usize,
    },
}

impl fmt::Display for IdShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdShapeError::Malformed { detail } => {
                write!(f, "malformed ID encoding: {detail}")
            }
            IdShapeError::UnsupportedShape { tag } => {
                write!(f, "unsupported ID shape tag 0x{tag:02x}")
            }
            IdShapeError::UnsupportedVersion { shape, version } => {
                write!(f, "unsupported {shape} ID version {version}")
            }
            IdShapeError::UnsupportedProfile { profile } => {
                write!(f, "unsupported content profile {profile}")
            }
            IdShapeError::UnsupportedDerivedKind { tag } => {
                write!(f, "unsupported derived ID kind tag 0x{tag:02x}")
            }
            IdShapeError::DigestMismatch => f.write_str("content digest does not match the ID"),
            IdShapeError::InvalidProfileParameters { profile, detail } => {
                write!(f, "invalid parameters for profile {profile}: {detail}")
            }
            IdShapeError::ResourceLimit { limit, actual } => {
                write!(f, "resource limit exceeded: {limit} (actual {actual})")
            }
        }
    }
}

impl std::error::Error for IdShapeError {}

fn malformed(detail: impl Into<String>) -> IdShapeError {
    IdShapeError::Malformed {
        detail: detail.into(),
    }
}

/// A content-addressed ID: a hash profile plus its 32-byte digest.
/// The digest names content independently of where copies live
/// (S3.10) and is verifiable by recomputation (S3.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentId {
    profile: HashProfile,
    digest: [u8; DIGEST_BYTES],
}

impl ContentId {
    /// Build a content ID from an explicit profile and digest.
    pub fn new(profile: HashProfile, digest: [u8; DIGEST_BYTES]) -> Self {
        Self { profile, digest }
    }

    /// Compute the content ID of `bytes` under `profile`.
    pub fn of_bytes(profile: HashProfile, bytes: &[u8]) -> Result<Self, IdShapeError> {
        Ok(Self {
            profile,
            digest: profile.digest(bytes)?,
        })
    }

    /// The hash profile this ID dispatches on.
    pub fn profile(&self) -> HashProfile {
        self.profile
    }

    /// The raw digest bytes.
    pub fn digest(&self) -> &[u8; DIGEST_BYTES] {
        &self.digest
    }

    /// Verify `bytes` against this ID by recomputation — the only
    /// verification there is; there is no lookup shortcut
    /// (A-9113.2: no "unknown digest equals absent").
    pub fn verify(&self, bytes: &[u8]) -> Result<(), IdShapeError> {
        let computed = self.profile.digest(bytes)?;
        if computed == self.digest {
            Ok(())
        } else {
            Err(IdShapeError::DigestMismatch)
        }
    }
}

/// An array-addressed ID: an ordered list of slots, each null or
/// bytes (the shape-level reference form of the #9118 array core).
/// `None` (null), `Some(vec![])` (empty bytes) and an absent slot
/// are three distinct states.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArrayId {
    slots: Vec<Option<Vec<u8>>>,
}

impl ArrayId {
    /// Build an array ID from its slots, enforcing the resource
    /// limits so every constructed value has a parseable encoding.
    pub fn new(slots: Vec<Option<Vec<u8>>>) -> Result<Self, IdShapeError> {
        check_array_limits(&slots)?;
        Ok(Self { slots })
    }

    /// The slots, in order.
    pub fn slots(&self) -> &[Option<Vec<u8>>] {
        &self.slots
    }
}

fn check_array_limits(slots: &[Option<Vec<u8>>]) -> Result<(), IdShapeError> {
    if slots.len() > MAX_ARRAY_SLOTS {
        return Err(IdShapeError::ResourceLimit {
            limit: "array slot count",
            actual: slots.len(),
        });
    }
    for slot in slots.iter().flatten() {
        if slot.len() > MAX_SLOT_BYTES {
            return Err(IdShapeError::ResourceLimit {
                limit: "array slot bytes",
                actual: slot.len(),
            });
        }
    }
    Ok(())
}

/// A registered derived family (derived namespaces/lenses over the
/// two cores). The registry is finite; adding a family is additive
/// and changes no existing interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DerivedKind {
    /// String-path names (#9121).
    Name,
    /// Numeric registry family (#9122).
    Numeric,
    /// Addressable live streams (#9123).
    Stream,
    /// Signed coordinate worlds (design V4).
    SignedWorld,
}

impl DerivedKind {
    /// The kind token used in text forms.
    pub fn token(self) -> &'static str {
        match self {
            DerivedKind::Name => "name",
            DerivedKind::Numeric => "numeric",
            DerivedKind::Stream => "stream",
            DerivedKind::SignedWorld => "signed-world",
        }
    }

    /// Strict token parsing: exact match only.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "name" => Some(DerivedKind::Name),
            "numeric" => Some(DerivedKind::Numeric),
            "stream" => Some(DerivedKind::Stream),
            "signed-world" => Some(DerivedKind::SignedWorld),
            _ => None,
        }
    }

    /// The binary kind tag.
    pub fn tag(self) -> u8 {
        match self {
            DerivedKind::Name => DERIVED_TAG_NAME,
            DerivedKind::Numeric => DERIVED_TAG_NUMERIC,
            DerivedKind::Stream => DERIVED_TAG_STREAM,
            DerivedKind::SignedWorld => DERIVED_TAG_SIGNED_WORLD,
        }
    }

    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            DERIVED_TAG_NAME => Some(DerivedKind::Name),
            DERIVED_TAG_NUMERIC => Some(DerivedKind::Numeric),
            DERIVED_TAG_STREAM => Some(DerivedKind::Stream),
            DERIVED_TAG_SIGNED_WORLD => Some(DerivedKind::SignedWorld),
            _ => None,
        }
    }
}

impl fmt::Display for DerivedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.token())
    }
}

/// A derived ID: a registered derived family plus its canonical
/// payload bytes. The payload's internal semantics belong to the
/// family's own slice; this layer carries it losslessly. A `Name`
/// payload is a UTF-8 path by family rule and is validated as such.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DerivedId {
    kind: DerivedKind,
    payload: Vec<u8>,
}

impl DerivedId {
    /// Build a derived ID, enforcing the resource limit and the
    /// name family's UTF-8 rule.
    pub fn new(kind: DerivedKind, payload: Vec<u8>) -> Result<Self, IdShapeError> {
        if payload.len() > MAX_DERIVED_PAYLOAD_BYTES {
            return Err(IdShapeError::ResourceLimit {
                limit: "derived payload bytes",
                actual: payload.len(),
            });
        }
        if kind == DerivedKind::Name && std::str::from_utf8(&payload).is_err() {
            return Err(malformed("name derived payload is not valid UTF-8"));
        }
        Ok(Self { kind, payload })
    }

    /// The derived family.
    pub fn kind(&self) -> DerivedKind {
        self.kind
    }

    /// The canonical payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// An ID of any admitted shape — the tagged sum consumers dispatch
/// on (A-9113.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Id {
    /// Content-addressed ID.
    Content(ContentId),
    /// Array-addressed ID.
    Array(ArrayId),
    /// Derived-family ID.
    Derived(DerivedId),
}

impl Id {
    /// The shape discriminator of this ID.
    pub fn shape(&self) -> IdShape {
        match self {
            Id::Content(_) => IdShape::Content,
            Id::Array(_) => IdShape::Array,
            Id::Derived(_) => IdShape::Derived,
        }
    }

    /// The encoding version of this ID.
    pub fn version(&self) -> u8 {
        ID_SHAPE_VERSION
    }

    /// Encode to the canonical binary form (see module docs).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![self.shape().tag(), ID_SHAPE_VERSION];
        match self {
            Id::Content(content) => {
                out.push(content.profile.tag());
                out.extend_from_slice(&content.digest);
            }
            Id::Array(array) => {
                out.extend_from_slice(&(array.slots.len() as u32).to_le_bytes());
                for slot in &array.slots {
                    match slot {
                        None => out.push(0x00),
                        Some(bytes) => {
                            out.push(0x01);
                            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                            out.extend_from_slice(bytes);
                        }
                    }
                }
            }
            Id::Derived(derived) => {
                out.push(derived.kind.tag());
                out.extend_from_slice(&(derived.payload.len() as u32).to_le_bytes());
                out.extend_from_slice(&derived.payload);
            }
        }
        out
    }

    /// Parse the canonical binary form under the default policy
    /// (every known profile admitted).
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, IdShapeError> {
        Self::parse_bytes_with(bytes, &ParsePolicy::ALL)
    }

    /// Parse the canonical binary form under a configured profile
    /// policy. Strict: unknown critical tags, unknown versions,
    /// truncation and trailing bytes all reject.
    pub fn parse_bytes_with(bytes: &[u8], policy: &ParsePolicy) -> Result<Self, IdShapeError> {
        if bytes.len() > MAX_ENCODED_BYTES {
            return Err(IdShapeError::ResourceLimit {
                limit: "encoded ID bytes",
                actual: bytes.len(),
            });
        }
        let mut reader = Reader { bytes, pos: 0 };
        let tag = reader.u8("shape tag")?;
        let shape = IdShape::from_tag(tag).ok_or(IdShapeError::UnsupportedShape { tag })?;
        let version = reader.u8("shape version")?;
        if version != ID_SHAPE_VERSION {
            return Err(IdShapeError::UnsupportedVersion { shape, version });
        }
        let id = match shape {
            IdShape::Content => {
                let profile_tag = reader.u8("profile tag")?;
                let profile =
                    HashProfile::from_tag(profile_tag).ok_or(IdShapeError::UnsupportedProfile {
                        profile: format!("tag 0x{profile_tag:02x}"),
                    })?;
                if !policy.admits(profile) {
                    return Err(IdShapeError::UnsupportedProfile {
                        profile: profile.token().to_string(),
                    });
                }
                let digest_bytes = reader.take(DIGEST_BYTES, "content digest")?;
                let mut digest = [0u8; DIGEST_BYTES];
                digest.copy_from_slice(digest_bytes);
                Id::Content(ContentId::new(profile, digest))
            }
            IdShape::Array => {
                let count = reader.u32("array slot count")? as usize;
                if count > MAX_ARRAY_SLOTS {
                    return Err(IdShapeError::ResourceLimit {
                        limit: "array slot count",
                        actual: count,
                    });
                }
                let mut slots = Vec::with_capacity(count);
                for _ in 0..count {
                    match reader.u8("array slot marker")? {
                        0x00 => slots.push(None),
                        0x01 => {
                            let len = reader.u32("array slot length")? as usize;
                            if len > MAX_SLOT_BYTES {
                                return Err(IdShapeError::ResourceLimit {
                                    limit: "array slot bytes",
                                    actual: len,
                                });
                            }
                            slots.push(Some(reader.take(len, "array slot bytes")?.to_vec()));
                        }
                        other => {
                            return Err(malformed(format!(
                                "unknown array slot marker 0x{other:02x}"
                            )));
                        }
                    }
                }
                Id::Array(ArrayId { slots })
            }
            IdShape::Derived => {
                let kind_tag = reader.u8("derived kind tag")?;
                let kind = DerivedKind::from_tag(kind_tag)
                    .ok_or(IdShapeError::UnsupportedDerivedKind { tag: kind_tag })?;
                let len = reader.u32("derived payload length")? as usize;
                if len > MAX_DERIVED_PAYLOAD_BYTES {
                    return Err(IdShapeError::ResourceLimit {
                        limit: "derived payload bytes",
                        actual: len,
                    });
                }
                let payload = reader.take(len, "derived payload")?.to_vec();
                Id::Derived(DerivedId::new(kind, payload)?)
            }
        };
        if reader.pos != bytes.len() {
            return Err(malformed("trailing bytes after complete ID"));
        }
        Ok(id)
    }
}

/// Design-shaped free-function spelling of [`Id::parse_bytes_with`]
/// (A-9113.2 API: `parse_id(bytes, profile_policy) -> Id | IdError`).
pub fn parse_id_bytes(bytes: &[u8], profile_policy: &ParsePolicy) -> Result<Id, IdShapeError> {
    Id::parse_bytes_with(bytes, profile_policy)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize, what: &str) -> Result<&'a [u8], IdShapeError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| malformed(format!("{what} length overflows the input")))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| malformed(format!("truncated ID encoding at {what}")))?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self, what: &str) -> Result<u8, IdShapeError> {
        Ok(self.take(1, what)?[0])
    }

    fn u32(&mut self, what: &str) -> Result<u32, IdShapeError> {
        let bytes = self.take(4, what)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

impl Id {
    /// Encode to the canonical text form (see module docs).
    pub fn to_text(&self) -> String {
        match self {
            Id::Content(content) => format!(
                "content/{}.{}",
                content.profile.token(),
                hex_encode(&content.digest)
            ),
            Id::Array(array) => {
                let mut out = String::from("array");
                for slot in &array.slots {
                    match slot {
                        None => out.push_str("/~"),
                        Some(bytes) => {
                            out.push_str("/b.");
                            out.push_str(&percent_encode(bytes));
                        }
                    }
                }
                out
            }
            Id::Derived(derived) => format!(
                "derived/{}/v{}.{}",
                derived.kind.token(),
                ID_SHAPE_VERSION,
                hex_encode(&derived.payload)
            ),
        }
    }

    /// Parse the canonical text form under the default policy.
    pub fn parse_text(text: &str) -> Result<Self, IdShapeError> {
        Self::parse_text_with(text, &ParsePolicy::ALL)
    }

    /// Parse the canonical text form under a configured profile
    /// policy. Strict: only the exact canonical spellings admit —
    /// uppercase hex, lowercase percent escapes, redundant escapes,
    /// unknown shape/profile/kind tokens and stray characters all
    /// reject.
    pub fn parse_text_with(text: &str, policy: &ParsePolicy) -> Result<Self, IdShapeError> {
        if text.len() > MAX_ENCODED_BYTES {
            return Err(IdShapeError::ResourceLimit {
                limit: "encoded ID bytes",
                actual: text.len(),
            });
        }
        if let Some(rest) = text.strip_prefix("content/") {
            let (token, hex) = rest
                .split_once('.')
                .ok_or_else(|| malformed("content text lacks the profile/digest '.'"))?;
            let profile =
                HashProfile::from_token(token).ok_or(IdShapeError::UnsupportedProfile {
                    profile: token.to_string(),
                })?;
            if !policy.admits(profile) {
                return Err(IdShapeError::UnsupportedProfile {
                    profile: token.to_string(),
                });
            }
            let digest_vec = hex_decode(hex, "content digest")?;
            let digest: [u8; DIGEST_BYTES] = digest_vec
                .try_into()
                .map_err(|_| malformed("content digest is not 32 bytes"))?;
            return Ok(Id::Content(ContentId::new(profile, digest)));
        }
        if text == "array" || text.starts_with("array/") {
            let rest = &text["array".len()..];
            let mut slots = Vec::new();
            if !rest.is_empty() {
                for segment in rest[1..].split('/') {
                    if segment == "~" {
                        slots.push(None);
                    } else if let Some(encoded) = segment.strip_prefix("b.") {
                        slots.push(Some(percent_decode(encoded)?));
                    } else {
                        return Err(malformed(format!("unknown array slot segment {segment:?}")));
                    }
                }
            }
            return Ok(Id::Array(ArrayId::new(slots)?));
        }
        if let Some(rest) = text.strip_prefix("derived/") {
            let (token, rest) = rest
                .split_once('/')
                .ok_or_else(|| malformed("derived text lacks the kind/version '/'"))?;
            let kind = DerivedKind::from_token(token)
                .ok_or_else(|| malformed(format!("unknown derived kind {token:?}")))?;
            let version_prefix = format!("v{ID_SHAPE_VERSION}.");
            let hex = rest.strip_prefix(&version_prefix).ok_or_else(|| {
                if let Some(v) = rest.strip_prefix('v') {
                    let version = v
                        .split('.')
                        .next()
                        .and_then(|n| n.parse::<u8>().ok())
                        .unwrap_or(0);
                    IdShapeError::UnsupportedVersion {
                        shape: IdShape::Derived,
                        version,
                    }
                } else {
                    malformed("derived text lacks a version")
                }
            })?;
            let payload = hex_decode(hex, "derived payload")?;
            return Ok(Id::Derived(DerivedId::new(kind, payload)?));
        }
        Err(malformed(
            "text ID does not start with a known shape token (content/, array, derived/)",
        ))
    }
}

/// Design-shaped free-function spelling of [`Id::parse_text_with`]
/// (A-9113.2 API: `parse_id(text, profile_policy) -> Id | IdError`).
pub fn parse_id_text(text: &str, profile_policy: &ParsePolicy) -> Result<Id, IdShapeError> {
    Id::parse_text_with(text, profile_policy)
}

/// Design-shaped free-function spelling of [`Id::to_text`]
/// (A-9113.2 API: `format_id(Id, encoding_profile) -> text`).
pub fn format_id(id: &Id) -> String {
    id.to_text()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(text: &str, what: &str) -> Result<Vec<u8>, IdShapeError> {
    if !text.len().is_multiple_of(2) {
        return Err(malformed(format!("{what} hex has odd length")));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let hi = hex_value(pair[0], what)?;
        let lo = hex_value(pair[1], what)?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_value(byte: u8, what: &str) -> Result<u8, IdShapeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(malformed(format!(
            "{what} is not lowercase hex (byte 0x{byte:02x})"
        ))),
    }
}

/// RFC 3986 unreserved octets — the only bytes the #9117 slot
/// grammar spells literally.
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode(text: &str) -> Result<Vec<u8>, IdShapeError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let pair = bytes
                    .get(i + 1..i + 3)
                    .ok_or_else(|| malformed("truncated percent escape in array slot"))?;
                let hi = percent_value(pair[0])?;
                let lo = percent_value(pair[1])?;
                let byte = (hi << 4) | lo;
                if is_unreserved(byte) {
                    return Err(malformed(
                        "redundant percent escape for an unreserved octet",
                    ));
                }
                out.push(byte);
                i += 3;
            }
            byte if is_unreserved(byte) => {
                out.push(byte);
                i += 1;
            }
            byte => {
                return Err(malformed(format!(
                    "byte 0x{byte:02x} must be percent-encoded in an array slot"
                )));
            }
        }
    }
    if out.len() > MAX_SLOT_BYTES {
        return Err(IdShapeError::ResourceLimit {
            limit: "array slot bytes",
            actual: out.len(),
        });
    }
    Ok(out)
}

fn percent_value(byte: u8) -> Result<u8, IdShapeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(malformed(format!(
            "percent escape is not uppercase hex (byte 0x{byte:02x})"
        ))),
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&Id::Content(*self).to_text())
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}
