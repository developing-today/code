//! atlas-search — the Atlas access layer: hybrid search over the corpus.
//!
//! The one-shot CLI (epic developing-today/code#5350, step 3,
//! #5353): a surface-parity port of the Python `atlas_search.py`'s
//! front-end over atlas-core (step 2) and atlas-embed (step 1).
//! Two surface divergences from the Python are intentional
//! (#13280): search's `--context` defaults to `fleet-internal`
//! (the Python defaults both verbs to `local-owner`), and
//! get-chunk has no `--max-sensitivity` — its ceiling arrives
//! via `--context` (the Python get-chunk takes
//! `--max-sensitivity`, default `secret-bearing`).
//! Standalone is the default and is complete — embed the query
//! in-process, talk to Turso or a local publish file directly.
//! `--via-service` prefers a running service (the step-4 job
//! protocol, #5354) and falls back to standalone loudly on any
//! service failure. `serve` runs that service; `init` generates
//! its client table.
//!
//! Exit behavior mirrors the Python: `die()` fatals print
//! `atlas-search: <msg>` on stderr and exit 2 (argparse errors
//! likewise exit 2 via clap); get-chunk's not-found exits 1, as do
//! backend/transport failures (Python tracebacks, also exit 1 —
//! the binary prints the message instead of the traceback).

mod fetch_model;
mod pyjson;
mod render;
mod server;
mod service;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::OnceLock;

use atlas_core::filters::{Filters, split_csv};
use atlas_core::shards::{ShardedBackend, TierCeiling, open_from_spec};
use atlas_core::turso::{TURSO_HOST, resolve_turso_token};
use atlas_core::{
    AtlasEmbedder, CachingEmbedder, DisclosureContext, Error as CoreError, FeedbackInput,
    FixedEmbedder, JsonlFeedbackSink, JsonlTraceSink, LocalBackend, QueryEmbedder,
    QueryEmbeddingCache, SearchBackend, SearchMode, SearchOptions, TraceContext, TraceDocument,
    TursoBackend, VectorBackend, Vote, aggregate_traces, feedback_for, ingest_traces,
    load_feedback_dir, load_feedback_refs, load_feedback_votes, load_query_feedback_votes,
    load_query_requesters, normalise_records, record_feedback, run_get_chunk_with_context,
    run_retention, run_search_traced, search_traces, valid_civil_date, weekly_report,
    write_normalised, write_report,
};
use atlas_proto::{
    FilterSpec, QueryVariant, SearchMode as ProtoSearchMode, SearchParametersV1, SearchRequest,
    Sensitivity as ProtoSensitivity, VectorBackend as ProtoVectorBackend,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::Value as Json;

// ---------------------------------------------------------------- args --

#[derive(Parser)]
#[command(
    name = "atlas-search",
    about = "atlas-search — the Atlas access layer: hybrid search over the corpus."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// hybrid FTS+vector search with classification filters
    Search(Box<SearchArgs>),
    /// one chunk with provenance
    GetChunk(GetChunkArgs),
    /// run the Atlas service (loopback HTTP job protocol; step 4)
    Serve(ServeArgs),
    /// generate the service config + v1 client table (0600)
    Init(InitArgs),
    /// download + verify the pinned Snowflake model into the state dir (step 5)
    FetchModel,
    /// ingest query traces as a statistics-and-reports corpus, or run trace retention (design s4)
    Traces(TracesArgs),
    /// cast a thumbs vote on a served result, or retrieve feedback by its IDs (design s6.1); a vote requires exactly one of --up / --down — use --get to retrieve instead
    Feedback(FeedbackArgs),
}

#[derive(Args)]
struct FeedbackArgs {
    /// the query_id the search response envelope carried (invariant 1)
    #[arg(long)]
    query: String,
    /// the result's chunk_id; omit for a per-query vote (design s6.1)
    #[arg(long)]
    chunk: Option<i64>,
    /// thumbs up (conflicts with --down)
    #[arg(long, conflicts_with = "down")]
    up: bool,
    /// thumbs down (conflicts with --up)
    #[arg(long)]
    down: bool,
    /// optional comment, verbatim; @file reads the comment from a file
    #[arg(long)]
    comment: Option<String>,
    /// the result was opened/expanded before this vote (design s6.4)
    #[arg(long)]
    opened: bool,
    /// requester kind: drew | agent | user | anonymous (design s6.3)
    #[arg(long, default_value = "user")]
    requester_kind: String,
    /// stable requester id
    #[arg(long, default_value = "cli")]
    requester_id: String,
    /// retrieve the feedback for --query [--chunk] instead of casting a vote (with --get, neither --up nor --down is needed; without it, exactly one of them is required)
    #[arg(long)]
    get: bool,
    /// feedback store directory (default: the JsonlFeedbackSink default dir)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// go through a running service instead of the local store
    #[arg(long)]
    via_service: bool,
    /// service base URL for --via-service (env ATLAS_SEARCH_URL)
    #[arg(long, env = "ATLAS_SEARCH_URL")]
    service_url: Option<String>,
}

#[derive(Args)]
struct TracesArgs {
    #[command(subcommand)]
    cmd: TracesCmd,
}

#[derive(Subcommand)]
enum TracesCmd {
    /// ingest trace JSONL into the statistics-and-reports corpus; --query searches it
    Ingest(TraceIngestArgs),
    /// run the 180-day hot retention pass (cold-tier move + compaction aggregate)
    Retention(TraceRetentionArgs),
    /// daily batch aggregation over stored traces (design s5)
    Stats(TraceStatsArgs),
    /// weekly statistics report into the statistics-and-reports source set (design s7); also writes the report JSONL and a Markdown rendering into the traces dir and reports both paths as report_file / markdown_file
    Report(TraceReportArgs),
    /// agent normalisation pass over the raw feedback store (design s6.2)
    Normalise(TraceNormaliseArgs),
}

#[derive(Args)]
struct TraceIngestArgs {
    /// traces directory (default: the JsonlTraceSink default dir)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// only print documents matching this corpus query
    #[arg(long)]
    query: Option<String>,
}

#[derive(Args)]
struct TraceRetentionArgs {
    /// traces directory (default: the JsonlTraceSink default dir)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// today as YYYY-MM-DD (default: today, UTC)
    #[arg(long)]
    today: Option<String>,
    /// feedback JSONL file naming referenced query ids (repeatable)
    #[arg(long)]
    feedback: Vec<PathBuf>,
}

#[derive(Args)]
struct TraceStatsArgs {
    /// traces directory (default: the JsonlTraceSink default dir)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// window end as YYYY-MM-DD (default: today, UTC)
    #[arg(long)]
    today: Option<String>,
    /// rolling window length in days (default: 7)
    #[arg(long, default_value_t = 7)]
    window_days: i64,
    /// feedback JSONL file with vote records (repeatable)
    #[arg(long)]
    feedback: Vec<PathBuf>,
}

#[derive(Args)]
struct TraceReportArgs {
    /// traces directory (default: the JsonlTraceSink default dir)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// report day as YYYY-MM-DD (default: today, UTC)
    #[arg(long)]
    today: Option<String>,
    /// feedback JSONL file with vote records (repeatable)
    #[arg(long)]
    feedback: Vec<PathBuf>,
}

#[derive(Args)]
struct TraceNormaliseArgs {
    /// traces directory, for query requester identities (default: the JsonlTraceSink default dir)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// feedback store directory (default: the JsonlFeedbackSink default dir)
    #[arg(long)]
    feedback_dir: Option<PathBuf>,
    /// derivative output directory (default: <feedback store>/normalised)
    #[arg(long)]
    out_dir: Option<PathBuf>,
    /// derivative day as YYYY-MM-DD (default: today, UTC)
    #[arg(long)]
    today: Option<String>,
}

#[derive(Args)]
struct ServeArgs {
    /// service config path (default: ~/.local/state/atlas-search/service.json; env ATLAS_SEARCH_SERVICE_CONFIG)
    #[arg(long, env = "ATLAS_SEARCH_SERVICE_CONFIG")]
    config: Option<PathBuf>,
    /// listen address override (loopback only)
    #[arg(long)]
    listen: Option<SocketAddr>,
    /// completed-job TTL override, milliseconds (default 600000)
    #[arg(long)]
    job_ttl_ms: Option<u64>,
    /// submit hard-cap override, milliseconds (default 3600000)
    #[arg(long)]
    hard_ttl_ms: Option<u64>,
}

#[derive(Args)]
struct InitArgs {
    /// service config path (default: ~/.local/state/atlas-search/service.json; env ATLAS_SEARCH_SERVICE_CONFIG)
    #[arg(long, env = "ATLAS_SEARCH_SERVICE_CONFIG")]
    config: Option<PathBuf>,
    /// replace an existing config (rotates every client token)
    #[arg(long)]
    force: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum BackendChoice {
    Auto,
    Turso,
    Local,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ModeChoice {
    Hybrid,
    Fts,
    Vector,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
enum VariantChoice {
    #[default]
    Raw,
    Rewritten,
    Both,
}

fn parameter_defaults() -> SearchParametersV1 {
    SearchParametersV1::default()
}

fn default_parameters_version() -> i64 {
    i64::from(parameter_defaults().parameters_version)
}

fn default_shard_top_k() -> i64 {
    parameter_defaults().shard_top_k as i64
}

fn default_global_top_k() -> i64 {
    parameter_defaults().global_top_k as i64
}

fn default_rrf_k() -> i64 {
    parameter_defaults().rrf_k as i64
}

fn default_variant_choice() -> VariantChoice {
    match parameter_defaults().variant {
        QueryVariant::Raw => VariantChoice::Raw,
        QueryVariant::Rewritten => VariantChoice::Rewritten,
        QueryVariant::Both => VariantChoice::Both,
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum EmbedderChoice {
    Auto,
    Fastembed,
    St,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum VectorBackendChoice {
    Scan,
    IndexFill,
    IndexFixed,
}

/// Backend selection + via-service routing, shared by both
/// subcommands (the Python's `add_backend_args` + step 3's
/// service routing).
#[derive(Args, Clone)]
struct BackendArgs {
    /// data backend (auto: turso when a token resolves, else local)
    #[arg(long, value_enum, default_value_t = BackendChoice::Auto)]
    backend: BackendChoice,
    /// local backend: SQLite DB path (default: canonical atlas.db)
    #[arg(long)]
    db: Option<String>,
    /// local backend: comma-separated vector DB path(s) if the corpus DB has no vec table
    #[arg(long)]
    vecdb: Option<String>,
    /// prefer a running atlas service; fall back to standalone, loudly.
    /// The service route serves only the Turso backend with the
    /// production vector table: --backend local or a non-default
    /// --vec-table fall back with a note, and any --shards spec is
    /// served standalone without attempting the service
    #[arg(long)]
    via_service: bool,
    /// service base URL for --via-service (env ATLAS_SEARCH_URL)
    #[arg(long, env = "ATLAS_SEARCH_URL")]
    service_url: Option<String>,
}

/// The Python's `add_vector_args` (search only).
#[derive(Args, Clone)]
struct VectorArgs {
    /// embedding model (default Snowflake/snowflake-arctic-embed-m-v2.0; env ATLAS_SEARCH_MODEL)
    #[arg(long, env = "ATLAS_SEARCH_MODEL", default_value = atlas_core::DEFAULT_MODEL)]
    model: String,
    /// vector table name (default vec_snowflake_m_v2_768_f32; env ATLAS_VEC_TABLE)
    #[arg(long, env = "ATLAS_VEC_TABLE", default_value = atlas_core::DEFAULT_VEC_TABLE)]
    vec_table: String,
    /// query embedder: fastembed (bge) or st = sentence-transformers with prompt_name='query' (Snowflake); auto picks by model name
    #[arg(long, value_enum, default_value_t = EmbedderChoice::Auto)]
    embedder: EmbedderChoice,
    /// truncate query+doc vectors to N dims (Matryoshka; 0 = full)
    #[arg(long, default_value_t = 0)]
    trunc: i64,
    /// turso vector branch: scan = exact vector_distance_cos over the filtered set; index-fixed = one vector_top_k lookup at pool depth, filtered afterwards; index-fill = vector_top_k with escalating depth until the pool fills after filtering (cap 3200)
    #[arg(long, value_enum, default_value_t = VectorBackendChoice::Scan)]
    vector_backend: VectorBackendChoice,
}

#[derive(Args)]
struct SearchArgs {
    query: String,
    /// final results (default 10)
    #[arg(long, default_value_t = SearchParametersV1::default_k() as i64)]
    k: i64,
    /// per-branch candidates (default 100)
    #[arg(long, default_value_t = SearchParametersV1::default_pool() as i64)]
    pool: i64,
    #[arg(long, value_enum, default_value_t = ModeChoice::Hybrid)]
    mode: ModeChoice,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    topic: Option<String>,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    client: Option<String>,
    /// YYYY-MM-DD, on sessions.created_at (inclusive)
    #[arg(long)]
    since: Option<String>,
    /// YYYY-MM-DD, on sessions.created_at (inclusive)
    #[arg(long)]
    until: Option<String>,
    /// ceiling in shareable<personal<local-only<secret-bearing, or the canonical public<internal<private<sensitive (default personal; the top rung must be spelled out)
    #[arg(long, default_value = "personal")]
    max_sensitivity: String,
    /// disclosure context of the destination surface (#12404): fleet-internal (default; this CLI serves fleet surfaces), hosted-per-user, public-surface, or local-owner. The context ceiling intersects --max-sensitivity; neither widens the other, and excluded content is never served
    #[arg(long, default_value = "fleet-internal")]
    context: String,
    /// roll chunk hits up to sessions (score = best chunk, plus hit count)
    #[arg(long)]
    group_by_session: bool,
    /// include N neighboring chunks (same message) around each hit
    #[arg(long, default_value_t = SearchParametersV1::default_expand() as i64)]
    expand: i64,
    /// version of the shared query-parameter contract (currently 1)
    #[arg(long = "parameters_version", default_value_t = default_parameters_version())]
    parameters_version: i64,
    /// candidates scanned per shard (default 100)
    #[arg(long = "shard_top_k", default_value_t = default_shard_top_k())]
    shard_top_k: i64,
    /// candidates retained after shard fan-in (default 100)
    #[arg(long = "global_top_k", default_value_t = default_global_top_k())]
    global_top_k: i64,
    /// reciprocal-rank-fusion multiplier for FTS (default 1)
    #[arg(long = "fts_weight", default_value_t = parameter_defaults().fts_weight)]
    fts_weight: f64,
    /// reciprocal-rank-fusion multiplier for vectors (default 1)
    #[arg(long = "vector_weight", default_value_t = parameter_defaults().vector_weight)]
    vector_weight: f64,
    /// candidate depth for Jaccard rescoring over the fused union (0 = off; #5733)
    #[arg(long = "jaccard_depth", default_value_t = parameter_defaults().jaccard_depth as i64)]
    jaccard_depth: i64,
    /// near-exact Jaccard threshold in (0, 1] (0 = the design default 0.60; #5733)
    #[arg(long = "jaccard_threshold", default_value_t = parameter_defaults().jaccard_threshold)]
    jaccard_threshold: f64,
    /// reciprocal-rank-fusion constant (default 10)
    #[arg(long = "rrf_k", default_value_t = default_rrf_k())]
    rrf_k: i64,
    /// enable query planner/rewriter (not implemented yet)
    #[arg(long = "planner_enabled")]
    planner_enabled: bool,
    /// enable spell correction (not implemented yet)
    #[arg(long = "spell_enabled")]
    spell_enabled: bool,
    /// query-class hint (not implemented yet)
    #[arg(long = "query_class_hint")]
    query_class_hint: Option<String>,
    /// query variant: raw (supported), rewritten or both (unsupported)
    #[arg(long, value_enum, default_value_t = default_variant_choice())]
    variant: VariantChoice,
    /// use the verbatim query with planner/spell and expansion disabled
    #[arg(long)]
    raw: bool,
    /// include the parent message text (truncated at 50000 chars)
    #[arg(long)]
    with_message: bool,
    /// bypass the service cache reads; the fresh result still lands in the cache (refresh). Service mode only: the local path has no result cache (#9942)
    #[arg(long)]
    no_cache: bool,
    /// block for the full fused result (the one-shot CLI implies --wait; this spells it out)
    #[arg(long, conflicts_with = "no_wait")]
    wait: bool,
    /// do not block: print the service job handle plus an empty first page and return
    #[arg(long)]
    no_wait: bool,
    /// hydrate the top N results to full chunk + message content from the content store (NOT --expand: that keeps its neighbor-chunks meaning)
    #[arg(long, default_value_t = 0)]
    hydrate: i64,
    /// ungrouped output rows: bare --raw-rows for flat fused rows, --raw-rows=branches for pre-fusion branch rows. Distinct from --raw (verbatim query), which is not repurposed
    #[arg(long, num_args = 0..=1, default_missing_value = "ungrouped", value_parser = ["ungrouped", "branches"])]
    raw_rows: Option<String>,
    /// human-readable output instead of JSON
    #[arg(long)]
    text: bool,
    /// EXPERIMENTAL, off by default: serve the vector branch from the atlas core shard fan-out (dir:<path> or turso[:<org>]); FTS, details and --expand stay on the backend (issue #5755). Skips --via-service.
    #[arg(long, env = "ATLAS_SHARDS")]
    shards: Option<String>,
    /// path to the catalog/source-registry database for occurrence attribution (#3125 slice 7): with --shards, occurrence rows resolve their source locators to registered roots (root_id). Requires --shards.
    #[arg(long = "source-registry", env = "ATLAS_SOURCE_REGISTRY")]
    source_registry: Option<PathBuf>,
    #[command(flatten)]
    backend: BackendArgs,
    #[command(flatten)]
    vector: VectorArgs,
}

#[derive(Args)]
struct GetChunkArgs {
    chunk_id: i64,
    #[arg(long, default_value_t = 0)]
    neighbors: i64,
    /// disclosure context of the destination surface (#12404): local-owner (default; this is the owner's explicit-id tool), fleet-internal, hosted-per-user, or public-surface
    #[arg(long, default_value = "local-owner")]
    context: String,
    /// human-readable output instead of JSON
    #[arg(long)]
    text: bool,
    #[command(flatten)]
    backend: BackendArgs,
}

// -------------------------------------------------------------- errors --

/// (exit code, message) — main prints `atlas-search: <message>`.
type CliError = (u8, String);

fn map_err(e: CoreError) -> CliError {
    match e {
        CoreError::Fatal(m) => (2, m),
        CoreError::Backend(m) | CoreError::Embed(m) => (1, m),
    }
}

/// Resolve a `--context` value (#12404). An unknown context is
/// a usage error (exit 2), never a silent default: the caller
/// stated a destination, and guessing at it could widen
/// disclosure. (The wire protocol's absent/unknown rule —
/// fail closed to public-surface — applies to requests that
/// arrive without a stated context, not to a CLI flag the user
/// typed.)
fn parse_context(value: &str) -> Result<DisclosureContext, CliError> {
    DisclosureContext::from_name(value).ok_or_else(|| {
        (
            2,
            format!(
                "invalid --context {value:?}: expected one of {}",
                DisclosureContext::all()
                    .iter()
                    .map(|c| c.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    })
}

/// get-chunk's not-found is the Python's one `die(..., code=1)`.
fn map_get_chunk_err(e: CoreError, chunk_id: i64) -> CliError {
    if let CoreError::Fatal(m) = &e
        && *m == format!("chunk {chunk_id} not found")
    {
        return (1, m.clone());
    }
    map_err(e)
}

// ------------------------------------------------------------ backends --

fn home_dir() -> PathBuf {
    // Home-resolution contract (issues #5640 / #5762): a missing or
    // empty HOME is an error naming HOME, never a substitute
    // directory - the old fallback to a literal "~" silently
    // resolved state/config/token paths relative to the CWD.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => panic!(
            "HOME is not set or is empty: atlas-search requires a non-empty HOME to resolve its state/config paths (issue #5640)"
        ),
    }
}

/// The Python's `default_db_path`.
fn default_db_path() -> String {
    if let Ok(env) = std::env::var("ATLAS_DB")
        && !env.is_empty()
    {
        return env;
    }
    let laptop = home_dir().join("atlas/turso-atlas/data/atlas.db");
    if laptop.exists() {
        return laptop.to_string_lossy().into_owned();
    }
    // The Python's script-relative ../data/atlas.db; the binary's
    // analogue is its own directory's ../data.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent().and_then(|p| p.parent())
    {
        let cand = dir.join("data/atlas.db");
        if cand.exists() {
            return cand.to_string_lossy().into_owned();
        }
    }
    laptop.to_string_lossy().into_owned()
}

/// The Python's `pick_backend`: explicit turso/local, else turso
/// when a token resolves through the standard chain, else local.
fn pick_backend(
    args: &BackendArgs,
    vec_table: &str,
    trunc: usize,
) -> Result<Box<dyn SearchBackend>, CliError> {
    let local = |args: &BackendArgs| -> Result<Box<dyn SearchBackend>, CliError> {
        let db = args.db.clone().unwrap_or_else(default_db_path);
        LocalBackend::new(&db, args.vecdb.as_deref(), vec_table, trunc)
            .map(|b| Box::new(b) as Box<dyn SearchBackend>)
            .map_err(map_err)
    };
    match args.backend {
        BackendChoice::Turso => TursoBackend::new(vec_table)
            .map(|b| Box::new(b) as Box<dyn SearchBackend>)
            .map_err(map_err),
        BackendChoice::Local => local(args),
        BackendChoice::Auto => match resolve_turso_token() {
            Ok((token, _source)) => Ok(Box::new(TursoBackend::with_token(
                TURSO_HOST, token, vec_table,
            ))),
            Err(_) => local(args),
        },
    }
}

// ------------------------------------------------------------ embedder --

fn resolve_model_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("ATLAS_MODEL_DIR")
        && !d.is_empty()
    {
        let p = PathBuf::from(&d);
        return if p.is_dir() { Some(p) } else { None };
    }
    let home = home_dir();
    let state = home.join(format!(
        ".local/state/atlas-search/model/{}",
        fetch_model::MODEL_REV
    ));
    if state.is_dir() {
        return Some(state);
    }
    let hf = home.join(format!(
        ".cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/{}",
        fetch_model::MODEL_REV
    ));
    if hf.is_dir() {
        return Some(hf);
    }
    None
}

fn resolve_ort_lib() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ATLAS_ORT_LIB")
        && !p.is_empty()
    {
        let pb = PathBuf::from(&p);
        return if pb.is_file() { Some(pb) } else { None };
    }
    let cand = home_dir().join(".local/state/atlas-search/ort/libonnxruntime.so");
    if cand.is_file() {
        return Some(cand);
    }
    // The packaged binary's closure path, baked in at build time
    // (step 5, #5355): pkgs/atlas-search/default.nix compiles with
    // ATLAS_SEARCH_ORT_LIB_DEFAULT set to its onnxruntime store
    // path, so the packaged build resolves the runtime from its
    // own closure — no env, no state-dir staging. Dev builds do
    // not set it and never reach this branch.
    if let Some(p) = option_env!("ATLAS_SEARCH_ORT_LIB_DEFAULT") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    None
}

enum LoadedEmbedder {
    Real(Box<CachingEmbedder<AtlasEmbedder>>),
    Fixed(FixedEmbedder),
}

/// The CLI's lazy query embedder: loads the ONNX session on first
/// use only, so FTS-only searches and get-chunk never pay for (or
/// fail on) a missing model — the Python's degradation shape.
///
/// Test hook: `ATLAS_SEARCH_QVEC_FILE` (a JSON array of floats)
/// substitutes a fixed query vector — the same monkeypatch the
/// Python fixture generator applies to `embed_query`; used by the
/// CLI parity tests, meaningless in production.
struct CliEmbedder {
    cell: OnceLock<Result<LoadedEmbedder, String>>,
    model: String,
    embedder: EmbedderChoice,
}

impl CliEmbedder {
    fn new(model: String, embedder: EmbedderChoice) -> CliEmbedder {
        CliEmbedder {
            cell: OnceLock::new(),
            model,
            embedder,
        }
    }

    fn load(&self) -> Result<&LoadedEmbedder, CoreError> {
        self.cell
            .get_or_init(|| {
                if let Ok(f) = std::env::var("ATLAS_SEARCH_QVEC_FILE")
                    && !f.is_empty()
                {
                    let text = std::fs::read_to_string(&f)
                        .map_err(|e| format!("ATLAS_SEARCH_QVEC_FILE: cannot read {f}: {e}"))?;
                    let raw: Vec<f32> = serde_json::from_str::<Vec<f64>>(&text)
                        .map(|v| v.into_iter().map(|x| x as f32).collect())
                        .map_err(|e| {
                            format!("ATLAS_SEARCH_QVEC_FILE: bad JSON vector in {f}: {e}")
                        })?;
                    return Ok(LoadedEmbedder::Fixed(FixedEmbedder { raw }));
                }
                load_embedder(&self.model, self.embedder).map(|e| {
                    let cache = QueryEmbeddingCache::from_env(&self.model, fetch_model::MODEL_REV);
                    LoadedEmbedder::Real(Box::new(CachingEmbedder::new(
                        e,
                        std::sync::Arc::new(cache),
                    )))
                })
            })
            .as_ref()
            .map_err(|m| CoreError::fatal(m.clone()))
    }
}

impl QueryEmbedder for CliEmbedder {
    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>, CoreError> {
        match self.load()? {
            LoadedEmbedder::Real(e) => e.embed_query(text, trunc),
            LoadedEmbedder::Fixed(e) => e.embed_query(text, trunc),
        }
    }
}

/// Build the production embedder, with the Python's loud-failure
/// shape for the combinations it also refuses (an embedder it
/// cannot construct is a die(), never a silent downgrade).
fn load_embedder(model: &str, embedder: EmbedderChoice) -> Result<AtlasEmbedder, String> {
    if embedder == EmbedderChoice::Fastembed {
        return Err(
            "vector/hybrid modes with a fastembed embedder are not available in this \
             binary (the bge/fastembed fallback lives in the Python CLI); use --mode fts \
             or the Python atlas-search"
                .to_string(),
        );
    }
    if model != atlas_core::DEFAULT_MODEL {
        return Err(format!(
            "vector/hybrid modes: this binary embeds only {} (atlas-embed); \
             model {model} needs the Python CLI",
            atlas_core::DEFAULT_MODEL
        ));
    }
    let model_dir = resolve_model_dir().ok_or_else(|| {
        "Snowflake model not found (looked in ATLAS_MODEL_DIR, \
         ~/.local/state/atlas-search/model/<rev>, and the HF cache snapshot); \
         FTS-only search and get-chunk work without it"
            .to_string()
    })?;
    let ort_lib = resolve_ort_lib().ok_or_else(|| {
        "ONNX Runtime library not found: set ATLAS_ORT_LIB (the packaged \
         binary resolves it from its nix closure — step 5, #5355); \
         FTS-only search and get-chunk work without it"
            .to_string()
    })?;
    let emb = atlas_embed::Embedder::load(&model_dir, &ort_lib)
        .map_err(|e| format!("embedder load failed: {e}"))?;
    Ok(AtlasEmbedder::new(emb))
}

// ------------------------------------------------------------- service --

fn via_requested(args: &BackendArgs) -> bool {
    args.via_service || service::via_env()
}

/// The shipped fallback contract: warn loudly on stderr and
/// record the same fact in the envelope's notes.
fn via_fallback_note(detail: String) -> String {
    eprintln!("atlas-search: warning: --via-service: {detail}; falling back to standalone");
    format!("via-service: {detail}; served standalone")
}

/// Translate the CLI's search surface into the wire request.
fn proto_search_request(args: &SearchArgs) -> SearchRequest {
    SearchRequest {
        query: args.query.clone(),
        mode: match args.mode {
            ModeChoice::Hybrid => ProtoSearchMode::Hybrid,
            ModeChoice::Fts => ProtoSearchMode::Fts,
            ModeChoice::Vector => ProtoSearchMode::Vector,
        },
        filters: FilterSpec {
            kind: split_csv(args.kind.as_deref()),
            topic: split_csv(args.topic.as_deref()),
            project: split_csv(args.project.as_deref()),
            client: split_csv(args.client.as_deref()),
            since: args.since.clone(),
            until: args.until.clone(),
            dimensions: Default::default(),
        },
        ceiling: ProtoSensitivity::from_ceiling_name(&args.max_sensitivity)
            .unwrap_or(ProtoSensitivity::Personal),
        k: u(args.k),
        pool: u(args.pool),
        group_by_session: args.group_by_session,
        expand: u(args.expand),
        parameters: parameters_from_args(args),
        with_message: args.with_message,
        model: args.vector.model.clone(),
        embedder: match args.vector.embedder {
            EmbedderChoice::Auto => "auto",
            EmbedderChoice::Fastembed => "fastembed",
            EmbedderChoice::St => "st",
        }
        .to_string(),
        trunc: u(args.vector.trunc),
        vector_backend: match args.vector.vector_backend {
            VectorBackendChoice::Scan => ProtoVectorBackend::Scan,
            VectorBackendChoice::IndexFill => ProtoVectorBackend::IndexFill,
            VectorBackendChoice::IndexFixed => ProtoVectorBackend::IndexFixed,
        },
        page_limit: None,
        context: Some(args.context.clone()),
    }
}

fn parameters_from_args(args: &SearchArgs) -> SearchParametersV1 {
    SearchParametersV1 {
        parameters_version: u(args.parameters_version) as u32,
        shard_top_k: u(args.shard_top_k),
        global_top_k: u(args.global_top_k),
        fts_weight: args.fts_weight,
        vector_weight: args.vector_weight,
        jaccard_depth: u(args.jaccard_depth),
        jaccard_threshold: args.jaccard_threshold,
        rrf_k: u(args.rrf_k),
        planner_enabled: args.planner_enabled,
        spell_enabled: args.spell_enabled,
        query_class_hint: args.query_class_hint.clone(),
        variant: match args.variant {
            VariantChoice::Raw => QueryVariant::Raw,
            VariantChoice::Rewritten => QueryVariant::Rewritten,
            VariantChoice::Both => QueryVariant::Both,
        },
        raw: args.raw,
        no_cache: args.no_cache,
        // The CLI one-shot implies --wait (design s3.3); only
        // --no-wait turns it off.
        wait: !args.no_wait,
        hydrate: u(args.hydrate),
        raw_mode: match args.raw_rows.as_deref() {
            Some("branches") => atlas_proto::RawMode::Branches,
            Some(_) => atlas_proto::RawMode::Ungrouped,
            None => atlas_proto::RawMode::Off,
        },
    }
}

/// Try the service for a search. `Ok(Some(envelope))` = served by
/// the service; `Ok(None)` = not applicable, caller serves
/// standalone silently; `Err(note)` = attempted and fell back,
/// with the envelope note to record.
fn try_via_service_search(args: &SearchArgs) -> Result<Option<Json>, String> {
    if !via_requested(&args.backend) {
        return Ok(None);
    }
    if ProtoSensitivity::from_ceiling_name(&args.max_sensitivity).is_none() {
        // Let the standalone path produce the Python's exact
        // validation fatal rather than silently clamping here.
        return Ok(None);
    }
    let base = service::base_url(args.backend.service_url.as_deref());
    if args.backend.backend == BackendChoice::Local {
        return Err(via_fallback_note(format!(
            "service at {base} serves the Turso backend only; a local backend was requested"
        )));
    }
    if args.vector.vec_table != atlas_core::DEFAULT_VEC_TABLE {
        return Err(via_fallback_note(format!(
            "service at {base} is configured for the production vector table {}; --vec-table {} needs standalone",
            atlas_core::DEFAULT_VEC_TABLE,
            args.vector.vec_table
        )));
    }
    match service::search(&base, &proto_search_request(args)) {
        Ok(out) => Ok(Some(out)),
        Err(detail) => Err(via_fallback_note(detail)),
    }
}

/// get-chunk has no via-service path in the job protocol (chunk
/// reads are scoped to an existing job), so it keeps the shipped
/// loud-fallback shape.
fn via_service_note_get_chunk(args: &BackendArgs) -> Option<String> {
    if !via_requested(args) {
        return None;
    }
    let base = service::base_url(args.service_url.as_deref());
    Some(via_fallback_note(service::get_chunk_fallback_detail(&base)))
}

fn push_note(out: &mut Json, note: String) {
    if let Some(obj) = out.as_object_mut() {
        match obj.get_mut("notes") {
            Some(Json::Array(notes)) => notes.push(Json::String(note)),
            _ => {
                obj.insert("notes".to_string(), Json::Array(vec![Json::String(note)]));
            }
        }
    }
}

// -------------------------------------------------------------- output --

fn print_output(out: &Json, text: bool) {
    if text {
        println!("{}", render::render_text(out));
    } else {
        println!("{}", pyjson::dumps_pretty(out));
    }
}

/// Unchecked signed->unsigned conversion for the proto request
/// translation, whose callers validate the counts first
/// ([`count`], in `cmd_search`). Never use this on raw CLI input:
/// it clamps negatives to 0, which is how #5407's silent empty
/// searches happened.
fn u(v: i64) -> usize {
    usize::try_from(v).unwrap_or(0)
}

/// Validate one CLI count flag against the service's request
/// contract (design section 2; `validate_request` in server.rs):
/// `--k` at least 1, `--pool` between 1 and 400, `--expand` between
/// 0 and `MAX_EXPAND_NEIGHBORS`, `--neighbors` also capped at the
/// service's limit, and the other zero-is-valid counts (`--trunc`)
/// non-negative. Out-of-range values are fatal (exit 2), never a
/// silent clamp to 0 (#5407).
fn count(flag: &str, v: i64, min: i64, max: Option<i64>) -> Result<usize, CliError> {
    if v >= min && max.is_none_or(|m| v <= m) {
        return Ok(usize::try_from(v).expect("range-checked count converts"));
    }
    if v < min && min == 0 {
        return Err((2, format!("{flag} must not be negative")));
    }
    let msg = match (min, max) {
        (_, Some(m)) => format!("{flag} must be between {min} and {m} (design section 2)"),
        (1, None) => format!("{flag} must be at least 1"),
        _ => format!("{flag} must not be negative"),
    };
    Err((2, msg))
}

// -------------------------------------------------------------- commands --

fn cmd_search(args: SearchArgs) -> Result<(), CliError> {
    // Count validation comes first (#5407): before the via-service
    // attempt and any backend/embedder work, so an out-of-range
    // count fails identically whether or not a service is running.
    let k = count("--k", args.k, 1, None)?;
    let pool = count("--pool", args.pool, 1, Some(400))?;
    let expand = count(
        "--expand",
        args.expand,
        0,
        Some(i64::try_from(atlas_core::MAX_EXPAND_NEIGHBORS).expect("neighbor cap fits in i64")),
    )?;
    let trunc = count("--trunc", args.vector.trunc, 0, None)?;
    count("--parameters_version", args.parameters_version, 1, Some(1))?;
    count("--shard_top_k", args.shard_top_k, 1, Some(400))?;
    count("--global_top_k", args.global_top_k, 1, Some(400))?;
    count("--jaccard_depth", args.jaccard_depth, 0, Some(4000))?;
    count("--rrf_k", args.rrf_k, 1, Some(10000))?;
    parameters_from_args(&args)
        .validate(expand)
        .map_err(|m| (2, m))?;
    // The disclosure context validates up front too (#12404),
    // so a bad value fails identically on every path.
    let context = parse_context(&args.context)?;
    if args.source_registry.is_some() && args.shards.is_none() {
        return Err((
            2,
            "--source-registry requires --shards: occurrence attribution rides the shard fan-out (#3125)"
                .to_string(),
        ));
    }
    let mut note: Option<String> = None;
    // The shard fan-out is standalone-only: the service does not
    // honor it yet, so a via-service answer would silently come
    // from the monolith.
    if args.shards.is_none() {
        match try_via_service_search(&args) {
            Ok(Some(out)) => {
                print_output(&out, args.text);
                return Ok(());
            }
            Ok(None) => {}
            Err(n) => note = Some(n),
        }
    }
    let filters = Filters::new(
        split_csv(args.kind.as_deref()),
        split_csv(args.topic.as_deref()),
        split_csv(args.project.as_deref()),
        split_csv(args.client.as_deref()),
        &args.max_sensitivity,
        args.since.clone(),
        args.until.clone(),
    )
    .map_err(map_err)?;
    let mut backend = pick_backend(&args.backend, &args.vector.vec_table, trunc)?;
    if let Some(spec) = args.shards.as_deref() {
        let set = open_from_spec(
            spec,
            TierCeiling::from_sensitivity(filters.max_sensitivity),
            &args.vector.vec_table,
        )
        .map_err(map_err)?;
        let mut sharded = ShardedBackend::new(backend, set);
        if let Some(path) = args.source_registry.as_deref() {
            let conn = atlas_core::source_registry::open_source_registry(path).map_err(map_err)?;
            sharded = sharded.with_source_registry(conn);
        }
        backend = Box::new(sharded);
    }
    let embedder = CliEmbedder::new(args.vector.model.clone(), args.vector.embedder);
    let opts = SearchOptions {
        query: args.query.clone(),
        mode: match args.mode {
            ModeChoice::Hybrid => SearchMode::Hybrid,
            ModeChoice::Fts => SearchMode::Fts,
            ModeChoice::Vector => SearchMode::Vector,
        },
        k,
        pool,
        group_by_session: args.group_by_session,
        expand,
        with_message: args.with_message,
        model: args.vector.model.clone(),
        embedder_label: match args.vector.embedder {
            EmbedderChoice::Auto => "auto",
            EmbedderChoice::Fastembed => "fastembed",
            EmbedderChoice::St => "st",
        }
        .to_string(),
        trunc,
        vector_backend: match args.vector.vector_backend {
            VectorBackendChoice::Scan => VectorBackend::Scan,
            VectorBackendChoice::IndexFill => VectorBackend::IndexFill,
            VectorBackendChoice::IndexFixed => VectorBackend::IndexFixed,
        },
        parameters: parameters_from_args(&args),
        context,
    };
    // Trace capture (issue #9973): every executed search persists
    // an atlas-query-trace/v1 record and returns its query_id in
    // the envelope; a trace write failure fails the search.
    let trace_sink = JsonlTraceSink::new(JsonlTraceSink::default_dir());
    let mut trace_ctx = TraceContext::new("cli", "user", "cli");
    trace_ctx.retain_query_text = true;
    let mut out = run_search_traced(
        backend.as_ref(),
        &embedder,
        &filters,
        &opts,
        &trace_sink,
        trace_ctx,
    )
    .map_err(map_err)?;
    if let Some(note) = note {
        push_note(&mut out, note);
    }
    print_output(&out, args.text);
    Ok(())
}

fn cmd_get_chunk(args: GetChunkArgs) -> Result<(), CliError> {
    let neighbors = count("--neighbors", args.neighbors, 0, Some(100))?;
    let context = parse_context(&args.context)?;
    let note = via_service_note_get_chunk(&args.backend);
    // The Python's get-chunk takes no vector args: pick_backend
    // sees the defaults (getattr fallbacks in its main()).
    let backend = pick_backend(&args.backend, atlas_core::DEFAULT_VEC_TABLE, 0)?;
    let mut out = run_get_chunk_with_context(
        backend.as_ref(),
        args.chunk_id,
        neighbors,
        context,
        context.ceiling(),
    )
    .map_err(|e| map_get_chunk_err(e, args.chunk_id))?;
    if let Some(note) = note {
        push_note(&mut out, note);
    }
    print_output(&out, args.text);
    Ok(())
}

/// The shard discovery ceiling for serve mode (#6666 slice 1):
/// the highest client ceiling cap in the service config.
/// Discovery opens at the service maximum; per-caller ceilings
/// only narrow the set at query time (`ShardSet::select`).
fn serve_discovery_ceiling(clients: &[server::ClientConfig]) -> atlas_core::shards::TierCeiling {
    fn height(t: atlas_core::shards::Tier) -> u8 {
        match t {
            atlas_core::shards::Tier::Public => 0,
            atlas_core::shards::Tier::Internal => 1,
            atlas_core::shards::Tier::Secret => 2,
        }
    }
    let mut ceiling =
        atlas_core::shards::TierCeiling::from_sensitivity(atlas_core::Sensitivity::Shareable);
    for c in clients {
        let cap = atlas_core::shards::TierCeiling::from_sensitivity(
            atlas_core::Sensitivity::from_name(c.ceiling_cap.name())
                .unwrap_or(atlas_core::Sensitivity::Personal),
        );
        if height(cap.0) > height(ceiling.0) {
            ceiling = cap;
        }
    }
    ceiling
}

fn cmd_serve(args: ServeArgs) -> Result<(), CliError> {
    let path = server::resolve_service_config_path(args.config.as_deref());
    let settings = server::load_service_settings(
        &path,
        server::ServeOverrides {
            listen: args.listen,
            job_ttl_ms: args.job_ttl_ms,
            hard_ttl_ms: args.hard_ttl_ms,
        },
    )
    .map_err(|m| (2, m))?;
    let backend = TursoBackend::new(&settings.vec_table).map_err(map_err)?;
    let shared = std::sync::Arc::new(server::SharedEmbedder::new(std::sync::Arc::new(
        CliEmbedder::new(atlas_core::DEFAULT_MODEL.to_string(), EmbedderChoice::Auto),
    )));
    let state_fn = shared.state_fn();
    // Serve-mode shard fan-out (#6666 slice 1): with a `shards`
    // spec in the service config, the vector branch is served by
    // the shard set, re-discovered on the configured interval;
    // without one the monolith serves exactly as before.
    if let Some(spec) = &settings.shards {
        let ceiling = serve_discovery_ceiling(&settings.clients);
        let set = open_from_spec(spec, ceiling, &settings.vec_table).map_err(map_err)?;
        let mut backend = ShardedBackend::new(backend, set).with_refresh(
            spec.clone(),
            ceiling,
            settings.shard_refresh,
        );
        if let Some(path) = &settings.source_registry {
            let conn = atlas_core::source_registry::open_source_registry(path).map_err(map_err)?;
            backend = backend.with_source_registry(conn);
        }
        let engine = std::sync::Arc::new(server::CoreSearchEngine::new(
            std::sync::Arc::new(backend),
            shared,
            state_fn,
        ));
        return server::serve_forever(settings, engine).map_err(|m| (1, m));
    }
    let engine = std::sync::Arc::new(server::CoreSearchEngine::new(
        std::sync::Arc::new(backend),
        shared,
        state_fn,
    ));
    server::serve_forever(settings, engine).map_err(|m| (1, m))
}

fn cmd_feedback(args: FeedbackArgs) -> Result<(), CliError> {
    if args.get {
        let out = if args.via_service {
            let base = service::base_url(args.service_url.as_deref());
            service::get_feedback(&base, &args.query, args.chunk).map_err(|m| (1, m))?
        } else {
            let dir = args
                .dir
                .clone()
                .unwrap_or_else(JsonlFeedbackSink::default_dir);
            let records = load_feedback_dir(&dir).map_err(map_err)?;
            let views = feedback_for(&records, &args.query, args.chunk);
            serde_json::json!({
                "query_id": args.query,
                "chunk_id": args.chunk,
                "feedback": views.iter().map(|v| v.to_json()).collect::<Vec<_>>(),
            })
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(());
    }
    let vote = match (args.up, args.down) {
        (true, false) => Vote::Up,
        (false, true) => Vote::Down,
        _ => {
            return Err((
                2,
                "feedback needs exactly one of --up / --down (or --get to retrieve)".to_string(),
            ));
        }
    };
    let comment = match &args.comment {
        Some(raw) if raw.starts_with('@') => {
            let path = &raw[1..];
            Some(
                std::fs::read_to_string(path)
                    .map_err(|e| (1, format!("cannot read comment file {path}: {e}")))?,
            )
        }
        other => other.clone(),
    };
    if args.via_service {
        let base = service::base_url(args.service_url.as_deref());
        let payload = serde_json::json!({
            "query_id": args.query,
            "chunk_id": args.chunk,
            "vote": vote.as_str(),
            "comment": comment,
            "opened": args.opened,
            "requester": {"kind": args.requester_kind, "id": args.requester_id},
        });
        let out = service::post_feedback(&base, &payload).map_err(|m| (1, m))?;
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(());
    }
    let input = FeedbackInput {
        query_id: args.query.clone(),
        chunk_id: args.chunk,
        vote,
        comment,
        requester_kind: args.requester_kind.clone(),
        requester_id: args.requester_id.clone(),
        opened: args.opened,
    };
    let dir = args
        .dir
        .clone()
        .unwrap_or_else(JsonlFeedbackSink::default_dir);
    let sink = JsonlFeedbackSink::new(dir);
    let record = record_feedback(&sink, &input).map_err(map_err)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&record.to_json()).unwrap_or_default()
    );
    Ok(())
}

fn cmd_traces(args: TracesArgs) -> Result<(), CliError> {
    match args.cmd {
        TracesCmd::Ingest(a) => {
            let dir = a.dir.unwrap_or_else(JsonlTraceSink::default_dir);
            let corpus = ingest_traces(&dir).map_err(map_err)?;
            let docs: Vec<&TraceDocument> = match &a.query {
                Some(q) => search_traces(&corpus, q),
                None => corpus.documents.iter().collect(),
            };
            let out = serde_json::json!({
                "source_id": corpus.registration.source_id,
                "source_class": corpus.registration.source_class,
                "record_schema": corpus.registration.record_schema,
                "dir": dir.display().to_string(),
                "files": corpus.files,
                "documents": corpus.documents.len(),
                "skipped_lines": corpus.skipped_lines,
                "query": a.query,
                "matches": docs
                    .iter()
                    .map(|d| serde_json::json!({
                        "query_id": d.query_id,
                        "ts": d.ts,
                        "surface": d.surface,
                        "requester": format!("{}:{}", d.requester_kind, d.requester_id),
                        "query_text": d.query_text,
                        "query_text_hash": d.query_text_hash,
                        "served": d.served,
                        "merged": d.merged,
                    }))
                    .collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            Ok(())
        }
        TracesCmd::Retention(a) => {
            let dir = a.dir.unwrap_or_else(JsonlTraceSink::default_dir);
            let today_str = match &a.today {
                Some(t) => t.clone(),
                None => {
                    let ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    atlas_core::trace::rfc3339_utc(ms)[..10].to_string()
                }
            };
            // Strict calendar validation: every component must
            // parse, and the result must be a real date. An
            // out-of-range month/day would otherwise be silently
            // normalised by the day-count math and could age
            // recent traces out of the hot tier by years.
            let today: Option<(i64, u32, u32)> = (|| {
                let mut parts = today_str.split('-');
                let y: i64 = parts.next()?.parse().ok()?;
                let m: u32 = parts.next()?.parse().ok()?;
                let d: u32 = parts.next()?.parse().ok()?;
                if parts.next().is_some() || !valid_civil_date(y, m, d) {
                    return None;
                }
                Some((y, m, d))
            })();
            let Some(today) = today else {
                return Err((
                    2,
                    format!("--today must be a valid YYYY-MM-DD date, got {today_str}"),
                ));
            };
            let refs = load_feedback_refs(&a.feedback).map_err(map_err)?;
            let report = run_retention(&dir, today, &refs).map_err(map_err)?;
            let out = serde_json::json!({
                "dir": dir.display().to_string(),
                "today": today_str,
                "feedback_refs": refs.len(),
                "kept_files": report.kept_files,
                "moved_files": report.moved_files,
                "compacted_records": report.compacted_records,
                "exempt_records": report.exempt_records,
                "cold_files": report.cold_files,
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            Ok(())
        }
        TracesCmd::Stats(a) => {
            let dir = a.dir.unwrap_or_else(JsonlTraceSink::default_dir);
            let today = parse_today_arg(&a.today)?;
            let votes = load_feedback_votes(&a.feedback).map_err(map_err)?;
            let query_votes = load_query_feedback_votes(&a.feedback).map_err(map_err)?;
            let stats = aggregate_traces(&dir, today, a.window_days, &votes, &query_votes)
                .map_err(map_err)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&stats.to_json()).unwrap_or_default()
            );
            Ok(())
        }
        TracesCmd::Report(a) => {
            let dir = a.dir.unwrap_or_else(JsonlTraceSink::default_dir);
            let today = parse_today_arg(&a.today)?;
            let votes = load_feedback_votes(&a.feedback).map_err(map_err)?;
            let query_votes = load_query_feedback_votes(&a.feedback).map_err(map_err)?;
            let report = weekly_report(&dir, today, &votes, &query_votes).map_err(map_err)?;
            let report_file = write_report(&dir, &report).map_err(map_err)?;
            let md_file = dir.join(format!("atlas-statistics-report-{}.md", report.report_day));
            std::fs::write(&md_file, report.render_markdown())
                .map_err(|e| (1, format!("cannot write {}: {e}", md_file.display())))?;
            let mut out = report.to_json();
            out["report_file"] = serde_json::json!(report_file.display().to_string());
            out["markdown_file"] = serde_json::json!(md_file.display().to_string());
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            Ok(())
        }
        TracesCmd::Normalise(a) => {
            let dir = a.dir.unwrap_or_else(JsonlTraceSink::default_dir);
            let today = parse_today_arg(&a.today)?;
            let day = atlas_core::format_day(today.0, today.1, today.2);
            let feedback_dir = a
                .feedback_dir
                .unwrap_or_else(JsonlFeedbackSink::default_dir);
            let records = load_feedback_dir(&feedback_dir).map_err(map_err)?;
            let requesters = load_query_requesters(&dir).map_err(map_err)?;
            let normalised = normalise_records(&records, &requesters);
            let out_dir = a.out_dir.unwrap_or_else(|| feedback_dir.join("normalised"));
            let file = write_normalised(&out_dir, &day, &normalised).map_err(map_err)?;
            let self_feedback = normalised.iter().filter(|n| n.self_feedback).count();
            let out = serde_json::json!({
                "feedback_dir": feedback_dir.display().to_string(),
                "raw_records": records.len(),
                "normalised": normalised.len(),
                "self_feedback": self_feedback,
                "normalised_file": file.display().to_string(),
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            Ok(())
        }
    }
}

/// Parse a `--today YYYY-MM-DD` argument (default: today, UTC)
/// with the same strict calendar validation the retention pass
/// applies: an out-of-range month/day must be a usage error,
/// never a silently normalised date.
fn parse_today_arg(arg: &Option<String>) -> Result<(i64, u32, u32), CliError> {
    let today_str = match arg {
        Some(t) => t.clone(),
        None => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            atlas_core::trace::rfc3339_utc(ms)[..10].to_string()
        }
    };
    let parsed: Option<(i64, u32, u32)> = (|| {
        let mut parts = today_str.split('-');
        let y: i64 = parts.next()?.parse().ok()?;
        let m: u32 = parts.next()?.parse().ok()?;
        let d: u32 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || !valid_civil_date(y, m, d) {
            return None;
        }
        Some((y, m, d))
    })();
    parsed.ok_or((
        2,
        format!("--today must be a valid YYYY-MM-DD date, got {today_str}"),
    ))
}

fn cmd_fetch_model() -> Result<(), CliError> {
    fetch_model::fetch_model().map_err(|m| (2, m))
}

fn cmd_init(args: InitArgs) -> Result<(), CliError> {
    let path = server::resolve_service_config_path(args.config.as_deref());
    let summary = server::init_service_config(&path, args.force).map_err(|m| (2, m))?;
    println!(
        "atlas-search: wrote service config {} (0600) with the v1 client table:",
        path.display()
    );
    for (name, cap) in summary {
        println!("  {name} (ceiling cap: {})", cap.name());
    }
    println!(
        "atlas-search: token values are in the config file only; copy each client's token into its own client config — never commit them"
    );
    Ok(())
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.cmd {
        Cmd::Search(args) => cmd_search(*args),
        Cmd::GetChunk(args) => cmd_get_chunk(args),
        Cmd::Serve(args) => cmd_serve(args),
        Cmd::Init(args) => cmd_init(args),
        Cmd::FetchModel => cmd_fetch_model(),
        Cmd::Traces(args) => cmd_traces(args),
        Cmd::Feedback(args) => cmd_feedback(args),
    }
}

fn main() -> ExitCode {
    // --version is answered before clap, like the hand-rolled Atlas
    // binaries: the family reports `<version>+g<source revision>`
    // (#13279 part 2), which clap's CARGO_PKG_VERSION magic cannot.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!(
            "atlas-search {}",
            atlas_core::version::version_string(env!("CARGO_PKG_VERSION"))
        );
        return ExitCode::SUCCESS;
    }
    let cli = <Cli as clap::Parser>::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("atlas-search: {msg}");
            ExitCode::from(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_and_raw_request_share_query_parameter_names_and_values() {
        let cli = Cli::try_parse_from([
            "atlas-search",
            "search",
            "atlas",
            "--shard_top_k",
            "32",
            "--global_top_k",
            "8",
            "--fts_weight",
            "1.5",
            "--vector_weight",
            "0.75",
            "--rrf_k",
            "20",
            "--raw",
        ])
        .unwrap();
        let Cmd::Search(args) = cli.cmd else {
            panic!("expected search command")
        };
        let cli_request = proto_search_request(&args);
        let raw_request: SearchRequest = serde_json::from_value(serde_json::json!({
            "query":"atlas",
            "parameters_version":1,
            "shard_top_k":32,
            "global_top_k":8,
            "fts_weight":1.5,
            "vector_weight":0.75,
            "rrf_k":20,
            "raw":true,
            // The CLI one-shot implies --wait (design s3.3), so
            // the equivalent raw request spells it out; every
            // other slice-2 flag shares the off default.
            "wait":true
        }))
        .unwrap();
        assert_eq!(cli_request.parameters, raw_request.parameters);
    }

    #[test]
    fn feedback_cli_parses_the_design_invocation() {
        // Oracle: design section 6.1's CLI form —
        // `atlas feedback --query <query_id> --chunk <chunk_id>
        // --up|--down [--comment <text|@file>]`.
        let cli = Cli::try_parse_from([
            "atlas-search",
            "feedback",
            "--query",
            "q-1",
            "--chunk",
            "42",
            "--down",
            "--comment",
            "stale chunk",
        ])
        .unwrap();
        let Cmd::Feedback(args) = cli.cmd else {
            panic!("expected feedback command")
        };
        assert_eq!(args.query, "q-1");
        assert_eq!(args.chunk, Some(42));
        assert!(args.down && !args.up);
        assert_eq!(args.comment.as_deref(), Some("stale chunk"));
        assert!(!args.get);

        // Per-query vote: no --chunk, and --get retrieves.
        let cli =
            Cli::try_parse_from(["atlas-search", "feedback", "--query", "q-1", "--get"]).unwrap();
        let Cmd::Feedback(args) = cli.cmd else {
            panic!("expected feedback command")
        };
        assert_eq!(args.chunk, None);
        assert!(args.get);

        // --up and --down conflict at parse time.
        assert!(
            Cli::try_parse_from([
                "atlas-search",
                "feedback",
                "--query",
                "q-1",
                "--up",
                "--down",
            ])
            .is_err()
        );
    }

    #[test]
    fn count_validation_mirrors_the_service_contract() {
        // k: at least 1 (server.rs validate_request)
        assert_eq!(count("--k", 1, 1, None), Ok(1));
        assert_eq!(count("--k", 10, 1, None), Ok(10));
        assert_eq!(
            count("--k", 0, 1, None).unwrap_err(),
            (2, "--k must be at least 1".to_string())
        );
        assert!(count("--k", -1, 1, None).is_err());
        // pool: 1..=400 (design section 2)
        assert_eq!(count("--pool", 400, 1, Some(400)), Ok(400));
        assert_eq!(
            count("--pool", 401, 1, Some(400)).unwrap_err(),
            (
                2,
                "--pool must be between 1 and 400 (design section 2)".to_string()
            )
        );
        assert!(count("--pool", 0, 1, Some(400)).is_err());
        assert!(count("--pool", -5, 1, Some(400)).is_err());
        // --expand shares the service's 100-neighbor ceiling.
        let max_expand = atlas_core::MAX_EXPAND_NEIGHBORS as i64;
        assert_eq!(count("--expand", max_expand, 0, Some(max_expand)), Ok(100));
        assert_eq!(
            count("--expand", max_expand + 1, 0, Some(max_expand)).unwrap_err(),
            (
                2,
                "--expand must be between 0 and 100 (design section 2)".to_string()
            )
        );
        // zero-is-valid counts: 0 stays valid, negatives do not
        for flag in ["--trunc"] {
            assert_eq!(count(flag, 0, 0, None), Ok(0), "{flag}");
            assert_eq!(
                count(flag, -1, 0, None).unwrap_err(),
                (2, format!("{flag} must not be negative")),
                "{flag}"
            );
        }
        // --neighbors has a 100 upper bound (service cap)
        assert_eq!(count("--neighbors", 0, 0, Some(100)), Ok(0));
        assert_eq!(count("--neighbors", 100, 0, Some(100)), Ok(100));
        assert_eq!(
            count("--neighbors", 101, 0, Some(100)).unwrap_err(),
            (
                2,
                "--neighbors must be between 0 and 100 (design section 2)".to_string()
            )
        );
        assert_eq!(
            count("--neighbors", -1, 0, Some(100)).unwrap_err(),
            (2, "--neighbors must not be negative".to_string())
        );
    }
}
