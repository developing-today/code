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
    /// Extend the lease the caller holds on `task_id`.
    fn heartbeat(
        &mut self,
        task_id: i64,
        worker: &str,
        lease_seconds: i64,
    ) -> Result<(), ReplicaError>;
    /// Record progress on a task the caller holds.
    fn checkpoint(
        &mut self,
        task_id: i64,
        worker: &str,
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

    fn require_held(&self, task_id: i64, worker: &str) -> Result<(), ReplicaError> {
        match self.live_claim(task_id) {
            Some(claim) if claim.worker == worker => Ok(()),
            Some(claim) => Err(ReplicaError::NotHeld(format!(
                "task {task_id} is held by {}, not {worker}",
                claim.worker
            ))),
            None => Err(ReplicaError::NotHeld(format!(
                "task {task_id} is not held by {worker}"
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
        lease_seconds: i64,
    ) -> Result<(), ReplicaError> {
        self.require_held(task_id, worker)?;
        self.expiries.insert(task_id, self.now_unix + lease_seconds);
        Ok(())
    }

    fn checkpoint(
        &mut self,
        task_id: i64,
        worker: &str,
        progress: &str,
    ) -> Result<(), ReplicaError> {
        self.require_held(task_id, worker)?;
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
    use super::{
        CoordinationReplica, DbRoute, LegAvailability, MemoryReplica, Mode, ReplicaError,
        ROUTE_ORDER, mode_line, route_for,
    };

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
        replica.heartbeat(7, "worker-a", 600).unwrap();
        replica.checkpoint(7, "worker-a", "half done").unwrap();
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
        let err = replica.heartbeat(7, "worker-b", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
        let err = replica.heartbeat(9, "worker-a", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
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
