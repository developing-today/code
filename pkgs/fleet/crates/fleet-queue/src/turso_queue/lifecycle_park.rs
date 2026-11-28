//! turso_queue::lifecycle_park — wait-condition mapping, wait
//! floors, and the park verb (#16255).
//!
//! Split from turso_queue::lifecycle for #16255 (production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

/// wait_kind mapping, copied from the PG park_task body (0023 §3).
pub(crate) fn wait_kind(condition: &Value) -> &'static str {
    match condition.get("kind").and_then(|k| k.as_str()) {
        Some("time") => "timer",
        Some("triage") => "triage",
        Some("ci") | Some("ci_green") | Some("pr_merged") | Some("issue_closed")
        | Some("upstream_pr") => "ci",
        Some("review") | Some("codex_review") | Some("pr_review") | Some("person_answer") => {
            "review"
        }
        Some("capacity") | Some("quota_reset") => "capacity",
        _ => "external",
    }
}

/// Broker-owned minimum wait TTLs (#8184): whatever delay a lane
/// asks for, a recorded wait is held at least this long. Lanes
/// re-discovering an unchanged wait every 20-25 minutes was the
/// park churn: one discovery park per wait at these scales costs
/// ~3% of the cycles. Floors: triage waits 6h (the triage step
/// works on its own cadence and wakes the item via /wake when
/// done), ci and capacity waits 1h (backlog-scale, not
/// poll-scale). Timer/review/external waits keep the caller's
/// delay — those parks carry an explicit owner-chosen horizon.
pub(crate) fn wait_floor_seconds(wait_kind: &str) -> Option<i64> {
    match wait_kind {
        "triage" => Some(21_600),
        "ci" | "capacity" => Some(3_600),
        _ => None,
    }
}

/// Apply the wait-kind floor to a park's not_before: a requested
/// wake earlier than the floor is bumped to the floor (and the
/// park becomes relative); a later one stands.
pub(crate) fn apply_wait_floor(
    wait_kind: &str,
    not_before: String,
    park_kind: &'static str,
) -> (String, &'static str) {
    match wait_floor_seconds(wait_kind) {
        Some(floor) => {
            let earliest = after_iso(floor);
            if not_before < earliest {
                (earliest, "relative")
            } else {
                (not_before, park_kind)
            }
        }
        None => (not_before, park_kind),
    }
}

/// #9179 class (c)4: a park carrying a structured time condition
/// (`{"kind": "time", "until": <rfc3339>}`) must not lapse before
/// that condition's own horizon. The audit's live case (task
/// 13224) was parked for a day while its recorded resume
/// condition was a month out, so it lapsed into the unchanged
/// condition and burned a lane pass re-parking. When the
/// condition's `until` parses and is later than the requested
/// lapse term, the lapse term rises to `until` (and the park
/// becomes absolute — its term is now that fixed timestamp).
/// An earlier, equal, missing, or unparseable `until`, and any
/// non-time condition, leave the park exactly as requested.
pub(crate) fn apply_time_condition_floor(
    condition: &Value,
    not_before: String,
    park_kind: &'static str,
) -> (String, &'static str) {
    if condition.get("kind").and_then(|k| k.as_str()) != Some("time") {
        return (not_before, park_kind);
    }
    let Some(until) = condition.get("until").and_then(|u| u.as_str()) else {
        return (not_before, park_kind);
    };
    let (Some(until_epoch), Some(nb_epoch)) = (
        crate::disposition::parse_rfc3339_epoch(until),
        crate::disposition::parse_rfc3339_epoch(&not_before),
    ) else {
        return (not_before, park_kind);
    };
    if until_epoch > nb_epoch {
        (until.to_string(), "absolute")
    } else {
        (not_before, park_kind)
    }
}

/// #3418: the required budget a park records. An explicit
/// `required_budget_minutes` in the park request wins. Otherwise a
/// capacity wait (ready-condition kind `capacity`) defaults the
/// requirement from the packet's own `timebox_minutes` — the
/// natural default source in the #3418 design sketch — so a
/// capacity park is budget-matched without the parker restating
/// the packet's number. Any other park records no requirement and
/// keeps behaving exactly as before. Non-positive values record
/// no requirement.
pub(crate) fn park_required_budget(
    explicit: Option<i64>,
    condition: &Value,
    payload: &Value,
) -> Option<i64> {
    if let Some(v) = explicit {
        return (v > 0).then_some(v);
    }
    if condition.get("kind").and_then(|k| k.as_str()) == Some("capacity") {
        return payload
            .get("timebox_minutes")
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0);
    }
    None
}

/// #9178 §2 (FDR-0002): the live manual-claim override a park must
/// honor. Returns the recorded audit when the row carries a
/// `result.manual_override` bound to the CURRENT claim and the
/// incoming park's (wait_kind, ready_condition) equals the recorded
/// previous pair — the owner already overrode exactly this
/// condition, so a boot-time re-park for it must refuse. Anything
/// else (no audit, a materially new condition, or a stale marker
/// from an earlier claim — every claim stamps a fresh claimed_at,
/// so the audit's claimed_at only matches the live claim) parks as
/// before.
pub(crate) fn manual_override_park_cover(
    row: &Value,
    wait_kind: &str,
    condition: &Value,
) -> Option<Value> {
    let audit = row.get("result")?.get("manual_override")?;
    if !audit.is_object() {
        return None;
    }
    if audit.get("claimed_at") != row.get("claimed_at") {
        return None;
    }
    if audit.get("previous_wait_kind").and_then(Value::as_str) != Some(wait_kind) {
        return None;
    }
    let previous = audit
        .get("previous_ready_condition")
        .unwrap_or(&Value::Null);
    if !same_wait_condition(previous, condition, wait_kind) {
        return None;
    }
    Some(audit.clone())
}

/// #16392: is the incoming park's condition the SAME wait the audit
/// recorded, for cover purposes? Exact equality is the base case,
/// but a time condition's `until` is a lapse horizon minted fresh
/// on every bare park (now + delay) — it is not what makes the wait
/// the wait (#9179 (c)4's own framing). Two time conditions are
/// therefore the same wait when everything except `until` matches,
/// and a recorded NULL condition (older rows parked before a
/// condition was always stored) matches a bare timer re-park: the
/// recorded wait carried no explicit condition, so there is no
/// materially new condition to honor instead.
fn same_wait_condition(previous: &Value, incoming: &Value, wait_kind: &str) -> bool {
    if previous == incoming {
        return true;
    }
    if previous.is_null() {
        return wait_kind == "timer"
            && incoming.get("kind").and_then(Value::as_str) == Some("time");
    }
    let strip_until = |v: &Value| -> Value {
        let mut obj = match v.as_object() {
            Some(o) => o.clone(),
            None => return v.clone(),
        };
        obj.remove("until");
        Value::Object(obj)
    };
    previous.get("kind").and_then(Value::as_str) == Some("time")
        && incoming.get("kind").and_then(Value::as_str) == Some("time")
        && strip_until(previous) == strip_until(incoming)
}

pub(crate) fn verb_park(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "park_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    // #9179: an explicit not_before is validated HERE, at the
    // storage layer — the dispatch gate (#6426) only covers
    // requests that pass through it, and the audit's malformed
    // rows ("3600"/"7200" stored verbatim) prove a path around it
    // existed. A value that fails the rule fails the park before
    // any state changes.
    if let Some(nb) = req_str(req, "not_before") {
        if let Err(e) = validate_stored_not_before(nb) {
            return fail(2, format!("park_task: {e}"));
        }
    }
    let (not_before, park_kind) = match req_str(req, "not_before") {
        Some(nb) => (nb.to_string(), "absolute"),
        None => (
            after_iso(req_i64(req, "delay_seconds").unwrap_or(1200)),
            "relative",
        ),
    };
    let given_progress = req.get("progress").cloned().unwrap_or(json!({}));
    let mut condition_defaulted = false;
    let condition = match req
        .get("ready_condition")
        .filter(|v| !v.is_null())
        .cloned()
        .or_else(|| given_progress.get("ready_condition").cloned())
    {
        Some(c) => c,
        None => {
            condition_defaulted = true;
            json!({"kind": "time", "until": not_before})
        }
    };
    // #8184: a park whose recorded stage is awaiting_triage IS a
    // triage wait, whatever condition the caller attached — the
    // broker classifies it, floors its TTL, and owns the wait.
    let wk = if progress_stage(&given_progress) == Some("awaiting_triage") {
        "triage"
    } else {
        wait_kind(&condition)
    };
    let (not_before, park_kind) = apply_wait_floor(wk, not_before, park_kind);
    let condition = if condition_defaulted {
        json!({"kind": "time", "until": not_before})
    } else {
        condition
    };
    // #9179 (c)4: a caller-supplied time condition whose `until`
    // is later than the park's lapse term raises the lapse term —
    // the park must not resume into the unchanged condition.
    let (not_before, park_kind) = apply_time_condition_floor(&condition, not_before, park_kind);
    // #9178 §2 (FDR-0002): a worker booting under a manual claim
    // must not immediately re-park the task for the same unchanged
    // condition the owner overrode. The cover binds to the live
    // claim via claimed_at, so a stale marker from an earlier claim
    // is inert and a materially new condition still parks.
    if let Some(audit) = manual_override_park_cover(&row, wk, &condition) {
        let detail = json!({
            "reason": reason, "status": "claimed",
            "via": "park_task", "refusal": "manual_override_honored",
            "wait_kind": wk, "ready_condition": condition,
            "manual_override": audit,
        });
        let _ = event(
            t,
            Some(&worker),
            Some(task_id),
            "park_refused",
            &with_marks(&detail, &row),
        );
        return fail(
            5,
            format!(
                "park_task: task {task_id} keeps its manual-claim override ({} at {}, reason: {}) — the recorded wait condition ({wk}) is unchanged, so the park is refused; a materially new condition may still park it",
                audit
                    .get("worker_id")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                audit
                    .get("claimed_at")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                audit.get("reason").and_then(Value::as_str).unwrap_or("?"),
            ),
        );
    }
    // #3418: the budget requirement this park records (explicit in
    // the request, or defaulted from the packet timebox for a
    // capacity wait). A capacity park stays timer-floored as
    // before, but a claim declaring enough remaining budget takes
    // it immediately through the pick predicate's capacity bypass;
    // external-wait parks record no requirement unless the parker
    // sets one explicitly, and the two conditions then compose.
    let required_budget = park_required_budget(
        req_i64(req, "required_budget_minutes"),
        &condition,
        row.get("payload").unwrap_or(&Value::Null),
    );
    let required_budget_arg = match required_budget {
        Some(v) => Arg::Int(v),
        None => Arg::Null,
    };
    let (progress, cleared) = clear_binding(
        &row.get("progress").cloned().unwrap_or(json!({})),
        "park_task",
    );
    let progress = merge_obj(&progress, &given_progress);
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"park_reason": reason}),
    );
    // #8234: the park verb classifies its own landing. A park whose
    // merged progress is a recorded opencode handoff with no
    // caller-declared genuine wait is not a wait at all: it lands
    // in the handoff sink (status 'handed_off', every wait field
    // NULL) instead of 'parked' under the default timer. The
    // triage classification above is a genuine wait by #8184's own
    // rule and never sinks.
    let to_sink = wk != "triage" && handoff_park(&progress, condition_defaulted, &condition);
    // #8368: an explicit caller delay (delay_seconds / not_before in
    // the request) is a caller-declared wait. The sink landing below
    // keeps it as not_before even though every other wait field is
    // NULL — discarding it re-served bound tasks immediately,
    // defeating the park.
    let explicit_delay = req.get("not_before").is_some_and(|v| !v.is_null())
        || req.get("delay_seconds").is_some_and(|v| !v.is_null());
    let work = (|| -> Result<Value, TErr> {
        // #9487: both landings carry the holder guard; a park from
        // a claim that changed after claimed_by read it writes
        // nothing.
        let upd = if to_sink {
            let mut args = vec![
                if explicit_delay {
                    Arg::Text(not_before.clone())
                } else {
                    Arg::Null
                },
                Arg::Text(now_iso()),
                Arg::Text(progress.to_string()),
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ];
            args.extend(holder_guard_args(&worker, &row));
            t.exec(
                &format!(
                    "UPDATE tasks SET status = 'handed_off', worker_id = NULL, lease_until = NULL, \
                       not_before = ?, park_kind = NULL, wait_kind = NULL, ready_condition = NULL, \
                       required_budget_minutes = NULL, \
                       last_checked_at = ?, check_outcome = NULL, progress = ?, result = ?, \
                       updated_at = ? WHERE id = ?{HOLDER_GUARD}"
                ),
                &args,
            )?
        } else {
            let mut args = vec![
                Arg::Text(not_before.clone()),
                Arg::Text(park_kind.to_string()),
                Arg::Text(wk.to_string()),
                Arg::Text(condition.to_string()),
                required_budget_arg,
                Arg::Text(now_iso()),
                Arg::Text(progress.to_string()),
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ];
            args.extend(holder_guard_args(&worker, &row));
            t.exec(
                &format!(
                    "UPDATE tasks SET status = 'parked', worker_id = NULL, lease_until = NULL, \
                       not_before = ?, park_kind = ?, wait_kind = ?, ready_condition = ?, \
                       required_budget_minutes = ?, \
                       last_checked_at = ?, check_outcome = NULL, progress = ?, result = ?, \
                       updated_at = ? WHERE id = ?{HOLDER_GUARD}"
                ),
                &args,
            )?
        };
        if upd.affected == 0 {
            return Ok(claim_changed("park_task", task_id));
        }
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, task_id, old, "park_task")?;
        }
        let detail = if to_sink {
            json!({
                "reason": reason, "status": "handed_off",
                "park_kind": Value::Null, "wait_kind": Value::Null,
                "not_before": if explicit_delay {
                    Value::String(not_before.clone())
                } else {
                    Value::Null
                },
                "ready_condition": Value::Null,
                "cleared_session_id": cleared,
            })
        } else {
            json!({
                "reason": reason, "status": "parked",
                "park_kind": park_kind, "wait_kind": wk,
                "not_before": not_before, "ready_condition": condition,
                "required_budget_minutes": required_budget,
                "cleared_session_id": cleared,
            })
        };
        event(
            t,
            Some(&worker),
            Some(task_id),
            "parked",
            &with_marks(&detail, &row),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}
