//! repo-queue client — the queue domain's client surface
//! (fleet-queue, step 2 of epic developing-today/code#5412, #5430;
//! ported from the standalone repo-queue crate, code#5362).
//!
//! Same CLI surface, exit codes, and stdout formats as the retired
//! repo-queue.py. Two transports reach the same queue-broker: from
//! the sandbox, the session proxy (the laptop-agent broker's Unix
//! socket); on the laptop itself, a direct HTTP connection selected
//! by REPO_QUEUE_BROKER_URL or ~/.config/repo-queue/broker-url (the
//! opencode claimant and the worker sessions it launches use this).
//! Without a configured URL, the direct path is probe-selected
//! (#6218): fleet-config's transport choice for the queue-broker
//! role picks the broker's Unix socket when it answers; an absent
//! or stale socket keeps the session-proxy route, and a failed
//! UDS attempt falls back to it within the same call.
//! No Bitwarden fetch, no direct Supabase access. Exit 3 means the
//! broker path itself is unavailable.
//!
//! Entry points: the fleet binary's queue domain and the standalone
//! `repo-queue` artifact (src/bin/repo-queue.rs) both call `run`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

const QUEUE_BROKER_PORT: u64 = 4801;
const DEFAULT_QUEUE: &str = "repo-health-code-shadow";
const DEFAULT_WORKER: &str = "repo-worker-code";
const TELEMETRY_FIELDS: [&str; 4] = ["pool", "model", "effort", "cost_usd"];
const TERMINAL_STATUSES: [&str; 3] = ["done", "cancelled", "failed"];

fn home() -> String {
    crate::home::from_env().unwrap_or_else(|error| die(3, error))
}

fn eprint(s: &str) {
    eprintln!("{s}");
}

fn die(code: i32, msg: &str) -> ! {
    eprintln!("repo-queue: {msg}");
    exit(code);
}

fn print_json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".to_string())
    );
}

// ---------------------------------------------------------------------------
// Session-proxy transport
// ---------------------------------------------------------------------------

fn broker_sock() -> String {
    // #6218: ask fleet-config instead of hardcoding the path --
    // env beats the config file beats the compiled-in default,
    // which is the same path broker_sock_from_home builds. A
    // config load error falls back to the home-anchored default.
    match fleet_config::Config::load() {
        Ok(cfg) => cfg.broker_socket().to_string_lossy().into_owned(),
        Err(_) => broker_sock_from_home(Some(&home())).unwrap_or_else(|error| die(3, error)),
    }
}

fn broker_sock_from_home(home_dir: Option<&str>) -> Result<String, &'static str> {
    let home = crate::home::resolve(home_dir)?;
    Ok(format!("{home}/.cache/laptop-agent/broker.sock"))
}

// ---------------------------------------------------------------------------
// Direct-broker transport (on-laptop use: the opencode claimant and
// the worker sessions it launches run on the same host as the
// queue-broker, where no session proxy exists). Selected by
// REPO_QUEUE_BROKER_URL or ~/.config/repo-queue/broker-url; the token
// comes from REPO_QUEUE_TOKEN_FILE or ~/.config/queue-broker/token.
// With neither configured, the session-proxy transport below is used,
// exactly as before.
// ---------------------------------------------------------------------------

fn direct_broker_url() -> Option<String> {
    if let Ok(u) = std::env::var("REPO_QUEUE_BROKER_URL") {
        let u = u.trim().to_string();
        if !u.is_empty() {
            return Some(u);
        }
    }
    let p = format!("{}/.config/repo-queue/broker-url", home());
    if let Ok(s) = std::fs::read_to_string(&p) {
        let u = s.trim().to_string();
        if !u.is_empty() {
            return Some(u);
        }
    }
    None
}

/// The direct route to the queue-broker (#6218): its TCP
/// listener, or its Unix socket (the broker dual-binds both,
/// #6216). Selected by [`direct_route`].
#[derive(Debug, PartialEq, Eq)]
enum DirectRoute {
    Tcp(String),
    Unix(std::path::PathBuf),
}

/// Parse a configured broker URL into a route: `unix://<path>`
/// selects the socket; anything else keeps the historical
/// `http://host:port` TCP reading (validated in the TCP leg).
fn route_from_configured_url(url: &str) -> Result<DirectRoute, String> {
    if let Some(p) = url.strip_prefix("unix://") {
        if p.is_empty() {
            return Err(format!("bad broker URL {url:?}"));
        }
        return Ok(DirectRoute::Unix(std::path::PathBuf::from(p)));
    }
    Ok(DirectRoute::Tcp(url.to_string()))
}

/// Which direct route, if any (#6218): an explicitly configured
/// URL (env or broker-url file) wins; otherwise fleet-config's
/// probe-based choice for the queue-broker role -- a live role
/// socket selects UDS, the TCP choice keeps today's behavior
/// (None: the session-proxy route below IS the sandbox's TCP
/// route, and sandbox consumers are untouched by design).
fn direct_route() -> Option<DirectRoute> {
    if let Some(url) = direct_broker_url() {
        return match route_from_configured_url(&url) {
            Ok(route) => Some(route),
            Err(e) => die(3, &e),
        };
    }
    let cfg = fleet_config::Config::load().unwrap_or_default();
    match cfg.role_transport_choice("queue-broker") {
        Some(fleet_config::RoleTransport::Socket(socket)) => Some(DirectRoute::Unix(socket)),
        _ => None,
    }
}

fn direct_token() -> Result<String, String> {
    let token_file_env = std::env::var("REPO_QUEUE_TOKEN_FILE").ok();
    let home_env = std::env::var("HOME").ok();
    let token_file = token_file_from_env(token_file_env.as_deref(), home_env.as_deref())
        .unwrap_or_else(|error| die(3, error));
    let token = crate::token_file::load_token_file(&token_file)?;
    Ok(token.trim().to_string())
}

/// The response handling both direct legs share: status line,
/// JSON body, 401 and >=400 die with exit 3 exactly as before.
fn finish_direct_response(raw: &[u8]) -> Result<Value, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("queue-broker bad reply")?;
    let head_txt = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u64 = head_txt
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let resp_body: Value = serde_json::from_slice(&raw[split + 4..]).unwrap_or(Value::Null);
    if status == 401 {
        die(3, "queue-broker rejected the client token (HTTP 401)");
    }
    if status >= 400 {
        let msg = resp_body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("queue-broker error")
            .to_string();
        die(3, &msg);
    }
    Ok(resp_body)
}

fn direct_tcp_roundtrip(url: &str, path: &str, body: &Value) -> Result<Value, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("broker URL must be http://host:port")?;
    let hostport = rest.trim_end_matches('/');
    if hostport.is_empty() || hostport.contains('/') {
        return Err(format!("bad broker URL {url:?}"));
    }
    let token = direct_token()?;
    let payload = serde_json::to_vec(body).unwrap();
    let mut s = TcpStream::connect(hostport)
        .map_err(|e| format!("cannot connect to queue-broker at {hostport}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(300))).ok();
    s.set_write_timeout(Some(Duration::from_secs(60))).ok();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nX-Agent-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("queue-broker write failed: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("queue-broker read failed: {e}"))?;
    finish_direct_response(&raw)
}

/// The UDS leg of the direct path (#6218): the same HTTP/1.1
/// exchange over the queue-broker's Unix socket. The broker's
/// handler is transport-agnostic (uds_http), so the wire surface
/// -- routes, token auth, JSON envelope -- is identical.
fn direct_uds_roundtrip(
    socket: &std::path::Path,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let token = direct_token()?;
    let payload = serde_json::to_vec(body).unwrap();
    let mut s = UnixStream::connect(socket).map_err(|e| {
        format!(
            "cannot connect to queue-broker at unix://{}: {e}",
            socket.display()
        )
    })?;
    s.set_read_timeout(Some(Duration::from_secs(300))).ok();
    s.set_write_timeout(Some(Duration::from_secs(60))).ok();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Agent-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("queue-broker write failed: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("queue-broker read failed: {e}"))?;
    finish_direct_response(&raw)
}

fn broker_roundtrip(path: &str, body: &Value) -> Result<Value, String> {
    match direct_route() {
        Some(DirectRoute::Tcp(url)) => return direct_tcp_roundtrip(&url, path, body),
        Some(DirectRoute::Unix(socket)) => match direct_uds_roundtrip(&socket, path, body) {
            Ok(v) => return Ok(v),
            // #6218 acceptance: a failed socket attempt falls back
            // to TCP within the same connection attempt -- here the
            // session-proxy route below, the sandbox's TCP route.
            Err(_) => {}
        },
        None => {}
    }
    let payload = json!({
        "method": "POST",
        "path": path,
        "body": body,
        "port": QUEUE_BROKER_PORT,
        "token_file": format!("{}/tools/queue-broker-token", home()),
    });
    let data = serde_json::to_vec(&payload).unwrap();
    for attempt in 0..2 {
        match unix_roundtrip(&data) {
            Ok(resp) => {
                let status = resp.get("status").and_then(|s| s.as_u64()).unwrap_or(0);
                let body = resp.get("body").cloned().unwrap_or(Value::Null);
                if status == 401 {
                    die(3, "queue-broker rejected the client token (HTTP 401)");
                }
                if status >= 400 {
                    let msg = body
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("session proxy error")
                        .to_string();
                    die(3, &msg);
                }
                return Ok(body);
            }
            Err(e) => {
                if attempt == 0 {
                    spawn_broker();
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err("unreachable".to_string())
}

fn unix_roundtrip(data: &[u8]) -> Result<Value, String> {
    let mut s = UnixStream::connect(broker_sock())
        .map_err(|e| format!("cannot connect to session proxy: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(660))).ok();
    s.set_write_timeout(Some(Duration::from_secs(60))).ok();
    s.write_all(&(data.len() as u64).to_be_bytes())
        .and_then(|_| s.write_all(data))
        .map_err(|e| format!("session proxy write failed: {e}"))?;
    let mut hdr = [0u8; 8];
    s.read_exact(&mut hdr)
        .map_err(|e| format!("session proxy read failed: {e}"))?;
    let n = u64::from_be_bytes(hdr) as usize;
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)
        .map_err(|e| format!("session proxy read failed: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("session proxy bad reply: {e}"))
}

/// Which broker program to auto-start: a sibling `fleet` binary
/// next to the running one wins (step 3, #5431 — `fleet proxy
/// serve` is the Python broker's successor); the standalone
/// broker builds from the fleet-brokers rollback package have no
/// fleet beside them and keep the Python spawn below.
fn broker_program() -> (String, Vec<String>) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return broker_program_in(dir);
        }
    }
    python_broker_program()
}

fn python_broker_program() -> (String, Vec<String>) {
    (
        "python3".to_string(),
        vec![format!("{}/tools/laptop-agent-broker.py", home())],
    )
}

/// The selection rule, pure over the directory beside the running
/// binary so it is testable without touching the real install
/// (#5758): a sibling `fleet` binary ALWAYS wins — no auto-start
/// path may spawn the Python broker while a fleet binary exists;
/// Python is the fallback for the standalone rollback builds that
/// ship without one.
fn broker_program_in(dir: &std::path::Path) -> (String, Vec<String>) {
    let fleet = dir.join("fleet");
    if fleet.exists() {
        return (
            fleet.to_string_lossy().into_owned(),
            vec!["proxy".to_string(), "serve".to_string()],
        );
    }
    python_broker_program()
}

fn spawn_broker() {
    let dir = format!("{}/.cache/laptop-agent", home());
    let _ = std::fs::create_dir_all(&dir);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{dir}/broker.log"));
    let (out, err) = match log {
        Ok(f) => (
            std::process::Stdio::from(f.try_clone().unwrap()),
            std::process::Stdio::from(f),
        ),
        Err(_) => (std::process::Stdio::null(), std::process::Stdio::null()),
    };
    let (program, prefix_args) = broker_program();
    let _ = std::process::Command::new(program)
        .args(prefix_args)
        .stdout(out)
        .stderr(err)
        .stdin(std::process::Stdio::null())
        .spawn();
    for _ in 0..50 {
        if std::path::Path::new(&broker_sock()).exists() {
            // Give the listener a moment to accept.
            std::thread::sleep(Duration::from_millis(100));
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// POST one queue-broker command; dies (exit 3) when the broker path
/// is unavailable, otherwise returns the response envelope (which may
/// itself be {ok:false, exit, error} — the caller decides).
fn call_result(path: &str, body: &Value) -> Result<Value, (i32, String)> {
    // #6450 posture (crate::backoff — the one shared policy): a
    // broker envelope carrying a 57014 / PGRST002-class error is
    // retried with exponential backoff + jitter instead of dying on
    // the caller. Safe for writes: a 57014 means Postgres cancelled
    // the statement, so the write rolled back and cannot
    // double-apply (enqueue dedupe keys are a second belt).
    let mut attempt: u32 = 0;
    loop {
        match broker_roundtrip(path, body) {
            Ok(v) => {
                let retryable = v.get("ok").and_then(|o| o.as_bool()) == Some(false)
                    && v.get("error")
                        .and_then(|e| e.as_str())
                        .map(crate::backoff::is_retryable)
                        .unwrap_or(false);
                if retryable && attempt + 1 < crate::backoff::MAX_ATTEMPTS {
                    crate::backoff::sleep_after_failure(attempt);
                    attempt += 1;
                    continue;
                }
                return Ok(v);
            }
            Err(e) => return Err((3, e)),
        }
    }
}

fn call(path: &str, body: &Value) -> Value {
    match call_result(path, body) {
        Ok(v) => v,
        Err((code, msg)) => die(code, &msg),
    }
}

// ---------------------------------------------------------------------------
// Permanent fallback arming (#6741; Drew's master plan, 2026-11-06:
// "encounter any issue -> fallback"). The GitHub fallback is not an
// outage-only mode any more: for the work verbs (claim, heartbeat,
// checkpoint, complete, health), ANY primary-store failure — broker
// unreachable, or an error envelope from the broker — switches the
// caller to the GitHub fallback for that work instead of dying.
// The switch is machine-readable: one JSON object on stdout with
// "fallback": true, and exit code FALLBACK_SWITCH_EXIT, so a worker
// loop can branch on it without parsing prose. Set
// REPO_QUEUE_NO_FALLBACK_SWITCH=1 to restore the plain die-on-error
// behavior (operator escape hatch).
// ---------------------------------------------------------------------------

const FALLBACK_SWITCH_EXIT: i32 = 6;

fn fallback_switch_enabled() -> bool {
    std::env::var("REPO_QUEUE_NO_FALLBACK_SWITCH")
        .map(|v| v != "1")
        .unwrap_or(true)
}

fn fallback_switch_json(verb: &str, code: i32, msg: &str) -> Value {
    json!({
        "ok": false,
        "fallback": true,
        "switch_to": "github-fallback",
        "verb": verb,
        "broker_exit": code,
        "error": msg,
        "hint": "continue this work with: fleet queue fallback scan|claim|heartbeat|release",
    })
}

fn fallback_switch(verb: &str, code: i32, msg: &str) -> ! {
    eprint(&format!(
        "repo-queue: primary queue store failed on {verb} (exit {code}): {msg};          switching this work to the GitHub fallback"
    ));
    print_json(&fallback_switch_json(verb, code, msg));
    exit(FALLBACK_SWITCH_EXIT);
}

fn call_fb(path: &str, body: &Value) -> Value {
    match call_result(path, body) {
        Ok(v) => v,
        Err((code, msg)) => {
            if fallback_switch_enabled() {
                fallback_switch(path, code, &msg)
            } else {
                die(code, &msg)
            }
        }
    }
}

fn unwrap_ok_fb(resp: Value, verb: &str) -> Value {
    if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
        let code = resp.get("exit").and_then(|e| e.as_i64()).unwrap_or(4) as i32;
        let msg = resp
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("queue-broker error")
            .to_string();
        if fallback_switch_enabled() {
            fallback_switch(verb, code, &msg);
        }
        die(code, &msg);
    }
    resp
}

/// Unwrap an envelope: on {ok:false} print the error and exit with its
/// code; otherwise return the envelope for field extraction.
fn unwrap_ok(resp: Value) -> Value {
    if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
        let code = resp.get("exit").and_then(|e| e.as_i64()).unwrap_or(4) as i32;
        let msg = resp
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("queue-broker error")
            .to_string();
        die(code, &msg);
    }
    resp
}

fn require_row(resp: &Value, fn_name: &str) -> Value {
    let row = resp.get("row").cloned().unwrap_or(Value::Null);
    if row.is_null() {
        die(
            5,
            &format!(
                "{fn_name}: no matching claimed row (task not claimed by this worker, or already terminal)"
            ),
        );
    }
    row
}

fn row_or_ledger(resp: &Value, fn_name: &str) {
    let row = require_row(resp, fn_name);
    // Broker envelope extras a caller must not lose: the triage
    // completion follow-on (#7016) rides alongside the row.
    let follow_on = resp.get("follow_on_task_id");
    match (resp.get("lane_ledger"), follow_on) {
        (Some(ledger), None) => {
            print_json(&json!({"task": row, "lane_ledger": ledger}));
        }
        (Some(ledger), Some(fid)) => {
            print_json(&json!({
                "task": row, "lane_ledger": ledger, "follow_on_task_id": fid
            }));
        }
        (None, Some(fid)) => {
            print_json(&json!({"task": row, "follow_on_task_id": fid}));
        }
        (None, None) => {
            print_json(&row);
        }
    }
}

// ---------------------------------------------------------------------------
// Time helpers (ISO 8601 <-> epoch, civil-date math)
// ---------------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse an ISO 8601 timestamp to epoch microseconds (None if unusable).
fn parse_ts(v: &Value) -> Option<i64> {
    let s = v.as_str()?;
    parse_ts_str(s)
}

fn parse_ts_str(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    let (time, offset_min) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0i64)
    } else if let Some(idx) = rest.rfind(['+', '-']) {
        if idx == 0 {
            return None;
        }
        let (t, off) = rest.split_at(idx);
        let sign = if rest.as_bytes()[idx] == b'-' { -1 } else { 1 };
        let mut op = off[1..].split(':');
        let oh: i64 = op.next()?.parse().ok()?;
        let om: i64 = op.next().unwrap_or("0").parse().ok()?;
        (t, sign * (oh * 60 + om))
    } else {
        (rest, 0)
    };
    let mut tp = time.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next()?.parse().ok()?;
    let sec_part = tp.next().unwrap_or("0");
    let (sec_s, frac) = match sec_part.split_once('.') {
        Some((a, b)) => (a, b),
        None => (sec_part, ""),
    };
    let sec: i64 = sec_s.parse().ok()?;
    let mut micros: i64 = 0;
    let mut scale = 100_000i64;
    for c in frac.chars().take(6) {
        micros += c.to_digit(10)? as i64 * scale;
        scale /= 10;
    }
    let days = days_from_civil(y, mo, d);
    let epoch_s = days * 86400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Some(epoch_s * 1_000_000 + micros)
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

fn utcnow_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn age_str(created: &Value) -> String {
    let Some(c) = parse_ts(created) else {
        return String::new();
    };
    let secs = ((now_micros() - c) / 1_000_000).max(0);
    let (d, rem) = (secs / 86400, secs % 86400);
    let (h, rem) = (rem / 3600, rem % 3600);
    let (m, s) = (rem / 60, rem % 60);
    if d > 0 {
        format!("{d}d {h:02}:{m:02}")
    } else {
        format!("{h}:{m:02}:{s:02}")
    }
}

// ---------------------------------------------------------------------------
// Formatting (mirrors repo-queue.py tables)
// ---------------------------------------------------------------------------

fn fmt_cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::Object(_)) | Some(Value::Array(_)) => {
            serde_json::to_string(v.unwrap()).unwrap_or_default()
        }
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n - 1).collect();
        format!("{head}…")
    }
}

fn print_table(cols: &[String], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = cols.iter().map(|c| c.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let fmt_row = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let pad = widths[i] - c.chars().count();
                format!("{}{}", c, " ".repeat(pad))
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    println!("{}", fmt_row(cols));
    let dashes: Vec<String> = cols.iter().map(|c| "-".repeat(c.chars().count())).collect();
    println!("{}", fmt_row(&dashes));
    for r in rows {
        println!("{}", fmt_row(r));
    }
}

// ---------------------------------------------------------------------------
// Argument parsing (argparse-shaped subset the fleet uses)
// ---------------------------------------------------------------------------

struct Args {
    pos: Vec<String>,
    opts: Map<String, Value>,
}

fn parse_args(rest: &[String], flags: &[&str], bools: &[&str]) -> Args {
    match parse_args_inner(rest, flags, bools) {
        Ok(args) => args,
        Err(msg) => die(2, &msg),
    }
}

/// Pure core of `parse_args`, returning the usage error instead of
/// exiting so the rejection shapes are unit-testable (#5939).
///
/// Duplicate and contradictory flags are usage errors, never silent
/// last-wins: repeating a value flag used to retarget an invocation
/// (e.g. `--queue a --queue b` recorded queue=b), and a boolean given
/// an inline value (`--dry-run=false`) used to be read as `true` —
/// a real enqueue silently became a dry run that enqueued nothing
/// and exited 0.
fn parse_args_inner(rest: &[String], flags: &[&str], bools: &[&str]) -> Result<Args, String> {
    let mut pos = Vec::new();
    let mut opts = Map::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if let Some(long) = a.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (long.to_string(), None),
            };
            if opts.contains_key(&name) {
                return Err(format!("argument --{name}: duplicate flag"));
            }
            if bools.contains(&name.as_str()) {
                if let Some(v) = inline {
                    return Err(format!(
                        "argument --{name}: boolean flag takes no value (got ={v})"
                    ));
                }
                opts.insert(name, Value::Bool(true));
                i += 1;
            } else if flags.contains(&name.as_str()) {
                if let Some(v) = inline {
                    opts.insert(name, Value::String(v));
                    i += 1;
                } else if i + 1 < rest.len() {
                    opts.insert(name, Value::String(rest[i + 1].clone()));
                    i += 2;
                } else {
                    return Err(format!("argument --{name}: expected one argument"));
                }
            } else {
                return Err(format!("unrecognized argument: --{name}"));
            }
        } else {
            pos.push(a.clone());
            i += 1;
        }
    }
    Ok(Args { pos, opts })
}

impl Args {
    fn get(&self, name: &str) -> Option<String> {
        self.opts
            .get(name)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
    fn has(&self, name: &str) -> bool {
        self.opts.contains_key(name)
    }
    fn get_i64(&self, name: &str) -> Option<i64> {
        match self.get(name) {
            None => None,
            Some(s) => match parse_i64_flag(name, &s) {
                Ok(n) => Some(n),
                Err(msg) => die(2, &msg),
            },
        }
    }
    fn pos_i64(&self, idx: usize, what: &str) -> i64 {
        match self.pos.get(idx).and_then(|s| s.parse::<i64>().ok()) {
            Some(n) => n,
            None => die(2, &format!("argument {what}: invalid int value")),
        }
    }
}

/// Die with a usage error when `args` carries more than `expected`
/// positionals for `verb` (#5592). `parse_args` collects positionals
/// without arity checking, and verbs read only the ones they expect —
/// so an extra positional (a second packet path, a stray word after a
/// task id, junk after `health`) was silently dropped while the verb
/// did its real work and exited 0. Every verb rejects extras through
/// this check before any broker call; unknown `--flags` were already
/// rejected in `parse_args`, this gives positionals the same treatment.
fn reject_extra_pos(verb: &str, args: &Args, expected: usize) {
    if let Some(extra) = extra_positional(&args.pos, expected) {
        die(2, &format!("{verb}: unexpected extra argument {extra:?}"));
    }
}

/// The first positional past `expected`, if any.
fn extra_positional(pos: &[String], expected: usize) -> Option<&str> {
    pos.get(expected).map(|s| s.as_str())
}

/// Parse a numeric flag's value (#5939): a non-numeric value is a
/// usage error naming the flag and the value — the same treatment
/// `pos_i64` gives positionals — never a silent default (a mistyped
/// `--limit abc` used to fall back to 20 without a word).
fn parse_i64_flag(name: &str, raw: &str) -> Result<i64, String> {
    raw.parse::<i64>()
        .map_err(|_| format!("argument --{name}: invalid int value {raw:?}"))
}

/// The accepted range for `list --limit` (#6419; design #6448 Phase 3
/// pin: out-of-range is REJECTED, never clamped, at both the client
/// and the broker). `limit=0` used to silently report no tasks and an
/// unbounded value reached the PostgREST query verbatim.
pub(crate) const LIST_LIMIT_MAX: i64 = 500;
pub(crate) const LIST_LIMIT_DEFAULT: i64 = 20;

/// Validate a `list --limit` value against `1..=LIST_LIMIT_MAX`; the
/// error names `--limit` so a usage failure points at the flag.
pub(crate) fn validate_list_limit(limit: i64) -> Result<i64, String> {
    if (1..=LIST_LIMIT_MAX).contains(&limit) {
        Ok(limit)
    } else {
        Err(format!(
            "argument --limit: value {limit} out of range 1..={LIST_LIMIT_MAX}"
        ))
    }
}

/// The `list --limit` value for a call: the flag's value validated, or
/// the unchanged default when the flag is absent.
pub(crate) fn list_limit_or_default(raw: Option<i64>) -> Result<i64, String> {
    validate_list_limit(raw.unwrap_or(LIST_LIMIT_DEFAULT))
}

fn env_or_default(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn default_queue_from_env(value: Option<&str>) -> String {
    env_or_default(value, DEFAULT_QUEUE)
}

fn signal_path_from_env(
    value: Option<&str>,
    home_dir: Option<&str>,
) -> Result<String, &'static str> {
    let home_dir = crate::home::resolve(home_dir)?;
    let default = format!(
        "{home_dir}/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json"
    );
    Ok(env_or_default(value, &default))
}

fn token_file_from_env(
    value: Option<&str>,
    home_dir: Option<&str>,
) -> Result<String, &'static str> {
    let home_dir = crate::home::resolve(home_dir)?;
    let default = format!("{home_dir}/.config/queue-broker/token");
    Ok(env_or_default(value, &default))
}

fn default_queue() -> String {
    let value = std::env::var("REPO_QUEUE_DEFAULT_QUEUE").ok();
    default_queue_from_env(value.as_deref())
}

fn opt_queue(args: &Args) -> String {
    args.get("queue").unwrap_or_else(default_queue)
}

/// Required queue resolution for claim/list/health (#7358, decision
/// recorded on the issue): the --queue flag wins, then the
/// REPO_QUEUE_DEFAULT_QUEUE env, and with neither nonblank the
/// command fails loudly before contacting the broker. There is
/// deliberately NO built-in default here, because the old
/// DEFAULT_QUEUE silently resolved to the shadow queue - the same
/// trap enqueue was hardened against in #7016. Pure, so the
/// resolution order is unit-testable.
fn resolve_required_queue(
    flag: Option<&str>,
    env_default: Option<&str>,
    verb: &str,
) -> Result<String, String> {
    for candidate in [flag, env_default] {
        if let Some(q) = candidate.map(str::trim).filter(|q| !q.is_empty()) {
            return Ok(q.to_string());
        }
    }
    Err(format!(
        "{verb}: no queue resolved - pass --queue or set REPO_QUEUE_DEFAULT_QUEUE ({verb} has no default queue; the old shadow default silently misrouted work)"
    ))
}

fn required_queue(args: &Args, verb: &str) -> String {
    let env_default = std::env::var("REPO_QUEUE_DEFAULT_QUEUE").ok();
    match resolve_required_queue(args.get("queue").as_deref(), env_default.as_deref(), verb) {
        Ok(q) => q,
        Err(e) => die(2, &e),
    }
}

fn opt_worker(args: &Args) -> String {
    args.get("worker")
        .unwrap_or_else(|| DEFAULT_WORKER.to_string())
}

fn load_packet(path: &str) -> Value {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => die(2, &format!("cannot read packet file: {e}")),
    };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => die(2, &format!("packet file is not valid JSON: {e}")),
    }
}

fn progress_arg(args: &Args) -> Option<Value> {
    let raw = args.get("progress");
    let pfile = args.get("progress-file");
    if raw.is_some() && pfile.is_some() {
        die(2, "pass only one of --progress / --progress-file");
    }
    let raw = if let Some(f) = pfile {
        match std::fs::read_to_string(&f) {
            Ok(t) => Some(t),
            Err(e) => die(2, &format!("cannot read progress file: {e}")),
        }
    } else {
        raw
    };
    let raw = raw?;
    let val: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => die(2, &format!("--progress is not valid JSON: {e}")),
    };
    if !val.is_object() {
        die(2, "--progress must be a JSON object");
    }
    Some(val)
}

fn execution_arg(args: &Args) -> Option<Value> {
    let raw = args.get("execution")?;
    let val: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => die(2, &format!("--execution is not valid JSON: {e}")),
    };
    if !val.is_object() {
        die(2, "--execution must be a JSON object");
    }
    if let Some(lane) = val.get("lane") {
        if lane != &json!("opencode") && lane != &json!("direct") {
            die(2, "--execution.lane must be 'opencode' or 'direct'");
        }
    }
    Some(val)
}

fn py_str_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", inner.join(", "))
}

fn check_telemetry_fields(obj: &Value, source: &str) {
    for field in TELEMETRY_FIELDS {
        let Some(v) = obj.get(field) else { continue };
        if v.is_null() {
            continue;
        }
        if field == "cost_usd" {
            let ok = matches!(v, Value::Number(n) if n.as_f64().map(|x| x >= 0.0).unwrap_or(false));
            if !ok {
                die(
                    2,
                    &format!(
                        "{source}: telemetry field 'cost_usd' must be a non-negative number or null"
                    ),
                );
            }
        } else if !matches!(v, Value::String(s) if !s.is_empty()) {
            die(
                2,
                &format!("{source}: telemetry field '{field}' must be a non-empty string or null"),
            );
        }
    }
}

/// Shared validate flow for `validate` and `enqueue`: returns warnings
/// after printing them; dies (exit 2) on errors exactly like
/// repo-queue.py's check_packet.
fn validate_via_broker(packet: &Value, quiet: bool) -> Vec<String> {
    let resp = call("/validate", &json!({"packet": packet}));
    let resp = unwrap_ok(resp);
    let warnings: Vec<String> = resp
        .get("warnings")
        .and_then(|w| w.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let errors: Vec<String> = resp
        .get("errors")
        .and_then(|w| w.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    for w in &warnings {
        eprint(&format!("warning: {w}"));
    }
    if !errors.is_empty() {
        for e in &errors {
            eprint(&format!("error: {e}"));
        }
        die(
            2,
            &format!("packet validation failed ({} error(s))", errors.len()),
        );
    }
    if !quiet {
        eprint("packet valid (repo-task/v1)");
    }
    warnings
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// #8097 Rule 2: a --platform value is a coarse FAMILY selector,
/// never a lane name: exactly "openchamber" or "muse". Shared by
/// the enqueue and claim verbs so both spell the rule once.
fn platform_flag(v: &str) -> Result<String, String> {
    match v {
        "openchamber" | "muse" => Ok(v.to_string()),
        other => Err(format!(
            "invalid --platform {other:?} (expected \"openchamber\" or \"muse\")"
        )),
    }
}

/// #8176: the claim-model stamp flags. The claiming session reports
/// its own serving leaf — provider and model together (the leaf it
/// read from its session surface, route_usage's lastServing,
/// opencode#1891), variant optional. Naming either half alone is a
/// usage error, mirroring the broker's parse: a half-stamp must
/// never reach the wire.
fn model_stamp_flags(
    provider: Option<&str>,
    model: Option<&str>,
    variant: Option<&str>,
) -> Result<Option<(String, String, Option<String>)>, String> {
    fn clean<'a>(v: Option<&'a str>) -> Option<&'a str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    match (clean(provider), clean(model)) {
        (None, None) => {
            if clean(variant).is_some() {
                return Err(
                    "--model-variant requires --model-provider and --model-id".to_string(),
                );
            }
            Ok(None)
        }
        (Some(p), Some(m)) => Ok(Some((
            p.to_string(),
            m.to_string(),
            clean(variant).map(str::to_string),
        ))),
        (Some(_), None) => Err(
            "--model-provider requires --model-id (the claim stamp names the serving leaf, provider and model together)"
                .to_string(),
        ),
        (None, Some(_)) => Err(
            "--model-id requires --model-provider (the claim stamp names the serving leaf, provider and model together)"
                .to_string(),
        ),
    }
}

fn cmd_validate(rest: &[String]) -> i32 {
    let args = parse_args(rest, &[], &[]);
    reject_extra_pos("validate", &args, 1);
    let path = args
        .pos
        .first()
        .cloned()
        .unwrap_or_else(|| die(2, "validate: packet required"));
    let packet = load_packet(&path);
    validate_via_broker(&packet, false);
    print_json(&json!({
        "valid": true,
        "dedupe_key": packet.get("dedupe_key").cloned().unwrap_or(Value::Null),
        "kind": packet.get("payload").and_then(|p| p.get("kind")).cloned().unwrap_or(Value::Null),
        "title": packet.get("title").cloned().unwrap_or(Value::Null),
    }));
    0
}

/// Enqueue queue resolution (#7016, the shadow-default trap): the
/// --queue flag wins, then the packet's own declared queue, then
/// the REPO_QUEUE_DEFAULT_QUEUE env. There is deliberately NO
/// built-in default for enqueue: with none of the three the enqueue
/// fails loudly instead of silently landing in the shadow queue —
/// the failure that misrouted ~291 tasks on 2026-11-07. Pure, so
/// the resolution order is unit-testable.
fn resolve_enqueue_queue(
    flag: Option<&str>,
    packet: &Value,
    env_default: Option<&str>,
) -> Result<String, String> {
    let packet_queue = packet
        .get("queue")
        .and_then(|q| q.as_str())
        .map(str::trim)
        .filter(|q| !q.is_empty());
    for candidate in [flag, packet_queue, env_default] {
        if let Some(q) = candidate.map(str::trim).filter(|q| !q.is_empty()) {
            return Ok(q.to_string());
        }
    }
    Err(
        "enqueue: no queue resolved — pass --queue, declare \"queue\" in the \
         packet, or set REPO_QUEUE_DEFAULT_QUEUE (enqueue has no default queue; \
         the old shadow default silently misrouted work)"
            .to_string(),
    )
}

fn cmd_enqueue(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["queue", "platform"], &["dry-run", "shadow-intent"]);
    reject_extra_pos("enqueue", &args, 1);
    let path = args
        .pos
        .first()
        .cloned()
        .unwrap_or_else(|| die(2, "enqueue: packet required"));
    let packet = load_packet(&path);
    let dry_run = args.has("dry-run");
    validate_via_broker(&packet, dry_run);
    let env_default = std::env::var("REPO_QUEUE_DEFAULT_QUEUE").ok();
    let queue = match resolve_enqueue_queue(
        args.get("queue").as_deref(),
        &packet,
        env_default.as_deref(),
    ) {
        Ok(q) => q,
        Err(e) => die(2, &e),
    };
    // #8097 Rule 2: the platform selector is optional; when given
    // it restricts the item to that family. Absent = unset.
    let platform = match args.get("platform") {
        None => None,
        Some(p) => match platform_flag(&p) {
            Ok(v) => Some(v),
            Err(e) => die(2, &e),
        },
    };
    let shadow_intent = args.has("shadow-intent");
    if queue.ends_with("-shadow") && !shadow_intent {
        die(
            2,
            &format!(
                "enqueue: queue {queue} is a shadow queue — pass --shadow-intent \
                 to enqueue there deliberately"
            ),
        );
    }
    let call_started = now_micros();
    let resp = call(
        "/enqueue",
        &json!({
            "packet": packet, "queue": queue, "dry_run": dry_run,
            "shadow_intent": shadow_intent, "platform": platform,
        }),
    );
    let resp = unwrap_ok(resp);
    if dry_run {
        let mut task = resp.get("task").cloned().unwrap_or(json!({}));
        if let Some(o) = task.as_object_mut() {
            o.insert("_dry_run".to_string(), Value::Bool(true));
            o.insert("_rpc".to_string(), json!("agent.enqueue_repo_task"));
        }
        print_json(&task);
        return 0;
    }
    let row = resp.get("row").cloned().unwrap_or(Value::Null);
    // Outcome classification (mirrors _enqueue_outcome in repo-queue.py).
    let tid = row.get("id").cloned().unwrap_or(Value::Null);
    let q = row
        .get("queue")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let s = row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_raw = row.get("created_at").cloned().unwrap_or(Value::Null);
    let created = parse_ts(&created_raw);
    let updated = parse_ts(&row.get("updated_at").cloned().unwrap_or(Value::Null));
    let skew = 10_000_000i64; // 10 s in micros
    let mut warnings: Vec<String> = Vec::new();
    let line;
    if let Some(pred) = resp.get("revived_from") {
        // Broker-side revive (#6838): the key's prior task was
        // terminal, so this enqueue created a fresh task linked to
        // its predecessor.
        line = format!(
            "revived: new task {} (queue {q}, status {s}), predecessor task {} ({})",
            fmt_cell(Some(&tid)),
            fmt_cell(pred.get("task_id")),
            fmt_cell(pred.get("status")),
        );
    } else if created.map(|c| c >= call_started - skew).unwrap_or(false) {
        line = format!(
            "enqueued new task {} (queue {q}, status {s})",
            fmt_cell(Some(&tid))
        );
    } else {
        if q != queue {
            warnings.push(format!(
                "dedupe hit is in a different queue ({q} != requested {queue})"
            ));
        }
        if s == "queued" && updated.map(|u| u >= call_started - skew).unwrap_or(false) {
            line = format!(
                "revived: existing task {} (queue {q}, status {s}, created {})",
                fmt_cell(Some(&tid)),
                fmt_cell(Some(&created_raw))
            );
        } else {
            if TERMINAL_STATUSES.contains(&s.as_str()) {
                warnings.push(format!("dedupe hit is in a terminal state (status {s})"));
            }
            line = format!(
                "dedupe hit: existing task {} (queue {q}, status {s}, created {})",
                fmt_cell(Some(&tid)),
                fmt_cell(Some(&created_raw))
            );
        }
    }
    println!("{line}");
    for w in &warnings {
        println!("warning: {w}");
    }
    print_json(&row);
    0
}

/// One `/list` row as the printed table cells, in column order
/// (task, priority, status, age, dedupe_key, title). Pure so the
/// claimed-list recovery contract (#5486) is unit-testable: the cells
/// are built from exactly the identifying fields the broker returns.
fn list_row_cells(r: &Value) -> Vec<String> {
    vec![
        format!("task {}", fmt_cell(r.get("id"))),
        fmt_cell(r.get("priority")),
        fmt_cell(r.get("status")),
        age_str(&r.get("created_at").cloned().unwrap_or(Value::Null)),
        fmt_cell(r.get("dedupe_key")),
        truncate(&fmt_cell(r.get("title")), 72),
    ]
}

/// Find a task row by id in a `/list` rows array — the recovery path a
/// worker uses when a claim's own output was lost: `list --status
/// claimed` must return the same task with its identifying fields.
fn find_task_row(rows: &[Value], task_id: i64) -> Option<&Value> {
    rows.iter()
        .find(|r| r.get("id").and_then(|v| v.as_i64()) == Some(task_id))
}

fn cmd_list(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &["queue", "status", "limit", "not-before", "lane", "worker"],
        &[],
    );
    reject_extra_pos("list", &args, 0);
    let limit = match list_limit_or_default(args.get_i64("limit")) {
        Ok(limit) => limit,
        Err(e) => die(2, &e),
    };
    let resp = call(
        "/list",
        &json!({
            "queue": required_queue(&args, "list"),
            "status": args.get("status"),
            "limit": limit,
            "not_before": args.get("not-before"),
            "lane": args.get("lane"),
            "worker": args.get("worker"),
        }),
    );
    let resp = unwrap_ok(resp);
    let rows = resp
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if rows.is_empty() {
        println!("(no tasks)");
        return 0;
    }
    let cols: Vec<String> = ["task", "priority", "status", "age", "dedupe_key", "title"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let str_rows: Vec<Vec<String>> = rows.iter().map(list_row_cells).collect();
    print_table(&cols, &str_rows);
    0
}

fn cmd_claim(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &[
            "queue",
            "worker",
            "lease-seconds",
            "lane",
            "task",
            "platform",
            "session-id",
            "model-provider",
            "model-id",
            "model-variant",
        ],
        &["handoff"],
    );
    reject_extra_pos("claim", &args, 0);
    let worker = opt_worker(&args);
    let mut body = json!({
        "queue": required_queue(&args, "claim"),
        "worker": worker,
        "lease_seconds": args.get_i64("lease-seconds").unwrap_or(2700),
    });
    if let Some(lane) = args.get("lane") {
        body["lane"] = json!(lane);
    }
    // --platform <family>: declare this claim's platform MARK
    // (#8097 Rule 2, demoted by #8187). The broker stamps it on
    // the claimed row and the claimed event; it filters NOTHING —
    // a mismatched or undeclared claim claims items all the same.
    if let Some(p) = args.get("platform") {
        match platform_flag(&p) {
            Ok(v) => body["platform"] = json!(v),
            Err(e) => die(2, &e),
        }
    }
    // --session-id: the claiming session's own id, recorded as a
    // mark on the claimed event when the surface knows it (#8187).
    if let Some(sid) = args.get("session-id") {
        body["session_id"] = json!(sid);
    }
    // --model-provider/--model-id[/--model-variant]: the claiming
    // session's own model, stamped on the task by the broker
    // (#8176). The session reads its serving leaf from its own
    // session surface (route_usage lastServing, opencode#1891)
    // and passes it here; the broker stores it as the row's
    // claim_provider / claim_model / claim_variant.
    match model_stamp_flags(
        args.get("model-provider").as_deref(),
        args.get("model-id").as_deref(),
        args.get("model-variant").as_deref(),
    ) {
        Ok(Some((provider, model, variant))) => {
            body["model_provider"] = json!(provider);
            body["model_id"] = json!(model);
            if let Some(v) = variant {
                body["model_variant"] = json!(v);
            }
        }
        Ok(None) => {}
        Err(e) => die(2, &e),
    }
    // --handoff: the no-id collection of the next parked recorded
    // opencode handoff (#8097 Rule 1) — the claimant's default
    // parked-handoff path. The BROKER picks the row; the caller
    // names no task id. (--task remains the special-case claim.)
    if args.has("handoff") {
        body["pick"] = json!("parked_handoff");
    }
    // --task <id>: by-id claim — take exactly that row when the
    // broker's by-id predicate allows it (a queued row under the
    // ordinary predicate, or a parked recorded opencode handoff).
    // The opencode claimant uses this to collect handed-off tasks
    // out of the parked sink; a plain claim never names a task.
    if let Some(task) = args.get_i64("task") {
        body["task_id"] = json!(task);
    }
    let resp = unwrap_ok_fb(call_fb("/claim", &body), "/claim");
    let (mut out, code) = claim_output(&resp);
    if code != 0 {
        eprint("repo-queue: no claimable task");
        print_json(&out);
        return code;
    }
    // No-double-serve guard (#6741): a task whose issue is held in
    // the GitHub fallback right now must not be served from the
    // primary store — block it back and report no claim.
    match fallback_guard(&out, &worker) {
        GuardOutcome::HeldBy(holder) => {
            let task_id = out.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let (repo, issue) = task_issue(&out);
            let blocker = format!(
                "issue {repo}#{issue} is held in the GitHub fallback by {holder}; \
                 primary-store claim auto-blocked to prevent double-serve (#6741)"
            );
            let resp = call(
                "/block",
                &json!({
                    "task_id": task_id,
                    "worker": worker,
                    "reason_class": "fallback_held",
                    "blocker": blocker,
                    "progress": {},
                }),
            );
            if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
                eprint(
                    "repo-queue: WARNING: fallback guard block failed;                      the task stays claimed under its lease",
                );
            }
            print_json(&json!({
                "task": Value::Null,
                "guard": "fallback_held",
                "held_by": holder,
                "repo": repo,
                "issue": issue,
            }));
            NO_CLAIM_EXIT
        }
        GuardOutcome::Unchecked(e) => {
            if let Some(o) = out.as_object_mut() {
                o.insert(
                    "guard_warning".to_string(),
                    json!(format!(
                        "fallback double-serve guard could not check GitHub: {e}"
                    )),
                );
            }
            print_json(&out);
            code
        }
        GuardOutcome::Serve => {
            print_json(&out);
            code
        }
    }
}

/// The (repo, issue) identity a claimed task's payload carries, for
/// the fallback guard. ("", 0) when the packet names none.
fn task_issue(row: &Value) -> (String, i64) {
    let payload = row.get("payload").cloned().unwrap_or(Value::Null);
    (
        payload
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        payload
            .get("issue")
            .and_then(|i| i.get("number"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
    )
}

#[derive(Debug, PartialEq, Eq)]
enum GuardDecision {
    Serve,
    HeldBy(String),
}

/// Pure guard decision over a fallback claim-state object (the
/// shape `fallback status` prints): live or contested in the
/// fallback under a DIFFERENT worker means hands off; anything
/// else (free / released / stale / held by me) may be served.
fn guard_decision(state: &Value, my_worker: &str) -> GuardDecision {
    let name = state.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let holder = state.get("worker").and_then(|v| v.as_str()).unwrap_or("");
    if (name == "live" || name == "contested") && !holder.is_empty() && holder != my_worker {
        GuardDecision::HeldBy(holder.to_string())
    } else {
        GuardDecision::Serve
    }
}

enum GuardOutcome {
    Serve,
    HeldBy(String),
    /// The GitHub check itself failed: serve, but say so — the
    /// guard fails open with a warning rather than stalling every
    /// claim on a GitHub outage (the primary store failing is what
    /// the fallback switch above is for).
    Unchecked(String),
}

fn fallback_guard(row: &Value, my_worker: &str) -> GuardOutcome {
    let (repo, issue) = task_issue(row);
    if repo.is_empty() || issue <= 0 {
        return GuardOutcome::Serve;
    }
    match crate::fallback::issue_claim_state(&repo, issue) {
        Ok(state) => match guard_decision(&state, my_worker) {
            GuardDecision::HeldBy(h) => GuardOutcome::HeldBy(h),
            GuardDecision::Serve => GuardOutcome::Serve,
        },
        Err(e) => GuardOutcome::Unchecked(e),
    }
}

/// Exit code for a claim that found no task: nonzero, so an empty
/// claim can never be mistaken for a successful one (#5486).
const NO_CLAIM_EXIT: i32 = 5;

/// Decode a `/claim` broker envelope into the (stdout value, exit
/// code) pair `cmd_claim` emits. Pure so the wire/output contract is
/// unit-testable without a broker. The oracle is the envelope shape
/// itself (#5486): `{ok:true, row:{id,queue,status,dedupe_key,...}}`
/// prints the row VERBATIM with exit 0 — the reported failure was an
/// all-null row printed instead of the claimed task — while a null
/// (or missing) row yields the distinct `{"task": null}` marker with
/// NO_CLAIM_EXIT, a value carrying no id/queue/status/dedupe_key
/// fields a caller could mistake for a task.
fn claim_output(resp: &Value) -> (Value, i32) {
    let row = resp.get("row").cloned().unwrap_or(Value::Null);
    if row.is_null() {
        return (json!({"task": Value::Null}), NO_CLAIM_EXIT);
    }
    // Defense in depth (#5444): the broker's claim path validates
    // packets before handing them out, but a row that still arrives
    // carrying a payload field that is null or an empty object is a
    // payload-less packet — never print it as a claimed task. Rows
    // without a payload field keep the #5486 verbatim contract.
    if let Some(payload) = row.get("payload") {
        let missing = payload.is_null() || payload.as_object().is_some_and(|o| o.is_empty());
        if missing {
            eprint(&format!(
                "repo-queue: claim skipped: task {} arrived payload-less (#5444); treating as no claim",
                row.get("id").cloned().unwrap_or(Value::Null)
            ));
            return (json!({"task": Value::Null}), NO_CLAIM_EXIT);
        }
    }
    (row, 0)
}

fn cmd_lane(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &["worker", "exclusion-reason", "opencode-reason"],
        &[],
    );
    reject_extra_pos("lane", &args, 1);
    let resp = unwrap_ok(call(
        "/lane",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": opt_worker(&args),
            "exclusion_reason": args.get("exclusion-reason"),
            "opencode_reason": args.get("opencode-reason"),
        }),
    ));
    print_json(&require_row(&resp, "assign_repo_task_lane"));
    0
}

fn cmd_bind(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["worker", "session-id", "launched-by"], &[]);
    reject_extra_pos("bind", &args, 1);
    let session_id = args
        .get("session-id")
        .unwrap_or_else(|| die(2, "bind: --session-id is required"));
    let resp = unwrap_ok(call(
        "/bind",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": opt_worker(&args),
            "session_id": session_id,
            "launched_by": args.get("launched-by").unwrap_or_else(|| "opencode-claimant".to_string()),
        }),
    ));
    print_json(&require_row(&resp, "bind_repo_task_lane_session"));
    0
}

fn cmd_heartbeat(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &["worker", "extend-seconds", "progress", "progress-file"],
        &[],
    );
    reject_extra_pos("heartbeat", &args, 1);
    let resp = unwrap_ok_fb(
        call_fb(
            "/heartbeat",
            &json!({
                "task_id": args.pos_i64(0, "task_id"),
                "worker": opt_worker(&args),
                "extend_seconds": args.get_i64("extend-seconds").unwrap_or(900),
                "progress": progress_arg(&args),
            }),
        ),
        "/heartbeat",
    );
    print_json(&require_row(&resp, "heartbeat_task"));
    0
}

fn cmd_checkpoint(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["worker", "stage", "progress", "progress-file"], &[]);
    reject_extra_pos("checkpoint", &args, 1);
    let stage = args
        .get("stage")
        .unwrap_or_else(|| die(2, "checkpoint: --stage is required"));
    let resp = unwrap_ok_fb(
        call_fb(
            "/checkpoint",
            &json!({
                "task_id": args.pos_i64(0, "task_id"),
                "worker": opt_worker(&args),
                "stage": stage,
                "progress": progress_arg(&args).unwrap_or(json!({})),
            }),
        ),
        "/checkpoint",
    );
    print_json(&require_row(&resp, "checkpoint_task"));
    0
}

fn cmd_park(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &[
            "worker",
            "reason",
            "delay-seconds",
            "not-before",
            "progress",
            "progress-file",
            "ready-condition",
            "execution",
        ],
        &[],
    );
    reject_extra_pos("park", &args, 1);
    if args.has("delay-seconds") && args.has("not-before") {
        die(
            2,
            "argument --not-before: not allowed with argument --delay-seconds",
        );
    }
    let reason = args
        .get("reason")
        .unwrap_or_else(|| die(2, "park: --reason is required"));
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": reason,
        "progress": progress_arg(&args).unwrap_or(json!({})),
    });
    if let Some(nb) = args.get("not-before") {
        body["not_before"] = json!(nb);
    } else if let Some(d) = args.get_i64("delay-seconds") {
        body["delay_seconds"] = json!(d);
    }
    if let Some(rc_raw) = args.get("ready-condition") {
        let rc: Value = match serde_json::from_str(&rc_raw) {
            Ok(v) => v,
            Err(e) => die(2, &format!("--ready-condition is not valid JSON: {e}")),
        };
        if !rc.is_object() {
            die(2, "--ready-condition must be a JSON object");
        }
        body["ready_condition"] = rc;
    }
    if let Some(ex) = execution_arg(&args) {
        body["execution"] = ex;
    }
    let resp = unwrap_ok(call("/park", &body));
    row_or_ledger(&resp, "park_task");
    0
}

/// wake (#8184): release a broker-held parked wait now that
/// its condition is satisfied (e.g. the triage step finished
/// triaging the item). The broker flips it back to queued.
fn cmd_wake(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["worker", "reason"], &[]);
    reject_extra_pos("wake", &args, 1);
    let body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": args.get("reason").unwrap_or_default(),
    });
    let resp = unwrap_ok(call("/wake", &body));
    print_json(&require_row(&resp, "wake_task"));
    0
}

fn cmd_block(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &[
            "worker",
            "reason-class",
            "blocker",
            "progress",
            "progress-file",
        ],
        &[],
    );
    reject_extra_pos("block", &args, 1);
    let rc = args
        .get("reason-class")
        .unwrap_or_else(|| die(2, "block: --reason-class is required"));
    let blocker = args
        .get("blocker")
        .unwrap_or_else(|| die(2, "block: --blocker is required"));
    let resp = unwrap_ok(call(
        "/block",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": opt_worker(&args),
            "reason_class": rc,
            "blocker": blocker,
            "progress": progress_arg(&args).unwrap_or(json!({})),
        }),
    ));
    print_json(&require_row(&resp, "block_task"));
    0
}

fn cmd_cancel(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["worker", "reason"], &[]);
    reject_extra_pos("cancel", &args, 1);
    let reason = args
        .get("reason")
        .unwrap_or_else(|| die(2, "cancel: --reason is required"));
    let worker = args
        .get("worker")
        .unwrap_or_else(|| "fleet-monitor".to_string());
    let resp = unwrap_ok(call(
        "/cancel",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": worker,
            "reason": reason,
        }),
    ));
    print_json(&require_row(&resp, "cancel_repo_task"));
    0
}

/// Revive a dead task in place (#6838): a failed task, or a queued
/// task dead at cap (attempts exhausted), gets attempts=0 and a
/// fresh not_before through the broker's /revive verb — the
/// supported replacement for the monitor's direct-SQL resets.
fn cmd_revive(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["worker", "reason", "not-before"], &[]);
    reject_extra_pos("revive", &args, 1);
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": args.get("reason").unwrap_or_default(),
    });
    if let Some(nb) = args.get("not-before") {
        body["not_before"] = json!(nb);
    }
    let resp = unwrap_ok(call("/revive", &body));
    print_json(&require_row(&resp, "revive_task"));
    0
}

fn cmd_fail(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["worker", "error", "reason-class", "execution"], &[]);
    reject_extra_pos("fail", &args, 1);
    let error = args
        .get("error")
        .unwrap_or_else(|| die(2, "fail: --error is required"));
    let reason_class = args
        .get("reason-class")
        .unwrap_or_else(|| "transient".to_string());
    if !["transient", "deterministic", "blocked"].contains(&reason_class.as_str()) {
        die(
            2,
            &format!("argument --reason-class: invalid choice: '{reason_class}'"),
        );
    }
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "error": error,
        "reason_class": reason_class,
    });
    if let Some(ex) = execution_arg(&args) {
        body["execution"] = ex;
    }
    let resp = unwrap_ok(call("/fail", &body));
    row_or_ledger(&resp, "fail_repo_task");
    0
}

fn cmd_complete(rest: &[String]) -> i32 {
    let args = parse_args(
        rest,
        &["worker", "result", "result-file", "telemetry", "execution"],
        &[],
    );
    reject_extra_pos("complete", &args, 1);
    let mut result_raw = args.get("result");
    if let Some(f) = args.get("result-file") {
        result_raw = Some(match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => die(2, &format!("cannot read result file: {e}")),
        });
    }
    let mut result_obj = json!({});
    if let Some(raw) = result_raw {
        if !raw.is_empty() {
            result_obj = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => die(2, &format!("--result is not valid JSON: {e}")),
            };
            if !result_obj.is_object() {
                die(2, "--result must be a JSON object");
            }
        }
    }
    check_telemetry_fields(&result_obj, "--result");
    if let Some(tel_raw) = args.get("telemetry") {
        let tel: Value = match serde_json::from_str(&tel_raw) {
            Ok(v) => v,
            Err(e) => die(2, &format!("--telemetry is not valid JSON: {e}")),
        };
        if !tel.is_object() {
            die(2, "--telemetry must be a JSON object");
        }
        let unknown: Vec<String> = tel
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| !TELEMETRY_FIELDS.contains(&k.as_str()))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            let mut sorted = unknown.clone();
            sorted.sort();
            die(
                2,
                &format!(
                    "--telemetry: unknown field(s) {}; allowed: {}",
                    py_str_list(&sorted),
                    py_str_list(
                        &TELEMETRY_FIELDS
                            .iter()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                    )
                ),
            );
        }
        check_telemetry_fields(&tel, "--telemetry");
        for field in TELEMETRY_FIELDS {
            if tel.get(field).is_some() && result_obj.get(field).is_none() {
                result_obj[field] = tel[field].clone();
            }
        }
    }
    let ex = execution_arg(&args);
    if let Some(ex) = &ex {
        if result_obj.get("execution").is_none() {
            let mut block = Map::new();
            block.insert(
                "lane".to_string(),
                ex.get("lane").cloned().unwrap_or(Value::Null),
            );
            block.insert(
                "session_id".to_string(),
                ex.get("session_id").cloned().unwrap_or(Value::Null),
            );
            let has_fb = ex
                .get("fallback_reason")
                .map(|v| !v.is_null())
                .unwrap_or(false);
            block.insert("fallback".to_string(), Value::Bool(has_fb));
            if has_fb {
                block.insert("fallback_reason".to_string(), ex["fallback_reason"].clone());
            }
            if let Some(a) = ex.get("assigned") {
                block.insert("assigned".to_string(), a.clone());
            }
            result_obj["execution"] = Value::Object(block);
        }
    }
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "result": result_obj,
    });
    if let Some(ex) = ex {
        body["execution"] = ex;
    }
    let resp = unwrap_ok_fb(call_fb("/complete", &body), "/complete");
    row_or_ledger(&resp, "complete_task");
    0
}

fn cmd_health(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["queue"], &[]);
    reject_extra_pos("health", &args, 0);
    let queue = required_queue(&args, "health");
    let body = json!({"queue": queue});
    let resp = unwrap_ok_fb(call_fb("/health", &body), "/health");
    let netloc = resp
        .get("project_netloc")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source = resp
        .get("project_source")
        .and_then(|v| v.as_str())
        .unwrap_or("bitwarden");
    println!("project: {netloc} (resolved via {source})");
    let rows = resp
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if rows.is_empty() {
        println!("(no queues have tasks)");
        return 0;
    }
    let cols: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let str_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| cols.iter().map(|c| fmt_cell(r.get(c))).collect())
        .collect();
    print_table(&cols, &str_rows);
    0
}

fn cmd_signal(rest: &[String]) -> i32 {
    let args = parse_args(rest, &["queue", "reason"], &[]);
    reject_extra_pos("signal", &args, 0);
    let queue = opt_queue(&args);
    let reason = args
        .get("reason")
        .unwrap_or_else(|| "manual_signal".to_string());
    let resp = unwrap_ok(call("/signal-count", &json!({"queue": queue})));
    let queued_count = resp.get("queued_count").cloned().unwrap_or(json!(0));
    let oldest = resp.get("oldest_queued_at").cloned().unwrap_or(Value::Null);

    let signal_path_env = std::env::var("REPO_QUEUE_SIGNAL_PATH").ok();
    let signal_path = signal_path_from_env(signal_path_env.as_deref(), Some(&home()))
        .unwrap_or_else(|error| die(3, error));
    let dir = std::path::Path::new(&signal_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let state_path = format!("{dir}/repo-queue-signal-state.json");
    let mut generation: i64 = 0;
    for candidate in [&signal_path, &state_path] {
        if let Ok(text) = std::fs::read_to_string(candidate) {
            if let Ok(prev) = serde_json::from_str::<Value>(&text) {
                if prev.get("queue").and_then(|q| q.as_str()) == Some(queue.as_str()) {
                    if let Some(g) = prev.get("generation").and_then(|g| g.as_i64()) {
                        generation = generation.max(g);
                    }
                }
            }
        }
    }
    generation += 1;
    let updated = utcnow_iso();
    let signal = json!({
        "queue": queue,
        "generation": generation,
        "queued_count": queued_count,
        "oldest_queued_at": oldest,
        "updated_at": updated,
        "reason": reason,
    });
    let state = json!({
        "queue": queue,
        "generation": generation,
        "updated_at": updated,
    });
    let _ = std::fs::create_dir_all(&dir);
    atomic_write_json(&signal_path, &signal);
    atomic_write_json(&state_path, &state);
    print_json(&signal);
    0
}

fn atomic_write_json(path: &str, obj: &Value) {
    let tmp = format!("{path}.tmp{}", std::process::id());
    let mut text = serde_json::to_string_pretty(obj).unwrap_or_default();
    text.push('\n');
    match std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, path)) {
        Ok(_) => {}
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            die(3, &format!("cannot write {path}: {e}"));
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    if args.is_empty() {
        eprint(
            "usage: repo-queue <validate|list|enqueue|claim|lane|bind|heartbeat|checkpoint|park|wake|block|cancel|fail|complete|revive|health|signal|fallback> ...",
        );
        return 2;
    }
    // Fail before dispatching any verb, so unset and empty HOME never
    // reach a broker or filesystem path with a root-anchored default.
    let _ = home();
    let sub = args[0].as_str();
    let rest = &args[1..];
    match sub {
        "validate" => cmd_validate(rest),
        "enqueue" => cmd_enqueue(rest),
        "list" => cmd_list(rest),
        "claim" => cmd_claim(rest),
        "lane" => cmd_lane(rest),
        "bind" => cmd_bind(rest),
        "heartbeat" => cmd_heartbeat(rest),
        "checkpoint" => cmd_checkpoint(rest),
        "park" => cmd_park(rest),
        "wake" => cmd_wake(rest),
        "block" => cmd_block(rest),
        "cancel" => cmd_cancel(rest),
        "revive" => cmd_revive(rest),
        "fail" => cmd_fail(rest),
        "complete" => cmd_complete(rest),
        "health" => cmd_health(rest),
        "signal" => cmd_signal(rest),
        "fallback" => crate::fallback::run(rest),
        _ => {
            eprint(&format!("repo-queue: unknown command {sub:?}"));
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    #[test]
    fn model_stamp_flags_require_provider_and_model_together() {
        // #8176 oracle: the claim stamp names the serving leaf —
        // provider and model together, variant optional. Either
        // half alone (or a variant alone) is a usage error.
        assert_eq!(
            model_stamp_flags(Some("_claude"), Some("claude-sonnet-5-5"), Some("medium")),
            Ok(Some((
                "_claude".to_string(),
                "claude-sonnet-5-5".to_string(),
                Some("medium".to_string()),
            )))
        );
        assert_eq!(
            model_stamp_flags(Some("openai.main"), Some("gpt-6-luna"), None),
            Ok(Some((
                "openai.main".to_string(),
                "gpt-6-luna".to_string(),
                None,
            )))
        );
        assert_eq!(model_stamp_flags(None, None, None), Ok(None));
        assert!(model_stamp_flags(Some("_claude"), None, None).is_err());
        assert!(model_stamp_flags(None, Some("claude-sonnet-5-5"), None).is_err());
        assert!(model_stamp_flags(None, None, Some("medium")).is_err());
    }

    #[test]
    fn platform_flag_accepts_only_the_two_families() {
        // #8097 Rule 2: the selector is a coarse family, never a
        // lane name — a lane id as --platform is a usage error.
        assert_eq!(platform_flag("openchamber"), Ok("openchamber".to_string()));
        assert_eq!(platform_flag("muse"), Ok("muse".to_string()));
        assert!(platform_flag("code-a").is_err());
        assert!(platform_flag("").is_err());
    }

    #[test]
    fn fallback_switch_json_shape() {
        let v = fallback_switch_json("/claim", 4, "store down");
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["fallback"], json!(true));
        assert_eq!(v["switch_to"], json!("github-fallback"));
        assert_eq!(v["verb"], json!("/claim"));
        assert_eq!(v["broker_exit"], json!(4));
        assert_eq!(FALLBACK_SWITCH_EXIT, 6);
    }

    #[test]
    fn guard_decision_holds_only_live_or_contested_others() {
        let live_other = json!({"state": "live", "worker": "lane-b"});
        assert_eq!(
            guard_decision(&live_other, "lane-a"),
            GuardDecision::HeldBy("lane-b".to_string())
        );
        let contested = json!({"state": "contested", "worker": "lane-b"});
        assert_eq!(
            guard_decision(&contested, "lane-a"),
            GuardDecision::HeldBy("lane-b".to_string())
        );
        // Held by me, stale, released, free: all servable.
        assert_eq!(guard_decision(&live_other, "lane-b"), GuardDecision::Serve);
        assert_eq!(
            guard_decision(&json!({"state": "stale", "worker": "lane-b"}), "lane-a"),
            GuardDecision::Serve
        );
        assert_eq!(
            guard_decision(&json!({"state": "released", "worker": "lane-b"}), "lane-a"),
            GuardDecision::Serve
        );
        assert_eq!(
            guard_decision(&json!({"state": "free"}), "lane-a"),
            GuardDecision::Serve
        );
    }

    #[test]
    fn task_issue_reads_payload_identity() {
        let row = json!({"payload": {"repo": "developing-today/code", "issue": {"number": 6741}}});
        assert_eq!(
            task_issue(&row),
            ("developing-today/code".to_string(), 6741)
        );
        assert_eq!(task_issue(&json!({"payload": {}})), (String::new(), 0));
    }

    fn pos(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn assert_env_resolution(resolve: impl Fn(Option<&str>) -> String, default: &str) {
        for value in [None, Some(""), Some(" \t\n")] {
            assert_eq!(resolve(value), default);
        }
        assert_eq!(
            resolve(Some(" /tmp/configured path ")),
            "/tmp/configured path"
        );
    }

    #[test]
    fn default_queue_empty_env_values_use_default() {
        assert_env_resolution(default_queue_from_env, DEFAULT_QUEUE);
    }

    #[test]
    fn enqueue_queue_resolution_never_falls_back_to_shadow() {
        // #7016: the flag wins over the packet, the packet wins over
        // the env, and with none of the three the enqueue fails
        // loudly — it never resolves to the shadow default.
        let packet = json!({"queue": "repo-health-code-pilot"});
        assert_eq!(
            resolve_enqueue_queue(Some("repo-health-nix-pilot"), &packet, Some("env-q")).unwrap(),
            "repo-health-nix-pilot"
        );
        assert_eq!(
            resolve_enqueue_queue(None, &packet, Some("env-q")).unwrap(),
            "repo-health-code-pilot"
        );
        assert_eq!(
            resolve_enqueue_queue(None, &json!({}), Some("env-q")).unwrap(),
            "env-q"
        );
        assert!(resolve_enqueue_queue(None, &json!({}), None).is_err());
        assert!(resolve_enqueue_queue(None, &json!({"queue": "  "}), Some("")).is_err());
        assert!(resolve_enqueue_queue(Some(""), &json!({}), None).is_err());
    }

    #[test]
    fn required_queue_resolution_never_falls_back_to_shadow() {
        // #7358: claim/list/health share this rule - the flag wins
        // over the env, and with neither nonblank the resolution
        // fails loudly naming --queue; it never resolves to the
        // shadow default.
        assert_eq!(
            resolve_required_queue(Some("repo-health-nix-pilot"), Some("env-q"), "claim").unwrap(),
            "repo-health-nix-pilot"
        );
        assert_eq!(
            resolve_required_queue(None, Some("env-q"), "list").unwrap(),
            "env-q"
        );
        assert_eq!(
            resolve_required_queue(Some("  "), Some(" env-q "), "health").unwrap(),
            "env-q"
        );
        // Explicit shadow stays reachable when passed deliberately.
        assert_eq!(
            resolve_required_queue(Some("repo-health-code-shadow"), None, "claim").unwrap(),
            "repo-health-code-shadow"
        );
        for verb in ["claim", "list", "health"] {
            let err = resolve_required_queue(None, None, verb).unwrap_err();
            assert!(err.contains("--queue"), "{err}");
            assert!(err.starts_with(verb), "{err}");
            assert!(resolve_required_queue(Some(""), Some(" \t\n"), verb).is_err());
        }
    }

    #[test]
    fn signal_path_empty_env_values_use_home_default() {
        let home_dir = "/home/test";
        let default = format!(
            "{home_dir}/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json"
        );
        assert_env_resolution(
            |value| signal_path_from_env(value, Some(home_dir)).unwrap(),
            &default,
        );
    }

    #[test]
    fn configured_url_routes_unix_and_http_forms() {
        // #6218: unix:// selects the socket route; the historical
        // http:// form stays TCP; an empty unix path is rejected.
        assert_eq!(
            route_from_configured_url("unix:///run/fleet/queue-broker.sock"),
            Ok(DirectRoute::Unix(std::path::PathBuf::from(
                "/run/fleet/queue-broker.sock"
            )))
        );
        assert_eq!(
            route_from_configured_url("http://100.99.113.103:4801"),
            Ok(DirectRoute::Tcp("http://100.99.113.103:4801".to_string()))
        );
        assert!(route_from_configured_url("unix://").is_err());
    }

    #[test]
    fn finish_direct_response_parses_status_and_body() {
        // #6218: both direct legs share this response handling.
        let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}";
        assert_eq!(
            finish_direct_response(ok).expect("200 parses"),
            json!({"ok": true})
        );
        assert!(finish_direct_response(b"no header split").is_err());
    }

    #[test]
    fn token_file_empty_env_values_use_home_default() {
        let home_dir = "/home/test";
        let default = format!("{home_dir}/.config/queue-broker/token");
        assert_env_resolution(
            |value| token_file_from_env(value, Some(home_dir)).unwrap(),
            &default,
        );
    }

    #[test]
    fn missing_or_empty_home_rejects_socket_token_and_signal_paths() {
        for home_dir in [None, Some("")] {
            assert_eq!(
                broker_sock_from_home(home_dir),
                Err(crate::home::HOME_REQUIRED)
            );
            assert_eq!(
                token_file_from_env(None, home_dir),
                Err(crate::home::HOME_REQUIRED)
            );
            assert_eq!(
                signal_path_from_env(None, home_dir),
                Err(crate::home::HOME_REQUIRED)
            );
        }
    }

    #[test]
    fn home_derived_socket_token_and_signal_paths_are_unchanged() {
        let home_dir = Some("/home/test");
        assert_eq!(
            broker_sock_from_home(home_dir),
            Ok("/home/test/.cache/laptop-agent/broker.sock".to_string())
        );
        assert_eq!(
            token_file_from_env(None, home_dir),
            Ok("/home/test/.config/queue-broker/token".to_string())
        );
        assert_eq!(
            signal_path_from_env(None, home_dir),
            Ok("/home/test/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json".to_string())
        );
    }

    #[test]
    fn extra_positional_names_the_first_unexpected_one() {
        // #5592: at or under the verb's arity there is nothing to reject.
        assert_eq!(extra_positional(&pos(&[]), 0), None);
        assert_eq!(extra_positional(&pos(&["a.json"]), 1), None);
        assert_eq!(extra_positional(&pos(&["123"]), 1), None);
        // Past it, the FIRST extra is the one named in the usage error:
        // validate/enqueue read pos[0] only, task verbs pos_i64(0) only,
        // and list/health/signal read none.
        assert_eq!(extra_positional(&pos(&["junk"]), 0), Some("junk"));
        assert_eq!(
            extra_positional(&pos(&["a.json", "b.json"]), 1),
            Some("b.json")
        );
        assert_eq!(extra_positional(&pos(&["123", "456"]), 1), Some("456"));
    }

    #[test]
    fn duplicate_value_flag_is_a_usage_error() {
        // #5939: last-wins used to silently retarget the invocation.
        let err = parse_args_inner(&pos(&["--queue", "a", "--queue", "b"]), &["queue"], &[])
            .err()
            .expect("duplicate --queue must be rejected");
        assert!(err.contains("--queue"), "error names the flag: {err}");
        // Inline and separate forms of the same flag also collide.
        assert!(parse_args_inner(&pos(&["--queue=a", "--queue", "b"]), &["queue"], &[]).is_err());
    }

    #[test]
    fn duplicate_bool_flag_is_a_usage_error() {
        let err = parse_args_inner(&pos(&["--dry-run", "--dry-run"]), &[], &["dry-run"])
            .err()
            .expect("duplicate --dry-run must be rejected");
        assert!(err.contains("--dry-run"), "error names the flag: {err}");
    }

    #[test]
    fn bool_flag_with_inline_value_is_a_usage_error() {
        // #5939: --dry-run=false used to be read as true.
        for raw in ["--dry-run=false", "--dry-run=true"] {
            let err = parse_args_inner(&pos(&[raw]), &[], &["dry-run"])
                .err()
                .expect("bool with inline value must be rejected");
            assert!(err.contains("--dry-run"), "error names the flag: {err}");
        }
        // A bare boolean keeps working.
        let args = parse_args_inner(&pos(&["--dry-run"]), &[], &["dry-run"])
            .ok()
            .expect("bare bool parses");
        assert!(args.has("dry-run"));
    }

    #[test]
    fn non_numeric_numeric_flag_is_a_usage_error() {
        // #5939: get_i64 used parse().ok(), so --limit abc silently
        // fell back to the caller's default.
        let err = parse_i64_flag("limit", "abc")
            .err()
            .expect("non-numeric --limit must be rejected");
        assert!(err.contains("--limit"), "error names the flag: {err}");
        assert!(err.contains("abc"), "error names the value: {err}");
        assert_eq!(parse_i64_flag("limit", "20"), Ok(20));
    }

    #[test]
    fn list_limit_range_is_validated() {
        // #6419: 0 silently reported no tasks; negatives and unbounded
        // values reached the broker verbatim. Out-of-range is a usage
        // error naming --limit (design #6448 Phase 3: reject, not clamp).
        for bad in [0, -1, LIST_LIMIT_MAX + 1, i64::MAX] {
            let err = validate_list_limit(bad)
                .err()
                .expect("out-of-range --limit must be rejected");
            assert!(err.contains("--limit"), "error names the flag: {err}");
        }
        // Boundaries and the ordinary value pass through unchanged.
        assert_eq!(validate_list_limit(1), Ok(1));
        assert_eq!(validate_list_limit(20), Ok(20));
        assert_eq!(validate_list_limit(LIST_LIMIT_MAX), Ok(LIST_LIMIT_MAX));
        // Absent --limit keeps the existing default of 20.
        assert_eq!(list_limit_or_default(None), Ok(20));
        assert_eq!(list_limit_or_default(Some(0)), validate_list_limit(0));
    }

    #[test]
    fn well_formed_invocation_is_unchanged() {
        // Control (#5939 acceptance): list --queue alpha records
        // queue=alpha and the absent --limit defaults at the call site.
        let args = parse_args_inner(&pos(&["--queue", "alpha"]), &["queue", "limit"], &[])
            .ok()
            .expect("well-formed invocation parses");
        assert_eq!(args.get("queue"), Some("alpha".to_string()));
        assert_eq!(args.get_i64("limit").unwrap_or(20), 20);
    }

    /// One spelling of a value flag: `--flag value` or `--flag=value`.
    fn flag_tokens(flag: &str, value: &str, inline: bool) -> Vec<String> {
        if inline {
            vec![format!("--{flag}={value}")]
        } else {
            vec![format!("--{flag}"), value.to_string()]
        }
    }

    // Property-based coverage of the #5939 parser invariants (the
    // oracle is the invariant itself; the fixed examples above stay
    // as smoke coverage). Per the repo testing doctrine, parsers get
    // PBT: a generator hammers spellings and positions no example
    // list would enumerate.

    proptest! {
        #[test]
        fn duplicate_flags_rejected_in_any_spelling_and_position(
            flag in prop_oneof![Just("queue"), Just("worker")],
            inline1 in any::<bool>(),
            inline2 in any::<bool>(),
            v1 in "[a-z]{1,4}",
            v2 in "[a-z]{1,4}",
            lead in proptest::collection::vec("[a-z]{1,3}", 0..3),
            mid in proptest::collection::vec("[a-z]{1,3}", 0..3),
        ) {
            let mut rest: Vec<String> = lead;
            rest.extend(flag_tokens(flag, &v1, inline1));
            rest.extend(mid);
            rest.extend(flag_tokens(flag, &v2, inline2));
            let err = parse_args_inner(&rest, &["queue", "worker"], &[])
                .err()
                .expect("a repeated flag must be rejected wherever it appears");
            prop_assert!(
                err.contains(&format!("--{flag}")),
                "error names the flag: {}",
                err
            );
        }

        #[test]
        fn bool_with_any_inline_value_rejected(
            v in prop_oneof![
                Just("true".to_string()),
                Just("false".to_string()),
                "[a-z]{0,5}",
                "[0-9]{1,3}",
            ],
        ) {
            let err = parse_args_inner(&pos(&[&format!("--dry-run={v}")]), &[], &["dry-run"])
                .err()
                .expect("a boolean with an inline value must be rejected");
            prop_assert!(err.contains("--dry-run"), "error names the flag: {}", err);
        }

        #[test]
        fn bare_bool_accepted_in_any_position(
            lead in proptest::collection::vec("[a-z]{1,3}", 0..3),
            tail in proptest::collection::vec("[a-z]{1,3}", 0..3),
        ) {
            let mut rest: Vec<String> = lead;
            rest.push("--dry-run".to_string());
            rest.extend(tail);
            let args = parse_args_inner(&rest, &["queue"], &["dry-run"])
                .ok()
                .expect("a bare boolean parses wherever it appears");
            prop_assert!(args.has("dry-run"));
        }

        #[test]
        fn numeric_flag_roundtrips_every_i64(n in any::<i64>()) {
            prop_assert_eq!(parse_i64_flag("limit", &n.to_string()), Ok(n));
        }

        #[test]
        fn non_numeric_flag_value_rejected_for_any_spelling(
            s in "[a-z][a-z0-9]{0,6}",
        ) {
            // A leading letter can never spell an i64, so the oracle
            // is decidable for every generated string.
            let err = parse_i64_flag("limit", &s)
                .err()
                .expect("a non-numeric value must be rejected");
            prop_assert!(err.contains("--limit"), "error names the flag: {}", err);
            prop_assert!(err.contains(&s), "error names the value: {}", err);
        }

        #[test]
        fn distinct_flags_all_preserved(
            use_queue in any::<bool>(),
            use_worker in any::<bool>(),
            use_lane in any::<bool>(),
            inline in any::<bool>(),
            vq in "[a-z]{1,4}",
            vw in "[a-z]{1,4}",
            vl in "[a-z]{1,4}",
        ) {
            let mut rest: Vec<String> = Vec::new();
            let mut expected: Vec<(&str, &str)> = Vec::new();
            if use_queue {
                rest.extend(flag_tokens("queue", &vq, inline));
                expected.push(("queue", &vq));
            }
            if use_worker {
                rest.extend(flag_tokens("worker", &vw, inline));
                expected.push(("worker", &vw));
            }
            if use_lane {
                rest.extend(flag_tokens("lane", &vl, inline));
                expected.push(("lane", &vl));
            }
            let args = parse_args_inner(&rest, &["queue", "worker", "lane"], &[])
                .ok()
                .expect("distinct flags are never rejected");
            for (name, value) in expected {
                prop_assert_eq!(args.get(name), Some(value.to_string()));
            }
        }
    }

    // #5486 regression coverage: the broker envelope -> CLI output
    // contract for claim, plus claimed-list recovery. Oracles are the
    // envelope/row shapes the broker actually returns (queue_broker
    // wraps RPC results as {ok:true, row:...} / {ok:true, rows:[...]}).

    fn claimed_row_fixture() -> Value {
        json!({
            "id": 10811,
            "queue": "repo-health-code-pilot",
            "status": "claimed",
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:5486",
            "title": "Add repo-queue claim response-shape regression tests (#5486)",
            "priority": 50,
            "worker_id": "repo-worker-code",
        })
    }

    #[test]
    fn claim_success_envelope_prints_the_real_row() {
        // The reported #5486 failure printed an all-null task row for
        // a claim the broker had actually made. The row must come
        // through verbatim: real id, queue, status, and dedupe_key.
        let row = claimed_row_fixture();
        let resp = json!({"ok": true, "row": row.clone()});
        let (out, code) = claim_output(&resp);
        assert_eq!(code, 0);
        assert_eq!(out, row);
        assert_eq!(out.get("id"), Some(&json!(10811)));
        assert_eq!(out.get("queue"), Some(&json!("repo-health-code-pilot")));
        assert_eq!(out.get("status"), Some(&json!("claimed")));
        assert_eq!(
            out.get("dedupe_key"),
            Some(&json!(
                "repo-health:v1:fix:developing-today/code:issue:5486"
            ))
        );
        for field in ["id", "queue", "status", "dedupe_key"] {
            assert!(
                !out.get(field).expect(field).is_null(),
                "claimed row field {field} must not be null"
            );
        }
    }

    #[test]
    fn claim_null_row_is_a_distinguishable_nonzero_no_claim() {
        // Both a null row and a missing row field are "no task
        // claimable": nonzero exit, and the marker carries none of
        // the fields that would make it look like a task row.
        for resp in [json!({"ok": true, "row": Value::Null}), json!({"ok": true})] {
            let (out, code) = claim_output(&resp);
            assert_eq!(code, NO_CLAIM_EXIT);
            assert_ne!(code, 0, "an empty claim must not exit as success");
            assert_eq!(out, json!({"task": Value::Null}));
            for field in ["id", "queue", "status", "dedupe_key"] {
                assert!(
                    out.get(field).is_none(),
                    "no-claim marker must not carry task field {field}"
                );
            }
        }
    }

    #[test]
    fn claim_payload_less_row_is_a_no_claim_not_a_task() {
        // #5444 defense in depth: a claimed row that CARRIES a
        // payload field which is null or an empty object is the
        // 2026-11-04 payload-less incident shape — it must degrade
        // to the distinguishable no-claim marker with a nonzero
        // exit, never print as a task a worker would try to route.
        for payload in [Value::Null, json!({})] {
            let mut row = claimed_row_fixture();
            row["payload"] = payload;
            let (out, code) = claim_output(&json!({"ok": true, "row": row}));
            assert_eq!(code, NO_CLAIM_EXIT);
            assert_eq!(out, json!({"task": Value::Null}));
        }
        // A row carrying a real payload object passes through.
        let mut row = claimed_row_fixture();
        row["payload"] = json!({"schema": "repo-task/v1", "kind": "small_fix"});
        let (out, code) = claim_output(&json!({"ok": true, "row": row.clone()}));
        assert_eq!(code, 0);
        assert_eq!(out, row);
    }

    #[test]
    fn claimed_list_recovers_the_claimed_task() {
        // After a successful claim, `list --status claimed` returns
        // rows the worker can recover the same task from, with its
        // identifying fields intact.
        let row = claimed_row_fixture();
        let other = json!({
            "id": 9999,
            "queue": "repo-health-code-pilot",
            "status": "claimed",
            "dedupe_key": "repo-health:v1:triage:developing-today/code:issue:1",
            "title": "some other claimed task",
            "priority": 30,
        });
        let rows = vec![other, row];
        let found = find_task_row(&rows, 10811).expect("claimed task must be recoverable by id");
        assert_eq!(found.get("status"), Some(&json!("claimed")));
        assert_eq!(
            found.get("dedupe_key"),
            Some(&json!(
                "repo-health:v1:fix:developing-today/code:issue:5486"
            ))
        );
        assert!(find_task_row(&rows, 424242).is_none());
        // The printed cells carry the same identifying fields.
        let cells = list_row_cells(found);
        assert_eq!(cells.len(), 6);
        assert_eq!(cells[0], "task 10811");
        assert_eq!(cells[1], "50");
        assert_eq!(cells[2], "claimed");
        assert_eq!(
            cells[4],
            "repo-health:v1:fix:developing-today/code:issue:5486"
        );
        assert_eq!(
            cells[5],
            "Add repo-queue claim response-shape regression tests (#5486)"
        );
    }

    proptest! {
        #[test]
        fn claim_output_passes_any_non_null_row_through_verbatim(
            id in 1i64..1_000_000,
            priority in 0i64..100,
            title in "[a-z ]{1,40}",
        ) {
            // Invariant (the #5486 oracle): whatever identifying
            // fields a successful claim row carries reach stdout
            // unchanged, with a success exit — no field is nulled,
            // dropped, or re-keyed in decoding.
            let row = json!({
                "id": id,
                "queue": "repo-health-code-pilot",
                "status": "claimed",
                "dedupe_key": format!("key-{id}"),
                "title": title,
                "priority": priority,
            });
            let (out, code) = claim_output(&json!({"ok": true, "row": row.clone()}));
            prop_assert_eq!(code, 0);
            prop_assert_eq!(out, row);
        }
    }

    #[test]
    fn broker_program_prefers_a_sibling_fleet_binary() {
        // #5758 acceptance: while a fleet binary exists beside the
        // running one, auto-start spawns `fleet proxy serve` —
        // never the Python broker.
        let dir =
            std::env::temp_dir().join(format!("fleet-queue-broker-program-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fleet"), b"#!/bin/sh\n").unwrap();
        let (program, args) = broker_program_in(&dir);
        assert_eq!(program, dir.join("fleet").to_string_lossy());
        assert_eq!(args, vec!["proxy".to_string(), "serve".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_program_falls_back_to_python_without_a_sibling() {
        // The standalone rollback builds (no fleet beside them)
        // keep the Python spawn.
        let dir = std::env::temp_dir().join(format!(
            "fleet-queue-broker-program-none-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (program, args) = broker_program_in(&dir);
        assert_eq!(program, "python3");
        assert_eq!(args.len(), 1);
        assert!(
            args[0].ends_with("/tools/laptop-agent-broker.py"),
            "{args:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
