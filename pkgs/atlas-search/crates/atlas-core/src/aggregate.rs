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
//!    mandated model identity ([`EMBEDDING_MODEL_ID`]). A staged
//!    chunk that tokenizes past the model context (a whole-file
//!    config unit can: infra/dns/dns_config.yaml is 10,413 tokens
//!    against the 8,192 context) is split at embed time into
//!    context-fitting pieces at line boundaries — the corpus
//!    pipeline's own contract is that the embedder only ever sees
//!    in-context inputs, and atlas-embed refuses longer ones. The
//!    pieces partition the chunk text exactly and become separate
//!    shard chunks under the same source pointer, in order. Pieces
//!    split within one staged chunk, so a piece never crosses the
//!    classification boundary its chunk was cut on (#12628), and
//!    each distinct piece text is embedded once per run: identical
//!    texts share the one computed vector.
//! 3. Write one new local core scan shard in `shard_dir` —
//!    `atlas-core-<tier>-<topic>-<nn>.db` carrying exactly the shard
//!    schema shard discovery and the fan-out scan read (`chunks`, the
//!    [`DEFAULT_VEC_TABLE`] vector table, `atlas_manifest`), so the
//!    promoted entries are searchable by the existing fan-out with no
//!    new read path. The shard's tier is the unit's recorded
//!    sensitivity; each chunk's `sensitivity` is written in the scan
//!    vocabulary (see [`tier_sensitivity_name`]) — for an item staged
//!    with classification records (#12628), the chunk's own persisted
//!    effective sensitivity; otherwise its item's tier.
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
//! deployment surface) and multi-unit aggregation into one shard. The
//! merge-on-close archival of the staging DB file is performed by
//! [`Registry::promote`] (the unit's DB is branched beside itself as
//! `*.archive` at promotion, stage.rs), so it needs nothing here.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::embed::DocEmbedder;
use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::shards::{EMBEDDING_MODEL_ID, Tier};
use crate::stage::{Registry, UnitState, chunk_classifications};
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
    observed_at: String,
    item_sensitivity: String,
    text: String,
    content_hash: String,
    /// Byte offsets in the original staged item, not the embed-time
    /// piece. These are accumulated in staging chunk order below.
    span_start: i64,
    /// The chunk's persisted effective sensitivity (#12628), for
    /// items staged with classification records: `Some` whenever the
    /// item carries records — a chunk whose record is missing from a
    /// classified item resolves to `secret-bearing` here, never to
    /// the item's tier — and `None` for legacy items staged without
    /// records, whose chunks keep the item-tier behaviour.
    effective_sensitivity: Option<Sensitivity>,
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
    // The unit's per-item classification records, resolved once per
    // item: item id -> (records present?, per-chunk classifications).
    let mut classifications: std::collections::HashMap<
        i64,
        Option<Vec<crate::stage::ChunkClassification>>,
    > = std::collections::HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT id FROM items WHERE unit_id = ?1")
            .map_err(|e| Error::backend(format!("aggregator: reading staged items: {e}")))?;
        let ids = stmt
            .query_map(params![unit_id], |r| r.get::<_, i64>(0))
            .map_err(|e| Error::backend(format!("aggregator: reading staged items: {e}")))?;
        for id in ids {
            let id =
                id.map_err(|e| Error::backend(format!("aggregator: reading staged items: {e}")))?;
            classifications.insert(id, chunk_classifications(&conn, id)?);
        }
    }
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.source, i.created_at, i.sensitivity, c.text, c.content_hash, c.idx \
             FROM chunks_raw c JOIN items i ON i.id = c.item_id \
             WHERE i.unit_id = ?1 ORDER BY i.id, c.idx",
        )
        .map_err(|e| Error::backend(format!("aggregator: reading staged chunks: {e}")))?;
    let rows = stmt
        .query_map(params![unit_id], |r| {
            Ok((
                StagedChunk {
                    item_id: r.get(0)?,
                    item_source: r.get(1)?,
                    observed_at: r.get(2)?,
                    item_sensitivity: r.get(3)?,
                    text: r.get(4)?,
                    content_hash: r.get(5)?,
                    effective_sensitivity: None,
                    span_start: 0,
                },
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(|e| Error::backend(format!("aggregator: reading staged chunks: {e}")))?;
    let mut out = Vec::new();
    let mut item_offsets = std::collections::HashMap::<i64, i64>::new();
    for row in rows {
        let (mut chunk, idx) =
            row.map_err(|e| Error::backend(format!("aggregator: reading staged chunks: {e}")))?;
        chunk.span_start = *item_offsets.entry(chunk.item_id).or_default();
        *item_offsets
            .get_mut(&chunk.item_id)
            .expect("inserted above") += chunk.text.len() as i64;
        chunk.effective_sensitivity = match classifications.get(&chunk.item_id) {
            // Classified item: the chunk's own record governs; a
            // chunk with no record in a classified item fails closed.
            Some(Some(records)) => Some(
                records
                    .get(idx as usize)
                    .map(|record| record.sensitivity)
                    .unwrap_or(Sensitivity::SecretBearing),
            ),
            // Legacy item (or item not seen above): no records.
            _ => None,
        };
        out.push(chunk);
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
        if let Some((t, tp, part)) = crate::shards::parse_shard_name(stem)
            && t == tier
            && tp == topic
        {
            next = next.max(part + 1);
        }
    }
    Ok(next)
}

/// One shard-bound chunk: a staged chunk, or one piece of it after
/// the embed-time context split.
struct ShardChunk {
    item_source: Option<String>,
    observed_at: String,
    item_sensitivity: String,
    /// The staged chunk's persisted effective sensitivity (#12628),
    /// inherited by every embed-time piece of that chunk: pieces are
    /// split *within* one staged chunk, so they never cross the
    /// boundary the chunk was cut on and keep its classification.
    effective_sensitivity: Option<Sensitivity>,
    chunk_idx: i64,
    span_start: i64,
    span_end: i64,
    text: String,
    content_hash: String,
}

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Claim a shard-unique content hash for one piece. The first piece
/// with a given text keeps the plain SHA-256 of that text; a later
/// piece with the same text gets a hash of the text hash, its item id
/// and its chunk index (plus a counter in the unlikely case that is
/// taken too), so the shard insert never trips the UNIQUE constraint.
fn unique_piece_hash(
    used: &mut std::collections::HashSet<String>,
    text_hash: String,
    item_id: i64,
    chunk_idx: i64,
) -> String {
    if used.insert(text_hash.clone()) {
        return text_hash;
    }
    let mut attempt = 0u64;
    loop {
        let candidate = sha256_hex(&format!("{text_hash}\0{item_id}\0{chunk_idx}\0{attempt}"));
        if used.insert(candidate.clone()) {
            return candidate;
        }
        attempt += 1;
    }
}

/// Split `text` into pieces the embedder accepts: each piece
/// tokenizes to at most `embedder.max_input_tokens()`. Pieces break
/// at line boundaries (lines keep their terminators, so the pieces
/// partition `text` exactly); a single line still over the limit is
/// split mid-line at the largest fitting char prefix. An embedder
/// that cannot count tokens gets the text whole — its own
/// `embed_doc` is then the only bound, as for fixture embedders.
fn split_for_embed(text: &str, embedder: &dyn DocEmbedder) -> Result<Vec<String>> {
    let Some(total) = embedder.token_count(text) else {
        return Ok(vec![text.to_string()]);
    };
    let budget = embedder.max_input_tokens();
    if total <= budget {
        return Ok(vec![text.to_string()]);
    }
    let count = |s: &str| -> Result<usize> {
        embedder.token_count(s).ok_or_else(|| {
            Error::fatal(
                "aggregator: embedder stopped returning token counts mid-split".to_string(),
            )
        })
    };
    let mut pieces: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_tokens = 0usize;
    for line in text.split_inclusive('\n') {
        let line_tokens = count(line)?;
        if line_tokens > budget {
            if !cur.is_empty() {
                pieces.push(std::mem::take(&mut cur));
                cur_tokens = 0;
            }
            // Mid-line split: binary search the largest char-boundary
            // prefix that fits (token count is monotone in prefix
            // length for the model's BPE tokenizer).
            let mut rest = line;
            while !rest.is_empty() {
                if count(rest)? <= budget {
                    cur_tokens = count(rest)?;
                    cur = rest.to_string();
                    break;
                }
                let mut lo = 0usize; // chars known to fit
                let mut hi = rest.chars().count(); // chars known not to fit
                while lo + 1 < hi {
                    let mid = (lo + hi) / 2;
                    let prefix: String = rest.chars().take(mid).collect();
                    if count(&prefix)? <= budget {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                if lo == 0 {
                    return Err(Error::fatal(
                        "aggregator: a single character exceeds the embedder token budget"
                            .to_string(),
                    ));
                }
                let split_at = rest
                    .char_indices()
                    .nth(lo)
                    .map(|(b, _)| b)
                    .unwrap_or(rest.len());
                pieces.push(rest[..split_at].to_string());
                rest = &rest[split_at..];
            }
            continue;
        }
        if cur_tokens + line_tokens > budget && !cur.is_empty() {
            pieces.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push_str(line);
        cur_tokens += line_tokens;
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    Ok(pieces)
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

    // Split over-context chunks into embeddable pieces (module
    // docs, step 2), then embed everything before writing anything:
    // a model failure leaves no partial shard and an unstamped,
    // still-open unit.
    let mut pieces: Vec<ShardChunk> = Vec::new();
    let mut item_piece_ordinals: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();
    // The shard's `content_hash` column is UNIQUE, but two pieces can
    // carry the same text: a file that repeats a block splits into
    // identical pieces, and two items can stage identical whole files.
    // Every piece is kept (the pieces must still partition each item's
    // text), so a repeat gets a hash derived from its position (#10836).
    let mut used_hashes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in &chunks {
        let mut piece_offset = c.span_start;
        for piece in split_for_embed(&c.text, embedder)? {
            let ordinal = item_piece_ordinals.entry(c.item_id).or_insert(0);
            let chunk_idx = *ordinal;
            *ordinal += 1;
            let text_hash = if piece == c.text {
                c.content_hash.clone()
            } else {
                sha256_hex(&piece)
            };
            let content_hash = unique_piece_hash(&mut used_hashes, text_hash, c.item_id, chunk_idx);
            pieces.push(ShardChunk {
                item_source: c.item_source.clone(),
                observed_at: c.observed_at.clone(),
                item_sensitivity: c.item_sensitivity.clone(),
                effective_sensitivity: c.effective_sensitivity,
                chunk_idx,
                span_start: piece_offset,
                span_end: piece_offset + piece.len() as i64,
                content_hash,
                text: piece,
            });
            piece_offset = pieces.last().expect("piece just pushed").span_end;
        }
    }
    // Embed each distinct text once (#12628): a document vector is a
    // pure function of (text, model), and this run uses one model,
    // so identical pieces — repeated blocks, identical items, and
    // chunks a boundary revision left unchanged — share the single
    // computed vector instead of being re-embedded. Only a text that
    // actually changed is embedded anew.
    let mut vecs: Vec<Vec<f32>> = Vec::with_capacity(pieces.len());
    let mut vec_by_text: std::collections::HashMap<String, Vec<f32>> =
        std::collections::HashMap::new();
    for piece in &pieces {
        let key = sha256_hex(&piece.text);
        if let Some(vec) = vec_by_text.get(&key) {
            vecs.push(vec.clone());
            continue;
        }
        let vec = embedder.embed_doc(&piece.text)?;
        vec_by_text.insert(key, vec.clone());
        vecs.push(vec);
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
             CREATE TABLE chunk_occurrences ( \
               shard_chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE, \
               canonical_hash TEXT NOT NULL, source TEXT NOT NULL, \
               span_start INTEGER NOT NULL, span_end INTEGER NOT NULL, observed_at TEXT NOT NULL, \
               PRIMARY KEY (shard_chunk_id, source, span_start)); \
             CREATE INDEX chunk_occurrences_hash ON chunk_occurrences (canonical_hash); \
             CREATE TABLE atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);"
        ))
        .map_err(|e| Error::backend(format!("aggregator: shard schema: {e}")))?;
        conn.execute_batch("BEGIN")
            .map_err(|e| Error::backend(format!("aggregator: shard begin: {e}")))?;
        for (i, (c, v)) in pieces.iter().zip(vecs.iter()).enumerate() {
            let id = (i + 1) as i64;
            // The chunk's sensitivity: for a classified item, its
            // persisted effective sensitivity, whose name is already
            // scan vocabulary (the fan-out filters chunk sensitivity
            // against the Sensitivity ladder). Otherwise its item's
            // recorded tier, in scan vocabulary; an unparseable
            // record falls back to the unit's tier rather than
            // widening access.
            let sens = match c.effective_sensitivity {
                Some(sensitivity) => sensitivity.name(),
                None => Tier::parse(&c.item_sensitivity)
                    .map(tier_sensitivity_name)
                    .unwrap_or_else(|| tier_sensitivity_name(unit.sensitivity)),
            };
            let message_id = c
                .item_source
                .clone()
                .unwrap_or_else(|| format!("unit:{unit_id}"));
            conn.execute(
                "INSERT INTO chunks VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, message_id, c.chunk_idx, c.text, c.content_hash, sens],
            )
            .map_err(|e| Error::backend(format!("aggregator: shard chunk {id}: {e}")))?;
            conn.execute(
                &format!("INSERT INTO {DEFAULT_VEC_TABLE} VALUES (?1, ?2)"),
                params![id, vec_blob(v)],
            )
            .map_err(|e| Error::backend(format!("aggregator: shard vector {id}: {e}")))?;
            let source = c
                .item_source
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("unit:{unit_id}"));
            conn.execute(
                "INSERT INTO chunk_occurrences \
                 (shard_chunk_id, canonical_hash, source, span_start, span_end, observed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    sha256_hex(&c.text),
                    source,
                    c.span_start,
                    c.span_end,
                    c.observed_at
                ],
            )
            .map_err(|e| Error::backend(format!("aggregator: shard occurrence {id}: {e}")))?;
        }
        let manifest: [(&str, String); 9] = [
            ("shard", shard_name.clone()),
            ("tier", unit.sensitivity.name().to_string()),
            ("topic", topic.to_string()),
            ("part", part.to_string()),
            ("expected_chunks", pieces.len().to_string()),
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
        chunks: pieces.len(),
        dims,
    })
}
