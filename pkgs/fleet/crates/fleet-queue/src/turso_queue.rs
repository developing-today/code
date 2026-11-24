//! queue-broker Turso backend — the queue core on libSQL (#6741).
//!
//! The queue core (tasks / leases / lanes / workers / checkpoints /
//! repo_task_lane_ledger / events) on a dedicated Turso database
//! (`fleet-queue`, org dezren39, group great-star-lord; rw token in
//! BWS as `turso.fleet-queue-rw-token`) behind the broker protocol.
//! It is the queue broker's only backend. URL:
//! `QUEUE_BROKER_TURSO_URL` (default the fleet-queue HTTPS URL).
//! Token: `QUEUE_BROKER_TURSO_TOKEN_FILE`, else the secrets broker
//! by the name above.
//!
//! Schema + semantics are reconstructed from the recovered live PG
//! function bodies in pm-stack/opencode-claimant/migration-0021.sql
//! .. migration-0024.sql (claim_repo_task, fail_repo_task, park_task,
//! expire_leases, assign_repo_task_lane,
//! bind_repo_task_lane_session) plus the broker/client contract in
//! this crate for the verbs whose PG bodies were not recovered
//! (enqueue dedupe, heartbeat, checkpoint, block, complete, cancel,
//! record execution, health aggregation). Every deviation from the
//! PG behavior is listed in the cutover runbook.
//!
//! agent.events (#5690): the PG stream's queue-transition writers
//! are this backend's `event()` calls (same dispatch, same database
//! as the task write — the closest libSQL gets to the PG RPCs'
//! single-transaction coupling; see doc/fleet/agent-events-turso.md
//! for the full writer/reader inventory and the coupling verdict).
//! The non-queue writers (github-ci webhooks, fleet-fetch) and the
//! stream's consumers get broker verbs here: /events-append,
//! /events-seen (the event_seen ready-condition primitive),
//! /events-count (observability + health), /events-prune (the
//! prune_events port).
//!
//! Atomicity: libSQL is single-writer. A claim is ONE conditional
//! UPDATE whose WHERE clause re-checks claimability (status in
//! queued/parked with a due not_before, attempts left) against the
//! row a subquery picks — the same shape as PG's FOR UPDATE SKIP
//! LOCKED pick, with the write serialization libSQL already
//! guarantees. A competing claim after the first lands matches no
//! row and returns `{row: null}` (client exit 5), exactly the PG
//! envelope.

use serde_json::{Map, Value, json};

use crate::validate::{task_from_packet, validate_packet};

mod actions_exec;
mod admin;
mod claim;
mod closed_bound;
mod core;
mod label_enrich;
mod lifecycle;
mod placement_coord;
mod placement_sessions;
mod repair;
mod stacks_outbox;
mod staleness;
mod work_identity;

pub use actions_exec::*;
pub use admin::*;
pub use claim::*;
pub use closed_bound::*;
pub use core::*;
pub use label_enrich::*;
pub use lifecycle::*;
pub use placement_coord::*;
pub use placement_sessions::*;
pub use repair::*;
pub use stacks_outbox::*;
pub use staleness::*;
pub use work_identity::*;

#[cfg(test)]
mod tests {

    /// The full backend source, concatenated across the #8589
    /// module split (parent file last, so the cfg(test) marker
    /// still terminates the production text). Each file is
    /// truncated at its own first `#[cfg(test)]` so inline test
    /// modules (staleness, work_identity, closed_bound, lifecycle
    /// reprioritize_tests) never hide a later file's production
    /// text from the source-content oracles.
    fn test_source() -> String {
        [
            include_str!("turso_queue/core.rs"),
            include_str!("turso_queue/claim.rs"),
            include_str!("turso_queue/label_enrich.rs"),
            include_str!("turso_queue/lifecycle.rs"),
            include_str!("turso_queue/placement_coord.rs"),
            include_str!("turso_queue/admin.rs"),
            include_str!("turso_queue/stacks_outbox.rs"),
            include_str!("turso_queue/staleness.rs"),
            include_str!("turso_queue/work_identity.rs"),
            include_str!("turso_queue/closed_bound.rs"),
            include_str!("turso_queue.rs"),
        ]
        .iter()
        .map(|s| s.split("#[cfg(test)]").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .concat()
    }
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn budget_gate_3418_predicate_shape() {
        // #3418 oracle (broker level): the default pick predicate
        // budget-gates BOTH the queued and the parked branches,
        // gives capacity waits a budget bypass of their timer, and
        // carries exactly four placeholders ahead of the
        // platform-limit clause — the two branch `now`s, then the
        // claimer's declared remaining budget twice (bypass gate,
        // outer gate). claim_pick_once pushes its arguments in
        // exactly that order for both the pick SELECT and the
        // claim UPDATE re-check, which embeds the same predicate.
        let pred = default_pick_predicate();
        assert_eq!(pred.matches('?').count(), 4, "{pred}");
        assert_eq!(pred.matches("COALESCE(?, -1)").count(), 2, "{pred}");
        assert!(pred.contains("wait_kind = 'capacity'"), "{pred}");
        assert!(
            pred.contains("json_extract(payload, '$.required_budget_minutes')"),
            "{pred}"
        );
        assert!(claim_update_sql(&pred).contains(&pred));
        assert!(claim_update_sql(&pred).contains("required_budget_minutes = NULL"));
        // An undeclared budget (NULL) fails the gate for a
        // requirement-bearing row and passes a requirement-free
        // one — that is the COALESCE(?, -1) shape asserted above:
        // -1 >= any positive requirement is false, and the
        // `IS NULL` arm passes rows with no requirement.
    }

    #[test]
    fn park_required_budget_3418_defaults() {
        // #3418: explicit wins; a capacity wait defaults from the
        // packet timebox; every other park records no requirement.
        let payload = json!({"timebox_minutes": 120});
        let capacity = json!({"kind": "capacity"});
        let ci = json!({"kind": "ci", "pr": 1});
        assert_eq!(park_required_budget(None, &capacity, &payload), Some(120));
        assert_eq!(park_required_budget(None, &ci, &payload), None);
        assert_eq!(
            park_required_budget(Some(30), &capacity, &payload),
            Some(30)
        );
        assert_eq!(park_required_budget(Some(30), &ci, &payload), Some(30));
        assert_eq!(park_required_budget(None, &capacity, &json!({})), None);
        assert_eq!(park_required_budget(Some(0), &capacity, &payload), None);
    }

    #[test]
    fn enqueue_insert_stamps_required_budget_column_3418() {
        // #3418 (#4712) oracle: the live enqueue INSERT persists the
        // effective requirement in the queryable column (NULL when
        // the packet declares none), and the triage follow-on
        // INSERT carries the cloned payload's requirement the same
        // way. Both inserts must name the column exactly once.
        let src = &test_source();
        // Both INSERT sites name the column exactly once: the main
        // enqueue INSERT and the triage follow-on INSERT below it
        // (matched here by its distinct column list).
        assert!(
            src.contains("status, not_before, max_attempts, required_budget_minutes, platform"),
            "main enqueue INSERT stamps the column"
        );
        assert!(
            src.contains("max_attempts, required_budget_minutes, progress, result,"),
            "triage follow-on INSERT stamps the column"
        );
    }

    #[test]
    fn budget_skip_summary_query_shape_3418() {
        // #3418 (#4712) oracle: the impossible-fit summary counts
        // otherwise-eligible rows whose effective requirement
        // exceeds the claimer's declared budget, and reports the
        // max. Placeholders: the two liveness `now`s, then the
        // declared budget once. No `IS NULL` pass arm — a NULL
        // requirement is not a skip.
        let sql = budget_skip_sql(" AND queue = ?");
        assert_eq!(sql.matches('?').count(), 4, "{sql}");
        assert!(sql.contains("COUNT(*) AS n"), "{sql}");
        assert!(sql.contains("MAX("), "{sql}");
        assert!(sql.contains("wait_kind = 'capacity'"), "{sql}");
        assert!(
            sql.contains("json_extract(payload, '$.required_budget_minutes')"),
            "{sql}"
        );
        assert!(!sql.contains("IS NULL OR COALESCE"), "{sql}");
        assert!(sql.ends_with(" AND queue = ?"), "{sql}");
    }

    #[test]
    fn no_claim_counts_query_shape_8154() {
        // #8154 oracle (follow-on #2937 acceptance 4): the no-claim
        // counts aggregate splits the searched scope by reason —
        // queued, at-cap (attempts >= max_attempts, the #6528
        // definition), not-yet-due, parked-due — over payload-bearing
        // rows only. Placeholders: two `now`s, then the scope's.
        let sql = no_claim_counts_sql(" AND queue = ?");
        assert_eq!(sql.matches('?').count(), 3, "{sql}");
        assert!(sql.contains("AS queued"), "{sql}");
        assert!(sql.contains("AS at_cap_queued"), "{sql}");
        assert!(sql.contains("attempts >= max_attempts"), "{sql}");
        assert!(sql.contains("AS queued_not_yet_due"), "{sql}");
        assert!(sql.contains("not_before > ?"), "{sql}");
        assert!(sql.contains("AS parked_due"), "{sql}");
        assert!(sql.contains("wait_kind = 'capacity'"), "{sql}");
        assert!(sql.ends_with(" AND queue = ?"), "{sql}");
    }

    #[test]
    fn task_creation_defaults_max_attempts_1000_per_8973() {
        // #8973 oracle: every task-creation default in this crate is
        // 1000 (Drew order 2026-11-12) - the tasks schema default
        // and the enqueue insert fallback (which reads the one
        // TASK_DEFAULT_MAX_ATTEMPTS constant in validate.rs; the
        // task_from_packet default is covered in validate tests).
        // #10799: the four former hard-coded-4 copies of the task
        // default (the expire-leases, revive, and fail-disposition
        // read fallbacks, and the triage follow-on insert fallback)
        // are unified on the same constant, so no stale copy can
        // drift back. The creation_outbox retry cap is a different
        // domain (GitHub-filing delivery attempts, #3706 bounded
        // pilot - it never becomes a task row value) and
        // intentionally stays at its own DEFAULT_MAX_ATTEMPTS.
        let src = &test_source();
        assert!(
            src.contains("max_attempts INTEGER NOT NULL DEFAULT 1000"),
            "tasks schema default is 1000"
        );
        assert!(
            src.contains("crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64"),
            "enqueue insert fallback uses the shared task default"
        );
        // The stale-default needle is built with format! so this
        // test file does not itself contain the literal it bans.
        let stale = format!("max_attempts INTEGER NOT NULL DEFAULT {}", 4);
        assert!(!src.contains(&stale), "no stale tasks schema default");
        // #10799: the stale fallback needle, same construction.
        let stale_fallback = format!("unwrap_or({})", 4);
        assert!(
            !src.contains(&stale_fallback),
            "no stale max_attempts fallback copies"
        );
        // The shared constant backs every site: the enqueue insert
        // fallback plus the four unified #10799 sites.
        assert!(
            src.matches("crate::validate::TASK_DEFAULT_MAX_ATTEMPTS as i64")
                .count()
                >= 5,
            "every max_attempts fallback reads the shared task default"
        );
    }

    #[test]
    fn enqueue_dry_run_defaults_max_attempts_1000_per_8973() {
        // #8973 acceptance, enqueue path: a valid packet that omits
        // max_attempts in BOTH copies enqueues with the default
        // applied. Dry-run returns the exact row shape the insert
        // consumes, so this is a readback of the creation default
        // without a live store; the insert fallback and the schema
        // default are covered by the source-oracle test above.
        let backend = TursoBackend::new(
            "http://127.0.0.1:0".to_string(),
            "test".to_string(),
            ureq::agent(),
        );
        let packet = json!({
            "title": "probe", "priority": 30,
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:8973",
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "p",
                "priority_reason": "r",
                "evidence": ["e"],
                "acceptance": ["a"],
                "tests": ["t"],
                "context_refs": [],
                "scope": {"include": ["x"], "exclude": ["y"] },
                "risk": {"level": "low", "flags": [] },
                "timebox_minutes": 45,
                "issue": {"number": 8973 },
                "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test" }
            }
        });
        let req = json!({
            "queue": "repo-health-code-pilot",
            "packet": packet,
            "dry_run": true
        });
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["task"]["max_attempts"], 1000, "{out}");
    }

    /// A one-shot stub of the Turso pipeline endpoint for the
    /// #12744 tests: captures the first request body it receives
    /// and answers it with the canned pipeline response. The
    /// thread is detached — a code path that never queries leaves
    /// it parked on accept, which is exactly the fail-before shape
    /// (the assertions on the reply and the capture then fail).
    fn stub_pipeline(
        response_body: String,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
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
            reader.read_exact(&mut body).unwrap();
            sink.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&body).to_string());
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    /// A canned pipeline reply carrying one github_mirror_issues
    /// row whose labels column is `labels_json` (or no rows when
    /// None).
    fn mirror_pipeline_reply(labels_json: Option<&str>) -> String {
        let rows = match labels_json {
            Some(l) => json!([[{"type": "text", "value": l}]]),
            None => json!([]),
        };
        json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": [{"name": "labels"}],
                "rows": rows,
                "affected_row_count": 0,
                "last_insert_rowid": null}}},
            {"type": "ok", "response": {"type": "close"}}
        ]})
        .to_string()
    }

    /// A valid repo-task/v1 packet for an issue, in the shape the
    /// #8973 dry-run test established, with a stated priority and
    /// NO payload.labels — the production shape #12744 is about.
    fn packet_12744(issue: i64, stated: i64) -> Value {
        json!({
            "title": "probe", "priority": stated,
            "dedupe_key": format!("repo-health:v1:fix:developing-today/code:issue:{issue}"),
            "payload": {
                "schema": "repo-task/v1",
                "kind": "small_fix",
                "repo": "developing-today/code",
                "problem": "p",
                "priority_reason": "r",
                "evidence": ["e"],
                "acceptance": ["a"],
                "tests": ["t"],
                "context_refs": [],
                "scope": {"include": ["x"], "exclude": ["y"] },
                "risk": {"level": "low", "flags": [] },
                "timebox_minutes": 45,
                "issue": {"number": issue },
                "dedupe": {"checked_at": "2026-11-14T00:00:00Z", "basis": "test" }
            }
        })
    }

    #[test]
    fn enqueue_mirror_labels_select_band_12744() {
        // #12744 acceptance oracle, at the verb: a producer-path
        // enqueue for an expedite-labelled issue whose packet
        // carries no labels stores at the ratified band 15 with
        // the mirror's labels on the payload — no manual update.
        // The mirror row is the stub's; the band is Drew's.
        let (url, captured) =
            stub_pipeline(mirror_pipeline_reply(Some("[\"expedite\",\"kind/bug\"]")));
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let req = json!({
            "queue": "repo-health-code-pilot",
            "packet": packet_12744(12228, 30),
            "dry_run": true
        });
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["task"]["priority"], 15, "{out}");
        assert_eq!(
            out["task"]["payload"]["labels"],
            json!(["expedite", "kind/bug"]),
            "{out}"
        );
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        assert!(bodies[0].contains("github_mirror_issues"), "{bodies:?}");
        assert!(bodies[0].contains("developing-today/code"), "{bodies:?}");
    }

    #[test]
    fn enqueue_producer_labels_stand_over_mirror_12744() {
        // A packet that carries its own labels array keeps it —
        // the mirror is a fill, never an override. The backend is
        // unreachable, so any lookup attempt would surface as an
        // enrichment warning on the reply; none may appear.
        let backend = TursoBackend::new(
            "http://127.0.0.1:0".to_string(),
            "test".to_string(),
            ureq::agent(),
        );
        let mut packet = packet_12744(12228, 90);
        packet["payload"]["labels"] = json!(["next-up"]);
        let req = json!({
            "queue": "repo-health-code-pilot",
            "packet": packet,
            "dry_run": true
        });
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["task"]["priority"], 25, "{out}");
        assert_eq!(
            out["task"]["payload"]["labels"],
            json!(["next-up"]),
            "{out}"
        );
        let warnings = out["warnings"].to_string();
        assert!(!warnings.contains("mirror enrichment"), "{out}");
    }

    #[test]
    fn enqueue_mirror_miss_keeps_stated_priority_12744() {
        // No mirror row for the issue: the packet stands exactly
        // as producers have always filed it — stated priority, no
        // labels key written.
        let (url, _captured) = stub_pipeline(mirror_pipeline_reply(None));
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let req = json!({
            "queue": "repo-health-code-pilot",
            "packet": packet_12744(424242, 30),
            "dry_run": true
        });
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["task"]["priority"], 30, "{out}");
        assert!(out["task"]["payload"].get("labels").is_none(), "{out}");
    }

    #[test]
    fn enqueue_mirror_error_warns_and_proceeds_12744() {
        // A store without the mirror tables (a fresh or test DB):
        // the lookup fails, the enqueue warns and proceeds — the
        // mirror never gates work.
        let (url, _captured) = stub_pipeline(
            json!({"results": [
                {"type": "error", "error": {"message": "no such table: github_mirror_issues"}}
            ]})
            .to_string(),
        );
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let req = json!({
            "queue": "repo-health-code-pilot",
            "packet": packet_12744(12228, 30),
            "dry_run": true
        });
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["task"]["priority"], 30, "{out}");
        assert!(
            out["warnings"]
                .to_string()
                .contains("mirror enrichment skipped"),
            "{out}"
        );
    }

    // ---- #13091: the broker must not re-serve parked or completed
    // work. The incidents (2026-11-23): a parked task whose recorded
    // term was a condition/stand-down was timer-served to lanes that
    // could only re-park it, and a fresh filing for already-completed
    // work created a new servable task that a lane completed a
    // second time. The tests below pin both halves at the verb/SQL
    // surface the serve path actually runs.

    /// A content-routed stub of the Turso pipeline endpoint for the
    /// #13091 tests: each request is answered by the first route
    /// whose needle appears in its body; unmatched requests get a
    /// bare ok execute reply. Captures every request body.
    fn stub_pipeline_routed_13091(
        routes: Vec<(&'static str, String)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        let routes = std::sync::Arc::new(routes);
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
                let body = String::from_utf8_lossy(&body).to_string();
                sink.lock().unwrap().push(body.clone());
                let resp = routes
                    .iter()
                    .find(|(needle, _)| body.contains(needle))
                    .map(|(_, reply)| reply.clone())
                    .unwrap_or_else(|| {
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

    /// A SELECT reply carrying full task rows (TASK_COLS order).
    fn task_select_reply_13091(rows: Vec<Value>) -> String {
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

    /// One task row in TASK_COLS order for the #13091 stub replies.
    fn task_row_13091(id: i64, status: &str, payload: &Value, dedupe_key: &str) -> Value {
        let text = |v: &str| json!({"type": "text", "value": v});
        let int = |v: i64| json!({"type": "integer", "value": v.to_string()});
        let null = json!({"type": "null"});
        json!([
            int(id),
            text("repo-health-code-pilot"),
            text("probe"),
            null,
            text(&payload.to_string()),
            text(dedupe_key),
            int(30),
            text(status),
            null,
            null,
            null,
            null,
            null,
            null,
            int(1),
            int(1000),
            null,
            null,
            null,
            null,
            text("{}"),
            text("{}"),
            null,
            null,
            null,
            null,
            null,
            null,
            null,
            text("2026-11-20T00:00:00Z"),
            text("2026-11-21T00:00:00Z"),
            text("2026-11-21T00:00:00Z"),
            null,
            null,
            null,
            null
        ])
    }

    fn insert_ok_13091(id: i64) -> String {
        json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": [], "rows": [],
                "affected_row_count": 1,
                "last_insert_rowid": id.to_string()}}},
            {"type": "ok", "response": {"type": "close"}}
        ]})
        .to_string()
    }

    /// A #12744-shaped packet whose payload carries labels, so the
    /// mirror enrichment (and its backend call) is skipped and the
    /// first backend call is the dedupe read.
    fn packet_13091(issue: i64) -> Value {
        let mut p = packet_12744(issue, 30);
        p["payload"]["labels"] = json!([]);
        p
    }

    /// The full valid packet payload: a holder row only absorbs
    /// when it is not quarantined, and the claim guard validates
    /// the holder's payload — a minimal payload would (correctly)
    /// read as invalid and absorb nothing.
    fn payload_13091(issue: i64) -> Value {
        packet_12744(issue, 30)["payload"].clone()
    }

    #[test]
    fn enqueue_after_completion_on_the_same_key_files_no_new_task_13091() {
        // Incident shape: the task for this dedupe key completed
        // with a verified disposition; a generator re-files the same
        // packet. The broker must answer with the completed task —
        // never a fresh servable row.
        let done = task_row_13091(
            4242,
            "done",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let fresh = task_row_13091(
            4243,
            "queued",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let (url, captured) = stub_pipeline_routed_13091(vec![
            ("dedupe_key = ?", task_select_reply_13091(vec![done])),
            ("status IN (", task_select_reply_13091(vec![])),
            ("status = 'done'", task_select_reply_13091(vec![])),
            ("INSERT INTO tasks", insert_ok_13091(4243)),
            (
                "FROM tasks WHERE id = ?",
                task_select_reply_13091(vec![fresh]),
            ),
        ]);
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let req = json!({"queue": "repo-health-code-pilot", "packet": packet_13091(10283)});
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], 4242, "{out}");
        assert_eq!(out["row"]["status"], "done", "{out}");
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1, "only the dedupe read may run: {bodies:?}");
        assert!(
            !bodies.iter().any(|b| b.contains("INSERT INTO tasks")),
            "no new task may be filed: {bodies:?}"
        );
    }

    #[test]
    fn enqueue_after_completion_under_another_key_files_no_new_task_13091() {
        // Incident 21296's shape: the completed task sits under a
        // DIFFERENT dedupe key (generators key the same work
        // differently), so only the work identity can see it. A done
        // holder of the identity absorbs the filing the same way.
        let done = task_row_13091(
            4242,
            "done",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let fresh = task_row_13091(
            4243,
            "queued",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283:dispatch:0123abcd",
        );
        let (url, captured) = stub_pipeline_routed_13091(vec![
            ("dedupe_key = ?", task_select_reply_13091(vec![])),
            ("status IN (", task_select_reply_13091(vec![])),
            ("status = 'done'", task_select_reply_13091(vec![done])),
            ("INSERT INTO tasks", insert_ok_13091(4243)),
            (
                "FROM tasks WHERE id = ?",
                task_select_reply_13091(vec![fresh]),
            ),
        ]);
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let mut packet = packet_13091(10283);
        packet["dedupe_key"] =
            json!("repo-health:v1:fix:developing-today/code:issue:10283:dispatch:0123abcd");
        let req = json!({"queue": "repo-health-code-pilot", "packet": packet});
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], 4242, "{out}");
        assert_eq!(out["work_identity_collision"]["status"], "done", "{out}");
        let bodies = captured.lock().unwrap();
        assert!(
            !bodies.iter().any(|b| b.contains("INSERT INTO tasks")),
            "no new task may be filed: {bodies:?}"
        );
    }

    #[test]
    fn enqueue_done_predecessor_with_override_files_a_deliberate_parallel_13091() {
        // The escape hatch stands: a packet that declares
        // dedupe_override files a deliberate parallel even over a
        // completed predecessor (the revive-on-enqueue path).
        let done = task_row_13091(
            4242,
            "done",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let fresh = task_row_13091(
            4243,
            "queued",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let (url, captured) = stub_pipeline_routed_13091(vec![
            ("dedupe_key = ?", task_select_reply_13091(vec![done])),
            ("INSERT INTO tasks", insert_ok_13091(4243)),
            (
                "FROM tasks WHERE id = ?",
                task_select_reply_13091(vec![fresh]),
            ),
        ]);
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let mut packet = packet_13091(10283);
        packet["payload"]["dedupe_override"] = json!("deliberate parallel after a regression");
        let req = json!({"queue": "repo-health-code-pilot", "packet": packet});
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], 4243, "{out}");
        let bodies = captured.lock().unwrap();
        assert!(
            bodies.iter().any(|b| b.contains("INSERT INTO tasks")),
            "the override must file: {bodies:?}"
        );
    }

    #[test]
    fn enqueue_failed_predecessor_still_revives_13091() {
        // Only completion is terminal for a filing: a FAILED
        // predecessor keeps the #6838 revive-on-enqueue behavior.
        let failed = task_row_13091(
            4242,
            "failed",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let fresh = task_row_13091(
            4243,
            "queued",
            &payload_13091(10283),
            "repo-health:v1:fix:developing-today/code:issue:10283",
        );
        let (url, captured) = stub_pipeline_routed_13091(vec![
            ("dedupe_key = ?", task_select_reply_13091(vec![failed])),
            ("status IN (", task_select_reply_13091(vec![])),
            ("status = 'done'", task_select_reply_13091(vec![])),
            ("INSERT INTO tasks", insert_ok_13091(4243)),
            (
                "FROM tasks WHERE id = ?",
                task_select_reply_13091(vec![fresh]),
            ),
        ]);
        let backend = TursoBackend::new(url, "test".to_string(), ureq::agent());
        let req = json!({"queue": "repo-health-code-pilot", "packet": packet_13091(10283)});
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["row"]["id"], 4243, "{out}");
        let bodies = captured.lock().unwrap();
        assert!(
            bodies.iter().any(|b| b.contains("INSERT INTO tasks")),
            "a failed predecessor must still revive: {bodies:?}"
        );
    }

    #[test]
    fn claim_pick_never_serves_broker_or_person_condition_parks_13091() {
        // The parked half, at the exact SQL the serve path runs:
        // the default pick predicate (shared verbatim by the pick
        // SELECT and the claim UPDATE re-check) evaluated per row.
        // A park whose recorded term is a broker-owned condition
        // (task_terminal / task_status / event_seen — released by
        // the park-revalidation sweeper) or a person's answer
        // (person_answer — released by /wake when the person acts)
        // is NOT the pick's to timer-serve; every other park keeps
        // its timer release, and a completed row is never servable.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (id INTEGER PRIMARY KEY, queue TEXT, payload TEXT,
               status TEXT, attempts INTEGER, max_attempts INTEGER, not_before TEXT,
               wait_kind TEXT, ready_condition TEXT, required_budget_minutes INTEGER,
               priority INTEGER, created_at TEXT, progress TEXT);
             CREATE TABLE repo_task_lane_ledger (task_id INTEGER, platform_limit TEXT,
               platform_limit_reason TEXT);",
        )
        .unwrap();
        let now = "2026-11-23T12:00:00Z";
        let past = "2026-11-23T11:00:00Z";
        let rows: [(i64, &str, Option<&str>, Option<&str>); 8] = [
            (1, "queued", None, None),
            (2, "parked", Some(past), Some("{\"kind\":\"time\"}")),
            (3, "parked", Some(past), Some("{\"kind\":\"pr_merged\"}")),
            (
                4,
                "parked",
                Some(past),
                Some("{\"kind\":\"task_terminal\",\"task_id\":99}"),
            ),
            (
                5,
                "parked",
                Some(past),
                Some("{\"kind\":\"person_answer\"}"),
            ),
            (
                6,
                "parked",
                Some(past),
                Some("{\"kind\":\"event_seen\",\"event_kind\":\"x\"}"),
            ),
            (7, "done", None, None),
            (8, "parked", Some(past), None),
        ];
        for (id, status, nb, rc) in rows {
            conn.execute(
                "INSERT INTO tasks (id, queue, payload, status, attempts, max_attempts, \
                   not_before, wait_kind, ready_condition, required_budget_minutes, \
                   priority, created_at) \
                 VALUES (?, 'repo-health-code-pilot', '{\"kind\":\"small_fix\"}', ?, \
                   0, 1000, ?, NULL, ?, NULL, 30, '2026-11-20T00:00:00Z')",
                rusqlite::params![id, status, nb, rc],
            )
            .unwrap();
        }
        let pred = default_pick_predicate();
        let scope = claim_scope(None, None, &ClaimerCtx::default());
        let mut args: Vec<rusqlite::types::Value> = vec![
            rusqlite::types::Value::Text(now.to_string()),
            rusqlite::types::Value::Text(now.to_string()),
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Null,
        ];
        for a in &scope.args {
            args.push(match a {
                Arg::Text(s) => rusqlite::types::Value::Text(s.clone()),
                Arg::Int(i) => rusqlite::types::Value::Integer(*i),
                Arg::Float(f) => rusqlite::types::Value::Real(*f),
                Arg::Null => rusqlite::types::Value::Null,
            });
        }
        let servable = |id: i64| -> bool {
            let sql = format!(
                "SELECT COUNT(*) FROM tasks WHERE id = ? AND payload IS NOT NULL \
                 AND payload <> '' AND payload <> '{{}}' AND ( {pred} ) \
                 {PLATFORM_LIMIT_PICK_CLAUSE}{}",
                scope.sql
            );
            let mut all: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Integer(id)];
            all.extend(args.iter().cloned());
            conn.query_row(&sql, rusqlite::params_from_iter(all), |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
                > 0
        };
        assert!(servable(1), "a due queued row stays servable");
        assert!(servable(2), "a timed park keeps its timer release");
        assert!(
            servable(3),
            "a lane-recheck condition park keeps its timer release"
        );
        assert!(
            !servable(4),
            "a task_terminal park is the sweeper's to release, not the pick's"
        );
        assert!(
            !servable(5),
            "a person_answer park waits on a person, not on a timer"
        );
        assert!(
            !servable(6),
            "an event_seen park is the sweeper's to release, not the pick's"
        );
        assert!(!servable(7), "a completed task is never served");
        assert!(
            servable(8),
            "a legacy park with no recorded condition keeps its timer release"
        );
        // The claim UPDATE re-check embeds the same predicate, so a
        // row the pick cannot select cannot be claimed by racing it.
        let upd = claim_update_sql(&pred);
        assert!(
            upd.contains("person_answer"),
            "the claim re-check carries the same exclusion: {upd}"
        );
    }

    #[test]
    fn payload_issue_ref_shapes_12744() {
        // The identity the enrichment keys on: the payload's own
        // repo first, the repo-health dedupe key as fallback, and
        // nothing for shapes that cannot key the mirror.
        let p = json!({"repo": "developing-today/code", "issue": {"number": 7}});
        assert_eq!(
            payload_issue_ref(&p, None),
            Some(("developing-today/code".to_string(), 7))
        );
        let p = json!({"issue": {"number": 7}});
        assert_eq!(
            payload_issue_ref(
                &p,
                Some("repo-health:v1:triage:developing-today/code:issue:7")
            ),
            Some(("developing-today/code".to_string(), 7))
        );
        assert_eq!(payload_issue_ref(&p, None), None);
        assert_eq!(
            payload_issue_ref(&p, Some("repo-health:v1:pr:developing-today/code:pr:7")),
            None
        );
        let p = json!({"repo": "code", "issue": {"number": 7}});
        assert_eq!(payload_issue_ref(&p, None), None);
        let p = json!({"repo": "developing-today/code"});
        assert_eq!(payload_issue_ref(&p, None), None);
    }

    #[test]
    fn follow_on_band_or_inherited_12744() {
        // The triage follow-on's stored priority: the label band
        // wins over the inherited row priority (the label is
        // Drew's signal), the inherited value stands without one.
        assert_eq!(
            band_or_inherited(&json!({"labels": ["expedite"]}), Some(30)),
            Some(15)
        );
        assert_eq!(
            band_or_inherited(&json!({"labels": ["next-up"]}), Some(30)),
            Some(25)
        );
        assert_eq!(band_or_inherited(&json!({}), Some(30)), Some(30));
        assert_eq!(band_or_inherited(&json!({}), None), None);
        assert_eq!(
            band_or_inherited(&json!({"labels": ["expedite"]}), None),
            Some(15)
        );
    }

    #[test]
    fn not_claimed_reason_names_a_lapsed_lease_12618() {
        // #12618 oracle: a worker whose lease expired is told the task
        // went back to the queue, not just that it is "not claimed";
        // a claim held by another worker names that worker; a missing
        // task says so; and an expiry by someone else, or one followed
        // by a new claim, adds no lease detail.
        let queued = json!({"status": "queued", "worker_id": null});
        let expired = json!({
            "worker_id": "archer",
            "kind": "lease_expired",
            "created_at": "2026-11-22T16:00:00Z",
        });
        let reason = not_claimed_reason(Some(&queued), Some(&expired), "archer");
        assert!(
            reason.starts_with("it is queued; archer's lease expired at 2026-11-22T16:00:00Z"),
            "{reason}"
        );
        assert!(
            reason.contains("claim it again before completing it"),
            "{reason}"
        );

        let held = json!({"status": "claimed", "worker_id": "felix"});
        assert_eq!(
            not_claimed_reason(Some(&held), None, "archer"),
            "it is claimed by felix"
        );
        assert_eq!(not_claimed_reason(None, None, "archer"), "no such task");

        let other = json!({"worker_id": "felix", "kind": "lease_expired"});
        assert_eq!(
            not_claimed_reason(Some(&queued), Some(&other), "archer"),
            "it is queued"
        );
        let reclaimed = json!({"worker_id": "archer", "kind": "claimed"});
        assert_eq!(
            not_claimed_reason(Some(&queued), Some(&reclaimed), "archer"),
            "it is queued"
        );
    }

    #[test]
    fn bind_rejects_malformed_session_id_8287() {
        // #8287 oracle: verb_bind refuses a malformed session id
        // as a usage error (exit 2) before any store access, so
        // the task is left unbound — the dummy backend proves the
        // refusal happens pre-store (any store touch would fail
        // differently). A well-formed id must NOT produce the
        // invalid-session refusal (it proceeds to the store path
        // and fails there against this backend, or on the task).
        let backend = TursoBackend::new(
            "http://127.0.0.1:0".to_string(),
            "test".to_string(),
            ureq::agent(),
        );
        for bad in ["not-a-session", "", "ses_", "ses_a'; DROP TABLE tasks; --"] {
            let out = verb_bind(
                &backend,
                &json!({"task_id": 1, "worker": "w", "session_id": bad}),
            );
            assert_eq!(out["ok"], false, "{bad}: {out}");
            assert_eq!(out["exit"], 2, "{bad}: {out}");
            assert!(
                out["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains("invalid session_id"),
                "{bad}: {out}"
            );
        }
        let out = verb_bind(
            &backend,
            &json!({"task_id": 1, "worker": "w", "session_id": "ses_e5e37addeb0ffeumu1UPO2bI8G"}),
        );
        assert!(
            !out["error"]
                .as_str()
                .unwrap_or("")
                .contains("invalid session_id"),
            "well-formed id refused as invalid: {out}"
        );
    }

    #[test]
    fn model_stamp_detail_9046_carries_session_and_trigger() {
        // #9046 oracle: a stamp detail names the session it
        // belongs to (or null — a session is never fabricated),
        // the serving leaf, and the touch that produced it.
        let m = ClaimModel {
            provider: "opencode-route".to_string(),
            model: Some("workhorse".to_string()),
            variant: None,
        };
        let d = model_stamp_detail(&m, Some("ses_abc"), "transfer");
        assert_eq!(d["session_id"], json!("ses_abc"));
        assert_eq!(d["provider"], json!("opencode-route"));
        assert_eq!(d["model"], json!("workhorse"));
        assert_eq!(d["variant"], Value::Null);
        assert_eq!(d["trigger"], json!("transfer"));
        let d2 = model_stamp_detail(&m, None, "claim");
        assert_eq!(d2["session_id"], Value::Null);
        assert_eq!(d2["trigger"], json!("claim"));
    }

    #[test]
    fn timeline_wiring_9046() {
        // #9046 source oracle: the broker half of the timeline —
        // the claim_session_id column, model_stamp events from
        // the one stamp site, session_unbound from the one clear
        // funnel, the /events-list read verb, and claimed_at on
        // the rich list (cmd_complete's window source).
        let src = &test_source();
        assert!(
            src.contains("claim_session_id TEXT"),
            "schema carries claim_session_id"
        );
        assert!(
            src.contains("\"/events-list\" => verb_events_list(t, req)"),
            "dispatch routes /events-list"
        );
        let stamp_seg = seg8217(src, "fn stamp_claim_model(", "\n/// The SQL predicate");
        assert!(
            stamp_seg.contains("\"model_stamp\""),
            "the stamp site writes the timeline event"
        );
        assert!(
            stamp_seg.contains("claim_session_id = ?"),
            "the stamp site updates claim_session_id"
        );
        let clear_seg = seg8217(src, "fn on_binding_cleared(", "\n/// What lease expiry");
        assert!(
            clear_seg.contains("\"session_unbound\""),
            "the clear funnel writes session_unbound"
        );
        assert!(
            src.contains("worker_id, claimed_at, lease_until"),
            "rich list carries claimed_at"
        );
    }

    #[test]
    fn events_list_cross_task_window() {
        // Claim-link rollup (#10357): /events-list also serves
        // the cross-task window read — task_id optional, a since
        // floor, and task_id among the selected columns — while
        // the per-task #9046 read keeps its shape.
        let src = &test_source();
        let seg = seg8217(src, "fn verb_events_list(", "\n/// prune_events, shared by");
        assert!(
            !seg.contains("task_id is required"),
            "cross-task listing must not hard-require task_id"
        );
        assert!(
            seg.contains("created_at >= ?"),
            "the since floor filters on created_at"
        );
        assert!(
            seg.contains("id, task_id, worker_id, kind, detail, created_at"),
            "listed rows carry their task_id for the rollup join"
        );
    }

    #[test]
    fn list_exact_readback_filters_9420() {
        // #9420 oracle: exact-task readback works on the Turso
        // verb_list — the task filter, a dedupe_key filter, and a
        // queue predicate that applies only when a queue is named (an
        // unnamed queue must not degenerate into queue = '').
        let src = &test_source();
        let seg = seg8217(src, "fn verb_list(", "\nfn verb_health");
        assert!(
            seg.contains("req_i64(req, \"task\")"),
            "task filter present"
        );
        assert!(seg.contains("AND id = ?"), "task filter narrows by id");
        assert!(
            seg.contains("req_str(req, \"dedupe_key\")") && seg.contains("AND dedupe_key = ?"),
            "dedupe_key filter present"
        );
        assert!(
            seg.contains("if !queue.is_empty()"),
            "queue predicate is conditional on a named queue"
        );
    }

    #[test]
    fn stack_storage_wiring_8582() {
        // Source oracle for increment 2 of #8582: the schema applies
        // the core's DDL of record, dispatch routes both stack
        // verbs, and the record verb decides validation + append
        // planning in the pure core (never in SQL).
        let src = &test_source();
        assert!(
            src.contains("crate::merge_intent::MERGE_INTENT_DDL"),
            "schema applies the merge_intents DDL of record"
        );
        let dispatch_seg = seg8217(src, "pub fn dispatch(", "\n#[cfg(test)]");
        assert!(
            dispatch_seg.contains("\"/stack-record\" => verb_stack_record(t, req)"),
            "dispatch routes /stack-record"
        );
        assert!(
            dispatch_seg.contains("\"/stack-list\" => verb_stack_list(t, req)"),
            "dispatch routes /stack-list"
        );
        let record = seg8217(src, "fn verb_stack_record(", "\nfn verb_stack_list(");
        assert!(
            record.contains("validate_append("),
            "record validates via the core"
        );
        assert!(record.contains("plan_append("), "record plans via the core");
        // Increment 3: the record verb validates the owner
        // lane/worker pair against the lanes registry, so a
        // conflict route recorded here names a real owner.
        assert!(
            record.contains("FROM lanes WHERE lane_id = ?"),
            "record validates the owner pair against the lane registry"
        );
        assert!(
            record.contains("unknown owner lane_id"),
            "record refuses an unregistered owner lane"
        );
        let list = seg8217(src, "fn verb_stack_list(", "\nfn verb_signal_count(");
        assert!(
            list.contains("stack_state("),
            "list answers the derived stack state"
        );
    }

    #[test]
    fn stack_transition_wiring_8582() {
        // Source oracle for increment 4 of #8582: dispatch routes
        // the transition verb, and the verb decides legality in the
        // pure core's transition table (never in SQL), recording
        // the landing PR number and the conflict detail.
        let src = &test_source();
        let dispatch_seg = seg8217(src, "pub fn dispatch(", "\n#[cfg(test)]");
        assert!(
            dispatch_seg.contains("\"/stack-transition\" => verb_stack_transition(t, req)"),
            "dispatch routes /stack-transition"
        );
        let seg = seg8217(src, "fn verb_stack_transition(", "\nfn verb_signal_count(");
        assert!(
            seg.contains("can_transition("),
            "transition legality comes from the core table"
        );
        assert!(
            seg.contains("illegal move"),
            "an illegal move is a refusal, not a write"
        );
        assert!(seg.contains("pr_number"), "a landing records its PR number");
        assert!(
            seg.contains("conflict_detail"),
            "a conflict records its routing detail"
        );
        assert!(
            seg.contains("stack_state("),
            "transition answers the derived stack state"
        );
    }

    #[test]
    fn stack_restack_wiring_8582() {
        // Source oracle for the restack acknowledgement (#8582;
        // Codex P2, PR #8849): dispatch routes the restack verb,
        // and the verb decides the refresh in the pure core
        // (plan_restack) — never in SQL — and answers the derived
        // stack state.
        let src = &test_source();
        let dispatch_seg = seg8217(src, "pub fn dispatch(", "\n#[cfg(test)]");
        assert!(
            dispatch_seg.contains("\"/stack-restack\" => verb_stack_restack(t, req)"),
            "dispatch routes /stack-restack"
        );
        let seg = seg8217(src, "fn verb_stack_restack(", "\nfn verb_signal_count(");
        assert!(
            seg.contains("plan_restack("),
            "restack planning comes from the core"
        );
        assert!(
            seg.contains("base_branch = ?"),
            "restack writes the refreshed base"
        );
        assert!(
            seg.contains("stack_state("),
            "restack answers the derived stack state"
        );
    }

    #[test]
    fn canonical_lanes_are_the_full_family_universe() {
        // Drew, 2026-11-16 (#6853): the canonical seed set is the
        // full variation space -- six families x a..=z = 156
        // lanes, every one enabled. This supersedes the #8097 pin
        // (36 lanes, 17 enabled): the trims left live lanes
        // unseeded (opencode-a) or seeded disabled (not-code-c),
        // and claims met "unknown lane_id" / "lane is disabled"
        // for days. Seeding an empty lanes table executes
        // lane_seed_sql() -- a pure function of this set -- so
        // pinning the set and its per-lane tuples (next test)
        // pins the seeded database exactly.
        let lanes = canonical_lanes();
        assert_eq!(lanes.len(), 156);
        let mut ids: Vec<&str> = lanes.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 156, "duplicate lane ids in the canonical set");
        let mut expected_ids: Vec<String> = Vec::new();
        for family in ["code", "any", "not-code", "opencode", "openchamber", "nix"] {
            for c in 'a'..='z' {
                expected_ids.push(format!("{family}-{c}"));
            }
        }
        expected_ids.sort();
        assert_eq!(
            ids,
            expected_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "the canonical set is not the full family universe"
        );
        // The lanes whose absence or disabled state broke live
        // claims: all present now.
        for id in [
            "opencode-a",
            "openchamber-a",
            "nix-a",
            "code-z",
            "any-z",
            "not-code-z",
            "not-code-c",
        ] {
            assert!(ids.contains(&id), "canonical set misses {id}");
        }
        assert!(
            ids.iter().all(|id| !id.starts_with("tmp-")),
            "tmp rename ids must not be seeded"
        );
        // Every canonical lane is enabled (Drew, 2026-11-16).
        assert!(
            lanes.iter().all(|(_, _, e)| *e),
            "every canonical lane is enabled"
        );
        // Worker ownership follows the family.
        let owner = |id: &str| {
            lanes
                .iter()
                .find(|(l, _, _)| l.as_str() == id)
                .map(|(_, w, _)| *w)
        };
        assert_eq!(owner("code-a"), Some("repo-worker-code"));
        assert_eq!(owner("code-z"), Some("repo-worker-code"));
        assert_eq!(owner("any-z"), Some("repo-worker-code"));
        assert_eq!(owner("not-code-z"), Some("repo-worker-code"));
        assert_eq!(owner("not-code-c"), Some("repo-worker-code"));
        assert_eq!(owner("not-code-f"), Some("repo-worker-code"));
        // The not-code lanes other workers own keep their owners.
        assert_eq!(owner("not-code-a"), Some("repo-worker-nix"));
        assert_eq!(owner("not-code-g"), Some("repo-worker-nix"));
        assert_eq!(owner("not-code-b"), Some("repo-worker-opencode"));
        assert_eq!(owner("not-code-e"), Some("repo-worker-opencode"));
        assert_eq!(owner("not-code-d"), Some("repo-worker-openchamber"));
        assert_eq!(owner("opencode-a"), Some("repo-worker-opencode"));
        assert_eq!(owner("openchamber-a"), Some("repo-worker-openchamber"));
        assert_eq!(owner("nix-a"), Some("repo-worker-nix"));
    }

    #[test]
    fn execution_lane_fields_split_7598() {
        // Oracle (#7598): the execution record splits into the
        // mode and the executing lane id, so a claim stamped with
        // a lane id and a completion carrying that same id record
        // executed_lane_id == claim lane -- no mismatch. A raw
        // lane id in `lane` (an unnormalized caller) translates
        // exactly like the client's normalization.
        assert_eq!(
            execution_lane_fields(&json!({"lane": "any-n"})),
            (Some("direct".to_string()), Some("any-n".to_string()))
        );
        assert_eq!(
            execution_lane_fields(&json!({"lane": "opencode-a"})),
            (Some("opencode".to_string()), Some("opencode-a".to_string()))
        );
        assert_eq!(
            execution_lane_fields(&json!({"lane": "direct", "lane_id": "code-a"})),
            (Some("direct".to_string()), Some("code-a".to_string()))
        );
        assert_eq!(
            execution_lane_fields(&json!({"lane": "opencode"})),
            (Some("opencode".to_string()), None)
        );
        assert_eq!(execution_lane_fields(&json!({})), (None, None));
    }

    #[test]
    fn lane_family_of_accepts_known_families_and_rejects_garbage() {
        // The family rule (Drew, 2026-11-16), kept for
        // execution_lane_fields' mode derivation (#7598): known
        // family + nonempty lowercase alphabetic suffix. Claims no
        // longer consult it (#12544): any lane name auto-registers
        // on first claim, whatever family it parses to (or none).
        for (id, family) in [
            ("code-a", "code"),
            ("code-z", "code"),
            ("code-abc", "code"),
            ("any-a", "any"),
            ("any-z", "any"),
            ("not-code-c", "not-code"),
            ("not-code-z", "not-code"),
            ("opencode-a", "opencode"),
            ("openchamber-b", "openchamber"),
            ("nix-a", "nix"),
            ("darwin-nix-a", "darwin-nix"),
        ] {
            assert_eq!(lane_family_of(id), Some(family), "{id}");
        }
        for id in [
            "banana-1", // unknown family, non-alpha suffix
            "code-",    // empty suffix
            "code",     // no suffix at all
            "",         // empty id
            "-a",       // empty family
            "code-A",   // uppercase suffix
            "code-a1",  // non-alpha suffix
            "unknown-x",
            "not-code", // a family prefix alone is not a lane id
            "code-a-b", // the family would be "code-a": unknown
            "tmp-a",    // rename scratch ids are not lanes
            "CODE-a",   // uppercase family
        ] {
            assert_eq!(lane_family_of(id), None, "{id}");
        }
    }

    #[test]
    fn lane_provision_sql_registers_enabled_unbound_for_the_claimer_12544() {
        // #12544 oracle (auto-register): the statement
        // claim_preamble runs for a lane with no registry row
        // registers exactly the canonical row shape -- the
        // claiming worker as owner (the second bound parameter),
        // the one-queue binding "*", enabled = 1 -- and INSERT OR
        // IGNORE means an existing row (in particular one
        // disabled by hand) is never overwritten by a registration
        // attempt. After this insert the preamble re-reads the
        // row, so a claim under a never-seen name ("archer",
        // "banana-1", a bare "not-code") proceeds with the lane
        // present and enabled in the registry.
        assert_eq!(
            lane_provision_sql(),
            "INSERT OR IGNORE INTO lanes (lane_id, worker_id, queue, enabled) \
             VALUES (?, ?, '*', 1)"
        );
    }

    #[test]
    fn claim_preamble_auto_registers_instead_of_refusing_unknown_lanes_12544() {
        // #12544 oracle (unknown-lane errors are gone; the
        // existing-row refusals stay): the claim preamble
        // provisions a missing lane via lane_provision_sql and
        // never refuses a name as unknown -- the unknown-lane
        // verdict function is deleted outright -- while the
        // checks an EXISTING row carries keep their refusals in
        // the same preamble: worker ownership (a lane disabled or
        // owned is an explicit recorded state, not an unknown
        // name) and the disabled flag both still refuse, and the
        // stream-binding check stays.
        let src = &test_source();
        let preamble = seg8217(src, "fn claim_preamble(", "\nfn claim_pick_once(");
        assert!(
            preamble.contains("lane_provision_sql()"),
            "preamble auto-registers a missing lane"
        );
        assert!(
            !preamble.contains("unknown_lane_error"),
            "preamble never refuses a lane as unknown"
        );
        // The banned needle is built with format! so this test
        // file does not itself contain the literal it bans.
        let verdict_fn = format!("fn unknown_lane_{}", "error");
        assert!(
            !src.contains(&verdict_fn),
            "the unknown-lane verdict is gone from the backend"
        );
        assert!(
            preamble.contains("belongs to worker"),
            "an existing row's owner still binds the lane"
        );
        assert!(
            preamble.contains("is disabled"),
            "a lane disabled by hand still refuses"
        );
        assert!(
            preamble.contains("lane_queue_binds("),
            "preamble honors the unbound registration marker"
        );
    }

    #[test]
    fn lane_seed_sql_is_insert_or_ignore_over_the_canonical_set() {
        // #6853 oracle: the seed can only fill missing rows -- an
        // existing registration (in particular a lane disabled via
        // enabled = 0) is never overwritten, so seeding at every
        // broker start cannot undo a disable -- and every seeded
        // row carries its canonical fields, so a restored lane
        // comes back with its owning worker, the one-queue
        // binding, and enabled = 1 (the whole canonical set is
        // enabled since 2026-11-16).
        let sql = lane_seed_sql();
        assert!(sql.starts_with("INSERT OR IGNORE INTO lanes"), "{sql}");
        for (id, worker, enabled) in canonical_lanes() {
            let tuple = format!("('{id}', '{worker}', '*', {})", i32::from(enabled));
            assert!(sql.contains(&tuple), "seed misses {tuple}");
        }
        // Nothing outside the canonical set is seeded: the
        // wildcard-only darwin-nix family and the rename scratch
        // ids stay unseeded (a darwin-nix lane row appears only
        // when a claim provisions it).
        for outside in ["darwin-nix-a", "darwin-nix-z"] {
            assert!(
                !sql.contains(&format!("('{outside}'")),
                "seed must not name {outside}"
            );
        }
        assert!(!sql.contains("('tmp-"), "seed must not name tmp ids");
    }

    #[test]
    fn claim_lease_is_the_requested_lease_not_the_heartbeat_default() {
        // #7617 oracle: a claim's stored lease_until is claimed_at +
        // the lease the caller requested (2700 s for a 45-minute
        // claim), never claimed_at + the 900 s heartbeat extension
        // default. Task 11049 (2026-11-08) reported the former and
        // died at the latter.
        let claimed = 1_794_020_400; // 2026-11-07T03:00:00Z
        assert_eq!(claim_lease_until(claimed, 2700), "2026-11-07T03:45:00Z");
        assert_eq!(
            claim_lease_until(claimed, CLAIM_DEFAULT_LEASE_SECONDS),
            claim_lease_until(claimed, 2700)
        );
        assert_ne!(
            claim_lease_until(claimed, CLAIM_DEFAULT_LEASE_SECONDS),
            iso(claimed + HEARTBEAT_DEFAULT_EXTEND_SECONDS)
        );
        assert_eq!(CLAIM_DEFAULT_LEASE_SECONDS, 2700);
        assert_eq!(HEARTBEAT_DEFAULT_EXTEND_SECONDS, 900);
    }

    #[test]
    fn claim_lane_argument_is_stored_verbatim() {
        // #7617 / #7598 oracle: the lane passed at claim is the lane
        // stored on the task row — the claim writes lane_id through
        // arg_str(lane) with no substitution, and a lane-less claim
        // stores NULL rather than some other lane's id.
        match arg_str(Some("code-u")) {
            Arg::Text(v) => assert_eq!(v, "code-u"),
            _ => panic!("a passed lane must reach the UPDATE as text"),
        }
        match arg_str(None) {
            Arg::Null => {}
            _ => panic!("a lane-less claim must store NULL lane_id"),
        }
    }

    #[test]
    fn claim_update_sql_rechecks_pick_eligibility_for_the_picked_id() {
        // #7068 oracle, statement shape: the claim UPDATE is keyed
        // to the picked row's id and re-checks the full pick
        // predicate, so when two lanes pick the same row in the
        // same instant the loser's UPDATE matches 0 rows (the
        // winner already flipped the row to 'claimed', which the
        // predicate excludes) and the claim loop re-picks instead
        // of handing the loser the winner's row.
        let predicate = "(status = 'queued' AND attempts < max_attempts \
             AND (not_before IS NULL OR not_before <= ?)) \
           OR (status = 'parked' AND not_before IS NOT NULL AND not_before <= ?)";
        let sql = claim_update_sql(predicate);
        assert!(sql.starts_with("UPDATE tasks SET status = 'claimed'"));
        assert!(sql.contains("WHERE id = ?"), "claim is keyed by id: {sql}");
        assert!(
            sql.contains("status = 'queued'"),
            "predicate re-checked: {sql}"
        );
        assert!(
            sql.contains("status = 'parked'"),
            "predicate re-checked: {sql}"
        );
        assert!(
            sql.contains("attempts < max_attempts"),
            "cap re-checked: {sql}"
        );
        assert!(
            sql.contains("lane_id = ?"),
            "winner's lane stamp is the claim's own: {sql}"
        );
    }

    #[test]
    fn claim_race_two_lanes_exactly_one_winner_per_task() {
        // #7068 fixture (task 11895, lanes code-m/code-f,
        // 2026-11-08): two lanes with distinct lane ids race one
        // queued task in the same queue. Exactly one claim wins;
        // the loser's eligibility view of that row closes the
        // instant the winner's claim lands, and the next queued
        // row stays claimable so the loser moves on to it (or to
        // a no-task answer) instead of receiving the winner's row.
        let now = "2026-11-08T02:32:59Z";
        let mut task = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        // Both lanes see the row as claimable before the race.
        assert!(by_id_claimable(&task, now));
        // Lane code-f wins: its claim flips the row and stamps
        // code-f's lane id (the state lane code-m's losing claim
        // call observed on task 11895).
        task["status"] = json!("claimed");
        task["lane_id"] = json!("code-f");
        task["worker_id"] = json!("repo-worker-code");
        // The loser (code-m, same shared worker id) can no longer
        // claim this row by id, and the pick predicate's status
        // terms ('queued'/'parked') no longer match it either.
        assert!(!by_id_claimable(&task, now));
        // The next eligible task remains claimable for the loser.
        let next = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        assert!(by_id_claimable(&next, now));
    }

    #[test]
    fn claim_race_same_worker_two_lanes_live_claim_refusal_is_recorded() {
        // #6985 recurrence fixture (task 17490, 2026-11-16, lanes
        // code-j / code-g, both worker repo-worker-code): the second
        // lane's claim attempt against a live-claimed row must be
        // refused by every claim surface AND leave a record. Before
        // this fix the by-id refusal was a silent Ok(None): no
        // event, indistinguishable from the first claim in every
        // readback the fleet has.
        let now = "2026-11-16T13:47:00Z";
        let task = json!({
            "status": "claimed", "attempts": 1, "max_attempts": 1000,
            "not_before": null, "platform": null, "lane_id": "code-j",
            "worker_id": "repo-worker-code",
            "lease_until": "2026-11-16T14:25:22Z",
        });
        // No claim surface may take the row while the lease stands.
        assert!(!by_id_claimable(&task, now));
        // The refusal names the holder from the row's lease fields.
        let (worker, lane, lease_until) =
            live_claim_holder(&task, now).expect("a live claim has a holder");
        assert_eq!(worker, "repo-worker-code");
        assert_eq!(lane.as_deref(), Some("code-j"));
        assert_eq!(lease_until, "2026-11-16T14:25:22Z");
        // Once the lease lapses the row is no longer live-held (the
        // expire path, not a second claim, is what releases it).
        assert!(live_claim_holder(&task, "2026-11-16T14:25:23Z").is_none());
        // A queued row has no holder.
        let queued = json!({"status": "queued", "lease_until": null});
        assert!(live_claim_holder(&queued, now).is_none());
        // Source oracle: the by-id refusal path records the
        // claim_refused event (the event half of this fix).
        let src = &test_source();
        let seg = src
            .split("fn claim_by_id(")
            .nth(1)
            .and_then(|s| s.split("\nfn ").next())
            .unwrap_or("");
        assert!(
            seg.contains("\"claim_refused\""),
            "claim_by_id records a claim_refused event for a live-claimed row"
        );
        assert!(
            seg.contains("live_claim_holder(&row, &now)"),
            "the refusal is keyed on the row's live lease, not the claimer"
        );
    }

    #[test]
    fn lifecycle_verbs_refuse_another_lanes_claim_6985() {
        // #6985 (task 12453, 2026-11-08): lane B's checkpoint and
        // complete succeeded on lane A's live claim because every
        // lane is worker repo-worker-code. A verb that names its
        // lane is refused on a row another lane claimed.
        let row =
            json!({"status": "claimed", "worker_id": "repo-worker-code", "lane_id": "code-a"});
        assert_eq!(
            foreign_lane_claim(&row, &json!({"lane": "code-b"})),
            Some(("code-a".to_string(), "code-b".to_string()))
        );
        // The holder's own lane passes.
        assert_eq!(foreign_lane_claim(&row, &json!({"lane": "code-a"})), None);
        // No caller lane, a null lane, an execution mode, or a row
        // claimed with no lane keeps the worker-only check.
        assert_eq!(foreign_lane_claim(&row, &json!({})), None);
        assert_eq!(foreign_lane_claim(&row, &json!({"lane": null})), None);
        assert_eq!(foreign_lane_claim(&row, &json!({"lane": "direct"})), None);
        let laneless =
            json!({"status": "claimed", "worker_id": "repo-worker-code", "lane_id": null});
        assert_eq!(
            foreign_lane_claim(&laneless, &json!({"lane": "code-b"})),
            None
        );
        // Source oracle: every holder-checked verb hands its request
        // to the guard, and the guard records the refusal.
        let src = &test_source();
        for verb in [
            "heartbeat_task",
            "checkpoint_task",
            "park_task",
            "release_repo_task",
            "block_task",
            "fail_repo_task",
            "complete_task",
        ] {
            assert!(
                src.contains(&format!("claimed_by(t, task_id, &worker, \"{verb}\", req)")),
                "{verb} passes its request (and lane) to claimed_by"
            );
        }
        let guard = src
            .split("pub(crate) fn claimed_by(")
            .nth(1)
            .and_then(|s| s.split("\npub(crate) fn ").next())
            .unwrap_or("");
        assert!(guard.contains("\"claimed_by_another_lane\""));
    }

    #[test]
    fn claim_paths_never_reread_a_workers_newest_claimed_row() {
        // #7068 source oracle: the pre-#7617 claim shape claimed
        // via a subquery and re-read the result as the newest
        // claimed row for (queue, worker_id) — with every lane
        // sharing worker_id repo-worker-code, a losing claim was
        // handed another lane's just-claimed task, lane stamp and
        // all. That re-read must never come back: both claim paths
        // (pick and by-id) read the claimed row back by its own id.
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        let code: String = impl_src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "ORDER BY claimed_at",
            "WHERE queue = ? AND worker_id",
            "worker_id = ? ORDER BY",
        ] {
            assert!(
                !code.contains(forbidden),
                "claim path must not re-read by worker: found {forbidden}"
            );
        }
        assert!(
            code.contains("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
            "claim read-back is by the claimed row's own id"
        );
        assert!(
            code.contains("claim_update_sql(&pick_predicate)"),
            "pick path claims through the id-keyed conditional UPDATE"
        );
    }

    #[test]
    fn claim_model_stamp_requires_provider_and_model_together() {
        // #8176 oracle: the stamp names the serving leaf — provider
        // and model together, variant optional. Naming either half
        // alone (or a variant alone) is an error, so a half-stamp
        // can never land on a task row.
        let full = json!({
            "model_provider": "_claude", "model_id": "claude-sonnet-5-5",
            "model_variant": "medium",
        });
        let parsed = parse_claim_model(&full).expect("full stamp parses");
        assert_eq!(
            parsed,
            Some(ClaimModel {
                provider: "_claude".to_string(),
                model: Some("claude-sonnet-5-5".to_string()),
                variant: Some("medium".to_string()),
            })
        );
        let no_variant = json!({
            "model_provider": "openai.main", "model_id": "gpt-6-luna",
        });
        assert_eq!(
            parse_claim_model(&no_variant)
                .expect("variant-less stamp parses")
                .and_then(|m| m.variant),
            None
        );
        assert!(
            parse_claim_model(&json!({}))
                .expect("empty claim")
                .is_none()
        );
        assert!(parse_claim_model(&json!({"model_provider": "_claude"})).is_err());
        assert!(parse_claim_model(&json!({"model_id": "claude-sonnet-5-5"})).is_err());
        assert!(parse_claim_model(&json!({"model_variant": "medium"})).is_err());
        // The non-session marker (Muse lanes) carries a provider
        // and no model — parse keeps it distinct from a leaf.
        let marker = json!({"model_provider": "muse-lane", "model_id": "muse-lane"});
        assert!(parse_claim_model(&marker).is_ok());
    }

    #[test]
    fn claim_model_columns_are_in_the_task_read_and_schema() {
        // #8176 source oracle: the stamp columns are part of the
        // task read surface (TASK_COLS), the fresh schema, and the
        // additive migration, so a claimed row reads its stamp back
        // through every verb that returns tasks.
        assert!(TASK_COLS.contains("claim_provider"));
        assert!(TASK_COLS.contains("claim_model"));
        assert!(TASK_COLS.contains("claim_variant"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("claim_provider TEXT"));
        assert!(schema.contains("claim_model TEXT"));
        assert!(schema.contains("claim_variant TEXT"));
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("\"claim_provider\""));
        assert!(impl_src.contains("stamp_claim_model"));
    }

    #[test]
    fn iso_epoch_and_known_instants() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        // 2026-11-07T03:00:00Z == 1794020400 (the night this landed).
        assert_eq!(iso(1_794_020_400), "2026-11-07T03:00:00Z");
        assert_eq!(iso(1_794_020_400 + 2700), "2026-11-07T03:45:00Z");
    }

    #[test]
    fn iso_strings_sort_chronologically() {
        let mut v: Vec<String> = [100_000, 0, 1_794_020_400, 86_400, 999_999_999]
            .iter()
            .map(|n| iso(*n))
            .collect();
        let mut sorted = v.clone();
        sorted.sort();
        v.sort_by_key(|s| s.clone());
        assert_eq!(v, sorted);
        let mut by_str = vec![iso(200), iso(100), iso(300)];
        by_str.sort();
        assert_eq!(by_str, vec![iso(100), iso(200), iso(300)]);
    }

    #[test]
    fn wait_floors_match_broker_policy_8184() {
        // #8184: triage waits are held 6h, ci/capacity 1h; waits
        // with an owner-chosen horizon are unfloored.
        assert_eq!(wait_floor_seconds("triage"), Some(21_600));
        assert_eq!(wait_floor_seconds("ci"), Some(3_600));
        assert_eq!(wait_floor_seconds("capacity"), Some(3_600));
        assert_eq!(wait_floor_seconds("timer"), None);
        assert_eq!(wait_floor_seconds("review"), None);
        assert_eq!(wait_floor_seconds("external"), None);
    }

    #[test]
    fn apply_wait_floor_bumps_short_waits_only_8184() {
        // A long-past requested wake is bumped to the floor and
        // becomes relative; a far-future one stands as absolute.
        let (nb, kind) = apply_wait_floor("triage", "2000-01-01T00:00:00Z".to_string(), "absolute");
        assert_eq!(kind, "relative");
        assert!(nb > "2000-01-01T00:00:00Z".to_string());
        let (nb, kind) = apply_wait_floor("triage", "2999-01-01T00:00:00Z".to_string(), "absolute");
        assert_eq!(nb, "2999-01-01T00:00:00Z");
        assert_eq!(kind, "absolute");
        // Unfloored kinds never move.
        let (nb, kind) = apply_wait_floor("timer", "2000-01-01T00:00:00Z".to_string(), "absolute");
        assert_eq!(nb, "2000-01-01T00:00:00Z");
        assert_eq!(kind, "absolute");
    }

    #[test]
    fn default_pick_drains_parked_handoff_sink_8187() {
        // #8187 (amending #8184): the lane pick ADMITS recorded
        // opencode handoffs (the handoff sink) through a third
        // branch with NO not_before gate, so lanes can claim
        // stranded sink tasks and work them direct. #8234: the sink
        // is its own status ('handed_off'), so the timed parked
        // branch no longer needs its NOT sink exclusion — a sink
        // row cannot be 'parked' — and the sink branch keys on the
        // status, never on a reconstructed stage (#8192). The
        // queued branch keeps its attempts cap, and the sink branch
        // adds no placeholder: the predicate carries the two `now`
        // arguments the pick SELECT and claim UPDATE bind — plus,
        // since #3418, the claimer's declared remaining budget
        // twice (capacity bypass gate + outer budget gate).
        let pred = default_pick_predicate();
        assert!(pred.contains("status = 'queued' AND attempts < max_attempts"));
        assert!(
            pred.contains("status = 'parked' AND ((not_before IS NOT NULL AND not_before <= ?)")
        );
        assert!(
            !pred.contains("NOT ("),
            "no sink exclusion since #8234: {pred}"
        );
        assert!(pred.contains("status = 'handed_off'"));
        assert!(pred.contains("json_extract(progress, '$.lane') = 'opencode'"));
        assert!(
            !pred.contains("last_checkpoint"),
            "no stage reconstruction in the pick (#8192): {pred}"
        );
        // Third branch: the full sink predicate, ungated.
        let sink_branch = format!("OR ({HANDOFF_SQL})");
        assert!(pred.contains(&sink_branch), "sink branch present: {pred}");
        assert_eq!(pred.matches("not_before <= ?").count(), 2);
        assert_eq!(
            pred.matches('?').count(),
            4,
            "two `now`s + the two #3418 budget bindings: {pred}"
        );
        // And the claim UPDATE re-check carries the same sink branch,
        // and collecting a sink row costs no attempt (#8234).
        let upd = claim_update_sql(&pred);
        assert!(upd.contains(&sink_branch));
        assert!(upd.contains("status IN ('parked', 'handed_off')"));
        assert_eq!(upd.matches("not_before <= ?").count(), 2);
    }

    #[test]
    fn fresh_lane_assignment_defaults_direct_8187() {
        // #8187: no reason -> direct (the claiming session owns the
        // task); NO balancing, NO opencode routing by default.
        assert_eq!(lane_decision(None, None), ("direct", "default_direct"));
        // --exclusion-reason forces direct, as before.
        assert_eq!(
            lane_decision(Some("claude-only work"), None),
            ("direct", "exclusion")
        );
        // --opencode-reason is the only door to opencode, recorded.
        assert_eq!(
            lane_decision(None, Some("needs opencode session")),
            ("opencode", "opencode_reason")
        );
        // Both passed: exclusion (the must-NOT-go-to-opencode
        // contract) wins over the opencode reason.
        assert_eq!(
            lane_decision(Some("no opencode"), Some("wants opencode")),
            ("direct", "exclusion")
        );
    }

    #[test]
    fn lane_reassignment_flips_only_unbound_opencode_with_exclusion_8187() {
        // Sink/legacy opencode row + exclusion -> flip to direct.
        assert!(lane_reassignment_flip(
            "opencode",
            None,
            Some("lane takes it direct")
        ));
        assert!(lane_reassignment_flip("opencode", Some(""), Some("x")));
        // No exclusion: claimant flows keep their assignment.
        assert!(!lane_reassignment_flip("opencode", None, None));
        // A session-bound row NEVER flips, exclusion or not.
        assert!(!lane_reassignment_flip(
            "opencode",
            Some("ses_123"),
            Some("x")
        ));
        assert!(!lane_reassignment_flip("opencode", Some("ses_123"), None));
        // Direct rows are already direct; nothing to flip.
        assert!(!lane_reassignment_flip("direct", None, Some("x")));
        assert!(!lane_reassignment_flip("direct", None, None));
    }

    #[test]
    fn bound_session_treats_broker_cleared_as_unbound_8368() {
        // #8368: a ledger session_id the broker itself cleared
        // (progress.session_cleared names it) is not a live binding.
        assert_eq!(bound_session(None, None), None);
        assert_eq!(bound_session(Some(""), Some("ses_x")), None);
        assert_eq!(bound_session(Some("ses_dead"), Some("ses_dead")), None);
        assert_eq!(
            bound_session(Some("ses_live"), Some("ses_dead")),
            Some("ses_live")
        );
        assert_eq!(bound_session(Some("ses_live"), None), Some("ses_live"));
        // Composed with the flip predicates, as verb_lane calls them:
        // the cleared row flips; the live-bound row never does.
        assert!(lane_reassignment_flip(
            "opencode",
            bound_session(Some("ses_dead"), Some("ses_dead")),
            Some("lane takes it direct")
        ));
        assert!(!lane_reassignment_flip(
            "opencode",
            bound_session(Some("ses_live"), Some("ses_dead")),
            Some("lane takes it direct")
        ));
        assert!(lane_flip_to_opencode(
            "direct",
            bound_session(Some("ses_dead"), Some("ses_dead")),
            Some("send to opencode")
        ));
    }

    #[test]
    fn lane_and_park_honor_cleared_session_and_explicit_delay_8368() {
        // Source-shape companions to the predicate test above, in
        // this file's seg8217 style: verb_lane must consult the
        // task's session_cleared stamp and refuse live-bound flips
        // explicitly; verb_park's sink landing must be able to
        // carry an explicit caller delay as not_before.
        let src = &test_source();
        let lane_seg = seg8217(src, "fn verb_lane(", "\nfn verb_bind(");
        assert!(
            lane_seg.contains("cleared_session_id(&task)"),
            "verb_lane consults progress.session_cleared (#8368)"
        );
        assert!(
            lane_seg.contains("is bound to session"),
            "a live-bound flip refusal is an explicit error naming the session (#8368)"
        );
        let park_seg = seg8217(src, "fn verb_park(", "\nfn verb_release(");
        assert!(
            park_seg.contains("explicit_delay"),
            "the park verb tracks an explicit caller delay (#8368)"
        );
        assert!(
            park_seg.contains("not_before = ?, park_kind = NULL"),
            "the sink landing carries the explicit not_before (#8368)"
        );
    }

    #[test]
    fn lane_flip_to_opencode_only_unbound_direct_with_reason_8210() {
        // #8210: a claimed task's default direct ledger row flips
        // to opencode only with a stated reason and no bound
        // session — the send path's assignment step.
        assert!(lane_flip_to_opencode(
            "direct",
            None,
            Some("sender at time limit; opencode continues")
        ));
        assert!(lane_flip_to_opencode("direct", Some(""), Some("reason")));
        // No reason (or a blank one): no flip — opencode routing
        // stays reason-gated (#8187).
        assert!(!lane_flip_to_opencode("direct", None, None));
        assert!(!lane_flip_to_opencode("direct", None, Some("  ")));
        // A bound row never flips, either direction.
        assert!(!lane_flip_to_opencode(
            "direct",
            Some("ses_1"),
            Some("reason")
        ));
        // Opencode rows are not this flip's business.
        assert!(!lane_flip_to_opencode("opencode", None, Some("reason")));
    }

    fn transfer_row(status: &str, progress: Value, lease_until: &str) -> Value {
        let mut row = claimable_row(valid_payload());
        row["status"] = json!(status);
        row["progress"] = progress;
        row["lease_until"] = json!(lease_until);
        row["worker_id"] = json!("repo-worker-code-m");
        row["attempts"] = json!(2);
        row["max_attempts"] = json!(4);
        row
    }

    #[test]
    fn transfer_accepts_each_holder_session_mark_8210() {
        let now = "2026-11-10T20:00:00Z";
        let live = "2999-01-01T00:00:00Z";
        // Holder mark on the latest claimed event (a session that
        // claimed with --session-id).
        let row = transfer_row("claimed", json!({"lane": "direct"}), live);
        assert_eq!(
            transfer_refusal(
                &row,
                "ses_muse_1",
                Some("ses_muse_1"),
                None,
                Some("ses_new"),
                now
            ),
            None
        );
        // Holder mark checkpointed into progress.from_session.
        let row = transfer_row(
            "claimed",
            json!({"lane": "direct", "from_session": "ses_muse_2"}),
            live,
        );
        assert_eq!(
            transfer_refusal(&row, "ses_muse_2", None, None, Some("ses_new"), now),
            None
        );
        // A bound holder session passing its own task on: the
        // bound session IS the holder mark.
        let row = transfer_row(
            "claimed",
            json!({"lane": "opencode", "session_id": "ses_holder"}),
            live,
        );
        assert_eq!(
            transfer_refusal(
                &row,
                "ses_holder",
                None,
                Some("ses_holder"),
                Some("ses_new"),
                now
            ),
            None
        );
    }

    #[test]
    fn transfer_refuses_poach_terminal_parked_expired_and_bound_8210() {
        let now = "2026-11-10T20:00:00Z";
        let live = "2999-01-01T00:00:00Z";
        // A random session cannot poach a claimed task by id:
        // its id matches no holder mark.
        let row = transfer_row("claimed", json!({"lane": "direct"}), live);
        let r = transfer_refusal(
            &row,
            "ses_random",
            Some("ses_muse_1"),
            None,
            Some("ses_new"),
            now,
        )
        .expect("poach must refuse");
        assert!(r.contains("only the current holder"), "{r}");
        // No holder mark anywhere: nobody can give the task away.
        assert!(transfer_refusal(&row, "ses_muse_1", None, None, Some("ses_new"), now).is_some());
        // Terminal states refuse.
        for st in ["done", "cancelled", "failed"] {
            let row = transfer_row(st, json!({"from_session": "ses_m"}), live);
            assert!(
                transfer_refusal(&row, "ses_m", None, None, None, now).is_some(),
                "{st}"
            );
        }
        // Queued and parked rows are not transfer sources (a
        // genuine wait-park stays excluded, as by-id claim has it).
        for st in ["queued", "parked", "blocked"] {
            let row = transfer_row(st, json!({"from_session": "ses_m"}), live);
            assert!(
                transfer_refusal(&row, "ses_m", None, None, None, now).is_some(),
                "{st}"
            );
        }
        // A dead lease is not a live claim.
        let row = transfer_row(
            "claimed",
            json!({"from_session": "ses_m"}),
            "2000-01-01T00:00:00Z",
        );
        assert!(transfer_refusal(&row, "ses_m", None, None, None, now).is_some());
        // Bound to a third session (neither passer nor claimer):
        // a live launch, not a transfer.
        let row = transfer_row(
            "claimed",
            json!({"from_session": "ses_m", "session_id": "ses_third"}),
            live,
        );
        let r = transfer_refusal(&row, "ses_m", None, Some("ses_third"), Some("ses_new"), now)
            .expect("third-party binding must refuse");
        assert!(r.contains("already bound"), "{r}");
    }

    #[test]
    fn transfer_update_is_one_atomic_claimed_to_claimed_move_8210() {
        // #8210: the whole transfer is ONE conditional UPDATE —
        // status stays claimed, attempts untouched, and the WHERE
        // re-checks status + the holder worker so a lost race
        // affects 0 rows and the original claim stands.
        let sql = transfer_update_sql();
        assert!(sql.contains("status = 'claimed'"), "{sql}");
        assert!(sql.contains("worker_id = ?"), "{sql}");
        assert!(!sql.contains("attempts"), "{sql}");
        // Source oracles: the transfer path, its event marks, and
        // the client flag all exist.
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("claim_repo_task_transfer"));
        assert!(impl_src.contains("\"transfer\": true"));
        assert!(impl_src.contains("\"from_session\": from_session"));
        assert!(impl_src.contains("\"previous_worker\": old_worker"));
        let client_src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let client_src = client_src.as_str();
        assert!(client_src.contains("\"from-session\""));
        assert!(client_src.contains("body[\"from_session\"] = json!(fs)"));
    }

    #[test]
    fn platform_limit_rules_8210() {
        // No limitation: open default, either family may claim,
        // and nothing is invalid or refused.
        assert!(!platform_limit_invalid(None, None));
        assert_eq!(platform_limit_error(None, None), None);
        assert_eq!(platform_limit_refusal(None, None, Some("muse")), None);
        assert_eq!(
            platform_limit_refusal(None, None, Some("openchamber")),
            None
        );
        assert_eq!(platform_limit_refusal(None, None, None), None);
        // Recording one demands a real family AND a reason.
        assert!(platform_limit_error(Some("muse"), Some("needs claude-only context")).is_none());
        let e = platform_limit_error(Some("muse"), None)
            .expect("a limitation without a reason is refused at the verb");
        assert!(e.contains("requires a recorded reason"), "{e}");
        assert!(platform_limit_error(Some("muse"), Some("  ")).is_some());
        assert!(platform_limit_error(Some("bogus"), Some("reason")).is_some());
        assert!(platform_limit_error(None, Some("orphan reason")).is_some());
        // A stored limitation with no reason is invalid: never
        // enforced (the transfer path clears it instead).
        assert!(platform_limit_invalid(Some("muse"), None));
        assert!(platform_limit_invalid(Some("muse"), Some(" ")));
        assert!(!platform_limit_invalid(Some("muse"), Some("real reason")));
        assert_eq!(
            platform_limit_refusal(Some("muse"), None, Some("openchamber")),
            None
        );
        // A valid limitation binds transfers: the limited family
        // passes, every other family refuses — naming the
        // limitation and its reason — and an undeclared family
        // cannot take a limited task.
        let reason = Some("needs claude-only context window");
        assert_eq!(
            platform_limit_refusal(Some("muse"), reason, Some("muse")),
            None
        );
        let r = platform_limit_refusal(Some("muse"), reason, Some("openchamber"))
            .expect("muse-limited task refuses an opencode transfer");
        assert!(r.contains("limited to platform 'muse'"), "{r}");
        assert!(r.contains("needs claude-only context window"), "{r}");
        let r = platform_limit_refusal(Some("openchamber"), reason, Some("muse"))
            .expect("opencode-limited task refuses a muse transfer");
        assert!(r.contains("limited to platform 'openchamber'"), "{r}");
        assert!(platform_limit_refusal(Some("muse"), reason, None).is_some());
        // Source oracles: columns, migration, event fields, and
        // the client flags all exist.
        assert!(LEDGER_COLS.contains("platform_limit"));
        assert!(LEDGER_COLS.contains("platform_limit_reason"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("platform_limit TEXT"));
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("ADD COLUMN {col} TEXT"));
        assert!(impl_src.contains("platform_limit_cleared_invalid"));
        assert!(impl_src.contains("\"platform_limit_cleared\": true"));
        let client_src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let client_src = client_src.as_str();
        assert!(client_src.contains("\"limit-platform\""));
        assert!(client_src.contains("\"platform_limit\": args.get(\"limit-platform\")"));
        assert!(client_src.contains("\"clear-platform-limit\""));
    }

    #[test]
    fn opencode_reason_is_recorded_on_the_ledger_8187() {
        // Source oracle: the reason column exists on the read
        // surface, the fresh schema, the additive migration, and
        // the fresh-assignment INSERT; the client exposes the flag.
        assert!(LEDGER_COLS.contains("opencode_reason"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("opencode_reason TEXT"));
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(
            impl_src.contains("ALTER TABLE repo_task_lane_ledger ADD COLUMN opencode_reason TEXT")
        );
        assert!(impl_src.contains("assigned_lane, exclusion_reason, opencode_reason, assigned_by"));
        assert!(impl_src.contains("\"reason\": reason"));
        assert!(impl_src.contains("\"reassignment\": true"));
        // No balancing artifact survives in the lane decision path.
        assert!(!impl_src.contains("opencode_count"));
        assert!(!impl_src.contains("o_count <= d_count"));
        let client_src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let client_src = client_src.as_str();
        assert!(client_src.contains("\"opencode-reason\""));
        assert!(client_src.contains("\"opencode_reason\": args.get(\"opencode-reason\")"));
    }

    #[test]
    fn platform_is_a_mark_not_a_lane_input_8187() {
        // #8187 platform audit: tasks.platform holds only the two
        // platform marks and lane_decision takes no platform input
        // (its signature is exclusion + opencode_reason only), so a
        // platform selector can never route a task to opencode.
        assert_eq!(PLATFORM_FAMILIES, ["openchamber", "muse"]);
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        let lane_fn = impl_src
            .split("fn verb_lane")
            .nth(1)
            .expect("verb_lane exists")
            .split("fn verb_bind")
            .next()
            .unwrap_or("");
        // #8210 amendment: verb_lane now also records a platform
        // LIMITATION (platform_limit / platform_limit_reason
        // request fields, written to the lane ledger with their
        // recorded reason) — that is not a read of the mark. The
        // invariant that stands: verb_lane never reads the task
        // row's platform FIELD (the mark) as an input.
        assert!(
            !lane_fn.contains("get(\"platform\")"),
            "verb_lane must not read tasks.platform: {lane_fn}"
        );
    }

    #[test]
    fn expired_claims_return_handoffs_to_sink_only_8184() {
        // #8184: expiry of a claimed recorded handoff returns to
        // the parked sink (no attempt burn); anything else queues.
        assert!(expired_claim_returns_to_sink(
            &json!({"lane": "opencode", "stage": "handed_off"})
        ));
        assert!(expired_claim_returns_to_sink(
            &json!({"lane": "opencode", "stage": "launch_failed"})
        ));
        assert!(expired_claim_returns_to_sink(&json!({
            "lane": "opencode",
            "last_checkpoint": {"stage": "handed_off"}
        })));
        assert!(!expired_claim_returns_to_sink(
            &json!({"lane": "direct", "stage": "handed_off"})
        ));
        assert!(!expired_claim_returns_to_sink(&json!({
            "lane": "opencode",
            "stage": "implementation_complete"
        })));
        assert!(!expired_claim_returns_to_sink(&json!({})));
    }

    #[test]
    fn wake_eligibility_is_parked_only_8184() {
        assert!(wake_eligible("parked").is_ok());
        assert!(wake_eligible("queued").is_err());
        assert!(wake_eligible("claimed").is_err());
        assert!(wake_eligible("done").is_err());
        // #8234: a sink row is refused wake with the reason — it is
        // not a wait at all, it is already claimable.
        let err = wake_eligible("handed_off").unwrap_err();
        assert!(err.contains("already claimable"), "{err}");
    }

    #[test]
    fn expiry_disposition_fails_only_at_cap() {
        // Below cap: the task returns to queued for another attempt.
        assert_eq!(expiry_disposition(0, 4), ExpiryDisposition::Requeue);
        assert_eq!(expiry_disposition(3, 4), ExpiryDisposition::Requeue);
        // At or past cap (#6463): terminal failure, never a silent
        // return to a queued state the claim query cannot select.
        assert_eq!(expiry_disposition(4, 4), ExpiryDisposition::FailAtCap);
        assert_eq!(expiry_disposition(24, 24), ExpiryDisposition::FailAtCap);
        assert_eq!(expiry_disposition(9, 4), ExpiryDisposition::FailAtCap);
    }

    #[test]
    fn wait_kind_mapping_matches_pg_park() {
        assert_eq!(wait_kind(&json!({"kind": "triage"})), "triage");
        assert_eq!(wait_kind(&json!({"kind": "time"})), "timer");
        assert_eq!(wait_kind(&json!({"kind": "pr_merged"})), "ci");
        assert_eq!(wait_kind(&json!({"kind": "person_answer"})), "review");
        assert_eq!(wait_kind(&json!({"kind": "quota_reset"})), "capacity");
        assert_eq!(wait_kind(&json!({"kind": "something_else"})), "external");
        assert_eq!(wait_kind(&json!({})), "external");
    }

    #[test]
    fn clear_binding_stamps_and_strips() {
        let p = json!({"session_id": "s1", "launched_by": "x", "keep": 1});
        let (out, cleared) = clear_binding(&p, "expire_leases");
        assert_eq!(cleared.as_deref(), Some("s1"));
        assert!(out.get("session_id").is_none());
        assert!(out.get("launched_by").is_none());
        assert_eq!(out.get("keep"), Some(&json!(1)));
        assert_eq!(
            out.get("session_cleared").and_then(|c| c.get("via")),
            Some(&json!("expire_leases"))
        );
        let (out2, cleared2) = clear_binding(&json!({"keep": 1}), "x");
        assert!(cleared2.is_none());
        assert_eq!(out2, json!({"keep": 1}));
    }

    #[test]
    fn event_append_requires_a_kind() {
        assert!(parse_event_append(&json!({})).is_err());
        assert!(parse_event_append(&json!({"kind": "  "})).is_err());
        let (w, tid, kind, detail) =
            parse_event_append(&json!({"kind": "push", "worker_id": "github-webhook"})).unwrap();
        assert_eq!(kind, "push");
        assert_eq!(w.as_deref(), Some("github-webhook"));
        assert_eq!(tid, None);
        assert_eq!(detail, json!({}));
        let (_, tid2, _, _) =
            parse_event_append(&json!({"kind": "claimed", "task_id": 42, "detail": {"a": 1}}))
                .unwrap();
        assert_eq!(tid2, Some(42));
    }

    #[test]
    fn prune_cutoff_clamps_and_subtracts_days() {
        let now = 1_794_020_400; // 2026-11-07T03:00:00Z
        assert_eq!(prune_cutoff(now, 30), iso(now - 30 * 86_400));
        assert_eq!(prune_cutoff(now, 0), iso(now - 86_400));
        assert_eq!(prune_cutoff(now, 99_999), iso(now - 3650 * 86_400));
    }

    #[test]
    fn revalidate_outcome_matrix() {
        // Time parks are never released by the sweeper: elapsed time
        // is not evidence; they release through claim at not_before.
        assert_eq!(
            revalidate_outcome(Some("time"), None, None, false),
            "confirmed"
        );
        // No condition, or a kind the database cannot verify: stale,
        // so the park surfaces instead of sitting silently.
        assert_eq!(revalidate_outcome(None, None, None, false), "stale");
        assert_eq!(
            revalidate_outcome(Some("issue_closed"), None, None, false),
            "stale"
        );
        assert_eq!(
            revalidate_outcome(Some("person_answer"), None, None, false),
            "stale"
        );
        // task_terminal releases on done/cancelled only.
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), Some("done"), None, false),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), Some("cancelled"), None, false),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), Some("queued"), None, false),
            "confirmed"
        );
        assert_eq!(
            revalidate_outcome(Some("task_terminal"), None, None, false),
            "stale"
        );
        // task_status releases on the exact wanted status.
        assert_eq!(
            revalidate_outcome(Some("task_status"), Some("blocked"), Some("blocked"), false),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("task_status"), Some("queued"), Some("blocked"), false),
            "confirmed"
        );
        assert_eq!(
            revalidate_outcome(Some("task_status"), None, Some("blocked"), false),
            "stale"
        );
        // event_seen releases when the event exists for the task.
        assert_eq!(
            revalidate_outcome(Some("event_seen"), None, None, true),
            "released"
        );
        assert_eq!(
            revalidate_outcome(Some("event_seen"), None, None, false),
            "confirmed"
        );
    }

    #[test]
    fn dedupe_partition_live_hit_wins_and_terminal_is_newest() {
        let rows = vec![
            json!({"id": 1, "status": "done"}),
            json!({"id": 2, "status": "cancelled"}),
        ];
        let (live, terminal) = dedupe_partition(&rows);
        assert!(live.is_none());
        // The newest terminal row is the revive predecessor.
        assert_eq!(terminal.unwrap().get("id"), Some(&json!(2)));
        let rows = vec![
            json!({"id": 1, "status": "done"}),
            json!({"id": 3, "status": "parked"}),
            json!({"id": 4, "status": "failed"}),
        ];
        let (live, terminal) = dedupe_partition(&rows);
        assert_eq!(live.unwrap().get("id"), Some(&json!(3)));
        assert_eq!(terminal.unwrap().get("id"), Some(&json!(4)));
        let (live, terminal) = dedupe_partition(&[]);
        assert!(live.is_none() && terminal.is_none());
    }

    #[test]
    fn revive_eligibility_matches_6838() {
        // Failed rows can always be re-queued.
        assert!(revive_eligible("failed", 4, 4).is_ok());
        assert!(revive_eligible("failed", 1, 4).is_ok());
        // Queued rows only when dead at cap (the #6463 zombie state).
        assert!(revive_eligible("queued", 4, 4).is_ok());
        assert!(revive_eligible("queued", 3, 4).is_err());
        // Live work and done work are untouchable.
        assert!(revive_eligible("claimed", 4, 4).is_err());
        assert!(revive_eligible("done", 4, 4).is_err());
        assert!(revive_eligible("cancelled", 4, 4).is_err());
    }

    // ---- #8276 dependency blocking --------------------------------------

    #[test]
    fn dep_clamps_match_broker_policy_8276() {
        // Recheck: default 15 min, clamped to [5 min, 1 h].
        assert_eq!(clamp_dep_recheck_seconds(None), 900);
        assert_eq!(clamp_dep_recheck_seconds(Some(10)), 300);
        assert_eq!(clamp_dep_recheck_seconds(Some(1800)), 1800);
        assert_eq!(clamp_dep_recheck_seconds(Some(7200)), 3600);
        // Max TTL: default 3 days, floor 60 s (the live-demo
        // range), ceiling 7 days — nothing sits blocked forever.
        assert_eq!(clamp_dep_ttl_seconds(None), 259_200);
        assert_eq!(clamp_dep_ttl_seconds(Some(5)), 60);
        assert_eq!(clamp_dep_ttl_seconds(Some(3600)), 3600);
        assert_eq!(clamp_dep_ttl_seconds(Some(999_999_999)), 604_800);
    }

    #[test]
    fn dep_parse_validates_refs_8276() {
        let now = "2026-11-10T21:00:00Z";
        // No deps key (or an empty array) is the classic block.
        assert_eq!(parse_block_deps(&json!({}), "w", now).unwrap(), None);
        assert_eq!(
            parse_block_deps(&json!({"deps": []}), "w", now).unwrap(),
            None
        );
        let deps = parse_block_deps(
            &json!({"deps": [
                {"kind": "pr", "repo": "developing-today/code", "number": 3931, "reason": "gate"},
                {"kind": "issue", "repo": "developing-today/code", "number": 3901},
                {"kind": "task", "task_id": 5865},
                {"kind": "ci", "repo": "developing-today/code", "run_id": 12345},
            ]}),
            "lane-x",
            now,
        )
        .unwrap()
        .unwrap();
        assert_eq!(deps.len(), 4);
        assert_eq!(deps[0]["state"], json!("open"));
        assert_eq!(deps[0]["recorded_by"], json!("lane-x"));
        assert_eq!(deps[0]["reason"], json!("gate"));
        assert_eq!(dep_key(&deps[0]), "pr:developing-today/code#3931");
        assert_eq!(dep_key(&deps[1]), "issue:developing-today/code#3901");
        assert_eq!(dep_key(&deps[2]), "task:5865");
        assert_eq!(dep_key(&deps[3]), "ci:developing-today/code#12345");
        // A dep the broker cannot watch is a block-time error.
        assert!(
            parse_block_deps(
                &json!({"deps": [{"kind": "pr", "repo": "noslash", "number": 1}]}),
                "w",
                now,
            )
            .is_err()
        );
        assert!(
            parse_block_deps(
                &json!({"deps": [{"kind": "pr", "repo": "a/b", "number": 0}]}),
                "w",
                now,
            )
            .is_err()
        );
        assert!(
            parse_block_deps(
                &json!({"deps": [{"kind": "task", "task_id": -3}]}),
                "w",
                now,
            )
            .is_err()
        );
        assert!(
            parse_block_deps(&json!({"deps": [{"kind": "ci", "repo": "a/b"}]}), "w", now,).is_err()
        );
        assert!(
            parse_block_deps(
                &json!({"deps": [{"kind": "milestone", "number": 1}]}),
                "w",
                now,
            )
            .is_err()
        );
        assert!(parse_block_deps(&json!({"deps": "pr"}), "w", now).is_err());
    }

    #[test]
    fn task_dep_obs_matrix_8276() {
        // done clears; cancelled/failed release with the outcome
        // named; anything live holds; a missing task is broken.
        assert_eq!(
            task_dep_obs(Some("done"), 5),
            DepObs::Cleared("dep task done".to_string())
        );
        assert!(
            matches!(task_dep_obs(Some("cancelled"), 5), DepObs::Cleared(ref e) if e.contains("cancelled"))
        );
        assert!(
            matches!(task_dep_obs(Some("failed"), 5), DepObs::Cleared(ref e) if e.contains("failed"))
        );
        assert!(matches!(task_dep_obs(Some("claimed"), 5), DepObs::Open(_)));
        assert!(matches!(task_dep_obs(Some("queued"), 5), DepObs::Open(_)));
        assert!(matches!(task_dep_obs(Some("blocked"), 5), DepObs::Open(_)));
        assert!(matches!(task_dep_obs(None, 5), DepObs::Broken(_)));
    }

    #[test]
    fn dep_obs_transitions_and_all_must_clear_8276() {
        let now = "2026-11-10T21:00:00Z";
        let mut d = json!({"kind": "pr", "state": "open"});
        // A failed check (rate limit, token down) is NOT
        // clearance: no transition, dep stays open, the failure
        // is recorded as the last observation.
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Failed("github http 403".to_string()), now),
            None
        );
        assert_eq!(d["state"], json!("open"));
        assert_eq!(d["last_observed"], json!("github http 403"));
        // Still-open observation: no transition either.
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Open("pr open".to_string()), now),
            None
        );
        assert_eq!(d["state"], json!("open"));
        // Cleared transitions exactly once; a resolved dep is
        // terminal — later observations are no-ops.
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Cleared("pr merged".to_string()), now),
            Some("cleared")
        );
        assert_eq!(d["state"], json!("cleared"));
        assert_eq!(
            apply_dep_obs(&mut d, &DepObs::Open("pr open".to_string()), now),
            None
        );
        assert_eq!(d["state"], json!("cleared"));
        // Broken releases too — a typo'd gate is not a life
        // sentence.
        let mut b = json!({"kind": "issue", "state": "open"});
        assert_eq!(
            apply_dep_obs(&mut b, &DepObs::Broken("github 404".to_string()), now),
            Some("broken")
        );
        // All-must-clear: one open dep holds the whole block.
        assert!(deps_all_resolved(&[d.clone(), b.clone()]));
        let mut held = vec![d.clone(), b.clone()];
        held.push(json!({"kind": "task", "state": "open"}));
        assert!(!deps_all_resolved(&held));
        assert!(!deps_all_resolved(&[]));
    }

    #[test]
    fn dep_wiring_oracles_8276() {
        // Schema, read surface, additive migration.
        assert!(TASK_COLS.contains("blocked_deps"));
        assert!(TASK_COLS.contains("dep_blocked_until"));
        assert!(TASK_COLS.contains("dep_next_recheck_at"));
        assert!(TASK_JSON_COLS.iter().any(|c| *c == "blocked_deps"));
        let schema = SCHEMA.join(" ");
        assert!(schema.contains("blocked_deps TEXT"));
        assert!(schema.contains("dep_recheck_seconds INTEGER"));
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("ALTER TABLE tasks ADD COLUMN {col} {ty}"));
        // Routes, sweep ride, audit kinds, health bucket.
        assert!(impl_src.contains("\"/dep-recheck\" => verb_dep_recheck"));
        assert!(impl_src.contains("\"/unblock\" => verb_unblock"));
        // The sweep ride calls run_dep_rechecks with the sweep's own
        // "dep_limit" key; rustfmt splits that call across lines, so
        // the oracle matches on whitespace-flattened source. The
        // /dep-recheck verb's own call (client sends "limit") is
        // asserted separately below.
        let impl_flat = impl_src.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(impl_flat.contains("run_dep_rechecks( t, req_i64(req, \"dep_limit\")"));
        assert!(impl_src.contains("run_dep_rechecks(t, req_i64(req, \"limit\")"));
        assert!(impl_src.contains("\"dep_ttl_expired\""));
        assert!(impl_src.contains("\"dep_unblocked\""));
        assert!(impl_src.contains("\"dep_unblock_override\""));
        assert!(impl_src.contains("\"dep_blocked\": 0"));
        assert!(impl_src.contains("Some(until) if until < next.as_str()"));
        // The default pick has no blocked branch: dep-blocked
        // tasks (status 'blocked') are invisible to claims, so
        // no lane ever re-verifies a gate by claiming the item.
        let pred = default_pick_predicate();
        assert!(
            !pred.contains("blocked"),
            "pick must exclude blocked: {pred}"
        );
        // Client surface.
        let client_src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let client_src = client_src.as_str();
        for f in [
            "blocked-by-pr",
            "blocked-by-issue",
            "blocked-by-task",
            "blocked-by-ci",
            "dep-recheck-seconds",
            "dep-max-ttl-seconds",
        ] {
            assert!(client_src.contains(f), "client flag {f}");
        }
        assert!(client_src.contains("\"dep-recheck\" => cmd_dep_recheck"));
        assert!(client_src.contains("\"unblock\" => cmd_unblock"));
        // The production probe is wired where the broker builds
        // its Turso backend, authenticated by secret name.
        let broker_src = include_str!("queue_broker.rs");
        assert!(broker_src.contains("GhDepProbe"));
        assert!(broker_src.contains("set_gh_probe"));
        assert!(broker_src.contains("github.developing-today-code-automation"));
    }

    #[test]
    fn dep_drop_12777() {
        // Oracle (#12777): the surgical unblock. A stored dep
        // record drops exactly the entries a spec names — kind
        // is part of the identity, so an issue spec never eats
        // a pr record with the same repo and number — and a
        // spec that matches nothing fails the whole correction.
        let issue_dep = json!({"kind": "issue", "repo": "developing-today/code",
            "number": 9513, "state": "open", "reason": "gate"});
        let pr_dep = json!({"kind": "pr", "repo": "developing-today/code",
            "number": 9513, "state": "open"});
        let self_loop = json!({"kind": "task", "task_id": 20408, "state": "open"});
        let ci_dep = json!({"kind": "ci", "repo": "developing-today/code",
            "run_id": 777, "state": "open"});
        let deps = vec![
            self_loop.clone(),
            issue_dep.clone(),
            pr_dep.clone(),
            ci_dep.clone(),
        ];
        // The #12777 case: drop the bogus self-loop entry, keep
        // the real deps watched, order preserved.
        let (remaining, dropped) = remove_deps(&deps, &[json!({"kind": "task", "task_id": 20408})])
            .expect("the self-loop spec matches its record");
        assert_eq!(dropped, vec![self_loop.clone()]);
        assert_eq!(
            remaining,
            vec![issue_dep.clone(), pr_dep.clone(), ci_dep.clone()]
        );
        // Kind is part of the identity.
        let (remaining, dropped) = remove_deps(
            &deps,
            &[json!({"kind": "issue", "repo": "developing-today/code", "number": 9513})],
        )
        .expect("the issue spec matches its record");
        assert_eq!(dropped, vec![issue_dep]);
        assert!(remaining.contains(&pr_dep));
        // CI deps match on run_id, not number.
        let (_, dropped) = remove_deps(
            &deps,
            &[json!({"kind": "ci", "repo": "developing-today/code", "run_id": 777})],
        )
        .expect("the ci spec matches its record");
        assert_eq!(dropped, vec![ci_dep]);
        // Dropping every entry empties the record — the broker
        // releases the task on exactly this outcome.
        let (remaining, dropped) = remove_deps(
            &deps,
            &[
                json!({"kind": "task", "task_id": 20408}),
                json!({"kind": "issue", "repo": "developing-today/code", "number": 9513}),
                json!({"kind": "pr", "repo": "developing-today/code", "number": 9513}),
                json!({"kind": "ci", "repo": "developing-today/code", "run_id": 777}),
            ],
        )
        .expect("all four specs match");
        assert!(remaining.is_empty());
        assert_eq!(dropped.len(), 4);
        // A spec that matches nothing fails the whole correction
        // and names itself for the refusal message.
        let err = remove_deps(
            &deps,
            &[json!({"kind": "issue", "repo": "developing-today/code", "number": 9999})],
        )
        .unwrap_err();
        assert_eq!(err.len(), 1);
        assert_eq!(dep_label(&err[0]), "issue developing-today/code#9999");
        assert_eq!(dep_label(&self_loop), "task 20408");
        // parse_drop_deps: the broker-side validation of the
        // wire shape — strict refs, identity-only specs, and
        // None for the classic full override.
        assert_eq!(parse_drop_deps(&json!({})).expect("no field"), None);
        assert_eq!(
            parse_drop_deps(&json!({"drop_deps": []})).expect("empty list"),
            None
        );
        assert_eq!(
            parse_drop_deps(&json!({"drop_deps": [{"kind": "task", "task_id": 20408}]}))
                .expect("task spec"),
            Some(vec![json!({"kind": "task", "task_id": 20408})])
        );
        assert!(parse_drop_deps(&json!({"drop_deps": "task:20408"})).is_err());
        assert!(
            parse_drop_deps(&json!({"drop_deps": [{"kind": "issue", "repo": "bad", "number": 1}]}))
                .is_err()
        );
        assert!(parse_drop_deps(&json!({"drop_deps": [{"kind": "task", "task_id": 0}]})).is_err());
        assert!(parse_drop_deps(&json!({"drop_deps": [{"kind": "bogus", "task_id": 1}]})).is_err());
        // Wiring: the client flag and parser, the broker's
        // audit kind for a partial drop.
        let client_src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        assert!(client_src.contains("drop-dep"), "client flag drop-dep");
        assert!(client_src.contains("parse_drop_dep_list"));
        assert!(client_src.contains("\"drop_deps\""));
        let src = test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("\"dep_entry_dropped\""));
        assert!(impl_src.contains("parse_drop_deps"));
        assert!(impl_src.contains("unblock_drop"));
    }

    #[test]
    fn quarantine_atomic_sql_10671() {
        // Oracle: synthetic interleavings execute the exact production SQL
        // against SQLite, not a mocked affected-row count or live broker.
        let output = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/quarantine_atomic_sql.py"
            ))
            .output()
            .expect("quarantine SQL tests require Python 3 with stdlib sqlite3");
        assert!(
            output.status.success(),
            "quarantine SQL regression failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn claim_quarantine_budget_is_separate_from_race_budget_9609() {
        // #9609 oracle: quarantines in the claim pick loop carry
        // their own budget (MAX_CLAIM_QUARANTINES_PER_CALL), sized
        // to chew through a defective intake wave in one call; the
        // 32-cap bounds race losses only. Under the shared cap,
        // the 2026-11-16 wave (~460 defective packets at one
        // queue's head) made every claim call return "no claimable
        // task" behind the wave for a whole shift.
        let src = &test_source();
        assert!(
            src.contains("pub const MAX_CLAIM_QUARANTINES_PER_CALL: usize = 512;"),
            "quarantine budget constant"
        );
        let pick = seg8217(src, "fn claim_pick_once(", "\nfn parse_batch_count(");
        assert!(
            pick.contains("quarantined += 1;"),
            "quarantine branch counts quarantines"
        );
        assert!(
            pick.contains("quarantined >= MAX_CLAIM_QUARANTINES_PER_CALL"),
            "quarantine branch bounded by the quarantine budget"
        );
        assert_eq!(
            pick.matches("skipped += 1;").count(),
            1,
            "only the race-loss branch consumes the race budget"
        );
        assert!(pick.contains("skipped >= 32"), "race budget unchanged");
    }

    #[test]
    fn quarantine_recheck_rotates_its_window_9609() {
        // #9609 oracle: rows the rechecker evaluates and leaves
        // blocked without another write get the rotation touch, so
        // the sweep's ORDER BY updated_at ASC window advances
        // through the whole quarantined population instead of
        // rechecking the same front rows every pass (the live
        // 2026-11-16 fossilization: 197 rows rechecked minutely,
        // ~1,490 never reached).
        let src = &test_source();
        let one = seg8217(
            src,
            "fn quarantine_recheck_one(",
            "\npub fn run_quarantine_rechecks(",
        );
        assert_eq!(
            one.matches("touch_recheck_cursor(t, id)?;").count(),
            4,
            "every no-write outcome rotates, a raced guarded write included (#10671)"
        );
        assert_eq!(
            one.matches("return Ok(\"release_raced\");").count(),
            2,
            "both guarded writes report a race"
        );
        for raced in one.split("return Ok(\"release_raced\");").take(2) {
            assert!(
                raced.trim_end().ends_with("touch_recheck_cursor(t, id)?;"),
                "a raced guarded write rotates before it returns (#10671)"
            );
        }
        let touch = seg8217(
            src,
            "fn touch_recheck_cursor(",
            "\npub(crate) fn quarantine_recheck_one(",
        );
        assert!(
            touch.contains("SET updated_at = ?"),
            "touch bumps updated_at only"
        );
        assert!(!touch.contains("event("), "touch writes no event");
        let run = seg8217(
            src,
            "pub fn run_quarantine_rechecks(",
            "\npub(crate) fn verb_quarantine_recheck(",
        );
        assert!(
            run.contains("ORDER BY updated_at ASC"),
            "window order is the rotation cursor"
        );
    }

    #[test]
    fn enqueue_rejects_packet_missing_payload_dedupe_9609() {
        // #9609 oracle: the enqueue boundary rejects the exact
        // shape the 2026-11-16 bulk intake wrote directly to the
        // DB — a repo-task/v1 packet with no payload.dedupe
        // object. Validation runs before any store access, so the
        // dummy backend is never touched on this path.
        let backend = TursoBackend::new(
            "http://127.0.0.1:0".to_string(),
            "test".to_string(),
            ureq::agent(),
        );
        let packet = json!({
            "title": "probe", "priority": 30,
            "dedupe_key": "repo-health:v1:triage:developing-today/code:issue:9609",
            "payload": {
                "schema": "repo-task/v1",
                "kind": "triage",
                "repo": "developing-today/code",
                "problem": "p",
                "priority_reason": "r",
                "evidence": ["e"],
                "acceptance": ["a"],
                "tests": ["t"],
                "context_refs": [],
                "scope": {"include": ["x"], "exclude": ["y"] },
                "risk": {"level": "low", "flags": [] },
                "timebox_minutes": 20,
                "issue": {"number": 9609 }
            }
        });
        let req = json!({
            "queue": "repo-health-code-pilot",
            "packet": packet,
            "dry_run": true
        });
        let out = verb_enqueue(&backend, &req);
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["exit"], 2, "{out}");
        assert!(
            out["errors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e.as_str().unwrap_or("").contains("payload.dedupe")),
            "{out}"
        );
    }

    #[test]
    fn quarantine_recheck_verdicts_10648() {
        // Oracle: the 2026-11-16 blocked-pile audit. A blocked row
        // whose recorded claim-path quarantine no longer reproduces
        // must RELEASE; a row still failing the guard stays, with
        // the current reason; rows that are not quarantine fossils
        // are never this pass's business.
        let recorded = "quarantined by claim path: packet failed validation \
                        (#5444): packet.max_attempts: must match payload.max_attempts";
        // The incident fossil itself: row copy 1000, payload copy
        // 3, recorded error the split-copy mismatch. Under the old
        // agreement rule the guard still fired; under the
        // single-source-of-truth rule (#10648) the packet is valid
        // and the fossil must release.
        let mut fossil = claimable_row(valid_payload());
        fossil["payload"]["max_attempts"] = json!(3);
        fossil["max_attempts"] = json!(1000);
        fossil["status"] = json!("blocked");
        fossil["last_error"] = json!(recorded);
        assert_eq!(
            quarantine_recheck_verdict(&fossil),
            QuarantineRecheck::Release
        );
        // Still invalid (schema broken): stays quarantined, and the
        // verdict carries today's reason, not the recorded one.
        let mut still_bad = fossil.clone();
        still_bad["payload"]["schema"] = json!("repo-task/v0");
        match quarantine_recheck_verdict(&still_bad) {
            QuarantineRecheck::StillQuarantined(reason) => {
                assert!(reason.contains("packet failed validation"), "{reason}");
            }
            other => panic!("expected StillQuarantined, got {other:?}"),
        }
        // Payload-less fossil: the missing-payload quarantine also
        // still reproduces.
        let mut no_payload = fossil.clone();
        no_payload["payload"] = json!({});
        no_payload["last_error"] =
            json!("quarantined by claim path: payload missing/empty (#5444)");
        match quarantine_recheck_verdict(&no_payload) {
            QuarantineRecheck::StillQuarantined(reason) => {
                assert!(reason.contains("payload missing/empty"), "{reason}");
            }
            other => panic!("expected StillQuarantined, got {other:?}"),
        }
        // Not this pass's business: a dep-blocked row (the dep
        // rechecker owns it, even under a stale quarantine error),
        // a classically blocked row, and a non-blocked row.
        let mut dep_row = fossil.clone();
        dep_row["blocked_deps"] =
            json!([{"kind": "issue", "repo": "developing-today/code", "number": 5444}]);
        assert_eq!(
            quarantine_recheck_verdict(&dep_row),
            QuarantineRecheck::NotApplicable
        );
        let mut classic = fossil.clone();
        classic["last_error"] = json!("Drew decision pending on the linked issue");
        assert_eq!(
            quarantine_recheck_verdict(&classic),
            QuarantineRecheck::NotApplicable
        );
        let mut queued = fossil.clone();
        queued["status"] = json!("queued");
        assert_eq!(
            quarantine_recheck_verdict(&queued),
            QuarantineRecheck::NotApplicable
        );
        // #10714: a row a LANE blocked afterwards is never released by
        // this pass, even when its last_error is the fossil quarantine
        // error. verb_block records blocker/reason_class (and, since
        // this fix, blocked_via=lane) in progress and result.
        let mut lane_block = fossil.clone();
        lane_block["result"] = json!({
            "blocker": "escalated to Drew", "reason_class": "escalated-to-user",
            "blocked_via": "lane",
        });
        assert_eq!(
            quarantine_recheck_verdict(&lane_block),
            QuarantineRecheck::NotApplicable
        );
        // Legacy lane blocks predate the marker; blocker/reason_class
        // alone must still protect them.
        let mut legacy_lane_block = fossil.clone();
        legacy_lane_block["progress"] = json!({
            "blocker": "CI red on PR 1", "reason_class": "blocked",
        });
        assert_eq!(
            quarantine_recheck_verdict(&legacy_lane_block),
            QuarantineRecheck::NotApplicable
        );
    }

    #[test]
    fn verb_block_clears_stale_quarantine_error_10714() {
        // Oracle: the SQL verb_block runs. A lane block must clear the
        // fossil quarantine last_error (so the rechecker's prefix
        // selection cannot see the row) and stamp the lane marker.
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        let body = impl_src
            .split("fn verb_block(")
            .nth(1)
            .and_then(|s| s.split("\nfn ").next())
            .unwrap_or("");
        assert!(!body.is_empty(), "verb_block body found");
        assert!(
            body.contains("last_error = NULL"),
            "verb_block must clear the stale quarantine last_error"
        );
        assert!(
            body.contains("\"blocked_via\": \"lane\""),
            "verb_block must stamp the lane block marker"
        );
    }

    #[test]
    fn quarantine_wiring_oracles_10648() {
        // Routes, sweep ride, client surface — the same wiring
        // shape the dep rechecker established in #8276.
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(impl_src.contains("\"/quarantine-recheck\" => verb_quarantine_recheck"));
        let impl_flat = impl_src.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            impl_flat.contains("run_quarantine_rechecks( t, req_i64(req, \"quarantine_limit\")")
        );
        assert!(impl_src.contains("run_quarantine_rechecks(t, req_i64(req, \"limit\")"));
        assert!(impl_src.contains("\"via\": \"quarantine_recheck\""));
        let client_src = [
            include_str!("client/transport.rs"),
            include_str!("client/support.rs"),
            include_str!("client/commands_query.rs"),
            include_str!("client/commands_lifecycle.rs"),
            include_str!("client/fallback_stack.rs"),
            include_str!("client.rs"),
        ]
        .concat();
        let client_src = client_src.as_str();
        assert!(client_src.contains("\"quarantine-recheck\" => cmd_quarantine_recheck"));
        assert!(client_src.contains("call(\"/quarantine-recheck\""));
    }

    #[test]
    fn stored_not_before_rule_9179() {
        // #9179 (closing the #6426 bypass): the values the audit
        // found stored verbatim — delay-seconds in the absolute
        // field — fail the storage-layer rule, as do the other
        // garbage shapes; valid timestamps pass; a timestamp past
        // the 366-day horizon fails like everywhere else the rule
        // is applied.
        for bad in ["3600", "7200", "banana", "", "0", "9999-01-01T00:00:00Z"] {
            let err = validate_stored_not_before(bad)
                .err()
                .expect("malformed not_before must be rejected");
            assert!(err.contains("not_before"), "error names the field: {err}");
        }
        for ok in ["2020-01-01T00:00:00Z", "2026-11-06T05:30:00+05:30"] {
            assert!(validate_stored_not_before(ok).is_ok(), "{ok} must pass");
        }
        // Both verbs that store a caller-supplied not_before apply
        // the rule before mutating (source oracle over each verb's
        // own segment).
        let src = &test_source();
        for (verb, next) in [
            ("fn verb_park(", "\nfn verb_release("),
            ("fn verb_revive(", "\nfn verb_"),
        ] {
            let seg = src
                .split(verb)
                .nth(1)
                .and_then(|s| s.split(next).next())
                .unwrap_or("");
            assert!(
                seg.contains("validate_stored_not_before(nb)"),
                "{verb} validates an explicit not_before at the storage layer"
            );
        }
        // The hygiene fold counts only failing rows, per queue.
        let mut by_queue: Map<String, Value> = Map::new();
        by_queue.insert("q1".to_string(), health_entry("q1"));
        let rows = vec![
            json!({"queue": "q1", "not_before": "3600"}),
            json!({"queue": "q1", "not_before": "2020-01-01T00:00:00Z"}),
            json!({"queue": "q1", "not_before": "7200"}),
        ];
        apply_malformed_parked(&mut by_queue, &rows);
        assert_eq!(by_queue["q1"]["malformed_parked"], 2);
        assert_eq!(health_entry("q2")["malformed_parked"], 0);
    }

    #[test]
    fn health_malformed_parked_includes_null_not_before_9179() {
        // Oracle: stored not_before values must pass the same validator
        // used by the hygiene fold; NULL is not an RFC 3339 timestamp.
        // Health must therefore fetch NULL rows rather than filtering
        // them out before the fold can report them.
        let src = &test_source();
        let sql = src
            .split("SELECT queue, not_before FROM tasks")
            .nth(1)
            .and_then(|s| s.split("let hrows").next())
            .unwrap_or("");
        assert!(sql.contains("status = 'parked'"), "{sql}");
        assert!(
            !sql.contains("not_before IS NOT NULL"),
            "NULL not_before rows must reach the malformed-row validator: {sql}"
        );
        let mut by_queue: Map<String, Value> = Map::new();
        by_queue.insert("q1".to_string(), health_entry("q1"));
        apply_malformed_parked(&mut by_queue, &[json!({"queue": "q1", "not_before": null})]);
        assert_eq!(by_queue["q1"]["malformed_parked"], 1);
    }

    #[test]
    fn health_entry_defaults_at_cap_queued_to_zero() {
        let e = health_entry("repo-health-code-pilot");
        assert_eq!(e.get("at_cap_queued"), Some(&json!(0)));
        assert_eq!(e.get("queued"), Some(&json!(0)));
        // #8234: the sink is its own health bucket — 'parked' in
        // health output means genuine waits only.
        assert_eq!(e.get("handed_off"), Some(&json!(0)));
    }

    #[test]
    fn health_entry_defaults_failed_events_to_zero() {
        // #1233 criterion 1 ("zero dead-in-queue over a rolling 7
        // days") is unmeasured today: health lumps failed terminal
        // events into terminal_events_24h/7d together with completed
        // ones, so a queue can show 2846 terminal events in 7d with
        // no way to tell whether any were failures. The failed-only
        // counters default to zero here and are filled by the
        // terminal-events fold.
        let e = health_entry("repo-health-code-pilot");
        assert_eq!(e.get("failed_events_24h"), Some(&json!(0)));
        assert_eq!(e.get("failed_events_7d"), Some(&json!(0)));
    }

    #[test]
    fn terminal_events_fold_splits_failed_from_completed() {
        // Fixture for #1233: queue-mixed had 5 terminal events in
        // 7d (3 in 24h), of which 2 were failures (1 in 24h);
        // queue-clean had terminal events but zero failures;
        // queue-quiet had none.
        let mut by_queue: Map<String, Value> = Map::new();
        for q in ["queue-mixed", "queue-clean", "queue-quiet"] {
            by_queue.insert(q.to_string(), health_entry(q));
        }
        let rows = vec![
            json!({"queue": "queue-mixed", "n24": 3, "n7": 5, "f24": 1, "f7": 2}),
            json!({"queue": "queue-clean", "n24": 4, "n7": 9, "f24": 0, "f7": 0}),
        ];
        apply_terminal_events(&mut by_queue, &rows);
        let mixed = &by_queue["queue-mixed"];
        assert_eq!(mixed.get("terminal_events_24h"), Some(&json!(3)));
        assert_eq!(mixed.get("terminal_events_7d"), Some(&json!(5)));
        assert_eq!(mixed.get("failed_events_24h"), Some(&json!(1)));
        assert_eq!(mixed.get("failed_events_7d"), Some(&json!(2)));
        // Completed-only rows leave the failed counters at zero,
        // and a queue with no terminal events at all keeps every
        // default.
        assert_eq!(
            by_queue["queue-clean"].get("failed_events_7d"),
            Some(&json!(0))
        );
        assert_eq!(
            by_queue["queue-quiet"].get("failed_events_7d"),
            Some(&json!(0))
        );
        assert_eq!(
            by_queue["queue-quiet"].get("terminal_events_7d"),
            Some(&json!(0))
        );
    }

    #[test]
    fn health_failed_events_count_terminal_failures_only() {
        // Codex review on #12682 (discussion 4660035001): the
        // failed-only sums counted every 'failed' event, so a
        // transient requeue or a block read as a dead-in-queue
        // failure, while a terminal lease expiry at cap read as
        // zero. The terminal-events query must distinguish
        // dispositions: requeued and blocked 'failed' events are
        // live retries, lease_expired_at_cap is a terminal failure.
        let src = &test_source();
        let seg = seg8217(src, "fn verb_health(", "Merge-intent stack storage");
        assert!(
            seg.contains("'lease_expired_at_cap'"),
            "the kind filter includes terminal lease-expiry events"
        );
        assert!(
            seg.contains("json_extract(e.detail, '$.requeued')"),
            "the failed sums read the requeued disposition from the event detail"
        );
        assert!(
            seg.contains("json_extract(e.detail, '$.reason_class')"),
            "the failed sums read the reason_class disposition from the event detail"
        );
        assert!(
            seg.contains("'blocked'"),
            "blocked failures are excluded from the terminal failed sums"
        );
        assert!(
            !seg.contains("e.kind = 'failed' THEN 1"),
            "the failed sums no longer count every 'failed' event"
        );
    }

    #[test]
    fn at_cap_fold_covers_zero_claimable_and_partial_queues() {
        // Fixture for #6528 criterion 3: queue-dead has queued > 0
        // with EVERY queued task at cap (zero claimable); queue-live
        // has some at cap; queue-calm has none.
        let mut by_queue: Map<String, Value> = Map::new();
        for (q, queued) in [("queue-dead", 3), ("queue-live", 5), ("queue-calm", 2)] {
            let mut e = health_entry(q);
            e.as_object_mut()
                .unwrap()
                .insert("queued".to_string(), json!(queued));
            by_queue.insert(q.to_string(), e);
        }
        let rows = vec![
            json!({"queue": "queue-dead", "n": 3}),
            json!({"queue": "queue-live", "n": 2}),
        ];
        apply_at_cap_queued(&mut by_queue, &rows);
        let dead = &by_queue["queue-dead"];
        assert_eq!(dead.get("queued"), Some(&json!(3)));
        assert_eq!(dead.get("at_cap_queued"), Some(&json!(3)));
        // at_cap == queued means nothing in this queue can be claimed.
        assert_eq!(dead.get("at_cap_queued"), dead.get("queued"));
        assert_eq!(by_queue["queue-live"].get("at_cap_queued"), Some(&json!(2)));
        assert_eq!(by_queue["queue-calm"].get("at_cap_queued"), Some(&json!(0)));
    }

    #[test]
    fn merge_obj_is_shallow_and_object_only() {
        let out = merge_obj(&json!({"a": 1, "b": 2}), &json!({"b": 3, "c": 4}));
        assert_eq!(out, json!({"a": 1, "b": 3, "c": 4}));
        assert_eq!(merge_obj(&json!(null), &json!({"a": 1})), json!({"a": 1}));
    }

    /// Runs the gate with a queue that holds `tasks` (#10611).
    fn gate_with(
        class: &CompletionClass,
        expected_issue: Option<i64>,
        fix_follow_on: bool,
        tasks: &[crate::disposition::SuccessorTask],
        result: &Value,
    ) -> Result<Option<TriageOutcome>, String> {
        let mut lookup = |id: i64| Ok(tasks.iter().find(|t| t.id == id).cloned());
        let mut ctx = GateContext {
            task_id: 15345,
            expected_issue,
            fix_follow_on,
            lookup: &mut lookup,
        };
        gate_completion(class, &mut ctx, result)
    }

    fn gate(class: &CompletionClass, result: &Value) -> Result<Option<TriageOutcome>, String> {
        gate_with(class, None, true, &[], result)
    }

    /// A disposition that a gate accepts for "no work left": the
    /// issue closed, the linked pull merged, readback taken now.
    fn live_disposition(issue: i64, pulls: Value) -> Value {
        json!({
            "checked_at": now_iso(),
            "source": "fleet gh readback",
            "issue": {"number": issue, "state": "closed", "closed_at": now_iso()},
            "pulls": pulls,
            "verdict": "merged_verified",
        })
    }

    #[test]
    fn completion_gate_fix_class_requires_evidence() {
        let fix = CompletionClass::Fix;
        let merged = live_disposition(
            1,
            json!([{"number": 1, "state": "closed", "merged_at": now_iso()}]),
        );
        assert!(gate(&fix, &json!({})).is_err());
        assert!(gate(&fix, &json!({"note": "did stuff"})).is_err());
        assert!(gate(&fix, &json!({"evidence": ""})).is_err());
        assert!(gate(&fix, &json!({"evidence": []})).is_err());
        // #10611: evidence alone is no longer a completion.
        assert!(
            gate(
                &fix,
                &json!({"pr_url": "https://github.com/developing-today/code/pull/1"})
            )
            .is_err()
        );
        assert!(
            gate(
                &fix,
                &json!({"pr_url": "https://github.com/developing-today/code/pull/1",
                        "disposition": merged})
            )
            .is_ok()
        );
        assert!(gate(&fix, &json!({"commit": "abc123", "disposition": merged})).is_ok());
        assert!(
            gate(
                &fix,
                &json!({"verified_closure": "closed by #1, verified on the issue",
                        "disposition": merged})
            )
            .is_ok()
        );
        assert!(
            gate(
                &fix,
                &json!({"evidence": ["artifact"], "disposition": merged})
            )
            .is_ok()
        );
        // A disposition that does not cover the claimed PR is a rejection.
        assert!(
            gate(
                &fix,
                &json!({"pr_url": "https://x/pull/2", "disposition": merged})
            )
            .is_err()
        );
    }

    #[test]
    fn completion_gate_slice_class_requires_evidence() {
        // #12233 F2c: a feature_slice completion with no artifact is
        // rejected, exactly like a fix — the gate is the actual
        // completion path (verb_complete runs gate_completion before
        // any mutation), not just the class mapping.
        let slice = CompletionClass::Slice;
        assert!(gate(&slice, &json!({})).is_err());
        assert!(gate(&slice, &json!({"note": "did stuff"})).is_err());
        assert!(gate(&slice, &json!({"evidence": ""})).is_err());
        assert!(gate(&slice, &json!({"evidence": []})).is_err());
        // Evidence of the delivered slice completes — and, unlike a
        // fix, needs no verified disposition: the parent issue stays
        // open while slices land, so an open parent with no
        // disposition is not a rejection.
        let evidence = json!({"pr_url": "https://github.com/developing-today/code/pull/12570"});
        assert!(gate(&slice, &evidence).is_ok());
        assert!(
            gate_with(&slice, Some(1062), false, &[], &evidence).is_ok(),
            "an open parent issue must not block a slice completion"
        );
        assert!(
            gate(&slice, &json!({"commit": "abc123"})).is_ok(),
            "a commit is slice evidence too"
        );
    }

    #[test]
    fn completion_gate_rejects_disposition_for_another_issue() {
        // #10611: the readback must describe the packet's issue — a
        // stale readback for a sibling issue is not closure evidence.
        let merged = live_disposition(
            1,
            json!([{"number": 1, "state": "closed", "merged_at": now_iso()}]),
        );
        let err = gate_with(
            &CompletionClass::Fix,
            Some(10611),
            true,
            &[],
            &json!({"pr_url": "https://x/pull/1", "disposition": merged}),
        )
        .unwrap_err();
        assert!(err.contains("10611"), "{err}");
    }

    #[test]
    fn completion_gate_rejects_stale_disposition() {
        let mut stale = live_disposition(
            1,
            json!([{"number": 1, "state": "closed", "merged_at": now_iso()}]),
        );
        stale["checked_at"] = json!("2020-01-01T00:00:00Z");
        let err = gate(
            &CompletionClass::Fix,
            &json!({"pr_url": "https://x/pull/1", "disposition": stale}),
        )
        .unwrap_err();
        assert!(err.contains("old"), "{err}");
    }

    /// #9155 as the broker sees it after an intentional close: the
    /// issue closed while draft #9245 is still open.
    fn closed_9155_with_open_draft(follow_up: Value) -> Value {
        json!({
            "checked_at": now_iso(),
            "source": "fleet gh readback",
            "issue": {"number": 9155, "state": "closed", "state_reason": "completed",
                      "closed_at": now_iso(), "closed_by": "dezren39"},
            "pulls": [
                {"number": 9245, "state": "open", "is_draft": true, "merged_at": null},
                {"number": 9300, "state": "closed", "merged_at": now_iso()},
            ],
            "verdict": "closed_verified",
            "follow_up": follow_up,
        })
    }

    fn successor(id: i64, status: &str, issue: Option<i64>) -> crate::disposition::SuccessorTask {
        crate::disposition::SuccessorTask {
            id,
            live: LIVE_STATUSES.contains(&status),
            status: status.to_string(),
            issue,
        }
    }

    #[test]
    fn completion_gate_closed_issue_with_unfinished_draft() {
        // Review on #10616: the broker (not only the CLI) must hold a
        // closed issue with an open draft to a verified follow-up.
        let fix = CompletionClass::Fix;
        let complete = |follow_up: Value, tasks: &[crate::disposition::SuccessorTask]| {
            gate_with(
                &fix,
                Some(9155),
                true,
                tasks,
                &json!({"pr_url": "https://x/pull/9300",
                        "disposition": closed_9155_with_open_draft(follow_up)}),
            )
        };
        // no follow-up
        let err = complete(Value::Null, &[]).unwrap_err();
        assert!(err.contains("#9245") && err.contains("follow_up"), "{err}");
        // a follow-up task that does not exist
        let follow = json!({"task_id": 17413, "reason": "#9245 stays with 17413"});
        assert!(
            complete(follow.clone(), &[])
                .unwrap_err()
                .contains("does not exist")
        );
        // a terminal one
        assert!(
            complete(follow.clone(), &[successor(17413, "done", Some(9155))])
                .unwrap_err()
                .contains("terminal")
        );
        // an unrelated one
        assert!(
            complete(follow.clone(), &[successor(17413, "queued", Some(9999))])
                .unwrap_err()
                .contains("unrelated")
        );
        // the completing task itself
        let own = json!({"task_id": 15345, "reason": "r"});
        assert!(
            complete(own, &[successor(15345, "claimed", Some(9155))])
                .unwrap_err()
                .contains("its own successor")
        );
        // a real successor task
        assert_eq!(
            complete(follow, &[successor(17413, "queued", Some(9155))]).unwrap(),
            None
        );
        // a verified merged successor PR
        assert_eq!(
            complete(
                json!({"successor_pr": 9300, "reason": "#9300 replaced #9245"}),
                &[]
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn completion_gate_triage_doctrine() {
        let triage = CompletionClass::Triage;
        let merged = live_disposition(
            1,
            json!([{"number": 2, "state": "closed", "merged_at": now_iso()}]),
        );
        // Default: the issue goes back in the queue as a follow-on.
        assert_eq!(
            gate(&triage, &json!({})).unwrap(),
            Some(TriageOutcome::FollowOn)
        );
        assert_eq!(
            gate(&triage, &json!({"triage_outcome": "follow_on"})).unwrap(),
            Some(TriageOutcome::FollowOn)
        );
        // Invalid is an exit only with the validation evidence.
        assert!(gate(&triage, &json!({"triage_outcome": "invalid"})).is_err());
        assert_eq!(
            gate(
                &triage,
                &json!({"triage_outcome": "invalid", "triage_note": "not a real issue: duplicate of #1"})
            )
            .unwrap(),
            Some(TriageOutcome::ExitInvalid)
        );
        // Already-solved needs a real evidence reference AND the
        // verified disposition (#10611).
        assert!(gate(&triage, &json!({"triage_outcome": "already_solved"})).is_err());
        assert!(
            gate(
                &triage,
                &json!({"triage_outcome": "already_solved", "pr_url": "https://x/pull/2"})
            )
            .is_err()
        );
        assert_eq!(
            gate(
                &triage,
                &json!({"triage_outcome": "already_solved", "pr_url": "https://x/pull/2",
                        "disposition": merged})
            )
            .unwrap(),
            Some(TriageOutcome::ExitAlreadySolved)
        );
        assert!(gate(&triage, &json!({"triage_outcome": "done"})).is_err());
    }

    /// The 15345 receipt: a triage of #9155 completed `done` with
    /// outcome `triaged` while draft #9245 was open and #9155 was open
    /// after its reopen. With no fix follow-on for its key, nothing
    /// live owned the work afterwards.
    fn open_9155_with_draft(follow_up: Value) -> Value {
        json!({
            "checked_at": now_iso(),
            "source": "fleet gh readback",
            "issue": {"number": 9155, "state": "open",
                      "closed_at": "2026-11-13T01:45:20Z"},
            "pulls": [{"number": 9245, "state": "open", "is_draft": true, "merged_at": null}],
            "verdict": "open_draft_with_followup",
            "follow_up": follow_up,
        })
    }

    #[test]
    fn replay_15345_triage_cannot_deliver_without_a_live_successor() {
        let triage = CompletionClass::Triage;
        let receipt = json!({"outcome": "triaged", "evidence": "triaged #9155"});
        // No fix follow-on for this key and no handoff: rejected.
        let err = gate_with(&triage, Some(9155), false, &[], &receipt).unwrap_err();
        assert!(err.contains("disposition"), "{err}");
        // A readback alone is not a handoff while #9155 is open.
        let mut with_readback = receipt.clone();
        with_readback["disposition"] = open_9155_with_draft(Value::Null);
        assert!(gate_with(&triage, Some(9155), false, &[], &with_readback).is_err());
        // A handoff to a task that is gone is not a successor either.
        let follow = json!({"task_id": 17413, "reason": "#9245 still carries the #9155 work"});
        let mut handed = receipt.clone();
        handed["disposition"] = open_9155_with_draft(follow);
        assert!(
            gate_with(
                &triage,
                Some(9155),
                false,
                &[successor(17413, "cancelled", Some(9155))],
                &handed
            )
            .is_err()
        );
        // A legitimate triage that records a live handoff passes.
        assert_eq!(
            gate_with(
                &triage,
                Some(9155),
                false,
                &[successor(17413, "queued", Some(9155))],
                &handed
            )
            .unwrap(),
            Some(TriageOutcome::FollowOn)
        );
        // With a fix-family key the broker enqueues the successor
        // itself, so ordinary triage needs nothing more.
        assert_eq!(
            gate_with(&triage, Some(9155), true, &[], &receipt).unwrap(),
            Some(TriageOutcome::FollowOn)
        );
    }

    fn claimable_row(payload: Value) -> Value {
        json!({
            "id": 42,
            "queue": "repo-health-code-pilot",
            "title": "Fix #5444: claim payload guard",
            "body": "b",
            "priority": 30,
            "dedupe_key": "repo-health:v1:fix:developing-today/code:issue:5444",
            "payload": payload,
        })
    }

    fn valid_payload() -> Value {
        json!({
            "schema": "repo-task/v1",
            "kind": "small_fix",
            "repo": "developing-today/code",
            "issue": {"number": 5444},
            "problem": "p",
            "priority_reason": "r",
            "evidence": ["e"],
            "acceptance": ["a"],
            "tests": ["t"],
            "context_refs": [],
            "scope": {"include": ["x"], "exclude": ["y"]},
            "risk": {"level": "low", "flags": []},
            "timebox_minutes": 45,
            "max_attempts": 4,
            "dedupe": {"checked_at": "2026-11-04T00:00:00Z", "basis": "search"},
        })
    }

    #[test]
    fn claim_guard_accepts_a_valid_packet() {
        // Oracle: validate_packet, the /enqueue validator — a row
        // enqueue would accept must pass the claim guard unchanged.
        assert_eq!(claim_guard_reason(&claimable_row(valid_payload())), None);
    }

    #[test]
    fn claim_guard_accepts_a_body_less_stored_row() {
        // task_from_packet stores an absent body as SQL NULL, so a
        // body-less packet's stored row carries body:null. The #6430
        // body type check must not quarantine it (Codex P1, PR #8477).
        let mut row = claimable_row(valid_payload());
        row["body"] = Value::Null;
        assert_eq!(claim_guard_reason(&row), None);
        let mut row = claimable_row(valid_payload());
        row.as_object_mut().unwrap().remove("body");
        assert_eq!(claim_guard_reason(&row), None);
    }

    #[test]
    fn claim_guard_rejects_payload_less_rows() {
        // The 2026-11-04 incident shapes: no payload field, a null
        // payload, and an empty-object payload must never be handed
        // out (#5444). Each is a quarantine, not a claim.
        let mut no_field = claimable_row(valid_payload());
        no_field.as_object_mut().unwrap().remove("payload");
        for row in [
            no_field,
            claimable_row(Value::Null),
            claimable_row(json!({})),
        ] {
            let reason = claim_guard_reason(&row).expect("payload-less row must be guarded");
            assert!(reason.contains("payload missing/empty"), "{reason}");
        }
    }

    #[test]
    fn claim_guard_rejects_schema_invalid_packets() {
        // A present-but-invalid packet (wrong schema, kind/family
        // mismatch) fails the same validator enqueue runs.
        let mut bad = valid_payload();
        bad["schema"] = json!("repo-task/v0");
        let reason =
            claim_guard_reason(&claimable_row(bad)).expect("schema-invalid packet must be guarded");
        assert!(reason.contains("packet failed validation"), "{reason}");
        // Invariant sweep: mutating any required identity field of a
        // valid packet to a placeholder must trip the guard — claim
        // never returns a row failing validate_packet.
        for mutated in [json!({"number": 0}), json!({"number": -1})] {
            let mut p = valid_payload();
            p["issue"] = mutated;
            assert!(claim_guard_reason(&claimable_row(p)).is_some());
        }
    }

    fn handoff_row(status: &str, progress: Value) -> Value {
        let mut row = claimable_row(valid_payload());
        row["status"] = json!(status);
        row["progress"] = progress;
        row["attempts"] = json!(1);
        row["max_attempts"] = json!(4);
        row["not_before"] = json!("2999-01-01T00:00:00Z");
        row
    }

    #[test]
    fn by_id_claim_takes_a_parked_recorded_handoff() {
        // The #7907/#6533 loop shape, post-#8234: opencode lane
        // assigned, yield recorded under last_checkpoint (the #7706
        // lift), no session bound — and the row RESTS IN THE SINK:
        // status 'handed_off', no wait fields. The not_before on
        // this fixture row is deliberately in the far future: a
        // sink row's claim ignores timers entirely, because the
        // sink is not a wait.
        let row = handoff_row(
            "handed_off",
            json!({
                "lane": "opencode",
                "last_checkpoint": {"stage": "handed_off", "at": "2026-11-09T23:15:55Z"},
                "session_id": null,
            }),
        );
        assert!(is_handed_off(&row));
        assert!(by_id_claimable(&row, "2026-11-09T23:20:00Z"));
        // The launch_failed recovery yield in the sink is the same
        // recorded handoff in its other spelling.
        let row2 = handoff_row(
            "handed_off",
            json!({"lane": "opencode", "stage": "launch_failed"}),
        );
        assert!(by_id_claimable(&row2, "2026-11-09T23:20:00Z"));
        // #8192 regression: the stage sources DISAGREE on this row
        // (top-level 'working' vs last_checkpoint 'handed_off') —
        // sink membership is the status, so it is claimable either
        // way; neither the broker nor the claimant reads a stage
        // for a resting sink row anymore.
        let row3 = handoff_row(
            "handed_off",
            json!({
                "lane": "opencode", "stage": "working",
                "last_checkpoint": {"stage": "handed_off"},
            }),
        );
        assert!(is_handed_off(&row3));
        assert!(by_id_claimable(&row3, "2026-11-09T23:20:00Z"));
    }

    #[test]
    fn by_id_claim_refuses_genuine_wait_parks_and_bound_rows() {
        // A gated dependency park: opencode lane, but its stage is
        // the park itself, not a yield — never a handoff.
        let gated = handoff_row(
            "parked",
            json!({"lane": "opencode", "stage": "parked", "gate": "#3801 OPEN"}),
        );
        assert!(!by_id_claimable(&gated, "2026-11-09T23:20:00Z"));
        // #8234: a 'parked' row with handoff-SHAPED progress (the
        // pre-migration / task-10995 shape: a recorded handoff that
        // later took a genuine triage wait) is a genuine wait park,
        // NOT a sink row — the status decides, the shape does not.
        let wait_shaped = handoff_row(
            "parked",
            json!({
                "lane": "opencode",
                "last_checkpoint": {"stage": "handed_off"},
                "session_id": null,
            }),
        );
        assert!(!is_handed_off(&wait_shaped));
        assert!(!by_id_claimable(&wait_shaped, "2026-11-09T23:20:00Z"));
        // A direct-lane yield in the sink status is not the
        // claimant's row (the lane guard holds).
        let direct = handoff_row(
            "handed_off",
            json!({"lane": "direct", "stage": "handed_off"}),
        );
        assert!(!by_id_claimable(&direct, "2026-11-09T23:20:00Z"));
        // A bound session id means a launch already happened; the
        // binding is supervision's, never a fresh claim's.
        let bound = handoff_row(
            "handed_off",
            json!({
                "lane": "opencode", "stage": "handed_off",
                "session_id": "ses_abc",
            }),
        );
        assert!(!by_id_claimable(&bound, "2026-11-09T23:20:00Z"));
        // A claimed row is owned; by-id never steals a live claim.
        let claimed = handoff_row(
            "claimed",
            json!({"lane": "opencode", "stage": "handed_off"}),
        );
        assert!(!by_id_claimable(&claimed, "2026-11-09T23:20:00Z"));
    }

    #[test]
    fn park_classification_lands_handoffs_in_the_sink_8234() {
        // A recorded handoff parked with no declared condition
        // sinks; so does one carrying only the bare time timer
        // (the park verb's own default bookkeeping).
        let handoff = json!({
            "lane": "opencode",
            "last_checkpoint": {"stage": "handed_off"},
            "session_id": null,
        });
        assert!(handoff_park(&handoff, true, &json!({"kind": "time"})));
        assert!(handoff_park(&handoff, false, &json!({"kind": "time"})));
        // A caller-named real wait always wins, even on a
        // handoff-shaped row: CI, a task dependency, a quota reset.
        assert!(!handoff_park(
            &handoff,
            false,
            &json!({"kind": "ci", "pr": 1})
        ));
        assert!(!handoff_park(
            &handoff,
            false,
            &json!({"kind": "task_terminal", "task_id": 7})
        ));
        assert!(!handoff_park(
            &handoff,
            false,
            &json!({"kind": "quota_reset", "at": "2026-11-13T00:00:00Z"})
        ));
        // A triage-stage park is the broker's triage wait, never
        // a handoff — it does not even have the handoff shape.
        let triage = json!({"lane": "opencode", "stage": "awaiting_triage"});
        assert!(!handoff_park(&triage, true, &json!({"kind": "time"})));
        // A direct-lane yield is not an opencode handoff.
        let direct = json!({"lane": "direct", "stage": "handed_off"});
        assert!(!handoff_park(&direct, true, &json!({"kind": "time"})));
        // A still-bound row never sinks, whatever the stage says:
        // the binding means a live session owns it.
        let bound = json!({
            "lane": "opencode", "stage": "handed_off",
            "session_id": "ses_abc",
        });
        assert!(!handoff_park(&bound, true, &json!({"kind": "time"})));
    }

    #[test]
    fn sink_return_lands_in_handed_off_without_a_timer_8234() {
        // The lease-expiry sink return: status 'handed_off', every
        // wait field NULL, and no trace of the pre-#8234 fake timer
        // (+1200s not_before with a time ready_condition).
        let sql = sink_return_sql();
        assert!(sql.contains("status = 'handed_off'"));
        assert!(sql.contains("not_before = NULL"));
        assert!(sql.contains("ready_condition = NULL"));
        assert!(sql.contains("park_kind = NULL"));
        assert!(!sql.contains("1200"));
        assert!(!sql.contains("'parked'"));
        // Live statuses include the sink (dedupe ownership).
        assert!(LIVE_STATUSES.contains(&"handed_off"));
    }

    #[test]
    fn platform_parse_accepts_only_the_two_families() {
        // #8097 Rule 2: unset is absence; the only set values are
        // the two coarse families; anything else errors.
        assert_eq!(parse_platform(None), Ok(None));
        assert_eq!(parse_platform(Some("")), Ok(None));
        assert_eq!(
            parse_platform(Some("openchamber")),
            Ok(Some("openchamber".to_string()))
        );
        assert_eq!(parse_platform(Some("muse")), Ok(Some("muse".to_string())));
        assert!(parse_platform(Some("code-a")).is_err());
        assert!(parse_platform(Some("OpenChamber")).is_err());
    }

    #[test]
    fn platform_is_a_mark_not_a_claim_filter_8187() {
        // #8187 amendment (demoting #8097 Rule 2): platform is a
        // MARK, never a filter or a requirement.
        // (a) A claim facing a MISMATCHED platform mark still
        // claims the item: by-id eligibility takes no platform
        // input at all, so a muse-marked queued row is claimable
        // by any claim (an openchamber claimant's included).
        let mut row = handoff_row("queued", json!({}));
        row["not_before"] = json!("2026-11-09T00:00:00Z");
        row["platform"] = json!("muse");
        let now = "2026-11-09T23:20:00Z";
        assert!(by_id_claimable(&row, now));
        // (b) A claim declaring NO platform works identically:
        // unmarked and marked rows are equally claimable, and the
        // pick path binds no platform argument to match against.
        let mut unmarked = handoff_row("queued", json!({}));
        unmarked["not_before"] = json!("2026-11-09T00:00:00Z");
        assert!(by_id_claimable(&unmarked, now));
        // (c) The mark is RECORDED on the claimed row: both claim
        // UPDATEs stamp the declared platform over the row's mark
        // (COALESCE keeps the existing mark when none is declared)
        // and NEITHER carries a platform filter.
        let upd = claim_update_sql("(status = 'queued')");
        assert!(upd.contains("platform = COALESCE(?, platform)"), "{upd}");
        assert!(!upd.contains("platform IS NULL"), "{upd}");
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(!impl_src.contains("platform IS NULL OR platform = ?"));
        assert!(!impl_src.contains("platform_allows"));
        assert!(impl_src.matches("platform = COALESCE(?, platform)").count() >= 2);
    }

    #[test]
    fn claim_event_carries_lane_pool_model_session_marks_8187() {
        // #8187 amendments 2+3: the audit marks name who is looking
        // at the item — platform, lane, pool (= queue), model, the
        // bound session, and the task id.
        let task = json!({
            "id": 4242, "queue": "repo-health-code-pilot",
            "platform": "openchamber", "lane_id": "code-a",
            "claim_provider": "_claude", "claim_model": "claude-sonnet-5-5",
            "claim_variant": "medium",
            "progress": {"session_id": "ses_test_1", "lane": "direct"},
        });
        let detail = with_marks(&json!({"via": "claim_repo_task_turso"}), &task);
        assert_eq!(detail["task_id"], json!(4242));
        assert_eq!(detail["pool"], json!("repo-health-code-pilot"));
        assert_eq!(detail["queue"], json!("repo-health-code-pilot"));
        assert_eq!(detail["platform"], json!("openchamber"));
        assert_eq!(detail["lane_id"], json!("code-a"));
        assert_eq!(detail["model_provider"], json!("_claude"));
        assert_eq!(detail["model_id"], json!("claude-sonnet-5-5"));
        assert_eq!(detail["model_variant"], json!("medium"));
        assert_eq!(detail["session_id"], json!("ses_test_1"));
        // Explicit detail fields win over the marks.
        let d2 = with_marks(&json!({"session_id": "ses_bound_now"}), &task);
        assert_eq!(d2["session_id"], json!("ses_bound_now"));
        // Unmarked rows still emit the fields — as nulls. Marks are
        // recorded when available, never required.
        let bare = with_marks(&json!({}), &json!({"id": 7}));
        assert_eq!(bare["task_id"], json!(7));
        assert_eq!(bare["platform"], Value::Null);
        assert_eq!(bare["session_id"], Value::Null);
        // Source oracle: every audited transition (claim pick +
        // by-id, lane assign + flip, bind, park, wake, complete,
        // fail) routes its event through with_marks.
        let src = &test_source();
        let impl_src = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(
            impl_src.matches("with_marks(").count() >= 9,
            "audited transitions must carry marks: {}",
            impl_src.matches("with_marks(").count()
        );
    }

    #[test]
    fn by_id_claim_queued_rows_keep_the_pick_predicate() {
        // Queued + attempts left + not_before passed: claimable.
        let mut row = handoff_row("queued", json!({}));
        row["not_before"] = json!("2026-11-09T00:00:00Z");
        assert!(by_id_claimable(&row, "2026-11-09T23:20:00Z"));
        // Queued but parked into the future: not claimable by id.
        let future = handoff_row("queued", json!({}));
        assert!(!by_id_claimable(&future, "2026-11-09T23:20:00Z"));
        // Queued at the attempts cap: not claimable by id.
        let mut capped = handoff_row("queued", json!({}));
        capped["not_before"] = json!("2026-11-09T00:00:00Z");
        capped["attempts"] = json!(4);
        assert!(!by_id_claimable(&capped, "2026-11-09T23:20:00Z"));
    }

    #[test]
    fn completion_class_and_follow_on_key() {
        assert_eq!(
            completion_class(&json!({"kind": "small_fix"})),
            CompletionClass::Fix
        );
        assert_eq!(
            completion_class(&json!({"kind": "triage"})),
            CompletionClass::Triage
        );
        assert_eq!(
            completion_class(&json!({"kind": "feature_slice"})),
            CompletionClass::Slice
        );
        assert_eq!(completion_class(&json!({})), CompletionClass::Ungated);
        // Ungated completions never gate, whatever the result.
        assert_eq!(gate(&CompletionClass::Ungated, &json!({})).unwrap(), None);
        assert_eq!(
            follow_on_key("repo-health:v1:triage:developing-today/code:issue:787"),
            Some("repo-health:v1:fix:developing-today/code:issue:787".to_string())
        );
        // Maintenance triage keys have no fix counterpart.
        assert_eq!(
            follow_on_key("repo-health:v1:maintenance:developing-today/code:weekly:2026-W44"),
            None
        );
    }

    // -- /coverage (#7016) --------------------------------------------------

    #[test]
    fn issue_number_parses_canonical_keys() {
        let repo = "developing-today/code";
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today/code:issue:7016",
                repo
            ),
            Some(7016)
        );
        assert_eq!(
            issue_number_from_dedupe("repo-health:v1:fix:developing-today/code:issue:5486", repo),
            Some(5486)
        );
    }

    #[test]
    fn issue_number_parses_dispatch_suffixed_keys() {
        let repo = "developing-today/code";
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today/code:issue:7016:dispatch:abc123",
                repo
            ),
            Some(7016)
        );
    }

    #[test]
    fn issue_number_rejects_other_repos_and_non_issue_keys() {
        let repo = "developing-today/code";
        // A different repo is not coverage for this one.
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today-forks/opencode:issue:195",
                repo
            ),
            None
        );
        // Maintenance keys carry no issue number.
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:maintenance:developing-today/code:weekly:2026-W44",
                repo
            ),
            None
        );
        // No number at all.
        assert_eq!(
            issue_number_from_dedupe("repo-health:v1:triage:developing-today/code:issue:", repo),
            None
        );
        // A tail that is neither empty nor a separator is not a match
        // (e.g. `:issue:12abc`), so a malformed key cannot silently
        // claim coverage for issue 12.
        assert_eq!(
            issue_number_from_dedupe(
                "repo-health:v1:triage:developing-today/code:issue:12abc",
                repo
            ),
            None
        );
    }

    #[test]
    fn issue_number_matches_legacy_repo_aliases() {
        // The two names of each renamed repo are the same coverage.
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:triage:developing-today/opencode:issue:195",
                "developing-today-forks/opencode"
            ),
            Some(195)
        );
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:fix:developing-today/openchamber:issue:42",
                "developing-today-forks/openchamber"
            ),
            Some(42)
        );
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:triage:dezren39/nix:issue:1637",
                "dezren39/darwin-nix"
            ),
            Some(1637)
        );
        // Matching works in the other direction too: a new-name key is
        // coverage when the caller names the old one.
        assert_eq!(
            issue_number_for_repo(
                "repo-health:v1:triage:developing-today-forks/opencode:issue:195",
                "developing-today/opencode"
            ),
            Some(195)
        );
    }

    #[test]
    fn alias_matching_does_not_cross_repos() {
        // An opencode key is never coverage for code or openchamber,
        // alias or not.
        for repo in [
            "developing-today/code",
            "developing-today-forks/openchamber",
            "dezren39/darwin-nix",
        ] {
            assert_eq!(
                issue_number_for_repo(
                    "repo-health:v1:triage:developing-today/opencode:issue:1",
                    repo
                ),
                None
            );
        }
    }

    proptest! {
        /// Any canonical key built from a repo and a positive issue number
        /// round-trips through the parser, for every class and every
        /// dispatch suffix.
        #[test]
        fn prop_issue_number_round_trips(
            n in 1i64..10_000_000i64,
            class in prop::sample::select(vec!["triage", "fix"]),
            suffix in prop::option::of("[a-z0-9]{1,12}"),
        ) {
            let repo = "developing-today/code";
            let mut key = format!("repo-health:v1:{class}:{repo}:issue:{n}");
            if let Some(s) = &suffix {
                key.push_str(&format!(":dispatch:{s}"));
            }
            prop_assert_eq!(issue_number_from_dedupe(&key, repo), Some(n));
        }

        /// A key for a different repo never matches, whatever the number.
        #[test]
        fn prop_other_repo_never_matches(n in 1i64..10_000_000i64) {
            let key = format!("repo-health:v1:triage:developing-today-forks/opencode:issue:{n}");
            prop_assert_eq!(issue_number_from_dedupe(&key, "developing-today/code"), None);
        }
    }

    // -- /coverage PR half (#12280): a ready PR is covered iff a LIVE
    // -- task's dedupe key names it. The parser decides which keys name
    // -- a PR; the live-status rule is the same SQL filter as issues.

    #[test]
    fn pr_number_parses_warden_and_merge_watch_keys() {
        let repo = "developing-today/code";
        // The stale-PR warden's shepherd key.
        assert_eq!(
            pr_number_from_dedupe("repo-health:v1:pr:developing-today/code:pr:12206", repo),
            Some(12206)
        );
        // The coverage reconciliation's merge-watch key (#4951) — a live
        // merge-watch is a live pr_shepherd naming the PR, so it must
        // count as coverage or the fallback would double-file.
        assert_eq!(
            pr_number_from_dedupe(
                "repo-health:v1:merge-watch:developing-today/code:pr:12206",
                repo
            ),
            Some(12206)
        );
        // Dispatch-suffixed variants parse like the issue classes do.
        assert_eq!(
            pr_number_from_dedupe(
                "repo-health:v1:pr:developing-today/code:pr:12206:dispatch:abc123",
                repo
            ),
            Some(12206)
        );
    }

    #[test]
    fn pr_number_rejects_other_repos_and_non_pr_keys() {
        let repo = "developing-today/code";
        // A different repo's PR is not coverage for this one.
        assert_eq!(
            pr_number_from_dedupe(
                "repo-health:v1:pr:developing-today-forks/opencode:pr:2496",
                repo
            ),
            None
        );
        // An issue key never reads as PR coverage: triage for issue 5
        // does not name PR 5.
        assert_eq!(
            pr_number_from_dedupe("repo-health:v1:triage:developing-today/code:issue:5", repo),
            None
        );
        // And a PR key never reads as issue coverage.
        assert_eq!(
            issue_number_from_dedupe("repo-health:v1:pr:developing-today/code:pr:5", repo),
            None
        );
        // No number at all, and the malformed-tail rule.
        assert_eq!(
            pr_number_from_dedupe("repo-health:v1:pr:developing-today/code:pr:", repo),
            None
        );
        assert_eq!(
            pr_number_from_dedupe("repo-health:v1:pr:developing-today/code:pr:12abc", repo),
            None
        );
    }

    #[test]
    fn pr_number_matches_legacy_repo_aliases() {
        // A shepherd filed under the pre-rename repo name is coverage
        // when the caller names the current one, and vice versa.
        assert_eq!(
            pr_number_for_repo(
                "repo-health:v1:pr:developing-today/opencode:pr:2496",
                "developing-today-forks/opencode"
            ),
            Some(2496)
        );
        assert_eq!(
            pr_number_for_repo(
                "repo-health:v1:merge-watch:developing-today-forks/openchamber:pr:81",
                "developing-today/openchamber"
            ),
            Some(81)
        );
        // Aliases never cross into a different repo.
        assert_eq!(
            pr_number_for_repo(
                "repo-health:v1:pr:developing-today/opencode:pr:1",
                "developing-today/code"
            ),
            None
        );
    }

    proptest! {
        /// Canonical PR keys round-trip for both PR-naming classes,
        /// with or without a dispatch suffix.
        #[test]
        fn prop_pr_number_round_trips(
            n in 1i64..10_000_000i64,
            class in prop::sample::select(vec!["pr", "merge-watch"]),
            suffix in prop::option::of("[a-z0-9]{1,12}"),
        ) {
            let repo = "developing-today/code";
            let mut key = format!("repo-health:v1:{class}:{repo}:pr:{n}");
            if let Some(s) = &suffix {
                key.push_str(&format!(":dispatch:{s}"));
            }
            prop_assert_eq!(pr_number_from_dedupe(&key, repo), Some(n));
        }

        /// A PR key for a different repo never matches.
        #[test]
        fn prop_pr_other_repo_never_matches(n in 1i64..10_000_000i64) {
            let key = format!("repo-health:v1:pr:developing-today-forks/opencode:pr:{n}");
            prop_assert_eq!(pr_number_from_dedupe(&key, "developing-today/code"), None);
        }
    }

    // ---- #8217: batch claim, platform limitation in the pick,
    // ---- and the release verb ----

    fn seg8217<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
        src.split(from)
            .nth(1)
            .unwrap_or("")
            .split(to)
            .next()
            .unwrap_or("")
    }

    #[test]
    fn bundle_cap_refusal_boundary_11333() {
        // #11333 oracle: the broker's own claim count for a
        // (lane, session) launch. Below the cap there is no
        // refusal; at the cap the next ordinary claim is refused
        // with a message naming the cap, the launch and the
        // count — enforcement lives in this decision, not in any
        // lane prompt.
        assert_eq!(BUNDLE_CLAIM_CAP, 10);
        for n in [0, 1, 9] {
            assert_eq!(bundle_cap_refusal(n, "any-n", "ses_x"), None, "count {n}");
        }
        for n in [10, 11, 25] {
            let msg = bundle_cap_refusal(n, "any-n", "ses_x").expect("refusal at cap");
            assert!(msg.contains("bundle cap"), "{msg}");
            assert!(msg.contains("10"), "{msg}");
            assert!(msg.contains("any-n") && msg.contains("ses_x"), "{msg}");
            assert!(msg.contains(&n.to_string()), "{msg}");
            assert!(msg.starts_with("claim_repo_task:"), "{msg}");
        }
    }

    #[test]
    fn bundle_cap_count_sql_counts_only_ordinary_picks_11333() {
        // The counting oracle counts 'claimed' events carrying
        // the ordinary-pick mark for exactly this (lane,
        // session): handoff picks (pick="handed_off") and by-id
        // / transfer claims (no pick mark) can never enter the
        // count. The behavioural run of this SQL against
        // fabricated events lives in tests/bundle_cap_sql.py.
        let sql = BUNDLE_CLAIM_COUNT_SQL;
        assert!(sql.contains("kind = 'claimed'"), "{sql}");
        assert!(
            sql.contains("json_extract(detail, '$.pick') = 'default'"),
            "{sql}"
        );
        assert!(
            sql.contains("json_extract(detail, '$.lane_id') = ?"),
            "{sql}"
        );
        assert!(
            sql.contains("json_extract(detail, '$.session_id') = ?"),
            "{sql}"
        );
    }

    #[test]
    fn bundle_cap_gate_sits_in_pick_and_batch_paths_11333() {
        let src = &test_source();
        // The gate runs inside the pick path, before the pick
        // loop, and handoff picks skip it.
        let pick = seg8217(src, "fn claim_pick_once(", "\nfn parse_batch_count(");
        let gate = pick.find("bundle_cap_gate").expect("gate in pick path");
        let loop_start = pick.find("let mut row: Value;").expect("pick loop");
        assert!(gate < loop_start, "gate precedes the pick loop");
        assert!(pick.contains("if !handoff_pick"), "handoff picks exempt");
        // The batch loop clamps to the launch's remaining
        // allowance so a batch stops at exactly the cap.
        let batch = seg8217(src, "fn verb_claim_batch(", "\nfn verb_claim(");
        assert!(
            batch.contains("bundle_claim_count"),
            "batch consults the count"
        );
        assert!(
            batch.contains("bundle_cap_refusal"),
            "batch refuses at the cap"
        );
        assert!(
            batch.contains("count.min(BUNDLE_CLAIM_CAP - claimed)"),
            "batch clamps to the allowance"
        );
        // Exempt by construction: by-id and transfer claims
        // never call the gate.
        let by_id = seg8217(src, "fn claim_by_id(", "\n/// The pick path's claim UPDATE");
        assert!(!by_id.contains("bundle_cap"), "by-id exempt");
        let transfer = seg8217(src, "fn claim_transfer(", "\n/// By-id claim:");
        assert!(!transfer.contains("bundle_cap"), "transfer exempt");
    }

    #[test]
    fn batch_count_parsing_and_cap_8217() {
        // The cap is a named constant with the reasoning on #8217:
        // one batch = one lane touch = one supervisor.
        assert_eq!(BATCH_CLAIM_MAX, 5);
        assert_eq!(parse_batch_count(&json!({})), Ok(1));
        assert_eq!(parse_batch_count(&json!({"count": 1})), Ok(1));
        assert_eq!(parse_batch_count(&json!({"count": 3})), Ok(3));
        assert_eq!(
            parse_batch_count(&json!({"count": BATCH_CLAIM_MAX})),
            Ok(BATCH_CLAIM_MAX)
        );
        let over = parse_batch_count(&json!({"count": BATCH_CLAIM_MAX + 1})).unwrap_err();
        assert!(
            over.contains("out of range") && over.contains("1..=5"),
            "{over}"
        );
        assert!(parse_batch_count(&json!({"count": 0})).is_err());
        assert!(parse_batch_count(&json!({"count": -2})).is_err());
    }

    #[test]
    fn batch_claim_response_shape_8217() {
        // The batch verb's wire shape: the claimed rows in pick
        // order, how many were claimed, how many were requested —
        // a short batch is a normal partial success, and an empty
        // one is the usual no-task verdict on the client.
        let rows = vec![json!({"id": 11}), json!({"id": 7}), json!({"id": 42})];
        let out = json!({"rows": rows, "claimed": 3, "requested": 5});
        assert_eq!(out["claimed"], json!(3));
        assert_eq!(out["requested"], json!(5));
        let ids: Vec<i64> = out["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![11, 7, 42], "rows keep pick order");
    }

    #[test]
    fn claim_flow_composes_claim_lane_bind_in_order_5757() {
        // #5757-A oracle: /claim-flow is dispatched, prechecks
        // before any step runs, and composes the three verbs in
        // claim -> lane -> bind order, answering with all three
        // rows. (The verbs themselves are covered by their own
        // tests; this pins the composition and the route.)
        let src = &test_source();
        assert!(
            src.contains("\"/claim-flow\" => verb_claim_flow(t, req)"),
            "dispatch routes /claim-flow"
        );
        let flow = seg8217(src, "fn verb_claim_flow(", "\nfn claimed_by(");
        let pre = flow.find("claim_flow_precheck").expect("precheck present");
        let c = flow.find("verb_claim(").expect("claim step");
        let l = flow.find("verb_lane(").expect("lane step");
        let b = flow.find("verb_bind(").expect("bind step");
        assert!(
            pre < c && c < l && l < b,
            "steps run precheck -> claim -> lane -> bind"
        );
        assert!(flow.contains("\"bind_row\""), "answer carries the bind row");
        assert!(flow.contains("\"lane_row\""), "answer carries the lane row");
    }

    #[test]
    fn claim_flow_post_commit_failures_are_partial_not_errors_5757() {
        // #5757-A Codex P1 oracle (PR #8493): once the claim step
        // has committed, a lane/bind failure must answer as a
        // partial success carrying the claimed row and a
        // claim_flow_partial marker for EACH step — never as the
        // step's error envelope, which the client's retry loop
        // would replay into a second claim.
        let src = &test_source();
        let flow = seg8217(src, "fn verb_claim_flow(", "\nfn claimed_by(");
        assert_eq!(
            flow.matches("\"claim_flow_partial\"").count(),
            2,
            "lane and bind failures both carry claim_flow_partial"
        );
        assert!(
            flow.contains("\"step\": \"lane\""),
            "lane partial names its step"
        );
        assert!(
            flow.contains("\"step\": \"bind\""),
            "bind partial names its step"
        );
        assert!(
            !flow.contains("return lane;") && !flow.contains("return bind;"),
            "post-commit step envelopes are never returned verbatim"
        );
    }

    #[test]
    fn batch_claim_runs_the_single_pick_repeatedly_8217() {
        // Oracle: the batch verb is a loop over claim_pick_once
        // (the single claim's pick — same eligibility, same order,
        // same per-item lease, marks and claimed event) behind ONE
        // claim_preamble, and the single claim routes count > 1 to
        // it while refusing the by-id combination.
        let src = &test_source();
        let batch = seg8217(src, "fn verb_claim_batch(", "\nfn verb_claim(");
        assert!(batch.contains("claim_preamble("), "one preamble per batch");
        assert!(
            batch.contains("claim_pick_once("),
            "batch = the single pick, repeated"
        );
        assert!(
            batch.contains("\"rows\": rows"),
            "batch returns the claimed set"
        );
        let single = seg8217(src, "fn verb_claim(", "\nfn verb_bind(");
        assert!(single.contains("count > 1"), "single claim routes batches");
        assert!(
            single.contains("cannot be combined with --task"),
            "by-id and transfer claims stay single"
        );
        assert!(
            single.contains("claim_preamble(") && single.contains("claim_pick_once("),
            "single claim is one preamble + one pick"
        );
    }

    #[test]
    fn platform_limit_pick_clause_shape_8217() {
        // The clause consults the lane ledger and skips only rows
        // with a VALID stored limitation (a recorded reason)
        // naming the OTHER family; reason-less rows are not
        // limitations (Drew's rule: no reason, no limitation).
        for needle in [
            "repo_task_lane_ledger",
            "platform_limit IS NOT NULL",
            "platform_limit_reason IS NOT NULL",
            "platform_limit_reason <> ''",
            "(? IS NULL OR",
        ] {
            assert!(
                PLATFORM_LIMIT_PICK_CLAUSE.contains(needle),
                "clause misses {needle}"
            );
        }
        // Both pick statements carry the clause after the
        // eligibility predicate, with the claim's family bound
        // twice: 7 fixed placeholders + 4 predicate (the two
        // `now`s + the two #3418 budget bindings) + 2 clause.
        let upd = claim_update_sql(&default_pick_predicate());
        assert!(
            upd.contains("AND NOT EXISTS (SELECT 1 FROM repo_task_lane_ledger"),
            "{upd}"
        );
        assert_eq!(upd.matches('?').count(), 13, "claim UPDATE bindings: {upd}");
        let src = &test_source();
        let pick = seg8217(src, "fn claim_pick_once(", "\nfn parse_batch_count(");
        assert_eq!(
            pick.matches("pick_args.push(arg_str(family.as_deref()))")
                .count(),
            2,
            "candidate SELECT binds the family twice"
        );
        assert_eq!(
            pick.matches("upd_args.push(arg_str(family.as_deref()))")
                .count(),
            2,
            "claim UPDATE binds the family twice"
        );
    }

    #[test]
    fn claim_by_id_enforces_platform_limit_like_the_pick_8217() {
        // Oracle: a plain by-id claim refuses a cross-family valid
        // limitation (the row reads as not claimable) and CLEARS a
        // reason-less stored one instead of enforcing it.
        let src = &test_source();
        let seg = seg8217(src, "fn claim_by_id(", "\nfn claim_transfer(");
        assert!(
            seg.contains("platform_limit_refusal("),
            "by-id claim enforces the limitation"
        );
        assert!(
            seg.contains("platform_limit = NULL") && seg.contains("platform_limit_reason = NULL"),
            "an invalid stored limit is cleared, not enforced"
        );
    }

    #[test]
    fn claim_by_id_platform_limit_block_is_reachable_8456() {
        // #8456 (G7) oracle: the by-id platform-limit enforcement
        // must EXECUTE — before this fix it sat inside the
        // claim-guard block, after that block's `return Ok(None)`,
        // as unreachable code (rustc warned; the source-shape test
        // above still passed, because the code was present, just
        // dead). Reachability half: in claim_by_id's own segment,
        // the guard block contains no platform_limit_refusal call,
        // and the enforcement call appears after the guard block's
        // closing brace.
        let src = &test_source();
        let seg = src
            .split("fn claim_by_id(")
            .nth(1)
            .unwrap_or("")
            .split("\n}\n")
            .next()
            .unwrap_or("");
        let guard_open = seg
            .find("if let Some(reason) = claim_guard_reason(&row)")
            .expect("claim_by_id opens with the claim guard");
        let guard_close = seg[guard_open..]
            .find("\n    }\n")
            .map(|i| guard_open + i)
            .expect("the claim-guard block closes");
        let guard_block = &seg[guard_open..guard_close];
        assert!(
            !guard_block.contains("platform_limit_refusal("),
            "the enforcement must not sit inside the claim-guard block (dead after its return)"
        );
        let enforce_at = seg
            .find("platform_limit_refusal(")
            .expect("claim_by_id enforces the recorded platform limitation");
        assert!(
            enforce_at > guard_close,
            "the enforcement runs after the guard block, on the live by-id path"
        );
        // Behavior half — the decisions the now-reachable block
        // makes, pinned on the pure predicates it runs (the
        // claim_race idiom: no live store needed). A valid
        // limitation refuses every other family, and a claim
        // declaring no family at all:
        assert!(
            platform_limit_refusal(
                Some("muse"),
                Some("needs the muse surface"),
                Some("openchamber"),
            )
            .is_some()
        );
        assert!(
            platform_limit_refusal(Some("muse"), Some("needs the muse surface"), None,).is_some()
        );
        // The limited family itself is not refused:
        assert!(
            platform_limit_refusal(Some("muse"), Some("needs the muse surface"), Some("muse"),)
                .is_none()
        );
        // A reason-less stored limitation is invalid: the block
        // clears it, never enforces it.
        assert!(platform_limit_invalid(Some("muse"), None));
        assert!(platform_limit_refusal(Some("muse"), None, Some("openchamber")).is_none());
    }

    #[test]
    fn claim_by_id_targets_exactly_the_named_task_7735() {
        // #7735 oracle: a targeted claim is a claim of one row or
        // no claim — it can never fall through to the queue's
        // next-up row (the monitor's Duty 1B footgun: an untargeted
        // claim fired on whatever was next). The by-id path reads
        // the named row, flips it with an UPDATE keyed to that id
        // plus the status it read, and reads the result back by
        // the same id; it contains no pick at all.
        let src = &test_source();
        // Bound the segment at claim_by_id's own closing brace:
        // seg8217's usual end marker (claim_transfer) is defined
        // BEFORE claim_by_id in this file, so its segment would
        // run to EOF and the forbidden-needle checks below would
        // trip on the pick function further down.
        let seg = src
            .split("fn claim_by_id(")
            .nth(1)
            .unwrap_or("")
            .split("\n}\n")
            .next()
            .unwrap_or("");
        assert!(
            seg.contains("get_task(t, task_id)"),
            "by-id claim reads the named row"
        );
        assert!(
            seg.contains("WHERE id = ? AND status = ? AND queue = ?"),
            "by-id claim's UPDATE is keyed to the named id and the status it read"
        );
        assert!(
            seg.contains("SELECT {TASK_COLS} FROM tasks WHERE id = ?"),
            "by-id claim reads its result back by the named id"
        );
        for forbidden in ["claim_pick_once(", "ORDER BY priority"] {
            assert!(
                !seg.contains(forbidden),
                "by-id claim must never pick a different task: found {forbidden}"
            );
        }
        // Predicate half, in the claim_race idiom: the named row is
        // claimable while it is queued, and the instant it is
        // claimed (by anyone) a targeted claim of it refuses —
        // there is no state in which it silently substitutes the
        // next queued row, which stays claimable under its own id.
        let now = "2026-11-11T02:45:00Z";
        let mut named = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        assert!(by_id_claimable(&named, now));
        named["status"] = json!("claimed");
        assert!(!by_id_claimable(&named, now));
        let next = json!({
            "status": "queued", "attempts": 0, "max_attempts": 6,
            "not_before": null, "platform": null, "lane_id": null,
        });
        assert!(by_id_claimable(&next, now));
    }

    #[test]
    fn release_verb_contract_8217() {
        // Oracle: release is holder-checked and reason-required,
        // and returns the row to `queued` with its lease and
        // session binding cleared and a `released` audit event;
        // attempts are NOT handed back.
        let src = &test_source();
        let seg = seg8217(
            src,
            "/// Release a claimed task straight back",
            "\nfn verb_park(",
        );
        for needle in [
            "a release requires a reason",
            "claimed_by(",
            "\"released\"",
            "DELETE FROM leases",
            "attempt change",
            "SET status = 'queued'",
        ] {
            assert!(seg.contains(needle), "verb_release misses {needle}");
        }
        assert!(
            src.contains("\"/release\" => verb_release(t, req),"),
            "release is dispatched"
        );
    }

    #[test]
    fn one_queue_filter_parse_and_compile_8097() {
        // The designated lanes' expressions parse and compile:
        // not-code is `repo != code`; code-only is `repo = code`.
        let not_code = parse_filter("repo != code").expect("not-code parses");
        let mut args: Vec<Arg> = Vec::new();
        let sql = filter_sql(&not_code, &mut args);
        assert!(sql.contains("NOT"), "negation compiles: {sql}");
        assert!(
            sql.contains("json_extract(payload, '$.repo')"),
            "repo identity: {sql}"
        );
        assert_eq!(args.len(), 2, "repo equality binds the value twice: {sql}");
        match (&args[0], &args[1]) {
            (Arg::Text(a), Arg::Text(b)) => assert_eq!((a.as_str(), b.as_str()), ("code", "code")),
            _ => panic!("repo binds text values"),
        }
        // Boolean composition: AND over OR, NOT tightest.
        let expr = parse_filter("(repo = nix OR repo = opencode) AND NOT platform = muse")
            .expect("composed filter parses");
        let mut args: Vec<Arg> = Vec::new();
        let sql = filter_sql(&expr, &mut args);
        assert!(sql.contains(" OR "), "{sql}");
        assert!(sql.contains(" AND "), "{sql}");
        assert!(sql.contains("(NOT "), "{sql}");
        assert_eq!(args.len(), 5, "two repo values x2 + one platform: {sql}");
        // IN lists and priority ordering.
        let expr = parse_filter("repo IN (nix, opencode) AND priority <= 3").expect("IN parses");
        let mut args: Vec<Arg> = Vec::new();
        let sql = filter_sql(&expr, &mut args);
        assert!(sql.contains("priority <= ?"), "{sql}");
        assert!(
            matches!(args.last(), Some(Arg::Int(3))),
            "priority binds an integer"
        );
        // Malformed expressions are usage errors, never silent.
        for bad in [
            "",
            "repo < code",
            "priority <= soon",
            "bogus = x",
            "(repo = code",
            "repo = ",
            "repo IN ()",
            "repo ~ code",
        ] {
            assert!(parse_filter(bad).is_err(), "rejects {bad:?}");
        }
    }

    #[test]
    fn one_queue_union_match_is_empty_by_default_8097() {
        // Drew's model (2026-11-10): the union of the task's
        // requirements and the claimer's requirements, each
        // filtering on the other's metadata — and BOTH sides are
        // empty by default, so matching is pure priority order.
        let code_task = json!({
            "queue": "repo-health-code-pilot", "priority": 1,
            "payload": {"schema": "repo-task/v1", "repo": "developing-today/code", "kind": "small_fix"},
        });
        let nix_task = json!({
            "queue": "repo-health-nix-pilot", "priority": 2,
            "payload": {"schema": "repo-task/v1", "repo": "dezren39/darwin-nix", "kind": "small_fix"},
        });
        let stream_only_task = json!({
            "queue": "repo-health-code-pilot", "priority": 3,
            "payload": {"schema": "repo-task/v1", "kind": "triage"},
        });
        let meta_task = json!({
            "queue": "meta", "priority": 4, "payload": {},
        });
        // No requirements on any of these tasks: every claimer
        // meets them, whatever its identity — identity alone
        // never filters.
        let claimer = ClaimerCtx {
            family: Some("openchamber".to_string()),
            lane: Some("code-z".to_string()),
            worker: "repo-worker-code".to_string(),
            pool: None,
            model: None,
        };
        for task in [&code_task, &nix_task, &stream_only_task, &meta_task] {
            assert_eq!(task_requirements_unmet(task, &claimer), None);
        }
        // Claimer-side expressions over task metadata.
        let not_code = parse_filter("repo != code").unwrap();
        assert!(!filter_matches(&not_code, &code_task), "code task is code");
        assert!(filter_matches(&not_code, &nix_task), "nix task is not code");
        assert!(
            filter_matches(&not_code, &meta_task),
            "meta task is not code"
        );
        let code_only = parse_filter("repo = code").unwrap();
        assert!(
            filter_matches(&code_only, &code_task),
            "payload repo basename matches"
        );
        assert!(
            filter_matches(&code_only, &stream_only_task),
            "stream-derived identity matches a payload without repo"
        );
        assert!(!filter_matches(&code_only, &nix_task));
        let full_name = parse_filter("repo = developing-today/code").unwrap();
        assert!(
            filter_matches(&full_name, &code_task),
            "full owner/name matches"
        );
        // Priority and kind dimensions.
        let urgent_code = parse_filter("priority <= 2 AND kind = small_fix").unwrap();
        assert!(filter_matches(&urgent_code, &code_task));
        assert!(filter_matches(&urgent_code, &nix_task));
        assert!(
            !filter_matches(&urgent_code, &stream_only_task),
            "priority 3 is not <= 2"
        );
    }

    #[test]
    fn implementation_repo_sql_and_budget_matrix() {
        // Oracle: execute production expressions in isolated SQLite against a model.
        let output = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/implementation_repo_sql.py"
            ))
            .output()
            .expect("claim SQL tests require Python 3 with stdlib sqlite3");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn implementation_repo_filters_work_not_tracking_issue() {
        // Oracle: issue #10415 lives in code, but its deliverable is in Kaneo.
        // Tracking/dedupe identity must remain code; claim eligibility must not.
        let row = json!({"queue":"repo-health-code-pilot", "payload": {
            "repo":"developing-today/code", "implementation_repo":"developing-today/kaneo",
            "kind":"small_fix", "issue":{"number":10415}
        }});
        assert!(!filter_matches(&parse_filter("repo = code").unwrap(), &row));
        assert!(filter_matches(&parse_filter("repo = kaneo").unwrap(), &row));
        assert!(filter_matches(&parse_filter("repo != code").unwrap(), &row));
        let encoded =
            json!({"queue":"repo-health-code-pilot", "payload":row["payload"].to_string()});
        assert!(filter_matches(
            &parse_filter("repo = kaneo").unwrap(),
            &encoded
        ));
        assert_eq!(task_repo_identity(&row), "developing-today/code");
        assert_eq!(work_identity(&row).unwrap().repo, "developing-today/code");
        let unknown = json!({"payload":{"repo":"developing-today/code", "risk":{"flags":["cross-repo-change"]}}});
        assert!(
            claim_guard_reason(&unknown)
                .unwrap()
                .contains("implementation_repo")
        );
        let legacy =
            json!({"queue":"repo-health-code-pilot", "payload":{"repo":"developing-today/code"}});
        assert!(filter_matches(
            &parse_filter("repo = code").unwrap(),
            &legacy
        ));
    }

    #[test]
    fn one_queue_task_requirements_match_claimers_automatically_8097() {
        // A task's own declared requirements are honored inside
        // the claim: an ineligible claimer simply does not get
        // the item; no out-of-band step exists.
        let muse_only = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"platform": "muse"}},
        });
        let oc_claimer = ClaimerCtx {
            family: Some("openchamber".to_string()),
            ..Default::default()
        };
        let muse_claimer = ClaimerCtx {
            family: Some("muse".to_string()),
            ..Default::default()
        };
        let anon_claimer = ClaimerCtx::default();
        assert!(task_requirements_unmet(&muse_only, &oc_claimer).is_some());
        assert!(task_requirements_unmet(&muse_only, &anon_claimer).is_some());
        assert_eq!(task_requirements_unmet(&muse_only, &muse_claimer), None);
        // Lane requirement: only the named lane.
        let lane_task = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"lane": "code-b"}},
        });
        let lane_b = ClaimerCtx {
            lane: Some("code-b".to_string()),
            ..Default::default()
        };
        let lane_c = ClaimerCtx {
            lane: Some("code-c".to_string()),
            ..Default::default()
        };
        assert_eq!(task_requirements_unmet(&lane_task, &lane_b), None);
        assert!(task_requirements_unmet(&lane_task, &lane_c).is_some());
        assert!(task_requirements_unmet(&lane_task, &anon_claimer).is_some());
        // Pool and model requirements.
        let pooled = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"pool": "summit", "model": "claude-sonnet-5-5"}},
        });
        let summit = ClaimerCtx {
            pool: Some("summit".to_string()),
            model: Some(ClaimModel {
                provider: "_claude".to_string(),
                model: Some("claude-sonnet-5-5".to_string()),
                variant: None,
            }),
            ..Default::default()
        };
        assert_eq!(task_requirements_unmet(&pooled, &summit), None);
        let wrong_pool = ClaimerCtx {
            pool: Some("workhorse".to_string()),
            ..summit.clone()
        };
        assert!(task_requirements_unmet(&pooled, &wrong_pool).is_some());
        // The full "provider/model" spelling also meets a model
        // requirement, and a model-less claimer meets none.
        let full_model_task = json!({
            "queue": "meta", "priority": 1,
            "payload": {"requires": {"model": "_claude/claude-sonnet-5-5"}},
        });
        assert_eq!(task_requirements_unmet(&full_model_task, &summit), None);
        assert!(task_requirements_unmet(&full_model_task, &anon_claimer).is_some());
    }

    #[test]
    fn one_queue_scope_sql_shape_8097() {
        let claimer = ClaimerCtx {
            family: Some("muse".to_string()),
            lane: Some("nix-a".to_string()),
            worker: "repo-worker-nix".to_string(),
            pool: None,
            model: None,
        };
        // The default claim: no stream, no filter — the scope is
        // exactly the task-requirements clause (5 bindings), so
        // the pick spans the whole queue in priority order.
        let scope = claim_scope(None, None, &claimer);
        assert!(!scope.sql.contains("queue = ?"), "unscoped: {}", scope.sql);
        assert!(
            scope.sql.contains("requires.platform"),
            "requirements always bind"
        );
        assert_eq!(scope.args.len(), 5);
        // A stream-scoped claim keeps the old shape: queue first.
        let scope = claim_scope(Some("repo-health-code-pilot"), None, &claimer);
        assert!(scope.sql.starts_with(" AND queue = ?"), "{}", scope.sql);
        match &scope.args[0] {
            Arg::Text(q) => assert_eq!(q, "repo-health-code-pilot"),
            _ => panic!("stream binds first"),
        }
        assert_eq!(scope.args.len(), 6);
        // A filtered claim: filter args follow the stream arg and
        // precede the requirement bindings.
        let expr = parse_filter("repo != code").unwrap();
        let scope = claim_scope(None, Some(&expr), &claimer);
        assert_eq!(scope.args.len(), 7);
        match (&scope.args[0], &scope.args[1]) {
            (Arg::Text(a), Arg::Text(b)) => {
                assert_eq!((a.as_str(), b.as_str()), ("code", "code"))
            }
            _ => panic!("filter binds before requirements"),
        }
        // The registry's unbound markers.
        assert!(!lane_queue_binds("*"));
        assert!(!lane_queue_binds("all"));
        assert!(!lane_queue_binds(""));
        assert!(lane_queue_binds("repo-health-code-pilot"));
    }

    #[test]
    fn one_queue_claim_wiring_8097() {
        // Source oracle: the claim verb parses the filter, builds
        // the union scope, and the single pick applies it in both
        // statements; the registry binding is consulted only for
        // stream-scoped claims.
        let src = &test_source();
        let single = seg8217(src, "fn verb_claim(", "\nfn verb_bind(");
        assert!(single.contains("parse_filter("), "claim parses --filter");
        assert!(
            single.contains("claim_scope("),
            "claim builds the union scope"
        );
        assert!(
            single.contains("\"filter\""),
            "claim reads the filter field"
        );
        assert!(single.contains("\"pool\""), "claim reads the pool identity");
        let pick = seg8217(src, "fn claim_pick_once(", "\nfn parse_batch_count(");
        assert!(pick.contains("scope.sql"), "pick SELECT applies the scope");
        assert!(
            pick.contains("scope.args"),
            "pick binds the scope arguments"
        );
        let preamble = seg8217(src, "fn claim_preamble(", "\nfn claim_pick_once(");
        assert!(
            preamble.contains("lane_queue_binds("),
            "preamble honors the unbound registration marker"
        );
        let by_id = seg8217(src, "fn claim_by_id(", "\nfn claim_transfer(");
        assert!(
            by_id.contains("task_requirements_unmet("),
            "by-id claim honors the task's requirements"
        );
    }
}
