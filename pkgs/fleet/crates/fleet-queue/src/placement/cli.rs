use super::*;

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
const EMBEDDED_REGISTRY: &str = include_str!("../../../../fleet-surfaces.json");

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
[--allow-paid-blacksmith MINUTES] \
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
    };
    let registry = registry_value(&cli)?;
    let gh = RealGh::new();
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
            load_store()
                .get("sessions")
                .and_then(Value::as_object)
                .and_then(|sessions| sessions.get(handle))
                .cloned()
                .ok_or_else(|| Die(format!(
                    "--followup {handle}: no session with this handle in the local Fleet placement store"
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
    let (adapter, reasoning) = decide(&registry, &job, &ctx, &gh)?;
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
    if !dispatch {
        out.insert(
            "mode".into(),
            json!("dry-run (no session written; pass --dispatch to submit)"),
        );
        print_out(&Value::Object(out), cli.json, render_decision);
        return Ok(0);
    }
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
            commit_new_session(&rec, None)?;
            out.insert("handle".into(), json!(handle));
            out.insert("state".into(), json!("queued"));
        }
        Some(adapter) => {
            let result = adapter.dispatch(&job, &gh)?;
            let surface = adapter.surface().clone();
            let sid = surf_str(&surface, "id").to_string();
            let kind = surf_str(&surface, "kind").to_string();
            let runner_repo = surf_str(&surface, "repo").to_string();
            let mut rec = Map::new();
            rec.insert("handle".into(), json!(handle));
            rec.insert("job".into(), job.to_json());
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
            commit_new_session(&Value::Object(rec), blacksmith_entry.as_ref())?;
            out.insert("handle".into(), json!(handle));
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
    }
    print_out(&Value::Object(out), cli.json, render_decision);
    Ok(0)
}

const SESSION_USAGE: &str = "usage: repo-queue session <status|result|list> [HANDLE|OWNER/REPO#RUN_ID] \
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
            let registry = registry_value(&cli)?;
            let rec = if parse_portable_run_reference(&handle).is_some() {
                resolve_portable_run_reference(&handle, &registry, &gh)?
            } else {
                with_store_lock(|| {
                    let mut store = load_store();
                    let rec = refresh_session(&mut store, &handle, &registry, &gh)?;
                    save_store(&store)?;
                    Ok(rec)
                })?
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

fn run_surfaces(args: &[String]) -> R<i32> {
    let (cli, _rest) = extract_common(args)?;
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
        }
    }

    fn ctx(load: &[(&str, (i64, i64))], blacksmith_used: f64, laptop_busy: bool) -> Ctx {
        Ctx {
            owners: registry()["owners"].clone(),
            owner_load: load.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
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
        )
        .unwrap();
        let back = load_store();
        assert_eq!(back["sessions"]["fp_other"]["state"], json!("running"));
        assert_eq!(back["sessions"]["fp_mine"]["state"], json!("queued"));
        assert_eq!(back["blacksmith_dispatches"].as_array().unwrap().len(), 1);
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
}
