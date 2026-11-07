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
    not_before, worker_id, claimed_at, lease_until, lane_id, attempts, max_attempts, \
    progress, result, last_error, park_kind, wait_kind, ready_condition, \
    last_checked_at, check_outcome, created_at, updated_at, completed_at";
const LEDGER_COLS: &str = "id, queue, task_id, assigned_lane, executed_lane, \
    exclusion_reason, assigned_by, session_id, fallback_reason, \
    assigned_at, executed_at, updated_at";
/// Task columns whose stored TEXT is JSON (PG jsonb) — parsed back
/// into values so the wire shape matches the Postgres broker's.
const TASK_JSON_COLS: [&str; 4] = ["payload", "progress", "result", "ready_condition"];

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

pub struct TursoBackend {
    url: String,
    token: String,
    agent: ureq::Agent,
}

type TErr = String;

impl TursoBackend {
    pub fn new(url: String, token: String, agent: ureq::Agent) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token,
            agent,
        }
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
        Ok(self.pipeline(&[(sql, args.to_vec())])?.into_iter().next().unwrap())
    }

    /// Rows of one SELECT as JSON objects, with the given TEXT
    /// columns parsed back into JSON values.
    fn query_objs(
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
                    obj.insert(col.clone(), v);
                }
                Value::Object(obj)
            })
            .collect())
    }

    pub fn ensure_schema(&self) -> Result<(), TErr> {
        let stmts: Vec<(&str, Vec<Arg>)> = SCHEMA.iter().map(|s| (*s, Vec::new())).collect();
        self.pipeline(&stmts)?;
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

const SCHEMA: [&str; 10] = [
    "CREATE TABLE IF NOT EXISTS tasks (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, \
        queue TEXT NOT NULL, title TEXT, body TEXT, \
        payload TEXT NOT NULL DEFAULT '{}', dedupe_key TEXT, \
        priority INTEGER, status TEXT NOT NULL DEFAULT 'queued', \
        not_before TEXT, worker_id TEXT, claimed_at TEXT, lease_until TEXT, \
        lane_id TEXT, attempts INTEGER NOT NULL DEFAULT 0, \
        max_attempts INTEGER NOT NULL DEFAULT 4, \
        progress TEXT NOT NULL DEFAULT '{}', result TEXT NOT NULL DEFAULT '{}', \
        last_error TEXT, park_kind TEXT, wait_kind TEXT, ready_condition TEXT, \
        last_checked_at TEXT, check_outcome TEXT, \
        created_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
        completed_at TEXT)",
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
        exclusion_reason TEXT, assigned_by TEXT, session_id TEXT, \
        fallback_reason TEXT, assigned_at TEXT, executed_at TEXT, \
        updated_at TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS events (\
        id INTEGER PRIMARY KEY AUTOINCREMENT, worker_id TEXT, task_id INTEGER, \
        kind TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '{}', \
        created_at TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS events_task ON events(task_id, id)",
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
        &format!(
            "SELECT {LEDGER_COLS} FROM repo_task_lane_ledger WHERE task_id = ?"
        ),
        &[Arg::Int(task_id)],
        &[],
    )?;
    Ok(rows.into_iter().next())
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
fn expire_leases(t: &TursoBackend) -> Result<(), TErr> {
    let now = now_iso();
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
        let (progress, cleared) = clear_binding(&progress, "expire_leases");
        let upd = t.exec(
            "UPDATE tasks SET status = 'queued', worker_id = NULL, lease_until = NULL, \
               progress = ?, updated_at = ? \
             WHERE id = ? AND status = 'claimed' AND lease_until < ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now.clone()),
                Arg::Int(id),
                Arg::Text(now.clone()),
            ],
        )?;
        if upd.affected == 0 {
            continue;
        }
        event(
            t,
            Some(&worker),
            Some(id),
            "lease_expired",
            &json!({"via": "expire_leases", "cleared_session_id": cleared}),
        )?;
        if let Some(old) = cleared {
            on_binding_cleared(t, &worker, id, &old, "expire_leases")?;
        }
    }
    t.exec(
        "DELETE FROM leases WHERE lease_until < ?",
        &[Arg::Text(now)],
    )?;
    Ok(())
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

fn verb_with_execution(
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

fn req_str<'a>(req: &'a Value, key: &str) -> Option<&'a str> {
    req.get(key).and_then(|v| v.as_str())
}

fn req_i64(req: &Value, key: &str) -> Option<i64> {
    req.get(key).and_then(|v| v.as_i64())
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

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
    let task = task_from_packet(&packet, &queue);
    if req.get("dry_run").and_then(|d| d.as_bool()).unwrap_or(false) {
        return ok(json!({"dry_run": true, "task": task, "warnings": warnings}));
    }
    let now = now_iso();
    let dedupe = task.get("dedupe_key").and_then(|v| v.as_str()).map(str::to_string);
    let work = (|| -> Result<Value, TErr> {
        if let Some(dk) = &dedupe {
            let existing = t.query_objs(
                &format!(
                    "SELECT {TASK_COLS} FROM tasks WHERE queue = ? AND dedupe_key = ?"
                ),
                &[Arg::Text(queue.clone()), Arg::Text(dk.clone())],
                &TASK_JSON_COLS,
            )?;
            if let Some(row) = existing.into_iter().next() {
                return Ok(row);
            }
        }
        let ins = t.exec(
            "INSERT INTO tasks (queue, title, body, payload, dedupe_key, priority, \
               status, not_before, max_attempts, progress, result, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?, '{}', '{}', ?, ?)",
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
                Arg::Text(now.clone()),
                Arg::Text(now),
            ],
        )?;
        let id = ins.last_insert_rowid.unwrap_or(0);
        get_task(t, id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({
            "row": row, "rpc": "enqueue_repo_task_turso", "warnings": warnings
        })),
        Ok(_) => fail(5, "enqueue_repo_task_turso returned no row"),
        Err(e) => terr(e),
    }
}

fn verb_claim(t: &TursoBackend, req: &Value) -> Value {
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    let lease_seconds = req_i64(req, "lease_seconds").unwrap_or(2700);
    let lane = req_str(req, "lane").map(str::to_string);
    let work = (|| -> Result<Option<Value>, TErr> {
        if let Some(lane_id) = &lane {
            let lanes = t.query_objs(
                "SELECT lane_id, worker_id, queue, enabled FROM lanes WHERE lane_id = ?",
                &[Arg::Text(lane_id.clone())],
                &[],
            )?;
            let Some(l) = lanes.into_iter().next() else {
                return Err(format!("claim_repo_task: unknown lane_id {lane_id}"));
            };
            if l.get("worker_id").and_then(|v| v.as_str()) != Some(worker.as_str()) {
                return Err(format!(
                    "claim_repo_task: lane {lane_id} belongs to worker {}, not {worker}",
                    l.get("worker_id").and_then(|v| v.as_str()).unwrap_or("")
                ));
            }
            if l.get("enabled").and_then(|v| v.as_i64()) == Some(0) {
                return Err(format!("claim_repo_task: lane {lane_id} is disabled"));
            }
            if l.get("queue").and_then(|v| v.as_str()) != Some(queue.as_str()) {
                return Err(format!(
                    "claim_repo_task: lane {lane_id} is for queue {}, not {queue}",
                    l.get("queue").and_then(|v| v.as_str()).unwrap_or("")
                ));
            }
        }
        expire_leases(t)?;
        let now = now_iso();
        let lease_until = after_iso(lease_seconds);
        // One conditional UPDATE = the claim. The subquery picks the
        // same row PG's SKIP LOCKED pick would (priority, then age;
        // parked rows whose not_before has passed resume without an
        // attempts increment); the outer status re-check keeps the
        // statement atomic against a competing claim.
        let upd = t.exec(
            "UPDATE tasks SET status = 'claimed', worker_id = ?, claimed_at = ?, \
               lease_until = ?, \
               attempts = attempts + (CASE WHEN status = 'parked' THEN 0 ELSE 1 END), \
               lane_id = ?, park_kind = NULL, wait_kind = NULL, not_before = NULL, \
               updated_at = ? \
             WHERE id = ( \
               SELECT id FROM tasks WHERE queue = ? AND ( \
                 (status = 'queued' AND attempts < max_attempts \
                   AND (not_before IS NULL OR not_before <= ?)) \
                 OR (status = 'parked' AND not_before IS NOT NULL AND not_before <= ?)) \
               ORDER BY (priority IS NULL), priority ASC, created_at ASC LIMIT 1) \
             AND status IN ('queued', 'parked')",
            &[
                Arg::Text(worker.clone()),
                Arg::Text(now.clone()),
                Arg::Text(lease_until.clone()),
                arg_str(lane.as_deref()),
                Arg::Text(now.clone()),
                Arg::Text(queue.clone()),
                Arg::Text(now.clone()),
                Arg::Text(now.clone()),
            ],
        )?;
        if upd.affected == 0 {
            return Ok(None);
        }
        let rows = t.query_objs(
            &format!(
                "SELECT {TASK_COLS} FROM tasks \
                 WHERE queue = ? AND worker_id = ? AND status = 'claimed' \
                 ORDER BY claimed_at DESC LIMIT 1"
            ),
            &[Arg::Text(queue.clone()), Arg::Text(worker.clone())],
            &TASK_JSON_COLS,
        )?;
        let Some(mut row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
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
        touch_worker(t, &worker, Some(id))?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, id, old, "claim_repo_task")?;
        }
        event(
            t,
            Some(&worker),
            Some(id),
            "claimed",
            &json!({
                "lease_seconds": lease_seconds, "via": "claim_repo_task_turso",
                "cleared_session_id": cleared, "lane_id": lane,
            }),
        )?;
        Ok(Some(row))
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
    let work = (|| -> Result<Value, TErr> {
        let Some(task) = get_task(t, task_id)? else {
            return Err(format!(
                "assign_repo_task_lane: task {task_id} does not exist"
            ));
        };
        if let Some(existing) = get_ledger(t, task_id)? {
            // Idempotent (migration 0021): keep progress.lane in step.
            let lane = existing
                .get("assigned_lane")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
            return Ok(existing);
        }
        let queue = task
            .get("queue")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let counts = t.query_objs(
            "SELECT \
               COALESCE(SUM(CASE WHEN COALESCE(executed_lane, assigned_lane) = 'opencode' THEN 1 ELSE 0 END), 0) AS o, \
               COALESCE(SUM(CASE WHEN COALESCE(executed_lane, assigned_lane) = 'direct' THEN 1 ELSE 0 END), 0) AS d \
             FROM repo_task_lane_ledger WHERE queue = ? AND exclusion_reason IS NULL",
            &[Arg::Text(queue.clone())],
            &[],
        )?;
        let (o_count, d_count) = counts
            .first()
            .map(|r| {
                (
                    r.get("o").and_then(|v| v.as_i64()).unwrap_or(0),
                    r.get("d").and_then(|v| v.as_i64()).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let lane = if exclusion.is_some() {
            "direct"
        } else if o_count <= d_count {
            "opencode"
        } else {
            "direct"
        };
        let now = now_iso();
        t.exec(
            "INSERT INTO repo_task_lane_ledger \
             (queue, task_id, assigned_lane, exclusion_reason, assigned_by, assigned_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                Arg::Text(queue),
                Arg::Int(task_id),
                Arg::Text(lane.to_string()),
                arg_str(exclusion.as_deref()),
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
            &json!({
                "assigned_lane": lane, "exclusion_reason": exclusion,
                "opencode_count": o_count, "direct_count": d_count,
                "via": "assign_repo_task_lane_turso",
            }),
        )?;
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
                task.get("lease_until").and_then(|v| v.as_str()).unwrap_or("")
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
            &json!({
                "session_id": session_id, "launched_by": launched_by,
                "via": "bind_repo_task_lane_session_turso",
            }),
        )?;
        get_ledger(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) if e.starts_with("bind_repo_task_lane_session:") => fail(4, e),
        Err(e) => terr(e),
    }
}

/// Shared guard for the claimed-by verbs: returns the task row or
/// the PG-shaped refusal.
fn claimed_by(
    t: &TursoBackend,
    task_id: i64,
    worker: &str,
    verb: &str,
) -> Result<Value, Value> {
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
    let extend = req_i64(req, "extend_seconds").unwrap_or(900);
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
        &merge_obj(&given, &json!({"last_checkpoint": {"stage": stage, "at": now_iso()}})),
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
                Arg::Text(
                    json!({"stage": stage, "progress": given, "at": now_iso()}).to_string(),
                ),
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
        Some("ci") | Some("ci_green") | Some("pr_merged") | Some("issue_closed")
        | Some("upstream_pr") => "ci",
        Some("review") | Some("codex_review") | Some("pr_review")
        | Some("person_answer") => "review",
        Some("capacity") | Some("quota_reset") => "capacity",
        _ => "external",
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
    let condition = req
        .get("ready_condition")
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| given_progress.get("ready_condition").cloned())
        .unwrap_or_else(|| json!({"kind": "time", "until": not_before}));
    let wk = wait_kind(&condition);
    let (progress, cleared) = clear_binding(
        &row.get("progress").cloned().unwrap_or(json!({})),
        "park_task",
    );
    let progress = merge_obj(&progress, &given_progress);
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"park_reason": reason}),
    );
    let work = (|| -> Result<Value, TErr> {
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
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, task_id, old, "park_task")?;
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "parked",
            &json!({
                "reason": reason, "park_kind": park_kind, "wait_kind": wk,
                "not_before": not_before, "ready_condition": condition,
            }),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
    })();
    match work {
        Ok(v) => v,
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
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET status = 'blocked', worker_id = NULL, lease_until = NULL, \
               progress = ?, result = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
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
            "blocked",
            &json!({"blocker": blocker, "reason_class": reason_class}),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => terr(e),
    }
}

fn verb_cancel(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("fleet-monitor").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, TErr> {
        let Some(row) = get_task(t, task_id)? else {
            return Ok(Value::Null);
        };
        if ["done", "cancelled", "failed"].contains(
            &row.get("status").and_then(|v| v.as_str()).unwrap_or(""),
        ) {
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
        Ok(_) => fail(5, format!("cancel_repo_task: task {task_id} does not exist")),
        Err(e) => terr(e),
    }
}

fn verb_fail(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let error = req_str(req, "error").unwrap_or("").to_string();
    let reason_class = req_str(req, "reason_class").unwrap_or("transient").to_string();
    let row = match claimed_by(t, task_id, &worker, "fail_repo_task") {
        Ok(r) => r,
        Err(v) => return v,
    };
    // Migration 0023 §2, faithfully: transient + attempts left
    // requeues with a backoff (900 s first, else 2700 s); 'blocked'
    // blocks; anything else fails terminally.
    let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
    let max_attempts = row.get("max_attempts").and_then(|v| v.as_i64()).unwrap_or(4);
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
            &json!({
                "error": error, "reason_class": reason_class, "requeued": requeue,
                "via": "fail_repo_task_turso",
            }),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

fn verb_complete(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "complete_task") {
        Ok(r) => r,
        Err(v) => return v,
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
            &json!({"via": "complete_task_turso"}),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
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
         worker_id, lease_until, lane_id, attempts, max_attempts, progress"
    } else {
        "id, queue, title, priority, status, dedupe_key, created_at, not_before"
    };
    let mut sql = format!(
        "SELECT {select} FROM tasks WHERE queue = ?"
    );
    let mut args: Vec<Arg> = vec![Arg::Text(queue)];
    if let Some(s) = req_str(req, "status") {
        sql += " AND status = ?";
        args.push(Arg::Text(s.to_string()));
    }
    if let Some(nb) = req_str(req, "not_before") {
        sql += " AND (not_before IS NULL OR not_before <= ?)";
        args.push(Arg::Text(nb.to_string()));
    }
    sql += &format!(
        " ORDER BY (priority IS NULL), priority ASC, created_at ASC LIMIT {limit}"
    );
    let json_cols: &[&str] = if rich { &["progress"] } else { &[] };
    match t.query_objs(&sql, &args, json_cols) {
        Ok(rows) => ok(json!({"rows": rows})),
        Err(e) => terr(e),
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
            let entry = by_queue.entry(q.clone()).or_insert_with(|| {
                json!({
                    "queue": q, "queued": 0, "claimed": 0, "parked": 0,
                    "blocked": 0, "done": 0, "cancelled": 0, "failed": 0,
                    "total": 0, "oldest_queued_at": Value::Null,
                })
            });
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

/// The Turso backend's half of the broker dispatch: every verb the
/// Postgres path serves except /validate (purely local) and
/// /healthz (transport-level), which stay in queue_broker::dispatch.
pub fn dispatch(t: &TursoBackend, path: &str, req: &Value) -> Value {
    match path {
        "/enqueue" => verb_enqueue(t, req),
        "/claim" => verb_claim(t, req),
        "/lane" => verb_lane(t, req),
        "/bind" => verb_bind(t, req),
        "/heartbeat" => verb_heartbeat(t, req),
        "/checkpoint" => verb_checkpoint(t, req),
        "/park" => verb_park(t, req),
        "/block" => verb_block(t, req),
        "/cancel" => verb_cancel(t, req),
        "/fail" => verb_fail(t, req),
        "/complete" => verb_complete(t, req),
        "/list" => verb_list(t, req),
        "/health" => verb_health(t, req),
        "/signal-count" => verb_signal_count(t, req),
        _ => fail(2, format!("unknown command {path}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
    fn wait_kind_mapping_matches_pg_park() {
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
    fn merge_obj_is_shallow_and_object_only() {
        let out = merge_obj(&json!({"a": 1, "b": 2}), &json!({"b": 3, "c": 4}));
        assert_eq!(out, json!({"a": 1, "b": 3, "c": 4}));
        assert_eq!(merge_obj(&json!(null), &json!({"a": 1})), json!({"a": 1}));
    }
}
