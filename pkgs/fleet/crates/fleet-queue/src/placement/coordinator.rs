//! Placement coordinator client — the seat's half of the central
//! placement coordinator (#13765).
//!
//! The queue backend's coordinator verbs (see
//! `crate::turso_queue::placement_coord`) hold the cross-seat true
//! load view, the fleet-wide Blacksmith budget, and the waitlist.
//! This module is the best-effort client that consults them before
//! dispatching, in the `placement/remote.rs` pattern: every
//! operation is allowed to fail, and a failure means "coordinator
//! unreachable" — the caller dispatches exactly as the
//! pre-coordinator client did (the singular fallback), with the
//! session record and output naming `placement_path:
//! "singular-fallback"` plus the error. Coordinator-served paths
//! name `placement_path: "coordinator"`.
//!
//! The operator escape hatch is `FLEET_PLACE_COORD=off`: every call
//! fails, so the client stays on the singular path, identical to the
//! pre-coordinator client.

use serde_json::{Map, Value, json};
use std::time::Duration;

/// One coordinator roundtrip's budget. The broker is on the tailnet
/// or the same host; five seconds is generous for the claim SQL and
/// far below the queue client's 300s legs.
pub const COORD_BUDGET: Duration = Duration::from_secs(5);

/// The minutes a claim reserves against the Blacksmith budget before
/// the run's real duration is known (settled by release).
pub const CLAIM_ESTIMATE_MINUTES: f64 = 10.0;

/// The queue-backend half of the placement coordinator. The
/// production implementation is [`BrokerCoordinator`]; tests
/// substitute a fake. All methods are best-effort by contract: an
/// Err means "coordinator unreachable", never a dispatch failure.
pub trait CoordinatorRemote {
    /// Hold a slot for this dispatch (`/placement-claim`).
    fn claim(
        &self,
        handle: &str,
        surface: &str,
        seat: &str,
        est_minutes: f64,
        allow_paid: f64,
    ) -> Result<Value, String>;
    /// Confirm a granted slot after dispatch (`/placement-confirm`).
    fn confirm(&self, handle: &str, run_id: Option<i64>) -> Result<Value, String>;
    /// Give a slot back (`/placement-release`).
    fn release(&self, handle: &str, actual_minutes: Option<f64>) -> Result<Value, String>;
    /// One claim plus the owner's true view, resolvable by any seat
    /// (`/placement-claim-status`).
    fn claim_status(&self, handle: &str) -> Result<Value, String>;
    /// Every owner's true view (`/placement-load`).
    fn load(&self) -> Result<Value, String>;
}

/// The production remote: the coordinator verbs over the bounded
/// broker transport.
pub struct BrokerCoordinator;

impl BrokerCoordinator {
    fn call(&self, path: &str, body: &Value) -> Result<Value, String> {
        let resp = crate::client::broker_roundtrip_bounded(path, body, COORD_BUDGET)?;
        if resp.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(resp)
        } else {
            Err(resp
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("queue backend refused the coordinator call")
                .to_string())
        }
    }
}

impl CoordinatorRemote for BrokerCoordinator {
    fn claim(
        &self,
        handle: &str,
        surface: &str,
        seat: &str,
        est_minutes: f64,
        allow_paid: f64,
    ) -> Result<Value, String> {
        self.call(
            "/placement-claim",
            &json!({
                "handle": handle, "surface": surface, "seat": seat,
                "est_minutes": est_minutes, "allow_paid": allow_paid,
            }),
        )
    }

    fn confirm(&self, handle: &str, run_id: Option<i64>) -> Result<Value, String> {
        let mut body = json!({"handle": handle});
        if let Some(id) = run_id {
            body["run_id"] = json!(id);
        }
        self.call("/placement-confirm", &body)
    }

    fn release(&self, handle: &str, actual_minutes: Option<f64>) -> Result<Value, String> {
        let mut body = json!({"handle": handle});
        if let Some(a) = actual_minutes {
            body["actual_minutes"] = json!(a);
        }
        self.call("/placement-release", &body)
    }

    fn claim_status(&self, handle: &str) -> Result<Value, String> {
        self.call("/placement-claim-status", &json!({"handle": handle}))
    }

    fn load(&self) -> Result<Value, String> {
        self.call("/placement-load", &json!({}))
    }
}

/// The disabled remote (`FLEET_PLACE_COORD=off`): every call fails,
/// which callers already treat as "coordinator unreachable" —
/// singular-path behavior, identical to the pre-coordinator client.
pub struct NullCoordinator;

impl CoordinatorRemote for NullCoordinator {
    fn claim(&self, _h: &str, _s: &str, _t: &str, _e: f64, _a: f64) -> Result<Value, String> {
        Err("placement coordinator disabled".to_string())
    }
    fn confirm(&self, _h: &str, _r: Option<i64>) -> Result<Value, String> {
        Err("placement coordinator disabled".to_string())
    }
    fn release(&self, _h: &str, _a: Option<f64>) -> Result<Value, String> {
        Err("placement coordinator disabled".to_string())
    }
    fn claim_status(&self, _h: &str) -> Result<Value, String> {
        Err("placement coordinator disabled".to_string())
    }
    fn load(&self) -> Result<Value, String> {
        Err("placement coordinator disabled".to_string())
    }
}

/// The coordinator for this process: the broker verbs unless the
/// operator escape hatch `FLEET_PLACE_COORD=off` is set.
pub fn coordinator() -> Box<dyn CoordinatorRemote> {
    match std::env::var("FLEET_PLACE_COORD") {
        Ok(v) if v == "off" => Box::new(NullCoordinator),
        _ => Box::new(BrokerCoordinator),
    }
}

/// This seat's name in claim rows: the same `user@host` identity
/// session records carry.
pub fn seat_name() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{user}@{host}")
}

/// What the claim answer means for this dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDecision {
    /// Slot held: dispatch as today, then confirm with the run id.
    Granted,
    /// Owner full: commit the session `queued`, dispatch nothing.
    Queued,
    /// Budget spent: re-decide with Blacksmith removed.
    Refused,
    /// Coordinator unreachable: today's path verbatim.
    Unreachable,
}

/// Classify a `/placement-claim` answer (or its failure) into the
/// dispatch decision. A `refused` answer carries no claim row —
/// nothing polls it. Any transport or backend error is the singular
/// fallback, never a dispatch failure.
pub fn classify_claim(
    answer: Result<Value, String>,
) -> (ClaimDecision, Option<Value>, Option<String>) {
    match answer {
        Err(e) => (ClaimDecision::Unreachable, None, Some(e)),
        Ok(v) => match v.get("status").and_then(|s| s.as_str()) {
            Some("claimed") | Some("granted") => (ClaimDecision::Granted, Some(v), None),
            Some("queued") => (ClaimDecision::Queued, Some(v), None),
            Some("refused") => (ClaimDecision::Refused, Some(v), None),
            other => (
                ClaimDecision::Unreachable,
                None,
                Some(format!(
                    "coordinator claim answered without a usable status: {}",
                    other.unwrap_or("missing")
                )),
            ),
        },
    }
}

/// The `placement_path` the session record and output carry: the
/// singular fallback names itself plus the error; every
/// coordinator-served path (granted, queued, or refused-then-placed)
/// names the coordinator.
pub fn placement_path(decision: &ClaimDecision) -> &'static str {
    match decision {
        ClaimDecision::Unreachable => "singular-fallback",
        ClaimDecision::Granted | ClaimDecision::Queued | ClaimDecision::Refused => "coordinator",
    }
}

/// True when the claim answer says to dispatch now (granted, or the
/// unreachable fallback). Queued and refused dispatch nothing on
/// this pass.
pub fn dispatch_now(decision: &ClaimDecision) -> bool {
    matches!(
        decision,
        ClaimDecision::Granted | ClaimDecision::Unreachable
    )
}

/// A clone of the registry with every Blacksmith surface removed —
/// the budget-refusal re-decide. Owner entries stay untouched (caps
/// are per owner, not per surface, and the coordinator still gates
/// on them).
pub fn registry_without_blacksmith(registry: &Value) -> Value {
    let mut reg = registry.clone();
    if let Some(surfaces) = reg.get_mut("surfaces").and_then(|s| s.as_array_mut()) {
        surfaces.retain(|s| s.get("kind").and_then(|k| k.as_str()) != Some("blacksmith"));
    }
    reg
}

/// `--assume-owner-load`-style overrides from the coordinator's true
/// view (`/placement-load`): `owner=occupancy` per owner, sorted, so
/// the seat re-decides against the counts it could not see. Owners
/// the coordinator has never seen keep their local figure (absent
/// from the string).
pub fn coord_overrides(load: &Value) -> String {
    let mut pairs: Vec<(String, i64)> = load
        .get("owners")
        .and_then(|o| o.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|o| {
            let owner = o
                .get("owner")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let occupancy = o.get("occupancy").and_then(|v| v.as_i64()).unwrap_or(0);
            (owner, occupancy)
        })
        .filter(|(owner, _)| !owner.is_empty())
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
        .iter()
        .map(|(owner, occupancy)| format!("{owner}={occupancy}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One owner's true view as a single operator line for the
/// `placement-load` front door (#15900): the GitHub counts the
/// coordinator sweep last read, the active coordinator claims, the
/// resulting occupancy against the owner's cap, and the read's age
/// evidence (`load_read_at` / `load_source` — the sweep freshness
/// fields; a null read time means no sweep has read GitHub yet).
pub fn format_owner_load(owner: &Value) -> String {
    let field = |name: &str| -> String {
        match owner.get(name) {
            Some(Value::Null) | None => "-".to_string(),
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
        }
    };
    format!(
        "owner={} cap={} gh_running={} gh_queued={} claims_active={} occupancy={} load_read_at={} load_source={}",
        field("owner"),
        field("cap"),
        field("gh_running"),
        field("gh_queued"),
        field("claims_active"),
        field("occupancy"),
        field("load_read_at"),
        field("load_source"),
    )
}

/// The `fleet queue placement-load` report (#15900): the read-only
/// front door over the coordinator's `/placement-load` view, so an
/// operator (or a verification pass) can inspect observed GitHub
/// load, active claims, and sweep freshness without dispatching.
/// Pure over the coordinator answer so the read path and its
/// broker-unavailable behavior are testable with a fake remote:
/// returns (exit code, stdout, stderr). Success prints the owners
/// (pretty JSON with `as_json`, operator lines otherwise); a
/// coordinator failure is exit 3 naming the unavailability — never
/// a fabricated empty load view.
pub fn placement_load_report(answer: Result<Value, String>, as_json: bool) -> (i32, String, String) {
    match answer {
        Err(e) => (
            3,
            String::new(),
            format!("repo-queue placement-load: coordinator unavailable: {e}\n"),
        ),
        Ok(load) => {
            if as_json {
                let text = serde_json::to_string_pretty(&load)
                    .unwrap_or_else(|_| "null".to_string());
                return (0, format!("{text}\n"), String::new());
            }
            let mut out = String::from("placement load (broker coordinator /placement-load):\n");
            let owners = load
                .get("owners")
                .and_then(|o| o.as_array())
                .cloned()
                .unwrap_or_default();
            if owners.is_empty() {
                out.push_str("(no owners in the coordinator view)\n");
            } else {
                for owner in &owners {
                    out.push_str(&format_owner_load(owner));
                    out.push('\n');
                }
            }
            (0, out, String::new())
        }
    }
}

/// The queued-session record: the shape the all-tiers-exhausted path
/// already writes (full job, surface, reasoning, coverage), plus the
/// claim id and the coordinator path. Committed without any GitHub
/// dispatch; a later `session status` polls the claim and dispatches
/// on grant.
#[allow(clippy::too_many_arguments)]
pub fn queued_session_record(
    handle: &str,
    job_json: Value,
    surface_id: &str,
    claim_handle: &str,
    reasoning: &[String],
    coverage: Value,
    submitted_by: &str,
    actor: &str,
    created_at: &str,
) -> Value {
    let mut rec = Map::new();
    rec.insert("handle".into(), json!(handle));
    rec.insert("job".into(), job_json);
    rec.insert("surface".into(), json!(surface_id));
    rec.insert("claim_handle".into(), json!(claim_handle));
    rec.insert("placement_path".into(), json!("coordinator"));
    rec.insert("decision_reasoning".into(), json!(reasoning));
    rec.insert("owner_load_coverage".into(), coverage);
    rec.insert("run_id".into(), Value::Null);
    rec.insert("run_url".into(), Value::Null);
    rec.insert("state".into(), json!("queued"));
    rec.insert("submitted_by".into(), json!(submitted_by));
    rec.insert("actor".into(), json!(actor));
    rec.insert("created_at".into(), json!(created_at));
    rec.insert("updated_at".into(), json!(created_at));
    Value::Object(rec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// In-memory coordinator: scripted claim answers, recording
    /// confirm/release calls, and a switchable failing mode (the
    /// coordinator-down case).
    struct FakeCoordinator {
        claim_answer: RefCell<Result<Value, String>>,
        confirms: RefCell<Vec<(String, Option<i64>)>>,
        releases: RefCell<Vec<(String, Option<f64>)>>,
        load_answer: RefCell<Result<Value, String>>,
    }

    impl FakeCoordinator {
        fn granted() -> Self {
            FakeCoordinator {
                claim_answer: RefCell::new(Ok(json!({"status": "claimed", "handle": "fp_x"}))),
                confirms: RefCell::new(Vec::new()),
                releases: RefCell::new(Vec::new()),
                load_answer: RefCell::new(Err("unused".to_string())),
            }
        }
        fn queued() -> Self {
            FakeCoordinator {
                claim_answer: RefCell::new(Ok(json!({
                    "status": "queued", "handle": "fp_x", "position": 1,
                }))),
                confirms: RefCell::new(Vec::new()),
                releases: RefCell::new(Vec::new()),
                load_answer: RefCell::new(Err("unused".to_string())),
            }
        }
        fn refused() -> Self {
            FakeCoordinator {
                claim_answer: RefCell::new(Ok(json!({
                    "status": "refused", "reason": "budget-exhausted",
                }))),
                confirms: RefCell::new(Vec::new()),
                releases: RefCell::new(Vec::new()),
                load_answer: RefCell::new(Err("unused".to_string())),
            }
        }
        fn down() -> Self {
            FakeCoordinator {
                claim_answer: RefCell::new(Err("connection refused".to_string())),
                confirms: RefCell::new(Vec::new()),
                releases: RefCell::new(Vec::new()),
                load_answer: RefCell::new(Err("unused".to_string())),
            }
        }
    }

    impl CoordinatorRemote for FakeCoordinator {
        fn claim(&self, _h: &str, _s: &str, _t: &str, _e: f64, _a: f64) -> Result<Value, String> {
            self.claim_answer.borrow().clone()
        }
        fn confirm(&self, handle: &str, run_id: Option<i64>) -> Result<Value, String> {
            self.confirms
                .borrow_mut()
                .push((handle.to_string(), run_id));
            Ok(json!({"confirmed": true}))
        }
        fn release(&self, handle: &str, actual: Option<f64>) -> Result<Value, String> {
            self.releases
                .borrow_mut()
                .push((handle.to_string(), actual));
            Ok(json!({"released": true}))
        }
        fn claim_status(&self, _h: &str) -> Result<Value, String> {
            Err("unused".to_string())
        }
        fn load(&self) -> Result<Value, String> {
            self.load_answer.borrow().clone()
        }
    }

    /// Test 5a: the coordinator error falls back to the singular
    /// path — the dispatch still succeeds (dispatch_now), and the
    /// record/output name `singular-fallback` with the error.
    #[test]
    fn coordinator_error_falls_back_and_names_singular_fallback() {
        let remote = FakeCoordinator::down();
        let (decision, _, error) =
            classify_claim(remote.claim("fp_x", "fleet-org", "seat", 10.0, 0.0));
        assert_eq!(decision, ClaimDecision::Unreachable);
        // The dispatch still succeeds: unreachable dispatches today.
        assert!(dispatch_now(&decision));
        assert_eq!(placement_path(&decision), "singular-fallback");
        assert!(error.unwrap().contains("connection refused"));
    }

    /// Test 5b: a granted slot dispatches and confirms — confirm is
    /// called with the run id, the path names `coordinator`, and a
    /// dispatch failure would release (the fake records both legs).
    #[test]
    fn granted_slot_confirms_and_names_coordinator() {
        let remote = FakeCoordinator::granted();
        let (decision, _, _) = classify_claim(remote.claim("fp_x", "fleet-org", "seat", 10.0, 0.0));
        assert_eq!(decision, ClaimDecision::Granted);
        assert!(dispatch_now(&decision));
        assert_eq!(placement_path(&decision), "coordinator");
        // Granted dispatches, then confirms with the run id.
        remote.confirm("fp_x", Some(4242)).unwrap();
        assert_eq!(
            remote.confirms.borrow().as_slice(),
            &[("fp_x".to_string(), Some(4242))]
        );
        // A dispatch failure releases the held slot instead.
        remote.release("fp_x", None).unwrap();
        assert_eq!(
            remote.releases.borrow().as_slice(),
            &[("fp_x".to_string(), None)]
        );
    }

    /// Test 5c: a queued claim commits the session queued and
    /// dispatches nothing to GitHub — the record carries state
    /// queued, the surface, and the claim id, on the coordinator
    /// path.
    #[test]
    fn queued_claim_commits_queued_session_without_dispatch() {
        let remote = FakeCoordinator::queued();
        let (decision, answer, _) =
            classify_claim(remote.claim("fp_x", "fleet-org", "seat", 10.0, 0.0));
        assert_eq!(decision, ClaimDecision::Queued);
        // Queued dispatches nothing on this pass.
        assert!(!dispatch_now(&decision));
        assert_eq!(placement_path(&decision), "coordinator");
        let claim_handle = answer
            .as_ref()
            .and_then(|v| v.get("handle"))
            .and_then(|h| h.as_str())
            .unwrap()
            .to_string();
        let rec = queued_session_record(
            "fp_x",
            json!({"repo": "o/r", "command": "nix build"}),
            "fleet-org",
            &claim_handle,
            &["tier 1 saturated; coordinator queued at position 1".to_string()],
            json!({}),
            "archer@test",
            "user token (dispatch_as: user)",
            "2026-11-24T00:00:00Z",
        );
        assert_eq!(rec["state"], "queued");
        assert_eq!(rec["surface"], "fleet-org");
        assert_eq!(rec["claim_handle"], "fp_x");
        assert_eq!(rec["placement_path"], "coordinator");
        assert_eq!(rec["actor"], "user token (dispatch_as: user)");
        // No dispatch leg ran: no confirm, no release, no GitHub call.
        assert!(remote.confirms.borrow().is_empty());
        assert!(remote.releases.borrow().is_empty());
    }

    #[test]
    fn refused_claim_redacts_blacksmith_and_stays_coordinator() {
        let remote = FakeCoordinator::refused();
        let (decision, answer, _) =
            classify_claim(remote.claim("fp_x", "fleet-org-blacksmith", "seat", 10.0, 0.0));
        assert_eq!(decision, ClaimDecision::Refused);
        assert!(!dispatch_now(&decision));
        assert_eq!(placement_path(&decision), "coordinator");
        assert_eq!(answer.unwrap()["reason"], "budget-exhausted");
        let reg = json!({"surfaces": [
            {"id": "fleet-org", "kind": "github-actions"},
            {"id": "fleet-org-blacksmith", "kind": "blacksmith"},
        ]});
        let redacted = registry_without_blacksmith(&reg);
        let ids: Vec<&str> = redacted["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.get("id").and_then(|i| i.as_str()))
            .collect();
        assert_eq!(ids, vec!["fleet-org"]);
    }

    /// #15900: the placement-load front door renders the true view
    /// — GitHub counts, active claims, occupancy, and the sweep
    /// freshness fields — and its JSON form is the broker's answer
    /// verbatim. Oracle: the canned `/placement-load` answer.
    #[test]
    fn placement_load_report_renders_the_true_view() {
        let remote = FakeCoordinator::granted();
        *remote.load_answer.borrow_mut() = Ok(json!({"owners": [
            {"owner": "dezren39", "cap": 40, "gh_running": 3, "gh_queued": 1,
             "claims_active": 2, "occupancy": 6,
             "load_read_at": "2026-11-27T03:40:00Z", "load_source": "BrokerCoordGh"},
            {"owner": "developing-today", "cap": 20, "gh_running": 0, "gh_queued": 0,
             "claims_active": 0, "occupancy": 0,
             "load_read_at": null, "load_source": null},
        ]}));
        let (code, out, err) = placement_load_report(remote.load(), false);
        assert_eq!(code, 0);
        assert!(err.is_empty());
        assert!(out.contains(
            "owner=dezren39 cap=40 gh_running=3 gh_queued=1 claims_active=2 \
             occupancy=6 load_read_at=2026-11-27T03:40:00Z load_source=BrokerCoordGh"
        ));
        // A never-swept owner shows the freshness fields as absent,
        // not as a fabricated read.
        assert!(out.contains("owner=developing-today"));
        assert!(out.contains("load_read_at=- load_source=-"));
        let (code, out, _) = placement_load_report(remote.load(), true);
        assert_eq!(code, 0);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["owners"][0]["occupancy"], 6);
    }

    /// #15900: a coordinator failure is an unavailable report
    /// (exit 3, the error named on stderr), never an empty load
    /// view on stdout. Oracle: the fake remote's transport error.
    #[test]
    fn placement_load_report_names_coordinator_unavailable() {
        let remote = FakeCoordinator::down();
        *remote.load_answer.borrow_mut() = Err("connection refused".to_string());
        let (code, out, err) = placement_load_report(remote.load(), false);
        assert_eq!(code, 3);
        assert!(out.is_empty(), "no fabricated load view: {out:?}");
        assert!(err.contains("coordinator unavailable: connection refused"));
    }

    #[test]
    fn coord_load_becomes_assume_overrides() {
        let load = json!({"owners": [
            {"owner": "dezren39", "occupancy": 41},
            {"owner": "developing-today", "occupancy": 3},
        ]});
        assert_eq!(coord_overrides(&load), "developing-today=3,dezren39=41");
        assert_eq!(coord_overrides(&json!({})), "");
    }
}
