//! atlas-search — the Atlas access layer: hybrid search over the corpus.
//!
//! The one-shot CLI (epic developing-today/code#5350, step 3,
//! #5353): a surface-parity port of the Python `atlas_search.py`'s
//! front-end over atlas-core (step 2) and atlas-embed (step 1).
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
use atlas_core::turso::{TURSO_HOST, resolve_turso_token};
use atlas_core::{
    AtlasEmbedder, Error as CoreError, FixedEmbedder, LocalBackend, QueryEmbedder, SearchBackend,
    SearchMode, SearchOptions, TursoBackend, VectorBackend, run_get_chunk, run_search,
};
use atlas_proto::{
    FilterSpec, SearchMode as ProtoSearchMode, SearchRequest, Sensitivity as ProtoSensitivity,
    VectorBackend as ProtoVectorBackend,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::Value as Json;

// ---------------------------------------------------------------- args --

#[derive(Parser)]
#[command(
    name = "atlas-search",
    version,
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
}

#[derive(Args)]
struct ServeArgs {
    /// service config path (default: ~/.local/state/atlas-search/service.json; env ATLAS_SEARCH_SERVICE_CONFIG)
    #[arg(long)]
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
    #[arg(long)]
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
    /// prefer a running atlas service; fall back to standalone, loudly
    #[arg(long)]
    via_service: bool,
    /// service base URL for --via-service (env ATLAS_SEARCH_URL)
    #[arg(long)]
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
    #[arg(long, default_value_t = 10)]
    k: i64,
    /// per-branch candidates (default 100)
    #[arg(long, default_value_t = 100)]
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
    /// ceiling in shareable<personal<local-only<secret-bearing (default personal; secret-bearing must be spelled out)
    #[arg(long, default_value = "personal")]
    max_sensitivity: String,
    /// roll chunk hits up to sessions (score = best chunk, plus hit count)
    #[arg(long)]
    group_by_session: bool,
    /// include N neighboring chunks (same message) around each hit
    #[arg(long, default_value_t = 0)]
    expand: i64,
    /// include the parent message text (truncated at 50000 chars)
    #[arg(long)]
    with_message: bool,
    /// human-readable output instead of JSON
    #[arg(long)]
    text: bool,
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
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"))
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
    Real(Box<AtlasEmbedder>),
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
                load_embedder(&self.model, self.embedder).map(|e| LoadedEmbedder::Real(Box::new(e)))
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
        },
        ceiling: ProtoSensitivity::from_name(&args.max_sensitivity)
            .unwrap_or(ProtoSensitivity::Personal),
        k: u(args.k),
        pool: u(args.pool),
        group_by_session: args.group_by_session,
        expand: u(args.expand),
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
    if ProtoSensitivity::from_name(&args.max_sensitivity).is_none() {
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
/// `--k` at least 1, `--pool` between 1 and 400, and the
/// zero-is-valid counts (`--expand`, `--trunc`, `--neighbors`) are
/// non-negative; `--neighbors` is also capped at the service's limit.
/// Out-of-range values are fatal (exit 2), never silently clamped to 0
/// (#5407).
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
    let expand = count("--expand", args.expand, 0, None)?;
    let trunc = count("--trunc", args.vector.trunc, 0, None)?;
    let mut note: Option<String> = None;
    match try_via_service_search(&args) {
        Ok(Some(out)) => {
            print_output(&out, args.text);
            return Ok(());
        }
        Ok(None) => {}
        Err(n) => note = Some(n),
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
    let backend = pick_backend(&args.backend, &args.vector.vec_table, trunc)?;
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
    };
    let mut out = run_search(backend.as_ref(), &embedder, &filters, &opts).map_err(map_err)?;
    if let Some(note) = note {
        push_note(&mut out, note);
    }
    print_output(&out, args.text);
    Ok(())
}

fn cmd_get_chunk(args: GetChunkArgs) -> Result<(), CliError> {
    let neighbors = count("--neighbors", args.neighbors, 0, Some(100))?;
    let note = via_service_note_get_chunk(&args.backend);
    // The Python's get-chunk takes no vector args: pick_backend
    // sees the defaults (getattr fallbacks in its main()).
    let backend = pick_backend(&args.backend, atlas_core::DEFAULT_VEC_TABLE, 0)?;
    let mut out = run_get_chunk(backend.as_ref(), args.chunk_id, neighbors)
        .map_err(|e| map_get_chunk_err(e, args.chunk_id))?;
    if let Some(note) = note {
        push_note(&mut out, note);
    }
    print_output(&out, args.text);
    Ok(())
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
    let engine = std::sync::Arc::new(server::CoreSearchEngine::new(
        std::sync::Arc::new(backend),
        shared,
        state_fn,
    ));
    server::serve_forever(settings, engine).map_err(|m| (1, m))
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
    }
}

fn main() -> ExitCode {
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
        // zero-is-valid counts: 0 stays valid, negatives do not
        for flag in ["--expand", "--trunc"] {
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
