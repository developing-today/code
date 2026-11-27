//! Lane-share tally for the daily lane-share measurement (#1657,
//! OpenCode-half step 4; the assignment ledger it reads is #1654).
//!
//! This module is the pure core of the measurement: it takes
//! assignment-ledger / terminal-result rows and produces the counts
//! the daily `lane-share.json` will publish. It does no I/O — a later
//! slice wires it to the ledger, the JSON file, the morning readout
//! and the alert surfaces.
//!
//! Oracle / invariant: every terminal row is accounted for exactly
//! once — `opencode + direct + excluded == total` — for every day,
//! every queue, and the fleet as a whole. The executed share is
//! `opencode / (opencode + direct)`; excluded rows never enter the
//! share denominator, but they are always tallied and reported
//! alongside it, broken out by reason, never silently dropped.
//! Fallbacks (a task that fell back before executing) are likewise
//! broken out by reason; a fallback row still counts toward the lane
//! that ultimately executed it.

use std::collections::BTreeMap;

/// The share floor: a rolling executed share strictly below this is
/// "below floor" for the alert rule.
pub const SHARE_FLOOR: f64 = 0.5;

/// Rolling window length, in days, for the executed share.
pub const WINDOW_DAYS: i64 = 7;

/// Consecutive below-floor days required before the alert fires.
pub const ALERT_STREAK_DAYS: u32 = 2;

/// Exclusion reason recorded for a terminal row that carries neither
/// an executed lane nor an exclusion reason, so the reconciliation
/// invariant holds for every input row instead of dropping it.
pub const UNRECORDED_EXCLUSION: &str = "unrecorded";

/// The lane that executed a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecutedLane {
    Opencode,
    Direct,
}

/// One assignment-ledger / terminal-result row: a task that reached a
/// terminal state on `completed_on` (an ISO `YYYY-MM-DD` date) in
/// `queue`.
///
/// A row is *excluded* when `exclusion_reason` is set (the task never
/// executed, or is out of scope for the share); exclusion takes
/// precedence over `executed_lane` if both are somehow present. An
/// executed row carries `executed_lane` and no exclusion reason.
/// `fallback_reason` may accompany an executed row when the task fell
/// back before the lane that executed it picked it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    pub queue: String,
    pub completed_on: String,
    pub executed_lane: Option<ExecutedLane>,
    pub exclusion_reason: Option<String>,
    pub fallback_reason: Option<String>,
}

impl LedgerRow {
    /// An executed terminal row.
    pub fn executed(queue: &str, completed_on: &str, lane: ExecutedLane) -> Self {
        Self {
            queue: queue.to_string(),
            completed_on: completed_on.to_string(),
            executed_lane: Some(lane),
            exclusion_reason: None,
            fallback_reason: None,
        }
    }

    /// An excluded terminal row, with its reason.
    pub fn excluded(queue: &str, completed_on: &str, reason: &str) -> Self {
        Self {
            queue: queue.to_string(),
            completed_on: completed_on.to_string(),
            executed_lane: None,
            exclusion_reason: Some(reason.to_string()),
            fallback_reason: None,
        }
    }

    /// Attach a fallback reason to an executed row.
    pub fn with_fallback(mut self, reason: &str) -> Self {
        self.fallback_reason = Some(reason.to_string());
        self
    }

    fn is_excluded(&self) -> bool {
        self.exclusion_reason.is_some() || self.executed_lane.is_none()
    }

    fn exclusion_key(&self) -> &str {
        self.exclusion_reason
            .as_deref()
            .unwrap_or(UNRECORDED_EXCLUSION)
    }
}

/// Counts over a set of terminal rows, with the reconciliation
/// invariant `opencode + direct + excluded == total`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tally {
    pub total: u64,
    pub opencode: u64,
    pub direct: u64,
    pub excluded: u64,
    pub exclusions_by_reason: BTreeMap<String, u64>,
    pub fallbacks_by_reason: BTreeMap<String, u64>,
}

impl Tally {
    pub fn add(&mut self, row: &LedgerRow) {
        self.total += 1;
        if let Some(reason) = &row.fallback_reason {
            *self.fallbacks_by_reason.entry(reason.clone()).or_default() += 1;
        }
        if row.is_excluded() {
            self.excluded += 1;
            *self
                .exclusions_by_reason
                .entry(row.exclusion_key().to_string())
                .or_default() += 1;
            return;
        }
        match row.executed_lane {
            Some(ExecutedLane::Opencode) => self.opencode += 1,
            Some(ExecutedLane::Direct) => self.direct += 1,
            None => unreachable!("non-excluded rows carry an executed lane"),
        }
    }

    fn merge(&mut self, other: &Tally) {
        self.total += other.total;
        self.opencode += other.opencode;
        self.direct += other.direct;
        self.excluded += other.excluded;
        for (k, v) in &other.exclusions_by_reason {
            *self.exclusions_by_reason.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.fallbacks_by_reason {
            *self.fallbacks_by_reason.entry(k.clone()).or_default() += v;
        }
    }

    /// The reconciliation invariant: 100% of terminal rows accounted.
    pub fn reconciles(&self) -> bool {
        self.opencode + self.direct + self.excluded == self.total
    }

    /// Executed share `opencode / (opencode + direct)`; `None` when
    /// nothing executed (the denominator excludes excluded rows).
    pub fn executed_share(&self) -> Option<f64> {
        let denom = self.opencode + self.direct;
        if denom == 0 {
            None
        } else {
            Some(self.opencode as f64 / denom as f64)
        }
    }
}

/// One day's tally: fleet-wide plus a per-queue breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct DayTally {
    pub date: String,
    pub fleet: Tally,
    pub per_queue: BTreeMap<String, Tally>,
}

/// Tally rows per completion day, sorted by date. Every day's fleet
/// tally and every per-queue tally reconcile by construction.
pub fn tally_by_day(rows: &[LedgerRow]) -> Vec<DayTally> {
    let mut days: BTreeMap<String, DayTally> = BTreeMap::new();
    for row in rows {
        let day = days
            .entry(row.completed_on.clone())
            .or_insert_with(|| DayTally {
                date: row.completed_on.clone(),
                fleet: Tally::default(),
                per_queue: BTreeMap::new(),
            });
        day.fleet.add(row);
        day.per_queue.entry(row.queue.clone()).or_default().add(row);
    }
    days.into_values().collect()
}

/// The rolling executed share ending on one day (the window is that
/// day plus the previous `WINDOW_DAYS - 1` days), per queue and
/// fleet-wide, with the window tallies the shares came from.
#[derive(Debug, Clone, PartialEq)]
pub struct RollingShare {
    pub date: String,
    pub fleet: Tally,
    pub per_queue: BTreeMap<String, Tally>,
}

impl RollingShare {
    pub fn fleet_share(&self) -> Option<f64> {
        self.fleet.executed_share()
    }

    pub fn queue_share(&self, queue: &str) -> Option<f64> {
        self.per_queue.get(queue).and_then(Tally::executed_share)
    }
}

/// Rolling `WINDOW_DAYS`-day shares for every completion day present
/// in `rows`, sorted by date.
pub fn rolling_shares(rows: &[LedgerRow]) -> Vec<RollingShare> {
    let days = tally_by_day(rows);
    let mut out = Vec::with_capacity(days.len());
    for (i, day) in days.iter().enumerate() {
        let mut fleet = Tally::default();
        let mut per_queue: BTreeMap<String, Tally> = BTreeMap::new();
        for prior in &days[..=i] {
            if !in_window(&day.date, &prior.date) {
                continue;
            }
            fleet.merge(&prior.fleet);
            for (queue, tally) in &prior.per_queue {
                per_queue.entry(queue.clone()).or_default().merge(tally);
            }
        }
        out.push(RollingShare {
            date: day.date.clone(),
            fleet,
            per_queue,
        });
    }
    out
}

/// The below-floor alert decision over a chronological series of
/// daily rolling shares: the alert fires exactly once per streak, on
/// the day the streak of consecutive below-floor days reaches
/// `ALERT_STREAK_DAYS`. A day at or above the floor, a day with no
/// executed tasks (share `None`), or a calendar gap resets the
/// streak. Returns the dates on which the alert fires.
pub fn alert_fire_dates(series: &[RollingShare]) -> Vec<String> {
    let mut fires = Vec::new();
    let mut streak: u32 = 0;
    let mut prev_day: Option<i64> = None;
    for day in series {
        let day_num = day_number(&day.date);
        let consecutive = match (day_num, prev_day) {
            (Some(cur), Some(prev)) => cur == prev + 1,
            _ => false,
        };
        if !consecutive {
            streak = 0;
        }
        prev_day = day_num;
        match day.fleet_share() {
            Some(share) if share < SHARE_FLOOR => {
                streak += 1;
                if streak == ALERT_STREAK_DAYS {
                    fires.push(day.date.clone());
                }
            }
            _ => streak = 0,
        }
    }
    fires
}

fn in_window(end: &str, date: &str) -> bool {
    match (day_number(end), day_number(date)) {
        (Some(e), Some(d)) => d <= e && e - d < WINDOW_DAYS,
        _ => end == date,
    }
}

/// Days since the civil epoch for an ISO `YYYY-MM-DD` date
/// (Howard Hinnant's `days_from_civil`); `None` if unparseable.
fn day_number(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oc(queue: &str, date: &str) -> LedgerRow {
        LedgerRow::executed(queue, date, ExecutedLane::Opencode)
    }

    fn direct(queue: &str, date: &str) -> LedgerRow {
        LedgerRow::executed(queue, date, ExecutedLane::Direct)
    }

    // Oracle: the reconciliation invariant — opencode + direct +
    // excluded == total for every day, queue, and the fleet.
    #[test]
    fn lane_share_tally_reconciles_every_day_and_queue() {
        let rows = vec![
            oc("code", "2026-11-20"),
            direct("code", "2026-11-20"),
            LedgerRow::excluded("code", "2026-11-20", "not-a-task"),
            oc("nix", "2026-11-20"),
            LedgerRow::excluded("nix", "2026-11-21", "duplicate"),
            direct("nix", "2026-11-21"),
            // Neither lane nor reason: still accounted, as `unrecorded`.
            LedgerRow {
                queue: "code".into(),
                completed_on: "2026-11-21".into(),
                executed_lane: None,
                exclusion_reason: None,
                fallback_reason: None,
            },
        ];
        let days = tally_by_day(&rows);
        assert_eq!(days.len(), 2);
        let total: u64 = days.iter().map(|d| d.fleet.total).sum();
        assert_eq!(total, rows.len() as u64);
        for day in &days {
            assert!(day.fleet.reconciles(), "fleet tally for {}", day.date);
            for (queue, tally) in &day.per_queue {
                assert!(tally.reconciles(), "{queue} tally for {}", day.date);
            }
        }
        assert_eq!(days[0].fleet.opencode, 2);
        assert_eq!(days[0].fleet.direct, 1);
        assert_eq!(days[0].fleet.excluded, 1);
        assert_eq!(days[1].fleet.excluded, 2);
        assert_eq!(days[1].fleet.exclusions_by_reason[UNRECORDED_EXCLUSION], 1);
    }

    // Oracle: hand-computed reason breakdowns over a fixed row set.
    #[test]
    fn lane_share_breaks_out_exclusions_and_fallbacks_by_reason() {
        let rows = vec![
            oc("code", "2026-11-20").with_fallback("opencode-busy"),
            oc("code", "2026-11-20").with_fallback("opencode-busy"),
            direct("code", "2026-11-20").with_fallback("timeout"),
            LedgerRow::excluded("code", "2026-11-20", "cancelled"),
            LedgerRow::excluded("code", "2026-11-20", "cancelled"),
            LedgerRow::excluded("code", "2026-11-20", "not-a-task"),
        ];
        let days = tally_by_day(&rows);
        let fleet = &days[0].fleet;
        assert_eq!(fleet.exclusions_by_reason["cancelled"], 2);
        assert_eq!(fleet.exclusions_by_reason["not-a-task"], 1);
        assert_eq!(fleet.fallbacks_by_reason["opencode-busy"], 2);
        assert_eq!(fleet.fallbacks_by_reason["timeout"], 1);
        assert!(fleet.reconciles());
    }

    // Oracle: share = opencode / (opencode + direct) computed by
    // hand; excluded rows are reported but never in the denominator.
    #[test]
    fn lane_share_executed_share_excludes_excluded_from_denominator() {
        let rows = vec![
            oc("code", "2026-11-20"),
            oc("code", "2026-11-20"),
            oc("code", "2026-11-20"),
            direct("code", "2026-11-20"),
            LedgerRow::excluded("code", "2026-11-20", "cancelled"),
            LedgerRow::excluded("code", "2026-11-20", "cancelled"),
        ];
        let days = tally_by_day(&rows);
        assert_eq!(days[0].fleet.executed_share(), Some(0.75));
        assert_eq!(days[0].fleet.excluded, 2);

        let only_excluded = vec![LedgerRow::excluded("code", "2026-11-20", "x")];
        assert_eq!(tally_by_day(&only_excluded)[0].fleet.executed_share(), None);
    }

    // Oracle: a hand-built 8-day series — the window ending on day 8
    // drops day 1, so its share is computed from days 2..=8 only.
    #[test]
    fn lane_share_rolling_window_is_seven_days_per_queue_and_fleet() {
        let mut rows = vec![oc("code", "2026-11-01")]; // day 1: opencode only
        for date in [
            "2026-11-02",
            "2026-11-03",
            "2026-11-04",
            "2026-11-05",
            "2026-11-06",
            "2026-11-07",
            "2026-11-08",
        ] {
            rows.push(direct("code", date)); // days 2..=8: direct only
            rows.push(oc("nix", date));
        }
        let series = rolling_shares(&rows);
        assert_eq!(series.len(), 8);
        // Day 7 window covers days 1..=7: 1 opencode, 6 direct (code)
        // plus 6 opencode (nix) → fleet 7/13.
        assert_eq!(series[6].fleet_share(), Some(7.0 / 13.0));
        // Day 8 window drops day 1: code 0/7, nix 7/7, fleet 7/14.
        let day8 = &series[7];
        assert_eq!(day8.queue_share("code"), Some(0.0));
        assert_eq!(day8.queue_share("nix"), Some(1.0));
        assert_eq!(day8.fleet_share(), Some(0.5));
        assert!(day8.fleet.reconciles());
    }

    // Oracle: the alert rule stated in #1657 — below floor on two
    // consecutive days fires exactly once, on the day the streak
    // reaches 2; a longer streak does not refire.
    #[test]
    fn lane_share_alert_fires_once_on_second_consecutive_below_floor_day() {
        // Direct-only days: rolling share is 0.0 from day 1 on.
        let rows: Vec<LedgerRow> = ["2026-11-01", "2026-11-02", "2026-11-03", "2026-11-04"]
            .iter()
            .map(|d| direct("code", d))
            .collect();
        let fires = alert_fire_dates(&rolling_shares(&rows));
        assert_eq!(fires, vec!["2026-11-02".to_string()]);
    }

    // Oracle: the same rule — recovery to at/above the floor resets
    // the streak, and a later streak fires again, once.
    #[test]
    fn lane_share_alert_resets_after_recovery_and_can_fire_again() {
        let mut rows = Vec::new();
        // Days 1-2 below floor (direct only) → fire on day 2.
        rows.push(direct("code", "2026-11-01"));
        rows.push(direct("code", "2026-11-02"));
        // Days 3-9: heavy opencode so the 7-day window recovers to
        // >= 50% by day 9 (window days 3..=9: 70 opencode, 0 direct).
        for date in [
            "2026-11-03",
            "2026-11-04",
            "2026-11-05",
            "2026-11-06",
            "2026-11-07",
            "2026-11-08",
            "2026-11-09",
        ] {
            for _ in 0..10 {
                rows.push(oc("code", date));
            }
        }
        // Days 10-16: heavy direct; the window ending day 16 covers
        // days 10..=16 only, so the share is back below floor well
        // before then and a new streak fires exactly once more.
        for date in [
            "2026-11-10",
            "2026-11-11",
            "2026-11-12",
            "2026-11-13",
            "2026-11-14",
            "2026-11-15",
            "2026-11-16",
        ] {
            for _ in 0..10 {
                rows.push(direct("code", date));
            }
        }
        let series = rolling_shares(&rows);
        let fires = alert_fire_dates(&series);
        assert_eq!(fires.first().map(String::as_str), Some("2026-11-02"));
        assert_eq!(fires.len(), 2, "one fire per streak, got {fires:?}");
    }

    // Oracle: a single below-floor day, and a streak broken by a
    // calendar gap, never fire.
    #[test]
    fn lane_share_alert_needs_two_consecutive_days() {
        let single = vec![direct("code", "2026-11-01")];
        assert!(alert_fire_dates(&rolling_shares(&single)).is_empty());

        let gapped = vec![
            direct("code", "2026-11-01"),
            direct("code", "2026-11-03"), // 2026-11-02 missing: gap resets
        ];
        assert!(alert_fire_dates(&rolling_shares(&gapped)).is_empty());
    }
}
