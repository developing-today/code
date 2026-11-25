//! turso_queue::core — backend core: wire types, schema, task/ledger reads, events, sweep and coverage.
//!
//! Split from turso_queue.rs for #8589 (F11.10 production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

pub const DEFAULT_TURSO_URL: &str = "https://fleet-queue-dezren39.aws-us-east-1.turso.io";
pub const TURSO_TOKEN_SECRET_NAME: &str = "turso.fleet-queue-rw-token";

pub(crate) const TASK_COLS: &str = "id, queue, manual_only, title, body, payload, metadata, dedupe_key, priority, status, \
    not_before, worker_id, claimed_at, lease_until, lane_id, platform, attempts, max_attempts, \
    claim_provider, claim_model, claim_variant, claim_session_id, \
    progress, result, last_error, park_kind, wait_kind, ready_condition, \
    required_budget_minutes, \
    last_checked_at, check_outcome, created_at, updated_at, completed_at, \
    blocked_deps, dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds, \
    resolution, resolution_category, resolution_substatus, resolution_reason, resolution_ref, \
    resolved_by, resolved_at";
pub(crate) const LEDGER_COLS: &str = "id, queue, task_id, assigned_lane, executed_lane, executed_lane_id, \
    exclusion_reason, opencode_reason, platform_limit, platform_limit_reason, \
    assigned_by, session_id, fallback_reason, \
    assigned_at, executed_at, updated_at";
/// Task columns whose stored TEXT is JSON (PG jsonb) — parsed back
/// into values so the wire shape matches the Postgres broker's.
pub(crate) const TASK_JSON_COLS: [&str; 6] = [
    "payload",
    "metadata",
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

pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn now_iso() -> String {
    iso(now_unix())
}

pub(crate) fn after_iso(secs: i64) -> String {
    iso(now_unix() + secs)
}

// ---------------------------------------------------------------------------
// libSQL HTTP pipeline client (same wire as fleet-dispatch-test, #6580)
// ---------------------------------------------------------------------------

pub(crate) enum Arg {
    Text(String),
    Int(i64),
    Float(f64),
    Null,
}

impl Arg {
    fn to_json(&self) -> Value {
        match self {
            Arg::Text(v) => json!({"type": "text", "value": v}),
            Arg::Int(v) => json!({"type": "integer", "value": v.to_string()}),
            // hrana carries floats as JSON numbers (integers go as
            // strings); both shapes are accepted on read.
            Arg::Float(v) => json!({"type": "float", "value": v}),
            Arg::Null => json!({"type": "null"}),
        }
    }
}

pub(crate) fn arg_str(v: Option<&str>) -> Arg {
    v.map(|s| Arg::Text(s.to_string())).unwrap_or(Arg::Null)
}

pub(crate) fn arg_json(v: &Value) -> Arg {
    if v.is_null() {
        Arg::Null
    } else {
        Arg::Text(v.to_string())
    }
}

pub(crate) struct StmtResult {
    pub(crate) cols: Vec<String>,
    pub(crate) rows: Vec<Vec<Value>>,
    pub(crate) affected: u64,
    pub(crate) last_insert_rowid: Option<i64>,
}

pub(crate) fn cell_to_json(cell: &Value) -> Value {
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
        // The pipeline may carry a float as a JSON number
        // (hrana) or as a string (mirrors that stringify args);
        // accept both so REAL columns read back either way.
        "float" => raw
            .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok()))
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
    /// #10626: whether an issue closed / a PR merged, and when, for
    /// the claim-time staleness check. Implementations bound it by
    /// STALENESS_LOOKUP_TIMEOUT and map every problem to Unknown.
    /// The default knows nothing, so a probe without it never
    /// retires a task.
    fn probe_landing(&self, _kind: LinkKind, _repo: &str, _number: i64) -> Landing {
        Landing::Unknown("this probe does not read landing times".to_string())
    }
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
        Some("cancelled") => {
            DepObs::Cleared("dep task cancelled; dependent released for re-evaluation".to_string())
        }
        Some("failed") => {
            DepObs::Cleared("dep task failed; dependent released for re-evaluation".to_string())
        }
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

/// A human label for one dep identity, used in drop errors and
/// audit records: `issue owner/repo#12`, `pr owner/repo#3`,
/// `ci owner/repo#456`, `task 789`.
pub fn dep_label(dep: &Value) -> String {
    let kind = dep.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
    let repo = dep.get("repo").and_then(|v| v.as_str()).unwrap_or("?");
    match kind {
        "task" => format!(
            "task {}",
            dep.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0)
        ),
        "ci" => format!(
            "ci {repo}#{}",
            dep.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0)
        ),
        _ => format!(
            "{kind} {repo}#{}",
            dep.get("number").and_then(|v| v.as_i64()).unwrap_or(0)
        ),
    }
}

/// Identity match between a stored dep record and a drop spec
/// (#12777): same kind, and the same identity fields — repo +
/// number for pr/issue, repo + run_id for ci, task_id for task.
/// State, reason and bookkeeping fields never take part, so a
/// spec written from the block-time ref alone always matches
/// the record the broker stored for it.
pub fn dep_spec_matches(dep: &Value, spec: &Value) -> bool {
    let kind = spec.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    if dep.get("kind").and_then(|v| v.as_str()) != Some(kind) {
        return false;
    }
    let same = |k: &str| dep.get(k) == spec.get(k);
    match kind {
        "pr" | "issue" => same("repo") && same("number"),
        "ci" => same("repo") && same("run_id"),
        "task" => same("task_id"),
        _ => false,
    }
}

/// Remove the dropped dep entries from a blocked task's record
/// (#12777). Ok((remaining, dropped)) preserves record order;
/// Err carries every drop spec that matched nothing — the
/// caller refuses the whole correction, so a mistyped ref can
/// never silently drop a different dep, or none at all.
pub fn remove_deps(
    deps: &[Value],
    drops: &[Value],
) -> Result<(Vec<Value>, Vec<Value>), Vec<Value>> {
    let mut remaining: Vec<Value> = deps.to_vec();
    let mut dropped = Vec::new();
    let mut unmatched = Vec::new();
    for spec in drops {
        if let Some(pos) = remaining.iter().position(|d| dep_spec_matches(d, spec)) {
            dropped.push(remaining.remove(pos));
        } else {
            unmatched.push(spec.clone());
        }
    }
    if unmatched.is_empty() {
        Ok((remaining, dropped))
    } else {
        Err(unmatched)
    }
}

/// Validate the drop specs of an unblock request (#12777): the
/// surgical form of /unblock. Ok(None) when the request carries
/// no drop list — the classic full override, unchanged. Specs
/// are identity-only and held to the same strictness as
/// parse_block_deps: a ref the broker cannot name is a usage
/// error here, never a silent no-match at drop time.
pub fn parse_drop_deps(req: &Value) -> Result<Option<Vec<Value>>, String> {
    let Some(raw) = req.get("drop_deps") else {
        return Ok(None);
    };
    let Some(arr) = raw.as_array() else {
        return Err("unblock: drop_deps must be an array".to_string());
    };
    if arr.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for d in arr {
        let kind = d.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let mut spec = json!({ "kind": kind });
        let obj = spec.as_object_mut().expect("spec object");
        match kind {
            "pr" | "issue" | "ci" => {
                let repo = d.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                if !valid_dep_repo(repo) {
                    return Err(format!(
                        "unblock: drop dep repo must be owner/name, got '{repo}'"
                    ));
                }
                obj.insert("repo".to_string(), json!(repo));
                if kind == "ci" {
                    let run_id = d.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
                    if run_id <= 0 {
                        return Err("unblock: ci drop dep needs a positive run_id".to_string());
                    }
                    obj.insert("run_id".to_string(), json!(run_id));
                } else {
                    let number = d.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                    if number <= 0 {
                        return Err(format!("unblock: {kind} drop dep needs a positive number"));
                    }
                    obj.insert("number".to_string(), json!(number));
                }
            }
            "task" => {
                let task_id = d.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                if task_id <= 0 {
                    return Err("unblock: task drop dep needs a positive task_id".to_string());
                }
                obj.insert("task_id".to_string(), json!(task_id));
            }
            _ => {
                return Err(format!(
                    "unblock: unknown drop dep kind '{kind}' (pr|issue|task|ci)"
                ));
            }
        }
        out.push(spec);
    }
    Ok(Some(out))
}

pub(crate) fn valid_dep_repo(repo: &str) -> bool {
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
                    return Err(format!("block: dep repo must be owner/name, got '{repo}'"));
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
                        return Err(format!("block: {kind} dep needs a positive number"));
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
                ));
            }
        }
        out.push(dep);
    }
    Ok(Some(out))
}

pub struct TursoBackend {
    pub(crate) url: String,
    pub(crate) token: String,
    pub(crate) agent: ureq::Agent,
    /// #8276: the GitHub dep probe, installed by the broker shell
    /// at startup (queue_broker.rs owns the secrets Resolver).
    /// None in tests and embedders: GitHub dep checks then report
    /// Failed, and only a block's max TTL can release it.
    pub(crate) gh_probe: Option<std::sync::Arc<dyn DepProbe>>,
    /// #10626: the claim-time staleness check's cache and failure
    /// stand-down (see staleness.rs).
    pub(crate) staleness: StalenessState,
}

pub(crate) type TErr = String;

/// The canonical fleet lane registry (#6853): every lane id a
/// fleet worker may claim with, the worker that owns it, and the
/// enabled flag a missing registration is restored with. The
/// queue binding is the one-queue default ("*": the lane may
/// claim any stream; see lane_queue_binds).
///
/// Drew, 2026-11-16: the registry knows EVERY variation -- each
/// family spans the full a..=z suffix space and every canonical
/// lane is enabled. This supersedes the 2026-11-10 trims and the
/// #8097 36-lane set (code-a..j, not-code-a..g, any-a..s, most of
/// them disabled or absent): those trims are why live claims kept
/// meeting "unknown lane_id" (opencode-a/b, openchamber-a/b and
/// nix-a/b were not seeded at all) and "lane is disabled"
/// (not-code-c and most of the any-* range). The seed stays
/// INSERT OR IGNORE, so it fills missing rows with exactly these
/// fields and never touches a row that already exists -- a lane
/// disabled by hand stays disabled until it is re-enabled by
/// hand, and a pre-existing row keeps its recorded worker.
/// Registrations for session-scoped lanes (UUID ids) are created
/// by their sessions and are not part of the canonical set.
pub(crate) fn canonical_lanes() -> Vec<(String, &'static str, bool)> {
    const FAMILIES: [(&str, &str); 6] = [
        ("code", "repo-worker-code"),
        ("any", "repo-worker-code"),
        ("not-code", "repo-worker-code"),
        ("opencode", "repo-worker-opencode"),
        ("openchamber", "repo-worker-openchamber"),
        ("nix", "repo-worker-nix"),
    ];
    let mut lanes = Vec::with_capacity(FAMILIES.len() * 26);
    for (family, worker) in FAMILIES {
        for c in 'a'..='z' {
            let id = format!("{family}-{c}");
            let worker = not_code_lane_owner(&id).unwrap_or(worker);
            lanes.push((id, worker, true));
        }
    }
    lanes
}

/// The five not-code lanes that belong to another worker than the
/// family default. They predate the full-universe seed (#8097's
/// not-code-a..g set): not-code-a was nix-a and not-code-g is the
/// nix event-hook worker's own lane, not-code-b was opencode-a and
/// not-code-e is the opencode-project OpenChamber task, not-code-d
/// is OpenChamber's. Live rows carry these owners, and claim_preamble
/// refuses a claim whose worker is not the row's, so a re-seed (a
/// fresh database, or a row deleted by hand) must restore the same
/// owner or that worker's claims fail with "lane ... belongs to
/// worker repo-worker-code".
pub(crate) fn not_code_lane_owner(lane_id: &str) -> Option<&'static str> {
    match lane_id {
        "not-code-a" | "not-code-g" => Some("repo-worker-nix"),
        "not-code-b" | "not-code-e" => Some("repo-worker-opencode"),
        "not-code-d" => Some("repo-worker-openchamber"),
        _ => None,
    }
}

/// The family of a lane id shaped `<family>-<suffix>`, when the
/// family is one the broker knows and the suffix is a nonempty
/// lowercase alphabetic string ("code-a", "not-code-c",
/// "darwin-nix-a"). Since #12544 (Drew, 2026-11-22) claims no
/// longer consult this: claim_preamble auto-registers ANY lane
/// name on first claim. The family rule survives for
/// execution_lane_fields, which derives an execution record's
/// mode from the family of the lane id it carries (#7598).
/// darwin-nix remains wildcard-only in the seed sense: it is
/// deliberately not part of the canonical seed set above, but a
/// darwin-nix lane claim registers its row on first use, like
/// any other name.
pub(crate) fn lane_family_of(lane_id: &str) -> Option<&str> {
    let (family, suffix) = lane_id.rsplit_once('-')?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    LANE_FAMILIES.contains(&family).then_some(family)
}

/// The lane families the broker knows (see lane_family_of).
const LANE_FAMILIES: [&str; 7] = [
    "code",
    "any",
    "not-code",
    "opencode",
    "openchamber",
    "nix",
    "darwin-nix",
];

/// The registry seed statement (#6853): INSERT OR IGNORE, so it
/// restores a missing registration with its canonical fields and
/// can never resurrect or re-enable a lane whose row already
/// exists in any state -- a lane disabled by hand stays disabled
/// across broker restarts until it is re-enabled by hand. Lanes
/// outside the canonical set are never seeded at all;
/// claim_preamble's auto-registration (lane_provision_sql,
/// #12544) registers any lane the seed does not name, on first
/// claim.
pub(crate) fn lane_seed_sql() -> String {
    let values: Vec<String> = canonical_lanes()
        .iter()
        .map(|(id, worker, enabled)| format!("('{id}', '{worker}', '*', {})", i32::from(*enabled)))
        .collect();
    format!(
        "INSERT OR IGNORE INTO lanes (lane_id, worker_id, queue, enabled) VALUES {}",
        values.join(", ")
    )
}

/// The claim-time auto-registration statement (#12544, Drew
/// 2026-11-22): a claim naming a lane with no registry row
/// registers it on the spot -- the claiming worker as owner, the
/// one-queue binding ("*"), enabled = 1 -- and the claim
/// proceeds. INSERT OR IGNORE, so a row that already exists (or
/// lands concurrently) is never overwritten: an existing row's
/// owner and enabled flag always win over a registration
/// attempt. Extracted as a builder so the tests pin its shape
/// without a live store (the claim_update_sql pattern).
pub(crate) fn lane_provision_sql() -> &'static str {
    "INSERT OR IGNORE INTO lanes (lane_id, worker_id, queue, enabled) VALUES (?, ?, '*', 1)"
}

impl TursoBackend {
    pub fn new(url: String, token: String, agent: ureq::Agent) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token,
            agent,
            gh_probe: None,
            staleness: StalenessState::default(),
        }
    }

    /// #8276: install the GitHub dep probe used by the dep
    /// rechecker for pr/issue/ci deps.
    pub fn set_gh_probe(&mut self, probe: std::sync::Arc<dyn DepProbe>) {
        self.gh_probe = Some(probe);
    }

    pub(crate) fn gh_probe(&self) -> Option<&dyn DepProbe> {
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

    pub(crate) fn exec(&self, sql: &str, args: &[Arg]) -> Result<StmtResult, TErr> {
        Ok(self
            .pipeline(&[(sql, args.to_vec())])?
            .into_iter()
            .next()
            .unwrap())
    }

    /// #13146: several statements in one pipeline request (the
    /// placement-sync batch insert). Same per-statement results as
    /// repeated `exec` calls, one round trip.
    pub(crate) fn exec_batch(&self, stmts: &[(&str, Vec<Arg>)]) -> Result<Vec<StmtResult>, TErr> {
        self.pipeline(stmts)
    }

    /// Rows of one SELECT as JSON objects, with the given TEXT
    /// columns parsed back into JSON values.
    pub(crate) fn query_objs(
        &self,
        sql: &str,
        args: &[Arg],
        json_cols: &[&str],
    ) -> Result<Vec<Value>, TErr> {
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
                    if col == "manual_only" {
                        v = Value::Bool(v.as_i64().unwrap_or(0) != 0);
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
        // #6853: seed the canonical lane registry. Registrations
        // used to be hand-inserted rows with no code-owned seed, so
        // a store rebuild/restore silently dropped lanes that were
        // not in whoever's seed set — openchamber-a/b were disabled
        // on 2026-11-08 (rows present) and unknown by 2026-11-11
        // (rows gone). INSERT OR IGNORE fills only MISSING rows: an
        // existing registration — including a lane Drew disabled
        // (enabled = 0) — is never overwritten by the seed.
        // #8097: the canonical set is exactly the post-rename
        // registry universe, so a hand-deleted legacy lane (one
        // outside the set) is not resurrected either.
        self.exec(&lane_seed_sql(), &[])?;
        // Additive task columns land here for databases created
        // before them: #8097 Rule 2's tasks.platform (the optional
        // coarse platform selector — existing rows read NULL: unset,
        // either family may claim them) and #8176's claim-model
        // stamp (claim_provider / claim_model / claim_variant —
        // existing rows read NULL: the claim carried no model).
        // No backfill, nothing destructive.
        let cols = self.query_objs("SELECT name FROM pragma_table_info('tasks')", &[], &[])?;
        for col in [
            "platform",
            "manual_only",
            "claim_provider",
            "claim_model",
            "claim_variant",
            "claim_session_id",
        ] {
            let has = cols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col));
            if !has {
                let ty = if col == "manual_only" {
                    "INTEGER NOT NULL DEFAULT 0"
                } else {
                    "TEXT"
                };
                self.exec(&format!("ALTER TABLE tasks ADD COLUMN {col} {ty}"), &[])?;
            }
        }
        // #8276: the dependency-block columns (blocked_deps JSON,
        // dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds)
        // land the same additive way for task tables created before
        // them. No backfill: existing rows read NULL, which the
        // rechecker reads as "not a dep-block" and never touches.
        for (col, ty) in [
            ("metadata", "TEXT NOT NULL DEFAULT '{}'"),
            ("blocked_deps", "TEXT"),
            ("dep_next_recheck_at", "TEXT"),
            ("dep_blocked_until", "TEXT"),
            ("dep_recheck_seconds", "INTEGER"),
            ("required_budget_minutes", "INTEGER"),
            // #14170 Q5: the label cache (mirror's JSON array of
            // names) and its sync stamp. Existing rows read `[]` and
            // NULL — "never synced" — until a transition or the
            // label sweep copies them from the mirror.
            ("labels", "TEXT NOT NULL DEFAULT '[]'"),
            ("labels_synced_at", "TEXT"),
            // #14170 Q2: additive terminal-resolution taxonomy. Old
            // terminal rows remain NULL unless the Q2 backfill proves
            // their verdict from an existing result or unique twin.
            ("resolution", "TEXT"),
            ("resolution_category", "TEXT"),
            ("resolution_substatus", "TEXT"),
            ("resolution_reason", "TEXT"),
            ("resolution_ref", "TEXT"),
            ("resolved_by", "TEXT"),
            ("resolved_at", "TEXT"),
        ] {
            let has = cols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col));
            if !has {
                self.exec(&format!("ALTER TABLE tasks ADD COLUMN {col} {ty}"), &[])?;
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
        // #8582: the merge_intents base-tip linkage column
        // (base_head_sha -- the recorded head of an intent's base
        // at stack/restack time) lands the same additive way for
        // merge_intents tables created before it. No backfill:
        // existing rows read NULL (no linkage recorded), which the
        // taint derivation reads as unverified only inside stacks
        // that hold an abandoned intent.
        let micols = self.query_objs(
            "SELECT name FROM pragma_table_info('merge_intents')",
            &[],
            &[],
        )?;
        if !micols.is_empty()
            && !micols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some("base_head_sha"))
        {
            self.exec(
                "ALTER TABLE merge_intents ADD COLUMN base_head_sha TEXT",
                &[],
            )?;
        }
        // #13774 (ACTIONS-EXEC-3): the tiered-extension columns
        // (ext_count / ext_total_s / ext_tier / ext_decide_after /
        // ext_reason) and the session context land the same
        // additive way for actions_exec_sessions tables created
        // before them. No backfill: existing rows read the column
        // default / NULL, which the verbs read as "no extensions
        // yet, no context recorded".
        let aecols = self.query_objs(
            "SELECT name FROM pragma_table_info('actions_exec_sessions')",
            &[],
            &[],
        )?;
        if !aecols.is_empty() {
            for (col, ty) in [
                ("ext_count", "INTEGER NOT NULL DEFAULT 0"),
                ("ext_total_s", "INTEGER NOT NULL DEFAULT 0"),
                ("ext_tier", "INTEGER"),
                ("ext_decide_after", "TEXT"),
                ("ext_reason", "TEXT"),
                ("context", "TEXT"),
            ] {
                let has = aecols
                    .iter()
                    .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col));
                if !has {
                    self.exec(
                        &format!("ALTER TABLE actions_exec_sessions ADD COLUMN {col} {ty}"),
                        &[],
                    )?;
                }
            }
        }
        // #8210: the ledger's platform-limitation columns (a
        // task may be limited to one platform family, but only
        // with a good reason recorded beside it) land the same
        // additive way.
        for col in [
            "platform_limit",
            "platform_limit_reason",
            "executed_lane_id",
        ] {
            if !lcols
                .iter()
                .any(|c| c.get("name").and_then(|v| v.as_str()) == Some(col))
            {
                self.exec(
                    &format!("ALTER TABLE repo_task_lane_ledger ADD COLUMN {col} TEXT"),
                    &[],
                )?;
            }
        }
        super::links::ensure_q6_backfill(self)?;
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
            Arg::Float(f) => Arg::Float(*f),
            Arg::Null => Arg::Null,
        }
    }
}

pub(crate) const SCHEMA: [&str; 45] = [
    "CREATE TABLE IF NOT EXISTS tasks (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, \
        queue TEXT NOT NULL, manual_only INTEGER NOT NULL DEFAULT 0, title TEXT, body TEXT, \
        payload TEXT NOT NULL DEFAULT '{}', metadata TEXT NOT NULL DEFAULT '{}', dedupe_key TEXT, \
        priority INTEGER, status TEXT NOT NULL DEFAULT 'queued', \
        not_before TEXT, worker_id TEXT, claimed_at TEXT, lease_until TEXT, \
        lane_id TEXT, platform TEXT, attempts INTEGER NOT NULL DEFAULT 0, \
        max_attempts INTEGER NOT NULL DEFAULT 1000, \
        claim_provider TEXT, claim_model TEXT, claim_variant TEXT, claim_session_id TEXT, \
        progress TEXT NOT NULL DEFAULT '{}', result TEXT NOT NULL DEFAULT '{}', \
        last_error TEXT, park_kind TEXT, wait_kind TEXT, ready_condition TEXT, \
        required_budget_minutes INTEGER, \
        last_checked_at TEXT, check_outcome TEXT, \
        created_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
        completed_at TEXT, \
        blocked_deps TEXT, dep_next_recheck_at TEXT, dep_blocked_until TEXT, \
        dep_recheck_seconds INTEGER, \
        labels TEXT NOT NULL DEFAULT '[]', labels_synced_at TEXT, \
        resolution TEXT, resolution_category TEXT, resolution_substatus TEXT, \
        resolution_reason TEXT, resolution_ref TEXT, resolved_by TEXT, resolved_at TEXT)",
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
    include_str!("heartbeat_lease_mirror.sql"),
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
        task_id INTEGER NOT NULL UNIQUE, assigned_lane TEXT, executed_lane TEXT, executed_lane_id TEXT, \
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
    "CREATE TABLE IF NOT EXISTS task_history (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER UNIQUE, \
        task_id INTEGER NOT NULL, kind TEXT NOT NULL, \
        from_status TEXT, to_status TEXT, from_priority INTEGER, to_priority INTEGER, \
        actor TEXT, session_id TEXT, model_provider TEXT, model_id TEXT, \
        detail TEXT NOT NULL DEFAULT '{}', source TEXT NOT NULL DEFAULT 'live', \
        created_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS task_history_task ON task_history(task_id, id)",
    "CREATE INDEX IF NOT EXISTS task_history_created ON task_history(created_at)",
    // #14170 Q4: the append-only notes stream (turso_queue::notes).
    "CREATE TABLE IF NOT EXISTS task_notes (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL, \
        author TEXT NOT NULL, author_kind TEXT NOT NULL DEFAULT 'agent', \
        session_id TEXT, model_provider TEXT, model_id TEXT, model_variant TEXT, \
        body TEXT NOT NULL, created_at TEXT NOT NULL, \
        edited_at TEXT, deleted_at TEXT)",
    "CREATE INDEX IF NOT EXISTS task_notes_task ON task_notes(task_id, id)",
    // #14170 Q6: normalized task relationships and blocking edges.
    // link_key/dep_key make the nullable reference columns idempotent
    // on SQLite, whose UNIQUE indexes treat NULL values as distinct.
    "CREATE TABLE IF NOT EXISTS queue_links (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL, \
        other_kind TEXT NOT NULL, other_repo TEXT, other_number INTEGER, \
        other_task_id INTEGER, link_kind TEXT NOT NULL, link_key TEXT NOT NULL, \
        created_by TEXT NOT NULL, created_at TEXT NOT NULL, \
        removed_at TEXT, removed_by TEXT)",
    "CREATE INDEX IF NOT EXISTS queue_links_task ON queue_links(task_id, id)",
    "CREATE INDEX IF NOT EXISTS queue_links_target ON queue_links(other_kind, other_repo, other_number, other_task_id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS queue_links_active_key \
        ON queue_links(task_id, link_kind, link_key) WHERE removed_at IS NULL",
    "CREATE TABLE IF NOT EXISTS task_deps (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL, \
        block_id TEXT NOT NULL, dep_kind TEXT NOT NULL, dep_key TEXT NOT NULL, ref_repo TEXT, \
        ref_number INTEGER, ref_task_id INTEGER, \
        state TEXT NOT NULL DEFAULT 'open' CHECK(state IN ('open','cleared','malformed')), \
        created_at TEXT NOT NULL, cleared_at TEXT, cleared_by TEXT, \
        detail TEXT NOT NULL DEFAULT '{}', UNIQUE(task_id, block_id, dep_key))",
    "CREATE INDEX IF NOT EXISTS task_deps_task ON task_deps(task_id, state)",
    "CREATE INDEX IF NOT EXISTS task_deps_block ON task_deps(task_id, block_id, state)",
    "CREATE INDEX IF NOT EXISTS task_deps_ref ON task_deps(ref_repo, ref_number, state)",
    "CREATE TABLE IF NOT EXISTS queue_schema_migrations (\
        name TEXT PRIMARY KEY, applied_at TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}')",
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
    // #14100: durable retry for priority-category synchronization.
    // This is intentionally separate from creation_outbox: reprioritize
    // edits an existing issue and is not an issue-creation intent.
    "CREATE TABLE IF NOT EXISTS reprioritize_sync_outbox (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL, \
        repo TEXT NOT NULL, issue INTEGER NOT NULL, category TEXT NOT NULL, \
        label TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending', \
        attempts INTEGER NOT NULL DEFAULT 0, available_at TEXT NOT NULL, \
        last_error_class TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS reprioritize_sync_due \
        ON reprioritize_sync_outbox(state, available_at)",
    // #8582 increment 2: the merge-intent table, DDL of record in
    // the pure core module.
    crate::merge_intent::MERGE_INTENT_DDL,
    // #13146: placement session records — the designed home for the
    // fleet-place session store (intake DDL in
    // doc/placement-intake/placement.md, carried into the Turso
    // backend). `record` holds the full session JSON — the client
    // record shape is the contract; the scalar columns are
    // extractions for list/filter queries.
    "CREATE TABLE IF NOT EXISTS placement_sessions (\
        handle TEXT PRIMARY KEY, record TEXT NOT NULL DEFAULT '{}', \
        state TEXT, surface TEXT, submitted_by TEXT, run_id INTEGER, \
        created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS placement_sessions_created \
        ON placement_sessions(created_at)",
    // #13774: actions exec sessions — DDL of record in the
    // actions_exec module.
    crate::turso_queue::ACTIONS_EXEC_DDL_SESSIONS,
    crate::turso_queue::ACTIONS_EXEC_DDL_COMMANDS,
    crate::turso_queue::ACTIONS_EXEC_DDL_COMMANDS_IDX,
    crate::turso_queue::ACTIONS_EXEC_DDL_IO,
    crate::turso_queue::ACTIONS_EXEC_DDL_IO_IDX,
    // #13765: central placement coordinator — slot claims, the
    // cross-seat load view, the fleet-wide Blacksmith budget, and
    // the waitlist. DDL of record in the placement_coord module.
    crate::turso_queue::PLACEMENT_COORD_DDL_OWNER,
    crate::turso_queue::PLACEMENT_COORD_DDL_CLAIMS,
    crate::turso_queue::PLACEMENT_COORD_DDL_BUDGETS,
    crate::turso_queue::PLACEMENT_COORD_DDL_CLAIMS_IDX,
];

// ---------------------------------------------------------------------------
// Envelope (identical shapes to the Postgres broker dispatch)
// ---------------------------------------------------------------------------

pub(crate) fn ok(fields: Value) -> Value {
    let mut m = json!({"ok": true});
    if let (Some(dst), Some(src)) = (m.as_object_mut(), fields.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    m
}

pub(crate) fn fail(exit: i32, msg: impl Into<String>) -> Value {
    json!({"ok": false, "exit": exit, "error": msg.into()})
}

pub(crate) fn terr(e: TErr) -> Value {
    // Transport/store failures mirror the Postgres broker's exit 4.
    fail(4, e)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(crate) fn merge_obj(base: &Value, patch: &Value) -> Value {
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

pub(crate) fn get_task(t: &TursoBackend, id: i64) -> Result<Option<Value>, TErr> {
    let rows = t.query_objs(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
        &[Arg::Int(id)],
        &TASK_JSON_COLS,
    )?;
    Ok(rows.into_iter().next())
}

pub(crate) fn get_ledger(t: &TursoBackend, task_id: i64) -> Result<Option<Value>, TErr> {
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
pub(crate) fn audit_marks(task: &Value) -> Value {
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
pub(crate) fn with_marks(detail: &Value, task: &Value) -> Value {
    merge_obj(&audit_marks(task), detail)
}

/// Status transition each lifecycle event kind records in
/// task_history: (from_status, to_status). `None` means the
/// transition's prior status is not known from the event alone
/// (blocked/cancelled/failed/task_revived can come from several
/// states), so from_status is left NULL rather than guessed.
/// `reprioritized` changes priority, not status: its from/to are
/// the priority values carried in the event detail. Kinds absent
/// here (checkpoint, claim_refused, session_bound, lease_expired,
/// ...) are not status transitions and get no history row.
pub(crate) fn history_transition(
    kind: &str,
) -> Option<(Option<&'static str>, Option<&'static str>)> {
    Some(match kind {
        "enqueued" => (None, Some("queued")),
        "task_revived" => (None, Some("queued")),
        "revived" => (None, Some("queued")),
        "claimed" => (Some("queued"), Some("claimed")),
        "parked" => (Some("claimed"), Some("parked")),
        "released" => (Some("claimed"), Some("queued")),
        "woken" => (Some("parked"), Some("queued")),
        "blocked" => (None, Some("blocked")),
        "unblocked" | "dep_unblocked" => (Some("blocked"), Some("queued")),
        "completed" => (Some("claimed"), Some("done")),
        "cancelled" => (None, Some("cancelled")),
        "failed" => (None, Some("failed")),
        "reprioritized" => (None, None),
        "meta_updated" => (None, None),
        "linked" | "unlinked" => (None, None),
        _ => return None,
    })
}

/// The history row for the event just inserted by the same
/// transaction: it reads the newest events row, so it must run
/// inside the BEGIN IMMEDIATE batch that inserted that event. Marks
/// come from the event detail, which the transition wrote from its
/// own context (with_marks), never from the task's current claim.
pub(crate) const HISTORY_INSERT_SQL: &str = "INSERT OR IGNORE INTO task_history \
    (event_id, task_id, kind, from_status, to_status, from_priority, to_priority, \
     actor, session_id, model_provider, model_id, detail, source, created_at) \
    SELECT e.id, e.task_id, e.kind, ?, ?, \
        CASE WHEN e.kind = 'reprioritized' THEN json_extract(e.detail, '$.from') END, \
        CASE WHEN e.kind = 'reprioritized' THEN json_extract(e.detail, '$.to') END, \
        COALESCE(json_extract(e.detail, '$.actor'), e.worker_id), \
        json_extract(e.detail, '$.session_id'), \
        json_extract(e.detail, '$.model_provider'), \
        json_extract(e.detail, '$.model_id'), \
        e.detail, 'live', e.created_at \
    FROM events e WHERE e.id = (SELECT max(id) FROM events) AND changes() = 1";

/// Every event kind that history_transition maps: the backfill census.
pub(crate) const HISTORY_KINDS: [&str; 16] = [
    "enqueued",
    "task_revived",
    "revived",
    "claimed",
    "parked",
    "released",
    "woken",
    "blocked",
    "unblocked",
    "dep_unblocked",
    "completed",
    "cancelled",
    "failed",
    "meta_updated",
    "linked",
    "unlinked",
];

/// Backfill: one history row per mapped events row that has none
/// (INSERT OR IGNORE on event_id), source='backfill'. The from/to
/// CASE expressions come from history_transition, so backfill and
/// the live dual-write cannot disagree. `reprioritized` is included.
fn backfill_sql() -> String {
    let mut kinds: Vec<&str> = HISTORY_KINDS.to_vec();
    kinds.push("reprioritized");
    let arm = |pick: fn((Option<&'static str>, Option<&'static str>)) -> Option<&'static str>| {
        kinds
            .iter()
            .map(|k| {
                let v = history_transition(k).and_then(pick);
                format!(
                    "WHEN '{k}' THEN {}",
                    v.map(|s| format!("'{s}'")).unwrap_or_else(|| "NULL".into())
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let in_list = kinds
        .iter()
        .map(|k| format!("'{k}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT OR IGNORE INTO task_history \
         (event_id, task_id, kind, from_status, to_status, from_priority, to_priority, \
          actor, session_id, model_provider, model_id, detail, source, created_at) \
         SELECT e.id, e.task_id, e.kind, \
            CASE e.kind {from} END, CASE e.kind {to} END, \
            CASE WHEN e.kind = 'reprioritized' THEN json_extract(e.detail, '$.from') END, \
            CASE WHEN e.kind = 'reprioritized' THEN json_extract(e.detail, '$.to') END, \
            COALESCE(json_extract(e.detail, '$.actor'), e.worker_id), \
            json_extract(e.detail, '$.session_id'), \
            json_extract(e.detail, '$.model_provider'), \
            json_extract(e.detail, '$.model_id'), \
            e.detail, 'backfill', e.created_at \
         FROM events e WHERE e.task_id IS NOT NULL AND e.kind IN ({in_list})",
        from = arm(|(f, _)| f),
        to = arm(|(_, t)| t),
    )
}

/// /history-backfill: idempotent. Reports the census of mapped
/// events, the rows inserted now, and the rows skipped because a
/// history row already existed for that event.
pub(crate) fn verb_history_backfill(t: &TursoBackend, _req: &Value) -> Value {
    let mut kinds: Vec<&str> = HISTORY_KINDS.to_vec();
    kinds.push("reprioritized");
    let in_list = kinds
        .iter()
        .map(|k| format!("'{k}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let work = (|| -> Result<Value, TErr> {
        let census = t
            .query_objs(
                &format!("SELECT count(*) AS n FROM events WHERE task_id IS NOT NULL AND kind IN ({in_list})"),
                &[],
                &[],
            )?
            .first()
            .and_then(|r| r["n"].as_i64())
            .unwrap_or(0);
        let inserted = t.exec(&backfill_sql(), &[])?.affected as i64;
        Ok(json!({
            "source": "backfill",
            "census": census,
            "inserted": inserted,
            "skipped": census - inserted,
        }))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => fail(2, e),
    }
}

/// /history-since: task_history rows in id order, optionally for one
/// task and/or created at or after `since`. The per-task timeline and
/// the notes interleave read this table.
pub(crate) fn verb_history_since(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id");
    let limit = req_i64(req, "limit").unwrap_or(200).clamp(1, 50_000);
    let mut sql = "SELECT id, event_id, task_id, kind, from_status, to_status, from_priority, \
                   to_priority, actor, session_id, model_provider, model_id, detail, source, created_at \
                   FROM task_history WHERE 1 = 1"
        .to_string();
    let mut args: Vec<Arg> = vec![];
    if let Some(id) = task_id {
        sql += " AND task_id = ?";
        args.push(Arg::Int(id));
    }
    if let Some(s) = req_str(req, "since") {
        sql += " AND created_at >= ?";
        args.push(Arg::Text(s.to_string()));
    }
    sql += &format!(" ORDER BY id ASC LIMIT {limit}");
    match t.query_objs(&sql, &args, &["detail"]) {
        Ok(rows) => ok(json!({"rows": rows, "task_id": task_id})),
        Err(e) => fail(2, e),
    }
}

/// /staleness: the stewardship cycle's canned read (design §9, #14170 Q8).
/// The first query returns every actionable task whose last task_history
/// transition is older than `older_than_seconds`, oldest first; a task
/// with no history row falls back to its `updated_at` and reports a null
/// `last_history_id`, so pre-history rows stay visible instead of
/// silently dropping out of the staleness read. The second query returns
/// claimed tasks whose lease expires within `lease_cap_seconds`
/// (already-expired leases sort first). The cycle reads staleness and
/// lease pressure from this one route instead of assembling either
/// client-side from /list pages.
pub(crate) fn verb_staleness(t: &TursoBackend, req: &Value) -> Value {
    let older_than = req_i64(req, "older_than_seconds").unwrap_or(86_400).max(0);
    let lease_cap = req_i64(req, "lease_cap_seconds").unwrap_or(900).max(0);
    let limit = req_i64(req, "limit").unwrap_or(200).clamp(1, 50_000);
    let queue = req_str(req, "queue");
    let now = now_unix();
    let cutoff = iso(now - older_than);
    let lease_cutoff = iso(now + lease_cap);
    let mut stale_sql = "SELECT t.id, t.queue, t.status, t.title, t.worker_id, t.priority, \
                   h.id AS last_history_id, h.kind AS last_kind, h.to_status AS last_to_status, \
                   h.actor AS last_actor, COALESCE(h.created_at, t.updated_at) AS last_transition_at \
                   FROM tasks t \
                   LEFT JOIN task_history h ON h.id = \
                     (SELECT MAX(h2.id) FROM task_history h2 WHERE h2.task_id = t.id) \
                   WHERE t.status IN ('queued', 'claimed', 'parked', 'blocked', 'handed_off') \
                   AND COALESCE(h.created_at, t.updated_at) < ?"
        .to_string();
    let mut stale_args: Vec<Arg> = vec![Arg::Text(cutoff)];
    if let Some(q) = queue {
        stale_sql += " AND t.queue = ?";
        stale_args.push(Arg::Text(q.to_string()));
    }
    stale_sql += &format!(" ORDER BY last_transition_at ASC LIMIT {limit}");
    let rows = match t.query_objs(&stale_sql, &stale_args, &[]) {
        Ok(rows) => rows,
        Err(e) => return fail(2, e),
    };
    let mut lease_sql =
        "SELECT id, queue, status, title, worker_id, priority, claimed_at, lease_until \
                   FROM tasks \
                   WHERE status = 'claimed' AND lease_until IS NOT NULL AND lease_until <= ?"
            .to_string();
    let mut lease_args: Vec<Arg> = vec![Arg::Text(lease_cutoff)];
    if let Some(q) = queue {
        lease_sql += " AND queue = ?";
        lease_args.push(Arg::Text(q.to_string()));
    }
    lease_sql += &format!(" ORDER BY lease_until ASC LIMIT {limit}");
    let claims = match t.query_objs(&lease_sql, &lease_args, &[]) {
        Ok(rows) => rows,
        Err(e) => return fail(2, e),
    };
    ok(json!({
        "rows": rows,
        "claims_near_lease_cap": claims,
        "older_than_seconds": older_than,
        "lease_cap_seconds": lease_cap,
        "queue": queue,
        "now": iso(now),
    }))
}

/// /needs-user: the stewardship cycle's open-questions read (design §9,
/// #14170 Q8): blocked/parked tasks carrying a `question` metadata entry
/// (Q3 registry shape {ledger_id, state}), oldest last transition first,
/// with the question object lifted out of the metadata. The
/// `blocked_by_count_cache` column counts the task's `blocked_deps`
/// cache entries; it is the honest pre-Q6 source and is superseded by
/// task_deps counts when Q6 lands.
pub(crate) fn verb_needs_user(t: &TursoBackend, req: &Value) -> Value {
    let limit = req_i64(req, "limit").unwrap_or(200).clamp(1, 50_000);
    let queue = req_str(req, "queue");
    let state = req_str(req, "state");
    let mut sql = "SELECT t.id, t.queue, t.status, t.title, t.worker_id, t.priority, \
                   json_extract(t.metadata, '$.question') AS question, \
                   (SELECT MAX(h.created_at) FROM task_history h WHERE h.task_id = t.id) \
                     AS last_transition_at, \
                   CASE WHEN t.blocked_deps LIKE '[%' THEN json_array_length(t.blocked_deps) \
                        ELSE 0 END AS blocked_by_count_cache \
                   FROM tasks t \
                   WHERE t.status IN ('blocked', 'parked') \
                   AND json_extract(t.metadata, '$.question') IS NOT NULL"
        .to_string();
    let mut args: Vec<Arg> = vec![];
    if let Some(q) = queue {
        sql += " AND t.queue = ?";
        args.push(Arg::Text(q.to_string()));
    }
    if let Some(s) = state {
        sql += " AND json_extract(t.metadata, '$.question.state') = ?";
        args.push(Arg::Text(s.to_string()));
    }
    sql += &format!(" ORDER BY COALESCE(last_transition_at, t.updated_at) ASC LIMIT {limit}");
    match t.query_objs(&sql, &args, &["question"]) {
        Ok(rows) => ok(json!({"rows": rows, "queue": queue})),
        Err(e) => fail(2, e),
    }
}

/// Queue metadata read/merge-patch surface. The write and its audit row are
/// committed together; null patch values remove keys and omitted keys stand.
pub(crate) fn verb_meta(t: &TursoBackend, req: &Value) -> Value {
    let op = req_str(req, "op").unwrap_or("");
    if op == "keys" {
        return ok(
            json!({"row": crate::validate::METADATA_REGISTRY.iter().map(|(name, ty, meaning)| json!({"name":name,"type":ty,"meaning":meaning})).collect::<Vec<_>>()}),
        );
    }
    let Some(id) = req_i64(req, "task_id") else {
        return fail(2, "meta: task_id is required");
    };
    let work = (|| -> Result<Value, String> {
        let row = get_task(t, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("meta: task {id} not found"))?;
        let current = row.get("metadata").cloned().unwrap_or_else(|| json!({}));
        match op {
            "get" => {
                let out = match req_str(req, "key") {
                    Some(key) => current.get(&key).cloned().unwrap_or(Value::Null),
                    None => current,
                };
                Ok(json!({"row": out}))
            }
            "keys" => unreachable!(),
            "set" | "unset" => {
                let mut patch = req.get("patch").cloned().unwrap_or_else(|| json!({}));
                if op == "unset" {
                    let key = req_str(req, "key")
                        .ok_or_else(|| "meta unset: key is required".to_string())?;
                    let mut object = Map::new();
                    object.insert(key.to_string(), Value::Null);
                    patch = Value::Object(object);
                } else if let Some(key) = req_str(req, "key") {
                    let mut object = Map::new();
                    object.insert(
                        key.to_string(),
                        req.get("value").cloned().unwrap_or(Value::Null),
                    );
                    patch = Value::Object(object);
                }
                if !patch.is_object() {
                    return Err("meta set: patch must be a JSON object".into());
                }
                let mut next = current.as_object().cloned().unwrap_or_default();
                let mut changed = serde_json::Map::new();
                for (key, value) in patch.as_object().expect("checked object") {
                    let old = next.get(key).cloned().unwrap_or(Value::Null);
                    if value.is_null() {
                        next.remove(key);
                    } else {
                        next.insert(key.clone(), value.clone());
                    }
                    let new = next.get(key).cloned().unwrap_or(Value::Null);
                    if old != new {
                        changed.insert(
                            key.clone(),
                            json!({"old": history_value(&old), "new": history_value(&new)}),
                        );
                    }
                }
                let next = Value::Object(next);
                crate::validate::validate_metadata(&next)?;
                if changed.is_empty() {
                    return Ok(json!({"row": current, "changed": false}));
                }
                let changed_names: Vec<String> = changed.keys().cloned().collect();
                let detail = with_marks(&json!({"changed": changed, "keys": changed_names}), &row);
                let now = now_iso();
                let updated_at = row
                    .get("updated_at")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("meta: task {id} has no updated_at timestamp"))?;
                let results = t.exec_batch(&[
                    ("BEGIN IMMEDIATE", vec![]),
                    ("UPDATE tasks SET metadata = ?, updated_at = ? WHERE id = ? AND updated_at = ? AND metadata = ?", vec![Arg::Text(next.to_string()), Arg::Text(now.clone()), Arg::Int(id), Arg::Text(updated_at.to_string()), Arg::Text(current.to_string())]),
                    ("INSERT INTO events (worker_id, task_id, kind, detail, created_at) SELECT ?, ?, 'meta_updated', ?, ? WHERE changes() = 1", vec![Arg::Text("fleet queue meta".into()), Arg::Int(id), Arg::Text(detail.to_string()), Arg::Text(now)]),
                    (HISTORY_INSERT_SQL, vec![Arg::Null, Arg::Null]),
                    ("COMMIT", vec![]),
                ]).map_err(|e| e.to_string())?;
                if results.get(1).is_none_or(|result| result.affected == 0) {
                    return Err(format!(
                        "meta: task {id} changed concurrently; retry the metadata update"
                    ));
                }
                Ok(json!({"row": next, "changed": true}))
            }
            _ => Err(format!("meta: unknown operation {op:?}")),
        }
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => fail(2, e),
    }
}

fn history_value(value: &Value) -> Value {
    let bytes = serde_json::to_vec(value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if bytes > 512 {
        json!({"truncated": true, "bytes": bytes})
    } else {
        value.clone()
    }
}

pub(crate) fn event(
    t: &TursoBackend,
    worker: Option<&str>,
    task_id: Option<i64>,
    kind: &str,
    detail: &Value,
) -> Result<(), TErr> {
    // #14191: rows written while serving a request that carried
    // context name its id, so the event store joins the trip logs.
    let detail = &super::ctx_scope::stamp(detail);
    let transition = history_transition(kind).filter(|_| task_id.is_some());
    let Some((from, to)) = transition else {
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
        return Ok(());
    };
    t.exec_batch(&[
        ("BEGIN IMMEDIATE", vec![]),
        (
            "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
             VALUES (?, ?, ?, ?, ?)",
            vec![
                arg_str(worker),
                task_id.map(Arg::Int).unwrap_or(Arg::Null),
                Arg::Text(kind.to_string()),
                Arg::Text(detail.to_string()),
                Arg::Text(now_iso()),
            ],
        ),
        (HISTORY_INSERT_SQL, vec![arg_str(from), arg_str(to)]),
        ("COMMIT", vec![]),
    ])?;
    // #14170 Q5: every transition refreshes the task's label cache
    // from the mirror, after the transition has committed. Fail-open:
    // the mirror never gates work (#8575), so a missing mirror table
    // or a failed read leaves the cache as it was.
    if let Some(id) = task_id {
        let _ = refresh_task_labels(t, id);
    }
    Ok(())
}

/// Parse an /events-append request: kind is required and must be a
/// non-empty string; worker_id/task_id optional (the PG stream's
/// non-queue writers always wrote task_id NULL); detail defaults {}.
pub(crate) fn parse_event_append(
    req: &Value,
) -> Result<(Option<String>, Option<i64>, String, Value), String> {
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
pub(crate) fn prune_cutoff(now: i64, keep_days: i64) -> String {
    let days = keep_days.clamp(1, 3650);
    iso(now - days * 86_400)
}

pub(crate) fn verb_events_append(t: &TursoBackend, req: &Value) -> Value {
    let (worker, task_id, kind, detail) = match parse_event_append(req) {
        Ok(v) => v,
        Err(e) => return fail(2, e),
    };
    let detail = super::ctx_scope::stamp(&detail);
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
pub(crate) fn verb_events_seen(t: &TursoBackend, req: &Value) -> Value {
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
pub(crate) fn verb_events_count(t: &TursoBackend, req: &Value) -> Value {
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

/// #9046: read one task's event timeline — the model_stamp /
/// session_bound / session_unbound history and everything else
/// recorded on the task, oldest first (the events_task index).
/// `kind` narrows to one kind; `limit` defaults to 200 and is
/// clamped to 1..=1000.
pub(crate) fn verb_events_list(t: &TursoBackend, req: &Value) -> Value {
    // #9046 per-task read, widened for the claim-link rollup
    // (#10357): task_id is optional — without it the verb lists
    // across tasks inside a `since` window (kind-filtered), the
    // read `repo-queue models` aggregates over. Rows always
    // carry their task_id so cross-task rows join back.
    let task_id = req_i64(req, "task_id");
    let limit = req_i64(req, "limit")
        .unwrap_or(200)
        .clamp(1, if task_id.is_some() { 1000 } else { 50_000 });
    let mut sql = "SELECT id, task_id, worker_id, kind, detail, created_at FROM events \
                   WHERE 1 = 1"
        .to_string();
    let mut args: Vec<Arg> = vec![];
    if let Some(id) = task_id {
        sql += " AND task_id = ?";
        args.push(Arg::Int(id));
    }
    if let Some(k) = req_str(req, "kind") {
        sql += " AND kind = ?";
        args.push(Arg::Text(k.to_string()));
    }
    if let Some(s) = req_str(req, "since") {
        sql += " AND created_at >= ?";
        args.push(Arg::Text(s.to_string()));
    }
    sql += &format!(" ORDER BY id ASC LIMIT {limit}");
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(&sql, &args, &["detail"])?;
        Ok(json!({"rows": rows, "task_id": task_id}))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// prune_events, shared by the /events-prune verb and /sweep:
/// delete events older than keep_days and self-log an events_pruned
/// row, like the PG function. Returns (deleted, cutoff).
pub(crate) fn prune_events(t: &TursoBackend, keep_days: i64) -> Result<(i64, String), TErr> {
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
pub(crate) fn verb_events_prune(t: &TursoBackend, req: &Value) -> Value {
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
pub(crate) fn revalidate_outcome(
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
pub(crate) fn revalidate_parked(
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
pub(crate) fn verb_sweep(t: &TursoBackend, req: &Value) -> Value {
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
            Some(run_dep_rechecks(
                t,
                req_i64(req, "dep_limit").unwrap_or(200),
            )?)
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
        // #10648: the quarantine rechecker rides the sweep beside
        // the dep rechecker, on the same expire timer — a blocked
        // row whose recorded claim-path quarantine no longer
        // reproduces is requeued automatically, so a repaired
        // packet can never fossilize in blocked again. Opt out
        // per call: quarantine_recheck=false.
        let quarantine_summary = if req_bool(req, "quarantine_recheck").unwrap_or(true) {
            Some(run_quarantine_rechecks(
                t,
                req_i64(req, "quarantine_limit").unwrap_or(200),
            )?)
        } else {
            None
        };
        let quarantine_checked = quarantine_summary
            .as_ref()
            .and_then(|s| s.get("checked"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if let Some(s) = quarantine_summary {
            out["quarantine_recheck"] = s;
        }
        // #9177: the closed-bound detective rides the sweep too —
        // the same minutely timer already POSTs /sweep, so every
        // bound (session-stamped claimed|parked) task gets its
        // issue read on GitHub once per cycle and crossed with the
        // closure log; a closed issue without the bound task's
        // completion record raises one throat alert. Opt out per
        // call: closed_bound=false. The detective never fails the
        // sweep: its errors (store, unreadable log) are reported
        // inside its own summary and the cycle continues.
        let closed_bound_summary = if req_bool(req, "closed_bound").unwrap_or(true) {
            match run_closed_bound_checks(t, req_i64(req, "closed_bound_limit").unwrap_or(200)) {
                Ok(s) => Some(s),
                Err(e) => Some(json!({"error": e})),
            }
        } else {
            None
        };
        let closed_bound_missing = closed_bound_summary
            .as_ref()
            .and_then(|s| s.get("missing_record"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if let Some(s) = closed_bound_summary {
            out["closed_bound"] = s;
        }
        // #14170 Q5: the label cache sweep rides the sweep, at most
        // hourly (a checkpoint records the last pass), over
        // actionable statuses only. It never fails the sweep. Opt
        // out per call: labels_sync=false; labels_sync_force=true
        // runs it regardless of the interval.
        if req_bool(req, "labels_sync").unwrap_or(true) {
            if let Some(s) =
                sweep_task_labels(t, req_bool(req, "labels_sync_force").unwrap_or(false))
            {
                out["labels_sync"] = s;
            }
        }
        if expired > 0
            || do_revalidate
            || do_prune
            || dep_checked > 0
            || quarantine_checked > 0
            || closed_bound_missing > 0
        {
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
/// The caller may also supply `pulls` (`[{"number": 123}]`, #12280): the
/// answer then carries `covered_pulls` / `uncovered_pulls` under the same
/// live-task rule, matched on the PR dedupe-key classes (the warden's
/// `pr` key and the reconciliation's `merge-watch` key). Callers that do
/// not send `pulls` get the issue-only answer, unchanged.
///
/// Read-only by construction: it never writes tasks or events, so it is
/// safe to run on a schedule and safe to run in observe mode. The caller
/// decides what to enqueue; the dedupe key makes that idempotent.
pub(crate) fn verb_coverage(t: &TursoBackend, req: &Value) -> Value {
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
    let pulls: Option<Vec<Value>> = match req.get("pulls") {
        None => None,
        Some(v) => match v.as_array() {
            Some(a) => Some(a.clone()),
            None => return fail(2, "coverage: pulls must be an array".to_string()),
        },
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
        let mut live_prs: std::collections::HashMap<i64, (i64, String)> =
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
                live.entry(n).or_insert((id, status.clone()));
            }
            if let Some(n) = pr_number_for_repo(key, &repo) {
                live_prs.entry(n).or_insert((id, status));
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
        let mut out = json!({
            "queue": queue,
            "repo": repo,
            "open_issues": issues.len(),
            "covered": covered.len(),
            "uncovered": uncovered.len(),
            "uncovered_issues": uncovered,
            "covered_issues": covered,
        });
        if let Some(pulls) = &pulls {
            let mut covered_pulls: Vec<Value> = Vec::new();
            let mut uncovered_pulls: Vec<Value> = Vec::new();
            for pr in pulls {
                let Some(n) = pr.get("number").and_then(|v| v.as_i64()) else {
                    continue;
                };
                match live_prs.get(&n) {
                    Some((id, status)) => covered_pulls.push(json!({
                        "number": n,
                        "task": id,
                        "status": status,
                    })),
                    None => uncovered_pulls.push(json!({ "number": n })),
                }
            }
            out["covered_pulls"] = json!(covered_pulls);
            out["uncovered_pulls"] = json!(uncovered_pulls);
        }
        Ok(out)
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

/// The number after a `:<repo>:<class>:` marker in a dedupe key, with
/// the shared tail rule: what follows the digits must be empty or a
/// further `:`-separated suffix (e.g. `:dispatch:<x>`), so a malformed
/// key like `…:issue:12abc` cannot silently claim coverage for 12.
fn number_after_marker(key: &str, marker: &str) -> Option<i64> {
    let start = key.find(marker)? + marker.len();
    let rest = &key[start..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let tail = &rest[digits.len()..];
    if !(tail.is_empty() || tail.starts_with(':')) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Extract the issue number from a repo-health dedupe key for `repo`.
///
/// Accepted shapes (all seen in the store): the canonical
/// `repo-health:v1:(triage|fix):<repo>:issue:<n>` plus dispatch-suffixed
/// variants (`…:issue:<n>:dispatch:<x>`) and the legacy repo-name aliases
/// the reconciliation run of 2026-11-07 had to match by hand.
pub(crate) fn issue_number_from_dedupe(key: &str, repo: &str) -> Option<i64> {
    number_after_marker(key, &format!(":{repo}:issue:"))
}

/// Extract the PR number from a repo-health dedupe key for `repo`.
///
/// Both PR-naming key classes carry the same `:<repo>:pr:<n>` segment:
/// the stale-PR warden's shepherd key `repo-health:v1:pr:<repo>:pr:<n>`
/// and the coverage reconciliation's merge-watch key
/// `repo-health:v1:merge-watch:<repo>:pr:<n>` (#4951). One marker parse
/// covers both, so a live task of either family is PR coverage (#12280).
pub(crate) fn pr_number_from_dedupe(key: &str, repo: &str) -> Option<i64> {
    number_after_marker(key, &format!(":{repo}:pr:"))
}

/// The other name a repository's issue dedupe keys may carry.
///
/// Two repos moved to the developing-today-forks org on 2026-11-04 and
/// `dezren39/nix` was renamed to `dezren39/darwin-nix`; `validate.rs`
/// keeps both names allowlisted because in-flight tasks still carry the
/// old ones. A live task under either name is coverage for the issue, so
/// matching only the current name would report covered issues as
/// uncovered and import duplicate triage work.
pub(crate) fn legacy_alias(repo: &str) -> Option<&'static str> {
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
pub(crate) fn issue_number_for_repo(key: &str, repo: &str) -> Option<i64> {
    issue_number_from_dedupe(key, repo)
        .or_else(|| legacy_alias(repo).and_then(|alias| issue_number_from_dedupe(key, alias)))
}

/// PR number from a key under `repo` or any legacy alias of it.
pub(crate) fn pr_number_for_repo(key: &str, repo: &str) -> Option<i64> {
    pr_number_from_dedupe(key, repo)
        .or_else(|| legacy_alias(repo).and_then(|alias| pr_number_from_dedupe(key, alias)))
}

pub(crate) fn touch_worker(
    t: &TursoBackend,
    worker: &str,
    task_id: Option<i64>,
) -> Result<(), TErr> {
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
pub(crate) fn clear_binding(progress: &Value, via: &str) -> (Value, Option<String>) {
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

pub(crate) fn on_binding_cleared(
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
    )?;
    // #9046: the canonical unbound half of the binding history,
    // written beside the legacy kind its existing consumers read.
    event(
        t,
        Some(worker),
        Some(task_id),
        "session_unbound",
        &json!({"session_id": old_session, "via": via}),
    )
}

/// What lease expiry does to one claimed task (#6463): a task with
/// attempts left returns to queued; a task already at its attempt cap
/// fails terminally instead. Before #6463 the at-cap task also went
/// back to queued, where the claim query (attempts < max_attempts)
/// could never select it again -- it sat in queued forever, invisible
/// to terminal counts, with no failed event (142 such silent zombies
/// measured in repo-health-code-pilot on 2026-11-06).
#[derive(Debug, PartialEq)]
pub(crate) enum ExpiryDisposition {
    Requeue,
    FailAtCap,
}

pub(crate) fn expiry_disposition(attempts: i64, max_attempts: i64) -> ExpiryDisposition {
    if attempts >= max_attempts {
        ExpiryDisposition::FailAtCap
    } else {
        ExpiryDisposition::Requeue
    }
}

/// expire_leases (migration 0023 §1): claimed rows whose lease has
/// lapsed return to queued, binding cleared, lease rows dropped --
/// except rows already at their attempt cap, which transition to
/// failed with a lease_expired_at_cap event (#6463, mirroring
/// fail_repo_task's terminal branch). Sink-bound rows (#8234) keep
/// their sink return regardless of cap: the sink pick serves them
/// under its own rule, so the at-cap failure applies to the ordinary
/// claim path only.
/// Returns how many tasks were returned to queued (PG returned the
/// count; claim ignores it, the /sweep verb reports it, #6803).
/// At-cap failures are not counted: they were not returned to queued.
pub(crate) fn expire_leases(t: &TursoBackend) -> Result<i64, TErr> {
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
        let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let max_attempts = row
            .get("max_attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64);
        let at_cap =
            !to_sink && expiry_disposition(attempts, max_attempts) == ExpiryDisposition::FailAtCap;
        let upd = if at_cap {
            t.exec(
                "UPDATE tasks SET status = 'failed', worker_id = NULL, lease_until = NULL, \
                   last_error = ?, progress = ?, updated_at = ? \
                 WHERE id = ? AND status = 'claimed' AND lease_until < ?",
                &[
                    Arg::Text(format!(
                        "lease expired at attempt cap ({attempts}/{max_attempts})"
                    )),
                    Arg::Text(progress.to_string()),
                    Arg::Text(now.clone()),
                    Arg::Int(id),
                    Arg::Text(now.clone()),
                ],
            )?
        } else if to_sink {
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
        if at_cap {
            event(
                t,
                Some(&worker),
                Some(id),
                "lease_expired_at_cap",
                &json!({
                    "via": "expire_leases",
                    "attempts": attempts,
                    "max_attempts": max_attempts,
                    "cleared_session_id": cleared,
                }),
            )?;
        } else {
            expired += 1;
            event(
                t,
                Some(&worker),
                Some(id),
                "lease_expired",
                &json!({"via": "expire_leases", "cleared_session_id": cleared,
                        "return": if to_sink { "handed_off" } else { "queued" }}),
            )?;
        }
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

/// #7598: split an execution record into (mode, lane_id). `lane`
/// is the execution MODE ('opencode' | 'direct'); a lane id that
/// arrives in `lane` anyway (a caller that skipped the client's
/// normalization) is translated the same way the client does --
/// mode derived from its family, the id preserved -- and an
/// explicit `lane_id` is carried through. Before this split the
/// ledger recorded the surface name as executed_lane next to a
/// claim stamped with a lane id, which read as attribution
/// inversion; now the round trip records both, unambiguously.
pub(crate) fn execution_lane_fields(ex: &Value) -> (Option<String>, Option<String>) {
    let lane = ex.get("lane").and_then(|v| v.as_str());
    let lane_id = ex.get("lane_id").and_then(|v| v.as_str());
    match lane {
        Some(l) if lane_family_of(l).is_some() => {
            let mode = if lane_family_of(l) == Some("opencode") {
                "opencode"
            } else {
                "direct"
            };
            (
                Some(mode.to_string()),
                Some(lane_id.unwrap_or(l).to_string()),
            )
        }
        _ => (lane.map(str::to_string), lane_id.map(str::to_string)),
    }
}

/// record_repo_task_lane_execution, contract-shaped: stamp the
/// executed lane (+ fallback reason / session) onto the task's
/// ledger row, creating a minimal row when assignment never ran.
pub(crate) fn record_execution(
    t: &TursoBackend,
    req: &Value,
    row: &Value,
) -> Result<Option<Value>, TErr> {
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
    let (ex_mode, ex_lane_id) = execution_lane_fields(ex);
    if get_ledger(t, task_id)?.is_some() {
        t.exec(
            "UPDATE repo_task_lane_ledger SET \
               executed_lane = COALESCE(?, executed_lane), \
               executed_lane_id = COALESCE(?, executed_lane_id), \
               fallback_reason = COALESCE(?, fallback_reason), \
               session_id = COALESCE(?, session_id), \
               executed_at = COALESCE(executed_at, ?), updated_at = ? \
             WHERE task_id = ?",
            &[
                arg_str(ex_mode.as_deref()),
                arg_str(ex_lane_id.as_deref()),
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
                 (queue, task_id, assigned_lane, executed_lane, executed_lane_id, fallback_reason, session_id, assigned_at, executed_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            &[
                Arg::Text(queue),
                Arg::Int(task_id),
                arg_str(ex_mode.as_deref()),
                arg_str(ex_mode.as_deref()),
                arg_str(ex_lane_id.as_deref()),
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

pub(crate) fn verb_with_execution(
    t: &TursoBackend,
    req: &Value,
    row: Value,
) -> Result<Value, TErr> {
    if row.is_null() {
        return Ok(ok(json!({"row": Value::Null})));
    }
    match record_execution(t, req, &row)? {
        Some(ledger) => Ok(ok(json!({"row": row, "lane_ledger": ledger}))),
        None => Ok(ok(json!({"row": row}))),
    }
}

pub(crate) fn req_str<'a>(req: &'a Value, key: &str) -> Option<&'a str> {
    req.get(key).and_then(|v| v.as_str())
}

pub(crate) fn req_i64(req: &Value, key: &str) -> Option<i64> {
    req.get(key).and_then(|v| v.as_i64())
}

pub(crate) fn req_bool(req: &Value, key: &str) -> Option<bool> {
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
pub(crate) fn dedupe_partition(rows: &[Value]) -> (Option<Value>, Option<Value>) {
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
