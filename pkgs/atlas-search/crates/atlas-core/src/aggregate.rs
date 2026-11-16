//! The staging aggregator's embed-at-promotion step (developing-today/code#5669
//! aggregator slice; first consumer developing-today/code#1518).
//!
//! The staging design (doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision
//! section 4.1) keeps staging DBs vector-free — "the aggregator embeds at
//! promotion; a staging DB never stores vectors". This module is that
//! aggregator step, scoped to one unit and one new core scan shard:
//!
//! 1. Read the unit's staged `items` + `chunks_raw` from its staging DB
//!    (raw SQL, in staging order — the read happens before any stamp, so
//!    the v1 schema invariant still holds at read time).
//! 2. Embed every chunk document-side (prompt-free, the production
//!    corpus convention; see [`crate::embed::DocEmbedder`]) with the
//!    mandated model identity ([`EMBEDDING_MODEL_ID`]).
//! 3. Write one new local core scan shard in `shard_dir` —
//!    `atlas-core-<tier>-<topic>-<nn>.db` carrying exactly the shard
//!    schema shard discovery and the fan-out scan read (`chunks`, the
//!    [`DEFAULT_VEC_TABLE`] vector table, `atlas_manifest`), so the
//!    promoted entries are searchable by the existing fan-out with no
//!    new read path. The shard's tier is the unit's recorded
//!    sensitivity; each chunk's `sensitivity` is written in the scan
//!    vocabulary (see [`tier_sensitivity_name`]).
//! 4. Stamp the unit DB's `atlas_manifest` `embedding_model` with the
//!    mandated model — the provenance record [`Registry::promote`]'s
//!    M6 gate (#7648) reads. Stamping adds the manifest table to the
//!    staging DB, which closes it to further `stage_artifact` writes
//!    (the v1 schema check asserts the exact table set): promotion is
//!    the unit's terminal write, matching merge-on-close.
//! 5. Promote the unit through [`Registry::promote`], whose gate
//!    re-reads the stamp; the unit ends `merged`.
//!
//! Failure shape: any error before the stamp leaves the unit `open`
//! and removes a partially written shard file, so a retry starts
//! clean. An error at the promote step itself leaves a complete,
//! discoverable shard and a stamped unit DB — re-running refuses at
//! the shard-name step only if the file still exists, so the operator
//! removes the recorded shard and retries, or promotes the unit
//! directly through the registry (the gate will pass: the stamp is
//! already in place).
//!
//! Out of scope (later #5669 slices): hosted Turso shard creation (the
//! deployment surface), multi-unit aggregation into one shard, and
//! merge-on-close archival of the staging DB file.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::embed::DocEmbedder;
use crate::error::{Error, Result};
use crate::shards::{EMBEDDING_MODEL_ID, Tier};
use crate::stage::{Registry, UnitState};
use crate::types::DEFAULT_VEC_TABLE;

/// The scan-vocabulary sensitivity name a chunk of this tier carries
/// in a core shard. The fan-out scan filters `chunks.sensitivity`
/// against the [`crate::filters::Sensitivity`] ladder, whose tier
/// mapping is [`crate::shards::TierCeiling::from_sensitivity`]
/// (shareable -> public; personal / local-only -> internal;
/// secret-bearing -> secret). Promotion writes the canonical
/// representative of the unit's tier, so an internal unit's chunks
/// are admitted under the default `personal` ceiling and a secret
/// unit's chunks only under an explicit `secret-bearing` ceiling.
pub fn tier_sensitivity_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Public => "shareable",
        Tier::Internal => "personal",
        Tier::Secret => "secret-bearing",
    }
}

/// What one embed-at-promotion run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteReport {
    pub unit_id: String,
    pub shard_name: String,
    pub shard_path: PathBuf,
    pub items: usize,
    pub chunks: usize,
    pub dims: usize,
}

struct StagedChunk {
    item_id: i64,
    item_source: Option<String>,
    item_sensitivity: String,
    idx: i64,
    text: String,
    content_hash: String,
}

/// Read one unit's staged chunks in staging order (item id, then
/// chunk idx), with each chunk's item source pointer and sensitivity.
fn read_staged_chunks(db_path: &Path, unit_id: &str) -> Result<Vec<StagedChunk>> {
    let conn = Connection::open(db_path).map_err(|e| {
        Error::backend(format!(
            "aggregator: opening staging DB {}: {e}",
            db_path.display()
        ))
    })?;
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.source, i.sensitivity, c.idx, c.text, c.content_hash \
             FROM chunks_raw c JOIN items i ON i.id = c.item_id \
             WHERE i.unit_id = ?1 ORDER BY i.id, c.idx",
        )
        .map_err(|e| Error::backend(format!("aggregator: reading staged chunks: {e}")))?;
    let rows = stmt
        .query_map(params![unit_id], |r| {
            Ok(StagedChunk {
                item_id: r.get(0)?,
                item_source: r.get(1)?,
                item_sensitivity: r.get(2)?,
                idx: r.get(3)?,
                text: r.get(4)?,
                content_hash: r.get(5)?,
            })
        })
        .map_err(|e| Error::backend(format!("aggregator: reading staged chunks: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(
            row.map_err(|e| Error::backend(format!("aggregator: reading staged chunks: {e}")))?,
        );
    }
    Ok(out)
}

/// The next free part number for `(tier, topic)` in `shard_dir`:
/// one past the highest existing shard part, or 0 when none exists.
fn next_shard_part(shard_dir: &Path, tier: Tier, topic: &str) -> Result<u32> {
    let mut next: u32 = 0;
    let rd = match std::fs::read_dir(shard_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(Error::backend(format!(
                "aggregator: listing shard dir {}: {e}",
                shard_dir.display()
            )));
        }
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.extension().and_then(|x| x.to_str()) != Some("db") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some((t, tp, part)) = crate::shards::parse_shard_name(stem) {
            if t == tier && tp == topic {
                next = next.max(part + 1);
            }
        }
    }
    Ok(next)
}

fn vec_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Embed one staging unit's chunks into a new core scan shard and
/// promote the unit (see the module docs for the step order and the
/// failure shape). `topic` is the shard topic the promoted entries
/// land under (e.g. `codeconfig` for #1518 code/config entries).
pub fn embed_unit_into_shard(
    registry: &Registry,
    unit_id: &str,
    shard_dir: &Path,
    topic: &str,
    embedder: &dyn DocEmbedder,
    now: &str,
) -> Result<PromoteReport> {
    let unit = registry.lookup(unit_id)?.ok_or_else(|| {
        Error::fatal(format!(
            "aggregator: no staging unit {unit_id:?} to promote"
        ))
    })?;
    if unit.state == UnitState::Merged {
        return Err(Error::fatal(format!(
            "aggregator: staging unit {unit_id:?} is already merged"
        )));
    }
    if topic.is_empty()
        || !topic
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(Error::fatal(format!(
            "aggregator: shard topic {topic:?} must be non-empty [A-Za-z0-9-]"
        )));
    }
    let db_path = PathBuf::from(&unit.db_uri);
    let chunks = read_staged_chunks(&db_path, unit_id)?;
    if chunks.is_empty() {
        return Err(Error::fatal(format!(
            "aggregator: staging unit {unit_id:?} has no staged chunks to embed"
        )));
    }
    let items = {
        let mut ids: Vec<i64> = Vec::new();
        for c in &chunks {
            if !ids.contains(&c.item_id) {
                ids.push(c.item_id);
            }
        }
        ids.len()
    };

    // Embed everything before writing anything: a model failure
    // leaves no partial shard and an unstamped, still-open unit.
    let mut vecs: Vec<Vec<f32>> = Vec::with_capacity(chunks.len());
    for c in &chunks {
        vecs.push(embedder.embed_doc(&c.text)?);
    }
    let dims = vecs[0].len();
    if dims == 0 || vecs.iter().any(|v| v.len() != dims) {
        return Err(Error::fatal(format!(
            "aggregator: embedder returned inconsistent vector dims for unit {unit_id:?} (first {dims})"
        )));
    }

    std::fs::create_dir_all(shard_dir).map_err(|e| {
        Error::backend(format!(
            "aggregator: creating shard dir {}: {e}",
            shard_dir.display()
        ))
    })?;
    let part = next_shard_part(shard_dir, unit.sensitivity, topic)?;
    let shard_name = format!(
        "atlas-core-{}-{}-{:02}",
        unit.sensitivity.name(),
        topic,
        part
    );
    let shard_path = shard_dir.join(format!("{shard_name}.db"));
    if shard_path.exists() {
        return Err(Error::fatal(format!(
            "aggregator: shard file {} already exists; refusing to overwrite",
            shard_path.display()
        )));
    }

    let write = (|| -> Result<()> {
        let conn = Connection::open(&shard_path).map_err(|e| {
            Error::backend(format!(
                "aggregator: creating shard {}: {e}",
                shard_path.display()
            ))
        })?;
        conn.execute_batch(&format!(
            "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
               chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
               sensitivity TEXT); \
             CREATE TABLE {DEFAULT_VEC_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL); \
             CREATE TABLE atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);"
        ))
        .map_err(|e| Error::backend(format!("aggregator: shard schema: {e}")))?;
        conn.execute_batch("BEGIN")
            .map_err(|e| Error::backend(format!("aggregator: shard begin: {e}")))?;
        for (i, (c, v)) in chunks.iter().zip(vecs.iter()).enumerate() {
            let id = (i + 1) as i64;
            // The chunk's sensitivity is its item's recorded tier,
            // in scan vocabulary; an unparseable record falls back
            // to the unit's tier rather than widening access.
            let sens = Tier::parse(&c.item_sensitivity)
                .map(tier_sensitivity_name)
                .unwrap_or_else(|| tier_sensitivity_name(unit.sensitivity));
            let message_id = c
                .item_source
                .clone()
                .unwrap_or_else(|| format!("unit:{unit_id}"));
            conn.execute(
                "INSERT INTO chunks VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, message_id, c.idx, c.text, c.content_hash, sens],
            )
            .map_err(|e| Error::backend(format!("aggregator: shard chunk {id}: {e}")))?;
            conn.execute(
                &format!("INSERT INTO {DEFAULT_VEC_TABLE} VALUES (?1, ?2)"),
                params![id, vec_blob(v)],
            )
            .map_err(|e| Error::backend(format!("aggregator: shard vector {id}: {e}")))?;
        }
        let manifest: [(&str, String); 9] = [
            ("shard", shard_name.clone()),
            ("tier", unit.sensitivity.name().to_string()),
            ("topic", topic.to_string()),
            ("part", part.to_string()),
            ("expected_chunks", chunks.len().to_string()),
            ("metric", "cosine".to_string()),
            ("embedding_model", EMBEDDING_MODEL_ID.to_string()),
            ("engine", "tursodb".to_string()),
            ("source_unit", unit_id.to_string()),
        ];
        for (k, v) in &manifest {
            conn.execute(
                "INSERT OR REPLACE INTO atlas_manifest VALUES (?1, ?2)",
                params![k, v],
            )
            .map_err(|e| Error::backend(format!("aggregator: shard manifest {k}: {e}")))?;
        }
        conn.execute_batch("COMMIT")
            .map_err(|e| Error::backend(format!("aggregator: shard commit: {e}")))?;
        Ok(())
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&shard_path);
        return Err(e);
    }

    // Stamp the unit DB's embedding-model provenance (the M6 gate's
    // record), then promote through the registry gate. On failure the
    // shard stays on disk, complete and discoverable, and the report
    // path is in the module docs.
    let stamp = (|| -> Result<()> {
        let conn = Connection::open(&db_path).map_err(|e| {
            Error::backend(format!(
                "aggregator: opening staging DB {} for the model stamp: {e}",
                db_path.display()
            ))
        })?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);",
        )
        .map_err(|e| Error::backend(format!("aggregator: model stamp table: {e}")))?;
        conn.execute(
            "INSERT OR REPLACE INTO atlas_manifest VALUES ('embedding_model', ?1)",
            params![EMBEDDING_MODEL_ID],
        )
        .map_err(|e| Error::backend(format!("aggregator: model stamp: {e}")))?;
        Ok(())
    })();
    if let Err(e) = stamp {
        let _ = std::fs::remove_file(&shard_path);
        return Err(e);
    }

    registry.promote(unit_id, now)?;

    Ok(PromoteReport {
        unit_id: unit_id.to_string(),
        shard_name,
        shard_path,
        items,
        chunks: chunks.len(),
        dims,
    })
}
