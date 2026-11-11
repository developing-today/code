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
//!      QUEUE_BROKER_SECRETS_URL (default http://100.99.113.103:4800,
//!      or unix://<path> for the secrets broker's socket, #6218),
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
        if let Some(socket) = self.url.strip_prefix("unix://") {
            return self.resolve_uds(name, socket);
        }
        let url = format!("{}/secret/{}", self.url, name);
        let mut resp = self
            .agent
            .get(&url)
            .header("X-Agent-Token", &self.token)
            .call()
            .map_err(|e| unavailable(format!("secrets broker unreachable: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        secret_from_response(name, status, &text)
    }
}

impl DirectSecrets {
    /// The UDS leg (#6218): the same GET /secret/<name> exchange
    /// over the secrets broker's Unix socket -- its handler is
    /// transport-agnostic (uds_http), so routes and auth are
    /// identical. A connect failure falls back to the TCP endpoint
    /// within the same resolve (the #6218 stale-socket rule).
    fn resolve_uds(&self, name: &str, socket: &str) -> Result<SecretValue, SecretError> {
        use std::io::{Read, Write};
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail,
        };
        let mut stream = match std::os::unix::net::UnixStream::connect(socket) {
            Ok(stream) => stream,
            Err(_) => {
                let tcp = DirectSecrets {
                    url: "http://100.99.113.103:4800".to_string(),
                    token: self.token.clone(),
                    agent: self.agent.clone(),
                };
                return tcp.resolve(name);
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(90))).ok();
        let req = format!(
            "GET /secret/{name} HTTP/1.1\r\nHost: localhost\r\nX-Agent-Token: {}\r\nConnection: close\r\n\r\n",
            self.token
        );
        stream
            .write_all(req.as_bytes())
            .map_err(|e| unavailable(format!("secrets broker unreachable: {e}")))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| unavailable(format!("secrets broker unreachable: {e}")))?;
        let text_raw = String::from_utf8_lossy(&raw).into_owned();
        let mut parts = text_raw.splitn(2, "\r\n\r\n");
        let head = parts.next().unwrap_or("");
        let status: u16 = head
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let text = parts.next().unwrap_or("").to_string();
        secret_from_response(name, status, &text)
    }
}

/// The response tail both DirectSecrets legs share: 404 is
/// NotFound, >=300 is Unavailable, a 2xx body must carry `value`.
fn secret_from_response(name: &str, status: u16, text: &str) -> Result<SecretValue, SecretError> {
    let unavailable = |detail: String| SecretError::Unavailable {
        source: "secrets-broker",
        name: name.to_string(),
        detail,
    };
    if status == 404 {
        return Err(SecretError::NotFound {
            source: "secrets-broker",
            name: name.to_string(),
        });
    }
    if status >= 300 {
        return Err(unavailable(format!(
            "secrets broker HTTP {status} for {name}: {}",
            trim(text, 200)
        )));
    }
    let v: Value = serde_json::from_str(text)
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

/// The queue broker's secrets-broker endpoint (#6218): an explicit
/// QUEUE_BROKER_SECRETS_URL wins (http:// or unix:// form); without
/// one, fleet-config's probe-based choice for the secrets-broker
/// role decides -- a live role socket selects the UDS endpoint,
/// otherwise the historical TCP default.
fn default_secrets_url(explicit: Option<String>) -> String {
    if let Some(u) = explicit {
        if !u.trim().is_empty() {
            return u;
        }
    }
    let cfg = fleet_config::Config::load().unwrap_or_default();
    match cfg.role_transport_choice("secrets-broker") {
        Some(fleet_config::RoleTransport::Socket(socket)) => {
            format!("unix://{}", socket.display())
        }
        _ => "http://100.99.113.103:4800".to_string(),
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

/// One duration field of a request, validated before the RPC
/// (#6420): the broker is the trust boundary for non-CLI callers, so
/// an out-of-range value is rejected with the same usage error the
/// client itself raises (design #6448 Phase 3: reject, not clamp).
/// An absent or null field stays absent — the SQL defaults apply,
/// exactly as before. A field that is PRESENT but not an i64 (a
/// float, a string, an integer beyond i64) is rejected, never
/// treated as absent: before this guard it reached the SQL integer
/// parameter and failed there, and silently defaulting it would
/// mutate the task on a value the caller never chose (Codex
/// finding on PR #8425).
fn duration_field(req: &Value, field: &str, flag: &str, max: i64) -> Result<Option<i64>, String> {
    let raw = match req.get(field) {
        None => return Ok(None),
        Some(v) if v.is_null() => return Ok(None),
        Some(v) => v,
    };
    match raw.as_i64() {
        Some(v) => crate::client::validate_duration(flag, v, max).map(Some),
        None => Err(format!(
            "argument --{flag}: value {raw} out of range 1..={max}"
        )),
    }
}

/// The `/claim` lease, validated before the RPC (#6420).
fn lease_seconds(req: &Value) -> Result<Option<i64>, String> {
    duration_field(
        req,
        "lease_seconds",
        "lease-seconds",
        crate::client::LEASE_SECONDS_MAX,
    )
}

/// The `/heartbeat` extend, validated before the RPC (#6420).
fn extend_seconds(req: &Value) -> Result<Option<i64>, String> {
    duration_field(
        req,
        "extend_seconds",
        "extend-seconds",
        crate::client::EXTEND_SECONDS_MAX,
    )
}

/// The `/park` delay, validated before the RPC (#6420).
fn delay_seconds(req: &Value) -> Result<Option<i64>, String> {
    duration_field(
        req,
        "delay_seconds",
        "delay-seconds",
        crate::client::DELAY_SECONDS_MAX,
    )
}

/// The duration guard for one request path (#6420), run at the top
/// of `dispatch` BEFORE backend selection: the Turso backend
/// dispatches verbatim, so validating only inside the PostgREST
/// handlers would leave the trust boundary open on that backend
/// (Codex finding on PR #8425). The PostgREST handlers keep their
/// own checks as defense in depth.
fn validate_durations(path: &str, req: &Value) -> Result<(), String> {
    match path {
        "/claim" => lease_seconds(req).map(|_| ()),
        "/heartbeat" => extend_seconds(req).map(|_| ()),
        "/park" => delay_seconds(req).map(|_| ()),
        _ => Ok(()),
    }
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

/// The GitHub token the dep rechecker authenticates with,
/// resolved by name through the secrets broker like every other
/// broker credential (#8276).
const GH_DEP_TOKEN_SECRET: &str = "github.developing-today-code-automation";

/// #8276: the production DepProbe — GitHub state reads for the
/// dep rechecker, inside the broker process. The token resolves
/// by name through the secrets Resolver (the same one the Turso
/// token uses) and is cached for ten minutes; the value never
/// leaves this process. Every transport/auth failure maps to
/// DepObs::Failed — a broken checker never clears a gate; only
/// a block's max TTL can release it then. A 404 maps to Broken:
/// the reference does not exist, and a typo'd gate must not
/// become a life sentence.
struct GhDepProbe {
    // The secrets coordinates, not a Resolver: fleet_secrets'
    // Resolver is not Send/Sync, and the probe lives behind an
    // Arc<dyn DepProbe>. The source is rebuilt per resolve,
    // exactly like the broker builds its own per backend.
    secrets_url: String,
    secrets_token: String,
    agent: ureq::Agent,
    token: Mutex<Option<(String, std::time::Instant)>>,
}

impl GhDepProbe {
    fn token(&self) -> Result<String, String> {
        let mut slot = self.token.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((tok, at)) = &*slot {
            if at.elapsed() < std::time::Duration::from_secs(600) {
                return Ok(tok.clone());
            }
        }
        let source = DirectSecrets {
            url: self.secrets_url.clone(),
            token: self.secrets_token.clone(),
            agent: self.agent.clone(),
        };
        let v = source
            .resolve(GH_DEP_TOKEN_SECRET)
            .map_err(|e| format!("github token resolve failed: {e}"))?;
        let tok = v.expose().to_string();
        *slot = Some((tok.clone(), std::time::Instant::now()));
        Ok(tok)
    }

    fn get_json(&self, path: &str) -> Result<Value, crate::turso_queue::DepObs> {
        use crate::turso_queue::DepObs;
        let tok = self.token().map_err(DepObs::Failed)?;
        let url = format!("https://api.github.com{path}");
        let mut resp = match self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {tok}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "fleet-queue-broker")
            .call()
        {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(404)) => {
                return Err(DepObs::Broken(format!("github 404 for {path}")));
            }
            Err(ureq::Error::StatusCode(code)) => {
                return Err(DepObs::Failed(format!("github http {code} for {path}")));
            }
            Err(e) => {
                return Err(DepObs::Failed(format!("github request failed: {e}")));
            }
        };
        let status = resp.status().as_u16();
        if status == 404 {
            return Err(DepObs::Broken(format!("github 404 for {path}")));
        }
        if !(200..300).contains(&status) {
            return Err(DepObs::Failed(format!("github http {status} for {path}")));
        }
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| DepObs::Failed(format!("github response unreadable: {e}")))?;
        serde_json::from_str(&text)
            .map_err(|e| DepObs::Failed(format!("github response not json: {e}")))
    }
}

impl crate::turso_queue::DepProbe for GhDepProbe {
    fn probe_issue(&self, repo: &str, number: i64) -> crate::turso_queue::DepObs {
        use crate::turso_queue::DepObs;
        match self.get_json(&format!("/repos/{repo}/issues/{number}")) {
            Ok(v) => {
                let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("");
                if state == "closed" {
                    DepObs::Cleared("issue closed".to_string())
                } else {
                    DepObs::Open(format!("issue {state}"))
                }
            }
            Err(o) => o,
        }
    }

    fn probe_pr(&self, repo: &str, number: i64) -> crate::turso_queue::DepObs {
        use crate::turso_queue::DepObs;
        match self.get_json(&format!("/repos/{repo}/pulls/{number}")) {
            Ok(v) => {
                let merged = v.get("merged").and_then(|m| m.as_bool()).unwrap_or(false);
                let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("");
                if merged {
                    DepObs::Cleared("pr merged".to_string())
                } else if state == "closed" {
                    DepObs::Cleared("pr closed without merge".to_string())
                } else {
                    DepObs::Open(format!("pr {state}"))
                }
            }
            Err(o) => o,
        }
    }

    fn probe_ci(&self, repo: &str, run_id: i64) -> crate::turso_queue::DepObs {
        use crate::turso_queue::DepObs;
        match self.get_json(&format!("/repos/{repo}/actions/runs/{run_id}")) {
            Ok(v) => {
                let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                if status == "completed" {
                    let conclusion = v
                        .get("conclusion")
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown");
                    DepObs::Cleared(format!("run completed: {conclusion}"))
                } else {
                    DepObs::Open(format!("run {status}"))
                }
            }
            Err(o) => o,
        }
    }
}

fn dispatch(st: &State, path: &str, req: &Value) -> Value {
    if let Err(e) = validate_durations(path, req) {
        return fail(2, e);
    }
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
            let lease = match lease_seconds(req) {
                Ok(v) => v,
                Err(e) => return fail(2, e),
            };
            let mut args = json!({
                "p_worker": req.get("worker"),
                "p_queue": req.get("queue"),
                "p_lease_seconds": lease,
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
        "/heartbeat" => {
            let extend = match extend_seconds(req) {
                Ok(v) => v,
                Err(e) => return fail(2, e),
            };
            simple_rpc(
                st,
                "heartbeat_task",
                json!({
                    "p_task_id": req.get("task_id"),
                    "p_worker": req.get("worker"),
                    "p_extend_seconds": extend,
                    "p_progress": req.get("progress"),
                }),
            )
        }
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
            let delay = match delay_seconds(req) {
                Ok(v) => v,
                Err(e) => return fail(2, e),
            };
            let mut args = json!({
                "p_task_id": req.get("task_id"),
                "p_worker": req.get("worker"),
                "p_reason": req.get("reason"),
                "p_progress": req.get("progress").cloned().unwrap_or(json!({})),
            });
            if let Some(nb) = req.get("not_before").filter(|v| !v.is_null()) {
                args["p_not_before"] = nb.clone();
            } else if let Some(d) = delay {
                args["p_delay_seconds"] = json!(d);
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

/// Pre-body gate for the broker transports (#6515, PR #6514 thread
/// 4492609604). Oracle: an unauthorized or oversized request is
/// refused from its headers alone, before a single body byte is
/// read — an unauthenticated client can no longer stream a large or
/// unfinished body and hold the connection until the handler's 401.
/// Returns the refusal to send, or `None` to proceed to the body.
/// The only unauthenticated route is the health probe, mirroring
/// `handle_request`.
fn pre_body_refusal(
    method: &str,
    target: &str,
    provided_token: Option<&str>,
    token: &str,
    content_length: Option<u64>,
) -> Option<(u16, Value)> {
    let path = target.split('?').next().unwrap_or("");
    if method == "GET" && path == "/healthz" {
        return None;
    }
    if !token_matches(provided_token, token) {
        return Some((401, json!({"error": "unauthorized"})));
    }
    if let Some(n) = content_length {
        if n > crate::uds_http::MAX_BODY_BYTES as u64 {
            return Some((413, json!({"error": "request body too large"})));
        }
    }
    None
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
    use std::io::Read as _;
    let bind =
        std::env::var("QUEUE_BROKER_BIND").unwrap_or_else(|_| "100.99.113.103:4801".to_string());
    let home = match crate::home::from_env() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("queue-broker: {error}");
            return 2;
        }
    };
    let token_file = std::env::var("QUEUE_BROKER_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/queue-broker/token"));
    let token = match crate::token_file::load_token_file(&token_file) {
        Ok(v) => v,
        Err(e) => {
            crate::log::info("queue-broker", &e);
            return 3;
        }
    };
    let secrets_url = default_secrets_url(std::env::var("QUEUE_BROKER_SECRETS_URL").ok());
    let secrets_token_file = std::env::var("QUEUE_BROKER_SECRETS_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/secrets-broker/token"));
    let secrets_token = match crate::token_file::load_token_file(&secrets_token_file) {
        Ok(v) => v,
        Err(e) => {
            crate::log::info("queue-broker", &e);
            return 3;
        }
    };

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
            Ok(f) => match crate::token_file::load_token_file(&f) {
                Ok(v) => v,
                Err(e) => {
                    crate::log::info("queue-broker", &e);
                    return 3;
                }
            },
            Err(_) => {
                let resolver = Resolver::new(vec![Box::new(DirectSecrets {
                    url: secrets_url.clone(),
                    token: secrets_token.clone(),
                    agent: agent.clone(),
                })]);
                match resolve_with_startup_retry(
                    &resolver,
                    crate::turso_queue::TURSO_TOKEN_SECRET_NAME,
                    Duration::from_secs(30),
                ) {
                    Ok(v) => v.expose().to_string(),
                    Err(e) => {
                        crate::log::info(
                            "queue-broker",
                            &format!(
                                "turso backend token resolve failed after 30s of retries: {e}"
                            ),
                        );
                        return 3;
                    }
                }
            }
        };
        let mut backend = crate::turso_queue::TursoBackend::new(url, token, agent.clone());
        // #8276: the dep rechecker's GitHub window, authenticated
        // by name through the same secrets Resolver as the Turso
        // token above.
        backend.set_gh_probe(Arc::new(GhDepProbe {
            secrets_url: secrets_url.clone(),
            secrets_token: secrets_token.clone(),
            agent: agent.clone(),
            token: Mutex::new(None),
        }));
        if let Err(e) = backend.ensure_schema() {
            crate::log::info(
                "queue-broker",
                &format!("turso backend schema ensure failed: {e}"),
            );
            return 3;
        }
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

    let server = match Server::http(&bind) {
        Ok(server) => server,
        Err(e) => {
            crate::log::info("queue-broker", &format!("cannot bind {bind}: {e:?}"));
            return 3;
        }
    };
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
                let pre_tok = tok.clone();
                crate::uds_http::serve_with_precheck(
                    listener,
                    move |head| {
                        // #6515: same pre-body gate as the TCP loop.
                        let content_length = head
                            .header("Content-Length")
                            .and_then(|v| v.parse::<u64>().ok());
                        pre_body_refusal(
                            &head.method,
                            &head.target,
                            head.header("X-Agent-Token"),
                            &pre_tok,
                            content_length,
                        )
                        .map(|(status, value)| {
                            (
                                status,
                                serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
                            )
                        })
                    },
                    move |req| {
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
                    },
                );
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
            // #6515: authenticate and size-check from the headers
            // before reading any body byte. tiny_http 0.12 exposes
            // no per-connection read timeout, so the residual TCP
            // slow-body risk is bounded by this gate plus the capped
            // read below and thread-per-connection isolation; the
            // UDS transport carries real read timeouts (uds_http).
            let content_length = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "content-length")
                .and_then(|h| h.value.as_str().parse::<u64>().ok());
            if let Some((status, value)) = pre_body_refusal(
                &method,
                &target,
                provided.as_deref(),
                &token,
                content_length,
            ) {
                let _ = req.respond(json_response(status, value));
                return;
            }
            let mut body = Vec::new();
            if method == "POST" {
                // Capped read: cap+1 bytes detect an overrun that
                // the declared length hid (or never declared).
                let cap = (crate::uds_http::MAX_BODY_BYTES + 1) as u64;
                let _ = req.as_reader().take(cap).read_to_end(&mut body);
                if body.len() > crate::uds_http::MAX_BODY_BYTES {
                    let _ = req.respond(json_response(
                        413,
                        json!({"error": "request body too large"}),
                    ));
                    return;
                }
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
    fn pre_body_refusal_decides_from_headers_alone() {
        // #6515 oracle (thread 4492609604): the gate's answer
        // depends only on method/target/token/declared length —
        // never on body bytes — so an unauthorized or oversized
        // request is refused before its body is read.
        let over = crate::uds_http::MAX_BODY_BYTES as u64 + 1;
        // Unauthorized: refused even with an enormous declared body.
        assert_eq!(
            pre_body_refusal("POST", "/claim", None, "tok", Some(over)).map(|r| r.0),
            Some(401)
        );
        assert_eq!(
            pre_body_refusal("POST", "/claim", Some("wrong"), "tok", None).map(|r| r.0),
            Some(401)
        );
        // Authorized but oversized: 413.
        assert_eq!(
            pre_body_refusal("POST", "/claim", Some("tok"), "tok", Some(over)).map(|r| r.0),
            Some(413)
        );
        // Authorized and within cap (or undeclared): proceeds.
        assert_eq!(
            pre_body_refusal("POST", "/claim", Some("tok"), "tok", Some(2)),
            None
        );
        assert_eq!(
            pre_body_refusal("POST", "/claim", Some("tok"), "tok", None),
            None
        );
        // The health probe stays unauthenticated, as in handle_request.
        assert_eq!(pre_body_refusal("GET", "/healthz", None, "tok", None), None);
    }

    #[test]
    fn secrets_url_explicit_wins_and_default_is_a_known_endpoint() {
        // #6218: QUEUE_BROKER_SECRETS_URL beats probe selection in
        // both forms; empty counts as unset; the default is either
        // the probed socket endpoint or the historical TCP one.
        assert_eq!(
            default_secrets_url(Some("http://127.0.0.1:9".to_string())),
            "http://127.0.0.1:9"
        );
        assert_eq!(
            default_secrets_url(Some("unix:///tmp/s.sock".to_string())),
            "unix:///tmp/s.sock"
        );
        let default = default_secrets_url(Some("  ".to_string()));
        assert!(
            default == "http://100.99.113.103:4800" || default.starts_with("unix://"),
            "unexpected default endpoint: {default}"
        );
        let default_none = default_secrets_url(None);
        assert_eq!(default_none, default);
    }

    #[test]
    fn secret_from_response_maps_statuses() {
        // #6218: the shared tail of both DirectSecrets legs.
        let ok = secret_from_response("n", 200, "{\"value\":\"v1\"}").expect("200 resolves");
        assert_eq!(ok.expose(), "v1");
        assert!(matches!(
            secret_from_response("n", 404, ""),
            Err(SecretError::NotFound { .. })
        ));
        assert!(matches!(
            secret_from_response("n", 500, "boom"),
            Err(SecretError::Unavailable { .. })
        ));
        assert!(matches!(
            secret_from_response("n", 200, "{}"),
            Err(SecretError::Unavailable { .. })
        ));
    }

    #[test]
    fn direct_secrets_resolves_over_a_unix_socket() {
        // #6218: the UDS leg speaks the same GET /secret/<name>
        // exchange the TCP leg makes with ureq.
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join(format!("fleet-queue-uds-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let socket = dir.join("secrets.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind socket");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let body = "{\"value\":\"uds-secret\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).expect("write reply");
        });
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let source = DirectSecrets {
            url: format!("unix://{}", socket.display()),
            token: "test-token".to_string(),
            agent,
        };
        let value = source.resolve("some-name").expect("UDS resolve succeeds");
        assert_eq!(value.expose(), "uds-secret");
        server.join().expect("server thread");
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    #[test]
    fn duration_fields_reject_out_of_range_before_the_rpc() {
        // #6420: a raw caller could send a born-expired lease, a
        // self-lapsing heartbeat extend, or a park delay in the past
        // straight to the RPC. The broker rejects all three with the
        // client's usage error before any RPC is built.
        for bad in [0, -1, 86_401, i64::MAX] {
            let err = lease_seconds(&json!({"lease_seconds": bad}))
                .err()
                .expect("out-of-range lease_seconds must be rejected");
            assert!(
                err.contains("--lease-seconds"),
                "error names the flag: {err}"
            );
            let err = extend_seconds(&json!({"extend_seconds": bad}))
                .err()
                .expect("out-of-range extend_seconds must be rejected");
            assert!(
                err.contains("--extend-seconds"),
                "error names the flag: {err}"
            );
        }
        for bad in [0, -1, 604_801, i64::MAX] {
            let err = delay_seconds(&json!({"delay_seconds": bad}))
                .err()
                .expect("out-of-range delay_seconds must be rejected");
            assert!(
                err.contains("--delay-seconds"),
                "error names the flag: {err}"
            );
        }
        assert_eq!(
            lease_seconds(&json!({"lease_seconds": 2700})),
            Ok(Some(2700))
        );
        assert_eq!(
            extend_seconds(&json!({"extend_seconds": 900})),
            Ok(Some(900))
        );
        assert_eq!(
            delay_seconds(&json!({"delay_seconds": 1200})),
            Ok(Some(1200))
        );
        // Absent fields stay absent (SQL defaults apply, as before).
        assert_eq!(lease_seconds(&json!({})), Ok(None));
        assert_eq!(extend_seconds(&json!({})), Ok(None));
        assert_eq!(delay_seconds(&json!({})), Ok(None));
        // Present but not an i64 (beyond-i64 integer, float, string)
        // is rejected, never silently defaulted (Codex finding on
        // PR #8425): before this guard such a value failed at the
        // SQL integer parameter instead.
        for bad in [json!(9223372036854775808u64), json!(1.5), json!("2700")] {
            let err = lease_seconds(&json!({"lease_seconds": bad}))
                .err()
                .expect("present-but-invalid lease_seconds must be rejected");
            assert!(
                err.contains("--lease-seconds"),
                "error names the flag: {err}"
            );
            let err = extend_seconds(&json!({"extend_seconds": bad}))
                .err()
                .expect("present-but-invalid extend_seconds must be rejected");
            assert!(
                err.contains("--extend-seconds"),
                "error names the flag: {err}"
            );
            let err = delay_seconds(&json!({"delay_seconds": bad}))
                .err()
                .expect("present-but-invalid delay_seconds must be rejected");
            assert!(
                err.contains("--delay-seconds"),
                "error names the flag: {err}"
            );
        }
        // The dispatch-level guard covers both backends: it fires
        // per path before backend selection (Codex finding on
        // PR #8425), and leaves unrelated paths alone.
        assert!(validate_durations("/claim", &json!({"lease_seconds": 0})).is_err());
        assert!(validate_durations("/heartbeat", &json!({"extend_seconds": -1})).is_err());
        assert!(validate_durations("/park", &json!({"delay_seconds": 604_801})).is_err());
        assert_eq!(
            validate_durations("/claim", &json!({"lease_seconds": 2700})),
            Ok(())
        );
        assert_eq!(validate_durations("/heartbeat", &json!({})), Ok(()));
        assert_eq!(
            validate_durations("/list", &json!({"lease_seconds": 0})),
            Ok(())
        );
    }
}

/// Resolve `name` through `resolver`, retrying until `budget`
/// elapses. Startup only: a co-hosted or freshly started
/// secrets-broker binds before its first vault refresh completes
/// and answers 503 while its cache is cold, so a one-shot resolve
/// races its readiness and the whole serve list exits (#8394
/// Codex P2). Bounded: once the budget is spent the last error is
/// returned and the caller fails exactly as the one-shot did.
fn resolve_with_startup_retry(
    resolver: &Resolver,
    name: &str,
    budget: Duration,
) -> Result<SecretValue, SecretError> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        match resolver.resolve(name) {
            Ok(v) => return Ok(v),
            Err(e) => {
                if std::time::Instant::now() >= deadline {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

#[cfg(test)]
mod startup_retry_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A source that answers Unavailable for the first `fail_first`
    /// calls, then resolves — the cold secrets-broker shape.
    struct FlakySource {
        calls: AtomicUsize,
        fail_first: usize,
    }

    impl SecretSource for FlakySource {
        fn source_name(&self) -> &'static str {
            "flaky-test"
        }
        fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) < self.fail_first {
                return Err(SecretError::Unavailable {
                    source: "flaky-test",
                    name: name.to_string(),
                    detail: "cold cache".to_string(),
                });
            }
            Ok(SecretValue::from_resolved("tok".to_string()))
        }
    }

    #[test]
    fn startup_retry_outlasts_a_cold_source() {
        let resolver = Resolver::new(vec![Box::new(FlakySource {
            calls: AtomicUsize::new(0),
            fail_first: 3,
        })]);
        let v = resolve_with_startup_retry(&resolver, "some.token", Duration::from_secs(5))
            .expect("retry should reach the warmed source");
        assert_eq!(v.expose(), "tok");
    }

    #[test]
    fn startup_retry_gives_up_within_budget() {
        let resolver = Resolver::new(vec![Box::new(FlakySource {
            calls: AtomicUsize::new(0),
            fail_first: usize::MAX,
        })]);
        let start = std::time::Instant::now();
        let r = resolve_with_startup_retry(&resolver, "some.token", Duration::from_millis(300));
        assert!(r.is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
