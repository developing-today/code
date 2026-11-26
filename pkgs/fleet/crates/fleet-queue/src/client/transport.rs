//! broker/direct transport for the repo-queue client (split from client.rs, #8588).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::*;

pub(super) const QUEUE_BROKER_PORT: u64 = 4801;
pub(super) const DEFAULT_QUEUE: &str = "repo-health-code-shadow";
pub(super) const DEFAULT_WORKER: &str = "repo-worker-code";
pub(super) const TELEMETRY_FIELDS: [&str; 4] = ["pool", "model", "effort", "cost_usd"];
pub(super) const TERMINAL_STATUSES: [&str; 3] = ["done", "cancelled", "failed"];

pub(super) fn home() -> String {
    crate::home::from_env().unwrap_or_else(|error| die(3, error))
}

pub(super) fn eprint(s: &str) {
    eprintln!("{s}");
}

pub(super) fn die(code: i32, msg: &str) -> ! {
    eprintln!("repo-queue: {msg}");
    exit(code);
}

pub(super) fn print_json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".to_string())
    );
}

// ---------------------------------------------------------------------------
// Session-proxy transport
// ---------------------------------------------------------------------------

pub(super) fn broker_sock() -> String {
    // #6218: ask fleet-config instead of hardcoding the path --
    // env beats the config file beats the compiled-in default,
    // which is the same path broker_sock_from_home builds. A
    // config load error falls back to the home-anchored default.
    match fleet_config::Config::load() {
        Ok(cfg) => cfg.broker_socket().to_string_lossy().into_owned(),
        Err(_) => broker_sock_from_home(Some(&home())).unwrap_or_else(|error| die(3, error)),
    }
}

pub(super) fn broker_sock_from_home(home_dir: Option<&str>) -> Result<String, &'static str> {
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

pub(super) fn direct_broker_url() -> Option<String> {
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
pub(super) enum DirectRoute {
    Tcp(String),
    Unix(std::path::PathBuf),
}

/// Parse a configured broker URL into a route: `unix://<path>`
/// selects the socket; anything else keeps the historical
/// `http://host:port` TCP reading (validated in the TCP leg).
pub(super) fn route_from_configured_url(url: &str) -> Result<DirectRoute, String> {
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
pub(super) fn direct_route() -> Option<DirectRoute> {
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

pub(super) fn direct_token() -> Result<String, String> {
    let token_file_env = std::env::var("REPO_QUEUE_TOKEN_FILE").ok();
    let home_env = std::env::var("HOME").ok();
    let token_file = token_file_from_env(token_file_env.as_deref(), home_env.as_deref())
        .unwrap_or_else(|error| die(3, error));
    let token = crate::token_file::load_token_file(&token_file)?;
    Ok(token.trim().to_string())
}

/// Decode one complete HTTP chunked body. The direct transports read
/// through connection close, so an incomplete chunk is a definite
/// truncation rather than a request for more bytes.
pub(super) fn decode_chunked_body(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| {
                "queue-broker response truncated inside a chunk-size line".to_string()
            })?;
        let line = std::str::from_utf8(&rest[..line_end])
            .map_err(|_| "queue-broker response has a non-ASCII chunk-size line".to_string())?;
        let size_field = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_field, 16).map_err(|_| {
            format!("queue-broker response has malformed chunk size {size_field:?}")
        })?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            loop {
                if rest.starts_with(b"\r\n") {
                    rest = &rest[2..];
                    if !rest.is_empty() {
                        return Err(
                            "queue-broker response has bytes after the terminal chunk".into()
                        );
                    }
                    return Ok(decoded);
                }
                let trailer_end = rest
                    .windows(2)
                    .position(|window| window == b"\r\n")
                    .ok_or_else(|| {
                        "queue-broker response truncated inside chunk trailers".to_string()
                    })?;
                if !rest[..trailer_end].contains(&b':') {
                    return Err("queue-broker response has a malformed chunk trailer".into());
                }
                rest = &rest[trailer_end + 2..];
            }
        }
        let chunk_end = size
            .checked_add(2)
            .ok_or_else(|| "queue-broker response chunk size overflow".to_string())?;
        if rest.len() < chunk_end {
            return Err("queue-broker response truncated inside a chunk".into());
        }
        if &rest[size..chunk_end] != b"\r\n" {
            return Err("queue-broker response chunk is missing its CRLF terminator".into());
        }
        decoded.extend_from_slice(&rest[..size]);
        rest = &rest[chunk_end..];
    }
}

/// The response handling both direct legs share: status line,
/// transfer framing, JSON body, 401 and >=400 die with exit 3 exactly
/// as before. A malformed successful JSON body is never converted to
/// `Value::Null`: callers must not mistake it for an empty result.
pub(super) fn finish_direct_response(raw: &[u8]) -> Result<Value, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("queue-broker bad reply")?;
    let head_txt = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u64 = head_txt
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| "queue-broker answered with a malformed status line".to_string())?;
    let mut transfer_codings = Vec::new();
    let mut content_length = None;
    for line in head_txt.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("transfer-encoding") {
                transfer_codings.extend(
                    value
                        .split(',')
                        .map(|coding| coding.trim().to_ascii_lowercase())
                        .filter(|coding| !coding.is_empty()),
                );
            } else if name.eq_ignore_ascii_case("content-length") {
                for part in value.split(',') {
                    let length: usize = part.trim().parse().map_err(|_| {
                        format!("queue-broker response has malformed Content-Length {value:?}")
                    })?;
                    match content_length {
                        None => content_length = Some(length),
                        Some(previous) if previous == length => {}
                        Some(previous) => {
                            return Err(format!(
                                "queue-broker response has conflicting Content-Length values {previous} and {length}"
                            ));
                        }
                    }
                }
            }
        }
    }
    let wire_body = &raw[split + 4..];
    let decoded_body = match (content_length, transfer_codings.as_slice()) {
        (Some(expected), []) => {
            if wire_body.len() != expected {
                return Err(format!(
                    "queue-broker response truncated: Content-Length {expected}, received {} body bytes",
                    wire_body.len()
                ));
            }
            wire_body.to_vec()
        }
        (None, []) => wire_body.to_vec(),
        (None, [coding]) if coding == "chunked" => decode_chunked_body(wire_body)?,
        (Some(_), _) => {
            return Err(
                "queue-broker response has both Content-Length and Transfer-Encoding".into(),
            );
        }
        _ => {
            return Err(format!(
                "queue-broker response has unsupported Transfer-Encoding: {}",
                transfer_codings.join(", ")
            ));
        }
    };
    let parsed_body: Result<Value, _> = serde_json::from_slice(&decoded_body);
    if status == 401 {
        die(3, "queue-broker rejected the client token (HTTP 401)");
    }
    if status >= 400 {
        let resp_body: Value = parsed_body.unwrap_or(Value::Null);
        let msg = resp_body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("queue-broker error")
            .to_string();
        die(3, &msg);
    }
    parsed_body.map_err(|error| {
        format!("queue-broker answered HTTP {status} with invalid JSON body: {error}")
    })
}

/// #14188: the caller's FLEET_CTX_* record as raw `X-Fleet-Ctx-*`
/// header lines for the hand-written direct legs (empty when there
/// is no context — absence is representable, never an error).
fn ctx_header_lines() -> String {
    fleet_config::ctx::Ctx::from_env().header_lines()
}

/// #14188: the same record on a session-proxy frame (`ctx`), which
/// the proxy forwards as `X-Fleet-Ctx-*` headers. Omitted when empty.
fn with_ctx_frame(mut frame: Value) -> Value {
    if let (Some(obj), Some(ctx)) = (
        frame.as_object_mut(),
        fleet_config::ctx::Ctx::from_env().to_frame(),
    ) {
        obj.insert("ctx".to_string(), json!(ctx));
    }
    frame
}

/// The host part of a `host:port` broker address — the name
/// the broker's BT-1 leaf must carry as a SAN (its tailnet IP
/// or its `.internal` DNS name) for TLS validation (BT-3).
fn broker_host(hostport: &str) -> &str {
    let host = hostport
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(hostport);
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

/// One direct broker request head. `trust_bundle_version` is the
/// client-side trust-store acknowledgement BT-2 needs the broker to see
/// on its health surface; `None` means this request used plaintext,
/// UDS, or the named break-glass posture and must not claim validation.
fn direct_request_head(
    path: &str,
    host: &str,
    token: &str,
    ctx: &str,
    content_length: usize,
    trust_bundle_version: Option<u32>,
) -> String {
    let trust_bundle_version = trust_bundle_version
        .map(|version| format!("X-Fleet-Trust-Bundle-Version: {version}\r\n"))
        .unwrap_or_default();
    format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nX-Agent-Token: {token}\r\n{trust_bundle_version}{ctx}Content-Type: application/json\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    )
}

pub(super) fn direct_tcp_roundtrip(url: &str, path: &str, body: &Value) -> Result<Value, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("broker URL must be http://host:port")?;
    let hostport = rest.trim_end_matches('/');
    if hostport.is_empty() || hostport.contains('/') {
        return Err(format!("bad broker URL {url:?}"));
    }
    let token = direct_token()?;
    let payload = serde_json::to_vec(body).unwrap();
    let s = TcpStream::connect(hostport)
        .map_err(|e| format!("cannot connect to queue-broker at {hostport}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(300))).ok();
    s.set_write_timeout(Some(Duration::from_secs(60))).ok();
    // BT-3 (#14401): with a trust store configured the leg is
    // TLS and the broker leaf is validated, fail closed; with
    // neither knob set this is the Phase 0 plaintext stream.
    let mode = crate::broker_tls::BrokerTlsClient::from_env()?;
    let trust_bundle_version = mode.bundle_version();
    let mut s = mode.wrap(s, broker_host(hostport))?;
    let ctx = ctx_header_lines();
    let head = direct_request_head(
        path,
        hostport,
        &token,
        &ctx,
        payload.len(),
        trust_bundle_version,
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
pub(super) fn direct_uds_roundtrip(
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
    let ctx = ctx_header_lines();
    let head = direct_request_head(path, "localhost", &token, &ctx, payload.len(), None);
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("queue-broker write failed: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("queue-broker read failed: {e}"))?;
    finish_direct_response(&raw)
}

pub(super) fn broker_roundtrip(path: &str, body: &Value) -> Result<Value, String> {
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
    let payload = with_ctx_frame(payload);
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

pub(super) fn unix_roundtrip(data: &[u8]) -> Result<Value, String> {
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

/// #13146: the bounded, never-dying sibling of [`broker_roundtrip`]
/// for best-effort callers (the placement session mirror). Same
/// route selection, but: every timeout is the caller's budget, HTTP
/// error statuses come back as Err instead of dying with exit 3,
/// a malformed configured broker URL degrades to Err instead of
/// dying, and the session broker is never auto-spawned — a caller
/// that can proceed without the broker must never be able to fail
/// or stall on it.
pub(crate) fn broker_roundtrip_bounded(
    path: &str,
    body: &Value,
    budget: Duration,
) -> Result<Value, String> {
    if let Some(url) = direct_broker_url() {
        return match route_from_configured_url(&url)? {
            DirectRoute::Tcp(u) => bounded_tcp_roundtrip(&u, path, body, budget),
            DirectRoute::Unix(s) => bounded_uds_roundtrip(&s, path, body, budget),
        };
    }
    let cfg = fleet_config::Config::load().unwrap_or_default();
    if let Some(fleet_config::RoleTransport::Socket(socket)) =
        cfg.role_transport_choice("queue-broker")
    {
        // Mirror broker_roundtrip: a failed socket attempt falls
        // back to the session-proxy route within the same call.
        if let Ok(v) = bounded_uds_roundtrip(&socket, path, body, budget) {
            return Ok(v);
        }
    }
    bounded_proxy_roundtrip(path, body, budget)
}

/// The direct HTTP exchange with caller-bounded timeouts, shared
/// by the bounded TCP and UDS legs.
fn bounded_direct_exchange<S: Read + Write>(
    mut s: S,
    host: &str,
    path: &str,
    body: &Value,
    token: &str,
    trust_bundle_version: Option<u32>,
) -> Result<Value, String> {
    let payload = serde_json::to_vec(body).unwrap();
    let ctx = ctx_header_lines();
    let head = direct_request_head(
        path,
        host,
        token,
        &ctx,
        payload.len(),
        trust_bundle_version,
    );
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("queue-broker write failed: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("queue-broker read failed: {e}"))?;
    finish_bounded_response(&raw)
}

/// Status handling for the bounded legs: an HTTP error status is
/// an Err here, never a die — [`finish_direct_response`] (which
/// dies on >= 400) only ever sees success statuses.
fn finish_bounded_response(raw: &[u8]) -> Result<Value, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("queue-broker bad reply")?;
    let head_txt = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u64 = head_txt
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| "queue-broker answered with a malformed status line".to_string())?;
    if status >= 400 {
        return Err(format!("queue-broker answered HTTP {status}"));
    }
    finish_direct_response(raw)
}

fn bounded_tcp_roundtrip(
    url: &str,
    path: &str,
    body: &Value,
    budget: Duration,
) -> Result<Value, String> {
    use std::net::ToSocketAddrs as _;
    let rest = url
        .strip_prefix("http://")
        .ok_or("broker URL must be http://host:port")?;
    let hostport = rest.trim_end_matches('/');
    if hostport.is_empty() || hostport.contains('/') {
        return Err(format!("bad broker URL {url:?}"));
    }
    let token = direct_token()?;
    let addr = hostport
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve queue-broker at {hostport}: {e}"))?
        .next()
        .ok_or_else(|| format!("cannot resolve queue-broker at {hostport}"))?;
    let s = TcpStream::connect_timeout(&addr, budget)
        .map_err(|e| format!("cannot connect to queue-broker at {hostport}: {e}"))?;
    s.set_read_timeout(Some(budget)).ok();
    s.set_write_timeout(Some(budget)).ok();
    // BT-3 (#14401): same TLS posture as the unbounded leg.
    let mode = crate::broker_tls::BrokerTlsClient::from_env()?;
    let trust_bundle_version = mode.bundle_version();
    let s = mode.wrap(s, broker_host(hostport))?;
    bounded_direct_exchange(s, hostport, path, body, &token, trust_bundle_version)
}

fn bounded_uds_roundtrip(
    socket: &std::path::Path,
    path: &str,
    body: &Value,
    budget: Duration,
) -> Result<Value, String> {
    let token = direct_token()?;
    let mut s = UnixStream::connect(socket).map_err(|e| {
        format!(
            "cannot connect to queue-broker at unix://{}: {e}",
            socket.display()
        )
    })?;
    s.set_read_timeout(Some(budget)).ok();
    s.set_write_timeout(Some(budget)).ok();
    bounded_direct_exchange(&mut s, "localhost", path, body, &token, None)
}

/// The session-proxy leg, bounded: one attempt, no broker spawn,
/// and a proxy HTTP error is an Err (the unbounded leg dies on it).
fn bounded_proxy_roundtrip(path: &str, body: &Value, budget: Duration) -> Result<Value, String> {
    let payload = json!({
        "method": "POST",
        "path": path,
        "body": body,
        "port": QUEUE_BROKER_PORT,
        "token_file": format!("{}/tools/queue-broker-token", home()),
    });
    let payload = with_ctx_frame(payload);
    let data = serde_json::to_vec(&payload).unwrap();
    let mut s = UnixStream::connect(broker_sock())
        .map_err(|e| format!("cannot connect to session proxy: {e}"))?;
    s.set_read_timeout(Some(budget)).ok();
    s.set_write_timeout(Some(budget)).ok();
    s.write_all(&(data.len() as u64).to_be_bytes())
        .and_then(|_| s.write_all(&data))
        .map_err(|e| format!("session proxy write failed: {e}"))?;
    let mut hdr = [0u8; 8];
    s.read_exact(&mut hdr)
        .map_err(|e| format!("session proxy read failed: {e}"))?;
    let n = u64::from_be_bytes(hdr) as usize;
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)
        .map_err(|e| format!("session proxy read failed: {e}"))?;
    let resp: Value =
        serde_json::from_slice(&buf).map_err(|e| format!("session proxy bad reply: {e}"))?;
    let status = resp.get("status").and_then(|s| s.as_u64()).unwrap_or(0);
    let resp_body = resp.get("body").cloned().unwrap_or(Value::Null);
    if status >= 400 {
        let msg = resp_body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("session proxy error")
            .to_string();
        return Err(msg);
    }
    Ok(resp_body)
}

/// Which broker program to auto-start: a sibling `fleet` binary
/// next to the running one wins (step 3, #5431 — `fleet proxy
/// serve` is the Python broker's successor); the standalone
/// broker builds from the fleet-brokers rollback package have no
/// fleet beside them and keep the Python spawn below.
pub(super) fn broker_program() -> (String, Vec<String>) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return broker_program_in(dir);
        }
    }
    python_broker_program()
}

pub(super) fn python_broker_program() -> (String, Vec<String>) {
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
pub(super) fn broker_program_in(dir: &std::path::Path) -> (String, Vec<String>) {
    let fleet = dir.join("fleet");
    if fleet.exists() {
        return (
            fleet.to_string_lossy().into_owned(),
            vec!["proxy".to_string(), "serve".to_string()],
        );
    }
    python_broker_program()
}

pub(super) fn spawn_broker() {
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
pub(super) fn call_result(path: &str, body: &Value) -> Result<Value, (i32, String)> {
    // #6450 posture (crate::backoff — the one shared policy): a
    // broker envelope carrying a 57014 / PGRST002-class error is
    // retried with exponential backoff + jitter instead of dying on
    // the caller. Safe for writes: a 57014 means Postgres cancelled
    // the statement, so the write rolled back and cannot
    // double-apply (enqueue dedupe keys are a second belt).
    // /claim-flow invariant (#5757-A, Codex P1 on PR #8493): this
    // whole-request retry is safe ONLY because both brokers answer
    // any post-claim-commit lane/bind failure as a partial SUCCESS
    // (ok envelope + `claim_flow_partial`), never as a retryable
    // error — a retryable error out of /claim-flow can therefore
    // only come from the claim step itself, whose statement rolled
    // back. Do not weaken the broker side of this invariant.
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

pub(super) fn call(path: &str, body: &Value) -> Value {
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

pub(super) const FALLBACK_SWITCH_EXIT: i32 = 6;

pub(super) fn fallback_switch_enabled() -> bool {
    std::env::var("REPO_QUEUE_NO_FALLBACK_SWITCH")
        .map(|v| v != "1")
        .unwrap_or(true)
}

pub(super) fn fallback_switch_json(verb: &str, code: i32, msg: &str) -> Value {
    // #8581: the switch IS the db-degraded route — carry the
    // contract's mode line (selected through route_for from the
    // store-down legs) so a worker loop reads the mode and route
    // from the same envelope it already branches on.
    json!({
        "ok": false,
        "fallback": true,
        "switch_to": "github-fallback",
        "verb": verb,
        "mode_line": mode_line_for(verb, legs_store_down()).json_value(),
        "broker_exit": code,
        "error": msg,
        "hint": "continue this work with: fleet queue fallback scan|claim|heartbeat|release|unblock",
    })
}

pub(super) fn fallback_switch(verb: &str, code: i32, msg: &str) -> ! {
    eprint(&format!(
        "repo-queue: primary queue store failed on {verb} (exit {code}): {msg};          switching this work to the GitHub fallback"
    ));
    print_json(&fallback_switch_json(verb, code, msg));
    exit(FALLBACK_SWITCH_EXIT);
}

pub(super) fn call_fb(path: &str, body: &Value) -> Value {
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

pub(super) fn unwrap_ok_fb(resp: Value, verb: &str) -> Value {
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
pub(super) fn unwrap_ok(resp: Value) -> Value {
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

pub(super) fn require_row(resp: &Value, fn_name: &str) -> Value {
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

pub(super) fn row_or_ledger(resp: &Value, fn_name: &str, verb: &str) {
    let row = served_row(require_row(resp, fn_name), verb);
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

#[cfg(test)]
mod bounded_tests {
    use super::*;

    #[test]
    fn bounded_exchange_times_out_on_a_silent_peer() {
        // Oracle (#13146): the bounded leg gives up within its
        // budget when the peer accepts but never answers — the
        // unbounded legs would wait out their 300s read timeout,
        // and the session-proxy leg would die(3) on an error.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (_sock, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_millis(300))).ok();
        s.set_write_timeout(Some(Duration::from_millis(300))).ok();
        let started = std::time::Instant::now();
        let out = bounded_direct_exchange(
            &mut s,
            "127.0.0.1",
            "/placement-get",
            &json!({"handle": "fp_x"}),
            "test-token",
            None,
        );
        assert!(out.is_err(), "{out:?}");
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "bounded leg took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bounded_response_maps_http_error_to_err_not_death() {
        // Oracle: an HTTP error status is data (Err) for the
        // bounded caller; a success status parses as usual.
        let raw = b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\n\
                    Content-Length: 24\r\n\r\n{\"error\": \"unauthorized\"}";
        let out = finish_bounded_response(raw);
        assert!(matches!(out, Err(ref e) if e.contains("401")), "{out:?}");
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                    Content-Length: 7\r\n\r\n{\"a\":1}";
        let out = finish_bounded_response(raw).unwrap();
        assert_eq!(out, json!({"a": 1}));
    }

    #[test]
    fn direct_request_presents_the_validated_trust_bundle_version() {
        // Oracle: the serialized request carries exactly the verified
        // trust-bundle version the broker must acknowledge for BT-2.
        let head = direct_request_head(
            "/placement-get",
            "queue-broker.internal:4801",
            "test-token",
            "",
            0,
            Some(17),
        );
        let versions: Vec<_> = head
            .lines()
            .filter(|line| line.starts_with("X-Fleet-Trust-Bundle-Version:"))
            .collect();
        assert_eq!(versions, vec!["X-Fleet-Trust-Bundle-Version: 17"]);
    }

    #[test]
    fn direct_request_without_a_trust_store_makes_no_version_claim() {
        // Oracle: plaintext, UDS and break-glass requests have no
        // validated trust bundle and therefore emit no version header.
        let head = direct_request_head(
            "/placement-get",
            "queue-broker.internal:4801",
            "test-token",
            "",
            0,
            None,
        );
        assert!(!head.contains("X-Fleet-Trust-Bundle-Version:"));
    }
}
