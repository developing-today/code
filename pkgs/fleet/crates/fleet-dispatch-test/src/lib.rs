//! fleet-dispatch-test — the dispatch-test domain of the fleet
//! multicall binary (design: developing-today/code#6580).
//!
//! An explicit-dispatch CI test surface over GitHub Actions:
//!
//!   fleet dispatch-test scan    count active Actions jobs in each
//!                               execution-home copy and name the
//!                               first one with free capacity + the
//!                               generic workflow present
//!   fleet dispatch-test submit  store a job row in the Turso jobs
//!                               store, then dispatch the generic
//!                               workflow (fleet-dispatch-test.yml)
//!                               in the chosen copy with the job id
//!   fleet dispatch-test status  print one stored job row as JSON
//!
//! Nothing here is trigger-driven: every verb runs only when a
//! caller invokes it, and `submit` dispatches a workflow_dispatch
//! run explicitly. No Actions triggers are enabled by this crate.
//!
//! Execution homes (phase 2, Drew's corrected design): ONE repo —
//! the fleet repo (default `dezren39/fleet`, config
//! `[dispatch-test] fleet_repo`) — holds the other fleet repos as
//! git submodules and carries the generic workflow. Eligible
//! homes are its COPIES: the source repo plus every repo found by
//! following its fork network (forks, and forks of forks). The
//! scan discovers them per invocation unless `[dispatch-test]
//! scopes` pins an explicit list (the pin is an override, not the
//! default). A job names a SUBMODULE + REF + COMMANDS; the
//! generic workflow checks out the fleet repo, checks out the
//! named submodule at the named ref, and runs the commands inside
//! it, in the fleet-build container image whose baked Nix store
//! is the fleet repo's partial preloaded cache.
//!
//! Capacity counting (scan): for each scope repo, ACTIVE JOBS =
//! (jobs with status in_progress or queued across all in_progress
//! runs) + (queued runs, counted as 1 active unit each). Queued
//! runs have not started and expose no jobs yet, so the queued-run
//! term is an approximation: a queued run counts once no matter how
//! many jobs it will eventually fan out to.
//!
//! Auth: both tokens resolve **by name** through fleet-secrets —
//! the GitHub token (`[dispatch-test] token_name`, default
//! `github.ghcr-classic-pat`) and the Turso token
//! (`[dispatch-test] turso_token_name`, default
//! `turso.fleet-dispatch-jobs-token`). Values enter exactly one
//! Authorization header per request; they are never printed,
//! logged, or stored.
//!
//! Exit codes follow the workspace conventions: 0 ok · 2 usage ·
//! 3 transport down / credentials unresolved or no eligible scope ·
//! 4 an HTTP error status from GitHub or Turso (including an
//! unknown job id on `status`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fleet_config::Config;
use fleet_secrets::{Resolver, SecretValue};
use serde_json::{Map, Value, json};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_TRANSPORT: i32 = 3;
const EXIT_HTTP: i32 = 4;

const GITHUB_API_BASE: &str = "https://api.github.com";
const WORKFLOW_FILE: &str = "fleet-dispatch-test.yml";
const DEFAULT_COMMANDS: &str =
    "set -eux; echo dispatch-test smoke; git rev-parse HEAD; uname -a";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum DtError {
    Usage(String),
    Transport(String),
    Http { status: u16, message: String },
}

impl DtError {
    fn exit_code(&self) -> i32 {
        match self {
            DtError::Usage(_) => EXIT_USAGE,
            DtError::Transport(_) => EXIT_TRANSPORT,
            DtError::Http { .. } => EXIT_HTTP,
        }
    }
    fn message(&self) -> String {
        match self {
            DtError::Usage(m) | DtError::Transport(m) => m.clone(),
            DtError::Http { status, message } => format!("HTTP {status}: {message}"),
        }
    }
}

type Result<T> = std::result::Result<T, DtError>;

fn usage(msg: impl Into<String>) -> DtError {
    DtError::Usage(msg.into())
}

fn transport_err(msg: impl Into<String>) -> DtError {
    DtError::Transport(msg.into())
}

fn http_err(status: u16, message: impl Into<String>) -> DtError {
    DtError::Http {
        status,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// Time + job ids (pure)
// ---------------------------------------------------------------------------

/// RFC3339 UTC for a unix-seconds timestamp, seconds precision
/// (e.g. `2026-11-06T17:00:00Z`). Hand-rolled: this crate carries no
/// date dependency.
fn rfc3339_utc(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs = unix_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_rfc3339() -> String {
    rfc3339_utc(now_unix_secs())
}

fn base36(mut n: u128) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while n > 0 {
        out.push(digits[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// 4 hex chars from /dev/urandom (2 bytes, read_exact — urandom
/// never EOFs, so an unbounded read would never return).
fn random_hex4() -> Result<String> {
    use std::io::Read;
    let mut b = [0u8; 2];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| transport_err(format!("cannot read /dev/urandom: {e}")))?;
    Ok(format!("{:02x}{:02x}", b[0], b[1]))
}

/// Job id shape: `dt-<unix_millis base36>-<4 hex chars>`.
fn job_id_at(unix_millis: u128, hex4: &str) -> String {
    format!("dt-{}-{}", base36(unix_millis), hex4)
}

fn generate_job_id() -> Result<String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Ok(job_id_at(millis, &random_hex4()?))
}

// ---------------------------------------------------------------------------
// Arg parsing (fleet-dispatch's workspace-style minimal parser)
// ---------------------------------------------------------------------------

struct Args {
    values: Map<String, Value>,
    flags: Map<String, Value>,
}

fn parse_args(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<Args> {
    let mut values = Map::new();
    let mut flags = Map::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let Some(long) = arg.strip_prefix("--") else {
            return Err(usage(format!("unexpected argument {arg:?}")));
        };
        let (name, inline) = match long.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (long, None),
        };
        if bool_flags.contains(&name) {
            if inline.is_some() {
                return Err(usage(format!("--{name} takes no value")));
            }
            flags.insert(name.to_string(), Value::Bool(true));
        } else if value_flags.contains(&name) {
            let value = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| usage(format!("--{name} needs a value")))?
                }
            };
            values.insert(name.to_string(), Value::String(value));
        } else {
            return Err(usage(format!("unknown option --{name}")));
        }
        i += 1;
    }
    Ok(Args { values, flags })
}

impl Args {
    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(|v| v.as_str())
    }
    fn flag(&self, name: &str) -> bool {
        self.flags
            .get(name)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

fn parse_threshold(raw: Option<&str>, config: &Config) -> Result<u64> {
    match raw {
        None => Ok(config.dispatch_test_threshold()),
        Some(v) => v.parse::<u64>().map_err(|_| {
            usage(format!(
                "--threshold must be a non-negative integer, got {v:?}"
            ))
        }),
    }
}

// ---------------------------------------------------------------------------
// Secrets (by name, through fleet-secrets — the one path)
// ---------------------------------------------------------------------------

fn resolve_named(config: &Config, name: &str) -> Result<SecretValue> {
    Resolver::default_chain(config)
        .resolve(name)
        .map_err(|e| transport_err(e.to_string()))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into()
}

// ---------------------------------------------------------------------------
// GitHub API
// ---------------------------------------------------------------------------

struct Github {
    token: String,
    agent: ureq::Agent,
}

impl Github {
    fn new(config: &Config) -> Result<Github> {
        let token = resolve_named(config, config.dispatch_test_token_name())?;
        Ok(Github {
            token: token.expose().to_string(),
            agent: agent(),
        })
    }

    /// One GitHub exchange. 2xx returns the parsed body (empty body
    /// → Null, e.g. the 204 from a workflow dispatch); any other
    /// status is an Http error carrying GitHub's `message` field.
    fn exchange(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{GITHUB_API_BASE}{path}");
        let auth = format!("Bearer {}", self.token);
        let mut resp = match method {
            "GET" => self
                .agent
                .get(&url)
                .header("Authorization", &auth)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "fleet-dispatch-test")
                .call()
                .map_err(|e| transport_err(format!("GitHub API unreachable: {e}")))?,
            _ => self
                .agent
                .post(&url)
                .header("Authorization", &auth)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "fleet-dispatch-test")
                .header("Content-Type", "application/json")
                .send(
                    serde_json::to_string(body.unwrap_or(&Value::Null))
                        .unwrap_or_default()
                        .as_str(),
                )
                .map_err(|e| transport_err(format!("GitHub API unreachable: {e}")))?,
        };
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| transport_err(format!("cannot read GitHub response: {e}")))?;
        if (200..300).contains(&status) {
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text)
                .map_err(|e| transport_err(format!("GitHub answered a non-JSON body: {e}")));
        }
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| {
                let snippet: String = text.chars().take(300).collect();
                if snippet.is_empty() {
                    "no message".to_string()
                } else {
                    snippet
                }
            });
        Err(http_err(status, format!("GitHub API: {message}")))
    }

    fn get(&self, path: &str) -> Result<Value> {
        self.exchange("GET", path, None)
    }

    /// GET a list endpoint completely: page through it with
    /// `fetch_all_pages` and return one merged body shaped like a
    /// single-page response (#6605).
    fn get_paginated(&self, path: &str) -> Result<Value> {
        let sep = if path.contains('?') { '&' } else { '?' };
        fetch_all_pages(|page| self.get(&format!("{path}{sep}per_page={PAGE_SIZE}&page={page}")))
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.exchange("POST", path, Some(body))
    }
}

// ---------------------------------------------------------------------------
// Scan (pure counting over GitHub response shapes, then the live walk)
// ---------------------------------------------------------------------------

/// Page size for every list the scan reads, and the loud cap on
/// pages per list: 100 pages x 100 items = 10,000 entries is far
/// past any real home's lists, and past the cap the read errors
/// instead of presenting a page-bound count as exact (#6605).
const PAGE_SIZE: u64 = 100;
const MAX_PAGES: u64 = 100;

/// The items of one list page: the array of a bare-array body
/// (forks), or the single array field of an object body
/// (`workflow_runs` / `jobs` / `workflows`).
fn page_items(body: &Value) -> Vec<Value> {
    if let Some(items) = body.as_array() {
        return items.clone();
    }
    if let Some(obj) = body.as_object() {
        for value in obj.values() {
            if let Some(items) = value.as_array() {
                return items.clone();
            }
        }
    }
    Vec::new()
}

/// Append `next` page's items onto `merged` (the first page's body,
/// kept as the shape template); an object body's `total_count` is
/// rewritten to the merged length so it can never again describe
/// only the first page.
fn append_page(merged: &mut Value, next: &Value) {
    let items = page_items(next);
    if let Some(arr) = merged.as_array_mut() {
        arr.extend(items);
        return;
    }
    if let Some(obj) = merged.as_object_mut() {
        let key = obj
            .iter()
            .find(|(_, v)| v.is_array())
            .map(|(k, _)| k.clone());
        if let Some(key) = key {
            if let Some(arr) = obj.get_mut(&key).and_then(|v| v.as_array_mut()) {
                arr.extend(items);
                let total = arr.len() as u64;
                if obj.contains_key("total_count") {
                    obj.insert("total_count".to_string(), json!(total));
                }
            }
        }
    }
}

/// Read a GitHub list to completion (#6605): request pages through
/// `fetch` (1-based page numbers) until a short page arrives, and
/// merge them into one body shaped like a single-page response, so
/// the counting helpers see every item. Counting only the first
/// page presented a lower bound as the exact `active` number and
/// could report a saturated home as free. Where a term needs only
/// a count, the response's `total_count` is the complete answer and
/// no pagination is needed — that is what the queued-runs term in
/// `scan_repo` does, deliberately unchanged.
fn fetch_all_pages<F: FnMut(u64) -> Result<Value>>(mut fetch: F) -> Result<Value> {
    let mut merged = fetch(1)?;
    let mut last_len = page_items(&merged).len() as u64;
    let mut page = 1;
    while last_len >= PAGE_SIZE {
        page += 1;
        if page > MAX_PAGES {
            return Err(transport_err(format!(
                "GitHub list exceeds {MAX_PAGES} pages of {PAGE_SIZE};                  refusing to present a truncated count as exact"
            )));
        }
        let next = fetch(page)?;
        last_len = page_items(&next).len() as u64;
        append_page(&mut merged, &next);
    }
    Ok(merged)
}

/// Run ids from a `GET .../actions/runs` body.
fn run_ids(runs_body: &Value) -> Vec<u64> {
    runs_body
        .get("workflow_runs")
        .and_then(|v| v.as_array())
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("id").and_then(|id| id.as_u64()))
                .collect()
        })
        .unwrap_or_default()
}

/// `total_count` from a list body (0 when absent/malformed).
fn total_count(body: &Value) -> u64 {
    body.get("total_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Jobs in a `GET .../runs/{id}/jobs` body that occupy capacity:
/// status `in_progress` or `queued`.
fn count_active_jobs(jobs_body: &Value) -> u64 {
    jobs_body
        .get("jobs")
        .and_then(|v| v.as_array())
        .map(|jobs| {
            jobs.iter()
                .filter(|j| {
                    matches!(
                        j.get("status").and_then(|s| s.as_str()),
                        Some("in_progress") | Some("queued")
                    )
                })
                .count() as u64
        })
        .unwrap_or(0)
}

/// Whether the workflows list names an active workflow whose path
/// ends with the pilot workflow file.
fn workflow_present(workflows_body: &Value) -> bool {
    workflows_body
        .get("workflows")
        .and_then(|v| v.as_array())
        .map(|wfs| {
            wfs.iter().any(|w| {
                let active = w
                    .get("state")
                    .and_then(|s| s.as_str())
                    .map(|s| s == "active")
                    .unwrap_or(true);
                let path_hit = w
                    .get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| p.ends_with(WORKFLOW_FILE))
                    .unwrap_or(false);
                active && path_hit
            })
        })
        .unwrap_or(false)
}

fn eligible(active: u64, threshold: u64, workflow: bool) -> bool {
    workflow && active < threshold
}

#[derive(Debug, Clone)]
struct RepoScan {
    repo: String,
    active: u64,
    workflow: bool,
}

impl RepoScan {
    fn eligible(&self, threshold: u64) -> bool {
        eligible(self.active, threshold, self.workflow)
    }
    fn to_json(&self, threshold: u64) -> Value {
        json!({
            "repo": self.repo,
            "active": self.active,
            "workflow": self.workflow,
            "eligible": self.eligible(threshold),
        })
    }
}

/// The live capacity walk for one repo (see crate docs for the
/// counting rule and the queued-run approximation).
fn scan_repo(github: &Github, repo: &str) -> Result<RepoScan> {
    let mut active: u64 = 0;
    let in_progress =
        github.get_paginated(&format!("/repos/{repo}/actions/runs?status=in_progress"))?;
    for run_id in run_ids(&in_progress) {
        let jobs = github.get_paginated(&format!("/repos/{repo}/actions/runs/{run_id}/jobs"))?;
        active += count_active_jobs(&jobs);
    }
    let queued = github.get(&format!(
        "/repos/{repo}/actions/runs?status=queued&per_page=100"
    ))?;
    active += total_count(&queued);
    let workflows = github.get_paginated(&format!("/repos/{repo}/actions/workflows"))?;
    Ok(RepoScan {
        repo: repo.to_string(),
        active,
        workflow: workflow_present(&workflows),
    })
}

fn scan_scopes(github: &Github, scopes: &[String]) -> Result<Vec<RepoScan>> {
    scopes.iter().map(|r| scan_repo(github, r)).collect()
}

fn choose(scans: &[RepoScan], threshold: u64) -> Option<&RepoScan> {
    scans.iter().find(|s| s.eligible(threshold))
}

// ---------------------------------------------------------------------------
// Discovery: the fleet repo's copies (source + fork network)
// ---------------------------------------------------------------------------

/// `full_name`s from a `GET /repos/{r}/forks` body.
fn fork_full_names(forks_body: &Value) -> Vec<String> {
    forks_body
        .as_array()
        .map(|forks| {
            forks
                .iter()
                .filter_map(|f| f.get("full_name").and_then(|n| n.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Append `name` to `scopes` when new; returns whether it was new.
/// (Pure dedup step, unit-tested.)
fn push_scope(scopes: &mut Vec<String>, name: &str) -> bool {
    if scopes.iter().any(|s| s == name) {
        false
    } else {
        scopes.push(name.to_string());
        true
    }
}

/// Discover the execution homes: the fleet repo itself, then its
/// fork network breadth-first (forks, then forks of forks, ...
/// depth-capped at 8 levels — the real network is 1 level today).
/// Order is selection order: source first, then copies in the
/// order the API names them.
fn discover_scopes(github: &Github, fleet_repo: &str) -> Result<Vec<String>> {
    let mut scopes = vec![fleet_repo.to_string()];
    let mut level = vec![fleet_repo.to_string()];
    for _ in 0..8 {
        let mut next: Vec<String> = Vec::new();
        for repo in &level {
            let body = github.get_paginated(&format!("/repos/{repo}/forks"))?;
            for name in fork_full_names(&body) {
                if push_scope(&mut scopes, &name) {
                    next.push(name);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    Ok(scopes)
}

/// The scope list + where it came from: the config pin when
/// `[dispatch-test] scopes` is non-empty, otherwise fork-network
/// discovery rooted at the fleet repo.
fn resolve_scopes(github: &Github, config: &Config) -> Result<(Vec<String>, &'static str)> {
    let pinned = config.dispatch_test_scopes();
    if !pinned.is_empty() {
        return Ok((pinned, "config"));
    }
    let scopes = discover_scopes(github, config.dispatch_test_fleet_repo())?;
    Ok((scopes, "fork-network"))
}

/// A submodule name is a single path segment of the fleet repo:
/// alphanumerics, `-`, `_`, `.` — never empty, never `.`/`..`,
/// never containing a slash (it is joined onto the checkout).
fn valid_submodule(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

// ---------------------------------------------------------------------------
// Turso jobs store (HTTPS pipeline, hrana-over-HTTP v2 shape)
// ---------------------------------------------------------------------------

// Integer args are part of the pipeline protocol (values encoded
// as JSON strings, #6580) and are exercised by the unit tests; the
// current verbs bind text/null only.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
enum TursoArg {
    Text(String),
    Integer(i64),
    Null,
}

impl TursoArg {
    fn to_json(&self) -> Value {
        match self {
            TursoArg::Text(v) => json!({"type": "text", "value": v}),
            TursoArg::Integer(v) => json!({"type": "integer", "value": v.to_string()}),
            TursoArg::Null => json!({"type": "null"}),
        }
    }
}

/// The pipeline request body for one execute + close.
fn pipeline_body(sql: &str, args: &[TursoArg]) -> Value {
    json!({
        "requests": [
            {
                "type": "execute",
                "stmt": {
                    "sql": sql,
                    "args": args.iter().map(|a| a.to_json()).collect::<Vec<_>>(),
                }
            },
            {"type": "close"}
        ]
    })
}

/// One Turso result set: column names + rows of typed cells already
/// converted to JSON values.
#[derive(Debug)]
struct TursoRows {
    cols: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn cell_to_json(cell: &Value) -> Value {
    let ty = cell.get("type").and_then(|t| t.as_str()).unwrap_or("null");
    let raw = cell.get("value");
    match ty {
        "text" => raw
            .and_then(|v| v.as_str())
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
        "integer" => raw
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .map(|n| json!(n))
            .unwrap_or(Value::Null),
        "float" => raw
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .and_then(|n| serde_json::Number::from_f64(n))
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "blob" => raw.cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Pull the first execute result out of a pipeline response body.
///
/// Failure shapes (proven live, #6580): a request-level failure is
/// a top-level `{"error": ...}` with no usable results at all, and
/// a per-request failure is `results[i].type == "error"`. Only
/// `results[i].type == "ok"` carries a result set — anything else
/// is a transport-class failure, never an empty result.
fn parse_pipeline_rows(body: &Value) -> Result<TursoRows> {
    if let Some(err) = body.get("error") {
        let msg = match err {
            Value::String(s) => s.clone(),
            other => other
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| other.to_string()),
        };
        return Err(transport_err(format!("Turso pipeline error: {msg}")));
    }
    let first = body
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| transport_err("Turso pipeline response has no results"))?;
    if first.get("type").and_then(|t| t.as_str()) != Some("ok") {
        let msg = first
            .get("error")
            .and_then(|e| match e {
                Value::String(s) => Some(s.clone()),
                other => other
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string),
            })
            .unwrap_or_else(|| "unknown Turso error".to_string());
        return Err(transport_err(format!("Turso pipeline error: {msg}")));
    }
    let result = first
        .get("response")
        .and_then(|r| r.get("result"))
        .ok_or_else(|| transport_err("Turso pipeline response has no result"))?;
    let cols = result
        .get("cols")
        .and_then(|c| c.as_array())
        .map(|cols| {
            cols.iter()
                .map(|c| {
                    c.get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let rows = result
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| cells.iter().map(cell_to_json).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(TursoRows { cols, rows })
}

struct Turso {
    url: String,
    token: String,
    agent: ureq::Agent,
}

impl Turso {
    fn new(config: &Config) -> Result<Turso> {
        let token = resolve_named(config, config.dispatch_test_turso_token_name())?;
        Ok(Turso {
            url: config
                .dispatch_test_turso_url()
                .trim_end_matches('/')
                .to_string(),
            token: token.expose().to_string(),
            agent: agent(),
        })
    }

    fn execute(&self, sql: &str, args: &[TursoArg]) -> Result<TursoRows> {
        let url = format!("{}/v2/pipeline", self.url);
        let auth = format!("Bearer {}", self.token);
        let body = pipeline_body(sql, args);
        let mut resp = self
            .agent
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .send(serde_json::to_string(&body).unwrap_or_default().as_str())
            .map_err(|e| transport_err(format!("Turso store unreachable: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| transport_err(format!("cannot read Turso response: {e}")))?;
        if !(200..300).contains(&status) {
            let snippet: String = text.chars().take(300).collect();
            return Err(http_err(status, format!("Turso store: {snippet}")));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| transport_err(format!("Turso answered a non-JSON body: {e}")))?;
        parse_pipeline_rows(&parsed)
    }
}

/// Phase-2 store migration, applied idempotently by the verbs
/// that touch the new columns: jobs gain `submodule` (the fleet
/// repo submodule the job runs in) and `commands` (the shell
/// commands run inside it). The MVP columns stay: `target_ref`
/// now carries the SUBMODULE ref (empty = the umbrella's recorded
/// SHA) and `script` mirrors `commands` so MVP-era readers still
/// see the payload. A duplicate-column error means the migration
/// already ran and is not a failure.
fn ensure_schema(turso: &Turso) -> Result<()> {
    for sql in [
        "ALTER TABLE jobs ADD COLUMN submodule TEXT",
        "ALTER TABLE jobs ADD COLUMN commands TEXT",
    ] {
        match turso.execute(sql, &[]) {
            Ok(_) => {}
            Err(e) if e.message().contains("duplicate column") => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

const SCAN_USAGE: &str = "\
Usage: fleet dispatch-test scan [--threshold N] [--json]

For each execution-home copy (in order): count ACTIVE JOBS via
the GitHub API — jobs with status in_progress or queued across
in_progress runs, plus queued runs counted as 1 active unit each
(approximation: a queued run has not started and exposes no jobs,
so it counts once regardless of its eventual fan-out). Also probe
whether the generic workflow (fleet-dispatch-test.yml) is
present and active. Copies are DISCOVERED — the fleet repo
([dispatch-test] fleet_repo, default dezren39/fleet) plus its
fork network — unless [dispatch-test] scopes pins a list.

Prints one line per scope:
  <repo> active=<n> workflow=<yes|no> eligible=<yes|no>
and a final 'chosen: <repo|none>' line — the first scope with
active < threshold (default 40) AND the workflow present.
--json prints the same facts as JSON.";

fn verb_scan(config: &Config, args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{SCAN_USAGE}");
        return Ok(());
    }
    let parsed = parse_args(args, &["threshold"], &["json"])?;
    let threshold = parse_threshold(parsed.get("threshold"), config)?;
    let github = Github::new(config)?;
    let (scopes, scopes_source) = resolve_scopes(&github, config)?;
    let scans = scan_scopes(&github, &scopes)?;
    let chosen = choose(&scans, threshold).map(|s| s.repo.clone());
    if parsed.flag("json") {
        let doc = json!({
            "threshold": threshold,
            "fleet_repo": config.dispatch_test_fleet_repo(),
            "scopes_source": scopes_source,
            "scopes": scans.iter().map(|s| s.to_json(threshold)).collect::<Vec<_>>(),
            "chosen": chosen,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "null".to_string())
        );
    } else {
        println!(
            "scopes source: {} (fleet repo {})",
            scopes_source,
            config.dispatch_test_fleet_repo()
        );
        for s in &scans {
            println!(
                "{} active={} workflow={} eligible={}",
                s.repo,
                s.active,
                if s.workflow { "yes" } else { "no" },
                if s.eligible(threshold) { "yes" } else { "no" }
            );
        }
        println!("chosen: {}", chosen.as_deref().unwrap_or("none"));
    }
    Ok(())
}

const SUBMIT_USAGE: &str = "\
Usage: fleet dispatch-test submit --submodule NAME [--threshold N]
                                  [--repo owner/name] [--ref REF]
                                  [--name TEXT]
                                  [--commands TEXT | --commands-file PATH]

Store a job in the Turso jobs store (status queued), pick an
execution-home copy (--repo forces one; otherwise the scan rule:
first discovered copy with active < threshold and the generic
workflow present), dispatch fleet-dispatch-test.yml in that copy
with the job id as its only input, and only then mark the row
dispatched — a failed dispatch leaves the row queued (#6598). The generic workflow checks out the fleet repo, checks
out the named submodule at the named ref, and runs the commands
inside it (fleet-build container: the fleet repo's preloaded
nix cache).

  --threshold N      capacity threshold (default: config, else 40)
  --repo owner/name  force this execution-home copy
  --submodule NAME   fleet-repo submodule to run in (required) —
                     e.g. code, opencode, openchamber, nix, hatch
  --ref REF          ref inside the SUBMODULE repo: branch, tag
                     or SHA (default: empty = the submodule stays
                     at the umbrella's recorded SHA)
  --name TEXT        job name recorded on the row
  --commands TEXT    shell commands run inside the submodule
  --commands-file PATH
                     read the commands from a file
                     (default: a trivial smoke command set)

Prints JSON: {job_id, repo, submodule, ref, active_at_scan,
threshold, status}.";

/// The submit tail's step order (#6598): record the chosen home,
/// dispatch the workflow, and only after the dispatch succeeds mark
/// the row 'dispatched'. A failure in any step stops the sequence, so
/// a failed dispatch leaves the row 'queued' (with its target repo
/// recorded) — the same truthful, retryable state as the
/// no-eligible-scope outcome — instead of a 'dispatched' row for a
/// workflow that will never run.
fn run_dispatch_sequence(
    set_target: impl FnOnce() -> Result<()>,
    dispatch: impl FnOnce() -> Result<()>,
    mark_dispatched: impl FnOnce() -> Result<()>,
) -> Result<()> {
    set_target()?;
    dispatch()?;
    mark_dispatched()
}

fn verb_submit(config: &Config, args: &[String]) -> Result<Value> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{SUBMIT_USAGE}");
        return Ok(Value::Null);
    }
    let parsed = parse_args(
        args,
        &[
            "threshold",
            "repo",
            "submodule",
            "ref",
            "name",
            "commands",
            "commands-file",
        ],
        &[],
    )?;
    if parsed.get("commands").is_some() && parsed.get("commands-file").is_some() {
        return Err(usage("--commands and --commands-file are mutually exclusive"));
    }
    let threshold = parse_threshold(parsed.get("threshold"), config)?;
    let submodule = match parsed.get("submodule") {
        Some(s) if valid_submodule(s) => s.to_string(),
        Some(s) => {
            return Err(usage(format!(
                "--submodule must be a single fleet-repo submodule name                  (letters, digits, '-', '_', '.'), got {s:?}"
            )));
        }
        None => {
            return Err(usage(
                "--submodule is required: the fleet-repo submodule to run in",
            ));
        }
    };
    let commands = match parsed.get("commands-file") {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| usage(format!("cannot read commands file {path}: {e}")))?,
        None => parsed
            .get("commands")
            .map(str::to_string)
            .unwrap_or_else(|| DEFAULT_COMMANDS.to_string()),
    };
    let target_ref = parsed.get("ref").unwrap_or("").to_string();
    let name = parsed.get("name").map(str::to_string);
    if let Some(repo) = parsed.get("repo") {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
            return Err(usage(format!("--repo must be owner/name, got {repo:?}")));
        }
    }

    let github = Github::new(config)?;
    let turso = Turso::new(config)?;
    ensure_schema(&turso)?;

    // (a) job id, (b) INSERT as queued. `script` mirrors
    // `commands` (MVP-era readers); `target_ref` is the
    // submodule ref ('' = recorded SHA).
    let id = generate_job_id()?;
    let created = now_rfc3339();
    turso.execute(
        "INSERT INTO jobs (id, created_at, name, target_repo, target_ref, script, status, \
         run_id, run_url, exit_code, output_tail, updated_at, submodule, commands) \
         VALUES (?, ?, ?, NULL, ?, ?, 'queued', NULL, NULL, NULL, NULL, ?, ?, ?)",
        &[
            TursoArg::Text(id.clone()),
            TursoArg::Text(created.clone()),
            name.map(TursoArg::Text).unwrap_or(TursoArg::Null),
            TursoArg::Text(target_ref.clone()),
            TursoArg::Text(commands.clone()),
            TursoArg::Text(created.clone()),
            TursoArg::Text(submodule.clone()),
            TursoArg::Text(commands),
        ],
    )?;

    // (c) pick the execution-home copy.
    let (repo, active_at_scan) = if let Some(forced) = parsed.get("repo") {
        let scan = scan_repo(&github, forced)?;
        (forced.to_string(), scan.active)
    } else {
        let (scopes, _source) = resolve_scopes(&github, config)?;
        let scans = scan_scopes(&github, &scopes)?;
        match choose(&scans, threshold) {
            Some(s) => (s.repo.clone(), s.active),
            None => {
                return Err(transport_err(format!(
                    "no eligible scope: no execution-home copy has active < {threshold} \
                     with the generic workflow present (job {id} remains queued in the store)"
                )));
            }
        }
    };

    // (d)+(e) record the chosen home, dispatch the generic workflow
    // at the copy's default branch, and only then mark the row
    // dispatched (#6598 — see run_dispatch_sequence).
    run_dispatch_sequence(
        || {
            turso.execute(
                "UPDATE jobs SET target_repo = ?, updated_at = ? WHERE id = ?",
                &[
                    TursoArg::Text(repo.clone()),
                    TursoArg::Text(now_rfc3339()),
                    TursoArg::Text(id.clone()),
                ],
            )?;
            Ok(())
        },
        || {
            let repo_body = github.get(&format!("/repos/{repo}"))?;
            let default_branch = repo_body
                .get("default_branch")
                .and_then(|b| b.as_str())
                .ok_or_else(|| {
                    transport_err(format!("GitHub repo {repo} has no default_branch"))
                })?;
            github.post(
                &format!("/repos/{repo}/actions/workflows/{WORKFLOW_FILE}/dispatches"),
                &json!({
                    "ref": default_branch,
                    "inputs": {"job_id": id},
                }),
            )?;
            Ok(())
        },
        || {
            turso.execute(
                "UPDATE jobs SET status = 'dispatched', updated_at = ? WHERE id = ?",
                &[
                    TursoArg::Text(now_rfc3339()),
                    TursoArg::Text(id.clone()),
                ],
            )?;
            Ok(())
        },
    )?;

    // (f) report.
    Ok(json!({
        "job_id": id,
        "repo": repo,
        "submodule": submodule,
        "ref": if target_ref.is_empty() { Value::Null } else { Value::String(target_ref) },
        "active_at_scan": active_at_scan,
        "threshold": threshold,
        "status": "dispatched",
    }))
}

const STATUS_USAGE: &str = "\
Usage: fleet dispatch-test status <job-id>

Print the stored jobs row for <job-id> as pretty JSON (all
columns). An unknown id is an error (exit 4).";

fn verb_status(config: &Config, args: &[String]) -> Result<Value> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{STATUS_USAGE}");
        return Ok(Value::Null);
    }
    if args.len() != 1 {
        return Err(usage(format!(
            "status takes exactly one job id\n\n{STATUS_USAGE}"
        )));
    }
    let id = &args[0];
    let turso = Turso::new(config)?;
    ensure_schema(&turso)?;
    let result = turso.execute(
        "SELECT id, created_at, name, target_repo, target_ref, submodule, commands, \
         script, status, run_id, run_url, exit_code, output_tail, updated_at \
         FROM jobs WHERE id = ?",
        &[TursoArg::Text(id.clone())],
    )?;
    let row = result
        .rows
        .first()
        .ok_or_else(|| http_err(404, format!("job {id} not found in the jobs store")))?;
    let mut obj = Map::new();
    for (i, col) in result.cols.iter().enumerate() {
        obj.insert(col.clone(), row.get(i).cloned().unwrap_or(Value::Null));
    }
    Ok(Value::Object(obj))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

const USAGE: &str = "\
Usage: fleet dispatch-test <command> [args...]

Explicit-dispatch CI test (developing-today/code#6580): scan
fleet repos for free GitHub Actions capacity, submit a job to
the Turso-backed jobs store and dispatch the pilot workflow,
then track the job's stored status.

  scan [--threshold N] [--json]   capacity + workflow scan of the fleet-repo copies
  submit [options]                store a job and dispatch the generic workflow
  status <job-id>                 print one stored job row as JSON

Run 'fleet dispatch-test <command> --help' for a verb's options.
Note: queued runs count as 1 active unit each in scan — a queued
run exposes no jobs yet, so that term is an approximation.";

fn print_json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".to_string())
    );
}

/// The dispatch-test domain's entry point (the bin crate routes
/// here once the capability checks have passed). Mirrors
/// fleet-dispatch's run signature and exit-code conventions.
pub fn run(config: &Config, args: &[String]) -> i32 {
    let verb = args.first().map(|s| s.as_str());
    let rest: &[String] = if args.is_empty() { &[][..] } else { &args[1..] };
    let result: Result<Option<Value>> = match verb {
        Some("scan") => verb_scan(config, rest).map(|_| None),
        Some("submit") => verb_submit(config, rest).map(Some),
        Some("status") => verb_status(config, rest).map(Some),
        Some(other) => Err(usage(format!(
            "unknown dispatch-test command {other:?}\n\n{USAGE}"
        ))),
        None => Err(usage(USAGE.to_string())),
    };
    match result {
        Ok(Some(Value::Null)) | Ok(None) => EXIT_OK,
        Ok(Some(body)) => {
            print_json(&body);
            EXIT_OK
        }
        Err(e) => {
            eprintln!("fleet dispatch-test: {}", e.message());
            e.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_jobs() -> Value {
        json!({
            "total_count": 4,
            "jobs": [
                {"id": 1, "status": "in_progress"},
                {"id": 2, "status": "queued"},
                {"id": 3, "status": "completed"},
                {"id": 4, "status": "in_progress"}
            ]
        })
    }

    #[test]
    fn active_jobs_count_only_running_and_queued() {
        assert_eq!(count_active_jobs(&sample_jobs()), 3);
        assert_eq!(count_active_jobs(&json!({"jobs": []})), 0);
        assert_eq!(count_active_jobs(&json!({})), 0);
    }

    #[test]
    fn run_ids_and_total_count_parse() {
        let body = json!({
            "total_count": 2,
            "workflow_runs": [{"id": 111}, {"id": 222}, {"name": "no-id"}]
        });
        assert_eq!(run_ids(&body), vec![111, 222]);
        assert_eq!(total_count(&body), 2);
        assert_eq!(total_count(&json!({})), 0);
    }

    #[test]
    fn paginated_runs_merge_all_pages() {
        // #6605: 150 in-progress runs over two pages must all be
        // visited — page 1 alone would visit only 100.
        let page = |items: Vec<Value>| json!({"total_count": 150, "workflow_runs": items});
        let mut calls = 0;
        let merged = fetch_all_pages(|n| {
            calls += 1;
            Ok(match n {
                1 => page((1..=100).map(|i| json!({"id": i})).collect()),
                _ => page((101..=150).map(|i| json!({"id": i})).collect()),
            })
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(run_ids(&merged).len(), 150);
        assert_eq!(total_count(&merged), 150);
    }

    #[test]
    fn paginated_jobs_count_beyond_first_page() {
        // #6605: one run fanning out to 250 jobs — the active jobs
        // on pages 2 and 3 count exactly like page 1's.
        let jobs_page = |statuses: &[&str]| json!({"total_count": 250, "jobs": statuses.iter().map(|s| json!({"status": s})).collect::<Vec<_>>()});
        let merged = fetch_all_pages(|n| {
            Ok(match n {
                1 => jobs_page(
                    &vec!["in_progress"; 30]
                        .into_iter()
                        .chain(vec!["completed"; 70])
                        .collect::<Vec<_>>(),
                ),
                2 => jobs_page(&vec!["queued"; 100]),
                _ => jobs_page(&vec!["in_progress"; 50]),
            })
        })
        .unwrap();
        assert_eq!(count_active_jobs(&merged), 180);
    }

    #[test]
    fn paginated_workflows_and_forks_beyond_first_page() {
        // #6605: the pilot workflow hiding on page 2 is present,
        // and forks past 100 are discovered as scopes.
        let workflows = fetch_all_pages(|n| {
            Ok(match n {
                1 => json!({"total_count": 101, "workflows": (0..100).map(|i| json!({"path": format!(".github/workflows/w{i}.yml"), "state": "active"})).collect::<Vec<_>>() }),
                _ => json!({"total_count": 101, "workflows": [json!({"path": ".github/workflows/fleet-dispatch-test.yml", "state": "active"})]}),
            })
        })
        .unwrap();
        assert!(workflow_present(&workflows));

        let forks = fetch_all_pages(|n| {
            Ok(match n {
                1 => json!(
                    (0..100)
                        .map(|i| json!({"full_name": format!("o/r{i}")}))
                        .collect::<Vec<_>>()
                ),
                _ => json!([json!({"full_name": "o/last"})]),
            })
        })
        .unwrap();
        let names = fork_full_names(&forks);
        assert_eq!(names.len(), 101);
        assert_eq!(names[100], "o/last");
    }

    #[test]
    fn pagination_exact_page_then_empty_and_error_propagates() {
        // A full page followed by an empty page ends the walk; a
        // transport error mid-walk fails the scan, never a
        // truncated count.
        let mut calls = 0;
        let merged = fetch_all_pages(|n| {
            calls += 1;
            Ok(match n {
                1 => json!(
                    (0..100)
                        .map(|i| json!({"full_name": format!("o/r{i}")}))
                        .collect::<Vec<_>>()
                ),
                _ => json!([]),
            })
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(fork_full_names(&merged).len(), 100);

        let failed = fetch_all_pages(|n| {
            if n == 1 {
                Ok(json!(
                    (0..100).map(|i| json!({"id": i})).collect::<Vec<_>>()
                ))
            } else {
                Err(transport_err("boom"))
            }
        });
        assert!(failed.is_err());
    }

    #[test]
    fn workflow_presence_needs_active_and_path_suffix() {
        let yes = json!({"workflows": [
            {"path": ".github/workflows/other.yml", "state": "active"},
            {"path": ".github/workflows/fleet-dispatch-test.yml", "state": "active"}
        ]});
        assert!(workflow_present(&yes));
        let disabled = json!({"workflows": [
            {"path": ".github/workflows/fleet-dispatch-test.yml", "state": "disabled_manually"}
        ]});
        assert!(!workflow_present(&disabled));
        assert!(!workflow_present(&json!({"workflows": []})));
    }

    #[test]
    fn eligibility_is_threshold_and_workflow() {
        assert!(eligible(39, 40, true));
        assert!(!eligible(40, 40, true));
        assert!(!eligible(0, 40, false));
    }

    #[test]
    fn job_id_shape() {
        let id = job_id_at(1_762_000_000_000, "0a9f");
        assert!(id.starts_with("dt-"));
        assert!(id.ends_with("-0a9f"));
        let mid = &id[3..id.len() - 5];
        assert!(!mid.is_empty());
        assert!(
            mid.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        let live = generate_job_id().unwrap();
        let suffix = live.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 4);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_735_689_600), "2025-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_735_689_600 + 3661), "2025-01-01T01:01:01Z");
    }

    #[test]
    fn pipeline_body_shape() {
        let body = pipeline_body(
            "INSERT INTO jobs (id, run_id) VALUES (?, ?)",
            &[TursoArg::Text("dt-x".to_string()), TursoArg::Integer(42)],
        );
        assert_eq!(body["requests"][0]["type"], "execute");
        assert_eq!(
            body["requests"][0]["stmt"]["sql"],
            "INSERT INTO jobs (id, run_id) VALUES (?, ?)"
        );
        assert_eq!(
            body["requests"][0]["stmt"]["args"][0],
            json!({"type": "text", "value": "dt-x"})
        );
        assert_eq!(
            body["requests"][0]["stmt"]["args"][1],
            json!({"type": "integer", "value": "42"})
        );
        assert_eq!(body["requests"][1]["type"], "close");
        let null_body = pipeline_body("SELECT ?", &[TursoArg::Null]);
        assert_eq!(
            null_body["requests"][0]["stmt"]["args"][0],
            json!({"type": "null"})
        );
    }

    #[test]
    fn pipeline_rows_parse_typed_cells() {
        let body = json!({
            "results": [{
                "type": "ok",
                "response": {
                    "type": "execute",
                    "result": {
                        "cols": [{"name": "id"}, {"name": "run_id"}, {"name": "name"}],
                        "rows": [[
                            {"type": "text", "value": "dt-abc"},
                            {"type": "integer", "value": "123"},
                            {"type": "null"}
                        ]]
                    }
                }
            }]
        });
        let rows = parse_pipeline_rows(&body).unwrap();
        assert_eq!(rows.cols, vec!["id", "run_id", "name"]);
        assert_eq!(rows.rows[0][0], json!("dt-abc"));
        assert_eq!(rows.rows[0][1], json!(123));
        assert_eq!(rows.rows[0][2], Value::Null);
    }

    #[test]
    fn pipeline_error_shapes_are_failures_not_empty_rows() {
        // Top-level error (the raw-integer-arg failure mode, #6580).
        let top = json!({"error": "JSON parse error: invalid type: integer `42`, expected a borrowed string"});
        let err = parse_pipeline_rows(&top).unwrap_err();
        assert_eq!(err.exit_code(), EXIT_TRANSPORT);
        assert!(err.message().contains("JSON parse error"));
        // Per-request error result.
        let per = json!({"results": [{
            "type": "error",
            "error": {"message": "no such table: jobs", "code": "SQLITE_ERROR"}
        }]});
        let err = parse_pipeline_rows(&per).unwrap_err();
        assert_eq!(err.exit_code(), EXIT_TRANSPORT);
        assert!(err.message().contains("no such table"));
    }

    #[test]
    fn fork_names_parse_and_scopes_dedup_in_order() {
        let body = json!([
            {"full_name": "developing-today/fleet"},
            {"full_name": "developing-today-forks/fleet"},
            {"name": "no-full-name"}
        ]);
        assert_eq!(
            fork_full_names(&body),
            vec![
                "developing-today/fleet".to_string(),
                "developing-today-forks/fleet".to_string()
            ]
        );
        assert!(fork_full_names(&json!({"forks": []})).is_empty());
        let mut scopes = vec!["dezren39/fleet".to_string()];
        assert!(push_scope(&mut scopes, "developing-today/fleet"));
        assert!(!push_scope(&mut scopes, "dezren39/fleet"));
        assert!(push_scope(&mut scopes, "developing-today-forks/fleet"));
        assert_eq!(
            scopes,
            vec![
                "dezren39/fleet",
                "developing-today/fleet",
                "developing-today-forks/fleet"
            ]
        );
    }

    #[test]
    fn submodule_names_are_single_path_segments() {
        for ok in ["code", "opencode", "openchamber", "nix", "hatch-personal", "dezren39-zettel"] {
            assert!(valid_submodule(ok), "{ok} should be valid");
        }
        for bad in ["", ".", "..", "../code", "code/sub", "a b", "a/b", "x;rm"] {
            assert!(!valid_submodule(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn dispatch_sequence_runs_target_dispatch_mark_in_order() {
        let calls = std::cell::RefCell::new(Vec::new());
        let result = run_dispatch_sequence(
            || {
                calls.borrow_mut().push("target");
                Ok(())
            },
            || {
                calls.borrow_mut().push("dispatch");
                Ok(())
            },
            || {
                calls.borrow_mut().push("mark");
                Ok(())
            },
        );
        assert!(result.is_ok());
        assert_eq!(*calls.borrow(), vec!["target", "dispatch", "mark"]);
    }

    #[test]
    fn dispatch_sequence_failure_never_marks_dispatched() {
        // The #6598 regression: the dispatch call fails (HTTP 422, a
        // missing workflow_dispatch trigger) and the row must not end
        // up marked dispatched.
        let calls = std::cell::RefCell::new(Vec::new());
        let result = run_dispatch_sequence(
            || {
                calls.borrow_mut().push("target");
                Ok(())
            },
            || {
                calls.borrow_mut().push("dispatch");
                Err(transport_err("GitHub API: workflow does not have a workflow_dispatch trigger"))
            },
            || {
                calls.borrow_mut().push("mark");
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(*calls.borrow(), vec!["target", "dispatch"]);
    }

    #[test]
    fn dispatch_sequence_target_failure_stops_before_dispatch() {
        let calls = std::cell::RefCell::new(Vec::new());
        let result = run_dispatch_sequence(
            || {
                calls.borrow_mut().push("target");
                Err(transport_err("store unreachable"))
            },
            || {
                calls.borrow_mut().push("dispatch");
                Ok(())
            },
            || {
                calls.borrow_mut().push("mark");
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(*calls.borrow(), vec!["target"]);
    }

    #[test]
    fn exit_code_conventions() {
        assert_eq!(usage("x").exit_code(), 2);
        assert_eq!(transport_err("x").exit_code(), 3);
        assert_eq!(http_err(404, "x").exit_code(), 4);
    }

    #[test]
    fn help_needs_no_credentials() {
        // Help returns before any token resolution, so these calls
        // double as no-network pins.
        let config = Config::default();
        assert_eq!(
            run(&config, &["scan".to_string(), "--help".to_string()]),
            EXIT_OK
        );
        assert_eq!(
            verb_status(&config, &["a".to_string(), "b".to_string()])
                .unwrap_err()
                .exit_code(),
            EXIT_USAGE
        );
    }
}
