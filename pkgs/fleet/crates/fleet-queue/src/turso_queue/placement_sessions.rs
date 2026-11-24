//! turso_queue::placement_sessions — placement session records
//! (#13146): the queue-backend home for fleet placement sessions.
//!
//! Split as its own module following the #8589 pattern. The client
//! session record JSON is the contract: `record` stores it verbatim
//! and the scalar columns are extractions for list/filter queries,
//! the same shape tasks use for payload/progress/result.
//!
//! The mirror is one-directional per verb: /placement-put upserts
//! (a seat refreshing a session's state is the authority for that
//! update), /placement-sync inserts-or-ignores (the migration path
//! for pre-existing local stores — it can never overwrite a row the
//! backend already holds).

use super::*;

const PLACEMENT_PUT_SQL: &str = "INSERT INTO placement_sessions \
    (handle, record, state, surface, submitted_by, run_id, created_at, updated_at) \
    VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
    ON CONFLICT(handle) DO UPDATE SET \
    record = excluded.record, state = excluded.state, \
    surface = excluded.surface, submitted_by = excluded.submitted_by, \
    run_id = excluded.run_id, updated_at = excluded.updated_at";

const PLACEMENT_SYNC_SQL: &str = "INSERT OR IGNORE INTO placement_sessions \
    (handle, record, state, surface, submitted_by, run_id, created_at, updated_at) \
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)";

/// The most records one /placement-sync call accepts. The client
/// sweeps in batches of 100; 500 leaves headroom without letting a
/// single request grow unbounded (the broker caps bodies at
/// MAX_BODY_BYTES regardless).
const PLACEMENT_SYNC_MAX: usize = 500;

/// Extract one placement_sessions row from a client session record.
/// The record must be an object carrying a non-empty `fp_` handle
/// and created_at/updated_at stamps — the client always writes all
/// three; anything else is a caller bug, not data to store.
fn session_row(rec: &Value) -> Result<Vec<Arg>, String> {
    if !rec.is_object() {
        return Err("record must be a JSON object".to_string());
    }
    let handle = rec
        .get("handle")
        .and_then(|h| h.as_str())
        .filter(|h| h.starts_with("fp_") && h.len() > 3)
        .ok_or_else(|| "record.handle must be an fp_ session handle".to_string())?;
    let created_at = rec
        .get("created_at")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "record.created_at is required".to_string())?;
    let updated_at = rec
        .get("updated_at")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "record.updated_at is required".to_string())?;
    Ok(vec![
        Arg::Text(handle.to_string()),
        Arg::Text(rec.to_string()),
        arg_str(rec.get("state").and_then(|v| v.as_str())),
        arg_str(rec.get("surface").and_then(|v| v.as_str())),
        arg_str(rec.get("submitted_by").and_then(|v| v.as_str())),
        rec.get("run_id")
            .and_then(|v| v.as_i64())
            .map(Arg::Int)
            .unwrap_or(Arg::Null),
        Arg::Text(created_at.to_string()),
        Arg::Text(updated_at.to_string()),
    ])
}

/// Column names, for the row-shape assertion in tests.
#[cfg(test)]
const PLACEMENT_COLS: &str = "handle, record, state, surface, submitted_by, \
    run_id, created_at, updated_at";

pub(crate) fn verb_placement_put(t: &TursoBackend, req: &Value) -> Value {
    let rec = req.get("record").cloned().unwrap_or(Value::Null);
    let args = match session_row(&rec) {
        Ok(a) => a,
        Err(e) => return fail(2, e),
    };
    let handle = rec
        .get("handle")
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .to_string();
    let work = (|| -> Result<Value, TErr> {
        t.exec(PLACEMENT_PUT_SQL, &args)?;
        Ok(json!({"handle": handle}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_placement_get(t: &TursoBackend, req: &Value) -> Value {
    let Some(handle) = req_str(req, "handle").filter(|h| !h.is_empty()) else {
        return fail(2, "handle is required".to_string());
    };
    let handle = handle.to_string();
    let work = (|| -> Result<Value, TErr> {
        let rows = t.query_objs(
            "SELECT record FROM placement_sessions WHERE handle = ?",
            &[Arg::Text(handle.clone())],
            &["record"],
        )?;
        let record = rows
            .into_iter()
            .next()
            .and_then(|r| r.get("record").cloned())
            .unwrap_or(Value::Null);
        Ok(json!({"handle": handle, "record": record}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_placement_list(t: &TursoBackend, req: &Value) -> Value {
    let limit = req_i64(req, "limit").unwrap_or(100).clamp(1, 500);
    let mine = req_str(req, "submitted_by").map(str::to_string);
    let work = (|| -> Result<Value, TErr> {
        let (sql, args): (&str, Vec<Arg>) = match &mine {
            Some(who) => (
                "SELECT record FROM placement_sessions \
                 WHERE submitted_by = ? ORDER BY created_at DESC LIMIT ?",
                vec![Arg::Text(who.clone()), Arg::Int(limit)],
            ),
            None => (
                "SELECT record FROM placement_sessions \
                 ORDER BY created_at DESC LIMIT ?",
                vec![Arg::Int(limit)],
            ),
        };
        let rows = t.query_objs(sql, &args, &["record"])?;
        let sessions: Vec<Value> = rows
            .iter()
            .filter_map(|r| r.get("record").cloned())
            .filter(|r| !r.is_null())
            .collect();
        Ok(json!({"sessions": sessions}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

pub(crate) fn verb_placement_sync(t: &TursoBackend, req: &Value) -> Value {
    let Some(records) = req.get("records").and_then(|r| r.as_array()) else {
        return fail(2, "records array is required".to_string());
    };
    if records.len() > PLACEMENT_SYNC_MAX {
        return fail(
            2,
            format!(
                "records is capped at {PLACEMENT_SYNC_MAX} per sync call, got {}",
                records.len()
            ),
        );
    }
    // Validate every row before any write: a malformed record fails
    // the batch whole, and the client (best-effort) retries the
    // sweep on a later backend contact.
    let mut stmts: Vec<(&str, Vec<Arg>)> = Vec::with_capacity(records.len());
    for rec in records {
        match session_row(rec) {
            Ok(args) => stmts.push((PLACEMENT_SYNC_SQL, args)),
            Err(e) => return fail(2, e),
        }
    }
    let received = stmts.len();
    let work = (|| -> Result<Value, TErr> {
        let mut inserted: u64 = 0;
        for chunk in stmts.chunks(100) {
            for result in t.exec_batch(chunk)? {
                inserted += result.affected;
            }
        }
        Ok(json!({"inserted": inserted, "received": received}))
    })();
    match work {
        Ok(v) => ok(v),
        Err(e) => terr(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    fn sample_record(handle: &str, updated_at: &str) -> Value {
        json!({
            "handle": handle,
            "job": {"repo": "developing-today/code", "command": "cargo test"},
            "surface": "fleet-org",
            "surface_kind": "github-actions",
            "decision_reasoning": ["test"],
            "run_id": 42,
            "run_url": "https://example.invalid/runs/42",
            "state": "running",
            "submitted_by": "archer@test",
            "actor": "developing-today[bot] (GitHub App dispatch)",
            "created_at": "2026-11-23T08:00:00Z",
            "updated_at": updated_at,
        })
    }

    /// A stub of the Turso pipeline endpoint in the #12744 shape:
    /// captures request bodies and answers each execute statement
    /// with a canned result — `select_rows` (record JSON strings)
    /// when the batch is a SELECT, affected=1 per statement
    /// otherwise.
    fn stub_pipeline(
        select_rows: Vec<String>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            loop {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
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
                let text = String::from_utf8_lossy(&body).to_string();
                let executes = text.matches("\"type\":\"execute\"").count();
                let is_select = text.contains("SELECT record FROM placement_sessions");
                sink.lock().unwrap().push(text);
                let mut results = Vec::new();
                for i in 0..executes {
                    let result = if is_select && i == 0 {
                        let rows: Vec<Value> = select_rows
                            .iter()
                            .map(|r| json!([{"type": "text", "value": r}]))
                            .collect();
                        json!({"cols": [{"name": "record"}], "rows": rows,
                               "affected_row_count": 0, "last_insert_rowid": null})
                    } else {
                        json!({"cols": [], "rows": [],
                               "affected_row_count": 1, "last_insert_rowid": null})
                    };
                    results.push(
                        json!({"type": "ok", "response": {"type": "execute", "result": result}}),
                    );
                }
                let resp_body = json!({"results": results}).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                if sock.write_all(resp.as_bytes()).is_err() {
                    return;
                }
            }
        });
        (format!("http://{addr}"), captured)
    }

    fn backend(url: String) -> TursoBackend {
        TursoBackend::new(url, "test".to_string(), ureq::agent())
    }

    #[test]
    fn session_row_extracts_scalars_and_rejects_bad_records() {
        // Oracle: the row shape the schema declares — handle first,
        // record verbatim, scalars extracted, run_id as INTEGER.
        let rec = sample_record("fp_abc123", "2026-11-23T08:05:00Z");
        let args = session_row(&rec).unwrap();
        assert_eq!(args.len(), PLACEMENT_COLS.split(',').count());
        assert!(matches!(&args[0], Arg::Text(h) if h == "fp_abc123"));
        assert!(matches!(&args[1], Arg::Text(r) if r == &rec.to_string()));
        assert!(matches!(&args[5], Arg::Int(42)));
        // A record without an fp_ handle is a caller bug.
        assert!(session_row(&json!({"handle": "xx"})).is_err());
        assert!(session_row(&json!({"handle": "fp_x"})).is_err());
        assert!(session_row(&json!({})).is_err());
        assert!(session_row(&json!("nope")).is_err());
    }

    #[test]
    fn placement_put_upserts_the_record() {
        // Oracle: one execute carrying the upsert SQL and the full
        // record; the reply names the stored handle.
        let (url, captured) = stub_pipeline(vec![]);
        let out = verb_placement_put(
            &backend(url),
            &json!({"record": sample_record("fp_abc123", "2026-11-23T08:05:00Z")}),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["handle"], "fp_abc123", "{out}");
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        assert!(
            bodies[0].contains("INSERT INTO placement_sessions"),
            "{bodies:?}"
        );
        assert!(
            bodies[0].contains("ON CONFLICT(handle) DO UPDATE"),
            "{bodies:?}"
        );
        assert!(bodies[0].contains("fp_abc123"), "{bodies:?}");
    }

    #[test]
    fn placement_put_rejects_a_record_without_handle() {
        let (url, captured) = stub_pipeline(vec![]);
        let out = verb_placement_put(&backend(url), &json!({"record": {"state": "running"}}));
        assert_eq!(out["ok"], false, "{out}");
        // Validation happens before any statement is sent.
        assert!(captured.lock().unwrap().is_empty());
    }

    #[test]
    fn placement_get_returns_the_stored_record_or_null() {
        // Oracle: the stored record JSON round-trips through the
        // record column; a missing handle reads back as null, not
        // an error (a fresh seat has no rows yet).
        let rec = sample_record("fp_abc123", "2026-11-23T08:05:00Z");
        let (url, _) = stub_pipeline(vec![rec.to_string()]);
        let out = verb_placement_get(&backend(url), &json!({"handle": "fp_abc123"}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["record"], rec, "{out}");

        let (url, _) = stub_pipeline(vec![]);
        let out = verb_placement_get(&backend(url), &json!({"handle": "fp_missing"}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["record"], Value::Null, "{out}");
    }

    #[test]
    fn placement_list_returns_records_newest_first_shape() {
        let a = sample_record("fp_a", "2026-11-23T08:05:00Z");
        let b = sample_record("fp_b", "2026-11-23T08:06:00Z");
        let (url, captured) = stub_pipeline(vec![b.to_string(), a.to_string()]);
        let out = verb_placement_list(&backend(url), &json!({"limit": 10}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["sessions"], json!([b, a]), "{out}");
        let bodies = captured.lock().unwrap();
        assert!(bodies[0].contains("ORDER BY created_at DESC"), "{bodies:?}");
    }

    #[test]
    fn placement_sync_inserts_or_ignores_and_counts() {
        // Oracle: the migration verb NEVER overwrites — its SQL is
        // INSERT OR IGNORE — and the reply counts affected rows
        // against received records.
        let (url, captured) = stub_pipeline(vec![]);
        let out = verb_placement_sync(
            &backend(url),
            &json!({"records": [
                sample_record("fp_a", "2026-11-23T08:05:00Z"),
                sample_record("fp_b", "2026-11-23T08:06:00Z"),
            ]}),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["received"], 2, "{out}");
        assert_eq!(out["inserted"], 2, "{out}");
        let bodies = captured.lock().unwrap();
        assert!(
            bodies[0].contains("INSERT OR IGNORE INTO placement_sessions"),
            "{bodies:?}"
        );
        assert!(!bodies[0].contains("ON CONFLICT"), "{bodies:?}");
    }

    #[test]
    fn placement_sync_rejects_a_bad_record_before_writing() {
        let (url, captured) = stub_pipeline(vec![]);
        let out = verb_placement_sync(
            &backend(url),
            &json!({"records": [sample_record("fp_a", "2026-11-23T08:05:00Z"), {"no": "handle"}]}),
        );
        assert_eq!(out["ok"], false, "{out}");
        assert!(captured.lock().unwrap().is_empty());
    }
}
