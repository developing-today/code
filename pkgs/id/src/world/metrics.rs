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
}

#[derive(Debug, Default)]
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

    /// Snapshot every counter. `sessions` is the live session count,
    /// which only the actor knows (its presence map), so it is supplied
    /// at snapshot time rather than kept here.
    #[must_use]
    pub fn snapshot(&self, sessions: u64) -> MetricsSnapshot {
        MetricsSnapshot {
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
        }
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
}
