//! Host metrics for one world (F11.7 slice 1, issue #553).
//!
//! A [`WorldMetrics`] is created with the world and shared between the
//! actor (command latency, journal append latency, checkpoint duration,
//! queue depth), the journal path, and the session layer (frame bytes).
//! Every field is an atomic or a fixed array of atomics: recording a
//! metric never allocates and never locks, so the hot paths stay cheap.
//! [`WorldMetrics::snapshot`] is what `id world stats` surfaces; the
//! Prometheus endpoint (slice 2) and tracing spans (slice 3) build on the
//! same registry later.
//!
//! Since #2275 the registry also carries a bounded write-locality table
//! (which regions writes concentrate in) and retains the last few
//! snapshot samples so each snapshot can carry the sustained hotness
//! evaluation from [`super::hotness`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Upper bounds of the latency histogram buckets, in microseconds. The
/// final bucket is the overflow (`> 1s`).
const BUCKET_BOUNDS_US: [u64; 7] = [100, 500, 2_000, 10_000, 50_000, 250_000, 1_000_000];
const BUCKETS: usize = BUCKET_BOUNDS_US.len() + 1;

/// A fixed-bucket latency histogram: bucket counts, total count, and the
/// sum of observed microseconds (so a mean is always derivable).
#[derive(Debug)]
struct Histogram {
    buckets: [AtomicU64; BUCKETS],
    count: AtomicU64,
    sum_us: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            sum_us: AtomicU64::new(0),
        }
    }
}

impl Histogram {
    fn observe(&self, duration: Duration) {
        let us = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
        let bucket = BUCKET_BOUNDS_US
            .iter()
            .position(|bound| us <= *bound)
            .unwrap_or(BUCKETS - 1);
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
    }

    fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            count: self.count.load(Ordering::Relaxed),
            sum_us: self.sum_us.load(Ordering::Relaxed),
            bucket_bounds_us: BUCKET_BOUNDS_US.to_vec(),
            buckets: self
                .buckets
                .iter()
                .map(|bucket| bucket.load(Ordering::Relaxed))
                .collect(),
        }
    }
}

/// One latency histogram at snapshot time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistogramSnapshot {
    /// Observations recorded.
    pub count: u64,
    /// Sum of the observed latencies, in microseconds.
    pub sum_us: u64,
    /// Upper bound of each bucket except the last (the overflow bucket).
    pub bucket_bounds_us: Vec<u64>,
    /// Per-bucket observation counts; one longer than `bucket_bounds_us`.
    pub buckets: Vec<u64>,
}

impl HistogramSnapshot {
    /// Approximate p95 latency in microseconds: the upper bound of the
    /// first bucket holding the 95th-percentile observation. The
    /// overflow bucket reports `u64::MAX` (past every bound). `None`
    /// when nothing was observed.
    #[must_use]
    pub fn p95_us(&self) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let target = self.count * 95 / 100 + 1;
        let mut cumulative = 0_u64;
        for (index, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket;
            if cumulative >= target {
                return Some(
                    self.bucket_bounds_us
                        .get(index)
                        .copied()
                        .unwrap_or(u64::MAX),
                );
            }
        }
        Some(u64::MAX)
    }
}

/// Writes recorded for one region (topic) of the world.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionWrites {
    /// FNV-1a hash of the region name, lowercase hex. The record path
    /// never stores or allocates the name itself; the hash is the
    /// stable identity consumers group by.
    pub region_hash: String,
    /// Journal writes recorded for the region.
    pub writes: u64,
}

/// The write-locality table at snapshot time (#2275): which regions
/// the world's writes concentrate in — the evidence the #2059 rung-1
/// (topic partition) vs rung-2 (region shard) decision reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteLocalitySnapshot {
    /// Regions with at least one recorded write, most writes first.
    pub regions: Vec<RegionWrites>,
    /// Writes whose region found no free slot (table full).
    pub overflow_writes: u64,
    /// All locality-recorded writes, regions and overflow together.
    pub total_writes: u64,
}

/// Number of region slots in the write-locality table. Bounded by
/// design: recording never allocates, so a world writing to more
/// distinct regions than this counts the rest as overflow.
const WRITE_LOCALITY_SLOTS: usize = 16;

/// One locality slot: the region hash it claimed (0 = unclaimed) and
/// its write count. Claimed with a single CAS on the record path.
#[derive(Debug, Default)]
struct LocalitySlot {
    region_hash: AtomicU64,
    writes: AtomicU64,
}

/// How many snapshot samples the hotness window retains.
const HOTNESS_WINDOW: usize = 8;

/// One retained snapshot sample for the hotness window. Plain atomics,
/// like every other field here: pushing a sample never allocates.
#[derive(Debug, Default)]
struct RetainedSample {
    seq: AtomicU64,
    at_ms: AtomicU64,
    commands_total: AtomicU64,
    queue_depth: AtomicU64,
    command_count: AtomicU64,
    command_p95_us: AtomicU64,
    journal_count: AtomicU64,
    journal_p95_us: AtomicU64,
}

/// FNV-1a hash of a region name; never 0 (0 marks an unclaimed slot).
fn region_hash(name: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if hash == 0 { 1 } else { hash }
}

/// All of one world's host metrics at snapshot time — the `metrics`
/// object in `id world stats` output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Actor commands handled since the world opened.
    pub commands_total: u64,
    /// Time spent handling actor commands.
    pub command_latency: HistogramSnapshot,
    /// Journal entries appended (each append fsyncs before returning).
    pub journal_appends_total: u64,
    /// Journal append latency, including the fsync.
    pub journal_append_latency: HistogramSnapshot,
    /// Journal checkpoints (compactions) completed.
    pub checkpoints_total: u64,
    /// Time spent checkpointing the journal.
    pub checkpoint_duration: HistogramSnapshot,
    /// Bytes of session frames sent for this world.
    pub frame_bytes_total: u64,
    /// Commands waiting in the actor's queue at the last dequeue.
    pub actor_queue_depth: u64,
    /// Deepest actor queue depth observed at a dequeue.
    pub actor_queue_depth_max: u64,
    /// Live sessions (joined participants) at snapshot time.
    pub sessions: u64,
    /// Write locality (#2275): per-region journal write counts.
    pub write_locality: WriteLocalitySnapshot,
    /// Sustained hotness over the retained snapshot window (#2275),
    /// evaluated by [`super::hotness`] at snapshot time.
    pub hotness: super::hotness::HotnessReport,
}

#[derive(Debug)]
struct Inner {
    commands_total: AtomicU64,
    command_latency: Histogram,
    journal_appends_total: AtomicU64,
    journal_append_latency: Histogram,
    checkpoints_total: AtomicU64,
    checkpoint_duration: Histogram,
    frame_bytes_total: AtomicU64,
    actor_queue_depth: AtomicU64,
    actor_queue_depth_max: AtomicU64,
    write_locality: [LocalitySlot; WRITE_LOCALITY_SLOTS],
    write_locality_overflow: AtomicU64,
    write_locality_total: AtomicU64,
    hotness_window: [RetainedSample; HOTNESS_WINDOW],
    hotness_seq: AtomicU64,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            commands_total: AtomicU64::new(0),
            command_latency: Histogram::default(),
            journal_appends_total: AtomicU64::new(0),
            journal_append_latency: Histogram::default(),
            checkpoints_total: AtomicU64::new(0),
            checkpoint_duration: Histogram::default(),
            frame_bytes_total: AtomicU64::new(0),
            actor_queue_depth: AtomicU64::new(0),
            actor_queue_depth_max: AtomicU64::new(0),
            write_locality: std::array::from_fn(|_| LocalitySlot::default()),
            write_locality_overflow: AtomicU64::new(0),
            write_locality_total: AtomicU64::new(0),
            hotness_window: std::array::from_fn(|_| RetainedSample::default()),
            hotness_seq: AtomicU64::new(0),
        }
    }
}

/// The shared metrics registry for one world. Cloning is cheap (an
/// `Arc`); every clone records into the same counters.
#[derive(Clone, Debug, Default)]
pub struct WorldMetrics {
    inner: Arc<Inner>,
}

impl WorldMetrics {
    /// Record one handled actor command.
    pub fn record_command(&self, latency: Duration) {
        self.inner.commands_total.fetch_add(1, Ordering::Relaxed);
        self.inner.command_latency.observe(latency);
    }

    /// Record one journal append (the append includes its fsync).
    pub fn record_journal_append(&self, latency: Duration) {
        self.inner
            .journal_appends_total
            .fetch_add(1, Ordering::Relaxed);
        self.inner.journal_append_latency.observe(latency);
    }

    /// Record one completed journal checkpoint.
    pub fn record_checkpoint(&self, duration: Duration) {
        self.inner.checkpoints_total.fetch_add(1, Ordering::Relaxed);
        self.inner.checkpoint_duration.observe(duration);
    }

    /// Record `bytes` of session frames sent for this world.
    pub fn add_frame_bytes(&self, bytes: u64) {
        self.inner
            .frame_bytes_total
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record the actor queue depth observed at a dequeue.
    pub fn record_queue_depth(&self, depth: u64) {
        self.inner.actor_queue_depth.store(depth, Ordering::Relaxed);
        self.inner
            .actor_queue_depth_max
            .fetch_max(depth, Ordering::Relaxed);
    }

    /// Record one journal write against `region` (a topic or entity
    /// reference — the write's locality). Bounded and lock-free: the
    /// region claims a free slot with one CAS or overflows; recording
    /// never allocates.
    pub fn record_write_locality(&self, region: &str) {
        let hash = region_hash(region);
        self.inner
            .write_locality_total
            .fetch_add(1, Ordering::Relaxed);
        let start = usize::try_from(hash % WRITE_LOCALITY_SLOTS as u64).unwrap_or(0);
        for offset in 0..WRITE_LOCALITY_SLOTS {
            let slot = &self.inner.write_locality[(start + offset) % WRITE_LOCALITY_SLOTS];
            let claimed = slot.region_hash.load(Ordering::Relaxed);
            if claimed == hash {
                slot.writes.fetch_add(1, Ordering::Relaxed);
                return;
            }
            if claimed == 0
                && slot
                    .region_hash
                    .compare_exchange(0, hash, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                slot.writes.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        self.inner
            .write_locality_overflow
            .fetch_add(1, Ordering::Relaxed);
    }

    fn write_locality_snapshot(&self) -> WriteLocalitySnapshot {
        let mut regions: Vec<RegionWrites> = self
            .inner
            .write_locality
            .iter()
            .filter_map(|slot| {
                let hash = slot.region_hash.load(Ordering::Relaxed);
                if hash == 0 {
                    None
                } else {
                    Some(RegionWrites {
                        region_hash: format!("{hash:016x}"),
                        writes: slot.writes.load(Ordering::Relaxed),
                    })
                }
            })
            .collect();
        regions.sort_by_key(|region| std::cmp::Reverse(region.writes));
        WriteLocalitySnapshot {
            regions,
            overflow_writes: self.inner.write_locality_overflow.load(Ordering::Relaxed),
            total_writes: self.inner.write_locality_total.load(Ordering::Relaxed),
        }
    }

    /// Snapshot every counter. `sessions` is the live session count,
    /// which only the actor knows (its presence map), so it is supplied
    /// at snapshot time rather than kept here.
    #[must_use]
    pub fn snapshot(&self, sessions: u64) -> MetricsSnapshot {
        let mut snapshot = MetricsSnapshot {
            commands_total: self.inner.commands_total.load(Ordering::Relaxed),
            command_latency: self.inner.command_latency.snapshot(),
            journal_appends_total: self.inner.journal_appends_total.load(Ordering::Relaxed),
            journal_append_latency: self.inner.journal_append_latency.snapshot(),
            checkpoints_total: self.inner.checkpoints_total.load(Ordering::Relaxed),
            checkpoint_duration: self.inner.checkpoint_duration.snapshot(),
            frame_bytes_total: self.inner.frame_bytes_total.load(Ordering::Relaxed),
            actor_queue_depth: self.inner.actor_queue_depth.load(Ordering::Relaxed),
            actor_queue_depth_max: self.inner.actor_queue_depth_max.load(Ordering::Relaxed),
            sessions,
            write_locality: self.write_locality_snapshot(),
            hotness: super::hotness::HotnessReport::default(),
        };
        snapshot.hotness = self.retain_and_evaluate(&snapshot);
        snapshot
    }

    /// Push this snapshot's sample into the retained window and
    /// evaluate the window's sustained hotness (#2275). Called by
    /// [`WorldMetrics::snapshot`]; the window is fixed-size atomics,
    /// so retaining never allocates either.
    fn retain_and_evaluate(&self, snapshot: &MetricsSnapshot) -> super::hotness::HotnessReport {
        use super::hotness::HotnessSample;
        let at_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_millis()),
        )
        .unwrap_or(u64::MAX);
        let sample = HotnessSample::from_snapshot(at_ms, snapshot);
        let seq = self.inner.hotness_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let slot = &self.inner.hotness_window[(seq as usize - 1) % HOTNESS_WINDOW];
        slot.at_ms.store(sample.at_ms, Ordering::Relaxed);
        slot.commands_total
            .store(sample.commands_total, Ordering::Relaxed);
        slot.queue_depth
            .store(sample.queue_depth, Ordering::Relaxed);
        slot.command_count
            .store(snapshot.command_latency.count, Ordering::Relaxed);
        slot.command_p95_us
            .store(sample.command_p95_us.unwrap_or(u64::MAX), Ordering::Relaxed);
        slot.journal_count
            .store(snapshot.journal_append_latency.count, Ordering::Relaxed);
        slot.journal_p95_us
            .store(sample.journal_p95_us.unwrap_or(u64::MAX), Ordering::Relaxed);
        // Publish last: readers treat a zero seq as an empty slot.
        slot.seq.store(seq, Ordering::Relaxed);

        let mut retained: Vec<(u64, HotnessSample)> = self
            .inner
            .hotness_window
            .iter()
            .filter_map(|slot| {
                let slot_seq = slot.seq.load(Ordering::Relaxed);
                if slot_seq == 0 {
                    return None;
                }
                let command_count = slot.command_count.load(Ordering::Relaxed);
                let journal_count = slot.journal_count.load(Ordering::Relaxed);
                Some((
                    slot_seq,
                    HotnessSample {
                        at_ms: slot.at_ms.load(Ordering::Relaxed),
                        commands_total: slot.commands_total.load(Ordering::Relaxed),
                        queue_depth: slot.queue_depth.load(Ordering::Relaxed),
                        command_p95_us: (command_count > 0)
                            .then(|| slot.command_p95_us.load(Ordering::Relaxed)),
                        journal_p95_us: (journal_count > 0)
                            .then(|| slot.journal_p95_us.load(Ordering::Relaxed)),
                    },
                ))
            })
            .collect();
        retained.sort_by_key(|(slot_seq, _)| *slot_seq);
        let samples: Vec<HotnessSample> = retained.into_iter().map(|(_, sample)| sample).collect();
        super::hotness::evaluate(&samples, &super::hotness::HotnessThresholds::default())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn histogram_buckets_and_snapshot_shape() {
        let metrics = WorldMetrics::default();
        metrics.record_command(Duration::from_micros(50));
        metrics.record_command(Duration::from_micros(700));
        metrics.record_command(Duration::from_secs(2));
        metrics.record_journal_append(Duration::from_micros(120));
        metrics.record_checkpoint(Duration::from_millis(3));
        metrics.add_frame_bytes(41);
        metrics.add_frame_bytes(1);
        metrics.record_queue_depth(3);
        metrics.record_queue_depth(1);

        let snapshot = metrics.snapshot(2);
        assert_eq!(snapshot.commands_total, 3);
        assert_eq!(snapshot.command_latency.count, 3);
        assert_eq!(snapshot.command_latency.sum_us, 50 + 700 + 2_000_000);
        assert_eq!(snapshot.command_latency.buckets.len(), BUCKETS);
        assert_eq!(snapshot.command_latency.buckets[0], 1, "50us in bucket 0");
        assert_eq!(snapshot.command_latency.buckets[2], 1, "700us in bucket 2");
        assert_eq!(
            snapshot.command_latency.buckets[BUCKETS - 1],
            1,
            "2s in the overflow bucket"
        );
        assert_eq!(snapshot.journal_appends_total, 1);
        assert_eq!(snapshot.journal_append_latency.count, 1);
        assert_eq!(snapshot.checkpoints_total, 1);
        assert_eq!(snapshot.checkpoint_duration.count, 1);
        assert_eq!(snapshot.frame_bytes_total, 42);
        assert_eq!(snapshot.actor_queue_depth, 1);
        assert_eq!(snapshot.actor_queue_depth_max, 3);
        assert_eq!(snapshot.sessions, 2);
        // The snapshot is the `id world stats` payload: it must serialize.
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["commands_total"], 3);
        assert_eq!(json["frame_bytes_total"], 42);
    }

    #[test]
    fn write_locality_counts_regions_and_overflows_when_full() {
        let metrics = WorldMetrics::default();
        for _ in 0..3 {
            metrics.record_write_locality("alpha");
        }
        metrics.record_write_locality("beta");
        let snapshot = metrics.snapshot(0);
        assert_eq!(snapshot.write_locality.total_writes, 4);
        assert_eq!(snapshot.write_locality.overflow_writes, 0);
        assert_eq!(snapshot.write_locality.regions.len(), 2);
        // Most writes first: alpha (3) ahead of beta (1).
        assert_eq!(snapshot.write_locality.regions[0].writes, 3);
        assert_eq!(
            snapshot.write_locality.regions[0].region_hash,
            format!("{:016x}", region_hash("alpha"))
        );
        assert_eq!(snapshot.write_locality.regions[1].writes, 1);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["write_locality"]["total_writes"], 4);

        // More distinct regions than slots: the table stays bounded
        // and the rest land in overflow.
        let metrics = WorldMetrics::default();
        for index in 0..(WRITE_LOCALITY_SLOTS + 4) {
            metrics.record_write_locality(&format!("region-{index}"));
        }
        let snapshot = metrics.snapshot(0);
        assert_eq!(
            snapshot.write_locality.total_writes,
            (WRITE_LOCALITY_SLOTS + 4) as u64
        );
        assert!(snapshot.write_locality.regions.len() <= WRITE_LOCALITY_SLOTS);
        assert!(snapshot.write_locality.overflow_writes > 0);
    }

    #[test]
    fn snapshot_hotness_uses_the_retained_window() {
        use super::super::hotness::HotnessBand;
        let metrics = WorldMetrics::default();
        // One snapshot is never enough to call a world hot.
        assert_eq!(metrics.snapshot(0).hotness.band, HotnessBand::Healthy);
        // Sustained queue growth across snapshots is critical (#2275):
        // each snapshot retains its sample, so the third sees growth.
        metrics.record_queue_depth(1);
        assert_eq!(metrics.snapshot(0).hotness.band, HotnessBand::Healthy);
        metrics.record_queue_depth(2);
        let snapshot = metrics.snapshot(0);
        assert_eq!(snapshot.hotness.band, HotnessBand::Critical);
        assert_eq!(snapshot.hotness.samples, 3);
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["hotness"]["band"], "critical");
    }
}
