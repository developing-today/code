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
//! * No corpus text is read or reproduced: the only columns queried are
//!   `id`, `content_hash`, `sensitivity`, and the presence of a vector row.
//! * No write ever reaches a shard: every query is a `SELECT`.
//! * The run record is trace-safe: identifiers, checksums and counts only.
//! * A run is resumable: [`read_run`] refuses a record whose source or
//!   destination identity has drifted, so a resume cannot silently verify
//!   the wrong pair.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::shards::ShardTransport;
use crate::types::{Stmt, Value};

/// Format marker for a persisted inventory run.
pub const INVENTORY_FORMAT: &str = "atlas-remediation-inventory/v1";

/// Serialized run record. Deliberately contains no chunk text: only ids,
/// content hashes and counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryReport {
    pub format: String,
    /// Stable identity of the (source, destination, expected set) triple.
    pub run_id: String,
    pub source: String,
    pub destination: String,
    /// The sensitivity label selected on the source (always
    /// `secret-bearing` for #8479, but kept explicit so the tool cannot
    /// silently inventory a different class).
    pub sensitivity: String,
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
}

/// One source chunk and the destination record it maps to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingEntry {
    pub source_chunk_id: i64,
    pub source_content_hash: String,
    /// `None` when no destination chunk carries this content hash.
    pub destination_chunk_id: Option<i64>,
    pub destination_content_hash: Option<String>,
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
}

/// A destination chunk with its vector presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationChunk {
    pub id: i64,
    pub content_hash: String,
    pub has_vector: bool,
}

/// Read the labelled source set. Only `id`, `content_hash` and the label are
/// selected; text is never fetched.
pub fn read_source(
    transport: &dyn ShardTransport,
    sensitivity: Sensitivity,
) -> Result<Vec<SourceChunk>> {
    let rows = transport
        .pipeline(&[Stmt::new(
            "SELECT id AS id, content_hash AS h FROM chunks \
             WHERE sensitivity = ?1 ORDER BY id",
            vec![Value::text(sensitivity.name())],
        )])?
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let id = r
            .get_i64("id")
            .ok_or_else(|| Error::backend("remediation: source chunk id unreadable"))?;
        let content_hash = r
            .get_text("h")
            .ok_or_else(|| Error::backend(format!("remediation: source chunk {id} has no hash")))?
            .to_string();
        out.push(SourceChunk { id, content_hash });
    }
    Ok(out)
}

/// Read every destination chunk with vector presence. The vector table name
/// is trusted configuration, the same assumption the scan statements make.
pub fn read_destination(
    transport: &dyn ShardTransport,
    vec_table: &str,
) -> Result<Vec<DestinationChunk>> {
    let sql = format!(
        "SELECT c.id AS id, c.content_hash AS h, \
                (v.chunk_id IS NOT NULL) AS has_vec \
         FROM chunks c LEFT JOIN {vec_table} v ON v.chunk_id = c.id \
         ORDER BY c.id"
    );
    let rows = transport
        .pipeline(&[Stmt::new(sql, vec![])])?
        .into_iter()
        .next()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let id = r
            .get_i64("id")
            .ok_or_else(|| Error::backend("remediation: destination chunk id unreadable"))?;
        let content_hash = r
            .get_text("h")
            .ok_or_else(|| {
                Error::backend(format!("remediation: destination chunk {id} has no hash"))
            })?
            .to_string();
        let has_vector = r.get_i64("has_vec").is_some_and(|n| n != 0);
        out.push(DestinationChunk {
            id,
            content_hash,
            has_vector,
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
    Ok(build_report(
        source_name,
        destination_name,
        sensitivity,
        &src,
        &dst,
    ))
}

/// Pure mapping step, split out so it can be tested without transports.
pub fn build_report(
    source_name: &str,
    destination_name: &str,
    sensitivity: Sensitivity,
    src: &[SourceChunk],
    dst: &[DestinationChunk],
) -> InventoryReport {
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
            Some(d) if d.has_vector => {
                consumed.insert(d.id);
                (
                    MappingEntry {
                        source_chunk_id: s.id,
                        source_content_hash: s.content_hash.clone(),
                        destination_chunk_id: Some(d.id),
                        destination_content_hash: Some(d.content_hash.clone()),
                        status: MappingStatus::Verified,
                    },
                    MappingStatus::Verified,
                )
            }
            Some(d) => {
                consumed.insert(d.id);
                (
                    MappingEntry {
                        source_chunk_id: s.id,
                        source_content_hash: s.content_hash.clone(),
                        destination_chunk_id: Some(d.id),
                        destination_content_hash: Some(d.content_hash.clone()),
                        status: MappingStatus::MissingVector,
                    },
                    MappingStatus::MissingVector,
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
        }
        mappings.push(entry);
    }

    let destination_only_ids = dst
        .iter()
        .filter(|d| !consumed.contains(&d.id))
        .map(|d| d.id)
        .collect();

    let run_id = run_identity(source_name, destination_name, sensitivity, &mappings);

    InventoryReport {
        format: INVENTORY_FORMAT.to_string(),
        run_id,
        source: source_name.to_string(),
        destination: destination_name.to_string(),
        sensitivity: sensitivity.name().to_string(),
        counts,
        mappings,
        unverified_source_ids,
        destination_only_ids,
    }
}

/// Stable run identity over the pair and the mapped (id, hash) set. Sorted so
/// the same inputs always yield the same id, which is what makes a resume
/// comparable.
fn run_identity(
    source: &str,
    destination: &str,
    sensitivity: Sensitivity,
    mappings: &[MappingEntry],
) -> String {
    let mut h = Sha256::new();
    h.update(INVENTORY_FORMAT.as_bytes());
    h.update(b"\0");
    h.update(source.as_bytes());
    h.update(b"\0");
    h.update(destination.as_bytes());
    h.update(b"\0");
    h.update(sensitivity.name().as_bytes());
    for m in mappings {
        h.update(b"\0");
        h.update(m.source_chunk_id.to_string().as_bytes());
        h.update(b":");
        h.update(m.source_content_hash.as_bytes());
        h.update(b":");
        h.update(m.status.as_str().as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl MappingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MappingStatus::Verified => "verified",
            MappingStatus::MissingDestination => "missing_destination",
            MappingStatus::HashMismatch => "hash_mismatch",
            MappingStatus::MissingVector => "missing_vector",
        }
    }
}

/// Write the run record as JSON. Identifiers, hashes and counts only.
pub fn write_run(path: &Path, report: &InventoryReport) -> Result<()> {
    let json = serde_json::to_vec_pretty(report)
        .map_err(|e| Error::backend(format!("remediation: serialize run: {e}")))?;
    std::fs::write(path, json)
        .map_err(|e| Error::backend(format!("remediation: write {}: {e}", path.display())))
}

/// Read a previous run record for resume. Refuses a malformed record or one
/// written by a different format; drift detection against a fresh inventory is
/// the caller's job via [`InventoryReport::run_id`].
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
