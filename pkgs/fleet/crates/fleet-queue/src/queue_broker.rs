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
//!      QUEUE_BROKER_SECRETS_TOKEN_FILE (default ~/.config/secrets-broker/token).
//!
//! Backend: the queue core on the dedicated Turso DB `fleet-queue`
//! (crate::turso_queue), at QUEUE_BROKER_TURSO_URL with the token
//! from QUEUE_BROKER_TURSO_TOKEN_FILE or the secrets broker. Turso is
//! the only backend: QUEUE_BROKER_BACKEND may be unset or `turso`;
//! any other value is a startup config error (exit 3).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fleet_secrets::{Resolver, SecretError, SecretSource, SecretValue};
use serde_json::{Value, json};
use tiny_http::{Header, Response, Server, StatusCode};

struct State {
    secrets_url: String,
    secrets_token: String,
    agent: ureq::Agent,
    /// Filled by the startup thread once the Turso token resolves and
    /// the schema is ensured (#12247); until then queue verbs answer
    /// exit 3 with `turso_pending`.
    turso: std::sync::OnceLock<crate::turso_queue::TursoBackend>,
    /// Why the backend is not ready yet: the latest startup failure.
    turso_pending: Mutex<String>,
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
fn maybe_raise_needs_drew(st: &State, req: &Value, resp: &Value) {
    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return;
    }
    if req.get("reason_class").and_then(|v| v.as_str()) != Some("escalated-to-user") {
        return;
    }
    let blocker = req.get("blocker").and_then(|v| v.as_str()).unwrap_or("");
    let Some(nd) = crate::needs_drew::from_block_request(req, blocker) else {
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
            .resolve(crate::needs_drew::ALERTS_INTAKE_TOKEN_SECRET)
            .map_err(|e| e.to_string())
            .and_then(|token| crate::needs_drew::post_needs_drew(token.expose(), &nd));
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
    let info = crate::needs_drew::ExtensionAlert {
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
    let body = crate::needs_drew::build_extension_alert_body(&info);
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
            .resolve(crate::needs_drew::ALERTS_INTAKE_TOKEN_SECRET)
            .map_err(|e| e.to_string())
            .and_then(|token| {
                crate::needs_drew::post_plain_alert_to(
                    &crate::needs_drew::alerts_url(),
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
        "/healthz" => ok(json!({"service": "queue-broker"})),
        _ => {
            let Some(turso) = st.turso.get() else {
                let why = st
                    .turso_pending
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                return fail(3, format!("queue backend not ready: {why}"));
            };
            let resp = crate::turso_queue::dispatch(turso, path, req);
            // #8568: the broker-side needs-drew raise for an
            // escalated block (fire-and-forget).
            if path == "/block" {
                maybe_raise_needs_drew(st, req, &resp);
            }
            // #13774: the emergency-extension needs-drew raise.
            if path == "/actions-exec-extend-request" {
                maybe_raise_actions_exec_extension(st, &resp);
            }
            resp
        }
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
    } else if !actions_exec_agent_call(path, provided_token)
        && !token_matches(provided_token, token)
    {
        (401, json!({"error": "unauthorized"}))
    } else if method != "POST" {
        (405, json!({"error": "POST only"}))
    } else {
        match std::str::from_utf8(body) {
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
        turso: std::sync::OnceLock::new(),
        turso_pending: Mutex::new("starting".to_string()),
    });

    let server = match Server::http(&bind) {
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
