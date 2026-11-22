//! Database-leg routing for the Fleet verb contract (#8581, #8583).
//!
//! Pure contract: given which legs are reachable, decide which route a
//! coordination verb (claim, lease, heartbeat, checkpoint) takes and
//! which mode it reports. No I/O happens here and no route is
//! attempted; adapters call [`route_for`] and then serve the verb on
//! the route it names. The #8583 Turso embedded replica implements
//! [`CoordinationReplica`]; this module only fixes the interface it
//! must satisfy and the order in which routes are tried, so the
//! replica can be built against a settled boundary instead of an
//! invented one.
//!
//! Route order, decided here per FDR-0013 (the #12601 contract left
//! multi-leg precedence open): the cloud database first; when it is
//! unreachable, the local replica; when both are unreachable, the
//! GitHub-marker fallback. For coordination verbs the database leg
//! dominates the GitHub legs: a verb that routes through the replica
//! reports `db-degraded` even when a GitHub leg is also down, because
//! the route actually selected is the database-degraded one and any
//! GitHub outcome is buffered within it. A verb whose database is up
//! reports the GitHub leg that constrains it, writes before reads (a
//! failed write blocks durable outcomes; a failed read only degrades
//! reads).

use std::collections::HashMap;

use super::degraded::{Mode, ModeLine};

/// A route a coordination verb can be served on, in try order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbRoute {
    /// The cloud coordination database (normal operation).
    Cloud,
    /// The local embedded replica on the laptop (#8583).
    LocalReplica,
    /// The GitHub-marker fallback, last resort behind the replica.
    GitHubMarkers,
}

impl DbRoute {
    /// Stable machine-readable spelling, shared with [`ModeLine::route`].
    pub fn label(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::LocalReplica => "local-replica",
            Self::GitHubMarkers => "github-markers",
        }
    }
}

/// The ordered route list, first tried to last resort.
pub const ROUTE_ORDER: [DbRoute; 3] = [
    DbRoute::Cloud,
    DbRoute::LocalReplica,
    DbRoute::GitHubMarkers,
];

/// Which legs a verb can currently reach. Adapters fill this in from
/// their probes; the selection below is a pure function of it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LegAvailability {
    /// The cloud coordination database answers.
    pub database: bool,
    /// The local embedded replica answers.
    pub replica: bool,
    /// GitHub writes succeed.
    pub github_write: bool,
    /// GitHub reads succeed.
    pub github_read: bool,
}

impl LegAvailability {
    /// All legs up: normal operation.
    pub fn all_up() -> Self {
        Self {
            database: true,
            replica: true,
            github_write: true,
            github_read: true,
        }
    }
}

/// Select the mode and route for a coordination verb from leg availability.
///
/// - Database up: route [`DbRoute::Cloud`]; the mode names the GitHub
///   leg that constrains the verb, writes before reads, else
///   [`Mode::Normal`].
/// - Database down, replica up: [`Mode::DbDegraded`] on
///   [`DbRoute::LocalReplica`], whatever the GitHub legs say.
/// - Both down: [`Mode::DbDegraded`] on [`DbRoute::GitHubMarkers`].
pub fn route_for(legs: LegAvailability) -> (Mode, DbRoute) {
    if legs.database {
        let mode = if !legs.github_write {
            Mode::GhWriteDegraded
        } else if !legs.github_read {
            Mode::GhReadDegraded
        } else {
            Mode::Normal
        };
        (mode, DbRoute::Cloud)
    } else if legs.replica {
        (Mode::DbDegraded, DbRoute::LocalReplica)
    } else {
        (Mode::DbDegraded, DbRoute::GitHubMarkers)
    }
}

/// Build the machine-readable mode line a routed verb reports: the verb,
/// the mode it actually operated in, and the route it actually used.
pub fn mode_line<'a>(verb: &'a str, legs: LegAvailability) -> ModeLine<'a> {
    let (mode, route) = route_for(legs);
    ModeLine {
        verb,
        mode,
        route: route.label(),
    }
}

/// A claim held by a coordination store (cloud or replica): the task,
/// who holds it, and the lease granted, in seconds. The adapter
/// computes expiry from its own clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaClaim {
    pub task_id: i64,
    pub worker: String,
    pub lane: String,
    pub lease_seconds: i64,
}

/// One claim the replica honored locally while the cloud held the same
/// task: surfaced by reconciliation, never silently merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoubleClaim {
    pub task_id: i64,
    pub local_worker: String,
    pub cloud_worker: String,
}

/// What a replica sync reports: every local claim reconciled, the
/// claims lost (none expected), and any partition double-claims.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Local claims now honored globally.
    pub synced_claims: Vec<i64>,
    /// Local claims that did not survive the sync; must stay empty.
    pub lost_claims: Vec<i64>,
    /// Tasks claimed on both sides of the partition.
    pub double_claims: Vec<DoubleClaim>,
}

impl ReconcileReport {
    /// The #8583 acceptance bar: zero lost claims.
    pub fn zero_lost(&self) -> bool {
        self.lost_claims.is_empty()
    }
}

/// Failure of one replica operation. `Unavailable` is the only
/// transport-shaped failure; the rest are coordination answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaError {
    /// The task is not claimable (already held, or not queued).
    NotClaimable(String),
    /// The caller does not hold this task's lease.
    NotHeld(String),
    /// The replica itself could not be reached.
    Unavailable(String),
}

/// The minimal interface a local coordination replica must satisfy so
/// claims, leases, and heartbeats continue while the cloud database
/// is unreachable (#8583). The Turso embedded replica implements this
/// trait; adapters serve verbs through it when [`route_for`] selects
/// [`DbRoute::LocalReplica`].
pub trait CoordinationReplica {
    /// Claim a queued task for `worker` on `lane` for `lease_seconds`.
    /// A second claim on a held task fails with
    /// [`ReplicaError::NotClaimable`] — the single-writer invariant.
    fn claim(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
        lease_seconds: i64,
    ) -> Result<ReplicaClaim, ReplicaError>;
    /// Extend the lease the caller holds on `task_id`. The caller is
    /// identified by (`worker`, `lane`): lanes share one worker
    /// (`repo-worker-code`), so the worker alone cannot tell a foreign
    /// lane's heartbeat from the owning lane's — the cloud
    /// implementation enforces the same boundary through `claimed_by`
    /// and `foreign_lane_claim` (`turso_queue/lifecycle.rs`).
    fn heartbeat(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
        lease_seconds: i64,
    ) -> Result<(), ReplicaError>;
    /// Record progress on a task the caller holds, identified the same
    /// way as [`CoordinationReplica::heartbeat`].
    fn checkpoint(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
        progress: &str,
    ) -> Result<(), ReplicaError>;
    /// Push local state to the cloud and reconcile. A claim honored
    /// locally is honored globally; double-claims are reported in
    /// [`ReconcileReport::double_claims`], never silently merged.
    fn sync_up(&mut self) -> Result<ReconcileReport, ReplicaError>;
}

/// In-memory reference implementation of [`CoordinationReplica`]: the
/// oracle the Turso replica is checked against, and the stand-in the
/// failure drill runs before Turso is wired. Its clock is
/// caller-supplied (`now_unix`) so lease expiry is deterministic in
/// tests. `sync_up` reports every local claim as synced — a claim
/// honored locally is honored globally — and consumes the injected
/// cloud-side view after reporting it.
#[derive(Debug, Default)]
pub struct MemoryReplica {
    now_unix: i64,
    claims: HashMap<i64, ReplicaClaim>,
    expiries: HashMap<i64, i64>,
    checkpoints: HashMap<i64, String>,
    cloud_claims: HashMap<i64, String>,
}

impl MemoryReplica {
    /// Empty replica at `now_unix` (seconds since the epoch).
    pub fn new(now_unix: i64) -> Self {
        Self {
            now_unix,
            ..Default::default()
        }
    }

    /// Move the fake clock; leases expire against this time.
    pub fn set_now(&mut self, now_unix: i64) {
        self.now_unix = now_unix;
    }

    /// Record that the cloud holds `task_id` for `worker` (drill /
    /// test injection for the partition double-claim case).
    pub fn inject_cloud_claim(&mut self, task_id: i64, worker: &str) {
        self.cloud_claims.insert(task_id, worker.to_string());
    }

    /// The live local claim on `task_id`, if one is held and unexpired.
    fn live_claim(&self, task_id: i64) -> Option<&ReplicaClaim> {
        let claim = self.claims.get(&task_id)?;
        let expiry = self.expiries.get(&task_id).copied().unwrap_or(0);
        if expiry > self.now_unix {
            Some(claim)
        } else {
            None
        }
    }

    fn require_held(&self, task_id: i64, worker: &str, lane: &str) -> Result<(), ReplicaError> {
        match self.live_claim(task_id) {
            Some(claim) if claim.worker == worker && claim.lane == lane => Ok(()),
            Some(claim) => Err(ReplicaError::NotHeld(format!(
                "task {task_id} is held by {} on lane {}, not {worker} on lane {lane}",
                claim.worker, claim.lane
            ))),
            None => Err(ReplicaError::NotHeld(format!(
                "task {task_id} is not held by {worker} on lane {lane}"
            ))),
        }
    }
}

impl CoordinationReplica for MemoryReplica {
    fn claim(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
        lease_seconds: i64,
    ) -> Result<ReplicaClaim, ReplicaError> {
        if let Some(claim) = self.live_claim(task_id) {
            return Err(ReplicaError::NotClaimable(format!(
                "task {task_id} already held by {}",
                claim.worker
            )));
        }
        let claim = ReplicaClaim {
            task_id,
            worker: worker.to_string(),
            lane: lane.to_string(),
            lease_seconds,
        };
        self.expiries.insert(task_id, self.now_unix + lease_seconds);
        self.claims.insert(task_id, claim.clone());
        Ok(claim)
    }

    fn heartbeat(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
        lease_seconds: i64,
    ) -> Result<(), ReplicaError> {
        self.require_held(task_id, worker, lane)?;
        self.expiries.insert(task_id, self.now_unix + lease_seconds);
        Ok(())
    }

    fn checkpoint(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
        progress: &str,
    ) -> Result<(), ReplicaError> {
        self.require_held(task_id, worker, lane)?;
        self.checkpoints.insert(task_id, progress.to_string());
        Ok(())
    }

    fn sync_up(&mut self) -> Result<ReconcileReport, ReplicaError> {
        let mut report = ReconcileReport::default();
        let mut task_ids: Vec<i64> = self.claims.keys().copied().collect();
        task_ids.sort_unstable();
        for task_id in task_ids {
            let claim = &self.claims[&task_id];
            match self.cloud_claims.get(&task_id) {
                Some(cloud_worker) if cloud_worker != &claim.worker => {
                    report.double_claims.push(DoubleClaim {
                        task_id,
                        local_worker: claim.worker.clone(),
                        cloud_worker: cloud_worker.clone(),
                    });
                }
                _ => report.synced_claims.push(task_id),
            }
        }
        self.cloud_claims.clear();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        CoordinationReplica, DbRoute, DoubleClaim, LegAvailability, MemoryReplica, Mode,
        ReconcileReport, ReplicaClaim, ReplicaError, ROUTE_ORDER, mode_line, route_for,
    };
    use proptest::{prop_assert, prop_assert_eq, proptest};

    #[test]
    fn route_order_prefers_cloud_when_database_is_up() {
        assert_eq!(
            route_for(LegAvailability::all_up()),
            (Mode::Normal, DbRoute::Cloud)
        );
        let gh_write_down = LegAvailability {
            github_write: false,
            ..LegAvailability::all_up()
        };
        assert_eq!(
            route_for(gh_write_down),
            (Mode::GhWriteDegraded, DbRoute::Cloud)
        );
        let gh_read_down = LegAvailability {
            github_read: false,
            ..LegAvailability::all_up()
        };
        assert_eq!(
            route_for(gh_read_down),
            (Mode::GhReadDegraded, DbRoute::Cloud)
        );
        let both_gh_down = LegAvailability {
            github_write: false,
            github_read: false,
            ..LegAvailability::all_up()
        };
        assert_eq!(
            route_for(both_gh_down),
            (Mode::GhWriteDegraded, DbRoute::Cloud)
        );
    }

    #[test]
    fn database_down_replica_up_selects_local_replica() {
        let legs = LegAvailability {
            database: false,
            replica: true,
            github_write: false,
            github_read: false,
        };
        assert_eq!(
            route_for(legs),
            (Mode::DbDegraded, DbRoute::LocalReplica)
        );
    }

    #[test]
    fn database_and_replica_down_falls_back_to_github_markers() {
        let legs = LegAvailability {
            database: false,
            replica: false,
            github_write: true,
            github_read: true,
        };
        assert_eq!(
            route_for(legs),
            (Mode::DbDegraded, DbRoute::GitHubMarkers)
        );
    }

    #[test]
    fn route_order_lists_cloud_replica_markers() {
        assert_eq!(
            ROUTE_ORDER,
            [
                DbRoute::Cloud,
                DbRoute::LocalReplica,
                DbRoute::GitHubMarkers
            ]
        );
        assert_eq!(
            ROUTE_ORDER.map(DbRoute::label),
            ["cloud", "local-replica", "github-markers"]
        );
    }

    #[test]
    fn mode_line_names_the_route_actually_used() {
        let legs = LegAvailability {
            database: false,
            replica: true,
            github_write: true,
            github_read: true,
        };
        let value = mode_line("claim", legs).json_value();
        assert_eq!(
            value,
            serde_json::json!({
                "verb": "claim",
                "mode": "db-degraded",
                "route": "local-replica",
            })
        );
    }

    #[test]
    fn memory_replica_claim_heartbeat_checkpoint_sync() {
        let mut replica = MemoryReplica::new(1_000);
        let claim = replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        assert_eq!(claim.task_id, 7);
        assert_eq!(claim.lease_seconds, 300);
        replica.heartbeat(7, "worker-a", "lane-x", 600).unwrap();
        replica
            .checkpoint(7, "worker-a", "lane-x", "half done")
            .unwrap();
        let report = replica.sync_up().unwrap();
        assert_eq!(report.synced_claims, vec![7]);
        assert!(report.zero_lost());
        assert!(report.double_claims.is_empty());
    }

    #[test]
    fn memory_replica_second_claim_conflicts() {
        let mut replica = MemoryReplica::new(1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        let err = replica.claim(7, "worker-b", "lane-y", 300).unwrap_err();
        assert_eq!(
            err,
            ReplicaError::NotClaimable("task 7 already held by worker-a".to_string())
        );
    }

    #[test]
    fn memory_replica_expired_lease_is_reclaimable() {
        let mut replica = MemoryReplica::new(1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        replica.set_now(1_500);
        let claim = replica.claim(7, "worker-b", "lane-y", 300).unwrap();
        assert_eq!(claim.worker, "worker-b");
    }

    #[test]
    fn memory_replica_heartbeat_requires_holding_the_lease() {
        let mut replica = MemoryReplica::new(1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        let err = replica.heartbeat(7, "worker-b", "lane-y", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
        let err = replica.heartbeat(9, "worker-a", "lane-x", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
    }

    #[test]
    fn memory_replica_heartbeat_and_checkpoint_require_the_owning_lane() {
        // Lanes share one worker (repo-worker-code): the owning lane's
        // heartbeat/checkpoint succeed, a foreign lane's fail even with
        // the same worker, so one lane cannot extend another lane's
        // lease or overwrite its checkpoint.
        let mut replica = MemoryReplica::new(1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        let err = replica.heartbeat(7, "worker-a", "lane-y", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
        let err = replica
            .checkpoint(7, "worker-a", "lane-y", "half done")
            .unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
        replica.heartbeat(7, "worker-a", "lane-x", 600).unwrap();
        replica
            .checkpoint(7, "worker-a", "lane-x", "half done")
            .unwrap();
    }

    /// Reference model for the replica state machine: claims keyed by
    /// task with (worker, lane, expiry), checkpoints, and the injected
    /// cloud view. It mirrors the contract `MemoryReplica` implements —
    /// including that `sync_up` reports every recorded claim, live or
    /// expired — so the generated test below checks the implementation
    /// against an explicit oracle rather than fixed examples.
    #[derive(Debug, Default)]
    struct ModelReplica {
        now_unix: i64,
        claims: HashMap<i64, (String, String, i64)>,
        checkpoints: HashMap<i64, String>,
        cloud_claims: HashMap<i64, String>,
    }

    impl ModelReplica {
        fn live(&self, task_id: i64) -> Option<&(String, String, i64)> {
            self.claims
                .get(&task_id)
                .filter(|(_, _, expiry)| *expiry > self.now_unix)
        }

        fn claim(&mut self, task_id: i64, worker: &str, lane: &str, lease: i64) -> bool {
            if self.live(task_id).is_some() {
                return false;
            }
            self.claims.insert(
                task_id,
                (worker.to_string(), lane.to_string(), self.now_unix + lease),
            );
            true
        }

        fn held_by(&self, task_id: i64, worker: &str, lane: &str) -> bool {
            matches!(self.live(task_id), Some((w, l, _)) if w == worker && l == lane)
        }

        fn heartbeat(&mut self, task_id: i64, worker: &str, lane: &str, lease: i64) -> bool {
            if !self.held_by(task_id, worker, lane) {
                return false;
            }
            self.claims.get_mut(&task_id).unwrap().2 = self.now_unix + lease;
            true
        }

        fn checkpoint(&mut self, task_id: i64, worker: &str, lane: &str, progress: &str) -> bool {
            if !self.held_by(task_id, worker, lane) {
                return false;
            }
            self.checkpoints.insert(task_id, progress.to_string());
            true
        }

        fn sync_up(&mut self) -> ReconcileReport {
            let mut report = ReconcileReport::default();
            let mut task_ids: Vec<i64> = self.claims.keys().copied().collect();
            task_ids.sort_unstable();
            for task_id in task_ids {
                let (worker, _, _) = &self.claims[&task_id];
                match self.cloud_claims.get(&task_id) {
                    Some(cloud_worker) if cloud_worker != worker => {
                        report.double_claims.push(DoubleClaim {
                            task_id,
                            local_worker: worker.clone(),
                            cloud_worker: cloud_worker.clone(),
                        });
                    }
                    _ => report.synced_claims.push(task_id),
                }
            }
            self.cloud_claims.clear();
            report
        }
    }

    proptest! {
        #[test]
        fn memory_replica_matches_model_over_generated_sequences(
            ops in proptest::prop::collection::vec(
                (0..6u8, 1..4i64, 0..2usize, 0..2usize, 0..2i64, 1000..2000i64),
                1..40,
            ),
        ) {
            // Generated operation sequences with the model above as the
            // oracle: single-writer, lease-expiry, lane isolation, and
            // reconciliation (including expiry crossings and repeated
            // syncs) are checked on every step, not just on examples.
            // Op kinds: 0 claim, 1 heartbeat, 2 checkpoint, 3 sync_up,
            // 4 inject a cloud claim, 5 move the clock.
            let workers = ["worker-a", "worker-b"];
            let lanes = ["lane-x", "lane-y"];
            let leases = [60i64, 300];
            let progresses = ["half done", "done"];
            let mut replica = MemoryReplica::new(1_000);
            let mut model = ModelReplica {
                now_unix: 1_000,
                ..Default::default()
            };
            for (kind, task_id, worker_idx, lane_idx, choice, now) in ops {
                let worker = workers[worker_idx];
                let lane = lanes[lane_idx];
                match kind {
                    0 => {
                        let lease = leases[choice as usize];
                        let got = replica.claim(task_id, worker, lane, lease);
                        let want = model.claim(task_id, worker, lane, lease);
                        prop_assert_eq!(got.is_ok(), want);
                        if let Ok(claim) = got {
                            prop_assert_eq!(claim, ReplicaClaim {
                                task_id,
                                worker: worker.to_string(),
                                lane: lane.to_string(),
                                lease_seconds: lease,
                            });
                        }
                    }
                    1 => {
                        let lease = leases[choice as usize];
                        let got = replica.heartbeat(task_id, worker, lane, lease);
                        let want = model.heartbeat(task_id, worker, lane, lease);
                        prop_assert_eq!(got.is_ok(), want);
                    }
                    2 => {
                        let progress = progresses[choice as usize];
                        let got = replica.checkpoint(task_id, worker, lane, progress);
                        let want = model.checkpoint(task_id, worker, lane, progress);
                        prop_assert_eq!(got.is_ok(), want);
                    }
                    3 => {
                        let got = replica.sync_up().unwrap();
                        let want = model.sync_up();
                        prop_assert!(got.zero_lost());
                        prop_assert_eq!(got, want);
                    }
                    4 => {
                        replica.inject_cloud_claim(task_id, worker);
                        model.cloud_claims.insert(task_id, worker.to_string());
                    }
                    _ => {
                        replica.set_now(now);
                        model.now_unix = now;
                    }
                }
            }
            // A final reconciliation must agree too, so divergent
            // state cannot hide behind matching per-step outcomes.
            let got = replica.sync_up().unwrap();
            let want = model.sync_up();
            prop_assert!(got.zero_lost());
            prop_assert_eq!(got, want);
        }
    }

    #[test]
    fn memory_replica_double_claim_surfaces_in_sync_report() {
        let mut replica = MemoryReplica::new(1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        replica.inject_cloud_claim(7, "worker-b");
        let report = replica.sync_up().unwrap();
        assert!(report.zero_lost());
        assert!(report.synced_claims.is_empty());
        assert_eq!(report.double_claims.len(), 1);
        assert_eq!(report.double_claims[0].task_id, 7);
        assert_eq!(report.double_claims[0].local_worker, "worker-a");
        assert_eq!(report.double_claims[0].cloud_worker, "worker-b");
    }
}