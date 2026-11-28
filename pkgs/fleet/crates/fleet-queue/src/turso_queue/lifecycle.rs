//! turso_queue::lifecycle — lane assignment and transfer, claim verbs, heartbeat, checkpoint, park, release, wake.
//!
//! Split from turso_queue.rs for #8589 (F11.10 production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

/// Change a live task's priority without changing any lifecycle/claim
/// fields. The row priority is authoritative; the packet mirror is
/// updated in the same SQL statement when it exists, and the event
/// records the snapshot used by the compare-and-swap guard.
pub(crate) fn verb_reprioritize(t: &TursoBackend, req: &Value) -> Value {
    let id = req_i64(req, "task_id")
        .or_else(|| req_i64(req, "id"))
        .unwrap_or(0);
    // Authenticated caller identity (#14100: the event's actor): the
    // worker id presented over the broker's authenticated channel,
    // exactly like the other lifecycle verbs. A legacy `actor`
    // string is honored only when no worker is present (old
    // clients); it is never trusted over the worker.
    let actor = req_str(req, "worker")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| req_str(req, "actor"))
        .unwrap_or("fleet-queue-cli");
    let reason = req_str(req, "reason")
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let raw = req.get("priority").cloned().unwrap_or(Value::Null);
    let to = match crate::validate::resolve_priority(&raw, "priority") {
        Ok(n) => n,
        Err(e) => return fail(2, e),
    };
    let work = (|| -> Result<Value, String> {
        let row = get_task(t, id)
            .map_err(|e| e)?
            .ok_or_else(|| format!("reprioritize_task: task {id} does not exist"))?;
        let status = row.get("status").and_then(Value::as_str).unwrap_or("");
        if !["queued", "claimed", "parked", "blocked"].contains(&status) {
            return Err(format!(
                "reprioritize_task: task {id} is terminal ({status})"
            ));
        }
        let from = row.get("priority").and_then(Value::as_i64).unwrap_or(90);
        if from == to {
            return Ok(row);
        }
        let payload = row.get("payload").cloned().unwrap_or(json!({}));
        let payload_text = if payload.get("priority").is_some() {
            let mut p = payload;
            p["priority"] = json!(to);
            Some(p.to_string())
        } else {
            None
        };
        let category = crate::validate::category_for_priority(to).unwrap_or("unknown");
        let issue = row
            .get("payload")
            .and_then(|p| p.get("issue"))
            .cloned()
            .unwrap_or(Value::Null);
        let repo = row
            .get("payload")
            .and_then(|p| p.get("repo"))
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty());
        let issue_number = issue
            .get("number")
            .and_then(Value::as_i64)
            .filter(|n| *n > 0);
        // Queue-only tasks (no linked issue) note that explicitly;
        // linked tasks note the sync intent the broker will fulfill.
        let sync_note = if repo.is_some() && issue_number.is_some() {
            "github-pending"
        } else {
            "no issue linked"
        };
        let detail = with_marks(
            &json!({"from": from, "to": to, "category": category, "issue": issue, "actor": actor, "reason": reason, "sync": sync_note}),
            &row,
        );
        // One Turso pipeline transaction couples the guarded task
        // mutation, packet mirror, and history event. The conditional
        // event insert is gated by changes() from the CAS UPDATE.
        let update = t.exec_batch(&[
            ("BEGIN IMMEDIATE", vec![]),
            ("UPDATE tasks SET priority = ?, payload = CASE WHEN ? IS NULL THEN payload ELSE ? END, progress = CASE WHEN ? IS NOT NULL AND ? > 0 THEN json_set(COALESCE(progress, '{}'), '$.reprioritize_sync_pending', true) ELSE progress END, updated_at = ? WHERE id = ? AND priority = ? AND status IN ('queued','claimed','parked','blocked')", vec![Arg::Int(to), payload_text.clone().map(Arg::Text).unwrap_or(Arg::Null), payload_text.map(Arg::Text).unwrap_or(Arg::Null), repo.map(|s| Arg::Text(s.to_owned())).unwrap_or(Arg::Null), issue_number.map(Arg::Int).unwrap_or(Arg::Null), Arg::Text(now_iso()), Arg::Int(id), Arg::Int(from)]),
            ("INSERT INTO events (worker_id, task_id, kind, detail, created_at) SELECT ?, ?, 'reprioritized', ?, ? WHERE changes() = 1", vec![Arg::Text(actor.to_string()), Arg::Int(id), Arg::Text(detail.to_string()), Arg::Text(now_iso())]),
            (HISTORY_INSERT_SQL, vec![Arg::Null, Arg::Null]),
            // Persist the external side-effect intent in the same
            // transaction. Queue-only tasks have no linked issue and
            // intentionally produce no sync row.
            ("INSERT INTO reprioritize_sync_outbox (task_id, repo, issue, category, label, state, attempts, available_at, created_at, updated_at) SELECT ?, ?, ?, ?, ?, 'pending', 0, ?, ?, ? WHERE changes() = 1 AND ? IS NOT NULL AND ? > 0", vec![Arg::Int(id), repo.map(|s| Arg::Text(s.to_owned())).unwrap_or(Arg::Null), issue_number.map(Arg::Int).unwrap_or(Arg::Null), Arg::Text(category.to_owned()), Arg::Text(crate::validate::label_for_priority(to).unwrap_or("priority/unknown").to_owned()), Arg::Text(now_iso()), Arg::Text(now_iso()), Arg::Text(now_iso()), repo.map(|s| Arg::Text(s.to_owned())).unwrap_or(Arg::Null), issue_number.map(Arg::Int).unwrap_or(Arg::Null)]),
            ("COMMIT", vec![]),
        ]).map_err(|e| e)?;
        if update.get(1).map(|r| r.affected).unwrap_or(0) != 1 {
            return Err(format!(
                "reprioritize_task: task {id} priority changed concurrently; retry"
            ));
        }
        get_task(t, id)
            .map_err(|e| e)?
            .ok_or_else(|| format!("reprioritize_task: task {id} disappeared"))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) => fail(2, e),
    }
}

/// The session a lane row is *live*-bound to (#8368): the ledger
/// session_id, unless the broker itself cleared that exact binding
/// (progress.session_cleared.session_id, stamped by clear_binding
/// when park/release/expire dropped it). A broker-cleared session is
/// not bound in any live sense — a row still carrying its id must
/// behave as unbound for reassignment, or the task loops claim/park
/// across lanes with no lane-side exit (no flip, no transfer, no
/// fresh launch).
pub(crate) fn bound_session<'a>(
    session_id: Option<&'a str>,
    cleared_session_id: Option<&str>,
) -> Option<&'a str> {
    let sid = session_id.map(str::trim).unwrap_or("");
    if sid.is_empty() || cleared_session_id == Some(sid) {
        None
    } else {
        session_id
    }
}

/// The session id the broker recorded as cleared on a task's
/// progress (clear_binding's session_cleared stamp), if any (#8368).
pub(crate) fn cleared_session_id(task: &Value) -> Option<&str> {
    task.get("progress")
        .and_then(|p| p.get("session_cleared"))
        .and_then(|c| c.get("session_id"))
        .and_then(|v| v.as_str())
}

// Lane-assignment flips, platform limitations, and transfer guards live in
// lifecycle_limits.rs (split for #16255); re-exported through turso_queue.

/// The claim preamble (#8217: shared by the single claim and
/// the batch claim): validate the caller's lane against the
/// registry, sweep expired leases, and quarantine payload-less
/// rows in this queue before any pick runs.
pub(crate) fn claim_preamble(
    t: &TursoBackend,
    queue: Option<&str>,
    worker: &str,
    lane: &Option<String>,
) -> Result<(), TErr> {
    if let Some(lane_id) = &lane {
        let lanes = t.query_objs(
            "SELECT lane_id, worker_id, queue, enabled FROM lanes WHERE lane_id = ?",
            &[Arg::Text(lane_id.clone())],
            &[],
        )?;
        let l = match lanes.into_iter().next() {
            Some(l) => l,
            None => {
                // Auto-registration (Drew, 2026-11-22; #12544): the
                // lane registry is not a gate. A claim naming a
                // lane with no registry row registers the lane on
                // the spot -- owned by the claiming worker, enabled,
                // unbound queue -- and the claim proceeds through
                // the same checks below. Any name may register: an
                // agent name ("archer"), a bare family name, a name
                // in no known family at all. An existing row always
                // takes precedence: a lane disabled by hand keeps
                // refusing (an explicit admin act, not an unknown
                // name), and a lane owned by another worker keeps
                // its owner. This supersedes the 2026-11-16
                // known-family wildcard (#6853), which provisioned
                // only <family>-<suffix> ids in LANE_FAMILIES and
                // refused everything else as "unknown lane_id".
                t.exec(
                    lane_provision_sql(),
                    &[Arg::Text(lane_id.clone()), Arg::Text(worker.to_string())],
                )?;
                let lanes = t.query_objs(
                    "SELECT lane_id, worker_id, queue, enabled FROM lanes WHERE lane_id = ?",
                    &[Arg::Text(lane_id.clone())],
                    &[],
                )?;
                match lanes.into_iter().next() {
                    Some(l) => l,
                    None => {
                        return Err(format!(
                            "claim_repo_task: lane {lane_id} registration did not land"
                        ));
                    }
                }
            }
        };
        if l.get("worker_id").and_then(|v| v.as_str()) != Some(worker) {
            return Err(format!(
                "claim_repo_task: lane {lane_id} belongs to worker {}, not {worker}",
                l.get("worker_id").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
        if l.get("enabled").and_then(|v| v.as_i64()) == Some(0) {
            return Err(format!("claim_repo_task: lane {lane_id} is disabled"));
        }
        // One-queue model (#8097, 2026-11-10): the registration's
        // queue binding is enforced ONLY when the claim itself is
        // stream-scoped AND the registration binds a stream. An
        // unscoped claim (the default) is never refused over the
        // lane's registration — identity is recorded, never used
        // to filter — and a registration carrying an unbound
        // marker ("*", see lane_queue_binds) binds nothing.
        if let Some(q) = queue {
            let registered = l.get("queue").and_then(|v| v.as_str()).unwrap_or("");
            if lane_queue_binds(registered) && registered != q {
                return Err(format!(
                    "claim_repo_task: lane {lane_id} is for queue {registered}, not {q}"
                ));
            }
        }
    }
    expire_leases(t)?;
    match queue {
        Some(q) => quarantine_payload_missing(t, q)?,
        None => quarantine_payload_missing_all(t)?,
    }
    Ok(())
}

pub(crate) fn claim_pick_once(
    t: &TursoBackend,
    scope: &ClaimScope,
    worker: String,
    lane: &Option<String>,
    lease_seconds: i64,
    family: Option<String>,
    claim_session: Option<String>,
    resume_fallback: Option<String>,
    model: &Option<ClaimModel>,
    handoff_pick: bool,
    remaining_budget_minutes: Option<i64>,
) -> Result<Option<Value>, TErr> {
    let now_unix = now_unix();
    let now = iso(now_unix);
    let lease_until = claim_lease_until(now_unix, lease_seconds);
    // One conditional UPDATE = the claim. The subquery picks the
    // same row PG's SKIP LOCKED pick would (priority, then age;
    // parked rows whose not_before has passed resume without an
    // attempts increment); the outer status re-check keeps the
    // statement atomic against a competing claim.
    // #5444: the pick loops past rows the guard quarantines —
    // a skipped row degrades to the next candidate or to no
    // claim, never to a handed-out invalid packet.
    // #9609: race losses and quarantines carry SEPARATE budgets.
    // The race budget bounds contention retries; the quarantine
    // budget bounds how much defective head-of-queue one call
    // chews through. They shared one 32-cap until 2026-11-16,
    // when a ~460-packet defective intake wave made every claim
    // call spend its whole budget quarantining and return "no
    // claimable task" while claimable rows sat behind the wave.
    // #11333: the bundle cap. An ordinary pick by a launch
    // that has already claimed BUNDLE_CLAIM_CAP tasks is
    // refused before any pick work — the cap lives here, in
    // the broker, not in the claiming lane's prompt. Handoff
    // picks are the claimant's sink collection and are exempt
    // (their claimed events also stay out of the count:
    // pick="handed_off").
    if !handoff_pick {
        bundle_cap_gate(t, lane, &claim_session)?;
    }
    let mut row: Value;
    let mut id: i64;
    let mut skipped = 0usize;
    let mut quarantined = 0usize;
    let mut retired = 0usize;
    loop {
        // #7617: pick the candidate id FIRST, then claim THAT row by
        // id, and read it back by id. The previous shape claimed via
        // a subquery and re-read the result as "the newest claimed
        // row for (queue, worker_id)" — but every lane shares
        // worker_id repo-worker-code, so under concurrent claims
        // that re-read could return ANOTHER lane's just-claimed
        // task: the caller was handed a row stamped with a foreign
        // lane_id (live repro 2026-11-08: lane code-u claims
        // returned tasks stamped code-e and code-t), it checkpointed
        // and parked a task it did not own, and the guard below
        // could quarantine a live foreign claim. The conditional
        // UPDATE by id stays atomic: it re-checks the full pick
        // eligibility (status, attempts cap, not_before) at claim
        // instant, so if a competing claim takes the picked row
        // first — or it is parked into the future in between — this
        // UPDATE affects 0 rows and the loop re-picks.
        // #8097: the pick predicate is built per pick mode.
        // #8187: NEITHER mode filters by platform — the mark a
        // claim declares (family) is stamped by the claim
        // UPDATE, never matched against the item. The default mode keeps the ordinary predicate
        // (queued rows due; parked rows whose timer has passed;
        // sink rows by their own branch).
        // Handoff mode picks the next sink row (HANDOFF_SQL:
        // status 'handed_off', #8234) with no caller-named id
        // and no timer gating — Rule 1's default collection
        // path for the claimant.
        let pick_predicate = if handoff_pick {
            handoff_pick_predicate()
        } else {
            default_pick_predicate()
        };
        let pick_sql = format!(
            "SELECT id FROM tasks WHERE 1=1 \
           AND payload IS NOT NULL AND payload <> '' AND payload <> '{{}}' \
           AND ( {pick_predicate} ) {PLATFORM_LIMIT_PICK_CLAUSE}{} \
         ORDER BY (priority IS NULL), priority ASC, created_at ASC LIMIT 1",
            scope.sql
        );
        let budget_arg = match remaining_budget_minutes {
            Some(v) => Arg::Int(v),
            None => Arg::Null,
        };
        let mut pick_args: Vec<Arg> = Vec::new();
        if !handoff_pick {
            pick_args.push(Arg::Text(now.clone()));
            pick_args.push(Arg::Text(now.clone()));
            pick_args.push(budget_arg.clone());
            pick_args.push(budget_arg.clone());
        }
        pick_args.push(arg_str(family.as_deref()));
        pick_args.push(arg_str(family.as_deref()));
        for a in &scope.args {
            pick_args.push(a.clone());
        }
        let pick = t.query_objs(&pick_sql, &pick_args, &[])?;
        let Some(picked) = pick.into_iter().next() else {
            return Ok(None);
        };
        let pick_id = picked.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let upd_sql = format!("{}{}", claim_update_sql(&pick_predicate), scope.sql);
        let mut upd_args: Vec<Arg> = vec![
            Arg::Text(worker.clone()),
            Arg::Text(now.clone()),
            Arg::Text(lease_until.clone()),
            arg_str(lane.as_deref()),
            arg_str(family.as_deref()),
            Arg::Text(now.clone()),
            Arg::Int(pick_id),
        ];
        if !handoff_pick {
            upd_args.push(Arg::Text(now.clone()));
            upd_args.push(Arg::Text(now.clone()));
            upd_args.push(budget_arg.clone());
            upd_args.push(budget_arg.clone());
        }
        upd_args.push(arg_str(family.as_deref()));
        upd_args.push(arg_str(family.as_deref()));
        for a in &scope.args {
            upd_args.push(a.clone());
        }
        let upd = t.exec(&upd_sql, &upd_args)?;
        if upd.affected == 0 {
            // Lost the race for the picked row; re-pick.
            skipped += 1;
            if skipped >= 32 {
                return Ok(None);
            }
            continue;
        }
        let rows = t.query_objs(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
            &[Arg::Int(pick_id)],
            &TASK_JSON_COLS,
        )?;
        let Some(candidate) = rows.into_iter().next() else {
            return Ok(None);
        };
        if let Some(reason) = claim_guard_reason(&candidate) {
            let cid = candidate.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            quarantine_task(t, cid, &reason)?;
            quarantined += 1;
            if quarantined >= MAX_CLAIM_QUARANTINES_PER_CALL {
                return Ok(None);
            }
            continue;
        }
        // #10626: a default pick whose linked issue already closed
        // (or PR already merged) is retired here as already_done,
        // before any lease, binding or claimed event, and the pick
        // moves on. Fails open: no probe, no linkage or a failed
        // lookup claims the row exactly as before.
        if !handoff_pick {
            if let Some(stale) = stale_pick(t, &candidate) {
                retire_stale(t, &worker, &candidate, &stale, "claim_staleness_check")?;
                retired += 1;
                if retired >= MAX_STALENESS_RETIRES_PER_CALL {
                    return Ok(None);
                }
                continue;
            }
        }
        id = candidate.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        row = candidate;
        break;
    }
    let mut row = row;
    // Migration 0023 §4: a fresh claim never inherits a binding.
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let (progress, cleared) = clear_binding(&progress, "claim_repo_task");
    // #9046: a pick that names its session binds it — previously
    // the session lived only in the claimed event's detail, so no
    // binding existed for later audit marks or the timeline.
    let progress = match claim_session.as_deref() {
        Some(sid) => merge_obj(
            &progress,
            &json!({"session_id": sid, "launched_by": worker}),
        ),
        None => progress,
    };
    if cleared.is_some() || claim_session.is_some() {
        t.exec(
            "UPDATE tasks SET progress = ? WHERE id = ?",
            &[Arg::Text(progress.to_string()), Arg::Int(id)],
        )?;
        if let Some(obj) = row.as_object_mut() {
            obj.insert("progress".to_string(), progress);
        }
    }
    t.exec(
        "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id, \
           lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at",
        &[
            Arg::Int(id),
            Arg::Text(worker.clone()),
            Arg::Text(lease_until),
            Arg::Text(now_iso()),
        ],
    )?;
    if let Some(sid) = claim_session.as_deref() {
        record_session_bound(t, Some(&worker), id, &row, sid, "claim")?;
    }
    stamp_claim_model(
        t,
        id,
        &mut row,
        model,
        Some(&worker),
        claim_session.as_deref(),
        "claim",
    )?;
    touch_worker(t, &worker, Some(id))?;
    if let Some(old) = &cleared {
        on_binding_cleared(t, &worker, id, old, "claim_repo_task")?;
    }
    let mut claim_detail = json!({
        "lease_seconds": lease_seconds, "via": "claim_repo_task_turso",
        "cleared_session_id": cleared, "lane_id": lane,
        "platform": family, "session_id": claim_session,
        "model_provider": model.as_ref().map(|m| m.provider.clone()),
        "model_id": model.as_ref().and_then(|m| m.model.clone()),
        "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
        "pick": if handoff_pick { "handed_off" } else { "default" },
    });
    // #9178 §1 (FDR-0001), claim-wire slice: same requested/actual
    // pair as the by-id path, plus the fallback reason (explicit
    // null when the claim names none); unhinted picks stay
    // byte-identical.
    if let Some(requested) = row.get("resume_session_id").and_then(|v| v.as_str()) {
        claim_detail["resume_requested_session_id"] = json!(requested);
        claim_detail["resume_actual_session_id"] = claim_session
            .as_deref()
            .map(|s| json!(s))
            .unwrap_or(Value::Null);
        claim_detail["resume_fallback"] = resume_fallback
            .as_deref()
            .map(|f| json!(f))
            .unwrap_or(Value::Null);
    }
    event(
        t,
        Some(&worker),
        Some(id),
        "claimed",
        &with_marks(&claim_detail, &row),
    )?;
    Ok(Some(row))
}

/// The most rows one claim call may quarantine before giving up
/// (#9609). Quarantines do not consume the claim loop's race-loss
/// budget: the two bound different things — the race budget bounds
/// contention retries, this one bounds how much defective
/// head-of-queue a single call chews through. The shared 32-cap
/// let the 2026-11-16 defective intake wave (~460 packets in one
/// queue) starve the stream for a shift: every call burned its
/// budget on quarantines and returned no claim. 512 absorbs a
/// wave of that size in one call while keeping one call's write
/// load bounded; a larger wave still drains across calls and via
/// the quarantine rechecker's releases.
pub const MAX_CLAIM_QUARANTINES_PER_CALL: usize = 512;

/// The batch-claim bound (#8217) is defined at the crate root so
/// the client surface validates `--count` against the same value
/// the broker enforces here; re-exported so this module's parse
/// path and the `turso_queue::BATCH_CLAIM_MAX` path keep working.
pub use crate::BATCH_CLAIM_MAX;
pub use crate::BUNDLE_CLAIM_CAP;

/// Parse a claim request's batch count (#8217): absent means the
/// single claim (1); anything outside 1..=BATCH_CLAIM_MAX is a
/// usage error, never a silent clamp.
pub(crate) fn parse_batch_count(req: &Value) -> Result<i64, String> {
    match req_i64(req, "count") {
        None => Ok(1),
        Some(n) if (1..=BATCH_CLAIM_MAX).contains(&n) => Ok(n),
        Some(n) => Err(format!(
            "count {n} is out of range: a batch claim takes              1..={BATCH_CLAIM_MAX} items (#8217)"
        )),
    }
}

/// The bundle cap's counting query (#11333): how many ordinary
/// claims one launch — a (lane, session) pair — has already
/// made. The oracle is the events table: the pick path writes
/// exactly one 'claimed' event per successful pick, stamped
/// with the claim's lane and session marks and pick="default".
/// The pick mark is what keeps the exempt paths out of the
/// count: handoff picks (the claimant's sink collection) write
/// pick="handed_off", and by-id / transfer claims write no pick
/// mark at all (by_id=true; transfers via
/// claim_repo_task_transfer), so none of them can inflate a
/// launch's bundle. Events are retained 30 days
/// (prune_events), far beyond any launch's lifetime, so the
/// count is complete for every live launch.
pub(crate) const BUNDLE_CLAIM_COUNT_SQL: &str = "SELECT COUNT(*) AS n FROM events WHERE kind = 'claimed' \
     AND json_extract(detail, '$.pick') = 'default' \
     AND json_extract(detail, '$.lane_id') = ? \
     AND json_extract(detail, '$.session_id') = ?";

/// Ordinary claims already made by this (lane, session) launch
/// (#11333). This is the broker's claim count for the launch —
/// the number bundle_cap_refusal judges.
pub(crate) fn bundle_claim_count(t: &TursoBackend, lane: &str, session: &str) -> Result<i64, TErr> {
    let rows = t.query_objs(
        BUNDLE_CLAIM_COUNT_SQL,
        &[Arg::Text(lane.to_string()), Arg::Text(session.to_string())],
        &[],
    )?;
    Ok(rows
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0))
}

/// The bundle-cap decision (#11333), pure so the boundary is
/// unit-testable: at or past BUNDLE_CLAIM_CAP the next ordinary
/// claim is refused with a message naming the cap, the launch
/// and the count; below it there is no refusal. Reset
/// semantics: the count is keyed by session, so a new session
/// (a new launch) starts a fresh bundle — nothing else resets
/// it, and completing or releasing a task does not refund a
/// claim: the cap budgets a launch's claims, not its
/// concurrent holdings.
pub(crate) fn bundle_cap_refusal(claimed: i64, lane: &str, session: &str) -> Option<String> {
    if claimed >= BUNDLE_CLAIM_CAP {
        Some(format!(
            "claim_repo_task: bundle cap reached: lane {lane} session {session} \
             has already claimed {claimed} tasks in this launch; the bundle \
             cap is {BUNDLE_CLAIM_CAP} (#11333) — start a new session to \
             claim a new bundle"
        ))
    } else {
        None
    }
}

/// The bundle-cap gate for one ordinary pick (#11333). Only a
/// claim carrying BOTH a lane and a session names a launch, so
/// only it is capped: a sessionless claim (an interactive or
/// agent-next claim) has no bundle to budget. Exempt paths
/// never reach this gate — by-id and transfer claims do not
/// pass through the pick path at all, and the caller skips
/// handoff picks.
pub(crate) fn bundle_cap_gate(
    t: &TursoBackend,
    lane: &Option<String>,
    claim_session: &Option<String>,
) -> Result<(), TErr> {
    if let (Some(lane_id), Some(sid)) = (lane.as_deref(), claim_session.as_deref()) {
        let claimed = bundle_claim_count(t, lane_id, sid)?;
        if let Some(msg) = bundle_cap_refusal(claimed, lane_id, sid) {
            return Err(msg);
        }
    }
    Ok(())
}

/// Batch claim (#8217): the claim preamble once, then the
/// identical atomic pick up to `count` times — same eligibility,
/// same priority/age order, same attempts/lease semantics and
/// per-item claimed events as that many single claims. A partial
/// batch is normal: the loop stops at the first no-task pick and
/// returns what exists.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verb_claim_batch(
    t: &TursoBackend,
    queue: Option<&str>,
    scope: &ClaimScope,
    worker: &str,
    lane: &Option<String>,
    lease_seconds: i64,
    family: &Option<String>,
    claim_session: &Option<String>,
    model: &Option<ClaimModel>,
    handoff_pick: bool,
    remaining_budget_minutes: Option<i64>,
    count: i64,
) -> Value {
    let work = (|| -> Result<Vec<Value>, TErr> {
        claim_preamble(t, queue, worker, lane)?;
        // #11333: a batch may not carry a launch past its
        // bundle cap either — clamp the loop to the launch's
        // remaining allowance (a launch already at the cap is
        // refused, not silently handed an empty batch). The
        // pick path re-checks the gate per pick, so a clamped
        // loop never trips it mid-batch and strands claimed
        // rows behind an error envelope.
        let take = match (lane, claim_session) {
            (Some(lane_id), Some(sid)) => {
                let claimed = bundle_claim_count(t, lane_id, sid)?;
                if let Some(msg) = bundle_cap_refusal(claimed, lane_id, sid) {
                    return Err(msg);
                }
                count.min(BUNDLE_CLAIM_CAP - claimed)
            }
            _ => count,
        };
        let mut rows = Vec::new();
        for _ in 0..take {
            let Some(row) = claim_pick_once(
                t,
                scope,
                worker.to_string(),
                lane,
                lease_seconds,
                family.clone(),
                claim_session.clone(),
                None,
                model,
                handoff_pick,
                remaining_budget_minutes,
            )?
            else {
                break;
            };
            rows.push(row);
        }
        Ok(rows)
    })();
    match work {
        Ok(rows) => {
            let claimed = rows.len();
            let mut env = json!({
                "rows": rows,
                "claimed": claimed,
                "requested": count,
            });
            // #8154: an empty batch names why, same as a single claim.
            if claimed == 0 {
                let counts = no_claim_counts(t, &iso(now_unix()), scope);
                if let Some(obj) = env.as_object_mut() {
                    if let Some(c) = counts.as_object() {
                        for (k, v) in c {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            ok(env)
        }
        Err(e) if e.starts_with("claim_repo_task:") => fail(4, e),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_claim(t: &TursoBackend, req: &Value) -> Value {
    // Reject malformed manual-override intent before the claim preamble
    // can expire leases, touch lanes, or otherwise mutate the test/store.
    if let Some(reason) = req.get("manual_override") {
        if reason.as_str().map_or(true, |s| s.trim().is_empty()) {
            return fail(
                2,
                "claim_repo_task: manual_override requires a non-empty reason",
            );
        }
        if req_i64(req, "task_id").filter(|id| *id > 0).is_none()
            || req.get("from_session").is_some()
            || req.get("pick").is_some()
            || req
                .get("count")
                .is_some_and(|count| count.as_i64() != Some(1))
        {
            return fail(
                2,
                "claim_repo_task: manual_override requires one --task and cannot combine with transfer, handoff, or batch",
            );
        }
    }
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    // One-queue model (#8097, 2026-11-10): the request's queue
    // is OPTIONAL. Absent, empty, "*" or "all" names no stream:
    // the claim spans the one fleet queue in priority order. A
    // named stream still scopes the claim exactly as before, so
    // every existing scoped caller keeps working unchanged.
    let queue: Option<String> = match req_str(req, "queue") {
        None | Some("") | Some("*") | Some("all") => None,
        Some(q) => Some(q.to_string()),
    };
    // The claimer-side filter expression (--filter): a boolean
    // expression over task metadata, evaluated inside the claim
    // (see parse_filter). Absent is the default: no filtering.
    let filter_expr = match req_str(req, "filter") {
        None | Some("") => None,
        Some(f) => match parse_filter(f) {
            Ok(e) => Some(e),
            Err(e) => return fail(2, format!("claim_repo_task: bad filter: {e}")),
        },
    };
    // The claimer's declared pool identity (--pool): recorded
    // like the other identity marks and used only to satisfy a
    // task's own requires.pool, never as a filter.
    let pool = req_str(req, "pool").map(str::to_string);
    let lease_seconds = req_i64(req, "lease_seconds").unwrap_or(CLAIM_DEFAULT_LEASE_SECONDS);
    // #3418: the claimer's declared remaining shift budget. Absent
    // is a claim that cannot prove budget fit: requirement-bearing
    // rows skip it (see budget_gate_sql); requirement-free rows
    // behave exactly as before.
    let remaining_budget_minutes = req_i64(req, "remaining_budget_minutes");
    let lane = req_str(req, "lane").map(str::to_string);
    // #8187 (demoting #8097 Rule 2): the claim's declared platform
    // is a MARK, never a filter — it is stamped on the claimed row
    // (platform = COALESCE(declared, existing)) and carried on the
    // claimed event, and it excludes NOTHING: a claim whose
    // platform mismatches the item's mark claims it all the same,
    // and a claim declaring no platform works identically.
    let family = match parse_platform(req_str(req, "platform")) {
        Ok(f) => f,
        Err(e) => return fail(2, format!("claim_repo_task: {e}")),
    };
    // The claiming session's own id, when the surface knows it:
    // another mark for the claimed event, never required.
    let claim_session = req_str(req, "session_id").map(str::to_string);
    // #9178 §1 (FDR-0001): the claimer's recorded reason when the
    // session actually used is a fallback for the task's resume
    // hint. Only the two FDR-0001 values exist; anything else is a
    // caller error, never silently recorded or dropped.
    let resume_fallback = match req_str(req, "resume_fallback") {
        None => None,
        Some(f @ ("missing_session" | "worktree_mismatch")) => Some(f.to_string()),
        Some(other) => {
            return fail(
                2,
                format!("claim_repo_task: unknown resume_fallback {other:?}"),
            );
        }
    };
    // #8176: the claiming session's own model, stamped on the row
    // the claim returns (claim_provider / claim_model /
    // claim_variant). Absent fields stamp nothing.
    let model = match parse_claim_model(req) {
        Ok(m) => m,
        Err(e) => return fail(2, format!("claim_repo_task: {e}")),
    };
    // #8097 Rule 1: pick mode. The default pick is the queue's next
    // suitable item. "handed_off" restricts the pick to recorded
    // opencode handoffs in the sink (status 'handed_off', #8234) —
    // the broker chooses the row, the caller names NO task id.
    // Naming a task (task_id, below) stays the explicit special
    // case. ("parked_handoff" is the pre-#8234 spelling of the
    // mode, kept as a deprecated alias so a straggler caller fails
    // soft; the sink has not been a park since #8234.)
    let handoff_pick = match req_str(req, "pick") {
        None => false,
        Some("handed_off") | Some("parked_handoff") => true,
        Some(other) => {
            return fail(2, format!("claim_repo_task: unknown pick mode {other:?}"));
        }
    };
    // #8217: batch claim. `count` absent or 1 is the single claim
    // below, byte for byte. count > 1 runs the same pick that many
    // times (verb_claim_batch) and returns every claimed row; it
    // never combines with a named task (by-id and transfer claims
    // take exactly one row).
    // Union matching (#8097 one-queue model): the claim's
    // scope compiles the claimer's requirements (stream +
    // filter) together with the task-side requirements clause;
    // both are evaluated inside the pick and its UPDATE
    // re-check. The claimer context is this claim's identity —
    // recorded on the claim, and used only to satisfy tasks'
    // own declared requirements.
    let claimer = ClaimerCtx {
        family: family.clone(),
        lane: lane.clone(),
        worker: worker.clone(),
        pool: pool.clone(),
        model: model.clone(),
    };
    let scope = claim_scope(queue.as_deref(), filter_expr.as_ref(), &claimer);
    let count = match parse_batch_count(req) {
        Ok(c) => c,
        Err(e) => return fail(2, format!("claim_repo_task: {e}")),
    };
    if count > 1 {
        if req_i64(req, "task_id").is_some() {
            return fail(
                2,
                "claim_repo_task: --count cannot be combined with --task: \
                 a by-id claim takes exactly one task (#8217)"
                    .to_string(),
            );
        }
        return verb_claim_batch(
            t,
            queue.as_deref(),
            &scope,
            &worker,
            &lane,
            lease_seconds,
            &family,
            &claim_session,
            &model,
            handoff_pick,
            remaining_budget_minutes,
            count,
        );
    }

    let work = (|| -> Result<Option<Value>, TErr> {
        claim_preamble(t, queue.as_deref(), &worker, &lane)?;
        // By-id claim — the SPECIAL CASE (#8097 Rule 1): when the
        // request names a task, take exactly that row (claim_by_id)
        // instead of the pick. Reserved for one-off revivals of a
        // specific row (e.g. a single sink row); the default flow
        // never names a task — the claimant collects sink rows
        // through the pick="handed_off" mode above.
        if let Some(want_id) = req_i64(req, "task_id") {
            // #8210: task_id + from_session is a session-to-session
            // TRANSFER — take the row directly from its current
            // holder, never through queued or parked. (A bare
            // task_id stays the by-id special case below.)
            if let Some(from_session) = req_str(req, "from_session") {
                return claim_transfer(
                    t,
                    want_id,
                    queue.as_deref(),
                    &worker,
                    &lane,
                    lease_seconds,
                    family.as_deref(),
                    &model,
                    from_session,
                    claim_session.as_deref(),
                    &pool,
                );
            }
            return claim_by_id(
                t,
                want_id,
                queue.as_deref(),
                &worker,
                &lane,
                lease_seconds,
                family.as_deref(),
                &model,
                claim_session.as_deref(),
                resume_fallback.as_deref(),
                &pool,
                req_str(req, "manual_override"),
            );
        }
        claim_pick_once(
            t,
            &scope,
            worker.clone(),
            &lane,
            lease_seconds,
            family,
            claim_session,
            resume_fallback,
            &model,
            handoff_pick,
            remaining_budget_minutes,
        )
    })();
    match work {
        Ok(Some(row)) => ok(json!({"row": row})),
        Ok(None) => {
            // #3418 (#4712): surface the budget-skip summary on an
            // empty default pick so impossible-fit work escalates
            // instead of skipping silently forever. By-id and
            // handoff picks name their row/mode, so no summary.
            let mut resp = json!({"row": Value::Null});
            if !handoff_pick && req_i64(req, "task_id").is_none() {
                let summary =
                    budget_skip_summary(t, &iso(now_unix()), &scope, remaining_budget_minutes);
                if let Some(obj) = resp.as_object_mut() {
                    if let Some(s) = summary.as_object() {
                        for (k, v) in s {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
                // #8154 (follow-on #2937 acceptance 4): name WHY the
                // claim found nothing — the scoped queued / at-cap /
                // not-yet-due / parked-due counts ride the no-row
                // envelope so the client diagnostic can say whether
                // the stream is empty, capped, or merely not due.
                let counts = no_claim_counts(t, &iso(now_unix()), &scope);
                if let Some(obj) = resp.as_object_mut() {
                    if let Some(c) = counts.as_object() {
                        for (k, v) in c {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            ok(resp)
        }
        Err(e) if e.starts_with("claim_repo_task:") => fail(4, e),
        Err(e) => terr(e),
    }
}

/// #3418 (#4712): impossible-fit surfacing. When a default-pick
/// claim finds no claimable row, count the otherwise-eligible rows
/// skipped purely on budget and report the largest requirement
/// seen, so a lane or orchestrator can escalate work no shift can
/// fit instead of silently skipping it forever. The liveness
/// predicate mirrors default_pick_predicate minus the budget gate
/// (queued-due rows under the attempts cap; parked rows whose
/// timer passed, plus capacity parks, which the budget bypass can
/// take early). Scope (queue/filter/requirements) applies, so the
/// summary describes exactly the stream this claim searched.
/// #3418 (#4712): the impossible-fit summary query shape. Counts
/// otherwise-eligible rows skipped purely on budget and reports the
/// largest requirement seen. The liveness predicate mirrors
/// default_pick_predicate minus the budget gate; scope applies so
/// the summary describes exactly the stream the claim searched.
pub(crate) fn budget_skip_sql(scope_sql: &str) -> String {
    let req_expr = REQUIRED_BUDGET_EXPR;
    format!(
        "SELECT COUNT(*) AS n, MAX({req_expr}) AS m FROM tasks \
         WHERE payload IS NOT NULL AND payload <> '' AND payload <> '{{}}' \
         AND (((status = 'queued' AND attempts < max_attempts \
           AND (not_before IS NULL OR not_before <= ?)) \
         OR (status = 'parked' AND ((not_before IS NOT NULL AND not_before <= ?) \
           OR wait_kind = 'capacity') AND {PARKED_TERM_EXCLUSION})) \
         AND {req_expr} IS NOT NULL \
         AND {req_expr} > COALESCE(?, -1)){scope_sql}"
    )
}

pub(crate) fn budget_skip_summary(
    t: &TursoBackend,
    now: &str,
    scope: &ClaimScope,
    remaining_budget_minutes: Option<i64>,
) -> Value {
    let sql = budget_skip_sql(&scope.sql);
    let remaining_arg = match remaining_budget_minutes {
        Some(v) => Arg::Int(v),
        None => Arg::Null,
    };
    let mut args: Vec<Arg> = vec![
        Arg::Text(now.to_string()),
        Arg::Text(now.to_string()),
        remaining_arg,
    ];
    for a in &scope.args {
        args.push(a.clone());
    }
    let (n, m) = match t.query_objs(&sql, &args, &[]) {
        Ok(rows) => {
            let first = rows.into_iter().next();
            let n = first
                .as_ref()
                .and_then(|r| r.get("n"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let m = first.and_then(|mut r| r.get_mut("m").map(|v| v.take()));
            (n, m.unwrap_or(Value::Null))
        }
        Err(_) => return json!({}),
    };
    json!({
        "budget_skipped": n,
        "max_required_budget_minutes": m,
    })
}

/// #8154 (follow-on #2937 acceptance 4): the no-claim counts query
/// shape. One aggregate pass over the claim's own scope counts the
/// rows the claim searched but could not take, split by reason:
/// queued (all), at_cap_queued (attempts >= max_attempts — the
/// #6528 health definition), queued_not_yet_due (under cap, parked
/// behind a future not_before), parked_due (parked rows the pick
/// could have taken early: timer lapsed or a capacity park).
/// Placeholders: the not-yet-due `now`, then the parked-due `now`,
/// then the scope args.
pub(crate) fn no_claim_counts_sql(scope_sql: &str) -> String {
    format!(
        "SELECT \
         COALESCE(SUM(CASE WHEN status = 'queued' THEN 1 ELSE 0 END), 0) AS queued, \
         COALESCE(SUM(CASE WHEN status = 'queued' AND attempts >= max_attempts THEN 1 ELSE 0 END), 0) AS at_cap_queued, \
         COALESCE(SUM(CASE WHEN status = 'queued' AND attempts < max_attempts \
           AND not_before IS NOT NULL AND not_before > ? THEN 1 ELSE 0 END), 0) AS queued_not_yet_due, \
         COALESCE(SUM(CASE WHEN status = 'parked' AND ((not_before IS NOT NULL AND not_before <= ?) \
           OR wait_kind = 'capacity') AND {PARKED_TERM_EXCLUSION} THEN 1 ELSE 0 END), 0) AS parked_due \
         FROM tasks \
         WHERE payload IS NOT NULL AND payload <> '' AND payload <> '{{}}'{scope_sql}"
    )
}

pub(crate) fn no_claim_counts(t: &TursoBackend, now: &str, scope: &ClaimScope) -> Value {
    let sql = no_claim_counts_sql(&scope.sql);
    let mut args: Vec<Arg> = vec![Arg::Text(now.to_string()), Arg::Text(now.to_string())];
    for a in &scope.args {
        args.push(a.clone());
    }
    match t.query_objs(&sql, &args, &[]) {
        Ok(rows) => {
            let first = rows.into_iter().next().unwrap_or_else(|| json!({}));
            let get = |k: &str| first.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
            json!({
                "queued": get("queued"),
                "at_cap_queued": get("at_cap_queued"),
                "queued_not_yet_due": get("queued_not_yet_due"),
                "parked_due": get("parked_due"),
            })
        }
        // A failed count leaves the envelope without counts (the
        // client then falls back to the bare diagnostic) — never a
        // fabricated number.
        Err(_) => json!({}),
    }
}

pub(crate) fn verb_lane(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let exclusion = req_str(req, "exclusion_reason").map(str::to_string);
    let opencode_reason = req_str(req, "opencode_reason").map(str::to_string);
    let limit_platform = req_str(req, "platform_limit").map(str::to_string);
    let limit_reason = req_str(req, "platform_limit_reason").map(str::to_string);
    let clear_limit = req
        .get("clear_platform_limit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let work = (|| -> Result<Value, TErr> {
        let Some(task) = get_task(t, task_id)? else {
            return Err(format!(
                "assign_repo_task_lane: task {task_id} does not exist"
            ));
        };
        // #8210: a platform limitation is recorded only with its
        // reason — validate before anything mutates.
        if !clear_limit {
            if let Some(e) =
                platform_limit_error(limit_platform.as_deref(), limit_reason.as_deref())
            {
                return Err(format!("assign_repo_task_lane: {e}"));
            }
        }
        // Apply a limitation set/clear to the ledger row (called
        // once the row exists, on either path). Records the
        // limitation + reason (or the clearing) on a
        // lane_assigned event, like every assignment change.
        let apply_limit = |t: &TursoBackend| -> Result<(), TErr> {
            if clear_limit {
                t.exec(
                    "UPDATE repo_task_lane_ledger SET platform_limit = NULL, \
                       platform_limit_reason = NULL, updated_at = ? \
                     WHERE task_id = ?",
                    &[Arg::Text(now_iso()), Arg::Int(task_id)],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "platform_limit": Value::Null,
                            "platform_limit_reason": Value::Null,
                            "platform_limit_cleared": true,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
            } else if let Some(l) = limit_platform.as_deref() {
                let fam = parse_platform(Some(l)).ok().flatten();
                t.exec(
                    "UPDATE repo_task_lane_ledger SET platform_limit = ?, \
                       platform_limit_reason = ?, updated_at = ? \
                     WHERE task_id = ?",
                    &[
                        arg_str(fam.as_deref()),
                        arg_str(limit_reason.as_deref()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "platform_limit": fam,
                            "platform_limit_reason": limit_reason,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
            }
            Ok(())
        };
        if let Some(existing) = get_ledger(t, task_id)? {
            // Idempotent (migration 0021): keep progress.lane in step.
            let mut lane = existing
                .get("assigned_lane")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // #8368: the binding that counts is the *live* one —
            // a ledger session_id the broker itself cleared (it
            // matches progress.session_cleared) is not bound, and a
            // row carrying it flips like an unbound row.
            let bound = bound_session(
                existing.get("session_id").and_then(|v| v.as_str()),
                cleared_session_id(&task),
            );
            // #8187 reassignment: an unbound opencode row flips to
            // direct when the caller passes --exclusion-reason (a
            // Muse lane taking a sink/legacy opencode task direct).
            // The progress.lane re-sync below then stamps direct.
            if lane_reassignment_flip(&lane, bound, exclusion.as_deref()) {
                t.exec(
                    "UPDATE repo_task_lane_ledger SET assigned_lane = 'direct', \
                       session_id = NULL, exclusion_reason = ?, updated_at = ? WHERE task_id = ?",
                    &[
                        arg_str(exclusion.as_deref()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "assigned_lane": "direct", "previous_lane": "opencode",
                            "exclusion_reason": exclusion, "opencode_reason": Value::Null,
                            "reason": "exclusion", "reassignment": true,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
                lane = "direct".to_string();
            }
            // #8210: the symmetric flip — an existing DIRECT row
            // flips to opencode when the caller passes
            // --opencode-reason (a Muse session sending its
            // task to opencode with a stated reason) and no
            // session is bound. The progress.lane re-sync below
            // then stamps opencode.
            if lane_flip_to_opencode(&lane, bound, opencode_reason.as_deref()) {
                t.exec(
                    "UPDATE repo_task_lane_ledger SET assigned_lane = 'opencode', \
                       session_id = NULL, opencode_reason = ?, exclusion_reason = NULL, updated_at = ? \
                     WHERE task_id = ?",
                    &[
                        arg_str(opencode_reason.as_deref()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
                event(
                    t,
                    Some(&worker),
                    Some(task_id),
                    "lane_assigned",
                    &with_marks(
                        &json!({
                            "assigned_lane": "opencode", "previous_lane": "direct",
                            "exclusion_reason": Value::Null,
                            "opencode_reason": opencode_reason,
                            "reason": "opencode_reason", "reassignment": true,
                            "via": "assign_repo_task_lane_turso",
                        }),
                        &task,
                    ),
                )?;
                lane = "opencode".to_string();
            }
            // #8368: a requested flip that a *live* bound session
            // refuses is an explicit error naming the session —
            // never the silent unchanged row, which lanes cannot
            // distinguish from success.
            if exclusion.is_some() && lane == "opencode" {
                if let Some(sid) = bound {
                    return Err(format!(
                        "assign_repo_task_lane: task {task_id} is bound to session {sid} \
                         (live, not cleared) \u{2014} the #8187 exclusion flip is refused \
                         for a live bound session; its owner must release or transfer it"
                    ));
                }
            }
            if opencode_reason.is_some() && lane == "direct" {
                if let Some(sid) = bound {
                    return Err(format!(
                        "assign_repo_task_lane: task {task_id} is bound to session {sid} \
                         (live, not cleared) \u{2014} the #8210 opencode flip is refused \
                         for a live bound session; its owner must release or transfer it"
                    ));
                }
            }
            let cur = task
                .get("progress")
                .and_then(|p| p.get("lane"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if cur != lane {
                let progress = merge_obj(
                    &task.get("progress").cloned().unwrap_or(json!({})),
                    &json!({"lane": lane}),
                );
                t.exec(
                    "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
                    &[
                        Arg::Text(progress.to_string()),
                        Arg::Text(now_iso()),
                        Arg::Int(task_id),
                    ],
                )?;
            }
            apply_limit(t)?;
            return Ok(get_ledger(t, task_id)?.unwrap_or(existing));
        }
        let queue = task
            .get("queue")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // #8187: no balancing counts — direct by default; opencode
        // only for a specific reason recorded at assignment time.
        let (lane, reason) = lane_decision(exclusion.as_deref(), opencode_reason.as_deref());
        let now = now_iso();
        t.exec(
            "INSERT INTO repo_task_lane_ledger \
             (queue, task_id, assigned_lane, exclusion_reason, opencode_reason, assigned_by, assigned_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                Arg::Text(queue),
                Arg::Int(task_id),
                Arg::Text(lane.to_string()),
                arg_str(exclusion.as_deref()),
                arg_str(opencode_reason.as_deref()),
                Arg::Text(worker.clone()),
                Arg::Text(now.clone()),
                Arg::Text(now),
            ],
        )?;
        let progress = merge_obj(
            &task.get("progress").cloned().unwrap_or(json!({})),
            &json!({"lane": lane}),
        );
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "lane_assigned",
            &with_marks(
                &json!({
                    "assigned_lane": lane, "exclusion_reason": exclusion,
                    "opencode_reason": opencode_reason, "reason": reason,
                    "via": "assign_repo_task_lane_turso",
                }),
                &task,
            ),
        )?;
        apply_limit(t)?;
        get_ledger(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) if e.starts_with("assign_repo_task_lane:") => fail(4, e),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_bind(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let session_id = req_str(req, "session_id").unwrap_or("").to_string();
    let launched_by = req_str(req, "launched_by")
        .unwrap_or("opencode-claimant")
        .to_string();
    // #8287: a malformed session id refuses here, as a usage
    // error, before any state is touched — binding a ghost id
    // strands the task until stuck-bound supervision recovers
    // it. The predicate is the one the client/claim paths keep
    // (client::valid_session_id); do not fork a second one.
    if !crate::client::valid_session_id(&session_id) {
        return fail(
            2,
            format!(
                "bind_repo_task_lane_session: invalid session_id {session_id:?} \
                 (expected ses_ + alphanumeric/underscore)"
            ),
        );
    }
    // #9046: a bind may carry the bound session's own model (the
    // claimant derives it from the session record, #8176); when
    // it does, the bind stamps it via stamp_claim_model below.
    let model = match parse_claim_model(req) {
        Ok(m) => m,
        Err(e) => return fail(2, format!("bind_repo_task_lane_session: {e}")),
    };
    let work = (|| -> Result<Value, TErr> {
        let Some(task) = get_task(t, task_id)? else {
            return Err(format!(
                "bind_repo_task_lane_session: task {task_id} does not exist"
            ));
        };
        let now = now_iso();
        let live = task.get("status").and_then(|v| v.as_str()) == Some("claimed")
            && task
                .get("lease_until")
                .and_then(|v| v.as_str())
                .map(|l| l > now.as_str())
                .unwrap_or(false);
        if !live {
            return Err(format!(
                "bind_repo_task_lane_session: task {task_id} is not claimed with a live lease (status {}, lease_until {})",
                task.get("status").and_then(|v| v.as_str()).unwrap_or(""),
                task.get("lease_until")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            ));
        }
        let Some(ledger) = get_ledger(t, task_id)? else {
            return Err(format!(
                "bind_repo_task_lane_session: no lane ledger row for task {task_id}"
            ));
        };
        if let Some(existing) = ledger.get("session_id").and_then(|v| v.as_str()) {
            if existing != session_id {
                return Err(format!(
                    "bind_repo_task_lane_session: task {task_id} already bound to session {existing}"
                ));
            }
        }
        t.exec(
            "UPDATE repo_task_lane_ledger SET session_id = ?, updated_at = ? \
             WHERE task_id = ?",
            &[
                Arg::Text(session_id.clone()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        let progress = merge_obj(
            &task.get("progress").cloned().unwrap_or(json!({})),
            &json!({"session_id": session_id, "launched_by": launched_by}),
        );
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        // #9046: the canonical binding record, beside the legacy
        // lane_session_bound event below; a model-carrying bind
        // also stamps (the claimant's collect -> launch -> bind
        // flow is where the working session becomes known).
        record_session_bound(t, Some(&worker), task_id, &task, &session_id, "bind")?;
        if model.is_some() {
            let mut task_now = get_task(t, task_id)?.unwrap_or_else(|| task.clone());
            stamp_claim_model(
                t,
                task_id,
                &mut task_now,
                &model,
                Some(&worker),
                Some(&session_id),
                "bind",
            )?;
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "lane_session_bound",
            &with_marks(
                &json!({
                    "session_id": session_id, "launched_by": launched_by,
                    "via": "bind_repo_task_lane_session_turso",
                }),
                &task,
            ),
        )?;
        get_ledger(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
        Err(e) if e.starts_with("bind_repo_task_lane_session:") => fail(4, e),
        Err(e) => terr(e),
    }
}

/// Whether a broker envelope reports success.
pub(crate) fn envelope_is_ok(v: &Value) -> bool {
    v.get("ok").and_then(|o| o.as_bool()) == Some(true)
}

/// #5757-A: claim + lane-assign + session-bind in one broker
/// round trip — the composition of the three verbs, in order,
/// with the same semantics as calling them in sequence: the
/// claim step's error envelope, or an empty claim, returns
/// verbatim and later steps are not attempted. A LANE/BIND
/// failure is different: the claim has already committed, so it
/// answers as a partial success (ok envelope + claimed row +
/// `claim_flow_partial` naming the failed step), never as an
/// error envelope — a retryable error there would make the
/// client's call_result replay the WHOLE flow and claim a
/// second task, leaking this one (Codex P1 on PR #8493).
/// The caller finishes the failed step with the single-verb
/// /lane or /bind call for this task id. The answer carries
/// every step's row: `row` is the claimed task row (the shape
/// /claim answers and the client's claim_output consumes);
/// `lane_row` and `bind_row` are exactly what /lane and /bind
/// answer in the three-verb flow.
pub(crate) fn verb_claim_flow(t: &TursoBackend, req: &Value) -> Value {
    if let Err(e) = crate::client::claim_flow_precheck(req) {
        return fail(2, e);
    }
    let claim = verb_claim(t, req);
    if !envelope_is_ok(&claim) {
        return claim;
    }
    let row = claim.get("row").cloned().unwrap_or(Value::Null);
    if row.is_null() {
        return claim;
    }
    let task_id = row.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut lane_req = req.clone();
    if let Some(obj) = lane_req.as_object_mut() {
        obj.insert("task_id".to_string(), json!(task_id));
    }
    let lane = verb_lane(t, &lane_req);
    if !envelope_is_ok(&lane) {
        return ok(json!({
            "row": row,
            "claim_flow_partial": {
                "step": "lane",
                "exit": lane.get("exit").cloned().unwrap_or(Value::Null),
                "error": lane.get("error").cloned().unwrap_or(Value::Null),
            },
        }));
    }
    let bind_req = json!({
        "task_id": task_id,
        "worker": req.get("worker"),
        "session_id": req.get("session_id"),
        "launched_by": req.get("launched_by"),
    });
    let bind = verb_bind(t, &bind_req);
    if !envelope_is_ok(&bind) {
        return ok(json!({
            "row": row,
            "lane_row": lane.get("row").cloned().unwrap_or(Value::Null),
            "claim_flow_partial": {
                "step": "bind",
                "exit": bind.get("exit").cloned().unwrap_or(Value::Null),
                "error": bind.get("error").cloned().unwrap_or(Value::Null),
            },
        }));
    }
    ok(json!({
        "row": row,
        "lane_row": lane.get("row").cloned().unwrap_or(Value::Null),
        "bind_row": bind.get("row").cloned().unwrap_or(Value::Null),
    }))
}

/// Shared guard for the claimed-by verbs: returns the task row or
/// the PG-shaped refusal.
///
/// #6985: every scheduled lane claims as the same worker
/// (repo-worker-code), so a worker match alone let one lane
/// heartbeat, checkpoint, park or complete another lane's live
/// claim — the second lane's verbs all succeeded and the overlap
/// showed only on GitHub as two agent notes. When the request
/// names the caller's lane (`lane`, a registry lane id) and the
/// row records the claiming lane, the two must agree. Either side
/// missing keeps the worker-only check (older callers, claims made
/// with no --lane).
pub(crate) fn claimed_by(
    t: &TursoBackend,
    task_id: i64,
    worker: &str,
    verb: &str,
    req: &Value,
) -> Result<Value, Value> {
    match get_task(t, task_id) {
        Ok(Some(row))
            if row.get("status").and_then(|v| v.as_str()) == Some("claimed")
                && row.get("worker_id").and_then(|v| v.as_str()) == Some(worker) =>
        {
            if let Some((holder, caller)) = foreign_lane_claim(&row, req) {
                let _ = event(
                    t,
                    Some(worker),
                    Some(task_id),
                    "claim_refused",
                    &json!({
                        "via": verb,
                        "reason": "claimed_by_another_lane",
                        "holder_lane_id": holder,
                        "caller_lane_id": caller,
                    }),
                );
                return Err(fail(
                    5,
                    format!(
                        "{verb}: task {task_id} is not claimed by {worker} on lane {caller}: lane {holder} holds it"
                    ),
                ));
            }
            Ok(row)
        }
        Ok(row) => {
            // #12618: say what the task is now, so a worker whose
            // claim lapsed can tell that from a claim held under
            // another name. The latest claim or lease expiry is read
            // fail-soft: a miss only drops the lease detail.
            let last_event = row.as_ref().and_then(|_| {
                t.query_objs(
                    "SELECT worker_id, kind, created_at FROM events WHERE task_id = ? \
                     AND kind IN ('claimed', 'lease_expired') ORDER BY id DESC LIMIT 1",
                    &[Arg::Int(task_id)],
                    &[],
                )
                .ok()
                .and_then(|rows| rows.into_iter().next())
            });
            Err(fail(
                5,
                format!(
                    "{verb}: task {task_id} is not claimed by {worker}: {}",
                    not_claimed_reason(row.as_ref(), last_event.as_ref(), worker)
                ),
            ))
        }
        Err(e) => Err(terr(e)),
    }
}

/// Why a task is not claimed by `worker`, for the claimed-by
/// refusal (#12618): the task's status and holder, and, when the
/// task's latest claim or lease expiry (`last_event`) is this
/// worker's own lease expiring, that the claim
/// lapsed and the task went back to the queue, so it must be claimed
/// again before it can be completed.
pub(crate) fn not_claimed_reason(
    row: Option<&Value>,
    last_event: Option<&Value>,
    worker: &str,
) -> String {
    let Some(row) = row else {
        return "no such task".to_string();
    };
    let status = row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let holder = row
        .get("worker_id")
        .and_then(|v| v.as_str())
        .filter(|w| !w.is_empty());
    let mut reason = match (status, holder) {
        ("claimed", Some(h)) => format!("it is claimed by {h}"),
        (s, _) => format!("it is {s}"),
    };
    let field = |key: &str| last_event.and_then(|e| e.get(key)).and_then(|v| v.as_str());
    if status != "claimed"
        && field("kind") == Some("lease_expired")
        && field("worker_id") == Some(worker)
    {
        reason.push_str(&format!(
            "; {worker}'s lease expired{} and the task went back to the queue, so claim it \
             again before completing it (heartbeat to keep a lease; checkpoint saves \
             progress but does not renew it)",
            field("created_at")
                .map(|at| format!(" at {at}"))
                .unwrap_or_default()
        ));
    }
    reason
}

/// The (holder lane, caller lane) pair when a claimed row's lane
/// and the request's lane are both registry lane ids and differ
/// (#6985). An execution mode ("direct", "opencode") or a missing
/// lane on either side is no lane identity and never refuses.
pub(crate) fn foreign_lane_claim(row: &Value, req: &Value) -> Option<(String, String)> {
    let caller = req_str(req, "lane")
        .map(str::trim)
        .filter(|l| !l.is_empty())?;
    let holder = row
        .get("lane_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|l| !l.is_empty())?;
    if matches!(caller, "direct" | "opencode") || matches!(holder, "direct" | "opencode") {
        return None;
    }
    (caller != holder).then(|| (holder.to_string(), caller.to_string()))
}

/// #9487 (REL-02 design record §10): the holder guard every
/// holder write carries. `claimed_by` reads the claim, then the
/// write commits later; an unconditional `WHERE id = ?` let a
/// writer whose claim lapsed and was re-taken in that window
/// overwrite the new holder's row. The guard is the heartbeat's
/// claim tuple (heartbeat.sql), so a write lands only while the
/// claim it read is still the claim on the row. Lanes share one
/// worker id, so worker_id alone would not tell two claims apart;
/// claimed_at, lane_id and attempts do.
pub(crate) const HOLDER_GUARD: &str = " AND status = 'claimed' AND worker_id = ? AND claimed_at IS ? AND lane_id IS ? AND attempts = ?";

/// The four arguments that bind HOLDER_GUARD, taken from the row
/// `claimed_by` returned.
pub(crate) fn holder_guard_args(worker: &str, row: &Value) -> [Arg; 4] {
    [
        Arg::Text(worker.to_string()),
        arg_str(row["claimed_at"].as_str()),
        arg_str(row["lane_id"].as_str()),
        Arg::Int(row["attempts"].as_i64().unwrap_or(0)),
    ]
}

/// The refusal a guarded holder write returns when it matched no
/// row: the claim changed between the read and the write, and
/// nothing was written.
pub(crate) fn claim_changed(verb: &str, task_id: i64) -> Value {
    fail(
        5,
        format!("{verb}: claim for task {task_id} changed before the write committed"),
    )
}

pub(crate) fn verb_heartbeat(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let extend = req_i64(req, "extend_seconds").unwrap_or(HEARTBEAT_DEFAULT_EXTEND_SECONDS);
    let row = match claimed_by(t, task_id, &worker, "heartbeat_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let progress = merge_obj(
        &row.get("progress").cloned().unwrap_or(json!({})),
        &req.get("progress").cloned().unwrap_or(json!({})),
    );
    // Compare against the deadline in the row at write time. The trigger
    // installed with the schema mirrors the effective deadline in this same
    // statement, including on an early heartbeat that only updates progress.
    let sql = format!("{} RETURNING {TASK_COLS}", include_str!("heartbeat.sql"));
    let result = t.query_objs(
        &sql,
        &[
            Arg::Text(after_iso(extend)),
            Arg::Text(progress.to_string()),
            Arg::Text(now_iso()),
            Arg::Int(task_id),
            Arg::Text(worker.clone()),
            arg_str(row["claimed_at"].as_str()),
            arg_str(row["lane_id"].as_str()),
            Arg::Int(row["attempts"].as_i64().unwrap_or(0)),
        ],
        &TASK_JSON_COLS,
    );
    let mut rows = match result {
        Ok(rows) => rows,
        Err(e) => return terr(e),
    };
    let Some(updated) = rows.pop() else {
        return fail(
            5,
            format!("heartbeat_task: claim for task {task_id} changed"),
        );
    };
    match touch_worker(t, &worker, Some(task_id)) {
        Ok(()) => ok(json!({"row": updated})),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_checkpoint(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let stage = req_str(req, "stage").unwrap_or("").to_string();
    let row = match claimed_by(t, task_id, &worker, "checkpoint_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let given = req.get("progress").cloned().unwrap_or(json!({}));
    let progress = merge_obj(
        &row.get("progress").cloned().unwrap_or(json!({})),
        &merge_obj(
            &given,
            &json!({"last_checkpoint": {"stage": stage, "at": now_iso()}}),
        ),
    );
    let work = (|| -> Result<Value, TErr> {
        // #9487: the task row's progress write goes first and
        // carries the holder guard, so a checkpoint from a claim
        // that changed after claimed_by read it writes nothing --
        // neither the task row nor the per-worker KV entry.
        let mut args = vec![
            Arg::Text(progress.to_string()),
            Arg::Text(now_iso()),
            Arg::Int(task_id),
        ];
        args.extend(holder_guard_args(&worker, &row));
        let upd = t.exec(
            &format!("UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?{HOLDER_GUARD}"),
            &args,
        )?;
        if upd.affected == 0 {
            return Ok(claim_changed("checkpoint_task", task_id));
        }
        // PG checkpoints is a per-worker KV (worker_id, key, value):
        // the checkpoint lands under task:<id> with the stage +
        // progress as the value; the task row's progress merge above
        // is the other half of the contract.
        t.exec(
            "INSERT INTO checkpoints (worker_id, key, value, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(worker_id, key) DO UPDATE SET \
               value = excluded.value, updated_at = excluded.updated_at",
            &[
                Arg::Text(worker.clone()),
                Arg::Text(format!("task:{task_id}")),
                Arg::Text(json!({"stage": stage, "progress": given, "at": now_iso()}).to_string()),
                Arg::Text(now_iso()),
            ],
        )?;
        touch_worker(t, &worker, Some(task_id))?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "checkpoint",
            &json!({"stage": stage, "via": "checkpoint_task_turso"}),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        Ok(ok(json!({"row": row})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

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

/// Whether a task row may be woken by the /wake verb (#8184):
/// only a parked (broker-held waiting) row. Claimed/queued rows
/// are already in the claim flow; terminal rows are done.
pub(crate) fn wake_eligible(status: &str) -> Result<(), String> {
    match status {
        "parked" => Ok(()),
        // #8234: a sink row is not a wait and cannot be woken —
        // it is already claimable; the refusal says so instead of
        // the generic not-a-wait message.
        "handed_off" => Err(
            "task status handed_off cannot be woken: a handed_off row is not \
             a wait — it is already claimable, claim it instead"
                .to_string(),
        ),
        other => Err(format!(
            "task status {other} cannot be woken (only parked waits)"
        )),
    }
}

/// The wake verb (#8184): release a broker-held wait NOW because
/// its condition was satisfied — the triage step waking an item
/// it just triaged is the canonical caller. The row returns to
/// queued with its wait fields cleared, due immediately; the
/// next claim is an ordinary execution claim. This replaces the
/// lanes' 25-minute re-check loop as the wake path for recorded
/// waits: the park TTL floor is only the backstop.
/// Release a claimed task straight back to queued (#8217): the
/// holder gives the item back to the pool NOW — no park timer, no
/// attempt change (the claim happened; the `released` event
/// carries the reason, and revive remains the cap-repair tool).
/// One exception (#12901): when the released claim consumed the
/// task's last attempt, a queued return would be a zombie — the
/// claim pick (attempts < max_attempts) can never select the row
/// again — so the release lands where an at-cap lease expiry
/// lands (#6463): failed, with the release reason in last_error,
/// and revive takes it from there.
/// Holder-only, through the same claimed_by guard as checkpoint.
/// This is the dispatch failure path: an item whose sub-task
/// session failed to launch/bind returns to the pool immediately
/// instead of sitting claimed-idle under the dispatcher.
/// A release may also carry a platform limitation (#14993): the
/// route-mismatch hand-back, recorded on the lane ledger so the
/// claim pick stops offering the task to the wrong family.
pub(crate) fn verb_release(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    if reason.trim().is_empty() {
        return fail(
            2,
            "release_repo_task: a release requires a reason              (--reason): the audit event is the only record of why              the item went back"
                .to_string(),
        );
    }
    // #14993: a release may carry a platform limitation -- the
    // route-mismatch hand-back. A lane that draws a task routed to
    // a seat it is not (a laptop checkout, an opencode-only
    // packet) records the limitation as it hands the task back,
    // and the claim pick (PLATFORM_LIMIT_PICK_CLAUSE) then never
    // offers the task to another family again, instead of
    // re-offering it on every redraw. Validated exactly like the
    // lane verb's set (#8210): a real family, and a recorded
    // reason of its own.
    let limit_platform = req_str(req, "platform_limit").map(str::to_string);
    let limit_reason = req_str(req, "platform_limit_reason").map(str::to_string);
    if let Some(e) = platform_limit_error(limit_platform.as_deref(), limit_reason.as_deref()) {
        return fail(2, format!("release_repo_task: {e}"));
    }
    let limit_family = limit_platform
        .as_deref()
        .and_then(|l| parse_platform(Some(l)).ok().flatten());
    let row = match claimed_by(t, task_id, &worker, "release_repo_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let work = (|| -> Result<Value, TErr> {
        // A released claim never leaves a binding behind
        // (migration 0023 §4's rule, mirrored from the claim path).
        let progress = row.get("progress").cloned().unwrap_or(json!({}));
        // #12901: where a released claim lands is the question
        // lease expiry already answers — expire_leases'
        // expiry_disposition oracle, computed on the same inputs
        // (the sink shape is read from the progress BEFORE the
        // binding is cleared, exactly as expire_leases reads it).
        // Below the cap the row returns to queued as before. At
        // the cap a queued return is the silent zombie of the
        // claim→release churn (task 21418: four sessionless
        // claim/release cycles in ~13 minutes left the row queued
        // at attempts == max_attempts, unclaimable, counted as
        // queued work, with nothing on the row recording why) —
        // so an at-cap release fails the row with the release
        // reason in last_error, and the audit event is the fail
        // event, whose history transition (claimed → failed) is
        // the true one. Sink rows are exempt, as in expire_leases:
        // the sink pick serves them under its own rule.
        let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let max_attempts = row
            .get("max_attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64);
        let to_sink = expired_claim_returns_to_sink(&progress);
        let fail_at_cap =
            !to_sink && expiry_disposition(attempts, max_attempts) == ExpiryDisposition::FailAtCap;
        let (progress, cleared) = clear_binding(&progress, "release_repo_task");
        let cap_error = format!("released at attempt cap ({attempts}/{max_attempts}): {reason}");
        let mut args = vec![];
        let sql = if fail_at_cap {
            args.push(Arg::Text(cap_error.clone()));
            format!(
                "UPDATE tasks SET status = 'failed', worker_id = NULL, \
                   lease_until = NULL, not_before = NULL, park_kind = NULL, \
                   wait_kind = NULL, last_error = ?, progress = ?, updated_at = ? \
                 WHERE id = ?{HOLDER_GUARD}"
            )
        } else {
            format!(
                "UPDATE tasks SET status = 'queued', worker_id = NULL, \
                   lease_until = NULL, not_before = NULL, park_kind = NULL, \
                   wait_kind = NULL, progress = ?, updated_at = ? \
                 WHERE id = ?{HOLDER_GUARD}"
            )
        };
        args.push(Arg::Text(progress.to_string()));
        args.push(Arg::Text(now_iso()));
        args.push(Arg::Int(task_id));
        args.extend(holder_guard_args(&worker, &row));
        let upd = t.exec(&sql, &args)?;
        if upd.affected == 0 {
            return Ok(claim_changed("release_repo_task", task_id));
        }
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        // #14993: record the limitation on the task's lane ledger
        // (the row the claim pick reads). A claimed task may have
        // no ledger row yet -- assignment creates one, a bare claim
        // does not -- so the write upserts: update the row when one
        // exists, insert a minimal one carrying just the limit
        // when none does.
        if let Some(fam) = &limit_family {
            let upd = t.exec(
                "UPDATE repo_task_lane_ledger SET platform_limit = ?, \
                   platform_limit_reason = ?, updated_at = ? \
                 WHERE task_id = ?",
                &[
                    Arg::Text(fam.clone()),
                    arg_str(limit_reason.as_deref()),
                    Arg::Text(now_iso()),
                    Arg::Int(task_id),
                ],
            )?;
            if upd.affected == 0 {
                let queue = row
                    .get("queue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let now = now_iso();
                t.exec(
                    "INSERT INTO repo_task_lane_ledger \
                     (queue, task_id, platform_limit, platform_limit_reason, assigned_by, assigned_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    &[
                        Arg::Text(queue),
                        Arg::Int(task_id),
                        Arg::Text(fam.clone()),
                        arg_str(limit_reason.as_deref()),
                        Arg::Text(worker.clone()),
                        Arg::Text(now.clone()),
                        Arg::Text(now),
                    ],
                )?;
            }
        }
        touch_worker(t, &worker, Some(task_id))?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, task_id, old, "release_repo_task")?;
        }
        // #12901: an at-cap release records a `failed` event (the
        // verb_fail shape: the error text plus the release's own
        // marks), so the history transition written for the event
        // is the true claimed → failed; a below-cap release keeps
        // the `released` event.
        let (kind, detail) = if fail_at_cap {
            (
                "failed",
                json!({
                    "reason": reason, "error": cap_error,
                    "attempts": attempts, "max_attempts": max_attempts,
                    "released_at_cap": true,
                    "via": "release_repo_task_turso",
                    "cleared_session_id": cleared,
                    "platform_limit": limit_family,
                    "platform_limit_reason": limit_reason,
                }),
            )
        } else {
            (
                "released",
                json!({
                    "reason": reason, "via": "release_repo_task_turso",
                    "cleared_session_id": cleared,
                    "platform_limit": limit_family,
                    "platform_limit_reason": limit_reason,
                }),
            )
        };
        event(
            t,
            Some(&worker),
            Some(task_id),
            kind,
            &with_marks(&detail, &row),
        )?;
        let row = get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))?;
        Ok(ok(json!({ "row": row })))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_wake(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker")
        .unwrap_or("fleet-queue-cli")
        .to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id).map_err(|e| e)? else {
            return Err(format!("wake_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let prev_wait_kind = row
            .get("wait_kind")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if status == "blocked" && dep_block_active(&row) {
            // #8276: wake on a dep-blocked task forces an immediate
            // recheck of that task — the only honest answer to
            // "is the gate clear yet?" is the broker's own check.
            // The recheck unblocks iff every dep has resolved; the
            // row returned is the post-recheck state either way.
            let mut cache = std::collections::HashMap::new();
            dep_recheck_one(t, &row, &mut cache, now_unix()).map_err(|e| e)?;
            return get_task(t, task_id)
                .map(|r| r.unwrap_or(Value::Null))
                .map_err(|e| e);
        }
        wake_eligible(&status).map_err(|e| format!("wake_task: task {task_id}: {e}"))?;
        t.exec(
            "UPDATE tasks SET status = 'queued', not_before = NULL, \
               park_kind = NULL, wait_kind = NULL, worker_id = NULL, \
               lease_until = NULL, updated_at = ? \
             WHERE id = ? AND status = 'parked'",
            &[Arg::Text(now_iso()), Arg::Int(task_id)],
        )
        .map_err(|e| e)?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "woken",
            &with_marks(
                &json!({
                    "reason": reason, "previous_status": status,
                    "previous_wait_kind": prev_wait_kind,
                    "via": "wake_task_turso",
                }),
                &row,
            ),
        )
        .map_err(|e| e)?;
        get_task(t, task_id)
            .map(|r| r.unwrap_or(Value::Null))
            .map_err(|e| e)
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(5, format!("wake_task: task {task_id} does not exist")),
        Err(e) if e.starts_with("wake_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

// ---------------------------------------------------------------------------
// Tests (#14100): reprioritize T1–T7 against a rusqlite-backed fake of
// the Turso pipeline endpoint (the actions_exec proof-bar pattern —
// real SQL semantics, so the guarded CAS UPDATE, the claim order,
// and the lease/worker/attempts invariance are exercised, not
// mimicked). GitHub itself is not touched: the sync leg's contract
// (queue-first, priority/*-only labels, durable outbox + visible
// pending marker) is covered through the rows the verb commits
// atomically and the broker source pins in queue_broker.rs.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod reprioritize_tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::{Arc, Mutex};

    pub(super) fn fake_backend() -> (TursoBackend, Arc<Mutex<rusqlite::Connection>>) {
        let (t, db, _) = fake_backend_with_race();
        (t, db)
    }

    /// SQL the fake runs once, just before the next statement that
    /// starts with `UPDATE tasks` (#9487): the race window between
    /// a verb's claimed_by read and its first write, made
    /// deterministic.
    pub(super) type RaceHook = Arc<Mutex<Option<String>>>;

    pub(super) fn fake_backend_with_race()
    -> (TursoBackend, Arc<Mutex<rusqlite::Connection>>, RaceHook) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for ddl in SCHEMA {
            conn.execute_batch(ddl).unwrap();
        }
        let db = Arc::new(Mutex::new(conn));
        let hook: RaceHook = Arc::new(Mutex::new(None));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let db_thread = db.clone();
        let hook_thread = hook.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(sock) = sock else { return };
                let db_conn = db_thread.clone();
                let hook_conn = hook_thread.clone();
                std::thread::spawn(move || handle_conn(sock, db_conn, hook_conn));
            }
        });
        (
            TursoBackend::new(base, "test".to_string(), ureq::agent()),
            db,
            hook,
        )
    }

    fn handle_conn(
        mut sock: std::net::TcpStream,
        db: Arc<Mutex<rusqlite::Connection>>,
        hook: RaceHook,
    ) {
        let mut reader = BufReader::new(sock.try_clone().unwrap());
        loop {
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                return;
            }
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some((name, value)) = trimmed.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            let payload = run_pipeline(&db, &hook, &body);
            let text = payload.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n{}",
                text.len(),
                text
            );
            if sock.write_all(resp.as_bytes()).is_err() {
                return;
            }
        }
    }

    fn run_pipeline(db: &Arc<Mutex<rusqlite::Connection>>, hook: &RaceHook, body: &[u8]) -> Value {
        let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let conn = db.lock().unwrap();
        let mut results = Vec::new();
        for r in req
            .get("requests")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
        {
            match r.get("type").and_then(|v| v.as_str()) {
                Some("close") => {
                    results.push(json!({"type": "ok", "response": {"type": "close"}}));
                }
                Some("execute") => {
                    let stmt = r.get("stmt").cloned().unwrap_or(Value::Null);
                    let sql = stmt.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                    let mut params: Vec<rusqlite::types::Value> = Vec::new();
                    if let Some(args) = stmt.get("args").and_then(|v| v.as_array()) {
                        for a in args {
                            params.push(match a.get("type").and_then(|v| v.as_str()) {
                                Some("text") => rusqlite::types::Value::Text(
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                ),
                                Some("integer") => rusqlite::types::Value::Integer(
                                    a.get("value")
                                        .and_then(|v| v.as_str())
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0),
                                ),
                                _ => rusqlite::types::Value::Null,
                            });
                        }
                    }
                    if sql.trim_start().starts_with("UPDATE tasks ") {
                        if let Some(race) = hook.lock().unwrap().take() {
                            conn.execute_batch(&race).unwrap();
                        }
                    }
                    results.push(execute_sql(&conn, sql, &params));
                }
                _ => {}
            }
        }
        json!({"results": results, "baton": null})
    }

    fn cell_json(v: rusqlite::types::ValueRef) -> Value {
        use rusqlite::types::ValueRef::*;
        match v {
            Null => json!({"type": "null"}),
            Integer(i) => json!({"type": "integer", "value": i.to_string()}),
            Real(f) => json!({"type": "float", "value": f}),
            Text(s) => json!({"type": "text", "value": String::from_utf8_lossy(s)}),
            Blob(b) => json!({"type": "blob", "value": hex_encode(b)}),
        }
    }

    fn hex_encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn execute_sql(
        conn: &rusqlite::Connection,
        sql: &str,
        params: &[rusqlite::types::Value],
    ) -> Value {
        let is_query = sql.trim_start().to_ascii_uppercase().starts_with("SELECT");
        // BEGIN/COMMIT are connection state, not preparable statements.
        if !is_query {
            let head = sql.trim_start().to_ascii_uppercase();
            if head.starts_with("BEGIN") {
                return match conn.execute_batch(sql) {
                    Ok(()) => json!({"type": "ok", "response": {"type": "execute", "result": {
                        "cols": [], "rows": [], "affected_row_count": 0, "last_insert_rowid": null}}}),
                    Err(e) => {
                        json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}})
                    }
                };
            }
            if head.starts_with("COMMIT") || head.starts_with("ROLLBACK") {
                return match conn.execute_batch(sql) {
                    Ok(()) => json!({"type": "ok", "response": {"type": "execute", "result": {
                        "cols": [], "rows": [], "affected_row_count": 0, "last_insert_rowid": null}}}),
                    Err(e) => {
                        json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}})
                    }
                };
            }
        }
        let mut stmt = match conn.prepare(sql) {
            Ok(s) => s,
            Err(e) => {
                return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
            }
        };
        if is_query {
            let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
            let mut rows_out = Vec::new();
            match stmt.query(rusqlite::params_from_iter(params.iter())) {
                Ok(mut rows) => loop {
                    match rows.next() {
                        Ok(Some(row)) => {
                            let mut cells = Vec::new();
                            for i in 0..cols.len() {
                                cells.push(cell_json(row.get_ref(i).unwrap()));
                            }
                            rows_out.push(Value::Array(cells));
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
                        }
                    }
                },
                Err(e) => {
                    return json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}});
                }
            }
            json!({"type": "ok", "response": {"type": "execute", "result": {
                "cols": cols.iter().map(|c| json!({"name": c, "decltype": null})).collect::<Vec<_>>(),
                "rows": rows_out, "affected_row_count": 0, "last_insert_rowid": null}}})
        } else {
            match stmt.execute(rusqlite::params_from_iter(params.iter())) {
                Ok(changed) => json!({"type": "ok", "response": {"type": "execute", "result": {
                    "cols": [], "rows": [], "affected_row_count": changed,
                    "last_insert_rowid": conn.last_insert_rowid().to_string()}}}),
                Err(e) => {
                    json!({"type": "error", "error": {"message": e.to_string(), "code": "SQLITE"}})
                }
            }
        }
    }

    /// Seed one live task row; returns its id.
    fn seed_task(
        t: &TursoBackend,
        status: &str,
        priority: i64,
        payload: Value,
        worker: Option<&str>,
    ) -> i64 {
        let payload_text = payload.to_string();
        let r = t
            .exec(
                "INSERT INTO tasks (queue, title, payload, priority, status, worker_id, lease_until, attempts, progress, created_at, updated_at) \
                 VALUES ('repo-health-code-pilot', 't', ?, ?, ?, ?, ?, 0, '{}', ?, ?)",
                &[
                    Arg::Text(payload_text),
                    Arg::Int(priority),
                    Arg::Text(status.to_string()),
                    worker.map(|w| Arg::Text(w.to_string())).unwrap_or(Arg::Null),
                    if worker.is_some() {
                        Arg::Text("2026-11-24T02:00:00Z".to_string())
                    } else {
                        Arg::Null
                    },
                    Arg::Text("2026-11-24T01:00:00Z".to_string()),
                    Arg::Text("2026-11-24T01:00:00Z".to_string()),
                ],
            )
            .unwrap();
        r.last_insert_rowid.unwrap()
    }

    pub(super) fn events_for(t: &TursoBackend, task_id: i64, kind: &str) -> Vec<Value> {
        t.query_objs(
            "SELECT id, worker_id, task_id, kind, detail, created_at FROM events WHERE task_id = ? AND kind = ? ORDER BY id",
            &[Arg::Int(task_id), Arg::Text(kind.to_string())],
            &[],
        )
        .unwrap()
    }

    /// T6a: terminal states are refused.
    #[test]
    fn terminal_tasks_are_refused() {
        let (t, _) = fake_backend();
        for status in ["done", "failed", "cancelled"] {
            let id = seed_task(&t, status, 90, json!({}), None);
            let out = verb_reprioritize(&t, &json!({"task_id": id, "priority": 20, "worker": "w"}));
            assert_eq!(out["ok"], false, "{status}: {out}");
            assert!(
                out["error"].as_str().unwrap_or("").contains("terminal"),
                "{status}: {out}"
            );
            assert!(events_for(&t, id, "reprioritized").is_empty());
        }
    }

    /// T6b: off-scale targets are refused with the scale error.
    #[test]
    fn off_scale_targets_are_refused() {
        let (t, _) = fake_backend();
        let id = seed_task(&t, "queued", 90, json!({}), None);
        for bad in [json!(15), json!(25), json!(5), json!(99), json!("high")] {
            let out =
                verb_reprioritize(&t, &json!({"task_id": id, "priority": bad, "worker": "w"}));
            assert_eq!(out["ok"], false, "{bad}: {out}");
            assert!(
                out["error"].as_str().unwrap_or("").contains("design scale"),
                "{bad}: {out}"
            );
        }
        // The row is untouched by every refusal.
        let row = get_task(&t, id).unwrap().unwrap();
        assert_eq!(row["priority"], 90);
        assert!(events_for(&t, id, "reprioritized").is_empty());
    }

    /// T2: the reprioritized event carries from/to/category/actor and
    /// lands in the same transaction as the row change.
    #[test]
    fn history_event_carries_from_to_category_actor() {
        let (t, _) = fake_backend();
        let id = seed_task(
            &t,
            "queued",
            90,
            json!({"repo": "developing-today/code", "issue": {"number": 1}}),
            None,
        );
        let out = verb_reprioritize(
            &t,
            &json!({"task_id": id, "priority": 20, "worker": "conductor", "reason": "hot"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let evs = events_for(&t, id, "reprioritized");
        assert_eq!(evs.len(), 1, "{evs:?}");
        let detail: Value = serde_json::from_str(evs[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["from"], 90);
        assert_eq!(detail["to"], 20);
        assert_eq!(detail["category"], "high");
        assert_eq!(detail["actor"], "conductor");
        assert_eq!(detail["reason"], "hot");
        assert_eq!(detail["task_id"], id);
        assert_eq!(detail["sync"], "github-pending");
    }

    /// T5: a claimed task changes priority only — lease, worker and
    /// attempts are byte-identical before and after.
    #[test]
    fn claimed_task_changes_priority_only() {
        let (t, _) = fake_backend();
        let id = seed_task(&t, "claimed", 90, json!({}), Some("holder-1"));
        t.exec(
            "UPDATE tasks SET attempts = 3 WHERE id = ?",
            &[Arg::Int(id)],
        )
        .unwrap();
        let before = get_task(&t, id).unwrap().unwrap();
        let out = verb_reprioritize(
            &t,
            &json!({"task_id": id, "priority": 30, "worker": "conductor"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let after = get_task(&t, id).unwrap().unwrap();
        assert_eq!(after["priority"], 30);
        for k in ["worker_id", "lease_until", "attempts", "status"] {
            assert_eq!(after[k], before[k], "column {k} moved");
        }
    }

    /// T7: the stored packet's payload.priority copy moves with the
    /// row, and the row still passes claim-guard revalidation.
    #[test]
    fn packet_copy_moves_with_row() {
        let (t, _) = fake_backend();
        let id = seed_task(
            &t,
            "queued",
            90,
            json!({"priority": 90, "repo": "developing-today/code", "issue": {"number": 2}}),
            None,
        );
        let out = verb_reprioritize(&t, &json!({"task_id": id, "priority": 40, "worker": "w"}));
        assert_eq!(out["ok"], true, "{out}");
        let row = get_task(&t, id).unwrap().unwrap();
        assert_eq!(row["priority"], 40);
        assert_eq!(row["payload"]["priority"], 40);
        // Claim-guard revalidation: the row priority is on-scale and
        // the packet mirror agrees.
        assert!(crate::validate::PRIORITY_SCALE.contains(&row["priority"].as_i64().unwrap()));
        // A task without a stored priority copy leaves payload alone.
        let bare = seed_task(
            &t,
            "parked",
            90,
            json!({"repo": "developing-today/code"}),
            None,
        );
        let out = verb_reprioritize(&t, &json!({"task_id": bare, "priority": 20, "worker": "w"}));
        assert_eq!(out["ok"], true, "{out}");
        let row = get_task(&t, bare).unwrap().unwrap();
        assert_eq!(row["priority"], 20);
        assert!(row["payload"].get("priority").is_none());
        // Queue-only change notes that no issue is linked.
        let evs = events_for(&t, bare, "reprioritized");
        assert_eq!(evs.len(), 1);
        let detail: Value = serde_json::from_str(evs[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["sync"], "no issue linked");
    }

    /// T1: reprioritizing the later task above the earlier one flips
    /// the next-claim order (claims read priority ASC).
    #[test]
    fn reprioritize_flips_claim_order() {
        let (t, _) = fake_backend();
        let early = seed_task(&t, "queued", 70, json!({}), None);
        let late = seed_task(&t, "queued", 90, json!({}), None);
        let first_before: Vec<Value> = t
            .query_objs(
                "SELECT id FROM tasks WHERE status = 'queued' ORDER BY priority ASC, id ASC LIMIT 1",
                &[],
                &[],
            )
            .unwrap();
        assert_eq!(first_before[0]["id"], early);
        let out = verb_reprioritize(&t, &json!({"task_id": late, "priority": 20, "worker": "w"}));
        assert_eq!(out["ok"], true, "{out}");
        let first_after: Vec<Value> = t
            .query_objs(
                "SELECT id FROM tasks WHERE status = 'queued' ORDER BY priority ASC, id ASC LIMIT 1",
                &[],
                &[],
            )
            .unwrap();
        assert_eq!(first_after[0]["id"], late);
    }

    /// T4: the queue change commits its durable sync intent in the
    /// same transaction and marks the row visibly pending — so a
    /// GitHub-leg failure can never silently drop the sync.
    #[test]
    fn sync_intent_is_atomic_and_visible() {
        let (t, _) = fake_backend();
        let id = seed_task(
            &t,
            "blocked",
            90,
            json!({"priority": 90, "repo": "developing-today/code", "issue": {"number": 14100}}),
            None,
        );
        let out = verb_reprioritize(&t, &json!({"task_id": id, "priority": 10, "worker": "w"}));
        assert_eq!(out["ok"], true, "{out}");
        // The row carries the visible pending marker fleet queue show reads.
        // (json_set stores the boolean as 1 — the production readback shape.)
        let row = get_task(&t, id).unwrap().unwrap();
        assert_eq!(row["priority"], 10);
        assert!(
            row["progress"]["reprioritize_sync_pending"] == json!(1)
                || row["progress"]["reprioritize_sync_pending"] == json!(true),
            "pending marker missing: {}",
            row["progress"]
        );
        // The durable outbox intent for the mapped category+label.
        let rows = t
            .query_objs(
                "SELECT task_id, repo, issue, category, label, state FROM reprioritize_sync_outbox WHERE task_id = ?",
                &[Arg::Int(id)],
                &[],
            )
            .unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["repo"], "developing-today/code");
        assert_eq!(rows[0]["issue"], 14100);
        assert_eq!(rows[0]["category"], "critical");
        assert_eq!(rows[0]["label"], "priority/critical");
        assert_eq!(rows[0]["state"], "pending");
        // Queue-only tasks produce no sync row and no pending marker.
        let bare = seed_task(&t, "queued", 90, json!({}), None);
        let out = verb_reprioritize(&t, &json!({"task_id": bare, "priority": 20, "worker": "w"}));
        assert_eq!(out["ok"], true, "{out}");
        let row = get_task(&t, bare).unwrap().unwrap();
        assert!(row["progress"].get("reprioritize_sync_pending").is_none());
        let rows = t
            .query_objs(
                "SELECT task_id FROM reprioritize_sync_outbox WHERE task_id = ?",
                &[Arg::Int(bare)],
                &[],
            )
            .unwrap();
        assert!(rows.is_empty());
    }

    // -- #14317: park ready_condition retention ---------------------

    /// Park one claimed task with a caller-supplied ready
    /// condition; returns (task id, park envelope).
    fn park_with_condition(t: &TursoBackend) -> (i64, Value) {
        let id = seed_task(t, "claimed", 90, json!({}), Some("w"));
        let out = verb_park(
            t,
            &json!({
                "task_id": id, "worker": "w", "reason": "waiting on the trigger",
                "not_before": "2027-06-01T00:00:00Z",
                "ready_condition": {"kind": "event_seen", "event_kind": "trigger_fired",
                                    "note": "trigger prioritized"},
                "progress": {},
            }),
        );
        assert_eq!(out["ok"], true, "{out}");
        (id, out)
    }

    /// #14317 oracle (storage + read-back): parking with
    /// --ready-condition persists the supplied structured
    /// condition, and the rich /list read-back behind `queue show`
    /// / `list --json` returns it intact — before the fix the
    /// rich projection omitted ready_condition (and park_kind /
    /// wait_kind / last_error) entirely, so a parked task read
    /// back `ready_condition: null` and lost its early-wake hook.
    /// Oracle: the supplied condition object itself, compared
    /// verbatim at both layers.
    #[test]
    fn park_retains_ready_condition_through_rich_list_readback_14317() {
        let (t, _) = fake_backend();
        let (id, out) = park_with_condition(&t);
        let supplied = json!({"kind": "event_seen", "event_kind": "trigger_fired",
                              "note": "trigger prioritized"});
        // The park envelope's own row carries the condition.
        assert_eq!(out["row"]["ready_condition"], supplied, "{out}");
        // The stored row carries it, parked until the explicit
        // not_before.
        let row = get_task(&t, id).unwrap().unwrap();
        assert_eq!(row["status"], "parked");
        assert_eq!(row["ready_condition"], supplied);
        assert_eq!(row["not_before"], "2027-06-01T00:00:00Z");
        // The rich list read-back (queue show / list --json)
        // carries it too, alongside the other park wait fields
        // the show field list reads from the row.
        let list = verb_list(&t, &json!({"rich": true, "task": id, "limit": 1}));
        assert_eq!(list["ok"], true, "{list}");
        let rows = list["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{list}");
        assert_eq!(rows[0]["ready_condition"], supplied, "{list}");
        assert_eq!(rows[0]["park_kind"], "absolute");
        assert_eq!(rows[0]["wait_kind"], "external");
        assert!(rows[0].get("last_error").is_some(), "{list}");
    }

    /// #14317 oracle (early wake): the stored condition is what
    /// the park-revalidation sweeper evaluates — when the named
    /// event fires, the sweeper releases the parked task even
    /// though its not_before is still months away.
    #[test]
    fn parked_ready_condition_is_evaluated_before_not_before_14317() {
        let (t, _) = fake_backend();
        let (id, _) = park_with_condition(&t);
        // The trigger fires: the named event lands on the task.
        t.exec(
            "INSERT INTO events (worker_id, task_id, kind, detail, created_at) \
             VALUES ('trigger-source', ?, 'trigger_fired', '{}', ?)",
            &[Arg::Int(id), Arg::Text(now_iso())],
        )
        .unwrap();
        // Age the park past the sweeper's recheck cutoff.
        t.exec(
            "UPDATE tasks SET last_checked_at = '2020-01-01T00:00:00Z', \
             updated_at = '2020-01-01T00:00:00Z' WHERE id = ?",
            &[Arg::Int(id)],
        )
        .unwrap();
        let summary = revalidate_parked(&t, 0, 10, false).unwrap();
        assert_eq!(summary["released"], 1, "{summary}");
        let row = get_task(&t, id).unwrap().unwrap();
        assert_eq!(row["status"], "queued", "{row}");
        assert_eq!(row["not_before"], Value::Null, "{row}");
    }
}

#[cfg(test)]
mod task_history_tests {
    use super::reprioritize_tests::{events_for, fake_backend};
    use super::*;

    fn history_rows(t: &TursoBackend, task_id: i64) -> Vec<Value> {
        t.query_objs(
            "SELECT event_id, kind, from_status, to_status, from_priority, to_priority, actor, session_id, model_id \
             FROM task_history WHERE task_id = ? ORDER BY id",
            &[Arg::Int(task_id)],
            &[],
        )
        .unwrap()
    }

    fn insert_metadata_task(t: &TursoBackend, title: &str) -> i64 {
        let now = now_iso();
        t.exec(
            "INSERT INTO tasks (queue, title, payload, metadata, status, created_at, updated_at) VALUES (?, ?, '{}', '{}', 'queued', ?, ?)",
            &[Arg::Text("metadata-test".into()), Arg::Text(title.into()), Arg::Text(now.clone()), Arg::Text(now)],
        )
        .unwrap()
        .last_insert_rowid
        .unwrap()
    }

    #[test]
    fn metadata_merge_patch_limits_filter_and_history_oracle() {
        let (t, _) = fake_backend();
        let first = insert_metadata_task(&t, "first");
        let other = insert_metadata_task(&t, "other");

        let set = super::super::verb_meta(
            &t,
            &json!({"op":"set", "task_id":first, "patch":{
                "programme":"audit-7",
                "tracking_issue":"developing-today/code#14170",
                "custom.with-dot":"kept"
            }}),
        );
        assert_eq!(set["ok"], true);

        let patch = super::super::verb_meta(
            &t,
            &json!({"op":"set", "task_id":first, "patch":{
                "programme":null,
                "source_issue":"developing-today/code#14158"
            }}),
        );
        assert_eq!(patch["ok"], true);
        let metadata =
            super::super::verb_meta(&t, &json!({"op":"get", "task_id":first}))["row"].clone();
        assert!(metadata.get("programme").is_none());
        assert_eq!(metadata["tracking_issue"], "developing-today/code#14170");
        assert_eq!(metadata["source_issue"], "developing-today/code#14158");
        assert_eq!(metadata["custom.with-dot"], "kept");

        let filtered = super::verb_list(
            &t,
            &json!({"queue":"metadata-test", "meta":{
                "tracking_issue":"developing-today/code#14170",
                "custom.with-dot":"kept"
            }}),
        );
        assert_eq!(filtered["total"], 1);
        assert_eq!(filtered["rows"][0]["id"], first);

        let too_large = Value::Object(
            ["a", "b", "c", "d", "e"]
                .into_iter()
                .map(|key| (key.to_string(), Value::String("x".repeat(2000))))
                .collect(),
        );
        let rejected =
            super::super::verb_meta(&t, &json!({"op":"set", "task_id":other, "patch":too_large}));
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"].as_str().unwrap().contains("8192"));

        let history = t
            .query_objs(
                "SELECT kind, detail FROM events WHERE task_id = ? AND kind = 'meta_updated' ORDER BY id",
                &[Arg::Int(first)],
                &["detail"],
            )
            .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(
            history[0]["detail"]["changed"]["programme"]["new"],
            "audit-7"
        );
        assert_eq!(
            history[1]["detail"]["changed"]["programme"]["old"],
            "audit-7"
        );
        assert!(history[1]["detail"]["changed"]["programme"]["new"].is_null());
        assert_eq!(
            history_rows(&t, first)
                .iter()
                .filter(|r| r["kind"] == "meta_updated")
                .count(),
            2
        );
    }

    /// Oracle (#14170 Q1): a scripted lifecycle through the real verbs
    /// writes exactly one task_history row per mapped event, in order,
    /// and each row's to_status equals the task's actual status right
    /// after that verb. from/to, actor, session and model are asserted
    /// on the rows the claim, park, block and complete verbs wrote.
    #[test]
    fn scripted_lifecycle_history_is_one_to_one_with_transitions() {
        let (t, _) = fake_backend();
        let queue = "repo-health-code-pilot";
        let enq = verb_enqueue(
            &t,
            &json!({"queue": queue, "packet": {
                "title": "oracle", "priority": 90,
                "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:14170",
                "payload": {
                    "schema": "repo-task/v1", "kind": "small_fix",
                    "repo": "developing-today/code", "problem": "p", "priority_reason": "r",
                    "evidence": ["e"], "acceptance": ["a"], "tests": ["t"], "context_refs": [],
                    "scope": {"include": ["x"], "exclude": ["y"]}, "risk": {"level": "low", "flags": []},
                    "timebox_minutes": 45, "issue": {"number": 14170},
                    "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test"}
                }
            }}),
        );
        assert_eq!(enq["ok"], true, "{enq}");
        let id = enq["row"]["id"].as_i64().expect("enqueued task id");
        let status_of = |t: &TursoBackend| {
            get_task(t, id).unwrap().unwrap()["status"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let last_to = |t: &TursoBackend| {
            history_rows(t, id)
                .last()
                .map(|r| r["to_status"].as_str().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        assert_eq!(history_rows(&t, id).len(), 1, "enqueued row");
        assert_eq!(last_to(&t), status_of(&t));

        let claim = |t: &TursoBackend| {
            verb_claim(
                t,
                &json!({"worker": "oracle-w", "queue": queue, "lane": "code-oracle",
                        "session_id": "ses_oracle", "model_provider": "anthropic",
                        "model_id": "claude-haiku-5-5", "model_variant": "default"}),
            )
        };
        let c = claim(&t);
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(last_to(&t), "claimed");
        assert_eq!(last_to(&t), status_of(&t));

        let p = verb_park(
            &t,
            &json!({"task_id": id, "worker": "oracle-w", "reason": "wait", "delay_seconds": 60}),
        );
        assert_eq!(p["ok"], true, "{p}");
        assert_eq!(last_to(&t), "parked");
        assert_eq!(last_to(&t), status_of(&t));

        let w = verb_wake(
            &t,
            &json!({"task_id": id, "worker": "oracle-w", "reason": "ready"}),
        );
        assert_eq!(w["ok"], true, "{w}");
        assert_eq!(last_to(&t), "queued", "wake returns the task to the queue");
        assert_eq!(last_to(&t), status_of(&t));

        let c2 = claim(&t);
        assert_eq!(c2["ok"], true, "{c2}");
        assert_eq!(last_to(&t), "claimed");
        let b = verb_block(
            &t,
            &json!({"task_id": id, "worker": "oracle-w", "reason_class": "external", "blocker": "oracle"}),
        );
        assert_eq!(b["ok"], true, "{b}");
        assert_eq!(last_to(&t), "blocked");
        assert_eq!(last_to(&t), status_of(&t));

        let u = verb_unblock(
            &t,
            &json!({"task_id": id, "worker": "oracle-w", "reason": "clear"}),
        );
        assert_eq!(u["ok"], true, "{u}");
        assert_eq!(last_to(&t), status_of(&t));

        let r = verb_reprioritize(
            &t,
            &json!({"task_id": id, "priority": 20, "worker": "oracle-w", "reason": "hot"}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let rp = history_rows(&t, id).last().unwrap().clone();
        assert_eq!(rp["kind"], "reprioritized");
        assert_eq!(
            (rp["from_priority"].as_i64(), rp["to_priority"].as_i64()),
            (Some(90), Some(20))
        );

        let c3 = claim(&t);
        assert_eq!(c3["ok"], true, "{c3}");
        let done = verb_complete(
            &t,
            &json!({"task_id": id, "worker": "oracle-w", "result": {
                "evidence": "https://github.com/developing-today/code/pull/14249",
                "disposition": {
                    "checked_at": now_iso(), "source": "oracle",
                    "issue": {"number": 14170, "state": "closed", "closed_at": now_iso()},
                    "pulls": [{"number": 14249, "state": "closed", "merged_at": now_iso()}],
                    "verdict": "merged_verified"
                }
            }}),
        );
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!(last_to(&t), "done");
        assert_eq!(last_to(&t), status_of(&t));

        let rows = history_rows(&t, id);
        let events: Vec<Value> = {
            let mut all = Vec::new();
            for kind in [
                "enqueued",
                "claimed",
                "parked",
                "woken",
                "blocked",
                "unblocked",
                "reprioritized",
                "completed",
            ] {
                all.extend(events_for(&t, id, kind));
            }
            all.sort_by_key(|e| e["id"].as_i64().unwrap_or(0));
            all
        };
        assert_eq!(
            rows.len(),
            events.len(),
            "1:1 history rows and mapped events"
        );
        for (row, ev) in rows.iter().zip(events.iter()) {
            assert_eq!(row["event_id"], ev["id"], "row maps to its event");
            assert_eq!(row["kind"], ev["kind"]);
        }
        let claimed = rows.iter().find(|r| r["kind"] == "claimed").unwrap();
        assert_eq!(claimed["actor"], "oracle-w");
        assert_eq!(claimed["session_id"], "ses_oracle");
        assert_eq!(claimed["model_id"], "claude-haiku-5-5");
        let parked = rows.iter().find(|r| r["kind"] == "parked").unwrap();
        assert_eq!(
            (parked["from_status"].as_str(), parked["to_status"].as_str()),
            (Some("claimed"), Some("parked"))
        );
        let blocked = rows.iter().find(|r| r["kind"] == "blocked").unwrap();
        assert_eq!(blocked["to_status"], "blocked");
        assert_eq!(blocked["actor"], "oracle-w");
    }

    /// Oracle (#14170 Q1): backfill inserts one row per mapped event
    /// that has none (source='backfill'), reports census = inserted +
    /// skipped, is idempotent, and history-since returns the rows in
    /// id order for the task.
    #[test]
    fn backfill_reconciles_to_events_census_and_is_idempotent() {
        let (t, _) = fake_backend();
        let mut task = |w: &str| {
            t.exec(
                "INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES (?, 7, ?, '{}', '2026-11-01T00:00:00Z')",
                &[Arg::Text(w.to_string()), Arg::Text("claimed".to_string())],
            )
            .unwrap();
        };
        task("w1");
        task("w2");
        t.exec(
            "INSERT INTO events (worker_id, task_id, kind, detail, created_at) VALUES ('w3', 7, 'checkpoint', '{}', '2026-11-01T00:00:00Z')",
            &[],
        )
        .unwrap();
        let first = verb_history_backfill(&t, &json!({}));
        assert_eq!(first["ok"], true, "{first}");
        assert_eq!(first["census"], 2, "{first}");
        assert_eq!(first["inserted"], 2, "{first}");
        assert_eq!(first["skipped"], 0, "{first}");
        let second = verb_history_backfill(&t, &json!({}));
        assert_eq!(second["inserted"], 0, "{second}");
        assert_eq!(second["skipped"], 2, "{second}");
        let rows = history_rows(&t, 7);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r["from_status"] == "queued" && r["to_status"] == "claimed")
        );
        let since = verb_history_since(&t, &json!({"task_id": 7}));
        assert_eq!(since["rows"].as_array().unwrap().len(), 2, "{since}");
    }

    fn seed_consumer_task(
        t: &TursoBackend,
        queue: &str,
        status: &str,
        updated_secs_ago: i64,
    ) -> i64 {
        let now = now_unix();
        let updated = iso(now - updated_secs_ago);
        t.exec(
            "INSERT INTO tasks (queue, title, payload, metadata, status, created_at, updated_at) \
             VALUES (?, 'consumer oracle', '{}', '{}', ?, ?, ?)",
            &[
                Arg::Text(queue.into()),
                Arg::Text(status.into()),
                Arg::Text(updated.clone()),
                Arg::Text(updated),
            ],
        )
        .unwrap()
        .last_insert_rowid
        .unwrap()
    }

    fn seed_history_row(t: &TursoBackend, task_id: i64, kind: &str, secs_ago: i64) {
        let at = iso(now_unix() - secs_ago);
        t.exec(
            "INSERT INTO task_history (task_id, kind, from_status, to_status, actor, detail, source, created_at) \
             VALUES (?, ?, 'queued', 'parked', 'oracle-actor', '{}', 'live', ?)",
            &[Arg::Int(task_id), Arg::Text(kind.into()), Arg::Text(at)],
        )
        .unwrap();
    }

    /// Oracle (#14170 Q8, design §9): the stewardship staleness read is
    /// one canned query — a task whose last transition is older than the
    /// threshold is returned with that transition's kind/actor, a fresh
    /// task is not, a terminal task is not, a history-less task falls
    /// back to updated_at with a null last_history_id, and claims near
    /// the lease cap come back in the same response.
    #[test]
    fn staleness_reads_last_transition_and_lease_cap_in_one_call() {
        let (t, _) = fake_backend();
        let stale = seed_consumer_task(&t, "q8-oracle", "parked", 7_200);
        seed_history_row(&t, stale, "parked", 7_200);
        let fresh = seed_consumer_task(&t, "q8-oracle", "queued", 60);
        seed_history_row(&t, fresh, "enqueued", 60);
        let near_cap = seed_consumer_task(&t, "q8-oracle", "claimed", 60);
        seed_history_row(&t, near_cap, "claimed", 60);
        let far_lease = seed_consumer_task(&t, "q8-oracle", "claimed", 60);
        seed_history_row(&t, far_lease, "claimed", 60);
        let done = seed_consumer_task(&t, "q8-oracle", "done", 7_200);
        seed_history_row(&t, done, "completed", 7_200);
        let no_history = seed_consumer_task(&t, "q8-oracle", "blocked", 7_000);
        let other_queue = seed_consumer_task(&t, "q8-other", "parked", 7_200);
        seed_history_row(&t, other_queue, "parked", 7_200);
        let now = now_unix();
        for (id, lease_in) in [(near_cap, 300), (far_lease, 86_400)] {
            t.exec(
                "UPDATE tasks SET worker_id = 'w1', claimed_at = ?, lease_until = ? WHERE id = ?",
                &[
                    Arg::Text(iso(now - 60)),
                    Arg::Text(iso(now + lease_in)),
                    Arg::Int(id),
                ],
            )
            .unwrap();
        }

        let resp = verb_staleness(
            &t,
            &json!({"queue": "q8-oracle", "older_than_seconds": 3_600, "lease_cap_seconds": 900}),
        );
        assert_eq!(resp["ok"], true, "{resp}");
        let rows = resp["rows"].as_array().unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![stale, no_history], "{resp}");
        assert_eq!(rows[0]["last_kind"], "parked", "{resp}");
        assert_eq!(rows[0]["last_actor"], "oracle-actor", "{resp}");
        assert!(rows[1]["last_history_id"].is_null(), "{resp}");
        let claims = resp["claims_near_lease_cap"].as_array().unwrap();
        assert_eq!(claims.len(), 1, "{resp}");
        assert_eq!(claims[0]["id"], near_cap, "{resp}");

        let all = verb_staleness(&t, &json!({"older_than_seconds": 3_600}));
        let all_ids: Vec<i64> = all["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_i64().unwrap())
            .collect();
        assert!(all_ids.contains(&other_queue), "{all}");
    }

    /// #13068: `list --status dep_blocked` returns the rows health counts
    /// as dep-blocked (blocked rows with a `blocked_deps` record) through
    /// the broker `/list` route, while `--status blocked` still lists
    /// every blocked row.
    #[test]
    fn list_dep_blocked_matches_health_count_13068() {
        let (t, _) = fake_backend();
        let queue = "dep-list-13068";
        let dep_blocked = seed_consumer_task(&t, queue, "blocked", 3_000);
        let plain_blocked = seed_consumer_task(&t, queue, "blocked", 2_000);
        let queued_with_deps = seed_consumer_task(&t, queue, "queued", 1_000);
        let other_queue = seed_consumer_task(&t, "dep-list-13068-other", "blocked", 1_000);
        for id in [dep_blocked, queued_with_deps, other_queue] {
            t.exec(
                "UPDATE tasks SET blocked_deps = ? WHERE id = ?",
                &[
                    Arg::Text(r#"[{"kind":"issue","ref":"developing-today/code#1"}]"#.into()),
                    Arg::Int(id),
                ],
            )
            .unwrap();
        }

        let ids = |resp: &Value| -> Vec<i64> {
            let mut ids: Vec<i64> = resp["rows"]
                .as_array()
                .unwrap_or_else(|| panic!("no rows: {resp}"))
                .iter()
                .map(|r| r["id"].as_i64().unwrap())
                .collect();
            ids.sort_unstable();
            ids
        };

        let dep = crate::turso_queue::dispatch(
            &t,
            "/list",
            &json!({"queue": queue, "status": "dep_blocked", "limit": 500}),
        );
        assert_eq!(ids(&dep), vec![dep_blocked], "{dep}");
        assert_eq!(dep["total"], 1, "{dep}");

        let blocked = crate::turso_queue::dispatch(
            &t,
            "/list",
            &json!({"queue": queue, "status": "blocked", "limit": 500}),
        );
        assert_eq!(ids(&blocked), vec![dep_blocked, plain_blocked], "{blocked}");

        let health = super::super::verb_health(&t, &json!({"queue": queue}));
        let entry = health["rows"]
            .as_array()
            .and_then(|rows| rows.iter().find(|r| r["queue"] == queue))
            .unwrap_or_else(|| panic!("no health row for {queue}: {health}"));
        assert_eq!(entry["dep_blocked"], dep["total"], "{health}");
        assert_eq!(entry["blocked"], 2, "{health}");
    }

    /// Oracle (#14170 Q8, design §9): the needs-user read returns only
    /// blocked/parked tasks carrying a question metadata entry, lifts
    /// the question object out of the metadata, and counts blocked-by
    /// entries from the blocked_deps cache.
    #[test]
    fn needs_user_reads_open_questions_from_metadata() {
        let (t, _) = fake_backend();
        let asked = seed_consumer_task(&t, "q8-oracle", "blocked", 5_000);
        seed_history_row(&t, asked, "blocked", 5_000);
        let answered = seed_consumer_task(&t, "q8-oracle", "parked", 4_000);
        let no_question = seed_consumer_task(&t, "q8-oracle", "blocked", 4_000);
        let queued_with_question = seed_consumer_task(&t, "q8-oracle", "queued", 4_000);
        let _ = no_question;
        for (id, metadata, deps) in [
            (
                asked,
                r#"{"question": {"ledger_id": "L-1", "state": "open"}}"#,
                Some(
                    r#"[{"kind":"issue","ref":"developing-today/code#1"},{"kind":"task","ref":"2"}]"#,
                ),
            ),
            (
                answered,
                r#"{"question": {"ledger_id": "L-2", "state": "answered"}}"#,
                None,
            ),
            (
                queued_with_question,
                r#"{"question": {"ledger_id": "L-3", "state": "open"}}"#,
                None,
            ),
        ] {
            t.exec(
                "UPDATE tasks SET metadata = ?, blocked_deps = ? WHERE id = ?",
                &[
                    Arg::Text(metadata.into()),
                    match deps {
                        Some(d) => Arg::Text(d.into()),
                        None => Arg::Null,
                    },
                    Arg::Int(id),
                ],
            )
            .unwrap();
        }

        let resp = verb_needs_user(&t, &json!({"queue": "q8-oracle"}));
        assert_eq!(resp["ok"], true, "{resp}");
        let rows = resp["rows"].as_array().unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![asked, answered], "{resp}");
        assert_eq!(rows[0]["question"]["ledger_id"], "L-1", "{resp}");
        assert_eq!(rows[0]["question"]["state"], "open", "{resp}");
        assert_eq!(rows[0]["blocked_by_count_cache"], 2, "{resp}");
        assert!(rows[0]["last_transition_at"].is_string(), "{resp}");

        let open_only = verb_needs_user(&t, &json!({"queue": "q8-oracle", "state": "open"}));
        let open_rows = open_only["rows"].as_array().unwrap();
        assert_eq!(open_rows.len(), 1, "{open_only}");
        assert_eq!(open_rows[0]["id"], asked, "{open_only}");
    }
}

#[cfg(test)]
mod task_labels_tests {
    use super::reprioritize_tests::fake_backend;
    use super::*;
    use std::sync::{Arc, Mutex};

    const QUEUE: &str = "repo-health-code-pilot";

    fn packet(number: i64) -> Value {
        json!({
            "title": format!("labels oracle {number}"), "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1", "kind": "small_fix",
                "repo": "developing-today/code", "problem": "p", "priority_reason": "r",
                "evidence": ["e"], "acceptance": ["a"], "tests": ["t"], "context_refs": [],
                "scope": {"include": ["x"], "exclude": ["y"]}, "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45, "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test"}
            }
        })
    }

    fn mirror(conn: &Arc<Mutex<rusqlite::Connection>>) {
        conn.lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE github_mirror_issues (repo TEXT NOT NULL, number INTEGER NOT NULL, \
                 labels TEXT, PRIMARY KEY (repo, number))",
            )
            .unwrap();
    }

    fn set_mirror_labels(conn: &Arc<Mutex<rusqlite::Connection>>, number: i64, labels: &str) {
        conn.lock()
            .unwrap()
            .execute(
                "INSERT INTO github_mirror_issues (repo, number, labels) VALUES ('developing-today/code', ?1, ?2) \
                 ON CONFLICT(repo, number) DO UPDATE SET labels = excluded.labels",
                rusqlite::params![number, labels],
            )
            .unwrap();
    }

    fn cache(t: &TursoBackend, id: i64) -> (Value, Value) {
        let rows = t
            .query_objs(
                "SELECT labels, labels_synced_at FROM tasks WHERE id = ?",
                &[Arg::Int(id)],
                &["labels"],
            )
            .unwrap();
        (
            rows[0]["labels"].clone(),
            rows[0]["labels_synced_at"].clone(),
        )
    }

    fn enqueue(t: &TursoBackend, number: i64) -> i64 {
        let out = verb_enqueue(t, &json!({"queue": QUEUE, "packet": packet(number)}));
        assert_eq!(out["ok"], true, "{out}");
        out["row"]["id"].as_i64().expect("task id")
    }

    fn listed(t: &TursoBackend, label: &str) -> Vec<i64> {
        let out = verb_list(t, &json!({"queue": QUEUE, "label": label, "limit": 100}));
        assert_eq!(out["ok"], true, "{out}");
        let mut ids: Vec<i64> = list_rows(&out)
            .unwrap()
            .iter()
            .filter_map(|r| r["id"].as_i64())
            .collect();
        ids.sort();
        ids
    }

    fn list_rows(out: &Value) -> Result<Vec<Value>, String> {
        crate::client::list_rows(out)
    }

    /// Oracle (#14170 Q5, broker half): enqueue copies the mirror's
    /// labels into the cache with a sync stamp; a label change on the
    /// mirror reaches the cache through the sweep with no queue write;
    /// an issue with no mirror row reads `[]` and "never synced"; and
    /// `--label user/*` lists exactly the held set.
    #[test]
    fn label_cache_follows_mirror_and_filters_held_set() {
        let (t, conn) = fake_backend();
        mirror(&conn);
        set_mirror_labels(&conn, 14170, r#"["priority/high","user/hold"]"#);
        set_mirror_labels(&conn, 14171, r#"["priority/high"]"#);

        let held = enqueue(&t, 14170);
        let plain = enqueue(&t, 14171);
        let unmirrored = enqueue(&t, 14172);

        let (labels, synced) = cache(&t, held);
        assert_eq!(labels, json!(["priority/high", "user/hold"]));
        assert!(synced.is_string(), "enqueue stamps the sync time: {synced}");
        let (labels, synced) = cache(&t, unmirrored);
        assert_eq!(labels, json!([]));
        assert!(synced.is_null(), "no mirror row reads as never synced");

        assert_eq!(listed(&t, "user/*"), vec![held]);
        assert_eq!(listed(&t, "user/hold"), vec![held]);
        let mut both = vec![held, plain];
        both.sort();
        assert_eq!(listed(&t, "priority/high"), both);
        assert!(
            listed(&t, "user/hol").is_empty(),
            "exact names do not prefix-match"
        );

        // A label change on GitHub lands in the mirror only; the sweep
        // carries it into the cache without any queue transition.
        set_mirror_labels(&conn, 14171, r#"["user/hold"]"#);
        set_mirror_labels(&conn, 14170, r#"[]"#);
        let sync = verb_labels_sync(&t, &json!({}));
        assert_eq!(sync["ok"], true, "{sync}");
        assert_eq!(sync["row"]["scanned"], 3, "{sync}");
        assert_eq!(sync["row"]["with_issue_ref"], 3, "{sync}");
        assert_eq!(sync["row"]["stamped"], 2, "{sync}");
        assert_eq!(listed(&t, "user/*"), vec![plain]);
        assert_eq!(cache(&t, held).0, json!([]));

        // A transition refreshes too: the claim re-reads the mirror.
        set_mirror_labels(&conn, 14170, r#"["user/hold"]"#);
        let c = verb_claim(
            &t,
            &json!({"worker": "w", "queue": QUEUE, "lane": "code-oracle",
                    "session_id": "ses_labels", "model_provider": "anthropic",
                    "model_id": "claude-haiku-5-5", "model_variant": "default"}),
        );
        assert_eq!(c["ok"], true, "{c}");
        assert_eq!(
            c["row"]["id"].as_i64(),
            Some(held),
            "oldest equal-priority task is claimed first: {c}"
        );
        assert_eq!(cache(&t, held).0, json!(["user/hold"]));

        // The rich row carries the cache and its stamp for `show`.
        let rich = verb_list(&t, &json!({"rich": true, "task": held, "limit": 1}));
        let row = &list_rows(&rich).unwrap()[0];
        assert_eq!(row["labels"], json!(["user/hold"]), "{rich}");
        assert!(row["labels_synced_at"].is_string(), "{rich}");
    }

    /// The sweep's label pass runs at most hourly unless forced, and
    /// a missing mirror table never fails a transition or the sweep.
    #[test]
    fn label_sweep_is_hourly_and_fail_open() {
        let (t, conn) = fake_backend();
        // No mirror table yet: transitions still succeed.
        let id = enqueue(&t, 14180);
        assert_eq!(cache(&t, id), (json!([]), Value::Null));
        let s = sweep_task_labels(&t, false).expect("first pass reports");
        assert!(
            s.get("error").is_some(),
            "missing mirror is reported, not raised: {s}"
        );

        mirror(&conn);
        set_mirror_labels(&conn, 14180, r#"["user/hold"]"#);
        let s = sweep_task_labels(&t, false).expect("no pass recorded yet, so it runs");
        assert_eq!(s["stamped"], 1, "{s}");
        assert!(
            sweep_task_labels(&t, false).is_none(),
            "within the hour it waits"
        );
        let forced = sweep_task_labels(&t, true).expect("force runs regardless");
        assert_eq!(forced["stamped"], 1, "{forced}");
        assert_eq!(cache(&t, id).0, json!(["user/hold"]));

        let full = verb_sweep(
            &t,
            &json!({"labels_sync_force": true, "dep_recheck": false,
                    "quarantine_recheck": false, "closed_bound": false}),
        );
        assert_eq!(full["ok"], true, "{full}");
        assert_eq!(full["row"]["labels_sync"]["scope"], "actionable", "{full}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod manual_claim_override_audit_tests {
    use super::reprioritize_tests::{events_for, fake_backend};
    use super::*;
    use serde_json::{Value, json};

    const QUEUE: &str = "repo-health-code-pilot";

    fn packet(number: i64) -> Value {
        json!({
            "title": format!("manual claim oracle {number}"),
            "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "manual claim test fixture",
                "priority_reason": "test oracle",
                "evidence": ["fixture"],
                "acceptance": ["fixture"],
                "tests": ["fixture"],
                "context_refs": [],
                "scope": {"include": ["fixture"], "exclude": ["none"]},
                "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45,
                "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-26T00:00:00Z", "basis": "test"}
            }
        })
    }

    #[test]
    fn manual_override_records_full_audit_9178() {
        // Oracle: the prior park state and explicit actor are auditable on
        // both the task result and the claimed event; unrelated result data
        // and historical attempt counts survive.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91781)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        t.exec(
            "UPDATE tasks SET status = 'parked', attempts = 2, max_attempts = 5, \
             not_before = '2027-01-01T00:00:00Z', park_kind = 'backoff', \
             wait_kind = 'dependency', ready_condition = ?, result = ? WHERE id = ?",
            &[
                Arg::Text(json!({"kind": "time", "until": "2027-01-01T00:00:00Z"}).to_string()),
                Arg::Text(
                    json!({"park_reason": "waiting on dependency", "unrelated": {"keep": true}})
                        .to_string(),
                ),
                Arg::Int(id),
            ],
        )
        .unwrap();
        let before = get_task(&t, id).unwrap().expect("parked task");
        assert_eq!(
            claim_guard_reason(&before),
            None,
            "fixture must be claimable: {before}"
        );

        let out = verb_claim(
            &t,
            &json!({
                "worker": "manual-override-worker",
                "queue": QUEUE,
                "lane": "manual-override-lane",
                "task_id": id,
                "session_id": "ses_manual_override",
                "manual_override": "  dependency independently verified  "
            }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], id, "{out}");
        let row = get_task(&t, id).unwrap().expect("claimed task row");
        assert_eq!(row["status"], "claimed");
        assert_eq!(row["attempts"], 2);
        assert_eq!(row["max_attempts"], 5);
        for field in ["not_before", "park_kind", "wait_kind", "ready_condition"] {
            assert!(row[field].is_null(), "{field}: {row}");
        }
        let expected_audit = json!({
            "reason": "dependency independently verified",
            "claimed_at": row["claimed_at"],
            "worker_id": "manual-override-worker",
            "lane_id": "manual-override-lane",
            "session_id": "ses_manual_override",
            "previous_status": "parked",
            "previous_attempts": before["attempts"],
            "previous_max_attempts": before["max_attempts"],
            "previous_not_before": before["not_before"],
            "previous_park_reason": before["result"]["park_reason"],
            "previous_park_kind": before["park_kind"],
            "previous_wait_kind": before["wait_kind"],
            "previous_ready_condition": before["ready_condition"]
        });
        assert_eq!(row["result"]["manual_override"], expected_audit);
        let mut expected_result = before["result"].clone();
        expected_result["manual_override"] = expected_audit.clone();
        assert_eq!(row["result"], expected_result);
        let events = events_for(&t, id, "claimed");
        assert_eq!(events.len(), 1);
        let detail: Value = serde_json::from_str(events[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["manual_override"], expected_audit);
    }

    #[test]
    fn max_attempts_manual_override_preserves_history_and_is_idempotent_9178() {
        // Oracle: a capped parked task refuses without explicit intent;
        // override claims atomically without changing historical attempts,
        // records its full prior state, retries for the same owner are
        // idempotent, and another owner cannot disturb the live claim.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91783)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        t.exec(
            "UPDATE tasks SET status = 'parked', attempts = 5, max_attempts = 5, \
             not_before = '2027-01-01T00:00:00Z', park_kind = 'backoff', \
             wait_kind = 'dependency', result = ? WHERE id = ?",
            &[
                Arg::Text(json!({"park_reason": "retry budget exhausted"}).to_string()),
                Arg::Int(id),
            ],
        )
        .unwrap();
        let before = get_task(&t, id).unwrap().expect("capped parked task");
        assert_eq!(
            claim_guard_reason(&before),
            None,
            "fixture must be claimable: {before}"
        );

        let refused = verb_claim(
            &t,
            &json!({"worker": "max-attempt-worker", "queue": QUEUE, "task_id": id}),
        );
        assert_eq!(
            refused["ok"], true,
            "refusal is a clean no-claim result: {refused}"
        );
        assert!(refused.get("row").is_none_or(Value::is_null), "{refused}");
        assert_eq!(get_task(&t, id).unwrap().unwrap(), before);

        let request = json!({
            "worker": "max-attempt-worker",
            "queue": QUEUE,
            "lane": "max-attempt-lane",
            "task_id": id,
            "session_id": "ses_max_attempt_override",
            "manual_override": "  retry after independent review  "
        });
        let claimed = verb_claim(&t, &request);
        assert_eq!(claimed["ok"], true, "{claimed}");
        assert_eq!(claimed["row"]["attempts"], 5, "{claimed}");
        assert_eq!(claimed["row"]["max_attempts"], 5, "{claimed}");
        let row = get_task(&t, id).unwrap().expect("claimed task");
        let audit = &row["result"]["manual_override"];
        assert_eq!(audit["previous_status"], "parked");
        assert_eq!(audit["previous_attempts"], 5);
        assert_eq!(audit["previous_max_attempts"], 5);
        assert_eq!(audit["previous_not_before"], "2027-01-01T00:00:00Z");
        assert_eq!(audit["previous_park_reason"], "retry budget exhausted");
        assert_eq!(audit["previous_park_kind"], "backoff");
        assert_eq!(audit["previous_wait_kind"], "dependency");
        assert_eq!(audit["reason"], "retry after independent review");
        assert_eq!(audit["worker_id"], "max-attempt-worker");
        assert_eq!(audit["lane_id"], "max-attempt-lane");
        assert_eq!(audit["session_id"], "ses_max_attempt_override");
        assert!(
            audit["claimed_at"]
                .as_str()
                .is_some_and(|at| !at.is_empty())
        );
        let events = events_for(&t, id, "claimed");
        let detail: Value =
            serde_json::from_str(events.last().unwrap()["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["manual_override"], *audit);

        let retry = verb_claim(&t, &request);
        assert_eq!(
            retry["ok"], true,
            "same-owner retry should succeed idempotently: {retry}"
        );
        assert_eq!(retry["row"], row);
        let other_owner = verb_claim(
            &t,
            &json!({
                "worker": "different-worker", "queue": QUEUE, "lane": "other-lane",
                "task_id": id, "session_id": "ses_other",
                "manual_override": "different reason"
            }),
        );
        assert!(
            other_owner.get("row").is_none_or(Value::is_null),
            "{other_owner}"
        );
        let after = get_task(&t, id).unwrap().expect("live claim remains");
        assert_eq!(after["worker_id"], "max-attempt-worker");
        assert_eq!(after["attempts"], 5);
    }

    #[test]
    fn first_boot_honors_manual_override_same_condition_repark_refused_9178() {
        // Oracle for #9178 §2's remaining criterion (FDR-0002): the
        // boot trap — a worker booting under a manual claim whose
        // supervision re-parks for the UNCHANGED recorded condition
        // must keep the task claimed, with the honored override on
        // the refusal record. A materially new condition may still
        // park it, and the new condition is recorded.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91785)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        let claimed = verb_claim(
            &t,
            &json!({"worker": "boot-worker", "queue": QUEUE, "task_id": id}),
        );
        assert_eq!(claimed["ok"], true, "{claimed}");
        let condition = json!({"kind": "dependency", "on": ["queue:42"]});
        let parked = verb_park(
            &t,
            &json!({
                "worker": "boot-worker", "task_id": id,
                "reason": "waiting on dependency", "ready_condition": condition.clone(),
            }),
        );
        assert_eq!(parked["ok"], true, "{parked}");

        let out = verb_claim(
            &t,
            &json!({
                "worker": "boot-worker", "queue": QUEUE, "task_id": id,
                "session_id": "ses_boot_override",
                "manual_override": "dependency independently verified"
            }),
        );
        assert_eq!(out["ok"], true, "{out}");
        let row = get_task(&t, id).unwrap().expect("claimed task row");
        assert_eq!(row["status"], "claimed");
        assert_eq!(row["attempts"], 1);
        let audit = row["result"]["manual_override"].clone();
        assert_eq!(audit["previous_wait_kind"], "external");
        assert_eq!(audit["previous_ready_condition"], condition);

        // First boot: supervision re-evaluates the unchanged
        // condition and re-parks. The override holds.
        let boot = verb_park(
            &t,
            &json!({
                "worker": "boot-worker", "task_id": id,
                "reason": "waiting on dependency", "ready_condition": condition.clone(),
            }),
        );
        assert_eq!(
            boot["ok"], false,
            "same-condition re-park under a live manual override must refuse: {boot}"
        );
        assert!(
            boot["error"]
                .as_str()
                .is_some_and(|e| e.contains("manual-claim override")),
            "{boot}"
        );
        let row = get_task(&t, id).unwrap().expect("task row");
        assert_eq!(row["status"], "claimed");
        assert_eq!(row["worker_id"], "boot-worker");
        assert_eq!(row["attempts"], 1);
        assert_eq!(row["result"]["manual_override"], audit);
        let refusals = events_for(&t, id, "park_refused");
        assert_eq!(refusals.len(), 1, "one recorded refusal");
        let detail: Value = serde_json::from_str(refusals[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["refusal"], "manual_override_honored");
        assert_eq!(detail["manual_override"], audit);

        // A materially new condition may still park, and records
        // itself.
        let new_condition = json!({"kind": "review", "by": "owner"});
        let parked = verb_park(
            &t,
            &json!({
                "worker": "boot-worker", "task_id": id,
                "reason": "needs owner review", "ready_condition": new_condition.clone(),
            }),
        );
        assert_eq!(parked["ok"], true, "{parked}");
        let row = get_task(&t, id).unwrap().expect("parked task row");
        assert_eq!(row["status"], "parked");
        assert_eq!(row["wait_kind"], "review");
        assert_eq!(row["ready_condition"], new_condition);
        assert_eq!(row["result"]["park_reason"], "needs owner review");
    }

    #[test]
    fn first_boot_honors_manual_override_default_timer_repark_refused_16392() {
        // Oracle for #16392: a bare (default-timer) park stores
        // {"kind": "time", "until": T1}; a bare re-park under the
        // manual override mints a FRESH until (T2), so exact
        // ready_condition equality can never hold — yet it is the
        // unchanged wait and the §2 cover must still refuse it.
        // Fixed explicit not_before values keep the repro
        // deterministic (no wall-clock dependence).
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91786)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        let claimed = verb_claim(
            &t,
            &json!({"worker": "timer-boot-worker", "queue": QUEUE, "task_id": id}),
        );
        assert_eq!(claimed["ok"], true, "{claimed}");
        let parked = verb_park(
            &t,
            &json!({
                "worker": "timer-boot-worker", "task_id": id,
                "reason": "supervision backoff", "not_before": "2027-01-01T00:00:00Z",
            }),
        );
        assert_eq!(parked["ok"], true, "{parked}");
        let before = get_task(&t, id).unwrap().expect("parked task row");
        assert_eq!(before["wait_kind"], "timer", "{before}");
        assert_eq!(before["ready_condition"]["kind"], "time", "{before}");

        let out = verb_claim(
            &t,
            &json!({
                "worker": "timer-boot-worker", "queue": QUEUE, "task_id": id,
                "session_id": "ses_timer_boot_override",
                "manual_override": "timer wait independently verified"
            }),
        );
        assert_eq!(out["ok"], true, "{out}");
        let row = get_task(&t, id).unwrap().expect("claimed task row");
        assert_eq!(row["status"], "claimed");
        let audit = row["result"]["manual_override"].clone();
        assert_eq!(audit["previous_wait_kind"], "timer");
        assert_eq!(audit["previous_ready_condition"], before["ready_condition"]);

        // Boot re-parks bare with a later explicit not_before: the
        // stored condition's until differs from the recorded one,
        // but the wait is unchanged — the override must hold.
        let boot = verb_park(
            &t,
            &json!({
                "worker": "timer-boot-worker", "task_id": id,
                "reason": "supervision backoff", "not_before": "2027-06-01T00:00:00Z",
            }),
        );
        assert_eq!(
            boot["ok"], false,
            "default-timer re-park under a live manual override must refuse: {boot}"
        );
        assert!(
            boot["error"]
                .as_str()
                .is_some_and(|e| e.contains("manual-claim override")),
            "{boot}"
        );
        let row = get_task(&t, id).unwrap().expect("task row");
        assert_eq!(row["status"], "claimed");
        assert_eq!(row["result"]["manual_override"], audit);
        let refusals = events_for(&t, id, "park_refused");
        assert_eq!(refusals.len(), 1, "one recorded refusal");
        let detail: Value = serde_json::from_str(refusals[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["refusal"], "manual_override_honored");
    }

    #[test]
    fn ordinary_queued_by_id_claim_keeps_attempt_increment_9178() {
        // Oracle: a normal queued by-ID claim still increments attempts
        // and does not create a manual-override audit record.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91784)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        let claimed = verb_claim(
            &t,
            &json!({"worker": "ordinary-queued-worker", "queue": QUEUE, "task_id": id}),
        );
        assert_eq!(claimed["ok"], true, "{claimed}");
        assert_eq!(claimed["row"]["status"], "claimed");
        assert_eq!(claimed["row"]["attempts"], 1);
        assert!(claimed["row"]["result"].get("manual_override").is_none());
    }

    #[test]
    fn parked_by_id_claim_without_manual_override_still_refuses_9178() {
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91782)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        t.exec(
            "UPDATE tasks SET status = 'parked', attempts = 3, max_attempts = 5, \
             not_before = '2027-01-01T00:00:00Z', park_kind = 'backoff', \
             result = ? WHERE id = ?",
            &[
                Arg::Text(json!({"park_reason": "waiting on dependency"}).to_string()),
                Arg::Int(id),
            ],
        )
        .unwrap();
        let before = get_task(&t, id).unwrap().expect("parked task");
        assert_eq!(
            claim_guard_reason(&before),
            None,
            "fixture must be claimable: {before}"
        );

        let out = verb_claim(
            &t,
            &json!({"worker": "ordinary-worker", "queue": QUEUE, "task_id": id}),
        );
        assert_eq!(out["ok"], true, "refusal is a clean no-claim result: {out}");
        assert!(out.get("row").is_none_or(Value::is_null), "{out}");
        let row = get_task(&t, id).unwrap().expect("parked task row");
        assert_eq!(row["status"], "parked");
        assert_eq!(row["attempts"], 3);
        assert_eq!(row["result"]["park_reason"], "waiting on dependency");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod manual_claim_override_tests {
    use super::reprioritize_tests::fake_backend;
    use super::*;
    use serde_json::{Value, json};

    const QUEUE: &str = "repo-health-code-pilot";

    fn packet(number: i64) -> Value {
        json!({
            "title": format!("manual claim oracle {number}"),
            "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "manual claim test fixture",
                "priority_reason": "test oracle",
                "evidence": ["fixture"],
                "acceptance": ["fixture"],
                "tests": ["fixture"],
                "context_refs": [],
                "scope": {"include": ["fixture"], "exclude": ["none"]},
                "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45,
                "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-26T00:00:00Z", "basis": "test"}
            }
        })
    }

    #[test]
    fn manual_override_claims_parked_task_and_preserves_attempts_9178() {
        // Oracle: a parked row under its cap is claimed with an explicit
        // reason, but its historical attempts and cap remain unchanged.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91781)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        t.exec(
            "UPDATE tasks SET status = 'parked', attempts = 2, max_attempts = 5, \
             not_before = '2027-01-01T00:00:00Z', park_kind = 'backoff', \
             wait_kind = 'dependency', ready_condition = ?, result = ? WHERE id = ?",
            &[
                Arg::Text(json!({"kind": "time", "until": "2027-01-01T00:00:00Z"}).to_string()),
                Arg::Text(json!({"park_reason": "waiting on dependency"}).to_string()),
                Arg::Int(id),
            ],
        )
        .unwrap();
        let before = get_task(&t, id).unwrap().expect("parked task");
        assert_eq!(
            claim_guard_reason(&before),
            None,
            "fixture must be claimable: {before}"
        );

        let out = verb_claim(
            &t,
            &json!({
                "worker": "manual-override-worker",
                "queue": QUEUE,
                "lane": "manual-override-lane",
                "task_id": id,
                "session_id": "ses_manual_override",
                "manual_override": "dependency independently verified"
            }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], id, "{out}");
        let row = get_task(&t, id).unwrap().expect("claimed task row");
        assert_eq!(row["status"], "claimed");
        assert_eq!(row["attempts"], 2);
        assert_eq!(row["max_attempts"], 5);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod resume_session_storage_tests {
    // #9178 §1 (FDR-0001), storage slice: `resume_session_id`
    // persists from packet to row; the claim wire is a later slice,
    // so the claim/pick path must behave exactly as before.
    // Oracle: the task row is the contract.
    use super::reprioritize_tests::{events_for, fake_backend};
    use super::*;
    use crate::validate::{task_from_packet, validate_packet};
    use serde_json::{Value, json};

    const QUEUE: &str = "repo-health-code-pilot";

    fn packet(number: i64) -> Value {
        json!({
            "title": format!("resume storage oracle {number}"),
            "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "resume storage test fixture",
                "priority_reason": "test oracle",
                "evidence": ["fixture"],
                "acceptance": ["fixture"],
                "tests": ["fixture"],
                "context_refs": [],
                "scope": {"include": ["fixture"], "exclude": ["none"]},
                "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45,
                "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-26T00:00:00Z", "basis": "test"}
            }
        })
    }

    fn packet_with_hint(number: i64, hint: Value) -> Value {
        let mut p = packet(number);
        p["resume_session_id"] = hint;
        p
    }

    #[test]
    fn resume_hint_persists_from_packet_to_row_9178() {
        // Oracle: a packet hint lands on the row verbatim and is
        // audit-visible on the enqueued event.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(
            &t,
            &json!({"queue": QUEUE, "packet": packet_with_hint(91801, json!("ses_resume_target"))}),
        );
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        let row = get_task(&t, id).unwrap().expect("task row");
        assert_eq!(
            row["resume_session_id"],
            json!("ses_resume_target"),
            "{row}"
        );
        assert_eq!(
            enqueued["row"]["resume_session_id"],
            json!("ses_resume_target"),
            "{enqueued}"
        );
        let events = events_for(&t, id, "enqueued");
        assert_eq!(events.len(), 1);
        let detail: Value = serde_json::from_str(events[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(
            detail["resume_session_id"],
            json!("ses_resume_target"),
            "{detail}"
        );
    }

    #[test]
    fn absent_hint_reads_null_9178() {
        // Oracle: no hint in the packet means NULL on the row —
        // never an empty string — so "no hint" stays one state.
        let (t, _) = fake_backend();
        let enqueued = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91802)}));
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let id = enqueued["row"]["id"].as_i64().expect("task id");
        let row = get_task(&t, id).unwrap().expect("task row");
        assert!(row["resume_session_id"].is_null(), "{row}");
    }

    #[test]
    fn hint_whitespace_is_trimmed_at_normalization_9178() {
        // Oracle: task_from_packet stores the trimmed hint.
        let task = task_from_packet(&packet_with_hint(91803, json!("  ses_trimmed  ")), QUEUE);
        assert_eq!(task["resume_session_id"], json!("ses_trimmed"), "{task}");
    }

    #[test]
    fn blank_and_non_string_hints_rejected_9178() {
        // Oracle: validate_packet refuses empty, whitespace-only,
        // and non-string hints; absent and null pass.
        for bad in [json!(""), json!("   "), json!(123), json!(true)] {
            let (errors, _) = validate_packet(&packet_with_hint(91804, bad.clone()));
            assert!(
                errors.iter().any(|e| e.contains("resume_session_id")),
                "hint {bad} must be refused: {errors:?}"
            );
        }
        for good in [packet(91805), packet_with_hint(91806, Value::Null)] {
            let (errors, _) = validate_packet(&good);
            assert!(
                !errors.iter().any(|e| e.contains("resume_session_id")),
                "packet must pass resume hint validation: {errors:?}"
            );
        }
    }

    #[test]
    fn hint_changes_nothing_on_the_claim_path_9178() {
        // Oracle: storage without the wire — a hinted task claims
        // through the ordinary by-id path with the hint untouched,
        // and a hint-less task claims exactly as before.
        let (t, _) = fake_backend();
        let hinted = verb_enqueue(
            &t,
            &json!({"queue": QUEUE, "packet": packet_with_hint(91807, json!("ses_not_yet_wired"))}),
        );
        assert_eq!(hinted["ok"], true, "{hinted}");
        let hinted_id = hinted["row"]["id"].as_i64().expect("task id");
        let plain = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(91808)}));
        assert_eq!(plain["ok"], true, "{plain}");
        let plain_id = plain["row"]["id"].as_i64().expect("task id");
        for id in [hinted_id, plain_id] {
            let out = verb_claim(
                &t,
                &json!({"worker": "resume-storage-worker", "queue": QUEUE, "task_id": id}),
            );
            assert_eq!(out["ok"], true, "{out}");
            assert_eq!(out["row"]["id"], id, "{out}");
        }
        let row = get_task(&t, hinted_id).unwrap().expect("claimed row");
        assert_eq!(row["status"], "claimed");
        assert_eq!(
            row["resume_session_id"],
            json!("ses_not_yet_wired"),
            "{row}"
        );
        let plain_row = get_task(&t, plain_id).unwrap().expect("claimed row");
        assert!(plain_row["resume_session_id"].is_null(), "{plain_row}");
    }
}

#[cfg(test)]
mod claim_resume_event_tests {
    // #9178 §1 (FDR-0001), claim-wire slice: a hinted claim's
    // `claimed` event carries both the requested session and the
    // session actually used (explicit null when the claim names
    // none); an unhinted claim's event is byte-identical to before
    // (neither key present). Oracle: the claimed event detail.
    use super::reprioritize_tests::{events_for, fake_backend};
    use super::*;
    use serde_json::{Value, json};

    const QUEUE: &str = "repo-health-code-pilot";

    fn packet(number: i64) -> Value {
        json!({
            "title": format!("resume event oracle {number}"),
            "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "resume event test fixture",
                "priority_reason": "test oracle",
                "evidence": ["fixture"],
                "acceptance": ["fixture"],
                "tests": ["fixture"],
                "context_refs": [],
                "scope": {"include": ["fixture"], "exclude": ["none"]},
                "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45,
                "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-26T00:00:00Z", "basis": "test"}
            }
        })
    }

    fn packet_with_hint(number: i64, hint: &str) -> Value {
        let mut p = packet(number);
        p["resume_session_id"] = json!(hint);
        p
    }

    fn enqueue(number: i64, hint: Option<&str>) -> (crate::turso_queue::TursoBackend, i64) {
        let (t, _) = fake_backend();
        let p = match hint {
            Some(h) => packet_with_hint(number, h),
            None => packet(number),
        };
        let out = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": p}));
        assert_eq!(out["ok"], true, "{out}");
        let id = out["row"]["id"].as_i64().expect("task id");
        (t, id)
    }

    fn claimed_detail(t: &crate::turso_queue::TursoBackend, id: i64) -> Value {
        let events = events_for(t, id, "claimed");
        assert_eq!(events.len(), 1, "{events:?}");
        serde_json::from_str(events[0]["detail"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn hinted_by_id_without_session_records_requested_and_null_actual_9178() {
        let (t, id) = enqueue(91901, Some("ses_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let detail = claimed_detail(&t, id);
        assert_eq!(
            detail["resume_requested_session_id"],
            json!("ses_requested"),
            "{detail}"
        );
        assert!(detail.get("resume_actual_session_id").is_some(), "{detail}");
        assert!(detail["resume_actual_session_id"].is_null(), "{detail}");
    }

    #[test]
    fn hinted_by_id_with_session_records_both_9178() {
        let (t, id) = enqueue(91902, Some("ses_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id, "session_id": "ses_actual"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let detail = claimed_detail(&t, id);
        assert_eq!(
            detail["resume_requested_session_id"],
            json!("ses_requested"),
            "{detail}"
        );
        assert_eq!(
            detail["resume_actual_session_id"],
            json!("ses_actual"),
            "{detail}"
        );
    }

    #[test]
    fn unhinted_by_id_carries_neither_key_9178() {
        let (t, id) = enqueue(91903, None);
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id, "session_id": "ses_actual"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let detail = claimed_detail(&t, id);
        assert!(
            detail.get("resume_requested_session_id").is_none(),
            "{detail}"
        );
        assert!(detail.get("resume_actual_session_id").is_none(), "{detail}");
    }

    #[test]
    fn hinted_by_id_with_fallback_records_reason_9178() {
        let (t, id) = enqueue(91905, Some("ses_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id, "session_id": "ses_actual", "resume_fallback": "missing_session"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let detail = claimed_detail(&t, id);
        assert_eq!(
            detail["resume_requested_session_id"],
            json!("ses_requested"),
            "{detail}"
        );
        assert_eq!(detail["resume_actual_session_id"], json!("ses_actual"), "{detail}");
        assert_eq!(
            detail["resume_fallback"],
            json!("missing_session"),
            "{detail}"
        );
    }

    #[test]
    fn hinted_by_id_without_fallback_records_null_9178() {
        let (t, id) = enqueue(91906, Some("ses_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id, "session_id": "ses_actual"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let detail = claimed_detail(&t, id);
        assert!(detail.get("resume_fallback").is_some(), "{detail}");
        assert!(detail["resume_fallback"].is_null(), "{detail}");
    }

    #[test]
    fn unhinted_by_id_with_fallback_carries_no_resume_keys_9178() {
        let (t, id) = enqueue(91907, None);
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id, "session_id": "ses_actual", "resume_fallback": "missing_session"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let detail = claimed_detail(&t, id);
        assert!(detail.get("resume_requested_session_id").is_none(), "{detail}");
        assert!(detail.get("resume_actual_session_id").is_none(), "{detail}");
        assert!(detail.get("resume_fallback").is_none(), "{detail}");
    }

    #[test]
    fn unknown_resume_fallback_fails_the_claim_9178() {
        let (t, id) = enqueue(91908, Some("ses_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "task_id": id, "resume_fallback": "bogus_reason"}),
        );
        assert_eq!(out["ok"], false, "{out}");
    }

    #[test]
    fn hinted_pick_with_fallback_records_reason_9178() {
        let (t, id) = enqueue(91909, Some("ses_pick_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE, "resume_fallback": "worktree_mismatch"}),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], id, "{out}");
        let detail = claimed_detail(&t, id);
        assert_eq!(
            detail["resume_fallback"],
            json!("worktree_mismatch"),
            "{detail}"
        );
    }

    #[test]
    fn hinted_pick_records_requested_and_null_actual_9178() {
        let (t, id) = enqueue(91904, Some("ses_pick_requested"));
        let out = verb_claim(
            &t,
            &json!({"worker": "resume-event-worker", "queue": QUEUE}),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], id, "{out}");
        let detail = claimed_detail(&t, id);
        assert_eq!(
            detail["resume_requested_session_id"],
            json!("ses_pick_requested"),
            "{detail}"
        );
        assert!(detail["resume_actual_session_id"].is_null(), "{detail}");
    }
}

#[cfg(test)]
mod task_resolution_tests {
    use super::super::{verb_cancel, verb_complete, verb_resolution_backfill, verb_revive};
    use super::reprioritize_tests::fake_backend;
    use super::*;

    const QUEUE: &str = "repo-health-code-pilot";

    fn packet(number: i64) -> Value {
        json!({
            "title": format!("resolution oracle {number}"), "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1", "kind": "small_fix",
                "repo": "developing-today/code", "problem": "p", "priority_reason": "r",
                "evidence": ["e"], "acceptance": ["a"], "tests": ["t"], "context_refs": [],
                "scope": {"include": ["x"], "exclude": ["y"]}, "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45, "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test"}
            }
        })
    }

    fn enqueue_claim(t: &TursoBackend, number: i64) -> i64 {
        let enq = verb_enqueue(t, &json!({"queue": QUEUE, "packet": packet(number)}));
        assert_eq!(enq["ok"], true, "{enq}");
        let claimed = verb_claim(
            t,
            &json!({
                "worker": "resolution-worker", "queue": QUEUE, "lane": "resolution-test",
                "session_id": "ses_resolution", "model_provider": "anthropic",
                "model_id": "claude-haiku-5-5", "model_variant": "default"
            }),
        );
        assert_eq!(claimed["ok"], true, "{claimed}");
        enq["row"]["id"].as_i64().expect("task id")
    }

    fn row(t: &TursoBackend, id: i64) -> Value {
        get_task(t, id).unwrap().expect("task row")
    }

    #[test]
    fn cancel_requires_a_resolution_and_disposition_stamps_verified_done() {
        let (t, _) = fake_backend();
        let cancelled_id = enqueue_claim(&t, 14170);
        let rejected = verb_cancel(
            &t,
            &json!({"task_id": cancelled_id, "worker": "resolution-worker", "reason": "missing label"}),
        );
        assert_eq!(rejected["ok"], false, "{rejected}");
        assert_eq!(row(&t, cancelled_id)["status"], "claimed");

        let cancelled = verb_cancel(
            &t,
            &json!({"task_id": cancelled_id, "worker": "resolution-worker", "reason": "duplicate of task 99", "resolution": "duplicate", "resolution_ref": "task:99"}),
        );
        assert_eq!(cancelled["ok"], true, "{cancelled}");
        assert_eq!(cancelled["row"]["resolution"], "duplicate");
        assert_eq!(cancelled["row"]["resolution_category"], "redirection");
        assert_eq!(cancelled["row"]["resolution_ref"], "task:99");

        let done_id = enqueue_claim(&t, 14171);
        let now = now_iso();
        let complete = verb_complete(
            &t,
            &json!({
                "task_id": done_id, "worker": "resolution-worker",
                "result": {
                    "evidence": "verified closure note",
                    "disposition": {
                        "checked_at": now, "source": "fleet gh readback",
                        "issue": {"number": 14171, "state": "closed", "closed_at": now},
                        "pulls": [], "verdict": "closed_verified"
                    }
                }
            }),
        );
        assert_eq!(complete["ok"], true, "{complete}");
        assert_eq!(complete["row"]["resolution"], "done");
        assert_eq!(complete["row"]["resolution_category"], "success");
        assert_eq!(complete["row"]["resolution_substatus"], "verified");
    }

    #[test]
    fn revive_clears_resolution_but_keeps_the_old_verdict_in_history() {
        let (t, _) = fake_backend();
        let enq = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(14172)}));
        let id = enq["row"]["id"].as_i64().unwrap();
        t.exec(
            "UPDATE tasks SET status='failed', resolution='wontfix', resolution_category='rejection', \
               resolution_substatus='out_of_scope', resolution_reason='legacy decision', \
               resolved_by='resolution-worker', resolved_at='2026-11-24T00:00:00Z' WHERE id=?",
            &[Arg::Int(id)],
        )
        .unwrap();
        let revived = verb_revive(
            &t,
            &json!({"task_id": id, "worker": "resolution-worker", "reason": "scope reopened"}),
        );
        assert_eq!(revived["ok"], true, "{revived}");
        assert!(row(&t, id)["resolution"].is_null());
        let events = t
            .query_objs(
                "SELECT detail FROM events WHERE task_id=? AND kind='revived' ORDER BY id DESC LIMIT 1",
                &[Arg::Int(id)],
                &["detail"],
            )
            .unwrap();
        assert_eq!(
            events[0]["detail"]["resolution_cleared"]["resolution"],
            "wontfix"
        );
        assert_eq!(
            events[0]["detail"]["resolution_cleared"]["reason"],
            "legacy decision"
        );
        let history = t
            .query_objs(
                "SELECT detail FROM task_history WHERE task_id=? AND kind='revived' ORDER BY id DESC LIMIT 1",
                &[Arg::Int(id)],
                &["detail"],
            )
            .unwrap();
        assert_eq!(
            history[0]["detail"]["resolution_cleared"]["resolution"],
            "wontfix"
        );
        assert_eq!(
            history[0]["detail"]["resolution_cleared"]["reason"],
            "legacy decision"
        );
    }

    #[test]
    fn resolution_backfill_only_writes_provable_rows_and_reports_baseline() {
        let (t, _) = fake_backend();
        let verified = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(14173)}));
        let verified_id = verified["row"]["id"].as_i64().unwrap();
        let duplicate = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(14174)}));
        let duplicate_id = duplicate["row"]["id"].as_i64().unwrap();
        t.exec(
            "UPDATE tasks SET status='done', result=? WHERE id=?",
            &[
                Arg::Text(json!({"disposition":{"verdict":"merged_verified"}}).to_string()),
                Arg::Int(verified_id),
            ],
        )
        .unwrap();
        t.exec(
            "UPDATE tasks SET status='cancelled', dedupe_key='repo-health:v1:fix:developing-today/code:issue:14174', result=? WHERE id=?",
            &[Arg::Text(json!({"cancel_reason":"dedupe duplicate"}).to_string()), Arg::Int(duplicate_id)],
        ).unwrap();
        let surviving = verb_enqueue(&t, &json!({"queue": QUEUE, "packet": packet(14175)}));
        let surviving_id = surviving["row"]["id"].as_i64().unwrap();
        t.exec(
            "UPDATE tasks SET dedupe_key='repo-health:v1:fix:developing-today/code:issue:14174' WHERE id=?",
            &[Arg::Int(surviving_id)],
        ).unwrap();
        let out = verb_resolution_backfill(&t, &json!({}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["census"], 2, "{out}");
        assert_eq!(out["row"]["proved"], 2, "{out}");
        assert_eq!(row(&t, verified_id)["resolution_substatus"], "verified");
        assert_eq!(
            row(&t, duplicate_id)["resolution_ref"],
            format!("task:{surviving_id}")
        );
        let again = verb_resolution_backfill(&t, &json!({}));
        assert_eq!(again["row"]["proved"], 0, "idempotent: {again}");
    }
}
// ---------------------------------------------------------------------------
// Tests (#14170 Q4): the task_notes stream against the rusqlite-backed
// fake pipeline. Oracle: two concurrent note adds both survive in
// order; an edit preserves the original in history; the history rows
// a note writes are state-neutral and carry the note pointer.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod task_notes_tests {
    use super::reprioritize_tests::fake_backend;
    use super::*;

    fn task(t: &TursoBackend, worker: Option<&str>) -> i64 {
        t.exec(
            "INSERT INTO tasks (queue, title, payload, priority, status, worker_id, \
             claim_provider, claim_model, claim_variant, claim_session_id, progress, \
             created_at, updated_at) \
             VALUES ('repo-health-code-pilot', 't', '{}', 50, ?, ?, ?, ?, NULL, ?, '{}', \
             '2026-11-25T00:00:00Z', '2026-11-25T00:00:00Z')",
            &[
                Arg::Text(
                    if worker.is_some() {
                        "claimed"
                    } else {
                        "queued"
                    }
                    .into(),
                ),
                arg_str(worker),
                arg_str(worker.map(|_| "anthropic")),
                arg_str(worker.map(|_| "claude-opus")),
                arg_str(worker.map(|_| "ses_claim")),
            ],
        )
        .unwrap()
        .last_insert_rowid
        .unwrap()
    }

    fn add(t: &TursoBackend, task_id: i64, worker: &str, body: &str) -> Value {
        verb_note_add(
            t,
            &json!({"task_id": task_id, "worker": worker, "body": body}),
        )
    }

    fn history(t: &TursoBackend, task_id: i64) -> Vec<Value> {
        t.query_objs(
            "SELECT kind, from_status, to_status, actor, session_id, model_provider, \
             model_id, detail FROM task_history WHERE task_id = ? ORDER BY id",
            &[Arg::Int(task_id)],
            &["detail"],
        )
        .unwrap()
    }

    #[test]
    fn concurrent_note_adds_both_survive_in_order() {
        let (t, _db) = fake_backend();
        let id = task(&t, None);
        let t = std::sync::Arc::new(t);
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let t = t.clone();
                std::thread::spawn(move || {
                    let out = add(&t, id, &format!("w{i}"), &format!("note from w{i}"));
                    assert_eq!(out["ok"], true, "{out}");
                    out["row"]["id"].as_i64().unwrap()
                })
            })
            .collect();
        let mut written: Vec<i64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        written.sort();
        assert_ne!(written[0], written[1], "each add is its own row");
        let listed = verb_note_list(&t, &json!({"task_id": id}));
        let rows = listed["row"]["rows"].as_array().unwrap().clone();
        assert_eq!(listed["row"]["count"], 2);
        assert_eq!(
            rows.iter()
                .map(|r| r["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            written,
            "both notes survive, oldest first"
        );
        let mut authors: Vec<&str> = rows.iter().map(|r| r["author"].as_str().unwrap()).collect();
        authors.sort();
        assert_eq!(authors, ["w0", "w1"]);
        // One state-neutral history row per note, pointing at it.
        let h = history(&t, id);
        assert_eq!(h.len(), 2);
        for (row, note) in h.iter().zip(&rows) {
            assert_eq!(row["kind"], "note_added");
            assert!(row["from_status"].is_null() && row["to_status"].is_null());
            assert_eq!(row["detail"]["note_id"], note["id"]);
            assert_eq!(row["actor"], note["author"]);
        }
    }

    #[test]
    fn edit_preserves_the_original_in_history_and_is_author_only() {
        let (t, _db) = fake_backend();
        let id = task(&t, Some("lane-9"));
        let added = add(&t, id, "lane-9", "first draft");
        assert_eq!(added["ok"], true, "{added}");
        let note = added["row"].clone();
        let note_id = note["id"].as_i64().unwrap();
        // The claim holder's note copies the claim's marks.
        assert_eq!(note["session_id"], "ses_claim");
        assert_eq!(note["model_provider"], "anthropic");
        assert_eq!(note["model_id"], "claude-opus");
        assert_eq!(note["author_kind"], "agent");

        let refused = verb_note_edit(
            &t,
            &json!({"note_id": note_id, "worker": "someone-else", "body": "hijack"}),
        );
        assert_eq!(refused["ok"], false);
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("only its author"),
            "{refused}"
        );

        let edited = verb_note_edit(
            &t,
            &json!({"note_id": note_id, "worker": "lane-9", "body": "second draft"}),
        );
        assert_eq!(edited["ok"], true, "{edited}");
        assert_eq!(edited["row"]["body"], "second draft");
        assert!(!edited["row"]["edited_at"].is_null());

        let h = history(&t, id);
        let kinds: Vec<&str> = h.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            ["note_added", "note_edited"],
            "the refused edit wrote no history"
        );
        assert_eq!(h[1]["detail"]["previous_body"], "first draft");
        assert_eq!(h[1]["detail"]["note_id"], note_id);
        assert_eq!(h[1]["session_id"], "ses_claim");

        let removed = verb_note_remove(&t, &json!({"note_id": note_id, "worker": "lane-9"}));
        assert_eq!(removed["ok"], true, "{removed}");
        assert!(!removed["row"]["deleted_at"].is_null());
        let again = verb_note_remove(&t, &json!({"note_id": note_id, "worker": "lane-9"}));
        assert!(
            again["error"].as_str().unwrap().contains("was removed"),
            "{again}"
        );
        let live = verb_note_list(&t, &json!({"task_id": id}));
        assert_eq!(live["row"]["count"], 0);
        assert_eq!(live["row"]["rows"].as_array().unwrap().len(), 0);
        let all = verb_note_list(&t, &json!({"task_id": id, "include_removed": true}));
        assert_eq!(
            all["row"]["rows"][0]["body"], "second draft",
            "soft delete keeps the row"
        );
        let h = history(&t, id);
        assert_eq!(h[2]["kind"], "note_removed");
        assert_eq!(h[2]["detail"]["previous_body"], "second draft");
    }

    #[test]
    fn note_add_validates_body_author_kind_and_task() {
        let (t, _db) = fake_backend();
        let id = task(&t, None);
        let empty = add(&t, id, "w", "   ");
        assert_eq!(empty["exit"], 2, "{empty}");
        let long = add(&t, id, "w", &"x".repeat(NOTE_BODY_MAX_CHARS + 1));
        assert!(long["error"].as_str().unwrap().contains("4096"), "{long}");
        let at_limit = add(&t, id, "w", &"é".repeat(NOTE_BODY_MAX_CHARS));
        assert_eq!(
            at_limit["ok"], true,
            "the limit counts characters, not bytes"
        );
        let no_author = verb_note_add(&t, &json!({"task_id": id, "body": "b"}));
        assert!(
            no_author["error"]
                .as_str()
                .unwrap()
                .contains("worker is required")
        );
        let bad_kind = verb_note_add(
            &t,
            &json!({"task_id": id, "worker": "w", "body": "b", "author_kind": "robot"}),
        );
        assert_eq!(bad_kind["exit"], 2, "{bad_kind}");
        let missing = add(&t, id + 999, "w", "b");
        assert!(
            missing["error"]
                .as_str()
                .unwrap()
                .contains("does not exist"),
            "{missing}"
        );
        // A non-holder's note carries no claim marks.
        let plain = add(&t, id, "w", "plain");
        assert!(plain["row"]["session_id"].is_null());
        // latest keeps the newest n, oldest first; count stays total.
        add(&t, id, "w", "third");
        let latest = verb_note_list(&t, &json!({"task_id": id, "latest": 2}));
        let bodies: Vec<&str> = latest["row"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["body"].as_str().unwrap())
            .collect();
        assert_eq!(bodies, ["plain", "third"]);
        assert_eq!(latest["row"]["count"], 3);
    }

    #[test]
    fn q6_normalized_link_and_dependency_tables_exist() {
        let (t, _db) = fake_backend();
        let tables = t
            .query_objs(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name IN ('queue_links', 'task_deps') ORDER BY name",
                &[],
                &[],
            )
            .expect("Q6 tables are readable from the SQL store");
        let names: Vec<&str> = tables
            .iter()
            .map(|row| row["name"].as_str().expect("table name is text"))
            .collect();
        assert_eq!(names, ["queue_links", "task_deps"]);
    }

    #[test]
    fn q6_enqueue_seeds_issue_and_pull_request_links() {
        let (t, _db) = fake_backend();
        let packet = json!({
            "title": "Q6 link seed",
            "priority": 50,
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:14181",
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "seed links",
                "evidence": ["fixture"],
                "acceptance": ["issue and PR links are normalized"],
                "scope": {"include": ["Q6"], "exclude": ["other slices"]},
                "tests": ["SQL link seed oracle"],
                "risk": {"level": "low", "flags": []},
                "priority_reason": "fixture",
                "timebox_minutes": 45,
                "max_attempts": 6,
                "context_refs": [],
                "issue": {"number": 14181},
                "pr": {"number": 14441},
                "dedupe": {"checked_at": "2026-11-25T00:00:00Z", "basis": "fixture"}
            }
        });
        let enqueued = verb_enqueue(
            &t,
            &json!({"queue": "repo-health-code-pilot", "packet": packet.clone()}),
        );
        assert_eq!(enqueued["ok"], true, "{enqueued}");
        let task_id = enqueued["row"]["id"].as_i64().expect("task id");
        let links = t
            .query_objs(
                "SELECT other_kind, other_repo, other_number, link_kind FROM queue_links \
                 WHERE task_id = ? AND removed_at IS NULL ORDER BY other_kind",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("normalized links are readable");
        assert_eq!(links.len(), 2, "{links:?}");
        assert_eq!(links[0]["other_kind"], "issue");
        assert_eq!(links[0]["other_number"], 14181);
        assert_eq!(links[0]["link_kind"], "implements");
        assert_eq!(links[1]["other_kind"], "pr");
        assert_eq!(links[1]["other_number"], 14441);
        assert_eq!(links[1]["link_kind"], "pr_for");
        let removed = verb_link_remove(
            &t,
            &json!({"task_id":task_id,"worker":"q6-test","link_kind":"implements",
                "other_issue":{"repo":"developing-today/code","number":14181}}),
        );
        assert_eq!(removed["ok"], true, "{removed}");
        t.exec(
            "DELETE FROM queue_links WHERE task_id = ? AND other_kind = 'pr'",
            &[Arg::Int(task_id)],
        )
        .expect("simulate a transiently failed link seed");
        let retry = verb_enqueue(
            &t,
            &json!({"queue": "repo-health-code-pilot", "packet": packet}),
        );
        assert_eq!(retry["ok"], true, "{retry}");
        assert_eq!(retry["row"]["id"], task_id);
        let repaired = t
            .query_objs(
                "SELECT id FROM queue_links WHERE task_id = ? AND removed_at IS NULL",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("enqueue dedupe retry repairs missing packet links");
        assert_eq!(repaired.len(), 1, "the removed issue link stays removed");
        let tombstone = t
            .query_objs(
                "SELECT removed_at FROM queue_links WHERE task_id = ? AND other_kind = 'issue'",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("soft-removed packet link remains a tombstone");
        assert_eq!(tombstone.len(), 1);
        assert!(!tombstone[0]["removed_at"].is_null());
        t.exec(
            "DELETE FROM queue_schema_migrations WHERE name = 'q6-links-deps-v1'",
            &[],
        )
        .expect("reset the migration marker to exercise the backfill path");
        super::links::ensure_q6_backfill(&t).expect("backfill re-run is safe");
        let still_two = t
            .query_objs(
                "SELECT id FROM queue_links WHERE task_id = ? AND removed_at IS NULL",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("seeded links remain queryable");
        assert_eq!(still_two.len(), 1, "backfill preserves the link tombstone");
    }

    #[test]
    fn q6_sql_census_reproduces_recorded_blocker_counts() {
        let (t, _db) = fake_backend();
        let mut task_id = 1i64;
        let mut open_remaining = 241usize;
        while open_remaining > 0 {
            let count = if task_id <= 63 { 2 } else { 1 };
            let count = count.min(open_remaining);
            let deps: Vec<Value> = (0..count)
                .map(|idx| {
                    let number = if task_id <= 7 && idx == 0 {
                        3399
                    } else {
                        5000 + task_id * 10 + idx as i64
                    };
                    json!({"kind":"issue", "repo":"developing-today/code", "number":number, "state":"open"})
                })
                .collect();
            super::links::sync_task_deps(
                &t,
                task_id,
                "census-fixture",
                &deps,
                "2026-11-24T00:00:00Z",
                "fixture",
            )
            .expect("normalize open census edges");
            open_remaining -= count;
            task_id += 1;
        }
        for offset in 0..50i64 {
            let dep = json!({"kind":"issue", "repo":"developing-today/code", "number":6000 + offset, "state":"cleared", "cleared_at":"2026-11-24T01:00:00Z"});
            super::links::sync_task_deps(
                &t,
                1000 + offset,
                "census-fixture",
                &[dep],
                "2026-11-24T00:00:00Z",
                "fixture",
            )
            .expect("normalize cleared census edges");
        }
        for task in [16024i64, 20343] {
            let malformed = json!({"kind":"task", "state":"open"});
            super::links::sync_task_deps(
                &t,
                task,
                "census-fixture",
                &[malformed],
                "2026-11-24T00:00:00Z",
                "fixture",
            )
            .expect("retain malformed task:None edge");
        }
        let counts = t
            .query_objs(
                "SELECT state, count(*) AS n FROM task_deps GROUP BY state ORDER BY state",
                &[],
                &[],
            )
            .expect("SQL-only blocker state census");
        assert_eq!(
            counts.iter().find(|r| r["state"] == "open").unwrap()["n"],
            241
        );
        assert_eq!(
            counts.iter().find(|r| r["state"] == "cleared").unwrap()["n"],
            50
        );
        assert_eq!(
            counts.iter().find(|r| r["state"] == "malformed").unwrap()["n"],
            2
        );
        let top = t
            .query_objs(
                "SELECT ref_number, count(*) AS n FROM task_deps WHERE state = 'open' \
                 AND ref_number IS NOT NULL GROUP BY ref_number ORDER BY n DESC, ref_number LIMIT 1",
                &[],
                &[],
            )
            .expect("SQL top blocker query");
        assert_eq!(top[0]["ref_number"], 3399);
        assert_eq!(top[0]["n"], 7);
        let tasks = t
            .query_objs(
                "SELECT count(DISTINCT task_id) AS n FROM task_deps WHERE state = 'open'",
                &[],
                &[],
            )
            .expect("SQL distinct blocked task count");
        assert_eq!(tasks[0]["n"], 178);
    }

    #[test]
    fn q6_link_change_emits_history_without_changing_task_status() {
        let (t, _db) = fake_backend();
        let task_id = task(&t, None);
        let request = json!({"task_id":task_id,"worker":"q6-test","link_kind":"parent",
            "other_issue":{"repo":"developing-today/code","number":14170}});
        assert_eq!(verb_link_add(&t, &request)["ok"], true);
        assert_eq!(verb_link_remove(&t, &request)["ok"], true);
        let events = t
            .query_objs(
                "SELECT kind FROM events WHERE task_id = ? AND kind IN ('linked','unlinked') ORDER BY id",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("link transition event rows");
        assert_eq!(
            events
                .iter()
                .map(|r| r["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["linked", "unlinked"]
        );
        let history = t
            .query_objs(
                "SELECT kind, from_status, to_status, \
                 json_extract(detail, '$.link_kind') AS link_kind, \
                 json_extract(detail, '$.other_number') AS other_number \
                 FROM task_history WHERE task_id = ? AND kind IN ('linked','unlinked') ORDER BY id",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("parent change is visible in child history");
        assert_eq!(history.len(), 2);
        for row in &history {
            assert_eq!(row["link_kind"], "parent");
            assert_eq!(row["other_number"], 14170);
            assert!(row["from_status"].is_null());
            assert!(row["to_status"].is_null());
        }
        let task = get_task(&t, task_id).unwrap().unwrap();
        assert_eq!(task["status"], "queued");
    }

    #[test]
    fn q6_block_unblock_keeps_dependency_rows_and_cache_aligned() {
        let (t, _db) = fake_backend();
        let task_id = task(&t, Some("q6-worker"));
        let blocked = verb_block(
            &t,
            &json!({
                "task_id": task_id,
                "worker": "q6-worker",
                "reason_class": "dependency",
                "blocker": "issue is still open",
                "deps": [{"kind":"issue", "repo":"developing-today/code", "number":14170, "reason":"parent work"}]
            }),
        );
        assert_eq!(blocked["ok"], true, "{blocked}");
        let normalized = t
            .query_objs(
                "SELECT dep_kind, ref_repo, ref_number, state FROM task_deps WHERE task_id = ?",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("normalized edge after block");
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0]["dep_kind"], "blocked-by-issue");
        assert_eq!(normalized[0]["ref_number"], 14170);
        assert_eq!(normalized[0]["state"], "open");
        let task_row = get_task(&t, task_id).unwrap().unwrap();
        assert_eq!(task_row["blocked_deps"][0]["number"], 14170);
        let block_id = task_row["dep_blocked_until"].as_str().unwrap().to_owned();

        let unblocked = verb_unblock(
            &t,
            &json!({"task_id": task_id, "worker": "q6-worker", "reason": "parent resolved"}),
        );
        assert_eq!(unblocked["ok"], true, "{unblocked}");
        let normalized = t
            .query_objs(
                "SELECT state, cleared_by FROM task_deps WHERE task_id = ?",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("cleared edge remains in normalized history");
        assert_eq!(normalized[0]["state"], "cleared");
        assert_eq!(normalized[0]["cleared_by"], "q6-worker");
        let task_row = get_task(&t, task_id).unwrap().unwrap();
        assert!(task_row["blocked_deps"].is_null());
        let stale = json!({
            "kind":"issue", "repo":"developing-today/code", "number":14170,
            "state":"open", "recorded_at":"2026-11-25T00:00:00Z"
        });
        let (cache, writes) = super::links::prepare_task_deps(
            task_id,
            &block_id,
            &[stale],
            "2026-11-25T00:00:00Z",
            "stale_recheck_fixture",
        );
        let mut statements = vec![("BEGIN IMMEDIATE", Vec::new())];
        statements.extend(super::links::guard_task_dep_writes(
            writes,
            task_id,
            Some(&block_id),
            &cache,
        ));
        statements.push(("COMMIT", Vec::new()));
        t.exec_batch(&statements)
            .expect("a stale recheck must be a safe no-op after unblock");
        let final_deps = t
            .query_objs(
                "SELECT state FROM task_deps WHERE task_id = ?",
                &[Arg::Int(task_id)],
                &[],
            )
            .expect("final normalized edge remains readable");
        assert_eq!(final_deps[0]["state"], "cleared");
    }
}

// ---------------------------------------------------------------------------
// Tests (#9487, REL-02 design record §10): every holder write is
// conditional on the claim it read. The race hook re-claims the row
// between claimed_by's read and the verb's first `UPDATE tasks`, the
// way a lapsed claim taken by another lane would; the verb must
// refuse with exit 5 and leave the new holder's row, its lease, its
// checkpoint KV and the event log exactly as they were.
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod holder_guard_tests {
    use super::super::verb_complete;
    use super::reprioritize_tests::{events_for, fake_backend_with_race};
    use super::*;

    const QUEUE: &str = "repo-health-code-pilot";
    const WORKER: &str = "guard-worker";

    fn enqueue_claim(t: &TursoBackend, number: i64) -> i64 {
        let packet = json!({
            "title": format!("holder guard {number}"), "priority": 50,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{number}"),
            "payload": {
                "schema": "repo-task/v1", "kind": "small_fix",
                "repo": "developing-today/code", "problem": "p", "priority_reason": "r",
                "evidence": ["e"], "acceptance": ["a"], "tests": ["t"], "context_refs": [],
                "scope": {"include": ["x"], "exclude": ["y"]}, "risk": {"level": "low", "flags": []},
                "timebox_minutes": 45, "issue": {"number": number},
                "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test"}
            }
        });
        let enq = verb_enqueue(t, &json!({"queue": QUEUE, "packet": packet}));
        assert_eq!(enq["ok"], true, "{enq}");
        let id = enq["row"]["id"].as_i64().expect("task id");
        let claimed = verb_claim(
            t,
            &json!({
                "worker": WORKER, "queue": QUEUE, "lane": "guard-lane-a",
                "session_id": "ses_guard", "model_provider": "anthropic",
                "model_id": "claude-haiku-5-5", "model_variant": "default"
            }),
        );
        assert_eq!(claimed["ok"], true, "{claimed}");
        assert_eq!(claimed["row"]["id"].as_i64(), Some(id), "{claimed}");
        id
    }

    /// The other lane's re-claim, applied inside the race window.
    fn reclaim_sql(id: i64) -> String {
        format!(
            "UPDATE tasks SET claimed_at = '2099-01-01T00:00:00Z', lane_id = 'guard-lane-b', \
             attempts = attempts + 1, progress = '{{\"holder\":\"lane-b\"}}' WHERE id = {id}"
        )
    }

    fn count(t: &TursoBackend, sql: &str, id: i64) -> i64 {
        let rows = t.query_objs(sql, &[Arg::Int(id)], &[]).unwrap();
        rows[0]["n"].as_i64().unwrap()
    }

    fn snapshot(t: &TursoBackend, id: i64) -> (Value, i64, i64, i64) {
        (
            get_task(t, id).unwrap().expect("task row"),
            count(t, "SELECT COUNT(*) AS n FROM leases WHERE task_id = ?", id),
            count(t, "SELECT COUNT(*) AS n FROM events WHERE task_id = ?", id),
            count(
                t,
                "SELECT COUNT(*) AS n FROM checkpoints WHERE key = 'task:' || ?",
                id,
            ),
        )
    }

    fn complete_req(id: i64, number: i64) -> Value {
        let now = now_iso();
        json!({
            "task_id": id, "worker": WORKER,
            "result": {
                "evidence": "verified closure note",
                "disposition": {
                    "checked_at": now, "source": "fleet gh readback",
                    "issue": {"number": number, "state": "closed", "closed_at": now},
                    "pulls": [], "verdict": "closed_verified"
                }
            }
        })
    }

    type Verb = fn(&TursoBackend, &Value) -> Value;
    type Req = fn(i64, i64) -> Value;

    fn cases() -> Vec<(&'static str, Verb, Req)> {
        vec![
            (
                "release_repo_task",
                verb_release as Verb,
                (|id, _| json!({"task_id": id, "worker": WORKER, "reason": "guard test"})) as Req,
            ),
            (
                "park_task",
                verb_park as Verb,
                (|id, _| json!({"task_id": id, "worker": WORKER, "reason": "guard test", "delay_seconds": 1200}))
                    as Req,
            ),
            (
                "checkpoint_task",
                verb_checkpoint as Verb,
                (|id, _| json!({"task_id": id, "worker": WORKER, "stage": "guard", "progress": {"step": 1}}))
                    as Req,
            ),
            ("complete_task", verb_complete as Verb, complete_req as Req),
        ]
    }

    #[test]
    fn holder_writes_refuse_a_claim_changed_after_the_read_9487() {
        let (t, _db, race) = fake_backend_with_race();
        for (n, (verb, call, req)) in cases().into_iter().enumerate() {
            let number = 948_700 + n as i64;
            let id = enqueue_claim(&t, number);
            let (_, leases_before, events_before, _) = snapshot(&t, id);
            *race.lock().unwrap() = Some(reclaim_sql(id));
            let out = call(&t, &req(id, number));
            assert!(
                race.lock().unwrap().is_none(),
                "{verb}: race window reached"
            );
            let (after_row, leases, events, checkpoints) = snapshot(&t, id);
            assert_eq!(out["ok"], false, "{verb}: {out}");
            assert_eq!(out["exit"], 5, "{verb}: {out}");
            assert!(
                out["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains(&format!("{verb}: claim for task {id} changed")),
                "{verb}: {out}"
            );
            // The new holder's claim stands untouched.
            assert_eq!(after_row["status"], "claimed", "{verb}: {after_row}");
            assert_eq!(after_row["lane_id"], "guard-lane-b", "{verb}: {after_row}");
            assert_eq!(after_row["claimed_at"], "2099-01-01T00:00:00Z", "{verb}");
            assert_eq!(after_row["progress"], json!({"holder": "lane-b"}), "{verb}");
            assert!(after_row["resolution"].is_null(), "{verb}: {after_row}");
            assert_eq!(checkpoints, 0, "{verb}: no checkpoint KV write");
            // Nothing after the refused write ran: the lease is
            // still there and no event was appended.
            assert_eq!(leases, leases_before, "{verb}: lease kept");
            assert_eq!(events, events_before, "{verb}: no event appended");
            for kind in ["released", "parked", "checkpoint", "completed"] {
                assert!(
                    events_for(&t, id, kind).is_empty(),
                    "{verb}: no {kind} event"
                );
            }
        }
    }

    #[test]
    fn holder_writes_land_on_an_unchanged_claim_9487() {
        for (n, (verb, call, req)) in cases().into_iter().enumerate() {
            // A fresh store per verb: a released task is queued
            // again, and the next case's claim would pick it.
            let (t, _db, _race) = fake_backend_with_race();
            let number = 948_750 + n as i64;
            let id = enqueue_claim(&t, number);
            let out = call(&t, &req(id, number));
            assert_eq!(out["ok"], true, "{verb}: {out}");
            let row = get_task(&t, id).unwrap().expect("task row");
            let want = match verb {
                "release_repo_task" => "queued",
                "park_task" => "parked",
                "checkpoint_task" => "claimed",
                _ => "done",
            };
            assert_eq!(row["status"], want, "{verb}: {row}");
        }
    }

    #[test]
    fn holder_guard_matches_the_heartbeat_tuple_9487() {
        // Source oracle: the guard is the heartbeat statement's own
        // claim tuple, and each holder verb's write carries it.
        let hb = include_str!("heartbeat.sql");
        for col in [
            "status = 'claimed'",
            "worker_id = ?5",
            "claimed_at IS ?6",
            "lane_id IS ?7",
            "attempts = ?8",
        ] {
            assert!(hb.contains(col), "{col}");
        }
        for col in [
            "status = 'claimed'",
            "worker_id = ?",
            "claimed_at IS ?",
            "lane_id IS ?",
            "attempts = ?",
        ] {
            assert!(HOLDER_GUARD.contains(col), "{col}");
        }
        assert_eq!(
            HOLDER_GUARD.matches('?').count(),
            holder_guard_args("w", &json!({})).len()
        );
    }
}
