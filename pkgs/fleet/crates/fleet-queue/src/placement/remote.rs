//! Placement session mirror to the queue backend (#13146).
//!
//! The local JSON store stays the always-available store and the
//! ONLY input to placement decisions; this module is the
//! best-effort mirror that makes session records visible from
//! every seat. Every operation is allowed to fail — callers treat
//! an Err as "backend unreachable" and proceed exactly as the
//! pre-mirror client did. The transport is the queue client's
//! bounded roundtrip (see client::transport): short budget, errors
//! returned, never a process exit, never a broker auto-spawn, so a
//! dispatch can neither fail nor stall on the mirror.

use serde_json::{Value, json};
use std::time::Duration;

/// One mirror roundtrip's budget. The broker is on the tailnet or
/// the same host; five seconds is generous for four small SQL
/// statements and far below the queue client's 300s legs.
pub const REMOTE_BUDGET: Duration = Duration::from_secs(5);

/// How many local records one migration sweep pushes. The backend
/// caps /placement-sync at 500; 100 keeps each sweep's body small
/// and bounds the work a single dispatch/list call can do.
pub const SYNC_BATCH: usize = 100;

/// The queue-backend half of the placement session store. The
/// production implementation is [`BrokerRemote`]; tests substitute
/// a fake. All methods are best-effort by contract.
pub trait SessionRemote {
    /// Upsert one session record (state refreshes included).
    fn put(&self, rec: &Value) -> Result<(), String>;
    /// The backend's record for a handle, if it holds one.
    fn get(&self, handle: &str) -> Result<Option<Value>, String>;
    /// Newest-first session records, optionally one submitter's.
    fn list(&self, submitted_by: Option<&str>, limit: usize) -> Result<Vec<Value>, String>;
    /// Insert-or-ignore a batch of local records (migration);
    /// returns how many were newly inserted.
    fn sync(&self, records: &[Value]) -> Result<i64, String>;
}

/// The production remote: the queue broker's /placement-* verbs
/// over the bounded transport.
pub struct BrokerRemote;

impl BrokerRemote {
    fn call(&self, path: &str, body: &Value) -> Result<Value, String> {
        let resp = crate::client::broker_roundtrip_bounded(path, body, REMOTE_BUDGET)?;
        if resp.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(resp)
        } else {
            Err(resp
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("queue backend refused the placement mirror call")
                .to_string())
        }
    }
}

impl SessionRemote for BrokerRemote {
    fn put(&self, rec: &Value) -> Result<(), String> {
        self.call("/placement-put", &json!({"record": rec}))?;
        Ok(())
    }

    fn get(&self, handle: &str) -> Result<Option<Value>, String> {
        let resp = self.call("/placement-get", &json!({"handle": handle}))?;
        match resp.get("record") {
            Some(Value::Null) | None => Ok(None),
            Some(rec) => Ok(Some(rec.clone())),
        }
    }

    fn list(&self, submitted_by: Option<&str>, limit: usize) -> Result<Vec<Value>, String> {
        let mut body = json!({"limit": limit});
        if let Some(who) = submitted_by {
            body["submitted_by"] = json!(who);
        }
        let resp = self.call("/placement-list", &body)?;
        Ok(resp
            .get("sessions")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default())
    }

    fn sync(&self, records: &[Value]) -> Result<i64, String> {
        let resp = self.call("/placement-sync", &json!({"records": records}))?;
        Ok(resp.get("inserted").and_then(|i| i.as_i64()).unwrap_or(0))
    }
}

/// The disabled remote (`FLEET_PLACE_REMOTE=off`): every call
/// fails, which callers already treat as "backend unreachable" —
/// local-only behavior, identical to the pre-#13146 client.
pub struct NullRemote;

impl SessionRemote for NullRemote {
    fn put(&self, _rec: &Value) -> Result<(), String> {
        Err("placement remote disabled".to_string())
    }
    fn get(&self, _handle: &str) -> Result<Option<Value>, String> {
        Err("placement remote disabled".to_string())
    }
    fn list(&self, _submitted_by: Option<&str>, _limit: usize) -> Result<Vec<Value>, String> {
        Err("placement remote disabled".to_string())
    }
    fn sync(&self, _records: &[Value]) -> Result<i64, String> {
        Err("placement remote disabled".to_string())
    }
}

/// The remote for this process: the broker mirror unless the
/// operator escape hatch `FLEET_PLACE_REMOTE=off` is set.
pub fn session_remote() -> Box<dyn SessionRemote> {
    match std::env::var("FLEET_PLACE_REMOTE") {
        Ok(v) if v == "off" => Box::new(NullRemote),
        _ => Box::new(BrokerRemote),
    }
}

fn updated_at(rec: &Value) -> &str {
    rec.get("updated_at").and_then(|v| v.as_str()).unwrap_or("")
}

fn created_at(rec: &Value) -> &str {
    rec.get("created_at").and_then(|v| v.as_str()).unwrap_or("")
}

fn handle_of(rec: &Value) -> &str {
    rec.get("handle").and_then(|v| v.as_str()).unwrap_or("")
}

/// Pick between the local and backend copies of one session: the
/// record with the newer `updated_at` wins; a tie goes to the
/// backend copy (it is the shared record other seats converge on).
pub fn pick_session(local: Option<Value>, remote: Option<Value>) -> Option<Value> {
    match (local, remote) {
        (Some(l), Some(r)) => {
            if updated_at(&r) >= updated_at(&l) {
                Some(r)
            } else {
                Some(l)
            }
        }
        (Some(l), None) => Some(l),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}

/// Union two session lists by handle (per-handle winner by
/// [`pick_session`]), newest first — the `session list` view across
/// the local store and the backend. No record present in either
/// input is dropped.
pub fn merge_session_records(local: Vec<Value>, remote: Vec<Value>) -> Vec<Value> {
    let mut by_handle: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for rec in local {
        by_handle.insert(handle_of(&rec).to_string(), rec);
    }
    for rec in remote {
        let key = handle_of(&rec).to_string();
        let cur = by_handle.remove(&key);
        if let Some(winner) = pick_session(cur, Some(rec)) {
            by_handle.insert(key, winner);
        }
    }
    let mut out: Vec<Value> = by_handle.into_values().collect();
    out.sort_by(|a, b| created_at(b).cmp(created_at(a)));
    out
}

/// Best-effort migration sweep: push the local store's most recent
/// records to the backend, which inserts-or-ignores them — a
/// record the backend already holds is never overwritten, so the
/// sweep is idempotent and safe to run on every dispatch and list.
/// Failures are the caller's signal to stay local-only; nothing
/// here is allowed to surface as an error.
pub fn sync_local_sessions(remote: &dyn SessionRemote, store: &Value) {
    let mut recs: Vec<Value> = store
        .get("sessions")
        .and_then(|s| s.as_object())
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();
    if recs.is_empty() {
        return;
    }
    recs.sort_by(|a, b| created_at(b).cmp(created_at(a)));
    recs.truncate(SYNC_BATCH);
    let _ = remote.sync(&recs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn rec(handle: &str, created: &str, updated: &str, state: &str) -> Value {
        json!({
            "handle": handle,
            "state": state,
            "created_at": created,
            "updated_at": updated,
            "submitted_by": "archer@test",
        })
    }

    /// In-memory remote: records calls, serves stored records,
    /// and can be switched to failing (the backend-down case).
    struct FakeRemote {
        stored: RefCell<std::collections::HashMap<String, Value>>,
        fail: RefCell<bool>,
        syncs: RefCell<Vec<Vec<Value>>>,
    }

    impl FakeRemote {
        fn new() -> Self {
            FakeRemote {
                stored: RefCell::new(std::collections::HashMap::new()),
                fail: RefCell::new(false),
                syncs: RefCell::new(Vec::new()),
            }
        }
    }

    impl SessionRemote for FakeRemote {
        fn put(&self, rec: &Value) -> Result<(), String> {
            if *self.fail.borrow() {
                return Err("backend down".to_string());
            }
            self.stored
                .borrow_mut()
                .insert(handle_of(rec).to_string(), rec.clone());
            Ok(())
        }
        fn get(&self, handle: &str) -> Result<Option<Value>, String> {
            if *self.fail.borrow() {
                return Err("backend down".to_string());
            }
            Ok(self.stored.borrow().get(handle).cloned())
        }
        fn list(&self, _who: Option<&str>, _limit: usize) -> Result<Vec<Value>, String> {
            if *self.fail.borrow() {
                return Err("backend down".to_string());
            }
            Ok(self.stored.borrow().values().cloned().collect())
        }
        fn sync(&self, records: &[Value]) -> Result<i64, String> {
            if *self.fail.borrow() {
                return Err("backend down".to_string());
            }
            self.syncs.borrow_mut().push(records.to_vec());
            let mut stored = self.stored.borrow_mut();
            let mut inserted = 0;
            for rec in records {
                let key = handle_of(rec).to_string();
                if !stored.contains_key(&key) {
                    stored.insert(key, rec.clone());
                    inserted += 1;
                }
            }
            Ok(inserted)
        }
    }

    #[test]
    fn pick_session_prefers_the_newer_record() {
        // Oracle: updated_at decides; ties go to the backend copy.
        let older = rec("fp_a", "2026-11-23T08:00:00Z", "2026-11-23T08:01:00Z", "running");
        let newer = rec("fp_a", "2026-11-23T08:00:00Z", "2026-11-23T08:02:00Z", "succeeded");
        assert_eq!(
            pick_session(Some(older.clone()), Some(newer.clone())).unwrap(),
            newer
        );
        assert_eq!(
            pick_session(Some(newer.clone()), Some(older.clone())).unwrap(),
            newer
        );
        let tied_remote = rec("fp_a", "2026-11-23T08:00:00Z", "2026-11-23T08:02:00Z", "failed");
        assert_eq!(
            pick_session(Some(newer), Some(tied_remote.clone())).unwrap(),
            tied_remote
        );
        assert!(pick_session(None, None).is_none());
    }

    #[test]
    fn merge_unions_without_dropping_local_only_records() {
        // Oracle: list shows every record from either store, the
        // newer copy of a shared handle wins, newest first.
        let local_only = rec("fp_local", "2026-11-23T08:00:00Z", "2026-11-23T08:00:00Z", "queued");
        let shared_old = rec("fp_shared", "2026-11-23T07:00:00Z", "2026-11-23T07:30:00Z", "running");
        let shared_new = rec("fp_shared", "2026-11-23T07:00:00Z", "2026-11-23T07:45:00Z", "succeeded");
        let remote_only = rec("fp_remote", "2026-11-23T09:00:00Z", "2026-11-23T09:00:00Z", "running");
        let merged = merge_session_records(
            vec![local_only.clone(), shared_old],
            vec![shared_new.clone(), remote_only.clone()],
        );
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], remote_only);
        assert_eq!(merged[1], local_only);
        assert_eq!(merged[2], shared_new);
    }

    #[test]
    fn sync_sweeps_local_records_and_tolerates_failure() {
        // Oracle: the sweep offers the local store's records to the
        // backend insert-or-ignore; with the backend down it is a
        // silent no-op (the caller proceeds local-only).
        let store = json!({"sessions": {
            "fp_a": rec("fp_a", "2026-11-23T08:00:00Z", "2026-11-23T08:00:00Z", "queued"),
            "fp_b": rec("fp_b", "2026-11-23T08:01:00Z", "2026-11-23T08:01:00Z", "running"),
        }});
        let remote = FakeRemote::new();
        sync_local_sessions(&remote, &store);
        assert_eq!(remote.syncs.borrow().len(), 1);
        assert_eq!(remote.stored.borrow().len(), 2);
        // A second sweep inserts nothing new (idempotent).
        sync_local_sessions(&remote, &store);
        assert_eq!(remote.stored.borrow().len(), 2);

        let down = FakeRemote::new();
        *down.fail.borrow_mut() = true;
        sync_local_sessions(&down, &store); // must not panic or surface
        assert!(down.stored.borrow().is_empty());
    }

    #[test]
    fn fake_remote_round_trips_a_put_through_get() {
        // Harness sanity: the fake the placement wiring tests use
        // behaves like the verb contract (put upserts, get reads).
        let remote = FakeRemote::new();
        let r = rec("fp_a", "2026-11-23T08:00:00Z", "2026-11-23T08:00:00Z", "queued");
        remote.put(&r).unwrap();
        assert_eq!(remote.get("fp_a").unwrap(), Some(r));
        assert_eq!(remote.get("fp_nope").unwrap(), None);
    }
}
