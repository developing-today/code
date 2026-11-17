//! `fleet gh` credential/token cache (developing-today/code#10742).
//!
//! Every `fleet gh` call used to re-resolve secrets, re-sign App
//! JWTs, re-list installations and re-mint installation tokens; the
//! 2026-11-16 closure watchdog could not clear a ~41h backlog in
//! 10 min partly for want of this. The daemon below caches all
//! four, memory-only:
//!
//! * secret resolutions — short TTL ([`SECRET_TTL_SECS`]);
//! * App JWTs — cached within their expiry (the mint reports its
//!   own TTL, already skewed);
//! * installation tokens — cached until shortly before their
//!   `expires_at` (the mint reports its own TTL, already skewed);
//! * installation lists, per App — TTL ([`INSTALLATIONS_TTL_SECS`]).
//!
//! Minting is single-flight per key: concurrent requests for one
//! key block on the in-flight mint instead of duplicating it.
//! Nothing is ever written to disk; a daemon restart is a cold
//! cache, never a stale one.
//!
//! The wire is newline-delimited JSON over TCP (one request object
//! per line, one response object per line), so both the localhost
//! daemon and the laptop daemon serve the same client.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a resolved secret stays cached: secrets can rotate at
/// any time, so the cache only smooths bursts within one watchdog
/// sweep, never pins a value.
pub const SECRET_TTL_SECS: u64 = 60;
/// How long one App's installation list stays cached: installs
/// change only when the App is (un)installed somewhere — minutes
/// stale is harmless (a miss resolves to "no installation", which
/// fails over to the next rotation source with a named error).
pub const INSTALLATIONS_TTL_SECS: u64 = 300;
/// Seconds shaved off a minted credential's own lifetime before it
/// is cached: a JWT or installation token is served only while it
/// still has this much life left.
pub const EXPIRY_SKEW_SECS: u64 = 60;
/// Floor for a cached token TTL: a mint that reports near-zero life
/// is still served once, but never cached beyond one second.
pub const MIN_TOKEN_TTL_SECS: u64 = 1;

/// The wire protocol version. A daemon that does not know a
/// request's version answers `{"ok":false,...}` instead of
/// guessing — a mismatch is loud, never silently served.
pub const PROTOCOL_VERSION: u32 = 1;

/// Raw credential operations, with no caching. `fleet gh`'s
/// provider implements this (secret resolution, JWT signing,
/// installation listing, token minting); both the daemon and the
/// no-daemon in-process fallback compose the same cache over it.
pub trait Backend: Send + Sync {
    /// Resolve a named secret's value (fleet-secrets chain).
    fn resolve_secret(&self, name: &str) -> Result<String, String>;
    /// Sign one App's JWT. Returns the JWT and the seconds it may
    /// be cached (its own lifetime minus [`EXPIRY_SKEW_SECS`]).
    fn sign_jwt(&self, app: &str) -> Result<(String, u64), String>;
    /// List one App's installations as `(owner_login, id)` pairs,
    /// paginated by the implementation.
    fn list_installations(&self, app: &str) -> Result<Vec<(String, i64)>, String>;
    /// Mint an installation token for one installation. Returns the
    /// token and the seconds it may be cached (its `expires_at`
    /// minus [`EXPIRY_SKEW_SECS`], floored at
    /// [`MIN_TOKEN_TTL_SECS`]).
    fn mint_token(&self, app: &str, installation_id: i64) -> Result<(String, u64), String>;
}

struct Entry {
    value: String,
    expires_at: Instant,
}

struct State {
    entries: HashMap<String, Entry>,
    inflight: HashSet<String>,
    hits: u64,
    misses: u64,
    started: Instant,
}

/// The memory-only cache over a [`Backend`]. Shareable across
/// threads (`Arc<Cache<B>>`); the daemon holds one, and the
/// no-daemon fallback holds a process-local one.
pub struct Cache<B> {
    backend: B,
    mu: Mutex<State>,
    cv: Condvar,
}

impl<B: Backend> Cache<B> {
    pub fn new(backend: B) -> Cache<B> {
        Cache {
            backend,
            mu: Mutex::new(State {
                entries: HashMap::new(),
                inflight: HashSet::new(),
                hits: 0,
                misses: 0,
                started: Instant::now(),
            }),
            cv: Condvar::new(),
        }
    }

    /// The single-flight core: serve a live entry, join an
    /// in-flight mint, or become the minter. Exactly one thread
    /// mints per key; every other thread for that key waits and
    /// then re-reads the cache.
    fn get_or_mint(
        &self,
        key: &str,
        mint: impl FnOnce(&B) -> Result<(String, u64), String>,
    ) -> Result<(String, u64), String> {
        let mut state = self.mu.lock().expect("gh cache lock");
        loop {
            let hit: Option<(String, u64)> = state
                .entries
                .get(key)
                .filter(|entry| entry.expires_at > Instant::now())
                .map(|entry| {
                    (
                        entry.value.clone(),
                        (entry.expires_at - Instant::now()).as_secs(),
                    )
                });
            if let Some((value, ttl)) = hit {
                state.hits += 1;
                return Ok((value, ttl));
            }
            if !state.inflight.contains(key) {
                break;
            }
            state = self.cv.wait(state).expect("gh cache lock");
        }
        state.inflight.insert(key.to_string());
        state.misses += 1;
        drop(state);
        let minted = mint(&self.backend);
        let mut state = self.mu.lock().expect("gh cache lock");
        state.inflight.remove(key);
        match minted {
            Ok((value, ttl)) => {
                let ttl = ttl.max(MIN_TOKEN_TTL_SECS);
                state.entries.insert(
                    key.to_string(),
                    Entry {
                        value: value.clone(),
                        expires_at: Instant::now() + Duration::from_secs(ttl),
                    },
                );
                self.cv.notify_all();
                Ok((value, ttl))
            }
            Err(e) => {
                self.cv.notify_all();
                Err(e)
            }
        }
    }

    /// A cached secret resolution ([`SECRET_TTL_SECS`]).
    pub fn secret(&self, name: &str) -> Result<String, String> {
        self.get_or_mint(&format!("secret:{name}"), |b| {
            b.resolve_secret(name).map(|v| (v, SECRET_TTL_SECS))
        })
        .map(|(v, _)| v)
    }

    /// A cached App JWT (within its own expiry, per the mint).
    pub fn jwt(&self, app: &str) -> Result<String, String> {
        self.get_or_mint(&format!("jwt:{app}"), |b| b.sign_jwt(app))
            .map(|(v, _)| v)
    }

    /// A cached installation list for one App
    /// ([`INSTALLATIONS_TTL_SECS`]), as `(owner_login, id)` pairs.
    /// Oracle: the daemon's JSON encoding round-trips the exact
    /// pairs the backend reported (the cache-stores-strings test
    /// below pins this).
    pub fn installations(&self, app: &str) -> Result<Vec<(String, i64)>, String> {
        let (raw, _) = self.get_or_mint(&format!("installations:{app}"), |b| {
            b.list_installations(app).and_then(|pairs| {
                serde_json::to_string(&pairs)
                    .map(|v| (v, INSTALLATIONS_TTL_SECS))
                    .map_err(|e| format!("installations encode error: {e}"))
            })
        })?;
        serde_json::from_str(&raw).map_err(|_| "installations: cache body unparseable".to_string())
    }

    /// A cached installation token (until shortly before its own
    /// expiry, per the mint). The installation id resolves through
    /// the cached installation list, so one call warms both.
    pub fn installation_token(&self, app: &str, owner: &str) -> Result<String, String> {
        let pairs = self.installations(app)?;
        let id = pairs
            .iter()
            .find(|(login, _)| login.eq_ignore_ascii_case(owner))
            .map(|(_, id)| *id)
            .ok_or_else(|| format!("no app installation found for owner '{owner}'"))?;
        self.get_or_mint(&format!("token:{app}:{owner}"), |b| b.mint_token(app, id))
            .map(|(v, _)| v)
    }

    /// The status snapshot: per-kind entry counts, hit/miss
    /// counters, uptime, and the soonest expiry. Key names and
    /// values never leave the daemon — counts only, since key
    /// names carry secret names and owner logins.
    pub fn snapshot(&self) -> serde_json::Value {
        let state = self.mu.lock().expect("gh cache lock");
        let mut kinds: HashMap<&str, u64> = HashMap::new();
        let mut min_ttl: Option<u64> = None;
        let now = Instant::now();
        for (key, entry) in &state.entries {
            let kind = key.split(':').next().unwrap_or("other");
            *kinds.entry(kind).or_default() += 1;
            if entry.expires_at > now {
                let ttl = (entry.expires_at - now).as_secs();
                min_ttl = Some(min_ttl.map_or(ttl, |m: u64| m.min(ttl)));
            }
        }
        serde_json::json!({
            "uptime_secs": state.started.elapsed().as_secs(),
            "hits": state.hits,
            "misses": state.misses,
            "live_entries": state.entries.len(),
            "inflight": state.inflight.len(),
            "kinds": kinds,
            "min_ttl_secs": min_ttl.unwrap_or(0),
        })
    }
}

// ---------------------------------------------------------------------------
// The wire: one JSON object per line each way.
// ---------------------------------------------------------------------------

/// Build a request object for one operation. `op` is `personal`,
/// `jwt`, `token` or `status`; `app`/`owner` qualify `jwt`/`token`,
/// and `name` carries the secret name for the `personal` op (so the
/// daemon never hardcodes which secret personal means).
pub fn request(
    op: &str,
    app: Option<&str>,
    owner: Option<&str>,
    name: Option<&str>,
) -> serde_json::Value {
    let mut req = serde_json::json!({"v": PROTOCOL_VERSION, "op": op});
    if let Some(app) = app {
        req["app"] = app.into();
    }
    if let Some(owner) = owner {
        req["owner"] = owner.into();
    }
    if let Some(name) = name {
        req["name"] = name.into();
    }
    req
}

/// Serve one already-parsed request against the cache. Returns the
/// response object; transport errors never reach here (they fail
/// the connection, not the request).
pub fn answer<B: Backend>(cache: &Cache<B>, req: &serde_json::Value) -> serde_json::Value {
    let err = |message: String| serde_json::json!({"ok": false, "error": message});
    if req.get("v").and_then(|v| v.as_u64()) != Some(u64::from(PROTOCOL_VERSION)) {
        return err(format!(
            "unsupported protocol version (want v{})",
            PROTOCOL_VERSION
        ));
    }
    let op = req.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let app = req.get("app").and_then(|v| v.as_str()).unwrap_or("");
    let owner = req.get("owner").and_then(|v| v.as_str()).unwrap_or("");
    let name = req.get("name").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "personal" if !name.is_empty() => match cache.secret(name) {
            Ok(value) => ok_token(&value, SECRET_TTL_SECS),
            Err(e) => err(e),
        },
        "jwt" if !app.is_empty() => match cache.jwt(app) {
            Ok(token) => ok_token(&token, 0),
            Err(e) => err(e),
        },
        "token" if !app.is_empty() && !owner.is_empty() => {
            match cache.installation_token(app, owner) {
                Ok(token) => ok_token(&token, 0),
                Err(e) => err(e),
            }
        }
        "status" => serde_json::json!({"ok": true, "status": cache.snapshot()}),
        _ => err(format!(
            "unknown op {op:?} (want personal|jwt|token|status)"
        )),
    }
}

fn ok_token(token: &str, ttl: u64) -> serde_json::Value {
    serde_json::json!({"ok": true, "token": token, "expires_in": ttl})
}

/// Parse a daemon response line: `Ok(token)` on success,
/// `Err(message)` on a daemon-reported failure or an unparseable
/// line (the line is summarized, never echoed: it could carry a
/// token on a buggy day).
pub fn parse_response(line: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(line).map_err(|_| "cache daemon: unparseable response".to_string())
}

/// Extract the token from an `ok` response object.
pub fn response_token(resp: &serde_json::Value) -> Result<String, String> {
    if resp.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        resp.get("token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "cache daemon: ok response carried no token".to_string())
    } else {
        Err(resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("cache daemon reported failure")
            .to_string())
    }
}

// ---------------------------------------------------------------------------
// Daemon and client.
// ---------------------------------------------------------------------------

/// How long the client waits to connect to one daemon address: a
/// dead localhost listener must fail fast so the laptop dial (or
/// the in-process fallback) starts promptly.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long one request may take: a cold daemon mints over the
/// network (secret resolution, JWT signing, two GitHub round
/// trips), so this covers a slow first fill, not just a hit.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Run the cache daemon forever on `bind` (code#10742
/// `cache-serve`): accept loop, one thread per connection, one
/// JSON request per line. Returns only on bind failure (with the
/// bind error named); per-connection errors end the connection,
/// never the daemon.
pub fn serve<B: Backend + 'static>(backend: B, bind: &str) -> i32 {
    let listener = match TcpListener::bind(bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("fleet gh cache-serve: cannot bind '{bind}': {e}");
            return 3;
        }
    };
    let (_cache, accept) = serve_on(backend, listener);
    // serve_on's accept loop runs on its own thread and serve_on
    // returns at once; the daemon must not return while that loop
    // is healthy. An earlier serve() fell through to the failure
    // exit below the moment serve_on returned — the process bound,
    // printed the failure line, and died before answering a single
    // request (found live 2026-11-16, task 17509). Join the accept
    // thread instead: serve() returns only when the loop actually
    // breaks, which is the only way to reach the failure exit.
    let _ = accept.join();
    eprintln!("fleet gh cache-serve: listener on '{bind}' failed; exiting");
    1
}

/// The accept loop over an already-bound listener, sharing one
/// cache across connections. Exposed for the loopback test; the
/// daemon entry point is [`serve`].
pub fn serve_on<B: Backend + 'static>(
    backend: B,
    listener: TcpListener,
) -> (Arc<Cache<B>>, std::thread::JoinHandle<()>) {
    let cache = Arc::new(Cache::new(backend));
    let cache_worker = Arc::clone(&cache);
    let accept = std::thread::spawn(move || {
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => {
                    let cache = Arc::clone(&cache_worker);
                    std::thread::spawn(move || handle_conn(&cache, stream));
                }
                Err(_) => break,
            }
        }
    });
    (cache, accept)
}

fn handle_conn<B: Backend>(cache: &Cache<B>, stream: TcpStream) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(req) => answer(cache, &req),
            Err(_) => serde_json::json!({"ok": false, "error": "unparseable request"}),
        };
        let stream = reader.get_mut();
        let mut text = serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        if stream.write_all(text.as_bytes()).is_err() {
            break;
        }
    }
}

/// Query one daemon address with one request: connect, send, read
/// one response line. Transport failures name the address and the
/// stage, never any credential.
pub fn fetch_one(
    addr: &str,
    req: &serde_json::Value,
    connect_timeout: Duration,
) -> Result<serde_json::Value, String> {
    let sock_addr: Vec<std::net::SocketAddr> = addr
        .to_socket_addrs()
        .map_err(|e| format!("cache daemon {addr}: address error: {e}"))?
        .collect();
    let Some(first) = sock_addr.first() else {
        return Err(format!("cache daemon {addr}: address resolved empty"));
    };
    let stream = TcpStream::connect_timeout(first, connect_timeout)
        .map_err(|e| format!("cache daemon {addr}: connect failed: {e}"))?;
    stream
        .set_read_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|e| format!("cache daemon {addr}: timeout setup failed: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("cache daemon {addr}: timeout setup failed: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut text = serde_json::to_string(req).unwrap_or_else(|_| "{}".to_string());
    text.push('\n');
    reader
        .get_mut()
        .write_all(text.as_bytes())
        .map_err(|e| format!("cache daemon {addr}: request write failed: {e}"))?;
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("cache daemon {addr}: response read failed: {e}"))?;
    parse_response(line.trim())
}

/// Try each address in order, returning the first daemon's parsed
/// response. Every address is attempted: a refused localhost must
/// not stop the laptop dial.
pub fn fetch_any(
    addrs: &[String],
    req: &serde_json::Value,
    connect_timeout: Duration,
) -> Result<serde_json::Value, String> {
    let mut errors = Vec::new();
    for addr in addrs {
        match fetch_one(addr, req, connect_timeout) {
            Ok(resp) => return Ok(resp),
            Err(e) => errors.push(e),
        }
    }
    Err(errors.join("; "))
}

/// Parse a GitHub `expires_at` (`2026-11-16T12:34:56Z`) into
/// cacheable seconds from now, minus [`EXPIRY_SKEW_SECS`], floored
/// at [`MIN_TOKEN_TTL_SECS`]. Unparseable input falls back to a
/// just-under-an-hour TTL (installation tokens live one hour):
/// a wrong-but-short TTL only re-mints early, never serves dead.
pub fn token_ttl_from_expires_at(expires_at: &str) -> u64 {
    const FALLBACK: u64 = 3500;
    let ttl = parse_rfc3339_z(expires_at).map(|exp| exp.saturating_sub(current_unix()));
    match ttl {
        Some(secs) if secs > EXPIRY_SKEW_SECS => secs - EXPIRY_SKEW_SECS,
        Some(_) => MIN_TOKEN_TTL_SECS,
        None => FALLBACK,
    }
}

fn current_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Minimal `YYYY-MM-DDTHH:MM:SSZ` parser (GitHub's `expires_at`
/// shape): twenty lines beat a datetime dependency for one field.
fn parse_rfc3339_z(s: &str) -> Option<u64> {
    let s = s.trim().strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let mut t = time.split(':');
    let (hh, mm, ss): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    if !(0..24).contains(&hh) || !(0..60).contains(&mm) || !(0..61).contains(&ss) {
        return None;
    }
    // Howard Hinnant's days_from_civil: days since 1970-01-01.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    u64::try_from(days * 86400 + hh * 3600 + mm * 60 + ss).ok()
}

// ---------------------------------------------------------------------------
// Tests. Every test names its oracle (fleet testing doctrine).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scripted backend: counts mints, sleeps to expose races.
    struct FakeBackend {
        mints: AtomicUsize,
        mint_delay: Duration,
    }

    impl FakeBackend {
        fn new() -> FakeBackend {
            FakeBackend {
                mints: AtomicUsize::new(0),
                mint_delay: Duration::ZERO,
            }
        }

        fn mint_count(&self) -> usize {
            self.mints.load(Ordering::SeqCst)
        }
    }

    impl Backend for FakeBackend {
        fn resolve_secret(&self, _name: &str) -> Result<String, String> {
            self.mints.fetch_add(1, Ordering::SeqCst);
            if !self.mint_delay.is_zero() {
                std::thread::sleep(self.mint_delay);
            }
            Ok("tok-secret".to_string())
        }

        fn sign_jwt(&self, app: &str) -> Result<(String, u64), String> {
            self.mints.fetch_add(1, Ordering::SeqCst);
            Ok((format!("jwt-{app}"), 400))
        }

        fn list_installations(&self, app: &str) -> Result<Vec<(String, i64)>, String> {
            self.mints.fetch_add(1, Ordering::SeqCst);
            Ok(vec![
                ("developing-today".to_string(), 7),
                (app.to_string(), 9),
            ])
        }

        fn mint_token(&self, app: &str, id: i64) -> Result<(String, u64), String> {
            self.mints.fetch_add(1, Ordering::SeqCst);
            if !self.mint_delay.is_zero() {
                std::thread::sleep(self.mint_delay);
            }
            Ok((format!("ghs-{app}-{id}"), 3500))
        }
    }

    // Oracle: the cache contract — a second read of a live key
    // must not mint again; the mint count pins this.
    #[test]
    fn live_entries_do_not_remint() {
        let cache = Cache::new(FakeBackend::new());
        assert_eq!(cache.secret("s").unwrap(), "tok-secret");
        assert_eq!(cache.secret("s").unwrap(), "tok-secret");
        assert_eq!(cache.jwt("fleet").unwrap(), "jwt-fleet");
        assert_eq!(
            cache
                .installation_token("fleet", "developing-today")
                .unwrap(),
            "ghs-fleet-7"
        );
        // secret + jwt + installations-list + token = 4 mints;
        // the repeats above served from cache.
        assert_eq!(
            cache.backend.mint_count(),
            4,
            "each key minted exactly once"
        );
    }

    // Oracle: an unknown owner is a named error, never a mint —
    // the installations list has no such login.
    #[test]
    fn unknown_owner_names_the_owner() {
        let cache = Cache::new(FakeBackend::new());
        let err = cache
            .installation_token("fleet", "nobody-here")
            .unwrap_err();
        assert!(err.contains("nobody-here"), "{err}");
    }

    // Oracle: the single-flight rule — ten threads racing one
    // cold key produce exactly one mint (the delay makes the race
    // real; without single-flight the count would be ten).
    #[test]
    fn concurrent_miss_mints_once() {
        let cache = Arc::new(Cache::new(FakeBackend {
            mints: AtomicUsize::new(0),
            mint_delay: Duration::from_millis(50),
        }));
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let cache = Arc::clone(&cache);
                std::thread::spawn(move || cache.installation_token("fleet", "developing-today"))
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap().unwrap(), "ghs-fleet-7");
        }
        assert_eq!(
            cache.backend.mint_count(),
            2,
            "single-flight minted once per key"
        );
    }

    // Oracle: RFC 3339 — the parser agrees with `date -u +%s` on
    // GitHub's expires_at shape, and rejects garbage to the
    // just-under-an-hour fallback.
    #[test]
    fn expires_at_vectors() {
        assert_eq!(parse_rfc3339_z("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_z("2026-01-01T00:00:00Z"), Some(1767225600));
        assert_eq!(parse_rfc3339_z("not a date"), None);
        assert_eq!(parse_rfc3339_z("2026-13-01T00:00:00Z"), None);
        assert_eq!(token_ttl_from_expires_at("not a date"), 3500);
        // An already-dead expiry floors, never underflows.
        assert_eq!(token_ttl_from_expires_at("1970-01-01T00:00:00Z"), 1);
    }

    // Oracle: the kind-only status rule — the snapshot carries
    // counts and TTLs but no key names (which embed secret names
    // and owner logins) and no values.
    #[test]
    fn status_snapshot_leaks_no_names_or_values() {
        let cache = Cache::new(FakeBackend::new());
        cache.secret("super-secret-name").unwrap();
        let snap = cache.snapshot();
        let text = serde_json::to_string(&snap).unwrap();
        assert!(!text.contains("super-secret-name"), "{text}");
        assert!(!text.contains("tok-secret"), "{text}");
        assert_eq!(snap["kinds"]["secret"], 1);
    }

    // Oracle: the loopback round-trip — a real daemon on an
    // ephemeral port answers a real TCP client with the minted
    // token, and a second call is served from the daemon's cache
    // (one backend mint total).
    #[test]
    fn loopback_daemon_serves_the_client() {
        struct Counting {
            mints: std::sync::Arc<AtomicUsize>,
        }
        impl Backend for Counting {
            fn resolve_secret(&self, _n: &str) -> Result<String, String> {
                self.mints.fetch_add(1, Ordering::SeqCst);
                Ok("tok-loopback".to_string())
            }
            fn sign_jwt(&self, _a: &str) -> Result<(String, u64), String> {
                unreachable!()
            }
            fn list_installations(&self, _a: &str) -> Result<Vec<(String, i64)>, String> {
                unreachable!()
            }
            fn mint_token(&self, _a: &str, _i: i64) -> Result<(String, u64), String> {
                unreachable!()
            }
        }
        let mints = std::sync::Arc::new(AtomicUsize::new(0));
        let backend = Counting {
            mints: std::sync::Arc::clone(&mints),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _cache = serve_on(backend, listener);
        let addr = format!("127.0.0.1:{port}");
        let req = request("personal", None, None, Some("s"));
        for _ in 0..2 {
            let resp = fetch_one(&addr, &req, Duration::from_secs(2)).unwrap();
            assert_eq!(response_token(&resp).unwrap(), "tok-loopback");
        }
        assert_eq!(
            mints.load(Ordering::SeqCst),
            1,
            "daemon cached across connections"
        );

        // A bad version is a loud error, never a guess.
        let bad = serde_json::json!({"v": 999u32, "op": "personal"});
        let resp = fetch_one(&addr, &bad, Duration::from_secs(2)).unwrap();
        assert!(response_token(&resp).is_err());
    }

    // Oracle: the daemon ENTRY POINT (serve, what `fleet gh
    // cache-serve` actually runs) stays up and serves: it binds,
    // answers real TCP clients, and caches across connections.
    // serve_on alone passing is not proof — serve() once delegated
    // to serve_on's spawned thread and exited immediately (task
    // 17509, 2026-11-16), leaving no daemon listening at all.
    #[test]
    fn serve_entry_point_stays_up_and_caches() {
        // Pick a free port the same way the loopback test picks its
        // listener, then hand the address to serve() as a string.
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        struct ServeCounting {
            mints: std::sync::Arc<AtomicUsize>,
        }
        impl Backend for ServeCounting {
            fn resolve_secret(&self, _n: &str) -> Result<String, String> {
                self.mints.fetch_add(1, Ordering::SeqCst);
                Ok("tok-secret".to_string())
            }
            fn sign_jwt(&self, _a: &str) -> Result<(String, u64), String> {
                unreachable!()
            }
            fn list_installations(&self, _a: &str) -> Result<Vec<(String, i64)>, String> {
                unreachable!()
            }
            fn mint_token(&self, _a: &str, _i: i64) -> Result<(String, u64), String> {
                unreachable!()
            }
        }
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let backend = ServeCounting {
            mints: std::sync::Arc::clone(&counter),
        };
        let server = std::thread::spawn(move || serve(backend, &format!("127.0.0.1:{port}")));
        let addr = format!("127.0.0.1:{port}");
        let req = request("personal", None, None, Some("s"));
        // Bounded retry until the daemon thread binds and answers.
        let mut resp = None;
        for _ in 0..50 {
            if let Ok(r) = fetch_one(&addr, &req, Duration::from_millis(200)) {
                resp = Some(r);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let resp = resp.expect("serve() daemon never answered a client");
        assert_eq!(response_token(&resp).unwrap(), "tok-secret");
        let resp = fetch_one(&addr, &req, Duration::from_secs(2)).unwrap();
        assert_eq!(response_token(&resp).unwrap(), "tok-secret");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "daemon cached across connections"
        );
        assert!(
            !server.is_finished(),
            "serve() returned while its listener was healthy"
        );
    }
}
