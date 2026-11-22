//! Durable local coordination replica for database-down resilience
//! (#8583): a [`CoordinationReplica`] backed by a local SQLite store.
//!
//! The contract in [`super::routing`] fixes the interface and the
//! reconciliation rules; [`super::routing::MemoryReplica`] is the
//! in-memory reference. This implementation keeps the same semantics
//! — caller-supplied clock, (worker, lane) identity, a claim honored
//! locally is honored globally, partition double-claims surface in the
//! [`ReconcileReport`] and are never silently merged — but records
//! claims, leases, and checkpoints in a SQLite file on the local
//! machine, so coordination state survives a process restart while
//! the cloud database is unreachable.
//!
//! The store holds coordination only (claims, leases, checkpoints):
//! per slice 1's classification it passes the wipe test by
//! construction — deleting the file loses nothing the cloud does not
//! re-derive, because every claim it holds is pushed to the cloud by
//! [`CoordinationReplica::sync_up`] on recovery.
//!
//! `sync_up` reconciles against a cloud store the caller attaches
//! ([`SqliteReplica::attach_cloud`]). In the drill the cloud store is
//! a second SQLite file standing in for the Turso database; claims
//! the cloud side took during the partition are injected into its
//! view ([`SqliteReplica::inject_cloud_claim`]) exactly as tests
//! inject them into `MemoryReplica`, and a sync consumes that view:
//! a local claim whose task the view holds for a different
//! (worker, lane) is reported as a double-claim, and every other
//! local claim is applied to the cloud store and reported synced.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::routing::{
    CoordinationReplica, DoubleClaim, ReconcileReport, ReplicaClaim, ReplicaError,
};

const LOCAL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS replica_claims (
    task_id INTEGER PRIMARY KEY,
    worker TEXT NOT NULL,
    lane TEXT NOT NULL,
    lease_seconds INTEGER NOT NULL,
    expiry_unix INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS replica_checkpoints (
    task_id INTEGER PRIMARY KEY,
    progress TEXT NOT NULL
);
";

const CLOUD_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS cloud_claims (
    task_id INTEGER PRIMARY KEY,
    worker TEXT NOT NULL,
    lane TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS cloud_view (
    task_id INTEGER PRIMARY KEY,
    worker TEXT NOT NULL,
    lane TEXT NOT NULL
);
";

fn unavailable(context: &str, err: rusqlite::Error) -> ReplicaError {
    ReplicaError::Unavailable(format!("{context}: {err}"))
}

/// A [`CoordinationReplica`] persisted in a local SQLite file.
/// The clock is caller-supplied (`now_unix`), matching
/// [`super::routing::MemoryReplica`], so lease expiry is
/// deterministic in tests and drills.
#[derive(Debug)]
pub struct SqliteReplica {
    conn: Connection,
    cloud: Option<Connection>,
    now_unix: i64,
}

impl SqliteReplica {
    /// Open (creating if needed) the local replica store at `path`.
    pub fn open(path: impl AsRef<Path>, now_unix: i64) -> Result<Self, ReplicaError> {
        let path = path.as_ref();
        let conn = Connection::open(path)
            .map_err(|e| unavailable(&format!("open replica store {}", path.display()), e))?;
        conn.execute_batch(LOCAL_SCHEMA)
            .map_err(|e| unavailable("initialise replica store", e))?;
        Ok(Self {
            conn,
            cloud: None,
            now_unix,
        })
    }

    /// Move the clock; leases expire against this time.
    pub fn set_now(&mut self, now_unix: i64) {
        self.now_unix = now_unix;
    }

    /// Attach the cloud store `sync_up` reconciles against (a file
    /// standing in for the cloud database in drills and tests).
    pub fn attach_cloud(&mut self, path: impl AsRef<Path>) -> Result<(), ReplicaError> {
        let path = path.as_ref();
        let conn = Connection::open(path)
            .map_err(|e| unavailable(&format!("open cloud store {}", path.display()), e))?;
        conn.execute_batch(CLOUD_SCHEMA)
            .map_err(|e| unavailable("initialise cloud store", e))?;
        self.cloud = Some(conn);
        Ok(())
    }

    /// Record that the cloud holds `task_id` for (`worker`, `lane`)
    /// (drill / test injection for the partition double-claim case,
    /// mirroring `MemoryReplica::inject_cloud_claim`). The next
    /// `sync_up` consumes the injected view.
    pub fn inject_cloud_claim(
        &mut self,
        task_id: i64,
        worker: &str,
        lane: &str,
    ) -> Result<(), ReplicaError> {
        let cloud = self
            .cloud
            .as_ref()
            .ok_or_else(|| ReplicaError::Unavailable("no cloud store attached".to_string()))?;
        cloud
            .execute(
                "INSERT INTO cloud_view (task_id, worker, lane) VALUES (?1, ?2, ?3)
                 ON CONFLICT(task_id) DO UPDATE SET worker = excluded.worker, lane = excluded.lane",
                params![task_id, worker, lane],
            )
            .map_err(|e| unavailable("inject cloud claim", e))?;
        Ok(())
    }

    /// The claim the cloud store currently honors for `task_id`,
    /// if a sync has applied one.
    pub fn cloud_claim(&self, task_id: i64) -> Result<Option<(String, String)>, ReplicaError> {
        let cloud = self
            .cloud
            .as_ref()
            .ok_or_else(|| ReplicaError::Unavailable("no cloud store attached".to_string()))?;
        cloud
            .query_row(
                "SELECT worker, lane FROM cloud_claims WHERE task_id = ?1",
                params![task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| unavailable("read cloud claim", e))
    }

    /// The live local claim on `task_id`, if one is held and unexpired.
    fn live_claim(&self, task_id: i64) -> Result<Option<ReplicaClaim>, ReplicaError> {
        let claim = self
            .conn
            .query_row(
                "SELECT worker, lane, lease_seconds, expiry_unix FROM replica_claims
                 WHERE task_id = ?1",
                params![task_id],
                |row| {
                    Ok((
                        ReplicaClaim {
                            task_id,
                            worker: row.get(0)?,
                            lane: row.get(1)?,
                            lease_seconds: row.get(2)?,
                        },
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| unavailable("read local claim", e))?;
        Ok(match claim {
            Some((claim, expiry)) if expiry > self.now_unix => Some(claim),
            _ => None,
        })
    }

    fn require_held(&self, task_id: i64, worker: &str, lane: &str) -> Result<(), ReplicaError> {
        match self.live_claim(task_id)? {
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

impl CoordinationReplica for SqliteReplica {
    fn claim(
        &mut self,
        _task_id: i64,
        _worker: &str,
        _lane: &str,
        _lease_seconds: i64,
    ) -> Result<ReplicaClaim, ReplicaError> {
        Err(ReplicaError::Unavailable(
            "sqlite replica not implemented yet".to_string(),
        ))
    }

    fn heartbeat(
        &mut self,
        _task_id: i64,
        _worker: &str,
        _lane: &str,
        _lease_seconds: i64,
    ) -> Result<(), ReplicaError> {
        Err(ReplicaError::Unavailable(
            "sqlite replica not implemented yet".to_string(),
        ))
    }

    fn checkpoint(
        &mut self,
        _task_id: i64,
        _worker: &str,
        _lane: &str,
        _progress: &str,
    ) -> Result<(), ReplicaError> {
        Err(ReplicaError::Unavailable(
            "sqlite replica not implemented yet".to_string(),
        ))
    }

    fn sync_up(&mut self) -> Result<ReconcileReport, ReplicaError> {
        Err(ReplicaError::Unavailable(
            "sqlite replica not implemented yet".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::SqliteReplica;
    use crate::work::routing::{
        CoordinationReplica, DoubleClaim, ReconcileReport, ReplicaClaim, ReplicaError,
    };
    use proptest::{prop_assert, prop_assert_eq, proptest};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh pair of store paths (local replica + cloud stand-in)
    /// unique to this test invocation.
    fn temp_stores(tag: &str) -> (PathBuf, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir();
        let base = format!("fleet-sqlite-replica-{tag}-{}-{n}", std::process::id());
        (
            dir.join(format!("{base}-local.db")),
            dir.join(format!("{base}-cloud.db")),
        )
    }

    fn cleanup(paths: &[&PathBuf]) {
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    }

    fn open_replica(tag: &str, now: i64) -> (SqliteReplica, PathBuf, PathBuf) {
        let (local, cloud) = temp_stores(tag);
        let mut replica = SqliteReplica::open(&local, now).unwrap();
        replica.attach_cloud(&cloud).unwrap();
        (replica, local, cloud)
    }

    #[test]
    fn sqlite_replica_claim_heartbeat_checkpoint_sync() {
        let (mut replica, local, cloud) = open_replica("basic", 1_000);
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
        // The claim was applied to the cloud store, not just reported.
        assert_eq!(
            replica.cloud_claim(7).unwrap(),
            Some(("worker-a".to_string(), "lane-x".to_string()))
        );
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_second_claim_conflicts() {
        let (mut replica, local, cloud) = open_replica("conflict", 1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        let err = replica.claim(7, "worker-b", "lane-y", 300).unwrap_err();
        assert_eq!(
            err,
            ReplicaError::NotClaimable("task 7 already held by worker-a".to_string())
        );
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_expired_lease_is_reclaimable() {
        let (mut replica, local, cloud) = open_replica("expiry", 1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        replica.set_now(1_500);
        let claim = replica.claim(7, "worker-b", "lane-y", 300).unwrap();
        assert_eq!(claim.worker, "worker-b");
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_heartbeat_requires_holding_the_lease() {
        let (mut replica, local, cloud) = open_replica("held", 1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        let err = replica.heartbeat(7, "worker-b", "lane-y", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
        let err = replica.heartbeat(9, "worker-a", "lane-x", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotHeld(_)));
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_heartbeat_and_checkpoint_require_the_owning_lane() {
        // Same boundary as MemoryReplica: lanes share one worker, so a
        // foreign lane's heartbeat/checkpoint fail even with the same
        // worker id.
        let (mut replica, local, cloud) = open_replica("lane", 1_000);
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
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_claim_survives_reopen() {
        // The durability MemoryReplica cannot offer: a claim recorded
        // before the store is closed is still held after it reopens,
        // so a restarted tool keeps honoring the lease it took.
        let (local, cloud) = temp_stores("reopen");
        {
            let mut replica = SqliteReplica::open(&local, 1_000).unwrap();
            replica.claim(7, "worker-a", "lane-x", 300).unwrap();
            replica
                .checkpoint(7, "worker-a", "lane-x", "half done")
                .unwrap();
        }
        let mut replica = SqliteReplica::open(&local, 1_100).unwrap();
        replica.attach_cloud(&cloud).unwrap();
        let err = replica.claim(7, "worker-b", "lane-y", 300).unwrap_err();
        assert!(matches!(err, ReplicaError::NotClaimable(_)));
        replica.heartbeat(7, "worker-a", "lane-x", 600).unwrap();
        let report = replica.sync_up().unwrap();
        assert_eq!(report.synced_claims, vec![7]);
        assert!(report.zero_lost());
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_sync_without_a_cloud_store_is_unavailable() {
        let (local, _cloud) = temp_stores("nocloud");
        let mut replica = SqliteReplica::open(&local, 1_000).unwrap();
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        let err = replica.sync_up().unwrap_err();
        assert!(matches!(err, ReplicaError::Unavailable(_)));
        cleanup(&[&local]);
    }

    #[test]
    fn sqlite_replica_double_claim_surfaces_in_sync_report() {
        let (mut replica, local, cloud) = open_replica("double", 1_000);
        replica.claim(7, "worker-a", "lane-x", 300).unwrap();
        replica.inject_cloud_claim(7, "worker-b", "lane-y").unwrap();
        let report = replica.sync_up().unwrap();
        assert!(report.zero_lost());
        assert!(report.synced_claims.is_empty());
        assert_eq!(report.double_claims.len(), 1);
        assert_eq!(report.double_claims[0].task_id, 7);
        assert_eq!(report.double_claims[0].local_worker, "worker-a");
        assert_eq!(report.double_claims[0].local_lane, "lane-x");
        assert_eq!(report.double_claims[0].cloud_worker, "worker-b");
        assert_eq!(report.double_claims[0].cloud_lane, "lane-y");
        // A double-claim is reported, never silently merged: the cloud
        // store does not gain the local holder's claim.
        assert_eq!(replica.cloud_claim(7).unwrap(), None);
        cleanup(&[&local, &cloud]);
    }

    #[test]
    fn sqlite_replica_double_claim_distinguishes_lanes_sharing_a_worker() {
        let (mut replica, local, cloud) = open_replica("doublelane", 1_000);
        replica
            .claim(7, "repo-worker-code", "code-h", 300)
            .unwrap();
        replica
            .inject_cloud_claim(7, "repo-worker-code", "code-i")
            .unwrap();
        let report = replica.sync_up().unwrap();
        assert!(report.zero_lost());
        assert!(report.synced_claims.is_empty());
        assert_eq!(report.double_claims.len(), 1);
        assert_eq!(report.double_claims[0].local_lane, "code-h");
        assert_eq!(report.double_claims[0].cloud_lane, "code-i");
        cleanup(&[&local, &cloud]);
    }

    /// Reference model for the replica state machine — the oracle for
    /// the generated test below. It mirrors the contract semantics
    /// (the same model `MemoryReplica` is checked against in
    /// `work/routing.rs`): claims keyed by task with
    /// (worker, lane, expiry), an injected cloud view that `sync_up`
    /// consumes, double-claims on a divergent view holder, and every
    /// other local claim synced.
    #[derive(Debug, Default)]
    struct ModelReplica {
        now_unix: i64,
        claims: HashMap<i64, (String, String, i64)>,
        checkpoints: HashMap<i64, String>,
        cloud_claims: HashMap<i64, (String, String)>,
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
                let (worker, lane, _) = &self.claims[&task_id];
                match self.cloud_claims.get(&task_id) {
                    Some((cloud_worker, cloud_lane))
                        if cloud_worker != worker || cloud_lane != lane =>
                    {
                        report.double_claims.push(DoubleClaim {
                            task_id,
                            local_worker: worker.clone(),
                            local_lane: lane.clone(),
                            cloud_worker: cloud_worker.clone(),
                            cloud_lane: cloud_lane.clone(),
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
        fn sqlite_replica_matches_model_over_generated_sequences(
            ops in proptest::collection::vec(
                (0..6u8, 1..4i64, 0..2usize, 0..2usize, 0..2i64, 1000..2000i64),
                1..40,
            ),
        ) {
            // Generated operation sequences with the model above as
            // the oracle: single-writer, lease-expiry, lane isolation,
            // durability-shaped reconciliation (including expiry
            // crossings and repeated syncs) are checked on every step.
            // Op kinds: 0 claim, 1 heartbeat, 2 checkpoint, 3 sync_up,
            // 4 inject a cloud claim, 5 move the clock.
            let workers = ["worker-a", "worker-b"];
            let lanes = ["lane-x", "lane-y"];
            let leases = [60i64, 300];
            let progresses = ["half done", "done"];
            let (mut replica, local, cloud) = open_replica("prop", 1_000);
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
                        replica.inject_cloud_claim(task_id, worker, lane).unwrap();
                        model.cloud_claims.insert(
                            task_id,
                            (worker.to_string(), lane.to_string()),
                        );
                    }
                    _ => {
                        replica.set_now(now);
                        model.now_unix = now;
                    }
                }
            }
            let got = replica.sync_up().unwrap();
            let want = model.sync_up();
            prop_assert!(got.zero_lost());
            prop_assert_eq!(got, want);
            cleanup(&[&local, &cloud]);
        }
    }
}
