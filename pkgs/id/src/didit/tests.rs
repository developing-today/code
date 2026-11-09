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

fn overnight_hours(mut expectation: Expectation) -> Expectation {
    expectation.active = ActiveWindow::Hours {
        hours: vec![HoursSpan {
            from_minute: 1200,
            to_minute: 360,
            days: None,
        }],
        tz: CHICAGO.to_owned(),
    };
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

    // With every gap inside the window, the same span is satisfied. The
    // record set above cannot be repaired by moving only the last record:
    // 12:10 to 16:45 is itself a 16500s gap. This set keeps 07:05, 09:30,
    // 12:10, 16:45 and 21:00 and inserts 14:30 and 19:00, so every
    // consecutive gap is inside 7200..=14400 (8700, 9600, 8400, 8100,
    // 8100, 7200) and the span-end gap (22:00 - 21:00) is 3600.
    let at_14_30 = record(
        0x26,
        vec![dog_a(), dog_b()],
        "outside",
        chicago(2026, 6, 15, 14, 30),
    );
    let at_19_00 = record(
        0x27,
        vec![dog_a(), dog_b()],
        "outside",
        chicago(2026, 6, 15, 19, 0),
    );
    let verdicts = evaluate(
        &[outside],
        &[
            records[0].clone(),
            records[1].clone(),
            records[2].clone(),
            at_14_30,
            records[3].clone(),
            at_19_00,
            records[4].clone(),
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
fn day_count_includes_previous_days_wrapping_span() {
    let outside = overnight_hours(expectation(
        0x55,
        vec![dog_a()],
        "outside",
        Window::CountPerPeriod {
            count: 1,
            period: Period::Day,
        },
    ));
    let at_2am = record(0x56, vec![dog_a()], "outside", chicago(2026, 6, 16, 2, 0));
    let now = chicago(2026, 6, 16, 12, 0);

    let verdicts = evaluate(&[outside], &[at_2am], now).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 6, 16));
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].state, VerdictState::Satisfied);
    assert_eq!(today[0].count, 1);
}

#[test]
fn count_verdict_is_violated_after_last_active_span_closes() {
    // #5049: CountPerPeriod 2/Day with the default active window
    // (07:00-22:00 America/Chicago) and no records. Once the period's last
    // active span closes, no further record can count in the period, so an
    // unmet count is Violated, not Due until the period end.
    let fed = expectation(
        0x70,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 2,
            period: Period::Day,
        },
    );
    for (now, expected) in [
        (chicago(2026, 6, 15, 23, 0), VerdictState::Violated),
        // Exactly at the span close is already unmeetable.
        (chicago(2026, 6, 15, 22, 0), VerdictState::Violated),
        // Mid-span control: a record can still count, so still Due.
        (chicago(2026, 6, 15, 12, 0), VerdictState::Due),
    ] {
        let verdicts = evaluate(&[fed.clone()], &[], now).expect("evaluate");
        let today = current(&verdicts, midnight(2026, 6, 15));
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].state, expected, "now={now}");
        assert_eq!(today[0].count, 0);
    }
    // ActiveWindow::Always is unchanged: its span is the whole period, so
    // late in the period an unmet count is still Due. (A Custom period is
    // used here because `Always` periods are UTC-calendar based; the
    // anchor pins the window start to the same Chicago midnight.)
    let fed_always = always(expectation(
        0x72,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 2,
            period: Period::Custom(PeriodCustom {
                anchor: midnight(2026, 6, 15),
                length_seconds: 86_400,
            }),
        },
    ));
    let verdicts = evaluate(&[fed_always], &[], chicago(2026, 6, 15, 23, 0)).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 6, 15));
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].state, VerdictState::Due);
}

#[test]
fn count_verdict_wrapping_span_stays_due_while_a_record_can_still_count() {
    // #5049 wrapping case: the active span wraps past midnight
    // (20:00-06:00). At 07:00 the previous night's span has closed, but
    // tonight's span still offers countable time inside today's period,
    // so an unmet count stays Due.
    let mut fed = expectation(
        0x71,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 1,
            period: Period::Day,
        },
    );
    fed.active = ActiveWindow::Hours {
        hours: vec![HoursSpan {
            from_minute: 1200,
            to_minute: 360,
            days: None,
        }],
        tz: CHICAGO.to_owned(),
    };
    let verdicts = evaluate(&[fed], &[], chicago(2026, 6, 16, 7, 0)).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 6, 16));
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].state, VerdictState::Due);
}

#[test]
fn count_verdict_overnight_carry_in_span_is_not_unmeetable() {
    // #5049, Codex review on the fix: the only active span starts Monday
    // 20:00 and wraps to Tuesday 06:00. On Tuesday the period's countable
    // time is that carry-in portion, so at 02:00 an unmet count is Due
    // (a record can still count), and only after 06:00 is it Violated.
    // 2026-06-15 is a Monday; days are numbered from Monday (0).
    let mut fed = expectation(
        0x73,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 1,
            period: Period::Day,
        },
    );
    fed.active = ActiveWindow::Hours {
        hours: vec![HoursSpan {
            from_minute: 1200,
            to_minute: 360,
            days: Some(vec![0]),
        }],
        tz: CHICAGO.to_owned(),
    };
    for (now, expected) in [
        (chicago(2026, 6, 16, 2, 0), VerdictState::Due),
        (chicago(2026, 6, 16, 7, 0), VerdictState::Violated),
    ] {
        let verdicts = evaluate(&[fed.clone()], &[], now).expect("evaluate");
        let today = current(&verdicts, midnight(2026, 6, 16));
        assert_eq!(today.len(), 1);
        assert_eq!(today[0].state, expected, "now={now}");
    }
    // A record inside the carry-in portion counts toward Tuesday.
    let carry_in = record(0x74, vec![dog_a()], "fed", chicago(2026, 6, 16, 2, 0));
    let verdicts = evaluate(&[fed], &[carry_in], chicago(2026, 6, 16, 3, 0)).expect("evaluate");
    let today = current(&verdicts, midnight(2026, 6, 16));
    assert_eq!(today.len(), 1);
    assert_eq!(today[0].state, VerdictState::Satisfied);
    assert_eq!(today[0].count, 1);
}

#[test]
fn week_count_includes_previous_days_wrapping_span() {
    let outside = overnight_hours(expectation(
        0x57,
        vec![dog_a()],
        "outside",
        Window::CountPerPeriod {
            count: 1,
            period: Period::Week,
        },
    ));
    let at_2am = record(0x58, vec![dog_a()], "outside", chicago(2026, 6, 15, 2, 0));
    let now = chicago(2026, 6, 15, 12, 0);

    let verdicts = evaluate(&[outside], &[at_2am], now).expect("evaluate");
    let this_week = current(&verdicts, midnight(2026, 6, 15));
    assert_eq!(this_week.len(), 1);
    assert_eq!(this_week[0].state, VerdictState::Satisfied);
    assert_eq!(this_week[0].count, 1);
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
fn self_supersede_is_rejected_when_decoding() {
    let valid = always(expectation(
        0x72,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    ));
    let encoded = encode_expectation_body(&valid).expect("encode");
    let mut items = match Value::decode(&encoded).expect("decode value") {
        Value::Array(items) => items,
        other => panic!("not an array: {other:?}"),
    };
    items[7] = Value::ByteString(valid.id.as_bytes().to_vec().into());
    let invalid_body = Value::Array(items).encode();
    assert!(matches!(
        decode_expectation_body(&invalid_body),
        Err(DiditError::Invalid(_))
    ));

    let mut self_superseding = valid;
    self_superseding.supersedes = Some(self_superseding.id);
    assert!(matches!(
        encode_expectation_body(&self_superseding),
        Err(DiditError::Invalid(_))
    ));
}

fn supersede_cycle(ids: &[u8]) -> Vec<Expectation> {
    let mut expectations: Vec<_> = ids
        .iter()
        .map(|byte| {
            always(expectation(
                *byte,
                vec![dog_a()],
                "fed",
                Window::AtLeastOncePer {
                    period: Period::Day,
                },
            ))
        })
        .collect();
    for index in 0..expectations.len() {
        let superseded = expectations[(index + 1) % expectations.len()].id;
        expectations[index].supersedes = Some(superseded);
    }
    expectations
}

#[test]
fn evaluator_rejects_a_self_supersede_cycle() {
    let mut self_superseding = always(expectation(
        0x73,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    ));
    self_superseding.supersedes = Some(self_superseding.id);
    assert_eq!(
        evaluate(&[self_superseding], &[], chicago(2026, 6, 15, 20, 0)).expect_err("cycle"),
        EvalError::SupersedeCycle(vec![id(0x73)])
    );
}

#[test]
fn evaluator_rejects_a_two_expectation_supersede_cycle() {
    let cycle = supersede_cycle(&[0x81, 0x80]);
    assert_eq!(
        evaluate(&cycle, &[], chicago(2026, 6, 15, 20, 0)).expect_err("cycle"),
        EvalError::SupersedeCycle(vec![id(0x80), id(0x81)])
    );
}

#[test]
fn evaluator_rejects_a_three_expectation_supersede_cycle() {
    let cycle = supersede_cycle(&[0x84, 0x82, 0x83]);
    assert_eq!(
        evaluate(&cycle, &[], chicago(2026, 6, 15, 20, 0)).expect_err("cycle"),
        EvalError::SupersedeCycle(vec![id(0x82), id(0x83), id(0x84)])
    );
}

#[test]
fn evaluator_keeps_only_the_head_of_a_supersede_chain() {
    let oldest = always(expectation(
        0x87,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    ));
    let mut middle = always(expectation(
        0x86,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    ));
    let mut head = always(expectation(
        0x85,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Day,
        },
    ));
    middle.supersedes = Some(oldest.id);
    head.supersedes = Some(middle.id);
    let verdicts = evaluate(
        &[oldest, middle, head.clone()],
        &[],
        chicago(2026, 6, 15, 20, 0),
    )
    .expect("normal chain evaluates");
    assert!(!verdicts.is_empty());
    assert!(
        verdicts
            .iter()
            .all(|verdict| verdict.expectation == head.id)
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
#[test]
fn interval_gap_crossing_midnight_is_judged_across_the_seam() {
    // #5144: an Always expectation is one continuous active stretch,
    // but spans arrive per local day. A gap crossing midnight must be
    // judged as the single consecutive gap it is, not as two edge
    // gaps that each fit under the max.
    let outside = always(expectation(
        0xE0,
        vec![dog_a()],
        "outside",
        Window::Interval {
            min_seconds: 0,
            max_seconds: 3_600,
        },
    ));
    // 2026-06-15T00:00:00Z; an Always expectation evaluates in UTC.
    let day = 1_781_481_600;
    let mut records = Vec::new();
    // Hourly through Jun 15, then 01:00 and 02:00 on Jun 16 — the
    // 00:00 record is skipped, so the consecutive gap across the
    // seam is 23:00 -> 01:00 = 7200s, twice the max.
    for hour in 0..24i64 {
        records.push(record(
            0x30 + hour as u8,
            vec![dog_a()],
            "outside",
            day + hour * 3_600,
        ));
    }
    records.push(record(0x48, vec![dog_a()], "outside", day + 25 * 3_600));
    records.push(record(0x49, vec![dog_a()], "outside", day + 26 * 3_600));
    let now = day + 26 * 3_600 + 1_800;
    let verdicts = evaluate(&[outside], &records, now).expect("evaluate");
    assert_eq!(verdicts.len(), 1, "the two days merge into one span");
    let verdict = &verdicts[0];
    assert_eq!(verdict.state, VerdictState::Violated);
    assert_eq!(verdict.count, 26);
    assert!(
        verdict.detail.contains("7200"),
        "the cross-seam gap is the violation: {}",
        verdict.detail
    );
    // The verdict window reflects the merged continuous span.
    assert_eq!(verdict.window_start, day);
    assert_eq!(verdict.window_end, day + 2 * 86_400);
}

#[test]
fn interval_gap_inside_one_day_still_violates() {
    // The same 7200s gap inside a single day was always judged by the
    // pair check; merging spans must keep that verdict (#5144 control).
    let outside = always(expectation(
        0xE1,
        vec![dog_a()],
        "outside",
        Window::Interval {
            min_seconds: 0,
            max_seconds: 3_600,
        },
    ));
    let day = 1_781_481_600; // 2026-06-15T00:00:00Z
    let mut records = Vec::new();
    // Hourly through Jun 14, so the previous day is satisfied and the
    // merged span's first gap is zero.
    for hour in 0..24i64 {
        records.push(record(
            0x30 + hour as u8,
            vec![dog_a()],
            "outside",
            day - 86_400 + hour * 3_600,
        ));
    }
    // Hourly through Jun 15 with 12:00 skipped: 11:00 -> 13:00 = 7200s.
    for hour in 0..24i64 {
        if hour == 12 {
            continue;
        }
        records.push(record(
            0x50 + hour as u8,
            vec![dog_a()],
            "outside",
            day + hour * 3_600,
        ));
    }
    let now = day + 13 * 3_600 + 1_800;
    let verdicts = evaluate(&[outside], &records, now).expect("evaluate");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].state, VerdictState::Violated);
    assert_eq!(verdicts[0].count, 47);
    assert!(
        verdicts[0].detail.contains("7200"),
        "{}",
        verdicts[0].detail
    );
}

#[test]
fn interval_gap_crossing_sleep_window_seam_is_judged_across_it() {
    // #5144: a non-wrapping sleep window's active complement is cut
    // at midnight — yesterday's (wake, midnight) piece and today's
    // (midnight, sleep) piece are one continuous stretch, and a gap
    // across that seam is one consecutive gap.
    let mut outside = expectation(
        0xE2,
        vec![dog_a()],
        "outside",
        Window::Interval {
            min_seconds: 0,
            max_seconds: 3_600,
        },
    );
    // Asleep 01:00-07:00 UTC; active 07:00 -> 01:00 across midnight.
    outside.active = ActiveWindow::SleepWindow {
        from_minute: 60,
        to_minute: 420,
        tz: "UTC".to_owned(),
    };
    let day = 1_781_481_600; // 2026-06-15T00:00:00Z
    let mut records = Vec::new();
    // Hourly 07:00-23:00 on Jun 15.
    for hour in 7..24i64 {
        records.push(record(
            0x30 + hour as u8,
            vec![dog_a()],
            "outside",
            day + hour * 3_600,
        ));
    }
    // The 00:00 hour is skipped; the next record is 00:45, still
    // inside the active stretch: 23:00 -> 00:45 = 6300s > 3600s max.
    records.push(record(0x48, vec![dog_a()], "outside", day + 86_400 + 2_700));
    let now = day + 86_400 + 3_000;
    let verdicts = evaluate(&[outside], &records, now).expect("evaluate");
    let seam = current(&verdicts, day + 7 * 3_600);
    assert_eq!(seam.len(), 1, "one verdict for the merged seam span");
    assert_eq!(seam[0].state, VerdictState::Violated);
    assert_eq!(seam[0].count, 18);
    assert!(seam[0].detail.contains("6300"), "{}", seam[0].detail);
    assert_eq!(seam[0].window_end, day + 86_400 + 3_600);
}

#[test]
fn entity_alias_count_is_capped() {
    use crate::didit::body::MAX_ALIASES;
    // Oracle: the named cap MAX_ALIASES (mirroring MAX_SUBJECTS/MAX_DETAILS),
    // sized well under the CBOR decoder's 8192-element LENGTH_LIMIT — a body
    // at the cap round-trips, one alias over is refused by encode with an
    // error naming the count and the limit.
    let mut entity = Entity {
        entity: dog_a(),
        household: household(),
        kind: "dog".to_owned(),
        name: "Ada".to_owned(),
        aliases: (0..MAX_ALIASES).map(|i| format!("alias-{i}")).collect(),
        retired_at: None,
    };
    let bytes = encode_entity_body(&entity).expect("at the cap encodes");
    assert_eq!(decode_entity_body(&bytes).expect("decode"), entity);

    entity.aliases.push("one-too-many".to_owned());
    assert!(matches!(
        encode_entity_body(&entity).expect_err("over the cap is refused"),
        DiditError::Invalid(message)
            if message.contains("513") && message.contains("512")
    ));
}

#[test]
fn hours_span_count_is_capped() {
    use crate::didit::body::MAX_HOURS_SPANS;
    // Oracle: the named cap MAX_HOURS_SPANS, same shape as the alias cap —
    // an expectation at the cap round-trips, one span over is refused by
    // encode with an error naming the count and the limit.
    let span = HoursSpan {
        from_minute: 420,
        to_minute: 1320,
        days: None,
    };
    let mut fed = expectation(
        0x61,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 1,
            period: Period::Custom(PeriodCustom {
                anchor: midnight(2026, 6, 15),
                length_seconds: 86_400,
            }),
        },
    );
    fed.active = ActiveWindow::Hours {
        hours: vec![span.clone(); MAX_HOURS_SPANS],
        tz: CHICAGO.to_owned(),
    };
    let bytes = encode_expectation_body(&fed).expect("at the cap encodes");
    assert_eq!(decode_expectation_body(&bytes).expect("decode"), fed);

    let ActiveWindow::Hours { hours, .. } = &mut fed.active else {
        panic!("the expectation carries an hours window");
    };
    hours.push(span);
    assert!(matches!(
        encode_expectation_body(&fed).expect_err("over the cap is refused"),
        DiditError::Invalid(message)
            if message.contains("513") && message.contains("512")
    ));
}

#[test]
fn custom_period_extreme_anchor_is_out_of_range_not_a_panic_or_wrapped_window() {
    // #7766: Period::check rejects only a zero length, so a body can carry
    // any i64 anchor. count_periods must answer OutOfRange for windows its
    // arithmetic cannot represent — the unchecked operators panicked in
    // debug builds and silently wrapped to a phantom (or end-before-start)
    // window in release builds.
    for (anchor, length_seconds, at) in [
        (i64::MIN, 86_400_u64, 0_i64),
        (i64::MAX - 100, 1_000, i64::MAX - 100),
        (i64::MIN + 1_000, 86_400, i64::MIN + 1_000),
    ] {
        let fed = always(expectation(
            0x62,
            vec![dog_a()],
            "fed",
            Window::CountPerPeriod {
                count: 1,
                period: Period::Custom(PeriodCustom {
                    anchor,
                    length_seconds,
                }),
            },
        ));
        assert_eq!(
            evaluate(&[fed], &[], at),
            Err(EvalError::OutOfRange),
            "anchor={anchor} length={length_seconds}"
        );
    }
    // An ordinary custom period still evaluates.
    let fed = always(expectation(
        0x63,
        vec![dog_a()],
        "fed",
        Window::CountPerPeriod {
            count: 1,
            period: Period::Custom(PeriodCustom {
                anchor: midnight(2026, 6, 15),
                length_seconds: 86_400,
            }),
        },
    ));
    assert!(evaluate(&[fed], &[], midnight(2026, 6, 16)).is_ok());
}

#[test]
fn week_period_at_calendar_boundary_is_out_of_range_not_a_panic() {
    // #7771: the Week branch of count_periods used unchecked
    // NaiveDate +/- Duration, which panics (chrono's expect) when
    // monday + 7, monday - 7, or the Monday offset itself leaves the
    // representable calendar — where the Day branch answers
    // OutOfRange via succ_opt/pred_opt (and Custom since #7766).
    // `always` expectations evaluate in UTC.
    for (label, at) in [
        (
            "max",
            NaiveDate::MAX
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp(),
        ),
        (
            "min",
            NaiveDate::MIN
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp(),
        ),
    ] {
        let fed = always(expectation(
            0x64,
            vec![dog_a()],
            "fed",
            Window::AtLeastOncePer {
                period: Period::Week,
            },
        ));
        assert_eq!(
            evaluate(&[fed], &[], at),
            Err(EvalError::OutOfRange),
            "week period at calendar {label}"
        );
    }
    // An ordinary week still evaluates.
    let fed = always(expectation(
        0x65,
        vec![dog_a()],
        "fed",
        Window::AtLeastOncePer {
            period: Period::Week,
        },
    ));
    assert!(evaluate(&[fed], &[], midnight(2026, 6, 16)).is_ok());
}
