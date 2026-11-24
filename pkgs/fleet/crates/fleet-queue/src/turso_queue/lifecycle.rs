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

/// Whether an EXISTING ledger row flips to direct (#8187): only an
/// opencode row, only when the caller passes --exclusion-reason
/// (a Muse lane taking a sink/legacy opencode task direct), and
/// never once a session is bound (session_id set) — a bound row's
/// assignment is history, not a routing decision. Claimant flows
/// that pass no exclusion keep their existing assignment.
pub(crate) fn lane_reassignment_flip(
    assigned_lane: &str,
    session_id: Option<&str>,
    exclusion: Option<&str>,
) -> bool {
    assigned_lane == "opencode"
        && exclusion.is_some()
        && session_id.map(str::trim).unwrap_or("").is_empty()
}

/// Whether an EXISTING direct ledger row flips to opencode
/// (#8210): only a direct row, only when the caller passes
/// --opencode-reason (a Muse session sending its task to
/// opencode with a stated reason), and never once a session is
/// bound — the mirror of lane_reassignment_flip. Before #8210 the
/// existing-row path of verb_lane only flipped opencode -> direct,
/// so a lane that had claimed a task (fresh direct ledger row)
/// had no way to record an opencode assignment on it at all.
pub(crate) fn lane_flip_to_opencode(
    assigned_lane: &str,
    session_id: Option<&str>,
    opencode_reason: Option<&str>,
) -> bool {
    assigned_lane == "direct"
        && !opencode_reason.map(str::trim).unwrap_or("").is_empty()
        && session_id.map(str::trim).unwrap_or("").is_empty()
}

/// Platform limitation validity (#8210 — Drew: a task may be
/// limited to muse or opencode "but better be good reason"). A
/// limitation is an EXCEPTION beside the open default: it exists
/// only with a reason recorded next to it. A stored limitation
/// whose reason is blank is invalid — never enforced; the
/// transfer path clears it and records the clearing.
pub(crate) fn platform_limit_invalid(limit: Option<&str>, reason: Option<&str>) -> bool {
    !limit.map(str::trim).unwrap_or("").is_empty() && reason.map(str::trim).unwrap_or("").is_empty()
}

/// Why a requested limitation cannot be recorded, or None when
/// it can: the family must be a real platform family (#8187's
/// vocabulary), and the reason must be present and non-blank —
/// the lane verb refuses the set otherwise. A reason without a
/// limitation is likewise refused (it would record nothing).
pub(crate) fn platform_limit_error(limit: Option<&str>, reason: Option<&str>) -> Option<String> {
    let l = limit.map(str::trim).unwrap_or("");
    let r = reason.map(str::trim).unwrap_or("");
    if l.is_empty() {
        if !r.is_empty() {
            return Some("platform_limit_reason requires a platform_limit to limit".to_string());
        }
        return None;
    }
    if let Err(e) = parse_platform(Some(l)) {
        return Some(e);
    }
    if r.is_empty() {
        return Some(format!(
            "a platform limitation requires a recorded reason \
             (--limit-reason): a task is limited to '{l}' only for \
             a good reason, recorded on the task"
        ));
    }
    None
}

/// Transfer refusal for a platform limitation (#8210), or None
/// when the limitation does not bind this claim: no limitation,
/// an invalid (reason-less) limitation — those are cleared, not
/// enforced — or a limitation naming the claimer's own family.
/// A valid limitation refuses every other family, and refuses a
/// claim that declares no family at all (it cannot show it is
/// the allowed one). The refusal names the limitation and its
/// recorded reason.
pub(crate) fn platform_limit_refusal(
    limit: Option<&str>,
    reason: Option<&str>,
    family: Option<&str>,
) -> Option<String> {
    let l = limit.map(str::trim).unwrap_or("");
    if l.is_empty() || platform_limit_invalid(limit, reason) {
        return None;
    }
    let r = reason.map(str::trim).unwrap_or("");
    match family {
        Some(f) if f == l => None,
        Some(f) => Some(format!(
            "task is limited to platform '{l}' (reason: {r}); \
             a '{f}' transfer-claim cannot take it"
        )),
        None => Some(format!(
            "task is limited to platform '{l}' (reason: {r}); \
             the transfer-claim must declare its platform family to take it"
        )),
    }
}

/// Session-to-session transfer refusal (#8210), or None when the
/// transfer may proceed. Pure over the gathered marks so the
/// tests pin every guard without a live store; claim_transfer
/// runs this BEFORE any mutation, so a refusal leaves the
/// original claim untouched. Guards, in order:
/// - the row must be claimed under a live lease — terminal rows,
///   queued rows and parked rows (genuine broker wait-parks stay
///   excluded from by-id taking, exactly as by_id_claimable has
///   them) are not transfer sources;
/// - from_session must match a holder session mark: the bound
///   progress.session_id, a progress.from_session the holder
///   checkpointed, or the session on the task's latest claimed
///   event — only the current holder can give a task away;
/// - a row already bound to a session that is neither the passer
///   nor the claiming session refuses: that binding is a live
///   launch, not a pass.
pub(crate) fn transfer_refusal(
    row: &Value,
    from_session: &str,
    claimed_event_session: Option<&str>,
    ledger_session: Option<&str>,
    new_session: Option<&str>,
    now: &str,
) -> Option<String> {
    let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "claimed" {
        return Some(format!(
            "task is {status}, not claimed: only a live claim can be transferred"
        ));
    }
    let lease = row
        .get("lease_until")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if lease <= now {
        return Some("the holder's lease is not live: no live claim to transfer".to_string());
    }
    if from_session.trim().is_empty() {
        return Some("from_session is empty".to_string());
    }
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let bound = progress
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let offered = progress
        .get("from_session")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let marks = [bound, offered, claimed_event_session.unwrap_or("")];
    if !marks.iter().any(|m| !m.is_empty() && *m == from_session) {
        return Some(
            "from_session does not match the current holder's session mark: \
             only the current holder can give this task away"
                .to_string(),
        );
    }
    for existing in [bound, ledger_session.unwrap_or("")] {
        if !existing.is_empty() && existing != from_session && Some(existing) != new_session {
            return Some(format!(
                "task is already bound to session {existing}: \
                 a bound launch is not a transfer"
            ));
        }
    }
    None
}

/// The transfer UPDATE (#8210): one conditional statement moves
/// a live claim holder -> claimer. Status stays 'claimed',
/// attempts are NOT touched (a transfer is not a new attempt),
/// and the WHERE re-checks the status and the HOLDER worker read
/// above, so a competing transition in between loses cleanly
/// (0 rows -> refusal, original claim intact).
pub(crate) fn transfer_update_sql() -> &'static str {
    "UPDATE tasks SET worker_id = ?, claimed_at = ?, lease_until = ?, \
       lane_id = ?, platform = COALESCE(?, platform), \
       park_kind = NULL, wait_kind = NULL, not_before = NULL, updated_at = ? \
     WHERE id = ? AND status = 'claimed' AND worker_id = ?"
}

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
            HANDOFF_SQL.to_string()
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
    event(
        t,
        Some(&worker),
        Some(id),
        "claimed",
        &with_marks(
            &json!({
                "lease_seconds": lease_seconds, "via": "claim_repo_task_turso",
                "cleared_session_id": cleared, "lane_id": lane,
                "platform": family, "session_id": claim_session,
                "model_provider": model.as_ref().map(|m| m.provider.clone()),
                "model_id": model.as_ref().and_then(|m| m.model.clone()),
                "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
                "pick": if handoff_pick { "handed_off" } else { "default" },
            }),
            &row,
        ),
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
                &pool,
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
             again before completing it (heartbeat or checkpoint to keep a lease)",
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
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
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
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({"row": row})),
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
        if to_sink {
            t.exec(
                "UPDATE tasks SET status = 'handed_off', worker_id = NULL, lease_until = NULL, \
                   not_before = ?, park_kind = NULL, wait_kind = NULL, ready_condition = NULL, \
                   required_budget_minutes = NULL, \
                   last_checked_at = ?, check_outcome = NULL, progress = ?, result = ?, \
                   updated_at = ? WHERE id = ?",
                &[
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
                ],
            )?;
        } else {
            t.exec(
                "UPDATE tasks SET status = 'parked', worker_id = NULL, lease_until = NULL, \
                   not_before = ?, park_kind = ?, wait_kind = ?, ready_condition = ?, \
                   required_budget_minutes = ?, \
                   last_checked_at = ?, check_outcome = NULL, progress = ?, result = ?, \
                   updated_at = ? WHERE id = ?",
                &[
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
                ],
            )?;
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
/// Holder-only, through the same claimed_by guard as checkpoint.
/// This is the dispatch failure path: an item whose sub-task
/// session failed to launch/bind returns to the pool immediately
/// instead of sitting claimed-idle under the dispatcher.
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
    let row = match claimed_by(t, task_id, &worker, "release_repo_task", req) {
        Ok(r) => r,
        Err(v) => return v,
    };
    let work = (|| -> Result<Value, TErr> {
        // A released claim never leaves a binding behind
        // (migration 0023 §4's rule, mirrored from the claim path).
        let progress = row.get("progress").cloned().unwrap_or(json!({}));
        let (progress, cleared) = clear_binding(&progress, "release_repo_task");
        t.exec(
            "UPDATE tasks SET status = 'queued', worker_id = NULL, \
               lease_until = NULL, not_before = NULL, park_kind = NULL, \
               wait_kind = NULL, progress = ?, updated_at = ? \
             WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(task_id)])?;
        touch_worker(t, &worker, Some(task_id))?;
        if let Some(old) = &cleared {
            on_binding_cleared(t, &worker, task_id, old, "release_repo_task")?;
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "released",
            &with_marks(
                &json!({
                    "reason": reason, "via": "release_repo_task_turso",
                    "cleared_session_id": cleared,
                }),
                &row,
            ),
        )?;
        get_task(t, task_id).map(|r| r.unwrap_or(Value::Null))
    })();
    match work {
        Ok(row) => ok(json!({ "row": row })),
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

    fn fake_backend() -> (TursoBackend, Arc<Mutex<rusqlite::Connection>>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for ddl in SCHEMA {
            conn.execute_batch(ddl).unwrap();
        }
        let db = Arc::new(Mutex::new(conn));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let db_thread = db.clone();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let Ok(sock) = sock else { return };
                let db_conn = db_thread.clone();
                std::thread::spawn(move || handle_conn(sock, db_conn));
            }
        });
        (
            TursoBackend::new(base, "test".to_string(), ureq::agent()),
            db,
        )
    }

    fn handle_conn(mut sock: std::net::TcpStream, db: Arc<Mutex<rusqlite::Connection>>) {
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
            let payload = run_pipeline(&db, &body);
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

    fn run_pipeline(db: &Arc<Mutex<rusqlite::Connection>>, body: &[u8]) -> Value {
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

    fn events_for(t: &TursoBackend, task_id: i64, kind: &str) -> Vec<Value> {
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
}
