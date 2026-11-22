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
//! `repo-queue session status|result|list`. STOPGAP (labeled, per
//! the spec): records live in a local JSON store at
//! `~/.local/state/fleet-place/sessions.json` — the same path and
//! record shape the reference implementation used, so records
//! migrate rather than being redesigned when the designed
//! `agent.placement_sessions` table lands in the queue backend.
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
}

// ----------------------------------------------------------------- context

/// Everything a decision needs beyond the registry and the job:
/// live per-owner load, the Blacksmith budget position, and the
/// laptop saturation flag. Built by `build_ctx`; tests build it
/// directly.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub owners: Value,
    pub owner_load: HashMap<String, (i64, i64)>, // owner -> (running, queued)
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
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
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
/// process loaded its snapshot are preserved.
fn commit_new_session(rec: &Value, blacksmith_entry: Option<&Value>) -> R<()> {
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
    })
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

#[derive(Debug, Clone)]
pub struct Capacity {
    pub summary: String,
    pub free_slots: Option<i64>,
    pub remaining_total_minutes: Option<f64>,
}

#[derive(Debug, Default)]
pub struct DispatchResult {
    pub run_id: Option<i64>,
    pub run_url: Option<String>,
    pub note: Option<String>,
    pub codespace: Option<String>,
    /// Set when the dispatch itself already knows the terminal
    /// outcome (Codespace ssh exit code), so the session record
    /// is terminal from the start instead of stuck `dispatched`.
    pub terminal_state: Option<String>,
}

/// One adapter per registry `kind` (spec: "Adapters per placement
/// kind"). Adding a placement type = a new adapter here plus a new
/// `kind` value in the registry; the cascade below does not change.
pub trait Adapter {
    fn kind(&self) -> &'static str;
    fn surface(&self) -> &Value;
    fn eligible(&self, job: &Job, ctx: &Ctx, gh: &dyn Gh) -> (bool, String);
    fn capacity(&self, job: &Job, ctx: &Ctx) -> Capacity;
    fn dispatch(&self, job: &Job, gh: &dyn Gh) -> R<DispatchResult>;
    fn status(&self, session: &Value, gh: &dyn Gh) -> String {
        session
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string()
    }
}

fn base_eligible(surface: &Value, job: &Job) -> (bool, String) {
    let status = surf_str(surface, "status");
    if status != "active" {
        return (false, format!("surface status is '{status}', not 'active'"));
    }
    let dispatch_as = surf_str(surface, "dispatch_as");
    if !matches!(dispatch_as, "" | "app" | "user") {
        return (
            false,
            format!("surface dispatch_as is '{dispatch_as}', not 'app' or 'user'"),
        );
    }
    if !surface_targets_job(surface, job) {
        return (
            false,
            format!("{} is not in this surface's targets", job.repo),
        );
    }
    (true, "eligible".to_string())
}

/// The dispatch_id for a surface whose workflow declares a
/// `dispatch_id` input (registry flag `dispatch_id_input`, set for
/// org-run.yml and dezren39/fleet's fleet-run.yml, which surface it
/// in the run display title via `run-name:` — issue #8371). A fresh
/// fp_ handle per dispatch. Surfaces without the flag get None:
/// GitHub rejects dispatch inputs the workflow does not declare,
/// so those keep the snapshot-exclusion fallback in correlate_run.
fn dispatch_id_for(surface: &Value) -> Option<String> {
    if surface.get("dispatch_id_input").and_then(|v| v.as_bool()) == Some(true) {
        Some(new_handle(&rand_source::URandom))
    } else {
        None
    }
}

/// Pick the run a dispatch created out of a workflow run list.
/// Runs in the pre-dispatch snapshot (before_ids) and runs created
/// before `since` are never candidates. With a dispatch_id, only a
/// run whose display_title carries the id is a candidate — exact
/// correlation even when another dispatch of the same workflow
/// lands in the same poll window (issue #8371). Without one, the
/// earliest new run wins (the snapshot-exclusion fallback).
fn correlate_run(
    runs: &[Value],
    before_ids: &std::collections::HashSet<i64>,
    since: i64,
    dispatch_id: Option<&str>,
) -> Option<(i64, String)> {
    let mut candidates: Vec<(i64, i64, String)> = Vec::new();
    for r in runs {
        let id = r.get("id").and_then(|i| i.as_i64()).unwrap_or(0);
        if before_ids.contains(&id) {
            continue;
        }
        let created = r
            .get("created_at")
            .and_then(|c| c.as_str())
            .and_then(parse_iso)
            .unwrap_or(0);
        if created < since {
            continue;
        }
        if let Some(d) = dispatch_id {
            let title = r
                .get("display_title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !title.contains(d) {
                continue;
            }
        }
        candidates.push((
            created,
            id,
            r.get("html_url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
        ));
    }
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|(_created, id, url)| (id, url))
}

pub struct GithubActionsAdapter {
    surface: Value,
    /// The runner this adapter dispatches: "github" for the plain
    /// adapter, "blacksmith" for the Blacksmith variant (the
    /// reference models Blacksmith as a subclass overriding
    /// runner_name).
    force_runner: &'static str,
}

impl GithubActionsAdapter {
    /// The fleet-run workflow inputs for a job. Cache options travel
    /// in a `jobs` entry (the workflows that accept bare repo/command
    /// inputs ignore entry fields they do not know); the `runner`
    /// input is sent only when it changes something (blacksmith) —
    /// upstream's fleet-run rejects unknown inputs.
    fn workflow_inputs(&self, job: &Job, dispatch_id: Option<&str>) -> Value {
        if surf_str(&self.surface, "workflow") == "fleet-command.yml" {
            let mut inputs = Map::new();
            inputs.insert("repo".into(), json!(job.repo));
            inputs.insert("command".into(), json!(job.command));
            inputs.insert("runner".into(), json!(self.runner_name(job)));
            inputs.insert("platform".into(), json!(job.platform));
            if !job.git_ref.is_empty() {
                inputs.insert("ref".into(), json!(job.git_ref));
            }
            if self.surface.get("container_input").and_then(Value::as_bool) == Some(true) {
                if let Some(container) = &job.container {
                    inputs.insert("container_image".into(), json!(container));
                }
            }
            if let Some(cache) = &job.actions_cache {
                let (key, path) = cache.split_once(':').unwrap_or((cache.as_str(), "."));
                inputs.insert("actions_cache_key".into(), json!(key));
                inputs.insert("actions_cache_path".into(), json!(path));
                inputs.insert(
                    "actions_cache_restore_keys".into(),
                    json!(job.actions_cache_restore_keys.join("\n")),
                );
            }
            if job.push_nix_cache {
                inputs.insert("push_nix_cache".into(), json!(true));
            }
            if self
                .surface
                .get("previous_run_id_input")
                .and_then(Value::as_bool)
                == Some(true)
            {
                if let Some(run_id) = job.previous_run_id {
                    inputs.insert("previous_run_id".into(), json!(run_id.to_string()));
                }
                if let Some(repo) = &job.previous_run_repo {
                    inputs.insert("previous_run_repo".into(), json!(repo));
                }
            }
            if let Some(id) = dispatch_id {
                inputs.insert("dispatch_id".into(), json!(id));
            }
            return Value::Object(inputs);
        }
        let mut entry = Map::new();
        entry.insert("repo".into(), json!(job.repo));
        entry.insert("command".into(), json!(job.command));
        if !job.git_ref.is_empty() {
            entry.insert("ref".into(), json!(job.git_ref));
        }
        let mut cache = Map::new();
        if job.push_nix_cache {
            cache.insert("push_nix".into(), json!(true));
        }
        if let Some(ac) = &job.actions_cache {
            let (key, path) = match ac.split_once(':') {
                Some((key, path)) => (key, path),
                None => (ac.as_str(), "."),
            };
            cache.insert("actions_key".into(), json!(key));
            cache.insert("actions_path".into(), json!(path));
        }
        if !job.actions_cache_restore_keys.is_empty() {
            cache.insert(
                "actions_restore_keys".into(),
                json!(job.actions_cache_restore_keys),
            );
        }
        let has_artifact_input = self
            .surface
            .get("previous_run_id_input")
            .and_then(Value::as_bool)
            == Some(true);
        let has_platform_input =
            self.surface.get("platform_input").and_then(Value::as_bool) == Some(true);
        if has_artifact_input {
            if let Some(run_id) = job.previous_run_id {
                entry.insert("previous_run_id".into(), json!(run_id.to_string()));
            }
            if let Some(repo) = &job.previous_run_repo {
                entry.insert("previous_run_repo".into(), json!(repo));
            }
        }
        if has_platform_input {
            entry.insert("platform".into(), json!(job.platform));
        }
        let mut inputs = Map::new();
        let needs_jobs = !cache.is_empty()
            || (has_artifact_input && job.previous_run_id.is_some())
            || (has_platform_input && job.platform != "linux");
        if needs_jobs {
            if !cache.is_empty() {
                entry.insert("cache".into(), Value::Object(cache));
            }
            inputs.insert(
                "jobs".into(),
                json!(Value::Array(vec![Value::Object(entry)]).to_string()),
            );
            // org-run.yml declares the top-level `repo` input
            // required even when `jobs` is given — retain it.
            inputs.insert("repo".into(), json!(job.repo));
        } else {
            inputs.insert("repo".into(), json!(job.repo));
            inputs.insert("command".into(), json!(job.command));
        }
        if has_artifact_input {
            if let Some(run_id) = job.previous_run_id {
                inputs.insert("previous_run_id".into(), json!(run_id.to_string()));
            }
            if let Some(repo) = &job.previous_run_repo {
                inputs.insert("previous_run_repo".into(), json!(repo));
            }
        }
        if has_platform_input {
            inputs.insert("platform".into(), json!(job.platform));
        }
        if surf_str(&self.surface, "workflow") == "org-run.yml" {
            // org-run's setup job defaults to a Blacksmith runner;
            // pin it to this dispatch's provider so a GitHub-only
            // dispatch does not silently consume Blacksmith
            // minutes outside the budget/ledger (Codex P2).
            inputs.insert("setup_runner".into(), json!(self.runner_name(job)));
        }
        if self.runner_name(job) == "blacksmith" {
            inputs.insert("runner".into(), json!("blacksmith"));
        }
        if !job.git_ref.is_empty() {
            inputs.insert("ref".into(), json!(job.git_ref));
        }
        if let Some(id) = dispatch_id {
            inputs.insert("dispatch_id".into(), json!(id));
        }
        Value::Object(inputs)
    }

    fn runner_name(&self, _job: &Job) -> &'static str {
        self.force_runner
    }

    fn dispatch_workflow(&self, job: &Job, gh: &dyn Gh) -> R<DispatchResult> {
        let s = &self.surface;
        let repo = surf_str(s, "repo").to_string();
        let workflow = surf_str(s, "workflow").to_string();
        let token = surface_token(s, gh).ok_or_else(|| Die(no_token_reason(s)))?;
        let dispatch_id = dispatch_id_for(s);
        let inputs = self.workflow_inputs(job, dispatch_id.as_deref());
        // Correlate the created run with THIS dispatch (Codex P1
        // on PR #8325, exact-id fix in issue #8371): when the
        // workflow declares a dispatch_id input (dispatch_id_for),
        // the id generated above travels with the dispatch and the
        // workflow surfaces it in the run's display title, so
        // correlate_run can match exactly this dispatch's run even
        // when another dispatch lands in the same poll window.
        // The pre-dispatch snapshot of existing run ids is kept
        // for every surface: it is the only correlation for
        // workflows without the input (the fallback), and with an
        // id it still excludes runs that predate this dispatch.
        let list_runs = |gh: &dyn Gh| -> R<Vec<Value>> {
            let (_st, runs) = gh.api(
                "GET",
                &format!(
                    "/repos/{repo}/actions/workflows/{workflow}/runs?event=workflow_dispatch&per_page=100"
                ),
                &token,
                None,
            )?;
            Ok(runs
                .get("workflow_runs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default())
        };
        let before_ids: std::collections::HashSet<i64> = list_runs(gh)
            .unwrap_or_default()
            .iter()
            .filter_map(|r| r.get("id").and_then(|i| i.as_i64()))
            .collect();
        let (status, payload) = gh.api(
            "POST",
            &format!("/repos/{repo}/actions/workflows/{workflow}/dispatches"),
            &token,
            Some(&json!({ "ref": "main", "inputs": inputs })),
        )?;
        if status != 204 {
            return die(format!(
                "dispatch to {} failed: HTTP {status}: {}",
                surf_str(s, "id"),
                payload
                    .get("message")
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| payload.to_string())
            ));
        }
        // Find the run the dispatch created: the earliest run
        // that was not in the pre-dispatch snapshot and was
        // created at/after the dispatch instant.
        let since = gh.now_epoch() - 5;
        let mut found: Option<(i64, String)> = None;
        for _ in 0..12 {
            gh.sleep_secs(3);
            // A failed run-list request after the POST was
            // accepted (HTTP 204) must not abort the command: the
            // job is already executing, and discarding the
            // session would lose the reconnectable handle (and
            // invite a duplicate submission). Treat it as "not
            // visible yet" and keep polling; after the loop the
            // caller records an accepted-but-unresolved session.
            if let Some((id, url)) = correlate_run(
                &list_runs(gh).unwrap_or_default(),
                &before_ids,
                since,
                dispatch_id.as_deref(),
            ) {
                found = Some((id, url));
                break;
            }
        }
        match found {
            Some((id, url)) => Ok(DispatchResult {
                run_id: Some(id),
                run_url: Some(url),
                ..Default::default()
            }),
            None => Ok(DispatchResult {
                note: Some(
                    "dispatch accepted (HTTP 204) but the run did not appear \
                     within ~40s; resolve it from the workflow's run list"
                        .to_string(),
                ),
                ..Default::default()
            }),
        }
    }

    fn run_state(&self, session: &Value, gh: &dyn Gh) -> String {
        let run_id = match session.get("run_id").and_then(|r| r.as_i64()) {
            Some(id) => id,
            None => {
                return session
                    .get("state")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();
            }
        };
        let repo = surf_str(&self.surface, "repo");
        let token = match surface_token(&self.surface, gh) {
            Some(t) => t,
            None => {
                return session
                    .get("state")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string();
            }
        };
        match gh.api(
            "GET",
            &format!("/repos/{repo}/actions/runs/{run_id}"),
            &token,
            None,
        ) {
            Ok((200, run)) => map_run_state(&run),
            _ => session
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string(),
        }
    }
}

impl Adapter for GithubActionsAdapter {
    fn kind(&self) -> &'static str {
        "github-actions"
    }
    fn surface(&self) -> &Value {
        &self.surface
    }
    fn eligible(&self, job: &Job, ctx: &Ctx, gh: &dyn Gh) -> (bool, String) {
        let _ = ctx;
        if job.platform == "macos"
            && self.surface.get("platform_input").and_then(Value::as_bool) != Some(true)
        {
            return (
                false,
                "this surface workflow does not declare the macOS platform input".to_string(),
            );
        }
        let (ok, why) = base_eligible(&self.surface, job);
        if !ok {
            return (ok, why);
        }
        if job.runner == "blacksmith" {
            return (
                false,
                "job pins runner=blacksmith; this is a github-runner surface".to_string(),
            );
        }
        // A surface the App cannot reach can never dispatch
        // (Codex P2 on PR #8325): live token availability is part
        // of eligibility, so an inaccessible surface cannot win
        // the cascade and then abort at dispatch time.
        let token = match surface_token(&self.surface, gh) {
            Some(token) => token,
            None => return (false, no_token_reason(&self.surface)),
        };
        if self
            .surface
            .get("public_targets_only")
            .and_then(Value::as_bool)
            == Some(true)
        {
            match gh.api("GET", &format!("/repos/{}", job.repo), &token, None) {
                Ok((200, target))
                    if target.get("private").and_then(Value::as_bool) == Some(false) => {}
                Ok((200, _)) => {
                    return (
                        false,
                        "this surface has no private-repository checkout token; target is not verified public"
                            .to_string(),
                    );
                }
                Ok((status, _)) => {
                    return (
                        false,
                        format!(
                            "cannot verify public target {} on this surface (HTTP {status})",
                            job.repo
                        ),
                    );
                }
                Err(error) => {
                    return (
                        false,
                        format!("cannot verify public target {}: {}", job.repo, error.0),
                    );
                }
            }
        }
        (true, "eligible".to_string())
    }
    fn capacity(&self, job: &Job, ctx: &Ctx) -> Capacity {
        let owner = surf_str(&self.surface, "owner");
        let (running, queued) = ctx.owner_load.get(owner).copied().unwrap_or((0, 0));
        let cap = if job.platform == "macos" {
            // Count all host runs conservatively as potential macOS
            // consumers so dispatch never oversubscribes the small pool.
            let macos_cap = ctx
                .owners
                .get(owner)
                .and_then(|value| value.get("concurrent_macos_cap"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            ctx.owner_cap(owner).min(macos_cap)
        } else {
            ctx.owner_cap(owner)
        };
        let active = running + queued;
        let free = (cap - active).max(0);
        Capacity {
            summary: format!(
                "owner {owner}: {active} active job(s) on fleet runner hosts \
                 (proxy for account load) vs cap {cap} -> {free} free slot(s)"
            ),
            free_slots: Some(free),
            remaining_total_minutes: None,
        }
    }
    fn dispatch(&self, job: &Job, gh: &dyn Gh) -> R<DispatchResult> {
        self.dispatch_workflow(job, gh)
    }
    fn status(&self, session: &Value, gh: &dyn Gh) -> String {
        self.run_state(session, gh)
    }
}

/// Map a GitHub Actions run onto the session state vocabulary.
pub fn map_run_state(run: &Value) -> String {
    let status = run.get("status").and_then(|s| s.as_str()).unwrap_or("");
    match status {
        "queued" | "waiting" | "pending" | "requested" => "queued".to_string(),
        "in_progress" => "running".to_string(),
        "completed" => match run.get("conclusion").and_then(|c| c.as_str()) {
            Some("success") | Some("skipped") | Some("neutral") => "succeeded".to_string(),
            Some("cancelled") => "cancelled".to_string(),
            _ => "failed".to_string(),
        },
        _ => "unknown".to_string(),
    }
}

pub struct BlacksmithAdapter {
    inner: GithubActionsAdapter,
}

impl BlacksmithAdapter {
    fn budget(&self, job: &Job, ctx: &Ctx) -> (f64, f64, f64) {
        let b = self
            .inner
            .surface
            .get("budget")
            .cloned()
            .unwrap_or(json!({}));
        let free = b
            .get("free_minutes_monthly")
            .and_then(|v| v.as_f64())
            .unwrap_or(3000.0);
        let paid_cap = b
            .get("paid_cap_minutes")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            + job.allow_paid_blacksmith;
        (free, paid_cap, ctx.blacksmith_used_minutes)
    }
}

impl Adapter for BlacksmithAdapter {
    fn kind(&self) -> &'static str {
        "blacksmith"
    }
    fn surface(&self) -> &Value {
        &self.inner.surface
    }
    fn eligible(&self, job: &Job, ctx: &Ctx, gh: &dyn Gh) -> (bool, String) {
        if surf_str(&self.inner.surface, "owner") == "dezren39" {
            return (
                false,
                "Blacksmith is orgs-only; never the personal copy".to_string(),
            );
        }
        if job.platform != "linux" {
            return (
                false,
                "Blacksmith surfaces provide Linux runners only".to_string(),
            );
        }
        let (ok, why) = base_eligible(&self.inner.surface, job);
        if !ok {
            return (ok, why);
        }
        if job.runner == "github" {
            return (false, "job pins runner=github".to_string());
        }
        if surface_token(&self.inner.surface, gh).is_none() {
            return (false, no_token_reason(&self.inner.surface));
        }
        // The budget binds pinned surfaces too (Codex round-4 on
        // PR #8325): a --surface pin is a placement choice, not a
        // paid-usage authorization. Remaining minutes already include
        // the registry paid cap and any explicit
        // --allow-paid-blacksmith allowance carried on the job.
        let (free, paid_cap, used) = self.budget(job, ctx);
        if free + paid_cap - used <= 0.0 {
            return (
                false,
                format!(
                    "blacksmith budget spent: {used:.0} of {free:.0} free                      min/mo used (+{paid_cap:.0} paid cap allowed) \u{2014} \
                     pass --allow-paid-blacksmith to authorize paid minutes"
                ),
            );
        }
        (true, "eligible".to_string())
    }
    fn capacity(&self, job: &Job, ctx: &Ctx) -> Capacity {
        let (free, paid_cap, used) = self.budget(job, ctx);
        let remaining = (free + paid_cap - used).max(0.0);
        Capacity {
            summary: format!(
                "blacksmith budget: {used:.0} of {free} free min/mo used \
                 (+{paid_cap:.0} paid cap allowed) -> {remaining:.0} min placeable"
            ),
            free_slots: None,
            remaining_total_minutes: Some(remaining),
        }
    }
    fn dispatch(&self, job: &Job, gh: &dyn Gh) -> R<DispatchResult> {
        // Same workflow as the github-actions surface hosting it;
        // the inner adapter carries force_runner = "blacksmith", so
        // the workflow inputs carry runner=blacksmith.
        self.inner.dispatch_workflow(job, gh)
    }
    fn status(&self, session: &Value, gh: &dyn Gh) -> String {
        self.inner.run_state(session, gh)
    }
}

pub struct CodespaceAdapter {
    surface: Value,
    scope: RefCell<Option<bool>>,
}

impl CodespaceAdapter {
    fn scope_ok(&self, gh: &dyn Gh) -> bool {
        let mut slot = self.scope.borrow_mut();
        if let Some(v) = *slot {
            return v;
        }
        let ok = gh.codespace_list().is_ok();
        *slot = Some(ok);
        ok
    }
}

impl Adapter for CodespaceAdapter {
    fn kind(&self) -> &'static str {
        "codespace"
    }
    fn surface(&self) -> &Value {
        &self.surface
    }
    fn eligible(&self, job: &Job, ctx: &Ctx, gh: &dyn Gh) -> (bool, String) {
        // The surface's registry status is pending-scope while the
        // OAuth scope is ungranted; the live probe is the real gate
        // either way (reference behavior).
        if !self.scope_ok(gh) {
            return (
                false,
                "gh codespace list fails (codespace OAuth scope not granted \
                 on this token) — surface pending-scope"
                    .to_string(),
            );
        }
        if job.platform != "linux" {
            return (
                false,
                "Codespaces provide Linux workspaces only".to_string(),
            );
        }
        let explicitly_selected = job.surface_pin.as_deref() == Some(surf_str(&self.surface, "id"))
            || job.preferred_surface.as_deref() == Some(surf_str(&self.surface, "id"));
        if !ctx.laptop_busy && !explicitly_selected {
            return (
                false,
                "codespaces are the last resort: laptop is not marked \
                 saturated, so this tier is not reached"
                    .to_string(),
            );
        }
        if !surface_targets_job(&self.surface, job) {
            return (
                false,
                format!("{} is not in this surface's targets", job.repo),
            );
        }
        (true, "eligible".to_string())
    }
    fn capacity(&self, _job: &Job, _ctx: &Ctx) -> Capacity {
        let core_hours = self
            .surface
            .get("budget")
            .and_then(|b| b.get("core_hours_monthly"))
            .and_then(|v| v.as_f64())
            .unwrap_or(180.0);
        let max_live = self
            .surface
            .get("max_live")
            .and_then(|v| v.as_i64())
            .unwrap_or(3);
        Capacity {
            summary: format!(
                "codespace budget: {core_hours} core-hours/mo (Pro); \
                 reuse-only, max {max_live} live"
            ),
            free_slots: None,
            remaining_total_minutes: None,
        }
    }
    fn dispatch(&self, job: &Job, gh: &dyn Gh) -> R<DispatchResult> {
        let spaces = gh
            .codespace_list()
            .map_err(|_| Die("codespace list failed at dispatch time (scope lost?)".to_string()))?;
        let space = spaces
            .iter()
            .find(|c| c.get("repository").and_then(|r| r.as_str()) == Some(job.repo.as_str()));
        let space = match space {
            Some(s) => s,
            None => {
                return die(format!(
                    "codespace surface selected but no existing codespace for {} \
                     to reuse — fleet-place never creates codespaces; create one \
                     deliberately, then resubmit",
                    job.repo
                ));
            }
        };
        let name = space
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        // Honor the requested revision (Codex P2 on PR #8325):
        // the reused codespace must be checked out at the
        // requested ref; fleet-place validates and refuses rather
        // than running against different code than the session
        // records.
        if !job.git_ref.is_empty() {
            let esc = job.git_ref.replace('\'', "'\\''");
            let (hcode, head) = gh.codespace_ssh_capture(&name, "git rev-parse HEAD")?;
            let (tcode, target) = gh.codespace_ssh_capture(
                &name,
                &format!(
                    "git rev-parse --verify '{esc}^{{commit}}' 2>/dev/null || git rev-parse --verify 'origin/{esc}^{{commit}}' 2>/dev/null"
                ),
            )?;
            if hcode != 0 || head.is_empty() || tcode != 0 || target.is_empty() {
                return die(format!(
                    "codespace {name}: cannot validate the requested ref {} \u{2014} \
                     the reused codespace's checkout could not be resolved \
                     against it; check out the ref in the codespace and \
                     resubmit, or drop --ref",
                    job.git_ref
                ));
            }
            if head != target {
                return die(format!(
                    "codespace {name} is checked out at {head}, not the \
                     requested ref {} ({target}) \u{2014} fleet-place does not \
                     move a reused codespace's checkout; check out the ref \
                     in the codespace and resubmit, or drop --ref",
                    job.git_ref
                ));
            }
        }
        let code = gh.codespace_ssh(&name, &job.command)?;
        Ok(DispatchResult {
            codespace: Some(name.clone()),
            note: Some(format!(
                "job handed to a session in codespace {name} (ssh exit {code})"
            )),
            // The ssh call IS the execution: its exit code is the
            // terminal outcome (Codex P2 on PR #8325).
            terminal_state: Some(if code == 0 { "succeeded" } else { "failed" }.to_string()),
            ..Default::default()
        })
    }
}

pub struct LaptopAdapter {
    surface: Value,
}

impl Adapter for LaptopAdapter {
    fn kind(&self) -> &'static str {
        "laptop"
    }
    fn surface(&self) -> &Value {
        &self.surface
    }
    fn eligible(&self, job: &Job, _ctx: &Ctx, _gh: &dyn Gh) -> (bool, String) {
        if job.surface_pin.as_deref() == Some(surf_str(&self.surface, "id")) {
            return (true, "job pins the laptop surface".to_string());
        }
        (
            false,
            "laptop work is owned by pm-dispatch/repo-queue lanes; \
             fleet-place only records pinned handoffs"
                .to_string(),
        )
    }
    fn capacity(&self, _job: &Job, ctx: &Ctx) -> Capacity {
        Capacity {
            summary: format!(
                "laptop busy flag: {} ({})",
                ctx.laptop_busy, ctx.laptop_busy_source
            ),
            free_slots: None,
            remaining_total_minutes: None,
        }
    }
    fn dispatch(&self, _job: &Job, _gh: &dyn Gh) -> R<DispatchResult> {
        Ok(DispatchResult {
            note: Some(
                "handoff recorded; execution belongs to the laptop's \
                 pm-dispatch/repo-queue lanes"
                    .to_string(),
            ),
            ..Default::default()
        })
    }
}

fn adapter_for(surface: &Value) -> Option<Box<dyn Adapter>> {
    match surf_str(surface, "kind") {
        "github-actions" => Some(Box::new(GithubActionsAdapter {
            surface: surface.clone(),
            force_runner: "github",
        })),
        "blacksmith" => Some(Box::new(BlacksmithAdapter {
            inner: GithubActionsAdapter {
                surface: surface.clone(),
                force_runner: "blacksmith",
            },
        })),
        "codespace" => Some(Box::new(CodespaceAdapter {
            surface: surface.clone(),
            scope: RefCell::new(None),
        })),
        "laptop" => Some(Box::new(LaptopAdapter {
            surface: surface.clone(),
        })),
        _ => None,
    }
}

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
    let mut read_errors: Vec<String> = Vec::new();
    for (owner, repos) in &hosts {
        let mut running: i64 = 0;
        let mut queued: i64 = 0;
        for repo in repos {
            let minted = if user_repos.contains(repo) {
                gh.user_token()
            } else {
                gh.mint_token(repo)
            };
            let token = match minted {
                Some(t) => t,
                None => {
                    read_errors.push(format!(
                        "{repo}: no token available (not covered) — load counted as 0"
                    ));
                    continue;
                }
            };
            for status_name in ["in_progress", "queued"] {
                match gh.api(
                    "GET",
                    &format!("/repos/{repo}/actions/runs?status={status_name}&per_page=1"),
                    &token,
                    None,
                ) {
                    Ok((200, payload)) => {
                        let total = payload
                            .get("total_count")
                            .and_then(|t| t.as_i64())
                            .unwrap_or(0);
                        if status_name == "in_progress" {
                            running += total;
                        } else {
                            queued += total;
                        }
                    }
                    Ok((code, _)) => {
                        read_errors.push(format!("{repo} ({status_name}): HTTP {code}"))
                    }
                    Err(e) => read_errors.push(format!("{repo} ({status_name}): {}", e.0)),
                }
            }
        }
        if let Some(n) = overrides.get(owner) {
            running = *n;
            queued = 0;
        }
        owner_load.insert(owner.clone(), (running, queued));
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
    if let Some(sessions) = store.get_mut("sessions").and_then(|s| s.as_object_mut()) {
        sessions.insert(handle.to_string(), rec.clone());
    }
    Ok(rec)
}

// Command-line parsing, rendering, and dispatch/session entry points.
mod cli;
pub use cli::{cmd_dispatch, cmd_session, cmd_surfaces};
