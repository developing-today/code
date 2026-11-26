//! #8479 secret-bearing remediation: no-write inventory and destination
//! verifier (issue #12406).
//!
//! This is the read-only half of the remediation order of operations in
//! `doc/2026-11-21T21-32-00Z_design_atlas_classification_model_v1.md` §6:
//! inventory the reported chunks, produce a no-write source-to-destination
//! mapping with checksums, and verify the secret-tier destination's content
//! and vectors *before* any visibility switch. Anything that cannot be
//! verified is reported explicitly and never treated as safe to purge.
//!
//! Guarantees this module is built to keep (design §8, items 9 and 10):
//!
//! * No corpus text is read or reproduced: only ids, content hashes,
//!   sensitivity, and vector bytes (hashed in memory) are queried.
//! * No write ever reaches a shard: every query is a `SELECT`.
//! * The run record is trace-safe: identifiers, checksums and counts only.
//! * A run is resumable: [`validate_resume`] pins the canonical endpoints and
//!   source set, then compares a separate destination fingerprint before the
//!   prior run is reused.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::shards::ShardTransport;
use crate::types::{DEFAULT_VEC_TABLE, Row, Stmt, Value};

/// Format marker for a persisted inventory run.
pub const INVENTORY_FORMAT: &str = "atlas-remediation-inventory/v3";
const SHA256_HEX_LENGTH: usize = 64;
const VECTOR_BYTES: usize = 768 * 4;

/// Serialized run record. Deliberately contains no chunk text: only ids,
/// content hashes and counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryReport {
    pub format: String,
    /// Stable identity of the endpoints and source ID/hash set.
    pub run_id: String,
    pub source: String,
    pub destination: String,
    /// The sensitivity label selected on the source (always
    /// `secret-bearing` for #8479, but kept explicit so the tool cannot
    /// silently inventory a different class).
    pub sensitivity: String,
    /// Snapshot of the complete destination id/hash/vector set. It is kept
    /// separate from `run_id`, which identifies the stable source and endpoints.
    pub destination_fingerprint: String,
    pub counts: InventoryCounts,
    pub mappings: Vec<MappingEntry>,
    /// Source chunk ids with no verified destination copy. These must stay
    /// reported as unresolved; they are never a reason to expose the
    /// source.
    pub unverified_source_ids: Vec<i64>,
    /// Destination chunk ids with no source counterpart (informational).
    pub destination_only_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryCounts {
    pub source: u64,
    pub destination: u64,
    pub verified: u64,
    pub missing_destination: u64,
    pub hash_mismatch: u64,
    pub missing_vector: u64,
    pub vector_mismatch: u64,
}

/// One source chunk and the destination record it maps to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingEntry {
    pub source_chunk_id: i64,
    pub source_content_hash: String,
    /// `None` when no destination chunk carries this content hash.
    pub destination_chunk_id: Option<i64>,
    pub destination_content_hash: Option<String>,
    pub source_vector_fingerprint: Option<String>,
    pub destination_vector_fingerprint: Option<String>,
    pub status: MappingStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingStatus {
    /// Destination carries the same content hash and a vector row.
    Verified,
    /// No destination chunk carries the content hash.
    MissingDestination,
    /// A destination chunk exists under the same id but with a different
    /// content hash (and no other destination chunk matches).
    HashMismatch,
    /// The content is present but the vector row is missing.
    MissingVector,
    /// The source and destination have different vector bytes.
    VectorMismatch,
}

impl InventoryReport {
    /// True when every inventoried source chunk is verified at the
    /// destination. A caller must require this before any visibility switch
    /// or purge (design §6).
    pub fn destination_complete(&self) -> bool {
        self.unverified_source_ids.is_empty()
            && self.counts.source == self.counts.verified
            && self.counts.missing_destination == 0
            && self.counts.hash_mismatch == 0
            && self.counts.missing_vector == 0
            && self.counts.vector_mismatch == 0
    }

    /// The post-run absence predicate: after a purge, the source must return
    /// no rows for the inventoried sensitivity. Kept here so the check is
    /// the same function the run report is built from.
    pub fn absence_holds(remaining: usize) -> bool {
        remaining == 0
    }
}

/// A source chunk selected by sensitivity label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChunk {
    pub id: i64,
    pub content_hash: String,
    pub vector_fingerprint: Option<String>,
}

/// A destination chunk with a fingerprint of its vector bytes, when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationChunk {
    pub id: i64,
    pub content_hash: String,
    pub vector_fingerprint: Option<String>,
}

/// Read the labelled source set. Text is never fetched and vector bytes are
/// only hashed in memory.
pub fn read_source(
    transport: &dyn ShardTransport,
    sensitivity: Sensitivity,
) -> Result<Vec<SourceChunk>> {
    let sql = format!(
        "WITH classified AS ( \
           SELECT c.id AS id, c.content_hash AS h, v.vec AS vec, \
                  CASE \
                    WHEN c.sensitivity IN ('shareable','personal','local-only','secret-bearing') THEN c.sensitivity \
                    WHEN c.sensitivity IS NOT NULL THEN 'secret-bearing' \
                    WHEN l.sensitivity IN ('shareable','personal','local-only','secret-bearing') THEN l.sensitivity \
                    ELSE 'secret-bearing' \
                  END AS sensitivity \
           FROM chunks c \
           LEFT JOIN messages m ON m.id = c.message_id \
           LEFT JOIN session_labels l ON l.session_id = m.session_id \
           LEFT JOIN {DEFAULT_VEC_TABLE} v ON v.chunk_id = c.id \
         ), selected AS ( \
           SELECT id, h, vec FROM classified WHERE sensitivity = ?1 \
         ), counts AS (SELECT COUNT(*) AS total FROM selected) \
         SELECT counts.total AS total, selected.id AS id, selected.h AS h, selected.vec AS vec \
         FROM counts LEFT JOIN selected ON 1=1 ORDER BY selected.id"
    );
    let mut results = transport
        .pipeline(&[Stmt::new(sql, vec![Value::text(sensitivity.name())])])?
        .into_iter();
    let rows = results
        .next()
        .ok_or_else(|| Error::backend("remediation: source statement result missing"))?;
    if results.next().is_some() {
        return Err(Error::backend(
            "remediation: source returned extra statement results",
        ));
    }
    let first = rows
        .first()
        .ok_or_else(|| Error::backend("remediation: source statement rows missing"))?;
    let count = first
        .get_i64("total")
        .filter(|count| *count >= 0)
        .ok_or_else(|| Error::backend("remediation: source count result missing or invalid"))?
        as u64;
    if count == 0 {
        if rows.len() == 1
            && is_null_column(first, "id")
            && is_null_column(first, "h")
            && is_null_column(first, "vec")
        {
            return Ok(Vec::new());
        }
        return Err(Error::backend("remediation: malformed empty source result"));
    }
    if count != rows.len() as u64 {
        return Err(Error::backend(format!(
            "remediation: source count {count} disagrees with {} returned rows",
            rows.len()
        )));
    }
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        if r.get_i64("total") != Some(count as i64) {
            return Err(Error::backend(
                "remediation: source result count changed within snapshot",
            ));
        }
        let id = r
            .get_i64("id")
            .ok_or_else(|| Error::backend("remediation: source chunk id unreadable"))?;
        let content_hash = r
            .get_text("h")
            .ok_or_else(|| Error::backend(format!("remediation: source chunk {id} has no hash")))?
            .to_string();
        if !is_sha256_hex(&content_hash) {
            return Err(Error::backend(format!(
                "remediation: source chunk {id} has an invalid SHA-256 content_hash"
            )));
        }
        out.push(SourceChunk {
            id,
            content_hash,
            vector_fingerprint: fingerprint_vector(r, id, "source")?,
        });
    }
    Ok(out)
}

/// Read every destination chunk with vector bytes hashed in memory.
/// Validate the vector table as a plain SQL identifier before interpolation.
pub fn read_destination(
    transport: &dyn ShardTransport,
    vec_table: &str,
) -> Result<Vec<DestinationChunk>> {
    if !is_valid_vector_table_identifier(vec_table) {
        return Err(Error::fatal(format!(
            "remediation: invalid vector table identifier `{vec_table}`"
        )));
    }
    let sql = format!(
        "WITH selected AS ( \
           SELECT c.id AS id, c.content_hash AS h, v.vec AS vec \
           FROM chunks c LEFT JOIN {vec_table} v ON v.chunk_id = c.id \
         ), counts AS (SELECT COUNT(*) AS total FROM selected) \
         SELECT counts.total AS total, selected.id AS id, selected.h AS h, selected.vec AS vec \
         FROM counts LEFT JOIN selected ON 1=1 ORDER BY selected.id"
    );
    let mut results = transport.pipeline(&[Stmt::new(sql, vec![])])?.into_iter();
    let rows = results
        .next()
        .ok_or_else(|| Error::backend("remediation: destination statement result missing"))?;
    if results.next().is_some() {
        return Err(Error::backend(
            "remediation: destination returned extra statement results",
        ));
    }
    let first = rows
        .first()
        .ok_or_else(|| Error::backend("remediation: destination statement rows missing"))?;
    let count = first
        .get_i64("total")
        .filter(|count| *count >= 0)
        .ok_or_else(|| Error::backend("remediation: destination count result missing or invalid"))?
        as u64;
    if count == 0 {
        if rows.len() == 1
            && is_null_column(first, "id")
            && is_null_column(first, "h")
            && is_null_column(first, "vec")
        {
            return Ok(Vec::new());
        }
        return Err(Error::backend(
            "remediation: malformed empty destination result",
        ));
    }
    if count != rows.len() as u64 {
        return Err(Error::backend(format!(
            "remediation: destination count {count} disagrees with {} returned rows",
            rows.len()
        )));
    }
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        if r.get_i64("total") != Some(count as i64) {
            return Err(Error::backend(
                "remediation: destination result count changed within snapshot",
            ));
        }
        let id = r
            .get_i64("id")
            .ok_or_else(|| Error::backend("remediation: destination chunk id unreadable"))?;
        let content_hash = r
            .get_text("h")
            .ok_or_else(|| {
                Error::backend(format!("remediation: destination chunk {id} has no hash"))
            })?
            .to_string();
        if !is_sha256_hex(&content_hash) {
            return Err(Error::backend(format!(
                "remediation: destination chunk {id} has an invalid SHA-256 content_hash"
            )));
        }
        out.push(DestinationChunk {
            id,
            content_hash,
            vector_fingerprint: fingerprint_vector(r, id, "destination")?,
        });
    }
    Ok(out)
}

/// Build the no-write mapping and report. Both reads are `SELECT`s; nothing
/// is written to either shard.
pub fn inventory(
    source: &dyn ShardTransport,
    destination: &dyn ShardTransport,
    source_name: &str,
    destination_name: &str,
    sensitivity: Sensitivity,
    vec_table: &str,
) -> Result<InventoryReport> {
    let src = read_source(source, sensitivity)?;
    let dst = read_destination(destination, vec_table)?;
    build_report(source_name, destination_name, sensitivity, &src, &dst)
}

/// Pure mapping step, split out so it can be tested without transports.
pub fn build_report(
    source_name: &str,
    destination_name: &str,
    sensitivity: Sensitivity,
    src: &[SourceChunk],
    dst: &[DestinationChunk],
) -> Result<InventoryReport> {
    for chunk in src {
        if !is_sha256_hex(&chunk.content_hash) {
            return Err(Error::backend(format!(
                "remediation: source chunk {} has an invalid SHA-256 content_hash",
                chunk.id
            )));
        }
    }
    for chunk in dst {
        if !is_sha256_hex(&chunk.content_hash) {
            return Err(Error::backend(format!(
                "remediation: destination chunk {} has an invalid SHA-256 content_hash",
                chunk.id
            )));
        }
    }
    // Content hash -> destination rows (ids are unique; hashes are unique in
    // the chunk schema, but a map of the first wins defensively).
    let mut by_hash: BTreeMap<&str, &DestinationChunk> = BTreeMap::new();
    for d in dst {
        by_hash.entry(d.content_hash.as_str()).or_insert(d);
    }
    // Destination id lookup for the id-preserving case, so a mismatch can be
    // reported as such rather than as a plain miss.
    let by_id: BTreeMap<i64, &DestinationChunk> = dst.iter().map(|d| (d.id, d)).collect();

    let mut counts = InventoryCounts {
        source: src.len() as u64,
        destination: dst.len() as u64,
        ..InventoryCounts::default()
    };
    let mut mappings = Vec::with_capacity(src.len());
    let mut unverified_source_ids = Vec::new();
    let mut consumed: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();

    for s in src {
        let (entry, status) = match by_hash.get(s.content_hash.as_str()) {
            Some(d) => {
                consumed.insert(d.id);
                let status = match (&s.vector_fingerprint, &d.vector_fingerprint) {
                    (Some(source), Some(destination)) if source == destination => {
                        MappingStatus::Verified
                    }
                    (Some(_), Some(_)) => MappingStatus::VectorMismatch,
                    _ => MappingStatus::MissingVector,
                };
                (
                    MappingEntry {
                        source_chunk_id: s.id,
                        source_content_hash: s.content_hash.clone(),
                        destination_chunk_id: Some(d.id),
                        destination_content_hash: Some(d.content_hash.clone()),
                        source_vector_fingerprint: s.vector_fingerprint.clone(),
                        destination_vector_fingerprint: d.vector_fingerprint.clone(),
                        status,
                    },
                    status,
                )
            }
            None => match by_id.get(&s.id) {
                Some(d) => {
                    consumed.insert(d.id);
                    (
                        MappingEntry {
                            source_chunk_id: s.id,
                            source_content_hash: s.content_hash.clone(),
                            destination_chunk_id: Some(d.id),
                            destination_content_hash: Some(d.content_hash.clone()),
                            source_vector_fingerprint: s.vector_fingerprint.clone(),
                            destination_vector_fingerprint: d.vector_fingerprint.clone(),
                            status: MappingStatus::HashMismatch,
                        },
                        MappingStatus::HashMismatch,
                    )
                }
                None => (
                    MappingEntry {
                        source_chunk_id: s.id,
                        source_content_hash: s.content_hash.clone(),
                        destination_chunk_id: None,
                        destination_content_hash: None,
                        source_vector_fingerprint: s.vector_fingerprint.clone(),
                        destination_vector_fingerprint: None,
                        status: MappingStatus::MissingDestination,
                    },
                    MappingStatus::MissingDestination,
                ),
            },
        };
        match status {
            MappingStatus::Verified => counts.verified += 1,
            MappingStatus::MissingDestination => {
                counts.missing_destination += 1;
                unverified_source_ids.push(s.id);
            }
            MappingStatus::HashMismatch => {
                counts.hash_mismatch += 1;
                unverified_source_ids.push(s.id);
            }
            MappingStatus::MissingVector => {
                counts.missing_vector += 1;
                unverified_source_ids.push(s.id);
            }
            MappingStatus::VectorMismatch => {
                counts.vector_mismatch += 1;
                unverified_source_ids.push(s.id);
            }
        }
        mappings.push(entry);
    }

    let destination_only_ids = dst
        .iter()
        .filter(|d| !consumed.contains(&d.id))
        .map(|d| d.id)
        .collect();

    let run_id = run_identity(source_name, destination_name, sensitivity, src);
    let destination_fingerprint = destination_fingerprint(dst);

    Ok(InventoryReport {
        format: INVENTORY_FORMAT.to_string(),
        run_id,
        source: source_name.to_string(),
        destination: destination_name.to_string(),
        sensitivity: sensitivity.name().to_string(),
        destination_fingerprint,
        counts,
        mappings,
        unverified_source_ids,
        destination_only_ids,
    })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == SHA256_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_null_column(row: &Row, column: &str) -> bool {
    row.get(column).is_some_and(Value::is_null)
}

fn fingerprint_vector(row: &Row, id: i64, side: &str) -> Result<Option<String>> {
    match row.get("vec") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Blob(bytes)) if bytes.len() == VECTOR_BYTES => {
            let digest = Sha256::digest(bytes);
            Ok(Some(
                digest.iter().map(|byte| format!("{byte:02x}")).collect(),
            ))
        }
        Some(Value::Blob(_)) => Err(Error::backend(format!(
            "remediation: {side} chunk {id} has an invalid vector shape"
        ))),
        Some(_) => Err(Error::backend(format!(
            "remediation: {side} chunk {id} vector bytes are unreadable"
        ))),
    }
}

/// Stable identity over canonical endpoints and source ID/hash rows. The
/// destination snapshot is fingerprinted separately so resumes can reject
/// destination drift without changing the run identity.
fn run_identity(
    source: &str,
    destination: &str,
    sensitivity: Sensitivity,
    source_chunks: &[SourceChunk],
) -> String {
    let mut h = Sha256::new();
    h.update(INVENTORY_FORMAT.as_bytes());
    h.update(b"\0");
    h.update(source.as_bytes());
    h.update(b"\0");
    h.update(destination.as_bytes());
    h.update(b"\0");
    h.update(sensitivity.name().as_bytes());
    let mut source_chunks = source_chunks.iter().collect::<Vec<_>>();
    source_chunks
        .sort_by(|left, right| (left.id, &left.content_hash).cmp(&(right.id, &right.content_hash)));
    for chunk in source_chunks {
        h.update(b"\0");
        h.update(chunk.id.to_string().as_bytes());
        h.update(b":");
        h.update(chunk.content_hash.as_bytes());
        h.update(b":");
        h.update(
            chunk
                .vector_fingerprint
                .as_deref()
                .unwrap_or("missing")
                .as_bytes(),
        );
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn destination_fingerprint(destination: &[DestinationChunk]) -> String {
    let mut destination = destination.iter().collect::<Vec<_>>();
    destination.sort_by(|left, right| {
        (left.id, &left.content_hash, &left.vector_fingerprint).cmp(&(
            right.id,
            &right.content_hash,
            &right.vector_fingerprint,
        ))
    });
    let mut h = Sha256::new();
    h.update(INVENTORY_FORMAT.as_bytes());
    h.update(b"\0destination\0");
    for chunk in destination {
        h.update(chunk.id.to_string().as_bytes());
        h.update(b":");
        h.update(chunk.content_hash.as_bytes());
        h.update(b":");
        h.update(
            chunk
                .vector_fingerprint
                .as_deref()
                .unwrap_or("missing")
                .as_bytes(),
        );
        h.update(b"\0");
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Refuse resume when the source or destination snapshot differs from the
/// record. The stable run id pins endpoints and source content; the separate
/// destination fingerprint detects repair or drift between reads.
pub fn validate_resume(previous: &InventoryReport, fresh: &InventoryReport) -> Result<()> {
    if previous.format != INVENTORY_FORMAT || fresh.format != INVENTORY_FORMAT {
        return Err(Error::fatal(
            "remediation: cannot resume a foreign report format",
        ));
    }
    if previous.run_id != fresh.run_id {
        return Err(Error::fatal(
            "remediation: source identity or source set changed; start a new inventory run",
        ));
    }
    if previous.destination_fingerprint != fresh.destination_fingerprint
        || previous.mappings != fresh.mappings
    {
        return Err(Error::fatal(
            "remediation: destination mapping changed since the saved run; re-inventory before resuming",
        ));
    }
    Ok(())
}

pub fn is_valid_vector_table_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

impl MappingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MappingStatus::Verified => "verified",
            MappingStatus::MissingDestination => "missing_destination",
            MappingStatus::HashMismatch => "hash_mismatch",
            MappingStatus::MissingVector => "missing_vector",
            MappingStatus::VectorMismatch => "vector_mismatch",
        }
    }
}

/// Write the run record as JSON without replacing an existing file. This also
/// prevents an output path that aliases a local source shard from truncating it.
pub fn write_run(path: &Path, report: &InventoryReport) -> Result<()> {
    let json = serde_json::to_vec_pretty(report)
        .map_err(|e| Error::backend(format!("remediation: serialize run: {e}")))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|e| Error::backend(format!("remediation: create {}: {e}", path.display())))?;
    file.write_all(&json)
        .map_err(|e| Error::backend(format!("remediation: write {}: {e}", path.display())))
}

/// Read a previous run record for resume. Refuses malformed or foreign-format
/// records; compare it with a fresh inventory using [`validate_resume`].
pub fn read_run(path: &Path) -> Result<InventoryReport> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::backend(format!("remediation: read {}: {e}", path.display())))?;
    let report: InventoryReport = serde_json::from_slice(&bytes)
        .map_err(|e| Error::backend(format!("remediation: parse {}: {e}", path.display())))?;
    if report.format != INVENTORY_FORMAT {
        return Err(Error::fatal(format!(
            "remediation: {} is format `{}`, expected `{INVENTORY_FORMAT}`",
            path.display(),
            report.format
        )));
    }
    Ok(report)
}
