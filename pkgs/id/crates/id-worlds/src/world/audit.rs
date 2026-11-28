//! The retained audit log for one world (F12.6, issue #1063).
//!
//! The world actor and the session layer build one
//! [`AuditRecord`] for every administrative
//! action and provider-use decision and emit it on the `id::audit`
//! tracing target. Emission alone is not a log: the records were
//! fire-and-forget, so nothing could export them. This module is the
//! retention + export half of F12.6 — every record the actor emits is
//! also appended to a [`WorldAuditLog`] shared between the actor, the
//! handle, and the session layer (the same sharing pattern as the
//! F11.7 [`super::WorldMetrics`] registry) — plus the two consumers
//! the F12.6 exit criteria name:
//!
//! - [`export_otlp_json`]: the retained records as an OpenTelemetry
//!   OTLP/JSON logs payload, so audit and provider-use events leave
//!   through the standard telemetry shape.
//! - [`replay_grants`]: fold the log's accepted `UpdateCaps` /
//!   `SetCapsCeiling` records plus `provider_use` first-use grants back
//!   into the final grants state, the same fold the tracing-capture
//!   replay tests prove, here over the retained log itself.
//!
//! The log is bounded ([`MAX_AUDIT_RECORDS`]); when it fills, the
//! oldest records drop. It is an operational log, not the journal:
//! durability lives in the world journal, whose grants entries the
//! records' parameters mirror. Session-layer records (hub and
//! first-frame admin actions) are emitted on the same tracing target
//! but are not retained here — retention covers the actor's stream,
//! which is the grants-bearing one.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use super::AuditRecord;

/// Maximum audit records retained per world.
pub const MAX_AUDIT_RECORDS: usize = 4096;

/// A world's retained audit records, shared between the actor (which
/// appends), the handle, and the session layer. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct WorldAuditLog {
    inner: Arc<Mutex<VecDeque<AuditRecord>>>,
}

impl WorldAuditLog {
    /// A new, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one record, dropping the oldest records past
    /// [`MAX_AUDIT_RECORDS`].
    pub fn record(&self, record: AuditRecord) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.len() >= MAX_AUDIT_RECORDS {
            inner.pop_front();
        }
        inner.push_back(record);
    }

    /// A snapshot of the retained records, oldest first.
    #[must_use]
    pub fn records(&self) -> Vec<AuditRecord> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.iter().cloned().collect()
    }

    /// How many records are currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Whether no records are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The retained records as JSON lines — one audit line per
    /// record, in order. This is the audit log's on-disk shape.
    #[must_use]
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for record in self.records() {
            if let Ok(line) = serde_json::to_string(&record) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }
}

/// Grants state reconstructed by [`replay_grants`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrantsState {
    /// The world's capability grants after the folded records.
    pub granted: BTreeSet<String>,
    /// The capability ceiling after the folded records, when any
    /// accepted `SetCapsCeiling` record set one.
    pub ceiling: Option<BTreeSet<String>>,
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Replay audit records to the final grants state. Only `accepted`
/// admin records move state — a refused or denied admin action changed
/// nothing, so it replays as nothing. `UpdateCaps` folds exactly as
/// the actor applies it: revokes are removed first, then grants are
/// added. `SetCapsCeiling` replaces the ceiling. A `provider_use`
/// record with outcome `first_use_grant` folds the used capability into
/// `granted`, mirroring `CapLedger::decide_for` under `grant_on_use`.
/// An `allowed` use folds nothing: `decide_for` returns `Allow` only for
/// a name already granted or covered by a `*` grant, and in neither case
/// does it add the name, so folding it would keep a wildcard-covered
/// name granted after `*` is revoked (#10828). Every other action
/// leaves grants untouched.
#[must_use]
pub fn replay_grants(records: &[AuditRecord]) -> GrantsState {
    let mut state = GrantsState::default();
    for record in records {
        if record.action == "provider_use" {
            if record.outcome == "first_use_grant" {
                if let Some(capability) = record
                    .parameters
                    .get("capability")
                    .and_then(serde_json::Value::as_str)
                {
                    state.granted.insert(capability.to_owned());
                }
            }
            continue;
        }
        if record.outcome != "accepted" {
            continue;
        }
        match record.action.as_str() {
            "UpdateCaps" => {
                for name in string_list(record.parameters.get("revoke")) {
                    state.granted.remove(&name);
                }
                for name in string_list(record.parameters.get("grant")) {
                    state.granted.insert(name);
                }
            }
            "SetCapsCeiling" => {
                state.ceiling = Some(
                    string_list(record.parameters.get("ceiling"))
                        .into_iter()
                        .collect(),
                );
            }
            _ => {}
        }
    }
    state
}

/// Whether an audit outcome is a success outcome for severity
/// mapping: accepted admin actions and allowed provider use are
/// informational; refusals, denials, and errors are warnings.
fn is_success_outcome(outcome: &str) -> bool {
    matches!(outcome, "accepted" | "allowed" | "first_use_grant")
}

/// Export audit records as an OpenTelemetry OTLP/JSON logs payload
/// (`LogsData`): one log record per audit record, the record's own
/// JSON line as the body, and the record's fields as attributes.
/// This is the export half of F12.6 — the same records the actor
/// emits on the tracing telemetry path, in the wire shape an OTLP
/// collector accepts.
#[must_use]
pub fn export_otlp_json(records: &[AuditRecord]) -> serde_json::Value {
    let log_records: Vec<serde_json::Value> = records
        .iter()
        .map(|record| {
            let mut attributes = vec![
                serde_json::json!({"key": "audit.actor", "value": {"stringValue": record.actor}}),
                serde_json::json!({"key": "audit.action", "value": {"stringValue": record.action}}),
                serde_json::json!({"key": "audit.outcome", "value": {"stringValue": record.outcome}}),
                serde_json::json!({"key": "audit.world_id", "value": {"stringValue": record.world_id}}),
                serde_json::json!({"key": "audit.target", "value": {"stringValue": record.target}}),
            ];
            if !record.correlation_id.is_empty() {
                attributes.push(serde_json::json!({
                    "key": "audit.correlation_id",
                    "value": {"stringValue": record.correlation_id},
                }));
            }
            let (severity_number, severity_text) = if is_success_outcome(&record.outcome) {
                (9, "INFO")
            } else {
                (13, "WARN")
            };
            serde_json::json!({
                "timeUnixNano": (u128::from(record.timestamp_ms) * 1_000_000).to_string(),
                "severityNumber": severity_number,
                "severityText": severity_text,
                "body": {"stringValue": serde_json::to_string(record).unwrap_or_default()},
                "attributes": attributes,
            })
        })
        .collect();
    let world_id = records
        .first()
        .map(|record| record.world_id.as_str())
        .unwrap_or_default();
    serde_json::json!({
        "resourceLogs": [{
            "resource": {
                "attributes": [
                    {"key": "service.name", "value": {"stringValue": "id"}},
                    {"key": "world.id", "value": {"stringValue": world_id}},
                ],
            },
            "scopeLogs": [{
                "scope": {"name": "id.audit", "version": "1"},
                "logRecords": log_records,
            }],
        }],
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn record(action: &str, outcome: &str, parameters: serde_json::Value) -> AuditRecord {
        AuditRecord {
            schema_version: 2,
            timestamp_ms: 1_700_000_000_000,
            world_id: "world-a".to_owned(),
            actor: "admin".to_owned(),
            action: action.to_owned(),
            parameters,
            outcome: outcome.to_owned(),
            target: "world-a".to_owned(),
            correlation_id: "audit:1".to_owned(),
        }
    }

    #[test]
    fn replay_folds_grants_in_actor_order() {
        // Oracle: the actor applies UpdateCaps as revoke-then-grant
        // (world/actor.rs UpdateCaps arm); replay must land on the
        // same set the actor's ledger holds.
        let records = vec![
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": ["kv", "blob"], "revoke": []}),
            ),
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": ["notify"], "revoke": ["kv"]}),
            ),
        ];
        let state = replay_grants(&records);
        assert_eq!(
            state.granted,
            BTreeSet::from(["blob".to_owned(), "notify".to_owned()])
        );
        assert_eq!(state.ceiling, None);
    }

    #[test]
    fn replay_ignores_refused_and_denied_records() {
        // Oracle: a refused/denied action changed no state in the
        // actor, so it must change no state in replay.
        let records = vec![
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": ["kv"], "revoke": []}),
            ),
            record(
                "UpdateCaps",
                "refused",
                serde_json::json!({"grant": ["blob"], "revoke": ["kv"]}),
            ),
            record(
                "UpdateCaps",
                "denied",
                serde_json::json!({"grant": ["notify"], "revoke": []}),
            ),
        ];
        let state = replay_grants(&records);
        assert_eq!(state.granted, BTreeSet::from(["kv".to_owned()]));
    }

    #[test]
    fn replay_tracks_the_ceiling() {
        let records = vec![
            record(
                "SetCapsCeiling",
                "accepted",
                serde_json::json!({"ceiling": ["kv", "blob"]}),
            ),
            record(
                "SetCapsCeiling",
                "refused",
                serde_json::json!({"ceiling": []}),
            ),
        ];
        let state = replay_grants(&records);
        assert_eq!(
            state.ceiling,
            Some(BTreeSet::from(["blob".to_owned(), "kv".to_owned()]))
        );
    }

    #[test]
    fn replay_folds_first_use_grants() {
        // Oracle: CapLedger::decide_for under grant_on_use inserts the
        // cap and returns AllowAndGrant, audited as provider_use /
        // first_use_grant (actor.rs audit_provider_decision). Replay
        // must land on the same granted set the ledger holds.
        let records = vec![
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": ["chat.say"], "revoke": []}),
            ),
            record(
                "provider_use",
                "first_use_grant",
                serde_json::json!({"capability": "time.now"}),
            ),
            record(
                "provider_use",
                "allowed",
                serde_json::json!({"capability": "time.now"}),
            ),
            record(
                "provider_use",
                "denied",
                serde_json::json!({"capability": "camera.snapshot"}),
            ),
        ];
        let state = replay_grants(&records);
        assert_eq!(
            state.granted,
            BTreeSet::from(["chat.say".to_owned(), "time.now".to_owned()])
        );
    }

    #[test]
    fn replay_does_not_grant_wildcard_covered_uses() {
        // Oracle: CapLedger::decide_for returns Allow for a non-sensor
        // cap covered by `*` without inserting the name, so revoking
        // `*` leaves nothing granted. Replay must agree (#10828).
        let records = vec![
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": ["*"], "revoke": []}),
            ),
            record(
                "provider_use",
                "allowed",
                serde_json::json!({"capability": "time.now"}),
            ),
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": [], "revoke": ["*"]}),
            ),
        ];
        let state = replay_grants(&records);
        assert!(state.granted.is_empty(), "{:?}", state.granted);
    }

    #[test]
    fn replay_ignores_malformed_provider_use_capability() {
        let records = vec![
            record(
                "provider_use",
                "first_use_grant",
                serde_json::json!({"capability": 123}),
            ),
            record("provider_use", "first_use_grant", serde_json::json!({})),
        ];
        let state = replay_grants(&records);
        assert!(state.granted.is_empty());
    }

    #[test]
    fn retained_log_is_bounded_and_ordered() {
        let log = WorldAuditLog::new();
        for index in 0..(MAX_AUDIT_RECORDS + 3) {
            let mut entry = record("UpdateCaps", "accepted", serde_json::json!({}));
            entry.timestamp_ms = index as u64;
            log.record(entry);
        }
        assert_eq!(log.len(), MAX_AUDIT_RECORDS);
        let records = log.records();
        // The three oldest were dropped; order is append order.
        assert_eq!(records.first().map(|r| r.timestamp_ms), Some(3));
        assert_eq!(
            records.last().map(|r| r.timestamp_ms),
            Some((MAX_AUDIT_RECORDS + 2) as u64)
        );
    }

    #[test]
    fn jsonl_round_trips_through_replay() {
        // Oracle: the JSONL form is the log; parsing it back and
        // replaying must give the same grants as replaying the
        // retained records directly.
        let log = WorldAuditLog::new();
        log.record(record(
            "UpdateCaps",
            "accepted",
            serde_json::json!({"grant": ["kv"], "revoke": []}),
        ));
        let parsed: Vec<AuditRecord> = log
            .to_jsonl()
            .lines()
            .map(|line| serde_json::from_str(line).expect("audit line parses"))
            .collect();
        assert_eq!(parsed, log.records());
        assert_eq!(replay_grants(&parsed), replay_grants(&log.records()));
    }

    #[test]
    fn otlp_export_carries_every_record() {
        // Oracle: OTLP/JSON LogsData — one log record per audit
        // record, fields as attributes, severity by outcome.
        let records = vec![
            record(
                "UpdateCaps",
                "accepted",
                serde_json::json!({"grant": ["kv"], "revoke": []}),
            ),
            record(
                "provider_use",
                "denied",
                serde_json::json!({"capability": "kv"}),
            ),
        ];
        let payload = export_otlp_json(&records);
        let log_records = &payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"];
        let log_records = log_records.as_array().expect("log records array");
        assert_eq!(log_records.len(), 2);
        assert_eq!(log_records[0]["severityText"], "INFO");
        assert_eq!(log_records[1]["severityText"], "WARN");
        assert_eq!(
            payload["resourceLogs"][0]["scopeLogs"][0]["scope"]["name"],
            "id.audit"
        );
        let attributes = log_records[0]["attributes"].as_array().expect("attributes");
        assert!(
            attributes.iter().any(|attr| attr["key"] == "audit.action"
                && attr["value"]["stringValue"] == "UpdateCaps")
        );
    }
}
