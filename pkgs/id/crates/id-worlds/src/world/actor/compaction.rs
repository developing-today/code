//! Journal compaction for the world actor: policy-driven checkpoints,
//! forced compaction for oversized legacy entries, and the replication
//! reset publish that follows a compaction.
//! Extracted from `actor.rs` for the production-line guard (#14547):
//! moves only, no behaviour change.

use super::*;

impl WorldActor {
    /// Trim the journal behind a verified snapshot, at most once per run of
    /// commits and never while the program cannot snapshot.
    pub(super) async fn compact_if_due(&mut self) {
        if self.read_only
            || self.compaction_off
            || self.storage_failed
            || !self.program_healthy
            || !self.core.checkpoint_due()
        {
            return;
        }
        let started = std::time::Instant::now();
        let compacted = match self.journal.as_mut() {
            Some(journal) => {
                compact_journal(&mut self.core, self.program.as_mut(), journal.as_mut())
            }
            None => return,
        };
        match compacted {
            Ok(true) => {
                self.metrics.record_checkpoint(started.elapsed());
                self.publish_replication_reset();
            }
            Ok(false) => self.compaction_off = true,
            Err(error) => {
                // The journal is still complete; just do not retry until
                // something changes.
                tracing::warn!("world checkpoint skipped: {error:#}");
                self.compaction_off = true;
            }
        }
    }

    /// Re-compact a legacy journal only when a feed discovers an entry that
    /// cannot fit the v1 per-entry payload bound. Unlike policy-driven
    /// compaction, this intentionally ignores `checkpoint_due()`.
    pub(super) fn force_compact_for_replication(&mut self, ordinal: usize, encoded_size: usize) -> Result<()> {
        let unavailable = if self.read_only {
            Some("the world is read-only")
        } else if self.compaction_off {
            Some("compaction was disabled after an earlier failure")
        } else if self.storage_failed {
            Some("world storage has failed")
        } else if !self.program_healthy {
            Some("the active program is unhealthy")
        } else if self.core.active_module_hash.is_none() {
            Some("there is no active program to checkpoint")
        } else if self.journal.is_none() {
            Some("the world has no durable journal to compact")
        } else {
            None
        };
        if let Some(reason) = unavailable {
            return Err(oversized_journal_compaction_error(
                ordinal,
                encoded_size,
                reason,
            ));
        }

        let started = std::time::Instant::now();
        let Some(journal) = self.journal.as_mut() else {
            return Err(oversized_journal_compaction_error(
                ordinal,
                encoded_size,
                "the world has no durable journal to compact",
            ));
        };
        let compacted = compact_journal(&mut self.core, self.program.as_mut(), journal.as_mut());
        match compacted {
            Ok(true) => {
                self.metrics.record_checkpoint(started.elapsed());
                self.publish_replication_reset();
                Ok(())
            }
            Ok(false) => {
                self.compaction_off = true;
                Err(oversized_journal_compaction_error(
                    ordinal,
                    encoded_size,
                    "the active program cannot provide a checkpoint snapshot",
                ))
            }
            Err(error) => {
                self.compaction_off = true;
                Err(oversized_journal_compaction_error(
                    ordinal,
                    encoded_size,
                    &format!("forced compaction failed: {error:#}"),
                ))
            }
        }
    }

    pub(super) fn publish_replication_reset(&mut self) {
        let Some(journal) = self.journal.as_mut() else {
            return;
        };
        let update = journal
            .replication_snapshot()
            .map(ReplicationUpdate::Reset)
            .unwrap_or_else(|error| {
                tracing::error!(
                    "world {}: replication reset snapshot failed: {error:#}",
                    self.core.world_id()
                );
                ReplicationUpdate::ResetFailed
            });
        let _ = self.replication_sender.send(update);
    }
}
