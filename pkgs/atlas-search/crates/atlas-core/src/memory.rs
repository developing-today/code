//! The Atlas memory-integration connectors (developing-today/code#8622,
//! slice 1a of the #8567 design record,
//! `pkgs/atlas-search/docs/memory-integration-design-8567.md`
//! sections 1–2).
//!
//! The design unifies the fleet's own corpora in the existing
//! stage/registry path: one staging unit per corpus, staged units
//! carrying the design's provenance set (corpus, agent, source_ref,
//! updated_at, sensitivity, zone), promotion through the registry's
//! embedding-model gate, and search over the promoted shards. This
//! module is the first connector slice:
//!
//! * [`MEMORY_CORPORA`] + [`register_memory_corpora`] register the
//!   design's five corpora as staging units with provenance. Each
//!   unit's registry record carries its embedding-model identity
//!   (#8623 slice 2): re-registration verifies the recorded identity
//!   is the mandated model and refuses a corpus recorded under a
//!   retired model, so ingestion never mixes incomparable vectors.
//! * [`MemoryEntry`] + [`parse_memory_export`] read a read-only JSON
//!   export of OpenClaw's memory-lancedb store. The export carries
//!   the store's columns; its `vector` column is deliberately
//!   ignored — Atlas re-embeds at promotion with the mandated model
//!   (design section 2), so vectors stay comparable across corpora
//!   and the store's own embedding model never leaks into the index.
//! * [`stage_memory_entry`] / [`stage_memory_export`] stage entries
//!   into the agent-memory unit through the same `items` +
//!   `chunks_raw` write path every staged unit uses; promotion and
//!   search are the existing aggregator and fan-out, unchanged.
//!
//! Slice 1b (developing-today/code#14039) adds replay-safe staging
//! keyed by the stable LanceDB row id and source update timestamp
//! (with content/metadata comparison when the source has no
//! timestamp), plus the native reader [`read_lancedb_store`] and
//! idempotent [`sync_memory_entries`] path. The rebuild proof
//! ([`rebuild_agent_memory_index`]) shows the Atlas index is
//! disposable: the same source rebuilt into a fresh staging area
//! yields the same shard fingerprints and fan-out ranking, and a
//! rebuild from a revised source carries only the current source
//! rows.
//!
//! Slice 4's fleet-memory half stages the Muse memory tree
//! (`MEMORY.md`, topic files, daily logs) into the `fleet-memory`
//! unit through [`scan_memory_files`] + [`stage_memory_files`]:
//! one `items` row plus its `chunks_raw` per markdown file, with
//! the file's mtime as `updated_at` and replay-safe re-staging on
//! text/mtime comparison.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::aggregate::embed_unit_into_shard;
use crate::embed::DocEmbedder;
use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::{
    Registry, StagingUnit, check_stage_schema, chunk_artifact_text, open, sha256_hex,
};
use crate::trace::rfc3339_utc;

/// The staging-unit id of the agent-memory corpus — the corpus this
/// connector populates.
pub const AGENT_MEMORY_UNIT: &str = "agent-memory";

/// One corpus of the design's section-1 corpus set: the staging
/// unit that represents it in the registry, the owner recorded as
/// its provenance, and the unit's physical sensitivity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryCorpus {
    /// The staging-unit id (also the corpus name every staged unit
    /// of this corpus carries in its provenance).
    pub unit_id: &'static str,
    /// The corpus's title in the design's corpus table.
    pub title: &'static str,
    /// The corpus's existing owner (the design keeps each corpus's
    /// source of truth and owner; Atlas holds the index).
    pub owner: &'static str,
    /// The unit's recorded sensitivity tier.
    pub sensitivity: Tier,
}

/// The design's five corpora (section 1), in the design's order.
pub const MEMORY_CORPORA: [MemoryCorpus; 5] = [
    MemoryCorpus {
        unit_id: AGENT_MEMORY_UNIT,
        title: "Agent memory (OpenClaw memory-lancedb store)",
        owner: "openclaw",
        sensitivity: Tier::Internal,
    },
    MemoryCorpus {
        unit_id: "fleet-memory",
        title: "Fleet memory files (the Muse memory tree)",
        owner: "muse",
        sensitivity: Tier::Internal,
    },
    MemoryCorpus {
        unit_id: "zettelkasten",
        title: "Zettelkasten / Emanote notes (the notes vault)",
        owner: "muse",
        sensitivity: Tier::Internal,
    },
    MemoryCorpus {
        unit_id: "repo-docs",
        title: "Repo docs (the four fleet repos' doc trees)",
        owner: "fleet",
        sensitivity: Tier::Internal,
    },
    MemoryCorpus {
        unit_id: "research-reports",
        title: "Research reports (prior fleet research outputs)",
        owner: "fleet",
        sensitivity: Tier::Internal,
    },
];

/// Register the five design corpora in the stage registry: one
/// staging unit per corpus, provisioned under `staging_dir` with the
/// corpus's owner and sensitivity as its registry provenance.
/// Registration is idempotent — a corpus already in the registry is
/// returned as recorded, never re-provisioned — so a connector run
/// can call this first, unconditionally.
///
/// Slice 2 (#8623): a corpus already in the registry is checked
/// against the mandated embedding-model identity before it is
/// returned. A corpus recorded under a retired model fails loudly
/// here — ingestion into it is refused — instead of mixing vectors
/// from different models into one corpus. Rebuild the corpus with
/// the re-embed job first; this function never rewrites identity.
pub fn register_memory_corpora(
    registry: &Registry,
    staging_dir: &Path,
    now: &str,
) -> Result<Vec<StagingUnit>> {
    use crate::shards::EMBEDDING_MODEL_ID;

    let mut out = Vec::with_capacity(MEMORY_CORPORA.len());
    for corpus in MEMORY_CORPORA {
        if let Some(unit) = registry.lookup(corpus.unit_id)? {
            if unit.embedding_model != EMBEDDING_MODEL_ID {
                return Err(Error::fatal(format!(
                    "memory connector: refusing corpus {}: its recorded embedding_model `{}` is not the mandated `{EMBEDDING_MODEL_ID}`; vectors from different embedding models are not comparable — rebuild the corpus with the re-embed job first",
                    corpus.unit_id, unit.embedding_model
                )));
            }
            out.push(unit);
            continue;
        }
        let db = staging_dir.join(format!("{}.db", corpus.unit_id));
        out.push(registry.provision(&db, corpus.unit_id, corpus.owner, corpus.sensitivity, now)?);
    }
    Ok(out)
}

/// One row of the memory-lancedb store, as staged: the store's own
/// columns (minus its vector — see the module docs), with the row's
/// sensitivity recorded the way every staged unit records it.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEntry {
    /// The store's row id.
    pub id: String,
    /// The memory text.
    pub text: String,
    /// The store's `agentId` (the agent family that wrote the row),
    /// where recorded.
    pub agent: Option<String>,
    /// The store's `category`, where recorded.
    pub category: Option<String>,
    /// The store's `importance`, where recorded.
    pub importance: Option<f64>,
    /// The store's `createdAt` (milliseconds since the unix epoch),
    /// where recorded.
    pub created_at_ms: Option<f64>,
    /// The row's physical sensitivity tier (the export records it
    /// per row; the connector's default is the corpus's tier).
    pub sensitivity: Tier,
}

impl MemoryEntry {
    /// The entry's source-of-truth pointer: the store and row id a
    /// reader opens to see the source, carried as the staged item's
    /// `source` and in its provenance as `source_ref`.
    pub fn source_ref(&self) -> String {
        format!("memory-lancedb:{}", self.id)
    }

    /// The entry's `updated_at` provenance value: the store's
    /// `createdAt` instant as RFC 3339 UTC, where recorded.
    pub fn updated_at(&self) -> Option<String> {
        self.created_at_ms
            .filter(|ms| *ms >= 0.0)
            .map(|ms| rfc3339_utc(ms as u64))
    }
}

fn export_req_str(row: &serde_json::Value, idx: usize, name: &str) -> Result<String> {
    match row.get(name).and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(Error::fatal(format!(
            "memory connector: export row {idx} is missing a non-empty string field `{name}`"
        ))),
    }
}

fn export_opt_str(row: &serde_json::Value, name: &str) -> Option<String> {
    row.get(name).and_then(|v| v.as_str()).map(str::to_string)
}

fn export_opt_num(row: &serde_json::Value, name: &str) -> Option<f64> {
    row.get(name).and_then(|v| v.as_f64())
}

/// Parse a read-only JSON export of the memory-lancedb store: an
/// array of row objects carrying the store's columns (`id`, `text`,
/// `agentId`, `category`, `importance`, `createdAt`, and a `vector`
/// this connector ignores by design). A row may additionally record
/// its `sensitivity` tier by name; rows that do not are staged at
/// the corpus's internal tier. Malformed input is a loud error
/// naming the row and field — a partial or silently defaulted ingest
/// is exactly what the registry's provenance discipline exists to
/// prevent.
pub fn parse_memory_export(json: &str) -> Result<Vec<MemoryEntry>> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Error::fatal(format!("memory connector: export is not valid JSON: {e}")))?;
    let rows = value.as_array().ok_or_else(|| {
        Error::fatal("memory connector: export must be a JSON array of memory rows".to_string())
    })?;
    let mut out = Vec::with_capacity(rows.len());
    for (idx, row) in rows.iter().enumerate() {
        let sensitivity = match export_opt_str(row, "sensitivity") {
            None => Tier::Internal,
            Some(name) => Tier::parse(&name).ok_or_else(|| {
                Error::fatal(format!(
                    "memory connector: export row {idx} records unknown sensitivity `{name}`"
                ))
            })?,
        };
        out.push(MemoryEntry {
            id: export_req_str(row, idx, "id")?,
            text: export_req_str(row, idx, "text")?,
            agent: export_opt_str(row, "agentId"),
            category: export_opt_str(row, "category"),
            importance: export_opt_num(row, "importance"),
            created_at_ms: export_opt_num(row, "createdAt"),
            sensitivity,
        });
    }
    Ok(out)
}

/// Stage one memory entry into a v1 staging DB as one `items` row
/// plus its `chunks_raw` rows, on the same write path as every
/// staged unit: `kind` is `memory`, `source` is the entry's
/// source-of-truth pointer, and `meta` carries the design's
/// provenance set (corpus, agent, source_ref, updated_at,
/// sensitivity, zone — `fleet` for every corpus in the design's
/// pre-ratification scope, section 5). Returns the new item's id.
/// The DB must already be a v1 staging DB
/// ([`check_stage_schema`]); vectors are produced later, at
/// promotion, as for every staged item.
pub fn stage_memory_entry(
    conn: &Connection,
    unit_id: &str,
    entry: &MemoryEntry,
    now: &str,
) -> Result<i64> {
    stage_memory_entry_with_status(conn, unit_id, entry, now).map(|(id, _)| id)
}

fn stage_memory_entry_with_status(
    conn: &Connection,
    unit_id: &str,
    entry: &MemoryEntry,
    now: &str,
) -> Result<(i64, bool)> {
    let tx = conn.unchecked_transaction().map_err(|e| {
        Error::backend(format!("memory connector: starting stage transaction: {e}"))
    })?;
    let result = stage_memory_entry_in_transaction(&tx, unit_id, entry, now)?;
    tx.commit().map_err(|e| {
        Error::backend(format!(
            "memory connector: committing stage transaction: {e}"
        ))
    })?;
    Ok(result)
}

fn stage_memory_entry_in_transaction(
    conn: &Connection,
    unit_id: &str,
    entry: &MemoryEntry,
    now: &str,
) -> Result<(i64, bool)> {
    check_stage_schema(conn)?;
    let chunks = chunk_artifact_text(&entry.text);
    let source = entry.source_ref();
    let updated_at = entry.updated_at();
    let meta = serde_json::json!({
        "corpus": AGENT_MEMORY_UNIT,
        "agent": entry.agent,
        "source_ref": source,
        "updated_at": updated_at,
        "sensitivity": entry.sensitivity.name(),
        "zone": "fleet",
        "category": entry.category,
        "importance": entry.importance,
        "lancedb_id": entry.id,
        "staged_at": now,
        "chunks": chunks.len(),
    })
    .to_string();
    let existing = conn
        .query_row(
            "SELECT id, text, sensitivity, meta FROM items \
             WHERE unit_id = ?1 AND kind = 'memory' AND source = ?2 ORDER BY id DESC LIMIT 1",
            params![unit_id, source],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: finding item for {}: {e}",
                entry.id
            ))
        })?;

    let (item_id, changed) = match existing {
        Some((id, text, sensitivity, old_meta)) => {
            let unchanged = text == entry.text
                && sensitivity == entry.sensitivity.name()
                && old_meta
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .is_some_and(|old| {
                        let new: serde_json::Value =
                            serde_json::from_str(&meta).unwrap_or_default();
                        old.get("updated_at") == new.get("updated_at")
                            && old.get("agent") == new.get("agent")
                            && old.get("category") == new.get("category")
                            && old.get("importance") == new.get("importance")
                            && old.get("sensitivity") == new.get("sensitivity")
                    });
            if unchanged {
                return Ok((id, false));
            }
            conn.execute(
                "UPDATE items SET text = ?1, sensitivity = ?2, created_at = ?3, meta = ?4 WHERE id = ?5",
                params![
                    entry.text,
                    entry.sensitivity.name(),
                    updated_at.as_deref().unwrap_or(now),
                    meta,
                    id,
                ],
            )
            .map_err(|e| Error::backend(format!("memory connector: updating item for {}: {e}", entry.id)))?;
            conn.execute("DELETE FROM chunks_raw WHERE item_id = ?1", [id])
                .map_err(|e| {
                    Error::backend(format!(
                        "memory connector: replacing chunks for {}: {e}",
                        entry.id
                    ))
                })?;
            (id, true)
        }
        None => {
            conn.execute(
                "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    unit_id,
                    "memory",
                    source,
                    entry.text,
                    entry.sensitivity.name(),
                    updated_at.as_deref().unwrap_or(now),
                    meta,
                ],
            )
            .map_err(|e| {
                Error::backend(format!(
                    "memory connector: inserting item for {}: {e}",
                    entry.id
                ))
            })?;
            (conn.last_insert_rowid(), true)
        }
    };
    for (idx, chunk) in chunks.iter().enumerate() {
        conn.execute(
            "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item_id, idx as i64, chunk, sha256_hex(chunk), now],
        )
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: inserting chunk {idx} for {}: {e}",
                entry.id
            ))
        })?;
    }
    Ok((item_id, changed))
}

/// Stage a whole parsed export into a unit, in export order.
/// Returns the new items' ids.
pub fn stage_memory_export(
    conn: &Connection,
    unit_id: &str,
    entries: &[MemoryEntry],
    now: &str,
) -> Result<Vec<i64>> {
    entries
        .iter()
        .map(|entry| stage_memory_entry_with_status(conn, unit_id, entry, now))
        .filter_map(|result| match result {
            Ok((id, true)) => Some(Ok(id)),
            Ok((_, false)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

/// [`stage_memory_export`] against a staging DB file: the form the
/// `atlas-stage ingest-memory` tool calls, so the CLI needs no DB
/// handle of its own.
pub fn stage_memory_export_in_db(
    db: &Path,
    unit_id: &str,
    entries: &[MemoryEntry],
    now: &str,
) -> Result<Vec<i64>> {
    let conn = open(db)?;
    stage_memory_export(&conn, unit_id, entries, now)
}

/// What one [`sync_memory_entries`] run did to a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// Entries staged for the first time.
    pub added: usize,
    /// Entries whose recorded content changed, restaged in place.
    pub restaged: usize,
    /// Entries already staged with identical content — untouched.
    pub unchanged: usize,
}

/// Read the memory-lancedb store at `path` natively: open the
/// `memories.lance` dataset with the `lance` crate and map its
/// `id`, `text`, `agentId`, `category`, `importance` and `createdAt`
/// columns onto [`MemoryEntry`]. The `vector` column is never read.
/// A missing required column or an empty id/text is a loud error.
#[cfg(feature = "lancedb")]
pub async fn read_lancedb_store(path: &Path) -> Result<Vec<MemoryEntry>> {
    use arrow::array::{Array, ArrayRef, RecordBatch};
    use arrow::compute::cast;
    use arrow::datatypes::DataType;

    let uri = path
        .to_str()
        .ok_or_else(|| Error::fatal("memory connector: store path is not valid UTF-8"))?;
    let dataset = lance::Dataset::open(uri)
        .await
        .map_err(|e| Error::backend(format!("memory connector: opening {uri}: {e}")))?;
    let batch: RecordBatch = dataset
        .scan()
        .try_into_batch()
        .await
        .map_err(|e| Error::backend(format!("memory connector: scanning {uri}: {e}")))?;

    let column = |name: &str| -> Result<Option<ArrayRef>> {
        match batch.column_by_name(name) {
            None => Ok(None),
            Some(col) => cast(col, &DataType::Utf8)
                .map(Some)
                .map_err(|e| Error::fatal(format!("memory connector: column `{name}`: {e}"))),
        }
    };
    let numbers = |name: &str| -> Result<Option<ArrayRef>> {
        match batch.column_by_name(name) {
            None => Ok(None),
            Some(col) => cast(col, &DataType::Float64)
                .map(Some)
                .map_err(|e| Error::fatal(format!("memory connector: column `{name}`: {e}"))),
        }
    };
    let id =
        column("id")?.ok_or_else(|| Error::fatal("memory connector: store has no `id` column"))?;
    let text = column("text")?
        .ok_or_else(|| Error::fatal("memory connector: store has no `text` column"))?;
    let agent = column("agentId")?;
    let category = column("category")?;
    let importance = numbers("importance")?;
    let created = numbers("createdAt")?;

    fn str_at(col: &Option<ArrayRef>, i: usize) -> Option<String> {
        let arr = col
            .as_ref()?
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()?;
        (!arr.is_null(i)).then(|| arr.value(i).to_string())
    }
    fn num_at(col: &Option<ArrayRef>, i: usize) -> Option<f64> {
        let arr = col
            .as_ref()?
            .as_any()
            .downcast_ref::<arrow::array::Float64Array>()?;
        (!arr.is_null(i)).then(|| arr.value(i))
    }

    let mut out = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        let row_id = str_at(&Some(id.clone()), i)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::fatal(format!("memory connector: row {i} has no `id`")))?;
        let row_text = str_at(&Some(text.clone()), i)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::fatal(format!("memory connector: row {row_id} has no `text`")))?;
        out.push(MemoryEntry {
            id: row_id,
            text: row_text,
            agent: str_at(&agent, i),
            category: str_at(&category, i),
            importance: num_at(&importance, i),
            created_at_ms: num_at(&created, i),
            sensitivity: Tier::Internal,
        });
    }
    Ok(out)
}

/// Blocking form of [`read_lancedb_store`] for the CLI and tests.
#[cfg(feature = "lancedb")]
pub fn read_lancedb_store_blocking(path: &Path) -> Result<Vec<MemoryEntry>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::backend(format!("memory connector: tokio runtime: {e}")))?
        .block_on(read_lancedb_store(path))
}

/// Read a native memory-lancedb dataset and idempotently sync its rows
/// into an already-provisioned Atlas staging database.
#[cfg(feature = "lancedb")]
pub fn sync_lancedb_store_in_db(
    store: &Path,
    db: &Path,
    unit_id: &str,
    now: &str,
) -> Result<SyncReport> {
    if unit_id != AGENT_MEMORY_UNIT {
        return Err(Error::fatal(format!(
            "memory connector: native memory-lancedb rows must be staged in `{AGENT_MEMORY_UNIT}`"
        )));
    }
    let entries = read_lancedb_store_blocking(store)?;
    let conn = open(db)?;
    sync_memory_entries(&conn, unit_id, &entries, now)
}

/// Sync entries into a unit, idempotently: an entry whose source_ref
/// is already staged with the same text is skipped; one whose text
/// changed is restaged (its old item and chunks removed first); a new
/// one is staged. Re-running with the same export changes nothing.
pub fn sync_memory_entries(
    conn: &Connection,
    unit_id: &str,
    entries: &[MemoryEntry],
    now: &str,
) -> Result<SyncReport> {
    check_stage_schema(conn)?;
    let mut report = SyncReport::default();
    for entry in entries {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM items WHERE unit_id = ?1 AND kind = 'memory' AND source = ?2)",
                params![unit_id, entry.source_ref()],
                |row| row.get(0),
            )
            .map_err(|e| Error::backend(format!("memory connector: {e}")))?;
        let (_, changed) = stage_memory_entry_with_status(conn, unit_id, entry, now)?;
        match (exists, changed) {
            (false, true) => report.added += 1,
            (true, true) => report.restaged += 1,
            (_, false) => report.unchanged += 1,
        }
    }
    Ok(report)
}

/// What one rebuild-from-sources run did: the promotion report plus
/// the staging delta and the rebuilt shard's chunk fingerprints, so
/// two builds from the same source can be compared for equality and
/// a build from a revised source can be compared for convergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildReport {
    /// The staging-unit id that was rebuilt.
    pub unit_id: String,
    /// The new core scan shard's file name.
    pub shard_name: String,
    /// The new core scan shard's path.
    pub shard_path: PathBuf,
    /// Staged items embedded into the shard.
    pub items: usize,
    /// Staged chunks embedded into the shard.
    pub chunks: usize,
    /// The embedding dimension the shard's vectors carry.
    pub dims: usize,
    /// Item ids the staging step changed in this run (new or
    /// revised rows). In a fresh staging area this is every row;
    /// the rebuild's convergence claim is about the shard, not
    /// this list.
    pub staged_changed: Vec<i64>,
    /// The rebuilt shard's chunk fingerprints, in shard order:
    /// (source-of-truth pointer, content hash) per shard chunk.
    /// Two builds from the same source must produce the same list.
    pub fingerprints: Vec<(String, String)>,
}

/// Read a core scan shard's chunk fingerprints, in shard order:
/// (source-of-truth pointer, content hash) per `chunks` row. The
/// fingerprint is the rebuild comparison's oracle: the hash is a
/// pure function of the staged chunk text (and, for repeated text,
/// its position), so equal fingerprints mean equal index content.
pub fn shard_chunk_fingerprints(shard_db: &Path) -> Result<Vec<(String, String)>> {
    let conn = Connection::open(shard_db).map_err(|e| {
        Error::backend(format!(
            "memory connector: opening shard {}: {e}",
            shard_db.display()
        ))
    })?;
    let mut stmt = conn
        .prepare("SELECT message_id, content_hash FROM chunks ORDER BY id")
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: reading fingerprints from {}: {e}",
                shard_db.display()
            ))
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: reading fingerprints from {}: {e}",
                shard_db.display()
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: reading fingerprints from {}: {e}",
                shard_db.display()
            ))
        })?;
    Ok(rows)
}

/// Rebuild the agent-memory Atlas index from its sources
/// (developing-today/code#14039, remaining acceptance: the index is
/// disposable, the corpora never are — design section 2).
///
/// One call, in order: register the five design corpora (idempotent),
/// stage the source entries into the agent-memory unit through the
/// replay-safe staging path, and promote the unit into a new core
/// scan shard in `shard_dir` (embed-at-promotion, so the shard's
/// vectors come from the mandated model, never from the source
/// store's own vectors).
///
/// The staging area is disposable by design: promotion stamps the
/// staging DB and closes it to further writes, and a merged unit
/// refuses a second promotion — so a rebuild targets a fresh
/// registry + staging dir, and the rebuilt shard is compared with
/// the previous build's [`RebuildReport::fingerprints`], not by
/// re-promoting into the same area. A rebuild from a revised source
/// carries only the current source's rows: rows the source dropped
/// are absent from the fresh staging area, so they are absent from
/// the rebuilt shard. (Incremental staging is different by design:
/// a partial export never deletes — see the adapter-fixtures record.
/// Deletion converges at rebuild, never at incremental stage.)
pub fn rebuild_agent_memory_index(
    registry: &Registry,
    staging_dir: &Path,
    entries: &[MemoryEntry],
    shard_dir: &Path,
    embedder: &dyn DocEmbedder,
    now: &str,
) -> Result<RebuildReport> {
    let units = register_memory_corpora(registry, staging_dir, now)?;
    let agent_db = units
        .iter()
        .find(|u| u.unit_id == AGENT_MEMORY_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .ok_or_else(|| {
            Error::fatal(format!(
                "memory connector: {AGENT_MEMORY_UNIT} unit missing after registration"
            ))
        })?;
    let staged_changed = stage_memory_export_in_db(&agent_db, AGENT_MEMORY_UNIT, entries, now)?;
    let promote =
        embed_unit_into_shard(registry, AGENT_MEMORY_UNIT, shard_dir, "agent-memory", embedder, now)?;
    let fingerprints = shard_chunk_fingerprints(&promote.shard_path)?;
    Ok(RebuildReport {
        unit_id: promote.unit_id,
        shard_name: promote.shard_name,
        shard_path: promote.shard_path,
        items: promote.items,
        chunks: promote.chunks,
        dims: promote.dims,
        staged_changed,
        fingerprints,
    })
}

/// The staging-unit id of the fleet-memory-files corpus — the
/// corpus this connector's second slice populates (design section
/// 1: the Muse memory tree, `MEMORY.md`, topic files, daily
/// logs).
pub const FLEET_MEMORY_UNIT: &str = "fleet-memory";

/// One memory-tree file, as scanned: its path relative to the
/// tree root (the stable source identity, `/`-separated), its
/// text, its modification instant as RFC 3339 UTC where known,
/// and its physical sensitivity tier (the corpus default is the
/// corpus's internal tier).
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryFile {
    /// The file's path relative to the scanned root,
    /// `/`-separated (e.g. `MEMORY.md`, `topics/deploy.md`).
    pub rel_path: String,
    /// The file's full text.
    pub text: String,
    /// The file's modification instant as RFC 3339 UTC, where the
    /// filesystem reports one.
    pub updated_at: Option<String>,
    /// The file's physical sensitivity tier.
    pub sensitivity: Tier,
}

impl MemoryFile {
    /// The file's source-of-truth pointer: the corpus scheme plus
    /// its tree-relative path, carried as the staged item's
    /// `source` and in its provenance as `source_ref`.
    pub fn source_ref(&self) -> String {
        format!("memory-file:{}", self.rel_path)
    }
}

/// Format a filesystem modification instant as RFC 3339 UTC
/// (second precision). Times before the unix epoch report `None`
/// and let the caller fall back to the staging instant.
fn system_time_rfc3339(mtime: std::time::SystemTime) -> Option<String> {
    let secs = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    // Days-to-civil, shared with the `atlas-stage now_utc` helper:
    // RFC 3339 records when, not sub-second detail.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    Some(format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z"))
}

/// Scan a memory tree root for stageable files: every `*.md`
/// file under `root` (the tree's `MEMORY.md`, topic files, and
/// daily logs are all markdown), in sorted tree-relative order so
/// a scan is deterministic. Non-markdown files are not corpus
/// content and are skipped; unreadable files and invalid UTF-8
/// are loud errors naming the path — a partial or silently
/// defaulted ingest is exactly what the registry's provenance
/// discipline exists to prevent.
pub fn scan_memory_files(root: &Path) -> Result<Vec<MemoryFile>> {
    if !root.is_dir() {
        return Err(Error::fatal(format!(
            "memory connector: memory tree {} is not a directory",
            root.display()
        )));
    }
    let mut rel_paths: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| {
            Error::backend(format!("memory connector: listing {}: {e}", dir.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                Error::backend(format!("memory connector: listing {}: {e}", dir.display()))
            })?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file()
                && path
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("md"))
            {
                let rel = path.strip_prefix(root).map_err(|e| {
                    Error::backend(format!(
                        "memory connector: relativizing {}: {e}",
                        path.display()
                    ))
                })?;
                rel_paths.push(
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    rel_paths.sort();
    rel_paths
        .iter()
        .map(|rel| {
            // Rebuild the filesystem path from the `/`-joined
            // relative form (components were split on the platform
            // separator, so this round-trips on any host).
            let mut path = root.to_path_buf();
            for comp in rel.split('/') {
                path.push(comp);
            }
            let text = std::fs::read_to_string(&path).map_err(|e| {
                Error::backend(format!("memory connector: reading memory file {rel}: {e}"))
            })?;
            let updated_at = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(system_time_rfc3339);
            Ok(MemoryFile {
                rel_path: rel.clone(),
                text,
                updated_at,
                sensitivity: Tier::Internal,
            })
        })
        .collect()
}

/// Stage one memory-tree file into a v1 staging DB as one `items`
/// row plus its `chunks_raw` rows, on the same write path as every
/// staged unit: `kind` is `memory`, `source` is the file's
/// source-of-truth pointer, and `meta` carries the design's
/// provenance set (corpus `fleet-memory`, agent `null` — the tree
/// records no author — source_ref, updated_at from the file's
/// mtime, sensitivity, zone `fleet`). Returns the new item's id.
/// The DB must already be a v1 staging DB
/// ([`check_stage_schema`]); vectors are produced later, at
/// promotion, as for every staged item.
pub fn stage_memory_file(
    conn: &Connection,
    unit_id: &str,
    file: &MemoryFile,
    now: &str,
) -> Result<i64> {
    stage_memory_file_with_status(conn, unit_id, file, now).map(|(id, _)| id)
}

fn stage_memory_file_with_status(
    conn: &Connection,
    unit_id: &str,
    file: &MemoryFile,
    now: &str,
) -> Result<(i64, bool)> {
    let tx = conn.unchecked_transaction().map_err(|e| {
        Error::backend(format!("memory connector: starting stage transaction: {e}"))
    })?;
    let result = stage_memory_file_in_transaction(&tx, unit_id, file, now)?;
    tx.commit().map_err(|e| {
        Error::backend(format!(
            "memory connector: committing stage transaction: {e}"
        ))
    })?;
    Ok(result)
}

fn stage_memory_file_in_transaction(
    conn: &Connection,
    unit_id: &str,
    file: &MemoryFile,
    now: &str,
) -> Result<(i64, bool)> {
    check_stage_schema(conn)?;
    let chunks = chunk_artifact_text(&file.text);
    let source = file.source_ref();
    let meta = serde_json::json!({
        "corpus": FLEET_MEMORY_UNIT,
        "agent": Option::<String>::None,
        "source_ref": source,
        "updated_at": file.updated_at,
        "sensitivity": file.sensitivity.name(),
        "zone": "fleet",
        "rel_path": file.rel_path,
        "staged_at": now,
        "chunks": chunks.len(),
    })
    .to_string();
    let existing = conn
        .query_row(
            "SELECT id, text, sensitivity, meta FROM items \
             WHERE unit_id = ?1 AND kind = 'memory' AND source = ?2 ORDER BY id DESC LIMIT 1",
            params![unit_id, source],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: finding item for {}: {e}",
                file.rel_path
            ))
        })?;

    let (item_id, changed) = match existing {
        Some((id, text, sensitivity, old_meta)) => {
            let unchanged = text == file.text
                && sensitivity == file.sensitivity.name()
                && old_meta
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .is_some_and(|old| {
                        let new: serde_json::Value =
                            serde_json::from_str(&meta).unwrap_or_default();
                        old.get("updated_at") == new.get("updated_at")
                            && old.get("sensitivity") == new.get("sensitivity")
                    });
            if unchanged {
                return Ok((id, false));
            }
            conn.execute(
                "UPDATE items SET text = ?1, sensitivity = ?2, created_at = ?3, meta = ?4 WHERE id = ?5",
                params![
                    file.text,
                    file.sensitivity.name(),
                    file.updated_at.as_deref().unwrap_or(now),
                    meta,
                    id,
                ],
            )
            .map_err(|e| Error::backend(format!("memory connector: updating item for {}: {e}", file.rel_path)))?;
            conn.execute("DELETE FROM chunks_raw WHERE item_id = ?1", [id])
                .map_err(|e| {
                    Error::backend(format!(
                        "memory connector: replacing chunks for {}: {e}",
                        file.rel_path
                    ))
                })?;
            (id, true)
        }
        None => {
            conn.execute(
                "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    unit_id,
                    "memory",
                    source,
                    file.text,
                    file.sensitivity.name(),
                    file.updated_at.as_deref().unwrap_or(now),
                    meta,
                ],
            )
            .map_err(|e| {
                Error::backend(format!(
                    "memory connector: inserting item for {}: {e}",
                    file.rel_path
                ))
            })?;
            (conn.last_insert_rowid(), true)
        }
    };
    for (idx, chunk) in chunks.iter().enumerate() {
        conn.execute(
            "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item_id, idx as i64, chunk, sha256_hex(chunk), now],
        )
        .map_err(|e| {
            Error::backend(format!(
                "memory connector: inserting chunk {idx} for {}: {e}",
                file.rel_path
            ))
        })?;
    }
    Ok((item_id, changed))
}

/// Stage whole scanned memory-tree files into a unit, in scan
/// order. Returns the changed items' ids: an unchanged re-scan
/// stages zero.
pub fn stage_memory_files(
    conn: &Connection,
    unit_id: &str,
    files: &[MemoryFile],
    now: &str,
) -> Result<Vec<i64>> {
    files
        .iter()
        .map(|file| stage_memory_file_with_status(conn, unit_id, file, now))
        .filter_map(|result| match result {
            Ok((id, true)) => Some(Ok(id)),
            Ok((_, false)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

/// [`stage_memory_files`] against a staging DB file: the form the
/// `atlas-stage ingest-memory-files` tool calls, so the CLI needs
/// no DB handle of its own.
pub fn stage_memory_files_in_db(
    db: &Path,
    unit_id: &str,
    files: &[MemoryFile],
    now: &str,
) -> Result<Vec<i64>> {
    let conn = open(db)?;
    stage_memory_files(&conn, unit_id, files, now)
}

/// Scan a memory tree root and stage every file found into a
/// staging DB file: the connective form the
/// `atlas-stage ingest-memory-files` tool calls. Returns the
/// changed items' ids.
pub fn scan_and_stage_memory_files_in_db(
    db: &Path,
    unit_id: &str,
    root: &Path,
    now: &str,
) -> Result<Vec<i64>> {
    let files = scan_memory_files(root)?;
    stage_memory_files_in_db(db, unit_id, &files, now)
}
