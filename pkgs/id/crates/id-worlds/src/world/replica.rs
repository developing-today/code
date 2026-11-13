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
//! a replica directory, and the resilience exercises. The offline-admission
//! policy choice for NEW spectators after a writer outage (design §8) is
//! settled: policy B — a writer-signed admission lease — was adopted by
//! the ruling on #806, and the lease primitive now lives in
//! [`super::admission_lease`]. Until the session-admission path checks
//! that lease, behavior here is unchanged: a replica serves whoever its
//! verified journal already authorizes; it never admits anyone the
//! journal does not know.

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
    /// Participant IDs present at the verified journal cursor; preserved
    /// as actor session state without replaying synthetic leaves.
    pub presence: std::collections::BTreeMap<u64, u64>,
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
    restore_replica_journal_with_runtime(
        world_id,
        limits,
        crate::world_limits::RuntimeLimits::default(),
        journal_bytes,
        directory,
        subjects,
    )
}

/// As [`restore_replica_journal`], resolving the journal's per-world memory
/// request against the current server runtime ceilings.
pub fn restore_replica_journal_with_runtime(
    world_id: &str,
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
    journal_bytes: &[u8],
    directory: &crate::directory::Directory,
    subjects: &BTreeSet<String>,
) -> Result<ReplicaRestore> {
    let entries = parse_replica_journal(journal_bytes, world_id)?;
    restore_replica_entries_with_runtime(world_id, limits, runtime, entries, directory, subjects)
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
            (
                true,
                JournalEntry::Created {
                    world_id: header_world,
                    version,
                },
            ) => {
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
    restore_replica_entries_with_runtime(
        world_id,
        limits,
        crate::world_limits::RuntimeLimits::default(),
        entries,
        directory,
        subjects,
    )
}

/// As [`restore_replica_entries`], resolving persisted resource requests
/// against the current server runtime ceilings.
pub fn restore_replica_entries_with_runtime(
    world_id: &str,
    limits: WorldLimits,
    runtime: crate::world_limits::RuntimeLimits,
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
    let mut restored = WorldCore::restore_replica_with_runtime(world_id, limits, runtime, entries)?;
    let facts = directory.replica_facts_for(subjects)?;
    for fact in facts {
        restored
            .core
            .directory_mut()
            .apply_replica_fact(&fact)
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
        presence: restored.presence,
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
    anyhow::ensure!(
        restored
            .program
            .as_ref()
            .is_none_or(|spec| spec.snapshot.is_none()),
        "replica journal has a checkpoint snapshot; restore the snapshot before serving"
    );
    for event in &restored.replay {
        program.update(event).with_context(|| {
            format!("replica program rejected replayed input {}", event.sequence)
        })?;
    }
    let cursor = restored.cursor;
    let handle = WorldHandle::spawn_read_only(restored.core, program, restored.presence)?;
    Ok((handle, cursor))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::directory::Actor;
    use crate::world::{
        CapabilityBounds, JournalEntry, WorldEvent, WorldEventKind, WorldScopes, journal_entry_line,
    };

    struct ReplicaProgram {
        updates: Arc<AtomicUsize>,
        online: bool,
    }

    impl WorldProgram for ReplicaProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            self.updates.fetch_add(1, Ordering::SeqCst);
            if let WorldEventKind::System { event } = &event.kind {
                let event: serde_json::Value = serde_json::from_str(event)?;
                if event["cap"] == "players" && event["event"] == "joined" {
                    self.online = true;
                } else if event["cap"] == "players" && event["event"] == "left" {
                    self.online = false;
                }
            }
            Ok(())
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(serde_json::json!({"online": self.online}).to_string()))
        }
    }

    fn program(updates: Arc<AtomicUsize>) -> Box<dyn WorldProgram> {
        Box::new(ReplicaProgram {
            updates,
            online: false,
        })
    }

    fn journal(entries: &[JournalEntry]) -> Vec<u8> {
        entries.iter().flat_map(journal_entry_line).collect()
    }

    #[tokio::test]
    async fn friend_delegated_spectator_serves_offline_and_replica_is_hard_read_only() {
        let world_id = "replica-friend-test";
        let mut source = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (ann_credential, _) = source.directory_mut().sign_up("Ann").unwrap();
        let ann = source.directory().account_for(&ann_credential).unwrap();
        let (bo_credential, _) = source.directory_mut().sign_up("Bo").unwrap();
        let bo = source.directory().account_for(&bo_credential).unwrap();
        source
            .directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        source
            .directory_mut()
            .request_friend(&ann_credential, &bo, world_id, 1)
            .unwrap();
        source
            .directory_mut()
            .accept_friend(&bo_credential, &ann, world_id, 2)
            .unwrap();

        let (owner, owner_token) = source
            .issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let (spectator, spectator_token) = source
            .delegate_to_friend(
                &owner_token,
                &bo,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Bo spectator",
            )
            .unwrap();
        let source_directory = source.directory().clone();
        let entries = vec![
            JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: super::super::JOURNAL_VERSION,
            },
            source.issued_entry(owner.id).unwrap(),
            source.issued_entry(spectator.id).unwrap(),
            // The last verified journal cursor records a participant online.
            // Replica restore must not feed a synthetic leave to its program.
            JournalEntry::System {
                event: WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: serde_json::json!({
                            "cap": "players",
                            "event": "joined",
                            "participant": {"id": spectator.id, "name": "Bo spectator"}
                        })
                        .to_string(),
                    },
                },
            },
        ];
        let journal_bytes = journal(&entries);
        let subjects = BTreeSet::from([ann, bo]);
        // No world handle, writer actor, or network contact exists beyond
        // this point; only the verified journal and copied directory facts.
        drop(source);

        let restored = restore_replica_journal(
            world_id,
            WorldLimits::default(),
            &journal_bytes,
            &source_directory,
            &subjects,
        )
        .unwrap();
        assert_eq!(restored.presence.get(&spectator.id), Some(&1));
        let updates = Arc::new(AtomicUsize::new(0));
        let (replica, cursor) = serve_replica(restored, program(Arc::clone(&updates))).unwrap();
        assert!(replica.is_read_only());
        assert_eq!(cursor.sequence, 0);
        assert_eq!(updates.load(Ordering::SeqCst), 1);
        assert_eq!(replica.records().1["online"], true);

        let snapshot = replica.snapshot(spectator_token.clone()).await.unwrap();
        assert_eq!(snapshot.current_sequence, 0);
        assert!(snapshot.events.is_empty());
        assert!(
            replica
                .chat(spectator_token.clone(), "must not commit")
                .await
                .is_err(),
            "replica refuses chat even if a bearer token has wider source scopes"
        );
        assert!(
            replica
                .input(spectator_token, b"must not run".to_vec())
                .await
                .is_err(),
            "replica refuses program inputs before running the program"
        );
        assert_eq!(replica.records().1["online"], true);
        assert_eq!(updates.load(Ordering::SeqCst), 1);
        assert!(replica.subscribe().try_recv().is_err());
        replica.shutdown().await.unwrap();
    }

    #[test]
    fn verified_replica_journal_parser_refuses_partial_or_wrong_world_bytes() {
        let valid = journal(&[JournalEntry::Created {
            world_id: "replica-parser-test".to_owned(),
            version: super::super::JOURNAL_VERSION,
        }]);
        assert_eq!(
            parse_replica_journal(&valid, "replica-parser-test")
                .unwrap()
                .len(),
            1
        );
        assert!(parse_replica_journal(&valid[..valid.len() - 1], "replica-parser-test").is_err());
        assert!(parse_replica_journal(&valid, "another-world").is_err());
        assert!(parse_replica_journal(b"not json\n", "replica-parser-test").is_err());
    }

    #[test]
    fn serve_replica_refuses_a_checkpoint_until_its_snapshot_is_restored() {
        let world_id = "replica-checkpoint-test";
        let restored = WorldCore::restore_replica(
            world_id,
            WorldLimits::default(),
            [JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: super::super::JOURNAL_VERSION,
            }],
        )
        .unwrap();
        let replica = ReplicaRestore {
            core: restored.core,
            program: Some(RestoredProgram {
                module_hash: "ab".repeat(32),
                seed: 1,
                snapshot: Some("checkpoint state".to_owned()),
                provenance: None,
            }),
            replay: Vec::new(),
            presence: restored.presence,
            cursor: ReplicaCursor {
                entries: 1,
                sequence: 0,
                header_version: super::super::JOURNAL_VERSION,
            },
        };
        let error = serve_replica(replica, Box::new(Counter { count: 0 })).unwrap_err();
        assert!(error.to_string().contains("checkpoint snapshot"));
    }

    #[test]
    fn serve_replica_refuses_an_unrestored_checkpoint_snapshot() {
        let world_id = "replica-checkpoint-test";
        let restored = WorldCore::restore_replica(
            world_id,
            WorldLimits::default(),
            [JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: super::super::JOURNAL_VERSION,
            }],
        )
        .unwrap();
        let replica = ReplicaRestore {
            core: restored.core,
            program: Some(RestoredProgram {
                module_hash: "ab".repeat(32),
                seed: 1,
                snapshot: Some("checkpoint state".to_owned()),
                provenance: None,
            }),
            replay: Vec::new(),
            presence: restored.presence,
            cursor: ReplicaCursor {
                entries: 1,
                sequence: 0,
                header_version: super::super::JOURNAL_VERSION,
            },
        };
        let error = serve_replica(replica, Box::new(Counter { count: 0 })).unwrap_err();
        assert!(error.to_string().contains("checkpoint snapshot"));
    }

    /// Counts inputs; records are the count. Mirrors the writer/reader
    /// contract in `world::tests` so parity compares real program state.
    struct Counter {
        count: u64,
    }

    impl WorldProgram for Counter {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::Input(_) = &event.kind {
                self.count += 1;
            }
            Ok(())
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(format!("{{\"count\":{}}}", self.count)))
        }
    }

    /// Drive a writer actor over a real [`FileJournal`](crate::world_store::FileJournal),
    /// then serve its exact bytes from a replica with the writer shut down:
    /// offline startup reaches the writer's sequence and `Records`.
    #[tokio::test]
    async fn replica_reaches_the_writer_sequence_and_records_offline() {
        use crate::world::WorldJournal;
        use crate::world_store::FileJournal;

        let world_id = "replica-offline-test";
        let dir = tempfile::TempDir::new().unwrap();
        let journal_path = dir.path().join("writer.jsonl");
        let mut journal = FileJournal::open(&journal_path, world_id).unwrap().0;
        let mut core = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (ann, ann_token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        journal.append(&core.issued_entry(ann.id).unwrap()).unwrap();
        let writer = WorldHandle::spawn_durable(
            core,
            Box::new(Counter { count: 0 }),
            Some(Box::new(journal)),
        );
        writer
            .input(ann_token.clone(), b"a".to_vec())
            .await
            .unwrap();
        writer
            .input(ann_token.clone(), b"b".to_vec())
            .await
            .unwrap();
        let (writer_sequence, writer_records) = writer.records();
        assert_eq!(writer_sequence, 2);
        assert_eq!(writer_records["count"], 2);
        // The writer is gone from here on: shutdown drops the actor and the
        // test never touches the writer handle again.
        let journal_bytes = std::fs::read(&journal_path).unwrap();
        writer.shutdown().await.unwrap();

        let restored = restore_replica_journal(
            world_id,
            WorldLimits::default(),
            &journal_bytes,
            &crate::directory::Directory::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        let (replica, cursor) = serve_replica(restored, Box::new(Counter { count: 0 })).unwrap();
        assert!(replica.is_read_only());
        assert_eq!(cursor.sequence, writer_sequence);
        assert_eq!(cursor.entries, 4);
        let (replica_sequence, replica_records) = replica.records();
        assert_eq!(replica_sequence, writer_sequence);
        assert_eq!(*replica_records, *writer_records);
        // The bearer token from the writer journal authorizes spectator
        // reads on the replica with no writer contact.
        let snapshot = replica.snapshot(ann_token.clone()).await.unwrap();
        assert_eq!(snapshot.current_sequence, writer_sequence);
        assert_eq!(snapshot.events.len(), 2);
        replica.shutdown().await.unwrap();
    }

    /// Every mutating handle path refuses on a replica; every read path
    /// serves. The journal bytes are unchanged by serving.
    #[tokio::test]
    async fn replica_refuses_every_mutation_and_serves_every_read() {
        use crate::world::{Authority, FileScope, Isolation};

        let world_id = "replica-read-only-test";
        let mut core = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (ann, ann_token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let entries = vec![
            JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: super::super::JOURNAL_VERSION,
            },
            core.issued_entry(ann.id).unwrap(),
        ];
        let journal_bytes = journal(&entries);
        let restored = restore_replica_journal(
            world_id,
            WorldLimits::default(),
            &journal_bytes,
            &crate::directory::Directory::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        let (replica, _) = serve_replica(restored, Box::new(Counter { count: 0 })).unwrap();

        // Reads serve.
        replica.snapshot(ann_token.clone()).await.unwrap();
        replica.events_after(ann_token.clone(), 0).await.unwrap();
        replica.view(ann_token.clone()).await.unwrap();
        replica.metrics().await.unwrap();
        replica.policy().await.unwrap();
        replica
            .directory(
                crate::directory_auth::Caller::default(),
                crate::directory_view::DirectoryAction::View,
                false,
            )
            .await
            .unwrap();
        let artifact_read = replica
            .directory(
                crate::directory_auth::Caller::default(),
                crate::directory_view::DirectoryAction::ReadArtifacts {
                    account: "01".repeat(32),
                    after: 0,
                },
                false,
            )
            .await;
        if let Err(error) = artifact_read {
            assert!(!format!("{error:#}").contains("read-only"));
        }

        // Mutations refuse with the replica error.
        let err = |result: Result<()>| format!("{result:?}");
        assert!(err(replica.chat(ann_token.clone(), "no").await.map(|_| ())).contains("read-only"));
        assert!(
            err(replica
                .input(ann_token.clone(), b"no".to_vec())
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica.issue("bo", WorldScopes::GUEST).await.map(|_| ())).contains("read-only")
        );
        assert!(
            err(replica
                .attenuate(
                    ann_token.clone(),
                    WorldScopes::JOIN,
                    CapabilityBounds::default(),
                    "bo"
                )
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .delegate_to_friend(
                    ann_token.clone(),
                    "friend",
                    WorldScopes::JOIN,
                    CapabilityBounds::default(),
                    "friend",
                )
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            !replica.revoke(ann.id).await.unwrap(),
            "replica revokes nothing"
        );
        assert!(
            err(replica
                .bind_author_key(ann_token.clone(), [9; 32])
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .install_program("ab".repeat(32), 1, Box::new(Counter { count: 0 }))
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .update_caps(vec!["chat.say".to_owned()], vec![])
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(err(replica.set_caps_ceiling(vec![]).await.map(|_| ())).contains("read-only"));
        assert!(
            err(replica
                .set_execution_profile(crate::world_limits::WorldExecutionProfile::default())
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .set_isolation(Authority::Admin, Isolation::Unisolated)
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .set_cross_world_write(Authority::Admin, true)
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .force_cross_world_write(Authority::Admin)
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .set_file_scope(Authority::Admin, FileScope::Unrestricted, Some(true))
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .admin_schedule_at("s".to_owned(), 1_700_000_000_000)
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            err(replica
                .admin_cancel_schedule("s".to_owned())
                .await
                .map(|_| ()))
            .contains("read-only")
        );
        assert!(
            replica
                .apply_directory_entry(crate::directory::DirectoryEntry::AccountKeyed {
                    id: "01".repeat(32),
                    name: "local write".to_owned(),
                })
                .await
                .is_err()
        );
        assert!(
            replica
                .directory(
                    crate::directory_auth::Caller::default(),
                    crate::directory_view::DirectoryAction::SignUp {
                        name: "local write".to_owned(),
                    },
                    false,
                )
                .await
                .is_err()
        );
        assert!(
            replica
                .directory(
                    crate::directory_auth::Caller::default(),
                    crate::directory_view::DirectoryAction::SignUp {
                        name: "local write".to_owned(),
                    },
                    false,
                )
                .await
                .is_err()
        );
        assert!(
            replica.replication_feed().await.is_err(),
            "a replica is never a replication source"
        );
        // Nothing was emitted or persisted anywhere.
        assert!(replica.subscribe().try_recv().is_err());
        assert_eq!(journal(&entries), journal_bytes);
        replica.shutdown().await.unwrap();
    }

    /// Without the directory facts, the friend-delegated spectator stops at
    /// authorization: the dependency phase 4 carries is load-bearing.
    #[tokio::test]
    async fn replica_without_directory_facts_rejects_the_delegated_spectator() {
        let world_id = "replica-facts-test";
        let mut source = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (ann_credential, _) = source.directory_mut().sign_up("Ann").unwrap();
        let ann = source.directory().account_for(&ann_credential).unwrap();
        let (bo_credential, _) = source.directory_mut().sign_up("Bo").unwrap();
        let bo = source.directory().account_for(&bo_credential).unwrap();
        source
            .directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        source
            .directory_mut()
            .request_friend(&ann_credential, &bo, world_id, 1)
            .unwrap();
        source
            .directory_mut()
            .accept_friend(&bo_credential, &ann, world_id, 2)
            .unwrap();
        let (_, owner_token) = source
            .issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let (spectator, spectator_token) = source
            .delegate_to_friend(
                &owner_token,
                &bo,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        let entries = vec![
            JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: super::super::JOURNAL_VERSION,
            },
            source.issued_entry(1).unwrap(),
            source.issued_entry(spectator.id).unwrap(),
        ];
        let journal_bytes = journal(&entries);
        let subjects = BTreeSet::from([ann, bo]);
        drop(source);

        let restored = restore_replica_journal(
            world_id,
            WorldLimits::default(),
            &journal_bytes,
            &crate::directory::Directory::new(),
            &subjects,
        )
        .unwrap();
        let (replica, _) = serve_replica(restored, Box::new(Counter { count: 0 })).unwrap();
        assert!(
            replica.snapshot(spectator_token).await.is_err(),
            "an empty directory must not authorize the friend delegation"
        );
        replica.shutdown().await.unwrap();
    }

    /// The existing world session protocol serves spectators from a replica
    /// and refuses its writes, with no session-layer changes.
    #[tokio::test]
    async fn replica_serves_spectator_sessions_over_the_existing_protocol() {
        use crate::world_hub::WorldHub;
        use crate::world_session::{
            Inbound, SessionConfig, SessionIo, WorldService, run_session_with,
        };
        use tokio::sync::mpsc;

        struct ChannelIo {
            inbound: mpsc::Receiver<Inbound>,
            outbound: mpsc::Sender<String>,
        }

        impl SessionIo for ChannelIo {
            async fn recv(&mut self) -> Inbound {
                self.inbound.recv().await.unwrap_or(Inbound::Closed)
            }

            async fn send(&mut self, frame: String) -> bool {
                self.outbound.send(frame).await.is_ok()
            }
        }

        let world_id = "replica-session-test";
        let mut core = WorldCore::new(world_id, WorldLimits::default()).unwrap();
        let (_, token) = core
            .issue_capability("spectator", WorldScopes::JOIN, CapabilityBounds::default())
            .unwrap();
        let entries = vec![
            JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: super::super::JOURNAL_VERSION,
            },
            core.issued_entry(1).unwrap(),
        ];
        let journal_bytes = journal(&entries);
        drop(core);
        let restored = restore_replica_journal(
            world_id,
            WorldLimits::default(),
            &journal_bytes,
            &crate::directory::Directory::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        let (replica, _) = serve_replica(restored, Box::new(Counter { count: 0 })).unwrap();
        let hub = WorldHub::single(WorldService::new(replica, None));
        let (to_server, inbound) = mpsc::channel(16);
        let (outbound, mut from_server) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut io = ChannelIo { inbound, outbound };
            run_session_with(&hub, &mut io, SessionConfig::default()).await;
        });

        async fn drain(from_server: &mut mpsc::Receiver<String>) -> Vec<serde_json::Value> {
            let mut frames = Vec::new();
            while let Ok(Some(text)) =
                tokio::time::timeout(std::time::Duration::from_millis(300), from_server.recv())
                    .await
            {
                frames.push(serde_json::from_str(&text).unwrap());
            }
            frames
        }

        let cap = token.expose().to_owned();
        to_server
            .send(Inbound::Text(
                serde_json::json!({"type": "join", "capability": cap, "after": null}).to_string(),
            ))
            .await
            .unwrap();
        let frames = drain(&mut from_server).await;
        assert!(
            frames.iter().any(|frame| frame["type"] == "snapshot"),
            "spectator joins: {frames:?}"
        );

        to_server
            .send(Inbound::Text(
                serde_json::json!({"type": "chat", "text": "hi"}).to_string(),
            ))
            .await
            .unwrap();
        let frames = drain(&mut from_server).await;
        assert!(
            frames.iter().any(|frame| frame["type"] == "error"
                && frame["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("read-only")),
            "spectator chat refuses: {frames:?}"
        );

        to_server
            .send(Inbound::Text(serde_json::json!({"type": "records", "capability": cap, "prefix": null, "after": null}).to_string()))
            .await
            .unwrap();
        let frames = drain(&mut from_server).await;
        assert!(
            frames.iter().any(|frame| frame["type"] == "records"),
            "spectator reads records: {frames:?}"
        );
        drop(to_server);
    }
}
