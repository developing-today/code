//! Legacy metadata read bridge and immutable snapshot migration
//! (slice A9114-snapshot-migration, issue #10270; parent #9114; design
//! `part-a-addressing/versioned-design.md` section "#9114 — Complete
//! metadata sidecars and mutable head", A-9114.1–A-9114.3, statements
//! S3.11–S3.22).
//!
//! The legacy metadata form is the `.meta` JSON document the store's
//! tag system kept before signed sidecars: one JSON object
//! `{version, tags}` whose tags carry a subject name, a key, an
//! optional value, an optional related link — a content hash or a
//! path/name reference; related links are path references, not
//! restricted to hash IDs (S3.21, S3.22) — and creation/modification
//! times. This module is the bridge from that form to the signed
//! sidecar form of [`crate::sidecar_snapshot`]:
//!
//! - [`parse_legacy`] reads the legacy form without modifying the
//!   source bytes. The read is stable: the same bytes always parse
//!   to the same document, before and after any migration.
//! - A migration may only run on a [`LegacySourceView`]: a parsed
//!   document plus evidence that the bytes are one consistent
//!   source snapshot — either two byte-identical reads, or one read
//!   whose digest matches an independently attested digest. A single
//!   unaccompanied read can be parsed and inspected but can never
//!   publish a snapshot: the migration entry point accepts only
//!   `LegacySourceView`, so claiming a consistent source snapshot
//!   without evidence is unrepresentable. A parsed document whose
//!   tags are internally inconsistent (a creation time after its
//!   modification time) is likewise refused as a source view.
//! - [`migrate`] converts one subject's complete legacy view into a
//!   signed [`SidecarEnvelopeV1`] and advances the scope head
//!   through the snapshot slice's compare-and-swap
//!   ([`SidecarHeadV1::genesis`] / [`SidecarHeadV1::advance`]), so
//!   only a consistent source view may advance the head. The legacy
//!   bytes are never modified and no legacy hash or signature is
//!   rewritten: the snapshot is a new signed record whose payload is
//!   the converted view, and the outcome carries the source digest
//!   as provenance.
//! - Migration is idempotent: when the head already points at a
//!   verified snapshot whose view equals the converted source view,
//!   migration reports [`MigrationOutcome::AlreadyMigrated`] and
//!   the head does not advance.
//!
//! The per-subject view conversion is total and deterministic:
//! every legacy tag for the subject appears in the view, grouped by
//! key, each tag carried as an entry map holding its times and, when
//! present, its value and link. Entries within a key are ordered by
//! their canonical CBOR bytes and exact duplicates collapse, so two
//! legacy documents that differ only in tag order migrate to the
//! same view.
//!
//! The crate-root export belongs to slice 9110-02; until that lands
//! the integration test includes this module (and
//! `sidecar_snapshot`) by path.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use id_core::canonical_cbor::{CborError, Value, encode_canonical};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};

use crate::sidecar_snapshot::{
    SidecarBodyV1, SidecarEnvelopeV1, SidecarError, SidecarHeadV1, SnapshotId, encode_view,
};

/// Why a legacy document, source view or migration was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    /// The bytes are not a legacy metadata document this bridge can
    /// read completely: not JSON, not the legacy shape, or carrying
    /// a field outside the legacy schema.
    LegacyJson(String),
    /// Two reads of the legacy source disagree: there is no single
    /// consistent source snapshot to migrate. Carries the BLAKE3
    /// digest of each read.
    InconsistentReads {
        /// Digest of the first read.
        first: [u8; 32],
        /// Digest of the second read.
        second: [u8; 32],
    },
    /// A read does not match the independently attested source
    /// digest.
    DigestMismatch {
        /// The attested digest.
        expected: [u8; 32],
        /// The digest of the bytes actually read.
        actual: [u8; 32],
    },
    /// A legacy tag is internally inconsistent, so the document
    /// cannot be one consistent source view.
    InconsistentTag {
        /// The tag's subject.
        subject: String,
        /// The tag's key.
        key: String,
        /// What was wrong.
        problem: &'static str,
    },
    /// The legacy document carries no tags for the subject, so
    /// there is no complete view to migrate.
    SubjectAbsent {
        /// The subject that was asked for.
        subject: String,
    },
    /// A converted view entry could not be canonically encoded.
    Cbor(CborError),
    /// The snapshot or head operation was rejected.
    Sidecar(SidecarError),
}

impl MigrationError {
    /// A stable failure class.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Self::LegacyJson(_) => "malformed_legacy",
            Self::InconsistentReads { .. } | Self::InconsistentTag { .. } => "inconsistent_source",
            Self::DigestMismatch { .. } => "digest_mismatch",
            Self::SubjectAbsent { .. } => "subject_absent",
            Self::Cbor(_) => "malformed",
            Self::Sidecar(error) => error.class(),
        }
    }
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LegacyJson(problem) => write!(f, "legacy metadata rejected: {problem}"),
            Self::InconsistentReads { .. } => {
                write!(
                    f,
                    "legacy source reads disagree: no consistent source snapshot"
                )
            }
            Self::DigestMismatch { .. } => {
                write!(f, "legacy source does not match the attested digest")
            }
            Self::InconsistentTag {
                subject,
                key,
                problem,
            } => write!(f, "legacy tag {subject}/{key} is inconsistent: {problem}"),
            Self::SubjectAbsent { subject } => {
                write!(f, "legacy document carries no tags for subject {subject}")
            }
            Self::Cbor(error) => write!(f, "view encoding rejected: {error}"),
            Self::Sidecar(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MigrationError {}

impl From<CborError> for MigrationError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<SidecarError> for MigrationError {
    fn from(error: SidecarError) -> Self {
        Self::Sidecar(error)
    }
}

/// The legacy metadata document, mirroring the `.meta` JSON blob
/// the legacy tag system writes (`MetaDoc` in the store's tag
/// module). The bridge reads it under its own types so the legacy
/// schema is pinned here: a field outside this schema rejects the
/// document rather than being silently dropped on conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyMetaDoc {
    /// Schema version of the legacy document.
    pub version: u32,
    /// All legacy metadata tags, across every subject.
    pub tags: Vec<LegacyMetaTag>,
}

/// A single legacy metadata tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyMetaTag {
    /// Subject this tag describes (usually a file name).
    pub subject: String,
    /// Tag key (e.g. `created`, `category`, `author`).
    pub key: String,
    /// Optional value; absent for key-only tags like `pinned`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Optional link to another entity (S3.21).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LegacyMetaLink>,
    /// When this tag was created (unix seconds).
    pub created_at: u64,
    /// When this tag was last modified (unix seconds).
    pub modified_at: u64,
}

/// A legacy link target: a content hash, or a path/name reference.
/// Related links are path references, not restricted to hash IDs
/// (S3.22); both forms are preserved verbatim on conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "target")]
pub enum LegacyMetaLink {
    /// Points to a content hash (hex string).
    Hash(String),
    /// Points to a file name / tag name.
    Name(String),
}

/// The digest a source view is evidenced by: BLAKE3 over the exact
/// legacy bytes — the same hash that addresses the legacy blob in
/// the store. Computing it never modifies the bytes.
#[must_use]
pub fn legacy_source_digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// Reads the legacy metadata form. This is the read bridge only:
/// it carries no consistency evidence, so its result can be
/// inspected and converted for display but cannot be migrated —
/// [`migrate`] requires a [`LegacySourceView`].
///
/// # Errors
///
/// [`MigrationError::LegacyJson`] when the bytes are not a legacy
/// metadata document.
pub fn parse_legacy(bytes: &[u8]) -> Result<LegacyMetaDoc, MigrationError> {
    serde_json::from_slice(bytes).map_err(|error| MigrationError::LegacyJson(error.to_string()))
}

fn validate_doc(doc: &LegacyMetaDoc) -> Result<(), MigrationError> {
    for tag in &doc.tags {
        if tag.created_at > tag.modified_at {
            return Err(MigrationError::InconsistentTag {
                subject: tag.subject.clone(),
                key: tag.key.clone(),
                problem: "created_at is after modified_at",
            });
        }
    }
    Ok(())
}

/// A legacy document plus the evidence that its bytes are one
/// consistent source snapshot. Only values of this type may be
/// migrated; see the module docs for the evidence forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySourceView {
    doc: LegacyMetaDoc,
    source_digest: [u8; 32],
}

impl LegacySourceView {
    fn from_bytes(bytes: &[u8]) -> Result<Self, MigrationError> {
        let doc = parse_legacy(bytes)?;
        validate_doc(&doc)?;
        Ok(Self {
            doc,
            source_digest: legacy_source_digest(bytes),
        })
    }

    /// Builds a source view from two reads of the legacy source.
    /// The reads must be byte-identical: a source that changed
    /// between reads is not one consistent snapshot.
    ///
    /// # Errors
    ///
    /// [`MigrationError::InconsistentReads`] when the reads differ,
    /// or as [`parse_legacy`] / document validation.
    pub fn from_double_read(first: &[u8], second: &[u8]) -> Result<Self, MigrationError> {
        if first != second {
            return Err(MigrationError::InconsistentReads {
                first: legacy_source_digest(first),
                second: legacy_source_digest(second),
            });
        }
        Self::from_bytes(first)
    }

    /// Builds a source view from one read whose digest must match
    /// an independently attested digest (for example the digest the
    /// source was addressed by when it was fetched).
    ///
    /// # Errors
    ///
    /// [`MigrationError::DigestMismatch`] when the read does not
    /// match the attested digest, or as [`parse_legacy`] / document
    /// validation.
    pub fn attested(bytes: &[u8], expected: &[u8; 32]) -> Result<Self, MigrationError> {
        let actual = legacy_source_digest(bytes);
        if actual != *expected {
            return Err(MigrationError::DigestMismatch {
                expected: *expected,
                actual,
            });
        }
        Self::from_bytes(bytes)
    }

    /// The parsed legacy document.
    #[must_use]
    pub const fn doc(&self) -> &LegacyMetaDoc {
        &self.doc
    }

    /// The digest of the exact source bytes this view was evidenced
    /// from, carried into migration outcomes as provenance.
    #[must_use]
    pub const fn source_digest(&self) -> [u8; 32] {
        self.source_digest
    }

    /// The complete metadata view for one subject, in the signed
    /// sidecar payload form: every legacy tag for the subject,
    /// grouped by key, each tag an entry map with its times and,
    /// when present, its value and link. Entries within a key are
    /// ordered by their canonical CBOR bytes and exact duplicates
    /// collapse, so the conversion is deterministic and independent
    /// of legacy tag order.
    ///
    /// # Errors
    ///
    /// [`MigrationError::SubjectAbsent`] when the document carries
    /// no tags for the subject, or [`MigrationError::Cbor`] when an
    /// entry cannot be canonically encoded.
    pub fn subject_view(&self, subject: &str) -> Result<BTreeMap<String, Value>, MigrationError> {
        let mut by_key: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for tag in &self.doc.tags {
            if tag.subject == subject {
                by_key
                    .entry(tag.key.clone())
                    .or_default()
                    .push(tag_entry(tag));
            }
        }
        if by_key.is_empty() {
            return Err(MigrationError::SubjectAbsent {
                subject: subject.to_owned(),
            });
        }
        let mut view = BTreeMap::new();
        for (key, entries) in by_key {
            let mut keyed: Vec<(Vec<u8>, Value)> = Vec::with_capacity(entries.len());
            for entry in entries {
                let canonical = encode_canonical(&entry)?;
                keyed.push((canonical, entry));
            }
            keyed.sort_by(|a, b| a.0.cmp(&b.0));
            keyed.dedup_by(|a, b| a.0 == b.0);
            view.insert(
                key,
                Value::Array(keyed.into_iter().map(|(_, e)| e).collect()),
            );
        }
        Ok(view)
    }
}

/// One legacy tag as a view entry: its times and, when present, its
/// value and link. Absent fields are omitted, never null.
fn tag_entry(tag: &LegacyMetaTag) -> Value {
    // Keep the in-memory map in the same order as the canonical CBOR
    // encoder (shorter text keys sort before longer keys). This makes
    // a decoded snapshot view compare equal to the source view and is
    // required for migration idempotence.
    let mut entries: Vec<(Value, Value)> = Vec::new();
    if let Some(link) = &tag.link {
        let (kind, target) = match link {
            LegacyMetaLink::Hash(target) => ("hash", target),
            LegacyMetaLink::Name(target) => ("name", target),
        };
        entries.push((
            Value::Text("link".to_owned()),
            Value::Map(vec![
                (Value::Text("kind".to_owned()), Value::Text(kind.to_owned())),
                (
                    Value::Text("target".to_owned()),
                    Value::Text(target.clone()),
                ),
            ]),
        ));
    }
    if let Some(value) = &tag.value {
        entries.push((Value::Text("value".to_owned()), Value::Text(value.clone())));
    }
    entries.push((
        Value::Text("created_at".to_owned()),
        Value::UInt(tag.created_at),
    ));
    entries.push((
        Value::Text("modified_at".to_owned()),
        Value::UInt(tag.modified_at),
    ));
    Value::Map(entries)
}

/// The scope and signing parameters a migration publishes under.
/// The legacy subject is a name; its sidecar subject identity is
/// the content ID the caller binds that name to — the bridge never
/// invents one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationParams {
    /// The subject this metadata describes, as a content ID.
    pub subject_content_id: ContentId,
    /// The namespace the snapshot is complete within.
    pub namespace_id: String,
    /// The tenant scope the snapshot is complete within.
    pub tenant_scope: String,
    /// Signing domain and context of the snapshot.
    pub domain_context: String,
    /// Who signs the snapshot and head, as the signer names itself.
    pub signer_id: String,
    /// The explicit profile snapshot payload IDs are computed under.
    pub payload_profile: HashProfile,
    /// Advisory creation time (unix milliseconds) of the snapshot.
    pub created_at: u64,
}

/// The result of a migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// A new signed snapshot was published and the head advanced by
    /// exactly one generation (or was created at genesis).
    Advanced {
        /// The new complete signed snapshot.
        snapshot: SidecarEnvelopeV1,
        /// The head after the compare-and-swap.
        head: SidecarHeadV1,
        /// Digest of the legacy source bytes, as provenance.
        source_digest: [u8; 32],
    },
    /// The head already pointed at a verified snapshot whose view
    /// equals the converted source view; nothing was published and
    /// the head did not move.
    AlreadyMigrated {
        /// The current head, unchanged.
        head: SidecarHeadV1,
        /// The snapshot the head already points at.
        snapshot_id: SnapshotId,
        /// Digest of the legacy source bytes, as provenance.
        source_digest: [u8; 32],
    },
}

/// Migrates one subject's complete legacy view into the signed
/// sidecar form. With no current head this publishes the genesis
/// snapshot; with a current head it publishes the exact successor
/// and advances the head through the compare-and-swap, so lost
/// updates and ABA remain detectable. When the current head's
/// verified snapshot already carries exactly this view, the
/// migration is a no-op ([`MigrationOutcome::AlreadyMigrated`]).
///
/// # Errors
///
/// [`MigrationError`]: the subject is absent from the source, or
/// the snapshot/head operations reject — an unverified or
/// mismatched current record, a stale head, a subject or scope
/// mismatch — classed by [`MigrationError::class`].
pub fn migrate(
    source: &LegacySourceView,
    subject_name: &str,
    params: &MigrationParams,
    current: Option<(&SidecarHeadV1, &SidecarEnvelopeV1)>,
    key: &SigningKey,
) -> Result<MigrationOutcome, MigrationError> {
    let view = source.subject_view(subject_name)?;
    let payload = encode_view(&view)?;
    let source_digest = source.source_digest();
    if let Some((head, current_snapshot)) = current {
        current_snapshot.verify(&ProfileDispatch::default())?;
        if head.snapshot_id != current_snapshot.snapshot_id()?
            || head.generation != current_snapshot.body.generation
        {
            return Err(SidecarError::StaleHead.into());
        }
        if current_snapshot.view()? == view {
            return Ok(MigrationOutcome::AlreadyMigrated {
                head: head.clone(),
                snapshot_id: head.snapshot_id,
                source_digest,
            });
        }
        let generation =
            head.generation
                .checked_add(1)
                .ok_or(SidecarError::GenerationMismatch {
                    expected: head.generation,
                    actual: head.generation,
                })?;
        let body = SidecarBodyV1::for_payload(
            params.subject_content_id,
            params.namespace_id.clone(),
            params.tenant_scope.clone(),
            generation,
            Some(head.snapshot_id),
            params.created_at,
            params.domain_context.clone(),
            params.signer_id.clone(),
            key.verifying_key().to_bytes(),
            params.payload_profile,
            &payload,
        )?;
        let snapshot = SidecarEnvelopeV1::sign(body, payload, key)?;
        let head = head.advance(current_snapshot, &snapshot, key)?;
        Ok(MigrationOutcome::Advanced {
            snapshot,
            head,
            source_digest,
        })
    } else {
        let body = SidecarBodyV1::for_payload(
            params.subject_content_id,
            params.namespace_id.clone(),
            params.tenant_scope.clone(),
            0,
            None,
            params.created_at,
            params.domain_context.clone(),
            params.signer_id.clone(),
            key.verifying_key().to_bytes(),
            params.payload_profile,
            &payload,
        )?;
        let snapshot = SidecarEnvelopeV1::sign(body, payload, key)?;
        let head = SidecarHeadV1::genesis(&snapshot, key)?;
        Ok(MigrationOutcome::Advanced {
            snapshot,
            head,
            source_digest,
        })
    }
}
