// Adapter implementations for placement surfaces.

use super::*;

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
/// in the run display title via `run-name:` — issue #8371). The job's
/// placement session handle when it carries one (#13178), else a
/// fresh fp_ handle. Surfaces without the flag get None:
/// GitHub rejects dispatch inputs the workflow does not declare,
/// so those keep the snapshot-exclusion fallback in correlate_run.
pub(super) fn dispatch_id_for(surface: &Value, job: &Job) -> Option<String> {
    if surface.get("dispatch_id_input").and_then(|v| v.as_bool()) == Some(true) {
        Some(
            job.session_handle
                .clone()
                .unwrap_or_else(|| new_handle(&rand_source::URandom)),
        )
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
pub(super) fn correlate_run(
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
    pub(super) surface: Value,
    /// The runner this adapter dispatches: "github" for the plain
    /// adapter, "blacksmith" for the Blacksmith variant (the
    /// reference models Blacksmith as a subclass overriding
    /// runner_name).
    pub(super) force_runner: &'static str,
}

impl GithubActionsAdapter {
    /// The fleet-run workflow inputs for a job. Cache options travel
    /// in a `jobs` entry (the workflows that accept bare repo/command
    /// inputs ignore entry fields they do not know); the `runner`
    /// input is sent only when it changes something (blacksmith) —
    /// upstream's fleet-run rejects unknown inputs.
    pub(super) fn workflow_inputs(&self, job: &Job, dispatch_id: Option<&str>) -> Value {
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
            if let Some(minutes) = job.timeout_minutes {
                inputs.insert("timeout_minutes".into(), json!(minutes.to_string()));
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
        let dispatch_id = dispatch_id_for(s, job);
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
        // A numeric free-slot estimate requires complete load
        // knowledge: measured reads, an explicitly labeled
        // simulation, or a known subtotal that already proves
        // saturation. Anything else reports unknown capacity
        // instead of dressing an unreadable owner up as idle
        // (issue #13611).
        let coverage = ctx.owner_load_coverage.get(owner);
        let free_slots = coverage.and_then(|c| c.estimated_free(cap));
        let summary = match coverage {
            Some(c) => match c.status {
                LoadCoverageStatus::Measured => format!(
                    "owner {owner}: {active} active job(s) on fleet runner hosts \
                     (proxy for account load) vs cap {cap} -> {} free slot(s)",
                    free_slots.unwrap_or(0)
                ),
                LoadCoverageStatus::Simulated => format!(
                    "owner {owner}: {active} active job(s) (simulated via \
                     --assume-owner-load, not live counts) vs cap {cap} -> {} free slot(s)",
                    free_slots.unwrap_or(0)
                ),
                LoadCoverageStatus::Partial => {
                    if free_slots == Some(0) {
                        format!(
                            "owner {owner}: load only partially observed, but the known \
                             subtotal of {active} active job(s) already reaches cap {cap} \
                             — saturated on the evidence available (missing reads: {})",
                            c.missing_reads.join("; ")
                        )
                    } else {
                        format!(
                            "owner {owner}: load partially observed — at least {active} \
                             active job(s) known vs cap {cap}; free slots unknown \
                             (missing reads: {})",
                            c.missing_reads.join("; ")
                        )
                    }
                }
                LoadCoverageStatus::Unknown => format!(
                    "owner {owner}: load unknown — no runner-host read succeeded \
                     (missing reads: {}); free slots unknown, not zero load",
                    c.missing_reads.join("; ")
                ),
            },
            None => format!(
                "owner {owner}: load unknown — no coverage recorded for this owner; \
                 free slots unknown, not zero load"
            ),
        };
        Capacity {
            summary,
            free_slots,
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

pub(super) fn adapter_for(surface: &Value) -> Option<Box<dyn Adapter>> {
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
