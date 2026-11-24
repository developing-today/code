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
//!   design's five corpora as staging units with provenance.
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
//! Out of scope (slice 1b, the #8622 remainder): the native LanceDB
//! reader, idempotency keyed on id+updated_at, and the
//! rebuild-from-sources proof. Until 1b, this connector stages a
//! unit once and the unit is then promoted — promotion is the
//! terminal write, as for every staged unit.

use std::path::Path;

use rusqlite::{Connection, params};

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
pub fn register_memory_corpora(
    registry: &Registry,
    staging_dir: &Path,
    now: &str,
) -> Result<Vec<StagingUnit>> {
    let mut out = Vec::with_capacity(MEMORY_CORPORA.len());
    for corpus in MEMORY_CORPORA {
        if let Some(unit) = registry.lookup(corpus.unit_id)? {
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
    let item_id = conn.last_insert_rowid();
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
    Ok(item_id)
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
        .map(|entry| stage_memory_entry(conn, unit_id, entry, now))
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
