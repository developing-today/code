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
//!      same shape the secrets broker itself uses to fill its cache;
//!   3. the **`op` CLI**, only where a vault is named
//!      (`FLEET_OP_VAULT`): `op read op://<vault>/<name>/<field>`.
//!      A seat that keeps its secrets in 1Password resolves the same
//!      names without a broker or a Bitwarden token ([`OpSource`]).
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
//!
//! Backend selection (#14151): the secrets broker is Bitwarden-only
//! (its refresh and read-through speak `bws`; see
//! `fleet-queue`'s `secrets_broker`). 1Password stays a
//! **client-chain-only** source ([`OpSource`]): the broker serves
//! one shared vault cache keyed by name, and an op backend would
//! need a second per-seat credential, a second refresh loop, and a
//! merge rule for names present in both vaults — all for a seat
//! that already resolves the same names locally through this
//! crate. The broker's `/secrets` listing therefore never carries
//! a 1Password-only name; 1Password-only seats resolve through
//! the client chain below.

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
    /// The store/vault selection itself is not usable (#10358 S3):
    /// an unknown store, or a vault with no store. The verb maps it
    /// to a usage error (exit 2). Carries no secret material.
    InvalidSelection(String),
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
            SecretError::InvalidSelection(detail) => {
                write!(f, "invalid secrets store selection: {detail}")
            }
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

/// A namespace view over secret names (#10358 design record,
/// `doc/2026-11-26T07-45-48Z_design_fleet_secrets_scoping/`, D1 to D5).
///
/// A prefix is a namespace view, not a security layer: what a device
/// can read is bounded by the keys it holds, never by the prefix.
/// [`Scope::none`] is the identity, so an unconfigured instance
/// behaves exactly as it did before scoping existed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    prefix: Option<String>,
}

impl Scope {
    /// The one separator between a prefix and the rest of a name.
    pub const SEPARATOR: char = '.';
    /// The environment variable that names the prefix; it beats
    /// `[secrets] prefix` in `fleet.toml` (D9).
    pub const PREFIX_ENV: &'static str = "FLEET_SECRETS_PREFIX";

    /// No prefix: every name maps and displays as itself (D1).
    pub fn none() -> Scope {
        Scope { prefix: None }
    }

    /// A prefix scope. The prefix must itself be a valid name and may
    /// not end with the separator; a dotted prefix such as `team.42`
    /// is legal and matched as a whole (D9).
    pub fn prefixed(prefix: &str) -> Result<Scope, SecretError> {
        validate_name(prefix)?;
        if prefix.ends_with(Self::SEPARATOR) {
            return Err(SecretError::InvalidName(prefix.to_string()));
        }
        Ok(Scope {
            prefix: Some(prefix.to_string()),
        })
    }

    /// The scope from an environment value and a config value: the
    /// environment wins, and an empty or blank value in either counts
    /// as unset (D9).
    pub fn from_sources(
        env_value: Option<&str>,
        config_value: Option<&str>,
    ) -> Result<Scope, SecretError> {
        fn set(v: Option<&str>) -> Option<&str> {
            v.filter(|s| !s.trim().is_empty())
        }
        match set(env_value).or_else(|| set(config_value)) {
            Some(prefix) => Scope::prefixed(prefix),
            None => Ok(Scope::none()),
        }
    }

    /// The scope a resolved config runs with: `$FLEET_SECRETS_PREFIX`,
    /// else `[secrets] prefix`, else none.
    pub fn from_config(config: &fleet_config::Config) -> Result<Scope, SecretError> {
        Scope::from_sources(
            std::env::var(Self::PREFIX_ENV).ok().as_deref(),
            config.secrets.prefix.as_deref(),
        )
    }

    /// The configured prefix, if any.
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// `P.`, the start every stored name in this scope shares.
    fn lead(prefix: &str) -> String {
        format!("{prefix}{}", Self::SEPARATOR)
    }

    /// The stored name a caller's name means (D2 to D4). A name that
    /// already starts with `P.` is full and used as is; any other name
    /// gains `P.` once. The full name is validated, and a full name
    /// with nothing after `P.` is refused.
    pub fn to_full(&self, name: &str) -> Result<String, SecretError> {
        validate_name(name)?;
        let Some(prefix) = self.prefix.as_deref() else {
            return Ok(name.to_string());
        };
        let lead = Scope::lead(prefix);
        let full = if name.starts_with(&lead) {
            name.to_string()
        } else {
            format!("{lead}{name}")
        };
        if full.len() == lead.len() {
            return Err(SecretError::InvalidName(name.to_string()));
        }
        validate_name(&full)?;
        Ok(full)
    }

    /// How a stored name is shown in this scope (D5), or `None` when it
    /// lies outside the scope. `P.` is stripped, except where the rest
    /// itself starts with `P.` or is a bare dot segment (`.` or `..`):
    /// that name is shown in full, so every displayed name maps back
    /// through [`Scope::to_full`] to the stored name it came from.
    pub fn display(&self, stored: &str) -> Option<String> {
        let Some(prefix) = self.prefix.as_deref() else {
            return Some(stored.to_string());
        };
        let lead = Scope::lead(prefix);
        let rest = stored.strip_prefix(&lead)?;
        if rest.is_empty() {
            return None;
        }
        // A rest that starts with `P.` would gain the prefix twice, and
        // a rest that is a bare dot segment is not a valid name on its
        // own: both are shown in full so they still map back.
        if rest.starts_with(&lead) || rest == "." || rest == ".." {
            Some(stored.to_string())
        } else {
            Some(rest.to_string())
        }
    }
}

/// Which store a call reads from (#10358 D8, S3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StoreKind {
    /// No store named anywhere: today's chain exactly (D1).
    #[default]
    Auto,
    /// Broker legs and `bws`; never `op`.
    Bitwarden,
    /// `op` only.
    OnePassword,
}

/// A resolved store + vault selection (#10358 D8/D9, S3): which
/// chain a call uses, and the vault inside that store — a Bitwarden
/// project id for [`StoreKind::Bitwarden`], a 1Password vault for
/// [`StoreKind::OnePassword`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreSelection {
    pub store: StoreKind,
    pub vault: Option<String>,
}

impl StoreSelection {
    /// The environment variable that names the store; it beats
    /// `[secrets] store` in `fleet.toml` (D9).
    pub const STORE_ENV: &'static str = "FLEET_SECRETS_STORE";
    /// The environment variable that names the vault; it beats
    /// `[secrets] vault` in `fleet.toml` (D9).
    pub const VAULT_ENV: &'static str = "FLEET_SECRETS_VAULT";

    /// No selection: the historical chain exactly (D1).
    pub fn auto() -> StoreSelection {
        StoreSelection::default()
    }

    /// Resolve one setting from its three sources in D9 precedence:
    /// flag, then environment, then config. An empty or blank value
    /// in any source counts as unset and falls through.
    fn pick<'a>(
        flag: Option<&'a str>,
        env: Option<&'a str>,
        config: Option<&'a str>,
    ) -> Option<&'a str> {
        [flag, env, config]
            .into_iter()
            .flatten()
            .find(|v| !v.trim().is_empty())
            .map(|v| v.trim())
    }

    /// The selection from explicit flag, environment and config
    /// values (each the raw value of its source, `None` when the
    /// source does not name one). An unknown store, or a vault with
    /// no store, is [`SecretError::InvalidSelection`].
    pub fn from_sources(
        flag_store: Option<&str>,
        flag_vault: Option<&str>,
        env_store: Option<&str>,
        env_vault: Option<&str>,
        config_store: Option<&str>,
        config_vault: Option<&str>,
    ) -> Result<StoreSelection, SecretError> {
        let store_raw = Self::pick(flag_store, env_store, config_store);
        let vault = Self::pick(flag_vault, env_vault, config_vault).map(str::to_string);
        let store = match store_raw {
            None => StoreKind::Auto,
            Some("bitwarden") => StoreKind::Bitwarden,
            Some("1password") => StoreKind::OnePassword,
            Some(other) => {
                return Err(SecretError::InvalidSelection(format!(
                    "unknown store {other:?}: expected \"bitwarden\" or \"1password\""
                )));
            }
        };
        if vault.is_some() && store == StoreKind::Auto {
            return Err(SecretError::InvalidSelection(
                "a vault is set but no store is: set a store (\"bitwarden\" or \
                 \"1password\") alongside the vault"
                    .to_string(),
            ));
        }
        Ok(StoreSelection { store, vault })
    }

    /// The selection for a resolved config plus per-call flags:
    /// `$FLEET_SECRETS_STORE` / `$FLEET_SECRETS_VAULT` sit between
    /// the flags and `[secrets] store` / `vault` (D9).
    pub fn from_flags_and_config(
        flag_store: Option<&str>,
        flag_vault: Option<&str>,
        config: &fleet_config::Config,
    ) -> Result<StoreSelection, SecretError> {
        let env_store = std::env::var(Self::STORE_ENV).ok();
        let env_vault = std::env::var(Self::VAULT_ENV).ok();
        Self::from_sources(
            flag_store,
            flag_vault,
            env_store.as_deref(),
            env_vault.as_deref(),
            config.secrets.store.as_deref(),
            config.secrets.vault.as_deref(),
        )
    }

    /// The selection a resolved config runs with when no per-call
    /// flags exist (the bridge's reads and writes).
    pub fn from_config(config: &fleet_config::Config) -> Result<StoreSelection, SecretError> {
        Self::from_flags_and_config(None, None, config)
    }

    /// The Bitwarden project a `bitwarden` selection is scoped to,
    /// if a vault is set.
    pub fn bitwarden_project(&self) -> Option<&str> {
        match self.store {
            StoreKind::Bitwarden => self.vault.as_deref(),
            _ => None,
        }
    }
}

/// The pseudo-name a listing's errors carry: a listing has no single
/// secret to name.
const LIST_PSEUDO_NAME: &str = "names";

/// The pseudo-name a triggered refresh's errors carry: a refresh
/// names no secret.
const REFRESH_PSEUDO_NAME: &str = "refresh";

/// The pseudo-name a health read's errors carry: a health probe
/// names no secret.
const HEALTH_PSEUDO_NAME: &str = "health";

/// The broker's liveness record (`GET /health`), as the `refresh`
/// verb reports it (#13948 S5): whether the broker is serving, how
/// many secrets it holds, whether a refresh is in flight, which leg
/// served the last completed cycle (`peer`, `upstream-delta`,
/// `upstream-full`, or `None` before the first success), and the
/// durable watermark that cycle installed. Names, counts and the
/// leg only — no values, no hashes, so the record is safe to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerHealth {
    pub serving: bool,
    pub secrets: usize,
    pub refreshing: bool,
    pub refresh_leg: Option<String>,
    pub last_refresh_unix: Option<u64>,
}

impl BrokerHealth {
    /// Parse a `/health` body strictly: `serving`, `secrets` and
    /// `refreshing` are required (every broker since #13081 serves
    /// them); `refresh_leg` and `last_refresh_unix` are optional so
    /// the parse also accepts pre-S5 brokers. A non-string
    /// `refresh_leg` is a source failure, never silently dropped.
    pub fn parse(body: &serde_json::Value) -> Result<BrokerHealth, SecretError> {
        let unavailable = |detail: &str| SecretError::Unavailable {
            source: "secrets-broker",
            name: HEALTH_PSEUDO_NAME.to_string(),
            detail: detail.to_string(),
        };
        let serving = body
            .get("serving")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| unavailable("health reply has no boolean \"serving\" field"))?;
        let secrets = body
            .get("secrets")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| unavailable("health reply has no numeric \"secrets\" field"))?;
        let refreshing = body
            .get("refreshing")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| unavailable("health reply has no boolean \"refreshing\" field"))?;
        let refresh_leg = match body.get("refresh_leg") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(leg)) => Some(leg.clone()),
            Some(_) => return Err(unavailable("health reply has a non-string \"refresh_leg\"")),
        };
        let last_refresh_unix = body.get("last_refresh_unix").and_then(|v| v.as_u64());
        Ok(BrokerHealth {
            serving,
            secrets: secrets as usize,
            refreshing,
            refresh_leg,
            last_refresh_unix,
        })
    }
}

/// The broker route a listing in `scope` asks for (#10358 D6). An
/// unscoped listing never sends the parameter, so it works against old
/// and new brokers alike.
pub fn list_path(scope: &Scope) -> String {
    list_path_scoped(scope, &StoreSelection::auto())
}

/// The broker route a listing in `scope` under `selection` asks for
/// (#10358 S3): beside `prefix`, a `bitwarden` selection with a vault
/// sends `project=<id>`, so the broker answers only that project's
/// names. With neither set the path is exactly `/secrets`.
pub fn list_path_scoped(scope: &Scope, selection: &StoreSelection) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(prefix) = scope.prefix() {
        params.push(format!("prefix={}", query_encode(prefix)));
    }
    if let Some(project) = selection.bitwarden_project() {
        params.push(format!("project={}", query_encode(project)));
    }
    if params.is_empty() {
        "/secrets".to_string()
    } else {
        format!("/secrets?{}", params.join("&"))
    }
}

/// Percent-encode everything outside RFC 3986's unreserved set, so a
/// prefix holding `&`, `=` or `+` stays one query value.
fn query_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The names a broker listing answered. A scoped listing that the
/// broker does not know (404) is unresolved and never retried
/// unscoped (#10358 D6).
fn names_from_listing(
    status: u16,
    body: &serde_json::Value,
    scope: &Scope,
) -> Result<Vec<String>, SecretError> {
    let unavailable = |detail: String| SecretError::Unavailable {
        source: "secrets-broker",
        name: LIST_PSEUDO_NAME.to_string(),
        detail,
    };
    match status {
        200 => {
            let names = body
                .get("names")
                .and_then(|v| v.as_array())
                .ok_or_else(|| unavailable("200 response without a \"names\" array".to_string()))?;
            names
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| {
                        unavailable("\"names\" array holds a non-string entry".to_string())
                    })
                })
                .collect()
        }
        404 if scope.prefix().is_some() => Err(unavailable(
            "broker does not support scoped list".to_string(),
        )),
        status => Err(unavailable(format!(
            "secrets-broker answered HTTP {status}"
        ))),
    }
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

/// Per-invocation ceiling on a single fallback-tier subprocess
/// wait (`bws` / `op` through a [`CommandRunner`], and the
/// `--version` probe in program resolution). The broker legs bound
/// each exchange with [`BROKER_ATTEMPT_TIMEOUT`]; the fallback
/// tiers spawn children, and `Command::output()` waits forever --
/// a wedged `bws` or `op` child was the one remaining path by
/// which a secrets client could still hang under the never-hang
/// ruling (#14040 Residual B). Same discipline: one named
/// per-attempt ceiling, in one obvious place.
pub const SUBPROCESS_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// Ceiling on the `--version` probe [`program_runs`] runs against
/// a candidate program. A healthy candidate answers instantly, so
/// the probe gets a shorter bound than a real invocation: a wedged
/// candidate must fail the gate, not hang resolution.
pub const PROGRAM_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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
                // A cold secrets broker reports this stable error
                // while its first refresh is in flight. Waiting out
                // the generic restart budget here wedges clients
                // behind a healthy-but-not-yet-serving broker; allow
                // the resolver chain to try its next source now.
                if is_initial_refresh_unavailable(&err) {
                    return Err(err);
                }
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

fn is_initial_refresh_unavailable(error: &SecretError) -> bool {
    matches!(error, SecretError::Unavailable { source: "secrets-broker", detail, .. }
        if detail.contains("secrets not loaded yet"))
}

/// The ordered chain: broker protocol first, `bws` fallback (§1).
pub struct Resolver {
    sources: Vec<Box<dyn SecretSource>>,
}

impl Resolver {
    pub fn new(sources: Vec<Box<dyn SecretSource>>) -> Resolver {
        Resolver { sources }
    }

    /// The chain's leg names in order (`secrets-broker`, `bws`,
    /// `op`): the seam `--upstream` / `--no-cache` composition tests
    /// assert through, and operators debugging a resolution path
    /// can print.
    pub fn source_names(&self) -> Vec<&'static str> {
        self.sources.iter().map(|s| s.source_name()).collect()
    }

    /// The default chain for a resolved fleet config. On a host
    /// where the secrets-broker's own socket answers (the laptop,
    /// #6218), the direct source comes first: there is no session
    /// broker there. On the sandbox the probe selects TCP, no
    /// direct source is inserted, and the broker leg is the
    /// historical broker-protocol shape. Behind the broker leg come
    /// the `bws` fallback and, where a vault is named (env
    /// `FLEET_OP_VAULT` or `[secrets] op_vault`, #14151), the
    /// client-side op source.
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
        if let Some(op) = OpSource::from_config(config) {
            sources.push(Box::new(op));
        }
        Resolver::new(sources)
    }

    /// The chain for a store selection (#10358 D8, S3).
    ///
    /// * `Auto` is [`Resolver::default_chain`] exactly (D1).
    /// * `Bitwarden` is the broker legs and `bws`, with no `op` leg;
    ///   with a vault, the `bws` leg matches only that project. The
    ///   broker legs are unfiltered on `get` (D7: one full name is
    ///   already exact); the project filter applies to `list`.
    /// * `OnePassword` is `op` only, in the selection's vault, or in
    ///   `$FLEET_OP_VAULT` when the selection names no vault. With no
    ///   usable vault the chain is empty and every resolve fails as a
    ///   chain error rather than falling back to another store.
    pub fn chain_for(config: &fleet_config::Config, selection: &StoreSelection) -> Resolver {
        match selection.store {
            StoreKind::Auto => Self::default_chain(config),
            StoreKind::Bitwarden => {
                let mut sources: Vec<Box<dyn SecretSource>> = Vec::new();
                let mut broker_retry = RetryPolicy::default();
                if let Some(direct) = DirectBrokerSource::from_config(config) {
                    sources.push(Box::new(direct));
                    broker_retry = RetryPolicy::NONE;
                }
                sources.push(Box::new(
                    BrokerSource::new(config.broker_socket(), config.secrets_token_file())
                        .with_retry_policy(broker_retry),
                ));
                let bws = match selection.vault.as_deref() {
                    Some(project) => BwsSource::with_project(project.to_string()),
                    None => BwsSource::new(),
                };
                sources.push(Box::new(bws));
                Resolver::new(sources)
            }
            StoreKind::OnePassword => {
                let vault = selection
                    .vault
                    .clone()
                    .or_else(|| std::env::var(OpSource::VAULT_ENV).ok());
                let field = std::env::var(OpSource::FIELD_ENV).ok();
                let sources: Vec<Box<dyn SecretSource>> = match OpSource::from_values(
                    vault.as_deref(),
                    field.as_deref(),
                    // Self-loaded token (#14151), same as the default
                    // chain: an explicit 1Password selection must not
                    // need a shell wrapper either.
                    Box::new(OpTokenRunner::from_environment()),
                ) {
                    Some(op) => vec![Box::new(op)],
                    None => Vec::new(),
                };
                Resolver::new(sources)
            }
        }
    }

    /// The upstream chain (#13948 S5, `get --upstream`): the
    /// primary store and the 1Password mirror, with no broker leg.
    /// A broker cache miss is never final (read-through), but a
    /// broker cache HIT can be stale; `--upstream` skips the cache
    /// entirely and resolves from the stores that the brokers
    /// themselves refresh from. The `bws` leg is always present; the
    /// `op` leg joins under the same vault rule as the default
    /// chain. A vault-scoped `bitwarden` selection scopes the `bws`
    /// leg to that project, exactly as in [`Resolver::chain_for`].
    pub fn upstream_chain(config: &fleet_config::Config, selection: &StoreSelection) -> Resolver {
        let mut sources: Vec<Box<dyn SecretSource>> = Vec::new();
        let bws = match selection.bitwarden_project() {
            Some(project) => BwsSource::with_project(project.to_string()),
            None => BwsSource::new(),
        };
        sources.push(Box::new(bws));
        // The mirror leg under the default-chain rule: only where a
        // vault is named. A `bitwarden` selection excludes it (the
        // caller asked for one store); any other selection keeps it.
        let want_op = !matches!(selection.store, StoreKind::Bitwarden);
        if want_op {
            if let Some(op) = OpSource::from_config(config) {
                sources.push(Box::new(op));
            }
        }
        Resolver::new(sources)
    }

    /// The primary-store-only chain (#13948 S5, `get --no-cache`):
    /// straight to Bitwarden, bypassing every broker cache AND the
    /// 1Password mirror. The answer is the commit-point truth (§5.5:
    /// the primary store is what a put commits to first), never a
    /// cached or mirrored copy.
    pub fn primary_only(selection: &StoreSelection) -> Resolver {
        let bws = match selection.bitwarden_project() {
            Some(project) => BwsSource::with_project(project.to_string()),
            None => BwsSource::new(),
        };
        Resolver::new(vec![Box::new(bws)])
    }

    /// Resolve a caller's name inside a scope: the name is mapped to
    /// its full stored name once, before any source sees it, so every
    /// source in the chain receives the same full name (D2, D12).
    pub fn resolve_scoped(&self, scope: &Scope, name: &str) -> Result<SecretValue, SecretError> {
        self.resolve(&scope.to_full(name)?)
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
        self.request_with_method("GET", path, name)
    }

    /// The same exchange under an explicit HTTP method (#13948 S5):
    /// `POST /refresh` triggers a broker refresh cycle; the
    /// session-broker frame carries the method through to the
    /// secrets broker exactly as it carries the path.
    fn request_with_method(
        &self,
        method: &str,
        path: &str,
        name: &str,
    ) -> Result<Response, SecretError> {
        let request = Request {
            method: method.to_string(),
            path: path.to_string(),
            body: None,
            port: Some(fleet_config::PORT_SECRETS_BROKER),
            token_file: Some(self.token_file.to_string_lossy().into_owned()),
            trip: None,
            transport: None,
            cf_ok: None,
            // #14188: the caller's FLEET_CTX_* record rides the frame.
            ctx: fleet_config::ctx::Ctx::from_env().to_frame(),
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

    /// Trigger a broker refresh cycle now (#13948 S5, `fleet
    /// secrets refresh`): the broker runs §5.2/§5.1 out of band
    /// instead of at the next TTL tick. The broker answers 202 with
    /// the cycle running in the background; the caller polls
    /// [`BrokerSource::health`] for `refreshing == false` and reads
    /// the serving leg there. A 404 means the broker predates the
    /// refresh route; any other non-2xx is `Unavailable`.
    pub fn post_refresh(&self) -> Result<(), SecretError> {
        let response = self.request_with_method("POST", "/refresh", REFRESH_PSEUDO_NAME)?;
        match response.status {
            200 | 202 => Ok(()),
            404 => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: REFRESH_PSEUDO_NAME.to_string(),
                detail: "broker does not serve POST /refresh (predates #13948 S5)".to_string(),
            }),
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: REFRESH_PSEUDO_NAME.to_string(),
                detail: broker_error_detail(status, &response.body),
            }),
        }
    }

    /// The broker's liveness record (`GET /health`, #13948 S5): the
    /// same unauthenticated probe a heal uses, read here so the
    /// `refresh` verb can wait out a triggered cycle and report the
    /// leg that served it.
    pub fn health(&self) -> Result<BrokerHealth, SecretError> {
        let response = self.request("/health", HEALTH_PSEUDO_NAME)?;
        // /health answers 200 serving or 503 still loading — both
        // carry the record; anything else is a broker failure.
        match response.status {
            200 | 503 => BrokerHealth::parse(&response.body),
            404 => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: HEALTH_PSEUDO_NAME.to_string(),
                detail: "broker does not serve GET /health".to_string(),
            }),
            status => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: HEALTH_PSEUDO_NAME.to_string(),
                detail: broker_error_detail(status, &response.body),
            }),
        }
    }

    /// The broker's names listing (`GET /secrets` — the endpoint the
    /// step-6 verb's `list` renders). Names only: the broker's reply
    /// carries no values on this route, and neither does this API.
    /// Errors are [`SecretError::Unavailable`] under the pseudo-name
    /// `"names"` — a listing has no single secret to name.
    pub fn list_names(&self) -> Result<Vec<String>, SecretError> {
        self.list_names_in(&Scope::none())
    }

    /// The names listing for a scope (#10358 D6): a prefixed scope
    /// asks the broker for `GET /secrets?prefix=P`; [`Scope::none`]
    /// sends exactly today's `GET /secrets`. The names come back as
    /// the broker stored them; the caller applies [`Scope::display`].
    pub fn list_names_in(&self, scope: &Scope) -> Result<Vec<String>, SecretError> {
        self.list_names_scoped(scope, &StoreSelection::auto())
    }

    /// The names listing for a scope under a store selection
    /// (#10358 S3): a `bitwarden` vault adds `project=<id>` beside
    /// `prefix` (see [`list_path_scoped`]).
    pub fn list_names_scoped(
        &self,
        scope: &Scope,
        selection: &StoreSelection,
    ) -> Result<Vec<String>, SecretError> {
        let response = self.request(&list_path_scoped(scope, selection), LIST_PSEUDO_NAME)?;
        names_from_listing(response.status, &response.body, scope)
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
        // No socket file at all means no session broker on this box (a
        // fresh sandbox before init, or a host that never runs one), not
        // a broker mid-restart: a restarting broker leaves its socket
        // path behind (ECONNREFUSED, retried). Spending the retry budget
        // here only delays the bws fallback by two minutes per secret.
        if !self.broker_socket.exists() {
            return self.resolve_once(name);
        }
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
        self.request_raw("GET", path, name)
    }

    /// One raw HTTP exchange over the broker socket (#13948 S5):
    /// `GET` as before, plus `POST /refresh` (empty body) for the
    /// `refresh` verb. `POST` sends `Content-Length: 0` so the
    /// broker's body gate never waits on bytes that never come.
    fn request_raw(
        &self,
        method: &str,
        path: &str,
        name: &str,
    ) -> Result<(u16, serde_json::Value), SecretError> {
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
        // #14188: the caller's FLEET_CTX_* record as X-Fleet-Ctx-*
        // headers (empty when there is no context).
        let ctx_headers = fleet_config::ctx::Ctx::from_env().header_lines();
        let req = if method == "POST" {
            format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Agent-Token: {}\r\n{ctx_headers}Content-Length: 0\r\nConnection: close\r\n\r\n",
                token.trim()
            )
        } else {
            format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nX-Agent-Token: {}\r\n{ctx_headers}Connection: close\r\n\r\n",
                token.trim()
            )
        };
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

    /// Trigger a broker refresh cycle now over the direct socket
    /// (#13948 S5): the same `POST /refresh` the session-broker leg
    /// sends, without the framed hop. Status mapping matches
    /// [`BrokerSource::post_refresh`].
    pub fn post_refresh(&self) -> Result<(), SecretError> {
        let (status, body) = self.request_raw("POST", "/refresh", REFRESH_PSEUDO_NAME)?;
        match status {
            200 | 202 => Ok(()),
            404 => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: REFRESH_PSEUDO_NAME.to_string(),
                detail: "broker does not serve POST /refresh (predates #13948 S5)".to_string(),
            }),
            _ => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: REFRESH_PSEUDO_NAME.to_string(),
                detail: broker_error_detail(status, &body),
            }),
        }
    }

    /// The broker's liveness record over the direct socket (#13948
    /// S5): same parse as [`BrokerSource::health`].
    pub fn health(&self) -> Result<BrokerHealth, SecretError> {
        let (status, body) = self.get("/health", HEALTH_PSEUDO_NAME)?;
        match status {
            200 | 503 => BrokerHealth::parse(&body),
            404 => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: HEALTH_PSEUDO_NAME.to_string(),
                detail: "broker does not serve GET /health".to_string(),
            }),
            _ => Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: HEALTH_PSEUDO_NAME.to_string(),
                detail: broker_error_detail(status, &body),
            }),
        }
    }

    /// The broker's names listing over the direct socket -- the
    /// same `GET /secrets` route [`BrokerSource::list_names`]
    /// reads through the session broker. Names only.
    pub fn list_names(&self) -> Result<Vec<String>, SecretError> {
        self.list_names_in(&Scope::none())
    }

    /// The scoped names listing over the direct socket, as
    /// [`BrokerSource::list_names_in`].
    pub fn list_names_in(&self, scope: &Scope) -> Result<Vec<String>, SecretError> {
        self.list_names_scoped(scope, &StoreSelection::auto())
    }

    /// The scoped names listing under a store selection, as
    /// [`BrokerSource::list_names_scoped`].
    pub fn list_names_scoped(
        &self,
        scope: &Scope,
        selection: &StoreSelection,
    ) -> Result<Vec<String>, SecretError> {
        let (status, body) = self.get(&list_path_scoped(scope, selection), LIST_PSEUDO_NAME)?;
        names_from_listing(status, &body, scope)
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

/// Run `cmd` to completion collecting its output -- a bounded
/// `Command::output()`. The unbounded call waits forever: a child
/// that never exits (a `bws` wedged on a dead network, an `op`
/// stuck on a prompt nobody can see) would block the fallback tier
/// -- and with it the whole chain -- indefinitely. Here the child
/// is polled and killed once `timeout` elapses, the same
/// spawn/poll/kill shape the secrets broker's own `bws` runner
/// uses (fleet-queue's `BwsRunner`). Stdout and stderr are drained
/// on threads so a chatty child cannot fill a pipe and deadlock
/// the wait. Error shapes match the runners' existing ones: spawn
/// failures name the io error kind; a timeout names the bound.
fn run_command_bounded(
    cmd: &mut Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    use std::io::Read;
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn failed: {}", e.kind()))?;
    let mut stdout_pipe = child.stdout.take();
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let mut stderr_pipe = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait failed: {}", e.kind()));
            }
        }
    };
    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub struct ProcessRunner {
    timeout: Duration,
}

impl ProcessRunner {
    /// A runner with the standard per-invocation ceiling
    /// ([`SUBPROCESS_ATTEMPT_TIMEOUT`]).
    pub fn new() -> ProcessRunner {
        ProcessRunner {
            timeout: SUBPROCESS_ATTEMPT_TIMEOUT,
        }
    }

    /// A runner with a caller-chosen ceiling -- the shape tests
    /// use to prove a never-exiting child fails within the bound
    /// without waiting out the production ceiling.
    pub fn with_timeout(timeout: Duration) -> ProcessRunner {
        ProcessRunner { timeout }
    }
}

impl Default for ProcessRunner {
    fn default() -> ProcessRunner {
        ProcessRunner::new()
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        let out = run_command_bounded(&mut cmd, self.timeout)?;
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

/// X_OK is not sufficient: a Nix binary whose ELF interpreter is
/// absent passes `access(2)` and then fails every exec with
/// ENOENT -- the #12498 defect the shell tooling fixed by
/// resolving through test execution. A candidate must also
/// actually run: probe it with `--version` (the harmless
/// invocation the shell resolver uses), stdio discarded; a spawn
/// error or a non-zero exit means the candidate does not run on
/// this host and must not shadow a working one later in the
/// order.
fn program_runs(p: &Path) -> bool {
    program_runs_within(p, PROGRAM_PROBE_TIMEOUT)
}

/// [`program_runs`] with a caller-chosen bound, split out for
/// tests: the probe is a subprocess wait like any other, and an
/// unbounded one would hang program resolution itself on a wedged
/// candidate (#14040 Residual B).
fn program_runs_within(p: &Path, timeout: Duration) -> bool {
    let mut cmd = std::process::Command::new(p);
    cmd.arg("--version");
    run_command_bounded(&mut cmd, timeout)
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// The full candidate gate for [`resolve_bws_program_from`]:
/// an executable file that actually runs.
fn is_runnable_program(p: &Path) -> bool {
    is_executable_file(p) && program_runs(p)
}

/// The core of [`resolve_bws_program`], split out for tests:
/// a candidate wins only when it is an executable file that
/// actually runs, in override -> PATH -> well-known-locations
/// order.
fn resolve_bws_program_from(
    env_override: Option<&str>,
    path_var: Option<&std::ffi::OsStr>,
    home: &Path,
) -> PathBuf {
    if let Some(p) = env_override {
        let p = p.trim();
        if !p.is_empty() && is_runnable_program(Path::new(p)) {
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
            if is_runnable_program(&cand) {
                return cand;
            }
        }
    }
    for cand in [
        home.join("tools/bin/bws"),
        home.join(".nix-profile/bin/bws"),
        PathBuf::from("/run/current-system/sw/bin/bws"),
    ] {
        if is_runnable_program(&cand) {
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
    timeout: Duration,
}

impl BwsTokenRunner {
    pub fn from_environment() -> BwsTokenRunner {
        BwsTokenRunner {
            token: bws_access_token(),
            timeout: SUBPROCESS_ATTEMPT_TIMEOUT,
        }
    }

    pub fn with_token(token: Option<String>) -> BwsTokenRunner {
        BwsTokenRunner {
            token,
            timeout: SUBPROCESS_ATTEMPT_TIMEOUT,
        }
    }

    /// The same runner with a caller-chosen subprocess ceiling --
    /// the shape tests use to prove a never-exiting child fails
    /// within the bound without waiting out the production one.
    pub fn with_timeout(mut self, timeout: Duration) -> BwsTokenRunner {
        self.timeout = timeout;
        self
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
        let out = run_command_bounded(&mut cmd, self.timeout)?;
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
    /// The Bitwarden project reads are scoped to (#10358 D8, S3):
    /// when set, only listing entries whose `projectId` equals it
    /// can match. `None` is today's unfiltered behaviour.
    project_id: Option<String>,
}

impl BwsSource {
    pub fn new() -> BwsSource {
        BwsSource {
            runner: Box::new(BwsTokenRunner::from_environment()),
            project_id: None,
        }
    }

    /// A source scoped to one Bitwarden project (the `bitwarden`
    /// store selection's vault).
    pub fn with_project(project_id: String) -> BwsSource {
        BwsSource {
            runner: Box::new(BwsTokenRunner::from_environment()),
            project_id: Some(project_id),
        }
    }

    pub fn with_runner(runner: Box<dyn CommandRunner>) -> BwsSource {
        BwsSource {
            runner,
            project_id: None,
        }
    }

    /// A source with an explicit runner scoped to one project —
    /// the fake-runner test seam for [`BwsSource::with_project`].
    pub fn with_runner_and_project(
        runner: Box<dyn CommandRunner>,
        project_id: String,
    ) -> BwsSource {
        BwsSource {
            runner,
            project_id: Some(project_id),
        }
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
            // #10358 D8: a project-scoped source sees only its own
            // project's entries; an entry in another project (or
            // with no project recorded) is not a match.
            if let Some(project) = self.project_id.as_deref() {
                if item.get("projectId").and_then(|p| p.as_str()) != Some(project) {
                    continue;
                }
            }
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

/// Finds the id of the secret keyed `name` in a `bws secret list`
/// reply, with [`BwsSource`]'s discipline: a listing that is not a
/// JSON array is a source failure, and a malformed id on the matching
/// entry is malformed, never absent. Entries for other names are not
/// inspected.
fn bws_listed_id(list_raw: &str, name: &str) -> Result<Option<String>, SecretError> {
    let unavailable = |detail: &str| SecretError::Unavailable {
        source: "bws",
        name: name.to_string(),
        detail: detail.to_string(),
    };
    let list: serde_json::Value = serde_json::from_str(list_raw)
        .map_err(|_| unavailable("bws secret list: output is not JSON"))?;
    let items = list
        .as_array()
        .ok_or_else(|| unavailable("bws secret list: output is not a JSON array"))?;
    for item in items {
        if item.get("key").and_then(|k| k.as_str()) == Some(name) {
            let raw_id = item.get("id").and_then(|i| i.as_str()).ok_or_else(|| {
                unavailable(
                    "bws secret list: entry for the requested key has a missing or non-string id",
                )
            })?;
            return Ok(Some(raw_id.to_string()));
        }
    }
    Ok(None)
}

/// Which project a secret write lands in: the environment value
/// wins, the config value is the fallback, a blank value counts as
/// absent (the #6405 misspelling class).
fn bws_project_id_from(env_value: Option<&str>, config_value: Option<&str>) -> Option<String> {
    let clean = |v: Option<&str>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    clean(env_value).or_else(|| clean(config_value))
}

/// The write counterpart of [`BwsSource`] (#14370): stores a value
/// under a name in one Bitwarden Secrets Manager project, through the
/// same `bws` CLI and runner. This is the vault write path
/// `fleet bridge exchange` uses to put a freshly minted per-agent
/// credential under its name (`bridge.<agent>-fleet-token`,
/// `bridge.<agent>-token`).
///
/// bws has no stdin or file input for values, so the value passes
/// through the `bws` child's argv — transiently, the exposure the
/// house backup script already accepts (pkgs/agent-backup) — and
/// nowhere else: never in this process's argv, a file, output or an
/// error (runner errors carry exit status only, and the `--`
/// separator before the positionals keeps bws's own parser from
/// echoing the value on a usage error). Storing over an existing
/// name replaces it (delete, then create), so re-enrolment rotates
/// the credential. The store verifies its work by reading the name
/// back and comparing in memory; a mismatch is a loud failure.
pub struct BwsStore {
    runner: Box<dyn CommandRunner>,
    project_id: String,
}

impl BwsStore {
    /// The environment variable that names the write project when the
    /// config does not (the [`OpSource`] env-first pattern).
    pub const PROJECT_ID_ENV: &'static str = "FLEET_BWS_PROJECT_ID";

    pub fn new(project_id: String) -> BwsStore {
        BwsStore {
            runner: Box::new(BwsTokenRunner::from_environment()),
            project_id,
        }
    }

    pub fn with_runner(runner: Box<dyn CommandRunner>, project_id: String) -> BwsStore {
        BwsStore { runner, project_id }
    }

    /// The store for a config: the project id from
    /// $FLEET_BWS_PROJECT_ID, else `[secrets] bws_project_id`. None
    /// when neither names one — reads are unaffected, only writes
    /// are impossible.
    pub fn from_config(config: &fleet_config::Config) -> Option<BwsStore> {
        let project_id = bws_project_id_from(
            std::env::var(Self::PROJECT_ID_ENV).ok().as_deref(),
            config.secrets.bws_project_id.as_deref(),
        )?;
        Some(BwsStore::new(project_id))
    }

    /// The store for a config under a store selection (#10358 D8,
    /// S3): a `bitwarden` selection's vault is the write project,
    /// overriding both `$FLEET_BWS_PROJECT_ID` and
    /// `[secrets] bws_project_id`. Any other selection keeps
    /// [`BwsStore::from_config`]'s behaviour exactly.
    pub fn from_config_for(
        config: &fleet_config::Config,
        selection: &StoreSelection,
    ) -> Option<BwsStore> {
        if let Some(project) = selection.bitwarden_project() {
            return Some(BwsStore::new(project.to_string()));
        }
        Self::from_config(config)
    }

    /// Store `value` under `name`, replacing any existing secret of
    /// that name in the project. Errors are value-free.
    pub fn store(&self, name: &str, value: &str) -> Result<(), SecretError> {
        validate_name(name)?;
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "bws",
            name: name.to_string(),
            detail,
        };
        if value.is_empty() {
            return Err(unavailable("refusing to store an empty value".to_string()));
        }
        let list_raw = self
            .runner
            .run("bws", &["secret", "list", "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret list: {d}")))?;
        if let Some(old_id) = bws_listed_id(&list_raw, name)? {
            self.runner
                .run("bws", &["secret", "delete", &old_id, "-o", "json"])
                .map_err(|d| unavailable(format!("bws secret delete: {d}")))?;
        }
        // Options first, then `--`, then the positionals: a value
        // starting with '-' must never be parsed as a flag (bws's
        // parser echoes the offending value in its error otherwise).
        self.runner
            .run(
                "bws",
                &[
                    "secret",
                    "create",
                    "-o",
                    "json",
                    "--",
                    name,
                    value,
                    &self.project_id,
                ],
            )
            .map_err(|d| unavailable(format!("bws secret create: {d}")))?;
        // Verify by reading the name back; the comparison happens in
        // memory and the value is never printed either way.
        let list_raw = self
            .runner
            .run("bws", &["secret", "list", "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret list: {d}")))?;
        let id = bws_listed_id(&list_raw, name)?.ok_or_else(|| {
            unavailable(
                "bws secret create: the name is absent from the listing afterwards".to_string(),
            )
        })?;
        let got_raw = self
            .runner
            .run("bws", &["secret", "get", &id, "-o", "json"])
            .map_err(|d| unavailable(format!("bws secret get: {d}")))?;
        let got: serde_json::Value = serde_json::from_str(&got_raw)
            .map_err(|_| unavailable("bws secret get: output is not JSON".to_string()))?;
        let stored = got
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| unavailable("bws secret get: no string \"value\" field".to_string()))?;
        if stored != value {
            return Err(unavailable(
                "bws secret create: the value read back does not match what was stored".to_string(),
            ));
        }
        Ok(())
    }
}

/// The 1Password service-account token for an `op` child,
/// resolved from an env value and a home dir: the env value when
/// non-empty, else the on-disk token files (the seat's
/// `.config/op/service-account-token`, then
/// `.config/1password/service-account-token`). This is the `op`
/// analogue of the broker-mirroring [`bws_token_from`] lookup, so
/// the op leg works for non-interactive callers without a shell
/// wrapper pre-loading the variable (#14151). The token is
/// returned trimmed; empty files and an empty env value count as
/// absent. Fleet never fetches this token from a vault itself: a
/// seat that keeps it in Bitwarden loads it into one of these
/// places (env or file) once, outside this crate.
fn op_token_from(env_value: Option<&str>, home: &Path) -> Option<String> {
    if let Some(t) = env_value {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    for rel in [
        ".config/op/service-account-token",
        ".config/1password/service-account-token",
    ] {
        if let Ok(t) = std::fs::read_to_string(home.join(rel)) {
            if !t.trim().is_empty() {
                return Some(t.trim().to_string());
            }
        }
    }
    None
}

/// The ambient lookup: this process's env + the fleet home dir.
fn op_service_account_token() -> Option<String> {
    op_token_from(
        std::env::var("OP_SERVICE_ACCOUNT_TOKEN").ok().as_deref(),
        &fleet_config::Config::home_dir(),
    )
}

/// A [`CommandRunner`] that hands the child the resolved 1Password
/// service-account token explicitly, mirroring [`BwsTokenRunner`]
/// (including its bounded subprocess wait, #14040). Before this
/// runner existed, the op leg inherited the ambient environment
/// only, so non-interactive callers without a shell wrapper
/// pre-loading `OP_SERVICE_ACCOUNT_TOKEN` failed every op read.
/// The runner always controls the variable: set from the resolved
/// token, or removed when no token resolves.
pub struct OpTokenRunner {
    token: Option<String>,
    timeout: Duration,
}

impl OpTokenRunner {
    pub fn from_environment() -> OpTokenRunner {
        OpTokenRunner {
            token: op_service_account_token(),
            timeout: SUBPROCESS_ATTEMPT_TIMEOUT,
        }
    }

    pub fn with_token(token: Option<String>) -> OpTokenRunner {
        OpTokenRunner {
            token,
            timeout: SUBPROCESS_ATTEMPT_TIMEOUT,
        }
    }

    /// The same runner with a caller-chosen subprocess ceiling --
    /// the shape tests use to prove a never-exiting child fails
    /// within the bound without waiting out the production one.
    pub fn with_timeout(mut self, timeout: Duration) -> OpTokenRunner {
        self.timeout = timeout;
        self
    }
}

impl CommandRunner for OpTokenRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        match &self.token {
            Some(token) => {
                cmd.env("OP_SERVICE_ACCOUNT_TOKEN", token);
            }
            None => {
                cmd.env_remove("OP_SERVICE_ACCOUNT_TOKEN");
            }
        }
        let out = run_command_bounded(&mut cmd, self.timeout)?;
        if !out.status.success() {
            return Err(format!("exit status {}", out.status));
        }
        String::from_utf8(out.stdout).map_err(|_| "output not utf8".to_string())
    }
}

/// Source 3, opt-in: the 1Password CLI. One read per name:
/// `op read --no-newline op://<vault>/<name>/<field>`, so the item
/// title in the vault is the fleet secret name. It joins the default
/// chain only where a vault is named, so a seat without 1Password
/// keeps the historical chain exactly.
///
/// Every `op` failure is `Unavailable`, never `NotFound`: the CLI
/// exits non-zero the same way for a missing item, a locked vault
/// and a signed-out session, and this crate does not read a child's
/// stderr (it could quote vault material) — so absence cannot be
/// told apart from a broken read here.
pub struct OpSource {
    vault: String,
    field: String,
    runner: Box<dyn CommandRunner>,
}

impl OpSource {
    /// Names the vault; setting it is what turns the source on.
    pub const VAULT_ENV: &'static str = "FLEET_OP_VAULT";
    /// Names the item field that holds the value.
    pub const FIELD_ENV: &'static str = "FLEET_OP_FIELD";
    /// 1Password's own default field for an API-credential item.
    pub const DEFAULT_FIELD: &'static str = "credential";

    /// The source for a vault and an optional field, or None where
    /// no usable vault is named. A vault or field that could not be
    /// one path segment of a secret reference turns the source off
    /// rather than reading from somewhere unintended.
    pub fn from_values(
        vault: Option<&str>,
        field: Option<&str>,
        runner: Box<dyn CommandRunner>,
    ) -> Option<OpSource> {
        let vault = vault.map(str::trim).filter(|v| Self::is_segment(v))?;
        let field = match field.map(str::trim).filter(|f| !f.is_empty()) {
            Some(f) if Self::is_segment(f) => f,
            Some(_) => return None,
            None => Self::DEFAULT_FIELD,
        };
        Some(OpSource {
            vault: vault.to_string(),
            field: field.to_string(),
            runner,
        })
    }

    pub fn from_environment() -> Option<OpSource> {
        let vault = std::env::var(Self::VAULT_ENV).ok();
        let field = std::env::var(Self::FIELD_ENV).ok();
        Self::from_values(
            vault.as_deref(),
            field.as_deref(),
            Box::new(OpTokenRunner::from_environment()),
        )
    }

    /// The source for a resolved fleet config (#14151): `[secrets]
    /// op_vault` / `op_field` name the vault, with `FLEET_OP_VAULT` /
    /// `FLEET_OP_FIELD` beating the file when both are set. The
    /// credential is the same self-loaded service-account token as
    /// [`OpSource::from_environment`].
    pub fn from_config(config: &fleet_config::Config) -> Option<OpSource> {
        let env_vault = std::env::var(Self::VAULT_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let env_field = std::env::var(Self::FIELD_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let vault = env_vault.or_else(|| config.secrets_op_vault());
        let field = env_field.or_else(|| config.secrets_op_field());
        Self::from_values(
            vault.as_deref(),
            field.as_deref(),
            Box::new(OpTokenRunner::from_environment()),
        )
    }

    fn is_segment(s: &str) -> bool {
        !s.is_empty()
            && !s
                .chars()
                .any(|c| c == '/' || c == '?' || c == '#' || c.is_control())
    }

    /// The secret reference `op read` is given for a name.
    pub fn reference(&self, name: &str) -> String {
        format!("op://{}/{}/{}", self.vault, name, self.field)
    }

    /// The vault this source reads from.
    pub fn vault(&self) -> &str {
        &self.vault
    }

    /// The names in the vault (#10358 D8, S3): one
    /// `op item list --vault <v> --format json` call, titles only —
    /// no item values are ever requested. A reply that is not a JSON
    /// array of objects with string `title`s is a source failure,
    /// never a confident empty listing.
    pub fn list_names(&self) -> Result<Vec<String>, SecretError> {
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "op",
            name: LIST_PSEUDO_NAME.to_string(),
            detail,
        };
        let raw = self
            .runner
            .run(
                "op",
                &["item", "list", "--vault", &self.vault, "--format", "json"],
            )
            .map_err(|d| unavailable(format!("op item list: {d}")))?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| unavailable("op item list: output is not JSON".to_string()))?;
        let items = parsed
            .as_array()
            .ok_or_else(|| unavailable("op item list: output is not a JSON array".to_string()))?;
        let mut names = Vec::with_capacity(items.len());
        for item in items {
            let title = item.get("title").and_then(|t| t.as_str()).ok_or_else(|| {
                unavailable("op item list: an item has no string \"title\"".to_string())
            })?;
            names.push(title.to_string());
        }
        Ok(names)
    }
}

impl SecretSource for OpSource {
    fn source_name(&self) -> &'static str {
        "op"
    }

    fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
        validate_name(name)?;
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "op",
            name: name.to_string(),
            detail,
        };
        let reference = self.reference(name);
        let value = self
            .runner
            .run("op", &["read", "--no-newline", &reference])
            .map_err(|d| unavailable(format!("op read: {d}")))?;
        if value.is_empty() {
            return Err(unavailable("op read: empty value".to_string()));
        }
        Ok(SecretValue(value))
    }
}

/// The 1Password capability spike (#13948 S5, design §3 + §7-S5):
/// what this seat can do with `op`, established WITHOUT reading
/// any item value. `op item list` returns metadata only (titles,
/// ids, timestamps) and `op account list` names configured
/// accounts; neither call touches a vault item's fields, so the
/// probe never sees secret material.
///
/// Live result, laptop seat, 2026-11-28: the `op` CLI is present
/// (2.41.1) but reports "No accounts configured for use with
/// 1Password CLI" and no `OP_SERVICE_ACCOUNT_TOKEN` is set in a
/// plain shell, so `mirror_write_ready` is false there — exactly
/// the design's documented unverified premise. The mirror write
/// path is therefore built dark (never attempted while the probe
/// fails) with the failure recorded as `propagation pending` for
/// the S6 reconcile, and open design question 4 (write identity:
/// service account vs desktop-integration session) stays with
/// Archer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpCapability {
    /// An `op` binary resolves and runs.
    pub cli_present: bool,
    /// `op account list` succeeds: at least one account (or a
    /// service-account token the CLI accepts) is configured.
    pub account_configured: bool,
    /// `OP_SERVICE_ACCOUNT_TOKEN` names a non-empty token in this
    /// environment (self-loaded by [`OpTokenRunner`]).
    pub service_account_token: bool,
    /// A mirror vault is named (`FLEET_OP_VAULT` / `[secrets]
    /// vault` / `[secrets] op_vault`), so a write would know where
    /// to go.
    pub vault_named: bool,
}

impl OpCapability {
    /// Whether a mirror write may be attempted: CLI present, an
    /// account configured, and a vault named. The service-account
    /// token is informational (an account session can also authorize
    /// the write), so it never gates. Anything less records
    /// `propagation pending` instead of failing the put (§5.5).
    pub fn mirror_write_ready(&self) -> bool {
        self.cli_present && self.account_configured && self.vault_named
    }

    /// Build a capability from four metadata-only observations, so
    /// the readiness rule is pinned by unit tests without spawning
    /// a child.
    pub fn from_observations(
        cli_present: bool,
        account_configured: bool,
        service_account_token: bool,
        vault_named: bool,
    ) -> OpCapability {
        OpCapability {
            cli_present,
            account_configured,
            service_account_token,
            vault_named,
        }
    }
}

/// Probe this seat's 1Password capability: one `op --version`
/// (presence), one `op account list --format json` (metadata only,
/// never an item value), plus the token/vault environment. The
/// runner is the same [`OpTokenRunner`] discipline reads use, so
/// the probe observes exactly the credential a read would carry.
pub fn probe_op_capability() -> OpCapability {
    let runner = OpTokenRunner::from_environment();
    let cli_present = runner.run("op", &["--version"]).is_ok();
    let account_configured = cli_present
        && runner
            .run("op", &["account", "list", "--format", "json"])
            .is_ok();
    let service_account_token = op_service_account_token()
        .is_some_and(|t| !t.trim().is_empty());
    let vault_named = std::env::var(OpSource::VAULT_ENV)
        .ok()
        .is_some_and(|v| !v.trim().is_empty());
    OpCapability::from_observations(
        cli_present,
        account_configured,
        service_account_token,
        vault_named,
    )
}

/// The 1Password mirror-write end of a `put` dual-write (#13948 S5,
/// §5.5): the primary commit goes to Bitwarden first and the mirror
/// follows, never the reverse, so a mirror failure degrades to
/// `propagation pending` for the S6 reconcile instead of failing
/// the put.
///
/// UNVERIFIED SPIKE (open design question 4, with Archer): no seat
/// with a configured account was available — the laptop seat that
/// ran the S5 spike has the CLI but no account — so the exact
/// create/edit argv below has never run against a live vault. The
/// verb ships dark ([secrets] `put_enabled` /
/// `FLEET_SECRETS_PUT_ENABLED`) until the S6 reconcile is live; by
/// then this path must have a live-spike receipt or it stays off.
/// Unit tests pin the argv shape through the fake-runner seam only.
///
/// Secret handling: the value travels as one `field=value` argv
/// entry — visible in the process list while the child runs. The
/// primary path never takes values on argv; this mirror path
/// accepts the narrower exposure because `op` offers no
/// stdin-value form for item fields, and says so here so S6 can
/// revisit it (write identity + argv exposure, open question 4).
pub struct OpStore {
    runner: Box<dyn CommandRunner>,
    vault: String,
    field: String,
}

impl OpStore {
    /// The same vault/field rule reads use: `FLEET_OP_VAULT` /
    /// `FLEET_OP_FIELD` beat `[secrets] op_vault` / `op_field`.
    /// None names no vault — mirror writes are impossible, which
    /// the caller reports as `propagation pending`, never an error.
    pub fn from_config(config: &fleet_config::Config) -> Option<OpStore> {
        let vault = OpSource::from_config(config)?.vault().to_string();
        let field = config
            .secrets_op_field()
            .unwrap_or_else(|| OpSource::DEFAULT_FIELD.to_string());
        Some(OpStore {
            runner: Box::new(OpTokenRunner::from_environment()),
            vault,
            field,
        })
    }

    /// The fake-runner seam unit tests pin the argv shape through.
    pub fn with_runner(runner: Box<dyn CommandRunner>, vault: String, field: String) -> OpStore {
        OpStore {
            runner,
            vault,
            field,
        }
    }

    /// Whether this store names the item `name` (titles only —
    /// metadata, never values — the same discipline as
    /// [`OpSource::list_names`]).
    fn item_exists(&self, name: &str) -> Result<bool, SecretError> {
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "op-store",
            name: name.to_string(),
            detail,
        };
        let raw = self
            .runner
            .run("op", &["item", "list", "--vault", &self.vault, "--format", "json"])
            .map_err(|d| unavailable(format!("op item list: {d}")))?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| unavailable("op item list: output is not JSON".to_string()))?;
        let items = parsed
            .as_array()
            .ok_or_else(|| unavailable("op item list: output is not a JSON array".to_string()))?;
        for item in items {
            let title = item.get("title").and_then(|t| t.as_str()).ok_or_else(|| {
                unavailable("op item list: an item has no string \"title\"".to_string())
            })?;
            if title == name {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Store `value` under item `name`: edit the field on the
    /// existing item, or create a `password`-category item carrying
    /// it. The name is validated exactly as reads validate it; the
    /// value is stored exactly as given.
    pub fn store(&self, name: &str, value: &str) -> Result<(), SecretError> {
        validate_name(name)?;
        let unavailable = |detail: String| SecretError::Unavailable {
            source: "op-store",
            name: name.to_string(),
            detail,
        };
        let assignment = format!("{}={}", self.field, value);
        let (op, args): (&str, Vec<&str>) = if self.item_exists(name)? {
            (
                "edit",
                vec!["item", "edit", "--vault", &self.vault, name, &assignment],
            )
        } else {
            (
                "create",
                vec![
                    "item",
                    "create",
                    "--category",
                    "password",
                    "--vault",
                    &self.vault,
                    "--title",
                    name,
                    &assignment,
                ],
            )
        };
        self.runner
            .run("op", &args)
            .map(|_| ())
            .map_err(|d| unavailable(format!("op item {op}: {d}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Scope (#10358 S1, design record §6) ----

    /// The record's nine-row matrix: (caller name, prefix, stored name
    /// or None when refused).
    const SCOPE_MATRIX: [(&str, Option<&str>, Option<&str>); 9] = [
        ("alpha", None, Some("alpha")),
        ("42.alpha", None, Some("42.alpha")),
        ("alpha", Some("42"), Some("42.alpha")),
        ("42.alpha", Some("42"), Some("42.alpha")),
        ("42.x", Some("42"), Some("42.x")),
        ("42.42.x", Some("42"), Some("42.42.x")),
        ("42", Some("42"), Some("42.42")),
        ("42.", Some("42"), None),
        ("420.x", Some("42"), Some("42.420.x")),
    ];

    fn scope_for(prefix: Option<&str>) -> Scope {
        match prefix {
            Some(p) => Scope::prefixed(p).unwrap(),
            None => Scope::none(),
        }
    }

    #[test]
    fn scope_maps_the_nine_row_matrix() {
        for (row, (name, prefix, stored)) in SCOPE_MATRIX.iter().enumerate() {
            let got = scope_for(*prefix).to_full(name);
            match stored {
                Some(want) => assert_eq!(
                    got.as_deref().ok(),
                    Some(*want),
                    "row {}: {name:?} under {prefix:?}",
                    row + 1
                ),
                None => assert!(
                    matches!(got, Err(SecretError::InvalidName(_))),
                    "row {}: {name:?} under {prefix:?} must be refused, got {got:?}",
                    row + 1
                ),
            }
        }
    }

    #[test]
    fn scope_none_is_identity_on_every_matrix_row() {
        let none = Scope::none();
        assert_eq!(none.prefix(), None);
        for (name, _, _) in SCOPE_MATRIX {
            assert_eq!(none.to_full(name).unwrap(), name);
            assert_eq!(none.display(name).as_deref(), Some(name));
        }
        assert_eq!(Scope::default(), Scope::none());
    }

    #[test]
    fn scope_refuses_names_validate_name_refuses() {
        let scope = Scope::prefixed("42").unwrap();
        for bad in ["", ".", "..", "a/b", "a b", "a%2f", "a?b", "a#b", "a\nb"] {
            assert!(
                matches!(scope.to_full(bad), Err(SecretError::InvalidName(_))),
                "{bad:?} must stay refused under a prefix"
            );
        }
    }

    #[test]
    fn scope_prefix_must_be_a_name_without_a_trailing_separator() {
        for bad in ["", ".", "..", "42.", "a/b", "a b"] {
            assert!(Scope::prefixed(bad).is_err(), "prefix {bad:?}");
        }
        let dotted = Scope::prefixed("team.42").unwrap();
        assert_eq!(dotted.to_full("x").unwrap(), "team.42.x");
        assert_eq!(dotted.to_full("team.42.x").unwrap(), "team.42.x");
        assert_eq!(dotted.to_full("team.x").unwrap(), "team.42.team.x");
    }

    #[test]
    fn scope_sources_prefer_env_and_treat_empty_as_unset() {
        let scope = |e, c| Scope::from_sources(e, c).unwrap();
        assert_eq!(scope(None, None), Scope::none());
        assert_eq!(scope(Some(""), Some("")), Scope::none());
        assert_eq!(scope(Some("  "), None), Scope::none());
        assert_eq!(scope(Some("env"), Some("cfg")).prefix(), Some("env"));
        assert_eq!(scope(Some(""), Some("cfg")).prefix(), Some("cfg"));
        assert_eq!(scope(None, Some("cfg")).prefix(), Some("cfg"));
        assert!(Scope::from_sources(Some("42."), None).is_err());
    }

    #[test]
    fn scope_display_filters_and_strips_for_list() {
        let scope = Scope::prefixed("42").unwrap();
        let stored = ["alpha", "42.alpha", "42.42.x", "420.y", "43.z", "42."];
        let shown: Vec<String> = stored.iter().filter_map(|s| scope.display(s)).collect();
        assert_eq!(shown, ["alpha", "42.42.x"]);
    }

    #[test]
    fn a_dot_segment_rest_is_shown_in_full() {
        // `42..` and `42...` are valid stored names, but `.` and `..`
        // are not valid names on their own, so stripping `42.` would
        // show a name that cannot be typed back.
        let scope = Scope::prefixed("42").unwrap();
        for stored in ["42..", "42..."] {
            let shown = scope.display(stored).unwrap();
            assert_eq!(shown, stored);
            assert_eq!(scope.to_full(&shown).unwrap(), stored);
        }
        assert_eq!(scope.display("42....").as_deref(), Some("..."));
    }

    #[test]
    fn scope_display_round_trips_through_to_full() {
        // Property over a deterministic spread of stored names under
        // the prefix: every displayed name maps back to its stored name.
        let parts = [
            "a", "42", "4", "420", "x.y", "42.", ".", "-", "42.42", "b.42",
        ];
        for prefix in ["42", "team.42", "x"] {
            let scope = Scope::prefixed(prefix).unwrap();
            for first in parts {
                for second in parts {
                    let stored = format!("{prefix}.{first}{second}");
                    if validate_name(&stored).is_err() {
                        continue;
                    }
                    let shown = scope
                        .display(&stored)
                        .unwrap_or_else(|| panic!("{stored:?} lies under {prefix:?}"));
                    assert_eq!(
                        scope.to_full(&shown).unwrap(),
                        stored,
                        "{stored:?} shown as {shown:?} under {prefix:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn list_path_sends_a_query_only_when_scoped() {
        assert_eq!(list_path(&Scope::none()), "/secrets");
        assert_eq!(
            list_path(&Scope::prefixed("42").unwrap()),
            "/secrets?prefix=42"
        );
        assert_eq!(
            list_path(&Scope::prefixed("team.4&2=+").unwrap()),
            "/secrets?prefix=team.4%262%3D%2B"
        );
    }

    #[test]
    fn a_scoped_listing_the_broker_does_not_know_is_unresolved() {
        let scoped = Scope::prefixed("42").unwrap();
        let err = names_from_listing(404, &serde_json::Value::Null, &scoped).unwrap_err();
        assert!(
            err.to_string()
                .contains("broker does not support scoped list"),
            "{err}"
        );
        // Unscoped, a 404 keeps the generic status detail.
        let err = names_from_listing(404, &serde_json::Value::Null, &Scope::none()).unwrap_err();
        assert!(err.to_string().contains("HTTP 404"), "{err}");
        let names =
            names_from_listing(200, &serde_json::json!({"names": ["42.a"]}), &scoped).unwrap();
        assert_eq!(names, ["42.a"]);
    }

    // ---- StoreSelection (#10358 S3, design record D8/D9) ----

    fn selection(
        flag_store: Option<&str>,
        flag_vault: Option<&str>,
        env_store: Option<&str>,
        env_vault: Option<&str>,
        config_store: Option<&str>,
        config_vault: Option<&str>,
    ) -> Result<StoreSelection, SecretError> {
        StoreSelection::from_sources(
            flag_store,
            flag_vault,
            env_store,
            env_vault,
            config_store,
            config_vault,
        )
    }

    #[test]
    fn selection_precedence_is_flag_then_env_then_config() {
        // Oracle: design record D9's table, read row by row.
        let s = selection(
            Some("1password"),
            Some("flag-vault"),
            Some("bitwarden"),
            Some("env-vault"),
            Some("bitwarden"),
            Some("config-vault"),
        )
        .unwrap();
        assert_eq!(s.store, StoreKind::OnePassword);
        assert_eq!(s.vault.as_deref(), Some("flag-vault"));
        let s = selection(
            None,
            None,
            Some("bitwarden"),
            Some("env-vault"),
            Some("1password"),
            Some("config-vault"),
        )
        .unwrap();
        assert_eq!(s.store, StoreKind::Bitwarden);
        assert_eq!(s.vault.as_deref(), Some("env-vault"));
        let s = selection(
            None,
            None,
            None,
            None,
            Some("bitwarden"),
            Some("config-vault"),
        )
        .unwrap();
        assert_eq!(s.store, StoreKind::Bitwarden);
        assert_eq!(s.vault.as_deref(), Some("config-vault"));
        // Nothing anywhere: auto, no vault.
        assert_eq!(
            selection(None, None, None, None, None, None).unwrap(),
            StoreSelection::auto()
        );
        // A blank flag counts as unset and falls through to env.
        let s = selection(Some(""), None, Some("bitwarden"), None, None, None).unwrap();
        assert_eq!(s.store, StoreKind::Bitwarden);
    }

    #[test]
    fn selection_refuses_a_vault_without_a_store_and_an_unknown_store() {
        // The vault may arrive from a different source than the
        // (absent) store: the resolved pair is what is judged.
        let err = selection(None, Some("v"), None, None, None, None).unwrap_err();
        assert!(matches!(err, SecretError::InvalidSelection(_)), "{err}");
        let err = selection(None, None, None, Some("v"), None, None).unwrap_err();
        assert!(matches!(err, SecretError::InvalidSelection(_)), "{err}");
        let err = selection(Some("vaultwarden"), None, None, None, None, None).unwrap_err();
        assert!(matches!(err, SecretError::InvalidSelection(_)), "{err}");
        // A flag store rescues an env vault.
        let s = selection(Some("bitwarden"), None, None, Some("v"), None, None).unwrap();
        assert_eq!(s.bitwarden_project(), Some("v"));
    }

    #[test]
    fn list_path_scoped_adds_project_beside_prefix() {
        let bw = StoreSelection {
            store: StoreKind::Bitwarden,
            vault: Some("proj-1".to_string()),
        };
        assert_eq!(
            list_path_scoped(&Scope::none(), &bw),
            "/secrets?project=proj-1"
        );
        assert_eq!(
            list_path_scoped(&Scope::prefixed("42").unwrap(), &bw),
            "/secrets?prefix=42&project=proj-1"
        );
        // No vault, or a non-bitwarden store: no project parameter,
        // and the unscoped path is exactly today's.
        let bw_no_vault = StoreSelection {
            store: StoreKind::Bitwarden,
            vault: None,
        };
        assert_eq!(list_path_scoped(&Scope::none(), &bw_no_vault), "/secrets");
        assert_eq!(
            list_path_scoped(&Scope::none(), &StoreSelection::auto()),
            list_path(&Scope::none())
        );
    }

    /// A runner that answers `bws secret list` with fixed entries
    /// carrying `projectId`s, and `bws secret get` by id.
    struct FakeBwsProjects(Vec<serde_json::Value>);

    impl CommandRunner for FakeBwsProjects {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            assert_eq!(program, "bws");
            match args {
                ["secret", "list", "-o", "json"] => {
                    Ok(serde_json::Value::Array(self.0.clone()).to_string())
                }
                ["secret", "get", id, "-o", "json"] => {
                    let entry = self
                        .0
                        .iter()
                        .find(|e| e.get("id").and_then(|i| i.as_str()) == Some(id))
                        .ok_or("exit status 1")?;
                    let mut got = entry.clone();
                    got["value"] = serde_json::json!(format!("value-of-{id}"));
                    Ok(got.to_string())
                }
                _ => Err("exit status 2".to_string()),
            }
        }
    }

    #[test]
    fn bws_source_with_a_project_matches_only_that_project() {
        // Oracle: design record D8 — with a vault, `bws` matches
        // only entries whose projectId equals it.
        let entries = vec![
            serde_json::json!({"id": "id-1", "key": "alpha", "projectId": "proj-1"}),
            serde_json::json!({"id": "id-2", "key": "beta", "projectId": "proj-2"}),
            serde_json::json!({"id": "id-3", "key": "gamma"}),
        ];
        let scoped = BwsSource::with_runner_and_project(
            Box::new(FakeBwsProjects(entries.clone())),
            "proj-1".to_string(),
        );
        assert_eq!(scoped.resolve("alpha").unwrap().expose(), "value-of-id-1");
        // Same name-space, wrong project: not found, never the
        // other project's value.
        assert!(matches!(
            scoped.resolve("beta"),
            Err(SecretError::NotFound { source: "bws", .. })
        ));
        // No project recorded: not a match under a project scope.
        assert!(matches!(
            scoped.resolve("gamma"),
            Err(SecretError::NotFound { source: "bws", .. })
        ));
        // Unscoped, every entry matches as today.
        let unscoped = BwsSource::with_runner(Box::new(FakeBwsProjects(entries)));
        assert_eq!(unscoped.resolve("beta").unwrap().expose(), "value-of-id-2");
    }

    /// A runner that records `op` calls and answers `item list`.
    struct FakeOpList(std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>);

    impl CommandRunner for FakeOpList {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            assert_eq!(program, "op");
            self.0
                .lock()
                .unwrap()
                .push(args.iter().map(|a| a.to_string()).collect());
            Ok(serde_json::json!([{"title": "alpha"}, {"title": "beta"}]).to_string())
        }
    }

    #[test]
    fn op_source_lists_titles_from_its_vault() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let source =
            OpSource::from_values(Some("vault-a"), None, Box::new(FakeOpList(calls.clone())))
                .unwrap();
        assert_eq!(source.list_names().unwrap(), ["alpha", "beta"]);
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0],
            ["item", "list", "--vault", "vault-a", "--format", "json"]
        );
    }

    #[test]
    fn bws_store_vault_overrides_the_configured_write_project() {
        // Oracle: design record D8 — writes through BwsStore use
        // the selection's vault as the project, overriding
        // bws_project_id.
        let mut config = fleet_config::Config::default();
        config.secrets.bws_project_id = Some("config-project".to_string());
        let bw = StoreSelection {
            store: StoreKind::Bitwarden,
            vault: Some("vault-project".to_string()),
        };
        let store = BwsStore::from_config_for(&config, &bw).unwrap();
        assert_eq!(store.project_id, "vault-project");
        // No vault in the selection: the configured project stands.
        let bw_no_vault = StoreSelection {
            store: StoreKind::Bitwarden,
            vault: None,
        };
        let store = BwsStore::from_config_for(&config, &bw_no_vault).unwrap();
        assert_eq!(store.project_id, "config-project");
    }

    #[test]
    fn chain_composition_follows_the_store_value() {
        // Oracle: design record D8's chain column. Source names are
        // the observable composition.
        let config = fleet_config::Config::default();
        let names = |r: &Resolver| -> Vec<&'static str> {
            r.sources.iter().map(|s| s.source_name()).collect()
        };
        // Auto: exactly default_chain's composition.
        assert_eq!(
            names(&Resolver::chain_for(&config, &StoreSelection::auto())),
            names(&Resolver::default_chain(&config))
        );
        // Bitwarden: broker legs + bws, and no op leg — even though
        // this process may have FLEET_OP_VAULT set, the selection's
        // chain is built without consulting it.
        let bw = StoreSelection {
            store: StoreKind::Bitwarden,
            vault: Some("proj-1".to_string()),
        };
        let bw_names = names(&Resolver::chain_for(&config, &bw));
        assert!(bw_names.contains(&"bws"), "{bw_names:?}");
        assert!(!bw_names.contains(&"op"), "{bw_names:?}");
        // 1Password: op only.
        let op = StoreSelection {
            store: StoreKind::OnePassword,
            vault: Some("vault-a".to_string()),
        };
        assert_eq!(names(&Resolver::chain_for(&config, &op)), ["op"]);
    }

    /// A chain source that records the name it was asked for.
    struct RecordingSource(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl SecretSource for RecordingSource {
        fn source_name(&self) -> &'static str {
            "recording"
        }
        fn resolve(&self, name: &str) -> Result<SecretValue, SecretError> {
            self.0.lock().unwrap().push(name.to_string());
            Ok(SecretValue("fixture-value-not-a-secret".to_string()))
        }
    }

    #[test]
    fn resolve_scoped_sends_the_full_name_to_the_chain() {
        for (row, (name, prefix, stored)) in SCOPE_MATRIX.iter().enumerate() {
            let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let resolver = Resolver::new(vec![Box::new(RecordingSource(asked.clone()))]);
            let got = resolver.resolve_scoped(&scope_for(*prefix), name);
            let asked = asked.lock().unwrap().clone();
            match stored {
                Some(want) => {
                    assert!(got.is_ok(), "row {}", row + 1);
                    assert_eq!(asked, [*want], "row {}", row + 1);
                }
                None => {
                    assert!(
                        matches!(got, Err(SecretError::InvalidName(_))),
                        "row {}",
                        row + 1
                    );
                    assert!(asked.is_empty(), "row {}: no source may be asked", row + 1);
                }
            }
        }
    }

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

    /// An in-memory `bws` for the store tests: entries keyed by id,
    /// every call recorded so the argv shape itself is assertable.
    struct FakeBws {
        entries: std::cell::RefCell<Vec<(String, String, String)>>, // (id, key, value)
        calls: std::cell::RefCell<Vec<Vec<String>>>,
        fail_create: bool,
    }

    impl FakeBws {
        fn new(entries: Vec<(&str, &str, &str)>) -> FakeBws {
            FakeBws {
                entries: std::cell::RefCell::new(
                    entries
                        .into_iter()
                        .map(|(i, k, v)| (i.to_string(), k.to_string(), v.to_string()))
                        .collect(),
                ),
                calls: std::cell::RefCell::new(Vec::new()),
                fail_create: false,
            }
        }
        fn value_of(&self, key: &str) -> Option<String> {
            self.entries
                .borrow()
                .iter()
                .find(|(_, k, _)| k == key)
                .map(|(_, _, v)| v.clone())
        }
    }

    impl CommandRunner for FakeBws {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            assert_eq!(program, "bws");
            self.calls
                .borrow_mut()
                .push(args.iter().map(|a| a.to_string()).collect());
            match args {
                ["secret", "list", "-o", "json"] => {
                    let items: Vec<serde_json::Value> = self
                        .entries
                        .borrow()
                        .iter()
                        .map(|(id, key, _)| serde_json::json!({"id": id, "key": key}))
                        .collect();
                    Ok(serde_json::Value::Array(items).to_string())
                }
                ["secret", "get", id, "-o", "json"] => {
                    let entries = self.entries.borrow();
                    let (id, key, value) = entries
                        .iter()
                        .find(|(i, _, _)| i == id)
                        .ok_or("exit status 1")?;
                    Ok(serde_json::json!({"id": id, "key": key, "value": value}).to_string())
                }
                ["secret", "delete", id, "-o", "json"] => {
                    self.entries.borrow_mut().retain(|(i, _, _)| i != id);
                    Ok("{}".to_string())
                }
                [
                    "secret",
                    "create",
                    "-o",
                    "json",
                    "--",
                    name,
                    value,
                    _project,
                ] => {
                    if self.fail_create {
                        return Err("exit status 1".to_string());
                    }
                    let id = format!("id-{}", self.entries.borrow().len() + 1);
                    self.entries.borrow_mut().push((
                        id.clone(),
                        name.to_string(),
                        value.to_string(),
                    ));
                    Ok(serde_json::json!({"id": id}).to_string())
                }
                other => Err(format!("unexpected args: {other:?}")),
            }
        }
    }

    #[test]
    fn bws_store_creates_with_the_house_argv_shape_and_verifies() {
        // Drive it through a shared fake so the calls stay observable.
        let fake = std::rc::Rc::new(FakeBws::new(vec![]));
        struct Shared(std::rc::Rc<FakeBws>);
        impl CommandRunner for Shared {
            fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
                self.0.run(program, args)
            }
        }
        let store = BwsStore::with_runner(Box::new(Shared(fake.clone())), "proj-1".to_string());
        store
            .store("bridge.new-1-token", "fixture-value-not-a-secret")
            .unwrap();
        assert_eq!(
            fake.value_of("bridge.new-1-token").as_deref(),
            Some("fixture-value-not-a-secret")
        );
        let calls = fake.calls.borrow();
        let create = calls
            .iter()
            .find(|c| c[1] == "create")
            .expect("a create call happened");
        assert_eq!(
            create,
            &vec![
                "secret".to_string(),
                "create".to_string(),
                "-o".to_string(),
                "json".to_string(),
                "--".to_string(),
                "bridge.new-1-token".to_string(),
                "fixture-value-not-a-secret".to_string(),
                "proj-1".to_string(),
            ],
            "options first, -- before the positionals, project last"
        );
    }

    #[test]
    fn bws_store_replaces_an_existing_name() {
        let fake = std::rc::Rc::new(FakeBws::new(vec![(
            "id-old",
            "bridge.new-1-token",
            "old-value",
        )]));
        struct Shared(std::rc::Rc<FakeBws>);
        impl CommandRunner for Shared {
            fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
                self.0.run(program, args)
            }
        }
        let store = BwsStore::with_runner(Box::new(Shared(fake.clone())), "proj-1".to_string());
        store.store("bridge.new-1-token", "new-value").unwrap();
        assert_eq!(
            fake.value_of("bridge.new-1-token").as_deref(),
            Some("new-value")
        );
        let calls = fake.calls.borrow();
        let verbs: Vec<&str> = calls.iter().map(|c| c[1].as_str()).collect();
        let delete_at = verbs
            .iter()
            .position(|v| *v == "delete")
            .expect("old entry deleted");
        let create_at = verbs
            .iter()
            .position(|v| *v == "create")
            .expect("new entry created");
        assert!(
            delete_at < create_at,
            "rotation deletes before it creates: {verbs:?}"
        );
        assert_eq!(calls[delete_at][2], "id-old");
    }

    #[test]
    fn bws_store_errors_never_carry_the_value() {
        let mut fake = FakeBws::new(vec![]);
        fake.fail_create = true;
        let store = BwsStore::with_runner(Box::new(fake), "proj-1".to_string());
        let err = store
            .store("bridge.new-1-token", "fixture-value-not-a-secret")
            .unwrap_err();
        let text = format!("{err}");
        assert!(
            text.contains("bridge.new-1-token"),
            "error names the secret: {text}"
        );
        assert!(
            !text.contains("fixture-value-not-a-secret"),
            "error must not carry the value: {text}"
        );
        // An empty value and a bad name are refused before any bws call.
        let store = BwsStore::with_runner(Box::new(FakeBws::new(vec![])), "proj-1".to_string());
        assert!(store.store("bridge.new-1-token", "").is_err());
        assert!(store.store("bad/name", "x").is_err());
    }

    #[test]
    fn bws_project_id_prefers_env_then_config() {
        assert_eq!(
            bws_project_id_from(Some(" env-proj "), Some("cfg-proj")).as_deref(),
            Some("env-proj")
        );
        assert_eq!(
            bws_project_id_from(Some("  "), Some("cfg-proj")).as_deref(),
            Some("cfg-proj")
        );
        assert_eq!(bws_project_id_from(None, None), None);
        assert_eq!(bws_project_id_from(Some(""), Some("")), None);
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
    fn broker_scoped_list_sends_the_prefix_and_never_retries_unscoped() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!(
            "fleet-secrets-scoped-list-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("broker.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            // Two exchanges: a broker that answers the scoped form,
            // then an older broker that does not know it (404).
            for status in [200u16, 404] {
                let (mut stream, _) = listener.accept().unwrap();
                let req: Request = fleet_proxy_proto::read_typed(&mut stream).unwrap();
                assert_eq!(req.path, "/secrets?prefix=42");
                let resp = Response {
                    status,
                    body: serde_json::json!({"names": ["42.a"]}),
                    transport: None,
                };
                fleet_proxy_proto::write_frame(&mut stream, &resp).unwrap();
            }
        });
        let source = BrokerSource::new(sock, PathBuf::from("/nonexistent/token"));
        let scope = Scope::prefixed("42").unwrap();
        assert_eq!(source.list_names_in(&scope).unwrap(), ["42.a"]);
        let err = source.list_names_in(&scope).unwrap_err();
        assert!(
            err.to_string()
                .contains("broker does not support scoped list"),
            "{err}"
        );
        // Exactly two requests reached the broker: no unscoped retry.
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
            ctx: None,
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
        let expected = if is_runnable_program(sys) {
            sys.to_path_buf()
        } else {
            PathBuf::from("bws")
        };
        assert_eq!(resolve_bws_program_from(None, None, &empty_home), expected);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bws_program_resolution_skips_candidates_that_do_not_run() {
        // oracle (#12498): a candidate that passes access(X_OK)
        // but cannot actually run -- a script naming a missing
        // interpreter stands in for a Nix binary whose ELF
        // interpreter is absent -- is skipped, not trusted, and
        // does not shadow a working candidate later in the order.
        // Same for a candidate that runs but exits non-zero.
        use std::os::unix::fs::PermissionsExt;
        let dir = test_temp_dir("bws-resolve-broken");
        let path_dir = dir.join("pathbin");
        let home = dir.join("home");
        std::fs::create_dir_all(&path_dir).unwrap();
        std::fs::create_dir_all(home.join("tools/bin")).unwrap();
        let mk = |p: &Path, body: &str| {
            std::fs::write(p, body).unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let broken = dir.join("broken-bws");
        mk(&broken, "#!/nonexistent/fleet-test-interp\n");
        let nonzero = dir.join("nonzero-bws");
        mk(&nonzero, "#!/bin/sh\nexit 3\n");
        let working = path_dir.join("bws");
        mk(&working, "#!/bin/sh\n");
        let path_var = std::env::join_paths([&path_dir]).unwrap();
        // Both unrunnable overrides are skipped for the working
        // PATH candidate.
        assert_eq!(
            resolve_bws_program_from(Some(broken.to_str().unwrap()), Some(&path_var), &home),
            working
        );
        assert_eq!(
            resolve_bws_program_from(Some(nonzero.to_str().unwrap()), Some(&path_var), &home),
            working
        );
        // An unrunnable PATH candidate is skipped for the working
        // well-known home location.
        let home_bws = home.join("tools/bin/bws");
        mk(&home_bws, "#!/bin/sh\n");
        let broken_path_dir = dir.join("broken-pathbin");
        std::fs::create_dir_all(&broken_path_dir).unwrap();
        mk(
            &broken_path_dir.join("bws"),
            "#!/nonexistent/fleet-test-interp\n",
        );
        let broken_path_var = std::env::join_paths([&broken_path_dir]).unwrap();
        assert_eq!(
            resolve_bws_program_from(None, Some(&broken_path_var), &home),
            home_bws
        );
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
    fn cold_refresh_unavailable_fails_over_without_spending_restart_budget() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        let started = Instant::now();
        let out = resolve_with_retry(&test_retry_policy(), || {
            calls.set(calls.get() + 1);
            Err(SecretError::Unavailable {
                source: "secrets-broker",
                name: "x".to_string(),
                detail: "secrets-broker answered HTTP 503: secrets not loaded yet".to_string(),
            })
        });
        assert!(matches!(out, Err(SecretError::Unavailable { .. })));
        assert_eq!(
            calls.get(),
            1,
            "cold-refresh response should fall through immediately"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
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
    fn broker_cold_refresh_response_returns_within_client_bound() {
        // Oracle (#13893 / #13948 S1): a broker that explicitly says
        // its first refresh has not loaded must not consume the long
        // restart retry budget before the resolver tries its next
        // source. The response body is the production 503 shape.
        let (source, handle, dir) = scripted_broker_replies(vec![(
            503,
            serde_json::json!({"error": "secrets not loaded yet"}),
        )]);
        let started = Instant::now();
        let result = source.resolve("wanted.name");
        assert!(matches!(result, Err(SecretError::Unavailable { .. })));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            handle.join().unwrap(),
            1,
            "one prompt request reaches the broker"
        );
        let _ = std::fs::remove_dir_all(dir);
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
    fn missing_broker_socket_falls_through_to_bws_without_spending_the_budget() {
        // Default policy (2-minute budget): a box with no session broker
        // must reach bws at once, not after the budget.
        let broker = BrokerSource::new(
            PathBuf::from("/nonexistent/broker.sock"),
            PathBuf::from("/nonexistent/token"),
        );
        let resolver = Resolver::new(vec![
            Box::new(broker),
            Box::new(BwsSource::with_runner(Box::new(FakeRunner {
                list: r#"[{"id":"id-1","key":"wanted.name"}]"#.to_string(),
                get: r#"{"id":"id-1","key":"wanted.name","value":"fixture-value-not-a-secret"}"#
                    .to_string(),
            }))),
        ]);
        let started = Instant::now();
        let v = resolver.resolve("wanted.name").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
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

    // ---- bounded subprocess waits (#14040 Residual B) ----
    //
    // Oracle: the never-hang ruling -- a fallback-tier child that
    // never exits must fail its tier within the named bound
    // instead of blocking the caller forever. Each test drives a
    // real `sh -c "sleep 60"` child through the production code
    // path with a short caller-chosen ceiling and asserts both the
    // timeout error and that the elapsed time stays bounded.

    #[test]
    fn bounded_run_kills_a_never_exiting_child_within_the_bound() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60"]);
        let started = Instant::now();
        let err = run_command_bounded(&mut cmd, Duration::from_millis(200))
            .expect_err("a never-exiting child must fail, not hang");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the bound must actually bound: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bounded_run_returns_output_and_status_for_a_quick_child() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hello; exit 3"]);
        let out = run_command_bounded(&mut cmd, Duration::from_secs(10)).unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "hello\n");
    }

    #[test]
    fn process_runner_times_out_a_hanging_child() {
        let runner = ProcessRunner::with_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let err = runner
            .run("sh", &["-c", "sleep 60"])
            .expect_err("a hanging child must fail the tier");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn bws_token_runner_times_out_a_hanging_child() {
        let runner = BwsTokenRunner::with_token(None).with_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let err = runner
            .run("sh", &["-c", "sleep 60"])
            .expect_err("a hanging child must fail the tier");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn program_probe_times_out_a_hanging_candidate() {
        use std::os::unix::fs::PermissionsExt;
        let dir = test_temp_dir("probe-hang");
        let script = dir.join("bws");
        std::fs::write(&script, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        assert!(!program_runs_within(&script, Duration::from_millis(200)));
        assert!(started.elapsed() < Duration::from_secs(10));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- op (1Password) source ----
    //
    // Oracle: the 1Password secret-reference shape
    // `op://<vault>/<item>/<field>` and this crate's own rules
    // (an empty value and a failed child are source failures).

    struct OpRunner {
        answer: Result<String, String>,
        seen: std::rc::Rc<std::cell::RefCell<Vec<Vec<String>>>>,
    }

    impl CommandRunner for OpRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            assert_eq!(program, "op");
            self.seen
                .borrow_mut()
                .push(args.iter().map(|a| a.to_string()).collect());
            self.answer.clone()
        }
    }

    fn op_source(
        vault: Option<&str>,
        field: Option<&str>,
        answer: Result<String, String>,
    ) -> (
        Option<OpSource>,
        std::rc::Rc<std::cell::RefCell<Vec<Vec<String>>>>,
    ) {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let runner = OpRunner {
            answer,
            seen: seen.clone(),
        };
        (OpSource::from_values(vault, field, Box::new(runner)), seen)
    }

    #[test]
    fn op_reads_one_reference_built_from_vault_name_and_field() {
        let (source, seen) = op_source(
            Some("fleet"),
            None,
            Ok("fixture-value-not-a-secret".to_string()),
        );
        let source = source.expect("a named vault turns the source on");
        let v = source.resolve("message-bridge.fleet-token").unwrap();
        assert_eq!(v.expose(), "fixture-value-not-a-secret");
        assert_eq!(
            seen.borrow().as_slice(),
            [vec![
                "read".to_string(),
                "--no-newline".to_string(),
                "op://fleet/message-bridge.fleet-token/credential".to_string(),
            ]]
        );
        // A named field replaces the default one.
        let (source, _) = op_source(Some("fleet"), Some("password"), Ok("x".to_string()));
        assert_eq!(source.unwrap().reference("a.b"), "op://fleet/a.b/password");
    }

    #[test]
    fn op_is_off_unless_a_usable_vault_is_named() {
        for vault in [None, Some(""), Some("   "), Some("a/b"), Some("a?b")] {
            let (source, _) = op_source(vault, None, Ok("x".to_string()));
            assert!(source.is_none(), "vault {vault:?} must leave op off");
        }
        // A field that is not one path segment turns it off too,
        // rather than reading some other field.
        let (source, _) = op_source(Some("fleet"), Some("a/b"), Ok("x".to_string()));
        assert!(source.is_none());
        // Positive control: surrounding whitespace is not a reason.
        let (source, _) = op_source(Some(" fleet "), Some(" "), Ok("x".to_string()));
        assert_eq!(source.unwrap().reference("n"), "op://fleet/n/credential");
    }

    #[test]
    fn op_failures_and_empty_values_are_unavailable_and_carry_no_value() {
        let (source, _) = op_source(Some("fleet"), None, Err("exit status 1".to_string()));
        match source.unwrap().resolve("wanted.name") {
            Err(SecretError::Unavailable { source, detail, .. }) => {
                assert_eq!(source, "op");
                assert_eq!(detail, "op read: exit status 1");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        let (source, _) = op_source(Some("fleet"), None, Ok(String::new()));
        match source.unwrap().resolve("wanted.name") {
            Err(SecretError::Unavailable { source, .. }) => assert_eq!(source, "op"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        // A name that is not a valid secret name never reaches op.
        let (source, seen) = op_source(Some("fleet"), None, Ok("x".to_string()));
        assert!(matches!(
            source.unwrap().resolve("bad/name"),
            Err(SecretError::InvalidName(_))
        ));
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn op_token_prefers_a_nonempty_env_value() {
        let dir = test_temp_dir("op-token-env");
        assert_eq!(
            op_token_from(Some("  env-token  "), &dir).as_deref(),
            Some("env-token")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn op_token_falls_back_to_the_token_files() {
        let dir = test_temp_dir("op-token-file");
        let config_dir = dir.join(".config/op");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("service-account-token"), "file-token\n").unwrap();
        assert_eq!(op_token_from(None, &dir).as_deref(), Some("file-token"));
        // An empty (or whitespace) env value counts as absent —
        // the 2026-11-15 shape for the bws twin, mirrored here.
        assert_eq!(op_token_from(Some(""), &dir).as_deref(), Some("file-token"));
        assert_eq!(
            op_token_from(Some("   "), &dir).as_deref(),
            Some("file-token")
        );
        // The second path is the fallback for the fallback.
        std::fs::remove_dir_all(&dir).ok();
        let dir = test_temp_dir("op-token-file-alt");
        let alt_dir = dir.join(".config/1password");
        std::fs::create_dir_all(&alt_dir).unwrap();
        std::fs::write(alt_dir.join("service-account-token"), "alt-token\n").unwrap();
        assert_eq!(op_token_from(None, &dir).as_deref(), Some("alt-token"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn op_token_absent_everywhere_is_none() {
        let dir = test_temp_dir("op-token-none");
        assert_eq!(op_token_from(None, &dir), None);
        assert_eq!(op_token_from(Some(""), &dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn op_token_runner_controls_the_child_env() {
        // /bin/sh reports whether the variable is set in its
        // environment: the runner must set it from the resolved
        // token, and must remove it when no token resolved (so a
        // stale ambient value cannot leak through either).
        let probe = |runner: &OpTokenRunner| -> String {
            runner
                .run(
                    "sh",
                    &[
                        "-c",
                        "if [ -n \"$OP_SERVICE_ACCOUNT_TOKEN\" ]; then echo set; else echo unset; fi",
                    ],
                )
                .unwrap()
                .trim()
                .to_string()
        };
        assert_eq!(
            probe(&OpTokenRunner::with_token(Some(
                "fixture-token".to_string()
            ))),
            "set"
        );
        assert_eq!(probe(&OpTokenRunner::with_token(None)), "unset");
    }

    #[test]
    fn op_token_runner_times_out_a_hanging_child() {
        let runner = OpTokenRunner::with_token(None).with_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let err = runner
            .run("sh", &["-c", "sleep 60"])
            .expect_err("a hanging child must fail the tier");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn op_from_config_prefers_env_then_file() {
        // Env manipulation is process-wide: save and restore so
        // parallel tests never observe our values.
        let saved_vault = std::env::var("FLEET_OP_VAULT").ok();
        let saved_field = std::env::var("FLEET_OP_FIELD").ok();
        unsafe {
            std::env::remove_var("FLEET_OP_VAULT");
            std::env::remove_var("FLEET_OP_FIELD");
        }
        let mut config = fleet_config::Config::default();
        config.secrets.op_vault = Some("file-vault".to_string());
        config.secrets.op_field = Some("password".to_string());
        // File values turn the source on with the configured field.
        let source = OpSource::from_config(&config).expect("file vault turns op on");
        assert_eq!(source.reference("a.b"), "op://file-vault/a.b/password");
        // Env beats the file when both are set; a blank env value
        // counts as absent, not as a vault.
        unsafe {
            std::env::set_var("FLEET_OP_VAULT", " env-vault ");
            std::env::set_var("FLEET_OP_FIELD", "  ");
        }
        let source = OpSource::from_config(&config).expect("env vault turns op on");
        assert_eq!(source.reference("a.b"), "op://env-vault/a.b/password");
        // Neither names a vault: the source stays off.
        unsafe {
            std::env::remove_var("FLEET_OP_VAULT");
            std::env::remove_var("FLEET_OP_FIELD");
        }
        let config = fleet_config::Config::default();
        assert!(OpSource::from_config(&config).is_none());
        // Restore the ambient values.
        unsafe {
            match saved_vault {
                Some(v) => std::env::set_var("FLEET_OP_VAULT", v),
                None => std::env::remove_var("FLEET_OP_VAULT"),
            }
            match saved_field {
                Some(v) => std::env::set_var("FLEET_OP_FIELD", v),
                None => std::env::remove_var("FLEET_OP_FIELD"),
            }
        }
    }

    // ---- S5 client surface (#13948 §5.4) ----

    #[test]
    fn op_capability_readiness_needs_every_leg() {
        // All four observations present: a mirror write may be
        // attempted.
        assert!(
            OpCapability::from_observations(true, true, true, true).mirror_write_ready()
        );
        // Any single gating leg missing: the put records propagation
        // pending instead of attempting the mirror write (§5.5). The
        // service-account token is informational (an account session
        // can also authorize the write), so it alone never gates.
        for missing in [0, 1, 3] {
            let mut legs = [true, true, true, true];
            legs[missing] = false;
            assert!(
                !OpCapability::from_observations(legs[0], legs[1], legs[2], legs[3])
                    .mirror_write_ready(),
                "leg {missing} missing must keep the mirror write off"
            );
        }
        assert!(
            OpCapability::from_observations(true, true, false, true).mirror_write_ready(),
            "token alone is informational, not gating"
        );
        // The live spike shape (2026-11-28 laptop seat: CLI present,
        // no account, no token, no vault) is not ready.
        assert!(
            !OpCapability::from_observations(true, false, false, false).mirror_write_ready()
        );
    }

    #[test]
    fn broker_health_parse_accepts_s5_and_pre_s5_bodies() {
        // A current broker: leg present, watermark installed.
        let body = serde_json::json!({
            "ok": true,
            "serving": true,
            "secrets": 42,
            "last_refresh_unix": 1700000000u64,
            "refreshing": false,
            "refresh_progress": null,
            "refresh_leg": "upstream-delta",
            "oldest_secret_age_seconds": 7,
        });
        assert_eq!(
            BrokerHealth::parse(&body).unwrap(),
            BrokerHealth {
                serving: true,
                secrets: 42,
                refreshing: false,
                refresh_leg: Some("upstream-delta".to_string()),
                last_refresh_unix: Some(1700000000),
            }
        );
        // A pre-S5 broker: no refresh_leg, no watermark yet — the
        // parse still succeeds with both as None.
        let body = serde_json::json!({
            "ok": false,
            "serving": false,
            "secrets": 0,
            "last_refresh_unix": null,
            "refreshing": true,
            "refresh_progress": {"started_unix": 1, "fetched": 2, "total": 9},
        });
        assert_eq!(
            BrokerHealth::parse(&body).unwrap(),
            BrokerHealth {
                serving: false,
                secrets: 0,
                refreshing: true,
                refresh_leg: None,
                last_refresh_unix: None,
            }
        );
        // An explicit null leg is None, not an error.
        let body = serde_json::json!({
            "serving": true,
            "secrets": 1,
            "refreshing": false,
            "refresh_leg": null,
        });
        assert_eq!(BrokerHealth::parse(&body).unwrap().refresh_leg, None);
        // A non-string leg is a source failure, never silently
        // dropped: the caller must not misreport the serving leg.
        let body = serde_json::json!({
            "serving": true,
            "secrets": 1,
            "refreshing": false,
            "refresh_leg": 7,
        });
        assert!(BrokerHealth::parse(&body).is_err());
        // A body missing the required liveness fields is a source
        // failure, never a defaulted guess.
        let body = serde_json::json!({"refreshing": false});
        assert!(BrokerHealth::parse(&body).is_err());
    }

    #[test]
    fn op_store_edits_the_existing_item_and_creates_the_missing_one() {
        // The argv shape is pinned but has no live-spike receipt
        // (open design question 4): these tests prove what the dark
        // verb WOULD run, not that a vault accepts it.
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let present =
            OpStore::with_runner(Box::new(FakeOpList(calls.clone())), "vault-a".into(), "credential".into());
        present.store("alpha", "s3cr3t").unwrap();
        let calls = calls.lock().unwrap();
        // First the metadata-only existence check, then the edit.
        assert_eq!(calls[0], ["item", "list", "--vault", "vault-a", "--format", "json"]);
        assert_eq!(
            calls[1],
            ["item", "edit", "--vault", "vault-a", "alpha", "credential=s3cr3t"]
        );

        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let missing =
            OpStore::with_runner(Box::new(FakeOpList(calls.clone())), "vault-a".into(), "credential".into());
        missing.store("gamma", "s3cr3t").unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls[1][..7], ["item", "create", "--category", "password", "--vault", "vault-a", "--title"]);
        assert_eq!(calls[1][7], "gamma");
        assert_eq!(calls[1][8], "credential=s3cr3t");
    }

    #[test]
    fn op_store_refuses_bad_names_before_spawning() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let store =
            OpStore::with_runner(Box::new(FakeOpList(calls.clone())), "vault-a".into(), "credential".into());
        assert!(store.store("", "s3cr3t").is_err());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn upstream_chain_skips_broker_legs_but_keeps_stores() {
        let config = fleet_config::Config::default();
        let auto = StoreSelection::auto();
        // No vault named in this environment's config: bws only, no
        // broker leg, no op leg.
        let names = Resolver::upstream_chain(&config, &auto).source_names();
        assert!(!names.contains(&"secrets-broker"), "upstream skips caches: {names:?}");
        assert!(names.contains(&"bws"), "upstream keeps the primary: {names:?}");
        assert!(!names.contains(&"op"), "no vault, no mirror leg: {names:?}");
        // A bitwarden project selection scopes the single bws leg.
        let project = StoreSelection::from_flags_and_config(Some("bitwarden"), Some("proj-1"), &config)
            .unwrap();
        assert_eq!(
            Resolver::upstream_chain(&config, &project).source_names(),
            ["bws"]
        );
        assert_eq!(
            Resolver::primary_only(&project).source_names(),
            ["bws"],
            "no-cache is the commit-point truth only"
        );
        assert_eq!(
            Resolver::primary_only(&auto).source_names(),
            ["bws"]
        );
    }
}
