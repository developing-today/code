//! The Atlas service wire contract, mirrored (fleet step 5, #5433).
//!
//! The contract of record is `atlas-proto`
//! (pkgs/atlas-search/crates/atlas-proto): the job protocol the
//! deployed atlas-search service speaks and its own CLI client is
//! built against. That crate is *untouched* by this step and stays
//! the server's side of the contract. The fleet workspace's nix
//! source is this directory alone (pkgs/fleet/default.nix:
//! `src = ./.`), so the fleet bin cannot path-depend on the atlas
//! workspace; the types the client needs are mirrored here, field
//! for field, exactly as fleet-dispatch mirrors pm-dispatch's API
//! shapes. Policy — especially sensitivity clamping — lives in the
//! server; this module merely carries the requested ceiling, and
//! the server reports the effective one.

// The mirror keeps atlas-proto's accessor surface complete (the
// contract is the point), even where this client does not call
// every accessor yet.
#![allow(dead_code)]

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

/// Classification/date filters. The ceiling is a sibling on
/// [`SearchRequest`], not a member here: the protocol makes the
/// requested/effective ceiling distinction explicit.
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

/// A search job submission — the complete one-shot CLI search
/// surface translated to the wire (atlas-proto's `SearchRequest`).
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
    #[serde(default = "default_k")]
    pub k: usize,
    /// Per-branch candidate pool depth. The server enforces the
    /// design maximum of 400 (the CLI validates before submitting).
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
    /// One-shot endpoint only; job submissions leave this unset.
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
    /// status/results endpoints; it is not a client credential and
    /// cannot submit work. Never printed or logged.
    pub job_token: String,
    pub status: JobStatus,
    pub effective_ceiling: Sensitivity,
    #[serde(default)]
    pub queue_depth: usize,
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

/// `GET /v1/search/jobs/{id}`. Required fields mirror atlas-proto
/// exactly (the server always sends them); fields the client does
/// not consume are carried as opaque JSON or defaulted, per
/// atlas-proto's own serde attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStatusResponse {
    pub job_id: String,
    pub status: JobStatus,
    /// Per-branch execution states; the client does not branch on
    /// them, but the field is required on the wire.
    pub branch_states: Json,
    pub effective_ceiling: Sensitivity,
    #[serde(default)]
    pub pool: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub submitted_at_ms: u64,
    pub expires_at_ms: u64,
}

/// One immutable page from a completed job. `envelope` is the
/// search envelope with its `results` array replaced by this
/// page's slice; all other envelope fields are identical on every
/// page.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitivity_names_match_atlas_proto() {
        for (name, want) in [
            ("shareable", Sensitivity::Shareable),
            ("personal", Sensitivity::Personal),
            ("local-only", Sensitivity::LocalOnly),
            ("secret-bearing", Sensitivity::SecretBearing),
        ] {
            assert_eq!(Sensitivity::from_name(name), Some(want));
            assert_eq!(want.name(), name);
            assert_eq!(serde_json::to_string(&want).unwrap(), format!("\"{name}\""));
        }
        assert_eq!(Sensitivity::from_name("bogus"), None);
    }

    #[test]
    fn search_request_defaults_are_production_defaults() {
        // The oracle is atlas-proto's own defaults test: a bare
        // {"query": ...} body must mean hybrid / personal / k=10 /
        // pool=100 / scan on the wire.
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
