//! atlas-proto — the Atlas service job protocol (design section 2;
//! developing-today/code#5354).
//!
//! These are serde types only: no HTTP, no storage, and no search
//! policy lives here. The server and the CLI's via-service client
//! both depend on this crate so their wire definitions cannot drift.
//! Policy — especially sensitivity clamping — lives in the server;
//! this crate merely carries the requested ceiling and the server's
//! reported effective ceiling.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// The sensitivity ladder, least to most restricted. The ordering
/// is the security ordering used by the server-side clamp.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Sensitivity {
    Shareable,
    #[default]
    Personal,
    LocalOnly,
    SecretBearing,
}

impl Sensitivity {
    pub fn from_name(name: &str) -> Option<Sensitivity> {
        match name {
            "shareable" => Some(Sensitivity::Shareable),
            "personal" => Some(Sensitivity::Personal),
            "local-only" => Some(Sensitivity::LocalOnly),
            "secret-bearing" => Some(Sensitivity::SecretBearing),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sensitivity::Shareable => "shareable",
            Sensitivity::Personal => "personal",
            Sensitivity::LocalOnly => "local-only",
            Sensitivity::SecretBearing => "secret-bearing",
        }
    }

    /// `min(self, cap)` under the sensitivity ordering.
    pub fn clamped_to(self, cap: Sensitivity) -> Sensitivity {
        std::cmp::min(self, cap)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Fts,
    Vector,
    #[default]
    Hybrid,
}

impl SearchMode {
    pub fn from_name(name: &str) -> Option<SearchMode> {
        match name {
            "fts" => Some(SearchMode::Fts),
            "vector" => Some(SearchMode::Vector),
            "hybrid" => Some(SearchMode::Hybrid),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SearchMode::Fts => "fts",
            SearchMode::Vector => "vector",
            SearchMode::Hybrid => "hybrid",
        }
    }

    /// The maximum number of branch executions this mode can use.
    /// A hybrid query whose FTS expression is empty may use only
    /// the vector branch; reserving two is deliberately conservative.
    pub fn branch_demand(self) -> usize {
        match self {
            SearchMode::Hybrid => 2,
            SearchMode::Fts | SearchMode::Vector => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VectorBackend {
    #[default]
    Scan,
    IndexFixed,
    IndexFill,
}

impl VectorBackend {
    pub fn from_name(name: &str) -> Option<VectorBackend> {
        match name {
            "scan" => Some(VectorBackend::Scan),
            "index-fixed" => Some(VectorBackend::IndexFixed),
            "index-fill" => Some(VectorBackend::IndexFill),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            VectorBackend::Scan => "scan",
            VectorBackend::IndexFixed => "index-fixed",
            VectorBackend::IndexFill => "index-fill",
        }
    }
}

/// Classification/date filters. The ceiling is deliberately a
/// sibling on [`SearchRequest`], not a member here: the protocol
/// makes the requested/effective ceiling distinction explicit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kind: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

fn default_k() -> usize {
    SearchParametersV1::default_k()
}

fn default_pool() -> usize {
    SearchParametersV1::default_pool()
}

fn default_expand() -> usize {
    SearchParametersV1::default_expand()
}

fn query_defaults() -> &'static Json {
    static DEFAULTS: OnceLock<Json> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        serde_json::from_str(include_str!("../query-parameters-v1.json"))
            .expect("query-parameters-v1.json must be valid JSON")
    })
}

fn default_usize(name: &str) -> usize {
    query_defaults()[name]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .expect("query-parameters-v1.json is missing an unsigned default")
}

fn default_u32(name: &str) -> u32 {
    query_defaults()[name]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .expect("query-parameters-v1.json is missing a u32 default")
}

fn default_f64(name: &str) -> f64 {
    query_defaults()[name]
        .as_f64()
        .expect("query-parameters-v1.json is missing a numeric default")
}

fn default_bool(name: &str) -> bool {
    query_defaults()[name]
        .as_bool()
        .expect("query-parameters-v1.json is missing a boolean default")
}

fn default_query_hint() -> Option<String> {
    query_defaults()["query_class_hint"]
        .as_str()
        .map(str::to_string)
}

fn default_variant() -> QueryVariant {
    serde_json::from_value(query_defaults()["variant"].clone())
        .expect("query-parameters-v1.json has an invalid variant default")
}

fn default_model() -> String {
    "Snowflake/snowflake-arctic-embed-m-v2.0".to_string()
}

fn default_embedder() -> String {
    "auto".to_string()
}

/// Query-time controls shared by the CLI, raw JSON request and atlas-core.
/// The shape and provisional production defaults are versioned together in
/// `query-parameters-v1.json`. Controls for stages not implemented yet
/// remain representable but are rejected when set to non-default values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchParametersV1 {
    #[serde(default = "default_parameters_version")]
    pub parameters_version: u32,
    #[serde(default = "default_shard_top_k")]
    pub shard_top_k: usize,
    #[serde(default = "default_global_top_k")]
    pub global_top_k: usize,
    #[serde(default = "default_fts_weight")]
    pub fts_weight: f64,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    #[serde(default = "default_jaccard_depth")]
    pub jaccard_depth: usize,
    #[serde(default = "default_jaccard_threshold")]
    pub jaccard_threshold: f64,
    #[serde(default = "default_rrf_k")]
    pub rrf_k: usize,
    #[serde(default = "default_planner_enabled")]
    pub planner_enabled: bool,
    #[serde(default = "default_spell_enabled")]
    pub spell_enabled: bool,
    #[serde(default = "default_query_hint")]
    pub query_class_hint: Option<String>,
    #[serde(default = "default_variant")]
    pub variant: QueryVariant,
    #[serde(default = "default_raw")]
    pub raw: bool,
    /// #5668 slice 2 (issue #9942): bypass cache reads; results
    /// still land in the cache (refresh semantics).
    #[serde(default = "default_no_cache")]
    pub no_cache: bool,
    /// #5668 slice 2: block for the full fused result instead of
    /// returning a job handle + first page. The service protocol
    /// default is false; the CLI one-shot sets it (its mode
    /// implies --wait) unless --no-wait is passed.
    #[serde(default = "default_wait")]
    pub wait: bool,
    /// #5668 slice 2: hydrate the top N results to full content
    /// from the content store. 0 = details as today. --expand
    /// keeps its shipped neighbor-chunks meaning (the 2026-11-11
    /// contract on #5668); hydration is this field only.
    #[serde(default = "default_hydrate")]
    pub hydrate: usize,
    /// #5668 slice 2: ungrouped output rows (see RawMode). Named
    /// raw_mode, not raw: `raw` above is the shipped verbatim-query
    /// flag and is not repurposed, exactly as --expand was not.
    #[serde(default = "default_raw_mode")]
    pub raw_mode: RawMode,
}

fn default_parameters_version() -> u32 {
    default_u32("parameters_version")
}
fn default_shard_top_k() -> usize {
    default_usize("shard_top_k")
}
fn default_global_top_k() -> usize {
    default_usize("global_top_k")
}
fn default_fts_weight() -> f64 {
    default_f64("fts_weight")
}
fn default_vector_weight() -> f64 {
    default_f64("vector_weight")
}
fn default_jaccard_depth() -> usize {
    default_usize("jaccard_depth")
}
fn default_jaccard_threshold() -> f64 {
    default_f64("jaccard_threshold")
}
fn default_rrf_k() -> usize {
    default_usize("rrf_k")
}
fn default_planner_enabled() -> bool {
    default_bool("planner_enabled")
}
fn default_spell_enabled() -> bool {
    default_bool("spell_enabled")
}
fn default_raw() -> bool {
    default_bool("raw")
}
fn default_no_cache() -> bool {
    default_bool("no_cache")
}
fn default_wait() -> bool {
    default_bool("wait")
}
fn default_hydrate() -> usize {
    default_usize("hydrate")
}
fn default_raw_mode() -> RawMode {
    serde_json::from_value(query_defaults()["raw_mode"].clone())
        .expect("query-parameters-v1.json has an invalid raw_mode default")
}

/// Ungrouped-output mode (#5668 slice 2, design 2026-11-04 s3.3).
/// `Off` is the shipped grouped/fused rendering. `Ungrouped`
/// returns flat fused rows with no session grouping. `Branches`
/// returns the pre-fusion branch rows exactly as the branches
/// produced them (the design's --raw=branches).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RawMode {
    #[default]
    Off,
    Ungrouped,
    Branches,
}

impl RawMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RawMode::Off => "off",
            RawMode::Ungrouped => "ungrouped",
            RawMode::Branches => "branches",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryVariant {
    #[default]
    Raw,
    Rewritten,
    Both,
}

impl Default for SearchParametersV1 {
    fn default() -> Self {
        serde_json::from_str(include_str!("../query-parameters-v1.json"))
            .expect("query-parameters-v1.json must match SearchParametersV1")
    }
}

impl SearchParametersV1 {
    pub fn default_k() -> usize {
        default_usize("k")
    }

    pub fn default_pool() -> usize {
        default_usize("pool")
    }

    pub fn default_expand() -> usize {
        default_usize("expand")
    }

    /// Validate controls available on this implementation. Unsupported
    /// non-default controls fail explicitly rather than being ignored.
    pub fn validate(&self, expand: usize) -> Result<(), String> {
        if self.parameters_version != 1 {
            return Err(format!(
                "parameters_version {} is unsupported (expected 1)",
                self.parameters_version
            ));
        }
        for (name, value) in [
            ("shard_top_k", self.shard_top_k),
            ("global_top_k", self.global_top_k),
        ] {
            if !(1..=400).contains(&value) {
                return Err(format!("{name} must be between 1 and 400"));
            }
        }
        if self.rrf_k == 0 || self.rrf_k > 10_000 {
            return Err("rrf_k must be between 1 and 10000".to_string());
        }
        for (name, value) in [
            ("fts_weight", self.fts_weight),
            ("vector_weight", self.vector_weight),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be a finite non-negative number"));
            }
        }
        if self.fts_weight == 0.0 && self.vector_weight == 0.0 {
            return Err("at least one of fts_weight or vector_weight must be positive".to_string());
        }
        if !self.jaccard_threshold.is_finite()
            || self.jaccard_threshold < 0.0
            || self.jaccard_threshold > 1.0
        {
            return Err("jaccard_threshold must be a finite number in [0, 1]".to_string());
        }
        if self.planner_enabled || self.spell_enabled {
            return Err("planner/spell stages are not implemented yet".to_string());
        }
        if self.query_class_hint.is_some() {
            return Err("query_class_hint is not implemented yet".to_string());
        }
        if self.variant != QueryVariant::Raw {
            return Err("rewritten/both query variants are not implemented yet".to_string());
        }
        if self.raw && expand != 0 {
            return Err("raw mode cannot be combined with non-zero expand".to_string());
        }
        if self.hydrate > 1000 {
            return Err("hydrate must be at most 1000".to_string());
        }
        Ok(())
    }
}

impl SearchParametersV1 {
    /// The canonical key material for every cache keyed by a
    /// search's outcome (#5668 s3.2/s3.3): all of the parameters
    /// plus the request-level result-shaping controls (expand,
    /// group_by_session, with_message) in a fixed order with
    /// explicit labels, so two searches that could render
    /// differently never share key material. This is THE
    /// constructor: the service dedupe key stores it verbatim and
    /// the result cache hashes it (see result_cache_key); a flag
    /// added here must be added to this string, and the flag
    /// sensitivity tests below fail until it is.
    pub fn canonical_key_material(
        &self,
        expand: usize,
        group_by_session: bool,
        with_message: bool,
    ) -> String {
        format!(
            "pv={}|shard_top_k={}|global_top_k={}|fts_weight={}|vector_weight={}|jaccard_depth={}|jaccard_threshold={}|rrf_k={}|planner={}|spell={}|hint={}|variant={:?}|raw={}|no_cache={}|wait={}|hydrate={}|raw_mode={}|expand={}|group={}|with_message={}",
            self.parameters_version,
            self.shard_top_k,
            self.global_top_k,
            self.fts_weight,
            self.vector_weight,
            self.jaccard_depth,
            self.jaccard_threshold,
            self.rrf_k,
            self.planner_enabled,
            self.spell_enabled,
            self.query_class_hint.as_deref().unwrap_or(""),
            self.variant,
            self.raw,
            self.no_cache,
            self.wait,
            self.hydrate,
            self.raw_mode.as_str(),
            expand,
            group_by_session,
            with_message,
        )
    }

    /// The design's result-cache key (#5668 s3.2):
    /// sha256(query_text || snapshot_id || ceiling || flags),
    /// hex-encoded. `snapshot_id` names the corpus state (the
    /// publish-manifest hash; the result-cache slice supplies
    /// it); `ceiling` is the EFFECTIVE (clamped) ceiling name,
    /// so a lower-ceiling request can never be served from a
    /// higher-ceiling entry.
    pub fn result_cache_key(
        &self,
        query: &str,
        snapshot_id: &str,
        ceiling: &str,
        expand: usize,
        group_by_session: bool,
        with_message: bool,
    ) -> String {
        let material = format!(
            "atlas-result-cache/v1|query={query}|snapshot={snapshot_id}|ceiling={ceiling}|{}",
            self.canonical_key_material(expand, group_by_session, with_message)
        );
        let digest = Sha256::digest(material.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// A search job submission. This is the complete one-shot CLI
/// search surface translated to the wire; the server maps it to
/// atlas-core's `SearchOptions` after clamping `ceiling`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default)]
    pub filters: FilterSpec,
    /// The requested ceiling. The server clamps this to the
    /// authenticated client's cap and reports the result as the
    /// job's effective ceiling.
    #[serde(default)]
    pub ceiling: Sensitivity,
    /// Final result entries to compute (flat results, or sessions
    /// when `group_by_session` is set).
    #[serde(default = "default_k")]
    pub k: usize,
    /// Per-branch candidate pool depth. The server enforces the
    /// design maximum of 400.
    #[serde(default = "default_pool")]
    pub pool: usize,
    #[serde(default)]
    pub group_by_session: bool,
    #[serde(default = "default_expand")]
    pub expand: usize,
    /// Flattened on the wire so each JSON parameter name matches its CLI
    /// spelling without the leading `--`.
    #[serde(flatten)]
    pub parameters: SearchParametersV1,
    #[serde(default)]
    pub with_message: bool,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_embedder")]
    pub embedder: String,
    #[serde(default)]
    pub trunc: usize,
    #[serde(default)]
    pub vector_backend: VectorBackend,
    /// One-shot endpoint only: maximum entries in its first page.
    /// Job submissions ignore this field; job clients choose a
    /// page size on each results request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_limit: Option<usize>,
}

impl Default for SearchRequest {
    fn default() -> Self {
        SearchRequest {
            query: String::new(),
            mode: SearchMode::default(),
            filters: FilterSpec::default(),
            ceiling: Sensitivity::default(),
            k: default_k(),
            pool: default_pool(),
            group_by_session: false,
            expand: default_expand(),
            parameters: SearchParametersV1::default(),
            with_message: false,
            model: default_model(),
            embedder: default_embedder(),
            trunc: 0,
            vector_backend: VectorBackend::default(),
            page_limit: None,
        }
    }
}

impl SearchRequest {
    pub fn new(query: impl Into<String>) -> SearchRequest {
        SearchRequest {
            query: query.into(),
            ..SearchRequest::default()
        }
    }
}

/// The 202 response from `POST /v1/search/jobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCreated {
    pub job_id: String,
    /// Per-job capability token. It authorizes only this job's
    /// status/results/chunk/delete endpoints; it is not a client
    /// credential and cannot submit work.
    pub job_token: String,
    pub status: JobStatus,
    pub effective_ceiling: Sensitivity,
    /// Jobs ahead of / including this job in the bounded FIFO
    /// execution queue at response time (0 when it started
    /// immediately). Queueing is a protocol property: agent
    /// clients may poll while the CLI client blocks.
    #[serde(default)]
    pub queue_depth: usize,
    /// Zero-based position in the FIFO queue when queued; absent
    /// once the job is running or finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<usize>,
    pub submitted_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    #[default]
    Queued,
    Running,
    Done,
    Failed,
    Expired,
}

impl JobStatus {
    pub fn name(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BranchState {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchStates {
    pub fts: BranchState,
    pub vector: BranchState,
}

/// `GET /v1/search/jobs/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStatusResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub branch_states: BranchStates,
    pub effective_ceiling: Sensitivity,
    /// The requested per-branch pool actually used by the job.
    pub pool: usize,
    #[serde(default)]
    pub queue_depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<usize>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_count: Option<usize>,
    pub submitted_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    pub expires_at_ms: u64,
}

/// One immutable page from a completed job, or the one-shot
/// endpoint's first page. `envelope` is the search envelope with
/// its `results` array replaced by this page's slice; all other
/// envelope fields are identical on every page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchPage {
    pub job_id: String,
    pub status: JobStatus,
    pub effective_ceiling: Sensitivity,
    pub envelope: Json,
    pub results: Vec<Json>,
    pub next_cursor: Option<String>,
    pub total_results: usize,
}

/// One row of the per-client recovery list
/// (`GET /v1/search/jobs`): metadata only — never result content
/// and never the job's capability token. A session that
/// disconnected after submitting uses this to find its
/// outstanding jobs again; reading results still requires the
/// per-job capability token (or the owning client's token).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSummary {
    pub job_id: String,
    pub status: JobStatus,
    /// Query summary: the submitted query text and shape.
    pub query: String,
    pub mode: SearchMode,
    pub pool: usize,
    pub effective_ceiling: Sensitivity,
    pub submitted_at_ms: u64,
    pub expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<usize>,
}

/// `GET /v1/search/jobs` — the authenticated client's own
/// outstanding jobs, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobSummary>,
}

/// The response from `DELETE /v1/search/jobs/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDeleted {
    pub job_id: String,
    pub status: String,
}

/// The uniform error body. HTTP status carries the broad class;
/// `code` is the stable machine-readable detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: ApiError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_depth: Option<usize>,
}

impl ErrorResponse {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> ErrorResponse {
        ErrorResponse {
            error: ApiError {
                code: code.into(),
                message: message.into(),
                retry_after_seconds: None,
                queue_depth: None,
            },
        }
    }

    pub fn rate_limited(
        message: impl Into<String>,
        retry_after_seconds: u64,
        queue_depth: usize,
    ) -> ErrorResponse {
        ErrorResponse {
            error: ApiError {
                code: "rate_limited".to_string(),
                message: message.into(),
                retry_after_seconds: Some(retry_after_seconds),
                queue_depth: Some(queue_depth),
            },
        }
    }
}

/// `GET /health` — no auth, no corpus data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub embedder_state: String,
    pub queue_depth: usize,
    pub active_branch_executions: usize,
    pub active_jobs: usize,
}

/// `POST /v1/embed` request (#6788): embed one query text in the
/// production query-vector space. `trunc` absent/0 = full width.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EmbedRequest {
    pub query: String,
    #[serde(default)]
    pub trunc: Option<usize>,
}

/// `POST /v1/embed` response (#6788): the production-shaped query
/// vector (prompt applied, truncated, renormalized by atlas-core)
/// and the space it lives in, named so a caller can refuse a
/// vector from the wrong space before storing or using it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmbedResponse {
    pub model: String,
    pub dims: usize,
    pub vector: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_wire_roundtrip() {
        // #6788: a missing trunc is the full-width request; the
        // response names its model space and dimension so the
        // Worker can pin both before caching the vector.
        let req: EmbedRequest = serde_json::from_str(r#"{"query": "alpha"}"#).unwrap();
        assert_eq!(req.query, "alpha");
        assert_eq!(req.trunc, None);
        let resp = EmbedResponse {
            model: "m".to_string(),
            dims: 2,
            vector: vec![1.0, 0.0],
        };
        let wire = serde_json::to_value(&resp).unwrap();
        assert_eq!(wire["model"], serde_json::json!("m"));
        assert_eq!(wire["dims"], serde_json::json!(2));
        assert_eq!(wire["vector"], serde_json::json!([1.0, 0.0]));
    }

    #[test]
    fn sensitivity_clamp_uses_ladder() {
        assert_eq!(
            Sensitivity::SecretBearing.clamped_to(Sensitivity::Personal),
            Sensitivity::Personal
        );
        assert_eq!(
            Sensitivity::Shareable.clamped_to(Sensitivity::SecretBearing),
            Sensitivity::Shareable
        );
        assert_eq!(
            serde_json::to_string(&Sensitivity::LocalOnly).unwrap(),
            "\"local-only\""
        );
    }

    #[test]
    fn search_request_defaults_are_production_defaults() {
        let req: SearchRequest = serde_json::from_str("{\"query\":\"atlas\"}").unwrap();
        assert_eq!(req.mode, SearchMode::Hybrid);
        assert_eq!(req.ceiling, Sensitivity::Personal);
        assert_eq!(req.k, 10);
        assert_eq!(req.pool, 100);
        assert_eq!(req.vector_backend, VectorBackend::Scan);
        assert_eq!(req.parameters, SearchParametersV1::default());
    }

    #[test]
    fn query_parameters_are_flattened_and_keep_cli_json_names() {
        let raw = r#"{
            "query":"atlas",
            "parameters_version":1,
            "shard_top_k":32,
            "global_top_k":8,
            "fts_weight":1.5,
            "vector_weight":0.75,
            "jaccard_depth":0,
            "jaccard_threshold":0.0,
            "rrf_k":20,
            "planner_enabled":false,
            "spell_enabled":false,
            "query_class_hint":null,
            "variant":"raw",
            "raw":true
        }"#;
        let req: SearchRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.parameters.shard_top_k, 32);
        assert_eq!(req.parameters.global_top_k, 8);
        assert_eq!(req.parameters.rrf_k, 20);
        assert!(req.parameters.raw);
        let wire = serde_json::to_value(req).unwrap();
        assert_eq!(wire["shard_top_k"], 32);
        assert_eq!(wire["global_top_k"], 8);
        assert_eq!(wire["rrf_k"], 20);
        assert_eq!(wire["raw"], true);
        assert!(wire.get("parameters").is_none());
    }

    #[test]
    fn unsupported_or_invalid_query_parameters_are_rejected() {
        let defaults = SearchParametersV1::default();
        assert!(defaults.validate(0).is_ok());
        // Jaccard controls are implemented (#5733): depth is a plain
        // count, and the threshold is range-checked instead.
        let mut p = defaults.clone();
        p.jaccard_depth = 10;
        assert!(p.validate(0).is_ok());
        let mut p = defaults.clone();
        p.jaccard_threshold = 1.5;
        assert!(p.validate(0).unwrap_err().contains("jaccard_threshold"));
        let mut p = defaults.clone();
        p.planner_enabled = true;
        assert!(p.validate(0).unwrap_err().contains("not implemented"));
        let mut p = defaults.clone();
        p.rrf_k = 0;
        assert!(p.validate(0).unwrap_err().contains("rrf_k"));
        let mut p = defaults.clone();
        p.fts_weight = f64::NAN;
        assert!(p.validate(0).unwrap_err().contains("finite"));
        let mut p = defaults;
        p.raw = true;
        assert!(p.validate(1).unwrap_err().contains("raw mode"));
    }

    #[test]
    fn protocol_enums_round_trip() {
        assert_eq!(
            VectorBackend::from_name("index-fill"),
            Some(VectorBackend::IndexFill)
        );
        assert_eq!(SearchMode::from_name("vector"), Some(SearchMode::Vector));
        assert_eq!(
            serde_json::to_string(&JobStatus::Queued).unwrap(),
            "\"queued\""
        );
    }

    #[test]
    fn flag_defaults_come_from_the_versioned_json() {
        let p = SearchParametersV1::default();
        assert!(!p.no_cache);
        assert!(!p.wait);
        assert_eq!(p.hydrate, 0);
        assert_eq!(p.raw_mode, RawMode::Off);
        // A request JSON that predates slice 2 (no flag fields)
        // still parses, with the same defaults.
        let old: SearchParametersV1 =
            serde_json::from_str("{\"raw\": false}").expect("old shape parses");
        assert_eq!(old, p);
    }

    #[test]
    fn every_flag_changes_the_result_cache_key() {
        let base = SearchParametersV1::default();
        let key = |p: &SearchParametersV1, expand: usize, group: bool, wm: bool| {
            p.result_cache_key("query", "snap1", "personal", expand, group, wm)
        };
        let base_key = key(&base, 0, false, false);
        // Deterministic: same inputs, same key.
        assert_eq!(base_key, key(&base, 0, false, false));
        assert_eq!(base_key.len(), 64);
        // Query / snapshot / ceiling sensitivity.
        assert_ne!(
            base_key,
            base.result_cache_key("other", "snap1", "personal", 0, false, false)
        );
        assert_ne!(
            base_key,
            base.result_cache_key("query", "snap2", "personal", 0, false, false)
        );
        assert_ne!(
            base_key,
            base.result_cache_key("query", "snap1", "local-only", 0, false, false)
        );
        // Each slice-2 flag, flipped one at a time.
        let mut p = base.clone();
        p.no_cache = true;
        assert_ne!(
            base_key,
            key(&p, 0, false, false),
            "no_cache must be in the key"
        );
        let mut p = base.clone();
        p.wait = true;
        assert_ne!(
            base_key,
            key(&p, 0, false, false),
            "wait must be in the key"
        );
        let mut p = base.clone();
        p.hydrate = 3;
        assert_ne!(
            base_key,
            key(&p, 0, false, false),
            "hydrate must be in the key"
        );
        let mut p = base.clone();
        p.raw_mode = RawMode::Ungrouped;
        assert_ne!(
            base_key,
            key(&p, 0, false, false),
            "raw_mode must be in the key"
        );
        let mut p = base.clone();
        p.raw_mode = RawMode::Branches;
        assert_ne!(
            base_key,
            key(&p, 0, false, false),
            "raw_mode=branches must be in the key"
        );
        // Request-level shaping controls.
        assert_ne!(
            base_key,
            key(&base, 2, false, false),
            "expand must be in the key"
        );
        assert_ne!(
            base_key,
            key(&base, 0, true, false),
            "group_by_session must be in the key"
        );
        assert_ne!(
            base_key,
            key(&base, 0, false, true),
            "with_message must be in the key"
        );
    }

    proptest::proptest! {
        /// Property: the key is a function of exactly the inputs
        /// it claims -- two parameter sets that differ in any
        /// single rendered field produce different keys, and
        /// identical inputs always produce the same key.
        #[test]
        fn result_cache_key_is_flag_sensitive(
            no_cache in proptest::prelude::any::<bool>(),
            wait in proptest::prelude::any::<bool>(),
            hydrate in 0usize..5,
            raw_mode_idx in 0usize..3,
            expand in 0usize..4,
            group in proptest::prelude::any::<bool>(),
        ) {
            let modes = [RawMode::Off, RawMode::Ungrouped, RawMode::Branches];
            let mut p = SearchParametersV1::default();
            p.no_cache = no_cache;
            p.wait = wait;
            p.hydrate = hydrate;
            p.raw_mode = modes[raw_mode_idx];
            let k1 = p.result_cache_key("q", "s", "personal", expand, group, false);
            let k2 = p.result_cache_key("q", "s", "personal", expand, group, false);
            proptest::prop_assert_eq!(&k1, &k2);
            // Flip exactly one component: the key must move.
            let mut q = p.clone();
            q.no_cache = !q.no_cache;
            proptest::prop_assert_ne!(&k1, &q.result_cache_key("q", "s", "personal", expand, group, false));
            let mut q = p.clone();
            q.hydrate += 1;
            proptest::prop_assert_ne!(&k1, &q.result_cache_key("q", "s", "personal", expand, group, false));
            let mut q = p.clone();
            q.raw_mode = modes[(raw_mode_idx + 1) % 3];
            proptest::prop_assert_ne!(&k1, &q.result_cache_key("q", "s", "personal", expand, group, false));
        }
    }
}
