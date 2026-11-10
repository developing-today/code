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

use serde_json::{json, Map, Value};
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
    pub surface_pin: Option<String>,
    pub push_nix_cache: bool,
    pub actions_cache: Option<String>,
    pub allow_paid_blacksmith: f64,
}

impl Job {
    fn to_json(&self) -> Value {
        json!({
            "repo": self.repo,
            "command": self.command,
            "ref": self.git_ref,
            "runner": self.runner,
            "surface_pin": self.surface_pin,
            "push_nix_cache": self.push_nix_cache,
            "actions_cache": self.actions_cache,
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
                || v.as_f64() == Some(0.0);
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
    fn api(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> R<(u16, Value)>;
    /// `gh codespace list --json name,repository,state`, parsed.
    fn codespace_list(&self) -> R<Vec<Value>>;
    /// `gh codespace ssh -c NAME -- COMMAND`; returns the exit code.
    fn codespace_ssh(&self, name: &str, command: &str) -> R<i32>;
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
        self.tokens.borrow_mut().insert(repo.to_string(), tok.clone());
        Some(tok)
    }

    fn api(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> R<(u16, Value)> {
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
}

// ---------------------------------------------------------------- registry

fn surf_str<'a>(surface: &'a Value, key: &str) -> &'a str {
    surface.get(key).and_then(|v| v.as_str()).unwrap_or("")
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
        return (
            false,
            format!("surface status is '{status}', not 'active'"),
        );
    }
    let targets = surf_targets(surface);
    if !targets.iter().any(|t| t == "*") && !targets.iter().any(|t| t == &job.repo) {
        return (
            false,
            format!("{} is not in this surface's targets", job.repo),
        );
    }
    (true, "eligible".to_string())
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
    fn workflow_inputs(&self, job: &Job) -> Value {
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
            let (key, _, path) = match ac.split_once(':') {
                Some((k, p)) => (k, (), p),
                None => (ac.as_str(), (), ""),
            };
            cache.insert("actions_key".into(), json!(key));
            cache.insert(
                "actions_path".into(),
                json!(if path.is_empty() { "." } else { path }),
            );
        }
        let mut inputs = Map::new();
        if !cache.is_empty() {
            entry.insert("cache".into(), Value::Object(cache));
            inputs.insert("jobs".into(), json!(Value::Array(vec![Value::Object(entry)]).to_string()));
        } else {
            inputs.insert("repo".into(), json!(job.repo));
            inputs.insert("command".into(), json!(job.command));
        }
        if self.runner_name(job) == "blacksmith" {
            inputs.insert("runner".into(), json!("blacksmith"));
        }
        if !job.git_ref.is_empty() {
            inputs.insert("ref".into(), json!(job.git_ref));
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
        let token = gh.app_token(&repo)?;
        let inputs = self.workflow_inputs(job);
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
        // Find the run the dispatch created (newest workflow_dispatch
        // run at/after the dispatch instant).
        let since = gh.now_epoch() - 5;
        let mut found: Option<(i64, String)> = None;
        for _ in 0..12 {
            gh.sleep_secs(3);
            let (_st, runs) = gh.api(
                "GET",
                &format!(
                    "/repos/{repo}/actions/workflows/{workflow}/runs?event=workflow_dispatch&per_page=5"
                ),
                &token,
                None,
            )?;
            for r in runs
                .get("workflow_runs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let created = r
                    .get("created_at")
                    .and_then(|c| c.as_str())
                    .and_then(parse_iso)
                    .unwrap_or(0);
                if created >= since {
                    found = Some((
                        r.get("id").and_then(|i| i.as_i64()).unwrap_or(0),
                        r.get("html_url")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ));
                    break;
                }
            }
            if found.is_some() {
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
                    .to_string()
            }
        };
        let repo = surf_str(&self.surface, "repo");
        let token = match gh.mint_token(repo) {
            Some(t) => t,
            None => {
                return session
                    .get("state")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown")
                    .to_string()
            }
        };
        match gh.api("GET", &format!("/repos/{repo}/actions/runs/{run_id}"), &token, None) {
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
    fn eligible(&self, job: &Job, ctx: &Ctx, _gh: &dyn Gh) -> (bool, String) {
        let _ = ctx;
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
        (true, "eligible".to_string())
    }
    fn capacity(&self, _job: &Job, ctx: &Ctx) -> Capacity {
        let owner = surf_str(&self.surface, "owner");
        let (running, queued) = ctx
            .owner_load
            .get(owner)
            .copied()
            .unwrap_or((0, 0));
        let cap = ctx.owner_cap(owner);
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
        let b = self.inner.surface.get("budget").cloned().unwrap_or(json!({}));
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
    fn eligible(&self, job: &Job, _ctx: &Ctx, _gh: &dyn Gh) -> (bool, String) {
        if surf_str(&self.inner.surface, "owner") == "dezren39" {
            return (
                false,
                "Blacksmith is orgs-only; never the personal copy".to_string(),
            );
        }
        let (ok, why) = base_eligible(&self.inner.surface, job);
        if !ok {
            return (ok, why);
        }
        if job.runner == "github" {
            return (
                false,
                "job pins runner=github".to_string(),
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
        if !ctx.laptop_busy {
            return (
                false,
                "codespaces are the last resort: laptop is not marked \
                 saturated, so this tier is not reached"
                    .to_string(),
            );
        }
        let targets = surf_targets(&self.surface);
        if !targets.iter().any(|t| t == "*") && !targets.iter().any(|t| t == &job.repo) {
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
        let spaces = gh.codespace_list().map_err(|_| {
            Die("codespace list failed at dispatch time (scope lost?)".to_string())
        })?;
        let space = spaces.iter().find(|c| {
            c.get("repository").and_then(|r| r.as_str()) == Some(job.repo.as_str())
        });
        let space = match space {
            Some(s) => s,
            None => {
                return die(format!(
                    "codespace surface selected but no existing codespace for {} \
                     to reuse — fleet-place never creates codespaces; create one \
                     deliberately, then resubmit",
                    job.repo
                ))
            }
        };
        let name = space
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        let code = gh.codespace_ssh(&name, &job.command)?;
        Ok(DispatchResult {
            codespace: Some(name.clone()),
            note: Some(format!(
                "job handed to a session in codespace {name} (ssh exit {code})"
            )),
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
    for s in registry_surfaces(registry) {
        let kind = surf_str(&s, "kind");
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
            let token = match gh.mint_token(repo) {
                Some(t) => t,
                None => {
                    read_errors.push(format!(
                        "{repo}: app token unavailable (not covered) — load counted as 0"
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

    // Tier 1 — free GitHub-hosted Actions capacity, least-loaded first.
    let mut tier1: Vec<(i64, String)> = Vec::new();
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
                tier1.push((free, id));
            }
        }
    }
    if !tier1.is_empty() {
        tier1.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let sid = tier1[0].1.clone();
        reasoning.push(format!(
            "tier1 pick {sid}: most free slots among eligible GitHub-hosted surfaces"
        ));
        let surface = find_surface(registry, &sid).unwrap().clone();
        return Ok((adapter_for(&surface), reasoning));
    }
    reasoning.push(
        "tier1 exhausted: no eligible GitHub-hosted surface has a free slot".to_string(),
    );

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
        .ok_or_else(|| Die(format!("no session with handle {handle} in {}", state_file())))?;
    let run_id = rec.get("run_id").and_then(|r| r.as_i64());
    let surface_id = rec
        .get("surface")
        .and_then(|s| s.as_str())
        .map(str::to_string);
    if let (Some(_), Some(sid)) = (run_id, surface_id) {
        if let Some(surface) = find_surface(registry, &sid) {
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
                    if let Some(token) = gh.mint_token(&repo) {
                        if let Ok((200, run)) = gh.api(
                            "GET",
                            &format!("/repos/{repo}/actions/runs/{}", run_id.unwrap()),
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
                                        if d.get("handle").and_then(|h| h.as_str())
                                            == Some(handle)
                                        {
                                            d["minutes"] = json!(minutes);
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

// --------------------------------------------------------------------- CLI

struct Cli {
    registry: Option<String>,
    json: bool,
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

const DISPATCH_USAGE: &str = "usage: repo-queue dispatch --repo <owner/name> --command '<cmd>' \
[--ref SHA] [--runner auto|github|blacksmith] [--surface ID] [--push-nix-cache] \
[--actions-cache KEY:PATH] [--allow-paid-blacksmith MINUTES] [--laptop-busy] [--dispatch] \
[--json] [--registry PATH]\nDry-run unless --dispatch; prints the placement decision.";

fn run_dispatch(args: &[String]) -> R<i32> {
    let (cli, rest) = extract_common(args)?;
    let mut repo: Option<String> = None;
    let mut command: Option<String> = None;
    let mut git_ref = String::new();
    let mut runner = "auto".to_string();
    let mut surface_pin: Option<String> = None;
    let mut push_nix_cache = false;
    let mut actions_cache: Option<String> = None;
    let mut allow_paid: f64 = 0.0;
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
            "--push-nix-cache" => push_nix_cache = true,
            "--actions-cache" => {
                i += 1;
                actions_cache = Some(need(&rest, i, "--actions-cache")?);
            }
            "--allow-paid-blacksmith" => {
                i += 1;
                let v = need(&rest, i, "--allow-paid-blacksmith")?;
                allow_paid = v
                    .parse::<f64>()
                    .map_err(|_| Die(format!("--allow-paid-blacksmith: not a number: {v}")))?;
            }
            "--laptop-busy" => laptop_busy_flag = true,
            "--assume-owner-load" => {
                i += 1;
                assume_owner_load = need(&rest, i, "--assume-owner-load")?;
            }
            "--assume-blacksmith-used" => {
                i += 1;
                let v = need(&rest, i, "--assume-blacksmith-used")?;
                assume_blacksmith_used = Some(
                    v.parse::<f64>()
                        .map_err(|_| Die(format!("--assume-blacksmith-used: not a number: {v}")))?,
                );
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
    let job = Job {
        repo: repo.ok_or_else(|| Die("--repo is required".to_string()))?,
        command: command.ok_or_else(|| Die("--command is required".to_string()))?,
        git_ref,
        runner,
        surface_pin,
        push_nix_cache,
        actions_cache,
        allow_paid_blacksmith: allow_paid,
    };
    let registry = load_registry(&registry_path(&cli))?;
    let gh = RealGh::new();
    let store = load_store();
    let ctx = build_ctx(
        &registry,
        &store,
        laptop_busy_flag,
        &assume_owner_load,
        assume_blacksmith_used,
        &gh,
    );
    let (adapter, reasoning) = decide(&registry, &job, &ctx, &gh)?;
    let mut out = Map::new();
    out.insert("job".into(), job.to_json_compact());
    out.insert(
        "decision".into(),
        json!(adapter
            .as_ref()
            .map(|a| surf_str(a.surface(), "id").to_string())
            .unwrap_or_else(|| "queued".to_string())),
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
    if !dispatch {
        out.insert(
            "mode".into(),
            json!("dry-run (no session written; pass --dispatch to submit)"),
        );
        print_out(&Value::Object(out), cli.json, render_decision);
        return Ok(0);
    }
    let mut store = store;
    let handle = new_handle(&rand_source::URandom);
    match adapter {
        None => {
            let rec = json!({
                "handle": handle,
                "job": job.to_json(),
                "surface": Value::Null,
                "decision_reasoning": reasoning,
                "run_id": Value::Null,
                "run_url": Value::Null,
                "state": "queued",
                "submitted_by": submitted_by(),
                "actor": "developing-today[bot] (GitHub App dispatch)",
                "created_at": now_iso(),
                "updated_at": now_iso(),
            });
            if let Some(sessions) = store["sessions"].as_object_mut() {
                sessions.insert(handle.clone(), rec);
            }
            save_store(&store)?;
            out.insert("handle".into(), json!(handle));
            out.insert("state".into(), json!("queued"));
        }
        Some(adapter) => {
            let result = adapter.dispatch(&job, &gh)?;
            let surface = adapter.surface().clone();
            let sid = surf_str(&surface, "id").to_string();
            let kind = surf_str(&surface, "kind").to_string();
            let mut rec = Map::new();
            rec.insert("handle".into(), json!(handle));
            rec.insert("job".into(), job.to_json());
            rec.insert("surface".into(), json!(sid));
            rec.insert("surface_kind".into(), json!(kind));
            rec.insert("decision_reasoning".into(), json!(reasoning));
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
                json!(if result.run_id.is_some() {
                    "queued"
                } else {
                    "dispatched"
                }),
            );
            rec.insert("submitted_by".into(), json!(submitted_by()));
            rec.insert(
                "actor".into(),
                json!("developing-today[bot] (GitHub App dispatch)"),
            );
            rec.insert("created_at".into(), json!(now_iso()));
            rec.insert("updated_at".into(), json!(now_iso()));
            if let Some(note) = &result.note {
                rec.insert("note".into(), json!(note));
            }
            if let Some(cs) = &result.codespace {
                rec.insert("codespace".into(), json!(cs));
            }
            if let Some(sessions) = store["sessions"].as_object_mut() {
                sessions.insert(handle.clone(), Value::Object(rec));
            }
            if kind == "blacksmith" {
                store["blacksmith_dispatches"].as_array_mut().map(|arr| {
                    arr.push(json!({
                        "handle": handle,
                        "month": current_month(),
                        "minutes": 0.0,
                        "run_id": result.run_id,
                        "note": "minutes reconciled from the run's duration on later invocations",
                    }))
                });
            }
            save_store(&store)?;
            out.insert("handle".into(), json!(handle));
            out.insert(
                "run_id".into(),
                result.run_id.map(Value::from).unwrap_or(Value::Null),
            );
            out.insert(
                "run_url".into(),
                result.run_url.map(Value::from).unwrap_or(Value::Null),
            );
        }
    }
    print_out(&Value::Object(out), cli.json, render_decision);
    Ok(0)
}

const SESSION_USAGE: &str = "usage: repo-queue session <status|result|list> [HANDLE] \
[--mine] [--json] [--registry PATH]";

fn run_session(args: &[String]) -> R<i32> {
    let (cli, rest) = extract_common(args)?;
    let sub = rest.first().map(|s| s.as_str()).unwrap_or("");
    let gh = RealGh::new();
    match sub {
        "status" | "result" => {
            let handle = rest
                .get(1)
                .cloned()
                .ok_or_else(|| Die(format!("session {sub} needs a HANDLE")))?;
            let registry = load_registry(&registry_path(&cli))?;
            let mut store = load_store();
            let rec = refresh_session(&mut store, &handle, &registry, &gh)?;
            save_store(&store)?;
            if sub == "status" {
                print_out(&rec, cli.json, |r| {
                    format!(
                        "{}: state={} surface={} run={}",
                        r.get("handle").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("state").and_then(|v| v.as_str()).unwrap_or(""),
                        r.get("surface")
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "null".to_string()),
                        r.get("run_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("None"),
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
                        r.get("terminal")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        r.get("run_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("None"),
                    )
                });
            }
            Ok(0)
        }
        "list" => {
            let mine = rest.iter().any(|a| a == "--mine");
            let store = load_store();
            let me = submitted_by();
            let mut sessions: Vec<Value> = store
                .get("sessions")
                .and_then(|s| s.as_object())
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default();
            sessions.sort_by(|a, b| {
                b.get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(a.get("created_at").and_then(|v| v.as_str()).unwrap_or(""))
            });
            if mine {
                sessions
                    .retain(|s| s.get("submitted_by").and_then(|v| v.as_str()) == Some(me.as_str()));
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
                            r.get("run_url")
                                .and_then(|v| v.as_str())
                                .unwrap_or("None"),
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

fn run_surfaces(args: &[String]) -> R<i32> {
    let (cli, _rest) = extract_common(args)?;
    let registry = load_registry(&registry_path(&cli))?;
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

    #[derive(Default)]
    struct FakeGh {
        tokens_ok: bool,
        workflow_runs: Vec<Value>,
        runs_by_id: Map2<i64, Value>,
        load_by_repo: Map2<String, (i64, i64)>, // repo -> (in_progress, queued)
        codespace_ok: bool,
        codespace_items: Vec<Value>,
        posts: RefCell<Vec<(String, Value)>>,
    }

    impl Gh for FakeGh {
        fn mint_token(&self, _repo: &str) -> Option<String> {
            if self.tokens_ok {
                Some("installation-token".to_string())
            } else {
                None
            }
        }
        fn api(
            &self,
            method: &str,
            path: &str,
            _token: &str,
            body: Option<&Value>,
        ) -> R<(u16, Value)> {
            if method == "POST" {
                self.posts
                    .borrow_mut()
                    .push((path.to_string(), body.cloned().unwrap_or(Value::Null)));
                return Ok((204, json!({})));
            }
            if path.contains("/runs?event=workflow_dispatch") {
                return Ok((200, json!({ "workflow_runs": self.workflow_runs })));
            }
            if path.contains("/actions/runs?status=") {
                let repo = path
                    .trim_start_matches("/repos/")
                    .split("/actions/")
                    .next()
                    .unwrap_or("")
                    .to_string();
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
            Ok(0)
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
                  "app_installation": 175747567, "status": "active",
                  "targets": ["developing-today/code"] },
                { "id": "org-run", "kind": "github-actions", "owner": "developing-today",
                  "repo": "developing-today/code", "workflow": "org-run.yml",
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
            surface_pin: None,
            push_nix_cache: false,
            actions_cache: None,
            allow_paid_blacksmith: 0.0,
        }
    }

    fn ctx(load: &[(&str, (i64, i64))], blacksmith_used: f64, laptop_busy: bool) -> Ctx {
        Ctx {
            owners: registry()["owners"].clone(),
            owner_load: load
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
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
    fn tier1_picks_most_free_slots() {
        let reg = registry();
        let gh = FakeGh::default();
        // Personal copy has 40 free, org-run 20 free -> personal wins.
        let c = ctx(&[("dezren39", (0, 0)), ("developing-today", (0, 0))], 0.0, false);
        assert_eq!(pick(&reg, &job(), &c, &gh).as_deref(), Some("fleet-personal"));
        // Personal saturated, org partially loaded -> org-run.
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
        let c = ctx(&[("dezren39", (0, 0)), ("developing-today", (0, 0))], 0.0, false);
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
        assert!(reasoning
            .last()
            .map(|r| r.contains("stays queued"))
            .unwrap_or(false));
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
        let c = ctx(&[("dezren39", (0, 0)), ("developing-today", (0, 0))], 0.0, false);
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
        assert!(result
            .note
            .as_deref()
            .unwrap_or("")
            .contains("handoff recorded"));
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
        let inputs = ad.workflow_inputs(&job());
        assert_eq!(inputs["repo"], json!("developing-today/code"));
        assert_eq!(inputs["command"], json!("nix build .#mcpx"));
        assert!(inputs.get("jobs").is_none());
        assert!(inputs.get("runner").is_none());
        // Cache options travel in a jobs entry.
        let mut j = job();
        j.push_nix_cache = true;
        j.actions_cache = Some("tests:./out".to_string());
        j.git_ref = "abc123".to_string();
        let inputs = ad.workflow_inputs(&j);
        let jobs: Value = serde_json::from_str(inputs["jobs"].as_str().unwrap()).unwrap();
        assert_eq!(jobs[0]["cache"]["push_nix"], json!(true));
        assert_eq!(jobs[0]["cache"]["actions_key"], json!("tests"));
        assert_eq!(jobs[0]["cache"]["actions_path"], json!("./out"));
        assert_eq!(inputs["ref"], json!("abc123"));
        // Blacksmith variant sends the runner input.
        let bs_surface = find_surface(&reg, "fleet-org-blacksmith").unwrap();
        let bs = GithubActionsAdapter {
            surface: bs_surface.clone(),
            force_runner: "blacksmith",
        };
        let inputs = bs.workflow_inputs(&job());
        assert_eq!(inputs["runner"], json!("blacksmith"));
    }

    #[test]
    fn dispatch_posts_as_app_and_finds_run() {
        let reg = registry();
        let surface = find_surface(&reg, "fleet-personal").unwrap().clone();
        let ad = adapter_for(&surface).unwrap();
        let gh = FakeGh {
            tokens_ok: true,
            workflow_runs: vec![json!({
                "id": 4242,
                "html_url": "https://github.com/dezren39/fleet/actions/runs/4242",
                "created_at": "2026-01-01T00:00:10Z"
            })],
            ..Default::default()
        };
        let result = ad.dispatch(&job(), &gh).unwrap();
        assert_eq!(result.run_id, Some(4242));
        let posts = gh.posts.borrow();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].0.contains("/repos/dezren39/fleet/actions/workflows/fleet-run.yml/dispatches"));
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
        let gh = FakeGh::default(); // no tokens: load reads error out, counted 0
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
    fn store_roundtrip_preserves_sessions() {
        let dir = std::env::temp_dir().join(format!(
            "fleet-place-test-{}",
            std::process::id()
        ));
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
}
