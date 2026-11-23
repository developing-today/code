//! turso_queue::stacks_outbox — merge-intent stacks, signal, creation outbox, dispatch.
//!
//! Split from turso_queue.rs for #8589 (F11.10 production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

pub(crate) const MERGE_INTENT_COLS: &str = "id, stack_id, position, repo, branch, base_branch, \
    head_sha, base_head_sha, issue_number, pr_number, owner_lane, owner_worker, owner_session, \
    state, conflict_detail, created_at, updated_at";

pub(crate) fn merge_intent_core(row: &Value) -> Option<crate::merge_intent::MergeIntent> {
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

pub(crate) fn stack_rows(t: &TursoBackend, stack_id: &str) -> Result<Vec<Value>, TErr> {
    t.query_objs(
        &format!(
            "SELECT {MERGE_INTENT_COLS} FROM merge_intents \
             WHERE stack_id = ? ORDER BY position ASC"
        ),
        &[Arg::Text(stack_id.to_string())],
        &[],
    )
}

/// The stack state the list endpoint answers: every recorded intent
/// in merge order, plus the core's derived view (the replay walk,
/// and where the next append lands) — all from the database alone.
pub(crate) fn stack_state(stack_id: &str, rows: &[Value]) -> Value {
    let core: Vec<crate::merge_intent::MergeIntent> =
        rows.iter().filter_map(merge_intent_core).collect();
    let walk: Vec<Value> = crate::merge_intent::replay_prefix(&core, stack_id)
        .iter()
        .map(|i| {
            json!({
                "position": i.position,
                "branch": i.branch,
                "base_branch": i.base_branch,
            })
        })
        .collect();
    json!({
        "stack_id": stack_id,
        "intents": rows,
        "replay_prefix": walk,
        "next_position": crate::merge_intent::next_position(&core, stack_id),
        "next_base_branch": crate::merge_intent::next_base_branch(&core, stack_id),
    })
}

pub(crate) fn verb_stack_record(t: &TursoBackend, req: &Value) -> Value {
    let append = crate::merge_intent::AppendRequest {
        stack_id: req_str(req, "stack_id").unwrap_or("").to_string(),
        repo: req_str(req, "repo").unwrap_or("").to_string(),
        branch: req_str(req, "branch").unwrap_or("").to_string(),
        base_branch: req_str(req, "base_branch").map(|s| s.to_string()),
        head_sha: req_str(req, "head_sha").map(|s| s.to_string()),
        issue_number: req_i64(req, "issue_number"),
        pr_number: req_i64(req, "pr_number"),
        owner_lane: req_str(req, "owner_lane").unwrap_or("").to_string(),
        owner_worker: req_str(req, "owner_worker").unwrap_or("").to_string(),
        owner_session: req_str(req, "owner_session").map(|s| s.to_string()),
    };
    if let Err(e) = crate::merge_intent::validate_append(&append) {
        return fail(2, e);
    }
    // Owner lane/worker pair validation against the lane registry
    // (#8582 increment 3; deferred from the CodeRabbit round on
    // PR #8660, where the pair originates at the client verbs):
    // a recorded intent's conflict is routed to its owner pair, so
    // the pair must be one the registry actually holds — an
    // unknown lane, a lane owned by a different worker, or a
    // disabled lane would strand that routing. Mirrors the claim
    // preamble's registry check (minus the queue binding: stack
    // recording is not a claim).
    {
        let lanes = match t.query_objs(
            "SELECT lane_id, worker_id, enabled FROM lanes WHERE lane_id = ?",
            &[Arg::Text(append.owner_lane.clone())],
            &[],
        ) {
            Ok(rows) => rows,
            Err(e) => return terr(e),
        };
        let Some(l) = lanes.into_iter().next() else {
            return fail(
                2,
                format!("stack_record: unknown owner lane_id {}", append.owner_lane),
            );
        };
        if l.get("worker_id").and_then(|v| v.as_str()) != Some(append.owner_worker.as_str()) {
            return fail(
                2,
                format!(
                    "stack_record: lane {} belongs to worker {}, not {}",
                    append.owner_lane,
                    l.get("worker_id").and_then(|v| v.as_str()).unwrap_or(""),
                    append.owner_worker
                ),
            );
        }
        if l.get("enabled").and_then(|v| v.as_i64()) == Some(0) {
            return fail(
                2,
                format!("stack_record: lane {} is disabled", append.owner_lane),
            );
        }
    }
    // One stack, one repository (#8582): refuse a cross-repo append
    // before planning — plan_append would otherwise base the new
    // branch on a tip from the other repository (CodeRabbit finding,
    // PR #8660). A read failure here is not a verdict: the work
    // closure below re-reads under the retry loop and surfaces it.
    if let Ok(pre_rows) = stack_rows(t, &append.stack_id) {
        let pre_existing: Vec<crate::merge_intent::MergeIntent> =
            pre_rows.iter().filter_map(merge_intent_core).collect();
        if let Err(e) = crate::merge_intent::validate_append_against_stack(&pre_existing, &append) {
            return fail(2, e);
        }
    }
    let work = (|| -> Result<Value, TErr> {
        // Concurrent lanes recording into the same stack can read the
        // same rows and plan the same next position; the loser's INSERT
        // then hits UNIQUE(stack_id, position). Retry the whole
        // read/plan/insert cycle on that conflict so position AND base
        // branch are recalculated from the winner's row (Codex P2 on
        // PR #8660). Other errors — including the UNIQUE(repo, branch)
        // duplicate-branch refusal — return immediately.
        let mut attempt = 0;
        loop {
            attempt += 1;
            let existing_rows = stack_rows(t, &append.stack_id)?;
            let existing: Vec<crate::merge_intent::MergeIntent> =
                existing_rows.iter().filter_map(merge_intent_core).collect();
            // Safety net for a cross-repo row landing between the
            // pre-check below and this read: never plan on its tip.
            crate::merge_intent::validate_append_against_stack(&existing, &append)?;
            let planned = crate::merge_intent::plan_append(&existing, &append);
            let now = now_iso();
            let res = t.exec(
                "INSERT INTO merge_intents (stack_id, position, repo, branch, base_branch, \
                 head_sha, base_head_sha, issue_number, pr_number, owner_lane, owner_worker, owner_session, \
                 state, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    Arg::Text(planned.stack_id.clone()),
                    Arg::Int(planned.position),
                    Arg::Text(planned.repo.clone()),
                    Arg::Text(planned.branch.clone()),
                    Arg::Text(planned.base_branch.clone()),
                    arg_str(planned.head_sha.as_deref()),
                    arg_str(planned.base_head_sha.as_deref()),
                    planned.issue_number.map(Arg::Int).unwrap_or(Arg::Null),
                    planned.pr_number.map(Arg::Int).unwrap_or(Arg::Null),
                    Arg::Text(planned.owner_lane.clone()),
                    Arg::Text(planned.owner_worker.clone()),
                    arg_str(planned.owner_session.as_deref()),
                    Arg::Text(planned.state.as_str().to_string()),
                    Arg::Text(now.clone()),
                    Arg::Text(now),
                ],
            );
            let res = match res {
                Ok(res) => res,
                Err(e)
                    if attempt < 5
                        && e.contains("UNIQUE constraint failed")
                        && e.contains("stack_id") =>
                {
                    continue;
                }
                Err(e) => return Err(e),
            };
            let rows = stack_rows(t, &append.stack_id)?;
            let inserted = rows
                .iter()
                .find(|r| r.get("id").and_then(|v| v.as_i64()) == res.last_insert_rowid)
                .cloned()
                .unwrap_or(Value::Null);
            return Ok(json!({
                "intent": inserted,
                "stack": stack_state(&append.stack_id, &rows),
            }));
        }
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_stack_list(t: &TursoBackend, req: &Value) -> Value {
    let Some(stack_id) = req_str(req, "stack_id") else {
        return fail(2, "stack_id is required".to_string());
    };
    let stack_id = stack_id.to_string();
    let work = (|| -> Result<Value, TErr> {
        let rows = stack_rows(t, &stack_id)?;
        Ok(stack_state(&stack_id, &rows))
    })();
    match work {
        Ok(v) => ok(json!({"row": v})),
        Err(e) => terr(e),
    }
}

/// Move one recorded intent through its lifecycle (#8582
/// increment 4): the storage half of the replay executor. The
/// transition rules are the pure core's (`IntentState::
/// can_transition`) — this verb only reads the current row,
/// refuses an illegal move, and writes the new state (plus the PR
/// number a landing records, and the conflict detail a conflict
/// routing carries). It performs no GitHub work: the client
/// executor drives git/gh and reports each state change here, so
/// the stack state stays fully visible from the database alone
/// even mid-replay.
pub(crate) fn verb_stack_transition(t: &TursoBackend, req: &Value) -> Value {
    let Some(stack_id) = req_str(req, "stack_id") else {
        return fail(2, "stack_id is required".to_string());
    };
    let stack_id = stack_id.to_string();
    let Some(position) = req_i64(req, "position") else {
        return fail(2, "position is required".to_string());
    };
    let Some(to) = req_str(req, "to_state").and_then(crate::merge_intent::IntentState::parse)
    else {
        return fail(
            2,
            format!(
                "stack_transition: unknown to_state {:?}",
                req_str(req, "to_state")
            ),
        );
    };
    let rows = match stack_rows(t, &stack_id) {
        Ok(r) => r,
        Err(e) => return terr(e),
    };
    let Some(row) = rows
        .iter()
        .find(|r| r.get("position").and_then(|v| v.as_i64()) == Some(position))
    else {
        return fail(
            2,
            format!("stack_transition: no intent at stack {stack_id} position {position}"),
        );
    };
    let Some(current) = merge_intent_core(row) else {
        return fail(
            2,
            "stack_transition: stored intent row is unparseable".to_string(),
        );
    };
    if !current.state.can_transition(to) {
        return fail(
            2,
            format!(
                "stack_transition: illegal move {} -> {} for stack {} position {}",
                current.state.as_str(),
                to.as_str(),
                stack_id,
                position
            ),
        );
    }
    // A landing records the PR that landed it; an explicit number
    // wins, otherwise the recorded one (if any) is kept.
    let pr_number = req_i64(req, "pr_number").or(current.pr_number);
    // The conflict detail lives only while the intent is
    // conflicted: a fresh detail replaces it, a move out of
    // conflicted clears it.
    let conflict_detail: Option<String> = if to == crate::merge_intent::IntentState::Conflicted {
        req_str(req, "conflict_detail")
            .map(|s| s.to_string())
            .or_else(|| {
                row.get("conflict_detail")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    } else {
        None
    };
    let now = now_iso();
    // Condition the write on the state that was validated above
    // (Codex P2, PR #8849): the broker serves connections
    // concurrently, so a second transition can land between this
    // verb's read and its UPDATE — an unconditional write would let
    // it overwrite a terminal state the core has already sealed.
    // With the validated state in the predicate, a lost race
    // affects 0 rows and is refused as a concurrent change.
    let upd = match t.exec(
        "UPDATE merge_intents SET state = ?, pr_number = ?, conflict_detail = ?, \
         updated_at = ? WHERE stack_id = ? AND position = ? AND state = ?",
        &[
            Arg::Text(to.as_str().to_string()),
            pr_number.map(Arg::Int).unwrap_or(Arg::Null),
            arg_str(conflict_detail.as_deref()),
            Arg::Text(now),
            Arg::Text(stack_id.clone()),
            Arg::Int(position),
            Arg::Text(current.state.as_str().to_string()),
        ],
    ) {
        Ok(r) => r,
        Err(e) => return terr(e),
    };
    if upd.affected == 0 {
        return fail(
            2,
            format!(
                "stack_transition: intent at stack {stack_id} position {position} changed \
                 concurrently (validated state {} is no longer current); re-read the stack \
                 and retry",
                current.state.as_str()
            ),
        );
    }
    let rows = match stack_rows(t, &stack_id) {
        Ok(r) => r,
        Err(e) => return terr(e),
    };
    let updated = rows
        .iter()
        .find(|r| r.get("position").and_then(|v| v.as_i64()) == Some(position))
        .cloned()
        .unwrap_or(Value::Null);
    ok(json!({"row": {"intent": updated, "stack": stack_state(&stack_id, &rows)}}))
}

/// Re-record one intent's base after its author restacked the
/// branch off a withdrawn base (#8582; Codex P2, PR #8849): the
/// storage half of the `stack restack` acknowledgement. The
/// refresh rules are the pure core's (`plan_restack`) — this verb
/// only reads the stack, refuses what the core refuses, and
/// writes the new base (plus the restacked head, when the author
/// recorded one), conditioned on the base that was validated so a
/// concurrent change is refused rather than overwritten (the
/// transition verb's CAS discipline).
pub(crate) fn verb_stack_restack(t: &TursoBackend, req: &Value) -> Value {
    let Some(stack_id) = req_str(req, "stack_id") else {
        return fail(2, "stack_id is required".to_string());
    };
    let stack_id = stack_id.to_string();
    let Some(position) = req_i64(req, "position") else {
        return fail(2, "position is required".to_string());
    };
    let Some(new_base) = req_str(req, "new_base_branch") else {
        return fail(2, "new_base_branch is required".to_string());
    };
    let rows = match stack_rows(t, &stack_id) {
        Ok(r) => r,
        Err(e) => return terr(e),
    };
    let core: Vec<crate::merge_intent::MergeIntent> =
        rows.iter().filter_map(merge_intent_core).collect();
    let restack = crate::merge_intent::RestackRequest {
        stack_id: stack_id.clone(),
        position,
        new_base_branch: new_base.to_string(),
        new_head_sha: req_str(req, "head_sha").map(|s| s.to_string()),
    };
    let planned = match crate::merge_intent::plan_restack(&core, &restack) {
        Ok(pl) => pl,
        Err(e) => return fail(2, e),
    };
    // plan_restack found the intent; its validated base AND its
    // validated lifecycle state are the CAS predicates for the
    // write below. The state predicate matters (Codex P2, PR
    // #8849, comment 4549335606): transitions do not change
    // base_branch, so a base-only predicate would let a restack
    // acknowledgement rewrite the recorded base/head of an intent
    // that a concurrent transition just landed or abandoned --
    // terminal records are sealed.
    let (current_base, current_state) = core
        .iter()
        .find(|i| i.position == position)
        .map(|i| (i.base_branch.clone(), i.state.as_str().to_string()))
        .unwrap_or_default();
    let now = now_iso();
    let upd = match t.exec(
        "UPDATE merge_intents SET base_branch = ?, head_sha = ?, base_head_sha = ?, updated_at = ? \
         WHERE stack_id = ? AND position = ? AND base_branch = ? AND state = ?",
        &[
            Arg::Text(planned.base_branch.clone()),
            arg_str(planned.head_sha.as_deref()),
            arg_str(planned.base_head_sha.as_deref()),
            Arg::Text(now),
            Arg::Text(stack_id.clone()),
            Arg::Int(position),
            Arg::Text(current_base),
            Arg::Text(current_state),
        ],
    ) {
        Ok(r) => r,
        Err(e) => return terr(e),
    };
    if upd.affected == 0 {
        return fail(
            2,
            format!(
                "stack_restack: intent at stack {stack_id} position {position} changed \
                 concurrently (validated base or lifecycle state is no longer current); \
                 re-read the stack and retry"
            ),
        );
    }
    let rows = match stack_rows(t, &stack_id) {
        Ok(r) => r,
        Err(e) => return terr(e),
    };
    let updated = rows
        .iter()
        .find(|r| r.get("position").and_then(|v| v.as_i64()) == Some(position))
        .cloned()
        .unwrap_or(Value::Null);
    ok(json!({"row": {"intent": updated, "stack": stack_state(&stack_id, &rows)}}))
}

pub(crate) fn verb_signal_count(t: &TursoBackend, req: &Value) -> Value {
    let queue = req_str(req, "queue").unwrap_or("").to_string();
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(
            "SELECT COUNT(*) AS n, MIN(created_at) AS oldest FROM tasks \
             WHERE queue = ? AND status = 'queued'",
            &[Arg::Text(queue)],
            &[],
        )?;
        let first = rows.into_iter().next().unwrap_or(json!({}));
        Ok(json!({
            "queued_count": first.get("n").cloned().unwrap_or(json!(0)),
            "oldest_queued_at": first.get("oldest").cloned().unwrap_or(Value::Null),
        }))
    })();
    match work {
        Ok(v) => {
            let mut m = json!({"ok": true});
            if let (Some(dst), Some(src)) = (m.as_object_mut(), v.as_object()) {
                for (k, val) in src {
                    dst.insert(k.clone(), val.clone());
                }
            }
            m
        }
        Err(e) => terr(e),
    }
}

// ---------------------------------------------------------------------------
// Creation outbox (#3706) — the bounded pilot's Turso verbs.
//
// Storage half of `crate::creation_outbox`: the enqueue API, the
// serial drainer's claim/resolve verbs, and metrics, over the
// `creation_outbox` table (schema in SCHEMA above) plus the
// persisted circuit-breaker row (`creation_outbox_circuit`, id=1),
// so every drainer sees the same circuit. The state-machine rules
// (dedupe/conflict, backoff, pacing, read-back-before-done,
// uncertain reconciliation) are the core module's; these verbs are
// their SQL spelling. The drainer's GitHub leg is the client's
// `fleet queue outbox drain` (client::outbox); these verbs never
// shell out.
// ---------------------------------------------------------------------------

pub(crate) const OUTBOX_COLS: &str = "id, repo, kind, payload, dedup_key, payload_hash, \
    producer, state, created_at, available_at, attempt_count, max_attempts, \
    last_error_class, lease_owner, lease_until, github_number, github_url, completed_at";

pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = (y_adj - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Parse the `iso()` format back to unix seconds (the outbox core
/// works in unix time; rows store ISO strings like every other
/// table here, where lexicographic order is chronological order).
pub(crate) fn iso_to_unix(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let time = time.trim_end_matches('Z');
    let mut tp = time.split(':');
    let hh: i64 = tp.next()?.parse().ok()?;
    let mm: i64 = tp.next()?.parse().ok()?;
    let ss: i64 = tp.next()?.parse::<f64>().ok()? as i64;
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss)
}

pub(crate) fn outbox_jitter_unit() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    ((nanos ^ (u64::from(std::process::id()) << 32)) % 10_000) as f64 / 10_000.0
}

pub(crate) fn outbox_get(t: &TursoBackend, id: &str) -> Result<Option<Value>, TErr> {
    let rows = t.query_objs(
        &format!("SELECT {OUTBOX_COLS} FROM creation_outbox WHERE id = ?"),
        &[Arg::Text(id.to_string())],
        &["payload"],
    )?;
    Ok(rows.into_iter().next())
}

pub(crate) fn outbox_circuit(t: &TursoBackend) -> Result<crate::creation_outbox::Circuit, TErr> {
    let rows = t.query_objs(
        "SELECT consecutive_rate_limits, open_until, last_error_class \
         FROM creation_outbox_circuit WHERE id = 1",
        &[],
        &[],
    )?;
    let mut circuit = crate::creation_outbox::Circuit::default();
    if let Some(row) = rows.into_iter().next() {
        circuit.consecutive_rate_limits = row
            .get("consecutive_rate_limits")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as u32;
        circuit.open_until = row
            .get("open_until")
            .and_then(|v| v.as_str())
            .and_then(iso_to_unix)
            .unwrap_or(0);
        circuit.last_error_class = row
            .get("last_error_class")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    Ok(circuit)
}

pub(crate) fn outbox_circuit_save(
    t: &TursoBackend,
    c: &crate::creation_outbox::Circuit,
) -> Result<(), TErr> {
    t.exec(
        "INSERT INTO creation_outbox_circuit (id, consecutive_rate_limits, open_until, last_error_class, updated_at) \
         VALUES (1, ?, ?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET consecutive_rate_limits = excluded.consecutive_rate_limits, \
           open_until = excluded.open_until, last_error_class = excluded.last_error_class, \
           updated_at = excluded.updated_at",
        &[
            Arg::Int(i64::from(c.consecutive_rate_limits)),
            if c.open_until > 0 { Arg::Text(iso(c.open_until)) } else { Arg::Null },
            arg_str(c.last_error_class.as_deref()),
            Arg::Text(now_iso()),
        ],
    )?;
    Ok(())
}

/// Enqueue one creation intent. Insert + dedupe are one read-then-
/// insert against the UNIQUE(repo, dedup_key) index (the same shape
/// as task enqueue): same key + same payload hash returns the
/// original row (`replay`), same key + different hash is a conflict
/// (exit 5), never a silent overwrite.
pub(crate) fn verb_outbox_enqueue(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let repo = req_str(req, "repo").unwrap_or("");
        let kind = req_str(req, "kind").unwrap_or("");
        let dedup_key = req_str(req, "dedup_key").unwrap_or("");
        let payload = req.get("payload").cloned().unwrap_or(json!({}));
        if repo.is_empty() || dedup_key.is_empty() {
            return Ok(fail(2, "outbox enqueue needs repo and dedup_key"));
        }
        if !crate::creation_outbox::KINDS.contains(&kind) {
            return Ok(fail(
                2,
                format!("kind {kind:?} is not an outbox creation kind"),
            ));
        }
        let hash = crate::creation_outbox::payload_hash(&payload);
        let existing = t.query_objs(
            &format!("SELECT {OUTBOX_COLS} FROM creation_outbox WHERE repo = ? AND dedup_key = ?"),
            &[
                Arg::Text(repo.to_string()),
                Arg::Text(dedup_key.to_string()),
            ],
            &["payload"],
        )?;
        if let Some(row) = existing.into_iter().next() {
            if row.get("payload_hash").and_then(|v| v.as_str()) == Some(hash.as_str()) {
                return Ok(ok(json!({"row": row, "outcome": "replay"})));
            }
            return Ok(fail(
                5,
                format!(
                    "outbox dedup conflict: ({repo}, {dedup_key}) already holds entry {} with a different payload",
                    row.get("id").and_then(|v| v.as_str()).unwrap_or("?")
                ),
            ));
        }
        let now = now_unix();
        let id = format!(
            "ob-{}-{}",
            now,
            &crate::creation_outbox::sha256_hex(format!("{repo}\n{dedup_key}\n{hash}").as_bytes())
                [..12]
        );
        t.exec(
            "INSERT INTO creation_outbox (id, repo, kind, payload, dedup_key, payload_hash, \
             producer, state, created_at, available_at, attempt_count, max_attempts) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, 0, ?)",
            &[
                Arg::Text(id.clone()),
                Arg::Text(repo.to_string()),
                Arg::Text(kind.to_string()),
                Arg::Text(payload.to_string()),
                Arg::Text(dedup_key.to_string()),
                Arg::Text(hash),
                arg_str(req_str(req, "producer")),
                Arg::Text(iso(now)),
                Arg::Text(iso(now)),
                Arg::Int(crate::creation_outbox::DEFAULT_MAX_ATTEMPTS),
            ],
        )?;
        let row = outbox_get(t, &id)?.unwrap_or(json!({}));
        Ok(ok(json!({"row": row, "outcome": "created"})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Claim the oldest due entry for a drainer: pending-and-available,
/// an expired lease (crashed drainer), or an uncertain entry whose
/// reconciliation delay has elapsed (claimed AS uncertain — the
/// drainer must reconcile by marker before any re-create). Refuses
/// while the circuit is open or the rolling-hour creation budget
/// is spent; pacing (minimum gap between creates) is the drainer
/// client's side of the same policy.
pub(crate) fn verb_outbox_claim(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let owner = req_str(req, "owner").unwrap_or("outbox-drainer");
        let now = now_unix();
        let circuit = outbox_circuit(t)?;
        if circuit.is_open(now) {
            return Ok(ok(json!({
                "row": Value::Null,
                "circuit_open_until": iso(circuit.open_until),
            })));
        }
        let budget_rows = t.query_objs(
            "SELECT COUNT(*) AS n FROM creation_outbox \
             WHERE completed_at IS NOT NULL AND completed_at > ?",
            &[Arg::Text(iso(now - 3600))],
            &[],
        )?;
        let recent = budget_rows
            .into_iter()
            .next()
            .and_then(|r| r.get("n").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        if recent >= crate::creation_outbox::HOURLY_BUDGET as i64 {
            return Ok(ok(json!({"row": Value::Null, "budget_exhausted": true})));
        }
        let rows = t.query_objs(
            &format!(
                "SELECT {OUTBOX_COLS} FROM creation_outbox \
                 WHERE (state = 'pending' AND available_at <= ?) \
                    OR (state = 'leased' AND lease_until IS NOT NULL AND lease_until <= ?) \
                    OR (state = 'uncertain' AND available_at <= ?) \
                 ORDER BY created_at ASC LIMIT 1"
            ),
            &[
                Arg::Text(iso(now)),
                Arg::Text(iso(now)),
                Arg::Text(iso(now)),
            ],
            &["payload"],
        )?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(ok(json!({"row": Value::Null})));
        };
        let id = row
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let state = row
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if state == "leased" {
            // An expired lease: the previous drainer died
            // mid-flight and its create may have landed. The row
            // is claimed AS uncertain — the drainer reconciles by
            // marker before any re-create (kill/restart trial).
            t.exec(
                "UPDATE creation_outbox SET state = 'uncertain', available_at = ?, \
                 lease_owner = ?, lease_until = ? WHERE id = ?",
                &[
                    Arg::Text(iso(now)),
                    Arg::Text(owner.to_string()),
                    Arg::Text(iso(now + crate::creation_outbox::LEASE_SECONDS)),
                    Arg::Text(id.clone()),
                ],
            )?;
        } else if state != "uncertain" {
            t.exec(
                "UPDATE creation_outbox SET state = 'leased', lease_owner = ?, lease_until = ? \
                 WHERE id = ?",
                &[
                    Arg::Text(owner.to_string()),
                    Arg::Text(iso(now + crate::creation_outbox::LEASE_SECONDS)),
                    Arg::Text(id.clone()),
                ],
            )?;
        } else {
            t.exec(
                "UPDATE creation_outbox SET lease_owner = ?, lease_until = ? WHERE id = ?",
                &[
                    Arg::Text(owner.to_string()),
                    Arg::Text(iso(now + crate::creation_outbox::LEASE_SECONDS)),
                    Arg::Text(id.clone()),
                ],
            )?;
        }
        let claimed = outbox_get(t, &id)?.unwrap_or(json!({}));
        Ok(ok(json!({"row": claimed})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Resolve a claimed entry. One verb, four outcomes — the SQL
/// spelling of the core drainer's transitions:
/// - `done` {github_number, github_url}: only with a verified
///   number/URL (the drainer read the object back first); a done
///   without both is refused (exit 2) — a gh exit 0 never marks done.
/// - `rate_limited` {retry_after?}: backoff per the core policy and
///   a strike on the persisted circuit.
/// - `uncertain` {error}: the silent-loss shape; the entry waits
///   for marker reconciliation, never a blind retry.
/// - `failed` {error}: backoff-retry until max_attempts, then blocked.
pub(crate) fn verb_outbox_resolve(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let id = req_str(req, "id").unwrap_or("");
        let outcome = req_str(req, "outcome").unwrap_or("");
        let Some(current) = outbox_get(t, id)? else {
            return Ok(fail(5, format!("outbox entry {id} not found")));
        };
        let now = now_unix();
        let attempts = current
            .get("attempt_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let max_attempts = current
            .get("max_attempts")
            .and_then(|v| v.as_i64())
            .unwrap_or(crate::creation_outbox::DEFAULT_MAX_ATTEMPTS);
        match outcome {
            "done" => {
                let number = req_i64(req, "github_number");
                let url = req_str(req, "github_url");
                let (Some(number), Some(url)) = (number, url) else {
                    return Ok(fail(
                        2,
                        "outbox done requires the read-back-verified github_number and github_url",
                    ));
                };
                t.exec(
                    "UPDATE creation_outbox SET state = 'done', attempt_count = attempt_count + 1, github_number = ?, github_url = ?, \
                     completed_at = ?, last_error_class = NULL, lease_owner = NULL, lease_until = NULL \
                     WHERE id = ?",
                    &[
                        Arg::Int(number),
                        Arg::Text(url.to_string()),
                        Arg::Text(iso(now)),
                        Arg::Text(id.to_string()),
                    ],
                )?;
                let mut circuit = outbox_circuit(t)?;
                circuit.record_success();
                outbox_circuit_save(t, &circuit)?;
            }
            "rate_limited" => {
                let mut circuit = outbox_circuit(t)?;
                circuit.record_rate_limit(now);
                outbox_circuit_save(t, &circuit)?;
                let delay = crate::creation_outbox::backoff_delay(
                    attempts + 1,
                    req_i64(req, "retry_after"),
                    outbox_jitter_unit(),
                );
                t.exec(
                    "UPDATE creation_outbox SET state = 'pending', attempt_count = attempt_count + 1, \
                     available_at = ?, last_error_class = 'rate_limit', \
                     lease_owner = NULL, lease_until = NULL WHERE id = ?",
                    &[Arg::Text(iso(now + delay)), Arg::Text(id.to_string())],
                )?;
            }
            "uncertain" => {
                let why = req_str(req, "error").unwrap_or("ambiguous create");
                t.exec(
                    "UPDATE creation_outbox SET state = 'uncertain', attempt_count = attempt_count + 1, \
                     available_at = ?, last_error_class = ?, lease_owner = NULL, lease_until = NULL \
                     WHERE id = ?",
                    &[
                        Arg::Text(iso(now + crate::creation_outbox::BACKOFF_BASE_SECONDS)),
                        Arg::Text(format!("ambiguous: {why}")),
                        Arg::Text(id.to_string()),
                    ],
                )?;
            }
            "failed" => {
                let why = req_str(req, "error").unwrap_or("create failed");
                if attempts + 1 >= max_attempts {
                    t.exec(
                        "UPDATE creation_outbox SET state = 'blocked', attempt_count = attempt_count + 1, \
                         last_error_class = ?, lease_owner = NULL, lease_until = NULL WHERE id = ?",
                        &[
                            Arg::Text(format!("failed: {why}")),
                            Arg::Text(id.to_string()),
                        ],
                    )?;
                } else {
                    let delay = crate::creation_outbox::backoff_delay(
                        attempts + 1,
                        None,
                        outbox_jitter_unit(),
                    );
                    t.exec(
                        "UPDATE creation_outbox SET state = 'pending', attempt_count = attempt_count + 1, \
                         available_at = ?, last_error_class = ?, lease_owner = NULL, lease_until = NULL \
                         WHERE id = ?",
                        &[
                            Arg::Text(iso(now + delay)),
                            Arg::Text(format!("failed: {why}")),
                            Arg::Text(id.to_string()),
                        ],
                    )?;
                }
            }
            other => {
                return Ok(fail(2, format!("unknown outbox outcome {other:?}")));
            }
        }
        let row = outbox_get(t, id)?.unwrap_or(json!({}));
        Ok(ok(json!({"row": row})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_outbox_list(t: &TursoBackend, req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let mut sql = format!("SELECT {OUTBOX_COLS} FROM creation_outbox WHERE 1 = 1");
        let mut args: Vec<Arg> = Vec::new();
        if let Some(repo) = req_str(req, "repo") {
            sql += " AND repo = ?";
            args.push(Arg::Text(repo.to_string()));
        }
        if let Some(state) = req_str(req, "state") {
            sql += " AND state = ?";
            args.push(Arg::Text(state.to_string()));
        }
        sql += " ORDER BY created_at ASC LIMIT 200";
        let rows = t.query_objs(&sql, &args, &["payload"])?;
        Ok(ok(json!({"rows": rows})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}

/// Metrics: per-state counts, oldest pending age, total attempts,
/// last failure class, and the circuit — the alert fields the
/// #3706 design requires (pending depth / oldest-pending-age).
pub(crate) fn verb_outbox_metrics(t: &TursoBackend, _req: &Value) -> Value {
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(
            &format!("SELECT {OUTBOX_COLS} FROM creation_outbox"),
            &[],
            &["payload"],
        )?;
        let now = now_unix();
        let mut entries = Vec::new();
        for row in &rows {
            let parse_time = |key: &str| {
                row.get(key)
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_unix)
                    .unwrap_or(0)
            };
            entries.push(crate::creation_outbox::Entry {
                id: row
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                repo: row
                    .get("repo")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: row
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                payload: row.get("payload").cloned().unwrap_or(json!({})),
                dedup_key: row
                    .get("dedup_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                payload_hash: row
                    .get("payload_hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                producer: row
                    .get("producer")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                state: row
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                created_at: parse_time("created_at"),
                available_at: parse_time("available_at"),
                attempt_count: row
                    .get("attempt_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                max_attempts: row
                    .get("max_attempts")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                last_error_class: row
                    .get("last_error_class")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                lease_owner: row
                    .get("lease_owner")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                lease_until: row
                    .get("lease_until")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_unix),
                github_number: row.get("github_number").and_then(|v| v.as_i64()),
                github_url: row
                    .get("github_url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                completed_at: row
                    .get("completed_at")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_unix),
            });
        }
        let circuit = outbox_circuit(t)?;
        let m = crate::creation_outbox::metrics_of(&entries, &circuit, now);
        Ok(ok(json!({"row": {
            "pending": m.pending,
            "leased": m.leased,
            "uncertain": m.uncertain,
            "done": m.done,
            "blocked": m.blocked,
            "oldest_pending_age_seconds": m.oldest_pending_age_seconds,
            "total_attempts": m.total_attempts,
            "last_error_class": m.last_error_class,
            "circuit_open_until": m.circuit_open_until.map(iso),
        }})))
    })();
    match work {
        Ok(v) => v,
        Err(e) => terr(e),
    }
}
/// The Turso backend's half of the broker dispatch: every verb
/// except /validate (purely local) and /healthz (transport-level),
/// which stay in queue_broker::dispatch.
pub fn dispatch(t: &TursoBackend, path: &str, req: &Value) -> Value {
    match path {
        "/enqueue" => verb_enqueue(t, req),
        "/revive" => verb_revive(t, req),
        "/claim" => verb_claim(t, req),
        "/claim-flow" => verb_claim_flow(t, req),
        "/lane" => verb_lane(t, req),
        "/bind" => verb_bind(t, req),
        "/heartbeat" => verb_heartbeat(t, req),
        "/checkpoint" => verb_checkpoint(t, req),
        "/park" => verb_park(t, req),
        "/release" => verb_release(t, req),
        "/wake" => verb_wake(t, req),
        "/block" => verb_block(t, req),
        "/unblock" => verb_unblock(t, req),
        "/dep-recheck" => verb_dep_recheck(t, req),
        "/quarantine-recheck" => verb_quarantine_recheck(t, req),
        "/cancel" => verb_cancel(t, req),
        "/fail" => verb_fail(t, req),
        "/complete" => verb_complete(t, req),
        "/list" => verb_list(t, req),
        "/health" => verb_health(t, req),
        "/signal-count" => verb_signal_count(t, req),
        "/stack-record" => verb_stack_record(t, req),
        "/stack-list" => verb_stack_list(t, req),
        "/stack-transition" => verb_stack_transition(t, req),
        "/stack-restack" => verb_stack_restack(t, req),
        "/events-append" => verb_events_append(t, req),
        "/placement-put" => verb_placement_put(t, req),
        "/placement-get" => verb_placement_get(t, req),
        "/placement-list" => verb_placement_list(t, req),
        "/placement-sync" => verb_placement_sync(t, req),
        "/events-seen" => verb_events_seen(t, req),
        "/events-count" => verb_events_count(t, req),
        "/events-list" => verb_events_list(t, req),
        "/events-prune" => verb_events_prune(t, req),
        "/outbox-enqueue" => verb_outbox_enqueue(t, req),
        "/outbox-claim" => verb_outbox_claim(t, req),
        "/outbox-resolve" => verb_outbox_resolve(t, req),
        "/outbox-list" => verb_outbox_list(t, req),
        "/outbox-metrics" => verb_outbox_metrics(t, req),
        "/sweep" => verb_sweep(t, req),
        "/coverage" => verb_coverage(t, req),
        _ => fail(2, format!("unknown command {path}")),
    }
}
