//! lane_share — the daily lane-share tally for
//! developing-today/code#1657 (OpenCode-half step 4).
//!
//! The issue makes Drew's 2026-10-28 "at least half of fleet tasks
//! via OpenCode" order a published number: per queue and
//! fleet-wide, the rolling 7-day EXECUTED share
//! opencode / (opencode + direct), with exclusions reported
//! alongside (never silently dropped from the denominator view),
//! plus a below-50%-on-two-consecutive-days alert that fires
//! exactly once per streak.
//!
//! This module is the first slice: the pure tally over terminal
//! tasks joined to their `repo_task_lane_ledger` rows (the join
//! half lives in `turso_queue`, the daily production + readout
//! wiring are later slices). It depends only on `serde_json`, so
//! it compiles on every crate surface. Bucketing rules:
//!
//! * exclusion first: a row whose ledger `exclusion_reason` is
//!   non-blank counts as EXCLUDED (its reason is histogrammed)
//!   and never enters the share denominator — an excluded task
//!   still executed somewhere, but the issue counts it outside
//!   the split, visibly.
//! * executed_lane == "opencode" counts as OPENCODE, anything
//!   else counts as DIRECT. A terminal row with no executed lane
//!   and no exclusion still ran outside OpenCode attribution, so
//!   it counts DIRECT (conservative: it lowers the share) and is
//!   additionally counted as unattributed so the daily JSON can
//!   surface how much of `direct` is unattributed.
//! * terminal means done/cancelled/failed — the same three
//!   `turso_queue::core::TERMINAL_STATUSES` carries (that const is
//!   the source of truth; the mirror here keeps this module
//!   feature-gate-free).

use std::collections::BTreeMap;

use serde_json::Value;

/// The executed-lane mode value the ledger records for an
/// OpenCode execution (see `execution_lane_fields`: the mode
/// half of the (mode, lane_id) split).
pub const OPENCODE_MODE: &str = "opencode";
/// The executed-lane mode value for a non-OpenCode execution.
pub const DIRECT_MODE: &str = "direct";

/// Terminal task statuses. Mirror of
/// `turso_queue::core::TERMINAL_STATUSES`, kept local so this
/// module builds on every feature surface.
pub const TERMINAL_STATUSES: [&str; 3] = ["done", "cancelled", "failed"];

/// The rolling window (#1657: the 7-day executed share).
pub const ROLLING_WINDOW_DAYS: usize = 7;
/// The alert floor (#1657: below 50% ...).
pub const FLOOR_DEFAULT: f64 = 0.5;
/// Consecutive below-floor days that fire the alert (#1657: two).
pub const ALERT_STREAK_DAYS: usize = 2;

/// One terminal task's lane record: a `tasks` row in a terminal
/// status joined to its `repo_task_lane_ledger` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLaneRow {
    /// The queue the task completed in (ledger `queue`).
    pub queue: String,
    /// Completion day, YYYY-MM-DD UTC (from `completed_at`).
    pub day: String,
    /// Ledger `executed_lane`: `opencode` | `direct` (None when
    /// the row never recorded an execution lane).
    pub executed_lane: Option<String>,
    /// Ledger `exclusion_reason` (None/blank when not excluded).
    pub exclusion_reason: Option<String>,
    /// Ledger `fallback_reason` (None/blank when no fallback ran).
    pub fallback_reason: Option<String>,
}

impl TerminalLaneRow {
    /// Build from the broker's joined row JSON (`queue`,
    /// `status`, `completed_at` or `day`, plus the ledger's
    /// `executed_lane` / `exclusion_reason` / `fallback_reason`).
    /// Returns None when the row is not terminal or carries no
    /// queue/day — a non-terminal row is not a share input.
    pub fn from_joined_row(row: &Value) -> Option<Self> {
        let status = row.get("status")?.as_str()?;
        if !TERMINAL_STATUSES.contains(&status) {
            return None;
        }
        let queue = row.get("queue")?.as_str()?.to_string();
        let day = match row.get("completed_at").and_then(Value::as_str) {
            Some(ts) => ts.get(..10)?.to_string(),
            None => row.get("day")?.as_str()?.to_string(),
        };
        let text = |key: &str| non_blank(row.get(key).and_then(Value::as_str));
        Some(Self {
            queue,
            day,
            executed_lane: text("executed_lane"),
            exclusion_reason: text("exclusion_reason"),
            fallback_reason: text("fallback_reason"),
        })
    }
}

fn non_blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Per-scope counts. `unattributed_direct` is a subset of
/// `direct`: terminal rows with no executed lane and no
/// exclusion, counted direct (conservative) but flagged so the
/// daily JSON shows how much of `direct` is unattributed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LaneCounts {
    pub opencode: u64,
    pub direct: u64,
    pub excluded: u64,
    pub unattributed_direct: u64,
    pub exclusion_reasons: BTreeMap<String, u64>,
    pub fallback_reasons: BTreeMap<String, u64>,
}

impl LaneCounts {
    /// Every input row lands in exactly one bucket: the
    /// reconciliation invariant (#1657: 100% of terminal tasks
    /// accounted: opencode + direct + excluded).
    pub fn total(&self) -> u64 {
        self.opencode + self.direct + self.excluded
    }

    /// The executed share opencode / (opencode + direct), or
    /// None when nothing executed (all-excluded or empty days
    /// carry no share, not a zero).
    pub fn share(&self) -> Option<f64> {
        let executed = self.opencode + self.direct;
        (executed > 0).then(|| self.opencode as f64 / executed as f64)
    }

    /// Bucket one row per the module rules.
    pub fn record_row(&mut self, row: &TerminalLaneRow) {
        if let Some(reason) = &row.exclusion_reason {
            self.excluded += 1;
            *self.exclusion_reasons.entry(reason.clone()).or_insert(0) += 1;
        } else if row.executed_lane.as_deref() == Some(OPENCODE_MODE) {
            self.opencode += 1;
        } else {
            self.direct += 1;
            if row.executed_lane.is_none() {
                self.unattributed_direct += 1;
            }
        }
        if let Some(reason) = &row.fallback_reason {
            *self.fallback_reasons.entry(reason.clone()).or_insert(0) += 1;
        }
    }

    /// Add another scope's counts into this one.
    pub fn absorb(&mut self, other: &LaneCounts) {
        self.opencode += other.opencode;
        self.direct += other.direct;
        self.excluded += other.excluded;
        self.unattributed_direct += other.unattributed_direct;
        for (reason, n) in &other.exclusion_reasons {
            *self.exclusion_reasons.entry(reason.clone()).or_insert(0) += n;
        }
        for (reason, n) in &other.fallback_reasons {
            *self.fallback_reasons.entry(reason.clone()).or_insert(0) += n;
        }
    }
}

/// One day's tally: per-queue counts plus the fleet rollup.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DayTally {
    pub day: String,
    pub per_queue: BTreeMap<String, LaneCounts>,
    pub fleet: LaneCounts,
}

/// Tally one completion day's terminal rows.
pub fn tally_day(day: &str, rows: &[TerminalLaneRow]) -> DayTally {
    let mut tally = DayTally {
        day: day.to_string(),
        ..DayTally::default()
    };
    for row in rows {
        tally
            .per_queue
            .entry(row.queue.clone())
            .or_default()
            .record_row(row);
        tally.fleet.record_row(row);
    }
    tally
}

/// Sum counts across days (the trailing-window rollup the
/// rolling share divides).
pub fn roll_up(days: &[DayTally]) -> LaneCounts {
    let mut total = LaneCounts::default();
    for day in days {
        total.absorb(&day.fleet);
    }
    total
}

/// The rolling executed share over the given days: the summed
/// counts' share, not a mean of daily shares.
pub fn rolling_share(days: &[DayTally]) -> Option<f64> {
    roll_up(days).share()
}

/// The below-floor rule (#1657: below `floor` on `streak`
/// consecutive days fires exactly once per streak). Given the
/// day-ordered share series ending today, returns today when
/// today COMPLETES a fresh streak — the trailing `streak`
/// shares are all below the floor and the day before the
/// streak is not (or there is no day before). A longer streak
/// stays silent after its completion day, and a recovered then
/// re-sunk series fires again. Missing shares (None: nothing
/// executed) break a streak without firing.
pub fn streak_alert_day(
    days: &[(String, Option<f64>)],
    floor: f64,
    streak: usize,
) -> Option<String> {
    if streak == 0 || days.len() < streak {
        return None;
    }
    let below = |share: &Option<f64>| share.is_some_and(|s| s < floor);
    let start = days.len() - streak;
    if !days[start..].iter().all(|(_, share)| below(share)) {
        return None;
    }
    let completes_fresh = start == 0 || !below(&days[start - 1].1);
    completes_fresh.then(|| days[days.len() - 1].0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn row(queue: &str, day: &str, lane: Option<&str>) -> TerminalLaneRow {
        TerminalLaneRow {
            queue: queue.to_string(),
            day: day.to_string(),
            executed_lane: lane.map(str::to_string),
            exclusion_reason: None,
            fallback_reason: None,
        }
    }

    fn excluded(queue: &str, day: &str, reason: &str) -> TerminalLaneRow {
        TerminalLaneRow {
            exclusion_reason: Some(reason.to_string()),
            ..row(queue, day, Some(DIRECT_MODE))
        }
    }

    // Oracle: a hand-counted spot-check day, the way #1657 asks
    // for it ("reconciles with the ledger for a spot-checked
    // day"). Six terminal tasks on 2026-11-20:
    //   code: 3 opencode, 1 direct, 1 excluded(sink) -> share 3/4
    //   nix:  1 direct (no executed lane, unattributed)
    // Fleet: opencode 3, direct 2, excluded 1, total 6.
    #[test]
    fn spot_check_day_reconciles_by_hand() {
        let rows = vec![
            row("code", "2026-11-20", Some(OPENCODE_MODE)),
            row("code", "2026-11-20", Some(OPENCODE_MODE)),
            row("code", "2026-11-20", Some(OPENCODE_MODE)),
            row("code", "2026-11-20", Some(DIRECT_MODE)),
            excluded("code", "2026-11-20", "sink"),
            row("nix", "2026-11-20", None),
        ];
        let day = tally_day("2026-11-20", &rows);
        let code = &day.per_queue["code"];
        assert_eq!((code.opencode, code.direct, code.excluded), (3, 1, 1));
        assert_eq!(code.share(), Some(0.75));
        let nix = &day.per_queue["nix"];
        assert_eq!((nix.direct, nix.unattributed_direct), (1, 1));
        assert_eq!(day.fleet.total(), 6);
        assert_eq!(day.fleet.opencode, 3);
        assert_eq!(day.fleet.direct, 2);
        assert_eq!(day.fleet.excluded, 1);
        assert_eq!(day.fleet.exclusion_reasons["sink"], 1);
    }

    #[test]
    fn excluded_rows_stay_out_of_the_denominator() {
        let rows = vec![
            row("code", "2026-11-20", Some(OPENCODE_MODE)),
            excluded("code", "2026-11-20", "sink"),
            excluded("code", "2026-11-20", "sink"),
        ];
        let c = tally_day("2026-11-20", &rows).fleet;
        assert_eq!(c.share(), Some(1.0));
        assert_eq!(c.total(), 3);
    }

    #[test]
    fn an_all_excluded_scope_has_no_share_not_zero() {
        let c = tally_day("2026-11-20", &[excluded("code", "2026-11-20", "sink")]).fleet;
        assert_eq!(c.share(), None);
    }

    #[test]
    fn rolling_share_sums_counts_not_daily_shares() {
        // Day A: 1 opencode of 1 (share 1.0). Day B: 0 of 9 (0.0).
        // Mean of shares would be 0.5; the summed rollup is 1/10.
        let a = tally_day("2026-11-19", &[row("code", "2026-11-19", Some(OPENCODE_MODE))]);
        let b = tally_day(
            "2026-11-20",
            &(0..9)
                .map(|_| row("code", "2026-11-20", Some(DIRECT_MODE)))
                .collect::<Vec<_>>(),
        );
        assert_eq!(rolling_share(&[a, b]), Some(0.1));
    }

    #[test]
    fn non_terminal_rows_are_not_share_inputs() {
        let queued = json!({"queue": "code", "status": "queued", "completed_at": null,
                            "executed_lane": "opencode"});
        assert_eq!(TerminalLaneRow::from_joined_row(&queued), None);
        let done = json!({"queue": "code", "status": "done",
                          "completed_at": "2026-11-20T10:00:00Z",
                          "executed_lane": "opencode"});
        let parsed = TerminalLaneRow::from_joined_row(&done).expect("done row parses");
        assert_eq!(parsed.day, "2026-11-20");
        assert_eq!(parsed.executed_lane.as_deref(), Some("opencode"));
    }

    #[test]
    fn blank_exclusion_reason_is_not_an_exclusion() {
        let done = json!({"queue": "code", "status": "failed",
                          "completed_at": "2026-11-20T10:00:00Z",
                          "executed_lane": "direct", "exclusion_reason": "  "});
        let parsed = TerminalLaneRow::from_joined_row(&done).expect("failed row parses");
        let c = tally_day("2026-11-20", &[parsed]).fleet;
        assert_eq!((c.direct, c.excluded), (1, 0));
    }

    // The streak rule (#1657): two consecutive below-floor days
    // fire exactly once; a longer streak stays silent; recovery
    // re-arms it.
    fn series(vals: &[Option<f64>]) -> Vec<(String, Option<f64>)> {
        vals.iter()
            .enumerate()
            .map(|(i, v)| (format!("2026-11-{:02}", i + 1), *v))
            .collect()
    }

    #[test]
    fn two_day_below_floor_streak_fires_exactly_once() {
        let s = series(&[Some(0.9), Some(0.3), Some(0.4)]);
        assert_eq!(
            streak_alert_day(&s[..2], FLOOR_DEFAULT, ALERT_STREAK_DAYS),
            None,
            "one below-floor day is not a streak"
        );
        assert_eq!(
            streak_alert_day(&s[..3], FLOOR_DEFAULT, ALERT_STREAK_DAYS),
            Some("2026-11-03".to_string()),
            "the second consecutive day fires"
        );
        let s4 = series(&[Some(0.9), Some(0.3), Some(0.4), Some(0.2)]);
        assert_eq!(
            streak_alert_day(&s4, FLOOR_DEFAULT, ALERT_STREAK_DAYS),
            None,
            "a third below-floor day stays silent"
        );
    }

    #[test]
    fn recovery_re_arms_the_alert() {
        let s = series(&[Some(0.3), Some(0.3), Some(0.8), Some(0.2), Some(0.2)]);
        assert_eq!(
            streak_alert_day(&s[..2], FLOOR_DEFAULT, ALERT_STREAK_DAYS),
            Some("2026-11-02".to_string())
        );
        assert_eq!(streak_alert_day(&s[..4], FLOOR_DEFAULT, ALERT_STREAK_DAYS), None);
        assert_eq!(
            streak_alert_day(&s, FLOOR_DEFAULT, ALERT_STREAK_DAYS),
            Some("2026-11-05".to_string())
        );
    }

    #[test]
    fn a_no_execution_day_breaks_a_streak_without_firing() {
        let s = series(&[Some(0.3), None, Some(0.3)]);
        assert_eq!(streak_alert_day(&s, FLOOR_DEFAULT, ALERT_STREAK_DAYS), None);
    }

    proptest! {
        // Oracle: the ledger's own count. Every terminal row lands in
        // exactly one bucket, so the fleet total is the row count.
        #[test]
        fn every_row_is_accounted_exactly_once(
            lanes in proptest::collection::vec(0u8..4, 0..60),
        ) {
            let rows: Vec<TerminalLaneRow> = lanes
                .iter()
                .map(|k| match k {
                    0 => row("code", "2026-11-20", Some(OPENCODE_MODE)),
                    1 => row("code", "2026-11-20", Some(DIRECT_MODE)),
                    2 => excluded("code", "2026-11-20", "sink"),
                    _ => row("nix", "2026-11-20", None),
                })
                .collect();
            let day = tally_day("2026-11-20", &rows);
            prop_assert_eq!(day.fleet.total(), rows.len() as u64);
            let per_queue_total: u64 = day.per_queue.values().map(|c| c.total()).sum();
            prop_assert_eq!(per_queue_total, rows.len() as u64);
        }

        #[test]
        fn share_is_bounded_when_present(
            lanes in proptest::collection::vec(0u8..3, 1..60),
        ) {
            let rows: Vec<TerminalLaneRow> = lanes
                .iter()
                .map(|k| match k {
                    0 => row("code", "2026-11-20", Some(OPENCODE_MODE)),
                    1 => row("code", "2026-11-20", Some(DIRECT_MODE)),
                    _ => excluded("code", "2026-11-20", "sink"),
                })
                .collect();
            if let Some(s) = tally_day("2026-11-20", &rows).fleet.share() {
                prop_assert!((0.0..=1.0).contains(&s));
            }
        }
    }
}
