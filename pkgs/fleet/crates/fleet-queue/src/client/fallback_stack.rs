//! GitHub fallback creation path (GhRunner/GhCreator), the outbox drain, and the stack verbs including replay (split from client.rs, #8588).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::*;

/// How one `gh` invocation is run. Production runs the real `gh`
/// binary; tests substitute a stub binary through this seam so the
/// exact argv and the failure shapes are observable (#8361 — the
/// creation-outbox production leg previously had no seam at all and
/// its marker search was never exercised by a test).
#[derive(Debug, Clone)]
pub(super) struct GhRunner {
    pub(super) bin: String,
}

impl GhRunner {
    fn system() -> Self {
        let bin = std::env::var("FLEET_QUEUE_GH_BIN")
            .ok()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| "gh".to_string());
        GhRunner { bin }
    }

    fn with_bin(bin: impl Into<String>) -> Self {
        GhRunner { bin: bin.into() }
    }

    /// Run one `gh` command and capture (success, stdout, stderr)
    /// without a shell — arguments never pass through shell expansion.
    fn run(&self, argv: &[String]) -> (bool, String, String) {
        match std::process::Command::new(&self.bin).args(argv).output() {
            Ok(out) => (
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ),
            Err(e) => (false, String::new(), format!("cannot run gh: {e}")),
        }
    }

    /// Run one `gh` command expecting JSON on stdout. A failure is
    /// an `Err` carrying the error class derived from stderr —
    /// never a silent `None` — so callers that must distinguish
    /// "checked and absent" from "could not check" can (#8361).
    fn json(&self, argv: &[String]) -> Result<Value, String> {
        let (ok, stdout, stderr) = self.run(argv);
        if !ok {
            return Err(gh_error_class(&stderr));
        }
        serde_json::from_str(&stdout).map_err(|_| "unparseable_json".to_string())
    }
}

/// Classify a failed `gh` invocation's stderr into the error-class
/// vocabulary recorded in an outbox entry's `last_error_class`.
pub(super) fn gh_error_class(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("rate limit")
        || lower.contains("submitted too quickly")
        || lower.contains("429")
    {
        "rate_limit".to_string()
    } else if lower.contains("404") {
        "http_404".to_string()
    } else if lower.contains("403") {
        "http_403".to_string()
    } else if lower.contains("cannot run gh") {
        "spawn_failed".to_string()
    } else if stderr.trim().is_empty() {
        "gh_failed_no_stderr".to_string()
    } else {
        "gh_failed".to_string()
    }
}

/// The three-way result of a marker lookup (#8361). The old
/// `Option` collapsed "the search ran and found nothing" into the
/// same `None` as "the search itself failed", and the drain loop
/// re-created on both — duplicating objects whose create had
/// actually landed.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum MarkerLookup {
    Found(i64, String),
    /// The search completed and no listed object carries the marker:
    /// absence is proven, re-creating is safe.
    Absent,
    /// The lookup could not be performed (non-2xx, transport error,
    /// unparseable response). Absence is NOT proven: the entry must
    /// stay uncertain and must never be re-created on this. Carries
    /// the stderr error class.
    CouldNotCheck(String),
}

pub(super) fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// The drainer's GitHub leg (production `Creator` for the
/// creation_outbox state machine): submits with the entry's marker
/// in the body, classifies the result, and verifies by read-back.
pub(super) struct GhCreator {
    pub(super) gh: GhRunner,
}

impl GhCreator {
    fn new() -> Self {
        GhCreator {
            gh: GhRunner::system(),
        }
    }

    fn with_runner(gh: GhRunner) -> Self {
        GhCreator { gh }
    }
}

impl GhCreator {
    fn create(&mut self, entry: &Value) -> crate::creation_outbox::CreateOutcome {
        use crate::creation_outbox::CreateOutcome;
        let repo = s(entry, "repo");
        let kind = s(entry, "kind");
        let id = s(entry, "id");
        let payload = entry.get("payload").cloned().unwrap_or(json!({}));
        let body = format!(
            "{}\n\n{}",
            s(&payload, "body"),
            crate::creation_outbox::marker(&id)
        );
        let title = s(&payload, "title");
        let argv: Vec<String> = match kind.as_str() {
            "issue" => {
                let mut a = vec![
                    "issue".into(),
                    "create".into(),
                    "--repo".into(),
                    repo.clone(),
                    "--title".into(),
                    title,
                    "--body".into(),
                    body,
                ];
                if let Some(labels) = payload.get("labels").and_then(|l| l.as_array()) {
                    for label in labels.iter().filter_map(|l| l.as_str()) {
                        a.push("--label".into());
                        a.push(label.to_string());
                    }
                }
                a
            }
            "issue_comment" | "pr_comment" => {
                let target = payload
                    .get("target_number")
                    .and_then(|n| n.as_i64())
                    .unwrap_or(0);
                if target <= 0 {
                    return CreateOutcome::Failed(format!("{kind} payload needs target_number"));
                }
                vec![
                    "issue".into(),
                    "comment".into(),
                    target.to_string(),
                    "--repo".into(),
                    repo.clone(),
                    "--body".into(),
                    body,
                ]
            }
            "pull_request" => vec![
                "pr".into(),
                "create".into(),
                "--repo".into(),
                repo.clone(),
                "--title".into(),
                title,
                "--body".into(),
                body,
                "--head".into(),
                s(&payload, "head"),
                "--base".into(),
                {
                    let base = s(&payload, "base");
                    if base.is_empty() {
                        "main".to_string()
                    } else {
                        base
                    }
                },
            ],
            other => return CreateOutcome::Failed(format!("unknown outbox kind {other:?}")),
        };
        let (ok, stdout, stderr) = self.gh.run(&argv);
        if !ok {
            let lower = stderr.to_lowercase();
            if lower.contains("403")
                || lower.contains("429")
                || lower.contains("rate limit")
                || lower.contains("submitted too quickly")
            {
                return CreateOutcome::RateLimited { retry_after: None };
            }
            return CreateOutcome::Failed(stderr);
        }
        // The created object's URL is the last stdout line; its
        // trailing path segment (or #issuecomment-<id>) is the
        // number the read-back verifies. Empty or unparseable
        // output on a zero exit is the silent-loss shape.
        let url = stdout.lines().last().unwrap_or("").trim().to_string();
        let number = url
            .rsplit('#')
            .next()
            .and_then(|frag| frag.strip_prefix("issuecomment-"))
            .and_then(|n| n.parse::<i64>().ok())
            .or_else(|| url.rsplit('/').next().and_then(|n| n.parse::<i64>().ok()));
        match number {
            Some(number) => CreateOutcome::Created { number, url },
            None => {
                CreateOutcome::Ambiguous(format!("gh succeeded with unusable output: {stdout:?}"))
            }
        }
    }

    fn read_back(&mut self, entry: &Value, number: i64) -> bool {
        matches!(self.read_back_checked(entry, number), Ok(true))
    }

    /// The read-back with the failure kept: `Err(class)` when the
    /// verifying call itself could not be performed, so a marker
    /// lookup never mistakes a failed verification for a proven
    /// absence (#8361).
    fn read_back_checked(&mut self, entry: &Value, number: i64) -> Result<bool, String> {
        let repo = s(entry, "repo");
        let kind = s(entry, "kind");
        let id = s(entry, "id");
        let obj = match kind.as_str() {
            "issue" => self.gh.json(&[
                "issue".into(),
                "view".into(),
                number.to_string(),
                "--repo".into(),
                repo,
                "--json".into(),
                "number,url,body".into(),
            ])?,
            "pull_request" => self.gh.json(&[
                "pr".into(),
                "view".into(),
                number.to_string(),
                "--repo".into(),
                repo,
                "--json".into(),
                "number,url,body".into(),
            ])?,
            _ => self.gh.json(&[
                "api".into(),
                format!("repos/{repo}/issues/comments/{number}"),
            ])?,
        };
        Ok(crate::creation_outbox::marker_id(&s(&obj, "body")) == Some(id))
    }

    /// Reconcile an uncertain entry by searching for its marker.
    /// Three-way (#8361): the search is issued as a GET — the old
    /// `gh api search/issues -f q=...` form makes gh use POST,
    /// /search/issues has no POST route (404), and the failure read
    /// as proven absence, so every uncertain entry was re-created.
    /// Search absence alone never proves absence for issue-kind
    /// entries (#8331): the search index lags, so the REST issues
    /// list is scanned as a second leg before `Absent` (see
    /// `find_issue_in_recent_list`).
    fn find_by_marker(&mut self, entry: &Value) -> MarkerLookup {
        let repo = s(entry, "repo");
        let kind = s(entry, "kind");
        let id = s(entry, "id");
        let q = format!("repo:{repo} fleet-outbox:{id}");
        let results = match self.gh.json(&[
            "api".into(),
            "--method".into(),
            "GET".into(),
            "search/issues".into(),
            "-f".into(),
            format!("q={q}"),
        ]) {
            Ok(v) => v,
            Err(class) => return MarkerLookup::CouldNotCheck(class),
        };
        let Some(items) = results.get("items").and_then(|i| i.as_array()) else {
            return MarkerLookup::CouldNotCheck("unexpected_search_shape".to_string());
        };
        // For comment entries the search hit's number is the ISSUE
        // (or PR) number -- a different id namespace from comment
        // ids -- so the marker must be resolved through the issue's
        // comment list, and what is recorded is the COMMENT's id and
        // html_url (#8363). Reading the issue number back as a
        // comment id can never match a landed comment, so every
        // landed comment was judged absent and re-created.
        let comment_kind = matches!(kind.as_str(), "issue_comment" | "pr_comment");
        for item in items {
            let Some(number) = item.get("number").and_then(|n| n.as_i64()) else {
                continue;
            };
            if comment_kind {
                match self.find_comment_on_issue(entry, number) {
                    Ok(Some(found)) => return MarkerLookup::Found(found.0, found.1),
                    Ok(None) => {}
                    Err(class) => return MarkerLookup::CouldNotCheck(class),
                }
                continue;
            }
            let url = s(item, "html_url");
            match self.read_back_checked(entry, number) {
                Ok(true) => return MarkerLookup::Found(number, url),
                Ok(false) => {}
                Err(class) => return MarkerLookup::CouldNotCheck(class),
            }
        }
        // #8331: search is only eventually consistent -- an issue
        // created seconds-to-minutes before a drainer restart (the
        // crash window the outbox exists to protect) can be missing
        // from the index while already existing, and concluding
        // Absent from search alone re-creates it. For issue-kind
        // entries, absence must also survive a scan of the strongly
        // consistent REST issues list before it is proven.
        if kind == "issue" {
            match self.find_issue_in_recent_list(entry) {
                Ok(Some(found)) => return MarkerLookup::Found(found.0, found.1),
                Ok(None) => {}
                Err(class) => return MarkerLookup::CouldNotCheck(class),
            }
        }
        MarkerLookup::Absent
    }

    /// Scan the repo's most recent issues through the REST issues
    /// list -- a strongly consistent source, unlike the search
    /// index -- for an issue-kind entry's marker (#8331). Returns
    /// the (number, html_url) of the issue carrying the marker, or
    /// None when the scan terminated (a non-full page) without
    /// finding it. Any failed or misshaped page, and a listing
    /// that still has not terminated at the page bound, is an
    /// `Err` carrying the error class -- never a proven absence,
    /// the same discipline as the comment-list bound in
    /// `find_comment_on_issue`. The bound covers the 500 most
    /// recently created issues: a create that landed in the crash
    /// window this leg protects is necessarily among them.
    fn find_issue_in_recent_list(
        &mut self,
        entry: &Value,
    ) -> Result<Option<(i64, String)>, String> {
        let repo = s(entry, "repo");
        let id = s(entry, "id");
        const PER_PAGE: usize = 100;
        const MAX_PAGES: usize = 5;
        for page in 1..=MAX_PAGES {
            let issues = self.gh.json(&[
                "api".into(),
                format!(
                    "repos/{repo}/issues?state=all&sort=created&direction=desc&per_page={PER_PAGE}&page={page}"
                ),
            ])?;
            let Some(list) = issues.as_array() else {
                return Err("unexpected_issues_list_shape".to_string());
            };
            let full_page = list.len() == PER_PAGE;
            for issue in list {
                if crate::creation_outbox::marker_id(&s(issue, "body")) == Some(id.clone()) {
                    let Some(number) = issue.get("number").and_then(|n| n.as_i64()) else {
                        continue;
                    };
                    return Ok(Some((number, s(issue, "html_url"))));
                }
            }
            if !full_page {
                return Ok(None);
            }
        }
        Err("issues_list_unbounded".to_string())
    }

    /// Resolve a comment entry's marker on one candidate issue:
    /// page the issue's comments and return the (id, html_url) of
    /// the one carrying the entry's marker, or None when the issue's
    /// comments were fully listed and none carries it (#8363). The
    /// page bound mirrors the fallback marker read's: a listing that
    /// still has not terminated there is an error, never a proven
    /// absence.
    fn find_comment_on_issue(
        &mut self,
        entry: &Value,
        issue_number: i64,
    ) -> Result<Option<(i64, String)>, String> {
        let repo = s(entry, "repo");
        let id = s(entry, "id");
        const PER_PAGE: usize = 100;
        const MAX_PAGES: usize = 10;
        for page in 1..=MAX_PAGES {
            let comments = self.gh.json(&[
                "api".into(),
                format!(
                    "repos/{repo}/issues/{issue_number}/comments?per_page={PER_PAGE}&page={page}"
                ),
            ])?;
            let Some(list) = comments.as_array() else {
                return Err("unexpected_comments_shape".to_string());
            };
            let full_page = list.len() == PER_PAGE;
            for comment in list {
                if crate::creation_outbox::marker_id(&s(comment, "body")) == Some(id.clone()) {
                    let Some(cid) = comment.get("id").and_then(|n| n.as_i64()) else {
                        continue;
                    };
                    return Ok(Some((cid, s(comment, "html_url"))));
                }
            }
            if !full_page {
                return Ok(None);
            }
        }
        Err("comments_list_unbounded".to_string())
    }
}

pub(super) fn cmd_outbox_drain(rest: &[String]) -> i32 {
    use crate::creation_outbox::CreateOutcome;
    let args = parse_verb_args("outbox drain", rest, &["owner", "max"], &[]);
    reject_extra_pos("outbox drain", &args, 0);
    let owner = args
        .get("owner")
        .unwrap_or_else(|| "outbox-drainer".to_string());
    let max = args.get_i64("max").unwrap_or(20).max(1);
    let mut creator = GhCreator::new();
    let mut processed = 0i64;
    let mut last_create: Option<std::time::Instant> = None;
    for _ in 0..max {
        let resp = outbox_claim(&owner);
        let row = resp.get("row").cloned().unwrap_or(Value::Null);
        if row.is_null() {
            println!(
                "{}",
                json!({"drained": processed, "idle": true, "claim": resp})
            );
            return 0;
        }
        let id = s(&row, "id");
        let state = s(&row, "state");
        processed += 1;

        // Uncertain first: reconcile by marker, never blind-retry.
        if state == "uncertain" {
            match creator.find_by_marker(&row) {
                MarkerLookup::Found(number, url) => {
                    let resp = outbox_resolve(
                        &id,
                        "done",
                        json!({"github_number": number, "github_url": url, "reconciled": true}),
                    );
                    println!(
                        "{}",
                        json!({"id": id, "outcome": "reconciled", "number": number, "row": resp.get("row")})
                    );
                    continue;
                }
                MarkerLookup::Absent => {
                    // Marker search proves absence: fall through and
                    // re-create under the same entry.
                }
                MarkerLookup::CouldNotCheck(class) => {
                    // The lookup itself failed (#8361): absence is
                    // NOT proven. Keep the entry uncertain (the
                    // resolve applies the backoff) with the error
                    // class recorded, and never re-create on an
                    // unchecked lookup.
                    let resp = outbox_resolve(
                        &id,
                        "uncertain",
                        json!({"error": format!("marker lookup could not check: {class}")}),
                    );
                    println!(
                        "{}",
                        json!({"id": id, "outcome": "lookup_unchecked", "error_class": class, "row": resp.get("row")})
                    );
                    continue;
                }
            }
        }

        // Pacing: the minimum gap between creates (the broker also
        // enforces the circuit and the hourly budget at claim).
        if let Some(last) = last_create {
            let elapsed = last.elapsed().as_secs() as i64;
            let wait = crate::creation_outbox::MIN_GAP_SECONDS - elapsed;
            if wait > 0 {
                std::thread::sleep(Duration::from_secs(wait as u64));
            }
        }
        last_create = Some(std::time::Instant::now());
        match creator.create(&row) {
            CreateOutcome::Created { number, url } => {
                if creator.read_back(&row, number) {
                    outbox_resolve(
                        &id,
                        "done",
                        json!({"github_number": number, "github_url": url}),
                    );
                    println!(
                        "{}",
                        json!({"id": id, "outcome": "done", "number": number, "url": url})
                    );
                } else {
                    outbox_resolve(
                        &id,
                        "uncertain",
                        json!({"error": "read-back failed after create"}),
                    );
                    println!(
                        "{}",
                        json!({"id": id, "outcome": "uncertain", "reason": "readback_failed"})
                    );
                }
            }
            CreateOutcome::RateLimited { retry_after } => {
                outbox_resolve(&id, "rate_limited", json!({"retry_after": retry_after}));
                println!(
                    "{}",
                    json!({"id": id, "outcome": "rate_limited", "retry_after": retry_after})
                );
            }
            CreateOutcome::Ambiguous(why) => {
                outbox_resolve(&id, "uncertain", json!({"error": why}));
                println!(
                    "{}",
                    json!({"id": id, "outcome": "uncertain", "reason": why})
                );
            }
            CreateOutcome::Failed(why) => {
                outbox_resolve(&id, "failed", json!({"error": why}));
                println!("{}", json!({"id": id, "outcome": "failed", "reason": why}));
            }
        }
    }
    println!("{}", json!({"drained": processed, "idle": false}));
    0
}

/// Merge-intent stack verbs (#8582 increments 3-4): the client
/// surface for the broker's /stack-record and /stack-list routes
/// (increment 2), the /stack-transition route, and the replay
/// executor (increment 4, `stack replay` below). Recording a finished branch names its authoring lane/worker
/// pair; the broker validates that pair against the lane registry
/// (the validation deferred from the CodeRabbit round on PR #8660,
/// where the pair originates — here, at the client verbs).
/// Stack list answers the full stack state from the database alone.
pub(crate) fn stack_record_body(
    stack_id: &str,
    repo: &str,
    branch: &str,
    base_branch: Option<&str>,
    head_sha: Option<&str>,
    issue_number: Option<i64>,
    pr_number: Option<i64>,
    owner_lane: &str,
    owner_worker: &str,
    owner_session: Option<&str>,
) -> Result<Value, String> {
    // Fail fast with the pure core's own append rules, so a bad
    // record is a usage error here and the identical verdict at
    // the broker (validate_append is the one rule set, #8582).
    let append = crate::merge_intent::AppendRequest {
        stack_id: stack_id.to_string(),
        repo: repo.to_string(),
        branch: branch.to_string(),
        base_branch: base_branch.map(|s| s.to_string()),
        head_sha: head_sha.map(|s| s.to_string()),
        issue_number,
        pr_number,
        owner_lane: owner_lane.to_string(),
        owner_worker: owner_worker.to_string(),
        owner_session: owner_session.map(|s| s.to_string()),
    };
    crate::merge_intent::validate_append(&append)?;
    Ok(json!({
        "stack_id": stack_id,
        "repo": repo,
        "branch": branch,
        "base_branch": base_branch,
        "head_sha": head_sha,
        "issue_number": issue_number,
        "pr_number": pr_number,
        "owner_lane": owner_lane,
        "owner_worker": owner_worker,
        "owner_session": owner_session,
    }))
}

pub(super) fn cmd_stack_record(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "stack record",
        rest,
        &[
            "stack",
            "repo",
            "branch",
            "base-branch",
            "head-sha",
            "issue",
            "pr",
            "lane",
            "worker",
            "session-id",
        ],
        &[],
    );
    reject_extra_pos("stack record", &args, 0);
    let stack_id = args
        .get("stack")
        .unwrap_or_else(|| die(2, "stack record: --stack is required"));
    let repo = args
        .get("repo")
        .unwrap_or_else(|| die(2, "stack record: --repo is required"));
    let branch = args
        .get("branch")
        .unwrap_or_else(|| die(2, "stack record: --branch is required"));
    // The owner pair originates here (#8582): the authoring lane is
    // required, and the worker defaults to this caller's worker —
    // the broker refuses a pair the lane registry does not hold.
    let owner_lane = args
        .get("lane")
        .unwrap_or_else(|| die(2, "stack record: --lane is required (the authoring lane)"));
    let body = match stack_record_body(
        &stack_id,
        &repo,
        &branch,
        args.get("base-branch").as_deref(),
        args.get("head-sha").as_deref(),
        args.get_i64("issue"),
        args.get_i64("pr"),
        &owner_lane,
        &opt_worker(&args),
        args.get("session-id").as_deref(),
    ) {
        Ok(b) => b,
        Err(e) => die(2, &e),
    };
    let resp = unwrap_ok(call("/stack-record", &body));
    print_json(&require_row(&resp, "stack_record"));
    0
}

pub(super) fn cmd_stack_list(rest: &[String]) -> i32 {
    let args = parse_verb_args("stack list", rest, &["stack", "worker"], &[]);
    reject_extra_pos("stack list", &args, 1);
    let positional = args.pos.first().cloned();
    let stack_id = match (positional, args.get("stack")) {
        (Some(p), Some(f)) if p != f => {
            die(2, "stack list: positional stack id and --stack disagree")
        }
        (Some(p), _) => p,
        (None, Some(f)) => f,
        (None, None) => die(2, "stack list: a stack id is required"),
    };
    let resp = unwrap_ok(call("/stack-list", &json!({"stack_id": stack_id})));
    print_json(&require_row(&resp, "stack_list"));
    0
}

/// Body for the broker's /stack-transition route (#8582 increment
/// 4), shaped and fail-fast validated here: the target state must
/// parse in the pure core, and a move to `conflicted` must carry
/// the detail the authoring lane will resolve against — a
/// conflict recorded without context strands its owner.
pub(crate) fn stack_transition_body(
    stack_id: &str,
    position: i64,
    to_state: &str,
    pr_number: Option<i64>,
    conflict_detail: Option<&str>,
) -> Result<Value, String> {
    if stack_id.trim().is_empty() {
        return Err("stack transition requires a non-empty stack_id".to_string());
    }
    let Some(to) = crate::merge_intent::IntentState::parse(to_state) else {
        return Err(format!(
            "stack transition: unknown target state {to_state:?}"
        ));
    };
    if to == crate::merge_intent::IntentState::Conflicted {
        match conflict_detail {
            Some(d) if !d.trim().is_empty() => {}
            _ => {
                return Err(
                    "stack transition: a move to conflicted requires --conflict-detail \
                     naming what the authoring lane must resolve"
                        .to_string(),
                );
            }
        }
    }
    Ok(json!({
        "stack_id": stack_id,
        "position": position,
        "to_state": to.as_str(),
        "pr_number": pr_number,
        "conflict_detail": conflict_detail,
    }))
}

pub(super) fn stack_transition(
    stack_id: &str,
    position: i64,
    to_state: &str,
    pr_number: Option<i64>,
    conflict_detail: Option<&str>,
) -> Value {
    let body = match stack_transition_body(stack_id, position, to_state, pr_number, conflict_detail)
    {
        Ok(b) => b,
        Err(e) => die(2, &e),
    };
    let resp = unwrap_ok(call("/stack-transition", &body));
    require_row(&resp, "stack_transition")
}

/// The replay executor's transition helper: like
/// [`stack_transition`] but it returns the broker's verdict instead
/// of exiting on it (CodeRabbit finding, PR #8849, comment
/// 4547592887). The broker's store/transport failures carry exit 4,
/// which inside `stack replay` is reserved for a conflict routed to
/// its authoring lane — a helper that dies with the envelope's code
/// makes a store failure indistinguishable from a routed conflict,
/// and prints no replay JSON at all. Replay maps an Err onto its
/// transient path (exit 3, intent left `replaying`, resumable).
pub(super) fn stack_transition_result(
    stack_id: &str,
    position: i64,
    to_state: &str,
    pr_number: Option<i64>,
    conflict_detail: Option<&str>,
) -> Result<Value, String> {
    let body = stack_transition_body(stack_id, position, to_state, pr_number, conflict_detail)?;
    let resp = call("/stack-transition", &body);
    if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
        let msg = resp
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("queue-broker error");
        return Err(format!("stack transition to {to_state} failed: {msg}"));
    }
    Ok(resp.get("row").cloned().unwrap_or(Value::Null))
}

pub(super) fn cmd_stack_transition(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "stack transition",
        rest,
        &["stack", "position", "to", "pr", "conflict-detail", "worker"],
        &[],
    );
    reject_extra_pos("stack transition", &args, 0);
    let stack_id = args
        .get("stack")
        .unwrap_or_else(|| die(2, "stack transition: --stack is required"));
    let position = args
        .get_i64("position")
        .unwrap_or_else(|| die(2, "stack transition: --position is required"));
    let to = args
        .get("to")
        .unwrap_or_else(|| die(2, "stack transition: --to is required"));
    let row = stack_transition(
        &stack_id,
        position,
        &to,
        args.get_i64("pr"),
        args.get("conflict-detail").as_deref(),
    );
    print_json(&row);
    0
}

/// Body for the broker's /stack-restack route (#8582; Codex P2,
/// PR #8849, comment 4547537340): the acknowledgement that an
/// intent's author restacked its branch off a withdrawn base, so
/// the recorded base_branch can be refreshed. Fail-fast validated
/// here for shape; the broker re-plans with the pure core's
/// `plan_restack` against the stored rows, which is where the
/// taint rules bind.
pub(crate) fn stack_restack_body(
    stack_id: &str,
    position: i64,
    new_base_branch: &str,
    new_head_sha: Option<&str>,
) -> Result<Value, String> {
    if stack_id.trim().is_empty() {
        return Err("stack restack requires a non-empty stack_id".to_string());
    }
    if new_base_branch.trim().is_empty() {
        return Err(
            "stack restack requires a non-empty --base (the branch's new base)".to_string(),
        );
    }
    if let Some(sha) = new_head_sha {
        if sha.trim().is_empty() {
            return Err(
                "stack restack requires a non-empty --head-sha when one is given".to_string(),
            );
        }
    }
    Ok(json!({
        "stack_id": stack_id,
        "position": position,
        "new_base_branch": new_base_branch,
        "head_sha": new_head_sha,
    }))
}

pub(super) fn cmd_stack_restack(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "stack restack",
        rest,
        &["stack", "position", "base", "head-sha", "worker"],
        &[],
    );
    reject_extra_pos("stack restack", &args, 0);
    let stack_id = args
        .get("stack")
        .unwrap_or_else(|| die(2, "stack restack: --stack is required"));
    let position = args
        .get_i64("position")
        .unwrap_or_else(|| die(2, "stack restack: --position is required"));
    let base = args.get("base").unwrap_or_else(|| {
        die(
            2,
            "stack restack: --base is required (the branch's new base after restacking)",
        )
    });
    let body = match stack_restack_body(&stack_id, position, &base, args.get("head-sha").as_deref())
    {
        Ok(b) => b,
        Err(e) => die(2, &e),
    };
    let resp = unwrap_ok(call("/stack-restack", &body));
    print_json(&require_row(&resp, "stack_restack"));
    0
}

/// One recorded intent, parsed back from a /stack-list row for the
/// replay executor (the broker's row shape is the storage shape).
pub(super) fn stack_intent_from_row(row: &Value) -> Option<crate::merge_intent::MergeIntent> {
    Some(crate::merge_intent::MergeIntent {
        stack_id: row.get("stack_id")?.as_str()?.to_string(),
        position: row.get("position")?.as_i64()?,
        repo: row.get("repo")?.as_str()?.to_string(),
        branch: row.get("branch")?.as_str()?.to_string(),
        base_branch: row.get("base_branch")?.as_str()?.to_string(),
        head_sha: row
            .get("head_sha")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        base_head_sha: row
            .get("base_head_sha")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        issue_number: row.get("issue_number").and_then(|v| v.as_i64()),
        pr_number: row.get("pr_number").and_then(|v| v.as_i64()),
        owner_lane: row.get("owner_lane")?.as_str()?.to_string(),
        owner_worker: row.get("owner_worker")?.as_str()?.to_string(),
        owner_session: row
            .get("owner_session")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        state: crate::merge_intent::IntentState::parse(row.get("state")?.as_str()?)?,
    })
}

pub(super) fn replay_snippet(s: &str) -> String {
    s.chars().take(300).collect()
}

/// Run one external step of a replay (git/gh) in the repo
/// checkout: Ok(trimmed stdout) on success, Err(a bounded
/// stderr/stdout summary) otherwise.
pub(super) fn replay_run(dir: &str, program: &str, argv: &[String]) -> Result<String, String> {
    match std::process::Command::new(program)
        .args(argv)
        .current_dir(dir)
        .output()
    {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            Err(replay_snippet(if err.is_empty() { &out } else { &err }))
        }
        Err(e) => Err(format!("{program}: {e}")),
    }
}

/// Whether the checkout at `dir` already has a rebase in progress
/// that this replay did NOT start (Codex P2, PR #8849). Replay must
/// refuse that checkout state up front: `git rebase --abort` on a
/// rebase somebody else is mid-way through would discard their
/// conflict-resolution progress.
pub(super) fn rebase_in_progress(dir: &str) -> bool {
    for kind in ["rebase-merge", "rebase-apply"] {
        if let Ok(p) = replay_run(dir, "git", &replay_args(&["rev-parse", "--git-path", kind])) {
            let path = std::path::Path::new(&p);
            let full = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::path::Path::new(dir).join(path)
            };
            if full.exists() {
                return true;
            }
        }
    }
    false
}

pub(super) fn replay_args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// The replay executor (#8582 increment 4): walk one stack's
/// recorded merge order bottom-up and land it — rebase onto the
/// new main, push, open the PR, merge, mark landed, post the
/// outcome note — one intent at a time, in the recorded order.
/// The plan is the pure core's (`plan_for_walk` over the
/// `resumable_walk`), so `--dry-run` prints exactly what a live
/// pass would do. A rebase or merge conflict is never resolved
/// here: the intent is marked `conflicted` with the detail, the
/// walk stops, and the summary routes the conflict to the
/// authoring lane recorded on the intent (exit 4). Any other step
/// failure stops the pass with the intent left `replaying` — a
/// later `stack replay` resumes it (exit 3).
pub(super) fn cmd_stack_replay(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "stack replay",
        rest,
        &["stack", "worker", "repo-dir"],
        &["dry-run"],
    );
    reject_extra_pos("stack replay", &args, 0);
    let stack_id = args
        .get("stack")
        .unwrap_or_else(|| die(2, "stack replay: --stack is required"));
    let dir = args.get("repo-dir").unwrap_or_else(|| ".".to_string());
    let resp = unwrap_ok(call("/stack-list", &json!({"stack_id": stack_id})));
    let state = require_row(&resp, "stack_list");
    let rows = state
        .get("intents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let intents: Vec<crate::merge_intent::MergeIntent> =
        rows.iter().filter_map(stack_intent_from_row).collect();
    let walk = crate::merge_intent::resumable_walk(&intents, &stack_id);
    let plan = crate::merge_intent::plan_for_walk(&walk);
    if args.has("dry-run") {
        let steps: Vec<Value> = plan
            .iter()
            .map(|s| {
                json!({
                    "position": s.position,
                    "branch": s.branch,
                    "action": s.action.as_str(),
                })
            })
            .collect();
        print_json(&json!({"stack_id": stack_id, "dry_run": true, "plan": steps}));
        return 0;
    }
    if plan.is_empty() {
        print_json(&json!({
            "stack_id": stack_id,
            "landed": [],
            "note": "no replayable prefix: the stack is empty, finished, or blocked at a conflicted head (see stack state)",
            "stack": state,
        }));
        return 0;
    }
    let by_position: std::collections::HashMap<i64, &crate::merge_intent::MergeIntent> =
        intents.iter().map(|i| (i.position, i)).collect();
    let mut pr_numbers: std::collections::HashMap<i64, Option<i64>> =
        intents.iter().map(|i| (i.position, i.pr_number)).collect();
    let mut landed: Vec<i64> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    // The verified pre-rebase tip of every branch this pass rebases
    // (position -> sha recorded immediately before its rebase ran):
    // the Push step pins its force-with-lease to this tip and refuses
    // to rewrite a branch whose remote moved past it (Codex P1,
    // PR #8849, comment 4547537332).
    // Persisted across attempts (Codex P2, PR #8849, comment
    // 4548246124): a pass that rebases a published branch and then
    // stops transiently at Push must not lose the verified tip --
    // on the next invocation the intent can look like the walk's
    // bottom and never rebase again, so the tip is stored in the
    // checkout's git dir, keyed by stack, and cleared as intents
    // land. A stale entry is only ever used as a force-with-lease
    // pin after a live ls-remote confirms the remote is still
    // exactly at that tip.
    let tips_path = replay_run(
        &dir,
        "git",
        &replay_args(&[
            "rev-parse",
            "--git-path",
            "fleet-replay-pre-rebase-tips.json",
        ]),
    )
    .ok()
    .map(|rel| {
        let path = std::path::Path::new(&rel);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::path::Path::new(&dir).join(path)
        }
    });
    let mut pre_rebase_tips: std::collections::HashMap<i64, String> =
        std::collections::HashMap::new();
    if let Some(path) = &tips_path {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(saved) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) {
                if let Some(entry) = saved.get(&stack_id) {
                    if let Some(obj) = entry.as_object() {
                        for (pos, sha) in obj {
                            if let (Ok(pos), Some(sha)) = (pos.parse::<i64>(), sha.as_str()) {
                                pre_rebase_tips.insert(pos, sha.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    if let Err(e) = replay_run(&dir, "git", &replay_args(&["fetch", "origin", "main"])) {
        print_json(&json!({
            "stack_id": stack_id,
            "landed": landed,
            "stopped_at": {"step": "fetch", "error": e},
        }));
        return 3;
    }
    // Preflight (Codex P2, PR #8849): a checkout with somebody
    // else's rebase already in progress is not replay's to abort
    // or build on — refuse before any step runs.
    if rebase_in_progress(&dir) {
        print_json(&json!({
            "stack_id": stack_id,
            "landed": landed,
            "stopped_at": {"step": "preflight", "error": "checkout already has a rebase in progress that this replay did not start; finish or abort it outside replay first"},
        }));
        return 3;
    }
    // Shared conflict exit: mark the intent conflicted (it is
    // `replaying` by then — Begin precedes every other step in the
    // plan), name the route to its authoring lane, and stop.
    let conflict_exit = |position: i64, detail: String, landed: &[i64]| -> i32 {
        // Exit 4 is earned only once the conflicted transition is
        // recorded (CodeRabbit, PR #8849): if the store refuses it,
        // the conflict is not routed yet — report a resumable stop
        // (exit 3) with the conflict named, never a bare 4.
        if let Err(e) =
            stack_transition_result(&stack_id, position, "conflicted", None, Some(&detail))
        {
            print_json(&json!({
                "stack_id": stack_id,
                "landed": landed,
                "stopped_at": {"position": position, "step": "conflict_transition", "error": e},
                "conflict": {"position": position, "detail": detail},
                "note": "the conflict could not be recorded; the intent is left replaying and a later pass resumes it",
            }));
            return 3;
        }
        let route = crate::merge_intent::route_replay_conflict(&intents, &stack_id, position);
        print_json(&json!({
            "stack_id": stack_id,
            "landed": landed,
            "conflict": {
                "position": position,
                "detail": detail,
                "route": route.map(|r| json!({
                    "owner_lane": r.owner_lane,
                    "owner_worker": r.owner_worker,
                    "owner_session": r.owner_session,
                    "repo": r.repo,
                    "branch": r.branch,
                    "base_branch": r.base_branch,
                    "issue_number": r.issue_number,
                })),
            },
        }));
        4
    };
    let transient_exit = |step: &crate::merge_intent::PlannedStep,
                          e: String,
                          landed: &[i64]|
     -> i32 {
        print_json(&json!({
            "stack_id": stack_id,
            "landed": landed,
            "stopped_at": {
                "position": step.position,
                "action": step.action.as_str(),
                "error": e,
            },
            "note": "the intent is left replaying; fix the cause and re-run stack replay to resume",
        }));
        3
    };
    for step in &plan {
        let intent = by_position[&step.position];
        let repo = intent.repo.clone();
        let branch = intent.branch.clone();
        match step.action {
            crate::merge_intent::ReplayAction::Begin => {
                if intent.state != crate::merge_intent::IntentState::Replaying {
                    if let Err(e) =
                        stack_transition_result(&stack_id, step.position, "replaying", None, None)
                    {
                        return transient_exit(step, e, &landed);
                    }
                }
            }
            crate::merge_intent::ReplayAction::RebaseOntoMain => {
                if rebase_in_progress(&dir) {
                    return transient_exit(
                        step,
                        "checkout already has a rebase in progress that this replay did not start; finish or abort it outside replay first".to_string(),
                        &landed,
                    );
                }
                if let Ok(tip) = replay_run(&dir, "git", &replay_args(&["rev-parse", &branch])) {
                    pre_rebase_tips.insert(step.position, tip);
                    if let Some(path) = &tips_path {
                        let mut saved: serde_json::Map<String, Value> =
                            std::fs::read_to_string(path)
                                .ok()
                                .and_then(|t| serde_json::from_str(&t).ok())
                                .unwrap_or_default();
                        let entry = saved.entry(stack_id.clone()).or_insert_with(|| json!({}));
                        if let Some(obj) = entry.as_object_mut() {
                            for (pos, sha) in &pre_rebase_tips {
                                obj.insert(pos.to_string(), json!(sha));
                            }
                        }
                        let _ =
                            std::fs::write(path, serde_json::to_string(&saved).unwrap_or_default());
                    }
                }
                // Merge-bearing histories are refused, never
                // flattened (Codex P1, PR #8849, comment
                // 4549335625): a plain rebase skips merge commits,
                // so a merge commit carrying merge-only changes (a
                // fixup or new file added while merging the base)
                // would be silently discarded and the altered tree
                // published as success. If the range replay would
                // rebase contains any merge commit, route it to
                // the authoring lane as a conflict: the author
                // restacks preserving the merges (or merges main
                // in) and replay resumes afterwards.
                match replay_run(
                    &dir,
                    "git",
                    &replay_args(&["rev-list", "--merges", &format!("origin/main..{branch}")]),
                ) {
                    Ok(list) if !list.is_empty() => {
                        return conflict_exit(
                            step.position,
                            format!(
                                "branch {branch} contains merge commits ({}) in the range replay would rebase onto main; a plain rebase would discard their merge-only changes, so replay refuses to flatten this history -- the authoring lane restacks it preserving the merges and re-runs replay",
                                list.lines().take(5).collect::<Vec<_>>().join(", ")
                            ),
                            &landed,
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return transient_exit(
                            step,
                            format!(
                                "checking {branch} for merge commits before its rebase failed: {e}"
                            ),
                            &landed,
                        );
                    }
                }
                if let Err(e) = replay_run(
                    &dir,
                    "git",
                    &replay_args(&["rebase", "origin/main", &branch]),
                ) {
                    // Distinguish a rebase conflict from a rebase
                    // that never started (Codex P2, PR #8849,
                    // comment 4547537349): git refuses to START a
                    // rebase — dirty checkout, missing branch,
                    // spawn failure — without leaving any rebase
                    // state behind, and that failure is transient:
                    // the intent stays `replaying` and a later pass
                    // resumes it once the cause is fixed. Only a
                    // rebase this step actually started can be a
                    // conflict — the guard above proved none was in
                    // progress before, so state present now is ours
                    // to abort before routing the conflict to the
                    // authoring lane.
                    if rebase_in_progress(&dir) {
                        let _ = replay_run(&dir, "git", &replay_args(&["rebase", "--abort"]));
                        return conflict_exit(
                            step.position,
                            format!("rebase of {branch} onto origin/main failed at replay: {e}"),
                            &landed,
                        );
                    }
                    return transient_exit(
                        step,
                        format!("rebase of {branch} onto origin/main never started: {e}"),
                        &landed,
                    );
                }
            }
            crate::merge_intent::ReplayAction::Push => {
                // Publishing a branch must never delete commits this
                // pass has not accounted for (Codex P1, PR #8849,
                // comment 4547537332). The earlier fix for
                // already-published branches (comment 4546983376)
                // used a bare --force-with-lease, whose expected
                // value is the mutable remote-tracking ref: an
                // ordinary fetch can move that ref to a tip carrying
                // remote-only commits the local branch lacks, and
                // the lease then passes while the push deletes them.
                // So decide from the LIVE remote tip, read with
                // ls-remote at push time:
                // - no remote branch yet: plain first push;
                // - remote tip == local tip, or an ancestor of the
                //   local branch: a no-op or a fast-forward (the
                //   untouched bottom branch's case) — plain push;
                // - this pass rebased the branch and the remote tip
                //   is still the verified pre-rebase tip: the
                //   rewrite is this pass's alone — publish with the
                //   lease PINNED to that tip;
                // - anything else (the remote moved past what this
                //   pass verified): refuse. The intent stays
                //   `replaying`; its author reconciles the
                //   remote-only commits and a later pass resumes.
                let remote_tip = replay_run(
                    &dir,
                    "git",
                    &replay_args(&["ls-remote", "origin", &format!("refs/heads/{branch}")]),
                )
                .ok()
                .and_then(|out| out.split_whitespace().next().map(|s| s.to_string()));
                let push = match remote_tip {
                    None => replay_run(&dir, "git", &replay_args(&["push", "origin", &branch])),
                    Some(remote_sha) => {
                        // Make the remote tip's object available for
                        // the ancestry check below.
                        let _ =
                            replay_run(&dir, "git", &replay_args(&["fetch", "origin", &branch]));
                        let local_tip =
                            replay_run(&dir, "git", &replay_args(&["rev-parse", &branch]))
                                .unwrap_or_default();
                        let remote_is_ancestor = replay_run(
                            &dir,
                            "git",
                            &replay_args(&["merge-base", "--is-ancestor", &remote_sha, &branch]),
                        )
                        .is_ok();
                        if remote_sha == local_tip || remote_is_ancestor {
                            replay_run(&dir, "git", &replay_args(&["push", "origin", &branch]))
                        } else if pre_rebase_tips.get(&step.position) == Some(&remote_sha) {
                            replay_run(
                                &dir,
                                "git",
                                &replay_args(&[
                                    "push",
                                    &format!("--force-with-lease=refs/heads/{branch}:{remote_sha}"),
                                    "origin",
                                    &branch,
                                ]),
                            )
                        } else {
                            return transient_exit(
                                step,
                                format!(
                                    "remote branch {branch} is at {remote_sha}, which is \
                                     neither contained in the local branch nor the verified \
                                     pre-rebase tip of this pass; refusing to force-push \
                                     over possible remote-only commits — the authoring \
                                     lane reconciles the remote branch first"
                                ),
                                &landed,
                            );
                        }
                    }
                };
                if let Err(e) = push {
                    return transient_exit(step, e, &landed);
                }
            }
            crate::merge_intent::ReplayAction::OpenPr => {
                if pr_numbers.get(&step.position).copied().flatten().is_none() {
                    let title = format!("stack replay {} #{}: {}", stack_id, step.position, branch);
                    let body = format!(
                        "Replayed from merge-intent stack {stack_id} position {} (#8582). \
                         Recorded author: lane {} / worker {}.",
                        step.position, intent.owner_lane, intent.owner_worker
                    );
                    let created = replay_run(
                        &dir,
                        "gh",
                        &replay_args(&[
                            "pr", "create", "--repo", &repo, "--base", "main", "--head", &branch,
                            "--title", &title, "--body", &body,
                        ]),
                    );
                    let found = match &created {
                        Ok(url) => url
                            .rsplit('/')
                            .next()
                            .and_then(|n| n.trim().parse::<i64>().ok()),
                        Err(_) => replay_run(
                            &dir,
                            "gh",
                            &replay_args(&[
                                "pr", "list", "--repo", &repo, "--head", &branch, "--state", "all",
                                "--json", "number", "--limit", "1",
                            ]),
                        )
                        .ok()
                        .and_then(|out| serde_json::from_str::<Value>(&out).ok())
                        .and_then(|v| {
                            v.as_array()
                                .and_then(|a| a.first())
                                .and_then(|r| r.get("number"))
                                .and_then(|n| n.as_i64())
                        }),
                    };
                    match found {
                        Some(n) => {
                            pr_numbers.insert(step.position, Some(n));
                        }
                        None => {
                            return transient_exit(
                                step,
                                created.err().unwrap_or_else(|| {
                                    "PR number not parseable from gh output".to_string()
                                }),
                                &landed,
                            );
                        }
                    }
                }
            }
            crate::merge_intent::ReplayAction::Merge => {
                let pr = pr_numbers
                    .get(&step.position)
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| die(2, "stack replay: merge step without a PR number"));
                let pr_s = pr.to_string();
                // Resume reconciliation (Codex P1, PR #8849): if an
                // earlier pass merged this PR but stopped before the
                // landed transition recorded it, the intent is still
                // `replaying` with no persisted number — the OpenPr
                // fallback above recovered the number across all PR
                // states, and an already-MERGED PR here is success,
                // not a merge to re-attempt (re-attempting fails and
                // would block the rest of the stack forever).
                let pr_view = replay_run(
                    &dir,
                    "gh",
                    &replay_args(&[
                        "pr",
                        "view",
                        &pr_s,
                        "--repo",
                        &repo,
                        "--json",
                        "state,baseRefName",
                    ]),
                )
                .ok()
                .and_then(|out| serde_json::from_str::<Value>(&out).ok());
                let pr_field = |name: &str| {
                    pr_view
                        .as_ref()
                        .and_then(|v| v.get(name))
                        .and_then(|f| f.as_str())
                        .map(|f| f.to_string())
                };
                let pr_state = pr_field("state");
                // Base validation (Codex P1, PR #8849, comment
                // 4546983383): a reused PR recorded from the stack
                // may still target its parent branch; merging it
                // would land the changes into that branch, not
                // main, while Land records success. Retarget to
                // main before merging.
                if pr_state.as_deref() != Some("MERGED")
                    && pr_field("baseRefName").as_deref() != Some("main")
                {
                    if let Err(e) = replay_run(
                        &dir,
                        "gh",
                        &replay_args(&["pr", "edit", &pr_s, "--repo", &repo, "--base", "main"]),
                    ) {
                        return transient_exit(
                            step,
                            format!("retargeting reused PR #{pr} to base main failed: {e}"),
                            &landed,
                        );
                    }
                }
                // Abandon coordination (Codex P1, PR #8849,
                // comment 4549335594): the plan executes a stack
                // snapshot taken at pass start, but an author may
                // withdraw this intent (replaying -> abandoned is a
                // legal transition) while the pass works. Merging
                // on the stale snapshot would land withdrawn
                // changes on main; the Land transition afterwards
                // would refuse, but only after the irreversible
                // step. So re-read the intent's live state
                // immediately before merging and honor a
                // withdrawal: stop the pass without merging. Any
                // state other than replaying, or a state that
                // cannot be read at all, also stops the pass --
                // never merge blind.
                let mut abandon_stop: Option<String> = None;
                if pr_state.as_deref() != Some("MERGED") {
                    let resp = call("/stack-list", &json!({"stack_id": stack_id}));
                    if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
                        abandon_stop = Some(
                            "could not re-read the stack state before merging; refusing to merge on a stale snapshot".to_string(),
                        );
                    } else {
                        let fresh_rows = resp
                            .get("row")
                            .and_then(|r| r.get("intents"))
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        let fresh_state = fresh_rows
                            .iter()
                            .filter_map(stack_intent_from_row)
                            .find(|i| i.position == step.position)
                            .map(|i| i.state);
                        match fresh_state {
                            Some(crate::merge_intent::IntentState::Replaying) => {}
                            Some(crate::merge_intent::IntentState::Abandoned) => {
                                abandon_stop = Some(format!(
                                    "intent at position {} was abandoned by its author during this replay pass; PR #{pr} was NOT merged",
                                    step.position
                                ));
                            }
                            Some(other) => {
                                abandon_stop = Some(format!(
                                    "intent at position {} is {} in the live stack state, not replaying; refusing to merge on a stale snapshot",
                                    step.position,
                                    other.as_str()
                                ));
                            }
                            None => {
                                abandon_stop = Some(format!(
                                    "intent at position {} is missing from the live stack state; refusing to merge on a stale snapshot",
                                    step.position
                                ));
                            }
                        }
                    }
                }
                if let Some(stop) = abandon_stop {
                    return transient_exit(step, stop, &landed);
                }
                if pr_state.as_deref() == Some("MERGED") {
                    // Fall through to Land: the merge already
                    // happened; recording it is the remaining step.
                } else if let Err(e) = replay_run(
                    &dir,
                    "gh",
                    &replay_args(&["pr", "merge", &pr_s, "--repo", &repo, "--merge"]),
                ) {
                    let mergeable = replay_run(
                        &dir,
                        "gh",
                        &replay_args(&[
                            "pr",
                            "view",
                            &pr_s,
                            "--repo",
                            &repo,
                            "--json",
                            "mergeable",
                        ]),
                    )
                    .ok()
                    .and_then(|out| serde_json::from_str::<Value>(&out).ok())
                    .and_then(|v| {
                        v.get("mergeable")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string())
                    });
                    if mergeable.as_deref() == Some("CONFLICTING") {
                        return conflict_exit(
                            step.position,
                            format!(
                                "PR #{pr} ({branch}) is conflicting against main at replay: {e}"
                            ),
                            &landed,
                        );
                    }
                    return transient_exit(step, e, &landed);
                }
                // Refresh main after each merge (Codex P2, PR
                // #8849, comment 4546983362): the next intent
                // rebases onto origin/main, which must already
                // contain this just-landed parent — rebasing onto
                // the pre-replay tip replays the parent's commits
                // and can conflict falsely. A fetch failure here is
                // resumable: the PR is merged, the intent is still
                // replaying, and the Merge arm's already-MERGED
                // reconciliation lands it on the next pass.
                if let Err(e) = replay_run(&dir, "git", &replay_args(&["fetch", "origin", "main"]))
                {
                    return transient_exit(
                        step,
                        format!(
                            "PR #{pr} merged but refreshing origin/main before the next intent failed: {e}"
                        ),
                        &landed,
                    );
                }
            }
            crate::merge_intent::ReplayAction::Land => {
                let pr = pr_numbers.get(&step.position).copied().flatten();
                // A refused landing is a resumable stop, not a
                // landed intent: the PR is merged, the intent stays
                // `replaying`, and the Merge arm's already-MERGED
                // reconciliation records the landing on the next
                // pass (CodeRabbit, PR #8849).
                if let Err(e) =
                    stack_transition_result(&stack_id, step.position, "landed", pr, None)
                {
                    return transient_exit(step, e, &landed);
                }
                landed.push(step.position);
                pre_rebase_tips.remove(&step.position);
                if let Some(path) = &tips_path {
                    if let Ok(text) = std::fs::read_to_string(path) {
                        if let Ok(mut saved) =
                            serde_json::from_str::<serde_json::Map<String, Value>>(&text)
                        {
                            if let Some(obj) =
                                saved.get_mut(&stack_id).and_then(|e| e.as_object_mut())
                            {
                                obj.remove(&step.position.to_string());
                            }
                            let _ = std::fs::write(
                                path,
                                serde_json::to_string(&saved).unwrap_or_default(),
                            );
                        }
                    }
                }
                // The outcome note replays in the same pass: the
                // intent's issue hears that its branch landed, in
                // the recorded order, as soon as it lands.
                if let (Some(issue), Some(pr)) = (intent.issue_number, pr) {
                    let issue_s = issue.to_string();
                    let note = format!(
                        "stack replay: position {} of stack {stack_id} landed via PR #{pr} \
                         (branch {branch}), in the recorded merge order (#8582).",
                        step.position
                    );
                    if let Err(e) = replay_run(
                        &dir,
                        "gh",
                        &replay_args(&[
                            "issue", "comment", &issue_s, "--repo", &repo, "--body", &note,
                        ]),
                    ) {
                        warnings.push(format!(
                            "position {}: outcome note on issue #{issue} failed: {e}",
                            step.position
                        ));
                    }
                }
            }
        }
    }
    let resp = unwrap_ok(call("/stack-list", &json!({"stack_id": stack_id})));
    let state = require_row(&resp, "stack_list");
    print_json(&json!({
        "stack_id": stack_id,
        "landed": landed,
        "warnings": warnings,
        "stack": state,
    }));
    0
}

pub(super) fn cmd_stack(rest: &[String]) -> i32 {
    if rest.is_empty() || is_help_arg(&rest[0]) {
        println!(
            "usage: repo-queue stack <record|list|transition|restack|replay> ...\n\nRun `repo-queue stack <sub> --help` for flags."
        );
        return if rest.is_empty() { 2 } else { 0 };
    }
    let sub = rest[0].as_str();
    let tail = &rest[1..];
    match sub {
        "record" => cmd_stack_record(tail),
        "list" => cmd_stack_list(tail),
        "transition" => cmd_stack_transition(tail),
        "restack" => cmd_stack_restack(tail),
        "replay" => cmd_stack_replay(tail),
        _ => {
            eprint(&format!(
                "repo-queue: unknown stack command {sub:?}\nusage: repo-queue stack <record|list|transition|restack|replay> ..."
            ));
            2
        }
    }
}

pub(super) fn cmd_outbox(rest: &[String]) -> i32 {
    if rest.is_empty() || is_help_arg(&rest[0]) {
        println!(
            "usage: repo-queue outbox <enqueue|list|metrics|claim|resolve|drain> ...\n\nRun `repo-queue outbox <sub> --help` for flags."
        );
        return if rest.is_empty() { 2 } else { 0 };
    }
    let sub = rest[0].as_str();
    let tail = &rest[1..];
    match sub {
        "enqueue" => cmd_outbox_enqueue(tail),
        "list" => cmd_outbox_list(tail),
        "metrics" => cmd_outbox_metrics(tail),
        "claim" => cmd_outbox_claim(tail),
        "resolve" => cmd_outbox_resolve(tail),
        "drain" => cmd_outbox_drain(tail),
        _ => {
            eprint(&format!("repo-queue: unknown outbox command {sub:?}"));
            2
        }
    }
}
