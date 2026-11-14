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
/// RustDesk's direct-access TCP endpoint on the laptop.
pub const PORT_RUSTDESK: u16 = 21118;
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

/// The `fleet serve` role names a `[host]` manifest may assign
/// (design #5687 §1). The role set of a manifest is exactly the key
/// set of its `[host.roles]` table; any other name is a loud parse
/// error, never silently ignored — a typo'd role that never starts
/// is exactly the failure the manifest exists to prevent.
pub const HOST_ROLES: [&str; 5] = [
    "proxy",
    "queue-broker",
    "secrets-broker",
    "search",
    "laptop-agent",
];

/// How `fleet init` ensures a location's roles (design #5687 §5):
/// `spawn` detached-spawns (the sandbox's session proxy), `systemd`
/// reports unit state and asks systemd — never a detached spawn of
/// a daemon systemd owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitMode {
    Spawn,
    Systemd,
}

impl InitMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            InitMode::Spawn => "spawn",
            InitMode::Systemd => "systemd",
        }
    }

    fn parse(value: &str) -> Option<InitMode> {
        match value {
            "spawn" => Some(InitMode::Spawn),
            "systemd" => Some(InitMode::Systemd),
            _ => None,
        }
    }
}

/// Per-role options under `[host.roles]` (design #5687 §1):
/// deliberately thin in v1 — `enabled` only. Everything else a role
/// needs already has a home (`[proxy]`, `[dispatch]`, `[search]`,
/// the `[domains]` kill switch) and must not be duplicated here.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostRoleOptions {
    /// Default true; `false` skips the role without editing the set.
    pub enabled: Option<bool>,
}

/// `[host]` — this machine's location/role manifest (design #5687
/// §1), also the grammar of the `FLEET_HOST_CONFIG` env override
/// (§2: the env value is this table's contents as inline TOML, so
/// one parser serves both). All fields are optional at parse time;
/// a section with any field set but no `location` is a loud load
/// error (see [`HostConfig::validate`]). Provisioning writes this
/// section; `fleet init` never does.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostConfig {
    /// This machine's location/role-set name (e.g. "sandbox").
    pub location: Option<String>,
    /// "spawn" | "systemd" (see [`InitMode`]).
    pub init_mode: Option<String>,
    /// Role name → per-role options. The key set IS the role set.
    #[serde(default)]
    pub roles: BTreeMap<String, HostRoleOptions>,
}

impl HostConfig {
    /// Any field set at all: a `[host]` header with only a comment
    /// under it is no manifest, but one written key is a manifest
    /// that must name its location.
    fn is_present(&self) -> bool {
        self.location.is_some() || self.init_mode.is_some() || !self.roles.is_empty()
    }

    /// The manifest's own loud rules (design #5687 §1/§2): an
    /// unknown role name, an unknown `init_mode`, and an empty
    /// location are misspelling-class errors in the crate's usual
    /// shape — a silently ignored manifest entry would defeat the
    /// manifest exactly the way a silently ignored `[domains]`
    /// disable defeats that lever (#5460).
    fn validate(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        let err = |message: String| ConfigError::Parse {
            path: path.to_path_buf(),
            message,
        };
        if let Some(location) = &self.location {
            if location.trim().is_empty() {
                return Err(err(
                    "[host] location must not be empty; omit the [host] section for no manifest"
                        .to_string(),
                ));
            }
        }
        if let Some(mode) = &self.init_mode {
            if InitMode::parse(mode).is_none() {
                return Err(err(format!(
                    "[host] init_mode {mode:?} is not a known mode; use \"spawn\" or \"systemd\""
                )));
            }
        }
        for role in self.roles.keys() {
            if !HOST_ROLES.contains(&role.as_str()) {
                return Err(err(format!(
                    "[host.roles] {role:?} is not a fleet serve role; known roles: {}",
                    HOST_ROLES.join(", ")
                )));
            }
        }
        Ok(())
    }

    /// Roles the manifest assigns and does not disable, in sorted
    /// (map) order.
    fn enabled_roles(&self) -> Vec<String> {
        self.roles
            .iter()
            .filter(|(_, opts)| opts.enabled != Some(false))
            .map(|(role, _)| role.clone())
            .collect()
    }

    /// Roles the manifest assigns but disables (`enabled = false`):
    /// reported as skipped with this reason, never silently dropped
    /// (design #5687 §4).
    fn disabled_roles(&self) -> Vec<String> {
        self.roles
            .iter()
            .filter(|(_, opts)| opts.enabled == Some(false))
            .map(|(role, _)| role.clone())
            .collect()
    }
}

/// Where a resolved host manifest came from (design #5687 §4) —
/// named in the `fleet init` report so an operator can see which
/// source is authoritative on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSource {
    /// `FLEET_HOST_CONFIG` — a complete inline manifest; wins outright.
    EnvContents,
    /// `FLEET_LOCATION` — names the location; roles came from the
    /// matching file manifest or the built-in role map.
    EnvLocation,
    /// The config file's `[host]` section.
    File,
}

impl HostSource {
    pub fn describe(&self, file_path: &std::path::Path) -> String {
        match self {
            HostSource::EnvContents => "$FLEET_HOST_CONFIG".to_string(),
            HostSource::EnvLocation => "$FLEET_LOCATION".to_string(),
            HostSource::File => format!("{} [host]", file_path.display()),
        }
    }
}

/// A location + role set after resolution (design #5687 §4):
/// exactly one source won; nothing was field-merged across sources
/// (merging is how a stale laptop file's roles would end up running
/// on the sandbox).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHost {
    pub location: String,
    pub init_mode: InitMode,
    /// Enabled roles to ensure, sorted.
    pub roles: Vec<String>,
    /// `(role, reason)` pairs the manifest itself skips
    /// (`enabled = false`); the `[domains]` gate is applied by the
    /// caller, which owns the rest of the report.
    pub skipped_roles: Vec<(String, String)>,
    pub source: HostSource,
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
    /// `[queue-broker]` — the queue broker role's local view (#6215).
    #[serde(rename = "queue-broker")]
    pub queue_broker: RoleConfig,
    /// `[secrets-broker]` — the secrets broker role's local view (#6215).
    #[serde(rename = "secrets-broker")]
    pub secrets_broker: RoleConfig,
    pub dispatch: DispatchConfig,
    #[serde(rename = "dispatch-test")]
    pub dispatch_test: DispatchTestConfig,
    pub search: SearchConfig,
    /// `[host]` — the location/role manifest (design #5687,
    /// implementation #6770). Absent means "no manifest": fleet
    /// init reports an unknown location and starts nothing.
    pub host: HostConfig,
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

/// `[<role>]` — a broker role's local transport view (#6215,
/// design #5699). Both fields are optional and both are
/// preferences between *working* routes, never a disable lever:
/// an unrecognized `transport` is ignored by the role's resolution
/// (which then falls through to the probe), exactly like
/// [`SearchConfig::via`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleConfig {
    /// Transport preference: "socket" | "tcp". Any other value is
    /// ignored by [`Config::role_transport`].
    pub transport: Option<String>,
    /// Explicit socket path for the role. Relative paths resolve
    /// under $HOME, like every other socket path here.
    pub socket: Option<String>,
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
/// spelling. Scoped to the HOME anchor only: relative XDG bases are
/// issue #6396, and relative values from explicit overrides
/// (FLEET_CONFIG, socket paths) keep their documented
/// resolve-under-home meaning.
///
/// The fallback decision (issue #5737): when `HOME` is
/// unset/empty/relative, the anchor is the account's real home
/// from the passwd database (the home OpenSSH and other tools
/// resolve `~` from) — never ".". Anchoring on "." resolved the
/// config file, the state dir, and every broker socket
/// CWD-relative, so a `fleet.toml` planted in the working
/// directory loaded for any fleet process started without `HOME`.
/// If no passwd entry with an absolute home exists for the
/// process uid, resolution fails loudly: the panic names `HOME`
/// and the fix, rather than silently anchoring on the CWD.
fn home_anchor(value: Option<PathBuf>) -> PathBuf {
    non_empty_path(value)
        .filter(|p| p.is_absolute())
        .or_else(passwd_home)
        .expect(
            "fleet-config: HOME is unset, empty, or relative and no passwd              entry with an absolute home directory exists for this process's              uid; set HOME to an absolute path (issue #5737)",
        )
}

/// The process's real uid (the first `Uid:` field of
/// `/proc/self/status`). Linux-only by construction; elsewhere
/// there is no uid to look up and the passwd fallback is absent.
fn process_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("Uid:")?;
        rest.split_whitespace().next()?.parse().ok()
    })
}

/// The home directory recorded in passwd-database text for `uid`,
/// when that home is absolute. Pure over the database text so the
/// parsing is pinnable in tests.
fn passwd_home_for_uid(passwd: &str, uid: u32) -> Option<PathBuf> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let _name = fields.next()?;
        let _passwd = fields.next()?;
        let entry_uid: u32 = fields.next()?.parse().ok()?;
        if entry_uid != uid {
            return None;
        }
        let _gid = fields.next()?;
        let _gecos = fields.next()?;
        let home = PathBuf::from(fields.next()?);
        home.is_absolute().then_some(home)
    })
}

/// This process account's real home directory from the passwd
/// database (`/etc/passwd`, matched by the process uid).
fn passwd_home() -> Option<PathBuf> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd_home_for_uid(&passwd, process_uid()?)
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

/// The transport a fleet client should use to reach a broker role
/// on this host (#6218, from the #5699 design adopted 2026-11-05):
/// the role's Unix socket, or its TCP listener. Selected by
/// [`Config::role_transport_choice`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleTransport {
    Socket(PathBuf),
    Tcp,
}

/// Probe a broker role's Unix socket: the file exists AND a connect
/// succeeds. A stale socket file (left by a dead broker) fails the
/// connect, so callers fall back to TCP within the same connection
/// attempt (#6218 acceptance).
pub fn probe_socket(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

impl Config {
    /// The config file path: `$FLEET_CONFIG`, else
    /// `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`.
    /// Fleet's home directory: `$HOME` when it is absolute and
    /// non-empty, else the passwd database's home for the process
    /// uid — never `.` (issue #5737).
    /// This is the home every Fleet path resolution uses; callers
    /// that hand paths to external tools (e.g. OpenSSH, which
    /// expands `~` from the passwd database instead) must build
    /// absolute paths from here rather than passing `~` (#6680).
    pub fn home_dir() -> PathBuf {
        home_dir()
    }

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
    /// these at load, naming the field; omitting the field keeps the
    /// documented default. There is no zero spelling for "disabled".
    /// The `[dispatch-test] threshold` (#6603) joined the rule when it
    /// landed: `active < 0` can never hold, so 0 is never a capacity
    /// bound either.
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
        // #6603: the [dispatch-test] threshold landed later (#6580)
        // and is the same misspelling class — `active < 0` is false
        // for every count, so 0 makes every execution home
        // permanently ineligible instead of meaning "no limit".
        if self.dispatch_test.threshold == Some(0) {
            return Err(reject(
                "[dispatch-test] threshold",
                DEFAULT_DISPATCH_TEST_THRESHOLD,
            ));
        }
        // Blank scalar strings (#6405): an empty or
        // whitespace-only string in a path/URL/name field is the
        // same misspelling class as the zero rule above — a
        // templated config rendering a variable that came out
        // empty. Left alone, an empty broker_socket resolves to
        // $HOME itself and an empty dispatch url surfaces as a
        // configured Some("") instead of the documented default.
        // Reject at load, naming the field; omitting the field
        // keeps the default. The configured value is never echoed
        // in the error.
        let blank = |value: &Option<String>| value.as_deref().is_some_and(|v| v.trim().is_empty());
        for (value, field) in [
            (&self.proxy.broker_socket, "[proxy] broker_socket"),
            (&self.proxy.target_host, "[proxy] target_host"),
            (&self.proxy.connect_udp_listen, "[proxy] connect_udp_listen"),
            (&self.secrets.token_file, "[secrets] token_file"),
            (&self.dispatch.url, "[dispatch] url"),
            (&self.dispatch.token_name, "[dispatch] token_name"),
        ] {
            if blank(value) {
                return Err(ConfigError::Parse {
                    path: path.to_path_buf(),
                    message: format!(
                        "{field} must not be empty or blank; omit the field for the default"
                    ),
                });
            }
        }
        // [host] (#6770, design #5687 §1): the manifest's loud
        // rules run at load like every other section's, plus one
        // structural rule of its own — a section that sets
        // anything but names no location is a manifest that
        // cannot resolve, and saying so at load beats an
        // unknown-location report at every init.
        self.host.validate(path)?;
        if self.host.is_present() && self.host.location.is_none() {
            return Err(ConfigError::Parse {
                path: path.to_path_buf(),
                message: "[host] section sets init_mode/roles but no location;                           a manifest must name its location"
                    .to_string(),
            });
        }
        // [[proxy.services]] rows (#6397): the routing table is
        // collected into a port-keyed map downstream (fleet-proxy's
        // services_map), so a duplicate port silently collapses
        // last-wins, a duplicate name makes the advertised hello map
        // ambiguous, and port 0 or an empty name can never be routed
        // to. All four are misspelling-class errors: loud at load,
        // naming the offending row. The built-in default table is
        // not validated here — only rows the operator declared.
        let mut port_rows: BTreeMap<u16, usize> = BTreeMap::new();
        let mut name_rows: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, row) in self.proxy.services.iter().enumerate() {
            let human_row = index + 1;
            if row.port == 0 {
                return Err(ConfigError::Parse {
                    path: path.to_path_buf(),
                    message: format!(
                        "[[proxy.services]] row {human_row} (name {:?}) has port 0; a service port must be between 1 and 65535",
                        row.name
                    ),
                });
            }
            if row.name.trim().is_empty() {
                return Err(ConfigError::Parse {
                    path: path.to_path_buf(),
                    message: format!(
                        "[[proxy.services]] row {human_row} (port {}) has an empty name; every routed service needs a name",
                        row.port
                    ),
                });
            }
            if let Some(first) = port_rows.insert(row.port, index) {
                return Err(ConfigError::Parse {
                    path: path.to_path_buf(),
                    message: format!(
                        "[[proxy.services]] row {human_row} (name {:?}) duplicates port {} already used by row {}; duplicate ports collapse last-wins in the routing map",
                        row.name,
                        row.port,
                        first + 1
                    ),
                });
            }
            if let Some(first) = name_rows.insert(row.name.as_str(), index) {
                return Err(ConfigError::Parse {
                    path: path.to_path_buf(),
                    message: format!(
                        "[[proxy.services]] row {human_row} duplicates service name {:?} already used by row {}",
                        row.name,
                        first + 1
                    ),
                });
            }
        }
        Ok(())
    }

    /// The built-in role map for a location (design #5687 §5):
    /// the roles a location runs when a source names the location
    /// but carries no role set of its own. Only the two locations
    /// that exist today have maps; a future lab node provisions
    /// its own `[host]` section instead of growing this table.
    pub fn builtin_host(location: &str) -> Option<(InitMode, Vec<String>)> {
        match location {
            "sandbox" => Some((InitMode::Spawn, vec!["proxy".to_string()])),
            "laptop" => Some((
                InitMode::Systemd,
                vec![
                    "laptop-agent".to_string(),
                    "queue-broker".to_string(),
                    "search".to_string(),
                    "secrets-broker".to_string(),
                ],
            )),
            _ => None,
        }
    }

    /// Resolve this machine's location + role set from the live
    /// environment (design #5687 §4). An empty env value counts as
    /// unset, per the crate's #5473 rule.
    pub fn resolve_host(&self) -> Result<Option<ResolvedHost>, ConfigError> {
        let env_contents = std::env::var("FLEET_HOST_CONFIG")
            .ok()
            .filter(|v| !v.is_empty());
        let env_location = std::env::var("FLEET_LOCATION")
            .ok()
            .filter(|v| !v.is_empty());
        self.resolve_host_from(
            env_contents.as_deref(),
            env_location.as_deref(),
            &Self::file_path(),
        )
    }

    /// Resolution, pure over the three sources so the precedence
    /// is pinnable in tests. Highest first, no field-merging
    /// across sources (design #5687 §4):
    ///
    /// 1. `FLEET_HOST_CONFIG` — a complete inline manifest; wins
    ///    outright. Malformed TOML, an unknown role, or a missing
    ///    location is a loud error, never a silent fall-through
    ///    to the file.
    /// 2. `FLEET_LOCATION` — sets the location. Roles come from
    ///    the file's `[host.roles]` only if the file's location
    ///    equals it; otherwise from the built-in map for that
    ///    location; otherwise the location is unknown.
    /// 3. The file `[host]` section.
    ///
    /// `Ok(None)` is the unknown-location case: no manifest from
    /// any source, or a location with no roles from any source.
    /// It is not an error — `fleet init` reports it and starts
    /// nothing (exit 69).
    pub fn resolve_host_from(
        &self,
        env_contents: Option<&str>,
        env_location: Option<&str>,
        file_path: &std::path::Path,
    ) -> Result<Option<ResolvedHost>, ConfigError> {
        if let Some(text) = env_contents {
            let label = PathBuf::from("$FLEET_HOST_CONFIG");
            let host: HostConfig = toml::from_str(text).map_err(|e| ConfigError::Parse {
                path: label.clone(),
                message: e.to_string(),
            })?;
            host.validate(&label)?;
            let location = host.location.clone().ok_or_else(|| ConfigError::Parse {
                path: label.clone(),
                message: "FLEET_HOST_CONFIG manifest has no location".to_string(),
            })?;
            return Ok(Self::resolved_from_manifest(
                host,
                &location,
                HostSource::EnvContents,
            ));
        }
        if let Some(location) = env_location {
            let location = location.trim();
            if location.is_empty() {
                // An all-whitespace value is the empty case of the
                // #5473 rule wearing spaces; treat it as unset and
                // fall through to the file.
                return self.resolve_host_from(None, None, file_path);
            }
            if self.host.location.as_deref() == Some(location) && self.host.is_present() {
                return Ok(Self::resolved_from_manifest(
                    self.host.clone(),
                    location,
                    HostSource::EnvLocation,
                ));
            }
            return Ok(
                Self::builtin_host(location).map(|(mode, roles)| ResolvedHost {
                    location: location.to_string(),
                    init_mode: mode,
                    roles,
                    skipped_roles: Vec::new(),
                    source: HostSource::EnvLocation,
                }),
            );
        }
        if self.host.is_present() {
            if let Some(location) = self.host.location.clone() {
                return Ok(Self::resolved_from_manifest(
                    self.host.clone(),
                    &location,
                    HostSource::File,
                ));
            }
        }
        Ok(None)
    }

    /// Build the resolution for a winning manifest. The role set
    /// is the manifest's own, exactly — no built-in supplementation
    /// (a complete manifest that assigns no enabled role resolves
    /// to no roles, which is the unknown case of §4). `init_mode`
    /// falls back to the built-in map's mode for the location,
    /// then to `spawn`, when the manifest omits it.
    fn resolved_from_manifest(
        host: HostConfig,
        location: &str,
        source: HostSource,
    ) -> Option<ResolvedHost> {
        let roles = host.enabled_roles();
        if roles.is_empty() {
            return None;
        }
        let init_mode = host
            .init_mode
            .as_deref()
            .and_then(InitMode::parse)
            .or_else(|| Self::builtin_host(location).map(|(mode, _)| mode))
            .unwrap_or(InitMode::Spawn);
        Some(ResolvedHost {
            location: location.to_string(),
            init_mode,
            roles,
            skipped_roles: host
                .disabled_roles()
                .into_iter()
                .map(|role| (role, "disabled in [host.roles]".to_string()))
                .collect(),
            source,
        })
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
    /// beats the config file's `[queue-broker] socket` beats the
    /// runtime-dir default (#6215).
    pub fn queue_broker_socket(&self) -> PathBuf {
        Self::role_socket_from(
            env_path("QUEUE_BROKER_SOCKET"),
            self.queue_broker.socket.as_deref(),
            "queue-broker",
            &home_dir(),
            env_xdg_base("XDG_RUNTIME_DIR"),
        )
    }

    /// The secrets broker's Unix socket path (#6216).
    /// `SECRETS_BROKER_SOCKET` beats the config file's
    /// `[secrets-broker] socket` beats the runtime-dir default (#6215).
    pub fn secrets_broker_socket(&self) -> PathBuf {
        Self::role_socket_from(
            env_path("SECRETS_BROKER_SOCKET"),
            self.secrets_broker.socket.as_deref(),
            "secrets-broker",
            &home_dir(),
            env_xdg_base("XDG_RUNTIME_DIR"),
        )
    }

    /// The transport preference for a broker role from
    /// `[queue-broker] transport` / `[secrets-broker] transport`
    /// ("socket" | "tcp"), when configured (#6215). `None` for an
    /// unknown role or an unrecognized value: the caller then
    /// probes, so a typo degrades to today's behavior instead of
    /// disabling a route.
    pub fn role_transport(&self, role: &str) -> Option<String> {
        let configured = match role {
            "queue-broker" => self.queue_broker.transport.as_deref(),
            "secrets-broker" => self.secrets_broker.transport.as_deref(),
            _ => None,
        };
        configured
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| v == "socket" || v == "tcp")
    }

    /// A broker role's Unix socket path by role name (#6218):
    /// `queue-broker` / `secrets-broker`; `None` for an unknown role.
    pub fn role_socket(&self, role: &str) -> Option<PathBuf> {
        match role {
            "queue-broker" => Some(self.queue_broker_socket()),
            "secrets-broker" => Some(self.secrets_broker_socket()),
            _ => None,
        }
    }

    /// The transport a client should use to reach a broker role
    /// (#6218): the `[<role>] transport` preference wins ("tcp"
    /// selects TCP without probing; "socket" selects the role's
    /// Unix socket); with no preference, probe the role socket --
    /// a socket that answers selects UDS, an absent or stale one
    /// selects TCP. `None` for an unknown role (the caller then
    /// keeps whatever route it already had).
    pub fn role_transport_choice(&self, role: &str) -> Option<RoleTransport> {
        let socket = self.role_socket(role)?;
        let probe_ok = probe_socket(&socket);
        Some(Self::role_transport_choice_from(
            self.role_transport(role).as_deref(),
            socket,
            probe_ok,
        ))
    }

    /// The pure core of [`Config::role_transport_choice`], split
    /// out so the socket-present / socket-absent / socket-stale
    /// cases are testable without a live broker: `probe_ok` is the
    /// probe's verdict (a stale socket file probes false).
    fn role_transport_choice_from(
        pref: Option<&str>,
        socket: PathBuf,
        probe_ok: bool,
    ) -> RoleTransport {
        match pref {
            Some("tcp") => RoleTransport::Tcp,
            Some("socket") => RoleTransport::Socket(socket),
            _ if probe_ok => RoleTransport::Socket(socket),
            _ => RoleTransport::Tcp,
        }
    }

    /// A role socket's resolution (#6215, design #5699): explicit
    /// env > config > `$XDG_RUNTIME_DIR/fleet/<role>.sock` when
    /// `XDG_RUNTIME_DIR` is an absolute non-empty base (tmpfs,
    /// per-login, auto-cleaned — the right home for a runtime
    /// socket) > the role's historical `$HOME`-anchored default.
    /// Explicit overrides keep their documented relative meaning
    /// (resolved under $HOME), like [`Config::broker_socket`].
    fn role_socket_from(
        env_socket: Option<PathBuf>,
        config_socket: Option<&str>,
        role: &str,
        home: &std::path::Path,
        xdg_runtime_dir: Option<PathBuf>,
    ) -> PathBuf {
        if let Some(p) = env_socket {
            return resolve_under(home, &p);
        }
        if let Some(p) = config_socket {
            return resolve_under(home, &PathBuf::from(p));
        }
        if let Some(runtime) = xdg_runtime_dir {
            return runtime.join("fleet").join(format!("{role}.sock"));
        }
        let default = match role {
            "queue-broker" => DEFAULT_QUEUE_BROKER_SOCKET,
            _ => DEFAULT_SECRETS_BROKER_SOCKET,
        };
        resolve_under(home, &PathBuf::from(default))
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
    /// roles, and 21118 RustDesk direct access (§4: a serve build
    /// without a role advertises a map without its port; the table
    /// is how that is said).
    pub fn services(&self) -> Vec<ServiceConfig> {
        if !self.proxy.services.is_empty() {
            return self.proxy.services.clone();
        }
        let fleet_version = env!("CARGO_PKG_VERSION").to_string();
        [
            (PORT_LAPTOP_AGENT, "laptop-agent", "python".to_string()),
            (PORT_SECRETS_BROKER, "secrets-broker", fleet_version.clone()),
            (PORT_QUEUE_BROKER, "queue-broker", fleet_version),
            (PORT_RUSTDESK, "rustdesk", "rustdesk-direct-v1".to_string()),
        ]
        .into_iter()
        .map(|(port, name, version)| ServiceConfig {
            port,
            name: name.to_string(),
            version: Some(version),
        })
        .collect()
    }

    /// Port-to-socket resolution beside [`Config::services`]
    /// (#6219, from the #5699 design): the Unix socket a colocated
    /// caller dials for the service routed on `port`, when that
    /// service is a broker role with a socket. `None` when the
    /// port is unrouted or its service has no socket -- the caller
    /// keeps its TCP route. The hello/services wire contract is
    /// untouched: this is a local dial decision only, derived
    /// from the same port-keyed table `services()` returns.
    pub fn service_socket(&self, port: u16) -> Option<PathBuf> {
        let name = self.services().into_iter().find(|s| s.port == port)?.name;
        self.role_socket(&name)
    }

    /// The transport choice for the service routed on `port`
    /// (#6219): port -> service name (via [`Config::services`])
    /// -> [`Config::role_transport_choice`]. `None` when the port
    /// is unrouted or its service is not a broker role (the
    /// caller keeps whatever route it already had).
    pub fn service_transport_choice(&self, port: u16) -> Option<RoleTransport> {
        let name = self.services().into_iter().find(|s| s.port == port)?.name;
        self.role_transport_choice(&name)
    }

    /// The socket and configured transport preference for the
    /// service routed on `port` (#6219), WITHOUT probing: the
    /// dial itself is the probe on the proxy's dial path, and it
    /// must stay inside the caller's setup deadline (a probe
    /// connect here could block unboundedly on a wedged listener
    /// whose backlog is full). `None` when the port is unrouted
    /// or its service has no socket.
    pub fn service_socket_resolution(&self, port: u16) -> Option<(PathBuf, Option<String>)> {
        let name = self.services().into_iter().find(|s| s.port == port)?.name;
        let socket = self.role_socket(&name)?;
        Some((socket, self.role_transport(&name)))
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
        assert_eq!(services.len(), 4);
        assert_eq!(services[0].port, PORT_LAPTOP_AGENT);
        assert_eq!(services[0].name, "laptop-agent");
        assert_eq!(services[1].port, PORT_SECRETS_BROKER);
        assert_eq!(services[2].port, PORT_QUEUE_BROKER);
        assert_eq!(services[3].port, PORT_RUSTDESK);
        assert_eq!(services[3].name, "rustdesk");
        assert!(services.iter().all(|s| s.version.is_some()));
        assert_eq!(cfg.target_host(), DEFAULT_TARGET_HOST);
        assert_eq!(cfg.egress_proxy_port(), DEFAULT_EGRESS_PROXY_PORT);
        assert!(!cfg.proxy_direct());
    }

    #[test]
    fn service_socket_resolves_broker_ports_only() {
        // #6219: the port-to-socket map beside services() resolves
        // the two broker-role ports to their role sockets; the
        // laptop-agent port (no role socket) and unrouted ports
        // resolve to None, so their callers keep TCP.
        let cfg = Config::default();
        assert_eq!(
            cfg.service_socket(PORT_QUEUE_BROKER),
            Some(cfg.queue_broker_socket())
        );
        assert_eq!(
            cfg.service_socket(PORT_SECRETS_BROKER),
            Some(cfg.secrets_broker_socket())
        );
        assert_eq!(cfg.service_socket(PORT_LAPTOP_AGENT), None);
        assert_eq!(cfg.service_socket(9999), None);
        // The transport choice follows the same mapping: a broker
        // port yields a choice, an unrouted port yields None.
        assert!(cfg.service_transport_choice(PORT_QUEUE_BROKER).is_some());
        assert!(cfg.service_transport_choice(PORT_LAPTOP_AGENT).is_none());
        assert!(cfg.service_transport_choice(9999).is_none());
        // The probe-free resolution pairs each broker port with
        // its socket; unrouted / socket-less ports give None.
        let (sock, _pref) = cfg.service_socket_resolution(PORT_QUEUE_BROKER).unwrap();
        assert_eq!(sock, cfg.queue_broker_socket());
        assert!(cfg.service_socket_resolution(PORT_LAPTOP_AGENT).is_none());
        assert!(cfg.service_socket_resolution(9999).is_none());
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
    fn services_row_with_port_zero_is_a_loud_error() {
        let (_dir, path) = write_temp("[[proxy.services]]\nport = 0\nname = \"laptop-agent\"\n");
        match Config::load_from(&path) {
            Err(ConfigError::Parse { message, .. }) => {
                assert!(message.contains("port 0"), "{message}");
                assert!(message.contains("laptop-agent"), "{message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn services_row_with_empty_name_is_a_loud_error() {
        let (_dir, path) = write_temp("[[proxy.services]]\nport = 4799\nname = \"\"\n");
        match Config::load_from(&path) {
            Err(ConfigError::Parse { message, .. }) => {
                assert!(message.contains("empty name"), "{message}");
                assert!(message.contains("4799"), "{message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn services_rows_with_duplicate_port_are_a_loud_error() {
        // The triage probe's impostor case (#6397): two rows on one
        // port used to load fine and collapse last-wins downstream.
        let (_dir, path) = write_temp(
            "[[proxy.services]]\nport = 4801\nname = \"queue-broker\"\n[[proxy.services]]\nport = 4801\nname = \"impostor\"\n",
        );
        match Config::load_from(&path) {
            Err(ConfigError::Parse { message, .. }) => {
                assert!(message.contains("4801"), "{message}");
                assert!(message.contains("impostor"), "{message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn services_rows_with_duplicate_name_are_a_loud_error() {
        let (_dir, path) = write_temp(
            "[[proxy.services]]\nport = 4800\nname = \"secrets-broker\"\n[[proxy.services]]\nport = 4801\nname = \"secrets-broker\"\n",
        );
        match Config::load_from(&path) {
            Err(ConfigError::Parse { message, .. }) => {
                assert!(message.contains("secrets-broker"), "{message}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn services_rows_valid_table_loads_verbatim() {
        let (_dir, path) = write_temp(
            "[[proxy.services]]\nport = 4799\nname = \"laptop-agent\"\n[[proxy.services]]\nport = 4801\nname = \"queue-broker\"\n",
        );
        let cfg = Config::load_from(&path).unwrap();
        let services = cfg.services();
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].name, "laptop-agent");
        assert_eq!(services[1].port, 4801);
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// PBT (#6397, testing doctrine): load acceptance for a
        /// generated [[proxy.services]] table is decided by an
        /// independent predicate over the four invariants — every
        /// port nonzero, every name nonblank after trimming, ports
        /// unique, names unique — never by the loader's own say-so.
        /// The generators draw ports and names from small pools so
        /// collisions, zeroes, and blank/whitespace names occur in
        /// arbitrary row orders; a table the predicate accepts must
        /// load verbatim, and one it rejects must fail with a Parse
        /// error.
        #[test]
        fn services_tables_load_exactly_when_the_invariants_hold(
            rows in prop::collection::vec(
                (
                    prop_oneof![
                        Just(0_u16),
                        Just(4799_u16),
                        Just(4800_u16),
                        Just(4801_u16),
                        any::<u16>(),
                    ],
                    prop_oneof![
                        Just(String::new()),
                        Just(" ".to_string()),
                        Just("laptop-agent".to_string()),
                        Just("queue-broker".to_string()),
                        "[a-z]{1,6}",
                    ],
                ),
                0..6,
            )
        ) {
            let mut text = String::new();
            for (port, name) in &rows {
                text.push_str(&format!(
                    "[[proxy.services]]\nport = {port}\nname = \"{name}\"\n"
                ));
            }
            let valid = rows.iter().all(|(port, name)| *port != 0 && !name.trim().is_empty())
                && rows.iter().map(|(port, _)| *port).collect::<std::collections::HashSet<_>>().len() == rows.len()
                && rows.iter().map(|(_, name)| name.as_str()).collect::<std::collections::HashSet<_>>().len() == rows.len();
            let (_dir, path) = write_temp(&text);
            match (valid, Config::load_from(&path)) {
                (true, Ok(cfg)) => {
                    let services = cfg.services();
                    if rows.is_empty() {
                        // No declared rows is the documented "empty
                        // means the built-in table" case (see the
                        // services field doc), not a verbatim load.
                        prop_assert_eq!(services.len(), Config::default().services().len());
                    } else {
                        prop_assert_eq!(services.len(), rows.len());
                        for (service, (port, name)) in services.iter().zip(&rows) {
                            prop_assert_eq!(service.port, *port);
                            prop_assert_eq!(&service.name, name);
                        }
                    }
                }
                (true, Err(e)) => {
                    prop_assert!(false, "predicate-valid table rejected: {e}");
                }
                (false, Err(ConfigError::Parse { .. })) => {}
                (false, other) => {
                    prop_assert!(false, "predicate-invalid table not rejected with Parse: {other:?}");
                }
            }
        }
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
    fn role_sockets_prefer_the_runtime_dir_then_the_home_default() {
        // #6215 (design #5699): env > config > $XDG_RUNTIME_DIR/fleet/
        // <role>.sock > the role's historical $HOME default. Oracle:
        // the documented precedence, checked at every rung.
        let home = PathBuf::from("/home/tester");
        let runtime = Some(PathBuf::from("/run/user/1000"));
        let queue_default = home.join(DEFAULT_QUEUE_BROKER_SOCKET);
        let secrets_default = home.join(DEFAULT_SECRETS_BROKER_SOCKET);

        // No override, no runtime dir: today's HOME-anchored default.
        assert_eq!(
            Config::role_socket_from(None, None, "queue-broker", &home, None),
            queue_default
        );
        assert_eq!(
            Config::role_socket_from(None, None, "secrets-broker", &home, None),
            secrets_default
        );

        // A runtime dir wins over the default, per role name.
        assert_eq!(
            Config::role_socket_from(None, None, "queue-broker", &home, runtime.clone()),
            PathBuf::from("/run/user/1000/fleet/queue-broker.sock")
        );
        assert_eq!(
            Config::role_socket_from(None, None, "secrets-broker", &home, runtime.clone()),
            PathBuf::from("/run/user/1000/fleet/secrets-broker.sock")
        );

        // The config field wins over the runtime dir; a relative
        // config path resolves under $HOME like every other socket.
        assert_eq!(
            Config::role_socket_from(
                None,
                Some("rel/q.sock"),
                "queue-broker",
                &home,
                runtime.clone()
            ),
            home.join("rel").join("q.sock")
        );

        // The env var wins over everything.
        assert_eq!(
            Config::role_socket_from(
                Some(PathBuf::from("/abs/env.sock")),
                Some("/abs/config.sock"),
                "queue-broker",
                &home,
                runtime
            ),
            PathBuf::from("/abs/env.sock")
        );
    }

    #[test]
    fn role_transport_parses_only_the_two_working_routes() {
        // #6215: a preference between working routes, not a disable
        // lever — an unrecognized value is None (the caller probes).
        let cfg: Config = toml::from_str(
            r#"
[queue-broker]
transport = " Socket "
[secrets-broker]
transport = "udp"
"#,
        )
        .expect("parse");
        assert_eq!(
            cfg.role_transport("queue-broker").as_deref(),
            Some("socket")
        );
        assert_eq!(cfg.role_transport("secrets-broker"), None);
        assert_eq!(cfg.role_transport("dispatch"), None);
        assert_eq!(Config::default().role_transport("queue-broker"), None);
    }

    #[test]
    fn role_transport_choice_pref_wins_then_probe() {
        // #6218: explicit preference beats the probe; with no
        // preference the probe decides (present -> socket, absent
        // or stale -> TCP).
        let sock = PathBuf::from("/tmp/role.sock");
        assert_eq!(
            Config::role_transport_choice_from(Some("tcp"), sock.clone(), true),
            RoleTransport::Tcp
        );
        assert_eq!(
            Config::role_transport_choice_from(Some("socket"), sock.clone(), false),
            RoleTransport::Socket(sock.clone())
        );
        assert_eq!(
            Config::role_transport_choice_from(None, sock.clone(), true),
            RoleTransport::Socket(sock.clone())
        );
        assert_eq!(
            Config::role_transport_choice_from(None, sock.clone(), false),
            RoleTransport::Tcp
        );
        // An unrecognized preference degrades to the probe, exactly
        // like role_transport's None.
        assert_eq!(
            Config::role_transport_choice_from(Some("udp"), sock, false),
            RoleTransport::Tcp
        );
    }

    #[test]
    fn probe_socket_distinguishes_live_absent_and_stale() {
        // #6218 acceptance: socket-present, socket-absent, and
        // socket-stale probe outcomes.
        let dir =
            std::env::temp_dir().join(format!("fleet-config-probe-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let live = dir.join("live.sock");
        let stale = dir.join("stale.sock");
        let absent = dir.join("absent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&live).expect("bind live socket");
        assert!(probe_socket(&live), "live socket must probe true");
        drop(listener);
        // The socket file outlives its listener: stale.
        assert!(live.exists(), "socket file remains after drop");
        assert!(!probe_socket(&live), "stale socket must probe false");
        std::fs::write(&stale, b"not a socket").expect("write stale placeholder");
        assert!(!probe_socket(&stale), "regular file must probe false");
        assert!(!probe_socket(&absent), "absent socket must probe false");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn role_socket_dispatches_known_roles_only() {
        let cfg = Config::default();
        assert_eq!(
            cfg.role_socket("queue-broker"),
            Some(cfg.queue_broker_socket())
        );
        assert_eq!(
            cfg.role_socket("secrets-broker"),
            Some(cfg.secrets_broker_socket())
        );
        assert_eq!(cfg.role_socket("dispatch"), None);
        assert_eq!(cfg.role_transport_choice("dispatch"), None);
    }

    #[test]
    fn role_config_rejects_unknown_fields() {
        // #6215: deny-unknown-fields discipline holds for the new
        // role tables, so a typo is a parse error, not silence.
        let err = toml::from_str::<Config>(
            r#"
[queue-broker]
transports = "socket"
"#,
        )
        .expect_err("unknown role field must not parse");
        assert!(err.to_string().contains("transports"), "{err}");
    }

    #[test]
    fn role_socket_overrides_read_from_the_config_file() {
        // #6215: the public accessors wire env/config/runtime together;
        // the config-file source is checked here (the env source is
        // covered by role_sockets_prefer_the_runtime_dir_then_the_home_default).
        let cfg: Config = toml::from_str(
            r#"
[queue-broker]
socket = "/abs/q.sock"
[secrets-broker]
socket = "rel/s.sock"
"#,
        )
        .expect("parse");
        assert_eq!(
            Config::role_socket_from(
                None,
                cfg.queue_broker.socket.as_deref(),
                "queue-broker",
                &PathBuf::from("/home/tester"),
                Some(PathBuf::from("/run/user/1000"))
            ),
            PathBuf::from("/abs/q.sock")
        );
        assert_eq!(
            Config::role_socket_from(
                None,
                cfg.secrets_broker.socket.as_deref(),
                "secrets-broker",
                &PathBuf::from("/home/tester"),
                Some(PathBuf::from("/run/user/1000"))
            ),
            PathBuf::from("/home/tester").join("rel").join("s.sock")
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
            Config::file_path_from(None, absolute_path(Some(PathBuf::from("relcfg"))), &home),
            home.join(".config").join("fleet").join("fleet.toml")
        );
        assert_eq!(
            Config::file_path_from(None, absolute_path(Some(PathBuf::from("/cfg"))), &home),
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
        // #5737: the unset/empty/relative spellings share the
        // passwd-home fallback — an absolute anchor, never ".".
        assert_eq!(anchor, home_anchor(None));
        assert_eq!(anchor, home_anchor(Some(PathBuf::new())));
        assert!(anchor.is_absolute());
        assert_ne!(anchor, PathBuf::from("."));
        assert_eq!(
            home_anchor(Some(PathBuf::from("/home/tester"))),
            PathBuf::from("/home/tester")
        );
        assert_eq!(
            Config::file_path_from(None, None, &anchor),
            anchor.join(".config").join("fleet").join("fleet.toml")
        );
        assert_eq!(
            Config::state_dir_from(None, &anchor),
            anchor.join(".local").join("state").join("fleet")
        );
        assert_eq!(
            Config::broker_socket_from(None, None, &anchor),
            anchor.join(DEFAULT_BROKER_SOCKET)
        );
    }

    #[test]
    fn passwd_home_for_uid_reads_the_matching_entry() {
        let passwd = "root:x:0:0:root:/root:/bin/sh\n\
                      tester:x:1000:1000:Tester:/home/tester:/bin/sh\n\
                      other:x:1001:1001:Other:relhome:/bin/sh\n";
        assert_eq!(
            passwd_home_for_uid(passwd, 1000),
            Some(PathBuf::from("/home/tester"))
        );
        assert_eq!(passwd_home_for_uid(passwd, 0), Some(PathBuf::from("/root")));
        // Unknown uid, and a uid whose recorded home is relative,
        // both yield no passwd home (the loud-failure path).
        assert_eq!(passwd_home_for_uid(passwd, 4242), None);
        assert_eq!(passwd_home_for_uid(passwd, 1001), None);
        assert_eq!(passwd_home_for_uid("", 1000), None);
    }

    #[test]
    fn unset_or_empty_home_never_anchors_on_the_cwd() {
        // #5737 pinning: for HOME unset and HOME empty, file_path,
        // state_dir, and broker_socket all resolve under the
        // passwd-home anchor — absolute paths that a planted
        // ./.config/fleet/fleet.toml in the working directory can
        // never shadow.
        for anchor in [home_anchor(None), home_anchor(Some(PathBuf::new()))] {
            assert!(anchor.is_absolute());
            let file_path = Config::file_path_from(None, None, &anchor);
            assert!(file_path.is_absolute());
            assert_eq!(
                file_path,
                anchor.join(".config").join("fleet").join("fleet.toml")
            );
            assert_ne!(
                file_path,
                PathBuf::from(".config").join("fleet").join("fleet.toml")
            );
            let state_dir = Config::state_dir_from(None, &anchor);
            assert!(state_dir.is_absolute());
            assert_eq!(state_dir, anchor.join(".local").join("state").join("fleet"));
            let socket = Config::broker_socket_from(None, None, &anchor);
            assert!(socket.is_absolute());
            assert_eq!(socket, anchor.join(DEFAULT_BROKER_SOCKET));
        }
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
            (
                "[dispatch-test]\nthreshold = 0\n",
                "[dispatch-test] threshold",
            ),
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
        // #6603: an omitted dispatch-test threshold keeps the
        // documented default (40) and a positive value passes
        // unchanged.
        assert_eq!(
            cfg.dispatch_test_threshold(),
            DEFAULT_DISPATCH_TEST_THRESHOLD
        );
        let (_dir3, path3) = write_temp("[dispatch-test]\nthreshold = 7\n");
        let cfg = Config::load_from(&path3).unwrap();
        assert_eq!(cfg.dispatch_test_threshold(), 7);
    }
    #[test]
    fn blank_scalar_strings_are_a_loud_error() {
        // #6405: an empty or whitespace-only string in a scalar
        // path/URL/name field fails at load naming its field
        // (oracle: the Parse message contains the field name for
        // both the empty and the whitespace spelling of each of
        // the six fields). Nonblank values surface unchanged and
        // omitted fields keep their documented defaults.
        for (contents, field) in [
            ("[proxy]\nbroker_socket = \"\"\n", "[proxy] broker_socket"),
            (
                "[proxy]\nbroker_socket = \"   \"\n",
                "[proxy] broker_socket",
            ),
            ("[proxy]\ntarget_host = \"\"\n", "[proxy] target_host"),
            ("[proxy]\ntarget_host = \"\t\"\n", "[proxy] target_host"),
            (
                "[proxy]\nconnect_udp_listen = \"\"\n",
                "[proxy] connect_udp_listen",
            ),
            (
                "[proxy]\nconnect_udp_listen = \"  \"\n",
                "[proxy] connect_udp_listen",
            ),
            ("[secrets]\ntoken_file = \"\"\n", "[secrets] token_file"),
            ("[secrets]\ntoken_file = \"  \"\n", "[secrets] token_file"),
            ("[dispatch]\nurl = \"\"\n", "[dispatch] url"),
            ("[dispatch]\nurl = \"  \"\n", "[dispatch] url"),
            ("[dispatch]\ntoken_name = \"\"\n", "[dispatch] token_name"),
            ("[dispatch]\ntoken_name = \"  \"\n", "[dispatch] token_name"),
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
        // Valid control: every field set to a nonblank value loads
        // and surfaces exactly that value.
        let (_dir, path) = write_temp(
            "[proxy]\nbroker_socket = \"/run/fleet.sock\"\ntarget_host = \"host.example\"\nconnect_udp_listen = \"127.0.0.1:1080\"\n[secrets]\ntoken_file = \"/run/token\"\n[dispatch]\nurl = \"http://host:4880\"\ntoken_name = \"custom-token\"\n",
        );
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.broker_socket(), PathBuf::from("/run/fleet.sock"));
        assert_eq!(cfg.target_host(), "host.example");
        assert_eq!(cfg.connect_udp_listen(), "127.0.0.1:1080");
        assert_eq!(cfg.secrets_token_file(), PathBuf::from("/run/token"));
        assert_eq!(cfg.dispatch_url(), Some("http://host:4880".to_string()));
        assert_eq!(cfg.dispatch_token_name(), "custom-token");
        // Omitted control: an empty file keeps every default.
        let (_dir, path) = write_temp("");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.target_host(), DEFAULT_TARGET_HOST);
        assert_eq!(cfg.connect_udp_listen(), DEFAULT_CONNECT_UDP_LISTEN);
        assert_eq!(cfg.dispatch_url(), None);
        assert_eq!(cfg.dispatch_token_name(), DEFAULT_DISPATCH_TOKEN_NAME);
    }
    // ---- [host] manifest + location resolution (#6770, #5687) ----

    fn cfg_from(contents: &str) -> (tempfile_dir::Dir, PathBuf, Config) {
        let (dir, path) = write_temp(contents);
        let cfg = Config::load_from(&path).unwrap();
        (dir, path, cfg)
    }

    #[test]
    fn host_file_manifest_resolves_with_its_roles() {
        let (_d, path, cfg) = cfg_from(
            "[host]\nlocation = \"sandbox\"\ninit_mode = \"spawn\"\n\n[host.roles]\nproxy = {}\n",
        );
        let resolved = cfg
            .resolve_host_from(None, None, &path)
            .unwrap()
            .expect("a file manifest resolves");
        assert_eq!(resolved.location, "sandbox");
        assert_eq!(resolved.init_mode, InitMode::Spawn);
        assert_eq!(resolved.roles, vec!["proxy".to_string()]);
        assert_eq!(resolved.source, HostSource::File);
    }

    #[test]
    fn host_no_manifest_resolves_to_unknown() {
        let cfg = Config::default();
        let path = PathBuf::from("/nonexistent/fleet.toml");
        assert_eq!(cfg.resolve_host_from(None, None, &path).unwrap(), None);
        // A bare [host] header (nothing set) is no manifest either.
        let (_d, path, cfg) = cfg_from("[host]\n");
        assert_eq!(cfg.resolve_host_from(None, None, &path).unwrap(), None);
    }

    #[test]
    fn host_unknown_role_is_a_loud_load_error() {
        let (_d, path) = write_temp("[host]\nlocation = \"sandbox\"\n\n[host.roles]\nprox = {}\n");
        let err = Config::load_from(&path).expect_err("a typo'd role must not load");
        assert!(err.to_string().contains("prox"), "{err}");
        assert!(err.to_string().contains("not a fleet serve role"), "{err}");
    }

    #[test]
    fn host_bad_init_mode_is_a_loud_load_error() {
        let (_d, path) = write_temp("[host]\nlocation = \"sandbox\"\ninit_mode = \"spwan\"\n");
        let err = Config::load_from(&path).expect_err("a typo'd mode must not load");
        assert!(err.to_string().contains("spwan"), "{err}");
    }

    #[test]
    fn host_section_without_location_is_a_loud_load_error() {
        let (_d, path) = write_temp("[host]\ninit_mode = \"spawn\"\n");
        let err = Config::load_from(&path).expect_err("a manifest must name its location");
        assert!(err.to_string().contains("no location"), "{err}");
    }

    #[test]
    fn host_disabled_role_is_skipped_not_active() {
        let (_d, path, cfg) = cfg_from(
            "[host]\nlocation = \"laptop\"\ninit_mode = \"systemd\"\n\n[host.roles]\nqueue-broker = {}\nsecrets-broker = { enabled = false }\n",
        );
        let resolved = cfg.resolve_host_from(None, None, &path).unwrap().unwrap();
        assert_eq!(resolved.roles, vec!["queue-broker".to_string()]);
        assert_eq!(
            resolved.skipped_roles,
            vec![(
                "secrets-broker".to_string(),
                "disabled in [host.roles]".to_string()
            )]
        );
    }

    #[test]
    fn host_env_contents_wins_outright_over_file_and_location() {
        let (_d, path, cfg) =
            cfg_from("[host]\nlocation = \"sandbox\"\n\n[host.roles]\nproxy = {}\n");
        let env = "location=\"laptop\"\ninit_mode=\"systemd\"\n[roles]\nqueue-broker={}";
        let resolved = cfg
            .resolve_host_from(Some(env), Some("sandbox"), &path)
            .unwrap()
            .expect("the env manifest resolves");
        assert_eq!(resolved.location, "laptop");
        assert_eq!(resolved.init_mode, InitMode::Systemd);
        assert_eq!(resolved.roles, vec!["queue-broker".to_string()]);
        assert_eq!(resolved.source, HostSource::EnvContents);
    }

    #[test]
    fn host_malformed_env_contents_is_loud_never_a_fallthrough() {
        let (_d, path, cfg) =
            cfg_from("[host]\nlocation = \"sandbox\"\n\n[host.roles]\nproxy = {}\n");
        let err = cfg
            .resolve_host_from(Some("location = ["), None, &path)
            .expect_err("malformed env TOML is an error, not a fall-through to the file");
        assert!(err.to_string().contains("FLEET_HOST_CONFIG"), "{err}");
        // An env manifest naming an unknown role is loud the same way.
        let err = cfg
            .resolve_host_from(Some("location=\"sandbox\"\n[roles]\nprox={}"), None, &path)
            .expect_err("an unknown env role is an error");
        assert!(err.to_string().contains("not a fleet serve role"), "{err}");
        // And one with no location at all.
        let err = cfg
            .resolve_host_from(Some("[roles]\nproxy={}"), None, &path)
            .expect_err("an env manifest without a location is an error");
        assert!(err.to_string().contains("no location"), "{err}");
    }

    #[test]
    fn host_env_location_uses_file_roles_only_on_a_match() {
        let (_d, path, cfg) = cfg_from(
            "[host]\nlocation = \"sandbox\"\ninit_mode = \"spawn\"\n\n[host.roles]\nproxy = {}\nsearch = {}\n",
        );
        // Match: the file's role set (not the built-in sandbox set,
        // which is proxy-only) wins, sourced to the env selection.
        let resolved = cfg
            .resolve_host_from(None, Some("sandbox"), &path)
            .unwrap()
            .unwrap();
        assert_eq!(
            resolved.roles,
            vec!["proxy".to_string(), "search".to_string()]
        );
        assert_eq!(resolved.source, HostSource::EnvLocation);
        // Mismatch: the file's roles do NOT leak across locations;
        // the built-in laptop map supplies the set instead.
        let resolved = cfg
            .resolve_host_from(None, Some("laptop"), &path)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.init_mode, InitMode::Systemd);
        assert!(resolved.roles.contains(&"queue-broker".to_string()));
        assert!(!resolved.roles.contains(&"proxy".to_string()));
        // No built-in map and no file match: unknown, not an error.
        assert_eq!(
            cfg.resolve_host_from(None, Some("lab-99"), &path).unwrap(),
            None
        );
    }

    #[test]
    fn host_env_location_sandbox_builtin_map_is_proxy_spawn() {
        // The hermetic-provisioning shape: FLEET_LOCATION alone on
        // a machine with no [host] section resolves to exactly
        // today's sandbox behaviour (design #5687 §5/§6).
        let cfg = Config::default();
        let path = PathBuf::from("/nonexistent/fleet.toml");
        let resolved = cfg
            .resolve_host_from(None, Some("sandbox"), &path)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.init_mode, InitMode::Spawn);
        assert_eq!(resolved.roles, vec!["proxy".to_string()]);
    }

    #[test]
    fn host_manifest_with_no_enabled_roles_is_unknown() {
        // §4: a location with no roles from any source is unknown —
        // a manifest whose only role is disabled resolves to None,
        // so init reports and starts nothing rather than succeeding
        // vacuously.
        let (_d, path, cfg) = cfg_from(
            "[host]\nlocation = \"sandbox\"\n\n[host.roles]\nproxy = { enabled = false }\n",
        );
        assert_eq!(cfg.resolve_host_from(None, None, &path).unwrap(), None);
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
