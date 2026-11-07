//! Golden tests for the did-it model (design §4 and §8): every worked example
//! round-trips through canonical CBOR, and the pure evaluator reproduces the
//! documented verdicts — including a DST day, a wrapping active span, and a
//! supersede whose order does not matter.

use cbor_core::Value;
use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use p2panda_core::Hash;

use super::*;

const CHICAGO: &str = "America/Chicago";

fn id(byte: u8) -> Hash {
    Hash::from_bytes([byte; 32])
}

fn dog_a() -> Hash {
    id(0xA1)
}

fn dog_b() -> Hash {
    id(0xB2)
}

fn household() -> Hash {
    id(0xC0)
}

fn chicago(y: i32, m: u32, d: u32, hour: u32, minute: u32) -> i64 {
    let tz: Tz = CHICAGO.parse().unwrap();
    let naive = NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap();
    tz.from_local_datetime(&naive).single().unwrap().timestamp()
}

fn midnight(y: i32, m: u32, d: u32) -> i64 {
    chicago(y, m, d, 0, 0)
}

fn expectation(id_byte: u8, subjects: Vec<Hash>, act: &str, window: Window) -> Expectation {
    Expectation {
        id: id(id_byte),
        subjects,
        act: act.to_owned(),
        window,
        active: ActiveWindow::Hours {
            hours: vec![HoursSpan {
                from_minute: 420,
                to_minute: 1320,
                days: None,
            }],
            tz: CHICAGO.to_owned(),
        },
        supersedes: None,
        ends_at: None,
        note: None,
    }
}

fn always(mut expectation: Expectation) -> Expectation {
    expectation.active = ActiveWindow::Always;
    expectation
}

fn record(id_byte: u8, subjects: Vec<Hash>, act: &str, at: i64) -> Record {
    Record {
        id: id(id_byte),
        subjects,
        act: act.to_owned(),
        at,
        tz_offset_minutes: -300,
        details: Vec::new(),
        shared: None,
        measure: None,
        note: None,
        place: None,
    }
}

fn round_trip_record(record: &Record) -> Vec<u8> {
    let bytes = encode_record_body(record).expect("encode");
    let decoded = decode_record_body(&bytes).expect("decode");
    assert_eq!(&decoded, record);
    let again = encode_record_body(&decoded).expect("re-encode");
    assert_eq!(again, bytes, "re-encoding must reproduce the same bytes");
    bytes
}

fn current(verdicts: &[Verdict], start: i64) -> Vec<&Verdict> {
    verdicts
        .iter()
        .filter(|verdict| verdict.window_start == start)
        .collect()
}

#[test]
fn worked_example_fed_twice_a_day() {
    // §4(a): one expectation, two records whose subjects cover both dogs.
    let mut fed = always(expectation(
        0x10,
        vec![dog_a(), dog_b()],
        "fed",
        Window::CountPerPeriod {
            count: 2,
            period: Period::Custom(PeriodCustom {
                anchor: midnight(2026, 6, 15),
                length_seconds: 86_400,
            }),
        },
    ));
    fed.note = Some("twice a day".to_owned());

    let mut morning = record(
        0x11,
        vec![dog_a(), dog_b()],
        "fed",
        chicago(2026, 6, 15, 8, 12),
    );
    morning.shared = Some(RecordDetail {
        note: None,
        outcome: None,
        measure: Some(Measure {
            value: 200.0,
            unit: "g".to_owned(),
        }),
    });
    let evening = record(
        0x12,
        vec![dog_a(), dog_b()],
        "fed",
        chicago(2026, 6, 15, 19, 40),
    );

    // §4(a): the evening record has no measure, and that is valid content.
    round_trip_record(&morning);
    round_trip_record(&evening);
    let bytes = encode_expectation_body(&fed).expect("encode");
    assert_eq!(decode_expectation_body(&bytes).expect("decode"), fed);

    let now = chicago(2026, 6, 15, 20, 0);
    let verdicts = evaluate(&[fed], &[morning, evening], now).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 6, 15));
    assert_eq!(today.len(), 2, "one verdict per subject");
    for verdict in today {
        assert_eq!(verdict.state, VerdictState::Satisfied);
        assert_eq!(verdict.count, 2);
        assert_eq!(verdict.records.len(), 2);
    }
    // The previous day was evaluated and was not fed: a real violation, not a
    // missing datum.
    let yesterday = current(&verdicts, midnight(2026, 6, 14));
    assert_eq!(yesterday.len(), 2);
    assert!(yesterday.iter().all(|v| v.state == VerdictState::Violated));
}

#[test]
fn worked_example_outside_every_two_to_four_hours() {
    // §4(b): one gap of 4h15m is the single violation.
    let outside = expectation(
        0x20,
        vec![dog_a(), dog_b()],
        "outside",
        Window::Interval {
            min_seconds: 7_200,
            max_seconds: 14_400,
        },
    );
    let records = [
        record(
            0x21,
            vec![dog_a(), dog_b()],
            "outside",
            chicago(2026, 6, 15, 7, 5),
        ),
        record(
            0x22,
            vec![dog_a(), dog_b()],
            "outside",
            chicago(2026, 6, 15, 9, 30),
        ),
        record(
            0x23,
            vec![dog_a(), dog_b()],
            "outside",
            chicago(2026, 6, 15, 12, 10),
        ),
        record(
            0x24,
            vec![dog_a(), dog_b()],
            "outside",
            chicago(2026, 6, 15, 16, 45),
        ),
        record(
            0x25,
            vec![dog_a(), dog_b()],
            "outside",
            chicago(2026, 6, 15, 21, 0),
        ),
    ];
    for record in &records {
        round_trip_record(record);
    }
    let now = chicago(2026, 6, 15, 22, 30);
    let verdicts = evaluate(&[outside.clone()], &records, now).expect("evaluate");
    assert_eq!(verdicts.len(), 2, "one span, one verdict per subject");
    for verdict in &verdicts {
        assert_eq!(verdict.state, VerdictState::Violated);
        assert_eq!(verdict.count, 5);
        // The design says "one violated interval", but its own record set has
        // two over-max gaps: 12:10 to 16:45 is 4h35 (16500s), and 16:45 to
        // 21:00 is 4h15 (15300s). The evaluator reports the first.
        assert!(
            verdict.detail.contains("16500"),
            "4h35m is the first violated gap: {}",
            verdict.detail
        );
    }

    // With the last gap inside the maximum, the same span is satisfied.
    let fixed = record(
        0x26,
        vec![dog_a(), dog_b()],
        "outside",
        chicago(2026, 6, 15, 20, 0),
    );
    let verdicts = evaluate(
        &[outside],
        &[
            records[0].clone(),
            records[1].clone(),
            records[2].clone(),
            records[3].clone(),
            fixed,
        ],
        now,
    )
    .expect("evaluate");
    assert!(verdicts.iter().all(|v| v.state == VerdictState::Satisfied));
}

#[test]
fn worked_example_one_vs_both_walk_with_divergent_detail() {
    // §4(c): one act, two subjects, divergent per-subject detail.
    let mut walk = record(
        0x30,
        vec![dog_a(), dog_b()],
        "walked",
        chicago(2026, 6, 15, 16, 0),
    );
    walk.shared = Some(RecordDetail {
        note: None,
        outcome: None,
        measure: Some(Measure {
            value: 25.0,
            unit: "min".to_owned(),
        }),
    });
    walk.details = vec![
        (
            dog_a(),
            RecordDetail {
                note: None,
                outcome: Some("peed".to_owned()),
                measure: None,
            },
        ),
        (
            dog_b(),
            RecordDetail {
                note: None,
                outcome: Some("did not pee".to_owned()),
                measure: None,
            },
        ),
    ];
    round_trip_record(&walk);
    assert_eq!(
        walk.detail_for(&dog_a()).unwrap().outcome.as_deref(),
        Some("peed")
    );
    assert_eq!(
        walk.detail_for(&dog_b()).unwrap().outcome.as_deref(),
        Some("did not pee")
    );
    assert_eq!(
        walk.shared
            .as_ref()
            .unwrap()
            .measure
            .as_ref()
            .unwrap()
            .value,
        25.0
    );

    // The one act satisfies an expectation for both subjects.
    let walked = always(expectation(
        0x31,
        vec![dog_a(), dog_b()],
        "walked",
        Window::AtLeastOncePer {
            period: Period::Custom(PeriodCustom {
                anchor: midnight(2026, 6, 15),
                length_seconds: 86_400,
            }),
        },
    ));
    let verdicts = evaluate(&[walked], &[walk], chicago(2026, 6, 15, 18, 0)).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 6, 15));
    assert_eq!(today.len(), 2);
    assert!(
        today
            .iter()
            .all(|v| v.state == VerdictState::Satisfied && v.count == 1)
    );
}

#[test]
fn worked_example_non_metric_water_record() {
    // §4(d): the record exists to say it happened, with no measure at all.
    let water = record(0x40, vec![dog_a()], "water", chicago(2026, 6, 15, 13, 0));
    let bytes = round_trip_record(&water);
    let decoded = decode_record_body(&bytes).expect("decode");
    assert!(decoded.measure.is_none());
    assert!(decoded.shared.is_none());
    assert!(decoded.details.is_empty());
    assert!(decoded.note.is_none());
}

#[test]
fn worked_example_opinion_attached_later() {
    // §4(e): the opinion names the record and never mutates it.
    let walk_id = id(0x30);
    let opinion = Opinion {
        about: vec![walk_id],
        stance: "bad".to_owned(),
        note: Some("25 minutes at 4pm in July is too hot for B".to_owned()),
        at: chicago(2026, 6, 15, 19, 0),
    };
    let bytes = encode_opinion_body(&opinion).expect("encode");
    let decoded = decode_opinion_body(&bytes).expect("decode");
    assert_eq!(decoded, opinion);
    assert_eq!(encode_opinion_body(&decoded).expect("re-encode"), bytes);
}

#[test]
fn dst_day_is_twenty_three_hours() {
    // §8: America/Chicago springs forward on 2026-03-08; the local day is 23h.
    let mut fed = always(expectation(
        0x50,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 2,
            period: Period::Day,
        },
    ));
    fed.active = ActiveWindow::Hours {
        hours: vec![HoursSpan {
            from_minute: 0,
            to_minute: 1440,
            days: None,
        }],
        tz: CHICAGO.to_owned(),
    };
    // 01:30 CST and 03:30 CDT are both on the same local day.
    let first = record(0x51, vec![dog_a()], "fed", chicago(2026, 3, 8, 1, 30));
    let second = record(0x52, vec![dog_a()], "fed", chicago(2026, 3, 8, 3, 30));
    let now = chicago(2026, 3, 8, 23, 0);
    let verdicts = evaluate(&[fed], &[first, second], now).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 3, 8));
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].state, VerdictState::Satisfied);
    assert_eq!(today[0].count, 2);
    assert_eq!(
        today[0].window_end - today[0].window_start,
        23 * 60 * 60,
        "a local day across spring-forward is 23 hours"
    );
}

#[test]
fn wrapping_active_span_is_handled() {
    // "Which may not be daytime": 20:00–06:00 is one span across midnight.
    let outside = expectation(
        0x60,
        vec![dog_a()],
        "outside",
        Window::Interval {
            min_seconds: 7_200,
            max_seconds: 14_400,
        },
    );
    let mut outside = outside;
    outside.active = ActiveWindow::Hours {
        hours: vec![HoursSpan {
            from_minute: 1200,
            to_minute: 420,
            days: None,
        }],
        tz: CHICAGO.to_owned(),
    };
    let records = [
        record(0x61, vec![dog_a()], "outside", chicago(2026, 6, 15, 23, 0)),
        record(0x62, vec![dog_a()], "outside", chicago(2026, 6, 16, 2, 0)),
        record(0x63, vec![dog_a()], "outside", chicago(2026, 6, 16, 5, 0)),
    ];
    let now = chicago(2026, 6, 16, 6, 0);
    let verdicts = evaluate(&[outside.clone()], &records, now).expect("evaluate");
    assert_eq!(verdicts.len(), 1, "one wrapping span reaches now");
    assert_eq!(verdicts[0].state, VerdictState::Satisfied);
    assert_eq!(verdicts[0].count, 3);

    // A gap below the minimum is a violation too.
    let too_soon = record(0x64, vec![dog_a()], "outside", chicago(2026, 6, 15, 23, 30));
    let verdicts = evaluate(&[outside], &[records[0].clone(), too_soon], now).expect("evaluate");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].state, VerdictState::Violated);
}

#[test]
fn supersede_is_order_independent() {
    // §8: the later declaration wins by reference, not by arrival position.
    let older = always(expectation(
        0x70,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 2,
            period: Period::Day,
        },
    ));
    let mut newer = always(expectation(
        0x71,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 3,
            period: Period::Day,
        },
    ));
    newer.supersedes = Some(older.id);
    let now = chicago(2026, 6, 15, 20, 0);
    let forwards = evaluate(&[older.clone(), newer.clone()], &[], now).expect("evaluate");
    let backwards = evaluate(&[newer.clone(), older], &[], now).expect("evaluate");
    assert_eq!(forwards, backwards);
    assert!(
        forwards
            .iter()
            .all(|verdict| verdict.expectation == newer.id),
        "only the superseding expectation is evaluated"
    );
}

#[test]
fn ends_at_stops_evaluation() {
    let mut ended = always(expectation(
        0x80,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    ));
    ended.ends_at = Some(chicago(2026, 6, 15, 0, 0));
    let verdicts = evaluate(&[ended], &[], chicago(2026, 6, 15, 20, 0)).expect("evaluate");
    assert!(verdicts.is_empty());
}

#[test]
fn empty_subjects_are_invalid() {
    let mut record = record(0x90, vec![], "fed", chicago(2026, 6, 15, 8, 0));
    let error = encode_record_body(&record).expect_err("empty subjects are refused");
    assert!(matches!(error, DiditError::Invalid(_)), "{error:?}");

    let expectation = Expectation {
        id: id(0x91),
        subjects: vec![],
        act: "fed".to_owned(),
        window: Window::AtLeastOncePer {
            period: Period::Day,
        },
        active: ActiveWindow::Always,
        supersedes: None,
        ends_at: None,
        note: None,
    };
    assert!(matches!(
        encode_expectation_body(&expectation).expect_err("refused"),
        DiditError::Invalid(_)
    ));

    let opinion = Opinion {
        about: vec![],
        stance: "good".to_owned(),
        note: None,
        at: 0,
    };
    assert!(matches!(
        encode_opinion_body(&opinion).expect_err("refused"),
        DiditError::Invalid(_)
    ));

    // Details must name a subject the record lists.
    record.subjects = vec![dog_a()];
    record.details = vec![(
        dog_b(),
        RecordDetail {
            note: None,
            outcome: None,
            measure: None,
        },
    )];
    assert!(matches!(
        encode_record_body(&record).expect_err("refused"),
        DiditError::Invalid(_)
    ));
}

#[test]
fn unknown_time_zone_is_refused() {
    let mut expectation = expectation(
        0xA0,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    );
    expectation.active = ActiveWindow::Hours {
        hours: vec![HoursSpan {
            from_minute: 0,
            to_minute: 60,
            days: None,
        }],
        tz: "Mars/Olympus".to_owned(),
    };
    assert!(matches!(
        encode_expectation_body(&expectation).expect_err("refused"),
        DiditError::Invalid(_)
    ));
    // A caller that built the type without checking still gets a clean error.
    let error = evaluate(&[expectation], &[], 0).expect_err("unknown zone");
    assert_eq!(error, EvalError::UnknownTimeZone("Mars/Olympus".to_owned()));
}

#[test]
fn canonical_and_kind_checks() {
    let entity = Entity {
        entity: dog_a(),
        household: household(),
        kind: "dog".to_owned(),
        name: "Ada".to_owned(),
        aliases: vec!["Adie".to_owned()],
        retired_at: None,
    };
    let bytes = encode_entity_body(&entity).expect("encode");
    assert_eq!(decode_entity_body(&bytes).expect("decode"), entity);

    // The decoder refuses a body tagged as another kind.
    assert!(matches!(
        decode_record_body(&bytes).expect_err("wrong kind"),
        DiditError::Kind(_, RECORD_KIND)
    ));

    // A non-canonical encoding (trailing bytes) is refused.
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(decode_entity_body(&longer).is_err());

    // An unknown format version is refused.
    let mut items = match Value::decode(&bytes).expect("decode value") {
        Value::Array(items) => items,
        other => panic!("not an array: {other:?}"),
    };
    items[1] = Value::Unsigned(VERSION + 1);
    let future = Value::Array(items).encode();
    assert!(matches!(
        decode_entity_body(&future).expect_err("future version"),
        DiditError::Version(_)
    ));
}
