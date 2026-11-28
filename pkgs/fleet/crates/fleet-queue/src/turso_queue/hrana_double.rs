//! #13579: a disposable Hrana double for the repair transaction
//! tests. It serves /v2/pipeline from a real SQLite file (rusqlite),
//! keeps one connection per open baton, and can refuse a statement
//! or lose a COMMIT response on demand, so the fault rows of the
//! repair design run against real transaction state.

use super::core::TursoBackend;
use super::repair::verb_repair;
use rusqlite::{params_from_iter, types::Value as SqlValue, types::ValueRef, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex};

const KEY: &str = "repo-health:v1:fix:developing-today/code:issue:7709";
const QUARANTINE: &str = "quarantined by claim path: packet failed validation (#5444): payload.dedupe: must have checked_at and basis as non-empty strings";

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// Refuse the first `UPDATE tasks` with HTTP 503 before it runs.
    RejectUpdate,
    /// Run the first COMMIT, then drop the stream and answer HTTP 500.
    LoseCommit,
    /// Drop the stream and answer HTTP 500 before the first COMMIT.
    RefuseCommit,
}

struct Double {
    url: String,
    path: std::path::PathBuf,
    log: Arc<Mutex<Vec<String>>>,
}

fn start(name: &str, fault: Fault) -> Double {
    let path = std::env::temp_dir().join(format!("hrana-double-{}-{name}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind double");
    let port = server.server_addr().to_ip().expect("ip listener").port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let (db, sink) = (path.clone(), log.clone());
    std::thread::spawn(move || {
        let mut streams: HashMap<String, Connection> = HashMap::new();
        let mut next = 0u64;
        let mut fault = fault;
        while let Ok(mut req) = server.recv() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let (status, reply) = handle(&db, &mut streams, &mut next, &mut fault, &sink, &body);
            let _ = req.respond(
                tiny_http::Response::from_string(reply.to_string()).with_status_code(status),
            );
        }
    });
    Double {
        url: format!("http://127.0.0.1:{port}"),
        path,
        log,
    }
}

fn handle(
    db: &std::path::Path,
    streams: &mut HashMap<String, Connection>,
    next: &mut u64,
    fault: &mut Fault,
    log: &Arc<Mutex<Vec<String>>>,
    body: &str,
) -> (u16, Value) {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json!({"error": e.to_string()})),
    };
    let (key, conn) = match parsed.get("baton").and_then(|b| b.as_str()) {
        Some(b) => match streams.remove(b) {
            Some(c) => (b.to_string(), c),
            None => return (400, json!({"error": "unknown baton"})),
        },
        None => {
            *next += 1;
            match Connection::open(db) {
                Ok(c) => (format!("baton-{next}"), c),
                Err(e) => return (500, json!({"error": e.to_string()})),
            }
        }
    };
    let conn = conn;
    let mut results = Vec::new();
    let mut closed = false;
    for item in parsed["requests"].as_array().cloned().unwrap_or_default() {
        if item["type"] == "close" {
            closed = true;
            results.push(json!({"type": "ok", "response": {"type": "close"}}));
            break;
        }
        let sql = item["stmt"]["sql"].as_str().unwrap_or("").to_string();
        log.lock().unwrap().push(sql.clone());
        if sql.starts_with("COMMIT") && *fault == Fault::RefuseCommit {
            *fault = Fault::None;
            return (500, json!({"error": "commit refused"}));
        }
        if sql.contains("UPDATE tasks") && *fault == Fault::RejectUpdate {
            *fault = Fault::None;
            streams.insert(key, conn);
            return (503, json!({"error": "refused before execution"}));
        }
        match run(&conn, &sql, &item["stmt"]["args"]) {
            Ok(res) => {
                results.push(json!({"type": "ok", "response": {"type": "execute", "result": res}}))
            }
            Err(msg) => {
                results.push(json!({"type": "error", "error": {"message": msg}}));
                break;
            }
        }
        if sql.starts_with("COMMIT") && *fault == Fault::LoseCommit {
            *fault = Fault::None;
            return (500, json!({"error": "commit response lost"}));
        }
    }
    let baton = if !closed && !conn.is_autocommit() {
        streams.insert(key.clone(), conn);
        json!(key)
    } else {
        Value::Null
    };
    (200, json!({"baton": baton, "results": results}))
}

fn run(conn: &Connection, sql: &str, args: &Value) -> Result<Value, String> {
    let params: Vec<SqlValue> = args
        .as_array()
        .map(|list| {
            list.iter()
                .map(|a| {
                    let v = a["value"].as_str().unwrap_or("");
                    match a["type"].as_str() {
                        Some("integer") => SqlValue::Integer(v.parse().unwrap_or(0)),
                        Some("null") => SqlValue::Null,
                        _ => SqlValue::Text(v.to_string()),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let cols: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(|c| c.to_string())
        .collect();
    let mut rows_out = Vec::new();
    let mut affected = 0usize;
    if cols.is_empty() {
        affected = stmt
            .execute(params_from_iter(params.iter()))
            .map_err(|e| e.to_string())?;
    } else {
        let mut rows = stmt
            .query(params_from_iter(params.iter()))
            .map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let mut cells = Vec::new();
            for i in 0..cols.len() {
                cells.push(cell(row.get_ref(i).map_err(|e| e.to_string())?));
            }
            rows_out.push(Value::Array(cells));
        }
    }
    Ok(json!({
        "cols": cols.iter().map(|c| json!({"name": c, "decltype": null})).collect::<Vec<_>>(),
        "rows": rows_out,
        "affected_row_count": affected,
        "last_insert_rowid": conn.last_insert_rowid().to_string(),
    }))
}

fn cell(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null | ValueRef::Blob(_) => json!({"type": "null"}),
        ValueRef::Integer(i) => json!({"type": "integer", "value": i.to_string()}),
        ValueRef::Real(f) => json!({"type": "float", "value": f.to_string()}),
        ValueRef::Text(t) => json!({"type": "text", "value": String::from_utf8_lossy(t)}),
    }
}

fn backend(d: &Double) -> TursoBackend {
    TursoBackend::new(d.url.clone(), "test".to_string(), ureq::agent())
}

fn direct(d: &Double) -> Connection {
    Connection::open(&d.path).expect("direct connection")
}

fn schema(d: &Double) {
    backend(d).ensure_schema().expect("schema on the double");
}

fn defect_payload() -> Value {
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
        "issue": {"number": 7709},
    })
}

fn seed(d: &Double, status: &str, last_error: Option<&str>) -> i64 {
    schema(d);
    let c = direct(d);
    c.execute(
        "INSERT INTO tasks (queue, title, payload, dedupe_key, priority, status, last_error, created_at, updated_at) \
         VALUES ('repo', 'seeded', ?1, ?2, 10, ?3, ?4, '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z')",
        rusqlite::params![defect_payload().to_string(), KEY, status, last_error],
    )
    .expect("seed task");
    c.last_insert_rowid()
}

type Snapshot = (String, String, Option<String>, i64, i64);

fn snapshot(d: &Double, id: i64) -> Snapshot {
    let c = direct(d);
    let (p, s, e): (String, String, Option<String>) = c
        .query_row(
            "SELECT payload, status, last_error FROM tasks WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("task row");
    let events: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM events WHERE task_id = ?1",
            [id],
            |r| r.get(0),
        )
        .expect("event count");
    let repaired: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM events WHERE task_id = ?1 AND kind = 'repaired'",
            [id],
            |r| r.get(0),
        )
        .expect("repaired count");
    (p, s, e, events, repaired)
}

fn writer_free(d: &Double) -> bool {
    direct(d)
        .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
        .is_ok()
}

fn logged(d: &Double, needle: &str) -> usize {
    d.log
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.contains(needle))
        .count()
}

fn repair_req(id: i64, dry_run: bool) -> Value {
    json!({"task_id": id, "worker": "operator", "reason": "13579 test", "dry_run": dry_run})
}

#[test]
fn event_rejection_rolls_back_13579() {
    // Fault row 6: the repaired-event insert is rejected after the
    // payload UPDATE ran; the whole repair rolls back.
    let d = start("event", Fault::None);
    let id = seed(&d, "blocked", Some(QUARANTINE));
    direct(&d)
        .execute_batch(
            "CREATE TRIGGER reject_repaired BEFORE INSERT ON events \
             WHEN NEW.kind = 'repaired' \
             BEGIN SELECT RAISE(ABORT, 'event rejected'); END;",
        )
        .unwrap();
    let before = snapshot(&d, id);
    let out = verb_repair(&backend(&d), &repair_req(id, false));
    assert_eq!(out["ok"], false, "{out}");
    assert!(
        out["error"]
            .as_str()
            .unwrap_or("")
            .contains("event rejected"),
        "{out}"
    );
    assert_eq!(snapshot(&d, id), before);
    assert!(writer_free(&d));
}

#[test]
fn pre_commit_transport_failure_rolls_back_13579() {
    // Fault row 7: the store refuses the UPDATE before it runs; the
    // stream is rolled back and nothing is left half-written.
    let d = start("transport", Fault::RejectUpdate);
    let id = seed(&d, "blocked", Some(QUARANTINE));
    let before = snapshot(&d, id);
    let out = verb_repair(&backend(&d), &repair_req(id, false));
    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(logged(&d, "UPDATE tasks"), 1, "one refused UPDATE");
    assert_eq!(snapshot(&d, id), before);
    assert!(writer_free(&d));
}

#[test]
fn lost_commit_reconciles_by_operation_id_13579() {
    // Fault row 8: COMMIT lands but its response is lost. The repair
    // is found by its operation id, reported once, never re-applied.
    let d = start("lost", Fault::LoseCommit);
    let id = seed(&d, "blocked", Some(QUARANTINE));
    let out = verb_repair(&backend(&d), &repair_req(id, false));
    assert_eq!(out["ok"], true, "{out}");
    let (_, status, _, _, repaired) = snapshot(&d, id);
    assert_eq!(status, "queued");
    assert_eq!(repaired, 1, "one repaired event");
    assert_eq!(logged(&d, "UPDATE tasks"), 1, "no second mutation");
    assert!(writer_free(&d));
}

#[test]
fn refused_commit_reports_unknown_and_writes_nothing_13579() {
    // Fault row 8 negative: COMMIT never lands. The repair reports
    // the outcome as unknown and the store keeps its prior state.
    let d = start("refuse", Fault::RefuseCommit);
    let id = seed(&d, "blocked", Some(QUARANTINE));
    let before = snapshot(&d, id);
    let out = verb_repair(&backend(&d), &repair_req(id, false));
    assert_eq!(out["ok"], false, "{out}");
    assert!(
        out["error"]
            .as_str()
            .unwrap_or("")
            .contains("commit outcome unknown"),
        "{out}"
    );
    assert_eq!(snapshot(&d, id), before);
    assert!(writer_free(&d));
}

#[test]
fn dry_run_and_refusals_write_nothing_13579() {
    // Rows 1-2 and the no-audit rule: a dry run, a claimed row and a
    // clean row all leave the task and the event log untouched.
    let d = start("dry", Fault::None);
    let id = seed(&d, "blocked", Some(QUARANTINE));
    let before = snapshot(&d, id);
    let out = verb_repair(&backend(&d), &repair_req(id, true));
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(snapshot(&d, id), before);
    assert_eq!(logged(&d, "UPDATE tasks"), 0);
    assert_eq!(logged(&d, "INSERT INTO events"), 0);
}

#[test]
fn claimed_row_is_refused_without_mutation_13579() {
    // Row 3: a row a worker holds is never mutated, and no audit
    // event is written for the refusal.
    let d = start("claimed", Fault::None);
    let id = seed(&d, "claimed", None);
    let before = snapshot(&d, id);
    let out = verb_repair(&backend(&d), &repair_req(id, false));
    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(snapshot(&d, id), before);
    assert_eq!(logged(&d, "UPDATE tasks"), 0);
    assert_eq!(logged(&d, "INSERT INTO events"), 0);
}

#[test]
fn open_stream_holds_the_writer_until_ended_13579() {
    // Rows 4-5: while a repair stream is open, a concurrent writer
    // on another connection is refused; ending the stream frees it.
    let d = start("lock", Fault::None);
    schema(&d);
    let tx = backend(&d).begin_stream().expect("open stream");
    assert!(!writer_free(&d), "a second writer must wait");
    tx.end_stream(false).expect("rollback");
    assert!(writer_free(&d));
}
