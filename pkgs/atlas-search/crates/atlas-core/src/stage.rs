//! The Atlas staging tier: the versioned per-unit schema, one
//! provisioning tool, and the unit registry (developing-today/code#5669,
//! design `doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision`
//! section 4.1).
//!
//! The design's staging tier is "many small DBs, free writes": one
//! database per unit of work that produces data (a queue task, a
//! session, an epic, a notes batch), each carrying the *same* versioned
//! schema — `atlas_stage` v1: `items`, `chunks_raw`, `notes`, `kv`,
//! `_schema_version` — applied by one tool. A registry (internal core
//! tier) maps unit id → staging DB, owner, sensitivity level, state
//! (open / closing / merged) and schema version.
//!
//! Scope of this module (the slice the design's foundation needs, and
//! no more):
//! * [`provision_staging_db`] creates one staging DB and applies the
//!   v1 DDL from the versioned artifact [`STAGE_V1_DDL`];
//! * [`Registry`] records units and looks them up;
//! * [`Registry::provision`] is the one tool that does both.
//! * [`stage_artifact`] is the item writer for code/config
//!   embedding entries (#1518): one `items` row plus its
//!   `chunks_raw`, carrying the confirmed definition's metadata
//!   (repo, path, commit SHA, kind, language, unit).
//! * [`stage_classified_artifact`] is the classified item writer
//!   (#12628): the producer's declared source-native classification
//!   segments are composed through the boundary splitter
//!   ([`crate::boundary::split_by_boundaries`]) into boundary-safe
//!   chunks, and one #12408 classification record per chunk — exact
//!   source offsets and effective sensitivity — is persisted in the
//!   item's `meta` for promotion to enforce
//!   ([`chunk_classifications`]).
//! * [`Registry::promote`] is the merge-on-close transition, gated on
//!   the embedding-model contract (#7648, M6 of the #6060 decision
//!   record): a unit is promoted only when its database's recorded
//!   `atlas_manifest` `embedding_model` equals the mandated model.
//!   A successful promotion writes the unit's [`MergeManifest`],
//!   branches its staging DB as the unit's archive
//!   ([`branch_staging_db_as_archive`]), and takes the unit out of
//!   every active state; the manifest and the archive are the
//!   durable record (#5669 criterion 4).
//!
//! Deliberately out of scope here (later slices of #5669): the
//! aggregator/embedder and merge-on-close promotion, the physical core
//! sensitivity stores, the session-close extraction hook, and the
//! end-to-end demo. Provisioning targets a local libSQL/SQLite file
//! today; hosted Turso creation (`turso db create`) is the deployment
//! surface the aggregator slice names, not this foundation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::boundary::{ClassificationSegment, split_by_boundaries};
use crate::classification::{ClassificationProvenance, ClassificationRecord};
use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::shards::{EMBEDDING_MODEL_ID, Tier};

/// The staging schema version this build knows how to provision and
/// read. Bump only together with a new `atlas_stage_vN.sql` artifact;
/// an applied version is never edited in place (a staging DB is
/// promoted by being read, never by being migrated).
pub const STAGE_SCHEMA_VERSION: i64 = 1;

/// The v1 staging DDL, as the single versioned artifact
/// (`pkgs/atlas-search/schema/atlas_stage_v1.sql`) — the same file one
/// tool applies to every unit.
pub const STAGE_V1_DDL: &str = include_str!("../../../schema/atlas_stage_v1.sql");

/// The registry's own v1 DDL artifact
/// (`pkgs/atlas-search/schema/stage_registry_v1.sql`).
pub const REGISTRY_V1_DDL: &str = include_str!("../../../schema/stage_registry_v1.sql");

/// The exact v1 staging table set, in one place so the schema-version
/// invariant (and tests) can name it.
pub const STAGE_V1_TABLES: [&str; 5] = ["_schema_version", "items", "chunks_raw", "notes", "kv"];

/// A staging unit's lifecycle state (design 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    /// Writes are free and unmediated.
    Open,
    /// Closing: the aggregator may pick it up.
    Closing,
    /// Merged: promoted, archived by a branch, dropped from active use.
    Merged,
}

impl UnitState {
    pub fn name(self) -> &'static str {
        match self {
            UnitState::Open => "open",
            UnitState::Closing => "closing",
            UnitState::Merged => "merged",
        }
    }

    pub fn parse(s: &str) -> Option<UnitState> {
        match s {
            "open" => Some(UnitState::Open),
            "closing" => Some(UnitState::Closing),
            "merged" => Some(UnitState::Merged),
            _ => None,
        }
    }
}

impl std::fmt::Display for UnitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One row of the registry: unit id → staging DB, owner, sensitivity
/// level, state, schema version (design 4.1), plus the corpus's
/// recorded embedding-model identity (#8623 slice 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingUnit {
    pub unit_id: String,
    /// The staging database's location as the provisioning tool
    /// recorded it (a local file path today).
    pub db_uri: String,
    pub owner: String,
    /// Physical sensitivity level (design 4.2: public / internal /
    /// secret; the default is internal).
    pub sensitivity: Tier,
    pub state: UnitState,
    pub schema_version: i64,
    pub created_at: String,
    pub updated_at: String,
    /// The embedding model + pipeline identity this corpus's vectors
    /// are recorded as produced with
    /// ([`crate::shards::EMBEDDING_MODEL_ID`]). Recorded at provision
    /// time; ingestion refuses a corpus whose recorded identity is not
    /// the mandated one, and the re-embed job updates it when it
    /// rebuilds the corpus under a new model.
    pub embedding_model: String,
}

/// Immutable audit record written as part of a successful promotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeManifest {
    pub unit_id: String,
    pub rows_in: i64,
    pub rows_promoted: i64,
    pub content_hash: String,
    pub embedding_model: String,
    pub promoted_at: String,
}

// ------------------------------------------------------- provisioning --

/// Create one staging DB at `path` and apply the v1 DDL. The
/// destination must be a new or empty database: applying v1 over an
/// existing staging set is an error, not a silent no-op, so a
/// re-provision can never half-clobber a unit's data. On success the
/// database reports `_schema_version = 1` and exactly
/// [`STAGE_V1_TABLES`].
pub fn provision_staging_db(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::backend(format!("staging: creating {}: {e}", parent.display())))?;
    }
    let conn = open(path)?;
    let existing = user_tables(&conn)?;
    if !existing.is_empty() {
        return Err(Error::fatal(format!(
            "staging: {} is not empty (tables: {}); refusing to re-provision",
            path.display(),
            existing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    conn.execute_batch(STAGE_V1_DDL).map_err(|e| {
        Error::backend(format!(
            "staging: applying v1 DDL to {}: {e}",
            path.display()
        ))
    })?;
    check_stage_schema(&conn)?;
    Ok(())
}

/// The `_schema_version` a staging database reports.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT version FROM _schema_version", [], |r| r.get(0))
        .map_err(|e| Error::backend(format!("staging: reading _schema_version: {e}")))
}

/// The set of user tables in a database (internal SQLite tables
/// excluded), used both for the empty check and the table-set
/// invariant.
pub fn user_tables(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| Error::backend(format!("staging: listing tables: {e}")))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| Error::backend(format!("staging: listing tables: {e}")))?;
    let mut out = BTreeSet::new();
    for row in rows {
        out.insert(row.map_err(|e| Error::backend(format!("staging: listing tables: {e}")))?);
    }
    Ok(out)
}

/// Assert a database is exactly a v1 staging DB: `_schema_version = 1`
/// and the exact [`STAGE_V1_TABLES`] table set. This is the invariant
/// the schema oracle checks. One deliberate consequence: a promoted
/// unit's DB no longer satisfies it — promotion stamps a sixth
/// table (`atlas_manifest`) into the unit DB, which is what closes
/// it to further `stage_artifact` writes (see
/// [`crate::aggregate`]'s step 4; #13284).
pub fn check_stage_schema(conn: &Connection) -> Result<()> {
    let version = schema_version(conn)?;
    if version != STAGE_SCHEMA_VERSION {
        return Err(Error::fatal(format!(
            "staging: schema version {version} (want {STAGE_SCHEMA_VERSION})"
        )));
    }
    let want: BTreeSet<&str> = STAGE_V1_TABLES.into_iter().collect();
    let got = user_tables(conn)?;
    let got_refs: BTreeSet<&str> = got.iter().map(String::as_str).collect();
    if got_refs != want {
        return Err(Error::fatal(format!(
            "staging: table set {:?} (want {:?})",
            got, STAGE_V1_TABLES
        )));
    }
    Ok(())
}

// ------------------------------------------------------------- items --

/// The #1518 whole-file ceiling: a small config at or below this
/// size stages as one whole-file unit. Above it, the artifact is
/// split into declaration-sized chunks (see [`chunk_artifact_text`]).
pub const WHOLE_FILE_MAX_BYTES: usize = 32 * 1024;

/// The hard ceiling for one `chunks_raw` chunk: a single
/// declaration larger than this is split further at line
/// boundaries (never mid-line), so no chunk is unbounded.
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;

/// One repository artifact to stage as a code/config embedding
/// entry (#1518, definition in
/// `doc/2026-11-14T19-00-00Z_design_code-config-embeddings.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactEntry {
    /// The entry kind: `prose` | `code` | `config`.
    pub kind: String,
    /// The repository the bytes came from (e.g. `developing-today/code`).
    pub repo: String,
    /// The artifact's path inside the repo.
    pub path: String,
    /// The commit SHA the bytes were read at. Together with repo
    /// and path this is the entry's source-of-truth pointer: the
    /// repo at that revision is the truth; the embedding is a cache.
    pub commit_sha: String,
    /// The artifact language (`yaml`, `toml`, `rust`, ...).
    pub language: String,
    /// The existing basic sensitivity classification; recorded on
    /// the row, as for every staged item.
    pub sensitivity: Tier,
    /// The artifact's full text.
    pub text: String,
}

/// One staged item, read back from a staging DB ([`list_staged_items`]),
/// with its full text and chunk count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedItem {
    pub id: i64,
    pub unit_id: String,
    pub kind: String,
    pub source: Option<String>,
    pub text: String,
    pub sensitivity: String,
    pub created_at: String,
    pub meta: Option<String>,
    pub chunk_count: i64,
}

fn artifact_source(entry: &ArtifactEntry) -> String {
    format!("{}:{}@{}", entry.repo, entry.path, entry.commit_sha)
}

fn is_top_level_declaration(line: &str) -> bool {
    let t = line.trim_end_matches(['\n', '\r']);
    !t.is_empty() && !t.starts_with([' ', '\t', '#']) && (t.contains(':') || t.ends_with('{'))
}

/// Split artifact text into `chunks_raw` chunks under the #1518
/// unit rules: at or below [`WHOLE_FILE_MAX_BYTES`] the whole text
/// is one chunk; above it, chunks break before top-level
/// declaration lines (a non-indented, non-comment line carrying a
/// `:` or opening a block), and any chunk still over
/// [`MAX_CHUNK_BYTES`] is split further at line boundaries. The
/// chunks always partition the input exactly: concatenating them
/// reproduces `text` byte for byte.
pub fn chunk_artifact_text(text: &str) -> Vec<String> {
    if text.len() <= WHOLE_FILE_MAX_BYTES {
        return vec![text.to_string()];
    }
    let mut starts: Vec<usize> = vec![0];
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if offset > 0 && is_top_level_declaration(line) {
            starts.push(offset);
        }
        offset += line.len();
    }
    starts.push(text.len());
    let mut chunks = Vec::new();
    for w in starts.windows(2) {
        let piece = &text[w[0]..w[1]];
        if piece.len() <= MAX_CHUNK_BYTES {
            chunks.push(piece.to_string());
            continue;
        }
        // One declaration over the hard ceiling: split at line
        // boundaries, keeping every piece within the ceiling when
        // a boundary exists inside it.
        let mut start = 0usize;
        let mut acc = 0usize;
        for line in piece.split_inclusive('\n') {
            if acc > 0 && acc + line.len() > MAX_CHUNK_BYTES {
                chunks.push(piece[start..start + acc].to_string());
                start += acc;
                acc = 0;
            }
            acc += line.len();
        }
        if acc > 0 {
            chunks.push(piece[start..start + acc].to_string());
        }
    }
    chunks
}

pub(crate) fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Stage one code/config artifact into a v1 staging DB as one
/// `items` row plus its `chunks_raw` rows, per the #1518
/// definition: `source` carries the source-of-truth pointer
/// (`repo:path@commit_sha`) and `meta` the required metadata set
/// (repo, path, commit_sha, kind, language, unit, embedded_at).
/// Returns the new item's id. The DB must already be a v1 staging
/// DB ([`check_stage_schema`]); vectors are produced later, at
/// promotion, as for every staged item.
pub fn stage_artifact(
    conn: &Connection,
    unit_id: &str,
    entry: &ArtifactEntry,
    now: &str,
) -> Result<i64> {
    check_stage_schema(conn)?;
    let chunks = chunk_artifact_text(&entry.text);
    let unit = if entry.text.len() <= WHOLE_FILE_MAX_BYTES {
        "whole-file"
    } else {
        "declaration"
    };
    let meta = serde_json::json!({
        "repo": entry.repo,
        "path": entry.path,
        "commit_sha": entry.commit_sha,
        "kind": entry.kind,
        "language": entry.language,
        "unit": unit,
        "embedded_at": now,
        "chunks": chunks.len(),
    })
    .to_string();
    conn.execute(
        "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            unit_id,
            entry.kind,
            artifact_source(entry),
            entry.text,
            entry.sensitivity.name(),
            now,
            meta,
        ],
    )
    .map_err(|e| Error::backend(format!("staging: inserting item for {}: {e}", entry.path)))?;
    let item_id = conn.last_insert_rowid();
    for (idx, chunk) in chunks.iter().enumerate() {
        conn.execute(
            "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item_id, idx as i64, chunk, sha256_hex(chunk), now],
        )
        .map_err(|e| {
            Error::backend(format!(
                "staging: inserting chunk {idx} for {}: {e}",
                entry.path
            ))
        })?;
    }
    Ok(item_id)
}

/// [`stage_artifact`] against a staging DB file: the form the
/// `atlas-stage ingest` tool calls, so the CLI needs no DB handle
/// of its own.
pub fn stage_artifact_in_db(
    db: &Path,
    unit_id: &str,
    entry: &ArtifactEntry,
    now: &str,
) -> Result<i64> {
    let conn = open(db)?;
    stage_artifact(&conn, unit_id, entry, now)
}

/// One chunk's persisted classification, read back from a staged
/// item's per-embed records ([`stage_classified_artifact`]): the
/// chunk's exact byte range in the item's text, its effective
/// sensitivity, and whether the boundary/decision behind it was
/// uncertain (and therefore handled at `secret-bearing`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkClassification {
    pub start: usize,
    pub end: usize,
    pub sensitivity: Sensitivity,
    pub uncertain: bool,
    pub embed_id: String,
}

/// The provenance actor recorded on classification records written
/// by [`stage_classified_artifact`].
pub const CLASSIFICATION_ACTOR: &str = "atlas-stage";

/// The decision method recorded on those records: the records are
/// the staging-time composition of the producer's declared segments
/// through the boundary splitter (#12403).
pub const CLASSIFICATION_METHOD: &str = "boundary-split";

/// Build the #12408 classification record for one staged chunk, as
/// JSON. The chunk's exact byte range and uncertainty ride as record
/// extras (`start` / `end` / `uncertain`), which
/// [`ClassificationRecord`] preserves verbatim, so the persisted
/// record carries offsets and effective sensitivity together.
fn classification_record_json(
    source: &str,
    idx: usize,
    start: usize,
    end: usize,
    sensitivity: Sensitivity,
    uncertain: bool,
    now: &str,
) -> serde_json::Value {
    let mut extras = serde_json::Map::new();
    extras.insert("start".into(), serde_json::json!(start));
    extras.insert("end".into(), serde_json::json!(end));
    extras.insert("uncertain".into(), serde_json::json!(uncertain));
    ClassificationRecord {
        schema_version: 1,
        embed_id: format!("{source}#chunk-{idx}"),
        sensitivity,
        disposition: None,
        provenance: ClassificationProvenance {
            actor: CLASSIFICATION_ACTOR.to_string(),
            decided_at: now.to_string(),
            method: CLASSIFICATION_METHOD.to_string(),
            method_version: "1".to_string(),
            config_version: "1".to_string(),
            evidence_refs: vec![source.to_string()],
            previous_decision_id: None,
            extras: serde_json::Map::new(),
        },
        extras,
    }
    .to_json()
}

/// Read one persisted classification record back, failing closed: a
/// record that does not parse, or whose uncertainty flag is absent,
/// is `secret-bearing` and uncertain (the #12408 convention: an
/// unrecognized sensitivity already resolves to `secret-bearing`
/// inside [`ClassificationRecord::parse`]).
fn chunk_classification_from_json(value: &serde_json::Value) -> ChunkClassification {
    let Ok(record) = ClassificationRecord::parse(value.clone()) else {
        return ChunkClassification {
            start: 0,
            end: 0,
            sensitivity: Sensitivity::SecretBearing,
            uncertain: true,
            embed_id: String::new(),
        };
    };
    ChunkClassification {
        start: record
            .extras
            .get("start")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize,
        end: record
            .extras
            .get("end")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize,
        sensitivity: record.sensitivity,
        uncertain: record
            .extras
            .get("uncertain")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        embed_id: record.embed_id,
    }
}

/// The persisted per-chunk classifications of one staged item, in
/// chunk order: `None` for an item staged without classification
/// (the legacy [`stage_artifact`] path — promotion falls back to the
/// item's recorded tier for those), `Some` for an item staged by
/// [`stage_classified_artifact`]. Malformed records fail closed per
/// record (see [`chunk_classification_from_json`]).
pub fn chunk_classifications(
    conn: &Connection,
    item_id: i64,
) -> Result<Option<Vec<ChunkClassification>>> {
    let meta: Option<String> = conn
        .query_row(
            "SELECT meta FROM items WHERE id = ?1",
            params![item_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| Error::backend(format!("staging: reading item {item_id} meta: {e}")))?
        .flatten();
    let Some(meta) = meta else {
        return Ok(None);
    };
    let meta: serde_json::Value = serde_json::from_str(&meta)
        .map_err(|e| Error::backend(format!("staging: item {item_id} meta is not JSON: {e}")))?;
    let Some(records) = meta.get("classification").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    Ok(Some(
        records.iter().map(chunk_classification_from_json).collect(),
    ))
}

/// Split one boundary chunk's byte range further at line
/// boundaries so no piece exceeds [`MAX_CHUNK_BYTES`]. Pieces stay
/// inside the chunk's range, so they inherit its classification and
/// can never cross a boundary; a single line over the ceiling stays
/// whole (no boundary exists inside it), as in
/// [`chunk_artifact_text`].
fn split_range_at_lines(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let piece = &text[start..end];
    let mut out = Vec::new();
    let mut piece_start = 0usize;
    let mut acc = 0usize;
    for line in piece.split_inclusive('\n') {
        if acc > 0 && acc + line.len() > MAX_CHUNK_BYTES {
            out.push((start + piece_start, start + piece_start + acc));
            piece_start += acc;
            acc = 0;
        }
        acc += line.len();
    }
    if acc > 0 {
        out.push((start + piece_start, start + piece_start + acc));
    }
    out
}

/// Stage one code/config artifact whose producer declared
/// source-native classification segments (#12403 wiring, #12628):
/// like [`stage_artifact`], but the `chunks_raw` chunks are the
/// boundary-safe chunks of [`split_by_boundaries`] over the declared
/// `segments` — no chunk crosses a declared boundary, and gaps,
/// overlaps and uncertain declarations fail closed to
/// `secret-bearing` chunks — and the item's `meta` carries one #12408
/// classification record per chunk (see
/// [`classification_record_json`]) with the chunk's exact source
/// offsets and effective sensitivity. The item row's own
/// `sensitivity` keeps the [`ArtifactEntry`] semantics (the entry's
/// declared tier); the per-chunk records are the effective
/// classification promotion enforces. Segment *declaration* stays
/// with the producer (the classifier / boundary detector); staging
/// composes declarations into chunks and records.
pub fn stage_classified_artifact(
    conn: &Connection,
    unit_id: &str,
    entry: &ArtifactEntry,
    segments: &[ClassificationSegment],
    now: &str,
) -> Result<i64> {
    check_stage_schema(conn)?;
    // Final chunk ranges: boundary chunks, sub-split at line
    // boundaries only where one chunk exceeds the chunk ceiling.
    let mut ranges: Vec<(usize, usize, Sensitivity, bool)> = Vec::new();
    for chunk in split_by_boundaries(&entry.text, segments) {
        if chunk.end - chunk.start <= MAX_CHUNK_BYTES {
            ranges.push((chunk.start, chunk.end, chunk.sensitivity, chunk.uncertain));
        } else {
            for (s, e) in split_range_at_lines(&entry.text, chunk.start, chunk.end) {
                ranges.push((s, e, chunk.sensitivity, chunk.uncertain));
            }
        }
    }
    let source = artifact_source(entry);
    let records: Vec<serde_json::Value> = ranges
        .iter()
        .enumerate()
        .map(|(idx, &(start, end, sensitivity, uncertain))| {
            classification_record_json(&source, idx, start, end, sensitivity, uncertain, now)
        })
        .collect();
    let meta = serde_json::json!({
        "repo": entry.repo,
        "path": entry.path,
        "commit_sha": entry.commit_sha,
        "kind": entry.kind,
        "language": entry.language,
        "unit": "boundary",
        "embedded_at": now,
        "chunks": ranges.len(),
        "classification": records,
    })
    .to_string();
    conn.execute(
        "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            unit_id,
            entry.kind,
            source,
            entry.text,
            entry.sensitivity.name(),
            now,
            meta,
        ],
    )
    .map_err(|e| Error::backend(format!("staging: inserting item for {}: {e}", entry.path)))?;
    let item_id = conn.last_insert_rowid();
    for (idx, &(start, end, _, _)) in ranges.iter().enumerate() {
        let chunk = &entry.text[start..end];
        conn.execute(
            "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item_id, idx as i64, chunk, sha256_hex(chunk), now],
        )
        .map_err(|e| {
            Error::backend(format!(
                "staging: inserting chunk {idx} for {}: {e}",
                entry.path
            ))
        })?;
    }
    Ok(item_id)
}

/// [`stage_classified_artifact`] against a staging DB file: the
/// form the `atlas-stage ingest --segments` tool calls.
pub fn stage_classified_artifact_in_db(
    db: &Path,
    unit_id: &str,
    entry: &ArtifactEntry,
    segments: &[ClassificationSegment],
    now: &str,
) -> Result<i64> {
    let conn = open(db)?;
    stage_classified_artifact(&conn, unit_id, entry, segments, now)
}

/// Read back the items staged in a v1 staging DB, optionally only
/// one unit's, each with its chunk count.
pub fn list_staged_items(conn: &Connection, unit_id: Option<&str>) -> Result<Vec<StagedItem>> {
    check_stage_schema(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.unit_id, i.kind, i.source, i.text, i.sensitivity, i.created_at, i.meta, \
             (SELECT COUNT(*) FROM chunks_raw c WHERE c.item_id = i.id) \
             FROM items i WHERE (?1 IS NULL OR i.unit_id = ?1) ORDER BY i.id",
        )
        .map_err(|e| Error::backend(format!("staging: listing items: {e}")))?;
    let rows = stmt
        .query_map(params![unit_id], |r| {
            Ok(StagedItem {
                id: r.get(0)?,
                unit_id: r.get(1)?,
                kind: r.get(2)?,
                source: r.get(3)?,
                text: r.get(4)?,
                sensitivity: r.get(5)?,
                created_at: r.get(6)?,
                meta: r.get(7)?,
                chunk_count: r.get(8)?,
            })
        })
        .map_err(|e| Error::backend(format!("staging: listing items: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| Error::backend(format!("staging: listing items: {e}")))?);
    }
    Ok(out)
}

/// [`list_staged_items`] against a staging DB file.
pub fn list_staged_items_in_db(db: &Path, unit_id: Option<&str>) -> Result<Vec<StagedItem>> {
    let conn = open(db)?;
    list_staged_items(&conn, unit_id)
}

// ---------------------------------------------------------- registry --

/// The staging-unit registry (design 4.1: unit id → staging DB, owner,
/// sensitivity level, state, schema version). Backed by a libSQL/SQLite
/// file so the foundation works offline; the internal-core-tier
/// placement is the deployment choice, not this API's.
pub struct Registry {
    conn: Connection,
}

impl Registry {
    /// Open (creating if needed) a registry at `path` and ensure its
    /// v1 table exists.
    pub fn open(path: &Path) -> Result<Registry> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::backend(format!("registry: creating {}: {e}", parent.display()))
            })?;
        }
        let conn = open(path)?;
        ensure_registry(&conn)?;
        Ok(Registry { conn })
    }

    /// The registry's connection, for sibling modules that keep their
    /// own tables in the registry DB (the ingest gate's records).
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// An in-memory registry (tests, ephemeral tools).
    pub fn in_memory() -> Result<Registry> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::backend(format!("registry: opening memory db: {e}")))?;
        ensure_registry(&conn)?;
        Ok(Registry { conn })
    }

    /// The one provisioning tool: create the unit's staging DB, apply
    /// the v1 schema, and register the unit as `open`. The unit's
    /// recorded embedding-model identity is the mandated model
    /// (#8623 slice 2): every corpus starts unified, and a model
    /// change afterwards is an explicit re-embed, never silent drift.
    pub fn provision(
        &self,
        staging_db: &Path,
        unit_id: &str,
        owner: &str,
        sensitivity: Tier,
        now: &str,
    ) -> Result<StagingUnit> {
        provision_staging_db(staging_db)?;
        // Persist an absolute path: a relative one is only meaningful
        // in the provisioning process's working directory, so a
        // registry read from any other directory would resolve the
        // unit's db_uri to a different (or nonexistent) database
        // (Codex P2, PR #7365).
        let db_path = absolute_path(staging_db);
        let unit = StagingUnit {
            unit_id: unit_id.to_string(),
            db_uri: db_path.to_string_lossy().into_owned(),
            owner: owner.to_string(),
            sensitivity,
            state: UnitState::Open,
            schema_version: STAGE_SCHEMA_VERSION,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            embedding_model: EMBEDDING_MODEL_ID.to_string(),
        };
        // If registration fails, remove the just-provisioned database:
        // leaving it behind would strand an unregistered staging DB
        // that a retry can neither re-provision (the non-empty check
        // refuses) nor recover through the CLI (Codex P2, PR #7365).
        // Cleanup is best-effort; the registration error is the one
        // the caller needs.
        if let Err(e) = self.register(&unit) {
            remove_staging_db(&db_path);
            return Err(e);
        }
        Ok(unit)
    }

    /// Insert (or replace) a unit record. Re-registering a unit id
    /// overwrites it — the registry is a mapping, not an append log.
    /// The record carries the unit's embedding-model identity (#8623
    /// slice 2); callers that re-register an existing corpus must pass
    /// the recorded identity back unchanged unless the corpus was
    /// just rebuilt under a new model by the re-embed job.
    pub fn register(&self, unit: &StagingUnit) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO staging_units \
                 (unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at, embedding_model) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT(unit_id) DO UPDATE SET \
                   db_uri = excluded.db_uri, owner = excluded.owner, \
                   sensitivity = excluded.sensitivity, state = excluded.state, \
                   schema_version = excluded.schema_version, updated_at = excluded.updated_at, \
                   embedding_model = excluded.embedding_model",
                params![
                    unit.unit_id,
                    unit.db_uri,
                    unit.owner,
                    unit.sensitivity.name(),
                    unit.state.name(),
                    unit.schema_version,
                    unit.created_at,
                    unit.updated_at,
                    unit.embedding_model,
                ],
            )
            .map_err(|e| Error::backend(format!("registry: registering {}: {e}", unit.unit_id)))?;
        Ok(())
    }

    /// The ingestion-time embedding-identity check (#8623 slice 2):
    /// refuse a corpus whose recorded model is not the mandated one.
    /// Ingestion paths call this for an already-registered corpus
    /// before staging into it, so a corpus recorded under a retired
    /// model fails loudly instead of mixing incomparable vectors into
    /// one corpus. A corpus recorded under the mandated model passes.
    pub fn assert_corpus_model(&self, unit_id: &str) -> Result<()> {
        let unit = self.lookup(unit_id)?.ok_or_else(|| {
            Error::fatal(format!("registry: no staging unit {unit_id:?} to check"))
        })?;
        if unit.embedding_model == EMBEDDING_MODEL_ID {
            return Ok(());
        }
        Err(Error::fatal(format!(
            "registry: refusing ingestion into corpus {unit_id}: its recorded embedding_model `{}` is not the mandated `{EMBEDDING_MODEL_ID}`; vectors from different embedding models are not comparable — rebuild the corpus with the re-embed job first",
            unit.embedding_model
        )))
    }

    /// Record a corpus's embedding-model identity after an explicit
    /// rebuild under a new model (#8623 slice 2): the re-embed job's
    /// close-out. Refuses an unknown unit. This is the only path that
    /// changes a live corpus's identity — ingestion never does.
    pub fn record_embedding_model(&self, unit_id: &str, model: &str, now: &str) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE staging_units SET embedding_model = ?2, updated_at = ?3 WHERE unit_id = ?1",
                params![unit_id, model, now],
            )
            .map_err(|e| {
                Error::backend(format!("registry: recording embedding model for {unit_id}: {e}"))
            })?;
        if changed == 0 {
            return Err(Error::fatal(format!(
                "registry: no staging unit {unit_id:?} to record an embedding model for"
            )));
        }
        Ok(())
    }

    /// Look a unit up by id.
    pub fn lookup(&self, unit_id: &str) -> Result<Option<StagingUnit>> {
        self.conn
            .query_row(
                "SELECT unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at, embedding_model \
                 FROM staging_units WHERE unit_id = ?1",
                params![unit_id],
                row_to_unit,
            )
            .optional()
            .map_err(|e| Error::backend(format!("registry: looking up {unit_id}: {e}")))
    }

    /// Everything the aggregator's poll reads: units in a given state.
    pub fn units_in_state(&self, state: UnitState) -> Result<Vec<StagingUnit>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at, embedding_model \
                 FROM staging_units WHERE state = ?1 ORDER BY unit_id",
            )
            .map_err(|e| Error::backend(format!("registry: listing {}: {e}", state.name())))?;
        let rows = stmt
            .query_map(params![state.name()], row_to_unit)
            .map_err(|e| Error::backend(format!("registry: listing {}: {e}", state.name())))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|e| {
                    Error::backend(format!("registry: listing {}: {e}", state.name()))
                })?,
            );
        }
        Ok(out)
    }

    /// Move a unit to a new lifecycle state.
    pub fn set_state(&self, unit_id: &str, state: UnitState, now: &str) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE staging_units SET state = ?2, updated_at = ?3 WHERE unit_id = ?1",
                params![unit_id, state.name(), now],
            )
            .map_err(|e| Error::backend(format!("registry: setting {unit_id} → {state}: {e}")))?;
        if changed == 0 {
            return Err(Error::fatal(format!(
                "registry: no staging unit {unit_id:?} to move to {state}"
            )));
        }
        Ok(())
    }

    /// Return the durable promotion record for a unit, if it has been promoted.
    pub fn merge_manifest(&self, unit_id: &str) -> Result<Option<MergeManifest>> {
        self.conn.query_row(
            "SELECT unit_id, rows_in, rows_promoted, content_hash, embedding_model, promoted_at \
             FROM merge_manifests WHERE unit_id = ?1",
            params![unit_id],
            |r| Ok(MergeManifest {
                unit_id: r.get(0)?, rows_in: r.get(1)?, rows_promoted: r.get(2)?,
                content_hash: r.get(3)?, embedding_model: r.get(4)?, promoted_at: r.get(5)?,
            }),
        ).optional().map_err(|e| Error::backend(format!("registry: reading merge manifest for {unit_id}: {e}")))
    }

    /// Promote a unit into a tier: the merge-on-close transition to
    /// [`UnitState::Merged`], gated on the embedding-model contract
    /// (#7648, contract point M6 of the #6060 decision record — one
    /// embedding model for every Atlas tier).
    ///
    /// Before the unit's state moves, the model recorded in its
    /// database's `atlas_manifest` `embedding_model` key (stamped by
    /// the embed pipeline when the unit's vectors were written, the
    /// M1/M2 key) must equal the mandated model
    /// ([`EMBEDDING_MODEL_ID`]). A unit recording a different model is
    /// refused with an error naming the unit and both model ids, and
    /// a unit recording no model at all is refused as unknown
    /// provenance — the same refusal shard discovery applies
    /// (#7646), so a source whose embedding step ran a non-mandated
    /// model cannot be promoted silently (#6019 ingestion inherits
    /// this gate by promoting through here). The registry's own
    /// per-corpus identity record is checked first (#8623 slice 2):
    /// a stale registry record refuses even when the staging DB was
    /// freshly stamped. On refusal the unit's state does not change.
    ///
    /// A promotion that is accepted performs the design's close-out
    /// (#5669 criterion 4, design 4.1):
    ///
    /// 1. the unit's staging DB is branched by copying a consistent
    ///    snapshot beside it as the unit's archive
    ///    ([`branch_staging_db_as_archive`]) — a failing archive is a
    ///    failed promotion, so a `merged` unit always has its archive;
    /// 2. the [`MergeManifest`] is written and the unit's registry row
    ///    moves to `merged` in one transaction — the unit is no longer
    ///    returned by any active-state query
    ///    ([`Registry::units_in_state`]), while its manifest and
    ///    archive remain the durable record.
    ///
    /// The archive is written before the registry transition commits;
    /// if that commit fails the unit stays active and the promotion
    /// can be retried (the archive copy is simply refreshed).
    pub fn promote(&self, unit_id: &str, now: &str) -> Result<()> {
        let unit = self.lookup(unit_id)?.ok_or_else(|| {
            Error::fatal(format!("registry: no staging unit {unit_id:?} to promote"))
        })?;
        if unit.state == UnitState::Merged {
            return Err(Error::fatal(format!(
                "registry: unit {unit_id:?} is already merged"
            )));
        }
        if unit.embedding_model != EMBEDDING_MODEL_ID {
            return Err(Error::fatal(format!(
                "registry: refusing to promote unit {unit_id}: its recorded embedding_model `{}`, not the mandated `{EMBEDDING_MODEL_ID}`; vectors from different embedding models are not comparable — rebuild the corpus with the re-embed job first",
                unit.embedding_model
            )));
        }
        assert_unit_embedding_model(&unit)?;
        let staged = build_merge_manifest(&unit, now)?;
        branch_staging_db_as_archive(&unit)?;
        let tx = self.conn.unchecked_transaction().map_err(|e| {
            Error::backend(format!("registry: beginning promotion for {unit_id}: {e}"))
        })?;
        tx.execute(
            "INSERT INTO merge_manifests (unit_id, rows_in, rows_promoted, content_hash, embedding_model, promoted_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![staged.unit_id, staged.rows_in, staged.rows_promoted, staged.content_hash, staged.embedding_model, staged.promoted_at],
        ).map_err(|e| Error::backend(format!("registry: writing merge manifest for {unit_id}: {e}")))?;
        let changed = tx.execute(
            "UPDATE staging_units SET state = 'merged', updated_at = ?2 WHERE unit_id = ?1 AND state != 'merged'",
            params![unit_id, now],
        ).map_err(|e| Error::backend(format!("registry: closing promoted unit {unit_id}: {e}")))?;
        if changed == 0 {
            return Err(Error::fatal(format!(
                "registry: unit {unit_id:?} is already merged"
            )));
        }
        tx.commit().map_err(|e| {
            Error::backend(format!("registry: committing promotion for {unit_id}: {e}"))
        })
    }
}

/// The archive path a promotion branches a unit's staging DB to: the
/// staging DB file name with `.archive` appended, in the same
/// directory (`unit.db` → `unit.db.archive`), so the archive sits
/// beside the DB it preserves and moves with it.
pub fn staging_archive_path(db_uri: &str) -> PathBuf {
    let db = Path::new(db_uri);
    let mut name = db
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "staging.db".into());
    name.push(".archive");
    db.with_file_name(name)
}

/// Branch a unit's staging DB as its archive (#5669 criterion 4,
/// design 4.1: a merged unit is "promoted, archived by a branch,
/// dropped from active use"): copy the unit's staged set — schema
/// version, items, chunks, notes, kv — into a fresh database at
/// [`staging_archive_path`] with `VACUUM INTO`, which reads a
/// consistent snapshot and writes a complete standalone database.
/// The staging DB itself stays untouched (retained); the archive is
/// the durable, verifiable record next to it. Any previous archive
/// at the path is replaced only once the new snapshot has been
/// written (the copy goes to a temporary sibling first), so a failed
/// re-archive never leaves a half-written archive behind.
///
/// Called by [`Registry::promote`]; public so the archive of a
/// promoted unit can be recomputed or inspected by later slices.
pub fn branch_staging_db_as_archive(unit: &StagingUnit) -> Result<PathBuf> {
    let archive = staging_archive_path(&unit.db_uri);
    let name = archive
        .file_name()
        .map(|n| n.to_os_string())
        .ok_or_else(|| {
            Error::fatal(format!(
                "staging: archive path {} has no file name",
                archive.display()
            ))
        })?;
    let mut tmp_name = name;
    tmp_name.push(".tmp");
    let tmp = archive.with_file_name(tmp_name);
    // Clear any stale temporary from an earlier run (VACUUM INTO needs
    // a bare path). The previous archive stays in place until the
    // rename below replaces it, as the doc above promises.
    remove_staging_db(&tmp);
    let conn = open(Path::new(&unit.db_uri))?;
    let target = sql_string_literal(&tmp.to_string_lossy());
    conn.execute_batch(&format!("VACUUM INTO {target}"))
        .map_err(|e| {
            Error::backend(format!(
                "staging: branching {} as archive for unit {}: {e}",
                unit.db_uri, unit.unit_id
            ))
        })?;
    // A sidecar left by an earlier reader of the old archive must not
    // be replayed against the new snapshot: drop only the sidecars here,
    // the old archive itself is replaced atomically by the rename.
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = archive.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
    std::fs::rename(&tmp, &archive).map_err(|e| {
        Error::backend(format!(
            "staging: moving archive snapshot into place at {}: {e}",
            archive.display()
        ))
    })?;
    Ok(archive)
}

/// Quote a value as a single-quoted SQL string literal, doubling
/// embedded quotes per SQLite's literal syntax (`VACUUM INTO` takes
/// its target as a string expression).
fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Hash the complete staged set in a stable, typed encoding. The table
/// and column order is explicit and rows are ordered by rowid; SQLite
/// storage classes are tagged so NULL, integers, text and blobs cannot
/// collide. Each table's row count contributes to the manifest counts.
fn build_merge_manifest(unit: &StagingUnit, now: &str) -> Result<MergeManifest> {
    use rusqlite::types::ValueRef;
    use sha2::{Digest, Sha256};
    let conn = open(Path::new(&unit.db_uri))?;
    let tables = [
        (
            "items",
            "SELECT id, unit_id, kind, source, text, sensitivity, created_at, meta FROM items WHERE unit_id = ?1 ORDER BY rowid",
        ),
        (
            "chunks_raw",
            "SELECT c.id, c.item_id, c.idx, c.text, c.content_hash, c.created_at FROM chunks_raw c JOIN items i ON i.id = c.item_id WHERE i.unit_id = ?1 ORDER BY c.rowid",
        ),
        (
            "notes",
            "SELECT id, unit_id, body, created_at, meta FROM notes WHERE unit_id = ?1 ORDER BY rowid",
        ),
        ("kv", "SELECT key, value, updated_at FROM kv ORDER BY rowid"),
    ];
    let mut hasher = Sha256::new();
    let mut rows_in = 0i64;
    for (table, query) in tables {
        hasher.update((table.len() as u64).to_le_bytes());
        hasher.update(table.as_bytes());
        let mut stmt = conn
            .prepare(query)
            .map_err(|e| Error::backend(format!("staging: preparing {table} for manifest: {e}")))?;
        let columns = stmt.column_count();
        let mut rows = if query.contains("?1") {
            stmt.query(params![unit.unit_id])
        } else {
            stmt.query([])
        }
        .map_err(|e| Error::backend(format!("staging: reading {table} for manifest: {e}")))?;
        while let Some(row) = rows
            .next()
            .map_err(|e| Error::backend(format!("staging: reading {table} for manifest: {e}")))?
        {
            rows_in += 1;
            hasher.update([0xff]);
            for col in 0..columns {
                match row.get_ref(col).map_err(|e| {
                    Error::backend(format!("staging: reading {table} manifest value: {e}"))
                })? {
                    ValueRef::Null => hasher.update([0]),
                    ValueRef::Integer(v) => {
                        hasher.update([1]);
                        hasher.update(v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        hasher.update([2]);
                        hasher.update(v.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(v) => {
                        hasher.update([3]);
                        hasher.update((v.len() as u64).to_le_bytes());
                        hasher.update(v);
                    }
                    ValueRef::Blob(v) => {
                        hasher.update([4]);
                        hasher.update((v.len() as u64).to_le_bytes());
                        hasher.update(v);
                    }
                }
            }
        }
    }
    if rows_in == 0 {
        return Err(Error::fatal(format!(
            "staging: refusing to promote empty unit {}",
            unit.unit_id
        )));
    }
    let content_hash = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(MergeManifest {
        unit_id: unit.unit_id.clone(),
        rows_in,
        rows_promoted: rows_in,
        content_hash,
        embedding_model: EMBEDDING_MODEL_ID.to_string(),
        promoted_at: now.to_string(),
    })
}

/// The staging unit's recorded `embedding_model`, if its database
/// carries an `atlas_manifest` with that key.
fn recorded_embedding_model(db_uri: &str) -> Result<Option<String>> {
    let conn = open(Path::new(db_uri))?;
    let has_manifest: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'atlas_manifest')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| Error::backend(format!("staging: reading {db_uri} manifest: {e}")))?;
    if !has_manifest {
        return Ok(None);
    }
    conn.query_row(
        "SELECT value FROM atlas_manifest WHERE \"key\" = 'embedding_model'",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| Error::backend(format!("staging: reading {db_uri} embedding_model: {e}")))
}

/// The M6 gate behind [`Registry::promote`]: the unit's recorded
/// embedding model must be the mandated one.
fn assert_unit_embedding_model(unit: &StagingUnit) -> Result<()> {
    match recorded_embedding_model(&unit.db_uri)? {
        Some(got) if got == EMBEDDING_MODEL_ID => Ok(()),
        Some(got) => Err(Error::fatal(format!(
            "staging: refusing to promote unit {}: its atlas_manifest records embedding_model `{got}`, not the mandated `{EMBEDDING_MODEL_ID}`; vectors from different embedding models are not comparable",
            unit.unit_id
        ))),
        None => Err(Error::fatal(format!(
            "staging: refusing to promote unit {}: its database records no `embedding_model` in atlas_manifest, so its embedding provenance is unknown; the mandated model is `{EMBEDDING_MODEL_ID}`",
            unit.unit_id
        ))),
    }
}

fn row_to_unit(row: &rusqlite::Row<'_>) -> rusqlite::Result<StagingUnit> {
    let state_s: String = row.get(4)?;
    let sens_s: String = row.get(3)?;
    let state = UnitState::parse(&state_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown staging state {state_s:?}").into(),
        )
    })?;
    let sensitivity = Tier::parse(&sens_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            format!("unknown sensitivity tier {sens_s:?}").into(),
        )
    })?;
    Ok(StagingUnit {
        unit_id: row.get(0)?,
        db_uri: row.get(1)?,
        owner: row.get(2)?,
        sensitivity,
        state,
        schema_version: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        embedding_model: row.get(8)?,
    })
}

/// Resolve `path` to an absolute path. The file exists by the time
/// provisioning persists it, so canonicalize is the primary route;
/// fall back to anchoring a relative path at the current directory.
fn absolute_path(path: &Path) -> std::path::PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

/// Best-effort removal of a staging database file and its SQLite
/// sidecars (-wal / -shm / -journal). Errors are ignored on purpose:
/// this runs while returning a registration error, and that error —
/// not a cleanup failure — is what the caller must see.
fn remove_staging_db(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(sidecar));
    }
}

fn ensure_registry(conn: &Connection) -> Result<()> {
    conn.execute_batch(REGISTRY_V1_DDL)
        .map_err(|e| Error::backend(format!("registry: applying v1 DDL: {e}")))?;
    // Slice-2 migration (#8623): pre-slice-2 registry databases were
    // created by a DDL without the per-corpus `embedding_model`
    // column. Fresh databases already have it (see the v1 DDL); this
    // migrates the old ones in place, defaulting every existing
    // corpus to the mandated model — the one model the pre-slice-2
    // gate ever allowed through promotion, so the default rewrites
    // no history.
    let has_column: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('staging_units') WHERE name = 'embedding_model')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| Error::backend(format!("registry: reading staging_units columns: {e}")))?;
    if !has_column {
        conn.execute_batch(&format!(
            "ALTER TABLE staging_units ADD COLUMN embedding_model TEXT NOT NULL DEFAULT '{EMBEDDING_MODEL_ID}'"
        ))
        .map_err(|e| Error::backend(format!("registry: adding embedding_model column: {e}")))?;
    }
    Ok(())
}

pub(crate) fn open(path: &Path) -> Result<Connection> {
    Connection::open(path)
        .map_err(|e| Error::backend(format!("staging: opening {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle: the filesystem after a failed provision. When the
    /// registry INSERT fails (here: the registry table is gone, the
    /// deterministic stand-in for a lock timeout), the newly
    /// provisioned staging DB must not be left behind — a retry of
    /// the same provision command must be possible once the
    /// transient failure clears (Codex P2, PR #7365).
    #[test]
    fn failed_registration_removes_the_new_staging_db() {
        let dir =
            std::env::temp_dir().join(format!("atlas-stage-unit-{}-regfail", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let conn = Connection::open_in_memory().expect("memory registry");
        ensure_registry(&conn).expect("registry ddl");
        conn.execute_batch("DROP TABLE staging_units")
            .expect("drop registry table so register() fails");
        let reg = Registry { conn };

        let db = dir.join("unit.db");
        let err = reg
            .provision(
                &db,
                "task-12742",
                "repo-worker-code",
                Tier::Internal,
                "2026-11-08T00:00:00Z",
            )
            .expect_err("provision must fail when registration fails");
        assert!(
            err.to_string().contains("registry: registering"),
            "unexpected error: {err}"
        );
        assert!(
            !db.exists(),
            "a failed registration must not strand the provisioned staging DB"
        );

        // And the retry path works once registration can succeed:
        // the same provision command against a healthy registry
        // succeeds instead of dying on the non-empty check.
        let reg2 = Registry::in_memory().expect("healthy registry");
        reg2.provision(
            &db,
            "task-12742",
            "repo-worker-code",
            Tier::Internal,
            "2026-11-08T00:00:00Z",
        )
        .expect("retry after cleanup must succeed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn stamp_embedding_model(db: &Path, model: &str) {
        let conn = Connection::open(db).expect("open unit db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT NOT NULL)",
        )
        .expect("manifest ddl");
        conn.execute(
            "INSERT OR REPLACE INTO atlas_manifest VALUES ('embedding_model', ?1)",
            params![model],
        )
        .expect("stamp model");
    }

    fn provision_unit(dir: &Path, reg: &Registry, unit_id: &str) -> std::path::PathBuf {
        let db = dir.join(format!("{unit_id}.db"));
        reg.provision(
            &db,
            unit_id,
            "repo-worker-code",
            Tier::Internal,
            "2026-11-10T00:00:00Z",
        )
        .expect("provision unit");
        db
    }

    /// Oracle: the #7648 acceptance, both directions. A unit whose
    /// manifest names the mandated model promotes (state becomes
    /// merged); a unit naming any other model is refused with an
    /// error naming the unit and both model ids, and stays
    /// unpromoted; a unit with no recorded model is refused as
    /// unknown provenance.
    #[test]
    fn promote_asserts_the_mandated_embedding_model() {
        let dir =
            std::env::temp_dir().join(format!("atlas-stage-unit-{}-promote", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let reg = Registry::in_memory().expect("registry");

        // Mandated model: promotion succeeds exactly as a state move.
        let db = provision_unit(&dir, &reg, "unit-good");
        stamp_embedding_model(&db, EMBEDDING_MODEL_ID);
        Connection::open(&db).expect("open staged unit")
            .execute("INSERT INTO items (unit_id, kind, text, sensitivity, created_at) VALUES ('unit-good', 'test', 'payload', 'internal', 'now')", [])
            .expect("stage test item");
        reg.promote("unit-good", "2026-11-10T01:00:00Z")
            .expect("mandated model must promote");
        assert_eq!(
            reg.lookup("unit-good")
                .expect("lookup")
                .expect("unit")
                .state,
            UnitState::Merged
        );
        let promoted = reg
            .merge_manifest("unit-good")
            .expect("manifest query")
            .expect("manifest persisted");
        assert_eq!(promoted.rows_in, 1);
        assert_eq!(promoted.rows_promoted, 1);
        assert_eq!(promoted.embedding_model, EMBEDDING_MODEL_ID);
        assert_eq!(promoted.promoted_at, "2026-11-10T01:00:00Z");
        // Independent oracle: the single items row's canonical encoding is
        // the table tag, row marker, and each typed SQLite value in SELECT order.
        use sha2::{Digest, Sha256};
        // The query includes id, NULL source, and NULL meta around the
        // inserted fields; encode those values independently as well.
        let mut expected = Sha256::new();
        expected.update(("items".len() as u64).to_le_bytes());
        expected.update(b"items");
        expected.update([0xff]);
        expected.update([1]);
        expected.update(1i64.to_le_bytes());
        expected.update([3]);
        expected.update((9u64).to_le_bytes());
        expected.update(b"unit-good");
        expected.update([3]);
        expected.update(4u64.to_le_bytes());
        expected.update(b"test");
        expected.update([0]);
        expected.update([3]);
        expected.update(7u64.to_le_bytes());
        expected.update(b"payload");
        expected.update([3]);
        expected.update(8u64.to_le_bytes());
        expected.update(b"internal");
        expected.update([3]);
        expected.update(3u64.to_le_bytes());
        expected.update(b"now");
        expected.update([0]);
        for table in ["chunks_raw", "notes", "kv"] {
            expected.update((table.len() as u64).to_le_bytes());
            expected.update(table.as_bytes());
        }
        let expected_hash: String = expected
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(promoted.content_hash, expected_hash);
        // Close-out, part 1: the unit is out of the active registry
        // (the aggregator's poll only reads open/closing), while its
        // manifest remains retrievable.
        for state in [UnitState::Open, UnitState::Closing] {
            let active = reg.units_in_state(state).expect("active state query");
            assert!(
                active.iter().all(|u| u.unit_id != "unit-good"),
                "a merged unit must not be returned as active ({state})"
            );
        }
        // Close-out, part 2: the staging DB was branched as the
        // unit's archive beside it; the original is retained.
        let archive = staging_archive_path(&db.to_string_lossy());
        assert!(archive.exists(), "promotion must branch the archive");
        assert!(db.exists(), "the staging DB is retained, not moved");
        let archive_bytes = std::fs::read(&archive).expect("archive bytes");
        assert!(reg.promote("unit-good", "2026-11-10T02:00:00Z").is_err());
        assert_eq!(
            reg.merge_manifest("unit-good").expect("manifest query"),
            Some(promoted)
        );
        // A refused repeat promotion changes nothing, archive included.
        assert_eq!(
            std::fs::read(&archive).expect("archive bytes after refusal"),
            archive_bytes,
            "a refused promotion must not touch the archive"
        );

        // Non-mandated model: refusal naming unit and both ids, and
        // the unit stays unpromoted.
        let db = provision_unit(&dir, &reg, "unit-bad");
        stamp_embedding_model(&db, "some-other-model-v9");
        let err = reg
            .promote("unit-bad", "2026-11-10T01:00:00Z")
            .expect_err("non-mandated model must be refused");
        let msg = err.to_string();
        assert!(msg.contains("unit-bad"), "{msg}");
        assert!(msg.contains("some-other-model-v9"), "{msg}");
        assert!(msg.contains(EMBEDDING_MODEL_ID), "{msg}");
        assert_eq!(
            reg.lookup("unit-bad").expect("lookup").expect("unit").state,
            UnitState::Open
        );
        assert!(
            reg.merge_manifest("unit-bad")
                .expect("manifest query")
                .is_none()
        );
        assert!(
            !staging_archive_path(&db.to_string_lossy()).exists(),
            "a refused promotion must not branch an archive"
        );

        // No recorded model: refused as unknown provenance, state
        // unchanged.
        let _db = provision_unit(&dir, &reg, "unit-unknown");
        let err = reg
            .promote("unit-unknown", "2026-11-10T01:00:00Z")
            .expect_err("unknown provenance must be refused");
        assert!(err.to_string().contains("unit-unknown"), "{err}");
        assert_eq!(
            reg.lookup("unit-unknown")
                .expect("lookup")
                .expect("unit")
                .state,
            UnitState::Open
        );
        assert!(
            reg.merge_manifest("unit-unknown")
                .expect("manifest query")
                .is_none()
        );
        assert!(
            !staging_archive_path(&_db.to_string_lossy()).exists(),
            "an unknown-provenance refusal must not branch an archive"
        );

        let empty_db = provision_unit(&dir, &reg, "unit-empty");
        stamp_embedding_model(&empty_db, EMBEDDING_MODEL_ID);
        assert!(reg.promote("unit-empty", "2026-11-10T01:00:00Z").is_err());
        assert_eq!(
            reg.lookup("unit-empty")
                .expect("lookup")
                .expect("unit")
                .state,
            UnitState::Open
        );
        assert!(
            reg.merge_manifest("unit-empty")
                .expect("manifest query")
                .is_none()
        );
        assert!(
            !staging_archive_path(&empty_db.to_string_lossy()).exists(),
            "an empty-unit refusal must not branch an archive"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Oracle: the archive itself, read back as a database. After
    /// promotion the branched archive holds the same staged set the
    /// manifest counted, and the manifest's content hash recomputes
    /// from the archive's own rows — the round-trip verification the
    /// design's audit trail is defined against (#5669 criterion 4).
    #[test]
    fn promotion_archive_preserves_the_staged_set_and_verifies() {
        let dir = std::env::temp_dir().join(format!("atlas-stage-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let reg = Registry::in_memory().expect("registry");
        let db = provision_unit(&dir, &reg, "unit-archived");
        let entry = artifact_entry("argo.internal:\n  type: A\n  content: 10.10.32.1\n");
        stage_artifact_in_db(&db, "unit-archived", &entry, "2026-11-17T00:00:00Z")
            .expect("stage artifact");
        stamp_embedding_model(&db, EMBEDDING_MODEL_ID);
        reg.promote("unit-archived", "2026-11-17T01:00:00Z")
            .expect("promote staged unit");
        let manifest = reg
            .merge_manifest("unit-archived")
            .expect("manifest query")
            .expect("manifest persisted");

        let archive = staging_archive_path(&db.to_string_lossy());
        assert!(archive.exists(), "the unit's archive must exist");
        let conn = open(&archive).expect("open archive");
        // The archive is a complete staging DB: schema version 1 and
        // the full staged set (the embed pipeline's atlas_manifest
        // table travels with it, as in any embedded staging DB).
        assert_eq!(
            schema_version(&conn).expect("archive schema version"),
            STAGE_SCHEMA_VERSION
        );
        let tables = user_tables(&conn).expect("archive tables");
        for table in STAGE_V1_TABLES {
            assert!(tables.contains(table), "archive is missing table {table}");
        }
        let items: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE unit_id = ?1",
                params!["unit-archived"],
                |r| r.get(0),
            )
            .expect("archive item count");
        assert_eq!(items, 1);
        let chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks_raw", [], |r| r.get(0))
            .expect("archive chunk count");
        assert_eq!(chunks, 1);
        // Independent hash oracle over the archive's own rows: the
        // manifest hash and counts recompute from the archive exactly.
        let archived = StagingUnit {
            unit_id: "unit-archived".to_string(),
            db_uri: archive.to_string_lossy().into_owned(),
            owner: "repo-worker-code".to_string(),
            sensitivity: Tier::Internal,
            state: UnitState::Merged,
            schema_version: STAGE_SCHEMA_VERSION,
            created_at: "2026-11-10T00:00:00Z".to_string(),
            updated_at: "2026-11-17T01:00:00Z".to_string(),
            embedding_model: EMBEDDING_MODEL_ID.to_string(),
        };
        let recomputed =
            build_merge_manifest(&archived, "2026-11-17T01:00:00Z").expect("recompute manifest");
        assert_eq!(recomputed.rows_in, manifest.rows_in);
        assert_eq!(recomputed.content_hash, manifest.content_hash);
        // The staging DB is retained beside its archive, untouched.
        assert!(db.exists(), "the staging DB stays in place");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // ---- #1518: code/config artifact item writer ----
    // Oracle: the staged rows themselves, read back from a real
    // provisioned staging DB -- the items row carries the
    // definition's metadata and source-of-truth pointer, the
    // chunks_raw rows partition the artifact text exactly, and
    // each chunk's content_hash is the SHA-256 of its text.

    fn artifact_entry(text: &str) -> ArtifactEntry {
        ArtifactEntry {
            kind: "config".to_string(),
            repo: "developing-today/code".to_string(),
            path: "infra/dns/dns_config.yaml".to_string(),
            commit_sha: "abc1234".to_string(),
            language: "yaml".to_string(),
            sensitivity: Tier::Internal,
            text: text.to_string(),
        }
    }

    fn provisioned_conn(dir: &std::path::Path) -> Connection {
        let db = dir.join("unit.db");
        provision_staging_db(&db).expect("provision staging db");
        open(&db).expect("open staging db")
    }

    #[test]
    fn staged_small_config_is_one_whole_file_item() {
        let dir = std::env::temp_dir().join(format!("atlas-stage-writer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let conn = provisioned_conn(&dir);
        let text = "argo.internal:\n  type: A\n  content: 10.10.32.1\n";
        let item_id = stage_artifact(
            &conn,
            "task-17164",
            &artifact_entry(text),
            "2026-11-16T00:00:00Z",
        )
        .expect("stage artifact");
        let items = list_staged_items(&conn, Some("task-17164")).expect("list items");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, item_id);
        assert_eq!(item.kind, "config");
        assert_eq!(
            item.source.as_deref(),
            Some("developing-today/code:infra/dns/dns_config.yaml@abc1234")
        );
        assert_eq!(item.chunk_count, 1);
        let meta: serde_json::Value =
            serde_json::from_str(item.meta.as_deref().expect("meta")).expect("meta json");
        assert_eq!(meta["repo"], "developing-today/code");
        assert_eq!(meta["path"], "infra/dns/dns_config.yaml");
        assert_eq!(meta["commit_sha"], "abc1234");
        assert_eq!(meta["kind"], "config");
        assert_eq!(meta["language"], "yaml");
        assert_eq!(meta["unit"], "whole-file");
        // The single chunk is the whole text, hash-verified.
        let (chunk_text, chunk_hash): (String, String) = conn
            .query_row(
                "SELECT text, content_hash FROM chunks_raw WHERE item_id = ?1 AND idx = 0",
                params![item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("chunk row");
        assert_eq!(chunk_text, text);
        assert_eq!(chunk_hash, sha256_hex(text));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staged_large_config_chunks_partition_the_text_exactly() {
        let dir =
            std::env::temp_dir().join(format!("atlas-stage-writer-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let conn = provisioned_conn(&dir);
        // 200 top-level records of ~200 bytes each: ~40 KiB, over
        // the whole-file ceiling, under the per-chunk ceiling.
        let mut text = String::new();
        for i in 0..200 {
            text.push_str(&format!(
                "record_{i}:\n  type: A\n  content: 10.10.32.1\n  filler: {}\n",
                "x".repeat(150)
            ));
        }
        assert!(
            text.len() > WHOLE_FILE_MAX_BYTES,
            "fixture must exceed the whole-file ceiling"
        );
        let item_id = stage_artifact(
            &conn,
            "task-17164",
            &artifact_entry(&text),
            "2026-11-16T00:00:00Z",
        )
        .expect("stage artifact");
        let items = list_staged_items(&conn, Some("task-17164")).expect("list items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].chunk_count, 200, "one chunk per top-level record");
        let meta: serde_json::Value =
            serde_json::from_str(items[0].meta.as_deref().expect("meta")).expect("meta json");
        assert_eq!(meta["unit"], "declaration");
        // Lossless partition oracle: chunks in idx order rejoin to
        // the exact artifact bytes, and each chunk opens at a
        // top-level record line.
        let mut stmt = conn
            .prepare("SELECT text FROM chunks_raw WHERE item_id = ?1 ORDER BY idx")
            .expect("chunk query");
        let chunks: Vec<String> = stmt
            .query_map(params![item_id], |r| r.get(0))
            .expect("chunk rows")
            .collect::<std::result::Result<_, _>>()
            .expect("chunk values");
        assert_eq!(chunks.concat(), text);
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.starts_with(&format!("record_{i}:")),
                "chunk {i} must open at its top-level record"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_into_a_non_staging_db_is_refused() {
        let conn = Connection::open_in_memory().expect("memory db");
        let err = stage_artifact(
            &conn,
            "task-17164",
            &artifact_entry("a: b\n"),
            "2026-11-16T00:00:00Z",
        )
        .expect_err("a non-staging DB must refuse the write");
        assert!(
            err.to_string().contains("staging:"),
            "unexpected error: {err}"
        );
    }
}
