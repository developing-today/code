//! The pure did-it evaluator (design §1.2, §3.3, decision D2).
//!
//! [`evaluate`] reads expectations and records and returns one verdict per
//! expectation, subject, and window. It writes nothing and keeps no `due`
//! state: the same inputs always produce the same verdicts, on every device,
//! and the function can be replayed for any past instant.
//!
//! Windows evaluate in the **expectation's** IANA `tz`, converted from each
//! record's absolute `at` (design open question 1). A record's own
//! `tz_offset_minutes` is never used for a boundary; a travelling author's
//! disagreement stays visible instead of being silently coerced.
//!
//! Superseding is a set operation, not an arrival-order one: an expectation
//! named by another's `supersedes` is not evaluated, whatever order the events
//! are passed in (design §8, "supersede ordering").
//!
//! A supersede chain that returns to one of its expectation IDs is invalid:
//! cycles have no latest event, so evaluation returns [`EvalError::SupersedeCycle`]
//! with the cycle IDs sorted by bytes. This also catches self-loops built in
//! memory that bypass body validation.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use p2panda_core::Hash;

use super::body::{ActiveWindow, Expectation, HoursSpan, Period, Record, Window};

/// Why an evaluation could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvalError {
    /// The expectation names a time zone this build does not know.
    #[error("{0:?} is not a known IANA time zone")]
    UnknownTimeZone(String),
    /// An instant is outside the range the calendar can represent.
    #[error("a timestamp is out of range")]
    OutOfRange,
    /// A supersede chain returns to one of its expectation IDs.
    #[error("supersede cycle among expectation ids: {0:?}")]
    SupersedeCycle(Vec<Hash>),
}

/// How an expectation is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerdictState {
    /// The window's requirement is met.
    Satisfied,
    /// The window is still open and the requirement can still be met.
    Due,
    /// The window is still open and the requirement is already late.
    Overdue,
    /// A completed window, or a completed gap, breaks the requirement.
    Violated,
}

/// One evaluation result: an expectation, one of its subjects, and one window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The expectation's event ID.
    pub expectation: Hash,
    /// The entity this verdict is about.
    pub subject: Hash,
    /// The window's start, in unix seconds.
    pub window_start: i64,
    /// The window's end, in unix seconds.
    pub window_end: i64,
    /// The verdict.
    pub state: VerdictState,
    /// The number of matching records inside the window.
    pub count: u64,
    /// The IDs of the matching records, in time order.
    pub records: Vec<Hash>,
    /// Why the verdict is what it is, in one short phrase.
    pub detail: String,
}

/// Evaluates `expectations` against `records` at `now`.
///
/// Expectations that another live expectation supersedes are skipped, as are
/// expectations whose `ends_at` has passed. Every remaining expectation is
/// evaluated per subject: a record satisfies a subject when it names that
/// subject, matches the expectation's `act`, and falls inside the window and
/// the active spans.
///
/// Count windows produce verdicts for the period containing `now` and the
/// preceding completed period. Interval windows produce verdicts for the active
/// spans that have begun within the last two local days.
///
/// # Errors
///
/// Returns [`EvalError::UnknownTimeZone`] for a zone this build does not know,
/// [`EvalError::OutOfRange`] for an instant outside the calendar range, or
/// [`EvalError::SupersedeCycle`] when superseding expectations form a cycle.
pub fn evaluate(
    expectations: &[Expectation],
    records: &[Record],
    now: i64,
) -> Result<Vec<Verdict>, EvalError> {
    check_supersede_cycles(expectations)?;
    let superseded: HashSet<Hash> = expectations.iter().filter_map(|e| e.supersedes).collect();
    let mut verdicts = Vec::new();
    for expectation in expectations {
        if superseded.contains(&expectation.id) {
            continue;
        }
        if expectation.ends_at.is_some_and(|end| end <= now) {
            continue;
        }
        for subject in &expectation.subjects {
            match &expectation.window {
                Window::CountPerPeriod { .. } | Window::AtLeastOncePer { .. } => {
                    verdicts.extend(count_verdicts(expectation, subject, records, now)?);
                }
                Window::Interval { .. } => {
                    verdicts.extend(interval_verdicts(expectation, subject, records, now)?);
                }
            }
        }
    }
    verdicts.sort_by(|a, b| {
        a.expectation
            .as_bytes()
            .cmp(b.expectation.as_bytes())
            .then_with(|| a.subject.as_bytes().cmp(b.subject.as_bytes()))
            .then_with(|| a.window_start.cmp(&b.window_start))
    });
    Ok(verdicts)
}

fn check_supersede_cycles(expectations: &[Expectation]) -> Result<(), EvalError> {
    let supersedes: HashMap<Hash, Hash> = expectations
        .iter()
        .filter_map(|expectation| {
            expectation
                .supersedes
                .map(|superseded| (expectation.id, superseded))
        })
        .collect();
    let mut starts: Vec<Hash> = expectations
        .iter()
        .map(|expectation| expectation.id)
        .collect();
    starts.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    starts.dedup();

    let mut finished = HashSet::new();
    for start in starts {
        if finished.contains(&start) {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = HashMap::new();
        let mut current = start;
        while !finished.contains(&current) {
            if let Some(cycle_start) = positions.get(&current).copied() {
                let mut cycle = path[cycle_start..].to_vec();
                cycle.sort_by(|left: &Hash, right: &Hash| left.as_bytes().cmp(right.as_bytes()));
                return Err(EvalError::SupersedeCycle(cycle));
            }
            let Some(next) = supersedes.get(&current).copied() else {
                break;
            };
            positions.insert(current, path.len());
            path.push(current);
            current = next;
        }
        finished.extend(path);
    }
    Ok(())
}

fn count_verdicts(
    expectation: &Expectation,
    subject: &Hash,
    records: &[Record],
    now: i64,
) -> Result<Vec<Verdict>, EvalError> {
    let tz = expectation_tz(expectation)?;
    let (current, previous) = count_periods(tz, &expectation.window, now)?;
    let required = match &expectation.window {
        Window::CountPerPeriod { count, .. } => *count,
        Window::AtLeastOncePer { .. } => 1,
        Window::Interval { .. } => unreachable!("count_verdicts is only called for count windows"),
    };
    let mut verdicts = Vec::new();
    for (start, end) in [previous, current] {
        if end <= start {
            continue;
        }
        let spans = active_spans_between(tz, &expectation.active, start, end)?;
        let mut matching: Vec<&Record> = records
            .iter()
            .filter(|record| {
                record.act == expectation.act
                    && record.subjects.contains(subject)
                    && record.at >= start
                    && record.at < end
                    && in_spans(record.at, &spans)
            })
            .collect();
        matching.sort_by_key(|record| record.at);
        let count = matching.len() as u64;
        // No record can count after the period's last active span closes:
        // records are filtered to spans clipped to this period, so the last
        // countable instant is the latest span end inside the period (for
        // `Always` that is the period end itself, leaving it unchanged). A
        // period with no active span at all is unmeetable from its start.
        // A wrapping span is clipped at the period end, matching the
        // `record.at < end` filter above.
        let last_countable = spans
            .iter()
            .filter(|(span_start, span_end)| *span_start < end && *span_end > start)
            .map(|(_, span_end)| (*span_end).min(end))
            .max();
        let unmeetable = match last_countable {
            Some(deadline) => deadline <= now,
            None => start <= now,
        };
        let state = if count >= required {
            VerdictState::Satisfied
        } else if end <= now || unmeetable {
            VerdictState::Violated
        } else {
            VerdictState::Due
        };
        let detail = format!("{count} of {required} records in the window");
        verdicts.push(Verdict {
            expectation: expectation.id,
            subject: *subject,
            window_start: start,
            window_end: end,
            state,
            count,
            records: matching.iter().map(|record| record.id).collect(),
            detail,
        });
    }
    Ok(verdicts)
}

fn interval_verdicts(
    expectation: &Expectation,
    subject: &Hash,
    records: &[Record],
    now: i64,
) -> Result<Vec<Verdict>, EvalError> {
    let tz = expectation_tz(expectation)?;
    let (min_seconds, max_seconds) = match &expectation.window {
        Window::Interval {
            min_seconds,
            max_seconds,
        } => (*min_seconds as i64, *max_seconds as i64),
        _ => unreachable!("interval_verdicts is only called for interval windows"),
    };
    let today = local_date(now, tz)?;
    // A span that wraps past midnight began on the previous local day, so both
    // days are scanned.
    let mut spans = Vec::new();
    for date in [today.pred_opt(), Some(today)].into_iter().flatten() {
        for span in spans_for_date(tz, &expectation.active, date)? {
            // Only spans that have begun and ended or are ongoing within the
            // last day are current enough to judge.
            if span.0 <= now && span.1 > now - 86_400 {
                spans.push(span);
            }
        }
    }
    spans.sort_unstable();
    spans.dedup();
    // Adjacent spans are one continuous active stretch split only by
    // the calendar: an Always day ends where the next begins, and a
    // non-wrapping sleep window's active complement is cut at
    // midnight. Merge spans that touch or overlap before judging, so
    // a record gap crossing the seam is judged as the single
    // consecutive gap it is (#5144) — split at the seam, each edge gap
    // could pass while the real gap ran to nearly twice the max.
    // Verdict windows below therefore span the merged stretch.
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 <= last.1 => last.1 = last.1.max(span.1),
            _ => merged.push(span),
        }
    }
    let spans = merged;
    let mut verdicts = Vec::new();
    for (start, end) in spans {
        let mut matching: Vec<&Record> = records
            .iter()
            .filter(|record| {
                record.act == expectation.act
                    && record.subjects.contains(subject)
                    && record.at >= start
                    && record.at < end
            })
            .collect();
        matching.sort_by_key(|record| record.at);
        let anchors: Vec<i64> = matching.iter().map(|record| record.at).collect();
        let mut state = VerdictState::Satisfied;
        let mut detail = "every gap is inside the window".to_owned();
        if anchors.is_empty() {
            if end <= now {
                if end - start > max_seconds {
                    state = VerdictState::Violated;
                    detail = "the active span ended with no record".to_owned();
                }
            } else {
                let gap = now - start;
                if gap > max_seconds {
                    state = VerdictState::Overdue;
                    detail = format!("{gap}s since the span began, the max is {max_seconds}s");
                } else if gap >= min_seconds {
                    state = VerdictState::Due;
                    detail = format!("{gap}s since the span began");
                }
            }
        } else {
            let first_gap = anchors[0] - start;
            if first_gap > max_seconds {
                state = VerdictState::Violated;
                detail = format!("first record {first_gap}s after the span began");
            } else {
                for pair in anchors.windows(2) {
                    let gap = pair[1] - pair[0];
                    if gap < min_seconds || gap > max_seconds {
                        state = VerdictState::Violated;
                        detail = format!("a {gap}s gap is outside {min_seconds}..={max_seconds}s");
                        break;
                    }
                }
            }
            if state != VerdictState::Violated {
                let last = *anchors.last().expect("anchors is not empty");
                if end <= now {
                    let gap = end - last;
                    if gap > max_seconds {
                        state = VerdictState::Violated;
                        detail = format!("{gap}s from the last record to the span's end");
                    }
                } else {
                    let gap = now - last;
                    if gap > max_seconds {
                        state = VerdictState::Overdue;
                        detail = format!("{gap}s since the last record, the max is {max_seconds}s");
                    } else if gap >= min_seconds {
                        state = VerdictState::Due;
                        detail = format!("{gap}s since the last record");
                    }
                }
            }
        }
        verdicts.push(Verdict {
            expectation: expectation.id,
            subject: *subject,
            window_start: start,
            window_end: end,
            state,
            count: matching.len() as u64,
            records: matching.iter().map(|record| record.id).collect(),
            detail,
        });
    }
    Ok(verdicts)
}

fn expectation_tz(expectation: &Expectation) -> Result<Tz, EvalError> {
    match expectation.active.tz() {
        Some(name) => name
            .parse::<Tz>()
            .map_err(|_| EvalError::UnknownTimeZone(name.to_owned())),
        // `Always` has no zone of its own; periods still need a calendar, and
        // UTC is the only zone-free choice. Hours windows always name a zone.
        None => Ok(Tz::UTC),
    }
}

/// The current and immediately preceding period windows for a count window.
fn count_periods(tz: Tz, window: &Window, now: i64) -> Result<((i64, i64), (i64, i64)), EvalError> {
    let period = match window {
        Window::CountPerPeriod { period, .. } | Window::AtLeastOncePer { period } => period,
        Window::Interval { .. } => return Err(EvalError::OutOfRange),
    };
    match period {
        Period::Day => {
            let today = local_date(now, tz)?;
            let start = midnight(tz, today)?;
            let end = midnight(tz, today.succ_opt().ok_or(EvalError::OutOfRange)?)?;
            let previous_start = midnight(tz, today.pred_opt().ok_or(EvalError::OutOfRange)?)?;
            Ok(((start, end), (previous_start, start)))
        }
        Period::Week => {
            let today = local_date(now, tz)?;
            let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
            let start = midnight(tz, monday)?;
            let end = midnight(tz, monday + Duration::days(7))?;
            let previous_start = midnight(tz, monday - Duration::days(7))?;
            Ok(((start, end), (previous_start, start)))
        }
        Period::Custom(custom) => {
            let length = i64::try_from(custom.length_seconds).map_err(|_| EvalError::OutOfRange)?;
            if length <= 0 {
                return Err(EvalError::OutOfRange);
            }
            let index = (now - custom.anchor).div_euclid(length);
            let start = custom.anchor + index * length;
            let end = start + length;
            Ok(((start, end), (start - length, start)))
        }
    }
}

/// The active spans covering `date`, in absolute seconds.
///
/// A wrapping hours span is one span that crosses midnight: for its start date
/// it runs from `from_minute` on that date to `to_minute` on the next.
fn spans_for_date(
    tz: Tz,
    active: &ActiveWindow,
    date: NaiveDate,
) -> Result<Vec<(i64, i64)>, EvalError> {
    match active {
        ActiveWindow::Always => Ok(vec![(
            midnight(tz, date)?,
            midnight(tz, date.succ_opt().ok_or(EvalError::OutOfRange)?)?,
        )]),
        ActiveWindow::Hours { hours, .. } => {
            let weekday = date.weekday().num_days_from_monday() as u8;
            let mut spans = Vec::new();
            for span in hours {
                if !day_included(span, weekday) {
                    continue;
                }
                if span.from_minute < span.to_minute {
                    spans.push((
                        at_minute(tz, date, span.from_minute)?,
                        at_minute(tz, date, span.to_minute)?,
                    ));
                } else {
                    spans.push((
                        at_minute(tz, date, span.from_minute)?,
                        at_minute(
                            tz,
                            date.succ_opt().ok_or(EvalError::OutOfRange)?,
                            span.to_minute,
                        )?,
                    ));
                }
            }
            Ok(spans)
        }
        ActiveWindow::SleepWindow {
            from_minute,
            to_minute,
            ..
        } => {
            // Active is the complement of the sleep window.
            if from_minute < to_minute {
                Ok(vec![
                    (midnight(tz, date)?, at_minute(tz, date, *from_minute)?),
                    (
                        at_minute(tz, date, *to_minute)?,
                        midnight(tz, date.succ_opt().ok_or(EvalError::OutOfRange)?)?,
                    ),
                ])
            } else {
                Ok(vec![(
                    at_minute(tz, date, *to_minute)?,
                    at_minute(tz, date, *from_minute)?,
                )])
            }
        }
    }
}

fn day_included(span: &HoursSpan, weekday: u8) -> bool {
    match &span.days {
        Some(days) => days.contains(&weekday),
        None => true,
    }
}

/// Every active span overlapping `[start, end)`, including the previous local
/// date so wrapping spans begun before the range can cover its leading edge.
fn active_spans_between(
    tz: Tz,
    active: &ActiveWindow,
    start: i64,
    end: i64,
) -> Result<Vec<(i64, i64)>, EvalError> {
    if matches!(active, ActiveWindow::Always) {
        return Ok(vec![(start, end)]);
    }
    let first = local_date(start, tz)?
        .pred_opt()
        .ok_or(EvalError::OutOfRange)?;
    let last = local_date(end.saturating_sub(1), tz)?;
    let mut spans = Vec::new();
    // A span that wraps past midnight began on the preceding local date, so
    // that date is scanned too: its carry-in portion overlaps this range.
    let mut date = first.pred_opt().unwrap_or(first);
    loop {
        spans.extend(spans_for_date(tz, active, date)?);
        if date >= last {
            break;
        }
        date = date.succ_opt().ok_or(EvalError::OutOfRange)?;
    }
    Ok(spans)
}

fn in_spans(at: i64, spans: &[(i64, i64)]) -> bool {
    spans.iter().any(|(start, end)| at >= *start && at < *end)
}

fn local_date(instant: i64, tz: Tz) -> Result<NaiveDate, EvalError> {
    Ok(local_datetime(instant)?.with_timezone(&tz).date_naive())
}

fn local_datetime(instant: i64) -> Result<DateTime<Utc>, EvalError> {
    Utc.timestamp_opt(instant, 0)
        .single()
        .ok_or(EvalError::OutOfRange)
}

fn midnight(tz: Tz, date: NaiveDate) -> Result<i64, EvalError> {
    at_minute(tz, date, 0)
}

fn at_minute(tz: Tz, date: NaiveDate, minute: u32) -> Result<i64, EvalError> {
    let days = i64::from(minute / 1440);
    let minute_of_day = minute % 1440;
    let date = date
        .checked_add_signed(Duration::days(days))
        .ok_or(EvalError::OutOfRange)?;
    let hour = minute_of_day / 60;
    let minute = minute_of_day % 60;
    let naive = date
        .and_hms_opt(hour, minute, 0)
        .ok_or(EvalError::OutOfRange)?;
    Ok(resolve_local(tz, naive))
}

/// Resolves a local wall-clock time to an instant. An ambiguous time takes the
/// earlier occurrence; a time inside a spring-forward gap moves one hour later.
fn resolve_local(tz: Tz, naive: NaiveDateTime) -> i64 {
    if let Some(datetime) = tz.from_local_datetime(&naive).earliest() {
        return datetime.timestamp();
    }
    let shifted = naive + Duration::hours(1);
    if let Some(datetime) = tz.from_local_datetime(&shifted).earliest() {
        return datetime.timestamp();
    }
    tz.from_utc_datetime(&naive).timestamp()
}
