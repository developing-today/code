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
    10
}

fn default_pool() -> usize {
    100
}

fn default_model() -> String {
    "Snowflake/snowflake-arctic-embed-m-v2.0".to_string()
}

fn default_embedder() -> String {
    "auto".to_string()
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
    #[serde(default)]
    pub expand: usize,
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
            expand: 0,
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
}
