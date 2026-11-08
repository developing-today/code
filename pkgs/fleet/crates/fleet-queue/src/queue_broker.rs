//! queue-broker — the repo-health queue as a resident laptop service
//! (code#5362). Server-side port of repo-queue.py's HTTP layer: the
//! Supabase service-role key lives only here, fetched from the
//! secrets broker; all queue semantics stay in the Postgres RPCs.
//! Sandbox clients reach it only through the session proxy.
//!
//! Protocol: POST /<command> with a JSON args object; response is a
//! JSON envelope {ok:true, ...} or {ok:false, exit, error} where exit
//! mirrors repo-queue.py's exit codes (2 validation, 3 config, 4 HTTP,
//! 5 no row). Auth: X-Agent-Token header.
//!
//! Env: QUEUE_BROKER_BIND (default 100.99.113.103:4801),
//!      QUEUE_BROKER_SOCKET (Unix socket dual-bind, #6216; default
//!      ~/.config/queue-broker/broker.sock via fleet-config),
//!      QUEUE_BROKER_TOKEN_FILE (default ~/.config/queue-broker/token),
//!      QUEUE_BROKER_SECRETS_URL (default http://100.99.113.103:4800),
//!      QUEUE_BROKER_SECRETS_TOKEN_FILE (default ~/.config/secrets-broker/token).
//!
//! Backend (#6741): QUEUE_BROKER_BACKEND=postgres (default; the
//! Supabase PostgREST path above) or =turso — the queue core on the
//! dedicated Turso DB `fleet-queue` (crate::turso_queue), selected by
//! QUEUE_BROKER_TURSO_URL / QUEUE_BROKER_TURSO_TOKEN_FILE. The
//! production default does not flip with the code; cutover is
//! doc/fleet/queue-turso-cutover.md.

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use fleet_secrets::{Resolver, SecretError, SecretSource, SecretValue};
use serde_json::{Value, json};
use tiny_http::{Header, Response, Server, StatusCode};

const PILOT_REPO: &str = "developing-today/code";

/// #6450 posture (Drew ruling 2026-11-06): broker-side health cache
/// TTL — inside the recorded 10-15 s band. Simultaneous lane gate
/// calls collapse to one `repo_queue_health` view read per TTL
/// window instead of one read per lane.
const HEALTH_CACHE_TTL: Duration = Duration::from_secs(12);

/// #6450 posture: enqueue pacing token bucket — a burst allowance,
/// then a sustained refill rate. A producer burst of N tasks now
/// spreads over ~(N - burst) / refill seconds against agent.tasks
/// instead of landing inside a single second while claims are in
/// flight (the 12:22Z burst that preceded the 12:23Z window).
/// Single enqueues are unaffected while the bucket holds tokens.
const ENQUEUE_BURST: f64 = 20.0;
const ENQUEUE_REFILL_PER_SEC: f64 = 20.0;

#[derive(Clone)]
struct Creds {
    key: String,
    base: String, // project URL, no trailing slash
}

/// One cached `/health` read: the rows of the `repo_queue_health`
/// view for one queue filter (the key), and when they were fetched.
struct HealthCacheEntry {
    key: String,
    at: Instant,
    rows: Value,
}

/// Token-bucket state for enqueue pacing (see ENQUEUE_BURST).
struct EnqueueGate {
    tokens: f64,
    last: Instant,
}

impl EnqueueGate {
    fn new() -> Self {
        Self {
            tokens: ENQUEUE_BURST,
            last: Instant::now(),
        }
    }
}

struct State {
    secrets_url: String,
    secrets_token: String,
    creds: RwLock<Option<Creds>>,
    agent: ureq::Agent,
    health_cache: RwLock<Option<HealthCacheEntry>>,
    enqueue_gate: Mutex<EnqueueGate>,
    failure_events_path: Option<std::path::PathBuf>,
    /// #6741: Some when QUEUE_BROKER_BACKEND=turso; dispatch routes
    /// every verb except /validate + /healthz to the Turso backend.
    turso: Option<crate::turso_queue::TursoBackend>,
}

struct BErr {
    exit: i32,
    msg: String,
}

fn err4(msg: String) -> BErr {
    BErr { exit: 4, msg }
}

/// The queue broker's secrets-by-name source: a direct HTTP call to
/// the secrets broker on this host (the two brokers are colocated;
/// no session proxy sits between them). The wire exchange is the
/// established one — GET /secret/<name> with the X-Agent-Token
/// header — and the error strings preserve the standalone broker's
/// wording inside the platform crate's error shape.
struct DirectSecrets {
    url: String,
    token: String,
    agent: ureq::Agent,
}

impl SecretSource for DirectSecrets {
    fn source_name(&self) -> &'static str {
        "secrets-broker"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail,
        };
        let url = format!("{}/secret/{}", self.url, name);
        let mut resp = self
            .agent
            .get(&url)
            .header("X-Agent-Token", &self.token)
            .call()
            .map_err(|e| unavailable(format!("secrets broker unreachable: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        if status == 404 {
            return Err(SecretError::NotFound {
                source: "secrets-broker",
                name: name.to_string(),
            });
        }
        if status >= 300 {
            return Err(unavailable(format!(
                "secrets broker HTTP {status} for {name}: {}",
                trim(&text, 200)
            )));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| unavailable(format!("secrets broker bad JSON for {name}: {e}")))?;
        v.get("value")
            .and_then(|x| x.as_str())
            .map(|s| SecretValue::from_resolved(s.to_string()))
            .ok_or(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: format!("secrets broker response for {name} has no value"),
            })
    }
}

impl State {
    fn fetch_creds(&self) -> Result<Creds, BErr> {
        // Secrets resolve by name through the platform crate
        // (fleet-secrets, design §1): the one-off HTTP exchange with
        // the colocated secrets broker is a SecretSource, and the
        // names flow through validate_name / SecretValue / the chain
        // error shape like every other fleet consumer.
        let resolver = Resolver::new(vec![Box::new(DirectSecrets {
            url: self.secrets_url.clone(),
            token: self.secrets_token.clone(),
            agent: self.agent.clone(),
        })]);
        let get_secret = |name: &str| -> Result<String, BErr> {
            resolver
                .resolve(name)
                .map(|v| v.expose().to_string())
                .map_err(|e| BErr {
                    exit: 3,
                    msg: e.to_string(),
                })
        };
        // Startup race: the secrets broker may still be doing its first
        // vault load; retry 503s for up to ~60 s.
        let mut last_err = None;
        for _ in 0..20 {
            match (
                get_secret("supabase.muse-dev2.service-role-key"),
                get_secret("supabase.muse-dev2.project-url"),
            ) {
                (Ok(key), Ok(url)) => {
                    let creds = Creds {
                        key,
                        base: url.trim_end_matches('/').to_string(),
                    };
                    *self.creds.write().unwrap() = Some(creds.clone());
                    return Ok(creds);
                }
                (Err(e), _) | (_, Err(e)) => {
                    let retryable = e.msg.contains("503");
                    last_err = Some(e);
                    if !retryable {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
        Err(last_err.unwrap_or(BErr {
            exit: 3,
            msg: "credential fetch failed".into(),
        }))
    }

    fn creds(&self) -> Result<Creds, BErr> {
        if let Some(c) = self.creds.read().unwrap().clone() {
            return Ok(c);
        }
        self.fetch_creds()
    }

    /// One PostgREST request (a single credential-refresh cycle).
    /// Returns (parsed body, content-range). The retry posture lives
    /// in the `postgrest` wrapper below.
    fn postgrest_once(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        count_exact: bool,
    ) -> Result<(Value, Option<String>), BErr> {
        for attempt in 0..2 {
            let creds = self.creds()?;
            let url = format!("{}/rest/v1/{}", creds.base, path);
            let bearer = format!("Bearer {}", creds.key);
            let mut headers: Vec<(&str, &str)> = vec![
                ("apikey", &creds.key),
                ("Authorization", &bearer),
                ("Accept-Profile", "agent"),
            ];
            if method != "GET" {
                headers.push(("Content-Profile", "agent"));
                headers.push(("Content-Type", "application/json"));
            }
            if count_exact {
                headers.push(("Prefer", "count=exact"));
            }
            // ureq's builders are typestated (WithBody vs WithoutBody),
            // so each arm applies the same header set itself.
            let sent = match (method, body) {
                ("GET", _) => {
                    let mut r = self.agent.get(&url);
                    for (k, v) in &headers {
                        r = r.header(*k, *v);
                    }
                    r.call()
                }
                (_, Some(b)) => {
                    let mut r = self.agent.post(&url);
                    for (k, v) in &headers {
                        r = r.header(*k, *v);
                    }
                    r.send_json(b.clone())
                }
                (_, None) => {
                    let mut r = self.agent.post(&url);
                    for (k, v) in &headers {
                        r = r.header(*k, *v);
                    }
                    r.send("")
                }
            };
            let mut resp = sent.map_err(|e| err4(format!("PostgREST request failed: {e}")))?;
            let status = resp.status().as_u16();
            let content_range = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let text = resp.body_mut().read_to_string().unwrap_or_default();
            if status == 401 && attempt == 0 {
                // Key may have rotated: refetch from the secrets broker.
                *self.creds.write().unwrap() = None;
                continue;
            }
            if status >= 300 {
                return Err(err4(format!(
                    "HTTP {status} from PostgREST: {}",
                    trim(text.trim(), 500)
                )));
            }
            let parsed = if text.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text)
                    .map_err(|e| err4(format!("PostgREST returned non-JSON: {e}")))?
            };
            return Ok((parsed, content_range));
        }
        Err(err4(
            "PostgREST request failed after credential refresh".into(),
        ))
    }

    /// PostgREST with the #6450 posture backoff (crate::backoff):
    /// 57014 / PGRST002-class failures retry under the shared
    /// policy. A final failure of that class is also the standing
    /// capture's trigger: it is appended to the failure-events file
    /// for the capture job to mine (arm (a) of the #6450 ruling).
    fn postgrest(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        count_exact: bool,
    ) -> Result<(Value, Option<String>), BErr> {
        let mut attempt: u32 = 0;
        loop {
            match self.postgrest_once(method, path, body, count_exact) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if !crate::backoff::is_retryable(&e.msg) {
                        return Err(e);
                    }
                    if attempt + 1 >= crate::backoff::MAX_ATTEMPTS {
                        self.record_failure_event(path, &e.msg, attempt + 1);
                        return Err(e);
                    }
                    let slept = crate::backoff::sleep_after_failure(attempt);
                    crate::log::info(
                        "queue-broker",
                        &format!(
                            "postgrest {path} 57014-class failure (attempt {}): {}; backing off {}ms",
                            attempt + 1,
                            trim(&e.msg, 160),
                            slept.as_millis()
                        ),
                    );
                    attempt += 1;
                }
            }
        }
    }

    /// Append one failure-window record for the standing capture.
    /// Best-effort: a logging failure never fails a request.
    fn record_failure_event(&self, path: &str, msg: &str, attempts: u32) {
        crate::log::info(
            "queue-broker",
            &format!(
                "failure window: postgrest {path} exhausted backoff after {attempts} attempts: {}",
                trim(msg, 200)
            ),
        );
        let Some(file) = &self.failure_events_path else {
            return;
        };
        let ts_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let health_age_ms = self
            .health_cache
            .read()
            .unwrap()
            .as_ref()
            .map(|e| e.at.elapsed().as_millis() as u64);
        let line = json!({
            "ts_epoch": ts_epoch,
            "path": path,
            "attempts": attempts,
            "error": trim(msg, 300),
            "health_cache_age_ms": health_age_ms,
        });
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
    }

    /// The cached health rows for `key` when fresher than the TTL,
    /// with their age in milliseconds.
    fn health_cache_get(&self, key: &str) -> Option<(Value, u64)> {
        let guard = self.health_cache.read().unwrap();
        let entry = guard.as_ref()?;
        if entry.key != key {
            return None;
        }
        let age = entry.at.elapsed();
        if age > HEALTH_CACHE_TTL {
            return None;
        }
        Some((entry.rows.clone(), age.as_millis() as u64))
    }

    fn health_cache_put(&self, key: &str, rows: Value) {
        *self.health_cache.write().unwrap() = Some(HealthCacheEntry {
            key: key.to_string(),
            at: Instant::now(),
            rows,
        });
    }

    /// #6450 posture: pace the enqueue RPC through the token bucket
    /// (burst ENQUEUE_BURST, refilled at ENQUEUE_REFILL_PER_SEC).
    fn pace_enqueue(&self) {
        loop {
            let wait = {
                let mut g = self.enqueue_gate.lock().unwrap();
                let now = Instant::now();
                let elapsed = now.duration_since(g.last).as_secs_f64();
                g.tokens = (g.tokens + elapsed * ENQUEUE_REFILL_PER_SEC).min(ENQUEUE_BURST);
                g.last = now;
                if g.tokens >= 1.0 {
                    g.tokens -= 1.0;
                    return;
                }
                Duration::from_secs_f64((1.0 - g.tokens) / ENQUEUE_REFILL_PER_SEC)
            };
            std::thread::sleep(wait.min(Duration::from_millis(100)));
        }
    }

    fn rpc(&self, fn_name: &str, args: Value) -> Result<Value, BErr> {
        let (v, _) = self.postgrest("POST", &format!("rpc/{fn_name}"), Some(&args), false)?;
        Ok(v)
    }
}

fn trim(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Percent-encode like Python urllib quote(s, safe).
fn quote(s: &str, safe: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "-._~".contains(c) || safe.contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `/list` limit, validated before it is interpolated into the
/// PostgREST path (#6419): an out-of-range value from any client is
/// rejected with the same usage error the client itself raises
/// (design #6448 Phase 3: reject, not clamp). Absent means the
/// existing default of 20.
fn list_limit(req: &Value) -> Result<i64, String> {
    let raw = req.get("limit").and_then(|l| l.as_i64());
    crate::client::list_limit_or_default(raw)
}

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

fn record_execution(st: &State, req: &Value, row: &Value) -> Result<Option<Value>, BErr> {
    let Some(ex) = req.get("execution").filter(|e| e.is_object()) else {
        return Ok(None);
    };
    let ledger = st.rpc(
        "record_repo_task_lane_execution",
        json!({
            "p_task_id": req.get("task_id"),
            "p_worker": req.get("worker"),
            "p_executed_lane": ex.get("lane"),
            "p_fallback_reason": ex.get("fallback_reason"),
            "p_session_id": ex.get("session_id"),
        }),
    )?;
    let _ = row;
    Ok(Some(ledger))
}

/// Verbs whose RPC takes the args object nearly verbatim.
fn simple_rpc(st: &State, fn_name: &str, args: Value) -> Value {
    match st.rpc(fn_name, args) {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => fail(e.exit, e.msg),
    }
}

fn verb_with_execution(st: &State, fn_name: &str, args: Value, req: &Value) -> Value {
    let row = match st.rpc(fn_name, args) {
        Ok(r) => r,
        Err(e) => return fail(e.exit, e.msg),
    };
    if row.is_null() {
        return ok(json!({"row": Value::Null}));
    }
    match record_execution(st, req, &row) {
        Ok(Some(ledger)) => ok(json!({"row": row, "lane_ledger": ledger})),
        Ok(None) => ok(json!({"row": row})),
        Err(e) => fail(e.exit, e.msg),
    }
}

fn dispatch(st: &State, path: &str, req: &Value) -> Value {
    if let Some(t) = &st.turso {
        if path != "/validate" && path != "/healthz" {
            return crate::turso_queue::dispatch(t, path, req);
        }
    }
    match path {
        "/validate" => {
            let packet = req.get("packet").cloned().unwrap_or(Value::Null);
            let (errors, warnings) = crate::validate::validate_packet(&packet);
            ok(json!({"errors": errors, "warnings": warnings}))
        }
        "/enqueue" => {
            let packet = req.get("packet").cloned().unwrap_or(Value::Null);
            let (errors, warnings) = crate::validate::validate_packet(&packet);
            if !errors.is_empty() {
                return json!({
                    "ok": false, "exit": 2,
                    "error": format!("packet validation failed ({} error(s))", errors.len()),
                    "errors": errors, "warnings": warnings,
                });
            }
            let queue = req
                .get("queue")
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            let task = crate::validate::task_from_packet(&packet, &queue);
            if req
                .get("dry_run")
                .and_then(|d| d.as_bool())
                .unwrap_or(false)
            {
                return ok(json!({"dry_run": true, "task": task, "warnings": warnings}));
            }
            let repo = packet
                .get("payload")
                .and_then(|p| p.get("repo"))
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            let rpc_name = match repo.as_deref() {
                None | Some(PILOT_REPO) => "enqueue_repo_task",
                _ => "enqueue_repo_task_fleet",
            };
            let args = json!({
                "p_queue": task.get("queue"),
                "p_title": task.get("title"),
                "p_body": task.get("body"),
                "p_payload": task.get("payload"),
                "p_dedupe_key": task.get("dedupe_key"),
                "p_priority": task.get("priority"),
                "p_not_before": task.get("not_before"),
                "p_max_attempts": task.get("max_attempts"),
            });
            st.pace_enqueue();
            match st.rpc(rpc_name, args) {
                Ok(row) if !row.is_null() => {
                    ok(json!({"row": row, "rpc": rpc_name, "warnings": warnings}))
                }
                Ok(_) => fail(5, format!("{rpc_name} returned no row")),
                Err(e) => fail(e.exit, e.msg),
            }
        }
        "/claim" => {
            let mut args = json!({
                "p_worker": req.get("worker"),
                "p_queue": req.get("queue"),
                "p_lease_seconds": req.get("lease_seconds"),
            });
            if let Some(lane) = req.get("lane").filter(|l| !l.is_null()) {
                args["p_lane_id"] = lane.clone();
            }
            simple_rpc(st, "claim_repo_task", args)
        }
        "/lane" => simple_rpc(
            st,
            "assign_repo_task_lane",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_exclusion_reason": req.get("exclusion_reason"),
            }),
        ),
        "/bind" => simple_rpc(
            st,
            "bind_repo_task_lane_session",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_session_id": req.get("session_id"),
                "p_launched_by": req.get("launched_by"),
            }),
        ),
        "/heartbeat" => simple_rpc(
            st,
            "heartbeat_task",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_extend_seconds": req.get("extend_seconds"),
                "p_progress": req.get("progress"),
            }),
        ),
        "/checkpoint" => simple_rpc(
            st,
            "checkpoint_task",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_stage": req.get("stage"),
                "p_progress": req.get("progress").cloned().unwrap_or(json!({})),
            }),
        ),
        "/park" => {
            let mut args = json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_reason": req.get("reason"),
                "p_progress": req.get("progress").cloned().unwrap_or(json!({})),
            });
            if let Some(nb) = req.get("not_before").filter(|v| !v.is_null()) {
                args["p_not_before"] = nb.clone();
            } else if let Some(d) = req.get("delay_seconds").filter(|v| !v.is_null()) {
                args["p_delay_seconds"] = d.clone();
            }
            if let Some(rc) = req.get("ready_condition").filter(|v| !v.is_null()) {
                args["p_ready_condition"] = rc.clone();
            }
            verb_with_execution(st, "park_task", args, req)
        }
        "/block" => simple_rpc(
            st,
            "block_task",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_reason_class": req.get("reason_class"),
                "p_blocker": req.get("blocker"),
                "p_progress": req.get("progress").cloned().unwrap_or(json!({})),
            }),
        ),
        "/cancel" => simple_rpc(
            st,
            "cancel_repo_task",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_reason": req.get("reason"),
            }),
        ),
        "/fail" => verb_with_execution(
            st,
            "fail_repo_task",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_error": req.get("error"),
                "p_reason_class": req.get("reason_class"),
            }),
            req,
        ),
        "/complete" => verb_with_execution(
            st,
            "complete_task",
            json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_result": req.get("result").cloned().unwrap_or(json!({})),
            }),
            req,
        ),
        "/list" => {
            let queue = req.get("queue").and_then(|q| q.as_str()).unwrap_or("");
            let limit = match list_limit(req) {
                Ok(limit) => limit,
                Err(e) => return fail(2, e),
            };
            // `rich` adds the claim/binding fields the on-laptop
            // opencode claimant's poll.py needs (it reads through the
            // broker since the binary-direct migration retired the
            // Python helper it used to import).
            let select = if req.get("rich").and_then(|r| r.as_bool()).unwrap_or(false) {
                "id,queue,title,priority,status,dedupe_key,created_at,not_before,worker_id,lease_until,lane_id,attempts,max_attempts,progress,payload"
            } else {
                "id,queue,title,priority,status,dedupe_key,created_at,not_before"
            };
            let mut path = format!(
                "tasks?select={select}&queue=eq.{}&order=priority.asc,created_at.asc&limit={limit}",
                quote(queue, "")
            );
            if let Some(s) = req.get("status").and_then(|s| s.as_str()) {
                path += &format!("&status=eq.{}", quote(s, ""));
            }
            if let Some(nb) = req.get("not_before").and_then(|s| s.as_str()) {
                path += &format!("&or=(not_before.is.null,not_before.lte.{})", quote(nb, ":"));
            }
            // #5444: --lane/--worker list filters, PostgREST side
            // (the Turso verb_list applies the same filters).
            if let Some(l) = req.get("lane").and_then(|s| s.as_str()) {
                path += &format!("&lane_id=eq.{}", quote(l, ""));
            }
            if let Some(w) = req.get("worker").and_then(|s| s.as_str()) {
                path += &format!("&worker_id=eq.{}", quote(w, ""));
            }
            match st.postgrest("GET", &path, None, false) {
                Ok((rows, _)) => ok(json!({"rows": rows})),
                Err(e) => fail(e.exit, e.msg),
            }
        }
        "/health" => {
            let creds = match st.creds() {
                Ok(c) => c,
                Err(e) => return fail(e.exit, e.msg),
            };
            let netloc = creds
                .base
                .split("://")
                .nth(1)
                .unwrap_or(&creds.base)
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
            let mut path = "repo_queue_health?select=*".to_string();
            if let Some(q) = req.get("queue").and_then(|q| q.as_str()) {
                path += &format!("&queue=eq.{}", quote(q, ""));
            }
            let cache_key = req
                .get("queue")
                .and_then(|q| q.as_str())
                .unwrap_or("*")
                .to_string();
            if let Some((rows, age_ms)) = st.health_cache_get(&cache_key) {
                return ok(json!({
                    "project_netloc": netloc,
                    "project_source": "bitwarden",
                    "rows": rows,
                    "cache": {"hit": true, "age_ms": age_ms, "ttl_ms": HEALTH_CACHE_TTL.as_millis() as u64},
                }));
            }
            match st.postgrest("GET", &path, None, false) {
                Ok((rows, _)) => {
                    st.health_cache_put(&cache_key, rows.clone());
                    ok(json!({
                        "project_netloc": netloc,
                        "project_source": "bitwarden",
                        "rows": rows,
                        "cache": {"hit": false, "ttl_ms": HEALTH_CACHE_TTL.as_millis() as u64},
                    }))
                }
                Err(e) => fail(e.exit, e.msg),
            }
        }
        "/signal-count" => {
            let queue = req.get("queue").and_then(|q| q.as_str()).unwrap_or("");
            let path = format!(
                "tasks?select=id,created_at&queue=eq.{}&status=eq.queued&order=created_at.asc&limit=1",
                quote(queue, "")
            );
            match st.postgrest("GET", &path, None, true) {
                Ok((rows, content_range)) => {
                    let arr = rows.as_array().cloned().unwrap_or_default();
                    let mut count = arr.len() as i64;
                    if let Some(cr) = content_range {
                        if let Some(total) = cr.rsplit('/').next() {
                            if let Ok(n) = total.parse::<i64>() {
                                count = n;
                            }
                        }
                    }
                    let oldest = arr
                        .first()
                        .and_then(|r| r.get("created_at"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    ok(json!({"queued_count": count, "oldest_queued_at": oldest}))
                }
                Err(e) => fail(e.exit, e.msg),
            }
        }
        "/healthz" => ok(json!({"service": "queue-broker"})),
        _ => json!({"ok": false, "exit": 2, "error": format!("unknown command {path}")}),
    }
}

fn json_response(status: u16, body: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let data = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
}

fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    // #6320 defence in depth: an empty expected token never authenticates,
    // not even against an empty X-Agent-Token header.
    if expected.is_empty() {
        return false;
    }
    provided.map(|v| v == expected).unwrap_or(false)
}

/// Transport-agnostic request handling (#6216): the TCP loop adapts
/// tiny_http requests to this function and the UDS loop adapts
/// Unix-socket requests; routes and auth are identical on both
/// transports by construction. Returns (status, JSON body).
fn handle_request(
    st: &State,
    token: &str,
    method: &str,
    target: &str,
    provided_token: Option<&str>,
    body: &[u8],
) -> (u16, Value) {
    let path = target.split('?').next().unwrap_or("");
    if method == "GET" && path == "/healthz" {
        (200, ok(json!({"service": "queue-broker"})))
    } else if !token_matches(provided_token, token) {
        (401, json!({"error": "unauthorized"}))
    } else if method != "POST" {
        (405, json!({"error": "POST only"}))
    } else {
        match std::str::from_utf8(body) {
            Ok(buf) => {
                let parsed: Value = serde_json::from_str(buf).unwrap_or(Value::Null);
                (200, dispatch(st, path, &parsed))
            }
            Err(e) => (
                400,
                json!({"ok": false, "exit": 2, "error": format!("bad body: {e}")}),
            ),
        }
    }
}

pub fn run() -> i32 {
    let bind =
        std::env::var("QUEUE_BROKER_BIND").unwrap_or_else(|_| "100.99.113.103:4801".to_string());
    let home = crate::home::from_env().unwrap_or_else(|error| {
        eprintln!("queue-broker: {error}");
        std::process::exit(2);
    });
    let token_file = std::env::var("QUEUE_BROKER_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/queue-broker/token"));
    let token = crate::token_file::load_token_file(&token_file).unwrap_or_else(|e| {
        crate::log::info("queue-broker", &e);
        std::process::exit(3);
    });
    let secrets_url = std::env::var("QUEUE_BROKER_SECRETS_URL")
        .unwrap_or_else(|_| "http://100.99.113.103:4800".to_string());
    let secrets_token_file = std::env::var("QUEUE_BROKER_SECRETS_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/secrets-broker/token"));
    let secrets_token =
        crate::token_file::load_token_file(&secrets_token_file).unwrap_or_else(|e| {
            crate::log::info("queue-broker", &e);
            std::process::exit(3);
        });

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(90)))
        .build()
        .into();
    let failure_events_path = std::env::var("QUEUE_BROKER_FAILURE_EVENTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(format!(
                "{home}/.local/state/queue-broker/failure-events.jsonl"
            ))
        });
    if let Some(dir) = failure_events_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    // #6741 backend selection: Postgres stays the default; the
    // Turso backend is opt-in per broker process until the cutover
    // runbook flips the unit env.
    let turso = if std::env::var("QUEUE_BROKER_BACKEND")
        .map(|v| v.eq_ignore_ascii_case("turso"))
        .unwrap_or(false)
    {
        let url = std::env::var("QUEUE_BROKER_TURSO_URL")
            .unwrap_or_else(|_| crate::turso_queue::DEFAULT_TURSO_URL.to_string());
        let token = match std::env::var("QUEUE_BROKER_TURSO_TOKEN_FILE") {
            Ok(f) => crate::token_file::load_token_file(&f).unwrap_or_else(|e| {
                crate::log::info("queue-broker", &e);
                std::process::exit(3);
            }),
            Err(_) => {
                let resolver = Resolver::new(vec![Box::new(DirectSecrets {
                    url: secrets_url.clone(),
                    token: secrets_token.clone(),
                    agent: agent.clone(),
                })]);
                resolver
                    .resolve(crate::turso_queue::TURSO_TOKEN_SECRET_NAME)
                    .map(|v| v.expose().to_string())
                    .unwrap_or_else(|e| {
                        crate::log::info(
                            "queue-broker",
                            &format!("turso backend token resolve failed: {e}"),
                        );
                        std::process::exit(3);
                    })
            }
        };
        let backend = crate::turso_queue::TursoBackend::new(url, token, agent.clone());
        backend.ensure_schema().unwrap_or_else(|e| {
            crate::log::info(
                "queue-broker",
                &format!("turso backend schema ensure failed: {e}"),
            );
            std::process::exit(3);
        });
        crate::log::info(
            "queue-broker",
            &format!("queue backend: turso ({})", backend.url()),
        );
        Some(backend)
    } else {
        None
    };

    let state = Arc::new(State {
        secrets_url,
        secrets_token,
        creds: RwLock::new(None),
        agent,
        health_cache: RwLock::new(None),
        enqueue_gate: Mutex::new(EnqueueGate::new()),
        failure_events_path: Some(failure_events_path),
        turso,
    });

    let server = Server::http(&bind).unwrap_or_else(|e| {
        crate::log::info("queue-broker", &format!("cannot bind {bind}: {e:?}"));
        std::process::exit(3);
    });
    crate::log::info("queue-broker", &format!("listening on {bind}"));

    // Dual-bind (#6216): the same handler over a Unix socket. A bind
    // failure or a live owner degrades to TCP only, never to exit:
    // the TCP listener is the established surface.
    let socket_path = fleet_config::Config::load()
        .unwrap_or_default()
        .queue_broker_socket();
    match crate::uds_http::bind_socket(&socket_path) {
        Ok(Some(listener)) => {
            let st = Arc::clone(&state);
            let tok = token.clone();
            crate::log::info(
                "queue-broker",
                &format!("listening on uds {}", socket_path.display()),
            );
            std::thread::spawn(move || {
                crate::uds_http::serve(listener, move |req| {
                    let (status, value) = handle_request(
                        &st,
                        &tok,
                        &req.method,
                        &req.target,
                        req.header("X-Agent-Token"),
                        &req.body,
                    );
                    (
                        status,
                        serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
                    )
                });
            });
        }
        Ok(None) => crate::log::info(
            "queue-broker",
            &format!(
                "uds {} already owned by a live broker; TCP only",
                socket_path.display()
            ),
        ),
        Err(e) => crate::log::info(
            "queue-broker",
            &format!("cannot bind uds {}: {e}; TCP only", socket_path.display()),
        ),
    }

    for mut req in server.incoming_requests() {
        let state = Arc::clone(&state);
        let token = token.clone();
        std::thread::spawn(move || {
            let method = req.method().as_str().to_string();
            let target = req.url().to_string();
            let provided = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "x-agent-token")
                .map(|h| h.value.as_str().to_string());
            let mut body = Vec::new();
            if method == "POST" {
                let _ = req.as_reader().read_to_end(&mut body);
            }
            let (status, value) =
                handle_request(&state, &token, &method, &target, provided.as_deref(), &body);
            let _ = req.respond(json_response(status, value));
        });
    }
    // incoming_requests() ends only when the server stops.
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_limit_rejects_out_of_range_before_the_postgrest_path() {
        // #6419: the broker interpolated the limit unchecked, so a
        // client that skipped validation (or a raw caller) could send
        // 0, negatives, or an unbounded value straight to PostgREST.
        for bad in [0, -1, 501, i64::MAX] {
            let err = list_limit(&json!({"limit": bad}))
                .err()
                .expect("out-of-range limit must be rejected");
            assert!(err.contains("--limit"), "error names the flag: {err}");
        }
        assert_eq!(list_limit(&json!({"limit": 1})), Ok(1));
        assert_eq!(list_limit(&json!({"limit": 500})), Ok(500));
        // Absent limit keeps the existing default of 20.
        assert_eq!(list_limit(&json!({})), Ok(20));
    }
}
