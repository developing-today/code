//! Durable operator status for a verified journal replica.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::ReplicationCursor;

/// Status sidecar written only after a signed terminal frame commits.
pub const REPLICA_STATUS_FILE: &str = "replica-status.json";

/// Last complete cursor verified and durably installed by the follower.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaStatus {
    /// Status schema version.
    pub version: u8,
    /// Pinned writer node ID.
    pub source: [u8; 32],
    /// Replicated world identifier.
    pub world_id: String,
    /// Last committed replication cursor.
    pub cursor: ReplicationCursor,
    /// Unix time when this cursor was durably committed.
    pub verified_at_unix_secs: u64,
}

impl ReplicaStatus {
    /// Build the status record for a committed cursor.
    #[must_use]
    pub fn now(source: [u8; 32], world_id: String, cursor: ReplicationCursor) -> Self {
        Self {
            version: 1,
            source,
            world_id,
            cursor,
            verified_at_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Persist status atomically after the receiver has committed its journal.
    ///
    /// # Errors
    ///
    /// Returns an error if the sidecar cannot be written, flushed or renamed.
    pub fn write(&self, replica_dir: &Path) -> Result<()> {
        ensure!(self.version == 1, "unsupported replica status version");
        fs::create_dir_all(replica_dir)
            .with_context(|| format!("create replica directory {}", replica_dir.display()))?;
        let path = replica_dir.join(REPLICA_STATUS_FILE);
        let temporary = replica_dir.join(format!("{REPLICA_STATUS_FILE}.tmp"));
        let bytes = serde_json::to_vec(self).context("encode replica status")?;
        {
            let mut file = File::create(&temporary)
                .with_context(|| format!("create {}", temporary.display()))?;
            file.write_all(&bytes)
                .with_context(|| format!("write {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("sync {}", temporary.display()))?;
        }
        fs::rename(&temporary, &path).with_context(|| format!("install {}", path.display()))?;
        File::open(replica_dir)
            .and_then(|directory| directory.sync_all())
            .with_context(|| format!("sync replica directory {}", replica_dir.display()))?;
        Ok(())
    }

    /// Read status only when it names the expected pinned source and world.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, unsupported, or mismatched status.
    pub fn read(replica_dir: &Path, source: [u8; 32], world_id: &str) -> Result<Self> {
        let state_dir = super::active_replica_dir(replica_dir)?;
        let path = state_dir.join(REPLICA_STATUS_FILE);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let status: Self = serde_json::from_slice(&bytes).context("decode replica status")?;
        ensure!(status.version == 1, "unsupported replica status version");
        ensure!(
            status.source == source,
            "replica status source does not match pin"
        );
        ensure!(
            status.world_id == world_id,
            "replica status world does not match request"
        );
        Ok(status)
    }

    /// Seconds elapsed since this cursor was verified, saturating if the local
    /// clock moved backwards.
    #[must_use]
    pub fn staleness_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(self.verified_at_unix_secs)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::world::chain_base_cursor;

    #[test]
    fn status_round_trips_and_reports_nonnegative_staleness() {
        let directory = tempfile::tempdir().unwrap();
        let source = [7; 32];
        let cursor = chain_base_cursor("lobby", &source, 1);
        let status = ReplicaStatus::now(source, "lobby".to_owned(), cursor);

        status.write(directory.path()).unwrap();
        let restored = ReplicaStatus::read(directory.path(), source, "lobby").unwrap();

        assert_eq!(restored, status);
        assert!(restored.staleness_secs() < 2);
        assert!(ReplicaStatus::read(directory.path(), [8; 32], "lobby").is_err());
    }
}
