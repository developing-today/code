//! secrets-broker — resident Bitwarden Secrets Manager cache (code#5362).
//!
//! Fetches the vault's secrets ONCE per TTL via the `bws` CLI on the
//! laptop and serves them to token-authed local consumers, replacing the
//! per-call `bws secret list` + `get` fetch that made every
//! repo-queue.py invocation pay 10-40 s. Binds the laptop tailnet IP
//! only; consumers reach it through the sandbox session proxy.
//!
//! Endpoints (header `X-Agent-Token` required except /health):
//!   GET /health        -> {ok, secrets, last_refresh_unix, refreshing}
//!   GET /secrets       -> {names: [...]}
//!   GET /secret/<name> -> {key, value} | 404 | 503 (not loaded yet)
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
//!      SECRETS_BROKER_TTL_S (default 900).

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tiny_http::{Header, Response, Server, StatusCode};

struct State {
    secrets: RwLock<HashMap<String, String>>,
    last_refresh: RwLock<Option<u64>>,
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

/// Runs `bws` for the vault refresh, through the platform crate's
/// CommandRunner discipline (fleet-secrets, design §1): stdout on
/// success; on failure the error carries the exit status only —
/// never stderr, which could quote vault material. (The standalone
/// broker included a trimmed stderr excerpt in this log-only error;
/// the port deliberately adopts the stricter platform rule.)
struct BwsRunner {
    token: String,
}

impl fleet_secrets::CommandRunner for BwsRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .env("BWS_ACCESS_TOKEN", &self.token)
            .output()
            .map_err(|e| format!("spawn failed: {}", e.kind()))?;
        if !out.status.success() {
            return Err(out.status.to_string());
        }
        String::from_utf8(out.stdout).map_err(|_| "output not utf8".to_string())
    }
}

fn bws_run(args: &[&str], token: &str) -> Result<String, String> {
    let runner = BwsRunner {
        token: token.to_string(),
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
    // Every per-item failure is counted: a refresh that could not fetch
    // a listed secret must fail as a whole (the caller keeps serving the
    // previous cache, per the stale-while-revalidate contract in run())
    // instead of installing the partial map and evicting the secrets it
    // failed to fetch (#5504).
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
                        got.write().unwrap().insert(key.clone(), val);
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
    if failed > 0 {
        return Err(format!(
            "{failed} of {total} secret fetches failed; keeping the previous cache"
        ));
    }
    let map = got.read().unwrap().clone();
    let n = map.len();
    *state.secrets.write().unwrap() = map;
    *state.last_refresh.write().unwrap() = Some(now_unix());
    Ok(n)
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
    if let Some(v) = state.secrets.read().unwrap().get(name) {
        return Ok(Some(v.clone()));
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
            state
                .secrets
                .write()
                .unwrap()
                .insert(name.to_string(), v.clone());
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
        (
            200,
            json!({
                "ok": true,
                "secrets": state.secrets.read().unwrap().len(),
                "last_refresh_unix": state.last_refresh.read().unwrap().clone(),
                "refreshing": *state.refreshing.read().unwrap(),
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
            Some(v) => (200, json!({"key": name, "value": v})),
            None => {
                if state.last_refresh.read().unwrap().is_none() {
                    (503, json!({"error": "secrets not loaded yet"}))
                } else {
                    // A miss on a loaded cache reads through to
                    // Bitwarden for this one name (#6275) before the
                    // name is declared absent: a secret minted since
                    // the last refresh resolves now, not at the next
                    // TTL tick. A source failure is a loud 502, never
                    // a silent 404-as-if-absent.
                    match fetch_through(state, &name) {
                        Ok(Some(v)) => (200, json!({"key": name, "value": v})),
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

pub fn run() -> i32 {
    let bind =
        std::env::var("SECRETS_BROKER_BIND").unwrap_or_else(|_| "100.99.113.103:4800".to_string());
    let home = crate::home::from_env().unwrap_or_else(|error| {
        eprintln!("secrets-broker: {error}");
        std::process::exit(2);
    });
    let token_file = std::env::var("SECRETS_BROKER_TOKEN_FILE")
        .unwrap_or_else(|_| format!("{home}/.config/secrets-broker/token"));
    let token = crate::token_file::load_token_file(&token_file).unwrap_or_else(|e| {
        crate::log::info("secrets-broker", &e);
        std::process::exit(3);
    });
    let ttl_s: u64 = std::env::var("SECRETS_BROKER_TTL_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900);

    let state = Arc::new(State {
        secrets: RwLock::new(HashMap::new()),
        last_refresh: RwLock::new(None),
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

    let server = Server::http(&bind).unwrap_or_else(|e| {
        crate::log::info("secrets-broker", &format!("cannot bind {bind}: {e:?}"));
        std::process::exit(3);
    });
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
                crate::uds_http::serve(listener, move |req| {
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
                });
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
            if req.method().as_str() == "POST" {
                // POST /refresh's body is ignored by the handler;
                // drain it as the pre-#6216 loop did.
                let mut buf = String::new();
                let _ = req.as_reader().read_to_string(&mut buf);
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
    use super::{State, fetch_through_with, parse_list_entries, resolve_list_id, url_decode};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, RwLock};

    fn empty_state() -> Arc<State> {
        Arc::new(State {
            secrets: RwLock::new(HashMap::new()),
            last_refresh: RwLock::new(Some(1)),
            refreshing: RwLock::new(false),
            inflight: Mutex::new(HashMap::new()),
        })
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
                .map(String::as_str),
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
}
