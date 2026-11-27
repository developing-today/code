//! Fleet placement dispatch and sessions — the absorption of the
//! `fleet-place` reference implementation into repo-queue
//! (developing-today/code#5413; frozen spec: `docs/placement.md`,
//! intake copy at `doc/placement-intake/placement.md`).
//!
//! Placement decides WHERE a build job runs across the fleet's
//! GitHub Actions copies and runner providers, and dispatches it
//! there AS THE developing-today GitHub App. The surface registry
//! (`pkgs/fleet/fleet-surfaces.json` in this repo — the same format
//! as the fleet umbrella's `tools/fleet-surfaces.json`) is data —
//! adding a location is
//! adding an entry; no surface is hardcoded here. Each registry
//! `kind` is served by one adapter implementing
//! eligible/capacity/dispatch/status; a new placement type is a new
//! adapter plus a `kind` value, with the cascade and session
//! handling untouched.
//!
//! Sessions are server-side records keyed by an unguessable handle
//! (`fp_` + 128-bit random), so any client can reconnect:
//! `repo-queue session status|result|list`. Records live in the
//! local JSON store at `~/.local/state/fleet-place/sessions.json`
//! — the same path and record shape the reference implementation
//! used — and mirror best-effort to the queue backend's
//! `placement_sessions` table (#13146; the designed home the intake
//! spec pointed at), so a handle resolves from any seat. The local
//! store remains the always-available fallback and the only input
//! to placement decisions: the mirror can neither fail nor stall a
//! dispatch, and `FLEET_PLACE_REMOTE=off` disables it.
//!
//! Client interface (frozen v1):
//!   repo-queue dispatch --repo <owner/name> --command '<cmd>'
//!       [--ref SHA] [--runner auto|github|blacksmith]
//!       [--surface ID] [--push-nix-cache] [--actions-cache KEY:PATH]
//!       [--allow-paid-blacksmith MINUTES] [--laptop-busy]
//!       [--dispatch] [--json] [--registry PATH]
//!     Dry-run unless --dispatch. Prints the decision and, when a
//!     session is written, its handle.
//!   repo-queue session status HANDLE | result HANDLE |
//!              list [--mine] [--json]
//!   repo-queue surfaces [--registry PATH]
//!
//! Testing overrides (from the reference CLI, labeled as simulation
//! in the output): --assume-owner-load owner=N[,owner=N...] and
//! --assume-blacksmith-used MINUTES.

use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ------------------------------------------------------------------ errors

/// A user-facing fatal error; the command prints it and exits 2,
/// mirroring the reference's die().
#[derive(Debug)]
struct Die(String);

type R<T> = Result<T, Die>;

fn die<T>(msg: impl Into<String>) -> R<T> {
    Err(Die(msg.into()))
}

// -------------------------------------------------------------------- time

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn iso_from_epoch(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_iso() -> String {
    iso_from_epoch(now_epoch())
}

/// Parse the GitHub timestamp shape `2026-11-04T12:00:00Z` to epoch
/// seconds; None when the shape does not match.
fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |a: usize, n: usize| s.get(a..a + n)?.parse::<i64>().ok();
    let days = days_from_civil(num(0, 4)?, num(5, 2)? as u32, num(8, 2)? as u32);
    Some(days * 86_400 + num(11, 2)? * 3600 + num(14, 2)? * 60 + num(17, 2)?)
}

fn current_month() -> String {
    now_iso()[..7].to_string()
}

// --------------------------------------------------------------------- job

#[derive(Debug, Clone)]
pub struct Job {
    pub repo: String,
    pub command: String,
    pub git_ref: String,
    pub runner: String,
    pub platform: String,
    pub container: Option<String>,
    pub surface_pin: Option<String>,
    pub preferred_surface: Option<String>,
    pub previous_run_id: Option<i64>,
    pub previous_run_repo: Option<String>,
    pub push_nix_cache: bool,
    pub actions_cache: Option<String>,
    pub actions_cache_restore_keys: Vec<String>,
    pub allow_paid_blacksmith: f64,
    /// Workflow job timeout (`--timeout-minutes`, 5–120). None leaves
    /// the workflow's own default (60) in force (#13178).
    pub timeout_minutes: Option<u32>,
    /// The placement session handle this dispatch belongs to. Not part
    /// of the stored job: it travels as the `dispatch_id` input so the
    /// run's display title names the session (#13178).
    pub session_handle: Option<String>,
}

impl Job {
    fn to_json(&self) -> Value {
        json!({
            "repo": self.repo,
            "command": self.command,
            "ref": self.git_ref,
            "runner": self.runner,
            "platform": self.platform,
            "container": self.container,
            "surface_pin": self.surface_pin,
            "preferred_surface": self.preferred_surface,
            "previous_run_id": self.previous_run_id,
            "previous_run_repo": self.previous_run_repo,
            "push_nix_cache": self.push_nix_cache,
            "actions_cache": self.actions_cache,
            "actions_cache_restore_keys": self.actions_cache_restore_keys,
            "allow_paid_blacksmith": self.allow_paid_blacksmith,
            "timeout_minutes": self.timeout_minutes,
        })
    }

    /// The printed form drops empty/false fields, as the reference does.
    fn to_json_compact(&self) -> Value {
        let mut out = Map::new();
        for (k, v) in self.to_json().as_object().unwrap() {
            let empty = v.is_null()
                || v.as_str() == Some("")
                || v.as_bool() == Some(false)
                || v.as_f64() == Some(0.0)
                || v.as_array().is_some_and(|values| values.is_empty());
            if !empty {
                out.insert(k.clone(), v.clone());
            }
        }
        Value::Object(out)
    }

    /// Rebuild a job from its stored record — the `session status`
    /// grant path re-dispatches a coordinator-queued session from the
    /// job its dispatch pass committed. Unknown or missing fields
    /// fall back to the dispatch defaults, exactly as the flag
    /// parser would supply them.
    fn from_json(v: &Value) -> Option<Job> {
        let repo = v.get("repo")?.as_str()?.to_string();
        let command = v.get("command")?.as_str()?.to_string();
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        Some(Job {
            repo,
            command,
            git_ref: s("ref").unwrap_or_default(),
            runner: s("runner").unwrap_or_else(|| "auto".to_string()),
            platform: s("platform").unwrap_or_else(|| "linux".to_string()),
            container: s("container"),
            surface_pin: s("surface_pin"),
            preferred_surface: s("preferred_surface"),
            previous_run_id: v.get("previous_run_id").and_then(|x| x.as_i64()),
            previous_run_repo: s("previous_run_repo"),
            push_nix_cache: v
                .get("push_nix_cache")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            actions_cache: s("actions_cache"),
            actions_cache_restore_keys: v
                .get("actions_cache_restore_keys")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            allow_paid_blacksmith: v
                .get("allow_paid_blacksmith")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            timeout_minutes: v
                .get("timeout_minutes")
                .and_then(|x| x.as_u64())
                .and_then(|t| u32::try_from(t).ok())
                .filter(|t| TIMEOUT_MINUTES_RANGE.contains(t)),
            session_handle: None,
        })
    }
}

/// The range fleet-command.yml accepts for its `timeout_minutes` input.
pub const TIMEOUT_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 5..=120;

// ----------------------------------------------------------------- context

/// Everything a decision needs beyond the registry and the job:
/// live per-owner load, the Blacksmith budget position, and the
/// laptop saturation flag. Built by `build_ctx`; tests build it
/// directly.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub owners: Value,
    pub owner_load: HashMap<String, (i64, i64)>, // owner -> (running, queued)
    pub owner_load_coverage: HashMap<String, LoadCoverage>,
    pub owner_last_dispatch: HashMap<String, i64>,
    pub surface_last_dispatch: HashMap<String, i64>,
    pub owner_load_overridden: Vec<String>,
    pub load_read_errors: Vec<String>,
    pub laptop_busy: bool,
    pub laptop_busy_source: String,
    pub blacksmith_used_minutes: f64,
}

impl Ctx {
    fn owner_cap(&self, owner: &str) -> i64 {
        self.owners
            .get(owner)
            .and_then(|o| o.get("concurrent_jobs_cap"))
            .and_then(|c| c.as_i64())
            .unwrap_or(0)
    }
}

/// How completely an owner's live load was observed while building
/// the context. The collector counts running+queued runs per owner
/// through the GitHub API; every required read either yields a count
/// or is recorded as missing, so a zero is only ever reported as a
/// *measured* zero when every read succeeded. Before this existed, a
/// missing token, a 403, or a malformed 200 all collapsed to the
/// same numeric 0 as a genuinely idle account (issue #13611).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadCoverageStatus {
    /// Every required host/status read returned a valid count.
    Measured,
    /// Some reads returned counts; the rest are missing. The known
    /// subtotal is a lower bound on the owner's true load.
    Partial,
    /// No read returned a count; the owner's load is unknown.
    Unknown,
    /// The load was supplied by the explicit `--assume-owner-load`
    /// simulation override, not read from GitHub.
    Simulated,
}

impl LoadCoverageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoadCoverageStatus::Measured => "measured",
            LoadCoverageStatus::Partial => "partial",
            LoadCoverageStatus::Unknown => "unknown",
            LoadCoverageStatus::Simulated => "simulated",
        }
    }
}

/// Per-owner load-read coverage: what was observed, what is known,
/// and what could not be read. Carried into the decision JSON so an
/// operator can tell a measured idle account from an unreadable one.
#[derive(Debug, Clone)]
pub struct LoadCoverage {
    pub status: LoadCoverageStatus,
    pub known_running: i64,
    pub known_queued: i64,
    pub cap: i64,
    pub read_at: i64,
    pub source: String,
    pub missing_reads: Vec<String>,
}

impl LoadCoverage {
    /// The observed running+queued subtotal: exact for measured and
    /// simulated coverage, a lower bound for partial coverage.
    pub fn known_load(&self) -> i64 {
        self.known_running.saturating_add(self.known_queued)
    }

    /// A numeric free-slot estimate is only meaningful when the
    /// load figure is complete (measured) or an explicitly labeled
    /// simulation. Partial coverage still proves saturation when
    /// the known subtotal already reaches the cap.
    pub fn estimated_free(&self, cap: i64) -> Option<i64> {
        match self.status {
            LoadCoverageStatus::Measured | LoadCoverageStatus::Simulated => {
                Some((cap - self.known_load()).max(0))
            }
            LoadCoverageStatus::Partial if self.known_load() >= cap => Some(0),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "known_running": self.known_running,
            "known_queued": self.known_queued,
            "known_load": self.known_load(),
            "cap": self.cap,
            "read_at": self.read_at,
            "source": self.source,
            "missing_reads": self.missing_reads,
        })
    }
}

/// The `owner_load_coverage` fragment of the decision JSON, keyed
/// by owner in sorted order so the output is deterministic.
pub fn owner_load_coverage_json(ctx: &Ctx) -> Value {
    let mut owners: Vec<&String> = ctx.owner_load_coverage.keys().collect();
    owners.sort();
    let mut out = Map::new();
    for owner in owners {
        out.insert(owner.clone(), ctx.owner_load_coverage[owner].to_json());
    }
    Value::Object(out)
}

// ------------------------------------------------------- side-effect surface

/// The boundary between the pure placement logic and the world:
/// the GitHub API (as the App), the fleet-paperwork token flow, and
/// the `gh codespace` surface. Production uses [`RealGh`]; tests
/// substitute a fake.
pub trait Gh {
    /// An installation token for the repo's owner via the
    /// fleet-paperwork helper, or None when the app does not cover
    /// the repo / the helper is unavailable.
    fn mint_token(&self, repo: &str) -> Option<String>;
    /// Like the reference's app_token(): minting is mandatory on the
    /// dispatch path — a surface the app cannot reach is ineligible,
    /// loudly.
    fn app_token(&self, repo: &str) -> R<String> {
        self.mint_token(repo).ok_or_else(|| {
            Die(format!(
                "app token mint failed for {repo} (app does not cover it, \
                 or the fleet-paperwork helper is unavailable) — surface \
                 is ineligible for app dispatch until covered"
            ))
        })
    }
    /// The signed-in user's own token, for a surface whose registry
    /// entry says `"dispatch_as": "user"`. Dispatching a workflow with
    /// a token the fleet already holds is a routine operation
    /// (FDR-0013), so a surface the App is not installed on is still
    /// usable. None where no user token is available.
    fn user_token(&self) -> Option<String> {
        None
    }
    fn api(&self, method: &str, path: &str, token: &str, body: Option<&Value>) -> R<(u16, Value)>;
    /// `gh codespace list --json name,repository,state`, parsed.
    fn codespace_list(&self) -> R<Vec<Value>>;
    /// `gh codespace ssh -c NAME -- COMMAND`; returns the exit code.
    fn codespace_ssh(&self, name: &str, command: &str) -> R<i32>;
    /// Like codespace_ssh but also returns trimmed stdout (used to
    /// validate the reused codespace's checked-out revision).
    fn codespace_ssh_capture(&self, name: &str, command: &str) -> R<(i32, String)> {
        Ok((self.codespace_ssh(name, command)?, String::new()))
    }
    fn sleep_secs(&self, secs: u64) {
        std::thread::sleep(Duration::from_secs(secs));
    }
    fn now_epoch(&self) -> i64 {
        now_epoch()
    }
}

struct RealGh {
    agent: ureq::Agent,
    tokens: RefCell<HashMap<String, String>>,
    /// The user's token, looked up at most once per run: a miss is
    /// remembered too, so a cascade over several `dispatch_as: user`
    /// surfaces does not repeat a slow lookup.
    user: RefCell<Option<Option<String>>>,
}

/// The fleet-secrets name of the `gh` hosts file (the personal leg
/// `fleet gh` uses).
const USER_HOSTS_SECRET: &str = "github-cli.hosts-yml";

/// The `oauth_token` value in gh's hosts.yml content. Line-shaped on
/// purpose, like `fleet gh`'s own reader: the file is a small
/// fixed-shape config.
fn hosts_yml_token(hosts_yml: &str) -> Option<String> {
    hosts_yml.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("oauth_token:")?;
        let value = rest.trim().trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The first usable token out of an ordered list of lookups; a
/// lookup that yields only whitespace counts as a miss.
fn first_token(lookups: &[&dyn Fn() -> Option<String>]) -> Option<String> {
    lookups
        .iter()
        .filter_map(|lookup| lookup())
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())
}

impl RealGh {
    fn new() -> Self {
        // Platform root store, not webpki-roots alone: hosts
        // behind a TLS-intercepting egress proxy present the
        // proxy's CA, which only the platform store carries (the
        // sandbox seat's dispatch died with UnknownIssuer before
        // this — code#13122; same fix as fleet's gh.rs App leg).
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        RealGh {
            agent,
            user: RefCell::new(None),
            tokens: RefCell::new(HashMap::new()),
        }
    }
}

fn paperwork_path() -> String {
    std::env::var("FLEET_PAPERWORK").unwrap_or_else(|_| {
        format!(
            "{}/git/code/infra/fleet-paperwork/fleet_paperwork.py",
            home_dir()
        )
    })
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_default()
}

impl Gh for RealGh {
    /// Where the user's token comes from, cheapest first: the
    /// environment, the seat's own `gh` login, then the fleet secrets
    /// chain by name (the same hosts file `fleet gh` reads). The
    /// value goes into Authorization headers only.
    fn user_token(&self) -> Option<String> {
        if let Some(known) = self.user.borrow().as_ref() {
            return known.clone();
        }
        let from_env = || {
            ["GH_TOKEN", "GITHUB_TOKEN"]
                .iter()
                .filter_map(|k| std::env::var(k).ok())
                .find(|v| !v.trim().is_empty())
        };
        let from_gh = || {
            let out = std::process::Command::new("gh")
                .args(["auth", "token"])
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).to_string())
        };
        let from_secrets = || {
            let config = fleet_config::Config::load().unwrap_or_default();
            let hosts = fleet_secrets::Resolver::default_chain(&config)
                .resolve(USER_HOSTS_SECRET)
                .ok()?;
            hosts_yml_token(hosts.expose())
        };
        let found = first_token(&[&from_env, &from_gh, &from_secrets]);
        *self.user.borrow_mut() = Some(found.clone());
        found
    }

    fn mint_token(&self, repo: &str) -> Option<String> {
        if let Some(tok) = self.tokens.borrow().get(repo) {
            return Some(tok.clone());
        }
        let path = paperwork_path();
        if !std::path::Path::new(&path).exists() {
            return None;
        }
        let out = std::process::Command::new("python3")
            .args([&path, "token", "--repo", repo])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let tok = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if tok.is_empty() {
            return None;
        }
        self.tokens
            .borrow_mut()
            .insert(repo.to_string(), tok.clone());
        Some(tok)
    }

    fn api(&self, method: &str, path: &str, token: &str, body: Option<&Value>) -> R<(u16, Value)> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("https://api.github.com{path}")
        };
        let auth = format!("Bearer {token}");
        let mut resp = if method == "POST" {
            self.agent
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("Authorization", &auth)
                .send_json(body.cloned().unwrap_or_else(|| json!({})))
                .map_err(|e| Die(format!("github api {method} {path}: {e}")))?
        } else {
            self.agent
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("Authorization", &auth)
                .call()
                .map_err(|e| Die(format!("github api {method} {path}: {e}")))?
        };
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let payload = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "message": text }))
        };
        Ok((status, payload))
    }

    fn codespace_list(&self) -> R<Vec<Value>> {
        let out = std::process::Command::new("gh")
            .args(["codespace", "list", "--json", "name,repository,state"])
            .output()
            .map_err(|e| Die(format!("gh codespace list: {e}")))?;
        if !out.status.success() {
            return die(format!(
                "gh codespace list failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            return Ok(vec![]);
        }
        serde_json::from_str(&text).map_err(|e| Die(format!("gh codespace list json: {e}")))
    }

    fn codespace_ssh(&self, name: &str, command: &str) -> R<i32> {
        let out = std::process::Command::new("gh")
            .args(["codespace", "ssh", "-c", name, "--", command])
            .output()
            .map_err(|e| Die(format!("gh codespace ssh: {e}")))?;
        Ok(out.status.code().unwrap_or(-1))
    }

    fn codespace_ssh_capture(&self, name: &str, command: &str) -> R<(i32, String)> {
        let out = std::process::Command::new("gh")
            .args(["codespace", "ssh", "-c", name, "--", command])
            .output()
            .map_err(|e| Die(format!("gh codespace ssh: {e}")))?;
        Ok((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    }
}

// ---------------------------------------------------------------- registry

fn surf_str<'a>(surface: &'a Value, key: &str) -> &'a str {
    surface.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// Whether a surface dispatches with the user's own token instead of
/// the App's installation token (registry field `dispatch_as`).
fn dispatches_as_user(surface: &Value) -> bool {
    surf_str(surface, "dispatch_as") == "user"
}

/// The identity a surface dispatches as, for a session record's
/// `actor` (#13178): the user's own credentials for a `dispatch_as:
/// user` surface or a codespace, no remote identity for a laptop
/// handoff, else the GitHub App installation. `None` (nothing chosen)
/// records that no dispatch happened.
pub(crate) fn dispatch_actor(surface: Option<&Value>) -> &'static str {
    let Some(surface) = surface else {
        return "none (no surface dispatched)";
    };
    match surf_str(surface, "kind") {
        "laptop" => "laptop handoff (no remote dispatch)",
        "codespace" => "user gh CLI credentials (codespace)",
        _ if dispatches_as_user(surface) => "user token (dispatch_as: user)",
        _ => "developing-today[bot] (GitHub App dispatch)",
    }
}

/// The token a surface's API calls use, when one is available: the
/// user's for a `dispatch_as: user` surface, else the App's.
fn surface_token(surface: &Value, gh: &dyn Gh) -> Option<String> {
    if dispatches_as_user(surface) {
        gh.user_token()
    } else {
        gh.mint_token(surf_str(surface, "repo"))
    }
}

/// Why a surface has no token, for an eligibility reason or an error.
fn no_token_reason(surface: &Value) -> String {
    let repo = surf_str(surface, "repo");
    if dispatches_as_user(surface) {
        format!(
            "{repo} dispatches as the user, and no user token is available here \
             (GH_TOKEN, GITHUB_TOKEN or `gh auth token`)"
        )
    } else {
        format!(
            "app token cannot be minted for {repo} \u{2014} surface is \
             unreachable by the App"
        )
    }
}

fn surf_targets(surface: &Value) -> Vec<String> {
    surface
        .get("targets")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Target rules accept an exact owner/repo, a whole-repository star,
/// or an owner-scoped owner/* rule. Owner matching is exact and the
/// repository component must be a single non-empty path segment.
fn target_matches(target: &str, repo: &str) -> bool {
    if target == "*" {
        return repo.split_once('/').is_some_and(|(owner, name)| {
            !owner.is_empty() && !name.is_empty() && !name.contains('/')
        });
    }
    if let Some(owner) = target.strip_suffix("/*") {
        return repo.split_once('/').is_some_and(|(repo_owner, name)| {
            repo_owner == owner && !name.is_empty() && !name.contains('/')
        });
    }
    target == repo
}

fn surface_targets_job(surface: &Value, job: &Job) -> bool {
    surf_targets(surface)
        .iter()
        .any(|target| target_matches(target, &job.repo))
}

fn valid_owner_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    let valid_part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    valid_part(owner) && valid_part(name) && !name.contains('/')
}

/// A portable run reference names the Fleet runner repository and its
/// GitHub Actions run ID. Unlike an fp_ handle, it is resolvable from
/// any workspace with GitHub access and the same surface registry.
fn parse_portable_run_reference(reference: &str) -> Option<(String, i64)> {
    let (repo, raw_id) = reference.rsplit_once('#')?;
    let run_id = raw_id.parse::<i64>().ok()?;
    if !valid_owner_repo(repo) || run_id <= 0 {
        return None;
    }
    Some((repo.to_string(), run_id))
}

fn resolve_portable_run_reference(reference: &str, registry: &Value, gh: &dyn Gh) -> R<Value> {
    let (repo, run_id) = parse_portable_run_reference(reference).ok_or_else(|| {
        Die(format!(
            "invalid portable run reference {reference:?}; expected owner/repo#run_id"
        ))
    })?;
    let surface = registry_surfaces(registry)
        .into_iter()
        .find(|surface| {
            surf_str(surface, "repo") == repo
                && matches!(surf_str(surface, "kind"), "github-actions" | "blacksmith")
        })
        .ok_or_else(|| {
            Die(format!(
                "{repo} is not a registered Fleet GitHub Actions runner repository"
            ))
        })?;
    let token = surface_token(&surface, gh).ok_or_else(|| {
        Die(format!(
            "no GitHub token is available to read runner {repo}"
        ))
    })?;
    let (status, run) = gh.api(
        "GET",
        &format!("/repos/{repo}/actions/runs/{run_id}"),
        &token,
        None,
    )?;
    if status != 200 {
        return die(format!(
            "cannot read Fleet run {reference} (GitHub API HTTP {status})"
        ));
    }
    let run_url = run
        .get("html_url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{repo}/actions/runs/{run_id}"));
    Ok(json!({
        "handle": reference,
        "portable_id": reference,
        "runner_repo": repo,
        "run_id": run_id,
        "run_url": run_url,
        "surface": surf_str(&surface, "id"),
        "surface_kind": surf_str(&surface, "kind"),
        "state": map_run_state(&run),
        "created_at": run.get("created_at").cloned().unwrap_or(Value::Null),
        "updated_at": run.get("updated_at").cloned().unwrap_or(Value::Null),
    }))
}

fn load_registry(path: &str) -> R<Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Die(format!("cannot read registry {path}: {e}")))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Die(format!("registry {path} is not valid JSON: {e}")))?;
    if value.get("surfaces").and_then(|s| s.as_array()).is_none() {
        return die(format!("registry {path} has no surfaces array"));
    }
    Ok(value)
}

fn registry_surfaces(registry: &Value) -> Vec<Value> {
    registry
        .get("surfaces")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default()
}

fn find_surface<'a>(registry: &'a Value, id: &str) -> Option<&'a Value> {
    registry
        .get("surfaces")
        .and_then(|s| s.as_array())?
        .iter()
        .find(|s| surf_str(s, "id") == id)
}

// ------------------------------------------------------------ session store

fn state_dir() -> String {
    std::env::var("FLEET_PLACE_STATE")
        .unwrap_or_else(|_| format!("{}/.local/state/fleet-place", home_dir()))
}

fn state_file() -> String {
    format!("{}/sessions.json", state_dir())
}

/// Interprocess lock for the session store: a lock directory
/// created with create_dir (atomic) next to the store file.
/// Holders write pid+epoch into it; a lock older than 120s is
/// stale (crashed holder) and is broken. This serializes the
/// store's read-modify-write transactions so concurrent
/// dispatches/status calls cannot drop each other's sessions
/// (Codex P1 on PR #8325).
struct StoreLock {
    dir: String,
}

impl StoreLock {
    fn acquire() -> R<StoreLock> {
        let dir = format!("{}/sessions.lockdir", state_dir());
        std::fs::create_dir_all(state_dir())
            .map_err(|e| Die(format!("cannot create state dir: {e}")))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            match std::fs::create_dir(&dir) {
                Ok(()) => {
                    let _ = std::fs::write(
                        format!("{dir}/owner"),
                        format!("{} {}", std::process::id(), now_epoch()),
                    );
                    return Ok(StoreLock { dir });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&dir)
                        .and_then(|m| m.modified())
                        .map(|t| t.elapsed().map(|d| d.as_secs() > 120).unwrap_or(false))
                        .unwrap_or(false);
                    if stale {
                        let _ = std::fs::remove_dir_all(&dir);
                        continue;
                    }
                    if std::time::Instant::now() > deadline {
                        return die(format!(
                            "session store is locked by another fleet-place process ({dir}); retry shortly"
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return die(format!("cannot lock session store {dir}: {e}"));
                }
            }
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn with_store_lock<T>(f: impl FnOnce() -> R<T>) -> R<T> {
    let _guard = StoreLock::acquire()?;
    f()
}

/// Commit one new session record (and optionally its Blacksmith
/// ledger entry) under the store lock, merging into the CURRENT
/// store on disk — records written by other processes since this
/// process loaded its snapshot are preserved. The local commit is
/// the commit: afterwards the record mirrors best-effort to the
/// queue backend (#13146), where a failure changes nothing locally.
fn commit_new_session(
    rec: &Value,
    blacksmith_entry: Option<&Value>,
    remote: &dyn remote::SessionRemote,
) -> R<()> {
    let handle = rec
        .get("handle")
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .to_string();
    with_store_lock(|| {
        let mut store = load_store();
        if let Some(sessions) = store["sessions"].as_object_mut() {
            sessions.insert(handle.clone(), rec.clone());
        }
        if let Some(entry) = blacksmith_entry {
            if let Some(arr) = store["blacksmith_dispatches"].as_array_mut() {
                let exists = arr
                    .iter()
                    .any(|d| d.get("handle").and_then(|h| h.as_str()) == Some(handle.as_str()));
                if !exists {
                    arr.push(entry.clone());
                }
            }
        }
        save_store(&store)
    })?;
    let _ = remote.put(rec);
    Ok(())
}

fn load_store() -> Value {
    let path = state_file();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            return v;
        }
    }
    json!({ "sessions": {}, "blacksmith_dispatches": [] })
}

fn save_store(store: &Value) -> R<()> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| Die(format!("cannot create state dir {dir}: {e}")))?;
    let path = state_file();
    let tmp = format!("{path}.tmp{}", std::process::id());
    let mut text = serde_json::to_string_pretty(store).unwrap_or_default();
    text.push('\n');
    // Keys sorted for a stable store file, as the reference wrote it.
    let text = sort_json_keys(&text);
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| Die(format!("cannot write session store {path}: {e}")))
}

fn sort_json_keys(text: &str) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for k in keys {
                    out.insert(k.clone(), sort(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    match serde_json::from_str::<Value>(text) {
        Ok(v) => serde_json::to_string_pretty(&sort(&v)).unwrap_or_else(|_| text.to_string()),
        Err(_) => text.to_string(),
    }
}

fn new_handle<Src: rand_source::RandSource>(rng: &Src) -> String {
    let bytes = rng.bytes16();
    let mut out = String::with_capacity(35);
    out.push_str("fp_");
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Randomness for handles, behind a tiny seam so tests are
/// deterministic. Production reads /dev/urandom.
mod rand_source {
    pub trait RandSource {
        fn bytes16(&self) -> [u8; 16];
    }
    pub struct URandom;
    impl RandSource for URandom {
        fn bytes16(&self) -> [u8; 16] {
            use std::io::Read;
            let mut buf = [0u8; 16];
            // Read exactly 16 bytes: /dev/urandom is an endless
            // stream, so a read-to-end here would never return.
            if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
                if f.read_exact(&mut buf).is_ok() {
                    return buf;
                }
            }
            // Fallback: time + pid mixing (never expected on Linux).
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
                ^ (std::process::id() as u64) << 32;
            let mut x = seed | 1;
            for b in buf.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = (x >> 32) as u8;
            }
            buf
        }
    }
}

fn submitted_by() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{user}@{host}")
}

// ---------------------------------------------------------------- adapters

mod adapters;
mod coordinator;
pub(crate) use coordinator::{coordinator, placement_load_report};
mod remote;
use adapters::adapter_for;
#[cfg(test)]
use adapters::correlate_run;
pub use adapters::{
    Adapter, BlacksmithAdapter, Capacity, CodespaceAdapter, DispatchResult, GithubActionsAdapter,
    LaptopAdapter, map_run_state,
};

// ----------------------------------------------------------------- context

/// Build the decision context: live per-owner load counted through
/// the App (running+queued runs on each owner's fleet runner-host
/// repos — a proxy for account load; GitHub exposes no account-wide
/// runs endpoint), the month's Blacksmith consumption from the
/// session store, and the laptop saturation flag. Testing overrides
/// are applied and labeled.
pub fn build_ctx(
    registry: &Value,
    store: &Value,
    laptop_busy_flag: bool,
    assume_owner_load: &str,
    assume_blacksmith_used: Option<f64>,
    gh: &dyn Gh,
) -> Ctx {
    let owners = registry.get("owners").cloned().unwrap_or(json!({}));
    let mut hosts: HashMap<String, Vec<String>> = HashMap::new();
    // Host repos whose every API call uses the user's token.
    let mut user_repos: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in registry_surfaces(registry) {
        let kind = surf_str(&s, "kind");
        if dispatches_as_user(&s) {
            user_repos.insert(surf_str(&s, "repo").to_string());
        }
        if (kind == "github-actions" || kind == "blacksmith") && !surf_str(&s, "repo").is_empty() {
            let repos = hosts.entry(surf_str(&s, "owner").to_string()).or_default();
            let repo = surf_str(&s, "repo").to_string();
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        }
    }
    let mut overrides: HashMap<String, i64> = HashMap::new();
    for raw in assume_owner_load.split(',') {
        if let Some((k, v)) = raw.split_once('=') {
            if let Ok(n) = v.trim().parse::<i64>() {
                overrides.insert(k.trim().to_string(), n);
            }
        }
    }
    let mut owner_load: HashMap<String, (i64, i64)> = HashMap::new();
    let mut owner_load_coverage: HashMap<String, LoadCoverage> = HashMap::new();
    let mut read_errors: Vec<String> = Vec::new();
    let read_at = gh.now_epoch();
    for (owner, repos) in &hosts {
        let mut running: i64 = 0;
        let mut queued: i64 = 0;
        let mut observed: usize = 0;
        let mut expected: usize = 0;
        let mut missing_reads: Vec<String> = Vec::new();
        for repo in repos {
            let minted = if user_repos.contains(repo) {
                gh.user_token()
            } else {
                gh.mint_token(repo)
            };
            let token = match minted {
                Some(t) => t,
                None => {
                    expected += 2;
                    let msg = format!(
                        "{repo}: no token available (not covered) — load unobserved for this repo"
                    );
                    read_errors.push(msg.clone());
                    missing_reads.push(msg);
                    continue;
                }
            };
            for status_name in ["in_progress", "queued"] {
                expected += 1;
                match gh.api(
                    "GET",
                    &format!("/repos/{repo}/actions/runs?status={status_name}&per_page=1"),
                    &token,
                    None,
                ) {
                    Ok((200, payload)) => {
                        match payload.get("total_count").and_then(|t| t.as_i64()) {
                            Some(total) => {
                                observed += 1;
                                if status_name == "in_progress" {
                                    running += total;
                                } else {
                                    queued += total;
                                }
                            }
                            None => {
                                let msg = format!(
                                    "{repo} ({status_name}): HTTP 200 without a valid \
                                     total_count — load unobserved"
                                );
                                read_errors.push(msg.clone());
                                missing_reads.push(msg);
                            }
                        }
                    }
                    Ok((code, _)) => {
                        let msg = format!("{repo} ({status_name}): HTTP {code}");
                        read_errors.push(msg.clone());
                        missing_reads.push(msg);
                    }
                    Err(e) => {
                        let msg = format!("{repo} ({status_name}): {}", e.0);
                        read_errors.push(msg.clone());
                        missing_reads.push(msg);
                    }
                }
            }
        }
        let status = if overrides.contains_key(owner) {
            LoadCoverageStatus::Simulated
        } else if observed == expected {
            LoadCoverageStatus::Measured
        } else if observed > 0 {
            LoadCoverageStatus::Partial
        } else {
            LoadCoverageStatus::Unknown
        };
        if let Some(n) = overrides.get(owner) {
            running = *n;
            queued = 0;
        }
        owner_load.insert(owner.clone(), (running, queued));
        owner_load_coverage.insert(
            owner.clone(),
            LoadCoverage {
                status,
                known_running: running,
                known_queued: queued,
                cap: owners
                    .get(owner)
                    .and_then(|o| o.get("concurrent_jobs_cap"))
                    .and_then(|c| c.as_i64())
                    .unwrap_or(0),
                read_at,
                source: if status == LoadCoverageStatus::Simulated {
                    "--assume-owner-load override (simulation, not live counts)".to_string()
                } else {
                    "live GitHub Actions run counts over fleet runner hosts".to_string()
                },
                missing_reads,
            },
        );
    }
    let mut owner_last_dispatch: HashMap<String, i64> = HashMap::new();
    let mut surface_last_dispatch: HashMap<String, i64> = HashMap::new();
    if let Some(sessions) = store.get("sessions").and_then(Value::as_object) {
        for rec in sessions.values() {
            let surface_id = rec.get("surface").and_then(Value::as_str).unwrap_or("");
            let created = rec
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(parse_iso)
                .unwrap_or(0);
            if surface_id.is_empty() || created <= 0 {
                continue;
            }
            let Some(surface) = find_surface(registry, surface_id) else {
                continue;
            };
            let owner = surf_str(surface, "owner").to_string();
            surface_last_dispatch
                .entry(surface_id.to_string())
                .and_modify(|latest| *latest = (*latest).max(created))
                .or_insert(created);
            owner_last_dispatch
                .entry(owner)
                .and_modify(|latest| *latest = (*latest).max(created))
                .or_insert(created);
        }
    }
    let laptop_cfg = registry.get("laptop").cloned().unwrap_or(json!({}));
    let laptop_busy = laptop_busy_flag
        || laptop_cfg
            .get("busy")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
    let laptop_busy_source = if laptop_busy_flag {
        "--laptop-busy flag".to_string()
    } else {
        laptop_cfg
            .get("busy_source")
            .and_then(|s| s.as_str())
            .unwrap_or("registry laptop.busy")
            .to_string()
    };
    let month = current_month();
    let mut used: f64 = store
        .get("blacksmith_dispatches")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|r| r.get("month").and_then(|m| m.as_str()) == Some(month.as_str()))
                .map(|r| r.get("minutes").and_then(|m| m.as_f64()).unwrap_or(0.0))
                .sum()
        })
        .unwrap_or(0.0);
    if let Some(v) = assume_blacksmith_used {
        used = v;
    }
    let mut overridden: Vec<String> = overrides.keys().cloned().collect();
    overridden.sort();
    Ctx {
        owners,
        owner_load,
        owner_load_coverage,
        owner_last_dispatch,
        surface_last_dispatch,
        owner_load_overridden: overridden,
        load_read_errors: read_errors,
        laptop_busy,
        laptop_busy_source,
        blacksmith_used_minutes: used,
    }
}

// ----------------------------------------------------------------- cascade

/// The placement cascade (spec §"The placement cascade"). Returns
/// the chosen adapter (None = the job stays queued) and the
/// reasoning lines that explain the decision.
pub fn decide(
    registry: &Value,
    job: &Job,
    ctx: &Ctx,
    gh: &dyn Gh,
) -> R<(Option<Box<dyn Adapter>>, Vec<String>)> {
    let mut reasoning: Vec<String> = Vec::new();
    let surfaces = registry_surfaces(registry);
    let mut adapters: HashMap<String, Box<dyn Adapter>> = HashMap::new();
    for s in &surfaces {
        if let Some(ad) = adapter_for(s) {
            adapters.insert(surf_str(s, "id").to_string(), ad);
        }
    }

    if let Some(pin) = &job.surface_pin {
        let ad = match adapters.get(pin) {
            Some(ad) => ad,
            None => return die(format!("--surface {pin}: no such surface in the registry")),
        };
        let (ok, why) = ad.eligible(job, ctx, gh);
        reasoning.push(format!("pinned surface {pin}: eligible={ok} ({why})"));
        if !ok {
            return die(format!("pinned surface {pin} is ineligible: {why}"));
        }
        // Rebuild the boxed adapter to return (the map borrow ends here).
        let surface = find_surface(registry, pin).unwrap().clone();
        return Ok((adapter_for(&surface), reasoning));
    }

    // Followups keep their previous host repo and cache while it
    // remains healthy. A hard pin above always wins; this preference
    // falls through when the owner is full or has a material backlog.
    if let Some(preferred) = &job.preferred_surface {
        if let Some(surface) = find_surface(registry, preferred) {
            if let Some(ad) = adapters.get(preferred) {
                let (ok, why) = ad.eligible(job, ctx, gh);
                let cap = ad.capacity(job, ctx);
                let owner = surf_str(surface, "owner");
                let queued = ctx.owner_load.get(owner).map(|(_, q)| *q).unwrap_or(0);
                let queue_threshold = surface
                    .get("sticky_queue_threshold")
                    .and_then(Value::as_i64)
                    .unwrap_or(2)
                    .max(1);
                let has_capacity = cap
                    .free_slots
                    .map(|free| free > 0)
                    .or_else(|| cap.remaining_total_minutes.map(|minutes| minutes > 0.0))
                    .unwrap_or(true);
                if ok && has_capacity && queued < queue_threshold {
                    reasoning.push(format!(
                        "followup affinity keeps {preferred}: {}",
                        cap.summary
                    ));
                    return Ok((adapter_for(surface), reasoning));
                }
                reasoning.push(format!(
                    "followup affinity released from {preferred}: eligible={ok},                      capacity={has_capacity}, queued={queued} (threshold {queue_threshold}); {why}"
                ));
            } else {
                reasoning.push(format!(
                    "followup affinity released from {preferred}: adapter is unavailable"
                ));
            }
        } else {
            reasoning.push(format!(
                "followup affinity released from {preferred}: surface is no longer registered"
            ));
        }
    }

    // Tier 1 — free GitHub-hosted Actions capacity. Balance owners by
    // live load first, then use the least-recently-used owner to
    // distribute idle bursts evenly across accounts. Within an owner,
    // prefer its Fleet runner repo so followups can reuse that cache.
    let mut tier1: Vec<(i64, i64, i64, i64, String)> = Vec::new();
    // Eligible surfaces whose owner load could not be fully read
    // (and is not proven saturated): they carry no numeric estimate,
    // but stay placeable under the unknown-capacity rule below.
    let mut tier1_unknown: Vec<(i64, i64, i64, String)> = Vec::new();
    for s in &surfaces {
        if surf_str(s, "kind") != "github-actions" {
            continue;
        }
        let id = surf_str(s, "id").to_string();
        let ad = &adapters[&id];
        let (ok, why) = ad.eligible(job, ctx, gh);
        if !ok {
            reasoning.push(format!("tier1 skip {id}: {why}"));
            continue;
        }
        let cap = ad.capacity(job, ctx);
        reasoning.push(format!("tier1 candidate {id}: {}", cap.summary));
        if let Some(free) = cap.free_slots {
            if free > 0 {
                let owner = surf_str(s, "owner");
                let (running, queued) = ctx.owner_load.get(owner).copied().unwrap_or((0, 0));
                let load = running.saturating_add(queued);
                let owner_last = ctx.owner_last_dispatch.get(owner).copied().unwrap_or(0);
                let surface_last = ctx.surface_last_dispatch.get(&id).copied().unwrap_or(0);
                let priority = s
                    .get("selection_priority")
                    .and_then(Value::as_i64)
                    .unwrap_or(50);
                tier1.push((load, owner_last, priority, surface_last, id));
            } else {
                reasoning.push(format!("tier1 unavailable {id}: no free account slots"));
            }
        } else {
            let owner = surf_str(s, "owner");
            let owner_last = ctx.owner_last_dispatch.get(owner).copied().unwrap_or(0);
            let surface_last = ctx.surface_last_dispatch.get(&id).copied().unwrap_or(0);
            let priority = s
                .get("selection_priority")
                .and_then(Value::as_i64)
                .unwrap_or(50);
            tier1_unknown.push((owner_last, priority, surface_last, id));
        }
    }
    if !tier1.is_empty() {
        tier1.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
                .then_with(|| a.4.cmp(&b.4))
        });
        let sid = tier1[0].4.clone();
        reasoning.push(format!(
            "tier1 pick {sid}: least-loaded eligible owner, with idle owners rotated by last dispatch"
        ));
        let surface = find_surface(registry, &sid).unwrap().clone();
        return Ok((adapter_for(&surface), reasoning));
    }
    if !tier1_unknown.is_empty() {
        // Unknown-capacity rule: with no measured headroom anywhere,
        // an eligible free surface whose load is merely unreadable
        // may still be picked — deterministically, and with the
        // uncertainty named in the receipt. Unread load never
        // authorizes paid overage: this tier is free GitHub-hosted
        // capacity only, and tier 2 (Blacksmith) is not reached on
        // the strength of an unknown.
        tier1_unknown.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
        });
        let sid = tier1_unknown[0].3.clone();
        reasoning.push(format!(
            "tier1 pick {sid}: unknown-capacity rule — no eligible surface has \
             measured headroom, and this surface's owner load could not be \
             fully read, so its occupancy is unknown rather than zero; picked \
             deterministically among unknown-capacity surfaces (owner \
             rotation, selection priority, surface last dispatch, surface id)"
        ));
        let surface = find_surface(registry, &sid).unwrap().clone();
        return Ok((adapter_for(&surface), reasoning));
    }
    reasoning
        .push("tier1 exhausted: no eligible GitHub-hosted surface has a free slot".to_string());

    // Tier 2 — Blacksmith within budget (org surfaces only).
    for s in &surfaces {
        if surf_str(s, "kind") != "blacksmith" {
            continue;
        }
        let id = surf_str(s, "id").to_string();
        let ad = &adapters[&id];
        let (ok, why) = ad.eligible(job, ctx, gh);
        if !ok {
            reasoning.push(format!("tier2 skip {id}: {why}"));
            continue;
        }
        let cap = ad.capacity(job, ctx);
        reasoning.push(format!("tier2 candidate {id}: {}", cap.summary));
        if cap.remaining_total_minutes.unwrap_or(0.0) > 0.0 {
            reasoning.push(format!("tier2 pick {id}: budget remains"));
            let surface = s.clone();
            return Ok((adapter_for(&surface), reasoning));
        }
    }
    reasoning.push("tier2 exhausted: Blacksmith budget spent or ineligible".to_string());

    // Tier 3 — Codespaces (last resort; needs laptop saturation).
    for s in &surfaces {
        if surf_str(s, "kind") != "codespace" {
            continue;
        }
        let id = surf_str(s, "id").to_string();
        let ad = &adapters[&id];
        let (ok, why) = ad.eligible(job, ctx, gh);
        reasoning.push(format!(
            "tier3 {} {id}: {why}",
            if ok { "candidate" } else { "skip" }
        ));
        if ok {
            reasoning.push(format!("tier3 pick {id}"));
            let surface = s.clone();
            return Ok((adapter_for(&surface), reasoning));
        }
    }

    reasoning.push(
        "all tiers exhausted — job stays queued (recorded, never dropped; \
         repo-queue remains its system of record)"
            .to_string(),
    );
    Ok((None, reasoning))
}

// --------------------------------------------------------------- sessions

/// #13765 queued-session completion: a session the coordinator
/// queued carries its claim handle and job. Once the coordinator
/// grants the claim, dispatch the stored job on the granted surface,
/// confirm the slot, and move the record off `queued`. A failed
/// dispatch releases the slot and fails the session. Best-effort: an
/// unreachable coordinator or a claim that is not granted leaves the
/// record queued for the next status call.
fn complete_queued_session(
    store: &mut Value,
    handle: &str,
    rec: &mut Value,
    registry: &Value,
    gh: &dyn Gh,
) {
    if rec.get("state").and_then(Value::as_str) != Some("queued") {
        return;
    }
    let Some(claim_handle) = rec
        .get("claim_handle")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let coord = coordinator::coordinator();
    let Ok(status) = coord.claim_status(&claim_handle) else {
        return;
    };
    let claim = status.get("claim").cloned().unwrap_or(Value::Null);
    if claim.get("state").and_then(Value::as_str) != Some("granted") {
        return;
    }
    let Some(mut job) = rec.get("job").and_then(Job::from_json) else {
        return;
    };
    job.session_handle = Some(handle.to_string());
    let Some(sid) = claim
        .get("surface")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    let Some(surface) = find_surface(registry, &sid) else {
        return;
    };
    let Some(ad) = adapter_for(surface) else {
        return;
    };
    let kind = surf_str(surface, "kind").to_string();
    let runner_repo = surf_str(surface, "repo").to_string();
    match ad.dispatch(&job, gh) {
        Err(Die(msg)) => {
            let _ = coord.release(&claim_handle, None);
            rec["state"] = json!("failed");
            rec["note"] = json!(format!("coordinator grant dispatch failed: {msg}"));
        }
        Ok(result) => {
            let _ = coord.confirm(&claim_handle, result.run_id);
            rec["actor"] = json!(dispatch_actor(Some(surface)));
            rec["surface"] = json!(sid);
            rec["surface_kind"] = json!(kind);
            if !runner_repo.is_empty() {
                rec["runner_repo"] = json!(runner_repo.clone());
            }
            if let Some(run_id) = result.run_id {
                rec["portable_id"] = json!(format!("{runner_repo}#{run_id}"));
            }
            rec["run_id"] = result.run_id.map(Value::from).unwrap_or(Value::Null);
            rec["run_url"] = result
                .run_url
                .clone()
                .map(Value::from)
                .unwrap_or(Value::Null);
            rec["state"] = json!(match &result.terminal_state {
                Some(s) => s.clone(),
                None if result.run_id.is_some() => "queued".to_string(),
                None => "dispatched".to_string(),
            });
            if let Some(note) = &result.note {
                rec["note"] = json!(note);
            }
            if kind == "blacksmith" {
                if let Some(arr) = store
                    .get_mut("blacksmith_dispatches")
                    .and_then(Value::as_array_mut)
                {
                    arr.push(json!({
                        "handle": handle,
                        "month": current_month(),
                        "minutes": 0.0,
                        "run_id": result.run_id,
                        "note": "minutes reconciled from the run's duration on later invocations",
                        "reconciled": false,
                    }));
                }
            }
        }
    }
    rec["updated_at"] = json!(now_iso());
}

/// Give a terminal session's coordinator slot back. Blacksmith slots
/// settle with the run's reconciled minutes and wait for them; every
/// other slot releases as soon as its run is terminal.
fn settle_coordinator_claim(store: &Value, handle: &str, rec: &mut Value, state: &str, kind: &str) {
    let Some(claim_handle) = rec
        .get("claim_handle")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    if rec.get("coord_released").and_then(Value::as_bool) == Some(true) {
        return;
    }
    if !["succeeded", "failed", "cancelled"].contains(&state) {
        return;
    }
    let actual = if kind == "blacksmith" {
        let reconciled = store
            .get("blacksmith_dispatches")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter().find(|d| {
                    d.get("handle").and_then(Value::as_str) == Some(handle)
                        && d.get("reconciled").and_then(Value::as_bool) == Some(true)
                })
            })
            .and_then(|d| d.get("minutes").and_then(Value::as_f64));
        match reconciled {
            Some(minutes) => Some(minutes),
            None => return,
        }
    } else {
        None
    };
    if coordinator::coordinator()
        .release(&claim_handle, actual)
        .is_ok()
    {
        rec["coord_released"] = json!(true);
    }
}

fn refresh_session(store: &mut Value, handle: &str, registry: &Value, gh: &dyn Gh) -> R<Value> {
    let mut rec = store
        .get("sessions")
        .and_then(|s| s.get(handle))
        .cloned()
        .ok_or_else(|| {
            Die(format!(
                "no session with handle {handle} in {}",
                state_file()
            ))
        })?;
    complete_queued_session(store, handle, &mut rec, registry, gh);
    let run_id = rec.get("run_id").and_then(|r| r.as_i64());
    let surface_id = rec
        .get("surface")
        .and_then(|s| s.as_str())
        .map(str::to_string);
    if let (Some(run_id), Some(sid)) = (run_id, surface_id) {
        if let Some(surface) = find_surface(registry, &sid) {
            let runner_repo = surf_str(surface, "repo");
            if !runner_repo.is_empty() {
                rec["runner_repo"] = json!(runner_repo);
                rec["portable_id"] = json!(format!("{runner_repo}#{run_id}"));
            }
            if let Some(ad) = adapter_for(surface) {
                let state = ad.status(&rec, gh);
                if state != rec.get("state").and_then(|s| s.as_str()).unwrap_or("") {
                    rec["state"] = json!(state);
                    rec["updated_at"] = json!(now_iso());
                }
                // Reconcile Blacksmith minutes from the run's real
                // duration once the run is terminal.
                if surf_str(surface, "kind") == "blacksmith"
                    && ["succeeded", "failed", "cancelled"].contains(&state.as_str())
                {
                    let repo = surf_str(surface, "repo").to_string();
                    if let Some(token) = surface_token(surface, gh) {
                        if let Ok((200, run)) = gh.api(
                            "GET",
                            &format!("/repos/{repo}/actions/runs/{run_id}"),
                            &token,
                            None,
                        ) {
                            if let (Some(started), Some(updated)) = (
                                run.get("run_started_at")
                                    .and_then(|v| v.as_str())
                                    .and_then(parse_iso),
                                run.get("updated_at")
                                    .and_then(|v| v.as_str())
                                    .and_then(parse_iso),
                            ) {
                                let minutes =
                                    ((updated - started) as f64 / 60.0 * 100.0).round() / 100.0;
                                if let Some(arr) = store
                                    .get_mut("blacksmith_dispatches")
                                    .and_then(|d| d.as_array_mut())
                                {
                                    for d in arr.iter_mut() {
                                        if d.get("handle").and_then(|h| h.as_str()) == Some(handle)
                                        {
                                            d["minutes"] = json!(minutes);
                                            // Mark reconciliation itself,
                                            // separately from the session's
                                            // execution state (Codex P2 on
                                            // PR #8325): a terminal session
                                            // whose duration fetch failed
                                            // stays unreconciled and is
                                            // retried by later dispatches.
                                            d["reconciled"] = json!(true);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let state = rec
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let kind = rec
        .get("surface")
        .and_then(Value::as_str)
        .and_then(|sid| find_surface(registry, sid))
        .map(|s| surf_str(s, "kind").to_string())
        .unwrap_or_default();
    settle_coordinator_claim(store, handle, &mut rec, &state, &kind);
    if let Some(sessions) = store.get_mut("sessions").and_then(|s| s.as_object_mut()) {
        sessions.insert(handle.to_string(), rec.clone());
    }
    Ok(rec)
}

// --------------------------------------------------------------------- CLI

struct Cli {
    registry: Option<String>,
    json: bool,
}

/// The shipped surface registry, embedded so the installed
/// binary has a cwd-independent default (Codex P2 on PR #8325):
/// neither the nix package nor a caller's working directory has
/// to provide the file. Explicit --registry / FLEET_PLACE_REGISTRY
/// / the ~/git/code copy still win when present.
const EMBEDDED_REGISTRY: &str = include_str!("../../../fleet-surfaces.json");

fn load_embedded_registry() -> R<Value> {
    let value: Value = serde_json::from_str(EMBEDDED_REGISTRY)
        .map_err(|e| Die(format!("embedded registry is not valid JSON: {e}")))?;
    if value.get("surfaces").and_then(|s| s.as_array()).is_none() {
        return die("embedded registry has no surfaces array".to_string());
    }
    Ok(value)
}

/// Registry resolution: an explicit override (--registry flag or
/// FLEET_PLACE_REGISTRY) is strict — a bad path is an error, never
/// a silent fallback. Without an override, the ~/git/code copy
/// wins when it exists (a live checkout can carry a newer
/// registry), then the source-tree relative path, and finally
/// the embedded copy.
fn registry_value(cli: &Cli) -> R<Value> {
    if cli.registry.is_some() || std::env::var("FLEET_PLACE_REGISTRY").is_ok() {
        return load_registry(&registry_path(cli));
    }
    let home_copy = format!("{}/git/code/pkgs/fleet/fleet-surfaces.json", home_dir());
    if std::path::Path::new(&home_copy).exists() {
        return load_registry(&home_copy);
    }
    let rel = "pkgs/fleet/fleet-surfaces.json";
    if std::path::Path::new(rel).exists() {
        return load_registry(rel);
    }
    load_embedded_registry()
}

fn registry_path(cli: &Cli) -> String {
    if let Some(p) = &cli.registry {
        return p.clone();
    }
    if let Ok(p) = std::env::var("FLEET_PLACE_REGISTRY") {
        return p;
    }
    let home_copy = format!("{}/git/code/pkgs/fleet/fleet-surfaces.json", home_dir());
    if std::path::Path::new(&home_copy).exists() {
        return home_copy;
    }
    "pkgs/fleet/fleet-surfaces.json".to_string()
}

/// Pull the shared flags (--registry, --json) out of an arg list,
/// returning the rest in order.
fn extract_common(args: &[String]) -> R<(Cli, Vec<String>)> {
    let mut cli = Cli {
        registry: None,
        json: false,
    };
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--registry" => {
                i += 1;
                cli.registry = Some(
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| Die("--registry needs a path".to_string()))?,
                );
            }
            "--json" => cli.json = true,
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    Ok((cli, rest))
}

fn print_out(out: &Value, json_mode: bool, render: impl Fn(&Value) -> String) {
    if json_mode {
        println!(
            "{}",
            serde_json::to_string_pretty(out).unwrap_or_else(|_| "null".to_string())
        );
    } else {
        println!("{}", render(out));
    }
}

fn render_decision(out: &Value) -> String {
    let mut lines = vec![format!(
        "decision: {}",
        out.get("decision").and_then(|d| d.as_str()).unwrap_or("")
    )];
    if let Some(h) = out.get("handle").and_then(|h| h.as_str()) {
        lines.push(format!("handle:   {h}"));
    }
    if let Some(u) = out.get("run_url").and_then(|u| u.as_str()) {
        lines.push(format!("run:      {u}"));
    }
    if let Some(arr) = out.get("reasoning").and_then(|r| r.as_array()) {
        for r in arr {
            if let Some(s) = r.as_str() {
                lines.push(format!("  - {s}"));
            }
        }
    }
    if let Some(n) = out.get("note").and_then(|n| n.as_str()) {
        lines.push(format!("note: {n}"));
    }
    if let Some(m) = out.get("mode").and_then(|m| m.as_str()) {
        lines.push(m.to_string());
    }
    lines.join("\n")
}

/// Handles whose Blacksmith usage still needs reconciliation:
/// every non-terminal Blacksmith session, plus terminal sessions
/// whose ledger entry was never successfully reconciled (zero
/// minutes and no reconciled mark) — a transient duration-fetch
/// failure must not permanently bypass budget accounting
/// (Codex P2 on PR #8325).
fn blacksmith_reconcile_handles(store: &Value, registry: &Value) -> Vec<String> {
    let ledger = store
        .get("blacksmith_dispatches")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    store
        .get("sessions")
        .and_then(|s| s.as_object())
        .map(|m| {
            m.iter()
                .filter(|(h, rec)| {
                    let sid = rec.get("surface").and_then(|v| v.as_str()).unwrap_or("");
                    if !find_surface(registry, sid)
                        .map(|s| surf_str(s, "kind") == "blacksmith")
                        .unwrap_or(false)
                    {
                        return false;
                    }
                    if rec.get("run_id").and_then(|v| v.as_i64()).is_none() {
                        return false;
                    }
                    let state = rec.get("state").and_then(|v| v.as_str()).unwrap_or("");
                    if !["succeeded", "failed", "cancelled"].contains(&state) {
                        return true;
                    }
                    ledger.iter().any(|d| {
                        d.get("handle").and_then(|v| v.as_str()) == Some(h.as_str())
                            && d.get("reconciled").and_then(|v| v.as_bool()) != Some(true)
                            && d.get("minutes").and_then(|v| v.as_f64()).unwrap_or(0.0) == 0.0
                    })
                })
                .map(|(h, _)| h.clone())
                .collect()
        })
        .unwrap_or_default()
}

const DISPATCH_USAGE: &str = "usage: repo-queue dispatch --repo <owner/name> --command '<cmd>' \
[--ref REF] [--runner auto|github|blacksmith] [--platform linux|macos] [--container IMAGE] \
[--surface ID] [--followup HANDLE|OWNER/REPO#RUN_ID] [--push-nix-cache] \
[--actions-cache KEY:PATH] [--actions-cache-restore-key PREFIX]... \
[--allow-paid-blacksmith MINUTES] [--timeout-minutes 5-120] \
[--laptop-busy] [--dispatch] [--json] [--registry PATH]\nDry-run unless --dispatch; prints the placement decision.";

fn run_dispatch(args: &[String]) -> R<i32> {
    let (cli, rest) = extract_common(args)?;
    let mut repo: Option<String> = None;
    let mut command: Option<String> = None;
    let mut git_ref = String::new();
    let mut runner = "auto".to_string();
    let mut platform = "linux".to_string();
    let mut container: Option<String> = None;
    let mut surface_pin: Option<String> = None;
    let mut followup_handle: Option<String> = None;
    let mut push_nix_cache = false;
    let mut actions_cache: Option<String> = None;
    let mut actions_cache_restore_keys: Vec<String> = Vec::new();
    let mut allow_paid: f64 = 0.0;
    let mut timeout_minutes: Option<u32> = None;
    let mut laptop_busy_flag = false;
    let mut assume_owner_load = String::new();
    let mut assume_blacksmith_used: Option<f64> = None;
    let mut dispatch = false;
    let mut i = 0;
    let need = |args: &[String], i: usize, flag: &str| -> R<String> {
        args.get(i)
            .cloned()
            .ok_or_else(|| Die(format!("{flag} needs a value")))
    };
    while i < rest.len() {
        match rest[i].as_str() {
            "--repo" => {
                i += 1;
                repo = Some(need(&rest, i, "--repo")?);
            }
            "--command" => {
                i += 1;
                command = Some(need(&rest, i, "--command")?);
            }
            "--ref" => {
                i += 1;
                git_ref = need(&rest, i, "--ref")?;
            }
            "--runner" => {
                i += 1;
                runner = need(&rest, i, "--runner")?;
                if !["auto", "github", "blacksmith"].contains(&runner.as_str()) {
                    return die(format!(
                        "--runner must be auto|github|blacksmith, got {runner}"
                    ));
                }
            }
            "--surface" => {
                i += 1;
                surface_pin = Some(need(&rest, i, "--surface")?);
            }
            "--followup" => {
                i += 1;
                followup_handle = Some(need(&rest, i, "--followup")?);
            }
            "--platform" => {
                i += 1;
                platform = need(&rest, i, "--platform")?;
                if !["linux", "macos"].contains(&platform.as_str()) {
                    return die(format!("--platform must be linux|macos, got {platform}"));
                }
            }
            "--macos" => platform = "macos".to_string(),
            "--container" => {
                i += 1;
                let image = need(&rest, i, "--container")?;
                if image.trim().is_empty() {
                    return die("--container cannot be empty");
                }
                container = Some(image);
            }
            "--push-nix-cache" => push_nix_cache = true,
            "--actions-cache" => {
                i += 1;
                let value = need(&rest, i, "--actions-cache")?;
                let (key, cache_path) = value
                    .split_once(':')
                    .ok_or_else(|| Die("--actions-cache needs KEY:PATH".to_string()))?;
                if key.trim().is_empty() || cache_path.trim().is_empty() {
                    return die("--actions-cache needs a non-empty KEY and PATH");
                }
                actions_cache = Some(value);
            }
            "--actions-cache-restore-key" => {
                i += 1;
                let prefix = need(&rest, i, "--actions-cache-restore-key")?;
                if prefix.trim().is_empty() {
                    return die("--actions-cache-restore-key cannot be empty");
                }
                actions_cache_restore_keys.push(prefix);
            }
            "--allow-paid-blacksmith" => {
                i += 1;
                let v = need(&rest, i, "--allow-paid-blacksmith")?;
                allow_paid = v
                    .parse::<f64>()
                    .map_err(|_| Die(format!("--allow-paid-blacksmith: not a number: {v}")))?;
            }
            "--timeout-minutes" => {
                i += 1;
                let v = need(&rest, i, "--timeout-minutes")?;
                let t = v
                    .parse::<u32>()
                    .ok()
                    .filter(|t| TIMEOUT_MINUTES_RANGE.contains(t))
                    .ok_or_else(|| Die(format!("--timeout-minutes must be 5..=120, got {v}")))?;
                timeout_minutes = Some(t);
            }
            "--laptop-busy" => laptop_busy_flag = true,
            "--assume-owner-load" => {
                i += 1;
                assume_owner_load = need(&rest, i, "--assume-owner-load")?;
            }
            "--assume-blacksmith-used" => {
                i += 1;
                let v = need(&rest, i, "--assume-blacksmith-used")?;
                assume_blacksmith_used =
                    Some(v.parse::<f64>().map_err(|_| {
                        Die(format!("--assume-blacksmith-used: not a number: {v}"))
                    })?);
            }
            "--dispatch" => dispatch = true,
            "-h" | "--help" => {
                println!("{DISPATCH_USAGE}");
                return Ok(0);
            }
            other => return die(format!("dispatch: unknown flag {other}")),
        }
        i += 1;
    }
    if !actions_cache_restore_keys.is_empty() && actions_cache.is_none() {
        return die("--actions-cache-restore-key requires --actions-cache");
    }
    let repo = repo.ok_or_else(|| Die("--repo is required".to_string()))?;
    if !valid_owner_repo(&repo) {
        return die(format!(
            "--repo must be a single owner/repository name, got {repo:?}"
        ));
    }
    let mut job = Job {
        repo,
        command: command.ok_or_else(|| Die("--command is required".to_string()))?,
        git_ref,
        runner,
        platform,
        container,
        surface_pin,
        preferred_surface: None,
        previous_run_id: None,
        previous_run_repo: None,
        push_nix_cache,
        actions_cache,
        actions_cache_restore_keys,
        allow_paid_blacksmith: allow_paid,
        timeout_minutes,
        session_handle: None,
    };
    let registry = registry_value(&cli)?;
    let gh = RealGh::new();
    // #13146: the session mirror to the queue backend. Best-effort
    // throughout — nothing below may fail because it did.
    let remote = remote::session_remote();
    // Reconcile outstanding Blacksmith sessions BEFORE the
    // placement decision (Codex P2 on PR #8325), so the budget
    // figure build_ctx sums reflects real run durations rather
    // than stored zeros whenever a dispatch is being decided.
    if dispatch && assume_blacksmith_used.is_none() {
        let _ = with_store_lock(|| {
            let mut store = load_store();
            let handles = blacksmith_reconcile_handles(&store, &registry);
            for h in &handles {
                let _ = refresh_session(&mut store, h, &registry, &gh);
            }
            if !handles.is_empty() {
                save_store(&store)?;
            }
            Ok(())
        });
    }
    if let Some(handle) = &followup_handle {
        let previous = if parse_portable_run_reference(handle).is_some() {
            resolve_portable_run_reference(handle, &registry, &gh)?
        } else if dispatch {
            with_store_lock(|| {
                let mut store = load_store();
                let previous = refresh_session(&mut store, handle, &registry, &gh)?;
                save_store(&store)?;
                Ok(previous)
            })?
        } else {
            // #13146: the previous session may live only in the
            // backend mirror (dispatched from another seat) — take
            // the newer of the local and backend copies.
            let local = load_store()
                .get("sessions")
                .and_then(Value::as_object)
                .and_then(|sessions| sessions.get(handle))
                .cloned();
            let mirrored = remote.get(handle).ok().flatten();
            remote::pick_session(local, mirrored)
                .ok_or_else(|| Die(format!(
                    "--followup {handle}: no session with this handle in the local or backend Fleet placement store"
                )))?
        };
        let state = previous
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if dispatch
            && previous.get("run_id").and_then(Value::as_i64).is_some()
            && !["succeeded", "failed", "cancelled"].contains(&state)
        {
            return die(format!(
                "--followup {handle}: previous run is still {state}; wait for a terminal result before downloading its output"
            ));
        }
        if dispatch && state == "dispatched" {
            return die(format!(
                "--followup {handle}: previous dispatch has no correlated run yet; resolve its status before continuing"
            ));
        }
        let previous_surface = previous
            .get("surface")
            .and_then(Value::as_str)
            .filter(|surface| !surface.is_empty());
        if let Some(surface_id) = previous_surface {
            job.preferred_surface = Some(surface_id.to_string());
            if let Some(run_id) = previous.get("run_id").and_then(Value::as_i64) {
                let host_repo = previous
                    .get("runner_repo")
                    .and_then(Value::as_str)
                    .filter(|repo| !repo.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        find_surface(&registry, surface_id)
                            .map(|surface| surf_str(surface, "repo").to_string())
                            .filter(|repo| !repo.is_empty())
                    });
                if let Some(host_repo) = host_repo {
                    job.previous_run_id = Some(run_id);
                    job.previous_run_repo = Some(host_repo);
                }
            }
        }
    }
    let store = load_store();
    let ctx = build_ctx(
        &registry,
        &store,
        laptop_busy_flag,
        &assume_owner_load,
        assume_blacksmith_used,
        &gh,
    );
    let (mut adapter, mut reasoning) = decide(&registry, &job, &ctx, &gh)?;
    // #13765: decide() found no surface — consult the coordinator's
    // true view before queuing. A seat with zero coverage (no token
    // for a host) sees no room where the fleet has some; when the
    // coordinator's occupancy shows room, re-decide once against
    // those counts. An unreachable coordinator keeps today's queued
    // path and names the fallback.
    let mut coord_path = "coordinator";
    let mut coord_error: Option<String> = None;
    if adapter.is_none() && dispatch {
        // Preserve the existing dry-run decision/output: only a real dispatch
        // consults the coordinator's true view before it enters claim flow.
        let coord = coordinator::coordinator();
        match coord.load() {
            Ok(load) => {
                let overrides = coordinator::coord_overrides(&load);
                if !overrides.is_empty() {
                    let ctx2 = build_ctx(
                        &registry,
                        &store,
                        laptop_busy_flag,
                        &overrides,
                        assume_blacksmith_used,
                        &gh,
                    );
                    if let Ok((adapter2, mut reasoning2)) = decide(&registry, &job, &ctx2, &gh) {
                        if adapter2.is_some() {
                            reasoning.push(format!(
                                "coordinator true view shows room ({overrides}); re-decided"
                            ));
                            reasoning.append(&mut reasoning2);
                            adapter = adapter2;
                        }
                    }
                }
            }
            Err(e) => {
                coord_path = "singular-fallback";
                coord_error = Some(e);
            }
        }
    }
    let mut out = Map::new();
    out.insert("job".into(), job.to_json_compact());
    out.insert(
        "decision".into(),
        json!(
            adapter
                .as_ref()
                .map(|a| surf_str(a.surface(), "id").to_string())
                .unwrap_or_else(|| "queued".to_string())
        ),
    );
    out.insert("reasoning".into(), json!(reasoning));
    if !ctx.owner_load_overridden.is_empty() {
        out.insert(
            "note".into(),
            json!(format!(
                "owner load OVERRIDDEN for this decision ({}) — simulation, not live counts",
                ctx.owner_load_overridden.join(",")
            )),
        );
    }
    if !ctx.load_read_errors.is_empty() {
        out.insert("load_read_errors".into(), json!(ctx.load_read_errors));
    }
    if !ctx.owner_load_coverage.is_empty() {
        out.insert("owner_load_coverage".into(), owner_load_coverage_json(&ctx));
    }
    if !dispatch {
        out.insert(
            "mode".into(),
            json!("dry-run (no session written; pass --dispatch to submit)"),
        );
        print_out(&Value::Object(out), cli.json, render_decision);
        return Ok(0);
    }
    let handle = new_handle(&rand_source::URandom);
    job.session_handle = Some(handle.clone());
    let coord = coordinator::coordinator();
    let seat = coordinator::seat_name();
    let mut chosen = adapter;
    let mut claim_error = coord_error;
    let mut claim: Option<(coordinator::ClaimDecision, Option<Value>)> = None;
    let mut refused_once = false;
    // #13765 claim-before-dispatch: a chosen surface is claimed from the
    // coordinator first. A refused claim (budget spent) re-decides once
    // with Blacksmith removed and claims the replacement.
    while let Some(ad) = chosen.as_ref() {
        let sid = surf_str(ad.surface(), "id").to_string();
        let answer = coord.claim(
            &handle,
            &sid,
            &seat,
            coordinator::CLAIM_ESTIMATE_MINUTES,
            job.allow_paid_blacksmith,
        );
        let (decision, claim_answer, err) = coordinator::classify_claim(answer);
        if err.is_some() {
            claim_error = err;
        }
        if decision == coordinator::ClaimDecision::Refused && !refused_once {
            refused_once = true;
            reasoning.push(format!(
                "coordinator refused {sid} (budget spent); re-decided without Blacksmith"
            ));
            let reg2 = coordinator::registry_without_blacksmith(&registry);
            let ctx2 = build_ctx(
                &reg2,
                &store,
                laptop_busy_flag,
                &assume_owner_load,
                assume_blacksmith_used,
                &gh,
            );
            let (next, mut reasoning2) = decide(&reg2, &job, &ctx2, &gh)?;
            reasoning.append(&mut reasoning2);
            chosen = next;
            continue;
        }
        claim = Some((decision, claim_answer));
        break;
    }
    let placement_path = match &claim {
        Some((decision, _)) => coordinator::placement_path(decision),
        None => coord_path,
    };
    let mut rec = Map::new();
    rec.insert("handle".into(), json!(handle));
    rec.insert("job".into(), job.to_json());
    rec.insert("placement_path".into(), json!(placement_path));
    if let Some(err) = &claim_error {
        rec.insert("coord_error".into(), json!(err));
    }
    rec.insert("decision_reasoning".into(), json!(reasoning));
    rec.insert("owner_load_coverage".into(), owner_load_coverage_json(&ctx));
    rec.insert("submitted_by".into(), json!(submitted_by()));
    rec.insert(
        "actor".into(),
        json!(dispatch_actor(chosen.as_ref().map(|ad| ad.surface()))),
    );
    rec.insert("created_at".into(), json!(now_iso()));
    rec.insert("updated_at".into(), json!(now_iso()));
    match (chosen, claim) {
        (Some(ad), Some((coordinator::ClaimDecision::Queued, claim_answer))) => {
            let sid = surf_str(ad.surface(), "id").to_string();
            let position = claim_answer
                .as_ref()
                .and_then(|v| v.get("position"))
                .and_then(|p| p.as_i64());
            let mut queued = coordinator::queued_session_record(
                &handle,
                job.to_json(),
                &sid,
                &handle,
                &reasoning,
                owner_load_coverage_json(&ctx),
                &submitted_by(),
                dispatch_actor(Some(ad.surface())),
                &now_iso(),
            );
            if let Some(obj) = queued.as_object_mut() {
                if let Some(err) = &claim_error {
                    obj.insert("coord_error".into(), json!(err));
                }
            }
            commit_new_session(&queued, None, &*remote)?;
            out.insert("handle".into(), json!(handle));
            out.insert("state".into(), json!("queued"));
            out.insert("placement_path".into(), json!(placement_path));
            if let Some(pos) = position {
                out.insert("queue_position".into(), json!(pos));
            }
        }
        (Some(ad), Some((decision, _))) if coordinator::dispatch_now(&decision) => {
            let granted = decision == coordinator::ClaimDecision::Granted;
            let result = match ad.dispatch(&job, &gh) {
                Ok(r) => r,
                Err(e) => {
                    if granted {
                        let _ = coord.release(&handle, None);
                    }
                    return Err(e);
                }
            };
            let surface = ad.surface().clone();
            let sid = surf_str(&surface, "id").to_string();
            let kind = surf_str(&surface, "kind").to_string();
            let runner_repo = surf_str(&surface, "repo").to_string();
            rec.insert("surface".into(), json!(sid));
            rec.insert("surface_kind".into(), json!(kind));
            if !runner_repo.is_empty() {
                rec.insert("runner_repo".into(), json!(runner_repo.clone()));
            }
            if let Some(run_id) = result.run_id {
                rec.insert(
                    "portable_id".into(),
                    json!(format!("{runner_repo}#{run_id}")),
                );
            }
            if granted {
                rec.insert("claim_handle".into(), json!(handle));
            }
            rec.insert(
                "run_id".into(),
                result.run_id.map(Value::from).unwrap_or(Value::Null),
            );
            rec.insert(
                "run_url".into(),
                result
                    .run_url
                    .clone()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            rec.insert(
                "state".into(),
                json!(match &result.terminal_state {
                    Some(s) => s.clone(),
                    None => {
                        if result.run_id.is_some() {
                            "queued".to_string()
                        } else {
                            "dispatched".to_string()
                        }
                    }
                }),
            );
            if granted {
                if let Err(e) = coord.confirm(&handle, result.run_id) {
                    rec.insert("confirm_error".into(), json!(e));
                }
            }
            if let Some(note) = &result.note {
                rec.insert("note".into(), json!(note));
            }
            if let Some(cs) = &result.codespace {
                rec.insert("codespace".into(), json!(cs));
            }
            let blacksmith_entry = if kind == "blacksmith" {
                Some(json!({
                    "handle": handle,
                    "month": current_month(),
                    "minutes": 0.0,
                    "run_id": result.run_id,
                    "note": "minutes reconciled from the run's duration on later invocations",
                    "reconciled": false,
                }))
            } else {
                None
            };
            commit_new_session(&Value::Object(rec), blacksmith_entry.as_ref(), &*remote)?;
            out.insert("handle".into(), json!(handle));
            out.insert("placement_path".into(), json!(placement_path));
            if let Some(run_id) = result.run_id {
                if !runner_repo.is_empty() {
                    out.insert("runner_repo".into(), json!(runner_repo));
                    out.insert(
                        "portable_id".into(),
                        json!(format!("{runner_repo}#{run_id}")),
                    );
                }
            }
            out.insert(
                "run_id".into(),
                result.run_id.map(Value::from).unwrap_or(Value::Null),
            );
            out.insert(
                "run_url".into(),
                result.run_url.map(Value::from).unwrap_or(Value::Null),
            );
        }
        _ => {
            rec.insert("surface".into(), Value::Null);
            rec.insert("run_id".into(), Value::Null);
            rec.insert("run_url".into(), Value::Null);
            rec.insert("state".into(), json!("queued"));
            commit_new_session(&Value::Object(rec), None, &*remote)?;
            out.insert("handle".into(), json!(handle));
            out.insert("state".into(), json!("queued"));
            out.insert("placement_path".into(), json!(placement_path));
        }
    }
    // #13146: opportunistic migration — push any local records the
    // backend still lacks (insert-or-ignore there; idempotent).
    remote::sync_local_sessions(&*remote, &load_store());
    print_out(&Value::Object(out), cli.json, render_decision);
    Ok(0)
}

const SESSION_USAGE: &str = "usage: repo-queue session <status|result|list> [HANDLE|OWNER/REPO#RUN_ID] \
[--mine] [--json] [--registry PATH]";

/// True when a session subcommand's tail asks for help. A help flag
/// is never a session handle: `session status --help` used to look
/// up a session literally named `--help` and error (#13175).
fn session_help_requested(sub: &str, rest: &[String]) -> bool {
    matches!(sub, "status" | "result" | "list")
        && rest
            .get(1..)
            .is_some_and(|tail| tail.iter().any(|a| matches!(a.as_str(), "--help" | "-h")))
}

fn run_session(args: &[String]) -> R<i32> {
    let (cli, rest) = extract_common(args)?;
    let sub = rest.first().map(|s| s.as_str()).unwrap_or("");
    if session_help_requested(sub, &rest) {
        println!("{SESSION_USAGE}");
        return Ok(0);
    }
    let gh = RealGh::new();
    // #13146: the session mirror to the queue backend (best-effort).
    let remote = remote::session_remote();
    match sub {
        "status" | "result" => {
            let handle = rest
                .get(1)
                .cloned()
                .ok_or_else(|| Die(format!("session {sub} needs a HANDLE")))?;
            let registry = registry_value(&cli)?;
            let rec = if parse_portable_run_reference(&handle).is_some() {
                resolve_portable_run_reference(&handle, &registry, &gh)?
            } else {
                // #13146: seed the local store from the backend
                // mirror when it holds the newer copy (or the only
                // copy — a session dispatched from another seat),
                // refresh as usual, and mirror the refreshed record
                // back. Any mirror failure leaves today's local-only
                // behavior untouched.
                let mirrored = remote.get(&handle).ok().flatten();
                let rec = with_store_lock(|| {
                    let mut store = load_store();
                    if let Some(m) = &mirrored {
                        let local = store.get("sessions").and_then(|s| s.get(&handle)).cloned();
                        if let Some(winner) = remote::pick_session(local, Some(m.clone())) {
                            if let Some(sessions) = store["sessions"].as_object_mut() {
                                sessions.insert(handle.clone(), winner);
                            }
                        }
                    }
                    let rec = refresh_session(&mut store, &handle, &registry, &gh)?;
                    save_store(&store)?;
                    Ok(rec)
                })?;
                let _ = remote.put(&rec);
                rec
            };
            if sub == "status" {
                print_out(&rec, cli.json, |r| {
                    format!(
                        "{}: state={} surface={} run={}",
                        r.get("handle").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("surface")
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "null".to_string()),
                        r.get("run_url").and_then(|v| v.as_str()).unwrap_or("None"),
                    )
                });
            } else {
                let terminal = ["succeeded", "failed", "cancelled"]
                    .contains(&rec.get("state").and_then(|s| s.as_str()).unwrap_or(""));
                let mut out = rec.as_object().cloned().unwrap_or_default();
                out.insert("terminal".into(), json!(terminal));
                if !terminal {
                    out.insert(
                        "note".into(),
                        json!("not terminal yet — reconnect with the same handle"),
                    );
                }
                print_out(&Value::Object(out), cli.json, |r| {
                    format!(
                        "{}: {} (terminal={}) run={}",
                        r.get("handle").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("terminal").and_then(|v| v.as_bool()).unwrap_or(false),
                        r.get("run_url").and_then(|v| v.as_str()).unwrap_or("None"),
                    )
                });
            }
            Ok(0)
        }
        "list" => {
            let mine = rest.iter().any(|a| a == "--mine");
            let store = load_store();
            let me = submitted_by();
            let local_sessions: Vec<Value> = store
                .get("sessions")
                .and_then(|s| s.as_object())
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default();
            // #13146: union in the backend mirror's records (a
            // failed mirror read lists local-only, as before), then
            // take the chance to migrate local records upward.
            let mirrored_sessions = remote
                .list(if mine { Some(me.as_str()) } else { None }, 200)
                .unwrap_or_default();
            let mut sessions = remote::merge_session_records(local_sessions, mirrored_sessions);
            remote::sync_local_sessions(&*remote, &store);
            sessions.sort_by(|a, b| {
                b.get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(a.get("created_at").and_then(|v| v.as_str()).unwrap_or(""))
            });
            if mine {
                sessions.retain(|s| {
                    s.get("submitted_by").and_then(|v| v.as_str()) == Some(me.as_str())
                });
            }
            let out = json!({ "sessions": sessions });
            print_out(&out, cli.json, |o| {
                let arr = o
                    .get("sessions")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default();
                if arr.is_empty() {
                    return "no sessions".to_string();
                }
                arr.iter()
                    .map(|r| {
                        format!(
                            "{}: state={} surface={} run={}",
                            r.get("handle").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("surface")
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "null".to_string()),
                            r.get("run_url").and_then(|v| v.as_str()).unwrap_or("None"),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            Ok(0)
        }
        "-h" | "--help" | "" => {
            println!("{SESSION_USAGE}");
            Ok(if sub.is_empty() { 2 } else { 0 })
        }
        other => die(format!("session: unknown subcommand {other}")),
    }
}

const SURFACES_USAGE: &str = "usage: repo-queue surfaces [--json] [--registry PATH]\n\nPrints the placement surface registry as JSON.";

fn run_surfaces(args: &[String]) -> R<i32> {
    let (cli, rest) = extract_common(args)?;
    if rest.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        println!("{SURFACES_USAGE}");
        return Ok(0);
    }
    let registry = registry_value(&cli)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&registry).unwrap_or_else(|_| "null".to_string())
    );
    Ok(0)
}

fn report(result: R<i32>) -> i32 {
    match result {
        Ok(code) => code,
        Err(Die(msg)) => {
            eprintln!("repo-queue: {msg}");
            2
        }
    }
}

pub fn cmd_dispatch(args: &[String]) -> i32 {
    report(run_dispatch(args))
}

pub fn cmd_session(args: &[String]) -> i32 {
    report(run_session(args))
}

pub fn cmd_surfaces(args: &[String]) -> i32 {
    report(run_surfaces(args))
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map2;

    const FIXED_NOW: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

    /// Tests that set the process-wide FLEET_PLACE_STATE variable
    /// must not run concurrently (parallel test threads would
    /// swap each other's state dir, and one test removing the
    /// variable would drop the other into the real user store).
    static STORE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct FakeGh {
        tokens_ok: bool,
        workflow_runs: Vec<Value>,
        post_runs: Vec<Value>,
        runs_by_id: Map2<i64, Value>,
        load_by_repo: Map2<String, (i64, i64)>, // repo -> (in_progress, queued)
        load_replies: Map2<(String, String), (u16, Value)>, // (repo, status) -> canned load reply; repo "*" matches any
        codespace_ok: bool,
        codespace_items: Vec<Value>,
        codespace_exit: i32,
        posts: RefCell<Vec<(String, Value)>>,
        user_token: Option<String>,
        post_tokens: RefCell<Vec<String>>,
    }

    impl Default for FakeGh {
        fn default() -> Self {
            FakeGh {
                tokens_ok: true,
                workflow_runs: Vec::new(),
                post_runs: Vec::new(),
                runs_by_id: Map2::new(),
                load_by_repo: Map2::new(),
                load_replies: Map2::new(),
                codespace_ok: false,
                codespace_items: Vec::new(),
                codespace_exit: 0,
                posts: RefCell::new(Vec::new()),
                user_token: None,
                post_tokens: RefCell::new(Vec::new()),
            }
        }
    }

    impl Gh for FakeGh {
        fn mint_token(&self, _repo: &str) -> Option<String> {
            if self.tokens_ok {
                Some("installation-token".to_string())
            } else {
                None
            }
        }
        fn user_token(&self) -> Option<String> {
            self.user_token.clone()
        }
        fn api(
            &self,
            method: &str,
            path: &str,
            token: &str,
            body: Option<&Value>,
        ) -> R<(u16, Value)> {
            if method == "POST" {
                self.post_tokens.borrow_mut().push(token.to_string());
                self.posts
                    .borrow_mut()
                    .push((path.to_string(), body.cloned().unwrap_or(Value::Null)));
                return Ok((204, json!({})));
            }
            if path.contains("/runs?event=workflow_dispatch") {
                // Runs created by a dispatch (post_runs) become
                // visible only after this fake has seen a POST, so
                // the pre-dispatch snapshot in dispatch_workflow
                // behaves like the real API.
                let mut runs = self.workflow_runs.clone();
                if !self.posts.borrow().is_empty() {
                    runs.extend(self.post_runs.iter().cloned());
                    // Simulate run-name surfacing (issue #8371): a
                    // workflow that received a dispatch_id input
                    // shows it in the display_title of the run it
                    // created, so post_runs without a title get the
                    // id from the recorded POST.
                    let posted_id = self.posts.borrow().last().and_then(|(_, b)| {
                        b.get("inputs")
                            .and_then(|i| i.get("dispatch_id"))
                            .and_then(|d| d.as_str())
                            .map(|s| s.to_string())
                    });
                    if let Some(did) = posted_id {
                        let pre = self.workflow_runs.len();
                        for r in runs.iter_mut().skip(pre) {
                            if r.get("display_title").is_none() {
                                if let Some(obj) = r.as_object_mut() {
                                    obj.insert(
                                        "display_title".into(),
                                        json!(format!("fleet-run {did}")),
                                    );
                                }
                            }
                        }
                    }
                }
                return Ok((200, json!({ "workflow_runs": runs })));
            }
            if path.contains("/actions/runs?status=") {
                let repo = path
                    .trim_start_matches("/repos/")
                    .split("/actions/")
                    .next()
                    .unwrap_or("")
                    .to_string();
                let status_name = if path.contains("status=in_progress") {
                    "in_progress"
                } else {
                    "queued"
                };
                if let Some((code, payload)) = self
                    .load_replies
                    .get(&(repo.clone(), status_name.to_string()))
                    .or_else(|| {
                        self.load_replies
                            .get(&("*".to_string(), status_name.to_string()))
                    })
                {
                    return Ok((*code, payload.clone()));
                }
                let (running, queued) = self.load_by_repo.get(&repo).copied().unwrap_or((0, 0));
                let total = if path.contains("status=in_progress") {
                    running
                } else {
                    queued
                };
                return Ok((200, json!({ "total_count": total })));
            }
            if let Some(id_str) = path.rsplit('/').next() {
                if let Ok(id) = id_str.parse::<i64>() {
                    if let Some(run) = self.runs_by_id.get(&id) {
                        return Ok((200, run.clone()));
                    }
                }
            }
            Ok((404, json!({ "message": "not found" })))
        }
        fn codespace_list(&self) -> R<Vec<Value>> {
            if self.codespace_ok {
                Ok(self.codespace_items.clone())
            } else {
                die("403 scope")
            }
        }
        fn codespace_ssh(&self, _name: &str, _command: &str) -> R<i32> {
            Ok(self.codespace_exit)
        }
        fn sleep_secs(&self, _secs: u64) {}
        fn now_epoch(&self) -> i64 {
            FIXED_NOW
        }
    }

    fn registry() -> Value {
        json!({
            "version": 1,
            "laptop": { "busy": false, "busy_source": "manual-flag" },
            "owners": {
                "developing-today": { "concurrent_jobs_cap": 20, "plan": "free-org" },
                "dezren39": { "concurrent_jobs_cap": 40, "plan": "pro" }
            },
            "surfaces": [
                { "id": "fleet-personal", "kind": "github-actions", "owner": "dezren39",
                  "repo": "dezren39/fleet", "workflow": "fleet-run.yml",
                  "dispatch_id_input": true,
                  "app_installation": 175747567, "status": "active",
                  "targets": ["developing-today/code"] },
                { "id": "org-run", "kind": "github-actions", "owner": "developing-today",
                  "repo": "developing-today/code", "workflow": "org-run.yml",
                  "dispatch_id_input": true,
                  "app_installation": 174792676, "status": "active",
                  "targets": ["developing-today/code", "developing-today/opencode"] },
                { "id": "fleet-org-blacksmith", "kind": "blacksmith", "owner": "developing-today",
                  "repo": "developing-today/fleet", "workflow": "fleet-run.yml",
                  "app_installation": 174792676, "status": "active",
                  "budget": { "free_minutes_monthly": 3000, "paid_cap_minutes": 0 },
                  "targets": ["developing-today/code"] },
                { "id": "codespace-overflow", "kind": "codespace", "owner": "dezren39",
                  "repo": null, "workflow": null, "status": "pending-scope",
                  "budget": { "core_hours_monthly": 180 }, "max_live": 3,
                  "targets": ["developing-today/code"] },
                { "id": "laptop", "kind": "laptop", "owner": "dezren39",
                  "repo": null, "workflow": null, "status": "handoff-only",
                  "targets": ["*"] }
            ]
        })
    }

    fn job() -> Job {
        Job {
            repo: "developing-today/code".to_string(),
            command: "nix build .#mcpx".to_string(),
            git_ref: String::new(),
            runner: "auto".to_string(),
            platform: "linux".to_string(),
            container: None,
            surface_pin: None,
            preferred_surface: None,
            previous_run_id: None,
            previous_run_repo: None,
            push_nix_cache: false,
            actions_cache: None,
            actions_cache_restore_keys: vec![],
            allow_paid_blacksmith: 0.0,
            timeout_minutes: None,
            session_handle: None,
        }
    }

    fn ctx(load: &[(&str, (i64, i64))], blacksmith_used: f64, laptop_busy: bool) -> Ctx {
        let owners = registry()["owners"].clone();
        let owner_load: HashMap<String, (i64, i64)> =
            load.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        // Fixtures state load as fact, so coverage is measured for
        // every registry owner (defaulting to an observed zero).
        let owner_load_coverage = owners
            .as_object()
            .unwrap()
            .iter()
            .map(|(owner, o)| {
                let (running, queued) = owner_load.get(owner).copied().unwrap_or((0, 0));
                (
                    owner.clone(),
                    LoadCoverage {
                        status: LoadCoverageStatus::Measured,
                        known_running: running,
                        known_queued: queued,
                        cap: o
                            .get("concurrent_jobs_cap")
                            .and_then(Value::as_i64)
                            .unwrap_or(0),
                        read_at: FIXED_NOW,
                        source: "test fixture".to_string(),
                        missing_reads: vec![],
                    },
                )
            })
            .collect();
        Ctx {
            owners,
            owner_load,
            owner_load_coverage,
            owner_last_dispatch: HashMap::new(),
            surface_last_dispatch: HashMap::new(),
            owner_load_overridden: vec![],
            load_read_errors: vec![],
            laptop_busy,
            laptop_busy_source: "test".to_string(),
            blacksmith_used_minutes: blacksmith_used,
        }
    }

    fn pick(reg: &Value, job: &Job, ctx: &Ctx, gh: &FakeGh) -> Option<String> {
        let (ad, _reasoning) = decide(reg, job, ctx, gh).unwrap();
        ad.map(|a| surf_str(a.surface(), "id").to_string())
    }

    #[test]
    fn tier1_balances_load_and_rotates_idle_owners() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut c = ctx(
            &[("dezren39", (0, 0)), ("developing-today", (0, 0))],
            0.0,
            false,
        );
        // A cold registry tie has a deterministic first choice.
        assert_eq!(
            pick(&reg, &job(), &c, &gh).as_deref(),
            Some("fleet-personal")
        );
        // Thereafter idle owners rotate even though their plan caps differ.
        c.owner_last_dispatch.insert("dezren39".to_string(), 100);
        assert_eq!(pick(&reg, &job(), &c, &gh).as_deref(), Some("org-run"));
        c.owner_last_dispatch
            .insert("developing-today".to_string(), 200);
        assert_eq!(
            pick(&reg, &job(), &c, &gh).as_deref(),
            Some("fleet-personal")
        );

        // Live load beats the cap-size bias: the less-loaded account wins.
        let c = ctx(
            &[("dezren39", (5, 0)), ("developing-today", (1, 0))],
            0.0,
            false,
        );
        assert_eq!(pick(&reg, &job(), &c, &gh).as_deref(), Some("org-run"));
        // A full personal account does not block the organization.
        let c = ctx(
            &[("dezren39", (40, 0)), ("developing-today", (5, 0))],
            0.0,
            false,
        );
        assert_eq!(pick(&reg, &job(), &c, &gh).as_deref(), Some("org-run"));
    }

    #[test]
    fn tier1_target_restriction_skips_org_run_for_other_repos() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.repo = "developing-today/opencode".to_string();
        let c = ctx(
            &[("dezren39", (0, 0)), ("developing-today", (0, 0))],
            0.0,
            false,
        );
        // fleet-personal does not target opencode; org-run does.
        assert_eq!(pick(&reg, &j, &c, &gh).as_deref(), Some("org-run"));
    }

    #[test]
    fn tier2_blacksmith_inside_budget() {
        let reg = registry();
        let gh = FakeGh::default();
        let c = ctx(
            &[("dezren39", (40, 0)), ("developing-today", (20, 0))],
            100.0,
            false,
        );
        assert_eq!(
            pick(&reg, &job(), &c, &gh).as_deref(),
            Some("fleet-org-blacksmith")
        );
    }

    #[test]
    fn blacksmith_budget_spent_stays_queued_without_laptop_saturation() {
        let reg = registry();
        let gh = FakeGh {
            codespace_ok: true,
            ..Default::default()
        };
        let c = ctx(
            &[("dezren39", (40, 0)), ("developing-today", (20, 0))],
            3000.0,
            false,
        );
        let (ad, reasoning) = decide(&reg, &job(), &c, &gh).unwrap();
        assert!(ad.is_none());
        assert!(
            reasoning
                .last()
                .map(|r| r.contains("stays queued"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn codespace_is_last_resort_when_laptop_saturated() {
        let reg = registry();
        let gh = FakeGh {
            codespace_ok: true,
            ..Default::default()
        };
        let c = ctx(
            &[("dezren39", (40, 0)), ("developing-today", (20, 0))],
            3000.0,
            true,
        );
        assert_eq!(
            pick(&reg, &job(), &c, &gh).as_deref(),
            Some("codespace-overflow")
        );
        // Scope missing -> the tier is not reachable at all.
        let gh2 = FakeGh::default();
        assert_eq!(pick(&reg, &job(), &c, &gh2), None);
    }

    #[test]
    fn paid_blacksmith_allowance_reopens_tier2() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.allow_paid_blacksmith = 250.0;
        let c = ctx(
            &[("dezren39", (40, 0)), ("developing-today", (20, 0))],
            3000.0,
            false,
        );
        assert_eq!(
            pick(&reg, &j, &c, &gh).as_deref(),
            Some("fleet-org-blacksmith")
        );
    }

    #[test]
    fn runner_pin_blacksmith_skips_tier1() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.runner = "blacksmith".to_string();
        let c = ctx(
            &[("dezren39", (0, 0)), ("developing-today", (0, 0))],
            0.0,
            false,
        );
        assert_eq!(
            pick(&reg, &j, &c, &gh).as_deref(),
            Some("fleet-org-blacksmith")
        );
    }

    #[test]
    fn pinned_surface_ineligible_is_loud() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.runner = "github".to_string();
        j.surface_pin = Some("fleet-org-blacksmith".to_string());
        let c = ctx(&[], 0.0, false);
        let err = match decide(&reg, &j, &c, &gh) {
            Ok(_) => panic!("pinned ineligible surface must die"),
            Err(e) => e,
        };
        assert!(err.0.contains("ineligible"), "{}", err.0);
        let mut j2 = job();
        j2.surface_pin = Some("nope".to_string());
        let err2 = match decide(&reg, &j2, &c, &gh) {
            Ok(_) => panic!("unknown pin must die"),
            Err(e) => e,
        };
        assert!(err2.0.contains("no such surface"), "{}", err2.0);
    }

    #[test]
    fn pinned_blacksmith_budget_spent_is_ineligible() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.surface_pin = Some("fleet-org-blacksmith".to_string());
        let c = ctx(&[], 3000.0, false);
        let err = match decide(&reg, &j, &c, &gh) {
            Ok(_) => panic!("pinned blacksmith with spent budget must die"),
            Err(e) => e,
        };
        assert!(err.0.contains("budget spent"), "{}", err.0);
        j.allow_paid_blacksmith = 250.0;
        let (ad, _r) = decide(&reg, &j, &c, &gh).unwrap();
        assert_eq!(
            surf_str(ad.unwrap().surface(), "id"),
            "fleet-org-blacksmith"
        );
    }

    #[test]
    fn pinned_laptop_records_handoff() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.surface_pin = Some("laptop".to_string());
        let c = ctx(&[], 0.0, false);
        let (ad, _r) = decide(&reg, &j, &c, &gh).unwrap();
        let ad = ad.unwrap();
        assert_eq!(surf_str(ad.surface(), "id"), "laptop");
        let result = ad.dispatch(&j, &gh).unwrap();
        assert!(
            result
                .note
                .as_deref()
                .unwrap_or("")
                .contains("handoff recorded")
        );
    }

    #[test]
    fn workflow_inputs_shapes() {
        let reg = registry();
        let surface = find_surface(&reg, "fleet-personal").unwrap();
        let ad = GithubActionsAdapter {
            surface: surface.clone(),
            force_runner: "github",
        };
        // Plain job: bare repo/command inputs, no runner, no jobs.
        let inputs = ad.workflow_inputs(&job(), None);
        assert_eq!(inputs["repo"], json!("developing-today/code"));
        assert_eq!(inputs["command"], json!("nix build .#mcpx"));
        assert!(inputs.get("jobs").is_none());
        assert!(inputs.get("runner").is_none());
        // Cache options travel in a jobs entry.
        let mut j = job();
        j.push_nix_cache = true;
        j.actions_cache = Some("tests:./out".to_string());
        j.git_ref = "abc123".to_string();
        let inputs = ad.workflow_inputs(&j, None);
        let jobs: Value = serde_json::from_str(inputs["jobs"].as_str().unwrap()).unwrap();
        assert_eq!(jobs[0]["cache"]["push_nix"], json!(true));
        assert_eq!(jobs[0]["cache"]["actions_key"], json!("tests"));
        assert_eq!(jobs[0]["cache"]["actions_path"], json!("./out"));
        assert_eq!(inputs["ref"], json!("abc123"));
        // The jobs form retains the required top-level repo input.
        assert_eq!(inputs["repo"], json!("developing-today/code"));
        // org-run dispatches pin setup_runner to the dispatch's
        // own provider (its setup job otherwise defaults to
        // Blacksmith).
        let org_surface = find_surface(&reg, "org-run").unwrap();
        let org_ad = GithubActionsAdapter {
            surface: org_surface.clone(),
            force_runner: "github",
        };
        let inputs = org_ad.workflow_inputs(&job(), None);
        assert_eq!(inputs["setup_runner"], json!("github"));
        // Blacksmith variant sends the runner input.
        let bs_surface = find_surface(&reg, "fleet-org-blacksmith").unwrap();
        let bs = GithubActionsAdapter {
            surface: bs_surface.clone(),
            force_runner: "blacksmith",
        };
        let inputs = bs.workflow_inputs(&job(), None);
        assert_eq!(inputs["runner"], json!("blacksmith"));
        // A dispatch_id is sent only when the caller generated one
        // for a surface whose workflow declares the input (#8371).
        let inputs = ad.workflow_inputs(&job(), Some("fp_abc123"));
        assert_eq!(inputs["dispatch_id"], json!("fp_abc123"));
        let inputs = ad.workflow_inputs(&job(), None);
        assert!(inputs.get("dispatch_id").is_none());
    }

    #[test]
    fn dispatch_posts_as_app_and_finds_run() {
        let reg = registry();
        let surface = find_surface(&reg, "fleet-personal").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            tokens_ok: true,
            post_runs: vec![json!({
                "id": 4242,
                "html_url": "https://github.com/dezren39/fleet/actions/runs/4242",
                "created_at": "2026-01-01T00:00:10Z"
            })],
            ..Default::default()
        };
        let result = ad.dispatch(&job(), &gh).unwrap();
        assert_eq!(result.run_id, Some(4242));
        let posts = gh.posts.borrow();
        let posted_id = posts[0].1["inputs"]["dispatch_id"].as_str().unwrap();
        assert!(
            posted_id.starts_with("fp_"),
            "dispatch_id posted: {posted_id}"
        );
        assert_eq!(posts.len(), 1);
        assert!(
            posts[0]
                .0
                .contains("/repos/dezren39/fleet/actions/workflows/fleet-run.yml/dispatches")
        );
    }

    #[test]
    fn map_run_state_covers_vocabulary() {
        let case = |status: &str, conclusion: Option<&str>| {
            map_run_state(&json!({ "status": status, "conclusion": conclusion }))
        };
        assert_eq!(case("queued", None), "queued");
        assert_eq!(case("waiting", None), "queued");
        assert_eq!(case("in_progress", None), "running");
        assert_eq!(case("completed", Some("success")), "succeeded");
        assert_eq!(case("completed", Some("skipped")), "succeeded");
        assert_eq!(case("completed", Some("failure")), "failed");
        assert_eq!(case("completed", Some("timed_out")), "failed");
        assert_eq!(case("completed", Some("cancelled")), "cancelled");
        assert_eq!(case("completed", None), "failed");
        assert_eq!(case("something-else", None), "unknown");
    }

    struct FixedRand;
    impl rand_source::RandSource for FixedRand {
        fn bytes16(&self) -> [u8; 16] {
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        }
    }

    #[test]
    fn handle_is_fp_plus_128_bit_hex() {
        assert_eq!(
            new_handle(&FixedRand),
            "fp_000102030405060708090a0b0c0d0e0f"
        );
    }

    #[test]
    fn urandom_handles_return_and_differ() {
        // Regression: /dev/urandom is an endless stream — reading it
        // to end-of-file spins forever consuming memory. Reading
        // exactly 16 bytes returns immediately.
        let a = new_handle(&rand_source::URandom);
        let b = new_handle(&rand_source::URandom);
        assert_eq!(a.len(), 35);
        assert!(a.starts_with("fp_"));
        assert_ne!(a, b);
    }

    #[test]
    fn iso_time_roundtrips() {
        assert_eq!(iso_from_epoch(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(parse_iso("2026-01-01T00:00:00Z"), Some(1_767_225_600));
        assert_eq!(
            parse_iso(&iso_from_epoch(1_762_000_000)),
            Some(1_762_000_000)
        );
        assert_eq!(parse_iso("not a date"), None);
    }

    #[test]
    fn blacksmith_capacity_reports_remaining_budget() {
        let reg = registry();
        let surface = find_surface(&reg, "fleet-org-blacksmith").unwrap();
        let ad = adapter_for(surface).unwrap();
        let mut j = job();
        j.allow_paid_blacksmith = 100.0;
        let c = ctx(&[], 250.0, false);
        let cap = ad.capacity(&j, &c);
        assert_eq!(cap.remaining_total_minutes, Some(2850.0));
    }

    #[test]
    fn build_ctx_sums_current_month_blacksmith_minutes() {
        let reg = registry();
        let month = current_month();
        let store = json!({
            "sessions": {},
            "blacksmith_dispatches": [
                { "handle": "fp_a", "month": month, "minutes": 12.5 },
                { "handle": "fp_b", "month": month, "minutes": 2.5 },
                { "handle": "fp_old", "month": "1999-01", "minutes": 900.0 }
            ]
        });
        let gh = FakeGh {
            tokens_ok: false,
            ..Default::default()
        }; // no tokens: load reads error out, counted 0
        let c = build_ctx(&reg, &store, false, "", None, &gh);
        assert_eq!(c.blacksmith_used_minutes, 15.0);
        assert!(!c.load_read_errors.is_empty());
        // Override path replaces live counting.
        let c2 = build_ctx(&reg, &store, false, "dezren39=40", Some(7.0), &gh);
        assert_eq!(c2.owner_load.get("dezren39"), Some(&(40, 0)));
        assert_eq!(c2.blacksmith_used_minutes, 7.0);
        assert_eq!(c2.owner_load_overridden, vec!["dezren39".to_string()]);
    }

    #[test]
    fn dispatch_id_correlates_exactly_with_interleaved_runs() {
        // Issue #8371 acceptance: another dispatch of the same
        // workflow lands in the same poll window, EARLIER than
        // ours, carrying a different dispatch_id in its title.
        // Snapshot exclusion alone would attach that run; the
        // display-title match must attach ours instead.
        let reg = registry();
        let surface = find_surface(&reg, "fleet-personal").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            tokens_ok: true,
            post_runs: vec![
                json!({
                    "id": 7001,
                    "html_url": "https://github.com/dezren39/fleet/actions/runs/7001",
                    "created_at": "2026-01-01T00:00:08Z",
                    "display_title": "fleet-run fp_ffffffffffffffffffffffffffffffff"
                }),
                // Ours: no title in the fixture - FakeGh fills it
                // from the recorded POST, like run-name does.
                json!({
                    "id": 7002,
                    "html_url": "https://github.com/dezren39/fleet/actions/runs/7002",
                    "created_at": "2026-01-01T00:00:20Z"
                }),
            ],
            ..Default::default()
        };
        let result = ad.dispatch(&job(), &gh).unwrap();
        assert_eq!(result.run_id, Some(7002));
    }

    #[test]
    fn dispatch_id_with_only_a_foreign_run_stays_unresolved() {
        // With an id in play, a same-window run that is not ours
        // must never be attached; the dispatch stays
        // accepted-but-unresolved instead.
        let reg = registry();
        let surface = find_surface(&reg, "fleet-personal").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            tokens_ok: true,
            post_runs: vec![json!({
                "id": 8001,
                "html_url": "https://github.com/dezren39/fleet/actions/runs/8001",
                "created_at": "2026-01-01T00:00:10Z",
                "display_title": "fleet-run fp_ffffffffffffffffffffffffffffffff"
            })],
            ..Default::default()
        };
        let result = ad.dispatch(&job(), &gh).unwrap();
        assert_eq!(result.run_id, None);
        assert!(result.note.unwrap().contains("dispatch accepted"));
    }

    #[test]
    fn correlate_run_fallback_and_id_match() {
        let before: std::collections::HashSet<i64> = [1111].into_iter().collect();
        let runs = vec![
            json!({"id": 1111, "created_at": "2026-01-01T00:00:10Z", "html_url": "u1"}),
            json!({"id": 2222, "created_at": "2026-01-01T00:00:20Z", "html_url": "u2",
                    "display_title": "fleet-run fp_aaaa"}),
            json!({"id": 3333, "created_at": "2026-01-01T00:00:08Z", "html_url": "u3",
                    "display_title": "fleet-run fp_bbbb"}),
        ];
        let since = FIXED_NOW - 5;
        // Fallback: earliest new run.
        assert_eq!(
            correlate_run(&runs, &before, since, None),
            Some((3333, "u3".to_string()))
        );
        // Id match: the run carrying the id, not the earliest.
        assert_eq!(
            correlate_run(&runs, &before, since, Some("fp_aaaa")),
            Some((2222, "u2".to_string()))
        );
        // Id with no matching run: no candidate.
        assert_eq!(correlate_run(&runs, &before, since, Some("fp_zzzz")), None);
    }

    #[test]
    fn dispatch_ignores_preexisting_run_and_picks_earliest_new_run() {
        // Fallback path (issue #8371): a surface whose workflow
        // declares no dispatch_id input keeps snapshot exclusion.
        let reg = registry();
        let surface = find_surface(&reg, "fleet-org-blacksmith").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            tokens_ok: true,
            // A run from another job, created inside the time
            // window: the pre-dispatch snapshot must exclude it.
            workflow_runs: vec![json!({
                "id": 1111,
                "html_url": "https://github.com/dezren39/fleet/actions/runs/1111",
                "created_at": "2026-01-01T00:00:10Z"
            })],
            // Two runs appear after our dispatch; the earliest is
            // ours, the later one is a concurrent submission.
            post_runs: vec![
                json!({
                    "id": 2222,
                    "html_url": "https://github.com/dezren39/fleet/actions/runs/2222",
                    "created_at": "2026-01-01T00:00:20Z"
                }),
                json!({
                    "id": 3333,
                    "html_url": "https://github.com/dezren39/fleet/actions/runs/3333",
                    "created_at": "2026-01-01T00:00:08Z"
                }),
            ],
            ..Default::default()
        };
        let result = ad.dispatch(&job(), &gh).unwrap();
        assert_eq!(result.run_id, Some(3333));
    }

    #[test]
    fn surface_ineligible_when_app_token_cannot_be_minted() {
        let reg = registry();
        let gh = FakeGh {
            tokens_ok: false,
            ..Default::default()
        };
        let c = ctx(
            &[("dezren39", (0, 0)), ("developing-today", (0, 0))],
            0.0,
            false,
        );
        let picked = pick(&reg, &job(), &c, &gh);
        assert_eq!(picked, None);
    }

    #[test]
    fn codespace_dispatch_records_terminal_state_from_ssh_exit() {
        let reg = registry();
        let surface = find_surface(&reg, "codespace-overflow").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let mk = |exit: i32| FakeGh {
            codespace_ok: true,
            codespace_exit: exit,
            codespace_items: vec![json!({
                "name": "cs-1",
                "repository": "developing-today/code",
                "state": "Available"
            })],
            ..Default::default()
        };
        let ok = ad.dispatch(&job(), &mk(0)).unwrap();
        assert_eq!(ok.terminal_state.as_deref(), Some("succeeded"));
        let bad = ad.dispatch(&job(), &mk(3)).unwrap();
        assert_eq!(bad.terminal_state.as_deref(), Some("failed"));
        // A --ref the fake cannot validate is refused, not run
        // against the wrong revision.
        let mut j = job();
        j.git_ref = "deadbeef".to_string();
        assert!(ad.dispatch(&j, &mk(0)).is_err());
    }

    #[test]
    fn commit_new_session_merges_without_dropping_other_sessions() {
        let _store_env_guard = STORE_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir =
            std::env::temp_dir().join(format!("fleet-place-test-merge-{}", std::process::id()));
        unsafe { std::env::set_var("FLEET_PLACE_STATE", &dir) };
        let mut store = load_store();
        store["sessions"]["fp_other"] = json!({ "handle": "fp_other", "state": "running" });
        save_store(&store).unwrap();
        // A second process's snapshot taken before fp_other was
        // written must not erase it when committing its own
        // session (the old load-modify-save race).
        commit_new_session(
            &json!({ "handle": "fp_mine", "state": "queued" }),
            Some(&json!({ "handle": "fp_mine", "month": current_month(), "minutes": 0.0 })),
            &remote::NullRemote,
        )
        .unwrap();
        let back = load_store();
        assert_eq!(back["sessions"]["fp_other"]["state"], json!("running"));
        assert_eq!(back["sessions"]["fp_mine"]["state"], json!("queued"));
        assert_eq!(back["blacksmith_dispatches"].as_array().unwrap().len(), 1);
        unsafe { std::env::remove_var("FLEET_PLACE_STATE") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A placement-mirror fake for the commit wiring tests:
    /// records puts, and can be told to fail (the backend-down
    /// case).
    struct RecordingRemote {
        puts: RefCell<Vec<Value>>,
        fail: bool,
    }

    impl remote::SessionRemote for RecordingRemote {
        fn put(&self, rec: &Value) -> Result<(), String> {
            if self.fail {
                return Err("backend down".to_string());
            }
            self.puts.borrow_mut().push(rec.clone());
            Ok(())
        }
        fn get(&self, _handle: &str) -> Result<Option<Value>, String> {
            Err("not used by commit".to_string())
        }
        fn list(&self, _who: Option<&str>, _limit: usize) -> Result<Vec<Value>, String> {
            Err("not used by commit".to_string())
        }
        fn sync(&self, _records: &[Value]) -> Result<i64, String> {
            Err("not used by commit".to_string())
        }
    }

    #[test]
    fn commit_new_session_mirrors_and_survives_remote_failure() {
        // Oracle (#13146): the local commit lands identically
        // whether the mirror accepts the record or is down; when
        // the mirror is up it receives exactly the committed
        // record — the local store is the commit, the mirror is a
        // mirror.
        let _store_env_guard = STORE_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir =
            std::env::temp_dir().join(format!("fleet-place-test-mirror-{}", std::process::id()));
        unsafe { std::env::set_var("FLEET_PLACE_STATE", &dir) };
        let rec = json!({ "handle": "fp_mirror", "state": "queued" });
        let up = RecordingRemote {
            puts: RefCell::new(Vec::new()),
            fail: false,
        };
        commit_new_session(&rec, None, &up).unwrap();
        assert_eq!(up.puts.borrow().as_slice(), &[rec.clone()]);
        assert_eq!(
            load_store()["sessions"]["fp_mirror"]["state"],
            json!("queued")
        );
        let down = RecordingRemote {
            puts: RefCell::new(Vec::new()),
            fail: true,
        };
        commit_new_session(
            &json!({ "handle": "fp_offline", "state": "queued" }),
            None,
            &down,
        )
        .unwrap();
        assert_eq!(
            load_store()["sessions"]["fp_offline"]["state"],
            json!("queued")
        );
        unsafe { std::env::remove_var("FLEET_PLACE_STATE") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_registry_parses_and_has_org_run() {
        let reg = load_embedded_registry().unwrap();
        let org = find_surface(&reg, "org-run").expect("org-run in embedded registry");
        let org_job = job();
        assert!(surface_targets_job(org, &org_job));
        let mut fork_job = job();
        fork_job.repo = "developing-today-forks/opencode".to_string();
        let canonical_org = find_surface(&reg, "fleet-org").unwrap();
        assert!(surface_targets_job(canonical_org, &fork_job));
        let forks_runner = find_surface(&reg, "fleet-forks").unwrap();
        assert!(surface_targets_job(forks_runner, &fork_job));
    }

    #[test]
    fn blacksmith_reconcile_candidates_cover_unreconciled_terminal() {
        let reg = registry();
        let store = json!({
            "sessions": {
                "fp_live": { "handle": "fp_live", "surface": "fleet-org-blacksmith", "run_id": 11, "state": "running" },
                "fp_done_unrec": { "handle": "fp_done_unrec", "surface": "fleet-org-blacksmith", "run_id": 12, "state": "succeeded" },
                "fp_done_rec": { "handle": "fp_done_rec", "surface": "fleet-org-blacksmith", "run_id": 13, "state": "succeeded" },
                "fp_gh": { "handle": "fp_gh", "surface": "fleet-personal", "run_id": 14, "state": "running" }
            },
            "blacksmith_dispatches": [
                { "handle": "fp_done_unrec", "month": "2026-01", "minutes": 0.0 },
                { "handle": "fp_done_rec", "month": "2026-01", "minutes": 42.0, "reconciled": true }
            ]
        });
        let mut handles = blacksmith_reconcile_handles(&store, &reg);
        handles.sort();
        assert_eq!(
            handles,
            vec!["fp_done_unrec".to_string(), "fp_live".to_string()]
        );
    }

    #[test]
    fn portable_run_reference_parser_validates_repo_and_id() {
        assert_eq!(
            parse_portable_run_reference("developing-today/fleet#12345"),
            Some(("developing-today/fleet".to_string(), 12345))
        );
        assert_eq!(
            parse_portable_run_reference("developing-today/fleet#0"),
            None
        );
        assert_eq!(parse_portable_run_reference("not-a-repo#12345"), None);
        assert_eq!(
            parse_portable_run_reference("developing-today/fleet#latest"),
            None
        );
    }

    #[test]
    fn portable_run_reference_resolves_without_local_session_store() {
        let reg = registry();
        let reference = "developing-today/code#12345";
        let gh = FakeGh {
            runs_by_id: Map2::from([(
                12345,
                json!({
                    "status": "completed",
                    "conclusion": "success",
                    "html_url": "https://github.com/developing-today/code/actions/runs/12345"
                }),
            )]),
            ..Default::default()
        };

        let rec = resolve_portable_run_reference(reference, &reg, &gh).unwrap();

        assert_eq!(rec["portable_id"], json!(reference));
        assert_eq!(rec["runner_repo"], json!("developing-today/code"));
        assert_eq!(rec["run_id"], json!(12345));
        assert_eq!(rec["surface"], json!("org-run"));
        assert_eq!(rec["state"], json!("succeeded"));
        assert_eq!(
            rec["run_url"],
            json!("https://github.com/developing-today/code/actions/runs/12345")
        );
    }

    #[test]
    fn refresh_session_marks_blacksmith_reconciliation() {
        let reg = registry();
        let mut store = json!({
            "sessions": {
                "fp_bs": { "handle": "fp_bs", "surface": "fleet-org-blacksmith", "run_id": 777, "state": "running" }
            },
            "blacksmith_dispatches": [
                { "handle": "fp_bs", "month": "2026-01", "minutes": 0.0 }
            ]
        });
        let gh = FakeGh {
            runs_by_id: Map2::from([(
                777,
                json!({
                    "status": "completed",
                    "conclusion": "success",
                    "run_started_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:30:00Z"
                }),
            )]),
            ..Default::default()
        };
        let rec = refresh_session(&mut store, "fp_bs", &reg, &gh).unwrap();
        assert_eq!(rec["state"], json!("succeeded"));
        assert_eq!(store["blacksmith_dispatches"][0]["minutes"], json!(30.0));
        assert_eq!(store["blacksmith_dispatches"][0]["reconciled"], json!(true));
    }

    #[test]
    fn store_roundtrip_preserves_sessions() {
        let _store_env_guard = STORE_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join(format!("fleet-place-test-{}", std::process::id()));
        // SAFETY: this test owns the variable for its duration and
        // no other test reads the placement state path.
        unsafe { std::env::set_var("FLEET_PLACE_STATE", &dir) };
        let mut store = load_store();
        store["sessions"]["fp_test"] = json!({ "handle": "fp_test", "state": "queued" });
        save_store(&store).unwrap();
        let back = load_store();
        assert_eq!(back["sessions"]["fp_test"]["state"], json!("queued"));
        unsafe { std::env::remove_var("FLEET_PLACE_STATE") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- dispatch_as: user (FDR-0013) ----
    //
    // Oracle: the registry field's contract. A surface that says
    // `dispatch_as: user` uses the user's token for eligibility and
    // dispatch; every other surface keeps the App-only behaviour.

    /// The personal fleet surface, re-declared as a fork the App is
    /// not installed on, dispatching as the user.
    fn user_surface() -> Value {
        let reg = registry();
        let mut surface = find_surface(&reg, "fleet-personal").unwrap().clone();
        surface["id"] = json!("fleet-forks");
        surface["dispatch_as"] = json!("user");
        surface["app_installation"] = Value::Null;
        surface
    }

    #[test]
    fn hosts_yml_token_reads_the_oauth_token_line() {
        // Oracle: gh's hosts.yml shape (a user block carrying
        // oauth_token), the same fixture shape `fleet gh` tests use.
        let yml = "github.com:\n    git_protocol: https\n    users:\n        someone:\n            oauth_token: TOKEN_VALUE\n    user: someone\n";
        assert_eq!(hosts_yml_token(yml).as_deref(), Some("TOKEN_VALUE"));
        assert_eq!(
            hosts_yml_token("oauth_token: \"QUOTED\"\n").as_deref(),
            Some("QUOTED")
        );
        // An empty value and a file without the line are misses.
        assert_eq!(hosts_yml_token("oauth_token:\n"), None);
        assert_eq!(hosts_yml_token("github.com:\n    user: someone\n"), None);
    }

    #[test]
    fn first_token_takes_the_first_non_blank_lookup_in_order() {
        let none = || None;
        let blank = || Some("  \n".to_string());
        let a = || Some(" a-token\n".to_string());
        let b = || Some("b-token".to_string());
        assert_eq!(
            first_token(&[&none, &blank, &a, &b]).as_deref(),
            Some("a-token")
        );
        assert_eq!(first_token(&[&b, &a]).as_deref(), Some("b-token"));
        assert_eq!(first_token(&[&none, &blank]), None);
        assert_eq!(first_token(&[]), None);
    }

    #[test]
    fn a_user_dispatch_surface_is_eligible_and_posts_with_the_user_token() {
        let surface = user_surface();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            // The App cannot mint for this repo at all.
            tokens_ok: false,
            user_token: Some("user-token".to_string()),
            post_runs: vec![json!({
                "id": 4242,
                "html_url": "https://github.com/dezren39/fleet/actions/runs/4242",
                "created_at": "2026-01-01T00:00:10Z"
            })],
            ..Default::default()
        };
        let c = ctx(&[("dezren39", (0, 0))], 0.0, false);
        assert_eq!(ad.eligible(&job(), &c, &gh), (true, "eligible".to_string()));
        let result = ad.dispatch(&job(), &gh).unwrap();
        assert_eq!(result.run_id, Some(4242));
        assert_eq!(gh.post_tokens.borrow().as_slice(), ["user-token"]);
    }

    #[test]
    fn a_user_dispatch_surface_without_a_user_token_is_ineligible_and_says_why() {
        let surface = user_surface();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            // An App token being available must not rescue it: the
            // surface said which token it uses.
            tokens_ok: true,
            user_token: None,
            ..Default::default()
        };
        let c = ctx(&[("dezren39", (0, 0))], 0.0, false);
        let (ok, why) = ad.eligible(&job(), &c, &gh);
        assert!(!ok);
        assert!(why.contains("dispatches as the user"), "{why}");
        let err = match ad.dispatch(&job(), &gh) {
            Ok(_) => panic!("a surface with no token must not dispatch"),
            Err(e) => e,
        };
        assert!(err.0.contains("no user token"), "{}", err.0);
        assert!(gh.posts.borrow().is_empty());
    }

    #[test]
    fn an_app_surface_never_falls_back_to_the_user_token() {
        // Positive control for the two tests above: without the
        // registry field, a user token changes nothing.
        let reg = registry();
        let surface = find_surface(&reg, "fleet-personal").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            tokens_ok: false,
            user_token: Some("user-token".to_string()),
            ..Default::default()
        };
        let c = ctx(&[("dezren39", (0, 0))], 0.0, false);
        let (ok, why) = ad.eligible(&job(), &c, &gh);
        assert!(!ok);
        assert!(why.contains("unreachable by the App"), "{why}");
    }

    #[test]
    fn an_unknown_dispatch_as_value_is_refused() {
        let mut surface = user_surface();
        surface["dispatch_as"] = json!("robot");
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            user_token: Some("user-token".to_string()),
            ..Default::default()
        };
        let c = ctx(&[("dezren39", (0, 0))], 0.0, false);
        let (ok, why) = ad.eligible(&job(), &c, &gh);
        assert!(!ok);
        assert!(why.contains("not 'app' or 'user'"), "{why}");
    }

    #[test]
    fn load_for_a_user_dispatch_host_is_read_with_the_user_token() {
        let mut reg = registry();
        reg["surfaces"] = json!([user_surface()]);
        // No user token: the host's load cannot be read, and the
        // context says so instead of counting it as zero silently.
        let gh = FakeGh {
            tokens_ok: true,
            user_token: None,
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        assert_eq!(c.load_read_errors.len(), 1, "{:?}", c.load_read_errors);
        assert!(c.load_read_errors[0].contains("dezren39/fleet"));
        // With one, the read goes through.
        let gh = FakeGh {
            tokens_ok: false,
            user_token: Some("user-token".to_string()),
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        assert!(c.load_read_errors.is_empty(), "{:?}", c.load_read_errors);
    }
    #[test]
    fn owner_scoped_target_rules_are_exact_and_safe() {
        assert!(target_matches(
            "developing-today/*",
            "developing-today/opencode"
        ));
        assert!(target_matches("dezren39/*", "dezren39/fleet"));
        assert!(!target_matches(
            "developing-today/*",
            "developing-today-forks/opencode"
        ));
        assert!(!target_matches(
            "developing-today/*",
            "developing-today/a/b"
        ));
        assert!(!target_matches("*", "missing-owner"));
        assert!(target_matches(
            "developing-today/code",
            "developing-today/code"
        ));
        assert!(!target_matches(
            "developing-today/code",
            "developing-today/other"
        ));
    }

    #[test]
    fn followup_affinity_keeps_surface_until_backlog_or_capacity_limit() {
        let reg = registry();
        let gh = FakeGh::default();
        let mut j = job();
        j.preferred_surface = Some("org-run".to_string());
        let c = ctx(
            &[("dezren39", (0, 0)), ("developing-today", (3, 0))],
            0.0,
            false,
        );
        assert_eq!(pick(&reg, &j, &c, &gh).as_deref(), Some("org-run"));

        let c = ctx(
            &[("dezren39", (0, 0)), ("developing-today", (20, 2))],
            0.0,
            false,
        );
        assert_eq!(pick(&reg, &j, &c, &gh).as_deref(), Some("fleet-personal"));
    }

    #[test]
    fn workflow_inputs_carry_cache_restore_keys_and_previous_run() {
        let mut j = job();
        j.platform = "macos".to_string();
        j.actions_cache = Some(
            "rust-code-branch-hash:/root/.cargo/registry\n/root/.cargo/git\ntarget".to_string(),
        );
        j.actions_cache_restore_keys =
            vec!["rust-code-branch-".to_string(), "rust-code-".to_string()];
        j.previous_run_id = Some(1234);
        j.previous_run_repo = Some("developing-today/fleet".to_string());
        let mut surface = json!({
            "id": "fleet-personal",
            "kind": "github-actions",
            "repo": "dezren39/fleet",
            "workflow": "fleet-run.yml",
            "previous_run_id_input": true,
            "platform_input": true,
        });
        let adapter = GithubActionsAdapter {
            surface: surface.take(),
            force_runner: "github",
        };
        let inputs = adapter.workflow_inputs(&j, Some("fp_test"));
        let jobs: Value = serde_json::from_str(inputs["jobs"].as_str().unwrap()).unwrap();
        assert_eq!(jobs[0]["previous_run_id"], "1234");
        assert_eq!(jobs[0]["previous_run_repo"], "developing-today/fleet");
        assert_eq!(jobs[0]["platform"], "macos");
        assert_eq!(
            jobs[0]["cache"]["actions_restore_keys"][0],
            "rust-code-branch-"
        );
        assert_eq!(jobs[0]["cache"]["actions_restore_keys"][1], "rust-code-");
        assert_eq!(inputs["dispatch_id"], "fp_test");
    }

    /// #13178: fleet-command.yml dispatches send `timeout_minutes`
    /// only when the job sets one, as the string GitHub expects.
    #[test]
    fn fleet_command_sends_timeout_minutes_only_when_set_13178() {
        let adapter = GithubActionsAdapter {
            surface: json!({
                "id": "fleet-org",
                "kind": "github-actions",
                "repo": "developing-today/fleet",
                "workflow": "fleet-command.yml",
            }),
            force_runner: "github",
        };
        let inputs = adapter.workflow_inputs(&job(), None);
        assert!(inputs.get("timeout_minutes").is_none());
        let mut j = job();
        j.timeout_minutes = Some(90);
        let inputs = adapter.workflow_inputs(&j, None);
        assert_eq!(inputs["timeout_minutes"], "90");
    }

    /// #13178: the stored job keeps `timeout_minutes` so a queued
    /// session re-dispatches with it; out-of-range values are dropped
    /// rather than sent to a workflow that would reject them.
    #[test]
    fn job_round_trips_timeout_minutes_13178() {
        let mut j = job();
        j.timeout_minutes = Some(30);
        j.session_handle = Some("fp_sess".to_string());
        let stored = j.to_json();
        assert_eq!(stored["timeout_minutes"], 30);
        assert!(stored.get("session_handle").is_none());
        let back = Job::from_json(&stored).unwrap();
        assert_eq!(back.timeout_minutes, Some(30));
        assert_eq!(back.session_handle, None);
        let mut bad = stored.clone();
        bad["timeout_minutes"] = json!(500);
        assert_eq!(Job::from_json(&bad).unwrap().timeout_minutes, None);
        assert!(job().to_json_compact().get("timeout_minutes").is_none());
    }

    /// #13178: a surface whose workflow declares `dispatch_id` gets
    /// the placement session handle, so the run title names the
    /// session; without one a fresh handle is minted as before.
    #[test]
    fn dispatch_id_is_the_session_handle_13178() {
        let surface = json!({"dispatch_id_input": true});
        let mut j = job();
        j.session_handle = Some("fp_session".to_string());
        assert_eq!(
            super::adapters::dispatch_id_for(&surface, &j).as_deref(),
            Some("fp_session")
        );
        let fresh = super::adapters::dispatch_id_for(&surface, &job()).unwrap();
        assert!(fresh.starts_with("fp_") && fresh != "fp_session");
        assert_eq!(super::adapters::dispatch_id_for(&json!({}), &j), None);
    }

    /// #13178: the session record names who actually dispatched.
    #[test]
    fn dispatch_actor_follows_the_surface_auth_route_13178() {
        let reg: Value = serde_json::from_str(EMBEDDED_REGISTRY).unwrap();
        let actor = |id: &str| dispatch_actor(find_surface(&reg, id));
        assert_eq!(
            actor("fleet-personal"),
            "developing-today[bot] (GitHub App dispatch)"
        );
        assert_eq!(actor("fleet-org"), "user token (dispatch_as: user)");
        assert_eq!(actor("fleet-forks"), "user token (dispatch_as: user)");
        assert_eq!(
            actor("fleet-org-blacksmith"),
            "user token (dispatch_as: user)"
        );
        assert_eq!(
            actor("codespace-overflow"),
            "user gh CLI credentials (codespace)"
        );
        assert_eq!(actor("laptop"), "laptop handoff (no remote dispatch)");
        assert_eq!(dispatch_actor(None), "none (no surface dispatched)");
    }

    /// #13175 oracle: a help flag after a session subcommand is a
    /// help request, never a handle; a real handle is not.
    #[test]
    fn session_help_flag_is_not_a_handle() {
        let args = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<String>>();
        assert!(session_help_requested(
            "status",
            &args(&["status", "--help"])
        ));
        assert!(session_help_requested("result", &args(&["result", "-h"])));
        assert!(session_help_requested("list", &args(&["list", "--help"])));
        assert!(!session_help_requested(
            "status",
            &args(&["status", "fp_abc123"])
        ));
        assert!(!session_help_requested("status", &args(&["status"])));
        assert!(!session_help_requested(
            "bogus",
            &args(&["bogus", "--help"])
        ));
    }

    /// A load-reply table answering every fixture-registry host
    /// repo with one canned reply per status.
    fn all_load_replies(code: u16, payload: Value) -> Map2<(String, String), (u16, Value)> {
        let mut replies = Map2::new();
        for repo in [
            "dezren39/fleet",
            "developing-today/code",
            "developing-today/fleet",
        ] {
            for status in ["in_progress", "queued"] {
                replies.insert(
                    (repo.to_string(), status.to_string()),
                    (code, payload.clone()),
                );
            }
        }
        replies
    }

    #[test]
    fn coverage_measured_zero_yields_numeric_headroom() {
        // Fixture oracle (issue #13611): a fully observed zero with
        // cap 40 is a measured zero — numeric headroom, normal pick,
        // no unknown-capacity reasoning.
        let reg = registry();
        let gh = FakeGh::default();
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Measured);
        assert_eq!(cov.known_load(), 0);
        assert_eq!(cov.cap, 40);
        assert_eq!(cov.read_at, FIXED_NOW);
        assert!(cov.missing_reads.is_empty());
        let j = owner_load_coverage_json(&c);
        assert_eq!(j["dezren39"]["status"], json!("measured"));
        assert_eq!(j["dezren39"]["known_load"], json!(0));
        assert_eq!(j["dezren39"]["cap"], json!(40));
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        assert_eq!(ad.capacity(&job(), &c).free_slots, Some(40));
        let (picked, reasoning) = decide(&reg, &job(), &c, &gh).unwrap();
        assert_eq!(surf_str(picked.unwrap().surface(), "id"), "fleet-personal");
        assert!(
            !reasoning.iter().any(|r| r.contains("unknown-capacity")),
            "{reasoning:?}"
        );
    }

    #[test]
    fn coverage_missing_token_is_unknown_not_zero() {
        // Fixture oracle: no covering token -> the owner's load is
        // unknown; no numeric headroom may be reported for it.
        let reg = registry();
        let gh = FakeGh {
            tokens_ok: false,
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Unknown);
        assert_eq!(cov.known_load(), 0);
        assert!(!cov.missing_reads.is_empty());
        assert!(
            cov.missing_reads
                .iter()
                .all(|m| m.contains("no token available")),
            "{:?}",
            cov.missing_reads
        );
        let j = owner_load_coverage_json(&c);
        assert_eq!(j["dezren39"]["status"], json!("unknown"));
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        let cap = ad.capacity(&job(), &c);
        assert_eq!(cap.free_slots, None);
        assert!(cap.summary.contains("load unknown"), "{}", cap.summary);
    }

    #[test]
    fn coverage_http_403_is_unknown_and_falls_back_by_rule() {
        // Fixture oracle: every load read refused (403) -> unknown
        // coverage; with no measured headroom anywhere, tier 1 may
        // still pick a free surface, but only under the named
        // unknown-capacity rule, deterministically.
        let reg = registry();
        let gh = FakeGh {
            load_replies: all_load_replies(403, json!({ "message": "Forbidden" })),
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Unknown);
        assert!(
            cov.missing_reads.iter().all(|m| m.contains("HTTP 403")),
            "{:?}",
            cov.missing_reads
        );
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        assert_eq!(ad.capacity(&job(), &c).free_slots, None);
        let (picked, reasoning) = decide(&reg, &job(), &c, &gh).unwrap();
        assert_eq!(surf_str(picked.unwrap().surface(), "id"), "fleet-personal");
        assert!(
            reasoning
                .iter()
                .any(|r| r.contains("unknown-capacity rule")),
            "{reasoning:?}"
        );
    }

    #[test]
    fn coverage_malformed_200_is_unobserved_not_zero() {
        // Fixture oracle: an HTTP 200 without a valid total_count
        // observes nothing — it must not collapse to a measured 0.
        let reg = registry();
        let gh = FakeGh {
            load_replies: all_load_replies(200, json!({ "workflow_runs": [] })),
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Unknown);
        assert!(
            cov.missing_reads
                .iter()
                .all(|m| m.contains("without a valid total_count")),
            "{:?}",
            cov.missing_reads
        );
        let j = owner_load_coverage_json(&c);
        assert_eq!(j["dezren39"]["status"], json!("unknown"));
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        assert_eq!(ad.capacity(&job(), &c).free_slots, None);
    }

    #[test]
    fn coverage_partial_keeps_known_subtotal_and_yields_no_estimate() {
        // Fixture oracle: in_progress reads 3 while the queued read
        // fails -> partial coverage; the known subtotal 3 is kept as
        // a lower bound, no numeric headroom is reported, and a
        // measured surface elsewhere still wins the pick.
        let reg = registry();
        let mut replies = Map2::new();
        replies.insert(
            ("dezren39/fleet".to_string(), "in_progress".to_string()),
            (200, json!({ "total_count": 3 })),
        );
        replies.insert(
            ("dezren39/fleet".to_string(), "queued".to_string()),
            (500, json!({ "message": "boom" })),
        );
        let gh = FakeGh {
            load_replies: replies,
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Partial);
        assert_eq!(cov.known_running, 3);
        assert_eq!(cov.known_queued, 0);
        assert_eq!(cov.known_load(), 3);
        assert_eq!(cov.missing_reads.len(), 1);
        let j = owner_load_coverage_json(&c);
        assert_eq!(j["dezren39"]["status"], json!("partial"));
        assert_eq!(j["dezren39"]["known_load"], json!(3));
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        let cap = ad.capacity(&job(), &c);
        assert_eq!(cap.free_slots, None);
        assert!(cap.summary.contains("at least 3"), "{}", cap.summary);
        let (picked, reasoning) = decide(&reg, &job(), &c, &gh).unwrap();
        assert_eq!(surf_str(picked.unwrap().surface(), "id"), "org-run");
        assert!(
            !reasoning
                .iter()
                .any(|r| r.contains("unknown-capacity rule")),
            "{reasoning:?}"
        );
    }

    #[test]
    fn coverage_partial_subtotal_at_cap_proves_saturation() {
        // A known subtotal that already reaches the cap proves
        // saturation even when the remaining reads are missing:
        // the estimate is a proven 0, not an unknown.
        let reg = registry();
        let mut replies = Map2::new();
        replies.insert(
            ("dezren39/fleet".to_string(), "in_progress".to_string()),
            (200, json!({ "total_count": 40 })),
        );
        replies.insert(
            ("dezren39/fleet".to_string(), "queued".to_string()),
            (500, json!({ "message": "boom" })),
        );
        let gh = FakeGh {
            load_replies: replies,
            ..Default::default()
        };
        let c = build_ctx(&reg, &json!({}), false, "", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Partial);
        assert_eq!(cov.estimated_free(40), Some(0));
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        let cap = ad.capacity(&job(), &c);
        assert_eq!(cap.free_slots, Some(0));
        assert!(cap.summary.contains("saturated"), "{}", cap.summary);
        let (picked, _) = decide(&reg, &job(), &c, &gh).unwrap();
        assert_eq!(surf_str(picked.unwrap().surface(), "id"), "org-run");
    }

    #[test]
    fn coverage_override_is_labeled_simulated() {
        // Fixture oracle: an explicit --assume-owner-load override
        // is a simulation — labeled in the JSON and the summary —
        // and may yield a numeric estimate on that basis alone.
        let reg = registry();
        let gh = FakeGh::default();
        let c = build_ctx(&reg, &json!({}), false, "dezren39=5", None, &gh);
        let cov = &c.owner_load_coverage["dezren39"];
        assert_eq!(cov.status, LoadCoverageStatus::Simulated);
        assert_eq!(cov.known_load(), 5);
        assert!(cov.source.contains("simulation"), "{}", cov.source);
        let j = owner_load_coverage_json(&c);
        assert_eq!(j["dezren39"]["status"], json!("simulated"));
        assert_eq!(j["dezren39"]["known_load"], json!(5));
        let ad = adapter_for(find_surface(&reg, "fleet-personal").unwrap()).unwrap();
        let cap = ad.capacity(&job(), &c);
        assert_eq!(cap.free_slots, Some(35));
        assert!(cap.summary.contains("simulated"), "{}", cap.summary);
    }

    // Oracle (developing-today/code#15651): a dispatch-command input
    // becomes GitHub run metadata the moment it is POSTed, so a
    // command carrying a locally-resolvable laptop-tier credential
    // value must die by secret NAME before the POST — never
    // forwarding the value, and never echoing the command that
    // carries it.
    fn stub_resolver<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn dispatch_refuses_command_carrying_secret_value() {
        use super::adapters::reject_secrets_in_command;
        let value = "bearer-value-0123456789abcdef";
        let pairs = [("desktop-commander.machine-bearer", value)];
        let resolve = stub_resolver(&pairs);
        let cmd = format!("fleet commander exec -- echo {value}");
        let err = match reject_secrets_in_command(&cmd, &resolve) {
            Ok(()) => panic!("a command carrying the T1 bearer value must die"),
            Err(e) => e,
        };
        assert!(
            err.0.contains("desktop-commander.machine-bearer"),
            "{}",
            err.0
        );
        assert!(!err.0.contains(value), "the error must not echo the value");
        assert!(!err.0.contains(&cmd), "the error must not echo the command");
    }

    #[test]
    fn dispatch_allows_command_without_secret_values() {
        use super::adapters::reject_secrets_in_command;
        let pairs = [(
            "desktop-commander.machine-bearer",
            "bearer-value-0123456789abcdef",
        )];
        let resolve = stub_resolver(&pairs);
        assert!(reject_secrets_in_command("nix build .#mcpx", &resolve).is_ok());
    }

    #[test]
    fn dispatch_skips_secrets_it_cannot_resolve() {
        // Fail-open on resolution failure: when the broker cannot
        // answer there is nothing to compare against, and the fleet
        // stays dispatchable during a secrets-infra outage. A
        // positive match always fails closed (previous test).
        use super::adapters::reject_secrets_in_command;
        let resolve = stub_resolver(&[]);
        assert!(reject_secrets_in_command("nix build .#mcpx", &resolve).is_ok());
    }

    #[test]
    fn dispatch_ignores_short_secret_values() {
        // Short strings match innocent commands by accident (a
        // port, a single word); real bearers and tokens are far
        // longer, so values under the floor never trigger.
        use super::adapters::{embedded_secret_name, reject_secrets_in_command};
        assert_eq!(
            embedded_secret_name("run on port 8080 now", &[("s", "8080")]),
            None
        );
        let pairs = [("fleet.laptop-agent-token", "abc")];
        let resolve = stub_resolver(&pairs);
        assert!(reject_secrets_in_command("echo abc", &resolve).is_ok());
    }

    #[test]
    fn embedded_secret_reports_the_matching_name() {
        use super::adapters::{DISPATCH_SECRET_NAMES, embedded_secret_name};
        assert!(DISPATCH_SECRET_NAMES.contains(&"desktop-commander.machine-bearer"));
        assert!(DISPATCH_SECRET_NAMES.contains(&"fleet.laptop-agent-token"));
        let found = embedded_secret_name(
            "curl -H 'X-Agent-Token: agent-token-0123456789abcdef' https://laptop/",
            &[
                (
                    "desktop-commander.machine-bearer",
                    "bearer-value-0123456789abcdef",
                ),
                ("fleet.laptop-agent-token", "agent-token-0123456789abcdef"),
            ],
        );
        assert_eq!(found, Some("fleet.laptop-agent-token"));
        assert_eq!(
            embedded_secret_name("nix build .#mcpx", &[("s", "v")]),
            None
        );
    }
}
