//! turso_queue::repair — the `fleet queue repair` verb (#13191):
//! in-place repair of quarantined / malformed task packets.
//!
//! The claim path quarantines a stored packet that fails
//! validation (#5444): the row sits blocked with the reason in
//! last_error, and every later claim re-quarantines it.
//! quarantine-recheck can only release a row whose error no longer
//! reproduces — it cannot fix a packet that is still malformed.
//! Task 18533 (#7709) proved the gap: its `payload.dedupe` lacked
//! `checked_at`/`basis`, no verb could repair the payload in
//! place, and the only disposition was cancel-with-disposition.
//!
//! Repair is deliberately narrow. It runs the packet validators
//! (the same `validate_packet` oracle the claim guard runs) and
//! fixes only defect classes whose correct value can be
//! re-derived by performing the check the field attests to:
//!
//!   * dedupe stamp — `payload.dedupe` missing `checked_at` /
//!     `basis`. Repaired by re-performing the dedupe check live:
//!     no other live task may hold the same work identity
//!     (#10627) or the same dedupe key. When the check passes,
//!     the stamp records that the repair verb performed it, at
//!     repair time. A live duplicate refuses the repair —
//!     stamping anyway would be a false attestation.
//!
//! Anything else (a missing issue reference, a malformed prompt
//! body, a payload that is absent) is unrepairable: the verb
//! fails, lists the defects, and mutates nothing. Repairs are
//! computed and re-validated in memory first; the write happens
//! only when the repaired packet passes validation clean, and
//! lands as one status-guarded UPDATE plus the `repaired`
//! timeline event naming the verb, the operator, the defects
//! found, and the repairs applied.
//!
//! State rules: repair loads a task in a non-terminal state
//! (blocked, queued, parked, handed_off). A claimed task is
//! never mutated under its worker. Cancelled and failed tasks
//! stay `revive`'s job — repair composes with revive, it does
//! not replace it. On success a blocked task returns to queued,
//! or stays blocked with its recorded deps intact when it
//! carries a dep record; the quarantine error is discarded.

use super::*;

/// The basis a repair stamp records: the repair verb performed
/// the dedupe check itself, at repair time (#13191).
pub(crate) const REPAIR_DEDUPE_BASIS: &str = "fleet queue repair: dedupe re-checked live at repair time; no other live task holds this work identity or dedupe key";

/// Another live, valid task holding this row's dedupe key in the
/// same queue, if any. The key half of the dedupe re-check: a
/// quarantined (invalid) row never counts as a holder — it
/// carries no servable work, the same rule the enqueue identity
/// check applies (#10627) — and the candidate never counts
/// itself.
fn live_dedupe_key_holder(
    t: &TursoBackend,
    row: &Value,
    task_id: i64,
) -> Result<Option<Value>, TErr> {
    let key = row.get("dedupe_key").and_then(|v| v.as_str()).unwrap_or("");
    if key.is_empty() {
        return Ok(None);
    }
    let queue = row.get("queue").and_then(|v| v.as_str()).unwrap_or("");
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks \
             WHERE queue = ? AND dedupe_key = ? AND id != ? \
               AND status IN ('queued', 'claimed', 'parked', 'handed_off', 'blocked') \
             ORDER BY id ASC"
        ),
        &[
            Arg::Text(queue.to_string()),
            Arg::Text(key.to_string()),
            Arg::Int(task_id),
        ],
        &TASK_JSON_COLS,
    )?;
    Ok(rows.into_iter().find(|r| claim_guard_reason(r).is_none()))
}

/// /repair (#13191): repair one task's packet in place. See the
/// module header for the contract. The reply row is the repair
/// report: outcome, the defects found, the repairs applied (or
/// planned, under --dry-run), and the task row after the repair.
pub(crate) fn verb_repair(t: &TursoBackend, req: &Value) -> Value {
    let task_id = req_i64(req, "task_id").unwrap_or(0);
    let worker = req_str(req, "worker").unwrap_or("").to_string();
    let reason = req_str(req, "reason").unwrap_or("").to_string();
    let dry_run = req
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let work = (|| -> Result<Value, String> {
        let Some(row) = get_task(t, task_id)? else {
            return Err(format!("repair_task: task {task_id} does not exist"));
        };
        let status = row
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match status.as_str() {
            "blocked" | "queued" | "parked" | "handed_off" => {}
            "claimed" => {
                let holder = row
                    .get("worker_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("a worker");
                return Err(format!(
                    "repair_task: task {task_id} is claimed by {holder}; \
                     repair never mutates a task a worker holds"
                ));
            }
            "cancelled" | "failed" => {
                return Err(format!(
                    "repair_task: task {task_id} is {status}; repair does not \
                     restore a {status} task — revive it first (`fleet queue \
                     revive {task_id}`), then repair"
                ));
            }
            "done" => {
                return Err(format!(
                    "repair_task: task {task_id} is done (terminal); \
                     there is nothing to repair"
                ));
            }
            other => {
                return Err(format!(
                    "repair_task: task {task_id} is {other}; repair does not \
                     handle that state"
                ));
            }
        }
        // The defect list is the claim guard's own oracle,
        // itemized: claim and repair can never disagree about
        // what is broken.
        let defects = validate_packet(&row).0;
        if defects.is_empty() {
            return Ok(json!({
                "task_id": task_id, "outcome": "no_defects",
                "defects": defects, "repairs": [],
                "dry_run": dry_run, "status": status, "row": row,
            }));
        }
        let unrepairable: Vec<&String> = defects
            .iter()
            .filter(|d| d.as_str() != crate::validate::DEDUPE_STAMP_ERROR)
            .collect();
        if !unrepairable.is_empty() {
            return Err(format!(
                "repair_task: task {task_id} has defects repair cannot fix: {}; \
                 repairs never invent content, and nothing was changed",
                unrepairable
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        // The one repairable class today: the dedupe stamp.
        // Re-perform the dedupe check live before stamping.
        let payload = row.get("payload").cloned().unwrap_or(json!({}));
        let mut checks: Vec<String> = Vec::new();
        if let Some(identity) = work_identity(&row) {
            let kind = payload
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(holder) = live_identity_holder(t, &identity, &kind)? {
                let hid = holder.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let hstatus = holder
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let hqueue = holder
                    .get("queue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return Err(format!(
                    "repair_task: task {task_id}: dedupe re-check found a live \
                     duplicate — task {hid} already holds work identity {} \
                     (status {hstatus}, queue {hqueue}); stamping \
                     checked_at/basis would be a false attestation, and \
                     nothing was changed",
                    identity.key()
                ));
            }
            checks.push(format!("work identity {}", identity.key()));
        }
        if let Some(holder) = live_dedupe_key_holder(t, &row, task_id)? {
            let hid = holder.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let hstatus = holder
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return Err(format!(
                "repair_task: task {task_id}: dedupe re-check found a live \
                 duplicate — task {hid} already holds dedupe key {} in this \
                 queue (status {hstatus}); stamping checked_at/basis would \
                 be a false attestation, and nothing was changed",
                row.get("dedupe_key").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
        if !row
            .get("dedupe_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty()
        {
            checks.push(format!(
                "dedupe key {}",
                row.get("dedupe_key").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
        if checks.is_empty() {
            return Err(format!(
                "repair_task: task {task_id} carries neither a work identity \
                 nor a dedupe key; the dedupe stamp cannot be honestly \
                 re-derived, and nothing was changed"
            ));
        }
        // Stamp in memory, then re-validate the whole packet:
        // the write below happens only for a packet that passes
        // clean, so a repair can never leave a half-fixed row.
        let stamp = now_iso();
        let mut repaired_payload = payload.clone();
        {
            let Some(obj) = repaired_payload.as_object_mut() else {
                return Err(format!(
                    "repair_task: task {task_id}: payload is not an object; \
                     nothing was changed"
                ));
            };
            let mut dedupe = obj.get("dedupe").cloned().unwrap_or(json!({}));
            if !dedupe.is_object() {
                dedupe = json!({});
            }
            let d = dedupe.as_object_mut().unwrap();
            d.insert("checked_at".to_string(), json!(stamp));
            d.insert("basis".to_string(), json!(REPAIR_DEDUPE_BASIS));
            obj.insert("dedupe".to_string(), dedupe);
        }
        let mut repaired_row = row.clone();
        repaired_row["payload"] = repaired_payload.clone();
        let (errors, _) = validate_packet(&repaired_row);
        if !errors.is_empty() {
            return Err(format!(
                "repair_task: task {task_id}: repaired packet still fails \
                 validation: {}; nothing was changed",
                errors.join("; ")
            ));
        }
        if let Some(guard) = claim_guard_reason(&repaired_row) {
            return Err(format!(
                "repair_task: task {task_id}: repaired packet still fails \
                 the claim guard: {guard}; nothing was changed"
            ));
        }
        let repairs = json!([{
            "field": "payload.dedupe",
            "defect": crate::validate::DEDUPE_STAMP_ERROR,
            "action": "stamped checked_at/basis by re-performing the dedupe check live",
            "checked_at": stamp,
            "basis": REPAIR_DEDUPE_BASIS,
            "checks": checks,
        }]);
        // Restore: a blocked task with a dep record stays blocked
        // on its deps (the dep rechecker owns its release); any
        // other blocked task returns to queued. The quarantine
        // error is discarded — the packet it named is fixed. A
        // non-blocked task keeps its state and any unrelated
        // recorded error.
        let restore_status = if status == "blocked" && !dep_block_active(&row) {
            "queued"
        } else {
            status.as_str()
        };
        let recorded_error = row
            .get("last_error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let new_error: Option<&str> =
            if status == "blocked" || recorded_error.starts_with("quarantined by claim path") {
                None
            } else if recorded_error.is_empty() {
                None
            } else {
                Some(recorded_error.as_str())
            };
        if dry_run {
            return Ok(json!({
                "task_id": task_id, "outcome": "dry_run",
                "defects": defects, "repairs": repairs,
                "dry_run": true, "status": status,
                "status_after": restore_status, "row": row,
            }));
        }
        let res = t.exec(
            "UPDATE tasks SET payload = ?, status = ?, last_error = ?, \
               updated_at = ? WHERE id = ? AND status = ?",
            &[
                Arg::Text(repaired_payload.to_string()),
                Arg::Text(restore_status.to_string()),
                arg_str(new_error),
                Arg::Text(now_iso()),
                Arg::Int(task_id),
                Arg::Text(status.clone()),
            ],
        )?;
        if res.affected != 1 {
            return Err(format!(
                "repair_task: task {task_id} changed state while repair ran; \
                 nothing was changed"
            ));
        }
        event(
            t,
            Some(&worker),
            Some(task_id),
            "repaired",
            &with_marks(
                &json!({
                    "via": "repair_task_turso",
                    "operator": worker,
                    "reason": reason,
                    "defects": defects.clone(),
                    "repairs": repairs.clone(),
                    "status_before": status,
                    "status_after": restore_status,
                    "discarded_error": recorded_error,
                }),
                &row,
            ),
        )?;
        let after = get_task(t, task_id)?.unwrap_or(Value::Null);
        Ok(json!({
            "task_id": task_id, "outcome": "repaired",
            "defects": defects, "repairs": repairs,
            "dry_run": false, "status_before": status,
            "status_after": restore_status, "row": after,
        }))
    })();
    match work {
        Ok(report) => ok(json!({"row": report})),
        Err(e) if e.starts_with("repair_task:") => fail(5, e),
        Err(e) => terr(e),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A scripted libsql pipeline server, the closed_bound
    /// stub_pipeline_seq pattern: each backend call takes the
    /// next scripted reply; every request body is captured.
    fn stub_pipeline_seq(replies: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(
            replies
                .into_iter()
                .collect::<std::collections::VecDeque<_>>(),
        ));
        let sink = captured.clone();
        let pending = queue.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else { break };
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.trim().eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0u8; content_length];
                if reader.read_exact(&mut body).is_err() {
                    break;
                }
                sink.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&body).to_string());
                let resp = pending.lock().unwrap().pop_front().unwrap_or_else(|| {
                    json!({"results": [
                        {"type": "ok", "response": {"type": "execute", "result": {
                            "cols": [], "rows": [],
                            "affected_row_count": 0,
                            "last_insert_rowid": null}}},
                        {"type": "ok", "response": {"type": "close"}}
                    ]})
                    .to_string()
                });
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp.len(),
                    resp
                );
                if sock.write_all(out.as_bytes()).is_err() {
                    break;
                }
            }
        });
        (format!("http://{addr}"), captured)
    }

    fn cell_text(v: &str) -> Value {
        json!({"type": "text", "value": v})
    }

    fn cell_int(v: i64) -> Value {
        json!({"type": "integer", "value": v.to_string()})
    }

    /// One task row in TASK_COLS order for the stub replies.
    fn task_row(
        id: i64,
        status: &str,
        payload: &Value,
        dedupe_key: &str,
        last_error: Option<&str>,
        blocked_deps: Option<&str>,
    ) -> Value {
        let nullc = json!({"type": "null"});
        let opt = |v: Option<&str>| match v {
            Some(s) => cell_text(s),
            None => nullc.clone(),
        };
        json!([
            cell_int(id),
            cell_text("repo-health-code-pilot"),
            cell_text("probe"),
            nullc.clone(),
            cell_text(&payload.to_string()),
            cell_text(dedupe_key),
            cell_int(30),
            cell_text(status),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            cell_int(1),
            cell_int(1000),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            cell_text("{}"),
            cell_text("{}"),
            opt(last_error),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            nullc.clone(),
            cell_text("2026-11-20T00:00:00Z"),
            cell_text("2026-11-21T00:00:00Z"),
            nullc.clone(),
            opt(blocked_deps),
            nullc.clone(),
            nullc.clone(),
            nullc.clone()
        ])
    }

    fn select_reply(rows: Vec<Value>) -> String {
        let cols: Vec<Value> = TASK_COLS
            .split(", ")
            .map(|c| json!({"name": c.trim()}))
            .collect();
        json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": cols, "rows": rows,
                "affected_row_count": 0, "last_insert_rowid": null}}},
            {"type": "ok", "response": {"type": "close"}}
        ]})
        .to_string()
    }

    fn exec_ok(affected: i64) -> String {
        json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": [], "rows": [],
                "affected_row_count": affected,
                "last_insert_rowid": null}}},
            {"type": "ok", "response": {"type": "close"}}
        ]})
        .to_string()
    }

    /// The full valid packet payload (the #13091 holder shape):
    /// passes the claim guard as-is.
    fn valid_payload(issue: i64) -> Value {
        json!({
            "schema": "repo-task/v1",
            "kind": "small_fix",
            "repo": "developing-today/code",
            "problem": "p",
            "priority_reason": "r",
            "evidence": ["e"],
            "acceptance": ["a"],
            "tests": ["t"],
            "context_refs": [],
            "scope": {"include": ["x"], "exclude": ["y"]},
            "risk": {"level": "low", "flags": []},
            "timebox_minutes": 45,
            "issue": {"number": issue},
            "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test"}
        })
    }

    /// The #7709 shape: the same packet with the dedupe stamp
    /// missing entirely.
    fn dedupe_defect_payload(issue: i64) -> Value {
        let mut p = valid_payload(issue);
        p.as_object_mut().unwrap().remove("dedupe");
        p
    }

    const KEY_7709: &str = "repo-health:v1:fix:developing-today/code:issue:7709";
    const QUARANTINE_7709: &str = "quarantined by claim path: packet failed validation (#5444): payload.dedupe: must have checked_at and basis as non-empty strings";

    fn candidate_row() -> Value {
        task_row(
            77001,
            "blocked",
            &dedupe_defect_payload(7709),
            KEY_7709,
            Some(QUARANTINE_7709),
            None,
        )
    }

    fn repaired_row() -> Value {
        let mut p = dedupe_defect_payload(7709);
        p["dedupe"] = json!({
            "checked_at": "2026-11-23T10:00:00Z",
            "basis": REPAIR_DEDUPE_BASIS,
        });
        task_row(77001, "queued", &p, KEY_7709, None, None)
    }

    fn backend(url: String) -> TursoBackend {
        TursoBackend::new(url, "test".to_string(), ureq::agent())
    }

    #[test]
    fn repair_stamps_dedupe_and_requeues_13191() {
        // Oracle (#13191): the #7709 shape — dedupe payload
        // missing checked_at/basis, no duplicate live task —
        // repairs, re-validates clean, and becomes claimable.
        let (url, captured) = stub_pipeline_seq(vec![
            select_reply(vec![candidate_row()]),
            select_reply(vec![]),
            select_reply(vec![]),
            exec_ok(1),
            exec_ok(1),
            select_reply(vec![repaired_row()]),
        ]);
        let out = verb_repair(
            &backend(url),
            &json!({"task_id": 77001, "worker": "archer", "dry_run": false}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let report = &out["row"];
        assert_eq!(report["outcome"], "repaired", "{out}");
        assert_eq!(
            report["defects"],
            json!([crate::validate::DEDUPE_STAMP_ERROR]),
            "{out}"
        );
        assert_eq!(report["status_after"], "queued", "{out}");
        // Claimable: the row the verb read back passes the claim
        // guard — the same oracle the claim path runs.
        assert!(claim_guard_reason(&report["row"]).is_none(), "{out}");
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 6, "{bodies:?}");
        assert!(bodies[3].contains("UPDATE tasks SET payload"), "{bodies:?}");
        assert!(bodies[3].contains("checked_at"), "{bodies:?}");
        assert!(bodies[3].contains(REPAIR_DEDUPE_BASIS), "{bodies:?}");
    }

    #[test]
    fn repair_dry_run_writes_nothing_13191() {
        // Oracle (#13191): --dry-run on the #7709 shape reports
        // defects + planned repairs and writes nothing — no
        // UPDATE, no event INSERT reaches the store.
        let (url, captured) = stub_pipeline_seq(vec![
            select_reply(vec![candidate_row()]),
            select_reply(vec![]),
            select_reply(vec![]),
        ]);
        let out = verb_repair(
            &backend(url),
            &json!({"task_id": 77001, "worker": "archer", "dry_run": true}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let report = &out["row"];
        assert_eq!(report["outcome"], "dry_run", "{out}");
        assert_eq!(
            report["defects"],
            json!([crate::validate::DEDUPE_STAMP_ERROR]),
            "{out}"
        );
        assert_eq!(report["repairs"][0]["field"], "payload.dedupe", "{out}");
        assert_eq!(report["status_after"], "queued", "{out}");
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 3, "{bodies:?}");
        for b in bodies.iter() {
            assert!(!b.contains("UPDATE tasks"), "{b}");
            assert!(!b.contains("INSERT INTO events"), "{b}");
        }
    }

    #[test]
    fn repair_refuses_duplicate_identity_13191() {
        // Oracle (#13191): a duplicate-identity dedupe defect is
        // refused; stamping would be a false attestation and the
        // row is unchanged.
        let holder = task_row(77002, "queued", &valid_payload(7709), KEY_7709, None, None);
        let (url, captured) = stub_pipeline_seq(vec![
            select_reply(vec![candidate_row()]),
            select_reply(vec![holder]),
        ]);
        let out = verb_repair(
            &backend(url),
            &json!({"task_id": 77001, "worker": "archer", "dry_run": false}),
        );
        assert_eq!(out["ok"], false, "{out}");
        let err = out["error"].as_str().unwrap_or("");
        assert!(err.contains("live duplicate"), "{out}");
        assert!(err.contains("77002"), "{out}");
        assert!(err.contains("false attestation"), "{out}");
        let bodies = captured.lock().unwrap();
        for b in bodies.iter() {
            assert!(!b.contains("UPDATE tasks"), "{b}");
            assert!(!b.contains("INSERT INTO events"), "{b}");
        }
    }

    #[test]
    fn repair_refuses_unrepairable_defect_13191() {
        // Oracle (#13191): an unrepairable defect fails non-zero
        // and the row is unchanged — the verb makes no further
        // backend call after loading the row.
        let mut p = valid_payload(7709);
        p.as_object_mut().unwrap().remove("risk");
        let row = task_row(77001, "blocked", &p, KEY_7709, Some(QUARANTINE_7709), None);
        let (url, captured) = stub_pipeline_seq(vec![select_reply(vec![row])]);
        let out = verb_repair(
            &backend(url),
            &json!({"task_id": 77001, "worker": "archer", "dry_run": false}),
        );
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["exit"], 5, "{out}");
        let err = out["error"].as_str().unwrap_or("");
        assert!(err.contains("payload.risk.level"), "{out}");
        assert!(err.contains("nothing was changed"), "{out}");
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
    }

    #[test]
    fn repair_timeline_carries_the_repair_entry_13191() {
        // Oracle (#13191): a repaired task's timeline carries the
        // repair entry — verb, operator, defects, repairs.
        let (url, captured) = stub_pipeline_seq(vec![
            select_reply(vec![candidate_row()]),
            select_reply(vec![]),
            select_reply(vec![]),
            exec_ok(1),
            exec_ok(1),
            select_reply(vec![repaired_row()]),
        ]);
        let out = verb_repair(
            &backend(url),
            &json!({"task_id": 77001, "worker": "archer", "dry_run": false}),
        );
        assert_eq!(out["ok"], true, "{out}");
        let bodies = captured.lock().unwrap();
        let event_body = &bodies[4];
        assert!(event_body.contains("INSERT INTO events"), "{event_body}");
        assert!(event_body.contains("repaired"), "{event_body}");
        assert!(event_body.contains("repair_task_turso"), "{event_body}");
        assert!(event_body.contains("archer"), "{event_body}");
        assert!(
            event_body.contains(crate::validate::DEDUPE_STAMP_ERROR),
            "{event_body}"
        );
        assert!(event_body.contains("payload.dedupe"), "{event_body}");
    }

    #[test]
    fn unblock_refuses_a_still_quarantined_row_13191() {
        // The #13191 audit: unblock against a quarantined row
        // whose packet still fails the claim guard refuses,
        // names the quarantine, and points at repair — it must
        // not report the row queued when the next claim would
        // re-quarantine it.
        let (url, captured) = stub_pipeline_seq(vec![select_reply(vec![candidate_row()])]);
        let out = verb_unblock(
            &backend(url),
            &json!({"task_id": 77001, "worker": "archer", "reason": "manual"}),
        );
        assert_eq!(out["ok"], false, "{out}");
        let err = out["error"].as_str().unwrap_or("");
        assert!(err.contains("quarantined"), "{out}");
        assert!(err.contains("fleet queue repair 77001"), "{out}");
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
    }

    #[test]
    fn repair_wiring_is_complete_13191() {
        // The verb is wired end to end: broker dispatch, client
        // dispatch, and the help surface (the fleet book's
        // coverage checklist is generated from that surface).
        let dispatch = include_str!("stacks_outbox.rs");
        assert!(
            dispatch.contains("\"/repair\" => verb_repair"),
            "broker dispatch"
        );
        let client = include_str!("../client.rs");
        assert!(
            client.contains("\"repair\" => cmd_repair"),
            "client dispatch"
        );
        let lifecycle = include_str!("../client/commands_lifecycle.rs");
        assert!(lifecycle.contains("fn cmd_repair"), "client verb");
        let support = include_str!("../client/support.rs");
        assert!(support.contains("repair <task-id>"), "top-level help");
        assert!(support.contains("\"repair\" =>"), "per-verb help");
    }
}
