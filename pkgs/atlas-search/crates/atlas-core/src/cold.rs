//! Cold-tier shard reader — the verify-and-reconstruct half of the
//! restore importer for the #6042 cold-tier shard format.
//!
//! Design: `doc/2026-11-10T20-06-24Z_design_atlas_cold_tier_shard_format/`
//! (§2 container layout, §3 manifest schema, §6 restore procedure).
//! Issue #8228 (follow-on 2 of #6042). This module is §6 steps 2–4:
//! verify a fetched shard end to end, reconstruct version bytes
//! from the manifest's occurrence lists, and verify the
//! reconstruction against the version's `content_hash` before
//! anything hot is written. Cross-shard resolution (§6 step 1's
//! resolve-to-shard mechanics) and the idempotent restore decision
//! (§6's no-op/conflict rules) are pure functions here; the
//! persistent cold catalog and fan-out addressing belong to
//! follow-on 3 (#8229). The hot write (§6 steps 5–7) is
//! [`ColdShard::restore_version`]: it writes through the
//! [`HotVersionStore`] trait, so the orchestration — hash gate,
//! decision, provenance-additive insert, mark-hot — is landed and
//! tested here while a concrete hot versioning store plugs in
//! behind that trait when it lands.
//!
//! ## Byte layout (format_version 1)
//!
//! The design fixes the regions and their contents; where it leaves
//! the exact field layout open, this module is the reference reading
//! (recorded on #8228 in the 2026-11-24 decision comment):
//!
//! ```text
//! header  : "ATLASCLD" (8) | format_version u16 LE | manifest_len u32 LE
//! manifest: manifest_len bytes of UTF-8 JSON (§3)
//! spans   : span section; span-table offsets name positions in the
//!           *decompressed* section (§2). The whole section may use
//!           `none` or `zstd`; the footer checksum covers stored bytes.
//! vectors : vector section, f32le, `vector_dimensions` per vector
//! footer  : span_offset u64 LE | span_len u64 LE |
//!           vector_offset u64 LE | vector_len u64 LE |
//!           manifest_sha256 [32] | span_sha256 [32] |
//!           vector_sha256 [32] | "ATLASEND" (8)   (136 bytes)
//! ```
//!
//! The header's u32 is the manifest length: §2's manifest row gives
//! the manifest's length as coming "from the header", and a header
//! self-length would be a constant with no reader value.
//!
//! ## Trust order (§2, §6 step 2)
//!
//! The footer is the trust anchor. [`ColdShard::open`] validates the
//! trailer magic, then the SHA-256 of the manifest and of both
//! sections against the footer, and only then parses the manifest.
//! Any mismatch is a hard error: a corrupt shard is reported, never
//! partially imported — no API on this type yields payload bytes
//! from a shard that has not passed every checksum.

use std::collections::HashMap;
use std::io::Read;

use serde::Deserialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const HEADER_MAGIC: &[u8; 8] = b"ATLASCLD";
const TRAILER_MAGIC: &[u8; 8] = b"ATLASEND";
const HEADER_LEN: usize = 8 + 2 + 4;
const FOOTER_LEN: usize = 8 * 4 + 32 * 3 + 8;
const FORMAT_VERSION: u16 = 1;
const MAX_DECOMPRESSED_SPAN_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn corrupt(msg: impl Into<String>) -> Error {
    Error::fatal(format!("cold shard: {}", msg.into()))
}

/// The §3 manifest: one JSON object, field names stable within
/// format_version 1. Unknown fields are ignored (additive
/// evolution); a writer must not rename or repurpose a field.
#[derive(Debug, Clone, Deserialize)]
pub struct ColdManifest {
    pub format_version: u16,
    pub shard_id: String,
    pub corpus: String,
    /// The literal `cold` (§3).
    pub tier: String,
    pub created_at: String,
    pub created_by: String,
    #[serde(default)]
    pub source_marks: Vec<Json>,
    pub versions: Vec<ColdVersion>,
    pub span_table: Vec<SpanEntry>,
    /// `none` or `zstd` (§2); per-shard span-section codec.
    #[serde(default = "default_span_codec")]
    pub span_codec: String,
    pub vector_model: String,
    pub vector_dimensions: u32,
    #[serde(default = "default_vector_dtype")]
    pub vector_dtype: String,
    pub vector_table: Vec<VectorEntry>,
    pub retention: ColdRetention,
    pub provenance: ColdProvenance,
}

fn default_span_codec() -> String {
    "none".to_string()
}

fn default_vector_dtype() -> String {
    "f32le".to_string()
}

/// One packed version (§3.1): identity `(file_id, version_seq)`, the
/// content hash carried verbatim from hot metadata, a lineage
/// summary, and the occurrence list that binds span-table entries
/// to byte ranges of this version.
#[derive(Debug, Clone, Deserialize)]
pub struct ColdVersion {
    #[serde(deserialize_with = "de_file_id")]
    pub file_id: String,
    pub version_seq: u64,
    pub path_at_version: String,
    pub content_hash: String,
    pub ingested_at: String,
    /// Source coordinates (source id + source-native id/commit),
    /// carried as written; shape is the source's own (§3.1).
    #[serde(default)]
    pub source: Json,
    #[serde(default)]
    pub lineage: Option<LineageSummary>,
    #[serde(default)]
    pub occurrences: Vec<Occurrence>,
}

/// The version's predecessor and the edge kind that produced it,
/// from the #6026 §2.2 vocabulary (`edit`, `rename`, `split`,
/// `merge`, `tombstone`) (§3.1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LineageSummary {
    #[serde(default)]
    pub predecessor: Option<VersionRef>,
    #[serde(default)]
    pub edge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionRef {
    #[serde(deserialize_with = "de_file_id")]
    pub file_id: String,
    pub version_seq: u64,
}

/// One occurrence (§3.1): the span-table entry `span_ref` occupies
/// bytes `[span_start, span_start + span_len)` of the version.
#[derive(Debug, Clone, Deserialize)]
pub struct Occurrence {
    pub span_ref: String,
    pub span_start: u64,
    pub span_len: u64,
}

/// One distinct chunk in the span section (§3.2). `span_ref` is the
/// chunk's content hash, verbatim from hot metadata; `offset` and
/// `length` name its position in the decompressed span section.
#[derive(Debug, Clone, Deserialize)]
pub struct SpanEntry {
    pub span_ref: String,
    pub offset: u64,
    pub length: u64,
}

/// One vector in the vector section (§3.2): the vector embedding
/// chunk `span_ref`, at `index` (offset = index × dimensions × 4).
/// A chunk with no embedding has a span entry and no vector entry;
/// absence is data, not an error.
#[derive(Debug, Clone, Deserialize)]
pub struct VectorEntry {
    pub span_ref: String,
    pub index: u64,
}

/// Retention state at pack time (§3.3). A statement about pack
/// time; never updated in place.
#[derive(Debug, Clone, Deserialize)]
pub struct ColdRetention {
    #[serde(rename = "class", default)]
    pub corpus_class: String,
    #[serde(default)]
    pub state: String,
    pub cold_since: String,
    pub retire_not_before: String,
    /// The trigger snapshot that made the versions cold-eligible.
    #[serde(default)]
    pub trigger: Json,
}

/// Where the shard's versions were exported from (§3).
#[derive(Debug, Clone, Deserialize)]
pub struct ColdProvenance {
    pub hot_shard_id: String,
    pub export_run_id: String,
}

/// `file_id` is minted by Atlas at first ingestion (#6026 §2.1);
/// accept it as a JSON string or number and normalise to a string,
/// so the reader does not depend on the hot store's id encoding.
fn de_file_id<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Json::deserialize(deserializer)?;
    match v {
        Json::String(s) => Ok(s),
        Json::Number(n) => Ok(n.to_string()),
        _ => Err(serde::de::Error::custom(
            "file_id must be a string or a number",
        )),
    }
}

/// A cold shard that has passed every §6 step-2 verification. The
/// only way to obtain one is [`ColdShard::open`], so payload access
/// implies the shard verified.
#[derive(Debug)]
pub struct ColdShard {
    manifest: ColdManifest,
    /// Decompressed span section bytes.
    span_section: Vec<u8>,
    vector_section: Vec<u8>,
    span_index: HashMap<String, usize>,
}

impl ColdShard {
    /// Verify and open a fetched shard object (§6 step 2).
    ///
    /// Order: structural bounds, trailer magic, SHA-256 of the
    /// manifest and of each section against the footer, manifest
    /// parse, then structural cross-checks of the manifest's tables
    /// against the sections. Any failure aborts with an error
    /// naming the divergent part; nothing is imported.
    pub fn open(bytes: &[u8]) -> Result<ColdShard> {
        if bytes.len() < HEADER_LEN + FOOTER_LEN {
            return Err(corrupt(format!(
                "object is {} bytes; a cold shard is at least {}",
                bytes.len(),
                HEADER_LEN + FOOTER_LEN
            )));
        }
        if &bytes[0..8] != HEADER_MAGIC {
            return Err(corrupt("header magic ATLASCLD not found"));
        }
        let format_version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if format_version != FORMAT_VERSION {
            return Err(corrupt(format!(
                "format_version {format_version} is not supported (this reader speaks 1)"
            )));
        }
        let manifest_len =
            u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
        let manifest_end = HEADER_LEN
            .checked_add(manifest_len)
            .filter(|end| *end <= bytes.len() - FOOTER_LEN)
            .ok_or_else(|| corrupt("manifest length in header overruns the object"))?;
        let manifest_bytes = &bytes[HEADER_LEN..manifest_end];

        let footer_start = bytes.len() - FOOTER_LEN;
        let footer = &bytes[footer_start..];
        if &footer[FOOTER_LEN - 8..] != TRAILER_MAGIC {
            return Err(corrupt("footer trailer magic ATLASEND not found"));
        }
        let field = |at: usize| -> u64 {
            u64::from_le_bytes(footer[at..at + 8].try_into().expect("footer field width"))
        };
        let (span_offset, span_len) = (field(0), field(8));
        let (vector_offset, vector_len) = (field(16), field(24));
        let section = |offset: u64, len: u64, name: &str| -> Result<&[u8]> {
            let start = usize::try_from(offset)
                .ok()
                .filter(|s| *s >= manifest_end && *s <= footer_start)
                .ok_or_else(|| {
                    corrupt(format!("{name} section offset {offset} is out of bounds"))
                })?;
            let end = usize::try_from(len)
                .ok()
                .and_then(|l| start.checked_add(l))
                .filter(|e| *e <= footer_start)
                .ok_or_else(|| {
                    corrupt(format!("{name} section length {len} overruns the object"))
                })?;
            Ok(&bytes[start..end])
        };
        let span_section = section(span_offset, span_len, "span")?;
        let vector_section = section(vector_offset, vector_len, "vector")?;

        // The checksum gate (§2 trust order, §6 step 2): the footer
        // is the trust anchor, so the manifest and both sections
        // must match its SHA-256 digests before anything is parsed
        // or read. Any mismatch aborts the restore; a corrupt shard
        // is reported, never partially imported.
        if Sha256::digest(manifest_bytes).as_slice() != &footer[32..64] {
            return Err(corrupt(
                "manifest checksum mismatch: the manifest does not match the footer's \
                 SHA-256; restore aborted, nothing imported",
            ));
        }
        if Sha256::digest(span_section).as_slice() != &footer[64..96] {
            return Err(corrupt(
                "span section checksum mismatch: the span section does not match the \
                 footer's SHA-256; restore aborted, nothing imported",
            ));
        }
        if Sha256::digest(vector_section).as_slice() != &footer[96..128] {
            return Err(corrupt(
                "vector section checksum mismatch: the vector section does not match \
                 the footer's SHA-256; restore aborted, nothing imported",
            ));
        }

        let manifest: ColdManifest = serde_json::from_slice(manifest_bytes)
            .map_err(|e| corrupt(format!("manifest does not parse as a §3 document: {e}")))?;
        if manifest.format_version != format_version {
            return Err(corrupt(format!(
                "manifest format_version {} disagrees with the header's {format_version}",
                manifest.format_version
            )));
        }
        if manifest.tier != "cold" {
            return Err(corrupt(format!(
                "manifest tier {:?} is not the literal \"cold\"",
                manifest.tier
            )));
        }
        let span_section = match manifest.span_codec.as_str() {
            "none" => span_section.to_vec(),
            "zstd" => {
                let decoder = zstd::stream::read::Decoder::new(span_section)
                    .map_err(|e| corrupt(format!("span section zstd decoder failed: {e}")))?;
                let mut decoded = Vec::new();
                decoder
                    .take(MAX_DECOMPRESSED_SPAN_BYTES + 1)
                    .read_to_end(&mut decoded)
                    .map_err(|e| corrupt(format!("span section zstd decompression failed: {e}")))?;
                if decoded.len() as u64 > MAX_DECOMPRESSED_SPAN_BYTES {
                    return Err(corrupt(format!(
                        "span section expands beyond the {MAX_DECOMPRESSED_SPAN_BYTES}-byte limit"
                    )));
                }
                decoded
            }
            other => return Err(corrupt(format!("unknown span_codec {other:?}"))),
        };
        if manifest.vector_dtype != "f32le" {
            return Err(corrupt(format!(
                "vector_dtype {:?} is not \"f32le\"",
                manifest.vector_dtype
            )));
        }
        let mut span_index: HashMap<String, usize> = HashMap::new();
        for (i, entry) in manifest.span_table.iter().enumerate() {
            if entry.offset.checked_add(entry.length).is_none()
                || entry.offset + entry.length > span_section.len() as u64
            {
                return Err(corrupt(format!(
                    "span_table entry {} (offset {}, length {}) overruns the span section",
                    entry.span_ref, entry.offset, entry.length
                )));
            }
            if span_index.insert(entry.span_ref.clone(), i).is_some() {
                return Err(corrupt(format!(
                    "span_table carries span_ref {} twice; a chunk appears once (§3.2)",
                    entry.span_ref
                )));
            }
        }
        let vector_stride = u64::from(manifest.vector_dimensions) * 4;
        if vector_stride == 0 {
            return Err(corrupt("vector_dimensions is 0"));
        }
        for entry in &manifest.vector_table {
            let end = entry
                .index
                .checked_add(1)
                .and_then(|n| n.checked_mul(vector_stride));
            match end {
                Some(end) if end <= vector_len => {}
                _ => {
                    return Err(corrupt(format!(
                        "vector_table entry {} (index {}) overruns the vector section",
                        entry.span_ref, entry.index
                    )));
                }
            }
            if !span_index.contains_key(&entry.span_ref) {
                return Err(corrupt(format!(
                    "vector_table names span_ref {} which has no span_table entry",
                    entry.span_ref
                )));
            }
        }

        Ok(ColdShard {
            manifest,
            span_section: span_section.to_vec(),
            vector_section: vector_section.to_vec(),
            span_index,
        })
    }

    pub fn manifest(&self) -> &ColdManifest {
        &self.manifest
    }

    /// The manifest entry for one version, if the shard holds it.
    pub fn version(&self, file_id: &str, version_seq: u64) -> Option<&ColdVersion> {
        self.manifest
            .versions
            .iter()
            .find(|v| v.file_id == file_id && v.version_seq == version_seq)
    }

    /// Reconstruct a version's bytes from its occurrence list
    /// (§6 step 3): spans in version order, byte ranges as listed.
    /// The occurrence list must tile the version contiguously —
    /// a gap, an overlap, or a length that disagrees with the span
    /// table aborts the reconstruction.
    pub fn reconstruct_version(&self, file_id: &str, version_seq: u64) -> Result<Vec<u8>> {
        let version = self.version(file_id, version_seq).ok_or_else(|| {
            corrupt(format!(
                "version ({file_id}, {version_seq}) is not in shard {}",
                self.manifest.shard_id
            ))
        })?;
        let mut occurrences: Vec<&Occurrence> = version.occurrences.iter().collect();
        occurrences.sort_by_key(|o| o.span_start);
        let mut out = Vec::new();
        let mut cursor: u64 = 0;
        for occ in occurrences {
            let entry = self
                .span_index
                .get(&occ.span_ref)
                .map(|i| &self.manifest.span_table[*i])
                .ok_or_else(|| {
                    corrupt(format!(
                        "occurrence names span_ref {} which has no span_table entry",
                        occ.span_ref
                    ))
                })?;
            if occ.span_len != entry.length {
                return Err(corrupt(format!(
                    "occurrence for span_ref {} lists span_len {} but the span table says {}",
                    occ.span_ref, occ.span_len, entry.length
                )));
            }
            if occ.span_start != cursor {
                return Err(corrupt(format!(
                    "occurrence list for version ({file_id}, {version_seq}) does not tile \
                     the version: expected a span at byte {cursor}, found {}",
                    occ.span_start
                )));
            }
            let start = entry.offset as usize;
            let end = start + entry.length as usize;
            out.extend_from_slice(&self.span_section[start..end]);
            cursor += entry.length;
        }
        Ok(out)
    }

    /// The vector embedding one chunk, read as-is (§6 step 6):
    /// bit-identical little-endian f32, no decode step. `Ok(None)`
    /// when the chunk has no vector entry — absence is data (§3.2).
    pub fn vector_for_span(&self, span_ref: &str) -> Result<Option<Vec<f32>>> {
        let Some(entry) = self
            .manifest
            .vector_table
            .iter()
            .find(|v| v.span_ref == span_ref)
        else {
            return Ok(None);
        };
        let dims = self.manifest.vector_dimensions as usize;
        let start = entry.index as usize * dims * 4;
        let mut out = Vec::with_capacity(dims);
        for i in 0..dims {
            let at = start + i * 4;
            out.push(f32::from_le_bytes(
                self.vector_section[at..at + 4]
                    .try_into()
                    .expect("vector stride checked at open"),
            ));
        }
        Ok(Some(out))
    }

    /// §6 step 6's flagging input: true when the shard's vectors
    /// were produced by a different model than the hot tier's
    /// current one. The version still restores with its original
    /// vectors; restore never silently mixes models in a search set.
    pub fn vector_model_mismatch(&self, current_model: &str) -> bool {
        self.manifest.vector_model != current_model
    }

    /// Verify a reconstruction against the version's `content_hash`
    /// (§6 step 4): the gate before anything hot is written.
    /// Reconstructs the version, hashes the bytes, and compares
    /// against the hash carried verbatim from hot metadata. The
    /// reference reading (§3.1, #6026 §2.1): `content_hash` names
    /// the version's bytes as SHA-256 hex — the codebase's
    /// canonical content-hash spelling (`classify::sha256_hex`;
    /// chunk rows store the same form) — with an optional
    /// `sha256:` prefix accepted because hot metadata carries both
    /// spellings. A mismatch aborts that version and names it;
    /// other versions in the shard are unaffected. Returns the
    /// verified bytes on success, so the hot write can only ever
    /// receive bytes that passed this gate.
    pub fn verify_content_hash(&self, file_id: &str, version_seq: u64) -> Result<Vec<u8>> {
        let version = self.version(file_id, version_seq).ok_or_else(|| {
            corrupt(format!(
                "version ({file_id}, {version_seq}) is not in shard {}",
                self.manifest.shard_id
            ))
        })?;
        let bytes = self.reconstruct_version(file_id, version_seq)?;
        let actual = crate::classify::sha256_hex(&bytes);
        if normalise_content_hash(&version.content_hash) != actual {
            return Err(corrupt(format!(
                "version ({file_id}, {version_seq}) reconstructs to bytes whose SHA-256 \
                 does not match its content_hash; restore of this version aborted, \
                 nothing written"
            )));
        }
        Ok(bytes)
    }
}

/// Strip the optional `sha256:` prefix hot metadata sometimes
/// carries, and lowercase, so both hot spellings compare equal to
/// the canonical digest.
fn normalise_content_hash(hash: &str) -> String {
    hash.strip_prefix("sha256:")
        .unwrap_or(hash)
        .to_ascii_lowercase()
}

/// Resolve `(file_id, version_seq)` to the shard that holds it
/// (§6 step 1's resolve-to-shard mechanics, without the
/// persistent catalog — the catalog's rows and fan-out addressing
/// belong to #8229). The caller supplies the candidate shards the
/// catalog named (explicit addressing names exactly one; a
/// shard-addressed search names the set it fans out to). No holder
/// is a not-found error; more than one is an ambiguity error —
/// the catalog must never map one version to two shards, so the
/// importer reports the conflict rather than silently picking one.
pub fn resolve_version<'s>(
    shards: &'s [ColdShard],
    file_id: &str,
    version_seq: u64,
) -> Result<&'s ColdShard> {
    let mut holders = shards
        .iter()
        .filter(|s| s.version(file_id, version_seq).is_some());
    match (holders.next(), holders.next()) {
        (None, _) => Err(corrupt(format!(
            "version ({file_id}, {version_seq}) is in none of the {} candidate shards",
            shards.len()
        ))),
        (Some(only), None) => Ok(only),
        (Some(first), Some(_)) => Err(corrupt(format!(
            "version ({file_id}, {version_seq}) is claimed by more than one candidate \
             shard ({} first); the cold catalog must map a version to exactly one shard",
            first.manifest.shard_id
        ))),
    }
}

/// The §6 idempotence rule as a pure decision over hashes: the
/// importer computes this before any hot write. Restoring a
/// version that is already hot with the same content hash is a
/// no-op success; one whose hot copy differs is a reported
/// conflict and is never overwritten; an absent hot copy
/// proceeds. [`ColdShard::restore_version`] applies this decision
/// to its hot write; it is what keeps that write from ever
/// overwriting a differing hot copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreDecision {
    Proceed,
    NoopSuccess,
    Conflict { hot_content_hash: String },
}

pub fn restore_decision(
    cold_content_hash: &str,
    hot_content_hash: Option<&str>,
) -> RestoreDecision {
    match hot_content_hash {
        None => RestoreDecision::Proceed,
        Some(hot) if normalise_content_hash(hot) == normalise_content_hash(cold_content_hash) => {
            RestoreDecision::NoopSuccess
        }
        Some(hot) => RestoreDecision::Conflict {
            hot_content_hash: hot.to_string(),
        },
    }
}

/// Provenance for one restore (§6 step 5). Provenance is additive:
/// this record names the restore alongside — never instead of —
/// the version's original ingestion provenance, which the hot
/// store keeps untouched. `source_shard_id` and `export_run_id`
/// come from the cold shard's manifest provenance; `restore_run_id`
/// and `restored_at` are supplied by the caller driving the restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreProvenance {
    pub source_shard_id: String,
    pub export_run_id: String,
    pub restore_run_id: String,
    pub restored_at: String,
}

/// One chunk's vector being re-imported (§6 step 6): the vector
/// table's f32 values for `span_ref`, carried bit-identically —
/// restore never re-embeds.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoredVector {
    pub span_ref: String,
    pub vector: Vec<f32>,
}

/// Everything §6 steps 5–6 re-import into the hot tier for one
/// version, under its original identity: same `(file_id,
/// version_seq)`, same content hash, its lineage summary re-attached
/// as its lineage edge, its verified bytes, and its vectors as-is.
/// `vector_model_mismatch` is the §6 step-6 flag: when the shard's
/// `vector_model` differs from the hot tier's current model the
/// version still restores with its original vectors, flagged for
/// the hot tier's normal re-embedding policy — models are never
/// silently mixed in a search set.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoredVersion {
    pub file_id: String,
    pub version_seq: u64,
    pub path_at_version: String,
    pub content_hash: String,
    pub bytes: Vec<u8>,
    pub lineage: Option<LineageSummary>,
    pub vectors: Vec<RestoredVector>,
    pub vector_model_mismatch: bool,
    pub provenance: RestoreProvenance,
}

/// The hot side of a restore (§6 steps 5–7). A concrete hot
/// versioning store implements this trait; the restore
/// orchestration in [`ColdShard::restore_version`] is written
/// against it so the guarantees — hash verified before any write,
/// a differing hot copy never overwritten, mark-hot only after a
/// successful insert — hold for every store.
pub trait HotVersionStore {
    /// The content hash of the hot copy of `(file_id, version_seq)`,
    /// or `None` when the version is not hot.
    fn hot_content_hash(&self, file_id: &str, version_seq: u64) -> Result<Option<String>>;

    /// Insert one restored version under its original identity,
    /// with its restore provenance recorded additively. Called only
    /// after the version's reconstruction passed the §6 step-4
    /// content-hash gate and the restore decision was `Proceed`.
    fn insert_restored(&mut self, version: RestoredVersion) -> Result<()>;

    /// Mark `(file_id, version_seq)` hot in the catalog (§6 step 7).
    /// Called only after [`HotVersionStore::insert_restored`]
    /// succeeds. The cold shard is not deleted by a restore.
    fn mark_hot(&mut self, file_id: &str, version_seq: u64) -> Result<()>;
}

/// How one [`ColdShard::restore_version`] call ended (§6's
/// idempotence rule, applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The version was re-imported and marked hot.
    Restored,
    /// Already hot with the same content hash: no-op success,
    /// nothing written.
    NoopSuccess,
    /// Already hot with a differing content hash: reported, and
    /// the hot copy was never overwritten.
    Conflict { hot_content_hash: String },
}

impl ColdShard {
    /// Restore one version into a hot store (§6 steps 4–7, in
    /// order): verify the reconstruction against the version's
    /// `content_hash` before the store is touched at all, apply the
    /// idempotence decision against the hot copy, then — only on
    /// `Proceed` — insert the version under its original identity
    /// with additive restore provenance and its vectors as-is
    /// (flagged when their model differs from `current_vector_model`),
    /// and mark it hot. A hash mismatch aborts this version with an
    /// error and zero store writes; a same-hash hot copy is a no-op
    /// success; a differing hot copy is a reported conflict.
    pub fn restore_version<S: HotVersionStore>(
        &self,
        store: &mut S,
        file_id: &str,
        version_seq: u64,
        current_vector_model: &str,
        restore_run_id: &str,
        restored_at: &str,
    ) -> Result<RestoreOutcome> {
        // Step 4 gate first: no store call of any kind happens
        // before the reconstruction is hash-verified.
        let bytes = self.verify_content_hash(file_id, version_seq)?;
        let version = self
            .version(file_id, version_seq)
            .expect("verify_content_hash only succeeds for a version in the shard");
        match restore_decision(
            &version.content_hash,
            store.hot_content_hash(file_id, version_seq)?.as_deref(),
        ) {
            RestoreDecision::NoopSuccess => Ok(RestoreOutcome::NoopSuccess),
            RestoreDecision::Conflict { hot_content_hash } => {
                Ok(RestoreOutcome::Conflict { hot_content_hash })
            }
            RestoreDecision::Proceed => {
                // Step 6: vectors as-is, one per distinct chunk the
                // version's occurrences use, in occurrence order; a
                // chunk with no vector entry contributes none.
                let mut seen: Vec<&str> = Vec::new();
                let mut vectors = Vec::new();
                for occurrence in &version.occurrences {
                    if seen.contains(&occurrence.span_ref.as_str()) {
                        continue;
                    }
                    seen.push(occurrence.span_ref.as_str());
                    if let Some(vector) = self.vector_for_span(&occurrence.span_ref)? {
                        vectors.push(RestoredVector {
                            span_ref: occurrence.span_ref.clone(),
                            vector,
                        });
                    }
                }
                // Step 5: re-import under the original identity.
                store.insert_restored(RestoredVersion {
                    file_id: version.file_id.clone(),
                    version_seq: version.version_seq,
                    path_at_version: version.path_at_version.clone(),
                    content_hash: version.content_hash.clone(),
                    bytes,
                    lineage: version.lineage.clone(),
                    vectors,
                    vector_model_mismatch: self.vector_model_mismatch(current_vector_model),
                    provenance: RestoreProvenance {
                        source_shard_id: self.manifest.shard_id.clone(),
                        export_run_id: self.manifest.provenance.export_run_id.clone(),
                        restore_run_id: restore_run_id.to_string(),
                        restored_at: restored_at.to_string(),
                    },
                })?;
                // Step 7: mark hot only after the insert succeeded.
                store.mark_hot(file_id, version_seq)?;
                Ok(RestoreOutcome::Restored)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A spec-built shard plus the byte ranges a test may corrupt.
    struct Fixture {
        bytes: Vec<u8>,
        manifest_at: std::ops::Range<usize>,
        span_at: std::ops::Range<usize>,
        vector_at: std::ops::Range<usize>,
        footer_at: std::ops::Range<usize>,
    }

    /// Assemble a shard from parts, directly off the §2 layout:
    /// header, manifest JSON, span section, vector section, footer
    /// with the three SHA-256 checksums. Written from the design,
    /// independently of `ColdShard::open`'s parse path — the design
    /// is the oracle, the source payloads are the ground truth.
    fn build(manifest: Json, span_section: &[u8], vector_section: &[u8]) -> Fixture {
        let manifest_bytes = serde_json::to_vec(&manifest).expect("manifest serialises");
        let span_offset = HEADER_LEN + manifest_bytes.len();
        let vector_offset = span_offset + span_section.len();
        let mut out = Vec::new();
        out.extend_from_slice(HEADER_MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&(manifest_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&manifest_bytes);
        out.extend_from_slice(span_section);
        out.extend_from_slice(vector_section);
        let footer_at = out.len()..out.len() + FOOTER_LEN;
        out.extend_from_slice(&(span_offset as u64).to_le_bytes());
        out.extend_from_slice(&(span_section.len() as u64).to_le_bytes());
        out.extend_from_slice(&(vector_offset as u64).to_le_bytes());
        out.extend_from_slice(&(vector_section.len() as u64).to_le_bytes());
        out.extend_from_slice(&Sha256::digest(&manifest_bytes));
        out.extend_from_slice(&Sha256::digest(span_section));
        out.extend_from_slice(&Sha256::digest(vector_section));
        out.extend_from_slice(TRAILER_MAGIC);
        Fixture {
            manifest_at: HEADER_LEN..span_offset,
            span_at: span_offset..vector_offset,
            vector_at: vector_offset..vector_offset + vector_section.len(),
            footer_at,
            bytes: out,
        }
    }

    fn f32le(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Two versions of one file sharing one chunk; vectors for two
    /// of the three chunks; dimensions 2.
    fn standard_manifest() -> Json {
        json!({
            "format_version": 1,
            "shard_id": "shard-test-1",
            "corpus": "code",
            "tier": "cold",
            "created_at": "2026-11-24T00:00:00Z",
            "created_by": "atlas-test",
            "source_marks": [{"source": "git", "repo": "developing-today/code"}],
            "versions": [
                {
                    "file_id": "file-1",
                    "version_seq": 1,
                    "path_at_version": "a.txt",
                    "content_hash": "hash-of-hello-world",
                    "ingested_at": "2026-11-01T00:00:00Z",
                    "source": {"source": "git", "commit": "aaa"},
                    "lineage": null,
                    "occurrences": [
                        {"span_ref": "span-a", "span_start": 0, "span_len": 6},
                        {"span_ref": "span-b", "span_start": 6, "span_len": 5}
                    ]
                },
                {
                    "file_id": "file-1",
                    "version_seq": 2,
                    "path_at_version": "a.txt",
                    "content_hash": "hash-of-hello-bang",
                    "ingested_at": "2026-11-02T00:00:00Z",
                    "source": {"source": "git", "commit": "bbb"},
                    "lineage": {
                        "predecessor": {"file_id": "file-1", "version_seq": 1},
                        "edge": "edit"
                    },
                    "occurrences": [
                        {"span_ref": "span-a", "span_start": 0, "span_len": 6},
                        {"span_ref": "span-c", "span_start": 6, "span_len": 1}
                    ]
                }
            ],
            "span_table": [
                {"span_ref": "span-a", "offset": 0, "length": 6},
                {"span_ref": "span-b", "offset": 6, "length": 5},
                {"span_ref": "span-c", "offset": 11, "length": 1}
            ],
            "span_codec": "none",
            "vector_model": "embed-v1",
            "vector_dimensions": 2,
            "vector_dtype": "f32le",
            "vector_table": [
                {"span_ref": "span-a", "index": 0},
                {"span_ref": "span-b", "index": 1}
            ],
            "retention": {
                "class": "code",
                "state": "cold",
                "cold_since": "2026-11-20T00:00:00Z",
                "retire_not_before": "2027-02-20T00:00:00Z",
                "trigger": {"by_age_days": 90}
            },
            "provenance": {"hot_shard_id": "hot-1", "export_run_id": "run-1"}
        })
    }

    fn standard_fixture() -> Fixture {
        let span_section = b"hello world!".to_vec();
        let vector_section = f32le(&[1.0, -2.5, 0.5, 3.25]);
        build(standard_manifest(), &span_section, &vector_section)
    }

    #[test]
    fn reconstructs_versions_byte_identically() {
        let shard = ColdShard::open(&standard_fixture().bytes).expect("spec-built shard opens");
        assert_eq!(
            shard
                .reconstruct_version("file-1", 1)
                .expect("v1 reconstructs"),
            b"hello world"
        );
        assert_eq!(
            shard
                .reconstruct_version("file-1", 2)
                .expect("v2 reconstructs"),
            b"hello !"
        );
        // The shared chunk is stored once (§3.2) yet serves both.
        assert_eq!(shard.manifest().span_table.len(), 3);
        let v2 = shard.version("file-1", 2).expect("v2 entry");
        assert_eq!(v2.content_hash, "hash-of-hello-bang");
        let lineage = v2.lineage.as_ref().expect("v2 lineage summary");
        assert_eq!(lineage.edge.as_deref(), Some("edit"));
    }

    #[test]
    fn corrupt_span_section_is_reported_and_aborts() {
        let mut fixture = standard_fixture();
        fixture.bytes[fixture.span_at.start] ^= 0xFF;
        let err = ColdShard::open(&fixture.bytes).expect_err("corrupt span section must abort");
        assert!(
            err.to_string().contains("span section"),
            "error names the divergent section: {err}"
        );
    }

    #[test]
    fn corrupt_manifest_is_reported_and_aborts() {
        let mut fixture = standard_fixture();
        // Flip a byte inside a manifest string value, so the JSON
        // still parses: only the footer's manifest checksum can
        // catch this — which is exactly the §2 trust order.
        let manifest_slice = fixture.bytes[fixture.manifest_at.clone()].to_vec();
        let at = fixture.manifest_at.start
            + manifest_slice
                .windows(12)
                .position(|w| w == b"shard-test-1")
                .expect("fixture carries its shard_id");
        fixture.bytes[at] ^= 0x01;
        let err = ColdShard::open(&fixture.bytes).expect_err("corrupt manifest must abort");
        assert!(
            err.to_string().contains("manifest"),
            "error names the manifest: {err}"
        );
    }

    #[test]
    fn corrupt_footer_checksum_is_reported_and_aborts() {
        let mut fixture = standard_fixture();
        // First byte of the footer's span-section SHA-256.
        fixture.bytes[fixture.footer_at.start + 64] ^= 0xFF;
        let err = ColdShard::open(&fixture.bytes).expect_err("tampered footer must abort");
        assert!(
            err.to_string().contains("span section"),
            "error names the divergent section: {err}"
        );
    }

    #[test]
    fn corrupt_trailer_magic_aborts() {
        let mut fixture = standard_fixture();
        let last = fixture.bytes.len() - 1;
        fixture.bytes[last] ^= 0xFF;
        let err = ColdShard::open(&fixture.bytes).expect_err("bad trailer must abort");
        assert!(err.to_string().contains("ATLASEND"), "error: {err}");
    }

    #[test]
    fn truncated_object_aborts() {
        let fixture = standard_fixture();
        let err = ColdShard::open(&fixture.bytes[..40]).expect_err("truncated object must abort");
        assert!(err.to_string().contains("cold shard"), "error: {err}");
        // Truncation inside the payload breaks a checksum or a bound.
        let half = fixture.bytes.len() / 2;
        assert!(ColdShard::open(&fixture.bytes[..half]).is_err());
    }

    #[test]
    fn unknown_manifest_fields_are_ignored() {
        let mut manifest = standard_manifest();
        manifest["future_field"] = json!({"anything": true});
        manifest["versions"][0]["future_version_field"] = json!(42);
        let fixture = build(manifest, b"hello world!", &f32le(&[1.0, -2.5, 0.5, 3.25]));
        let shard = ColdShard::open(&fixture.bytes).expect("additive fields are ignored (§3)");
        assert_eq!(
            shard.reconstruct_version("file-1", 1).expect("v1"),
            b"hello world"
        );
    }

    #[test]
    fn vectors_read_back_bit_identical_and_absence_is_data() {
        let shard = ColdShard::open(&standard_fixture().bytes).expect("opens");
        let a = shard
            .vector_for_span("span-a")
            .expect("read")
            .expect("present");
        assert_eq!(
            a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            vec![1.0f32.to_bits(), (-2.5f32).to_bits()]
        );
        let b = shard
            .vector_for_span("span-b")
            .expect("read")
            .expect("present");
        assert_eq!(b, vec![0.5, 3.25]);
        assert_eq!(shard.vector_for_span("span-c").expect("read"), None);
    }

    #[test]
    fn version_not_in_shard_errors() {
        let shard = ColdShard::open(&standard_fixture().bytes).expect("opens");
        let err = shard
            .reconstruct_version("file-1", 99)
            .expect_err("missing version errors");
        assert!(err.to_string().contains("not in shard"), "error: {err}");
    }

    #[test]
    fn occurrence_gap_aborts_reconstruction() {
        let mut manifest = standard_manifest();
        manifest["versions"][0]["occurrences"][1]["span_start"] = json!(7);
        let fixture = build(manifest, b"hello world!", &f32le(&[1.0, -2.5, 0.5, 3.25]));
        let shard = ColdShard::open(&fixture.bytes).expect("opens");
        let err = shard
            .reconstruct_version("file-1", 1)
            .expect_err("a gap in the occurrence list aborts");
        assert!(err.to_string().contains("does not tile"), "error: {err}");
    }

    #[test]
    fn zstd_codec_decompresses_before_applying_span_offsets() {
        let mut manifest = standard_manifest();
        manifest["span_codec"] = json!("zstd");
        let encoded = zstd::encode_all(&b"hello world!"[..], 0).expect("test span compresses");
        let fixture = build(manifest, &encoded, &f32le(&[1.0, -2.5, 0.5, 3.25]));
        let shard = ColdShard::open(&fixture.bytes).expect("zstd span section opens");
        assert_eq!(
            shard
                .reconstruct_version("file-1", 1)
                .expect("v1 reconstructs"),
            b"hello world"
        );
        assert_eq!(
            shard
                .reconstruct_version("file-1", 2)
                .expect("v2 reconstructs"),
            b"hello !"
        );
    }

    #[test]
    fn vector_model_mismatch_is_flagged_not_silently_mixed() {
        let shard = ColdShard::open(&standard_fixture().bytes).expect("opens");
        assert!(shard.vector_model_mismatch("embed-v2"));
        assert!(!shard.vector_model_mismatch("embed-v1"));
    }

    #[test]
    fn vector_section_is_actually_read() {
        // Guard against a fixture mix-up: the vector range differs
        // from the span range, and corrupting a vector byte must
        // change the read-back (the checksum gate aside, the bytes
        // read are the bytes written).
        let fixture = standard_fixture();
        assert_ne!(fixture.vector_at, fixture.span_at);
        let shard = ColdShard::open(&fixture.bytes).expect("opens");
        assert_eq!(
            shard
                .vector_for_span("span-a")
                .expect("read")
                .expect("present"),
            vec![1.0, -2.5]
        );
    }

    /// Fixture whose `content_hash` values are the real SHA-256 of
    /// the reconstructed bytes (§6 step 4's ground truth): v1 is
    /// `hello world`, v2 is `hello !`.
    fn hashed_fixture() -> Fixture {
        let mut manifest = standard_manifest();
        manifest["versions"][0]["content_hash"] =
            json!(crate::classify::sha256_hex(b"hello world"));
        manifest["versions"][1]["content_hash"] = json!(crate::classify::sha256_hex(b"hello !"));
        build(manifest, b"hello world!", &f32le(&[1.0, -2.5, 0.5, 3.25]))
    }

    #[test]
    fn content_hash_verification_accepts_matching_reconstruction() {
        let shard = ColdShard::open(&hashed_fixture().bytes).expect("opens");
        assert_eq!(
            shard
                .verify_content_hash("file-1", 1)
                .expect("v1 hash verifies"),
            b"hello world"
        );
        assert_eq!(
            shard
                .verify_content_hash("file-1", 2)
                .expect("v2 hash verifies"),
            b"hello !"
        );
    }

    #[test]
    fn content_hash_verification_accepts_sha256_prefixed_spelling() {
        let mut manifest = standard_manifest();
        manifest["versions"][0]["content_hash"] = json!(format!(
            "sha256:{}",
            crate::classify::sha256_hex(b"hello world")
        ));
        manifest["versions"][1]["content_hash"] = json!(crate::classify::sha256_hex(b"hello !"));
        let fixture = build(manifest, b"hello world!", &f32le(&[1.0, -2.5, 0.5, 3.25]));
        let shard = ColdShard::open(&fixture.bytes).expect("opens");
        assert_eq!(
            shard
                .verify_content_hash("file-1", 1)
                .expect("prefixed v1 hash verifies"),
            b"hello world"
        );
    }

    #[test]
    fn content_hash_mismatch_aborts_that_version_only() {
        let mut manifest = standard_manifest();
        manifest["versions"][0]["content_hash"] =
            json!(crate::classify::sha256_hex(b"hello world"));
        manifest["versions"][1]["content_hash"] = json!("0".repeat(64));
        let fixture = build(manifest, b"hello world!", &f32le(&[1.0, -2.5, 0.5, 3.25]));
        let shard = ColdShard::open(&fixture.bytes).expect("shard itself still opens");
        let err = shard
            .verify_content_hash("file-1", 2)
            .expect_err("wrong hash aborts that version");
        assert!(
            err.to_string().contains("file-1") && err.to_string().contains('2'),
            "error names the aborted version: {err}"
        );
        // The abort is per-version: the sibling still verifies.
        assert_eq!(
            shard
                .verify_content_hash("file-1", 1)
                .expect("sibling unaffected"),
            b"hello world"
        );
    }

    #[test]
    fn resolve_version_finds_the_holding_shard() {
        let shard = ColdShard::open(&standard_fixture().bytes).expect("opens");
        let mut other_manifest = standard_manifest();
        other_manifest["shard_id"] = json!("shard-test-2");
        other_manifest["versions"] = json!([{
            "file_id": "file-9",
            "version_seq": 1,
            "path_at_version": "z.txt",
            "content_hash": "hash-of-z",
            "ingested_at": "2026-11-01T00:00:00Z",
            "occurrences": [{"span_ref": "span-a", "span_start": 0, "span_len": 6}]
        }]);
        let other = ColdShard::open(
            &build(
                other_manifest,
                b"hello world!",
                &f32le(&[1.0, -2.5, 0.5, 3.25]),
            )
            .bytes,
        )
        .expect("second shard opens");
        let shards = vec![shard, other];
        assert_eq!(
            resolve_version(&shards, "file-1", 2)
                .expect("resolves")
                .manifest()
                .shard_id,
            "shard-test-1"
        );
        assert_eq!(
            resolve_version(&shards, "file-9", 1)
                .expect("resolves")
                .manifest()
                .shard_id,
            "shard-test-2"
        );
        let err = resolve_version(&shards, "file-1", 99).expect_err("no holder errors");
        assert!(err.to_string().contains("none"), "error: {err}");
    }

    #[test]
    fn resolve_version_reports_a_version_claimed_twice() {
        let a = ColdShard::open(&standard_fixture().bytes).expect("opens");
        let b = ColdShard::open(&standard_fixture().bytes).expect("opens");
        let err = resolve_version(&[a, b], "file-1", 1).expect_err("double claim errors");
        assert!(
            err.to_string().contains("more than one"),
            "error reports the ambiguity: {err}"
        );
    }

    /// In-memory [`HotVersionStore`] recording every call, so the
    /// restore tests can assert the §6 ordering guarantees: no
    /// insert on a no-op/conflict/hash-abort, mark-hot only after a
    /// successful insert, and a differing hot copy left untouched.
    /// Oracle: the design §6 procedure, step by step.
    #[derive(Default)]
    struct FakeHotStore {
        hot: HashMap<(String, u64), String>,
        inserted: Vec<RestoredVersion>,
        marked_hot: Vec<(String, u64)>,
    }

    impl HotVersionStore for FakeHotStore {
        fn hot_content_hash(&self, file_id: &str, version_seq: u64) -> Result<Option<String>> {
            Ok(self.hot.get(&(file_id.to_string(), version_seq)).cloned())
        }

        fn insert_restored(&mut self, version: RestoredVersion) -> Result<()> {
            self.hot.insert(
                (version.file_id.clone(), version.version_seq),
                version.content_hash.clone(),
            );
            self.inserted.push(version);
            Ok(())
        }

        fn mark_hot(&mut self, file_id: &str, version_seq: u64) -> Result<()> {
            self.marked_hot.push((file_id.to_string(), version_seq));
            Ok(())
        }
    }

    #[test]
    fn restore_version_reimports_under_original_identity_with_provenance() {
        let shard = ColdShard::open(&hashed_fixture().bytes).expect("opens");
        let mut store = FakeHotStore::default();
        let outcome = shard
            .restore_version(
                &mut store,
                "file-1",
                2,
                "embed-v2",
                "restore-run-9",
                "2026-11-26T00:00:00Z",
            )
            .expect("restore proceeds");
        assert_eq!(outcome, RestoreOutcome::Restored);
        assert_eq!(store.inserted.len(), 1);
        let restored = &store.inserted[0];
        // Original identity, verified bytes, lineage re-attached (§6 step 5).
        assert_eq!(restored.file_id, "file-1");
        assert_eq!(restored.version_seq, 2);
        assert_eq!(restored.bytes, b"hello !");
        assert_eq!(
            restored.content_hash,
            crate::classify::sha256_hex(b"hello !")
        );
        let lineage = restored.lineage.as_ref().expect("lineage re-attached");
        assert_eq!(lineage.edge.as_deref(), Some("edit"));
        assert_eq!(lineage.predecessor.as_ref().map(|p| p.version_seq), Some(1));
        // Vectors as-is (§6 step 6): v2 uses span-a and span-c; only
        // span-a has a vector entry, carried bit-identically.
        assert_eq!(restored.vectors.len(), 1);
        assert_eq!(restored.vectors[0].span_ref, "span-a");
        assert_eq!(
            restored.vectors[0]
                .vector
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            vec![1.0f32.to_bits(), (-2.5f32).to_bits()]
        );
        // The shard's model is embed-v1, the hot tier's is embed-v2:
        // flagged, not silently mixed.
        assert!(restored.vector_model_mismatch);
        // Additive restore provenance (§6 step 5).
        assert_eq!(restored.provenance.source_shard_id, "shard-test-1");
        assert_eq!(restored.provenance.export_run_id, "run-1");
        assert_eq!(restored.provenance.restore_run_id, "restore-run-9");
        assert_eq!(restored.provenance.restored_at, "2026-11-26T00:00:00Z");
        // Marked hot after the insert (§6 step 7).
        assert_eq!(store.marked_hot, vec![("file-1".to_string(), 2)]);
    }

    #[test]
    fn restore_version_same_hash_hot_copy_is_a_noop() {
        let shard = ColdShard::open(&hashed_fixture().bytes).expect("opens");
        let mut store = FakeHotStore::default();
        store.hot.insert(
            ("file-1".to_string(), 1),
            crate::classify::sha256_hex(b"hello world"),
        );
        let outcome = shard
            .restore_version(&mut store, "file-1", 1, "embed-v1", "r", "t")
            .expect("no-op is a success");
        assert_eq!(outcome, RestoreOutcome::NoopSuccess);
        assert!(store.inserted.is_empty(), "no-op writes nothing");
        assert!(store.marked_hot.is_empty(), "no-op marks nothing");
    }

    #[test]
    fn restore_version_differing_hot_copy_conflicts_and_is_never_overwritten() {
        let shard = ColdShard::open(&hashed_fixture().bytes).expect("opens");
        let mut store = FakeHotStore::default();
        store
            .hot
            .insert(("file-1".to_string(), 1), "a-different-hash".to_string());
        let outcome = shard
            .restore_version(&mut store, "file-1", 1, "embed-v1", "r", "t")
            .expect("conflict is reported, not an error");
        assert_eq!(
            outcome,
            RestoreOutcome::Conflict {
                hot_content_hash: "a-different-hash".to_string()
            }
        );
        assert!(store.inserted.is_empty(), "conflict writes nothing");
        assert_eq!(
            store
                .hot
                .get(&("file-1".to_string(), 1))
                .map(String::as_str),
            Some("a-different-hash"),
            "the differing hot copy is untouched"
        );
    }

    #[test]
    fn restore_version_hash_mismatch_aborts_before_any_store_write() {
        // The standard fixture's content_hash values are not the
        // real SHA-256 of the reconstructions, so the §6 step-4
        // gate must abort before the store is written to.
        let shard = ColdShard::open(&standard_fixture().bytes).expect("opens");
        let mut store = FakeHotStore::default();
        let err = shard
            .restore_version(&mut store, "file-1", 1, "embed-v1", "r", "t")
            .expect_err("hash mismatch aborts the restore");
        assert!(
            err.to_string().contains("content_hash"),
            "error names the hash gate: {err}"
        );
        assert!(store.inserted.is_empty(), "abort writes nothing");
        assert!(store.marked_hot.is_empty(), "abort marks nothing");
    }

    #[test]
    fn restore_decision_is_idempotent_and_never_overwrites() {
        let cold = "abc123";
        assert_eq!(restore_decision(cold, None), RestoreDecision::Proceed);
        assert_eq!(
            restore_decision(cold, Some("abc123")),
            RestoreDecision::NoopSuccess
        );
        // The prefixed hot spelling is the same hash.
        assert_eq!(
            restore_decision(cold, Some("sha256:ABC123")),
            RestoreDecision::NoopSuccess
        );
        assert_eq!(
            restore_decision(cold, Some("different")),
            RestoreDecision::Conflict {
                hot_content_hash: "different".to_string()
            }
        );
    }
}
