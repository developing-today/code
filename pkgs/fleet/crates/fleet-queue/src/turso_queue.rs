//! queue-broker Turso backend — the queue core on libSQL (#6741).
//!
//! The Postgres queue core (agent.tasks / leases / lanes / workers /
//! checkpoints / repo_task_lane_ledger / events in muse-dev2) was the
//! one piece of the 2026-11-04..06 migration (#5689/#5690 series)
//! deliberately kept on Postgres for transactional coupling. When
//! the muse-dev2 data plane went down on 2026-11-06 (#6619, Supabase
//! platform fault) the queue core had no live home; the GitHub
//! fallback mode (doc/fleet/queue-fallback-mode.md) carried the work.
//! This module is the dedicated-Turso replacement Drew ordered the
//! same evening: a dedicated database (`fleet-queue`, org dezren39,
//! group great-star-lord; rw token in BWS as
//! `turso.fleet-queue-rw-token`) behind the SAME broker protocol, so
//! no client changes: backend selection is the broker's config.
//!
//! Selection: `QUEUE_BROKER_BACKEND=turso` (default `postgres` — the
//! production default does NOT flip with this code; cutover is the
//! runbook in doc/fleet/queue-turso-cutover.md). URL:
//! `QUEUE_BROKER_TURSO_URL` (default the fleet-queue HTTPS URL).
//! Token: `QUEUE_BROKER_TURSO_TOKEN_FILE`, else the secrets broker
//! by the name above.
//!
//! Schema + semantics are reconstructed from the recovered live PG
//! function bodies in pm-stack/opencode-claimant/migration-0021.sql
//! .. migration-0024.sql (claim_repo_task, fail_repo_task, park_task,
//! expire_leases, assign_repo_task_lane,
//! bind_repo_task_lane_session) plus the broker/client contract in
//! this crate for the verbs whose PG bodies were not recovered
//! (enqueue dedupe, heartbeat, checkpoint, block, complete, cancel,
//! record execution, health aggregation). Every deviation from the
//! PG behavior is listed in the cutover runbook.
//!
//! agent.events (#5690): the PG stream's queue-transition writers
//! are this backend's `event()` calls (same dispatch, same database
//! as the task write — the closest libSQL gets to the PG RPCs'
//! single-transaction coupling; see doc/fleet/agent-events-turso.md
//! for the full writer/reader inventory and the coupling verdict).
//! The non-queue writers (github-ci webhooks, fleet-fetch) and the
//! stream's consumers get broker verbs here: /events-append,
//! /events-seen (the event_seen ready-condition primitive),
//! /events-count (observability + health), /events-prune (the
//! prune_events port). All of it exists only on this backend; the
//! Postgres dispatch is untouched.
//!
//! Atomicity: libSQL is single-writer. A claim is ONE conditional
//! UPDATE whose WHERE clause re-checks claimability (status in
//! queued/parked with a due not_before, attempts left) against the
//! row a subquery picks — the same shape as PG's FOR UPDATE SKIP
//! LOCKED pick, with the write serialization libSQL already
//! guarantees. A competing claim after the first lands matches no
//! row and returns `{row: null}` (client exit 5), exactly the PG
//! envelope.

use serde_json::{Map, Value, json};

use crate::validate::{task_from_packet, validate_packet};

pub const DEFAULT_TURSO_URL: &str = "https://fleet-queue-dezren39.aws-us-east-1.turso.io";
pub const TURSO_TOKEN_SECRET_NAME: &str = "turso.fleet-queue-rw-token";

const TASK_COLS: &str = "id, queue, title, body, payload, dedupe_key, priority, status, \
    not_before, worker_id, claimed_at, lease_until, lane_id, platform, attempts, max_attempts, \
    claim_provider, claim_model, claim_variant, \
    progress, result, last_error, park_kind, wait_kind, ready_condition, \
    last_checked_at, check_outcome, created_at, updated_at, completed_at, \
    blocked_deps, dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds";
const LEDGER_COLS: &str = "id, queue, task_id, assigned_lane, executed_lane, \
    exclusion_reason, opencode_reason, platform_limit, platform_limit_reason, \
    assigned_by, session_id, fallback_reason, \
    assigned_at, executed_at, updated_at";
/// Task columns whose stored TEXT is JSON (PG jsonb) — parsed back
/// into values so the wire shape matches the Postgres broker's.
const TASK_JSON_COLS: [&str; 5] = [
    "payload",
    "progress",
    "result",
    "ready_condition",
    "blocked_deps",
];

// ---------------------------------------------------------------------------
// Time (no chrono in this crate: ISO-8601 UTC strings, lexicographic
// order == chronological order, the same trick the list/claim SQL
// relies on)
// ---------------------------------------------------------------------------

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn iso(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_iso() -> String {
    iso(now_unix())
}

fn after_iso(secs: i64) -> String {
    iso(now_unix() + secs)
}

// ---------------------------------------------------------------------------
// libSQL HTTP pipeline client (same wire as fleet-dispatch-test, #6580)
// ---------------------------------------------------------------------------

enum Arg {
    Text(String),
    Int(i64),
    Null,
}

impl Arg {
    fn to_json(&self) -> Value {
        match self {
            Arg::Text(v) => json!({"type": "text", "value": v}),
            Arg::Int(v) => json!({"type": "integer", "value": v.to_string()}),
            Arg::Null => json!({"type": "null"}),
        }
    }
}

fn arg_str(v: Option<&str>) -> Arg {
    v.map(|s| Arg::Text(s.to_string())).unwrap_or(Arg::Null)
}

fn arg_json(v: &Value) -> Arg {
    if v.is_null() {
        Arg::Null
    } else {
        Arg::Text(v.to_string())
    }
}

struct StmtResult {
    cols: Vec<String>,
    rows: Vec<Vec<Value>>,
    affected: u64,
    last_insert_rowid: Option<i64>,
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
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}


// ---------------------------------------------------------------------------
// Dependency blocking (#8276): a block declared ONCE, structurally,
// and watched by the broker — lanes never re-verify gates by hand.
//
// A task blocked WITH deps carries them on the row: blocked_deps
// (a JSON array), dep_next_recheck_at (when the broker next
// looks), dep_blocked_until (the max-TTL deadline), and
// dep_recheck_seconds. A block WITHOUT deps is the classic
// free-text block (an escalation, a human decision): the
// rechecker never touches it, and it never auto-clears.
//
// Clearance per dep kind:
//   task  — the dep task reaches a terminal state. done clears;
//           cancelled/failed release too (the dependent's lane
//           re-evaluates instead of waiting forever), with the
//           dep's outcome named in the event evidence.
//   issue — the GitHub issue is CLOSED.
//   pr    — the GitHub PR is MERGED or CLOSED.
//   ci    — the Actions run is completed, any conclusion (the
//           claiming lane evaluates success/failure; a failed
//           run must not hold the dependent hostage).
// A reference that cannot be resolved at all (GitHub 404, unknown
// task id) is BROKEN: it releases at that recheck, with the reason
// in the event — a typo'd gate is not a life sentence. A check
// that FAILS (rate limit, token down, network) is neither
// clearance nor expiry: it reschedules the recheck, and only the
// block's max TTL can release the task from a broken checker.
// ---------------------------------------------------------------------------

pub const DEP_RECHECK_DEFAULT_SECONDS: i64 = 900; // 15 min
pub const DEP_RECHECK_MIN_SECONDS: i64 = 300;
pub const DEP_RECHECK_MAX_SECONDS: i64 = 3600;
pub const DEP_TTL_DEFAULT_SECONDS: i64 = 259_200; // 3 days
pub const DEP_TTL_MIN_SECONDS: i64 = 60;
pub const DEP_TTL_MAX_SECONDS: i64 = 604_800; // 7 days

pub fn clamp_dep_recheck_seconds(v: Option<i64>) -> i64 {
    v.unwrap_or(DEP_RECHECK_DEFAULT_SECONDS)
        .clamp(DEP_RECHECK_MIN_SECONDS, DEP_RECHECK_MAX_SECONDS)
}

pub fn clamp_dep_ttl_seconds(v: Option<i64>) -> i64 {
    v.unwrap_or(DEP_TTL_DEFAULT_SECONDS)
        .clamp(DEP_TTL_MIN_SECONDS, DEP_TTL_MAX_SECONDS)
}

/// One dependency observation: from the store (task deps) or from
/// GitHub via a DepProbe. The string is the human evidence that
/// lands on the dep record and in the audit event ("pr merged",
/// "run completed: failure", "github http 403").
#[derive(Clone, Debug, PartialEq)]
pub enum DepObs {
    /// Still open; the gate holds.
    Open(String),
    /// The blocking condition is satisfied; the dep releases.
    Cleared(String),
    /// The reference does not resolve (404 / unknown task); the
    /// dep releases, with the breakage named.
    Broken(String),
    /// The check itself could not be performed; nothing is
    /// inferred from it and the dep stays open.
    Failed(String),
}

/// The broker's GitHub window for dep checks (#8276). The
/// production implementation lives in queue_broker.rs, which owns
/// the secrets Resolver; tests inject fixtures. Implementations
/// must map every transport/auth problem to DepObs::Failed: a
/// broken checker never clears a gate.
pub trait DepProbe: Send + Sync {
    fn probe_issue(&self, repo: &str, number: i64) -> DepObs;
    fn probe_pr(&self, repo: &str, number: i64) -> DepObs;
    fn probe_ci(&self, repo: &str, run_id: i64) -> DepObs;
}

/// Stable identity of a dep, used for per-pass probe caching and
/// in event details: "pr:owner/repo#12", "task:123", ...
pub fn dep_key(dep: &Value) -> String {
    let kind = dep.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
    match kind {
        "pr" | "issue" => format!(
            "{kind}:{}#{}",
            dep.get("repo").and_then(|v| v.as_str()).unwrap_or("?"),
            dep.get("number").and_then(|v| v.as_i64()).unwrap_or(0)
        ),
        "ci" => format!(
            "ci:{}#{}",
            dep.get("repo").and_then(|v| v.as_str()).unwrap_or("?"),
            dep.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0)
        ),
        "task" => format!(
            "task:{}",
            dep.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0)
        ),
        _ => format!("{kind}:?"),
    }
}

/// Map a dep task's stored status to an observation (pure).
pub fn task_dep_obs(status: Option<&str>, dep_id: i64) -> DepObs {
    match status {
        None => DepObs::Broken(format!("task {dep_id} not found")),
        Some("done") => DepObs::Cleared("dep task done".to_string()),
        Some("cancelled") => DepObs::Cleared(
            "dep task cancelled; dependent released for re-evaluation".to_string(),
        ),
        Some("failed") => DepObs::Cleared(
            "dep task failed; dependent released for re-evaluation".to_string(),
        ),
        Some(s) => DepObs::Open(format!("dep task status {s}")),
    }
}

/// Apply one observation to one dep record (pure). Returns the
/// transition to audit ("cleared" / "broken"), or None when
/// nothing transitioned — still open, or the check failed.
/// No-change rechecks write no events, so the audit log carries
/// transitions, not polls.
pub fn apply_dep_obs(dep: &mut Value, obs: &DepObs, now: &str) -> Option<&'static str> {
    let obj = dep.as_object_mut()?;
    if obj.get("state").and_then(|v| v.as_str()).unwrap_or("open") != "open" {
        return None;
    }
    obj.insert("last_checked_at".to_string(), json!(now));
    match obs {
        DepObs::Open(ev) | DepObs::Failed(ev) => {
            obj.insert("last_observed".to_string(), json!(ev));
            None
        }
        DepObs::Cleared(ev) => {
            obj.insert("state".to_string(), json!("cleared"));
            obj.insert("cleared_at".to_string(), json!(now));
            obj.insert("last_observed".to_string(), json!(ev));
            Some("cleared")
        }
        DepObs::Broken(ev) => {
            obj.insert("state".to_string(), json!("broken"));
            obj.insert("cleared_at".to_string(), json!(now));
            obj.insert("last_observed".to_string(), json!(ev));
            Some("broken")
        }
    }
}

/// A dep-block holds until EVERY dep has resolved (cleared or
/// broken); one open dep is enough to keep the task blocked.
pub fn deps_all_resolved(deps: &[Value]) -> bool {
    !deps.is_empty()
        && deps.iter().all(|d| {
            matches!(
                d.get("state").and_then(|v| v.as_str()),
                Some("cleared") | Some("broken")
            )
        })
}

/// The dep records of a task row, tolerant of the stored shape
/// (parsed JSON array on the read surface, raw TEXT, or absent).
pub fn deps_of(row: &Value) -> Vec<Value> {
    match row.get("blocked_deps") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// True when the row carries a live dep-block record.
pub fn dep_block_active(row: &Value) -> bool {
    !deps_of(row).is_empty()
}

fn valid_dep_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty()
    ) && !repo.contains(char::is_whitespace)
}

/// Validate and normalize the deps array of a block request
/// (#8276). Ok(None) when the request carries no deps — the
/// classic free-text block, unchanged. A dep the broker cannot
/// watch is a usage error at block time, never a silent prose
/// block: refs are strict (owner/repo#N, a positive task id, a
/// positive run id).
pub fn parse_block_deps(
    req: &Value,
    worker: &str,
    now: &str,
) -> Result<Option<Vec<Value>>, String> {
    let Some(raw) = req.get("deps") else {
        return Ok(None);
    };
    let Some(arr) = raw.as_array() else {
        return Err("block: deps must be an array".to_string());
    };
    if arr.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for d in arr {
        let kind = d.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let reason = d
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut dep = json!({
            "kind": kind, "state": "open", "reason": reason,
            "recorded_by": worker, "recorded_at": now,
            "last_checked_at": Value::Null, "last_observed": Value::Null,
        });
        let obj = dep.as_object_mut().expect("dep object");
        match kind {
            "pr" | "issue" | "ci" => {
                let repo = d.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                if !valid_dep_repo(repo) {
                    return Err(format!(
                        "block: dep repo must be owner/name, got '{repo}'"
                    ));
                }
                obj.insert("repo".to_string(), json!(repo));
                if kind == "ci" {
                    let run_id = d.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
                    if run_id <= 0 {
                        return Err("block: ci dep needs a positive run_id".to_string());
                    }
                    obj.insert("run_id".to_string(), json!(run_id));
                } else {
                    let number = d.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                    if number <= 0 {
                        return Err(format!(
                            "block: {kind} dep needs a positive number"
                        ));
                    }
                    obj.insert("number".to_string(), json!(number));
                }
            }
            "task" => {
                let task_id = d.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if task_id <= 0 {
                    return Err("block: task dep needs a positive task_id".to_string());
                }
                obj.insert("task_id".to_string(), json!(task_id));
            }
            _ => {
                return Err(format!(
                    "block: unknown dep kind '{kind}' (pr|issue|task|ci)"
                ))
            }
        }
        out.push(dep);
    }
    Ok(Some(out))
}

pub struct TursoBackend {
    url: String,
    token: String,
    agent: ureq::Agent,
    /// #8276: the GitHub dep probe, installed by the broker shell
    /// at startup (queue_broker.rs owns the secrets Resolver).
    /// None in tests and embedders: GitHub dep checks then report
    /// Failed, and only a block's max TTL can release it.
    gh_probe: Option<std::sync::Arc<dyn DepProbe>>,
}

type TErr = String;

impl TursoBackend {
    pub fn new(url: String, token: String, agent: ureq::Agent) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token,
            agent,
            gh_probe: None,
        }
    }

    /// #8276: install the GitHub dep probe used by the dep
    /// rechecker for pr/issue/ci deps.
    pub fn set_gh_probe(&mut self, probe: std::sync::Arc<dyn DepProbe>) {
        self.gh_probe = Some(probe);
    }

    fn gh_probe(&self) -> Option<&dyn DepProbe> {
        self.gh_probe.as_deref()
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn pipeline(&self, stmts: &[(&str, Vec<Arg>)]) -> Result<Vec<StmtResult>, TErr> {
        let mut requests: Vec<Value> = stmts
            .iter()
            .map(|(sql, args)| {
                json!({
                    "type": "execute",
                    "stmt": {
                        "sql": sql,
                        "args": args.iter().map(|a| a.to_json()).collect::<Vec<_>>(),
                    }
                })
            })
            .collect();
        requests.push(json!({"type": "close"}));
        let body = json!({"requests": requests});
        let auth = format!("Bearer {}", self.token);
        let mut resp = self
            .agent
            .post(&format!("{}/v2/pipeline", self.url))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .send(serde_json::to_string(&body).unwrap_or_default().as_str())
            .map_err(|e| format!("turso queue store unreachable: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("cannot read turso response: {e}"))?;
        if !(200..300).contains(&status) {
            let snippet: String = text.chars().take(300).collect();
            return Err(format!("turso queue store HTTP {status}: {snippet}"));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| format!("turso answered a non-JSON body: {e}"))?;
        if let Some(err) = parsed.get("error") {
            let msg = match err {
                Value::String(s) => s.clone(),
                other => other
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| other.to_string()),
            };
            return Err(format!("turso pipeline error: {msg}"));
        }
        let results = parsed
            .get("results")
            .and_then(|r| r.as_array())
            .ok_or("turso pipeline response has no results")?;
        let mut out = Vec::new();
        for (i, r) in results.iter().enumerate() {
            if r.get("response")
                .and_then(|x| x.get("type"))
                .and_then(|t| t.as_str())
                == Some("close")
            {
                continue;
            }
            if r.get("type").and_then(|t| t.as_str()) != Some("ok") {
                let msg = r
                    .get("error")
                    .and_then(|e| match e {
                        Value::String(s) => Some(s.clone()),
                        other => other
                            .get("message")
                            .and_then(|m| m.as_str())
                            .map(str::to_string),
                    })
                    .unwrap_or_else(|| "unknown turso error".to_string());
                return Err(format!("turso statement {} failed: {msg}", i + 1));
            }
            let result = r
                .get("response")
                .and_then(|x| x.get("result"))
                .ok_or("turso pipeline response has no result")?;
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
            out.push(StmtResult {
                cols,
                rows,
                affected: result
                    .get("affected_row_count")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0),
                last_insert_rowid: result
                    .get("last_insert_rowid")
                    .and_then(|n| n.as_str())
                    .and_then(|s| s.parse::<i64>().ok()),
            });
        }
        Ok(out)
    }

    fn exec(&self, sql: &str, args: &[Arg]) -> Result<StmtResult, TErr> {
        Ok(self
            .pipeline(&[(sql, args.to_vec())])?
            .into_iter()
            .next()
            .unwrap())
    }

    /// Rows of one SELECT as JSON objects, with the given TEXT
    /// columns parsed back into JSON values.
    fn query_objs(&self, sql: &str, args: &[Arg], json_cols: &[&str]) -> Result<Vec<Value>, TErr> {
        let r = self.exec(sql, args)?;
        Ok(r.rows
            .iter()
            .map(|row| {
                let mut obj = Map::new();
                for (i, col) in r.cols.iter().enumerate() {
                    let mut v = row.get(i).cloned().unwrap_or(Value::Null);
                    if json_cols.contains(&col.as_str()) {
                        if let Value::String(s) = &v {
                            v = serde_json::from_str(s).unwrap_or(Value::Null);
                        }
                    }
                    obj.insert(col.clone(), v);
                }
                Value::Object(obj)
            })
            .collect())
    }

    pub fn ensure_schema(&self) -> Result<(), TErr> {
        let stmts: Vec<(&str, Vec<Arg>)> = SCHEMA.iter().map(|s| (*s, Vec::new())).collect();
        self.pipeline(&stmts)?;
        // Additive task columns land here for databases created
        // before them: #8097 Rule 2's tasks.platform (the optional
        // coarse platform selector — existing rows read NULL: unset,
        // either family may claim them) and #8176's claim-model
        // stamp (claim_provider / claim_model / claim_variant —
        // existing rows read NULL: the claim carried no model).
        // No backfill, nothing destructive.
        let cols = self.query_objs("SELECT name FROM pragma_table_info('tasks')", &[], &[])?;
        for col in ["platform", "claim_provider", "claim_model", "claim_variant"] {
            let has = cols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col));
            if !has {
                self.exec(&format!("ALTER TABLE tasks ADD COLUMN {col} TEXT"), &[])?;
            }
        }
        // #8276: the dependency-block columns (blocked_deps JSON,
        // dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds)
        // land the same additive way for task tables created before
        // them. No backfill: existing rows read NULL, which the
        // rechecker reads as "not a dep-block" and never touches.
        for (col, ty) in [
            ("blocked_deps", "TEXT"),
            ("dep_next_recheck_at", "TEXT"),
            ("dep_blocked_until", "TEXT"),
            ("dep_recheck_seconds", "INTEGER"),
        ] {
            let has = cols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col));
            if !has {
                self.exec(
                    &format!("ALTER TABLE tasks ADD COLUMN {col} {ty}"),
                    &[],
                )?;
            }
        }
        // #8187: the ledger's opencode_reason column (the recorded
        // specific reason an opencode assignment exists at all) lands
        // the same additive way for ledgers created before it.
        let lcols = self.query_objs(
            "SELECT name FROM pragma_table_info('repo_task_lane_ledger')",
            &[],
            &[],
        )?;
        if !lcols
            .iter()
            .any(|c| c.get("name").and_then(|v| v.as_str()) == Some("opencode_reason"))
        {
            self.exec(
                "ALTER TABLE repo_task_lane_ledger ADD COLUMN opencode_reason TEXT",
                &[],
            )?;
        }
        // #8210: the ledger's platform-limitation columns (a
        // task may be limited to one platform family, but only
        // with a good reason recorded beside it) land the same
        // additive way.
        for col in ["platform_limit", "platform_limit_reason"] {
            if !lcols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col))
            {
                self.exec(
                    &format!(
                        "ALTER TABLE repo_task_lane_ledger ADD COLUMN {col} TEXT"
                    ),
                    &[],
                )?;
            }
        }
        Ok(())
    }
}

// Arg is cloned into pipeline bodies; derive Clone via manual impl
// (the enum is small and the values are request-scoped).
impl Clone for Arg {
    fn clone(&self) -> Self {
        match self {
            Arg::Text(s) => Arg::Text(s.clone()),
            Arg::Int(n) => Arg::Int(*n),
            Arg::Null => Arg::Null,
        }
    }
}

const SCHEMA: [&str; 16] = [
    "CREATE TABLE IF NOT EXISTS tasks (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, \
        queue TEXT NOT NULL, title TEXT, body TEXT, \
        payload TEXT NOT NULL DEFAULT '{}', dedupe_key TEXT, \
        priority INTEGER, status TEXT NOT NULL DEFAULT 'queued', \
        not_before TEXT, worker_id TEXT, claimed_at TEXT, lease_until TEXT, \
        lane_id TEXT, platform TEXT, attempts INTEGER NOT NULL DEFAULT 0, \
        max_attempts INTEGER NOT NULL DEFAULT 4, \
        claim_provider TEXT, claim_model TEXT, claim_variant TEXT, \
        progress TEXT NOT NULL DEFAULT '{}', result TEXT NOT NULL DEFAULT '{}', \
        last_error TEXT, park_kind TEXT, wait_kind TEXT, ready_condition TEXT, \
        last_checked_at TEXT, check_outcome TEXT, \
        created_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
        completed_at TEXT, \
        blocked_deps TEXT, dep_next_recheck_at TEXT, dep_blocked_until TEXT, \
        dep_recheck_seconds INTEGER)",
    // NOT unique: the PG data itself carries duplicate (queue, dedupe_key)
    // pairs from before dedupe was enforced, and the backup load must
    // accept them as-is; enqueue dedupe is a read-then-insert check.
    "CREATE INDEX IF NOT EXISTS tasks_queue_dedupe \
        ON tasks(queue, dedupe_key) WHERE dedupe_key IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS tasks_claim_scan \
        ON tasks(queue, status, priority, created_at)",
    "CREATE TABLE IF NOT EXISTS leases (\
        task_id INTEGER PRIMARY KEY, worker_id TEXT NOT NULL, \
        lease_until TEXT NOT NULL, heartbeat_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS lanes (\
        lane_id TEXT PRIMARY KEY, worker_id TEXT NOT NULL, \
        queue TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1)",
    "CREATE TABLE IF NOT EXISTS workers (\
        worker_id TEXT PRIMARY KEY, role TEXT NOT NULL DEFAULT '', \
        display_name TEXT, status TEXT NOT NULL DEFAULT 'idle', \
        last_heartbeat_at TEXT, created_at TEXT)",
    "CREATE TABLE IF NOT EXISTS checkpoints (\
        worker_id TEXT NOT NULL, key TEXT NOT NULL, \
        value TEXT NOT NULL DEFAULT '{}', updated_at TEXT NOT NULL, \
        PRIMARY KEY (worker_id, key))",
    "CREATE TABLE IF NOT EXISTS repo_task_lane_ledger (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, queue TEXT NOT NULL, \
        task_id INTEGER NOT NULL UNIQUE, assigned_lane TEXT, executed_lane TEXT, \
        exclusion_reason TEXT, opencode_reason TEXT, platform_limit TEXT, \
        platform_limit_reason TEXT, assigned_by TEXT, session_id TEXT, \
        fallback_reason TEXT, assigned_at TEXT, executed_at TEXT, \
        updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS events (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER, \
        kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}', \
        created_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS events_task ON events(task_id, id)",
    "CREATE INDEX IF NOT EXISTS events_kind_ts ON events(kind, created_at)",
    "CREATE INDEX IF NOT EXISTS events_created ON events(created_at)",
    "CREATE TABLE IF NOT EXISTS creation_outbox (\
        id TEXT PRIMARY KEY, repo TEXT NOT NULL, kind TEXT NOT NULL, \
        payload TEXT NOT NULL DEFAULT '{}', dedup_key TEXT NOT NULL, \
        payload_hash TEXT NOT NULL, producer TEXT, \
        state TEXT NOT NULL DEFAULT 'pending', \
        created_at TEXT NOT NULL, available_at TEXT NOT NULL, \
        attempt_count INTEGER NOT NULL DEFAULT 0, \
        max_attempts INTEGER NOT NULL DEFAULT 8, \
        last_error_class TEXT, lease_owner TEXT, lease_until TEXT, \
        github_number INTEGER, github_url TEXT, completed_at TEXT)",
    "CREATE UNIQUE INDEX IF NOT EXISTS creation_outbox_dedupe \
        ON creation_outbox(repo, dedup_key)",
    "CREATE INDEX IF NOT EXISTS creation_outbox_due \
        ON creation_outbox(state, available_at, created_at)",
    "CREATE TABLE IF NOT EXISTS creation_outbox_circuit (\
        id INTEGER PRIMARY KEY CHECK (id = 1), \
        consecutive_rate_limits INTEGER NOT NULL DEFAULT 0, \
        open_until TEXT, last_error_class TEXT, updated_at TEXT NOT NULL)",
];

// ---------------------------------------------------------------------------
// Envelope (identical shapes to the Postgres broker dispatch)
// ---------------------------------------------------------------------------

fn ok(fields: Value) -> Value {
    let mut m = json!({"ok": true});
    if let (Some(dst), Some(src)) = (m.as_object_mut(), fields.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    m
}

fn fail(exit: i32, msg: impl Into<String>) -> Value {
    json!({"ok": false, "exit": exit, "error": msg.into()})
}

fn terr(e: TErr) -> Value {
    // Transport/store failures mirror the Postgres broker's exit 4.
    fail(4, e)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn merge_obj(base: &Value, patch: &Value) -> Value {
    let mut out = match base {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    if let Value::Object(p) = patch {
        for (k, v) in p {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn get_task(t: &TursoBackend, id: i64) -> Result<Option<Value>, TErr> {
    let rows = t.query_objs(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
        &[Arg::Int(id)],
        &TASK_JSON_COLS,
    )?;
    Ok(rows.into_iter().next())
}

fn get_ledger(t: &TursoBackend, task_id: i64) -> Result<Option<Value>, TErr> {
    let rows = t.query_objs(
        &format!("SELECT {LEDGER_COLS} FROM repo_task_lane_ledger WHERE task_id = ?"),
        &[Arg::Int(task_id)],
        &[],
    )?;
    Ok(rows.into_iter().next())
}

/// The audit marks every broker event carries (#8187 amendments):
/// who is looking at the item — its platform mark, its claim lane,
/// its pool (the queue), the claiming session's model stamp
/// (#8176 claim_provider / claim_model / claim_variant), the bound
/// session id when one exists (progress.session_id), and the task
/// id itself. Marks are RECORDED, never required and never a
/// filter: any of them may be null on any given transition. The
/// events table is append-only (this module exposes no update or
/// delete path for it), so these marks are the durable audit log.
fn audit_marks(task: &Value) -> Value {
    let progress = task.get("progress").cloned().unwrap_or(json!({}));
    json!({
        "task_id": task.get("id").cloned().unwrap_or(Value::Null),
        "queue": task.get("queue").cloned().unwrap_or(Value::Null),
        "pool": task.get("queue").cloned().unwrap_or(Value::Null),
        "platform": task.get("platform").cloned().unwrap_or(Value::Null),
        "lane_id": task.get("lane_id").cloned().unwrap_or(Value::Null),
        "model_provider": task.get("claim_provider").cloned().unwrap_or(Value::Null),
        "model_id": task.get("claim_model").cloned().unwrap_or(Value::Null),
        "model_variant": task.get("claim_variant").cloned().unwrap_or(Value::Null),
        "session_id": progress.get("session_id").cloned().unwrap_or(Value::Null),
    })
}

/// An event detail with the audit marks filled in: the detail's
/// own explicit fields win, the marks supply the rest.
fn with_marks(detail: &Value, task: &Value) -> Value {
    merge_obj(&audit_marks(task), detail)
}

fn event(
    t: &TursoBackend,
    worker: Option<&str>,
    task_id: Option<i64>,
    kind: &str,
    detail: &Value,
) -> Result<(), TErr> {
    t.exec(
        "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
         VALUES (?, ?, ?, ?, ?)",
        &[
            arg_str(worker),
            task_id.map(Arg::Int).unwrap_or(Arg::Null),
            Arg::Text(kind.to_string()),
            Arg::Text(detail.to_string()),
            Arg::Text(now_iso()),
        ],
    )?;
    Ok(())
}

/// Parse an /events-append request: kind is required and must be a
/// non-empty string; worker_id/task_id optional (the PG stream's
/// non-queue writers always wrote task_id NULL); detail defaults {}.
fn parse_event_append(req: &Value) -> Result<(Option<String>, Option<i64>, String, Value), String> {
    let kind = req_str(req, "kind")
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| "kind is required".to_string())?
        .to_string();
    let worker = req_str(req, "worker_id").map(str::to_string);
    let task_id = req_i64(req, "task_id");
    let detail = req.get("detail").cloned().unwrap_or_else(|| json!({}));
    Ok((worker, task_id, kind, detail))
}

/// Retention cutoff for /events-prune: rows strictly older than
/// keep_days (clamped to 1..=3650, mirroring prune_events(30)'s
/// fixed 30-day default at the call site) are deleted.
fn prune_cutoff(now: i64, keep_days: i64) -> String {
    let days = keep_days.clamp(1, 3650);
    iso(now - days * 86_400)
}

fn verb_events_append(t: &TursoBackend, req: &Value) -> Value {
    let (worker, task_id, kind, detail) = match parse_event_append(req) {
        Ok(v) => v,
        Err(e) => return fail(2, e),
    };
    let work = (|| -> Result<Value, TErr> {
        let now = now_iso();
        let res = t.exec(
            "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
             VALUES (?, ?, ?, ?, ?)",
            &[
                arg_str(worker.as_deref()),
                task_id.map(Arg::Int).unwrap_or(Arg::Null),
                Arg::Text(kind.clone()),
                Arg::Text(detail.to_string()),
                Arg::Text(now),
            ],
        )?;
        Ok(json!({"id": res.last_insert_rowid.unwrap_or(0), "kind": kind}))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// The event_seen primitive: has an event of `kind` been recorded
/// for `task_id`? Replaces the PG revalidate_parked_tasks() EXISTS
/// subquery, which cannot reach across databases; a broker-side
/// revalidator sweeper calls this per parked task (cutover doc).
fn verb_events_seen(t: &TursoBackend, req: &Value) -> Value {
    let (Some(task_id), Some(kind)) = (req_i64(req, "task_id"), req_str(req, "kind")) else {
        return fail(2, "task_id and kind are required".to_string());
    };
    let kind = kind.to_string();
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(
            "SELECT COUNT(*) AS n, MAX(created_at) AS latest FROM events \
             WHERE task_id = ? AND kind = ?",
            &[Arg::Int(task_id), Arg::Text(kind.clone())],
            &[],
        )?;
        let first = rows.into_iter().next().unwrap_or(json!({}));
        let n = first.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
        Ok(json!({
            "task_id": task_id,
            "kind": kind,
            "seen": n > 0,
            "count": n,
            "latest_created_at": first
                .get("latest")
                .cloned()
                .unwrap_or(Value::Null),
        }))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// Observability shape parity with the ad-hoc PG queries
/// (count by kind, optionally since an ISO timestamp).
fn verb_events_count(t: &TursoBackend, req: &Value) -> Value {
    let mut sql = "SELECT COUNT(*) AS n FROM events WHERE 1 = 1".to_string();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(k) = req_str(req, "kind") {
        sql += " AND kind = ?";
        args.push(Arg::Text(k.to_string()));
    }
    if let Some(since) = req_str(req, "since") {
        sql += " AND created_at > ?";
        args.push(Arg::Text(since.to_string()));
    }
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(&sql, &args, &[])?;
        let n = rows
            .into_iter()
            .next()
            .and_then(|r| r.get("n").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        Ok(json!({"count": n}))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// prune_events, shared by the /events-prune verb and /sweep:
/// delete events older than keep_days and self-log an events_pruned
/// row, like the PG function. Returns (deleted, cutoff).
fn prune_events(t: &TursoBackend, keep_days: i64) -> Result<(i64, String), TErr> {
    let cutoff = prune_cutoff(now_unix(), keep_days);
    let deleted = t
        .exec(
            "DELETE FROM events WHERE created_at < ?",
            &[Arg::Text(cutoff.clone())],
        )?
        .affected as i64;
    event(
        t,
        Some("prune_events"),
        None,
        "events_pruned",
        &json!({"deleted": deleted, "cutoff": cutoff, "keep_days": keep_days.clamp(1, 3650)}),
    )?;
    Ok((deleted, cutoff))
}

/// prune_events port: delete events older than keep_days (default
/// 30, the PG cron's argument) and self-log an events_pruned row,
/// like the PG function. PG ran this daily at 04:17 via pg_cron;
/// post-cutover a timer calls this verb (cutover doc).
fn verb_events_prune(t: &TursoBackend, req: &Value) -> Value {
    let keep_days = req_i64(req, "keep_days").unwrap_or(30);
    let work = (|| -> Result<Value, TErr> {
        let (deleted, cutoff) = prune_events(t, keep_days)?;
        Ok(json!({"deleted": deleted, "cutoff": cutoff}))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// The revalidate_parked_tasks outcome for one parked task, as a
/// pure function of what the sweeper could observe (migration 0023
/// §revalidate): the DB-verifiable kinds release, time parks are
/// only ever confirmed (elapsed time is never evidence — they
/// release through claim at not_before), and anything the database
/// cannot verify goes stale so the park surfaces for an agent with
/// external access instead of sitting silently.
fn revalidate_outcome(
    kind: Option<&str>,
    target_status: Option<&str>,
    want_status: Option<&str>,
    event_seen: bool,
) -> &'static str {
    match kind {
        Some("time") => "confirmed",
        Some("task_terminal") => match target_status {
            None => "stale",
            Some("done") | Some("cancelled") => "released",
            Some(_) => "confirmed",
        },
        Some("task_status") => match target_status {
            None => "stale",
            Some(s) if Some(s) == want_status => "released",
            Some(_) => "confirmed",
        },
        Some("event_seen") => {
            if event_seen {
                "released"
            } else {
                "confirmed"
            }
        }
        _ => "stale",
    }
}

/// revalidate_parked_tasks port (migration 0023, as fixed by 0024),
/// replacing the retired pg_cron job park-revalidation-daily
/// (#6803). Checks parked tasks whose last check (or last update)
/// is older than min_age_seconds, oldest first, up to limit, and
/// stamps last_checked_at / check_outcome on each. Satisfied
/// DB-verifiable conditions release the task to queued with the
/// binding cleared and a park_released event; every check leaves a
/// park_revalidated event, worker 'park-revalidation-sweeper'.
///
/// observe_only (the C3 landing mode, agent-events-turso.md):
/// outcomes are computed and logged (releases as would_release)
/// but no task row is mutated, so the first enabled run re-checks
/// the same population.
fn revalidate_parked(
    t: &TursoBackend,
    min_age_seconds: i64,
    limit: i64,
    observe_only: bool,
) -> Result<Value, TErr> {
    let cutoff = iso(now_unix() - min_age_seconds.max(0));
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks WHERE status = 'parked' \
             AND COALESCE(last_checked_at, updated_at) < ? \
             ORDER BY COALESCE(last_checked_at, updated_at) ASC LIMIT ?"
        ),
        &[Arg::Text(cutoff), Arg::Int(limit.clamp(1, 5000))],
        &TASK_JSON_COLS,
    )?;
    let mut results: Vec<Value> = Vec::new();
    let (mut released, mut confirmed, mut stale, mut would_release) = (0i64, 0i64, 0i64, 0i64);
    for row in &rows {
        let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let condition = row.get("ready_condition").cloned().unwrap_or(Value::Null);
        let kind = condition.get("kind").and_then(|v| v.as_str());
        let mut target_status: Option<String> = None;
        let mut event_seen = false;
        match kind {
            Some("task_terminal") | Some("task_status") => {
                if let Some(tid) = condition.get("task_id").and_then(|v| v.as_i64()) {
                    let trow = t.query_objs(
                        "SELECT status FROM tasks WHERE id = ?",
                        &[Arg::Int(tid)],
                        &[],
                    )?;
                    target_status = trow
                        .into_iter()
                        .next()
                        .and_then(|r| r.get("status").and_then(|v| v.as_str()).map(str::to_string));
                }
            }
            Some("event_seen") => {
                let tid = condition
                    .get("task_id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(id);
                if let Some(ek) = condition.get("event_kind").and_then(|v| v.as_str()) {
                    let nrow = t.query_objs(
                        "SELECT COUNT(*) AS n FROM events WHERE task_id = ? AND kind = ?",
                        &[Arg::Int(tid), Arg::Text(ek.to_string())],
                        &[],
                    )?;
                    let n = nrow
                        .into_iter()
                        .next()
                        .and_then(|r| r.get("n").and_then(|v| v.as_i64()))
                        .unwrap_or(0);
                    event_seen = n > 0;
                }
            }
            _ => {}
        }
        let outcome = revalidate_outcome(
            kind,
            target_status.as_deref(),
            condition.get("status").and_then(|v| v.as_str()),
            event_seen,
        );
        if observe_only {
            let logged = if outcome == "released" {
                would_release += 1;
                "would_release"
            } else {
                match outcome {
                    "confirmed" => confirmed += 1,
                    _ => stale += 1,
                }
                outcome
            };
            event(
                t,
                Some("park-revalidation-sweeper"),
                Some(id),
                "park_revalidated",
                &json!({"outcome": logged, "condition_kind": kind, "observe_only": true, "via": "revalidate_parked_tasks"}),
            )?;
            results.push(json!({"task_id": id, "outcome": logged}));
            continue;
        }
        match outcome {
            "released" => {
                released += 1;
                let progress = row.get("progress").cloned().unwrap_or(json!({}));
                let (progress, cleared) = clear_binding(&progress, "revalidate_parked_tasks");
                let mut stamp = json!({
                    "outcome": "released", "at": now_iso(), "condition": condition,
                });
                if let Some(note) = condition.get("note") {
                    stamp["note"] = note.clone();
                }
                let progress = merge_obj(&progress, &json!({"park_revalidation": stamp}));
                t.exec(
                    "UPDATE tasks SET status = 'queued', not_before = NULL, \
                       last_checked_at = ?, check_outcome = 'released', \
                       progress = ?, updated_at = ? \
                     WHERE id = ? AND status = 'parked'",
                    &[
                        Arg::Text(now_iso()),
                        Arg::Text(progress.to_string()),
                        Arg::Text(now_iso()),
                        Arg::Int(id),
                    ],
                )?;
                if let Some(old) = cleared {
                    on_binding_cleared(
                        t,
                        "park-revalidation-sweeper",
                        id,
                        &old,
                        "revalidate_parked_tasks",
                    )?;
                }
                event(
                    t,
                    Some("park-revalidation-sweeper"),
                    Some(id),
                    "park_released",
                    &json!({"condition": condition, "via": "revalidate_parked_tasks"}),
                )?;
            }
            _ => {
                if outcome == "confirmed" {
                    confirmed += 1;
                } else {
                    stale += 1;
                }
                t.exec(
                    "UPDATE tasks SET last_checked_at = ?, check_outcome = ?, \
                       updated_at = ? WHERE id = ? AND status = 'parked'",
                    &[
                        Arg::Text(now_iso()),
                        Arg::Text(outcome.to_string()),
                        Arg::Text(now_iso()),
                        Arg::Int(id),
                    ],
                )?;
                event(
                    t,
                    Some("park-revalidation-sweeper"),
                    Some(id),
                    "park_revalidated",
                    &json!({"outcome": outcome, "condition_kind": kind, "via": "revalidate_parked_tasks"}),
                )?;
            }
        }
        results.push(json!({"task_id": id, "outcome": outcome}));
    }
    Ok(json!({
        "checked": rows.len(), "released": released, "confirmed": confirmed,
        "stale": stale, "would_release": would_release,
        "observe_only": observe_only, "results": results,
    }))
}

/// One sweeper cycle (#6803): the Turso replacement for the retired
/// pg_cron trio — expire_leases (was every minute), prune_events
/// (daily 04:17) and revalidate_parked_tasks (daily 05:17) — driven
/// by systemd timers POSTing here. Parts are opt-in per call except
/// expire, which defaults on: the minutely timer sends
/// {"expire": true} alone, the daily timer adds prune + revalidate.
/// observe_only defaults true (C3): releases must be opted into by
/// passing observe_only=false. A summary event is appended only
/// when the cycle changed something or ran the daily parts, so the
/// minutely expire stays silent while it has nothing to expire.
fn verb_sweep(t: &TursoBackend, req: &Value) -> Value {
    let do_expire = req_bool(req, "expire").unwrap_or(true);
    let do_revalidate = req_bool(req, "revalidate").unwrap_or(false);
    let do_prune = req_bool(req, "prune").unwrap_or(false);
    let observe_only = req_bool(req, "observe_only").unwrap_or(true);
    let work = (|| -> Result<Value, TErr> {
        let mut out = json!({"observe_only": observe_only});
        let mut expired = 0i64;
        if do_expire {
            expired = expire_leases(t)?;
            out["expired_leases"] = json!(expired);
        }
        if do_revalidate {
            out["revalidation"] = revalidate_parked(
                t,
                req_i64(req, "min_age_seconds").unwrap_or(86_400),
                req_i64(req, "limit").unwrap_or(500),
                observe_only,
            )?;
        }
        if do_prune {
            let (deleted, cutoff) = prune_events(t, req_i64(req, "keep_days").unwrap_or(30))?;
            out["prune"] = json!({"deleted": deleted, "cutoff": cutoff});
        }
        // #8276: the dep rechecker rides the sweep — the
        // fleet-queue-expire timer already POSTs /sweep every
        // minute, so dep-blocked tasks are re-examined on their
        // own dep_next_recheck_at schedule with no new service
        // and no lane awake. Opt out per call: dep_recheck=false.
        let dep_summary = if req_bool(req, "dep_recheck").unwrap_or(true) {
            Some(run_dep_rechecks(t, req_i64(req, "dep_limit").unwrap_or(200))?)
        } else {
            None
        };
        let dep_checked = dep_summary
            .as_ref()
            .and_then(|s| s.get("checked"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if let Some(s) = dep_summary {
            out["dep_recheck"] = s;
        }
        if expired > 0 || do_revalidate || do_prune || dep_checked > 0 {
            event(t, Some("fleet-sweeper"), None, "sweep", &out)?;
        }
        Ok(out)
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// `/coverage` — read-only reconciliation of an issue set against queue
/// coverage (#7016).
///
/// The caller supplies the open-issue set it just enumerated from GitHub
/// (`issues`: `[{"number": 7016, "title": "…"}]`); this verb answers, for
/// each one, whether its repo-health dedupe key is covered by a LIVE task
/// in the named queue. A done/cancelled/failed task is NOT coverage
/// (Drew's doctrine, 2026-11-07: a done task means triage or a slice, not
/// a resolved issue), and neither is a task in another queue.
///
/// Read-only by construction: it never writes tasks or events, so it is
/// safe to run on a schedule and safe to run in observe mode. The caller
/// decides what to enqueue; the dedupe key makes that idempotent.
fn verb_coverage(t: &TursoBackend, req: &Value) -> Value {
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    if queue.is_empty() {
        return fail(2, "coverage: queue is required".to_string());
    }
    let repo = req_str(req, "repo").unwrap_or("").to_string();
    if repo.is_empty() {
        return fail(2, "coverage: repo is required".to_string());
    }
    let issues = match req.get("issues").and_then(|v| v.as_array()) {
        Some(a) => a.clone(),
        None => return fail(2, "coverage: issues must be an array".to_string()),
    };
    let work = (|| -> Result<Value, TErr> {
        // One query for every live task in the queue that carries an
        // issue dedupe key; the match is done in Rust so both the
        // `triage` and `fix` classes count as coverage for the issue.
        let rows = t.query_objs(
            "SELECT id, status, dedupe_key FROM tasks \
             WHERE queue = ? AND dedupe_key IS NOT NULL \
               AND status IN ('queued','claimed','parked','handed_off','blocked')",
            &[Arg::Text(queue.clone())],
            &[],
        )?;
        let mut live: std::collections::HashMap<i64, (i64, String)> =
            std::collections::HashMap::new();
        for r in &rows {
            let id = r.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let status = r
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let Some(key) = r.get("dedupe_key").and_then(|v| v.as_str()) else {
                continue;
            };
            if let Some(n) = issue_number_for_repo(key, &repo) {
                live.entry(n).or_insert((id, status));
            }
        }
        let mut uncovered: Vec<Value> = Vec::new();
        let mut covered: Vec<Value> = Vec::new();
        for iss in &issues {
            let Some(n) = iss.get("number").and_then(|v| v.as_i64()) else {
                continue;
            };
            match live.get(&n) {
                Some((id, status)) => covered.push(json!({
                    "number": n,
                    "task": id,
                    "status": status,
                    "title": iss.get("title").cloned().unwrap_or(Value::Null),
                })),
                None => uncovered.push(json!({
                    "number": n,
                    "title": iss.get("title").cloned().unwrap_or(Value::Null),
                    "url": format!("https://github.com/{repo}/issues/{n}"),
                })),
            }
        }
        Ok(json!({
            "queue": queue,
            "repo": repo,
            "open_issues": issues.len(),
            "covered": covered.len(),
            "uncovered": uncovered.len(),
            "uncovered_issues": uncovered,
            "covered_issues": covered,
        }))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

/// Extract the issue number from a repo-health dedupe key for `repo`.
///
/// Accepted shapes (all seen in the store): the canonical
/// `repo-health:v1:(triage|fix):<repo>:issue:<n>` plus dispatch-suffixed
/// variants (`…:issue:<n>:dispatch:<x>`) and the legacy repo-name aliases
/// the reconciliation run of 2026-11-07 had to match by hand.
fn issue_number_from_dedupe(key: &str, repo: &str) -> Option<i64> {
    let marker = format!(":{repo}:issue:");
    let start = key.find(&marker)? + marker.len();
    let rest = &key[start..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    // Reject `:issue:12abc` style tails only when the tail is not a
    // known suffix separator.
    let tail = &rest[digits.len()..];
    if !(tail.is_empty() || tail.starts_with(':')) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// The other name a repository's issue dedupe keys may carry.
///
/// Two repos moved to the developing-today-forks org on 2026-11-04 and
/// `dezren39/nix` was renamed to `dezren39/darwin-nix`; `validate.rs`
/// keeps both names allowlisted because in-flight tasks still carry the
/// old ones. A live task under either name is coverage for the issue, so
/// matching only the current name would report covered issues as
/// uncovered and import duplicate triage work.
fn legacy_alias(repo: &str) -> Option<&'static str> {
    match repo {
        "developing-today-forks/opencode" => Some("developing-today/opencode"),
        "developing-today/opencode" => Some("developing-today-forks/opencode"),
        "developing-today-forks/openchamber" => Some("developing-today/openchamber"),
        "developing-today/openchamber" => Some("developing-today-forks/openchamber"),
        "dezren39/darwin-nix" => Some("dezren39/nix"),
        "dezren39/nix" => Some("dezren39/darwin-nix"),
        _ => None,
    }
}

/// Issue number from a key under `repo` or any legacy alias of it.
fn issue_number_for_repo(key: &str, repo: &str) -> Option<i64> {
    issue_number_from_dedupe(key, repo)
        .or_else(|| legacy_alias(repo).and_then(|alias| issue_number_from_dedupe(key, alias)))
}

fn touch_worker(t: &TursoBackend, worker: &str, task_id: Option<i64>) -> Result<(), TErr> {
    // PG workers is the roster table (id/role/display_name/status/
    // last_heartbeat_at): a touch upserts the heartbeat and marks
    // the worker working while it holds a task, idle when it
    // releases one. Unknown workers enter with an empty role.
    let status = if task_id.is_some() { "working" } else { "idle" };
    t.exec(
        "INSERT INTO workers (worker_id, role, status, last_heartbeat_at, created_at) \
         VALUES (?, '', ?, ?, ?) \
         ON CONFLICT(worker_id) DO UPDATE SET \
           last_heartbeat_at = excluded.last_heartbeat_at, \
           status = excluded.status",
        &[
            Arg::Text(worker.to_string()),
            Arg::Text(status.to_string()),
            Arg::Text(now_iso()),
            Arg::Text(now_iso()),
        ],
    )?;
    Ok(())
}

/// Remove a session binding from a progress object the way migration
/// 0023 does: drop session_id/launched_by, stamp session_cleared.
/// Returns (new progress, cleared session id).
fn clear_binding(progress: &Value, via: &str) -> (Value, Option<String>) {
    let old = progress
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(str::to_string);
    match old {
        None => (progress.clone(), None),
        Some(old) => {
            let mut p = match progress {
                Value::Object(m) => m.clone(),
                _ => Map::new(),
            };
            p.remove("session_id");
            p.remove("launched_by");
            p.insert(
                "session_cleared".to_string(),
                json!({"session_id": old, "cleared_at": now_iso(), "via": via}),
            );
            (Value::Object(p), Some(old))
        }
    }
}

fn on_binding_cleared(
    t: &TursoBackend,
    worker: &str,
    task_id: i64,
    old_session: &str,
    via: &str,
) -> Result<(), TErr> {
    t.exec(
        "UPDATE repo_task_lane_ledger SET session_id = NULL, updated_at = ? \
         WHERE task_id = ? AND session_id IS NOT NULL",
        &[Arg::Text(now_iso()), Arg::Int(task_id)],
    )?;
    event(
        t,
        Some(worker),
        Some(task_id),
        "session_binding_cleared",
        &json!({"session_id": old_session, "via": via}),
    )
}

/// expire_leases (migration 0023 §1): claimed rows whose lease has
/// lapsed return to queued, binding cleared, lease rows dropped.
/// Returns how many tasks were returned to queued (PG returned the
/// count; claim ignores it, the /sweep verb reports it, #6803).
fn expire_leases(t: &TursoBackend) -> Result<i64, TErr> {
    let now = now_iso();
    let mut expired = 0i64;
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks \
             WHERE status = 'claimed' AND lease_until < ?"
        ),
        &[Arg::Text(now.clone())],
        &TASK_JSON_COLS,
    )?;
    for row in &rows {
        let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let worker = row
            .get("worker_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let progress = row.get("progress").cloned().unwrap_or(json!({}));
        let to_sink = expired_claim_returns_to_sink(&progress);
        let (progress, cleared) = clear_binding(&progress, "expire_leases");
        let upd = if to_sink {
            // #8234: the sink is its own status now — the return
            // lands in 'handed_off' with NO wait fields at all. The
            // pre-#8234 shape stamped a fresh not_before = +1200s
            // and a {"kind": "time"} ready_condition here: a fake
            // timer, since the sink pick never read it.
            t.exec(
                sink_return_sql(),
                &[
                    Arg::Text(progress.to_string()),
                    Arg::Text(now.clone()),
                    Arg::Int(id),
                    Arg::Text(now.clone()),
                ],
            )?
        } else {
            t.exec(
                "UPDATE tasks SET status = 'queued', worker_id = NULL, lease_until = NULL, \
                   progress = ?, updated_at = ? \
                 WHERE id = ? AND status = 'claimed' AND lease_until < ?",
                &[
                    Arg::Text(progress.to_string()),
                    Arg::Text(now.clone()),
                    Arg::Int(id),
                    Arg::Text(now.clone()),
                ],
            )?
        };
        if upd.affected == 0 {
            continue;
        }
        expired += 1;
        event(
            t,
            Some(&worker),
            Some(id),
            "lease_expired",
            &json!({"via": "expire_leases", "cleared_session_id": cleared,
                    "return": if to_sink { "handed_off" } else { "queued" }}),
        )?;
        if let Some(old) = cleared {
            on_binding_cleared(t, &worker, id, &old, "expire_leases")?;
        }
    }
    t.exec(
        "DELETE FROM leases WHERE lease_until < ?",
        &[Arg::Text(now)],
    )?;
    Ok(expired)
}

/// record_repo_task_lane_execution, contract-shaped: stamp the
/// executed lane (+ fallback reason / session) onto the task's
/// ledger row, creating a minimal row when assignment never ran.
fn record_execution(t: &TursoBackend, req: &Value, row: &Value) -> Result<Option<Value>, TErr> {
    let Some(ex) = req.get("execution").filter(|e| e.is_object()) else {
        return Ok(None);
    };
    let task_id = req.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let queue = row
        .get("queue")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now = now_iso();
    if get_ledger(t, task_id)?.is_some() {
        t.exec(
            "UPDATE repo_task_lane_ledger SET \
               executed_lane = COALESCE(?, executed_lane), \
               fallback_reason = COALESCE(?, fallback_reason), \
               session_id = COALESCE(?, session_id), \
               executed_at = COALESCE(executed_at, ?), updated_at = ? \
             WHERE task_id = ?",
            &[
                arg_str(ex.get("lane").and_then(|v| v.as_str())),
                arg_str(ex.get("fallback_reason").and_then(|v| v.as_str())),
                arg_str(ex.get("session_id").and_then(|v| v.as_str())),
                Arg::Text(now.clone()),
                Arg::Text(now),
                Arg::Int(task_id),
            ],
        )?;
    } else {
        t.exec(
            &format!(
                "INSERT INTO repo_task_lane_ledger \
                 (queue, task_id, assigned_lane, executed_lane, fallback_reason, session_id, assigned_at, executed_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            &[
                Arg::Text(queue),
                Arg::Int(task_id),
                arg_str(ex.get("lane").and_then(|v| v.as_str())),
                arg_str(ex.get("lane").and_then(|v| v.as_str())),
                arg_str(ex.get("fallback_reason").and_then(|v| v.as_str())),
                arg_str(ex.get("session_id").and_then(|v| v.as_str())),
                Arg::Text(now.clone()),
                Arg::Text(now.clone()),
                Arg::Text(now),
            ],
        )?;
    }
    get_ledger(t, task_id)
}

fn verb_with_execution(t: &TursoBackend, req: &Value, row: Value) -> Result<Value, TErr> {
    if row.is_null() {
        return Ok(ok(json!({"row": Value::Null})));
    }
    match record_execution(t, req, &row)? {
        Some(ledger) => Ok(ok(json!({"row": row, "lane_ledger": ledger}))),
        None => Ok(ok(json!({"row": row}))),
    }
}

fn req_str<'a>(req: &'a Value, key: &str) -> Option<&'a str> {
    req.get(key).and_then(|v| v.as_str())
}

fn req_i64(req: &Value, key: &str) -> Option<i64> {
    req.get(key).and_then(|v| v.as_i64())
}

fn req_bool(req: &Value, key: &str) -> Option<bool> {
    req.get(key).and_then(|v| v.as_bool())
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

/// Statuses in which a task still owns its dedupe key: an enqueue
/// that dedupe-matches one of these is a true duplicate and returns
/// the live row. Terminal rows (done/cancelled/failed) do NOT block
/// the key (#6838): PG's enqueue revived on dedupe-over-terminal,
/// and the Turso port dropped that nuance (cutover doc deviation 6).
pub const LIVE_STATUSES: [&str; 5] = ["queued", "claimed", "parked", "handed_off", "blocked"];
pub const TERMINAL_STATUSES: [&str; 3] = ["done", "cancelled", "failed"];

/// Partition the rows sharing one (queue, dedupe_key) into the live
/// dedupe hit, if any, and the newest terminal predecessor, if any.
/// Rows arrive ordered by id ASC, so the last terminal row seen is
/// the most recent attempt — the one a revive links back to.
fn dedupe_partition(rows: &[Value]) -> (Option<Value>, Option<Value>) {
    let mut live: Option<Value> = None;
    let mut terminal: Option<Value> = None;
    for row in rows {
        let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if LIVE_STATUSES.contains(&status) {
            if live.is_none() {
                live = Some(row.clone());
            }
        } else if TERMINAL_STATUSES.contains(&status) {
            terminal = Some(row.clone());
        }
    }
    (live, terminal)
}

fn verb_enqueue(t: &TursoBackend, req: &Value) -> Value {
    let packet = req.get("packet").cloned().unwrap_or(Value::Null);
    let (errors, warnings) = validate_packet(&packet);
    if !errors.is_empty() {
        return json!({
            "ok": false, "exit": 2,
            "error": format!("packet validation failed ({} error(s))", errors.len()),
            "errors": errors, "warnings": warnings,
        });
    }
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    if queue.is_empty() {
        return fail(2, "enqueue: queue is required".to_string());
    }
    // Shadow queues are for deliberate shadow runs only (#7016): an
    // enqueue may land in one only when the caller declares shadow
    // intent, so a forgotten --queue can never silently misroute
    // real work there again.
    if queue.ends_with("-shadow") && !req_bool(req, "shadow_intent").unwrap_or(false) {
        return fail(
            2,
            format!(
                "enqueue: queue {queue} is a shadow queue; pass shadow_intent \
                 (CLI: --shadow-intent) to enqueue there deliberately"
            ),
        );
    }
    let mut task = task_from_packet(&packet, &queue);
    // #8097 Rule 2: the optional coarse platform selector. The
    // request field (CLI --platform) wins over the packet's own
    // "platform"; absent both, the row stores NULL (unset — either
    // family may claim it). The selector names a family, never a
    // lane, and is never required.
    let platform = match parse_platform(
        req_str(req, "platform").or_else(|| packet.get("platform").and_then(|v| v.as_str())),
    ) {
        Ok(p) => p,
        Err(e) => return fail(2, format!("enqueue: {e}")),
    };
    if let Some(obj) = task.as_object_mut() {
        obj.insert("platform".to_string(), json!(platform));
    }
    if req
        .get("dry_run")
        .and_then(|d| d.as_bool())
        .unwrap_or(false)
    {
        return ok(json!({"dry_run": true, "task": task, "warnings": warnings}));
    }
    let now = now_iso();
    let dedupe = task
        .get("dedupe_key")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let work = (|| -> Result<(Value, Option<Value>), TErr> {
        let mut predecessor: Option<Value> = None;
        if let Some(dk) = &dedupe {
            let existing = t.query_objs(
                &format!(
                    "SELECT {TASK_COLS} FROM tasks WHERE queue = ? AND dedupe_key = ? \
                     ORDER BY id ASC"
                ),
                &[Arg::Text(queue.clone()), Arg::Text(dk.clone())],
                &TASK_JSON_COLS,
            )?;
            let (live, terminal) = dedupe_partition(&existing);
            if let Some(row) = live {
                return Ok((row, None));
            }
            predecessor = terminal;
        }
        let ins = t.exec(
            "INSERT INTO tasks (queue, title, body, payload, dedupe_key, priority, \
               status, not_before, max_attempts, platform, progress, result, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, '{}', '{}', ?, ?)",
            &[
                Arg::Text(queue.clone()),
                arg_str(task.get("title").and_then(|v| v.as_str())),
                arg_str(task.get("body").and_then(|v| v.as_str())),
                Arg::Text(
                    task.get("payload").cloned().unwrap_or(json!({})).to_string(),
                ),
                arg_str(dedupe.as_deref()),
                task.get("priority")
                    .and_then(|v| v.as_i64())
                    .map(Arg::Int)
                    .unwrap_or(Arg::Null),
                arg_str(task.get("not_before").and_then(|v| v.as_str())),
                task.get("max_attempts")
                    .and_then(|v| v.as_i64())
                    .map(Arg::Int)
                    .unwrap_or(Arg::Int(4)),
                arg_str(platform.as_deref()),
                Arg::Text(now.clone()),
                Arg::Text(now),
            ],
        )?;
        let id = ins.last_insert_rowid.unwrap_or(0);
        // Revive-on-enqueue (#6838): the key's only prior row(s) are
        // terminal, so this fresh task is a revival, not a first
        // enqueue. Link it to its predecessor in progress (the audit
        // chain) and leave an event on BOTH rows.
        if let Some(pred) = &predecessor {
            let pred_id = pred.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let link = json!({
                "task_id": pred_id,
                "status": pred.get("status").cloned().unwrap_or(Value::Null),
                "completed_at": pred.get("completed_at").cloned().unwrap_or(Value::Null),
                "revived_at": now_iso(),
                "via": "enqueue_repo_task_turso",
            });
            t.exec(
                "UPDATE tasks SET progress = ? WHERE id = ?",
                &[
                    Arg::Text(json!({"revived_from": link}).to_string()),
                    Arg::Int(id),
                ],
            )?;
            event(
                t,
                Some("enqueue_repo_task"),
                Some(id),
                "task_revived",
                &json!({"predecessor": link, "dedupe_key": dedupe}),
            )?;
            event(
                t,
                Some("enqueue_repo_task"),
                Some(pred_id),
                "revived_by",
                &json!({"successor_task_id": id, "dedupe_key": dedupe}),
            )?;
        }
        let row = get_task(t, id).map(|r| r.unwrap_or(Value::Null))?;
        Ok((row, predecessor))
    })();
    match work {
        Ok((row, predecessor)) if !row.is_null() => {
            let mut envelope = json!({
                "row": row, "rpc": "enqueue_repo_task_turso", "warnings": warnings
            });
            if let Some(pred) = &predecessor {
                envelope["revived_from"] = json!({
                    "task_id": pred.get("id").cloned().unwrap_or(Value::Null),
                    "status": pred.get("status").cloned().unwrap_or(Value::Null),
                });
            }
            ok(envelope)
        }
        Ok(_) => fail(5, "enqueue_repo_task_turso returned no row"),
        Err(e) => terr(e),
    }
}

/// Whether a task row may be revived by the /revive verb (#6838):
/// a failed row can always be re-queued; a queued row only when it
/// is dead at cap (attempts exhausted — the silent-zombie state
/// #6463 censused). Anything else is live or terminal-done work the
/// verb must not touch.
fn revive_eligible(status: &str, attempts: i64, max_attempts: i64) -> Result<(), String> {
    match status {
        "failed" => Ok(()),
        "queued" if attempts >= max_attempts => Ok(()),
        "queued" => Err(format!(
            "task is queued with attempts {attempts} < max_attempts {max_attempts}: \
             not dead at cap, nothing to revive"
        )),
        other => Err(format!(
            "task status {other} cannot be revived (only failed, or queued dead-at-cap)"
        )),
    }
}

/// The revive verb (#6838): reset a dead task in place — attempts
/// back to 0, last_error cleared, status queued, due now (or at the
/// caller's not_before) — with a `revived` audit event. This is the
/// supported path the fleet monitor's dead-in-queue self-repair
/// re-points to, replacing its direct SQL.
fn verb_revive(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker")
        .unwrap_or("fleet-queue-cli")
        .to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let not_before = req_str(req, "not_before")
        .map(str::to_string)
        .unwrap_or_else(now_iso);
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id).map_err(|e| e)? else {
            return Err(format!("revive_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let max_attempts = row
            .get("max_attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(4);
        revive_eligible(&status, attempts, max_attempts)
            .map_err(|e| format!("revive_task: task {task_id}: {e}"))?;
        t.exec(
            "UPDATE tasks SET status = 'queued', attempts = 0, last_error = NULL, \
               not_before = ?, worker_id = NULL, lease_until = NULL, \
               updated_at = ? WHERE id = ?",
            &[
                Arg::Text(not_before.clone()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )
        .map_err(|e| e)?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "revived",
            &json!({
                "reason": reason, "previous_status": status,
                "previous_attempts": attempts, "not_before": not_before,
                "via": "revive_task_turso",
            }),
        )
        .map_err(|e| e)?;
        get_task(t, task_id)
            .map(|r| r.unwrap_or(Value::Null))
            .map_err(|e| e)
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(5, format!("revive_task: task {task_id} does not exist")),
        Err(e) if e.starts_with("revive_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

/// #5444: a stored payload counts as missing when it is NULL, or an
/// empty JSON object — the shape a payload-less row hands a worker.
/// (Empty/malformed payload TEXT never reaches here as a Value: the
/// claim candidate predicate filters it in SQL, and the quarantine
/// sweep below handles the stored forms.)
fn payload_missing(parsed: Option<&Value>) -> bool {
    match parsed {
        None => true,
        Some(Value::Null) => true,
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    }
}

/// The #5444 claim guard: a claimed row may be handed out only when
/// its payload is present AND the packet passes the same validator
/// /enqueue runs. The oracle is `validate_packet` — claim and enqueue
/// can never disagree about what a valid packet is. Returns the
/// quarantine reason for a row that must not be handed out.
fn claim_guard_reason(row: &Value) -> Option<String> {
    if payload_missing(row.get("payload")) {
        return Some("quarantined by claim path: payload missing/empty (#5444)".to_string());
    }
    let (errors, _warnings) = validate_packet(row);
    if errors.is_empty() {
        None
    } else {
        Some(format!(
            "quarantined by claim path: packet failed validation (#5444): {}",
            errors.join("; ")
        ))
    }
}

/// Quarantine one task (#5444): blocked with the reason in last_error
/// and a `blocked` event row, claim and lease cleared — the same
/// terminal shape verb_block produces — so a payload-less or
/// schema-invalid row stops circulating instead of crashing or
/// misleading the next worker that claims it.
fn quarantine_task(t: &TursoBackend, id: i64, reason: &str) -> Result<(), TErr> {
    t.exec(
        "UPDATE tasks SET status = 'blocked', worker_id = NULL, \
           lease_until = NULL, last_error = ?, updated_at = ? WHERE id = ?",
        &[
            Arg::Text(reason.to_string()),
            Arg::Text(now_iso()),
            Arg::Int(id),
        ],
    )?;
    t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(id)])?;
    event(
        t,
        None,
        Some(id),
        "blocked",
        &json!({"reason": reason, "via": "claim_payload_quarantine"}),
    )?;
    Ok(())
}

/// Sweep this queue's claimable rows for stored payload-less forms
/// (NULL / empty / '{}') and quarantine them (#5444 acceptance 1;
/// Turso mirror of migration-0025's quarantine step). Runs on the
/// claim path, which is where such rows would otherwise circulate.
fn quarantine_payload_missing(t: &TursoBackend, queue: &str) -> Result<(), TErr> {
    let rows = t.query_objs(
        "SELECT id FROM tasks WHERE queue = ? AND status IN ('queued', 'parked', 'handed_off') \
           AND (payload IS NULL OR payload = '' OR payload = '{}')",
        &[Arg::Text(queue.to_string())],
        &[],
    )?;
    for r in rows {
        if let Some(id) = r.get("id").and_then(|v| v.as_i64()) {
            quarantine_task(
                t,
                id,
                "quarantined by claim path: payload missing/empty (#5444)",
            )?;
        }
    }
    Ok(())
}

/// The one-queue form of the #5444 payload guard sweep: an
/// unscoped claim (no stream named) sweeps every stream's
/// payload-less rows, exactly as a scoped claim sweeps its own.
fn quarantine_payload_missing_all(t: &TursoBackend) -> Result<(), TErr> {
    let rows = t.query_objs(
        "SELECT id FROM tasks WHERE status IN ('queued', 'parked', 'handed_off') \
           AND (payload IS NULL OR payload = '' OR payload = '{}')",
        &[],
        &[],
    )?;
    for r in rows {
        if let Some(id) = r.get("id").and_then(|v| v.as_i64()) {
            quarantine_task(
                t,
                id,
                "quarantined by claim path: payload missing/empty (#5444)",
            )?;
        }
    }
    Ok(())
}

/// #7617: the claim lease default and the heartbeat extension
/// default are different contracts and must never be conflated: a
/// claim taken without --lease-seconds lives for
/// CLAIM_DEFAULT_LEASE_SECONDS (2700), while a heartbeat without
/// --extend-seconds extends by HEARTBEAT_DEFAULT_EXTEND_SECONDS
/// (900). The 2026-11-08 incident (task 11049) reported
/// lease_until = claimed_at + 2700 while the claim died at ~900 s.
pub(crate) const CLAIM_DEFAULT_LEASE_SECONDS: i64 = 2700;
pub(crate) const HEARTBEAT_DEFAULT_EXTEND_SECONDS: i64 = 900;

/// The lease_until a claim stores — on BOTH the tasks row and the
/// leases row: exactly claimed_at + the requested lease, never the
/// heartbeat extension default.
fn claim_lease_until(claimed_at_unix: i64, lease_seconds: i64) -> String {
    iso(claimed_at_unix + lease_seconds)
}

/// The coarse platform families a platform MARK may name (#8097
/// Rule 2, demoted by #8187): a task's `platform` is NULL (unmarked)
/// or exactly one of these. A platform names a FAMILY, never a
/// specific lane — and since #8187 it is a MARK ONLY: it records
/// which platform is looking at / looked at an item, and is never
/// a claim filter, never an eligibility condition, and never a
/// lane-assignment routing reason.
pub const PLATFORM_FAMILIES: [&str; 2] = ["openchamber", "muse"];

/// Parse an optional platform mark (an enqueue request field, a
/// packet field, or a claim's declared platform). Missing/empty
/// means unmarked; anything that is not one of PLATFORM_FAMILIES
/// is an error, so a typo can never silently corrupt the mark.
fn parse_platform(raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(p) if PLATFORM_FAMILIES.contains(&p) => Ok(Some(p.to_string())),
        Some(p) => Err(format!(
            "invalid platform {p:?} (expected \"openchamber\" or \"muse\")"
        )),
    }
}

/// The claiming session's own model, as a claim request carries it
/// (#8176): provider + model + optional variant — the serving leaf
/// the session read from its own session surface (the route_usage
/// lastServing readout, opencode#1891) at claim time. A claim that
/// names no model carries None and stamps nothing. Claims by
/// non-session actors (Muse lanes) carry the distinct provider
/// marker "muse-lane" with no model, never a fabricated leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaimModel {
    provider: String,
    model: Option<String>,
    variant: Option<String>,
}

/// Parse the optional claim-model fields of a claim request
/// (#8176). Provider and model travel together: naming one without
/// the other is a usage error, so a half-stamp can never land. The
/// variant is optional (a leaf need not carry one). Empty strings
/// count as absent.
fn parse_claim_model(req: &Value) -> Result<Option<ClaimModel>, String> {
    let field = |key: &str| {
        req_str(req, key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let provider = field("model_provider");
    let model = field("model_id");
    let variant = field("model_variant");
    match (provider, model) {
        (None, None) => {
            if variant.is_some() {
                return Err(
                    "model_variant requires model_provider and model_id".to_string()
                );
            }
            Ok(None)
        }
        (Some(p), Some(m)) => Ok(Some(ClaimModel {
            provider: p,
            model: Some(m),
            variant,
        })),
        (Some(_), None) => {
            Err("model_provider requires model_id (the claim-model stamp names the serving leaf, provider and model together)".to_string())
        }
        (None, Some(_)) => {
            Err("model_id requires model_provider (the claim-model stamp names the serving leaf, provider and model together)".to_string())
        }
    }
}

/// #8176: stamp the claiming session's own model on a just-claimed
/// row. Claim-time semantics: the stamp is the leaf serving the
/// session WHEN it claimed; a later failover changes the serving
/// leaf (still visible via route_usage) but never rewrites the
/// stamp. A claim carrying no model leaves the columns untouched
/// (NULL on a fresh row). The returned row is patched in place so
/// the claim's answer carries the stamp without a re-read.
fn stamp_claim_model(
    t: &TursoBackend,
    task_id: i64,
    row: &mut Value,
    model: &Option<ClaimModel>,
) -> Result<(), TErr> {
    let Some(m) = model else {
        return Ok(());
    };
    t.exec(
        "UPDATE tasks SET claim_provider = ?, claim_model = ?, \
           claim_variant = ? WHERE id = ?",
        &[
            Arg::Text(m.provider.clone()),
            arg_str(m.model.as_deref()),
            arg_str(m.variant.as_deref()),
            Arg::Int(task_id),
        ],
    )?;
    if let Some(obj) = row.as_object_mut() {
        obj.insert("claim_provider".to_string(), json!(m.provider));
        obj.insert("claim_model".to_string(), json!(m.model));
        obj.insert("claim_variant".to_string(), json!(m.variant));
    }
    Ok(())
}

/// The SQL predicate mirroring is_handed_off, for the broker's
/// no-id handoff pick (#8097 Rule 1): a row in the handoff sink —
/// status 'handed_off' (#8234), progress lane opencode, and no
/// session binding. Sink membership is the STATUS, written once
/// by the park verb's classification, the lease-expiry sink
/// return, or the #8234 migration; this predicate deliberately
/// does NOT reconstruct it from progress stage — the broker and
/// the claimant poll once disagreed about which stage source
/// (top-level vs last_checkpoint) named a sink row, and rows
/// stranded in that gap (#8192). The lane and session conditions
/// stay as guards: a bound row is a live session's business, and
/// only an opencode yield is the claimant's to collect.
const HANDOFF_SQL: &str = "status = 'handed_off' \
    AND json_extract(progress, '$.lane') = 'opencode' \
    AND json_extract(progress, '$.session_id') IS NULL";

/// The yield stage a task's progress records: the top-level `stage`
/// when present, else the last checkpoint's stage (the checkpoint
/// verb records it under progress.last_checkpoint.stage, #7706 — the
/// same lift poll.py's normalize_row performs).
fn progress_stage(progress: &Value) -> Option<&str> {
    progress.get("stage").and_then(|v| v.as_str()).or_else(|| {
        progress
            .get("last_checkpoint")
            .and_then(|c| c.get("stage"))
            .and_then(|v| v.as_str())
    })
}

/// The progress shape of a recorded opencode handoff (#8184):
/// the opencode lane assignment plus a yield stage (handed_off, or
/// launch_failed after a claimant recovery), regardless of the
/// row's status or session binding. This shape is the WRITE-side
/// classification: the park verb uses it to decide that a park
/// lands in the sink (status 'handed_off', #8234), and
/// expire_leases uses it to decide that an expired claimed row
/// returns to the sink. It is never used to decide sink
/// MEMBERSHIP for a resting row — that is the status itself
/// (is_handed_off) — because the shape's stage can be read two
/// ways (progress_stage prefers the top-level stage; the
/// claimant's normalize_row lifts last_checkpoint), and #8192 was
/// exactly the two readers disagreeing about a resting row.
/// Historically this shape sat in status 'parked' with the park
/// verb's default +20-minute timer stamped on it; 164 such rows
/// sat in repo-health-code-pilot alone on 2026-11-09 while their
/// issues (#7907, #6533) cycled claim → handoff → park → re-claim
/// for days.
fn handoff_shape(progress: &Value) -> bool {
    progress.get("lane").and_then(|v| v.as_str()) == Some("opencode")
        && matches!(
            progress_stage(progress),
            Some("handed_off") | Some("launch_failed")
        )
}

/// A row is IN the handoff sink (#8234) when its status is
/// 'handed_off', its progress carries the opencode lane, and no
/// session is bound. The status is the membership — written only
/// by the park classification, the lease-expiry sink return, and
/// the #8234 migration, all of which gate on handoff_shape plus
/// an absent binding — so the lane check here is a guard, not a
/// reconstruction, and no stage is consulted at all (#8192).
fn is_handed_off(row: &Value) -> bool {
    if row.get("status").and_then(|v| v.as_str()) != Some("handed_off") {
        return false;
    }
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    if progress.get("lane").and_then(|v| v.as_str()) != Some("opencode") {
        return false;
    }
    match progress.get("session_id") {
        Some(Value::Null) | None => true,
        Some(_) => false,
    }
}

/// Whether a park lands in the handoff sink (#8234): the merged
/// progress is a recorded opencode handoff (handoff_shape), and
/// the caller declared no genuine wait — the ready condition is
/// absent (defaulted by the park verb), or a bare {"kind": "time"}
/// timer, which on a handoff row was only ever the park verb's
/// own default bookkeeping. Any other condition kind (ci/triage
/// floors aside, task_terminal, task_status, event_seen,
/// quota_reset, person_answer) is a real wait the caller named,
/// and that park stays a park. The caller passes the
/// binding-cleared merged progress, so a bound session's row —
/// whose stage may still read as a yield — never sinks.
fn handoff_park(progress: &Value, condition_defaulted: bool, condition: &Value) -> bool {
    if !handoff_shape(progress) {
        return false;
    }
    match progress.get("session_id") {
        Some(Value::Null) | None => {}
        Some(_) => return false,
    }
    condition_defaulted || condition.get("kind").and_then(|v| v.as_str()) == Some("time")
}

/// Where an expired claim returns (#8184): a claimed row whose
/// progress is a recorded opencode handoff goes back to the
/// handoff sink (status 'handed_off', #8234), never to queued —
/// the claimant collects it from the sink at +0 attempts, so a
/// claimant launch that died on the lease can never burn an
/// attempt through the queued interlude. Anything else (a direct
/// claim, a bound session mid-work whose stage advanced past the
/// yield stages) returns to queued as before.
fn expired_claim_returns_to_sink(progress: &Value) -> bool {
    handoff_shape(progress)
}

/// The lease-expiry sink return as a statement builder (#8234),
/// extracted so the regression tests pin its shape without a
/// live store (the claim_update_sql pattern): an expired claimed
/// handoff lands in status 'handed_off' with every wait field
/// NULL — no not_before, no park_kind, no ready_condition. The
/// pre-#8234 return stamped a fresh +1200s not_before and a
/// {"kind": "time"} ready_condition: a fake timer on a state
/// that is not a wait, which the sink pick never read.
fn sink_return_sql() -> &'static str {
    "UPDATE tasks SET status = 'handed_off', worker_id = NULL, lease_until = NULL, \
       not_before = NULL, park_kind = NULL, wait_kind = NULL, \
       ready_condition = NULL, progress = ?, updated_at = ? \
     WHERE id = ? AND status = 'claimed' AND lease_until < ?"
}

/// Whether a by-id claim (claim_repo_task with task_id) may take
/// exactly this row. Two shapes only: a queued row under the pick
/// path's ordinary predicate (attempts cap + not_before), or a
/// sink row (is_handed_off, #8234) — the sink is not a wait, so
/// nothing gates the claimant collecting its own handoff.
/// Anything else (claimed, blocked, a genuine wait park — a
/// 'parked' row is never a sink row since #8234, whatever its
/// progress carries) is not claimable by id.
fn by_id_claimable(row: &Value, now: &str) -> bool {
    // #8187: platform is a mark, not a filter — a by-id claim takes
    // the row whatever platform either side carries (or none).
    match row.get("status").and_then(|v| v.as_str()) {
        Some("queued") => {
            let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
            let max = row
                .get("max_attempts")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let nb = row.get("not_before").and_then(|v| v.as_str());
            attempts < max && nb.map_or(true, |nb| nb <= now)
        }
        Some("handed_off") => is_handed_off(row),
        _ => false,
    }
}

/// Session-to-session transfer (#8210): claim task_id DIRECTLY
/// from its current holder, named by from_session, in one
/// operation — the task never passes through queued or parked,
/// there is no release-and-reclaim gap, and when the request
/// carries a session_id the claiming session is bound in the
/// same operation (progress + lane ledger, the marks a separate
/// bind would write). Callable by the claimant machinery itself:
/// the claiming worker is whoever invokes it, recorded like any
/// claim. Every refusal is computed before the single
/// conditional UPDATE and mutates nothing.
fn claim_transfer(
    t: &TursoBackend,
    task_id: i64,
    queue: Option<&str>,
    worker: &str,
    lane: &Option<String>,
    lease_seconds: i64,
    family: Option<&str>,
    model: &Option<ClaimModel>,
    from_session: &str,
    new_session: Option<&str>,
    pool: &Option<String>,
) -> Result<Option<Value>, TErr> {
    let Some(row) = get_task(t, task_id)? else {
        return Ok(None);
    };
    if let Some(q) = queue {
        if row.get("queue").and_then(|v| v.as_str()) != Some(q) {
            return Ok(None);
        }
    }
    if let Some(reason) = claim_guard_reason(&row) {
        quarantine_task(t, task_id, &reason)?;
        return Ok(None);
    }
    // The holder's session as recorded on the latest claimed
    // event — the mark a session leaves when it claims with
    // --session-id.
    let ev_rows = t.query_objs(
        "SELECT detail FROM events WHERE task_id = ? AND kind = 'claimed' \
         ORDER BY id DESC LIMIT 1",
        &[Arg::Int(task_id)],
        &[],
    )?;
    let ev_detail = ev_rows.first().and_then(|e| e.get("detail")).and_then(|d| {
        match d {
            Value::String(s) => serde_json::from_str(s).ok(),
            Value::Object(_) => Some(d.clone()),
            _ => None,
        }
    });
    let claimed_event_session = ev_detail
        .as_ref()
        .and_then(|d| d.get("session_id"))
        .and_then(|v| v.as_str());
    let ledger = get_ledger(t, task_id)?;
    let ledger_session = ledger
        .as_ref()
        .and_then(|l| l.get("session_id"))
        .and_then(|v| v.as_str());
    // Platform limitation (#8210): a recorded, reason-carrying
    // limitation binds this transfer; a reason-less stored one
    // is invalid — cleared here, never enforced, and the
    // clearing is carried on the claimed event below.
    let limit = ledger
        .as_ref()
        .and_then(|l| l.get("platform_limit"))
        .and_then(|v| v.as_str());
    let limit_reason = ledger
        .as_ref()
        .and_then(|l| l.get("platform_limit_reason"))
        .and_then(|v| v.as_str());
    let mut limit_cleared_invalid = false;
    if platform_limit_invalid(limit, limit_reason) {
        t.exec(
            "UPDATE repo_task_lane_ledger SET platform_limit = NULL, \
               platform_limit_reason = NULL, updated_at = ? WHERE task_id = ?",
            &[Arg::Text(now_iso()), Arg::Int(task_id)],
        )?;
        limit_cleared_invalid = true;
    } else if let Some(r) = platform_limit_refusal(limit, limit_reason, family) {
        return Err(format!("claim_repo_task: transfer refused: {r}"));
    }
    // One-queue union matching (#8097, 2026-11-10): the task's
    // own declared requirements (payload `requires`) bind a
    // transfer exactly as they bind the pick and the by-id
    // claim.
    let transfer_claimer = ClaimerCtx {
        family: family.map(str::to_string),
        lane: lane.clone(),
        worker: worker.to_string(),
        pool: pool.clone(),
        model: model.clone(),
    };
    if let Some(r) = task_requirements_unmet(&row, &transfer_claimer) {
        return Err(format!("claim_repo_task: transfer refused: {r}"));
    }
    let now_unix = now_unix();
    let now = iso(now_unix);
    if let Some(reason) = transfer_refusal(
        &row,
        from_session,
        claimed_event_session,
        ledger_session,
        new_session,
        &now,
    ) {
        return Err(format!("claim_repo_task: transfer refused: {reason}"));
    }
    let old_worker = row
        .get("worker_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_lane = row
        .get("lane_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let old_bound = row
        .get("progress")
        .and_then(|p| p.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let new_lane = lane.clone().or_else(|| old_lane.clone());
    let lease_until = claim_lease_until(now_unix, lease_seconds);
    let upd = t.exec(
        transfer_update_sql(),
        &[
            Arg::Text(worker.to_string()),
            Arg::Text(now.clone()),
            Arg::Text(lease_until.clone()),
            arg_str(new_lane.as_deref()),
            arg_str(family),
            Arg::Text(now.clone()),
            Arg::Int(task_id),
            Arg::Text(old_worker.clone()),
        ],
    )?;
    if upd.affected == 0 {
        return Err(
            "claim_repo_task: transfer refused: the claim changed under the transfer"
                .to_string(),
        );
    }
    t.exec(
        "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id, \
           lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at",
        &[
            Arg::Int(task_id),
            Arg::Text(worker.to_string()),
            Arg::Text(lease_until),
            Arg::Text(now_iso()),
        ],
    )?;
    let mut row = get_task(t, task_id)?.unwrap_or(row);
    // Bind the claiming session in the same operation.
    if let Some(sid) = new_session {
        let progress = merge_obj(
            &row.get("progress").cloned().unwrap_or(json!({})),
            &json!({"session_id": sid, "launched_by": worker}),
        );
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        if ledger.is_some() {
            t.exec(
                "UPDATE repo_task_lane_ledger SET session_id = ?, updated_at = ? \
                 WHERE task_id = ?",
                &[
                    Arg::Text(sid.to_string()),
                    Arg::Text(now_iso()),
                    Arg::Int(task_id),
                ],
            )?;
        }
        if let Some(obj) = row.as_object_mut() {
            obj.insert("progress".to_string(), progress);
        }
    }
    stamp_claim_model(t, task_id, &mut row, model)?;
    touch_worker(t, worker, Some(task_id))?;
    event(
        t,
        Some(worker),
        Some(task_id),
        "claimed",
        &with_marks(
            &json!({
                "lease_seconds": lease_seconds, "via": "claim_repo_task_transfer",
                "transfer": true, "by_id": true, "task_id": task_id,
                "from_session": from_session, "session_id": new_session,
                "previous_worker": old_worker, "previous_lane_id": old_lane,
                "previous_session_id": old_bound,
                "platform_limit": if limit_cleared_invalid { None } else { limit },
                "platform_limit_reason": if limit_cleared_invalid { None } else { limit_reason },
                "platform_limit_cleared_invalid": limit_cleared_invalid,
                "lane_id": new_lane, "platform": family,
                "model_provider": model.as_ref().map(|m| m.provider.clone()),
                "model_id": model.as_ref().and_then(|m| m.model.clone()),
                "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
            }),
            &row,
        ),
    )?;
    Ok(Some(row))
}

/// By-id claim: take exactly the requested row when by_id_claimable
/// allows it, through the same post-claim flow as the pick path
/// (guard quarantine, binding clear, lease row, claimed event).
/// The conditional UPDATE re-checks the status read above, so a
/// competing claim or transition in between loses cleanly (0 rows
/// affected → no claim). Returns the claimed row, or None when the
/// row is not claimable by id.
fn claim_by_id(
    t: &TursoBackend,
    task_id: i64,
    queue: Option<&str>,
    worker: &str,
    lane: &Option<String>,
    lease_seconds: i64,
    family: Option<&str>,
    model: &Option<ClaimModel>,
    pool: &Option<String>,
) -> Result<Option<Value>, TErr> {
    let Some(row) = get_task(t, task_id)? else {
        return Ok(None);
    };
    if let Some(q) = queue {
        if row.get("queue").and_then(|v| v.as_str()) != Some(q) {
            return Ok(None);
        }
    }
    let row_queue = row
        .get("queue")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(reason) = claim_guard_reason(&row) {
        quarantine_task(t, task_id, &reason)?;
        return Ok(None);
    // #8217: the plain by-id path honors the same recorded
    // platform limitation the transfer claim enforces (#8210):
    // an invalid (reason-less) stored limitation is cleared,
    // never enforced; a valid one refuses a claim from any
    // other family (or none) — the row simply reads unclaimable
    // by id, like any other by-id refusal.
    if let Some(ledger) = get_ledger(t, task_id)? {
        let limit = ledger
            .get("platform_limit")
            .and_then(|v| v.as_str());
        let limit_reason = ledger
            .get("platform_limit_reason")
            .and_then(|v| v.as_str());
        if platform_limit_invalid(limit, limit_reason) {
            t.exec(
                "UPDATE repo_task_lane_ledger SET platform_limit = NULL, \
                   platform_limit_reason = NULL, updated_at = ? WHERE task_id = ?",
                &[Arg::Text(now_iso()), Arg::Int(task_id)],
            )?;
        } else if platform_limit_refusal(limit, limit_reason, family).is_some() {
            return Ok(None);
        }
    }
    }
    let status = row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now_unix = now_unix();
    let now = iso(now_unix);
    if !by_id_claimable(&row, &now) {
        return Ok(None);
    }
    // One-queue union matching (#8097, 2026-11-10): a by-id
    // claim honors the task's own declared requirements
    // (payload `requires`) exactly as the pick does — an
    // ineligible claimer simply reads the row as unclaimable,
    // with no out-of-band step.
    let claimer = ClaimerCtx {
        family: family.map(str::to_string),
        lane: lane.clone(),
        worker: worker.to_string(),
        pool: pool.clone(),
        model: model.clone(),
    };
    if task_requirements_unmet(&row, &claimer).is_some() {
        return Ok(None);
    }
    let lease_until = claim_lease_until(now_unix, lease_seconds);
    let upd = t.exec(
        "UPDATE tasks SET status = 'claimed', worker_id = ?, claimed_at = ?, \
           lease_until = ?, \
           attempts = attempts + (CASE WHEN status IN ('parked', 'handed_off') THEN 0 ELSE 1 END), \
           lane_id = ?, platform = COALESCE(?, platform), \
           park_kind = NULL, wait_kind = NULL, not_before = NULL, \
           updated_at = ? \
         WHERE id = ? AND status = ? AND queue = ?",
        &[
            Arg::Text(worker.to_string()),
            Arg::Text(now.clone()),
            Arg::Text(lease_until.clone()),
            arg_str(lane.as_deref()),
            arg_str(family),
            Arg::Text(now.clone()),
            Arg::Int(task_id),
            Arg::Text(status),
            Arg::Text(row_queue.clone()),
        ],
    )?;
    if upd.affected == 0 {
        return Ok(None);
    }
    let rows = t.query_objs(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
        &[Arg::Int(task_id)],
        &TASK_JSON_COLS,
    )?;
    let Some(mut row) = rows.into_iter().next() else {
        return Ok(None);
    };
    // Migration 0023 §4: a fresh claim never inherits a binding.
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let (progress, cleared) = clear_binding(&progress, "claim_repo_task");
    if cleared.is_some() {
        t.exec(
            "UPDATE tasks SET progress = ? WHERE id = ?",
            &[Arg::Text(progress.to_string()), Arg::Int(task_id)],
        )?;
        if let Some(obj) = row.as_object_mut() {
            obj.insert("progress".to_string(), progress);
        }
    }
    t.exec(
        "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id, \
           lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at",
        &[
            Arg::Int(task_id),
            Arg::Text(worker.to_string()),
            Arg::Text(lease_until),
            Arg::Text(now_iso()),
        ],
    )?;
    stamp_claim_model(t, task_id, &mut row, model)?;
    touch_worker(t, worker, Some(task_id))?;
    if let Some(old) = &cleared {
        on_binding_cleared(t, worker, task_id, old, "claim_repo_task")?;
    }
    event(
        t,
        Some(worker),
        Some(task_id),
        "claimed",
        &with_marks(
            &json!({
                "lease_seconds": lease_seconds, "via": "claim_repo_task_turso",
                "cleared_session_id": cleared, "lane_id": lane,
                "by_id": true, "task_id": task_id, "platform": family,
                "model_provider": model.as_ref().map(|m| m.provider.clone()),
                "model_id": model.as_ref().and_then(|m| m.model.clone()),
                "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
            }),
            &row,
        ),
    )?;
    Ok(Some(row))
}

/// The pick path's claim UPDATE (#7068, #7617): keyed to the
/// picked row's id and re-checking the full pick predicate at claim
/// instant, so a claim that loses the race for a row affects 0 rows
/// and the caller re-picks — it can never be handed the winner's
/// row. Extracted as a builder so the regression tests below pin
/// the statement's shape without a live store.
/// One-queue claim matching (#8097 one-queue model, Drew
/// 2026-11-10): there is ONE fleet queue. A claim matches the
/// UNION of two sides' requirements, each side filtering on the
/// other's metadata:
///
///   * the TASK's requirements filter claimers: the lane-ledger
///     platform limitation (PLATFORM_LIMIT_PICK_CLAUSE, #8217) and
///     the payload `requires` object enforced by
///     TASK_REQUIREMENTS_CLAUSE below — `requires.platform` (a
///     PLATFORM_FAMILIES family), `requires.lane` (a lane id),
///     `requires.pool` (a pool name), `requires.model` (a model
///     id, or "provider/model"). A claimer that does not declare
///     the required attribute does not meet the requirement.
///   * the CLAIMER's requirements filter tasks: the optional
///     stream scope (the request's `queue`) and the boolean
///     filter expression (the request's `filter`, spelled
///     --filter on the CLI) compiled by filter_sql below.
///
/// By default BOTH sides are empty: a task carries no
/// requirements and a claim passes no queue and no filter, and
/// the claim is a pure priority-order pick over every available
/// row in the one queue. The claimer's identity (worker, lane,
/// platform family, pool, model) is recorded on every claim —
/// the claim stamps, unchanged — and is used ONLY to satisfy a
/// task's own declared requirements, never as a filter in its
/// own right. The registry's per-lane queue column no longer
/// scopes what a lane may claim: claim_preamble enforces the
/// lane<->stream binding only when the claim itself names a
/// stream AND the lane's registration binds one (a registration
/// of "*" is the recorded unbound state).
///
/// Filter grammar (recursive descent; AND binds tighter than
/// OR, NOT tightest; parentheses override):
///   expr       := or
///   or         := and (OR and)*
///   and        := unary (AND unary)*
///   unary      := NOT unary | primary
///   primary    := '(' expr ')' | comparison
///   comparison := field (('=' | '!=' | '<' | '<=' | '>' | '>=')
///                        value
///                        | [NOT] IN '(' value (',' value)* ')')
/// Fields (task metadata): repo, queue, pool (the task's stream
/// — pool names the task's queue, as in the claimed event's
/// marks), platform (the row's platform mark), label (membership
/// in the payload's `labels` array), kind (payload `kind`),
/// priority (the row's integer priority), lane / model (the
/// task's own declared requires.lane / requires.model). Ordering
/// comparisons are valid only on priority. A comparison against
/// an absent value is false for '=' / IN / ordering comparisons
/// and true for '!=' / NOT IN (the task does not carry that
/// value).
///
/// `repo` matches the task's repo identity: the payload's `repo`
/// (a full "owner/name") when present, else the stream name with
/// the repo-health- prefix and -pilot / -shadow suffixes
/// stripped (repo-health-code-pilot -> code). A filter value
/// matches the identity exactly or as its final path segment, so
/// `repo = code` matches payload repo "developing-today/code"
/// and stream-derived "code" alike, and `repo != code` is the
/// not-code lanes' claim expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Repo,
    Queue,
    Pool,
    Platform,
    Label,
    Kind,
    Priority,
    Lane,
    Model,
}

impl FilterField {
    fn parse(name: &str) -> Option<FilterField> {
        match name.to_ascii_lowercase().as_str() {
            "repo" => Some(FilterField::Repo),
            "queue" => Some(FilterField::Queue),
            "pool" => Some(FilterField::Pool),
            "platform" => Some(FilterField::Platform),
            "label" => Some(FilterField::Label),
            "kind" => Some(FilterField::Kind),
            "priority" => Some(FilterField::Priority),
            "lane" => Some(FilterField::Lane),
            "model" => Some(FilterField::Model),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterExpr {
    Or(Box<FilterExpr>, Box<FilterExpr>),
    And(Box<FilterExpr>, Box<FilterExpr>),
    Not(Box<FilterExpr>),
    Cmp {
        field: FilterField,
        op: CmpOp,
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum FilterTok {
    LParen,
    RParen,
    Comma,
    Op(CmpOp),
    Word(String),
    Str(String),
}

fn filter_tokens(src: &str) -> Result<Vec<FilterTok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let word_char = |c: char| {
        c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#' | '@' | '*')
    };
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                toks.push(FilterTok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(FilterTok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(FilterTok::Comma);
                i += 1;
            }
            '=' => {
                toks.push(FilterTok::Op(CmpOp::Eq));
                i += 1;
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(FilterTok::Op(CmpOp::Ne));
                    i += 2;
                } else {
                    return Err("filter: '!' is only valid as part of '!='".to_string());
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(FilterTok::Op(CmpOp::Le));
                    i += 2;
                } else {
                    toks.push(FilterTok::Op(CmpOp::Lt));
                    i += 1;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(FilterTok::Op(CmpOp::Ge));
                    i += 2;
                } else {
                    toks.push(FilterTok::Op(CmpOp::Gt));
                    i += 1;
                }
            }
            '\'' | '"' => {
                let q = c;
                let mut s = String::new();
                let mut j = i + 1;
                while j < chars.len() && chars[j] != q {
                    s.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    return Err("filter: unterminated quoted value".to_string());
                }
                toks.push(FilterTok::Str(s));
                i = j + 1;
            }
            c if word_char(c) => {
                let mut s = String::new();
                while i < chars.len() && word_char(chars[i]) {
                    s.push(chars[i]);
                    i += 1;
                }
                toks.push(FilterTok::Word(s));
            }
            other => return Err(format!("filter: unexpected character {other:?}")),
        }
    }
    Ok(toks)
}

struct FilterParser<'a> {
    toks: &'a [FilterTok],
    pos: usize,
}

impl<'a> FilterParser<'a> {
    fn peek(&self) -> Option<&'a FilterTok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a FilterTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn at_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(FilterTok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn parse_or(&mut self) -> Result<FilterExpr, String> {
        let mut lhs = self.parse_and()?;
        while self.at_keyword("or") {
            self.next();
            let rhs = self.parse_and()?;
            lhs = FilterExpr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<FilterExpr, String> {
        let mut lhs = self.parse_unary()?;
        while self.at_keyword("and") {
            self.next();
            let rhs = self.parse_unary()?;
            lhs = FilterExpr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<FilterExpr, String> {
        if self.at_keyword("not") {
            self.next();
            let inner = self.parse_unary()?;
            return Ok(FilterExpr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<FilterExpr, String> {
        if matches!(self.peek(), Some(FilterTok::LParen)) {
            self.next();
            let e = self.parse_or()?;
            if !matches!(self.next(), Some(FilterTok::RParen)) {
                return Err("filter: expected ')'".to_string());
            }
            return Ok(e);
        }
        self.parse_comparison()
    }

    fn parse_value(&mut self) -> Result<String, String> {
        match self.next() {
            Some(FilterTok::Word(w)) => Ok(w.clone()),
            Some(FilterTok::Str(s)) => Ok(s.clone()),
            other => Err(format!("filter: expected a value, found {other:?}")),
        }
    }

    fn parse_comparison(&mut self) -> Result<FilterExpr, String> {
        let field_name = match self.next() {
            Some(FilterTok::Word(w)) => w.clone(),
            other => {
                return Err(format!(
                    "filter: expected a field name, found {other:?}"
                ))
            }
        };
        let field = FilterField::parse(&field_name).ok_or_else(|| {
            format!(
                "filter: unknown field {field_name:?} (fields: repo, queue, pool, \
                 platform, label, kind, priority, lane, model)"
            )
        })?;
        let negated_in = self.at_keyword("not")
            && matches!(self.toks.get(self.pos + 1), Some(FilterTok::Word(w)) if w.eq_ignore_ascii_case("in"));
        if negated_in || self.at_keyword("in") {
            if negated_in {
                self.next();
            }
            self.next();
            if !matches!(self.next(), Some(FilterTok::LParen)) {
                return Err("filter: expected '(' after IN".to_string());
            }
            let mut values = vec![self.parse_value()?];
            while matches!(self.peek(), Some(FilterTok::Comma)) {
                self.next();
                values.push(self.parse_value()?);
            }
            if !matches!(self.next(), Some(FilterTok::RParen)) {
                return Err("filter: expected ')' after the IN list".to_string());
            }
            return Ok(FilterExpr::Cmp {
                field,
                op: if negated_in { CmpOp::NotIn } else { CmpOp::In },
                values,
            });
        }
        let op = match self.next() {
            Some(FilterTok::Op(o)) => *o,
            other => {
                return Err(format!(
                    "filter: expected an operator after {field_name}, found {other:?}"
                ))
            }
        };
        let value = self.parse_value()?;
        Ok(FilterExpr::Cmp {
            field,
            op,
            values: vec![value],
        })
    }
}

/// Parse a claim's filter expression (the request's `filter`
/// field, the CLI's --filter). An absent or empty filter is the
/// default claim — no expression at all — and never reaches
/// here; a malformed one is a usage error, never a silent
/// no-op filter.
pub fn parse_filter(src: &str) -> Result<FilterExpr, String> {
    let toks = filter_tokens(src)?;
    if toks.is_empty() {
        return Err("filter: empty expression".to_string());
    }
    let mut p = FilterParser { toks: &toks, pos: 0 };
    let expr = p.parse_or()?;
    if p.pos != toks.len() {
        return Err("filter: unexpected trailing input".to_string());
    }
    validate_filter(&expr)?;
    Ok(expr)
}

fn validate_filter(expr: &FilterExpr) -> Result<(), String> {
    match expr {
        FilterExpr::Or(a, b) | FilterExpr::And(a, b) => {
            validate_filter(a)?;
            validate_filter(b)
        }
        FilterExpr::Not(a) => validate_filter(a),
        FilterExpr::Cmp { field, op, values } => {
            if values.is_empty() {
                return Err("filter: a comparison needs at least one value".to_string());
            }
            if !matches!(op, CmpOp::In | CmpOp::NotIn) && values.len() != 1 {
                return Err("filter: only IN takes multiple values".to_string());
            }
            match field {
                FilterField::Priority => {
                    for v in values {
                        v.parse::<i64>().map_err(|_| {
                            format!("filter: priority value {v:?} is not an integer")
                        })?;
                    }
                    Ok(())
                }
                _ => {
                    if matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge) {
                        return Err(
                            "filter: ordering comparisons are only valid on priority"
                                .to_string(),
                        );
                    }
                    Ok(())
                }
            }
        }
    }
}

/// SQL for the task's repo identity (see the section comment):
/// the payload's declared repo, else the stream-derived repo.
const REPO_IDENTITY_SQL: &str = "COALESCE(NULLIF(json_extract(payload, '$.repo'), ''), \
    CASE WHEN queue LIKE 'repo-health-%' \
         THEN replace(replace(replace(queue, 'repo-health-', ''), '-pilot', ''), '-shadow', '') \
         ELSE queue END)";

fn repo_eq_sql(v: &str, args: &mut Vec<Arg>) -> String {
    args.push(Arg::Text(v.to_string()));
    args.push(Arg::Text(v.to_string()));
    format!("({REPO_IDENTITY_SQL} = ? OR {REPO_IDENTITY_SQL} LIKE '%/' || ?)")
}

fn label_eq_sql(v: &str, args: &mut Vec<Arg>) -> String {
    args.push(Arg::Text(v.to_string()));
    "EXISTS (SELECT 1 FROM json_each(payload, '$.labels') je WHERE je.value = ?)".to_string()
}

fn text_subject_sql(field: FilterField) -> &'static str {
    match field {
        FilterField::Queue | FilterField::Pool => "queue",
        FilterField::Platform => "platform",
        FilterField::Kind => "json_extract(payload, '$.kind')",
        FilterField::Lane => "json_extract(payload, '$.requires.lane')",
        FilterField::Model => "json_extract(payload, '$.requires.model')",
        FilterField::Repo | FilterField::Label | FilterField::Priority => "",
    }
}

fn cmp_sql(field: FilterField, op: CmpOp, values: &[String], args: &mut Vec<Arg>) -> String {
    match field {
        FilterField::Repo => {
            let parts: Vec<String> = values.iter().map(|v| repo_eq_sql(v, args)).collect();
            match op {
                CmpOp::Eq | CmpOp::In => format!("({})", parts.join(" OR ")),
                CmpOp::Ne | CmpOp::NotIn => format!("(NOT ({}))", parts.join(" OR ")),
                _ => unreachable!("validated at parse: no ordering on repo"),
            }
        }
        FilterField::Label => {
            let parts: Vec<String> = values.iter().map(|v| label_eq_sql(v, args)).collect();
            match op {
                CmpOp::Eq | CmpOp::In => format!("({})", parts.join(" OR ")),
                CmpOp::Ne | CmpOp::NotIn => format!("(NOT ({}))", parts.join(" OR ")),
                _ => unreachable!("validated at parse: no ordering on label"),
            }
        }
        FilterField::Priority => {
            let nums: Vec<i64> = values
                .iter()
                .map(|v| v.parse::<i64>().unwrap_or(0))
                .collect();
            match op {
                CmpOp::Eq => {
                    args.push(Arg::Int(nums[0]));
                    "priority = ?".to_string()
                }
                CmpOp::Ne => {
                    args.push(Arg::Int(nums[0]));
                    "(priority IS NULL OR priority <> ?)".to_string()
                }
                CmpOp::Lt => {
                    args.push(Arg::Int(nums[0]));
                    "priority < ?".to_string()
                }
                CmpOp::Le => {
                    args.push(Arg::Int(nums[0]));
                    "priority <= ?".to_string()
                }
                CmpOp::Gt => {
                    args.push(Arg::Int(nums[0]));
                    "priority > ?".to_string()
                }
                CmpOp::Ge => {
                    args.push(Arg::Int(nums[0]));
                    "priority >= ?".to_string()
                }
                CmpOp::In => {
                    let ph: Vec<&str> = nums.iter().map(|_| "?").collect();
                    for n in &nums {
                        args.push(Arg::Int(*n));
                    }
                    format!("priority IN ({})", ph.join(", "))
                }
                CmpOp::NotIn => {
                    let ph: Vec<&str> = nums.iter().map(|_| "?").collect();
                    for n in &nums {
                        args.push(Arg::Int(*n));
                    }
                    format!("(priority IS NULL OR priority NOT IN ({}))", ph.join(", "))
                }
            }
        }
        field => {
            let subj = text_subject_sql(field);
            match op {
                CmpOp::Eq => {
                    args.push(Arg::Text(values[0].clone()));
                    format!("{subj} = ?")
                }
                CmpOp::Ne => {
                    args.push(Arg::Text(values[0].clone()));
                    format!("({subj} IS NULL OR {subj} <> ?)")
                }
                CmpOp::In => {
                    let ph: Vec<&str> = values.iter().map(|_| "?").collect();
                    for v in values {
                        args.push(Arg::Text(v.clone()));
                    }
                    format!("{subj} IN ({})", ph.join(", "))
                }
                CmpOp::NotIn => {
                    let ph: Vec<&str> = values.iter().map(|_| "?").collect();
                    for v in values {
                        args.push(Arg::Text(v.clone()));
                    }
                    format!("({subj} IS NULL OR {subj} NOT IN ({}))", ph.join(", "))
                }
                _ => unreachable!("validated at parse: no ordering on text fields"),
            }
        }
    }
}

/// Compile a filter expression to a SQL fragment over the tasks
/// table, appending its arguments in placeholder order. The
/// fragment is a single boolean expression; callers parenthesize
/// it into the pick.
fn filter_sql(expr: &FilterExpr, args: &mut Vec<Arg>) -> String {
    match expr {
        FilterExpr::Or(a, b) => format!("({} OR {})", filter_sql(a, args), filter_sql(b, args)),
        FilterExpr::And(a, b) => {
            format!("({} AND {})", filter_sql(a, args), filter_sql(b, args))
        }
        FilterExpr::Not(a) => format!("(NOT {})", filter_sql(a, args)),
        FilterExpr::Cmp { field, op, values } => cmp_sql(*field, *op, values, args),
    }
}

/// The task-side half of the union match: a row whose payload
/// declares `requires` is servable only by a claimer declaring
/// the required attributes. Five placeholders, bound from the
/// claimer (family, lane, pool, "provider/model", model id); an
/// attribute the claimer does not declare binds NULL and meets
/// no requirement — the same shape as the platform limitation's
/// family-less rule (#8217).
const TASK_REQUIREMENTS_CLAUSE: &str =
    "(json_extract(payload, '$.requires.platform') IS NULL \
        OR json_extract(payload, '$.requires.platform') = ?) \
     AND (json_extract(payload, '$.requires.lane') IS NULL \
        OR json_extract(payload, '$.requires.lane') = ?) \
     AND (json_extract(payload, '$.requires.pool') IS NULL \
        OR json_extract(payload, '$.requires.pool') = ?) \
     AND (json_extract(payload, '$.requires.model') IS NULL \
        OR json_extract(payload, '$.requires.model') = ? \
        OR json_extract(payload, '$.requires.model') = ?)";

/// Who is claiming (#8097 one-queue model): the identity every
/// claim declares and the broker records (the claim stamps —
/// worker, lane, platform family, pool, model). Identity is
/// accountability, not eligibility: it enters matching only as
/// the values a task's own declared requirements are checked
/// against (requirement_args).
#[derive(Debug, Clone, Default)]
pub struct ClaimerCtx {
    family: Option<String>,
    lane: Option<String>,
    worker: String,
    pool: Option<String>,
    model: Option<ClaimModel>,
}

impl ClaimerCtx {
    /// The five requirement bindings, in TASK_REQUIREMENTS_CLAUSE
    /// order: family, lane, pool, "provider/model", model id.
    fn requirement_args(&self) -> Vec<Arg> {
        let (full, bare) = match &self.model {
            Some(m) => (
                m.model.as_ref().map(|id| format!("{}/{}", m.provider, id)),
                m.model.clone(),
            ),
            None => (None, None),
        };
        vec![
            arg_str(self.family.as_deref()),
            arg_str(self.lane.as_deref()),
            arg_str(self.pool.as_deref()),
            arg_str(full.as_deref()),
            arg_str(bare.as_deref()),
        ]
    }
}

/// Whether a lanes-table registration binds its lane to one
/// stream under the one-queue model: any recorded queue value
/// binds EXCEPT the unbound markers ("*", "all", ""), which are
/// the registry's recorded one-queue default — the lane may
/// claim any stream, and unscoped claims never consult the
/// registration at all.
fn lane_queue_binds(registered: &str) -> bool {
    !matches!(registered, "*" | "all" | "")
}

/// A claim's full scope: the compiled claimer-side conditions
/// (the optional stream scope and filter expression) plus the
/// task-side requirements clause, with their arguments in SQL
/// placeholder order. The scope SQL is appended after the pick
/// predicate and the platform limitation clause in BOTH the
/// pick SELECT and the claim UPDATE re-check, so a claim can
/// never be handed a row its own scope excluded or whose
/// requirements it does not meet — the #7068 discipline,
/// extended to the union match.
#[derive(Clone)]
pub struct ClaimScope {
    sql: String,
    args: Vec<Arg>,
}

pub fn claim_scope(
    queue: Option<&str>,
    filter: Option<&FilterExpr>,
    claimer: &ClaimerCtx,
) -> ClaimScope {
    let mut parts: Vec<String> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(q) = queue {
        parts.push("queue = ?".to_string());
        args.push(Arg::Text(q.to_string()));
    }
    if let Some(f) = filter {
        parts.push(format!("({})", filter_sql(f, &mut args)));
    }
    parts.push(TASK_REQUIREMENTS_CLAUSE.to_string());
    args.extend(claimer.requirement_args());
    ClaimScope {
        sql: format!(" AND {}", parts.join(" AND ")),
        args,
    }
}

/// The task row's payload as a JSON object: query_objs returns
/// payload already parsed; a raw string is parsed here; anything
/// else reads as an empty object (no metadata, no requirements).
fn task_payload(row: &Value) -> Value {
    match row.get("payload") {
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    }
}

/// The task's repo identity (mirrors REPO_IDENTITY_SQL): the
/// payload's declared repo when present and non-empty, else the
/// stream name with the repo-health- prefix and -pilot / -shadow
/// suffixes stripped.
pub fn task_repo_identity(row: &Value) -> String {
    let payload = task_payload(row);
    if let Some(r) = payload.get("repo").and_then(|v| v.as_str()) {
        if !r.is_empty() {
            return r.to_string();
        }
    }
    let queue = row.get("queue").and_then(|v| v.as_str()).unwrap_or("");
    match queue.strip_prefix("repo-health-") {
        Some(rest) => rest.replace("-pilot", "").replace("-shadow", ""),
        None => queue.to_string(),
    }
}

fn repo_value_matches(identity: &str, value: &str) -> bool {
    identity == value || identity.ends_with(&format!("/{value}"))
}

fn filter_subject(field: FilterField, row: &Value, payload: &Value) -> Option<String> {
    let s = |v: Option<&Value>| v.and_then(|x| x.as_str()).map(str::to_string);
    match field {
        FilterField::Queue | FilterField::Pool => s(row.get("queue")),
        FilterField::Platform => s(row.get("platform")),
        FilterField::Kind => s(payload.get("kind")),
        FilterField::Lane => s(payload.get("requires").and_then(|r| r.get("lane"))),
        FilterField::Model => s(payload.get("requires").and_then(|r| r.get("model"))),
        FilterField::Repo | FilterField::Label | FilterField::Priority => None,
    }
}

fn cmp_matches(field: FilterField, op: CmpOp, values: &[String], row: &Value) -> bool {
    let payload = task_payload(row);
    match field {
        FilterField::Repo => {
            let identity = task_repo_identity(row);
            let any = values.iter().any(|v| repo_value_matches(&identity, v));
            match op {
                CmpOp::Eq | CmpOp::In => any,
                CmpOp::Ne | CmpOp::NotIn => !any,
                _ => false,
            }
        }
        FilterField::Label => {
            let labels: Vec<&str> = payload
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let any = values.iter().any(|v| labels.contains(&v.as_str()));
            match op {
                CmpOp::Eq | CmpOp::In => any,
                CmpOp::Ne | CmpOp::NotIn => !any,
                _ => false,
            }
        }
        FilterField::Priority => {
            let subj = row.get("priority").and_then(|v| v.as_i64());
            let nums: Vec<i64> = values.iter().filter_map(|v| v.parse().ok()).collect();
            match op {
                CmpOp::Eq => subj.is_some() && nums.iter().any(|n| Some(*n) == subj),
                CmpOp::Ne => subj.map_or(true, |s| nums.iter().all(|n| *n != s)),
                CmpOp::Lt => matches!((subj, nums.first()), (Some(s), Some(n)) if s < *n),
                CmpOp::Le => matches!((subj, nums.first()), (Some(s), Some(n)) if s <= *n),
                CmpOp::Gt => matches!((subj, nums.first()), (Some(s), Some(n)) if s > *n),
                CmpOp::Ge => matches!((subj, nums.first()), (Some(s), Some(n)) if s >= *n),
                CmpOp::In => subj.is_some() && nums.contains(&subj.unwrap_or(0)),
                CmpOp::NotIn => subj.map_or(true, |s| !nums.contains(&s)),
            }
        }
        field => {
            let subj = filter_subject(field, row, &payload);
            match op {
                CmpOp::Eq => subj.is_some() && values.iter().any(|v| subj.as_deref() == Some(v)),
                CmpOp::Ne => subj
                    .as_deref()
                    .map_or(true, |s| values.iter().all(|v| v != s)),
                CmpOp::In => subj.is_some() && values.iter().any(|v| subj.as_deref() == Some(v)),
                CmpOp::NotIn => subj
                    .as_deref()
                    .map_or(true, |s| values.iter().all(|v| v != s)),
                _ => false,
            }
        }
    }
}

/// Evaluate a filter expression against a task row, in memory —
/// the pure mirror of filter_sql, used by the tests and by any
/// caller that already holds the row. Semantics match the SQL
/// exactly, including the absent-value rules.
pub fn filter_matches(expr: &FilterExpr, row: &Value) -> bool {
    match expr {
        FilterExpr::Or(a, b) => filter_matches(a, row) || filter_matches(b, row),
        FilterExpr::And(a, b) => filter_matches(a, row) && filter_matches(b, row),
        FilterExpr::Not(a) => !filter_matches(a, row),
        FilterExpr::Cmp { field, op, values } => cmp_matches(*field, *op, values, row),
    }
}

/// The task-side half of the union match, evaluated in memory:
/// Some(reason) when the task's payload `requires` object names
/// an attribute this claimer does not declare with the required
/// value; None when the task declares no requirements or the
/// claimer meets them all. This is the ONLY place claimer
/// identity shapes eligibility — a task's own declared
/// requirements, honored automatically inside the claim.
pub fn task_requirements_unmet(row: &Value, claimer: &ClaimerCtx) -> Option<String> {
    let payload = task_payload(row);
    let req = payload.get("requires")?.as_object()?;
    if let Some(p) = req.get("platform").and_then(|v| v.as_str()) {
        if claimer.family.as_deref() != Some(p) {
            return Some(format!("task requires platform {p}"));
        }
    }
    if let Some(l) = req.get("lane").and_then(|v| v.as_str()) {
        if claimer.lane.as_deref() != Some(l) {
            return Some(format!("task requires lane {l}"));
        }
    }
    if let Some(p) = req.get("pool").and_then(|v| v.as_str()) {
        if claimer.pool.as_deref() != Some(p) {
            return Some(format!("task requires pool {p}"));
        }
    }
    if let Some(m) = req.get("model").and_then(|v| v.as_str()) {
        let met = match &claimer.model {
            Some(cm) => {
                cm.model.as_deref() == Some(m)
                    || cm
                        .model
                        .as_ref()
                        .map(|id| format!("{}/{}", cm.provider, id))
                        .as_deref()
                        == Some(m)
            }
            None => false,
        };
        if !met {
            return Some(format!("task requires model {m}"));
        }
    }
    None
}

/// The recorded platform limitation, expressed for the claim
/// pick (#8217, extending #8210's transfer-claim enforcement to
/// every pick): a row is unservable while its lane-ledger row
/// carries a VALID limitation — platform_limit and
/// platform_limit_reason both recorded (a reason-less stored
/// limitation is invalid: cleared, never enforced, exactly as
/// platform_limit_refusal treats it) — naming a family other
/// than the claim's declared family. A claim declaring no family
/// is served only unlimited rows. The two placeholders are the
/// claim's family, twice; callers append them after the pick
/// predicate's own arguments, in both the pick SELECT and the
/// claim UPDATE re-check. Semantics mirror
/// platform_limit_refusal's truth table (pinned by
/// platform_limit_pick_semantics_8217).
const PLATFORM_LIMIT_PICK_CLAUSE: &str = "AND NOT EXISTS (SELECT 1 FROM \
    repo_task_lane_ledger l WHERE l.task_id = tasks.id \
      AND l.platform_limit IS NOT NULL AND l.platform_limit <> '' \
      AND l.platform_limit_reason IS NOT NULL \
      AND l.platform_limit_reason <> '' \
      AND (? IS NULL OR l.platform_limit <> ?))";

fn claim_update_sql(pick_predicate: &str) -> String {
    format!(
        "UPDATE tasks SET status = 'claimed', worker_id = ?, claimed_at = ?, \
           lease_until = ?, \
           attempts = attempts + (CASE WHEN status IN ('parked', 'handed_off') THEN 0 ELSE 1 END), \
           lane_id = ?, platform = COALESCE(?, platform), \
           park_kind = NULL, wait_kind = NULL, not_before = NULL, \
           updated_at = ? \
         WHERE id = ? \
           AND ( {pick_predicate} ) {PLATFORM_LIMIT_PICK_CLAUSE}"
    )
}

/// The default (lane) pick predicate (#8184, amended #8187,
/// #8234): queued rows due under the attempts cap, or parked rows
/// whose wait timer has passed — plus, as a third branch, handoff
/// sink rows (status 'handed_off', the #8185 sink with its own
/// status since #8234) with no timer at all: the sink is not a
/// wait. #8184 excluded sink rows from lanes entirely (they were
/// claimant-owned, and serving one to a lane only re-ran the
/// claim -> handoff -> park cycle); #8187 reverses that: a lane is
/// single-session by default, claims a sink row at +0 attempts
/// (per the claim UPDATE's parked/handed_off CASE), and verb_lane
/// flips its ledger assignment to direct when the lane passes
/// --exclusion-reason (see lane_reassignment_flip). The claimant's
/// handoff pick and by-id claim keep working unchanged. Since
/// #8234 a sink row is never 'parked', so the timed parked branch
/// needs no sink exclusion — genuine wait parks release through
/// claim at not_before, and only there. The sink branch carries
/// no `?` placeholder, so the pick SELECT and the claim UPDATE
/// re-check keep their two `now` arguments.
fn default_pick_predicate() -> String {
    format!(
        "(status = 'queued' AND attempts < max_attempts \
         AND (not_before IS NULL OR not_before <= ?)) \
       OR (status = 'parked' AND not_before IS NOT NULL AND not_before <= ?) \
       OR ({HANDOFF_SQL})"
    )
}

/// The lane a FRESH ledger assignment gets (#8187): `direct` by
/// default — the claiming session owns the task, and no task is
/// routed to opencode (or any specific lane) without a specific
/// reason recorded at assignment time. The only doors to an
/// opencode assignment are an explicit `--opencode-reason` from
/// the caller (recorded on the ledger row) — or, symmetrically, a
/// caller `--exclusion-reason` forces direct. Exclusion wins when
/// both are passed: it is the older, stronger contract ("this task
/// must NOT go to opencode"). Returns (lane, reason) where reason
/// is the value recorded on the lane_assigned event:
/// "default_direct" | "exclusion" | "opencode_reason".
///
/// tasks.platform plays NO part here, ever (#8187 amendment):
/// a task's platform stamp is a MARK of which platform is looking
/// at it (PLATFORM_FAMILIES: "openchamber" | "muse" — a family,
/// never a specific lane), never a routing reason and never an
/// eligibility condition. The ONLY door to opencode is the
/// explicit --opencode-reason above.
fn lane_decision(
    exclusion: Option<&str>,
    opencode_reason: Option<&str>,
) -> (&'static str, &'static str) {
    if exclusion.is_some() {
        ("direct", "exclusion")
    } else if opencode_reason.is_some() {
        ("opencode", "opencode_reason")
    } else {
        ("direct", "default_direct")
    }
}

/// Whether an EXISTING ledger row flips to direct (#8187): only an
/// opencode row, only when the caller passes --exclusion-reason
/// (a Muse lane taking a sink/legacy opencode task direct), and
/// never once a session is bound (session_id set) — a bound row's
/// assignment is history, not a routing decision. Claimant flows
/// that pass no exclusion keep their existing assignment.
fn lane_reassignment_flip(
    assigned_lane: &str,
    session_id: Option<&str>,
    exclusion: Option<&str>,
) -> bool {
    assigned_lane == "opencode"
        && exclusion.is_some()
        && session_id.map(str::trim).unwrap_or("").is_empty()
}

/// Whether an EXISTING direct ledger row flips to opencode
/// (#8210): only a direct row, only when the caller passes
/// --opencode-reason (a Muse session sending its task to
/// opencode with a stated reason), and never once a session is
/// bound — the mirror of lane_reassignment_flip. Before #8210 the
/// existing-row path of verb_lane only flipped opencode -> direct,
/// so a lane that had claimed a task (fresh direct ledger row)
/// had no way to record an opencode assignment on it at all.
fn lane_flip_to_opencode(
    assigned_lane: &str,
    session_id: Option<&str>,
    opencode_reason: Option<&str>,
) -> bool {
    assigned_lane == "direct"
        && !opencode_reason.map(str::trim).unwrap_or("").is_empty()
        && session_id.map(str::trim).unwrap_or("").is_empty()
}

/// Platform limitation validity (#8210 — Drew: a task may be
/// limited to muse or opencode "but better be good reason"). A
/// limitation is an EXCEPTION beside the open default: it exists
/// only with a reason recorded next to it. A stored limitation
/// whose reason is blank is invalid — never enforced; the
/// transfer path clears it and records the clearing.
fn platform_limit_invalid(limit: Option<&str>, reason: Option<&str>) -> bool {
    !limit.map(str::trim).unwrap_or("").is_empty()
        && reason.map(str::trim).unwrap_or("").is_empty()
}

/// Why a requested limitation cannot be recorded, or None when
/// it can: the family must be a real platform family (#8187's
/// vocabulary), and the reason must be present and non-blank —
/// the lane verb refuses the set otherwise. A reason without a
/// limitation is likewise refused (it would record nothing).
fn platform_limit_error(
    limit: Option<&str>,
    reason: Option<&str>,
) -> Option<String> {
    let l = limit.map(str::trim).unwrap_or("");
    let r = reason.map(str::trim).unwrap_or("");
    if l.is_empty() {
        if !r.is_empty() {
            return Some(
                "platform_limit_reason requires a platform_limit to limit"
                    .to_string(),
            );
        }
        return None;
    }
    if let Err(e) = parse_platform(Some(l)) {
        return Some(e);
    }
    if r.is_empty() {
        return Some(format!(
            "a platform limitation requires a recorded reason \
             (--limit-reason): a task is limited to '{l}' only for \
             a good reason, recorded on the task"
        ));
    }
    None
}

/// Transfer refusal for a platform limitation (#8210), or None
/// when the limitation does not bind this claim: no limitation,
/// an invalid (reason-less) limitation — those are cleared, not
/// enforced — or a limitation naming the claimer's own family.
/// A valid limitation refuses every other family, and refuses a
/// claim that declares no family at all (it cannot show it is
/// the allowed one). The refusal names the limitation and its
/// recorded reason.
fn platform_limit_refusal(
    limit: Option<&str>,
    reason: Option<&str>,
    family: Option<&str>,
) -> Option<String> {
    let l = limit.map(str::trim).unwrap_or("");
    if l.is_empty() || platform_limit_invalid(limit, reason) {
        return None;
    }
    let r = reason.map(str::trim).unwrap_or("");
    match family {
        Some(f) if f == l => None,
        Some(f) => Some(format!(
            "task is limited to platform '{l}' (reason: {r}); \
             a '{f}' transfer-claim cannot take it"
        )),
        None => Some(format!(
            "task is limited to platform '{l}' (reason: {r}); \
             the transfer-claim must declare its platform family to take it"
        )),
    }
}

/// Session-to-session transfer refusal (#8210), or None when the
/// transfer may proceed. Pure over the gathered marks so the
/// tests pin every guard without a live store; claim_transfer
/// runs this BEFORE any mutation, so a refusal leaves the
/// original claim untouched. Guards, in order:
/// - the row must be claimed under a live lease — terminal rows,
///   queued rows and parked rows (genuine broker wait-parks stay
///   excluded from by-id taking, exactly as by_id_claimable has
///   them) are not transfer sources;
/// - from_session must match a holder session mark: the bound
///   progress.session_id, a progress.from_session the holder
///   checkpointed, or the session on the task's latest claimed
///   event — only the current holder can give a task away;
/// - a row already bound to a session that is neither the passer
///   nor the claiming session refuses: that binding is a live
///   launch, not a pass.
fn transfer_refusal(
    row: &Value,
    from_session: &str,
    claimed_event_session: Option<&str>,
    ledger_session: Option<&str>,
    new_session: Option<&str>,
    now: &str,
) -> Option<String> {
    let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "claimed" {
        return Some(format!(
            "task is {status}, not claimed: only a live claim can be transferred"
        ));
    }
    let lease = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if lease <= now {
        return Some(
            "the holder's lease is not live: no live claim to transfer".to_string(),
        );
    }
    if from_session.trim().is_empty() {
        return Some("from_session is empty".to_string());
    }
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let bound = progress
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let offered = progress
        .get("from_session")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let marks = [bound, offered, claimed_event_session.unwrap_or("")];
    if !marks.iter().any(|m| !m.is_empty() && *m == from_session) {
        return Some(
            "from_session does not match the current holder's session mark: \
             only the current holder can give this task away"
                .to_string(),
        );
    }
    for existing in [bound, ledger_session.unwrap_or("")] {
        if !existing.is_empty() && existing != from_session && Some(existing) != new_session {
            return Some(format!(
                "task is already bound to session {existing}: \
                 a bound launch is not a transfer"
            ));
        }
    }
    None
}

/// The transfer UPDATE (#8210): one conditional statement moves
/// a live claim holder -> claimer. Status stays 'claimed',
/// attempts are NOT touched (a transfer is not a new attempt),
/// and the WHERE re-checks the status and the HOLDER worker read
/// above, so a competing transition in between loses cleanly
/// (0 rows -> refusal, original claim intact).
fn transfer_update_sql() -> &'static str {
    "UPDATE tasks SET worker_id = ?, claimed_at = ?, lease_until = ?, \
       lane_id = ?, platform = COALESCE(?, platform), \
       park_kind = NULL, wait_kind = NULL, not_before = NULL, updated_at = ? \
     WHERE id = ? AND status = 'claimed' AND worker_id = ?"
}

/// The claim preamble (#8217: shared by the single claim and
/// the batch claim): validate the caller's lane against the
/// registry, sweep expired leases, and quarantine payload-less
/// rows in this queue before any pick runs.
fn claim_preamble(
    t: &TursoBackend,
    queue: Option<&str>,
    worker: &str,
    lane: &Option<String>,
) -> Result<(), TErr> {
    if let Some(lane_id) = &lane {
        let lanes = t.query_objs(
            "SELECT lane_id, worker_id, queue, enabled FROM lanes WHERE lane_id = ?",
            &[Arg::Text(lane_id.clone())],
            &[],
        )?;
        let Some(l) = lanes.into_iter().next() else {
            return Err(format!("claim_repo_task: unknown lane_id {lane_id}"));
        };
        if l.get("worker_id").and_then(|v| v.as_str()) != Some(worker) {
            return Err(format!(
                "claim_repo_task: lane {lane_id} belongs to worker {}, not {worker}",
                l.get("worker_id").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
        if l.get("enabled").and_then(|v| v.as_i64()) == Some(0) {
            return Err(format!("claim_repo_task: lane {lane_id} is disabled"));
        }
        // One-queue model (#8097, 2026-11-10): the registration's
        // queue binding is enforced ONLY when the claim itself is
        // stream-scoped AND the registration binds a stream. An
        // unscoped claim (the default) is never refused over the
        // lane's registration — identity is recorded, never used
        // to filter — and a registration carrying an unbound
        // marker ("*", see lane_queue_binds) binds nothing.
        if let Some(q) = queue {
            let registered = l.get("queue").and_then(|v| v.as_str()).unwrap_or("");
            if lane_queue_binds(registered) && registered != q {
                return Err(format!(
                    "claim_repo_task: lane {lane_id} is for queue {registered}, not {q}"
                ));
            }
        }
    }
    expire_leases(t)?;
    match queue {
        Some(q) => quarantine_payload_missing(t, q)?,
        None => quarantine_payload_missing_all(t)?,
    }
    Ok(())
}

fn claim_pick_once(
    t: &TursoBackend,
    scope: &ClaimScope,
    worker: String,
    lane: &Option<String>,
    lease_seconds: i64,
    family: Option<String>,
    claim_session: Option<String>,
    model: &Option<ClaimModel>,
    handoff_pick: bool,
) -> Result<Option<Value>, TErr> {
    let now_unix = now_unix();
    let now = iso(now_unix);
    let lease_until = claim_lease_until(now_unix, lease_seconds);
    // One conditional UPDATE = the claim. The subquery picks the
    // same row PG's SKIP LOCKED pick would (priority, then age;
    // parked rows whose not_before has passed resume without an
    // attempts increment); the outer status re-check keeps the
    // statement atomic against a competing claim.
    // #5444: the pick loops past rows the guard quarantines —
    // a skipped row degrades to the next candidate or to no
    // claim, never to a handed-out invalid packet.
    let mut row: Value;
    let mut id: i64;
    let mut skipped = 0usize;
    loop {
        // #7617: pick the candidate id FIRST, then claim THAT row by
        // id, and read it back by id. The previous shape claimed via
        // a subquery and re-read the result as "the newest claimed
        // row for (queue, worker_id)" — but every lane shares
        // worker_id repo-worker-code, so under concurrent claims
        // that re-read could return ANOTHER lane's just-claimed
        // task: the caller was handed a row stamped with a foreign
        // lane_id (live repro 2026-11-08: lane code-u claims
        // returned tasks stamped code-e and code-t), it checkpointed
        // and parked a task it did not own, and the guard below
        // could quarantine a live foreign claim. The conditional
        // UPDATE by id stays atomic: it re-checks the full pick
        // eligibility (status, attempts cap, not_before) at claim
        // instant, so if a competing claim takes the picked row
        // first — or it is parked into the future in between — this
        // UPDATE affects 0 rows and the loop re-picks.
        // #8097: the pick predicate is built per pick mode.
        // #8187: NEITHER mode filters by platform — the mark a
        // claim declares (family) is stamped by the claim
        // UPDATE, never matched against the item. The default mode keeps the ordinary predicate
        // (queued rows due; parked rows whose timer has passed;
        // sink rows by their own branch).
        // Handoff mode picks the next sink row (HANDOFF_SQL:
        // status 'handed_off', #8234) with no caller-named id
        // and no timer gating — Rule 1's default collection
        // path for the claimant.
        let pick_predicate = if handoff_pick {
            HANDOFF_SQL.to_string()
        } else {
            default_pick_predicate()
        };
        let pick_sql = format!(
            "SELECT id FROM tasks WHERE 1=1 \
           AND payload IS NOT NULL AND payload <> '' AND payload <> '{{}}' \
           AND ( {pick_predicate} ) {PLATFORM_LIMIT_PICK_CLAUSE}{} \
         ORDER BY (priority IS NULL), priority ASC, created_at ASC LIMIT 1",
            scope.sql
        );
        let mut pick_args: Vec<Arg> = Vec::new();
        if !handoff_pick {
            pick_args.push(Arg::Text(now.clone()));
            pick_args.push(Arg::Text(now.clone()));
        }
        pick_args.push(arg_str(family.as_deref()));
        pick_args.push(arg_str(family.as_deref()));
        for a in &scope.args {
            pick_args.push(a.clone());
        }
        let pick = t.query_objs(&pick_sql, &pick_args, &[])?;
        let Some(picked) = pick.into_iter().next() else {
            return Ok(None);
        };
        let pick_id = picked.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let upd_sql = format!("{}{}", claim_update_sql(&pick_predicate), scope.sql);
        let mut upd_args: Vec<Arg> = vec![
            Arg::Text(worker.clone()),
            Arg::Text(now.clone()),
            Arg::Text(lease_until.clone()),
            arg_str(lane.as_deref()),
            arg_str(family.as_deref()),
            Arg::Text(now.clone()),
            Arg::Int(pick_id),
        ];
        if !handoff_pick {
            upd_args.push(Arg::Text(now.clone()));
            upd_args.push(Arg::Text(now.clone()));
        }
            upd_args.push(arg_str(family.as_deref()));
            upd_args.push(arg_str(family.as_deref()));
        for a in &scope.args {
            upd_args.push(a.clone());
        }
        let upd = t.exec(&upd_sql, &upd_args)?;
        if upd.affected == 0 {
            // Lost the race for the picked row; re-pick.
            skipped += 1;
            if skipped >= 32 {
                return Ok(None);
            }
            continue;
        }
        let rows = t.query_objs(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
            &[Arg::Int(pick_id)],
            &TASK_JSON_COLS,
        )?;
        let Some(candidate) = rows.into_iter().next() else {
            return Ok(None);
        };
        if let Some(reason) = claim_guard_reason(&candidate) {
            let cid = candidate.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            quarantine_task(t, cid, &reason)?;
            skipped += 1;
            if skipped >= 32 {
                return Ok(None);
            }
            continue;
        }
        id = candidate.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        row = candidate;
        break;
    }
    let mut row = row;
    // Migration 0023 §4: a fresh claim never inherits a binding.
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let (progress, cleared) = clear_binding(&progress, "claim_repo_task");
    if cleared.is_some() {
        t.exec(
            "UPDATE tasks SET progress = ? WHERE id = ?",
            &[Arg::Text(progress.to_string()), Arg::Int(id)],
        )?;
        if let Some(obj) = row.as_object_mut() {
            obj.insert("progress".to_string(), progress);
        }
    }
    t.exec(
        "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id, \
           lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at",
        &[
            Arg::Int(id),
            Arg::Text(worker.clone()),
            Arg::Text(lease_until),
            Arg::Text(now_iso()),
        ],
    )?;
    stamp_claim_model(t, id, &mut row, model)?;
    touch_worker(t, &worker, Some(id))?;
    if let Some(old) = &cleared {
        on_binding_cleared(t, &worker, id, old, "claim_repo_task")?;
    }
    event(
        t,
        Some(&worker),
        Some(id),
        "claimed",
        &with_marks(
            &json!({
                "lease_seconds": lease_seconds, "via": "claim_repo_task_turso",
                "cleared_session_id": cleared, "lane_id": lane,
                "platform": family, "session_id": claim_session,
                "model_provider": model.as_ref().map(|m| m.provider.clone()),
                "model_id": model.as_ref().and_then(|m| m.model.clone()),
                "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
                "pick": if handoff_pick { "handed_off" } else { "default" },
            }),
            &row,
        ),
    )?;
    Ok(Some(row))
}

/// The most tasks one batch claim may take (#8217). A batch is
/// only legitimate paired with immediate per-item dispatch, and
/// the dispatcher must launch + verify-bind every item inside one
/// touch before leases age: five dedicated launches fit that
/// window with margin, and a bigger batch would recreate the
/// claimed-and-idle pile the single-session design forbids.
pub const BATCH_CLAIM_MAX: i64 = 5;

/// Parse a claim request's batch count (#8217): absent means the
/// single claim (1); anything outside 1..=BATCH_CLAIM_MAX is a
/// usage error, never a silent clamp.
fn parse_batch_count(req: &Value) -> Result<i64, String> {
    match req_i64(req, "count") {
        None => Ok(1),
        Some(n) if (1..=BATCH_CLAIM_MAX).contains(&n) => Ok(n),
        Some(n) => Err(format!(
            "count {n} is out of range: a batch claim takes              1..={BATCH_CLAIM_MAX} items (#8217)"
        )),
    }
}

/// Batch claim (#8217): the claim preamble once, then the
/// identical atomic pick up to `count` times — same eligibility,
/// same priority/age order, same attempts/lease semantics and
/// per-item claimed events as that many single claims. A partial
/// batch is normal: the loop stops at the first no-task pick and
/// returns what exists.
#[allow(clippy::too_many_arguments)]
fn verb_claim_batch(
    t: &TursoBackend,
    queue: Option<&str>,
    scope: &ClaimScope,
    worker: &str,
    lane: &Option<String>,
    lease_seconds: i64,
    family: &Option<String>,
    claim_session: &Option<String>,
    model: &Option<ClaimModel>,
    handoff_pick: bool,
    count: i64,
) -> Value {
    let work = (|| -> Result<Vec<Value>, TErr> {
        claim_preamble(t, queue, worker, lane)?;
        let mut rows = Vec::new();
        for _ in 0..count {
            let Some(row) = claim_pick_once(
                t,
                scope,
                worker.to_string(),
                lane,
                lease_seconds,
                family.clone(),
                claim_session.clone(),
                model,
                handoff_pick,
            )?
            else {
                break;
            };
            rows.push(row);
        }
        Ok(rows)
    })();
    match work {
        Ok(rows) => {
            let claimed = rows.len();
            ok(json!({
                "rows": rows,
                "claimed": claimed,
                "requested": count,
            }))
        }
        Err(e) if e.starts_with("claim_repo_task:") => fail(4, e),
        Err(e) => terr(e),
    }
}

fn verb_claim(t: &TursoBackend, req: &Value) -> Value {
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    // One-queue model (#8097, 2026-11-10): the request's queue
    // is OPTIONAL. Absent, empty, "*" or "all" names no stream:
    // the claim spans the one fleet queue in priority order. A
    // named stream still scopes the claim exactly as before, so
    // every existing scoped caller keeps working unchanged.
    let queue: Option<String> = match req_str(req, "queue") {
        None | Some("") | Some("*") | Some("all") => None,
        Some(q) => Some(q.to_string()),
    };
    // The claimer-side filter expression (--filter): a boolean
    // expression over task metadata, evaluated inside the claim
    // (see parse_filter). Absent is the default: no filtering.
    let filter_expr = match req_str(req, "filter") {
        None | Some("") => None,
        Some(f) => match parse_filter(f) {
            Ok(e) => Some(e),
            Err(e) => return fail(2, format!("claim_repo_task: bad filter: {e}")),
        },
    };
    // The claimer's declared pool identity (--pool): recorded
    // like the other identity marks and used only to satisfy a
    // task's own requires.pool, never as a filter.
    let pool = req_str(req, "pool").map(str::to_string);
    let lease_seconds = req_i64(req, "lease_seconds").unwrap_or(CLAIM_DEFAULT_LEASE_SECONDS);
    let lane = req_str(req, "lane").map(str::to_string);
    // #8187 (demoting #8097 Rule 2): the claim's declared platform
    // is a MARK, never a filter — it is stamped on the claimed row
    // (platform = COALESCE(declared, existing)) and carried on the
    // claimed event, and it excludes NOTHING: a claim whose
    // platform mismatches the item's mark claims it all the same,
    // and a claim declaring no platform works identically.
    let family = match parse_platform(req_str(req, "platform")) {
        Ok(f) => f,
        Err(e) => return fail(2, format!("claim_repo_task: {e}")),
    };
    // The claiming session's own id, when the surface knows it:
    // another mark for the claimed event, never required.
    let claim_session = req_str(req, "session_id").map(str::to_string);
    // #8176: the claiming session's own model, stamped on the row
    // the claim returns (claim_provider / claim_model /
    // claim_variant). Absent fields stamp nothing.
    let model = match parse_claim_model(req) {
        Ok(m) => m,
        Err(e) => return fail(2, format!("claim_repo_task: {e}")),
    };
    // #8097 Rule 1: pick mode. The default pick is the queue's next
    // suitable item. "handed_off" restricts the pick to recorded
    // opencode handoffs in the sink (status 'handed_off', #8234) —
    // the broker chooses the row, the caller names NO task id.
    // Naming a task (task_id, below) stays the explicit special
    // case. ("parked_handoff" is the pre-#8234 spelling of the
    // mode, kept as a deprecated alias so a straggler caller fails
    // soft; the sink has not been a park since #8234.)
    let handoff_pick = match req_str(req, "pick") {
        None => false,
        Some("handed_off") | Some("parked_handoff") => true,
        Some(other) => {
            return fail(2, format!("claim_repo_task: unknown pick mode {other:?}"));
        }
    };
    // #8217: batch claim. `count` absent or 1 is the single claim
    // below, byte for byte. count > 1 runs the same pick that many
    // times (verb_claim_batch) and returns every claimed row; it
    // never combines with a named task (by-id and transfer claims
    // take exactly one row).
    // Union matching (#8097 one-queue model): the claim's
    // scope compiles the claimer's requirements (stream +
    // filter) together with the task-side requirements clause;
    // both are evaluated inside the pick and its UPDATE
    // re-check. The claimer context is this claim's identity —
    // recorded on the claim, and used only to satisfy tasks'
    // own declared requirements.
    let claimer = ClaimerCtx {
        family: family.clone(),
        lane: lane.clone(),
        worker: worker.clone(),
        pool: pool.clone(),
        model: model.clone(),
    };
    let scope = claim_scope(queue.as_deref(), filter_expr.as_ref(), &claimer);
    let count = match parse_batch_count(req) {
        Ok(c) => c,
        Err(e) => return fail(2, format!("claim_repo_task: {e}")),
    };
    if count > 1 {
        if req_i64(req, "task_id").is_some() {
            return fail(
                2,
                "claim_repo_task: --count cannot be combined with --task: \
                 a by-id claim takes exactly one task (#8217)"
                    .to_string(),
            );
        }
        return verb_claim_batch(
            t,
            queue.as_deref(),
            &scope,
            &worker,
            &lane,
            lease_seconds,
            &family,
            &claim_session,
            &model,
            handoff_pick,
            count,
        );
    }

    let work = (|| -> Result<Option<Value>, TErr> {
        claim_preamble(t, queue.as_deref(), &worker, &lane)?;
        // By-id claim — the SPECIAL CASE (#8097 Rule 1): when the
        // request names a task, take exactly that row (claim_by_id)
        // instead of the pick. Reserved for one-off revivals of a
        // specific row (e.g. a single sink row); the default flow
        // never names a task — the claimant collects sink rows
        // through the pick="handed_off" mode above.
        if let Some(want_id) = req_i64(req, "task_id") {
            // #8210: task_id + from_session is a session-to-session
            // TRANSFER — take the row directly from its current
            // holder, never through queued or parked. (A bare
            // task_id stays the by-id special case below.)
            if let Some(from_session) = req_str(req, "from_session") {
                return claim_transfer(
                    t,
                    want_id,
                    queue.as_deref(),
                    &worker,
                    &lane,
                    lease_seconds,
                    family.as_deref(),
                    &model,
                    from_session,
                    claim_session.as_deref(),
                    &pool,
                );
            }
            return claim_by_id(
                t,
                want_id,
                queue.as_deref(),
                &worker,
                &lane,
                lease_seconds,
                family.as_deref(),
                &model,
                &pool,
            );
        }
        claim_pick_once(
            t,
            &scope,
            worker.clone(),
            &lane,
            lease_seconds,
            family,
            claim_session,
            &model,
            handoff_pick,
        )
    })();
    match work {
        Ok(Some(row)) => ok(json!({"row": row})),
        Ok(None) => ok(json!({"row": Value::Null})),
        Err(e) if e.starts_with("claim_repo_task:") => fail(4, e),
        Err(e) => terr(e),
    }
}

fn verb_lane(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let exclusion = req_str(req, "exclusion_reason").map(str::to_string);
    let opencode_reason = req_str(req, "opencode_reason").map(str::to_string);
    let limit_platform = req_str(req, "platform_limit").map(str::to_string);
    let limit_reason = req_str(req, "platform_limit_reason").map(str::to_string);
    let clear_limit = req
        .get("clear_platform_limit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let work = (|| -> Result<Value, TErr> {
        let Some(task) = get_task(t, task_id)? else {
            return Err(format!(
                "assign_repo_task_lane: task {task_id} does not exist"
            ));
        };
        // #8210: a platform limitation is recorded only with its
        // reason — validate before anything mutates.
        if !clear_limit {
            if let Some(e) = platform_limit_error(
                limit_platform.as_deref(),
                limit_reason.as_deref(),
            ) {
                return Err(format!("assign_repo_task_lane: {e}"));
            }
        }
        // Apply a limitation set/clear to the ledger row (called
        // once the row exists, on either path). Records the
        // limitation + reason (or the clearing) on a
        // lane_assigned event, like every assignment change.
        let apply_limit = |t: &TursoBackend| -> Result<(), TErr> {
            if clear_limit {
                t.exec(
                    "UPDATE repo_task_lane_ledger SET platform_limit = NULL, \
                       platform_limit_reason = NULL, updated_at = ? \
                     WHERE task_id = ?",
                    &[Arg::Text(now_iso()), Arg::Int(task_id)],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "platform_limit": Value::Null,
                            "platform_limit_reason": Value::Null,
                            "platform_limit_cleared": true,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
            } else if let Some(l) = limit_platform.as_deref() {
                let fam = parse_platform(Some(l)).ok().flatten();
                t.exec(
                    "UPDATE repo_task_lane_ledger SET platform_limit = ?, \
                       platform_limit_reason = ?, updated_at = ? \
                     WHERE task_id = ?",
                    &[
                        arg_str(fam.as_deref()),
                        arg_str(limit_reason.as_deref()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "platform_limit": fam,
                            "platform_limit_reason": limit_reason,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
            }
            Ok(())
        };
        if let Some(existing) = get_ledger(t, task_id)? {
            // Idempotent (migration 0021): keep progress.lane in step.
            let mut lane = existing
                .get("assigned_lane")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // #8187 reassignment: an unbound opencode row flips to
            // direct when the caller passes --exclusion-reason (a
            // Muse lane taking a sink/legacy opencode task direct).
            // The progress.lane re-sync below then stamps direct.
            if lane_reassignment_flip(
                &lane,
                existing.get("session_id").and_then(|v| v.as_str()),
                exclusion.as_deref(),
            ) {
                t.exec(
                    "UPDATE repo_task_lane_ledger SET assigned_lane = 'direct', \
                       exclusion_reason = ?, updated_at = ? WHERE task_id = ?",
                    &[
                        arg_str(exclusion.as_deref()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "assigned_lane": "direct", "previous_lane": "opencode",
                            "exclusion_reason": exclusion, "opencode_reason": Value::Null,
                            "reason": "exclusion", "reassignment": true,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
                lane = "direct".to_string();
            }
            // #8210: the symmetric flip — an existing DIRECT row
            // flips to opencode when the caller passes
            // --opencode-reason (a Muse session sending its
            // task to opencode with a stated reason) and no
            // session is bound. The progress.lane re-sync below
            // then stamps opencode.
            if lane_flip_to_opencode(
                &lane,
                existing.get("session_id").and_then(|v| v.as_str()),
                opencode_reason.as_deref(),
            ) {
                t.exec(
                    "UPDATE repo_task_lane_ledger SET assigned_lane = 'opencode', \
                       opencode_reason = ?, exclusion_reason = NULL, updated_at = ? \
                     WHERE task_id = ?",
                    &[
                        arg_str(opencode_reason.as_deref()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "assigned_lane": "opencode", "previous_lane": "direct",
                            "exclusion_reason": Value::Null,
                            "opencode_reason": opencode_reason,
                            "reason": "opencode_reason", "reassignment": true,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
                lane = "opencode".to_string();
            }
            let cur = task
                .get("progress")
                .and_then(|p| p.get("lane"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if cur != lane {
                let progress = merge_obj(
                    &task.get("progress").cloned().unwrap_or(json!({})),
                    &json!({"lane": lane}),
                );
                t.exec(
                    "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
                    &[
                        Arg::Text(progress.to_string()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
            }
            apply_limit(t)?;
            return Ok(get_ledger(t, task_id)?.unwrap_or(existing));
        }
        let queue = task
            .get("queue")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // #8187: no balancing counts — direct by default; opencode
        // only for a specific reason recorded at assignment time.
        let (lane, reason) = lane_decision(exclusion.as_deref(), opencode_reason.as_deref());
        let now = now_iso();
        t.exec(
            "INSERT INTO repo_task_lane_ledger \
             (queue, task_id, assigned_lane, exclusion_reason, opencode_reason, assigned_by, assigned_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                Arg::Text(queue),
                Arg::Int(task_id),
                Arg::Text(lane.to_string()),
                arg_str(exclusion.as_deref()),
                arg_str(opencode_reason.as_deref()),
                Arg::Text(worker.clone()),
                Arg::Text(now.clone()),
                Arg::Text(now),
            ],
        )?;
        let progress = merge_obj(
            &task.get("progress").cloned().unwrap_or(json!({})),
            &json!({"lane": lane}),
        );
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "lane_assigned",
            &with_marks(
                &json!({
                    "assigned_lane": lane, "exclusion_reason": exclusion,
                    "opencode_reason": opencode_reason, "reason": reason,
                    "via": "assign_repo_task_lane_turso",
                }),
                &task,
            ),
        )?;
        apply_limit(t)?;
        get_ledger(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) if e.starts_with("assign_repo_task_lane:") => fail(4, e),
        Err(e) => terr(e),
    }
}

fn verb_bind(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let session_id = req_str(req, "session_id").unwrap_or("").to_string();
    let launched_by = req_str(req, "launched_by")
        .unwrap_or("opencode-claimant")
        .to_string();
    let work = (|| -> Result<Value, TErr> {
        let Some(task) = get_task(t, task_id)? else {
            return Err(format!(
                "bind_repo_task_lane_session: task {task_id} does not exist"
            ));
        };
        let now = now_iso();
        let live = task.get("status").and_then(|v| v.as_str()) == Some("claimed")
            && task
                .get("lease_until")
                .and_then(|v| v.as_str())
                .map(|l| l > now.as_str())
                .unwrap_or(false);
        if !live {
            return Err(format!(
                "bind_repo_task_lane_session: task {task_id} is not claimed with a live lease (status {}, lease_until {})",
                task.get("status").and_then(|v| v.as_str()).unwrap_or(""),
                task.get("lease_until")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            ));
        }
        let Some(ledger) = get_ledger(t, task_id)? else {
            return Err(format!(
                "bind_repo_task_lane_session: no lane ledger row for task {task_id}"
            ));
        };
        if let Some(existing) = ledger.get("session_id").and_then(|v| v.as_str()) {
            if existing != session_id {
                return Err(format!(
                    "bind_repo_task_lane_session: task {task_id} already bound to session {existing}"
                ));
            }
        }
        t.exec(
            "UPDATE repo_task_lane_ledger SET session_id = ?, updated_at = ? \
             WHERE task_id = ?",
            &[
                Arg::Text(session_id.clone()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        let progress = merge_obj(
            &task.get("progress").cloned().unwrap_or(json!({})),
            &json!({"session_id": session_id, "launched_by": launched_by}),
        );
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "lane_session_bound",
            &with_marks(
                &json!({
                    "session_id": session_id, "launched_by": launched_by,
                    "via": "bind_repo_task_lane_session_turso",
                }),
                &task,
            ),
        )?;
        get_ledger(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) if e.starts_with("bind_repo_task_lane_session:") => fail(4, e),
        Err(e) => terr(e),
    }
}

/// Whether a broker envelope reports success.
fn envelope_is_ok(v: &Value) -> bool {
    v.get("ok").and_then(|o| o.as_bool()) == Some(true)
}

/// #5757-A: claim + lane-assign + session-bind in one broker
/// round trip — the composition of the three verbs, in order,
/// with the same semantics as calling them in sequence: a
/// step's error envelope, or an empty claim, returns verbatim
/// and later steps are not attempted. The answer carries every
/// step's row: `row` is the claimed task row (the shape /claim
/// answers and the client's claim_output consumes); `lane_row`
/// and `bind_row` are exactly what /lane and /bind answer in
/// the three-verb flow.
fn verb_claim_flow(t: &TursoBackend, req: &Value) -> Value {
    if let Err(e) = crate::client::claim_flow_precheck(req) {
        return fail(2, e);
    }
    let claim = verb_claim(t, req);
    if !envelope_is_ok(&claim) {
        return claim;
    }
    let row = claim.get("row").cloned().unwrap_or(Value::Null);
    if row.is_null() {
        return claim;
    }
    let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut lane_req = req.clone();
    if let Some(obj) = lane_req.as_object_mut() {
        obj.insert("task_id".to_string(), json!(task_id));
    }
    let lane = verb_lane(t, &lane_req);
    if !envelope_is_ok(&lane) {
        return lane;
    }
    let bind_req = json!({
        "task_id": task_id,
        "worker": req.get("worker"),
        "session_id": req.get("session_id"),
        "launched_by": req.get("launched_by"),
    });
    let bind = verb_bind(t, &bind_req);
    if !envelope_is_ok(&bind) {
        return bind;
    }
    ok(json!({
        "row": row,
        "lane_row": lane.get("row").cloned().unwrap_or(Value::Null),
        "bind_row": bind.get("row").cloned().unwrap_or(Value::Null),
    }))
}

/// Shared guard for the claimed-by verbs: returns the task row or
/// the PG-shaped refusal.
fn claimed_by(t: &TursoBackend, task_id: i64, worker: &str, verb: &str) -> Result<Value, Value> {
    match get_task(t, task_id) {
        Ok(Some(row))
            if row.get("status").and_then(|v| v.as_str()) == Some("claimed")
                && row.get("worker_id").and_then(|v| v.as_str()) == Some(worker) =>
        {
            Ok(row)
        }
        Ok(_) => Err(fail(
            5,
            format!("{verb}: task {task_id} is not claimed by {worker}"),
        )),
        Err(e) => Err(terr(e)),
    }
}

fn verb_heartbeat(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let extend = req_i64(req, "extend_seconds").unwrap_or(HEARTBEAT_DEFAULT_EXTEND_SECONDS);
    let row = match claimed_by(t, task_id, &worker, "heartbeat_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    let progress = merge_obj(
        &row.get("progress").cloned().unwrap_or(json!({})),
        &req.get("progress").cloned().unwrap_or(json!({})),
    );
    let lease_until = after_iso(extend);
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET lease_until = ?, progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(lease_until.clone()),
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec(
            "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(task_id) DO UPDATE SET lease_until = excluded.lease_until, \
               heartbeat_at = excluded.heartbeat_at",
            &[
                Arg::Int(task_id),
                Arg::Text(worker.clone()),
                Arg::Text(lease_until),
                Arg::Text(now_iso()),
            ],
        )?;
        touch_worker(t, &worker, Some(task_id))?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => terr(e),
    }
}

fn verb_checkpoint(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let stage = req_str(req, "stage").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "checkpoint_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    let given = req.get("progress").cloned().unwrap_or(json!({}));
    let progress = merge_obj(
        &row.get("progress").cloned().unwrap_or(json!({})),
        &merge_obj(
            &given,
            &json!({"last_checkpoint": {"stage": stage, "at": now_iso()}}),
        ),
    );
    let work = (|| -> Result<Value, TErr> {
        // PG checkpoints is a per-worker KV (worker_id, key, value):
        // the checkpoint lands under task:<id> with the stage +
        // progress as the value; the task row's progress merge above
        // is the other half of the contract.
        t.exec(
            "INSERT INTO checkpoints (worker_id, key, value, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(worker_id, key) DO UPDATE SET \
               value = excluded.value, updated_at = excluded.updated_at",
            &[
                Arg::Text(worker.clone()),
                Arg::Text(format!("task:{task_id}")),
                Arg::Text(json!({"stage": stage, "progress": given, "at": now_iso()}).to_string()),
                Arg::Text(now_iso()),
            ],
        )?;
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        touch_worker(t, &worker, Some(task_id))?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "checkpoint",
            &json!({"stage": stage, "via": "checkpoint_task_turso"}),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => terr(e),
    }
}

/// wait_kind mapping, copied from the PG park_task body (0023 §3).
fn wait_kind(condition: &Value) -> &'static str {
    match condition.get("kind").and_then(|k| k.as_str()) {
        Some("time") => "timer",
        Some("triage") => "triage",
        Some("ci") | Some("ci_green") | Some("pr_merged") | Some("issue_closed")
        | Some("upstream_pr") => "ci",
        Some("review") | Some("codex_review") | Some("pr_review") | Some("person_answer") => {
            "review"
        }
        Some("capacity") | Some("quota_reset") => "capacity",
        _ => "external",
    }
}

/// Broker-owned minimum wait TTLs (#8184): whatever delay a lane
/// asks for, a recorded wait is held at least this long. Lanes
/// re-discovering an unchanged wait every 20-25 minutes was the
/// park churn: one discovery park per wait at these scales costs
/// ~3% of the cycles. Floors: triage waits 6h (the triage step
/// works on its own cadence and wakes the item via /wake when
/// done), ci and capacity waits 1h (backlog-scale, not
/// poll-scale). Timer/review/external waits keep the caller's
/// delay — those parks carry an explicit owner-chosen horizon.
fn wait_floor_seconds(wait_kind: &str) -> Option<i64> {
    match wait_kind {
        "triage" => Some(21_600),
        "ci" | "capacity" => Some(3_600),
        _ => None,
    }
}

/// Apply the wait-kind floor to a park's not_before: a requested
/// wake earlier than the floor is bumped to the floor (and the
/// park becomes relative); a later one stands.
fn apply_wait_floor(
    wait_kind: &str,
    not_before: String,
    park_kind: &'static str,
) -> (String, &'static str) {
    match wait_floor_seconds(wait_kind) {
        Some(floor) => {
            let earliest = after_iso(floor);
            if not_before < earliest {
                (earliest, "relative")
            } else {
                (not_before, park_kind)
            }
        }
        None => (not_before, park_kind),
    }
}

fn verb_park(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "park_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    let (not_before, park_kind) = match req_str(req, "not_before") {
        Some(nb) => (nb.to_string(), "absolute"),
        None => (
            after_iso(req_i64(req, "delay_seconds").unwrap_or(1200)),
            "relative",
        ),
    };
    let given_progress = req.get("progress").cloned().unwrap_or(json!({}));
    let mut condition_defaulted = false;
    let condition = match req
        .get("ready_condition")
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| given_progress.get("ready_condition").cloned())
    {
        Some(c) => c,
        None => {
            condition_defaulted = true;
            json!({"kind": "time", "until": not_before})
        }
    };
    // #8184: a park whose recorded stage is awaiting_triage IS a
    // triage wait, whatever condition the caller attached — the
    // broker classifies it, floors its TTL, and owns the wait.
    let wk = if progress_stage(&given_progress) == Some("awaiting_triage") {
        "triage"
    } else {
        wait_kind(&condition)
    };
    let (not_before, park_kind) = apply_wait_floor(wk, not_before, park_kind);
    let condition = if condition_defaulted {
        json!({"kind": "time", "until": not_before})
    } else {
        condition
    };
    let (progress, cleared) = clear_binding(
        &row.get("progress").cloned().unwrap_or(json!({})),
        "park_task",
    );
    let progress = merge_obj(&progress, &given_progress);
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"park_reason": reason}),
    );
    // #8234: the park verb classifies its own landing. A park whose
    // merged progress is a recorded opencode handoff with no
    // caller-declared genuine wait is not a wait at all: it lands
    // in the handoff sink (status 'handed_off', every wait field
    // NULL) instead of 'parked' under the default timer. The
    // triage classification above is a genuine wait by #8184's own
    // rule and never sinks.
    let to_sink = wk != "triage" && handoff_park(&progress, condition_defaulted, &condition);
    let work = (|| -> Result<Value, TErr> {
        if to_sink {
            t.exec(
                "UPDATE tasks SET status = 'handed_off', worker_id = NULL, lease_until = NULL, \
                   not_before = NULL, park_kind = NULL, wait_kind = NULL, ready_condition = NULL, \
                   last_checked_at = ?, check_outcome = NULL, progress = ?, result = ?, \
                   updated_at = ? WHERE id = ?",
                &[
                    Arg::Text(now_iso()),
                    Arg::Text(progress.to_string()),
                    Arg::Text(result.to_string()),
                    Arg::Text(now_iso()),
                    Arg::Int(task_id),
                ],
            )?;
        } else {
            t.exec(
                "UPDATE tasks SET status = 'parked', worker_id = NULL, lease_until = NULL, \
                   not_before = ?, park_kind = ?, wait_kind = ?, ready_condition = ?, \
                   last_checked_at = ?, check_outcome = NULL, progress = ?, result = ?, \
                   updated_at = ? WHERE id = ?",
                &[
                    Arg::Text(not_before.clone()),
                    Arg::Text(park_kind.to_string()),
                    Arg::Text(wk.to_string()),
                    Arg::Text(condition.to_string()),
                    Arg::Text(now_iso()),
                    Arg::Text(progress.to_string()),
                    Arg::Text(result.to_string()),
                    Arg::Text(now_iso()),
                    Arg::Int(task_id),
                ],
            )?;
        }
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, task_id, old, "park_task")?;
        }
        let detail = if to_sink {
            json!({
                "reason": reason, "status": "handed_off",
                "park_kind": Value::Null, "wait_kind": Value::Null,
                "not_before": Value::Null, "ready_condition": Value::Null,
                "cleared_session_id": cleared,
            })
        } else {
            json!({
                "reason": reason, "status": "parked",
                "park_kind": park_kind, "wait_kind": wk,
                "not_before": not_before, "ready_condition": condition,
                "cleared_session_id": cleared,
            })
        };
        event(
            t,
            Some(&worker),
            Some(task_id),
            "parked",
            &with_marks(&detail, &row),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Whether a task row may be woken by the /wake verb (#8184):
/// only a parked (broker-held waiting) row. Claimed/queued rows
/// are already in the claim flow; terminal rows are done.
fn wake_eligible(status: &str) -> Result<(), String> {
    match status {
        "parked" => Ok(()),
        // #8234: a sink row is not a wait and cannot be woken —
        // it is already claimable; the refusal says so instead of
        // the generic not-a-wait message.
        "handed_off" => Err(
            "task status handed_off cannot be woken: a handed_off row is not \
             a wait — it is already claimable, claim it instead"
                .to_string(),
        ),
        other => Err(format!(
            "task status {other} cannot be woken (only parked waits)"
        )),
    }
}

/// The wake verb (#8184): release a broker-held wait NOW because
/// its condition was satisfied — the triage step waking an item
/// it just triaged is the canonical caller. The row returns to
/// queued with its wait fields cleared, due immediately; the
/// next claim is an ordinary execution claim. This replaces the
/// lanes' 25-minute re-check loop as the wake path for recorded
/// waits: the park TTL floor is only the backstop.
/// Release a claimed task straight back to queued (#8217): the
/// holder gives the item back to the pool NOW — no park timer, no
/// attempt change (the claim happened; the `released` event
/// carries the reason, and revive remains the cap-repair tool).
/// Holder-only, through the same claimed_by guard as checkpoint.
/// This is the dispatch failure path: an item whose sub-task
/// session failed to launch/bind returns to the pool immediately
/// instead of sitting claimed-idle under the dispatcher.
fn verb_release(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    if reason.trim().is_empty() {
        return fail(
            2,
            "release_repo_task: a release requires a reason              (--reason): the audit event is the only record of why              the item went back"
                .to_string(),
        );
    }
    let row = match claimed_by(t, task_id, &worker, "release_repo_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    let work = (|| -> Result<Value, TErr> {
        // A released claim never leaves a binding behind
        // (migration 0023 §4's rule, mirrored from the claim path).
        let progress = row.get("progress").cloned().unwrap_or(json!({}));
        let (progress, cleared) = clear_binding(&progress, "release_repo_task");
        t.exec(
            "UPDATE tasks SET status = 'queued', worker_id = NULL, \
               lease_until = NULL, not_before = NULL, park_kind = NULL, \
               wait_kind = NULL, progress = ?, updated_at = ? \
             WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        touch_worker(t, &worker, Some(task_id))?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, task_id, old, "release_repo_task")?;
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "released",
            &with_marks(
                &json!({
                    "reason": reason, "via": "release_repo_task_turso",
                    "cleared_session_id": cleared,
                }),
                &row,
            ),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({ "row": row })),
        Err(e) => terr(e),
    }
}

fn verb_wake(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker")
        .unwrap_or("fleet-queue-cli")
        .to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id).map_err(|e| e)? else {
            return Err(format!("wake_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let prev_wait_kind = row
            .get("wait_kind")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if status == "blocked" && dep_block_active(&row) {
            // #8276: wake on a dep-blocked task forces an immediate
            // recheck of that task — the only honest answer to
            // "is the gate clear yet?" is the broker's own check.
            // The recheck unblocks iff every dep has resolved; the
            // row returned is the post-recheck state either way.
            let mut cache = std::collections::HashMap::new();
            dep_recheck_one(t, &row, &mut cache, now_unix()).map_err(|e| e)?;
            return get_task(t, task_id)
                .map(|r| r.unwrap_or(Value::Null))
                .map_err(|e| e);
        }
        wake_eligible(&status).map_err(|e| format!("wake_task: task {task_id}: {e}"))?;
        t.exec(
            "UPDATE tasks SET status = 'queued', not_before = NULL, \
               park_kind = NULL, wait_kind = NULL, worker_id = NULL, \
               lease_until = NULL, updated_at = ? \
             WHERE id = ? AND status = 'parked'",
            &[Arg::Text(now_iso()), Arg::Int(task_id)],
        )
        .map_err(|e| e)?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "woken",
            &with_marks(
                &json!({
                    "reason": reason, "previous_status": status,
                    "previous_wait_kind": prev_wait_kind,
                    "via": "wake_task_turso",
                }),
                &row,
            ),
        )
        .map_err(|e| e)?;
        get_task(t, task_id)
            .map(|r| r.unwrap_or(Value::Null))
            .map_err(|e| e)
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(5, format!("wake_task: task {task_id} does not exist")),
        Err(e) if e.starts_with("wake_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

fn verb_block(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason_class = req_str(req, "reason_class").unwrap_or("").to_string();
    let blocker = req_str(req, "blocker").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "block_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    let progress = merge_obj(
        &merge_obj(
            &row.get("progress").cloned().unwrap_or(json!({})),
            &req.get("progress").cloned().unwrap_or(json!({})),
        ),
        &json!({"blocker": blocker, "reason_class": reason_class}),
    );
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"blocker": blocker, "reason_class": reason_class}),
    );
    // #8276: typed deps, when the request carries them — the block
    // becomes data the broker watches (see run_dep_rechecks). The
    // first recheck is due immediately, so a dep that is already
    // satisfied (an already-merged PR) clears within one sweep.
    // A dep-less block NULLs the dep columns: a stale record can
    // never outlive the block that owned it.
    let now_unix_v = now_unix();
    let deps = match parse_block_deps(req, &worker, &iso(now_unix_v)) {
        Ok(d) => d,
        Err(e) => return fail(2, e),
    };
    let (dep_json, dep_next, dep_until, dep_rs, dep_event) = match &deps {
        Some(ds) => {
            let rs = clamp_dep_recheck_seconds(req_i64(req, "dep_recheck_seconds"));
            let ttl = clamp_dep_ttl_seconds(req_i64(req, "dep_max_ttl_seconds"));
            let until = iso(now_unix_v + ttl);
            (
                Some(json!(ds).to_string()),
                Some(iso(now_unix_v)),
                Some(until.clone()),
                Some(rs),
                json!({
                    "deps": ds, "dep_blocked_until": until,
                    "dep_recheck_seconds": rs,
                }),
            )
        }
        None => (None, None, None, None, json!({})),
    };
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET status = 'blocked', worker_id = NULL, lease_until = NULL, \
               progress = ?, result = ?, blocked_deps = ?, dep_next_recheck_at = ?, \
               dep_blocked_until = ?, dep_recheck_seconds = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(result.to_string()),
                arg_str(dep_json.as_deref()),
                arg_str(dep_next.as_deref()),
                arg_str(dep_until.as_deref()),
                dep_rs.map(Arg::Int).unwrap_or(Arg::Null),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "blocked",
            &merge_obj(
                &json!({"blocker": blocker, "reason_class": reason_class}),
                &dep_event,
            ),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => terr(e),
    }
}


fn emit_dep_transitions(
    t: &TursoBackend,
    task_id: i64,
    row: &Value,
    transitions: &[Value],
) -> Result<(), TErr> {
    for tr in transitions {
        let kind = if tr.get("transition").and_then(|v| v.as_str()) == Some("broken") {
            "dep_broken"
        } else {
            "dep_cleared"
        };
        event(
            t,
            Some("dep-rechecker"),
            Some(task_id),
            kind,
            &with_marks(
                &json!({
                    "dep": tr.get("dep").cloned().unwrap_or(Value::Null),
                    "evidence": tr.get("evidence").cloned().unwrap_or(Value::Null),
                    "via": "dep_recheck",
                }),
                row,
            ),
        )?;
    }
    Ok(())
}

/// One dep-blocked task's recheck (#8276). The max-TTL deadline
/// is checked before any probing: an expired block releases
/// without consulting GitHub, so a dead checker cannot stretch a
/// block past its TTL. Otherwise each open dep gets one
/// observation (task deps from the store, GitHub deps through the
/// probe, refs shared across the pass via `cache`), transitions
/// are audited, and the task either unblocks (every dep resolved)
/// or reschedules at its recheck interval. Returns the outcome
/// tag ("unblocked" / "ttl_expired" / "still_blocked" /
/// "not_dep_blocked") and the number of failed checks seen.
fn dep_recheck_one(
    t: &TursoBackend,
    row: &Value,
    cache: &mut std::collections::HashMap<String, DepObs>,
    now_unix_v: i64,
) -> Result<(&'static str, i64), TErr> {
    let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let now = iso(now_unix_v);
    let deps = deps_of(row);
    if deps.is_empty() {
        return Ok(("not_dep_blocked", 0));
    }
    if let Some(until) = row.get("dep_blocked_until").and_then(|v| v.as_str()) {
        if until <= now.as_str() {
            let cleared = t.exec(
                "UPDATE tasks SET status = 'queued', blocked_deps = NULL, \
                   dep_next_recheck_at = NULL, dep_blocked_until = NULL, \
                   dep_recheck_seconds = NULL, updated_at = ? \
                 WHERE id = ? AND status = 'blocked'",
                &[Arg::Text(now.clone()), Arg::Int(task_id)],
            )?;
            if cleared.affected > 0 {
                event(
                    t,
                    Some("dep-rechecker"),
                    Some(task_id),
                    "dep_ttl_expired",
                    &with_marks(
                        &json!({
                            "deps": deps, "dep_blocked_until": until,
                            "via": "dep_recheck",
                        }),
                        row,
                    ),
                )?;
                return Ok(("ttl_expired", 0));
            }
            return Ok(("still_blocked", 0));
        }
    }
    let recheck_seconds = row
        .get("dep_recheck_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(DEP_RECHECK_DEFAULT_SECONDS);
    let mut new_deps = deps.clone();
    let mut failures = 0i64;
    let mut transitions: Vec<Value> = Vec::new();
    for dep in new_deps.iter_mut() {
        if dep.get("state").and_then(|v| v.as_str()).unwrap_or("open") != "open" {
            continue;
        }
        let kind = dep
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let obs = match kind.as_str() {
            "task" => {
                let dep_id = dep.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                let st = get_task(t, dep_id)?.and_then(|r| {
                    r.get("status")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                });
                task_dep_obs(st.as_deref(), dep_id)
            }
            "issue" | "pr" | "ci" => {
                let key = dep_key(dep);
                if let Some(o) = cache.get(&key) {
                    o.clone()
                } else {
                    let repo = dep.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                    let num = |k: &str| dep.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
                    let o = match (t.gh_probe(), kind.as_str()) {
                        (Some(p), "issue") => p.probe_issue(repo, num("number")),
                        (Some(p), "pr") => p.probe_pr(repo, num("number")),
                        (Some(p), "ci") => p.probe_ci(repo, num("run_id")),
                        _ => DepObs::Failed(
                            "no github probe configured on this broker".to_string(),
                        ),
                    };
                    cache.insert(key, o.clone());
                    o
                }
            }
            _ => DepObs::Broken(format!("unknown dep kind '{kind}'")),
        };
        if matches!(obs, DepObs::Failed(_)) {
            failures += 1;
        }
        if let Some(tr) = apply_dep_obs(dep, &obs, &now) {
            transitions.push(json!({
                "dep": dep_key(dep), "transition": tr,
                "evidence": dep.get("last_observed").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    if deps_all_resolved(&new_deps) {
        let cleared = t.exec(
            "UPDATE tasks SET status = 'queued', blocked_deps = NULL, \
               dep_next_recheck_at = NULL, dep_blocked_until = NULL, \
               dep_recheck_seconds = NULL, updated_at = ? \
             WHERE id = ? AND status = 'blocked'",
            &[Arg::Text(now.clone()), Arg::Int(task_id)],
        )?;
        if cleared.affected == 0 {
            // Lost a race with a manual unblock or a re-block;
            // that transition owns the audit trail.
            return Ok(("still_blocked", failures));
        }
        emit_dep_transitions(t, task_id, row, &transitions)?;
        event(
            t,
            Some("dep-rechecker"),
            Some(task_id),
            "dep_unblocked",
            &with_marks(
                &json!({
                    "deps": new_deps, "transitions": transitions,
                    "via": "dep_recheck",
                }),
                row,
            ),
        )?;
        return Ok(("unblocked", failures));
    }
    t.exec(
        "UPDATE tasks SET blocked_deps = ?, dep_next_recheck_at = ?, updated_at = ? \
         WHERE id = ? AND status = 'blocked'",
        &[
            Arg::Text(json!(new_deps).to_string()),
            // The max TTL is a hard deadline: never schedule
            // the next recheck past dep_blocked_until, or expiry
            // would only be noticed one recheck too late. (ISO
            // strings compare lexicographically in this store.)
            Arg::Text({
                let next = iso(now_unix_v + recheck_seconds);
                match row.get("dep_blocked_until").and_then(|v| v.as_str()) {
                    Some(until) if until < next.as_str() => until.to_string(),
                    _ => next,
                }
            }),
            Arg::Text(now.clone()),
            Arg::Int(task_id),
        ],
    )?;
    emit_dep_transitions(t, task_id, row, &transitions)?;
    Ok(("still_blocked", failures))
}

/// The dep recheck pass (#8276): every dep-blocked task whose
/// recheck time has arrived, examined once, GitHub refs shared
/// across tasks through a per-pass cache. Driven by verb_sweep
/// (the fleet-queue-expire timer fires /sweep every minute, so
/// this runs with no lane awake), by /dep-recheck directly, and
/// per-task by wake.
pub fn run_dep_rechecks(t: &TursoBackend, limit: i64) -> Result<Value, TErr> {
    let now_unix_v = now_unix();
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks WHERE status = 'blocked' \
             AND blocked_deps IS NOT NULL \
             AND (dep_next_recheck_at IS NULL OR dep_next_recheck_at <= ?) \
             ORDER BY dep_next_recheck_at ASC LIMIT {limit}"
        ),
        &[Arg::Text(iso(now_unix_v))],
        &TASK_JSON_COLS,
    )?;
    let mut cache: std::collections::HashMap<String, DepObs> =
        std::collections::HashMap::new();
    let (mut checked, mut unblocked, mut ttl_expired, mut still_blocked, mut failures) =
        (0i64, 0i64, 0i64, 0i64, 0i64);
    for row in &rows {
        let (outcome, f) = dep_recheck_one(t, row, &mut cache, now_unix_v)?;
        checked += 1;
        failures += f;
        match outcome {
            "unblocked" => unblocked += 1,
            "ttl_expired" => ttl_expired += 1,
            _ => still_blocked += 1,
        }
    }
    Ok(json!({
        "checked": checked, "unblocked": unblocked,
        "ttl_expired": ttl_expired, "still_blocked": still_blocked,
        "check_failures": failures,
    }))
}

/// /dep-recheck (#8276): run the due pass, or — with a task_id —
/// force one dep-blocked task's recheck right now, schedule or
/// no schedule. The manual trigger lanes and the monitor use to
/// prove the machinery; production cadence is the sweep's.
fn verb_dep_recheck(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        if let Some(task_id) = req_i64(req, "task_id") {
            let Some(row) = get_task(t, task_id)? else {
                return Ok(json!({"task_id": task_id, "outcome": "no_such_task"}));
            };
            if row.get("status").and_then(|v| v.as_str()) != Some("blocked")
                || !dep_block_active(&row)
            {
                return Ok(json!({"task_id": task_id, "outcome": "not_dep_blocked"}));
            }
            let mut cache = std::collections::HashMap::new();
            let (outcome, failures) = dep_recheck_one(t, &row, &mut cache, now_unix())?;
            let after = get_task(t, task_id)?.unwrap_or(Value::Null);
            return Ok(json!({
                "task_id": task_id, "outcome": outcome,
                "check_failures": failures, "row": after,
            }));
        }
        run_dep_rechecks(t, req_i64(req, "limit").unwrap_or(200))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// /unblock (#8276): the manual override. A lane or the monitor
/// releases a blocked task now — dep-blocked or classic — and
/// the event names who overrode and what was discarded. The
/// rechecker's automatic paths never impersonate this: an
/// override is a human/agent decision, audited as one.
fn verb_unblock(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id).map_err(|e| e)? else {
            return Err(format!("unblock_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if status != "blocked" {
            return Err(format!(
                "unblock_task: task {task_id} is {status}, not blocked"
            ));
        }
        let prev_deps = deps_of(&row);
        t.exec(
            "UPDATE tasks SET status = 'queued', blocked_deps = NULL, \
               dep_next_recheck_at = NULL, dep_blocked_until = NULL, \
               dep_recheck_seconds = NULL, updated_at = ? \
             WHERE id = ? AND status = 'blocked'",
            &[Arg::Text(now_iso()), Arg::Int(task_id)],
        )
        .map_err(|e| e)?;
        let (kind, detail) = if prev_deps.is_empty() {
            (
                "unblocked",
                json!({"reason": reason, "via": "unblock_task_turso"}),
            )
        } else {
            (
                "dep_unblock_override",
                json!({
                    "reason": reason, "discarded_deps": prev_deps,
                    "via": "unblock_task_turso",
                }),
            )
        };
        event(
            t,
            Some(&worker),
            Some(task_id),
            kind,
            &with_marks(&detail, &row),
        )
        .map_err(|e| e)?;
        get_task(t, task_id)
            .map(|r| r.unwrap_or(Value::Null))
            .map_err(|e| e)
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(5, format!("unblock_task: task {task_id} does not exist")),
        Err(e) if e.starts_with("unblock_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

fn verb_cancel(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker")
        .unwrap_or("fleet-monitor")
        .to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, TErr> {
        let Some(row) = get_task(t, task_id)? else {
            return Ok(Value::Null);
        };
        if ["done", "cancelled", "failed"]
            .contains(&row.get("status").and_then(|v| v.as_str()).unwrap_or(""))
        {
            return Ok(row);
        }
        let result = merge_obj(
            &row.get("result").cloned().unwrap_or(json!({})),
            &json!({"cancel_reason": reason}),
        );
        t.exec(
            "UPDATE tasks SET status = 'cancelled', worker_id = NULL, lease_until = NULL, \
               result = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "cancelled",
            &json!({"reason": reason, "via": "cancel_repo_task_turso"}),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(
            5,
            format!("cancel_repo_task: task {task_id} does not exist"),
        ),
        Err(e) => terr(e),
    }
}

fn verb_fail(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let error = req_str(req, "error").unwrap_or("").to_string();
    let reason_class = req_str(req, "reason_class")
        .unwrap_or("transient")
        .to_string();
    let row = match claimed_by(t, task_id, &worker, "fail_repo_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    // Migration 0023 §2, faithfully: transient + attempts left
    // requeues with a backoff (900 s first, else 2700 s); 'blocked'
    // blocks; anything else fails terminally.
    let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
    let max_attempts = row
        .get("max_attempts")
        .and_then(|v| v.as_i64())
        .unwrap_or(4);
    let requeue = reason_class == "transient" && attempts < max_attempts;
    let new_status = if requeue {
        "queued"
    } else if reason_class == "blocked" {
        "blocked"
    } else {
        "failed"
    };
    let not_before = if requeue {
        after_iso(if attempts <= 1 { 900 } else { 2700 })
    } else {
        row.get("not_before")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let progress0 = row.get("progress").cloned().unwrap_or(json!({}));
    let (progress, cleared) = if requeue {
        clear_binding(&progress0, "fail_repo_task")
    } else {
        (progress0, None)
    };
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"reason_class": reason_class}),
    );
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET status = ?, worker_id = NULL, lease_until = NULL, \
               not_before = ?, last_error = ?, progress = ?, result = ?, updated_at = ? \
             WHERE id = ?",
            &[
                Arg::Text(new_status.to_string()),
                Arg::Text(not_before),
                Arg::Text(error.clone()),
                Arg::Text(progress.to_string()),
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        if requeue {
            if let Some(old) = &cleared {
                on_binding_cleared(t, &worker, task_id, old, "fail_repo_task")?;
            }
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "failed",
            &with_marks(
                &json!({
                    "error": error, "reason_class": reason_class, "requeued": requeue,
                    "via": "fail_repo_task_turso",
                }),
                &row,
            ),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Completion gating (Drew's triage doctrine, #7016; the
/// completion-without-evidence defect, 2026-11-07): what a task's
/// completion must carry depends on its class, derived from the
/// packet kind in its payload.
#[derive(Debug, PartialEq)]
enum CompletionClass {
    /// Work that produces an artifact: completion requires an
    /// evidence reference recorded on the task.
    Fix,
    /// Triage annotates and validates; it never completes the
    /// issue. Completion re-enqueues the follow-on by default.
    Triage,
    /// No packet kind (hand-built rows, legacy tasks): ungated, so
    /// the gate never breaks non-packet work.
    Ungated,
}

fn completion_class(payload: &Value) -> CompletionClass {
    match payload.get("kind").and_then(|k| k.as_str()) {
        Some("small_fix") | Some("ci_investigation") | Some("pr_shepherd") => CompletionClass::Fix,
        Some("triage") => CompletionClass::Triage,
        _ => CompletionClass::Ungated,
    }
}

/// The evidence reference on a completion result, if it carries
/// one: a merged PR, a commit, or a verified-closure note, under
/// any of the field names the fleet's completion notes use.
fn evidence_ref(result: &Value) -> Option<Value> {
    for key in [
        "evidence",
        "pr_url",
        "pr",
        "merged_pr",
        "commit",
        "commit_sha",
        "verified_closure",
    ] {
        match result.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                return Some(json!({key: s}));
            }
            Some(Value::Array(a)) if !a.is_empty() => {
                return Some(json!({key: a}));
            }
            Some(v @ Value::Object(_)) => return Some(json!({key: v})),
            _ => {}
        }
    }
    None
}

/// What a triage completion does with its issue (doctrine exits:
/// invalid is the only no-evidence-free exit, and both exits still
/// require the validation evidence on the record).
#[derive(Debug, PartialEq)]
enum TriageOutcome {
    FollowOn,
    ExitInvalid,
    ExitAlreadySolved,
}

/// The broker-side completion gate. Err = the completion is
/// rejected and the task stays claimed.
fn gate_completion(
    class: &CompletionClass,
    result: &Value,
) -> Result<Option<TriageOutcome>, String> {
    match class {
        CompletionClass::Ungated => Ok(None),
        CompletionClass::Fix => {
            if evidence_ref(result).is_none() {
                return Err("complete_task: fix-class completion requires an evidence \
                     reference on the result (merged PR, commit, or \
                     verified-closure note); the task remains claimed"
                    .to_string());
            }
            Ok(None)
        }
        CompletionClass::Triage => {
            let outcome = result
                .get("triage_outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("follow_on");
            match outcome {
                "follow_on" => Ok(Some(TriageOutcome::FollowOn)),
                "invalid" => {
                    // The validation note is the evidence for this
                    // exit; a bare verdict is not enough.
                    let noted = evidence_ref(result).is_some()
                        || matches!(
                            result.get("triage_note"),
                            Some(Value::String(s)) if !s.trim().is_empty()
                        );
                    if !noted {
                        return Err("complete_task: triage exit 'invalid' requires the \
                             validation evidence on the result (evidence or \
                             triage_note); the task remains claimed"
                            .to_string());
                    }
                    Ok(Some(TriageOutcome::ExitInvalid))
                }
                "already_solved" => {
                    if evidence_ref(result).is_none() {
                        return Err("complete_task: triage exit 'already_solved' requires \
                             an evidence reference on the result (merged PR, \
                             commit, or verified-closure note); the task \
                             remains claimed"
                            .to_string());
                    }
                    Ok(Some(TriageOutcome::ExitAlreadySolved))
                }
                other => Err(format!(
                    "complete_task: unknown triage_outcome {other:?} \
                     (follow_on | invalid | already_solved)"
                )),
            }
        }
    }
}

/// The follow-on fix key for a triage key: same family shape with
/// the family swapped triage -> fix. None for keys with no fix
/// counterpart (maintenance, pr families).
fn follow_on_key(triage_key: &str) -> Option<String> {
    if triage_key.contains(":triage:") && triage_key.contains(":issue:") {
        Some(triage_key.replacen(":triage:", ":fix:", 1))
    } else {
        None
    }
}

fn verb_complete(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "complete_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    // The completion gate runs BEFORE any mutation: a rejected
    // completion leaves the task claimed by its worker.
    let payload = row.get("payload").cloned().unwrap_or(json!({}));
    let class = completion_class(&payload);
    let gate_result = req.get("result").cloned().unwrap_or(json!({}));
    let triage_outcome = match gate_completion(&class, &gate_result) {
        Ok(o) => o,
        Err(e) => return fail(2, e),
    };
    // Terminal transitions keep the session binding (migration 0023
    // header): on a done row it is the execution record.
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &req.get("result").cloned().unwrap_or(json!({})),
    );
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET status = 'done', lease_until = NULL, result = ?, \
               completed_at = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        touch_worker(t, &worker, None)?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "completed",
            &with_marks(&json!({"via": "complete_task_turso"}), &row),
        )?;
        // Triage doctrine (#7016): a triage completion is not the
        // issue being done — the issue goes back in the queue as a
        // fix follow-on unless triage exited it (invalid /
        // already_solved, gated above). The follow-on dedupes
        // against any live fix task for the same key; a terminal
        // predecessor links via progress like an enqueue revive.
        let mut follow_on_id: Option<i64> = None;
        if triage_outcome == Some(TriageOutcome::FollowOn) {
            let queue = row
                .get("queue")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let triage_key = row
                .get("dedupe_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match follow_on_key(&triage_key) {
                Some(fix_key) => {
                    let existing = t.query_objs(
                        &format!(
                            "SELECT {TASK_COLS} FROM tasks \
                             WHERE queue = ? AND dedupe_key = ? ORDER BY id ASC"
                        ),
                        &[Arg::Text(queue.clone()), Arg::Text(fix_key.clone())],
                        &TASK_JSON_COLS,
                    )?;
                    let (live, terminal) = dedupe_partition(&existing);
                    let fid = if let Some(live_row) = live {
                        live_row.get("id").and_then(|v| v.as_i64()).unwrap_or(0)
                    } else {
                        let mut fpayload = payload.clone();
                        if let Some(obj) = fpayload.as_object_mut() {
                            obj.insert("kind".to_string(), json!("small_fix"));
                            obj.insert(
                                "triage_follow_on".to_string(),
                                json!({"from_task": task_id, "at": now_iso()}),
                            );
                        }
                        let fprogress = match &terminal {
                            Some(pred) => json!({
                                "triage_source_task": task_id,
                                "revived_from": {
                                    "task_id": pred.get("id").cloned().unwrap_or(Value::Null),
                                    "status": pred.get("status").cloned().unwrap_or(Value::Null),
                                    "via": "triage_completion_follow_on",
                                },
                            }),
                            None => json!({"triage_source_task": task_id}),
                        };
                        let ins = t.exec(
                            "INSERT INTO tasks (queue, title, body, payload, dedupe_key, \
                               priority, status, max_attempts, progress, result, \
                               created_at, updated_at) \
                             VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?, '{}', ?, ?)",
                            &[
                                Arg::Text(queue.clone()),
                                arg_str(row.get("title").and_then(|v| v.as_str())),
                                arg_str(row.get("body").and_then(|v| v.as_str())),
                                Arg::Text(fpayload.to_string()),
                                Arg::Text(fix_key.clone()),
                                row.get("priority")
                                    .and_then(|v| v.as_i64())
                                    .map(Arg::Int)
                                    .unwrap_or(Arg::Null),
                                Arg::Int(
                                    fpayload
                                        .get("max_attempts")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(4),
                                ),
                                Arg::Text(fprogress.to_string()),
                                Arg::Text(now_iso()),
                                Arg::Text(now_iso()),
                            ],
                        )?;
                        let new_id = ins.last_insert_rowid.unwrap_or(0);
                        event(
                            t,
                            Some(&worker),
                            Some(new_id),
                            "enqueued",
                            &json!({
                                "via": "triage_completion_follow_on",
                                "source_task": task_id, "dedupe_key": fix_key,
                            }),
                        )?;
                        new_id
                    };
                    follow_on_id = Some(fid);
                    event(
                        t,
                        Some(&worker),
                        Some(task_id),
                        "triage_follow_on_enqueued",
                        &json!({"follow_on_task_id": fid, "dedupe_key": fix_key}),
                    )?;
                }
                None => {
                    event(
                        t,
                        Some(&worker),
                        Some(task_id),
                        "triage_follow_on_skipped",
                        &json!({
                            "reason": "dedupe key has no fix-family counterpart",
                            "dedupe_key": triage_key,
                        }),
                    )?;
                }
            }
        }
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        let mut envelope = verb_with_execution(t, req, row)?;
        if let (Some(obj), Some(fid)) = (envelope.as_object_mut(), follow_on_id) {
            obj.insert("follow_on_task_id".to_string(), json!(fid));
        }
        Ok(envelope)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

fn verb_list(t: &TursoBackend, req: &Value) -> Value {
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    let limit = match crate::client::list_limit_or_default(req_i64(req, "limit")) {
        Ok(l) => l,
        Err(e) => return fail(2, e),
    };
    let rich = req.get("rich").and_then(|r| r.as_bool()).unwrap_or(false);
    let select = if rich {
        "id, queue, title, priority, status, dedupe_key, created_at, not_before, \
         worker_id, lease_until, lane_id, platform, attempts, max_attempts, \
         claim_provider, claim_model, claim_variant, progress, payload, \
         blocked_deps, dep_next_recheck_at, dep_blocked_until"
    } else {
        "id, queue, title, priority, status, dedupe_key, created_at, not_before, platform"
    };
    let mut sql = format!("SELECT {select} FROM tasks WHERE queue = ?");
    let mut args: Vec<Arg> = vec![Arg::Text(queue)];
    if let Some(s) = req_str(req, "status") {
        sql += " AND status = ?";
        args.push(Arg::Text(s.to_string()));
    }
    // #5444: recover a lane's or worker's live claims before lease
    // expiry — list filters end-to-end (client -> broker -> here).
    if let Some(l) = req_str(req, "lane") {
        sql += " AND lane_id = ?";
        args.push(Arg::Text(l.to_string()));
    }
    if let Some(w) = req_str(req, "worker") {
        sql += " AND worker_id = ?";
        args.push(Arg::Text(w.to_string()));
    }
    if let Some(nb) = req_str(req, "not_before") {
        sql += " AND (not_before IS NULL OR not_before <= ?)";
        args.push(Arg::Text(nb.to_string()));
    }
    sql += &format!(" ORDER BY (priority IS NULL), priority ASC, created_at ASC LIMIT {limit}");
    let json_cols: &[&str] = if rich {
        &["progress", "payload", "blocked_deps"]
    } else {
        &[]
    };
    match t.query_objs(&sql, &args, json_cols) {
        Ok(rows) => ok(json!({"rows": rows})),
        Err(e) => terr(e),
    }
}

/// One queue's health row with every counter zeroed. The client
/// renders the health table from these keys, so a field added here
/// surfaces in the health table output with no client change.
fn health_entry(queue: &str) -> Value {
    json!({
        "queue": queue, "queued": 0, "at_cap_queued": 0, "claimed": 0,
        "parked": 0, "handed_off": 0, "blocked": 0, "dep_blocked": 0, "done": 0, "cancelled": 0,
        "failed": 0, "total": 0, "oldest_queued_at": Value::Null,
        "terminal_events_24h": 0, "terminal_events_7d": 0,
    })
}

/// Fold per-queue at-cap queued counts (#6528) into health entries:
/// tasks with status='queued' AND attempts >= max_attempts can never
/// be claimed again (the claim query requires attempts <
/// max_attempts), so a queue whose queued tasks are all at cap reads
/// as dead-in-queue — health must name that instead of letting it
/// look like ordinary backlog. Queues with no at-cap rows keep the
/// zero default from health_entry.
fn apply_at_cap_queued(by_queue: &mut Map<String, Value>, rows: &[Value]) {
    for r in rows {
        let q = r.get("queue").and_then(|v| v.as_str()).unwrap_or("");
        let n = r.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(obj) = by_queue.get_mut(q).and_then(|e| e.as_object_mut()) {
            obj.insert("at_cap_queued".to_string(), json!(n));
        }
    }
}

/// Fold per-queue dep-blocked counts (#8276) into health
/// entries: blocked tasks carrying a structured dep record are a
/// different operational animal from classic blocks (escalations
/// wait for a human; dep-blocks wait for the broker's rechecker),
/// so health names them separately.
fn apply_dep_blocked(by_queue: &mut Map<String, Value>, rows: &[Value]) {
    for r in rows {
        let q = r.get("queue").and_then(|v| v.as_str()).unwrap_or("");
        let n = r.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(obj) = by_queue.get_mut(q).and_then(|e| e.as_object_mut()) {
            obj.insert("dep_blocked".to_string(), json!(n));
        }
    }
}

fn verb_health(t: &TursoBackend, req: &Value) -> Value {
    let queue_filter = req_str(req, "queue").map(str::to_string);
    let work = (|| -> Result<Value, TErr> {
        let mut sql = "SELECT queue, status, COUNT(*) AS n, \
               MIN(CASE WHEN status = 'queued' THEN created_at END) AS oldest_queued_at \
             FROM tasks"
            .to_string();
        let mut args: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            sql += " WHERE queue = ?";
            args.push(Arg::Text(q.clone()));
        }
        sql += " GROUP BY queue, status ORDER BY queue, status";
        let rows = t.query_objs(&sql, &args, &[])?;
        let mut by_queue: Map<String, Value> = Map::new();
        for r in &rows {
            let q = r
                .get("queue")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entry = by_queue
                .entry(q.clone())
                .or_insert_with(|| health_entry(&q));
            let status = r
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let n = r.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
            if let Some(obj) = entry.as_object_mut() {
                if let Some(slot) = obj.get_mut(&status) {
                    *slot = json!(n);
                }
                let total = obj.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
                obj.insert("total".to_string(), json!(total + n));
                if status == "queued" {
                    obj.insert(
                        "oldest_queued_at".to_string(),
                        r.get("oldest_queued_at").cloned().unwrap_or(Value::Null),
                    );
                }
            }
        }
        // At-cap queued counts (#6528): the dead-in-queue alert
        // class. Separate query because the status GROUP BY above
        // cannot express the attempts >= max_attempts predicate.
        let mut asql = "SELECT queue, COUNT(*) AS n FROM tasks \
             WHERE status = 'queued' AND attempts >= max_attempts"
            .to_string();
        let mut aargs: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            asql += " AND queue = ?";
            aargs.push(Arg::Text(q.clone()));
        }
        asql += " GROUP BY queue";
        let arows = t.query_objs(&asql, &aargs, &[])?;
        apply_at_cap_queued(&mut by_queue, &arows);
        // Dep-blocked counts (#8276): same separate-query shape as
        // at-cap — the status GROUP BY cannot express "carries a
        // dep record".
        let mut dsql = "SELECT queue, COUNT(*) AS n FROM tasks \
             WHERE status = 'blocked' AND blocked_deps IS NOT NULL"
            .to_string();
        let mut dargs: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            dsql += " AND queue = ?";
            dargs.push(Arg::Text(q.clone()));
        }
        dsql += " GROUP BY queue";
        let drows = t.query_objs(&dsql, &dargs, &[])?;
        apply_dep_blocked(&mut by_queue, &drows);
        // Terminal-event counters, the agent.repo_queue_health
        // view's terminal_events_24h/7d (#5690): events and tasks
        // share this database, so the JOIN the PG view did is a
        // local query here — no cross-database problem.
        let mut tsql = "SELECT t.queue AS queue, COUNT(*) AS n7, \
               SUM(CASE WHEN e.created_at > ? THEN 1 ELSE 0 END) AS n24 \
             FROM events e JOIN tasks t ON t.id = e.task_id \
             WHERE e.kind IN ('completed', 'failed') AND e.created_at > ?"
            .to_string();
        let mut targs: Vec<Arg> = vec![
            Arg::Text(iso(now_unix() - 86_400)),
            Arg::Text(iso(now_unix() - 7 * 86_400)),
        ];
        if let Some(q) = &queue_filter {
            tsql += " AND t.queue = ?";
            targs.push(Arg::Text(q.clone()));
        }
        tsql += " GROUP BY t.queue";
        let trows = t.query_objs(&tsql, &targs, &[])?;
        for r in &trows {
            let q = r
                .get("queue")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entry = by_queue
                .entry(q.clone())
                .or_insert_with(|| health_entry(&q));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "terminal_events_24h".to_string(),
                    r.get("n24").cloned().unwrap_or(json!(0)),
                );
                obj.insert(
                    "terminal_events_7d".to_string(),
                    r.get("n7").cloned().unwrap_or(json!(0)),
                );
            }
        }
        Ok(Value::Array(by_queue.into_iter().map(|(_, v)| v).collect()))
    })();
    match work {
        Ok(rows) => {
            let netloc = t
                .url()
                .split("://")
                .nth(1)
                .unwrap_or(t.url())
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
            ok(json!({
                "project_netloc": netloc,
                "project_source": "turso",
                "backend": "turso",
                "rows": rows,
            }))
        }
        Err(e) => terr(e),
    }
}

fn verb_signal_count(t: &TursoBackend, req: &Value) -> Value {
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(
            "SELECT COUNT(*) AS n, MIN(created_at) AS oldest FROM tasks \
             WHERE queue = ? AND status = 'queued'",
            &[Arg::Text(queue)],
            &[],
        )?;
        let first = rows.into_iter().next().unwrap_or(json!({}));
        Ok(json!({
            "queued_count": first.get("n").cloned().unwrap_or(json!(0)),
            "oldest_queued_at": first.get("oldest").cloned().unwrap_or(Value::Null),
        }))
    })();
    match work {
        Ok(v) => {
            let mut m = json!({"ok": true});
            if let (Some(dst), Some(src)) = (m.as_object_mut(), v.as_object()) {
                for (k, val) in src {
                    dst.insert(k.clone(), val.clone());
                }
            }
            m
        }
        Err(e) => terr(e),
    }
}

// ---------------------------------------------------------------------------
// Creation outbox (#3706) — the bounded pilot's Turso verbs.
//
// Storage half of `crate::creation_outbox`: the enqueue API, the
// serial drainer's claim/resolve verbs, and metrics, over the
// `creation_outbox` table (schema in SCHEMA above) plus the
// persisted circuit-breaker row (`creation_outbox_circuit`, id=1),
// so every drainer sees the same circuit. The state-machine rules
// (dedupe/conflict, backoff, pacing, read-back-before-done,
// uncertain reconciliation) are the core module's; these verbs are
// their SQL spelling. The drainer's GitHub leg is the client's
// `fleet queue outbox drain` (client::outbox); these verbs never
// shell out.
// ---------------------------------------------------------------------------

const OUTBOX_COLS: &str = "id, repo, kind, payload, dedup_key, payload_hash, \
    producer, state, created_at, available_at, attempt_count, max_attempts, \
    last_error_class, lease_owner, lease_until, github_number, github_url, completed_at";

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Parse the `iso()` format back to unix seconds (the outbox core
/// works in unix time; rows store ISO strings like every other
/// table here, where lexicographic order is chronological order).
fn iso_to_unix(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let time = time.trim_end_matches('Z');
    let mut tp = time.split(':');
    let hh: i64 = tp.next()?.parse().ok()?;
    let mm: i64 = tp.next()?.parse().ok()?;
    let ss: i64 = tp.next()?.parse::<f64>().ok()? as i64;
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn outbox_jitter_unit() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    ((nanos ^ (u64::from(std::process::id()) << 32)) % 10_000) as f64 / 10_000.0
}

fn outbox_get(t: &TursoBackend, id: &str) -> Result<Option<Value>, TErr> {
    let rows = t.query_objs(
        &format!("SELECT {OUTBOX_COLS} FROM creation_outbox WHERE id = ?"),
        &[Arg::Text(id.to_string())],
        &["payload"],
    )?;
    Ok(rows.into_iter().next())
}

fn outbox_circuit(t: &TursoBackend) -> Result<crate::creation_outbox::Circuit, TErr> {
    let rows = t.query_objs(
        "SELECT consecutive_rate_limits, open_until, last_error_class \
         FROM creation_outbox_circuit WHERE id = 1",
        &[],
        &[],
    )?;
    let mut circuit = crate::creation_outbox::Circuit::default();
    if let Some(row) = rows.into_iter().next() {
        circuit.consecutive_rate_limits = row
            .get("consecutive_rate_limits")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as u32;
        circuit.open_until = row
            .get("open_until")
            .and_then(|v| v.as_str())
            .and_then(iso_to_unix)
            .unwrap_or(0);
        circuit.last_error_class = row
            .get("last_error_class")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    Ok(circuit)
}

fn outbox_circuit_save(t: &TursoBackend, c: &crate::creation_outbox::Circuit) -> Result<(), TErr> {
    t.exec(
        "INSERT INTO creation_outbox_circuit (id, consecutive_rate_limits, open_until, last_error_class, updated_at) \
         VALUES (1, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET consecutive_rate_limits = excluded.consecutive_rate_limits, \
           open_until = excluded.open_until, last_error_class = excluded.last_error_class, \
           updated_at = excluded.updated_at",
        &[
            Arg::Int(i64::from(c.consecutive_rate_limits)),
            if c.open_until > 0 { Arg::Text(iso(c.open_until)) } else { Arg::Null },
            arg_str(c.last_error_class.as_deref()),
            Arg::Text(now_iso()),
        ],
    )?;
    Ok(())
}

/// Enqueue one creation intent. Insert + dedupe are one read-then-
/// insert against the UNIQUE(repo, dedup_key) index (the same shape
/// as task enqueue): same key + same payload hash returns the
/// original row (`replay`), same key + different hash is a conflict
/// (exit 5), never a silent overwrite.
fn verb_outbox_enqueue(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let repo = req_str(req, "repo").unwrap_or("");
        let kind = req_str(req, "kind").unwrap_or("");
        let dedup_key = req_str(req, "dedup_key").unwrap_or("");
        let payload = req.get("payload").cloned().unwrap_or(json!({}));
        if repo.is_empty() || dedup_key.is_empty() {
            return Ok(fail(2, "outbox enqueue needs repo and dedup_key"));
        }
        if !crate::creation_outbox::KINDS.contains(&kind) {
            return Ok(fail(
                2,
                format!("kind {kind:?} is not an outbox creation kind"),
            ));
        }
        let hash = crate::creation_outbox::payload_hash(&payload);
        let existing = t.query_objs(
            &format!("SELECT {OUTBOX_COLS} FROM creation_outbox WHERE repo = ? AND dedup_key = ?"),
            &[
                Arg::Text(repo.to_string()),
                Arg::Text(dedup_key.to_string()),
            ],
            &["payload"],
        )?;
        if let Some(row) = existing.into_iter().next() {
            if row.get("payload_hash").and_then(|v| v.as_str()) == Some(hash.as_str()) {
                return Ok(ok(json!({"row": row, "outcome": "replay"})));
            }
            return Ok(fail(
                5,
                format!(
                    "outbox dedup conflict: ({repo}, {dedup_key}) already holds entry {} with a different payload",
                    row.get("id").and_then(|v| v.as_str()).unwrap_or("?")
                ),
            ));
        }
        let now = now_unix();
        let id = format!(
            "ob-{}-{}",
            now,
            &crate::creation_outbox::sha256_hex(format!("{repo}\n{dedup_key}\n{hash}").as_bytes())
                [..12]
        );
        t.exec(
            "INSERT INTO creation_outbox (id, repo, kind, payload, dedup_key, payload_hash, \
             producer, state, created_at, available_at, attempt_count, max_attempts) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, 0, ?)",
            &[
                Arg::Text(id.clone()),
                Arg::Text(repo.to_string()),
                Arg::Text(kind.to_string()),
                Arg::Text(payload.to_string()),
                Arg::Text(dedup_key.to_string()),
                Arg::Text(hash),
                arg_str(req_str(req, "producer")),
                Arg::Text(iso(now)),
                Arg::Text(iso(now)),
                Arg::Int(crate::creation_outbox::DEFAULT_MAX_ATTEMPTS),
            ],
        )?;
        let row = outbox_get(t, &id)?.unwrap_or(json!({}));
        Ok(ok(json!({"row": row, "outcome": "created"})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Claim the oldest due entry for a drainer: pending-and-available,
/// an expired lease (crashed drainer), or an uncertain entry whose
/// reconciliation delay has elapsed (claimed AS uncertain — the
/// drainer must reconcile by marker before any re-create). Refuses
/// while the circuit is open or the rolling-hour creation budget
/// is spent; pacing (minimum gap between creates) is the drainer
/// client's side of the same policy.
fn verb_outbox_claim(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let owner = req_str(req, "owner").unwrap_or("outbox-drainer");
        let now = now_unix();
        let circuit = outbox_circuit(t)?;
        if circuit.is_open(now) {
            return Ok(ok(json!({
                "row": Value::Null,
                "circuit_open_until": iso(circuit.open_until),
            })));
        }
        let budget_rows = t.query_objs(
            "SELECT COUNT(*) AS n FROM creation_outbox \
             WHERE completed_at IS NOT NULL AND completed_at > ?",
            &[Arg::Text(iso(now - 3600))],
            &[],
        )?;
        let recent = budget_rows
            .into_iter()
            .next()
            .and_then(|r| r.get("n").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        if recent >= crate::creation_outbox::HOURLY_BUDGET as i64 {
            return Ok(ok(json!({"row": Value::Null, "budget_exhausted": true})));
        }
        let rows = t.query_objs(
            &format!(
                "SELECT {OUTBOX_COLS} FROM creation_outbox \
                 WHERE (state = 'pending' AND available_at <= ?) \
                    OR (state = 'leased' AND lease_until IS NOT NULL AND lease_until <= ?) \
                    OR (state = 'uncertain' AND available_at <= ?) \
                 ORDER BY created_at ASC LIMIT 1"
            ),
            &[
                Arg::Text(iso(now)),
                Arg::Text(iso(now)),
                Arg::Text(iso(now)),
            ],
            &["payload"],
        )?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(ok(json!({"row": Value::Null})));
        };
        let id = row
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let state = row
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if state == "leased" {
            // An expired lease: the previous drainer died
            // mid-flight and its create may have landed. The row
            // is claimed AS uncertain — the drainer reconciles by
            // marker before any re-create (kill/restart trial).
            t.exec(
                "UPDATE creation_outbox SET state = 'uncertain', available_at = ?, \
                 lease_owner = ?, lease_until = ? WHERE id = ?",
                &[
                    Arg::Text(iso(now)),
                    Arg::Text(owner.to_string()),
                    Arg::Text(iso(now + crate::creation_outbox::LEASE_SECONDS)),
                    Arg::Text(id.clone()),
                ],
            )?;
        } else if state != "uncertain" {
            t.exec(
                "UPDATE creation_outbox SET state = 'leased', lease_owner = ?, lease_until = ? \
                 WHERE id = ?",
                &[
                    Arg::Text(owner.to_string()),
                    Arg::Text(iso(now + crate::creation_outbox::LEASE_SECONDS)),
                    Arg::Text(id.clone()),
                ],
            )?;
        } else {
            t.exec(
                "UPDATE creation_outbox SET lease_owner = ?, lease_until = ? WHERE id = ?",
                &[
                    Arg::Text(owner.to_string()),
                    Arg::Text(iso(now + crate::creation_outbox::LEASE_SECONDS)),
                    Arg::Text(id.clone()),
                ],
            )?;
        }
        let claimed = outbox_get(t, &id)?.unwrap_or(json!({}));
        Ok(ok(json!({"row": claimed})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Resolve a claimed entry. One verb, four outcomes — the SQL
/// spelling of the core drainer's transitions:
/// - `done` {github_number, github_url}: only with a verified
///   number/URL (the drainer read the object back first); a done
///   without both is refused (exit 2) — a gh exit 0 never marks done.
/// - `rate_limited` {retry_after?}: backoff per the core policy and
///   a strike on the persisted circuit.
/// - `uncertain` {error}: the silent-loss shape; the entry waits
///   for marker reconciliation, never a blind retry.
/// - `failed` {error}: backoff-retry until max_attempts, then blocked.
fn verb_outbox_resolve(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let id = req_str(req, "id").unwrap_or("");
        let outcome = req_str(req, "outcome").unwrap_or("");
        let Some(current) = outbox_get(t, id)? else {
            return Ok(fail(5, format!("outbox entry {id} not found")));
        };
        let now = now_unix();
        let attempts = current
            .get("attempt_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let max_attempts = current
            .get("max_attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(crate::creation_outbox::DEFAULT_MAX_ATTEMPTS);
        match outcome {
            "done" => {
                let number = req_i64(req, "github_number");
                let url = req_str(req, "github_url");
                let (Some(number), Some(url)) = (number, url) else {
                    return Ok(fail(
                        2,
                        "outbox done requires the read-back-verified github_number and github_url",
                    ));
                };
                t.exec(
                    "UPDATE creation_outbox SET state = 'done', attempt_count = attempt_count + 1, github_number = ?, github_url = ?, \
                     completed_at = ?, last_error_class = NULL, lease_owner = NULL, lease_until = NULL \
                     WHERE id = ?",
                    &[
                        Arg::Int(number),
                        Arg::Text(url.to_string()),
                        Arg::Text(iso(now)),
                        Arg::Text(id.to_string()),
                    ],
                )?;
                let mut circuit = outbox_circuit(t)?;
                circuit.record_success();
                outbox_circuit_save(t, &circuit)?;
            }
            "rate_limited" => {
                let mut circuit = outbox_circuit(t)?;
                circuit.record_rate_limit(now);
                outbox_circuit_save(t, &circuit)?;
                let delay = crate::creation_outbox::backoff_delay(
                    attempts + 1,
                    req_i64(req, "retry_after"),
                    outbox_jitter_unit(),
                );
                t.exec(
                    "UPDATE creation_outbox SET state = 'pending', attempt_count = attempt_count + 1, \
                     available_at = ?, last_error_class = 'rate_limit', \
                     lease_owner = NULL, lease_until = NULL WHERE id = ?",
                    &[Arg::Text(iso(now + delay)), Arg::Text(id.to_string())],
                )?;
            }
            "uncertain" => {
                let why = req_str(req, "error").unwrap_or("ambiguous create");
                t.exec(
                    "UPDATE creation_outbox SET state = 'uncertain', attempt_count = attempt_count + 1, \
                     available_at = ?, last_error_class = ?, lease_owner = NULL, lease_until = NULL \
                     WHERE id = ?",
                    &[
                        Arg::Text(iso(now + crate::creation_outbox::BACKOFF_BASE_SECONDS)),
                        Arg::Text(format!("ambiguous: {why}")),
                        Arg::Text(id.to_string()),
                    ],
                )?;
            }
            "failed" => {
                let why = req_str(req, "error").unwrap_or("create failed");
                if attempts + 1 >= max_attempts {
                    t.exec(
                        "UPDATE creation_outbox SET state = 'blocked', attempt_count = attempt_count + 1, \
                         last_error_class = ?, lease_owner = NULL, lease_until = NULL WHERE id = ?",
                        &[
                            Arg::Text(format!("failed: {why}")),
                            Arg::Text(id.to_string()),
                        ],
                    )?;
                } else {
                    let delay = crate::creation_outbox::backoff_delay(
                        attempts + 1,
                        None,
                        outbox_jitter_unit(),
                    );
                    t.exec(
                        "UPDATE creation_outbox SET state = 'pending', attempt_count = attempt_count + 1, \
                         available_at = ?, last_error_class = ?, lease_owner = NULL, lease_until = NULL \
                         WHERE id = ?",
                        &[
                            Arg::Text(iso(now + delay)),
                            Arg::Text(format!("failed: {why}")),
                            Arg::Text(id.to_string()),
                        ],
                    )?;
                }
            }
            other => {
                return Ok(fail(2, format!("unknown outbox outcome {other:?}")));
            }
        }
        let row = outbox_get(t, id)?.unwrap_or(json!({}));
        Ok(ok(json!({"row": row})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

fn verb_outbox_list(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let mut sql = format!("SELECT {OUTBOX_COLS} FROM creation_outbox WHERE 1 = 1");
        let mut args: Vec<Arg> = Vec::new();
        if let Some(repo) = req_str(req, "repo") {
            sql += " AND repo = ?";
            args.push(Arg::Text(repo.to_string()));
        }
        if let Some(state) = req_str(req, "state") {
            sql += " AND state = ?";
            args.push(Arg::Text(state.to_string()));
        }
        sql += " ORDER BY created_at ASC LIMIT 200";
        let rows = t.query_objs(&sql, &args, &["payload"])?;
        Ok(ok(json!({"rows": rows})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Metrics: per-state counts, oldest pending age, total attempts,
/// last failure class, and the circuit — the alert fields the
/// #3706 design requires (pending depth / oldest-pending-age).
fn verb_outbox_metrics(t: &TursoBackend, _req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(
            &format!("SELECT {OUTBOX_COLS} FROM creation_outbox"),
            &[],
            &["payload"],
        )?;
        let now = now_unix();
        let mut entries = Vec::new();
        for row in &rows {
            let parse_time = |key: &str| {
                row.get(key)
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_unix)
                    .unwrap_or(0)
            };
            entries.push(crate::creation_outbox::Entry {
                id: row
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                repo: row
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: row
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                payload: row.get("payload").cloned().unwrap_or(json!({})),
                dedup_key: row
                    .get("dedup_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                payload_hash: row
                    .get("payload_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                producer: row
                    .get("producer")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                state: row
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                created_at: parse_time("created_at"),
                available_at: parse_time("available_at"),
                attempt_count: row
                    .get("attempt_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                max_attempts: row
                    .get("max_attempts")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                last_error_class: row
                    .get("last_error_class")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                lease_owner: row
                    .get("lease_owner")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                lease_until: row
                    .get("lease_until")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_unix),
                github_number: row.get("github_number").and_then(|v| v.as_i64()),
                github_url: row
                    .get("github_url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                completed_at: row
                    .get("completed_at")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_unix),
            });
        }
        let circuit = outbox_circuit(t)?;
        let m = crate::creation_outbox::metrics_of(&entries, &circuit, now);
        Ok(ok(json!({"row": {
            "pending": m.pending,
            "leased": m.leased,
            "uncertain": m.uncertain,
            "done": m.done,
            "blocked": m.blocked,
            "oldest_pending_age_seconds": m.oldest_pending_age_seconds,
            "total_attempts": m.total_attempts,
            "last_error_class": m.last_error_class,
            "circuit_open_until": m.circuit_open_until.map(iso),
        }})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}
/// The Turso backend's half of the broker dispatch: every verb the
/// Postgres path serves except /validate (purely local) and
/// /healthz (transport-level), which stay in queue_broker::dispatch.
pub fn dispatch(t: &TursoBackend, path: &str, req: &Value) -> Value {
    match path {
        "/enqueue" => verb_enqueue(t, req),
        "/revive" => verb_revive(t, req),
        "/claim" => verb_claim(t, req),
        "/claim-flow" => verb_claim_flow(t, req),
        "/lane" => verb_lane(t, req),
        "/bind" => verb_bind(t, req),
        "/heartbeat" => verb_heartbeat(t, req),
        "/checkpoint" => verb_checkpoint(t, req),
        "/park" => verb_park(t, req),
        "/release" => verb_release(t, req),
        "/wake" => verb_wake(t, req),
        "/block" => verb_block(t, req),
        "/unblock" => verb_unblock(t, req),
        "/dep-recheck" => verb_dep_recheck(t, req),
        "/cancel" => verb_cancel(t, req),
        "/fail" => verb_fail(t, req),
        "/complete" => verb_complete(t, req),
        "/list" => verb_list(t, req),
        "/health" => verb_health(t, req),
        "/signal-count" => verb_signal_count(t, req),
        "/events-append" => verb_events_append(t, req),
        "/events-seen" => verb_events_seen(t, req),
        "/events-count" => verb_events_count(t, req),
        "/events-prune" => verb_events_prune(t, req),
        "/outbox-enqueue" => verb_outbox_enqueue(t, req),
        "/outbox-claim" => verb_outbox_claim(t, req),
        "/outbox-resolve" => verb_outbox_resolve(t, req),
        "/outbox-list" => verb_outbox_list(t, req),
        "/outbox-metrics" => verb_outbox_metrics(t, req),
        "/sweep" => verb_sweep(t, req),
        "/coverage" => verb_coverage(t, req),
        _ => fail(2, format!("unknown command {path}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn claim_lease_is_the_requested_lease_not_the_heartbeat_default() {
        // #7617 oracle: a claim's stored lease_until is claimed_at +
        // the lease the caller requested (2700 s for a 45-minute
        // claim), never claimed_at + the 900 s heartbeat extension
        // default. Task 11049 (2026-11-08) reported the former and
        // died at the latter.
        let claimed = 1_794_020_400; // 2026-11-07T03:00:00Z
        assert_eq!(claim_lease_until(claimed, 2700), "2026-11-07T03:45:00Z");
        assert_eq!(
            claim_lease_until(claimed, CLAIM_DEFAULT_LEASE_SECONDS),
            claim_lease_until(claimed, 2700)
        );
        assert_ne!(
            claim_lease_until(claimed, CLAIM_DEFAULT_LEASE_SECONDS),
            iso(claimed + HEARTBEAT_DEFAULT_EXTEND_SECONDS)
        );
        assert_eq!(CLAIM_DEFAULT_LEASE_SECONDS, 2700);
        assert_eq!(HEARTBEAT_DEFAULT_EXTEND_SECONDS, 900);
    }

    #[test]
    fn claim_lane_argument_is_stored_verbatim() {
        // #7617 / #7598 oracle: the lane passed at claim is the lane
        // stored on the task row — the claim writes lane_id through
        // arg_str(lane) with no substitution, and a lane-less claim
        // stores NULL rather than some other lane's id.
        match arg_str(Some("code-u")) {
            Arg::Text(v) => assert_eq!(v, "code-u"),
            _ => panic!("a passed lane must reach the UPDATE as text"),
        }
        match arg_str(None) {
            Arg::Null => {}
            _ => panic!("a lane-less claim must store NULL lane_id"),
        }
    }

    #[test]
    fn claim_update_sql_rechecks_pick_eligibility_for_the_picked_id() {
        // #7068 oracle, statement shape: the claim UPDATE is keyed
        // to the picked row's id and re-checks the full pick
        // predicate, so when two lanes pick the same row in the
        // same instant the loser's UPDATE matches 0 rows (the
        // winner already flipped the row to 'claimed', which the
        // predicate excludes) and the claim loop re-picks instead
        // of handing the loser the winner's row.
        let predicate = "(status = 'queued' AND attempts < max_attempts \
             AND (not_before IS NULL OR not_before <= ?)) \
           OR (status = 'parked' AND not_before IS NOT NULL AND not_before <= ?)";
        let sql = claim_update_sql(predicate);
        assert!(sql.starts_with("UPDATE tasks SET status = 'claimed'"));
        assert!(sql.contains("WHERE id = ?"), "claim is keyed by id: {sql}");
        assert!(
            sql.contains("status = 'queued'"),
            "predicate re-checked: {sql}"
        );
        assert!(
            sql.contains("status = 'parked'"),
            "predicate re-checked: {sql}"
        );
        assert!(
            sql.contains("attempts < max_attempts"),
            "cap re-checked: {sql}"
        );
        assert!(
            sql.contains("lane_id = ?"),
            "winner's lane stamp is the claim's own: {sql}"
        );
    }

    #[test]
    fn claim_race_two_lanes_exactly_one_winner_per_task() {
        // #7068 fixture (task 11895, lanes code-m/code-f,
        // 2026-11-08): two lanes with distinct lane ids race one
        // queued task in the same queue. Exactly one claim wins;
        // the loser's eligibility view of that row closes the
        // instant the winner's claim lands, and the next queued
        // row stays claimable so the loser moves on to it (or to
        // a no-task answer) instead of receiving the winner's row.
        let now = "2026-11-08T02:32:59Z";
        let mut task = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        // Both lanes see the row as claimable before the race.
        assert!(by_id_claimable(&task, now));
        // Lane code-f wins: its claim flips the row and stamps
        // code-f's lane id (the state lane code-m's losing claim
        // call observed on task 11895).
        task["status"] = json!("claimed");
        task["lane_id"] = json!("code-f");
        task["worker_id"] = json!("repo-worker-code");
        // The loser (code-m, same shared worker id) can no longer
        // claim this row by id, and the pick predicate's status
        // terms ('queued'/'parked') no longer match it either.
        assert!(!by_id_claimable(&task, now));
        // The next eligible task remains claimable for the loser.
        let next = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        assert!(by_id_claimable(&next, now));
    }

    #[test]
    fn claim_paths_never_reread_a_workers_newest_claimed_row() {
        // #7068 source oracle: the pre-#7617 claim shape claimed
        // via a subquery and re-read the result as the newest
        // claimed row for (queue, worker_id) — with every lane
        // sharing worker_id repo-worker-code, a losing claim was
        // handed another lane's just-claimed task, lane stamp and
        // all. That re-read must never come back: both claim paths
        // (pick and by-id) read the claimed row back by its own id.
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        let code: String = impl_src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "ORDER BY claimed_at",
            "WHERE queue = ? AND worker_id",
            "worker_id = ? ORDER BY",
        ] {
            assert!(
                !code.contains(forbidden),
                "claim path must not re-read by worker: found {forbidden}"
            );
        }
        assert!(
            code.contains("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
            "claim read-back is by the claimed row's own id"
        );
        assert!(
            code.contains("claim_update_sql(&pick_predicate)"),
            "pick path claims through the id-keyed conditional UPDATE"
        );
    }

    #[test]
    fn claim_model_stamp_requires_provider_and_model_together() {
        // #8176 oracle: the stamp names the serving leaf — provider
        // and model together, variant optional. Naming either half
        // alone (or a variant alone) is an error, so a half-stamp
        // can never land on a task row.
        let full = json!({
            "model_provider": "_claude", "model_id": "claude-sonnet-5-5",
            "model_variant": "medium",
        });
        let parsed = parse_claim_model(&full).expect("full stamp parses");
        assert_eq!(
            parsed,
            Some(ClaimModel {
                provider: "_claude".to_string(),
                model: Some("claude-sonnet-5-5".to_string()),
                variant: Some("medium".to_string()),
            })
        );
        let no_variant = json!({
            "model_provider": "openai.main", "model_id": "gpt-6-luna",
        });
        assert_eq!(
            parse_claim_model(&no_variant)
                .expect("variant-less stamp parses")
                .and_then(|m| m.variant),
            None
        );
        assert!(
            parse_claim_model(&json!({}))
                .expect("empty claim")
                .is_none()
        );
        assert!(parse_claim_model(&json!({"model_provider": "_claude"})).is_err());
        assert!(parse_claim_model(&json!({"model_id": "claude-sonnet-5-5"})).is_err());
        assert!(parse_claim_model(&json!({"model_variant": "medium"})).is_err());
        // The non-session marker (Muse lanes) carries a provider
        // and no model — parse keeps it distinct from a leaf.
        let marker = json!({"model_provider": "muse-lane", "model_id": "muse-lane"});
        assert!(parse_claim_model(&marker).is_ok());
    }

    #[test]
    fn claim_model_columns_are_in_the_task_read_and_schema() {
        // #8176 source oracle: the stamp columns are part of the
        // task read surface (TASK_COLS), the fresh schema, and the
        // additive migration, so a claimed row reads its stamp back
        // through every verb that returns tasks.
        assert!(TASK_COLS.contains("claim_provider"));
        assert!(TASK_COLS.contains("claim_model"));
        assert!(TASK_COLS.contains("claim_variant"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("claim_provider TEXT"));
        assert!(schema.contains("claim_model TEXT"));
        assert!(schema.contains("claim_variant TEXT"));
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("\"claim_provider\""));
        assert!(impl_src.contains("stamp_claim_model"));
    }

    fn iso_epoch_and_known_instants() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        // 2026-11-07T03:00:00Z == 1794020400 (the night this landed).
        assert_eq!(iso(1_794_020_400), "2026-11-07T03:00:00Z");
        assert_eq!(iso(1_794_020_400 + 2700), "2026-11-07T03:45:00Z");
    }

    #[test]
    fn iso_strings_sort_chronologically() {
        let mut v: Vec<String> = [100_000, 0, 1_794_020_400, 86_400, 999_999_999]
            .iter()
            .map(|n| iso(*n))
            .collect();
        let mut sorted = v.clone();
        sorted.sort();
        v.sort_by_key(|s| s.clone());
        assert_eq!(v, sorted);
        let mut by_str = vec![iso(200), iso(100), iso(300)];
        by_str.sort();
        assert_eq!(by_str, vec![iso(100), iso(200), iso(300)]);
    }

    #[test]
    #[test]
    fn wait_floors_match_broker_policy_8184() {
        // #8184: triage waits are held 6h, ci/capacity 1h; waits
        // with an owner-chosen horizon are unfloored.
        assert_eq!(wait_floor_seconds("triage"), Some(21_600));
        assert_eq!(wait_floor_seconds("ci"), Some(3_600));
        assert_eq!(wait_floor_seconds("capacity"), Some(3_600));
        assert_eq!(wait_floor_seconds("timer"), None);
        assert_eq!(wait_floor_seconds("review"), None);
        assert_eq!(wait_floor_seconds("external"), None);
    }

    #[test]
    fn apply_wait_floor_bumps_short_waits_only_8184() {
        // A long-past requested wake is bumped to the floor and
        // becomes relative; a far-future one stands as absolute.
        let (nb, kind) = apply_wait_floor("triage", "2000-01-01T00:00:00Z".to_string(), "absolute");
        assert_eq!(kind, "relative");
        assert!(nb > "2000-01-01T00:00:00Z".to_string());
        let (nb, kind) = apply_wait_floor("triage", "2999-01-01T00:00:00Z".to_string(), "absolute");
        assert_eq!(nb, "2999-01-01T00:00:00Z");
        assert_eq!(kind, "absolute");
        // Unfloored kinds never move.
        let (nb, kind) = apply_wait_floor("timer", "2000-01-01T00:00:00Z".to_string(), "absolute");
        assert_eq!(nb, "2000-01-01T00:00:00Z");
        assert_eq!(kind, "absolute");
    }

    #[test]
    fn default_pick_drains_parked_handoff_sink_8187() {
        // #8187 (amending #8184): the lane pick ADMITS recorded
        // opencode handoffs (the handoff sink) through a third
        // branch with NO not_before gate, so lanes can claim
        // stranded sink tasks and work them direct. #8234: the sink
        // is its own status ('handed_off'), so the timed parked
        // branch no longer needs its NOT sink exclusion — a sink
        // row cannot be 'parked' — and the sink branch keys on the
        // status, never on a reconstructed stage (#8192). The
        // queued branch keeps its attempts cap, and the sink branch
        // adds no placeholder: the predicate still carries exactly
        // the two `now` arguments the pick SELECT and claim UPDATE
        // bind.
        let pred = default_pick_predicate();
        assert!(pred.contains("status = 'queued' AND attempts < max_attempts"));
        assert!(pred.contains("status = 'parked' AND not_before IS NOT NULL AND not_before <= ?"));
        assert!(!pred.contains("NOT ("), "no sink exclusion since #8234: {pred}");
        assert!(pred.contains("status = 'handed_off'"));
        assert!(pred.contains("json_extract(progress, '$.lane') = 'opencode'"));
        assert!(
            !pred.contains("last_checkpoint"),
            "no stage reconstruction in the pick (#8192): {pred}"
        );
        // Third branch: the full sink predicate, ungated.
        let sink_branch = format!("OR ({HANDOFF_SQL})");
        assert!(pred.contains(&sink_branch), "sink branch present: {pred}");
        assert_eq!(pred.matches("not_before <= ?").count(), 2);
        assert_eq!(pred.matches('?').count(), 2, "no new placeholders: {pred}");
        // And the claim UPDATE re-check carries the same sink branch,
        // and collecting a sink row costs no attempt (#8234).
        let upd = claim_update_sql(&pred);
        assert!(upd.contains(&sink_branch));
        assert!(upd.contains("status IN ('parked', 'handed_off')"));
        assert_eq!(upd.matches("not_before <= ?").count(), 2);
    }

    #[test]
    fn fresh_lane_assignment_defaults_direct_8187() {
        // #8187: no reason -> direct (the claiming session owns the
        // task); NO balancing, NO opencode routing by default.
        assert_eq!(lane_decision(None, None), ("direct", "default_direct"));
        // --exclusion-reason forces direct, as before.
        assert_eq!(
            lane_decision(Some("claude-only work"), None),
            ("direct", "exclusion")
        );
        // --opencode-reason is the only door to opencode, recorded.
        assert_eq!(
            lane_decision(None, Some("needs opencode session")),
            ("opencode", "opencode_reason")
        );
        // Both passed: exclusion (the must-NOT-go-to-opencode
        // contract) wins over the opencode reason.
        assert_eq!(
            lane_decision(Some("no opencode"), Some("wants opencode")),
            ("direct", "exclusion")
        );
    }

    #[test]
    fn lane_reassignment_flips_only_unbound_opencode_with_exclusion_8187() {
        // Sink/legacy opencode row + exclusion -> flip to direct.
        assert!(lane_reassignment_flip(
            "opencode",
            None,
            Some("lane takes it direct")
        ));
        assert!(lane_reassignment_flip("opencode", Some(""), Some("x")));
        // No exclusion: claimant flows keep their assignment.
        assert!(!lane_reassignment_flip("opencode", None, None));
        // A session-bound row NEVER flips, exclusion or not.
        assert!(!lane_reassignment_flip(
            "opencode",
            Some("ses_123"),
            Some("x")
        ));
        assert!(!lane_reassignment_flip("opencode", Some("ses_123"), None));
        // Direct rows are already direct; nothing to flip.
        assert!(!lane_reassignment_flip("direct", None, Some("x")));
        assert!(!lane_reassignment_flip("direct", None, None));
    }

    #[test]
    fn lane_flip_to_opencode_only_unbound_direct_with_reason_8210() {
        // #8210: a claimed task's default direct ledger row flips
        // to opencode only with a stated reason and no bound
        // session — the send path's assignment step.
        assert!(lane_flip_to_opencode(
            "direct",
            None,
            Some("sender at time limit; opencode continues")
        ));
        assert!(lane_flip_to_opencode("direct", Some(""), Some("reason")));
        // No reason (or a blank one): no flip — opencode routing
        // stays reason-gated (#8187).
        assert!(!lane_flip_to_opencode("direct", None, None));
        assert!(!lane_flip_to_opencode("direct", None, Some("  ")));
        // A bound row never flips, either direction.
        assert!(!lane_flip_to_opencode("direct", Some("ses_1"), Some("reason")));
        // Opencode rows are not this flip's business.
        assert!(!lane_flip_to_opencode("opencode", None, Some("reason")));
    }

    fn transfer_row(status: &str, progress: Value, lease_until: &str) -> Value {
        let mut row = claimable_row(valid_payload());
        row["status"] = json!(status);
        row["progress"] = progress;
        row["lease_until"] = json!(lease_until);
        row["worker_id"] = json!("repo-worker-code-m");
        row["attempts"] = json!(2);
        row["max_attempts"] = json!(4);
        row
    }

    #[test]
    fn transfer_accepts_each_holder_session_mark_8210() {
        let now = "2026-11-10T20:00:00Z";
        let live = "2999-01-01T00:00:00Z";
        // Holder mark on the latest claimed event (a session that
        // claimed with --session-id).
        let row = transfer_row("claimed", json!({"lane": "direct"}), live);
        assert_eq!(
            transfer_refusal(&row, "ses_muse_1", Some("ses_muse_1"), None, Some("ses_new"), now),
            None
        );
        // Holder mark checkpointed into progress.from_session.
        let row = transfer_row(
            "claimed",
            json!({"lane": "direct", "from_session": "ses_muse_2"}),
            live,
        );
        assert_eq!(
            transfer_refusal(&row, "ses_muse_2", None, None, Some("ses_new"), now),
            None
        );
        // A bound holder session passing its own task on: the
        // bound session IS the holder mark.
        let row = transfer_row(
            "claimed",
            json!({"lane": "opencode", "session_id": "ses_holder"}),
            live,
        );
        assert_eq!(
            transfer_refusal(&row, "ses_holder", None, Some("ses_holder"), Some("ses_new"), now),
            None
        );
    }

    #[test]
    fn transfer_refuses_poach_terminal_parked_expired_and_bound_8210() {
        let now = "2026-11-10T20:00:00Z";
        let live = "2999-01-01T00:00:00Z";
        // A random session cannot poach a claimed task by id:
        // its id matches no holder mark.
        let row = transfer_row("claimed", json!({"lane": "direct"}), live);
        let r = transfer_refusal(&row, "ses_random", Some("ses_muse_1"), None, Some("ses_new"), now)
            .expect("poach must refuse");
        assert!(r.contains("only the current holder"), "{r}");
        // No holder mark anywhere: nobody can give the task away.
        assert!(transfer_refusal(&row, "ses_muse_1", None, None, Some("ses_new"), now).is_some());
        // Terminal states refuse.
        for st in ["done", "cancelled", "failed"] {
            let row = transfer_row(st, json!({"from_session": "ses_m"}), live);
            assert!(transfer_refusal(&row, "ses_m", None, None, None, now).is_some(), "{st}");
        }
        // Queued and parked rows are not transfer sources (a
        // genuine wait-park stays excluded, as by-id claim has it).
        for st in ["queued", "parked", "blocked"] {
            let row = transfer_row(st, json!({"from_session": "ses_m"}), live);
            assert!(transfer_refusal(&row, "ses_m", None, None, None, now).is_some(), "{st}");
        }
        // A dead lease is not a live claim.
        let row = transfer_row(
            "claimed",
            json!({"from_session": "ses_m"}),
            "2000-01-01T00:00:00Z",
        );
        assert!(transfer_refusal(&row, "ses_m", None, None, None, now).is_some());
        // Bound to a third session (neither passer nor claimer):
        // a live launch, not a transfer.
        let row = transfer_row(
            "claimed",
            json!({"from_session": "ses_m", "session_id": "ses_third"}),
            live,
        );
        let r = transfer_refusal(&row, "ses_m", None, Some("ses_third"), Some("ses_new"), now)
            .expect("third-party binding must refuse");
        assert!(r.contains("already bound"), "{r}");
    }

    #[test]
    fn transfer_update_is_one_atomic_claimed_to_claimed_move_8210() {
        // #8210: the whole transfer is ONE conditional UPDATE —
        // status stays claimed, attempts untouched, and the WHERE
        // re-checks status + the holder worker so a lost race
        // affects 0 rows and the original claim stands.
        let sql = transfer_update_sql();
        assert!(sql.contains("status = 'claimed'"), "{sql}");
        assert!(sql.contains("worker_id = ?"), "{sql}");
        assert!(!sql.contains("attempts"), "{sql}");
        // Source oracles: the transfer path, its event marks, and
        // the client flag all exist.
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("claim_repo_task_transfer"));
        assert!(impl_src.contains("\"transfer\": true"));
        assert!(impl_src.contains("\"from_session\": from_session"));
        assert!(impl_src.contains("\"previous_worker\": old_worker"));
        let client_src = include_str!("client.rs");
        assert!(client_src.contains("\"from-session\""));
        assert!(client_src.contains("body[\"from_session\"] = json!(fs)"));
    }

    #[test]
    fn platform_limit_rules_8210() {
        // No limitation: open default, either family may claim,
        // and nothing is invalid or refused.
        assert!(!platform_limit_invalid(None, None));
        assert_eq!(platform_limit_error(None, None), None);
        assert_eq!(platform_limit_refusal(None, None, Some("muse")), None);
        assert_eq!(platform_limit_refusal(None, None, Some("openchamber")), None);
        assert_eq!(platform_limit_refusal(None, None, None), None);
        // Recording one demands a real family AND a reason.
        assert!(platform_limit_error(Some("muse"), Some("needs claude-only context")).is_none());
        let e = platform_limit_error(Some("muse"), None)
            .expect("a limitation without a reason is refused at the verb");
        assert!(e.contains("requires a recorded reason"), "{e}");
        assert!(platform_limit_error(Some("muse"), Some("  ")).is_some());
        assert!(platform_limit_error(Some("bogus"), Some("reason")).is_some());
        assert!(platform_limit_error(None, Some("orphan reason")).is_some());
        // A stored limitation with no reason is invalid: never
        // enforced (the transfer path clears it instead).
        assert!(platform_limit_invalid(Some("muse"), None));
        assert!(platform_limit_invalid(Some("muse"), Some(" ")));
        assert!(!platform_limit_invalid(Some("muse"), Some("real reason")));
        assert_eq!(
            platform_limit_refusal(Some("muse"), None, Some("openchamber")),
            None
        );
        // A valid limitation binds transfers: the limited family
        // passes, every other family refuses — naming the
        // limitation and its reason — and an undeclared family
        // cannot take a limited task.
        let reason = Some("needs claude-only context window");
        assert_eq!(platform_limit_refusal(Some("muse"), reason, Some("muse")), None);
        let r = platform_limit_refusal(Some("muse"), reason, Some("openchamber"))
            .expect("muse-limited task refuses an opencode transfer");
        assert!(r.contains("limited to platform 'muse'"), "{r}");
        assert!(r.contains("needs claude-only context window"), "{r}");
        let r = platform_limit_refusal(Some("openchamber"), reason, Some("muse"))
            .expect("opencode-limited task refuses a muse transfer");
        assert!(r.contains("limited to platform 'openchamber'"), "{r}");
        assert!(platform_limit_refusal(Some("muse"), reason, None).is_some());
        // Source oracles: columns, migration, event fields, and
        // the client flags all exist.
        assert!(LEDGER_COLS.contains("platform_limit"));
        assert!(LEDGER_COLS.contains("platform_limit_reason"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("platform_limit TEXT"));
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("ADD COLUMN {col} TEXT"));
        assert!(impl_src.contains("platform_limit_cleared_invalid"));
        assert!(impl_src.contains("\"platform_limit_cleared\": true"));
        let client_src = include_str!("client.rs");
        assert!(client_src.contains("\"limit-platform\""));
        assert!(client_src.contains("\"platform_limit\": args.get(\"limit-platform\")"));
        assert!(client_src.contains("\"clear-platform-limit\""));
    }

    #[test]
    fn opencode_reason_is_recorded_on_the_ledger_8187() {
        // Source oracle: the reason column exists on the read
        // surface, the fresh schema, the additive migration, and
        // the fresh-assignment INSERT; the client exposes the flag.
        assert!(LEDGER_COLS.contains("opencode_reason"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("opencode_reason TEXT"));
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(
            impl_src.contains("ALTER TABLE repo_task_lane_ledger ADD COLUMN opencode_reason TEXT")
        );
        assert!(impl_src.contains("assigned_lane, exclusion_reason, opencode_reason, assigned_by"));
        assert!(impl_src.contains("\"reason\": reason"));
        assert!(impl_src.contains("\"reassignment\": true"));
        // No balancing artifact survives in the lane decision path.
        assert!(!impl_src.contains("opencode_count"));
        assert!(!impl_src.contains("o_count <= d_count"));
        let client_src = include_str!("client.rs");
        assert!(client_src.contains("\"opencode-reason\""));
        assert!(client_src.contains("\"opencode_reason\": args.get(\"opencode-reason\")"));
    }

    #[test]
    fn platform_is_a_mark_not_a_lane_input_8187() {
        // #8187 platform audit: tasks.platform holds only the two
        // platform marks and lane_decision takes no platform input
        // (its signature is exclusion + opencode_reason only), so a
        // platform selector can never route a task to opencode.
        assert_eq!(PLATFORM_FAMILIES, ["openchamber", "muse"]);
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        let lane_fn = impl_src
            .split("fn verb_lane")
            .nth(1)
            .expect("verb_lane exists")
            .split("fn verb_bind")
            .next()
            .unwrap_or("");
        // #8210 amendment: verb_lane now also records a platform
        // LIMITATION (platform_limit / platform_limit_reason
        // request fields, written to the lane ledger with their
        // recorded reason) — that is not a read of the mark. The
        // invariant that stands: verb_lane never reads the task
        // row's platform FIELD (the mark) as an input.
        assert!(
            !lane_fn.contains("get(\"platform\")"),
            "verb_lane must not read tasks.platform: {lane_fn}"
        );
    }

    #[test]
    fn expired_claims_return_handoffs_to_sink_only_8184() {
        // #8184: expiry of a claimed recorded handoff returns to
        // the parked sink (no attempt burn); anything else queues.
        assert!(expired_claim_returns_to_sink(
            &json!({"lane": "opencode", "stage": "handed_off"})
        ));
        assert!(expired_claim_returns_to_sink(
            &json!({"lane": "opencode", "stage": "launch_failed"})
        ));
        assert!(expired_claim_returns_to_sink(&json!({
            "lane": "opencode",
            "last_checkpoint": {"stage": "handed_off"}
        })));
        assert!(!expired_claim_returns_to_sink(
            &json!({"lane": "direct", "stage": "handed_off"})
        ));
        assert!(!expired_claim_returns_to_sink(&json!({
            "lane": "opencode",
            "stage": "implementation_complete"
        })));
        assert!(!expired_claim_returns_to_sink(&json!({})));
    }

    #[test]
    fn wake_eligibility_is_parked_only_8184() {
        assert!(wake_eligible("parked").is_ok());
        assert!(wake_eligible("queued").is_err());
        assert!(wake_eligible("claimed").is_err());
        assert!(wake_eligible("done").is_err());
        // #8234: a sink row is refused wake with the reason — it is
        // not a wait at all, it is already claimable.
        let err = wake_eligible("handed_off").unwrap_err();
        assert!(err.contains("already claimable"), "{err}");
    }

    #[test]
    fn wait_kind_mapping_matches_pg_park() {
        assert_eq!(wait_kind(&json!({"kind": "triage"})), "triage");
        assert_eq!(wait_kind(&json!({"kind": "time"})), "timer");
        assert_eq!(wait_kind(&json!({"kind": "pr_merged"})), "ci");
        assert_eq!(wait_kind(&json!({"kind": "person_answer"})), "review");
        assert_eq!(wait_kind(&json!({"kind": "quota_reset"})), "capacity");
        assert_eq!(wait_kind(&json!({"kind": "something_else"})), "external");
        assert_eq!(wait_kind(&json!({})), "external");
    }

    #[test]
    fn clear_binding_stamps_and_strips() {
        let p = json!({"session_id": "s1", "launched_by": "x", "keep": 1});
        let (out, cleared) = clear_binding(&p, "expire_leases");
        assert_eq!(cleared.as_deref(), Some("s1"));
        assert!(out.get("session_id").is_none());
        assert!(out.get("launched_by").is_none());
        assert_eq!(out.get("keep"), Some(&json!(1)));
        assert_eq!(
            out.get("session_cleared").and_then(|c| c.get("via")),
            Some(&json!("expire_leases"))
        );
        let (out2, cleared2) = clear_binding(&json!({"keep": 1}), "x");
        assert!(cleared2.is_none());
        assert_eq!(out2, json!({"keep": 1}));
    }

    #[test]
    fn event_append_requires_a_kind() {
        assert!(parse_event_append(&json!({})).is_err());
        assert!(parse_event_append(&json!({"kind": "  "})).is_err());
        let (w, tid, kind, detail) =
            parse_event_append(&json!({"kind": "push", "worker_id": "github-webhook"})).unwrap();
        assert_eq!(kind, "push");
        assert_eq!(w.as_deref(), Some("github-webhook"));
        assert_eq!(tid, None);
        assert_eq!(detail, json!({}));
        let (_, tid2, _, _) =
            parse_event_append(&json!({"kind": "claimed", "task_id": 42, "detail": {"a": 1}}))
                .unwrap();
        assert_eq!(tid2, Some(42));
    }

    #[test]
    fn prune_cutoff_clamps_and_subtracts_days() {
        let now = 1_794_020_400; // 2026-11-07T03:00:00Z
        assert_eq!(prune_cutoff(now, 30), iso(now - 30 * 86_400));
        assert_eq!(prune_cutoff(now, 0), iso(now - 86_400));
        assert_eq!(prune_cutoff(now, 99_999), iso(now - 3650 * 86_400));
    }

    #[test]
    fn revalidate_outcome_matrix() {
        // Time parks are never released by the sweeper: elapsed time
        // is not evidence; they release through claim at not_before.
        assert_eq!(
            revalidate_outcome(Some("time"), None, None, false),
            "confirmed"
        );
        // No condition, or a kind the database cannot verify: stale,
        // so the park surfaces instead of sitting silently.
        assert_eq!(revalidate_outcome(None, None, None, false), "stale");
        assert_eq!(
            revalidate_outcome(Some("issue_closed"), None, None, false),
            "stale"
        );
        assert_eq!(
            revalidate_outcome(Some("person_answer"), None, None, false),
            "stale"
        );
        // task_terminal releases on done/cancelled only.
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), Some("done"), None, false),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), Some("cancelled"), None, false),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), Some("queued"), None, false),
            "confirmed"
        );
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), None, None, false),
            "stale"
        );
        // task_status releases on the exact wanted status.
        assert_eq!(
            revalidate_outcome(Some("task_status"), Some("blocked"), Some("blocked"), false),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("task_status"), Some("queued"), Some("blocked"), false),
            "confirmed"
        );
        assert_eq!(
            revalidate_outcome(Some("task_status"), None, Some("blocked"), false),
            "stale"
        );
        // event_seen releases when the event exists for the task.
        assert_eq!(
            revalidate_outcome(Some("event_seen"), None, None, true),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("event_seen"), None, None, false),
            "confirmed"
        );
    }

    #[test]
    fn dedupe_partition_live_hit_wins_and_terminal_is_newest() {
        let rows = vec![
            json!({"id": 1, "status": "done"}),
            json!({"id": 2, "status": "cancelled"}),
        ];
        let (live, terminal) = dedupe_partition(&rows);
        assert!(live.is_none());
        // The newest terminal row is the revive predecessor.
        assert_eq!(terminal.unwrap().get("id"), Some(&json!(2)));
        let rows = vec![
            json!({"id": 1, "status": "done"}),
            json!({"id": 3, "status": "parked"}),
            json!({"id": 4, "status": "failed"}),
        ];
        let (live, terminal) = dedupe_partition(&rows);
        assert_eq!(live.unwrap().get("id"), Some(&json!(3)));
        assert_eq!(terminal.unwrap().get("id"), Some(&json!(4)));
        let (live, terminal) = dedupe_partition(&[]);
        assert!(live.is_none() && terminal.is_none());
    }

    #[test]
    fn revive_eligibility_matches_6838() {
        // Failed rows can always be re-queued.
        assert!(revive_eligible("failed", 4, 4).is_ok());
        assert!(revive_eligible("failed", 1, 4).is_ok());
        // Queued rows only when dead at cap (the #6463 zombie state).
        assert!(revive_eligible("queued", 4, 4).is_ok());
        assert!(revive_eligible("queued", 3, 4).is_err());
        // Live work and done work are untouchable.
        assert!(revive_eligible("claimed", 4, 4).is_err());
        assert!(revive_eligible("done", 4, 4).is_err());
        assert!(revive_eligible("cancelled", 4, 4).is_err());
    }

    // ---- #8276 dependency blocking --------------------------------------

    #[test]
    fn dep_clamps_match_broker_policy_8276() {
        // Recheck: default 15 min, clamped to [5 min, 1 h].
        assert_eq!(clamp_dep_recheck_seconds(None), 900);
        assert_eq!(clamp_dep_recheck_seconds(Some(10)), 300);
        assert_eq!(clamp_dep_recheck_seconds(Some(1800)), 1800);
        assert_eq!(clamp_dep_recheck_seconds(Some(7200)), 3600);
        // Max TTL: default 3 days, floor 60 s (the live-demo
        // range), ceiling 7 days — nothing sits blocked forever.
        assert_eq!(clamp_dep_ttl_seconds(None), 259_200);
        assert_eq!(clamp_dep_ttl_seconds(Some(5)), 60);
        assert_eq!(clamp_dep_ttl_seconds(Some(3600)), 3600);
        assert_eq!(clamp_dep_ttl_seconds(Some(999_999_999)), 604_800);
    }

    #[test]
    fn dep_parse_validates_refs_8276() {
        let now = "2026-11-10T21:00:00Z";
        // No deps key (or an empty array) is the classic block.
        assert_eq!(parse_block_deps(&json!({}), "w", now).unwrap(), None);
        assert_eq!(
            parse_block_deps(&json!({"deps": []}), "w", now).unwrap(),
            None
        );
        let deps = parse_block_deps(
            &json!({"deps": [
                {"kind": "pr", "repo": "developing-today/code", "number": 3931, "reason": "gate"},
                {"kind": "issue", "repo": "developing-today/code", "number": 3901},
                {"kind": "task", "task_id": 5865},
                {"kind": "ci", "repo": "developing-today/code", "run_id": 12345},
            ]}),
            "lane-x",
            now,
        )
        .unwrap()
        .unwrap();
        assert_eq!(deps.len(), 4);
        assert_eq!(deps[0]["state"], json!("open"));
        assert_eq!(deps[0]["recorded_by"], json!("lane-x"));
        assert_eq!(deps[0]["reason"], json!("gate"));
        assert_eq!(dep_key(&deps[0]), "pr:developing-today/code#3931");
        assert_eq!(dep_key(&deps[1]), "issue:developing-today/code#3901");
        assert_eq!(dep_key(&deps[2]), "task:5865");
        assert_eq!(dep_key(&deps[3]), "ci:developing-today/code#12345");
        // A dep the broker cannot watch is a block-time error.
        assert!(parse_block_deps(
            &json!({"deps": [{"kind": "pr", "repo": "noslash", "number": 1}]}),
            "w", now,
        )
        .is_err());
        assert!(parse_block_deps(
            &json!({"deps": [{"kind": "pr", "repo": "a/b", "number": 0}]}),
            "w", now,
        )
        .is_err());
        assert!(parse_block_deps(
            &json!({"deps": [{"kind": "task", "task_id": -3}]}),
            "w", now,
        )
        .is_err());
        assert!(parse_block_deps(
            &json!({"deps": [{"kind": "ci", "repo": "a/b"}]}),
            "w", now,
        )
        .is_err());
        assert!(parse_block_deps(
            &json!({"deps": [{"kind": "milestone", "number": 1}]}),
            "w", now,
        )
        .is_err());
        assert!(parse_block_deps(&json!({"deps": "pr"}), "w", now).is_err());
    }

    #[test]
    fn task_dep_obs_matrix_8276() {
        // done clears; cancelled/failed release with the outcome
        // named; anything live holds; a missing task is broken.
        assert_eq!(
            task_dep_obs(Some("done"), 5),
            DepObs::Cleared("dep task done".to_string())
        );
        assert!(
            matches!(task_dep_obs(Some("cancelled"), 5), DepObs::Cleared(ref e) if e.contains("cancelled"))
        );
        assert!(
            matches!(task_dep_obs(Some("failed"), 5), DepObs::Cleared(ref e) if e.contains("failed"))
        );
        assert!(matches!(task_dep_obs(Some("claimed"), 5), DepObs::Open(_)));
        assert!(matches!(task_dep_obs(Some("queued"), 5), DepObs::Open(_)));
        assert!(matches!(task_dep_obs(Some("blocked"), 5), DepObs::Open(_)));
        assert!(matches!(task_dep_obs(None, 5), DepObs::Broken(_)));
    }

    #[test]
    fn dep_obs_transitions_and_all_must_clear_8276() {
        let now = "2026-11-10T21:00:00Z";
        let mut d = json!({"kind": "pr", "state": "open"});
        // A failed check (rate limit, token down) is NOT
        // clearance: no transition, dep stays open, the failure
        // is recorded as the last observation.
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Failed("github http 403".to_string()), now),
            None
        );
        assert_eq!(d["state"], json!("open"));
        assert_eq!(d["last_observed"], json!("github http 403"));
        // Still-open observation: no transition either.
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Open("pr open".to_string()), now),
            None
        );
        assert_eq!(d["state"], json!("open"));
        // Cleared transitions exactly once; a resolved dep is
        // terminal — later observations are no-ops.
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Cleared("pr merged".to_string()), now),
            Some("cleared")
        );
        assert_eq!(d["state"], json!("cleared"));
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Open("pr open".to_string()), now),
            None
        );
        assert_eq!(d["state"], json!("cleared"));
        // Broken releases too — a typo'd gate is not a life
        // sentence.
        let mut b = json!({"kind": "issue", "state": "open"});
        assert_eq!(
            apply_dep_obs(&mut b, &DepObs::Broken("github 404".to_string()), now),
            Some("broken")
        );
        // All-must-clear: one open dep holds the whole block.
        assert!(deps_all_resolved(&[d.clone(), b.clone()]));
        let mut held = vec![d.clone(), b.clone()];
        held.push(json!({"kind": "task", "state": "open"}));
        assert!(!deps_all_resolved(&held));
        assert!(!deps_all_resolved(&[]));
    }

    #[test]
    fn dep_wiring_oracles_8276() {
        // Schema, read surface, additive migration.
        assert!(TASK_COLS.contains("blocked_deps"));
        assert!(TASK_COLS.contains("dep_blocked_until"));
        assert!(TASK_COLS.contains("dep_next_recheck_at"));
        assert!(TASK_JSON_COLS.iter().any(|c| *c == "blocked_deps"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("blocked_deps TEXT"));
        assert!(schema.contains("dep_recheck_seconds INTEGER"));
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("ALTER TABLE tasks ADD COLUMN {col} {ty}"));
        // Routes, sweep ride, audit kinds, health bucket.
        assert!(impl_src.contains("\"/dep-recheck\" => verb_dep_recheck"));
        assert!(impl_src.contains("\"/unblock\" => verb_unblock"));
        assert!(impl_src.contains("run_dep_rechecks(t, req_i64(req, \"dep_limit\")"));
        assert!(impl_src.contains("\"dep_ttl_expired\""));
        assert!(impl_src.contains("\"dep_unblocked\""));
        assert!(impl_src.contains("\"dep_unblock_override\""));
        assert!(impl_src.contains("\"dep_blocked\": 0"));
        assert!(impl_src.contains("Some(until) if until < next.as_str()"));
        // The default pick has no blocked branch: dep-blocked
        // tasks (status 'blocked') are invisible to claims, so
        // no lane ever re-verifies a gate by claiming the item.
        let pred = default_pick_predicate();
        assert!(
            !pred.contains("blocked"),
            "pick must exclude blocked: {pred}"
        );
        // Client surface.
        let client_src = include_str!("client.rs");
        for f in [
            "blocked-by-pr",
            "blocked-by-issue",
            "blocked-by-task",
            "blocked-by-ci",
            "dep-recheck-seconds",
            "dep-max-ttl-seconds",
        ] {
            assert!(client_src.contains(f), "client flag {f}");
        }
        assert!(client_src.contains("\"dep-recheck\" => cmd_dep_recheck"));
        assert!(client_src.contains("\"unblock\" => cmd_unblock"));
        // The production probe is wired where the broker builds
        // its Turso backend, authenticated by secret name.
        let broker_src = include_str!("queue_broker.rs");
        assert!(broker_src.contains("GhDepProbe"));
        assert!(broker_src.contains("set_gh_probe"));
        assert!(broker_src.contains("github.developing-today-code-automation"));
    }

    #[test]
    fn health_entry_defaults_at_cap_queued_to_zero() {
        let e = health_entry("repo-health-code-pilot");
        assert_eq!(e.get("at_cap_queued"), Some(&json!(0)));
        assert_eq!(e.get("queued"), Some(&json!(0)));
        // #8234: the sink is its own health bucket — 'parked' in
        // health output means genuine waits only.
        assert_eq!(e.get("handed_off"), Some(&json!(0)));
    }

    #[test]
    fn at_cap_fold_covers_zero_claimable_and_partial_queues() {
        // Fixture for #6528 criterion 3: queue-dead has queued > 0
        // with EVERY queued task at cap (zero claimable); queue-live
        // has some at cap; queue-calm has none.
        let mut by_queue: Map<String, Value> = Map::new();
        for (q, queued) in [("queue-dead", 3), ("queue-live", 5), ("queue-calm", 2)] {
            let mut e = health_entry(q);
            e.as_object_mut()
                .unwrap()
                .insert("queued".to_string(), json!(queued));
            by_queue.insert(q.to_string(), e);
        }
        let rows = vec![
            json!({"queue": "queue-dead", "n": 3}),
            json!({"queue": "queue-live", "n": 2}),
        ];
        apply_at_cap_queued(&mut by_queue, &rows);
        let dead = &by_queue["queue-dead"];
        assert_eq!(dead.get("queued"), Some(&json!(3)));
        assert_eq!(dead.get("at_cap_queued"), Some(&json!(3)));
        // at_cap == queued means nothing in this queue can be claimed.
        assert_eq!(dead.get("at_cap_queued"), dead.get("queued"));
        assert_eq!(by_queue["queue-live"].get("at_cap_queued"), Some(&json!(2)));
        assert_eq!(by_queue["queue-calm"].get("at_cap_queued"), Some(&json!(0)));
    }

    #[test]
    fn merge_obj_is_shallow_and_object_only() {
        let out = merge_obj(&json!({"a": 1, "b": 2}), &json!({"b": 3, "c": 4}));
        assert_eq!(out, json!({"a": 1, "b": 3, "c": 4}));
        assert_eq!(merge_obj(&json!(null), &json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn completion_gate_fix_class_requires_evidence() {
        let fix = CompletionClass::Fix;
        assert!(gate_completion(&fix, &json!({})).is_err());
        assert!(gate_completion(&fix, &json!({"note": "did stuff"})).is_err());
        assert!(gate_completion(&fix, &json!({"evidence": ""})).is_err());
        assert!(gate_completion(&fix, &json!({"evidence": []})).is_err());
        assert!(
            gate_completion(
                &fix,
                &json!({"pr_url": "https://github.com/developing-today/code/pull/1"})
            )
            .is_ok()
        );
        assert!(gate_completion(&fix, &json!({"commit": "abc123"})).is_ok());
        assert!(
            gate_completion(
                &fix,
                &json!({"verified_closure": "closed by #1, verified on the issue"})
            )
            .is_ok()
        );
        assert!(gate_completion(&fix, &json!({"evidence": ["artifact"]})).is_ok());
    }

    #[test]
    fn completion_gate_triage_doctrine() {
        let triage = CompletionClass::Triage;
        // Default: the issue goes back in the queue as a follow-on.
        assert_eq!(
            gate_completion(&triage, &json!({})).unwrap(),
            Some(TriageOutcome::FollowOn)
        );
        assert_eq!(
            gate_completion(&triage, &json!({"triage_outcome": "follow_on"})).unwrap(),
            Some(TriageOutcome::FollowOn)
        );
        // Invalid is an exit only with the validation evidence.
        assert!(gate_completion(&triage, &json!({"triage_outcome": "invalid"})).is_err());
        assert_eq!(
            gate_completion(
                &triage,
                &json!({"triage_outcome": "invalid", "triage_note": "not a real issue: duplicate of #1"})
            )
            .unwrap(),
            Some(TriageOutcome::ExitInvalid)
        );
        // Already-solved needs a real evidence reference.
        assert!(gate_completion(&triage, &json!({"triage_outcome": "already_solved"})).is_err());
        assert_eq!(
            gate_completion(
                &triage,
                &json!({"triage_outcome": "already_solved", "pr_url": "https://x/pull/2"})
            )
            .unwrap(),
            Some(TriageOutcome::ExitAlreadySolved)
        );
        assert!(gate_completion(&triage, &json!({"triage_outcome": "done"})).is_err());
    }

    fn claimable_row(payload: Value) -> Value {
        json!({
            "id": 42,
            "queue": "repo-health-code-pilot",
            "title": "Fix #5444: claim payload guard",
            "body": "b",
            "priority": 30,
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:5444",
            "payload": payload,
        })
    }

    fn valid_payload() -> Value {
        json!({
            "schema": "repo-task/v1",
            "kind": "small_fix",
            "repo": "developing-today/code",
            "issue": {"number": 5444},
            "problem": "p",
            "priority_reason": "r",
            "evidence": ["e"],
            "acceptance": ["a"],
            "tests": ["t"],
            "context_refs": [],
            "scope": {"include": ["x"], "exclude": ["y"]},
            "risk": {"level": "low", "flags": []},
            "timebox_minutes": 45,
            "max_attempts": 4,
            "dedupe": {"checked_at": "2026-11-04T00:00:00Z", "basis": "search"},
        })
    }

    #[test]
    fn claim_guard_accepts_a_valid_packet() {
        // Oracle: validate_packet, the /enqueue validator — a row
        // enqueue would accept must pass the claim guard unchanged.
        assert_eq!(claim_guard_reason(&claimable_row(valid_payload())), None);
    }

    #[test]
    fn claim_guard_accepts_a_body_less_stored_row() {
        // task_from_packet stores an absent body as SQL NULL, so a
        // body-less packet's stored row carries body:null. The #6430
        // body type check must not quarantine it (Codex P1, PR #8477).
        let mut row = claimable_row(valid_payload());
        row["body"] = Value::Null;
        assert_eq!(claim_guard_reason(&row), None);
        let mut row = claimable_row(valid_payload());
        row.as_object_mut().unwrap().remove("body");
        assert_eq!(claim_guard_reason(&row), None);
    }

    #[test]
    fn claim_guard_rejects_payload_less_rows() {
        // The 2026-11-04 incident shapes: no payload field, a null
        // payload, and an empty-object payload must never be handed
        // out (#5444). Each is a quarantine, not a claim.
        let mut no_field = claimable_row(valid_payload());
        no_field.as_object_mut().unwrap().remove("payload");
        for row in [
            no_field,
            claimable_row(Value::Null),
            claimable_row(json!({})),
        ] {
            let reason = claim_guard_reason(&row).expect("payload-less row must be guarded");
            assert!(reason.contains("payload missing/empty"), "{reason}");
        }
    }

    #[test]
    fn claim_guard_rejects_schema_invalid_packets() {
        // A present-but-invalid packet (wrong schema, kind/family
        // mismatch) fails the same validator enqueue runs.
        let mut bad = valid_payload();
        bad["schema"] = json!("repo-task/v0");
        let reason =
            claim_guard_reason(&claimable_row(bad)).expect("schema-invalid packet must be guarded");
        assert!(reason.contains("packet failed validation"), "{reason}");
        // Invariant sweep: mutating any required identity field of a
        // valid packet to a placeholder must trip the guard — claim
        // never returns a row failing validate_packet.
        for mutated in [json!({"number": 0}), json!({"number": -1})] {
            let mut p = valid_payload();
            p["issue"] = mutated;
            assert!(claim_guard_reason(&claimable_row(p)).is_some());
        }
    }

    fn handoff_row(status: &str, progress: Value) -> Value {
        let mut row = claimable_row(valid_payload());
        row["status"] = json!(status);
        row["progress"] = progress;
        row["attempts"] = json!(1);
        row["max_attempts"] = json!(4);
        row["not_before"] = json!("2999-01-01T00:00:00Z");
        row
    }

    #[test]
    fn by_id_claim_takes_a_parked_recorded_handoff() {
        // The #7907/#6533 loop shape, post-#8234: opencode lane
        // assigned, yield recorded under last_checkpoint (the #7706
        // lift), no session bound — and the row RESTS IN THE SINK:
        // status 'handed_off', no wait fields. The not_before on
        // this fixture row is deliberately in the far future: a
        // sink row's claim ignores timers entirely, because the
        // sink is not a wait.
        let row = handoff_row(
            "handed_off",
            json!({
                "lane": "opencode",
                "last_checkpoint": {"stage": "handed_off", "at": "2026-11-09T23:15:55Z"},
                "session_id": null,
            }),
        );
        assert!(is_handed_off(&row));
        assert!(by_id_claimable(&row, "2026-11-09T23:20:00Z"));
        // The launch_failed recovery yield in the sink is the same
        // recorded handoff in its other spelling.
        let row2 = handoff_row(
            "handed_off",
            json!({"lane": "opencode", "stage": "launch_failed"}),
        );
        assert!(by_id_claimable(&row2, "2026-11-09T23:20:00Z"));
        // #8192 regression: the stage sources DISAGREE on this row
        // (top-level 'working' vs last_checkpoint 'handed_off') —
        // sink membership is the status, so it is claimable either
        // way; neither the broker nor the claimant reads a stage
        // for a resting sink row anymore.
        let row3 = handoff_row(
            "handed_off",
            json!({
                "lane": "opencode", "stage": "working",
                "last_checkpoint": {"stage": "handed_off"},
            }),
        );
        assert!(is_handed_off(&row3));
        assert!(by_id_claimable(&row3, "2026-11-09T23:20:00Z"));
    }

    #[test]
    fn by_id_claim_refuses_genuine_wait_parks_and_bound_rows() {
        // A gated dependency park: opencode lane, but its stage is
        // the park itself, not a yield — never a handoff.
        let gated = handoff_row(
            "parked",
            json!({"lane": "opencode", "stage": "parked", "gate": "#3801 OPEN"}),
        );
        assert!(!by_id_claimable(&gated, "2026-11-09T23:20:00Z"));
        // #8234: a 'parked' row with handoff-SHAPED progress (the
        // pre-migration / task-10995 shape: a recorded handoff that
        // later took a genuine triage wait) is a genuine wait park,
        // NOT a sink row — the status decides, the shape does not.
        let wait_shaped = handoff_row(
            "parked",
            json!({
                "lane": "opencode",
                "last_checkpoint": {"stage": "handed_off"},
                "session_id": null,
            }),
        );
        assert!(!is_handed_off(&wait_shaped));
        assert!(!by_id_claimable(&wait_shaped, "2026-11-09T23:20:00Z"));
        // A direct-lane yield in the sink status is not the
        // claimant's row (the lane guard holds).
        let direct = handoff_row(
            "handed_off",
            json!({"lane": "direct", "stage": "handed_off"}),
        );
        assert!(!by_id_claimable(&direct, "2026-11-09T23:20:00Z"));
        // A bound session id means a launch already happened; the
        // binding is supervision's, never a fresh claim's.
        let bound = handoff_row(
            "handed_off",
            json!({
                "lane": "opencode", "stage": "handed_off",
                "session_id": "ses_abc",
            }),
        );
        assert!(!by_id_claimable(&bound, "2026-11-09T23:20:00Z"));
        // A claimed row is owned; by-id never steals a live claim.
        let claimed = handoff_row(
            "claimed",
            json!({"lane": "opencode", "stage": "handed_off"}),
        );
        assert!(!by_id_claimable(&claimed, "2026-11-09T23:20:00Z"));
    }

    #[test]
    fn park_classification_lands_handoffs_in_the_sink_8234() {
        // A recorded handoff parked with no declared condition
        // sinks; so does one carrying only the bare time timer
        // (the park verb's own default bookkeeping).
        let handoff = json!({
            "lane": "opencode",
            "last_checkpoint": {"stage": "handed_off"},
            "session_id": null,
        });
        assert!(handoff_park(&handoff, true, &json!({"kind": "time"})));
        assert!(handoff_park(&handoff, false, &json!({"kind": "time"})));
        // A caller-named real wait always wins, even on a
        // handoff-shaped row: CI, a task dependency, a quota reset.
        assert!(!handoff_park(&handoff, false, &json!({"kind": "ci", "pr": 1})));
        assert!(!handoff_park(
            &handoff,
            false,
            &json!({"kind": "task_terminal", "task_id": 7})
        ));
        assert!(!handoff_park(
            &handoff,
            false,
            &json!({"kind": "quota_reset", "at": "2026-11-13T00:00:00Z"})
        ));
        // A triage-stage park is the broker's triage wait, never
        // a handoff — it does not even have the handoff shape.
        let triage = json!({"lane": "opencode", "stage": "awaiting_triage"});
        assert!(!handoff_park(&triage, true, &json!({"kind": "time"})));
        // A direct-lane yield is not an opencode handoff.
        let direct = json!({"lane": "direct", "stage": "handed_off"});
        assert!(!handoff_park(&direct, true, &json!({"kind": "time"})));
        // A still-bound row never sinks, whatever the stage says:
        // the binding means a live session owns it.
        let bound = json!({
            "lane": "opencode", "stage": "handed_off",
            "session_id": "ses_abc",
        });
        assert!(!handoff_park(&bound, true, &json!({"kind": "time"})));
    }

    #[test]
    fn sink_return_lands_in_handed_off_without_a_timer_8234() {
        // The lease-expiry sink return: status 'handed_off', every
        // wait field NULL, and no trace of the pre-#8234 fake timer
        // (+1200s not_before with a time ready_condition).
        let sql = sink_return_sql();
        assert!(sql.contains("status = 'handed_off'"));
        assert!(sql.contains("not_before = NULL"));
        assert!(sql.contains("ready_condition = NULL"));
        assert!(sql.contains("park_kind = NULL"));
        assert!(!sql.contains("1200"));
        assert!(!sql.contains("'parked'"));
        // Live statuses include the sink (dedupe ownership).
        assert!(LIVE_STATUSES.contains(&"handed_off"));
    }

    #[test]
    fn platform_parse_accepts_only_the_two_families() {
        // #8097 Rule 2: unset is absence; the only set values are
        // the two coarse families; anything else errors.
        assert_eq!(parse_platform(None), Ok(None));
        assert_eq!(parse_platform(Some("")), Ok(None));
        assert_eq!(
            parse_platform(Some("openchamber")),
            Ok(Some("openchamber".to_string()))
        );
        assert_eq!(parse_platform(Some("muse")), Ok(Some("muse".to_string())));
        assert!(parse_platform(Some("code-a")).is_err());
        assert!(parse_platform(Some("OpenChamber")).is_err());
    }

    #[test]
    fn platform_is_a_mark_not_a_claim_filter_8187() {
        // #8187 amendment (demoting #8097 Rule 2): platform is a
        // MARK, never a filter or a requirement.
        // (a) A claim facing a MISMATCHED platform mark still
        // claims the item: by-id eligibility takes no platform
        // input at all, so a muse-marked queued row is claimable
        // by any claim (an openchamber claimant's included).
        let mut row = handoff_row("queued", json!({}));
        row["not_before"] = json!("2026-11-09T00:00:00Z");
        row["platform"] = json!("muse");
        let now = "2026-11-09T23:20:00Z";
        assert!(by_id_claimable(&row, now));
        // (b) A claim declaring NO platform works identically:
        // unmarked and marked rows are equally claimable, and the
        // pick path binds no platform argument to match against.
        let mut unmarked = handoff_row("queued", json!({}));
        unmarked["not_before"] = json!("2026-11-09T00:00:00Z");
        assert!(by_id_claimable(&unmarked, now));
        // (c) The mark is RECORDED on the claimed row: both claim
        // UPDATEs stamp the declared platform over the row's mark
        // (COALESCE keeps the existing mark when none is declared)
        // and NEITHER carries a platform filter.
        let upd = claim_update_sql("(status = 'queued')");
        assert!(upd.contains("platform = COALESCE(?, platform)"), "{upd}");
        assert!(!upd.contains("platform IS NULL"), "{upd}");
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(!impl_src.contains("platform IS NULL OR platform = ?"));
        assert!(!impl_src.contains("platform_allows"));
        assert!(impl_src.matches("platform = COALESCE(?, platform)").count() >= 2);
    }

    #[test]
    fn claim_event_carries_lane_pool_model_session_marks_8187() {
        // #8187 amendments 2+3: the audit marks name who is looking
        // at the item — platform, lane, pool (= queue), model, the
        // bound session, and the task id.
        let task = json!({
            "id": 4242, "queue": "repo-health-code-pilot",
            "platform": "openchamber", "lane_id": "code-a",
            "claim_provider": "_claude", "claim_model": "claude-sonnet-5-5",
            "claim_variant": "medium",
            "progress": {"session_id": "ses_test_1", "lane": "direct"},
        });
        let detail = with_marks(&json!({"via": "claim_repo_task_turso"}), &task);
        assert_eq!(detail["task_id"], json!(4242));
        assert_eq!(detail["pool"], json!("repo-health-code-pilot"));
        assert_eq!(detail["queue"], json!("repo-health-code-pilot"));
        assert_eq!(detail["platform"], json!("openchamber"));
        assert_eq!(detail["lane_id"], json!("code-a"));
        assert_eq!(detail["model_provider"], json!("_claude"));
        assert_eq!(detail["model_id"], json!("claude-sonnet-5-5"));
        assert_eq!(detail["model_variant"], json!("medium"));
        assert_eq!(detail["session_id"], json!("ses_test_1"));
        // Explicit detail fields win over the marks.
        let d2 = with_marks(&json!({"session_id": "ses_bound_now"}), &task);
        assert_eq!(d2["session_id"], json!("ses_bound_now"));
        // Unmarked rows still emit the fields — as nulls. Marks are
        // recorded when available, never required.
        let bare = with_marks(&json!({}), &json!({"id": 7}));
        assert_eq!(bare["task_id"], json!(7));
        assert_eq!(bare["platform"], Value::Null);
        assert_eq!(bare["session_id"], Value::Null);
        // Source oracle: every audited transition (claim pick +
        // by-id, lane assign + flip, bind, park, wake, complete,
        // fail) routes its event through with_marks.
        let src = include_str!("turso_queue.rs");
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(
            impl_src.matches("with_marks(").count() >= 9,
            "audited transitions must carry marks: {}",
            impl_src.matches("with_marks(").count()
        );
    }

    #[test]
    fn by_id_claim_queued_rows_keep_the_pick_predicate() {
        // Queued + attempts left + not_before passed: claimable.
        let mut row = handoff_row("queued", json!({}));
        row["not_before"] = json!("2026-11-09T00:00:00Z");
        assert!(by_id_claimable(&row, "2026-11-09T23:20:00Z"));
        // Queued but parked into the future: not claimable by id.
        let future = handoff_row("queued", json!({}));
        assert!(!by_id_claimable(&future, "2026-11-09T23:20:00Z"));
        // Queued at the attempts cap: not claimable by id.
        let mut capped = handoff_row("queued", json!({}));
        capped["not_before"] = json!("2026-11-09T00:00:00Z");
        capped["attempts"] = json!(4);
        assert!(!by_id_claimable(&capped, "2026-11-09T23:20:00Z"));
    }

    #[test]
    fn completion_class_and_follow_on_key() {
        assert_eq!(
            completion_class(&json!({"kind": "small_fix"})),
            CompletionClass::Fix
        );
        assert_eq!(
            completion_class(&json!({"kind": "triage"})),
            CompletionClass::Triage
        );
        assert_eq!(completion_class(&json!({})), CompletionClass::Ungated);
        // Ungated completions never gate, whatever the result.
        assert_eq!(
            gate_completion(&CompletionClass::Ungated, &json!({})).unwrap(),
            None
        );
        assert_eq!(
            follow_on_key("repo-health:v1:triage:developing-today/code:issue:787"),
            Some("repo-health:v1:fix:developing-today/code:issue:787".to_string())
        );
        // Maintenance triage keys have no fix counterpart.
        assert_eq!(
            follow_on_key("repo-health:v1:maintenance:developing-today/code:weekly:2026-W44"),
            None
        );
    }

    // -- /coverage (#7016) --------------------------------------------------

    #[test]
    fn issue_number_parses_canonical_keys() {
        let repo = "developing-today/code";
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today/code:issue:7016",
                repo
            ),
            Some(7016)
        );
        assert_eq!(
            issue_number_from_dedupe("repo-health:v1:fix:developing-today/code:issue:5486", repo),
            Some(5486)
        );
    }

    #[test]
    fn issue_number_parses_dispatch_suffixed_keys() {
        let repo = "developing-today/code";
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today/code:issue:7016:dispatch:abc123",
                repo
            ),
            Some(7016)
        );
    }

    #[test]
    fn issue_number_rejects_other_repos_and_non_issue_keys() {
        let repo = "developing-today/code";
        // A different repo is not coverage for this one.
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today-forks/opencode:issue:195",
                repo
            ),
            None
        );
        // Maintenance keys carry no issue number.
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:maintenance:developing-today/code:weekly:2026-W44",
                repo
            ),
            None
        );
        // No number at all.
        assert_eq!(
            issue_number_from_dedupe("repo-health:v1:triage:developing-today/code:issue:", repo),
            None
        );
        // A tail that is neither empty nor a separator is not a match
        // (e.g. `:issue:12abc`), so a malformed key cannot silently
        // claim coverage for issue 12.
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today/code:issue:12abc",
                repo
            ),
            None
        );
    }

    #[test]
    fn issue_number_matches_legacy_repo_aliases() {
        // The two names of each renamed repo are the same coverage.
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:triage:developing-today/opencode:issue:195",
                "developing-today-forks/opencode"
            ),
            Some(195)
        );
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:fix:developing-today/openchamber:issue:42",
                "developing-today-forks/openchamber"
            ),
            Some(42)
        );
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:triage:dezren39/nix:issue:1637",
                "dezren39/darwin-nix"
            ),
            Some(1637)
        );
        // Matching works in the other direction too: a new-name key is
        // coverage when the caller names the old one.
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:triage:developing-today-forks/opencode:issue:195",
                "developing-today/opencode"
            ),
            Some(195)
        );
    }

    #[test]
    fn alias_matching_does_not_cross_repos() {
        // An opencode key is never coverage for code or openchamber,
        // alias or not.
        for repo in [
            "developing-today/code",
            "developing-today-forks/openchamber",
            "dezren39/darwin-nix",
        ] {
            assert_eq!(
                issue_number_for_repo(
                    "repo-health:v1:triage:developing-today/opencode:issue:1",
                    repo
                ),
                None
            );
        }
    }

    proptest! {
        /// Any canonical key built from a repo and a positive issue number
        /// round-trips through the parser, for every class and every
        /// dispatch suffix.
        #[test]
        fn prop_issue_number_round_trips(
            n in 1i64..10_000_000i64,
            class in prop::sample::select(vec!["triage", "fix"]),
            suffix in prop::option::of("[a-z0-9]{1,12}"),
        ) {
            let repo = "developing-today/code";
            let mut key = format!("repo-health:v1:{class}:{repo}:issue:{n}");
            if let Some(s) = &suffix {
                key.push_str(&format!(":dispatch:{s}"));
            }
            prop_assert_eq!(issue_number_from_dedupe(&key, repo), Some(n));
        }

        /// A key for a different repo never matches, whatever the number.
        #[test]
        fn prop_other_repo_never_matches(n in 1i64..10_000_000i64) {
            let key = format!("repo-health:v1:triage:developing-today-forks/opencode:issue:{n}");
            prop_assert_eq!(issue_number_from_dedupe(&key, "developing-today/code"), None);
        }
    }

    // ---- #8217: batch claim, platform limitation in the pick,
    // ---- and the release verb ----

    fn seg8217<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
        src.split(from)
            .nth(1)
            .unwrap_or("")
            .split(to)
            .next()
            .unwrap_or("")
    }

    #[test]
    fn batch_count_parsing_and_cap_8217() {
        // The cap is a named constant with the reasoning on #8217:
        // one batch = one lane touch = one supervisor.
        assert_eq!(BATCH_CLAIM_MAX, 5);
        assert_eq!(parse_batch_count(&json!({})), Ok(1));
        assert_eq!(parse_batch_count(&json!({"count": 1})), Ok(1));
        assert_eq!(parse_batch_count(&json!({"count": 3})), Ok(3));
        assert_eq!(
            parse_batch_count(&json!({"count": BATCH_CLAIM_MAX})),
            Ok(BATCH_CLAIM_MAX)
        );
        let over = parse_batch_count(&json!({"count": BATCH_CLAIM_MAX + 1})).unwrap_err();
        assert!(over.contains("out of range") && over.contains("1..=5"), "{over}");
        assert!(parse_batch_count(&json!({"count": 0})).is_err());
        assert!(parse_batch_count(&json!({"count": -2})).is_err());
    }

    #[test]
    fn batch_claim_response_shape_8217() {
        // The batch verb's wire shape: the claimed rows in pick
        // order, how many were claimed, how many were requested —
        // a short batch is a normal partial success, and an empty
        // one is the usual no-task verdict on the client.
        let rows = vec![json!({"id": 11}), json!({"id": 7}), json!({"id": 42})];
        let out = json!({"rows": rows, "claimed": 3, "requested": 5});
        assert_eq!(out["claimed"], json!(3));
        assert_eq!(out["requested"], json!(5));
        let ids: Vec<i64> = out["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![11, 7, 42], "rows keep pick order");
    }

    #[test]
    fn claim_flow_composes_claim_lane_bind_in_order_5757() {
        // #5757-A oracle: /claim-flow is dispatched, prechecks
        // before any step runs, and composes the three verbs in
        // claim -> lane -> bind order, answering with all three
        // rows. (The verbs themselves are covered by their own
        // tests; this pins the composition and the route.)
        let src = include_str!("turso_queue.rs");
        assert!(
            src.contains("\"/claim-flow\" => verb_claim_flow(t, req)"),
            "dispatch routes /claim-flow"
        );
        let flow = seg8217(src, "fn verb_claim_flow(", "\nfn claimed_by(");
        let pre = flow.find("claim_flow_precheck").expect("precheck present");
        let c = flow.find("verb_claim(").expect("claim step");
        let l = flow.find("verb_lane(").expect("lane step");
        let b = flow.find("verb_bind(").expect("bind step");
        assert!(pre < c && c < l && l < b, "steps run precheck -> claim -> lane -> bind");
        assert!(flow.contains("\"bind_row\""), "answer carries the bind row");
        assert!(flow.contains("\"lane_row\""), "answer carries the lane row");
    }

    #[test]
    fn batch_claim_runs_the_single_pick_repeatedly_8217() {
        // Oracle: the batch verb is a loop over claim_pick_once
        // (the single claim's pick — same eligibility, same order,
        // same per-item lease, marks and claimed event) behind ONE
        // claim_preamble, and the single claim routes count > 1 to
        // it while refusing the by-id combination.
        let src = include_str!("turso_queue.rs");
        let batch = seg8217(src, "fn verb_claim_batch(", "\nfn verb_claim(");
        assert!(batch.contains("claim_preamble("), "one preamble per batch");
        assert!(
            batch.contains("claim_pick_once("),
            "batch = the single pick, repeated"
        );
        assert!(batch.contains("\"rows\": rows"), "batch returns the claimed set");
        let single = seg8217(src, "fn verb_claim(", "\nfn verb_bind(");
        assert!(single.contains("count > 1"), "single claim routes batches");
        assert!(
            single.contains("cannot be combined with --task"),
            "by-id and transfer claims stay single"
        );
        assert!(
            single.contains("claim_preamble(") && single.contains("claim_pick_once("),
            "single claim is one preamble + one pick"
        );
    }

    #[test]
    fn platform_limit_pick_clause_shape_8217() {
        // The clause consults the lane ledger and skips only rows
        // with a VALID stored limitation (a recorded reason)
        // naming the OTHER family; reason-less rows are not
        // limitations (Drew's rule: no reason, no limitation).
        for needle in [
            "repo_task_lane_ledger",
            "platform_limit IS NOT NULL",
            "platform_limit_reason IS NOT NULL",
            "platform_limit_reason <> ''",
            "(? IS NULL OR",
        ] {
            assert!(
                PLATFORM_LIMIT_PICK_CLAUSE.contains(needle),
                "clause misses {needle}"
            );
        }
        // Both pick statements carry the clause after the
        // eligibility predicate, with the claim's family bound
        // twice: 7 fixed placeholders + 2 predicate + 2 clause.
        let upd = claim_update_sql(&default_pick_predicate());
        assert!(
            upd.contains("AND NOT EXISTS (SELECT 1 FROM repo_task_lane_ledger"),
            "{upd}"
        );
        assert_eq!(upd.matches('?').count(), 11, "claim UPDATE bindings: {upd}");
        let src = include_str!("turso_queue.rs");
        let pick = seg8217(src, "fn claim_pick_once(", "\nfn parse_batch_count(");
        assert_eq!(
            pick.matches("pick_args.push(arg_str(family.as_deref()))").count(),
            2,
            "candidate SELECT binds the family twice"
        );
        assert_eq!(
            pick.matches("upd_args.push(arg_str(family.as_deref()))").count(),
            2,
            "claim UPDATE binds the family twice"
        );
    }

    #[test]
    fn claim_by_id_enforces_platform_limit_like_the_pick_8217() {
        // Oracle: a plain by-id claim refuses a cross-family valid
        // limitation (the row reads as not claimable) and CLEARS a
        // reason-less stored one instead of enforcing it.
        let src = include_str!("turso_queue.rs");
        let seg = seg8217(src, "fn claim_by_id(", "\nfn claim_transfer(");
        assert!(
            seg.contains("platform_limit_refusal("),
            "by-id claim enforces the limitation"
        );
        assert!(
            seg.contains("platform_limit = NULL")
                && seg.contains("platform_limit_reason = NULL"),
            "an invalid stored limit is cleared, not enforced"
        );
    }

    #[test]
    fn claim_by_id_targets_exactly_the_named_task_7735() {
        // #7735 oracle: a targeted claim is a claim of one row or
        // no claim — it can never fall through to the queue's
        // next-up row (the monitor's Duty 1B footgun: an untargeted
        // claim fired on whatever was next). The by-id path reads
        // the named row, flips it with an UPDATE keyed to that id
        // plus the status it read, and reads the result back by
        // the same id; it contains no pick at all.
        let src = include_str!("turso_queue.rs");
        // Bound the segment at claim_by_id's own closing brace:
        // seg8217's usual end marker (claim_transfer) is defined
        // BEFORE claim_by_id in this file, so its segment would
        // run to EOF and the forbidden-needle checks below would
        // trip on the pick function further down.
        let seg = src
            .split("fn claim_by_id(")
            .nth(1)
            .unwrap_or("")
            .split("\n}\n")
            .next()
            .unwrap_or("");
        assert!(
            seg.contains("get_task(t, task_id)"),
            "by-id claim reads the named row"
        );
        assert!(
            seg.contains("WHERE id = ? AND status = ? AND queue = ?"),
            "by-id claim's UPDATE is keyed to the named id and the status it read"
        );
        assert!(
            seg.contains("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
            "by-id claim reads its result back by the named id"
        );
        for forbidden in ["claim_pick_once(", "ORDER BY priority"] {
            assert!(
                !seg.contains(forbidden),
                "by-id claim must never pick a different task: found {forbidden}"
            );
        }
        // Predicate half, in the claim_race idiom: the named row is
        // claimable while it is queued, and the instant it is
        // claimed (by anyone) a targeted claim of it refuses —
        // there is no state in which it silently substitutes the
        // next queued row, which stays claimable under its own id.
        let now = "2026-11-11T02:45:00Z";
        let mut named = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        assert!(by_id_claimable(&named, now));
        named["status"] = json!("claimed");
        assert!(!by_id_claimable(&named, now));
        let next = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        assert!(by_id_claimable(&next, now));
    }

    #[test]
    fn release_verb_contract_8217() {
        // Oracle: release is holder-checked and reason-required,
        // and returns the row to `queued` with its lease and
        // session binding cleared and a `released` audit event;
        // attempts are NOT handed back.
        let src = include_str!("turso_queue.rs");
        let seg = seg8217(src, "/// Release a claimed task straight back", "\nfn verb_park(");
        for needle in [
            "a release requires a reason",
            "claimed_by(",
            "\"released\"",
            "DELETE FROM leases",
            "attempt change",
            "SET status = 'queued'",
        ] {
            assert!(seg.contains(needle), "verb_release misses {needle}");
        }
        assert!(
            src.contains("\"/release\" => verb_release(t, req),"),
            "release is dispatched"
        );
    }

    #[test]
    fn one_queue_filter_parse_and_compile_8097() {
        // The designated lanes' expressions parse and compile:
        // not-code is `repo != code`; code-only is `repo = code`.
        let not_code = parse_filter("repo != code").expect("not-code parses");
        let mut args: Vec<Arg> = Vec::new();
        let sql = filter_sql(&not_code, &mut args);
        assert!(sql.contains("NOT"), "negation compiles: {sql}");
        assert!(sql.contains("json_extract(payload, '$.repo')"), "repo identity: {sql}");
        assert_eq!(args.len(), 2, "repo equality binds the value twice: {sql}");
        match (&args[0], &args[1]) {
            (Arg::Text(a), Arg::Text(b)) => assert_eq!((a.as_str(), b.as_str()), ("code", "code")),
            _ => panic!("repo binds text values"),
        }
        // Boolean composition: AND over OR, NOT tightest.
        let expr = parse_filter("(repo = nix OR repo = opencode) AND NOT platform = muse")
            .expect("composed filter parses");
        let mut args: Vec<Arg> = Vec::new();
        let sql = filter_sql(&expr, &mut args);
        assert!(sql.contains(" OR "), "{sql}");
        assert!(sql.contains(" AND "), "{sql}");
        assert!(sql.contains("(NOT "), "{sql}");
        assert_eq!(args.len(), 5, "two repo values x2 + one platform: {sql}");
        // IN lists and priority ordering.
        let expr = parse_filter("repo IN (nix, opencode) AND priority <= 3").expect("IN parses");
        let mut args: Vec<Arg> = Vec::new();
        let sql = filter_sql(&expr, &mut args);
        assert!(sql.contains("priority <= ?"), "{sql}");
        assert!(matches!(args.last(), Some(Arg::Int(3))), "priority binds an integer");
        // Malformed expressions are usage errors, never silent.
        for bad in [
            "",
            "repo < code",
            "priority <= soon",
            "bogus = x",
            "(repo = code",
            "repo = ",
            "repo IN ()",
            "repo ~ code",
        ] {
            assert!(parse_filter(bad).is_err(), "rejects {bad:?}");
        }
    }

    #[test]
    fn one_queue_union_match_is_empty_by_default_8097() {
        // Drew's model (2026-11-10): the union of the task's
        // requirements and the claimer's requirements, each
        // filtering on the other's metadata — and BOTH sides are
        // empty by default, so matching is pure priority order.
        let code_task = json!({
            "queue": "repo-health-code-pilot", "priority": 1,
            "payload": {"schema": "repo-task/v1", "repo": "developing-today/code", "kind": "small_fix"},
        });
        let nix_task = json!({
            "queue": "repo-health-nix-pilot", "priority": 2,
            "payload": {"schema": "repo-task/v1", "repo": "dezren39/darwin-nix", "kind": "small_fix"},
        });
        let stream_only_task = json!({
            "queue": "repo-health-code-pilot", "priority": 3,
            "payload": {"schema": "repo-task/v1", "kind": "triage"},
        });
        let meta_task = json!({
            "queue": "meta", "priority": 4, "payload": {},
        });
        // No requirements on any of these tasks: every claimer
        // meets them, whatever its identity — identity alone
        // never filters.
        let claimer = ClaimerCtx {
            family: Some("openchamber".to_string()),
            lane: Some("code-z".to_string()),
            worker: "repo-worker-code".to_string(),
            pool: None,
            model: None,
        };
        for task in [&code_task, &nix_task, &stream_only_task, &meta_task] {
            assert_eq!(task_requirements_unmet(task, &claimer), None);
        }
        // Claimer-side expressions over task metadata.
        let not_code = parse_filter("repo != code").unwrap();
        assert!(!filter_matches(&not_code, &code_task), "code task is code");
        assert!(filter_matches(&not_code, &nix_task), "nix task is not code");
        assert!(filter_matches(&not_code, &meta_task), "meta task is not code");
        let code_only = parse_filter("repo = code").unwrap();
        assert!(filter_matches(&code_only, &code_task), "payload repo basename matches");
        assert!(
            filter_matches(&code_only, &stream_only_task),
            "stream-derived identity matches a payload without repo"
        );
        assert!(!filter_matches(&code_only, &nix_task));
        let full_name = parse_filter("repo = developing-today/code").unwrap();
        assert!(filter_matches(&full_name, &code_task), "full owner/name matches");
        // Priority and kind dimensions.
        let urgent_code = parse_filter("priority <= 2 AND kind = small_fix").unwrap();
        assert!(filter_matches(&urgent_code, &code_task));
        assert!(filter_matches(&urgent_code, &nix_task));
        assert!(!filter_matches(&urgent_code, &stream_only_task), "priority 3 is not <= 2");
    }

    #[test]
    fn one_queue_task_requirements_match_claimers_automatically_8097() {
        // A task's own declared requirements are honored inside
        // the claim: an ineligible claimer simply does not get
        // the item; no out-of-band step exists.
        let muse_only = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"platform": "muse"}},
        });
        let oc_claimer = ClaimerCtx {
            family: Some("openchamber".to_string()),
            ..Default::default()
        };
        let muse_claimer = ClaimerCtx {
            family: Some("muse".to_string()),
            ..Default::default()
        };
        let anon_claimer = ClaimerCtx::default();
        assert!(task_requirements_unmet(&muse_only, &oc_claimer).is_some());
        assert!(task_requirements_unmet(&muse_only, &anon_claimer).is_some());
        assert_eq!(task_requirements_unmet(&muse_only, &muse_claimer), None);
        // Lane requirement: only the named lane.
        let lane_task = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"lane": "code-b"}},
        });
        let lane_b = ClaimerCtx {
            lane: Some("code-b".to_string()),
            ..Default::default()
        };
        let lane_c = ClaimerCtx {
            lane: Some("code-c".to_string()),
            ..Default::default()
        };
        assert_eq!(task_requirements_unmet(&lane_task, &lane_b), None);
        assert!(task_requirements_unmet(&lane_task, &lane_c).is_some());
        assert!(task_requirements_unmet(&lane_task, &anon_claimer).is_some());
        // Pool and model requirements.
        let pooled = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"pool": "summit", "model": "claude-sonnet-5-5"}},
        });
        let summit = ClaimerCtx {
            pool: Some("summit".to_string()),
            model: Some(ClaimModel {
                provider: "_claude".to_string(),
                model: Some("claude-sonnet-5-5".to_string()),
                variant: None,
            }),
            ..Default::default()
        };
        assert_eq!(task_requirements_unmet(&pooled, &summit), None);
        let wrong_pool = ClaimerCtx {
            pool: Some("workhorse".to_string()),
            ..summit.clone()
        };
        assert!(task_requirements_unmet(&pooled, &wrong_pool).is_some());
        // The full "provider/model" spelling also meets a model
        // requirement, and a model-less claimer meets none.
        let full_model_task = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"model": "_claude/claude-sonnet-5-5"}},
        });
        assert_eq!(task_requirements_unmet(&full_model_task, &summit), None);
        assert!(task_requirements_unmet(&full_model_task, &anon_claimer).is_some());
    }

    #[test]
    fn one_queue_scope_sql_shape_8097() {
        let claimer = ClaimerCtx {
            family: Some("muse".to_string()),
            lane: Some("nix-a".to_string()),
            worker: "repo-worker-nix".to_string(),
            pool: None,
            model: None,
        };
        // The default claim: no stream, no filter — the scope is
        // exactly the task-requirements clause (5 bindings), so
        // the pick spans the whole queue in priority order.
        let scope = claim_scope(None, None, &claimer);
        assert!(!scope.sql.contains("queue = ?"), "unscoped: {}", scope.sql);
        assert!(scope.sql.contains("requires.platform"), "requirements always bind");
        assert_eq!(scope.args.len(), 5);
        // A stream-scoped claim keeps the old shape: queue first.
        let scope = claim_scope(Some("repo-health-code-pilot"), None, &claimer);
        assert!(scope.sql.starts_with(" AND queue = ?"), "{}", scope.sql);
        match &scope.args[0] {
            Arg::Text(q) => assert_eq!(q, "repo-health-code-pilot"),
            _ => panic!("stream binds first"),
        }
        assert_eq!(scope.args.len(), 6);
        // A filtered claim: filter args follow the stream arg and
        // precede the requirement bindings.
        let expr = parse_filter("repo != code").unwrap();
        let scope = claim_scope(None, Some(&expr), &claimer);
        assert_eq!(scope.args.len(), 7);
        match (&scope.args[0], &scope.args[1]) {
            (Arg::Text(a), Arg::Text(b)) => {
                assert_eq!((a.as_str(), b.as_str()), ("code", "code"))
            }
            _ => panic!("filter binds before requirements"),
        }
        // The registry's unbound markers.
        assert!(!lane_queue_binds("*"));
        assert!(!lane_queue_binds("all"));
        assert!(!lane_queue_binds(""));
        assert!(lane_queue_binds("repo-health-code-pilot"));
    }

    #[test]
    fn one_queue_claim_wiring_8097() {
        // Source oracle: the claim verb parses the filter, builds
        // the union scope, and the single pick applies it in both
        // statements; the registry binding is consulted only for
        // stream-scoped claims.
        let src = include_str!("turso_queue.rs");
        let single = seg8217(src, "fn verb_claim(", "\nfn verb_bind(");
        assert!(single.contains("parse_filter("), "claim parses --filter");
        assert!(single.contains("claim_scope("), "claim builds the union scope");
        assert!(single.contains("\"filter\""), "claim reads the filter field");
        assert!(single.contains("\"pool\""), "claim reads the pool identity");
        let pick = seg8217(src, "fn claim_pick_once(", "\nfn parse_batch_count(");
        assert!(pick.contains("scope.sql"), "pick SELECT applies the scope");
        assert!(pick.contains("scope.args"), "pick binds the scope arguments");
        let preamble = seg8217(src, "fn claim_preamble(", "\nfn claim_pick_once(");
        assert!(
            preamble.contains("lane_queue_binds("),
            "preamble honors the unbound registration marker"
        );
        let by_id = seg8217(src, "fn claim_by_id(", "\nfn claim_transfer(");
        assert!(
            by_id.contains("task_requirements_unmet("),
            "by-id claim honors the task's requirements"
        );
    }
}
