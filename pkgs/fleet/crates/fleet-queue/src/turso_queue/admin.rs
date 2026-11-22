//! turso_queue::admin — block and dependency/quarantine rechecks, cancel, fail, complete, list, health.
//!
//! Split from turso_queue.rs for #8589 (F11.10 production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

pub(crate) fn verb_block(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason_class = req_str(req, "reason_class").unwrap_or("").to_string();
    let blocker = req_str(req, "blocker").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "block_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let progress = merge_obj(
        &merge_obj(
            &row.get("progress").cloned().unwrap_or(json!({})),
            &req.get("progress").cloned().unwrap_or(json!({})),
        ),
        &json!({"blocker": blocker, "reason_class": reason_class, "blocked_via": "lane"}),
    );
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"blocker": blocker, "reason_class": reason_class, "blocked_via": "lane"}),
    );
    // #8276: typed deps, when the request carries them — the block
    // becomes data the broker watches (see run_dep_rechecks). The
    // first recheck is due immediately, so a dep that is already
    // satisfied (an already-merged PR) clears within one sweep.
    // A dep-less block NULLs the dep columns: a stale record can
    // never outlive the block that owned it.
    let now_unix_v = now_unix();
    let deps = match parse_block_deps(req, &worker, &iso(now_unix_v)) {
        Ok(d) => d,
        Err(e) => return fail(2, e),
    };
    let (dep_json, dep_next, dep_until, dep_rs, dep_event) = match &deps {
        Some(ds) => {
            let rs = clamp_dep_recheck_seconds(req_i64(req, "dep_recheck_seconds"));
            let ttl = clamp_dep_ttl_seconds(req_i64(req, "dep_max_ttl_seconds"));
            let until = iso(now_unix_v + ttl);
            (
                Some(json!(ds).to_string()),
                Some(iso(now_unix_v)),
                Some(until.clone()),
                Some(rs),
                json!({
                    "deps": ds, "dep_blocked_until": until,
                    "dep_recheck_seconds": rs,
                }),
            )
        }
        None => (None, None, None, None, json!({})),
    };
    let work = (|| -> Result<Value, TErr> {
        // #10714: a lane block supersedes any stale last_error. The
        // quarantine rechecker (#10648) selects blocked rows by a
        // `last_error LIKE 'quarantined by claim path%'` prefix, so a
        // row that once carried a repaired #5444 quarantine error
        // would be requeued within one sweep minute after a lane
        // blocked it. Clearing the fossil here keeps the lane's block
        // authoritative; the rechecker's release branch also refuses
        // lane-blocked rows (defence in depth).
        t.exec(
            "UPDATE tasks SET status = 'blocked', worker_id = NULL, lease_until = NULL, \
               progress = ?, result = ?, blocked_deps = ?, dep_next_recheck_at = ?, \
               dep_blocked_until = ?, dep_recheck_seconds = ?, last_error = NULL, updated_at = ? \
             WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(result.to_string()),
                arg_str(dep_json.as_deref()),
                arg_str(dep_next.as_deref()),
                arg_str(dep_until.as_deref()),
                dep_rs.map(Arg::Int).unwrap_or(Arg::Null),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "blocked",
            &merge_obj(
                &json!({"blocker": blocker, "reason_class": reason_class}),
                &dep_event,
            ),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => terr(e),
    }
}

pub(crate) fn emit_dep_transitions(
    t: &TursoBackend,
    task_id: i64,
    row: &Value,
    transitions: &[Value],
) -> Result<(), TErr> {
    for tr in transitions {
        let kind = if tr.get("transition").and_then(|v| v.as_str()) == Some("broken") {
            "dep_broken"
        } else {
            "dep_cleared"
        };
        event(
            t,
            Some("dep-rechecker"),
            Some(task_id),
            kind,
            &with_marks(
                &json!({
                    "dep": tr.get("dep").cloned().unwrap_or(Value::Null),
                    "evidence": tr.get("evidence").cloned().unwrap_or(Value::Null),
                    "via": "dep_recheck",
                }),
                row,
            ),
        )?;
    }
    Ok(())
}

/// One dep-blocked task's recheck (#8276). The max-TTL deadline
/// is checked before any probing: an expired block releases
/// without consulting GitHub, so a dead checker cannot stretch a
/// block past its TTL. Otherwise each open dep gets one
/// observation (task deps from the store, GitHub deps through the
/// probe, refs shared across the pass via `cache`), transitions
/// are audited, and the task either unblocks (every dep resolved)
/// or reschedules at its recheck interval. Returns the outcome
/// tag ("unblocked" / "ttl_expired" / "still_blocked" /
/// "not_dep_blocked") and the number of failed checks seen.
pub(crate) fn dep_recheck_one(
    t: &TursoBackend,
    row: &Value,
    cache: &mut std::collections::HashMap<String, DepObs>,
    now_unix_v: i64,
) -> Result<(&'static str, i64), TErr> {
    let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let now = iso(now_unix_v);
    let deps = deps_of(row);
    if deps.is_empty() {
        return Ok(("not_dep_blocked", 0));
    }
    if let Some(until) = row.get("dep_blocked_until").and_then(|v| v.as_str()) {
        if until <= now.as_str() {
            let cleared = t.exec(
                "UPDATE tasks SET status = 'queued', blocked_deps = NULL, \
                   dep_next_recheck_at = NULL, dep_blocked_until = NULL, \
                   dep_recheck_seconds = NULL, updated_at = ? \
                 WHERE id = ? AND status = 'blocked'",
                &[Arg::Text(now.clone()), Arg::Int(task_id)],
            )?;
            if cleared.affected > 0 {
                event(
                    t,
                    Some("dep-rechecker"),
                    Some(task_id),
                    "dep_ttl_expired",
                    &with_marks(
                        &json!({
                            "deps": deps, "dep_blocked_until": until,
                            "via": "dep_recheck",
                        }),
                        row,
                    ),
                )?;
                return Ok(("ttl_expired", 0));
            }
            return Ok(("still_blocked", 0));
        }
    }
    let recheck_seconds = row
        .get("dep_recheck_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(DEP_RECHECK_DEFAULT_SECONDS);
    let mut new_deps = deps.clone();
    let mut failures = 0i64;
    let mut transitions: Vec<Value> = Vec::new();
    for dep in new_deps.iter_mut() {
        if dep.get("state").and_then(|v| v.as_str()).unwrap_or("open") != "open" {
            continue;
        }
        let kind = dep
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let obs = match kind.as_str() {
            "task" => {
                let dep_id = dep.get("task_id").and_then(|v| v.as_i64()).unwrap_or(0);
                let st = get_task(t, dep_id)?
                    .and_then(|r| r.get("status").and_then(|v| v.as_str()).map(str::to_string));
                task_dep_obs(st.as_deref(), dep_id)
            }
            "issue" | "pr" | "ci" => {
                let key = dep_key(dep);
                if let Some(o) = cache.get(&key) {
                    o.clone()
                } else {
                    let repo = dep.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                    let num = |k: &str| dep.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
                    let o = match (t.gh_probe(), kind.as_str()) {
                        (Some(p), "issue") => p.probe_issue(repo, num("number")),
                        (Some(p), "pr") => p.probe_pr(repo, num("number")),
                        (Some(p), "ci") => p.probe_ci(repo, num("run_id")),
                        _ => {
                            DepObs::Failed("no github probe configured on this broker".to_string())
                        }
                    };
                    cache.insert(key, o.clone());
                    o
                }
            }
            _ => DepObs::Broken(format!("unknown dep kind '{kind}'")),
        };
        if matches!(obs, DepObs::Failed(_)) {
            failures += 1;
        }
        if let Some(tr) = apply_dep_obs(dep, &obs, &now) {
            transitions.push(json!({
                "dep": dep_key(dep), "transition": tr,
                "evidence": dep.get("last_observed").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    if deps_all_resolved(&new_deps) {
        let cleared = t.exec(
            "UPDATE tasks SET status = 'queued', blocked_deps = NULL, \
               dep_next_recheck_at = NULL, dep_blocked_until = NULL, \
               dep_recheck_seconds = NULL, updated_at = ? \
             WHERE id = ? AND status = 'blocked'",
            &[Arg::Text(now.clone()), Arg::Int(task_id)],
        )?;
        if cleared.affected == 0 {
            // Lost a race with a manual unblock or a re-block;
            // that transition owns the audit trail.
            return Ok(("still_blocked", failures));
        }
        emit_dep_transitions(t, task_id, row, &transitions)?;
        event(
            t,
            Some("dep-rechecker"),
            Some(task_id),
            "dep_unblocked",
            &with_marks(
                &json!({
                    "deps": new_deps, "transitions": transitions,
                    "via": "dep_recheck",
                }),
                row,
            ),
        )?;
        return Ok(("unblocked", failures));
    }
    t.exec(
        "UPDATE tasks SET blocked_deps = ?, dep_next_recheck_at = ?, updated_at = ? \
         WHERE id = ? AND status = 'blocked'",
        &[
            Arg::Text(json!(new_deps).to_string()),
            // The max TTL is a hard deadline: never schedule
            // the next recheck past dep_blocked_until, or expiry
            // would only be noticed one recheck too late. (ISO
            // strings compare lexicographically in this store.)
            Arg::Text({
                let next = iso(now_unix_v + recheck_seconds);
                match row.get("dep_blocked_until").and_then(|v| v.as_str()) {
                    Some(until) if until < next.as_str() => until.to_string(),
                    _ => next,
                }
            }),
            Arg::Text(now.clone()),
            Arg::Int(task_id),
        ],
    )?;
    emit_dep_transitions(t, task_id, row, &transitions)?;
    Ok(("still_blocked", failures))
}

/// The dep recheck pass (#8276): every dep-blocked task whose
/// recheck time has arrived, examined once, GitHub refs shared
/// across tasks through a per-pass cache. Driven by verb_sweep
/// (the fleet-queue-expire timer fires /sweep every minute, so
/// this runs with no lane awake), by /dep-recheck directly, and
/// per-task by wake.
pub fn run_dep_rechecks(t: &TursoBackend, limit: i64) -> Result<Value, TErr> {
    let now_unix_v = now_unix();
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks WHERE status = 'blocked' \
             AND blocked_deps IS NOT NULL \
             AND (dep_next_recheck_at IS NULL OR dep_next_recheck_at <= ?) \
             ORDER BY dep_next_recheck_at ASC LIMIT {limit}"
        ),
        &[Arg::Text(iso(now_unix_v))],
        &TASK_JSON_COLS,
    )?;
    let mut cache: std::collections::HashMap<String, DepObs> = std::collections::HashMap::new();
    let (mut checked, mut unblocked, mut ttl_expired, mut still_blocked, mut failures) =
        (0i64, 0i64, 0i64, 0i64, 0i64);
    for row in &rows {
        let (outcome, f) = dep_recheck_one(t, row, &mut cache, now_unix_v)?;
        checked += 1;
        failures += f;
        match outcome {
            "unblocked" => unblocked += 1,
            "ttl_expired" => ttl_expired += 1,
            _ => still_blocked += 1,
        }
    }
    Ok(json!({
        "checked": checked, "unblocked": unblocked,
        "ttl_expired": ttl_expired, "still_blocked": still_blocked,
        "check_failures": failures,
    }))
}

/// The #10648 quarantine recheck verdict for one row — the pure
/// decision the sweep driver and the /quarantine-recheck verb
/// share. A row is this pass's business only when it is blocked,
/// carries no dep record (dep-blocked rows belong to the dep
/// rechecker), and its recorded error is a claim-path quarantine:
/// exactly the fossil shape the 2026-11-16 blocked-pile audit
/// found — rows whose recorded validation error had stopped
/// reproducing (in 99 cases both max_attempts copies already
/// agreed again) yet sat blocked forever, because nothing ever
/// re-validated a blocked row.
#[derive(Debug, PartialEq)]
pub(crate) enum QuarantineRecheck {
    NotApplicable,
    /// The recorded quarantine error no longer reproduces: the row
    /// passes the claim guard today and must be requeued.
    Release,
    /// The row still fails the claim guard; carries the CURRENT
    /// reason, which may differ from the recorded one.
    StillQuarantined(String),
}

/// #10714: was this row blocked by a lane (verb_block) rather than by
/// the #5444 claim-path quarantine? verb_block records the blocker and
/// the `blocked_via: "lane"` marker in both progress and result;
/// quarantine_task records neither. Rows blocked by a lane before the
/// marker existed still carry `blocker`/`reason_class`, so the check
/// covers legacy rows too. The quarantine rechecker must never undo a
/// lane's block, even when a fossil quarantine last_error survives on
/// the row.
pub(crate) fn lane_blocked(row: &Value) -> bool {
    fn marks_lane(v: Option<&Value>) -> bool {
        let Some(o) = v.filter(|v| v.is_object()) else {
            return false;
        };
        o.get("blocked_via").and_then(|v| v.as_str()) == Some("lane")
            || o.get("blocker")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            || o.get("reason_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
    }
    marks_lane(row.get("progress")) || marks_lane(row.get("result"))
}

pub(crate) fn quarantine_recheck_verdict(row: &Value) -> QuarantineRecheck {
    if row.get("status").and_then(|v| v.as_str()) != Some("blocked") {
        return QuarantineRecheck::NotApplicable;
    }
    if dep_block_active(row) {
        return QuarantineRecheck::NotApplicable;
    }
    // #10714 defence in depth: a lane block owns the row. Its
    // last_error may be a fossil of a repaired #5444 quarantine.
    if lane_blocked(row) {
        return QuarantineRecheck::NotApplicable;
    }
    let recorded = row.get("last_error").and_then(|v| v.as_str()).unwrap_or("");
    if !recorded.starts_with("quarantined by claim path") {
        return QuarantineRecheck::NotApplicable;
    }
    match claim_guard_reason(row) {
        None => QuarantineRecheck::Release,
        Some(reason) => QuarantineRecheck::StillQuarantined(reason),
    }
}

// Snapshot the raw stored columns before query_objs parses JSON. Comparing
// this opaque string avoids lossy malformed-JSON normalization and detects
// same-second replacements, not only updated_at changes. The event ID is
// selected in the same SQLite statement; unknown block provenance fails closed.
// Conservatively require the latest event, including non-transition events: an
// unrelated newer event or pruned provenance needs explicit reconciliation, not
// automatic release. This patch does not solve sweep fairness/history repair.
pub(crate) fn quarantine_select() -> String {
    format!(
        "SELECT {TASK_COLS}, json_array({TASK_COLS}) AS quarantine_snapshot, \
         (SELECT MAX(id) FROM events WHERE task_id = tasks.id) AS quarantine_event_id \
         FROM tasks"
    )
}

pub(crate) fn quarantine_transition_sql() -> String {
    include_str!("../quarantine_transition.sql")
        .replace("{snapshot}", &format!("json_array({TASK_COLS})"))
}

pub(crate) fn quarantine_transition(
    t: &TursoBackend,
    row: &Value,
    status: &str,
    error: Option<&str>,
) -> Result<bool, TErr> {
    let Some(snapshot) = row.get("quarantine_snapshot").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(event_id) = row.get("quarantine_event_id").and_then(Value::as_i64) else {
        return Ok(false);
    };
    let Some(id) = row.get("id").and_then(Value::as_i64) else {
        return Ok(false);
    };
    let result = t.exec(
        &quarantine_transition_sql(),
        &[
            Arg::Text(status.to_string()),
            arg_str(error),
            Arg::Text(now_iso()),
            Arg::Int(id),
            Arg::Text(snapshot.to_string()),
            Arg::Int(event_id),
        ],
    )?;
    Ok(result.affected == 1)
}

/// Rotation touch for the quarantine recheck window (#9609): a
/// row the rechecker evaluated and left blocked without any
/// other write gets updated_at bumped — deliberately no event,
/// so the minutely sweep stays log-quiet — moving it to the back
/// of the recheck's ORDER BY updated_at ASC window. Without it,
/// any quarantine backlog larger than one pass's limit
/// fossilizes: the front rows are rechecked every pass and the
/// rows behind them are never reached.
pub(crate) fn touch_recheck_cursor(t: &TursoBackend, id: i64) -> Result<(), TErr> {
    t.exec(
        "UPDATE tasks SET updated_at = ? WHERE id = ? AND status = 'blocked'",
        &[Arg::Text(now_iso()), Arg::Int(id)],
    )?;
    Ok(())
}

/// Apply one row's quarantine recheck verdict. Returns the outcome
/// name ("released" / "still_quarantined" / "not_applicable" /
/// "release_raced"). Release mirrors verb_unblock's shape (queued,
/// lease-free) plus clearing the recorded error —
/// the rechecker's whole point is that the recorded error is
/// stale — and the event names the discarded error, like the dep
/// override names discarded deps. A still-quarantined row whose
/// CURRENT reason differs from the recorded one gets its
/// last_error refreshed (with an event); an unchanged reason
/// writes no event and no error refresh, so the minutely sweep
/// cannot spam the log — the only write is the rotation touch on
/// updated_at (#9609), which lets the sweep's window advance
/// past the row.
pub(crate) fn quarantine_recheck_one(t: &TursoBackend, row: &Value) -> Result<&'static str, TErr> {
    let id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let recorded = row
        .get("last_error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match quarantine_recheck_verdict(row) {
        QuarantineRecheck::NotApplicable => {
            touch_recheck_cursor(t, id)?;
            Ok("not_applicable")
        }
        QuarantineRecheck::Release => {
            if !quarantine_transition(t, row, "queued", None)? {
                // #10671 fairness: a guarded write that did not apply
                // leaves the row as it is, so it rotates like the other
                // no-write outcomes. Without the touch, rows whose
                // provenance or lease keeps failing the guard pin the
                // front of the window every pass.
                touch_recheck_cursor(t, id)?;
                return Ok("release_raced");
            }
            // No lease deletion: a quarantined row must be lease-free at
            // the atomic write. A subsequent claimant owns its new lease.
            event(
                t,
                Some("quarantine-rechecker"),
                Some(id),
                "unblocked",
                &json!({
                    "reason": "recorded claim-path quarantine no longer reproduces; packet passes the claim guard (#10648 recheck)",
                    "via": "quarantine_recheck",
                    "discarded_error": recorded,
                }),
            )?;
            Ok("released")
        }
        QuarantineRecheck::StillQuarantined(reason) => {
            if reason != recorded {
                if !quarantine_transition(t, row, "blocked", Some(&reason))? {
                    touch_recheck_cursor(t, id)?;
                    return Ok("release_raced");
                }
                event(
                    t,
                    Some("quarantine-rechecker"),
                    Some(id),
                    "blocked",
                    &json!({"reason": reason, "via": "quarantine_recheck"}),
                )?;
            } else {
                touch_recheck_cursor(t, id)?;
            }
            Ok("still_quarantined")
        }
    }
}

/// The #10648 quarantine recheck pass: every blocked, dep-free
/// row whose recorded error is a claim-path quarantine is re-run
/// through the claim guard; a row whose error no longer
/// reproduces is requeued automatically. Lane-blocked rows are
/// excluded (#10714): a lane block owns the row, and a fossil
/// quarantine error under it must not undo that block. Rides the
/// sweep beside the dep rechecker (same timer, same opt-out shape), and
/// /quarantine-recheck runs it directly. This is the blocked-row
/// re-validation the dep rechecker never provided: it watches
/// dep-blocked rows only, and a quarantine block carries no deps.
/// Rows a pass evaluates and leaves blocked without any other
/// write get a bare updated_at touch (quarantine_recheck_one), so
/// the ORDER BY window below rotates: every quarantined row is
/// reached within ceil(population / limit) passes instead of the
/// front rows being rechecked forever (#9609 — at 1,687
/// quarantined rows against the sweep's 200-row window, the same
/// 197 rows were rechecked every minute while releasable rows
/// behind them fossilized).
pub fn run_quarantine_rechecks(t: &TursoBackend, limit: i64) -> Result<Value, TErr> {
    let rows = t.query_objs(
        &format!(
            "{} WHERE status = 'blocked' \
             AND last_error LIKE 'quarantined by claim path%' \
             ORDER BY updated_at ASC LIMIT {limit}",
            quarantine_select()
        ),
        &[],
        &TASK_JSON_COLS,
    )?;
    let (mut checked, mut released, mut still_quarantined, mut raced) = (0i64, 0i64, 0i64, 0i64);
    for row in &rows {
        match quarantine_recheck_one(t, row)? {
            "released" => {
                checked += 1;
                released += 1;
            }
            "still_quarantined" => {
                checked += 1;
                still_quarantined += 1;
            }
            "release_raced" => raced += 1,
            _ => {}
        }
    }
    Ok(json!({
        "checked": checked, "released": released,
        "still_quarantined": still_quarantined, "raced": raced,
    }))
}

/// /quarantine-recheck (#10648): run the recheck pass, or — with
/// a task_id — force one blocked task's recheck right now. The
/// manual trigger lanes and the monitor use to prove the
/// machinery; production cadence is the sweep's.
pub(crate) fn verb_quarantine_recheck(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        if let Some(task_id) = req_i64(req, "task_id") {
            let Some(row) = t
                .query_objs(
                    &format!("{} WHERE id = ?", quarantine_select()),
                    &[Arg::Int(task_id)],
                    &TASK_JSON_COLS,
                )?
                .into_iter()
                .next()
            else {
                return Ok(json!({"task_id": task_id, "outcome": "no_such_task"}));
            };
            if row.get("status").and_then(|v| v.as_str()) != Some("blocked") {
                return Ok(json!({"task_id": task_id, "outcome": "not_blocked"}));
            }
            let outcome = quarantine_recheck_one(t, &row)?;
            let after = get_task(t, task_id)?.unwrap_or(Value::Null);
            return Ok(json!({
                "task_id": task_id, "outcome": outcome, "row": after,
            }));
        }
        run_quarantine_rechecks(t, req_i64(req, "limit").unwrap_or(200))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// /dep-recheck (#8276): run the due pass, or — with a task_id —
/// force one dep-blocked task's recheck right now, schedule or
/// no schedule. The manual trigger lanes and the monitor use to
/// prove the machinery; production cadence is the sweep's.
pub(crate) fn verb_dep_recheck(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        if let Some(task_id) = req_i64(req, "task_id") {
            let Some(row) = get_task(t, task_id)? else {
                return Ok(json!({"task_id": task_id, "outcome": "no_such_task"}));
            };
            if row.get("status").and_then(|v| v.as_str()) != Some("blocked")
                || !dep_block_active(&row)
            {
                return Ok(json!({"task_id": task_id, "outcome": "not_dep_blocked"}));
            }
            let mut cache = std::collections::HashMap::new();
            let (outcome, failures) = dep_recheck_one(t, &row, &mut cache, now_unix())?;
            let after = get_task(t, task_id)?.unwrap_or(Value::Null);
            return Ok(json!({
                "task_id": task_id, "outcome": outcome,
                "check_failures": failures, "row": after,
            }));
        }
        run_dep_rechecks(t, req_i64(req, "limit").unwrap_or(200))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// /unblock (#8276): the manual override. A lane or the monitor
/// releases a blocked task now — dep-blocked or classic — and
/// the event names who overrode and what was discarded. The
/// rechecker's automatic paths never impersonate this: an
/// override is a human/agent decision, audited as one.
pub(crate) fn verb_unblock(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id).map_err(|e| e)? else {
            return Err(format!("unblock_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if status != "blocked" {
            return Err(format!(
                "unblock_task: task {task_id} is {status}, not blocked"
            ));
        }
        let prev_deps = deps_of(&row);
        t.exec(
            "UPDATE tasks SET status = 'queued', blocked_deps = NULL, \
               dep_next_recheck_at = NULL, dep_blocked_until = NULL, \
               dep_recheck_seconds = NULL, updated_at = ? \
             WHERE id = ? AND status = 'blocked'",
            &[Arg::Text(now_iso()), Arg::Int(task_id)],
        )
        .map_err(|e| e)?;
        let (kind, detail) = if prev_deps.is_empty() {
            (
                "unblocked",
                json!({"reason": reason, "via": "unblock_task_turso"}),
            )
        } else {
            (
                "dep_unblock_override",
                json!({
                    "reason": reason, "discarded_deps": prev_deps,
                    "via": "unblock_task_turso",
                }),
            )
        };
        event(
            t,
            Some(&worker),
            Some(task_id),
            kind,
            &with_marks(&detail, &row),
        )
        .map_err(|e| e)?;
        get_task(t, task_id)
            .map(|r| r.unwrap_or(Value::Null))
            .map_err(|e| e)
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(5, format!("unblock_task: task {task_id} does not exist")),
        Err(e) if e.starts_with("unblock_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_cancel(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker")
        .unwrap_or("fleet-monitor")
        .to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, TErr> {
        let Some(row) = get_task(t, task_id)? else {
            return Ok(Value::Null);
        };
        if ["done", "cancelled", "failed"]
            .contains(&row.get("status").and_then(|v| v.as_str()).unwrap_or(""))
        {
            return Ok(row);
        }
        let result = merge_obj(
            &row.get("result").cloned().unwrap_or(json!({})),
            &json!({"cancel_reason": reason}),
        );
        t.exec(
            "UPDATE tasks SET status = 'cancelled', worker_id = NULL, lease_until = NULL, \
               result = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "cancelled",
            &json!({"reason": reason, "via": "cancel_repo_task_turso"}),
        )?;
        // #9046: close the binding window on the timeline (the
        // binding stays on the row up to this transition; the
        // event ends the window the route_attempt join slices by).
        if let Some(sid) = row
            .get("progress")
            .and_then(|p| p.get("session_id"))
            .and_then(|v| v.as_str())
        {
            record_session_unbound(t, Some(&worker), task_id, &row, sid, "cancel")?;
        }
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(
            5,
            format!("cancel_repo_task: task {task_id} does not exist"),
        ),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_fail(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let error = req_str(req, "error").unwrap_or("").to_string();
    let reason_class = req_str(req, "reason_class")
        .unwrap_or("transient")
        .to_string();
    let row = match claimed_by(t, task_id, &worker, "fail_repo_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    // Migration 0023 §2, faithfully: transient + attempts left
    // requeues with a backoff (900 s first, else 2700 s); 'blocked'
    // blocks; anything else fails terminally.
    let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
    let max_attempts = row
        .get("max_attempts")
        .and_then(|v| v.as_i64())
        .unwrap_or(crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64);
    let requeue = reason_class == "transient" && attempts < max_attempts;
    let new_status = if requeue {
        "queued"
    } else if reason_class == "blocked" {
        "blocked"
    } else {
        "failed"
    };
    let not_before = if requeue {
        after_iso(if attempts <= 1 { 900 } else { 2700 })
    } else {
        row.get("not_before")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let progress0 = row.get("progress").cloned().unwrap_or(json!({}));
    let (progress, cleared) = if requeue {
        clear_binding(&progress0, "fail_repo_task")
    } else {
        (progress0, None)
    };
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &json!({"reason_class": reason_class}),
    );
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET status = ?, worker_id = NULL, lease_until = NULL, \
               not_before = ?, last_error = ?, progress = ?, result = ?, updated_at = ? \
             WHERE id = ?",
            &[
                Arg::Text(new_status.to_string()),
                Arg::Text(not_before),
                Arg::Text(error.clone()),
                Arg::Text(progress.to_string()),
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        if requeue {
            if let Some(old) = &cleared {
                on_binding_cleared(t, &worker, task_id, old, "fail_repo_task")?;
            }
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "failed",
            &with_marks(
                &json!({
                    "error": error, "reason_class": reason_class, "requeued": requeue,
                    "via": "fail_repo_task_turso",
                }),
                &row,
            ),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        verb_with_execution(t, req, row)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Completion gating (Drew's triage doctrine, #7016; the
/// completion-without-evidence defect, 2026-11-07): what a task's
/// completion must carry depends on its class, derived from the
/// packet kind in its payload.
#[derive(Debug, PartialEq)]
pub(crate) enum CompletionClass {
    /// Work that produces an artifact: completion requires an
    /// evidence reference recorded on the task.
    Fix,
    /// Triage annotates and validates; it never completes the
    /// issue. Completion re-enqueues the follow-on by default.
    Triage,
    /// No packet kind (hand-built rows, legacy tasks): ungated, so
    /// the gate never breaks non-packet work.
    Ungated,
}

pub(crate) fn completion_class(payload: &Value) -> CompletionClass {
    match payload.get("kind").and_then(|k| k.as_str()) {
        Some("small_fix") | Some("ci_investigation") | Some("pr_shepherd") => CompletionClass::Fix,
        Some("triage") => CompletionClass::Triage,
        _ => CompletionClass::Ungated,
    }
}

/// The evidence reference on a completion result, if it carries
/// one: a merged PR, a commit, or a verified-closure note, under
/// any of the field names the fleet's completion notes use.
pub(crate) fn evidence_ref(result: &Value) -> Option<Value> {
    for key in [
        "evidence",
        "pr_url",
        "pr",
        "merged_pr",
        "commit",
        "commit_sha",
        "verified_closure",
    ] {
        match result.get(key) {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                return Some(json!({key: s}));
            }
            Some(Value::Array(a)) if !a.is_empty() => {
                return Some(json!({key: a}));
            }
            Some(v @ Value::Object(_)) => return Some(json!({key: v})),
            _ => {}
        }
    }
    None
}

/// What a triage completion does with its issue (doctrine exits:
/// invalid is the only no-evidence-free exit, and both exits still
/// require the validation evidence on the record).
#[derive(Debug, PartialEq)]
pub(crate) enum TriageOutcome {
    FollowOn,
    ExitInvalid,
    ExitAlreadySolved,
}

/// What the completion gate knows about the row being completed.
pub(crate) struct GateContext<'a> {
    /// The completing task's id.
    pub(crate) task_id: i64,
    /// The issue the packet is for, when it names one (#10611).
    pub(crate) expected_issue: Option<i64>,
    /// Whether a triage follow-on has a fix-family key to land on,
    /// so the broker itself guarantees a live successor.
    pub(crate) fix_follow_on: bool,
    /// Looks up a follow-up task id in the queue.
    pub(crate) lookup:
        &'a mut dyn FnMut(i64) -> Result<Option<crate::disposition::SuccessorTask>, String>,
}

/// The broker-side completion gate. Err = the completion is
/// rejected and the task stays claimed.
///
/// #10611: a terminal exit that claims the issue is finished must
/// carry the verified disposition readback (`result.disposition`),
/// so a shape-only evidence string can no longer end an item while
/// the issue is open and a linked draft still carries work. A
/// recorded follow-up task is checked against the queue: it must
/// exist, be live and be for the same issue.
pub(crate) fn gate_completion(
    class: &CompletionClass,
    ctx: &mut GateContext<'_>,
    result: &Value,
) -> Result<Option<TriageOutcome>, String> {
    let verified = |result: &Value, ctx: &mut GateContext<'_>| {
        crate::disposition::require_verified(
            result,
            ctx.expected_issue,
            now_unix(),
            ctx.task_id,
            ctx.lookup,
        )
    };
    match class {
        CompletionClass::Ungated => Ok(None),
        CompletionClass::Fix => {
            if evidence_ref(result).is_none() {
                return Err("complete_task: fix-class completion requires an evidence \
                     reference on the result (merged PR, commit, or \
                     verified-closure note); the task remains claimed"
                    .to_string());
            }
            verified(result, ctx)?;
            Ok(None)
        }
        CompletionClass::Triage => {
            let outcome = result
                .get("triage_outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("follow_on");
            match outcome {
                // A follow-on whose key has a fix counterpart lands
                // on a live fix task the broker finds or creates.
                // One without (the 15345 shape: `done` with outcome
                // `triaged`, no fix follow-on enqueued) would end an
                // issue's queue item with no live successor, so it
                // must record a verified handoff instead.
                "follow_on" if ctx.fix_follow_on || ctx.expected_issue.is_none() => {
                    Ok(Some(TriageOutcome::FollowOn))
                }
                "follow_on" => {
                    let record = verified(result, ctx).map_err(|e| {
                        format!(
                            "{e} (this triage key has no fix follow-on, so the \
                             handoff must be on the record)"
                        )
                    })?;
                    if record.issue_state == "open" && record.follow_up.is_none() {
                        return Err(format!(
                            "complete_task: triage of open issue #{} enqueues no fix \
                             follow-on for its key and records no follow_up; name the \
                             live task or open issue that owns the work (`repo-queue \
                             verify-disposition --follow-up-task|--follow-up-issue`); \
                             the task remains claimed",
                            record.issue_number
                        ));
                    }
                    Ok(Some(TriageOutcome::FollowOn))
                }
                "invalid" => {
                    // The validation note is the evidence for this
                    // exit; a bare verdict is not enough.
                    let noted = evidence_ref(result).is_some()
                        || matches!(
                            result.get("triage_note"),
                            Some(Value::String(s)) if !s.trim().is_empty()
                        );
                    if !noted {
                        return Err("complete_task: triage exit 'invalid' requires the \
                             validation evidence on the result (evidence or \
                             triage_note); the task remains claimed"
                            .to_string());
                    }
                    Ok(Some(TriageOutcome::ExitInvalid))
                }
                "already_solved" => {
                    if evidence_ref(result).is_none() {
                        return Err("complete_task: triage exit 'already_solved' requires \
                             an evidence reference on the result (merged PR, \
                             commit, or verified-closure note); the task \
                             remains claimed"
                            .to_string());
                    }
                    verified(result, ctx)?;
                    Ok(Some(TriageOutcome::ExitAlreadySolved))
                }
                other => Err(format!(
                    "complete_task: unknown triage_outcome {other:?} \
                     (follow_on | invalid | already_solved)"
                )),
            }
        }
    }
}

/// The issue a task is for: its packet's `issue.number`, else the
/// issue its repo-health dedupe key names under the packet's repo.
pub(crate) fn task_issue(payload: &Value, dedupe_key: &str) -> Option<i64> {
    payload
        .get("issue")
        .and_then(|i| i.get("number"))
        .and_then(|n| n.as_i64())
        .or_else(|| {
            let repo = payload.get("repo").and_then(|r| r.as_str())?;
            issue_number_for_repo(dedupe_key, repo)
        })
}

/// The follow-on fix key for a triage key: same family shape with
/// the family swapped triage -> fix. None for keys with no fix
/// counterpart (maintenance, pr families).
pub(crate) fn follow_on_key(triage_key: &str) -> Option<String> {
    if triage_key.contains(":triage:") && triage_key.contains(":issue:") {
        Some(triage_key.replacen(":triage:", ":fix:", 1))
    } else {
        None
    }
}

pub(crate) fn verb_complete(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "complete_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    // The completion gate runs BEFORE any mutation: a rejected
    // completion leaves the task claimed by its worker.
    let payload = row.get("payload").cloned().unwrap_or(json!({}));
    let class = completion_class(&payload);
    let gate_result = req.get("result").cloned().unwrap_or(json!({}));
    let dedupe_key = row
        .get("dedupe_key")
        .and_then(|k| k.as_str())
        .unwrap_or("")
        .to_string();
    // The issue a completion must match: the packet's own `issue`
    // object, else the repo-health dedupe key the broker already
    // trusts for coverage (#10611).
    let expected_issue = task_issue(&payload, &dedupe_key);
    let mut lookup = |id: i64| -> Result<Option<crate::disposition::SuccessorTask>, String> {
        let found = get_task(t, id).map_err(|e| format!("{e:?}"))?;
        Ok(found.map(|r| {
            let status = r
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let key = r.get("dedupe_key").and_then(|k| k.as_str()).unwrap_or("");
            let p = r.get("payload").cloned().unwrap_or(json!({}));
            crate::disposition::SuccessorTask {
                id,
                live: LIVE_STATUSES.contains(&status.as_str()),
                status,
                issue: task_issue(&p, key),
            }
        }))
    };
    let mut ctx = GateContext {
        task_id,
        expected_issue,
        fix_follow_on: follow_on_key(&dedupe_key).is_some(),
        lookup: &mut lookup,
    };
    let triage_outcome = match gate_completion(&class, &mut ctx, &gate_result) {
        Ok(o) => o,
        Err(e) => return fail(2, e),
    };
    // Terminal transitions keep the session binding (migration 0023
    // header): on a done row it is the execution record.
    let result = merge_obj(
        &row.get("result").cloned().unwrap_or(json!({})),
        &req.get("result").cloned().unwrap_or(json!({})),
    );
    let work = (|| -> Result<Value, TErr> {
        t.exec(
            "UPDATE tasks SET status = 'done', lease_until = NULL, result = ?, \
               completed_at = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(result.to_string()),
                Arg::Text(now_iso()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        touch_worker(t, &worker, None)?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "completed",
            &with_marks(&json!({"via": "complete_task_turso"}), &row),
        )?;
        // #9046: close the binding window on the timeline. The
        // binding itself stays on the row as the execution record
        // (migration 0023 header above); this event only ends the
        // window the route_attempt join slices by.
        if let Some(sid) = row
            .get("progress")
            .and_then(|p| p.get("session_id"))
            .and_then(|v| v.as_str())
        {
            record_session_unbound(t, Some(&worker), task_id, &row, sid, "complete")?;
        }
        // Triage doctrine (#7016): a triage completion is not the
        // issue being done — the issue goes back in the queue as a
        // fix follow-on unless triage exited it (invalid /
        // already_solved, gated above). The follow-on dedupes
        // against any live fix task for the same key; a terminal
        // predecessor links via progress like an enqueue revive.
        let mut follow_on_id: Option<i64> = None;
        if triage_outcome == Some(TriageOutcome::FollowOn) {
            let queue = row
                .get("queue")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let triage_key = row
                .get("dedupe_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match follow_on_key(&triage_key) {
                Some(fix_key) => {
                    let existing = t.query_objs(
                        &format!(
                            "SELECT {TASK_COLS} FROM tasks \
                             WHERE queue = ? AND dedupe_key = ? ORDER BY id ASC"
                        ),
                        &[Arg::Text(queue.clone()), Arg::Text(fix_key.clone())],
                        &TASK_JSON_COLS,
                    )?;
                    let (live, terminal) = dedupe_partition(&existing);
                    let fid = if let Some(live_row) = live {
                        live_row.get("id").and_then(|v| v.as_i64()).unwrap_or(0)
                    } else {
                        let mut fpayload = payload.clone();
                        if let Some(obj) = fpayload.as_object_mut() {
                            obj.insert("kind".to_string(), json!("small_fix"));
                            obj.insert(
                                "triage_follow_on".to_string(),
                                json!({"from_task": task_id, "at": now_iso()}),
                            );
                        }
                        let fprogress = match &terminal {
                            Some(pred) => json!({
                                "triage_source_task": task_id,
                                "revived_from": {
                                    "task_id": pred.get("id").cloned().unwrap_or(Value::Null),
                                    "status": pred.get("status").cloned().unwrap_or(Value::Null),
                                    "via": "triage_completion_follow_on",
                                },
                            }),
                            None => json!({"triage_source_task": task_id}),
                        };
                        let ins = t.exec(
                            "INSERT INTO tasks (queue, title, body, payload, dedupe_key, \
                               priority, status, max_attempts, required_budget_minutes, progress, result, \
                               created_at, updated_at) \
                             VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, '{}', ?, ?)",
                            &[
                                Arg::Text(queue.clone()),
                                arg_str(row.get("title").and_then(|v| v.as_str())),
                                arg_str(row.get("body").and_then(|v| v.as_str())),
                                Arg::Text(fpayload.to_string()),
                                Arg::Text(fix_key.clone()),
                                row.get("priority")
                                    .and_then(|v| v.as_i64())
                                    .map(Arg::Int)
                                    .unwrap_or(Arg::Null),
                                Arg::Int(
                                    fpayload
                                        .get("max_attempts")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(
                                            crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64,
                                        ),
                                ),
                                // #3418: fpayload clones the triage
                                // source payload, so an explicit
                                // requirement rides along here.
                                fpayload
                                    .get("required_budget_minutes")
                                    .and_then(|v| v.as_i64())
                                    .map(Arg::Int)
                                    .unwrap_or(Arg::Null),
                                Arg::Text(fprogress.to_string()),
                                Arg::Text(now_iso()),
                                Arg::Text(now_iso()),
                            ],
                        )?;
                        let new_id = ins.last_insert_rowid.unwrap_or(0);
                        event(
                            t,
                            Some(&worker),
                            Some(new_id),
                            "enqueued",
                            &json!({
                                "via": "triage_completion_follow_on",
                                "source_task": task_id, "dedupe_key": fix_key,
                            }),
                        )?;
                        new_id
                    };
                    follow_on_id = Some(fid);
                    event(
                        t,
                        Some(&worker),
                        Some(task_id),
                        "triage_follow_on_enqueued",
                        &json!({"follow_on_task_id": fid, "dedupe_key": fix_key}),
                    )?;
                }
                None => {
                    event(
                        t,
                        Some(&worker),
                        Some(task_id),
                        "triage_follow_on_skipped",
                        &json!({
                            "reason": "dedupe key has no fix-family counterpart",
                            "dedupe_key": triage_key,
                        }),
                    )?;
                }
            }
        }
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        let mut envelope = verb_with_execution(t, req, row)?;
        if let (Some(obj), Some(fid)) = (envelope.as_object_mut(), follow_on_id) {
            obj.insert("follow_on_task_id".to_string(), json!(fid));
        }
        Ok(envelope)
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_list(t: &TursoBackend, req: &Value) -> Value {
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    let limit = match crate::client::list_limit_or_default(req_i64(req, "limit")) {
        Ok(l) => l,
        Err(e) => return fail(2, e),
    };
    let rich = req.get("rich").and_then(|r| r.as_bool()).unwrap_or(false);
    let select = if rich {
        "id, queue, title, priority, status, dedupe_key, created_at, not_before, \
         worker_id, claimed_at, lease_until, lane_id, platform, attempts, max_attempts, \
         claim_provider, claim_model, claim_variant, progress, payload, \
         blocked_deps, dep_next_recheck_at, dep_blocked_until"
    } else {
        "id, queue, title, priority, status, dedupe_key, created_at, not_before, platform"
    };
    // #9420: the queue predicate applies only when a queue is named;
    // an unnamed queue must not become `queue = ''`, which by
    // construction returns zero rows and breaks exact-task readback
    // ({"task": id} / {"dedupe_key": key} with no queue).
    let mut conds = String::from(" WHERE 1 = 1");
    let mut args: Vec<Arg> = vec![];
    if !queue.is_empty() {
        conds += " AND queue = ?";
        args.push(Arg::Text(queue));
    }
    if let Some(s) = req_str(req, "status") {
        conds += " AND status = ?";
        args.push(Arg::Text(s.to_string()));
    }
    // #5444: recover a lane's or worker's live claims before lease
    // expiry — list filters end-to-end (client -> broker -> here).
    if let Some(l) = req_str(req, "lane") {
        conds += " AND lane_id = ?";
        args.push(Arg::Text(l.to_string()));
    }
    if let Some(w) = req_str(req, "worker") {
        conds += " AND worker_id = ?";
        args.push(Arg::Text(w.to_string()));
    }
    if let Some(nb) = req_str(req, "not_before") {
        conds += " AND (not_before IS NULL OR not_before <= ?)";
        args.push(Arg::Text(nb.to_string()));
    }
    // #7871: --task narrows to one task id (the PostgREST /list
    // path applies the same filter), and the client's paged fetch
    // advances OFFSET page by page.
    if let Some(id) = req_i64(req, "task") {
        conds += " AND id = ?";
        args.push(Arg::Int(id));
    }
    // #9420: dedupe_key narrows to the task carrying that key (the
    // delivery publisher's enqueue readback verifies by dedupe key).
    if let Some(d) = req_str(req, "dedupe_key") {
        conds += " AND dedupe_key = ?";
        args.push(Arg::Text(d.to_string()));
    }
    let offset = crate::client::list_offset_or_default(req_i64(req, "offset"));
    let sql = format!(
        "SELECT {select} FROM tasks{conds} ORDER BY (priority IS NULL), priority ASC, created_at ASC, id ASC LIMIT {limit} OFFSET {offset}"
    );
    let json_cols: &[&str] = if rich {
        &["progress", "payload", "blocked_deps"]
    } else {
        &[]
    };
    match t.query_objs(&sql, &args, json_cols) {
        Ok(rows) => {
            // #10376 parity with the PostgREST /list: name the
            // total matching count so a limited listing cannot
            // read as complete. A failed count leaves the total
            // unknown (null), never a fabricated number.
            let count_sql = format!("SELECT COUNT(*) AS n FROM tasks{conds}");
            let total = t
                .query_objs(&count_sql, &args, &[])
                .ok()
                .and_then(|rs| rs.first().and_then(|r| r.get("n")).and_then(|v| v.as_i64()));
            ok(crate::client::list_envelope(json!(rows), total, offset))
        }
        Err(e) => terr(e),
    }
}

/// One queue's health row with every counter zeroed. The client
/// renders the health table from these keys, so a field added here
/// surfaces in the health table output with no client change.
pub(crate) fn health_entry(queue: &str) -> Value {
    json!({
        "queue": queue, "queued": 0, "at_cap_queued": 0, "claimed": 0,
        "parked": 0, "malformed_parked": 0, "handed_off": 0, "blocked": 0, "dep_blocked": 0, "done": 0, "cancelled": 0,
        "failed": 0, "total": 0, "oldest_queued_at": Value::Null,
        "terminal_events_24h": 0, "terminal_events_7d": 0,
    })
}

/// Fold per-queue at-cap queued counts (#6528) into health entries:
/// tasks with status='queued' AND attempts >= max_attempts can never
/// be claimed again (the claim query requires attempts <
/// max_attempts), so a queue whose queued tasks are all at cap reads
/// as dead-in-queue — health must name that instead of letting it
/// look like ordinary backlog. Queues with no at-cap rows keep the
/// zero default from health_entry.
pub(crate) fn apply_at_cap_queued(by_queue: &mut Map<String, Value>, rows: &[Value]) {
    for r in rows {
        let q = r.get("queue").and_then(|v| v.as_str()).unwrap_or("");
        let n = r.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(obj) = by_queue.get_mut(q).and_then(|e| e.as_object_mut()) {
            obj.insert("at_cap_queued".to_string(), json!(n));
        }
    }
}

/// Fold per-queue dep-blocked counts (#8276) into health
/// entries: blocked tasks carrying a structured dep record are a
/// different operational animal from classic blocks (escalations
/// wait for a human; dep-blocks wait for the broker's rechecker),
/// so health names them separately.
pub(crate) fn apply_dep_blocked(by_queue: &mut Map<String, Value>, rows: &[Value]) {
    for r in rows {
        let q = r.get("queue").and_then(|v| v.as_str()).unwrap_or("");
        let n = r.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(obj) = by_queue.get_mut(q).and_then(|e| e.as_object_mut()) {
            obj.insert("dep_blocked".to_string(), json!(n));
        }
    }
}

/// The storage-layer not_before rule (#9179, closing the #6426
/// bypass): every value a verb stores in a row's not_before must
/// satisfy the same rule the client flag validator and the broker
/// dispatch gate apply — a non-empty RFC 3339 timestamp no more
/// than 366 days out. The dispatch gate covers requests that pass
/// through it; the verbs are the last line, because a stored
/// "3600" (delay-seconds semantics in the absolute field) sorts
/// after every real timestamp in the claim predicate's TEXT
/// comparison and the parked row can never lapse.
pub(crate) fn validate_stored_not_before(raw: &str) -> Result<(), String> {
    crate::validate::validate_not_before(&json!(raw), "not_before")
}

/// Fold per-queue malformed parked counts (#9179 hygiene) into
/// health entries: parked rows whose stored not_before fails the
/// rule above are invisible to the lapse path until woken by hand,
/// so health names them instead of letting them hide inside the
/// ordinary parked count. Rows arrive as {queue, not_before}.
pub(crate) fn apply_malformed_parked(by_queue: &mut Map<String, Value>, rows: &[Value]) {
    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in rows {
        let nb = r.get("not_before").and_then(|v| v.as_str()).unwrap_or("");
        if validate_stored_not_before(nb).is_err() {
            let q = r.get("queue").and_then(|v| v.as_str()).unwrap_or("");
            *counts.entry(q.to_string()).or_insert(0) += 1;
        }
    }
    for (q, n) in counts {
        if let Some(obj) = by_queue.get_mut(&q).and_then(|e| e.as_object_mut()) {
            obj.insert("malformed_parked".to_string(), json!(n));
        }
    }
}

pub(crate) fn verb_health(t: &TursoBackend, req: &Value) -> Value {
    let queue_filter = req_str(req, "queue").map(str::to_string);
    let work = (|| -> Result<Value, TErr> {
        let mut sql = "SELECT queue, status, COUNT(*) AS n, \
               MIN(CASE WHEN status = 'queued' THEN created_at END) AS oldest_queued_at \
             FROM tasks"
            .to_string();
        let mut args: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            sql += " WHERE queue = ?";
            args.push(Arg::Text(q.clone()));
        }
        sql += " GROUP BY queue, status ORDER BY queue, status";
        let rows = t.query_objs(&sql, &args, &[])?;
        let mut by_queue: Map<String, Value> = Map::new();
        for r in &rows {
            let q = r
                .get("queue")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entry = by_queue
                .entry(q.clone())
                .or_insert_with(|| health_entry(&q));
            let status = r
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let n = r.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
            if let Some(obj) = entry.as_object_mut() {
                if let Some(slot) = obj.get_mut(&status) {
                    *slot = json!(n);
                }
                let total = obj.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
                obj.insert("total".to_string(), json!(total + n));
                if status == "queued" {
                    obj.insert(
                        "oldest_queued_at".to_string(),
                        r.get("oldest_queued_at").cloned().unwrap_or(Value::Null),
                    );
                }
            }
        }
        // At-cap queued counts (#6528): the dead-in-queue alert
        // class. Separate query because the status GROUP BY above
        // cannot express the attempts >= max_attempts predicate.
        let mut asql = "SELECT queue, COUNT(*) AS n FROM tasks \
             WHERE status = 'queued' AND attempts >= max_attempts"
            .to_string();
        let mut aargs: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            asql += " AND queue = ?";
            aargs.push(Arg::Text(q.clone()));
        }
        asql += " GROUP BY queue";
        let arows = t.query_objs(&asql, &aargs, &[])?;
        apply_at_cap_queued(&mut by_queue, &arows);
        // Dep-blocked counts (#8276): same separate-query shape as
        // at-cap — the status GROUP BY cannot express "carries a
        // dep record".
        let mut dsql = "SELECT queue, COUNT(*) AS n FROM tasks \
             WHERE status = 'blocked' AND blocked_deps IS NOT NULL"
            .to_string();
        let mut dargs: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            dsql += " AND queue = ?";
            dargs.push(Arg::Text(q.clone()));
        }
        dsql += " GROUP BY queue";
        let drows = t.query_objs(&dsql, &dargs, &[])?;
        apply_dep_blocked(&mut by_queue, &drows);
        // Malformed parked rows (#9179 hygiene): fetched as raw
        // values because the failure test is the shared validator,
        // not a SQL predicate; parked counts are small.
        let mut hsql = "SELECT queue, not_before FROM tasks \
             WHERE status = 'parked' AND not_before IS NOT NULL"
            .to_string();
        let mut hargs: Vec<Arg> = Vec::new();
        if let Some(q) = &queue_filter {
            hsql += " AND queue = ?";
            hargs.push(Arg::Text(q.clone()));
        }
        let hrows = t.query_objs(&hsql, &hargs, &[])?;
        apply_malformed_parked(&mut by_queue, &hrows);
        // Terminal-event counters, the agent.repo_queue_health
        // view's terminal_events_24h/7d (#5690): events and tasks
        // share this database, so the JOIN the PG view did is a
        // local query here — no cross-database problem.
        let mut tsql = "SELECT t.queue AS queue, COUNT(*) AS n7, \
               SUM(CASE WHEN e.created_at > ? THEN 1 ELSE 0 END) AS n24 \
             FROM events e JOIN tasks t ON t.id = e.task_id \
             WHERE e.kind IN ('completed', 'failed') AND e.created_at > ?"
            .to_string();
        let mut targs: Vec<Arg> = vec![
            Arg::Text(iso(now_unix() - 86_400)),
            Arg::Text(iso(now_unix() - 7 * 86_400)),
        ];
        if let Some(q) = &queue_filter {
            tsql += " AND t.queue = ?";
            targs.push(Arg::Text(q.clone()));
        }
        tsql += " GROUP BY t.queue";
        let trows = t.query_objs(&tsql, &targs, &[])?;
        for r in &trows {
            let q = r
                .get("queue")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let entry = by_queue
                .entry(q.clone())
                .or_insert_with(|| health_entry(&q));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "terminal_events_24h".to_string(),
                    r.get("n24").cloned().unwrap_or(json!(0)),
                );
                obj.insert(
                    "terminal_events_7d".to_string(),
                    r.get("n7").cloned().unwrap_or(json!(0)),
                );
            }
        }
        Ok(Value::Array(by_queue.into_iter().map(|(_, v)| v).collect()))
    })();
    match work {
        Ok(rows) => {
            let netloc = t
                .url()
                .split("://")
                .nth(1)
                .unwrap_or(t.url())
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
            ok(json!({
                "project_netloc": netloc,
                "project_source": "turso",
                "backend": "turso",
                "rows": rows,
            }))
        }
        Err(e) => terr(e),
    }
}

// ---------------------------------------------------------------------------
// Merge-intent stack storage (#8582 increment 2) — broker endpoints
// over the merge_intents table (DDL of record:
// crate::merge_intent::MERGE_INTENT_DDL, applied via SCHEMA above).
// Record/list came first; the transition verb (increment 4) moves
// intents through the core's lifecycle for the replay executor. The
// append rules themselves (position, base tip) are the pure core's
// (plan_append), so storage cannot drift from replay semantics, and
// a stack's full state — branches, order, owners — reads back from
// this database alone while GitHub is down.
// ---------------------------------------------------------------------------
