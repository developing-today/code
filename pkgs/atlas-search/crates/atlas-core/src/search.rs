//! Search orchestration and envelope rendering — the Python's
//! module-level `run_search` / `run_get_chunk` / `result_entry` /
//! `grouped_output` / `envelope`, minus argparse and printing
//! (front-ends own those). The JSON shapes, field order, note
//! placement and fatal messages mirror the Python exactly; the
//! parity tests diff rendered envelopes against envelopes
//! recorded from the production module.

use std::collections::HashMap;

use atlas_proto::SearchParametersV1;
use serde_json::{Map, Value as Json, json};

use crate::backend::SearchBackend;
use crate::embed::QueryEmbedder;
use crate::error::{Error, Result};
use crate::filters::Filters;
use crate::fusion::{FusedEntry, fts_match_expr, fuse_with_parameters};
use crate::types::{BranchPools, BranchQuery, ChunkDetail, DEFAULT_MODEL, VectorBackend, round6};

/// The Python's `MESSAGE_TEXT_CAP`.
pub const MESSAGE_TEXT_CAP: usize = 50_000;

/// Which branches run — the CLI's `--mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    Fts,
    Vector,
    #[default]
    Hybrid,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchMode::Fts => "fts",
            SearchMode::Vector => "vector",
            SearchMode::Hybrid => "hybrid",
        }
    }

    pub fn parse(s: &str) -> Option<SearchMode> {
        match s {
            "fts" => Some(SearchMode::Fts),
            "vector" => Some(SearchMode::Vector),
            "hybrid" => Some(SearchMode::Hybrid),
            _ => None,
        }
    }

    fn uses_fts(&self) -> bool {
        matches!(self, SearchMode::Fts | SearchMode::Hybrid)
    }

    fn uses_vector(&self) -> bool {
        matches!(self, SearchMode::Vector | SearchMode::Hybrid)
    }
}

/// Everything `run_search` reads from the Python's argparse
/// namespace. Defaults are the production CLI defaults.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub mode: SearchMode,
    /// Top-k results (grouped: top-k sessions). Default 10.
    pub k: usize,
    /// Per-branch candidate pool. Default 100.
    pub pool: usize,
    pub group_by_session: bool,
    /// Neighbor chunks to attach around each hit (0 = off).
    pub expand: usize,
    pub with_message: bool,
    /// Provenance strings, recorded verbatim in the envelope's
    /// `vector` block (the Python takes them from CLI args).
    pub model: String,
    pub embedder_label: String,
    /// Query-vector truncation (0 = full width).
    pub trunc: usize,
    pub vector_backend: VectorBackend,
    /// Versioned query-time controls shared with the wire protocol.
    pub parameters: SearchParametersV1,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            query: String::new(),
            mode: SearchMode::Hybrid,
            k: SearchParametersV1::default_k(),
            pool: SearchParametersV1::default_pool(),
            group_by_session: false,
            expand: SearchParametersV1::default_expand(),
            with_message: false,
            model: DEFAULT_MODEL.to_string(),
            embedder_label: "auto".to_string(),
            trunc: 0,
            vector_backend: VectorBackend::Scan,
            parameters: SearchParametersV1::default(),
        }
    }
}

impl SearchOptions {
    pub fn new(query: impl Into<String>) -> Self {
        SearchOptions {
            query: query.into(),
            ..SearchOptions::default()
        }
    }
}

/// The seam by which a front-end supplies branch pools to the
/// search orchestration. The default executor is the backend
/// itself; serve mode supplies an executor that runs the FTS and
/// vector branches concurrently while preserving this module's
/// exact validation, fusion, and rendering semantics.
pub trait BranchPoolExecutor {
    fn branch_pools(&self, query: &BranchQuery, filters: &Filters) -> Result<BranchPools>;
}

/// The production branch executor: one call to the backend, whose
/// implementations preserve the Python's branch-call pattern.
pub struct BackendBranchExecutor<'a> {
    backend: &'a dyn SearchBackend,
}

impl<'a> BackendBranchExecutor<'a> {
    pub fn new(backend: &'a dyn SearchBackend) -> BackendBranchExecutor<'a> {
        BackendBranchExecutor { backend }
    }
}

impl BranchPoolExecutor for BackendBranchExecutor<'_> {
    fn branch_pools(&self, query: &BranchQuery, filters: &Filters) -> Result<BranchPools> {
        self.backend.branch_pools(query, filters)
    }
}

/// The Python's `run_search`. The embedder is only invoked for
/// vector/hybrid modes; the filters' ceiling is enforced inside
/// the backend's branch construction (filter-before-rank).
pub fn run_search(
    backend: &dyn SearchBackend,
    embedder: &dyn QueryEmbedder,
    filters: &Filters,
    opts: &SearchOptions,
) -> Result<Json> {
    run_search_with_executor(
        backend,
        embedder,
        filters,
        opts,
        &BackendBranchExecutor::new(backend),
    )
}

/// [`run_search`] with a caller-supplied branch executor. Every
/// step other than obtaining the branch pools — validation, query
/// embedding, fusion, detail lookup, and envelope rendering — is
/// shared with `run_search` verbatim.
pub fn run_search_with_executor(
    backend: &dyn SearchBackend,
    embedder: &dyn QueryEmbedder,
    filters: &Filters,
    opts: &SearchOptions,
    executor: &dyn BranchPoolExecutor,
) -> Result<Json> {
    opts.parameters
        .validate(opts.expand)
        .map_err(Error::fatal)?;
    if opts.expand > crate::MAX_EXPAND_NEIGHBORS {
        return Err(Error::fatal(format!(
            "expand must be at most {}",
            crate::MAX_EXPAND_NEIGHBORS
        )));
    }
    if opts.mode.uses_vector()
        && opts.vector_backend != VectorBackend::Scan
        && backend.name() != "turso"
    {
        return Err(Error::fatal(
            "--vector-backend index modes need the turso backend \
             (the vector index lives in the Turso publish)",
        ));
    }
    let match_expr = if opts.mode.uses_fts() {
        fts_match_expr(&opts.query)
    } else {
        None
    };
    if opts.mode == SearchMode::Fts && match_expr.is_none() {
        return Err(Error::fatal(
            "query has no FTS-searchable terms (all words too short); try --mode vector",
        ));
    }
    let qvec = if opts.mode.uses_vector() {
        Some(embedder.embed_query(&opts.query, opts.trunc)?)
    } else {
        None
    };

    let mut notes = backend.notes();
    if backend.has_vector_store() == Some(false) && opts.mode.uses_vector() {
        notes.push(
            "no vector store found for the local backend; vector branch returned nothing"
                .to_string(),
        );
    }
    let pools = executor.branch_pools(
        &BranchQuery {
            match_expr,
            query_vector: qvec,
            pool: opts.pool,
            shard_top_k: opts.parameters.shard_top_k,
            global_top_k: opts.parameters.global_top_k,
            vector_backend: opts.vector_backend,
        },
        filters,
    )?;
    let (ranked, vec_dist) = fuse_with_parameters(
        &pools.fts,
        &pools.vector,
        opts.parameters.rrf_k,
        opts.parameters.fts_weight,
        opts.parameters.vector_weight,
    );

    // The local backend learns things mid-search (vector
    // coverage); fold any late notes in before rendering.
    for n in backend.notes() {
        if !notes.contains(&n) {
            notes.push(n);
        }
    }

    let mut out = if opts.group_by_session {
        grouped_output(backend, opts, filters, &ranked, &vec_dist, &notes)?
    } else {
        let keep = &ranked[..opts.k.min(ranked.len())];
        let ids: Vec<i64> = keep.iter().map(|e| e.chunk_id).collect();
        let details = details_map(backend, &ids)?;
        let mut results = Vec::new();
        for e in keep {
            let Some(d) = details.get(&e.chunk_id) else {
                continue;
            };
            results.push(result_entry(backend, opts, e, d, &vec_dist)?);
        }
        envelope(backend, opts, filters, Json::Array(results), &notes)
    };
    if opts.mode.uses_vector() {
        // Provenance: exactly which model/table produced the pool.
        let mut v = Map::new();
        v.insert("model".into(), Json::String(opts.model.clone()));
        v.insert(
            "table".into(),
            Json::String(backend.vec_table().to_string()),
        );
        v.insert("embedder".into(), Json::String(opts.embedder_label.clone()));
        v.insert(
            "trunc".into(),
            if opts.trunc > 0 {
                json!(opts.trunc)
            } else {
                Json::Null
            },
        );
        v.insert("pool".into(), json!(opts.pool));
        v.insert(
            "vector_backend".into(),
            Json::String(opts.vector_backend.as_str().to_string()),
        );
        out.as_object_mut()
            .expect("envelope is an object")
            .insert("vector".into(), Json::Object(v));
    }
    Ok(out)
}

/// The Python's `run_get_chunk`: a by-id lookup with NO
/// sensitivity ceiling (an explicit-id tool, like the Python).
pub fn run_get_chunk(backend: &dyn SearchBackend, chunk_id: i64, neighbors: usize) -> Result<Json> {
    let details = backend.chunk_details(&[chunk_id])?;
    let Some(d) = details.iter().find(|d| d.chunk_id == chunk_id) else {
        return Err(Error::fatal(format!("chunk {chunk_id} not found")));
    };
    let mut msg = Map::new();
    msg.insert("id".into(), Json::String(d.message_id.clone()));
    msg.insert(
        "role".into(),
        d.m_role
            .as_ref()
            .map_or(Json::Null, |r| Json::String(r.clone())),
    );
    msg.insert("session_id".into(), Json::String(d.session_id.clone()));
    let mut out = Map::new();
    out.insert("chunk_id".into(), json!(d.chunk_id));
    out.insert("text".into(), Json::String(d.text.clone()));
    out.insert("chunk_idx".into(), json!(d.chunk_idx));
    out.insert("message".into(), Json::Object(msg));
    out.insert("session".into(), session_obj(d));
    out.insert("session_id".into(), Json::String(d.session_id.clone()));
    if neighbors > 0 {
        out.insert("context".into(), neighbor_map(backend, d, neighbors)?);
    }
    let notes = backend.notes();
    if !notes.is_empty() {
        out.insert("notes".into(), json!(notes));
    }
    Ok(Json::Object(out))
}

fn details_map(backend: &dyn SearchBackend, ids: &[i64]) -> Result<HashMap<i64, ChunkDetail>> {
    let rows = backend.chunk_details(ids)?;
    Ok(rows.into_iter().map(|d| (d.chunk_id, d)).collect())
}

fn envelope(
    backend: &dyn SearchBackend,
    opts: &SearchOptions,
    filters: &Filters,
    results: Json,
    notes: &[String],
) -> Json {
    let mut out = Map::new();
    out.insert("query".into(), Json::String(opts.query.clone()));
    out.insert("mode".into(), Json::String(opts.mode.as_str().to_string()));
    out.insert("backend".into(), Json::String(backend.name().to_string()));
    out.insert("filters_applied".into(), filters.describe());
    out.insert("results".into(), results);
    if !notes.is_empty() {
        out.insert("notes".into(), json!(notes));
    }
    Json::Object(out)
}

fn session_obj(d: &ChunkDetail) -> Json {
    let mut s = Map::new();
    s.insert(
        "title".into(),
        d.s_title
            .as_ref()
            .map_or(Json::Null, |v| Json::String(v.clone())),
    );
    s.insert(
        "client".into(),
        d.s_client
            .as_ref()
            .map_or(Json::Null, |v| Json::String(v.clone())),
    );
    s.insert(
        "created_at".into(),
        d.s_created_at
            .as_ref()
            .map_or(Json::Null, |v| Json::String(v.clone())),
    );
    s.insert(
        "kind".into(),
        d.l_kind
            .as_ref()
            .map_or(Json::Null, |v| Json::String(v.clone())),
    );
    s.insert(
        "topics".into(),
        Json::Array(crate::types::parse_json_list(d.l_topics.as_deref())),
    );
    s.insert(
        "projects".into(),
        Json::Array(crate::types::parse_json_list(d.l_projects.as_deref())),
    );
    s.insert(
        "sensitivity".into(),
        d.eff_sensitivity
            .as_ref()
            .map_or(Json::Null, |v| Json::String(v.clone())),
    );
    Json::Object(s)
}

fn result_entry(
    backend: &dyn SearchBackend,
    opts: &SearchOptions,
    e: &FusedEntry,
    d: &ChunkDetail,
    vec_dist: &HashMap<i64, Option<f64>>,
) -> Result<Json> {
    let mut entry = Map::new();
    entry.insert("chunk_id".into(), json!(e.chunk_id));
    entry.insert("session_id".into(), Json::String(d.session_id.clone()));
    entry.insert("score".into(), json!(round6(e.score)));
    entry.insert("rank_sources".into(), e.rank_sources_json());
    entry.insert("text".into(), Json::String(d.text.clone()));
    entry.insert("session".into(), session_obj(d));
    entry_chunk_extras(backend, opts, e, d, vec_dist, &mut entry)?;
    Ok(Json::Object(entry))
}

/// vector_distance / context / message — the trailing fields
/// shared by flat result entries and grouped chunk entries.
fn entry_chunk_extras(
    backend: &dyn SearchBackend,
    opts: &SearchOptions,
    e: &FusedEntry,
    d: &ChunkDetail,
    vec_dist: &HashMap<i64, Option<f64>>,
    entry: &mut Map<String, Json>,
) -> Result<()> {
    if let Some(Some(dist)) = vec_dist.get(&e.chunk_id) {
        entry.insert("vector_distance".into(), json!(round6(*dist)));
    }
    if opts.expand > 0 {
        entry.insert("context".into(), neighbor_map(backend, d, opts.expand)?);
    }
    if opts.with_message {
        entry.insert("message".into(), message_obj(backend, d)?);
    }
    Ok(())
}

/// The Python's `neighbor_map`.
fn neighbor_map(backend: &dyn SearchBackend, d: &ChunkDetail, n: usize) -> Result<Json> {
    let n = i64::try_from(n)
        .map_err(|_| Error::fatal("neighbor count does not fit in the backend's signed range"))?;
    let rows = backend.neighbors(&d.message_id, d.chunk_idx, n)?;
    let mut before = Vec::new();
    let mut after = Vec::new();
    for r in &rows {
        let item = json!({"chunk_id": r.chunk_id, "text": r.text});
        if r.chunk_idx < d.chunk_idx {
            before.push(item);
        } else if r.chunk_idx > d.chunk_idx {
            after.push(item);
        }
    }
    Ok(json!({"before": before, "after": after}))
}

/// The Python's `message_obj` (null when the message is gone).
fn message_obj(backend: &dyn SearchBackend, d: &ChunkDetail) -> Result<Json> {
    let Some(m) = backend.message(&d.message_id)? else {
        return Ok(Json::Null);
    };
    let truncated = m.text.chars().count() > MESSAGE_TEXT_CAP;
    let text: String = m.text.chars().take(MESSAGE_TEXT_CAP).collect();
    Ok(json!({
        "id": m.id,
        "role": m.role,
        "text": text,
        "truncated": truncated,
    }))
}

struct Group {
    session_id: String,
    score: f64,
    hit_count: usize,
    first_detail: ChunkDetail,
    chunks: Vec<(FusedEntry, ChunkDetail)>,
}

/// The Python's `grouped_output`: session score = best chunk
/// score; sessions ordered by (best score, hit count), first-seen
/// order breaking full ties (Python's sort is stable; so is
/// Rust's `sort_by`).
fn grouped_output(
    backend: &dyn SearchBackend,
    opts: &SearchOptions,
    filters: &Filters,
    ranked: &[FusedEntry],
    vec_dist: &HashMap<i64, Option<f64>>,
    notes: &[String],
) -> Result<Json> {
    // The consideration window is k * 10, but k is unbounded at
    // every boundary that reaches this function (CLI, service), so
    // the multiplication must saturate: a wrap-range k would
    // otherwise wrap the window to near zero in release builds
    // (silently truncating the grouped results) and panic in
    // debug builds (#5423). Saturation makes a huge k behave as
    // "consider everything", which the .min(ranked.len()) then
    // bounds to the actual candidate list.
    let window = opts.k.saturating_mul(10).max(opts.pool).min(ranked.len());
    let consideration = &ranked[..window];
    let ids: Vec<i64> = consideration.iter().map(|e| e.chunk_id).collect();
    let details = details_map(backend, &ids)?;
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Group> = HashMap::new();
    for e in consideration {
        let Some(d) = details.get(&e.chunk_id) else {
            continue;
        };
        let sid = d.session_id.clone();
        if !groups.contains_key(&sid) {
            order.push(sid.clone());
            groups.insert(
                sid.clone(),
                Group {
                    session_id: sid.clone(),
                    score: e.score,
                    hit_count: 0,
                    first_detail: d.clone(),
                    chunks: Vec::new(),
                },
            );
        }
        let g = groups.get_mut(&sid).expect("group just inserted");
        g.hit_count += 1;
        if e.score > g.score {
            g.score = e.score;
        }
        g.chunks.push((e.clone(), d.clone()));
    }
    let mut sorted: Vec<Group> = order
        .into_iter()
        .map(|sid| groups.remove(&sid).expect("group exists"))
        .collect();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.hit_count.cmp(&a.hit_count))
    });
    sorted.truncate(opts.k);
    let mut results = Vec::new();
    for g in &sorted {
        let mut chunks = Vec::new();
        for (e, d) in g.chunks.iter().take(5) {
            let mut ce = Map::new();
            ce.insert("chunk_id".into(), json!(e.chunk_id));
            ce.insert("score".into(), json!(round6(e.score)));
            ce.insert("rank_sources".into(), e.rank_sources_json());
            ce.insert("text".into(), Json::String(d.text.clone()));
            entry_chunk_extras(backend, opts, e, d, vec_dist, &mut ce)?;
            chunks.push(Json::Object(ce));
        }
        let mut entry = Map::new();
        entry.insert("session_id".into(), Json::String(g.session_id.clone()));
        entry.insert("score".into(), json!(round6(g.score)));
        entry.insert("hit_count".into(), json!(g.hit_count));
        entry.insert("session".into(), session_obj(&g.first_detail));
        entry.insert("chunks".into(), Json::Array(chunks));
        results.push(Json::Object(entry));
    }
    Ok(envelope(
        backend,
        opts,
        filters,
        Json::Array(results),
        notes,
    ))
}
