//! Read-only world service over a verified replica journal (F12.13 phase 4).
//!
//! The durable receiver ([`super::replication_receiver`]) persists and
//! verifies the signed stream; this module serves from it. Opening a replica
//! is non-mutating: the verified journal bytes are parsed and restored with
//! [`WorldCore::restore_replica`] (presence-preserving, no header bumps, no
//! torn-tail repair), directory facts arrive through
//! [`Directory::replica_facts_for`], and the actor spawns with
//! [`WorldHandle::spawn_read_only`], so every local mutation and program
//! side effect is refused while spectator reads serve over the existing
//! world session protocol.
//!
//! What phase 4 deliberately does not do (phase 5 owns it): the operator
//! CLI/configuration that fetches the stream and the directory facts into
//! a replica directory, the resilience exercises, and the offline-admission
//! policy choice for NEW spectators after a writer outage (design §8). A
//! replica serves whoever its verified journal already authorizes; it never
//! admits anyone the journal does not know.

use std::collections::BTreeSet;

use anyhow::{Context, Result};

use super::{
    JournalEntry, MAX_JOURNAL_VERSION, RestoredProgram, WorldCore, WorldEvent, WorldHandle,
    WorldLimits, WorldProgram,
};

/// A replica journal restored and ready to serve: the admission-checked
/// core, the program the journal says was running, and the inputs to replay
/// through a fresh program instance before serving.
#[derive(Debug)]
pub struct ReplicaRestore {
    /// Capabilities, sequence and events rebuilt from the verified journal,
    /// with the replica directory facts installed.
    pub core: WorldCore,
    /// The last installed program, if any.
    pub program: Option<RestoredProgram>,
    /// Inputs committed after the install or checkpoint, in order, to replay
    /// through a fresh program instance.
    pub replay: Vec<WorldEvent>,
    /// Where the verified journal stands.
    pub cursor: ReplicaCursor,
}

/// Recomputed position of a verified replica journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplicaCursor {
    /// Journal entries restored, including the `Created` header.
    pub entries: u64,
    /// Last committed world sequence.
    pub sequence: u64,
    /// Journal header version the replica stands at.
    pub header_version: u32,
}

/// Restore a replica from already-verified journal bytes: parse them with
/// the same admission rules the receiver enforces (complete lines, this
/// world's `Created` header at a supported version, no second header),
/// restore presence-preserving, and install the directory facts for
/// `subjects` (every account appearing in the journal's capability chains;
/// see [`crate::directory::Directory::replica_facts_for`]).
///
/// # Errors
///
/// Fails when the bytes are not a well-formed replica journal for
/// `world_id` or the directory facts do not apply. A bad journal is
/// refused, never partially applied; nothing is written anywhere.
pub fn restore_replica_journal(
    world_id: &str,
    limits: WorldLimits,
    journal_bytes: &[u8],
    directory: &crate::directory::Directory,
    subjects: &BTreeSet<String>,
) -> Result<ReplicaRestore> {
    let entries = parse_replica_journal(journal_bytes, world_id)?;
    restore_replica_entries(world_id, limits, entries, directory, subjects)
}

/// Parse and validate verified replica journal bytes: complete entry lines
/// only, this world's `Created` header at a supported version first, and no
/// second header. Mirrors the admission in [`super::replication_receiver`]
/// so serving never reads back what the receiver would refuse at open.
pub fn parse_replica_journal(bytes: &[u8], world_id: &str) -> Result<Vec<JournalEntry>> {
    let corrupt = |reason: &str| anyhow::anyhow!("replica journal is corrupt: {reason}");
    if bytes.is_empty() {
        return Err(corrupt("journal is empty"));
    }
    if !bytes.ends_with(b"\n") {
        return Err(corrupt("journal does not end at an entry boundary"));
    }
    let mut entries = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let entry: JournalEntry =
            serde_json::from_slice(line).map_err(|_| corrupt("a journal line does not decode"))?;
        match (entries.is_empty(), &entry) {
            (true, JournalEntry::Created {
                world_id: header_world,
                version,
            }) => {
                if header_world != world_id {
                    return Err(corrupt("journal header names a different world"));
                }
                if *version == 0 || *version > MAX_JOURNAL_VERSION {
                    return Err(corrupt("journal header version is not supported"));
                }
            }
            (true, _) => return Err(corrupt("journal does not start with its Created header")),
            (false, JournalEntry::Created { .. }) => {
                return Err(corrupt("journal carries a second Created header"));
            }
            (false, _) => {}
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Restore a replica from already-parsed entries. Shared by the in-memory
/// open and the durable [`crate::world_store::open_replica`].
pub fn restore_replica_entries(
    world_id: &str,
    limits: WorldLimits,
    entries: Vec<JournalEntry>,
    directory: &crate::directory::Directory,
    subjects: &BTreeSet<String>,
) -> Result<ReplicaRestore> {
    let header_version = match entries.first() {
        Some(JournalEntry::Created { version, .. }) => *version,
        _ => anyhow::bail!("replica journal is corrupt: missing Created header"),
    };
    // Presence-preserving: the verified journal replays exactly; no
    // synthetic presence-off events are fed into the program.
    let entry_count = entries.len();
    let mut restored = WorldCore::restore_replica(world_id, limits, entries)?;
    let facts = directory.replica_facts_for(subjects)?;
    for fact in facts {
        restored
            .core
            .directory_mut()
            .apply(&fact)
            .with_context(|| format!("replica directory fact {fact:?} is refused"))?;
    }
    Ok(ReplicaRestore {
        cursor: ReplicaCursor {
            entries: entry_count as u64,
            sequence: restored.core.current_sequence(),
            header_version,
        },
        core: restored.core,
        program: restored.program,
        replay: restored.replay,
    })
}

/// Serve a restored replica: replay the journal's inputs through `program`
/// (exactly like the writer's restore path), then spawn the read-only
/// actor. Spectator reads serve from the restored state; every local
/// mutation and program side effect is refused.
///
/// # Errors
///
/// Fails when the program rejects a replayed input. The journal is left
/// untouched; the replica simply does not open.
pub fn serve_replica(
    restored: ReplicaRestore,
    mut program: Box<dyn WorldProgram>,
) -> Result<(WorldHandle, ReplicaCursor)> {
    for event in &restored.replay {
        program
            .update(event)
            .with_context(|| format!("replica program rejected replayed input {}", event.sequence))?;
    }
    let cursor = restored.cursor;
    let handle = WorldHandle::spawn_durable_read_only(restored.core, program, None);
    debug_assert!(handle.is_read_only());
    Ok((handle, cursor))
}
