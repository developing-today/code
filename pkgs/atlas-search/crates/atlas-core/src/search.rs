//! Search orchestration and envelope rendering — the Python's
//! module-level `run_search` / `run_get_chunk` / `result_entry` /
//! `grouped_output` / `envelope`, minus argparse and printing
//! (front-ends own those). The JSON shapes, field order, note
//! placement and fatal messages mirror the Python exactly; the
//! parity tests diff rendered envelopes against envelopes
//! recorded from the production module.

use std::collections::HashMap;

use atlas_proto::{RawMode, SearchParametersV1};
use serde_json::{Map, Value as Json, json};

use crate::backend::SearchBackend;
use crate::disclosure::{DisclosureContext, chunk_classification};
use crate::embed::QueryEmbedder;
use crate::error::{Error, Result};
use crate::filters::{Filters, Sensitivity};
use crate::fusion::{FusedEntry, fts_match_expr, fuse_with_parameters};
use crate::trace::{TraceContext, TraceSink, build_trace_record, new_query_id};
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
    /// The disclosure context of the destination surface
    /// (#12404, #9096 design section 4). The context ceiling
    /// intersects the filters' caller ceiling at the output
    /// boundary; neither can widen the other. The default is
    /// `public-surface` — fail-closed: a caller that does not
    /// state its surface's context gets the strictest outbound
    /// treatment. Front-ends state their context explicitly.
    pub context: DisclosureContext,
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
            context: DisclosureContext::PublicSurface,
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

/// A pending trace write (issue #9973): the sink + context a
/// traced search persists its `atlas-query-trace/v1` record
/// through, and the query_id minted at request receipt.
pub struct TraceJob<'a> {
    pub sink: &'a dyn TraceSink,
    pub ctx: TraceContext,
    pub query_id: String,
}

/// [`run_search`] with query-trace capture (issue #9973, design
/// sections 2-3): mints a `query_id`, returns it in the envelope,
/// and persists the trace record BEFORE the response is
/// released. Fail-closed: a trace write failure fails the search.
pub fn run_search_traced(
    backend: &dyn SearchBackend,
    embedder: &dyn QueryEmbedder,
    filters: &Filters,
    opts: &SearchOptions,
    sink: &dyn TraceSink,
    ctx: TraceContext,
) -> Result<Json> {
    run_search_with_executor_traced(
        backend,
        embedder,
        filters,
        opts,
        &BackendBranchExecutor::new(backend),
        sink,
        ctx,
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
    run_search_impl(backend, embedder, filters, opts, executor, None)
}

/// [`run_search_with_executor`] with query-trace capture; see
/// [`run_search_traced`].
#[allow(clippy::too_many_arguments)]
pub fn run_search_with_executor_traced(
    backend: &dyn SearchBackend,
    embedder: &dyn QueryEmbedder,
    filters: &Filters,
    opts: &SearchOptions,
    executor: &dyn BranchPoolExecutor,
    sink: &dyn TraceSink,
    ctx: TraceContext,
) -> Result<Json> {
    let job = TraceJob {
        sink,
        ctx,
        query_id: new_query_id(),
    };
    run_search_impl(backend, embedder, filters, opts, executor, Some(&job))
}

fn run_search_impl(
    backend: &dyn SearchBackend,
    embedder: &dyn QueryEmbedder,
    filters: &Filters,
    opts: &SearchOptions,
    executor: &dyn BranchPoolExecutor,
    trace: Option<&TraceJob<'_>>,
) -> Result<Json> {
    let trace_started = trace.map(|_| {
        (
            std::time::Instant::now(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        )
    });
    opts.parameters
        .validate(opts.expand)
        .map_err(Error::fatal)?;
    if opts.expand > crate::MAX_EXPAND_NEIGHBORS {
        return Err(Error::fatal(format!(
            "expand must be at most {}",
            crate::MAX_EXPAND_NEIGHBORS
        )));
    }
    // Hydration renders on the flat output; with session grouping
    // on and raw output off there is no flat row to hydrate.
    // Rather than silently ignoring the flag, refuse the
    // combination -- --raw-rows produces the flat rows hydration
    // attaches to (#9942).
    if opts.parameters.hydrate > 0
        && opts.group_by_session
        && opts.parameters.raw_mode == RawMode::Off
    {
        return Err(Error::fatal(
            "hydrate requires flat output: disable group_by_session or use raw_mode",
        ));
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
    let (mut ranked, vec_dist) = fuse_with_parameters(
        &pools.fts,
        &pools.vector,
        opts.parameters.rrf_k,
        opts.parameters.fts_weight,
        opts.parameters.vector_weight,
    );

    // Jaccard rescore (#5733): a third signal over the fused
    // candidate union only — retrieval is unchanged. Candidate
    // shingle sets are computed from chunk text at this stage (the
    // union is depth-bounded, so the cost is too); the design's
    // stored sets drop into rescore_fused unchanged when the ingest
    // side store lands. Off unless jaccard_depth is set.
    let mut jaccard_note: Option<String> = None;
    if opts.parameters.jaccard_depth > 0 && !ranked.is_empty() {
        let depth = opts.parameters.jaccard_depth.min(ranked.len());
        let ids: Vec<i64> = ranked[..depth].iter().map(|e| e.chunk_id).collect();
        let details = details_map(backend, &ids)?;
        let query_set = crate::shingles::ShingleSet::from_text(&opts.query);
        let sets: std::collections::HashMap<i64, crate::shingles::ShingleSet> = details
            .values()
            .map(|d| (d.chunk_id, crate::shingles::ShingleSet::from_text(&d.text)))
            .collect();
        let threshold = if opts.parameters.jaccard_threshold == 0.0 {
            crate::shingles::DEFAULT_NEAR_EXACT_THRESHOLD
        } else {
            opts.parameters.jaccard_threshold
        };
        let report =
            crate::shingles::rescore_fused(&mut ranked, &query_set, &sets, depth, threshold);
        jaccard_note = Some(format!(
            "jaccard rescore: {} candidates rescored (depth {}), {} near-exact at threshold {}",
            report.rescored, depth, report.near_exact, threshold
        ));
    }

    // The local backend learns things mid-search (vector
    // coverage); fold any late notes in before rendering.
    for n in backend.notes() {
        if !notes.contains(&n) {
            notes.push(n);
        }
    }
    if let Some(n) = jaccard_note {
        notes.push(n);
    }

    // Disclosure-context enforcement (#12404, #9096 design
    // section 4): the caller's ceiling was applied inside the
    // branch queries (filter-before-rank); here, at the final
    // output boundary, the destination context's ceiling
    // intersects it — neither can widen the other, and
    // excluded content is ineligible in every context. Every
    // rendered shape below (flat, grouped, raw branch rows)
    // draws only from `permitted`, and the trace's served set
    // is collected from the rendered envelope, so cached and
    // paged payloads inherit the same filter. Details are
    // resolved for every ranked candidate (the traced path
    // already pays this) because a candidate's classification
    // is a property of its detail row.
    let all_ids: Vec<i64> = ranked.iter().map(|e| e.chunk_id).collect();
    let all_details = details_map(backend, &all_ids)?;
    let mut permitted: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut withheld = 0usize;
    for e in &ranked {
        match all_details.get(&e.chunk_id) {
            Some(d) => {
                let class = chunk_classification(d.eff_sensitivity.as_deref());
                if opts
                    .context
                    .permits_with_grant(class, filters.max_sensitivity)
                {
                    permitted.insert(e.chunk_id);
                } else {
                    withheld += 1;
                }
            }
            // No detail row: the candidate renders nowhere
            // (the flat/grouped paths already skip it), and it
            // is not a disclosure decision, so it is not
            // counted as withheld.
            None => {}
        }
    }
    if withheld > 0 {
        notes.push(format!(
            "disclosure: context {} withheld {} candidate(s) (context ceiling {}, caller ceiling {})",
            opts.context.name(),
            withheld,
            opts.context.ceiling().name(),
            filters.max_sensitivity.name(),
        ));
    }
    // Full-content payloads (the parent message, hydration)
    // carry text beyond the classified chunk itself; a public
    // surface cannot safely filter them at chunk granularity,
    // so they are withheld there outright (design section 4:
    // "if an output cannot be safely filtered, withhold it").
    let withhold_full_content = opts.context == DisclosureContext::PublicSurface
        && (opts.with_message || opts.parameters.hydrate > 0);
    if withhold_full_content {
        notes.push(
            "disclosure: context public-surface withholds full-content message/hydration payloads"
                .to_string(),
        );
    }

    let mut out = if opts.parameters.raw_mode == RawMode::Branches {
        // Pre-fusion branch rows, exactly as the branches
        // produced them (design s3.3 --raw=branches): no fusion,
        // no grouping, no detail fetch beyond the disclosure
        // pass above.
        let mut rows = Vec::new();
        for (branch, hits) in [("fts", &pools.fts), ("vector", &pools.vector)] {
            for (i, hit) in hits.iter().enumerate() {
                if !permitted.contains(&hit.chunk_id) {
                    continue;
                }
                let mut row = Map::new();
                row.insert("chunk_id".into(), json!(hit.chunk_id));
                row.insert("branch".into(), json!(branch));
                row.insert("rank".into(), json!(i + 1));
                if let Some(dist) = hit.dist {
                    row.insert("vector_distance".into(), json!(round6(dist)));
                }
                rows.push(Json::Object(row));
            }
        }
        let mut env = envelope(backend, opts, filters, Json::Array(rows), &notes);
        env.as_object_mut()
            .expect("envelope is an object")
            .insert("raw".into(), json!("branches"));
        env
    } else if opts.group_by_session && opts.parameters.raw_mode == RawMode::Off {
        grouped_output(
            backend, opts, filters, &ranked, &vec_dist, &permitted, &notes,
        )?
    } else {
        // Take the top-k *permitted* candidates: a withheld
        // candidate must not shrink the page (the same
        // filter-before-serve rule the branch queries apply
        // to the caller ceiling).
        let mut results = Vec::new();
        for e in &ranked {
            if results.len() >= opts.k {
                break;
            }
            if !permitted.contains(&e.chunk_id) {
                continue;
            }
            let Some(d) = all_details.get(&e.chunk_id) else {
                continue;
            };
            let mut entry = result_entry(backend, opts, filters.max_sensitivity, e, d, &vec_dist)?;
            // Hydration: the first `hydrate` RENDERED results
            // (details present) gain full content — except on a
            // public surface, where full-content payloads are
            // withheld outright (see the disclosure pass above).
            if !withhold_full_content && results.len() < opts.parameters.hydrate {
                let hydrated = hydrate_entry(backend, d)?;
                entry
                    .as_object_mut()
                    .expect("result entry is an object")
                    .insert("hydrated".into(), hydrated);
            }
            results.push(entry);
        }
        let mut env = envelope(backend, opts, filters, Json::Array(results), &notes);
        if opts.parameters.raw_mode == RawMode::Ungrouped {
            env.as_object_mut()
                .expect("envelope is an object")
                .insert("raw".into(), json!("ungrouped"));
        }
        env
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
    if let (Some(job), Some((instant, started_ms))) = (trace, trace_started) {
        // Trace capture (issue #9973): resolve details for EVERY
        // pool entrant (not only the served top-k), build the
        // record, and persist it before the response is released.
        // Fail-closed: any failure here fails the search. The
        // disclosure pass above already resolved every ranked
        // candidate's details; withheld candidates appear in
        // the record as considered-but-not-served.
        let details = &all_details;
        let mut served: Vec<i64> = Vec::new();
        collect_served(out.get("results"), &mut served);
        let record = build_trace_record(
            &job.query_id,
            &job.ctx,
            started_ms,
            instant.elapsed().as_millis() as u64,
            opts,
            filters,
            backend.name(),
            &pools,
            &ranked,
            &vec_dist,
            details,
            &served,
        );
        job.sink.write_trace(&record)?;
        out.as_object_mut()
            .expect("envelope is an object")
            .insert("query_id".into(), Json::String(job.query_id.clone()));
    }
    Ok(out)
}

/// Chunk ids in rendered order, walking the envelope's results
/// (flat entries carry `chunk_id` directly; grouped session
/// entries nest them) — the trace's served set.
fn collect_served(v: Option<&Json>, acc: &mut Vec<i64>) {
    match v {
        Some(Json::Array(items)) => {
            for it in items {
                collect_served(Some(it), acc);
            }
        }
        Some(Json::Object(map)) => {
            if let Some(id) = map.get("chunk_id").and_then(Json::as_i64) {
                if !acc.contains(&id) {
                    acc.push(id);
                }
            }
            for val in map.values() {
                if val.is_array() {
                    collect_served(Some(val), acc);
                }
            }
        }
        _ => {}
    }
}

/// One hydrated payload (#9942): the full chunk text plus the
/// full, UNCAPPED message text from the backend's content store.
/// This is the explicit full-content path -- unlike
/// --with-message's display payload, nothing is truncated. (On
/// the shard path the tier content stores hydrate separately,
/// #6664; this covers the monolith backends, whose message store
/// IS the content store.)
fn hydrate_entry(backend: &dyn SearchBackend, d: &ChunkDetail) -> Result<Json> {
    let message = match backend.message(&d.message_id)? {
        Some(m) => json!({
            "id": m.id,
            "role": m.role,
            "text": m.text,
            "truncated": false,
        }),
        None => Json::Null,
    };
    Ok(json!({
        "chunk_text": d.text,
        "message": message,
    }))
}

/// The Python's `run_get_chunk`: a by-id lookup with NO caller
/// sensitivity ceiling (an explicit-id tool, like the Python).
/// The local-owner disclosure context still applies (#12404):
/// its ceiling is `secret-bearing`, so the only content this
/// entry point withholds that the Python served is
/// `excluded_personal_life` content, ineligible in every
/// context (design section 4).
pub fn run_get_chunk(backend: &dyn SearchBackend, chunk_id: i64, neighbors: usize) -> Result<Json> {
    run_get_chunk_with_context(backend, chunk_id, neighbors, DisclosureContext::LocalOwner)
}

/// `run_get_chunk` under an explicit disclosure context: the
/// chunk itself must be permitted by the context (a withheld
/// chunk reads exactly like a missing one — the lookup must not
/// confirm that classified content exists), and neighbor
/// expansion is filtered by each neighbor's own classification.
pub fn run_get_chunk_with_context(
    backend: &dyn SearchBackend,
    chunk_id: i64,
    neighbors: usize,
    context: DisclosureContext,
) -> Result<Json> {
    let details = backend.chunk_details(&[chunk_id])?;
    let Some(d) = details.iter().find(|d| d.chunk_id == chunk_id) else {
        return Err(Error::fatal(format!("chunk {chunk_id} not found")));
    };
    if !context.permits(chunk_classification(d.eff_sensitivity.as_deref())) {
        return Err(Error::fatal(format!("chunk {chunk_id} not found")));
    }
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
        out.insert(
            "context".into(),
            neighbor_map(backend, d, neighbors, context, context.ceiling())?,
        );
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
    grant: Sensitivity,
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
    entry_chunk_extras(backend, opts, grant, e, d, vec_dist, &mut entry)?;
    Ok(Json::Object(entry))
}

/// vector_distance / context / message — the trailing fields
/// shared by flat result entries and grouped chunk entries.
fn entry_chunk_extras(
    backend: &dyn SearchBackend,
    opts: &SearchOptions,
    grant: Sensitivity,
    e: &FusedEntry,
    d: &ChunkDetail,
    vec_dist: &HashMap<i64, Option<f64>>,
    entry: &mut Map<String, Json>,
) -> Result<()> {
    if let Some(Some(dist)) = vec_dist.get(&e.chunk_id) {
        entry.insert("vector_distance".into(), json!(round6(*dist)));
    }
    if opts.expand > 0 {
        entry.insert(
            "context".into(),
            neighbor_map(backend, d, opts.expand, opts.context, grant)?,
        );
    }
    // The parent message is a full-content payload: it carries
    // text beyond the classified chunk, so on a public surface
    // it is withheld outright (design section 4). Other
    // contexts gate it on the served chunk's own classification
    // via the disclosure pass that admitted the chunk.
    if opts.with_message && opts.context != DisclosureContext::PublicSurface {
        entry.insert("message".into(), message_obj(backend, d)?);
    }
    Ok(())
}

/// The Python's `neighbor_map`, with disclosure filtering
/// (#12404): neighbor rows come from a raw adjacency query that
/// carries no classification, so each neighbor's own detail row
/// decides whether its text may cross this boundary. A neighbor
/// with no detail row is withheld (fail-closed).
fn neighbor_map(
    backend: &dyn SearchBackend,
    d: &ChunkDetail,
    n: usize,
    context: DisclosureContext,
    grant: Sensitivity,
) -> Result<Json> {
    let n = i64::try_from(n)
        .map_err(|_| Error::fatal("neighbor count does not fit in the backend's signed range"))?;
    let rows = backend.neighbors(&d.message_id, d.chunk_idx, n)?;
    let ids: Vec<i64> = rows.iter().map(|r| r.chunk_id).collect();
    let details = details_map(backend, &ids)?;
    let mut before = Vec::new();
    let mut after = Vec::new();
    for r in &rows {
        let permitted = details.get(&r.chunk_id).is_some_and(|nd| {
            context.permits_with_grant(chunk_classification(nd.eff_sensitivity.as_deref()), grant)
        });
        if !permitted {
            continue;
        }
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
    permitted: &std::collections::HashSet<i64>,
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
        // Disclosure pass (#12404): a withheld candidate
        // contributes neither a chunk nor a hit_count.
        if !permitted.contains(&e.chunk_id) {
            continue;
        }
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
            entry_chunk_extras(
                backend,
                opts,
                filters.max_sensitivity,
                e,
                d,
                vec_dist,
                &mut ce,
            )?;
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

#[cfg(test)]
mod trace_tests {
    use super::*;
    use crate::trace::{JsonlTraceSink, TraceContext};
    use crate::types::{BranchHit, MessageRow, NeighborRow};
    use std::sync::Mutex;

    struct StubBackend;
    impl SearchBackend for StubBackend {
        fn name(&self) -> &'static str {
            "local"
        }
        fn vec_table(&self) -> &str {
            "vec_stub"
        }
        fn notes(&self) -> Vec<String> {
            Vec::new()
        }
        fn branch_pools(&self, _q: &BranchQuery, _f: &Filters) -> Result<BranchPools> {
            Ok(BranchPools {
                fts: vec![
                    BranchHit {
                        chunk_id: 1,
                        dist: None,
                    },
                    BranchHit {
                        chunk_id: 2,
                        dist: None,
                    },
                    BranchHit {
                        chunk_id: 3,
                        dist: None,
                    },
                ],
                vector: vec![BranchHit {
                    chunk_id: 2,
                    dist: Some(0.25),
                }],
                candidate_models: Default::default(),
            })
        }
        fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
            Ok(ids
                .iter()
                .map(|&id| ChunkDetail {
                    chunk_id: id,
                    text: format!("text {id}"),
                    chunk_idx: 0,
                    message_id: format!("m{id}"),
                    m_role: None,
                    session_id: "s1".into(),
                    s_title: None,
                    s_client: None,
                    s_created_at: None,
                    l_kind: None,
                    l_topics: None,
                    l_projects: None,
                    eff_sensitivity: Some("internal".into()),
                })
                .collect())
        }
        fn neighbors(&self, _m: &str, _i: i64, _n: i64) -> Result<Vec<NeighborRow>> {
            Ok(Vec::new())
        }
        fn message(&self, _m: &str) -> Result<Option<MessageRow>> {
            Ok(None)
        }
    }

    struct StubEmbedder;
    impl QueryEmbedder for StubEmbedder {
        fn embed_query(&self, _t: &str, _trunc: usize) -> Result<Vec<f32>> {
            Ok(vec![0.1, 0.2])
        }
    }

    #[derive(Default)]
    struct VecSink {
        records: Mutex<Vec<Json>>,
    }
    impl TraceSink for VecSink {
        fn write_trace(&self, record: &Json) -> Result<()> {
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    struct FailSink;
    impl TraceSink for FailSink {
        fn write_trace(&self, _record: &Json) -> Result<()> {
            Err(Error::backend("trace sink unavailable (test)"))
        }
    }

    /// The local operator's maximal grant: the stub chunks'
    /// non-ladder labels fail closed to secret-bearing under
    /// the disclosure pass, so the harness's ceiling must be
    /// secret-bearing for the chunks to be servable at all.
    fn local_filters() -> Filters {
        Filters::new(vec![], vec![], vec![], vec![], "secret-bearing", None, None).expect("filters")
    }

    fn fts_opts(k: usize) -> SearchOptions {
        SearchOptions {
            mode: SearchMode::Fts,
            k,
            // The harness models the local operator surface;
            // its stub labels are non-ladder values, which the
            // disclosure pass fail-closes to secret-bearing.
            context: DisclosureContext::LocalOwner,
            ..SearchOptions::new("backup database")
        }
    }

    #[test]
    fn traced_search_records_every_pool_entrant_and_returns_query_id() {
        let sink = VecSink::default();
        let ctx = TraceContext::new("cli", "user", "test");
        let out = run_search_traced(
            &StubBackend,
            &StubEmbedder,
            &local_filters(),
            &fts_opts(1),
            &sink,
            ctx,
        )
        .unwrap();
        let query_id = out
            .get("query_id")
            .and_then(Json::as_str)
            .expect("envelope carries query_id")
            .to_string();
        let records = sink.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        let rec = &records[0];
        assert_eq!(rec["schema"], "atlas-query-trace/v1");
        assert_eq!(rec["query_id"], query_id);
        assert_eq!(rec["retry_of"], Json::Null);
        assert_eq!(rec["pool_counts"]["merged"], 3);
        assert_eq!(rec["pool_counts"]["served"], 1);
        let results = rec["results"].as_array().unwrap();
        assert_eq!(results.len(), 3, "every pool entrant is traced");
        let served: Vec<&Json> = results
            .iter()
            .filter(|r| r["stages"]["final"]["served"] == true)
            .collect();
        assert_eq!(served.len(), 1);
        assert_eq!(served[0]["stages"]["final"]["rank"], 1);
        let unserved = results
            .iter()
            .find(|r| r["chunk_id"] == 3)
            .expect("chunk 3 entered the pool and was not served");
        assert_eq!(unserved["stages"]["final"]["served"], false);
        assert_eq!(unserved["stages"]["final"]["rank"], Json::Null);
        assert_eq!(unserved["stages"]["merge"]["rank_sources"]["fts"], 3);
        assert_eq!(unserved["stages"]["merge"]["rank_after_fusion"], 3);
        // Ranking behaviour unchanged: the untraced envelope is the
        // traced envelope minus query_id.
        let plain =
            run_search(&StubBackend, &StubEmbedder, &local_filters(), &fts_opts(1)).unwrap();
        let mut stripped = out.clone();
        stripped.as_object_mut().unwrap().remove("query_id");
        assert_eq!(stripped, plain);
    }

    #[test]
    fn trace_sink_failure_fails_the_search() {
        // Design invariant 2 (fail-closed): no silently untraced
        // success — the search errors when the trace cannot be
        // persisted.
        let err = run_search_traced(
            &StubBackend,
            &StubEmbedder,
            &local_filters(),
            &fts_opts(2),
            &FailSink,
            TraceContext::new("cli", "user", "test"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("trace sink unavailable"));
    }

    #[test]
    fn jsonl_sink_appends_one_line_per_record() {
        let dir =
            std::env::temp_dir().join(format!("atlas-trace-test-{}", crate::trace::new_query_id()));
        let sink = JsonlTraceSink::new(dir.clone());
        let rec = json!({"schema": "atlas-query-trace/v1", "ts": "2026-11-14T00:00:00Z"});
        sink.write_trace(&rec).unwrap();
        sink.write_trace(&rec).unwrap();
        let body = std::fs::read_to_string(dir.join("atlas-traces-2026-11-14.jsonl")).unwrap();
        assert_eq!(body.lines().count(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod jaccard_tests {
    use super::*;
    use crate::types::{BranchHit, MessageRow, NeighborRow};

    /// The pasted passage the query quotes verbatim (chunk 2's text);
    /// chunk 1 is merely topical and wins the fused ranking.
    const PASSAGE: &str =
        "error at line forty two the database disk image is malformed beyond repair";

    struct JaccardStub;
    impl SearchBackend for JaccardStub {
        fn name(&self) -> &'static str {
            "local"
        }
        fn vec_table(&self) -> &str {
            "vec_stub"
        }
        fn notes(&self) -> Vec<String> {
            Vec::new()
        }
        fn branch_pools(&self, _q: &BranchQuery, _f: &Filters) -> Result<BranchPools> {
            Ok(BranchPools {
                fts: vec![
                    BranchHit {
                        chunk_id: 1,
                        dist: None,
                    },
                    BranchHit {
                        chunk_id: 2,
                        dist: None,
                    },
                    BranchHit {
                        chunk_id: 3,
                        dist: None,
                    },
                ],
                vector: vec![],
                candidate_models: Default::default(),
            })
        }
        fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
            let texts: std::collections::HashMap<i64, &str> = std::collections::HashMap::from([
                (
                    1,
                    "database backup restore repair disk image troubleshooting guide",
                ),
                (2, PASSAGE),
                (
                    3,
                    "unrelated notes about cooking rice and lentils for dinner",
                ),
            ]);
            Ok(ids
                .iter()
                .map(|&id| ChunkDetail {
                    chunk_id: id,
                    text: texts.get(&id).copied().unwrap_or("").to_string(),
                    chunk_idx: 0,
                    message_id: format!("m{id}"),
                    m_role: None,
                    session_id: "s1".into(),
                    s_title: None,
                    s_client: None,
                    s_created_at: None,
                    l_kind: None,
                    l_topics: None,
                    l_projects: None,
                    eff_sensitivity: Some("internal".into()),
                })
                .collect())
        }
        fn neighbors(&self, _m: &str, _i: i64, _n: i64) -> Result<Vec<NeighborRow>> {
            Ok(Vec::new())
        }
        fn message(&self, _m: &str) -> Result<Option<MessageRow>> {
            Ok(None)
        }
    }

    struct StubEmbedder;
    impl QueryEmbedder for StubEmbedder {
        fn embed_query(&self, _t: &str, _trunc: usize) -> Result<Vec<f32>> {
            Ok(vec![0.1, 0.2])
        }
    }

    /// As in trace_tests: maximal local grant, because the
    /// stub labels fail closed to secret-bearing.
    fn local_filters() -> Filters {
        Filters::new(vec![], vec![], vec![], vec![], "secret-bearing", None, None).expect("filters")
    }

    fn opts(depth: usize) -> SearchOptions {
        let mut o = SearchOptions {
            mode: SearchMode::Fts,
            k: 3,
            // Local operator surface, as in trace_tests above.
            context: DisclosureContext::LocalOwner,
            ..SearchOptions::new(PASSAGE)
        };
        o.parameters.jaccard_depth = depth;
        o
    }

    fn result_ids(out: &Json) -> Vec<i64> {
        out["results"]
            .as_array()
            .expect("results array")
            .iter()
            .map(|r| r["chunk_id"].as_i64().expect("chunk_id"))
            .collect()
    }

    #[test]
    fn rescore_promotes_the_pasted_passage_end_to_end() {
        // Off (the production default): fused order stands, the
        // topical chunk leads.
        let off = run_search(&JaccardStub, &StubEmbedder, &local_filters(), &opts(0)).unwrap();
        assert_eq!(result_ids(&off)[0], 1);

        // On: the near-exact tier promotes the quoted passage itself
        // to rank 1, and the envelope notes the rescore.
        let on = run_search(&JaccardStub, &StubEmbedder, &local_filters(), &opts(100)).unwrap();
        assert_eq!(result_ids(&on)[0], 2);
        let notes = on["notes"].to_string();
        assert!(notes.contains("jaccard rescore"), "{notes}");
        assert!(notes.contains("1 near-exact"), "{notes}");
    }
}
