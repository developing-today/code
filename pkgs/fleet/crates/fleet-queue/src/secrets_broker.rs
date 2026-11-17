//! secrets-broker — resident Bitwarden Secrets Manager cache (code#5362).
//!
//! Fetches the vault's secrets ONCE per TTL via the `bws` CLI on the
//! laptop and serves them to token-authed local consumers, replacing the
//! per-call `bws secret list` + `get` fetch that made every
//! repo-queue.py invocation pay 10-40 s. Binds the laptop tailnet IP
//! only; consumers reach it through the sandbox session proxy.
//!
//! Endpoints (header `X-Agent-Token` required except /health):
//!   GET /health        -> {ok, serving, secrets, last_refresh_unix, refreshing,
//!                          oldest_secret_age_seconds} | 503 while `serving`
//!                          is false (no successful refresh yet, #10868)
//!   GET /secrets       -> {names: [...]}
//!   GET /secret/<name> -> {key, value, fetched_at_unix, age_seconds,
//!                          source} | 404 | 503 (not loaded yet)
//!
//! Every cached value carries the time the broker last fetched it
//! from Bitwarden (#6767), and /secret/<name> exposes that age plus
//! the value's source (cache vs a just-now read-through), so a
//! rotation runbook can tell whether it is verifying the broker or
//! Bitwarden itself. A refresh installs per secret: fetches that
//! succeed land immediately, a secret whose fetch fails keeps its
//! previous value and age, and deletions follow the authoritative
//! list — one broken fetch can no longer block every rotation in
//! the vault the way the fail-whole install did (#6767, cf. #5504,
//! whose no-eviction rule the per-secret install preserves). Only a
//! refresh in which EVERY fetch fails keeps the whole previous
//! cache untouched. A rotation therefore lands within one TTL of
//! the edit, bounded and observable via the exposed age.
//!
//! A miss on a loaded cache is not final: the broker reads through
//! to Bitwarden for that one name (single-flighted per name) and
//! serves what it finds, so a secret minted moments ago is visible
//! immediately instead of at the next TTL refresh (#6275).
//!
//! Env: SECRETS_BROKER_BIND (default 100.99.113.103:4800),
//!      SECRETS_BROKER_SOCKET (Unix socket dual-bind, #6216; default
//!      ~/.config/secrets-broker/broker.sock via fleet-config),
//!      SECRETS_BROKER_TOKEN_FILE (default ~/.config/secrets-broker/token),
//!      SECRETS_BROKER_TTL_S (default 900, minimum 30 — validated
//!      at startup, #5860).

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tiny_http::{Header, Response, Server, StatusCode};

/// One cached secret: its value plus when the broker last fetched
/// it from Bitwarden (#6767). The age is what lets a consumer (or a
/// rotation runbook) distinguish "the broker is serving a value N
/// seconds old" from "Bitwarden itself holds this value".
#[derive(Clone, Debug, PartialEq, Eq)]
struct CachedSecret {
    value: String,
    fetched_at_unix: u64,
}

struct State {
    secrets: RwLock<HashMap<String, CachedSecret>>,
    last_refresh: RwLock<Option<u64>>,
    /// The most recent vault-refresh failure, cleared by the next
    /// successful refresh. Served on the not-loaded 503 so a client
    /// can see WHY the broker has no secrets (#10743) instead of
    /// retry-looping a bare "not loaded yet". Status-only text by
    /// construction (BwsRunner discipline): never vault material.
    last_refresh_error: RwLock<Option<String>>,
    refreshing: RwLock<bool>,
    /// In-flight read-through fetches, one gate per name (#6275):
    /// the first misser fetches from Bitwarden; concurrent missers
    /// for the same name wait on the gate instead of stampeding the
    /// vault with duplicate list+get pairs.
    inflight: Mutex<HashMap<String, Arc<FetchGate>>>,
}

/// The outcome slot of one in-flight read-through fetch: the leader
/// stores its result and notifies; waiters clone the result out.
struct FetchGate {
    result: Mutex<Option<Result<Option<String>, String>>>,
    done: Condvar,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn bws_token() -> Option<String> {
    if let Ok(t) = std::env::var("BWS_ACCESS_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    let home = crate::home::from_env().ok()?;
    for rel in [".config/bitwarden/access-token", ".config/bws/access-token"] {
        if let Ok(t) = std::fs::read_to_string(format!("{home}/{rel}")) {
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

/// Deadline for one `bws` child (#10871). A child that never returns
/// must fail its fetch, or its worker never finishes, the refresh's
/// join never returns, and the `refreshing` guard stays set forever.
const BWS_CHILD_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `bws` for the vault refresh, through the platform crate's
/// CommandRunner discipline (fleet-secrets, design §1): stdout on
/// success; on failure the error carries the exit status only —
/// never stderr, which could quote vault material. (The standalone
/// broker included a trimmed stderr excerpt in this log-only error;
/// the port deliberately adopts the stricter platform rule.)
struct BwsRunner {
    token: String,
    timeout: Duration,
}

impl fleet_secrets::CommandRunner for BwsRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        // A bare `bws` spawn dies with NotFound when the broker was
        // launched under a stripped PATH (#10743); resolve through
        // the well-known install locations instead.
        let resolved;
        let program: &std::path::Path = if program == "bws" {
            resolved = fleet_secrets::resolve_bws_program();
            &resolved
        } else {
            std::path::Path::new(program)
        };
        let mut child = Command::new(program)
            .args(args)
            .env("BWS_ACCESS_TOKEN", &self.token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn failed: {}", e.kind()))?;
        let mut pipe = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(out) = pipe.as_mut() {
                let _ = std::io::Read::read_to_end(out, &mut buf);
            }
            buf
        });
        let deadline = std::time::Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", self.timeout.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(format!("wait failed: {}", e.kind())),
            }
        };
        let stdout = reader.join().unwrap_or_default();
        if !status.success() {
            return Err(status.to_string());
        }
        String::from_utf8(stdout).map_err(|_| "output not utf8".to_string())
    }
}

fn bws_run(args: &[&str], token: &str) -> Result<String, String> {
    let runner = BwsRunner {
        token: token.to_string(),
        timeout: BWS_CHILD_TIMEOUT,
    };
    fleet_secrets::CommandRunner::run(&runner, "bws", args)
        .map_err(|e| format!("bws {} failed: {e}", args.join(" ")))
}

/// One full vault refresh: list, then get each secret (8-way parallel).
fn refresh(state: &Arc<State>) -> Result<usize, String> {
    {
        let mut r = state.refreshing.write().unwrap();
        if *r {
            return Err("refresh already in progress".into());
        }
        *r = true;
    }
    let result = do_refresh(state);
    // The refresh-error bookkeeping lives here, in the one shared
    // operation, so every caller records it — the background loop
    // AND the manual POST /refresh path, whose caller discards the
    // result (CodeRabbit review, #10757). The guard rejection
    // above returns before this point and never overwrites the
    // recorded error: a rejected overlap is not an attempt.
    match &result {
        Ok(_) => *state.last_refresh_error.write().unwrap() = None,
        Err(e) => *state.last_refresh_error.write().unwrap() = Some(e.clone()),
    }
    *state.refreshing.write().unwrap() = false;
    result
}

/// Parse the `bws secret list` entries strictly: every entry must
/// carry a string `id` and a string `key`. A malformed entry fails
/// the whole parse (#6568): silently dropping it here would install
/// the partial listing as the whole cache and evict a secret that
/// exists, defeating the #5504 fail-whole fetch count below — the
/// count can only see entries this parse kept.
fn parse_list_entries(list: &Value) -> Result<Vec<(String, String)>, String> {
    let arr = list.as_array().ok_or("bws list is not an array")?;
    let mut items = Vec::with_capacity(arr.len());
    for (i, s) in arr.iter().enumerate() {
        let id = s
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("bws list entry {i} has a missing or non-string id"))?;
        let key = s
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("bws list entry {i} has a missing or non-string key"))?;
        items.push((id.to_string(), key.to_string()));
    }
    Ok(items)
}

/// Resolve one key to its id in a `bws secret list` payload.
/// Ok(None) is a definite "not in the vault"; a key-matching entry
/// whose id is missing or non-string is a source failure (Err), not
/// an absence (#6568) — reporting it as not-in-vault would send
/// consumers a 404 for a secret that exists.
fn resolve_list_id(list: &Value, name: &str) -> Result<Option<String>, String> {
    let arr = list.as_array().ok_or("bws list is not an array")?;
    for s in arr {
        if s.get("key").and_then(|k| k.as_str()) == Some(name) {
            return match s.get("id").and_then(|i| i.as_str()) {
                Some(id) => Ok(Some(id.to_string())),
                None => Err(format!(
                    "bws list entry for key {name:?} has a missing or non-string id"
                )),
            };
        }
    }
    Ok(None)
}

fn do_refresh(state: &Arc<State>) -> Result<usize, String> {
    let token = bws_token().ok_or("no Bitwarden access token available")?;
    let list_raw = bws_run(&["secret", "list", "-o", "json"], &token)?;
    let list: Value =
        serde_json::from_str(&list_raw).map_err(|e| format!("bws list not json: {e}"))?;
    let items: Vec<(String, String)> = parse_list_entries(&list)?;
    // Fetch values with a small worker pool over a shared index.
    // Every per-item failure is counted, and the install below is
    // per secret (#6767): successes land, failures keep their
    // secret's previous value — the #5504 no-eviction rule, applied
    // per secret instead of failing the whole refresh.
    let total = items.len();
    let items = Arc::new(items);
    let idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let got = Arc::new(RwLock::new(HashMap::new()));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let items = Arc::clone(&items);
        let idx = Arc::clone(&idx);
        let failed = Arc::clone(&failed);
        let got = Arc::clone(&got);
        let token = token.clone();
        handles.push(std::thread::spawn(move || {
            loop {
                let i = idx.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= items.len() {
                    return;
                }
                let (id, key) = &items[i];
                let fetched = bws_run(&["secret", "get", id, "-o", "json"], &token)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|v| v.get("value").and_then(|x| x.as_str()).map(str::to_string));
                match fetched {
                    Some(val) => {
                        // Stamp the value with ITS fetch time, not
                        // the batch's install time (Codex, PR #8491):
                        // a slow sibling fetch must not make this
                        // value look fresher than it is.
                        got.write().unwrap().insert(key.clone(), (val, now_unix()));
                    }
                    None => {
                        failed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let failed = failed.load(std::sync::atomic::Ordering::SeqCst);
    let fetched = got.read().unwrap().clone();
    // Install per secret (#6767): the successes land, a failed fetch
    // keeps its secret's previous value, and only a refresh in which
    // EVERY fetch failed keeps the whole previous cache (the
    // fail-whole tail this replaces let one broken fetch block every
    // rotation in the vault indefinitely).
    match install_refresh(state, &items, fetched, now_unix()) {
        Some(outcome) => {
            if outcome.kept_failed > 0 {
                crate::log::info(
                    "secrets-broker",
                    &format!(
                        "partial refresh: {failed} of {total} fetches failed; kept the previous values for {} secrets",
                        outcome.kept_failed
                    ),
                );
            }
            Ok(outcome.total)
        }
        None => Err(format!(
            "{failed} of {total} secret fetches failed; keeping the previous cache"
        )),
    }
}

/// What one refresh installed (#6767): how many listed secrets got a
/// fresh value, how many kept their previous value because their
/// fetch failed, how many cached names the authoritative list no
/// longer carries, and the cache size after the install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InstallOutcome {
    installed: usize,
    kept_failed: usize,
    removed: usize,
    total: usize,
}

/// Install one refresh's fetches into the cache, per secret (#6767).
///
/// - a listed secret whose fetch succeeded takes the new value and
///   its own fetch timestamp;
/// - a listed secret whose fetch failed keeps its previous value
///   AND its previous timestamp — a failure never evicts and never
///   makes a stale value look fresh (#5504's rule, per secret);
/// - a cached name absent from the list is removed: the list is
///   authoritative for existence (it parsed strictly, #6568, so a
///   name is absent only because the vault no longer carries it);
/// - if the list is non-empty but NOT ONE fetch succeeded, the
///   refresh is a source outage, not news about the vault: nothing
///   is installed or removed and the caller keeps the whole
///   previous cache (returns None).
fn install_refresh(
    state: &Arc<State>,
    listed: &[(String, String)],
    fetched: HashMap<String, (String, u64)>,
    now: u64,
) -> Option<InstallOutcome> {
    if fetched.is_empty() && !listed.is_empty() {
        return None;
    }
    let listed_keys: std::collections::HashSet<&str> =
        listed.iter().map(|(_, key)| key.as_str()).collect();
    let mut installed = 0;
    let mut kept_failed = 0;
    let (removed, total) = {
        let mut secrets = state.secrets.write().unwrap();
        let doomed: Vec<String> = secrets
            .keys()
            .filter(|k| !listed_keys.contains(k.as_str()))
            .cloned()
            .collect();
        let removed = doomed.len();
        for k in &doomed {
            secrets.remove(k);
        }
        for (_, key) in listed {
            match fetched.get(key) {
                Some((v, at)) => {
                    secrets.insert(
                        key.clone(),
                        CachedSecret {
                            value: v.clone(),
                            fetched_at_unix: *at,
                        },
                    );
                    installed += 1;
                }
                None => {
                    if secrets.contains_key(key) {
                        kept_failed += 1;
                    }
                }
            }
        }
        (removed, secrets.len())
    };
    *state.last_refresh.write().unwrap() = Some(now);
    Some(InstallOutcome {
        installed,
        kept_failed,
        removed,
        total,
    })
}

/// One name, straight from Bitwarden: resolve the key to its id via
/// the list, then get the value — the same two calls the full
/// refresh makes, scoped to a single secret. Ok(None) is a definite
/// "not in the vault"; Err is a source failure (its detail carries
/// the exit status only, per BwsRunner's discipline).
fn fetch_one(name: &str) -> Result<Option<String>, String> {
    let token = bws_token().ok_or("no Bitwarden access token available")?;
    let list_raw = bws_run(&["secret", "list", "-o", "json"], &token)?;
    let list: Value =
        serde_json::from_str(&list_raw).map_err(|e| format!("bws list not json: {e}"))?;
    let id = resolve_list_id(&list, name)?;
    let Some(id) = id else {
        return Ok(None);
    };
    let got_raw = bws_run(&["secret", "get", &id, "-o", "json"], &token)?;
    let got: Value =
        serde_json::from_str(&got_raw).map_err(|e| format!("bws get not json: {e}"))?;
    match got.get("value").and_then(|v| v.as_str()) {
        Some(v) => Ok(Some(v.to_string())),
        None => Err("bws get response has no string value".to_string()),
    }
}

/// Read-through on a cache miss (#6275; Drew 2026-11-05: "broker
/// should go get the secret pass through"): fetch the one name from
/// Bitwarden and install it in the cache, so a just-minted secret
/// resolves immediately. Only the fetch's success is cached — a
/// not-found or a failure is returned but never installed, so the
/// next miss retries against the source.
fn fetch_through(state: &Arc<State>, name: &str) -> Result<Option<String>, String> {
    fetch_through_with(state, name, &fetch_one)
}

fn fetch_through_with(
    state: &Arc<State>,
    name: &str,
    fetch: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    // A refresh (or an earlier fetch) may have landed the value
    // since the caller missed.
    if let Some(c) = state.secrets.read().unwrap().get(name) {
        return Ok(Some(c.value.clone()));
    }
    // Join the in-flight fetch for this name, or become its leader.
    let (gate, leader) = {
        let mut inflight = state.inflight.lock().unwrap();
        match inflight.get(name) {
            Some(g) => (Arc::clone(g), false),
            None => {
                let g = Arc::new(FetchGate {
                    result: Mutex::new(None),
                    done: Condvar::new(),
                });
                inflight.insert(name.to_string(), Arc::clone(&g));
                (g, true)
            }
        }
    };
    if !leader {
        let mut result = gate.result.lock().unwrap();
        while result.is_none() {
            result = gate.done.wait(result).unwrap();
        }
        return result.clone().unwrap();
    }
    let result = fetch(name);
    match &result {
        Ok(Some(v)) => {
            state.secrets.write().unwrap().insert(
                name.to_string(),
                CachedSecret {
                    value: v.clone(),
                    fetched_at_unix: now_unix(),
                },
            );
            crate::log::info(
                "secrets-broker",
                &format!("read-through: fetched {name:?} from Bitwarden on a cache miss"),
            );
        }
        Ok(None) => crate::log::info(
            "secrets-broker",
            &format!("read-through: {name:?} is not in Bitwarden"),
        ),
        Err(e) => crate::log::info(
            "secrets-broker",
            &format!("read-through: fetch of {name:?} failed: {e}"),
        ),
    }
    *gate.result.lock().unwrap() = Some(result.clone());
    gate.done.notify_all();
    state.inflight.lock().unwrap().remove(name);
    result
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
/// read. Returns the refusal to send, or `None` to proceed to the
/// body. The only unauthenticated route is the health probe,
/// mirroring `handle_request`.
fn pre_body_refusal(
    method: &str,
    target: &str,
    provided_token: Option<&str>,
    token: &str,
    content_length: Option<u64>,
) -> Option<(u16, Value)> {
    let path = target.split('?').next().unwrap_or("");
    if method == "GET" && path == "/health" {
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
/// transports by construction. Returns (status, JSON body). The
/// request body is part of the shared shape but unused here: the one
/// body-carrying route, POST /refresh, ignores its body.
fn handle_request(
    state: &Arc<State>,
    token: &str,
    method: &str,
    url: &str,
    provided_token: Option<&str>,
) -> (u16, Value) {
    let path = url.split('?').next().unwrap_or("").to_string();
    if method == "GET" && path == "/health" {
        let now = now_unix();
        let oldest_secret_age_seconds = state
            .secrets
            .read()
            .unwrap()
            .values()
            .map(|c| now.saturating_sub(c.fetched_at_unix))
            .max();
        let last_refresh_unix = state.last_refresh.read().unwrap().clone();
        let serving = last_refresh_unix.is_some();
        (
            if serving { 200 } else { 503 },
            json!({
                "ok": serving,
                "serving": serving,
                "secrets": state.secrets.read().unwrap().len(),
                "last_refresh_unix": last_refresh_unix,
                "refreshing": *state.refreshing.read().unwrap(),
                "oldest_secret_age_seconds": oldest_secret_age_seconds,
            }),
        )
    } else if !token_matches(provided_token, token) {
        (401, json!({"error": "unauthorized"}))
    } else if method == "GET" && path == "/secrets" {
        let mut names: Vec<String> = state.secrets.read().unwrap().keys().cloned().collect();
        names.sort();
        (200, json!({"names": names}))
    } else if method == "POST" && path == "/refresh" {
        let st = Arc::clone(state);
        std::thread::spawn(move || {
            let _ = refresh(&st);
        });
        (202, json!({"ok": true, "refresh": "started"}))
    } else if method == "GET" && path.starts_with("/secret/") {
        let name = path.trim_start_matches("/secret/").to_string();
        let name = url_decode(&name);
        let cached = state.secrets.read().unwrap().get(&name).cloned();
        match cached {
            Some(c) => (
                200,
                json!({
                    "key": name,
                    "value": c.value,
                    "fetched_at_unix": c.fetched_at_unix,
                    "age_seconds": now_unix().saturating_sub(c.fetched_at_unix),
                    "source": "cache",
                }),
            ),
            None => {
                if state.last_refresh.read().unwrap().is_none() {
                    let mut body = json!({"error": "secrets not loaded yet"});
                    if let Some(e) = state.last_refresh_error.read().unwrap().clone() {
                        body["refresh_error"] = json!(e);
                    }
                    (503, body)
                } else {
                    // A miss on a loaded cache reads through to
                    // Bitwarden for this one name (#6275) before the
                    // name is declared absent: a secret minted since
                    // the last refresh resolves now, not at the next
                    // TTL tick. A source failure is a loud 502, never
                    // a silent 404-as-if-absent.
                    match fetch_through(state, &name) {
                        Ok(Some(v)) => (
                            200,
                            json!({
                                "key": name,
                                "value": v,
                                "fetched_at_unix": now_unix(),
                                "age_seconds": 0,
                                "source": "bitwarden-read-through",
                            }),
                        ),
                        Ok(None) => (404, json!({"error": "no such secret"})),
                        Err(_) => (502, json!({"error": "bitwarden read-through failed"})),
                    }
                }
            }
        }
    } else {
        (404, json!({"error": "not found"}))
    }
}

/// Default vault-refresh interval when SECRETS_BROKER_TTL_S is unset.
pub const DEFAULT_TTL_S: u64 = 900;

/// Smallest accepted SECRETS_BROKER_TTL_S. One interval costs a full
/// vault refresh (one `bws secret list` plus one `bws secret get` per
/// secret); below this floor the loop spends its life inside bws — at 0
/// it never sleeps at all (#5860 measured ~110 full refreshes/sec).
/// Startup refuses smaller values rather than clamping: a broker
/// silently running at a different rate than configured is its own
/// surprise on the credential path.
pub const MIN_TTL_S: u64 = 30;

/// The outcome of parsing SECRETS_BROKER_TTL_S: the interval to run
/// with, plus a warning the caller must surface when the operator's
/// value could not be honored as written.
#[derive(Debug)]
pub struct TtlParse {
    pub ttl_s: u64,
    pub warning: Option<String>,
}

/// Parse and validate SECRETS_BROKER_TTL_S (#5860). Pure, so the policy
/// is pinned by unit tests:
/// - unset -> DEFAULT_TTL_S, no warning (the documented default);
/// - unparseable -> DEFAULT_TTL_S with a warning naming the value —
///   never the old silent fallback;
/// - parsed but below MIN_TTL_S (including 0) -> Err naming the
///   variable, the value, and the minimum; startup refuses to run.
pub fn parse_ttl_s(raw: Option<&str>) -> Result<TtlParse, String> {
    let Some(raw) = raw else {
        return Ok(TtlParse {
            ttl_s: DEFAULT_TTL_S,
            warning: None,
        });
    };
    match raw.trim().parse::<u64>() {
        Ok(v) if v < MIN_TTL_S => Err(format!(
            "SECRETS_BROKER_TTL_S={raw:?} is below the minimum of {MIN_TTL_S}s: one interval is a full vault refresh, and 0 spins that loop with no sleep at all (#5860) — refusing to start"
        )),
        Ok(v) => Ok(TtlParse {
            ttl_s: v,
            warning: None,
        }),
        Err(_) => Ok(TtlParse {
            ttl_s: DEFAULT_TTL_S,
            warning: Some(format!(
                "SECRETS_BROKER_TTL_S={raw:?} is not a valid number of seconds; using the default of {DEFAULT_TTL_S}s"
            )),
        }),
    }
}

pub fn run() -> i32 {
    use std::io::Read as _;
    // Stage-2 broker resilience: on the sandbox the broker is a
    // local capability, never a LAN surface — force loopback
    // regardless of the environment, so a stale or widened
    // SECRETS_BROKER_BIND can never expose the vault off-host.
    // The spawn in `fleet init` sets the loopback bind explicitly;
    // this is the second enforcement, at the bind itself.
    let sandbox = std::env::var("FLEET_LOCATION")
        .ok()
        .is_some_and(|v| v.trim() == "sandbox");
    let bind = if sandbox {
        "127.0.0.1:4800".to_string()
    } else {
        std::env::var("SECRETS_BROKER_BIND").unwrap_or_else(|_| "100.99.113.103:4800".to_string())
    };
    let home = match crate::home::from_env() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("secrets-broker: {error}");
            return 2;
        }
    };
    let token_file = std::env::var("SECRETS_BROKER_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/secrets-broker/token"));
    let token = match crate::token_file::load_token_file(&token_file) {
        Ok(v) => v,
        Err(e) => {
            crate::log::info("secrets-broker", &e);
            return 3;
        }
    };
    let ttl_s = match parse_ttl_s(std::env::var("SECRETS_BROKER_TTL_S").ok().as_deref()) {
        Ok(parsed) => {
            if let Some(warning) = &parsed.warning {
                crate::log::info("secrets-broker", warning);
                eprintln!("secrets-broker: {warning}");
            }
            parsed.ttl_s
        }
        Err(error) => {
            eprintln!("secrets-broker: {error}");
            return 2;
        }
    };

    let state = Arc::new(State {
        secrets: RwLock::new(HashMap::new()),
        last_refresh: RwLock::new(None),
        last_refresh_error: RwLock::new(None),
        refreshing: RwLock::new(false),
        inflight: Mutex::new(HashMap::new()),
    });

    // Background refresh loop: initial load + TTL refresh,
    // stale-while-revalidate (the cache keeps serving during a refresh,
    // and a failed refresh keeps the old values).
    {
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            loop {
                match refresh(&state) {
                    Ok(n) => crate::log::info("secrets-broker", &format!("refreshed {n} secrets")),
                    Err(e) => crate::log::info("secrets-broker", &format!("refresh failed: {e}")),
                }
                std::thread::sleep(Duration::from_secs(ttl_s));
            }
        });
    }

    let server = match Server::http(&bind) {
        Ok(server) => server,
        Err(e) => {
            crate::log::info("secrets-broker", &format!("cannot bind {bind}: {e:?}"));
            return 3;
        }
    };
    crate::log::info("secrets-broker", &format!("listening on {bind}"));

    // Dual-bind (#6216): the same handler over a Unix socket. A bind
    // failure or a live owner degrades to TCP only, never to exit:
    // the TCP listener is the established surface.
    let socket_path = fleet_config::Config::load()
        .unwrap_or_default()
        .secrets_broker_socket();
    match crate::uds_http::bind_socket(&socket_path) {
        Ok(Some(listener)) => {
            let st = Arc::clone(&state);
            let tok = token.clone();
            crate::log::info(
                "secrets-broker",
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
            "secrets-broker",
            &format!(
                "uds {} already owned by a live broker; TCP only",
                socket_path.display()
            ),
        ),
        Err(e) => crate::log::info(
            "secrets-broker",
            &format!("cannot bind uds {}: {e}; TCP only", socket_path.display()),
        ),
    }

    for mut req in server.incoming_requests() {
        let state = Arc::clone(&state);
        let token = token.clone();
        std::thread::spawn(move || {
            let url = req.url().to_string();
            let provided = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "x-agent-token")
                .map(|h| h.value.as_str().to_string());
            // #6515: authenticate and size-check from the headers
            // before draining any body byte (tiny_http 0.12 exposes
            // no per-connection read timeout; see queue_broker).
            let content_length = req
                .headers()
                .iter()
                .find(|h| h.field.as_str().to_ascii_lowercase() == "content-length")
                .and_then(|h| h.value.as_str().parse::<u64>().ok());
            if let Some((status, value)) = pre_body_refusal(
                req.method().as_str(),
                &url,
                provided.as_deref(),
                &token,
                content_length,
            ) {
                let _ = req.respond(json_response(status, value));
                return;
            }
            if req.method().as_str() == "POST" {
                // POST /refresh's body is ignored by the handler;
                // drain it (capped, #6515) as the pre-#6216 loop did.
                let cap = (crate::uds_http::MAX_BODY_BYTES + 1) as u64;
                let mut buf = Vec::new();
                let _ = req.as_reader().take(cap).read_to_end(&mut buf);
                if buf.len() > crate::uds_http::MAX_BODY_BYTES {
                    let _ = req.respond(json_response(
                        413,
                        json!({"error": "request body too large"}),
                    ));
                    return;
                }
            }
            let (status, value) = handle_request(
                &state,
                &token,
                req.method().as_str(),
                &url,
                provided.as_deref(),
            );
            let _ = req.respond(json_response(status, value));
        });
    }
    // incoming_requests() ends only when the server stops.
    0
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        // A path segment is not a form query: '+' is a literal plus
        // here (RFC 3986), never a space. Only %XX decodes (issue #5584).
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        CachedSecret, DEFAULT_TTL_S, MIN_TTL_S, State, fetch_through_with, handle_request,
        install_refresh, now_unix, parse_list_entries, parse_ttl_s, pre_body_refusal,
        resolve_list_id, url_decode,
    };
    use std::time::Duration;

    #[test]
    fn pre_body_refusal_decides_from_headers_alone() {
        // #6515 oracle (thread 4492609604): the gate's answer
        // depends only on method/target/token/declared length —
        // never on body bytes — so an unauthorized or oversized
        // request is refused before its body is read.
        let over = crate::uds_http::MAX_BODY_BYTES as u64 + 1;
        assert_eq!(
            pre_body_refusal("POST", "/refresh", None, "tok", Some(over)).map(|r| r.0),
            Some(401)
        );
        assert_eq!(
            pre_body_refusal("POST", "/refresh", Some("tok"), "tok", Some(over)).map(|r| r.0),
            Some(413)
        );
        assert_eq!(
            pre_body_refusal("POST", "/refresh", Some("tok"), "tok", Some(0)),
            None
        );
        // The health probe stays unauthenticated, as in handle_request.
        assert_eq!(pre_body_refusal("GET", "/health", None, "tok", None), None);
    }
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, RwLock};

    fn empty_state() -> Arc<State> {
        Arc::new(State {
            secrets: RwLock::new(HashMap::new()),
            last_refresh: RwLock::new(Some(1)),
            last_refresh_error: RwLock::new(None),
            refreshing: RwLock::new(false),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    #[test]
    fn health_reports_serving_only_after_a_successful_refresh() {
        // oracle (#10868): a live process whose refresh never
        // succeeded (stripped PATH, bws unspawnable) is not serving,
        // so the health probe must fail and init-ensure can replace
        // it; a loaded broker answers 200 with serving=true.
        let state = empty_state();
        *state.last_refresh.write().unwrap() = None;
        let (status, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(status, 503);
        assert_eq!(body["serving"], false);
        assert_eq!(body["ok"], false);
        assert!(body.get("refresh_error").is_none());

        *state.last_refresh.write().unwrap() = Some(1);
        let (status, body) = handle_request(&state, "tok", "GET", "/health", None);
        assert_eq!(status, 200);
        assert_eq!(body["serving"], true);
        assert_eq!(body["ok"], true);
    }

    #[test]
    fn not_loaded_503_names_the_refresh_failure() {
        // oracle (#10743): a broker that has never loaded because
        // its refresh is failing says why on the 503, so a client
        // surfaces the cause instead of a bare "not loaded yet".
        let state = empty_state();
        *state.last_refresh.write().unwrap() = None;
        *state.last_refresh_error.write().unwrap() =
            Some("bws secret list -o json failed: spawn failed: entity not found".to_string());
        let (status, body) = handle_request(&state, "tok", "GET", "/secret/alpha", Some("tok"));
        assert_eq!(status, 503);
        assert_eq!(body["error"], "secrets not loaded yet");
        assert_eq!(
            body["refresh_error"],
            "bws secret list -o json failed: spawn failed: entity not found"
        );
    }

    #[test]
    fn read_through_installs_the_fetched_value_in_the_cache() {
        let state = empty_state();
        let calls = AtomicUsize::new(0);
        let fetch = |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some("fixture-value-not-a-secret".to_string()))
        };
        let v = fetch_through_with(&state, "new.name", &fetch).unwrap();
        assert_eq!(v.as_deref(), Some("fixture-value-not-a-secret"));
        assert_eq!(
            state
                .secrets
                .read()
                .unwrap()
                .get("new.name")
                .map(|c| c.value.as_str()),
            Some("fixture-value-not-a-secret")
        );
        // Second read is a cache hit: no further fetch.
        let v = fetch_through_with(&state, "new.name", &fetch).unwrap();
        assert_eq!(v.as_deref(), Some("fixture-value-not-a-secret"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn read_through_not_found_is_returned_but_never_cached() {
        let state = empty_state();
        let calls = AtomicUsize::new(0);
        let fetch = |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        };
        assert_eq!(fetch_through_with(&state, "absent.name", &fetch), Ok(None));
        assert!(!state.secrets.read().unwrap().contains_key("absent.name"));
        // No negative caching: the next miss asks the source again.
        assert_eq!(fetch_through_with(&state, "absent.name", &fetch), Ok(None));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn read_through_failure_is_returned_but_never_cached() {
        let state = empty_state();
        let fetch = |_: &str| Err("exit status 1".to_string());
        let err = fetch_through_with(&state, "any.name", &fetch).unwrap_err();
        assert_eq!(err, "exit status 1");
        assert!(!state.secrets.read().unwrap().contains_key("any.name"));
    }

    #[test]
    fn concurrent_misses_for_one_name_fetch_exactly_once() {
        let state = empty_state();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let state = Arc::clone(&state);
            let calls = Arc::clone(&calls);
            handles.push(std::thread::spawn(move || {
                let fetch = |_: &str| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    Ok(Some("fixture-value-not-a-secret".to_string()))
                };
                fetch_through_with(&state, "hot.name", &fetch)
            }));
        }
        for h in handles {
            let v = h.join().unwrap().unwrap();
            assert_eq!(v.as_deref(), Some("fixture-value-not-a-secret"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn url_decode_keeps_plus_literal_in_a_path_segment() {
        // A secret name containing + is client-legal (fleet-secrets
        // validate_name permits it) and reaches the broker raw; decoding
        // it to a space looked the name up under the wrong spelling.
        assert_eq!(url_decode("a+b"), "a+b");
        assert_eq!(url_decode("+"), "+");
    }

    #[test]
    fn url_decode_still_decodes_percent_escapes() {
        assert_eq!(url_decode("a%2Bb"), "a+b");
        assert_eq!(url_decode("a%20b"), "a b");
        assert_eq!(url_decode("alpha"), "alpha");
        // Malformed escapes stay literal.
        assert_eq!(url_decode("a%2"), "a%2");
        assert_eq!(url_decode("a%zz"), "a%zz");
    }

    #[test]
    fn parse_list_entries_keeps_well_formed_entries_in_order() {
        let list = serde_json::json!([
            {"id": "id-1", "key": "alpha", "value": "x"},
            {"id": "id-2", "key": "beta"}
        ]);
        assert_eq!(
            parse_list_entries(&list).unwrap(),
            vec![
                ("id-1".to_string(), "alpha".to_string()),
                ("id-2".to_string(), "beta".to_string())
            ]
        );
    }

    #[test]
    fn parse_list_entries_fails_whole_on_a_malformed_entry() {
        // #6568: any entry with a missing or non-string id or key
        // fails the parse, so a refresh can never install a partial
        // listing as the whole cache and evict a listed secret.
        for bad in [
            serde_json::json!([{"key": "alpha"}]),
            serde_json::json!([{"id": 7, "key": "alpha"}]),
            serde_json::json!([{"id": "id-1"}]),
            serde_json::json!([{"id": "id-1", "key": 7}]),
            serde_json::json!([{"id": "id-1", "key": "alpha"}, {"key": "beta"}]),
        ] {
            assert!(parse_list_entries(&bad).is_err(), "accepted {bad}");
        }
        assert!(parse_list_entries(&serde_json::json!({"not": "array"})).is_err());
    }

    #[test]
    fn resolve_list_id_distinguishes_absent_from_malformed() {
        let list = serde_json::json!([
            {"id": "id-1", "key": "alpha"},
            {"id": "id-2", "key": "beta"}
        ]);
        assert_eq!(
            resolve_list_id(&list, "beta").unwrap(),
            Some("id-2".to_string())
        );
        assert_eq!(resolve_list_id(&list, "absent").unwrap(), None);
        assert_eq!(
            resolve_list_id(&serde_json::json!([]), "alpha").unwrap(),
            None
        );
        // #6568: a key-matching entry with a missing or non-string
        // id is a source failure, not "not in the vault".
        let missing = serde_json::json!([{"key": "alpha"}]);
        assert!(resolve_list_id(&missing, "alpha").is_err());
        let non_string = serde_json::json!([{"id": 7, "key": "alpha"}]);
        assert!(resolve_list_id(&non_string, "alpha").is_err());
        assert!(resolve_list_id(&serde_json::json!({"not": "array"}), "alpha").is_err());
    }

    fn seeded_state(entries: &[(&str, &str, u64)]) -> Arc<State> {
        let state = empty_state();
        {
            let mut secrets = state.secrets.write().unwrap();
            for (k, v, at) in entries {
                secrets.insert(
                    k.to_string(),
                    CachedSecret {
                        value: v.to_string(),
                        fetched_at_unix: *at,
                    },
                );
            }
        }
        state
    }

    #[test]
    fn install_refresh_lands_successes_keeps_failures_and_drops_delisted() {
        // #6767 oracle: one broken fetch must not block the rest of
        // the vault. alpha rotates (lands, fresh timestamp), beta's
        // fetch fails (previous value AND previous age kept — never
        // evicted, never made to look fresh), gamma left the list
        // (removed), delta is new (installed).
        let state = seeded_state(&[
            ("alpha", "old-alpha", 100),
            ("beta", "old-beta", 100),
            ("gamma", "old-gamma", 100),
        ]);
        let listed = vec![
            ("id-a".to_string(), "alpha".to_string()),
            ("id-b".to_string(), "beta".to_string()),
            ("id-d".to_string(), "delta".to_string()),
        ];
        let fetched: HashMap<String, (String, u64)> = [
            ("alpha".to_string(), ("new-alpha".to_string(), 190)),
            ("delta".to_string(), ("fresh-delta".to_string(), 200)),
        ]
        .into_iter()
        .collect();
        let outcome = install_refresh(&state, &listed, fetched, 200).expect("partial install");
        assert_eq!(outcome.installed, 2);
        assert_eq!(outcome.kept_failed, 1);
        assert_eq!(outcome.removed, 1);
        assert_eq!(outcome.total, 3);
        let secrets = state.secrets.read().unwrap();
        // Alpha keeps its own fetch time (190), not the install
        // time (200): a value fetched before a rotation must never
        // claim a timestamp after it (Codex, PR #8491).
        assert_eq!(
            secrets.get("alpha"),
            Some(&CachedSecret {
                value: "new-alpha".to_string(),
                fetched_at_unix: 190
            })
        );
        assert_eq!(
            secrets.get("beta"),
            Some(&CachedSecret {
                value: "old-beta".to_string(),
                fetched_at_unix: 100
            })
        );
        assert!(!secrets.contains_key("gamma"));
        assert_eq!(secrets.get("delta").map(|c| c.fetched_at_unix), Some(200));
        assert_eq!(*state.last_refresh.read().unwrap(), Some(200));
    }

    #[test]
    fn install_refresh_with_zero_successes_keeps_the_whole_previous_cache() {
        // A total fetch outage is not news about the vault: nothing
        // installs and nothing is removed, not even a delisted name.
        let state = seeded_state(&[("alpha", "old-alpha", 100), ("gamma", "old-gamma", 100)]);
        let listed = vec![("id-a".to_string(), "alpha".to_string())];
        assert_eq!(install_refresh(&state, &listed, HashMap::new(), 200), None);
        let secrets = state.secrets.read().unwrap();
        assert_eq!(secrets.len(), 2);
        assert_eq!(
            secrets.get("alpha"),
            Some(&CachedSecret {
                value: "old-alpha".to_string(),
                fetched_at_unix: 100
            })
        );
        assert_eq!(*state.last_refresh.read().unwrap(), Some(1));
    }

    #[test]
    fn secret_response_exposes_age_and_source() {
        // #6767: a rotation runbook must be able to tell whether it
        // is verifying the broker or Bitwarden — the cache response
        // names its source and the value's fetch time and age.
        let at = now_unix() - 120;
        let state = seeded_state(&[("alpha", "fixture-value-not-a-secret", at)]);
        let (status, body) = handle_request(&state, "tok", "GET", "/secret/alpha", Some("tok"));
        assert_eq!(status, 200);
        assert_eq!(body["source"], "cache");
        assert_eq!(body["fetched_at_unix"], at);
        let age = body["age_seconds"].as_u64().expect("age present");
        assert!((120..130).contains(&age), "age {age}");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// PBT (#6568, testing doctrine): the strict list parser
        /// succeeds iff every generated entry has string `id` and
        /// `key` fields, and on success the output is exactly those
        /// pairs in order. Oracle: the well-formedness predicate
        /// recomputed from the generated payload itself.
        #[test]
        fn parse_list_entries_succeeds_iff_every_entry_is_well_formed(
            entries in proptest::collection::vec(
                (0u8..3, 0u8..3, "[a-z]{0,6}", "[a-z]{0,6}"),
                0..12,
            )
        ) {
            let list = serde_json::Value::Array(
                entries
                    .into_iter()
                    .map(|(id_kind, key_kind, id_s, key_s)| {
                        let mut obj = serde_json::Map::new();
                        match id_kind {
                            1 => {
                                obj.insert("id".to_string(), serde_json::json!(id_s));
                            }
                            2 => {
                                obj.insert("id".to_string(), serde_json::json!(42));
                            }
                            _ => {}
                        }
                        match key_kind {
                            1 => {
                                obj.insert("key".to_string(), serde_json::json!(key_s));
                            }
                            2 => {
                                obj.insert("key".to_string(), serde_json::json!(true));
                            }
                            _ => {}
                        }
                        serde_json::Value::Object(obj)
                    })
                    .collect(),
            );
            let expected: Option<Vec<(String, String)>> = list
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    match (
                        s.get("id").and_then(|v| v.as_str()),
                        s.get("key").and_then(|v| v.as_str()),
                    ) {
                        (Some(i), Some(k)) => Some((i.to_string(), k.to_string())),
                        _ => None,
                    }
                })
                .collect();
            match expected {
                Some(pairs) => prop_assert_eq!(parse_list_entries(&list).unwrap(), pairs),
                None => prop_assert!(parse_list_entries(&list).is_err()),
            }
        }
    }

    #[test]
    fn ttl_unset_uses_the_documented_default() {
        let parsed = parse_ttl_s(None).expect("unset is valid");
        assert_eq!(parsed.ttl_s, DEFAULT_TTL_S);
        assert_eq!(parsed.warning, None);
    }

    #[test]
    fn ttl_valid_values_are_honored() {
        for (raw, want) in [("900", 900), ("60", 60), ("30", MIN_TTL_S), (" 120 ", 120)] {
            let parsed = parse_ttl_s(Some(raw)).expect("valid value");
            assert_eq!(parsed.ttl_s, want, "raw={raw:?}");
            assert_eq!(parsed.warning, None);
        }
    }

    #[test]
    fn ttl_zero_and_below_minimum_are_refused() {
        // #5860: TTL_S=0 spun the refresh loop at ~110 full vault
        // refreshes/sec. The refusal names the variable and the value.
        for raw in ["0", "1", "29"] {
            let err = parse_ttl_s(Some(raw)).expect_err("below-minimum must refuse");
            assert!(err.contains("SECRETS_BROKER_TTL_S"), "error: {err}");
            assert!(err.contains(raw), "error: {err}");
        }
    }

    fn sh_runner(timeout: Duration) -> super::BwsRunner {
        super::BwsRunner {
            token: "tok".to_string(),
            timeout,
        }
    }

    #[test]
    fn bws_child_that_never_exits_fails_at_the_deadline() {
        // #10871 oracle: a child that never exits must come back as
        // an error near its deadline, not block its caller forever.
        use fleet_secrets::CommandRunner;
        let started = std::time::Instant::now();
        let err = sh_runner(Duration::from_millis(300))
            .run("/bin/sh", &["-c", "exec sleep 600"])
            .expect_err("a hung child must fail");
        assert!(err.contains("timed out"), "error: {err}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn bws_child_within_the_deadline_returns_its_stdout() {
        use fleet_secrets::CommandRunner;
        let out = sh_runner(Duration::from_secs(20))
            .run("/bin/sh", &["-c", "printf '{\"value\":\"x\"}'"])
            .expect("a prompt child succeeds");
        assert_eq!(out, "{\"value\":\"x\"}");
    }

    #[test]
    fn bws_child_failure_reports_only_the_exit_status() {
        use fleet_secrets::CommandRunner;
        let err = sh_runner(Duration::from_secs(20))
            .run("/bin/sh", &["-c", "echo vault-material >&2; exit 3"])
            .expect_err("a non-zero exit is an error");
        assert!(!err.contains("vault-material"), "error: {err}");
    }

    #[test]
    fn ttl_garbage_defaults_with_a_warning_naming_the_value() {
        for raw in ["abc", "60s", "-5", ""] {
            let parsed = parse_ttl_s(Some(raw)).expect("garbage falls back, never refuses");
            assert_eq!(parsed.ttl_s, DEFAULT_TTL_S, "raw={raw:?}");
            let warning = parsed.warning.expect("the fallback must not be silent");
            assert!(
                warning.contains("SECRETS_BROKER_TTL_S"),
                "warning: {warning}"
            );
            assert!(warning.contains(raw), "warning: {warning}");
        }
    }
}
