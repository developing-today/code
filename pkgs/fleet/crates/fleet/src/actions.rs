//! `fleet actions` — developing-today/code#13774: live short-lived
//! exec sessions inside GitHub Actions runners, coordinator-
//! handed-off.
//!
//! The family has three leaves:
//!
//! - `fleet actions exec …` — the seat half. `start` opens a
//!   session row at the queue broker (the coordinator), attempts
//!   the #13765 placement claim (falling back to the singular path,
//!   named in the output via `placement_path`, while those verbs
//!   are not live), dispatches the `fleet-exec.yml` workflow with
//!   only the session handle and non-secret parameters, and binds
//!   the resulting run — the handoff is the handle plus the
//!   portable `OWNER/REPO#RUN_ID` + attempt form. `once` is the
//!   one-shot sugar over the same path: it finds a live session
//!   for the host repo in the local store or starts one (same
//!   defaults, same claim, same handoff), runs one command, and
//!   exits with its code. `run` sends one command and streams its
//!   IO back; `attach` / `watch` stream a session after handoff;
//!   `renew` extends the lease within the max
//!   lifespan; `extend` asks past the max (the broker raises
//!   the needs-drew alert; Drew's answer returns via `decide`);
//!   `close` ends the session.
//! - `fleet actions agent …` — the runner half, run by the
//!   fleet-exec-session composite action. It proves run membership
//!   with the job's GitHub OIDC token (fetched from the Actions
//!   runtime, never logged), registers with the broker's public
//!   door, long-polls for commands, executes them in the workdir,
//!   and streams IO back through the broker. The lease is enforced
//!   locally too: the agent exits at expiry even if partitioned
//!   from the broker.
//! - `fleet actions put <file>` — the egress helper (the agent
//!   installs it on the session PATH as `fleet-exec-put`): copies
//!   the file into the session outbox (uploaded as the run's
//!   artifact) and, when the session carries R2 credentials, PUTs
//!   it to the bucket with a SigV4 signature. No credential value
//!   is ever printed.
//!
//! Lease discipline: `start` requires `--timeout` — the lease —
//! by construction (there is no default); `--max-lifespan` caps
//! the session (default 30 min, ceiling 2 h, the fleet leg cap).

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_config::Config;
use serde_json::{Value, json};

pub const ACTIONS_USAGE: &str = "\
Usage: fleet actions <command> [args...]

Live short-lived exec sessions inside GitHub Actions runners
(#13774). Commands:

  exec start --repo OWNER/REPO --timeout S [--max-lifespan S]
             [--ref R] [--target-repo OWNER/REPO] [--hide-logs]
             [--detach] [--broker-url URL]
             Open a session, dispatch fleet-exec.yml, print the
             handoff (handle + OWNER/REPO#RUN_ID + attempt), then
             attach unless --detach. --timeout is the lease and is
             REQUIRED — there is no default. --repo is the workflow
             host repo (default dezren39/fleet, where fleet-exec.yml
             lives); its OIDC identity is what the broker binds.
  exec run <ref> [--timeout S] -- <command...>
             Run one command on the session's runner; streams its
             output and exits with the command's exit code. <ref>
             is a handle or the portable OWNER/REPO#RUN_ID form.
  exec once [--repo OWNER/REPO] [--target-repo OWNER/REPO]
             [--lease S] [--max-lifespan S] [--timeout S]
             [--broker-url URL] [--context TEXT] [--hide-logs]
             [--close] -- <command...>
             One-shot sugar over start+run: find a live session
             for --repo in the local store or start one (same
             claim, same handoff), run one command, exit with its
             code. Defaults: --repo dezren39/fleet, --lease 1800,
             --max-lifespan 1800, --timeout 600. The session stays
             open for the next once unless --close.
  exec attach <ref>
             Stream the session's IO (full replay) until it ends.
  exec watch <ref>
             Stream IO and report run/session status as it changes.
  exec renew <ref> --timeout S
             Extend the lease, clamped at the max lifespan.
  exec extend <ref> [--minutes M] [--reason TEXT]
             Request an emergency extension past the max lifespan;
             the broker raises a needs-drew alert for Drew's
             approve/deny.
  exec decide <ref> --approve|--deny
             Record Drew's answer to a pending extension request.
  exec close <ref>
             Close the session.
  agent --handle H --broker-url URL --lease-seconds S
             --max-seconds S --workdir D --outbox D [--hide-logs]
             The runner-side agent (invoked by the Action).
  put <file> Copy a file to the session outbox (+ R2 when the
             session carries credentials). Installed on the
             session PATH as fleet-exec-put.";

const EXIT_OK: i32 = 0;
const EXIT_ERROR: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_TRANSPORT: i32 = 3;

const DEFAULT_BROKER_URL: &str = "https://exec.security.cab";
const DEFAULT_HOST_REPO: &str = "dezren39/fleet";
const DEFAULT_WORKFLOW: &str = "fleet-exec.yml";
const GH_TOKEN_SECRET: &str = "github.ghcr-classic-pat";

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn seat_name() -> String {
    if let Ok(s) = std::env::var("FLEET_SEAT") {
        if !s.is_empty() {
            return s;
        }
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "seat".to_string());
    let host = std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown-host".to_string());
    format!("{user}@{host}")
}

// ---------------------------------------------------------------------------
// Local session store: handle -> handoff record, so the portable
// OWNER/REPO#RUN_ID form resolves on the seat that started it.
// ---------------------------------------------------------------------------

fn store_path() -> std::path::PathBuf {
    fleet_config::Config::state_dir().join("actions-exec-sessions.json")
}

fn store_load() -> Value {
    std::fs::read_to_string(store_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn store_save(store: &Value) {
    let path = store_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(store) {
        let _ = std::fs::write(&path, text);
    }
}

fn store_put(record: Value) {
    let mut store = store_load();
    if let (Some(obj), Some(handle)) = (
        store.as_object_mut(),
        record
            .get("handle")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    ) {
        obj.insert(handle, record);
        store_save(&store);
    }
}

/// Resolve a user reference — a bare handle or the portable
/// OWNER/REPO#RUN_ID form — to a session handle.
fn resolve_ref(reference: &str) -> Result<String, String> {
    if let Some((repo, run)) = reference.rsplit_once('#') {
        if repo.contains('/') && run.chars().all(|c| c.is_ascii_digit()) && !run.is_empty() {
            let store = store_load();
            if let Some(obj) = store.as_object() {
                for (handle, rec) in obj {
                    if rec.get("host_repo").and_then(|v| v.as_str()) == Some(repo)
                        && rec.get("run_id").and_then(|v| v.as_i64()) == run.parse::<i64>().ok()
                    {
                        return Ok(handle.clone());
                    }
                }
            }
            return Err(format!(
                "no local session record for {reference} (the portable form resolves on the seat that started the session; use the handle there)"
            ));
        }
    }
    Ok(reference.to_string())
}

// ---------------------------------------------------------------------------
// Broker envelope + GitHub helpers
// ---------------------------------------------------------------------------

fn verb_err(v: &Value) -> String {
    v.get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("broker returned an error envelope")
        .to_string()
}

fn check_envelope(v: Value) -> Result<Value, (i32, String)> {
    if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
        Ok(v)
    } else {
        let code = v.get("exit").and_then(|e| e.as_i64()).unwrap_or(1) as i32;
        Err((code, verb_err(&v)))
    }
}

fn gh_token(config: &Config) -> Result<String, String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            if !t.is_empty() {
                return Ok(t);
            }
        }
    }
    if let Ok(out) = Command::new("gh").args(["auth", "token"]).output() {
        if out.status.success() {
            let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !t.is_empty() {
                return Ok(t);
            }
        }
    }
    fleet_secrets::Resolver::default_chain(config)
        .resolve(GH_TOKEN_SECRET)
        .map(|s| s.expose().to_string())
        .map_err(|e| format!("no GitHub token available (env, gh auth, or secret by name): {e}"))
}

fn gh_api(
    config: &Config,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let token = gh_token(config)?;
    let url = format!("https://api.github.com{path}");
    // Platform root store, not webpki-roots alone: seats behind a
    // TLS-intercepting egress proxy (the sandbox) present the proxy's
    // CA, which only the platform store carries — with the default
    // agent the dispatch died with UnknownIssuer (code#15575; same
    // fix as the gh App leg, code#13122).
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut resp = if method == "POST" {
        agent
            .post(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Authorization", &format!("Bearer {token}"))
            .header("User-Agent", "fleet-actions")
            .header("Content-Type", "application/json")
            .send(
                serde_json::to_string(&body.unwrap_or(&Value::Null))
                    .unwrap_or_default()
                    .as_str(),
            )
            .map_err(|e| gh_transport_err(method, path, &e))?
    } else {
        agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Authorization", &format!("Bearer {token}"))
            .header("User-Agent", "fleet-actions")
            .call()
            .map_err(|e| gh_transport_err(method, path, &e))?
    };
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("github api {method} {path}: unreadable body: {e}"))?;
    if !(200..300).contains(&status) {
        let snippet: String = text.chars().take(200).collect();
        return Err(format!(
            "github api {method} {path}: HTTP {status}: {snippet}"
        ));
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|e| format!("github api {method} {path}: bad JSON: {e}"))
}

/// Actionable hint for seat-side GitHub API transport failures that look
/// like TLS certificate trust (code#15575). Returns `None` for
/// non-TLS failures so ordinary transport errors stay terse.
fn tls_trust_hint(err: &str) -> Option<&'static str> {
    let lower = err.to_lowercase();
    let looks_like_tls = [
        "unknownissuer",
        "unknown issuer",
        "invalid peer certificate",
        "certificate verify failed",
        "certificate has expired",
        "self-signed",
        "self signed",
        "unable to get local issuer",
        "certificate",
        "rustls",
        "x509",
        "tls handshake",
        "ssl certificate",
    ]
    .iter()
    .any(|sig| lower.contains(sig));
    if !looks_like_tls {
        return None;
    }
    Some(
        "seat TLS trust: the GitHub API TLS handshake failed certificate validation — \
         seats behind the sandbox TLS-intercepting egress proxy need a fleet binary carrying \
         code#15575 (platform certificate roots). Re-run scripts/muse/bootstrap.sh to refresh \
         the binary, then retry",
    )
}

fn gh_transport_err(method: &str, path: &str, e: &dyn std::fmt::Display) -> String {
    let base = format!("github api {method} {path}: transport: {e}");
    match tls_trust_hint(&e.to_string()) {
        Some(hint) => format!("{base} ({hint})"),
        None => base,
    }
}

// ---------------------------------------------------------------------------
// `fleet actions exec …` — the seat half
// ---------------------------------------------------------------------------

struct Flags {
    map: std::collections::HashMap<String, String>,
    switches: std::collections::HashSet<String>,
    positional: Vec<String>,
}

fn parse_flags(args: &[String], value_flags: &[&str], switches: &[&str]) -> Result<Flags, String> {
    let mut out = Flags {
        map: std::collections::HashMap::new(),
        switches: std::collections::HashSet::new(),
        positional: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            out.positional.extend_from_slice(&args[i + 1..]);
            break;
        } else if let Some(name) = a.strip_prefix("--") {
            if value_flags.contains(&name) {
                let Some(v) = args.get(i + 1) else {
                    return Err(format!("--{name} needs a value"));
                };
                out.map.insert(name.to_string(), v.clone());
                i += 2;
            } else if switches.contains(&name) {
                out.switches.insert(name.to_string());
                i += 1;
            } else {
                return Err(format!("unknown flag --{name}"));
            }
        } else {
            out.positional.push(a.clone());
            i += 1;
        }
    }
    Ok(out)
}

impl Flags {
    fn get(&self, name: &str) -> Option<&str> {
        self.map.get(name).map(|s| s.as_str())
    }
    fn has(&self, name: &str) -> bool {
        self.switches.contains(name)
    }
    fn seconds(&self, name: &str) -> Result<Option<i64>, String> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v
                .parse::<i64>()
                .map(Some)
                .map_err(|_| format!("--{name} must be whole seconds, got {v:?}")),
        }
    }
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("exec") => exec_run(config, &args[1..]),
        Some("agent") => agent_run(&args[1..]),
        Some("put") => put_run(&args[1..]),
        Some("--help") | Some("-h") | Some("help") | None => {
            println!("{ACTIONS_USAGE}");
            EXIT_OK
        }
        Some(other) => {
            eprintln!("fleet actions: unknown command {other:?}\n\n{ACTIONS_USAGE}");
            EXIT_USAGE
        }
    }
}

fn exec_run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("start") => exec_start(config, &args[1..]),
        Some("run") => exec_command(config, &args[1..]),
        Some("once") => exec_once(config, &args[1..]),
        Some("attach") => exec_attach(config, &args[1..], false),
        Some("watch") => exec_attach(config, &args[1..], true),
        Some("renew") => exec_renew(config, &args[1..]),
        Some("extend") => exec_extend(config, &args[1..]),
        Some("decide") => exec_decide(config, &args[1..]),
        Some("close") => exec_close(config, &args[1..]),
        Some("--help") | Some("-h") | None => {
            println!("{ACTIONS_USAGE}");
            EXIT_OK
        }
        Some(other) => {
            eprintln!("fleet actions exec: unknown command {other:?}\n\n{ACTIONS_USAGE}");
            EXIT_USAGE
        }
    }
}

fn fail_out(code: i32, msg: &str) -> i32 {
    eprintln!("fleet actions: {msg}");
    code
}

fn exec_start(config: &Config, args: &[String]) -> i32 {
    let flags = match parse_flags(
        args,
        &[
            "repo",
            "workflow",
            "ref",
            "target-repo",
            "target-ref",
            "timeout",
            "max-lifespan",
            "broker-url",
            "context",
        ],
        &["hide-logs", "detach"],
    ) {
        Ok(f) => f,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    // The lease is required by construction: no --timeout, no start.
    let lease_s = match flags.seconds("timeout") {
        Ok(Some(v)) => v,
        Ok(None) => {
            return fail_out(
                EXIT_USAGE,
                "exec start requires --timeout S (the session lease; there is no default)",
            );
        }
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let max_s = match flags.seconds("max-lifespan") {
        Ok(v) => v.unwrap_or(1800),
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let host_repo = flags.get("repo").unwrap_or(DEFAULT_HOST_REPO).to_string();
    let workflow = flags
        .get("workflow")
        .unwrap_or(DEFAULT_WORKFLOW)
        .to_string();
    let broker_url = flags
        .get("broker-url")
        .map(str::to_string)
        .or_else(|| std::env::var("FLEET_ACTIONS_EXEC_BROKER_URL").ok())
        .unwrap_or_else(|| DEFAULT_BROKER_URL.to_string());
    let seat = seat_name();

    // 1. Open the session row at the coordinator.
    let opened = match fleet_queue::client::actions_exec::ae_open(
        &host_repo,
        &workflow,
        &seat,
        lease_s,
        max_s,
        flags.has("hide-logs"),
        flags.get("context"),
    )
    .and_then(check_envelope)
    {
        Ok(v) => v,
        Err((c, m)) => return fail_out(c, &format!("open session: {m}")),
    };
    let handle = opened
        .get("handle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if handle.is_empty() {
        return fail_out(EXIT_ERROR, "broker opened a session without a handle");
    }

    // 2. The #13765 coordinator claim: attempted first; while the
    // verbs are not live the session proceeds on the singular path
    // and says so (placement_path), per the design.
    let coordinator =
        fleet_queue::client::actions_exec::ae_placement_claim(&host_repo, &seat, &handle);
    let placement_path = if coordinator {
        "coordinator (#13765 claim granted)".to_string()
    } else {
        "singular (coordinator claim unavailable — #13765 verbs not live; session proceeds on the broker row alone)"
            .to_string()
    };

    // 3. Dispatch the workflow with the handle + non-secret inputs.
    let dispatch_ref = match flags.get("ref") {
        Some(r) => r.to_string(),
        None => gh_api(config, "GET", &format!("/repos/{host_repo}"), None)
            .ok()
            .and_then(|v| {
                v.get("default_branch")
                    .and_then(|b| b.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "main".to_string()),
    };
    let dispatch_body = json!({
        "ref": dispatch_ref,
        "inputs": {
            "handle": handle,
            "broker_url": broker_url,
            "repo": flags.get("target-repo").unwrap_or(""),
            "ref": flags.get("target-ref").unwrap_or(""),
            "lease_seconds": lease_s.to_string(),
            "max_seconds": max_s.to_string(),
            "hide_logs": if flags.has("hide-logs") { "true" } else { "false" },
            "agent_ref": "",
        },
    });
    if let Err(e) = gh_api(
        config,
        "POST",
        &format!("/repos/{host_repo}/actions/workflows/{workflow}/dispatches"),
        Some(&dispatch_body),
    ) {
        let _ = fleet_queue::client::actions_exec::ae_close(&handle);
        return fail_out(EXIT_ERROR, &format!("workflow dispatch: {e}"));
    }

    // 4. Find the run (the workflow's run-name carries the handle)
    // and bind it — that bind is the handoff.
    let want_title = format!("Fleet exec session {handle}");
    let mut found: Option<(i64, i64)> = None;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if let Ok(runs) = gh_api(
            config,
            "GET",
            &format!(
                "/repos/{host_repo}/actions/workflows/{workflow}/runs?event=workflow_dispatch&per_page=30"
            ),
            None,
        ) {
            if let Some(list) = runs.get("workflow_runs").and_then(|v| v.as_array()) {
                for run in list {
                    if run.get("display_title").and_then(|v| v.as_str())
                        == Some(want_title.as_str())
                    {
                        if let (Some(id), Some(att)) = (
                            run.get("id").and_then(|v| v.as_i64()),
                            run.get("run_attempt").and_then(|v| v.as_i64()),
                        ) {
                            found = Some((id, att));
                            break;
                        }
                    }
                }
            }
        }
        if found.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    let Some((run_id, run_attempt)) = found else {
        return fail_out(
            EXIT_ERROR,
            "dispatched but the run did not appear within 120s; the session row stays open at the broker — check the workflow runs and bind by hand",
        );
    };
    match fleet_queue::client::actions_exec::ae_bind(&handle, run_id, run_attempt)
        .and_then(check_envelope)
    {
        Ok(_) => {}
        Err((c, m)) => return fail_out(c, &format!("bind run: {m}")),
    }

    let record = json!({
        "handle": handle,
        "host_repo": host_repo,
        "workflow": workflow,
        "run_id": run_id,
        "run_attempt": run_attempt,
        "lease_until": opened.get("lease_until").cloned().unwrap_or(Value::Null),
        "max_until": opened.get("max_until").cloned().unwrap_or(Value::Null),
        "placement_path": placement_path,
        "broker_url": broker_url,
        "created_at": now_unix(),
        "hide_logs": flags.has("hide-logs"),
    });
    store_put(record);

    println!("session {handle} open on {host_repo}");
    println!("handoff: {handle} · {host_repo}#{run_id} attempt {run_attempt}");
    println!("run: https://github.com/{host_repo}/actions/runs/{run_id}");
    println!("placement_path: {placement_path}");
    println!(
        "lease_until: {} · max_until: {}",
        opened
            .get("lease_until")
            .and_then(|v| v.as_str())
            .unwrap_or("?"),
        opened
            .get("max_until")
            .and_then(|v| v.as_str())
            .unwrap_or("?"),
    );
    println!("form: actions/fleet (fleet actions exec start)");
    if flags.has("detach") {
        println!(
            "detached. drive it with: fleet actions exec run {handle} -- <cmd> · watch: fleet actions exec watch {handle}"
        );
        return EXIT_OK;
    }
    println!(
        "attached — streaming session IO (commands go through `fleet actions exec run` from another shell; Ctrl-C detaches, the session lives on)"
    );
    stream_session(&handle, true, None)
}

/// List stored session records, most recently started first.
fn list_sessions() -> Vec<(String, Value)> {
    let store = store_load();
    let Some(obj) = store.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<(String, Value)> = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    out.sort_by_key(|(_, v)| v.get("created_at").and_then(|c| c.as_i64()).unwrap_or(0));
    out.reverse();
    out
}

/// The one-shot sugar (#13774): find a live session for the host
/// repo in the local store or start one with `exec start --detach`
/// (same lease discipline, same #13765 claim, same handoff), then
/// run one command through it and exit with its code. The session
/// stays open for the next `once` unless `--close`.
fn exec_once(config: &Config, args: &[String]) -> i32 {
    let flags = match parse_flags(
        args,
        &[
            "repo",
            "workflow",
            "ref",
            "target-repo",
            "target-ref",
            "lease",
            "max-lifespan",
            "timeout",
            "broker-url",
            "context",
        ],
        &["hide-logs", "close"],
    ) {
        Ok(f) => f,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let command_text = flags.positional.join(" ");
    if command_text.trim().is_empty() {
        return fail_out(
            EXIT_USAGE,
            "usage: fleet actions exec once [options] -- <command...>",
        );
    }
    let host_repo = flags.get("repo").unwrap_or(DEFAULT_HOST_REPO);
    let cmd_timeout_s = match flags.seconds("timeout") {
        Ok(v) => v.unwrap_or(600),
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };

    // Find: the most recent stored session for this host repo whose
    // row is still live at the broker.
    let mut handle: Option<String> = None;
    for (stored_handle, rec) in list_sessions() {
        if rec.get("host_repo").and_then(|v| v.as_str()) != Some(host_repo) {
            continue;
        }
        match fleet_queue::client::actions_exec::ae_stream(&stored_handle, 0, None)
            .and_then(check_envelope)
        {
            Ok(v) => {
                let state = v
                    .get("session")
                    .and_then(|s| s.get("state"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if matches!(state, "open" | "bound" | "live" | "extension-pending") {
                    handle = Some(stored_handle);
                    break;
                }
            }
            Err(_) => continue,
        }
    }

    // Create: same path as `exec start --detach` — the lease has a
    // one-shot default (1800 s) where start requires it by
    // construction.
    let handle = match handle {
        Some(h) => {
            println!("once: reusing live session {h} on {host_repo}");
            h
        }
        None => {
            let lease_s = match flags.seconds("lease") {
                Ok(v) => v.unwrap_or(1800),
                Err(e) => return fail_out(EXIT_USAGE, &e),
            };
            let before: std::collections::HashSet<String> =
                list_sessions().into_iter().map(|(h, _)| h).collect();
            let mut start_args: Vec<String> = vec![
                "--repo".to_string(),
                host_repo.to_string(),
                "--timeout".to_string(),
                lease_s.to_string(),
                "--detach".to_string(),
            ];
            for (flag, name) in [
                ("workflow", "workflow"),
                ("ref", "ref"),
                ("target-repo", "target-repo"),
                ("target-ref", "target-ref"),
                ("max-lifespan", "max-lifespan"),
                ("broker-url", "broker-url"),
                ("context", "context"),
            ] {
                if let Some(v) = flags.get(flag) {
                    start_args.push(format!("--{name}"));
                    start_args.push(v.to_string());
                }
            }
            if flags.has("hide-logs") {
                start_args.push("--hide-logs".to_string());
            }
            let code = exec_start(config, &start_args);
            if code != EXIT_OK {
                return code;
            }
            let new_handle = list_sessions()
                .into_iter()
                .map(|(h, _)| h)
                .find(|h| !before.contains(h));
            let Some(new_handle) = new_handle else {
                return fail_out(EXIT_ERROR, "start succeeded but recorded no session");
            };
            println!("once: started new session {new_handle} on {host_repo}");
            new_handle
        }
    };

    // Run: one command through the session; its exit code is ours.
    let cmd =
        match fleet_queue::client::actions_exec::ae_command(&handle, &command_text, cmd_timeout_s)
            .and_then(check_envelope)
        {
            Ok(v) => v,
            Err((c, m)) => return fail_out(c, &format!("enqueue command: {m}")),
        };
    let cmd_id = cmd
        .get("cmd_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if cmd_id.is_empty() {
        return fail_out(EXIT_ERROR, "broker accepted the command without an id");
    }
    let code = stream_session(&handle, false, Some(&cmd_id));
    if flags.has("close") {
        let _ = fleet_queue::client::actions_exec::ae_close(&handle);
        println!("once: closed session {handle}");
    }
    code
}

/// Stream a session's IO from the broker, printing new chunks as
/// they land. With `only_cmd`, restricts to one command and returns
/// its exit code when it finishes. Otherwise streams until the
/// session reaches a terminal state.
fn stream_session(handle: &str, _replay: bool, only_cmd: Option<&str>) -> i32 {
    let mut after_id: i64 = 0;
    let mut last_state = String::new();
    loop {
        let stream = match fleet_queue::client::actions_exec::ae_stream(handle, after_id, only_cmd)
            .and_then(check_envelope)
        {
            Ok(v) => v,
            Err((c, m)) => return fail_out(c, &format!("stream: {m}")),
        };
        if let Some(chunks) = stream.get("chunks").and_then(|v| v.as_array()) {
            for chunk in chunks {
                if let Some(id) = chunk.get("id").and_then(|v| v.as_i64()) {
                    after_id = after_id.max(id);
                }
                let data = chunk.get("data").and_then(|v| v.as_str()).unwrap_or("");
                match chunk.get("stream").and_then(|v| v.as_str()) {
                    Some("stderr") => {
                        eprint!("{data}");
                        let _ = std::io::stderr().flush();
                    }
                    _ => {
                        print!("{data}");
                        let _ = std::io::stdout().flush();
                    }
                }
            }
        }
        let state = stream
            .get("session")
            .and_then(|s| s.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if state != last_state {
            if !last_state.is_empty() || only_cmd.is_some() {
                eprintln!("[session {handle}: {state}]");
            }
            last_state = state.clone();
        }
        if let Some(cmd_id) = only_cmd {
            if let Some(cmds) = stream.get("commands").and_then(|v| v.as_array()) {
                for cmd in cmds {
                    if cmd.get("cmd_id").and_then(|v| v.as_str()) == Some(cmd_id) {
                        match cmd.get("state").and_then(|v| v.as_str()) {
                            Some("done") => {
                                return cmd.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0)
                                    as i32;
                            }
                            Some("error") => {
                                return fail_out(
                                    EXIT_ERROR,
                                    "the command ended in error (session closed or expired)",
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if matches!(state.as_str(), "closed" | "expired") {
            if only_cmd.is_some() {
                return fail_out(EXIT_ERROR, "the session ended before the command finished");
            }
            return EXIT_OK;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn exec_command(_config: &Config, args: &[String]) -> i32 {
    // Split: flags before the reference; the command is everything
    // after `--`, or the positional words after the reference.
    let flags = match parse_flags(args, &["timeout"], &[]) {
        Ok(f) => f,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let Some(reference) = flags.positional.first() else {
        return fail_out(
            EXIT_USAGE,
            "usage: fleet actions exec run <ref> [--timeout S] -- <command...>",
        );
    };
    let command_text = flags.positional[1..].join(" ");
    if command_text.trim().is_empty() {
        return fail_out(EXIT_USAGE, "exec run needs a command (after --)");
    }
    let timeout_s = match flags.seconds("timeout") {
        Ok(v) => v.unwrap_or(600),
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let handle = match resolve_ref(reference) {
        Ok(h) => h,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let cmd = match fleet_queue::client::actions_exec::ae_command(&handle, &command_text, timeout_s)
        .and_then(check_envelope)
    {
        Ok(v) => v,
        Err((c, m)) => return fail_out(c, &format!("enqueue command: {m}")),
    };
    let cmd_id = cmd
        .get("cmd_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if cmd_id.is_empty() {
        return fail_out(EXIT_ERROR, "broker accepted the command without an id");
    }
    stream_session(&handle, false, Some(&cmd_id))
}

fn exec_attach(_config: &Config, args: &[String], watch: bool) -> i32 {
    let flags = match parse_flags(args, &[], &[]) {
        Ok(f) => f,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let Some(reference) = flags.positional.first() else {
        return fail_out(EXIT_USAGE, "usage: fleet actions exec attach|watch <ref>");
    };
    let handle = match resolve_ref(reference) {
        Ok(h) => h,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    if watch {
        // One status snapshot up front (run + session), then the
        // stream itself reports state changes as they happen.
        if let Ok(stream) =
            fleet_queue::client::actions_exec::ae_stream(&handle, 0, None).and_then(check_envelope)
        {
            let s = &stream["session"];
            println!(
                "watching {handle}: state={} run={} attempt={} lease_until={} max_until={}",
                s.get("state").and_then(|v| v.as_str()).unwrap_or("?"),
                s.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0),
                s.get("run_attempt").and_then(|v| v.as_i64()).unwrap_or(0),
                s.get("lease_until").and_then(|v| v.as_str()).unwrap_or("?"),
                s.get("max_until").and_then(|v| v.as_str()).unwrap_or("?"),
            );
        }
    }
    stream_session(&handle, true, None)
}

fn ref_arg(
    args: &[String],
    value_flags: &[&str],
    switches: &[&str],
) -> Result<(Flags, String), i32> {
    let flags = match parse_flags(args, value_flags, switches) {
        Ok(f) => f,
        Err(e) => return Err(fail_out(EXIT_USAGE, &e)),
    };
    let Some(reference) = flags.positional.first().cloned() else {
        return Err(fail_out(EXIT_USAGE, "a session reference is required"));
    };
    match resolve_ref(&reference) {
        Ok(h) => Ok((flags, h)),
        Err(e) => Err(fail_out(EXIT_USAGE, &e)),
    }
}

fn exec_renew(_config: &Config, args: &[String]) -> i32 {
    let (flags, handle) = match ref_arg(args, &["timeout"], &[]) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let lease_s = match flags.seconds("timeout") {
        Ok(Some(v)) => v,
        Ok(None) => {
            return fail_out(
                EXIT_USAGE,
                "renew requires --timeout S (the renewal names its lease)",
            );
        }
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    match fleet_queue::client::actions_exec::ae_renew(&handle, lease_s).and_then(check_envelope) {
        Ok(v) => {
            println!(
                "renewed {handle}: lease_until={} max_until={} extension_required={}",
                v.get("lease_until").and_then(|x| x.as_str()).unwrap_or("?"),
                v.get("max_until").and_then(|x| x.as_str()).unwrap_or("?"),
                v.get("extension_required")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false),
            );
            if v.get("extension_required").and_then(|x| x.as_bool()) == Some(true) {
                println!(
                    "the ask passed the max lifespan and was clamped; past the max only an approved `fleet actions exec extend` helps"
                );
            }
            EXIT_OK
        }
        Err((c, m)) => fail_out(c, &format!("renew: {m}")),
    }
}

fn exec_extend(_config: &Config, args: &[String]) -> i32 {
    let (flags, handle) = match ref_arg(args, &["minutes", "reason"], &[]) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let minutes: i64 = match flags.get("minutes") {
        None => 30,
        Some(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => return fail_out(EXIT_USAGE, "--minutes must be a whole number"),
        },
    };
    let reason = flags.get("reason").unwrap_or("").to_string();
    match fleet_queue::client::actions_exec::ae_extend_request(&handle, minutes * 60, &reason)
        .and_then(check_envelope)
    {
        Ok(v) => {
            if v.get("requested").and_then(|x| x.as_bool()) == Some(true) {
                let tier = v.get("tier").and_then(|x| x.as_i64()).unwrap_or(3);
                let key = v
                    .get("alert_key")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                match tier {
                    1 => println!(
                        "extension auto-approved for {handle} (tier 1): +{}s; max_until={} lease_until={}; Drew has been notified (alert key {key})",
                        minutes * 60,
                        v.get("max_until").and_then(|x| x.as_str()).unwrap_or("?"),
                        v.get("lease_until").and_then(|x| x.as_str()).unwrap_or("?"),
                    ),
                    2 => {
                        println!(
                            "extension pending for {handle} (tier 2): +{}s; it approves itself at {} unless Drew denies (alert key {key})",
                            minutes * 60,
                            v.get("decide_after")
                                .and_then(|x| x.as_str())
                                .unwrap_or("?"),
                        );
                        println!(
                            "early answer: fleet actions exec decide {handle} --approve|--deny"
                        );
                    }
                    _ => {
                        println!(
                            "extension requested for {handle} (tier 3): +{}s past the max; the needs-drew alert is raised (source fleet-actions-exec, key {key}); decision grace runs to {}",
                            minutes * 60,
                            v.get("grace_until").and_then(|x| x.as_str()).unwrap_or("?"),
                        );
                        println!(
                            "Drew's answer returns via: fleet actions exec decide {handle} --approve|--deny"
                        );
                    }
                }
                EXIT_OK
            } else {
                fail_out(EXIT_ERROR, "the broker did not record the request")
            }
        }
        Err((c, m)) => fail_out(c, &format!("extend: {m}")),
    }
}

fn exec_decide(_config: &Config, args: &[String]) -> i32 {
    let (flags, handle) = match ref_arg(args, &[], &["approve", "deny"]) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let approve = match (flags.has("approve"), flags.has("deny")) {
        (true, false) => true,
        (false, true) => false,
        _ => return fail_out(EXIT_USAGE, "decide needs exactly one of --approve / --deny"),
    };
    match fleet_queue::client::actions_exec::ae_extend_decide(&handle, approve)
        .and_then(check_envelope)
    {
        Ok(v) => {
            if v.get("outcome").and_then(|x| x.as_str()) == Some("silence-approved") {
                println!(
                    "extension for {handle} was already auto-approved at the tier-2 deny-window end; this decision arrived too late to change it"
                );
                return EXIT_OK;
            }
            if approve {
                println!(
                    "extension approved for {handle}: max_until={} lease_until={}",
                    v.get("max_until").and_then(|x| x.as_str()).unwrap_or("?"),
                    v.get("lease_until").and_then(|x| x.as_str()).unwrap_or("?"),
                );
            } else {
                println!("extension denied for {handle}; the session expires on its lease");
            }
            EXIT_OK
        }
        Err((c, m)) => fail_out(c, &format!("decide: {m}")),
    }
}

fn exec_close(_config: &Config, args: &[String]) -> i32 {
    let (_flags, handle) = match ref_arg(args, &[], &[]) {
        Ok(v) => v,
        Err(c) => return c,
    };
    match fleet_queue::client::actions_exec::ae_close(&handle).and_then(check_envelope) {
        Ok(v) => {
            println!(
                "closed {handle}: {}",
                if v.get("closed").and_then(|x| x.as_bool()) == Some(true) {
                    "session closed"
                } else {
                    "already closed"
                }
            );
            EXIT_OK
        }
        Err((c, m)) => fail_out(c, &format!("close: {m}")),
    }
}

// ---------------------------------------------------------------------------
// `fleet actions agent` — the runner half
// ---------------------------------------------------------------------------

/// Parse the broker's ISO-8601 UTC stamps (inverse of its civil
/// conversion; same algorithm as the broker's own).
fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |i: usize, n: usize| -> Option<i64> {
        let mut v: i64 = 0;
        for k in 0..n {
            let c = b[i + k];
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    // Days from civil (Howard Hinnant's algorithm).
    let y_adj = if mo <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec)
}

/// Fetch a fresh OIDC token from the Actions runtime. The token is
/// a credential: it is only ever placed in the X-Agent-Token
/// header of broker calls, never printed.
fn fetch_oidc_token() -> Result<String, String> {
    let base = std::env::var("ACTIONS_ID_TOKEN_REQUEST_URL")
        .map_err(|_| "ACTIONS_ID_TOKEN_REQUEST_URL is not set (not running in Actions?)")?;
    let bearer = std::env::var("ACTIONS_ID_TOKEN_REQUEST_TOKEN")
        .map_err(|_| "ACTIONS_ID_TOKEN_REQUEST_TOKEN is not set")?;
    let sep = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{sep}audience=fleet-actions-exec");
    let mut resp = ureq::get(&url)
        .header("Authorization", &format!("Bearer {bearer}"))
        .header("User-Agent", "fleet-actions-agent")
        .call()
        .map_err(|e| format!("oidc token fetch: transport: {e}"))?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("oidc token fetch: unreadable body: {e}"))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("oidc token fetch: bad JSON: {e}"))?;
    v.get("value")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "oidc token fetch: response carried no token".to_string())
}

/// One agent→broker call through the public door. Errors name the
/// path and status only — never headers, never the token.
fn agent_call(broker_url: &str, path: &str, jwt: &str, body: &Value) -> Result<Value, String> {
    let url = format!("{}{path}", broker_url.trim_end_matches('/'));
    let mut resp = ureq::post(&url)
        .header("X-Agent-Token", jwt)
        .header("User-Agent", "fleet-actions-agent")
        .send(serde_json::to_string(body).unwrap_or_default().as_str())
        .map_err(|e| format!("{path}: transport: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{path}: unreadable body: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("{path}: HTTP {status}"));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: bad JSON: {e}"))?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        return Err(format!(
            "{path}: {}",
            v.get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("error envelope")
        ));
    }
    Ok(v)
}

struct AgentCtx {
    broker_url: String,
    handle: String,
    workdir: String,
    outbox: String,
    hide_logs: bool,
}

fn agent_run(args: &[String]) -> i32 {
    let flags = match parse_flags(
        args,
        &[
            "handle",
            "broker-url",
            "lease-seconds",
            "max-seconds",
            "workdir",
            "outbox",
        ],
        &["hide-logs"],
    ) {
        Ok(f) => f,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let (Some(handle), Some(broker_url), Some(workdir), Some(outbox)) = (
        flags.get("handle"),
        flags.get("broker-url"),
        flags.get("workdir"),
        flags.get("outbox"),
    ) else {
        return fail_out(
            EXIT_USAGE,
            "agent requires --handle, --broker-url, --workdir and --outbox",
        );
    };
    let max_s: i64 = flags
        .get("max-seconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1800);
    let started = now_unix();
    let local_max_deadline = started + max_s;
    let ctx = AgentCtx {
        broker_url: broker_url.to_string(),
        handle: handle.to_string(),
        workdir: workdir.to_string(),
        outbox: outbox.to_string(),
        hide_logs: flags.has("hide-logs"),
    };
    let _ = std::fs::create_dir_all(&ctx.outbox);
    install_put_helper(&ctx);

    // Register with a fresh OIDC token; the verified claims bind
    // this run to the session row.
    let jwt = match fetch_oidc_token() {
        Ok(t) => t,
        Err(e) => return fail_out(EXIT_ERROR, &e),
    };
    let registered = match agent_call(
        &ctx.broker_url,
        "/actions-exec-register",
        &jwt,
        &json!({"handle": ctx.handle}),
    ) {
        Ok(v) => v,
        Err(e) => return fail_out(EXIT_ERROR, &format!("register: {e}")),
    };
    let lease_deadline = registered
        .get("lease_until")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or(local_max_deadline);
    let mut deadline = lease_deadline.min(local_max_deadline);
    eprintln!(
        "fleet actions agent: session {} live; lease deadline {deadline}",
        ctx.handle
    );

    loop {
        if now_unix() >= deadline {
            eprintln!("fleet actions agent: lease expired; exiting");
            return EXIT_OK;
        }
        let jwt = match fetch_oidc_token() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("fleet actions agent: {e} (retrying)");
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        let poll = match agent_call(
            &ctx.broker_url,
            "/actions-exec-poll",
            &jwt,
            &json!({"handle": ctx.handle, "wait_s": 20}),
        ) {
            Ok(v) => v,
            Err(e) => {
                // Partition tolerance: the broker holds the
                // session; keep retrying until the LOCAL deadline
                // (the lease is enforced here too).
                eprintln!("fleet actions agent: poll: {e} (retrying until the local deadline)");
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        if let Some(d) = poll
            .get("session")
            .and_then(|s| s.get("deadline_unix"))
            .and_then(|v| v.as_i64())
        {
            deadline = d.min(local_max_deadline);
        }
        let state = poll
            .get("session")
            .and_then(|s| s.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if matches!(state.as_str(), "closed" | "expired") {
            eprintln!("fleet actions agent: session {state}; exiting");
            // Best-effort close so the seat sees a terminal row
            // even when the lease, not the seat, ended it.
            if state == "expired" {
                if let Ok(jwt) = fetch_oidc_token() {
                    let _ = agent_call(
                        &ctx.broker_url,
                        "/actions-exec-close",
                        &jwt,
                        &json!({"handle": ctx.handle}),
                    );
                }
            }
            return EXIT_OK;
        }
        if let Some(cmds) = poll.get("commands").and_then(|v| v.as_array()) {
            for cmd in cmds {
                run_one_command(&ctx, cmd);
            }
        }
    }
}

/// Write the fleet-exec-put helper into a session bin dir beside
/// the outbox and prepend it to this process's PATH (inherited by
/// every command shell the agent spawns).
fn install_put_helper(ctx: &AgentCtx) {
    let parent = std::path::Path::new(&ctx.outbox)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let bin_dir = parent.join("fleet-exec-bin");
    if std::fs::create_dir_all(&bin_dir).is_err() {
        return;
    }
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "fleet".to_string());
    let script = format!(
        "#!/bin/sh\n# fleet-exec-put: session egress helper (#13774).\nexec \"{exe}\" actions put \"$@\"\n"
    );
    let path = bin_dir.join("fleet-exec-put");
    if std::fs::write(&path, script).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        let old = std::env::var("PATH").unwrap_or_default();
        unsafe {
            std::env::set_var("PATH", format!("{}:{old}", bin_dir.to_string_lossy()));
        }
    }
}

/// Execute one queued command: stream its IO to the broker (and to
/// this process's own stdout unless the session hides logs), then
/// record the exit code.
fn run_one_command(ctx: &AgentCtx, cmd: &Value) {
    let cmd_id = cmd.get("cmd_id").and_then(|v| v.as_str()).unwrap_or("");
    let text = cmd.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let timeout_s = cmd.get("timeout_s").and_then(|v| v.as_i64()).unwrap_or(600);
    if cmd_id.is_empty() || text.is_empty() {
        return;
    }
    eprintln!("fleet actions agent: running {cmd_id}: {text}");
    let mut child = match Command::new("bash")
        .args(["-c", text])
        .current_dir(&ctx.workdir)
        .env("FLEET_EXEC_OUTBOX", &ctx.outbox)
        .env("FLEET_EXEC_HANDLE", &ctx.handle)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            post_result(ctx, cmd_id, 127);
            eprintln!("fleet actions agent: spawn failed for {cmd_id}: {e}");
            return;
        }
    };
    let (tx, rx) = mpsc::channel::<(&'static str, String)>();
    fn spawn_reader<R: Read + Send + 'static>(
        name: &'static str,
        stream: R,
        tx: mpsc::Sender<(&'static str, String)>,
    ) {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send((name, line.clone())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    if let Some(out) = child.stdout.take() {
        spawn_reader("stdout", out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_reader("stderr", err, tx.clone());
    }
    drop(tx);
    let mut seq = 0i64;
    let mut bufs: std::collections::HashMap<&'static str, String> =
        std::collections::HashMap::new();
    let started = Instant::now();
    let mut exit_code: Option<i32> = None;
    let mut timed_out = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((stream, data)) => {
                if !ctx.hide_logs {
                    match stream {
                        "stderr" => {
                            eprint!("{data}");
                            let _ = std::io::stderr().flush();
                        }
                        _ => {
                            print!("{data}");
                            let _ = std::io::stdout().flush();
                        }
                    }
                }
                let buf = bufs.entry(stream).or_default();
                buf.push_str(&data);
                if buf.len() >= 16384 {
                    seq += 1;
                    let chunk = std::mem::take(buf);
                    post_io(ctx, cmd_id, seq, stream, &chunk);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Both pipes closed; reap below.
                if let Ok(status) = child.wait() {
                    exit_code = status.code();
                    break;
                }
            }
        }
        if started.elapsed() > Duration::from_secs(timeout_s.max(1) as u64) {
            let _ = child.kill();
            timed_out = true;
            let _ = child.wait();
            exit_code = Some(124);
            break;
        }
        if exit_code.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                exit_code = status.code();
                // Drain whatever the reader threads still hold.
                while let Ok((stream, data)) = rx.try_recv() {
                    if !ctx.hide_logs {
                        match stream {
                            "stderr" => eprint!("{data}"),
                            _ => print!("{data}"),
                        }
                    }
                    bufs.entry(stream).or_default().push_str(&data);
                }
                break;
            }
        }
    }
    for (stream, buf) in &bufs {
        if !buf.is_empty() {
            seq += 1;
            post_io(ctx, cmd_id, seq, stream, buf);
        }
    }
    if timed_out {
        seq += 1;
        post_io(
            ctx,
            cmd_id,
            seq,
            "stderr",
            &format!("fleet actions agent: command timed out after {timeout_s}s\n"),
        );
    }
    post_result(ctx, cmd_id, exit_code.unwrap_or(1));
}

fn post_io(ctx: &AgentCtx, cmd_id: &str, seq: i64, stream: &str, data: &str) {
    // IO chunks are capped at 64 KiB by the broker; flush in
    // slices cut on char boundaries.
    let mut rest = data;
    while !rest.is_empty() {
        let mut cut = rest.len().min(32768);
        while cut > 0 && !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == 0 {
            break;
        }
        let (text, tail) = rest.split_at(cut);
        rest = tail;
        match fetch_oidc_token() {
            Ok(jwt) => {
                let _ = agent_call(
                    &ctx.broker_url,
                    "/actions-exec-io",
                    &jwt,
                    &json!({
                        "handle": ctx.handle, "cmd_id": cmd_id, "seq": seq,
                        "stream": stream, "data": text,
                    }),
                );
            }
            Err(e) => eprintln!("fleet actions agent: io post skipped: {e}"),
        }
    }
}

fn post_result(ctx: &AgentCtx, cmd_id: &str, exit_code: i32) {
    if let Ok(jwt) = fetch_oidc_token() {
        let _ = agent_call(
            &ctx.broker_url,
            "/actions-exec-result",
            &jwt,
            &json!({"handle": ctx.handle, "cmd_id": cmd_id, "exit_code": exit_code}),
        );
    }
}

// ---------------------------------------------------------------------------
// `fleet actions put` — the egress helper
// ---------------------------------------------------------------------------

fn put_run(args: &[String]) -> i32 {
    let flags = match parse_flags(args, &[], &[]) {
        Ok(f) => f,
        Err(e) => return fail_out(EXIT_USAGE, &e),
    };
    let Some(file) = flags.positional.first() else {
        return fail_out(EXIT_USAGE, "usage: fleet actions put <file>");
    };
    let outbox = match std::env::var("FLEET_EXEC_OUTBOX") {
        Ok(d) if !d.is_empty() => d,
        _ => {
            return fail_out(
                EXIT_USAGE,
                "FLEET_EXEC_OUTBOX is not set — put runs inside a session (as fleet-exec-put)",
            );
        }
    };
    let src = std::path::Path::new(file);
    let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return fail_out(EXIT_USAGE, "the file has no name");
    };
    let bytes = match std::fs::read(src) {
        Ok(b) => b,
        Err(e) => return fail_out(EXIT_ERROR, &format!("read {file}: {e}")),
    };
    if std::fs::create_dir_all(&outbox).is_err() {
        return fail_out(EXIT_ERROR, "cannot create the outbox directory");
    }
    let dst = std::path::Path::new(&outbox).join(&name);
    if let Err(e) = std::fs::write(&dst, &bytes) {
        return fail_out(EXIT_ERROR, &format!("write outbox copy: {e}"));
    }
    println!(
        "put: {name} ({} bytes) -> session outbox (workflow artifact)",
        bytes.len()
    );
    // The R2 leg: only when the session carries credentials, and
    // quietly skipped otherwise. Values come from the environment
    // the workflow populated from masked secrets; never printed.
    let (Ok(key_id), Ok(secret), Ok(endpoint), Ok(bucket)) = (
        std::env::var("FLEET_EXEC_R2_ACCESS_KEY_ID"),
        std::env::var("FLEET_EXEC_R2_SECRET_ACCESS_KEY"),
        std::env::var("FLEET_EXEC_R2_ENDPOINT"),
        std::env::var("FLEET_EXEC_R2_BUCKET"),
    ) else {
        return EXIT_OK;
    };
    if key_id.is_empty() || secret.is_empty() || endpoint.is_empty() || bucket.is_empty() {
        return EXIT_OK;
    }
    let handle = std::env::var("FLEET_EXEC_HANDLE").unwrap_or_else(|_| "session".to_string());
    let key = format!("fleet-exec/{handle}/{name}");
    match r2_put(&endpoint, &bucket, &key, &key_id, &secret, &bytes) {
        Ok(()) => {
            println!("put: {name} -> R2 {bucket}/{key}");
            EXIT_OK
        }
        Err(e) => fail_out(EXIT_ERROR, &format!("R2 put: {e}")),
    }
}

// --- SigV4 (S3-compatible, R2) ----------------------------------------------

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(data);
    h.finalize().into()
}

fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut block = if key.len() > 64 {
        sha256(key).to_vec()
    } else {
        key.to_vec()
    };
    block.resize(64, 0);
    let ipad: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    let mut inner = ipad;
    inner.extend_from_slice(data);
    let inner_hash = sha256(&inner);
    let mut outer = opad;
    outer.extend_from_slice(&inner_hash);
    sha256(&outer).to_vec()
}

fn amz_dates(unix: i64) -> (String, String) {
    // Civil from days (Hinnant), the inverse of parse_iso.
    let days = unix.div_euclid(86400);
    let secs = unix.rem_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    (
        format!("{year:04}{m:02}{d:02}"),
        format!("{year:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z"),
    )
}

fn r2_put(
    endpoint: &str,
    bucket: &str,
    key: &str,
    access_key: &str,
    secret_key: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let endpoint = endpoint.trim_end_matches('/');
    let host = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    if host.is_empty() {
        return Err("the R2 endpoint has no host".to_string());
    }
    let path = format!("/{bucket}/{key}");
    let url = format!("{endpoint}{path}");
    let payload_hash = sha256_hex(bytes);
    let (datestamp, amzdate) = amz_dates(now_unix());
    let region = "auto";
    let service = "s3";
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amzdate}\n");
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_request =
        format!("PUT\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
    let scope = format!("{datestamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amzdate}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), datestamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hmac_sha256(&k_signing, string_to_sign.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    let mut resp = ureq::put(&url)
        .header("Authorization", &authorization)
        .header("x-amz-date", &amzdate)
        .header("x-amz-content-sha256", &payload_hash)
        .header("User-Agent", "fleet-actions-put")
        .send(bytes)
        .map_err(|e| format!("transport: {e}"))?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        let _ = resp.body_mut().read_to_string();
        Err(format!("HTTP {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle: golden corpus — the exact seat failure from code#15575
    /// must name the trust problem and the remedy; non-TLS failures
    /// must pass through unchanged.
    #[test]
    fn tls_failures_name_the_trust_problem() {
        for err in [
            "io: invalid peer certificate: UnknownIssuer",
            "transport: invalid peer certificate: UnknownIssuer",
            "x509: certificate signed by unknown authority",
            "TLS handshake failed: certificate verify failed (self signed certificate)",
        ] {
            let hint = tls_trust_hint(err);
            assert!(hint.is_some(), "no hint for TLS failure: {err}");
            let hint = hint.unwrap();
            assert!(
                hint.contains("15575") && hint.contains("bootstrap"),
                "hint names neither the issue nor the remedy: {hint}"
            );
        }
    }

    #[test]
    fn non_tls_failures_stay_terse() {
        for err in [
            "connection refused",
            "HTTP 403: Resource not accessible by integration",
            "dns error: no records found",
            "timed out",
        ] {
            assert_eq!(tls_trust_hint(err), None, "false hint for: {err}");
        }
    }

    #[test]
    fn dispatch_error_carries_hint_only_for_tls() {
        let tls = gh_transport_err(
            "POST",
            "/repos/o/r/actions/workflows/fleet-exec.yml/dispatches",
            &"io: invalid peer certificate: UnknownIssuer",
        );
        assert!(tls.contains("transport: "), "{tls}");
        assert!(tls.contains("15575"), "{tls}");
        let plain = gh_transport_err(
            "POST",
            "/repos/o/r/actions/workflows/fleet-exec.yml/dispatches",
            &"connection refused",
        );
        assert_eq!(
            plain,
            "github api POST /repos/o/r/actions/workflows/fleet-exec.yml/dispatches: transport: connection refused"
        );
    }
}
