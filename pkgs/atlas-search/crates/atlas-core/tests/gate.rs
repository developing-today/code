//! The ingest gate's record and state machine (#7709): fail-closed stage
//! order, quarantine with bounded strikes, resume at the failing stage,
//! durable records, and no content in a record.

use std::collections::BTreeMap;

use atlas_core::gate::{
    BatchState, GateRecord, GateStage, ReasonCode, STRIKE_LIMIT, SkipReason, StageVerdict, Verdict,
};
use atlas_core::shards::Tier;
use atlas_core::stage::Registry;
use proptest::prelude::*;

fn hash(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn verdict(verdict: Verdict, input: u8) -> StageVerdict {
    StageVerdict {
        verdict,
        input_hash: hash(input),
        rows_in: 4,
        rows_out: 4,
        row_ids: vec![],
    }
}

fn pass(input: u8) -> StageVerdict {
    verdict(Verdict::Pass, input)
}

fn fail(code: ReasonCode, input: u8) -> StageVerdict {
    StageVerdict {
        row_ids: vec!["item:7".into(), "chunk:7/2".into()],
        ..verdict(
            Verdict::Fail {
                reasons: vec![code],
            },
            input,
        )
    }
}

fn gated(batch: &str) -> GateRecord {
    let mut record = GateRecord::new(batch, "unit-1", Tier::Internal).unwrap();
    record.close().unwrap();
    record.begin_gate().unwrap();
    record
}

fn hashes(inputs: &[(GateStage, u8)]) -> BTreeMap<GateStage, String> {
    inputs.iter().map(|&(stage, n)| (stage, hash(n))).collect()
}

#[test]
fn a_clean_batch_publishes_with_its_manifest() {
    let mut record = gated("b1");
    for stage in GateStage::ALL {
        record.record(stage, pass(1)).unwrap();
    }
    assert_eq!(record.next_stage(), None);
    record.publish(Some("manifest-1")).unwrap();
    assert_eq!(record.state, BatchState::Published);
    assert_eq!(record.publish_manifest_id.as_deref(), Some("manifest-1"));
}

#[test]
fn transitions_outside_the_state_machine_are_refused() {
    let mut record = GateRecord::new("b1", "unit-1", Tier::Public).unwrap();
    assert!(record.begin_gate().is_err(), "open cannot skip closing");
    assert!(
        record.record(GateStage::Write, pass(1)).is_err(),
        "no verdicts before gating"
    );
    assert!(record.publish(Some("m")).is_err());
    record.close().unwrap();
    assert!(record.close().is_err());
    record.begin_gate().unwrap();
    assert!(
        record.retry(&BTreeMap::new()).is_err(),
        "only a quarantined batch retries"
    );
    for stage in GateStage::ALL {
        record.record(stage, pass(1)).unwrap();
    }
    record.publish(Some("m")).unwrap();
    assert!(record.publish(Some("m")).is_err(), "published is terminal");
    assert!(record.close().is_err());
}

#[test]
fn a_missing_stage_verdict_is_not_a_pass() {
    let mut record = gated("b1");
    record.record(GateStage::Write, pass(1)).unwrap();
    record.record(GateStage::Classify, pass(1)).unwrap();
    // Dedup has no verdict: it has not passed, and nothing after it may be judged.
    assert!(!record.passed(GateStage::Dedup));
    assert_eq!(record.next_stage(), Some(GateStage::Dedup));
    for later in [GateStage::ProvenanceCheck, GateStage::Publish] {
        assert!(
            record.record(later, pass(1)).is_err(),
            "{later} recorded before dedup"
        );
    }
    assert!(
        record.publish(Some("m")).is_err(),
        "a record with a missing verdict cannot publish"
    );
    assert_eq!(record.state, BatchState::Gated);
}

#[test]
fn skips_are_only_the_two_the_design_allows() {
    let mut empty = gated("empty");
    empty
        .record(
            GateStage::Write,
            verdict(
                Verdict::Skip {
                    reason: SkipReason::EmptyBatch,
                },
                1,
            ),
        )
        .unwrap();
    let mut wrong = gated("wrong");
    wrong.record(GateStage::Write, pass(1)).unwrap();
    for reason in [
        SkipReason::EmptyBatch,
        SkipReason::ZeroSurvivorsNoOccurrences,
    ] {
        assert!(
            wrong
                .record(GateStage::Classify, verdict(Verdict::Skip { reason }, 1))
                .is_err()
        );
    }
    let mut zero = gated("zero");
    for stage in &GateStage::ALL[..4] {
        zero.record(*stage, pass(1)).unwrap();
    }
    zero.record(
        GateStage::Publish,
        verdict(
            Verdict::Skip {
                reason: SkipReason::ZeroSurvivorsNoOccurrences,
            },
            1,
        ),
    )
    .unwrap();
    assert!(
        zero.clone().publish(Some("m")).is_err(),
        "a skipped publish names no manifest"
    );
    zero.publish(None).unwrap();
    assert_eq!(zero.publish_manifest_id, None);
}

#[test]
fn failures_must_name_codes_of_their_own_stage() {
    let mut record = gated("b1");
    assert!(
        record
            .record(
                GateStage::Write,
                verdict(Verdict::Fail { reasons: vec![] }, 1)
            )
            .is_err()
    );
    assert!(
        record
            .record(GateStage::Write, fail(ReasonCode::ClassifyUnresolved, 1))
            .is_err()
    );
    assert_eq!(record.state, BatchState::Gated);
}

#[test]
fn a_failing_batch_quarantines_publishes_nothing_and_stops_after_three_strikes() {
    let mut record = gated("b1");
    let inputs = hashes(&[
        (GateStage::Write, 1),
        (GateStage::Classify, 1),
        (GateStage::Dedup, 1),
    ]);
    record.record(GateStage::Write, pass(1)).unwrap();
    record.record(GateStage::Classify, pass(1)).unwrap();
    for strike in 1..=STRIKE_LIMIT {
        record
            .record(GateStage::Dedup, fail(ReasonCode::DedupUndispositioned, 1))
            .unwrap();
        assert_eq!(record.state, BatchState::Quarantined);
        assert_eq!(record.failing_stage(), Some(GateStage::Dedup));
        assert_eq!(record.publish_manifest_id, None);
        assert!(record.publish(Some("m")).is_err());
        assert_eq!(record.strikes[&ReasonCode::DedupUndispositioned], strike);
        if strike < STRIKE_LIMIT {
            assert!(!record.needs_review);
            // Unchanged inputs: resume at the failing stage, keeping earlier verdicts.
            assert_eq!(record.retry(&inputs).unwrap(), GateStage::Dedup);
            assert!(record.passed(GateStage::Classify));
        }
    }
    assert!(record.needs_review);
    assert!(
        record.retry(&inputs).is_err(),
        "no automatic retry past three strikes"
    );
    assert_eq!(record.state, BatchState::Quarantined);
}

#[test]
fn strikes_count_per_reason_code() {
    let mut record = gated("b1");
    let inputs = hashes(&[(GateStage::Write, 1), (GateStage::Classify, 1)]);
    record.record(GateStage::Write, pass(1)).unwrap();
    for code in [
        ReasonCode::ClassifyUnresolved,
        ReasonCode::ClassifyUnresolved,
    ] {
        record.record(GateStage::Classify, fail(code, 1)).unwrap();
        record.retry(&inputs).unwrap();
    }
    record.record(GateStage::Classify, pass(1)).unwrap();
    record
        .record(GateStage::Dedup, fail(ReasonCode::DedupUndispositioned, 1))
        .unwrap();
    assert!(
        !record.needs_review,
        "two strikes on one code and one on another is not three"
    );
}

#[test]
fn write_failures_restart_at_write() {
    for code in [ReasonCode::WriteInvalid, ReasonCode::WriteBypass] {
        let mut record = gated("b1");
        record.record(GateStage::Write, fail(code, 1)).unwrap();
        assert_eq!(
            record.retry(&hashes(&[(GateStage::Write, 1)])).unwrap(),
            GateStage::Write
        );
        assert!(record.verdicts.is_empty());
    }
}

#[test]
fn changed_inputs_reenter_at_the_earliest_changed_stage() {
    let mut record = gated("b1");
    for stage in [GateStage::Write, GateStage::Classify, GateStage::Dedup] {
        record.record(stage, pass(1)).unwrap();
    }
    record
        .record(
            GateStage::ProvenanceCheck,
            fail(ReasonCode::ProvenanceUnresolvable, 1),
        )
        .unwrap();
    // Classify's input changed: its verdict and every later one are discarded.
    let changed = hashes(&[
        (GateStage::Write, 1),
        (GateStage::Classify, 2),
        (GateStage::Dedup, 2),
        (GateStage::ProvenanceCheck, 2),
    ]);
    assert_eq!(record.retry(&changed).unwrap(), GateStage::Classify);
    assert_eq!(
        record.verdicts.keys().copied().collect::<Vec<_>>(),
        vec![GateStage::Write]
    );
    assert_eq!(record.next_stage(), Some(GateStage::Classify));
    // A retry must state every upstream stage's current input identity.
    let mut record2 = gated("b2");
    record2.record(GateStage::Write, pass(1)).unwrap();
    record2
        .record(GateStage::Classify, fail(ReasonCode::ClassifyUnresolved, 1))
        .unwrap();
    assert!(record2.retry(&hashes(&[(GateStage::Classify, 1)])).is_err());
    assert_eq!(record2.state, BatchState::Quarantined);
}

#[test]
fn records_round_trip_through_the_registry() {
    let registry = Registry::in_memory().unwrap();
    assert_eq!(
        registry.load_gate_record("absent").unwrap(),
        None,
        "no record is no pass"
    );
    let mut record = gated("b1");
    record.record(GateStage::Write, pass(1)).unwrap();
    record
        .record(GateStage::Classify, fail(ReasonCode::ClassifyUnresolved, 3))
        .unwrap();
    registry
        .save_gate_record(&record, "2026-11-19T00:00:00Z")
        .unwrap();
    assert_eq!(
        registry.load_gate_record("b1").unwrap().as_ref(),
        Some(&record)
    );
    record
        .retry(&hashes(&[(GateStage::Write, 1), (GateStage::Classify, 3)]))
        .unwrap();
    for stage in &GateStage::ALL[1..] {
        record.record(*stage, pass(3)).unwrap();
    }
    record.publish(Some("manifest-9")).unwrap();
    registry
        .save_gate_record(&record, "2026-11-19T00:01:00Z")
        .unwrap();
    let loaded = registry.load_gate_record("b1").unwrap().unwrap();
    assert_eq!(loaded, record);
    assert_eq!(
        loaded.strikes[&ReasonCode::ClassifyUnresolved],
        1,
        "strikes survive a later publish"
    );
}

#[test]
fn a_secret_tier_record_holds_no_content() {
    let secret = "patient record: blood type O-negative, SSN 078-05-1120";
    let mut record = GateRecord::new("b-secret", "unit-secret", Tier::Secret).unwrap();
    record.close().unwrap();
    record.begin_gate().unwrap();
    record.record(GateStage::Write, pass(1)).unwrap();
    // Content can't ride in as a row id or a hash.
    for bad in [secret, "line one\nline two", ""] {
        let mut v = fail(ReasonCode::ClassifyUnresolved, 2);
        v.row_ids = vec![bad.to_owned()];
        assert!(record.record(GateStage::Classify, v).is_err());
    }
    let mut v = fail(ReasonCode::ClassifyUnresolved, 2);
    v.input_hash = secret.to_owned();
    assert!(record.record(GateStage::Classify, v).is_err());
    assert!(GateRecord::new(secret, "unit", Tier::Secret).is_err());
    record
        .record(GateStage::Classify, fail(ReasonCode::ClassifyUnresolved, 2))
        .unwrap();
    let registry = Registry::in_memory().unwrap();
    registry
        .save_gate_record(&record, "2026-11-19T00:00:00Z")
        .unwrap();
    // Every stored text field is an id, an enumerated code, a hash, a count or a timestamp.
    let loaded = registry.load_gate_record("b-secret").unwrap().unwrap();
    let dump = format!("{loaded:?}");
    assert!(
        ["patient", "blood", "SSN", "078-05"]
            .iter()
            .all(|w| !dump.contains(w))
    );
}

/// Independent model: the batch publishes iff every stage was recorded in
/// order with a pass or an allowed skip; any failure quarantines.
#[derive(Debug, Clone)]
enum Step {
    Pass,
    SkipAs(SkipReason),
    Fail(ReasonCode),
    Publish,
    Retry,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => Just(Step::Pass),
        1 => prop_oneof![Just(SkipReason::EmptyBatch), Just(SkipReason::ZeroSurvivorsNoOccurrences)].prop_map(Step::SkipAs),
        1 => prop_oneof![
            Just(ReasonCode::WriteInvalid),
            Just(ReasonCode::ClassifyUnresolved),
            Just(ReasonCode::DedupUndispositioned),
            Just(ReasonCode::ProvenanceUnresolvable),
            Just(ReasonCode::PublishPartial),
        ].prop_map(Step::Fail),
        1 => Just(Step::Publish),
        1 => Just(Step::Retry),
    ]
}

proptest! {
    #[test]
    fn published_implies_every_stage_passed(steps in prop::collection::vec(step(), 1..40)) {
        let mut record = gated("p");
        let all = hashes(&GateStage::ALL.map(|s| (s, 1)));
        for s in steps {
            let before = record.clone();
            let next = record.next_stage();
            let result = match s {
                Step::Pass => next.map_or(Err(()), |st| record.record(st, pass(1)).map_err(|_| ())),
                Step::SkipAs(reason) => next.map_or(Err(()), |st| record.record(st, verdict(Verdict::Skip { reason }, 1)).map_err(|_| ())),
                Step::Fail(code) => next.map_or(Err(()), |st| record.record(st, fail(code, 1)).map_err(|_| ())),
                Step::Publish => {
                    let skipped = matches!(record.verdicts.get(&GateStage::Publish).map(|v| &v.verdict), Some(Verdict::Skip { .. }));
                    record.publish(if skipped { None } else { Some("m") }).map_err(|_| ())
                }
                Step::Retry => record.retry(&all).map(|_| ()).map_err(|_| ()),
            };
            if result.is_err() {
                prop_assert_eq!(&record, &before, "a refused step leaves the record unchanged");
            }
            if record.state == BatchState::Published {
                prop_assert!(GateStage::ALL.iter().all(|&st| record.passed(st)));
                break;
            }
            if record.state == BatchState::Quarantined {
                prop_assert!(record.failing_stage().is_some());
                prop_assert!(record.publish_manifest_id.is_none());
            }
            prop_assert!(record.strikes.values().all(|&n| n <= STRIKE_LIMIT));
            prop_assert_eq!(record.needs_review, record.strikes.values().any(|&n| n >= STRIKE_LIMIT));
            // Verdicts are always a contiguous prefix of the stage order.
            let keys: Vec<_> = record.verdicts.keys().copied().collect();
            prop_assert_eq!(&keys[..], &GateStage::ALL[..keys.len()]);
        }
    }
}
