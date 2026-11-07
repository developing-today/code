//! fleet-config — the one place fleet configuration resolves
//! (design note §1/§7, epic developing-today/code#5412).
//!
//! Sources, in precedence order:
//!   1. env overrides (`FLEET_CONFIG`, `FLEET_BROKER_SOCKET`);
//!   2. the config file — `$FLEET_CONFIG`, else
//!      `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`;
//!   3. compiled-in defaults matching today's live systems.
//!
//! The `[domains]` table is the config-level domain disable (§7): a
//! domain set to `false` dispatches to the capability stub with exit 69
//! and `disabled by config on this host` wording — the per-host shaping
//! and incident lever. A missing file is not an error (defaults rule);
//! a malformed file is a loud error, never a silent fallback, because a
//! silently ignored disable would defeat the lever. Unknown sections and
//! keys are malformed for this purpose (#5460): a misspelled `[domains]`
//! table or `idle_exit_secs` key is a parse error, not a silent default.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// The session broker's socket, as the Python broker runs it today.
pub const DEFAULT_BROKER_SOCKET: &str = ".cache/laptop-agent/broker.sock";
/// The queue broker's Unix socket (#6216): dual-bound alongside its
/// TCP listener, in the broker's own config dir (token file sibling).
pub const DEFAULT_QUEUE_BROKER_SOCKET: &str = ".config/queue-broker/broker.sock";
/// The secrets broker's Unix socket (#6216): dual-bound alongside its
/// TCP listener, in the broker's own config dir (token file sibling).
pub const DEFAULT_SECRETS_BROKER_SOCKET: &str = ".config/secrets-broker/broker.sock";
/// The proxy's per-port routing table (§4): port → service name.
pub const PORT_LAPTOP_AGENT: u16 = 4799;
pub const PORT_SECRETS_BROKER: u16 = 4800;
pub const PORT_QUEUE_BROKER: u16 = 4801;
/// Broker idle exit (§4: retained from the Python broker — a warmth
/// cache that exits when cold is a feature on a shared sandbox).
pub const DEFAULT_IDLE_EXIT_SECS: u64 = 3600;
/// The laptop's tailnet address — the proxy's one reach target in
/// v1 (fleet-proxy's CONNECT construction dials it through the
/// egress proxy). Config can name another host for a future relay.
pub const DEFAULT_TARGET_HOST: &str = "100.99.113.103";
/// The tailnet egress proxy's port on the host named by
/// `HTTPS_PROXY` — fixed by the tunnel setup, as in
/// laptop-proxy-connect.py (the URL's own port is not used).
pub const DEFAULT_EGRESS_PROXY_PORT: u16 = 3130;

/// Default listen address for the CONNECT-UDP endpoint
/// (RFC 9298 spike, #6109) when `[proxy] connect_udp` enables
/// it: loopback only. The endpoint is a local capability,
/// never a LAN surface, until a future cutover decides
/// otherwise.
///
/// Port history (pm-stack/PORTS.md is the authoritative
/// registry): the original default was 3131, one above the
/// runtime egress proxy's 3130. The dark unit deployed on
/// amd on 2026-11-05 binds 3132 from its own config
/// (`~/.config/fleet/connect-udp.toml`) because 3131 was
/// believed — wrongly — to be the pm Pad service's port
/// (Pad is on 7777; the 3131 story was stale). The default
/// moved to 3132 to match the deployed unit, so code truth
/// and the one live deployment agree; 3131 is retired and
/// must not be re-claimed without updating the registry.
pub const DEFAULT_CONNECT_UDP_LISTEN: &str = "127.0.0.1:3132";
/// `[search]` — the search domain's routing view (step 5, #5433).
/// All optional: the Q3 host default needs no config at all (the
/// serving host's `service.json` marks the laptop seat). `via` is
/// "service" or "standalone"; any other value is ignored by the
/// domain's resolution (which then falls through to the host
/// default) — the value shapes a *preference between two working
/// routes*, not a disable, so unlike `[domains]` it is not a
/// loud-error lever.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    /// Routing preference: "service" | "standalone".
    pub via: Option<String>,
    /// Service base URL (`http://host[:port]`); the domain's own
    /// resolution is flag → this → `ATLAS_SEARCH_URL` → default.
    pub url: Option<String>,
}

/// The token file the session broker reads when a request names
/// none — the laptop-agent token, exactly the Python broker's
/// default (`la.TOKEN_FILE`).
pub const DEFAULT_BROKER_TOKEN_FILE: &str = "tools/laptop-agent-token";

/// The token file whose contents the session broker sends as
/// `X-Agent-Token` for secrets-broker requests. fleet only ever names
/// this file; the broker reads it per request (§1: token files stay
/// files, read per request, never stored, never logged).
pub const DEFAULT_SECRETS_TOKEN_FILE: &str = "tools/secrets-broker-token";

/// pm-dispatch's port on the laptop (pm-stack/dispatch: the intake
/// service binds loopback + the tailnet address there).
pub const PORT_DISPATCH: u16 = 4879;

/// The fleet-secrets name the dispatch domain resolves its bearer
/// token by (step 4, #5432). The value is pm-dispatch's
/// `DISPATCH_API_TOKEN`; its by-name home is the vault, per the
/// platform rule that secrets resolve by name through fleet-secrets
/// only.
pub const DEFAULT_DISPATCH_TOKEN_NAME: &str = "pm-dispatch.api-token";

/// The fleet-secrets name the dispatch-test domain resolves its
/// GitHub token by (design developing-today/code#6580).
pub const DEFAULT_DISPATCH_TEST_TOKEN_NAME: &str = "github.ghcr-classic-pat";

/// The fleet-secrets name the dispatch-test domain resolves its
/// Turso jobs-store token by.
pub const DEFAULT_DISPATCH_TEST_TURSO_TOKEN_NAME: &str = "turso.fleet-dispatch-jobs-token";

/// The Turso jobs store the dispatch-test domain records jobs in.
pub const DEFAULT_DISPATCH_TEST_TURSO_URL: &str =
    "https://fleet-dispatch-jobs-dezren39.aws-us-east-1.turso.io";

/// The default Actions-capacity threshold for dispatch-test scope
/// selection: a scope is eligible below this many active jobs.
pub const DEFAULT_DISPATCH_TEST_THRESHOLD: u64 = 40;

/// The fleet repo: the dezren39 repo that holds the other fleet
/// repos as git submodules (developing-today/code#6580 phase 2).
/// Its copies — the source plus its fork network — are the
/// dispatch-test execution homes, discovered dynamically unless
/// `[dispatch-test] scopes` pins an explicit list.
pub const DEFAULT_DISPATCH_TEST_FLEET_REPO: &str = "dezren39/fleet";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `[domains]` — domain name → enabled. Absent means enabled.
    pub domains: BTreeMap<String, bool>,
    pub proxy: ProxyConfig,
    pub secrets: SecretsConfig,
    pub dispatch: DispatchConfig,
    #[serde(rename = "dispatch-test")]
    pub dispatch_test: DispatchTestConfig,
    pub search: SearchConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyConfig {
    /// Session broker socket path (relative paths resolve under $HOME).
    pub broker_socket: Option<String>,
    /// Broker idle-exit timeout in seconds (default
    /// [`DEFAULT_IDLE_EXIT_SECS`]). `0` is rejected at load (#6412):
    /// it would exit the broker at its first idle tick, and there is
    /// no "never idle-exit" spelling.
    pub idle_exit_secs: Option<u64>,
    /// The reach target for CONNECT tunnels (default
    /// [`DEFAULT_TARGET_HOST`]).
    pub target_host: Option<String>,
    /// Port of the egress proxy on the `HTTPS_PROXY` host (default
    /// [`DEFAULT_EGRESS_PROXY_PORT`]). `0` is rejected at load
    /// (#6412): port 0 cannot be dialled.
    pub egress_proxy_port: Option<u16>,
    /// Dial the target directly instead of through a CONNECT tunnel
    /// (a broker running on the target host itself; also the
    /// hermetic-test mode). Default false — today's topology.
    pub direct: Option<bool>,
    /// Whether the broker also serves CONNECT-UDP (RFC 9298,
    /// #6109) on [`Config::connect_udp_listen`]. Default false:
    /// a spike capability that merges dark — nothing listens
    /// anywhere unless a host's config deliberately opts in,
    /// and opting in is a separate act from landing the code.
    pub connect_udp: Option<bool>,
    /// Listen address for the CONNECT-UDP endpoint when
    /// enabled (default [`DEFAULT_CONNECT_UDP_LISTEN`]).
    pub connect_udp_listen: Option<String>,
    /// The routing table the broker advertises in its hello
    /// (§4: the services map IS the routing table). Empty means the
    /// built-in table ([`Config::services`]).
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
}

/// One row of the proxy routing table: a laptop port and the
/// service the broker routes to it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    pub port: u16,
    pub name: String,
    /// Per-service version string for the hello map; absent means
    /// the built-in default for the name (see [`Config::services`]).
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecretsConfig {
    /// Path of the secrets-broker token file (handed to the session
    /// broker per request; fleet never opens it).
    pub token_file: Option<String>,
}

/// `[dispatch]` — the dispatch thin client's view of pm-dispatch
/// (step 4, #5432). All optional: the laptop seat needs no config
/// at all (the client probes the loopback bind), and the sandbox
/// seat rides the proxy egress to the target host's dispatch port.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DispatchConfig {
    /// Direct base URL (`http://host[:port]`) for pm-dispatch,
    /// bypassing transport selection. `FLEET_DISPATCH_URL` beats
    /// the file (the fleet-queue client's env-first pattern).
    pub url: Option<String>,
    /// The fleet-secrets name the bearer token resolves by
    /// (default [`DEFAULT_DISPATCH_TOKEN_NAME`]).
    pub token_name: Option<String>,
    /// pm-dispatch's port on the target host (default
    /// [`PORT_DISPATCH`]). `0` is rejected at load (#6412): port 0
    /// cannot be dialled.
    pub port: Option<u16>,
}

/// `[dispatch-test]` — the dispatch-test domain's view
/// (developing-today/code#6580). All optional: the defaults name
/// the live GitHub token, the Turso jobs store, and the scope
/// repos used for capacity-based selection.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DispatchTestConfig {
    /// The fleet-secrets name the GitHub token resolves by
    /// (default [`DEFAULT_DISPATCH_TEST_TOKEN_NAME`]).
    pub token_name: Option<String>,
    /// The fleet-secrets name the Turso token resolves by
    /// (default [`DEFAULT_DISPATCH_TEST_TURSO_TOKEN_NAME`]).
    pub turso_token_name: Option<String>,
    /// The Turso database URL (default
    /// [`DEFAULT_DISPATCH_TEST_TURSO_URL`]).
    pub turso_url: Option<String>,
    /// Actions-capacity threshold (default
    /// [`DEFAULT_DISPATCH_TEST_THRESHOLD`]).
    pub threshold: Option<u64>,
    /// The fleet repo whose copies are the execution homes
    /// (default [`DEFAULT_DISPATCH_TEST_FLEET_REPO`]).
    pub fleet_repo: Option<String>,
    /// Scope repos, in selection order — a PIN/OVERRIDE: when
    /// non-empty, exactly these execution-home copies are scanned.
    /// Empty (the default) means DISCOVER: the fleet repo plus its
    /// fork network.
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    /// The config file exists but does not parse. Carries the path and
    /// the parser's message — never config *values* beyond what the
    /// parser itself reports.
    Parse { path: PathBuf, message: String },
    /// The config file exists but cannot be read.
    Read { path: PathBuf, message: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Parse { path, message } => {
                write!(
                    f,
                    "fleet: cannot parse config {}: {message}",
                    path.display()
                )
            }
            ConfigError::Read { path, message } => {
                write!(f, "fleet: cannot read config {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// An env var read as a path, with an empty value treated as unset.
/// Empty-string env vars are a routine artifact (systemd units,
/// templated exports), and the XDG base-directory spec treats an
/// empty base as invalid; taking one literally here would turn the
/// config file, the state dir, and the broker socket into
/// CWD-relative paths (issue #5473).
fn non_empty_path(value: Option<PathBuf>) -> Option<PathBuf> {
    value.filter(|p| !p.as_os_str().is_empty())
}

fn env_path(var: &str) -> Option<PathBuf> {
    non_empty_path(std::env::var_os(var).map(PathBuf::from))
}

/// An XDG base directory: only an absolute, non-empty value counts.
/// The XDG base-directory spec says a relative value in these
/// variables should be considered invalid and ignored; taking one
/// literally would resolve the config file and the state dir from
/// the process's working directory, so a `fleet.toml` planted at
/// `./<relative-base>/fleet/fleet.toml` would load despite an
/// absolute `HOME` (issue #6396 — the relative spelling of the
/// #5473 planted-config hazard, and the XDG sibling of the
/// relative-HOME rule in `home_anchor`). Scoped to the XDG bases
/// only: explicit overrides (`FLEET_CONFIG`, the broker socket
/// paths) keep their documented relative meanings.
fn absolute_path(value: Option<PathBuf>) -> Option<PathBuf> {
    non_empty_path(value).filter(|p| p.is_absolute())
}

fn env_xdg_base(var: &str) -> Option<PathBuf> {
    absolute_path(std::env::var_os(var).map(PathBuf::from))
}

/// The home anchor: only an absolute, non-empty `HOME` may anchor
/// home-relative resolution. A relative `HOME` is treated as unset
/// for anchoring (issue #6404): used verbatim it would resolve the
/// config file, the state dir, and the broker socket CWD-relative,
/// so a `fleet.toml` planted under the working directory at
/// `./<relative-home>/.config/fleet/fleet.toml` would load — the
/// planted-config hazard of issue #5473 in its relative-HOME
/// spelling. Falling back to "." here means the unset/empty case
/// and the relative case share one behaviour, governed by the
/// fallback decision tracked in issue #5737. Scoped to the HOME
/// anchor only: relative XDG bases are issue #6396, and relative
/// values from explicit overrides (FLEET_CONFIG, socket paths) keep
/// their documented resolve-under-home meaning.
fn home_anchor(value: Option<PathBuf>) -> PathBuf {
    non_empty_path(value)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn home_dir() -> PathBuf {
    home_anchor(std::env::var_os("HOME").map(PathBuf::from))
}

fn resolve_under(home: &std::path::Path, path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

fn resolve_under_home(path: &str) -> PathBuf {
    resolve_under(&home_dir(), &PathBuf::from(path))
}

impl Config {
    /// The config file path: `$FLEET_CONFIG`, else
    /// `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`.
    pub fn file_path() -> PathBuf {
        Self::file_path_from(
            env_path("FLEET_CONFIG"),
            env_xdg_base("XDG_CONFIG_HOME"),
            &home_dir(),
        )
    }

    fn file_path_from(
        fleet_config: Option<PathBuf>,
        xdg_config_home: Option<PathBuf>,
        home: &std::path::Path,
    ) -> PathBuf {
        if let Some(p) = fleet_config {
            return p;
        }
        let base = xdg_config_home.unwrap_or_else(|| home.join(".config"));
        base.join("fleet").join("fleet.toml")
    }

    /// Load configuration. A missing file yields defaults; a malformed
    /// or unreadable file is an error (see the crate docs for why).
    pub fn load() -> Result<Config, ConfigError> {
        let path = Self::file_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Result<Config, ConfigError> {
        let cfg: Config = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| ConfigError::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                });
            }
        };
        cfg.validate(path)?;
        Ok(cfg)
    }

    /// The crate's single zero rule (#6412, matching the direction of
    /// #6397 for the `[[proxy.services]]` rows): a configured `0` for a
    /// scalar numeric setting is a misspelling-class error, never a
    /// value. Port 0 cannot be dialled, and `idle_exit_secs = 0` would
    /// make the broker exit at its first idle tick — the opposite of
    /// the "never idle-exit" an operator would mean by it. Reject all
    /// three at load, naming the field; omitting the field keeps the
    /// documented default. There is no zero spelling for "disabled".
    fn validate(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        let reject = |field: &str, default: u64| ConfigError::Parse {
            path: path.to_path_buf(),
            message: format!("{field} must not be 0; omit the field for the default ({default})"),
        };
        if self.proxy.egress_proxy_port == Some(0) {
            return Err(reject(
                "[proxy] egress_proxy_port",
                u64::from(DEFAULT_EGRESS_PROXY_PORT),
            ));
        }
        if self.dispatch.port == Some(0) {
            return Err(reject("[dispatch] port", u64::from(PORT_DISPATCH)));
        }
        if self.proxy.idle_exit_secs == Some(0) {
            return Err(reject("[proxy] idle_exit_secs", DEFAULT_IDLE_EXIT_SECS));
        }
        Ok(())
    }

    /// Is a domain enabled on this host? Default yes; `[domains]`
    /// decides. Unknown names are enabled (the registry, not config,
    /// decides what exists).
    pub fn domain_enabled(&self, domain: &str) -> bool {
        self.domains.get(domain).copied().unwrap_or(true)
    }

    /// The session broker socket path. `FLEET_BROKER_SOCKET` beats the
    /// config file beats the default.
    pub fn broker_socket(&self) -> PathBuf {
        Self::broker_socket_from(
            env_path("FLEET_BROKER_SOCKET"),
            self.proxy.broker_socket.as_deref(),
            &home_dir(),
        )
    }

    fn broker_socket_from(
        env_socket: Option<PathBuf>,
        config_socket: Option<&str>,
        home: &std::path::Path,
    ) -> PathBuf {
        // Every source resolves the same way: a relative path is under
        // $HOME (the ProxyConfig field's documented rule), never the
        // process's working directory (issue #5473).
        if let Some(p) = env_socket {
            return resolve_under(home, &p);
        }
        match config_socket {
            Some(p) => resolve_under(home, &PathBuf::from(p)),
            None => resolve_under(home, &PathBuf::from(DEFAULT_BROKER_SOCKET)),
        }
    }

    /// The queue broker's Unix socket path (#6216). `QUEUE_BROKER_SOCKET`
    /// beats the default; no config-file field exists for it, so the
    /// resolution is env -> default under $HOME, like the session
    /// broker's socket with the config source absent.
    pub fn queue_broker_socket(&self) -> PathBuf {
        Self::broker_socket_from(
            env_path("QUEUE_BROKER_SOCKET"),
            Some(DEFAULT_QUEUE_BROKER_SOCKET),
            &home_dir(),
        )
    }

    /// The secrets broker's Unix socket path (#6216).
    /// `SECRETS_BROKER_SOCKET` beats the default.
    pub fn secrets_broker_socket(&self) -> PathBuf {
        Self::broker_socket_from(
            env_path("SECRETS_BROKER_SOCKET"),
            Some(DEFAULT_SECRETS_BROKER_SOCKET),
            &home_dir(),
        )
    }

    pub fn idle_exit_secs(&self) -> u64 {
        self.proxy.idle_exit_secs.unwrap_or(DEFAULT_IDLE_EXIT_SECS)
    }

    /// The proxy's reach target (see [`DEFAULT_TARGET_HOST`]).
    pub fn target_host(&self) -> &str {
        self.proxy
            .target_host
            .as_deref()
            .unwrap_or(DEFAULT_TARGET_HOST)
    }

    /// The egress proxy port on the `HTTPS_PROXY` host.
    pub fn egress_proxy_port(&self) -> u16 {
        self.proxy
            .egress_proxy_port
            .unwrap_or(DEFAULT_EGRESS_PROXY_PORT)
    }

    /// Whether the broker dials the target directly (no CONNECT).
    pub fn proxy_direct(&self) -> bool {
        self.proxy.direct.unwrap_or(false)
    }

    /// Whether the CONNECT-UDP endpoint is enabled (#6109).
    /// Absent means disabled: the capability merges dark.
    pub fn connect_udp_enabled(&self) -> bool {
        self.proxy.connect_udp.unwrap_or(false)
    }

    /// The CONNECT-UDP endpoint's listen address.
    pub fn connect_udp_listen(&self) -> &str {
        self.proxy
            .connect_udp_listen
            .as_deref()
            .unwrap_or(DEFAULT_CONNECT_UDP_LISTEN)
    }

    /// The routing table: config's `[proxy] services`, or the
    /// built-in table matching today's live system — 4799 the
    /// (Python) laptop-agent daemon, 4800/4801 the fleet broker
    /// roles (§4: a serve build without a role advertises a map
    /// without its port; the table is how that is said).
    pub fn services(&self) -> Vec<ServiceConfig> {
        if !self.proxy.services.is_empty() {
            return self.proxy.services.clone();
        }
        let fleet_version = env!("CARGO_PKG_VERSION").to_string();
        [
            (PORT_LAPTOP_AGENT, "laptop-agent", "python".to_string()),
            (PORT_SECRETS_BROKER, "secrets-broker", fleet_version.clone()),
            (PORT_QUEUE_BROKER, "queue-broker", fleet_version),
        ]
        .into_iter()
        .map(|(port, name, version)| ServiceConfig {
            port,
            name: name.to_string(),
            version: Some(version),
        })
        .collect()
    }

    /// The token file the session broker reads when a request names
    /// none (see [`DEFAULT_BROKER_TOKEN_FILE`]).
    pub fn default_token_file(&self) -> PathBuf {
        resolve_under_home(DEFAULT_BROKER_TOKEN_FILE)
    }

    /// The dispatch base URL when one is configured:
    /// `FLEET_DISPATCH_URL` (empty counts as unset, as with every
    /// env path here) beats `[dispatch] url`.
    pub fn dispatch_url(&self) -> Option<String> {
        match std::env::var("FLEET_DISPATCH_URL") {
            Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            _ => self.dispatch.url.clone(),
        }
    }

    /// The fleet-secrets name the dispatch bearer token resolves by.
    pub fn dispatch_token_name(&self) -> &str {
        self.dispatch
            .token_name
            .as_deref()
            .unwrap_or(DEFAULT_DISPATCH_TOKEN_NAME)
    }

    /// pm-dispatch's port on the target host.
    pub fn dispatch_port(&self) -> u16 {
        self.dispatch.port.unwrap_or(PORT_DISPATCH)
    }

    /// The fleet-secrets name the dispatch-test GitHub token
    /// resolves by.
    pub fn dispatch_test_token_name(&self) -> &str {
        self.dispatch_test
            .token_name
            .as_deref()
            .unwrap_or(DEFAULT_DISPATCH_TEST_TOKEN_NAME)
    }

    /// The fleet-secrets name the dispatch-test Turso token
    /// resolves by.
    pub fn dispatch_test_turso_token_name(&self) -> &str {
        self.dispatch_test
            .turso_token_name
            .as_deref()
            .unwrap_or(DEFAULT_DISPATCH_TEST_TURSO_TOKEN_NAME)
    }

    /// The dispatch-test Turso jobs-store URL.
    pub fn dispatch_test_turso_url(&self) -> &str {
        self.dispatch_test
            .turso_url
            .as_deref()
            .unwrap_or(DEFAULT_DISPATCH_TEST_TURSO_URL)
    }

    /// The dispatch-test Actions-capacity threshold.
    pub fn dispatch_test_threshold(&self) -> u64 {
        self.dispatch_test
            .threshold
            .unwrap_or(DEFAULT_DISPATCH_TEST_THRESHOLD)
    }

    /// The dispatch-test fleet repo (execution-home source).
    pub fn dispatch_test_fleet_repo(&self) -> &str {
        self.dispatch_test
            .fleet_repo
            .as_deref()
            .unwrap_or(DEFAULT_DISPATCH_TEST_FLEET_REPO)
    }

    /// The pinned dispatch-test scope repos, in selection order.
    /// Empty means no pin: the caller discovers the fleet repo's
    /// copies (source + fork network) instead.
    pub fn dispatch_test_scopes(&self) -> Vec<String> {
        self.dispatch_test.scopes.clone()
    }

    /// The search domain's routing preference from `[search] via`
    /// ("service" | "standalone"), when configured.
    pub fn search_via(&self) -> Option<String> {
        self.search
            .via
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| v == "service" || v == "standalone")
    }

    /// The search service base URL from `[search] url`, when set.
    pub fn search_url(&self) -> Option<String> {
        self.search
            .url
            .as_deref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// The secrets-broker token *file path* (never its contents).
    pub fn secrets_token_file(&self) -> PathBuf {
        match &self.secrets.token_file {
            Some(p) => resolve_under_home(p),
            None => resolve_under_home(DEFAULT_SECRETS_TOKEN_FILE),
        }
    }

    /// `~/.local/state/fleet` (or `$XDG_STATE_HOME/fleet`).
    pub fn state_dir() -> PathBuf {
        Self::state_dir_from(env_xdg_base("XDG_STATE_HOME"), &home_dir())
    }

    fn state_dir_from(xdg_state_home: Option<PathBuf>, home: &std::path::Path) -> PathBuf {
        let base = xdg_state_home.unwrap_or_else(|| home.join(".local").join("state"));
        base.join("fleet")
    }

    /// The materialized PATH shim dir (§4), regenerated from the
    /// negotiated services map at every connect/init.
    pub fn shims_dir() -> PathBuf {
        Self::state_dir().join("shims")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(contents: &str) -> (tempfile_dir::Dir, PathBuf) {
        let dir = tempfile_dir::Dir::new();
        let path = dir.path().join("fleet.toml");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn missing_file_is_defaults() {
        let cfg = Config::load_from(PathBuf::from("/nonexistent/fleet.toml").as_path()).unwrap();
        assert!(cfg.domain_enabled("queue"));
        assert_eq!(cfg.idle_exit_secs(), DEFAULT_IDLE_EXIT_SECS);
    }

    #[test]
    fn connect_udp_is_dark_by_default() {
        // #6109: the spike capability must not listen anywhere
        // a config has not deliberately enabled it.
        let cfg = Config::load_from(PathBuf::from("/nonexistent/fleet.toml").as_path()).unwrap();
        assert!(!cfg.connect_udp_enabled());
        assert_eq!(cfg.connect_udp_listen(), DEFAULT_CONNECT_UDP_LISTEN);
        let (_dir, path) = write_temp("[proxy]\nconnect_udp = false\n");
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.connect_udp_enabled());
    }

    #[test]
    fn connect_udp_opt_in_parses() {
        let (_dir, path) =
            write_temp("[proxy]\nconnect_udp = true\nconnect_udp_listen = \"127.0.0.1:9999\"\n");
        let cfg = Config::load_from(&path).unwrap();
        assert!(cfg.connect_udp_enabled());
        assert_eq!(cfg.connect_udp_listen(), "127.0.0.1:9999");
    }

    #[test]
    fn domains_table_disables() {
        let (_dir, path) = write_temp("[domains]\nqueue = false\n");
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.domain_enabled("queue"));
        assert!(cfg.domain_enabled("search"));
    }

    #[test]
    fn search_section_parses() {
        let (_dir, path) =
            write_temp("[search]\nvia = \"service\"\nurl = \"http://127.0.0.1:18760\"\n");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.search_via().as_deref(), Some("service"));
        assert_eq!(cfg.search_url().as_deref(), Some("http://127.0.0.1:18760"));
        // An unrecognized via value is not a preference at all.
        let (_dir2, path2) = write_temp("[search]\nvia = \"sideways\"\n");
        let cfg2 = Config::load_from(&path2).unwrap();
        assert_eq!(cfg2.search_via(), None);
    }

    #[test]
    fn malformed_file_is_a_loud_error() {
        let (_dir, path) = write_temp("this is = = not toml [[");
        match Config::load_from(&path) {
            Err(ConfigError::Parse { .. }) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_section_or_key_is_a_loud_error() {
        // #5460: a typo must not silently substitute the defaults — a
        // misspelled [domains] table would otherwise leave the domain
        // the operator meant to disable enabled.
        for contents in [
            "[domain]\nqueue = false\n",
            "[proxy]\nidle_exit_sec = 60\n",
            "[proxy]\nbroker_soket = \"/tmp/x.sock\"\n",
            "[secrets]\ntoken_fil = \"/tmp/tok\"\n",
            "domainz = 1\n",
        ] {
            let (_dir, path) = write_temp(contents);
            match Config::load_from(&path) {
                Err(ConfigError::Parse { .. }) => {}
                other => panic!("expected Parse error for {contents:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn domains_stays_an_open_map() {
        // deny_unknown_fields applies to struct fields, not to domain
        // names: any domain may be named in [domains].
        let (_dir, path) = write_temp("[domains]\nsome-future-domain = false\n");
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.domain_enabled("some-future-domain"));
    }

    #[test]
    fn services_default_to_the_live_table() {
        let cfg = Config::default();
        let services = cfg.services();
        assert_eq!(services.len(), 3);
        assert_eq!(services[0].port, PORT_LAPTOP_AGENT);
        assert_eq!(services[0].name, "laptop-agent");
        assert_eq!(services[1].port, PORT_SECRETS_BROKER);
        assert_eq!(services[2].port, PORT_QUEUE_BROKER);
        assert!(services.iter().all(|s| s.version.is_some()));
        assert_eq!(cfg.target_host(), DEFAULT_TARGET_HOST);
        assert_eq!(cfg.egress_proxy_port(), DEFAULT_EGRESS_PROXY_PORT);
        assert!(!cfg.proxy_direct());
    }

    #[test]
    fn services_come_from_config_when_declared() {
        let (_dir, path) = write_temp("[[proxy.services]]\nport = 4799\nname = \"laptop-agent\"\n");
        let cfg = Config::load_from(&path).unwrap();
        let services = cfg.services();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "laptop-agent");
    }

    #[test]
    fn env_paths_ignore_empty_values_and_resolve_under_home() {
        // #5473: an empty env value is unset, never a literal path —
        // otherwise the config/state paths go CWD-relative and a
        // planted ./fleet/fleet.toml in the working directory loads.
        let home = PathBuf::from("/home/tester");
        assert_eq!(non_empty_path(Some(PathBuf::new())), None);
        assert_eq!(
            non_empty_path(Some(PathBuf::from("/x"))),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(
            Config::file_path_from(None, None, &home),
            home.join(".config").join("fleet").join("fleet.toml")
        );
        assert_eq!(
            Config::file_path_from(None, Some(PathBuf::from("/cfg")), &home),
            PathBuf::from("/cfg").join("fleet").join("fleet.toml")
        );
        assert_eq!(
            Config::file_path_from(Some(PathBuf::from("/named.toml")), None, &home),
            PathBuf::from("/named.toml")
        );
        assert_eq!(
            Config::state_dir_from(None, &home),
            home.join(".local").join("state").join("fleet")
        );
        assert_eq!(
            Config::state_dir_from(Some(PathBuf::from("/state")), &home),
            PathBuf::from("/state").join("fleet")
        );
        // A relative FLEET_BROKER_SOCKET in the default's own spelling
        // names the same socket as the default itself.
        assert_eq!(
            Config::broker_socket_from(Some(PathBuf::from(DEFAULT_BROKER_SOCKET)), None, &home),
            Config::broker_socket_from(None, None, &home)
        );
        assert_eq!(
            Config::broker_socket_from(Some(PathBuf::from("/abs/b.sock")), None, &home),
            PathBuf::from("/abs/b.sock")
        );
        assert_eq!(
            Config::broker_socket_from(None, Some("rel/c.sock"), &home),
            home.join("rel").join("c.sock")
        );
        // #6216: the queue/secrets broker socket defaults resolve under
        // $HOME in each broker's own config dir.
        assert_eq!(
            Config::broker_socket_from(None, Some(DEFAULT_QUEUE_BROKER_SOCKET), &home),
            home.join(".config")
                .join("queue-broker")
                .join("broker.sock")
        );
        assert_eq!(
            Config::broker_socket_from(None, Some(DEFAULT_SECRETS_BROKER_SOCKET), &home),
            home.join(".config")
                .join("secrets-broker")
                .join("broker.sock")
        );
    }

    #[test]
    fn relative_xdg_bases_are_ignored() {
        // #6396: a non-empty relative XDG base is invalid per the
        // XDG spec and is treated as unset, exactly like an empty
        // one (#5473) — resolution falls back to the HOME-anchored
        // defaults, so a fleet.toml planted under the working
        // directory at ./<relative-base>/fleet/fleet.toml is never
        // loaded when HOME is set.
        let home = PathBuf::from("/home/tester");
        assert_eq!(absolute_path(Some(PathBuf::from("relcfg"))), None);
        assert_eq!(absolute_path(Some(PathBuf::new())), None);
        assert_eq!(absolute_path(None), None);
        assert_eq!(
            absolute_path(Some(PathBuf::from("/cfg"))),
            Some(PathBuf::from("/cfg"))
        );
        assert_eq!(
            Config::file_path_from(
                None,
                absolute_path(Some(PathBuf::from("relcfg"))),
                &home
            ),
            home.join(".config").join("fleet").join("fleet.toml")
        );
        assert_eq!(
            Config::file_path_from(
                None,
                absolute_path(Some(PathBuf::from("/cfg"))),
                &home
            ),
            PathBuf::from("/cfg").join("fleet").join("fleet.toml")
        );
        assert_eq!(
            Config::state_dir_from(absolute_path(Some(PathBuf::from("relstate"))), &home),
            home.join(".local").join("state").join("fleet")
        );
        assert_eq!(
            Config::state_dir_from(absolute_path(Some(PathBuf::from("/state"))), &home),
            PathBuf::from("/state").join("fleet")
        );
        // The explicit per-file override is out of scope: a relative
        // FLEET_CONFIG keeps its verbatim meaning.
        assert_eq!(
            Config::file_path_from(Some(PathBuf::from("rel.toml")), None, &home),
            PathBuf::from("rel.toml")
        );
    }

    #[test]
    fn relative_home_is_not_an_anchor() {
        // #6404: a relative HOME must never anchor resolution —
        // file_path, state_dir, and broker_socket resolve exactly as
        // they do for an unset/empty HOME, so a fleet.toml planted at
        // ./<relative-home>/.config/fleet/fleet.toml is never loaded.
        let anchor = home_anchor(Some(PathBuf::from("relhome")));
        assert_eq!(anchor, PathBuf::from("."));
        assert_eq!(home_anchor(None), PathBuf::from("."));
        assert_eq!(home_anchor(Some(PathBuf::new())), PathBuf::from("."));
        assert_eq!(
            home_anchor(Some(PathBuf::from("/home/tester"))),
            PathBuf::from("/home/tester")
        );
        assert_eq!(
            Config::file_path_from(None, None, &anchor),
            PathBuf::from(".")
                .join(".config")
                .join("fleet")
                .join("fleet.toml")
        );
        assert_eq!(
            Config::state_dir_from(None, &anchor),
            PathBuf::from(".")
                .join(".local")
                .join("state")
                .join("fleet")
        );
        assert_eq!(
            Config::broker_socket_from(None, None, &anchor),
            PathBuf::from(".").join(DEFAULT_BROKER_SOCKET)
        );
    }

    #[test]
    fn dispatch_settings_default_to_the_live_service() {
        let cfg = Config::default();
        assert_eq!(cfg.dispatch_port(), PORT_DISPATCH);
        assert_eq!(cfg.dispatch_token_name(), DEFAULT_DISPATCH_TOKEN_NAME);
        assert_eq!(cfg.dispatch_url(), None);
    }

    #[test]
    fn dispatch_settings_resolve() {
        let (_dir, path) = write_temp(
            "[dispatch]\nport = 14879\ntoken_name = \"other.token\"\nurl = \"http://127.0.0.1:9\"\n",
        );
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.dispatch_port(), 14879);
        assert_eq!(cfg.dispatch_token_name(), "other.token");
        assert_eq!(cfg.dispatch_url().as_deref(), Some("http://127.0.0.1:9"));
        // A misspelled dispatch key is a loud error like any other.
        let (_dir2, path2) = write_temp("[dispatch]\nprt = 1\n");
        match Config::load_from(&path2) {
            Err(ConfigError::Parse { .. }) => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn proxy_settings_resolve() {
        let (_dir, path) =
            write_temp("[proxy]\nidle_exit_secs = 60\nbroker_socket = \"/tmp/b.sock\"\n");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.idle_exit_secs(), 60);
        assert_eq!(cfg.broker_socket(), PathBuf::from("/tmp/b.sock"));
    }

    #[test]
    fn zero_scalar_numerics_are_a_loud_error() {
        // #6412: a configured 0 is never a value — port 0 cannot be
        // dialled and idle_exit_secs 0 exits the broker at its first
        // tick. Each zero spelling fails at load naming its field;
        // absent and valid values resolve exactly as before.
        for (contents, field) in [
            ("[proxy]\negress_proxy_port = 0\n", "egress_proxy_port"),
            ("[dispatch]\nport = 0\n", "[dispatch] port"),
            ("[proxy]\nidle_exit_secs = 0\n", "idle_exit_secs"),
        ] {
            let (_dir, path) = write_temp(contents);
            match Config::load_from(&path) {
                Err(ConfigError::Parse { message, .. }) => {
                    assert!(
                        message.contains(field),
                        "error should name {field}: {message}"
                    );
                }
                other => panic!("expected Parse error for {contents:?}, got {other:?}"),
            }
        }
        let (_dir, path) = write_temp(
            "[proxy]\negress_proxy_port = 3131\nidle_exit_secs = 60\n[dispatch]\nport = 4880\n",
        );
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.egress_proxy_port(), 3131);
        assert_eq!(cfg.idle_exit_secs(), 60);
        assert_eq!(cfg.dispatch_port(), 4880);
    }
}

/// Minimal tempdir for tests (avoids a dev-dependency for one helper).
#[cfg(test)]
mod tempfile_dir {
    use std::path::PathBuf;

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new() -> Dir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "fleet-config-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
