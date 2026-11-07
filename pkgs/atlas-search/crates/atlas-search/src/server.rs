//! Serve mode (design sections 2-3; developing-today/code#5354).
//! Source of the verbatim copy in pkgs/fleet's fleet-search crate,
//! rehomed in #5604 (`fleet serve search`): the Atlas service must
//! behave identically under either name, so mirror every change
//! there. Drift guard: .github/scripts/check-fleet-search-parity.sh.
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
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atlas_core::types::BranchPools;
use atlas_core::{
    BranchPoolExecutor, BranchQuery, Error as CoreError, Filters, QueryEmbedder, SearchBackend,
    SearchMode as CoreSearchMode, SearchOptions, Sensitivity as CoreSensitivity,
    VectorBackend as CoreVectorBackend, run_get_chunk, run_search_with_executor,
};
use atlas_proto::{
    BranchState, BranchStates, ErrorResponse, HealthResponse, JobCreated, JobDeleted,
    JobListResponse, JobStatus, JobStatusResponse, JobSummary, SearchMode as ProtoSearchMode,
    SearchPage, SearchRequest, Sensitivity as ProtoSensitivity,
    VectorBackend as ProtoVectorBackend,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

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
    limits: Option<RuntimeLimitsFile>,
    clients: Vec<RawClientConfig>,
}

#[derive(Debug, Clone)]
pub struct ServeSettings {
    pub listen: SocketAddr,
    pub clients: Vec<ClientConfig>,
    pub vec_table: String,
    pub limits: RuntimeLimits,
}

pub fn default_service_config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("~"));
    home.join(".local/state/atlas-search/service.json")
}

pub fn resolve_service_config_path(flag: Option<&Path>) -> PathBuf {
    if let Some(p) = flag {
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
        let cap = ProtoSensitivity::from_name(&c.ceiling_cap).ok_or_else(|| {
            format!(
                "client {}: ceiling_cap must be one of shareable, personal, local-only, secret-bearing",
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
    Ok(ServeSettings {
        listen,
        clients,
        vec_table: raw
            .vec_table
            .unwrap_or_else(|| atlas_core::DEFAULT_VEC_TABLE.to_string()),
        limits,
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
    if req.pool == 0 || req.pool > 400 {
        return Err(ServerError::bad_request(
            "pool must be between 1 and 400 (design section 2)",
        ));
    }
    if req.expand > atlas_core::MAX_EXPAND_NEIGHBORS {
        return Err(ServerError::bad_request(format!(
            "expand must be at most {}",
            atlas_core::MAX_EXPAND_NEIGHBORS
        )));
    }
    // Deliberately leave k unbounded: it truncates the candidates produced by
    // the bounded pool, and grouped search caps its consideration window to
    // the available candidates in atlas-core.
    if req.k == 0 {
        return Err(ServerError::bad_request("k must be at least 1"));
    }
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
pub trait SearchEngine: Send + Sync {
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
        };
        let executor = ConcurrentBranchExecutor {
            backend: self.backend.as_ref(),
            branch_states: branch_states.clone(),
        };
        run_search_with_executor(
            self.backend.as_ref(),
            self.embedder.as_ref(),
            filters,
            &opts,
            &executor,
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
        if !allowed {
            return Err(not_found());
        }
        run_get_chunk(self.backend.as_ref(), chunk_id, neighbors)
            .map_err(|e| ServerError::from_core(&e))
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
            vector_backend: query.vector_backend,
        };
        let vector_query = BranchQuery {
            match_expr: None,
            query_vector: query.query_vector.clone(),
            pool: query.pool,
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
//    effective_ceiling)
//
// Exact-key sharing only: the effective ceiling is IN the key, so
// a lower-ceiling request can never be served from a
// higher-ceiling entry; filters are in the key, so a filtered
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
    query: String,
    mode: ProtoSearchMode,
    k: usize,
    pool: usize,
    group_by_session: bool,
    expand: usize,
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
    effective_ceiling: ProtoSensitivity,
}

impl CacheKey {
    fn from_request(req: &SearchRequest, effective_ceiling: ProtoSensitivity) -> CacheKey {
        CacheKey {
            query: req.query.clone(),
            mode: req.mode,
            k: req.k,
            pool: req.pool,
            group_by_session: req.group_by_session,
            expand: req.expand,
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
            effective_ceiling,
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
    /// FIFO queue of computation ids awaiting branch budget.
    queue: VecDeque<String>,
    active_branch_executions: usize,
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
        {
            let mut state = comp.state.lock().unwrap();
            if state.status == JobStatus::Running {
                match outcome {
                    Ok(Ok(envelope)) => {
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
        let key = CacheKey::from_request(&request, effective);
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
            // Cache resolution. An entry is attachable while it
            // is queued/running, or done within its TTL. Failed
            // entries were already evicted at completion; expired
            // ones are evicted here and recomputed.
            let cached = inner.cache.get(&key).cloned();
            let comp = match cached {
                Some(comp) if comp.status(&self.limits) != JobStatus::Expired => comp,
                Some(comp) => {
                    inner.cache.remove(&key);
                    inner.computations.remove(&comp.id);
                    self.new_computation(&mut inner, client, &request, effective, &key)?
                }
                None => self.new_computation(&mut inner, client, &request, effective, &key)?,
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

struct HttpRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn bearer(&self) -> Option<&str> {
        let h = self.headers.get("authorization")?;
        h.strip_prefix("Bearer ")
            .or_else(|| h.strip_prefix("bearer "))
    }

    fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, ServerError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| ServerError::bad_request(format!("invalid JSON body: {e}")))
    }
}

struct HttpResponse {
    status: u16,
    body: Json,
    retry_after_seconds: Option<u64>,
}

impl HttpResponse {
    fn json(status: u16, body: Json) -> HttpResponse {
        HttpResponse {
            status,
            body,
            retry_after_seconds: None,
        }
    }

    fn from_error(e: &ServerError) -> HttpResponse {
        HttpResponse {
            status: e.status,
            body: serde_json::to_value(&e.body).unwrap_or_else(
                |_| json!({"error": {"code": "internal", "message": "unserializable error"}}),
            ),
            retry_after_seconds: e.body.error.retry_after_seconds,
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        504 => "Gateway Timeout",
        _ => "Response",
    }
}

fn url_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>, ServerError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .map_err(|e| ServerError::new(500, "internal", e.to_string()))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 64 * 1024 {
            return Err(ServerError::bad_request("request headers too large"));
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|e| ServerError::new(400, "bad_request", format!("read failed: {e}")))?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(ServerError::bad_request("connection closed mid-request"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    if method.is_empty() || target.is_empty() {
        return Err(ServerError::bad_request("malformed request line"));
    }
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    if headers
        .get("expect")
        .map(|v| v.to_ascii_lowercase().contains("100-continue"))
        .unwrap_or(false)
    {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if content_length > 4 * 1024 * 1024 {
        return Err(ServerError::bad_request("request body too large"));
    }
    let mut body: Vec<u8> = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream
            .read(&mut chunk)
            .map_err(|e| ServerError::new(400, "bad_request", format!("read failed: {e}")))?;
        if n == 0 {
            return Err(ServerError::bad_request("connection closed mid-body"));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => {
            let mut map = HashMap::new();
            for pair in q.split('&').filter(|s| !s.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                map.insert(url_decode(k), url_decode(v));
            }
            (url_decode(p), map)
        }
        None => (url_decode(&target), HashMap::new()),
    };
    Ok(Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    }))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn write_response(stream: &mut TcpStream, resp: &HttpResponse) {
    let body = serde_json::to_string(&resp.body).unwrap_or_else(|_| "{}".to_string());
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        resp.status,
        reason(resp.status),
        body.len()
    );
    if resp.status == 401 {
        head.push_str("WWW-Authenticate: Bearer\r\n");
    }
    if let Some(secs) = resp.retry_after_seconds {
        head.push_str(&format!("Retry-After: {secs}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

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
}

pub fn start_server(
    settings: ServeSettings,
    engine: Arc<dyn SearchEngine>,
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
    fn expand_validation_matches_the_core_neighbor_limit() {
        let mut req = SearchRequest {
            expand: atlas_core::MAX_EXPAND_NEIGHBORS,
            ..SearchRequest::default()
        };
        assert!(validate_request(&req).is_ok());

        req.expand = atlas_core::MAX_EXPAND_NEIGHBORS + 1;
        let err = validate_request(&req).unwrap_err();
        assert_eq!(err.status, 400);
        assert!(
            err.body.error.message.contains("100"),
            "got: {:?}",
            err.body
        );
    }

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

        let low = submit(
            addr,
            PERSONAL_TOKEN,
            json!({"query": "secret canary", "ceiling": "secret-bearing"}),
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
            json!({"query": "secret canary", "ceiling": "secret-bearing"}),
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
}
