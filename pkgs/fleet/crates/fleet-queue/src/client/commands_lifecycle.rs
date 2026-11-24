//! lifecycle repo-queue verbs: claim, lane, bind, heartbeat, checkpoint, park, wake, block/unblock, dependency and quarantine rechecks, cancel, revive, fail, complete, health, signal, and the small outbox verbs (split from client.rs, #8588).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::*;

pub(super) fn cmd_claim(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "claim",
        rest,
        &[
            "queue",
            "worker",
            "lease-seconds",
            "lane",
            "task",
            "count",
            "from-session",
            "platform",
            "session-id",
            "model-provider",
            "model-id",
            "model-variant",
            "filter",
            "pool",
            "launched-by",
            "remaining-budget-minutes",
        ],
        &["handoff", "bind"],
    );
    reject_extra_pos("claim", &args, 0);
    // #8606: a zero-intent claim refuses before any broker
    // contact (see bare_claim_refusal).
    let queue_scope = opt_queue_scoped(&args);
    if let Some(msg) = bare_claim_refusal(&args, &queue_scope) {
        die(2, &msg);
    }
    let worker = opt_worker(&args);
    let lease_seconds = match lease_seconds_or_default(args.get_i64("lease-seconds")) {
        Ok(v) => v,
        Err(e) => die(2, &e),
    };
    let mut body = json!({
        "worker": worker,
        "lease_seconds": lease_seconds,
    });
    // One-queue model (#8097, 2026-11-10): a claim with no
    // --queue (and no REPO_QUEUE_DEFAULT_QUEUE in the env) is
    // UNSCOPED — the broker claims across the one fleet queue
    // in priority order. An explicit --queue still scopes the
    // claim to one stream exactly as before, so existing shift
    // bodies keep working unchanged.
    if let Some(q) = queue_scope {
        body["queue"] = json!(q);
    }
    // --filter '<expr>': the claimer-side boolean filter
    // expression over task metadata (repo, queue, pool,
    // platform, label, kind, priority, lane, model), evaluated
    // by the broker inside the claim. Absent: no filtering.
    if let Some(f) = args.get("filter") {
        body["filter"] = json!(f);
    }
    // --pool <name>: the claimer's declared pool identity,
    // recorded like the other claim marks and used only to
    // satisfy a task's own requires.pool.
    if let Some(p) = args.get("pool") {
        body["pool"] = json!(p);
    }
    // --remaining-budget-minutes <n>: the claimer's remaining
    // shift budget (#3418). The broker only matches
    // requirement-bearing tasks whose required budget fits it;
    // omitting it leaves requirement-bearing tasks unclaimed.
    if let Some(v) = args.get_i64("remaining-budget-minutes") {
        body["remaining_budget_minutes"] = json!(v);
    }
    if let Some(lane) = args.get("lane") {
        body["lane"] = json!(lane);
    }
    // --platform <family>: declare this claim's platform MARK
    // (#8097 Rule 2, demoted by #8187). The broker stamps it on
    // the claimed row and the claimed event; it filters NOTHING —
    // a mismatched or undeclared claim claims items all the same.
    if let Some(p) = args.get("platform") {
        match platform_flag(&p) {
            Ok(v) => body["platform"] = json!(v),
            Err(e) => die(2, &e),
        }
    }
    // --session-id: the claiming session's own id, recorded as a
    // mark on the claimed event when the surface knows it (#8187).
    if let Some(sid) = args.get("session-id") {
        body["session_id"] = json!(sid);
    }
    // --model-provider/--model-id[/--model-variant]: the claiming
    // session's own model, stamped on the task by the broker
    // (#8176). The session reads its serving leaf from its own
    // session surface (route_usage lastServing, opencode#1891)
    // and passes it here; the broker stores it as the row's
    // claim_provider / claim_model / claim_variant.
    match model_stamp_flags(
        args.get("model-provider").as_deref(),
        args.get("model-id").as_deref(),
        args.get("model-variant").as_deref(),
    ) {
        Ok(Some((provider, model, variant))) => {
            body["model_provider"] = json!(provider);
            body["model_id"] = json!(model);
            if let Some(v) = variant {
                body["model_variant"] = json!(v);
            }
        }
        Ok(None) => {}
        Err(e) => die(2, &e),
    }
    // #8176 (2026-11-12 fix): no explicit stamp — derive it from
    // the claiming session's own record when the claim names the
    // session (--session-id, the dispatch surfaces' form). Claims
    // that name no session keep the unstamped shape exactly as
    // before; derivation failure never fails the claim.
    if body.get("model_provider").is_none() {
        if let Some(sid) = args.get("session-id") {
            if let Some((provider, model, variant)) = derive_claim_model_stamp(&sid) {
                body["model_provider"] = json!(provider);
                body["model_id"] = json!(model);
                if let Some(v) = variant {
                    body["model_variant"] = json!(v);
                }
            }
        }
    }
    // --handoff: the no-id collection of the next recorded
    // opencode handoff from the sink (status 'handed_off', #8234;
    // #8097 Rule 1) — the claimant's default handoff path. The
    // BROKER picks the row; the caller names no task id. (--task
    // remains the special-case claim.)
    if args.has("handoff") {
        body["pick"] = json!("handed_off");
    }
    // --task <id>: by-id claim — take exactly that row when the
    // broker's by-id predicate allows it (a queued row under the
    // ordinary predicate, or a sink row). The opencode claimant
    // uses this to collect handed-off tasks out of the sink; a
    // plain claim never names a task.
    if let Some(task) = args.get_i64("task") {
        body["task_id"] = json!(task);
    }
    // --count <n> (#8217): batch claim — the broker runs the same
    // atomic pick n times and answers with every claimed row. The
    // cap is enforced here (local, instant) and broker-side.
    let count = match batch_count_flag(args.get_i64("count")) {
        Ok(c) => c,
        Err(e) => die(2, &e),
    };
    if count > 1 {
        body["count"] = json!(count);
    }
    // --from-session <ses>: with --task, a session-to-session
    // TRANSFER (#8210) — claim exactly this task DIRECTLY from
    // the session named, which must be the task's current holder
    // (its session mark is the capability; a mismatch refuses and
    // leaves the claim untouched). The task never passes through
    // queued or parked. --session-id on the same claim binds the
    // claiming session in the same operation.
    if let Some(fs) = args.get("from-session") {
        body["from_session"] = json!(fs);
    }
    // --bind (#5757-A): claim + lane-assign + session-bind in
    // ONE broker round trip via /claim-flow instead of the three
    // trips claim -> lane -> bind. The answer carries the same
    // task row in `row`, so claim_output and the #6741 fallback
    // guard below work unchanged. The flow goes through call_fb
    // like /claim: on a backend failure the fallback switch
    // fires identically, and lane/bind — broker-ledger steps —
    // are skipped in fallback mode (runbook §0a: no ledger
    // there), so there is no client-side three-verb replay.
    let bind = match claim_bind_path(
        args.has("bind"),
        count,
        args.get("from-session").is_some(),
        args.get("session-id").as_deref(),
    ) {
        Ok(bind) => bind,
        Err(e) => die(2, &e),
    };
    if bind {
        body["launched_by"] = json!(
            args.get("launched-by")
                .unwrap_or_else(|| "opencode-claimant".to_string())
        );
    }
    // #9046: the effective stamp + session for the timeline
    // appends below (explicit flags or session-derived — by now
    // both live on the request body).
    let tl_stamp = body_stamp(&body);
    let tl_session = args.get("session-id");
    let tl_trigger = if args.get("from-session").is_some() {
        "transfer"
    } else {
        "claim"
    };
    let resp = if bind {
        unwrap_ok_fb(call_fb("/claim-flow", &body), "/claim-flow")
    } else {
        unwrap_ok_fb(call_fb("/claim", &body), "/claim")
    };
    if let Some(note) = claim_flow_partial_note(&resp) {
        eprint(&note);
    }
    if count > 1 {
        // #9046: one binding + stamp timeline record per row.
        if let Some(rows) = resp.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                claim_timeline_appends(&worker, row, &tl_stamp, &tl_session, tl_trigger);
            }
        }
        return claim_batch_output(&resp, &worker, count);
    }
    // #9046: mirror the claim's binding + stamp into the
    // timeline (a no-op when the broker stamped in-op).
    if let Some(row) = resp.get("row") {
        if !row.is_null() {
            // A transfer closes the previous session's window
            // first: the CLI knows the from-session even when a
            // pre-switch broker recorded no binding to clear (its
            // claims never set progress.session_id). Same
            // capability signal as claim_timeline_appends — a
            // post-switch broker writes this unbound in-op via
            // on_binding_cleared, and its rows carry the key.
            if tl_trigger == "transfer" && row.get("claim_session_id").is_none() {
                if let (Some(fs), Some(tid)) = (
                    args.get("from-session"),
                    row.get("id").and_then(|v| v.as_i64()),
                ) {
                    append_timeline_event(
                        &worker,
                        tid,
                        "session_unbound",
                        json!({
                            "session_id": fs,
                            "trigger": "transfer",
                            "via": "fleet-cli",
                        }),
                    );
                }
            }
            claim_timeline_appends(&worker, row, &tl_stamp, &tl_session, tl_trigger);
        }
    }
    let (mut out, code) = claim_output(&resp);
    if code != 0 {
        // #8154: name why when the envelope carries the counts.
        eprint(
            &no_claim_diagnostic(&resp)
                .unwrap_or_else(|| "repo-queue: no claimable task".to_string()),
        );
        attach_mode_line(&mut out, "claim", legs_served(true, true));
        print_json(&out);
        return code;
    }
    // No-double-serve guard (#6741): a task whose issue is held in
    // the GitHub fallback right now must not be served from the
    // primary store — block it back and report no claim.
    match fallback_guard(&out, &worker) {
        GuardOutcome::HeldBy(holder) => {
            let task_id = out.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let (repo, issue) = task_issue(&out);
            let blocker = format!(
                "issue {repo}#{issue} is held in the GitHub fallback by {holder}; \
                 primary-store claim auto-blocked to prevent double-serve (#6741)"
            );
            let resp = call(
                "/block",
                &json!({
                    "task_id": task_id,
                    "worker": worker,
                    "reason_class": "fallback_held",
                    "blocker": blocker,
                    "progress": {},
                }),
            );
            if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
                eprint(
                    "repo-queue: WARNING: fallback guard block failed;                      the task stays claimed under its lease",
                );
            }
            let mut held_out = json!({
                "task": Value::Null,
                "guard": "fallback_held",
                "held_by": holder,
                "repo": repo,
                "issue": issue,
            });
            attach_mode_line(&mut held_out, "claim", legs_served(true, true));
            print_json(&held_out);
            NO_CLAIM_EXIT
        }
        GuardOutcome::Unchecked(e) => {
            if let Some(o) = out.as_object_mut() {
                o.insert(
                    "guard_warning".to_string(),
                    json!(format!(
                        "fallback double-serve guard could not check GitHub: {e}"
                    )),
                );
            }
            attach_mode_line(&mut out, "claim", legs_served(true, false));
            print_json(&out);
            code
        }
        GuardOutcome::Serve => {
            attach_mode_line(&mut out, "claim", legs_served(true, true));
            print_json(&out);
            code
        }
    }
}

/// The (repo, issue) identity a claimed task's payload carries, for
/// the fallback guard. ("", 0) when the packet names none.
pub(super) fn task_issue(row: &Value) -> (String, i64) {
    let payload = row.get("payload").cloned().unwrap_or(Value::Null);
    (
        payload
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        payload
            .get("issue")
            .and_then(|i| i.get("number"))
            .and_then(|n| n.as_i64())
            .unwrap_or(0),
    )
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum GuardDecision {
    Serve,
    HeldBy(String),
}

/// Pure guard decision over a fallback claim-state object (the
/// shape `fallback status` prints): live or contested in the
/// fallback under a DIFFERENT worker means hands off; anything
/// else (free / released / stale / held by me) may be served.
pub(super) fn guard_decision(state: &Value, my_worker: &str) -> GuardDecision {
    let name = state.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let holder = state.get("worker").and_then(|v| v.as_str()).unwrap_or("");
    if (name == "live" || name == "contested") && !holder.is_empty() && holder != my_worker {
        GuardDecision::HeldBy(holder.to_string())
    } else {
        GuardDecision::Serve
    }
}

pub(super) enum GuardOutcome {
    Serve,
    HeldBy(String),
    /// The GitHub check itself failed: serve, but say so — the
    /// guard fails open with a warning rather than stalling every
    /// claim on a GitHub outage (the primary store failing is what
    /// the fallback switch above is for).
    Unchecked(String),
}

/// #8217: --count validation for the batch claim. Absent or 1 is
/// the ordinary single claim; 2..=BATCH_CLAIM_MAX is a batch;
/// anything else is a usage error, checked locally so the failure
/// is instant (the broker enforces the same bound).
pub(super) fn batch_count_flag(n: Option<i64>) -> Result<i64, String> {
    match n {
        None => Ok(1),
        Some(n) if n == 1 => Ok(1),
        Some(n) if (2..=crate::BATCH_CLAIM_MAX).contains(&n) => Ok(n),
        Some(n) => Err(format!(
            "claim --count {n} is out of range: a batch claim takes \
             1..={} items (#8217)",
            crate::BATCH_CLAIM_MAX
        )),
    }
}

/// The batch claim's response handling (#8217): the broker ran the
/// pick repeatedly and returned every claimed row. Each row goes
/// through the SAME per-item post-claim handling as a single
/// claim — the payload defense in depth and the #6741
/// no-double-serve guard (a fallback-held row is blocked back,
/// never served) — then prints one verdict:
/// {"tasks": [...], "claimed": k, "requested": n, "guard_held": [...]}.
/// Zero served rows is the usual no-claim verdict (NO_CLAIM_EXIT).
pub(super) fn claim_batch_output(resp: &Value, worker: &str, requested: i64) -> i32 {
    let rows = resp
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut tasks: Vec<Value> = Vec::new();
    let mut held: Vec<Value> = Vec::new();
    // #8581: the batch's gh-read leg is its worst guard outcome —
    // one unchecked guard read degrades the batch's mode line.
    let mut github_read = true;
    for row in rows {
        // Payload defense in depth, as in claim_output (#5444): a
        // well-formed claim never carries a null or {} payload.
        if let Some(payload) = row.get("payload") {
            if payload.is_null() || payload.as_object().is_some_and(|o| o.is_empty()) {
                continue;
            }
        }
        match fallback_guard(&row, worker) {
            GuardOutcome::HeldBy(holder) => {
                let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let (repo, issue) = task_issue(&row);
                let blocker = format!(
                    "issue {repo}#{issue} is held in the GitHub fallback by {holder}; \
                     primary-store claim auto-blocked to prevent double-serve (#6741)"
                );
                let resp = call(
                    "/block",
                    &json!({
                        "task_id": task_id,
                        "worker": worker,
                        "reason_class": "fallback_held",
                        "blocker": blocker,
                        "progress": {},
                    }),
                );
                if resp.get("ok").and_then(|o| o.as_bool()) == Some(false) {
                    eprint(
                        "repo-queue: WARNING: fallback guard block failed; \
                         the task stays claimed under its lease",
                    );
                }
                held.push(json!({
                    "task": task_id,
                    "held_by": holder,
                    "repo": repo,
                    "issue": issue,
                }));
            }
            GuardOutcome::Unchecked(e) => {
                github_read = false;
                let mut row = row;
                if let Some(o) = row.as_object_mut() {
                    o.insert(
                        "guard_warning".to_string(),
                        json!(format!(
                            "fallback double-serve guard could not check GitHub: {e}"
                        )),
                    );
                }
                tasks.push(row);
            }
            GuardOutcome::Serve => tasks.push(row),
        }
    }
    let claimed = tasks.len();
    let mut out = json!({
        "tasks": tasks,
        "claimed": claimed,
        "requested": requested,
        "guard_held": held,
    });
    // #8154: an empty batch carries the no-claim counts too.
    if claimed == 0 {
        if let Some(obj) = out.as_object_mut() {
            for k in [
                "queued",
                "at_cap_queued",
                "queued_not_yet_due",
                "parked_due",
            ] {
                if let Some(v) = resp.get(k) {
                    obj.insert(k.to_string(), v.clone());
                }
            }
        }
    }
    attach_mode_line(&mut out, "claim", legs_served(true, github_read));
    print_json(&out);
    if claimed == 0 {
        eprint(
            &no_claim_diagnostic(resp)
                .unwrap_or_else(|| "repo-queue: no claimable task".to_string()),
        );
        return NO_CLAIM_EXIT;
    }
    0
}

/// Release a claimed task straight back to the queue (#8217):
/// `release <task_id> --worker <w> --reason <text>`. The broker
/// returns the row to `queued`, immediately claimable, and records
/// the reason on a `released` event. Dispatch uses it for an item
/// whose sub-task failed to launch or bind: the item goes back to
/// the pool in the same touch instead of sitting claimed and idle
/// until its lease expires.
pub(super) fn cmd_release(rest: &[String]) -> i32 {
    let args = parse_verb_args("release", rest, &["worker", "lane", "reason"], &[]);
    reject_extra_pos("release", &args, 1);
    let Some(reason) = args.get("reason") else {
        die(
            2,
            "release requires --reason \"<why this task goes back to the pool>\" (#8217)",
        );
    };
    if reason.trim().is_empty() {
        die(2, "release requires a non-empty --reason (#8217)");
    }
    let resp = unwrap_ok(call(
        "/release",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": opt_worker(&args),
            "lane": lane_arg(&args),
            "reason": reason,
        }),
    ));
    let row = served_row(require_row(&resp, "release_repo_task"), "release");
    // #9046: a release closes the binding window on the timeline.
    unbound_append(&opt_worker(&args), &row, "release");
    print_json(&row);
    0
}

pub(super) fn fallback_guard(row: &Value, my_worker: &str) -> GuardOutcome {
    let (repo, issue) = task_issue(row);
    if repo.is_empty() || issue <= 0 {
        return GuardOutcome::Serve;
    }
    match crate::fallback::issue_claim_state(&repo, issue) {
        Ok(state) => match guard_decision(&state, my_worker) {
            GuardDecision::HeldBy(h) => GuardOutcome::HeldBy(h),
            GuardDecision::Serve => GuardOutcome::Serve,
        },
        Err(e) => GuardOutcome::Unchecked(e),
    }
}

/// Exit code for a claim that found no task: nonzero, so an empty
/// claim can never be mistaken for a successful one (#5486).
pub(super) const NO_CLAIM_EXIT: i32 = 5;

/// Decode a `/claim` broker envelope into the (stdout value, exit
/// code) pair `cmd_claim` emits. Pure so the wire/output contract is
/// unit-testable without a broker. The oracle is the envelope shape
/// itself (#5486): `{ok:true, row:{id,queue,status,dedupe_key,...}}`
/// prints the row VERBATIM with exit 0 — the reported failure was an
/// all-null row printed instead of the claimed task — while a null
/// (or missing) row yields the distinct `{"task": null}` marker with
/// NO_CLAIM_EXIT, a value carrying no id/queue/status/dedupe_key
/// fields a caller could mistake for a task.
/// #8154 (follow-on #2937 acceptance 4): the no-claim diagnostic.
/// When the broker's no-row envelope carries the scoped counts,
/// name why the claim found nothing — most importantly the attempt
/// cap, the case that otherwise reads as a mysteriously empty
/// queue. Returns None when the envelope has no counts (an older
/// broker), so callers fall back to the bare line.
pub(super) fn no_claim_diagnostic(resp: &Value) -> Option<String> {
    let queued = resp.get("queued")?.as_i64()?;
    let at_cap = resp
        .get("at_cap_queued")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let not_due = resp
        .get("queued_not_yet_due")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let parked_due = resp.get("parked_due").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut line = format!(
        "repo-queue: no claimable task — {queued} queued ({at_cap} at the attempt cap, \
         {not_due} not yet due), {parked_due} parked due (#8154)"
    );
    if queued > 0 && at_cap >= queued {
        line.push_str("; every queued task is at the attempt cap");
    }
    Some(line)
}

pub(super) fn claim_output(resp: &Value) -> (Value, i32) {
    let row = resp.get("row").cloned().unwrap_or(Value::Null);
    if row.is_null() {
        // #3418 (#4712): carry the broker's budget-skip summary on
        // the no-claim output so lanes see impossible-fit work
        // instead of an empty claim.
        let mut out = json!({"task": Value::Null});
        if let Some(obj) = out.as_object_mut() {
            for k in [
                "budget_skipped",
                "max_required_budget_minutes",
                // #8154: the no-claim counts ride the printed
                // envelope next to the budget summary.
                "queued",
                "at_cap_queued",
                "queued_not_yet_due",
                "parked_due",
            ] {
                if let Some(v) = resp.get(k) {
                    obj.insert(k.to_string(), v.clone());
                }
            }
        }
        if resp
            .get("budget_skipped")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0
        {
            eprint(&format!(
                "repo-queue: claim skipped {} task(s) on budget (#3418); largest requirement: {} min",
                resp.get("budget_skipped").cloned().unwrap_or(Value::Null),
                resp.get("max_required_budget_minutes")
                    .cloned()
                    .unwrap_or(Value::Null),
            ));
        }
        return (out, NO_CLAIM_EXIT);
    }
    // Defense in depth (#5444): the broker's claim path validates
    // packets before handing them out, but a row that still arrives
    // carrying a payload field that is null or an empty object is a
    // payload-less packet — never print it as a claimed task. Rows
    // without a payload field keep the #5486 verbatim contract.
    if let Some(payload) = row.get("payload") {
        let missing = payload.is_null() || payload.as_object().is_some_and(|o| o.is_empty());
        if missing {
            eprint(&format!(
                "repo-queue: claim skipped: task {} arrived payload-less (#5444); treating as no claim",
                row.get("id").cloned().unwrap_or(Value::Null)
            ));
            return (json!({"task": Value::Null}), NO_CLAIM_EXIT);
        }
    }
    // #5757-A (Codex P2, PR #8493): a /claim-flow partial success
    // must stay distinguishable in the machine-readable output —
    // the stderr note alone leaves stdout consumers unable to tell
    // a completed bind from an unbound claim. Carry the broker's
    // claim_flow_partial marker on the printed row, alongside the
    // claimed task's identity; the claim itself is not retried.
    let mut row = row;
    if let (Some(partial), Some(obj)) = (resp.get("claim_flow_partial"), row.as_object_mut()) {
        obj.insert("claim_flow_partial".to_string(), partial.clone());
    }
    (row, 0)
}

pub(super) fn cmd_lane(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "lane",
        rest,
        &[
            "worker",
            "exclusion-reason",
            "opencode-reason",
            "limit-platform",
            "limit-reason",
        ],
        &["clear-platform-limit"],
    );
    reject_extra_pos("lane", &args, 1);
    let resp = unwrap_ok(call(
        "/lane",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": opt_worker(&args),
            "exclusion_reason": args.get("exclusion-reason"),
            "opencode_reason": args.get("opencode-reason"),
            // #8210: a platform limitation — an exception that
            // exists only with its recorded reason. The broker
            // refuses a limitation with no --limit-reason.
            "platform_limit": args.get("limit-platform"),
            "platform_limit_reason": args.get("limit-reason"),
            "clear_platform_limit": args.has("clear-platform-limit"),
        }),
    ));
    print_json(&served_row(
        require_row(&resp, "assign_repo_task_lane"),
        "lane",
    ));
    0
}

pub(super) fn cmd_bind(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "bind",
        rest,
        &[
            "worker",
            "session-id",
            "launched-by",
            "model-provider",
            "model-id",
            "model-variant",
        ],
        &[],
    );
    reject_extra_pos("bind", &args, 1);
    let session_id = args
        .get("session-id")
        .unwrap_or_else(|| die(2, "bind: --session-id is required"));
    // #9046: the bind stamps the bound session's own model —
    // explicit --model-* flags win; otherwise the stamp derives
    // from the session's own record, exactly as on claim (#8176).
    // The stamp rides the /bind body (a post-#9046 broker stamps
    // in-op) and the timeline appends below (the live path on a
    // pre-switch broker; the reader collapses the duplicate).
    let stamp = match model_stamp_flags(
        args.get("model-provider").as_deref(),
        args.get("model-id").as_deref(),
        args.get("model-variant").as_deref(),
    ) {
        Ok(Some(s)) => Some(s),
        Ok(None) => derive_claim_model_stamp(&session_id),
        Err(e) => die(2, &e),
    };
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "session_id": session_id,
        "launched_by": args.get("launched-by").unwrap_or_else(|| "opencode-claimant".to_string()),
    });
    if let Some((provider, model, variant)) = &stamp {
        body["model_provider"] = json!(provider);
        body["model_id"] = json!(model);
        if let Some(v) = variant {
            body["model_variant"] = json!(v);
        }
    }
    let resp = unwrap_ok(call("/bind", &body));
    let task_id = args.pos_i64(0, "task_id");
    let worker = opt_worker(&args);
    append_timeline_event(
        &worker,
        task_id,
        "session_bound",
        json!({"session_id": session_id, "trigger": "bind", "via": "fleet-cli"}),
    );
    if let Some((provider, model, variant)) = &stamp {
        append_timeline_event(
            &worker,
            task_id,
            "model_stamp",
            json!({
                "session_id": session_id,
                "provider": provider,
                "model": model,
                "variant": variant,
                "trigger": "bind",
                "via": "fleet-cli",
            }),
        );
    }
    print_json(&served_row(
        require_row(&resp, "bind_repo_task_lane_session"),
        "bind",
    ));
    0
}

pub(super) fn cmd_heartbeat(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "heartbeat",
        rest,
        &[
            "worker",
            "lane",
            "extend-seconds",
            "progress",
            "progress-file",
        ],
        &[],
    );
    reject_extra_pos("heartbeat", &args, 1);
    let extend_seconds = match extend_seconds_or_default(args.get_i64("extend-seconds")) {
        Ok(v) => v,
        Err(e) => die(2, &e),
    };
    let resp = unwrap_ok_fb(
        call_fb(
            "/heartbeat",
            &json!({
                "task_id": args.pos_i64(0, "task_id"),
                "worker": opt_worker(&args),
                "lane": lane_arg(&args),
                "extend_seconds": extend_seconds,
                "progress": progress_arg(&args),
            }),
        ),
        "/heartbeat",
    );
    let mut row = require_row(&resp, "heartbeat_task");
    attach_mode_line(&mut row, "heartbeat", legs_served(true, true));
    print_json(&row);
    0
}

pub(super) fn cmd_checkpoint(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "checkpoint",
        rest,
        &["worker", "lane", "stage", "progress", "progress-file"],
        &[],
    );
    reject_extra_pos("checkpoint", &args, 1);
    let stage = args
        .get("stage")
        .unwrap_or_else(|| die(2, "checkpoint: --stage is required"));
    let resp = unwrap_ok_fb(
        call_fb(
            "/checkpoint",
            &json!({
                "task_id": args.pos_i64(0, "task_id"),
                "worker": opt_worker(&args),
                "lane": lane_arg(&args),
                "stage": stage,
                "progress": progress_arg(&args).unwrap_or(json!({})),
            }),
        ),
        "/checkpoint",
    );
    let mut row = require_row(&resp, "checkpoint_task");
    attach_mode_line(&mut row, "checkpoint", legs_served(true, true));
    print_json(&row);
    0
}

pub(super) fn cmd_park(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "park",
        rest,
        &[
            "worker",
            "lane",
            "reason",
            "delay-seconds",
            "not-before",
            "progress",
            "progress-file",
            "ready-condition",
            "execution",
            "required-budget-minutes",
            // FDR-0013: a park may wait on a person only as a hands
            // item, named by these two flags.
            "hands",
            "action",
        ],
        &[],
    );
    reject_extra_pos("park", &args, 1);
    if args.has("delay-seconds") && args.has("not-before") {
        die(
            2,
            "argument --not-before: not allowed with argument --delay-seconds",
        );
    }
    let reason = args
        .get("reason")
        .unwrap_or_else(|| die(2, "park: --reason is required"));
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "lane": lane_arg(&args),
        "reason": reason,
        "progress": progress_arg(&args).unwrap_or(json!({})),
    });
    if let Some(nb) = args.get("not-before") {
        let nb = match validate_not_before_flag(&nb) {
            Ok(v) => v,
            Err(e) => die(2, &e),
        };
        body["not_before"] = json!(nb);
    } else if let Some(d) = args.get_i64("delay-seconds") {
        let d = match validate_delay_seconds(d) {
            Ok(v) => v,
            Err(e) => die(2, &e),
        };
        body["delay_seconds"] = json!(d);
    }
    if let Some(rc_raw) = args.get("ready-condition") {
        let rc: Value = match serde_json::from_str(&rc_raw) {
            Ok(v) => v,
            Err(e) => die(2, &format!("--ready-condition is not valid JSON: {e}")),
        };
        if !rc.is_object() {
            die(2, "--ready-condition must be a JSON object");
        }
        body["ready_condition"] = rc;
    }
    // FDR-0013: no park waits on a person to decide. Checked before
    // any broker contact; a hands item is stamped onto the progress so
    // the row says what a person has to do.
    let condition_kind = body["ready_condition"]
        .get("kind")
        .and_then(|k| k.as_str())
        .map(str::to_string);
    let hands_flag = args.get("hands");
    let action_flag = args.get("action");
    match crate::person_wait::check(&crate::person_wait::Wait {
        reason_class: None,
        condition_kind: condition_kind.as_deref(),
        text: &reason,
        hands: hands_flag.as_deref(),
        action: action_flag.as_deref(),
    }) {
        Ok(Some(hands)) => {
            body["progress"]["hands"] = json!({"kind": hands.kind, "action": hands.action});
        }
        Ok(None) => {}
        Err(message) => die(2, &format!("park: {message}")),
    }
    // --required-budget-minutes <n>: the minimum remaining shift
    // budget a future claimer must declare to take this park
    // (#3418). For a capacity wait the broker defaults it from
    // the packet's timebox when the flag is absent.
    if let Some(v) = args.get_i64("required-budget-minutes") {
        body["required_budget_minutes"] = json!(v);
    }
    if let Some(ex) = execution_arg(&args) {
        body["execution"] = ex;
    }
    let resp = unwrap_ok(call("/park", &body));
    // #9046: a park closes the binding window on the timeline.
    if let Some(row) = resp.get("row") {
        unbound_append(&opt_worker(&args), row, "park");
    }
    row_or_ledger(&resp, "park_task", "park");
    0
}

/// wake (#8184): release a broker-held parked wait now that
/// its condition is satisfied (e.g. the triage step finished
/// triaging the item). The broker flips it back to queued.
pub(super) fn cmd_wake(rest: &[String]) -> i32 {
    let args = parse_verb_args("wake", rest, &["worker", "reason"], &[]);
    reject_extra_pos("wake", &args, 1);
    let body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": args.get("reason").unwrap_or_default(),
    });
    let resp = unwrap_ok(call("/wake", &body));
    print_json(&served_row(require_row(&resp, "wake_task"), "wake"));
    0
}

/// #8568: resolve the PM aggregator's alerts-intake token by
/// name, through the same in-process route the fleet binary's
/// `fleet secrets get` verb uses — fleet-secrets'
/// `Resolver::default_chain` over the loaded fleet config (the
/// direct secrets-broker source on the laptop, the session-proxy
/// broker source in the sandbox, the `bws` CLI as fallback). The
/// queue client has no other named-secret resolution of its own;
/// this is that machinery, reused in-process (no shelling out).
/// The token value is used for the one POST and dropped — never
/// printed, never filed.
pub(super) fn resolve_alerts_intake_token() -> Result<String, String> {
    let config = fleet_config::Config::load().unwrap_or_default();
    fleet_secrets::Resolver::default_chain(&config)
        .resolve(crate::needs_drew::ALERTS_INTAKE_TOKEN_SECRET)
        .map(|v| v.expose().to_string())
        .map_err(|e| e.to_string())
}

/// #8568: parse cmd_block's --options-json flag: a JSON array of
/// {"id": ..., "label": ...} objects. Bad shapes are usage errors
/// at the caller's terminal, like --progress.
pub(super) fn block_options_arg(args: &Args) -> Vec<crate::needs_drew::NeedsDrewOption> {
    let Some(raw) = args.get("options-json") else {
        return Vec::new();
    };
    let val: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => die(2, &format!("block: --options-json is not valid JSON: {e}")),
    };
    let Some(arr) = val.as_array() else {
        die(2, "block: --options-json must be a JSON array");
    };
    arr.iter()
        .map(|o| {
            let id = o.get("id").and_then(|v| v.as_str());
            let label = o.get("label").and_then(|v| v.as_str());
            match (id, label) {
                (Some(id), Some(label)) => crate::needs_drew::NeedsDrewOption {
                    id: id.to_string(),
                    label: label.to_string(),
                },
                _ => die(
                    2,
                    "block: --options-json entries must be {\"id\": ..., \"label\": ...} objects",
                ),
            }
        })
        .collect()
}

/// #8568: parse cmd_block's --links-json flag: a JSON array of
/// strings.
pub(super) fn block_links_arg(args: &Args) -> Vec<String> {
    let Some(raw) = args.get("links-json") else {
        return Vec::new();
    };
    let val: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => die(2, &format!("block: --links-json is not valid JSON: {e}")),
    };
    let Some(arr) = val.as_array() else {
        die(2, "block: --links-json must be a JSON array");
    };
    arr.iter()
        .map(|l| match l.as_str() {
            Some(s) => s.to_string(),
            None => die(2, "block: --links-json entries must be strings"),
        })
        .collect()
}

pub(super) fn cmd_block(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "block",
        rest,
        &[
            "worker",
            "lane",
            "reason-class",
            "blocker",
            "progress",
            "progress-file",
            // #8568: needs-drew payload for an escalated-to-user
            // block (PM-stack notifications Phase 1). All optional;
            // without --ledger-id + --question an escalated block
            // behaves exactly as before.
            "ledger-id",
            "question",
            "context",
            "options-json",
            "links-json",
            // #8276: typed dependency blocks — the broker watches
            // these and releases the task itself when they clear
            // (or when the max TTL expires). Comma-separated for
            // several deps of one kind.
            "blocked-by-pr",
            "blocked-by-issue",
            "blocked-by-task",
            "blocked-by-ci",
            "dep-recheck-seconds",
            "dep-max-ttl-seconds",
            // FDR-0013: a block may wait on a person only as a hands
            // item, named by these two flags.
            "hands",
            "action",
        ],
        &[],
    );
    reject_extra_pos("block", &args, 1);
    let rc = args
        .get("reason-class")
        .unwrap_or_else(|| die(2, "block: --reason-class is required"));
    let blocker = args
        .get("blocker")
        .unwrap_or_else(|| die(2, "block: --blocker is required"));
    // FDR-0013: no block waits on a person to decide. Checked before
    // any broker contact, so a refused block changes nothing.
    let hands_flag = args.get("hands");
    let action_flag = args.get("action");
    let hands = match crate::person_wait::check(&crate::person_wait::Wait {
        reason_class: Some(&rc),
        condition_kind: None,
        text: &blocker,
        hands: hands_flag.as_deref(),
        action: action_flag.as_deref(),
    }) {
        Ok(hands) => hands,
        Err(message) => die(2, &format!("block: {message}")),
    };
    // #8568: the needs-drew payload is live only for an
    // escalated-to-user block that names its ledger item and its
    // question; every other block keeps today's shape exactly. A
    // hands item's action is the question when none was given, so the
    // push says what to do.
    let question = args
        .get("question")
        .or_else(|| hands.as_ref().map(|h| h.action.clone()));
    let needs_drew = if rc == "escalated-to-user" {
        match (args.get("ledger-id"), question) {
            (Some(ledger_id), Some(question)) => Some(crate::needs_drew::BlockNeedsDrew {
                ledger_id,
                question,
                context: args.get("context"),
                options: block_options_arg(&args),
                links: block_links_arg(&args),
                blocker: blocker.clone(),
            }),
            _ => None,
        }
    } else {
        None
    };
    let mut progress = progress_arg(&args).unwrap_or(json!({}));
    if let Some(hands) = &hands {
        progress["hands"] = json!({"kind": hands.kind, "action": hands.action});
    }
    if let Some(nd) = &needs_drew {
        // Stamp the ledger id onto the task row through the
        // progress the CURRENT broker already persists — no
        // broker change needed for the ledger-id half. Any
        // caller-supplied progress content is preserved; only a
        // caller-set needs_drew.ledger_id of their own would be
        // overwritten, and the flag is the authoritative source.
        let mut stamp = json!({"ledger_id": nd.ledger_id});
        if let Some(existing) = progress.get("needs_drew").and_then(|v| v.as_object()) {
            for (k, v) in existing {
                stamp[k] = v.clone();
            }
            stamp["ledger_id"] = json!(nd.ledger_id);
        }
        progress["needs_drew"] = stamp;
    }
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "lane": lane_arg(&args),
        "reason_class": rc,
        "blocker": blocker,
        "progress": progress,
    });
    if let Some(nd) = &needs_drew {
        // Snake_case keys, matching the /block body's own style;
        // the broker (and, later, the broker-side raise) reads
        // this shape back via needs_drew::from_block_request.
        let mut nd_json = json!({
            "ledger_id": nd.ledger_id,
            "question": nd.question,
            "options": nd
                .options
                .iter()
                .map(|o| json!({"id": o.id, "label": o.label}))
                .collect::<Vec<_>>(),
            "links": nd.links,
        });
        if let Some(context) = &nd.context {
            nd_json["context"] = json!(context);
        }
        body["needs_drew"] = nd_json;
    }
    let deps = dep_flags(&args);
    if !deps.is_empty() {
        body["deps"] = json!(deps);
    }
    if let Some(s) = args.get_i64("dep-recheck-seconds") {
        body["dep_recheck_seconds"] = json!(s);
    }
    if let Some(s) = args.get_i64("dep-max-ttl-seconds") {
        body["dep_max_ttl_seconds"] = json!(s);
    }
    let resp = unwrap_ok(call("/block", &body));
    print_json(&served_row(require_row(&resp, "block_task"), "block"));
    // #8568: the live raise path — after the block lands, post
    // the needs-drew alert to the PM throat. Emission failure
    // must NOT fail the block (the block already succeeded and
    // the throat dedupes, so a later retry / the broker-side
    // raise can still land it): warn on stderr, exit 0. Stdout
    // above keeps its current JSON shape exactly.
    if let Some(nd) = &needs_drew {
        let raised = resolve_alerts_intake_token()
            .and_then(|token| crate::needs_drew::post_needs_drew(&token, nd));
        match raised {
            Ok(_) => eprint(&format!(
                "repo-queue: needs-drew raised for ledger item {}",
                nd.ledger_id
            )),
            Err(e) => eprint(&format!(
                "repo-queue: WARNING: needs-drew throat post failed: {e}"
            )),
        }
    }
    0
}

/// #8276: parse the --blocked-by-* flags into the broker's deps
/// array. Refs are owner/repo#N for pr/issue/ci (the number is a
/// PR/issue number or an Actions run id) and bare ids for tasks;
/// comma-separated lists allow several deps of one kind. Bad
/// refs die here, at the caller's terminal — never as a prose
/// block the broker cannot watch.
pub(super) fn dep_flags(args: &Args) -> Vec<Value> {
    let mut deps = Vec::new();
    for (flag, kind) in [
        ("blocked-by-pr", "pr"),
        ("blocked-by-issue", "issue"),
        ("blocked-by-ci", "ci"),
    ] {
        if let Some(v) = args.get(flag) {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let Some((repo, num)) = item.rsplit_once('#') else {
                    die(
                        2,
                        &format!("block: --{flag} expects owner/repo#N, got '{item}'"),
                    );
                };
                let n: i64 = num.parse().unwrap_or_else(|_| {
                    die(
                        2,
                        &format!("block: --{flag} expects owner/repo#N, got '{item}'"),
                    )
                });
                if kind == "ci" {
                    deps.push(json!({"kind": kind, "repo": repo, "run_id": n}));
                } else {
                    deps.push(json!({"kind": kind, "repo": repo, "number": n}));
                }
            }
        }
    }
    if let Some(v) = args.get("blocked-by-task") {
        for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let n: i64 = item.parse().unwrap_or_else(|_| {
                die(
                    2,
                    &format!("block: --blocked-by-task expects a task id, got '{item}'"),
                )
            });
            deps.push(json!({"kind": "task", "task_id": n}));
        }
    }
    deps
}

/// Parse the --drop-dep list of an unblock (#12777): comma-
/// separated dep identities in the form kind:ref — issue:
/// owner/repo#N, pr:owner/repo#N, ci:owner/repo#RUN_ID,
/// task:ID — the same refs block's --blocked-by-* flags take,
/// kind-prefixed so one list can name several kinds. The
/// broker re-validates every spec; a bad ref dies here first,
/// at the caller's terminal.
pub(super) fn parse_drop_dep_list(list: &str) -> Result<Vec<Value>, String> {
    let mut deps = Vec::new();
    for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let bad = |what: &str| format!("unblock: --drop-dep expects {what}, got '{item}'");
        let Some((kind, r)) = item.split_once(':') else {
            return Err(bad("kind:ref"));
        };
        match kind {
            "pr" | "issue" | "ci" => {
                let Some((repo, num)) = r.rsplit_once('#') else {
                    return Err(bad("owner/repo#N after the kind"));
                };
                let n: i64 = num
                    .parse()
                    .map_err(|_| bad("owner/repo#N after the kind"))?;
                if kind == "ci" {
                    deps.push(json!({"kind": kind, "repo": repo, "run_id": n}));
                } else {
                    deps.push(json!({"kind": kind, "repo": repo, "number": n}));
                }
            }
            "task" => {
                let n: i64 = r.parse().map_err(|_| bad("a task id after 'task:'"))?;
                deps.push(json!({"kind": "task", "task_id": n}));
            }
            _ => return Err(bad("kind pr|issue|task|ci")),
        }
    }
    Ok(deps)
}

pub(super) fn drop_dep_flags(args: &Args) -> Vec<Value> {
    match args.get("drop-dep") {
        Some(v) => parse_drop_dep_list(&v).unwrap_or_else(|e| die(2, &e)),
        None => Vec::new(),
    }
}

/// unblock (#8276): the manual override — release a blocked
/// task now, dep-blocked or classic. The broker audits who
/// overrode and what dep record was discarded. With
/// --drop-dep (#12777) the override turns surgical: only the
/// named dep entries leave the task's record, and the task
/// stays blocked on the rest.
pub(super) fn cmd_unblock(rest: &[String]) -> i32 {
    let args = parse_verb_args("unblock", rest, &["worker", "reason", "drop-dep"], &[]);
    reject_extra_pos("unblock", &args, 1);
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": args.get("reason").unwrap_or_default(),
    });
    let drops = drop_dep_flags(&args);
    if !drops.is_empty() {
        body["drop_deps"] = json!(drops);
    }
    let resp = unwrap_ok(call("/unblock", &body));
    print_json(&served_row(require_row(&resp, "unblock_task"), "unblock"));
    0
}

/// dep-recheck (#8276): run the broker's dep recheck pass now,
/// or force one task's recheck with --task. Production cadence
/// is the sweep's (the expire timer fires it every minute);
/// this is the manual trigger.
pub(super) fn cmd_dep_recheck(rest: &[String]) -> i32 {
    let args = parse_verb_args("dep-recheck", rest, &["task", "limit"], &[]);
    reject_extra_pos("dep-recheck", &args, 0);
    let mut body = json!({});
    if let Some(t) = args.get_i64("task") {
        body["task_id"] = json!(t);
    }
    if let Some(l) = args.get_i64("limit") {
        body["limit"] = json!(l);
    }
    let resp = unwrap_ok(call("/dep-recheck", &body));
    print_json(&served_row(
        require_row(&resp, "dep_recheck"),
        "dep-recheck",
    ));
    0
}

/// quarantine-recheck (#10648): run the broker's quarantine
/// recheck pass now, or force one blocked task's recheck with
/// --task. The pass re-validates blocked, dep-free rows whose
/// recorded error is a claim-path quarantine and requeues the
/// ones whose error no longer reproduces. Production cadence
/// is the sweep's; this is the manual trigger.
pub(super) fn cmd_quarantine_recheck(rest: &[String]) -> i32 {
    let args = parse_verb_args("quarantine-recheck", rest, &["task", "limit"], &[]);
    reject_extra_pos("quarantine-recheck", &args, 0);
    let mut body = json!({});
    if let Some(t) = args.get_i64("task") {
        body["task_id"] = json!(t);
    }
    if let Some(l) = args.get_i64("limit") {
        body["limit"] = json!(l);
    }
    let resp = unwrap_ok(call("/quarantine-recheck", &body));
    print_json(&served_row(
        require_row(&resp, "quarantine_recheck"),
        "quarantine-recheck",
    ));
    0
}
/// repair (#13191): repair a quarantined / malformed task
/// packet in place. The broker re-runs the packet validators,
/// applies the deterministic repairs whose correct value can be
/// re-derived (today: the dedupe stamp, re-performed live), and
/// restores the task to claimable state; unrepairable defects
/// fail with the defect list and change nothing. --dry-run
/// reports defects + planned repairs and writes nothing.
pub(super) fn cmd_repair(rest: &[String]) -> i32 {
    let args = parse_verb_args("repair", rest, &["worker", "reason"], &["dry-run"]);
    reject_extra_pos("repair", &args, 1);
    let body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": args.get("reason").unwrap_or_default(),
        "dry_run": args.has("dry-run"),
    });
    let resp = unwrap_ok(call("/repair", &body));
    print_json(&require_row(&resp, "repair_task"));
    0
}

pub(super) fn cmd_cancel(rest: &[String]) -> i32 {
    let args = parse_verb_args("cancel", rest, &["worker", "reason"], &[]);
    reject_extra_pos("cancel", &args, 1);
    let reason = args
        .get("reason")
        .unwrap_or_else(|| die(2, "cancel: --reason is required"));
    let worker = args
        .get("worker")
        .unwrap_or_else(|| "fleet-monitor".to_string());
    let resp = unwrap_ok(call(
        "/cancel",
        &json!({
            "task_id": args.pos_i64(0, "task_id"),
            "worker": worker,
            "reason": reason,
        }),
    ));
    let row = served_row(require_row(&resp, "cancel_repo_task"), "cancel");
    // #9046: a cancel closes the binding window on the timeline.
    unbound_append(&worker, &row, "cancel");
    print_json(&row);
    0
}

/// Revive a dead task in place (#6838): a failed task, or a queued
/// task dead at cap (attempts exhausted), gets attempts=0 and a
/// fresh not_before through the broker's /revive verb — the
/// supported replacement for the monitor's direct-SQL resets.
pub(super) fn cmd_revive(rest: &[String]) -> i32 {
    let args = parse_verb_args("revive", rest, &["worker", "reason", "not-before"], &[]);
    reject_extra_pos("revive", &args, 1);
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "reason": args.get("reason").unwrap_or_default(),
    });
    if let Some(nb) = args.get("not-before") {
        let nb = match validate_not_before_flag(&nb) {
            Ok(v) => v,
            Err(e) => die(2, &e),
        };
        body["not_before"] = json!(nb);
    }
    let resp = unwrap_ok(call("/revive", &body));
    print_json(&served_row(require_row(&resp, "revive_task"), "revive"));
    0
}

pub(super) fn cmd_fail(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "fail",
        rest,
        &["worker", "lane", "error", "reason-class", "execution"],
        &[],
    );
    reject_extra_pos("fail", &args, 1);
    let error = args
        .get("error")
        .unwrap_or_else(|| die(2, "fail: --error is required"));
    let reason_class = args
        .get("reason-class")
        .unwrap_or_else(|| "transient".to_string());
    if !["transient", "deterministic", "blocked"].contains(&reason_class.as_str()) {
        die(
            2,
            &format!("argument --reason-class: invalid choice: '{reason_class}'"),
        );
    }
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "lane": lane_arg(&args),
        "error": error,
        "reason_class": reason_class,
    });
    if let Some(ex) = execution_arg(&args) {
        body["execution"] = ex;
    }
    let resp = unwrap_ok(call("/fail", &body));
    // #9046: a fail closes the binding window on the timeline.
    if let Some(row) = resp.get("row") {
        unbound_append(&opt_worker(&args), row, "fail");
    }
    row_or_ledger(&resp, "fail_repo_task", "fail");
    0
}

pub(super) fn cmd_complete(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "complete",
        rest,
        &[
            "worker",
            "lane",
            "result",
            "result-file",
            "telemetry",
            "execution",
        ],
        &[],
    );
    reject_extra_pos("complete", &args, 1);
    let mut result_raw = args.get("result");
    if let Some(f) = args.get("result-file") {
        result_raw = Some(match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => die(2, &format!("cannot read result file: {e}")),
        });
    }
    let mut result_obj = json!({});
    if let Some(raw) = result_raw {
        if !raw.is_empty() {
            result_obj = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => die(2, &format!("--result is not valid JSON: {e}")),
            };
            if !result_obj.is_object() {
                die(2, "--result must be a JSON object");
            }
        }
    }
    check_telemetry_fields(&result_obj, "--result");
    if let Some(tel_raw) = args.get("telemetry") {
        let tel: Value = match serde_json::from_str(&tel_raw) {
            Ok(v) => v,
            Err(e) => die(2, &format!("--telemetry is not valid JSON: {e}")),
        };
        if !tel.is_object() {
            die(2, "--telemetry must be a JSON object");
        }
        let unknown: Vec<String> = tel
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| !TELEMETRY_FIELDS.contains(&k.as_str()))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            let mut sorted = unknown.clone();
            sorted.sort();
            die(
                2,
                &format!(
                    "--telemetry: unknown field(s) {}; allowed: {}",
                    py_str_list(&sorted),
                    py_str_list(
                        &TELEMETRY_FIELDS
                            .iter()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                    )
                ),
            );
        }
        check_telemetry_fields(&tel, "--telemetry");
        for field in TELEMETRY_FIELDS {
            if tel.get(field).is_some() && result_obj.get(field).is_none() {
                result_obj[field] = tel[field].clone();
            }
        }
    }
    let ex = execution_arg(&args);
    if let Some(ex) = &ex {
        if result_obj.get("execution").is_none() {
            let mut block = Map::new();
            block.insert(
                "lane".to_string(),
                ex.get("lane").cloned().unwrap_or(Value::Null),
            );
            block.insert(
                "session_id".to_string(),
                ex.get("session_id").cloned().unwrap_or(Value::Null),
            );
            let has_fb = ex
                .get("fallback_reason")
                .map(|v| !v.is_null())
                .unwrap_or(false);
            block.insert("fallback".to_string(), Value::Bool(has_fb));
            if has_fb {
                block.insert("fallback_reason".to_string(), ex["fallback_reason"].clone());
            }
            if let Some(a) = ex.get("assigned") {
                block.insert("assigned".to_string(), a.clone());
            }
            result_obj["execution"] = Value::Object(block);
        }
    }
    // #9046 close-out convenience: record the bound session's
    // dominant model (route_attempt call count over this task's
    // claim window) in the result, so lists and reports don't
    // need the session join for the common case. Fail-soft at
    // every step — any miss completes without the summary, and
    // a caller-supplied dominant_model always wins.
    if result_obj.get("dominant_model").is_none() {
        if let Some(dom) = dominant_model_for_task(&args, args.pos_i64(0, "task_id")) {
            result_obj["dominant_model"] = dom;
        }
    }
    // #4919: completion telemetry (pool/model/effort/cost_usd) is
    // defaulted here, from the claim record and the completing
    // session, so it no longer depends on a worker remembering
    // --telemetry. Explicit caller values always win.
    fill_completion_telemetry(args.pos_i64(0, "task_id"), &mut result_obj);
    let mut body = json!({
        "task_id": args.pos_i64(0, "task_id"),
        "worker": opt_worker(&args),
        "lane": lane_arg(&args),
        "result": result_obj,
    });
    if let Some(ex) = ex {
        body["execution"] = ex;
    }
    let resp = unwrap_ok_fb(call_fb("/complete", &body), "/complete");
    // #9046: completion closes the binding window on the timeline.
    if let Some(row) = resp.get("row") {
        unbound_append(&opt_worker(&args), row, "complete");
    }
    row_or_ledger(&resp, "complete_task", "complete");
    0
}

/// Value flags the `health` verb accepts (#7349). `--worker` is
/// accepted for attribution only: the opencode-claimant packet
/// mandates `--worker` on every verb, and a packet-literal fire used
/// to abort on its first health call. It changes neither the request
/// body nor the printed table.
pub(super) const HEALTH_FLAGS: &[&str] = &["queue", "worker"];

pub(super) fn cmd_health(rest: &[String]) -> i32 {
    let args = parse_verb_args("health", rest, HEALTH_FLAGS, &[]);
    reject_extra_pos("health", &args, 0);
    let mut body = json!({});
    if let Some(q) = args.get("queue") {
        body["queue"] = json!(q);
    }
    let resp = unwrap_ok_fb(call_fb("/health", &body), "/health");
    let netloc = resp
        .get("project_netloc")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source = resp
        .get("project_source")
        .and_then(|v| v.as_str())
        .unwrap_or("bitwarden");
    println!("project: {netloc} (resolved via {source})");
    let rows = resp
        .get("rows")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if rows.is_empty() {
        println!("(no queues have tasks)");
        return 0;
    }
    let cols: Vec<String> = rows[0]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let str_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| cols.iter().map(|c| fmt_cell(r.get(c))).collect())
        .collect();
    print_table(&cols, &str_rows);
    0
}

pub(super) fn cmd_signal(rest: &[String]) -> i32 {
    let args = parse_verb_args("signal", rest, &["queue", "reason"], &[]);
    reject_extra_pos("signal", &args, 0);
    let queue = opt_queue(&args);
    let reason = args
        .get("reason")
        .unwrap_or_else(|| "manual_signal".to_string());
    let resp = unwrap_ok(call("/signal-count", &json!({"queue": queue})));
    let queued_count = resp.get("queued_count").cloned().unwrap_or(json!(0));
    let oldest = resp.get("oldest_queued_at").cloned().unwrap_or(Value::Null);

    let signal_path_env = std::env::var("REPO_QUEUE_SIGNAL_PATH").ok();
    let signal_path = signal_path_from_env(signal_path_env.as_deref(), Some(&home()))
        .unwrap_or_else(|error| die(3, error));
    let dir = std::path::Path::new(&signal_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let state_path = format!("{dir}/repo-queue-signal-state.json");
    let mut generation: i64 = 0;
    for candidate in [&signal_path, &state_path] {
        if let Ok(text) = std::fs::read_to_string(candidate) {
            if let Ok(prev) = serde_json::from_str::<Value>(&text) {
                if prev.get("queue").and_then(|q| q.as_str()) == Some(queue.as_str()) {
                    if let Some(g) = prev.get("generation").and_then(|g| g.as_i64()) {
                        generation = generation.max(g);
                    }
                }
            }
        }
    }
    generation += 1;
    let updated = utcnow_iso();
    let signal = json!({
        "queue": queue,
        "generation": generation,
        "queued_count": queued_count,
        "oldest_queued_at": oldest,
        "updated_at": updated,
        "reason": reason,
    });
    let state = json!({
        "queue": queue,
        "generation": generation,
        "updated_at": updated,
    });
    let _ = std::fs::create_dir_all(&dir);
    atomic_write_json(&signal_path, &signal);
    atomic_write_json(&state_path, &state);
    print_json(&signal);
    0
}

pub(super) fn atomic_write_json(path: &str, obj: &Value) {
    let tmp = format!("{path}.tmp{}", std::process::id());
    let mut text = serde_json::to_string_pretty(obj).unwrap_or_default();
    text.push('\n');
    match std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, path)) {
        Ok(_) => {}
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            die(3, &format!("cannot write {path}: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Creation outbox (#3706) — client verbs + the serial drainer.
//
// `fleet queue outbox enqueue|list|metrics|claim|resolve` are the
// producer API and the drainer's building blocks; `outbox drain`
// is the pilot drainer itself: claim the oldest due entry from the
// broker, run its create through `gh`, verify by read-back, and
// resolve. Exactly one producer is wired as the pilot
// (new-issue-creator-code-a — doc/fleet/creation-outbox.md); there
// is no dual-write and no automatic fallback to direct creation:
// a broker failure stops the drain visibly.
// ---------------------------------------------------------------------------

pub(super) fn outbox_payload(args: &Args) -> Value {
    if let Some(path) = args.get("payload-file") {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| die(2, &format!("cannot read payload file {path}: {e}")));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| die(2, &format!("payload file {path} is not JSON: {e}")))
    } else if let Some(raw) = args.get("payload") {
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| die(2, &format!("--payload is not JSON: {e}")))
    } else {
        die(2, "outbox enqueue needs --payload-file or --payload")
    }
}

pub(super) fn cmd_outbox_enqueue(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "outbox enqueue",
        rest,
        &[
            "repo",
            "kind",
            "dedup-key",
            "producer",
            "payload-file",
            "payload",
        ],
        &[],
    );
    reject_extra_pos("outbox enqueue", &args, 0);
    let resp = unwrap_ok(call(
        "/outbox-enqueue",
        &json!({
            "repo": args.get("repo"),
            "kind": args.get("kind"),
            "dedup_key": args.get("dedup-key"),
            "producer": args.get("producer"),
            "payload": outbox_payload(&args),
        }),
    ));
    print_json(&resp);
    0
}

pub(super) fn cmd_outbox_list(rest: &[String]) -> i32 {
    let args = parse_verb_args("outbox list", rest, &["repo", "state"], &[]);
    reject_extra_pos("outbox list", &args, 0);
    let resp = unwrap_ok(call(
        "/outbox-list",
        &json!({"repo": args.get("repo"), "state": args.get("state")}),
    ));
    print_json(&resp);
    0
}

pub(super) fn cmd_outbox_metrics(rest: &[String]) -> i32 {
    let args = parse_verb_args("outbox metrics", rest, &[], &[]);
    reject_extra_pos("outbox metrics", &args, 0);
    let resp = unwrap_ok(call("/outbox-metrics", &json!({})));
    print_json(&resp);
    0
}

pub(super) fn outbox_claim(owner: &str) -> Value {
    unwrap_ok(call("/outbox-claim", &json!({"owner": owner})))
}

pub(super) fn outbox_resolve(id: &str, outcome: &str, extra: Value) -> Value {
    let mut body = json!({"id": id, "outcome": outcome});
    if let (Some(dst), Some(src)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    unwrap_ok(call("/outbox-resolve", &body))
}

pub(super) fn cmd_outbox_claim(rest: &[String]) -> i32 {
    let args = parse_verb_args("outbox claim", rest, &["owner"], &[]);
    reject_extra_pos("outbox claim", &args, 0);
    let owner = args
        .get("owner")
        .unwrap_or_else(|| "outbox-drainer".to_string());
    print_json(&outbox_claim(&owner));
    0
}

pub(super) fn cmd_outbox_resolve(rest: &[String]) -> i32 {
    let args = parse_verb_args(
        "outbox resolve",
        rest,
        &[
            "outcome",
            "github-number",
            "github-url",
            "retry-after",
            "error",
        ],
        &[],
    );
    if args.pos.is_empty() {
        die(
            2,
            "usage: repo-queue outbox resolve <id> --outcome done|rate_limited|uncertain|failed",
        );
    }
    let outcome = args.get("outcome").unwrap_or_default();
    let resp = outbox_resolve(
        &args.pos[0],
        &outcome,
        json!({
            "github_number": args.get_i64("github-number"),
            "github_url": args.get("github-url"),
            "retry_after": args.get_i64("retry-after"),
            "error": args.get("error"),
        }),
    );
    print_json(&resp);
    0
}
