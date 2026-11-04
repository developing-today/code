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
/// The proxy's per-port routing table (§4): port → service name.
pub const PORT_LAPTOP_AGENT: u16 = 4799;
pub const PORT_SECRETS_BROKER: u16 = 4800;
pub const PORT_QUEUE_BROKER: u16 = 4801;
/// Broker idle exit (§4: retained from the Python broker — a warmth
/// cache that exits when cold is a feature on a shared sandbox).
pub const DEFAULT_IDLE_EXIT_SECS: u64 = 3600;
/// The token file whose contents the session broker sends as
/// `X-Agent-Token` for secrets-broker requests. fleet only ever names
/// this file; the broker reads it per request (§1: token files stay
/// files, read per request, never stored, never logged).
pub const DEFAULT_SECRETS_TOKEN_FILE: &str = ".config/secrets-broker/token";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `[domains]` — domain name → enabled. Absent means enabled.
    pub domains: BTreeMap<String, bool>,
    pub proxy: ProxyConfig,
    pub secrets: SecretsConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyConfig {
    /// Session broker socket path (relative paths resolve under $HOME).
    pub broker_socket: Option<String>,
    pub idle_exit_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecretsConfig {
    /// Path of the secrets-broker token file (handed to the session
    /// broker per request; fleet never opens it).
    pub token_file: Option<String>,
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

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_under_home(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        home_dir().join(p)
    }
}

impl Config {
    /// The config file path: `$FLEET_CONFIG`, else
    /// `${XDG_CONFIG_HOME:-~/.config}/fleet/fleet.toml`.
    pub fn file_path() -> PathBuf {
        if let Some(p) = std::env::var_os("FLEET_CONFIG") {
            return PathBuf::from(p);
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"));
        base.join("fleet").join("fleet.toml")
    }

    /// Load configuration. A missing file yields defaults; a malformed
    /// or unreadable file is an error (see the crate docs for why).
    pub fn load() -> Result<Config, ConfigError> {
        let path = Self::file_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Result<Config, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| ConfigError::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
        }
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
        if let Some(p) = std::env::var_os("FLEET_BROKER_SOCKET") {
            return PathBuf::from(p);
        }
        match &self.proxy.broker_socket {
            Some(p) => resolve_under_home(p),
            None => resolve_under_home(DEFAULT_BROKER_SOCKET),
        }
    }

    pub fn idle_exit_secs(&self) -> u64 {
        self.proxy.idle_exit_secs.unwrap_or(DEFAULT_IDLE_EXIT_SECS)
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
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".local").join("state"));
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
    fn domains_table_disables() {
        let (_dir, path) = write_temp("[domains]\nqueue = false\n");
        let cfg = Config::load_from(&path).unwrap();
        assert!(!cfg.domain_enabled("queue"));
        assert!(cfg.domain_enabled("search"));
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
    fn proxy_settings_resolve() {
        let (_dir, path) =
            write_temp("[proxy]\nidle_exit_secs = 60\nbroker_socket = \"/tmp/b.sock\"\n");
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg.idle_exit_secs(), 60);
        assert_eq!(cfg.broker_socket(), PathBuf::from("/tmp/b.sock"));
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
