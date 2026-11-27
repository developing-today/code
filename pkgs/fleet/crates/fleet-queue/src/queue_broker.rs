//! queue-broker — the repo-health queue as a resident laptop service
//! (code#5362). Sandbox clients reach it only through the session
//! proxy.
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
//!      QUEUE_BROKER_SECRETS_TOKEN_FILE (default ~/.config/secrets-broker/token),
//!      QUEUE_BROKER_DOOR_URL + QUEUE_BROKER_DOOR_TOKEN_FILE (optional,
//!      read-only hosted-door probe at POST /door-health).
//!
//! Backend: the queue core on the dedicated Turso DB `fleet-queue`
//! (crate::turso_queue), at QUEUE_BROKER_TURSO_URL with the token
//! from QUEUE_BROKER_TURSO_TOKEN_FILE or the secrets broker. Turso is
//! the only backend: QUEUE_BROKER_BACKEND may be unset or `turso`;
//! any other value is a startup config error (exit 3).

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fleet_secrets::{Resolver, SecretError, SecretSource, SecretValue};
use serde_json::{Value, json};

struct State {
    secrets_url: String,
    secrets_token: String,
    agent: ureq::Agent,
    /// Optional, read-only hosted-door probe (#14987). Queue traffic
    /// continues to use Turso until a later cutover explicitly routes
    /// verbs through this client.
    hosted_door: Option<HostedQueueDoor>,
    /// Filled by the startup thread once the Turso token resolves and
    /// the schema is ensured (#12247); until then queue verbs answer
    /// exit 3 with `turso_pending`.
    turso: std::sync::OnceLock<crate::turso_queue::TursoBackend>,
    /// Why the backend is not ready yet: the latest startup failure.
    turso_pending: Mutex<String>,
    /// Last trust-bundle observations by direct-TCP peer IP and presented
    /// version. Keeping versions separate prevents a newer process on the
    /// same host from hiding an older process during rollover. `None` records
    /// an authenticated TLS request that made no valid version claim.
    trust_bundle_clients: Mutex<BTreeMap<(IpAddr, Option<u32>), TrustBundleClientReport>>,
    /// #13765: the coordinator sweep must never overlap itself — a
    /// slow GitHub pass skips instead of piling a second grant loop
    /// onto the same counters.
    coord_sweep_running: Mutex<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrustBundleClientReport {
    last_seen_unix: u64,
}

/// Authenticated client for the hosted queue door. This first slice
/// exposes only a diagnostic probe; it deliberately does not switch
/// the broker's authoritative store or forward queue writes.
struct HostedQueueDoor {
    base_url: String,
    token: String,
    agent: ureq::Agent,
}

impl HostedQueueDoor {
    fn from_env(agent: ureq::Agent) -> Result<Option<Self>, String> {
        let url = std::env::var("QUEUE_BROKER_DOOR_URL").ok();
        let token_file = std::env::var("QUEUE_BROKER_DOOR_TOKEN_FILE").ok();
        match (url, token_file) {
            (None, None) => Ok(None),
            (Some(url), Some(token_file)) => {
                let url = url.trim().trim_end_matches('/');
                if !url.starts_with("https://") || url.len() <= "https://".len() {
                    return Err("QUEUE_BROKER_DOOR_URL must be an HTTPS origin".into());
                }
                let token = crate::token_file::load_token_file(&token_file)?;
                Ok(Some(Self {
                    base_url: url.to_string(),
                    token,
                    agent,
                }))
            }
            (Some(_), None) => {
                Err("QUEUE_BROKER_DOOR_URL requires QUEUE_BROKER_DOOR_TOKEN_FILE".into())
            }
            (None, Some(_)) => {
                Err("QUEUE_BROKER_DOOR_TOKEN_FILE requires QUEUE_BROKER_DOOR_URL".into())
            }
        }
    }

    fn health(&self, request: &Value) -> Result<Value, String> {
        let url = format!("{}/v1/queue/health", self.base_url);
        let mut response = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .send_json(request.clone())
            .map_err(|e| format!("hosted queue door health request failed: {e}"))?;
        response
            .body_mut()
            .read_json::<Value>()
            .map_err(|e| format!("hosted queue door health response was invalid JSON: {e}"))
    }
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

fn trim(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
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
/// of `dispatch`: the broker is the trust boundary for non-CLI
/// callers, and the Turso backend dispatches verbatim.
fn validate_durations(path: &str, req: &Value) -> Result<(), String> {
    match path {
        "/claim" | "/claim-flow" => lease_seconds(req).map(|_| ()),
        "/heartbeat" => extend_seconds(req).map(|_| ()),
        "/park" => delay_seconds(req).map(|_| ()),
        "/reprioritize" => crate::validate::resolve_priority(
            req.get("priority").unwrap_or(&Value::Null),
            "priority",
        )
        .map(|_| ()),
        _ => Ok(()),
    }
}

/// The not_before gate (#6426): /park and /revive carry a raw
/// not_before that dispatch forwards to the RPC (or the turso
/// backend), and /list carries one as a filter (#9027 — unvalidated,
/// a garbage value silently narrowed the Turso listing) — validate
/// it BEFORE either, with the same rule the
/// packet validator applies on /enqueue (validate_packet runs
/// inside both /enqueue handlers ahead of their RPC call). An
/// absent or null not_before is untouched; a present one must be a
/// non-empty RFC 3339 timestamp no more than 366 days out.
fn validate_request_not_before(path: &str, req: &Value) -> Result<(), String> {
    if !matches!(path, "/park" | "/revive" | "/list") {
        return Ok(());
    }
    match req.get("not_before").filter(|v| !v.is_null()) {
        Some(nb) => crate::validate::validate_not_before(nb, "not_before"),
        None => Ok(()),
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
        self.get_json_within(path, None)
    }

    /// get_json with an optional per-request timeout (#10626: the
    /// claim-time staleness lookup must never queue a claim behind
    /// a slow GitHub).
    fn get_json_within(
        &self,
        path: &str,
        timeout: Option<std::time::Duration>,
    ) -> Result<Value, crate::turso_queue::DepObs> {
        use crate::turso_queue::DepObs;
        let tok = self.token().map_err(DepObs::Failed)?;
        let url = format!("https://api.github.com{path}");
        let mut req = self.agent.get(&url);
        if let Some(limit) = timeout {
            req = req.config().timeout_global(Some(limit)).build();
        }
        let mut resp = match req
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

    fn probe_landing(
        &self,
        kind: crate::turso_queue::LinkKind,
        repo: &str,
        number: i64,
    ) -> crate::turso_queue::Landing {
        use crate::turso_queue::{Landing, LinkKind};
        let path = match kind {
            LinkKind::Issue => format!("/repos/{repo}/issues/{number}"),
            LinkKind::Pr => format!("/repos/{repo}/pulls/{number}"),
        };
        let v =
            match self.get_json_within(&path, Some(crate::turso_queue::STALENESS_LOOKUP_TIMEOUT)) {
                Ok(v) => v,
                Err(o) => return Landing::Unknown(format!("{o:?}")),
            };
        landing_from_json(kind, &v)
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

/// #13765: the production coordinator GitHub surface — the
/// broker's live load/run reads for the placement sweep, over the
/// same broker-owned token as the dep probe. Every read carries a
/// bounded per-request timeout (the staleness-lookup budget), so a
/// slow GitHub never queues a broker request behind the sweep.
struct BrokerCoordGh {
    secrets_url: String,
    secrets_token: String,
    agent: ureq::Agent,
    token: Mutex<Option<(String, std::time::Instant)>>,
    /// One sweep pass's responses by path: the run counts and the
    /// live run ids come from the same per_page=100 list read, so a
    /// pass costs 2 reads per repo instead of 4.
    pass: Mutex<std::collections::HashMap<String, Value>>,
}

/// ETag cache for the coordinator's GitHub reads, kept across sweep
/// passes: a repeat read sends If-None-Match, and GitHub's 304 answer
/// does not count against the token's hourly REST limit. Without it
/// the 20-second sweep spent ~2,900 of the shared 5,000 calls an hour.
fn coord_etag_cache() -> &'static Mutex<std::collections::HashMap<String, (String, Value)>> {
    static CACHE: std::sync::OnceLock<Mutex<std::collections::HashMap<String, (String, Value)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

impl BrokerCoordGh {
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

    fn get_json(&self, path: &str) -> Result<Value, String> {
        if let Some(v) = self
            .pass
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(path)
        {
            return Ok(v.clone());
        }
        let v = self.fetch_json(path)?;
        self.pass
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(path.to_string(), v.clone());
        Ok(v)
    }

    fn fetch_json(&self, path: &str) -> Result<Value, String> {
        use std::time::Duration as D;
        let tok = self.token()?;
        let url = format!("https://api.github.com{path}");
        let cached = coord_etag_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(path)
            .cloned();
        let req = self
            .agent
            .get(&url)
            .config()
            .timeout_global(Some(
                crate::turso_queue::STALENESS_LOOKUP_TIMEOUT.max(D::from_secs(2)),
            ))
            .build();
        let mut req = req
            .header("Authorization", &format!("Bearer {tok}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "fleet-queue-broker");
        if let Some((etag, _)) = &cached {
            req = req.header("If-None-Match", etag);
        }
        let mut resp = req
            .call()
            .map_err(|e| format!("github request failed for {path}: {e}"))?;
        let status = resp.status().as_u16();
        if status == 304 {
            if let Some((_, body)) = cached {
                return Ok(body);
            }
            return Err(format!("github 304 without a cached body for {path}"));
        }
        if status == 404 {
            return Err(format!("github 404 for {path}"));
        }
        if !(200..300).contains(&status) {
            return Err(format!("github http {status} for {path}"));
        }
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_string);
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("github response unreadable: {e}"))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("github response not json: {e}"))?;
        if let Some(etag) = etag {
            let mut cache = coord_etag_cache().lock().unwrap_or_else(|p| p.into_inner());
            // Bounded: run-detail paths come and go; the list reads
            // are a fixed handful per runner repo.
            if cache.len() > 512 {
                cache.clear();
            }
            cache.insert(path.to_string(), (etag, v.clone()));
        }
        Ok(v)
    }

    fn total_for(&self, repo: &str, status: &str) -> Result<i64, String> {
        // Same path as ids_for: total_count rides on the per_page=100
        // list, so this is answered from the pass cache.
        let v = self.get_json(&format!(
            "/repos/{repo}/actions/runs?status={status}&per_page=100"
        ))?;
        v.get("total_count")
            .and_then(|t| t.as_i64())
            .ok_or_else(|| format!("{repo} ({status}): HTTP 200 without total_count"))
    }

    fn ids_for(&self, repo: &str, status: &str) -> Result<Vec<i64>, String> {
        let v = self.get_json(&format!(
            "/repos/{repo}/actions/runs?status={status}&per_page=100"
        ))?;
        let runs = v
            .get("workflow_runs")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(runs
            .iter()
            .filter_map(|r| r.get("id").and_then(|i| i.as_i64()))
            .collect())
    }
}

impl crate::turso_queue::CoordGh for BrokerCoordGh {
    fn load_counts(&self, repo: &str) -> Result<(i64, i64), String> {
        Ok((
            self.total_for(repo, "in_progress")?,
            self.total_for(repo, "queued")?,
        ))
    }

    fn live_run_ids(&self, repo: &str) -> Result<Vec<i64>, String> {
        let mut ids = self.ids_for(repo, "in_progress")?;
        ids.extend(self.ids_for(repo, "queued")?);
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    fn run_actual_minutes(&self, repo: &str, run_id: i64) -> Result<Option<f64>, String> {
        let v = self.get_json(&format!("/repos/{repo}/actions/runs/{run_id}"))?;
        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if status != "completed" {
            return Ok(None);
        }
        let (Some(start), Some(end)) = (
            v.get("run_started_at").and_then(|s| s.as_str()),
            v.get("updated_at").and_then(|s| s.as_str()),
        ) else {
            return Ok(None);
        };
        // Whole minutes rounded up, at least one (the sweep's
        // shared helper); malformed stamps leave the claim alone.
        Ok(crate::turso_queue::coord_minutes_between(start, end))
    }
}

/// #10626: a GitHub issue or pull JSON as a staleness landing. An
/// issue landed when it closed (any reason: no work remains); a PR
/// landed only when it merged — a PR closed unmerged leaves the work
/// open. A terminal state without its timestamp infers nothing.
pub(crate) fn landing_from_json(
    kind: crate::turso_queue::LinkKind,
    v: &Value,
) -> crate::turso_queue::Landing {
    use crate::turso_queue::{Landing, LinkKind};
    let field = |k: &str| v.get(k).and_then(|s| s.as_str()).unwrap_or("").to_string();
    match kind {
        LinkKind::Issue => match field("state").as_str() {
            "closed" => match field("closed_at") {
                at if at.is_empty() => {
                    Landing::Unknown("closed issue with no closed_at".to_string())
                }
                at => Landing::Landed {
                    state: match field("state_reason") {
                        r if r.is_empty() => "closed".to_string(),
                        r => format!("closed ({r})"),
                    },
                    at,
                },
            },
            "open" => Landing::Open,
            other => Landing::Unknown(format!("issue state {other:?}")),
        },
        LinkKind::Pr => match field("merged_at") {
            at if !at.is_empty() => Landing::Landed {
                state: "merged".to_string(),
                at,
            },
            _ if matches!(field("state").as_str(), "open" | "closed") => Landing::Open,
            _ => Landing::Unknown(format!("pr state {:?}", field("state"))),
        },
    }
}

/// #8568 (PM-stack notifications Phase 1): after a block lands,
/// if it was an escalated-to-user block carrying a needs_drew
/// payload, raise the alert into the PM aggregator's throat from
/// inside the broker process — the same fixed wire contract the
/// CLI half posts (crate::needs_drew). The intake token resolves
/// by name through the in-process secrets source this broker
/// already uses for every other credential (DirectSecrets: the
/// colocated secrets broker). Fire-and-forget on a spawned
/// thread: the block response never waits on the throat, and a
/// failure is logged to stderr only — it never fails the block.
/// The throat dedupes by (source, key), so the CLI half and this
/// half posting for the same block is safe. This half goes live
/// only when the broker binary is replaced (the coordinated
/// switch); until then the CLI half is the live path.
fn maybe_raise_needs_user(st: &State, req: &Value, resp: &Value) {
    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return;
    }
    if req.get("reason_class").and_then(|v| v.as_str()) != Some("escalated-to-user") {
        return;
    }
    let blocker = req.get("blocker").and_then(|v| v.as_str()).unwrap_or("");
    let Some(nd) = crate::needs_user::from_block_request(req, blocker) else {
        return;
    };
    let secrets_url = st.secrets_url.clone();
    let secrets_token = st.secrets_token.clone();
    let agent = st.agent.clone();
    std::thread::spawn(move || {
        let resolver = Resolver::new(vec![Box::new(DirectSecrets {
            url: secrets_url,
            token: secrets_token,
            agent: agent.clone(),
        })]);
        let raised = resolver
            .resolve(crate::needs_user::ALERTS_INTAKE_TOKEN_SECRET)
            .map_err(|e| e.to_string())
            .and_then(|token| crate::needs_user::post_needs_user(token.expose(), &nd));
        match raised {
            Ok(_) => crate::log::info(
                "queue-broker",
                &format!("needs-drew raised for ledger item {}", nd.ledger_id),
            ),
            Err(e) => crate::log::info(
                "queue-broker",
                &format!("needs-drew throat post failed: {e}"),
            ),
        }
    });
}

/// #13774: the actions-exec agent verbs authenticate with the
/// job's GitHub OIDC JWT instead of the broker token — the agent
/// runs on a public runner and never holds the broker token. A
/// JWT-shaped X-Agent-Token (three base64url segments) on one of
/// these paths passes the token gate; the verb then verifies the
/// JWT against GitHub's JWKS and binds its claims to the session
/// row before acting. Any other call on these paths still needs
/// the broker token.
const ACTIONS_EXEC_AGENT_PATHS: [&str; 5] = [
    "/actions-exec-register",
    "/actions-exec-poll",
    "/actions-exec-io",
    "/actions-exec-result",
    "/actions-exec-close",
];

fn actions_exec_agent_call(path: &str, provided_token: Option<&str>) -> bool {
    if !ACTIONS_EXEC_AGENT_PATHS.contains(&path) {
        return false;
    }
    let Some(token) = provided_token else {
        return false;
    };
    let mut parts = token.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(s), None) => {
            !h.is_empty()
                && !p.is_empty()
                && !s.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        }
        _ => false,
    }
}

/// #13774: an extension request raises its tier's push on the
/// PM surface (fire-and-forget, the /block pattern): tier 1's
/// notification, tier 2's deny-window warning, tier 3's manual
/// ask. The verb has already recorded (or, tier 1, applied) the
/// request; this post is what puts it in front of Drew.
fn maybe_raise_actions_exec_extension(st: &State, resp: &Value) {
    if resp.get("requested").and_then(|v| v.as_bool()) != Some(true) {
        return;
    }
    let (Some(handle), Some(host_repo)) = (
        resp.get("handle").and_then(|v| v.as_str()),
        resp.get("host_repo").and_then(|v| v.as_str()),
    ) else {
        return;
    };
    let s = |k: &str| {
        resp.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let i = |k: &str, d: i64| resp.get(k).and_then(|v| v.as_i64()).unwrap_or(d);
    let info = crate::needs_user::ExtensionAlert {
        key: s("alert_key"),
        handle: handle.to_string(),
        context: resp
            .get("context")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        host_repo: host_repo.to_string(),
        workflow: s("workflow"),
        seat: s("seat"),
        run_id: resp.get("run_id").and_then(|v| v.as_i64()),
        run_attempt: resp.get("run_attempt").and_then(|v| v.as_i64()),
        created_at: s("created_at"),
        granted_s: i("granted_s", 0),
        elapsed_s: i("elapsed_s", 0),
        lease_until: s("lease_until"),
        max_until: s("max_until"),
        extend_s: i("extend_s", 1800),
        tier: i("tier", 3),
        ext_count_before: i("ext_count", 0),
        ext_total_s_before: i("ext_total_s", 0),
        decide_after: resp
            .get("decide_after")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        grace_until: s("grace_until"),
        reason: s("reason"),
    };
    let body = crate::needs_user::build_extension_alert_body(&info);
    let secrets_url = st.secrets_url.clone();
    let secrets_token = st.secrets_token.clone();
    let agent = st.agent.clone();
    std::thread::spawn(move || {
        let resolver = Resolver::new(vec![Box::new(DirectSecrets {
            url: secrets_url,
            token: secrets_token,
            agent: agent.clone(),
        })]);
        let raised = resolver
            .resolve(crate::needs_user::ALERTS_INTAKE_TOKEN_SECRET)
            .map_err(|e| e.to_string())
            .and_then(|token| {
                crate::needs_user::post_plain_alert_to(
                    &crate::needs_user::alerts_url(),
                    token.expose(),
                    &body,
                )
            });
        match raised {
            Ok(_) => crate::log::info("queue-broker", "actions-exec extension alert raised"),
            Err(e) => crate::log::info(
                "queue-broker",
                &format!("actions-exec extension alert failed: {e}"),
            ),
        }
    });
}

/// Broker-owned GitHub synchronization for a committed reprioritize.
/// Queue state is never rolled back on an API failure; the durable
/// retry row and task event are written instead. Only `priority/*`
/// labels are edited, so expedite/next-up and unrelated labels are
/// outside this operation's write set.
fn sync_reprioritized(st: &State, t: &crate::turso_queue::TursoBackend, resp: &Value) {
    let Some(row) = resp.get("row") else { return };
    let id = row.get("id").and_then(Value::as_i64).unwrap_or(0);
    let payload = row.get("payload").unwrap_or(&Value::Null);
    let Some(number) = payload.pointer("/issue/number").and_then(Value::as_i64) else {
        return; // queue-only task; no linked issue
    };
    let Some(repo) = payload.get("repo").and_then(Value::as_str) else {
        return;
    };
    let Some(priority) = row.get("priority").and_then(Value::as_i64) else {
        return;
    };
    let Some(label) = crate::validate::label_for_priority(priority) else {
        return;
    };
    let category = crate::validate::category_for_priority(priority).unwrap_or("unknown");
    let attempt = (|| -> Result<(), String> {
        let resolver = Resolver::new(vec![Box::new(DirectSecrets {
            url: st.secrets_url.clone(),
            token: st.secrets_token.clone(),
            agent: st.agent.clone(),
        })]);
        let token = resolver
            .resolve(GH_DEP_TOKEN_SECRET)
            .map_err(|e| e.to_string())?;
        let base = format!("https://api.github.com/repos/{repo}/issues/{number}");
        let get = st
            .agent
            .get(&base)
            .header("Authorization", &format!("Bearer {}", token.expose()))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "fleet-queue-broker")
            .call()
            .map_err(|e| format!("github_read_failed: {e}"))?;
        let issue: Value = get
            .into_body()
            .read_json()
            .map_err(|e| format!("github_read_failed: {e}"))?;
        let names: Vec<String> = issue
            .get("labels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|l| l.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect();
        for old in names
            .iter()
            .filter(|n| n.starts_with("priority/") && n.as_str() != label)
        {
            let url = format!("{base}/labels/{}", old.replace('/', "%2F"));
            st.agent
                .delete(&url)
                .header("Authorization", &format!("Bearer {}", token.expose()))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "fleet-queue-broker")
                .call()
                .map_err(|e| format!("github_label_remove_failed: {e}"))?;
        }
        if !names.iter().any(|n| n == label) {
            st.agent
                .post(&format!("{base}/labels"))
                .header("Authorization", &format!("Bearer {}", token.expose()))
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "fleet-queue-broker")
                .send_json(json!({"labels": [label]}))
                .map_err(|e| format!("github_label_add_failed: {e}"))?;
        }
        // Update lineage.priority only inside an existing metadata block.
        if let Some(body) = issue.get("body").and_then(Value::as_str) {
            if let (Some(start), Some(end)) = (
                body.find("<!-- pm-metadata:start -->"),
                body.find("<!-- pm-metadata:end -->"),
            ) {
                if end > start {
                    let block = &body[start..end];
                    if let Some(open) = block.find("```json") {
                        let json_start = open + "```json".len();
                        if let Some(relative_close) = block[json_start..].find("```") {
                            let close = json_start + relative_close;
                            let mut metadata: Value =
                                serde_json::from_str(block[json_start..close].trim())
                                    .map_err(|e| format!("pm_metadata_parse_failed: {e}"))?;
                            if metadata.get("lineage").is_some_and(Value::is_object) {
                                metadata["lineage"]["priority"] = json!(category);
                                let replacement = format!(
                                    "```json\n{}\n```",
                                    serde_json::to_string_pretty(&metadata).unwrap_or_default()
                                );
                                let patched = format!(
                                    "{}{}{}",
                                    &body[..start],
                                    block.replacen(&block[open..close + 3], &replacement, 1),
                                    &body[end..]
                                );
                                st.agent
                                    .patch(&base)
                                    .header("Authorization", &format!("Bearer {}", token.expose()))
                                    .header("Accept", "application/vnd.github+json")
                                    .header("User-Agent", "fleet-queue-broker")
                                    .send_json(json!({"body": patched}))
                                    .map_err(|e| format!("github_metadata_update_failed: {e}"))?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    // Do not acknowledge an older sync if another reprioritize landed
    // while the GitHub request was in flight. The durable intent for
    // the newer category remains pending and will reconcile from the
    // current task row on the next broker request.
    let still_current = crate::turso_queue::get_task(t, id)
        .ok()
        .flatten()
        .and_then(|current| current.get("priority").and_then(Value::as_i64))
        == Some(priority);
    if attempt.is_ok() && still_current {
        let _ = t.exec("UPDATE tasks SET progress = json_set(COALESCE(progress, '{}'), '$.reprioritize_sync_pending', false) WHERE id = ?", &[
            crate::turso_queue::Arg::Int(id),
        ]);
        let _ = t.exec("UPDATE reprioritize_sync_outbox SET state = 'done', updated_at = ? WHERE task_id = ? AND issue = ? AND state = 'pending'", &[
            crate::turso_queue::Arg::Text(crate::turso_queue::now_iso()), crate::turso_queue::Arg::Int(id), crate::turso_queue::Arg::Int(number),
        ]);
    }
    if let Err(error) = attempt {
        let now = crate::turso_queue::now_iso();
        let error_class = error.split(':').next().unwrap_or("github_sync_failed");
        let pending = t.query_objs("SELECT id FROM reprioritize_sync_outbox WHERE task_id = ? AND issue = ? AND state = 'pending' LIMIT 1", &[
            crate::turso_queue::Arg::Int(id), crate::turso_queue::Arg::Int(number),
        ], &[]).unwrap_or_default();
        if pending.is_empty() {
            let _ = t.exec("INSERT INTO reprioritize_sync_outbox (task_id, repo, issue, category, label, state, attempts, available_at, last_error_class, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?, ?)", &[
            crate::turso_queue::Arg::Int(id), crate::turso_queue::Arg::Text(repo.to_owned()), crate::turso_queue::Arg::Int(number),
            crate::turso_queue::Arg::Text(category.to_owned()), crate::turso_queue::Arg::Text(label.to_owned()), crate::turso_queue::Arg::Text(now.clone()),
            crate::turso_queue::Arg::Text(error_class.to_owned()), crate::turso_queue::Arg::Text(now.clone()), crate::turso_queue::Arg::Text(now.clone()),
        ]);
        } else {
            let _ = t.exec("UPDATE reprioritize_sync_outbox SET attempts = attempts + 1, available_at = ?, last_error_class = ?, updated_at = ? WHERE task_id = ? AND issue = ? AND state = 'pending'", &[
            crate::turso_queue::Arg::Text(crate::turso_queue::iso(crate::turso_queue::now_unix() + 30)), crate::turso_queue::Arg::Text(error_class.to_owned()), crate::turso_queue::Arg::Text(now.clone()), crate::turso_queue::Arg::Int(id), crate::turso_queue::Arg::Int(number),
        ]);
        }
        let detail = json!({"error_class": error_class, "issue": number, "category": category});
        let _ = t.exec("INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES ('queue-broker', ?, 'reprioritize_sync_pending', ?, ?)", &[
            crate::turso_queue::Arg::Int(id), crate::turso_queue::Arg::Text(detail.to_string()), crate::turso_queue::Arg::Text(now),
        ]);
        let _ = crate::log::info(
            "queue-broker",
            &format!("reprioritize GitHub sync pending for task {id}: {error_class}"),
        );
    }
}

/// Retry due sync intents before serving the next broker call. A
/// retry uses the latest task row (if it still exists), so a newer
/// reprioritize supersedes an older pending category cleanly.
fn retry_reprioritize_sync(st: &State, t: &crate::turso_queue::TursoBackend) {
    let Ok(rows) = t.query_objs("SELECT task_id FROM reprioritize_sync_outbox WHERE state = 'pending' AND available_at <= ? ORDER BY id LIMIT 20", &[
        crate::turso_queue::Arg::Text(crate::turso_queue::now_iso()),
    ], &[]) else { return };
    for r in rows {
        let id = r.get("task_id").and_then(Value::as_i64).unwrap_or(0);
        if let Ok(Some(row)) = crate::turso_queue::get_task(t, id) {
            sync_reprioritized(st, t, &json!({"row": row}));
        }
    }
}

/// Cursor form of `/history-since` for the PM projector. The original
/// timestamp filter remains available to older callers; `after_id` is
/// the monotonic resume point so equal-second history rows cannot be
/// skipped or replayed as the feed advances.
fn dispatch_history_since_cursor(t: &crate::turso_queue::TursoBackend, req: &Value) -> Value {
    let Some(after_id) = req.get("after_id").and_then(Value::as_i64) else {
        return fail(2, "history-since: after_id must be an integer");
    };
    if after_id < 0 {
        return fail(2, "history-since: after_id must be non-negative");
    }
    let limit = req
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(200)
        .clamp(1, 50_000) as usize;
    let task_id = req.get("task_id").and_then(Value::as_i64);
    let since = req.get("since").and_then(Value::as_str);
    let mut sql = "SELECT id, event_id, task_id, kind, from_status, to_status, from_priority, \
                   to_priority, actor, session_id, model_provider, model_id, detail, source, created_at \
                   FROM task_history WHERE id > ?"
        .to_string();
    let mut args = vec![crate::turso_queue::Arg::Int(after_id)];
    if let Some(id) = task_id {
        sql.push_str(" AND task_id = ?");
        args.push(crate::turso_queue::Arg::Int(id));
    }
    if let Some(since) = since {
        sql.push_str(" AND created_at >= ?");
        args.push(crate::turso_queue::Arg::Text(since.to_string()));
    }
    sql.push_str(&format!(" ORDER BY id ASC LIMIT {}", limit + 1));
    match t.query_objs(&sql, &args, &["detail"]) {
        Ok(rows) => history_cursor_page(rows, after_id, limit, task_id),
        Err(e) => fail(2, e),
    }
}

fn history_cursor_page(
    mut rows: Vec<Value>,
    after_id: i64,
    limit: usize,
    task_id: Option<i64>,
) -> Value {
    rows.retain(|row| {
        row.get("id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id > after_id)
    });
    rows.sort_by_key(|row| row.get("id").and_then(Value::as_i64).unwrap_or(i64::MAX));
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = rows
        .last()
        .and_then(|row| row.get("id"))
        .and_then(Value::as_i64)
        .unwrap_or(after_id);
    ok(json!({
        "rows": rows,
        "task_id": task_id,
        "next_cursor": next_cursor,
        "has_more": has_more
    }))
}

const TRUST_BUNDLE_VERSION_HEADER: &str = "X-Fleet-Trust-Bundle-Version";

/// Record an authenticated TLS TCP peer's presented version, or an unverified
/// observation when it makes no single positive version claim. The version
/// is client-reported, not a client identity or a server-side validation
/// result. Plaintext, UDS, unauthenticated requests, and missing peer IPs
/// cannot create either kind of observation.
fn record_trust_bundle_client(
    clients: &mut BTreeMap<(IpAddr, Option<u32>), TrustBundleClientReport>,
    transport: crate::uds_http::RequestTransport,
    headers: &[(String, String)],
    authenticated: bool,
    now_unix: u64,
) -> bool {
    if !authenticated || transport.kind != crate::uds_http::RequestTransportKind::TcpTls {
        return false;
    }
    let Some(client_ip) = transport.peer_ip else {
        return false;
    };
    let mut version_headers = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(TRUST_BUNDLE_VERSION_HEADER));
    let version = match (version_headers.next(), version_headers.next()) {
        (None, None) => None,
        (Some((_, raw_version)), None) => raw_version
            .parse::<u32>()
            .ok()
            .filter(|version| *version > 0),
        _ => None,
    };

    clients
        .entry((client_ip, version))
        .and_modify(|report| {
            report.last_seen_unix = report.last_seen_unix.max(now_unix);
        })
        .or_insert(TrustBundleClientReport {
            last_seen_unix: now_unix,
        });
    true
}

fn trust_bundle_health_payload(
    clients: &BTreeMap<(IpAddr, Option<u32>), TrustBundleClientReport>,
    now_unix: u64,
) -> Value {
    let clients = clients
        .iter()
        .map(|((client_ip, version), report)| {
            json!({
                "client_ip": client_ip.to_string(),
                "trust_bundle_version": version,
                "version_status": if version.is_some() { "presented" } else { "unverified" },
                "last_seen_unix": report.last_seen_unix,
                "age_seconds": now_unix.saturating_sub(report.last_seen_unix),
            })
        })
        .collect::<Vec<_>>();
    json!({"service": "queue-broker", "trust_bundle_clients": clients})
}

fn queue_broker_health(st: &State) -> Value {
    let clients = st
        .trust_bundle_clients
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    trust_bundle_health_payload(&clients, now_unix_seconds())
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn dispatch(st: &State, path: &str, req: &Value) -> Value {
    if let Err(e) = validate_durations(path, req) {
        return fail(2, e);
    }
    if let Err(e) = validate_request_not_before(path, req) {
        return fail(2, e);
    }
    match path {
        "/validate" => {
            let packet = req.get("packet").cloned().unwrap_or(Value::Null);
            let (errors, warnings) = crate::validate::validate_packet(&packet);
            ok(json!({"errors": errors, "warnings": warnings}))
        }
        "/healthz" => ok(queue_broker_health(st)),
        "/door-health" => match &st.hosted_door {
            Some(door) => door.health(req).unwrap_or_else(|e| fail(3, e)),
            None => fail(3, "hosted queue door is not configured"),
        },
        _ => {
            let Some(turso) = st.turso.get() else {
                let why = st
                    .turso_pending
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                return fail(3, format!("queue backend not ready: {why}"));
            };
            retry_reprioritize_sync(st, turso);
            // The worker id arrives over the broker's authenticated
            // (X-Agent-Token) channel, like every other lifecycle
            // verb — that is the authenticated caller identity the
            // reprioritized event records as actor. A legacy `actor`
            // string without a worker is promoted for back-compat;
            // a caller-provided actor never overrides the worker.
            let mut effective_req = req.clone();
            if path == "/reprioritize" {
                let has_worker = effective_req
                    .get("worker")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty());
                if !has_worker {
                    if let Some(a) = effective_req
                        .get("actor")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    {
                        if !a.trim().is_empty() {
                            effective_req["worker"] = json!(a);
                        }
                    }
                }
            }
            let resp = if path == "/history-since" && effective_req.get("after_id").is_some() {
                dispatch_history_since_cursor(turso, &effective_req)
            } else {
                crate::turso_queue::dispatch(turso, path, &effective_req)
            };
            if path == "/reprioritize" && resp.get("ok").and_then(Value::as_bool) == Some(true) {
                sync_reprioritized(st, turso, &resp);
            }
            // #8568: the broker-side needs-drew raise for an
            // escalated block (fire-and-forget).
            if path == "/block" {
                maybe_raise_needs_user(st, req, &resp);
            }
            // #13774: the emergency-extension needs-drew raise.
            if path == "/actions-exec-extend-request" {
                maybe_raise_actions_exec_extension(st, &resp);
            }
            resp
        }
    }
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
    if !actions_exec_agent_call(path, provided_token) && !token_matches(provided_token, token) {
        return Some((401, json!({"error": "unauthorized"})));
    }
    if let Some(n) = content_length {
        if n > crate::uds_http::MAX_BODY_BYTES as u64 {
            return Some((413, json!({"error": "request body too large"})));
        }
    }
    None
}

/// #14191: attribute every queue event this request writes to the
/// context id its `X-Fleet-Ctx-*` headers carry (none: no change).
fn request_ctx_scope(
    req: &crate::uds_http::HttpRequest,
) -> crate::turso_queue::ctx_scope::CtxScope {
    let ctx = fleet_config::ctx::Ctx::from_headers(
        req.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    );
    crate::turso_queue::ctx_scope::enter(ctx.id())
}

/// Transport-agnostic request handling (#6216): the TCP loop adapts
/// tiny_http requests to this function and the UDS loop adapts
/// Unix-socket requests; routes and auth are identical on both
/// transports by construction. Returns (status, JSON body).
fn handle_request(st: &State, token: &str, request: &crate::uds_http::HttpRequest) -> (u16, Value) {
    let path = request.target.split('?').next().unwrap_or("");
    let provided_token = request.header("X-Agent-Token");
    if request.method == "GET" && path == "/healthz" {
        // This unauthenticated endpoint is a liveness probe. Keep rollout
        // inventory behind the ordinary broker-token POST route below.
        return (200, ok(json!({"service": "queue-broker"})));
    }

    let token_authenticated = token_matches(provided_token, token);
    if !actions_exec_agent_call(path, provided_token) && !token_authenticated {
        return (401, json!({"error": "unauthorized"}));
    }
    if request.method != "POST" {
        return (405, json!({"error": "POST only"}));
    }
    // Only the ordinary broker token path is used for the trust-version
    // report. OIDC action calls remain authenticated by their own verifier,
    // but are not bundle-health clients. The transport metadata is set by
    // the TCP/UDS adapters, never by request headers.
    if token_authenticated {
        let mut clients = st
            .trust_bundle_clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        record_trust_bundle_client(
            &mut clients,
            request.transport,
            &request.headers,
            true,
            now_unix_seconds(),
        );
    }

    match std::str::from_utf8(&request.body) {
        Ok(buf) => {
            let mut parsed: Value = serde_json::from_str(buf).unwrap_or(Value::Null);
            // #13774: _oidc_jwt is broker-injected, never
            // client-supplied — strip any spoofed copy from
            // every body, then inject the header token on
            // agent-path calls for the verbs to verify.
            if let Some(obj) = parsed.as_object_mut() {
                obj.remove("_oidc_jwt");
                if actions_exec_agent_call(path, provided_token) {
                    if let Some(tok) = provided_token {
                        obj.insert("_oidc_jwt".to_string(), Value::String(tok.to_string()));
                    }
                }
            }
            (200, dispatch(st, path, &parsed))
        }
        Err(e) => (
            400,
            json!({"ok": false, "exit": 2, "error": format!("bad body: {e}")}),
        ),
    }
}

pub fn run() -> i32 {
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

    // Platform root store, not webpki-roots alone: this agent
    // carries the broker's GitHub dep-probe reads, and a broker
    // running behind a TLS-intercepting egress proxy presents
    // the proxy's CA, which only the platform store carries
    // (code#13122; same fix as fleet's gh.rs App leg).
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(90)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let hosted_door = match HostedQueueDoor::from_env(agent.clone()) {
        Ok(door) => door,
        Err(error) => {
            crate::log::info("queue-broker", &error);
            return 3;
        }
    };
    // Turso is the only queue backend: QUEUE_BROKER_BACKEND unset
    // (or empty) means turso; any other value is a config error.
    if let Ok(v) = std::env::var("QUEUE_BROKER_BACKEND") {
        let v = v.trim();
        if !v.is_empty() && !v.eq_ignore_ascii_case("turso") {
            crate::log::info(
                "queue-broker",
                &format!(
                    "QUEUE_BROKER_BACKEND={v} is not supported; turso is the only queue backend"
                ),
            );
            return 3;
        }
    }
    // A Turso token file, when named, is a local config read: a
    // missing or unreadable file stays a startup config error.
    let turso_url = std::env::var("QUEUE_BROKER_TURSO_URL")
        .unwrap_or_else(|_| crate::turso_queue::DEFAULT_TURSO_URL.to_string());
    let turso_token_from_file = match std::env::var("QUEUE_BROKER_TURSO_TOKEN_FILE") {
        Ok(f) => match crate::token_file::load_token_file(&f) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log::info("queue-broker", &e);
                return 3;
            }
        },
        Err(_) => None,
    };

    let state = Arc::new(State {
        secrets_url,
        secrets_token,
        agent,
        hosted_door,
        turso: std::sync::OnceLock::new(),
        turso_pending: Mutex::new("starting".to_string()),
        trust_bundle_clients: Mutex::new(BTreeMap::new()),
        coord_sweep_running: Mutex::new(false),
    });

    // #15533: the tailnet address may not exist yet at boot
    // (Tailscale assigns it after us); retry EADDRNOTAVAIL within
    // the bind-guard budget instead of burning the restart budget.
    let server = match crate::bind_guard::tcp_bind_with_retry("queue-broker", &bind) {
        Ok(server) => server,
        Err(e) => {
            crate::log::info("queue-broker", &format!("cannot bind {bind}: {e:?}"));
            return 3;
        }
    };
    crate::log::info("queue-broker", &format!("listening on {bind}"));

    // #12247: bind first, then bring the Turso backend up on a
    // startup thread, so /healthz and /validate answer at once and
    // queue verbs answer exit 3 with the latest startup error until
    // the backend is ready. A backend that cannot come up within the
    // retry budget still exits the broker with 3, so a supervisor
    // restarts it exactly as when the backend came up before bind.
    {
        let st = Arc::clone(&state);
        std::thread::spawn(move || {
            match start_turso_backend(&st, turso_url, turso_token_from_file) {
                Ok(backend) => {
                    crate::log::info(
                        "queue-broker",
                        &format!("queue backend: turso ({})", backend.url()),
                    );
                    let _ = st.turso.set(backend);
                }
                Err(e) => {
                    crate::log::info("queue-broker", &e);
                    std::process::exit(3);
                }
            }
        });
    }

    // #13765: the coordinator sweep, on its own thread beside the
    // #12247 startup thread: every COORD_SWEEP_INTERVAL_SECS (20s
    // default, COORD_SWEEP_INTERVAL_SECS override) it refreshes the
    // true load view, expires stale holds, settles finished
    // Blacksmith runs, recomputes the counters, and grants the
    // waitlist FIFO. The pass skips when the backend is not ready
    // yet or the previous pass is still running (a slow GitHub
    // never piles a second grant loop onto the same counters),
    // and every leg inside the sweep is failure-tolerant, so the
    // thread itself never dies — it logs the pass summary.
    {
        let st = Arc::clone(&state);
        std::thread::spawn(move || {
            let interval = std::env::var("COORD_SWEEP_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(crate::turso_queue::COORD_SWEEP_INTERVAL_SECS);
            loop {
                std::thread::sleep(Duration::from_secs(interval));
                let Some(turso) = st.turso.get() else {
                    continue;
                };
                let already = st
                    .coord_sweep_running
                    .lock()
                    .map(|mut g| std::mem::replace(&mut *g, true))
                    .unwrap_or(true);
                if already {
                    continue;
                }
                let gh = BrokerCoordGh {
                    secrets_url: st.secrets_url.clone(),
                    secrets_token: st.secrets_token.clone(),
                    agent: st.agent.clone(),
                    token: Mutex::new(None),
                    pass: Mutex::new(std::collections::HashMap::new()),
                };
                let now = crate::turso_queue::now_iso();
                let sum = crate::turso_queue::coordinator_sweep(turso, &gh, &now);
                if let Ok(mut g) = st.coord_sweep_running.lock() {
                    *g = false;
                }
                crate::log::info("queue-broker", &format!("coord sweep: {sum}"));
            }
        });
    }

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
                        let _ctx = request_ctx_scope(req);
                        let (status, value) = handle_request(&st, &tok, req);
                        crate::uds_http::log_ctx_trip("queue-broker", req, status);
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

    // BT-3 (#14401): Phase 1 TLS on the TCP leg, additive and
    // default-off (design rules M1/M2). A half-configured or
    // unusable leaf is a startup config error, never a silent
    // plaintext fallback.
    let broker_tls = match crate::broker_tls::BrokerTlsServer::from_env() {
        Ok(tls) => tls,
        Err(e) => {
            crate::log::info("queue-broker", &e);
            return 3;
        }
    };
    if broker_tls.is_some() {
        crate::log::info(
            "queue-broker",
            "TLS server authentication on the TCP leg (BT-3 Phase 1): presenting the \
             broker-server leaf; token auth unchanged; plaintext Phase 0 connections \
             still accepted on the same port and logged",
        );
    }

    let pre_token = token.clone();
    let precheck = move |head: &crate::uds_http::HttpRequest| {
        pre_body_refusal(
            &head.method,
            &head.target,
            head.header("X-Agent-Token"),
            &pre_token,
            head.header("Content-Length")
                .and_then(|v| v.parse::<u64>().ok()),
        )
        .map(|(status, value)| {
            (
                status,
                serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
            )
        })
    };
    let handler = move |req: &crate::uds_http::HttpRequest| {
        let _ctx = request_ctx_scope(req);
        let (status, value) = handle_request(&state, &token, req);
        crate::uds_http::log_ctx_trip("queue-broker", req, status);
        (
            status,
            serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
        )
    };
    match broker_tls {
        Some(tls) => crate::tcp_http::serve_tls(server, tls, precheck, handler),
        None => crate::tcp_http::serve(server, precheck, handler),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn test_state() -> State {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        State {
            secrets_url: String::new(),
            secrets_token: String::new(),
            agent,
            hosted_door: None,
            turso: std::sync::OnceLock::new(),
            turso_pending: Mutex::new("test backend".to_string()),
            trust_bundle_clients: Mutex::new(BTreeMap::new()),
            coord_sweep_running: Mutex::new(false),
        }
    }

    #[test]
    fn health_serializes_stale_client_version_and_age() {
        // Oracle: an old client's presented version remains visible with
        // last-seen and age data, so a rollover operator can identify it.
        let client_ip = "100.64.0.12".parse::<IpAddr>().unwrap();
        let reports = BTreeMap::from([
            (
                (client_ip, Some(7)),
                TrustBundleClientReport {
                    last_seen_unix: 100,
                },
            ),
            (
                (client_ip, Some(8)),
                TrustBundleClientReport {
                    last_seen_unix: 9_000,
                },
            ),
        ]);

        assert_eq!(
            trust_bundle_health_payload(&reports, 10_000),
            json!({
                "service": "queue-broker",
                "trust_bundle_clients": [
                    {
                        "client_ip": "100.64.0.12",
                        "trust_bundle_version": 7,
                        "version_status": "presented",
                        "last_seen_unix": 100,
                        "age_seconds": 9_900
                    },
                    {
                        "client_ip": "100.64.0.12",
                        "trust_bundle_version": 8,
                        "version_status": "presented",
                        "last_seen_unix": 9_000,
                        "age_seconds": 1_000
                    }
                ]
            })
        );
    }

    proptest! {
        #[test]
        fn trust_version_report_requires_tls_auth_peer_and_one_positive_header(
            kind in 0u8..3,
            authenticated in any::<bool>(),
            has_peer in any::<bool>(),
            has_header in any::<bool>(),
            version in any::<u32>(),
            malformed in any::<bool>(),
            duplicate in any::<bool>(),
        ) {
            // Oracle: the report is present iff the server observed direct
            // TLS, the request authenticated, a peer IP exists, and exactly
            // one positive version header was presented.
            let kind = match kind {
                0 => crate::uds_http::RequestTransportKind::Uds,
                1 => crate::uds_http::RequestTransportKind::TcpPlain,
                _ => crate::uds_http::RequestTransportKind::TcpTls,
            };
            let client_ip = "100.64.0.12".parse::<IpAddr>().unwrap();
            let transport = crate::uds_http::RequestTransport {
                kind,
                peer_ip: has_peer.then_some(client_ip),
            };
            let mut headers = has_header
                .then(|| {
                    (
                        TRUST_BUNDLE_VERSION_HEADER.to_string(),
                        if malformed {
                            "not-a-version".to_string()
                        } else {
                            version.to_string()
                        },
                    )
                })
                .into_iter()
                .collect::<Vec<_>>();
            if duplicate && has_header {
                headers.push((
                    TRUST_BUNDLE_VERSION_HEADER.to_string(),
                    version.to_string(),
                ));
            }
            let mut reports = BTreeMap::new();

            let recorded = record_trust_bundle_client(
                &mut reports,
                transport,
                &headers,
                authenticated,
                1234,
            );
            let expected_observation = kind == crate::uds_http::RequestTransportKind::TcpTls
                && authenticated
                && has_peer;
            prop_assert_eq!(recorded, expected_observation);
            prop_assert_eq!(reports.len(), usize::from(expected_observation));
            if expected_observation {
                let expected_version = if has_header && !duplicate && !malformed && version > 0 {
                    Some(version)
                } else {
                    None
                };
                let report = reports.get(&(client_ip, expected_version)).unwrap();
                prop_assert_eq!(report.last_seen_unix, 1234);
            }
        }
    }

    #[test]
    fn handler_records_only_authorized_tls_version_and_health_returns_it() {
        // Oracle: the direct-TCP TLS+token request updates health; a wrong
        // token, plaintext header spoof, and UDS header do not create a
        // version report; a later TLS request without a version is retained
        // separately as an unverified observation.
        let state = test_state();
        let client_ip = "100.64.0.12".parse::<IpAddr>().unwrap();
        let request = |kind, token: &str, version: Option<&str>| {
            let mut headers = vec![("X-Agent-Token".to_string(), token.to_string())];
            if let Some(version) = version {
                headers.push((TRUST_BUNDLE_VERSION_HEADER.to_string(), version.to_string()));
            }
            crate::uds_http::HttpRequest {
                method: "POST".to_string(),
                target: "/validate".to_string(),
                headers,
                body: br#"{"packet":{}}"#.to_vec(),
                transport: crate::uds_http::RequestTransport {
                    kind,
                    peer_ip: Some(client_ip),
                },
            }
        };

        let tls = request(
            crate::uds_http::RequestTransportKind::TcpTls,
            "token",
            Some("17"),
        );
        assert_eq!(handle_request(&state, "token", &tls).0, 200);
        let report = queue_broker_health(&state)["trust_bundle_clients"][0].clone();
        assert_eq!(report["client_ip"], "100.64.0.12");
        assert_eq!(report["trust_bundle_version"], 17);

        let bad_token = request(
            crate::uds_http::RequestTransportKind::TcpTls,
            "wrong",
            Some("99"),
        );
        assert_eq!(handle_request(&state, "token", &bad_token).0, 401);
        let plain = request(
            crate::uds_http::RequestTransportKind::TcpPlain,
            "token",
            Some("99"),
        );
        assert_eq!(handle_request(&state, "token", &plain).0, 200);
        let uds = request(
            crate::uds_http::RequestTransportKind::Uds,
            "token",
            Some("99"),
        );
        assert_eq!(handle_request(&state, "token", &uds).0, 200);
        let break_glass = request(crate::uds_http::RequestTransportKind::TcpTls, "token", None);
        assert_eq!(handle_request(&state, "token", &break_glass).0, 200);

        let clients = &queue_broker_health(&state)["trust_bundle_clients"];
        assert_eq!(clients.as_array().unwrap().len(), 2);
        let clients = clients.as_array().unwrap();
        assert!(clients.iter().any(|client| {
            client["trust_bundle_version"] == 17 && client["version_status"] == "presented"
        }));
        assert!(clients.iter().any(|client| {
            client["trust_bundle_version"].is_null() && client["version_status"] == "unverified"
        }));

        let mut authenticated_health =
            request(crate::uds_http::RequestTransportKind::TcpTls, "token", None);
        authenticated_health.target = "/healthz".to_string();
        let (status, health) = handle_request(&state, "token", &authenticated_health);
        assert_eq!(status, 200);
        assert_eq!(health, ok(queue_broker_health(&state)));
    }

    #[test]
    fn unauthenticated_get_healthz_only_reports_liveness() {
        // Oracle: a public liveness probe never exposes peer IPs, versions,
        // or last-seen timestamps from the authenticated rollout inventory.
        let state = test_state();
        let client_ip = "100.64.0.12".parse::<IpAddr>().unwrap();
        state.trust_bundle_clients.lock().unwrap().insert(
            (client_ip, Some(17)),
            TrustBundleClientReport {
                last_seen_unix: 1234,
            },
        );
        let request = crate::uds_http::HttpRequest {
            method: "GET".to_string(),
            target: "/healthz".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            transport: crate::uds_http::RequestTransport {
                kind: crate::uds_http::RequestTransportKind::TcpPlain,
                peer_ip: Some(client_ip),
            },
        };

        let (status, body) = handle_request(&state, "token", &request);
        assert_eq!(status, 200);
        assert_eq!(body, ok(json!({"service": "queue-broker"})));
        assert!(body.get("trust_bundle_clients").is_none());
    }

    #[test]
    fn door_health_uses_authenticated_hosted_queue_contract() {
        // Oracle: the hosted queue door contract is POST
        // /v1/queue/health with a Bearer service token and the original
        // JSON request; its broker-envelope response is returned intact.
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            let body_end = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "client closed before sending the request");
                request.extend_from_slice(&buffer[..count]);
                if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    let end = header_end + 4 + content_length;
                    if request.len() >= end {
                        break end;
                    }
                }
            };
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 21\r\nconnection: close\r\n\r\n{\"ok\":true,\"rows\":[]}",
                )
                .unwrap();
            String::from_utf8(request[..body_end].to_vec()).unwrap()
        });
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let door = HostedQueueDoor {
            base_url: format!("http://{address}"),
            token: "service-token".into(),
            agent,
        };

        let response = door
            .health(&json!({"queue": "repo-health-code-pilot"}))
            .unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/queue/health HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer service-token\r\n")
        );
        assert!(
            request.contains("repo-health-code-pilot"),
            "hosted request includes the requested queue: {request:?}"
        );
        assert_eq!(response, json!({"ok": true, "rows": []}));
    }

    #[test]
    fn landing_from_json_10626() {
        use crate::turso_queue::{Landing, LinkKind};
        let closed = json!({"state": "closed", "closed_at": "2026-10-25T00:00:00Z", "state_reason": "completed"});
        assert_eq!(
            landing_from_json(LinkKind::Issue, &closed),
            Landing::Landed {
                state: "closed (completed)".into(),
                at: "2026-10-25T00:00:00Z".into()
            }
        );
        assert_eq!(
            landing_from_json(LinkKind::Issue, &json!({"state": "open"})),
            Landing::Open
        );
        assert!(matches!(
            landing_from_json(LinkKind::Issue, &json!({"state": "closed"})),
            Landing::Unknown(_)
        ));
        assert!(matches!(
            landing_from_json(LinkKind::Issue, &json!({})),
            Landing::Unknown(_)
        ));
        let merged = json!({"state": "closed", "merged_at": "2026-11-01T00:00:00Z"});
        assert_eq!(
            landing_from_json(LinkKind::Pr, &merged),
            Landing::Landed {
                state: "merged".into(),
                at: "2026-11-01T00:00:00Z".into()
            }
        );
        // Closed unmerged leaves the work open.
        assert_eq!(
            landing_from_json(LinkKind::Pr, &json!({"state": "closed", "merged_at": null})),
            Landing::Open
        );
        assert_eq!(
            landing_from_json(LinkKind::Pr, &json!({"state": "open"})),
            Landing::Open
        );
    }

    #[test]
    fn park_and_revive_not_before_are_gated_before_the_rpc() {
        // #6426: /park and /revive forwarded a raw not_before to the
        // RPC unchecked. The dispatch gate rejects garbage and
        // far-future values; absent/null and valid values pass, and
        // other verbs are untouched by the gate.
        for path in ["/park", "/revive", "/list"] {
            for bad in [
                json!("banana"),
                json!(""),
                json!(42),
                json!("9999-01-01T00:00:00Z"),
            ] {
                let req = json!({"not_before": bad});
                let err = validate_request_not_before(path, &req)
                    .err()
                    .expect("invalid not_before must be rejected");
                assert!(err.contains("not_before"), "error names the field: {err}");
            }
            assert!(validate_request_not_before(path, &json!({})).is_ok());
            assert!(validate_request_not_before(path, &json!({"not_before": Value::Null})).is_ok());
            assert!(
                validate_request_not_before(path, &json!({"not_before": "2020-01-01T00:00:00Z"}))
                    .is_ok()
            );
        }
        // A verb without a not_before surface is not gated.
        assert!(validate_request_not_before("/claim", &json!({"not_before": "banana"})).is_ok());
    }

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
        // The dispatch-level guard fires per path before the Turso
        // dispatch (Codex finding on PR #8425), and leaves
        // unrelated paths alone.
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

/// The Turso backend's startup (#12247), run after bind: the token
/// (from the named file, or by name from the secrets broker within
/// the startup retry budget), the dep probe (#8276) and the schema.
/// Each failure is recorded in `turso_pending`, so a queue verb that
/// arrives first reports why the backend is not ready.
fn start_turso_backend(
    st: &State,
    url: String,
    token_from_file: Option<String>,
) -> Result<crate::turso_queue::TursoBackend, String> {
    let note = |why: String| {
        if let Ok(mut pending) = st.turso_pending.lock() {
            *pending = why;
        }
    };
    let token = match token_from_file {
        Some(token) => token,
        None => {
            let resolver = Resolver::new(vec![Box::new(DirectSecrets {
                url: st.secrets_url.clone(),
                token: st.secrets_token.clone(),
                agent: st.agent.clone(),
            })]);
            match resolve_with_startup_retry_observed(
                &resolver,
                crate::turso_queue::TURSO_TOKEN_SECRET_NAME,
                Duration::from_secs(30),
                |e| note(format!("turso backend token resolve: {e}")),
            ) {
                Ok(v) => v.expose().to_string(),
                Err(e) => {
                    let msg =
                        format!("turso backend token resolve failed after 30s of retries: {e}");
                    note(msg.clone());
                    return Err(msg);
                }
            }
        }
    };
    let mut backend = crate::turso_queue::TursoBackend::new(url, token, st.agent.clone());
    // #8276: the dep rechecker's GitHub window, authenticated by name
    // through the same secrets broker as the Turso token above.
    backend.set_gh_probe(Arc::new(GhDepProbe {
        secrets_url: st.secrets_url.clone(),
        secrets_token: st.secrets_token.clone(),
        agent: st.agent.clone(),
        token: Mutex::new(None),
    }));
    if let Err(e) = backend.ensure_schema() {
        let msg = format!("turso backend schema ensure failed: {e}");
        note(msg.clone());
        return Err(msg);
    }
    Ok(backend)
}

#[cfg(test)]
fn resolve_with_startup_retry(
    resolver: &Resolver,
    name: &str,
    budget: Duration,
) -> Result<SecretValue, SecretError> {
    resolve_with_startup_retry_observed(resolver, name, budget, |_| {})
}

/// Resolve `name` through `resolver`, retrying until `budget`
/// elapses. Startup only: a co-hosted or freshly started
/// secrets-broker binds before its first vault refresh completes
/// and answers 503 while its cache is cold, so a one-shot resolve
/// races its readiness and the whole serve list exits (#8394
/// Codex P2). Bounded: once the budget is spent the last error is
/// returned and the caller fails exactly as the one-shot did.
/// Each failed attempt is passed to `observe` (#12247), so the
/// broker can say why its backend is not ready yet.
fn resolve_with_startup_retry_observed(
    resolver: &Resolver,
    name: &str,
    budget: Duration,
    observe: impl Fn(&SecretError),
) -> Result<SecretValue, SecretError> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        match resolver.resolve(name) {
            Ok(v) => return Ok(v),
            Err(e) => {
                observe(&e);
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

#[cfg(test)]
mod reprioritize_sync_tests {
    /// T3/T4 contract pins (#14100): the GitHub sync leg runs after
    /// the queue commit, touches only priority/* labels, sets
    /// lineage.priority only inside an existing pm-metadata block,
    /// and records failures as pending + outbox without rolling
    /// back the queue change.
    const SRC: &str = include_str!("queue_broker.rs");

    fn sync_fn() -> &'static str {
        let start = SRC
            .find("fn sync_reprioritized")
            .expect("sync_reprioritized must exist");
        let rest = &SRC[start..];
        let end = rest
            .find("\nfn retry_reprioritize_sync")
            .expect("retry fn follows");
        &rest[..end]
    }

    #[test]
    fn sync_touches_only_priority_labels() {
        let f = sync_fn();
        // Removal is scoped to priority/* labels, add is the mapped one.
        assert!(f.contains("starts_with(\"priority/\")"), "{f}");
        // expedite/next-up are never in the sync write set.
        assert!(!f.contains("expedite"), "must not touch expedite");
        assert!(!f.contains("next-up"), "must not touch next-up");
    }

    #[test]
    fn sync_sets_lineage_only_inside_an_existing_block() {
        let f = sync_fn();
        assert!(f.contains("pm-metadata:start"), "reads existing block");
        assert!(
            f.contains("[\"lineage\"][\"priority\"]"),
            "sets lineage.priority"
        );
        assert!(
            f.contains("get(\"lineage\").is_some_and(Value::is_object)"),
            "never creates a block speculatively"
        );
    }

    #[test]
    fn queue_change_never_rolls_back_and_failure_is_recorded() {
        // The sync runs only after a committed queue write.
        assert!(
            SRC.contains("if path == \"/reprioritize\" && resp.get(\"ok\")"),
            "sync is post-commit only"
        );
        let f = sync_fn();
        // Failure path: pending event + durable outbox row, and the
        // success path clears the visible marker only when no newer
        // reprioritize landed meanwhile.
        assert!(f.contains("reprioritize_sync_pending"), "pending event");
        assert!(
            f.contains("INSERT INTO reprioritize_sync_outbox"),
            "durable intent"
        );
        assert!(
            f.contains("still_current"),
            "no stale clear of a newer intent"
        );
        // The retry runs ahead of serving, from the latest task row.
        assert!(SRC.contains("retry_reprioritize_sync(st, turso)"));
    }
}

#[cfg(test)]
mod history_since_cursor_contract_tests {
    use super::*;

    // Oracle: the PM projector resumes from a task_history id cursor,
    // receives rows in strictly increasing id order, and knows whether
    // another page remains.
    const SRC: &str = include_str!("queue_broker.rs");

    #[test]
    fn history_since_accepts_after_id_and_returns_page_cursor() {
        let marker = ["fn dispatch_history_", "since_cursor("].concat();
        let start = SRC
            .find(&marker)
            .expect("queue-broker cursor route must exist");
        let rest = &SRC[start..];
        let end = rest
            .find("\nfn dispatch(")
            .expect("cursor route must end at the next function");
        let route = &rest[..end];
        assert!(
            route.contains("id > ?"),
            "cursor excludes ids already seen: {route}"
        );
        assert!(
            route.contains("ORDER BY id ASC"),
            "pages are stable: {route}"
        );
        assert!(
            route.contains("next_cursor"),
            "the client can advance: {route}"
        );
        assert!(
            route.contains("has_more"),
            "the client can drain another page: {route}"
        );
        assert!(
            SRC.contains("path == \"/history-since\" && effective_req.get(\"after_id\").is_some()"),
            "after_id requests use the monotonic cursor route"
        );
    }

    #[test]
    fn cursor_pages_advance_without_replaying_seen_rows() {
        let first = history_cursor_page(
            vec![
                json!({"id": 42, "task_id": 8}),
                json!({"id": 40, "task_id": 7}),
                json!({"id": 41, "task_id": 7}),
            ],
            39,
            2,
            None,
        );
        assert_eq!(
            first["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![40, 41],
            "oracle: an ID cursor returns the next two unseen history rows in ascending order"
        );
        assert_eq!(first["next_cursor"], 41);
        assert_eq!(first["has_more"], true);

        let second = history_cursor_page(
            vec![json!({"id": 40}), json!({"id": 42})],
            first["next_cursor"].as_i64().unwrap(),
            2,
            None,
        );
        assert_eq!(second["rows"], json!([{"id": 42}]));
        assert_eq!(second["next_cursor"], 42);
        assert_eq!(second["has_more"], false);
    }
}
