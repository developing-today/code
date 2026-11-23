//! turso_queue::closed_bound — the closed-bound detective (#9177).
//!
//! The 2026-11-13 incident: an unidentified writer closed bound,
//! in-progress issue #9155 eight minutes into its bound task's run,
//! poisoning the fleet conductor. The preventive guard (fix 1, in
//! `fleet gh`) refuses such closes at the wrapper; this module is
//! the periodic net (fix 2) for everything else — including writes
//! that never pass through the wrapper: a bound in-progress task
//! whose issue GitHub reports CLOSED without the bound task's
//! completion record raises one throat alert per sweep cycle.
//!
//! The completion-record contract is the merged fleet-closure/v1
//! JSONL (PR #11440), one JSON object per line:
//!   {"schema":"fleet-closure/v1","action":"close",
//!    "item":{"kind":"issue","number":9177,...},"reason":...,
//!    "at_unix":...,
//!    "binding":{"run_id":null,"worker":...,"lane":...,
//!               "task_id":"20372","session_id":...,"user":...}}
//! A record is completion evidence for a bound task when its
//! action is "close", its item.number is the issue, its
//! binding.task_id is the task id (string compare), and its
//! item.kind is "issue" when the record names one.
//!
//! Single-host assumption: the closure log lives on the BROKER
//! host — `FLEET_CLOSURE_LOG` when set and non-empty, else the
//! fleet state dir's `closure-records.jsonl` (the same path the
//! `fleet gh` writer appends to). Workers on other hosts must
//! present their completion records through paths that land on the
//! broker host; a record the broker cannot read is the same as no
//! record. The alert is warning-severity for exactly that reason:
//! the guard is the preventive half, this pass only reports.
//!
//! Fail-open, always (standing rule 2026-10-30): no probe, a GitHub
//! read that fails (Unknown), or an unreadable log never alerts —
//! it is counted in the summary instead. The sweep wiring keeps the
//! same promise one level up: a detective error is reported inside
//! its summary and never fails the sweep.

use super::*;

use std::collections::HashMap;

/// Bound in-progress statuses per the #9177 incident definition:
/// the task holds the issue, so a close without its completion
/// record is the anomaly. Queued, blocked, handed_off and terminal
/// rows are never this pass's business.
const BOUND_STATUSES: [&str; 2] = ["claimed", "parked"];

/// One bound in-progress task's issue, resolved for the pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundIssue {
    pub task_id: i64,
    pub repo: String,
    pub issue: i64,
    pub status: String,
}

/// The pure per-issue verdict: what GitHub says, crossed with what
/// the closure log holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClosedBoundVerdict {
    /// The issue is open, or GitHub could not be read: never an
    /// alert (fail-open on Unknown).
    Open,
    /// Landed, and the bound task's completion record is on file.
    ClosedClean,
    /// Landed, with no completion record for the bound task: the
    /// alert shape, one event plus one throat post per pass.
    ClosedMissingRecord,
}

/// Extract the pass's bound issues from task rows (the SELECT the
/// driver ran, or test-built rows in the same shape).
///
/// A row counts ONLY when its status is claimed|parked AND its
/// `claim_session_id` is non-empty (the #9046 session stamp) — the
/// same 'bound' the preventive guard enforces, so guard and
/// detective agree. Returns the bound issues plus the count of
/// in-status rows skipped for want of a resolvable repo or issue
/// (payload.issue.number, else the repo-health dedupe key under the
/// packet's repo, via [`task_issue`]).
pub(crate) fn bound_issues(rows: &[Value]) -> (Vec<BoundIssue>, i64) {
    let mut bound = Vec::new();
    let mut skipped_no_issue = 0i64;
    for row in rows {
        let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if !BOUND_STATUSES.contains(&status) {
            continue;
        }
        // The session stamp is what makes a claimed|parked row
        // bound: without it no session is behind the task and the
        // guard would not have owned the close either.
        let stamped = row
            .get("claim_session_id")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if !stamped {
            continue;
        }
        let payload = row.get("payload").cloned().unwrap_or(Value::Null);
        let payload = match &payload {
            Value::String(s) => serde_json::from_str(s).unwrap_or(Value::Null),
            v => v.clone(),
        };
        let repo = payload
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let dedupe_key = row.get("dedupe_key").and_then(|v| v.as_str()).unwrap_or("");
        let issue = task_issue(&payload, dedupe_key);
        match (repo.is_empty(), issue) {
            (false, Some(issue)) => bound.push(BoundIssue {
                task_id: row.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                repo,
                issue,
                status: status.to_string(),
            }),
            _ => skipped_no_issue += 1,
        }
    }
    (bound, skipped_no_issue)
}

/// The record's item number, as i64: the writer stores a JSON
/// number, but a digit string reads the same.
fn record_item_number(item: &Value) -> Option<i64> {
    item.get("number").and_then(|n| {
        n.as_i64().or_else(|| {
            n.as_str()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse::<i64>().ok())
        })
    })
}

/// The record's bound task id, as a string: the writer stores the
/// task id as a string, but a JSON number compares the same.
fn record_task_id(binding: &Value) -> Option<String> {
    binding.get("task_id").and_then(|t| match t {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// Whether the closure log holds a completion record for the bound
/// task: one JSON object per line, malformed lines ignored, a line
/// matching only when its action is "close", its item.number is the
/// issue, its binding.task_id is the task id (string compare), and
/// its item.kind is "issue" when the line names one. Pure — the
/// driver reads the file once per pass and hands the text here.
pub(crate) fn has_completion_record(log_text: &str, task_id: i64, issue: i64) -> bool {
    let want_task = task_id.to_string();
    log_text.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() {
            return false;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if record.get("action").and_then(|v| v.as_str()) != Some("close") {
            return false;
        }
        let item = record.get("item").cloned().unwrap_or(Value::Null);
        // A kind other than issue is another record family (a PR
        // close, a successor note); an unnamed kind is the
        // writer's own shape, which always names the issue.
        if let Some(kind) = item.get("kind") {
            if !kind.is_null() && kind.as_str() != Some("issue") {
                return false;
            }
        }
        if record_item_number(&item) != Some(issue) {
            return false;
        }
        let binding = record.get("binding").cloned().unwrap_or(Value::Null);
        record_task_id(&binding).as_deref() == Some(want_task.as_str())
    })
}

/// Cross one GitHub read with the log evidence (pure): a landed
/// issue with the bound task's record is clean; a landed issue
/// without it is the alert; anything still open — or a GitHub read
/// that failed — never alerts.
pub(crate) fn closed_bound_verdict(landing: &Landing, has_record: bool) -> ClosedBoundVerdict {
    match (landing, has_record) {
        (Landing::Landed { .. }, true) => ClosedBoundVerdict::ClosedClean,
        (Landing::Landed { .. }, false) => ClosedBoundVerdict::ClosedMissingRecord,
        _ => ClosedBoundVerdict::Open,
    }
}

/// Build the throat POST body for one missing record. Plain — no
/// needsDrew object — under the same fixed wire contract the
/// aggregator already owns: source "fleet-queue", key
/// "closed-bound:<task_id>", warning severity. The body names the
/// task status and the guard hint: present the task's completion
/// record, or reopen the issue.
pub fn build_closed_bound_alert(task_id: i64, repo: &str, issue: i64, status: &str) -> Value {
    json!({
        "source": "fleet-queue",
        "key": format!("closed-bound:{task_id}"),
        "severity": "warning",
        "title": format!(
            "Bound task {task_id}: issue {repo}#{issue} closed without a completion record"
        ),
        "body": format!(
            "Task {task_id} is {status} on {repo}#{issue}, but GitHub reports the issue \
             CLOSED and the closure log holds no fleet-closure/v1 close record for task \
             {task_id}. Present the task's completion record, or reopen the issue."
        ),
    })
}

/// The closure log the writer appends to: `FLEET_CLOSURE_LOG` when
/// set and non-empty, else the fleet state dir's
/// `closure-records.jsonl` (the `fleet gh` writer's default). A
/// missing file is an empty log; any other read failure surfaces
/// so the sweep reports it instead of alerting on unread evidence.
fn closure_log_text() -> Result<String, TErr> {
    let path = std::env::var("FLEET_CLOSURE_LOG")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fleet_config::Config::state_dir().join("closure-records.jsonl"));
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!(
            "closed-bound: cannot read closure log {}: {e}",
            path.display()
        )),
    }
}

/// Resolve the PM aggregator's alerts-intake token by name through
/// fleet-secrets — the same path the CLI half's
/// `resolve_alerts_intake_token` takes (the default chain over the
/// loaded fleet config). The value is used for the one POST and
/// dropped, never printed, never filed.
fn resolve_alerts_intake_token() -> Result<String, String> {
    let config = fleet_config::Config::load().unwrap_or_default();
    fleet_secrets::Resolver::default_chain(&config)
        .resolve(crate::needs_drew::ALERTS_INTAKE_TOKEN_SECRET)
        .map(|v| v.expose().to_string())
        .map_err(|e| e.to_string())
}

/// The closed-bound detective pass (#9177): every claimed|parked
/// row carrying the #9046 session stamp, its issue read on GitHub
/// once per pass (a per-pass repo#issue cache beside the probe's
/// own, so one pass never re-reads an issue), crossed with the
/// closure log read once per pass. A landed issue with no
/// completion record for its bound task appends one
/// `closed_bound_alert` event and POSTs one throat alert; a
/// failed post (token or transport) counts `alert_failures` and
/// never fails the pass — the sweep wiring reports even a store
/// failure inside the summary and continues the cycle.
pub fn run_closed_bound_checks(t: &TursoBackend, limit: i64) -> Result<Value, TErr> {
    run_closed_bound_checks_with(t, limit, &|body| {
        resolve_alerts_intake_token().and_then(|token| {
            crate::needs_drew::post_plain_alert_to(&crate::needs_drew::alerts_url(), &token, body)
        })
    })
}

/// [`run_closed_bound_checks`] with the throat post injected: the
/// production half above wires the real throat; tests inject a
/// capture (or a failure) so no pass under test touches the
/// network or the secrets chain.
pub(crate) fn run_closed_bound_checks_with(
    t: &TursoBackend,
    limit: i64,
    post_alert: &dyn Fn(&Value) -> Result<(), String>,
) -> Result<Value, TErr> {
    let limit = limit.clamp(1, 1000);
    let rows = t.query_objs(
        &format!(
            "SELECT {TASK_COLS} FROM tasks WHERE status IN ('claimed','parked') \
             ORDER BY id LIMIT {limit}"
        ),
        &[],
        &TASK_JSON_COLS,
    )?;
    let (bound, skipped_no_issue) = bound_issues(&rows);
    let mut summary = json!({
        "checked": 0, "closed": 0, "missing_record": 0,
        "alerts": 0, "alert_failures": 0,
        "probe_unavailable": 0, "skipped_no_issue": skipped_no_issue,
    });
    // No probe, no GitHub work: the pass is a clean Ok with every
    // bound issue counted unavailable — a missing probe never
    // alerts and never fails the sweep.
    let Some(probe) = t.gh_probe() else {
        summary["probe_unavailable"] = json!(bound.len() as i64);
        return Ok(summary);
    };
    let log_text = closure_log_text()?;
    let mut cache: HashMap<String, Landing> = HashMap::new();
    let mut checked = 0i64;
    for b in &bound {
        let key = format!("{}#{}", b.repo, b.issue);
        let repo = b.repo.clone();
        let issue = b.issue;
        let landing = cache
            .entry(key)
            .or_insert_with(|| probe.probe_landing(LinkKind::Issue, &repo, issue))
            .clone();
        checked += 1;
        match closed_bound_verdict(
            &landing,
            has_completion_record(&log_text, b.task_id, b.issue),
        ) {
            ClosedBoundVerdict::Open | ClosedBoundVerdict::ClosedClean => {
                if matches!(landing, Landing::Landed { .. }) {
                    summary["closed"] = json!(summary["closed"].as_i64().unwrap_or(0) + 1);
                }
            }
            ClosedBoundVerdict::ClosedMissingRecord => {
                summary["closed"] = json!(summary["closed"].as_i64().unwrap_or(0) + 1);
                summary["missing_record"] =
                    json!(summary["missing_record"].as_i64().unwrap_or(0) + 1);
                event(
                    t,
                    Some("closed-bound-checker"),
                    Some(b.task_id),
                    "closed_bound_alert",
                    &json!({
                        "repo": b.repo, "issue": b.issue,
                        "task_id": b.task_id, "status": b.status,
                        "via": "sweep",
                    }),
                )?;
                match post_alert(&build_closed_bound_alert(
                    b.task_id, &b.repo, b.issue, &b.status,
                )) {
                    Ok(()) => {
                        summary["alerts"] = json!(summary["alerts"].as_i64().unwrap_or(0) + 1);
                    }
                    Err(_) => {
                        summary["alert_failures"] =
                            json!(summary["alert_failures"].as_i64().unwrap_or(0) + 1);
                    }
                }
            }
        }
    }
    summary["checked"] = json!(checked);
    Ok(summary)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn bound_row(id: i64, status: &str, session: Option<&str>, payload: Value, key: &str) -> Value {
        let mut row = json!({
            "id": id, "status": status,
            "payload": payload, "dedupe_key": key,
        });
        match session {
            Some(s) => row["claim_session_id"] = json!(s),
            None => row["claim_session_id"] = Value::Null,
        }
        row
    }

    fn issue_payload(repo: &str, issue: i64) -> Value {
        json!({"repo": repo, "issue": {"number": issue}})
    }

    #[test]
    fn bound_issues_keeps_only_session_stamped_claimed_and_parked() {
        // Oracle: the incident definition, as the guard enforces
        // it — claimed|parked AND the #9046 session stamp. A row
        // without the stamp is not bound even in an in-progress
        // status; queued/done rows never are; payload.issue.number
        // wins and the repo-health dedupe key covers packets that
        // carry the issue only there.
        let rows = vec![
            bound_row(1, "claimed", Some("ses-1"), issue_payload("o/r", 7), ""),
            bound_row(2, "parked", Some("ses-2"), issue_payload("o/r", 8), ""),
            // In-progress but unstamped: the guard would not have
            // owned the close, so the detective does not own it.
            bound_row(3, "claimed", None, issue_payload("o/r", 9), ""),
            bound_row(4, "parked", Some(""), issue_payload("o/r", 10), ""),
            bound_row(5, "queued", Some("ses-5"), issue_payload("o/r", 11), ""),
            bound_row(6, "done", Some("ses-6"), issue_payload("o/r", 12), ""),
            bound_row(7, "blocked", Some("ses-7"), issue_payload("o/r", 13), ""),
            // No payload issue, but the dedupe key names it under
            // the packet's repo.
            bound_row(
                8,
                "claimed",
                Some("ses-8"),
                json!({"repo": "developing-today/code"}),
                "repo-health:v1:fix:developing-today/code:issue:9155",
            ),
            // Stamped and in-progress, but no resolvable issue.
            bound_row(9, "claimed", Some("ses-9"), json!({"repo": "o/r"}), ""),
        ];
        let (bound, skipped) = bound_issues(&rows);
        assert_eq!(
            bound,
            vec![
                BoundIssue {
                    task_id: 1,
                    repo: "o/r".to_string(),
                    issue: 7,
                    status: "claimed".to_string(),
                },
                BoundIssue {
                    task_id: 2,
                    repo: "o/r".to_string(),
                    issue: 8,
                    status: "parked".to_string(),
                },
                BoundIssue {
                    task_id: 8,
                    repo: "developing-today/code".to_string(),
                    issue: 9155,
                    status: "claimed".to_string(),
                },
            ]
        );
        assert_eq!(skipped, 1);
    }

    #[test]
    fn has_completion_record_matches_the_v1_contract() {
        // Oracle: the merged fleet-closure/v1 JSONL (PR #11440).
        // action close + item.number + binding.task_id (string
        // compare) + kind issue-when-present; malformed lines are
        // not evidence and never fail the read.
        let good = json!({
            "schema": "fleet-closure/v1", "action": "close",
            "item": {"kind": "issue", "number": 9177, "repo": "o/r"},
            "reason": "done", "at_unix": 1,
            "binding": {"run_id": null, "worker": "w", "lane": "l",
                        "task_id": "20372", "session_id": "s", "user": "u"},
        })
        .to_string();
        assert!(has_completion_record(&good, 20372, 9177));
        // A task-id mismatch is another task's record.
        assert!(!has_completion_record(&good, 20373, 9177));
        // An issue mismatch is another issue's record.
        assert!(!has_completion_record(&good, 20372, 9178));
        // A reopen is not a completion.
        let reopen = good.replace("\"action\":\"close\"", "\"action\":\"reopen\"");
        assert!(!has_completion_record(&reopen, 20372, 9177));
        // A PR close is not an issue completion ...
        let pr = good.replace("\"kind\":\"issue\"", "\"kind\":\"pr\"");
        assert!(!has_completion_record(&pr, 20372, 9177));
        // ... but a record that names no kind is the writer's own
        // shape and still counts.
        let no_kind = good.replace("\"kind\":\"issue\",", "");
        assert!(has_completion_record(&no_kind, 20372, 9177));
        // Malformed lines are ignored, not fatal: garbage around
        // the one good line still finds it, garbage alone finds
        // nothing.
        let mixed = format!("not json\n{{\"action\": 1}}\n{good}\n[broken");
        assert!(has_completion_record(&mixed, 20372, 9177));
        assert!(!has_completion_record(
            "not json\n{\"action\": 1}\n",
            20372,
            9177
        ));
        assert!(!has_completion_record("", 20372, 9177));
    }

    #[test]
    fn verdict_maps_landing_and_record() {
        // Oracle: Landed+record is clean, Landed without a record
        // is the alert, and an open issue — or a GitHub read that
        // failed — never alerts however the log reads.
        let landed = Landing::Landed {
            state: "closed".to_string(),
            at: "2026-11-13T00:00:00Z".to_string(),
        };
        assert_eq!(
            closed_bound_verdict(&landed, true),
            ClosedBoundVerdict::ClosedClean
        );
        assert_eq!(
            closed_bound_verdict(&landed, false),
            ClosedBoundVerdict::ClosedMissingRecord
        );
        assert_eq!(
            closed_bound_verdict(&Landing::Open, false),
            ClosedBoundVerdict::Open
        );
        assert_eq!(
            closed_bound_verdict(&Landing::Open, true),
            ClosedBoundVerdict::Open
        );
        assert_eq!(
            closed_bound_verdict(&Landing::Unknown("github http 502".to_string()), false),
            ClosedBoundVerdict::Open
        );
    }

    #[test]
    fn alert_body_pins_the_plain_throat_shape() {
        // Oracle: the fixed wire contract minus needsDrew — the
        // aggregator dedupes on (source, key), so the key format
        // is the idempotency story.
        let body = build_closed_bound_alert(20372, "o/r", 9177, "claimed");
        assert_eq!(body["source"], json!("fleet-queue"));
        assert_eq!(body["key"], json!("closed-bound:20372"));
        assert_eq!(body["severity"], json!("warning"));
        assert_eq!(
            body["title"],
            json!("Bound task 20372: issue o/r#9177 closed without a completion record")
        );
        assert!(body["body"].as_str().unwrap_or("").contains("claimed"));
        assert!(body.get("needsDrew").is_none());
    }

    struct ClosedFixture {
        answer: Landing,
    }

    impl DepProbe for ClosedFixture {
        fn probe_issue(&self, _: &str, _: i64) -> DepObs {
            DepObs::Failed("unused".into())
        }
        fn probe_pr(&self, _: &str, _: i64) -> DepObs {
            DepObs::Failed("unused".into())
        }
        fn probe_ci(&self, _: &str, _: i64) -> DepObs {
            DepObs::Failed("unused".into())
        }
        fn probe_landing(&self, _: LinkKind, _: &str, _: i64) -> Landing {
            self.answer.clone()
        }
    }

    /// One pipeline request / one canned reply, in order; extra
    /// requests get a bare ok. Captures every request body.
    fn stub_pipeline_seq(
        replies: Vec<String>,
    ) -> (
        String,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<std::collections::VecDeque<String>>>,
    ) {
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
        (format!("http://{addr}"), captured, queue)
    }

    fn cell_text(v: &str) -> Value {
        json!({"type": "text", "value": v})
    }

    fn cell_int(v: i64) -> Value {
        json!({"type": "integer", "value": v.to_string()})
    }

    /// A SELECT reply carrying one bound row for task 20372 on
    /// issue 9177 (claimed, session-stamped), in the columns the
    /// driver reads.
    fn select_one_bound_row() -> String {
        let payload = issue_payload("developing-today/code", 9177).to_string();
        json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": [{"name": "id"}, {"name": "status"},
                         {"name": "payload"}, {"name": "dedupe_key"},
                         {"name": "claim_session_id"}],
                "rows": [[cell_int(20372), cell_text("claimed"),
                          cell_text(&payload),
                          cell_text("repo-health:v1:fix:developing-today/code:issue:9177"),
                          cell_text("ses-marker")]],
                "affected_row_count": 0,
                "last_insert_rowid": null}}},
            {"type": "ok", "response": {"type": "close"}}
        ]})
        .to_string()
    }

    fn insert_ok() -> String {
        json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": [], "rows": [],
                "affected_row_count": 1,
                "last_insert_rowid": "9"}}},
            {"type": "ok", "response": {"type": "close"}}
        ]})
        .to_string()
    }

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &std::path::Path) -> (MutexGuard<'static, ()>, EnvGuard) {
            let guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let prev = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            (guard, EnvGuard { key, prev })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    use std::sync::MutexGuard;

    fn temp_closure_log(name: &str, contents: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fleet-closed-bound-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("closure-records.jsonl");
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn landed_closed() -> Landing {
        Landing::Landed {
            state: "closed".to_string(),
            at: "2026-11-13T00:08:00Z".to_string(),
        }
    }

    #[test]
    fn driver_alerts_on_a_closed_issue_with_no_record() {
        // Oracle: the incident replayed end to end — a stamped
        // claimed task whose issue the probe reports closed, with
        // an empty closure log, appends one closed_bound_alert
        // event, calls the throat once with the pinned body, and
        // reports missing_record == 1.
        let (_lock, _env) = EnvGuard::set("FLEET_CLOSURE_LOG", &temp_closure_log("no-record", ""));
        let (url, captured, _queue) = stub_pipeline_seq(vec![select_one_bound_row(), insert_ok()]);
        let mut backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        backend.set_gh_probe(Arc::new(ClosedFixture {
            answer: landed_closed(),
        }));
        let posted = Arc::new(Mutex::new(Vec::new()));
        let sink = posted.clone();
        let out = run_closed_bound_checks_with(&backend, 200, &move |body| {
            sink.lock().unwrap().push(body.clone());
            Ok(())
        })
        .expect("the pass reports inside its summary, not via Err");
        assert_eq!(out["checked"], json!(1), "{out}");
        assert_eq!(out["closed"], json!(1), "{out}");
        assert_eq!(out["missing_record"], json!(1), "{out}");
        assert_eq!(out["alerts"], json!(1), "{out}");
        assert_eq!(out["alert_failures"], json!(0), "{out}");
        assert_eq!(out["skipped_no_issue"], json!(0), "{out}");
        let bodies = posted.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["key"], json!("closed-bound:20372"));
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 2, "{reqs:?}");
        assert!(reqs[1].contains("closed_bound_alert"), "{reqs:?}");
        assert!(reqs[1].contains("20372"), "{reqs:?}");
    }

    #[test]
    fn driver_stays_quiet_when_the_record_exists() {
        // Oracle: the same pass, but the temp closure log holds
        // the bound task's own fleet-closure/v1 close record —
        // no event, no throat call, missing_record == 0.
        let record = json!({
            "schema": "fleet-closure/v1", "action": "close",
            "item": {"kind": "issue", "number": 9177,
                     "repo": "developing-today/code"},
            "reason": "done", "at_unix": 1,
            "binding": {"run_id": null, "worker": "w", "lane": "l",
                        "task_id": "20372", "session_id": "s",
                        "user": "u"},
        })
        .to_string();
        let (_lock, _env) = EnvGuard::set(
            "FLEET_CLOSURE_LOG",
            &temp_closure_log("with-record", &format!("{record}\n")),
        );
        let (url, captured, _queue) = stub_pipeline_seq(vec![select_one_bound_row()]);
        let mut backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        backend.set_gh_probe(Arc::new(ClosedFixture {
            answer: landed_closed(),
        }));
        let posted = Arc::new(Mutex::new(0i64));
        let sink = posted.clone();
        let out = run_closed_bound_checks_with(&backend, 200, &move |_| {
            *sink.lock().unwrap() += 1;
            Ok(())
        })
        .expect("a clean pass never errs");
        assert_eq!(out["checked"], json!(1), "{out}");
        assert_eq!(out["closed"], json!(1), "{out}");
        assert_eq!(out["missing_record"], json!(0), "{out}");
        assert_eq!(out["alerts"], json!(0), "{out}");
        assert_eq!(*posted.lock().unwrap(), 0);
        // Only the SELECT went out: no event INSERT followed.
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[test]
    fn driver_counts_a_failed_post_without_failing() {
        // Oracle: the throat half may fail (token, transport) —
        // the event is already the durable record, so the pass
        // counts alert_failures and stays Ok.
        let (_lock, _env) = EnvGuard::set("FLEET_CLOSURE_LOG", &temp_closure_log("post-fail", ""));
        let (url, captured, _queue) = stub_pipeline_seq(vec![select_one_bound_row(), insert_ok()]);
        let mut backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        backend.set_gh_probe(Arc::new(ClosedFixture {
            answer: landed_closed(),
        }));
        let out = run_closed_bound_checks_with(&backend, 200, &|_| {
            Err("cannot connect to alerts throat".to_string())
        })
        .expect("a failed post never fails the pass");
        assert_eq!(out["missing_record"], json!(1), "{out}");
        assert_eq!(out["alerts"], json!(0), "{out}");
        assert_eq!(out["alert_failures"], json!(1), "{out}");
        assert!(captured.lock().unwrap()[1].contains("closed_bound_alert"));
    }

    #[test]
    fn driver_without_a_probe_counts_unavailable_and_stays_ok() {
        // Oracle: fail-open — with no GitHub probe installed the
        // pass does no GitHub work, alerts nothing, and returns a
        // clean Ok with every bound issue counted unavailable.
        let (_lock, _env) = EnvGuard::set("FLEET_CLOSURE_LOG", &temp_closure_log("no-probe", ""));
        let (url, captured, _queue) = stub_pipeline_seq(vec![select_one_bound_row()]);
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let out = run_closed_bound_checks_with(&backend, 200, &|_| {
            panic!("no probe means no alert attempt")
        })
        .expect("a probeless pass is a clean Ok");
        assert_eq!(out["checked"], json!(0), "{out}");
        assert_eq!(out["missing_record"], json!(0), "{out}");
        assert_eq!(out["alerts"], json!(0), "{out}");
        assert_eq!(out["probe_unavailable"], json!(1), "{out}");
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[test]
    fn sweep_wiring_runs_the_detective_beside_the_recheckers() {
        // Oracle: the ride shape the dep (#8276) and quarantine
        // (#10648) recheckers established — opt-out flag defaulting
        // on, its own limit key, summary under out["closed_bound"],
        // and a cycle that found missing records lands in the
        // sweep event.
        let src = include_str!("core.rs");
        let flat = src.split_whitespace().collect::<Vec<_>>().join(" ");
        // treefmt may lay the call out on one line or many (#13221:
        // 5a70b6bdb97 collapsed it), so assert the pieces, not the
        // spacing between "(" and "t".
        assert!(flat.contains("run_closed_bound_checks("));
        assert!(flat.contains("t, req_i64(req, \"closed_bound_limit\")"));
        assert!(src.contains("req_bool(req, \"closed_bound\")"));
        assert!(src.contains("out[\"closed_bound\"]"));
        assert!(src.contains("closed_bound_missing > 0"));
    }
}
