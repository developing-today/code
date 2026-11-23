//! Query-trace capture — `atlas-query-trace/v1` (issue #9973,
//! follow-on 1 of the #6027 design, doc
//! `2026-11-10T20-41-01Z_design_atlas_retrieval_stats_feedback.md`
//! sections 2-3).
//!
//! Every executed search mints a UUIDv7 `query_id`, builds one
//! trace record carrying per-result stage scores for EVERY pool
//! entrant (served or not), and persists it through a
//! [`TraceSink`] BEFORE the response is released. The write is
//! fail-closed (design invariant 2): a sink failure is a search
//! failure, never a silently untraced success.
//!
//! Embedding-model provenance (#7647, M5 of the #6060 one-model
//! decision): the record head carries `query_embedding_model`
//! (the model the query was embedded with) once, and every result
//! carries `embedding_model` copied from the manifest of the
//! shard that produced that candidate (`BranchPools::
//! candidate_models`; null when the candidate's provenance is
//! unknown, per the identity note below). The distinct candidate
//! models are listed at the head as `embedding_models`; when two
//! or more distinct values appear, `embedding_model_breach` is
//! true — positive evidence of an M1 breach, surfaced as-is,
//! never silently normalised or split. Statistics stay computed
//! over the whole result set without per-model conditioning.
//!
//! Identity note: the design resolves each chunk to
//! `(source_file_id, embed_id, embed_content_hash)` via the chunk
//! detail row and the versioning design's content version. That
//! versioning layer is not in this codebase yet — the detail row
//! carries no such columns — so those fields are emitted as null
//! until it lands; `sensitivity` is the detail row's resolved
//! effective sensitivity. The schema value stays
//! `atlas-query-trace/v1`: filling a null in is not a field
//! redefinition.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::filters::Filters;
use crate::fusion::FusedEntry;
use crate::search::SearchOptions;
use crate::types::{BranchPools, ChunkDetail, round6};

pub const TRACE_SCHEMA: &str = "atlas-query-trace/v1";

/// Who/what executed the search, per the design's record shape.
/// Front-ends fill this in; `retry_of` links a retry execution to
/// the query it retries (a retry is its own execution with its
/// own query_id).
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// `cli | service-api | ui:<name>`.
    pub surface: String,
    /// `drew | agent | user`.
    pub requester_kind: String,
    /// Stable requester id.
    pub requester_id: String,
    pub retry_of: Option<String>,
    /// Whether the surface's retention class permits storing the
    /// raw query text (the hash is always stored).
    pub retain_query_text: bool,
}

impl TraceContext {
    pub fn new(
        surface: impl Into<String>,
        requester_kind: impl Into<String>,
        requester_id: impl Into<String>,
    ) -> TraceContext {
        TraceContext {
            surface: surface.into(),
            requester_kind: requester_kind.into(),
            requester_id: requester_id.into(),
            retry_of: None,
            retain_query_text: false,
        }
    }
}

/// The trace persistence seam. Implementations must return Err on
/// any failure — the caller treats that as a failed search.
pub trait TraceSink {
    fn write_trace(&self, record: &Json) -> Result<()>;
}

/// The production sink: one JSONL file per day under the Atlas
/// state root (`traces/atlas-traces-YYYY-MM-DD.jsonl`), appended
/// one record per line, per design section 3.
#[derive(Debug, Clone)]
pub struct JsonlTraceSink {
    dir: PathBuf,
}

impl JsonlTraceSink {
    pub fn new(dir: PathBuf) -> JsonlTraceSink {
        JsonlTraceSink { dir }
    }

    /// `~/.local/state/atlas-search/traces` — the same state root
    /// as the service config and model store.
    pub fn default_dir() -> PathBuf {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".local/state/atlas-search/traces")
    }
}

impl TraceSink for JsonlTraceSink {
    fn write_trace(&self, record: &Json) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| {
            Error::backend(format!(
                "trace sink: cannot create {}: {e}",
                self.dir.display()
            ))
        })?;
        // File per day, from the record's own ts (its date part).
        let day = record
            .get("ts")
            .and_then(Json::as_str)
            .map(|ts| ts.get(..10).unwrap_or("unknown"))
            .unwrap_or("unknown");
        let path = self.dir.join(format!("atlas-traces-{day}.jsonl"));
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                Error::backend(format!("trace sink: cannot open {}: {e}", path.display()))
            })?;
        let line = serde_json::to_string(record)
            .map_err(|e| Error::backend(format!("trace sink: cannot serialise record: {e}")))?;
        f.write_all(line.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .map_err(|e| {
                Error::backend(format!("trace sink: cannot write {}: {e}", path.display()))
            })?;
        Ok(())
    }
}

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint a UUIDv7 (RFC 9562): 48-bit unix-ms timestamp, version 7,
/// variant 10, remaining bits from /dev/urandom (falling back to a
/// time/pid/counter mix where urandom is unreadable).
pub fn new_query_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut rand = [0u8; 10];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| {
            use std::io::Read;
            f.read_exact(&mut rand)
        })
        .is_ok();
    if !filled {
        let c = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let seed = ms ^ (std::process::id() as u64) << 17 ^ c.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut x = seed;
        for b in rand.iter_mut() {
            // xorshift64* — fallback entropy only.
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *b = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
    }
    let mut b = [0u8; 16];
    b[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    b[6] = 0x70 | (rand[0] & 0x0f);
    b[7] = rand[1];
    b[8] = 0x80 | (rand[2] & 0x3f);
    b[9..16].copy_from_slice(&rand[3..10]);
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// sha256 hex of the normalised query text (trimmed, whitespace
/// collapsed) — the grouping key statistics use.
pub fn query_text_hash(query: &str) -> String {
    let normalised = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let digest = Sha256::digest(normalised.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC3339 UTC for a unix-ms instant (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn rfc3339_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod / 60) % 60,
        tod % 60
    )
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Assemble one `atlas-query-trace/v1` record. `ranked` is the
/// full fused pool (every entrant, in fusion order); `served`
/// lists the chunk ids actually rendered, in rendered order —
/// entrants not in `served` are recorded with `served: false`
/// and no final rank. `shard_id` names the single logical shard
/// the backends present to fusion today (the backend name).
#[allow(clippy::too_many_arguments)]
pub fn build_trace_record(
    query_id: &str,
    ctx: &TraceContext,
    started_ms: u64,
    duration_ms: u64,
    opts: &SearchOptions,
    filters: &Filters,
    shard_id: &str,
    pools: &BranchPools,
    ranked: &[FusedEntry],
    vec_dist: &HashMap<i64, Option<f64>>,
    details: &HashMap<i64, ChunkDetail>,
    served: &[i64],
) -> Json {
    let fts_rank: HashMap<i64, usize> = pools
        .fts
        .iter()
        .enumerate()
        .map(|(i, h)| (h.chunk_id, i + 1))
        .collect();
    let vec_rank: HashMap<i64, usize> = pools
        .vector
        .iter()
        .enumerate()
        .map(|(i, h)| (h.chunk_id, i + 1))
        .collect();
    let served_rank: HashMap<i64, usize> = served
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i + 1))
        .collect();

    let mut distinct_models: Vec<String> = Vec::new();
    let mut results = Vec::new();
    for (i, e) in ranked.iter().enumerate() {
        let d = details.get(&e.chunk_id);
        // Shard stage: the branch where the entrant ranked best
        // (rank_sources in the merge stage preserves both). The
        // branch score is the vector distance for the vector
        // branch; the FTS branch carries no per-hit score in
        // BranchHit, so it is null there.
        let (branch, rank_in_shard, shard_score) =
            match (fts_rank.get(&e.chunk_id), vec_rank.get(&e.chunk_id)) {
                (Some(f), Some(v)) if v <= f => (
                    "vector",
                    *v,
                    vec_dist.get(&e.chunk_id).copied().flatten().map(round6),
                ),
                (Some(f), _) => ("fts", *f, None),
                (None, Some(v)) => (
                    "vector",
                    *v,
                    vec_dist.get(&e.chunk_id).copied().flatten().map(round6),
                ),
                (None, None) => ("fts", 0, None),
            };
        let mut stages = Map::new();
        stages.insert(
            "shard".into(),
            json!({
                "shard_id": shard_id,
                "branch": branch,
                "rank_in_shard": rank_in_shard,
                "score": shard_score.map(Json::from).unwrap_or(Json::Null),
            }),
        );
        stages.insert(
            "merge".into(),
            json!({
                "rank_after_fusion": i + 1,
                "fusion_score": round6(e.score),
                "rank_sources": e.rank_sources_json(),
            }),
        );
        // No rescore stage exists in the pipeline yet (design
        // weights.rescore is "none"); recorded explicitly.
        stages.insert(
            "rescore".into(),
            json!({"applied": false, "kind": "none", "score": Json::Null}),
        );
        let final_rank = served_rank.get(&e.chunk_id);
        stages.insert(
            "final".into(),
            json!({
                "rank": final_rank.map(|r| Json::from(*r)).unwrap_or(Json::Null),
                "served": final_rank.is_some(),
            }),
        );
        let mut r = Map::new();
        r.insert("chunk_id".into(), json!(e.chunk_id));
        // Embedding-model provenance (#7647): the model recorded
        // in the producing shard's atlas_manifest, or null when
        // the candidate carries no manifest provenance.
        let candidate_model = pools.candidate_models.get(&e.chunk_id);
        if let Some(m) = candidate_model
            && !distinct_models.contains(m)
        {
            distinct_models.push(m.clone());
        }
        r.insert(
            "embedding_model".into(),
            candidate_model.map_or(Json::Null, |m| Json::String(m.clone())),
        );
        // Resolved embed identity: null until the versioning
        // design's detail columns land (see module note).
        r.insert("source_file_id".into(), Json::Null);
        r.insert("embed_id".into(), Json::Null);
        r.insert("embed_content_hash".into(), Json::Null);
        r.insert(
            "embed_state".into(),
            json!({"version": Json::Null, "tier": Json::Null}),
        );
        r.insert("provenance_mark".into(), Json::Null);
        r.insert(
            "sensitivity".into(),
            d.and_then(|d| d.eff_sensitivity.as_ref())
                .map_or(Json::Null, |s| Json::String(s.clone())),
        );
        r.insert("stages".into(), Json::Object(stages));
        results.push(Json::Object(r));
    }

    let mut parameters = Map::new();
    parameters.insert("pool".into(), json!(opts.pool));
    parameters.insert("top_k".into(), json!(opts.k));
    parameters.insert(
        "vector_backend".into(),
        Json::String(opts.vector_backend.as_str().to_string()),
    );
    parameters.insert(
        "weights".into(),
        json!({
            "fts": opts.parameters.fts_weight,
            "vector": opts.parameters.vector_weight,
            "rescore": "none",
        }),
    );
    parameters.insert("filters".into(), filters.describe());

    let mut per_shard = Map::new();
    per_shard.insert(
        shard_id.to_string(),
        json!({"fts": pools.fts.len(), "vector": pools.vector.len()}),
    );

    let mut rec = Map::new();
    rec.insert("schema".into(), Json::String(TRACE_SCHEMA.to_string()));
    rec.insert("query_id".into(), Json::String(query_id.to_string()));
    rec.insert(
        "retry_of".into(),
        ctx.retry_of
            .as_ref()
            .map_or(Json::Null, |r| Json::String(r.clone())),
    );
    rec.insert("ts".into(), Json::String(rfc3339_utc(started_ms)));
    rec.insert("duration_ms".into(), json!(duration_ms));
    rec.insert("surface".into(), Json::String(ctx.surface.clone()));
    rec.insert(
        "requester".into(),
        json!({"kind": ctx.requester_kind, "id": ctx.requester_id}),
    );
    rec.insert(
        "query_text_hash".into(),
        Json::String(query_text_hash(&opts.query)),
    );
    rec.insert(
        "query_text".into(),
        if ctx.retain_query_text {
            Json::String(opts.query.clone())
        } else {
            Json::Null
        },
    );
    rec.insert("parameters".into(), Json::Object(parameters));
    // M5 head provenance (#7647): the query's embedding model
    // once, the distinct candidate models, and the M1-breach flag
    // (two or more distinct candidate models in one trace).
    rec.insert(
        "query_embedding_model".into(),
        Json::String(opts.model.clone()),
    );
    distinct_models.sort();
    rec.insert(
        "embedding_model_breach".into(),
        json!(distinct_models.len() > 1),
    );
    rec.insert("embedding_models".into(), json!(distinct_models));
    rec.insert("results".into(), Json::Array(results));
    rec.insert(
        "pool_counts".into(),
        json!({
            "per_shard": Json::Object(per_shard),
            "merged": ranked.len(),
            "rescored": 0,
            "served": served.len(),
        }),
    );
    Json::Object(rec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_id_is_uuidv7_shaped() {
        let id = new_query_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'7', "version nibble: {id}");
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_ne!(new_query_id(), new_query_id());
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_731_328_000_000), "2024-11-11T12:26:40Z");
        assert_eq!(rfc3339_utc(1_763_000_000_000), "2025-11-13T02:13:20Z");
    }

    fn trace_fixture(candidate_models: std::collections::HashMap<i64, String>) -> Json {
        let mut opts = SearchOptions::new("provenance query");
        opts.model = "model-query-v1".to_string();
        let pools = BranchPools {
            fts: Vec::new(),
            vector: vec![
                crate::types::BranchHit {
                    chunk_id: 1,
                    dist: Some(0.1),
                },
                crate::types::BranchHit {
                    chunk_id: 2,
                    dist: Some(0.2),
                },
            ],
            candidate_models,
        };
        let ranked = vec![
            crate::fusion::FusedEntry {
                chunk_id: 1,
                score: 0.5,
                fts_rank: None,
                vector_rank: Some(1),
            },
            crate::fusion::FusedEntry {
                chunk_id: 2,
                score: 0.4,
                fts_rank: None,
                vector_rank: Some(2),
            },
        ];
        let ctx = TraceContext::new("cli", "agent", "test");
        build_trace_record(
            "qid-test",
            &ctx,
            0,
            1,
            &opts,
            &Filters::default(),
            "turso",
            &pools,
            &ranked,
            &HashMap::new(),
            &HashMap::new(),
            &[1, 2],
        )
    }

    #[test]
    fn trace_records_uniform_embedding_model_provenance() {
        // Oracle (#7647 acceptance 1): a uniform-model trace shows
        // query_embedding_model at the head and the matching
        // embedding_model on every candidate, with no breach.
        let mut models = std::collections::HashMap::new();
        models.insert(1, "model-a".to_string());
        models.insert(2, "model-a".to_string());
        let rec = trace_fixture(models);
        assert_eq!(rec["query_embedding_model"], json!("model-query-v1"));
        assert_eq!(rec["embedding_model_breach"], json!(false));
        assert_eq!(rec["embedding_models"], json!(["model-a"]));
        let results = rec["results"].as_array().expect("results array");
        assert_eq!(results.len(), 2);
        for r in results {
            assert_eq!(r["embedding_model"], json!("model-a"));
        }
    }

    #[test]
    fn trace_flags_mixed_embedding_models_as_m1_breach() {
        // Oracle (#7647 acceptance 2): a mixed-model fixture
        // exposes both model ids and flags the breach; the values
        // are never normalised or split.
        let mut models = std::collections::HashMap::new();
        models.insert(1, "model-a".to_string());
        models.insert(2, "model-b".to_string());
        let rec = trace_fixture(models);
        assert_eq!(rec["embedding_model_breach"], json!(true));
        assert_eq!(rec["embedding_models"], json!(["model-a", "model-b"]));
        let results = rec["results"].as_array().expect("results array");
        assert_eq!(results[0]["embedding_model"], json!("model-a"));
        assert_eq!(results[1]["embedding_model"], json!("model-b"));
    }

    #[test]
    fn trace_without_manifest_provenance_records_null_models() {
        // Oracle (#7647): candidates with no shard-manifest
        // provenance record null, and a single distinct (absent)
        // model set is not a breach.
        let rec = trace_fixture(std::collections::HashMap::new());
        assert_eq!(rec["embedding_model_breach"], json!(false));
        assert_eq!(rec["embedding_models"], json!([]));
        let results = rec["results"].as_array().expect("results array");
        for r in results {
            assert_eq!(r["embedding_model"], Json::Null);
        }
    }

    #[test]
    fn hash_normalises_whitespace() {
        assert_eq!(
            query_text_hash("  how  do we\nback up "),
            query_text_hash("how do we back up")
        );
        assert_eq!(query_text_hash("x").len(), 64);
    }
}
