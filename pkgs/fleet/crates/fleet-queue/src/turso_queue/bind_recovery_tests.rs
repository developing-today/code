//! #16066 backend regressions. Oracle: real SQLite state after the
//! public bind verb, served through the existing Hrana test endpoint.
use super::reprioritize_tests::{events_for, fake_backend_with_race};
use super::*;

const SESSION: &str = "ses_prior16066";
const MARKER: &str = "opencode-claimant-recovery:error-stopped";

fn seed(t: &TursoBackend) -> i64 {
    let id = t.exec(
        "INSERT INTO tasks (queue, status, worker_id, lease_until, attempts, max_attempts, progress, created_at, updated_at) \
         VALUES ('test', 'claimed', 'holder', '2000-01-01T00:00:00Z', 2, 4, ?, 'old', 'old')",
        &[Arg::Text(json!({"lane": "opencode", "session_id": SESSION,
            "launched_by": "opencode-claimant", "keep": {"nested": true}}).to_string())],
    ).unwrap().last_insert_rowid.unwrap();
    t.exec(
        "INSERT INTO repo_task_lane_ledger (queue, task_id, assigned_lane, session_id, updated_at) \
         VALUES ('test', ?, 'opencode', ?, 'old')",
        &[Arg::Int(id), Arg::Text(SESSION.into())],
    )
    .unwrap();
    id
}

fn recover(t: &TursoBackend, id: i64, worker: &str, session: &str, marker: &str) -> Value {
    verb_bind(
        t,
        &json!({"task_id": id, "worker": worker,
        "session_id": session, "launched_by": marker}),
    )
}

fn snapshot(t: &TursoBackend, id: i64) -> Value {
    json!({"task": get_task(t, id).unwrap(), "ledger": get_ledger(t, id).unwrap(),
        "events": events_for(t, id, "lane_session_recovered")})
}

#[test]
fn recovery_refusals_preserve_both_bindings_and_attempt_budget() {
    let (t, _, _) = fake_backend_with_race();
    for mutation in [
        "UPDATE tasks SET status = 'queued' WHERE id = ?",
        "UPDATE tasks SET progress = json_set(progress, '$.lane', 'direct') WHERE id = ?",
        "UPDATE tasks SET progress = json_set(progress, '$.session_id', 'ses_replacement') WHERE id = ?",
        "UPDATE repo_task_lane_ledger SET assigned_lane = 'direct' WHERE task_id = ?",
        "UPDATE repo_task_lane_ledger SET session_id = 'ses_replacement' WHERE task_id = ?",
        "UPDATE repo_task_lane_ledger SET session_id = NULL WHERE task_id = ?",
        "DELETE FROM repo_task_lane_ledger WHERE task_id = ?",
        "UPDATE tasks SET lease_until = '2999-01-01T00:00:00Z' WHERE id = ?",
        "UPDATE tasks SET attempts = max_attempts WHERE id = ?",
    ] {
        let id = seed(&t);
        t.exec(mutation, &[Arg::Int(id)]).unwrap();
        let before = snapshot(&t, id);
        let out = recover(&t, id, "opencode-claimant", SESSION, MARKER);
        assert_eq!(out["ok"], false, "{mutation}: {out}");
        assert_eq!(snapshot(&t, id), before, "{mutation}");
    }
    for (worker, session) in [
        ("wrong-worker", SESSION),
        ("opencode-claimant", "ses_stale"),
    ] {
        let id = seed(&t);
        let before = snapshot(&t, id);
        let out = recover(&t, id, worker, session, MARKER);
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(snapshot(&t, id), before);
    }
}

#[test]
fn recovery_success_clears_both_and_records_one_attempt_and_event() {
    let (t, _, _) = fake_backend_with_race();
    for (index, marker) in RECOVERY_LAUNCHED_BY.iter().enumerate() {
        let id = seed(&t);
        // A missing lease is expired too, including at the attempt boundary.
        if index == 0 {
            t.exec(
                "UPDATE tasks SET lease_until = NULL, attempts = 3 WHERE id = ?",
                &[Arg::Int(id)],
            )
            .unwrap();
        }
        let before = get_task(&t, id).unwrap().unwrap();
        let out = recover(&t, id, "opencode-claimant", SESSION, marker);
        assert_eq!(out["ok"], true, "{out}");
        let task = get_task(&t, id).unwrap().unwrap();
        let ledger = get_ledger(&t, id).unwrap().unwrap();
        assert!(ledger["session_id"].is_null());
        assert_eq!(
            task["attempts"].as_i64(),
            before["attempts"].as_i64().map(|n| n + 1)
        );
        assert_eq!(task["progress"]["session_id"], Value::Null);
        assert_eq!(task["progress"]["launched_by"], Value::Null);
        assert_eq!(task["progress"]["stage"], "launch_failed");
        assert_eq!(task["progress"]["launch_failed_session_id"], SESSION);
        assert_eq!(
            task["progress"]["launch_failed_reason"],
            recovery_reason(marker)
        );
        assert_eq!(task["progress"]["launch_failed_at"], task["updated_at"]);
        assert_eq!(ledger["updated_at"], task["updated_at"]);
        for key in ["status", "lease_until", "worker_id", "max_attempts"] {
            assert_eq!(task[key], before[key], "{key}");
        }
        assert_eq!(task["progress"]["keep"], before["progress"]["keep"]);
        assert_eq!(task["progress"]["lane"], "opencode");
        let events = events_for(&t, id, "lane_session_recovered");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["worker_id"], "opencode-claimant");
        let detail: Value = serde_json::from_str(events[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["session_id"], SESSION);
        assert_eq!(detail["stage"], "launch_failed");
        assert_eq!(detail["reason"], recovery_reason(marker));
        // A stale retry must neither consume another attempt nor duplicate the event.
        let after = snapshot(&t, id);
        assert_eq!(
            recover(&t, id, "opencode-claimant", SESSION, marker)["ok"],
            false
        );
        assert_eq!(snapshot(&t, id), after);
    }
}

#[test]
fn recovery_rechecks_all_guards_after_unlocked_diagnostic_reads() {
    let (t, _, hook) = fake_backend_with_race();
    for mutation in [
        "UPDATE tasks SET status = 'done' WHERE id = {id}",
        "UPDATE tasks SET lease_until = '2999-01-01T00:00:00Z' WHERE id = {id}",
        "UPDATE tasks SET attempts = max_attempts WHERE id = {id}",
        "UPDATE tasks SET progress = json_set(progress, '$.lane', 'direct') WHERE id = {id}",
        "UPDATE tasks SET progress = json_set(progress, '$.session_id', 'ses_new') WHERE id = {id}",
        "UPDATE repo_task_lane_ledger SET assigned_lane = 'direct' WHERE task_id = {id}",
        "UPDATE repo_task_lane_ledger SET session_id = 'ses_new' WHERE task_id = {id}",
    ] {
        let id = seed(&t);
        let before = snapshot(&t, id);
        *hook.lock().unwrap() = Some(mutation.replace("{id}", &id.to_string()));
        let out = recover(&t, id, "opencode-claimant", SESSION, MARKER);
        assert_eq!(out["ok"], false, "{mutation}: {out}");
        // The fake injects the competing mutation inside our transaction;
        // rollback restores it too. A real competitor commits before BEGIN.
        assert_eq!(snapshot(&t, id), before, "{mutation}");
    }
}

#[test]
fn recovery_rolls_back_partial_writes_on_error_or_zero_row_clear() {
    for trigger in [
        "CREATE TRIGGER fail_clear BEFORE UPDATE ON repo_task_lane_ledger BEGIN SELECT RAISE(ABORT, 'injected ledger error'); END",
        "CREATE TRIGGER fail_clear BEFORE UPDATE ON repo_task_lane_ledger BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER fail_event BEFORE INSERT ON events WHEN NEW.kind = 'lane_session_recovered' BEGIN SELECT RAISE(ABORT, 'injected event error'); END",
    ] {
        let (t, db, _) = fake_backend_with_race();
        let id = seed(&t);
        let before = snapshot(&t, id);
        db.lock().unwrap().execute_batch(trigger).unwrap();
        let out = recover(&t, id, "opencode-claimant", SESSION, MARKER);
        assert_eq!(out["ok"], false, "{trigger}: {out}");
        assert_eq!(snapshot(&t, id), before, "{trigger}");
        assert!(
            db.lock().unwrap().is_autocommit(),
            "transaction must be closed"
        );
    }
}
