//! Serve mode (design sections 2-3; developing-today/code#5354).
//! Rehomed verbatim from pkgs/atlas-search's bin crate in #5604
//! (`fleet serve search`): the Atlas service must behave identically
//! under either name, so mirror every change in both copies.
//! Drift guard: .github/scripts/check-fleet-search-parity.sh.
//!
//! One process holds the shared query embedder, a Turso backend
//! carrying a read-only token, a bounded job scheduler, and an
//! in-memory temp store. Nothing about a job is written to disk:
//! the only file serve mode reads is the 0600 service config that
//! carries the client table. A restart therefore loses every job,
//! by design, and clients resubmit.
//!
//! Backpressure is branch-shaped. A job reserves one branch slot
//! for FTS/vector modes and two for hybrid mode before it starts;
//! at most `max_concurrent_branch_executions` (default 4) slots
//! are ever reserved. The FTS and vector branches themselves run
//! concurrently through atlas-core's `BranchPoolExecutor` seam,
//! and fusion happens once over their completed pools.

use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atlas_core::types::BranchPools;
use atlas_core::{
    BranchPoolExecutor, BranchQuery, DisclosureContext, Error as CoreError, Filters,
    JsonlFeedbackSink, JsonlTraceSink, Manifest, QueryEmbedder, SearchBackend,
    SearchMode as CoreSearchMode, SearchOptions, Sensitivity as CoreSensitivity, TraceContext,
    VectorBackend as CoreVectorBackend, builtin_manifest, chunk_classification, feedback_for,
    load_feedback_dir, record_feedback, run_get_chunk_with_context,
    run_search_with_executor_traced,
};
use atlas_proto::{
    BranchState, BranchStates, EmbedRequest, EmbedResponse, ErrorResponse, HealthResponse,
    JobCreated, JobDeleted, JobListResponse, JobStatus, JobStatusResponse, JobSummary,
    SearchMode as ProtoSearchMode, SearchPage, SearchRequest, Sensitivity as ProtoSensitivity,
    VectorBackend as ProtoVectorBackend,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

/// The disclosure context a service request runs under
/// (#12404, #9096 design section 4): a stated context resolves
/// fail-closed — an unknown value means `public-surface` — and
/// an unstated one takes this surface's stated default. The
/// service's callers are authenticated fleet surfaces (the job
/// API requires a client credential), so the default is
/// `fleet-internal`; a caller preparing output for any other
/// destination states that destination's context, and sinks
/// beyond this service apply their own.
fn request_context(req: &SearchRequest) -> DisclosureContext {
    match req.context.as_deref() {
        Some(name) => DisclosureContext::for_outbound(Some(name)),
        None => DisclosureContext::FleetInternal,
    }
}

// -------------------------------------------------------------- time ----

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn instant_ms(t: Instant, base: Instant, base_ms: u64) -> u64 {
    base_ms.saturating_add(t.duration_since(base).as_millis() as u64)
}

fn duration_ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

// ------------------------------------------------------------ random ----

fn random_bytes(n: usize) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open("/dev/urandom")?;
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn random_hex(n: usize) -> Result<String, String> {
    let bytes = random_bytes(n).map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn new_job_id() -> Result<String, String> {
    Ok(format!("job_{}", random_hex(16)?))
}

fn new_job_token() -> Result<String, String> {
    Ok(format!("job_{}", random_hex(32)?))
}

fn new_client_token() -> Result<String, String> {
    Ok(format!("atlas_{}", random_hex(32)?))
}

/// Length-aware constant-time string comparison for credentials.
/// Values are never logged anywhere in this module.
fn secure_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

// ------------------------------------------------------------ config ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// TTL for a completed/failed job's stored result: 10 minutes.
    pub job_ttl: Duration,
    /// GC bound for the snapshot-bound result cache (#9943): a
    /// cached ranking may be served for up to this long while its
    /// snapshot is current. Correctness never comes from this
    /// clock — a snapshot bump invalidates entries immediately —
    /// the TTL only bounds how long stale-snapshot / superseded
    /// entries occupy memory. Default 24h (design #5667 s3.2).
    pub result_cache_ttl: Duration,
    /// Hard cap from submit: 60 minutes.
    pub hard_ttl: Duration,
    /// Maximum jobs waiting in the global FIFO execution queue.
    /// Submissions beyond this bound are rejected with 429.
    pub max_queued_jobs: usize,
    pub max_concurrent_branch_executions: usize,
    pub max_concurrent_jobs_per_client: usize,
    pub max_queued_jobs_per_client: usize,
    pub retry_after_seconds: u64,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        RuntimeLimits {
            job_ttl: Duration::from_secs(600),
            result_cache_ttl: Duration::from_secs(24 * 3600),
            hard_ttl: Duration::from_secs(3600),
            max_queued_jobs: 8,
            max_concurrent_branch_executions: 4,
            max_concurrent_jobs_per_client: 2,
            max_queued_jobs_per_client: 8,
            retry_after_seconds: 5,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RuntimeLimitsFile {
    job_ttl_ms: Option<u64>,
    result_cache_ttl_ms: Option<u64>,
    hard_ttl_ms: Option<u64>,
    max_queued_jobs: Option<usize>,
    max_concurrent_branch_executions: Option<usize>,
    max_concurrent_jobs_per_client: Option<usize>,
    max_queued_jobs_per_client: Option<usize>,
    retry_after_seconds: Option<u64>,
}

impl RuntimeLimitsFile {
    fn into_limits(self) -> RuntimeLimits {
        let d = RuntimeLimits::default();
        RuntimeLimits {
            job_ttl: self
                .job_ttl_ms
                .map(Duration::from_millis)
                .unwrap_or(d.job_ttl),
            result_cache_ttl: self
                .result_cache_ttl_ms
                .map(Duration::from_millis)
                .unwrap_or(d.result_cache_ttl),
            hard_ttl: self
                .hard_ttl_ms
                .map(Duration::from_millis)
                .unwrap_or(d.hard_ttl),
            max_queued_jobs: self.max_queued_jobs.unwrap_or(d.max_queued_jobs),
            max_concurrent_branch_executions: self
                .max_concurrent_branch_executions
                .unwrap_or(d.max_concurrent_branch_executions),
            max_concurrent_jobs_per_client: self
                .max_concurrent_jobs_per_client
                .unwrap_or(d.max_concurrent_jobs_per_client),
            max_queued_jobs_per_client: self
                .max_queued_jobs_per_client
                .unwrap_or(d.max_queued_jobs_per_client),
            retry_after_seconds: self.retry_after_seconds.unwrap_or(d.retry_after_seconds),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub name: String,
    /// Secret. Held only in memory and in the 0600 service config;
    /// never logged, never serialized by this crate's responses.
    pub token: String,
    pub ceiling_cap: ProtoSensitivity,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClientConfig {
    name: String,
    token: String,
    ceiling_cap: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ServiceConfigFile {
    listen: Option<String>,
    vec_table: Option<String>,
    /// Shard spec for serve-mode fan-out (#6666 slice 1): the
    /// standalone `--shards` grammar (`dir:<path>` or
    /// `turso[:<org>]`). Absent = monolith only, as before.
    shards: Option<String>,
    /// Seconds between shard re-discoveries in serve mode
    /// (#6666 slice 1). Default 60; ignored without `shards`.
    shard_refresh_seconds: Option<u64>,
    /// Path to the catalog/source-registry database for
    /// occurrence attribution (#3125 slice 7). Requires
    /// `shards`; absent = occurrence rows stay unattributed.
    source_registry: Option<String>,
    limits: Option<RuntimeLimitsFile>,
    clients: Vec<RawClientConfig>,
}

#[derive(Debug, Clone)]
pub struct ServeSettings {
    pub listen: SocketAddr,
    pub clients: Vec<ClientConfig>,
    pub vec_table: String,
    pub limits: RuntimeLimits,
    /// Shard spec for serve-mode fan-out (#6666 slice 1), in the
    /// standalone `--shards` grammar. `None` = monolith only.
    pub shards: Option<String>,
    /// Interval between shard re-discoveries when `shards` is
    /// set; a changed shard set is swapped in atomically and the
    /// snapshot-bound result cache invalidates on the new
    /// snapshot id.
    pub shard_refresh: Duration,
    /// Path to the catalog/source-registry database for
    /// occurrence attribution (#3125 slice 7): opened at serve
    /// start and attached to the shard fan-out, so served
    /// get-chunk occurrence rows carry `root_id` for registered
    /// locators. Requires `shards`; `None` = unattributed rows,
    /// exactly as before slice 7.
    pub source_registry: Option<PathBuf>,
}

/// Default shard re-discovery interval for serve mode (#6666).
pub const DEFAULT_SHARD_REFRESH_SECONDS: u64 = 60;

fn home_dir() -> PathBuf {
    // Home-resolution contract (issues #5640 / #5762): a missing or
    // empty HOME is an error naming HOME, never a substitute
    // directory - the old fallback to a literal "~" silently
    // resolved state/config/token paths relative to the CWD.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => panic!(
            "HOME is not set or is empty: fleet-search requires a non-empty HOME to resolve its state/config paths (issue #5640)"
        ),
    }
}

pub fn default_service_config_path() -> PathBuf {
    home_dir().join(".local/state/atlas-search/service.json")
}

pub fn resolve_service_config_path(flag: Option<&Path>) -> PathBuf {
    // An empty flag value counts as absent (same reasoning as
    // service::base_url: the clap env binding on --config passes a
    // set-but-empty ATLAS_SEARCH_SERVICE_CONFIG through as a value).
    if let Some(p) = flag.filter(|p| !p.as_os_str().is_empty()) {
        return p.to_path_buf();
    }
    if let Ok(p) = std::env::var("ATLAS_SEARCH_SERVICE_CONFIG")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    default_service_config_path()
}

/// Optional serve-mode overrides from the command line. Durations
/// are milliseconds so tests and operators can tune TTL behavior
/// without editing the service config.
#[derive(Debug, Default, Clone, Copy)]
pub struct ServeOverrides {
    pub listen: Option<SocketAddr>,
    pub job_ttl_ms: Option<u64>,
    pub hard_ttl_ms: Option<u64>,
}

pub fn load_service_settings(
    path: &Path,
    overrides: ServeOverrides,
) -> Result<ServeSettings, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "cannot read service config {}: {e}; run `atlas-search init` first",
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|e| format!("cannot stat service config {}: {e}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "service config {} is mode {:04o}; it carries client tokens and must be 0600",
                path.display(),
                mode & 0o777
            ));
        }
    }
    let raw: ServiceConfigFile =
        serde_json::from_str(&text).map_err(|e| format!("bad service config JSON: {e}"))?;
    if raw.clients.is_empty() {
        return Err("service config has no clients; run `atlas-search init`".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    let mut clients = Vec::new();
    for c in raw.clients {
        if c.name.is_empty() || c.token.is_empty() {
            return Err("service config client rows need a name and a token".to_string());
        }
        if !seen.insert(c.name.clone()) {
            return Err(format!(
                "duplicate client name in service config: {}",
                c.name
            ));
        }
        let cap = ProtoSensitivity::from_ceiling_name(&c.ceiling_cap).ok_or_else(|| {
            format!(
                "client {}: ceiling_cap must be one of shareable, personal, local-only, secret-bearing (or public, internal, private, sensitive)",
                c.name
            )
        })?;
        clients.push(ClientConfig {
            name: c.name,
            token: c.token,
            ceiling_cap: cap,
        });
    }
    let listen_s = raw.listen.as_deref().unwrap_or("127.0.0.1:18760");
    let listen: SocketAddr = listen_s
        .parse()
        .map_err(|e| format!("service config listen `{listen_s}` is not a socket address: {e}"))?;
    let listen = overrides.listen.unwrap_or(listen);
    require_loopback(listen)?;
    let mut limits = raw.limits.map(|l| l.into_limits()).unwrap_or_default();
    if let Some(ms) = overrides.job_ttl_ms {
        limits.job_ttl = Duration::from_millis(ms);
    }
    if let Some(ms) = overrides.hard_ttl_ms {
        limits.hard_ttl = Duration::from_millis(ms);
    }
    validate_limits(&limits)?;
    let shards = match raw.shards {
        Some(spec) if !spec.trim().is_empty() => Some(spec),
        Some(_) => {
            return Err("service config shards spec must not be empty".to_string());
        }
        None => None,
    };
    let shard_refresh_seconds = raw
        .shard_refresh_seconds
        .unwrap_or(DEFAULT_SHARD_REFRESH_SECONDS);
    if shard_refresh_seconds == 0 {
        return Err("service config shard_refresh_seconds must be at least 1".to_string());
    }
    let source_registry = match raw.source_registry {
        Some(p) if !p.trim().is_empty() => Some(PathBuf::from(p)),
        Some(_) => {
            return Err("service config source_registry path must not be empty".to_string());
        }
        None => None,
    };
    if source_registry.is_some() && shards.is_none() {
        return Err(
            "service config source_registry requires shards: occurrence attribution rides the shard fan-out (#3125)"
                .to_string(),
        );
    }
    Ok(ServeSettings {
        listen,
        clients,
        vec_table: raw
            .vec_table
            .unwrap_or_else(|| atlas_core::DEFAULT_VEC_TABLE.to_string()),
        limits,
        shards,
        shard_refresh: Duration::from_secs(shard_refresh_seconds),
        source_registry,
    })
}

fn require_loopback(addr: SocketAddr) -> Result<(), String> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "refusing to bind {addr}: serve mode binds loopback only in step 4; tailnet exposure is packaging work (#5355)"
        ))
    }
}

fn validate_limits(limits: &RuntimeLimits) -> Result<(), String> {
    if limits.max_concurrent_branch_executions == 0 {
        return Err("max_concurrent_branch_executions must be at least 1".to_string());
    }
    if limits.max_concurrent_jobs_per_client == 0 {
        return Err("max_concurrent_jobs_per_client must be at least 1".to_string());
    }
    if limits.max_queued_jobs == 0 {
        return Err("max_queued_jobs must be at least 1".to_string());
    }
    if limits.max_queued_jobs_per_client == 0 {
        return Err("max_queued_jobs_per_client must be at least 1".to_string());
    }
    if limits.job_ttl.is_zero() || limits.hard_ttl.is_zero() {
        return Err("job TTLs must be positive".to_string());
    }
    if limits.result_cache_ttl.is_zero() {
        return Err("result_cache_ttl must be positive".to_string());
    }
    if limits.hard_ttl < limits.job_ttl {
        return Err("hard_ttl must be at least job_ttl".to_string());
    }
    Ok(())
}

/// The `init` verb: generate the v1 client table (design section
/// 3) into a new 0600 service config. Token values are written to
/// the file only — never printed. Returns the client names/caps
/// for the operator-facing summary.
pub fn init_service_config(
    path: &Path,
    force: bool,
) -> Result<Vec<(String, ProtoSensitivity)>, String> {
    if path.exists() && !force {
        return Err(format!(
            "service config {} already exists; pass --force to replace it (this rotates every client token)",
            path.display()
        ));
    }
    let rows: [(&str, ProtoSensitivity); 4] = [
        ("drew-cli", ProtoSensitivity::SecretBearing),
        ("openchamber", ProtoSensitivity::Personal),
        ("fork-server", ProtoSensitivity::Personal),
        ("sandbox-scripts", ProtoSensitivity::Personal),
    ];
    let mut clients = Vec::new();
    let mut summary = Vec::new();
    for (name, cap) in rows {
        clients.push(serde_json::json!({
            "name": name,
            "token": new_client_token()?,
            "ceiling_cap": cap.name(),
        }));
        summary.push((name.to_string(), cap));
    }
    let doc = serde_json::json!({
        "listen": "127.0.0.1:18760",
        "vec_table": atlas_core::DEFAULT_VEC_TABLE,
        "clients": clients,
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| format!("cannot write service config {}: {e}", path.display()))?;
    f.write_all(serde_json::to_string_pretty(&doc).unwrap().as_bytes())
        .and_then(|_| f.write_all(b"\n"))
        .map_err(|e| format!("cannot write service config {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot set 0600 on {}: {e}", path.display()))?;
    }
    Ok(summary)
}

impl ServeSettings {
    pub fn authenticate_client(&self, token: &str) -> Option<ClientConfig> {
        self.clients
            .iter()
            .find(|c| secure_eq(&c.token, token))
            .cloned()
    }
}

// ------------------------------------------------------------- error ----

#[derive(Debug, Clone)]
pub struct ServerError {
    pub status: u16,
    pub body: ErrorResponse,
}

impl ServerError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> ServerError {
        ServerError {
            status,
            body: ErrorResponse::new(code, message.into()),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> ServerError {
        ServerError::new(400, "bad_request", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> ServerError {
        ServerError::new(401, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> ServerError {
        ServerError::new(403, "forbidden", message)
    }

    pub fn job_expired() -> ServerError {
        ServerError::new(
            404,
            "job_expired_or_unknown",
            "job is expired or unknown; resubmit the search",
        )
    }

    pub fn job_gone() -> ServerError {
        ServerError::new(
            410,
            "job_expired",
            "job expired before its results were read; resubmit the search",
        )
    }

    pub fn rate_limited(retry_after_seconds: u64, queue_depth: usize) -> ServerError {
        ServerError {
            status: 429,
            body: ErrorResponse::rate_limited(
                "atlas service is saturated; retry after the indicated delay",
                retry_after_seconds,
                queue_depth,
            ),
        }
    }

    pub fn from_core(e: &CoreError) -> ServerError {
        match e {
            CoreError::Fatal(m) => ServerError::new(400, "search_rejected", m.clone()),
            CoreError::Backend(m) | CoreError::Embed(m) => {
                ServerError::new(500, "search_failed", m.clone())
            }
        }
    }
}

// ------------------------------------------------------ proto mapping ----

fn core_mode(m: ProtoSearchMode) -> CoreSearchMode {
    match m {
        ProtoSearchMode::Fts => CoreSearchMode::Fts,
        ProtoSearchMode::Vector => CoreSearchMode::Vector,
        ProtoSearchMode::Hybrid => CoreSearchMode::Hybrid,
    }
}

fn core_vector_backend(v: ProtoVectorBackend) -> CoreVectorBackend {
    match v {
        ProtoVectorBackend::Scan => CoreVectorBackend::Scan,
        ProtoVectorBackend::IndexFixed => CoreVectorBackend::IndexFixed,
        ProtoVectorBackend::IndexFill => CoreVectorBackend::IndexFill,
    }
}

fn core_sensitivity(s: ProtoSensitivity) -> CoreSensitivity {
    match s {
        ProtoSensitivity::Shareable => CoreSensitivity::Shareable,
        ProtoSensitivity::Personal => CoreSensitivity::Personal,
        ProtoSensitivity::LocalOnly => CoreSensitivity::LocalOnly,
        ProtoSensitivity::SecretBearing => CoreSensitivity::SecretBearing,
    }
}

fn build_filters(req: &SearchRequest, effective: ProtoSensitivity) -> Result<Filters, ServerError> {
    if req
        .filters
        .dimensions
        .values()
        .any(|values| !values.is_empty())
    {
        return Err(ServerError::bad_request(
            "generic filter dimensions are not supported by this search surface yet",
        ));
    }
    Filters::new(
        req.filters.kind.clone(),
        req.filters.topic.clone(),
        req.filters.project.clone(),
        req.filters.client.clone(),
        effective.name(),
        req.filters.since.clone(),
        req.filters.until.clone(),
    )
    .map_err(|e| ServerError::from_core(&e))
}

fn validate_request(req: &SearchRequest) -> Result<(), ServerError> {
    if req
        .filters
        .dimensions
        .values()
        .any(|values| !values.is_empty())
    {
        return Err(ServerError::bad_request(
            "generic filter dimensions are not supported by this search surface yet",
        ));
    }
    if req.pool == 0 || req.pool > 400 {
        return Err(ServerError::bad_request(
            "pool must be between 1 and 400 (design section 2)",
        ));
    }
    if req.k == 0 {
        return Err(ServerError::bad_request("k must be at least 1"));
    }
    req.parameters
        .validate(req.expand)
        .map_err(ServerError::bad_request)?;
    if let Some(limit) = req.page_limit
        && limit == 0
    {
        return Err(ServerError::bad_request("page_limit must be at least 1"));
    }
    Ok(())
}

// ------------------------------------------------------------ engine ----

/// What the job manager needs from a search implementation. The
/// production implementation is [`CoreSearchEngine`] over the
/// Turso backend; tests substitute a stub.
/// The snapshot id for an engine with no corpus snapshot: a
/// constant, so its result-cache entries form one stable
/// snapshot partition (the Turso monolith today).
pub const NO_SNAPSHOT_ID: &str = "no-snapshot";

pub trait SearchEngine: Send + Sync {
    /// The corpus snapshot the next search would serve from
    /// (#9943): the job manager binds every result-cache entry
    /// to the snapshot current at submit time, and a changed
    /// snapshot invalidates all entries at once. Engines whose
    /// backend carries no shard set report [`NO_SNAPSHOT_ID`].
    fn snapshot_id(&self) -> String {
        NO_SNAPSHOT_ID.to_string()
    }

    /// The effective filter-surface manifest the next search
    /// would serve under (#6228 Phase 4): the job manager binds
    /// every cache key to its `(version, content_hash)`, so a
    /// surface change invalidates coalescing and result reuse
    /// exactly like a snapshot change. Engines with no manifest
    /// surface report the built-in manifest.
    fn surface_manifest(&self) -> Manifest {
        builtin_manifest()
    }

    fn search(
        &self,
        request: &SearchRequest,
        filters: &Filters,
        branch_states: &Arc<Mutex<BranchStates>>,
    ) -> Result<Json, ServerError>;

    /// Ceiling-checked get-chunk: a chunk whose effective
    /// sensitivity exceeds `ceiling` — including one whose
    /// sensitivity is unknown — returns the same 404 shape as a
    /// missing chunk.
    fn chunk(
        &self,
        chunk_id: i64,
        neighbors: usize,
        ceiling: ProtoSensitivity,
    ) -> Result<Json, ServerError>;

    /// Embed one query text into the production query-vector
    /// space (#6788): the Cloudflare search Worker's miss path
    /// stores this vector in the query's Durable Object before
    /// searching. The default refuses; engines with a resident
    /// embedder override it.
    fn embed_query(&self, _text: &str, _trunc: usize) -> Result<Vec<f32>, ServerError> {
        Err(ServerError::new(
            501,
            "embed_unavailable",
            "this engine does not serve embeddings",
        ))
    }

    fn embedder_state(&self) -> String;
}

/// The production engine: atlas-core over a shared backend and a
/// shared embedder.
pub struct CoreSearchEngine<B, E> {
    backend: Arc<B>,
    embedder: Arc<E>,
    embedder_state: Arc<dyn Fn() -> String + Send + Sync>,
}

impl<B, E> CoreSearchEngine<B, E> {
    pub fn new(
        backend: Arc<B>,
        embedder: Arc<E>,
        embedder_state: Arc<dyn Fn() -> String + Send + Sync>,
    ) -> CoreSearchEngine<B, E> {
        CoreSearchEngine {
            backend,
            embedder,
            embedder_state,
        }
    }
}

impl<B, E> SearchEngine for CoreSearchEngine<B, E>
where
    B: SearchBackend + Send + Sync,
    E: QueryEmbedder + Send + Sync,
{
    fn search(
        &self,
        request: &SearchRequest,
        filters: &Filters,
        branch_states: &Arc<Mutex<BranchStates>>,
    ) -> Result<Json, ServerError> {
        let opts = SearchOptions {
            query: request.query.clone(),
            mode: core_mode(request.mode),
            k: request.k,
            pool: request.pool,
            group_by_session: request.group_by_session,
            expand: request.expand,
            with_message: request.with_message,
            model: request.model.clone(),
            embedder_label: request.embedder.clone(),
            trunc: request.trunc,
            vector_backend: core_vector_backend(request.vector_backend),
            parameters: request.parameters.clone(),
            context: request_context(request),
        };
        let executor = ConcurrentBranchExecutor {
            backend: self.backend.as_ref(),
            branch_states: branch_states.clone(),
        };
        // Trace capture (issue #9973): fail-closed — a trace
        // write failure fails the search. The service surface
        // does not retain raw query text (hash only).
        let trace_sink = JsonlTraceSink::new(JsonlTraceSink::default_dir());
        let trace_ctx = TraceContext::new("service-api", "agent", "atlas-service");
        run_search_with_executor_traced(
            self.backend.as_ref(),
            self.embedder.as_ref(),
            filters,
            &opts,
            &executor,
            &trace_sink,
            trace_ctx,
        )
        .map_err(|e| ServerError::from_core(&e))
    }

    fn chunk(
        &self,
        chunk_id: i64,
        neighbors: usize,
        ceiling: ProtoSensitivity,
    ) -> Result<Json, ServerError> {
        let not_found = || {
            ServerError::new(
                404,
                "chunk_not_found",
                format!("chunk {chunk_id} not found"),
            )
        };
        let details = self
            .backend
            .chunk_details(&[chunk_id])
            .map_err(|e| ServerError::from_core(&e))?;
        let Some(detail) = details.iter().find(|d| d.chunk_id == chunk_id) else {
            return Err(not_found());
        };
        let allowed = detail
            .eff_sensitivity
            .as_deref()
            .and_then(CoreSensitivity::from_name)
            .map(|s| s <= core_sensitivity(ceiling))
            .unwrap_or(false);
        // Excluded content (#12404): `excluded_personal_life`
        // is ineligible in every disclosure context. The label
        // is not a ladder value, so the check above already
        // refuses it; this states the rule explicitly so it
        // survives any change to the label mapping.
        let excluded = chunk_classification(detail.eff_sensitivity.as_deref()).is_excluded();
        if !allowed || excluded {
            return Err(not_found());
        }
        // The explicit-id gate keeps its documented form (a
        // by-id lookup with no caller ceiling; the LocalOwner
        // context applies, exactly like the Python), while the
        // caller's actual ceiling is threaded through so the
        // occurrence provenance is bounded by it — never by the
        // destination context's standalone ceiling (P1: a
        // personal-capped caller must not receive secret-tier
        // source locators, and a personal-only service must not
        // request a secret-tier fan-out).
        run_get_chunk_with_context(
            self.backend.as_ref(),
            chunk_id,
            neighbors,
            DisclosureContext::LocalOwner,
            core_sensitivity(ceiling),
        )
        .map_err(|e| ServerError::from_core(&e))
    }

    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>, ServerError> {
        self.embedder
            .embed_query(text, trunc)
            .map_err(|e| ServerError::from_core(&e))
    }

    fn snapshot_id(&self) -> String {
        self.backend
            .snapshot_id()
            .unwrap_or_else(|| NO_SNAPSHOT_ID.to_string())
    }

    fn surface_manifest(&self) -> Manifest {
        self.backend.manifest()
    }

    fn embedder_state(&self) -> String {
        (self.embedder_state)()
    }
}

/// One shared embedder handle with observable load state, for
/// `/health`. The wrapped embedder is the CLI's lazy
/// `CliEmbedder` in production (one `AtlasEmbedder`, mutex-held,
/// behind it); the ONNX session loads on the first vector query
/// and is shared by every later one.
pub struct SharedEmbedder {
    inner: Arc<dyn QueryEmbedder + Send + Sync>,
    state: Arc<Mutex<String>>,
}

impl SharedEmbedder {
    pub fn new(inner: Arc<dyn QueryEmbedder + Send + Sync>) -> SharedEmbedder {
        SharedEmbedder {
            inner,
            state: Arc::new(Mutex::new("cold".to_string())),
        }
    }

    pub fn state_fn(&self) -> Arc<dyn Fn() -> String + Send + Sync> {
        let state = self.state.clone();
        Arc::new(move || {
            state
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "error".to_string())
        })
    }
}

impl QueryEmbedder for SharedEmbedder {
    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>, CoreError> {
        if let Ok(mut s) = self.state.lock()
            && *s == "cold"
        {
            *s = "warming".to_string();
        }
        let out = self.inner.embed_query(text, trunc);
        if let Ok(mut s) = self.state.lock() {
            *s = if out.is_ok() {
                "ready".to_string()
            } else {
                "error".to_string()
            };
        }
        out
    }
}

/// Runs the FTS and vector branches concurrently (design section
/// 2: "workers run branches concurrently; fusion runs once over
/// complete branch pools"). Each branch is a tailored
/// `branch_pools` call carrying only its own inputs; atlas-core
/// fuses the combined pools through the shared orchestration, so
/// result semantics are exactly the standalone ones.
struct ConcurrentBranchExecutor<'a, B> {
    backend: &'a B,
    branch_states: Arc<Mutex<BranchStates>>,
}

impl<B> ConcurrentBranchExecutor<'_, B>
where
    B: SearchBackend + Sync,
{
    fn set_state(&self, vector: bool, state: BranchState) {
        if let Ok(mut states) = self.branch_states.lock() {
            if vector {
                states.vector = state;
            } else {
                states.fts = state;
            }
        }
    }

    fn run_branch(
        &self,
        vector: bool,
        query: &BranchQuery,
        filters: &Filters,
    ) -> Result<Vec<atlas_core::types::BranchHit>, CoreError> {
        self.set_state(vector, BranchState::Running);
        let out = self
            .backend
            .branch_pools(query, filters)
            .map(|p| if vector { p.vector } else { p.fts });
        self.set_state(
            vector,
            if out.is_ok() {
                BranchState::Done
            } else {
                BranchState::Failed
            },
        );
        out
    }
}

impl<B> BranchPoolExecutor for ConcurrentBranchExecutor<'_, B>
where
    B: SearchBackend + Sync,
{
    fn branch_pools(
        &self,
        query: &BranchQuery,
        filters: &Filters,
    ) -> Result<BranchPools, CoreError> {
        let uses_fts = query.match_expr.is_some();
        let uses_vector = query.query_vector.is_some();
        if let Ok(mut states) = self.branch_states.lock() {
            states.fts = if uses_fts {
                BranchState::Pending
            } else {
                BranchState::Skipped
            };
            states.vector = if uses_vector {
                BranchState::Pending
            } else {
                BranchState::Skipped
            };
        }
        let fts_query = BranchQuery {
            match_expr: query.match_expr.clone(),
            query_vector: None,
            pool: query.pool,
            shard_top_k: query.shard_top_k,
            global_top_k: query.global_top_k,
            vector_backend: query.vector_backend,
        };
        let vector_query = BranchQuery {
            match_expr: None,
            query_vector: query.query_vector.clone(),
            pool: query.pool,
            shard_top_k: query.shard_top_k,
            global_top_k: query.global_top_k,
            vector_backend: query.vector_backend,
        };
        // Fast path (Drew's usage profile, 2026-11-04): the
        // primary real-world job is laptop-local, unfiltered,
        // vector-only on an index backend. It has exactly one
        // branch, so execute it inline on the job worker — no FTS
        // branch, no extra thread hop, one branch-slot reservation.
        // FTS-only jobs take the same single-branch path.
        if uses_fts ^ uses_vector {
            let mut pools = BranchPools::default();
            if uses_fts {
                pools.fts = self.run_branch(false, &fts_query, filters)?;
            } else {
                pools.vector = self.run_branch(true, &vector_query, filters)?;
            }
            return Ok(pools);
        }
        std::thread::scope(|scope| {
            let fts_handle =
                uses_fts.then(|| scope.spawn(|| self.run_branch(false, &fts_query, filters)));
            let vector_handle =
                uses_vector.then(|| scope.spawn(|| self.run_branch(true, &vector_query, filters)));
            let mut pools = BranchPools::default();
            if let Some(h) = fts_handle {
                pools.fts = h
                    .join()
                    .map_err(|_| CoreError::backend("fts branch worker panicked"))??;
            }
            if let Some(h) = vector_handle {
                pools.vector = h
                    .join()
                    .map_err(|_| CoreError::backend("vector branch worker panicked"))??;
            }
            Ok(pools)
        })
    }
}

// --------------------------------------------------------- job store ----
//
// Dedupe model (Drew's ruling, 2026-11-04): most real queries are
// the same unfiltered search repeated, so execution is shared
// through a result cache. The unit of execution is a
// **Computation**, keyed by the EFFECTIVE request — everything
// about the search after the caller's ceiling has been clamped:
//
//   (query, mode, k, pool, group_by_session, expand, with_message,
//    model, embedder, trunc, vector_backend,
//    kind[], topic[], project[], client[], since, until,
//    effective_ceiling, disclosure_context)
//
// Exact-key sharing only: the effective ceiling is IN the key, so
// a lower-ceiling request can never be served from a
// higher-ceiling entry; the disclosure context is in the key for
// the same reason (#12404) — a payload filtered for one
// destination surface is never served to another; filters are in the key, so a filtered
// request is never answered by subsetting an unfiltered ranking
// (or vice versa). An identical submit while a matching
// computation is queued/running COALESCES onto it (no second
// branch execution); an identical submit within the stored
// entry's TTL returns the ready result. Every submitter still
// gets its own job handle — job id + capability token — over the
// shared immutable ranking, so listing, pickup, and per-job
// authorization work exactly as if it had computed alone. The
// ranking's TTL runs from the shared computation's completion.

/// The dedupe cache key: the effective request, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    /// Corpus snapshot at submit time (#9943): entries computed
    /// under a different snapshot never coalesce or serve.
    snapshot: String,
    /// The design's sha256 result-cache key (atlas-proto
    /// `result_cache_key`): sha256(query || snapshot || ceiling
    /// || flags). Stored alongside the verbatim fields so the
    /// snapshot-bound result cache and the dedupe cache agree on
    /// exactly which searches share an outcome.
    result_key: String,
    query: String,
    mode: ProtoSearchMode,
    k: usize,
    pool: usize,
    group_by_session: bool,
    expand: usize,
    parameters: String,
    with_message: bool,
    model: String,
    embedder: String,
    trunc: usize,
    vector_backend: ProtoVectorBackend,
    kind: Vec<String>,
    topic: Vec<String>,
    project: Vec<String>,
    client: Vec<String>,
    since: Option<String>,
    until: Option<String>,
    /// Generic dimension filters (#6228 Phase 3), in the
    /// dimension map's sorted (BTreeMap) order: a request filtered
    /// on a generic dimension never coalesces with one that is
    /// not, exactly like the named filters above.
    dimensions: Vec<(String, Vec<String>)>,
    /// The effective filter surface's identity (#6228 Phase 4,
    /// design note §7): `(manifest_version, content_hash)` of the
    /// manifest the serving backend loaded. Entries computed
    /// under a different surface never coalesce or serve, even on
    /// the same corpus snapshot.
    manifest_version: i64,
    content_hash: String,
    effective_ceiling: ProtoSensitivity,
    /// The resolved disclosure context (#12404): cached and
    /// coalesced payloads are context-filtered, so the context
    /// is part of the effective request exactly like the
    /// ceiling.
    context: DisclosureContext,
}

impl CacheKey {
    fn from_request(
        req: &SearchRequest,
        effective_ceiling: ProtoSensitivity,
        snapshot: &str,
        surface: &Manifest,
    ) -> CacheKey {
        CacheKey {
            snapshot: snapshot.to_string(),
            result_key: req.parameters.result_cache_key(
                &req.query,
                snapshot,
                effective_ceiling.name(),
                req.expand,
                req.group_by_session,
                req.with_message,
            ),
            query: req.query.clone(),
            mode: req.mode,
            k: req.k,
            pool: req.pool,
            group_by_session: req.group_by_session,
            expand: req.expand,
            // The shared canonical constructor (atlas-proto):
            // the same material the result-cache key hashes, so
            // the dedupe key and every outcome cache agree on
            // exactly which flags distinguish two searches.
            parameters: req.parameters.canonical_key_material(
                req.expand,
                req.group_by_session,
                req.with_message,
            ),
            with_message: req.with_message,
            model: req.model.clone(),
            embedder: req.embedder.clone(),
            trunc: req.trunc,
            vector_backend: req.vector_backend,
            kind: req.filters.kind.clone(),
            topic: req.filters.topic.clone(),
            project: req.filters.project.clone(),
            client: req.filters.client.clone(),
            since: req.filters.since.clone(),
            until: req.filters.until.clone(),
            dimensions: req
                .filters
                .dimensions
                .iter()
                .map(|(name, values)| (name.clone(), values.clone()))
                .collect(),
            manifest_version: surface.version,
            content_hash: surface.content_hash.clone(),
            effective_ceiling,
            context: request_context(req),
        }
    }
}

#[derive(Default)]
struct ComputationState {
    status: JobStatus,
    started: Option<(Instant, u64)>,
    completed: Option<(Instant, u64)>,
    result: Option<Json>,
    notes: Vec<String>,
    error: Option<String>,
}

/// One shared execution + its immutable stored ranking.
struct Computation {
    id: String,
    #[allow(dead_code)]
    key: CacheKey,
    /// The client whose submission created the computation (queue
    /// accounting attributes the execution to it; coalesced
    /// handles from other clients do not consume queue slots).
    owner_name: String,
    request: SearchRequest,
    filters: Filters,
    effective_ceiling: ProtoSensitivity,
    demand: usize,
    submitted: Instant,
    submitted_at_ms: u64,
    branch_states: Arc<Mutex<BranchStates>>,
    state: Mutex<ComputationState>,
}

impl Computation {
    fn status(&self, limits: &RuntimeLimits) -> JobStatus {
        let state = self.state.lock().unwrap();
        match state.status {
            JobStatus::Done => {
                let expired = state
                    .completed
                    .map(|(t, _)| Instant::now().duration_since(t) >= limits.job_ttl)
                    .unwrap_or(false);
                if expired {
                    JobStatus::Expired
                } else {
                    JobStatus::Done
                }
            }
            other => other,
        }
    }

    fn expires_at_ms(&self, limits: &RuntimeLimits) -> u64 {
        // TTL runs from the shared computation's completion.
        let state = self.state.lock().unwrap();
        match state.completed {
            Some((t, _)) => {
                instant_ms(t, self.submitted, self.submitted_at_ms) + duration_ms(limits.job_ttl)
            }
            None => self.submitted_at_ms + duration_ms(limits.hard_ttl),
        }
    }
}

/// A per-submission handle over a shared computation. Handles are
/// immutable after creation; all mutable state lives on the
/// computation.
pub(crate) struct JobRecord {
    job_id: String,
    owner_name: String,
    owner_token: String,
    job_token: String,
    computation: Arc<Computation>,
    submitted: Instant,
    submitted_at_ms: u64,
}

impl JobRecord {
    fn status(&self, limits: &RuntimeLimits) -> JobStatus {
        self.computation.status(limits)
    }

    fn status_response(
        &self,
        limits: &RuntimeLimits,
        queue_depth: usize,
        queue_position: Option<usize>,
    ) -> JobStatusResponse {
        let comp = &self.computation;
        let state = comp.state.lock().unwrap();
        // Derive status and expiry from the ALREADY-LOCKED state:
        // self.status() and comp.expires_at_ms() both re-lock
        // this mutex and would deadlock the handler.
        let status = match state.status {
            JobStatus::Done => {
                let expired = state
                    .completed
                    .map(|(t, _)| Instant::now().duration_since(t) >= limits.job_ttl)
                    .unwrap_or(false);
                if expired {
                    JobStatus::Expired
                } else {
                    JobStatus::Done
                }
            }
            other => other,
        };
        let expires_at_ms = match state.completed {
            Some((t, _)) => {
                instant_ms(t, comp.submitted, comp.submitted_at_ms) + duration_ms(limits.job_ttl)
            }
            None => comp.submitted_at_ms + duration_ms(limits.hard_ttl),
        };
        let branch_states = comp.branch_states.lock().map(|s| *s).unwrap_or_default();
        JobStatusResponse {
            job_id: self.job_id.clone(),
            status,
            branch_states,
            effective_ceiling: comp.effective_ceiling,
            pool: comp.request.pool,
            queue_depth,
            queue_position,
            notes: state.notes.clone(),
            error: state.error.clone(),
            result_count: state
                .result
                .as_ref()
                .and_then(|env| env.get("results").and_then(Json::as_array).map(|r| r.len())),
            submitted_at_ms: self.submitted_at_ms,
            started_at_ms: state
                .started
                .map(|(t, _)| instant_ms(t, comp.submitted, comp.submitted_at_ms)),
            completed_at_ms: state
                .completed
                .map(|(t, _)| instant_ms(t, comp.submitted, comp.submitted_at_ms)),
            expires_at_ms,
        }
    }
}

#[derive(Default)]
struct ManagerInner {
    /// Per-submission handles by job id.
    jobs: HashMap<String, Arc<JobRecord>>,
    /// Live computations by computation id.
    computations: HashMap<String, Arc<Computation>>,
    /// The dedupe cache: effective-request key -> computation.
    cache: HashMap<CacheKey, Arc<Computation>>,
    /// The snapshot-bound result cache (#9943, design #5667
    /// s3.2): completed rankings keyed by the same snapshot-bound
    /// key as the dedupe cache, held for `result_cache_ttl`
    /// (24h by default) — far beyond a job handle's own TTL, so a
    /// repeat submit on an unchanged snapshot is served with no
    /// branch execution at all. In-service memory only (#5354
    /// privacy rule): nothing is written to disk or shared
    /// across processes.
    result_cache: HashMap<CacheKey, ResultCacheEntry>,
    /// The snapshot the result cache's entries belong to. When a
    /// submit observes a different snapshot, every entry is
    /// dropped at once (key change makes them unreachable; this
    /// is the eager GC of that invalidation).
    result_cache_snapshot: Option<String>,
    /// FIFO queue of computation ids awaiting branch budget.
    queue: VecDeque<String>,
    active_branch_executions: usize,
}

/// One completed ranking in the snapshot-bound result cache.
struct ResultCacheEntry {
    /// The immutable result envelope, exactly as computed.
    envelope: Json,
    /// When the producing computation completed. The entry is a
    /// GC candidate once this is older than `result_cache_ttl`.
    stored_at: Instant,
}

/// The job scheduler + in-memory temp store + dedupe cache. All
/// state is behind one mutex; execution happens on spawned
/// threads outside the lock. Nothing here touches the filesystem.
pub struct JobManager {
    engine: Arc<dyn SearchEngine>,
    limits: RuntimeLimits,
    inner: Mutex<ManagerInner>,
    shutdown: AtomicBool,
}

pub struct ManagerStats {
    pub queue_depth: usize,
    pub active_branch_executions: usize,
    pub active_jobs: usize,
}

impl JobManager {
    pub fn new(engine: Arc<dyn SearchEngine>, limits: RuntimeLimits) -> Arc<JobManager> {
        Arc::new(JobManager {
            engine,
            limits,
            inner: Mutex::new(ManagerInner::default()),
            shutdown: AtomicBool::new(false),
        })
    }

    pub fn start_sweeper(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || {
            while let Some(manager) = weak.upgrade() {
                if manager.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                manager.sweep();
                drop(manager);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
    }

    pub fn stats(&self) -> ManagerStats {
        let inner = self.inner.lock().unwrap();
        let active_jobs = inner
            .computations
            .values()
            .filter(|c| {
                matches!(
                    c.state.lock().map(|s| s.status),
                    Ok(JobStatus::Queued | JobStatus::Running)
                )
            })
            .count();
        ManagerStats {
            queue_depth: inner.queue.len(),
            active_branch_executions: inner.active_branch_executions,
            active_jobs,
        }
    }

    /// (running, queued) COMPUTATIONS attributed to a client.
    fn computation_counts(inner: &ManagerInner, client: &str) -> (usize, usize) {
        let mut running = 0;
        let mut queued = 0;
        for comp in inner.computations.values() {
            if comp.owner_name != client {
                continue;
            }
            match comp.state.lock().map(|s| s.status) {
                Ok(JobStatus::Running) => running += 1,
                Ok(JobStatus::Queued) => queued += 1,
                _ => {}
            }
        }
        (running, queued)
    }

    /// Drop a computation's stored result once its TTL (from
    /// completion) has run, and evict finished computations from
    /// the cache/maps once handles have had an hour past the hard
    /// cap to observe the expiry. Handles keep their own Arc, so
    /// eviction never breaks a status read — it reports Expired.
    fn refresh_computation(&self, inner: &mut ManagerInner, comp: &Arc<Computation>) {
        let now = Instant::now();
        let mut clear_result = false;
        let mut evict = false;
        {
            let mut state = comp.state.lock().unwrap();
            if matches!(state.status, JobStatus::Done | JobStatus::Failed)
                && let Some((t, _)) = state.completed
                && now.duration_since(t) >= self.limits.job_ttl
            {
                state.result = None;
                clear_result = true;
            }
            if matches!(state.status, JobStatus::Done | JobStatus::Failed)
                && now.duration_since(comp.submitted)
                    >= self.limits.hard_ttl + Duration::from_secs(3600)
            {
                evict = true;
            }
        }
        let _ = clear_result;
        if evict {
            inner.computations.remove(&comp.id);
            inner.cache.retain(|_, c| c.id != comp.id);
        }
    }

    /// Handle-level refresh: handles an hour past the hard cap are
    /// removed entirely (their status reads become the same
    /// expired-or-unknown shape a restart produces).
    fn refresh_job(&self, inner: &mut ManagerInner, job_id: &str) -> bool {
        let Some(job) = inner.jobs.get(job_id).cloned() else {
            return false;
        };
        self.refresh_computation(inner, &job.computation);
        if Instant::now().duration_since(job.submitted)
            >= self.limits.hard_ttl + Duration::from_secs(3600)
        {
            inner.jobs.remove(job_id);
            return false;
        }
        true
    }

    fn sweep(self: &Arc<Self>) {
        let spawned = {
            let mut inner = self.inner.lock().unwrap();
            // Result-cache GC (#9943): the 24h TTL is a memory
            // bound only — entries past it are dropped here and
            // on submit; correctness comes from the snapshot key.
            let gc_now = Instant::now();
            inner
                .result_cache
                .retain(|_, e| gc_now.duration_since(e.stored_at) < self.limits.result_cache_ttl);
            let comps: Vec<Arc<Computation>> = inner.computations.values().cloned().collect();
            for comp in &comps {
                self.refresh_computation(&mut inner, comp);
            }
            let ids: Vec<String> = inner.jobs.keys().cloned().collect();
            for id in ids {
                self.refresh_job(&mut inner, &id);
            }
            self.dispatch_locked(&mut inner)
        };
        self.spawn_jobs(spawned);
    }

    /// Admit queued computations in strict FIFO order while
    /// per-client running caps and the global branch budget
    /// allow. Only the head of the queue is considered: a later
    /// computation never overtakes an earlier one.
    fn dispatch_locked(&self, inner: &mut ManagerInner) -> Vec<Arc<Computation>> {
        let mut spawned = Vec::new();
        while let Some(comp_id) = inner.queue.front().cloned() {
            let Some(comp) = inner.computations.get(&comp_id).cloned() else {
                inner.queue.pop_front();
                continue;
            };
            // Read the head computation's shape, then DROP its
            // state lock before counting: computation_counts locks
            // every computation's state (including this one), and
            // a std Mutex held across that call deadlocks.
            let (status, owner, demand) = {
                let state = comp.state.lock().unwrap();
                (state.status, comp.owner_name.clone(), comp.demand)
            };
            if status != JobStatus::Queued {
                inner.queue.pop_front();
                continue;
            }
            let (running, _) = JobManager::computation_counts(inner, &owner);
            let can_run = running < self.limits.max_concurrent_jobs_per_client
                && inner.active_branch_executions + demand
                    <= self.limits.max_concurrent_branch_executions;
            if !can_run {
                break;
            }
            {
                let mut state = comp.state.lock().unwrap();
                if state.status != JobStatus::Queued {
                    continue;
                }
                state.status = JobStatus::Running;
                state.started = Some((Instant::now(), now_ms()));
            }
            inner.active_branch_executions += demand;
            inner.queue.pop_front();
            spawned.push(comp);
        }
        spawned
    }

    fn spawn_jobs(self: &Arc<Self>, comps: Vec<Arc<Computation>>) {
        for comp in comps {
            let manager = self.clone();
            std::thread::spawn(move || manager.execute_computation(comp));
        }
    }

    fn execute_computation(self: &Arc<Self>, comp: Arc<Computation>) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.engine
                .search(&comp.request, &comp.filters, &comp.branch_states)
        }));
        let mut completed_envelope: Option<Json> = None;
        {
            let mut state = comp.state.lock().unwrap();
            if state.status == JobStatus::Running {
                match outcome {
                    Ok(Ok(envelope)) => {
                        completed_envelope = Some(envelope.clone());
                        state.notes = envelope
                            .get("notes")
                            .and_then(Json::as_array)
                            .map(|notes| {
                                notes
                                    .iter()
                                    .filter_map(|n| n.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        state.result = Some(envelope);
                        state.status = JobStatus::Done;
                    }
                    Ok(Err(e)) => {
                        state.status = JobStatus::Failed;
                        state.error = Some(e.body.error.message.clone());
                    }
                    Err(_) => {
                        state.status = JobStatus::Failed;
                        state.error = Some("search worker panicked".to_string());
                    }
                }
                state.completed = Some((Instant::now(), now_ms()));
            }
        }
        if let Some(envelope) = completed_envelope {
            // Snapshot-bound result cache write (#9943): every
            // completed ranking lands here, including a --no-cache
            // computation's — the flag bypasses reads and REFRESHES
            // writes. The entry is stored only while its snapshot
            // is still the cache's partition; a ranking computed
            // under a snapshot that has since rotated is dropped
            // instead of polluting the new partition (its key
            // would be unreachable either way).
            let mut inner = self.inner.lock().unwrap();
            if inner.result_cache_snapshot.as_deref() == Some(comp.key.snapshot.as_str()) {
                inner.result_cache.insert(
                    comp.key.clone(),
                    ResultCacheEntry {
                        envelope,
                        stored_at: Instant::now(),
                    },
                );
            }
        }
        if matches!(comp.state.lock().map(|s| s.status), Ok(JobStatus::Failed)) {
            // A failed computation must not poison the cache: an
            // identical later submit retries instead of attaching
            // to the failure. Existing handles keep the failure.
            let mut inner = self.inner.lock().unwrap();
            inner.cache.retain(|_, c| c.id != comp.id);
        }
        let spawned = {
            let mut inner = self.inner.lock().unwrap();
            inner.active_branch_executions =
                inner.active_branch_executions.saturating_sub(comp.demand);
            self.dispatch_locked(&mut inner)
        };
        self.spawn_jobs(spawned);
    }

    /// Submit a job for an authenticated client. Applies request
    /// validation and the ceiling clamp, then resolves the
    /// dedupe cache: an identical effective request that is
    /// queued, running, or within its result TTL coalesces onto
    /// the existing computation (no new branch execution, no
    /// queue slot consumed). Only a genuinely new computation
    /// passes through admission (bounded FIFO queue, 429 +
    /// Retry-After beyond capacity). Either way the submitter
    /// gets its own job handle immediately.
    pub fn submit(
        self: &Arc<Self>,
        client: &ClientConfig,
        request: SearchRequest,
    ) -> Result<JobCreated, ServerError> {
        validate_request(&request)?;
        let effective = request.ceiling.clamped_to(client.ceiling_cap);
        // Bind this submit to the corpus snapshot it would serve
        // from (#9943): the key — dedupe and result cache alike —
        // carries the snapshot, so nothing computed under another
        // snapshot can coalesce onto or answer this request.
        let snapshot = self.engine.snapshot_id();
        // Bind this submit to the effective filter surface too
        // (#6228 Phase 4): the key carries the loaded manifest's
        // (version, content_hash), so a manifest change — a new
        // generic dimension, a changed shape — can never be
        // answered from entries computed under the old surface.
        let surface = self.engine.surface_manifest();
        let key = CacheKey::from_request(&request, effective, &snapshot, &surface);
        let job_id = new_job_id().map_err(|m| ServerError::new(500, "internal", m))?;
        let job_token = new_job_token().map_err(|m| ServerError::new(500, "internal", m))?;
        let now = Instant::now();
        let submitted_at_ms = now_ms();

        let pre_spawn = {
            let mut inner = self.inner.lock().unwrap();
            let comps: Vec<Arc<Computation>> = inner.computations.values().cloned().collect();
            for comp in &comps {
                self.refresh_computation(&mut inner, comp);
            }
            self.dispatch_locked(&mut inner)
        };
        self.spawn_jobs(pre_spawn);

        let (handle, spawned, queue_depth, queue_position) = {
            let mut inner = self.inner.lock().unwrap();
            // Snapshot rotation (#9943): the result cache is
            // partitioned by snapshot. A submit that observes a
            // new snapshot drops every entry at once — they name
            // a corpus state that no longer exists — and starts
            // the new partition. (Dedupe entries from the old
            // snapshot are unreachable by key and expire on their
            // own TTL.)
            if inner.result_cache_snapshot.as_deref() != Some(key.snapshot.as_str()) {
                if inner.result_cache_snapshot.is_some() {
                    inner.result_cache.clear();
                }
                inner.result_cache_snapshot = Some(key.snapshot.clone());
            }
            // Cache resolution. An entry is attachable while it
            // is queued/running, or done within its TTL. Failed
            // entries were already evicted at completion; expired
            // ones are evicted here and recomputed.
            // --no-cache (#9942): skip the read entirely; the
            // fresh computation still lands in the cache via
            // new_computation, so the flag REFRESHES the entry.
            let cached = if request.parameters.no_cache {
                None
            } else {
                inner.cache.get(&key).cloned()
            };
            let comp = match cached {
                Some(comp) if comp.status(&self.limits) != JobStatus::Expired => comp,
                Some(comp) => {
                    inner.cache.remove(&key);
                    inner.computations.remove(&comp.id);
                    self.result_cache_or_new(&mut inner, client, &request, effective, &key)?
                }
                None => self.result_cache_or_new(&mut inner, client, &request, effective, &key)?,
            };
            let handle = Arc::new(JobRecord {
                job_id: job_id.clone(),
                owner_name: client.name.clone(),
                owner_token: client.token.clone(),
                job_token: job_token.clone(),
                computation: comp.clone(),
                submitted: now,
                submitted_at_ms,
            });
            inner.jobs.insert(job_id.clone(), handle.clone());
            let spawned = self.dispatch_locked(&mut inner);
            let queue_position = inner.queue.iter().position(|id| id == &comp.id);
            (handle, spawned, inner.queue.len(), queue_position)
        };
        self.spawn_jobs(spawned);
        Ok(JobCreated {
            job_id,
            job_token,
            status: handle.status(&self.limits),
            effective_ceiling: effective,
            queue_depth,
            queue_position,
            submitted_at_ms,
            expires_at_ms: handle.computation.expires_at_ms(&self.limits),
        })
    }

    /// Resolve a submit that missed the dedupe cache: serve the
    /// snapshot-bound result cache when it holds a fresh entry
    /// for this exact key, else create a fresh computation.
    /// A result-cache hit becomes a NEW computation that is
    /// already Done over the cached envelope — its own job
    /// handle, its own job TTL — with no queue slot and no
    /// branch execution. --no-cache bypasses this read (its
    /// fresh computation refreshes the entry on completion).
    fn result_cache_or_new(
        &self,
        inner: &mut ManagerInner,
        client: &ClientConfig,
        request: &SearchRequest,
        effective: ProtoSensitivity,
        key: &CacheKey,
    ) -> Result<Arc<Computation>, ServerError> {
        if !request.parameters.no_cache
            && let Some(entry) = inner.result_cache.get(key)
        {
            if Instant::now().duration_since(entry.stored_at) < self.limits.result_cache_ttl {
                let envelope = entry.envelope.clone();
                let notes = envelope
                    .get("notes")
                    .and_then(Json::as_array)
                    .map(|notes| {
                        notes
                            .iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let now = Instant::now();
                let now_ms_val = now_ms();
                let comp = Arc::new(Computation {
                    id: new_job_id().map_err(|m| ServerError::new(500, "internal", m))?,
                    key: key.clone(),
                    owner_name: client.name.clone(),
                    request: request.clone(),
                    filters: build_filters(request, effective)?,
                    effective_ceiling: effective,
                    demand: request.mode.branch_demand(),
                    submitted: now,
                    submitted_at_ms: now_ms_val,
                    branch_states: Arc::new(Mutex::new(BranchStates::default())),
                    state: Mutex::new(ComputationState {
                        status: JobStatus::Done,
                        started: Some((now, now_ms_val)),
                        completed: Some((now, now_ms_val)),
                        result: Some(envelope),
                        notes,
                        error: None,
                    }),
                });
                inner.cache.insert(key.clone(), comp.clone());
                inner.computations.insert(comp.id.clone(), comp.clone());
                return Ok(comp);
            }
            inner.result_cache.remove(key);
        }
        self.new_computation(inner, client, request, effective, key)
    }

    /// Create + enqueue a fresh computation (admission-checked).
    /// Called with the manager lock held; on a 429 nothing is
    /// inserted anywhere.
    fn new_computation(
        &self,
        inner: &mut ManagerInner,
        client: &ClientConfig,
        request: &SearchRequest,
        effective: ProtoSensitivity,
        key: &CacheKey,
    ) -> Result<Arc<Computation>, ServerError> {
        let (_running, queued_for_client) = JobManager::computation_counts(inner, &client.name);
        if queued_for_client >= self.limits.max_queued_jobs_per_client
            || inner.queue.len() >= self.limits.max_queued_jobs
        {
            return Err(ServerError::rate_limited(
                self.limits.retry_after_seconds,
                inner.queue.len(),
            ));
        }
        let filters = build_filters(request, effective)?;
        let comp = Arc::new(Computation {
            id: new_job_id().map_err(|m| ServerError::new(500, "internal", m))?,
            key: key.clone(),
            owner_name: client.name.clone(),
            request: request.clone(),
            filters,
            effective_ceiling: effective,
            demand: request.mode.branch_demand(),
            submitted: Instant::now(),
            submitted_at_ms: now_ms(),
            branch_states: Arc::new(Mutex::new(BranchStates::default())),
            state: Mutex::new(ComputationState {
                status: JobStatus::Queued,
                ..ComputationState::default()
            }),
        });
        inner.cache.insert(key.clone(), comp.clone());
        inner.computations.insert(comp.id.clone(), comp.clone());
        inner.queue.push_back(comp.id.clone());
        Ok(comp)
    }

    /// One-shot convenience: submit, wait, first page — through
    /// exactly the job machinery above (dedupe included).
    pub fn one_shot(
        self: &Arc<Self>,
        client: &ClientConfig,
        request: SearchRequest,
    ) -> Result<SearchPage, ServerError> {
        let page_limit = request.page_limit.unwrap_or(request.k).max(1);
        let created = self.submit(client, request)?;
        let job = self
            .job_for_client(&created.job_id, client)
            .map_err(|_| ServerError::job_expired())?;
        let deadline = Instant::now() + Duration::from_secs(330);
        loop {
            let status = {
                let mut inner = self.inner.lock().unwrap();
                self.refresh_job(&mut inner, &created.job_id);
                job.status(&self.limits)
            };
            match status {
                JobStatus::Done => {
                    return self.page(&job, None, page_limit);
                }
                JobStatus::Failed => {
                    let msg = job
                        .computation
                        .state
                        .lock()
                        .unwrap()
                        .error
                        .clone()
                        .unwrap_or_else(|| "search failed".to_string());
                    return Err(ServerError::new(500, "job_failed", msg));
                }
                JobStatus::Expired => return Err(ServerError::job_gone()),
                JobStatus::Queued | JobStatus::Running => {
                    if Instant::now() >= deadline {
                        return Err(ServerError::new(
                            504,
                            "job_timeout",
                            "one-shot search did not complete within 330 seconds; the job continues and can be polled by id",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    pub fn job_for_client(
        &self,
        job_id: &str,
        client: &ClientConfig,
    ) -> Result<Arc<JobRecord>, ServerError> {
        let job = {
            let mut inner = self.inner.lock().unwrap();
            if !self.refresh_job(&mut inner, job_id) {
                return Err(ServerError::job_expired());
            }
            inner.jobs.get(job_id).cloned()
        };
        let Some(job) = job else {
            return Err(ServerError::job_expired());
        };
        if job.owner_name == client.name && secure_eq(&job.owner_token, &client.token) {
            Ok(job)
        } else {
            Err(ServerError::forbidden("job belongs to a different client"))
        }
    }

    /// Authorize a job-scoped request: the owning client's bearer
    /// token, or the job's own capability token. Any other
    /// credential — including another job's token — is rejected.
    pub fn job_for_capability(
        &self,
        job_id: &str,
        bearer: &str,
        client: Option<&ClientConfig>,
    ) -> Result<Arc<JobRecord>, ServerError> {
        if let Some(client) = client {
            return self.job_for_client(job_id, client);
        }
        let job = {
            let mut inner = self.inner.lock().unwrap();
            if !self.refresh_job(&mut inner, job_id) {
                return Err(ServerError::job_expired());
            }
            inner.jobs.get(job_id).cloned()
        };
        let Some(job) = job else {
            return Err(ServerError::job_expired());
        };
        if secure_eq(&job.job_token, bearer) {
            Ok(job)
        } else {
            Err(ServerError::unauthorized(
                "missing or invalid bearer token for this job",
            ))
        }
    }

    /// Per-client job recovery (Drew's 2026-11-04 protocol
    /// delta): list the authenticated client's own outstanding
    /// jobs — metadata only. Other clients' jobs are never
    /// enumerated to this caller at all.
    pub fn list_jobs(&self, client: &ClientConfig) -> JobListResponse {
        let summaries = {
            let mut inner = self.inner.lock().unwrap();
            let ids: Vec<String> = inner.jobs.keys().cloned().collect();
            for id in &ids {
                self.refresh_job(&mut inner, id);
            }
            let queue: Vec<String> = inner.queue.iter().cloned().collect();
            let mut out = Vec::new();
            for job in inner.jobs.values() {
                if job.owner_name == client.name && secure_eq(&job.owner_token, &client.token) {
                    let comp = &job.computation;
                    out.push(JobSummary {
                        job_id: job.job_id.clone(),
                        status: job.status(&self.limits),
                        query: comp.request.query.clone(),
                        mode: comp.request.mode,
                        pool: comp.request.pool,
                        effective_ceiling: comp.effective_ceiling,
                        submitted_at_ms: job.submitted_at_ms,
                        expires_at_ms: comp.expires_at_ms(&self.limits),
                        queue_position: queue.iter().position(|id| id == &comp.id),
                    });
                }
            }
            out
        };
        let mut jobs = summaries;
        jobs.sort_by_key(|j| j.submitted_at_ms);
        JobListResponse { jobs }
    }

    pub fn status(&self, job: &Arc<JobRecord>) -> JobStatusResponse {
        let (queue_depth, queue_position) = {
            let mut inner = self.inner.lock().unwrap();
            self.refresh_job(&mut inner, &job.job_id);
            (
                inner.queue.len(),
                inner.queue.iter().position(|id| id == &job.computation.id),
            )
        };
        job.status_response(&self.limits, queue_depth, queue_position)
    }

    /// Slice one page out of the job's immutable stored ranking.
    /// Paging never re-executes anything: it reads the ranking
    /// the shared computation stored at completion.
    pub fn page(
        &self,
        job: &Arc<JobRecord>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<SearchPage, ServerError> {
        {
            let mut inner = self.inner.lock().unwrap();
            self.refresh_job(&mut inner, &job.job_id);
        }
        let comp = &job.computation;
        match job.status(&self.limits) {
            JobStatus::Done => {}
            JobStatus::Expired => return Err(ServerError::job_gone()),
            JobStatus::Failed => {
                let msg = comp
                    .state
                    .lock()
                    .unwrap()
                    .error
                    .clone()
                    .unwrap_or_else(|| "search failed".to_string());
                return Err(ServerError::new(409, "job_failed", msg));
            }
            JobStatus::Queued | JobStatus::Running => {
                return Err(ServerError::new(
                    409,
                    "job_not_done",
                    format!("job is {}", job.status(&self.limits).name()),
                ));
            }
        }
        let envelope = {
            let state = comp.state.lock().unwrap();
            state.result.clone()
        };
        let Some(envelope) = envelope else {
            return Err(ServerError::job_gone());
        };
        let all = envelope
            .get("results")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default();
        let offset = match cursor {
            None => 0,
            Some(c) => parse_cursor(c)?,
        };
        if offset > all.len() {
            return Err(ServerError::bad_request(
                "cursor is beyond the end of the ranking",
            ));
        }
        let limit = limit.clamp(1, 1000);
        let end = (offset + limit).min(all.len());
        let results: Vec<Json> = all[offset..end].to_vec();
        let next_cursor = if end < all.len() {
            Some(format_cursor(end))
        } else {
            None
        };
        let mut page_envelope = envelope.clone();
        if let Some(obj) = page_envelope.as_object_mut() {
            obj.insert("results".to_string(), Json::Array(results.clone()));
        }
        Ok(SearchPage {
            job_id: job.job_id.clone(),
            status: JobStatus::Done,
            effective_ceiling: comp.effective_ceiling,
            envelope: page_envelope,
            results,
            next_cursor,
            total_results: all.len(),
        })
    }

    pub fn delete(self: &Arc<Self>, job: &Arc<JobRecord>) -> Result<JobDeleted, ServerError> {
        let mut inner = self.inner.lock().unwrap();
        self.refresh_job(&mut inner, &job.job_id);
        if job.status(&self.limits) == JobStatus::Running {
            return Err(ServerError::new(
                409,
                "job_running",
                "a running job cannot be released; wait for completion or expiry",
            ));
        }
        // Deleting releases this submission's handle only. The
        // shared computation (and any other client's handle on
        // it) is unaffected; the cache entry lives until its TTL.
        inner.jobs.remove(&job.job_id);
        Ok(JobDeleted {
            job_id: job.job_id.clone(),
            status: "released".to_string(),
        })
    }

    pub fn chunk(
        &self,
        job: &Arc<JobRecord>,
        chunk_id: i64,
        neighbors: usize,
    ) -> Result<Json, ServerError> {
        {
            let mut inner = self.inner.lock().unwrap();
            self.refresh_job(&mut inner, &job.job_id);
        }
        match job.status(&self.limits) {
            JobStatus::Expired => return Err(ServerError::job_gone()),
            JobStatus::Failed => {
                return Err(ServerError::new(409, "job_failed", "the job failed"));
            }
            _ => {}
        }
        self.engine
            .chunk(chunk_id, neighbors, job.computation.effective_ceiling)
    }

    /// Embed one query through the engine's shared embedder
    /// (#6788). No job is created: the vector is a pure function
    /// of the text under the pinned model, and the caller (the
    /// search Worker) owns its retention.
    pub fn embed(&self, text: &str, trunc: usize) -> Result<Vec<f32>, ServerError> {
        self.engine.embed_query(text, trunc)
    }

    pub fn embedder_state(&self) -> String {
        self.engine.embedder_state()
    }
}

/// Cursors are opaque to clients; the encoding is a hex-wrapped
/// decimal offset into the immutable ranking.
fn format_cursor(offset: usize) -> String {
    let raw = format!("atlas-offset:{offset}");
    raw.bytes().map(|b| format!("{b:02x}")).collect()
}

fn parse_cursor(cursor: &str) -> Result<usize, ServerError> {
    // Every byte must be an ASCII hex digit and the length even (#7761,
    // the #2517 gate the sibling hex parsers already carry): slicing the
    // string at byte offsets without this gate panics on an odd-length
    // cursor (final pair out of range) or on a multi-byte character at a
    // misaligned offset, killing the per-connection thread instead of
    // returning the malformed-cursor 400 below. u8::from_str_radix also
    // accepts a leading plus sign, so the gate is what makes a pair two
    // hex digits and nothing else.
    let raw = cursor.as_bytes();
    if raw.len() % 2 != 0 || !raw.iter().all(u8::is_ascii_hexdigit) {
        return Err(ServerError::bad_request("malformed cursor"));
    }
    let bytes: Result<Vec<u8>, _> = (0..cursor.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cursor[i..i + 2], 16))
        .collect();
    let bytes = bytes.map_err(|_| ServerError::bad_request("malformed cursor"))?;
    let text =
        String::from_utf8(bytes).map_err(|_| ServerError::bad_request("malformed cursor"))?;
    text.strip_prefix("atlas-offset:")
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| ServerError::bad_request("malformed cursor"))
}

// -------------------------------------------------------------- HTTP ----

mod server_http;

use server_http::*;

// ---------------------------------------------------------- feedback ----

mod server_feedback;

use server_feedback::FeedbackRequest;

// ------------------------------------------------------------ routing ----

pub struct ServerHandle {
    pub addr: SocketAddr,
    /// The job manager behind the listener (tests and health
    /// introspection; the serving path routes through it too).
    #[allow(dead_code)]
    pub manager: Arc<JobManager>,
}

struct ServerInner {
    settings: ServeSettings,
    manager: Arc<JobManager>,
    /// Raw feedback store directory (design section 6, #9976):
    /// the JsonlFeedbackSink default in production; tests point
    /// it at a temp store via `start_server_with_feedback_dir`.
    feedback_dir: PathBuf,
}

pub fn start_server(
    settings: ServeSettings,
    engine: Arc<dyn SearchEngine>,
) -> Result<ServerHandle, String> {
    start_server_with_feedback_dir(settings, engine, JsonlFeedbackSink::default_dir())
}

pub fn start_server_with_feedback_dir(
    settings: ServeSettings,
    engine: Arc<dyn SearchEngine>,
    feedback_dir: PathBuf,
) -> Result<ServerHandle, String> {
    require_loopback(settings.listen)?;
    validate_limits(&settings.limits)?;
    let listener = TcpListener::bind(settings.listen)
        .map_err(|e| format!("cannot bind {}: {e}", settings.listen))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("cannot read listener address: {e}"))?;
    let manager = JobManager::new(engine, settings.limits);
    manager.start_sweeper();
    let inner = Arc::new(ServerInner {
        settings,
        manager: manager.clone(),
        feedback_dir,
    });
    std::thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let inner = inner.clone();
                    std::thread::spawn(move || {
                        let response = match read_request(&mut stream) {
                            Ok(Some(req)) => inner.route(&req),
                            Ok(None) => return,
                            Err(e) => HttpResponse::from_error(&e),
                        };
                        write_response(&mut stream, &response);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    });
    Ok(ServerHandle { addr, manager })
}

/// Run serve mode in the foreground (the binary's `serve` verb).
pub fn serve_forever(settings: ServeSettings, engine: Arc<dyn SearchEngine>) -> Result<(), String> {
    let handle = start_server(settings, engine)?;
    eprintln!(
        "atlas-search: serving on http://{} (loopback only; jobs are in-memory, TTL 10m from completion)",
        handle.addr
    );
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

impl ServerInner {
    fn route(&self, req: &HttpRequest) -> HttpResponse {
        match self.dispatch(req) {
            Ok(resp) => resp,
            Err(e) => HttpResponse::from_error(&e),
        }
    }

    fn require_client(&self, req: &HttpRequest) -> Result<ClientConfig, ServerError> {
        let bearer = req
            .bearer()
            .ok_or_else(|| ServerError::unauthorized("missing Authorization: Bearer token"))?;
        self.settings
            .authenticate_client(bearer)
            .ok_or_else(|| ServerError::unauthorized("invalid client token"))
    }

    fn job_from_request(
        &self,
        req: &HttpRequest,
        job_id: &str,
    ) -> Result<Arc<JobRecord>, ServerError> {
        let bearer = req
            .bearer()
            .ok_or_else(|| ServerError::unauthorized("missing Authorization: Bearer token"))?;
        let client = self.settings.authenticate_client(bearer);
        self.manager
            .job_for_capability(job_id, bearer, client.as_ref())
    }

    fn dispatch(&self, req: &HttpRequest) -> Result<HttpResponse, ServerError> {
        let segments: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
        match (req.method.as_str(), segments.as_slice()) {
            ("GET", ["health"]) => {
                let stats = self.manager.stats();
                Ok(HttpResponse::json(
                    200,
                    serde_json::to_value(HealthResponse {
                        status: "ok".to_string(),
                        embedder_state: self.manager.embedder_state(),
                        queue_depth: stats.queue_depth,
                        active_branch_executions: stats.active_branch_executions,
                        active_jobs: stats.active_jobs,
                    })
                    .unwrap(),
                ))
            }
            ("POST", ["v1", "embed"]) => {
                // Authenticated like every /v1 route. The vector
                // carries no corpus data, so any valid client may
                // embed; ceilings govern search, not embedding
                // (#6788).
                self.require_client(req)?;
                let request: EmbedRequest = req.json()?;
                if request.query.trim().is_empty() {
                    return Err(ServerError::bad_request("query must not be empty"));
                }
                let vector = self
                    .manager
                    .embed(&request.query, request.trunc.unwrap_or(0))?;
                Ok(HttpResponse::json(
                    200,
                    serde_json::to_value(EmbedResponse {
                        model: atlas_core::DEFAULT_MODEL.to_string(),
                        dims: vector.len(),
                        vector,
                    })
                    .unwrap(),
                ))
            }
            ("POST", ["v1", "feedback"]) | ("POST", ["feedback"]) => {
                // Feedback capture (design section 6.1, #9976):
                // authenticated like every /v1 route — the client
                // token is the surface's credential, while the
                // REQUESTER in the body may be anonymous (section
                // 6.3). The vote lands in the raw feedback store;
                // the response echoes the stored record.
                self.require_client(req)?;
                let request: FeedbackRequest = req.json()?;
                let input = request.into_input().map_err(ServerError::bad_request)?;
                let sink = JsonlFeedbackSink::new(self.feedback_dir.clone());
                let record = record_feedback(&sink, &input)
                    .map_err(|e| ServerError::new(500, "feedback_store", e.to_string()))?;
                Ok(HttpResponse::json(201, record.to_json()))
            }
            ("GET", ["v1", "feedback"]) | ("GET", ["feedback"]) => {
                // Retrieval by (query_id, result_id): every raw
                // record for the query — narrowed to one chunk
                // when `chunk_id` is given — with its derived
                // supersede state (section 6.4).
                self.require_client(req)?;
                let Some(query_id) = req.query.get("query_id") else {
                    return Err(ServerError::bad_request(
                        "query_id query parameter is required",
                    ));
                };
                let chunk_id = match req.query.get("chunk_id") {
                    Some(raw) => Some(
                        raw.parse::<i64>()
                            .map_err(|_| ServerError::bad_request("chunk_id must be an integer"))?,
                    ),
                    None => None,
                };
                let records = load_feedback_dir(&self.feedback_dir)
                    .map_err(|e| ServerError::new(500, "feedback_store", e.to_string()))?;
                let views = feedback_for(&records, query_id, chunk_id);
                Ok(HttpResponse::json(
                    200,
                    json!({
                        "query_id": query_id,
                        "chunk_id": chunk_id,
                        "feedback": views.iter().map(|v| v.to_json()).collect::<Vec<_>>(),
                    }),
                ))
            }
            ("POST", ["v1", "search"]) => {
                let client = self.require_client(req)?;
                let request: SearchRequest = req.json()?;
                let page = self.manager.one_shot(&client, request)?;
                Ok(HttpResponse::json(200, serde_json::to_value(page).unwrap()))
            }
            ("GET", ["v1", "search", "jobs"]) => {
                // Job recovery: a client bearer token lists its
                // OWN jobs (metadata only). Job capability tokens
                // are not client credentials and cannot list.
                let client = self.require_client(req)?;
                let list = self.manager.list_jobs(&client);
                Ok(HttpResponse::json(200, serde_json::to_value(list).unwrap()))
            }
            ("POST", ["v1", "search", "jobs"]) => {
                let client = self.require_client(req)?;
                let request: SearchRequest = req.json()?;
                let created = self.manager.submit(&client, request)?;
                Ok(HttpResponse::json(
                    202,
                    serde_json::to_value(created).unwrap(),
                ))
            }
            ("GET", ["v1", "search", "jobs", job_id]) => {
                let job = self.job_from_request(req, job_id)?;
                let status = self.manager.status(&job);
                Ok(HttpResponse::json(
                    200,
                    serde_json::to_value(status).unwrap(),
                ))
            }
            ("GET", ["v1", "search", "jobs", job_id, "results"]) => {
                let job = self.job_from_request(req, job_id)?;
                let limit = req
                    .query
                    .get("limit")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(100);
                let page =
                    self.manager
                        .page(&job, req.query.get("cursor").map(String::as_str), limit)?;
                Ok(HttpResponse::json(200, serde_json::to_value(page).unwrap()))
            }
            ("DELETE", ["v1", "search", "jobs", job_id]) => {
                let job = self.job_from_request(req, job_id)?;
                let deleted = self.manager.delete(&job)?;
                Ok(HttpResponse::json(
                    200,
                    serde_json::to_value(deleted).unwrap(),
                ))
            }
            ("GET", ["v1", "search", "jobs", job_id, "chunk", chunk_id]) => {
                let job = self.job_from_request(req, job_id)?;
                let chunk_id: i64 = chunk_id
                    .parse()
                    .map_err(|_| ServerError::bad_request("chunk id must be an integer"))?;
                let neighbors = req
                    .query
                    .get("neighbors")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                if neighbors > 100 {
                    return Err(ServerError::bad_request("neighbors must be at most 100"));
                }
                let out = self.manager.chunk(&job, chunk_id, neighbors)?;
                Ok(HttpResponse::json(200, out))
            }
            _ => Err(ServerError::new(
                404,
                "not_found",
                format!("no route for {} {}", req.method, req.path),
            )),
        }
    }
}

// -------------------------------------------------------------- tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::types::{BranchHit, ChunkDetail, MessageRow, NeighborRow};
    use atlas_core::{FixedEmbedder, SearchBackend};
    use std::net::TcpStream;
    use std::sync::Condvar;

    // ----- stub backend -------------------------------------------------

    #[derive(Default)]
    struct Gate {
        open: Mutex<bool>,
        cv: Condvar,
    }

    impl Gate {
        fn wait(&self) -> Result<(), CoreError> {
            let mut open = self.open.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !*open {
                let (guard, _) = self
                    .cv
                    .wait_timeout(open, Duration::from_millis(100))
                    .unwrap();
                open = guard;
                if Instant::now() >= deadline {
                    return Err(CoreError::backend("stub gate timed out"));
                }
            }
            Ok(())
        }

        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.cv.notify_all();
        }
    }

    struct StubBackend {
        calls: Arc<Mutex<Vec<BranchQuery>>>,
        gate: Option<Arc<Gate>>,
        /// Mutable corpus snapshot for the result-cache tests
        /// (#9943): rotating it mid-test models a re-shard /
        /// promotion bump.
        snapshot: Option<Arc<Mutex<String>>>,
    }

    fn detail(id: i64, text: &str, sensitivity: &str) -> ChunkDetail {
        ChunkDetail {
            chunk_id: id,
            text: text.to_string(),
            chunk_idx: 0,
            message_id: format!("m{id}"),
            m_role: Some("user".to_string()),
            session_id: format!("s{id}"),
            s_title: Some("stub session".to_string()),
            s_client: Some("stub".to_string()),
            s_created_at: Some("2026-11-04T00:00:00Z".to_string()),
            l_kind: Some("chat".to_string()),
            l_topics: Some("[]".to_string()),
            l_projects: Some("[]".to_string()),
            eff_sensitivity: Some(sensitivity.to_string()),
        }
    }

    impl StubBackend {
        fn hits(&self, q: &BranchQuery, filters: &Filters) -> Vec<i64> {
            let text = q.match_expr.clone().unwrap_or_default();
            if text.contains("secret") {
                if filters.max_sensitivity == CoreSensitivity::SecretBearing {
                    return vec![3];
                }
                return vec![1];
            }
            if text.contains("beta") {
                return vec![2];
            }
            if text.contains("gamma") {
                return vec![1];
            }
            vec![1, 2]
        }
    }

    impl SearchBackend for StubBackend {
        fn name(&self) -> &'static str {
            "turso"
        }

        fn snapshot_id(&self) -> Option<String> {
            self.snapshot
                .as_ref()
                .map(|snap| snap.lock().unwrap().clone())
        }

        fn vec_table(&self) -> &str {
            atlas_core::DEFAULT_VEC_TABLE
        }

        fn notes(&self) -> Vec<String> {
            Vec::new()
        }

        fn branch_pools(
            &self,
            q: &BranchQuery,
            filters: &Filters,
        ) -> Result<BranchPools, CoreError> {
            self.calls.lock().unwrap().push(q.clone());
            if let Some(gate) = &self.gate {
                gate.wait()?;
            }
            let ids = self.hits(q, filters);
            let mut pools = BranchPools::default();
            if q.match_expr.is_some() {
                pools.fts = ids
                    .iter()
                    .map(|id| BranchHit {
                        chunk_id: *id,
                        dist: None,
                    })
                    .collect();
            }
            if q.query_vector.is_some() {
                pools.vector = ids
                    .iter()
                    .enumerate()
                    .map(|(i, id)| BranchHit {
                        chunk_id: *id,
                        dist: Some(0.01 * (i as f64 + 1.0)),
                    })
                    .collect();
            }
            Ok(pools)
        }

        fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>, CoreError> {
            Ok(ids
                .iter()
                .filter_map(|id| match id {
                    1 => Some(detail(1, "alpha one", "personal")),
                    2 => Some(detail(2, "alpha two", "personal")),
                    3 => Some(detail(3, "secret canary", "secret-bearing")),
                    _ => None,
                })
                .collect())
        }

        fn neighbors(
            &self,
            _message_id: &str,
            _chunk_idx: i64,
            _n: i64,
        ) -> Result<Vec<NeighborRow>, CoreError> {
            Ok(Vec::new())
        }

        fn message(&self, _message_id: &str) -> Result<Option<MessageRow>, CoreError> {
            Ok(None)
        }
    }

    // ----- harness ------------------------------------------------------

    const PERSONAL_TOKEN: &str = "test-personal-token";
    const SECRET_TOKEN: &str = "test-secret-token";

    fn settings(limits: RuntimeLimits) -> ServeSettings {
        ServeSettings {
            listen: "127.0.0.1:0".parse().unwrap(),
            clients: vec![
                ClientConfig {
                    name: "personal-cli".to_string(),
                    token: PERSONAL_TOKEN.to_string(),
                    ceiling_cap: ProtoSensitivity::Personal,
                },
                ClientConfig {
                    name: "drew-cli".to_string(),
                    token: SECRET_TOKEN.to_string(),
                    ceiling_cap: ProtoSensitivity::SecretBearing,
                },
            ],
            vec_table: atlas_core::DEFAULT_VEC_TABLE.to_string(),
            limits,
            shards: None,
            shard_refresh: Duration::from_secs(DEFAULT_SHARD_REFRESH_SECONDS),
            source_registry: None,
        }
    }

    #[test]
    fn validate_limits_rejects_zero_queue_caps() {
        // #5630: the admission check is `queued >= cap`, so a zero
        // queue cap rejects every submission on an empty queue --
        // the service serves nothing but 429s. Both queue caps get
        // the same at-least-1 guard as the concurrency caps.
        let d = RuntimeLimits::default();
        assert!(validate_limits(&d).is_ok());
        assert_eq!(
            validate_limits(&RuntimeLimits {
                max_queued_jobs: 0,
                ..d
            })
            .unwrap_err(),
            "max_queued_jobs must be at least 1"
        );
        assert_eq!(
            validate_limits(&RuntimeLimits {
                max_queued_jobs_per_client: 0,
                ..d
            })
            .unwrap_err(),
            "max_queued_jobs_per_client must be at least 1"
        );
    }

    #[test]
    fn service_config_source_registry_round_trips() {
        // #3125 slice 7: `source_registry` in the service config
        // lands on ServeSettings as a path; it requires `shards`
        // (attribution rides the fan-out) and must not be empty.
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "atlas-service-registry-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            path
        };
        let client = r#"{"name": "probe", "token": "tok", "ceiling_cap": "personal"}"#;
        let wired = write(
            "wired.json",
            &format!(
                r#"{{"shards": "dir:/tmp/shards", "source_registry": "/tmp/catalog.db", "clients": [{client}]}}"#
            ),
        );
        let settings = load_service_settings(&wired, ServeOverrides::default()).unwrap();
        assert_eq!(
            settings.source_registry.as_deref(),
            Some(std::path::Path::new("/tmp/catalog.db"))
        );
        let orphan = write(
            "orphan.json",
            &format!(r#"{{"source_registry": "/tmp/catalog.db", "clients": [{client}]}}"#),
        );
        let err = load_service_settings(&orphan, ServeOverrides::default()).unwrap_err();
        assert!(
            err.contains("source_registry requires shards"),
            "got: {err}"
        );
        let empty = write(
            "empty.json",
            &format!(
                r#"{{"shards": "dir:/tmp/shards", "source_registry": "  ", "clients": [{client}]}}"#
            ),
        );
        let err = load_service_settings(&empty, ServeOverrides::default()).unwrap_err();
        assert!(
            err.contains("source_registry path must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn service_config_shards_spec_round_trips() {
        // #6666 slice 1: `shards` + `shard_refresh_seconds` in the
        // service config land on ServeSettings; without `shards`
        // the settings keep the monolith default (None).
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("atlas-service-shards-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            path
        };
        let client = r#"{"name": "probe", "token": "tok", "ceiling_cap": "personal"}"#;
        let sharded = write(
            "sharded.json",
            &format!(
                r#"{{"shards": "dir:/tmp/shards", "shard_refresh_seconds": 5, "clients": [{client}]}}"#
            ),
        );
        let settings = load_service_settings(&sharded, ServeOverrides::default()).unwrap();
        assert_eq!(settings.shards.as_deref(), Some("dir:/tmp/shards"));
        assert_eq!(settings.shard_refresh, Duration::from_secs(5));
        let plain = write("plain.json", &format!(r#"{{"clients": [{client}]}}"#));
        let settings = load_service_settings(&plain, ServeOverrides::default()).unwrap();
        assert_eq!(settings.shards, None);
        assert_eq!(
            settings.shard_refresh,
            Duration::from_secs(DEFAULT_SHARD_REFRESH_SECONDS)
        );
        let empty = write(
            "empty.json",
            &format!(r#"{{"shards": "  ", "clients": [{client}]}}"#),
        );
        let err = load_service_settings(&empty, ServeOverrides::default()).unwrap_err();
        assert!(err.contains("shards spec must not be empty"), "got: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn service_config_rejects_unknown_keys() {
        // #5632: a misspelled key in service.json must be a loud
        // parse error, never a silent fall back to defaults -- the
        // file carries client tokens, sensitivity ceilings, the
        // listen address, and the runtime limits. Mirrors the
        // fleet-config deny_unknown_fields fix (#5460).
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("atlas-service-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            path
        };
        let client = r#"{"name": "probe", "token": "tok", "ceiling_cap": "personal"}"#;
        // A fully-spelled config loads.
        let good = write(
            "good.json",
            &format!(
                r#"{{"listen": "127.0.0.1:18799", "vec_table": "vec_chunks", "limits": {{"max_queued_jobs": 3}}, "clients": [{client}]}}"#
            ),
        );
        let settings = load_service_settings(&good, ServeOverrides::default()).unwrap();
        assert_eq!(settings.limits.max_queued_jobs, 3);
        // Typo'd top-level key: the limits block must not vanish.
        let typo_top = write(
            "typo-top.json",
            &format!(r#"{{"limts": {{"max_queued_jobs": 0}}, "clients": [{client}]}}"#),
        );
        let err = load_service_settings(&typo_top, ServeOverrides::default()).unwrap_err();
        assert!(err.contains("unknown field `limts`"), "got: {err}");
        // Typo'd key inside limits: the cap must not vanish.
        let typo_inner = write(
            "typo-inner.json",
            &format!(r#"{{"limits": {{"max_queued_job": 0}}, "clients": [{client}]}}"#),
        );
        let err = load_service_settings(&typo_inner, ServeOverrides::default()).unwrap_err();
        assert!(err.contains("unknown field `max_queued_job`"), "got: {err}");
        // Typo'd key inside a client row.
        let typo_client = write(
            "typo-client.json",
            r#"{"clients": [{"name": "probe", "token": "tok", "ceiling_capp": "personal"}]}"#,
        );
        let err = load_service_settings(&typo_client, ServeOverrides::default()).unwrap_err();
        assert!(err.contains("unknown field"), "got: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn start(
        limits: RuntimeLimits,
        gate: Option<Arc<Gate>>,
    ) -> (ServerHandle, Arc<Mutex<Vec<BranchQuery>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(StubBackend {
            calls: calls.clone(),
            gate,
            snapshot: None,
        });
        let engine = Arc::new(CoreSearchEngine::new(
            backend,
            Arc::new(FixedEmbedder {
                raw: vec![1.0, 0.0],
            }),
            Arc::new(|| "ready".to_string()),
        ));
        let handle = start_server(settings(limits), engine).unwrap();
        (handle, calls)
    }

    /// Start a server whose backend reports a mutable corpus
    /// snapshot (#9943): the test rotates the string to model a
    /// snapshot bump between submits.
    fn start_with_snapshot(
        limits: RuntimeLimits,
        snapshot: Arc<Mutex<String>>,
    ) -> (ServerHandle, Arc<Mutex<Vec<BranchQuery>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(StubBackend {
            calls: calls.clone(),
            gate: None,
            snapshot: Some(snapshot),
        });
        let engine = Arc::new(CoreSearchEngine::new(
            backend,
            Arc::new(FixedEmbedder {
                raw: vec![1.0, 0.0],
            }),
            Arc::new(|| "ready".to_string()),
        ));
        let handle = start_server(settings(limits), engine).unwrap();
        (handle, calls)
    }

    struct TestResponse {
        status: u16,
        headers: HashMap<String, String>,
        body: Json,
    }

    fn http(
        addr: SocketAddr,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Json>,
    ) -> TestResponse {
        let mut stream = TcpStream::connect(addr).unwrap();
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        );
        if let Some(t) = token {
            req.push_str(&format!("Authorization: Bearer {t}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(&payload);
        stream.write_all(req.as_bytes()).unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let status: u16 = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let mut headers = HashMap::new();
        for line in head.lines().skip(1) {
            if let Some((k, v)) = line.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        TestResponse {
            status,
            headers,
            body: serde_json::from_str(body).unwrap_or(Json::Null),
        }
    }

    #[test]
    fn embed_route_serves_the_shared_embedder_vector() {
        // #6788: the Worker miss path embeds through the service
        // so the stored vector lives in the corpus's own space.
        // Oracle: the harness embedder is FixedEmbedder raw
        // [1, 0], which the shared finish step leaves as the unit
        // vector [1, 0]; dims and model must name that space.
        let (handle, _calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;
        let resp = http(
            addr,
            "POST",
            "/v1/embed",
            Some(PERSONAL_TOKEN),
            Some(json!({"query": "alpha"})),
        );
        assert_eq!(resp.status, 200, "body: {}", resp.body);
        assert_eq!(resp.body["model"], json!(atlas_core::DEFAULT_MODEL));
        assert_eq!(resp.body["dims"], json!(2));
        assert_eq!(resp.body["vector"], json!([1.0, 0.0]));
        // Unauthenticated embedding is refused like every /v1 route.
        let resp = http(
            addr,
            "POST",
            "/v1/embed",
            None,
            Some(json!({"query": "alpha"})),
        );
        assert_eq!(resp.status, 401);
        // An empty query is a client error, never an embedding.
        let resp = http(
            addr,
            "POST",
            "/v1/embed",
            Some(PERSONAL_TOKEN),
            Some(json!({"query": "  "})),
        );
        assert_eq!(resp.status, 400);
    }

    fn start_with_feedback(limits: RuntimeLimits) -> (ServerHandle, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "atlas-feedback-server-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let backend = Arc::new(StubBackend {
            calls: Arc::new(Mutex::new(Vec::new())),
            gate: None,
            snapshot: None,
        });
        let engine = Arc::new(CoreSearchEngine::new(
            backend,
            Arc::new(FixedEmbedder {
                raw: vec![1.0, 0.0],
            }),
            Arc::new(|| "ready".to_string()),
        ));
        let handle = start_server_with_feedback_dir(settings(limits), engine, dir.clone()).unwrap();
        (handle, dir)
    }

    #[test]
    fn feedback_routes_cast_and_retrieve_by_ids() {
        // Oracle: design section 6.1/6.4 (#9976) — a vote cast via
        // the service API against a real (query_id, result_id) is
        // retrievable by those IDs; an unopened down-vote stores
        // opened=false; a re-vote supersedes with the raw record
        // preserved.
        let (handle, dir) = start_with_feedback(RuntimeLimits::default());
        let addr = handle.addr;

        // Unauthenticated feedback is refused like every /v1 route.
        let resp = http(
            addr,
            "POST",
            "/v1/feedback",
            None,
            Some(json!({"query_id": "q-9", "chunk_id": 42, "vote": "up"})),
        );
        assert_eq!(resp.status, 401, "body: {}", resp.body);

        // Cast: thumbs down + comment on an unopened result.
        let cast = http(
            addr,
            "POST",
            "/v1/feedback",
            Some(PERSONAL_TOKEN),
            Some(json!({
                "query_id": "q-9",
                "chunk_id": 42,
                "vote": "down",
                "comment": "snippet looked right, chunk was stale",
                "opened": false,
                "requester": {"kind": "user", "id": "tester"}
            })),
        );
        assert_eq!(cast.status, 201, "body: {}", cast.body);
        assert_eq!(cast.body["vote"], json!("down"));
        assert_eq!(cast.body["opened"], json!(false));
        let first_id = cast.body["feedback_id"].as_str().unwrap().to_string();

        // Retrieve by (query_id, result_id).
        let got = http(
            addr,
            "GET",
            "/v1/feedback?query_id=q-9&chunk_id=42",
            Some(PERSONAL_TOKEN),
            None,
        );
        assert_eq!(got.status, 200, "body: {}", got.body);
        assert_eq!(got.body["feedback"].as_array().unwrap().len(), 1);
        assert_eq!(got.body["feedback"][0]["active"], json!(true));

        // Re-vote supersedes; both raw records stay retrievable.
        let revote = http(
            addr,
            "POST",
            "/feedback",
            Some(PERSONAL_TOKEN),
            Some(json!({
                "query_id": "q-9",
                "chunk_id": 42,
                "vote": "up",
                "opened": true,
                "requester": {"kind": "user", "id": "tester"}
            })),
        );
        assert_eq!(revote.status, 201, "body: {}", revote.body);
        let got = http(
            addr,
            "GET",
            "/v1/feedback?query_id=q-9&chunk_id=42",
            Some(PERSONAL_TOKEN),
            None,
        );
        let rows = got.body["feedback"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "raw records preserved: {}", got.body);
        let first = rows
            .iter()
            .find(|r| r["feedback_id"] == json!(first_id))
            .unwrap();
        assert_eq!(first["active"], json!(false));
        assert_eq!(
            rows.iter().filter(|r| r["active"] == json!(true)).count(),
            1
        );

        // A bad vote is a client error, never a stored record.
        let bad = http(
            addr,
            "POST",
            "/v1/feedback",
            Some(PERSONAL_TOKEN),
            Some(json!({"query_id": "q-9", "chunk_id": 42, "vote": "sideways"})),
        );
        assert_eq!(bad.status, 400, "body: {}", bad.body);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn submit(addr: SocketAddr, token: &str, body: Json) -> TestResponse {
        http(addr, "POST", "/v1/search/jobs", Some(token), Some(body))
    }

    fn wait_done(addr: SocketAddr, job_id: &str, token: &str) -> Json {
        for _ in 0..200 {
            let resp = http(
                addr,
                "GET",
                &format!("/v1/search/jobs/{job_id}"),
                Some(token),
                None,
            );
            assert_eq!(resp.status, 200, "status poll failed: {}", resp.body);
            if resp.body["status"] == "done" {
                return resp.body;
            }
            if resp.body["status"] == "failed" || resp.body["status"] == "expired" {
                panic!("job ended as {}: {}", resp.body["status"], resp.body);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("job {job_id} did not finish");
    }

    fn result_ids(page: &Json) -> Vec<i64> {
        page["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["chunk_id"].as_i64().unwrap())
            .collect()
    }

    fn result_fingerprint(page: &Json) -> Vec<Json> {
        page["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                json!({
                    "chunk_id": r["chunk_id"],
                    "score": r["score"],
                    "rank_sources": r["rank_sources"],
                })
            })
            .collect()
    }

    // ----- tests --------------------------------------------------------

    #[test]
    fn paging_reuses_stored_ranking_and_matches_one_shot() {
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        let one_shot = http(
            addr,
            "POST",
            "/v1/search",
            Some(PERSONAL_TOKEN),
            Some(json!({"query": "alpha", "page_limit": 10})),
        );
        assert_eq!(one_shot.status, 200, "{}", one_shot.body);
        assert_eq!(result_ids(&one_shot.body), vec![1, 2]);

        let created = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "k": 10, "pool": 100}),
        );
        assert_eq!(created.status, 202, "{}", created.body);
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &job_id, &job_token);

        // The one-shot and the identical job submission are the
        // SAME effective request, so they share one computation
        // (dedupe): 2 branch calls total. Paging must not add any.
        let calls_after_search = calls.lock().unwrap().len();
        assert_eq!(calls_after_search, 2);

        let page1 = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results?limit=1"),
            Some(&job_token),
            None,
        );
        assert_eq!(page1.status, 200, "{}", page1.body);
        assert_eq!(result_ids(&page1.body), vec![1]);
        let cursor = page1.body["next_cursor"].as_str().unwrap().to_string();
        let page2 = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results?limit=1&cursor={cursor}"),
            Some(&job_token),
            None,
        );
        assert_eq!(page2.status, 200, "{}", page2.body);
        assert_eq!(result_ids(&page2.body), vec![2]);
        assert!(page2.body["next_cursor"].is_null());
        assert_eq!(calls.lock().unwrap().len(), calls_after_search);

        let mut concatenated = result_fingerprint(&page1.body);
        concatenated.extend(result_fingerprint(&page2.body));
        assert_eq!(concatenated, result_fingerprint(&one_shot.body));
    }

    #[test]
    fn capability_and_ceiling_are_enforced_on_every_path() {
        let (handle, _calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        // A personal-capped client asking for secret-bearing is
        // clamped, and the clamp is what the job records.
        let created = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing"}),
        );
        assert_eq!(created.status, 202, "{}", created.body);
        assert_eq!(created.body["effective_ceiling"], "personal");
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        let status = wait_done(addr, &job_id, &job_token);
        assert_eq!(status["effective_ceiling"], "personal");

        let page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results"),
            Some(&job_token),
            None,
        );
        assert_eq!(page.status, 200);
        assert!(!result_ids(&page.body).contains(&3));

        // No token and a wrong token are rejected; another job's
        // capability token is rejected for this job.
        let no_auth = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results"),
            None,
            None,
        );
        assert_eq!(no_auth.status, 401);
        let wrong = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results"),
            Some("not-a-token"),
            None,
        );
        assert_eq!(wrong.status, 401);
        let other = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
        let other_token = other.body["job_token"].as_str().unwrap().to_string();
        let cross = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results"),
            Some(&other_token),
            None,
        );
        assert_eq!(cross.status, 401);

        // Job-scoped get-chunk: the secret-bearing chunk is
        // indistinguishable from a missing chunk at this ceiling.
        let chunk = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/chunk/3"),
            Some(&job_token),
            None,
        );
        assert_eq!(chunk.status, 404);
        assert_eq!(chunk.body["error"]["code"], "chunk_not_found");
        let missing = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/chunk/999"),
            Some(&job_token),
            None,
        );
        assert_eq!(missing.status, 404);
        assert_eq!(missing.body["error"]["code"], "chunk_not_found");

        // The secret-capped client's own job can read it.
        let secret_job = submit(
            addr,
            SECRET_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing"}),
        );
        assert_eq!(secret_job.body["effective_ceiling"], "secret-bearing");
        let secret_id = secret_job.body["job_id"].as_str().unwrap().to_string();
        let secret_token = secret_job.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &secret_id, &secret_token);
        let chunk = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{secret_id}/chunk/3"),
            Some(&secret_token),
            None,
        );
        assert_eq!(chunk.status, 200, "{}", chunk.body);
        assert_eq!(chunk.body["chunk_id"], 3);
    }

    #[test]
    fn ttl_runs_from_completion_not_submission() {
        let gate = Arc::new(Gate::default());
        let limits = RuntimeLimits {
            job_ttl: Duration::from_millis(250),
            hard_ttl: Duration::from_secs(60),
            max_concurrent_branch_executions: 1,
            max_concurrent_jobs_per_client: 2,
            ..RuntimeLimits::default()
        };
        let (handle, calls) = start(limits, Some(gate.clone()));
        let addr = handle.addr;

        let a = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "mode": "fts"}),
        );
        assert_eq!(a.status, 202, "{}", a.body);
        // Wait until A is actually executing (and blocked).
        for _ in 0..200 {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(calls.lock().unwrap().len(), 1);

        let b = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "beta", "mode": "fts"}),
        );
        assert_eq!(b.status, 202, "{}", b.body);
        assert_eq!(b.body["status"], "queued");
        assert_eq!(b.body["queue_position"], 0);
        let b_id = b.body["job_id"].as_str().unwrap().to_string();
        let b_token = b.body["job_token"].as_str().unwrap().to_string();

        // B sits in the queue for longer than the result TTL. Its
        // TTL must not start until it completes.
        std::thread::sleep(Duration::from_millis(400));
        let status = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{b_id}"),
            Some(&b_token),
            None,
        );
        assert_eq!(status.body["status"], "queued");

        gate.release();
        wait_done(addr, &b_id, &b_token);
        let page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{b_id}/results"),
            Some(&b_token),
            None,
        );
        assert_eq!(page.status, 200, "{}", page.body);
        assert_eq!(result_ids(&page.body), vec![2]);

        std::thread::sleep(Duration::from_millis(350));
        let expired = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{b_id}/results"),
            Some(&b_token),
            None,
        );
        assert_eq!(expired.status, 410);
        assert_eq!(expired.body["error"]["code"], "job_expired");
        let status = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{b_id}"),
            Some(&b_token),
            None,
        );
        assert_eq!(status.body["status"], "expired");
    }

    #[test]
    fn queue_is_bounded_fifo_and_overflow_is_429() {
        let gate = Arc::new(Gate::default());
        let limits = RuntimeLimits {
            max_concurrent_branch_executions: 1,
            max_concurrent_jobs_per_client: 1,
            max_queued_jobs_per_client: 8,
            max_queued_jobs: 2,
            retry_after_seconds: 7,
            ..RuntimeLimits::default()
        };
        let (handle, calls) = start(limits, Some(gate.clone()));
        let addr = handle.addr;

        let a = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "mode": "fts"}),
        );
        assert_eq!(a.status, 202, "{}", a.body);
        for _ in 0..200 {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let b = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "beta", "mode": "fts"}),
        );
        assert_eq!(b.status, 202, "{}", b.body);
        assert_eq!(b.body["status"], "queued");
        assert_eq!(b.body["queue_position"], 0);
        let c = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "gamma", "mode": "fts"}),
        );
        assert_eq!(c.status, 202, "{}", c.body);
        assert_eq!(c.body["queue_position"], 1);

        // The bounded FIFO queue is full (2 waiting): the next
        // submission is a 429 with Retry-After and the queue depth.
        let d = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "delta", "mode": "fts"}),
        );
        assert_eq!(d.status, 429, "{}", d.body);
        assert_eq!(d.headers.get("retry-after").map(String::as_str), Some("7"));
        assert_eq!(d.body["error"]["code"], "rate_limited");
        assert_eq!(d.body["error"]["queue_depth"], 2);
        assert_eq!(d.body["error"]["retry_after_seconds"], 7);

        gate.release();
        let b_id = b.body["job_id"].as_str().unwrap().to_string();
        let b_token = b.body["job_token"].as_str().unwrap().to_string();
        let c_id = c.body["job_id"].as_str().unwrap().to_string();
        let c_token = c.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &b_id, &b_token);
        wait_done(addr, &c_id, &c_token);

        // FIFO execution order: alpha, beta, gamma branch calls.
        let order: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|q| q.match_expr.clone().unwrap_or_default())
            .collect();
        assert_eq!(order.len(), 3);
        assert!(order[0].contains("alpha"), "{order:?}");
        assert!(order[1].contains("beta"), "{order:?}");
        assert!(order[2].contains("gamma"), "{order:?}");

        // The queued job's results match the one-shot path.
        let page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{b_id}/results"),
            Some(&b_token),
            None,
        );
        assert_eq!(page.status, 200);
        let one_shot = http(
            addr,
            "POST",
            "/v1/search",
            Some(PERSONAL_TOKEN),
            Some(json!({"query": "beta", "mode": "fts", "page_limit": 10})),
        );
        assert_eq!(one_shot.status, 200, "{}", one_shot.body);
        assert_eq!(
            result_fingerprint(&page.body),
            result_fingerprint(&one_shot.body)
        );
    }

    #[test]
    fn vector_index_modes_are_first_class_single_branch_jobs() {
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        let created = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "mode": "vector", "vector_backend": "index-fill", "k": 2}),
        );
        assert_eq!(created.status, 202, "{}", created.body);
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &job_id, &job_token);
        {
            let recorded = calls.lock().unwrap();
            assert_eq!(recorded.len(), 1);
            assert!(recorded[0].match_expr.is_none());
            assert!(recorded[0].query_vector.is_some());
            assert_eq!(recorded[0].vector_backend, CoreVectorBackend::IndexFill);
        }

        let created = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "mode": "hybrid", "vector_backend": "index-fixed", "k": 2}),
        );
        assert_eq!(created.status, 202, "{}", created.body);
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &job_id, &job_token);
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 3);
        let hybrid_calls = &recorded[1..];
        assert_eq!(
            hybrid_calls
                .iter()
                .filter(|q| q.match_expr.is_some() && q.query_vector.is_none())
                .count(),
            1
        );
        assert_eq!(
            hybrid_calls
                .iter()
                .filter(|q| {
                    q.query_vector.is_some() && q.vector_backend == CoreVectorBackend::IndexFixed
                })
                .count(),
            1
        );
    }

    #[test]
    fn health_is_public_and_delete_releases_the_job() {
        let (handle, _calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;
        let health = http(addr, "GET", "/health", None, None);
        assert_eq!(health.status, 200);
        assert_eq!(health.body["status"], "ok");
        assert_eq!(health.body["embedder_state"], "ready");

        let created = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &job_id, &job_token);
        let deleted = http(
            addr,
            "DELETE",
            &format!("/v1/search/jobs/{job_id}"),
            Some(&job_token),
            None,
        );
        assert_eq!(deleted.status, 200, "{}", deleted.body);
        assert_eq!(deleted.body["status"], "released");
        let status = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}"),
            Some(PERSONAL_TOKEN),
            None,
        );
        assert_eq!(status.status, 404);
        assert_eq!(status.body["error"]["code"], "job_expired_or_unknown");
    }

    #[test]
    fn clients_list_and_recover_only_their_own_jobs() {
        let (handle, _calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        // Client A submits; the "session" then drops everything
        // except its client token and (separately) the job token.
        let created = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
        assert_eq!(created.status, 202, "{}", created.body);
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &job_id, &job_token);

        // A lists: the job is there, with metadata only.
        let list = http(addr, "GET", "/v1/search/jobs", Some(PERSONAL_TOKEN), None);
        assert_eq!(list.status, 200, "{}", list.body);
        let jobs = list.body["jobs"].as_array().unwrap();
        let mine: Vec<&Json> = jobs.iter().filter(|j| j["job_id"] == job_id).collect();
        assert_eq!(
            mine.len(),
            1,
            "A's list must contain A's job: {}",
            list.body
        );
        let summary = mine[0];
        assert_eq!(summary["status"], "done");
        assert_eq!(summary["query"], "alpha");
        assert_eq!(summary["effective_ceiling"], "personal");
        assert!(
            summary.get("results").is_none(),
            "list must not carry results"
        );
        assert!(
            summary.get("job_token").is_none(),
            "list must not carry capability tokens"
        );
        assert!(
            summary.get("text").is_none(),
            "list must not carry chunk text"
        );

        // B lists: A's job is not there.
        let list_b = http(addr, "GET", "/v1/search/jobs", Some(SECRET_TOKEN), None);
        assert_eq!(list_b.status, 200, "{}", list_b.body);
        let jobs_b = list_b.body["jobs"].as_array().unwrap();
        assert!(
            jobs_b.iter().all(|j| j["job_id"] != job_id),
            "B must not see A's job: {}",
            list_b.body
        );

        // A job capability token is not a client credential: it
        // cannot list. Neither can an anonymous caller.
        let list_job_token = http(addr, "GET", "/v1/search/jobs", Some(&job_token), None);
        assert_eq!(list_job_token.status, 401);
        let list_anon = http(addr, "GET", "/v1/search/jobs", None, None);
        assert_eq!(list_anon.status, 401);

        // Pickup from a fresh connection: job id + capability
        // token reads the stored ranking (every test http call is
        // a new connection; this is the post-disconnect path).
        let page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results"),
            Some(&job_token),
            None,
        );
        assert_eq!(page.status, 200, "{}", page.body);
        assert_eq!(result_ids(&page.body), vec![1, 2]);
    }

    // ----- dedupe cache (Drew's 2026-11-04 ruling) ----------------------

    #[test]
    fn dedupe_identical_unfiltered_submits_execute_once() {
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        let j1 = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
        assert_eq!(j1.status, 202, "{}", j1.body);
        let id1 = j1.body["job_id"].as_str().unwrap().to_string();
        let tok1 = j1.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &id1, &tok1);
        let calls_after_first = calls.lock().unwrap().len();
        assert_eq!(
            calls_after_first, 2,
            "one hybrid computation = 2 branch calls"
        );

        // Identical submit within the TTL: coalesces onto the
        // completed computation — no new branch execution, its own
        // job id + token, already done.
        let j2 = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
        assert_eq!(j2.status, 202, "{}", j2.body);
        let id2 = j2.body["job_id"].as_str().unwrap().to_string();
        let tok2 = j2.body["job_token"].as_str().unwrap().to_string();
        assert_ne!(id1, id2, "each submitter gets its own job id");
        assert_ne!(tok1, tok2, "each submitter gets its own capability token");
        assert_eq!(j2.body["status"], "done");
        assert_eq!(calls.lock().unwrap().len(), calls_after_first);

        // Byte-identical results on both handles.
        let p1 = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{id1}/results?limit=100"),
            Some(&tok1),
            None,
        );
        let p2 = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{id2}/results?limit=100"),
            Some(&tok2),
            None,
        );
        assert_eq!(p1.status, 200);
        assert_eq!(p2.status, 200);
        assert_eq!(
            serde_json::to_string(&p1.body["results"]).unwrap(),
            serde_json::to_string(&p2.body["results"]).unwrap(),
            "coalesced results must be byte-identical"
        );
        assert_eq!(
            serde_json::to_string(&p1.body["envelope"]).unwrap(),
            serde_json::to_string(&p2.body["envelope"]).unwrap()
        );
        assert_eq!(calls.lock().unwrap().len(), calls_after_first);
    }

    #[test]
    fn dedupe_never_crosses_effective_ceilings() {
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        // Both jobs state the local-owner disclosure context
        // (#12404): the canary is secret-bearing, and under the
        // service's fleet-internal default context it is now
        // withheld from every caller — only the owner's local
        // surface may receive it. The variable under test here
        // stays the effective ceiling.
        let low = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing", "context": "local-owner"}),
        );
        let low_id = low.body["job_id"].as_str().unwrap().to_string();
        let low_tok = low.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &low_id, &low_tok);
        let calls_after_low = calls.lock().unwrap().len();

        // Same query text, HIGHER effective ceiling: a different
        // cache key, a second execution.
        let high = submit(
            addr,
            SECRET_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing", "context": "local-owner"}),
        );
        assert_eq!(high.body["effective_ceiling"], "secret-bearing");
        let high_id = high.body["job_id"].as_str().unwrap().to_string();
        let high_tok = high.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &high_id, &high_tok);
        assert!(
            calls.lock().unwrap().len() > calls_after_low,
            "different effective ceiling must execute separately"
        );

        let low_page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{low_id}/results"),
            Some(&low_tok),
            None,
        );
        assert!(
            !result_ids(&low_page.body).contains(&3),
            "canary must not cross to the lower ceiling"
        );
        let high_page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{high_id}/results"),
            Some(&high_tok),
            None,
        );
        assert!(
            result_ids(&high_page.body).contains(&3),
            "higher-ceiling job sees its own canary"
        );
    }

    #[test]
    fn dedupe_never_crosses_disclosure_contexts() {
        // #12404: the caller's ceiling is identical and maximal
        // for both jobs; only the destination context differs.
        // The context is part of the cache key, so the public
        // job executes separately and its stored payload is the
        // context-filtered one — the owner's unfiltered-by-
        // context ranking must never be served into it.
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        let owner = submit(
            addr,
            SECRET_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing", "context": "local-owner"}),
        );
        let owner_id = owner.body["job_id"].as_str().unwrap().to_string();
        let owner_tok = owner.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &owner_id, &owner_tok);
        let calls_after_owner = calls.lock().unwrap().len();

        let public = submit(
            addr,
            SECRET_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing", "context": "public-surface"}),
        );
        let public_id = public.body["job_id"].as_str().unwrap().to_string();
        let public_tok = public.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &public_id, &public_tok);
        assert!(
            calls.lock().unwrap().len() > calls_after_owner,
            "different disclosure context must execute separately"
        );

        let public_page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{public_id}/results"),
            Some(&public_tok),
            None,
        );
        assert!(
            !result_ids(&public_page.body).contains(&3),
            "canary must not cross to the public context"
        );
        let owner_page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{owner_id}/results"),
            Some(&owner_tok),
            None,
        );
        assert!(
            result_ids(&owner_page.body).contains(&3),
            "owner-context job sees its own canary"
        );
    }

    #[test]
    fn dedupe_distinguishes_filters() {
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;

        let f1 = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "filters": {"kind": ["chat"]}}),
        );
        let f1_id = f1.body["job_id"].as_str().unwrap().to_string();
        let f1_tok = f1.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &f1_id, &f1_tok);
        let after_f1 = calls.lock().unwrap().len();

        // Different filter tuple -> different entry, new execution
        // (never a subset of the stored ranking).
        let f2 = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "filters": {"kind": ["note"]}}),
        );
        let f2_id = f2.body["job_id"].as_str().unwrap().to_string();
        let f2_tok = f2.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &f2_id, &f2_tok);
        assert!(
            calls.lock().unwrap().len() > after_f1,
            "different filters must execute separately"
        );
        let after_f2 = calls.lock().unwrap().len();

        // The unfiltered form is yet another entry.
        let f3 = submit(addr, PERSONAL_TOKEN, json!({"query": "alpha"}));
        let f3_id = f3.body["job_id"].as_str().unwrap().to_string();
        let f3_tok = f3.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &f3_id, &f3_tok);
        assert!(
            calls.lock().unwrap().len() > after_f2,
            "unfiltered must not reuse a filtered entry"
        );
        let after_f3 = calls.lock().unwrap().len();

        // And the exact filtered repeat coalesces.
        let f1b = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "filters": {"kind": ["chat"]}}),
        );
        assert_eq!(f1b.body["status"], "done");
        assert_eq!(calls.lock().unwrap().len(), after_f3);
    }

    #[test]
    fn search_rejects_unsupported_parameters_before_backend_work() {
        // Oracle: SearchParametersV1::validate rejects an unimplemented
        // planner stage, and the HTTP handler validates before scheduling.
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let response = submit(
            handle.addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "planner_enabled": true}),
        );
        assert_eq!(response.status, 400, "{}", response.body);
        assert!(calls.lock().unwrap().is_empty(), "refused search did work");
    }

    #[test]
    fn search_rejects_generic_dimensions_before_backend_work() {
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let response = submit(
            handle.addr,
            PERSONAL_TOKEN,
            json!({
                "query": "alpha",
                "filters": { "dimensions": { "source-app": ["mail"] } }
            }),
        );
        assert_eq!(response.status, 400, "{}", response.body);
        assert!(
            response.body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("not supported")
        );
        assert!(calls.lock().unwrap().is_empty(), "refused search did work");
    }

    #[test]
    fn search_honors_and_dedupes_by_parameters() {
        // Oracle: the API contract flattens SearchParametersV1 onto the
        // request; effective controls reach branch execution and distinguish
        // cached searches when their values differ.
        let (handle, calls) = start(RuntimeLimits::default(), None);
        let addr = handle.addr;
        let request = json!({
            "query": "alpha",
            "shard_top_k": 17,
            "global_top_k": 23,
            "rrf_k": 61
        });
        let first = submit(addr, PERSONAL_TOKEN, request.clone());
        assert_eq!(first.status, 202, "{}", first.body);
        let first_id = first.body["job_id"].as_str().unwrap().to_string();
        let first_token = first.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &first_id, &first_token);
        let first_calls = calls.lock().unwrap().clone();
        assert!(
            first_calls
                .iter()
                .any(|query| query.shard_top_k == 17 && query.global_top_k == 23),
            "effective branch controls must reach the backend: {first_calls:?}"
        );

        let exact_repeat = submit(addr, PERSONAL_TOKEN, request);
        assert_eq!(exact_repeat.body["status"], "done");
        assert_eq!(calls.lock().unwrap().len(), first_calls.len());

        let different_parameters = submit(
            addr,
            PERSONAL_TOKEN,
            json!({
                "query": "alpha",
                "shard_top_k": 17,
                "global_top_k": 23,
                "rrf_k": 62
            }),
        );
        assert_eq!(
            different_parameters.status, 202,
            "{}",
            different_parameters.body
        );
        let different_id = different_parameters.body["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let different_token = different_parameters.body["job_token"]
            .as_str()
            .unwrap()
            .to_string();
        wait_done(addr, &different_id, &different_token);
        assert!(
            calls.lock().unwrap().len() > first_calls.len(),
            "different effective parameters must not reuse the first ranking"
        );
    }

    #[test]
    fn cache_key_binds_manifest_surface_and_generic_dimensions() {
        // #6228 Phase 3/4, oracle: key equality. Two submits that
        // differ only in the effective manifest's (version,
        // content_hash), or only in a generic dimension filter,
        // must NOT share a cache key; identical surface + filters
        // must share one.
        let mut req = SearchRequest::default();
        req.query = "alpha".to_string();
        let builtin = builtin_manifest();
        let base = CacheKey::from_request(&req, ProtoSensitivity::Personal, "snap-1", &builtin);
        let same = CacheKey::from_request(&req, ProtoSensitivity::Personal, "snap-1", &builtin);
        assert_eq!(base, same);

        let mut other_surface = builtin.clone();
        other_surface.version += 1;
        other_surface.content_hash = other_surface.compute_hash();
        let diff_surface =
            CacheKey::from_request(&req, ProtoSensitivity::Personal, "snap-1", &other_surface);
        assert_ne!(base, diff_surface);

        req.filters
            .set_dimension("source-app", vec!["mail".to_string()]);
        let diff_dimension =
            CacheKey::from_request(&req, ProtoSensitivity::Personal, "snap-1", &builtin);
        assert_ne!(base, diff_dimension);
    }

    #[test]
    fn generic_dimensions_are_rejected_until_search_support_lands() {
        let mut request = SearchRequest::default();
        request
            .filters
            .set_dimension("source-app", vec!["mail".to_string()]);

        let error = build_filters(&request, ProtoSensitivity::Personal).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(error.body.error.code, "bad_request");
        assert!(error.body.error.message.contains("not supported"));
    }

    // ----- snapshot-bound result cache (#9943, slice 3 of #5668) --------

    fn submit_and_page(addr: SocketAddr, body: Json) -> (String, String, Vec<i64>) {
        let created = submit(addr, PERSONAL_TOKEN, body);
        assert_eq!(created.status, 202, "{}", created.body);
        let job_id = created.body["job_id"].as_str().unwrap().to_string();
        let job_token = created.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &job_id, &job_token);
        let page = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{job_id}/results?limit=100"),
            Some(&job_token),
            None,
        );
        assert_eq!(page.status, 200, "{}", page.body);
        (job_id, job_token, result_ids(&page.body))
    }

    #[test]
    fn result_cache_serves_repeat_after_job_ttl_with_no_backend_calls() {
        // Oracle: the StubBackend branch-call count. The job TTL
        // is 250ms; the result-cache TTL is the 24h default. A
        // repeat submit after the first job's TTL has run must be
        // served from the snapshot-bound result cache — a fresh,
        // already-done job with identical results and ZERO new
        // backend (Turso round-trip) calls.
        let limits = RuntimeLimits {
            job_ttl: Duration::from_millis(250),
            hard_ttl: Duration::from_secs(60),
            ..RuntimeLimits::default()
        };
        let snapshot = Arc::new(Mutex::new("snap-1".to_string()));
        let (handle, calls) = start_with_snapshot(limits, snapshot);
        let addr = handle.addr;

        let (id1, _tok1, ids1) = submit_and_page(addr, json!({"query": "alpha", "mode": "fts"}));
        assert_eq!(ids1, vec![1, 2]);
        let calls_after_first = calls.lock().unwrap().len();
        assert_eq!(calls_after_first, 1, "one fts computation = 1 branch call");

        std::thread::sleep(Duration::from_millis(400));
        let (id2, _tok2, ids2) = submit_and_page(addr, json!({"query": "alpha", "mode": "fts"}));
        assert_ne!(id1, id2, "cache serve still mints its own job handle");
        assert_eq!(ids2, ids1, "cached results are the computed results");
        assert_eq!(
            calls.lock().unwrap().len(),
            calls_after_first,
            "repeat on an unchanged snapshot must not re-execute"
        );
    }

    #[test]
    fn result_cache_gc_ttl_bounds_serving() {
        // The 24h TTL is a GC bound only: with a tiny bound, an
        // entry past it is dropped and the repeat re-executes.
        let limits = RuntimeLimits {
            job_ttl: Duration::from_millis(250),
            hard_ttl: Duration::from_secs(60),
            result_cache_ttl: Duration::from_millis(250),
            ..RuntimeLimits::default()
        };
        let snapshot = Arc::new(Mutex::new("snap-1".to_string()));
        let (handle, calls) = start_with_snapshot(limits, snapshot);
        let addr = handle.addr;

        submit_and_page(addr, json!({"query": "alpha", "mode": "fts"}));
        assert_eq!(calls.lock().unwrap().len(), 1);
        std::thread::sleep(Duration::from_millis(400));
        submit_and_page(addr, json!({"query": "alpha", "mode": "fts"}));
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "entry past the GC bound must be recomputed"
        );
    }

    #[test]
    fn result_cache_no_cache_bypasses_reads_and_refreshes_writes() {
        // Acceptance (#9943): --no-cache bypasses result-cache
        // reads (re-executes even within every TTL) and refreshes
        // writes (the next ordinary repeat serves the refreshed
        // entry with no execution).
        let snapshot = Arc::new(Mutex::new("snap-1".to_string()));
        let (handle, calls) = start_with_snapshot(RuntimeLimits::default(), snapshot);
        let addr = handle.addr;

        submit_and_page(addr, json!({"query": "alpha", "mode": "fts"}));
        assert_eq!(calls.lock().unwrap().len(), 1);
        submit_and_page(
            addr,
            json!({"query": "alpha", "mode": "fts", "no_cache": true}),
        );
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "no_cache must bypass the cache read and re-execute"
        );
        submit_and_page(addr, json!({"query": "alpha", "mode": "fts"}));
        assert_eq!(
            calls.lock().unwrap().len(),
            2,
            "the refreshed entry serves the next repeat"
        );
    }

    #[test]
    fn result_cache_mbt_snapshot_rotation_never_serves_pre_rotation_entries() {
        // MBT (testing doctrine; oracle = backend call count):
        // a deterministic model drives submits, snapshot
        // rotations, and --no-cache submits. The model says an
        // execution happens exactly when (query, snapshot,
        // no-cache variant) has no live entry, or the submit is
        // --no-cache (which then refreshes ITS variant's entry:
        // the flag is part of the canonical key material, pinned
        // by atlas-proto's flag-sensitivity tests). After every
        // step the real backend call count must equal the
        // model's execution count — so a pre-rotation entry
        // served post-rotation (a missing execution) fails the
        // test at that step.
        let snapshot = Arc::new(Mutex::new("snap-0".to_string()));
        let (handle, calls) = start_with_snapshot(RuntimeLimits::default(), snapshot.clone());
        let addr = handle.addr;

        let mut live: std::collections::HashSet<(String, String, bool)> =
            std::collections::HashSet::new();
        let mut model_executions = 0usize;
        let mut rotation = 0usize;
        // Small deterministic LCG: the sequence is fixed, so a
        // failure reproduces exactly.
        let mut rng: u64 = 0x5668_9943;
        let mut next = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 33) as usize
        };
        for step in 0..48 {
            match next() % 4 {
                0 => {
                    rotation += 1;
                    *snapshot.lock().unwrap() = format!("snap-{rotation}");
                }
                op => {
                    let query = if op == 2 { "beta" } else { "alpha" };
                    let no_cache = op == 3;
                    let snap_now = snapshot.lock().unwrap().clone();
                    let key = (query.to_string(), snap_now, no_cache);
                    if no_cache || !live.contains(&key) {
                        model_executions += 1;
                        live.insert(key);
                    }
                    let mut body = json!({"query": query, "mode": "fts"});
                    if no_cache {
                        body["no_cache"] = json!(true);
                    }
                    let (_, _, ids) = submit_and_page(addr, body);
                    // StubBackend hits: alpha -> [1, 2], beta -> [2].
                    let expected = if query == "alpha" {
                        vec![1, 2]
                    } else {
                        vec![2]
                    };
                    assert_eq!(ids, expected, "step {step}");
                    assert_eq!(
                        calls.lock().unwrap().len(),
                        model_executions,
                        "step {step}: backend executions diverge from the model                          (a stale-snapshot entry was served, or a live entry missed)"
                    );
                }
            }
        }
        assert!(
            rotation >= 3,
            "the model must rotate repeatedly, got {rotation}"
        );
        assert!(model_executions >= 4, "the model must execute repeatedly");
    }

    #[test]
    fn dedupe_coalesces_in_flight_and_queued() {
        let gate = Arc::new(Gate::default());
        let limits = RuntimeLimits {
            max_concurrent_branch_executions: 1,
            max_concurrent_jobs_per_client: 2,
            ..RuntimeLimits::default()
        };
        let (handle, calls) = start(limits, Some(gate.clone()));
        let addr = handle.addr;

        // Running coalescing: job 2 attaches to job 1's running
        // computation and observes its state.
        let j1 = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "mode": "fts"}),
        );
        assert_eq!(j1.status, 202);
        for _ in 0..200 {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let j2 = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "alpha", "mode": "fts"}),
        );
        assert_eq!(j2.status, 202, "{}", j2.body);
        assert_eq!(
            j2.body["status"], "running",
            "coalesced handle observes the running computation"
        );
        let j2_id = j2.body["job_id"].as_str().unwrap().to_string();
        let j2_tok = j2.body["job_token"].as_str().unwrap().to_string();

        // Queued coalescing: a different query queues behind the
        // busy branch budget; its duplicate attaches as queued.
        let q1 = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "beta", "mode": "fts"}),
        );
        assert_eq!(q1.body["status"], "queued");
        let q2 = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "beta", "mode": "fts"}),
        );
        assert_eq!(q2.status, 202, "{}", q2.body);
        assert_eq!(
            q2.body["status"], "queued",
            "coalesced handle observes the queued computation"
        );
        let q2_id = q2.body["job_id"].as_str().unwrap().to_string();
        let q2_tok = q2.body["job_token"].as_str().unwrap().to_string();

        gate.release();
        let j1_id = j1.body["job_id"].as_str().unwrap().to_string();
        let j1_tok = j1.body["job_token"].as_str().unwrap().to_string();
        wait_done(addr, &j1_id, &j1_tok);
        wait_done(addr, &j2_id, &j2_tok);
        wait_done(addr, &q2_id, &q2_tok);

        // Exactly one branch execution per distinct query.
        {
            let recorded = calls.lock().unwrap();
            assert_eq!(recorded.len(), 2, "alpha once + beta once: {recorded:?}");
        }
        let p1 = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{j1_id}/results"),
            Some(&j1_tok),
            None,
        );
        let p2 = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{j2_id}/results"),
            Some(&j2_tok),
            None,
        );
        assert_eq!(
            serde_json::to_string(&p1.body["results"]).unwrap(),
            serde_json::to_string(&p2.body["results"]).unwrap()
        );
        let pq = http(
            addr,
            "GET",
            &format!("/v1/search/jobs/{q2_id}/results"),
            Some(&q2_tok),
            None,
        );
        assert_eq!(result_ids(&pq.body), vec![2]);
    }
    #[test]
    fn parse_cursor_refuses_malformed_input_without_panicking() {
        // #7761: odd-length, single-char, and multibyte cursors used to
        // panic on byte-offset slicing; every malformed shape must be
        // the malformed-cursor error instead.
        let valid = format_cursor(42);
        assert_eq!(parse_cursor(&valid).ok(), Some(42));
        assert!(parse_cursor("").is_err());
        assert!(parse_cursor("zzzz").is_err());
        assert!(parse_cursor("a").is_err());
        assert!(parse_cursor(&valid[..valid.len() - 1]).is_err());
        assert!(parse_cursor("6162aé").is_err());
        assert!(parse_cursor("+a").is_err());
    }
}
