//! Tests for the owner report projection (#11774). The oracle is the
//! fixture journal each test writes: counts come from the entries it
//! chose, not from a second implementation of the query.

use std::io::Cursor;

use proptest::prelude::*;

use super::*;
#[cfg(feature = "sandbox")]
use crate::sandbox::{CallMetrics, GuestCall};
use crate::world::MetricsSnapshot;
use crate::world::{JournalEntry, Participant};
use crate::world_storage::ByteCount;

const WORLD: &str = "w-owner";

fn request() -> ReportRequest {
    ReportRequest {
        world_id: WORLD.to_owned(),
        generated_at_unix_ms: 1_800_000_000_000,
        window: ReportWindow {
            from_unix_ms: 1_799_000_000_000,
            to_unix_ms: 1_800_000_000_000,
        },
        limits: ReportLimits::default(),
    }
}

fn header(version: u32) -> JournalEntry {
    JournalEntry::Created {
        world_id: WORLD.to_owned(),
        version,
    }
}

fn issued(id: u64, parent: Option<u64>, subject: Option<&str>) -> JournalEntry {
    JournalEntry::Issued {
        participant: Participant {
            id,
            display_name: format!("name-{id}"),
        },
        digest: format!("{id:064x}"),
        scopes: 1,
        parent,
        expires_at: None,
        uses: None,
        used: 0,
        subject: subject.map(str::to_owned),
    }
}

fn revoked(id: u64) -> JournalEntry {
    JournalEntry::Revoked {
        participant_id: id,
        by: None,
    }
}

fn jsonl(entries: &[JournalEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        out.extend(serde_json::to_vec(entry).unwrap());
        out.push(b'\n');
    }
    out
}

fn run(bytes: &[u8]) -> Result<OwnerReport, OwnerReportError> {
    project(Cursor::new(bytes.to_vec()), &request(), None)
}

#[test]
fn fixture_journal_gives_exact_join_counts() {
    let bytes = jsonl(&[
        header(1),
        issued(1, None, Some("acct-a")),
        issued(2, Some(1), None),
        revoked(2),
        issued(3, None, Some("acct-a")),
        issued(4, None, Some("acct-b")),
    ]);
    let report = run(&bytes).unwrap();
    let joins = &report.joins;
    assert_eq!(joins.count, 4);
    assert_eq!(joins.host_issued, 3);
    assert_eq!(joins.delegated, 1);
    assert_eq!(joins.revoked, 1);
    // A rejoin under one account is two participants and one subject.
    assert_eq!(joins.distinct_subjects, 2);
    assert!(joins.complete);
    assert_eq!(joins.inherited_from_fork, None);
    assert_eq!(
        joins
            .recent
            .iter()
            .map(|s| s.participant_id)
            .collect::<Vec<_>>(),
        vec![4, 3, 2, 1]
    );
    assert!(joins.recent[2].delegated && joins.recent[2].revoked);
    assert_eq!(report.journal.header_version, Some(1));
    assert_eq!(report.journal.entries, 6);
    assert!(!report.journal.torn_tail);
}

#[test]
fn join_times_are_unavailable_never_zero() {
    let report = run(&jsonl(&[header(1), issued(1, None, None)])).unwrap();
    assert_eq!(
        report.joins.in_window.reason(),
        Some(UnavailableReason::JoinTimeNotRecorded)
    );
    assert_eq!(report.joins.in_window.value(), None);
    assert_eq!(
        report.joins.available_since.reason(),
        Some(UnavailableReason::JoinTimeNotRecorded)
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(
        json["joins"]["in_window"],
        serde_json::json!({"value": null, "reason": "join_time_not_recorded"})
    );
    assert_eq!(
        json["members"]["active_count"]["reason"],
        "live_state_not_provided"
    );
    assert_eq!(
        json["program_runs"]["count"]["reason"],
        "program_runs_not_recorded"
    );
    assert_eq!(json["storage"]["reason"], "storage_not_measured");
    // A measured zero is a value, with no reason.
    assert_eq!(json["joins"]["revoked"], 0);
}

#[test]
fn empty_journal_is_an_empty_complete_world() {
    let report = run(b"").unwrap();
    assert_eq!(report.joins.count, 0);
    assert!(report.joins.complete);
    assert_eq!(report.journal.header_version, None);
    assert_eq!(report.journal.entries, 0);
}

#[test]
fn repeated_projection_is_identical() {
    let bytes = jsonl(&[
        header(1),
        issued(7, None, Some("x")),
        issued(9, Some(7), None),
        revoked(7),
    ]);
    let a = serde_json::to_string(&run(&bytes).unwrap()).unwrap();
    let b = serde_json::to_string(&run(&bytes).unwrap()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn blank_lines_are_skipped_like_the_journal_reader() {
    let mut bytes = jsonl(&[header(1)]);
    bytes.extend(b"\n");
    bytes.extend(jsonl(&[issued(1, None, None)]));
    let report = run(&bytes).unwrap();
    assert_eq!(report.joins.count, 1);
    assert_eq!(report.journal.entries, 2);
}

#[test]
fn a_repeated_issue_counts_once() {
    let report = run(&jsonl(&[
        header(1),
        issued(5, None, None),
        issued(5, None, None),
    ]))
    .unwrap();
    assert_eq!(report.joins.count, 1);
    assert_eq!(report.joins.recent.len(), 1);
}

#[test]
fn fork_record_names_inherited_participants() {
    let bytes = jsonl(&[
        header(1),
        issued(1, None, None),
        issued(2, None, None),
        JournalEntry::Forked {
            from_world_id: "parent".to_owned(),
            from_module_hash: String::new(),
            template: false,
            at_sequence: 0,
        },
        issued(3, None, None),
    ]);
    let report = run(&bytes).unwrap();
    assert_eq!(report.journal.forked_from.as_deref(), Some("parent"));
    assert_eq!(report.joins.inherited_from_fork, Some(2));
    assert_eq!(report.joins.count, 3);
}

#[test]
fn torn_tail_is_not_counted_and_marks_incomplete() {
    let mut bytes = jsonl(&[header(1), issued(1, None, None)]);
    bytes.extend(serde_json::to_vec(&issued(2, None, None)).unwrap());
    let report = run(&bytes).unwrap();
    assert!(report.journal.torn_tail);
    assert!(!report.joins.complete);
    assert_eq!(report.joins.count, 1);
}

#[test]
fn participant_limit_stops_counting_and_says_so() {
    let mut req = request();
    req.limits.max_participants = 2;
    let bytes = jsonl(&[
        header(1),
        issued(1, None, None),
        issued(2, None, None),
        issued(3, None, None),
        revoked(3),
    ]);
    let report = project(Cursor::new(bytes), &req, None).unwrap();
    assert_eq!(report.joins.count, 2);
    assert_eq!(report.joins.revoked, 0);
    assert!(!report.joins.complete);
}

#[test]
fn sample_keeps_only_the_newest() {
    let mut req = request();
    req.limits.sample_limit = 2;
    let bytes = jsonl(&[
        header(1),
        issued(1, None, None),
        issued(2, None, None),
        issued(3, None, None),
    ]);
    let report = project(Cursor::new(bytes.clone()), &req, None).unwrap();
    let ids: Vec<u64> = report
        .joins
        .recent
        .iter()
        .map(|s| s.participant_id)
        .collect();
    assert_eq!(ids, vec![3, 2]);
    req.limits.sample_limit = 0;
    let report = project(Cursor::new(bytes), &req, None).unwrap();
    assert!(report.joins.recent.is_empty());
    assert_eq!(report.joins.count, 3);
}

#[test]
fn sample_never_carries_names_or_capability_material() {
    let report = run(&jsonl(&[header(1), issued(1, None, Some("acct-secret"))])).unwrap();
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("name-1"));
    assert!(!json.contains(&format!("{:064x}", 1)));
    assert!(!json.contains("acct-secret"));
}

#[test]
fn supplied_storage_is_reported_as_measured() {
    let inventory = StorageInventory {
        journal: ByteCount {
            bytes: 10,
            files: 1,
        },
        modules: ByteCount::default(),
        other: ByteCount::default(),
        total_bytes: 10,
        exact: false,
        symlinks_skipped: 0,
        nested_worlds_skipped: 0,
        vanished: 0,
        special_files: 0,
        truncated: false,
        as_of_unix_ms: 5,
    };
    let report = project(
        Cursor::new(jsonl(&[header(1)])),
        &request(),
        Some(inventory.clone()),
    )
    .unwrap();
    assert_eq!(report.storage, Measured::Known(inventory));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["storage"]["value"]["total_bytes"], 10);
    assert!(json["storage"].get("reason").is_none());
}

#[test]
fn refusals_are_distinct() {
    assert!(matches!(
        run(&jsonl(&[issued(1, None, None)])),
        Err(OwnerReportError::MissingHeader)
    ));
    let other = JournalEntry::Created {
        world_id: "elsewhere".to_owned(),
        version: 1,
    };
    assert!(matches!(
        run(&jsonl(&[other])),
        Err(OwnerReportError::WrongWorld { found }) if found == "elsewhere"
    ));
    for version in [0, MAX_JOURNAL_VERSION + 1] {
        assert!(matches!(
            run(&jsonl(&[header(version)])),
            Err(OwnerReportError::UnsupportedJournalVersion { version: v }) if v == version
        ));
    }
    let mut bytes = jsonl(&[header(1)]);
    bytes.extend(b"{\"entry\":\"no_such_entry\"}\n");
    assert!(matches!(
        run(&bytes),
        Err(OwnerReportError::Malformed { line: 2 })
    ));
    assert!(matches!(
        run(&jsonl(&[header(1), header(1)])),
        Err(OwnerReportError::Malformed { line: 2 })
    ));
    let mut bytes = jsonl(&[header(1)]);
    bytes.extend(b"{\"entry\":\"execution_profile\",\"profile\":{}}\n");
    assert!(matches!(
        run(&bytes),
        Err(OwnerReportError::EntryNewerThanHeader { line: 2, .. })
    ));
    let mut req = request();
    req.window.to_unix_ms = req.window.from_unix_ms - 1;
    assert!(matches!(
        project(Cursor::new(Vec::new()), &req, None),
        Err(OwnerReportError::InvalidWindow)
    ));
}

#[test]
fn oversized_line_is_refused_not_buffered() {
    let mut req = request();
    let bytes = jsonl(&[header(1), issued(1, None, None)]);
    let second = serde_json::to_vec(&issued(1, None, None)).unwrap().len() + 1;
    req.limits.max_line_bytes = second - 1;
    assert!(matches!(
        project(Cursor::new(bytes.clone()), &req, None),
        Err(OwnerReportError::OversizedLine { line: 2 })
    ));
    req.limits.max_line_bytes = second;
    assert!(project(Cursor::new(bytes), &req, None).is_ok());
}

#[test]
fn schema_check_accepts_added_fields_and_refuses_other_majors() {
    let report = run(&jsonl(&[header(1)])).unwrap();
    let mut json = serde_json::to_value(&report).unwrap();
    json["added_later"] = serde_json::json!(true);
    assert_eq!(check_schema_version(&json), Ok(1));
    json["schema_version"] = serde_json::json!(2);
    assert_eq!(check_schema_version(&json), Err(Some(2)));
    let missing = serde_json::json!({"world_id": WORLD});
    assert_eq!(check_schema_version(&missing), Err(None));
}

fn live(calls: Option<ProgramCallCounts>) -> LiveSnapshot {
    LiveSnapshot {
        as_of_unix_ms: 1_800_000_000_500,
        connected_participants: 2,
        sessions: 3,
        program_calls: calls,
    }
}

#[test]
fn live_snapshot_fills_members_and_keeps_durable_sections() {
    let bytes = jsonl(&[header(1), issued(1, None, None), issued(2, None, None)]);
    let durable = run(&bytes).unwrap();
    let report = durable.clone().with_live(Some(&live(None)));
    assert_eq!(report.members.active_count, Measured::Known(2));
    assert_eq!(report.members.sessions, Measured::Known(3));
    assert_eq!(report.members.as_of_unix_ms, Some(1_800_000_000_500));
    // No program counter in the snapshot: still unavailable, not zero.
    assert_eq!(
        report.program_runs.count.reason(),
        Some(UnavailableReason::ProgramRunsNotRecorded)
    );
    assert_eq!(report.program_runs.scope, ProgramRunScope::None);
    assert_eq!(report.joins, durable.joins);
    assert_eq!(report.journal, durable.journal);
    // Without live state the live sections are unavailable again.
    let offline = report.with_live(None);
    assert_eq!(offline, durable);
}

#[test]
fn live_snapshot_reads_the_metrics_counts() {
    let metrics = MetricsSnapshot {
        sessions: 5,
        participants: 4,
        ..MetricsSnapshot::default()
    };
    let snapshot = LiveSnapshot::from_metrics(&metrics, 9);
    assert_eq!(snapshot.connected_participants, 4);
    assert_eq!(snapshot.sessions, 5);
    assert_eq!(snapshot.as_of_unix_ms, 9);
    assert!(snapshot.program_calls.is_none());
    // A snapshot from a binary before #11775 has no participants field.
    let mut json = serde_json::to_value(&metrics).unwrap();
    json.as_object_mut().unwrap().remove("participants");
    let old: MetricsSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(old.participants, 0);
}

#[cfg(feature = "sandbox")]
#[test]
fn program_calls_are_process_local_and_never_complete() {
    let calls = ProgramCallCounts::from_call_metrics(&CallMetrics::default(), Some(7));
    assert_eq!(calls.by_call.get("update"), Some(&0));
    assert_eq!(calls.by_call.len(), GuestCall::ALL.len());
    let mut calls = calls;
    calls.by_call.insert("update", 12);
    let report = run(&jsonl(&[header(1)]))
        .unwrap()
        .with_live(Some(&live(Some(calls))));
    let runs = &report.program_runs;
    assert_eq!(runs.count, Measured::Known(12));
    assert_eq!(runs.scope, ProgramRunScope::ProgramInstanceInProcess);
    assert_eq!(runs.since_unix_ms, Some(7));
    assert!(!runs.complete);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["program_runs"]["scope"], "program_instance_in_process");
    assert_eq!(json["program_runs"]["by_call"]["update"], 12);
    assert_eq!(json["program_runs"]["complete"], false);
}

proptest! {
    /// Counts match a model of the same issue/revoke sequence, and the
    /// serialized report is the same however often it is projected.
    #[test]
    fn counts_match_the_model(ops in prop::collection::vec((0u64..12, any::<bool>(), any::<bool>()), 0..60)) {
        let mut entries = vec![header(1)];
        let mut model: BTreeMap<u64, (bool, bool)> = BTreeMap::new();
        for (id, is_revoke, delegated) in ops {
            if is_revoke {
                entries.push(revoked(id));
                if let Some(flags) = model.get_mut(&id) {
                    flags.1 = true;
                }
            } else {
                entries.push(issued(id, delegated.then_some(100), None));
                model.entry(id).or_insert((delegated, false));
            }
        }
        let bytes = jsonl(&entries);
        let report = run(&bytes).unwrap();
        prop_assert_eq!(report.joins.count, model.len() as u64);
        prop_assert_eq!(report.joins.delegated, model.values().filter(|f| f.0).count() as u64);
        prop_assert_eq!(report.joins.revoked, model.values().filter(|f| f.1).count() as u64);
        prop_assert_eq!(report.joins.host_issued + report.joins.delegated, report.joins.count);
        prop_assert!(report.joins.recent.len() <= ReportLimits::default().sample_limit);
        let again = run(&bytes).unwrap();
        prop_assert_eq!(serde_json::to_string(&report).unwrap(), serde_json::to_string(&again).unwrap());
    }
}
