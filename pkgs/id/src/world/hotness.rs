//! Sustained hotness evaluation over metrics snapshots (#2275, the
//! consumer half of the #2059 hot-world investigation, doc §2).
//!
//! The investigation's trigger discipline: act only on **sustained**
//! signals (windows of 3+ snapshots), never on a spike — worlds are
//! bursty by nature. Every rule here therefore looks at the whole
//! sample window: latency bands must hold in *every* sample, queue
//! growth must persist across the window, and the command rate is a
//! window average. One hot sample among healthy ones fires nothing.
//!
//! The queue signal samples the `actor_queue_depth` gauge trend, per
//! the #2275 refinement (comment 6532029493): `actor_queue_depth_max`
//! is a lifetime high-water mark (`fetch_max`, never reset), so it can
//! prove a burst happened but can never show growth or recovery inside
//! a window — it is deliberately never read here.

use serde::{Deserialize, Serialize};

use super::metrics::MetricsSnapshot;

/// How hot a world is, from the §2 bands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotnessBand {
    /// No sustained signal in the window.
    #[default]
    Healthy,
    /// A signal held in its warning band for the whole window.
    Warning,
    /// A signal held in its critical band for the whole window.
    Critical,
}

/// Which §2 signal fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotnessSignal {
    /// Sustained command rate vs the ceiling.
    CommandRate,
    /// Sampled actor queue depth growing across the window.
    QueueGrowth,
    /// Sustained command-latency p95.
    CommandLatency,
    /// Sustained journal-append-latency p95.
    JournalLatency,
}

/// One signal that fired, and the band it reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiredSignal {
    /// The signal that fired.
    pub signal: HotnessSignal,
    /// The band it reached.
    pub band: HotnessBand,
}

/// The outcome of evaluating one window of samples.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotnessReport {
    /// The worst band any signal reached.
    pub band: HotnessBand,
    /// Every signal that fired, worst first.
    pub fired: Vec<FiredSignal>,
    /// Samples in the evaluated window.
    pub samples: usize,
}

/// One snapshot reduced to the fields the evaluator reads, stamped with
/// the time it was taken (the rate rule needs real time between the
/// window's first and last samples).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotnessSample {
    /// When the snapshot was taken, unix milliseconds.
    pub at_ms: u64,
    /// `commands_total` at snapshot time.
    pub commands_total: u64,
    /// Sampled `actor_queue_depth` (the gauge, never the max).
    pub queue_depth: u64,
    /// Command-latency p95, when any command was observed.
    pub command_p95_us: Option<u64>,
    /// Journal-append-latency p95, when any append was observed.
    pub journal_p95_us: Option<u64>,
}

impl HotnessSample {
    /// Reduce a snapshot to a sample taken at `at_ms`.
    #[must_use]
    pub fn from_snapshot(at_ms: u64, snapshot: &MetricsSnapshot) -> Self {
        Self {
            at_ms,
            commands_total: snapshot.commands_total,
            queue_depth: snapshot.actor_queue_depth,
            command_p95_us: snapshot.command_latency.p95_us(),
            journal_p95_us: snapshot.journal_append_latency.p95_us(),
        }
    }
}

/// Band thresholds, from the #2059 investigation §2. The command-rate
/// ceiling is the one number the investigation leaves to the operator
/// (it is the actor's saturation rate for the deployment), so it is a
/// field with a documented default, not a constant buried in the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotnessThresholds {
    /// Command-rate ceiling, commands per minute (the §2 "soft cap").
    pub command_rate_ceiling_per_minute: u64,
    /// Command-latency p95 warning bound, microseconds (§2: 10 ms).
    pub command_latency_warning_us: u64,
    /// Command-latency p95 critical bound, microseconds (§2: 50 ms).
    pub command_latency_critical_us: u64,
    /// Journal-latency p95 warning bound, microseconds (§2: 2 ms).
    pub journal_latency_warning_us: u64,
    /// Journal-latency p95 critical bound, microseconds (§2: 10 ms).
    pub journal_latency_critical_us: u64,
    /// Minimum samples before any rule may fire (§2: 3+ snapshots).
    pub min_samples: usize,
}

impl Default for HotnessThresholds {
    fn default() -> Self {
        Self {
            command_rate_ceiling_per_minute: 6_000,
            command_latency_warning_us: 10_000,
            command_latency_critical_us: 50_000,
            journal_latency_warning_us: 2_000,
            journal_latency_critical_us: 10_000,
            min_samples: 3,
        }
    }
}

/// Evaluate one window of samples (oldest first) against the §2 bands.
#[must_use]
pub fn evaluate(samples: &[HotnessSample], thresholds: &HotnessThresholds) -> HotnessReport {
    let mut fired: Vec<FiredSignal> = Vec::new();
    if samples.len() >= thresholds.min_samples {
        if let Some(band) = queue_growth_band(samples) {
            fired.push(FiredSignal {
                signal: HotnessSignal::QueueGrowth,
                band,
            });
        }
        if let Some(band) = sustained_latency_band(
            samples,
            |sample| sample.command_p95_us,
            thresholds.command_latency_warning_us,
            thresholds.command_latency_critical_us,
        ) {
            fired.push(FiredSignal {
                signal: HotnessSignal::CommandLatency,
                band,
            });
        }
        if let Some(band) = sustained_latency_band(
            samples,
            |sample| sample.journal_p95_us,
            thresholds.journal_latency_warning_us,
            thresholds.journal_latency_critical_us,
        ) {
            fired.push(FiredSignal {
                signal: HotnessSignal::JournalLatency,
                band,
            });
        }
        if let Some(band) = command_rate_band(samples, thresholds) {
            fired.push(FiredSignal {
                signal: HotnessSignal::CommandRate,
                band,
            });
        }
    }
    fired.sort_by_key(|fired| std::cmp::Reverse(fired.band));
    HotnessReport {
        band: fired.first().map_or(HotnessBand::Healthy, |fired| fired.band),
        fired,
        samples: samples.len(),
    }
}

/// Queue growth (§2): monotonic growth over the window is critical;
/// any net growth across it is a warning. Flat or shrinking is healthy.
fn queue_growth_band(samples: &[HotnessSample]) -> Option<HotnessBand> {
    let first = samples.first()?.queue_depth;
    let last = samples.last()?.queue_depth;
    if samples
        .windows(2)
        .all(|pair| pair[1].queue_depth > pair[0].queue_depth)
    {
        return Some(HotnessBand::Critical);
    }
    if last > first {
        return Some(HotnessBand::Warning);
    }
    None
}

/// A latency band fires only when the p95 holds at or past the bound in
/// *every* sample — that is what makes it sustained rather than a spike.
fn sustained_latency_band(
    samples: &[HotnessSample],
    p95: impl Fn(&HotnessSample) -> Option<u64>,
    warning_us: u64,
    critical_us: u64,
) -> Option<HotnessBand> {
    let mut all_warning = true;
    let mut all_critical = true;
    for sample in samples {
        let Some(p95) = p95(sample) else {
            return None;
        };
        all_warning &= p95 >= warning_us;
        all_critical &= p95 >= critical_us;
    }
    if all_critical {
        Some(HotnessBand::Critical)
    } else if all_warning {
        Some(HotnessBand::Warning)
    } else {
        None
    }
}

/// Sustained command rate vs ceiling (§2): the window's average rate —
/// ≥ 70% of ceiling is critical, ≥ 40% is a warning.
fn command_rate_band(samples: &[HotnessSample], thresholds: &HotnessThresholds) -> Option<HotnessBand> {
    let first = samples.first()?;
    let last = samples.last()?;
    let elapsed_ms = last.at_ms.checked_sub(first.at_ms)?;
    if elapsed_ms == 0 || thresholds.command_rate_ceiling_per_minute == 0 {
        return None;
    }
    let commands = last.commands_total.saturating_sub(first.commands_total);
    // Commands per minute, scaled by 100 so the band comparison stays
    // in integers: rate_pct = commands * 60_000 * 100 / (elapsed * ceiling).
    let rate_pct = u128::from(commands) * 60_000 * 100
        / (u128::from(elapsed_ms) * u128::from(thresholds.command_rate_ceiling_per_minute));
    if rate_pct >= 70 {
        Some(HotnessBand::Critical)
    } else if rate_pct >= 40 {
        Some(HotnessBand::Warning)
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn sample(at_ms: u64, commands: u64, queue: u64, command_p95: Option<u64>, journal_p95: Option<u64>) -> HotnessSample {
        HotnessSample {
            at_ms,
            commands_total: commands,
            queue_depth: queue,
            command_p95_us: command_p95,
            journal_p95_us: journal_p95,
        }
    }

    #[test]
    fn sustained_critical_latency_triggers() {
        // Command p95 at 60 ms (> the 50 ms critical bound) in every
        // sample of the window: sustained critical.
        let samples: Vec<_> = (0..3)
            .map(|i| sample(i * 60_000, 100, 0, Some(60_000), Some(100)))
            .collect();
        let report = evaluate(&samples, &HotnessThresholds::default());
        assert_eq!(report.band, HotnessBand::Critical);
        assert!(report.fired.contains(&FiredSignal {
            signal: HotnessSignal::CommandLatency,
            band: HotnessBand::Critical,
        }));
    }

    #[test]
    fn a_spike_does_not_trigger() {
        // One 60 ms sample among healthy ones: the latency rule needs
        // the band in *every* sample, so nothing fires.
        let samples = vec![
            sample(0, 100, 0, Some(60_000), Some(100)),
            sample(60_000, 200, 0, Some(1_000), Some(100)),
            sample(120_000, 300, 0, Some(1_000), Some(100)),
        ];
        let report = evaluate(&samples, &HotnessThresholds::default());
        assert_eq!(report.band, HotnessBand::Healthy);
        assert!(report.fired.is_empty());
    }

    #[test]
    fn queue_growth_is_detected_from_the_gauge_trend() {
        // Monotonic growth across 3+ snapshots is critical (§2), even
        // though no single depth is large.
        let samples: Vec<_> = (0..4)
            .map(|i| sample(i * 60_000, 100, i + 1, Some(100), Some(100)))
            .collect();
        let report = evaluate(&samples, &HotnessThresholds::default());
        assert_eq!(report.band, HotnessBand::Critical);
        assert!(report.fired.contains(&FiredSignal {
            signal: HotnessSignal::QueueGrowth,
            band: HotnessBand::Critical,
        }));

        // Net growth without monotonic growth is a warning.
        let samples = vec![
            sample(0, 100, 1, Some(100), Some(100)),
            sample(60_000, 200, 5, Some(100), Some(100)),
            sample(120_000, 300, 3, Some(100), Some(100)),
        ];
        let report = evaluate(&samples, &HotnessThresholds::default());
        assert_eq!(report.band, HotnessBand::Warning);
        assert!(report.fired.contains(&FiredSignal {
            signal: HotnessSignal::QueueGrowth,
            band: HotnessBand::Warning,
        }));
    }

    #[test]
    fn sustained_command_rate_triggers() {
        // 800 commands/minute against a 1000/minute ceiling = 80%.
        let thresholds = HotnessThresholds {
            command_rate_ceiling_per_minute: 1_000,
            ..HotnessThresholds::default()
        };
        let samples = vec![
            sample(0, 0, 0, Some(100), Some(100)),
            sample(300_000, 4_000, 0, Some(100), Some(100)),
            sample(600_000, 8_000, 0, Some(100), Some(100)),
        ];
        let report = evaluate(&samples, &thresholds);
        assert_eq!(report.band, HotnessBand::Critical);
        assert!(report.fired.contains(&FiredSignal {
            signal: HotnessSignal::CommandRate,
            band: HotnessBand::Critical,
        }));
    }

    #[test]
    fn too_few_samples_never_fire() {
        let samples = vec![
            sample(0, 0, 1, Some(60_000), Some(60_000)),
            sample(60_000, 100, 9, Some(60_000), Some(60_000)),
        ];
        let report = evaluate(&samples, &HotnessThresholds::default());
        assert_eq!(report.band, HotnessBand::Healthy);
        assert!(report.fired.is_empty());
        assert_eq!(report.samples, 2);
    }
}
