//! turso_queue::claim — enqueue, revive, claim model/filter/SQL and lane decision.
//!
//! Split from turso_queue.rs for #8589 (F11.10 production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

pub(crate) fn verb_enqueue(t: &TursoBackend, req: &Value) -> Value {
    let mut packet = req.get("packet").cloned().unwrap_or(Value::Null);
    let metadata = req.get("metadata").cloned().unwrap_or_else(|| json!({}));
    if let Err(e) = crate::validate::validate_metadata(&metadata) {
        return fail(2, format!("enqueue: {e}"));
    }
    let (errors, mut warnings) = validate_packet(&packet);
    if !errors.is_empty() {
        return json!({
            "ok": false, "exit": 2,
            "error": format!("packet validation failed ({} error(s))", errors.len()),
            "errors": errors, "warnings": warnings,
        });
    }
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    if queue.is_empty() {
        return fail(2, "enqueue: queue is required".to_string());
    }
    // Shadow queues are for deliberate shadow runs only (#7016): an
    // enqueue may land in one only when the caller declares shadow
    // intent, so a forgotten --queue can never silently misroute
    // real work there again.
    if queue.ends_with("-shadow") && !req_bool(req, "shadow_intent").unwrap_or(false) {
        return fail(
            2,
            format!(
                "enqueue: queue {queue} is a shadow queue; pass shadow_intent \
                 (CLI: --shadow-intent) to enqueue there deliberately"
            ),
        );
    }
    // #12744: label carriage. No producer writes payload.labels, so
    // the #7030 band selection in task_from_packet never fired on a
    // live packet. Before the row is built, fill the payload's
    // labels from the GitHub mirror in this same store when the
    // packet identifies an issue and carries no labels array of its
    // own (label_enrich.rs has the semantics). The mirror is a read
    // model and never gates work: a failed lookup warns and the
    // enqueue proceeds exactly as it did before this change.
    let dedupe = packet
        .get("dedupe_key")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(payload) = packet.get_mut("payload") {
        if let Err(e) = enrich_payload_labels(t, payload, dedupe.as_deref()) {
            warnings.push(format!(
                "payload.labels: GitHub mirror enrichment skipped ({e})"
            ));
        }
    }
    let mut task = task_from_packet(&packet, &queue);
    // #8097 Rule 2: the optional coarse platform selector. The
    // request field (CLI --platform) wins over the packet's own
    // "platform"; absent both, the row stores NULL (unset — either
    // family may claim it). The selector names a family, never a
    // lane, and is never required.
    let platform = match parse_platform(
        req_str(req, "platform").or_else(|| packet.get("platform").and_then(|v| v.as_str())),
    ) {
        Ok(p) => p,
        Err(e) => return fail(2, format!("enqueue: {e}")),
    };
    if let Some(obj) = task.as_object_mut() {
        obj.insert("platform".to_string(), json!(platform));
    }
    // #15070: the payload's optional structured route declaration.
    // validate_packet has already refused a malformed declaration;
    // this is the broker-side recheck (the broker is the trust
    // boundary) in the platform limitation's own terms, so a routed
    // packet can never land without its limitation.
    let route = packet.get("payload").and_then(route_declaration);
    if let Some((route_platform, route_reason)) = &route {
        if let Some(e) = platform_limit_error(Some(route_platform), Some(route_reason)) {
            return fail(2, format!("enqueue: route: {e}"));
        }
    }
    if req
        .get("dry_run")
        .and_then(|d| d.as_bool())
        .unwrap_or(false)
    {
        return ok(json!({"dry_run": true, "task": task, "warnings": warnings}));
    }
    let now = now_iso();
    let dedupe = task
        .get("dedupe_key")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // #10627: the packet's work identity and any deliberate-parallel
    // override, computed once from the row this enqueue would insert.
    let identity = work_identity(
        &json!({"queue": queue, "payload": task.get("payload").cloned().unwrap_or(json!({}))}),
    );
    let override_reason = dedupe_override(&packet);
    // The tuple is (row, predecessor, work-identity collision,
    // dedupe-key match report). The two reports are the envelope
    // fields a filer reads to learn its filing was absorbed rather
    // than created (#10627).
    let work = (|| -> Result<(Value, Option<Value>, Option<Value>, Option<Value>), TErr> {
        let mut predecessor: Option<Value> = None;
        if let Some(dk) = &dedupe {
            let existing = t.query_objs(
                &format!(
                    "SELECT {TASK_COLS} FROM tasks WHERE queue = ? AND dedupe_key = ? \
                     ORDER BY id ASC"
                ),
                &[Arg::Text(queue.clone()), Arg::Text(dk.clone())],
                &TASK_JSON_COLS,
            )?;
            let (live, terminal) = dedupe_partition(&existing);
            if let Some(row) = live {
                if let Some(id) = row.get("id").and_then(Value::as_i64) {
                    seed_task_links(t, id, &row, "enqueue_repo_task")?;
                }
                let report = dedupe_match_detail(&row, false);
                return Ok((row, None, None, Some(report)));
            }
            // #13091: a COMPLETED predecessor is terminal for the
            // key. The work was done and dispositioned; a re-filing
            // (a generator re-scanning, a stale snapshot) must not
            // manufacture a fresh servable task out of it — the
            // completed row is the dedupe hit, exactly like a live
            // one. Failed and cancelled predecessors keep
            // revive-on-enqueue (#6838), and a declared
            // dedupe_override still files its deliberate parallel
            // below, revive bookkeeping included.
            // #16349: triage is the exception — a triage completion
            // is not the issue being done (#7016 doctrine: the issue
            // goes back in the queue as a fix follow-on unless triage
            // exited it), so a triage re-file over a done predecessor
            // is a reassessment, never re-served work. It always
            // falls through and files a fresh claimable triage task
            // (revive bookkeeping below links it to this
            // predecessor). No fix work is re-served: a fix
            // follow-on the re-triage files carries the deterministic
            // fix key, still absorbed by this same rule; a closed
            // issue retires the revival at claim time (#10626).
            // #16007's narrower recovery (failed/cancelled/missing
            // follow-on only) is subsumed: a done follow-on with the
            // issue still open leaves the issue ownerless the same
            // way, and only the re-triage verdict reconciles it.
            if override_reason.is_none() {
                if let Some(row) = terminal
                    .as_ref()
                    .filter(|r| r.get("status").and_then(|v| v.as_str()) == Some("done"))
                {
                    let kind = task
                        .get("payload")
                        .and_then(|p| p.get("kind"))
                        .and_then(|k| k.as_str())
                        .unwrap_or("");
                    if kind != "triage" {
                        // #10627: the absorption itself is #13091's
                        // standing rule; what was missing is the report.
                        // Name the terminal match in the envelope so a
                        // caller can suffix the key and re-file in the
                        // same pass instead of mistaking the completed
                        // row for the task it asked for.
                        let report = dedupe_match_detail(row, true);
                        return Ok((row.clone(), None, None, Some(report)));
                    }
                }
            }
            predecessor = terminal;
        }
        // #10627: open-collision rule. A live task in any queue that
        // already holds this work identity absorbs the filing: no new
        // row, and the reply names the holder so the filer can route
        // its follow-up onto it. An override files anyway.
        if let (Some(ident), None) = (&identity, &override_reason) {
            let kind = task
                .get("payload")
                .and_then(|p| p.get("kind"))
                .and_then(|k| k.as_str())
                .unwrap_or("");
            if let Some(holder) = live_identity_holder(t, ident, kind)? {
                let detail = collision_detail(&holder, ident);
                return Ok((holder, None, Some(detail), None));
            }
            // #13091: a COMPLETED holder of the identity absorbs
            // the filing the same way. #10627 left terminal rows
            // out so failures could re-queue, and left already-done
            // detection to the claim-time staleness probe — which
            // fails open by standing rule, so while a probe was
            // degraded, completed work was served again and
            // completed a second time (2026-11-23 incidents). The
            // broker knows its own completions locally; no probe
            // is involved. Failed and cancelled holders still
            // never block a filing.
            // #16349: the same triage exception as the dedupe-key
            // check above — a completed triage holder never absorbs
            // a same-work-identity triage re-filing either; the
            // filing falls through and creates the fresh claimable
            // triage task reconciliation needs.
            if kind != "triage" {
                if let Some(holder) = done_identity_holder(t, ident, kind)? {
                    let mut detail = collision_detail(&holder, ident);
                    detail["rule"] = json!("done_work_identity");
                    detail["hint"] = json!(
                        "this work already completed under another task; a completed \
                         task is never re-served (#13091) — file with \
                         payload.dedupe_override=\"<reason>\" for a deliberate parallel"
                    );
                    return Ok((holder, None, Some(detail), None));
                }
            }
        }
        let ins = t.exec(
            "INSERT INTO tasks (queue, manual_only, resume_session_id, title, body, payload, metadata, dedupe_key, priority, \
               status, not_before, max_attempts, required_budget_minutes, platform, progress, result, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, '{}', '{}', ?, ?)",
            &[
                Arg::Text(queue.clone()),
                Arg::Int(if task.get("manual_only").and_then(Value::as_bool).unwrap_or(false) {
                    1
                } else {
                    0
                }),
                arg_str(task.get("resume_session_id").and_then(|v| v.as_str())),
                arg_str(task.get("title").and_then(|v| v.as_str())),
                arg_str(task.get("body").and_then(|v| v.as_str())),
                Arg::Text(
                    task.get("payload").cloned().unwrap_or(json!({})).to_string(),
                ),
                Arg::Text(metadata.to_string()),
                arg_str(dedupe.as_deref()),
                task.get("priority")
                    .and_then(|v| v.as_i64())
                    .map(Arg::Int)
                    .unwrap_or(Arg::Null),
                arg_str(task.get("not_before").and_then(|v| v.as_str())),
                task.get("max_attempts")
                    .and_then(|v| v.as_i64())
                    .map(Arg::Int)
                    .unwrap_or(Arg::Int(
                        crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64,
                    )),
                // #3418: persist the effective requirement in the
                // queryable column (task_from_packet selects
                // packet > payload, absent in both = NULL =
                // legacy claim behavior). The claim predicate
                // COALESCEs column-then-payload, so this stamp
                // and the payload mirror stay equivalent.
                task.get("required_budget_minutes")
                    .and_then(|v| v.as_i64())
                    .map(Arg::Int)
                    .unwrap_or(Arg::Null),
                arg_str(platform.as_deref()),
                Arg::Text(now.clone()),
                Arg::Text(now),
            ],
        )?;
        let id = ins.last_insert_rowid.unwrap_or(0);
        // #15070: translate the packet's structured route declaration
        // into the task's lane-ledger platform limitation at creation,
        // so the claim pick (PLATFORM_LIMIT_PICK_CLAUSE) and the by-id /
        // transfer paths refuse every other family from the moment the
        // task lands — not only after a wrong-family lane draws it and
        // records the limitation at release (#14993). The ledger row
        // carries only the limitation: assigned_lane stays NULL until
        // a real lane assignment (verb_lane / record_execution update
        // the existing row). A packet without a route creates no
        // ledger row and behaves exactly as before.
        if let Some((route_platform, route_reason)) = &route {
            let route_now = now_iso();
            t.exec(
                "INSERT INTO repo_task_lane_ledger \
                 (queue, task_id, platform_limit, platform_limit_reason, assigned_by, assigned_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                &[
                    Arg::Text(queue.clone()),
                    Arg::Int(id),
                    Arg::Text(route_platform.clone()),
                    Arg::Text(route_reason.clone()),
                    Arg::Text("enqueue_route_declaration".to_string()),
                    Arg::Text(route_now.clone()),
                    Arg::Text(route_now),
                ],
            )?;
            event(
                t,
                Some("enqueue_repo_task"),
                Some(id),
                "route_declared",
                &json!({
                    "platform_limit": route_platform,
                    "platform_limit_reason": route_reason,
                    "via": "enqueue_repo_task_turso",
                }),
            )?;
        }
        let mut link_row = task.clone();
        link_row["created_at"] = json!(now_iso());
        event(
            t,
            Some("enqueue_repo_task"),
            Some(id),
            "enqueued",
            &json!({"queue": queue, "dedupe_key": dedupe, "platform": platform, "manual_only": task.get("manual_only").and_then(Value::as_bool).unwrap_or(false), "resume_session_id": task.get("resume_session_id").cloned().unwrap_or(Value::Null)}),
        )?;
        seed_task_links(t, id, &link_row, "enqueue_repo_task")?;
        // Revive-on-enqueue (#6838): the key's only prior row(s) are
        // terminal, so this fresh task is a revival, not a first
        // enqueue. Link it to its predecessor in progress (the audit
        // chain) and leave an event on BOTH rows.
        if let Some(pred) = &predecessor {
            let pred_id = pred.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let link = json!({
                "task_id": pred_id,
                "status": pred.get("status").cloned().unwrap_or(Value::Null),
                "completed_at": pred.get("completed_at").cloned().unwrap_or(Value::Null),
                "revived_at": now_iso(),
                "via": "enqueue_repo_task_turso",
            });
            t.exec(
                "UPDATE tasks SET progress = ? WHERE id = ?",
                &[
                    Arg::Text(json!({"revived_from": link}).to_string()),
                    Arg::Int(id),
                ],
            )?;
            event(
                t,
                Some("enqueue_repo_task"),
                Some(id),
                "task_revived",
                &json!({"predecessor": link, "dedupe_key": dedupe}),
            )?;
            event(
                t,
                Some("enqueue_repo_task"),
                Some(pred_id),
                "revived_by",
                &json!({"successor_task_id": id, "dedupe_key": dedupe}),
            )?;
        }
        if let Some(reason) = &override_reason {
            event(
                t,
                Some("enqueue_repo_task"),
                Some(id),
                "dedupe_override",
                &json!({
                    "reason": reason,
                    "identity": identity.as_ref().map(WorkIdentity::key),
                }),
            )?;
        }
        let row = get_task(t, id).map(|r| r.unwrap_or(Value::Null))?;
        // #12966 (proposal step 5 of #10626): the filing-time state
        // re-check. A packet whose issue stamp is older than the
        // bound is re-verified now, at filing, instead of riding
        // the queue until the claim-time check catches it: when the
        // linked work already landed, the fresh row retires in
        // place with the same evidence and event the claim path
        // writes. A fresh or missing stamp files exactly as before.
        if let Some(stale) = filing_recheck(t, &row) {
            retire_stale(
                t,
                "enqueue_repo_task",
                &row,
                &stale,
                "enqueue_staleness_check",
            )?;
            let row = get_task(t, id).map(|r| r.unwrap_or(Value::Null))?;
            return Ok((row, predecessor, None, None));
        }
        Ok((row, predecessor, None, None))
    })();
    match work {
        Ok((row, predecessor, collision, dedupe_match)) if !row.is_null() => {
            let mut envelope = json!({
                "row": row, "rpc": "enqueue_repo_task_turso", "warnings": warnings
            });
            if let Some(c) = collision {
                envelope["work_identity_collision"] = c;
            }
            if let Some(m) = dedupe_match {
                envelope["dedupe_match"] = m;
            }
            if let Some(pred) = &predecessor {
                envelope["revived_from"] = json!({
                    "task_id": pred.get("id").cloned().unwrap_or(Value::Null),
                    "status": pred.get("status").cloned().unwrap_or(Value::Null),
                });
            }
            ok(envelope)
        }
        Ok(_) => fail(5, "enqueue_repo_task_turso returned no row"),
        Err(e) => terr(e),
    }
}

/// Whether a task row may be revived by the /revive verb (#6838):
/// a failed row can always be re-queued; a queued row only when it
/// is dead at cap (attempts exhausted — the silent-zombie state
/// #6463 censused). Anything else is live or terminal-done work the
/// verb must not touch.
pub(crate) fn revive_eligible(
    status: &str,
    attempts: i64,
    max_attempts: i64,
) -> Result<(), String> {
    match status {
        "failed" => Ok(()),
        "queued" if attempts >= max_attempts => Ok(()),
        "queued" => Err(format!(
            "task is queued with attempts {attempts} < max_attempts {max_attempts}: \
             not dead at cap, nothing to revive"
        )),
        other => Err(format!(
            "task status {other} cannot be revived (only failed, or queued dead-at-cap)"
        )),
    }
}

/// The revive verb (#6838): reset a dead task in place — attempts
/// back to 0, last_error cleared, status queued, due now (or at the
/// caller's not_before) — with a `revived` audit event. This is the
/// supported path the fleet monitor's dead-in-queue self-repair
/// re-points to, replacing its direct SQL.
pub(crate) fn verb_revive(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker")
        .unwrap_or("fleet-queue-cli")
        .to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    // #9179: same storage-layer rule as verb_park — a garbage
    // not_before stored on a revived (queued) row hits the same
    // TEXT comparison in the claim predicate and never lapses.
    if let Some(nb) = req_str(req, "not_before") {
        if let Err(e) = validate_stored_not_before(nb) {
            return fail(2, format!("revive_task: {e}"));
        }
    }
    let not_before = req_str(req, "not_before")
        .map(str::to_string)
        .unwrap_or_else(now_iso);
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id).map_err(|e| e)? else {
            return Err(format!("revive_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let max_attempts = row
            .get("max_attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64);
        revive_eligible(&status, attempts, max_attempts)
            .map_err(|e| format!("revive_task: task {task_id}: {e}"))?;
        let resolution_cleared = row
            .get("resolution")
            .and_then(|v| v.as_str())
            .map(|resolution| {
                json!({
                    "resolution": resolution,
                    "category": row.get("resolution_category").cloned().unwrap_or(Value::Null),
                    "substatus": row.get("resolution_substatus").cloned().unwrap_or(Value::Null),
                    "reason": row.get("resolution_reason").cloned().unwrap_or(Value::Null),
                    "reference": row.get("resolution_ref").cloned().unwrap_or(Value::Null),
                    "resolved_by": row.get("resolved_by").cloned().unwrap_or(Value::Null),
                    "resolved_at": row.get("resolved_at").cloned().unwrap_or(Value::Null),
                })
            });
        t.exec(
            "UPDATE tasks SET status = 'queued', attempts = 0, last_error = NULL, \
               not_before = ?, worker_id = NULL, lease_until = NULL, \
               resolution = NULL, resolution_category = NULL, resolution_substatus = NULL, \
               resolution_reason = NULL, resolution_ref = NULL, resolved_by = NULL, resolved_at = NULL, \
               updated_at = ? WHERE id = ?",
            &[
                Arg::Text(not_before.clone()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )
        .map_err(|e| e)?;
        event(
            t,
            Some(&worker),
            Some(task_id),
            "revived",
            &json!({
                "reason": reason, "previous_status": status,
                "previous_attempts": attempts, "not_before": not_before,
                "resolution_cleared": resolution_cleared,
                "via": "revive_task_turso",
            }),
        )
        .map_err(|e| e)?;
        get_task(t, task_id)
            .map(|r| r.unwrap_or(Value::Null))
            .map_err(|e| e)
    })();
    match work {
        Ok(row) if !row.is_null() => ok(json!({"row": row})),
        Ok(_) => fail(5, format!("revive_task: task {task_id} does not exist")),
        Err(e) if e.starts_with("revive_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

/// #5444: a stored payload counts as missing when it is NULL, or an
/// empty JSON object — the shape a payload-less row hands a worker.
/// (Empty/malformed payload TEXT never reaches here as a Value: the
/// claim candidate predicate filters it in SQL, and the quarantine
/// sweep below handles the stored forms.)
pub(crate) fn payload_missing(parsed: Option<&Value>) -> bool {
    match parsed {
        None => true,
        Some(Value::Null) => true,
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    }
}

/// The #5444 claim guard: a claimed row may be handed out only when
/// its payload is present AND the packet passes the same validator
/// /enqueue runs. The oracle is `validate_packet` — claim and enqueue
/// can never disagree about what a valid packet is. Returns the
/// quarantine reason for a row that must not be handed out.
pub(crate) fn claim_guard_reason(row: &Value) -> Option<String> {
    if payload_missing(row.get("payload")) {
        return Some("quarantined by claim path: payload missing/empty (#5444)".to_string());
    }
    let (errors, _warnings) = validate_packet(row);
    if errors.is_empty() {
        None
    } else {
        Some(format!(
            "quarantined by claim path: packet failed validation (#5444): {}",
            errors.join("; ")
        ))
    }
}

/// Quarantine one task (#5444): blocked with the reason in last_error
/// and a `blocked` event row, claim and lease cleared — the same
/// terminal shape verb_block produces — so a payload-less or
/// schema-invalid row stops circulating instead of crashing or
/// misleading the next worker that claims it.
pub(crate) fn quarantine_task(t: &TursoBackend, id: i64, reason: &str) -> Result<(), TErr> {
    t.exec(
        "UPDATE tasks SET status = 'blocked', worker_id = NULL, \
           lease_until = NULL, last_error = ?, updated_at = ? WHERE id = ?",
        &[
            Arg::Text(reason.to_string()),
            Arg::Text(now_iso()),
            Arg::Int(id),
        ],
    )?;
    t.exec("DELETE FROM leases WHERE task_id = ?", &[Arg::Int(id)])?;
    event(
        t,
        None,
        Some(id),
        "blocked",
        &json!({"reason": reason, "via": "claim_payload_quarantine"}),
    )?;
    Ok(())
}

/// Sweep this queue's claimable rows for stored payload-less forms
/// (NULL / empty / '{}') and quarantine them (#5444 acceptance 1;
/// Turso mirror of migration-0025's quarantine step). Runs on the
/// claim path, which is where such rows would otherwise circulate.
pub(crate) fn quarantine_payload_missing(t: &TursoBackend, queue: &str) -> Result<(), TErr> {
    let rows = t.query_objs(
        "SELECT id FROM tasks WHERE queue = ? AND status IN ('queued', 'parked', 'handed_off') \
           AND (payload IS NULL OR payload = '' OR payload = '{}')",
        &[Arg::Text(queue.to_string())],
        &[],
    )?;
    for r in rows {
        if let Some(id) = r.get("id").and_then(|v| v.as_i64()) {
            quarantine_task(
                t,
                id,
                "quarantined by claim path: payload missing/empty (#5444)",
            )?;
        }
    }
    Ok(())
}

/// The one-queue form of the #5444 payload guard sweep: an
/// unscoped claim (no stream named) sweeps every stream's
/// payload-less rows, exactly as a scoped claim sweeps its own.
pub(crate) fn quarantine_payload_missing_all(t: &TursoBackend) -> Result<(), TErr> {
    let rows = t.query_objs(
        "SELECT id FROM tasks WHERE status IN ('queued', 'parked', 'handed_off') \
           AND (payload IS NULL OR payload = '' OR payload = '{}')",
        &[],
        &[],
    )?;
    for r in rows {
        if let Some(id) = r.get("id").and_then(|v| v.as_i64()) {
            quarantine_task(
                t,
                id,
                "quarantined by claim path: payload missing/empty (#5444)",
            )?;
        }
    }
    Ok(())
}

/// #7617: the claim lease default and the heartbeat extension
/// default are different contracts and must never be conflated: a
/// claim taken without --lease-seconds lives for
/// CLAIM_DEFAULT_LEASE_SECONDS (2700), while a heartbeat without
/// --extend-seconds extends by HEARTBEAT_DEFAULT_EXTEND_SECONDS
/// (900). The 2026-11-08 incident (task 11049) reported
/// lease_until = claimed_at + 2700 while the claim died at ~900 s.
pub(crate) const CLAIM_DEFAULT_LEASE_SECONDS: i64 = 2700;
pub(crate) const HEARTBEAT_DEFAULT_EXTEND_SECONDS: i64 = 900;

/// The lease_until a claim stores — on BOTH the tasks row and the
/// leases row: exactly claimed_at + the requested lease, never the
/// heartbeat extension default.
pub(crate) fn claim_lease_until(claimed_at_unix: i64, lease_seconds: i64) -> String {
    iso(claimed_at_unix + lease_seconds)
}

/// The coarse platform families a platform MARK may name (#8097
/// Rule 2, demoted by #8187): a task's `platform` is NULL (unmarked)
/// or exactly one of these. A platform names a FAMILY, never a
/// specific lane — and since #8187 it is a MARK ONLY: it records
/// which platform is looking at / looked at an item, and is never
/// a claim filter, never an eligibility condition, and never a
/// lane-assignment routing reason.
pub const PLATFORM_FAMILIES: [&str; 2] = ["openchamber", "muse"];

/// Parse an optional platform mark (an enqueue request field, a
/// packet field, or a claim's declared platform). Missing/empty
/// means unmarked; anything that is not one of PLATFORM_FAMILIES
/// is an error, so a typo can never silently corrupt the mark.
pub(crate) fn parse_platform(raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(p) if PLATFORM_FAMILIES.contains(&p) => Ok(Some(p.to_string())),
        Some(p) => Err(format!(
            "invalid platform {p:?} (expected \"openchamber\" or \"muse\")"
        )),
    }
}

/// The claiming session's own model, as a claim request carries it
/// (#8176): provider + model + optional variant — the serving leaf
/// the session read from its own session surface (the route_usage
/// lastServing readout, opencode#1891) at claim time. A claim that
/// names no model carries None and stamps nothing. Claims by
/// non-session actors (Muse lanes) carry the distinct provider
/// marker "muse-lane" with no model, never a fabricated leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimModel {
    pub(crate) provider: String,
    pub(crate) model: Option<String>,
    pub(crate) variant: Option<String>,
}

/// Parse the optional claim-model fields of a claim request
/// (#8176). Provider and model travel together: naming one without
/// the other is a usage error, so a half-stamp can never land. The
/// variant is optional (a leaf need not carry one). Empty strings
/// count as absent.
pub(crate) fn parse_claim_model(req: &Value) -> Result<Option<ClaimModel>, String> {
    let field = |key: &str| {
        req_str(req, key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let provider = field("model_provider");
    let model = field("model_id");
    let variant = field("model_variant");
    match (provider, model) {
        (None, None) => {
            if variant.is_some() {
                return Err(
                    "model_variant requires model_provider and model_id".to_string()
                );
            }
            Ok(None)
        }
        (Some(p), Some(m)) => Ok(Some(ClaimModel {
            provider: p,
            model: Some(m),
            variant,
        })),
        (Some(_), None) => {
            Err("model_provider requires model_id (the claim-model stamp names the serving leaf, provider and model together)".to_string())
        }
        (None, Some(_)) => {
            Err("model_id requires model_provider (the claim-model stamp names the serving leaf, provider and model together)".to_string())
        }
    }
}

/// One model stamp in a task's model timeline (#9046): the pure
/// detail shape every model_stamp event carries — the session the
/// stamp belongs to (null when the stamping surface knew no
/// session; a session is never fabricated), the serving leaf, and
/// the touch that produced it (claim | transfer | bind).
pub(crate) fn model_stamp_detail(m: &ClaimModel, session_id: Option<&str>, trigger: &str) -> Value {
    json!({
        "session_id": session_id,
        "provider": m.provider,
        "model": m.model,
        "variant": m.variant,
        "trigger": trigger,
    })
}

/// #9046: record that a session bound to a task — one half of the
/// binding history whose windows slice the session's raw
/// route_attempt data per task (one session may serve several
/// tasks; each task's window is its own slice). The other half is
/// session_unbound, written by on_binding_cleared and by the
/// terminal verbs that keep the binding on the row
/// (verb_complete / verb_cancel via record_session_unbound).
pub(crate) fn record_session_bound(
    t: &TursoBackend,
    worker: Option<&str>,
    task_id: i64,
    row: &Value,
    session_id: &str,
    trigger: &str,
) -> Result<(), TErr> {
    event(
        t,
        worker,
        Some(task_id),
        "session_bound",
        &with_marks(&json!({"session_id": session_id, "trigger": trigger}), row),
    )
}

/// #9046: record that a task's session binding window closed, for
/// the terminal verbs that keep the binding on the row as the
/// execution record (complete / cancel) — the event ends the
/// window the route_attempt join slices by; the row is untouched.
pub(crate) fn record_session_unbound(
    t: &TursoBackend,
    worker: Option<&str>,
    task_id: i64,
    row: &Value,
    session_id: &str,
    trigger: &str,
) -> Result<(), TErr> {
    event(
        t,
        worker,
        Some(task_id),
        "session_unbound",
        &with_marks(&json!({"session_id": session_id, "trigger": trigger}), row),
    )
}

/// #8176 + #9046: stamp the claiming session's own model on a
/// just-claimed row AND append the stamp to the task's model
/// timeline (a model_stamp event). Claim-time semantics unchanged:
/// the stamp is the leaf serving the session WHEN it claimed; a
/// later failover changes the serving leaf (still visible via the
/// route_attempt join on the stamp's session) but never rewrites
/// history — a transition lands as a later stamp when the fleet
/// next touches the task. The row's claim_* columns, now including
/// claim_session_id (#9046), always hold the LATEST stamp; the
/// events are the history. A claim carrying no model leaves the
/// columns untouched (NULL on a fresh row) and writes no stamp
/// event. The returned row is patched in place so the claim's
/// answer carries the stamp without a re-read.
pub(crate) fn stamp_claim_model(
    t: &TursoBackend,
    task_id: i64,
    row: &mut Value,
    model: &Option<ClaimModel>,
    worker: Option<&str>,
    session_id: Option<&str>,
    trigger: &str,
) -> Result<(), TErr> {
    let Some(m) = model else {
        return Ok(());
    };
    t.exec(
        "UPDATE tasks SET claim_provider = ?, claim_model = ?, \
           claim_variant = ?, claim_session_id = ? WHERE id = ?",
        &[
            Arg::Text(m.provider.clone()),
            arg_str(m.model.as_deref()),
            arg_str(m.variant.as_deref()),
            arg_str(session_id),
            Arg::Int(task_id),
        ],
    )?;
    if let Some(obj) = row.as_object_mut() {
        obj.insert("claim_provider".to_string(), json!(m.provider));
        obj.insert("claim_model".to_string(), json!(m.model));
        obj.insert("claim_variant".to_string(), json!(m.variant));
        obj.insert(
            "claim_session_id".to_string(),
            session_id.map(|s| json!(s)).unwrap_or(Value::Null),
        );
    }
    event(
        t,
        worker,
        Some(task_id),
        "model_stamp",
        &with_marks(&model_stamp_detail(m, session_id, trigger), row),
    )?;
    Ok(())
}

/// The SQL predicate mirroring is_handed_off, for the broker's
/// no-id handoff pick (#8097 Rule 1): a row in the handoff sink —
/// status 'handed_off' (#8234), progress lane opencode, and no
/// session binding. Sink membership is the STATUS, written once
/// by the park verb's classification, the lease-expiry sink
/// return, or the #8234 migration; this predicate deliberately
/// does NOT reconstruct it from progress stage — the broker and
/// the claimant poll once disagreed about which stage source
/// (top-level vs last_checkpoint) named a sink row, and rows
/// stranded in that gap (#8192). The lane and session conditions
/// stay as guards: a bound row is a live session's business, and
/// only an opencode yield is the claimant's to collect.
pub(crate) const HANDOFF_SQL: &str = "status = 'handed_off' \
    AND json_extract(progress, '$.lane') = 'opencode' \
    AND json_extract(progress, '$.session_id') IS NULL";

/// The yield stage a task's progress records: the top-level `stage`
/// when present, else the last checkpoint's stage (the checkpoint
/// verb records it under progress.last_checkpoint.stage, #7706 — the
/// same lift poll.py's normalize_row performs).
pub(crate) fn progress_stage(progress: &Value) -> Option<&str> {
    progress.get("stage").and_then(|v| v.as_str()).or_else(|| {
        progress
            .get("last_checkpoint")
            .and_then(|c| c.get("stage"))
            .and_then(|v| v.as_str())
    })
}

/// The progress shape of a recorded opencode handoff (#8184):
/// the opencode lane assignment plus a yield stage (handed_off, or
/// launch_failed after a claimant recovery), regardless of the
/// row's status or session binding. This shape is the WRITE-side
/// classification: the park verb uses it to decide that a park
/// lands in the sink (status 'handed_off', #8234), and
/// expire_leases uses it to decide that an expired claimed row
/// returns to the sink. It is never used to decide sink
/// MEMBERSHIP for a resting row — that is the status itself
/// (is_handed_off) — because the shape's stage can be read two
/// ways (progress_stage prefers the top-level stage; the
/// claimant's normalize_row lifts last_checkpoint), and #8192 was
/// exactly the two readers disagreeing about a resting row.
/// Historically this shape sat in status 'parked' with the park
/// verb's default +20-minute timer stamped on it; 164 such rows
/// sat in repo-health-code-pilot alone on 2026-11-09 while their
/// issues (#7907, #6533) cycled claim → handoff → park → re-claim
/// for days.
pub(crate) fn handoff_shape(progress: &Value) -> bool {
    progress.get("lane").and_then(|v| v.as_str()) == Some("opencode")
        && matches!(
            progress_stage(progress),
            Some("handed_off") | Some("launch_failed")
        )
}

/// A row is IN the handoff sink (#8234) when its status is
/// 'handed_off', its progress carries the opencode lane, and no
/// session is bound. The status is the membership — written only
/// by the park classification, the lease-expiry sink return, and
/// the #8234 migration, all of which gate on handoff_shape plus
/// an absent binding — so the lane check here is a guard, not a
/// reconstruction, and no stage is consulted at all (#8192).
pub(crate) fn is_handed_off(row: &Value) -> bool {
    if row.get("status").and_then(|v| v.as_str()) != Some("handed_off") {
        return false;
    }
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    if progress.get("lane").and_then(|v| v.as_str()) != Some("opencode") {
        return false;
    }
    match progress.get("session_id") {
        Some(Value::Null) | None => true,
        Some(_) => false,
    }
}

/// Whether a park lands in the handoff sink (#8234): the merged
/// progress is a recorded opencode handoff (handoff_shape), and
/// the caller declared no genuine wait — the ready condition is
/// absent (defaulted by the park verb), or a bare {"kind": "time"}
/// timer, which on a handoff row was only ever the park verb's
/// own default bookkeeping. Any other condition kind (ci/triage
/// floors aside, task_terminal, task_status, event_seen,
/// quota_reset, person_answer) is a real wait the caller named,
/// and that park stays a park. The caller passes the
/// binding-cleared merged progress, so a bound session's row —
/// whose stage may still read as a yield — never sinks.
pub(crate) fn handoff_park(progress: &Value, condition_defaulted: bool, condition: &Value) -> bool {
    if !handoff_shape(progress) {
        return false;
    }
    match progress.get("session_id") {
        Some(Value::Null) | None => {}
        Some(_) => return false,
    }
    condition_defaulted || condition.get("kind").and_then(|v| v.as_str()) == Some("time")
}

/// Where an expired claim returns (#8184): a claimed row whose
/// progress is a recorded opencode handoff goes back to the
/// handoff sink (status 'handed_off', #8234), never to queued —
/// the claimant collects it from the sink at +0 attempts, so a
/// claimant launch that died on the lease can never burn an
/// attempt through the queued interlude. Anything else (a direct
/// claim, a bound session mid-work whose stage advanced past the
/// yield stages) returns to queued as before.
pub(crate) fn expired_claim_returns_to_sink(progress: &Value) -> bool {
    handoff_shape(progress)
}

/// The lease-expiry sink return as a statement builder (#8234),
/// extracted so the regression tests pin its shape without a
/// live store (the claim_update_sql pattern): an expired claimed
/// handoff lands in status 'handed_off' with every wait field
/// NULL — no not_before, no park_kind, no ready_condition. The
/// pre-#8234 return stamped a fresh +1200s not_before and a
/// {"kind": "time"} ready_condition: a fake timer on a state
/// that is not a wait, which the sink pick never read.
pub(crate) fn sink_return_sql() -> &'static str {
    "UPDATE tasks SET status = 'handed_off', worker_id = NULL, lease_until = NULL, \
       not_before = NULL, park_kind = NULL, wait_kind = NULL, \
       ready_condition = NULL, progress = ?, updated_at = ? \
     WHERE id = ? AND status = 'claimed' AND lease_until < ?"
}

/// Whether a by-id claim (claim_repo_task with task_id) may take
/// exactly this row. Two shapes only: a queued row under the pick
/// path's ordinary predicate (attempts cap + not_before), or a
/// sink row (is_handed_off, #8234) — the sink is not a wait, so
/// nothing gates the claimant collecting its own handoff.
/// Anything else (claimed, blocked, a genuine wait park — a
/// 'parked' row is never a sink row since #8234, whatever its
/// progress carries) is not claimable by id.
/// The live holder of a row, when the row is claimed and its lease
/// has not lapsed at `now`: (worker_id, lane_id, lease_until).
/// #6985 recurrence (task 17490, 2026-11-16): two lanes sharing
/// worker_id repo-worker-code both believed they held one task, and
/// nothing in the task's record showed the second claim attempt.
/// A claim surface that refuses a row because a live claim stands
/// must be able to name that claim — from the row's lease fields,
/// never from the claimer's worker identity, which all lanes share.
pub(crate) fn live_claim_holder(
    row: &Value,
    now: &str,
) -> Option<(String, Option<String>, String)> {
    if row.get("status").and_then(|v| v.as_str()) != Some("claimed") {
        return None;
    }
    let lease_until = row.get("lease_until").and_then(|v| v.as_str())?;
    if lease_until < now {
        return None;
    }
    let worker = row
        .get("worker_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let lane = row
        .get("lane_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some((worker, lane, lease_until.to_string()))
}

pub(crate) fn by_id_claimable(row: &Value, now: &str) -> bool {
    // #8187: platform is a mark, not a filter — a by-id claim takes
    // the row whatever platform either side carries (or none).
    match row.get("status").and_then(|v| v.as_str()) {
        Some("queued") => {
            let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
            let max = row
                .get("max_attempts")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let nb = row.get("not_before").and_then(|v| v.as_str());
            attempts < max && nb.map_or(true, |nb| nb <= now)
        }
        Some("handed_off") => is_handed_off(row),
        _ => false,
    }
}

/// #9178 §2: manual by-ID override admits a genuine parked row below
/// its cap (the original slice), or a queued/parked row at its cap.
/// Blocked, live, and terminal rows remain excluded.
pub(crate) fn by_id_manual_claimable(row: &Value) -> bool {
    let status = row.get("status").and_then(Value::as_str).unwrap_or("");
    let attempts = row.get("attempts").and_then(Value::as_i64).unwrap_or(0);
    let max_attempts = row.get("max_attempts").and_then(Value::as_i64).unwrap_or(0);
    if attempts < 0 || max_attempts <= 0 {
        return false;
    }
    (status == "parked" && attempts < max_attempts)
        || (matches!(status, "queued" | "parked") && attempts >= max_attempts)
}

/// Session-to-session transfer (#8210): claim task_id DIRECTLY
/// from its current holder, named by from_session, in one
/// operation — the task never passes through queued or parked,
/// there is no release-and-reclaim gap, and when the request
/// carries a session_id the claiming session is bound in the
/// same operation (progress + lane ledger, the marks a separate
/// bind would write). Callable by the claimant machinery itself:
/// the claiming worker is whoever invokes it, recorded like any
/// claim. Every refusal is computed before the single
/// conditional UPDATE and mutates nothing.
pub(crate) fn claim_transfer(
    t: &TursoBackend,
    task_id: i64,
    queue: Option<&str>,
    worker: &str,
    lane: &Option<String>,
    lease_seconds: i64,
    family: Option<&str>,
    model: &Option<ClaimModel>,
    from_session: &str,
    new_session: Option<&str>,
    pool: &Option<String>,
) -> Result<Option<Value>, TErr> {
    let Some(row) = get_task(t, task_id)? else {
        return Ok(None);
    };
    if let Some(q) = queue {
        if row.get("queue").and_then(|v| v.as_str()) != Some(q) {
            return Ok(None);
        }
    }
    if let Some(reason) = claim_guard_reason(&row) {
        quarantine_task(t, task_id, &reason)?;
        return Ok(None);
    }
    // The holder's session as recorded on the latest claimed
    // event — the mark a session leaves when it claims with
    // --session-id.
    let ev_rows = t.query_objs(
        "SELECT detail FROM events WHERE task_id = ? AND kind = 'claimed' \
         ORDER BY id DESC LIMIT 1",
        &[Arg::Int(task_id)],
        &[],
    )?;
    let ev_detail = ev_rows
        .first()
        .and_then(|e| e.get("detail"))
        .and_then(|d| match d {
            Value::String(s) => serde_json::from_str(s).ok(),
            Value::Object(_) => Some(d.clone()),
            _ => None,
        });
    let claimed_event_session = ev_detail
        .as_ref()
        .and_then(|d| d.get("session_id"))
        .and_then(|v| v.as_str());
    let ledger = get_ledger(t, task_id)?;
    let ledger_session = ledger
        .as_ref()
        .and_then(|l| l.get("session_id"))
        .and_then(|v| v.as_str());
    // Platform limitation (#8210): a recorded, reason-carrying
    // limitation binds this transfer; a reason-less stored one
    // is invalid — cleared here, never enforced, and the
    // clearing is carried on the claimed event below.
    let limit = ledger
        .as_ref()
        .and_then(|l| l.get("platform_limit"))
        .and_then(|v| v.as_str());
    let limit_reason = ledger
        .as_ref()
        .and_then(|l| l.get("platform_limit_reason"))
        .and_then(|v| v.as_str());
    let mut limit_cleared_invalid = false;
    if platform_limit_invalid(limit, limit_reason) {
        t.exec(
            "UPDATE repo_task_lane_ledger SET platform_limit = NULL, \
               platform_limit_reason = NULL, updated_at = ? WHERE task_id = ?",
            &[Arg::Text(now_iso()), Arg::Int(task_id)],
        )?;
        limit_cleared_invalid = true;
    } else if let Some(r) = platform_limit_refusal(limit, limit_reason, family) {
        return Err(format!("claim_repo_task: transfer refused: {r}"));
    }
    // One-queue union matching (#8097, 2026-11-10): the task's
    // own declared requirements (payload `requires`) bind a
    // transfer exactly as they bind the pick and the by-id
    // claim.
    let transfer_claimer = ClaimerCtx {
        family: family.map(str::to_string),
        lane: lane.clone(),
        worker: worker.to_string(),
        pool: pool.clone(),
        model: model.clone(),
    };
    if let Some(r) = task_requirements_unmet(&row, &transfer_claimer) {
        return Err(format!("claim_repo_task: transfer refused: {r}"));
    }
    let now_unix = now_unix();
    let now = iso(now_unix);
    if let Some(reason) = transfer_refusal(
        &row,
        from_session,
        claimed_event_session,
        ledger_session,
        new_session,
        &now,
    ) {
        return Err(format!("claim_repo_task: transfer refused: {reason}"));
    }
    let old_worker = row
        .get("worker_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_lane = row
        .get("lane_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let old_bound = row
        .get("progress")
        .and_then(|p| p.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let new_lane = lane.clone().or_else(|| old_lane.clone());
    let lease_until = claim_lease_until(now_unix, lease_seconds);
    let upd = t.exec(
        transfer_update_sql(),
        &[
            Arg::Text(worker.to_string()),
            Arg::Text(now.clone()),
            Arg::Text(lease_until.clone()),
            arg_str(new_lane.as_deref()),
            arg_str(family),
            Arg::Text(now.clone()),
            Arg::Int(task_id),
            Arg::Text(old_worker.clone()),
        ],
    )?;
    if upd.affected == 0 {
        return Err(
            "claim_repo_task: transfer refused: the claim changed under the transfer".to_string(),
        );
    }
    t.exec(
        "INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id, \
           lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at",
        &[
            Arg::Int(task_id),
            Arg::Text(worker.to_string()),
            Arg::Text(lease_until),
            Arg::Text(now_iso()),
        ],
    )?;
    let mut row = get_task(t, task_id)?.unwrap_or(row);
    // Bind the claiming session in the same operation.
    if let Some(sid) = new_session {
        let progress = merge_obj(
            &row.get("progress").cloned().unwrap_or(json!({})),
            &json!({"session_id": sid, "launched_by": worker}),
        );
        t.exec(
            "UPDATE tasks SET progress = ?, updated_at = ? WHERE id = ?",
            &[
                Arg::Text(progress.to_string()),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
            ],
        )?;
        if ledger.is_some() {
            t.exec(
                "UPDATE repo_task_lane_ledger SET session_id = ?, updated_at = ? \
                 WHERE task_id = ?",
                &[
                    Arg::Text(sid.to_string()),
                    Arg::Text(now_iso()),
                    Arg::Int(task_id),
                ],
            )?;
        }
        if let Some(obj) = row.as_object_mut() {
            obj.insert("progress".to_string(), progress);
        }
    }
    if let Some(sid) = new_session {
        record_session_bound(t, Some(worker), task_id, &row, sid, "transfer")?;
    }
    stamp_claim_model(
        t,
        task_id,
        &mut row,
        model,
        Some(worker),
        new_session,
        "transfer",
    )?;
    touch_worker(t, worker, Some(task_id))?;
    event(
        t,
        Some(worker),
        Some(task_id),
        "claimed",
        &with_marks(
            &json!({
                "lease_seconds": lease_seconds, "via": "claim_repo_task_transfer",
                "transfer": true, "by_id": true, "task_id": task_id,
                "from_session": from_session, "session_id": new_session,
                "previous_worker": old_worker, "previous_lane_id": old_lane,
                "previous_session_id": old_bound,
                "platform_limit": if limit_cleared_invalid { None } else { limit },
                "platform_limit_reason": if limit_cleared_invalid { None } else { limit_reason },
                "platform_limit_cleared_invalid": limit_cleared_invalid,
                "lane_id": new_lane, "platform": family,
                "model_provider": model.as_ref().map(|m| m.provider.clone()),
                "model_id": model.as_ref().and_then(|m| m.model.clone()),
                "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
            }),
            &row,
        ),
    )?;
    Ok(Some(row))
}

/// By-id claim: take exactly the requested row when by_id_claimable
/// allows it, through the same post-claim flow as the pick path
/// (guard quarantine, binding clear, lease row, claimed event).
/// The conditional UPDATE re-checks the status read above, so a
/// competing claim or transition in between loses cleanly (0 rows
/// affected → no claim). Returns the claimed row, or None when the
/// row is not claimable by id.
pub(crate) fn claim_by_id(
    t: &TursoBackend,
    task_id: i64,
    queue: Option<&str>,
    worker: &str,
    lane: &Option<String>,
    lease_seconds: i64,
    family: Option<&str>,
    model: &Option<ClaimModel>,
    claim_session: Option<&str>,
    pool: &Option<String>,
    manual_override: Option<&str>,
) -> Result<Option<Value>, TErr> {
    let Some(row) = get_task(t, task_id)? else {
        return Ok(None);
    };
    if let Some(q) = queue {
        if row.get("queue").and_then(|v| v.as_str()) != Some(q) {
            return Ok(None);
        }
    }
    let row_queue = row
        .get("queue")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(reason) = claim_guard_reason(&row) {
        quarantine_task(t, task_id, &reason)?;
        return Ok(None);
    }
    // #8217/#8456: the plain by-id path honors the same recorded
    // platform limitation the transfer claim enforces (#8210):
    // an invalid (reason-less) stored limitation is cleared,
    // never enforced; a valid one refuses a claim from any
    // other family (or none) — the row simply reads unclaimable
    // by id, like any other by-id refusal. This block must stay
    // OUTSIDE the claim-guard block above: inside it, it sat
    // after that block's `return Ok(None)` and never ran (G7).
    if let Some(ledger) = get_ledger(t, task_id)? {
        let limit = ledger.get("platform_limit").and_then(|v| v.as_str());
        let limit_reason = ledger.get("platform_limit_reason").and_then(|v| v.as_str());
        if platform_limit_invalid(limit, limit_reason) {
            t.exec(
                "UPDATE repo_task_lane_ledger SET platform_limit = NULL, \
               platform_limit_reason = NULL, updated_at = ? WHERE task_id = ?",
                &[Arg::Text(now_iso()), Arg::Int(task_id)],
            )?;
        } else if platform_limit_refusal(limit, limit_reason, family).is_some() {
            return Ok(None);
        }
    }
    let status = row
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now_unix = now_unix();
    let now = iso(now_unix);
    let manual_reason = manual_override
        .map(str::trim)
        .filter(|reason| !reason.is_empty());
    if let Some(reason) = manual_reason {
        let prior = row
            .get("result")
            .and_then(|result| result.get("manual_override"));
        let current_session = row
            .get("progress")
            .and_then(|progress| progress.get("session_id"))
            .and_then(Value::as_str);
        let live_holder = live_claim_holder(&row, &now);
        let same_owner = live_holder
            .as_ref()
            .is_some_and(|(holder_worker, holder_lane, _)| {
                holder_worker == worker && holder_lane == lane
            })
            && current_session == claim_session
            && prior
                .and_then(|audit| audit.get("reason"))
                .and_then(Value::as_str)
                == Some(reason);
        if same_owner {
            return Ok(Some(row));
        }
    }
    let manual = manual_reason.is_some() && by_id_manual_claimable(&row);
    if (manual_reason.is_some() && !manual) || (!manual && !by_id_claimable(&row, &now)) {
        // #6985: a by-id claim refused because a live claim stands
        // leaves a claim-transition event naming the holder's lease
        // (worker, lane, lease_until from the row) and the refused
        // claimer — a refused claim is part of the task's record,
        // not a silent no-op. The pick path's lost races stay
        // event-free by design (ordinary contention, re-picked in
        // the same call); a by-id refusal names its row explicitly,
        // so it is recorded exactly once here.
        if let Some((holder_worker, holder_lane, holder_lease_until)) =
            live_claim_holder(&row, &now)
        {
            let _ = event(
                t,
                Some(worker),
                Some(task_id),
                "claim_refused",
                &json!({
                    "via": "claim_by_id",
                    "reason": "already_claimed",
                    "holder_worker_id": holder_worker,
                    "holder_lane_id": holder_lane,
                    "holder_lease_until": holder_lease_until,
                    "claimer_lane_id": lane,
                }),
            );
        }
        return Ok(None);
    }
    // One-queue union matching (#8097, 2026-11-10): a by-id
    // claim honors the task's own declared requirements
    // (payload `requires`) exactly as the pick does — an
    // ineligible claimer simply reads the row as unclaimable,
    // with no out-of-band step.
    let claimer = ClaimerCtx {
        family: family.map(str::to_string),
        lane: lane.clone(),
        worker: worker.to_string(),
        pool: pool.clone(),
        model: model.clone(),
    };
    if task_requirements_unmet(&row, &claimer).is_some() {
        return Ok(None);
    }
    // Capture every prior park value before the compare-and-swap claim.
    // They remain in the audit record after the live wait fields are cleared.
    let manual_audit = manual_reason.map(|reason| {
        json!({
            "reason": reason,
            "claimed_at": now,
            "worker_id": worker,
            "lane_id": lane,
            "session_id": claim_session,
            "previous_status": status,
            "previous_attempts": row.get("attempts").cloned().unwrap_or(Value::Null),
            "previous_max_attempts": row.get("max_attempts").cloned().unwrap_or(Value::Null),
            "previous_not_before": row.get("not_before").cloned().unwrap_or(Value::Null),
            "previous_park_reason": row.get("result").and_then(|r| r.get("park_reason")).cloned().unwrap_or(Value::Null),
            "previous_park_kind": row.get("park_kind").cloned().unwrap_or(Value::Null),
            "previous_wait_kind": row.get("wait_kind").cloned().unwrap_or(Value::Null),
            "previous_ready_condition": row.get("ready_condition").cloned().unwrap_or(Value::Null),
        })
    });
    let lease_until = claim_lease_until(now_unix, lease_seconds);
    let upd = t.exec(
        "UPDATE tasks SET status = 'claimed', worker_id = ?, claimed_at = ?, \
           lease_until = ?, \
           attempts = attempts + (CASE WHEN status IN ('parked', 'handed_off') OR ? IS NOT NULL THEN 0 ELSE 1 END), \
           lane_id = ?, platform = COALESCE(?, platform), \
           park_kind = NULL, wait_kind = NULL, not_before = NULL, \
           ready_condition = CASE WHEN ? IS NULL THEN ready_condition ELSE NULL END, \
           result = CASE WHEN ? IS NULL THEN result ELSE json_set(COALESCE(result, '{}'), '$.manual_override', json(?)) END, \
           updated_at = ? \
         WHERE id = ? AND status = ? AND queue = ? \
           AND (? IS NULL OR (attempts = ? AND max_attempts = ? AND not_before IS ? \
           AND park_kind IS ? AND wait_kind IS ? \
           AND (ready_condition IS ? OR json(ready_condition) IS json(?)) \
           AND (result IS ? OR json(result) IS json(?))))",
        &[
            Arg::Text(worker.to_string()),
            Arg::Text(now.clone()),
            Arg::Text(lease_until.clone()),
            manual_audit.as_ref().map(|_| Arg::Int(1)).unwrap_or(Arg::Null),
            arg_str(lane.as_deref()),
            arg_str(family),
            manual_audit.as_ref().map(|_| Arg::Int(1)).unwrap_or(Arg::Null),
            manual_audit.as_ref().map(|_| Arg::Int(1)).unwrap_or(Arg::Null),
            manual_audit.as_ref().map(|audit| Arg::Text(audit.to_string())).unwrap_or(Arg::Null),
            Arg::Text(now.clone()),
            Arg::Int(task_id),
            Arg::Text(status),
            Arg::Text(row_queue.clone()),
            manual_audit.as_ref().map(|_| Arg::Int(1)).unwrap_or(Arg::Null),
            Arg::Int(row.get("attempts").and_then(Value::as_i64).unwrap_or(0)),
            Arg::Int(row.get("max_attempts").and_then(Value::as_i64).unwrap_or(0)),
            arg_str(row.get("not_before").and_then(Value::as_str)),
            arg_str(row.get("park_kind").and_then(Value::as_str)),
            arg_str(row.get("wait_kind").and_then(Value::as_str)),
            arg_json(row.get("ready_condition").unwrap_or(&Value::Null)),
            arg_json(row.get("ready_condition").unwrap_or(&Value::Null)),
            arg_json(row.get("result").unwrap_or(&Value::Null)),
            arg_json(row.get("result").unwrap_or(&Value::Null)),
        ],
    )?;
    if upd.affected == 0 {
        return Ok(None);
    }
    let rows = t.query_objs(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
        &[Arg::Int(task_id)],
        &TASK_JSON_COLS,
    )?;
    let Some(mut row) = rows.into_iter().next() else {
        return Ok(None);
    };
    // Migration 0023 §4: a fresh claim never inherits a binding.
    let progress = row.get("progress").cloned().unwrap_or(json!({}));
    let (progress, cleared) = clear_binding(&progress, "claim_repo_task");
    // #9046: a by-id claim that names its session binds it, like
    // the transfer path — the binding is what joins the task to
    // the session's raw opencode data from here on.
    let progress = match claim_session {
        Some(sid) => merge_obj(
            &progress,
            &json!({"session_id": sid, "launched_by": worker}),
        ),
        None => progress,
    };
    if cleared.is_some() || claim_session.is_some() {
        t.exec(
            "UPDATE tasks SET progress = ? WHERE id = ?",
            &[Arg::Text(progress.to_string()), Arg::Int(task_id)],
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
            Arg::Int(task_id),
            Arg::Text(worker.to_string()),
            Arg::Text(lease_until),
            Arg::Text(now_iso()),
        ],
    )?;
    if let Some(sid) = claim_session {
        record_session_bound(t, Some(worker), task_id, &row, sid, "claim")?;
    }
    stamp_claim_model(
        t,
        task_id,
        &mut row,
        model,
        Some(worker),
        claim_session,
        "claim",
    )?;
    touch_worker(t, worker, Some(task_id))?;
    if let Some(old) = &cleared {
        on_binding_cleared(t, worker, task_id, old, "claim_repo_task")?;
    }
    let mut detail = json!({
        "lease_seconds": lease_seconds, "via": "claim_repo_task_turso",
        "cleared_session_id": cleared, "lane_id": lane,
        "by_id": true, "task_id": task_id, "platform": family,
        "model_provider": model.as_ref().map(|m| m.provider.clone()),
        "model_id": model.as_ref().and_then(|m| m.model.clone()),
        "model_variant": model.as_ref().and_then(|m| m.variant.clone()),
    });
    if let Some(audit) = manual_audit {
        detail["manual_override"] = audit;
    }
    // #9178 §1 (FDR-0001), claim-wire slice: when the task carries a
    // resume hint, the claimed event records both the requested
    // session and the session actually used (explicit null when the
    // claim names none), so a silent substitution is impossible.
    // Unhinted tasks emit no new keys: their event is byte-identical
    // to before.
    if let Some(requested) = row.get("resume_session_id").and_then(|v| v.as_str()) {
        detail["resume_requested_session_id"] = json!(requested);
        detail["resume_actual_session_id"] = claim_session.map(|s| json!(s)).unwrap_or(Value::Null);
    }
    event(
        t,
        Some(worker),
        Some(task_id),
        "claimed",
        &with_marks(&detail, &row),
    )?;
    Ok(Some(row))
}

/// The task-side half of the union match: a row whose payload
/// declares `requires` is servable only by a claimer declaring
/// the required attributes. Five placeholders, bound from the
/// claimer (family, lane, pool, "provider/model", model id); an
/// attribute the claimer does not declare binds NULL and meets
/// no requirement — the same shape as the platform limitation's
/// family-less rule (#8217).
pub(crate) const TASK_REQUIREMENTS_CLAUSE: &str = "(json_extract(payload, '$.requires.platform') IS NULL \
        OR json_extract(payload, '$.requires.platform') = ?) \
     AND (json_extract(payload, '$.requires.lane') IS NULL \
        OR json_extract(payload, '$.requires.lane') = ?) \
     AND (json_extract(payload, '$.requires.pool') IS NULL \
        OR json_extract(payload, '$.requires.pool') = ?) \
     AND (json_extract(payload, '$.requires.model') IS NULL \
        OR json_extract(payload, '$.requires.model') = ? \
        OR json_extract(payload, '$.requires.model') = ?)";

/// Who is claiming (#8097 one-queue model): the identity every
/// claim declares and the broker records (the claim stamps —
/// worker, lane, platform family, pool, model). Identity is
/// accountability, not eligibility: it enters matching only as
/// the values a task's own declared requirements are checked
/// against (requirement_args).
#[derive(Debug, Clone, Default)]
pub struct ClaimerCtx {
    pub(crate) family: Option<String>,
    pub(crate) lane: Option<String>,
    pub(crate) worker: String,
    pub(crate) pool: Option<String>,
    pub(crate) model: Option<ClaimModel>,
}

impl ClaimerCtx {
    /// The five requirement bindings, in TASK_REQUIREMENTS_CLAUSE
    /// order: family, lane, pool, "provider/model", model id.
    fn requirement_args(&self) -> Vec<Arg> {
        let (full, bare) = match &self.model {
            Some(m) => (
                m.model.as_ref().map(|id| format!("{}/{}", m.provider, id)),
                m.model.clone(),
            ),
            None => (None, None),
        };
        vec![
            arg_str(self.family.as_deref()),
            arg_str(self.lane.as_deref()),
            arg_str(self.pool.as_deref()),
            arg_str(full.as_deref()),
            arg_str(bare.as_deref()),
        ]
    }
}

/// Whether a lanes-table registration binds its lane to one
/// stream under the one-queue model: any recorded queue value
/// binds EXCEPT the unbound markers ("*", "all", ""), which are
/// the registry's recorded one-queue default — the lane may
/// claim any stream, and unscoped claims never consult the
/// registration at all.
pub(crate) fn lane_queue_binds(registered: &str) -> bool {
    !matches!(registered, "*" | "all" | "")
}

/// A claim's full scope: the compiled claimer-side conditions
/// (the optional stream scope and filter expression) plus the
/// task-side requirements clause, with their arguments in SQL
/// placeholder order. The scope SQL is appended after the pick
/// predicate and the platform limitation clause in BOTH the
/// pick SELECT and the claim UPDATE re-check, so a claim can
/// never be handed a row its own scope excluded or whose
/// requirements it does not meet — the #7068 discipline,
/// extended to the union match.
#[derive(Clone)]
pub struct ClaimScope {
    pub(crate) sql: String,
    pub(crate) args: Vec<Arg>,
}

pub fn claim_scope(
    queue: Option<&str>,
    filter: Option<&FilterExpr>,
    claimer: &ClaimerCtx,
) -> ClaimScope {
    let mut parts: Vec<String> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(q) = queue {
        parts.push("queue = ?".to_string());
        args.push(Arg::Text(q.to_string()));
    }
    if let Some(f) = filter {
        parts.push(format!("({})", filter_sql(f, &mut args)));
    }
    parts.push(TASK_REQUIREMENTS_CLAUSE.to_string());
    args.extend(claimer.requirement_args());
    ClaimScope {
        sql: format!(" AND {}", parts.join(" AND ")),
        args,
    }
}

/// The task row's payload as a JSON object: query_objs returns
/// payload already parsed; a raw string is parsed here; anything
/// else reads as an empty object (no metadata, no requirements).
pub(crate) fn task_payload(row: &Value) -> Value {
    match row.get("payload") {
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    }
}

/// The task's tracking repo identity (not implementation eligibility): the
/// payload's declared repo when present and non-empty, else the
/// stream name with the repo-health- prefix and -pilot / -shadow
/// suffixes stripped.
pub fn task_repo_identity(row: &Value) -> String {
    let payload = task_payload(row);
    if let Some(r) = payload.get("repo").and_then(|v| v.as_str()) {
        if !r.is_empty() {
            return r.to_string();
        }
    }
    let queue = row.get("queue").and_then(|v| v.as_str()).unwrap_or("");
    match queue.strip_prefix("repo-health-") {
        Some(rest) => rest.replace("-pilot", "").replace("-shadow", ""),
        None => queue.to_string(),
    }
}

/// Claim filters use the explicit implementation target, not the issue host.
pub fn task_implementation_repo(row: &Value) -> String {
    task_payload(row)
        .get("implementation_repo")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| task_repo_identity(row))
}

pub(crate) fn repo_value_matches(identity: &str, value: &str) -> bool {
    identity == value || identity.ends_with(&format!("/{value}"))
}

pub(crate) fn filter_subject(field: FilterField, row: &Value, payload: &Value) -> Option<String> {
    let s = |v: Option<&Value>| v.and_then(|x| x.as_str()).map(str::to_string);
    match field {
        FilterField::Queue | FilterField::Pool => s(row.get("queue")),
        FilterField::Platform => s(row.get("platform")),
        FilterField::Kind => s(payload.get("kind")),
        FilterField::Lane => s(payload.get("requires").and_then(|r| r.get("lane"))),
        FilterField::Model => s(payload.get("requires").and_then(|r| r.get("model"))),
        FilterField::Repo | FilterField::Label | FilterField::Priority => None,
    }
}

pub(crate) fn cmp_matches(field: FilterField, op: CmpOp, values: &[String], row: &Value) -> bool {
    let payload = task_payload(row);
    match field {
        FilterField::Repo => {
            let identity = task_implementation_repo(row);
            let any = values.iter().any(|v| repo_value_matches(&identity, v));
            match op {
                CmpOp::Eq | CmpOp::In => any,
                CmpOp::Ne | CmpOp::NotIn => !any,
                _ => false,
            }
        }
        FilterField::Label => {
            let labels: Vec<&str> = payload
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let any = values.iter().any(|v| labels.contains(&v.as_str()));
            match op {
                CmpOp::Eq | CmpOp::In => any,
                CmpOp::Ne | CmpOp::NotIn => !any,
                _ => false,
            }
        }
        FilterField::Priority => {
            let subj = row.get("priority").and_then(|v| v.as_i64());
            let nums: Vec<i64> = values.iter().filter_map(|v| v.parse().ok()).collect();
            match op {
                CmpOp::Eq => subj.is_some() && nums.iter().any(|n| Some(*n) == subj),
                CmpOp::Ne => subj.map_or(true, |s| nums.iter().all(|n| *n != s)),
                CmpOp::Lt => matches!((subj, nums.first()), (Some(s), Some(n)) if s < *n),
                CmpOp::Le => matches!((subj, nums.first()), (Some(s), Some(n)) if s <= *n),
                CmpOp::Gt => matches!((subj, nums.first()), (Some(s), Some(n)) if s > *n),
                CmpOp::Ge => matches!((subj, nums.first()), (Some(s), Some(n)) if s >= *n),
                CmpOp::In => subj.is_some() && nums.contains(&subj.unwrap_or(0)),
                CmpOp::NotIn => subj.map_or(true, |s| !nums.contains(&s)),
            }
        }
        field => {
            let subj = filter_subject(field, row, &payload);
            match op {
                CmpOp::Eq => subj.is_some() && values.iter().any(|v| subj.as_deref() == Some(v)),
                CmpOp::Ne => subj
                    .as_deref()
                    .map_or(true, |s| values.iter().all(|v| v != s)),
                CmpOp::In => subj.is_some() && values.iter().any(|v| subj.as_deref() == Some(v)),
                CmpOp::NotIn => subj
                    .as_deref()
                    .map_or(true, |s| values.iter().all(|v| v != s)),
                _ => false,
            }
        }
    }
}

/// Evaluate a filter expression against a task row, in memory —
/// the pure mirror of filter_sql, used by the tests and by any
/// caller that already holds the row. Semantics match the SQL
/// exactly, including the absent-value rules.
pub fn filter_matches(expr: &FilterExpr, row: &Value) -> bool {
    match expr {
        FilterExpr::Or(a, b) => filter_matches(a, row) || filter_matches(b, row),
        FilterExpr::And(a, b) => filter_matches(a, row) && filter_matches(b, row),
        FilterExpr::Not(a) => !filter_matches(a, row),
        FilterExpr::Cmp { field, op, values } => cmp_matches(*field, *op, values, row),
    }
}

/// The task-side half of the union match, evaluated in memory:
/// Some(reason) when the task's payload `requires` object names
/// an attribute this claimer does not declare with the required
/// value; None when the task declares no requirements or the
/// claimer meets them all. This is the ONLY place claimer
/// identity shapes eligibility — a task's own declared
/// requirements, honored automatically inside the claim.
pub fn task_requirements_unmet(row: &Value, claimer: &ClaimerCtx) -> Option<String> {
    let payload = task_payload(row);
    let req = payload.get("requires")?.as_object()?;
    if let Some(p) = req.get("platform").and_then(|v| v.as_str()) {
        if claimer.family.as_deref() != Some(p) {
            return Some(format!("task requires platform {p}"));
        }
    }
    if let Some(l) = req.get("lane").and_then(|v| v.as_str()) {
        if claimer.lane.as_deref() != Some(l) {
            return Some(format!("task requires lane {l}"));
        }
    }
    if let Some(p) = req.get("pool").and_then(|v| v.as_str()) {
        if claimer.pool.as_deref() != Some(p) {
            return Some(format!("task requires pool {p}"));
        }
    }
    if let Some(m) = req.get("model").and_then(|v| v.as_str()) {
        let met = match &claimer.model {
            Some(cm) => {
                cm.model.as_deref() == Some(m)
                    || cm
                        .model
                        .as_ref()
                        .map(|id| format!("{}/{}", cm.provider, id))
                        .as_deref()
                        == Some(m)
            }
            None => false,
        };
        if !met {
            return Some(format!("task requires model {m}"));
        }
    }
    None
}

/// The recorded platform limitation, expressed for the claim
/// pick (#8217, extending #8210's transfer-claim enforcement to
/// every pick): a row is unservable while its lane-ledger row
/// carries a VALID limitation — platform_limit and
/// platform_limit_reason both recorded (a reason-less stored
/// limitation is invalid: cleared, never enforced, exactly as
/// platform_limit_refusal treats it) — naming a family other
/// than the claim's declared family. A claim declaring no family
/// is served only unlimited rows. The two placeholders are the
/// claim's family, twice; callers append them after the pick
/// predicate's own arguments, in both the pick SELECT and the
/// claim UPDATE re-check. Semantics mirror
/// platform_limit_refusal's truth table (pinned by
/// platform_limit_pick_semantics_8217).
pub(crate) const PLATFORM_LIMIT_PICK_CLAUSE: &str = "AND NOT EXISTS (SELECT 1 FROM \
    repo_task_lane_ledger l WHERE l.task_id = tasks.id \
      AND l.platform_limit IS NOT NULL AND l.platform_limit <> '' \
      AND l.platform_limit_reason IS NOT NULL \
      AND l.platform_limit_reason <> '' \
      AND (? IS NULL OR l.platform_limit <> ?))";

pub(crate) fn claim_update_sql(pick_predicate: &str) -> String {
    format!(
        "UPDATE tasks SET status = 'claimed', worker_id = ?, claimed_at = ?, \
           lease_until = ?, \
           attempts = attempts + (CASE WHEN status IN ('parked', 'handed_off') THEN 0 ELSE 1 END), \
           lane_id = ?, platform = COALESCE(?, platform), \
           park_kind = NULL, wait_kind = NULL, not_before = NULL, \
           required_budget_minutes = NULL, \
           updated_at = ? \
         WHERE id = ? \
           AND ( {pick_predicate} ) {PLATFORM_LIMIT_PICK_CLAUSE}"
    )
}

/// The default (lane) pick predicate (#8184, amended #8187,
/// #8234): queued rows due under the attempts cap, or parked rows
/// whose wait timer has passed — plus, as a third branch, handoff
/// sink rows (status 'handed_off', the #8185 sink with its own
/// status since #8234) with no timer at all: the sink is not a
/// wait. #8184 excluded sink rows from lanes entirely (they were
/// claimant-owned, and serving one to a lane only re-ran the
/// claim -> handoff -> park cycle); #8187 reverses that: a lane is
/// single-session by default, claims a sink row at +0 attempts
/// (per the claim UPDATE's parked/handed_off CASE), and verb_lane
/// flips its ledger assignment to direct when the lane passes
/// --exclusion-reason (see lane_reassignment_flip). The claimant's
/// handoff pick and by-id claim keep working unchanged. Since
/// #8234 a sink row is never 'parked', so the timed parked branch
/// needs no sink exclusion — genuine wait parks release through
/// claim at not_before, and only there. #13091 narrows "genuine
/// wait": a park whose recorded term is a broker-owned condition
/// or a person's answer (PARKED_TERM_EXCLUSION) is not the pick's
/// to release — the revalidation sweeper and /wake own those. The sink branch carries
/// no `?` placeholder, so the pick SELECT and the claim UPDATE
/// re-check keep their four leading arguments: the two `now`s of
/// the queued/parked branches, then the claimer's declared
/// remaining budget twice (#3418: the capacity-wait bypass gate
/// and the outer budget gate).
/// #3418: a row's effective required budget in minutes — the
/// park-stamped column wins; otherwise the packet's own declared
/// `required_budget_minutes`. NULL means no requirement: the row
/// matches every claim exactly as before this feature.
pub(crate) const REQUIRED_BUDGET_EXPR: &str = "COALESCE(required_budget_minutes, \
    CAST(json_extract(payload, '$.required_budget_minutes') AS INTEGER))";

/// #3418: the budget gate. A requirement-bearing row matches only
/// a claim declaring at least that many remaining minutes; the
/// placeholder binds NULL for an undeclared budget and
/// COALESCE(?, -1) makes an undeclared budget fail every
/// requirement (a claimer that cannot state its budget cannot
/// prove the fit) while requirement-free rows pass for all claims.
pub(crate) fn budget_gate_sql() -> String {
    format!("({REQUIRED_BUDGET_EXPR} IS NULL OR COALESCE(?, -1) >= {REQUIRED_BUDGET_EXPR})")
}

/// #3418: the capacity-wait timer bypass. Only a requirement-
/// bearing capacity park may be taken before its timer by a
/// claim whose declared budget fits — a requirement-free
/// capacity park (a quota-reset wait is the standing example)
/// keeps its timer exactly as before.
pub(crate) fn budget_bypass_sql() -> String {
    format!("({REQUIRED_BUDGET_EXPR} IS NOT NULL AND COALESCE(?, -1) >= {REQUIRED_BUDGET_EXPR})")
}

/// #13091: the park terms the default pick may NOT release on its
/// timer. A park whose recorded ready condition is a broker-owned
/// condition (`task_terminal`, `task_status`, `event_seen`) is
/// released by the park-revalidation sweeper when the broker
/// itself observes the condition — never by a lane claiming the
/// row on the clock. A `person_answer` park waits on a person and
/// releases through /wake when the person acts. Serving either
/// kind to a lane on the park's (possibly default) timer hands
/// work to a lane that can only re-park it — the double-serve
/// incidents of 2026-11-23. Every other park (time, ci, review,
/// capacity, triage, external, or a legacy row with no recorded
/// condition) keeps its timer release: for those, claim-at-
/// not_before IS the designed release path (FDR-0008). Literals
/// only — the predicate's placeholder count is pinned.
pub(crate) const PARKED_TERM_EXCLUSION: &str = "COALESCE(json_extract(ready_condition, '$.kind'), 'time') \
    NOT IN ('task_terminal', 'task_status', 'event_seen', 'person_answer')";

pub(crate) fn default_pick_predicate() -> String {
    let gate = budget_gate_sql();
    let bypass = budget_bypass_sql();
    format!(
        "manual_only = 0 AND ((((status = 'queued' AND attempts < max_attempts \
         AND (not_before IS NULL OR not_before <= ?)) \
       OR (status = 'parked' AND ((not_before IS NOT NULL AND not_before <= ?) \
         OR (wait_kind = 'capacity' AND {bypass})) AND {PARKED_TERM_EXCLUSION})) \
       AND {gate}) \
       OR ({HANDOFF_SQL}))"
    )
}

pub(crate) fn handoff_pick_predicate() -> String {
    format!("manual_only = 0 AND ({HANDOFF_SQL})")
}

/// The lane a FRESH ledger assignment gets (#8187): `direct` by
/// default — the claiming session owns the task, and no task is
/// routed to opencode (or any specific lane) without a specific
/// reason recorded at assignment time. The only doors to an
/// opencode assignment are an explicit `--opencode-reason` from
/// the caller (recorded on the ledger row) — or, symmetrically, a
/// caller `--exclusion-reason` forces direct. Exclusion wins when
/// both are passed: it is the older, stronger contract ("this task
/// must NOT go to opencode"). Returns (lane, reason) where reason
/// is the value recorded on the lane_assigned event:
/// "default_direct" | "exclusion" | "opencode_reason".
///
/// tasks.platform plays NO part here, ever (#8187 amendment):
/// a task's platform stamp is a MARK of which platform is looking
/// at it (PLATFORM_FAMILIES: "openchamber" | "muse" — a family,
/// never a specific lane), never a routing reason and never an
/// eligibility condition. The ONLY door to opencode is the
/// explicit --opencode-reason above.
pub(crate) fn lane_decision(
    exclusion: Option<&str>,
    opencode_reason: Option<&str>,
) -> (&'static str, &'static str) {
    if exclusion.is_some() {
        ("direct", "exclusion")
    } else if opencode_reason.is_some() {
        ("opencode", "opencode_reason")
    } else {
        ("direct", "default_direct")
    }
}
