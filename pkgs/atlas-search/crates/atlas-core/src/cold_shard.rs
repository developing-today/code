//! Atlas cold-tier shard format: container writer + reader (design
//! `doc/2026-11-10T20-06-24Z_design_atlas_cold_tier_shard_format/`,
//! §§2–4, §7 round-trip). Follow-on 1 of #6042: the write side of
//! the hot → cold mover.
//!
//! A cold shard is one immutable object: header (magic + format
//! version + manifest length), JSON manifest, span section, vector
//! section (f32le, uncompressed), footer (section offsets/lengths +
//! SHA-256 per section + manifest SHA-256 + trailer magic).
//!
//! This module owns the container layout, the manifest schema, the
//! packing rules that can be enforced at write time, and a reader
//! used by the writer's own test suite to wire the §7
//! `cold-roundtrip-verify` check before the standalone verifier
//! (#8230) lands. The restore importer (#8228) and cold catalog
//! (#8229) are siblings and are NOT implemented here.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

// (no crate::error import needed; cold shard errors are local)

/// Magic bytes at the start of every cold shard.
pub const MAGIC_HEADER: &[u8; 8] = b"ATLASCLD";
/// Trailer magic at the end of every cold shard.
pub const MAGIC_TRAILER: &[u8; 8] = b"ATLASEND";
/// Format version this writer produces.
pub const FORMAT_VERSION: u16 = 1;
/// Span section codec: uncompressed.
pub const SPAN_CODEC_NONE: &str = "none";
/// Vector dtype: little-endian f32.
pub const VECTOR_DTYPE: &str = "f32le";
/// Tier literal for cold shards.
pub const TIER_COLD: &str = "cold";

/// Target object size (~512 MiB) and hard ceiling (2 GiB).
pub const TARGET_BYTES: usize = 512 * 1024 * 1024;
pub const CEILING_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Fixed footer size in bytes (see module docs).
pub const FOOTER_SIZE: usize = 8 + 4 * 8 + 3 * 32;
// 8 (trailer) + 4*u64 (offsets/lengths: span_off, span_len, vec_off, vec_len)
// + 3*32 (manifest sha, span sha, vector sha) -- trailer counted in the 8.
// Layout of the footer region (excluding trailer) is documented in `unpack`.

// ---------------------------------------------------------------- manifest

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SourceMark {
    pub source_id: String,
    #[serde(default)]
    pub coords: Json,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LineageSummary {
    /// Predecessor (file_id, version_seq) as a two-element array-ish
    /// pair, null when there is no predecessor.
    #[serde(default)]
    pub predecessor_file_id: Option<u64>,
    #[serde(default)]
    pub predecessor_version_seq: Option<u64>,
    /// Edge kind from the #6026 §2.2 vocabulary.
    #[serde(default)]
    pub edge_kind: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VersionEntry {
    pub file_id: u64,
    pub version_seq: u64,
    pub path_at_version: String,
    pub content_hash: String,
    pub ingested_at: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_native_id: Option<String>,
    #[serde(default)]
    pub lineage: LineageSummary,
    /// (span_ref, span_start, span_len) triples binding span-table
    /// entries to byte ranges of this version, in version order.
    #[serde(default)]
    pub occurrences: Vec<(String, usize, usize)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpanTableEntry {
    /// The chunk's content hash, verbatim from hot metadata.
    pub span_ref: String,
    pub offset: usize,
    pub length: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VectorTableEntry {
    pub span_ref: String,
    pub index: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Retention {
    pub corpus_class: String,
    pub state: String, // "cold"
    pub cold_since: String,
    pub retire_not_before: String,
    #[serde(default)]
    pub trigger: Json,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Provenance {
    pub hot_shard_id: String,
    pub export_run_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub format_version: u16,
    pub shard_id: String,
    pub corpus: String,
    pub tier: String,
    pub created_at: String,
    pub created_by: String,
    #[serde(default)]
    pub source_marks: Vec<SourceMark>,
    pub versions: Vec<VersionEntry>,
    pub span_table: Vec<SpanTableEntry>,
    pub vector_model: String,
    pub vector_dimensions: usize,
    pub vector_dtype: String,
    pub vector_table: Vec<VectorTableEntry>,
    pub span_codec: String,
    pub retention: Retention,
    pub provenance: Provenance,
}

// ---------------------------------------------------------------- input

#[derive(Clone, Debug)]
pub struct SpanInput {
    pub span_ref: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct PackRequest {
    pub corpus: String,
    pub corpus_class: String,
    pub created_at: String,
    pub created_by: String,
    pub shard_id: Option<String>,
    pub source_marks: Vec<SourceMark>,
    pub versions: Vec<VersionEntry>,
    pub spans: Vec<SpanInput>,
    /// Vectors keyed by span_ref; a span with no vector entry is
    /// represented by absence (not an error).
    pub vectors: Vec<(String, Vec<f32>)>,
    pub span_codec: String,
    pub vector_model: String,
    pub vector_dimensions: usize,
    pub retention_cold_since: String,
    pub retention_retire_not_before: String,
    pub retention_trigger: Json,
    pub provenance_hot_shard_id: String,
    pub provenance_export_run_id: String,
}

// ---------------------------------------------------------------- errors

#[derive(Debug)]
pub enum ColdShardError {
    InvalidMagic,
    BadFormatVersion(u16),
    Truncated,
    ChecksumMismatch { what: &'static str },
    OffsetMismatch,
    Json(serde_json::Error),
    Packing(String),
    Compression(String),
}

impl std::fmt::Display for ColdShardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColdShardError::InvalidMagic => write!(f, "invalid magic"),
            ColdShardError::BadFormatVersion(v) => write!(f, "bad format version {v}"),
            ColdShardError::Truncated => write!(f, "truncated shard"),
            ColdShardError::ChecksumMismatch { what } => write!(f, "checksum mismatch: {what}"),
            ColdShardError::OffsetMismatch => write!(f, "section offset/length mismatch"),
            ColdShardError::Json(e) => write!(f, "json error: {e}"),
            ColdShardError::Packing(s) => write!(f, "packing error: {s}"),
            ColdShardError::Compression(s) => write!(f, "compression error: {s}"),
        }
    }
}

impl std::error::Error for ColdShardError {}

impl From<serde_json::Error> for ColdShardError {
    fn from(e: serde_json::Error) -> Self {
        ColdShardError::Json(e)
    }
}

pub type ColdResult<T> = std::result::Result<T, ColdShardError>;

// ---------------------------------------------------------------- helpers

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().into()
}

fn u64le(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn read_u64le(bytes: &[u8], off: &mut usize) -> ColdResult<u64> {
    if *off + 8 > bytes.len() {
        return Err(ColdShardError::Truncated);
    }
    let v = u64::from_le_bytes(bytes[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}

// ---------------------------------------------------------------- packing

/// Validate §4 packing rules that can be checked at write time, and
/// return an error describing the first violation.
fn validate_packing(req: &PackRequest) -> ColdResult<()> {
    // One corpus + one class per shard is inherent (single request).
    // Whole versions only: every version must have at least one occurrence
    // OR be an empty version; we don't split versions here, so nothing to
    // check beyond presence. Versions with empty occurrence lists are allowed
    // (a version with no spans) but unusual; allow.
    // Target/ceiling: compute approximate size.
    let span_bytes: usize = req.spans.iter().map(|s| s.bytes.len()).sum();
    let vec_bytes: usize = req.vectors.iter().map(|(_, v)| v.len() * 4).sum();
    let manifest_est = serde_json::to_vec(&build_manifest(req, &[], &[])).unwrap_or_default().len();
    let total = 14 + manifest_est + span_bytes + vec_bytes + FOOTER_SIZE;
    if total > CEILING_BYTES {
        return Err(ColdShardError::Packing(format!(
            "shard size {total} exceeds 2 GiB ceiling"
        )));
    }
    // span_refs referenced by versions must exist in spans
    let span_refs: std::collections::BTreeSet<&str> =
        req.spans.iter().map(|s| s.span_ref.as_str()).collect();
    for v in &req.versions {
        for (r, _, _) in &v.occurrences {
            if !span_refs.contains(r.as_str()) {
                return Err(ColdShardError::Packing(format!(
                    "version ({},{}) references unknown span_ref {r}",
                    v.file_id, v.version_seq
                )));
            }
        }
    }
    // vectors dimensions consistency
    for (_, vec) in &req.vectors {
        if vec.len() != req.vector_dimensions {
            return Err(ColdShardError::Packing(format!(
                "vector length {} != dimensions {}",
                vec.len(),
                req.vector_dimensions
            )));
        }
    }
    // lineage grouping is advisory; nothing to enforce mechanically.
    let _ = TARGET_BYTES;
    Ok(())
}

fn build_manifest(
    req: &PackRequest,
    span_table: &[SpanTableEntry],
    vector_table: &[VectorTableEntry],
) -> Manifest {
    let shard_id = req
        .shard_id
        .clone()
        .unwrap_or_else(|| format!("shard-{}", req.versions.len()));
    Manifest {
        format_version: FORMAT_VERSION,
        shard_id,
        corpus: req.corpus.clone(),
        tier: TIER_COLD.to_string(),
        created_at: req.created_at.clone(),
        created_by: req.created_by.clone(),
        source_marks: req.source_marks.clone(),
        versions: req.versions.clone(),
        span_table: span_table.to_vec(),
        vector_model: req.vector_model.clone(),
        vector_dimensions: req.vector_dimensions,
        vector_dtype: VECTOR_DTYPE.to_string(),
        vector_table: vector_table.to_vec(),
        span_codec: req.span_codec.clone(),
        retention: Retention {
            corpus_class: req.corpus_class.clone(),
            state: TIER_COLD.to_string(),
            cold_since: req.retention_cold_since.clone(),
            retire_not_before: req.retention_retire_not_before.clone(),
            trigger: req.retention_trigger.clone(),
        },
        provenance: Provenance {
            hot_shard_id: req.provenance_hot_shard_id.clone(),
            export_run_id: req.provenance_export_run_id.clone(),
        },
    }
}

/// Pack a request into a cold shard byte blob (§2 container layout).
pub fn pack(req: &PackRequest) -> ColdResult<Vec<u8>> {
    validate_packing(req)?;

    // Dedupe spans by span_ref (first wins), preserving input order.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut distinct: Vec<&SpanInput> = Vec::new();
    for s in &req.spans {
        if !seen.contains_key(&s.span_ref) {
            seen.insert(s.span_ref.clone(), distinct.len());
            distinct.push(s);
        }
    }

    // Build span section + span_table.
    let mut span_section: Vec<u8> = Vec::new();
    let mut span_table: Vec<SpanTableEntry> = Vec::with_capacity(distinct.len());
    for s in &distinct {
        let off = span_section.len();
        span_section.extend_from_slice(&s.bytes);
        span_table.push(SpanTableEntry {
            span_ref: s.span_ref.clone(),
            offset: off,
            length: s.bytes.len(),
        });
    }

    // Build vector section + vector_table. Order vectors by span_table
    // position when possible; otherwise input order.
    let pos: BTreeMap<String, usize> = span_table
        .iter()
        .enumerate()
        .map(|(i, e)| (e.span_ref.clone(), i))
        .collect();
    let mut ordered_vecs: Vec<(String, Vec<f32>)> = req.vectors.clone();
    ordered_vecs.sort_by_key(|(r, _)| pos.get(r).copied().unwrap_or(usize::MAX));

    let mut vector_section: Vec<u8> = Vec::new();
    let mut vector_table: Vec<VectorTableEntry> = Vec::with_capacity(ordered_vecs.len());
    for (r, v) in &ordered_vecs {
        let idx = vector_table.len();
        for f in v {
            vector_section.extend_from_slice(&f.to_le_bytes());
        }
        vector_table.push(VectorTableEntry {
            span_ref: r.clone(),
            index: idx,
        });
    }

    let manifest = build_manifest(req, &span_table, &vector_table);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(ColdShardError::Json)?;

    // Compress span section if requested (whole-section zstd).
    let stored_span_section: Vec<u8> = if req.span_codec == "zstd" {
        zstd::encode_all(span_section.as_slice(), 0)
            .map_err(|e| ColdShardError::Compression(e.to_string()))?
    } else {
        span_section.clone()
    };

    // Assemble.
    let manifest_sha = sha256(&manifest_bytes);
    let span_sha = sha256(&stored_span_section);
    let vector_sha = sha256(&vector_section);

    let header_len: u32 = manifest_bytes.len() as u32;

    // header is 8 magic + 2 version + 4 len = 14
    let span_offset = 14 + manifest_bytes.len();
    let vector_offset = span_offset + stored_span_section.len();

    let mut out: Vec<u8> = Vec::with_capacity(
        14 + manifest_bytes.len()
            + stored_span_section.len()
            + vector_section.len()
            + FOOTER_SIZE,
    );
    out.extend_from_slice(MAGIC_HEADER);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(&manifest_bytes);
    out.extend_from_slice(&stored_span_section);
    out.extend_from_slice(&vector_section);

    // Footer fields (before trailer).
    u64le(&mut out, span_offset as u64);
    u64le(&mut out, stored_span_section.len() as u64);
    u64le(&mut out, vector_offset as u64);
    u64le(&mut out, vector_section.len() as u64);
    out.extend_from_slice(&manifest_sha);
    out.extend_from_slice(&span_sha);
    out.extend_from_slice(&vector_sha);
    out.extend_from_slice(MAGIC_TRAILER);

    let _ = (span_offset, vector_offset);
    Ok(out)
}

// ---------------------------------------------------------------- unpack

#[derive(Debug, Clone)]
pub struct UnpackedShard {
    pub manifest: Manifest,
    pub span_section: Vec<u8>,
    pub vector_section: Vec<u8>,
}

/// Parse and verify a cold shard byte blob.
pub fn unpack(bytes: &[u8]) -> ColdResult<UnpackedShard> {
    if bytes.len() < 14 + FOOTER_SIZE {
        return Err(ColdShardError::Truncated);
    }
    // trailer magic
    let trailer_off = bytes.len() - 8;
    if bytes[trailer_off..] != *MAGIC_TRAILER {
        return Err(ColdShardError::InvalidMagic);
    }
    // footer region: 136 bytes before trailer? FOOTER_SIZE includes trailer.
    let footer_start = bytes.len() - FOOTER_SIZE;
    let mut off = footer_start;
    let span_offset = read_u64le(bytes, &mut off)? as usize;
    let span_len = read_u64le(bytes, &mut off)? as usize;
    let vector_offset = read_u64le(bytes, &mut off)? as usize;
    let vector_len = read_u64le(bytes, &mut off)? as usize;
    // checksums
    if off + 32 + 32 + 32 + 8 != bytes.len() {
        return Err(ColdShardError::Truncated);
    }
    let manifest_sha: [u8; 32] = bytes[off..off + 32].try_into().unwrap();
    off += 32;
    let span_sha: [u8; 32] = bytes[off..off + 32].try_into().unwrap();
    off += 32;
    let vector_sha: [u8; 32] = bytes[off..off + 32].try_into().unwrap();
    // trailer already checked.

    // header
    if bytes[0..8] != *MAGIC_HEADER {
        return Err(ColdShardError::InvalidMagic);
    }
    let format_version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
    if format_version != FORMAT_VERSION {
        return Err(ColdShardError::BadFormatVersion(format_version));
    }
    let header_len = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    if 14 + header_len > bytes.len() {
        return Err(ColdShardError::Truncated);
    }
    let manifest_bytes = &bytes[14..14 + header_len];

    // verify manifest checksum
    if sha256(manifest_bytes) != manifest_sha {
        return Err(ColdShardError::ChecksumMismatch { what: "manifest" });
    }

    // sections
    if span_offset + span_len > bytes.len() || vector_offset + vector_len > bytes.len() {
        return Err(ColdShardError::Truncated);
    }
    let stored_span_section = bytes[span_offset..span_offset + span_len].to_vec();
    let vector_section = bytes[vector_offset..vector_offset + vector_len].to_vec();

    if sha256(&stored_span_section) != span_sha {
        return Err(ColdShardError::ChecksumMismatch { what: "span section" });
    }
    if sha256(&vector_section) != vector_sha {
        return Err(ColdShardError::ChecksumMismatch { what: "vector section" });
    }

    // offsets consistency check (span section must follow manifest)
    if span_offset != 14 + header_len {
        return Err(ColdShardError::OffsetMismatch);
    }
    if vector_offset != span_offset + span_len {
        return Err(ColdShardError::OffsetMismatch);
    }

    let manifest: Manifest = serde_json::from_slice(manifest_bytes)?;
    // decompress span section if needed (offsets name decompressed positions)
    let span_section = if manifest.span_codec == "zstd" {
        zstd::decode_all(stored_span_section.as_slice())
            .map_err(|e| ColdShardError::Compression(e.to_string()))?
    } else {
        stored_span_section
    };
    Ok(UnpackedShard {
        manifest,
        span_section,
        vector_section,
    })
}

// ---------------------------------------------------------------- reconstruct

/// Reconstruct a version's bytes from its occurrence list using the
/// unpacked span section + span table. Returns the concatenated span
/// bytes in occurrence order (each occurrence contributes
/// `span_bytes[span_start .. span_start + span_len]`? No — here each
/// occurrence contributes the *whole* referenced span's bytes; the
/// span_start/span_len in the occurrence are recorded but reconstruction
/// uses span snippets in order). See module docs.
///
/// For the §7 round trip we reconstruct by walking occurrences in
/// order and appending the referenced span's full bytes (the span
/// snippet), which yields byte-identical content when the packer
/// packed whole spans. Occurrence byte ranges are verified to be
/// consistent (start 0, contiguous) as a sanity check.
pub fn reconstruct_version(
    unpacked: &UnpackedShard,
    version: &VersionEntry,
) -> ColdResult<Vec<u8>> {
    // map span_ref -> (offset, length) in span section
    let mut span_loc: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for e in &unpacked.manifest.span_table {
        span_loc.insert(e.span_ref.clone(), (e.offset, e.length));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut expected_start: usize = 0;
    for (span_ref, start, len) in &version.occurrences {
        let (off, span_len) = span_loc
            .get(span_ref)
            .copied()
            .ok_or(ColdShardError::Packing("unknown span_ref in occurrence".into()))?;
        if *start != expected_start {
            // allow non-contiguous? require contiguous for byte-identical
            // reconstruction; mismatch is a packing defect.
            return Err(ColdShardError::OffsetMismatch);
        }
        if *len > span_len {
            return Err(ColdShardError::OffsetMismatch);
        }
        // take `len` bytes from the span starting at 0 (whole-span
        // contribution model). For partial contributions the span stores
        // the full chunk; occurrence len selects a prefix. We use the
        // whole span bytes to keep round-trip simple and byte-identical.
        let _ = (off, span_len);
        // Append the span's full bytes from the section.
        let end = off + span_len;
        if end > unpacked.span_section.len() {
            return Err(ColdShardError::Truncated);
        }
        // Use occurrence len to slice the span's contribution.
        let take = if *len == 0 { span_len } else { *len };
        out.extend_from_slice(&unpacked.span_section[off..off + take]);
        expected_start += take;
    }
    // content hash check is caller's responsibility (compare to version.content_hash).
    Ok(out)
}

/// Read vectors for a span_ref from the vector section.
pub fn read_vector(
    unpacked: &UnpackedShard,
    span_ref: &str,
    dimensions: usize,
) -> Option<Vec<f32>> {
    let entry = unpacked
        .manifest
        .vector_table
        .iter()
        .find(|e| e.span_ref == span_ref)?;
    let off = entry.index * dimensions * 4;
    let end = off + dimensions * 4;
    if end > unpacked.vector_section.len() {
        return None;
    }
    let mut v = Vec::with_capacity(dimensions);
    for i in 0..dimensions {
        let b: [u8; 4] = unpacked.vector_section[off + i * 4..off + i * 4 + 4]
            .try_into()
            .ok()?;
        v.push(f32::from_le_bytes(b));
    }
    Some(v)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request() -> PackRequest {
        let span_a = b"hello span A".to_vec();
        let span_b = b"span B content here".to_vec();
        let v1_bytes_len = span_a.len() + span_b.len();
        let _ = v1_bytes_len;
        let content_hash_v1 = {
            let mut h = Sha256::new();
            h.update(&span_a);
            h.update(&span_b);
            hex(&h.finalize())
        };
        PackRequest {
            corpus: "code".into(),
            corpus_class: "code".into(),
            created_at: "2026-11-22T00:00:00Z".into(),
            created_by: "test".into(),
            shard_id: Some("shard-test-1".into()),
            source_marks: vec![],
            versions: vec![VersionEntry {
                file_id: 1,
                version_seq: 1,
                path_at_version: "a.txt".into(),
                content_hash: content_hash_v1,
                ingested_at: "2026-11-22T00:00:00Z".into(),
                source_id: None,
                source_native_id: None,
                lineage: LineageSummary {
                    predecessor_file_id: None,
                    predecessor_version_seq: None,
                    edge_kind: None,
                },
                occurrences: vec![
                    ("span-a".into(), 0, span_a.len()),
                    ("span-b".into(), span_a.len(), span_b.len()),
                ],
            }],
            spans: vec![
                SpanInput { span_ref: "span-a".into(), bytes: span_a.clone() },
                SpanInput { span_ref: "span-b".into(), bytes: span_b.clone() },
            ],
            vectors: vec![("span-a".into(), vec![0.1, 0.2, 0.3])],
            span_codec: "none".into(),
            vector_model: "m".into(),
            vector_dimensions: 3,
            retention_cold_since: "2026-11-22T00:00:00Z".into(),
            retention_retire_not_before: "2027-01-01T00:00:00Z".into(),
            retention_trigger: serde_json::json!({}),
            provenance_hot_shard_id: "hot-1".into(),
            provenance_export_run_id: "run-1".into(),
        }
    }

    fn hex(d: impl AsRef<[u8]>) -> String {
        d.as_ref().iter().map(|b| format!("{:02x}", b)).collect()
    }

    #[test]
    fn roundtrip_container() {
        let req = make_request();
        let bytes = pack(&req).expect("pack");
        let unpacked = unpack(&bytes).expect("unpack");
        assert_eq!(unpacked.manifest.shard_id, "shard-test-1");
        assert_eq!(unpacked.manifest.format_version, 1);
        assert_eq!(unpacked.manifest.span_codec, "none");
        // reconstruct version bytes
        let v = &unpacked.manifest.versions[0];
        let recon = reconstruct_version(&unpacked, v).expect("recon");
        // expected: span_a + span_b
        let mut expected = b"hello span A".to_vec();
        expected.extend_from_slice(b"span B content here");
        assert_eq!(recon, expected);
        // content hash matches
        let mut h = Sha256::new();
        h.update(&recon);
        assert_eq!(hex(h.finalize()), v.content_hash);
        // vector roundtrip
        let vec = read_vector(&unpacked, "span-a", 3).unwrap();
        assert!((vec[0] - 0.1).abs() < 1e-6);
        assert!((vec[1] - 0.2).abs() < 1e-6);
        assert!((vec[2] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn checksum_corruption_detected() {
        let req = make_request();
        let mut bytes = pack(&req).expect("pack");
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        assert!(unpack(&bytes).is_err());
    }

    #[test]
    fn roundtrip_zstd() {
        let mut req = make_request();
        req.shard_id = Some("shard-zstd".into());
        req.span_codec = "zstd".into();
        let bytes = pack(&req).unwrap();
        let unpacked = unpack(&bytes).unwrap();
        assert_eq!(unpacked.manifest.span_codec, "zstd");
        let v = &unpacked.manifest.versions[0];
        let recon = reconstruct_version(&unpacked, v).unwrap();
        let mut expected = b"hello span A".to_vec();
        expected.extend_from_slice(b"span B content here");
        assert_eq!(recon, expected);
    }

    #[test]
    fn idempotent_repack() {
        let req = make_request();
        let a = pack(&req).unwrap();
        let b = pack(&req).unwrap();
        // same content modulo shard_id (fixed here) => byte-identical
        assert_eq!(a, b);
    }

    #[test]
    fn packing_rejects_unknown_span_ref() {
        let mut req = make_request();
        req.versions[0].occurrences.push(("nope".into(), 0, 1));
        assert!(pack(&req).is_err());
    }

    #[test]
    fn packing_rejects_oversize() {
        let mut req = make_request();
        req.spans[0].bytes = vec![0u8; CEILING_BYTES];
        assert!(pack(&req).is_err());
    }

}
