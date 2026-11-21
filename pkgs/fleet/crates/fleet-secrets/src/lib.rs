//! fleet-secrets — the one secrets-by-name path for fleet
//! (design note §1, epic developing-today/code#5412).
//!
//! Platform rule, carried over from the brokers' lessons: **secrets are
//! resolved by name through this crate only.** No domain shells out to
//! `bws` or reads a token file on its own. Resolution order:
//!
//!   1. the **secrets-broker protocol** — a framed request through the
//!      session broker to port 4800, `GET /secret/<name>`, with the
//!      token *file* named in the request (the session broker reads it
//!      per request and sends `X-Agent-Token`; fleet never opens it);
//!   2. the **`bws` CLI fallback** — `bws secret list` + `get`, the
//!      same shape the secrets broker itself uses to fill its cache.
//!
//! Discipline, enforced by the types below:
//!   * a resolved value lives in [`SecretValue`], whose `Debug` is
//!     redacted and which exposes the string only through `expose()`;
//!   * errors name the secret's *name*, the source, and status codes —
//!     never a value, never a subprocess's stderr (which could quote
//!     vault material);
//!   * nothing here persists a value: no cache, no file, no log line.
//!   * an empty string is never a successful resolution (#6275): a
//!     broken read and a genuinely empty vault value are
//!     indistinguishable at this layer, and the 2026-11-05 incident
//!     was a caller exiting 0 with an empty value while its sources
//!     had actually failed — so an empty value counts as a source
//!     failure everywhere in this crate, and the chain moves on or
//!     fails loudly.
//!   * a broker leg that cannot answer is retried before the chain
//!     moves on ([`RetryPolicy`]): a broker restart — a switch
//!     restarting the laptop's units is the routine case — is a
//!     seconds-long window, and the 2026-11-15 incident was a
//!     16-second broker+tunnel restart degrading a whole consumer
//!     run that never retried. Only `Unavailable` outcomes retry:
//!     a broker's 404 is a definitive answer about the name and is
//!     never retried. Each attempt is time-bounded, so a peer that
//!     accepts a connection and then never answers cannot wedge
//!     the caller the way an unbounded read once could.
//!   * the `bws` fallback carries its own credential: the token is
//!     resolved the way the secrets broker resolves its own (the
//!     `BWS_ACCESS_TOKEN` env var when non-empty, else the on-disk
//!     machine-account token file) and handed to the `bws` child
//!     explicitly. Before this, the fallback inherited the ambient
//!     environment only, and non-interactive callers run with the
//!     variable unset or empty — the fallback was dead exactly
//!     when it was needed (the other half of 2026-11-15).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_proxy_proto::{Request, Response};

/// A resolved secret value. Construct only from a source; the inner
/// string is reachable only via [`SecretValue::expose`].
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap a value a [`SecretSource`] has just resolved from its
    /// backend. Source implementations outside this crate construct
    /// values only here, so every resolved secret immediately sits
    /// behind the redacted newtype.
    pub fn from_resolved(value: String) -> SecretValue {
        SecretValue(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(***)")
    }
}

/// Resolution failures. Deliberately value-free (see crate docs).
#[derive(Debug)]
pub enum SecretError {
    /// The name itself is not usable (it becomes a URL path segment,
    /// so separators and controls are rejected before any I/O).
    InvalidName(String),
    /// The source answered: no secret by that name.
    NotFound { source: &'static str, name: String },
    /// The source could not answer (transport down, bad status,
    /// subprocess failed). `detail` carries status codes and error
    /// kinds only — never payload or stderr content.
    Unavailable {
        source: &'static str,
        name: String,
        detail: String,
    },
    /// Every source in the chain failed; carries each source's error.
    Chain {
        name: String,
        errors: Vec<SecretError>,
    },
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::InvalidName(name) => {
                write!(
                    f,
                    "invalid secret name {name:?}: names may not contain '/', '?', '#', '%', whitespace, or control characters, and may not be '.' or '..'"
                )
            }
            SecretError::NotFound { source, name } => {
                write!(f, "secret {name:?} not found via {source}")
            }
            SecretError::Unavailable {
                source,
                name,
                detail,
            } => write!(f, "secret {name:?} unavailable via {source}: {detail}"),
            SecretError::Chain { name, errors } => {
                write!(
                    f,
                    "secret {name:?} unresolved: {} source(s) failed",
                    errors.len()
                )?;
                for e in errors {
                    write!(f, "; {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SecretError {}

/// Names become URL path segments (`GET /secret/<name>`); validate
/// before any source sees them.
pub fn validate_name(name: &str) -> Result<(), SecretError> {
    // A name is interpolated raw as one path segment, so the encoded
    // and dot-segment spellings of the refused characters are refused
    // too (issue #5469): a percent sign introduces every percent-encoded
    // spelling (an encoded slash decodes to a separator, an encoded
    // dot pair decodes to a dot segment), and a name that is exactly a
    // dot segment is removed by path normalization — the request would
    // no longer name a secret under the secret prefix at all.
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|c| {
            c == '/' || c == '?' || c == '#' || c == '%' || c.is_whitespace() || c.is_control()
        })
    {
        return Err(SecretError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// One way to resolve a name. Sources are tried in chain order.
pub trait SecretSource {
    /// Stable source name for error messages ("secrets-broker", "bws").
    fn source_name(&self) -> &'static str;
    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError>;
}

/// The broker retry budget, in one obvious place: how long a
/// broker leg keeps retrying `Unavailable` outcomes before the
/// chain moves to the next source. ~2 minutes spans the observed
/// restart windows (a full switch restarts the broker and its
/// tunnel together; the broker itself is back in ~16s) without
/// making a genuinely dead broker a hang.
pub const BROKER_RETRY_BUDGET: Duration = Duration::from_secs(120);
/// First pause between broker attempts; doubles per attempt.
pub const BROKER_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(2);
/// Cap on the pause between broker attempts.
pub const BROKER_RETRY_MAX_DELAY: Duration = Duration::from_secs(15);
/// Per-attempt ceiling on a single broker exchange (connect +
/// write + read). Bounds an accept-but-silent peer: without it
/// one attempt could block forever and no retry or fallback
/// would ever run.
pub const BROKER_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// How a broker source retries `Unavailable` outcomes before
/// giving up to the chain. `NotFound` (a broker 404) and
/// `InvalidName` are definitive and never retry, at any policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total wall-clock budget for the retry loop. Zero means a
    /// single attempt — the shape for a leg that must stay
    /// fast-failing (the `list` path, and the session-broker leg
    /// when a direct leg ahead of it already owns the budget).
    pub budget: Duration,
    /// First pause between attempts; doubles per attempt.
    pub initial_delay: Duration,
    /// Cap on the pause between attempts.
    pub max_delay: Duration,
    /// Per-attempt exchange timeout (see [`BROKER_ATTEMPT_TIMEOUT`]).
    pub attempt_timeout: Duration,
}

impl RetryPolicy {
    /// No retries: exactly one attempt. The per-attempt timeout
    /// still applies — a single attempt must not hang either.
    pub const NONE: RetryPolicy = RetryPolicy {
        budget: Duration::ZERO,
        initial_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
        attempt_timeout: BROKER_ATTEMPT_TIMEOUT,
    };
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            budget: BROKER_RETRY_BUDGET,
            initial_delay: BROKER_RETRY_INITIAL_DELAY,
            max_delay: BROKER_RETRY_MAX_DELAY,
            attempt_timeout: BROKER_ATTEMPT_TIMEOUT,
        }
    }
}

/// `delay` plus up to +50% jitter, derived from the wall clock —
/// enough to decorrelate clients that hit the same restart
/// window; this crate takes no RNG dependency for it.
fn jittered(delay: Duration) -> Duration {
    if delay.is_zero() {
        return delay;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    let base = delay.as_nanos() as u64;
    delay + Duration::from_nanos(nanos % (base / 2 + 1))
}

/// Run `attempt` until it succeeds, answers definitively
/// (`NotFound` / `InvalidName` — returned at once), or the
/// policy's budget is spent, retrying `Unavailable` outcomes
/// with capped exponential backoff + jitter. The error returned
/// on exhaustion is the most recent attempt's.
fn resolve_with_retry(
    policy: &RetryPolicy,
    mut attempt: impl FnMut() -> Result<SecretValue, SecretError>,
) -> Result<SecretValue, SecretError> {
    let started = Instant::now();
    let mut delay = policy.initial_delay;
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(err @ SecretError::Unavailable { .. }) => {
                let elapsed = started.elapsed();
                if elapsed >= policy.budget {
                    return Err(err);
                }
                let pause = jittered(if delay.is_zero() {
                    Duration::from_millis(1)
                } else {
                    delay
                })
                .min(policy.budget - elapsed);
                std::thread::sleep(pause);
                delay = delay.saturating_mul(2).min(if policy.max_delay.is_zero() {
                    Duration::from_millis(1)
                } else {
                    policy.max_delay
                });
            }
            Err(err) => return Err(err),
        }
    }
}

/// The ordered chain: broker protocol first, `bws` fallback (§1).
pub struct Resolver {
    sources: Vec<Box<dyn SecretSource>>,
}

impl Resolver {
    pub fn new(sources: Vec<Box<dyn SecretSource>>) -> Resolver {
        Resolver { sources }
    }

    /// The default chain for a resolved fleet config. On a host
    /// where the secrets-broker's own socket answers (the laptop,
    /// #6218), the direct source comes first: there is no session
    /// broker there. On the sandbox the probe selects TCP, no
    /// direct source is inserted, and the chain is exactly the
    /// historical broker-protocol-then-`bws` shape.
    pub fn default_chain(config: &fleet_config::Config) -> Resolver {
        let mut sources: Vec<Box<dyn SecretSource>> = Vec::new();
        // Exactly one broker leg owns the retry budget: the first
        // one. On the laptop that is the direct leg; the
        // session-broker leg behind it gets RetryPolicy::NONE —
        // there is no session broker on the laptop, so its socket
        // failure is a permanent shape, and retrying it would only
        // delay the `bws` fallback by a second budget. On the
        // sandbox there is no direct leg, and the session-broker
        // leg owns the budget.
        let mut broker_retry = RetryPolicy::default();
        if let Some(direct) = DirectBrokerSource::from_config(config) {
            sources.push(Box::new(direct));
            broker_retry = RetryPolicy::NONE;
        }
        sources.push(Box::new(
            BrokerSource::new(config.broker_socket(), config.secrets_token_file())
                .with_retry_policy(broker_retry),
        ));
        sources.push(Box::new(BwsSource::new()));
        Resolver::new(sources)
    }

    pub fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        let mut errors = Vec::new();
        for source in &self.sources {
            match source.resolve(name) {
                Ok(value) if !value.expose().is_empty() => return Ok(value),
                Ok(_) => errors.push(SecretError::Unavailable {
                    source: source.source_name(),
                    name: name.to_string(),
                    detail: "source returned an empty value".to_string(),
                }),
                Err(e) => errors.push(e),
            }
        }
        Err(SecretError::Chain {
            name: name.to_string(),
            errors,
        })
    }
}

/// Source 1: the secrets-broker protocol, through the session broker.
/// The exchange is one framed [`Request`] / [`Response`] pair over the
/// session broker's Unix socket — the same path every sandbox client
/// uses today.
pub struct BrokerSource {
    pub broker_socket: PathBuf,
    /// Path of the token file the session broker reads per request.
    /// fleet names it in the request and never opens it.
    pub token_file: PathBuf,
    /// Retry behaviour for `resolve` (see [`RetryPolicy`]).
    /// `list_names` never retries — a listing stays fast-failing —
    /// but the policy's per-attempt timeout bounds its exchange
    /// too.
    pub retry_policy: RetryPolicy,
}

impl BrokerSource {
    pub fn new(broker_socket: PathBuf, token_file: PathBuf) -> BrokerSource {
        BrokerSource {
            broker_socket,
            token_file,
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> BrokerSource {
        self.retry_policy = retry_policy;
        self
    }
    /// One framed exchange over an already-connected session-broker
    /// stream. Split from `resolve` so tests can drive it over a
    /// socket pair.
    fn exchange(
        &self,
        stream: &mut UnixStream,
        request: &Request,
        name: &str,
    ) -> Result<Response, SecretError> {
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail,
        };
        fleet_proxy_proto::write_frame(stream, request).map_err(|e| unavailable(e.to_string()))?;
        let response: Response =
            fleet_proxy_proto::read_typed(stream).map_err(|e| unavailable(e.to_string()))?;
        Ok(response)
    }

    /// One framed request/response over a fresh session-broker
    /// connection — the exchange [`BrokerSource::resolve`] and
    /// [`BrokerSource::list_names`] share.
    fn request(&self, path: &str, name: &str) -> Result<Response, SecretError> {
        let request = Request {
            method: "GET".to_string(),
            path: path.to_string(),
            body: None,
            port: Some(fleet_config::PORT_SECRETS_BROKER),
            token_file: Some(self.token_file.to_string_lossy().into_owned()),
            trip: None,
            transport: None,
            cf_ok: None,
        };
        let mut stream = self.connect(name)?;
        // Bound the exchange: a session broker (or the path behind
        // it) that accepts and then never answers must fail the
        // attempt, not wedge the caller — the retry loop and the
        // chain can only run if an attempt returns.
        if !self.retry_policy.attempt_timeout.is_zero() {
            let _ = stream.set_read_timeout(Some(self.retry_policy.attempt_timeout));
            let _ = stream.set_write_timeout(Some(self.retry_policy.attempt_timeout));
        }
        self.exchange(&mut stream, &request, name)
    }

    /// The broker's names listing (`GET /secrets` — the endpoint the
    /// step-6 verb's `list` renders). Names only: the broker's reply
    /// carries no values on this route, and neither does this API.
    /// Errors are [`SecretError::Unavailable`] under the pseudo-name
    /// `"names"` — a listing has no single secret to name.
    pub fn list_names(&self) -> Result<Vec<String>, SecretError> {
        let name = "names";
        let response = self.request("/secrets", name)?;
        match response.status {
            200 => {
                let names = response
                    .body
                    .get("names")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response without a \"names\" array".to_string(),
                    })?;
                names
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| SecretError::Unavailable {
                                source: "secrets-broker",
                                name: name.to_string(),
                                detail: "\"names\" array holds a non-string entry".to_string(),
                            })
                    })
                    .collect()
            }
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: format!("secrets-broker answered HTTP {status}"),
            }),
        }
    }

    fn connect(&self, name: &str) -> Result<UnixStream, SecretError> {
        UnixStream::connect(&self.broker_socket).map_err(|e| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail: format!(
                "session broker socket {}: {}",
                self.broker_socket.display(),
                e.kind()
            ),
        })
    }
}

impl SecretSource for BrokerSource {
    fn source_name(&self) -> &'static str {
        "secrets-broker"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        resolve_with_retry(&self.retry_policy, || self.resolve_once(name))
    }
}

impl BrokerSource {
    /// One resolution attempt: a single framed exchange and its
    /// status mapping. 404 maps to `NotFound` (definitive — the
    /// retry loop returns it at once); every other non-200, and
    /// every transport failure inside `request`, is `Unavailable`
    /// and therefore retryable under the source's policy.
    fn resolve_once(&self, name: &str) -> Result<SecretValue, SecretError> {
        let response = self.request(&format!("/secret/{name}"), name)?;
        match response.status {
            200 => {
                let value = response
                    .body
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response without a string \"value\" field".to_string(),
                    })?;
                if value.is_empty() {
                    return Err(SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response with an empty \"value\"".to_string(),
                    });
                }
                Ok(SecretValue(value.to_string()))
            }
            404 => Err(SecretError::NotFound {
                source: "secrets-broker",
                name: name.to_string(),
            }),
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: broker_error_detail(status, &response.body),
            }),
        }
    }
}

/// The `Unavailable` detail for a non-200 broker answer: the HTTP
/// status plus the broker's own `error` / `refresh_error` fields
/// when it sent them. A broker that has never loaded because its
/// vault refresh is failing (#10743) says so in `refresh_error`;
/// surfacing it turns a silent 503 retry loop into a named failure
/// the moment the retry budget is spent.
fn broker_error_detail(status: u16, body: &serde_json::Value) -> String {
    let mut detail = format!("secrets-broker answered HTTP {status}");
    if let Some(e) = body.get("error").and_then(|v| v.as_str()) {
        detail.push_str(&format!(": {e}"));
    }
    if let Some(e) = body.get("refresh_error").and_then(|v| v.as_str()) {
        detail.push_str(&format!(" (refresh failed: {e})"));
    }
    detail
}

/// The secrets-broker token file for direct (laptop-local)
/// connections: the broker's own config dir, sibling of its socket
/// and of the queue client's direct token default (#6218).
pub const DIRECT_SECRETS_TOKEN_FILE: &str = ".config/secrets-broker/token";

/// Source 1b: the secrets-broker protocol, direct over the
/// broker's own Unix socket (#6218) -- the laptop-local path. On
/// the laptop there is no session broker; the secrets broker
/// dual-binds its role socket (#6216) with the same handler, so
/// the exchange is a plain HTTP/1.1 GET. Unlike [`BrokerSource`],
/// which only *names* the token file for the session proxy, this
/// source reads the token file itself -- the value goes into the
/// request header and nowhere else.
pub struct DirectBrokerSource {
    pub socket: PathBuf,
    pub token_file: PathBuf,
    /// Retry behaviour for `resolve` (see [`RetryPolicy`]);
    /// `list_names` never retries, but the policy's per-attempt
    /// timeout bounds its exchange too.
    pub retry_policy: RetryPolicy,
}

impl DirectBrokerSource {
    pub fn new(socket: PathBuf, token_file: PathBuf) -> DirectBrokerSource {
        DirectBrokerSource {
            socket,
            token_file,
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> DirectBrokerSource {
        self.retry_policy = retry_policy;
        self
    }

    /// The direct source for a resolved config, when fleet-config's
    /// probe-based choice for the secrets-broker role selects the
    /// socket (#6218). `None` on the TCP choice (the sandbox): the
    /// caller keeps the session-broker source.
    pub fn from_config(config: &fleet_config::Config) -> Option<DirectBrokerSource> {
        match config.role_transport_choice("secrets-broker") {
            Some(fleet_config::RoleTransport::Socket(socket)) => Some(DirectBrokerSource::new(
                socket,
                fleet_config::Config::home_dir().join(DIRECT_SECRETS_TOKEN_FILE),
            )),
            _ => None,
        }
    }

    /// One GET exchange over the broker socket: status + JSON body.
    fn get(&self, path: &str, name: &str) -> Result<(u16, serde_json::Value), SecretError> {
        use std::io::{Read, Write};
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "secrets-broker",
            name: name.to_string(),
            detail,
        };
        let token = std::fs::read_to_string(&self.token_file).map_err(|e| {
            unavailable(format!(
                "secrets-broker token file {}: {}",
                self.token_file.display(),
                e.kind()
            ))
        })?;
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| {
            unavailable(format!(
                "secrets-broker socket {}: {}",
                self.socket.display(),
                e.kind()
            ))
        })?;
        // Bound the exchange, as in BrokerSource::request: an
        // accept-but-silent broker must fail the attempt so the
        // retry loop (and then the chain) can run.
        if !self.retry_policy.attempt_timeout.is_zero() {
            let _ = stream.set_read_timeout(Some(self.retry_policy.attempt_timeout));
            let _ = stream.set_write_timeout(Some(self.retry_policy.attempt_timeout));
        }
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nX-Agent-Token: {}\r\nConnection: close\r\n\r\n",
            token.trim()
        );
        stream
            .write_all(req.as_bytes())
            .map_err(|e| unavailable(e.to_string()))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| unavailable(e.to_string()))?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let mut parts = text.splitn(2, "\r\n\r\n");
        let head = parts.next().unwrap_or("");
        let status: u16 = head
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| unavailable("secrets-broker bad reply".to_string()))?;
        let body: serde_json::Value =
            serde_json::from_str(parts.next().unwrap_or("")).unwrap_or(serde_json::Value::Null);
        Ok((status, body))
    }

    /// The broker's names listing over the direct socket -- the
    /// same `GET /secrets` route [`BrokerSource::list_names`]
    /// reads through the session broker. Names only.
    pub fn list_names(&self) -> Result<Vec<String>, SecretError> {
        let name = "names";
        let (status, body) = self.get("/secrets", name)?;
        match status {
            200 => {
                let names = body
                    .get("names")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response without a \"names\" array".to_string(),
                    })?;
                names
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| SecretError::Unavailable {
                                source: "secrets-broker",
                                name: name.to_string(),
                                detail: "\"names\" array holds a non-string entry".to_string(),
                            })
                    })
                    .collect()
            }
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: format!("secrets-broker answered HTTP {status}"),
            }),
        }
    }
}

impl SecretSource for DirectBrokerSource {
    fn source_name(&self) -> &'static str {
        "secrets-broker"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        resolve_with_retry(&self.retry_policy, || self.resolve_once(name))
    }
}

impl DirectBrokerSource {
    /// One resolution attempt: a single direct GET and its status
    /// mapping — the same shapes as [`BrokerSource::resolve_once`]
    /// (404 definitive, everything else retryable `Unavailable`).
    fn resolve_once(&self, name: &str) -> Result<SecretValue, SecretError> {
        let (status, body) = self.get(&format!("/secret/{name}"), name)?;
        match status {
            200 => {
                let value = body.get("value").and_then(|v| v.as_str()).ok_or_else(|| {
                    SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response without a string \"value\" field".to_string(),
                    }
                })?;
                if value.is_empty() {
                    return Err(SecretError::Unavailable {
                        source: "secrets-broker",
                        name: name.to_string(),
                        detail: "200 response with an empty \"value\"".to_string(),
                    });
                }
                Ok(SecretValue(value.to_string()))
            }
            404 => Err(SecretError::NotFound {
                source: "secrets-broker",
                name: name.to_string(),
            }),
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: name.to_string(),
                detail: broker_error_detail(status, &body),
            }),
        }
    }
}

/// Runs external commands for the `bws` fallback. Injectable so tests
/// never spawn a real `bws` (and never see a real value).
pub trait CommandRunner {
    /// Run `program args`, returning stdout on success. Error details
    /// carry the exit status only — stderr is deliberately not captured
    /// into errors (it could quote vault material).
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("spawn failed: {}", e.kind()))?;
        if !out.status.success() {
            return Err(format!("exit status {}", out.status));
        }
        String::from_utf8(out.stdout).map_err(|_| "output not utf8".to_string())
    }
}

/// The Bitwarden machine-account token for a `bws` child,
/// resolved from an env value and a home dir: the env value when
/// non-empty, else the on-disk token files (the sandbox's
/// `.config/bitwarden/access-token`, the laptop's
/// `.config/bws/access-token`). This is the secrets broker's own
/// lookup (fleet-queue's `bws_token`), mirrored so the client
/// fallback resolves its credential exactly the way the broker
/// resolves its. The token is returned trimmed; empty files and
/// an empty env value count as absent.
fn bws_token_from(env_value: Option<&str>, home: &Path) -> Option<String> {
    if let Some(t) = env_value {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    for rel in [".config/bitwarden/access-token", ".config/bws/access-token"] {
        if let Ok(t) = std::fs::read_to_string(home.join(rel)) {
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

/// The ambient lookup: this process's env + the fleet home dir.
fn bws_access_token() -> Option<String> {
    bws_token_from(
        std::env::var("BWS_ACCESS_TOKEN").ok().as_deref(),
        &fleet_config::Config::home_dir(),
    )
}

/// Resolve the `bws` program to spawn: an explicit `BWS_BIN`
/// override first, then the ambient PATH, then the well-known
/// install locations a fleet host carries even when the calling
/// process was launched with a stripped PATH. The 2026-11-16
/// incident (#10743): the sandbox secrets-broker was restarted
/// without ~/tools/bin on its PATH, so a bare
/// `Command::new("bws")` failed every vault refresh with spawn
/// NotFound ("entity not found") while the broker served 503s and
/// every `fleet gh` call burned its full retry budget. Falls back
/// to the bare name, so a host with none of these keeps the
/// historical error shape.
pub fn resolve_bws_program() -> PathBuf {
    resolve_bws_program_from(
        std::env::var("BWS_BIN").ok().as_deref(),
        std::env::var_os("PATH").as_deref(),
        &fleet_config::Config::home_dir(),
    )
}

/// A candidate wins only when it is a regular file this process
/// may actually execute: `access(2)` X_OK, not a mode-bit guess —
/// a root-owned 0750 file passes a mode check for a caller outside
/// its group and then fails at spawn (CodeRabbit review, #10757).
fn is_executable_file(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    if !std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false) {
        return false;
    }
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    let rc = unsafe { libc::access(c.as_ptr(), libc::X_OK) };
    rc == 0
}

/// The pure core of [`resolve_bws_program`], split out for tests:
/// a candidate wins only when it is an executable file, in
/// override -> PATH -> well-known-locations order.
fn resolve_bws_program_from(
    env_override: Option<&str>,
    path_var: Option<&std::ffi::OsStr>,
    home: &Path,
) -> PathBuf {
    if let Some(p) = env_override {
        let p = p.trim();
        if !p.is_empty() && is_executable_file(Path::new(p)) {
            // Canonicalize a relative override: the bare name would
            // make Command::new search PATH and could run a
            // different program than the one validated here
            // (CodeRabbit review, #10757).
            return std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
        }
    }
    if let Some(path_var) = path_var {
        for dir in std::env::split_paths(path_var) {
            let cand = dir.join("bws");
            if is_executable_file(&cand) {
                return cand;
            }
        }
    }
    for cand in [
        home.join("tools/bin/bws"),
        home.join(".nix-profile/bin/bws"),
        PathBuf::from("/run/current-system/sw/bin/bws"),
    ] {
        if is_executable_file(&cand) {
            return cand;
        }
    }
    PathBuf::from("bws")
}

/// A [`ProcessRunner`] that hands the child the resolved
/// Bitwarden token explicitly. Before this runner existed, the
/// `bws` fallback inherited the ambient environment only — and
/// non-interactive callers (cron workers, scheduled sweeps) run
/// with `BWS_ACCESS_TOKEN` unset or set-but-empty, so the
/// fallback failed with bws's "missing access token" in exactly
/// the runs that needed it (2026-11-15). The runner always
/// controls the variable: set from the resolved token, or
/// removed when no token resolves, so a stale empty ambient
/// value cannot masquerade as a credential either.
pub struct BwsTokenRunner {
    token: Option<String>,
}

impl BwsTokenRunner {
    pub fn from_environment() -> BwsTokenRunner {
        BwsTokenRunner {
            token: bws_access_token(),
        }
    }

    pub fn with_token(token: Option<String>) -> BwsTokenRunner {
        BwsTokenRunner { token }
    }
}

impl CommandRunner for BwsTokenRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        // The fallback must find `bws` even under a stripped PATH
        // (#10743) -- resolve the bare name through the same
        // well-known locations the broker uses.
        let resolved;
        let program: &Path = if program == "bws" {
            resolved = resolve_bws_program();
            &resolved
        } else {
            Path::new(program)
        };
        let mut cmd = Command::new(program);
        cmd.args(args);
        match &self.token {
            Some(token) => {
                cmd.env("BWS_ACCESS_TOKEN", token);
            }
            None => {
                cmd.env_remove("BWS_ACCESS_TOKEN");
            }
        }
        let out = cmd
            .output()
            .map_err(|e| format!("spawn failed: {}", e.kind()))?;
        if !out.status.success() {
            return Err(format!("exit status {}", out.status));
        }
        String::from_utf8(out.stdout).map_err(|_| "output not utf8".to_string())
    }
}

/// Source 2: the `bws` CLI fallback — `bws secret list` to find the
/// id for a name, then `bws secret get <id>`, exactly the shape the
/// secrets broker's own cache refresh uses.
pub struct BwsSource {
    runner: Box<dyn CommandRunner>,
}

impl BwsSource {
    pub fn new() -> BwsSource {
        BwsSource {
            runner: Box::new(BwsTokenRunner::from_environment()),
        }
    }

    pub fn with_runner(runner: Box<dyn CommandRunner>) -> BwsSource {
        BwsSource { runner }
    }
}

impl Default for BwsSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretSource for BwsSource {
    fn source_name(&self) -> &'static str {
        "bws"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "bws",
            name: name.to_string(),
            detail,
        };
        let list_raw = self
            .runner
            .run("bws", &["secret", "list", "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret list: {d}")))?;
        let list: serde_json::Value = serde_json::from_str(&list_raw)
            .map_err(|_| unavailable("bws secret list: output is not JSON".to_string()))?;
        // A listing that cannot be understood is a source failure
        // (Unavailable), never a confident absence (NotFound): the
        // top level must be an array, and an entry keyed by the
        // requested name demonstrably exists, so a missing or
        // non-string id on THAT entry is malformed, not absent.
        // Entries for other names are not inspected: a malformed
        // id on an unrelated entry does not block this lookup.
        let items = list.as_array().ok_or_else(|| {
            unavailable("bws secret list: output is not a JSON array".to_string())
        })?;
        let mut found_id: Option<String> = None;
        for item in items {
            if item.get("key").and_then(|k| k.as_str()) == Some(name) {
                let raw_id = item.get("id").and_then(|i| i.as_str()).ok_or_else(|| {
                    unavailable(
                        "bws secret list: entry for the requested key has a missing or non-string id"
                            .to_string(),
                    )
                })?;
                found_id = Some(raw_id.to_string());
                break;
            }
        }
        let id = found_id.ok_or_else(|| SecretError::NotFound {
            source: "bws",
            name: name.to_string(),
        })?;
        let got_raw = self
            .runner
            .run("bws", &["secret", "get", &id, "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret get: {d}")))?;
        let got: serde_json::Value = serde_json::from_str(&got_raw)
            .map_err(|_| unavailable("bws secret get: output is not JSON".to_string()))?;
        let value = got
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| unavailable("bws secret get: no string \"value\" field".to_string()))?;
        if value.is_empty() {
            return Err(unavailable(
                "bws secret get: empty \"value\" field".to_string(),
            ));
        }
        Ok(SecretValue(value.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSource {
        name: &'static str,
        result: Result<String, SecretError>,
    }

    impl SecretSource for FakeSource {
        fn source_name(&self) -> &'static str {
            self.name
        }
        fn resolve(&self, _name: &str) -> Result<SecretValue, SecretError> {
            match &self.result {
                Ok(v) => Ok(SecretValue(v.clone())),
                Err(_) => Err(SecretError::Unavailable {
                    source: self.name,
                    name: "x".to_string(),
                    detail: "down".to_string(),
                }),
            }
        }
    }

    #[test]
    fn chain_tries_sources_in_order() {
        let resolver = Resolver::new(vec![
            Box::new(FakeSource {
                name: "secrets-broker",
                result: Err(SecretError::Unavailable {
                    source: "secrets-broker",
                    name: "x".into(),
                    detail: "down".into(),
                }),
            }),
            Box::new(FakeSource {
                name: "bws",
                result: Ok("fixture-value-not-a-secret".to_string()),
            }),
        ]);
        let v = resolver.resolve("some.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn chain_skips_an_empty_value_and_uses_the_next_source() {
        let resolver = Resolver::new(vec![
            Box::new(FakeSource {
                name: "secrets-broker",
                result: Ok(String::new()),
            }),
            Box::new(FakeSource {
                name: "bws",
                result: Ok("fixture-value-not-a-secret".to_string()),
            }),
        ]);
        let v = resolver.resolve("some.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn chain_of_only_empty_values_fails_loudly() {
        let resolver = Resolver::new(vec![Box::new(FakeSource {
            name: "secrets-broker",
            result: Ok(String::new()),
        })]);
        let err = resolver.resolve("some.name").unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains("empty value"),
            "the failure names the empty read: {text}"
        );
    }

    #[test]
    fn chain_reports_all_failures_without_values() {
        let resolver = Resolver::new(vec![Box::new(FakeSource {
            name: "bws",
            result: Err(SecretError::Unavailable {
                source: "bws",
                name: "x".into(),
                detail: "down".into(),
            }),
        })]);
        let err = resolver.resolve("some.name").unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("some.name"), "error names the secret: {text}");
        assert!(
            !text.contains("fixture"),
            "error must not carry values: {text}"
        );
    }

    #[test]
    fn debug_is_redacted() {
        let v = SecretValue("fixture-value-not-a-secret".to_string());
        let text = format!("{v:?}");
        assert_eq!(text, "SecretValue(***)");
    }

    #[test]
    fn name_validation_rejects_path_material() {
        for bad in [
            "", "a/b", "a?b", "a#b", "a b", "a\tb", "a\u{7}b", ".", "..", "%2e%2e", "%2f", "a%2fb",
            "a%3fb", "a%23b",
        ] {
            assert!(validate_name(bad).is_err(), "name {bad:?} must be rejected");
        }
        for good in [
            "turso.fleet-queue-rw-token",
            "openclaw.gateway-token",
            "a.b-c_d",
        ] {
            assert!(validate_name(good).is_ok(), "name {good:?} must pass");
        }
    }

    struct FakeRunner {
        list: String,
        get: String,
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            assert_eq!(program, "bws");
            match args {
                ["secret", "list", "-o", "json"] => Ok(self.list.clone()),
                ["secret", "get", _, "-o", "json"] => Ok(self.get.clone()),
                other => Err(format!("unexpected args: {other:?}")),
            }
        }
    }

    #[test]
    fn bws_fallback_lists_then_gets_by_name() {
        let source = BwsSource::with_runner(Box::new(FakeRunner {
            list: r#"[{"id":"id-1","key":"wanted.name"},{"id":"id-2","key":"other.name"}]"#
                .to_string(),
            get: r#"{"id":"id-1","key":"wanted.name","value":"fixture-value-not-a-secret"}"#
                .to_string(),
        }));
        let v = source.resolve("wanted.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
        match source.resolve("absent.name") {
            Err(SecretError::NotFound { source, .. }) => assert_eq!(source, "bws"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn bws_fallback_rejects_an_empty_value() {
        let source = BwsSource::with_runner(Box::new(FakeRunner {
            list: r#"[{"id":"id-1","key":"wanted.name"}]"#.to_string(),
            get: r#"{"id":"id-1","key":"wanted.name","value":""}"#.to_string(),
        }));
        match source.resolve("wanted.name") {
            Err(SecretError::Unavailable { source, detail, .. }) => {
                assert_eq!(source, "bws");
                assert!(detail.contains("empty"), "detail: {detail}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    fn bws_resolve_with_list(list: &str) -> Result<SecretValue, SecretError> {
        let source = BwsSource::with_runner(Box::new(FakeRunner {
            list: list.to_string(),
            get: r#"{"id":"id-1","key":"wanted.name","value":"fixture-value-not-a-secret"}"#
                .to_string(),
        }));
        source.resolve("wanted.name")
    }

    #[test]
    fn bws_fallback_non_array_listing_is_unavailable_not_notfound() {
        // #6577: valid JSON, wrong shape (object / string / number).
        for list in [r#"{"id":"id-1","key":"wanted.name"}"#, r#""hello""#, "42"] {
            match bws_resolve_with_list(list) {
                Err(SecretError::Unavailable { source, detail, .. }) => {
                    assert_eq!(source, "bws");
                    assert!(detail.contains("not a JSON array"), "detail: {detail}");
                }
                other => panic!("list {list}: expected Unavailable, got {other:?}"),
            }
        }
    }

    #[test]
    fn bws_fallback_key_match_with_bad_id_is_unavailable_not_notfound() {
        // #6577: the key matches, so the secret exists; its record
        // is malformed (numeric / missing / null id).
        for list in [
            r#"[{"id":123,"key":"wanted.name"}]"#,
            r#"[{"key":"wanted.name"}]"#,
            r#"[{"id":null,"key":"wanted.name"}]"#,
        ] {
            match bws_resolve_with_list(list) {
                Err(SecretError::Unavailable { source, detail, .. }) => {
                    assert_eq!(source, "bws");
                    assert!(detail.contains("non-string id"), "detail: {detail}");
                }
                other => panic!("list {list}: expected Unavailable, got {other:?}"),
            }
        }
    }

    #[test]
    fn bws_fallback_genuine_absence_stays_notfound() {
        // #6577 controls: empty list, and a well-formed list with no
        // entry for the name. A malformed id on an entry for ANOTHER
        // name does not block a lookup it is not part of.
        for list in [
            "[]",
            r#"[{"id":"id-2","key":"other.name"}]"#,
            r#"[{"id":123,"key":"other.name"}]"#,
        ] {
            match bws_resolve_with_list(list) {
                Err(SecretError::NotFound { source, .. }) => assert_eq!(source, "bws"),
                other => panic!("list {list}: expected NotFound, got {other:?}"),
            }
        }
        // ...and a malformed unrelated entry does not block a hit.
        let v = bws_resolve_with_list(
            r#"[{"id":123,"key":"other.name"},{"id":"id-1","key":"wanted.name"}]"#,
        )
        .unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn direct_broker_source_roundtrip_over_a_temp_socket() {
        // #6218: the laptop-local source speaks plain HTTP over
        // the broker's own socket, reading its token from the file.
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("fleet-secrets-direct-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("secrets.sock");
        let token_file = dir.join("token");
        std::fs::write(&token_file, "test-token\n").unwrap();
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                assert!(req.contains("X-Agent-Token: test-token"), "req: {req}");
                let body = if req.starts_with("GET /secrets") {
                    "{\"names\":[\"a.name\",\"b.name\"]}"
                } else {
                    assert!(req.starts_with("GET /secret/wanted.name"), "req: {req}");
                    "{\"value\":\"direct-value\"}"
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });
        let source = DirectBrokerSource::new(sock, token_file).with_retry_policy(RetryPolicy::NONE);
        let value = source.resolve("wanted.name").unwrap();
        assert_eq!(value.expose(), "direct-value");
        let names = source.list_names().unwrap();
        assert_eq!(names, vec!["a.name".to_string(), "b.name".to_string()]);
        handle.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn direct_broker_source_404_is_notfound() {
        // #6218: the direct leg maps statuses like the proxied one.
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("fleet-secrets-direct-404-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("secrets.sock");
        let token_file = dir.join("token");
        std::fs::write(&token_file, "test-token\n").unwrap();
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .unwrap();
        });
        let source = DirectBrokerSource::new(sock, token_file).with_retry_policy(RetryPolicy::NONE);
        assert!(matches!(
            source.resolve("missing.name"),
            Err(SecretError::NotFound { .. })
        ));
        handle.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broker_list_names_roundtrip_over_a_temp_socket() {
        use std::os::unix::net::UnixListener;

        let dir =
            std::env::temp_dir().join(format!("fleet-secrets-list-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("broker.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let req: Request = fleet_proxy_proto::read_typed(&mut stream).unwrap();
            assert_eq!(req.method, "GET");
            assert_eq!(req.path, "/secrets");
            assert_eq!(req.port, Some(fleet_config::PORT_SECRETS_BROKER));
            let resp = Response {
                status: 200,
                body: serde_json::json!({"names": ["a.name", "b.name"]}),
                transport: None,
            };
            fleet_proxy_proto::write_frame(&mut stream, &resp).unwrap();
        });
        let source = BrokerSource::new(sock, PathBuf::from("/nonexistent/token"));
        let names = source.list_names().unwrap();
        assert_eq!(names, vec!["a.name".to_string(), "b.name".to_string()]);
        handle.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broker_resolve_rejects_an_empty_value_over_a_temp_socket() {
        use std::os::unix::net::UnixListener;

        fn scripted_resolve(body: serde_json::Value) -> Result<SecretValue, SecretError> {
            let dir = std::env::temp_dir().join(format!(
                "fleet-secrets-resolve-test-{}-{}",
                std::process::id(),
                body["value"].as_str().map_or(0, str::len)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let sock = dir.join("broker.sock");
            let listener = UnixListener::bind(&sock).unwrap();
            let handle = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let req: Request = fleet_proxy_proto::read_typed(&mut stream).unwrap();
                assert_eq!(req.path, "/secret/wanted.name");
                let resp = Response {
                    status: 200,
                    body,
                    transport: None,
                };
                fleet_proxy_proto::write_frame(&mut stream, &resp).unwrap();
            });
            let source = BrokerSource::new(sock, PathBuf::from("/nonexistent/token"))
                .with_retry_policy(RetryPolicy::NONE);
            let out = source.resolve("wanted.name");
            handle.join().unwrap();
            std::fs::remove_dir_all(&dir).ok();
            out
        }

        let ok = scripted_resolve(serde_json::json!({
            "key": "wanted.name",
            "value": "fixture-value-not-a-secret"
        }))
        .unwrap();
        assert_eq!(ok.expose(), "fixture-value-not-a-secret");

        match scripted_resolve(serde_json::json!({"key": "wanted.name", "value": ""})) {
            Err(SecretError::Unavailable { source, detail, .. }) => {
                assert_eq!(source, "secrets-broker");
                assert!(detail.contains("empty"), "detail: {detail}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn broker_source_roundtrip_over_a_socket_pair() {
        // A real Unix socket pair with a scripted broker end: proves
        // BrokerSource's framed exchange + status mapping (200, 404)
        // without any live service or real secret.
        use std::os::unix::net::UnixStream as Pair;
        use std::path::PathBuf;

        fn scripted_broker(
            status: u16,
            body: serde_json::Value,
        ) -> (BrokerSource, std::thread::JoinHandle<()>, Pair) {
            let (client, mut server) = Pair::pair().unwrap();
            let handle = std::thread::spawn(move || {
                let req: Request = fleet_proxy_proto::read_typed(&mut server).unwrap();
                assert_eq!(req.method, "GET");
                assert_eq!(req.path, "/secret/wanted.name");
                assert_eq!(req.port, Some(fleet_config::PORT_SECRETS_BROKER));
                assert_eq!(req.token_file.as_deref(), Some("/nonexistent/token"));
                let resp = Response {
                    status,
                    body,
                    transport: None,
                };
                fleet_proxy_proto::write_frame(&mut server, &resp).unwrap();
            });
            let source = BrokerSource::new(
                PathBuf::from("/nonexistent/broker.sock"),
                PathBuf::from("/nonexistent/token"),
            );
            (source, handle, client)
        }

        // 200 -> value
        let (source, handle, mut client) = scripted_broker(
            200,
            serde_json::json!({"key": "wanted.name", "value": "fixture-value-not-a-secret"}),
        );
        let request = Request {
            method: "GET".to_string(),
            path: "/secret/wanted.name".to_string(),
            body: None,
            port: Some(fleet_config::PORT_SECRETS_BROKER),
            token_file: Some("/nonexistent/token".to_string()),
            trip: None,
            transport: None,
            cf_ok: None,
        };
        let resp = source
            .exchange(&mut client, &request, "wanted.name")
            .unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body["value"], "fixture-value-not-a-secret");
        handle.join().unwrap();

        // The resolve() status mapping, exercised against exchange()
        // results by shape: 404 maps to NotFound in resolve; here we
        // assert the raw status the mapping consumes.
        let (source, handle, mut client) =
            scripted_broker(404, serde_json::json!({"error": "not found"}));
        let resp = source
            .exchange(&mut client, &request, "wanted.name")
            .unwrap();
        assert_eq!(resp.status, 404);
        handle.join().unwrap();
    }

    /// A unique temp dir for a socket-level test.
    #[test]
    fn bws_program_resolution_order() {
        // oracle (#10743): override > PATH > well-known home
        // location, a candidate wins only when it is an executable
        // file, and with nothing anywhere the bare name is kept
        // (the historical error shape).
        use std::os::unix::fs::PermissionsExt;
        let dir = test_temp_dir("bws-resolve");
        let override_dir = dir.join("override");
        let path_dir = dir.join("pathbin");
        let home = dir.join("home");
        for d in [&override_dir, &path_dir, &home.join("tools/bin")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mk = |p: &Path| {
            std::fs::write(p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let override_bws = override_dir.join("bws");
        let path_bws = path_dir.join("bws");
        let home_bws = home.join("tools/bin/bws");
        mk(&override_bws);
        mk(&path_bws);
        mk(&home_bws);
        let path_var = std::env::join_paths([&path_dir]).unwrap();
        assert_eq!(
            resolve_bws_program_from(Some(override_bws.to_str().unwrap()), Some(&path_var), &home),
            std::fs::canonicalize(&override_bws).unwrap()
        );
        assert_eq!(
            resolve_bws_program_from(None, Some(&path_var), &home),
            path_bws
        );
        assert_eq!(resolve_bws_program_from(None, None, &home), home_bws);
        // A non-executable override is skipped, not trusted.
        let plain = dir.join("plain-bws");
        std::fs::write(&plain, "x").unwrap();
        assert_eq!(
            resolve_bws_program_from(Some(plain.to_str().unwrap()), Some(&path_var), &home),
            path_bws
        );
        // Nothing in the override, PATH, or home: the NixOS system
        // location when the host has one, else the bare name.
        let empty_home = dir.join("empty-home");
        std::fs::create_dir_all(&empty_home).unwrap();
        let sys = Path::new("/run/current-system/sw/bin/bws");
        let expected = if is_executable_file(sys) {
            sys.to_path_buf()
        } else {
            PathBuf::from("bws")
        };
        assert_eq!(resolve_bws_program_from(None, None, &empty_home), expected);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fleet-secrets-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fast policy for retry tests: millisecond backoff, a
    /// budget wide enough for the scripted failures, and a
    /// per-attempt timeout that cannot stall the suite.
    fn test_retry_policy() -> RetryPolicy {
        RetryPolicy {
            budget: Duration::from_secs(10),
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
            attempt_timeout: Duration::from_secs(2),
        }
    }

    #[test]
    fn retry_loop_retries_only_unavailable() {
        use std::cell::Cell;
        // NotFound is definitive: exactly one attempt.
        let calls = Cell::new(0);
        let out = resolve_with_retry(&test_retry_policy(), || {
            calls.set(calls.get() + 1);
            Err(SecretError::NotFound {
                source: "secrets-broker",
                name: "x".to_string(),
            })
        });
        assert!(matches!(out, Err(SecretError::NotFound { .. })));
        assert_eq!(calls.get(), 1);

        // Unavailable twice, then success.
        let calls = Cell::new(0);
        let out = resolve_with_retry(&test_retry_policy(), || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(SecretError::Unavailable {
                    source: "secrets-broker",
                    name: "x".to_string(),
                    detail: "down".to_string(),
                })
            } else {
                Ok(SecretValue("fixture-value-not-a-secret".to_string()))
            }
        });
        assert_eq!(out.unwrap().expose(), "fixture-value-not-a-secret");
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn retry_loop_spends_its_budget_and_returns_the_last_error() {
        use std::cell::Cell;
        let policy = RetryPolicy {
            budget: Duration::from_millis(30),
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            attempt_timeout: Duration::from_millis(1),
        };
        let calls = Cell::new(0);
        let started = Instant::now();
        let out = resolve_with_retry(&policy, || {
            calls.set(calls.get() + 1);
            Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: "x".to_string(),
                detail: "down".to_string(),
            })
        });
        assert!(matches!(out, Err(SecretError::Unavailable { .. })));
        assert!(calls.get() > 1, "the budget buys more than one attempt");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn retry_policy_none_is_a_single_attempt() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        let out = resolve_with_retry(&RetryPolicy::NONE, || {
            calls.set(calls.get() + 1);
            Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: "x".to_string(),
                detail: "down".to_string(),
            })
        });
        assert!(matches!(out, Err(SecretError::Unavailable { .. })));
        assert_eq!(calls.get(), 1);
    }

    /// A scripted session broker on a temp socket: answers the
    /// given (status, body) replies in order, one connection per
    /// reply, and reports how many requests it served.
    fn scripted_broker_replies(
        replies: Vec<(u16, serde_json::Value)>,
    ) -> (BrokerSource, std::thread::JoinHandle<usize>, PathBuf) {
        use std::os::unix::net::UnixListener;
        let dir = test_temp_dir("retry");
        let sock = dir.join("broker.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let req: Request = fleet_proxy_proto::read_typed(&mut stream).unwrap();
                assert_eq!(req.path, "/secret/wanted.name");
                let resp = Response {
                    status,
                    body,
                    transport: None,
                };
                fleet_proxy_proto::write_frame(&mut stream, &resp).unwrap();
                served += 1;
            }
            served
        });
        let source = BrokerSource::new(sock, PathBuf::from("/nonexistent/token"))
            .with_retry_policy(test_retry_policy());
        (source, handle, dir)
    }

    #[test]
    fn broker_retries_unavailable_statuses_then_resolves() {
        // The 2026-11-15 shape: while the broker and its tunnel
        // restart, the leg answers 503 (cache not loaded) and 502
        // (upstream path down), then serves. The caller rides out
        // the window instead of failing over.
        let (source, handle, dir) = scripted_broker_replies(vec![
            (503, serde_json::json!({"error": "not loaded"})),
            (502, serde_json::json!({"error": "bad gateway"})),
            (
                200,
                serde_json::json!({"key": "wanted.name", "value": "fixture-value-not-a-secret"}),
            ),
        ]);
        let v = source.resolve("wanted.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
        assert_eq!(handle.join().unwrap(), 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broker_404_is_definitive_and_never_retried() {
        // Even with the full production budget, a 404 returns
        // after exactly one attempt: it is an answer about the
        // name, not a failure to answer.
        let (source, handle, dir) =
            scripted_broker_replies(vec![(404, serde_json::json!({"error": "not found"}))]);
        let source = source.with_retry_policy(RetryPolicy::default());
        let started = Instant::now();
        let out = source.resolve("wanted.name");
        assert!(
            matches!(out, Err(SecretError::NotFound { .. })),
            "got {out:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(handle.join().unwrap(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broker_silent_peer_fails_each_attempt_on_the_attempt_timeout() {
        // A peer that accepts and never answers must fail the
        // attempt (on the per-attempt timeout), not wedge the
        // caller: before the timeout existed this read blocked
        // forever and neither retry nor fallback could run.
        use std::os::unix::net::UnixListener;
        let dir = test_temp_dir("silent");
        let sock = dir.join("broker.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                held.push(stream);
            }
        });
        let policy = RetryPolicy {
            budget: Duration::from_millis(600),
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
            attempt_timeout: Duration::from_millis(120),
        };
        let source =
            BrokerSource::new(sock, PathBuf::from("/nonexistent/token")).with_retry_policy(policy);
        let started = Instant::now();
        let out = source.resolve("wanted.name");
        assert!(
            matches!(out, Err(SecretError::Unavailable { .. })),
            "got {out:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "attempts must be time-bounded"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn chain_falls_through_to_bws_once_broker_retries_are_spent() {
        // Broker hard down (no socket at all): the broker leg
        // spends its (here, tiny) retry budget, then the chain
        // reaches the `bws` fallback — the rescue that failed on
        // 2026-11-15 for the token reason, exercised here with a
        // working fallback behind it.
        let broker = BrokerSource::new(
            PathBuf::from("/nonexistent/broker.sock"),
            PathBuf::from("/nonexistent/token"),
        )
        .with_retry_policy(RetryPolicy {
            budget: Duration::from_millis(50),
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            attempt_timeout: Duration::from_millis(50),
        });
        let resolver = Resolver::new(vec![
            Box::new(broker),
            Box::new(BwsSource::with_runner(Box::new(FakeRunner {
                list: r#"[{"id":"id-1","key":"wanted.name"}]"#.to_string(),
                get: r#"{"id":"id-1","key":"wanted.name","value":"fixture-value-not-a-secret"}"#
                    .to_string(),
            }))),
        ]);
        let v = resolver.resolve("wanted.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
    }

    #[test]
    fn direct_broker_source_retries_a_503_then_resolves() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        let dir = test_temp_dir("direct-retry");
        let sock = dir.join("secrets.sock");
        let token_file = dir.join("token");
        std::fs::write(&token_file, "test-token\n").unwrap();
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            for status in [503u16, 200] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let (line, body) = if status == 200 {
                    ("HTTP/1.1 200 OK", "{\"value\":\"direct-value\"}")
                } else {
                    ("HTTP/1.1 503 Service Unavailable", "{}")
                };
                let resp = format!(
                    "{line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });
        let source =
            DirectBrokerSource::new(sock, token_file).with_retry_policy(test_retry_policy());
        let v = source.resolve("wanted.name").unwrap();
        assert_eq!(v.expose(), "direct-value");
        handle.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bws_token_prefers_a_nonempty_env_value() {
        let dir = test_temp_dir("token-env");
        assert_eq!(
            bws_token_from(Some("  env-token  "), &dir).as_deref(),
            Some("env-token")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bws_token_falls_back_to_the_token_files() {
        let dir = test_temp_dir("token-file");
        let config_dir = dir.join(".config/bitwarden");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("access-token"), "file-token\n").unwrap();
        assert_eq!(bws_token_from(None, &dir).as_deref(), Some("file-token"));
        // An empty (or whitespace) env value counts as absent —
        // the 2026-11-15 shape, where the variable was present
        // but empty and the fallback died on it.
        assert_eq!(
            bws_token_from(Some(""), &dir).as_deref(),
            Some("file-token")
        );
        assert_eq!(
            bws_token_from(Some("   "), &dir).as_deref(),
            Some("file-token")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bws_token_absent_everywhere_is_none() {
        let dir = test_temp_dir("token-none");
        assert_eq!(bws_token_from(None, &dir), None);
        assert_eq!(bws_token_from(Some(""), &dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bws_token_runner_controls_the_child_env() {
        // /bin/sh reports whether the variable is set in its
        // environment: the runner must set it from the resolved
        // token, and must remove it when no token resolved (so a
        // stale empty ambient value cannot leak through either).
        let probe = |runner: &BwsTokenRunner| -> String {
            runner
                .run(
                    "sh",
                    &[
                        "-c",
                        "if [ -n \"$BWS_ACCESS_TOKEN\" ]; then echo set; else echo unset; fi",
                    ],
                )
                .unwrap()
                .trim()
                .to_string()
        };
        assert_eq!(
            probe(&BwsTokenRunner::with_token(Some(
                "fixture-token".to_string()
            ))),
            "set"
        );
        assert_eq!(probe(&BwsTokenRunner::with_token(None)), "unset");
    }
}
